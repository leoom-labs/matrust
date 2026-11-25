//! Image segmentation, implemented from public MathWorks function contracts.
//! Numerical methods: separable lower envelopes of squared-distance parabolas
//! (Felzenszwalb/Huttenlocher), Rosenfeld/Pfaltz chamfer scans, Meyer flooding,
//! Chan/Vese region energy, geodesic edge energy, and Moore/Jacob tracing.
//! No proprietary implementation or MATLAB-generated reference data is used.
use super::{Engine, Execution, Value};
use crate::sparse::SparseMatrix;
use crate::{Array, CellArray, LogicalMatrix, Matrix, NumericClass, Result, TypedArray};
use std::{cmp::Ordering, collections::BinaryHeap, sync::Arc};

const NAMES: &[&str] = &[
    "bwdist",
    "watershed",
    "activecontour",
    "imsegkmeans",
    "grayconnected",
    "bwboundaries",
    "bwperim",
    "bwarea",
    "bwareafilt",
    "bwpropfilt",
    "bwselect",
];
// Conservative aggregate workspace bound: at most 128 bytes/pixel. Images may
// use the engine's memory budget (2 Mi samples under the default 256 MiB);
// count arguments and the other fixed workspaces keep this default bound.
const MAX_PIXELS: usize = 2 * 1024 * 1024;
const BYTES_PER_PIXEL: usize = 128;

/// Image samples one call may hold: the memory budget at 128 bytes each.
fn max_samples(e: &Engine) -> usize {
    super::limits::memory_bytes(&e.limits) / BYTES_PER_PIXEL
}
const DIRS: [(isize, isize); 8] = [
    (-1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
    (1, 0),
    (1, -1),
    (0, -1),
    (-1, -1),
];
pub(super) fn handles(name: &str) -> bool {
    NAMES.contains(&name)
}

pub(super) fn call(
    e: &mut Engine,
    name: &str,
    a: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    e.check_cancel()?;
    let max = match name {
        "bwdist" | "imsegkmeans" | "bwselect" => 2,
        "bwboundaries" => 4,
        _ => 1,
    };
    if count > max {
        return Err(format!("{name}: too many output arguments"));
    }
    if a.is_empty() {
        return Err(format!("{name}: not enough input arguments"));
    }
    let result = match name {
        "bwdist" => distance(e, a),
        "watershed" => watershed(e, a).map(|v| vec![v]),
        "activecontour" => active_contour(e, a).map(|v| vec![v]),
        "imsegkmeans" => segment_kmeans(e, a, out),
        "grayconnected" => gray_connected(e, a).map(|v| vec![v]),
        "bwboundaries" => boundaries(e, a),
        "bwperim" => perimeter(e, a).map(|v| vec![v]),
        "bwarea" => area(e, a).map(|v| vec![v]),
        "bwareafilt" | "bwpropfilt" => filter(e, name, a, out).map(|v| vec![v]),
        "bwselect" => select(e, a),
        _ => Err("unknown image segmentation function".into()),
    };
    let mut result = result.map_err(|s| format!("{name}: {s}"))?;
    result.truncate(count.max(1));
    Ok(result)
}

struct Image {
    rows: usize,
    cols: usize,
    channels: usize,
    data: Vec<f64>,
    class: Option<NumericClass>,
    logical: bool,
}
fn image(e: &Engine, v: &Value, channels: bool) -> Result<Image> {
    image_within(v, channels, max_samples(e))
}
fn image_within(v: &Value, channels: bool, max: usize) -> Result<Image> {
    let dims = match v {
        Value::Matrix(m) => m.dims(),
        Value::Logical(m) => m.dims(),
        Value::Container(Array::Typed(t)) => t.dims(),
        _ => return Err("expected a real, full numeric or logical image".into()),
    };
    if dims.len() > if channels { 3 } else { 2 } {
        return Err("image must be 2-D (or a 2-D multichannel image where supported)".into());
    }
    let size = dims
        .iter()
        .try_fold(1usize, |s, &d| s.checked_mul(d))
        .ok_or("image dimensions overflow")?;
    if size > max || dims.iter().any(|&d| d > max) {
        return Err(format!(
            "image exceeds the segmentation workspace limit ({max} samples)"
        ));
    }
    let (class, logical, data) = match v {
        Value::Matrix(m) => (None, false, m.data().to_vec()),
        Value::Logical(m) => (None, true, m.data().iter().map(|&x| f64::from(x)).collect()),
        Value::Container(Array::Typed(t)) => (Some(t.class()), false, t.to_f64_vec_within(max)?),
        _ => unreachable!(),
    };
    Ok(Image {
        rows: dims[0],
        cols: dims[1],
        channels: dims.get(2).copied().unwrap_or(1),
        data,
        class,
        logical,
    })
}
fn numbers(v: &Value) -> Result<Vec<f64>> {
    Ok(image_within(v, false, MAX_PIXELS)?.data)
}
fn scalar(v: &Value) -> Result<f64> {
    let x = numbers(v)?;
    if x.len() != 1 {
        return Err("expected a numeric scalar".into());
    }
    Ok(x[0])
}
fn positive(v: &Value) -> Result<usize> {
    let x = scalar(v)?;
    if !x.is_finite() || x < 1. || x.fract() != 0. || x > MAX_PIXELS as f64 {
        return Err("expected a positive integer within the workspace limit".into());
    }
    Ok(x as usize)
}
fn text(v: &Value) -> Option<String> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(s.to_lowercase()),
        Value::Container(Array::Character(c)) if c.rows() <= 1 => {
            Some(String::from_utf16_lossy(c.units()).to_lowercase())
        }
        _ => None,
    }
}
fn conn(v: Option<&Value>, default: u8) -> Result<u8> {
    match v.map(scalar).transpose()? {
        None => Ok(default),
        Some(4.) => Ok(4),
        Some(8.) => Ok(8),
        _ => Err("connectivity must be 4 or 8".into()),
    }
}
fn arity(a: &[Value], low: usize, high: usize) -> Result<()> {
    if a.len() < low || a.len() > high {
        return Err("wrong number of input arguments".into());
    }
    Ok(())
}
fn finite(i: &Image) -> Result<()> {
    if i.data.iter().any(|x| !x.is_finite()) {
        return Err("image must contain finite values".into());
    }
    Ok(())
}
fn numeric(r: usize, c: usize, x: Vec<f64>, class: Option<NumericClass>) -> Result<Value> {
    Ok(match class {
        Some(k) => Value::Container(Array::Typed(TypedArray::from_reals(
            k,
            r,
            c,
            x.into_iter(),
        )?)),
        None => Value::Matrix(Matrix::from_column_vec(r, c, x)?),
    })
}
fn logical(e: &Engine, r: usize, c: usize, x: &[bool]) -> Result<Value> {
    Ok(Value::Logical(LogicalMatrix::from_column_vec(
        r,
        c,
        x.iter().map(|&b| u8::from(b)).collect(),
        &e.cancel,
    )?))
}
fn poll(e: &Engine, p: usize) -> Result<()> {
    if p.is_multiple_of(4096) {
        e.check_cancel()?;
    }
    Ok(())
}
fn neighbor(p: usize, r: usize, c: usize, d: usize) -> Option<usize> {
    let rr = (p % r).checked_add_signed(DIRS[d].0)?;
    let cc = (p / r).checked_add_signed(DIRS[d].1)?;
    (rr < r && cc < c).then_some(rr + cc * r)
}
fn neighbors(p: usize, r: usize, c: usize, conn: u8) -> impl Iterator<Item = usize> {
    (0..8)
        .step_by(if conn == 4 { 2 } else { 1 })
        .filter_map(move |d| neighbor(p, r, c, d))
}
fn labels(e: &Engine, r: usize, c: usize, fg: &[bool], conn: u8) -> Result<(Vec<usize>, usize)> {
    let mut l = vec![0; fg.len()];
    let mut n = 0;
    let mut stack = Vec::new();
    let mut visits = 0;
    for p in 0..fg.len() {
        poll(e, p)?;
        if !fg[p] || l[p] != 0 {
            continue;
        }
        n += 1;
        l[p] = n;
        stack.push(p);
        while let Some(q) = stack.pop() {
            visits += 1;
            poll(e, visits)?;
            for s in neighbors(q, r, c, conn) {
                if fg[s] && l[s] == 0 {
                    l[s] = n;
                    stack.push(s);
                }
            }
        }
    }
    Ok((l, n))
}

// Lower envelope of parabolas f[q]+(x-q)^2, O(n) time and scratch space.
// Infinite sites never enter the envelope (also handles an empty foreground).
fn edt_line(f: &[f64], source: &[usize], d: &mut [f64], idx: &mut [usize]) {
    let mut sites = Vec::with_capacity(f.len());
    let mut cuts = Vec::with_capacity(f.len());
    for q in 0..f.len() {
        if !f[q].is_finite() {
            continue;
        }
        let mut cut = f64::NEG_INFINITY;
        while let Some(&p) = sites.last() {
            cut =
                ((f[q] + (q as f64).powi(2)) - (f[p] + (p as f64).powi(2))) / (2. * (q - p) as f64);
            if cut > *cuts.last().unwrap() {
                break;
            }
            sites.pop();
            cuts.pop();
        }
        if sites.is_empty() {
            cut = f64::NEG_INFINITY;
        }
        sites.push(q);
        cuts.push(cut);
    }
    if sites.is_empty() {
        d.fill(f64::INFINITY);
        idx.fill(0);
        return;
    }
    let mut k = 0;
    for x in 0..f.len() {
        while k + 1 < sites.len() && cuts[k + 1] < x as f64 {
            k += 1;
        }
        let q = sites[k];
        d[x] = f[q] + (x as f64 - q as f64).powi(2);
        idx[x] = source[q];
    }
}
fn distance(e: &Engine, a: &[Value]) -> Result<Vec<Value>> {
    arity(a, 1, 2)?;
    let im = image(e, &a[0], false)?;
    let method = a
        .get(1)
        .map(|v| text(v).ok_or("method must be text"))
        .transpose()?
        .unwrap_or("euclidean".into());
    if !["euclidean", "cityblock", "chessboard", "quasi-euclidean"].contains(&method.as_str()) {
        return Err("unknown distance method".into());
    }
    let (r, c, n) = (im.rows, im.cols, im.data.len());
    let mut d: Vec<_> = im
        .data
        .iter()
        .map(|&x| if x != 0. { 0. } else { f64::INFINITY })
        .collect();
    let mut idx: Vec<_> = im
        .data
        .iter()
        .enumerate()
        .map(|(p, &x)| if x != 0. { p + 1 } else { 0 })
        .collect();
    if method == "euclidean" && n != 0 {
        let m = r.max(c);
        let mut f = vec![0.; m];
        let mut source = vec![0; m];
        let mut dist = vec![0.; m];
        let mut ix = vec![0; m];
        for col in 0..c {
            poll(e, col * r)?;
            let s = col * r;
            edt_line(&d[s..s + r], &idx[s..s + r], &mut dist[..r], &mut ix[..r]);
            d[s..s + r].copy_from_slice(&dist[..r]);
            idx[s..s + r].copy_from_slice(&ix[..r]);
        }
        for row in 0..r {
            e.check_cancel()?;
            for col in 0..c {
                f[col] = d[row + col * r];
                source[col] = idx[row + col * r];
            }
            edt_line(&f[..c], &source[..c], &mut dist[..c], &mut ix[..c]);
            for col in 0..c {
                d[row + col * r] = dist[col].sqrt();
                idx[row + col * r] = ix[col];
            }
        }
    } else {
        for reverse in [false, true] {
            for step in 0..n {
                poll(e, step)?;
                let p = if reverse { n - 1 - step } else { step };
                for dir in 0..8 {
                    if method == "cityblock" && dir % 2 != 0 {
                        continue;
                    }
                    if let Some(q) = neighbor(p, r, c, dir) {
                        if (q < p) == reverse {
                            continue;
                        }
                        let weight = if method == "quasi-euclidean" && dir % 2 != 0 {
                            std::f64::consts::SQRT_2
                        } else {
                            1.
                        };
                        if d[q] + weight < d[p] {
                            d[p] = d[q] + weight;
                            idx[p] = idx[q];
                        }
                    }
                }
            }
        }
    }
    Ok(vec![
        numeric(r, c, d, Some(NumericClass::Single))?,
        numeric(
            r,
            c,
            idx.iter().map(|&x| x as f64).collect(),
            Some(NumericClass::Uint32),
        )?,
    ])
}

fn gray_connected(e: &Engine, a: &[Value]) -> Result<Value> {
    arity(a, 3, 4)?;
    let im = image(e, &a[0], false)?;
    finite(&im)?;
    if im.logical {
        return Err("intensity image must be numeric".into());
    }
    let row = positive(&a[1])?;
    let col = positive(&a[2])?;
    if row > im.rows || col > im.cols {
        return Err("seed row or column is outside image dimensions".into());
    }
    let tolerance = a.get(3).map(scalar).transpose()?.unwrap_or(
        if im.class.is_some_and(|k| k != NumericClass::Single) {
            32.
        } else {
            0.1
        },
    );
    if !tolerance.is_finite() || tolerance < 0. {
        return Err("tolerance must be finite and nonnegative".into());
    }
    let p = row - 1 + (col - 1) * im.rows;
    let seed = im.data[p];
    let fg: Vec<_> = im
        .data
        .iter()
        .map(|&x| (x - seed).abs() <= tolerance)
        .collect();
    let (l, _) = labels(e, im.rows, im.cols, &fg, 8)?;
    logical(
        e,
        im.rows,
        im.cols,
        &l.iter().map(|&x| x == l[p]).collect::<Vec<_>>(),
    )
}
fn perimeter(e: &Engine, a: &[Value]) -> Result<Value> {
    arity(a, 1, 2)?;
    let im = image(e, &a[0], false)?;
    let conn = conn(a.get(1), 4)?;
    let mut result = vec![false; im.data.len()];
    for (p, b) in result.iter_mut().enumerate() {
        poll(e, p)?;
        *b = im.data[p] != 0.
            && (0..8)
                .step_by(if conn == 4 { 2 } else { 1 })
                .any(|d| neighbor(p, im.rows, im.cols, d).is_none_or(|q| im.data[q] == 0.));
    }
    logical(e, im.rows, im.cols, &result)
}
fn area(e: &Engine, a: &[Value]) -> Result<Value> {
    arity(a, 1, 1)?;
    let im = image(e, &a[0], false)?;
    let mut sum = 0.;
    // Zero padding makes every pixel contribute to exactly four windows.
    for c in 0..=im.cols {
        for r in 0..=im.rows {
            poll(e, r + c * (im.rows + 1))?;
            let at = |rr: usize, cc: usize| {
                rr > 0
                    && cc > 0
                    && rr <= im.rows
                    && cc <= im.cols
                    && im.data[rr - 1 + (cc - 1) * im.rows] != 0.
            };
            let b = [at(r, c), at(r + 1, c), at(r, c + 1), at(r + 1, c + 1)];
            sum += match b.iter().filter(|&&x| x).count() {
                0 => 0.,
                1 => 0.25,
                2 => {
                    if (b[0] && b[3]) || (b[1] && b[2]) {
                        0.75
                    } else {
                        0.5
                    }
                }
                3 => 0.875,
                _ => 1.,
            };
        }
    }
    Ok(Value::Matrix(Matrix::scalar(sum)))
}
fn select(e: &Engine, a: &[Value]) -> Result<Vec<Value>> {
    arity(a, 3, 4)?;
    let im = image(e, &a[0], false)?;
    let connectivity = conn(a.get(3), 8)?;
    let cols = numbers(&a[1])?;
    let rows = numbers(&a[2])?;
    if rows.len() != cols.len() {
        return Err("row and column vectors must have equal lengths".into());
    }
    let fg: Vec<_> = im.data.iter().map(|&x| x != 0.).collect();
    let (l, n) = labels(e, im.rows, im.cols, &fg, connectivity)?;
    let mut keep = vec![false; n + 1];
    for (&r, &c) in rows.iter().zip(&cols) {
        if !r.is_finite() || !c.is_finite() {
            return Err("seed coordinates must be finite".into());
        }
        let (r, c) = (r.round(), c.round());
        if r >= 1. && r <= im.rows as f64 && c >= 1. && c <= im.cols as f64 {
            keep[l[r as usize - 1 + (c as usize - 1) * im.rows]] = true;
        }
    }
    keep[0] = false;
    let mask: Vec<_> = l.iter().map(|&x| keep[x]).collect();
    let ix: Vec<_> = mask
        .iter()
        .enumerate()
        .filter(|(_, b)| **b)
        .map(|(p, _)| (p + 1) as f64)
        .collect();
    Ok(vec![
        logical(e, im.rows, im.cols, &mask)?,
        numeric(ix.len(), 1, ix, None)?,
    ])
}

fn filter(e: &mut Engine, name: &str, a: &[Value], out: &mut Execution) -> Result<Value> {
    arity(a, if name == "bwareafilt" { 2 } else { 3 }, 6)?;
    let im = image(e, &a[0], false)?;
    if !im.logical {
        return Err("BW must be a logical matrix".into());
    }
    let mut at = 1;
    let mut intensity = None;
    let property = if name == "bwareafilt" {
        "area".to_string()
    } else {
        if text(&a[at]).is_none() {
            intensity = Some(a[at].clone());
            at += 1;
        }
        let p = a.get(at).and_then(text).ok_or("property must be text")?;
        at += 1;
        p
    };
    if ![
        "area",
        "extent",
        "equivdiameter",
        "filledarea",
        "majoraxislength",
        "minoraxislength",
        "eccentricity",
        "orientation",
        "perimeter",
        "meanintensity",
        "minintensity",
        "maxintensity",
    ]
    .contains(&property.as_str())
    {
        return Err("unsupported scalar regionprops property".into());
    }
    let choice = numbers(a.get(at).ok_or("missing range or count")?)?;
    at += 1;
    let mut smallest = false;
    if let Some(t) = a.get(at).and_then(text) {
        if choice.len() != 1 || !["largest", "smallest"].contains(&t.as_str()) {
            return Err("keep must be largest or smallest and requires a count".into());
        }
        smallest = t == "smallest";
        at += 1;
    }
    let connectivity = conn(a.get(at), 8)?;
    if at < a.len() {
        at += 1;
    }
    if at != a.len() {
        return Err("too many input arguments".into());
    }
    let count = match choice.as_slice() {
        [n] if n.is_finite() && *n >= 1. && n.fract() == 0. && *n <= MAX_PIXELS as f64 => {
            Some(*n as usize)
        }
        [lo, hi] if !lo.is_nan() && !hi.is_nan() && lo <= hi => None,
        _ => return Err("expected positive count or ordered two-element range".into()),
    };
    let fg: Vec<_> = im.data.iter().map(|&x| x != 0.).collect();
    let (l, n) = labels(e, im.rows, im.cols, &fg, connectivity)?;
    let mut props = vec![numeric(
        im.rows,
        im.cols,
        l.iter().map(|&x| x as f64).collect(),
        None,
    )?];
    if let Some(i) = intensity {
        let check = image(e, &i, false)?;
        if check.rows != im.rows || check.cols != im.cols {
            return Err("intensity image dimensions must match BW".into());
        }
        props.push(i);
    }
    props.push(Value::Text(property));
    let result = e.image_morph_call("regionprops", &props, 1, out)?.remove(0);
    let Value::Container(Array::Struct(s)) = result else {
        unreachable!()
    };
    let mut values = Vec::with_capacity(n);
    for j in 0..n {
        values.push(scalar(&Value::from(s.get(0, j)?.as_ref().clone()))?);
    }
    let mut keep = vec![false; n + 1];
    if let Some(count) = count {
        let mut order: Vec<_> = (0..n).collect();
        // Public docs: at the cutoff keep only the first n objects. Stable
        // ties use bwlabel's column-major discovery order, even for largest.
        order.sort_by(|&i, &j| {
            let cmp = values[i].total_cmp(&values[j]);
            (if smallest { cmp } else { cmp.reverse() }).then(i.cmp(&j))
        });
        for &j in order.iter().take(count) {
            keep[j + 1] = true;
        }
    } else {
        for (j, &v) in values.iter().enumerate() {
            keep[j + 1] = v >= choice[0] && v <= choice[1];
        }
    }
    logical(
        e,
        im.rows,
        im.cols,
        &l.iter().map(|&x| keep[x]).collect::<Vec<_>>(),
    )
}

#[derive(Clone, Copy)]
struct Flood {
    height: f64,
    age: usize,
    pixel: usize,
}
impl PartialEq for Flood {
    fn eq(&self, other: &Self) -> bool {
        self.height.total_cmp(&other.height) == Ordering::Equal && self.age == other.age
    }
}
impl Eq for Flood {}
impl PartialOrd for Flood {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Flood {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .height
            .total_cmp(&self.height)
            .then(other.age.cmp(&self.age))
    }
}
fn watershed(e: &Engine, a: &[Value]) -> Result<Value> {
    arity(a, 1, 2)?;
    let im = image(e, &a[0], false)?;
    if im.data.iter().any(|x| x.is_nan()) {
        return Err("image must not contain NaN".into());
    }
    let conn = conn(a.get(1), 8)?;
    let (r, c, n) = (im.rows, im.cols, im.data.len());
    let mut seen = vec![false; n];
    let mut l = vec![0usize; n];
    let mut stack = Vec::new();
    let mut plateau = Vec::new();
    let mut next = 0;
    let mut visits = 0;
    // Regional minima are entire flat zones, not individual low pixels.
    for p in 0..n {
        poll(e, p)?;
        if seen[p] {
            continue;
        }
        seen[p] = true;
        stack.push(p);
        plateau.clear();
        let mut minimum = true;
        while let Some(q) = stack.pop() {
            visits += 1;
            poll(e, visits)?;
            plateau.push(q);
            for s in neighbors(q, r, c, conn) {
                if im.data[s] < im.data[p] {
                    minimum = false;
                }
                if !seen[s] && im.data[s] == im.data[p] {
                    seen[s] = true;
                    stack.push(s);
                }
            }
        }
        if minimum {
            next += 1;
            for &q in &plateau {
                l[q] = next;
            }
        }
    }
    let mut queued: Vec<_> = l.iter().map(|&x| x != 0).collect();
    let mut heap = BinaryHeap::new();
    let mut age = 0;
    for (p, &label) in l.iter().enumerate() {
        if label > 0 {
            for q in neighbors(p, r, c, conn) {
                if !queued[q] {
                    queued[q] = true;
                    heap.push(Flood {
                        height: im.data[q],
                        age,
                        pixel: q,
                    });
                    age += 1;
                }
            }
        }
    }
    // Meyer's priority flood: a pixel with distinct labelled neighbours is
    // a ridge and does not propagate. FIFO resolves equal-height plateaus.
    while let Some(Flood {
        height, pixel: p, ..
    }) = heap.pop()
    {
        visits += 1;
        poll(e, visits)?;
        let mut label = 0;
        let mut ridge = false;
        for q in neighbors(p, r, c, conn) {
            if l[q] != 0 {
                if label != 0 && label != l[q] {
                    ridge = true;
                    break;
                }
                label = l[q];
            }
        }
        if ridge || label == 0 {
            continue;
        }
        l[p] = label;
        for q in neighbors(p, r, c, conn) {
            if !queued[q] {
                queued[q] = true;
                heap.push(Flood {
                    height: height.max(im.data[q]),
                    age,
                    pixel: q,
                });
                age += 1;
            }
        }
    }
    numeric(r, c, l.into_iter().map(|x| x as f64).collect(), None)
}

fn trace(
    e: &Engine,
    l: &[usize],
    r: usize,
    c: usize,
    id: usize,
    start: usize,
) -> Result<Vec<usize>> {
    let mut path = vec![start];
    let mut p = start;
    let mut back = 6;
    let mut first = None;
    // Jacob's stopping criterion: revisit the starting directed edge, rather
    // than merely the starting pixel (which fails at diagonal junctions).
    for step in 0..=l.len().saturating_mul(8) {
        poll(e, step)?;
        let found = (1..=8).find_map(|offset| {
            let d = (back + offset) % 8;
            neighbor(p, r, c, d).filter(|&q| l[q] == id).map(|q| (q, d))
        });
        let Some((q, d)) = found else {
            path.push(start);
            return Ok(path);
        };
        if first == Some(q) && p == start {
            return Ok(path);
        }
        if first.is_none() {
            first = Some(q);
        }
        // The last examined background neighbour becomes the backtrack point.
        let prev = DIRS[(d + 7) % 8];
        let next = DIRS[d];
        let delta = (prev.0 - next.0, prev.1 - next.1);
        back = DIRS.iter().position(|&v| v == delta).unwrap_or((d + 4) % 8);
        p = q;
        path.push(p);
    }
    Err("boundary tracing exceeded the bounded walk length".into())
}
fn boundaries(e: &Engine, a: &[Value]) -> Result<Vec<Value>> {
    arity(a, 1, 3)?;
    let im = image(e, &a[0], false)?;
    if im.data.len() > MAX_PIXELS / 4 {
        return Err("boundary workspace exceeds the 524288-pixel limit".into());
    }
    let (r, c) = (im.rows, im.cols);
    let mut at = 1;
    let mut connectivity = 8;
    if a.get(at).is_some_and(|v| text(v).is_none()) {
        connectivity = conn(a.get(at), 8)?;
        at += 1;
    }
    let holes = if let Some(v) = a.get(at) {
        at += 1;
        match text(v).as_deref() {
            Some("holes") => true,
            Some("noholes") => false,
            _ => return Err("option must be holes or noholes".into()),
        }
    } else {
        true
    };
    if at != a.len() {
        return Err("too many input arguments".into());
    }
    let fg: Vec<_> = im.data.iter().map(|&x| x != 0.).collect();
    let (mut l, n) = labels(e, r, c, &fg, connectivity)?;
    let bg: Vec<_> = fg.iter().map(|&x| !x).collect();
    let (bl, bn) = labels(e, r, c, &bg, if connectivity == 8 { 4 } else { 8 })?;
    let mut exterior = vec![false; bn + 1];
    for p in 0..l.len() {
        if p % r == 0 || p % r == r - 1 || p / r == 0 || p / r == c - 1 {
            exterior[bl[p]] = true;
        }
    }
    let mut mapping = vec![0; bn + 1];
    let mut total = n;
    for b in 1..=bn {
        if !exterior[b] {
            total += 1;
            mapping[b] = total;
        }
    }
    for p in 0..l.len() {
        if l[p] == 0 {
            l[p] = mapping[bl[p]];
        }
    }
    // Region adjacency is a nesting tree rooted at exterior background.
    // Store sparse edges, never an O(number_of_regions^2) dense matrix.
    let mut adjacency = vec![Vec::new(); total + 1];
    for p in 0..l.len() {
        poll(e, p)?;
        let x = l[p];
        for d in [2, 4] {
            let y = neighbor(p, r, c, d).map_or(0, |q| l[q]);
            if x != y {
                adjacency[x].push(y);
                adjacency[y].push(x);
            }
        }
        if (p % r == 0 || p / r == 0) && x != 0 {
            adjacency[0].push(x);
            adjacency[x].push(0);
        }
    }
    for list in &mut adjacency {
        list.sort_unstable();
        list.dedup();
    }
    let mut parents = vec![usize::MAX; total + 1];
    parents[0] = 0;
    let mut queue = vec![0];
    let mut head = 0;
    while head < queue.len() {
        let p = queue[head];
        head += 1;
        for &q in &adjacency[p] {
            if parents[q] == usize::MAX {
                parents[q] = p;
                queue.push(q);
            }
        }
    }
    let output_n = if holes { total } else { n };
    let mut starts = vec![0; total + 1];
    for p in (0..l.len()).rev() {
        starts[l[p]] = p;
    }
    let mut cells = Vec::with_capacity(output_n);
    let mut entries = Vec::new();
    for id in 1..=output_n {
        e.check_cancel()?;
        let path = trace(e, &l, r, c, id, starts[id])?;
        let mut data: Vec<_> = path.iter().map(|&p| (p % r + 1) as f64).collect();
        data.extend(path.iter().map(|&p| (p / r + 1) as f64));
        cells.push(Arc::new(Array::Numeric(Matrix::from_column_vec(
            path.len(),
            2,
            data,
        )?)));
        let mut parent = parents[id];
        if !holes && parent > n && parent != usize::MAX {
            parent = parents[parent];
        }
        if parent > 0 && parent <= output_n {
            entries.push((id - 1, parent - 1, 1.));
        }
    }
    if !holes {
        for x in &mut l {
            if *x > n {
                *x = 0;
            }
        }
    }
    Ok(vec![
        Value::Container(Array::Cell(CellArray::from_shared(
            output_n, 1, cells, &e.cancel,
        )?)),
        numeric(r, c, l.iter().map(|&x| x as f64).collect(), None)?,
        Value::Matrix(Matrix::scalar(n as f64)),
        Value::Container(Array::Sparse(SparseMatrix::from_triplets(
            output_n, output_n, &entries, true, &e.cancel,
        )?)),
    ])
}

fn segment_kmeans(e: &mut Engine, a: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
    arity(a, 2, 10)?;
    let im = image(e, &a[0], true)?;
    finite(&im)?;
    if !matches!(
        im.class,
        Some(
            NumericClass::Uint8
                | NumericClass::Uint16
                | NumericClass::Int8
                | NumericClass::Int16
                | NumericClass::Single
        )
    ) {
        return Err(
            "I must be single, uint8, uint16, int8 or int16 (convert double with im2single)".into(),
        );
    }
    let k = positive(&a[1])?;
    let n = im.rows * im.cols;
    if n == 0 || im.channels == 0 || k > n {
        return Err("k must not exceed the number of pixels".into());
    }
    if n.saturating_mul(k) > MAX_PIXELS {
        return Err("pixel-by-cluster workspace exceeds segmentation limit".into());
    }
    let mut attempts = 3;
    let mut iterations = 100;
    let mut threshold = 1e-4;
    let mut normalize = true;
    if !a.len().is_multiple_of(2) {
        return Err("options must be name-value pairs".into());
    }
    for pair in a[2..].as_chunks::<2>().0 {
        match text(&pair[0]).as_deref() {
            Some("numattempts") => attempts = positive(&pair[1])?,
            Some("maxiterations") => iterations = positive(&pair[1])?,
            Some("threshold") => {
                threshold = scalar(&pair[1])?;
                if !threshold.is_finite() || threshold <= 0. {
                    return Err("Threshold must be positive and finite".into());
                }
            }
            Some("normalizeinput") => {
                normalize = match scalar(&pair[1])? {
                    0. => false,
                    1. => true,
                    _ => return Err("NormalizeInput must be logical scalar".into()),
                };
            }
            _ => return Err("unknown option".into()),
        }
    }
    let mut data = im.data;
    let mut means = vec![0.; im.channels];
    let mut scales = vec![1.; im.channels];
    if normalize {
        for ch in 0..im.channels {
            e.check_cancel()?;
            let slice = &mut data[ch * n..(ch + 1) * n];
            let mean = slice.iter().sum::<f64>() / n as f64;
            let sd = (slice.iter().map(|x| (x - mean).powi(2)).sum::<f64>()
                / n.saturating_sub(1).max(1) as f64)
                .sqrt();
            means[ch] = mean;
            scales[ch] = if sd > 0. { sd } else { 1. };
            for x in slice {
                *x = (*x - mean) / scales[ch];
            }
        }
    }
    let args = [
        numeric(n, im.channels, data, None)?,
        Value::Matrix(Matrix::scalar(k as f64)),
        Value::Text("Replicates".into()),
        Value::Matrix(Matrix::scalar(attempts as f64)),
        Value::Text("MaxIter".into()),
        Value::Matrix(Matrix::scalar(iterations as f64)),
    ];
    let mut result = super::stats_models::image_segment_kmeans(e, &args, threshold, out)?;
    let center = image(e, &result.remove(1), false)?;
    let labels = image(e, &result.remove(0), false)?;
    let mut centers = center.data;
    for ch in 0..im.channels {
        for j in 0..k {
            centers[j + ch * k] = centers[j + ch * k] * scales[ch] + means[ch];
        }
    }
    let label_class = if k <= 255 {
        NumericClass::Uint8
    } else if k <= 65535 {
        NumericClass::Uint16
    } else {
        NumericClass::Uint32
    };
    Ok(vec![
        numeric(im.rows, im.cols, labels.data, Some(label_class))?,
        numeric(k, im.channels, centers, im.class)?,
    ])
}

fn active_contour(e: &Engine, a: &[Value]) -> Result<Value> {
    arity(a, 2, 8)?;
    let mut im = image(e, &a[0], true)?;
    finite(&im)?;
    let mask = image(e, &a[1], false)?;
    // The public grayscale examples construct masks with zeros(), although
    // the type table lists logical. Accept numeric binary grayscale masks;
    // the multichannel contract explicitly requires logical masks.
    if !mask.logical && (im.channels != 1 || mask.data.iter().any(|&x| x != 0. && x != 1.)) {
        return Err("mask must be binary (logical for multichannel images)".into());
    }
    if im.rows != mask.rows || im.cols != mask.cols || im.channels == 0 {
        return Err("mask dimensions must match image".into());
    }
    let mut at = 2;
    let mut iterations = 100;
    if a.get(at).is_some_and(|v| text(v).is_none()) {
        iterations = positive(&a[at])?;
        at += 1;
    }
    let mut edge = false;
    if let Some(t) = a.get(at).and_then(text) {
        if t == "edge" || t == "chan-vese" {
            edge = t == "edge";
            at += 1;
        } else if t != "smoothfactor" && t != "contractionbias" {
            return Err("method must be Chan-Vese or edge".into());
        }
    }
    if edge && im.channels != 1 {
        return Err("edge method requires a 2-D grayscale image".into());
    }
    let mut smooth = if edge { 1. } else { 0. };
    let mut bias = if edge { 0.3 } else { 0. };
    if !(a.len() - at).is_multiple_of(2) {
        return Err("options must be name-value pairs".into());
    }
    for pair in a[at..].as_chunks::<2>().0 {
        let v = scalar(&pair[1])?;
        if !v.is_finite() {
            return Err("options must be finite".into());
        }
        match text(&pair[0]).as_deref() {
            Some("smoothfactor") if v >= 0. => smooth = v,
            Some("contractionbias") => bias = v,
            _ => return Err("unknown option or invalid SmoothFactor".into()),
        }
    }
    let (r, c, n) = (im.rows, im.cols, mask.data.len());
    let mut inside: Vec<_> = mask.data.iter().map(|&x| x != 0.).collect();
    if n == 0 {
        return logical(e, r, c, &inside);
    }
    for (p, b) in inside.iter_mut().enumerate() {
        if p % r == 0 || p % r == r - 1 || p / r == 0 || p / r == c - 1 {
            *b = false;
        }
    }
    let lo = im.data.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = im.data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let scale = hi.abs().max(lo.abs()).max(f64::MIN_POSITIVE);
    let low = lo / scale;
    let range = (hi / scale - low).max(f64::MIN_POSITIVE);
    for x in &mut im.data {
        *x = (*x / scale - low) / range;
    }
    if edge {
        return edge_contour(e, &im, inside, iterations, smooth, bias);
    }
    let mut history = std::collections::VecDeque::new();
    for _ in 0..iterations {
        e.check_cancel()?;
        let count = inside.iter().filter(|&&b| b).count();
        if count == 0 || count == n {
            break;
        }
        let mut mean_in = vec![0.; im.channels];
        let mut mean_out = vec![0.; im.channels];
        for ch in 0..im.channels {
            for (p, &b) in inside.iter().enumerate() {
                if b {
                    mean_in[ch] += im.data[p + ch * n];
                } else {
                    mean_out[ch] += im.data[p + ch * n];
                }
            }
            mean_in[ch] /= count as f64;
            mean_out[ch] /= (n - count) as f64;
        }
        let mut next = inside.clone();
        // Narrow-band descent of Chan/Vese's two-region energy. Only the
        // contour and its immediate neighbours can move in one iteration.
        for p in 0..n {
            poll(e, p)?;
            if p % r == 0 || p % r == r - 1 || p / r == 0 || p / r == c - 1 {
                continue;
            }
            let adjacent = neighbors(p, r, c, 8).any(|q| inside[q] != inside[p]);
            if !adjacent {
                continue;
            }
            let mut force = 0.;
            for ch in 0..im.channels {
                let x = im.data[p + ch * n];
                force += (x - mean_out[ch]).powi(2) - (x - mean_in[ch]).powi(2);
            }
            force /= im.channels as f64;
            let nearby = neighbors(p, r, c, 8).filter(|&q| inside[q]).count() as f64;
            force += smooth * (nearby / 4. - 1.) - bias;
            if force > 1e-12 {
                next[p] = true;
            } else if force < -1e-12 {
                next[p] = false;
            }
        }
        if next == inside || history.contains(&next) {
            inside = next;
            break;
        }
        history.push_back(inside);
        if history.len() > 5 {
            history.pop_front();
        }
        inside = next;
    }
    logical(e, r, c, &inside)
}

fn edge_contour(
    e: &Engine,
    im: &Image,
    inside: Vec<bool>,
    iterations: usize,
    smooth: f64,
    bias: f64,
) -> Result<Value> {
    let (r, c, n) = (im.rows, im.cols, inside.len());
    let at = |x: &[f64], p: usize, dr: isize, dc: isize| {
        let rr = (p % r).saturating_add_signed(dr).min(r - 1);
        let cc = (p / r).saturating_add_signed(dc).min(c - 1);
        x[rr + cc * r]
    };
    // A small separable Gaussian regularises the gradient driving the
    // geodesic active contour. phi<0 denotes the evolving interior.
    let mut blurred = vec![0.; n];
    let mut temp = vec![0.; n];
    for (p, value) in temp.iter_mut().enumerate() {
        *value = (at(&im.data, p, -1, 0) + 2. * im.data[p] + at(&im.data, p, 1, 0)) / 4.;
    }
    for p in 0..n {
        blurred[p] = (at(&temp, p, 0, -1) + 2. * temp[p] + at(&temp, p, 0, 1)) / 4.;
    }
    let mut g = vec![0.; n];
    for (p, x) in g.iter_mut().enumerate() {
        let dx = (at(&blurred, p, 0, 1) - at(&blurred, p, 0, -1)) / 2.;
        let dy = (at(&blurred, p, 1, 0) - at(&blurred, p, -1, 0)) / 2.;
        *x = 1. / (1. + 400. * (dx * dx + dy * dy));
    }
    let v = logical(e, r, c, &inside)?;
    let inv: Vec<_> = inside.iter().map(|&b| !b).collect();
    let outside_dist = image(e, &distance(e, &[v])?.remove(0), false)?.data;
    let inside_dist = image(
        e,
        &distance(e, &[logical(e, r, c, &inv)?])?.remove(0),
        false,
    )?
    .data;
    if outside_dist.iter().any(|x| !x.is_finite()) || inside_dist.iter().any(|x| !x.is_finite()) {
        return logical(e, r, c, &inside);
    }
    let mut phi: Vec<_> = outside_dist
        .iter()
        .zip(&inside_dist)
        .map(|(a, b)| if *a > 0. { a - 0.5 } else { 0.5 - b })
        .collect();
    let mut force = vec![0.; n];
    // Explicit finite differences for phi_t = g*(mu*kappa+bias)*|grad phi|
    // + grad(g).grad(phi), with a CFL-limited step and fixed exterior border.
    for _ in 0..iterations {
        e.check_cancel()?;
        let mut max_force: f64 = 0.;
        for p in 0..n {
            poll(e, p)?;
            let dxm = phi[p] - at(&phi, p, 0, -1);
            let dxp = at(&phi, p, 0, 1) - phi[p];
            let dym = phi[p] - at(&phi, p, -1, 0);
            let dyp = at(&phi, p, 1, 0) - phi[p];
            let dx = (dxp + dxm) / 2.;
            let dy = (dyp + dym) / 2.;
            let dxx = at(&phi, p, 0, 1) + at(&phi, p, 0, -1) - 2. * phi[p];
            let dyy = at(&phi, p, 1, 0) + at(&phi, p, -1, 0) - 2. * phi[p];
            let dxy =
                (at(&phi, p, 1, 1) + at(&phi, p, -1, -1) - at(&phi, p, 1, -1) - at(&phi, p, -1, 1))
                    / 4.;
            let curvature_speed =
                (dxx * dy * dy + dyy * dx * dx - 2. * dx * dy * dxy) / (dx * dx + dy * dy + 1e-12);
            // Godunov derivatives for normal propagation. Central gradients
            // in this Hamilton-Jacobi term create spurious interior contours.
            let norm = if bias >= 0. {
                (dxm.min(0.).powi(2)
                    + dxp.max(0.).powi(2)
                    + dym.min(0.).powi(2)
                    + dyp.max(0.).powi(2))
                .sqrt()
            } else {
                (dxm.max(0.).powi(2)
                    + dxp.min(0.).powi(2)
                    + dym.max(0.).powi(2)
                    + dyp.min(0.).powi(2))
                .sqrt()
            };
            let gx = (at(&g, p, 0, 1) - at(&g, p, 0, -1)) / 2.;
            let gy = (at(&g, p, 1, 0) - at(&g, p, -1, 0)) / 2.;
            // Upwind advection velocity is -grad(g), directed towards edges.
            let advection =
                gx * if gx > 0. { dxp } else { dxm } + gy * if gy > 0. { dyp } else { dym };
            force[p] = g[p] * (smooth * curvature_speed + bias * norm) + advection;
            max_force = max_force.max(force[p].abs());
        }
        if max_force < 1e-12 {
            break;
        }
        let dt = (0.2 / (smooth + bias.abs() + 1.)).min(0.45 / max_force);
        for p in 0..n {
            phi[p] += dt * force[p];
            if p % r == 0 || p % r == r - 1 || p / r == 0 || p / r == c - 1 {
                phi[p] = phi[p].max(1.);
            }
        }
    }
    logical(e, r, c, &phi.iter().map(|&x| x < 0.).collect::<Vec<_>>())
}
