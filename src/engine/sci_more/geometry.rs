//! Triangulation, hulls, point searches, Voronoi diagrams and scattered
//! interpolation in two dimensions (delaunay, delaunayn, convhull, convhulln,
//! dsearchn, tsearchn, voronoin, griddata, griddatan).
use super::{Grid, MAX_PAIRS, MAX_POINTS, NO_ND, NOT_ENOUGH, sc2_num, sc2_scalar, sc2_text};
use crate::engine::{Engine, Execution, Value};
use crate::matrix::MAX_ELEMENTS;
use crate::{Array, CellArray, Matrix, Result};
use std::collections::HashMap;
use std::sync::Arc;

type P = [f64; 2];
/// The vertex at infinity of the ghost triangles outside the hull.
const INF: usize = usize::MAX;
const NONE: usize = usize::MAX;

fn orient(a: P, b: P, c: P) -> f64 {
    robust::orient2d(
        robust::Coord { x: a[0], y: a[1] },
        robust::Coord { x: b[0], y: b[1] },
        robust::Coord { x: c[0], y: c[1] },
    )
}

/// Positive when d is inside the circle through the counter-clockwise a, b, c.
fn incircle(a: P, b: P, c: P, d: P) -> f64 {
    robust::incircle(
        robust::Coord { x: a[0], y: a[1] },
        robust::Coord { x: b[0], y: b[1] },
        robust::Coord { x: c[0], y: c[1] },
        robust::Coord { x: d[0], y: d[1] },
    )
}

/// d is inside the circle through the counter-clockwise a, b, c; four
/// co-circular points are decided by CGAL's symbolic perturbation (the
/// lexicographically largest point is perturbed outward), as the reference's
/// delaunayTriangulation and scatteredInterpolant do.
fn in_circle(a: P, b: P, c: P, d: P) -> bool {
    let s = incircle(a, b, c, d);
    if s != 0. {
        return s > 0.;
    }
    let mut order = [(a, 0), (b, 1), (c, 2), (d, 3)];
    order.sort_by(|x, y| x.0[0].total_cmp(&y.0[0]).then(x.0[1].total_cmp(&y.0[1])));
    for i in [3, 2] {
        let o = match order[i].1 {
            3 => return false,
            2 => orient(a, b, d),
            1 => orient(a, d, c),
            _ => orient(d, b, c),
        };
        if o != 0. {
            return o > 0.;
        }
    }
    false
}

fn circumcenter(a: P, b: P, c: P) -> P {
    let (bx, by) = (b[0] - a[0], b[1] - a[1]);
    let (cx, cy) = (c[0] - a[0], c[1] - a[1]);
    let d = 2. * (bx * cy - by * cx);
    let b2 = bx * bx + by * by;
    let c2 = cx * cx + cy * cy;
    [
        a[0] + (cy * b2 - by * c2) / d,
        a[1] + (bx * c2 - cx * b2) / d,
    ]
}

/// p lies strictly between a and b on their line.
fn between(a: P, b: P, p: P) -> bool {
    let dot1 = (p[0] - a[0]) * (b[0] - a[0]) + (p[1] - a[1]) * (b[1] - a[1]);
    let dot2 = (p[0] - b[0]) * (a[0] - b[0]) + (p[1] - b[1]) * (a[1] - b[1]);
    dot1 > 0. && dot2 > 0.
}

#[derive(Clone)]
struct Tri {
    v: [usize; 3],
    /// n[k]: the neighbour across the edge opposite v[k].
    n: [usize; 3],
}

/// Incremental (Bowyer-Watson) Delaunay triangulation with ghost triangles
/// `(a, b, INF)` beyond every hull edge; points are inserted in index order.
/// A point on a circumcircle counts as inside, so four co-circular points
/// join the later point to its predecessors as the reference's triangulations do.
pub(super) struct Delaunay {
    pts: Vec<P>,
    tris: Vec<Tri>,
    alive: Vec<bool>,
    last: usize,
}

impl Delaunay {
    /// `None` when the points do not span two dimensions.
    fn build(pts: Vec<P>, order: &[usize], engine: &Engine) -> Result<Option<Self>> {
        let Some(&i0) = order.first() else {
            return Ok(None);
        };
        let Some(&i1) = order.iter().find(|i| pts[**i] != pts[i0]) else {
            return Ok(None);
        };
        let Some(&i2) = order
            .iter()
            .find(|i| orient(pts[i0], pts[i1], pts[**i]) != 0.)
        else {
            return Ok(None);
        };
        let (b, c) = if orient(pts[i0], pts[i1], pts[i2]) > 0. {
            (i1, i2)
        } else {
            (i2, i1)
        };
        let mut d = Delaunay {
            pts,
            tris: Vec::new(),
            alive: Vec::new(),
            last: 0,
        };
        let first = [[i0, b, c], [b, i0, INF], [c, b, INF], [i0, c, INF]];
        for v in first {
            d.tris.push(Tri { v, n: [NONE; 3] });
            d.alive.push(true);
        }
        d.link(&[0, 1, 2, 3]);
        for (k, &i) in order.iter().enumerate() {
            if k % 256 == 0 {
                engine.check_cancel()?;
            }
            if i != i0 && i != b && i != c {
                d.insert(i);
            }
        }
        Ok(Some(d))
    }

    /// Links the edges of `new` triangles to each other.
    fn link(&mut self, new: &[usize]) {
        let mut edges = HashMap::with_capacity(new.len() * 3);
        for &t in new {
            let v = self.tris[t].v;
            for k in 0..3 {
                edges.insert((v[(k + 1) % 3], v[(k + 2) % 3]), t);
            }
        }
        for &t in new {
            let v = self.tris[t].v;
            for k in 0..3 {
                if self.tris[t].n[k] == NONE
                    && let Some(&s) = edges.get(&(v[(k + 2) % 3], v[(k + 1) % 3]))
                {
                    self.tris[t].n[k] = s;
                }
            }
        }
    }

    fn is_ghost(&self, t: usize) -> bool {
        self.tris[t].v[2] == INF
    }

    fn conflict(&self, t: usize, p: P) -> bool {
        let [a, b, c] = self.tris[t].v;
        if c == INF {
            let o = orient(self.pts[a], self.pts[b], p);
            o > 0. || (o == 0. && between(self.pts[a], self.pts[b], p))
        } else {
            in_circle(self.pts[a], self.pts[b], self.pts[c], p)
        }
    }

    /// A real triangle containing p (boundary included), or the ghost
    /// triangle of a hull edge that p lies beyond.
    fn locate(&self, p: P) -> usize {
        let mut t = self.last;
        if !self.alive[t] {
            t = (0..self.tris.len())
                .rev()
                .find(|t| self.alive[*t])
                .unwrap_or(0);
        }
        if self.is_ghost(t) {
            t = self.tris[t].n[2];
        }
        let limit = 4 * self.tris.len() + 64;
        'walk: for step in 0..limit {
            let v = self.tris[t].v;
            for j in 0..3 {
                let k = (j + step) % 3;
                let (u, w) = (v[(k + 1) % 3], v[(k + 2) % 3]);
                if orient(self.pts[u], self.pts[w], p) < 0. {
                    t = self.tris[t].n[k];
                    if self.is_ghost(t) {
                        return t;
                    }
                    continue 'walk;
                }
            }
            return t;
        }
        // Not reached for valid triangulations; scan as a safeguard.
        (0..self.tris.len())
            .find(|t| self.alive[*t] && !self.is_ghost(*t) && self.contains(*t, p))
            .or_else(|| (0..self.tris.len()).find(|t| self.alive[*t] && self.conflict(*t, p)))
            .unwrap_or(0)
    }

    fn contains(&self, t: usize, p: P) -> bool {
        let v = self.tris[t].v;
        (0..3).all(|k| orient(self.pts[v[k]], self.pts[v[(k + 1) % 3]], p) >= 0.)
    }

    fn insert(&mut self, i: usize) {
        let p = self.pts[i];
        let start = self.locate(p);
        let start = if self.conflict(start, p) {
            start
        } else {
            match (0..self.tris.len()).find(|t| self.alive[*t] && self.conflict(*t, p)) {
                Some(t) => t,
                None => return,
            }
        };
        let mut cavity = vec![start];
        let mut state: HashMap<usize, bool> = HashMap::new();
        state.insert(start, true);
        let mut boundary = Vec::new();
        let mut k = 0;
        while k < cavity.len() {
            let t = cavity[k];
            k += 1;
            for e in 0..3 {
                let s = self.tris[t].n[e];
                let inside = match state.get(&s) {
                    Some(inside) => *inside,
                    None => {
                        let inside = self.conflict(s, p);
                        state.insert(s, inside);
                        if inside {
                            cavity.push(s);
                        }
                        inside
                    }
                };
                if !inside {
                    let v = self.tris[t].v;
                    boundary.push((v[(e + 1) % 3], v[(e + 2) % 3], s));
                }
            }
        }
        for &t in &cavity {
            self.alive[t] = false;
        }
        let mut new = Vec::with_capacity(boundary.len());
        for (a, b, outer) in boundary {
            let v = if a == INF {
                [b, i, INF]
            } else if b == INF {
                [i, a, INF]
            } else {
                [a, b, i]
            };
            let t = self.tris.len();
            let mut n = [NONE; 3];
            // The edge a-b is opposite the new point.
            let slot = v.iter().position(|x| *x == i).expect("new vertex");
            n[slot] = outer;
            self.tris.push(Tri { v, n });
            self.alive.push(true);
            let ov = self.tris[outer].v;
            for e in 0..3 {
                if ov[(e + 1) % 3] == b && ov[(e + 2) % 3] == a {
                    self.tris[outer].n[e] = t;
                }
            }
            new.push(t);
        }
        self.link(&new);
        self.last = *new.iter().find(|t| !self.is_ghost(**t)).unwrap_or(&new[0]);
    }

    fn triangles(&self) -> Vec<[usize; 3]> {
        (0..self.tris.len())
            .filter(|t| self.alive[*t] && !self.is_ghost(*t))
            .map(|t| self.tris[t].v)
            .collect()
    }
}

/// Counter-clockwise rows, each starting at its smallest index, ordered by
/// their sorted indices from the largest down (Qhull's facet order in the
/// corpus).
fn ordered_rows(mut rows: Vec<[usize; 3]>) -> Vec<[usize; 3]> {
    for r in &mut rows {
        let k = (0..3).min_by_key(|k| r[*k]).expect("three");
        r.rotate_left(k);
    }
    let key = |r: &[usize; 3]| {
        let mut s = *r;
        s.sort_unstable();
        [s[2], s[1], s[0]]
    };
    rows.sort_by_key(key);
    rows
}

fn rows_value(rows: &[[usize; 3]]) -> Result<Value> {
    let m = rows.len();
    let mut data = vec![0.; m * 3];
    for (i, r) in rows.iter().enumerate() {
        for k in 0..3 {
            data[k * m + i] = (r[k] + 1) as f64;
        }
    }
    sc2_num(m, 3, data)
}

/// Unique points: the index of each point's first occurrence, in order.
fn first_occurrences(pts: &[P]) -> (Vec<usize>, Vec<usize>) {
    let mut seen: HashMap<(u64, u64), usize> = HashMap::with_capacity(pts.len());
    let mut unique = Vec::with_capacity(pts.len());
    let mut owner = Vec::with_capacity(pts.len());
    for (i, p) in pts.iter().enumerate() {
        // +0.0 and -0.0 are the same point.
        let key = ((p[0] + 0.).to_bits(), (p[1] + 0.).to_bits());
        let o = *seen.entry(key).or_insert_with(|| {
            unique.push(i);
            i
        });
        owner.push(o);
    }
    (unique, owner)
}

fn cross_hull(pts: &[P], order: &[usize], keep_collinear: bool) -> Vec<usize> {
    // Andrew's monotone chain, counter-clockwise, strict corners.
    let mut idx = order.to_vec();
    idx.sort_by(|a, b| {
        pts[*a][0]
            .total_cmp(&pts[*b][0])
            .then(pts[*a][1].total_cmp(&pts[*b][1]))
    });
    let mut hull: Vec<usize> = Vec::with_capacity(idx.len() + 1);
    for pass in 0..2 {
        let start = hull.len();
        let iter: Box<dyn Iterator<Item = &usize>> = if pass == 0 {
            Box::new(idx.iter())
        } else {
            Box::new(idx.iter().rev())
        };
        for &i in iter {
            while hull.len() >= start + 2
                && orient(pts[hull[hull.len() - 2]], pts[hull[hull.len() - 1]], pts[i]) <= 0.
            {
                hull.pop();
            }
            hull.push(i);
        }
        hull.pop();
    }
    if !keep_collinear {
        return hull;
    }
    let mut full = Vec::with_capacity(hull.len());
    for k in 0..hull.len() {
        let (a, b) = (hull[k], hull[(k + 1) % hull.len()]);
        full.push(a);
        let mut on: Vec<(f64, usize)> = order
            .iter()
            .filter(|i| **i != a && **i != b)
            .filter(|i| orient(pts[a], pts[b], pts[**i]) == 0. && between(pts[a], pts[b], pts[**i]))
            .map(|i| {
                let d = (pts[*i][0] - pts[a][0]).abs() + (pts[*i][1] - pts[a][1]).abs();
                (d, *i)
            })
            .collect();
        on.sort_by(|x, y| x.0.total_cmp(&y.0));
        full.extend(on.into_iter().map(|(_, i)| i));
    }
    full
}

fn polygon_area(pts: &[P], ring: &[usize]) -> f64 {
    let mut s = 0.;
    for k in 0..ring.len() {
        let (a, b) = (pts[ring[k]], pts[ring[(k + 1) % ring.len()]]);
        s += a[0] * b[1] - b[0] * a[1];
    }
    s.abs() / 2.
}

/// Solves the dense (n x n) system a x = b by Gaussian elimination with
/// partial pivoting; `None` when singular.
fn solve_small(mut a: Vec<f64>, mut b: Vec<f64>, n: usize) -> Option<Vec<f64>> {
    for col in 0..n {
        let pivot =
            (col..n).max_by(|x, y| a[x * n + col].abs().total_cmp(&a[y * n + col].abs()))?;
        if a[pivot * n + col] == 0. {
            return None;
        }
        if pivot != col {
            for k in 0..n {
                a.swap(col * n + k, pivot * n + k);
            }
            b.swap(col, pivot);
        }
        for r in col + 1..n {
            let f = a[r * n + col] / a[col * n + col];
            if f != 0. {
                for k in col..n {
                    a[r * n + k] -= f * a[col * n + k];
                }
                b[r] -= f * b[col];
            }
        }
    }
    let mut x = vec![0.; n];
    for r in (0..n).rev() {
        let mut s = b[r];
        for k in r + 1..n {
            s -= a[r * n + k] * x[k];
        }
        x[r] = s / a[r * n + r];
    }
    Some(x)
}

impl Engine {
    fn sc2_points(&self, value: &Value, what: &str) -> Result<Grid> {
        let g = self.sc2_grid(value, what)?;
        if g.rows > MAX_POINTS {
            return Err(format!("{what} has more than {MAX_POINTS} points"));
        }
        Ok(g)
    }

    fn sc2_pairs(&mut self, a: usize, b: usize) -> Result<()> {
        if a.saturating_mul(b) > MAX_PAIRS {
            return Err(format!(
                "The search compares more than {MAX_PAIRS} point pairs; use fewer points"
            ));
        }
        self.charge(a.saturating_add(b))
    }

    fn sc2_rows2(&self, g: &Grid) -> Vec<P> {
        (0..g.rows).map(|r| [g.at(r, 0), g.at(r, 1)]).collect()
    }

    /// The Delaunay triangulation of 2-D points (first occurrences only);
    /// `None` when the points are collinear.
    fn sc2_triangulate(&self, pts: &[P]) -> Result<Option<(Delaunay, Vec<[usize; 3]>)>> {
        if pts.iter().any(|p| !p[0].is_finite() || !p[1].is_finite()) {
            return Err("Data points must be finite.".into());
        }
        let (unique, _) = first_occurrences(pts);
        let Some(d) = Delaunay::build(pts.to_vec(), &unique, self)? else {
            return Ok(None);
        };
        let rows = ordered_rows(d.triangles());
        Ok(Some((d, rows)))
    }

    // -- delaunay / delaunayn ---------------------------------------------

    pub(super) fn sc2_delaunay(&mut self, args: &[Value]) -> Result<Value> {
        let pts = match args.len() {
            0 => return Err(NOT_ENOUGH.into()),
            1 => {
                let p = self.sc2_points(&args[0], "P")?;
                match p.cols {
                    2 => self.sc2_rows2(&p),
                    3 if p.rows < 4 => {
                        return Err("At least four input points must be provided to define a tetrahedral mesh.".into());
                    }
                    3 => return Err(format!("delaunay of three-dimensional points: {NO_ND}")),
                    _ => return Err("The input must be a matrix with two or three columns.".into()),
                }
            }
            2 => {
                let x = self.sc2_points(&args[0], "X")?;
                let y = self.sc2_points(&args[1], "Y")?;
                if x.len() != y.len() {
                    return Err("Data point coordinates have inconsistent dimension.".into());
                }
                if x.len() > MAX_POINTS {
                    return Err(format!("delaunay supports up to {MAX_POINTS} points"));
                }
                x.data.iter().zip(&y.data).map(|(a, b)| [*a, *b]).collect()
            }
            3 => return Err(format!("delaunay of three-dimensional points: {NO_ND}")),
            _ => return Err("Too many input arguments.".into()),
        };
        self.charge(pts.len().saturating_mul(6))?;
        match self.sc2_triangulate(&pts)? {
            Some((_, rows)) => rows_value(&rows),
            None => Err(
                "Error computing the Delaunay triangulation. The points may be collinear.".into(),
            ),
        }
    }

    pub(super) fn sc2_delaunayn(&mut self, args: &[Value]) -> Result<Value> {
        if args.is_empty() {
            return Err(NOT_ENOUGH.into());
        }
        if args.len() > 2 {
            return Err("Too many input arguments.".into());
        }
        let x = self.sc2_points(&args[0], "X")?;
        self.charge(x.len().saturating_mul(3))?;
        match x.cols {
            1 => {
                if x.data.iter().any(|v| !v.is_finite()) {
                    return Err("Data points must be finite.".into());
                }
                let pts: Vec<P> = x.data.iter().map(|v| [*v, 0.]).collect();
                let (mut unique, _) = first_occurrences(&pts);
                if unique.len() < 2 {
                    return Err("Not enough unique points specified.".into());
                }
                unique.sort_by(|a, b| x.data[*a].total_cmp(&x.data[*b]));
                let m = unique.len() - 1;
                let mut data = vec![0.; 2 * m];
                for k in 0..m {
                    data[k] = (unique[k] + 1) as f64;
                    data[m + k] = (unique[k + 1] + 1) as f64;
                }
                sc2_num(m, 2, data)
            }
            2 => {
                let pts = self.sc2_rows2(&x);
                let (unique, _) = first_occurrences(&pts);
                if unique.len() < 3 {
                    return Err("Not enough unique points specified.".into());
                }
                match self.sc2_triangulate(&pts)? {
                    Some((_, rows)) => rows_value(&rows),
                    // Qhull gives three collinear points one flat simplex.
                    None if unique.len() == 3 => rows_value(&[[unique[0], unique[1], unique[2]]]),
                    None => Err(
                        "Error computing the Delaunay triangulation. The points may be collinear."
                            .into(),
                    ),
                }
            }
            0 => Err("Not enough unique points specified.".into()),
            _ => Err(format!(
                "delaunayn of points in three or more dimensions: {NO_ND}"
            )),
        }
    }

    // -- convhull / convhulln -----------------------------------------------

    pub(super) fn sc2_convhull(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        let numeric = args.iter().take_while(|a| sc2_text(a).is_none()).count();
        let mut simplify = false;
        let mut rest = &args[numeric..];
        while !rest.is_empty() {
            let name = sc2_text(&rest[0]).unwrap_or_default().to_ascii_lowercase();
            if name != "simplify" || rest.len() < 2 {
                return Err(
                    "Unrecognized option; convhull accepts 'simplify', true or false.".into(),
                );
            }
            simplify = sc2_scalar(&rest[1]).ok_or("'simplify' must be true or false.")? != 0.;
            rest = &rest[2..];
        }
        let pts: Vec<P> = match numeric {
            0 => return Err(NOT_ENOUGH.into()),
            1 => {
                let p = self.sc2_points(&args[0], "P")?;
                match p.cols {
                    2 => self.sc2_rows2(&p),
                    3 if p.rows < 4 => {
                        return Err(
                            "Error computing the convex hull. Not enough unique points specified."
                                .into(),
                        );
                    }
                    3 => return Err(format!("convhull of three-dimensional points: {NO_ND}")),
                    _ => return Err("The input must be a matrix with two or three columns.".into()),
                }
            }
            2 => {
                let x = self.sc2_points(&args[0], "X")?;
                let y = self.sc2_points(&args[1], "Y")?;
                if x.len() != y.len() || x.len() > MAX_POINTS {
                    return Err("Data point coordinates have inconsistent dimension.".into());
                }
                x.data.iter().zip(&y.data).map(|(a, b)| [*a, *b]).collect()
            }
            3 => return Err(format!("convhull of three-dimensional points: {NO_ND}")),
            _ => return Err("Too many input arguments.".into()),
        };
        self.charge(pts.len().saturating_mul(2))?;
        let (hull, area) = self.sc2_hull(&pts, !simplify)?;
        let mut ring = hull.clone();
        ring.push(hull[0]);
        let k = ring.iter().map(|i| (*i + 1) as f64).collect::<Vec<_>>();
        Ok(vec![sc2_num(k.len(), 1, k)?, sc2_num(1, 1, vec![area])?])
    }

    /// Counter-clockwise hull from the smallest hull index, and its area.
    fn sc2_hull(&mut self, pts: &[P], keep_collinear: bool) -> Result<(Vec<usize>, f64)> {
        if pts.iter().any(|p| !p[0].is_finite() || !p[1].is_finite()) {
            return Err("Data points must be finite.".into());
        }
        let (unique, _) = first_occurrences(pts);
        if unique.len() < 3 {
            return Err(
                "Error computing the convex hull. Not enough unique points specified.".into(),
            );
        }
        let strict = cross_hull(pts, &unique, false);
        if strict.len() < 3 {
            return Err("Error computing the convex hull. The points may be collinear.".into());
        }
        if keep_collinear {
            self.sc2_pairs(unique.len(), strict.len())?;
        }
        let mut hull = if keep_collinear {
            cross_hull(pts, &unique, true)
        } else {
            strict
        };
        let k = (0..hull.len()).min_by_key(|k| hull[*k]).expect("hull");
        hull.rotate_left(k);
        let area = polygon_area(pts, &hull);
        Ok((hull, area))
    }

    pub(super) fn sc2_convhulln(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        if args.is_empty() {
            return Err(NOT_ENOUGH.into());
        }
        if args.len() > 2 {
            return Err("Too many input arguments.".into());
        }
        let x = self.sc2_points(&args[0], "X")?;
        match x.cols {
            0 | 1 => return Err("X must have at least 2 columns.".into()),
            2 => {}
            _ => {
                return Err(format!(
                    "convhulln of points in three or more dimensions: {NO_ND}"
                ));
            }
        }
        let pts = self.sc2_rows2(&x);
        self.charge(pts.len().saturating_mul(2))?;
        let (hull, area) = self.sc2_hull(&pts, false)?;
        let m = hull.len();
        let mut data = vec![0.; 2 * m];
        for k in 0..m {
            data[k] = (hull[k] + 1) as f64;
            data[m + k] = (hull[(k + 1) % m] + 1) as f64;
        }
        Ok(vec![sc2_num(m, 2, data)?, sc2_num(1, 1, vec![area])?])
    }

    // -- dsearchn / tsearchn ----------------------------------------------

    pub(super) fn sc2_dsearchn(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        if args.len() < 2 {
            return Err("Needs at least 2 inputs.".into());
        }
        if args.len() > 4 {
            return Err("Too many input arguments.".into());
        }
        let x = self.sc2_points(&args[0], "X")?;
        let (tri, xi) = if args.len() == 2 {
            (None, self.sc2_points(&args[1], "XI")?)
        } else {
            let t = self.sc2_grid(&args[1], "TRI")?;
            ((t.len() > 0).then_some(t), self.sc2_points(&args[2], "XI")?)
        };
        if x.cols != xi.cols {
            return Err("X and XI should have the same column dimension.".into());
        }
        let outval = match args.get(3) {
            Some(v) => Some(sc2_scalar(v).ok_or("OUTVAL must be a scalar.")?),
            None => None,
        };
        self.sc2_pairs(x.rows, xi.rows)?;
        let (n, dims) = (xi.rows, x.cols);
        let mut k = Vec::with_capacity(n);
        let mut d = Vec::with_capacity(n);
        for q in 0..n {
            self.sc2_poll(q)?;
            let mut best = (f64::INFINITY, 0usize);
            for p in 0..x.rows {
                let mut s = 0.;
                for c in 0..dims {
                    let t = x.at(p, c) - xi.at(q, c);
                    s += t * t;
                }
                // The reference breaks ties toward the first point without a
                // triangulation and toward the last one with it.
                if s < best.0 || (tri.is_some() && s == best.0) {
                    best = (s, p);
                }
            }
            k.push((best.1 + 1) as f64);
            d.push(best.0.sqrt());
        }
        if let (Some(t), Some(outval)) = (&tri, outval) {
            let found = self.sc2_simplex_search(&x, t, &xi)?;
            for (q, (index, _)) in found.iter().enumerate() {
                if index.is_none() {
                    k[q] = outval;
                    d[q] = outval;
                }
            }
        }
        Ok(vec![sc2_num(n, 1, k)?, sc2_num(n, 1, d)?])
    }

    /// For every row of xi, the first simplex of `tri` containing it and its
    /// barycentric coordinates.
    #[allow(clippy::type_complexity)]
    fn sc2_simplex_search(
        &mut self,
        x: &Grid,
        tri: &Grid,
        xi: &Grid,
    ) -> Result<Vec<(Option<usize>, Vec<f64>)>> {
        let dims = x.cols;
        if tri.cols != dims + 1 {
            return Err(format!(
                "The triangulation must have {} columns for {}-dimensional points.",
                dims + 1,
                dims
            ));
        }
        for v in &tri.data {
            if !(v.fract() == 0. && *v >= 1. && *v <= x.rows as f64) {
                return Err("The triangulation indexes points that do not exist.".into());
            }
        }
        self.sc2_pairs(tri.rows, xi.rows)?;
        if xi.rows == 0 {
            return Ok(Vec::new());
        }
        let n = dims + 1;
        // One (n x n) system per simplex is kept and one O(n^3) solve is made
        // per simplex and query point: bound both before anything is built.
        let stored = tri.rows.saturating_mul(n.saturating_mul(n));
        let work = tri
            .rows
            .saturating_mul(xi.rows)
            .saturating_mul(n.saturating_mul(n).saturating_mul(n));
        if stored > MAX_ELEMENTS || work > MAX_PAIRS.saturating_mul(64) {
            return Err(format!(
                "The search is too large: {} simplices of {n} points and {} query points",
                tri.rows, xi.rows
            ));
        }
        let found_cells = Matrix::checked_len(xi.rows, n)?;
        // Stored systems, returned barycentric vectors, and one live solve's
        // matrix/RHS/solution scratch all count before allocating any system.
        let scratch = if tri.rows == 0 {
            0
        } else {
            n.saturating_mul(n)
        };
        self.charge(
            stored
                .saturating_add(found_cells)
                .saturating_add(scratch)
                .saturating_add(n.saturating_mul(3)),
        )?;
        // Barycentric coordinates b of q: sum b_k x_k = q, sum b_k = 1.
        let systems: Vec<Vec<f64>> = (0..tri.rows)
            .map(|t| {
                self.sc2_poll(t)?;
                let mut a = vec![0.; n * n];
                for (k, v) in (0..n).map(|k| (k, tri.at(t, k) as usize - 1)) {
                    for c in 0..dims {
                        a[c * n + k] = x.at(v, c);
                    }
                    a[dims * n + k] = 1.;
                }
                Ok(a)
            })
            .collect::<Result<_>>()?;
        let tol = 1e-12;
        let mut found = Vec::with_capacity(xi.rows);
        for q in 0..xi.rows {
            self.sc2_poll(q)?;
            let mut rhs: Vec<f64> = (0..dims).map(|c| xi.at(q, c)).collect();
            rhs.push(1.);
            let mut hit = (None, vec![f64::NAN; n]);
            for (t, a) in systems.iter().enumerate() {
                self.sc2_poll(t)?;
                if let Some(b) = solve_small(a.clone(), rhs.clone(), n)
                    && b.iter().all(|v| *v >= -tol)
                {
                    hit = (Some(t), b);
                    break;
                }
            }
            found.push(hit);
        }
        Ok(found)
    }

    pub(super) fn sc2_tsearchn(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        if args.len() < 3 {
            return Err(NOT_ENOUGH.into());
        }
        if args.len() > 3 {
            return Err("Too many input arguments.".into());
        }
        let x = self.sc2_points(&args[0], "X")?;
        let tri = self.sc2_grid(&args[1], "TRI")?;
        let xi = self.sc2_points(&args[2], "XI")?;
        if x.cols != xi.cols {
            return Err("X and XI must have the same number of columns.".into());
        }
        if x.cols == 0 {
            return Err("X must have at least one column.".into());
        }
        let n = xi.rows;
        let width = x.cols + 1;
        let cells = Matrix::checked_len(n, width)?;
        self.charge(cells)?;
        let found = self.sc2_simplex_search(&x, &tri, &xi)?;
        let mut idx = Vec::with_capacity(n);
        let mut bary = vec![f64::NAN; n * width];
        for (q, (t, b)) in found.into_iter().enumerate() {
            idx.push(t.map_or(f64::NAN, |t| (t + 1) as f64));
            if t.is_some() {
                for (k, v) in b.into_iter().enumerate() {
                    bary[k * n + q] = v;
                }
            }
        }
        Ok(vec![sc2_num(n, 1, idx)?, sc2_num(n, width, bary)?])
    }

    // -- voronoin -----------------------------------------------------------

    pub(super) fn sc2_voronoin(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        if args.is_empty() {
            return Err(NOT_ENOUGH.into());
        }
        if args.len() > 2 {
            return Err("Too many input arguments.".into());
        }
        let x = self.sc2_points(&args[0], "X")?;
        match x.cols {
            0 | 1 => return Err("X must have at least 2 columns.".into()),
            2 => {}
            _ => {
                return Err(format!(
                    "voronoin of points in three or more dimensions: {NO_ND}"
                ));
            }
        }
        let pts = self.sc2_rows2(&x);
        self.charge(pts.len().saturating_mul(8))?;
        let Some((_, rows)) = self.sc2_triangulate(&pts)? else {
            return Err("Input points are colinear or coplanar.".into());
        };
        let m = rows.len();
        let mut v = vec![0.; 2 * (m + 1)];
        v[0] = f64::INFINITY;
        v[m + 1] = f64::INFINITY;
        let centers: Vec<P> = rows
            .iter()
            .map(|r| circumcenter(pts[r[0]], pts[r[1]], pts[r[2]]))
            .collect();
        for (k, c) in centers.iter().enumerate() {
            v[k + 1] = c[0];
            v[m + 2 + k] = c[1];
        }
        // Triangles around every point.
        let mut around: Vec<Vec<usize>> = vec![Vec::new(); pts.len()];
        for (t, r) in rows.iter().enumerate() {
            for &p in r {
                around[p].push(t);
            }
        }
        let (_, owner) = first_occurrences(&pts);
        let mut cells = Vec::with_capacity(pts.len());
        for i in 0..pts.len() {
            self.sc2_poll(i)?;
            let fan = &around[owner[i]];
            cells.push(Arc::new(Array::Numeric(voronoi_cell(
                owner[i], fan, &rows,
            )?)));
        }
        let cell = CellArray::from_shared(pts.len(), 1, cells, &self.cancel)?;
        Ok(vec![
            sc2_num(m + 1, 2, v)?,
            Value::Container(Array::Cell(cell)),
        ])
    }

    // -- griddata / griddatan -------------------------------------------------

    pub(super) fn sc2_griddata(
        &mut self,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        let numeric = args.iter().take_while(|a| sc2_text(a).is_none()).count();
        if numeric < 5 {
            return Err(NOT_ENOUGH.into());
        }
        if numeric >= 7 {
            return Err(format!("griddata of three-dimensional points: {NO_ND}"));
        }
        if numeric == 6 || args.len() > 6 {
            return Err("Too many input arguments.".into());
        }
        let method = match args.get(5) {
            None => "linear".to_string(),
            Some(v) => sc2_text(v).unwrap_or_default().to_ascii_lowercase(),
        };
        if !matches!(
            method.as_str(),
            "linear" | "nearest" | "natural" | "cubic" | "v4"
        ) {
            return Err("Unknown method.".into());
        }
        let x = self.sc2_points(&args[0], "X")?;
        let y = self.sc2_points(&args[1], "Y")?;
        let v = self.sc2_points(&args[2], "V")?;
        if x.len() != y.len() || x.len() != v.len() {
            return Err("The lengths of X, Y and V must match.".into());
        }
        if x.len() > MAX_POINTS {
            return Err(format!("griddata supports up to {MAX_POINTS} points"));
        }
        if x.data.iter().chain(&y.data).any(|v| !v.is_finite()) {
            return Err("Data points must be finite.".into());
        }
        let mut xq = self.sc2_grid(&args[3], "XQ")?;
        let mut yq = self.sc2_grid(&args[4], "YQ")?;
        if (xq.rows, xq.cols) != (yq.rows, yq.cols) {
            if xq.rows == 1 && yq.cols == 1 {
                (yq, xq) = self.sc2_mesh(yq.len(), xq.len(), |i| yq.data[i], |i| xq.data[i], 3)?;
            } else {
                return Err("XQ and YQ must be the same size or a row and a column vector.".into());
            }
        } else {
            self.charge(xq.len().saturating_mul(3))?;
        }
        let pts: Vec<P> = x.data.iter().zip(&y.data).map(|(a, b)| [*a, *b]).collect();
        // Average the values of duplicate points.
        let (unique, owner) = first_occurrences(&pts);
        let mut sums: HashMap<usize, (f64, usize)> = HashMap::new();
        for (i, o) in owner.iter().enumerate() {
            let e = sums.entry(*o).or_insert((0., 0));
            e.0 += v.data[i];
            e.1 += 1;
        }
        if unique.len() < pts.len() {
            self.emit(
                out,
                "Warning: Merged duplicate data points and averaged their values.".into(),
            )?;
        }
        let upts: Vec<P> = unique.iter().map(|i| pts[*i]).collect();
        let uval: Vec<f64> = unique
            .iter()
            .map(|i| {
                let (s, n) = sums[i];
                if n == 1 { s } else { s / n as f64 }
            })
            .collect();
        let queries: Vec<P> = xq
            .data
            .iter()
            .zip(&yq.data)
            .map(|(a, b)| [*a, *b])
            .collect();
        let values = if method == "v4" {
            self.sc2_biharmonic(&upts, &uval, &queries)?
        } else {
            let order: Vec<usize> = (0..upts.len()).collect();
            let Some(d) = Delaunay::build(upts.clone(), &order, self)? else {
                return Err(
                    "Error computing the Delaunay triangulation. The points may be collinear."
                        .into(),
                );
            };
            if method == "nearest" {
                self.sc2_pairs(upts.len(), queries.len())?;
            }
            let gradients = if method == "cubic" {
                self.charge(upts.len().saturating_mul(16))?;
                self.sc2_min_norm_gradients(&upts, &uval, &d.triangles())?
            } else {
                Vec::new()
            };
            let mut values = Vec::with_capacity(queries.len());
            for (k, q) in queries.iter().enumerate() {
                self.sc2_poll(k)?;
                values.push(match method.as_str() {
                    "linear" => linear_at(&d, &uval, *q),
                    "nearest" => nearest_at(&d, &upts, &uval, *q),
                    "cubic" => cubic_at(&d, &uval, &gradients, *q),
                    _ => natural_at(&d, &uval, *q),
                });
            }
            values
        };
        let shape = (xq.rows, xq.cols);
        let vq = sc2_num(shape.0, shape.1, values)?;
        Ok(match count {
            0 | 1 => vec![vq],
            _ => vec![xq.value()?, yq.value()?, vq],
        })
    }

    /// Gradient estimates of the minimum norm network (Nielson 1983, Renka
    /// and Cline 1984): the gradients that minimise, over all edges of the
    /// triangulation, the integral of the squared second derivative of the
    /// cubic Hermite curve the edge's end values and slopes define. The
    /// normal equations are symmetric positive definite; they are solved by
    /// conjugate gradients with the 2 x 2 diagonal blocks as preconditioner.
    fn sc2_min_norm_gradients(
        &mut self,
        pts: &[P],
        v: &[f64],
        tris: &[[usize; 3]],
    ) -> Result<Vec<[f64; 2]>> {
        let n = pts.len();
        if v.iter().any(|x| !x.is_finite()) {
            return Ok(vec![[f64::NAN; 2]; n]);
        }
        let mut edges: Vec<(usize, usize)> = tris
            .iter()
            .flat_map(|t| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])])
            .map(|(a, b)| (a.min(b), a.max(b)))
            .collect();
        edges.sort_unstable();
        edges.dedup();
        // Unit direction and 1 / length of every edge; the diagonal blocks and
        // the right-hand side (the system divided by 4).
        let mut dir = Vec::with_capacity(edges.len());
        let mut diag = vec![[0.; 3]; n];
        let mut rhs = vec![[0.; 2]; n];
        for &(i, j) in &edges {
            let e = [pts[j][0] - pts[i][0], pts[j][1] - pts[i][1]];
            let l = libm::hypot(e[0], e[1]);
            let (r, w) = ([e[0] / l, e[1] / l], 1. / l);
            let c = 3. * (v[j] - v[i]) * w * w;
            for k in [i, j] {
                diag[k][0] += 2. * w * r[0] * r[0];
                diag[k][1] += 2. * w * r[0] * r[1];
                diag[k][2] += 2. * w * r[1] * r[1];
                rhs[k][0] += c * r[0];
                rhs[k][1] += c * r[1];
            }
            dir.push((r, w));
        }
        let precondition = |r: &[[f64; 2]]| -> Vec<[f64; 2]> {
            r.iter()
                .zip(&diag)
                .map(|(r, d)| {
                    let det = d[0] * d[2] - d[1] * d[1];
                    if det > 0. {
                        [
                            (d[2] * r[0] - d[1] * r[1]) / det,
                            (d[0] * r[1] - d[1] * r[0]) / det,
                        ]
                    } else {
                        *r
                    }
                })
                .collect()
        };
        let apply = |x: &[[f64; 2]]| -> Vec<[f64; 2]> {
            let mut y = vec![[0.; 2]; n];
            for (&(i, j), &(r, w)) in edges.iter().zip(&dir) {
                let si = r[0] * x[i][0] + r[1] * x[i][1];
                let sj = r[0] * x[j][0] + r[1] * x[j][1];
                let (a, c) = ((2. * si + sj) * w, (si + 2. * sj) * w);
                y[i][0] += a * r[0];
                y[i][1] += a * r[1];
                y[j][0] += c * r[0];
                y[j][1] += c * r[1];
            }
            y
        };
        let dot = |a: &[[f64; 2]], b: &[[f64; 2]]| -> f64 {
            a.iter()
                .zip(b)
                .map(|(a, b)| a[0] * b[0] + a[1] * b[1])
                .sum()
        };
        let mut x = vec![[0.; 2]; n];
        let mut r = rhs;
        let stop = 1e-12 * dot(&r, &r).sqrt();
        let mut z = precondition(&r);
        let mut p = z.clone();
        let mut rz = dot(&r, &z);
        for it in 0..4000 {
            if it % 64 == 0 {
                self.check_cancel()?;
            }
            if dot(&r, &r).sqrt() <= stop {
                break;
            }
            let ap = apply(&p);
            let pap = dot(&p, &ap);
            if pap.partial_cmp(&0.) != Some(std::cmp::Ordering::Greater) {
                break;
            }
            let alpha = rz / pap;
            for k in 0..n {
                x[k][0] += alpha * p[k][0];
                x[k][1] += alpha * p[k][1];
                r[k][0] -= alpha * ap[k][0];
                r[k][1] -= alpha * ap[k][1];
            }
            z = precondition(&r);
            let next = dot(&r, &z);
            let beta = next / rz;
            rz = next;
            for k in 0..n {
                p[k][0] = z[k][0] + beta * p[k][0];
                p[k][1] = z[k][1] + beta * p[k][1];
            }
        }
        Ok(x)
    }

    /// griddata 'v4': Sandwell's biharmonic spline.
    fn sc2_biharmonic(&mut self, pts: &[P], v: &[f64], queries: &[P]) -> Result<Vec<f64>> {
        let n = pts.len();
        if n.saturating_mul(n) > 16 * 1024 * 1024 {
            return Err("griddata 'v4' supports up to 4096 points".into());
        }
        self.sc2_pairs(n, queries.len().max(n))?;
        let cells = Matrix::checked_len(n, n)?;
        // Green matrix + LU copy, RHS/solution and pivot/solve scratch.
        self.charge(
            cells
                .saturating_mul(2)
                .saturating_add(n.saturating_mul(4))
                .saturating_add(queries.len()),
        )?;
        self.check_cancel()?;
        let green = |d: f64| -> f64 {
            if d == 0. {
                0.
            } else {
                d * d * (libm::log(d) - 1.)
            }
        };
        let mut g = vec![0.; n * n];
        for j in 0..n {
            self.sc2_poll(j)?;
            for i in 0..n {
                self.sc2_poll(i)?;
                let d = libm::hypot(pts[i][0] - pts[j][0], pts[i][1] - pts[j][1]);
                g[j * n + i] = green(d);
            }
        }
        let a = Matrix::from_column_vec(n, n, g)?;
        let b = Matrix::from_column_vec(n, 1, v.to_vec())?;
        self.check_cancel()?;
        let w = a.solve(&b)?;
        self.check_cancel()?;
        let w = w.data();
        let mut out = Vec::with_capacity(queries.len());
        for (k, q) in queries.iter().enumerate() {
            self.sc2_poll(k)?;
            let mut s = 0.;
            for i in 0..n {
                self.sc2_poll(i)?;
                let d = libm::hypot(q[0] - pts[i][0], q[1] - pts[i][1]);
                s += green(d) * w[i];
            }
            out.push(s);
        }
        Ok(out)
    }

    pub(super) fn sc2_griddatan(&mut self, args: &[Value]) -> Result<Value> {
        if args.len() < 3 {
            return Err(NOT_ENOUGH.into());
        }
        if args.len() > 5 {
            return Err("Too many input arguments.".into());
        }
        let x = self.sc2_points(&args[0], "X")?;
        let y = self.sc2_points(&args[1], "Y")?;
        let xi = self.sc2_points(&args[2], "XI")?;
        if x.cols < 2 {
            return Err("X must have at least 2 columns.".into());
        }
        if y.len() != x.rows {
            return Err("Y must have the same number of points as X.".into());
        }
        let method = match args.get(3) {
            None => "linear".to_string(),
            Some(v) if super::sc2_is_empty(v) => "linear".to_string(),
            Some(v) => sc2_text(v).unwrap_or_default().to_ascii_lowercase(),
        };
        if !matches!(method.as_str(), "linear" | "nearest") {
            return Err("METHOD must be either 'linear' or 'nearest'.".into());
        }
        if x.cols > 2 {
            return Err(format!(
                "griddatan of points in three or more dimensions: {NO_ND}"
            ));
        }
        if xi.cols != 2 {
            return Err("XI must have the same number of columns as X.".into());
        }
        let pts = self.sc2_rows2(&x);
        self.charge(pts.len().saturating_mul(6))?;
        let Some((_, rows)) = self.sc2_triangulate(&pts)? else {
            return Err(
                "Error computing the Delaunay triangulation. The points may be collinear.".into(),
            );
        };
        let m = rows.len();
        let mut t = vec![0.; 3 * m];
        for (i, r) in rows.iter().enumerate() {
            for k in 0..3 {
                t[k * m + i] = (r[k] + 1) as f64;
            }
        }
        let tri = Grid {
            rows: m,
            cols: 3,
            data: t,
        };
        let n = xi.rows;
        let mut out = Vec::with_capacity(n);
        if method == "nearest" {
            self.sc2_pairs(x.rows, n)?;
            for q in 0..n {
                self.sc2_poll(q)?;
                let mut best = (f64::INFINITY, 0usize);
                for p in 0..x.rows {
                    let a = x.at(p, 0) - xi.at(q, 0);
                    let b = x.at(p, 1) - xi.at(q, 1);
                    let s = a * a + b * b;
                    if s <= best.0 {
                        best = (s, p);
                    }
                }
                out.push(y.data[best.1]);
            }
        } else {
            let found = self.sc2_simplex_search(&x, &tri, &xi)?;
            for (t, b) in found {
                out.push(match t {
                    Some(t) => (0..3)
                        .map(|k| b[k] * y.data[tri.at(t, k) as usize - 1])
                        .sum(),
                    None => f64::NAN,
                });
            }
        }
        sc2_num(n, 1, out)
    }
}

/// The Voronoi cell of point `site` as indices into V (1 is the point at
/// infinity): bounded cells start at their largest index and run clockwise;
/// unbounded cells put infinity between their two ends. `fan` holds the
/// triangles around the site: the cell is bounded exactly when they close up
/// into a ring (every triangle's far neighbour starts another triangle).
fn voronoi_cell(site: usize, fan: &[usize], rows: &[[usize; 3]]) -> Result<Matrix> {
    // (first, second neighbour counter-clockwise, vertex index in V).
    let edges: Vec<(usize, usize, usize)> = fan
        .iter()
        .map(|t| {
            let r = rows[*t];
            let k = r.iter().position(|v| *v == site).expect("fan vertex");
            (r[(k + 1) % 3], r[(k + 2) % 3], t + 2)
        })
        .collect();
    let k = edges.len();
    if k == 0 {
        return Matrix::from_column_vec(1, 0, Vec::new());
    }
    let by_first: HashMap<usize, usize> = edges.iter().enumerate().map(|(j, e)| (e.0, j)).collect();
    let seconds: std::collections::HashSet<usize> = edges.iter().map(|e| e.1).collect();
    let open = edges.iter().position(|e| !seconds.contains(&e.0));
    // Walk counter-clockwise from the triangle with no predecessor (or any).
    let first = open.unwrap_or(0);
    let mut ccw = Vec::with_capacity(k);
    let mut cur = first;
    loop {
        ccw.push(edges[cur].2);
        match by_first.get(&edges[cur].1) {
            Some(&next) if next != first && ccw.len() < k => cur = next,
            _ => break,
        }
    }
    // Clockwise.
    let ids: Vec<usize> = ccw.into_iter().rev().collect();
    let cell: Vec<usize> = if open.is_none() {
        let start = (0..k).max_by_key(|j| ids[*j]).expect("cell");
        (0..k).map(|j| ids[(start + j) % k]).collect()
    } else {
        let mut ring = vec![1];
        ring.extend(ids);
        let len = ring.len();
        let top = (0..len).max_by_key(|j| ring[*j]).expect("cell");
        let next = ring[(top + 1) % len];
        let prev = ring[(top + len - 1) % len];
        if next == 1 || prev != 1 {
            (0..len).map(|j| ring[(top + j) % len]).collect()
        } else {
            (0..len).map(|j| ring[(top + len - j) % len]).collect()
        }
    };
    Matrix::from_column_vec(1, cell.len(), cell.iter().map(|v| *v as f64).collect())
}

fn barycentric(d: &Delaunay, t: usize, q: P) -> [f64; 3] {
    let [a, b, c] = d.tris[t].v;
    let (pa, pb, pc) = (d.pts[a], d.pts[b], d.pts[c]);
    let area = orient(pa, pb, pc);
    let la = orient(pb, pc, q) / area;
    let lb = orient(pc, pa, q) / area;
    [la, lb, 1. - la - lb]
}

fn linear_at(d: &Delaunay, v: &[f64], q: P) -> f64 {
    if !q[0].is_finite() || !q[1].is_finite() {
        return f64::NAN;
    }
    let t = d.locate(q);
    if d.is_ghost(t) {
        return f64::NAN;
    }
    let l = barycentric(d, t, q);
    let [a, b, c] = d.tris[t].v;
    l[0] * v[a] + l[1] * v[b] + l[2] * v[c]
}

fn nearest_at(d: &Delaunay, pts: &[P], v: &[f64], q: P) -> f64 {
    if !q[0].is_finite() || !q[1].is_finite() || d.is_ghost(d.locate(q)) {
        return f64::NAN;
    }
    let mut best = (f64::INFINITY, 0);
    for (i, p) in pts.iter().enumerate() {
        let s = (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2);
        if s <= best.0 {
            best = (s, i);
        }
    }
    v[best.1]
}

/// griddata 'cubic': the Clough-Tocher interpolant (C1, cubic on each third
/// of a triangle) of the data and the minimum norm network gradients.
fn cubic_at(d: &Delaunay, v: &[f64], grad: &[[f64; 2]], q: P) -> f64 {
    if !q[0].is_finite() || !q[1].is_finite() {
        return f64::NAN;
    }
    let t = d.locate(q);
    if d.is_ghost(t) {
        return f64::NAN;
    }
    let tv = d.tris[t].v;
    clough_tocher(
        tv.map(|i| d.pts[i]),
        tv.map(|i| v[i]),
        tv.map(|i| grad[i]),
        q,
    )
}

/// Clough-Tocher macro element (Farin's Bezier form) on the counter-clockwise
/// triangle `p` split at its centroid: Bernstein-Bezier coefficients of three
/// cubics from the vertex values `f` and gradients `g`. The derivative across
/// each outer edge is linear along it, so the surface is C1 over a
/// triangulation.
fn clough_tocher(p: [P; 3], f: [f64; 3], g: [[f64; 2]; 3], q: P) -> f64 {
    let centre = [
        (p[0][0] + p[1][0] + p[2][0]) / 3.,
        (p[0][1] + p[1][1] + p[2][1]) / 3.,
    ];
    let along = |a: usize, b: usize| g[a][0] * (p[b][0] - p[a][0]) + g[a][1] * (p[b][1] - p[a][1]);
    // Outer edge coefficients: e[a][b] sits one third from a toward b.
    let mut e = [[0.; 3]; 3];
    for (a, row) in e.iter_mut().enumerate() {
        for (b, coefficient) in row.iter_mut().enumerate() {
            if a != b {
                *coefficient = f[a] + along(a, b) / 3.;
            }
        }
    }
    // Next to each vertex, toward the centre: the tangent plane there.
    let near: [f64; 3] =
        std::array::from_fn(|a| (f[a] + e[a][(a + 1) % 3] + e[a][(a + 2) % 3]) / 3.);
    // Middle of the row next to edge (a, a+1): the derivative across the edge
    // (along its normal n, so that neighbouring triangles agree: C1) is linear
    // between the vertex values g[a].n and g[b].n. With n = a2 (pb - pa) +
    // a3 (centre - pa) the middle coefficient follows from the Bezier form of
    // that derivative.
    let middle: [f64; 3] = std::array::from_fn(|a| {
        let b = (a + 1) % 3;
        let edge = [p[b][0] - p[a][0], p[b][1] - p[a][1]];
        let inward = [centre[0] - p[a][0], centre[1] - p[a][1]];
        let n = [-edge[1], edge[0]];
        let across = inward[0] * n[0] + inward[1] * n[1];
        let a3 = (edge[0] * edge[0] + edge[1] * edge[1]) / across;
        let a2 = -(edge[0] * inward[0] + edge[1] * inward[1]) / across;
        let a1 = -a2 - a3;
        let d11 = ((g[a][0] + g[b][0]) * n[0] + (g[a][1] + g[b][1]) * n[1]) / 2.;
        (d11 / 3. - a1 * e[a][b] - a2 * e[b][a]) / a3
    });
    // On the inner edge from vertex a to the centre (C1 across it).
    let inner: [f64; 3] = std::array::from_fn(|a| (middle[a] + middle[(a + 2) % 3] + near[a]) / 3.);
    let hub = (inner[0] + inner[1] + inner[2]) / 3.;
    // The third (a, a+1, centre) holding q: the one opposite the vertex q is
    // furthest from in barycentric terms.
    let area = orient(p[0], p[1], p[2]);
    let bary = [
        orient(p[1], p[2], q) / area,
        orient(p[2], p[0], q) / area,
        orient(p[0], p[1], q) / area,
    ];
    let k = (0..3)
        .min_by(|x, y| bary[*x].total_cmp(&bary[*y]))
        .expect("three");
    let (a, b) = ((k + 1) % 3, (k + 2) % 3);
    let span = orient(p[a], p[b], centre);
    let u = orient(q, p[b], centre) / span;
    let w = orient(p[a], q, centre) / span;
    let h = 1. - u - w;
    // Bernstein-Bezier form in (u, w, h), the weights of p[a], p[b], centre.
    f[a] * u * u * u
        + f[b] * w * w * w
        + hub * h * h * h
        + 3. * e[a][b] * u * u * w
        + 3. * e[b][a] * u * w * w
        + 3. * near[a] * u * u * h
        + 3. * near[b] * w * w * h
        + 3. * inner[a] * u * h * h
        + 3. * inner[b] * w * h * h
        + 6. * middle[a] * u * w * h
}

/// Sibson's natural neighbour interpolation: each neighbour's weight is the
/// area its Voronoi cell would lose to q, over the triangles whose
/// circumcircle contains q.
fn natural_at(d: &Delaunay, v: &[f64], q: P) -> f64 {
    if !q[0].is_finite() || !q[1].is_finite() {
        return f64::NAN;
    }
    let t = d.locate(q);
    if d.is_ghost(t) {
        return f64::NAN;
    }
    let tv = d.tris[t].v;
    for &i in &tv {
        if d.pts[i] == q {
            return v[i];
        }
    }
    // q on the hull boundary: linear along the edge.
    for k in 0..3 {
        let (a, b) = (tv[(k + 1) % 3], tv[(k + 2) % 3]);
        if orient(d.pts[a], d.pts[b], q) == 0. && d.is_ghost(d.tris[t].n[k]) {
            return linear_at(d, v, q);
        }
    }
    let mut cavity = vec![t];
    let mut seen = std::collections::HashSet::new();
    seen.insert(t);
    let mut k = 0;
    while k < cavity.len() {
        let s = cavity[k];
        k += 1;
        for e in 0..3 {
            let u = d.tris[s].n[e];
            if seen.insert(u) && !d.is_ghost(u) {
                let [a, b, c] = d.tris[u].v;
                if incircle(d.pts[a], d.pts[b], d.pts[c], q) > 0. {
                    cavity.push(u);
                }
            }
        }
    }
    let mut weights: HashMap<usize, f64> = HashMap::new();
    let mut magnitude = 0.;
    for &s in &cavity {
        let [a, b, c] = d.tris[s].v;
        let (pa, pb, pc) = (d.pts[a], d.pts[b], d.pts[c]);
        let g = circumcenter(pa, pb, pc);
        let uab = circumcenter(q, pa, pb);
        let ubc = circumcenter(q, pb, pc);
        let uca = circumcenter(q, pc, pa);
        for (i, u, w) in [(a, uca, uab), (b, uab, ubc), (c, ubc, uca)] {
            // These are numerical area contributions, not topology predicates.
            // Keep their arithmetic order for well-conditioned interpolation.
            let positive = (g[0] - u[0]) * (w[1] - u[1]);
            let negative = (g[1] - u[1]) * (w[0] - u[0]);
            *weights.entry(i).or_insert(0.) += positive - negative;
            magnitude += positive.abs() + negative.abs();
        }
    }
    let weight_scale: f64 = weights.values().map(|w| w.abs()).sum();
    if !magnitude.is_finite()
        || !weight_scale.is_finite()
        || 64. * f64::EPSILON * magnitude > 1e-12 * weight_scale
    {
        // Near interior edges, cancelling circumcentres can be enormous but
        // finite. Use the boundary polygons if the area arithmetic loses its
        // precision, as well as when a centre lies at infinity on an edge.
        weights = stolen_areas(d, &cavity, q);
    }
    let mut keys: Vec<usize> = weights.keys().copied().collect();
    keys.sort_unstable();
    let total: f64 = keys.iter().map(|i| weights[i]).sum();
    keys.iter().map(|i| weights[i] / total * v[*i]).sum()
}

/// Sibson weights as explicit polygons: for every vertex of the cavity's
/// boundary, the circumcentres of q with its two boundary edges and the old
/// circumcentres of the cavity triangles around it.
fn stolen_areas(d: &Delaunay, cavity: &[usize], q: P) -> HashMap<usize, f64> {
    let inside: std::collections::HashSet<usize> = cavity.iter().copied().collect();
    // Boundary edges a -> b (counter-clockwise) and their cavity triangle.
    let mut edge_tri: HashMap<(usize, usize), usize> = HashMap::new();
    let mut next: HashMap<usize, usize> = HashMap::new();
    for &s in cavity {
        let v = d.tris[s].v;
        for e in 0..3 {
            if !inside.contains(&d.tris[s].n[e]) {
                let (a, b) = (v[(e + 1) % 3], v[(e + 2) % 3]);
                edge_tri.insert((a, b), s);
                next.insert(a, b);
            }
        }
    }
    let mut weights = HashMap::new();
    for (&vi, &after) in &next {
        let mut poly = vec![circumcenter(q, d.pts[vi], d.pts[after])];
        let mut t = edge_tri[&(vi, after)];
        let mut guard = 0;
        loop {
            let v = d.tris[t].v;
            poly.push(circumcenter(d.pts[v[0]], d.pts[v[1]], d.pts[v[2]]));
            // With vi first, the triangle is (vi, a, b); cross the edge b-vi.
            let k = v.iter().position(|x| *x == vi).expect("fan vertex");
            let (a, b) = (v[(k + 1) % 3], v[(k + 2) % 3]);
            let across = d.tris[t].n[v.iter().position(|x| *x == a).expect("edge")];
            guard += 1;
            if !inside.contains(&across) || guard > cavity.len() {
                poly.push(circumcenter(q, d.pts[b], d.pts[vi]));
                break;
            }
            t = across;
        }
        let mut area = 0.;
        for k in 0..poly.len() {
            let (p1, p2) = (poly[k], poly[(k + 1) % poly.len()]);
            area += p1[0] * p2[1] - p2[0] * p1[1];
        }
        weights.insert(vi, area);
    }
    weights
}

#[cfg(test)]
mod fix3_tests {
    use super::*;

    #[test]
    fn sc2_v4_reserves_green_and_lu_before_allocating() {
        // The reviewer's 4096-site case cannot fit both dense matrices,
        // even with an otherwise empty evaluation budget.
        let pts: Vec<P> = (0..4096).map(|i| [i as f64, (i % 7) as f64]).collect();
        let values = vec![1.; pts.len()];
        let error = Engine::default()
            .sc2_biharmonic(&pts, &values, &[[0., 0.]])
            .unwrap_err();
        assert!(error.contains("evaluation budget"), "{error}");
        // Three sites need 2*9 matrix cells, 4*3 scratch, one result,
        // and 3+3 pair-accounting cells: 37 in total.
        let mut engine = Engine::default();
        engine.charge(32 * 1024 * 1024 - 36).unwrap();
        let error = engine
            .sc2_biharmonic(
                &[[0., 0.], [1., 0.], [0., 1.]],
                &[1., 2., 3.],
                &[[0.2, 0.2]],
            )
            .unwrap_err();
        assert!(error.contains("evaluation budget"), "{error}");
    }
}
