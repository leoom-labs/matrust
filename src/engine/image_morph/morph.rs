//! Flat morphology (imdilate, imerode, imopen, imclose, imtophat, imbothat),
//! bwmorph, imfill, bwareaopen and imclearborder.
//!
//! Grey dilation is the maximum over the reflected element, erosion the
//! minimum over the element (Soille 2003, ch. 3). Pixels outside the image
//! are -Inf for dilation and +Inf for erosion, so the border never creates
//! or removes pixels.
use super::regions::label;
use super::strel::{Se, se_arg};
use super::{
    Cx, Img, Kind, check_elements, connectivity, image, image_2d, mask_2d, mask_value, scalar,
    text_of, value,
};
use crate::Result;
use crate::engine::Value;
use rayon::prelude::*;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, VecDeque};

/// Pixel-offset visits above which a plane runs on several workers.
const PARALLEL_WORK: u64 = 1 << 22;
/// Neighbourhood reads one call may do (pixels x reads per pixel, summed over
/// its passes): 2^35 = 3.4e10, under a minute on a few cores at the measured
/// 0.5-1.3e9 reads per second (docs/tasks/courses-track-image-proc-031925-2-notes.md).
/// Large flat elements are not decomposed, so the work grows with their area.
const MAX_WORK: u64 = 1 << 35;
/// Reads between two cancellation polls.
const POLL_WORK: usize = 10_000_000;

/// Full rectangles take the two-pass (row, column) path.
fn is_rectangle(se: &Se) -> bool {
    se.is_full() && se.rows * se.cols > 1
}

/// Reads per pixel of one dilation or erosion pass: a column window plus a
/// row window for a rectangle, one read per member otherwise.
fn reads_per_pixel(se: &Se) -> usize {
    if is_rectangle(se) {
        se.rows + se.cols
    } else {
        se.mask.iter().filter(|&&b| b).count()
    }
}

/// Refuses a call before it starts when `passes` dilations or erosions of
/// `pixels` pixels with `se` need more than MAX_WORK reads.
fn check_work(cx: &Cx, pixels: usize, se: &Se, passes: usize) -> Result<()> {
    let per_pixel = passes * reads_per_pixel(se);
    let work = (pixels as u64).saturating_mul(per_pixel as u64);
    if work > MAX_WORK {
        return cx.err(format!(
            "{pixels} pixels at {per_pixel} neighbourhood reads each need {work} reads in total, \
             more than the {MAX_WORK}-read limit; use a smaller structuring element or image"
        ));
    }
    Ok(())
}

/// `max` = dilation (reads I(r - dr, c - dc)), otherwise erosion (reads
/// I(r + dr, c + dc)). An out-of-image position reads the padding value
/// (-Inf/+Inf); a window starts from NaN (nothing read yet), so it is NaN
/// only when every sample is NaN. An empty element gives the padding value.
fn extreme_plane(
    cx: &Cx,
    src: &[f64],
    rows: usize,
    cols: usize,
    se: &Se,
    max: bool,
) -> Result<Vec<f64>> {
    if is_rectangle(se) {
        return separable(cx, src, rows, cols, se, max);
    }
    let sign: isize = if max { -1 } else { 1 };
    let offsets: Vec<(isize, isize)> = se
        .offsets()
        .into_iter()
        .map(|(dr, dc)| (sign * dr, sign * dc))
        .collect();
    let pad = if max {
        f64::NEG_INFINITY
    } else {
        f64::INFINITY
    };
    let init = if offsets.is_empty() { pad } else { f64::NAN };
    let mut out = vec![pad; rows * cols];
    // Rows whose every offset stays inside the image; within them, columns
    // that do too read through precomputed linear offsets without checks.
    let rlo = offsets.iter().map(|o| -o.0).max().unwrap_or(0).max(0) as usize;
    let rhi =
        (rows as isize - offsets.iter().map(|o| o.0).max().unwrap_or(0).max(0)).max(0) as usize;
    let clo = offsets.iter().map(|o| -o.1).max().unwrap_or(0).max(0) as usize;
    let chi =
        (cols as isize - offsets.iter().map(|o| o.1).max().unwrap_or(0).max(0)).max(0) as usize;
    let linear: Vec<isize> = offsets
        .iter()
        .map(|&(dr, dc)| dc * rows as isize + dr)
        .collect();
    // Pixels between two cancellation polls: about POLL_WORK reads, also
    // inside one long column.
    let step = (POLL_WORK / offsets.len().max(1)).max(1);
    let column = |c: usize, dst: &mut [f64]| -> Result<()> {
        let inner_col = c >= clo && c < chi;
        for (b, block) in dst.chunks_mut(step).enumerate() {
            cx.check()?;
            for (i, slot) in block.iter_mut().enumerate() {
                let r = b * step + i;
                let mut acc = init;
                if inner_col && r >= rlo && r < rhi {
                    let base = (c * rows + r) as isize;
                    for &k in &linear {
                        let v = src[(base + k) as usize];
                        acc = if max {
                            pick_max(acc, v)
                        } else {
                            pick_min(acc, v)
                        };
                    }
                    *slot = acc;
                    continue;
                }
                for &(dr, dc) in &offsets {
                    let rr = r as isize + dr;
                    let cc = c as isize + dc;
                    let v = if rr < 0 || cc < 0 || rr >= rows as isize || cc >= cols as isize {
                        pad
                    } else {
                        src[cc as usize * rows + rr as usize]
                    };
                    acc = if max {
                        pick_max(acc, v)
                    } else {
                        pick_min(acc, v)
                    };
                }
                *slot = acc;
            }
        }
        Ok(())
    };
    if rows == 0 {
        return Ok(out);
    }
    let work = (rows as u64 * cols as u64).saturating_mul(offsets.len().max(1) as u64);
    if work >= PARALLEL_WORK && cols > 1 {
        out.par_chunks_mut(rows)
            .enumerate()
            .try_for_each(|(c, dst)| column(c, dst))?;
    } else {
        for (c, dst) in out.chunks_mut(rows).enumerate() {
            column(c, dst)?;
        }
    }
    Ok(out)
}

/// MATLAB max/min ignore NaN unless every value is NaN: folds start from
/// NaN, which the first sample replaces, and a NaN sample never wins.
fn pick_max(a: f64, b: f64) -> f64 {
    if b > a || a.is_nan() { b } else { a }
}
fn pick_min(a: f64, b: f64) -> f64 {
    if b < a || a.is_nan() { b } else { a }
}

/// Full rectangles: a row pass and a column pass of 1-D windows.
fn separable(
    cx: &Cx,
    src: &[f64],
    rows: usize,
    cols: usize,
    se: &Se,
    max: bool,
) -> Result<Vec<f64>> {
    let (r0, c0) = se.origin();
    // Window of positions read, relative to the output position.
    let window = |len: usize, origin: usize| -> (isize, isize) {
        let lo = -(origin as isize);
        let hi = (len - 1 - origin) as isize;
        if max { (-hi, -lo) } else { (lo, hi) }
    };
    let (rlo, rhi) = window(se.rows, r0);
    let (clo, chi) = window(se.cols, c0);
    // Padding value (-Inf/+Inf) read by windows that leave the image; a
    // window that stays inside starts from NaN, so all-NaN gives NaN.
    let pad = if max {
        f64::NEG_INFINITY
    } else {
        f64::INFINITY
    };
    let pick = |a: f64, b: f64| if max { pick_max(a, b) } else { pick_min(a, b) };
    let work = (rows as u64 * cols as u64).saturating_mul((se.rows + se.cols) as u64);
    let parallel = work >= PARALLEL_WORK && cols > 1;
    // Cancellation polls come about every POLL_WORK reads: every `down_step`
    // pixels of a column in pass 1, every `across_step` source columns
    // (rows reads each) in pass 2.
    let down_step = (POLL_WORK / se.rows).max(1);
    let across_step = (POLL_WORK / rows.max(1)).max(1);
    // Pass 1: windows down each column.
    let mut tmp = vec![pad; rows * cols];
    let down = |c: usize, dst: &mut [f64]| -> Result<()> {
        let col = &src[c * rows..(c + 1) * rows];
        for (n, block) in dst.chunks_mut(down_step).enumerate() {
            cx.check()?;
            for (i, slot) in block.iter_mut().enumerate() {
                let r = n * down_step + i;
                let clipped = r as isize + rlo < 0 || r as isize + rhi > rows as isize - 1;
                let a = (r as isize + rlo).max(0) as usize;
                let b = (r as isize + rhi).min(rows as isize - 1);
                let mut acc = if clipped { pad } else { f64::NAN };
                if b >= a as isize {
                    for &v in &col[a..=b as usize] {
                        acc = pick(acc, v);
                    }
                }
                *slot = acc;
            }
        }
        Ok(())
    };
    // Pass 2: windows along each row, written column by column.
    let mut out = vec![pad; rows * cols];
    let across = |tmp: &[f64], c: usize, dst: &mut [f64]| -> Result<()> {
        cx.check()?;
        let clipped = c as isize + clo < 0 || c as isize + chi > cols as isize - 1;
        let a = (c as isize + clo).max(0) as usize;
        let b = (c as isize + chi).min(cols as isize - 1);
        if b < a as isize {
            return Ok(());
        }
        dst.copy_from_slice(&tmp[a * rows..(a + 1) * rows]);
        for (n, k) in (a + 1..=b as usize).enumerate() {
            if n % across_step == 0 {
                cx.check()?;
            }
            for (slot, &v) in dst.iter_mut().zip(&tmp[k * rows..(k + 1) * rows]) {
                *slot = pick(*slot, v);
            }
        }
        if clipped {
            for slot in dst.iter_mut() {
                *slot = pick(*slot, pad);
            }
        }
        Ok(())
    };
    if rows == 0 {
        return Ok(out);
    }
    if parallel {
        tmp.par_chunks_mut(rows)
            .enumerate()
            .try_for_each(|(c, dst)| down(c, dst))?;
        out.par_chunks_mut(rows)
            .enumerate()
            .try_for_each(|(c, dst)| across(&tmp, c, dst))?;
    } else {
        for (c, dst) in tmp.chunks_mut(rows).enumerate() {
            down(c, dst)?;
        }
        for (c, dst) in out.chunks_mut(rows).enumerate() {
            across(&tmp, c, dst)?;
        }
    }
    Ok(out)
}

/// Dilation or erosion of every 2-D plane, in the class of the image.
fn extreme(cx: &Cx, img: &Img, se: &Se, max: bool) -> Result<Img> {
    let (rows, cols) = (img.rows(), img.cols());
    let plane = rows * cols;
    let mut data = Vec::with_capacity(img.len());
    for p in 0..img.planes() {
        let src = &img.data[p * plane..(p + 1) * plane];
        data.extend(extreme_plane(cx, src, rows, cols, se, max)?);
    }
    // Integer and logical classes cannot hold the infinite padding values.
    if img.kind != Kind::Double && img.kind != Kind::Single {
        let (low, width) = img.kind.range();
        for v in &mut data {
            *v = v.clamp(low, low + width);
        }
    }
    Ok(img.with(data, img.kind))
}

fn subtract(a: &Img, b: &Img) -> Img {
    let data = a
        .data
        .iter()
        .zip(&b.data)
        .map(|(x, y)| match a.kind {
            Kind::Logical => f64::from(u8::from(*x != 0. && *y == 0.)),
            Kind::Int(_) => {
                let (low, _) = a.kind.range();
                (x - y).max(low)
            }
            _ => x - y,
        })
        .collect();
    a.with(data, a.kind)
}

pub(super) fn morphology(cx: &Cx, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        if args.len() > 2 && text_of(&args[2]).is_some() {
            return cx
                .err("shape options ('same', 'full') and packed images are not supported yet");
        }
        return cx.err(format!("expected {}(I, SE)", cx.name));
    }
    let img = image(cx, &args[0])?;
    check_elements(cx, img.len())?;
    let se = se_arg(cx, &args[1])?;
    // Opening, closing and the top-hats run two passes.
    let passes = if matches!(cx.name, "imdilate" | "imerode") {
        1
    } else {
        2
    };
    check_work(cx, img.len(), &se, passes)?;
    let result = match cx.name {
        "imdilate" => extreme(cx, &img, &se, true)?,
        "imerode" => extreme(cx, &img, &se, false)?,
        "imopen" => extreme(cx, &extreme(cx, &img, &se, false)?, &se, true)?,
        "imclose" => extreme(cx, &extreme(cx, &img, &se, true)?, &se, false)?,
        "imtophat" => {
            let open = extreme(cx, &extreme(cx, &img, &se, false)?, &se, true)?;
            subtract(&img, &open)
        }
        _ => {
            let close = extreme(cx, &extreme(cx, &img, &se, true)?, &se, false)?;
            subtract(&close, &img)
        }
    };
    value(cx, result)
}

/// 8 neighbours of (r, c), outside = false: E, NE, N, NW, W, SW, S, SE.
fn neighbours(m: &[bool], rows: usize, cols: usize, r: usize, c: usize) -> [bool; 8] {
    let at = |dr: isize, dc: isize| -> bool {
        let rr = r as isize + dr;
        let cc = c as isize + dc;
        rr >= 0
            && cc >= 0
            && rr < rows as isize
            && cc < cols as isize
            && m[cc as usize * rows + rr as usize]
    };
    [
        at(0, 1),
        at(-1, 1),
        at(-1, 0),
        at(-1, -1),
        at(0, -1),
        at(1, -1),
        at(1, 0),
        at(1, 1),
    ]
}

/// One parallel thinning subiteration (Lam, Lee & Suen 1992, p. 879):
/// delete p when X_H(p) = 1, 2 <= min(n1, n2) <= 3 and, in the first
/// subiteration, (x2 | x3 | !x8) & x1 = 0 (second: (x6 | x7 | !x4) & x5 = 0).
fn thin_pass(m: &[bool], rows: usize, cols: usize, second: bool) -> (Vec<bool>, bool) {
    let mut out = m.to_vec();
    let mut changed = false;
    for c in 0..cols {
        for r in 0..rows {
            if !m[c * rows + r] {
                continue;
            }
            let n = neighbours(m, rows, cols, r, c);
            // x[k] for k = 1..9 with x9 = x1.
            let x = |k: usize| n[(k - 1) % 8];
            let xh = (1..=4)
                .filter(|&i| !x(2 * i - 1) && (x(2 * i) || x(2 * i + 1)))
                .count();
            let n1 = (1..=4).filter(|&k| x(2 * k - 1) || x(2 * k)).count();
            let n2 = (1..=4).filter(|&k| x(2 * k) || x(2 * k + 1)).count();
            let g2 = (2..=3).contains(&n1.min(n2));
            let g3 = if second {
                !((x(6) || x(7) || !x(4)) && x(5))
            } else {
                !((x(2) || x(3) || !x(8)) && x(1))
            };
            if xh == 1 && g2 && g3 {
                out[c * rows + r] = false;
                changed = true;
            }
        }
    }
    (out, changed)
}

fn bw_step(op: &str, m: &[bool], rows: usize, cols: usize) -> Vec<bool> {
    let mut out = m.to_vec();
    for c in 0..cols {
        for r in 0..rows {
            let p = m[c * rows + r];
            let n = neighbours(m, rows, cols, r, c);
            let count = n.iter().filter(|&&b| b).count();
            out[c * rows + r] = match op {
                // Interior pixels (all four edge neighbours set) are removed.
                "remove" => p && !(n[0] && n[2] && n[4] && n[6]),
                // Isolated pixels (no neighbour set) are removed.
                "clean" => p && count > 0,
                // Isolated interior 0s (all eight neighbours set) are filled.
                "fill" => p || count == 8,
                "majority" => count + usize::from(p) >= 5,
                _ => p,
            };
        }
    }
    out
}

pub(super) fn bwmorph(cx: &Cx, args: &[Value]) -> Result<Value> {
    if !(2..=3).contains(&args.len()) {
        return cx.err("expected bwmorph(BW, operation) or bwmorph(BW, operation, n)");
    }
    let (rows, cols, mut m) = mask_2d(cx, &args[0])?;
    let Some(op) = text_of(&args[1]) else {
        return cx.err("the operation must be text");
    };
    let op = op.to_ascii_lowercase();
    let n = match args.get(2) {
        None => 1.,
        Some(v) => scalar(cx, v, "n")?,
    };
    if !(n >= 0. && (n.fract() == 0. || n.is_infinite())) {
        return cx.err("n must be a nonnegative integer or Inf");
    }
    match op.as_str() {
        "remove" | "clean" | "fill" | "majority" | "dilate" | "erode" | "open" | "close"
        | "thin" => {}
        "skel" | "bothat" | "branchpoints" | "bridge" | "diag" | "endpoints" | "hbreak"
        | "shrink" | "spur" | "thicken" | "tophat" => {
            return cx.err(format!("the '{op}' operation is not supported yet"));
        }
        _ => return cx.err(format!("unknown operation '{op}'")),
    }
    let limit = if n.is_infinite() {
        rows * cols + 1
    } else {
        n as usize
    };
    let se = Se::square3();
    for i in 0..limit {
        cx.check()?;
        let next = match op.as_str() {
            "thin" => {
                let (a, _) = thin_pass(&m, rows, cols, false);
                thin_pass(&a, rows, cols, true).0
            }
            "dilate" | "erode" | "open" | "close" => {
                let img = Img {
                    dims: vec![rows, cols],
                    data: m.iter().map(|&b| f64::from(u8::from(b))).collect(),
                    kind: Kind::Logical,
                };
                let r = match op.as_str() {
                    "dilate" => extreme(cx, &img, &se, true)?,
                    "erode" => extreme(cx, &img, &se, false)?,
                    "open" => extreme(cx, &extreme(cx, &img, &se, false)?, &se, true)?,
                    _ => extreme(cx, &extreme(cx, &img, &se, true)?, &se, false)?,
                };
                r.data.iter().map(|&v| v != 0.).collect()
            }
            _ => bw_step(&op, &m, rows, cols),
        };
        let done = next == m;
        m = next;
        if done || i > rows * cols {
            break;
        }
    }
    mask_value(cx, rows, cols, &m)
}

const DIRS4: [(isize, isize); 4] = [(-1, 0), (0, -1), (0, 1), (1, 0)];
const DIRS8: [(isize, isize); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];
pub(super) fn dirs(conn: u8) -> &'static [(isize, isize)] {
    if conn == 4 { &DIRS4 } else { &DIRS8 }
}
fn step(rows: usize, cols: usize, p: usize, d: (isize, isize)) -> Option<usize> {
    let (r, c) = ((p % rows) as isize + d.0, (p / rows) as isize + d.1);
    (r >= 0 && c >= 0 && r < rows as isize && c < cols as isize)
        .then(|| c as usize * rows + r as usize)
}
fn border(rows: usize, cols: usize) -> impl Iterator<Item = usize> {
    (0..rows * cols).filter(move |&p| {
        let (r, c) = (p % rows, p / rows);
        r == 0 || c == 0 || r + 1 == rows || c + 1 == cols
    })
}

/// Background pixels reachable from `seeds` through background (conn).
fn flood(
    cx: &Cx,
    rows: usize,
    cols: usize,
    fg: &[bool],
    seeds: impl Iterator<Item = usize>,
    conn: u8,
) -> Result<Vec<bool>> {
    let mut seen = vec![false; rows * cols];
    let mut queue = VecDeque::new();
    for s in seeds {
        if !fg[s] && !seen[s] {
            seen[s] = true;
            queue.push_back(s);
        }
    }
    let mut n = 0;
    while let Some(p) = queue.pop_front() {
        n += 1;
        cx.poll(n)?;
        for &d in dirs(conn) {
            if let Some(q) = step(rows, cols, p, d)
                && !fg[q]
                && !seen[q]
            {
                seen[q] = true;
                queue.push_back(q);
            }
        }
    }
    Ok(seen)
}

#[derive(PartialEq)]
struct Key(f64, usize);
impl Eq for Key {}
impl PartialOrd for Key {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Key {
    fn cmp(&self, o: &Self) -> Ordering {
        self.0.total_cmp(&o.0).then(o.1.cmp(&self.1))
    }
}

/// Priority flood from the border. `fill` = true: grey hole filling, the
/// smallest over border paths of the path maximum (reconstruction by
/// erosion). `fill` = false: the largest over border paths of the path
/// minimum (reconstruction by dilation of the border values).
fn border_flood(
    cx: &Cx,
    img: &[f64],
    rows: usize,
    cols: usize,
    conn: u8,
    fill: bool,
) -> Result<Vec<f64>> {
    let mut out = vec![f64::NAN; rows * cols];
    let mut done = vec![false; rows * cols];
    let mut heap = BinaryHeap::new();
    let key = |v: f64| if fill { -v } else { v };
    for p in border(rows, cols) {
        done[p] = true;
        out[p] = img[p];
        heap.push(Key(key(img[p]), p));
    }
    let mut n = 0;
    while let Some(Key(k, p)) = heap.pop() {
        n += 1;
        cx.poll(n)?;
        let v = if fill { -k } else { k };
        for &d in dirs(conn) {
            if let Some(q) = step(rows, cols, p, d)
                && !done[q]
            {
                done[q] = true;
                let w = if fill { v.max(img[q]) } else { v.min(img[q]) };
                let w = if img[q].is_nan() { img[q] } else { w };
                out[q] = w;
                heap.push(Key(key(w), q));
            }
        }
    }
    Ok(out)
}

pub(super) fn imfill(cx: &Cx, args: &[Value]) -> Result<Value> {
    let img = image_2d(cx, &args[0])?;
    check_elements(cx, img.len())?;
    let (rows, cols) = (img.rows(), img.cols());
    let mut rest: Vec<&Value> = args[1..].iter().collect();
    let holes = match rest.last().and_then(|v| text_of(v)) {
        Some(t) if t.eq_ignore_ascii_case("holes") => {
            rest.pop();
            true
        }
        Some(t) => return cx.err(format!("unknown option '{t}' (only 'holes')")),
        None => false,
    };
    if let Some(t) = rest.iter().find_map(|v| text_of(v)) {
        return cx.err(format!(
            "unknown option '{t}' (only 'holes' as the last argument)"
        ));
    }
    let binary = img.kind == Kind::Logical;
    let fg: Vec<bool> = img.data.iter().map(|&v| v != 0.).collect();
    let finish = |out: Vec<bool>| -> Result<Value> {
        if binary {
            return mask_value(cx, rows, cols, &out);
        }
        let data = out.iter().map(|&b| f64::from(u8::from(b))).collect();
        value(cx, img.with(data, img.kind))
    };
    if holes {
        if rest.len() > 1 {
            return cx.err("expected imfill(BW, 'holes') or imfill(BW, conn, 'holes')");
        }
        let conn = match rest.first() {
            None => 4,
            Some(v) => connectivity(cx, v)?,
        };
        let outside = flood(cx, rows, cols, &fg, border(rows, cols), conn)?;
        return finish(outside.iter().map(|&o| !o).collect());
    }
    if binary {
        let Some(loc) = rest.first() else {
            return cx.err(
                "the interactive form is not supported; use imfill(BW, 'holes') or imfill(BW, locations)",
            );
        };
        if rest.len() > 2 {
            return cx.err("expected imfill(BW, locations) or imfill(BW, locations, conn)");
        }
        let conn = match rest.get(1) {
            None => 4,
            Some(v) => connectivity(cx, v)?,
        };
        let seeds = seeds(cx, loc, rows, cols)?;
        let reached = flood(cx, rows, cols, &fg, seeds.into_iter(), conn)?;
        return finish(fg.iter().zip(&reached).map(|(a, b)| *a || *b).collect());
    }
    // Grey image: holes are regional minima not connected to the border.
    if rest.len() > 1 {
        return cx.err("expected imfill(I) or imfill(I, conn) for a grayscale image");
    }
    let conn = match rest.first() {
        None => 4,
        Some(v) => connectivity(cx, v)?,
    };
    let data = border_flood(cx, &img.data, rows, cols, conn, true)?;
    value(cx, img.with(data, img.kind))
}

fn seeds(cx: &Cx, loc: &Value, rows: usize, cols: usize) -> Result<Vec<usize>> {
    let v = super::numbers(cx, loc, "locations")?;
    let dims = loc.dims();
    let mut out = Vec::new();
    if dims.len() == 2 && dims[1] == 2 && !v.is_empty() {
        let k = dims[0];
        for i in 0..k {
            let (r, c) = (v[i], v[k + i]);
            if !(r >= 1.
                && c >= 1.
                && r.fract() == 0.
                && c.fract() == 0.
                && r <= rows as f64
                && c <= cols as f64)
            {
                return cx.err("locations must be valid row and column subscripts");
            }
            out.push((c as usize - 1) * rows + r as usize - 1);
        }
    } else {
        for x in v {
            if !(x >= 1. && x.fract() == 0. && x <= (rows * cols) as f64) {
                return cx.err("locations must be valid linear indices");
            }
            out.push(x as usize - 1);
        }
    }
    Ok(out)
}

pub(super) fn bwareaopen(cx: &Cx, args: &[Value]) -> Result<Value> {
    if !(2..=3).contains(&args.len()) {
        return cx.err("expected bwareaopen(BW, P) or bwareaopen(BW, P, conn)");
    }
    let (rows, cols, m) = mask_2d(cx, &args[0])?;
    let p = scalar(cx, &args[1], "P")?;
    if p.is_nan() || p < 0. {
        return cx.err("P must be a nonnegative number");
    }
    let conn = match args.get(2) {
        None => 8,
        Some(v) => connectivity(cx, v)?,
    };
    let (labels, n) = label(cx, rows, cols, &m, conn)?;
    let mut sizes = vec![0usize; n + 1];
    for &l in &labels {
        sizes[l as usize] += 1;
    }
    let out: Vec<bool> = labels
        .iter()
        .map(|&l| l != 0 && sizes[l as usize] as f64 >= p)
        .collect();
    mask_value(cx, rows, cols, &out)
}

pub(super) fn imclearborder(cx: &Cx, args: &[Value]) -> Result<Value> {
    if !(1..=2).contains(&args.len()) {
        return cx.err("expected imclearborder(I) or imclearborder(I, conn)");
    }
    let img = image_2d(cx, &args[0])?;
    check_elements(cx, img.len())?;
    let conn = match args.get(1) {
        None => 8,
        Some(v) => connectivity(cx, v)?,
    };
    let (rows, cols) = (img.rows(), img.cols());
    if img.kind == Kind::Logical {
        let fg: Vec<bool> = img.data.iter().map(|&v| v != 0.).collect();
        let (labels, n) = label(cx, rows, cols, &fg, conn)?;
        let mut touching = vec![false; n + 1];
        for p in border(rows, cols) {
            touching[labels[p] as usize] = true;
        }
        let out: Vec<bool> = labels
            .iter()
            .map(|&l| l != 0 && !touching[l as usize])
            .collect();
        return mask_value(cx, rows, cols, &out);
    }
    let rec = border_flood(cx, &img.data, rows, cols, conn, false)?;
    let data = img.data.iter().zip(&rec).map(|(a, b)| a - b).collect();
    value(cx, img.with(data, img.kind))
}
