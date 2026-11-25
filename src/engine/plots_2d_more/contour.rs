//! contourc, contour, contourf, contour3 and clabel.
//!
//! The contour matrix follows GNU Octave 10.3's `__contourc__` exactly (the
//! reference for tests/contour_plots.rs): per level, lines that start on the
//! top and bottom edges (column by column), then the left and right edges
//! (row by row), then closed interior lines in row-major cell order. A saddle
//! cell is left through the first crossed edge in Octave's search order
//! (clockwise from the top and bottom edges, counterclockwise from the sides).
//! Levels: a count n gives linspace(min(Z), max(Z), n+2) without its ends
//! (Octave's linspace, built symmetrically from both ends); the default is 10.
//! contourf keeps min(Z) in front of them, as Octave's `__contour__` does, so
//! the band from the minimum up to the first line has its own level, colour
//! and CLim end; below an explicit first level nothing is filled.
//!
//! The figure is drawn from existing series: one `Line` series per level
//! (NaN-separated pieces) and, for contourf, `Patch` series per band built
//! cell by cell (axis-aligned rectangles for runs of whole cells, triangles
//! for cut cells). Labels are axes text objects. contour3 draws the same
//! lines at Z = level on a view(3) axes (no labels: they are 2-D text).
use super::*;
use crate::plot::{MAX_PATCH_FACES, MAX_POINTS, MAX_SERIES, MAX_TEXTS, TextObject};
use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub(super) const CONTOUR_NAMES: &[&str] =
    &["contourc", "contour", "contourf", "contour3", "clabel"];
/// Same element bound as histogram's data.
const MAX_Z: usize = 1_000_000;
/// Contour points (or band faces) one call may build before any figure work.
const MAX_BUILD: usize = 2_000_000;
const MAX_LEVELS: usize = 1000;
const DEFAULT_LEVELS: usize = 10;
const LINE_WIDTH: f64 = 0.5;
const NAMES: &[&str] = &[
    "LineColor",
    "LineWidth",
    "LineStyle",
    "LevelList",
    "ShowText",
    "DisplayName",
];

// ------------------------------------------------------------------ grid

/// Sample positions: a tensor grid (x per column, y per row) or a general
/// (curvilinear) mesh of Z's size.
#[derive(Clone)]
pub(super) enum Grid {
    /// `explicit` is false for the default 1:n positions.
    Tensor {
        x: Vec<f64>,
        y: Vec<f64>,
        explicit: bool,
    },
    Mesh {
        x: Matrix,
        y: Matrix,
    },
}

/// One contour line: its level and points.
pub(crate) type Line = (f64, Vec<[f64; 2]>);

fn z_matrix(value: &Value, who: &str) -> Result<Matrix> {
    let z = match value {
        Value::Matrix(m) => m.clone(),
        Value::Logical(_) => real_matrix(value, who)?,
        Value::Complex(_) => return Err(format!("{who}: Z must be real")),
        _ => return Err(format!("{who}: Z must be a real numeric matrix")),
    };
    if z.len() > MAX_Z {
        return Err(format!("{who}: Z exceeds 1000000 elements"));
    }
    if z.rows() < 2 || z.cols() < 2 {
        return Err(format!("{who}: Z must be at least a 2x2 matrix"));
    }
    if z.data().iter().any(|v| v.is_infinite()) {
        return Err(format!("{who}: Z must contain only finite values or NaN"));
    }
    Ok(z)
}

fn coordinate(value: &Value, who: &str, what: &str) -> Result<Matrix> {
    match value {
        Value::Matrix(_) | Value::Logical(_) => {
            let m = real_matrix(value, who)?;
            if m.data().iter().any(|v| !v.is_finite()) {
                return Err(format!("{who}: {what} must be finite"));
            }
            Ok(m)
        }
        Value::Complex(_) => Err(format!("{who}: {what} must be real")),
        _ => Err(format!(
            "{who}: {what} must be a real numeric vector or matrix"
        )),
    }
}

/// X and Y: vectors with one entry per column and row of Z, or matrices of
/// Z's size. Matrices from meshgrid become a tensor grid (as in Octave).
fn grid_of(x: &Value, y: &Value, z: &Matrix, who: &str) -> Result<Grid> {
    let (x, y) = (coordinate(x, who, "X")?, coordinate(y, who, "Y")?);
    let (nr, nc) = (z.rows(), z.cols());
    let vector = |m: &Matrix| m.rows() == 1 || m.cols() == 1;
    if vector(&x) && vector(&y) && !(x.rows() == nr && x.cols() == nc) {
        if x.len() != nc || y.len() != nr {
            return Err(format!(
                "{who}: the lengths of X and Y must match the number of columns and rows of Z"
            ));
        }
        return Ok(Grid::Tensor {
            x: x.data().to_vec(),
            y: y.data().to_vec(),
            explicit: true,
        });
    }
    if (x.rows(), x.cols()) != (nr, nc) || (y.rows(), y.cols()) != (nr, nc) {
        return Err(format!(
            "{who}: X and Y must be vectors matching Z's columns and rows, or matrices the size of Z"
        ));
    }
    let tensor = (0..nr).all(|r| (0..nc).all(|c| x.get(r, c) == x.get(0, c)))
        && (0..nc).all(|c| (0..nr).all(|r| y.get(r, c) == y.get(r, 0)));
    if tensor {
        return Ok(Grid::Tensor {
            x: (0..nc).map(|c| x.get(0, c)).collect(),
            y: (0..nr).map(|r| y.get(r, 0)).collect(),
            explicit: true,
        });
    }
    Ok(Grid::Mesh { x, y })
}

fn index_grid(z: &Matrix) -> Grid {
    Grid::Tensor {
        x: (1..=z.cols()).map(|i| i as f64).collect(),
        y: (1..=z.rows()).map(|i| i as f64).collect(),
        explicit: false,
    }
}

// ---------------------------------------------------------------- levels

/// Octave's linspace: exact ends, built symmetrically from both ends.
fn linspace(a: f64, b: f64, n: usize) -> Vec<f64> {
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![b];
    }
    if a == b {
        return vec![b; n];
    }
    let mut v = vec![0.; n];
    v[0] = a;
    v[n - 1] = b;
    let delta = (b - a) / (n - 1) as f64;
    let delta = if delta.is_infinite() { f64::NAN } else { delta };
    for i in 1..n / 2 {
        v[i] = a + i as f64 * delta;
        v[n - 1 - i] = b - i as f64 * delta;
    }
    if n % 2 == 1 {
        v[n / 2] = if a == -b {
            0.
        } else if delta.is_nan() {
            f64::NAN
        } else {
            (a + b) / 2.
        };
    }
    v
}

/// n levels strictly inside the data range (Octave's contourc rule). With
/// `with_min` (contourf, Octave's `__contour__`: linspace(min, max, n+2)
/// without its last entry) min(Z) comes first; constant data then gives the
/// single level min(Z).
pub(super) fn auto_levels(z: &[f64], n: usize, with_min: bool) -> Vec<f64> {
    let (lo, hi) = z
        .iter()
        .filter(|v| !v.is_nan())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &v| {
            (a.min(v), b.max(v))
        });
    if lo > hi {
        return Vec::new();
    }
    let v = linspace(lo, hi, n + 2);
    let mut v = v[usize::from(!with_min)..=n].to_vec();
    if with_min {
        v.dedup();
    }
    v
}

/// Explicit levels: sorted, duplicates removed (`[k k]` is the level k).
fn explicit_levels(v: &[f64], who: &str) -> Result<Vec<f64>> {
    if v.len() > MAX_LEVELS * 10 {
        return Err(format!("{who}: at most {} levels", MAX_LEVELS * 10));
    }
    if v.iter().any(|x| !x.is_finite()) {
        return Err(format!("{who}: contour levels must be finite"));
    }
    let mut v = v.to_vec();
    v.sort_by(f64::total_cmp);
    v.dedup();
    Ok(v)
}

/// The level argument: a count n (scalar) or a vector of levels.
fn levels_arg(value: &Value, z: &Matrix, with_min: bool, who: &str) -> Result<Vec<f64>> {
    let v = p2_vec(value, who)?;
    match level_selection(&v, who)? {
        LevelSelection::Automatic(n) => Ok(auto_levels(z.data(), n, with_min)),
        LevelSelection::Explicit(levels) => Ok(levels),
    }
}

pub(crate) enum LevelSelection {
    Automatic(usize),
    Explicit(Vec<f64>),
}

fn level_selection(v: &[f64], who: &str) -> Result<LevelSelection> {
    match v {
        [] => Err(format!("{who}: the level vector must not be empty")),
        [n] if *n >= 1. && *n <= MAX_LEVELS as f64 && n.fract() == 0. => {
            Ok(LevelSelection::Automatic(*n as usize))
        }
        [_] => Err(format!(
            "{who}: the number of levels must be a positive integer of at most {MAX_LEVELS}"
        )),
        _ => explicit_levels(v, who).map(LevelSelection::Explicit),
    }
}

pub(crate) fn slice_levels(value: Option<&Value>) -> Result<LevelSelection> {
    let Some(value) = value else {
        return Ok(LevelSelection::Automatic(DEFAULT_LEVELS));
    };
    let m = real_matrix(value, "contourslice")?;
    if m.rows() > 1 && m.cols() > 1 {
        return Err("contourslice: levels must be a count or a vector".into());
    }
    level_selection(m.data(), "contourslice")
}

// ------------------------------------------------------- marching squares

/// Facet edge bits (Octave's naming): 1 top (r), 2 right (c+1), 4 bottom
/// (r+1), 8 left (c). Corners 0 (r,c), 1 (r,c+1), 2 (r+1,c+1), 3 (r+1,c).
fn mark_facets(z: &[f64], nr: usize, nc: usize, lvl: f64) -> Vec<i8> {
    let (mr, mc) = (nr - 1, nc - 1);
    let adjust = |v: f64| {
        if v.abs() < f64::EPSILON {
            f64::EPSILON
        } else {
            v
        }
    };
    let mut mark = vec![0i8; mr * mc];
    for c in 0..mc {
        for r in 0..mr {
            let f0 = adjust(z[c * nr + r] - lvl);
            let f1 = adjust(z[(c + 1) * nr + r] - lvl);
            let f3 = adjust(z[c * nr + r + 1] - lvl);
            let f2 = adjust(z[(c + 1) * nr + r + 1] - lvl);
            let mut m = 0i8;
            if f1 * f2 < 0. {
                m += 2;
            }
            if f0 * f3 < 0. {
                m += 8;
            }
            if f0 * f1 < 0. {
                m += 1;
            }
            if f2 * f3 < 0. {
                m += 4;
            }
            mark[r + c * mr] = m;
        }
    }
    mark
}

struct Tracer<'a> {
    x: &'a [f64],
    y: &'a [f64],
    z: &'a [f64],
    nr: usize,
    mr: usize,
    mc: usize,
    lvl: f64,
    mark: Vec<i8>,
    lines: Vec<Line>,
}

impl Tracer<'_> {
    fn point(&self, px: &[f64; 4], py: &[f64; 4], pz: &[f64; 4], edge: usize) -> [f64; 2] {
        let (a, b) = (edge, (edge + 1) % 4);
        let tmp = pz[b].abs() / pz[a].abs();
        if tmp.is_nan() {
            [0.5, 0.5]
        } else {
            [
                px[a] + (px[b] - px[a]) / (1. + tmp),
                py[a] + (py[b] - py[a]) / (1. + tmp),
            ]
        }
    }

    /// Octave's drawcn: follow one line from facet (r, c) entering through
    /// `start` (None: pick the facet's highest marked edge).
    fn draw(&mut self, r: usize, c: usize, start: Option<usize>) {
        let (mut r, mut c) = (r as isize, c as isize);
        let mut start = start;
        let mut points: Vec<[f64; 2]> = Vec::new();
        let mut first = true;
        while r >= 0
            && c >= 0
            && (r as usize) < self.mr
            && (c as usize) < self.mc
            && self.mark[r as usize + c as usize * self.mr] > 0
        {
            let (ru, cu) = (r as usize, c as usize);
            let index = ru + cu * self.mr;
            let nr = self.nr;
            let px = [self.x[cu], self.x[cu + 1], self.x[cu + 1], self.x[cu]];
            let py = [self.y[ru], self.y[ru], self.y[ru + 1], self.y[ru + 1]];
            let pz = [
                self.z[cu * nr + ru] - self.lvl,
                self.z[(cu + 1) * nr + ru] - self.lvl,
                self.z[(cu + 1) * nr + ru + 1] - self.lvl,
                self.z[cu * nr + ru + 1] - self.lvl,
            ];
            let id = self.mark[index];
            let s = match start {
                Some(s) => s,
                None => match (0..4).rev().find(|&k| (1i8 << k) & id != 0) {
                    Some(k) => k,
                    None => break,
                },
            };
            self.mark[index] = self.mark[index].wrapping_sub(1i8 << s);
            if first {
                points.push(self.point(&px, &py, &pz, s));
                first = false;
            }
            let mut stop = s;
            for k in 1..=4 {
                stop = if s == 0 || s == 2 {
                    (s + k) % 4
                } else {
                    (s + 4 - k) % 4
                };
                if (1i8 << stop) & id != 0 {
                    break;
                }
            }
            points.push(self.point(&px, &py, &pz, stop));
            self.mark[index] = self.mark[index].wrapping_sub(1i8 << stop);
            match stop {
                0 => r -= 1,
                1 => c += 1,
                2 => r += 1,
                _ => c -= 1,
            }
            start = Some((stop + 2) % 4);
        }
        if points.len() >= 2 {
            self.lines.push((self.lvl, points));
        }
    }

    fn run(mut self) -> Vec<Line> {
        let (mr, mc) = (self.mr, self.mc);
        for c in 0..mc {
            if self.mark[c * mr] & 1 != 0 {
                self.draw(0, c, Some(0));
            }
            if self.mark[mr - 1 + c * mr] & 4 != 0 {
                self.draw(mr - 1, c, Some(2));
            }
        }
        for r in 0..mr {
            if self.mark[r] & 8 != 0 {
                self.draw(r, 0, Some(3));
            }
            if self.mark[r + (mc - 1) * mr] & 2 != 0 {
                self.draw(r, mc - 1, Some(1));
            }
        }
        for r in 0..mr {
            for c in 0..mc {
                if self.mark[r + c * mr] > 0 {
                    self.draw(r, c, None);
                }
            }
        }
        self.lines
    }
}

/// Bilinear interpolation of a mesh matrix at 1-based index-space (ci, cj).
pub(crate) fn mesh_at(m: &Matrix, ci: f64, cj: f64) -> f64 {
    let (nr, nc) = (m.rows(), m.cols());
    let ci = ci.clamp(1., nc as f64);
    let cj = cj.clamp(1., nr as f64);
    let i = (ci.floor() as usize).clamp(1, nc - 1);
    let j = (cj.floor() as usize).clamp(1, nr - 1);
    let (s, t) = (ci - i as f64, cj - j as f64);
    let a = m.get(j - 1, i - 1);
    let b = m.get(j - 1, i) - a;
    let c = m.get(j, i - 1) - a;
    let d = m.get(j, i) - a - b - c;
    a + b * s + c * t + d * s * t
}

/// Contour lines of every level, in Octave's order. Errors before tracing
/// once the crossings exceed the build bound.
pub(super) fn contour_lines(
    z: &Matrix,
    grid: &Grid,
    levels: &[f64],
    who: &str,
    cancel: &AtomicBool,
) -> Result<Vec<Line>> {
    let (nr, nc) = (z.rows(), z.cols());
    let index_x: Vec<f64>;
    let index_y: Vec<f64>;
    let (x, y) = match grid {
        Grid::Tensor { x, y, .. } => (x.as_slice(), y.as_slice()),
        Grid::Mesh { .. } => {
            index_x = (1..=nc).map(|i| i as f64).collect();
            index_y = (1..=nr).map(|i| i as f64).collect();
            (index_x.as_slice(), index_y.as_slice())
        }
    };
    let total = AtomicUsize::new(0);
    let one = |lvl: &f64| -> Result<Vec<Line>> {
        crate::dsp::check_cancel(cancel)?;
        let mark = mark_facets(z.data(), nr, nc, *lvl);
        let crossings: usize = mark.iter().map(|m| m.count_ones() as usize).sum::<usize>() / 2;
        if total.fetch_add(crossings, Ordering::Relaxed) + crossings > MAX_BUILD {
            return Err(format!(
                "{who}: the contours need more than {MAX_BUILD} points; use fewer levels or a coarser grid"
            ));
        }
        Ok(Tracer {
            x,
            y,
            z: z.data(),
            nr,
            mr: nr - 1,
            mc: nc - 1,
            lvl: *lvl,
            mark,
            lines: Vec::new(),
        }
        .run())
    };
    // Levels are independent; each keeps its serial arithmetic, so the result
    // does not depend on the worker count.
    let parallel = levels.len() > 1
        && z.len().saturating_mul(levels.len()) >= 1 << 18
        && crate::parallel::current_worker_count() > 1;
    let per_level: Vec<Vec<Line>> = if parallel {
        use rayon::prelude::*;
        levels.par_iter().map(one).collect::<Result<_>>()?
    } else {
        levels.iter().map(one).collect::<Result<_>>()?
    };
    let mut lines: Vec<Line> = per_level.into_iter().flatten().collect();
    if let Grid::Mesh { x: mx, y: my } = grid {
        for (_, points) in &mut lines {
            for p in points.iter_mut() {
                *p = [mesh_at(mx, p[0], p[1]), mesh_at(my, p[0], p[1])];
            }
        }
    }
    crate::dsp::check_cancel(cancel)?;
    Ok(lines)
}

/// Trace a sampled volume slice in 1-based grid coordinates.
pub(crate) fn slice_lines(
    e: &mut Engine,
    z: &Matrix,
    selection: &LevelSelection,
    work_remaining: &mut usize,
) -> Result<Vec<Line>> {
    const WHO: &str = "contourslice";
    if z.data().iter().any(|v| v.is_infinite()) {
        return Err(format!(
            "{WHO}: slice data must contain only finite values or NaN"
        ));
    }
    let levels = match selection {
        LevelSelection::Automatic(n) => Cow::Owned(auto_levels(z.data(), *n, false)),
        LevelSelection::Explicit(levels) => Cow::Borrowed(levels.as_slice()),
    };
    let work = z.len().saturating_mul(levels.len());
    *work_remaining = work_remaining
        .checked_sub(work)
        .ok_or_else(|| format!("{WHO}: too many samples and levels to trace; use fewer levels"))?;
    e.charge(work)?;
    contour_lines(z, &index_grid(z), &levels, WHO, &e.cancel)
}

/// The contour matrix: per line a column [level; npoints] then its points.
pub(super) fn c_matrix(lines: &[Line]) -> Result<Matrix> {
    let cols: usize = lines.iter().map(|(_, p)| p.len() + 1).sum();
    let mut data = Vec::with_capacity(cols * 2);
    for (level, points) in lines {
        data.extend([*level, points.len() as f64]);
        for p in points {
            data.extend(p);
        }
    }
    Matrix::from_column_vec(2, cols, data)
}

/// Lines of a contour matrix; errors on a malformed one (each block needs a
/// whole, positive point count that fits in the matrix).
fn parse_c(c: &Matrix, who: &str) -> Result<Vec<Line>> {
    if c.is_empty() {
        return Ok(Vec::new());
    }
    let bad = || format!("{who}: C is not a valid contour matrix");
    if c.rows() != 2 {
        return Err(bad());
    }
    let mut lines = Vec::new();
    let mut k = 0;
    while k < c.cols() {
        let (level, n) = (c.get(0, k), c.get(1, k));
        if !(n >= 1. && n.fract() == 0. && n < c.cols() as f64) || k + 1 + n as usize > c.cols() {
            return Err(bad());
        }
        let n = n as usize;
        lines.push((
            level,
            (k + 1..k + 1 + n)
                .map(|j| [c.get(0, j), c.get(1, j)])
                .collect(),
        ));
        k += n + 1;
    }
    Ok(lines)
}

// ------------------------------------------------------------- band fill

/// A polygon vertex in grid space with its value.
#[derive(Clone, Copy)]
struct Vz {
    p: [f64; 2],
    z: f64,
}

/// Sutherland-Hodgman clip of a convex polygon to f(z) >= 0, interpolating
/// linearly along the edges.
fn clip(poly: &[Vz], f: impl Fn(f64) -> f64) -> Vec<Vz> {
    let mut out = Vec::with_capacity(poly.len() + 2);
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let (fa, fb) = (f(a.z), f(b.z));
        if fa >= 0. {
            out.push(a);
        }
        if (fa >= 0.) != (fb >= 0.) {
            let t = fa / (fa - fb);
            out.push(Vz {
                p: [
                    a.p[0] + (b.p[0] - a.p[0]) * t,
                    a.p[1] + (b.p[1] - a.p[1]) * t,
                ],
                z: a.z + (b.z - a.z) * t,
            });
        }
    }
    out
}

fn twice_area(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// Filled bands: per band (k = from level k up to level k+1, 1-based),
/// axis-aligned rectangles (4 corners each) and triangles (3 corners each).
/// Band 0, below the first level, stays empty: Octave fills nothing there.
pub(super) struct Bands {
    pub rects: Vec<Vec<[f64; 2]>>,
    pub tris: Vec<Vec<[f64; 2]>>,
    pub faces: usize,
}

pub(super) fn bands(
    z: &Matrix,
    grid: &Grid,
    levels: &[f64],
    budget: usize,
    who: &str,
    cancel: &AtomicBool,
) -> Result<Bands> {
    let (nr, nc) = (z.rows(), z.cols());
    let nb = levels.len() + 1;
    let mut out = Bands {
        rects: vec![Vec::new(); nb],
        tris: vec![Vec::new(); nb],
        faces: 0,
    };
    let over = || {
        format!(
            "{who}: the filled bands need more than {budget} patch faces (a figure holds at most {MAX_PATCH_FACES}); use fewer levels or a coarser grid"
        )
    };
    let band_of = |v: f64| levels.partition_point(|&l| l <= v);
    let zat = |r: usize, c: usize| z.data()[c * nr + r];
    // Grid-space corners: tensor grids use their coordinates, meshes the
    // 1-based index space mapped afterwards.
    let (gx, gy, mesh): (Vec<f64>, Vec<f64>, Option<(&Matrix, &Matrix)>) = match grid {
        Grid::Tensor { x, y, .. } => (x.clone(), y.clone(), None),
        Grid::Mesh { x, y } => (
            (1..=nc).map(|i| i as f64).collect(),
            (1..=nr).map(|i| i as f64).collect(),
            Some((x, y)),
        ),
    };
    let map = |p: [f64; 2]| match mesh {
        Some((mx, my)) => [mesh_at(mx, p[0], p[1]), mesh_at(my, p[0], p[1])],
        None => p,
    };
    let push_tri = |out: &mut Bands, band: usize, t: [[f64; 2]; 3]| -> Result<()> {
        let t = t.map(map);
        if band == 0 {
            return Ok(());
        }
        if twice_area(t[0], t[1], t[2]) == 0. {
            return Ok(());
        }
        out.faces += 1;
        if out.faces > budget {
            return Err(over());
        }
        out.tris[band].extend(t);
        Ok(())
    };
    for r in 0..nr - 1 {
        crate::dsp::check_cancel(cancel)?;
        // A run of whole cells in one band: (band, first column).
        let mut run: Option<(usize, usize)> = None;
        for c in 0..=nc - 1 {
            let whole = if c < nc - 1 {
                let zs = [zat(r, c), zat(r, c + 1), zat(r + 1, c + 1), zat(r + 1, c)];
                if zs.iter().any(|v| v.is_nan()) {
                    None
                } else {
                    let b = zs.map(band_of);
                    if b.iter().all(|&k| k == b[0]) {
                        Some(b[0])
                    } else {
                        None
                    }
                }
            } else {
                None
            };
            if let Some((band, start)) = run
                && (whole != Some(band) || mesh.is_some())
            {
                // Close the run [start, c) (band 0 is never filled).
                if mesh.is_some() {
                    for cc in start..c {
                        let q = [
                            [gx[cc], gy[r]],
                            [gx[cc + 1], gy[r]],
                            [gx[cc + 1], gy[r + 1]],
                            [gx[cc], gy[r + 1]],
                        ];
                        push_tri(&mut out, band, [q[0], q[1], q[2]])?;
                        push_tri(&mut out, band, [q[0], q[2], q[3]])?;
                    }
                } else if band > 0 {
                    out.faces += 1;
                    if out.faces > budget {
                        return Err(over());
                    }
                    let (x0, x1, y0, y1) = (gx[start], gx[c], gy[r], gy[r + 1]);
                    out.rects[band].extend([[x0, y0], [x0, y1], [x1, y1], [x1, y0]]);
                }
                run = None;
            }
            if c == nc - 1 {
                break;
            }
            if let Some(b) = whole {
                if run.is_none() {
                    run = Some((b, c));
                }
                continue;
            }
            let zs = [zat(r, c), zat(r, c + 1), zat(r + 1, c + 1), zat(r + 1, c)];
            if zs.iter().any(|v| v.is_nan()) {
                continue;
            }
            let corners = [
                [gx[c], gy[r]],
                [gx[c + 1], gy[r]],
                [gx[c + 1], gy[r + 1]],
                [gx[c], gy[r + 1]],
            ];
            let quad: Vec<Vz> = (0..4)
                .map(|k| Vz {
                    p: corners[k],
                    z: zs[k],
                })
                .collect();
            let (lo_band, hi_band) = zs.iter().fold((usize::MAX, 0), |(a, b), &v| {
                (a.min(band_of(v)), b.max(band_of(v)))
            });
            // A saddle for some level: split at the cell centre (average value).
            let saddle = levels[lo_band..hi_band].iter().any(|&l| {
                let s = zs.map(|v| v >= l);
                s[0] == s[2] && s[1] == s[3] && s[0] != s[1]
            });
            let pieces: Vec<Vec<Vz>> = if saddle {
                let centre = Vz {
                    p: [(gx[c] + gx[c + 1]) / 2., (gy[r] + gy[r + 1]) / 2.],
                    z: (zs[0] + zs[1] + zs[2] + zs[3]) / 4.,
                };
                (0..4)
                    .map(|k| vec![quad[k], quad[(k + 1) % 4], centre])
                    .collect()
            } else {
                vec![quad]
            };
            for band in lo_band.max(1)..=hi_band {
                let lo = levels[band - 1];
                let hi = if band == levels.len() {
                    f64::INFINITY
                } else {
                    levels[band]
                };
                for piece in &pieces {
                    let mut poly = clip(piece, |v| v - lo);
                    if hi.is_finite() {
                        poly = clip(&poly, |v| hi - v);
                    }
                    for k in 1..poly.len().saturating_sub(1) {
                        push_tri(&mut out, band, [poly[0].p, poly[k].p, poly[k + 1].p])?;
                    }
                }
            }
        }
    }
    Ok(out)
}

// ------------------------------------------------------------- the chart

#[derive(Clone, Copy, PartialEq)]
pub(super) enum LineColor {
    /// Colormap colour of each level.
    Flat,
    Hidden,
    Rgb([u8; 3]),
}

#[derive(Clone, PartialEq)]
pub(super) enum Labels {
    Off,
    All,
    Levels(Vec<f64>),
    /// The lines of a contour matrix other than the contour's own.
    Given(Vec<Line>),
}

#[derive(Clone)]
pub(super) struct ContourRec {
    z: Matrix,
    grid: Grid,
    levels: Vec<f64>,
    fill: bool,
    line_color: LineColor,
    line_width: f64,
    pattern: LinePattern,
    marker: Option<(Option<crate::plot::MarkerShape>, bool)>,
    scatter: bool,
    labels: Labels,
    /// Label text objects this contour placed on its axes.
    texts: Vec<ObjectIdentity>,
    /// The automatic CLim last stored for this contour (None after it met a
    /// CLim set by the user). The axes' CLim counts as automatic while it
    /// equals one of these.
    auto_clim: Option<[f64; 2]>,
    visible: bool,
    lines: Vec<Line>,
    /// contour3: each level's line lies at Z = level.
    three: bool,
}

/// C's `%g`.
fn format_g(v: f64) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v > 0. { "Inf" } else { "-Inf" }.into();
    }
    if v == 0. {
        return "0".into();
    }
    let trim = |s: &str| {
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_owned()
        } else {
            s.to_owned()
        }
    };
    let scientific = format!("{v:.5e}");
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    if !(-4..6).contains(&exponent) {
        format!(
            "{}e{}{:02}",
            trim(mantissa),
            if exponent < 0 { '-' } else { '+' },
            exponent.abs()
        )
    } else {
        let decimals = (5 - exponent).max(0) as usize;
        trim(&format!("{v:.decimals$}"))
    }
}

fn map_color(map: &[[f64; 3]], clim: [f64; 2], v: f64) -> [u8; 3] {
    crate::plot::colormap::scaled_rgb(map, clim, v)
}

/// Lowest and highest of sorted levels.
fn level_range(levels: &[f64]) -> Option<[f64; 2]> {
    Some([*levels.first()?, *levels.last()?])
}

fn merge_range(a: Option<[f64; 2]>, b: Option<[f64; 2]>) -> Option<[f64; 2]> {
    match (a, b) {
        (Some(a), Some(b)) => Some([a[0].min(b[0]), a[1].max(b[1])]),
        (a, b) => a.or(b),
    }
}

/// The automatic CLim for a level range (a single level gets a unit margin).
fn clim_of(range: [f64; 2]) -> [f64; 2] {
    if range[0] == range[1] {
        [range[0] - 1., range[1] + 1.]
    } else {
        range
    }
}

/// The other contours drawn on an axes. While the axes' CLim is automatic it
/// spans the levels of all of them, so each contour needs the rest.
#[derive(Default)]
struct Siblings {
    /// Primary series of each.
    primaries: Vec<ObjectIdentity>,
    /// Lowest and highest level over them.
    range: Option<[f64; 2]>,
    /// The automatic CLim each last stored.
    recorded: Vec<[f64; 2]>,
}

fn siblings(e: &Engine, axes: &Axes) -> Siblings {
    let mut out = Siblings::default();
    for chart in &e.p2.charts {
        let P2Kind::Contour(rec) = &chart.kind else {
            continue;
        };
        if !axes
            .series
            .iter()
            .any(|s| s.identity.same_object(&chart.primary))
        {
            continue;
        }
        out.primaries.push(chart.primary.clone());
        out.range = merge_range(out.range, level_range(&rec.levels));
        out.recorded.extend(rec.auto_clim);
    }
    out
}

/// Midpoint by arc length.
fn midpoint(points: &[[f64; 2]]) -> [f64; 2] {
    let seg = |k: usize| {
        let (a, b) = (points[k], points[k + 1]);
        ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt()
    };
    let total: f64 = (0..points.len() - 1).map(seg).sum();
    let mut left = total / 2.;
    for k in 0..points.len() - 1 {
        let l = seg(k);
        if left <= l && l > 0. {
            let t = left / l;
            let (a, b) = (points[k], points[k + 1]);
            return [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        }
        left -= l;
    }
    points[points.len() / 2]
}

fn label_texts(
    lines: &[Line],
    keep: impl Fn(f64) -> bool,
    font: f64,
    visible: bool,
    room: usize,
    who: &str,
) -> Result<Vec<TextObject>> {
    let mut texts = Vec::new();
    for (level, points) in lines.iter().filter(|(l, p)| !p.is_empty() && keep(*l)) {
        if texts.len() == room {
            return Err(format!(
                "{who}: the labels exceed the axes' {MAX_TEXTS} text objects"
            ));
        }
        let at = midpoint(points);
        let mut t = TextObject::new(at[0], at[1], format_g(*level));
        t.horizontal = TextAlign::Center;
        t.font_size = font;
        t.visible = visible;
        texts.push(t);
    }
    Ok(texts)
}

/// What a contour draws on its axes.
struct Built {
    series: Vec<Series>,
    texts: Vec<TextObject>,
    clim: Option<[f64; 2]>,
}

/// Series and labels of `rec` for an axes holding `axes` without the
/// contour's own objects. While the axes' CLim is automatic (unset, or the
/// value a contour stored) it becomes the span of every contour's levels on
/// the axes; any other CLim was set by the user and is kept.
fn build(
    rec: &mut ContourRec,
    axes: &Axes,
    sib: &Siblings,
    who: &str,
    cancel: &AtomicBool,
) -> Result<Built> {
    let auto = match axes.extras.clim {
        None => true,
        Some(c) => rec.auto_clim == Some(c) || sib.recorded.contains(&c),
    };
    let clim = if auto {
        merge_range(level_range(&rec.levels), sib.range).map(clim_of)
    } else {
        None
    };
    rec.auto_clim = if auto {
        clim.or(axes.extras.clim)
    } else {
        None
    };
    let limits = clim.or(axes.extras.clim).unwrap_or([0., 1.]);
    let map = axes.extras.effective_colormap();
    let color = |v: f64| map_color(&map, limits, v);
    let used_series = axes.series.len();
    let used_points: usize = axes.series.iter().map(|s| s.points.len()).sum();
    let used_faces: usize = axes
        .series
        .iter()
        .filter(|s| s.kind == PlotKind::Patch)
        .map(|s| s.points.len() / s.patch.map_or(3, |p| p.vertices_per_face()))
        .sum();
    let lines = contour_lines(&rec.z, &rec.grid, &rec.levels, who, cancel)?;
    let mut series = Vec::new();
    if rec.fill {
        let budget = MAX_PATCH_FACES.saturating_sub(used_faces).min(MAX_BUILD);
        let b = bands(&rec.z, &rec.grid, &rec.levels, budget, who, cancel)?;
        // Band k has the colour of its lower level (contourf's first level
        // is min(Z) unless the levels were given).
        for (band, (rects, tris)) in b.rects.into_iter().zip(b.tris).enumerate().skip(1) {
            let rgb = color(rec.levels[band - 1]);
            if !rects.is_empty() {
                series.push(p2_patch(rects, 4, rgb)?);
            }
            if !tris.is_empty() {
                series.push(p2_patch(tris, 3, rgb)?);
            }
        }
    }
    let mut points_total = 0usize;
    if rec.line_color != LineColor::Hidden && rec.pattern != LinePattern::None {
        let mut k = 0;
        for &level in &rec.levels {
            let mut points = Vec::new();
            while k < lines.len() && lines[k].0 == level {
                if !points.is_empty() {
                    points.push([f64::NAN; 2]);
                }
                points.extend_from_slice(&lines[k].1);
                k += 1;
            }
            if points.is_empty() {
                continue;
            }
            points_total += points.len();
            let rgb = match rec.line_color {
                LineColor::Rgb(rgb) => rgb,
                _ => color(level),
            };
            let z: Option<std::sync::Arc<[f64]>> =
                rec.three.then(|| vec![level; points.len()].into());
            let mut s = Series::new(points, PlotKind::Line);
            if let Some(z) = z {
                s.z = z;
            }
            s.color = rgb;
            s.explicit_color = true;
            s.line_width = Some(rec.line_width);
            s.pattern = rec.pattern;
            if let Some((shape, filled)) = rec.marker {
                s.markers = true;
                s.marker_shape = shape;
                s.marker_filled = filled;
            }
            if rec.scatter {
                s.kind = PlotKind::Scatter;
            }
            series.push(s);
        }
    }
    if series.is_empty() {
        // The handle needs a series: an empty (NaN) line.
        let mut s = Series::new(vec![[f64::NAN; 2]], PlotKind::Line);
        s.explicit_color = true;
        s.color = [0; 3];
        s.line_width = Some(rec.line_width);
        series.push(s);
    }
    if used_series + series.len() > MAX_SERIES {
        return Err(format!(
            "{who}: {} levels need {} series; an axes holds at most {MAX_SERIES} (use fewer levels)",
            rec.levels.len(),
            series.len()
        ));
    }
    if used_points + points_total > MAX_POINTS {
        return Err(format!(
            "{who}: the contour lines need {points_total} points; a figure holds at most {MAX_POINTS}"
        ));
    }
    for s in &mut series {
        s.visible = rec.visible;
    }
    let font = axes.extras.font_size.unwrap_or(10.);
    let room = MAX_TEXTS.saturating_sub(axes.extras.texts.len());
    let texts = match &rec.labels {
        _ if rec.three => Vec::new(),
        Labels::Off => Vec::new(),
        Labels::All => label_texts(&lines, |_| true, font, rec.visible, room, who)?,
        Labels::Levels(v) => label_texts(&lines, |l| v.contains(&l), font, rec.visible, room, who)?,
        Labels::Given(given) => label_texts(given, |_| true, font, rec.visible, room, who)?,
    };
    rec.texts = texts.iter().map(|t| t.identity.clone()).collect();
    rec.lines = lines;
    Ok(Built {
        series,
        texts,
        clim,
    })
}

// ------------------------------------------------------------- arguments

struct Parsed {
    rec: ContourRec,
    label: Option<String>,
}

fn parse_chart(e: &Engine, args: &[Value], fill: bool, who: &str) -> Result<Parsed> {
    let (positional, rest) = p2_split(e, args, NAMES);
    let mut numeric = Vec::new();
    let mut spec: Option<String> = None;
    for a in positional {
        match p2_text(e, a) {
            Some(_) if numeric.is_empty() => {
                return Err(format!("{who}: Z must be a real numeric matrix"));
            }
            Some(t) => {
                if spec.is_some() {
                    return Err(format!("{who}: expected one line specification"));
                }
                spec = Some(t);
            }
            None => {
                if spec.is_some() {
                    return Err(format!("{who}: data must precede the line specification"));
                }
                numeric.push(a);
            }
        }
    }
    let (z, grid, level_arg) = match numeric.as_slice() {
        [z] => {
            let z = z_matrix(z, who)?;
            let g = index_grid(&z);
            (z, g, None)
        }
        [z, n] => {
            let z = z_matrix(z, who)?;
            let g = index_grid(&z);
            (z, g, Some(*n))
        }
        [x, y, z] => {
            let z = z_matrix(z, who)?;
            let g = grid_of(x, y, &z, who)?;
            (z, g, None)
        }
        [x, y, z, n] => {
            let z = z_matrix(z, who)?;
            let g = grid_of(x, y, &z, who)?;
            (z, g, Some(*n))
        }
        [] => return Err(format!("{who} requires input data")),
        _ => {
            return Err(format!(
                "Use {who}(Z), {who}(X,Y,Z), {who}(...,n) or {who}(...,v), then a line specification and Name,Value pairs"
            ));
        }
    };
    let mut levels = match level_arg {
        Some(v) => levels_arg(v, &z, fill, who)?,
        None => auto_levels(z.data(), DEFAULT_LEVELS, fill),
    };
    let mut rec = ContourRec {
        z,
        grid,
        levels: Vec::new(),
        fill,
        line_color: if fill {
            LineColor::Rgb([0; 3])
        } else {
            LineColor::Flat
        },
        line_width: LINE_WIDTH,
        pattern: LinePattern::Solid,
        marker: None,
        scatter: false,
        labels: Labels::Off,
        texts: Vec::new(),
        auto_clim: None,
        visible: true,
        lines: Vec::new(),
        three: who == "contour3",
    };
    if let Some(spec) = spec {
        let mut probe = Series::new(Vec::new(), PlotKind::Line);
        probe.line_spec(&spec)?;
        rec.pattern = probe.pattern;
        if probe.explicit_color {
            rec.line_color = LineColor::Rgb(probe.color);
        }
        if probe.markers {
            rec.marker = Some((probe.marker_shape, probe.marker_filled));
        }
        rec.scatter = probe.kind == PlotKind::Scatter;
    }
    let mut label = None;
    for (name, value) in p2_pairs(e, rest, who)? {
        match name.as_str() {
            "levellist" => {
                let v = p2_vec(value, who)?;
                if v.is_empty() {
                    return Err(format!("{who}: LevelList must not be empty"));
                }
                levels = explicit_levels(&v, who)?;
            }
            "displayname" => {
                label = Some(p2_text(e, value).ok_or(format!("{who}: DisplayName must be text"))?)
            }
            _ => apply_style(e, &mut rec, &name, value, who)?,
        }
    }
    rec.levels = levels;
    Ok(Parsed { rec, label })
}

/// LineColor, LineWidth, LineStyle and ShowText.
fn apply_style(
    e: &Engine,
    rec: &mut ContourRec,
    name: &str,
    value: &Value,
    who: &str,
) -> Result<()> {
    match name {
        "linecolor" => {
            rec.line_color = match p2_text(e, value).map(|s| s.to_ascii_lowercase()).as_deref() {
                Some("none") => LineColor::Hidden,
                Some("flat") => LineColor::Flat,
                _ => LineColor::Rgb(p2_color(e, value)?),
            }
        }
        "linewidth" => rec.line_width = p2_line_width(value)?,
        "linestyle" => rec.pattern = p2_line_style(e, value)?,
        "showtext" => {
            rec.labels = if p2_on_off(e, value, "ShowText")? {
                Labels::All
            } else {
                Labels::Off
            }
        }
        _ => return Err(format!("{who}: unsupported property '{name}'")),
    }
    Ok(())
}

// --------------------------------------------------------------- dispatch

pub(super) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    e.check_cancel()?;
    let max = if matches!(name, "contour" | "contourf" | "contour3") {
        2
    } else {
        1
    };
    if count > max {
        return Err(format!(
            "{name} returns at most {max} output{}",
            if max > 1 { "s" } else { "" }
        ));
    }
    let mut args = e.text_arguments(args.to_vec())?;
    if name == "contourc" {
        let c = contourc(e, &args)?;
        return Ok(vec![c]);
    }
    if let Some(Value::Container(Array::Graphics(h))) = args.first()
        && h.len() == 1
        && h.scalar_handle()?.kind() == GraphicsKind::Axes
    {
        let target = args.remove(0);
        let Value::Container(Array::Graphics(h)) = &target else {
            unreachable!()
        };
        let h = h.scalar_handle()?;
        if super::super::ui_components::is_uiaxes_fig_id(e.locate_graphics(h)?.figure) {
            // axes(uiaxes) cannot select a uiaxes host; draw through it instead.
            return e.with_axes(h, |e| call(e, name, &args, count, out));
        }
        e.call("axes", vec![target], out)?;
    }
    if args.is_empty() {
        return Err(format!("{name} requires input data"));
    }
    let old = (e.figure.clone(), e.p2.clone());
    let result = match name {
        "clabel" => clabel(e, &args).map(|h| vec![h]),
        _ => chart(e, &args, name == "contourf", name),
    };
    match result {
        Ok(mut v) => {
            v.truncate(count);
            Ok(v)
        }
        Err(error) => {
            (e.figure, e.p2) = old;
            Err(error)
        }
    }
}

fn contourc(e: &Engine, args: &[Value]) -> Result<Value> {
    let who = "contourc";
    if args.iter().any(|a| p2_text(e, a).is_some()) {
        return Err("contourc: X, Y, and Z must be real numeric matrices".into());
    }
    let (z, level_arg) = match args {
        [z] | [_, _, z] => (z_matrix(z, who)?, None),
        [z, n] | [_, _, z, n] => (z_matrix(z, who)?, Some(n)),
        _ => {
            return Err(
                "Use C = contourc(Z), contourc(Z,n|v), contourc(x,y,Z) or contourc(x,y,Z,n|v)"
                    .into(),
            );
        }
    };
    let grid = match args {
        [x, y, _] | [x, y, _, _] => grid_of(x, y, &z, who)?,
        _ => index_grid(&z),
    };
    let levels = match level_arg {
        Some(v) => levels_arg(v, &z, false, who)?,
        None => auto_levels(z.data(), DEFAULT_LEVELS, false),
    };
    let lines = contour_lines(&z, &grid, &levels, who, &e.cancel)?;
    Ok(Value::Matrix(c_matrix(&lines)?))
}

fn chart(e: &mut Engine, args: &[Value], fill: bool, who: &str) -> Result<Vec<Value>> {
    let Parsed { mut rec, label } = parse_chart(e, args, fill, who)?;
    let previous = e.figure.clone();
    let mut figure = e.p2_target();
    crate::engine::graphics_handles::plot_carry_over(previous.as_ref(), &mut figure, &mut [])?;
    // A replacing contour3 starts in view(3), as plot3 does.
    if rec.three && !previous.as_ref().is_some_and(|f| f.hold) {
        figure.axes.view = crate::plot::view3d::VIEW_3D;
    }
    let sib = siblings(e, &figure.axes);
    let built = build(&mut rec, &figure.axes, &sib, who, &e.cancel)?;
    let c = c_matrix(&rec.lines)?;
    let Built {
        mut series,
        texts,
        clim,
    } = built;
    if let Some(label) = label {
        series[0].label = label;
    }
    if clim.is_some() {
        figure.axes.extras.clim = clim;
    }
    figure.axes.extras.texts.extend(texts);
    // Octave 10.3 contourf sets Layer top, held or not, so ticks stay visible over the bands.
    if fill {
        figure.axes.extras.layer = crate::plot::AxesLayer::Top;
    }
    let (figure_id, axes_id) = (figure.identity.clone(), figure.axes.identity.clone());
    let ctx = P2Ctx::of(&figure, e.graphics_theme_context);
    let draw = P2Draw {
        charts: vec![P2New {
            series,
            kind: Some(P2Kind::Contour(Box::new(rec))),
        }],
        ..P2Draw::default()
    };
    let h = e.p2_publish(figure, &ctx, draw)?;
    if let Some(clim) = clim {
        refresh(e, &figure_id, &axes_id, &sib.primaries, clim)?;
    }
    Ok(vec![Value::Matrix(c), h])
}

/// Redraw the other contours of an axes whose colours were mapped with a
/// different automatic CLim than `clim`, the span of all of them.
fn refresh(
    e: &mut Engine,
    figure: &ObjectIdentity,
    axes: &ObjectIdentity,
    primaries: &[ObjectIdentity],
    clim: [f64; 2],
) -> Result<()> {
    let number = super::p2_all_figures(e)
        .find(|(_, f)| f.identity.same_object(figure))
        .map(|(n, _)| n)
        .ok_or("contour: the figure is no longer open")?;
    for primary in primaries {
        let Some(index) =
            e.p2.charts
                .iter()
                .position(|c| c.primary.same_object(primary))
        else {
            continue;
        };
        match &e.p2.charts[index].kind {
            P2Kind::Contour(rec) if rec.auto_clim != Some(clim) => {}
            _ => continue,
        }
        rebuild(e, index, number, axes, false, |_| Ok(()))?;
    }
    Ok(())
}

/// clabel(C), clabel(C, v), clabel(C, h), clabel(C, h, v).
fn clabel(e: &mut Engine, args: &[Value]) -> Result<Value> {
    let who = "clabel";
    let c = match args.first() {
        Some(v @ (Value::Matrix(_) | Value::Logical(_))) => real_matrix(v, who)?,
        _ => return Err("clabel: C must be a contour matrix".into()),
    };
    let lines = parse_c(&c, who)?;
    let handle = match args.get(1) {
        Some(Value::Container(Array::Graphics(h))) if h.len() == 1 => {
            Some(h.scalar_handle()?.clone())
        }
        Some(Value::Container(Array::Graphics(_))) => {
            return Err("clabel: h must be one contour handle".into());
        }
        _ => None,
    };
    let rest = &args[1 + usize::from(handle.is_some())..];
    let only = match rest {
        [] => None,
        [v] => Some(explicit_levels(&p2_vec(v, who)?, who)?),
        _ => return Err("Use clabel(C), clabel(C,v), clabel(C,h) or clabel(C,h,v)".into()),
    };
    if let Some(h) = handle {
        let index = e
            .p2_find(&h)
            .filter(|&i| matches!(e.p2.charts[i].kind, P2Kind::Contour(_)))
            .ok_or("clabel: h must be a contour handle")?;
        let P2Kind::Contour(own) = &e.p2.charts[index].kind else {
            unreachable!()
        };
        // The contour's own C labels like the contour; any other C labels its lines.
        let labels = if lines == own.lines {
            match only {
                Some(v) => Labels::Levels(v),
                None => Labels::All,
            }
        } else {
            Labels::Given(
                lines
                    .into_iter()
                    .filter(|(l, _)| only.as_ref().is_none_or(|v| v.contains(l)))
                    .collect(),
            )
        };
        let number = super::p2_all_figures(e)
            .find(|(_, f)| f.identity.same_object(&h.figure))
            .map(|(n, _)| n)
            .ok_or("clabel: the contour's figure is no longer open")?;
        let axes_id = h.axes.clone().ok_or("clabel: the contour has no axes")?;
        rebuild(e, index, number, &axes_id, true, |rec| {
            rec.labels = labels;
            Ok(())
        })?;
        let P2Kind::Contour(rec) = &e.p2.charts[index].kind else {
            unreachable!()
        };
        return text_handles(&h.figure, &axes_id, &rec.texts, &e.cancel);
    }
    // No handle: label the current axes from C alone.
    let mut figure = match e.figure.clone() {
        Some(f) => f,
        None => e.p2_target(),
    };
    let font = figure.axes.extras.font_size.unwrap_or(10.);
    let room = MAX_TEXTS.saturating_sub(figure.axes.extras.texts.len());
    let texts = match &only {
        Some(v) => label_texts(&lines, |l| v.contains(&l), font, true, room, who)?,
        None => label_texts(&lines, |_| true, font, true, room, who)?,
    };
    let ids: Vec<ObjectIdentity> = texts.iter().map(|t| t.identity.clone()).collect();
    figure.axes.extras.texts.extend(texts);
    figure.validate()?;
    let axes_id = figure.axes.identity.clone();
    let value = text_handles(&figure.identity, &axes_id, &ids, &e.cancel)?;
    e.figure = Some(figure);
    Ok(value)
}

fn text_handles(
    figure: &ObjectIdentity,
    axes: &ObjectIdentity,
    ids: &[ObjectIdentity],
    cancel: &AtomicBool,
) -> Result<Value> {
    let handles: Vec<Arc<GraphicsHandle>> = ids
        .iter()
        .map(|id| {
            Arc::new(GraphicsHandle::axes_object(
                figure,
                axes,
                id,
                GraphicsKind::Text,
            ))
        })
        .collect();
    Ok(Value::Container(
        GraphicsArray::from_shared(handles.len(), 1, handles, cancel)?.into(),
    ))
}

/// Apply `change` to contour record `index` and redraw it in place: its
/// series keep their place in the axes, the primary keeps its identity.
///
/// With `cascade`, the other contours of the axes are redrawn when this one
/// moved the automatic CLim.
fn rebuild(
    e: &mut Engine,
    index: usize,
    figure_number: u32,
    axes_id: &ObjectIdentity,
    cascade: bool,
    change: impl FnOnce(&mut ContourRec) -> Result<()>,
) -> Result<()> {
    let mut figure = e.p2_owned_figure(figure_number, axes_id)?;
    let chart = e.p2.charts[index].clone();
    let P2Kind::Contour(mut rec) = chart.kind else {
        return Err("Not a contour object".into());
    };
    change(&mut rec)?;
    let is_member = |s: &Series| {
        s.identity.same_object(&chart.primary)
            || chart.helpers.iter().any(|h| s.identity.same_object(h))
    };
    let position = figure
        .axes
        .series
        .iter()
        .position(is_member)
        .ok_or("Contour is no longer live")?;
    let primary_index = figure
        .axes
        .series
        .iter()
        .position(|s| s.identity.same_object(&chart.primary))
        .ok_or("Contour is no longer live")?;
    let primary_label = figure.axes.series[primary_index].label.clone();
    let mut axes = figure.axes.clone();
    axes.series.retain(|s| !is_member(s));
    axes.extras
        .texts
        .retain(|t| !rec.texts.iter().any(|id| id.same_object(&t.identity)));
    let sib = siblings(e, &axes);
    let Built {
        mut series,
        texts,
        clim,
    } = build(&mut rec, &axes, &sib, "contour", &e.cancel)?;
    series[0].identity = chart.primary.clone();
    series[0].label = primary_label;
    // Helpers are never legend members (as in p2_publish).
    for (k, s) in series.iter_mut().enumerate() {
        s.legend_hidden = k != 0;
    }
    let helpers: Vec<ObjectIdentity> = series[1..].iter().map(|s| s.identity.clone()).collect();
    let tail = axes.series.split_off(position.min(axes.series.len()));
    axes.series.extend(series);
    axes.series.extend(tail);
    if let Some(state) = &mut axes.legend_state {
        let live: Vec<ObjectIdentity> = axes.series.iter().map(|s| s.identity.clone()).collect();
        state
            .members
            .retain(|m| live.iter().any(|l| l.same_object(m)));
    }
    if clim.is_some() {
        axes.extras.clim = clim;
    }
    axes.extras.texts.extend(texts);
    figure.axes = axes;
    figure.validate()?;
    e.p2.charts[index] = P2Chart {
        primary: chart.primary,
        helpers,
        kind: P2Kind::Contour(rec),
    };
    let figure_id = figure.identity.clone();
    e.p2_store_figure(figure_number, figure);
    if let (true, Some(clim)) = (cascade, clim) {
        refresh(e, &figure_id, axes_id, &sib.primaries, clim)?;
    }
    Ok(())
}

// -------------------------------------------------------------- get / set

pub(super) fn get(rec: &ContourRec, name: &str) -> Result<Option<Value>> {
    Ok(Some(match name {
        "levellist" => p2_row(&rec.levels)?,
        "linecolor" => match rec.line_color {
            LineColor::Flat => p2_str("flat"),
            LineColor::Hidden => p2_str("none"),
            LineColor::Rgb(rgb) => p2_rgb_value(rgb)?,
        },
        "linewidth" => Value::Matrix(Matrix::scalar(rec.line_width)),
        "linestyle" => p2_str(match rec.pattern {
            LinePattern::Solid => "-",
            LinePattern::Dashed => "--",
            LinePattern::Dotted => ":",
            LinePattern::DashDot => "-.",
            LinePattern::None => "none",
        }),
        "showtext" => p2_str(if rec.labels == Labels::Off {
            "off"
        } else {
            "on"
        }),
        "fill" => p2_str(if rec.fill { "on" } else { "off" }),
        "zdata" => Value::Matrix(rec.z.clone()),
        "xdata" => match &rec.grid {
            Grid::Tensor { x, .. } => p2_row(x)?,
            Grid::Mesh { x, .. } => Value::Matrix(x.clone()),
        },
        "ydata" => match &rec.grid {
            Grid::Tensor { y, .. } => Value::Matrix(Matrix::from_column_slice(y.len(), 1, y)?),
            Grid::Mesh { y, .. } => Value::Matrix(y.clone()),
        },
        "contourmatrix" => Value::Matrix(c_matrix(&rec.lines)?),
        "visible" => p2_str(if rec.visible { "on" } else { "off" }),
        _ => return Ok(None),
    }))
}

const SETTABLE: &[&str] = &[
    "levellist",
    "linecolor",
    "linewidth",
    "linestyle",
    "showtext",
    "zdata",
    "visible",
    "fill",
];

/// set on a contour handle; Ok(false) when no pair is a contour property.
pub(super) fn set(
    e: &mut Engine,
    index: usize,
    h: &GraphicsHandle,
    figure_number: u32,
    pairs: &[(String, Value)],
) -> Result<bool> {
    let mine = pairs
        .iter()
        .filter(|(n, _)| SETTABLE.contains(&n.as_str()))
        .count();
    if mine == 0 {
        return Ok(false);
    }
    if mine != pairs.len() {
        return Err(
            "Set contour properties in their own set call, separate from generic series properties"
                .into(),
        );
    }
    if pairs.iter().any(|(n, _)| n == "fill") {
        return Err("contour: Fill is read-only (use contour or contourf)".into());
    }
    let axes_id = h.axes.clone().ok_or("Chart handle has no axes")?;
    let values: Vec<(String, Value)> = pairs.to_vec();
    let engine: &Engine = e;
    // Validate and stage the changes against a copy, then rebuild.
    let P2Kind::Contour(staged) = &engine.p2.charts[index].kind else {
        return Ok(false);
    };
    let mut staged = (**staged).clone();
    for (name, value) in &values {
        match name.as_str() {
            "levellist" => {
                let v = p2_vec(value, "set")?;
                if v.is_empty() {
                    return Err("set: LevelList must not be empty".into());
                }
                staged.levels = explicit_levels(&v, "set")?;
            }
            "zdata" => {
                let z = z_matrix(value, "set")?;
                match &staged.grid {
                    Grid::Tensor {
                        explicit: false, ..
                    } => staged.grid = index_grid(&z),
                    _ if (z.rows(), z.cols()) != (staged.z.rows(), staged.z.cols()) => {
                        return Err("set: ZData must keep the size of the X and Y data".into());
                    }
                    _ => {}
                }
                staged.z = z;
            }
            "visible" => staged.visible = p2_on_off(engine, value, "Visible")?,
            _ => apply_style(engine, &mut staged, name, value, "set")?,
        }
    }
    rebuild(e, index, figure_number, &axes_id, true, move |rec| {
        let (texts, auto_clim) = (rec.texts.clone(), rec.auto_clim);
        *rec = staged;
        rec.texts = texts;
        rec.auto_clim = auto_clim;
        Ok(())
    })?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn octave_linspace_is_symmetric_and_exact_at_the_ends() {
        let v = linspace(-3., 7., 12);
        assert_eq!((v[0], v[11]), (-3., 7.));
        assert_eq!(v[1], -3. + 10. / 11.);
        assert_eq!(v[10], 7. - 10. / 11.);
        assert_eq!(linspace(-2., 2., 5)[2], 0.);
    }

    #[test]
    fn percent_g_matches_c() {
        for (v, s) in [
            (0.5, "0.5"),
            (1e-5, "1e-05"),
            (123456., "123456"),
            (1234567., "1.23457e+06"),
            (-0.363_636_363_636_363_3, "-0.363636"),
            (3., "3"),
        ] {
            assert_eq!(format_g(v), s);
        }
    }

    #[test]
    fn bands_cover_the_cells() {
        let z = Matrix::from_column_slice(2, 2, &[0., 1., 1., 2.]).unwrap();
        let grid = index_grid(&z);
        let area = |levels: &[f64]| {
            let b = bands(&z, &grid, levels, 100, "t", &AtomicBool::new(false)).unwrap();
            assert!(b.rects[0].is_empty() && b.tris[0].is_empty());
            b.tris
                .iter()
                .flat_map(|t| t.chunks(3))
                .map(|t| twice_area(t[0], t[1], t[2]).abs() / 2.)
                .sum::<f64>()
        };
        // With min(Z) as the first level the bands cover the cell; below an
        // explicit first level (z < 0.5: the corner triangle) nothing is filled.
        assert!((area(&[0., 0.5, 1.5]) - 1.).abs() < 1e-12);
        assert!((area(&[0.5, 1.5]) - 0.875).abs() < 1e-12);
    }
}
