//! Standard neural time series: rows are signals, columns are timesteps.
use super::*;
pub(super) const NAMES: &[&str] = &["tonndata", "fromnndata", "con2seq", "seq2con"];
fn admit(e: &Engine, slots: usize, elements: usize) -> Result<()> {
    if slots > crate::script_containers::MAX_CELL_SLOTS
        || slots
            .saturating_mul(512)
            .saturating_add(elements.saturating_mul(16))
            > e.limits.memory_bytes
    {
        return Err("network: time-series cell workspace limit exceeded".into());
    }
    e.check_cancel()
}
pub(super) fn read(e: &Engine, v: &Value) -> Result<Vec<Vec<Matrix>>> {
    let Value::Container(Array::Cell(c)) = v else {
        return Err("network: time series must be a cell array".into());
    };
    admit(e, c.len(), 0)?;
    let mut rows = vec![Vec::with_capacity(c.cols()); c.rows()];
    for t in 0..c.cols() {
        if t.is_multiple_of(1024) {
            e.check_cancel()?;
        }
        for (r, row) in rows.iter_mut().enumerate() {
            let m = matrix(
                e,
                &Value::from(c.get_linear(r + t * c.rows())?.as_ref().clone()),
            )?;
            if m.data().iter().any(|x| !x.is_finite()) {
                return Err("network: time series must be finite".into());
            }
            if row
                .first()
                .is_some_and(|p: &Matrix| (p.rows(), p.cols()) != (m.rows(), m.cols()))
            {
                return Err("network: time series sample dimensions must agree".into());
            }
            row.push(m);
        }
    }
    Ok(rows)
}
pub(super) fn value(e: &Engine, rows: &[Vec<Matrix>], cols: usize) -> Result<Value> {
    admit(e, rows.len().saturating_mul(cols), 0)?;
    if rows.len() > crate::script_containers::MAX_CELL_SLOTS
        || cols > crate::script_containers::MAX_CELL_SLOTS
    {
        return Err("network: time-series dimension limit exceeded".into());
    }
    let mut v = Vec::with_capacity(rows.len() * cols);
    for t in 0..cols {
        if t.is_multiple_of(1024) {
            e.check_cancel()?;
        }
        for row in rows {
            v.push(Array::Numeric(row[t].clone()));
        }
    }
    Ok(Value::from(cell(rows.len(), cols, v, &e.cancel)?))
}
fn transpose(m: &Matrix) -> Result<Matrix> {
    Matrix::from_column_vec(
        m.cols(),
        m.rows(),
        (0..m.rows())
            .flat_map(|r| (0..m.cols()).map(move |c| m.data()[r + c * m.rows()]))
            .collect(),
    )
}
pub(super) fn call(e: &Engine, name: &str, args: &[Value]) -> Result<Vec<Value>> {
    if name == "con2seq" && args.len() == 2 {
        let rows = read(e, &args[0])?;
        let steps = size(matrix(e, &args[1])?.as_scalar()?, false)?;
        admit(e, rows.len().saturating_mul(steps), 0)?;
        if rows
            .iter()
            .any(|r| r.len() != 1 || !r[0].cols().is_multiple_of(steps))
        {
            return Err("con2seq: expected N-by-1 cells with column counts divisible by TS".into());
        }
        let mut output = Vec::with_capacity(rows.len());
        for row in rows {
            let m = &row[0];
            let width = m.cols() / steps;
            let mut samples = Vec::with_capacity(steps);
            for t in 0..steps {
                if t.is_multiple_of(1024) {
                    e.check_cancel()?;
                }
                samples.push(Matrix::from_column_slice(
                    m.rows(),
                    width,
                    &m.data()[t * width * m.rows()..(t + 1) * width * m.rows()],
                )?);
            }
            output.push(samples);
        }
        return Ok(vec![value(e, &output, steps)?]);
    }
    if name == "con2seq" || name == "tonndata" {
        arity(name, args, if name == "con2seq" { 1 } else { 3 })?;
        let column = name == "con2seq" || matrix(e, &args[1])?.as_scalar()? != 0.;
        let time = name == "tonndata" && matrix(e, &args[2])?.as_scalar()? != 0.;
        let was_matrix = !matches!(&args[0], Value::Container(Array::Cell(_)));
        let rows = if was_matrix {
            vec![vec![matrix(e, &args[0])?]]
        } else {
            read(e, &args[0])?
        };
        let slots = rows
            .iter()
            .flatten()
            .map(|m| {
                if time {
                    1
                } else if column {
                    m.cols()
                } else {
                    m.rows()
                }
            })
            .fold(0usize, usize::saturating_add);
        let elements = rows
            .iter()
            .flatten()
            .map(Matrix::len)
            .fold(0usize, usize::saturating_add);
        admit(e, slots, elements)?;
        let mut out = Vec::new();
        for row in rows {
            let mut v = Vec::new();
            for mut m in row {
                if !column {
                    m = transpose(&m)?;
                }
                if time {
                    v.push(m);
                } else {
                    for c in 0..m.cols() {
                        if c.is_multiple_of(1024) {
                            e.check_cancel()?;
                        }
                        v.push(Matrix::from_column_slice(
                            m.rows(),
                            1,
                            &m.data()[c * m.rows()..(c + 1) * m.rows()],
                        )?);
                    }
                }
            }
            out.push(v);
        }
        let cols = out.first().map_or(0, Vec::len);
        if out.iter().any(|r| r.len() != cols) {
            return Err("tonndata: inconsistent time lengths".into());
        }
        let y = value(e, &out, cols)?;
        if name == "con2seq" {
            Ok(vec![y])
        } else {
            Ok(vec![y, Value::Logical(LogicalMatrix::scalar(was_matrix))])
        }
    } else {
        arity(name, args, if name == "seq2con" { 1 } else { 4 })?;
        let rows = read(e, &args[0])?;
        let column = name == "seq2con" || matrix(e, &args[2])?.as_scalar()? != 0.;
        let time = name == "fromnndata" && matrix(e, &args[3])?.as_scalar()? != 0.;
        let was_matrix = name == "fromnndata" && matrix(e, &args[1])?.as_scalar()? != 0.;
        let mut out = Vec::new();
        for row in rows {
            if time {
                out.push(
                    row.into_iter()
                        .map(|m| if column { Ok(m) } else { transpose(&m) })
                        .collect::<Result<Vec<_>>>()?,
                );
            } else {
                let nr = row.first().map_or(0, Matrix::rows);
                let nc = row.iter().map(Matrix::cols).sum();
                let elements = Matrix::checked_len(nr, nc)?;
                admit(e, 0, elements)?;
                let m = Matrix::from_column_vec(
                    nr,
                    nc,
                    row.iter().flat_map(|m| m.data().iter().copied()).collect(),
                )?;
                out.push(vec![if column { m } else { transpose(&m)? }]);
            }
        }
        if was_matrix && out.len() == 1 && out[0].len() == 1 {
            Ok(vec![Value::Matrix(out.remove(0).remove(0))])
        } else {
            let cols = out.first().map_or(0, Vec::len);
            Ok(vec![value(e, &out, cols)?])
        }
    }
}
