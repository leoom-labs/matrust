//! Plane and surface sampling for volume graphics.
use super::super::{Engine, Execution, Value};
use crate::{Array, GraphicsArray, Matrix, Result, plot::MAX_SERIES};

const MAX_SAMPLES: usize = 1_000_000;

pub(super) struct SampledSlice {
    pub xyz: [Matrix; 3],
    pub values: Matrix,
}

fn matrix(e: &Engine, v: &Value) -> Result<Matrix> {
    v.clone().matrix(&e.cancel)?.with_dims(&v.dims())
}

pub(super) fn split_method<'a>(
    who: &str,
    args: &'a [Value],
) -> Result<(&'a [Value], Option<&'a Value>)> {
    if let Some((method @ Value::Text(s), rest)) = args.split_last() {
        if !["linear", "nearest", "cubic"]
            .iter()
            .any(|m| s.eq_ignore_ascii_case(m))
        {
            return Err(format!("{who}: method must be linear, nearest or cubic"));
        }
        Ok((rest, Some(method)))
    } else {
        Ok((args, None))
    }
}

fn axis_coordinates(
    e: &Engine,
    who: &str,
    grids: &[Value],
    dims: &[usize],
) -> Result<[Vec<f64>; 3]> {
    let mut axes = [Vec::new(), Vec::new(), Vec::new()];
    for (a, axis) in axes.iter_mut().enumerate() {
        let n = dims[[1, 0, 2][a]];
        *axis = if grids.is_empty() {
            (1..=n).map(|i| i as f64).collect()
        } else {
            let g = matrix(e, &grids[a])?;
            if g.dims().len() == 2 && (g.rows() == 1 || g.cols() == 1) && g.len() == n {
                g.data().to_vec()
            } else if g.dims().as_ref() == dims {
                let stride = [dims[0], 1, dims[0] * dims[1]][a];
                (0..n).map(|i| g.data()[i * stride]).collect()
            } else {
                return Err(format!("{who}: grid must match V"));
            }
        };
    }
    Ok(axes)
}

fn plane_grid(axes: &[Vec<f64>; 3], axis: usize, position: f64) -> Result<[Matrix; 3]> {
    let (rows, cols) = match axis {
        0 => (&axes[1], &axes[2]),
        1 => (&axes[0], &axes[2]),
        _ => (&axes[1], &axes[0]),
    };
    let n = rows.len() * cols.len();
    let mut xyz = [
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
    ];
    for &col in cols {
        for &row in rows {
            let point = match axis {
                0 => [position, row, col],
                1 => [row, position, col],
                _ => [col, row, position],
            };
            for (coordinate, value) in xyz.iter_mut().zip(point) {
                coordinate.push(value);
            }
        }
    }
    let [x, y, z] = xyz;
    Ok([
        Matrix::from_column_vec(rows.len(), cols.len(), x)?,
        Matrix::from_column_vec(rows.len(), cols.len(), y)?,
        Matrix::from_column_vec(rows.len(), cols.len(), z)?,
    ])
}

/// Check requested planes before allocating their grids or calling interp3.
pub(super) fn sample(
    e: &mut Engine,
    who: &str,
    args: &[Value],
    method: Option<&Value>,
    plane_limit: usize,
) -> Result<Vec<SampledSlice>> {
    let (grids, volume, positions) = match args {
        [v, positions @ ..] if positions.len() == 3 => (&[][..], v, positions),
        [_, _, _, v, positions @ ..] if positions.len() == 3 => (&args[..3], v, positions),
        _ => {
            return Err(format!(
                "{who}: expected V and three slice positions, optionally preceded by X, Y, Z"
            ));
        }
    };
    let dims = volume.dims();
    if dims.len() != 3 || dims.iter().any(|&n| n < 2) {
        return Err(format!(
            "{who}: V must be a 3-D array with at least two points per axis"
        ));
    }
    let xyz = [
        matrix(e, &positions[0])?,
        matrix(e, &positions[1])?,
        matrix(e, &positions[2])?,
    ];
    if xyz.iter().any(|m| m.dims().len() != 2) {
        return Err(format!(
            "{who}: slice positions must be vectors or matrices"
        ));
    }
    let surface = xyz.iter().any(|m| m.rows() > 1 && m.cols() > 1);
    if surface && xyz.iter().any(|m| m.dims() != xyz[0].dims()) {
        return Err(format!(
            "{who}: surface coordinates must be matrices of equal size"
        ));
    }
    let count = if surface {
        1
    } else {
        xyz.iter().map(Matrix::len).sum()
    };
    if count > plane_limit {
        return Err(format!(
            "{who}: requested {count} slices, but only {plane_limit} fit within the figure's {MAX_SERIES}-series limit"
        ));
    }
    if xyz.iter().any(|m| m.data().iter().any(|v| !v.is_finite())) {
        return Err(format!("{who}: slice positions must be finite"));
    }
    let axes = axis_coordinates(e, who, grids, &dims)?;
    let total = if surface {
        xyz[0].len()
    } else {
        let [ny, nx, nz] = [dims[0], dims[1], dims[2]];
        xyz.iter()
            .zip([ny * nz, nx * nz, ny * nx])
            .map(|(m, points)| m.len().saturating_mul(points))
            .fold(0usize, usize::saturating_add)
    };
    if total > MAX_SAMPLES {
        return Err(format!(
            "{who}: requested slices exceed {MAX_SAMPLES} samples"
        ));
    }
    e.preflight_elements(total.saturating_mul(4))?;

    let mut query = grids.to_vec();
    query.push(volume.clone());
    let input_count = query.len();
    let nearest = Value::Text("nearest".into());
    let method = method.or_else(|| (who == "contourslice" && !surface).then_some(&nearest));
    let mut sampled = Vec::with_capacity(count);
    let mut interpolate = |xyz: [Matrix; 3]| -> Result<()> {
        e.check_cancel()?;
        query.truncate(input_count);
        query.extend(xyz.iter().cloned().map(Value::Matrix));
        query.extend(method.cloned());
        let values = super::data::interp(e, &query)?.remove(0);
        sampled.push(SampledSlice {
            xyz,
            values: matrix(e, &values)?,
        });
        Ok(())
    };
    if surface {
        interpolate(xyz)?;
    } else {
        for (axis, positions) in xyz.iter().enumerate() {
            for &position in positions.data() {
                interpolate(plane_grid(&axes, axis, position)?)?;
            }
        }
    }
    Ok(sampled)
}

pub(in crate::engine) fn call(
    e: &mut Engine,
    args: &[Value],
    out: &mut Execution,
) -> Result<Value> {
    let (args, method) = split_method("slice", args)?;
    let original_hold = e.figure.as_ref().is_some_and(|f| f.hold);
    let used = e.figure.as_ref().map_or(0, |f| {
        f.series_count() - if original_hold { 0 } else { f.series.len() }
    });
    let surfaces = sample(e, "slice", args, method, MAX_SERIES.saturating_sub(used))?;
    let mut handles = Vec::with_capacity(surfaces.len());
    for SampledSlice { xyz, values } in surfaces {
        let [x, y, z] = xyz;
        let input = [x, y, z, values].map(Value::Matrix);
        let result = super::super::graphics_handles::surface_call(e, "surf", &input, 1, out)?;
        if let Value::Container(Array::Graphics(h)) = result {
            handles.push(h.get_linear(0)?.clone());
        }
        if let Some(f) = e.figure.as_mut() {
            f.hold = true;
        }
    }
    if let Some(f) = e.figure.as_mut() {
        f.hold = original_hold;
    }
    Ok(Value::Container(
        GraphicsArray::from_shared(handles.len(), 1, handles, &e.cancel)?.into(),
    ))
}
