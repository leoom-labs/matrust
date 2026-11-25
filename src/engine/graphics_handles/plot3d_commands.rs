//! 3-D line/scatter data (`plot3`, `scatter3`) and the `view` command. The
//! argument loop, LineSpec and Name,Value handling are the shared `plot` path.
use super::Value;
use crate::plot::{MAX_PLOT_SERIES, MAX_POINTS, PlotKind, Series, view3d};
use crate::{Matrix, Result};

/// One X,Y,Z triplet as line series: vectors of one length give one line;
/// matrices of one size give one line per column; vectors next to matrices
/// match their rows (one line per column) or else their columns (one line per
/// row), as `plot` does.
pub(in crate::engine) fn series_from_triplet(
    name: &str,
    data: [Result<&Matrix>; 3],
    kind: PlotKind,
) -> Result<Vec<Series>> {
    let [x, y, z] = data;
    let arrays = [x?, y?, z?];
    let vector = |m: &Matrix| m.rows() <= 1 || m.cols() <= 1;
    let matrices: Vec<&Matrix> = arrays.iter().copied().filter(|m| !vector(m)).collect();
    let (count, length, by_column) = if let Some(first) = matrices.first() {
        let (rows, cols) = (first.rows(), first.cols());
        if matrices
            .iter()
            .any(|m| m.rows() != rows || m.cols() != cols)
        {
            return Err(format!("{name}: X, Y and Z matrices must be the same size"));
        }
        let vectors: Vec<usize> = arrays
            .iter()
            .filter(|m| vector(m))
            .map(|m| m.len())
            .collect();
        if vectors.iter().all(|&n| n == rows) {
            (cols, rows, true)
        } else if vectors.iter().all(|&n| n == cols) {
            (rows, cols, false)
        } else {
            return Err(format!(
                "{name}: vector lengths must match the rows or columns of the X, Y and Z matrices"
            ));
        }
    } else {
        let n = arrays[0].len();
        if arrays.iter().any(|m| m.len() != n) {
            return Err(format!(
                "{name}: X, Y and Z must be vectors of the same length"
            ));
        }
        (1, n, true)
    };
    if count > MAX_PLOT_SERIES {
        return Err(format!("{name}: data exceeds series storage budget"));
    }
    if count.saturating_mul(length) > MAX_POINTS {
        return Err(format!("{name}: data exceeds {MAX_POINTS} points"));
    }
    let value = |m: &Matrix, line: usize, k: usize| {
        if vector(m) {
            m.data()[k]
        } else if by_column {
            m.get(k, line)
        } else {
            m.get(line, k)
        }
    };
    Ok((0..count)
        .map(|line| {
            let points: Vec<[f64; 2]> = (0..length)
                .map(|k| [value(arrays[0], line, k), value(arrays[1], line, k)])
                .collect();
            let z: std::sync::Arc<[f64]> = (0..length).map(|k| value(arrays[2], line, k)).collect();
            let mut series = Series::new(points, kind);
            series.z = z;
            series
        })
        .collect())
}

/// `view(az, el)`, `view([az el])`, `view(2)`, `view(3)`.
pub(in crate::engine) fn parse_view(args: &[Value]) -> Result<[f64; 2]> {
    let numbers = |value: &Value| -> Result<Vec<f64>> {
        match value {
            Value::Matrix(m) if m.rows() <= 1 || m.cols() <= 1 => Ok(m.data().to_vec()),
            // `view 3`, `view 30 45`: command syntax passes char words.
            Value::Text(s) => s
                .trim()
                .parse::<f64>()
                .map(|n| vec![n])
                .map_err(|_| "view requires real numeric angles".into()),
            _ => Err("view requires real numeric angles".into()),
        }
    };
    let view = match args {
        [one] => match numbers(one)?.as_slice() {
            [2.] => view3d::VIEW_2D,
            [3.] => view3d::VIEW_3D,
            [az, el] => [*az, *el],
            [_] => return Err("view(n) accepts only 2 or 3".into()),
            [_, _, _] => {
                return Err(
                    "view([x y z]) camera positions are not supported yet; use view(az,el)".into(),
                );
            }
            _ => return Err("Use view(az,el), view([az el]), view(2) or view(3)".into()),
        },
        [az, el] => match (numbers(az)?.as_slice(), numbers(el)?.as_slice()) {
            ([az], [el]) => [*az, *el],
            _ => return Err("view(az,el) requires two real scalars".into()),
        },
        _ => return Err("Use view(az,el), view([az el]), view(2) or view(3)".into()),
    };
    view3d::validate_view(view)?;
    Ok(view)
}

/// `[az, el] = view`: split the 1-by-2 query result.
pub(in crate::engine) fn view_outputs(value: Value, count: usize) -> Result<Vec<Value>> {
    let Value::Matrix(view) = value else {
        return Err("view did not return angles".into());
    };
    if count > 2 {
        return Err("Too many output arguments.".into());
    }
    if view.len() != 2 {
        return Err("view did not return two angles".into());
    }
    Ok(view
        .data()
        .iter()
        .map(|&angle| Value::Matrix(Matrix::scalar(angle)))
        .collect())
}

/// A 1-by-2 row of the axes view.
pub(in crate::engine) fn view_value(view: [f64; 2]) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_slice(1, 2, &view)?))
}
