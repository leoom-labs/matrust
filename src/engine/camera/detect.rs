//! detectCheckerboardPoints: checkerboard inner corners in images, to sub-pixel
//! accuracy, in the order of generateCheckerboardPoints (engine 7d2).
//!
//! Lives in its own file because the camera module (src/engine/camera/, 7d1) is
//! built in parallel; it can move there once both are merged.
//!
//! Method (published techniques, no MathWorks code):
//! 1. Grey image on 0..1 (class range; truecolor by the rgb2gray weights),
//!    Gaussian smoothing (sigma 1.2) and the Hessian saddle response
//!    Ixy^2 - Ixx*Iyy (positive only where the determinant is negative, i.e. at
//!    saddle points such as X-junctions), local maxima above a relative and an
//!    absolute floor. Steps 1 to 3 run on the image and on copies reduced 2, 4
//!    and 8 times (2-by-2 means, while the smaller side keeps 150 px): soft
//!    and high-resolution images are only whole on a reduced copy. The grid
//!    with the most corners wins, the finer size on a tie.
//! 2. Ring test: on a circle of radius 4 px the smoothed intensity must cross
//!    its mid level exactly four times with opposite crossings about pi apart
//!    (two straight edges through the point); the crossings give the two edge
//!    directions.
//! 3. Grid growing (after A. Geiger et al., "Automatic camera and range sensor
//!    calibration using a single shot", ICRA 2012, simplified): a 3x3 seed from a
//!    corner and its neighbours along both edge directions, then whole rows and
//!    columns added while every extrapolated position (quadratic, so perspective
//!    shrinkage is followed) has an unused candidate nearby. The largest grid of
//!    at least 3x3 corners whose cells alternate dark and bright (each cell
//!    against the cells next to it, so uneven lighting is allowed) wins.
//! 4. Sub-pixel position by gradient orthogonality (W. Foerstner and E. Guelch,
//!    1987; the criterion of OpenCV's cornerSubPix): the corner q minimises
//!    sum w (g . (p - q))^2 over a Gaussian-weighted window, iterated, on the
//!    full-size image. The window half-width is 0.3 of the smallest corner
//!    spacing (2 to 48 px), smaller for a corner close to the image border.
//! 5. Orientation: x runs along the longer side of the board (more squares); the
//!    origin square is black and the frame is right-handed in the image (y is x
//!    turned 90 degrees clockwise on screen). Boards with one even and one odd
//!    side are thereby unique; for the others the origin is the candidate corner
//!    closest to the image's top-left.
use crate::engine::{Engine, Execution, Value};
use crate::{Array, LogicalMatrix, Matrix, NumericClass, Result};

const NAME: &str = "detectCheckerboardPoints";
const RING_RADIUS: f64 = 4.0;
const RING_SAMPLES: usize = 48;
const SMOOTH_SIGMA: f64 = 1.2;
/// Most saddle candidates kept per image (strongest first).
const MAX_CANDIDATES: usize = 3000;
/// Seeds tried per image.
const MAX_SEEDS: usize = 60;
/// Smallest ring contrast (image on 0..1) for a corner.
const MIN_CONTRAST: f64 = 0.04;
/// Largest reduction of the image the grid is searched at.
const MAX_REDUCTION: usize = 8;
/// A reduced copy is searched while its smaller side has this many pixels.
const MIN_REDUCED_SIDE: usize = 150;
/// Largest half-width of the sub-pixel refinement window, in pixels.
const MAX_REFINE_HALF: usize = 48;

pub(in crate::engine) fn handles(name: &str) -> bool {
    name == NAME
}

pub(in crate::engine) fn call(
    e: &mut Engine,
    _name: &str,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    e.check_cancel()?;
    if args.is_empty() {
        return Err(format!("{NAME}: not enough input arguments"));
    }
    // Options: name-value pairs after the image argument.
    let mut rest = &args[1..];
    if rest.len() % 2 == 1 {
        if crate::engine::image_basic::text(e, &rest[0]).is_none() {
            return Err(format!(
                "{NAME}: stereo image pairs (two image sets) are not supported yet"
            ));
        }
        return Err(format!("{NAME}: name-value arguments must come in pairs"));
    }
    while let [key, value, tail @ ..] = rest {
        let Some(key) = crate::engine::image_basic::text(e, key) else {
            return Err(format!(
                "{NAME}: stereo image pairs (two image sets) are not supported yet"
            ));
        };
        match key.to_ascii_lowercase().as_str() {
            "partialdetections" | "highdistortion" => {
                flag(value).ok_or_else(|| format!("{NAME}: {key} must be true or false"))?;
            }
            "mincornermetric" => {
                return Err(format!("{NAME}: MinCornerMetric is not supported yet"));
            }
            _ => return Err(format!("{NAME}: unknown option '{key}'")),
        }
        rest = tail;
    }
    // File names: char row, cellstr or string array.
    let first = &args[0];
    let is_text = matches!(
        first,
        Value::Text(_)
            | Value::QuotedString(_)
            | Value::Container(Array::Character(_) | Array::Str(_) | Array::Cell(_))
    );
    if is_text {
        let names = crate::engine::table_fns::text_list(e, first, "image file names")?
            .ok_or_else(|| format!("{NAME}: image file names must be text"))?;
        if count > 3 {
            return Err(format!("{NAME}: too many output arguments"));
        }
        let mut found = Vec::with_capacity(names.len());
        for name in &names {
            let image = e
                .fl2_call("imread", &[Value::Text(name.clone())], out)?
                .ok_or_else(|| format!("{NAME}: cannot read '{name}'"))?;
            let images = grey_images(e, &image, true)?;
            if images.len() != 1 {
                return Err(format!("{NAME}: '{name}' is not a single image"));
            }
            found.push(detect(e, &images[0])?);
        }
        return multi_outputs(e, &found, count);
    }
    let dims = first.dims().into_owned();
    let single = dims.len() == 2 || (dims.len() == 3 && dims[2] == 3);
    let images = grey_images(e, first, single)?;
    if single {
        if count > 2 {
            return Err(format!(
                "{NAME}: too many output arguments; imagesUsed is returned for image stacks and file names"
            ));
        }
        let board = detect(e, &images[0])?;
        return Ok(match board {
            Some(b) => vec![
                points_value(&[&b])?,
                row(&[b.rows as f64 + 1., b.cols as f64 + 1.])?,
            ],
            None => vec![
                Value::Matrix(Matrix::from_column_vec(0, 0, Vec::new())?),
                row(&[0., 0.])?,
            ],
        });
    }
    if count > 3 {
        return Err(format!("{NAME}: too many output arguments"));
    }
    let mut found = Vec::with_capacity(images.len());
    for image in &images {
        found.push(detect(e, image)?);
    }
    multi_outputs(e, &found, count)
}

fn flag(value: &Value) -> Option<bool> {
    let v = crate::engine::image_basic::numbers(value)?;
    match v.as_slice() {
        [x] if *x == 0. || *x == 1. => Some(*x == 1.),
        _ => None,
    }
}

fn row(v: &[f64]) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(
        1,
        v.len(),
        v.to_vec(),
    )?))
}

/// M-by-2(-by-N) points, 1-based pixel coordinates.
fn points_value(boards: &[&Board]) -> Result<Value> {
    let m = boards[0].points.len();
    let n = boards.len();
    let mut data = Vec::with_capacity(m * 2 * n);
    for b in boards {
        data.extend(b.points.iter().map(|p| p.0 + 1.));
        data.extend(b.points.iter().map(|p| p.1 + 1.));
    }
    let matrix = Matrix::from_column_vec(m, 2 * n, data)?;
    Ok(Value::Matrix(if n > 1 {
        matrix.with_dims(&[m, 2, n])?
    } else {
        matrix
    }))
}

/// Outputs of the many-image forms: the most common board size wins (first
/// seen on a tie); images with another size or none are not used.
fn multi_outputs(e: &Engine, found: &[Option<Board>], count: usize) -> Result<Vec<Value>> {
    let mut sizes: Vec<((usize, usize), usize)> = Vec::new();
    for b in found.iter().flatten() {
        let key = (b.rows, b.cols);
        match sizes.iter_mut().find(|(k, _)| *k == key) {
            Some((_, c)) => *c += 1,
            None => sizes.push((key, 1)),
        }
    }
    let best = sizes
        .iter()
        .fold(None::<((usize, usize), usize)>, |acc, &(k, c)| match acc {
            Some((_, bc)) if bc >= c => acc,
            _ => Some((k, c)),
        })
        .map(|(k, _)| k);
    let used: Vec<bool> = found
        .iter()
        .map(|b| b.as_ref().is_some_and(|b| Some((b.rows, b.cols)) == best))
        .collect();
    let boards: Vec<&Board> = found
        .iter()
        .flatten()
        .filter(|b| Some((b.rows, b.cols)) == best)
        .collect();
    let mut outputs = match best {
        Some((r, c)) => vec![
            points_value(&boards)?,
            row(&[r as f64 + 1., c as f64 + 1.])?,
        ],
        None => vec![
            Value::Matrix(Matrix::from_column_vec(0, 0, Vec::new())?),
            row(&[0., 0.])?,
        ],
    };
    if count > 2 {
        outputs.push(Value::Logical(LogicalMatrix::from_column_vec(
            used.len(),
            1,
            used.iter().map(|&u| u8::from(u)).collect(),
            &e.cancel,
        )?));
    }
    Ok(outputs)
}

/// A grey image on 0..1, column-major (index r + c*h).
struct Grey {
    h: usize,
    w: usize,
    data: Vec<f64>,
}

impl Grey {
    #[inline]
    fn at(&self, r: usize, c: usize) -> f64 {
        self.data[r + c * self.h]
    }
    /// Bilinear sample at x (column) and y (row), 0-based pixel centres;
    /// clamped to the border.
    fn sample(&self, x: f64, y: f64) -> f64 {
        let x = x.clamp(0., (self.w - 1) as f64);
        let y = y.clamp(0., (self.h - 1) as f64);
        let (c0, r0) = (x.floor() as usize, y.floor() as usize);
        let (c1, r1) = ((c0 + 1).min(self.w - 1), (r0 + 1).min(self.h - 1));
        let (fx, fy) = (x - c0 as f64, y - r0 as f64);
        let top = self.at(r0, c0) * (1. - fx) + self.at(r0, c1) * fx;
        let bottom = self.at(r1, c0) * (1. - fx) + self.at(r1, c1) * fx;
        top * (1. - fy) + bottom * fy
    }
}

/// Splits a numeric image argument into grey images on 0..1. `single`: one
/// H-by-W or H-by-W-by-3 image; otherwise H-by-W-by-N, H-by-W-by-1-by-N or
/// H-by-W-by-3-by-N.
fn grey_images(e: &Engine, value: &Value, single: bool) -> Result<Vec<Grey>> {
    let dims = value.dims().into_owned();
    let (scale, offset) = match value {
        Value::Matrix(_) | Value::Container(Array::Numeric(_)) => (1., 0.),
        Value::Logical(_) | Value::Container(Array::Logical(_)) => (1., 0.),
        Value::Container(Array::Typed(t)) => match t.class() {
            NumericClass::Single => (1., 0.),
            NumericClass::Uint8 => (255., 0.),
            NumericClass::Uint16 => (65535., 0.),
            NumericClass::Int16 => (65535., 32768.),
            other => {
                return Err(format!("{NAME}: {} images are not supported", other.name()));
            }
        },
        Value::Complex(_) | Value::Container(Array::Complex(_)) => {
            return Err(format!("{NAME}: complex images are not supported"));
        }
        _ => {
            return Err(format!(
                "{NAME}: expected an image, image stack or file names"
            ));
        }
    };
    if dims.len() > 4 || dims.len() < 2 {
        return Err(format!(
            "{NAME}: expected an image, image stack or file names"
        ));
    }
    let (h, w) = (dims[0], dims[1]);
    let d3 = dims.get(2).copied().unwrap_or(1);
    let d4 = dims.get(3).copied().unwrap_or(1);
    let (channels, count) = if single {
        (d3, 1)
    } else if dims.len() == 4 {
        if d3 != 1 && d3 != 3 {
            return Err(format!(
                "{NAME}: a 4-D image stack must be H-by-W-by-1-by-N (grayscale) or H-by-W-by-3-by-N (truecolor)"
            ));
        }
        (d3, d4)
    } else {
        (1, d3)
    };
    if h == 0 || w == 0 || count == 0 {
        return Err(format!("{NAME}: the image must not be empty"));
    }
    let max = crate::engine::limits::memory_units(&e.limits);
    let plane = h.saturating_mul(w);
    // Conversion holds the double input and all grey planes; detection then
    // holds the grey set, smoothing, response/gradients and reduced copies.
    let conversion = plane
        .saturating_mul(count)
        .saturating_mul(channels.saturating_add(1));
    let detection = plane.saturating_mul(count.saturating_add(4));
    if conversion.max(detection) > max {
        return Err(crate::engine::limits::memory_error(
            &format!("{NAME}: the {h}-by-{w} image set"),
            &e.limits,
        ));
    }
    let data = crate::engine::image_basic::numbers(value)
        .ok_or_else(|| format!("{NAME}: expected a numeric or logical image"))?;
    let plane = h * w;
    let mut out = Vec::with_capacity(count);
    for k in 0..count {
        let base = k * plane * channels;
        let mut g = Vec::with_capacity(plane);
        for i in 0..plane {
            let v = if channels == 3 {
                0.298_936_021_293_775_6 * data[base + i]
                    + 0.587_043_074_451_121_1 * data[base + plane + i]
                    + 0.114_020_904_255_103_3 * data[base + 2 * plane + i]
            } else {
                data[base + i]
            };
            let v = (v + offset) / scale;
            g.push(if v.is_finite() { v } else { 0. });
        }
        crate::engine::image_basic::cancelled(&e.cancel)?;
        out.push(Grey { h, w, data: g });
    }
    Ok(out)
}

/// A detected board: `rows` x `cols` inner corners (y by x), points in
/// generateCheckerboardPoints order (y fastest), 0-based (x, y).
#[derive(Clone, Debug)]
struct Board {
    rows: usize,
    cols: usize,
    points: Vec<(f64, f64)>,
}

/// Separable Gaussian smoothing with replicated borders.
fn smooth(img: &Grey, sigma: f64) -> Grey {
    let radius = (3. * sigma).ceil() as isize;
    let kernel: Vec<f64> = (-radius..=radius)
        .map(|i| (-((i * i) as f64) / (2. * sigma * sigma)).exp())
        .collect();
    let sum: f64 = kernel.iter().sum();
    let kernel: Vec<f64> = kernel.iter().map(|k| k / sum).collect();
    let (h, w) = (img.h as isize, img.w as isize);
    let mut tmp = vec![0.; img.data.len()];
    for c in 0..w {
        for r in 0..h {
            let mut acc = 0.;
            for (k, kv) in kernel.iter().enumerate() {
                let rr = (r + k as isize - radius).clamp(0, h - 1);
                acc += kv * img.data[(rr + c * h) as usize];
            }
            tmp[(r + c * h) as usize] = acc;
        }
    }
    let mut out = vec![0.; img.data.len()];
    for c in 0..w {
        for r in 0..h {
            let mut acc = 0.;
            for (k, kv) in kernel.iter().enumerate() {
                let cc = (c + k as isize - radius).clamp(0, w - 1);
                acc += kv * tmp[(r + cc * h) as usize];
            }
            out[(r + c * h) as usize] = acc;
        }
    }
    Grey {
        h: img.h,
        w: img.w,
        data: out,
    }
}

/// A saddle candidate: position (x, y) and edge directions (angles mod pi).
#[derive(Clone, Debug)]
struct Candidate {
    x: f64,
    y: f64,
    score: f64,
    edges: [f64; 2],
    contrast: f64,
}

/// Saddle response, local maxima, ring test.
fn candidates(e: &Engine, s: &Grey) -> Result<Vec<Candidate>> {
    let (h, w) = (s.h, s.w);
    let mut resp = vec![0.; h * w];
    let mut max = 0f64;
    for c in 1..w.saturating_sub(1) {
        for r in 1..h.saturating_sub(1) {
            let ixx = s.at(r, c + 1) - 2. * s.at(r, c) + s.at(r, c - 1);
            let iyy = s.at(r + 1, c) - 2. * s.at(r, c) + s.at(r - 1, c);
            let ixy = (s.at(r + 1, c + 1) - s.at(r + 1, c - 1) - s.at(r - 1, c + 1)
                + s.at(r - 1, c - 1))
                / 4.;
            let v = (ixy * ixy - ixx * iyy).max(0.);
            resp[r + c * h] = v;
            max = max.max(v);
        }
        crate::engine::image_basic::poll_column(e, c, h)?;
    }
    let floor = (0.01 * max).max(1e-7);
    let margin = (RING_RADIUS as usize) + 2;
    let mut peaks = Vec::new();
    if w <= 2 * margin || h <= 2 * margin {
        return Ok(Vec::new());
    }
    for c in margin..w - margin {
        for r in margin..h - margin {
            let v = resp[r + c * h];
            if v <= floor {
                continue;
            }
            let mut is_max = true;
            'n: for dc in -2isize..=2 {
                for dr in -2isize..=2 {
                    if dc == 0 && dr == 0 {
                        continue;
                    }
                    let (rr, cc) = ((r as isize + dr) as usize, (c as isize + dc) as usize);
                    let o = resp[rr + cc * h];
                    // Ties go to the first in column-major order.
                    if o > v || (o == v && (dc < 0 || (dc == 0 && dr < 0))) {
                        is_max = false;
                        break 'n;
                    }
                }
            }
            if is_max {
                // Quadratic peak refinement of the response.
                let at = |dr: isize, dc: isize| {
                    resp[(r as isize + dr) as usize + (c as isize + dc) as usize * h]
                };
                let dx = 0.5 * (at(0, 1) - at(0, -1)) / (2. * v - at(0, 1) - at(0, -1)).max(1e-30);
                let dy = 0.5 * (at(1, 0) - at(-1, 0)) / (2. * v - at(1, 0) - at(-1, 0)).max(1e-30);
                peaks.push((
                    c as f64 + dx.clamp(-0.5, 0.5),
                    r as f64 + dy.clamp(-0.5, 0.5),
                    v,
                ));
            }
        }
        crate::engine::image_basic::poll_column(e, c, h)?;
    }
    peaks.sort_by(|a, b| b.2.total_cmp(&a.2));
    peaks.truncate(MAX_CANDIDATES);
    let mut out = Vec::new();
    for (x, y, score) in peaks {
        if let Some((edges, contrast)) = ring(s, x, y) {
            out.push(Candidate {
                x,
                y,
                score,
                edges,
                contrast,
            });
        }
    }
    Ok(out)
}

/// Ring test: four mid-level crossings, opposite ones about pi apart. Returns
/// the two edge angles (mod pi) and the ring contrast.
fn ring(s: &Grey, x: f64, y: f64) -> Option<([f64; 2], f64)> {
    use std::f64::consts::PI;
    let n = RING_SAMPLES;
    let step = 2. * PI / n as f64;
    let v: Vec<f64> = (0..n)
        .map(|k| {
            let a = k as f64 * step;
            s.sample(x + RING_RADIUS * a.cos(), y + RING_RADIUS * a.sin())
        })
        .collect();
    let (lo, hi) = v
        .iter()
        .fold((f64::MAX, f64::MIN), |(l, h), &x| (l.min(x), h.max(x)));
    let amp = hi - lo;
    if amp < MIN_CONTRAST {
        return None;
    }
    let mid = 0.5 * (hi + lo);
    let band = 0.15 * amp;
    // Labels with hysteresis; zeros take no part in sign changes.
    let label: Vec<i8> = v
        .iter()
        .map(|&x| {
            if x > mid + band {
                1
            } else if x < mid - band {
                -1
            } else {
                0
            }
        })
        .collect();
    let start = label.iter().position(|&l| l != 0)?;
    let mut crossings = Vec::new();
    let mut prev = start;
    for i in 1..=n {
        let k = (start + i) % n;
        if label[k] == 0 {
            continue;
        }
        if label[k] != label[prev] {
            // First mid crossing between prev and k (circular).
            let mut a = prev;
            loop {
                let b = (a + 1) % n;
                let (va, vb) = (v[a] - mid, v[b] - mid);
                if va == 0. || va.signum() != vb.signum() {
                    let f = if va == vb { 0. } else { va / (va - vb) };
                    let idx = a as f64 + f.clamp(0., 1.);
                    crossings.push(idx * step);
                    break;
                }
                a = b;
                if a == k {
                    crossings.push(k as f64 * step);
                    break;
                }
            }
        }
        prev = k;
    }
    if crossings.len() != 4 {
        return None;
    }
    crossings.sort_by(f64::total_cmp);
    let wrap = |a: f64| a.rem_euclid(2. * PI);
    let d02 = wrap(crossings[2] - crossings[0]);
    let d13 = wrap(crossings[3] - crossings[1]);
    if (d02 - PI).abs() > 0.5 || (d13 - PI).abs() > 0.5 {
        return None;
    }
    let mean_mod_pi = |a: f64, b: f64| {
        // Average of two angles taken mod pi (doubled-angle mean).
        let (s, c) = (
            (2. * a).sin() + (2. * b).sin(),
            (2. * a).cos() + (2. * b).cos(),
        );
        (s.atan2(c) / 2.).rem_euclid(PI)
    };
    let e1 = mean_mod_pi(crossings[0], crossings[2]);
    let e2 = mean_mod_pi(crossings[1], crossings[3]);
    let between = (e1 - e2).rem_euclid(PI);
    let between = between.min(PI - between);
    if between < 0.35 {
        return None;
    }
    Some(([e1, e2], amp))
}

fn angle_diff_mod_pi(a: f64, b: f64) -> f64 {
    use std::f64::consts::PI;
    let d = (a - b).rem_euclid(PI);
    d.min(PI - d)
}

/// Nearest candidate from `from` in direction `dir` (radians), within 15
/// degrees, which has an edge along the step.
fn neighbour(c: &[Candidate], from: usize, dir: f64, used: &[bool]) -> Option<usize> {
    let (x0, y0) = (c[from].x, c[from].y);
    let (dx, dy) = (dir.cos(), dir.sin());
    let mut best: Option<(usize, f64)> = None;
    for (i, cand) in c.iter().enumerate() {
        if i == from || used[i] {
            continue;
        }
        let (vx, vy) = (cand.x - x0, cand.y - y0);
        let d = (vx * vx + vy * vy).sqrt();
        if d < 2. * RING_RADIUS {
            continue;
        }
        if (vx * dx + vy * dy) / d < 0.966 {
            continue;
        }
        let a = vy.atan2(vx);
        if angle_diff_mod_pi(a, cand.edges[0]).min(angle_diff_mod_pi(a, cand.edges[1])) > 0.26 {
            continue;
        }
        if best.is_none_or(|(_, bd)| d < bd) {
            best = Some((i, d));
        }
    }
    best.map(|(i, _)| i)
}

/// Unused candidate nearest to (x, y) within `tol`.
fn nearest(c: &[Candidate], x: f64, y: f64, tol: f64, used: &[bool]) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (i, cand) in c.iter().enumerate() {
        if used[i] {
            continue;
        }
        let d = (cand.x - x).hypot(cand.y - y);
        if d <= tol && best.is_none_or(|(_, bd)| d < bd) {
            best = Some((i, d));
        }
    }
    best.map(|(i, _)| i)
}

/// Grid of candidate indices, rows x cols, row-major Vec<Vec>.
type Grid = Vec<Vec<usize>>;

fn seed(c: &[Candidate], s: usize) -> Option<Grid> {
    use std::f64::consts::PI;
    let used = vec![false; c.len()];
    let [e1, e2] = c[s].edges;
    let right = neighbour(c, s, e1, &used)?;
    let left = neighbour(c, s, e1 + PI, &used)?;
    let down = neighbour(c, s, e2, &used)?;
    let up = neighbour(c, s, e2 + PI, &used)?;
    let dist = |a: usize, b: usize| (c[a].x - c[b].x).hypot(c[a].y - c[b].y);
    let (dr, dl, dd, du) = (dist(s, right), dist(s, left), dist(s, down), dist(s, up));
    let ratio_ok = |a: f64, b: f64| a / b > 0.5 && a / b < 2.;
    if !ratio_ok(dr, dl) || !ratio_ok(dd, du) || !ratio_ok(dr.min(dl), dd.min(du)) {
        return None;
    }
    let tol = 0.3 * dr.min(dl).min(dd).min(du);
    let mut grid = vec![
        vec![usize::MAX, up, usize::MAX],
        vec![left, s, right],
        vec![usize::MAX, down, usize::MAX],
    ];
    let mut taken = vec![false; c.len()];
    for &i in &[s, right, left, down, up] {
        if taken[i] {
            return None;
        }
        taken[i] = true;
    }
    for (gr, vert) in [(0usize, up), (2usize, down)] {
        for (gc, horiz) in [(0usize, left), (2usize, right)] {
            let x = c[vert].x + c[horiz].x - c[s].x;
            let y = c[vert].y + c[horiz].y - c[s].y;
            let i = nearest(c, x, y, tol, &taken)?;
            taken[i] = true;
            grid[gr][gc] = i;
        }
    }
    Some(grid)
}

/// Extends the grid by whole rows/columns while every predicted corner is found.
fn grow(c: &[Candidate], mut grid: Grid) -> Grid {
    let mut taken = vec![false; c.len()];
    for row in &grid {
        for &i in row {
            taken[i] = true;
        }
    }
    let pos = |i: usize| (c[i].x, c[i].y);
    let predict = |line: &[usize]| -> (f64, f64, f64) {
        // line[0] is the edge corner, line[1], line[2] inward.
        let (p0, p1) = (pos(line[0]), pos(line[1]));
        let step = (p0.0 - p1.0).hypot(p0.1 - p1.1);
        let (x, y) = if line.len() >= 3 {
            let p2 = pos(line[2]);
            (3. * p0.0 - 3. * p1.0 + p2.0, 3. * p0.1 - 3. * p1.1 + p2.1)
        } else {
            (2. * p0.0 - p1.0, 2. * p0.1 - p1.1)
        };
        (x, y, step)
    };
    loop {
        let mut grew = false;
        for side in 0..4 {
            let (rows, cols) = (grid.len(), grid[0].len());
            let lines: Vec<Vec<usize>> = match side {
                0 => (0..cols)
                    .map(|j| (0..rows.min(3)).map(|i| grid[i][j]).collect())
                    .collect(),
                1 => (0..cols)
                    .map(|j| (0..rows.min(3)).map(|i| grid[rows - 1 - i][j]).collect())
                    .collect(),
                2 => (0..rows)
                    .map(|i| (0..cols.min(3)).map(|j| grid[i][j]).collect())
                    .collect(),
                _ => (0..rows)
                    .map(|i| (0..cols.min(3)).map(|j| grid[i][cols - 1 - j]).collect())
                    .collect(),
            };
            let mut new = Vec::with_capacity(lines.len());
            let mut claimed = taken.clone();
            let mut ok = true;
            for line in &lines {
                let (x, y, step) = predict(line);
                match nearest(c, x, y, 0.3 * step, &claimed) {
                    Some(i) => {
                        claimed[i] = true;
                        new.push(i);
                    }
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if !ok {
                continue;
            }
            taken = claimed;
            grew = true;
            match side {
                0 => grid.insert(0, new),
                1 => grid.push(new),
                2 => {
                    for (row, i) in grid.iter_mut().zip(new) {
                        row.insert(0, i);
                    }
                }
                _ => {
                    for (row, i) in grid.iter_mut().zip(new) {
                        row.push(i);
                    }
                }
            }
        }
        if !grew {
            return grid;
        }
    }
}

/// The cells (sampled at the mean of their four corners) must alternate dark
/// and bright: each cell differs from the mean of the cells sharing an edge
/// with it by at least half the ring contrast of its own corners, with the
/// same sign pattern over the whole board. Local on both sides, so uneven
/// lighting across the board does not matter.
fn cells_alternate(s: &Grey, pts: &[Vec<(f64, f64)>], contrast: &[Vec<f64>]) -> bool {
    let (rows, cols) = (pts.len() - 1, pts[0].len() - 1);
    let mut value = vec![vec![0.; cols]; rows];
    let mut local = vec![vec![0.; cols]; rows];
    for i in 0..rows {
        for j in 0..cols {
            let x = (pts[i][j].0 + pts[i + 1][j].0 + pts[i][j + 1].0 + pts[i + 1][j + 1].0) / 4.;
            let y = (pts[i][j].1 + pts[i + 1][j].1 + pts[i][j + 1].1 + pts[i + 1][j + 1].1) / 4.;
            value[i][j] = s.sample(x, y);
            local[i][j] =
                (contrast[i][j] + contrast[i + 1][j] + contrast[i][j + 1] + contrast[i + 1][j + 1])
                    / 4.;
        }
    }
    let mut sign = 0.;
    for i in 0..rows {
        for j in 0..cols {
            let (mut sum, mut n) = (0., 0.);
            if i > 0 {
                sum += value[i - 1][j];
                n += 1.;
            }
            if i + 1 < rows {
                sum += value[i + 1][j];
                n += 1.;
            }
            if j > 0 {
                sum += value[i][j - 1];
                n += 1.;
            }
            if j + 1 < cols {
                sum += value[i][j + 1];
                n += 1.;
            }
            if n == 0. {
                return false;
            }
            let d = value[i][j] - sum / n;
            let d = if (i + j) % 2 == 0 { d } else { -d };
            if sign == 0. {
                sign = if d >= 0. { 1. } else { -1. };
            }
            if d * sign <= 0.5 * local[i][j] {
                return false;
            }
        }
    }
    true
}

/// Gradient-orthogonality refinement (Foerstner); None when it wanders off.
fn refine(gx: &Grey, gy: &Grey, x0: f64, y0: f64, half: usize) -> Option<(f64, f64)> {
    let (mut x, mut y) = (x0, y0);
    let hw = half as isize;
    let sigma = half as f64 / 2.;
    let lim = (half as f64) + 1.;
    if x0 < lim || y0 < lim || x0 > gx.w as f64 - 1. - lim || y0 > gx.h as f64 - 1. - lim {
        return None;
    }
    for _ in 0..30 {
        let (mut a11, mut a12, mut a22, mut b1, mut b2) = (0., 0., 0., 0., 0.);
        for dj in -hw..=hw {
            for di in -hw..=hw {
                let (ox, oy) = (dj as f64, di as f64);
                let wgt = (-(ox * ox + oy * oy) / (2. * sigma * sigma)).exp();
                let (px, py) = (x + ox, y + oy);
                let (u, v) = (gx.sample(px, py), gy.sample(px, py));
                let (g11, g12, g22) = (u * u * wgt, u * v * wgt, v * v * wgt);
                a11 += g11;
                a12 += g12;
                a22 += g22;
                b1 += g11 * px + g12 * py;
                b2 += g12 * px + g22 * py;
            }
        }
        let det = a11 * a22 - a12 * a12;
        if det.abs() < 1e-20 {
            return None;
        }
        let nx = (a22 * b1 - a12 * b2) / det;
        let ny = (a11 * b2 - a12 * b1) / det;
        let moved = (nx - x).hypot(ny - y);
        x = nx;
        y = ny;
        if (x - x0).hypot(y - y0) > half as f64 {
            return None;
        }
        if x < lim || y < lim || x > gx.w as f64 - 1. - lim || y > gx.h as f64 - 1. - lim {
            return None;
        }
        if moved < 1e-4 {
            break;
        }
    }
    Some((x, y))
}

/// Refinement with the largest window that fits. A corner too close to the
/// image border for the board's window gets a smaller one, and keeps its
/// unrefined position when even the smallest fails; None only when a corner
/// with room for the whole window cannot be refined.
fn refine_fitting(gx: &Grey, gy: &Grey, x: f64, y: f64, half: usize) -> Option<(f64, f64)> {
    let room = x.min(y).min(gx.w as f64 - 1. - x).min(gx.h as f64 - 1. - y);
    let near_border = room < half as f64 + 3.;
    // `refine` keeps the window one pixel inside the image.
    let mut window = half.min((room - 1.).floor().max(0.) as usize);
    while window >= 2 {
        if let Some(p) = refine(gx, gy, x, y, window) {
            return Some(p);
        }
        if !near_border {
            return None;
        }
        window -= 1;
    }
    Some((x, y))
}

/// The image at half size (2-by-2 means): pixel (r, c) covers rows 2r, 2r+1
/// and columns 2c, 2c+1, so x_full = 2 x_half + 0.5.
fn halve(img: &Grey) -> Grey {
    let (h, w) = (img.h / 2, img.w / 2);
    let mut data = Vec::with_capacity(h * w);
    for c in 0..w {
        for r in 0..h {
            data.push(
                0.25 * (img.at(2 * r, 2 * c)
                    + img.at(2 * r + 1, 2 * c)
                    + img.at(2 * r, 2 * c + 1)
                    + img.at(2 * r + 1, 2 * c + 1)),
            );
        }
    }
    Grey { h, w, data }
}

/// Central-difference gradients of the image.
fn gradients(img: &Grey) -> (Grey, Grey) {
    let (h, w) = (img.h, img.w);
    let mut gx = vec![0.; h * w];
    let mut gy = vec![0.; h * w];
    for c in 0..w {
        for r in 0..h {
            let (cl, cr) = (c.saturating_sub(1), (c + 1).min(w - 1));
            let (ru, rd) = (r.saturating_sub(1), (r + 1).min(h - 1));
            gx[r + c * h] = (img.at(r, cr) - img.at(r, cl)) / (cr - cl).max(1) as f64;
            gy[r + c * h] = (img.at(rd, c) - img.at(ru, c)) / (rd - ru).max(1) as f64;
        }
    }
    (Grey { h, w, data: gx }, Grey { h, w, data: gy })
}

/// Puts the grid in board order (x along the longer side, black origin
/// square, right-handed) and flattens it y-fastest.
fn orient(s: &Grey, pts: Vec<Vec<(f64, f64)>>) -> Option<Board> {
    let transpose = |g: &Vec<Vec<(f64, f64)>>| -> Vec<Vec<(f64, f64)>> {
        (0..g[0].len())
            .map(|j| (0..g.len()).map(|i| g[i][j]).collect())
            .collect()
    };
    let flip_rows =
        |g: &Vec<Vec<(f64, f64)>>| -> Vec<Vec<(f64, f64)>> { g.iter().rev().cloned().collect() };
    let flip_cols = |g: &Vec<Vec<(f64, f64)>>| -> Vec<Vec<(f64, f64)>> {
        g.iter()
            .map(|r| r.iter().rev().copied().collect())
            .collect()
    };
    let (rows, cols) = (pts.len(), pts[0].len());
    let mut bases = Vec::new();
    if cols > rows {
        bases.push(pts.clone());
    } else if rows > cols {
        bases.push(transpose(&pts));
    } else {
        bases.push(pts.clone());
        bases.push(transpose(&pts));
    }
    let mut options = Vec::new();
    for b in &bases {
        for g in [
            b.clone(),
            flip_rows(b),
            flip_cols(b),
            flip_cols(&flip_rows(b)),
        ] {
            let (o, px, py) = (g[0][0], g[0][1], g[1][0]);
            let cross = (px.0 - o.0) * (py.1 - o.1) - (px.1 - o.1) * (py.0 - o.0);
            if cross > 0. {
                options.push(g);
            }
        }
    }
    let cell = |g: &Vec<Vec<(f64, f64)>>| {
        let (a, b, c, d) = (g[0][0], g[0][1], g[1][0], g[1][1]);
        s.sample((a.0 + b.0 + c.0 + d.0) / 4., (a.1 + b.1 + c.1 + d.1) / 4.)
    };
    // The handedness is tested at a different grid corner for each option, so
    // a badly bent grid may pass none.
    let first = options.first()?;
    let (ny, nx) = (first.len(), first[0].len());
    // Unique when one side has an even and the other an odd number of squares.
    let unique = (ny + 1) % 2 != (nx + 1) % 2 && ny != nx;
    let chosen = if unique {
        options
            .iter()
            .min_by(|a, b| cell(a).total_cmp(&cell(b)))
            .cloned()?
    } else {
        // Darkest origin cell first, then closest to the image's top-left.
        let darkest = options.iter().map(&cell).fold(f64::MAX, f64::min);
        let brightest = options.iter().map(&cell).fold(f64::MIN, f64::max);
        let tol = 0.25 * (brightest - darkest);
        options
            .iter()
            .filter(|g| cell(g) <= darkest + tol)
            .min_by(|a, b| (a[0][0].0 + a[0][0].1).total_cmp(&(b[0][0].0 + b[0][0].1)))
            .cloned()?
    };
    let (rows, cols) = (chosen.len(), chosen[0].len());
    let mut points = Vec::with_capacity(rows * cols);
    for j in 0..cols {
        for row in &chosen {
            points.push(row[j]);
        }
    }
    Some(Board { rows, cols, points })
}

/// Corner positions (x, y) by grid row.
type Corners = Vec<Vec<(f64, f64)>>;

/// The largest alternating corner grid of a smoothed image (unrefined).
fn find_grid(e: &Engine, s: &Grey) -> Result<Option<Corners>> {
    let cands = candidates(e, s)?;
    if cands.len() < 9 {
        return Ok(None);
    }
    let mut order: Vec<usize> = (0..cands.len()).collect();
    order.sort_by(|&a, &b| cands[b].score.total_cmp(&cands[a].score));
    let mut best: Option<Corners> = None;
    let mut in_best = vec![false; cands.len()];
    let mut tried = 0;
    for &i in &order {
        if tried >= MAX_SEEDS {
            break;
        }
        if in_best[i] {
            continue;
        }
        tried += 1;
        crate::engine::image_basic::cancelled(&e.cancel)?;
        let Some(g) = seed(&cands, i) else { continue };
        let g = grow(&cands, g);
        let pts: Corners = g
            .iter()
            .map(|r| r.iter().map(|&k| (cands[k].x, cands[k].y)).collect())
            .collect();
        let contrast: Vec<Vec<f64>> = g
            .iter()
            .map(|r| r.iter().map(|&k| cands[k].contrast).collect())
            .collect();
        if !cells_alternate(s, &pts, &contrast) {
            continue;
        }
        let size = g.len() * g[0].len();
        if best.as_ref().is_none_or(|b| size > b.len() * b[0].len()) {
            in_best = vec![false; cands.len()];
            for &k in g.iter().flatten() {
                in_best[k] = true;
            }
            best = Some(pts);
        }
    }
    Ok(best)
}

/// Finds the board in one image.
///
/// The grid is searched at full size and on copies reduced 2, 4 and 8 times
/// (while the smaller side keeps MIN_REDUCED_SIDE pixels): soft or
/// high-resolution images, whose corners are wider than the fixed ring, lose
/// corners at full size and are whole on a reduced copy. The grid with the
/// most corners wins, the finer size on a tie; its corners are then refined
/// on the full-size image.
fn detect(e: &Engine, img: &Grey) -> Result<Option<Board>> {
    if img.h < 16 || img.w < 16 {
        return Ok(None);
    }
    let s = smooth(img, SMOOTH_SIGMA);
    crate::engine::image_basic::cancelled(&e.cancel)?;
    let mut best: Option<Corners> = find_grid(e, &s)?;
    let mut reduced: Option<Grey> = None;
    let mut factor = 1usize;
    loop {
        let current = reduced.as_ref().unwrap_or(img);
        if factor >= MAX_REDUCTION || current.h.min(current.w) / 2 < MIN_REDUCED_SIDE {
            break;
        }
        let next = halve(current);
        factor *= 2;
        crate::engine::image_basic::cancelled(&e.cancel)?;
        if let Some(g) = find_grid(e, &smooth(&next, SMOOTH_SIGMA))? {
            let size = g.len() * g[0].len();
            if best.as_ref().is_none_or(|b| size > b.len() * b[0].len()) {
                // Pixel centres of the reduced copy in full-size coordinates.
                let f = factor as f64;
                let full = g
                    .into_iter()
                    .map(|r| {
                        r.into_iter()
                            .map(|(x, y)| (x * f + (f - 1.) / 2., y * f + (f - 1.) / 2.))
                            .collect()
                    })
                    .collect();
                best = Some(full);
            }
        }
        reduced = Some(next);
    }
    let Some(grid) = best else {
        return Ok(None);
    };
    // Sub-pixel refinement on the full-size image with a window of about a
    // third of the smallest corner spacing: it grows with the squares, which
    // soft and high-resolution images need (their edges are many pixels wide).
    let mut spacing = f64::MAX;
    for (i, row) in grid.iter().enumerate() {
        for (j, &p) in row.iter().enumerate() {
            if j + 1 < row.len() {
                let q = row[j + 1];
                spacing = spacing.min((p.0 - q.0).hypot(p.1 - q.1));
            }
            if i + 1 < grid.len() {
                let q = grid[i + 1][j];
                spacing = spacing.min((p.0 - q.0).hypot(p.1 - q.1));
            }
        }
    }
    let half = ((0.3 * spacing).round() as usize).clamp(2, MAX_REFINE_HALF);
    let (gx, gy) = gradients(img);
    let mut pts = Vec::with_capacity(grid.len());
    for row in &grid {
        let mut out = Vec::with_capacity(row.len());
        for &(x, y) in row {
            match refine_fitting(&gx, &gy, x, y, half) {
                Some(p) => out.push(p),
                None => return Ok(None),
            }
        }
        pts.push(out);
        crate::engine::image_basic::cancelled(&e.cancel)?;
    }
    Ok(orient(&s, pts))
}
