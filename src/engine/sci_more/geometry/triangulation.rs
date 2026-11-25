//! The immutable 2-D triangulation object and finite dual edges for voronoi.
use super::{Delaunay, P, Tri, circumcenter, connectivity, first_occurrences, incircle, orient};
use super::{MAX_POINTS, NO_ND, sc2_num, triangle_order_key};
use crate::engine::{Engine, Execution, Value};
use crate::{Array, CellArray, Matrix, Result};
use std::sync::Arc;

pub(in crate::engine) const CLASS: &str = "delaunayTriangulation";
pub(in crate::engine) const PROPERTIES: [&str; 3] = ["Points", "ConnectivityList", "Constraints"];
pub(in crate::engine) const METHODS: [&str; 6] = [
    "pointLocation",
    "nearestNeighbor",
    "edges",
    "neighbors",
    "convexHull",
    "vertexAttachments",
];

pub(in crate::engine) fn has_multiple_outputs(name: &str) -> bool {
    matches!(name, "pointLocation" | "nearestNeighbor" | "convexHull")
}

pub(in crate::engine) fn method_outputs(
    engine: &mut Engine,
    receiver: &Value,
    name: &str,
    args: &[crate::syntax::Expr],
    count: usize,
    out: &mut Execution,
) -> Result<Option<Vec<Value>>> {
    if count > 1
        && has_multiple_outputs(name)
        && let Some(dt) = crate::engine::objects::triangulation_value(engine, receiver)
    {
        let args = engine.function_arguments(args, None, out)?;
        let values = dt.call(engine, name, &args, count)?;
        engine.charge_function_outputs(&values)?;
        return Ok(Some(values));
    }
    Ok(None)
}

pub(in crate::engine) struct Triangulation {
    points: Matrix,
    pub(in crate::engine) connectivity: Matrix,
    mesh: Option<Delaunay>,
    real_count: usize,
    last_incident: Vec<usize>,
}

impl Triangulation {
    pub(in crate::engine) fn new(e: &mut Engine, args: &[Value]) -> Result<Self> {
        let points = match args {
            [] => Matrix::filled(0, 2, 0.)?,
            [p] => points_matrix(e, CLASS, p)?,
            [x, y] => coordinate_vectors(e, CLASS, x, y)?,
            [_, _, _] => {
                return Err(format!(
                    "{CLASS}: 3-D and constrained inputs are unsupported ({NO_ND} for 3-D)"
                ));
            }
            _ => return Err(format!("{CLASS}: expected P or x and y")),
        };
        let sites = finite_sites(e, CLASS, &points)?;
        let (unique, _) = first_occurrences(&sites);
        let (points, sites, order) = if unique.len() == sites.len() {
            (points, sites, unique)
        } else {
            let sites: Vec<_> = unique.into_iter().map(|i| sites[i]).collect();
            let data = (0..2).flat_map(|axis| sites.iter().map(move |p| p[axis]));
            let points = Matrix::from_column_vec(sites.len(), 2, data.collect())?;
            let order = (0..sites.len()).collect();
            (points, sites, order)
        };
        let mesh = build(e, sites, &order)?;
        let (mesh, real_count) = match mesh {
            Some(d) => {
                let (d, count) = compact(e, d)?;
                (Some(d), count)
            }
            None => (None, 0),
        };
        let real = mesh.as_ref().map_or(&[][..], |d| &d.tris[..real_count]);
        let connectivity = connectivity(real_count, real.iter().map(|t| t.v))?;
        // Shared edges and vertices select the largest incident triangle ID.
        let mut last_incident = vec![0; points.rows()];
        for (id, tri) in real.iter().enumerate() {
            e.sc2_poll(id)?;
            for i in tri.v {
                last_incident[i] = id;
            }
        }
        Ok(Self {
            points,
            connectivity,
            mesh,
            real_count,
            last_incident,
        })
    }

    pub(in crate::engine) fn bytes(&self) -> usize {
        size_of::<Self>()
            + size_of::<f64>() * (self.points.len() + self.connectivity.len())
            + size_of::<usize>() * self.last_incident.capacity()
            + self.mesh.as_ref().map_or(0, |d| {
                size_of::<P>() * d.pts.capacity()
                    + size_of::<Tri>() * d.tris.capacity()
                    + size_of::<bool>() * d.alive.capacity()
            })
    }

    pub(in crate::engine) fn property(&self, name: &str) -> Result<Value> {
        let matrix = match name {
            "Points" => self.points.clone(),
            "ConnectivityList" => self.connectivity.clone(),
            "Constraints" => Matrix::filled(0, 0, 0.)?,
            _ => return Err(format!("{CLASS} has no property '{name}'")),
        };
        Ok(Value::Matrix(matrix))
    }

    pub(in crate::engine) fn body(&self) -> String {
        format!(
            "  {CLASS} with properties:\n\n             Points: [{}x2 double]\n   ConnectivityList: [{}x3 double]\n        Constraints: []\n",
            self.points.rows(),
            self.real_count
        )
    }

    pub(in crate::engine) fn call(
        &self,
        e: &mut Engine,
        name: &str,
        args: &[Value],
        count: usize,
    ) -> Result<Vec<Value>> {
        if count > if has_multiple_outputs(name) { 2 } else { 1 } {
            return Err("Too many output arguments.".into());
        }
        if matches!(name, "pointLocation" | "nearestNeighbor") {
            let [q] = args else {
                return Err(format!("{name}: expected an N-by-2 query matrix"));
            };
            let q = points_matrix(e, name, q)?;
            return if name == "pointLocation" {
                self.point_location(e, &q, count)
            } else {
                self.nearest_neighbor(e, &q, count)
            };
        }
        if !args.is_empty() {
            if name == "vertexAttachments" {
                return Err(format!("{name}: a vertex ID list is not supported yet"));
            }
            return Err(format!("{name}: expected only a triangulation"));
        }
        match name {
            "edges" => Ok(vec![self.edges(e)?]),
            "neighbors" => Ok(vec![self.neighbors(e)?]),
            "convexHull" => self.hull(e, count),
            "vertexAttachments" => Ok(vec![self.vertex_attachments(e)?]),
            _ => unreachable!("checked by METHODS"),
        }
    }

    fn point_location(&self, e: &mut Engine, q: &Matrix, count: usize) -> Result<Vec<Value>> {
        let n = q.rows();
        e.sc2_pairs(self.real_count, n)?;
        let width = if count == 2 { 3 } else { 0 };
        e.preflight_elements(n * (1 + width))?;
        let mut ids = vec![f64::NAN; n];
        let mut bary = vec![f64::NAN; n * width];
        if let Some(d) = &self.mesh {
            for (i, p) in matrix_points(q).enumerate() {
                e.sc2_poll(i)?;
                if p.iter().any(|x| !x.is_finite()) {
                    continue;
                }
                let mut t = d.locate(p);
                if d.is_ghost(t) {
                    continue;
                }
                let [a, b, c] = d.tris[t].v.map(|v| d.pts[v]);
                let sides = [orient(b, c, p), orient(c, a, p), orient(a, b, p)];
                if !sides.iter().all(|s| *s >= 0.) {
                    continue;
                }
                if let Some(k) = sides.iter().position(|s| *s == 0.) {
                    let j = (k + 1) % 3;
                    let l = (k + 2) % 3;
                    let neighbor = d.tris[t].n[k];
                    t = if sides[j] == 0. {
                        self.last_incident[d.tris[t].v[l]]
                    } else if sides[l] == 0. {
                        self.last_incident[d.tris[t].v[j]]
                    } else if neighbor < self.real_count {
                        t.max(neighbor)
                    } else {
                        t
                    };
                }
                ids[i] = (t + 1) as f64;
                if width != 0 {
                    let [a, b, c] = d.tris[t].v.map(|v| d.pts[v]);
                    let area = orient(a, b, c);
                    bary[i] = orient(b, c, p) / area;
                    bary[n + i] = orient(c, a, p) / area;
                    bary[2 * n + i] = orient(a, b, p) / area;
                }
            }
        }
        query_outputs(ids, bary, width)
    }

    fn nearest_neighbor(&self, e: &mut Engine, q: &Matrix, count: usize) -> Result<Vec<Value>> {
        let n = q.rows();
        e.sc2_pairs(self.points.rows(), n)?;
        let width = usize::from(count == 2);
        e.preflight_elements(n * (1 + width))?;
        let mut ids = vec![f64::NAN; n];
        let mut distances = vec![f64::NAN; n * width];
        if !self.points.is_empty() {
            for (i, q) in matrix_points(q).enumerate() {
                e.sc2_poll(i)?;
                if q.iter().any(|x| !x.is_finite()) {
                    continue;
                }
                let mut best = (0, f64::INFINITY);
                for (j, p) in matrix_points(&self.points).enumerate() {
                    e.sc2_poll(j)?;
                    let d = (p[0] - q[0]).hypot(p[1] - q[1]);
                    if d < best.1 {
                        best = (j, d);
                    }
                }
                ids[i] = (best.0 + 1) as f64;
                if width != 0 {
                    distances[i] = best.1;
                }
            }
        }
        query_outputs(ids, distances, width)
    }

    fn edges(&self, e: &mut Engine) -> Result<Value> {
        let Some(d) = &self.mesh else {
            return sc2_num(0, 2, Vec::new());
        };
        // Each interior edge belongs to two triangles; visit it from only one.
        let boundary = d.tris.len() - self.real_count;
        let n = (3 * self.real_count + boundary) / 2;
        e.preflight_elements(4 * n)?;
        let mut edges = Vec::with_capacity(n);
        for (i, t) in d.tris[..self.real_count].iter().enumerate() {
            e.sc2_poll(i)?;
            for k in 0..3 {
                if t.n[k] > i {
                    let a = t.v[(k + 1) % 3];
                    let b = t.v[(k + 2) % 3];
                    edges.push([a.min(b), a.max(b)]);
                }
            }
        }
        edges.sort_unstable();
        e.check_cancel()?;
        let cols = (0..2).flat_map(|k| edges.iter().map(move |edge| (edge[k] + 1) as f64));
        sc2_num(edges.len(), 2, cols.collect())
    }

    fn neighbors(&self, e: &mut Engine) -> Result<Value> {
        let Some(d) = &self.mesh else {
            return sc2_num(0, 3, Vec::new());
        };
        e.preflight_elements(3 * self.real_count)?;
        let mut data = Vec::with_capacity(3 * self.real_count);
        for k in 0..3 {
            for (i, t) in d.tris[..self.real_count].iter().enumerate() {
                e.sc2_poll(i)?;
                data.push(if t.n[k] < self.real_count {
                    (t.n[k] + 1) as f64
                } else {
                    f64::NAN
                });
            }
        }
        sc2_num(self.real_count, 3, data)
    }

    /// One cell per vertex: the ascending ConnectivityList rows that use it.
    fn vertex_attachments(&self, e: &mut Engine) -> Result<Value> {
        let n = self.points.rows();
        let m = self.real_count;
        e.preflight_elements(3 * m + n)?;
        let rows = |k: usize| (0..m).map(move |i| self.connectivity.get(i, k) as usize - 1);
        let mut counts = vec![0usize; n];
        for k in 0..3 {
            for (i, v) in rows(k).enumerate() {
                e.sc2_poll(i)?;
                counts[v] += 1;
            }
        }
        let mut faces: Vec<Vec<f64>> = counts.iter().map(|&c| Vec::with_capacity(c)).collect();
        // Row-major over the stored rows keeps each list ascending.
        for i in 0..m {
            e.sc2_poll(i)?;
            for k in 0..3 {
                faces[self.connectivity.get(i, k) as usize - 1].push((i + 1) as f64);
            }
        }
        let mut cells = Vec::with_capacity(n);
        for (i, ids) in faces.into_iter().enumerate() {
            e.sc2_poll(i)?;
            cells.push(Arc::new(Array::Numeric(Matrix::from_column_vec(
                1,
                ids.len(),
                ids,
            )?)));
        }
        Ok(Value::Container(Array::Cell(CellArray::from_shared(
            n, 1, cells, &e.cancel,
        )?)))
    }

    fn hull(&self, e: &mut Engine, count: usize) -> Result<Vec<Value>> {
        let mut area = 0.;
        let mut ring = Vec::new();
        if let Some(d) = &self.mesh {
            let ghosts = &d.tris[self.real_count..];
            e.preflight_elements(d.pts.len() + ghosts.len() + 1)?;
            let mut next = vec![0; d.pts.len()];
            let mut start = d.pts.len();
            for (i, t) in ghosts.iter().enumerate() {
                e.sc2_poll(i)?;
                let [b, a, _] = t.v;
                next[a] = b;
                start = start.min(a);
            }
            ring.reserve_exact(ghosts.len() + 1);
            let mut a = start;
            for i in 0..ghosts.len() {
                e.sc2_poll(i)?;
                ring.push((a + 1) as f64);
                let b = next[a];
                if count == 2 {
                    area += orient(d.pts[start], d.pts[a], d.pts[b]) / 2.;
                }
                a = b;
            }
            ring.push((start + 1) as f64);
        }
        let mut values = vec![sc2_num(ring.len(), 1, ring)?];
        if count == 2 {
            values.push(Value::Matrix(Matrix::scalar(area)));
        }
        Ok(values)
    }
}

fn real_coordinates(e: &Engine, name: &str, input: &Value) -> Result<Matrix> {
    if input.is_nd()
        || !matches!(
            input,
            Value::Matrix(_) | Value::Logical(_) | Value::Container(Array::Typed(_))
        )
    {
        return Err(format!("{name}: coordinates must be real numeric arrays"));
    }
    input.clone().matrix(&e.cancel)
}

fn points_matrix(e: &Engine, name: &str, input: &Value) -> Result<Matrix> {
    let (rows, cols) = input.shape(&e.cancel)?;
    if rows > MAX_POINTS {
        return Err(format!("{name}: at most {MAX_POINTS} points are supported"));
    }
    if cols == 3 && name == CLASS {
        return Err(format!("{CLASS} of three-dimensional points: {NO_ND}"));
    }
    if cols != 2 && (rows, cols) != (0, 0) {
        return Err(format!("{name}: expected an N-by-2 point matrix"));
    }
    e.preflight_elements(rows * cols)?;
    let points = real_coordinates(e, name, input)?;
    if cols == 0 {
        Matrix::filled(0, 2, 0.)
    } else {
        Ok(points)
    }
}

fn coordinate_vectors(e: &Engine, name: &str, x: &Value, y: &Value) -> Result<Matrix> {
    let lengths = [x, y].map(|v| -> Result<usize> {
        let (rows, cols) = v.shape(&e.cancel)?;
        if rows.min(cols) > 1 {
            return Err(format!("{name}: x and y must be vectors"));
        }
        Ok(rows * cols)
    });
    let [nx, ny] = lengths;
    let (nx, ny) = (nx?, ny?);
    if nx != ny {
        return Err(format!(
            "{name}: x and y must have the same number of points"
        ));
    }
    if nx > MAX_POINTS {
        return Err(format!("{name}: at most {MAX_POINTS} points are supported"));
    }
    e.preflight_elements(2 * nx)?;
    let x = real_coordinates(e, name, x)?;
    let y = real_coordinates(e, name, y)?;
    Matrix::from_column_vec(nx, 2, x.data().iter().chain(y.data()).copied().collect())
}

fn matrix_points(m: &Matrix) -> impl Iterator<Item = P> + '_ {
    let (x, y) = m.data().split_at(m.rows());
    x.iter().zip(y).map(|(&x, &y)| [x, y])
}

fn finite_sites(e: &Engine, name: &str, points: &Matrix) -> Result<Vec<P>> {
    // Sites, the duplicate map and first-occurrence indices.
    e.preflight_elements(10 * points.rows())?;
    matrix_points(points)
        .enumerate()
        .map(|(i, p)| {
            e.sc2_poll(i)?;
            if p.iter().any(|x| !x.is_finite()) {
                return Err(format!("{name}: data points must be finite"));
            }
            Ok(p)
        })
        .collect()
}

fn build(e: &mut Engine, pts: Vec<P>, order: &[usize]) -> Result<Option<Delaunay>> {
    // The bounded kernel recycles the cavity; the live mesh, including the
    // triangles at infinity, contains at most twice the number of sites.
    let scratch = 12 * pts.len();
    e.preflight_elements(scratch)?;
    e.charge(8 * pts.len())?;
    let memory = crate::engine::limits::memory_units(&e.limits);
    let evaluation = e.limits.evaluation_units.map_or(usize::MAX, |limit| {
        limit.saturating_sub(e.evaluated_elements)
    });
    // Allow for capacity growth and the cavity's temporary edge tables.
    let budget = memory.min(evaluation).saturating_sub(scratch) / 32;
    let arena_limit = (2 * order.len()).min(budget);
    Delaunay::build_bounded(pts, order, e, arena_limit)
}

fn compact(e: &Engine, mut d: Delaunay) -> Result<(Delaunay, usize)> {
    let active = d.alive.iter().filter(|&&alive| alive).count();
    e.preflight_elements(d.tris.len() + 8 * active)?;
    let mut real = Vec::with_capacity(active);
    let mut ghosts = Vec::new();
    for t in 0..d.tris.len() {
        e.sc2_poll(t)?;
        if d.alive[t] {
            if d.is_ghost(t) {
                ghosts.push(t);
            } else {
                real.push(t);
            }
        }
    }
    real.sort_unstable_by_key(|&t| triangle_order_key(d.tris[t].v));
    let mut ids = vec![0; d.tris.len()];
    for (new, &old) in real.iter().chain(&ghosts).enumerate() {
        e.sc2_poll(new)?;
        ids[old] = new;
    }
    let mut tris = Vec::with_capacity(active);
    for (new, &old) in real.iter().chain(&ghosts).enumerate() {
        e.sc2_poll(new)?;
        let mut tri = d.tris[old].clone();
        tri.n = tri.n.map(|n| ids[n]);
        if new < real.len() {
            let k = (0..3).min_by_key(|&k| tri.v[k]).expect("three vertices");
            tri.v.rotate_left(k);
            tri.n.rotate_left(k);
        }
        tris.push(tri);
    }
    d.last = ids[d.last];
    d.tris = tris;
    d.alive = vec![true; active];
    Ok((d, real.len()))
}

fn query_outputs(ids: Vec<f64>, extra: Vec<f64>, width: usize) -> Result<Vec<Value>> {
    let n = ids.len();
    let mut out = vec![sc2_num(n, 1, ids)?];
    if width != 0 {
        out.push(sc2_num(n, width, extra)?);
    }
    Ok(out)
}

pub(in crate::engine) fn voronoi(
    e: &mut Engine,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    let [x, y] = args else {
        return Err("voronoi: expected x and y coordinate vectors".into());
    };
    let points = coordinate_vectors(e, "voronoi", x, y)?;
    let sites = finite_sites(e, "voronoi", &points)?;
    let (order, _) = first_occurrences(&sites);
    let segments = match build(e, sites, &order)? {
        Some(d) => finite_segments(e, &d)?,
        None => Vec::new(),
    };
    let plotted = count < 2;
    let width = if plotted { 3 } else { 2 };
    let samples = width * segments.len();
    if plotted && samples + points.rows() > crate::plot::MAX_POINTS {
        return Err("voronoi: plot exceeds the point limit".into());
    }
    e.preflight_elements(2 * samples)?;
    let axis_values = |axis| -> Vec<f64> {
        let mut data = Vec::with_capacity(samples);
        for [a, b] in &segments {
            data.extend([a[axis], b[axis]]);
            if plotted {
                data.push(f64::NAN);
            }
        }
        data
    };
    if !plotted {
        return Ok(vec![
            sc2_num(2, segments.len(), axis_values(0))?,
            sc2_num(2, segments.len(), axis_values(1))?,
        ]);
    }
    let sites = points.rows();
    let result = e.call(
        "plot",
        vec![
            Value::Matrix(Matrix::from_column_slice(
                sites,
                1,
                &points.data()[..sites],
            )?),
            Value::Matrix(Matrix::from_column_slice(
                sites,
                1,
                &points.data()[sites..],
            )?),
            Value::Text(".".into()),
            sc2_num(samples, 1, axis_values(0))?,
            sc2_num(samples, 1, axis_values(1))?,
            Value::Text("-".into()),
        ],
        out,
    )?;
    Ok(if count == 0 { Vec::new() } else { vec![result] })
}

fn finite_segments(e: &Engine, d: &Delaunay) -> Result<Vec<[P; 2]>> {
    let active = d.alive.iter().filter(|&&alive| alive).count();
    e.preflight_elements(2 * d.tris.len() + 6 * active)?;
    let mut centers = vec![[f64::NAN; 2]; d.tris.len()];
    for (i, t) in d.tris.iter().enumerate() {
        e.sc2_poll(i)?;
        if d.alive[i] && !d.is_ghost(i) {
            let [a, b, c] = t.v.map(|v| d.pts[v]);
            centers[i] = circumcenter(a, b, c);
        }
    }
    let mut segments = Vec::new();
    for (i, t) in d.tris.iter().enumerate() {
        e.sc2_poll(i)?;
        if !d.alive[i] || d.is_ghost(i) {
            continue;
        }
        for n in t.n {
            if n <= i || d.is_ghost(n) || centers[i] == centers[n] {
                continue;
            }
            let [a, b, c] = t.v.map(|v| d.pts[v]);
            let other = d.tris[n]
                .v
                .iter()
                .find(|v| !t.v.contains(v))
                .expect("opposite vertex");
            // A co-circular diagonal has a single dual vertex, even if the
            // two floating-point circumcenter computations round differently.
            if incircle(a, b, c, d.pts[*other]) != 0.
                && centers[i].iter().chain(&centers[n]).all(|x| x.is_finite())
            {
                segments.push([centers[i], centers[n]]);
            }
        }
    }
    Ok(segments)
}
