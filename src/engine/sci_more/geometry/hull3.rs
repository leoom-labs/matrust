//! Convex hull of three-dimensional points (convhull, convhulln).
//!
//! Quickhull on exact orientation predicates gives a closed, outward
//! oriented triangulation without zero-area triangles for any input. Facets
//! that lie in one plane within a scale-aware tolerance are then merged and
//! triangulated again on their corners only, so points on faces and edges
//! are not hull vertices. Where a merged facet is not a plain polygon the
//! exact triangles are kept.
use crate::Result;
use std::collections::HashMap;

pub(super) type P3 = [f64; 3];

const NONE: u32 = u32::MAX;
/// Distances up to `TOLERANCE` times the largest local coordinate magnitude count
/// as zero (512 machine epsilons, about 1.1e-13).
const TOLERANCE: f64 = 512. * f64::EPSILON;
/// Scaled coordinates below 2^-256 are zero, so no predicate underflows.
const TINY: f64 = 8.636168555094445e-78;
const TOO_FEW: &str = "Error computing the convex hull. Not enough unique points specified.";
const FLAT: &str = "Error computing the convex hull. The points may be coplanar or collinear.";
const BROKEN: &str = "Error computing the convex hull.";

pub(super) struct Hull3 {
    /// Outward triangles (counter-clockwise seen from outside), each from
    /// its smallest index, in ascending row order.
    pub(super) triangles: Vec<[u32; 3]>,
    pub(super) volume: f64,
}

fn sub(a: &P3, b: &P3) -> P3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: &P3, b: &P3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: &P3, b: &P3) -> P3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn coord(p: &P3) -> robust::Coord3D<f64> {
    robust::Coord3D {
        x: p[0],
        y: p[1],
        z: p[2],
    }
}

/// Positive when d is outside the plane of a, b, c, which run
/// counter-clockwise seen from outside; the sign is exact and the value is
/// six times the volume of the tetrahedron.
fn above(a: &P3, b: &P3, c: &P3, d: &P3) -> f64 {
    -robust::orient3d(coord(a), coord(b), coord(c), coord(d))
}

/// 2^k for k within the exponent range of normal numbers.
fn pow2(k: i32) -> f64 {
    f64::from_bits(((1023 + k.clamp(-1022, 1023)) as u64) << 52)
}

/// x * 2^k, exact unless the result leaves the normal range.
fn ldexp(x: f64, k: i32) -> f64 {
    let h = k / 2;
    x * pow2(h) * pow2(k - h)
}

/// e with x = m * 2^e and 0.5 <= m < 1, for positive finite x.
fn exponent(x: f64) -> i32 {
    let biased = |v: f64| ((v.to_bits() >> 52) & 0x7ff) as i32;
    if biased(x) == 0 {
        biased(x * pow2(64)) - 1022 - 64
    } else {
        biased(x) - 1022
    }
}

#[derive(Clone, Copy)]
struct Face {
    v: [u32; 3],
    /// adj[i]: the face across the edge v[i] -> v[i + 1].
    adj: [u32; 3],
    /// The points strictly outside this face, linked through `next`.
    head: u32,
    far: u32,
    far_value: f64,
    mark: u32,
    visible: bool,
    alive: bool,
}

struct Mesh {
    tris: Vec<[u32; 3]>,
    adj: Vec<[u32; 3]>,
}

/// The hull of `points` (finite, at most u32::MAX of them). `poll` is called
/// regularly and stops the work with its error; `max_tests` bounds the
/// orientation tests.
pub(super) fn convex_hull(
    points: &[P3],
    max_tests: u64,
    poll: &mut dyn FnMut() -> Result<()>,
) -> Result<Hull3> {
    if points.iter().flatten().any(|v| !v.is_finite()) {
        return Err("Data points must be finite.".into());
    }
    if points.len() < 4 {
        return Err(TOO_FEW.into());
    }
    // Centre the bounding box before choosing the predicate scale. Its
    // midpoint stays finite even when the full coordinate span overflows.
    let mut lo = points[0];
    let mut hi = points[0];
    for q in points {
        for k in 0..3 {
            lo[k] = lo[k].min(q[k]);
            hi[k] = hi[k].max(q[k]);
        }
    }
    let origin = std::array::from_fn(|k| lo[k].midpoint(hi[k]));
    let largest = (0..3).fold(0f64, |m, k| m.max(origin[k] - lo[k]).max(hi[k] - origin[k]));
    if largest == 0. {
        return Err(TOO_FEW.into());
    }
    // A power of two brings the largest magnitude into [0.5, 1): results do
    // not change and the exact predicates cannot overflow or underflow.
    let shift = exponent(largest);
    let p: Vec<P3> = points
        .iter()
        .map(|q| {
            sub(q, &origin).map(|v| {
                let s = ldexp(v, -shift);
                if s.abs() < TINY { 0. } else { s }
            })
        })
        .collect();
    let tol = TOLERANCE * ldexp(largest, -shift);
    let Some(simplex) = first_simplex(&p, tol) else {
        return Err(if distinct_points(points) < 4 {
            TOO_FEW
        } else {
            FLAT
        }
        .into());
    };
    let centre = |p: &[P3], simplex: [u32; 4]| -> P3 {
        std::array::from_fn(|k| simplex.iter().map(|i| p[*i as usize][k]).sum::<f64>() / 4.)
    };
    let mut inside = centre(&p, simplex);
    let mut mesh = quickhull(&p, simplex, max_tests, poll)?;
    poll()?;
    // Hull vertices closer together than the tolerance are one corner: the
    // hull of one vertex of each such group has no needle triangles.
    let mut p = p;
    let mut rows: Option<Vec<u32>> = None;
    if let Some(kept) = weld(&p, &mesh, tol) {
        let apart: Vec<P3> = kept.iter().map(|i| p[*i as usize]).collect();
        if let Some(simplex) = first_simplex(&apart, tol) {
            mesh = quickhull(&apart, simplex, max_tests, poll)?;
            inside = centre(&apart, simplex);
            p = apart;
            rows = Some(kept);
            poll()?;
        }
    }
    let mut triangles = match simplify(&p, &mesh, tol) {
        Some(clean) => clean,
        None => mesh.tris,
    };
    let mut six = 0.;
    for t in &triangles {
        let [a, b, c] = t.map(|i| sub(&p[i as usize], &inside));
        six += dot(&a, &cross(&b, &c));
    }
    let volume = ldexp(ldexp(ldexp(six / 6., shift), shift), shift);
    for t in &mut triangles {
        if let Some(rows) = &rows {
            *t = t.map(|i| rows[i as usize]);
        }
        let k = (0..3).min_by_key(|k| t[*k]).expect("three");
        t.rotate_left(k);
    }
    triangles.sort_unstable();
    Ok(Hull3 { triangles, volume })
}

fn distinct_points(points: &[P3]) -> usize {
    let mut keys: Vec<[u64; 3]> = points
        .iter()
        .map(|q| q.map(|v| (v + 0.).to_bits()))
        .collect();
    keys.sort_unstable();
    keys.dedup();
    keys.len()
}

/// Four points that span space by more than the tolerance, with the fourth
/// inside the plane of the first three (those are counter-clockwise seen
/// from outside); `None` for coincident, collinear or coplanar points.
fn first_simplex(p: &[P3], tol: f64) -> Option<[u32; 4]> {
    let first_max = |value: &dyn Fn(&P3) -> f64| {
        let mut best = (0usize, f64::NEG_INFINITY);
        for (i, q) in p.iter().enumerate() {
            let v = value(q);
            if v > best.1 {
                best = (i, v);
            }
        }
        best
    };
    let i0 = (0..p.len()).min_by(|a, b| {
        p[*a][0]
            .total_cmp(&p[*b][0])
            .then(p[*a][1].total_cmp(&p[*b][1]))
            .then(p[*a][2].total_cmp(&p[*b][2]))
    })?;
    let a = p[i0];
    let (i1, d1) = first_max(&|q| {
        let d = sub(q, &a);
        dot(&d, &d)
    });
    if d1.sqrt() <= tol {
        return None;
    }
    let u = sub(&p[i1], &a);
    let (i2, d2) = first_max(&|q| {
        let c = cross(&u, &sub(q, &a));
        dot(&c, &c)
    });
    if (d2 / dot(&u, &u)).sqrt() <= tol {
        return None;
    }
    let normal = cross(&u, &sub(&p[i2], &a));
    let (i3, d3) = first_max(&|q| dot(&normal, &sub(q, &a)).abs());
    if d3 / dot(&normal, &normal).sqrt() <= tol {
        return None;
    }
    let side = above(&a, &p[i1], &p[i2], &p[i3]);
    if side == 0. {
        return None;
    }
    let (i0, i1, i2, i3) = (i0 as u32, i1 as u32, i2 as u32, i3 as u32);
    Some(if side < 0. {
        [i0, i1, i2, i3]
    } else {
        [i0, i2, i1, i3]
    })
}

struct Quickhull<'a> {
    p: &'a [P3],
    faces: Vec<Face>,
    free: Vec<u32>,
    /// The next point in the outside list a point belongs to.
    next: Vec<u32>,
    tests: u64,
}

impl Quickhull<'_> {
    fn add_face(&mut self, v: [u32; 3], adj: [u32; 3]) -> u32 {
        let face = Face {
            v,
            adj,
            head: NONE,
            far: NONE,
            far_value: 0.,
            mark: 0,
            visible: false,
            alive: true,
        };
        match self.free.pop() {
            Some(id) => {
                self.faces[id as usize] = face;
                id
            }
            None => {
                self.faces.push(face);
                (self.faces.len() - 1) as u32
            }
        }
    }

    fn height(&mut self, face: u32, q: u32) -> f64 {
        self.tests += 1;
        let [a, b, c] = self.faces[face as usize].v;
        above(
            &self.p[a as usize],
            &self.p[b as usize],
            &self.p[c as usize],
            &self.p[q as usize],
        )
    }

    /// Hands q to the first of `faces` it is strictly outside of.
    fn assign(&mut self, q: u32, faces: &[u32]) {
        for &id in faces {
            let h = self.height(id, q);
            if h > 0. {
                let f = &mut self.faces[id as usize];
                self.next[q as usize] = f.head;
                f.head = q;
                if f.far == NONE || h > f.far_value || (h == f.far_value && q < f.far) {
                    f.far = q;
                    f.far_value = h;
                }
                return;
            }
        }
    }
}

fn quickhull(
    p: &[P3],
    simplex: [u32; 4],
    max_tests: u64,
    poll: &mut dyn FnMut() -> Result<()>,
) -> Result<Mesh> {
    let mut hull = Quickhull {
        p,
        faces: Vec::new(),
        free: Vec::new(),
        next: vec![NONE; p.len()],
        tests: 0,
    };
    let [a, b, c, d] = simplex;
    hull.add_face([a, b, c], [1, 2, 3]);
    hull.add_face([b, a, d], [0, 3, 2]);
    hull.add_face([c, b, d], [0, 1, 3]);
    hull.add_face([a, c, d], [0, 2, 1]);
    let first = [0, 1, 2, 3];
    for q in 0..p.len() as u32 {
        if q.is_multiple_of(4096) {
            poll()?;
        }
        if !simplex.contains(&q) {
            hull.assign(q, &first);
        }
    }
    let mut pending: Vec<u32> = first.to_vec();
    let mut stamp = 0u32;
    let mut visible: Vec<u32> = Vec::new();
    let mut stack: Vec<u32> = Vec::new();
    let mut horizon: Vec<(u32, u32, u32)> = Vec::new();
    let mut fresh: Vec<u32> = Vec::new();
    // The new face that starts at a horizon vertex, valid for one stamp.
    let mut from: Vec<(u32, u32)> = vec![(0, NONE); p.len()];
    let too_long = || {
        format!("The convex hull needs more than {max_tests} orientation tests; use fewer points")
    };
    while let Some(start) = pending.pop() {
        let face = hull.faces[start as usize];
        if !face.alive || face.head == NONE {
            continue;
        }
        poll()?;
        if hull.tests > max_tests {
            return Err(too_long());
        }
        let eye = face.far;
        stamp += 1;
        visible.clear();
        horizon.clear();
        fresh.clear();
        hull.faces[start as usize].mark = stamp;
        hull.faces[start as usize].visible = true;
        stack.push(start);
        while let Some(g) = stack.pop() {
            visible.push(g);
            for i in 0..3 {
                let h = hull.faces[g as usize].adj[i];
                if hull.faces[h as usize].mark != stamp {
                    let seen = hull.height(h, eye) > 0.;
                    let other = &mut hull.faces[h as usize];
                    other.mark = stamp;
                    other.visible = seen;
                    if seen {
                        stack.push(h);
                    }
                }
                if !hull.faces[h as usize].visible {
                    let v = hull.faces[g as usize].v;
                    horizon.push((v[i], v[(i + 1) % 3], h));
                }
            }
        }
        // A cone from the eye to the horizon replaces the visible faces.
        for &(a, b, outer) in &horizon {
            let id = hull.add_face([a, b, eye], [outer, NONE, NONE]);
            let back = hull.faces[outer as usize].v;
            let j = (0..3)
                .find(|j| back[*j] == b && back[(*j + 1) % 3] == a)
                .ok_or(BROKEN)?;
            hull.faces[outer as usize].adj[j] = id;
            if from[a as usize].0 == stamp {
                return Err(BROKEN.into());
            }
            from[a as usize] = (stamp, id);
            fresh.push(id);
        }
        for &id in &fresh {
            let b = hull.faces[id as usize].v[1];
            let (seen, following) = from[b as usize];
            if seen != stamp {
                return Err(BROKEN.into());
            }
            hull.faces[id as usize].adj[1] = following;
            hull.faces[following as usize].adj[2] = id;
        }
        // The horizon is one closed chain: every new face has both sides.
        if fresh
            .iter()
            .any(|id| hull.faces[*id as usize].adj[2] == NONE)
        {
            return Err(BROKEN.into());
        }
        let mut moved = 0usize;
        for &g in &visible {
            let mut q = hull.faces[g as usize].head;
            while q != NONE {
                let following = hull.next[q as usize];
                if q != eye {
                    hull.assign(q, &fresh);
                }
                q = following;
                moved += 1;
                if moved.is_multiple_of(4096) {
                    poll()?;
                    if hull.tests > max_tests {
                        return Err(too_long());
                    }
                }
            }
            let dead = &mut hull.faces[g as usize];
            dead.alive = false;
            dead.head = NONE;
        }
        hull.free.extend_from_slice(&visible);
        pending.extend(
            fresh
                .iter()
                .filter(|id| hull.faces[**id as usize].head != NONE),
        );
    }
    let mut index = vec![NONE; hull.faces.len()];
    let mut tris = Vec::new();
    for (id, face) in hull.faces.iter().enumerate() {
        if face.alive {
            index[id] = tris.len() as u32;
            tris.push(face.v);
        }
    }
    let adj = hull
        .faces
        .iter()
        .filter(|f| f.alive)
        .map(|f| f.adj.map(|g| index[g as usize]))
        .collect();
    Ok(Mesh { tris, adj })
}

/// The vertices of the mesh, in ascending order, without those that are
/// within the tolerance of an earlier kept one; `None` when all are kept
/// (or the points are too crowded to tell in linear time).
fn weld(p: &[P3], mesh: &Mesh, tol: f64) -> Option<Vec<u32>> {
    let mut used: Vec<u32> = mesh.tris.iter().flatten().copied().collect();
    used.sort_unstable();
    used.dedup();
    // Cells much wider than the tolerance: a point's neighbourhood is in
    // one cell, except beside a cell wall.
    let cell = 1024. * tol;
    let key = |q: &P3, shift: f64| q.map(|v| ((v + shift) / cell).floor() as i64);
    let mut heads: HashMap<[i64; 3], u32> = HashMap::with_capacity(used.len());
    // The kept vertices of one cell are chained through `chain`.
    let mut chain: Vec<u32> = Vec::with_capacity(used.len());
    let mut kept: Vec<u32> = Vec::with_capacity(used.len());
    let mut budget = 64 * used.len() + 4096;
    for &i in &used {
        let q = &p[i as usize];
        let (low, high) = (key(q, -tol), key(q, tol));
        let mut near = false;
        'cells: for x in low[0]..=high[0] {
            for y in low[1]..=high[1] {
                for z in low[2]..=high[2] {
                    let mut at = heads.get(&[x, y, z]).copied().unwrap_or(NONE);
                    while at != NONE {
                        if budget == 0 {
                            return None;
                        }
                        budget -= 1;
                        let d = sub(q, &p[kept[at as usize] as usize]);
                        if dot(&d, &d) <= tol * tol {
                            near = true;
                            break 'cells;
                        }
                        at = chain[at as usize];
                    }
                }
            }
        }
        if !near {
            let slot = kept.len() as u32;
            chain.push(heads.insert(key(q, 0.), slot).unwrap_or(NONE));
            kept.push(i);
        }
    }
    (kept.len() < used.len()).then_some(kept)
}

/// The projection of a facet plane onto the coordinate plane it is most
/// parallel to; triangles facing the plane's way come out counter-clockwise.
#[derive(Clone, Copy)]
struct Plane {
    axis: usize,
    positive: bool,
}

impl Plane {
    fn of(normal: &P3) -> Self {
        let axis = (0..3)
            .max_by(|a, b| normal[*a].abs().total_cmp(&normal[*b].abs()))
            .expect("three");
        Plane {
            axis,
            positive: normal[axis] > 0.,
        }
    }

    fn flat(&self, q: &P3) -> super::P {
        let (u, w) = (q[(self.axis + 1) % 3], q[(self.axis + 2) % 3]);
        if self.positive { [u, w] } else { [w, u] }
    }

    /// Exact sign: positive for a counter-clockwise projected triangle.
    fn turn(&self, a: &P3, b: &P3, c: &P3) -> f64 {
        super::orient(self.flat(a), self.flat(b), self.flat(c))
    }

    fn area2(&self, a: &P3, b: &P3, c: &P3) -> f64 {
        let (a, b, c) = (self.flat(a), self.flat(b), self.flat(c));
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    }
}

/// Squared distance from q to the segment a-b.
fn segment_distance2(q: &P3, a: &P3, b: &P3) -> f64 {
    let (ab, aq) = (sub(b, a), sub(q, a));
    let len2 = dot(&ab, &ab);
    let t = if len2 > 0. {
        (dot(&aq, &ab) / len2).clamp(0., 1.)
    } else {
        0.
    };
    let d: P3 = std::array::from_fn(|k| aq[k] - t * ab[k]);
    dot(&d, &d)
}

/// Merges the triangles of each flat facet and triangulates the facet on its
/// corners. `None` when nothing merges or the result would not be a closed
/// surface; the exact mesh then stands.
#[allow(clippy::needless_range_loop, clippy::neg_cmp_op_on_partial_ord)]
fn simplify(p: &[P3], mesh: &Mesh, tol: f64) -> Option<Vec<[u32; 3]>> {
    let nf = mesh.tris.len();
    let pt = |i: u32| &p[i as usize];
    let normals: Vec<P3> = mesh
        .tris
        .iter()
        .map(|t| cross(&sub(pt(t[1]), pt(t[0])), &sub(pt(t[2]), pt(t[0]))))
        .collect();
    let sizes: Vec<f64> = normals.iter().map(|n| dot(n, n).sqrt()).collect();
    let mut order: Vec<u32> = (0..nf as u32).collect();
    order.sort_unstable_by(|a, b| {
        sizes[*b as usize]
            .total_cmp(&sizes[*a as usize])
            .then(a.cmp(b))
    });

    let longest = |f: u32| {
        let t = mesh.tris[f as usize];
        (0..3)
            .map(|i| {
                let e = sub(pt(t[(i + 1) % 3]), pt(t[i]));
                dot(&e, &e)
            })
            .fold(0f64, f64::max)
    };
    // A triangle whose height is within the tolerance: its corners are on
    // one line, as far as the tolerance can tell, and it faces no way.
    let thin = |f: u32| sizes[f as usize] <= 4. * tol * longest(f).sqrt();

    // Regions grow from the largest triangles: a neighbour joins when its
    // far corner is within the tolerance of the seed's plane and it faces
    // the same way. (Three corners within the tolerance of the plane leave
    // a triangle that is not thin less than 30 degrees of tilt, so its
    // projection shows which way it faces.) A neighbour whose far corner is
    // within the tolerance of the shared edge itself has no area to speak
    // of and joins as well.
    let tol2 = tol * tol;
    let mut region = vec![NONE; nf];
    let mut planes: Vec<Plane> = Vec::new();
    let mut queue: Vec<u32> = Vec::new();
    for &seed in &order {
        if region[seed as usize] != NONE {
            continue;
        }
        let r = planes.len() as u32;
        region[seed as usize] = r;
        let t = mesh.tris[seed as usize];
        let normal = normals[seed as usize];
        let size = sizes[seed as usize];
        let plane = Plane::of(&normal);
        planes.push(plane);
        // The plane of a sliver is not reliable; it stays alone.
        if !(size > 1e-8 * longest(seed)) {
            continue;
        }
        let unit = normal.map(|v| v / size);
        let origin = *pt(t[0]);
        queue.push(seed);
        while let Some(g) = queue.pop() {
            for i in 0..3 {
                let h = mesh.adj[g as usize][i];
                if region[h as usize] != NONE {
                    continue;
                }
                let tg = mesh.tris[g as usize];
                let th = mesh.tris[h as usize];
                let far = th.iter().find(|v| **v != tg[i] && **v != tg[(i + 1) % 3])?;
                let flat = dot(&unit, &sub(pt(*far), &origin)).abs() <= tol
                    && (plane.turn(pt(th[0]), pt(th[1]), pt(th[2])) > 0. || thin(h));
                if flat || segment_distance2(pt(*far), pt(tg[i]), pt(tg[(i + 1) % 3])) <= tol2 {
                    region[h as usize] = r;
                    queue.push(h);
                }
            }
        }
    }
    let nr = planes.len();
    if nr == nf {
        return None;
    }

    // Faces by region.
    let mut start = vec![0usize; nr + 1];
    for r in &region {
        start[*r as usize + 1] += 1;
    }
    for r in 0..nr {
        start[r + 1] += start[r];
    }
    let mut cursor = start.clone();
    let mut members = vec![0u32; nf];
    for f in 0..nf {
        let r = region[f] as usize;
        members[cursor[r]] = f as u32;
        cursor[r] += 1;
    }
    let faces_of = |r: usize| &members[start[r]..start[r + 1]];

    // A vertex where three or more regions meet is a corner.
    let mut kept = vec![false; p.len()];
    let mut seen = vec![[NONE; 2]; p.len()];
    for f in 0..nf {
        for v in mesh.tris[f] {
            let s = &mut seen[v as usize];
            let r = region[f];
            if s[0] == NONE {
                s[0] = r;
            } else if s[0] != r {
                if s[1] == NONE {
                    s[1] = r;
                } else if s[1] != r {
                    kept[v as usize] = true;
                }
            }
        }
    }
    drop(seen);

    // The boundary of each region as one simple cycle; anything else keeps
    // its triangles.
    let mut fallback = vec![false; nr];
    let mut cycles: Vec<u32> = Vec::new();
    let mut range = vec![(0usize, 0usize); nr];
    let mut out: Vec<(u32, u32)> = vec![(NONE, NONE); p.len()];
    for r in 0..nr {
        let faces = faces_of(r);
        if faces.len() == 1 {
            fallback[r] = true;
            continue;
        }
        let (mut edges, mut first, mut simple) = (0usize, NONE, true);
        for &f in faces {
            for i in 0..3 {
                if region[mesh.adj[f as usize][i] as usize] as usize != r {
                    let t = mesh.tris[f as usize];
                    let (a, b) = (t[i], t[(i + 1) % 3]);
                    simple &= out[a as usize].0 != r as u32;
                    out[a as usize] = (r as u32, b);
                    edges += 1;
                    if first == NONE {
                        first = a;
                    }
                }
            }
        }
        let begin = cycles.len();
        let mut at = first;
        while simple && edges >= 3 {
            cycles.push(at);
            let (owner, to) = out[at as usize];
            if owner != r as u32 || cycles.len() - begin > edges {
                simple = false;
            }
            at = to;
            if at == first {
                break;
            }
        }
        if !simple || edges < 3 || cycles.len() - begin != edges {
            cycles.truncate(begin);
            fallback[r] = true;
        } else {
            range[r] = (begin, edges);
        }
    }
    drop(out);
    let keep_all = |r: usize, kept: &mut [bool]| {
        for &f in faces_of(r) {
            for v in mesh.tris[f as usize] {
                kept[v as usize] = true;
            }
        }
    };
    for r in 0..nr {
        if fallback[r] {
            keep_all(r, &mut kept);
        }
    }

    // Between two corners the boundary follows the edge of the facet: a
    // vertex further than the tolerance from that chord is a corner too.
    let mut budget = 64 * (p.len() as i64 + nf as i64) + (1 << 20);
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for r in 0..nr {
        if fallback[r] {
            continue;
        }
        let cycle = &cycles[range[r].0..range[r].0 + range[r].1];
        let m = cycle.len();
        let Some(first) = (0..m).find(|i| kept[cycle[*i] as usize]) else {
            fallback[r] = true;
            keep_all(r, &mut kept);
            continue;
        };
        let mut i = first;
        loop {
            let mut len = 1;
            while !kept[cycle[(i + len) % m] as usize] {
                len += 1;
            }
            if len > 1 {
                spans.push((i, len));
            }
            i = (i + len) % m;
            if i == first {
                break;
            }
        }
        while let Some((s, len)) = spans.pop() {
            budget -= len as i64;
            if budget < 0 {
                return None;
            }
            let (a, b) = (pt(cycle[s % m]), pt(cycle[(s + len) % m]));
            let mut worst = (0., 0usize);
            for k in 1..len {
                let d = segment_distance2(pt(cycle[(s + k) % m]), a, b);
                if d > worst.0 {
                    worst = (d, k);
                }
            }
            if worst.0 > tol2 {
                kept[cycle[(s + worst.1) % m] as usize] = true;
                if worst.1 > 1 {
                    spans.push((s, worst.1));
                }
                if len - worst.1 > 1 {
                    spans.push((s + worst.1, len - worst.1));
                }
            }
        }
    }

    // Triangulate each facet polygon; a facet that fails keeps its triangles
    // and all its vertices, which changes its neighbours' polygons.
    let mut result: Vec<[u32; 3]> = Vec::new();
    let mut polygon: Vec<u32> = Vec::new();
    let mut settled = false;
    for _round in 0..4 {
        result.clear();
        let mut changed = false;
        for r in 0..nr {
            if fallback[r] {
                continue;
            }
            let cycle = &cycles[range[r].0..range[r].0 + range[r].1];
            polygon.clear();
            polygon.extend(cycle.iter().filter(|v| kept[**v as usize]));
            let plane = planes[r];
            let before: f64 = faces_of(r)
                .iter()
                .map(|f| {
                    let t = mesh.tris[*f as usize];
                    plane.area2(pt(t[0]), pt(t[1]), pt(t[2]))
                })
                .sum();
            let mark = result.len();
            budget -= polygon.len() as i64;
            if clip_ears(p, plane, &polygon, before, tol, &mut result, &mut budget) {
                improve(p, plane, &mut result[mark..]);
            } else {
                if budget < 0 {
                    return None;
                }
                result.truncate(mark);
                fallback[r] = true;
                keep_all(r, &mut kept);
                changed = true;
            }
        }
        if !changed {
            settled = true;
            break;
        }
    }
    if !settled {
        return None;
    }
    for r in 0..nr {
        if fallback[r] {
            result.extend(faces_of(r).iter().map(|f| mesh.tris[*f as usize]));
        }
    }
    // The surface moved by the tolerance at most, and so did the volume.
    let base = pt(mesh.tris[0][0]);
    let six = |tris: &[[u32; 3]]| -> f64 {
        tris.iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| sub(pt(i), base));
                dot(&a, &cross(&b, &c))
            })
            .sum()
    };
    let (exact, clean) = (six(&mesh.tris), six(&result));
    let area2: f64 = sizes.iter().sum();
    let same = (clean - exact).abs() <= 192. * tol * area2 + 1e-12 * exact.abs();
    (same && closed_surface(&result)).then_some(result)
}

/// Ear clipping of the polygon in the plane's projection with exact turns:
/// every triangle is counter-clockwise with a positive area, also beside
/// straight or slightly reflex corners. Every other corner is cut in turn,
/// which keeps the triangles of a regular polygon balanced. `false` when
/// the polygon is not simple, its area differs from `before` (twice the
/// area of the merged triangles) or the budget runs out.
#[allow(clippy::needless_range_loop)]
fn clip_ears(
    p: &[P3],
    plane: Plane,
    polygon: &[u32],
    before: f64,
    tol: f64,
    result: &mut Vec<[u32; 3]>,
    budget: &mut i64,
) -> bool {
    let n = polygon.len();
    if n < 3 {
        return false;
    }
    let pt = |i: usize| &p[polygon[i] as usize];
    let mut prev: Vec<usize> = (0..n).map(|i| (i + n - 1) % n).collect();
    let mut next: Vec<usize> = (0..n).map(|i| (i + 1) % n).collect();
    let mut gone = vec![false; n];
    let mut reflex: Vec<bool> = (0..n)
        .map(|i| plane.turn(pt(prev[i]), pt(i), pt(next[i])) <= 0.)
        .collect();
    let mut hollow: Vec<usize> = (0..n).filter(|i| reflex[*i]).collect();
    let mut perimeter = 0.;
    for i in 0..n {
        let (a, b) = (plane.flat(pt(i)), plane.flat(pt(next[i])));
        perimeter += ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
    }
    let mark = result.len();
    let mut after = 0.;
    let (mut left, mut at, mut idle) = (n, 0usize, 0usize);
    while left > 3 {
        if idle >= left {
            return false;
        }
        let (a, c) = (prev[at], next[at]);
        let mut ear = !reflex[at];
        if ear {
            hollow.retain(|h| !gone[*h] && reflex[*h]);
            *budget -= 3 * hollow.len() as i64 + 1;
            if *budget < 0 {
                return false;
            }
            ear = hollow.iter().all(|&h| {
                h == a
                    || h == c
                    || plane.turn(pt(a), pt(at), pt(h)) < 0.
                    || plane.turn(pt(at), pt(c), pt(h)) < 0.
                    || plane.turn(pt(c), pt(a), pt(h)) < 0.
            });
        }
        if !ear {
            at = c;
            idle += 1;
            continue;
        }
        result.push([polygon[a], polygon[at], polygon[c]]);
        after += plane.area2(pt(a), pt(at), pt(c));
        gone[at] = true;
        next[a] = c;
        prev[c] = a;
        left -= 1;
        for v in [a, c] {
            let turn = plane.turn(pt(prev[v]), pt(v), pt(next[v])) <= 0.;
            if turn && !reflex[v] {
                hollow.push(v);
            }
            reflex[v] = turn;
        }
        // Skip the next corner, so the ears alternate around the polygon.
        at = next[c];
        idle = 0;
    }
    let (a, c) = (prev[at], next[at]);
    if plane.turn(pt(a), pt(at), pt(c)) <= 0. {
        return false;
    }
    result.push([polygon[a], polygon[at], polygon[c]]);
    after += plane.area2(pt(a), pt(at), pt(c));
    debug_assert_eq!(result.len() - mark, n - 2);
    (after - before).abs() <= 1e-9 * before.abs().max(after) + 8. * tol * perimeter
}

/// Flips diagonals of one facet's triangles towards its Delaunay
/// triangulation, which has the largest smallest angle: a corner that is
/// almost straight then joins far corners instead of closing a sliver with
/// its two neighbours. A diagonal is flipped when the angles opposite to it
/// add up to clearly more than a half turn (co-circular corners, as on a
/// regular polygon, stay as they are) and the exact turns keep both new
/// triangles counter-clockwise.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
fn improve(p: &[P3], plane: Plane, tris: &mut [[u32; 3]]) {
    if tris.len() < 2 {
        return;
    }
    let pt = |i: u32| &p[i as usize];
    let mut owner: HashMap<(u32, u32), usize> = HashMap::with_capacity(3 * tris.len());
    for (t, tri) in tris.iter().enumerate() {
        for i in 0..3 {
            owner.insert((tri[i], tri[(i + 1) % 3]), t);
        }
    }
    let mut stack: Vec<(u32, u32)> = Vec::new();
    for tri in tris.iter() {
        for i in 0..3 {
            let (a, b) = (tri[i], tri[(i + 1) % 3]);
            if a < b && owner.contains_key(&(b, a)) {
                stack.push((a, b));
            }
        }
    }
    let mut budget = 16 * tris.len() + 64;
    while let Some((a, b)) = stack.pop() {
        if budget == 0 {
            return;
        }
        budget -= 1;
        let (Some(&t), Some(&s)) = (owner.get(&(a, b)), owner.get(&(b, a))) else {
            continue;
        };
        let other = |tri: [u32; 3]| tri.into_iter().find(|v| *v != a && *v != b);
        let (Some(c), Some(d)) = (other(tris[t]), other(tris[s])) else {
            continue;
        };
        // sin(alpha + beta) of the angles at c and d, times the four lengths.
        let (u, v) = (sub(pt(a), pt(c)), sub(pt(b), pt(c)));
        let (w, z) = (sub(pt(a), pt(d)), sub(pt(b), pt(d)));
        let (uv, wz) = (cross(&u, &v), cross(&w, &z));
        let sine = dot(&uv, &uv).sqrt() * dot(&w, &z) + dot(&u, &v) * dot(&wz, &wz).sqrt();
        let lengths = (dot(&u, &u) * dot(&v, &v) * dot(&w, &w) * dot(&z, &z)).sqrt();
        if !(sine < -1e-6 * lengths)
            || plane.turn(pt(a), pt(d), pt(c)) <= 0.
            || plane.turn(pt(d), pt(b), pt(c)) <= 0.
        {
            continue;
        }
        tris[t] = [a, d, c];
        tris[s] = [d, b, c];
        owner.remove(&(a, b));
        owner.remove(&(b, a));
        owner.insert((d, c), t);
        owner.insert((c, d), s);
        owner.insert((a, d), t);
        owner.insert((b, c), s);
        stack.extend([(a, d), (d, b), (b, c), (c, a)]);
    }
}

/// Every directed edge occurs once and its reverse once.
fn closed_surface(triangles: &[[u32; 3]]) -> bool {
    if triangles.len() < 4 {
        return false;
    }
    let mut edges: Vec<(u32, u32)> = triangles
        .iter()
        .flat_map(|t| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])])
        .collect();
    edges.sort_unstable();
    edges.windows(2).all(|w| w[0] != w[1])
        && edges
            .iter()
            .all(|(a, b)| a != b && edges.binary_search(&(*b, *a)).is_ok())
}
