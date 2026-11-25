//! Isosurfaces, their gradient normals and boundary caps on column-major volumes.
use crate::engine::{Engine, Execution, Value};
use crate::{Array, Matrix, Result, StructArray, TypedArray};
use std::{collections::HashMap, sync::Arc};

#[path = "iso_table.rs"]
mod table;
use table::TRIANGLES;

const MAX_FACES: usize = 1_000_000;
const MAX_VERTICES: usize = 3 * MAX_FACES;
// Spatial coordinates are [column, row, page]. Lattice indices are [row, column, page].
const XYZ: [usize; 3] = [1, 0, 2];
const CORNERS: [[usize; 3]; 8] = [
    [0, 0, 0],
    [1, 0, 0],
    [1, 1, 0],
    [0, 1, 0],
    [0, 0, 1],
    [1, 0, 1],
    [1, 1, 1],
    [0, 1, 1],
];
const EDGES: [[usize; 2]; 12] = [
    [0, 1],
    [1, 2],
    [2, 3],
    [3, 0],
    [4, 5],
    [5, 6],
    [6, 7],
    [7, 4],
    [0, 4],
    [1, 5],
    [2, 6],
    [3, 7],
];

pub(in crate::engine) fn handles(name: &str) -> bool {
    matches!(name, "isosurface" | "isonormals" | "isocaps")
}

#[derive(Clone, Copy)]
enum Real<'a> {
    Double(&'a Matrix),
    Typed(&'a TypedArray),
}
impl<'a> Real<'a> {
    fn new(value: &'a Value) -> Result<Self> {
        match value {
            Value::Matrix(m) => Ok(Self::Double(m)),
            Value::Container(Array::Typed(a)) => Ok(Self::Typed(a)),
            _ => Err("Expected a real numeric array".into()),
        }
    }

    fn at(self, index: usize) -> f64 {
        match self {
            Self::Double(m) => m.data()[index],
            Self::Typed(a) => a.real(index),
        }
    }
}

#[derive(Clone, Copy)]
enum Coordinate<'a> {
    Implicit,
    Vector(Real<'a>),
    Full(Real<'a>),
}

struct Grid<'a> {
    shape: [usize; 3],
    coordinates: [Coordinate<'a>; 3],
    values: Real<'a>,
}

impl<'a> Grid<'a> {
    fn new(volume: &'a Value, coordinates: &'a [Value]) -> Result<Self> {
        let shape: [usize; 3] = volume
            .dims()
            .as_ref()
            .try_into()
            .map_err(|_| "V must be a three-dimensional volume")?;
        if shape.iter().any(|&n| n < 2) {
            return Err("V must have at least two samples along each dimension".into());
        }
        let values = Real::new(volume)?;
        let mut grid = Self {
            shape,
            coordinates: [Coordinate::Implicit; 3],
            values,
        };
        for (axis, value) in coordinates.iter().enumerate() {
            let dims = value.dims();
            let data = Real::new(value)?;
            grid.coordinates[axis] = if dims.contains(&0) {
                Coordinate::Implicit
            } else if dims.as_ref() == shape {
                Coordinate::Full(data)
            } else if dims.as_ref() == [1, shape[XYZ[axis]]]
                || dims.as_ref() == [shape[XYZ[axis]], 1]
            {
                Coordinate::Vector(data)
            } else {
                return Err(
                    "X, Y and Z must be coordinate vectors or arrays the same size as V".into(),
                );
            };
        }
        Ok(grid)
    }

    fn point(&self, lattice: [usize; 3]) -> [f64; 3] {
        let index = linear(self.shape, lattice);
        std::array::from_fn(|axis| match self.coordinates[axis] {
            Coordinate::Implicit => (lattice[XYZ[axis]] + 1) as f64,
            Coordinate::Vector(v) => v.at(lattice[XYZ[axis]]),
            Coordinate::Full(v) => v.at(index),
        })
    }

    fn axis(&self, axis: usize, index: usize) -> f64 {
        match self.coordinates[axis] {
            Coordinate::Implicit => (index + 1) as f64,
            Coordinate::Vector(v) => v.at(index),
            Coordinate::Full(v) => {
                let mut lattice = [0; 3];
                lattice[XYZ[axis]] = index;
                v.at(linear(self.shape, lattice))
            }
        }
    }

    fn rectilinear(&self, e: &Engine) -> Result<()> {
        for (axis, coordinates) in self.coordinates.iter().enumerate() {
            if matches!(coordinates, Coordinate::Implicit) {
                continue;
            }
            let n = self.shape[XYZ[axis]];
            let ascending = self.axis(axis, 1) > self.axis(axis, 0);
            let mut previous = self.axis(axis, 0);
            for i in 1..n {
                if i.is_multiple_of(4096) {
                    e.check_cancel()?;
                }
                let next = self.axis(axis, i);
                if !next.is_finite()
                    || !previous.is_finite()
                    || next == previous
                    || (next > previous) != ascending
                {
                    return Err("X, Y and Z must have finite, strictly monotonic axes".into());
                }
                previous = next;
            }
            if let Coordinate::Full(v) = coordinates {
                let stride: usize = self.shape[..XYZ[axis]].iter().product();
                for i in 0..self.shape.iter().product() {
                    if i.is_multiple_of(4096) {
                        e.check_cancel()?;
                    }
                    if v.at(i) != self.axis(axis, (i / stride) % n) {
                        return Err("X, Y and Z must form a rectilinear meshgrid".into());
                    }
                }
            }
        }
        Ok(())
    }

    fn cell(&self, axis: usize, coordinate: f64) -> Option<(usize, f64)> {
        let n = self.shape[XYZ[axis]];
        let (first, last) = (self.axis(axis, 0), self.axis(axis, n - 1));
        if !coordinate.is_finite() || coordinate < first.min(last) || coordinate > first.max(last) {
            return None;
        }
        if matches!(self.coordinates[axis], Coordinate::Implicit) {
            let offset = coordinate - 1.;
            let lo = (offset as usize).min(n - 2);
            return Some((lo, offset - lo as f64));
        }
        let ascending = first < last;
        let (mut lo, mut hi) = (0, n - 1);
        while hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            if (self.axis(axis, mid) <= coordinate) == ascending {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        Some((
            lo,
            fraction(self.axis(axis, lo), self.axis(axis, hi), coordinate),
        ))
    }

    fn gradient(&self, lattice: [usize; 3], axis: usize) -> f64 {
        let dimension = XYZ[axis];
        let i = lattice[dimension];
        let sample = |at| {
            let mut p = lattice;
            p[dimension] = at;
            (self.axis(axis, at), self.values.at(linear(self.shape, p)))
        };
        let slope = |(x0, v0): (f64, f64), (x1, v1): (f64, f64)| (v1 - v0) / (x1 - x0);
        let center = sample(i);
        if i == 0 {
            return slope(center, sample(1));
        }
        if i + 1 == self.shape[dimension] {
            return slope(sample(i - 1), center);
        }
        let (prev, next) = (sample(i - 1), sample(i + 1));
        let t = fraction(prev.0, next.0, center.0);
        // Derivative of the local quadratic on an irregular grid.
        lerp(slope(prev, center), slope(center, next), t)
    }
}

fn linear(shape: [usize; 3], [row, col, page]: [usize; 3]) -> usize {
    row + shape[0] * (col + shape[1] * page)
}

fn fraction(a: f64, b: f64, value: f64) -> f64 {
    let span = b - a;
    if span.is_finite() {
        (value - a) / span
    } else {
        (value * 0.5 - a * 0.5) / (b * 0.5 - a * 0.5)
    }
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    if t == 0. || a == b {
        a
    } else if t == 1. {
        b
    } else if (b - a).is_finite() {
        a + t * (b - a)
    } else {
        (1. - t) * a + t * b
    }
}

fn finite_range(e: &Engine, input: Real<'_>, n: usize, step: usize) -> Result<[f64; 2]> {
    let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
    for (i, index) in (0..n).step_by(step).enumerate() {
        if i.is_multiple_of(4096) {
            e.check_cancel()?;
        }
        let v = input.at(index);
        if v.is_finite() {
            min = min.min(v);
            max = max.max(v);
        }
    }
    Ok([min, max])
}

/// Pick a central bin in the populated portion of the histogram, following
/// Octave's sampled, 100-bin heuristic (MathWorks documents histogram selection).
fn default_level(e: &Engine, grid: &Grid<'_>) -> Result<f64> {
    let n = grid.shape.iter().product::<usize>();
    let step = if n > 20_000 { n / 10_000 } else { 1 };
    let [min, max] = finite_range(e, grid.values, n, step)?;
    if min >= max {
        return Ok(if min.is_finite() { min } else { 0. });
    }
    let mut bins = [0; 100];
    for (i, index) in (0..n).step_by(step).enumerate() {
        if i.is_multiple_of(4096) {
            e.check_cancel()?;
        }
        let v = grid.values.at(index);
        if v.is_finite() {
            // Octave assigns samples on a bin boundary to the lower bin.
            let bin = (fraction(min, max, v) * 100.).ceil() as usize;
            bins[bin.saturating_sub(1).min(99)] += 1;
        }
    }
    let start = if bins[..2]
        .iter()
        .any(|&b| b as f64 > n.div_ceil(step) as f64 / 10.)
    {
        2
    } else {
        0
    };
    let peak = *bins[start..].iter().max().unwrap();
    let occupied: Vec<_> = (start..100).filter(|&i| bins[i] * 50 >= peak).collect();
    let bin = if 100 - start - occupied.len() >= 90 {
        start + (100 - start) / 2 - 1
    } else {
        occupied[(occupied.len() / 2).saturating_sub(1)]
    };
    Ok(lerp(min, max, (bin as f64 + 0.5) / 100.))
}

#[derive(Clone, Copy)]
struct Vertex {
    point: [f64; 3],
    color: f64,
}

#[derive(Default)]
struct Mesh {
    vertices: Vec<Vertex>,
    faces: Vec<[usize; 3]>,
}

fn mesh_budget(e: &Engine, faces: usize, vertices: usize) -> Result<()> {
    if faces > MAX_FACES {
        return Err(format!("Isosurface geometry exceeds {MAX_FACES} triangles"));
    }
    if vertices > MAX_VERTICES {
        return Err(format!(
            "Isosurface geometry exceeds {MAX_VERTICES} vertices"
        ));
    }
    // Allow for Vec capacity, edge hash entries, the index remap and final
    // column-major matrices. The limits apply to the actual intersected region.
    e.preflight_elements(12 * faces + 24 * vertices)
}

#[derive(Clone, Copy)]
struct CapSlab {
    dimension: usize,
    above: bool,
    previous_faces: usize,
    previous_vertices: usize,
}

fn march(
    e: &Engine,
    shape: [usize; 3],
    level: f64,
    share: bool,
    slab: Option<CapSlab>,
    sample: impl Fn([usize; 3]) -> f64,
    vertex: impl Fn([usize; 3], [usize; 3], f64) -> Vertex,
) -> Result<Mesh> {
    let mut vertices: Vec<(usize, Vertex)> = Vec::new();
    let mut edges = HashMap::new();
    let mut edge_vertices: [Vec<usize>; 12] = std::array::from_fn(|_| Vec::new());
    let mut faces: [Vec<[usize; 3]>; 5] = std::array::from_fn(|_| Vec::new());
    let mut face_count = 0;
    let is_pad = |p: [usize; 3]| slab.is_some_and(|s| p[s.dimension] == 0);
    let budget = |faces, vertices| {
        mesh_budget(
            e,
            faces + slab.map_or(0, |s| s.previous_faces),
            vertices + slab.map_or(0, |s| s.previous_vertices),
        )
    };
    let cells = shape.map(|n| n - 1);
    for page in 0..cells[2] {
        for col in 0..cells[1] {
            for row in 0..cells[0] {
                let cell = linear(cells, [row, col, page]);
                if cell.is_multiple_of(4096) {
                    e.check_cancel()?;
                }
                let corners = CORNERS.map(|[r, c, p]| [row + r, col + c, page + p]);
                let values = corners.map(&sample);
                let case = values.iter().enumerate().fold(0, |mask, (i, &v)| {
                    let high = match slab {
                        Some(s) if is_pad(corners[i]) => !s.above,
                        _ => v > level,
                    };
                    mask | (usize::from(high) << i)
                });
                let triangles = TRIANGLES[case];
                if triangles.is_empty() {
                    continue;
                }
                let mut crossing = 0u16;
                for &edge in triangles.iter().flatten() {
                    crossing |= 1 << edge;
                }
                let mut local = [None; 12];
                for edge in 0..12 {
                    if crossing & (1 << edge) == 0 {
                        continue;
                    }
                    let [i, j] = EDGES[edge];
                    let (a, b) = (values[i], values[j]);
                    if !a.is_finite() || !b.is_finite() {
                        continue;
                    }
                    let (p, q) = (corners[i], corners[j]);
                    let t = match (is_pad(p), is_pad(q)) {
                        // A transverse crossing keeps its virtual coordinate
                        // until deduplication, with exact-level hits at layer 1.
                        (true, false) => {
                            if b == level {
                                1.
                            } else {
                                0.5
                            }
                        }
                        (false, true) => {
                            if a == level {
                                0.
                            } else {
                                0.5
                            }
                        }
                        _ => fraction(a, b, level),
                    };
                    let (s, r) = (linear(shape, p), linear(shape, q));
                    let key = if t == 0. {
                        (s, s)
                    } else if t == 1. {
                        (r, r)
                    } else {
                        (s.min(r), s.max(r))
                    };
                    if share && let Some(&idx) = edges.get(&key) {
                        let (old_edge, old_vertex) = &mut vertices[idx];
                        if edge < *old_edge {
                            *old_edge = edge;
                            *old_vertex = vertex(p, q, t);
                            edge_vertices[edge].push(idx);
                        }
                        local[edge] = Some(idx);
                        continue;
                    }
                    let point = vertex(p, q, t);
                    if point.point.iter().all(|x| x.is_finite()) {
                        let idx = vertices.len();
                        budget(face_count, idx + 1)?;
                        vertices.push((edge, point));
                        edge_vertices[edge].push(idx);
                        if share {
                            edges.insert(key, idx);
                        }
                        local[edge] = Some(idx);
                    }
                }
                // Gather by triangle slot, then by cell, in Octave order.
                for (slot, triangle) in triangles.iter().enumerate() {
                    if let [Some(a), Some(b), Some(c)] = triangle.map(|edge| local[edge])
                        && (!share || (a != b && b != c && a != c))
                    {
                        face_count += 1;
                        budget(face_count, vertices.len())?;
                        faces[slot].push([a, b, c]);
                    }
                }
            }
        }
    }
    drop(edges);
    e.check_cancel()?;

    // Number vertices by edge slot, then by cell. Deduplication keeps the
    // earliest occurrence of each intersection in this ordering.
    let mut remap = vec![0; vertices.len()];
    let mut mesh = Mesh {
        vertices: Vec::with_capacity(vertices.len()),
        faces: Vec::with_capacity(face_count),
    };
    for (edge, indices) in edge_vertices.into_iter().enumerate() {
        for (i, old) in indices.into_iter().enumerate() {
            if i.is_multiple_of(4096) {
                e.check_cancel()?;
            }
            let (representative, vertex) = vertices[old];
            if edge == representative {
                remap[old] = mesh.vertices.len();
                mesh.vertices.push(vertex);
            }
        }
    }
    for (i, triangle) in faces.into_iter().flatten().enumerate() {
        if i.is_multiple_of(4096) {
            e.check_cancel()?;
        }
        mesh.faces.push(triangle.map(|v| remap[v]));
    }
    Ok(mesh)
}

fn interpolated_vertex(a: [f64; 3], b: [f64; 3], t: f64, ca: f64, cb: f64) -> Vertex {
    Vertex {
        point: std::array::from_fn(|i| lerp(a[i], b[i], t)),
        color: lerp(ca, cb, t),
    }
}

fn matrix_columns(
    e: &Engine,
    rows: usize,
    cols: usize,
    value: impl Fn(usize, usize) -> f64,
) -> Result<Matrix> {
    let mut data = Vec::with_capacity(Matrix::checked_len(rows, cols)?);
    for col in 0..cols {
        for row in 0..rows {
            if row.is_multiple_of(4096) {
                e.check_cancel()?;
            }
            data.push(value(row, col));
        }
    }
    Matrix::from_column_vec(rows, cols, data)
}

fn mesh_outputs(
    e: &mut Engine,
    mesh: &Mesh,
    colors: bool,
    surface_level: Option<f64>,
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    let faces = matrix_columns(e, mesh.faces.len(), 3, |i, j| (mesh.faces[i][j] + 1) as f64)?;
    let vertices = matrix_columns(e, mesh.vertices.len(), 3, |i, j| mesh.vertices[i].point[j])?;
    let c = if colors && count != 2 {
        Some(matrix_columns(e, mesh.vertices.len(), 1, |i, _| {
            mesh.vertices[i].color
        })?)
    } else if let Some(level) = surface_level.filter(|_| count == 0) {
        Some(Matrix::filled(mesh.vertices.len(), 1, level)?)
    } else {
        None
    };
    if count == 0 {
        let mut args = vec![
            Value::Text("Faces".into()),
            Value::Matrix(faces),
            Value::Text("Vertices".into()),
            Value::Matrix(vertices),
            Value::Text("FaceColor".into()),
            Value::Text(
                if surface_level.is_some() {
                    "flat"
                } else {
                    "interp"
                }
                .into(),
            ),
            Value::Text("EdgeColor".into()),
            Value::Text("none".into()),
        ];
        if let Some(c) = c {
            args.extend([Value::Text("FaceVertexCData".into()), Value::Matrix(c)]);
        }
        e.graphics_handle_call("patch", &args, out)?;
        if surface_level.is_some()
            && let Some(figure) = &mut e.figure
            && !figure.axes.hold
        {
            figure.axes.view = crate::plot::view3d::VIEW_3D;
        }
        return Ok(Vec::new());
    }
    if count == 1 {
        let mut names = vec!["faces".into(), "vertices".into()];
        let mut fields = vec![Arc::new(faces.into()), Arc::new(vertices.into())];
        if let Some(c) = c {
            names.push("facevertexcdata".into());
            fields.push(Arc::new(c.into()));
        }
        return Ok(vec![Value::Container(
            StructArray::from_slots(1, 1, names, fields, &e.cancel)?.into(),
        )]);
    }
    let mut outputs = vec![Value::Matrix(faces), Value::Matrix(vertices)];
    if count == 3 {
        outputs.push(Value::Matrix(match c {
            Some(c) => c,
            None => Matrix::filled(0, 0, 0.)?,
        }));
    }
    Ok(outputs)
}

fn option(value: &Value) -> bool {
    matches!(
        value,
        Value::Text(_) | Value::QuotedString(_) | Value::Container(Array::Character(_))
    )
}

fn inputs<'a>(e: &Engine, args: &'a [Value]) -> Result<(&'a [Value], Vec<String>)> {
    let split = args.iter().position(option).unwrap_or(args.len());
    let options = args[split..]
        .iter()
        .map(|value| Ok(e.text_value(value.clone())?.to_ascii_lowercase()))
        .collect::<Result<_>>()?;
    Ok((&args[..split], options))
}

fn scalar_level(value: &Value) -> Result<f64> {
    let level = Real::new(value)?.at(0);
    if !level.is_finite() {
        return Err("The isovalue must be finite".into());
    }
    Ok(level)
}

fn level(e: &Engine, grid: &Grid<'_>, value: Option<&Value>) -> Result<f64> {
    match value {
        Some(v) if v.dims().as_ref() == [1, 1] => scalar_level(v),
        Some(v) if !v.dims().contains(&0) => Err("The isovalue must be a scalar".into()),
        _ => default_level(e, grid),
    }
}

fn surface(
    e: &mut Engine,
    args: &[Value],
    options: &[String],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    let (coordinates, data) = match args.len() {
        1..=3 => (&[][..], args),
        4..=6 => args.split_at(3),
        _ => {
            return Err(
                "Use isosurface(V[, isovalue[, colors]]) or isosurface(X,Y,Z,V,...)".into(),
            );
        }
    };
    let grid = Grid::new(&data[0], coordinates)?;
    let (selection, colors) = match &data[1..] {
        [c] if !c.dims().contains(&0) && c.dims().as_ref() != [1, 1] => (None, Some(c)),
        rest => (rest.first(), rest.get(1)),
    };
    let colors = colors.filter(|v| !v.dims().contains(&0));
    if colors.is_some_and(|c| c.dims().as_ref() != grid.shape) {
        return Err("The color array must have the same size as V".into());
    }
    let colors = colors.map(Real::new).transpose()?;
    let mut share = true;
    let mut verbose = false;
    for text in options {
        match text.as_str() {
            "noshare" => share = false,
            "verbose" => verbose = true,
            _ => return Err(format!("Unknown isosurface option '{text}'")),
        }
    }
    let level = level(e, &grid, selection)?;
    if verbose {
        e.emit(
            out,
            format!("isosurface: extracting the surface at {level}"),
        )?;
    }
    let mesh = march(
        e,
        grid.shape,
        level,
        share,
        None,
        |i| grid.values.at(linear(grid.shape, i)),
        |p, q, t| {
            let color = |p| colors.map_or(0., |c| c.at(linear(grid.shape, p)));
            interpolated_vertex(grid.point(p), grid.point(q), t, color(p), color(q))
        },
    )?;
    if verbose {
        e.emit(
            out,
            format!(
                "isosurface: {} faces, {} vertices",
                mesh.faces.len(),
                mesh.vertices.len()
            ),
        )?;
    }
    mesh_outputs(e, &mesh, colors.is_some(), Some(level), count, out)
}

fn normals(
    e: &mut Engine,
    args: &[Value],
    options: &[String],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    let sign = match options {
        [] => -1.,
        [option] if option == "negate" => 1.,
        _ => return Err("isonormals supports only the 'negate' option".into()),
    };
    let (grid, target) = match args {
        [v, target] => (Grid::new(v, &[])?, target),
        [_, _, _, v, target] => (Grid::new(v, &args[..3])?, target),
        _ => return Err("Use isonormals(V,vertices) or isonormals(X,Y,Z,V,vertices)".into()),
    };
    grid.rectilinear(e)?;
    let is_patch = matches!(target, Value::Container(Array::Graphics(_)));
    let from_patch;
    let input = if is_patch {
        from_patch = e
            .graphics_handle_call(
                "get",
                &[target.clone(), Value::Text("Vertices".into())],
                out,
            )?
            .ok_or("isonormals expects a patch handle with indexed vertices")?;
        &from_patch
    } else {
        target
    };
    let dims = input.dims();
    if dims.len() != 2 || dims[1] != 3 {
        return Err("isonormals needs a three-column vertex matrix or an indexed patch".into());
    }
    let input = Real::new(input)?;
    let n = dims[0];
    e.preflight_elements(6 * n)?;
    let mut data = vec![0.; Matrix::checked_len(n, 3)?];
    for i in 0..n {
        if i.is_multiple_of(1024) {
            e.check_cancel()?;
        }
        let coords: [Option<(usize, f64)>; 3] =
            std::array::from_fn(|k| grid.cell(k, input.at(i + k * n)));
        let norm = if let [Some((col, tx)), Some((row, ty)), Some((page, tz))] = coords {
            let mut g = [0.; 3];
            for [r, c, p] in CORNERS {
                let w = (if r == 0 { 1. - ty } else { ty })
                    * (if c == 0 { 1. - tx } else { tx })
                    * (if p == 0 { 1. - tz } else { tz });
                if w != 0. {
                    for (axis, v) in g.iter_mut().enumerate() {
                        *v += w * grid.gradient([row + r, col + c, page + p], axis);
                    }
                }
            }
            let length = g[0].hypot(g[1]).hypot(g[2]);
            if length == 0. {
                g
            } else {
                g.map(|v| sign * (v / length))
            }
        } else {
            [f64::NAN; 3]
        };
        for (axis, v) in norm.into_iter().enumerate() {
            data[i + axis * n] = v;
        }
    }
    let normals = Value::Matrix(Matrix::from_column_vec(n, 3, data)?);
    if count == 1 {
        return Ok(vec![normals]);
    }
    if is_patch {
        e.graphics_handle_call(
            "set",
            &[target.clone(), Value::Text("VertexNormals".into()), normals],
            out,
        )?;
    }
    Ok(Vec::new())
}

const PLANES: [&str; 6] = ["xmin", "xmax", "ymin", "ymax", "zmin", "zmax"];

fn caps(
    e: &mut Engine,
    args: &[Value],
    options: &[String],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    let (coordinates, data) = match args.len() {
        1..=2 => (&[][..], args),
        4..=5 => args.split_at(3),
        _ => return Err("Use isocaps(V,isovalue) or isocaps(X,Y,Z,V,isovalue)".into()),
    };
    let grid = Grid::new(&data[0], coordinates)?;
    let mut above = None;
    let mut selected = None;
    for text in options {
        match text.as_str() {
            "above" | "below" if above.is_none() => above = Some(text == "above"),
            "all" if selected.is_none() => selected = Some(0..6),
            plane if selected.is_none() => {
                let i = PLANES
                    .iter()
                    .position(|&name| name == plane)
                    .ok_or_else(|| format!("Unknown isocaps option '{plane}'"))?;
                selected = Some(i..i + 1);
            }
            _ => return Err(format!("Invalid or repeated isocaps option '{text}'")),
        }
    }
    grid.rectilinear(e)?;
    let level = level(e, &grid, data.get(1))?;
    let above = above.unwrap_or(true);
    let mut mesh = Mesh::default();
    for plane in selected.unwrap_or(0..6) {
        let axis = plane / 2;
        let dimension = XYZ[axis];
        let n = grid.shape[dimension];
        let ascending = grid.axis(axis, 0) < grid.axis(axis, n - 1);
        let end = if ascending == (plane % 2 == 0) {
            0
        } else {
            n - 1
        };
        let boundary = grid.axis(axis, end);
        let boundary_index = |mut lattice: [usize; 3]| {
            lattice[dimension] = end;
            lattice
        };
        let field = |p| grid.values.at(linear(grid.shape, boundary_index(p)));
        let mut shape = grid.shape;
        shape[dimension] = 2;

        // March a one-cell slab with exterior padding. Its transverse edge
        // intersections use the same samples and interpolation as the main surface.
        // Keep the virtual layer coordinate until vertices have been shared.
        let cap = march(
            e,
            shape,
            level,
            true,
            Some(CapSlab {
                dimension,
                above,
                previous_faces: mesh.faces.len(),
                previous_vertices: mesh.vertices.len(),
            }),
            field,
            |p, q, t| {
                let point = |p: [usize; 3]| {
                    let mut xyz = grid.point(boundary_index(p));
                    xyz[axis] = p[dimension] as f64;
                    xyz
                };
                interpolated_vertex(point(p), point(q), t, field(p), field(q))
            },
        )?;
        let offset = mesh.vertices.len();
        mesh_budget(
            e,
            mesh.faces.len() + cap.faces.len(),
            offset + cap.vertices.len(),
        )?;
        for (i, mut v) in cap.vertices.into_iter().enumerate() {
            if i.is_multiple_of(4096) {
                e.check_cancel()?;
            }
            v.point[axis] = boundary;
            mesh.vertices.push(v);
        }
        // The virtual pad lies at the low side. Orient the cap outward.
        let reverse_axes = (0..3)
            .filter(|&other| other != axis)
            .filter(|&other| grid.axis(other, 0) > grid.axis(other, grid.shape[XYZ[other]] - 1))
            .count()
            % 2
            == 1;
        for (i, mut face) in cap.faces.into_iter().enumerate() {
            if i.is_multiple_of(4096) {
                e.check_cancel()?;
            }
            if (!above) ^ (plane % 2 == 1) ^ reverse_axes {
                face.swap(1, 2);
            }
            mesh.faces.push(face.map(|i| i + offset));
        }
    }
    mesh_outputs(e, &mesh, true, None, count, out)
}

pub(in crate::engine) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if count > if name == "isonormals" { 1 } else { 3 } {
        return Err(format!("{name}: too many outputs"));
    }
    if args.len() > 8 {
        return Err(format!("{name}: too many inputs"));
    }
    e.check_cancel()?;
    let (args, options) = inputs(e, args)?;
    let values = match name {
        "isosurface" => surface(e, args, &options, count, out),
        "isonormals" => normals(e, args, &options, count, out),
        "isocaps" => caps(e, args, &options, count, out),
        _ => unreachable!(),
    };
    values.map_err(|error| {
        if error == "Cancelled" || error.starts_with(name) {
            error
        } else {
            format!("{name}: {error}")
        }
    })
}
