//! Triangle mesher for decomposed 2-D geometries (`Geometry2`): our own
//! constrained Delaunay triangulation plus Ruppert/Chew quality refinement.
//!
//! 1. Every geometry edge is cut into pieces by a graded size field (at most
//!    hmax, at least hmin where the edge allows, neighbouring pieces on an
//!    edge differ by at most hgrad); shared edges are cut once, so faces
//!    conform.
//! 2. The points go into a Delaunay triangulation; missing boundary pieces
//!    are recovered by splitting them on the curve.
//! 3. Faces come from the left/right labels of the pieces and a flood fill
//!    across unconstrained edges; outside and hole triangles are dropped.
//! 4. Refinement splits encroached pieces at their parameter midpoint on the
//!    curve and inserts circumcentres of triangles with an angle below 20.7
//!    degrees, an edge above hmax, or an edge well above the interior size
//!    field (boundary piece sizes growing at rate ln(hgrad) with distance).
//!    If a split leaves two neighbouring pieces of an edge more than hgrad
//!    apart, the final piece sizes become size demands and the mesh is
//!    rebuilt (steps 1-4) with the graded field; a few passes at most.
//! 5. Quadratic meshes get shared midside nodes, on the curve for boundary
//!    pieces.
//!
//! Deterministic: no hashing, no randomness, no threads.
mod cdt;

use super::pde_geometry::{GEdge, GKind, Geometry2};
use crate::{Matrix, Result};
use cdt::{Cdt, NONE, Seg, Trace, UNKNOWN};
use std::collections::{BTreeMap, VecDeque};

/// Largest node count a mesh may reach.
const MAX_NODES: usize = 2_000_000;
/// sin(20.7 deg): triangles whose smallest angle is below it are refined.
const SIN_MIN_ANGLE: f64 = 0.353_474_843_779_257_1;
/// Largest turning angle of one boundary piece on a curved edge (radians).
const MAX_TURN: f64 = std::f64::consts::PI / 6.;

/// Mesh size options.
#[derive(Clone, Copy, Debug)]
pub struct MeshOpts {
    pub hmax: f64,
    pub hmin: f64,
    pub hgrad: f64,
    pub quadratic: bool,
}

impl MeshOpts {
    pub fn new(hmax: f64) -> Self {
        Self {
            hmax,
            hmin: hmax / 100.,
            hgrad: 1.5,
            quadratic: false,
        }
    }
}

/// Triangle mesh: counter-clockwise triangles, 1-based face of each
/// triangle, the 1-based geometry edge of each boundary or interface node,
/// and whether a node is a geometry vertex. `mid` holds the midside nodes of
/// edges (0,1), (1,2), (2,0) for quadratic meshes.
#[derive(Clone, Debug, PartialEq)]
pub struct TriMesh {
    pub nodes: Vec<[f64; 2]>,
    pub tris: Vec<[usize; 3]>,
    pub mid: Option<Vec<[usize; 3]>>,
    pub tri_face: Vec<usize>,
    pub node_edge: Vec<Option<usize>>,
    pub node_vertex: Vec<bool>,
}

/// Test hook: meshes the geometry of a decomposed geometry matrix.
#[doc(hidden)]
pub fn pde_mesh_from_dl(dl: &Matrix, opts: &MeshOpts) -> Result<TriMesh> {
    mesh(&Geometry2::from_dl(dl)?, opts)
}

pub(crate) fn mesh(g: &Geometry2, opts: &MeshOpts) -> Result<TriMesh> {
    let MeshOpts {
        hmax, hmin, hgrad, ..
    } = *opts;
    if !(hmax.is_finite() && hmax > 0.) {
        return Err("mesh: Hmax must be a positive finite number".into());
    }
    if !(hmin.is_finite() && hmin >= 0. && hmin <= hmax) {
        return Err("mesh: Hmin must be a finite number in [0, Hmax]".into());
    }
    if !(hgrad > 1. && hgrad <= 2.) {
        return Err("mesh: Hgrad must be greater than 1 and at most 2".into());
    }
    if g.edges.is_empty() || g.num_faces == 0 {
        return Err("mesh: the geometry has no faces".into());
    }
    let (lo, hi) = bounds(g);
    let est = (hi[0] - lo[0]) * (hi[1] - lo[1]) / (0.4 * hmax * hmax);
    if !est.is_finite() || est > MAX_NODES as f64 {
        return Err(format!(
            "mesh: Hmax {hmax} is too small for this geometry (limit {MAX_NODES} nodes)"
        ));
    }
    let mut demand = Demand::new(g);
    let mut pass = 0;
    loop {
        let mut m = Cdt::new(lo, hi);
        insert_boundary(&mut m, g, opts, &demand)?;
        recover_segments(&mut m, g)?;
        assign_faces(&mut m, g)?;
        let field = SizeField::new(&m, hmax, grade_slope(hgrad));
        let mut r = Refiner {
            m,
            g,
            hmax,
            hmin,
            field,
            inserted: 0,
        };
        r.run()?;
        pass += 1;
        if pass >= MAX_PASSES || !demand.absorb(&r.m, g, hgrad) {
            return Ok(output(&r.m, g, opts.quadratic));
        }
    }
}

/// Most mesh passes (each later pass refines the boundary where the
/// previous one split pieces beyond hgrad).
const MAX_PASSES: usize = 6;

/// Growth rate of the size field per unit length. A size field growing at
/// rate k gives neighbouring pieces a ratio of at most e^k, so ln(hgrad)
/// keeps them within hgrad; 0.9 leaves room for table resolution.
fn grade_slope(hgrad: f64) -> f64 {
    0.9 * hgrad.ln()
}

fn bounds(g: &Geometry2) -> ([f64; 2], [f64; 2]) {
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    for e in &g.edges {
        for k in 0..=16 {
            let p = e.point_at(f64::from(k) / 16.);
            for d in 0..2 {
                lo[d] = lo[d].min(p[d]);
                hi[d] = hi[d].max(p[d]);
            }
        }
    }
    (lo, hi)
}

// ---------------------------------------------------------------------------
// Step 1: boundary discretisation.

/// Upper size bound from curvature: pieces turn at most MAX_TURN.
fn curve_cap(kind: &GKind) -> f64 {
    match *kind {
        GKind::Line => f64::INFINITY,
        GKind::Arc { r, .. } => r * MAX_TURN,
        GKind::EllipseArc { a, b, .. } => a.min(b).powi(2) / a.max(b) * MAX_TURN,
    }
}

/// Cumulative arc length at fractions k / n (normalised to 1 at the end).
fn arc_table(e: &GEdge, n: usize) -> Vec<f64> {
    let mut acc = vec![0.; n + 1];
    for k in 0..n {
        let s = (k as f64 + 0.5) / n as f64;
        let d = e.tangent_at(s);
        acc[k + 1] = acc[k] + d[0].hypot(d[1]);
    }
    let total = acc[n];
    acc.iter().map(|x| x / total).collect()
}

/// Resolution of the ellipse arc-length tables.
const ARC_N: usize = 2048;

/// Geometry vertices and size demands: the smallest piece sizes seen at
/// each vertex and along each edge in earlier passes.
struct Demand {
    corners: Vec<[f64; 2]>,
    /// Start and end vertex of each edge.
    ends: Vec<(usize, usize)>,
    lens: Vec<f64>,
    /// Arc-length fraction at point_at fraction k / ARC_N (ellipse arcs).
    tables: Vec<Option<Vec<f64>>>,
    vert: Vec<f64>,
    /// Per edge: (arc-length fraction, size).
    edge: Vec<Vec<(f64, f64)>>,
}

impl Demand {
    fn new(g: &Geometry2) -> Self {
        // Geometry vertices, merged by exact coordinates.
        let mut ids: BTreeMap<(u64, u64), usize> = BTreeMap::new();
        let mut corners: Vec<[f64; 2]> = Vec::new();
        let mut ends = Vec::with_capacity(g.edges.len());
        for e in &g.edges {
            let mut id = |p: [f64; 2]| {
                let key = ((p[0] + 0.).to_bits(), (p[1] + 0.).to_bits());
                *ids.entry(key).or_insert_with(|| {
                    corners.push(p);
                    corners.len() - 1
                })
            };
            ends.push((id(e.start), id(e.end)));
        }
        let vert = vec![f64::INFINITY; corners.len()];
        Self {
            corners,
            ends,
            lens: g.edges.iter().map(GEdge::length).collect(),
            tables: g
                .edges
                .iter()
                .map(|e| matches!(e.kind, GKind::EllipseArc { .. }).then(|| arc_table(e, ARC_N)))
                .collect(),
            vert,
            edge: vec![Vec::new(); g.edges.len()],
        }
    }

    /// point_at fraction to arc-length fraction on edge j.
    fn arc_frac(&self, j: usize, s: f64) -> f64 {
        match &self.tables[j] {
            None => s,
            Some(t) => {
                let x = (s * ARC_N as f64).clamp(0., ARC_N as f64);
                let i = (x.floor() as usize).min(ARC_N - 1);
                t[i] + (x - i as f64) * (t[i + 1] - t[i])
            }
        }
    }

    /// Arc-length fraction to point_at fraction on edge j.
    fn param(&self, j: usize, frac: f64) -> f64 {
        match &self.tables[j] {
            None => frac,
            Some(t) => {
                let i = t.partition_point(|&x| x < frac).clamp(1, ARC_N);
                let w = (frac - t[i - 1]) / (t[i] - t[i - 1]);
                (i as f64 - 1. + w) / ARC_N as f64
            }
        }
    }

    /// Checks the pieces of every edge of a finished pass. If two neighbours
    /// on one edge differ by more than hgrad, records every piece size as a
    /// demand and returns true (mesh again).
    fn absorb(&mut self, m: &Cdt, g: &Geometry2, hgrad: f64) -> bool {
        let mut pieces: Vec<Vec<(f64, f64)>> = vec![Vec::new(); g.edges.len()];
        for seg in &m.segs {
            let j = seg.edge;
            let (f0, f1) = (self.arc_frac(j, seg.s0), self.arc_frac(j, seg.s1));
            pieces[j].push((f0, (f1 - f0).abs() * self.lens[j]));
        }
        let mut bad = false;
        for p in &mut pieces {
            p.sort_by(|x, y| x.0.total_cmp(&y.0));
            bad |= p
                .windows(2)
                .any(|w| w[0].1.max(w[1].1) > hgrad * w[0].1.min(w[1].1) * (1. + 1e-12));
        }
        if !bad {
            return false;
        }
        for (j, p) in pieces.iter().enumerate() {
            let (a, b) = self.ends[j];
            for (k, &(f0, h)) in p.iter().enumerate() {
                let f1 = p.get(k + 1).map_or(1., |x| x.0);
                self.edge[j].push((f0, h));
                self.edge[j].push((f1, h));
                if k == 0 {
                    self.vert[a] = self.vert[a].min(h);
                }
                if k + 1 == p.len() {
                    self.vert[b] = self.vert[b].min(h);
                }
            }
        }
        true
    }
}

/// Fractions in (0, 1) (of point_at) of the interior points of edge j: the
/// pieces follow the size field
/// min(ha + k x, hb + k (len - x), cap, demand d + k |x - xd|), floored at
/// hmin, with k the grading slope; None if that needs too many pieces.
fn edge_fractions(
    dem: &Demand,
    j: usize,
    ha: f64,
    hb: f64,
    cap: f64,
    opts: &MeshOpts,
) -> Option<Vec<f64>> {
    let len = dem.lens[j];
    let slope = grade_slope(opts.hgrad);
    let demands = &dem.edge[j];
    let floor = opts.hmin.max(f64::MIN_POSITIVE);
    let low = demands
        .iter()
        .fold(ha.min(hb).min(cap), |m, d| m.min(d.1))
        .max(floor);
    if len / low > MAX_NODES as f64 {
        return None;
    }
    // At least 32 table cells per smallest piece.
    let n = ((32. * len / low).ceil() as usize).clamp(2048, 1 << 22);
    let dx = len / n as f64;
    let x_at = |k: usize| (k as f64 + 0.5) * dx;
    let mut size: Vec<f64> = (0..n)
        .map(|k| {
            let x = x_at(k);
            (ha + slope * x).min(hb + slope * (len - x)).min(cap)
        })
        .collect();
    for &(f, d) in demands {
        let xd = f * len;
        let k = ((xd / dx) as usize).min(n - 1);
        size[k] = size[k].min(d + slope * (x_at(k) - xd).abs());
    }
    for k in 1..n {
        size[k] = size[k].min(size[k - 1] + slope * dx);
    }
    for k in (0..n - 1).rev() {
        size[k] = size[k].min(size[k + 1] + slope * dx);
    }
    let mut acc = vec![0.; n + 1];
    for k in 0..n {
        acc[k + 1] = acc[k] + dx / size[k].max(floor);
    }
    let total = acc[n];
    let pieces = ((total - 1e-6).ceil() as usize).max(1);
    if pieces > MAX_NODES {
        return None;
    }
    let mut out = Vec::with_capacity(pieces - 1);
    let mut k = 0;
    for i in 1..pieces {
        let target = total * i as f64 / pieces as f64;
        while acc[k + 1] < target {
            k += 1;
        }
        let w = (target - acc[k]) / (acc[k + 1] - acc[k]);
        out.push(dem.param(j, (k as f64 + w) / n as f64));
    }
    Some(out)
}

fn insert_boundary(m: &mut Cdt, g: &Geometry2, opts: &MeshOpts, dem: &Demand) -> Result<()> {
    let (corners, ends, lens) = (&dem.corners, &dem.ends, &dem.lens);
    let caps: Vec<f64> = g
        .edges
        .iter()
        .map(|e| curve_cap(&e.kind).min(opts.hmax))
        .collect();
    // Vertex sizes: the smallest incident piece size or demand, graded
    // across edges.
    let mut hv: Vec<f64> = dem.vert.iter().map(|&d| d.min(opts.hmax)).collect();
    for (j, &(a, b)) in ends.iter().enumerate() {
        let h = caps[j].min(lens[j]);
        hv[a] = hv[a].min(h);
        hv[b] = hv[b].min(h);
    }
    let slope = grade_slope(opts.hgrad);
    for _ in 0..corners.len() {
        let mut changed = false;
        for (j, &(a, b)) in ends.iter().enumerate() {
            let via = slope * lens[j];
            if hv[a] + via < hv[b] {
                hv[b] = hv[a] + via;
                changed = true;
            }
            if hv[b] + via < hv[a] {
                hv[a] = hv[b] + via;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut hint = 0;
    let mut vid = Vec::with_capacity(corners.len());
    for (i, &p) in corners.iter().enumerate() {
        let (v, new) = m.insert_free(p, hint, NONE).ok_or_else(|| {
            format!(
                "mesh: could not insert geometry vertex {} ({}, {})",
                i + 1,
                p[0],
                p[1]
            )
        })?;
        m.vcorner[v] = true;
        hint = new[0];
        vid.push(v);
    }
    let too_many =
        || format!("mesh: boundary needs more than {MAX_NODES} nodes; increase Hmax or Hmin");
    let mut total = corners.len();
    for (j, e) in g.edges.iter().enumerate() {
        let (a, b) = ends[j];
        let fr = edge_fractions(dem, j, hv[a], hv[b], caps[j], opts).ok_or_else(too_many)?;
        total += fr.len();
        if total > MAX_NODES {
            return Err(too_many());
        }
        let mut prev = (vid[a], 0.);
        for &s in fr.iter().chain(std::iter::once(&1.)) {
            let v = if s >= 1. {
                vid[b]
            } else {
                let p = e.point_at(s);
                let (v, new) = m
                    .insert_free(p, hint, j)
                    .ok_or_else(|| format!("mesh: could not insert a point on edge {}", j + 1))?;
                hint = new[0];
                v
            };
            m.segs.push(Seg {
                a: prev.0,
                b: v,
                edge: j,
                s0: prev.1,
                s1: s,
            });
            prev = (v, s);
        }
        // An edge whose ends coincide (a full loop) needs at least 3 pieces.
        if a == b && fr.len() < 2 {
            return Err(format!("mesh: closed edge {} is too coarse", j + 1));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Step 2: segment recovery by splitting on the curve.

fn recover_segments(m: &mut Cdt, g: &Geometry2) -> Result<()> {
    let mut stack: Vec<usize> = (0..m.segs.len()).rev().collect();
    let cap = 64 * m.segs.len() + 1024;
    let mut splits = 0;
    while let Some(s) = stack.pop() {
        let seg = m.segs[s];
        if let Some((t, k)) = m.find_edge(seg.a, seg.b) {
            m.set_seg(t, k, s);
            continue;
        }
        splits += 1;
        if splits > cap {
            return Err(format!(
                "mesh: could not recover boundary edge {}",
                seg.edge + 1
            ));
        }
        let sm = (seg.s0 + seg.s1) / 2.;
        let p = g.edges[seg.edge].point_at(sm);
        let hint = m.last;
        let (v, _) = m
            .insert_free(p, hint, seg.edge)
            .ok_or_else(|| format!("mesh: could not recover boundary edge {}", seg.edge + 1))?;
        let s2 = m.segs.len();
        m.segs[s] = Seg {
            b: v,
            s1: sm,
            ..seg
        };
        m.segs.push(Seg {
            a: v,
            s0: sm,
            ..seg
        });
        stack.push(s2);
        stack.push(s);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Step 3: faces.

fn assign_faces(m: &mut Cdt, g: &Geometry2) -> Result<()> {
    let mut queue = VecDeque::new();
    for s in 0..m.segs.len() {
        let seg = m.segs[s];
        let e = &g.edges[seg.edge];
        for (a, b, f) in [(seg.a, seg.b, e.left), (seg.b, seg.a, e.right)] {
            let (t, _) = m.find_edge(a, b).ok_or("mesh: lost a boundary edge")?;
            if m.face[t] == UNKNOWN {
                m.face[t] = f;
                queue.push_back(t);
            } else if m.face[t] != f {
                return Err(format!(
                    "mesh: inconsistent face labels near edge {}",
                    seg.edge + 1
                ));
            }
        }
    }
    while let Some(t) = queue.pop_front() {
        for k in 0..3 {
            let n = m.tn[t][k];
            if n == NONE || m.ts[t][k] != NONE {
                continue;
            }
            if m.face[n] == UNKNOWN {
                m.face[n] = m.face[t];
                queue.push_back(n);
            } else if m.face[n] != m.face[t] {
                return Err(
                    "mesh: the geometry boundary is not closed (faces leak into each other)".into(),
                );
            }
        }
    }
    for t in 0..m.tv.len() {
        if !m.alive[t] {
            continue;
        }
        let outer = m.tv[t].iter().any(|&v| v < 3);
        if m.face[t] == UNKNOWN || outer {
            if outer && m.face[t] != UNKNOWN && m.face[t] != 0 {
                return Err(
                    "mesh: a face reaches outside the geometry (boundary not closed)".into(),
                );
            }
            m.face[t] = 0;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Step 4: Ruppert/Chew refinement.

struct Refiner<'a> {
    m: Cdt,
    g: &'a Geometry2,
    hmax: f64,
    hmin: f64,
    field: SizeField,
    inserted: usize,
}

/// A triangle is too large if its longest edge exceeds FIELD_SLACK times
/// the size field at its centroid (the slack keeps triangles on a boundary
/// piece of size h, whose edges run up to about 1.4 h, from being split).
const FIELD_SLACK: f64 = 1.5;
/// Cells per side of the size-field grid (at most).
const FIELD_GRID: usize = 256;
/// Size-field grid cells per boundary node (about).
const FIELD_CELLS_PER_NODE: f64 = 4.;

/// Interior size field: min over boundary nodes v of h_v + k |x - v|
/// (h_v the shortest boundary piece at v, k the grading slope), capped at
/// hmax. Evaluated through a grid that keeps, per cell, the node giving the
/// smallest value at its centre; a query takes the best of the 3 x 3 cells
/// around the point.
struct SizeField {
    lo: [f64; 2],
    cell: f64,
    nx: usize,
    ny: usize,
    src: Vec<usize>,
    pts: Vec<([f64; 2], f64)>,
    slope: f64,
    hmax: f64,
}

impl SizeField {
    fn new(m: &Cdt, hmax: f64, slope: f64) -> Self {
        let mut h: BTreeMap<usize, f64> = BTreeMap::new();
        for seg in &m.segs {
            let l = dist2(m.pts[seg.a], m.pts[seg.b]).sqrt();
            for v in [seg.a, seg.b] {
                let e = h.entry(v).or_insert(l);
                *e = e.min(l);
            }
        }
        let pts: Vec<([f64; 2], f64)> = h
            .into_iter()
            .filter(|&(_, l)| FIELD_SLACK * l < hmax)
            .map(|(v, l)| (m.pts[v], l))
            .collect();
        let mut lo = [f64::INFINITY; 2];
        let mut hi = [f64::NEG_INFINITY; 2];
        for (p, _) in &pts {
            for d in 0..2 {
                lo[d] = lo[d].min(p[d]);
                hi[d] = hi[d].max(p[d]);
            }
        }
        let mut f = Self {
            lo,
            cell: 1.,
            nx: 0,
            ny: 0,
            src: Vec::new(),
            pts,
            slope,
            hmax,
        };
        if f.pts.is_empty() {
            return f;
        }
        let span = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(f64::MIN_POSITIVE);
        let area = (hi[0] - lo[0]).max(span / FIELD_GRID as f64)
            * (hi[1] - lo[1]).max(span / FIELD_GRID as f64);
        f.cell = (area / (FIELD_CELLS_PER_NODE * f.pts.len() as f64))
            .sqrt()
            .max(span / FIELD_GRID as f64);
        f.nx = (((hi[0] - lo[0]) / f.cell) as usize + 1).min(FIELD_GRID + 1);
        f.ny = (((hi[1] - lo[1]) / f.cell) as usize + 1).min(FIELD_GRID + 1);
        f.src = vec![NONE; f.nx * f.ny];
        for i in 0..f.pts.len() {
            let c = f.cell_of(f.pts[i].0);
            if f.src[c] == NONE || f.value(i, f.centre(c)) < f.value(f.src[c], f.centre(c)) {
                f.src[c] = i;
            }
        }
        // Raster sweeps passing each cell's source to its 8 neighbours.
        for _ in 0..16 {
            let mut changed = false;
            for dir in [false, true] {
                for idx in 0..f.src.len() {
                    let c = if dir { f.src.len() - 1 - idx } else { idx };
                    let (ix, iy) = (c % f.nx, c / f.nx);
                    let centre = f.centre(c);
                    for (dx, dy) in [
                        (-1, -1),
                        (0, -1),
                        (1, -1),
                        (-1, 0),
                        (1, 0),
                        (-1, 1),
                        (0, 1),
                        (1, 1),
                    ] {
                        let (jx, jy) = (ix as isize + dx, iy as isize + dy);
                        if jx < 0 || jy < 0 || jx >= f.nx as isize || jy >= f.ny as isize {
                            continue;
                        }
                        let s = f.src[jy as usize * f.nx + jx as usize];
                        if s != NONE
                            && s != f.src[c]
                            && (f.src[c] == NONE || f.value(s, centre) < f.value(f.src[c], centre))
                        {
                            f.src[c] = s;
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        f
    }

    fn value(&self, i: usize, p: [f64; 2]) -> f64 {
        let (q, h) = self.pts[i];
        h + self.slope * dist2(p, q).sqrt()
    }

    fn cell_of(&self, p: [f64; 2]) -> usize {
        let ix = ((p[0] - self.lo[0]) / self.cell).max(0.) as usize;
        let iy = ((p[1] - self.lo[1]) / self.cell).max(0.) as usize;
        iy.min(self.ny - 1) * self.nx + ix.min(self.nx - 1)
    }

    fn centre(&self, c: usize) -> [f64; 2] {
        let (ix, iy) = (c % self.nx, c / self.nx);
        [
            self.lo[0] + (ix as f64 + 0.5) * self.cell,
            self.lo[1] + (iy as f64 + 0.5) * self.cell,
        ]
    }

    fn at(&self, p: [f64; 2]) -> f64 {
        if self.src.is_empty() {
            return self.hmax;
        }
        let c = self.cell_of(p);
        let (ix, iy) = ((c % self.nx) as isize, (c / self.nx) as isize);
        let mut best = self.hmax;
        for jy in iy - 1..=iy + 1 {
            for jx in ix - 1..=ix + 1 {
                if jx < 0 || jy < 0 || jx >= self.nx as isize || jy >= self.ny as isize {
                    continue;
                }
                let s = self.src[jy as usize * self.nx + jx as usize];
                if s != NONE {
                    best = best.min(self.value(s, p));
                }
            }
        }
        best
    }
}

fn dist2(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}

/// p lies strictly inside the diametral circle of a-b.
fn encroaches(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> bool {
    (a[0] - p[0]) * (b[0] - p[0]) + (a[1] - p[1]) * (b[1] - p[1]) < 0.
}

fn circumcentre(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> Option<[f64; 2]> {
    let (bx, by) = (b[0] - a[0], b[1] - a[1]);
    let (cx, cy) = (c[0] - a[0], c[1] - a[1]);
    let d = 2. * (bx * cy - by * cx);
    if d == 0. {
        return None;
    }
    let (b2, c2) = (bx * bx + by * by, cx * cx + cy * cy);
    let ux = (cy * b2 - by * c2) / d;
    let uy = (bx * c2 - cx * b2) / d;
    let p = [a[0] + ux, a[1] + uy];
    (p[0].is_finite() && p[1].is_finite()).then_some(p)
}

fn too_many() -> String {
    format!("mesh: refinement did not finish within {MAX_NODES} nodes; increase Hmax or Hmin")
}

enum Quality {
    Good,
    /// Too large (longest edge above hmax).
    Size,
    /// Small angle only.
    Angle,
}

impl Refiner<'_> {
    fn run(&mut self) -> Result<()> {
        let mut segq: VecDeque<usize> = (0..self.m.segs.len()).collect();
        let mut triq: VecDeque<(usize, [usize; 3])> = (0..self.m.tv.len())
            .filter(|&t| self.m.alive[t] && self.m.face[t] > 0)
            .map(|t| (t, self.m.tv[t]))
            .collect();
        let cap = MAX_NODES.saturating_sub(self.m.pts.len());
        loop {
            while let Some(s) = segq.pop_front() {
                if self.encroached(s) && self.splittable(s) {
                    self.split(s, &mut segq, &mut triq);
                }
                if self.inserted > cap {
                    return Err(too_many());
                }
            }
            let Some((t, verts)) = triq.pop_front() else {
                break;
            };
            if !self.m.alive[t] || self.m.tv[t] != verts || self.m.face[t] == 0 {
                continue;
            }
            if matches!(self.quality(t), Quality::Good) {
                continue;
            }
            self.refine(t, &mut segq, &mut triq);
            if self.inserted > cap {
                return Err(too_many());
            }
        }
        Ok(())
    }

    fn encroached(&self, s: usize) -> bool {
        let seg = self.m.segs[s];
        let (a, b) = (self.m.pts[seg.a], self.m.pts[seg.b]);
        for (x, y) in [(seg.a, seg.b), (seg.b, seg.a)] {
            if let Some((t, k)) = self.m.find_edge(x, y)
                && self.m.face[t] > 0
                && encroaches(a, b, self.m.pts[self.m.tv[t][k]])
            {
                return true;
            }
        }
        false
    }

    fn splittable(&self, s: usize) -> bool {
        let seg = self.m.segs[s];
        let len = dist2(self.m.pts[seg.a], self.m.pts[seg.b]).sqrt();
        let (lo, hi) = (seg.s0, seg.s1);
        len >= 2. * self.hmin && len > 0. && (hi - lo) > 4. * f64::EPSILON
    }

    fn quality(&self, t: usize) -> Quality {
        let [a, b, c] = self.m.tv[t].map(|v| self.m.pts[v]);
        let l = [dist2(b, c), dist2(c, a), dist2(a, b)];
        let longest = l[0].max(l[1]).max(l[2]);
        if longest > self.hmax * self.hmax * (1. + 1e-12) {
            return Quality::Size;
        }
        let centroid = [(a[0] + b[0] + c[0]) / 3., (a[1] + b[1] + c[1]) / 3.];
        let h = FIELD_SLACK * self.field.at(centroid).max(self.hmin);
        if h < self.hmax && longest > h * h {
            return Quality::Size;
        }
        let area2 = cdt::orient(a, b, c);
        let (i, shortest) = l
            .iter()
            .copied()
            .enumerate()
            .fold(
                (0, f64::INFINITY),
                |m, (i, x)| if x < m.1 { (i, x) } else { m },
            );
        // sin(smallest angle) = shortest * 2 area / (product of edges).
        let sin_min = shortest.sqrt() * area2 / (l[0] * l[1] * l[2]).sqrt();
        if sin_min >= SIN_MIN_ANGLE || shortest.sqrt() < self.hmin {
            return Quality::Good;
        }
        // The smallest angle sits opposite the shortest edge; leave angles
        // between two input segments alone (they cannot be improved).
        let (e1, e2) = ((i + 1) % 3, (i + 2) % 3);
        if self.m.ts[t][e1] != NONE && self.m.ts[t][e2] != NONE {
            return Quality::Good;
        }
        Quality::Angle
    }

    fn push_new(
        &self,
        new: &[usize],
        segq: &mut VecDeque<usize>,
        triq: &mut VecDeque<(usize, [usize; 3])>,
    ) {
        for &t in new {
            if self.m.face[t] > 0 {
                triq.push_back((t, self.m.tv[t]));
            }
            for k in 0..3 {
                let s = self.m.ts[t][k];
                if s != NONE {
                    segq.push_back(s);
                }
            }
        }
    }

    /// Puts t back in the queue if a segment split left it in place.
    fn requeue(&self, t: usize, verts: [usize; 3], triq: &mut VecDeque<(usize, [usize; 3])>) {
        if self.m.alive[t] && self.m.tv[t] == verts {
            triq.push_back((t, verts));
        }
    }

    fn split(
        &mut self,
        s: usize,
        segq: &mut VecDeque<usize>,
        triq: &mut VecDeque<(usize, [usize; 3])>,
    ) -> bool {
        let seg = self.m.segs[s];
        let sm = (seg.s0 + seg.s1) / 2.;
        let p = self.g.edges[seg.edge].point_at(sm);
        match self.m.split_segment(s, sm, p) {
            Some((_, new)) => {
                self.inserted += 1;
                self.push_new(&new, segq, triq);
                true
            }
            None => false,
        }
    }

    fn refine(
        &mut self,
        t: usize,
        segq: &mut VecDeque<usize>,
        triq: &mut VecDeque<(usize, [usize; 3])>,
    ) {
        let verts = self.m.tv[t];
        let [a, b, c] = verts.map(|v| self.m.pts[v]);
        let Some(cc) = circumcentre(a, b, c) else {
            return;
        };
        let inside = match self.m.trace(t, cc) {
            Trace::Inside(u) => u,
            Trace::Blocked(s) => {
                if self.splittable(s) && self.split(s, segq, triq) {
                    self.requeue(t, verts, triq);
                }
                return;
            }
            Trace::Failed => return,
        };
        let Some(cav) = self.m.cavity(cc, &[inside]) else {
            return;
        };
        let hit: Vec<usize> = self
            .m
            .cavity_boundary(&cav)
            .iter()
            .filter(|e| e.3 != NONE && encroaches(self.m.pts[e.0], self.m.pts[e.1], cc))
            .map(|e| e.3)
            .collect();
        if !hit.is_empty() {
            let mut any = false;
            for s in hit {
                if self.splittable(s) {
                    any |= self.split(s, segq, triq);
                }
            }
            if any {
                self.requeue(t, verts, triq);
            }
            return;
        }
        if let Some((_, new)) = self.m.commit(cc, &cav, None, NONE) {
            self.inserted += 1;
            self.push_new(&new, segq, triq);
        }
    }
}

// ---------------------------------------------------------------------------
// Output and step 5 (quadratic midside nodes).

fn output(m: &Cdt, g: &Geometry2, quadratic: bool) -> TriMesh {
    let mut map = vec![NONE; m.pts.len()];
    let mut nodes = Vec::new();
    let mut node_edge = Vec::new();
    let mut node_vertex = Vec::new();
    let mut tris = Vec::new();
    let mut tri_face = Vec::new();
    // Corner nodes get the lowest-numbered edge that touches them.
    let mut corner_edge = vec![NONE; m.pts.len()];
    for seg in &m.segs {
        for v in [seg.a, seg.b] {
            if m.vcorner[v] && seg.edge < corner_edge[v] {
                corner_edge[v] = seg.edge;
            }
        }
    }
    for v in 3..m.pts.len() {
        map[v] = nodes.len();
        nodes.push(m.pts[v]);
        let e = if m.vcorner[v] {
            corner_edge[v]
        } else {
            m.vedge[v]
        };
        node_edge.push((e != NONE).then_some(e + 1));
        node_vertex.push(m.vcorner[v]);
    }
    for t in 0..m.tv.len() {
        if m.alive[t] && m.face[t] > 0 && m.face[t] != UNKNOWN {
            tris.push(m.tv[t].map(|v| map[v]));
            tri_face.push(m.face[t]);
        }
    }
    // Every inserted vertex is a domain vertex, but compact in case one is not.
    let mut used = vec![false; nodes.len()];
    for t in &tris {
        for &v in t {
            used[v] = true;
        }
    }
    let mut mesh = TriMesh {
        nodes,
        tris,
        mid: None,
        tri_face,
        node_edge,
        node_vertex,
    };
    if used.iter().any(|&u| !u) {
        compact(&mut mesh, &used);
    }
    if quadratic {
        add_midside(&mut mesh, m, &map, g);
    }
    mesh
}

fn compact(mesh: &mut TriMesh, used: &[bool]) {
    let mut map = vec![NONE; used.len()];
    let mut n = 0;
    for (v, &u) in used.iter().enumerate() {
        if u {
            map[v] = n;
            mesh.nodes[n] = mesh.nodes[v];
            mesh.node_edge[n] = mesh.node_edge[v];
            mesh.node_vertex[n] = mesh.node_vertex[v];
            n += 1;
        }
    }
    mesh.nodes.truncate(n);
    mesh.node_edge.truncate(n);
    mesh.node_vertex.truncate(n);
    for t in &mut mesh.tris {
        *t = t.map(|v| map[v]);
    }
}

fn add_midside(mesh: &mut TriMesh, m: &Cdt, map: &[usize], g: &Geometry2) {
    // Boundary pieces by their (sorted) end nodes.
    let mut pieces: BTreeMap<(usize, usize), Seg> = BTreeMap::new();
    for seg in &m.segs {
        let (a, b) = (map[seg.a], map[seg.b]);
        pieces.insert((a.min(b), a.max(b)), *seg);
    }
    let mut made: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    let mut mid = Vec::with_capacity(mesh.tris.len());
    for t in 0..mesh.tris.len() {
        let tri = mesh.tris[t];
        let mut row = [0; 3];
        for (k, slot) in row.iter_mut().enumerate() {
            let (a, b) = (tri[k], tri[(k + 1) % 3]);
            let key = (a.min(b), a.max(b));
            *slot = *made.entry(key).or_insert_with(|| {
                let (p, edge) = match pieces.get(&key) {
                    Some(seg) => (
                        g.edges[seg.edge].point_at((seg.s0 + seg.s1) / 2.),
                        Some(seg.edge + 1),
                    ),
                    None => {
                        let (pa, pb) = (mesh.nodes[a], mesh.nodes[b]);
                        ([(pa[0] + pb[0]) / 2., (pa[1] + pb[1]) / 2.], None)
                    }
                };
                mesh.nodes.push(p);
                mesh.node_edge.push(edge);
                mesh.node_vertex.push(false);
                mesh.nodes.len() - 1
            });
        }
        mid.push(row);
    }
    mesh.mid = Some(mid);
}
