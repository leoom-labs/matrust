//! Exact transport-delay sampling on the requested time grid.
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn ctr_time_paths(
    e: &mut Engine,
    model: &Lti,
    name: &str,
    t: Vec<f64>,
    u: &Matrix,
    initial: DMatrix<f64>,
    foh: bool,
    save_states: bool,
    out: &mut Execution,
) -> Result<CtrTime> {
    let time = Value::Matrix(Matrix::from_column_slice(t.len(), 1, &t)?);
    let mut response = vec![0.; t.len()];
    let mut states = Vec::new();
    let mut offset = 0;
    for path in model.full_delay_paths() {
        e.check_cancel()?;
        let count = path.states();
        let initial = || {
            Matrix::from_column_slice(count, 1, &initial.as_slice()[offset..offset + count])
                .map(Value::Matrix)
        };
        let encoded = control_lti::encode(e, &path)?;
        let args = match name {
            "lsim" => vec![
                encoded,
                Value::Matrix(u.clone()),
                time.clone(),
                initial()?,
                Value::Text(if foh { "foh" } else { "zoh" }.into()),
            ],
            "initial" => vec![encoded, initial()?, time.clone()],
            _ => vec![encoded, time.clone()],
        };
        let path = ctr_time_response(e, name, &args, save_states, out)?;
        for (y, contribution) in response.iter_mut().zip(path.y.data()) {
            *y += contribution;
        }
        states.extend_from_slice(path.x.data());
        offset += count;
    }
    if response.iter().any(|y| !y.is_finite()) {
        return Err("Time response overflowed".into());
    }
    Ok(CtrTime {
        y: Matrix::from_column_vec(t.len(), 1, response)?,
        x: Matrix::from_column_vec(t.len(), if save_states { offset } else { 0 }, states)?,
        t,
        outputs: 1,
        inputs: 1,
    })
}

pub(super) struct CtrDelayedTime<'a> {
    pub(super) a: &'a Matrix,
    pub(super) b: &'a Matrix,
    pub(super) c: &'a Matrix,
    pub(super) d: &'a Matrix,
    pub(super) ts: f64,
    pub(super) t: Vec<f64>,
    pub(super) foh: bool,
    pub(super) propagation: CtrPropagation,
    pub(super) delays: [f64; 3],
    pub(super) save_states: bool,
}
struct CtrTrace {
    y: Vec<f64>,
    x: Vec<f64>,
}
fn ctr_delayed_trace(
    e: &mut Engine,
    sim: &CtrDelayedTime<'_>,
    delay: f64,
    mut state: DMatrix<f64>,
    input: impl Fn(usize) -> f64,
    save_states: bool,
    out: &mut Execution,
) -> Result<CtrTrace> {
    let len = sim.t.len();
    let n = sim.a.rows();
    let mut trace = CtrTrace {
        y: vec![0.; len],
        x: vec![0.; if save_states { len * n } else { 0 }],
    };
    let dt = sim.t[1];
    let steps = delay / dt;
    let nearest = steps.round();
    let steps = if (steps - nearest).abs() <= 16. * f64::EPSILON * steps {
        nearest
    } else {
        steps
    };
    if steps > (len - 1) as f64 {
        return Ok(trace);
    }
    let start = steps.ceil() as usize;
    let fraction = start as f64 - steps;
    let partial = if fraction == 0. || sim.ts > 0. {
        None
    } else {
        Some(ctr_propagation(
            e,
            sim.a,
            sim.b,
            0.,
            fraction * dt,
            sim.foh,
            out,
        )?)
    };
    let c = ctr_dm(sim.c);
    let advance =
        |p: &CtrPropagation, state: &DMatrix<f64>, u, next| &p.a * state + &p.b0 * u + &p.b1 * next;
    for k in start..len {
        e.check_cancel()?;
        let j = k - start;
        let u = input(j);
        let next = input(j + 1);
        let observed_u = if sim.foh {
            u + fraction * (next - u)
        } else {
            u
        };
        let at_sample = partial.as_ref().map(|p| advance(p, &state, u, observed_u));
        let observed = at_sample.as_ref().unwrap_or(&state);
        let y = (&c * observed)[(0, 0)] + sim.d.data()[0] * observed_u;
        if !y.is_finite() || observed.iter().any(|v| !v.is_finite()) {
            return Err("Time response overflowed".into());
        }
        trace.y[k] = y;
        if save_states {
            for (i, value) in observed.iter().enumerate() {
                trace.x[k + len * i] = *value;
            }
        }
        if k + 1 < len {
            state = advance(&sim.propagation, &state, u, next);
        }
    }
    Ok(trace)
}
pub(super) fn ctr_delayed_time(
    e: &mut Engine,
    sim: CtrDelayedTime<'_>,
    name: &str,
    u: &Matrix,
    initial: DMatrix<f64>,
    out: &mut Execution,
) -> Result<CtrTime> {
    let len = sim.t.len();
    let n = sim.a.rows();
    let save_states = sim.save_states;
    let [input_delay, output_delay, io_delay] = sim.delays;
    let drive_delay = input_delay + io_delay;
    let forced_delay = drive_delay + output_delay;
    let has_initial = initial.iter().any(|&v| v != 0.);
    let passes = 1
        + usize::from(save_states && output_delay != 0.)
        + usize::from(has_initial && name != "initial") * (1 + usize::from(output_delay != 0.));
    ctr_admit(n, 1, 1, passes, len)?;
    let input = |k: usize| match name {
        "step" => 1.,
        "impulse" if sim.ts > 0. && k == 0 => 1. / sim.ts,
        "lsim" => u.get(k.min(len - 1), 0),
        _ => 0.,
    };
    let state = if name == "impulse" && sim.ts == 0. {
        ctr_dm(sim.b)
    } else {
        DMatrix::zeros(n, 1)
    };
    let mut trace = if name == "initial" {
        ctr_delayed_trace(
            e,
            &sim,
            output_delay,
            initial.clone(),
            |_| 0.,
            save_states && output_delay == 0.,
            out,
        )?
    } else {
        ctr_delayed_trace(
            e,
            &sim,
            forced_delay,
            state.clone(),
            input,
            save_states && output_delay == 0.,
            out,
        )?
    };
    if save_states && output_delay != 0. {
        let state_delay = if name == "initial" { 0. } else { drive_delay };
        let state = if name == "initial" {
            initial.clone()
        } else {
            state
        };
        trace.x = ctr_delayed_trace(e, &sim, state_delay, state, input, true, out)?.x;
    }
    if has_initial && name != "initial" {
        let free = ctr_delayed_trace(
            e,
            &sim,
            output_delay,
            initial.clone(),
            |_| 0.,
            save_states && output_delay == 0.,
            out,
        )?;
        for (value, free) in trace.y.iter_mut().zip(free.y) {
            *value += free;
        }
        let free_x = if save_states && output_delay != 0. {
            ctr_delayed_trace(e, &sim, 0., initial, |_| 0., true, out)?.x
        } else {
            free.x
        };
        for (value, free) in trace.x.iter_mut().zip(free_x) {
            *value += free;
        }
    }
    if trace.y.iter().chain(&trace.x).any(|v| !v.is_finite()) {
        return Err("Time response overflowed".into());
    }
    Ok(CtrTime {
        y: Matrix::from_column_vec(len, 1, trace.y)?,
        x: Matrix::from_column_vec(len, if save_states { n } else { 0 }, trace.x)?,
        t: sim.t,
        inputs: 1,
        outputs: 1,
    })
}
