//! High-level indexed plots that render through the ordinary patch series.
use super::{Engine, GraphicsArray, GraphicsHandle, Matrix, Result, Value};
use crate::plot::{
    IndexedColor, IndexedPatch, LinePattern, MAX_PATCH_FACES, PatchFaces, PlotKind, Series,
    colormap, coordinates::GeometryCost,
};

const MAX_VERTICES: usize = 3 * MAX_PATCH_FACES;

fn matrix<'a>(value: &'a Value, name: &str) -> Result<&'a Matrix> {
    match value {
        Value::Matrix(m) if m.ndims() == 2 => Ok(m),
        _ => Err(format!("{name}: expected a real 2-D numeric matrix")),
    }
}

fn tetra_faces(e: &Engine, tetra: &Matrix) -> Result<Matrix> {
    let n = tetra.rows();
    let mut faces = Vec::with_capacity(12 * n);
    // For each tetrahedron, store its four oriented triangular faces in order.
    let corners = [[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]];
    for col in 0..3 {
        for row in 0..n {
            if row.is_multiple_of(2048) {
                e.check_cancel()?;
            }
            faces.extend(corners.iter().map(|face| tetra.get(row, face[col])));
        }
    }
    Matrix::from_column_vec(4 * n, 3, faces)
}

fn vertex_table(e: &Engine, coordinates: [&Matrix; 3]) -> Result<Matrix> {
    let n = coordinates[0].len();
    e.preflight_elements(3 * n)?;
    let mut vertices = Vec::with_capacity(3 * n);
    for matrix in coordinates {
        for chunk in matrix.data().chunks(8192) {
            e.check_cancel()?;
            vertices.extend_from_slice(chunk);
        }
    }
    Matrix::from_column_vec(n, 3, vertices)
}

pub(super) fn draw(e: &mut Engine, name: &str, args: &[Value]) -> Result<Value> {
    let tetra = name == "tetramesh";
    let wire = name == "trimesh";
    let (connectivity, coordinates) = match args {
        [t, coords @ ..]
            if (tetra && coords.len() == 1) || (!tetra && matches!(coords.len(), 3 | 4)) =>
        {
            (matrix(t, name)?, coords)
        }
        _ if tetra => return Err("Use tetramesh(T,X)".into()),
        _ => return Err(format!("Use {name}(T,X,Y,Z) or {name}(T,X,Y,Z,C)")),
    };
    let width = if tetra { 4 } else { 3 };
    if connectivity.cols() != width || connectivity.rows() == 0 {
        return Err(format!(
            "{name}: T must have {width} columns and at least one row"
        ));
    }
    let face_count = connectivity.rows() * if tetra { 4 } else { 1 };
    if face_count > MAX_PATCH_FACES {
        return Err(format!(
            "{name}: a figure holds at most {MAX_PATCH_FACES} patch faces"
        ));
    }
    let previous = e.figure.clone();
    let mut figure = e.surface_target();
    super::plot_carry_over(previous.as_ref(), &mut figure, &mut [])?;
    let defaults = e.graphics_plot_axes_defaults(&mut figure)?;
    figure
        .geometry_ledger()?
        .add(GeometryCost::dense_face_allocation(3 * face_count, 3)?)?;

    let used_vertices = figure
        .all_axes()
        .flat_map(|(_, ax)| &ax.series)
        .filter_map(|series| series.indexed_patch.as_ref())
        .map(|patch| patch.vertices.rows())
        .sum::<usize>();
    let check_vertex_count = |n: usize| {
        if n == 0 || n.saturating_add(used_vertices) > MAX_VERTICES {
            return Err(format!(
                "{name}: a figure holds 1 to {MAX_VERTICES} indexed vertices"
            ));
        }
        Ok(())
    };
    let (faces, vertices, colors) = if tetra {
        let vertices = matrix(&coordinates[0], name)?;
        if vertices.cols() != 3 {
            return Err("tetramesh: X must have three coordinate columns".into());
        }
        check_vertex_count(vertices.rows())?;
        e.preflight_elements(3 * face_count)?;
        (tetra_faces(e, connectivity)?, vertices.clone(), None)
    } else {
        let x = matrix(&coordinates[0], name)?;
        let y = matrix(&coordinates[1], name)?;
        let z = matrix(&coordinates[2], name)?;
        let c = coordinates.get(3).map(|c| matrix(c, name)).transpose()?;
        if x.len() != y.len() || x.len() != z.len() || c.is_some_and(|c| c.len() != z.len()) {
            return Err(format!(
                "{name}: X, Y, Z and C must have the same number of elements"
            ));
        }
        check_vertex_count(x.len())?;
        let c = c.unwrap_or(z).reshape(z.len(), 1)?;
        (connectivity.clone(), vertex_table(e, [x, y, z])?, Some(c))
    };
    let clim = match &colors {
        Some(colors) => {
            let mut low = f64::INFINITY;
            let mut high = f64::NEG_INFINITY;
            for chunk in colors.data().chunks(8192) {
                e.check_cancel()?;
                for &value in chunk {
                    if !value.is_finite() {
                        return Err(format!("{name}: C must be finite"));
                    }
                    low = low.min(value);
                    high = high.max(value);
                }
            }
            if let Some(clim) = figure.axes.extras.clim {
                clim
            } else if low == high {
                [low - 1., high + 1.]
            } else {
                [low, high]
            }
        }
        None => [0., 1.],
    };
    let map = figure.axes.extras.effective_colormap();
    let mut points = Vec::with_capacity(3 * face_count);
    let mut z = Vec::with_capacity(3 * face_count);
    let mut face_colors = colors.as_ref().map(|_| Vec::with_capacity(face_count));
    for row in 0..face_count {
        if row.is_multiple_of(2048) {
            e.check_cancel()?;
        }
        for col in 0..3 {
            let index = faces.get(row, col);
            if !(1. ..=vertices.rows() as f64).contains(&index) || index.fract() != 0. {
                return Err(format!(
                    "{name}: T must contain valid one-based vertex indices"
                ));
            }
            let index = index as usize - 1;
            let [px, py, pz] = std::array::from_fn(|axis| vertices.get(index, axis));
            if !px.is_finite() || !py.is_finite() || !pz.is_finite() {
                return Err(format!("{name}: vertex coordinates must be finite"));
            }
            points.push([px, py]);
            z.push(pz);
            if col == 0
                && let (Some(colors), Some(output)) = (&colors, &mut face_colors)
            {
                output.push(colormap::scaled_rgb(&map, clim, colors.data()[index]));
            }
        }
    }
    let mut series = Series::new(points, PlotKind::Patch);
    series.patch = Some(PatchFaces::new(3)?);
    series.z = z.into();
    series.pattern = LinePattern::Solid;
    series.face_edge_color = Some([0; 3]);
    e.apply_patch_defaults(&mut series, &figure.identity)?;
    if wire {
        series.face_color_none = true;
        series.face_edge_colors = face_colors.map(Into::into);
    } else if let Some(colors) = face_colors {
        series.color = colors[0];
        series.explicit_color = true;
        series.face_colors = Some(colors.into());
    }
    series.indexed_patch = Some(IndexedPatch {
        faces,
        vertices,
        colors,
        normals: None,
        face_color: if wire {
            IndexedColor::None
        } else if series.face_colors.is_some() {
            IndexedColor::Flat
        } else {
            IndexedColor::Rgb(series.color)
        },
        color_range: None,
        mapped_with: None,
    });
    let token = GraphicsHandle::series(&figure.identity, &figure.axes.identity, &series.identity);
    figure.add(vec![series])?;
    figure.prepare_legend_layout()?;
    figure.validate()?;
    let result = GraphicsArray::scalar(token, &e.cancel)?;
    e.check_cancel()?;
    e.graphics_finish_axes_defaults(&figure, defaults);
    e.figure = Some(figure);
    Ok(Value::Container(result.into()))
}
