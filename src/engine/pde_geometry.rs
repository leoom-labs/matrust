//! Constructive solid geometry for 2-D PDE domains: `decsg` and `csgdel`.
//! Matrix formats follow the public decsg/csgdel documentation: the geometry
//! description matrix gd (one shape per column), the set formula sf, the name
//! space ns (one name per column), the decomposed geometry matrix dl (one
//! boundary segment per column) and the Boolean table bt (region x shape).
//! The planar arrangement of the shape boundaries is built in
//! `arrangement.rs`; this file holds the shared geometry type, the dl
//! conversion and the builtins.
mod arrangement;

use super::{Engine, Execution, Value};
use crate::{Array, LogicalMatrix, Matrix, Result};
use std::f64::consts::TAU;

/// Shapes accepted in one gd matrix.
const MAX_SHAPES: usize = 512;
/// Boundary pieces (polygon sides plus four per circle or ellipse).
const MAX_PIECES: usize = 4096;
/// Columns accepted in a dl matrix.
const MAX_DL_COLUMNS: usize = 200_000;
/// Set formula length in characters.
const MAX_FORMULA: usize = 4096;

pub(super) fn handles(name: &str) -> bool {
    matches!(name, "decsg" | "csgdel")
}

pub(super) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
    _out: &mut Execution,
) -> Result<Vec<Value>> {
    if count > 2 {
        return Err(format!("{name}: too many output arguments"));
    }
    let (dl, bt) = match name {
        "decsg" => decsg(e, args)?,
        "csgdel" => csgdel(e, args)?,
        _ => return Err(format!("Unknown geometry function {name}")),
    };
    let mut values = vec![Value::Matrix(dl)];
    if count == 2 {
        values.push(Value::Logical(bt));
    }
    Ok(values)
}

// ---------------------------------------------------------------------------
// Shared geometry type (consumed by geometryFromEdges, pdegplot, the mesher).

/// Carrier curve of one boundary segment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum GKind {
    Line,
    /// Counter-clockwise circular arc.
    Arc {
        centre: [f64; 2],
        r: f64,
    },
    /// Counter-clockwise arc of the ellipse
    /// centre + R(phi) * (a cos t, b sin t).
    EllipseArc {
        centre: [f64; 2],
        a: f64,
        b: f64,
        phi: f64,
    },
}

/// One boundary segment. Lines use t in [0, 1]; arcs use their own angle
/// parameter with `t0` in (-pi, pi] and `t0 < t1 < t0 + 2 pi`, running
/// counter-clockwise from `start` to `end`. `left`/`right` are the face
/// labels on each side of the start-to-end direction (0 = outside).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GEdge {
    pub kind: GKind,
    pub start: [f64; 2],
    pub end: [f64; 2],
    pub t0: f64,
    pub t1: f64,
    pub left: usize,
    pub right: usize,
}

/// Decomposed 2-D geometry: boundary segments with faces 1..=num_faces.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Geometry2 {
    pub edges: Vec<GEdge>,
    pub num_faces: usize,
}

/// Conic frame shared by circles (a = b = r, phi = 0) and ellipses.
#[derive(Clone, Copy, Debug)]
pub(super) struct Conic {
    pub c: [f64; 2],
    pub a: f64,
    pub b: f64,
    pub cos: f64,
    pub sin: f64,
}

impl Conic {
    pub fn circle(c: [f64; 2], r: f64) -> Self {
        Self {
            c,
            a: r,
            b: r,
            cos: 1.,
            sin: 0.,
        }
    }
    pub fn ellipse(c: [f64; 2], a: f64, b: f64, phi: f64) -> Self {
        Self {
            c,
            a,
            b,
            cos: phi.cos(),
            sin: phi.sin(),
        }
    }
    /// Coordinates in the conic's own axes.
    pub fn local(&self, p: [f64; 2]) -> [f64; 2] {
        let (dx, dy) = (p[0] - self.c[0], p[1] - self.c[1]);
        [
            self.cos * dx + self.sin * dy,
            -self.sin * dx + self.cos * dy,
        ]
    }
    #[allow(clippy::wrong_self_convention)]
    pub fn from_local(&self, u: f64, v: f64) -> [f64; 2] {
        [
            self.c[0] + self.cos * u - self.sin * v,
            self.c[1] + self.sin * u + self.cos * v,
        ]
    }
    pub fn point(&self, t: f64) -> [f64; 2] {
        self.from_local(self.a * t.cos(), self.b * t.sin())
    }
    /// d point / dt.
    pub fn deriv(&self, t: f64) -> [f64; 2] {
        let (du, dv) = (-self.a * t.sin(), self.b * t.cos());
        [self.cos * du - self.sin * dv, self.sin * du + self.cos * dv]
    }
    /// Implicit value: negative inside, zero on the curve.
    pub fn implicit(&self, p: [f64; 2]) -> f64 {
        let [u, v] = self.local(p);
        (u / self.a).powi(2) + (v / self.b).powi(2) - 1.
    }
    /// Approximate distance of p from the curve.
    pub fn distance(&self, p: [f64; 2]) -> f64 {
        let [u, v] = self.local(p);
        if self.a == self.b {
            return (u.hypot(v) - self.a).abs();
        }
        let g = (u / self.a).powi(2) + (v / self.b).powi(2) - 1.;
        let grad = (2. * u / (self.a * self.a)).hypot(2. * v / (self.b * self.b));
        if grad == 0. {
            self.a.min(self.b)
        } else {
            g.abs() / grad
        }
    }
    /// Angle parameter of a point on (or near) the curve, in (-pi, pi].
    pub fn param(&self, p: [f64; 2]) -> f64 {
        let [u, v] = self.local(p);
        (v / self.b).atan2(u / self.a)
    }
    /// Signed curvature (positive = turning left) when moving counter-clockwise.
    pub fn curvature(&self, t: f64) -> f64 {
        let (s, c) = t.sin_cos();
        let q = (self.a * s).powi(2) + (self.b * c).powi(2);
        self.a * self.b / q.powf(1.5)
    }
}

/// Canonical arc parameters of the counter-clockwise arc from start to end.
fn arc_range(conic: &Conic, start: [f64; 2], end: [f64; 2]) -> (f64, f64) {
    let t0 = conic.param(start);
    let mut delta = (conic.param(end) - t0).rem_euclid(TAU);
    if delta == 0. {
        delta = TAU;
    }
    (t0, t0 + delta)
}

impl GKind {
    pub(super) fn conic(&self) -> Option<Conic> {
        match *self {
            GKind::Line => None,
            GKind::Arc { centre, r } => Some(Conic::circle(centre, r)),
            GKind::EllipseArc { centre, a, b, phi } => Some(Conic::ellipse(centre, a, b, phi)),
        }
    }
}

impl GEdge {
    /// Builds an edge with canonical parameters derived from its endpoints.
    pub(super) fn new(
        kind: GKind,
        start: [f64; 2],
        end: [f64; 2],
        left: usize,
        right: usize,
    ) -> Self {
        let (t0, t1) = match kind.conic() {
            None => (0., 1.),
            Some(conic) => arc_range(&conic, start, end),
        };
        Self {
            kind,
            start,
            end,
            t0,
            t1,
            left,
            right,
        }
    }

    /// Point at fraction s in [0, 1]: arc-length uniform for lines and
    /// circular arcs, parameter-uniform for ellipse arcs. The ends are exact.
    pub(crate) fn point_at(&self, s: f64) -> [f64; 2] {
        if s <= 0. {
            return self.start;
        }
        if s >= 1. {
            return self.end;
        }
        match self.kind.conic() {
            None => [
                self.start[0] + s * (self.end[0] - self.start[0]),
                self.start[1] + s * (self.end[1] - self.start[1]),
            ],
            Some(conic) => conic.point(self.t0 + s * (self.t1 - self.t0)),
        }
    }

    /// Tangent (d point / d s, not normalised) at fraction s.
    pub(crate) fn tangent_at(&self, s: f64) -> [f64; 2] {
        match self.kind.conic() {
            None => [self.end[0] - self.start[0], self.end[1] - self.start[1]],
            Some(conic) => {
                let d = conic.deriv(self.t0 + s * (self.t1 - self.t0));
                let k = self.t1 - self.t0;
                [d[0] * k, d[1] * k]
            }
        }
    }

    /// Arc length (Gauss-Legendre quadrature for ellipse arcs).
    pub(crate) fn length(&self) -> f64 {
        match self.kind {
            GKind::Line => (self.end[0] - self.start[0]).hypot(self.end[1] - self.start[1]),
            GKind::Arc { r, .. } => r * (self.t1 - self.t0),
            GKind::EllipseArc { a, b, .. } => {
                const X: [f64; 4] = [
                    0.183_434_642_495_649_8,
                    0.525_532_409_916_329,
                    0.796_666_477_413_626_7,
                    0.960_289_856_497_536_3,
                ];
                const W: [f64; 4] = [
                    0.362_683_783_378_362,
                    0.313_706_645_877_887_3,
                    0.222_381_034_453_374_5,
                    0.101_228_536_290_376_3,
                ];
                let pieces = 16;
                let h = (self.t1 - self.t0) / f64::from(pieces);
                let mut total = 0.;
                for k in 0..pieces {
                    let mid = self.t0 + (f64::from(k) + 0.5) * h;
                    for (x, w) in X.iter().zip(W) {
                        for t in [mid - x * h / 2., mid + x * h / 2.] {
                            total += w * (a * t.sin()).hypot(b * t.cos());
                        }
                    }
                }
                total * h / 2.
            }
        }
    }

    /// Integral of (x dy - y dx) / 2 from start to end.
    pub(crate) fn area_term(&self) -> f64 {
        let cross = self.start[0] * self.end[1] - self.end[0] * self.start[1];
        match self.kind {
            GKind::Line => cross / 2.,
            GKind::Arc { centre, r } => conic_area_term(centre, r * r, self),
            GKind::EllipseArc { centre, a, b, .. } => conic_area_term(centre, a * b, self),
        }
    }
}

fn conic_area_term(c: [f64; 2], ab: f64, e: &GEdge) -> f64 {
    (ab * (e.t1 - e.t0) + c[0] * (e.end[1] - e.start[1]) - c[1] * (e.end[0] - e.start[0])) / 2.
}

impl Geometry2 {
    /// Reads and validates a decomposed geometry matrix.
    pub(crate) fn from_dl(dl: &Matrix) -> Result<Self> {
        let (rows, cols) = (dl.rows(), dl.cols());
        if dl.is_nd() || (cols > 0 && rows < 7) {
            return Err(
                "dl must be a matrix with at least 7 rows, one column per boundary segment".into(),
            );
        }
        if cols > MAX_DL_COLUMNS {
            return Err(format!("dl has more than {MAX_DL_COLUMNS} columns"));
        }
        if dl.data().iter().any(|v| !v.is_finite()) {
            return Err("dl must contain finite real values".into());
        }
        let mut edges = Vec::with_capacity(cols);
        let mut num_faces = 0;
        let mut scale = 0f64;
        for j in 0..cols {
            let at = |i: usize| if i < rows { dl.get(i, j) } else { 0. };
            let column = j + 1;
            let start = [at(1), at(3)];
            let end = [at(2), at(4)];
            let label = |v: f64| -> Result<usize> {
                if v < 0. || v.fract() != 0. || v > MAX_DL_COLUMNS as f64 {
                    return Err(format!(
                        "dl column {column}: region labels must be nonnegative integers"
                    ));
                }
                Ok(v as usize)
            };
            let (left, right) = (label(at(5))?, label(at(6))?);
            if left == right {
                return Err(format!(
                    "dl column {column}: the regions on both sides must differ"
                ));
            }
            num_faces = num_faces.max(left).max(right);
            let kind = match at(0) {
                2. => GKind::Line,
                1. => {
                    if rows < 10 {
                        return Err(format!("dl column {column}: circle arcs need rows 8-10"));
                    }
                    GKind::Arc {
                        centre: [at(7), at(8)],
                        r: at(9),
                    }
                }
                4. => {
                    if rows < 12 {
                        return Err(format!("dl column {column}: ellipse arcs need rows 8-12"));
                    }
                    GKind::EllipseArc {
                        centre: [at(7), at(8)],
                        a: at(9),
                        b: at(10),
                        phi: at(11),
                    }
                }
                t => {
                    return Err(format!(
                        "dl column {column}: unknown segment type {t} (1 circle arc, 2 line, 4 ellipse arc)"
                    ));
                }
            };
            match kind {
                GKind::Arc { r, .. } if r <= 0. => {
                    return Err(format!("dl column {column}: radius must be positive"));
                }
                GKind::EllipseArc { a, b, .. } if a <= 0. || b <= 0. => {
                    return Err(format!("dl column {column}: semi-axes must be positive"));
                }
                _ => {}
            }
            if start == end {
                return Err(format!("dl column {column}: start and end points coincide"));
            }
            let edge = GEdge::new(kind, start, end, left, right);
            let len = edge.length();
            if len.is_nan() || len <= 0. {
                return Err(format!("dl column {column}: segment has zero length"));
            }
            if let Some(conic) = kind.conic() {
                let size = conic.a.max(conic.b);
                for p in [start, end] {
                    if conic.distance(p) > 1e-8 * size {
                        return Err(format!(
                            "dl column {column}: arc end points must lie on the curve"
                        ));
                    }
                }
            }
            for p in [start, end] {
                scale = scale.max(p[0].abs()).max(p[1].abs());
            }
            edges.push(edge);
        }
        let geometry = Self { edges, num_faces };
        geometry.check_closed(1e-12 * scale.max(f64::MIN_POSITIVE))?;
        Ok(geometry)
    }

    /// Every face label 1..=num_faces is used and every face boundary closes:
    /// at every boundary point as many segments of the face arrive as leave.
    fn check_closed(&self, tol: f64) -> Result<()> {
        let mut used = vec![false; self.num_faces + 1];
        // Snap end points to shared vertices.
        let mut vertices: Vec<[f64; 2]> = Vec::new();
        let mut grid = std::collections::HashMap::<(i64, i64), Vec<usize>>::new();
        let cell = (tol * 4.).max(f64::MIN_POSITIVE);
        let mut vid = |p: [f64; 2]| -> usize {
            let key = ((p[0] / cell).floor() as i64, (p[1] / cell).floor() as i64);
            for dx in -1..=1 {
                for dy in -1..=1 {
                    if let Some(list) = grid.get(&(key.0 + dx, key.1 + dy)) {
                        for &i in list {
                            let q = vertices[i];
                            if (q[0] - p[0]).abs() <= tol && (q[1] - p[1]).abs() <= tol {
                                return i;
                            }
                        }
                    }
                }
            }
            vertices.push(p);
            grid.entry(key).or_default().push(vertices.len() - 1);
            vertices.len() - 1
        };
        let mut balance = std::collections::HashMap::<(usize, usize), i64>::new();
        for e in &self.edges {
            let (a, b) = (vid(e.start), vid(e.end));
            if a == b {
                return Err("dl: a boundary segment has coincident end points".into());
            }
            for (face, from, to) in [(e.left, a, b), (e.right, b, a)] {
                used[face] = true;
                if face > 0 {
                    *balance.entry((face, from)).or_default() -= 1;
                    *balance.entry((face, to)).or_default() += 1;
                }
            }
        }
        if let Some(face) = (1..=self.num_faces).find(|&f| !used[f]) {
            return Err(format!(
                "dl: region labels must be 1..{} without gaps; region {face} has no boundary",
                self.num_faces
            ));
        }
        let mut open: Vec<_> = balance
            .iter()
            .filter(|(_, v)| **v != 0)
            .map(|(k, _)| k.0)
            .collect();
        open.sort_unstable();
        if let Some(face) = open.first() {
            return Err(format!("dl: the boundary of region {face} is not closed"));
        }
        Ok(())
    }

    /// The decomposed geometry matrix: 7 rows for lines only, 10 with circle
    /// arcs, 12 with ellipse arcs (unused rows zero).
    pub(crate) fn to_dl(&self) -> Result<Matrix> {
        let rows = if self
            .edges
            .iter()
            .any(|e| matches!(e.kind, GKind::EllipseArc { .. }))
        {
            12
        } else if self
            .edges
            .iter()
            .any(|e| matches!(e.kind, GKind::Arc { .. }))
        {
            10
        } else {
            7
        };
        let mut data = vec![0.; rows * self.edges.len()];
        for (j, e) in self.edges.iter().enumerate() {
            let col = &mut data[j * rows..(j + 1) * rows];
            col[1] = e.start[0];
            col[2] = e.end[0];
            col[3] = e.start[1];
            col[4] = e.end[1];
            col[5] = e.left as f64;
            col[6] = e.right as f64;
            match e.kind {
                GKind::Line => col[0] = 2.,
                GKind::Arc { centre, r } => {
                    col[0] = 1.;
                    col[7..10].copy_from_slice(&[centre[0], centre[1], r]);
                }
                GKind::EllipseArc { centre, a, b, phi } => {
                    col[0] = 4.;
                    col[7..12].copy_from_slice(&[centre[0], centre[1], a, b, phi]);
                }
            }
        }
        Matrix::from_column_vec(rows, self.edges.len(), data)
    }
}

// ---------------------------------------------------------------------------
// Input shapes.

#[derive(Clone, Debug)]
pub(super) enum Shape {
    Circle {
        c: [f64; 2],
        r: f64,
    },
    Polygon {
        vertices: Vec<[f64; 2]>,
    },
    Ellipse {
        c: [f64; 2],
        a: f64,
        b: f64,
        phi: f64,
    },
}

impl Shape {
    /// Strictly inside (the sample points never lie on a boundary).
    pub(super) fn contains(&self, p: [f64; 2]) -> bool {
        match self {
            Shape::Circle { c, r } => Conic::circle(*c, *r).implicit(p) < 0.,
            Shape::Ellipse { c, a, b, phi } => Conic::ellipse(*c, *a, *b, *phi).implicit(p) < 0.,
            Shape::Polygon { vertices } => {
                let n = vertices.len();
                let mut winding = 0i32;
                for i in 0..n {
                    let (a, b) = (vertices[i], vertices[(i + 1) % n]);
                    if a[1] <= p[1] {
                        if b[1] > p[1] && arrangement::orient(a, b, p) > 0. {
                            winding += 1;
                        }
                    } else if b[1] <= p[1] && arrangement::orient(a, b, p) < 0. {
                        winding -= 1;
                    }
                }
                winding != 0
            }
        }
    }
}

fn numeric(v: &Value, label: &str) -> Result<Matrix> {
    let m = match v {
        Value::Matrix(m) | Value::Container(Array::Numeric(m)) => m.clone(),
        Value::Logical(l) | Value::Container(Array::Logical(l)) => {
            let data = l.data().iter().map(|&b| f64::from(b)).collect();
            Matrix::from_column_vec(l.rows(), l.cols(), data)?
        }
        Value::Container(Array::Typed(a)) => a.to_matrix()?,
        _ => return Err(format!("{label} must be a real numeric matrix")),
    };
    if m.is_nd() || m.data().iter().any(|v| !v.is_finite()) {
        return Err(format!("{label} must be a finite real 2-D matrix"));
    }
    Ok(m)
}

fn parse_shapes(gd: &Matrix) -> Result<Vec<Shape>> {
    if gd.cols() == 0 {
        return Err("decsg: gd must have at least one shape column".into());
    }
    if gd.cols() > MAX_SHAPES {
        return Err(format!("decsg: at most {MAX_SHAPES} shapes are supported"));
    }
    let rows = gd.rows();
    let mut shapes = Vec::with_capacity(gd.cols());
    let mut pieces = 0usize;
    for j in 0..gd.cols() {
        let column = j + 1;
        let at = |i: usize| if i < rows { gd.get(i, j) } else { 0. };
        let need = |n: usize, what: &str| -> Result<()> {
            if rows < n {
                return Err(format!(
                    "decsg: gd column {column} ({what}) needs at least {n} rows"
                ));
            }
            Ok(())
        };
        let shape = match at(0) {
            1. => {
                need(4, "circle")?;
                let r = at(3);
                if r <= 0. {
                    return Err(format!(
                        "decsg: gd column {column}: circle radius must be positive"
                    ));
                }
                pieces += 4;
                Shape::Circle {
                    c: [at(1), at(2)],
                    r,
                }
            }
            code @ (2. | 3.) => {
                let what = if code == 2. { "polygon" } else { "rectangle" };
                let n = at(1);
                if code == 3. && n != 4. {
                    return Err(format!(
                        "decsg: gd column {column}: a rectangle has 4 vertices (row 2 must be 4)"
                    ));
                }
                if n.fract() != 0. || n < 3. {
                    return Err(format!(
                        "decsg: gd column {column}: a polygon needs at least 3 vertices"
                    ));
                }
                if n > MAX_PIECES as f64 {
                    return Err(format!(
                        "decsg: gd column {column}: more than {MAX_PIECES} vertices"
                    ));
                }
                let n = n as usize;
                need(2 + 2 * n, what)?;
                let vertices: Vec<[f64; 2]> = (0..n).map(|k| [at(2 + k), at(2 + n + k)]).collect();
                check_polygon(&vertices, column)?;
                pieces += n;
                Shape::Polygon { vertices }
            }
            4. => {
                need(6, "ellipse")?;
                let (a, b) = (at(3), at(4));
                if a <= 0. || b <= 0. {
                    return Err(format!(
                        "decsg: gd column {column}: ellipse semi-axes must be positive"
                    ));
                }
                pieces += 4;
                Shape::Ellipse {
                    c: [at(1), at(2)],
                    a,
                    b,
                    phi: at(5),
                }
            }
            code => {
                return Err(format!(
                    "decsg: gd column {column}: unknown shape code {code} (1 circle, 2 polygon, 3 rectangle, 4 ellipse)"
                ));
            }
        };
        if pieces > MAX_PIECES {
            return Err(format!("decsg: more than {MAX_PIECES} boundary pieces"));
        }
        shapes.push(shape);
    }
    Ok(shapes)
}

/// Rejects repeated vertices, zero area and self-intersections.
fn check_polygon(v: &[[f64; 2]], column: usize) -> Result<()> {
    let n = v.len();
    let bad = || {
        Err(format!(
            "decsg: gd column {column}: polygon is self-intersecting or degenerate"
        ))
    };
    for i in 0..n {
        if v[i] == v[(i + 1) % n] {
            return bad();
        }
    }
    let area: f64 = (0..n)
        .map(|i| v[i][0] * v[(i + 1) % n][1] - v[(i + 1) % n][0] * v[i][1])
        .sum();
    if area == 0. {
        return bad();
    }
    for i in 0..n {
        let (a, b) = (v[i], v[(i + 1) % n]);
        for j in i + 1..n {
            let (c, d) = (v[j], v[(j + 1) % n]);
            let adjacent = j == i + 1 || (i == 0 && j == n - 1);
            if adjacent {
                // Adjacent sides share one vertex; they must not fold back.
                let (shared, p, q) = if j == i + 1 { (b, a, d) } else { (a, b, c) };
                if arrangement::orient(p, shared, q) == 0.
                    && (p[0] - shared[0]) * (q[0] - shared[0])
                        + (p[1] - shared[1]) * (q[1] - shared[1])
                        > 0.
                {
                    return bad();
                }
            } else if arrangement::segments_touch(a, b, c, d) {
                return bad();
            }
        }
    }
    Ok(())
}

fn text(v: &Value) -> Option<String> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
        Value::Container(Array::Character(c)) if c.rows() <= 1 => {
            Some(String::from_utf16_lossy(c.units()))
        }
        _ => None,
    }
}

/// Names from ns: one per column of a char (or character-code) matrix.
fn names(v: &Value) -> Result<Vec<String>> {
    let (rows, cols, units): (usize, usize, Vec<u32>) = match v {
        Value::Text(s) => {
            let u: Vec<u32> = s.chars().map(u32::from).collect();
            (usize::from(!u.is_empty()), u.len(), u)
        }
        Value::Container(Array::Character(c)) => {
            if c.is_nd() {
                return Err("decsg: ns must be a 2-D char matrix".into());
            }
            (
                c.rows(),
                c.cols(),
                c.units().iter().map(|&u| u32::from(u)).collect(),
            )
        }
        Value::Matrix(_) | Value::Container(Array::Numeric(_)) => {
            let m = numeric(v, "decsg: ns")?;
            if m.data()
                .iter()
                .any(|&x| x < 0. || x.fract() != 0. || x > 0x10FFFF as f64)
            {
                return Err("decsg: ns must contain character codes".into());
            }
            (
                m.rows(),
                m.cols(),
                m.data().iter().map(|&x| x as u32).collect(),
            )
        }
        _ => return Err("decsg: ns must be a char matrix with one name per column".into()),
    };
    let mut out = Vec::with_capacity(cols);
    for j in 0..cols {
        let name: String = (0..rows)
            .filter_map(|i| char::from_u32(units[j * rows + i]))
            .collect::<String>()
            .trim_matches(|c: char| c == ' ' || c == '\0')
            .to_owned();
        if name.is_empty() {
            return Err(format!("decsg: ns column {} is an empty name", j + 1));
        }
        if !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Err(format!(
                "decsg: ns name '{name}' must contain only letters, digits and underscores"
            ));
        }
        if out.contains(&name) {
            return Err(format!("decsg: ns name '{name}' is used twice"));
        }
        out.push(name);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Set formula: `+` union, `*` intersection, `-` difference, parentheses.
// Precedence, highest first: `-`, `*`, `+`; each is left associative.

#[derive(Debug)]
enum SetExpr {
    Shape(usize),
    Union(Box<SetExpr>, Box<SetExpr>),
    Inter(Box<SetExpr>, Box<SetExpr>),
    Diff(Box<SetExpr>, Box<SetExpr>),
}

impl SetExpr {
    fn eval(&self, inside: &[bool]) -> bool {
        match self {
            SetExpr::Shape(i) => inside[*i],
            SetExpr::Union(a, b) => a.eval(inside) || b.eval(inside),
            SetExpr::Inter(a, b) => a.eval(inside) && b.eval(inside),
            SetExpr::Diff(a, b) => a.eval(inside) && !b.eval(inside),
        }
    }
}

struct FormulaParser<'a> {
    chars: Vec<char>,
    pos: usize,
    depth: usize,
    names: &'a [String],
}

fn parse_formula(sf: &str, names: &[String]) -> Result<SetExpr> {
    if sf.chars().count() > MAX_FORMULA {
        return Err(format!(
            "decsg: set formula longer than {MAX_FORMULA} characters"
        ));
    }
    let mut p = FormulaParser {
        chars: sf.chars().collect(),
        pos: 0,
        depth: 0,
        names,
    };
    let expr = p.sum()?;
    p.skip();
    if p.pos < p.chars.len() {
        return Err(format!(
            "decsg: malformed set formula '{sf}': unexpected '{}' at position {}",
            p.chars[p.pos],
            p.pos + 1
        ));
    }
    Ok(expr)
}

impl FormulaParser<'_> {
    fn skip(&mut self) {
        while self.pos < self.chars.len() && self.chars[self.pos].is_whitespace() {
            self.pos += 1;
        }
    }
    fn peek(&mut self) -> Option<char> {
        self.skip();
        self.chars.get(self.pos).copied()
    }
    fn error(&self, what: &str) -> String {
        let sf: String = self.chars.iter().collect();
        format!("decsg: malformed set formula '{sf}': {what}")
    }
    // Precedence, highest first: -, *, +; equal operators associate left to right.
    fn sum(&mut self) -> Result<SetExpr> {
        let mut left = self.product()?;
        while self.peek() == Some('+') {
            self.pos += 1;
            let right = self.product()?;
            left = SetExpr::Union(Box::new(left), Box::new(right));
        }
        Ok(left)
    }
    fn product(&mut self) -> Result<SetExpr> {
        let mut left = self.difference()?;
        while self.peek() == Some('*') {
            self.pos += 1;
            let right = self.difference()?;
            left = SetExpr::Inter(Box::new(left), Box::new(right));
        }
        Ok(left)
    }
    fn difference(&mut self) -> Result<SetExpr> {
        let mut left = self.atom()?;
        while self.peek() == Some('-') {
            self.pos += 1;
            let right = self.atom()?;
            left = SetExpr::Diff(Box::new(left), Box::new(right));
        }
        Ok(left)
    }
    fn atom(&mut self) -> Result<SetExpr> {
        match self.peek() {
            Some('(') => {
                self.depth += 1;
                if self.depth > 200 {
                    return Err(self.error("parentheses nested too deeply"));
                }
                self.pos += 1;
                let inner = self.sum()?;
                if self.peek() != Some(')') {
                    return Err(self.error("missing ')'"));
                }
                self.pos += 1;
                self.depth -= 1;
                Ok(inner)
            }
            Some(c) if c.is_alphanumeric() || c == '_' => {
                let start = self.pos;
                while self.pos < self.chars.len()
                    && (self.chars[self.pos].is_alphanumeric() || self.chars[self.pos] == '_')
                {
                    self.pos += 1;
                }
                let name: String = self.chars[start..self.pos].iter().collect();
                match self.names.iter().position(|n| *n == name) {
                    Some(i) => Ok(SetExpr::Shape(i)),
                    None => Err(format!(
                        "decsg: unknown name '{name}' in the set formula (not in ns)"
                    )),
                }
            }
            Some(c) => Err(self.error(&format!("expected a name or '(' but found '{c}'"))),
            None => Err(self.error("expected a name or '(' at the end")),
        }
    }
}

// ---------------------------------------------------------------------------
// Builtins.

fn decsg(e: &mut Engine, args: &[Value]) -> Result<(Matrix, LogicalMatrix)> {
    let formula = match args.len() {
        1 => None,
        3 => Some((&args[1], &args[2])),
        2 => {
            return Err(
                "decsg: give both the set formula and the name space (decsg(gd,sf,ns))".into(),
            );
        }
        _ => return Err("decsg: expected decsg(gd) or decsg(gd,sf,ns)".into()),
    };
    let gd = numeric(&args[0], "decsg: gd")?;
    let shapes = parse_shapes(&gd)?;
    let expr = match formula {
        None => None,
        Some((sf, ns)) => {
            let sf = text(sf).ok_or("decsg: sf must be a character vector or string")?;
            let names = names(ns)?;
            if names.len() != shapes.len() {
                return Err(format!(
                    "decsg: ns has {} names but gd has {} shapes",
                    names.len(),
                    shapes.len()
                ));
            }
            Some(parse_formula(&sf, &names)?)
        }
    };
    e.check_cancel()?;
    let faces = arrangement::decompose(&shapes, &|| e.check_cancel())?;
    // Kept regions are numbered in face order.
    let mut label = vec![0; faces.inside.len()];
    let mut kept = Vec::new();
    for (f, inside) in faces.inside.iter().enumerate() {
        let keep = match &expr {
            Some(x) => x.eval(inside),
            None => inside.iter().any(|&b| b),
        };
        if keep {
            kept.push(f);
            label[f] = kept.len();
        }
    }
    let edges: Vec<GEdge> = faces
        .edges
        .iter()
        .filter_map(|&(kind, start, end, left, right)| {
            let (l, r) = (left.map_or(0, |f| label[f]), right.map_or(0, |f| label[f]));
            (l != 0 || r != 0).then(|| GEdge::new(kind, start, end, l, r))
        })
        .collect();
    let geometry = Geometry2 {
        edges,
        num_faces: kept.len(),
    };
    let dl = geometry.to_dl()?;
    // The output must read back as the same geometry.
    if Geometry2::from_dl(&dl)? != geometry {
        return Err("decsg: internal error: the decomposed geometry does not round-trip".into());
    }
    let n = shapes.len();
    let mut bits = vec![0u8; kept.len() * n];
    for (i, &f) in kept.iter().enumerate() {
        for j in 0..n {
            bits[j * kept.len() + i] = u8::from(faces.inside[f][j]);
        }
    }
    let bt = LogicalMatrix::from_column_vec(kept.len(), n, bits, &e.cancel)?;
    Ok((dl, bt))
}

#[allow(clippy::needless_range_loop)]
fn csgdel(e: &mut Engine, args: &[Value]) -> Result<(Matrix, LogicalMatrix)> {
    if !(2..=3).contains(&args.len()) {
        return Err("csgdel: expected csgdel(dl,bt) or csgdel(dl,bt,bl)".into());
    }
    let dl = numeric(&args[0], "csgdel: dl")?;
    let bt = numeric(&args[1], "csgdel: bt")?;
    let mut g = Geometry2::from_dl(&dl).map_err(|m| format!("csgdel: {m}"))?;
    let nf = g.num_faces;
    if bt.rows() != nf && !(bt.is_empty() && nf == 0) {
        return Err(format!(
            "csgdel: bt has {} rows but dl has {nf} regions",
            bt.rows()
        ));
    }
    let interior = |e: &GEdge| e.left != 0 && e.right != 0;
    let mut remove = vec![false; g.edges.len()];
    match args.get(2) {
        None => {
            for (k, edge) in g.edges.iter().enumerate() {
                remove[k] = interior(edge);
            }
        }
        Some(bl) => {
            let bl = numeric(bl, "csgdel: bl")?;
            for &v in bl.data() {
                if v < 1. || v.fract() != 0. || v > g.edges.len() as f64 {
                    return Err(format!(
                        "csgdel: bl entries must be column indices of dl in 1..{}",
                        g.edges.len()
                    ));
                }
                let k = v as usize - 1;
                if !interior(&g.edges[k]) {
                    return Err(format!(
                        "csgdel: dl column {v} borders the outside and cannot be deleted"
                    ));
                }
                remove[k] = true;
            }
        }
    }
    // Merge the faces on both sides of every deleted border.
    let mut parent: Vec<usize> = (0..=nf).collect();
    fn root(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    for (k, edge) in g.edges.iter().enumerate() {
        if remove[k] {
            let (a, b) = (root(&mut parent, edge.left), root(&mut parent, edge.right));
            let (lo, hi) = (a.min(b), a.max(b));
            parent[hi] = lo;
        }
    }
    let mut new_label = vec![0usize; nf + 1];
    let mut count = 0;
    for f in 1..=nf {
        let r = root(&mut parent, f);
        if r == f {
            count += 1;
            new_label[f] = count;
        }
    }
    for f in 1..=nf {
        let r = root(&mut parent, f);
        new_label[f] = new_label[r];
    }
    e.check_cancel()?;
    let mut edges = Vec::with_capacity(g.edges.len());
    for edge in g.edges.drain(..) {
        let (l, r) = (new_label[edge.left], new_label[edge.right]);
        // Borders left with the same region on both sides go as well.
        if l != r {
            edges.push(GEdge {
                left: l,
                right: r,
                ..edge
            });
        }
    }
    let g = Geometry2 {
        edges,
        num_faces: count,
    };
    let dl2 = g.to_dl()?;
    let cols = bt.cols();
    let mut bits = vec![0u8; count * cols];
    for f in 1..=nf {
        let i = new_label[f] - 1;
        for j in 0..cols {
            if bt.get(f - 1, j) != 0. {
                bits[j * count + i] = 1;
            }
        }
    }
    let bt2 = LogicalMatrix::from_column_vec(count, cols, bits, &e.cancel)?;
    Ok((dl2, bt2))
}

/// Exact quarter points of a conic at its own parameter 0, pi/2, pi, 3 pi/2.
pub(super) fn quarter_points(conic: &Conic) -> [[f64; 2]; 4] {
    [
        conic.from_local(conic.a, 0.),
        conic.from_local(0., conic.b),
        conic.from_local(-conic.a, 0.),
        conic.from_local(0., -conic.b),
    ]
}
