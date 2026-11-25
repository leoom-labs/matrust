//! Public contract: https://www.mathworks.com/help/mpc/ref/mpc.sim.html and
//! https://www.mathworks.com/help/mpc/ref/mpcsimopt.html.
//! Physical signals stay in plant order. Controller work goes through mpcmove.
use super::super::{Engine, Execution, Value, control_lti, objects};
use super::object::{self, MpcObject};
use crate::{Array, CharacterRow, Matrix, Result, StructArray, syntax::PathStep};
use std::sync::Arc;

const EMPTY_FIELDS: &[&str] = &[
    "PlantInitialState",
    "ControllerInitialState",
    "UnmeasuredDisturbance",
    "InputNoise",
    "OutputNoise",
    "Model",
    "MVSignal",
];
const SWITCH_FIELDS: &[&str] = &[
    "OpenLoop",
    "StatusBar",
    "RefLookAhead",
    "MDLookAhead",
    "Constraints",
];
const MAX_SAMPLES: usize = 65_536;

pub(super) fn options(e: &Engine, args: &[Value], count: usize) -> Result<Vec<Value>> {
    if args.len() > 1
        || count > 1
        || args
            .first()
            .is_some_and(|v| objects::mpc_value(e, v).is_none())
    {
        return Err("mpcsimopt: expected zero inputs or an mpc controller and one output".into());
    }
    let mut fields = Vec::new();
    for &name in EMPTY_FIELDS {
        fields.push((name, Array::Numeric(Matrix::filled(0, 0, 0.)?)));
    }
    for &name in SWITCH_FIELDS {
        fields.push((
            name,
            Array::Character(CharacterRow::from_text(
                if name == "Constraints" { "on" } else { "off" },
                &e.cancel,
            )?),
        ));
    }
    Ok(vec![super::structure(e, fields)?])
}

fn empty(a: &Array) -> bool {
    matches!(a, Array::Numeric(m) if m.is_empty() && !m.is_nd())
}
fn field(s: &StructArray, name: &str) -> Result<Array> {
    Ok(s.get(
        s.field_index(name)
            .ok_or_else(|| format!("sim: missing option '{name}'"))?,
        0,
    )?
    .as_ref()
    .clone())
}
fn switch(e: &Engine, s: &StructArray, name: &str) -> Result<bool> {
    match super::text(e, &Value::from(field(s, name)?)).as_deref() {
        Some("on") => Ok(true),
        Some("off") => Ok(false),
        _ => Err(format!("sim: {name} must be 'on' or 'off'")),
    }
}
fn decode_options(e: &Engine, arg: Option<&Value>) -> Result<StructArray> {
    let Value::Container(Array::Struct(mut s)) = options(e, &[], 1)?.remove(0) else {
        unreachable!()
    };
    if let Some(v) = arg {
        let a = v.clone().array(&e.cancel)?;
        if empty(&a) {
            return Ok(s);
        }
        let Array::Struct(input) = a else {
            return Err("sim: options must be a scalar mpcsimopt structure".into());
        };
        if input.len() != 1 {
            return Err("sim: options must be a scalar structure".into());
        }
        for (i, name) in input.field_names().iter().enumerate() {
            if !EMPTY_FIELDS.contains(&name.as_str()) && !SWITCH_FIELDS.contains(&name.as_str()) {
                return Err(format!("sim: unsupported option '{name}'"));
            }
            s = s.with_field(name, input.get(i, 0)?.as_ref().clone(), &e.cancel)?;
        }
    }
    for &name in SWITCH_FIELDS {
        switch(e, &s, name)?;
    }
    Ok(s)
}
fn signal(a: &Array, channels: usize, default: &[f64], name: &str) -> Result<Matrix> {
    let Array::Numeric(m) = a else {
        return Err(format!("sim: {name} must be a real double matrix"));
    };
    if m.is_nd() {
        return Err(format!("sim: {name} must be two-dimensional"));
    }
    if m.is_empty() {
        return Matrix::from_column_vec(1, channels, default.to_vec());
    }
    if m.rows() > MAX_SAMPLES || m.cols() != channels || m.data().iter().any(|x| !x.is_finite()) {
        return Err(format!(
            "sim: {name} must have 1..{MAX_SAMPLES} finite rows and {channels} columns"
        ));
    }
    Ok(m.clone())
}
fn at(m: &Matrix, k: usize, j: usize) -> f64 {
    m.get(k.min(m.rows() - 1), j)
}
fn preview(m: &Matrix, k: usize, rows: usize) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(
        rows,
        m.cols(),
        (0..m.cols())
            .flat_map(|j| (0..rows).map(move |i| at(m, k + i, j)))
            .collect(),
    )?))
}
fn simulation_model(
    e: &mut Engine,
    m: &MpcObject,
    value: &Array,
    out: &mut Execution,
) -> Result<MpcObject> {
    if empty(value) {
        return Ok(m.clone());
    }
    let (plant, nominal) = if control_lti::class(value).is_some() {
        (value.clone(), None)
    } else {
        let Array::Struct(s) = value else {
            return Err("sim: Model must be an LTI plant or a Plant/Nominal structure".into());
        };
        if s.len() != 1 {
            return Err("sim: Model must be scalar".into());
        }
        for name in s.field_names() {
            if !matches!(name.as_str(), "Plant" | "Nominal") {
                return Err(format!("sim: unsupported Model field '{name}'"));
            }
        }
        (
            field(s, "Plant")?,
            s.field_index("Nominal")
                .map(|i| s.get(i, 0).map(|v| v.as_ref().clone()))
                .transpose()?,
        )
    };
    let mut result = object::assign(
        e,
        m,
        &[
            PathStep::Field("Model".into()),
            PathStep::Field("Plant".into()),
        ],
        plant.clone(),
        out,
    )?;
    if result.nx() > super::prediction::MAX_DIM {
        return Err("sim: simulation plant exceeds 128 states".into());
    }
    let groups = object::init_signals(e, &plant, result.nu(), result.ny())?;
    if groups.mv_idx != m.mv_idx
        || groups.md_idx != m.md_idx
        || groups.ud_idx != m.ud_idx
        || groups.mo_idx != m.mo_idx
        || groups.uo_idx != m.uo_idx
    {
        return Err("sim: Model signal groups must match the controller plant".into());
    }
    if control_lti::class(&plant) != Some("ss")
        || control_lti::class(&m.plant_array) != Some("ss")
        || result.nx() != m.nx()
    {
        result.nominal.x = Matrix::filled(result.nx(), 1, 0.)?;
        result.nominal.dx = Matrix::filled(result.nx(), 1, 0.)?;
    }
    if let Some(nominal) = nominal {
        result = object::assign(
            e,
            &result,
            &[
                PathStep::Field("Model".into()),
                PathStep::Field("Nominal".into()),
            ],
            nominal,
            out,
        )?;
    }
    Ok(result)
}
fn unconstrained(m: &MpcObject) -> MpcObject {
    let mut result = m.clone();
    for v in &mut result.mv {
        v.min = Matrix::scalar(f64::NEG_INFINITY);
        v.max = Matrix::scalar(f64::INFINITY);
        v.rate_min = Matrix::scalar(f64::NEG_INFINITY);
        v.rate_max = Matrix::scalar(f64::INFINITY);
    }
    for v in &mut result.ov {
        v.min = Matrix::scalar(f64::NEG_INFINITY);
        v.max = Matrix::scalar(f64::INFINITY);
    }
    result
}
fn snapshot(e: &Engine, s: &super::state::MpcState, slots: &mut Vec<Arc<Array>>) -> Result<()> {
    for name in super::state::PROPERTIES {
        slots.push(Arc::new(super::state::property(s, name)?.array(&e.cancel)?));
    }
    Ok(())
}

pub(super) fn outputs(
    e: &mut Engine,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    e.check_cancel()?;
    if !(3..=5).contains(&args.len()) || count > 6 {
        return Err("sim: expected controller,samples,reference[,disturbance,options] and at most six outputs".into());
    }
    let m = objects::mpc_value(e, &args[0]).ok_or("sim: first input must be an mpc controller")?;
    let n = super::real_scalar(&args[1])
        .filter(|x| x.is_finite() && *x >= 1. && x.fract() == 0. && *x <= MAX_SAMPLES as f64)
        .ok_or("sim: sample count must be a positive integer <= 65536")? as usize;
    let fourth_is_options = args
        .get(3)
        .is_some_and(|v| matches!(v, Value::Container(Array::Struct(_))));
    if fourth_is_options && args.len() == 5 {
        return Err("sim: options must be the last input".into());
    }
    let opts = decode_options(
        e,
        if fourth_is_options {
            args.get(3)
        } else {
            args.get(4)
        },
    )?;
    let open = switch(e, &opts, "OpenLoop")?;
    if !open && count >= 5 && n > crate::script_containers::MAX_CELL_SLOTS / 5 {
        return Err("sim: controller state history exceeds 65536 struct slots".into());
    }
    let plant = simulation_model(e, &m, &field(&opts, "Model")?, out)?;
    let dx = super::prediction::discrete_nominal_dx(e, &plant, out)?;
    let r = signal(
        &args[2].clone().array(&e.cancel)?,
        m.ny(),
        m.nominal.y.data(),
        "reference",
    )?;
    let v = signal(
        &if !fourth_is_options && args.len() >= 4 {
            args[3].clone().array(&e.cancel)?
        } else {
            Array::Numeric(Matrix::filled(0, 0, 0.)?)
        },
        m.md_idx.len(),
        &m.md_idx
            .iter()
            .map(|&j| m.nominal.u.get(j, 0))
            .collect::<Vec<_>>(),
        "measured disturbance",
    )?;
    let ud = signal(
        &field(&opts, "UnmeasuredDisturbance")?,
        plant.ud_idx.len(),
        &plant
            .ud_idx
            .iter()
            .map(|&j| plant.nominal.u.get(j, 0))
            .collect::<Vec<_>>(),
        "UnmeasuredDisturbance",
    )?;
    let input_noise = signal(
        &field(&opts, "InputNoise")?,
        m.n_mv(),
        &vec![0.; m.n_mv()],
        "InputNoise",
    )?;
    let output_noise = signal(
        &field(&opts, "OutputNoise")?,
        m.mo_idx.len(),
        &vec![0.; m.mo_idx.len()],
        "OutputNoise",
    )?;
    let mv_signal = signal(
        &field(&opts, "MVSignal")?,
        m.n_mv(),
        &vec![0.; m.n_mv()],
        "MVSignal",
    )?;
    let initial = field(&opts, "PlantInitialState")?;
    let mut x = if empty(&initial) {
        plant.nominal.x.data().to_vec()
    } else {
        let Array::Numeric(a) = &initial else {
            return Err("sim: PlantInitialState must be a finite vector".into());
        };
        if a.is_nd()
            || a.len() != plant.nx()
            || (a.rows() != 1 && a.cols() != 1)
            || a.data().iter().any(|x| !x.is_finite())
        {
            return Err(format!(
                "sim: PlantInitialState must be a finite vector of length {}",
                plant.nx()
            ));
        }
        a.data().to_vec()
    };
    // One private controller and one private state per run, never per sample.
    let controller = if !switch(e, &opts, "Constraints")? && !open {
        objects::mpc_new(e, unconstrained(&m))?
    } else {
        args[0].clone()
    };
    let initial = field(&opts, "ControllerInitialState")?;
    let state = if empty(&initial) {
        if open {
            None
        } else {
            Some(super::state::construct(e, &m, &[], 1, out)?.remove(0))
        }
    } else {
        let (_, s) = objects::mpc_state_value(e, &Value::from(initial))
            .ok_or("sim: ControllerInitialState must be an mpcstate handle")?;
        if open {
            None
        } else {
            Some(objects::mpc_state_new(e, s.as_ref().clone())?)
        }
    };
    let state_bytes = state
        .as_ref()
        .and_then(|s| objects::mpc_state_value(e, s))
        .map_or(0, |(_, s)| s.bytes());
    let cells = n
        .checked_mul(m.ny() + m.n_mv() + 1 + if count >= 4 { plant.nx() } else { 0 })
        .ok_or("sim: output size overflow")?;
    let bytes = cells.saturating_mul(8).saturating_add(if count >= 5 {
        n.saturating_mul(state_bytes + 5 * 64)
    } else {
        0
    });
    // Reserve the move workspace as well as simulation histories before looping.
    let move_workspace = if open {
        0
    } else {
        super::qp::workspace_bytes(super::qp::MAX_N, super::qp::MAX_M, 0)?
    };
    if bytes
        .saturating_add(move_workspace)
        .saturating_add(plant.bytes())
        .saturating_add(e.workspace_bytes())
        > e.limits.memory_bytes
    {
        return Err("sim: workspace/output exceeds memory limit".into());
    }
    let mut ys = vec![0.; n * m.ny()];
    let mut us = vec![0.; n * m.n_mv()];
    let mut xs = vec![0.; if count >= 4 { n * plant.nx() } else { 0 }];
    let mut states = Vec::with_capacity(if count >= 5 && !open { n * 5 } else { 0 });
    let ref_rows = if switch(e, &opts, "RefLookAhead")? {
        m.prediction_horizon
    } else {
        1
    };
    let md_rows = if switch(e, &opts, "MDLookAhead")? {
        m.prediction_horizon + 1
    } else {
        1
    };
    for k in 0..n {
        e.check_cancel()?;
        if count >= 4 {
            for (j, &value) in x.iter().enumerate() {
                xs[j * n + k] = value;
            }
        }
        let mut inputs = plant.nominal.u.data().to_vec();
        for (j, &i) in plant.md_idx.iter().enumerate() {
            inputs[i] = at(&v, k, j);
        }
        for (j, &i) in plant.ud_idx.iter().enumerate() {
            inputs[i] = at(&ud, k, j);
        }
        let output: Vec<f64> = (0..plant.ny())
            .map(|i| {
                plant.nominal.y.get(i, 0)
                    + (0..plant.nx())
                        .map(|j| plant.cd.get(i, j) * (x[j] - plant.nominal.x.get(j, 0)))
                        .sum::<f64>()
                    + (0..plant.nu())
                        .map(|j| plant.dd.get(i, j) * (inputs[j] - plant.nominal.u.get(j, 0)))
                        .sum::<f64>()
            })
            .collect();
        let mv = if let Some(state) = &state {
            let ym = Matrix::from_column_vec(
                m.mo_idx.len(),
                1,
                m.mo_idx
                    .iter()
                    .enumerate()
                    .map(|(j, &i)| output[i] + at(&output_noise, k, j))
                    .collect(),
            )?;
            let result = super::r#move::outputs(
                e,
                &[
                    controller.clone(),
                    state.clone(),
                    Value::Matrix(ym),
                    preview(&r, k, ref_rows)?,
                    preview(&v, k, md_rows)?,
                ],
                1,
                out,
            )?
            .remove(0);
            let Value::Matrix(mv) = result else {
                unreachable!()
            };
            if count >= 5 {
                snapshot(
                    e,
                    &objects::mpc_state_value(e, state)
                        .ok_or("sim: invalid private state")?
                        .1,
                    &mut states,
                )?;
            }
            mv
        } else {
            Matrix::from_column_vec(
                m.n_mv(),
                1,
                (0..m.n_mv()).map(|j| at(&mv_signal, k, j)).collect(),
            )?
        };
        for (j, &value) in output.iter().enumerate() {
            ys[j * n + k] = value;
        }
        for (j, &i) in plant.mv_idx.iter().enumerate() {
            us[j * n + k] = mv.get(j, 0);
            inputs[i] = mv.get(j, 0) + at(&input_noise, k, j);
        }
        // T stored samples correspond to T-1 plant transitions.
        if k + 1 < n {
            x = (0..plant.nx())
                .map(|i| {
                    plant.nominal.x.get(i, 0)
                        + dx.get(i, 0)
                        + (0..plant.nx())
                            .map(|j| plant.ad.get(i, j) * (x[j] - plant.nominal.x.get(j, 0)))
                            .sum::<f64>()
                        + (0..plant.nu())
                            .map(|j| plant.bd.get(i, j) * (inputs[j] - plant.nominal.u.get(j, 0)))
                            .sum::<f64>()
                })
                .collect();
            if x.iter().any(|v| !v.is_finite()) {
                return Err("sim: plant state became nonfinite".into());
            }
        }
    }
    let y = Value::Matrix(Matrix::from_column_vec(n, m.ny(), ys)?);
    let t = Value::Matrix(Matrix::from_column_vec(
        n,
        1,
        (0..n).map(|k| k as f64 * m.ts).collect(),
    )?);
    let u = Value::Matrix(Matrix::from_column_vec(n, m.n_mv(), us)?);
    if count == 0 {
        e.call("figure", vec![], out)?;
        for (i, values, label) in [(1., y, "Outputs"), (2., u, "Manipulated variables")] {
            e.call(
                "subplot",
                vec![
                    Value::Matrix(Matrix::scalar(2.)),
                    Value::Matrix(Matrix::scalar(1.)),
                    Value::Matrix(Matrix::scalar(i)),
                ],
                out,
            )?;
            // A 1-by-N history is a row vector to plot; pass each channel
            // explicitly so a single stored MIMO sample remains N series.
            let plot_args = if n == 1 {
                let Value::Matrix(history) = values else {
                    unreachable!()
                };
                history
                    .data()
                    .iter()
                    .flat_map(|&value| [t.clone(), Value::Matrix(Matrix::scalar(value))])
                    .collect()
            } else {
                vec![t.clone(), values]
            };
            e.call("plot", plot_args, out)?;
            e.call("title", vec![Value::Text(label.into())], out)?;
            e.call("xlabel", vec![Value::Text("Time".into())], out)?;
        }
        return Ok(vec![]);
    }
    let mut values = vec![y, t, u];
    if count >= 4 {
        values.push(Value::Matrix(Matrix::from_column_vec(n, plant.nx(), xs)?));
    }
    if count >= 5 {
        values.push(Value::Container(Array::Struct(StructArray::from_slots(
            if open { 0 } else { n },
            1,
            super::state::PROPERTIES
                .iter()
                .map(|s| s.to_string())
                .collect(),
            states,
            &e.cancel,
        )?)));
    }
    if count >= 6 {
        values.push(Value::Container(Array::Struct(opts)));
    }
    values.truncate(count);
    Ok(values)
}
