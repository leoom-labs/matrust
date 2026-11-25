//! Constrained Delaunay triangulation kernel: triangles with adjacency,
//! Bowyer-Watson cavity insertion that stops at constrained edges, segment
//! splitting, point location and segment-to-triangle lookup. Every
//! predicate uses the exact `robust` orient/incircle tests.
//!
//! Triangle `t` has vertices `tv[t]` counter-clockwise; edge `k` is the edge
//! opposite vertex `k`, from `tv[t][(k+1)%3]` to `tv[t][(k+2)%3]`; `tn[t][k]`
//! is the neighbour across it and `ts[t][k]` the constrained segment on it.

pub(super) const NONE: usize = usize::MAX;
/// Face label of a triangle before the flood fill.
pub(super) const UNKNOWN: usize = usize::MAX;

pub(super) fn orient(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    robust::orient2d(
        robust::Coord { x: a[0], y: a[1] },
        robust::Coord { x: b[0], y: b[1] },
        robust::Coord { x: c[0], y: c[1] },
    )
}

pub(super) fn incircle(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> f64 {
    robust::incircle(
        robust::Coord { x: a[0], y: a[1] },
        robust::Coord { x: b[0], y: b[1] },
        robust::Coord { x: c[0], y: c[1] },
        robust::Coord { x: d[0], y: d[1] },
    )
}

/// One constrained boundary piece: chord from `a` to `b` of geometry edge
/// `edge` (0-based), covering the `point_at` fractions `s0 < s1`.
#[derive(Clone, Copy, Debug)]
pub(super) struct Seg {
    pub a: usize,
    pub b: usize,
    pub edge: usize,
    pub s0: f64,
    pub s1: f64,
}

pub(super) struct Cdt {
    pub pts: Vec<[f64; 2]>,
    /// Geometry edge (0-based) of each vertex on a boundary, else NONE.
    pub vedge: Vec<usize>,
    /// Vertex is an end point of a geometry edge.
    pub vcorner: Vec<bool>,
    pub tv: Vec<[usize; 3]>,
    pub tn: Vec<[usize; 3]>,
    pub ts: Vec<[usize; 3]>,
    pub face: Vec<usize>,
    pub alive: Vec<bool>,
    pub segs: Vec<Seg>,
    vtri: Vec<usize>,
    free: Vec<usize>,
    mark: Vec<bool>,
    pub last: usize,
    walks: usize,
}

/// Outcome of tracing a straight line from a triangle towards a point.
pub(super) enum Trace {
    Inside(usize),
    Blocked(usize),
    Failed,
}

impl Cdt {
    /// Triangulation of one large triangle around the box `lo`..`hi`.
    pub fn new(lo: [f64; 2], hi: [f64; 2]) -> Self {
        let d = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(f64::MIN_POSITIVE);
        let c = [(lo[0] + hi[0]) / 2., (lo[1] + hi[1]) / 2.];
        let pts = vec![
            [c[0] - 40. * d, c[1] - 30. * d],
            [c[0] + 40. * d, c[1] - 30. * d],
            [c[0], c[1] + 40. * d],
        ];
        Self {
            pts,
            vedge: vec![NONE; 3],
            vcorner: vec![false; 3],
            tv: vec![[0, 1, 2]],
            tn: vec![[NONE; 3]],
            ts: vec![[NONE; 3]],
            face: vec![UNKNOWN],
            alive: vec![true],
            segs: Vec::new(),
            vtri: vec![0; 3],
            free: Vec::new(),
            mark: vec![false],
            last: 0,
            walks: 0,
        }
    }

    pub fn edge(&self, t: usize, k: usize) -> (usize, usize) {
        (self.tv[t][(k + 1) % 3], self.tv[t][(k + 2) % 3])
    }

    fn p(&self, v: usize) -> [f64; 2] {
        self.pts[v]
    }

    fn in_circle(&self, t: usize, p: [f64; 2]) -> bool {
        let [a, b, c] = self.tv[t];
        incircle(self.p(a), self.p(b), self.p(c), p) > 0.
    }

    pub fn contains(&self, t: usize, p: [f64; 2]) -> bool {
        (0..3).all(|k| {
            let (a, b) = self.edge(t, k);
            orient(self.p(a), self.p(b), p) >= 0.
        })
    }

    /// Triangle containing p (visibility walk with a brute-force fallback).
    pub fn locate(&mut self, p: [f64; 2], start: usize) -> Option<usize> {
        let mut t = if self.alive.get(start) == Some(&true) {
            start
        } else {
            self.last
        };
        if !self.alive[t] {
            t = self.alive.iter().position(|&a| a)?;
        }
        let cap = 4 * self.tv.len() + 64;
        for _ in 0..cap {
            self.walks = self.walks.wrapping_add(1);
            let first = self.walks % 3;
            let mut next = None;
            for j in 0..3 {
                let k = (first + j) % 3;
                let (a, b) = self.edge(t, k);
                if orient(self.p(a), self.p(b), p) < 0. {
                    next = Some(self.tn[t][k]);
                    break;
                }
            }
            match next {
                None => return Some(t),
                Some(NONE) => return None,
                Some(n) => t = n,
            }
        }
        (0..self.tv.len()).find(|&t| self.alive[t] && self.contains(t, p))
    }

    /// Triangle and edge index holding the directed edge a -> b.
    pub fn find_edge(&self, a: usize, b: usize) -> Option<(usize, usize)> {
        let start = self.vtri[a];
        let mut t = start;
        // Rotate one way; if the ring is open, rotate the other way too.
        for dir in [1usize, 2] {
            for _ in 0..100_000 {
                let i = self.tv[t].iter().position(|&v| v == a)?;
                if self.tv[t][(i + 1) % 3] == b {
                    return Some((t, (i + 2) % 3));
                }
                if self.tv[t][(i + 2) % 3] == b {
                    // b -> a in this triangle; a -> b is across that edge.
                    let n = self.tn[t][(i + 1) % 3];
                    if n == NONE {
                        return None;
                    }
                    let j = self.tv[n].iter().position(|&v| v == a)?;
                    return Some((n, (j + 2) % 3));
                }
                let n = self.tn[t][(i + dir) % 3];
                if n == NONE || n == start {
                    break;
                }
                t = n;
            }
            t = start;
        }
        None
    }

    /// Marks edge k of t and its twin as segment s.
    pub fn set_seg(&mut self, t: usize, k: usize, s: usize) {
        self.ts[t][k] = s;
        let n = self.tn[t][k];
        if n != NONE {
            let (a, b) = self.edge(t, k);
            let j = self.twin_index(n, a, b);
            self.ts[n][j] = s;
        }
    }

    /// Index of the edge of t joining a and b (either direction).
    fn twin_index(&self, t: usize, a: usize, b: usize) -> usize {
        (0..3)
            .find(|&k| self.tv[t][k] != a && self.tv[t][k] != b)
            .unwrap_or(0)
    }

    /// Cavity of p: seeds plus every triangle reachable across unconstrained
    /// edges whose circumcircle holds p, grown until p sees every boundary
    /// edge. None when p cannot be inserted without crossing a constraint.
    pub fn cavity(&mut self, p: [f64; 2], seeds: &[usize]) -> Option<Vec<usize>> {
        let mut cav = Vec::new();
        for &s in seeds {
            if !self.mark[s] {
                self.mark[s] = true;
                cav.push(s);
            }
        }
        let mut i = 0;
        while i < cav.len() {
            let t = cav[i];
            i += 1;
            for k in 0..3 {
                let n = self.tn[t][k];
                if n != NONE && !self.mark[n] && self.ts[t][k] == NONE && self.in_circle(n, p) {
                    self.mark[n] = true;
                    cav.push(n);
                }
            }
        }
        let mut ok = true;
        'grow: loop {
            for idx in 0..cav.len() {
                let t = cav[idx];
                for k in 0..3 {
                    let n = self.tn[t][k];
                    if n != NONE && self.mark[n] {
                        continue;
                    }
                    let (a, b) = self.edge(t, k);
                    if orient(self.p(a), self.p(b), p) <= 0. {
                        if n == NONE || self.ts[t][k] != NONE || cav.len() > 4096 {
                            ok = false;
                            break 'grow;
                        }
                        self.mark[n] = true;
                        cav.push(n);
                        continue 'grow;
                    }
                }
            }
            break;
        }
        for &t in &cav {
            self.mark[t] = false;
        }
        ok.then_some(cav)
    }

    /// Boundary edges of a cavity: (a, b, outer neighbour, segment, face).
    pub fn cavity_boundary(&mut self, cav: &[usize]) -> Vec<(usize, usize, usize, usize, usize)> {
        for &t in cav {
            self.mark[t] = true;
        }
        let mut out = Vec::new();
        for &t in cav {
            for k in 0..3 {
                let n = self.tn[t][k];
                if n == NONE || !self.mark[n] {
                    let (a, b) = self.edge(t, k);
                    out.push((a, b, n, self.ts[t][k], self.face[t]));
                }
            }
        }
        for &t in cav {
            self.mark[t] = false;
        }
        out
    }

    fn alloc(&mut self) -> usize {
        if let Some(t) = self.free.pop() {
            self.alive[t] = true;
            t
        } else {
            self.tv.push([NONE; 3]);
            self.tn.push([NONE; 3]);
            self.ts.push([NONE; 3]);
            self.face.push(UNKNOWN);
            self.alive.push(true);
            self.mark.push(false);
            self.tv.len() - 1
        }
    }

    /// Replaces the cavity by a fan around the new vertex p. With `split =
    /// Some((s, sm))` segment s (whose two sides are both in the cavity) is
    /// cut at p into s = (a, p) and a new segment (p, b). Returns the new
    /// vertex and triangles, or None if the cavity is not a simple polygon
    /// free of inner vertices.
    pub fn commit(
        &mut self,
        p: [f64; 2],
        cav: &[usize],
        split: Option<(usize, f64)>,
        edge: usize,
    ) -> Option<(usize, Vec<usize>)> {
        let mut bnd = self.cavity_boundary(cav);
        if bnd.len() != cav.len() + 2 {
            return None;
        }
        let mut starts: Vec<usize> = bnd.iter().map(|e| e.0).collect();
        starts.sort_unstable();
        if starts.windows(2).any(|w| w[0] == w[1]) {
            return None;
        }
        if bnd
            .iter()
            .any(|&(a, b, ..)| orient(self.p(a), self.p(b), p) <= 0.)
        {
            return None;
        }
        // (a, s, b, new segment id) of a split.
        let cut = match split {
            Some((s, _)) => {
                let seg = self.segs[s];
                if !starts.contains(&seg.a) || !starts.contains(&seg.b) {
                    return None;
                }
                Some((seg.a, s, seg.b, self.segs.len()))
            }
            None => None,
        };
        let v = self.pts.len();
        self.pts.push(p);
        self.vedge.push(edge);
        self.vcorner.push(false);
        self.vtri.push(NONE);
        if let (Some((s, sm)), Some(_)) = (split, cut) {
            let seg = self.segs[s];
            self.segs[s] = Seg {
                b: v,
                s1: sm,
                ..seg
            };
            self.segs.push(Seg {
                a: v,
                s0: sm,
                ..seg
            });
        }
        for &t in cav.iter().rev() {
            self.alive[t] = false;
            self.free.push(t);
        }
        let mut new = Vec::with_capacity(bnd.len());
        let mut by_start: Vec<(usize, usize)> = Vec::with_capacity(bnd.len());
        for e in &mut bnd {
            let (a, b, n, s, f) = *e;
            let t = self.alloc();
            self.tv[t] = [a, b, v];
            self.tn[t] = [NONE, NONE, n];
            self.ts[t] = [NONE, NONE, s];
            self.face[t] = f;
            if n != NONE {
                let j = self.twin_index(n, a, b);
                self.tn[n][j] = t;
            }
            self.vtri[a] = t;
            new.push(t);
            by_start.push((a, t));
        }
        by_start.sort_unstable();
        let seg_of = |x: usize| match cut {
            Some((a, s, _, _)) if x == a => s,
            Some((_, _, b, s2)) if x == b => s2,
            _ => NONE,
        };
        for &t in &new {
            let b = self.tv[t][1];
            // Edge (b, v) is opposite vertex 0; its twin starts at b.
            let i = by_start.binary_search_by_key(&b, |e| e.0).ok()?;
            let u = by_start[i].1;
            self.tn[t][0] = u;
            self.tn[u][1] = t;
            let s = seg_of(b);
            self.ts[t][0] = s;
            self.ts[u][1] = s;
        }
        self.vtri[v] = new[0];
        self.last = new[0];
        Some((v, new))
    }

    /// Inserts a free point (no segment split). None if it cannot be placed.
    pub fn insert_free(
        &mut self,
        p: [f64; 2],
        hint: usize,
        edge: usize,
    ) -> Option<(usize, Vec<usize>)> {
        let t = self.locate(p, hint)?;
        let cav = self.cavity(p, &[t])?;
        self.commit(p, &cav, None, edge)
    }

    /// Splits segment s at fraction sm, placing the new vertex at p.
    pub fn split_segment(&mut self, s: usize, sm: f64, p: [f64; 2]) -> Option<(usize, Vec<usize>)> {
        let seg = self.segs[s];
        let (t1, _) = self.find_edge(seg.a, seg.b)?;
        let (t2, _) = self.find_edge(seg.b, seg.a)?;
        let cav = self.cavity(p, &[t1, t2])?;
        self.commit(p, &cav, Some((s, sm)), seg.edge)
    }

    /// Walks the straight line from the centroid of t towards p.
    pub fn trace(&self, t: usize, p: [f64; 2]) -> Trace {
        let [a, b, c] = self.tv[t].map(|v| self.p(v));
        let g = [(a[0] + b[0] + c[0]) / 3., (a[1] + b[1] + c[1]) / 3.];
        let mut cur = t;
        for _ in 0..self.tv.len() + 8 {
            if self.contains(cur, p) {
                return Trace::Inside(cur);
            }
            let exit = (0..3).find(|&k| {
                let (u, v) = self.edge(cur, k);
                let (pu, pv) = (self.p(u), self.p(v));
                orient(pu, pv, p) < 0. && orient(g, p, pu) <= 0. && orient(g, p, pv) >= 0.
            });
            let Some(k) = exit else { return Trace::Failed };
            if self.ts[cur][k] != NONE {
                return Trace::Blocked(self.ts[cur][k]);
            }
            let n = self.tn[cur][k];
            if n == NONE {
                return Trace::Failed;
            }
            cur = n;
        }
        Trace::Failed
    }
}
