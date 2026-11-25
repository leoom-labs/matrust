//! LTI interconnections. Port maps preserve MATLAB's external signal
//! order, while the realizations retain each component's original state basis.
use super::*;
use nalgebra::{DMatrix, DMatrixView};
use std::ops::AddAssign;

const SERIES_SIZE: &str =
    "The number of outputs of the first model must match the number of inputs of the second model.";
const PARALLEL_SIZE: &str = "The models must have the same number of inputs and outputs.";
const FEEDBACK_SIZE: &str =
    "The first and second arguments of the \"feedback\" command must have compatible I/O sizes.";
pub(super) const LOOP: &str = "Feedback has a singular algebraic loop";
const MAX_CONNECT_WORK: usize = 500_000_000;

struct Realization {
    a: Matrix,
    b: Matrix,
    c: Matrix,
    d: Matrix,
}

impl Realization {
    fn of(e: &mut Engine, s: &Lti, out: &mut Execution) -> Result<Self> {
        let (a, b, c, d) = e.ctl_ss(s, out)?;
        Ok(Self { a, b, c, d })
    }

    fn states(&self) -> usize {
        self.a.rows()
    }

    fn into_model(self) -> Result<Model> {
        ss_model(self.a, self.b, self.c, self.d)
    }
}

fn strongest(systems: &[Lti]) -> &'static str {
    systems
        .iter()
        .map(Lti::kind)
        .max_by_key(|kind| rank(kind))
        .unwrap_or("tf")
}

pub(super) fn operand(e: &Engine, v: &Value, use_ss: bool) -> Result<Lti> {
    if let Value::Matrix(m) = v
        && (m.rows(), m.cols()) != (1, 1)
    {
        let d = real(v)?;
        let (ny, nu) = (d.rows(), d.cols());
        let model = if use_ss {
            ss_size_check(0, ny, nu)?;
            Model::Ss(
                Matrix::filled(0, 0, 0.)?,
                Matrix::filled(0, nu, 0.)?,
                Matrix::filled(ny, 0, 0.)?,
                d,
            )
        } else {
            mimo::static_tf(e, &d)?
        };
        return new(e, model, 0.);
    }
    get_lti(e, v)
}

fn use_ss(args: &[Value]) -> bool {
    args.iter()
        .any(|v| matches!(v, Value::Container(a) if class(a) == Some("ss")))
}

pub(super) fn binary(
    e: &mut Engine,
    op: &Token,
    left: &Value,
    right: &Value,
    out: &mut Execution,
) -> Result<Lti> {
    let args = [left.clone(), right.clone()];
    let ss = use_ss(&args);
    let (a, b) = (operand(e, left, ss)?, operand(e, right, ss)?);
    binary_models(e, op, [&a, &b], [left, right], out)
}

fn binary_models(
    e: &mut Engine,
    op: &Token,
    [a, b]: [&Lti; 2],
    [left, right]: [&Value; 2],
    out: &mut Execution,
) -> Result<Lti> {
    if matches!(op, Token::Star | Token::Plus | Token::Minus)
        && (a.ports() == (1, 1)) != (b.ports() == (1, 1))
    {
        let scalar_left = a.ports() == (1, 1);
        let (gain, system, value) = if scalar_left {
            (a, b, left)
        } else {
            (b, a, right)
        };
        if *op == Token::Star
            && let Ok(k) = scalar(value)
        {
            return scale(e, system.clone(), k);
        }
        let kind = if rank(a.kind()) >= rank(b.kind()) {
            a.kind()
        } else {
            b.kind()
        };
        let (nu, ny) = system.ports();
        let expanded = if *op == Token::Star {
            let count = if scalar_left { ny } else { nu };
            expand_siso(e, gain, count, count, true, out)?
        } else {
            expand_siso(e, gain, ny, nu, false, out)?
        };
        let mut result = if scalar_left {
            e.ctl_combine(op, &expanded, system, out)?
        } else {
            e.ctl_combine(op, system, &expanded, out)?
        };
        result.input = system.input.clone();
        result.output = system.output.clone();
        return e.ctl_convert(&result, kind, out);
    }
    e.ctl_combine(op, a, b, out)
}

fn expand_siso(
    e: &mut Engine,
    s: &Lti,
    ny: usize,
    nu: usize,
    diagonal: bool,
    out: &mut Execution,
) -> Result<Lti> {
    s.require_delay_free("Scalar expansion in LTI arithmetic")?;
    let mut expanded = e.ctl_convert(s, "ss", out)?;
    let Model::Ss(a, b, c, d) = &expanded.model else {
        unreachable!()
    };
    let states = a.rows();
    if diagonal && nu > 0 {
        ss_size_check(states * nu, ny, nu)?;
        return concatenate(e, vec![expanded; nu], Stack::Append, out);
    }
    // The same scalar transfer in every channel: sum the inputs before the
    // scalar system and broadcast its output, sharing the scalar state.
    let n = if diagonal { 0 } else { states };
    admit(e, n, ny, nu, 0)?;
    let a = if diagonal {
        Matrix::filled(0, 0, 0.)?
    } else {
        a.clone()
    };
    let b = Matrix::from_column_vec(n, nu, b.data().repeat(nu))?;
    let c = Matrix::from_column_vec(
        ny,
        n,
        c.data()
            .iter()
            .flat_map(|&v| std::iter::repeat_n(v, ny))
            .collect(),
    )?;
    let d = Matrix::filled(ny, nu, d.data()[0])?;
    expanded.model = ss_model(a, b, c, d)?;
    expanded.input = empty_names(e, nu)?;
    expanded.output = empty_names(e, ny)?;
    if diagonal {
        expanded.props.state = None;
    }
    Ok(expanded)
}

pub(super) fn scale(e: &Engine, mut s: Lti, k: f64) -> Result<Lti> {
    fn scaled(e: &Engine, s: &Lti, k: f64) -> Result<Model> {
        Ok(match &s.model {
            Model::Tf(n, d) => {
                let n = n.map(|x| k * x);
                if n.data().iter().any(|x| !x.is_finite()) {
                    return Err("LTI gain overflowed".into());
                }
                Model::Tf(n, d.clone())
            }
            Model::Zpk(z, p, g) if (k * g).is_finite() => Model::Zpk(z.clone(), p.clone(), k * g),
            Model::Zpk(..) => return Err("LTI gain overflowed".into()),
            Model::Ss(a, b, c, d) => {
                ss_model(a.clone(), b.clone(), c.map(|x| k * x), d.map(|x| k * x))?
            }
            _ => mimo::scale(e, s, k)?,
        })
    }
    s.model = scaled(e, &s, k)?;
    for path in &mut s.props.paths {
        path.model = scaled(e, path, k)?;
    }
    Ok(s)
}

fn indices(v: &Value, count: usize, name: &str) -> Result<Vec<usize>> {
    let m = real(v)?;
    if !m.is_empty() && m.rows() != 1 && m.cols() != 1 || m.len() > count {
        return Err(format!("{name} must be a vector of distinct port indices."));
    }
    let mut seen = vec![false; count];
    let mut ports = Vec::with_capacity(m.len());
    for &v in m.data() {
        if v < 1. || v > count as f64 || v.fract() != 0. {
            return Err(format!("{name} must contain valid port indices."));
        }
        let index = v as usize - 1;
        if std::mem::replace(&mut seen[index], true) {
            return Err(format!("{name} must contain distinct port indices."));
        }
        ports.push(index);
    }
    Ok(ports)
}

fn admit(e: &Engine, n: usize, ny: usize, nu: usize, work: usize) -> Result<()> {
    ss_size_check(n, ny, nu)?;
    if work > MAX_CONNECT_WORK {
        return Err("Interconnection exceeds its matrix work bound".into());
    }
    e.preflight_elements(ss_entries(n, ny, nu))
}

fn make_result(
    e: &mut Engine,
    systems: &[Lti],
    states: &[usize],
    ts: f64,
    matrices: Realization,
    names: (Array, Array),
    out: &mut Execution,
) -> Result<Lti> {
    let kind = strongest(systems);
    let mut result = new(e, matrices.into_model()?, ts)?;
    (result.input, result.output) = names;
    // Keep the variables and metadata used by the normal model conversions.
    copy_metadata(&mut result, &systems[0], systems.last().unwrap());
    if kind == "ss" {
        result.props.state =
            properties::state_names(e, systems.iter().zip(states.iter().copied()))?;
    }
    e.check_cancel()?;
    e.ctl_convert(&result, kind, out)
}

fn name_at(names: &Array, index: usize) -> Result<Arc<Array>> {
    match names {
        Array::Cell(c) => c.get_linear(index).cloned(),
        _ => Ok(Arc::new(names.clone())),
    }
}

fn mapped_names<'a>(
    e: &Engine,
    count: usize,
    sources: impl IntoIterator<Item = (&'a Array, &'a [usize])>,
) -> Result<Array> {
    let empty = Arc::new(char_array(e, "")?);
    let mut names = vec![empty.clone(); count];
    let mut conflict = vec![false; count];
    for (source, map) in sources {
        for (i, &dest) in map.iter().enumerate() {
            let name = name_at(source, i)?;
            if name.is_empty() || conflict[dest] {
                continue;
            }
            if names[dest].is_empty() {
                names[dest] = name;
            } else if let (Array::Character(old), Array::Character(new)) =
                (names[dest].as_ref(), name.as_ref())
                && old.units() != new.units()
            {
                names[dest] = empty.clone();
                conflict[dest] = true;
            }
        }
    }
    Ok(CellArray::from_shared(count, 1, names, &e.cancel)?.into())
}

#[derive(Clone, Copy)]
enum Stack {
    Append,
    Horizontal,
    Vertical,
}

struct PortMaps {
    inputs: Vec<usize>,
    outputs: Vec<usize>,
}

fn stack_timebase(systems: &[Lti]) -> Result<f64> {
    let mut base = &systems[0];
    let mut ts = base.ts;
    for sys in systems {
        sys.require_delay_free("LTI concatenation or indexed parallel connection")?;
        ts = timebase(base, sys)?;
        if base.ts != ts || static_model(base) && !static_model(sys) {
            base = sys;
        }
    }
    Ok(ts)
}

fn stack_names(
    e: &Engine,
    systems: &[Lti],
    maps: &[PortMaps],
    (ny, nu): (usize, usize),
) -> Result<(Array, Array)> {
    let names = systems.iter().zip(maps);
    let input = mapped_names(
        e,
        nu,
        names.clone().map(|(s, m)| (&s.input, m.inputs.as_slice())),
    )?;
    let output = mapped_names(e, ny, names.map(|(s, m)| (&s.output, m.outputs.as_slice())))?;
    Ok((input, output))
}

fn stack(
    e: &mut Engine,
    systems: &[Lti],
    maps: &[PortMaps],
    (ny, nu): (usize, usize),
    subtract: bool,
    out: &mut Execution,
) -> Result<Lti> {
    let ts = stack_timebase(systems)?;
    let mut realizations = Vec::with_capacity(systems.len());
    let mut states = Vec::with_capacity(systems.len());
    let mut n = 0;
    for sys in systems {
        e.check_cancel()?;
        let r = Realization::of(e, sys, out)?;
        let count = r.states();
        n += count;
        admit(e, n, ny, nu, 0)?;
        states.push(count);
        realizations.push(r);
    }
    let (mut a, mut b, mut c, mut d) = (
        vec![0.; n * n],
        vec![0.; n * nu],
        vec![0.; ny * n],
        vec![0.; ny * nu],
    );
    let mut start = 0;
    for (j, (r, map)) in realizations.iter().zip(maps).enumerate() {
        let ns = r.states();
        let sign = if subtract && j == 1 { -1. } else { 1. };
        for k in 0..ns {
            e.check_cancel()?;
            let col = (start + k) * n + start;
            a[col..col + ns].copy_from_slice(&r.a.data()[k * ns..(k + 1) * ns]);
            for (i, &dest) in map.outputs.iter().enumerate() {
                c[dest + ny * (start + k)] = sign * r.c.get(i, k);
            }
        }
        for (k, &dest) in map.inputs.iter().enumerate() {
            e.check_cancel()?;
            let col = dest * n + start;
            b[col..col + ns].copy_from_slice(&r.b.data()[k * ns..(k + 1) * ns]);
            for (i, &row) in map.outputs.iter().enumerate() {
                d[row + dest * ny] += sign * r.d.get(i, k);
            }
        }
        start += ns;
    }
    let names = stack_names(e, systems, maps, (ny, nu))?;
    let r = Realization {
        a: Matrix::from_column_vec(n, n, a)?,
        b: Matrix::from_column_vec(n, nu, b)?,
        c: Matrix::from_column_vec(ny, n, c)?,
        d: Matrix::from_column_vec(ny, nu, d)?,
    };
    make_result(e, systems, &states, ts, r, names, out)
}

fn concatenate(e: &mut Engine, systems: Vec<Lti>, mode: Stack, out: &mut Execution) -> Result<Lti> {
    let (first_inputs, first_outputs) = systems
        .first()
        .ok_or("Expected at least one LTI model.")?
        .ports();
    if systems.len() == 1 {
        return Ok(systems.into_iter().next().unwrap());
    }
    let mut ny = 0;
    let mut nu = 0;
    let mut maps = Vec::with_capacity(systems.len());
    for s in &systems {
        let (m, p) = s.ports();
        let (in_offset, out_offset) = match mode {
            Stack::Horizontal if p != first_outputs => return Err(
                "All models being horizontally concatenated must have the same number of outputs."
                    .into(),
            ),
            Stack::Vertical if m != first_inputs => {
                return Err(
                    "All models being vertically concatenated must have the same number of inputs."
                        .into(),
                );
            }
            Stack::Horizontal => (nu, 0),
            Stack::Vertical => (0, ny),
            Stack::Append => (nu, ny),
        };
        nu = in_offset + m;
        ny = out_offset + p;
        ss_size_check(0, ny, nu)?;
        maps.push(PortMaps {
            inputs: (in_offset..nu).collect(),
            outputs: (out_offset..ny).collect(),
        });
    }
    let kind = strongest(&systems);
    if kind == "ss" {
        return stack(e, &systems, &maps, (ny, nu), false, out);
    }

    let ts = stack_timebase(&systems)?;
    let (input, output) = stack_names(e, &systems, &maps, (ny, nu))?;
    let mut result = Lti {
        ts,
        input,
        output,
        ..systems[0].clone()
    };
    copy_metadata(&mut result, &systems[0], systems.last().unwrap());
    result.model = mimo::concatenate_rational(
        e,
        systems
            .iter()
            .zip(&maps)
            .map(|(s, m)| (s, m.inputs.as_slice(), m.outputs.as_slice())),
        (ny, nu),
        kind,
        &result.variable,
        out,
    )?;
    Ok(result)
}

pub(in crate::engine) fn literal(
    e: &mut Engine,
    rows: Vec<Vec<Value>>,
    out: &mut Execution,
) -> Result<Value> {
    let ss = rows.iter().any(|row| use_ss(row));
    let mut columns = Vec::with_capacity(rows.len());
    for row in rows {
        let models = row
            .iter()
            .filter(
                |v| !matches!(v, Value::Matrix(m) if !m.is_nd() && (m.rows(), m.cols()) == (0, 0)),
            )
            .map(|v| operand(e, v, ss))
            .collect::<Result<Vec<_>>>()?;
        if !models.is_empty() {
            columns.push(concatenate(e, models, Stack::Horizontal, out)?);
        }
    }
    let result = concatenate(e, columns, Stack::Vertical, out)?;
    encode(e, &result)
}

fn parallel_ports(
    first: usize,
    second: usize,
    selected1: &[usize],
    selected2: &[usize],
) -> (Vec<usize>, Vec<usize>, usize) {
    let mut left = vec![usize::MAX; first];
    let mut right = vec![usize::MAX; second];
    let mut next = first - selected1.len();
    for (&i, &j) in selected1.iter().zip(selected2) {
        left[i] = next;
        right[j] = next;
        next += 1;
    }
    for (dest, index) in left.iter_mut().filter(|i| **i == usize::MAX).zip(0..) {
        *dest = index;
    }
    for dest in right.iter_mut().filter(|i| **i == usize::MAX) {
        *dest = next;
        next += 1;
    }
    (left, right, next)
}

fn parallel(
    e: &mut Engine,
    systems: &[Lti; 2],
    selection: &[Value],
    subtract: bool,
    out: &mut Execution,
) -> Result<Lti> {
    let [(nu1, ny1), (nu2, ny2)] = systems.each_ref().map(Lti::ports);
    let (i1, i2, nu, o1, o2, ny) = if let [i1, i2, o1, o2] = selection {
        let (i1, i2) = (indices(i1, nu1, "in1")?, indices(i2, nu2, "in2")?);
        let (o1, o2) = (indices(o1, ny1, "out1")?, indices(o2, ny2, "out2")?);
        if i1.len() != i2.len() || o1.len() != o2.len() {
            return Err(
                "The input index vectors and the output index vectors must have matching lengths."
                    .into(),
            );
        }
        let (i1, i2, nu) = parallel_ports(nu1, nu2, &i1, &i2);
        let (o1, o2, ny) = parallel_ports(ny1, ny2, &o1, &o2);
        (i1, i2, nu, o1, o2, ny)
    } else {
        if (nu1, ny1) != (nu2, ny2) {
            return Err(PARALLEL_SIZE.into());
        }
        (
            (0..nu1).collect(),
            (0..nu2).collect(),
            nu1,
            (0..ny1).collect(),
            (0..ny2).collect(),
            ny1,
        )
    };
    stack(
        e,
        systems,
        &[
            PortMaps {
                inputs: i1,
                outputs: o1,
            },
            PortMaps {
                inputs: i2,
                outputs: o2,
            },
        ],
        (ny, nu),
        subtract,
        out,
    )
}

fn subports(m: &Matrix, rows: Option<&[usize]>, cols: Option<&[usize]>) -> DMatrix<f64> {
    DMatrix::from_fn(
        rows.map_or(m.rows(), <[_]>::len),
        cols.map_or(m.cols(), <[_]>::len),
        |i, j| m.get(rows.map_or(i, |r| r[i]), cols.map_or(j, |c| c[j])),
    )
}

fn dense(m: &Matrix) -> DMatrixView<'_, f64> {
    DMatrixView::from_slice(m.data(), m.rows(), m.cols())
}

fn matrix(m: DMatrix<f64>) -> Result<Matrix> {
    Matrix::from_column_vec(m.nrows(), m.ncols(), m.data.into())
}

fn series(
    e: &mut Engine,
    systems: &[Lti; 2],
    selection: &[Value],
    out: &mut Execution,
) -> Result<Lti> {
    let [first, second] = systems;
    let (nu, mid1) = first.ports();
    let (mid2, ny) = second.ports();
    let (outputs, inputs) = if let [o, i] = selection {
        let (o, i) = (indices(o, mid1, "out1")?, indices(i, mid2, "in2")?);
        if o.len() != i.len() {
            return Err(
                "The output index vector and input index vector must have the same length.".into(),
            );
        }
        (Some(o), Some(i))
    } else {
        if mid1 != mid2 {
            return Err(SERIES_SIZE.into());
        }
        (None, None)
    };
    first.require_delay_free("MIMO series connection")?;
    second.require_delay_free("MIMO series connection")?;
    let ts = timebase(first, second)?;
    let (u, v) = (
        Realization::of(e, first, out)?,
        Realization::of(e, second, out)?,
    );
    let (n1, n2) = (u.states(), v.states());
    let mid = outputs.as_ref().map_or(mid1, Vec::len);
    admit(e, n1 + n2, ny, nu, (n1 + nu) * mid * (n2 + ny))?;
    let (c1, d1) = (
        subports(&u.c, outputs.as_deref(), None),
        subports(&u.d, outputs.as_deref(), None),
    );
    let (b2, d2) = (
        subports(&v.b, None, inputs.as_deref()),
        subports(&v.d, None, inputs.as_deref()),
    );
    let r = Realization {
        a: block(vec![
            vec![u.a, Matrix::filled(n1, n2, 0.)?],
            vec![matrix(&b2 * &c1)?, v.a],
        ])?,
        b: block(vec![vec![u.b], vec![matrix(&b2 * &d1)?]])?,
        c: block(vec![vec![matrix(&d2 * &c1)?, v.c]])?,
        d: matrix(d2 * d1)?,
    };
    make_result(
        e,
        systems,
        &[n1, n2],
        ts,
        r,
        (first.input.clone(), second.output.clone()),
        out,
    )
}

fn feedback(
    e: &mut Engine,
    systems: &[Lti; 2],
    selection: &[Value],
    sign: f64,
    out: &mut Execution,
) -> Result<Lti> {
    let [plant, controller] = systems;
    let (nu, ny) = plant.ports();
    let (nh_in, nh_out) = controller.ports();
    let (inputs, outputs) = if let [i, o] = selection {
        (indices(i, nu, "feedin")?, indices(o, ny, "feedout")?)
    } else {
        ((0..nu).collect(), (0..ny).collect())
    };
    if (inputs.len(), outputs.len()) != (nh_out, nh_in) {
        return Err(FEEDBACK_SIZE.into());
    }
    plant.require_delay_free("MIMO feedback")?;
    controller.require_delay_free("MIMO feedback")?;
    if !matches!(sign, 1. | -1.) {
        return Err("feedback sign must be +1 or -1".into());
    }
    let ts = timebase(plant, controller)?;
    let (g, h) = (
        Realization::of(e, plant, out)?,
        Realization::of(e, controller, out)?,
    );
    let (ng, nh) = (g.states(), h.states());
    let width = ng + nh + nu;
    let k = inputs.len();
    admit(
        e,
        ng + nh,
        ny,
        nu,
        (k + nh_in) * width * (width + ny + k + nh_in),
    )?;
    let (cg, dg) = (
        subports(&g.c, Some(&outputs), None),
        subports(&g.d, Some(&outputs), None),
    );
    let loop_d = subports(&g.d, Some(&outputs), Some(&inputs));
    let (ch, dh) = (dense(&h.c), dense(&h.d));

    // Correction f at the selected plant inputs satisfies
    // (I - sign*Dh*Dg_sel) f = sign * (Dh*Cg_sel xg + Ch xh + Dh*Dg_out u).
    let mut rhs = DMatrix::zeros(k, width);
    rhs.columns_mut(0, ng).copy_from(&(dh * &cg * sign));
    rhs.columns_mut(ng, nh).copy_from(&(ch * sign));
    rhs.columns_mut(ng + nh, nu).copy_from(&(dh * &dg * sign));
    let loop_matrix = DMatrix::identity(k, k) - dh * &loop_d * sign;
    if loop_matrix.iter().any(|v| !v.is_finite()) {
        return Err(LOOP.into());
    }
    e.check_cancel()?;
    if k > 0 {
        // Elimination can leave a roundoff-sized pivot in an exactly dependent
        // column. Scale each pivot to its original column to allow disparate port gains.
        let scales: Vec<_> = loop_matrix.column_iter().map(|col| col.amax()).collect();
        let lu = loop_matrix.lu();
        let tolerance = k as f64 * f64::EPSILON;
        let bad_pivot = scales.iter().enumerate().any(|(i, scale)| {
            let pivot = lu.lu_internal()[(i, i)].abs();
            !pivot.is_finite() || pivot <= tolerance * scale
        });
        if bad_pivot || !lu.solve_mut(&mut rhs) {
            return Err(LOOP.into());
        }
    }
    if rhs.iter().any(|v| !v.is_finite()) {
        return Err(LOOP.into());
    }
    e.check_cancel()?;
    let n = ng + nh;
    let mut plant_state = subports(&g.b, None, Some(&inputs)) * &rhs;
    let mut plant_output = subports(&g.d, None, Some(&inputs)) * &rhs;
    let mut measured = loop_d * rhs;
    measured.columns_mut(0, ng).add_assign(cg);
    measured.columns_mut(n, nu).add_assign(dg);
    let mut controller_state = dense(&h.b) * measured;

    plant_state.columns_mut(0, ng).add_assign(dense(&g.a));
    plant_state.columns_mut(n, nu).add_assign(dense(&g.b));
    controller_state.columns_mut(ng, nh).add_assign(dense(&h.a));
    plant_output.columns_mut(0, ng).add_assign(dense(&g.c));
    plant_output.columns_mut(n, nu).add_assign(dense(&g.d));
    let r = Realization {
        a: block(vec![
            vec![matrix(plant_state.columns(0, n).into_owned())?],
            vec![matrix(controller_state.columns(0, n).into_owned())?],
        ])?,
        b: block(vec![
            vec![matrix(plant_state.columns(n, nu).into_owned())?],
            vec![matrix(controller_state.columns(n, nu).into_owned())?],
        ])?,
        c: matrix(plant_output.columns(0, n).into_owned())?,
        d: matrix(plant_output.columns(n, nu).into_owned())?,
    };
    make_result(
        e,
        systems,
        &[ng, nh],
        ts,
        r,
        (plant.input.clone(), plant.output.clone()),
        out,
    )
}

pub(super) fn combine(
    e: &mut Engine,
    op: &Token,
    left: &Lti,
    right: &Lti,
    out: &mut Execution,
) -> Result<Lti> {
    match op {
        Token::Star => {
            if left.ports().0 != right.ports().1 {
                return Err("Model I/O dimensions must agree.".into());
            }
            series(e, &[right.clone(), left.clone()], &[], out)
        }
        Token::Plus | Token::Minus => parallel(
            e,
            &[left.clone(), right.clone()],
            &[],
            *op == Token::Minus,
            out,
        ),
        _ => Err("LTI division requires SISO models".into()),
    }
}

pub(super) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    out: &mut Execution,
) -> Result<Value> {
    if name == "append" {
        if args.is_empty() || args.len() > 4096 || !args.iter().any(is_lti) {
            return Err("append requires 1..4096 models, including at least one LTI model.".into());
        }
        let ss = use_ss(args);
        let models = args
            .iter()
            .map(|v| operand(e, v, ss))
            .collect::<Result<Vec<_>>>()?;
        let result = concatenate(e, models, Stack::Append, out)?;
        return encode(e, &result);
    }
    let valid = match name {
        "series" => matches!(args.len(), 2 | 4),
        "parallel" => matches!(args.len(), 2 | 6),
        _ => matches!(args.len(), 2..=5),
    };
    if !valid {
        return Err(format!("Invalid arguments for {name}."));
    }
    if !args[..2].iter().any(is_lti) {
        return Err(format!("{name} requires an LTI model"));
    }
    let ss = use_ss(&args[..2]);
    let systems = [operand(e, &args[0], ss)?, operand(e, &args[1], ss)?];
    let [a, b] = &systems;
    let selection = &args[2..];
    let result = match name {
        "series" if selection.is_empty() && a.ports() == (1, 1) && b.ports() == (1, 1) => {
            e.ctl_combine(&Token::Star, b, a, out)?
        }
        "series"
            if selection.is_empty()
                && a.ports().1 != b.ports().0
                && ((a.ports() == (1, 1)) != (b.ports() == (1, 1))) =>
        {
            binary_models(e, &Token::Star, [b, a], [&args[1], &args[0]], out)?
        }
        "parallel" if selection.is_empty() => e.ctl_combine(&Token::Plus, a, b, out)?,
        "series" => series(e, &systems, selection, out)?,
        "parallel" => parallel(e, &systems, selection, false, out)?,
        _ => {
            let (ports, sign) = match selection {
                [] => (&[][..], -1.),
                [sign] => (&[][..], scalar(sign)?),
                [_, _] => (selection, -1.),
                [_, _, sign] => (&selection[..2], scalar(sign)?),
                _ => unreachable!(),
            };
            if ports.is_empty() && a.ports() == (1, 1) && b.ports() == (1, 1) {
                e.ctl_feedback(a, b, sign, out)?
            } else {
                feedback(e, &systems, ports, sign, out)?
            }
        }
    };
    encode(e, &result)
}
