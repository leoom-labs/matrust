//! Channel cells are stored in output-by-input order, with outputs contiguous.
use super::*;
use crate::{engine::indexing::Indices, syntax::Expr};

const MAX_PORTS: usize = 4096;
// A rational model needs two cell grids plus names inside one bounded struct.
const MAX_CHANNELS: usize = 16_384;
const MAX_CONVERSION_WORK: usize = 200_000_000;

fn check_grid(rows: usize, cols: usize) -> Result<()> {
    if rows > MAX_PORTS || cols > MAX_PORTS || rows.saturating_mul(cols) > MAX_CHANNELS {
        return Err("LTI channel grid exceeds 4096 ports per axis or 16384 channels".into());
    }
    Ok(())
}

type CellPair = (CellArray, CellArray);

fn cells(v: &Value) -> Option<&CellArray> {
    match v {
        Value::Container(Array::Cell(c)) => Some(c),
        _ => None,
    }
}

fn grid_shape(a: &Value, b: &Value, operation: &str) -> Result<(usize, usize)> {
    let (a, b) = (cells(a), cells(b));
    let shape = a.or(b).map_or((1, 1), |c| (c.rows(), c.cols()));
    check_grid(shape.0, shape.1)?;
    for c in [a, b].into_iter().flatten() {
        if (c.rows(), c.cols()) != shape {
            return Err(format!(
                "{operation}: coefficient or root cells must have matching output-by-input dimensions"
            ));
        }
    }
    Ok(shape)
}

fn cell_entry(v: &Value, index: usize) -> Result<Value> {
    match cells(v) {
        Some(c) => Ok(c.get_linear(index)?.as_ref().clone().into()),
        None => Ok(v.clone()),
    }
}

fn grid_pair(
    cancel: &AtomicBool,
    (rows, cols): (usize, usize),
    mut channel: impl FnMut(usize) -> Result<(Array, Array)>,
) -> Result<CellPair> {
    check_grid(rows, cols)?;
    let len = rows * cols;
    let mut first = Vec::with_capacity(len);
    let mut second = Vec::with_capacity(len);
    let mut first_entries = 0;
    let mut second_entries = 0;
    for index in 0..len {
        crate::dsp::check_cancel(cancel)?;
        let (a, b) = channel(index)?;
        first_entries += a.len();
        second_entries += b.len();
        if first_entries.max(second_entries) > MAX_ENTRIES {
            return Err("LTI channel data exceeds 1048576 entries per field".into());
        }
        first.push(Arc::new(a));
        second.push(Arc::new(b));
    }
    Ok((
        CellArray::from_shared(rows, cols, first, cancel)?,
        CellArray::from_shared(rows, cols, second, cancel)?,
    ))
}

pub(super) fn tf_model(
    e: &Engine,
    n: &Value,
    d: &Value,
    variable: &str,
    decoded: bool,
) -> Result<Model> {
    let shape = grid_shape(n, d, "tf")?;
    if shape == (1, 1) {
        return tf_storage(n, d, variable);
    }
    if decoded && let (Some(n), Some(d)) = (cells(n), cells(d)) {
        return Ok(Model::TfArray(n.clone(), d.clone()));
    }
    let (n, d) = grid_pair(&e.cancel, shape, |i| {
        let Model::Tf(n, d) = tf_storage(&cell_entry(n, i)?, &cell_entry(d, i)?, variable)? else {
            unreachable!()
        };
        let (n, d) = public_tf(&n, &d, variable)?;
        Ok((n.into(), d.into()))
    })?;
    Ok(Model::TfArray(n, d))
}

pub(super) fn zpk_model(
    e: &Engine,
    z: &Value,
    p: &Value,
    k: &Value,
    decoded: bool,
) -> Result<Model> {
    let shape = grid_shape(z, p, "zpk")?;
    let k = real(k)?;
    if (k.rows(), k.cols()) != shape || k.is_nd() {
        return Err("zpk: gain matrix must match the output-by-input root cells".into());
    }
    if shape == (1, 1) {
        return zpk_channel(z, p, k.data()[0], true);
    }
    let (Some(z), Some(p)) = (cells(z), cells(p)) else {
        return Err("zpk: multi-channel models require both zero and pole cells".into());
    };
    if decoded {
        return Ok(Model::ZpkArray(z.clone(), p.clone(), k));
    }
    let mut remaining = MAX_CONVERSION_WORK;
    rational_map(&e.cancel, shape, "zpk", "", |i| {
        let m = zpk_channel(
            &z.get_linear(i)?.as_ref().clone().into(),
            &p.get_linear(i)?.as_ref().clone().into(),
            k.data()[i],
            false,
        )?;
        charge_work(&mut remaining, channel_work(&m, 2), "zpk")?;
        let Model::Zpk(z, p, _) = &m else {
            unreachable!()
        };
        poly_roots(z)?;
        poly_roots(p)?;
        Ok(m)
    })
}

fn zpk_channel(z: &Value, p: &Value, gain: f64, validate_pairs: bool) -> Result<Model> {
    let z = roots_input(z)?;
    let p = roots_input(p)?;
    if validate_pairs {
        poly_roots(&z)?;
        poly_roots(&p)?;
    }
    Ok(Model::Zpk(z, p, gain))
}

pub(super) fn channel(s: &Lti, index: usize) -> Result<Model> {
    let entry =
        |c: &CellArray| -> Result<Value> { Ok(c.get_linear(index)?.as_ref().clone().into()) };
    match &s.model {
        Model::TfArray(n, d) => tf_storage(&entry(n)?, &entry(d)?, &s.variable),
        Model::ZpkArray(z, p, k) => zpk_channel(&entry(z)?, &entry(p)?, k.data()[index], false),
        _ => Err("Expected a rational LTI channel grid".into()),
    }
}

/// Read I/O metadata without decoding coefficient or root payloads.
pub(in crate::engine) fn io_shape(a: &Array) -> Option<(usize, usize)> {
    let kind = class(a)?;
    let Array::Struct(s) = a else { unreachable!() };
    let property = |name| s.get(s.field_index(name)?, 0).ok().map(AsRef::as_ref);
    match kind {
        "ss" => Some((property("C")?.rows(), property("B")?.cols())),
        "tf" => ["Numerator", "Denominator"]
            .into_iter()
            .find_map(|name| {
                if let Some(Array::Cell(c)) = property(name) {
                    Some((c.rows(), c.cols()))
                } else {
                    None
                }
            })
            .or(Some((1, 1))),
        "zpk" => property("K").map(|k| (k.rows(), k.cols())),
        _ => Some((1, 1)),
    }
}

fn selectors(
    e: &mut Engine,
    shape: (usize, usize),
    args: &[Expr],
    out: &mut Execution,
) -> Result<(Indices, Indices)> {
    let [row, col] = args else {
        return Err("LTI channel indexing requires two subscripts, sys(outputs,inputs)".into());
    };
    let (rows, _) = e.subscript(row, shape.0, out)?;
    let (cols, _) = e.subscript(col, shape.1, out)?;
    Ok((rows, cols))
}

fn names(e: &Engine, names: &Array, indices: &Indices) -> Result<Array> {
    let slots = (0..indices.len())
        .map(|i| match names {
            Array::Cell(c) => c.get_linear(indices.at(i)).cloned(),
            a => Ok(Arc::new(a.clone())),
        })
        .collect::<Result<_>>()?;
    Ok(CellArray::from_shared(indices.len(), 1, slots, &e.cancel)?.into())
}

fn gather(rows: usize, cols: usize, mut at: impl FnMut(usize, usize) -> Result<()>) -> Result<()> {
    for c in 0..cols {
        for r in 0..rows {
            at(r, c)?;
        }
    }
    Ok(())
}

fn pick_cells(e: &Engine, c: &CellArray, rows: &Indices, cols: &Indices) -> Result<CellArray> {
    let mut slots = Vec::with_capacity(rows.len() * cols.len());
    gather(rows.len(), cols.len(), |r, j| {
        slots.push(c.get_linear(c.rows() * cols.at(j) + rows.at(r))?.clone());
        Ok(())
    })?;
    CellArray::from_shared(rows.len(), cols.len(), slots, &e.cancel)
}

fn pick_matrix(
    e: &Engine,
    m: &Matrix,
    rows: Option<&Indices>,
    cols: Option<&Indices>,
) -> Result<Matrix> {
    let (ny, nu) = (
        rows.map_or(m.rows(), Indices::len),
        cols.map_or(m.cols(), Indices::len),
    );
    let mut data = Vec::with_capacity(ny * nu);
    gather(ny, nu, |r, c| {
        if data.len().is_multiple_of(1024) {
            e.check_cancel()?;
        }
        let r = rows.map_or(r, |rows| rows.at(r));
        let c = cols.map_or(c, |cols| cols.at(c));
        data.push(m.data()[m.rows() * c + r]);
        Ok(())
    })?;
    Matrix::from_column_vec(ny, nu, data)
}

pub(in crate::engine) fn index(
    e: &mut Engine,
    a: &Array,
    args: &[Expr],
    out: &mut Execution,
) -> Result<Option<Value>> {
    if !matches!(class(a), Some("tf" | "zpk" | "ss")) {
        return Ok(None);
    }
    let mut s = decode(e, a)?.ok_or("Invalid LTI model")?;
    let (nu, ny) = s.ports();
    // Keep the historical scalar object access for SISO values.
    if let [i] = args
        && (ny, nu) == (1, 1)
    {
        let (selection, _) = e.subscript(i, 1, out)?;
        if selection.len() == 1 {
            return Ok(Some(a.clone().into()));
        }
        return Err("LTI channel indexing requires output and input subscripts".into());
    }
    let (rows, cols) = selectors(e, (ny, nu), args, out)?;
    let shape = (rows.len(), cols.len());
    if shape == (ny, nu) && rows.contiguous() == Some(0) && cols.contiguous() == Some(0) {
        return Ok(Some(a.clone().into()));
    }
    if !matches!(s.model, Model::Ss(..)) {
        check_grid(shape.0, shape.1)?;
    }
    s.model = match &s.model {
        Model::Ss(a, b, c, d) => {
            ss_size_check(a.rows(), shape.0, shape.1)?;
            e.preflight_elements(ss_entries(a.rows(), shape.0, shape.1) - a.len())?;
            Model::Ss(
                a.clone(),
                pick_matrix(e, b, None, Some(&cols))?,
                pick_matrix(e, c, Some(&rows), None)?,
                pick_matrix(e, d, Some(&rows), Some(&cols))?,
            )
        }
        Model::TfArray(..) | Model::ZpkArray(..) if shape == (1, 1) => {
            channel(&s, ny * cols.at(0) + rows.at(0))?
        }
        Model::TfArray(n, d) => Model::TfArray(
            pick_cells(e, n, &rows, &cols)?,
            pick_cells(e, d, &rows, &cols)?,
        ),
        Model::ZpkArray(z, p, k) => Model::ZpkArray(
            pick_cells(e, z, &rows, &cols)?,
            pick_cells(e, p, &rows, &cols)?,
            pick_matrix(e, k, Some(&rows), Some(&cols))?,
        ),
        _ => rational_map(&e.cancel, shape, s.kind(), &s.variable, |_| {
            Ok(s.model.clone())
        })?,
    };
    s.input = names(e, &s.input, &cols)?;
    s.output = names(e, &s.output, &rows)?;
    if shape.0 == 0 || shape.1 == 0 {
        s.props.delay = [0.; 3];
        s.props.paths.clear();
    }
    encode(e, &s).map(Some)
}

pub(super) fn empty_rational(e: &Engine, kind: &str, (ny, nu): (usize, usize)) -> Result<Model> {
    let c = CellArray::empty(ny, nu, &e.cancel)?;
    if kind == "tf" {
        Ok(Model::TfArray(c.clone(), c))
    } else {
        Ok(Model::ZpkArray(c.clone(), c, Matrix::filled(ny, nu, 0.)?))
    }
}

pub(super) fn assign(
    e: &mut Engine,
    a: &Array,
    args: &[Expr],
    value: &Array,
    out: &mut Execution,
) -> Result<Array> {
    let mut s = decode(e, a)?.ok_or("Invalid LTI model")?;
    if s.kind() == "ss" || pid_kind(a).is_some() {
        return Err("LTI channel assignment requires a tf or zpk destination".into());
    }
    let (nu, ny) = s.ports();
    let (rows, cols) = selectors(e, (ny, nu), args, out)?;
    if rows.len() != 1 || cols.len() != 1 {
        return Err("LTI channel assignment requires one output and one input".into());
    }
    let rhs = decode(e, value)?.ok_or("LTI channel assignment requires a SISO LTI model")?;
    rhs.require_siso("LTI channel assignment")?;
    if timebase(&s, &rhs)? != s.ts {
        return Err("LTI channel assignment must preserve the destination sample time".into());
    }
    if rhs.has_internal_delays() || rhs.delays() != s.delays() {
        return Err("LTI channel assignment requires matching transport delays".into());
    }
    let rhs = e.ctl_convert(&rhs, s.kind(), out)?;
    let i = ny * cols.at(0) + rows.at(0);
    s.model = match (&s.model, rhs.model) {
        (Model::TfArray(old_n, old_d), Model::Tf(n, d)) => {
            let (n, d) = public_tf(&n, &d, &s.variable)?;
            Model::TfArray(
                old_n.replaced(i, n.into(), &e.cancel)?,
                old_d.replaced(i, d.into(), &e.cancel)?,
            )
        }
        (Model::ZpkArray(old_z, old_p, old_k), Model::Zpk(z, p, k)) => {
            let mut gains = old_k.data().to_vec();
            gains[i] = k;
            Model::ZpkArray(
                old_z.replaced(i, roots_value(&e.cancel, &z)?.array(&e.cancel)?, &e.cancel)?,
                old_p.replaced(i, roots_value(&e.cancel, &p)?.array(&e.cancel)?, &e.cancel)?,
                Matrix::from_column_vec(ny, nu, gains)?,
            )
        }
        (_, model) => model,
    };
    encode(e, &s)?.array(&e.cancel)
}

fn rational_map(
    cancel: &AtomicBool,
    shape: (usize, usize),
    kind: &str,
    variable: &str,
    mut f: impl FnMut(usize) -> Result<Model>,
) -> Result<Model> {
    if kind == "tf" {
        let (n, d) = grid_pair(cancel, shape, |i| {
            let Model::Tf(n, d) = f(i)? else {
                unreachable!()
            };
            let (n, d) = public_tf(&n, &d, variable)?;
            Ok((n.into(), d.into()))
        })?;
        Ok(Model::TfArray(n, d))
    } else {
        check_grid(shape.0, shape.1)?;
        let mut gains = Vec::with_capacity(shape.0 * shape.1);
        let (z, p) = grid_pair(cancel, shape, |i| {
            let Model::Zpk(z, p, k) = f(i)? else {
                unreachable!()
            };
            gains.push(k);
            Ok((
                roots_value(cancel, &z)?.array(cancel)?,
                roots_value(cancel, &p)?.array(cancel)?,
            ))
        })?;
        Ok(Model::ZpkArray(
            z,
            p,
            Matrix::from_column_vec(shape.0, shape.1, gains)?,
        ))
    }
}

pub(super) fn concatenate_rational<'a>(
    e: &mut Engine,
    blocks: impl IntoIterator<Item = (&'a Lti, &'a [usize], &'a [usize])>,
    (ny, nu): (usize, usize),
    kind: &str,
    variable: &str,
    out: &mut Execution,
) -> Result<Model> {
    check_grid(ny, nu)?;
    e.preflight_elements(4 * ny * nu)?;
    let mut channels = vec![None; ny * nu];
    for (s, inputs, outputs) in blocks {
        e.check_cancel()?;
        for (j, &col) in inputs.iter().enumerate() {
            for (i, &row) in outputs.iter().enumerate() {
                channels[col * ny + row] = Some((s, j * outputs.len() + i));
            }
        }
    }

    let zero = if kind == "tf" {
        Model::Tf(Matrix::scalar(0.), Matrix::scalar(1.))
    } else {
        Model::Zpk(Vec::new(), Vec::new(), 0.)
    };
    let mut remaining = MAX_CONVERSION_WORK;
    let cancel = e.cancel.clone();
    let mut get_channel = |index: usize| -> Result<Model> {
        let Some((s, i)) = channels[index] else {
            return Ok(zero.clone());
        };
        let model = if s.rational_grid() {
            channel(s, i)?
        } else {
            s.model.clone()
        };
        if s.kind() == kind {
            return Ok(model);
        }
        if matches!(&model, Model::Tf(n, _) if n.data() == [0.]) {
            return Ok(zero.clone());
        }
        charge_work(&mut remaining, channel_work(&model, 3), "LTI concatenation")?;
        Ok(e.ctl_convert(&single(s, model), kind, out)?.model)
    };
    if (ny, nu) == (1, 1) {
        get_channel(0)
    } else {
        rational_map(&cancel, (ny, nu), kind, variable, get_channel)
    }
}

fn ss_tf(e: &mut Engine, s: &Lti, out: &mut Execution) -> Result<Model> {
    let Model::Ss(a, b, c, d) = &s.model else {
        unreachable!()
    };
    let (ny, nu, n) = (c.rows(), b.cols(), a.rows());
    check_grid(ny, nu)?;
    if ny == 0 || nu == 0 {
        return empty_rational(e, "tf", (ny, nu));
    }
    // The characteristic polynomial is shared by every channel. Compute all
    // Markov parameters at each power using matrix products, as in ctl_tf.
    let pairs = ny * nu;
    let entries = pairs * (n + 1);
    let work = n.saturating_mul(n * n * nu + n * pairs + entries);
    if entries > MAX_ENTRIES || work > MAX_CONVERSION_WORK {
        return Err("tf: state-space conversion exceeds its coefficient or work bound".into());
    }
    e.preflight_elements(entries * 2 + 2 * b.len() + pairs)?;
    let den = if n == 0 {
        Matrix::scalar(1.)
    } else {
        e.ctl_poly(a, out)?
    };
    let mut num: Vec<f64> = d
        .data()
        .iter()
        .flat_map(|gain| den.data().iter().map(move |coefficient| gain * coefficient))
        .collect();
    let mut v = b.clone();
    for k in 0..n {
        e.check_cancel()?;
        let markov = c.mul(&v)?;
        for (poly, gain) in num.chunks_exact_mut(n + 1).zip(markov.data()) {
            for (value, coefficient) in poly[k + 1..].iter_mut().zip(den.data()) {
                *value += coefficient * gain;
            }
        }
        if k + 1 < n {
            v = a.mul(&v)?;
        }
    }
    rational_map(&e.cancel, (ny, nu), "tf", &s.variable, |i| {
        let num = row(num[i * (n + 1)..(i + 1) * (n + 1)].to_vec())?;
        normalize(trim_leading(num)?, den.clone())
    })
}

pub(super) fn tf_fields(e: &Engine, s: &Lti, variable: &str) -> Result<(Array, Array)> {
    if let Model::Tf(n, d) = &s.model {
        let (n, d) = public_tf(n, d, variable)?;
        return Ok((cell(e, n.into())?, cell(e, d.into())?));
    }
    let (nu, ny) = s.ports();
    let Model::TfArray(n, d) =
        rational_map(&e.cancel, (ny, nu), "tf", variable, |i| channel(s, i))?
    else {
        unreachable!()
    };
    Ok((n.into(), d.into()))
}

fn single(s: &Lti, model: Model) -> Lti {
    Lti { model, ..s.clone() }
}

pub(super) fn convert(e: &mut Engine, s: &Lti, kind: &str, out: &mut Execution) -> Result<Model> {
    if kind == "tf" && matches!(s.model, Model::Ss(..)) {
        return ss_tf(e, s, out);
    }
    let tf = if matches!(s.model, Model::Ss(..)) {
        single(s, ss_tf(e, s, out)?)
    } else {
        s.clone()
    };
    let (nu, ny) = tf.ports();
    let mut remaining = MAX_CONVERSION_WORK;
    // The closures call back into the engine's polynomial kernels.
    let cancel = e.cancel.clone();
    rational_map(&cancel, (ny, nu), kind, &s.variable, |i| {
        let model = channel(&tf, i)?;
        let exponent = if kind == "zpk" { 3 } else { 2 };
        charge_work(&mut remaining, channel_work(&model, exponent), kind)?;
        let (n, d) = e.ctl_tf(&single(&tf, model), out)?;
        if kind == "tf" {
            normalize(n, d)
        } else {
            let gain = n.data()[0] / d.data()[0];
            if !gain.is_finite() {
                return Err("zpk: conversion produced a nonfinite gain".into());
            }
            if gain == 0. {
                return Ok(Model::Zpk(vec![], vec![], 0.));
            }
            Ok(Model::Zpk(
                e.ctl_roots(&n, out)?,
                e.ctl_roots(&d, out)?,
                gain,
            ))
        }
    })
}

pub(super) fn realize(
    e: &mut Engine,
    s: &Lti,
    out: &mut Execution,
) -> Result<(Matrix, Matrix, Matrix, Matrix)> {
    let (nu, ny) = s.ports();
    let mut channels = Vec::with_capacity(ny * nu);
    let mut remaining = MAX_CONVERSION_WORK;
    let mut n = 0;
    for i in 0..ny * nu {
        e.check_cancel()?;
        let model = channel(s, i)?;
        charge_work(&mut remaining, channel_work(&model, 2), "ss")?;
        let (num, den) = e.ctl_tf(&single(s, model), out)?;
        if num.len() > den.len() {
            return Err(
                "ss: an improper transfer function has no ordinary state-space realization".into(),
            );
        }
        let degree = if num.data() == [0.] { 0 } else { den.len() - 1 };
        n += degree;
        ss_size_check(n, ny, nu)?;
        channels.push((num, den, degree));
    }
    e.preflight_elements(ss_entries(n, ny, nu))?;
    let mut a = vec![0.; n * n];
    let mut b = vec![0.; n * nu];
    let mut c = vec![0.; ny * n];
    let mut d = vec![0.; ny * nu];
    let mut offset = 0;
    // Direct sum of companion systems, connected to their original ports.
    for (i, (num, den, degree)) in channels.into_iter().enumerate() {
        if num.data() == [0.] {
            continue;
        }
        let (feedthrough, coefficients) = companion_coefficients(&num, &den)?;
        d[i] = feedthrough;
        if degree > 0 {
            b[offset + (i / ny) * n] = 1.;
        }
        for (j, (a_top, output)) in coefficients.enumerate() {
            let col = offset + j;
            a[offset + col * n] = a_top;
            if j + 1 < degree {
                a[col + 1 + col * n] = 1.;
            }
            c[i % ny + col * ny] = output;
        }
        offset += degree;
    }
    let (a, b, c, d) = (
        Matrix::from_column_vec(n, n, a)?,
        Matrix::from_column_vec(n, nu, b)?,
        Matrix::from_column_vec(ny, n, c)?,
        Matrix::from_column_vec(ny, nu, d)?,
    );
    ss_check(&a, &b, &c, &d)?;
    Ok((a, b, c, d))
}

pub(super) fn order(e: &Engine, s: &Lti) -> Result<usize> {
    let (nu, ny) = s.ports();
    let mut total = 0;
    for i in 0..nu * ny {
        e.check_cancel()?;
        total += match channel(s, i)? {
            Model::Tf(_, d) => d.len() - 1,
            Model::Zpk(_, p, _) => p.len(),
            _ => unreachable!(),
        };
    }
    Ok(total)
}

fn charge_work(remaining: &mut usize, work: usize, operation: &str) -> Result<()> {
    *remaining = remaining
        .checked_sub(work)
        .ok_or_else(|| format!("{operation}: channel computation exceeds its work bound"))?;
    Ok(())
}

fn channel_work(model: &Model, exponent: u32) -> usize {
    let degree = match model {
        Model::Tf(n, d) => n.len().max(d.len()) - 1,
        Model::Zpk(z, p, _) => z.len().max(p.len()),
        _ => unreachable!(),
    };
    degree.pow(exponent)
}

pub(super) fn poles(e: &mut Engine, s: &Lti, out: &mut Execution) -> Result<Vec<C>> {
    let (nu, ny) = s.ports();
    let mut remaining = MAX_CONVERSION_WORK;
    let mut poles = Vec::new();
    for i in 0..ny * nu {
        e.check_cancel()?;
        let model = channel(s, i)?;
        if let Model::Tf(_, d) = &model {
            charge_work(&mut remaining, (d.len() - 1).pow(3), "pole")?;
        }
        let next = e.ctl_poles(&single(s, model), out)?;
        if poles.len() + next.len() > MAX_ENTRIES {
            return Err("pole: total pole count exceeds the LTI output bound".into());
        }
        poles.extend(next);
    }
    Ok(poles)
}

pub(super) fn minreal(e: &mut Engine, s: &Lti, tol: f64, out: &mut Execution) -> Result<Lti> {
    if let Model::Ss(a, b, c, d) = &s.model {
        return minreal_ss(e, s, (a, b, c, d), tol);
    }
    let (nu, ny) = s.ports();
    let cancel = e.cancel.clone();
    let mut remaining = MAX_CONVERSION_WORK;
    let model = rational_map(&cancel, (ny, nu), s.kind(), &s.variable, |i| {
        let model = channel(s, i)?;
        let exponent = if s.kind() == "tf" { 3 } else { 2 };
        charge_work(&mut remaining, channel_work(&model, exponent), "minreal")?;
        Ok(e.ctl_minreal(&single(s, model), tol, out)?.model)
    })?;
    Ok(single(s, model))
}

fn minreal_ss(
    e: &Engine,
    s: &Lti,
    (a, b, c, d): (&Matrix, &Matrix, &Matrix, &Matrix),
    tol: f64,
) -> Result<Lti> {
    use nalgebra::DMatrix;
    let (n, ny, nu) = (a.rows(), c.rows(), b.cols());
    if n == 0 {
        return Ok(s.clone());
    }
    if (n * n).saturating_mul(n + ny + nu).saturating_mul(4) > MAX_CONVERSION_WORK {
        return Err("minreal: state-space projection exceeds its work bound".into());
    }
    e.preflight_elements(5 * a.len() + 3 * (b.len() + c.len()))?;
    let av = DMatrix::from_column_slice(n, n, a.data());
    let bv = DMatrix::from_column_slice(n, nu, b.data());
    let cv = DMatrix::from_column_slice(ny, n, c.data());
    let (av, bv, cv) = reachable(e, av, bv, cv, tol)?;
    // Observability of (C,A) is reachability of (A',C').
    let (av, cv, bv) = reachable(e, av.transpose(), cv.transpose(), bv.transpose(), tol)?;
    let r = av.nrows();
    if r == n {
        return Ok(s.clone());
    }
    let mut result = single(
        s,
        ss_model(
            Matrix::from_column_slice(r, r, av.transpose().as_slice())?,
            Matrix::from_column_slice(r, nu, bv.transpose().as_slice())?,
            Matrix::from_column_slice(ny, r, cv.transpose().as_slice())?,
            d.clone(),
        )?,
    );
    result.props.state = None;
    Ok(result)
}

type Dense = nalgebra::DMatrix<f64>;

fn reachable(e: &Engine, a: Dense, b: Dense, c: Dense, tol: f64) -> Result<(Dense, Dense, Dense)> {
    use nalgebra::DVector;
    let n = a.nrows();
    let mut basis = Dense::zeros(n, n);
    let mut rank = 0;
    let append = |basis: &mut Dense, rank: &mut usize, mut v: DVector<f64>| -> Result<()> {
        let norm = v.norm();
        if !norm.is_finite() {
            return Err("minreal: state-space projection overflowed".into());
        }
        // Reorthogonalize the Krylov basis. A roundoff floor prevents dependent
        // vectors from adding states when the caller requests zero tolerance.
        for _ in 0..2 {
            for j in 0..*rank {
                let q = basis.column(j);
                v.axpy(-q.dot(&v), &q, 1.);
            }
        }
        let residual = v.norm();
        if residual > norm * tol.max(f64::EPSILON * n as f64) {
            v /= residual;
            basis.set_column(*rank, &v);
            *rank += 1;
        }
        Ok(())
    };
    let mut column = 0;
    for j in 0..b.ncols() {
        e.check_cancel()?;
        append(&mut basis, &mut rank, b.column(j).into_owned())?;
        // Stop when the basis already spans every state.
        if rank == n {
            return Ok((a, b, c));
        }
    }
    while column < rank {
        e.check_cancel()?;
        let next = &a * basis.column(column);
        append(&mut basis, &mut rank, next)?;
        if rank == n {
            return Ok((a, b, c));
        }
        column += 1;
    }
    let q = basis.columns(0, rank);
    let qt = q.transpose();
    Ok((&qt * &a * q, &qt * b, c * q))
}

pub(super) fn display(e: &Engine, s: &Lti) -> Result<String> {
    use std::fmt::Write;
    let (nu, ny) = s.ports();
    let mut body = String::new();
    let inputs = display_labels(&s.input, nu);
    let outputs = display_labels(&s.output, ny);
    for (i, input) in inputs.iter().enumerate() {
        for (j, output) in outputs.iter().enumerate() {
            e.check_cancel()?;
            let text = channel_display(&channel(s, i * ny + j)?, &s.variable)?;
            let needed = input.len() + output.len() + text.len() + 32;
            if body.len() + needed > MAX_ENTRIES {
                return Err(
                    "LTI display exceeds 1048576 bytes; display individual channels".into(),
                );
            }
            if !body.is_empty() {
                body.push_str("\n\n");
            }
            write!(body, "  From input {input} to output {output}:\n{text}").unwrap();
        }
    }
    Ok(body)
}

fn display_labels(names: &Array, count: usize) -> Vec<String> {
    (0..count)
        .map(|i| {
            let a = match names {
                Array::Cell(c) => c.get_linear(i).ok().map(AsRef::as_ref),
                a => Some(a),
            };
            if let Some(Array::Character(text)) = a
                && !text.is_empty()
            {
                format!("\"{}\"", String::from_utf16_lossy(text.units()))
            } else {
                (i + 1).to_string()
            }
        })
        .collect()
}

pub(super) fn static_tf(e: &Engine, d: &Matrix) -> Result<Model> {
    rational_map(&e.cancel, (d.rows(), d.cols()), "tf", "s", |i| {
        Ok(Model::Tf(Matrix::scalar(d.data()[i]), Matrix::scalar(1.)))
    })
}

pub(super) fn scale(e: &Engine, s: &Lti, k: f64) -> Result<Model> {
    let (nu, ny) = s.ports();
    rational_map(
        &e.cancel,
        (ny, nu),
        s.kind(),
        &s.variable,
        |i| match channel(s, i)? {
            Model::Tf(n, d) => normalize(n.map(|x| k * x), d),
            Model::Zpk(z, p, g) if (g * k).is_finite() => Ok(Model::Zpk(z, p, k * g)),
            _ => Err("LTI gain overflowed".into()),
        },
    )
}

pub(in crate::engine) fn frequency_coefficients(
    e: &mut Engine,
    s: &Lti,
    out: &mut Execution,
) -> Result<Vec<(Matrix, Matrix)>> {
    if !s.rational_grid() {
        return Ok(vec![e.ctl_tf(s, out)?]);
    }
    let (nu, ny) = s.ports();
    let mut remaining = MAX_CONVERSION_WORK;
    (0..nu * ny)
        .map(|i| {
            e.check_cancel()?;
            let model = channel(s, i)?;
            charge_work(
                &mut remaining,
                channel_work(&model, 2),
                "Frequency response",
            )?;
            e.ctl_tf(&single(s, model), out)
        })
        .collect()
}

pub(in crate::engine) fn evalfr(
    e: &mut Engine,
    s: &Lti,
    point: C,
    out: &mut Execution,
) -> Result<Value> {
    let (nu, ny) = s.ports();
    if nu == 0 || ny == 0 {
        return Err("Frequency response requires at least one input and one output".into());
    }
    let coefficients = frequency_coefficients(e, s, out)?;
    let mut values = Vec::with_capacity(coefficients.len());
    let eval = |m: &Matrix| m.data().iter().fold(C::new(0., 0.), |v, &x| v * point + x);
    for (n, d) in coefficients {
        e.check_cancel()?;
        let den = eval(&d);
        if den == C::new(0., 0.) {
            return Err("Frequency coincides with a system pole".into());
        }
        let value = eval(&n) / den;
        if !value.re.is_finite() || !value.im.is_finite() {
            return Err("Frequency response overflowed".into());
        }
        values.push(value);
    }
    Ok(Value::Complex(ComplexMatrix::from_column_vec(
        ny, nu, values, &e.cancel,
    )?))
}
