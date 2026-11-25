//! Flat structuring elements: `strel` objects and plain neighbourhood matrices.
use super::{Cx, numbers, scalar, text_of};
use crate::engine::Value;
use crate::{Array, LogicalMatrix, Matrix, Result, StructArray};
use std::sync::Arc;

/// Value of the language-object tag that marks a `strel` object.
pub(in crate::engine) const STREL_TAG: f64 = 5.;
/// Neighbourhood side limit (a 2001-by-2001 element).
const MAX_SIDE: usize = 2001;

/// A flat 2-D neighbourhood in column-major order; the origin is element
/// (floor((rows+1)/2), floor((cols+1)/2)) (1-based).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Se {
    pub rows: usize,
    pub cols: usize,
    pub mask: Vec<bool>,
}
impl Se {
    fn from_points(points: &[(i64, i64)]) -> Self {
        // (dx, dy) with y pointing up; the result is centred on the origin.
        let hx = points.iter().map(|p| p.0.abs()).max().unwrap_or(0);
        let hy = points.iter().map(|p| p.1.abs()).max().unwrap_or(0);
        let rows = (2 * hy + 1) as usize;
        let cols = (2 * hx + 1) as usize;
        let mut mask = vec![false; rows * cols];
        for &(x, y) in points {
            let r = (hy - y) as usize;
            let c = (hx + x) as usize;
            mask[c * rows + r] = true;
        }
        Self { rows, cols, mask }
    }
    fn from_fn(half_rows: usize, half_cols: usize, f: impl Fn(i64, i64) -> bool) -> Self {
        let rows = 2 * half_rows + 1;
        let cols = 2 * half_cols + 1;
        let mut mask = Vec::with_capacity(rows * cols);
        for c in 0..cols {
            for r in 0..rows {
                mask.push(f(r as i64 - half_rows as i64, c as i64 - half_cols as i64));
            }
        }
        Self { rows, cols, mask }
    }
    fn filled(rows: usize, cols: usize) -> Self {
        Self {
            rows,
            cols,
            mask: vec![true; rows * cols],
        }
    }
    /// 0-based origin (row, col).
    pub fn origin(&self) -> (usize, usize) {
        (
            self.rows.saturating_sub(1) / 2,
            self.cols.saturating_sub(1) / 2,
        )
    }
    /// Member offsets (drow, dcol) from the origin, column-major order.
    pub fn offsets(&self) -> Vec<(isize, isize)> {
        let (r0, c0) = self.origin();
        let mut v = Vec::new();
        for c in 0..self.cols {
            for r in 0..self.rows {
                if self.mask[c * self.rows + r] {
                    v.push((r as isize - r0 as isize, c as isize - c0 as isize));
                }
            }
        }
        v
    }
    pub fn is_full(&self) -> bool {
        self.mask.iter().all(|&b| b)
    }
    /// The 3-by-3 square bwmorph uses.
    pub fn square3() -> Self {
        Self::filled(3, 3)
    }
}

fn integer(cx: &Cx, v: &Value, what: &str, min: f64) -> Result<usize> {
    let x = scalar(cx, v, what)?;
    if !(x >= min && x.fract() == 0. && x <= ((MAX_SIDE - 1) / 2) as f64) {
        return cx.err(format!(
            "{what} must be an integer from {min} to {}",
            (MAX_SIDE - 1) / 2
        ));
    }
    Ok(x as usize)
}

fn arity(cx: &Cx, shape: &str, args: &[Value], min: usize, max: usize) -> Result<()> {
    let n = args.len() - 1;
    if n < min || n > max {
        return cx.err(format!(
            "the '{shape}' shape takes {}",
            if min == max {
                format!("{min} argument{}", if min == 1 { "" } else { "s" })
            } else {
                format!("{min} to {max} arguments")
            }
        ));
    }
    Ok(())
}

/// The exact digital disk x^2 + y^2 <= r^2.
pub(super) fn exact_disk(r: usize) -> Se {
    let r2 = (r * r) as i64;
    Se::from_fn(r, r, |y, x| x * x + y * y <= r2)
}

/// Lattice directions (dx, dy) of the periodic lines for n = 4, 6, 8: the
/// direction sets are closed under 90-degree rotation and both flips.
fn directions(n: usize) -> &'static [(i64, i64)] {
    match n {
        4 => &[(1, 0), (1, 1), (0, 1), (-1, 1)],
        6 => &[(1, 0), (2, 1), (1, 2), (0, 1), (-1, 2), (-2, 1)],
        _ => &[
            (1, 0),
            (2, 1),
            (1, 1),
            (1, 2),
            (0, 1),
            (-1, 2),
            (-1, 1),
            (-2, 1),
        ],
    }
}

/// Inradius of the zonogon sum of segments of half-length `len[q]` in the
/// directions `theta[q]`: the support function sum len*|cos(theta - phi)| is
/// concave between its breakpoints, so its minimum is at a normal of one of
/// the segments.
fn inradius(theta: &[f64], len: &[f64]) -> f64 {
    theta
        .iter()
        .map(|t| {
            let phi = t + std::f64::consts::FRAC_PI_2;
            theta
                .iter()
                .zip(len)
                .map(|(u, l)| l * (u - phi).cos().abs())
                .sum::<f64>()
        })
        .fold(f64::INFINITY, f64::min)
}

/// Disk approximated by n periodic lines (Adams 1993; Jones & Soille 1996):
/// lines k*v, |k| <= p_v, in n lattice directions whose Minkowski sum is a
/// 2n-sided polygon of inradius a <= r, then dilated by the exact disk of
/// radius floor(r - a) so that the result reaches radius r.
pub(super) fn line_disk(r: usize, n: usize) -> Se {
    let dirs = directions(n);
    let theta: Vec<f64> = dirs
        .iter()
        .map(|&(x, y)| (y as f64).atan2(x as f64))
        .collect();
    let norm: Vec<f64> = dirs
        .iter()
        .map(|&(x, y)| ((x * x + y * y) as f64).sqrt())
        .collect();
    let unit = inradius(&theta, &vec![1.; n]);
    let half = r as f64 / unit;
    let reps: Vec<i64> = norm
        .iter()
        .map(|v| (half / v + 1e-9).floor() as i64)
        .collect();
    let lens: Vec<f64> = reps.iter().zip(&norm).map(|(k, v)| *k as f64 * v).collect();
    let a = inradius(&theta, &lens);
    let rest = (r as f64 - a + 1e-9).floor().max(0.) as i64;
    let mut points: std::collections::BTreeSet<(i64, i64)> = [(0, 0)].into();
    for (&(dx, dy), &k) in dirs.iter().zip(&reps) {
        if k == 0 {
            continue;
        }
        let mut next = std::collections::BTreeSet::new();
        for &(x, y) in &points {
            for i in -k..=k {
                next.insert((x + i * dx, y + i * dy));
            }
        }
        points = next;
    }
    let mut disk = Vec::new();
    for y in -rest..=rest {
        for x in -rest..=rest {
            if x * x + y * y <= rest * rest {
                disk.push((x, y));
            }
        }
    }
    let mut all = std::collections::BTreeSet::new();
    for &(x, y) in &points {
        for &(p, q) in &disk {
            all.insert((x + p, y + q));
        }
    }
    Se::from_points(&all.into_iter().collect::<Vec<_>>())
}

fn round_half_away(v: f64) -> i64 {
    (v.abs() + 0.5).floor().copysign(v) as i64
}

/// Digital line through the origin: along the major axis every integer t
/// with |t| <= round(((len-1)/2)*|cos| (or |sin|)), the minor coordinate
/// rounded half away from zero, so the element is symmetric about its centre.
pub(super) fn line(len: f64, deg: f64) -> Se {
    let ray = (len - 1.) / 2.;
    let t = deg.to_radians();
    let (s, c) = (t.sin(), t.cos());
    let mut points = Vec::new();
    if c.abs() >= s.abs() {
        let n = round_half_away(ray * c.abs());
        for i in -n..=n {
            points.push((i, round_half_away(i as f64 * s / c)));
        }
    } else {
        let n = round_half_away(ray * s.abs());
        for i in -n..=n {
            points.push((round_half_away(i as f64 * c / s), i));
        }
    }
    Se::from_points(&points)
}

/// Octagon of apothem r (a multiple of 3): |x|, |y| <= r and |x| + |y| <= 4r/3.
pub(super) fn octagon(r: usize) -> Se {
    let cut = (4 * r / 3) as i64;
    Se::from_fn(r, r, |y, x| x.abs() + y.abs() <= cut)
}

fn nhood_from(cx: &Cx, v: &Value) -> Result<Se> {
    let dims = v.dims();
    if dims.len() > 2 {
        return cx.err("only 2-D neighborhoods are supported");
    }
    let values = numbers(cx, v, "the neighborhood")?;
    if values.iter().any(|x| x.is_nan()) {
        return cx.err("the neighborhood must not contain NaN");
    }
    if dims[0] > MAX_SIDE || dims[1] > MAX_SIDE {
        return cx.err(format!("neighborhood sides are limited to {MAX_SIDE}"));
    }
    Ok(Se {
        rows: dims[0],
        cols: dims[1],
        mask: values.iter().map(|&x| x != 0.).collect(),
    })
}

/// A strel object, or a plain logical/numeric neighbourhood, as an element.
pub(super) fn se_arg(cx: &Cx, v: &Value) -> Result<Se> {
    if let Value::Container(Array::Struct(s)) = v {
        if !is_strel(s) {
            return cx
                .err("the structuring element must be a strel object or a neighborhood matrix");
        }
        let f = s
            .field_index("Neighborhood")
            .ok_or("strel without Neighborhood")?;
        return nhood_from(cx, &Value::from(s.get(f, 0)?.as_ref().clone()));
    }
    nhood_from(cx, v)
}

pub(super) fn is_strel(s: &StructArray) -> bool {
    s.len() == 1
        && s.field_index(crate::engine::lang_more::TAG)
            .and_then(|f| s.get(f, 0).ok())
            .is_some_and(|v| matches!(v.as_ref(), Array::Numeric(m) if m.data() == [STREL_TAG]))
}

fn object(cx: &Cx, se: &Se) -> Result<Value> {
    let bytes = se.mask.iter().map(|&b| u8::from(b)).collect();
    let nhood = LogicalMatrix::from_column_vec(se.rows, se.cols, bytes, cx.cancel)?;
    let names = vec![
        "Neighborhood".to_string(),
        "Dimensionality".to_string(),
        crate::engine::lang_more::TAG.to_string(),
    ];
    let slots = vec![
        Arc::new(Array::Logical(nhood)),
        Arc::new(Array::Numeric(Matrix::scalar(2.))),
        Arc::new(Array::Numeric(Matrix::scalar(STREL_TAG))),
    ];
    Ok(Value::Container(Array::Struct(StructArray::from_slots(
        1, 1, names, slots, cx.cancel,
    )?)))
}

pub(super) fn strel(cx: &Cx, args: &[Value]) -> Result<Value> {
    let Some(shape) = text_of(&args[0]) else {
        if args.len() > 1 {
            return cx.err("expected strel(nhood) or strel(shape, parameters)");
        }
        if let Value::Container(Array::Struct(s)) = &args[0]
            && is_strel(s)
        {
            return Ok(args[0].clone());
        }
        let se = nhood_from(cx, &args[0])?;
        return object(cx, &se);
    };
    let lower = shape.to_ascii_lowercase();
    let se = match lower.as_str() {
        "arbitrary" => {
            arity(cx, &lower, args, 1, 2)?;
            if args.len() == 3 {
                return cx.err(
                    "non-flat structuring elements (a height argument) are not supported yet",
                );
            }
            nhood_from(cx, &args[1])?
        }
        "square" => {
            arity(cx, &lower, args, 1, 1)?;
            let w = integer(cx, &args[1], "the width", 0.)?;
            let w = w.min(MAX_SIDE);
            Se::filled(w, w)
        }
        "rectangle" => {
            arity(cx, &lower, args, 1, 1)?;
            let mn = numbers(cx, &args[1], "the size")?;
            let ok = |v: f64| v >= 0. && v.fract() == 0. && v <= MAX_SIDE as f64;
            match mn.as_slice() {
                [m, n] if ok(*m) && ok(*n) => Se::filled(*m as usize, *n as usize),
                _ => return cx.err("the size must be [m n] with nonnegative integers"),
            }
        }
        "diamond" => {
            arity(cx, &lower, args, 1, 1)?;
            let r = integer(cx, &args[1], "the radius", 0.)? as i64;
            Se::from_fn(r as usize, r as usize, |y, x| x.abs() + y.abs() <= r)
        }
        "disk" => {
            arity(cx, &lower, args, 1, 2)?;
            let r = integer(cx, &args[1], "the radius", 0.)?;
            let n = match args.get(2) {
                None => 4.,
                Some(v) => scalar(cx, v, "n")?,
            };
            match n {
                0. => exact_disk(r),
                4. | 6. | 8. if r == 0 => exact_disk(0),
                4. | 6. | 8. => line_disk(r, n as usize),
                _ => return cx.err("n must be 0, 4, 6 or 8"),
            }
        }
        "line" => {
            arity(cx, &lower, args, 2, 2)?;
            let len = scalar(cx, &args[1], "the length")?;
            let deg = scalar(cx, &args[2], "the angle")?;
            if !(len > 0. && len <= MAX_SIDE as f64) || !deg.is_finite() {
                return cx.err(format!(
                    "the length must be positive (at most {MAX_SIDE}) and the angle finite"
                ));
            }
            line(len, deg)
        }
        "octagon" => {
            arity(cx, &lower, args, 1, 1)?;
            let r = integer(cx, &args[1], "the size", 0.)?;
            if r % 3 != 0 {
                return cx.err("the octagon size must be a nonnegative multiple of 3");
            }
            octagon(r)
        }
        "ball" | "sphere" | "cube" | "cuboid" | "periodicline" | "pair" => {
            return cx.err(format!("the '{lower}' shape is not supported yet"));
        }
        _ => return cx.err(format!("unknown shape '{shape}'")),
    };
    object(cx, &se)
}
