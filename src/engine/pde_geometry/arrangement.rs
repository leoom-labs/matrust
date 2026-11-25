//! Planar arrangement of shape boundaries for decsg.
//!
//! 1. Every shape boundary becomes pieces: polygon sides, and four quarter
//!    arcs per circle or ellipse (own parameter 0, pi/2, pi, 3 pi/2).
//! 2. Pieces of different shapes are intersected (lines analytically,
//!    line-conic by a quadratic, circle-circle analytically, other conic pairs
//!    by a quartic in tan(t/2) along one arc, polished by Newton). End points lying on
//!    another piece (T-junctions, shared and overlapping sides, coincident
//!    arcs) split that piece too. All points snap to shared vertices within
//!    1e-10 of the geometry size.
//! 3. Pieces are cut at their split vertices; identical sub-edges (same end
//!    vertices, same carrier class and midpoint) are merged, so coincident
//!    overlapping pieces become one edge.
//! 4. Faces come from the half-edge structure (outgoing edges sorted by
//!    tangent angle, ties by curvature). Counter-clockwise cycles bound faces;
//!    clockwise cycles are holes, assigned to the smallest enclosing face of
//!    another connected component (exact winding numbers with arcs).
//! 5. Each bounded face gets an interior sample point: from the midpoint of a
//!    boundary edge, a ray along the inward normal to the nearest other
//!    boundary; the point is halfway. The shapes are tested at that point.
use super::{Conic, GEdge, GKind, Shape, quarter_points};
use crate::Result;
use std::collections::HashMap;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

pub(in crate::engine) fn orient(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    robust::orient2d(
        robust::Coord { x: a[0], y: a[1] },
        robust::Coord { x: b[0], y: b[1] },
        robust::Coord { x: c[0], y: c[1] },
    )
}

/// Closed segments ab and cd share at least one point (exact predicates).
pub(super) fn segments_touch(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let (o1, o2, o3, o4) = (
        orient(a, b, c),
        orient(a, b, d),
        orient(c, d, a),
        orient(c, d, b),
    );
    let within = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| {
        r[0] >= p[0].min(q[0])
            && r[0] <= p[0].max(q[0])
            && r[1] >= p[1].min(q[1])
            && r[1] <= p[1].max(q[1])
    };
    if o1 * o2 < 0. && o3 * o4 < 0. {
        return true;
    }
    (o1 == 0. && within(a, b, c))
        || (o2 == 0. && within(a, b, d))
        || (o3 == 0. && within(c, d, a))
        || (o4 == 0. && within(c, d, b))
}

/// Result of the decomposition: per bounded face the shapes containing it,
/// and every arrangement edge with the bounded face on each side.
pub(super) struct Faces {
    pub inside: Vec<Vec<bool>>,
    #[allow(clippy::type_complexity)]
    pub edges: Vec<(GKind, [f64; 2], [f64; 2], Option<usize>, Option<usize>)>,
}

#[derive(Clone, Copy)]
struct Piece {
    kind: GKind,
    conic: Option<Conic>,
    shape: usize,
    a: usize,
    b: usize,
    /// Parameter range: [0, 1] for lines, unwrapped angles for arcs.
    t0: f64,
    t1: f64,
    lo: [f64; 2],
    hi: [f64; 2],
}

struct Vertices {
    points: Vec<[f64; 2]>,
    grid: HashMap<(i64, i64), Vec<usize>>,
    tol: f64,
}

impl Vertices {
    fn id(&mut self, p: [f64; 2]) -> usize {
        let cell = self.tol * 4.;
        let key = ((p[0] / cell).floor() as i64, (p[1] / cell).floor() as i64);
        let mut best: Option<(f64, usize)> = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for &i in self
                    .grid
                    .get(&(key.0 + dx, key.1 + dy))
                    .into_iter()
                    .flatten()
                {
                    let q = self.points[i];
                    let d = (q[0] - p[0]).hypot(q[1] - p[1]);
                    if d <= self.tol && best.is_none_or(|(bd, _)| d < bd) {
                        best = Some((d, i));
                    }
                }
            }
        }
        if let Some((_, i)) = best {
            return i;
        }
        self.points.push(p);
        self.grid
            .entry(key)
            .or_default()
            .push(self.points.len() - 1);
        self.points.len() - 1
    }
}

impl Piece {
    /// Position of p along the piece (s for lines, unwrapped angle for arcs).
    fn param(&self, p: [f64; 2], v: &[[f64; 2]]) -> f64 {
        match self.conic {
            None => {
                let (a, b) = (v[self.a], v[self.b]);
                let d = [b[0] - a[0], b[1] - a[1]];
                ((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / (d[0] * d[0] + d[1] * d[1])
            }
            Some(c) => {
                const SLACK: f64 = 1e-6;
                self.t0 + (c.param(p) - self.t0 + SLACK).rem_euclid(TAU) - SLACK
            }
        }
    }

    /// p lies on the piece within tol (distance) and its parameter range.
    fn holds(&self, p: [f64; 2], v: &[[f64; 2]], tol: f64) -> bool {
        if p[0] < self.lo[0] - tol
            || p[0] > self.hi[0] + tol
            || p[1] < self.lo[1] - tol
            || p[1] > self.hi[1] + tol
        {
            return false;
        }
        match self.conic {
            None => {
                let (a, b) = (v[self.a], v[self.b]);
                let len = (b[0] - a[0]).hypot(b[1] - a[1]);
                let dist =
                    ((b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])).abs() / len;
                let s = self.param(p, v);
                dist <= tol && s >= -tol / len && s <= 1. + tol / len
            }
            Some(c) => {
                let slack = tol / c.a.min(c.b);
                let t = self.param(p, v);
                c.distance(p) <= tol && t >= self.t0 - slack && t <= self.t1 + slack
            }
        }
    }
}

fn bbox(points: &[[f64; 2]]) -> ([f64; 2], [f64; 2]) {
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    for p in points {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    (lo, hi)
}

/// Real roots of a s^2 + b s + c with a > 0; a near-zero negative
/// discriminant counts as a tangency.
fn quadratic(a: f64, b: f64, c: f64) -> Vec<f64> {
    let mut disc = b * b - 4. * a * c;
    if disc < 0. {
        if disc < -1e-12 * (b * b + (4. * a * c).abs()) {
            return Vec::new();
        }
        disc = 0.;
    }
    let q = -0.5 * (b + b.signum() * disc.sqrt());
    if q == 0. {
        return vec![0.];
    }
    let (r1, r2) = (q / a, c / q);
    if disc == 0. { vec![r1] } else { vec![r1, r2] }
}

/// Parameters s of the line p + s d crossing the conic.
fn line_conic(p: [f64; 2], d: [f64; 2], c: &Conic) -> Vec<f64> {
    let [u, v] = c.local(p);
    let (du, dv) = (c.cos * d[0] + c.sin * d[1], -c.sin * d[0] + c.cos * d[1]);
    let (a2, b2) = (c.a * c.a, c.b * c.b);
    quadratic(
        du * du / a2 + dv * dv / b2,
        2. * (u * du / a2 + v * dv / b2),
        (u * u) / a2 + (v * v) / b2 - 1.,
    )
}

fn same_conic(p: &Conic, q: &Conic, tol: f64) -> bool {
    if (p.c[0] - q.c[0]).abs() > tol || (p.c[1] - q.c[1]).abs() > tol {
        return false;
    }
    if (p.a - p.b).abs() <= tol && (q.a - q.b).abs() <= tol {
        return (p.a - q.a).abs() <= tol;
    }
    // Same axes up to a half turn, or swapped axes at a quarter turn.
    let axis = (p.cos * q.cos + p.sin * q.sin).abs();
    let size = p.a.max(p.b);
    let a_tol = tol / size;
    ((p.a - q.a).abs() <= tol && (p.b - q.b).abs() <= tol && (axis - 1.).abs() <= a_tol)
        || ((p.a - q.b).abs() <= tol && (p.b - q.a).abs() <= tol && axis <= a_tol)
}

fn circle_circle(p: &Conic, q: &Conic) -> Vec<[f64; 2]> {
    let (dx, dy) = (q.c[0] - p.c[0], q.c[1] - p.c[1]);
    let d = dx.hypot(dy);
    if d == 0. {
        return Vec::new();
    }
    let (r1, r2) = (p.a, q.a);
    let slack = 1e-12 * (r1 + r2);
    if d > r1 + r2 + slack || d < (r1 - r2).abs() - slack {
        return Vec::new();
    }
    let a = (d * d + r1 * r1 - r2 * r2) / (2. * d);
    let h = (r1 * r1 - a * a).max(0.).sqrt();
    let (ex, ey) = (dx / d, dy / d);
    let base = [p.c[0] + a * ex, p.c[1] + a * ey];
    vec![
        [base[0] - h * ey, base[1] + h * ex],
        [base[0] + h * ey, base[1] - h * ex],
    ]
}

/// The other conic's implicit value along the arc of `p`, as the trig
/// polynomial k0 + kc cos t + ks sin t + kcc cos^2 t + kss sin^2 t + kcs cos t sin t.
#[derive(Clone, Copy)]
struct TrigQuad {
    k0: f64,
    kc: f64,
    ks: f64,
    kcc: f64,
    kss: f64,
    kcs: f64,
}

impl TrigQuad {
    fn new(p: &Conic, q: &Conic) -> Self {
        // p's point in q's own axes: u = u0 + A1 cos t + B1 sin t, v = v0 + A2 cos t + B2 sin t.
        let [u0, v0] = q.local(p.c);
        let (cr, sr) = (p.cos * q.cos + p.sin * q.sin, p.sin * q.cos - p.cos * q.sin);
        let (a1, b1, a2, b2) = (cr * p.a, -sr * p.b, sr * p.a, cr * p.b);
        let (wa, wb) = (1. / (q.a * q.a), 1. / (q.b * q.b));
        Self {
            k0: wa * u0 * u0 + wb * v0 * v0 - 1.,
            kc: 2. * (wa * u0 * a1 + wb * v0 * a2),
            ks: 2. * (wa * u0 * b1 + wb * v0 * b2),
            kcc: wa * a1 * a1 + wb * a2 * a2,
            kss: wa * b1 * b1 + wb * b2 * b2,
            kcs: 2. * (wa * a1 * b1 + wb * a2 * b2),
        }
    }
    /// Quartic in x = tan(t/2), descending powers, from multiplying by (1 + x^2)^2.
    fn quartic(&self) -> [f64; 5] {
        let k = self;
        [
            k.k0 - k.kc + k.kcc,
            2. * (k.ks - k.kcs),
            2. * (k.k0 - k.kcc) + 4. * k.kss,
            2. * (k.ks + k.kcs),
            k.k0 + k.kc + k.kcc,
        ]
    }
}

/// Direct evaluation of F(t) = q.implicit(p.point(t)) and its first two
/// derivatives. Computing q.local(p.point(t)) before squaring avoids the
/// (D/r)^2 cancellation of the expanded trig polynomial when q is small and
/// far from p's centre.
fn eval_implicit(p: &Conic, q: &Conic, t: f64) -> [f64; 3] {
    let (s, c) = t.sin_cos();
    let (lx, ly) = (p.a * c, p.b * s);
    let (rx, ry) = (p.cos * lx - p.sin * ly, p.sin * lx + p.cos * ly);
    let [u, v] = q.local([p.c[0] + rx, p.c[1] + ry]);

    let (dlx, dly) = (-p.a * s, p.b * c);
    let (dx, dy) = (p.cos * dlx - p.sin * dly, p.sin * dlx + p.cos * dly);
    let (du, dv) = (q.cos * dx + q.sin * dy, -q.sin * dx + q.cos * dy);

    // d^2 p / dt^2 = -(rx, ry).
    let (d2u, d2v) = (-(q.cos * rx + q.sin * ry), -(-q.sin * rx + q.cos * ry));

    let (wa, wb) = (1. / (q.a * q.a), 1. / (q.b * q.b));
    let f = (u / q.a).powi(2) + (v / q.b).powi(2) - 1.;
    let df = 2. * (u * du * wa + v * dv * wb);
    let ddf = 2. * ((du * du + u * d2u) * wa + (dv * dv + v * d2v) * wb);
    [f, df, ddf]
}

/// Newton steps for a zero of F (which = 0) or of F' (which = 1); stops on a flat spot.
fn newton(p: &Conic, q: &Conic, mut t: f64, which: usize, steps: usize) -> f64 {
    for _ in 0..steps {
        let d = eval_implicit(p, q, t);
        let (g, dg) = (d[which], d[which + 1]);
        if dg == 0. || !(g / dg).is_finite() {
            break;
        }
        // A step this large means a flat spot; the root estimate is better.
        let step = g / dg;
        if step.abs() > 0.1 {
            break;
        }
        t -= step;
    }
    t
}

/// Roots (re, im) of a polynomial (descending powers, leading term nonzero),
/// from the eigenvalues of its companion matrix. The QR iteration is bounded
/// (nalgebra's unbounded one cycles when all roots share one modulus, e.g.
/// 15x^4 - 18x^2 + 15); on failure the polynomial is Taylor-shifted and retried.
fn poly_roots(p: &[f64]) -> Option<Vec<(f64, f64)>> {
    let n = p.len() - 1;
    if n == 0 {
        return Some(Vec::new());
    }
    for h in [0., 0.5, -0.75, 1.25] {
        // Taylor shift q(y) = p(y + h) by repeated synthetic division.
        let mut q = p.to_vec();
        for i in 0..n {
            for j in 1..=n - i {
                q[j] += h * q[j - 1];
            }
        }
        let mut m = nalgebra::DMatrix::<f64>::zeros(n, n);
        for j in 0..n {
            m[(0, j)] = -q[j + 1] / q[0];
        }
        for j in 1..n {
            m[(j, j - 1)] = 1.;
        }
        if let Some(schur) = nalgebra::Schur::try_new(m, f64::EPSILON, 200) {
            let roots = schur.complex_eigenvalues();
            return Some(roots.iter().map(|z| (z.re + h, z.im)).collect());
        }
    }
    None
}

/// Parameters in [t0, t1] where the arc of `p` meets the conic `q` (exact:
/// the implicit equation of `q` along `p` is a quartic in tan(t/2)).
/// Tangent (double) roots come back as one parameter.
fn conic_conic(p: &Conic, t0: f64, t1: f64, q: &Conic) -> Result<Vec<f64>> {
    let f = TrigQuad::new(p, q);
    let mut poly = f.quartic();
    let scale = poly.iter().fold(0f64, |m, c| m.max(c.abs()));
    if scale == 0. || !scale.is_finite() {
        // Coincident conics (or degenerate input): no isolated crossings.
        return Ok(Vec::new());
    }
    for c in &mut poly {
        *c /= scale;
    }
    // Candidate parameters with a flag: from a nearly real complex pair.
    let mut cand: Vec<(f64, bool)> = Vec::new();
    let lead = poly.iter().position(|c| c.abs() > 1e-13).unwrap_or(4);
    if lead > 0 {
        // The x^4 term vanishes: t = pi is a root (x = tan(t/2) is infinite).
        // When lead > 1, F'(pi) also vanishes (tangency or near-tangency at pi).
        cand.push((PI, lead > 1));
    }
    let roots = poly_roots(&poly[lead..]).ok_or("decsg: curve intersection did not converge")?;
    for (re, im) in roots {
        if im.abs() <= 1e-6 * (1. + re.abs()) {
            cand.push((2. * re.atan(), im != 0.));
        }
    }
    // Group close candidates (a tangency or two crossings very near each other).
    const NEAR: f64 = 1e-4;
    let wrap = |d: f64| {
        let d = d.rem_euclid(TAU);
        if d > PI { d - TAU } else { d }
    };
    let mut groups: Vec<(f64, usize, bool)> = Vec::new();
    for (t, complex) in cand {
        match groups
            .iter_mut()
            .find(|g| wrap(t - g.0 / g.1 as f64).abs() < NEAR)
        {
            Some(g) => {
                let base = g.0 / g.1 as f64;
                g.0 += base + wrap(t - base);
                g.1 += 1;
                g.2 |= complex;
            }
            None => groups.push((t, 1, complex)),
        }
    }
    let mut roots = Vec::new();
    let coord = p.c[0]
        .abs()
        .max(p.c[1].abs())
        .max(q.c[0].abs())
        .max(q.c[1].abs())
        + p.a.max(p.b);
    // Rounding floor of F(t) = q.implicit(p.point(t)): below this level, fm at
    // an extremum is floating-point noise at an exact tangency and must not
    // split into two crossings ~sqrt(eps) apart.
    let f_noise = 32. * f64::EPSILON * (1. + coord / q.a.min(q.b));
    for (sum, count, complex) in groups {
        let mean = sum / count as f64;
        if count == 1 && !complex {
            roots.push(newton(p, q, mean, 0, 2));
            continue;
        }
        // Near a double root: F ~ Fm + F'' (t - tm)^2 / 2 around the extremum tm.
        let tm = newton(p, q, mean, 1, 2);
        let [fm, _, ddf] = eval_implicit(p, q, tm);
        let delta2 = if ddf == 0. || fm.abs() <= f_noise {
            -1.
        } else {
            -2. * fm / ddf
        };
        if delta2 > 0. {
            let delta = delta2.sqrt();
            roots.push(newton(p, q, tm - delta, 0, 2));
            roots.push(newton(p, q, tm + delta, 0, 2));
        } else {
            // Touch point; kept only if it lies on q within the snapping tolerance.
            roots.push(tm);
        }
    }
    const SLACK: f64 = 1e-7;
    Ok(roots
        .into_iter()
        .map(|t| t0 + (t - t0 + SLACK).rem_euclid(TAU) - SLACK)
        .filter(|&t| t <= t1 + SLACK)
        .collect())
}

fn build_pieces(shapes: &[Shape], verts: &mut Vertices) -> Vec<Piece> {
    let mut pieces = Vec::new();
    for (shape, s) in shapes.iter().enumerate() {
        match s {
            Shape::Polygon { vertices } => {
                let n = vertices.len();
                let ids: Vec<usize> = vertices.iter().map(|&p| verts.id(p)).collect();
                for i in 0..n {
                    let (lo, hi) = bbox(&[vertices[i], vertices[(i + 1) % n]]);
                    pieces.push(Piece {
                        kind: GKind::Line,
                        conic: None,
                        shape,
                        a: ids[i],
                        b: ids[(i + 1) % n],
                        t0: 0.,
                        t1: 1.,
                        lo,
                        hi,
                    });
                }
            }
            Shape::Circle { c, r } => {
                let conic = Conic::circle(*c, *r);
                quarters(
                    &mut pieces,
                    GKind::Arc { centre: *c, r: *r },
                    conic,
                    shape,
                    verts,
                );
            }
            Shape::Ellipse { c, a, b, phi } => {
                let conic = Conic::ellipse(*c, *a, *b, *phi);
                let kind = GKind::EllipseArc {
                    centre: *c,
                    a: *a,
                    b: *b,
                    phi: *phi,
                };
                quarters(&mut pieces, kind, conic, shape, verts);
            }
        }
    }
    pieces
}

fn quarters(
    pieces: &mut Vec<Piece>,
    kind: GKind,
    conic: Conic,
    shape: usize,
    verts: &mut Vertices,
) {
    let q = quarter_points(&conic);
    let ids: Vec<usize> = q.iter().map(|&p| verts.id(p)).collect();
    for k in 0..4 {
        let t0 = k as f64 * FRAC_PI_2;
        let t1 = t0 + FRAC_PI_2;
        let mut pts = vec![q[k], q[(k + 1) % 4]];
        // Exact box: end points plus the x and y extremes inside the arc.
        let (a, b) = (conic.a, conic.b);
        let tx = (-b * conic.sin).atan2(a * conic.cos);
        let ty = (b * conic.cos).atan2(a * conic.sin);
        for t in [tx, tx + PI, ty, ty + PI] {
            if t0 + (t - t0).rem_euclid(TAU) <= t1 {
                pts.push(conic.point(t));
            }
        }
        let (mut lo, mut hi) = bbox(&pts);
        // Rounding in point() only: a few ulps of the coordinate size.
        let pad = 8. * f64::EPSILON * (conic.c[0].abs().max(conic.c[1].abs()) + a.max(b));
        lo = [lo[0] - pad, lo[1] - pad];
        hi = [hi[0] + pad, hi[1] + pad];
        pieces.push(Piece {
            kind,
            conic: Some(conic),
            shape,
            a: ids[k],
            b: ids[(k + 1) % 4],
            t0,
            t1,
            lo,
            hi,
        });
    }
}

/// Crossing points of two pieces of different shapes (end points on the
/// other piece are handled separately).
fn crossings(p: &Piece, q: &Piece, v: &[[f64; 2]], tol: f64) -> Result<Vec<[f64; 2]>> {
    Ok(match (p.conic, q.conic) {
        (None, None) => {
            let (a, b, c, d) = (v[p.a], v[p.b], v[q.a], v[q.b]);
            let (o1, o2, o3, o4) = (
                orient(a, b, c),
                orient(a, b, d),
                orient(c, d, a),
                orient(c, d, b),
            );
            if o1 * o2 < 0. && o3 * o4 < 0. {
                let s = o3 / (o3 - o4);
                vec![[a[0] + s * (b[0] - a[0]), a[1] + s * (b[1] - a[1])]]
            } else {
                Vec::new()
            }
        }
        (None, Some(cq)) | (Some(cq), None) => {
            let line = if p.conic.is_none() { p } else { q };
            let (a, b) = (v[line.a], v[line.b]);
            let d = [b[0] - a[0], b[1] - a[1]];
            line_conic(a, d, &cq)
                .into_iter()
                .map(|s| [a[0] + s * d[0], a[1] + s * d[1]])
                .collect()
        }
        (Some(cp), Some(cq)) => {
            if same_conic(&cp, &cq, tol) {
                Vec::new()
            } else if cp.a == cp.b && cq.a == cq.b {
                circle_circle(&cp, &cq)
            } else {
                // Parameterize along the smaller conic so small circles on a
                // large ellipse have well-separated roots and no (D/r)^2
                // cancellation in the quartic, and both column orders follow
                // the same path.
                let swap = cq.a.max(cq.b) < cp.a.max(cp.b)
                    || (cq.a.max(cq.b) == cp.a.max(cp.b) && cq.a.min(cq.b) < cp.a.min(cp.b));
                if swap {
                    conic_conic(&cq, q.t0, q.t1, &cp)?
                        .into_iter()
                        .map(|t| cq.point(t))
                        .collect()
                } else {
                    conic_conic(&cp, p.t0, p.t1, &cq)?
                        .into_iter()
                        .map(|t| cp.point(t))
                        .collect()
                }
            }
        }
    }
    .into_iter()
    .filter(|&x| p.holds(x, v, tol) && q.holds(x, v, tol))
    .collect())
}

/// One arrangement edge between vertices a and b.
struct Edge {
    g: GEdge,
    a: usize,
    b: usize,
}

fn split_pieces(
    pieces: &[Piece],
    splits: &[Vec<usize>],
    v: &[[f64; 2]],
    tol: f64,
) -> Result<Vec<Edge>> {
    let mut edges: Vec<Edge> = Vec::new();
    let mut by_pair: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for (piece, list) in pieces.iter().zip(splits) {
        let mut inner: Vec<(f64, usize)> = list
            .iter()
            .filter(|&&id| id != piece.a && id != piece.b)
            .map(|&id| (piece.param(v[id], v), id))
            .collect();
        inner.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
        inner.dedup_by_key(|x| x.1);
        let mut chain = vec![piece.a];
        chain.extend(inner.iter().map(|x| x.1));
        chain.push(piece.b);
        for w in chain.windows(2) {
            let (a, b) = (w[0], w[1]);
            if a == b {
                continue;
            }
            let g = GEdge::new(piece.kind, v[a], v[b], 0, 0);
            if piece.conic.is_some() && g.t1 - g.t0 > PI {
                return Err("decsg: boundaries are too close together to resolve".into());
            }
            let mid = g.point_at(0.5);
            let key = (a.min(b), a.max(b));
            let duplicate = by_pair.get(&key).is_some_and(|list| {
                list.iter().any(|&k| {
                    let other = &edges[k].g;
                    let line = matches!(other.kind, GKind::Line);
                    if line != matches!(g.kind, GKind::Line) {
                        return false;
                    }
                    let m = other.point_at(0.5);
                    line || (m[0] - mid[0]).hypot(m[1] - mid[1]) <= 100. * tol
                })
            });
            if !duplicate {
                by_pair.entry(key).or_default().push(edges.len());
                edges.push(Edge { g, a, b });
            }
        }
    }
    Ok(edges)
}

/// Direction and signed curvature of half-edge h where it leaves its origin.
fn leaving(e: &Edge, forward: bool) -> (f64, f64) {
    let (dir, curvature) = match e.g.kind.conic() {
        None => (e.g.tangent_at(0.), 0.),
        Some(c) => {
            if forward {
                (c.deriv(e.g.t0), c.curvature(e.g.t0))
            } else {
                let d = c.deriv(e.g.t1);
                ([-d[0], -d[1]], -c.curvature(e.g.t1))
            }
        }
    };
    let dir = if forward || e.g.kind.conic().is_some() {
        dir
    } else {
        [-dir[0], -dir[1]]
    };
    let mut angle = dir[1].atan2(dir[0]);
    if angle <= -PI + 1e-12 {
        angle += TAU;
    }
    (angle, curvature)
}

/// Winding number of a closed half-edge cycle around q (q not on it).
fn winding(edges: &[Edge], cycle: &[usize], q: [f64; 2]) -> i64 {
    let mut total = 0.;
    for &h in cycle {
        let e = &edges[h / 2].g;
        let forward = h % 2 == 0;
        let parts = match e.kind {
            GKind::Line => 1,
            _ => ((e.t1 - e.t0) / FRAC_PI_2).ceil().max(1.) as usize,
        };
        for k in 0..parts {
            let (s0, s1) = (k as f64 / parts as f64, (k + 1) as f64 / parts as f64);
            let (mut p0, mut p1) = (e.point_at(s0), e.point_at(s1));
            if !forward {
                std::mem::swap(&mut p0, &mut p1);
            }
            let (a, b) = ([p0[0] - q[0], p0[1] - q[1]], [p1[0] - q[0], p1[1] - q[1]]);
            total += (a[0] * b[1] - a[1] * b[0]).atan2(a[0] * b[0] + a[1] * b[1]);
            if let Some(c) = e.kind.conic()
                && c.implicit(q) < 0.
            {
                let o = orient(p0, p1, q);
                if forward && o < 0. {
                    total += TAU;
                } else if !forward && o > 0. {
                    total -= TAU;
                }
            }
        }
    }
    (total / TAU).round() as i64
}

/// Distance along the ray m + s n (s > 0) to the nearest edge.
fn ray_hit(edges: &[Edge], m: [f64; 2], n: [f64; 2], skip: usize, tol: f64) -> f64 {
    let mut best = f64::INFINITY;
    for (k, e) in edges.iter().enumerate() {
        let g = &e.g;
        match g.kind.conic() {
            None => {
                if k == skip {
                    continue;
                }
                let (a, b) = (g.start, g.end);
                let d = [b[0] - a[0], b[1] - a[1]];
                let w = [a[0] - m[0], a[1] - m[1]];
                let denom = n[0] * d[1] - n[1] * d[0];
                let len = d[0].hypot(d[1]);
                if denom.abs() <= 1e-14 * len {
                    if (w[0] * n[1] - w[1] * n[0]).abs() <= tol {
                        for p in [a, b] {
                            let s = (p[0] - m[0]) * n[0] + (p[1] - m[1]) * n[1];
                            if s > 0. {
                                best = best.min(s);
                            }
                        }
                    }
                    continue;
                }
                let s = (w[0] * d[1] - w[1] * d[0]) / denom;
                let u = (w[0] * n[1] - w[1] * n[0]) / denom;
                if s > 0. && (-1e-12..=1. + 1e-12).contains(&u) {
                    best = best.min(s);
                }
            }
            Some(c) => {
                for s in line_conic(m, n, &c) {
                    let floor = if k == skip { 1e3 * tol } else { 0. };
                    if s <= floor || s >= best {
                        continue;
                    }
                    let p = [m[0] + s * n[0], m[1] + s * n[1]];
                    let dt = (c.param(p) - g.t0).rem_euclid(TAU);
                    if dt <= g.t1 - g.t0 + 1e-12 || dt >= TAU - 1e-12 {
                        best = s;
                    }
                }
            }
        }
    }
    best
}

pub(super) fn decompose(shapes: &[Shape], cancel: &dyn Fn() -> Result<()>) -> Result<Faces> {
    // Geometry size sets the snapping tolerance.
    let mut extent = 0f64;
    for s in shapes {
        match s {
            Shape::Polygon { vertices } => {
                for p in vertices {
                    extent = extent.max(p[0].abs()).max(p[1].abs());
                }
            }
            Shape::Circle { c, r } => extent = extent.max(c[0].abs() + r).max(c[1].abs() + r),
            Shape::Ellipse { c, a, b, .. } => {
                let r = a.max(*b);
                extent = extent.max(c[0].abs() + r).max(c[1].abs() + r);
            }
        }
    }
    let tol = 1e-10 * extent.max(f64::MIN_POSITIVE);
    let mut verts = Vertices {
        points: Vec::new(),
        grid: HashMap::new(),
        tol,
    };
    let pieces = build_pieces(shapes, &mut verts);
    let mut splits: Vec<Vec<usize>> = vec![Vec::new(); pieces.len()];
    for i in 0..pieces.len() {
        cancel()?;
        for j in i + 1..pieces.len() {
            let (p, q) = (&pieces[i], &pieces[j]);
            if p.shape == q.shape
                || p.lo[0] > q.hi[0] + tol
                || q.lo[0] > p.hi[0] + tol
                || p.lo[1] > q.hi[1] + tol
                || q.lo[1] > p.hi[1] + tol
            {
                continue;
            }
            for id in [q.a, q.b] {
                if p.holds(verts.points[id], &verts.points, tol) {
                    splits[i].push(id);
                }
            }
            for id in [p.a, p.b] {
                if q.holds(verts.points[id], &verts.points, tol) {
                    splits[j].push(id);
                }
            }
            for x in crossings(p, q, &verts.points, tol)? {
                let id = verts.id(x);
                splits[i].push(id);
                splits[j].push(id);
            }
        }
    }
    let v = verts.points;
    let edges = split_pieces(&pieces, &splits, &v, tol)?;
    cancel()?;

    // Half-edges 2e (a -> b) and 2e + 1 (b -> a); outgoing lists per vertex.
    let nh = 2 * edges.len();
    let origin = |h: usize| {
        if h.is_multiple_of(2) {
            edges[h / 2].a
        } else {
            edges[h / 2].b
        }
    };
    let mut out: Vec<Vec<(f64, f64, usize)>> = vec![Vec::new(); v.len()];
    for h in 0..nh {
        let (angle, curvature) = leaving(&edges[h / 2], h % 2 == 0);
        out[origin(h)].push((angle, curvature, h));
    }
    let mut position = vec![0; nh];
    for list in &mut out {
        list.sort_by(|x, y| {
            if (x.0 - y.0).abs() > 1e-10 {
                x.0.total_cmp(&y.0)
            } else {
                x.1.total_cmp(&y.1).then(x.2.cmp(&y.2))
            }
        });
        for (i, item) in list.iter().enumerate() {
            position[item.2] = i;
        }
    }
    let next = |h: usize| {
        let twin = h ^ 1;
        let list = &out[origin(twin)];
        list[(position[twin] + list.len() - 1) % list.len()].2
    };
    let mut cycle_of = vec![usize::MAX; nh];
    let mut cycles: Vec<Vec<usize>> = Vec::new();
    for h in 0..nh {
        if cycle_of[h] != usize::MAX {
            continue;
        }
        let mut cycle = Vec::new();
        let mut x = h;
        while cycle_of[x] == usize::MAX {
            cycle_of[x] = cycles.len();
            cycle.push(x);
            x = next(x);
            if cycle.len() > nh {
                return Err("decsg: internal error: face traversal did not close".into());
            }
        }
        cycles.push(cycle);
    }
    let area: Vec<f64> = cycles
        .iter()
        .map(|c| {
            c.iter()
                .map(|&h| {
                    let t = edges[h / 2].g.area_term();
                    if h % 2 == 0 { t } else { -t }
                })
                .sum()
        })
        .collect();
    cancel()?;

    // Connected components of the edge graph.
    let mut parent: Vec<usize> = (0..v.len()).collect();
    fn root(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    for e in &edges {
        let (a, b) = (root(&mut parent, e.a), root(&mut parent, e.b));
        parent[a] = b;
    }
    let component: Vec<usize> = cycles
        .iter()
        .map(|c| root(&mut parent, origin(c[0])))
        .collect();

    // Bounded faces = counter-clockwise cycles; holes join the smallest
    // enclosing face of another component.
    let mut face_of_cycle: Vec<Option<usize>> = vec![None; cycles.len()];
    let mut outer = Vec::new();
    for (c, &a) in area.iter().enumerate() {
        if a > 0. {
            face_of_cycle[c] = Some(outer.len());
            outer.push(c);
        }
    }
    for c in 0..cycles.len() {
        if area[c] > 0. {
            continue;
        }
        let q = v[origin(cycles[c][0])];
        let mut best: Option<(f64, usize)> = None;
        for (f, &p) in outer.iter().enumerate() {
            if component[p] != component[c]
                && best.is_none_or(|(a, _)| area[p] < a)
                && winding(&edges, &cycles[p], q) != 0
            {
                best = Some((area[p], f));
            }
        }
        face_of_cycle[c] = best.map(|b| b.1);
    }
    cancel()?;

    let mut inside = Vec::with_capacity(outer.len());
    for &c in &outer {
        let cycle = &cycles[c];
        let tries = cycle.len().min(4);
        let mut best: Option<(f64, [f64; 2], [f64; 2])> = None;
        for k in 0..tries {
            let h = cycle[k * cycle.len() / tries];
            let g = &edges[h / 2].g;
            let m = g.point_at(0.5);
            let mut t = g.tangent_at(0.5);
            if h % 2 == 1 {
                t = [-t[0], -t[1]];
            }
            let len = t[0].hypot(t[1]);
            let n = [-t[1] / len, t[0] / len];
            let d = ray_hit(&edges, m, n, h / 2, tol);
            if best.is_none_or(|b| d > b.0) {
                best = Some((d, m, n));
            }
        }
        let (d, m, n) = best.ok_or("decsg: internal error: empty face boundary")?;
        let d = if d.is_finite() { d } else { extent };
        let p = [m[0] + n[0] * d / 2., m[1] + n[1] * d / 2.];
        inside.push(shapes.iter().map(|s| s.contains(p)).collect());
    }
    let face = |h: usize| face_of_cycle[cycle_of[h]];
    let edges = edges
        .iter()
        .enumerate()
        .map(|(k, e)| (e.g.kind, e.g.start, e.g.end, face(2 * k), face(2 * k + 1)))
        .collect();
    Ok(Faces { inside, edges })
}
