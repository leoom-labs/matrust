//! UI-independent comparisons. Clean-room sources: MathWorks' public `visdiff`
//! documentation and Myers, Algorithmica 1 (1986), 251–266, doi:10.1007/BF01840446.
//!
//! Ranges of lines are one-based, half-open (an empty range denotes an insertion
//! point). Character ranges are zero-based Unicode scalar offsets, half-open.
//! Text retains terminators, including a missing final newline, for exact replay.
use crate::{Array, Result, dsp::check_cancel, files};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    fs::{self, File},
    io::{BufReader, Read},
    ops::Range,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

const MAX_TEXT_BYTES: usize = 16 * 1024 * 1024;
const MAX_LINES: usize = 100_000;
const MAX_REPORT_BYTES: usize = 16 * 1024 * 1024;
const MAX_WORK: usize = 64_000_000;
const MAX_ENTRIES: usize = 10_000;
const MAX_WORKSPACE: u64 = 256 * 1024 * 1024;
static NOT_CANCELLED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, Default)]
pub struct TextDiffOptions<'a> {
    pub ignore_whitespace: bool,
    pub ignore_case: bool,
    pub ignore_crlf: bool,
    pub cancel: Option<&'a AtomicBool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HunkKind {
    Equal,
    Insert,
    Delete,
    Replace,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineDiff {
    pub left_line: usize,
    pub right_line: usize,
    pub left: Vec<Range<usize>>,
    pub right: Vec<Range<usize>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextHunk {
    pub kind: HunkKind,
    pub left: Range<usize>,
    pub right: Range<usize>,
    pub inline: Vec<InlineDiff>,
}

#[derive(Clone, Debug)]
pub struct TextDiff {
    pub left_lines: Vec<String>,
    pub right_lines: Vec<String>,
    pub hunks: Vec<TextHunk>,
    pub deleted_lines: usize,
    pub inserted_lines: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edit {
    Equal,
    Delete,
    Insert,
}

struct Work<'a> {
    cancel: &'a AtomicBool,
    remaining: usize,
}
impl Work<'_> {
    fn tick(&mut self) -> Result<()> {
        self.remaining = self
            .remaining
            .checked_sub(1)
            .ok_or("Comparison exceeds work limit")?;
        if self.remaining.is_multiple_of(1024) {
            check_cancel(self.cancel)?;
        }
        Ok(())
    }
}

// Myers' bidirectional frontier finds a midpoint of a shortest edit path.
// Frontiers are discarded before processing either child: O(N+M) workspace,
// O((N+M)D) time (quadratic worst case), without a quadratic trace allocation.
fn midpoint<T: Eq>(a: &[T], b: &[T], work: &mut Work<'_>) -> Result<(usize, usize)> {
    let (n, m) = (a.len() as isize, b.len() as isize);
    let half = (n + m + 1) / 2;
    let offset = half + 1;
    let mut forward = vec![-1isize; (2 * half + 3) as usize];
    let mut reverse = forward.clone();
    forward[(offset + 1) as usize] = 0;
    reverse[(offset + 1) as usize] = 0;
    let delta = n - m;
    let odd = delta % 2 != 0;
    for d in 0..=half {
        check_cancel(work.cancel)?;
        for k in (-d..=d).step_by(2) {
            work.tick()?;
            let i = (offset + k) as usize;
            let mut x = if k == -d || (k != d && forward[i - 1] < forward[i + 1]) {
                forward[i + 1]
            } else {
                forward[i - 1] + 1
            };
            let mut y = x - k;
            while x < n && y < m && x >= 0 && y >= 0 && a[x as usize] == b[y as usize] {
                work.tick()?;
                x += 1;
                y += 1;
            }
            forward[i] = x;
            let rk = delta - k;
            if odd && rk > -d && rk < d {
                let rx = reverse[(offset + rk) as usize];
                if rx >= 0 && x >= n - rx {
                    return Ok((x as usize, y as usize));
                }
            }
        }
        for k in (-d..=d).step_by(2) {
            work.tick()?;
            let i = (offset + k) as usize;
            let mut x = if k == -d || (k != d && reverse[i - 1] < reverse[i + 1]) {
                reverse[i + 1]
            } else {
                reverse[i - 1] + 1
            };
            let mut y = x - k;
            while x < n
                && y < m
                && x >= 0
                && y >= 0
                && a[(n - x - 1) as usize] == b[(m - y - 1) as usize]
            {
                work.tick()?;
                x += 1;
                y += 1;
            }
            reverse[i] = x;
            let fk = delta - k;
            if !odd && fk >= -d && fk <= d {
                let fx = forward[(offset + fk) as usize];
                if fx >= 0 && fx >= n - x {
                    return Ok((fx as usize, (fx - fk) as usize));
                }
            }
        }
    }
    Err("Comparison could not find an edit path".into())
}

fn edits<T: Eq>(a: &[T], b: &[T], work: &mut Work<'_>) -> Result<Vec<Edit>> {
    enum Task {
        Pair(Range<usize>, Range<usize>),
        Equal(usize),
    }
    let mut tasks = vec![Task::Pair(0..a.len(), 0..b.len())];
    let mut result = Vec::with_capacity(a.len() + b.len());
    while let Some(task) = tasks.pop() {
        check_cancel(work.cancel)?;
        let Task::Pair(mut ar, mut br) = task else {
            let Task::Equal(n) = task else { unreachable!() };
            result.extend(std::iter::repeat_n(Edit::Equal, n));
            continue;
        };
        while !ar.is_empty() && !br.is_empty() && a[ar.start] == b[br.start] {
            work.tick()?;
            result.push(Edit::Equal);
            ar.start += 1;
            br.start += 1;
        }
        let mut suffix = 0;
        while !ar.is_empty() && !br.is_empty() && a[ar.end - 1] == b[br.end - 1] {
            work.tick()?;
            suffix += 1;
            ar.end -= 1;
            br.end -= 1;
        }
        if suffix != 0 {
            tasks.push(Task::Equal(suffix));
        }
        if ar.is_empty() || br.is_empty() {
            result.extend(std::iter::repeat_n(Edit::Delete, ar.len()));
            result.extend(std::iter::repeat_n(Edit::Insert, br.len()));
        } else if ar.len() == 1 || br.len() == 1 {
            // The single-element base case keeps every existing match.
            let found = if ar.len() == 1 {
                let mut found = None;
                for j in br.clone() {
                    work.tick()?;
                    if a[ar.start] == b[j] {
                        found = Some((ar.start, j));
                        break;
                    }
                }
                found
            } else {
                let mut found = None;
                for i in ar.clone() {
                    work.tick()?;
                    if a[i] == b[br.start] {
                        found = Some((i, br.start));
                        break;
                    }
                }
                found
            };
            if let Some((i, j)) = found {
                result.extend(std::iter::repeat_n(Edit::Delete, i - ar.start));
                result.extend(std::iter::repeat_n(Edit::Insert, j - br.start));
                result.push(Edit::Equal);
                result.extend(std::iter::repeat_n(Edit::Delete, ar.end - i - 1));
                result.extend(std::iter::repeat_n(Edit::Insert, br.end - j - 1));
            } else {
                result.extend(std::iter::repeat_n(Edit::Delete, ar.len()));
                result.extend(std::iter::repeat_n(Edit::Insert, br.len()));
            }
        } else {
            let (x, y) = midpoint(&a[ar.clone()], &b[br.clone()], work)?;
            if (x == 0 && y == 0) || (x == ar.len() && y == br.len()) {
                return Err("Comparison produced a non-progressing edit path".into());
            }
            let (i, j) = (ar.start + x, br.start + y);
            tasks.push(Task::Pair(i..ar.end, j..br.end));
            tasks.push(Task::Pair(ar.start..i, br.start..j));
        }
    }
    Ok(result)
}

// Preserve original scalar positions through whitespace/case normalization.
fn normalized(
    line: &str,
    opts: TextDiffOptions<'_>,
    work: &mut Work<'_>,
) -> Result<Vec<(char, Range<usize>)>> {
    let mut out = Vec::new();
    let mut space: Option<Range<usize>> = None;
    let cr = opts.ignore_crlf && line.ends_with("\r\n");
    let count = line.chars().count();
    if count > 65_536 {
        return Err("Text comparison exceeds 65536 characters per line".into());
    }
    for (i, c) in line.chars().enumerate() {
        work.tick()?;
        if cr && i + 2 == count {
            continue;
        }
        if opts.ignore_whitespace && c != '\n' && c.is_whitespace() {
            if let Some(r) = &mut space {
                r.end = i + 1;
            } else {
                space = Some(i..i + 1);
            }
            continue;
        }
        if let Some(r) = space.take()
            && !out.is_empty()
            && c != '\n'
        {
            out.push((' ', r));
        }
        if opts.ignore_case {
            out.extend(c.to_lowercase().map(|c| (c, i..i + 1)));
        } else {
            out.push((c, i..i + 1));
        }
    }
    Ok(out)
}

fn character_ranges(tokens: &[(char, Range<usize>)], changed: &[bool]) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = Vec::new();
    for ((_, r), yes) in tokens.iter().zip(changed) {
        if !yes {
            continue;
        }
        if let Some(last) = out.last_mut()
            && last.end >= r.start
        {
            last.end = last.end.max(r.end);
        } else {
            out.push(r.clone());
        }
    }
    out
}

pub fn text_diff(left: &str, right: &str, opts: TextDiffOptions<'_>) -> Result<TextDiff> {
    let mut work = Work {
        cancel: opts.cancel.unwrap_or(&NOT_CANCELLED),
        remaining: MAX_WORK,
    };
    check_cancel(work.cancel)?;
    if left.len().saturating_add(right.len()) > MAX_TEXT_BYTES {
        return Err("Text comparison exceeds 16 MiB input limit".into());
    }
    let lines = |s: &str| -> Result<Vec<String>> {
        let mut out = Vec::new();
        for line in s.split_inclusive('\n') {
            check_cancel(work.cancel)?;
            if out.len() == MAX_LINES {
                return Err("Text comparison exceeds 100000 lines".into());
            }
            out.push(line.to_owned());
        }
        Ok(out)
    };
    let left_lines = lines(left)?;
    let right_lines = lines(right)?;
    let mut keys = |lines: &[String]| -> Result<Vec<String>> {
        lines
            .iter()
            .map(|s| {
                Ok(normalized(s, opts, &mut work)?
                    .into_iter()
                    .map(|(c, _)| c)
                    .collect())
            })
            .collect()
    };
    let a = keys(&left_lines)?;
    let b = keys(&right_lines)?;
    let script = edits(&a, &b, &mut work)?;
    let mut hunks = Vec::new();
    let (mut i, mut j, mut pos, mut deleted, mut inserted) = (1, 1, 0, 0, 0);
    while pos < script.len() {
        check_cancel(work.cancel)?;
        let equal = script[pos] == Edit::Equal;
        let (start_i, start_j) = (i, j);
        while pos < script.len() && (script[pos] == Edit::Equal) == equal {
            match script[pos] {
                Edit::Equal => {
                    i += 1;
                    j += 1;
                }
                Edit::Delete => {
                    i += 1;
                    deleted += 1;
                }
                Edit::Insert => {
                    j += 1;
                    inserted += 1;
                }
            }
            pos += 1;
        }
        let kind = if equal {
            HunkKind::Equal
        } else if i == start_i {
            HunkKind::Insert
        } else if j == start_j {
            HunkKind::Delete
        } else {
            HunkKind::Replace
        };
        let mut inline = Vec::new();
        if kind == HunkKind::Replace {
            for (li, rj) in (start_i..i).zip(start_j..j) {
                let lt = normalized(&left_lines[li - 1], opts, &mut work)?;
                let rt = normalized(&right_lines[rj - 1], opts, &mut work)?;
                let lc: Vec<_> = lt.iter().map(|(c, _)| *c).collect();
                let rc: Vec<_> = rt.iter().map(|(c, _)| *c).collect();
                let mut lchanged = vec![false; lc.len()];
                let mut rchanged = vec![false; rc.len()];
                let (mut l, mut r) = (0, 0);
                for edit in edits(&lc, &rc, &mut work)? {
                    match edit {
                        Edit::Equal => {
                            l += 1;
                            r += 1;
                        }
                        Edit::Delete => {
                            lchanged[l] = true;
                            l += 1;
                        }
                        Edit::Insert => {
                            rchanged[r] = true;
                            r += 1;
                        }
                    }
                }
                inline.push(InlineDiff {
                    left_line: li,
                    right_line: rj,
                    left: character_ranges(&lt, &lchanged),
                    right: character_ranges(&rt, &rchanged),
                });
            }
        }
        hunks.push(TextHunk {
            kind,
            left: start_i..i,
            right: start_j..j,
            inline,
        });
    }
    Ok(TextDiff {
        left_lines,
        right_lines,
        hunks,
        deleted_lines: deleted,
        inserted_lines: inserted,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffStatus {
    OnlyLeft,
    OnlyRight,
    Same,
    Differs,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ValueDifference {
    pub count: usize,
    pub first_index: usize,
    /// Infinity for unmatched NaN/infinity, None for nonnumeric values.
    pub max_abs_difference: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DifferenceReason {
    Class { left: String, right: String },
    Size { left: Vec<usize>, right: Vec<usize> },
    Complexity,
    Sparsity,
    Values(ValueDifference),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ArrayDifference {
    /// Variable/field name, or a one-based cell/struct element index.
    pub name: String,
    pub status: DiffStatus,
    pub reasons: Vec<DifferenceReason>,
    pub children: Vec<ArrayDifference>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MatDiff {
    pub variables: Vec<ArrayDifference>,
}

fn real_difference(a: f64, b: f64) -> (bool, f64) {
    if a == b || (a.is_nan() && b.is_nan()) {
        (false, 0.)
    } else if a.is_nan() || b.is_nan() {
        (true, f64::INFINITY)
    } else {
        (true, (a - b).abs())
    }
}

fn scan_values(
    n: usize,
    numeric: bool,
    cancel: &AtomicBool,
    mut at: impl FnMut(usize) -> (bool, f64),
) -> Result<Option<DifferenceReason>> {
    let mut v = ValueDifference {
        count: 0,
        first_index: 0,
        max_abs_difference: numeric.then_some(0.),
    };
    for i in 0..n {
        if i.is_multiple_of(1024) {
            check_cancel(cancel)?;
        }
        let (different, delta) = at(i);
        if different {
            if v.count == 0 {
                v.first_index = i + 1;
            }
            v.count += 1;
            if let Some(max) = &mut v.max_abs_difference {
                *max = max.max(delta);
            }
        }
    }
    Ok((v.count != 0).then_some(DifferenceReason::Values(v)))
}

fn leaf(name: String, status: DiffStatus) -> ArrayDifference {
    ArrayDifference {
        name,
        status,
        reasons: Vec::new(),
        children: Vec::new(),
    }
}

/// Also usable for unsaved workspace values. Never converts integer or sparse
/// storage to dense doubles; integers above 2^53 retain exact comparisons.
pub fn array_diff(
    name: &str,
    left: &Array,
    right: &Array,
    cancel: &AtomicBool,
) -> Result<ArrayDifference> {
    check_cancel(cancel)?;
    let mut out = leaf(name.into(), DiffStatus::Same);
    if left.class() != right.class() {
        out.reasons.push(DifferenceReason::Class {
            left: left.class().into(),
            right: right.class().into(),
        });
    }
    if left.dims() != right.dims() {
        out.reasons.push(DifferenceReason::Size {
            left: left.dims().to_vec(),
            right: right.dims().to_vec(),
        });
    }
    if left.is_complex() != right.is_complex() {
        out.reasons.push(DifferenceReason::Complexity);
    }
    if matches!(left, Array::Sparse(_)) != matches!(right, Array::Sparse(_)) {
        out.reasons.push(DifferenceReason::Sparsity);
    }
    if !out.reasons.is_empty() {
        out.status = DiffStatus::Differs;
        return Ok(out);
    }
    let reason = match (left, right) {
        (Array::Numeric(a), Array::Numeric(b)) => scan_values(a.len(), true, cancel, |i| {
            real_difference(a.data()[i], b.data()[i])
        })?,
        (Array::Logical(a), Array::Logical(b)) => scan_values(a.len(), true, cancel, |i| {
            (
                a.data()[i] != b.data()[i],
                f64::from(a.data()[i].abs_diff(b.data()[i])),
            )
        })?,
        (Array::Typed(a), Array::Typed(b)) => scan_values(a.len(), true, cancel, |i| {
            if a.is_integer() {
                (a.int(i) != b.int(i), a.int(i).abs_diff(b.int(i)) as f64)
            } else {
                real_difference(a.real(i), b.real(i))
            }
        })?,
        (Array::Complex(a), Array::Complex(b)) => scan_values(a.len(), true, cancel, |i| {
            let (re, dr) = real_difference(a.data()[i].re, b.data()[i].re);
            let (im, di) = real_difference(a.data()[i].im, b.data()[i].im);
            (re || im, dr.hypot(di))
        })?,
        (Array::Character(a), Array::Character(b)) => scan_values(a.len(), false, cancel, |i| {
            (a.units()[i] != b.units()[i], 0.)
        })?,
        (Array::Str(a), Array::Str(b)) => {
            scan_values(a.len(), false, cancel, |i| (a.data()[i] != b.data()[i], 0.))?
        }
        (Array::Cell(a), Array::Cell(b)) => {
            for i in 0..a.len() {
                out.children.push(array_diff(
                    &format!("{{{}}}", i + 1),
                    a.get_linear(i)?,
                    b.get_linear(i)?,
                    cancel,
                )?);
            }
            None
        }
        (Array::Struct(a), Array::Struct(b)) => {
            let fields: BTreeSet<_> = a.field_names().iter().chain(b.field_names()).collect();
            for name in fields {
                check_cancel(cancel)?;
                match (a.field_index(name), b.field_index(name)) {
                    (Some(af), Some(bf)) => {
                        let mut field = leaf(name.clone(), DiffStatus::Same);
                        for i in 0..a.len() {
                            field.children.push(array_diff(
                                &format!("({})", i + 1),
                                a.get(af, i)?,
                                b.get(bf, i)?,
                                cancel,
                            )?);
                        }
                        if field.children.iter().any(|c| c.status != DiffStatus::Same) {
                            field.status = DiffStatus::Differs;
                        }
                        out.children.push(field);
                    }
                    (Some(_), None) => out.children.push(leaf(name.clone(), DiffStatus::OnlyLeft)),
                    (None, Some(_)) => out.children.push(leaf(name.clone(), DiffStatus::OnlyRight)),
                    _ => unreachable!(),
                }
            }
            None
        }
        (Array::Sparse(a), Array::Sparse(b)) => {
            let mut ai = a.entries().peekable();
            let mut bi = b.entries().peekable();
            let mut v = ValueDifference {
                count: 0,
                first_index: 0,
                max_abs_difference: Some(0.),
            };
            while ai.peek().is_some() || bi.peek().is_some() {
                check_cancel(cancel)?;
                let index = |&(r, c, _): &(usize, usize, f64)| c * a.rows() + r;
                let i = ai
                    .peek()
                    .map(index)
                    .unwrap_or(usize::MAX)
                    .min(bi.peek().map(index).unwrap_or(usize::MAX));
                let av = if ai.peek().is_some_and(|x| index(x) == i) {
                    ai.next().unwrap().2
                } else {
                    0.
                };
                let bv = if bi.peek().is_some_and(|x| index(x) == i) {
                    bi.next().unwrap().2
                } else {
                    0.
                };
                let (different, delta) = real_difference(av, bv);
                if different {
                    if v.count == 0 {
                        v.first_index = i + 1;
                    }
                    v.count += 1;
                    v.max_abs_difference = Some(v.max_abs_difference.unwrap().max(delta));
                }
            }
            (v.count != 0).then_some(DifferenceReason::Values(v))
        }
        _ => {
            return Err(format!(
                "visdiff: {} arrays are not supported",
                left.class()
            ));
        }
    };
    if let Some(reason) = reason {
        out.reasons.push(reason);
    }
    if !out.reasons.is_empty() || out.children.iter().any(|c| c.status != DiffStatus::Same) {
        out.status = DiffStatus::Differs;
    }
    Ok(out)
}

fn load_mat(path: &Path, cancel: &AtomicBool) -> Result<BTreeMap<String, Array>> {
    let catalog = files::inspect_workspace(path, cancel)?;
    let mut bytes = 0u64;
    let mut names = BTreeSet::new();
    for info in catalog.variables() {
        check_cancel(cancel)?;
        if let Some(reason) = &info.unavailable {
            return Err(format!("{}: {reason}", info.name));
        }
        bytes = bytes
            .checked_add(info.storage_bytes().ok_or("MAT size overflow")?)
            .ok_or("Workspace size overflow")?;
        names.insert(info.name.clone());
        if bytes > MAX_WORKSPACE || names.len() > MAX_ENTRIES {
            return Err("Resulting workspace exceeds 256 MiB or 10000 variables".into());
        }
    }
    if names.is_empty() {
        return Ok(BTreeMap::new());
    }
    files::load_selected_arrays(&catalog, &names, cancel)
}

pub fn mat_diff(left_path: &Path, right_path: &Path, cancel: &AtomicBool) -> Result<MatDiff> {
    let left = load_mat(left_path, cancel)?;
    let right = load_mat(right_path, cancel)?;
    let names: BTreeSet<_> = left.keys().chain(right.keys()).collect();
    let mut variables = Vec::with_capacity(names.len());
    for name in names {
        check_cancel(cancel)?;
        variables.push(match (left.get(name), right.get(name)) {
            (Some(a), Some(b)) => array_diff(name, a, b, cancel)?,
            (Some(_), None) => leaf(name.clone(), DiffStatus::OnlyLeft),
            (None, Some(_)) => leaf(name.clone(), DiffStatus::OnlyRight),
            _ => unreachable!(),
        });
    }
    Ok(MatDiff { variables })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryDiff {
    pub left_bytes: u64,
    pub right_bytes: u64,
    /// Zero-based byte offset, including EOF on a size difference.
    pub first_offset: Option<u64>,
}

fn path_metadata(path: &Path) -> Result<fs::Metadata> {
    fs::metadata(path).map_err(|e| format!("Could not open file '{}'. {e}", path.display()))
}

pub fn binary_diff(left: &Path, right: &Path, cancel: &AtomicBool) -> Result<BinaryDiff> {
    let (lm, rm) = (path_metadata(left)?, path_metadata(right)?);
    if !lm.is_file() || !rm.is_file() {
        return Err("visdiff requires regular files".into());
    }
    let mut a = BufReader::new(File::open(left).map_err(|e| e.to_string())?);
    let mut b = BufReader::new(File::open(right).map_err(|e| e.to_string())?);
    let mut offset = 0;
    let mut ab = [0u8; 65536];
    let mut bb = [0u8; 65536];
    loop {
        check_cancel(cancel)?;
        // Fill each chunk: a short read is not necessarily EOF.
        let an = read_chunk(&mut a, &mut ab)?;
        let bn = read_chunk(&mut b, &mut bb)?;
        if let Some(i) = ab[..an.min(bn)].iter().zip(&bb).position(|(x, y)| x != y) {
            return Ok(BinaryDiff {
                left_bytes: lm.len(),
                right_bytes: rm.len(),
                first_offset: Some(offset + i as u64),
            });
        }
        if an != bn {
            return Ok(BinaryDiff {
                left_bytes: lm.len(),
                right_bytes: rm.len(),
                first_offset: Some(offset + an.min(bn) as u64),
            });
        }
        if an == 0 {
            return Ok(BinaryDiff {
                left_bytes: lm.len(),
                right_bytes: rm.len(),
                first_offset: None,
            });
        }
        offset += an as u64;
    }
}

fn read_chunk(reader: &mut impl Read, out: &mut [u8]) -> Result<usize> {
    let mut n = 0;
    while n < out.len() {
        match reader.read(&mut out[n..]) {
            Ok(0) => break,
            Ok(m) => n += m,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(n)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderEntry {
    pub name: std::ffi::OsString,
    pub status: DiffStatus,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderDiff {
    pub files: Vec<FolderEntry>,
}

fn folder_files(path: &Path, cancel: &AtomicBool) -> Result<BTreeMap<std::ffi::OsString, PathBuf>> {
    let mut out = BTreeMap::new();
    let entries = fs::read_dir(path)
        .map_err(|e| format!("Could not open folder '{}'. {e}", path.display()))?;
    for (i, entry) in entries.enumerate() {
        check_cancel(cancel)?;
        if i == MAX_ENTRIES {
            return Err("Folder comparison exceeds 10000 entries".into());
        }
        let entry = entry.map_err(|e| e.to_string())?;
        // Do not follow directory symlinks or block on special files.
        if entry.file_type().map_err(|e| e.to_string())?.is_file() {
            out.insert(entry.file_name(), entry.path());
        }
    }
    Ok(out)
}

pub fn folder_diff(left_dir: &Path, right_dir: &Path) -> Result<FolderDiff> {
    folder_diff_with_cancel(left_dir, right_dir, &NOT_CANCELLED)
}

pub fn folder_diff_with_cancel(
    left_dir: &Path,
    right_dir: &Path,
    cancel: &AtomicBool,
) -> Result<FolderDiff> {
    let left = folder_files(left_dir, cancel)?;
    let right = folder_files(right_dir, cancel)?;
    let mut files = Vec::new();
    for name in left.keys().chain(right.keys()).collect::<BTreeSet<_>>() {
        check_cancel(cancel)?;
        let status = match (left.get(name), right.get(name)) {
            (Some(a), Some(b)) => {
                if binary_diff(a, b, cancel)?.first_offset.is_none() {
                    DiffStatus::Same
                } else {
                    DiffStatus::Differs
                }
            }
            (Some(_), None) => DiffStatus::OnlyLeft,
            (None, Some(_)) => DiffStatus::OnlyRight,
            _ => unreachable!(),
        };
        files.push(FolderEntry {
            name: name.clone(),
            status,
        });
    }
    Ok(FolderDiff { files })
}

#[derive(Clone, Debug)]
pub enum Comparison {
    Text(TextDiff),
    Mat(MatDiff),
    Folder(FolderDiff),
    Binary(BinaryDiff),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ComparisonType {
    #[default]
    Auto,
    Text,
    Binary,
}

pub fn compare_paths(
    left: &Path,
    right: &Path,
    kind: ComparisonType,
    cancel: &AtomicBool,
) -> Result<Comparison> {
    check_cancel(cancel)?;
    let (lm, rm) = (path_metadata(left)?, path_metadata(right)?);
    if lm.is_dir() != rm.is_dir() {
        return Err("Cannot compare a file with a folder.".into());
    }
    if lm.is_dir() {
        return folder_diff_with_cancel(left, right, cancel).map(Comparison::Folder);
    }
    if !lm.is_file() || !rm.is_file() {
        return Err("visdiff requires regular files or folders".into());
    }
    if kind == ComparisonType::Binary {
        return binary_diff(left, right, cancel).map(Comparison::Binary);
    }
    let mat = |p: &Path| p.extension().is_some_and(|s| s.eq_ignore_ascii_case("mat"));
    if kind == ComparisonType::Auto && (mat(left) || mat(right)) {
        return mat_diff(left, right, cancel).map(Comparison::Mat);
    }
    if lm.len().saturating_add(rm.len()) > MAX_TEXT_BYTES as u64 {
        return Err("Text comparison exceeds 16 MiB input limit".into());
    }
    let read = |p: &Path| -> Result<String> {
        let mut file = File::open(p).map_err(|e| e.to_string())?;
        let mut data = Vec::new();
        let mut chunk = [0u8; 65536];
        loop {
            check_cancel(cancel)?;
            let n = file.read(&mut chunk).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            if data.len() + n > MAX_TEXT_BYTES {
                return Err("Text comparison exceeds 16 MiB input limit".into());
            }
            data.extend_from_slice(&chunk[..n]);
        }
        String::from_utf8(data)
            .map_err(|_| "visdiff: only UTF-8 text is supported; use 'binary'".into())
    };
    text_diff(
        &read(left)?,
        &read(right)?,
        TextDiffOptions {
            cancel: Some(cancel),
            ..Default::default()
        },
    )
    .map(Comparison::Text)
}

fn report_bound(out: &str) -> Result<()> {
    if out.len() > MAX_REPORT_BYTES {
        Err("Comparison report exceeds 16 MiB output limit".into())
    } else {
        Ok(())
    }
}

fn unified_line(out: &mut String, prefix: char, text: &str) {
    out.push(prefix);
    out.push_str(text);
    if !text.ends_with('\n') {
        out.push_str("\n\\ No newline at end of file\n");
    }
}

fn unified_range(start: usize, len: usize) -> String {
    if len == 1 {
        start.to_string()
    } else {
        format!("{},{}", if len == 0 { start - 1 } else { start }, len)
    }
}

/// Unified text with three context lines, following difflib's header convention.
pub fn render_text_report(
    diff: &TextDiff,
    left_name: &str,
    right_name: &str,
    cancel: &AtomicBool,
) -> Result<String> {
    let mut out = String::new();
    let mut groups: Vec<(usize, usize)> = Vec::new();
    let mut rows = Vec::new();
    let (mut l, mut r) = (1, 1);
    for h in &diff.hunks {
        check_cancel(cancel)?;
        match h.kind {
            HunkKind::Equal => {
                for (li, ri) in h.left.clone().zip(h.right.clone()) {
                    rows.push((' ', li, ri));
                }
            }
            _ => {
                for li in h.left.clone() {
                    rows.push(('-', li, h.right.start));
                }
                for ri in h.right.clone() {
                    rows.push(('+', h.left.end, ri));
                }
            }
        }
    }
    for (i, row) in rows.iter().enumerate() {
        if row.0 == ' ' {
            continue;
        }
        let (start, end) = (i.saturating_sub(3), (i + 4).min(rows.len()));
        if let Some(last) = groups.last_mut()
            && start <= last.1
        {
            last.1 = end;
        } else {
            groups.push((start, end));
        }
    }
    if !groups.is_empty() {
        let _ = write!(out, "--- {left_name}\n+++ {right_name}\n");
        report_bound(&out)?;
    }
    let mut pos = 0;
    for (start, end) in groups {
        while pos < start {
            if rows[pos].0 != '+' {
                l += 1;
            }
            if rows[pos].0 != '-' {
                r += 1;
            }
            pos += 1;
        }
        let lc = rows[start..end].iter().filter(|x| x.0 != '+').count();
        let rc = rows[start..end].iter().filter(|x| x.0 != '-').count();
        let _ = writeln!(
            out,
            "@@ -{} +{} @@",
            unified_range(l, lc),
            unified_range(r, rc)
        );
        for &(prefix, li, ri) in &rows[start..end] {
            check_cancel(cancel)?;
            let text = if prefix == '+' {
                &diff.right_lines[ri - 1]
            } else {
                &diff.left_lines[li - 1]
            };
            unified_line(&mut out, prefix, text);
            report_bound(&out)?;
            if prefix != '+' {
                l += 1;
            }
            if prefix != '-' {
                r += 1;
            }
        }
        pos = end;
    }
    Ok(out)
}

fn status_text(s: DiffStatus) -> &'static str {
    match s {
        DiffStatus::OnlyLeft => "OnlyLeft",
        DiffStatus::OnlyRight => "OnlyRight",
        DiffStatus::Same => "Same",
        DiffStatus::Differs => "Differs",
    }
}

fn mat_rows(
    out: &mut String,
    rows: &[ArrayDifference],
    prefix: &str,
    cancel: &AtomicBool,
) -> Result<()> {
    for row in rows {
        check_cancel(cancel)?;
        let name = if prefix.is_empty() {
            row.name.clone()
        } else {
            format!("{prefix}.{}", row.name)
        };
        let _ = write!(out, "{name}\t{}", status_text(row.status));
        for reason in &row.reasons {
            match reason {
                DifferenceReason::Class { left, right } => {
                    let _ = write!(out, "\tclass: {left} -> {right}");
                }
                DifferenceReason::Size { left, right } => {
                    let _ = write!(out, "\tsize: {left:?} -> {right:?}");
                }
                DifferenceReason::Complexity => out.push_str("\tcomplexity"),
                DifferenceReason::Sparsity => out.push_str("\tsparsity"),
                DifferenceReason::Values(v) => {
                    let _ = write!(
                        out,
                        "\tvalues: {} differing, first index {}",
                        v.count, v.first_index
                    );
                    if let Some(max) = v.max_abs_difference {
                        let _ = write!(out, ", max abs difference {max}");
                    }
                }
            }
        }
        out.push('\n');
        report_bound(out)?;
        mat_rows(out, &row.children, &name, cancel)?;
    }
    Ok(())
}

/// Shared CLI / desktop export renderer. Identical text produces an empty diff.
pub fn render_report(
    comparison: &Comparison,
    left_name: &str,
    right_name: &str,
    cancel: &AtomicBool,
) -> Result<String> {
    check_cancel(cancel)?;
    let mut out = String::new();
    match comparison {
        Comparison::Text(diff) => return render_text_report(diff, left_name, right_name, cancel),
        Comparison::Mat(diff) => {
            out.push_str("Variable\tStatus\tDetails\n");
            mat_rows(&mut out, &diff.variables, "", cancel)?;
        }
        Comparison::Folder(diff) => {
            for file in &diff.files {
                check_cancel(cancel)?;
                let _ = writeln!(
                    out,
                    "{}\t{}",
                    status_text(file.status),
                    file.name.to_string_lossy()
                );
                report_bound(&out)?;
            }
        }
        Comparison::Binary(diff) => {
            if let Some(offset) = diff.first_offset {
                let _ = writeln!(
                    out,
                    "Binary files differ at byte offset {offset} (left: {} bytes, right: {} bytes).",
                    diff.left_bytes, diff.right_bytes
                );
            } else {
                out.push_str("Binary files are identical.\n");
            }
        }
    }
    report_bound(&out)?;
    Ok(out)
}
