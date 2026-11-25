//! Nichols and singular-value responses, all stability margins and the
//! Nichols grid, independent of the desktop. Calling forms follow the public
//! MathWorks reference pages of nichols, sigma, allmargin and ngrid; no
//! MathWorks code.
//!
//! - nichols samples the same frequency response as bode (same grid and
//!   evaluator); the chart plots open-loop gain in dB against phase.
//! - sigma gives the singular values of H(jw) (H(e^{jwTs}) for discrete
//!   models) from a complex SVD of every sample. Type 1 uses
//!   sv(inv(H)) = 1/sv(H); type 3 forms inv(H)(I + H) by an LU solve. No
//!   explicit inverse is formed.
//! - allmargin lists every crossing found by margin's search (margin keeps the
//!   dominant ones). The delay margin of a unity-gain crossing wc with phase
//!   margin PM is (PM mod 360) * pi / 180 / wc (seconds), divided by Ts for
//!   discrete models (sample periods). MIMO margins are loop-at-a-time with
//!   the other loops closed by unit negative feedback; Stable is the stability
//!   of the closed loop with every loop closed.
//! - ngrid draws contours of constant closed-loop magnitude and phase of
//!   T = G / (1 + G), traced through G = T / (1 - T) (the M and N loci of the
//!   Nichols chart: James, Nichols and Phillips, Theory of Servomechanisms,
//!   McGraw-Hill 1947, ch. 4).
use super::*;
use crate::CellArray;

/// Closed-loop magnitude levels of ngrid (dB).
const FQ_MAGNITUDES: &[f64] = &[
    6., 3., 1., 0.5, 0.25, 0., -1., -3., -6., -12., -20., -40., -60., -80., -100., -120., -140.,
    -160., -180., -200., -220., -240.,
];
/// Closed-loop phase levels of ngrid (degrees, one turn).
const FQ_PHASES: &[f64] = &[
    -1., -5., -10., -20., -30., -50., -90., -120., -150., -180., -210., -240., -270., -310., -330.,
    -340., -350., -355., -359.,
];
/// Most 360-degree turns ngrid draws.
const FQ_TURNS: usize = 10;
/// Complex multiply-adds sigma may spend on SVDs.
const FQ_SVD_WORK: usize = 400_000_000;
const FQ_GRID_COLOR: [f64; 3] = [0.6, 0.6, 0.6];

pub(super) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    match name {
        "ngrid" => {
            if count > 0 {
                return Err("ngrid returns no output".into());
            }
            fq_render(e, out, |e, out| fq_ngrid(e, args, out))?;
            Ok(vec![])
        }
        "allmargin" => {
            if count > 1 {
                return Err("allmargin supports 1 output".into());
            }
            Ok(vec![fq_allmargin(e, args, out)?])
        }
        "nichols" | "sigma" => {
            let parsed = fq_args(name, args)?;
            if count == 0 {
                return fq_render(e, out, |e, out| fq_plot(e, name, &parsed, out)).map(|()| vec![]);
            }
            let max = if name == "nichols" { 3 } else { 2 };
            if count > max {
                return Err(format!("{name} supports 1..{max} outputs"));
            }
            if parsed.systems.len() != 1 || parsed.systems[0].1.is_some() {
                return Err(format!(
                    "{name} returns responses of one system without line specifications"
                ));
            }
            let r = ctr_spectrum(
                e,
                &parsed.systems[0].0,
                parsed.w.as_ref(),
                name == "nichols" && count > 1,
                out,
            )?;
            let mut values = if name == "nichols" {
                let (mag, phase) = ctr_polar(&r);
                let pairs = r.inputs * r.outputs;
                vec![
                    Value::Matrix(Matrix::from_column_vec(r.w.len(), pairs, mag)?),
                    Value::Matrix(Matrix::from_column_vec(r.w.len(), pairs, phase)?),
                    ctr_column(r.w)?,
                ]
            } else {
                let (k, sv) = fq_singular(e, &r, parsed.kind)?;
                let nw = r.w.len();
                vec![
                    Value::Matrix(Matrix::from_column_vec(k, nw, sv)?),
                    ctr_column(r.w)?,
                ]
            };
            values.truncate(count);
            e.charge_function_outputs(&values)?;
            Ok(values)
        }
        _ => Err("Unknown control frequency function".into()),
    }
}

/// Restore all graphics selections and snapshots if a drawing step fails.
fn fq_render(
    e: &mut Engine,
    out: &mut Execution,
    draw: impl FnOnce(&mut Engine, &mut Execution) -> Result<()>,
) -> Result<()> {
    let old = (e.current_figure, e.figure.clone(), e.other_figures.clone());
    let result = draw(e, out);
    if result.is_err() {
        (e.current_figure, e.figure, e.other_figures) = old;
    }
    result
}

struct FqArgs {
    systems: Vec<(Value, Option<String>)>,
    w: Option<Value>,
    kind: u8,
}

/// `sys1[, spec1], sys2[, spec2], ...[, w][, type]` (type for sigma only).
fn fq_args(name: &str, args: &[Value]) -> Result<FqArgs> {
    let usage = if name == "sigma" {
        "Use sigma(sys1[, LineSpec], ...[, w][, type]) with w a frequency vector or {wmin, wmax} and type 0..3"
    } else {
        "Use nichols(sys1[, LineSpec], ...[, w]) with w a frequency vector or {wmin, wmax}"
    };
    let mut systems = Vec::new();
    let mut i = 0;
    while i < args.len() && control_lti::is_lti(&args[i]) {
        let spec = args
            .get(i + 1)
            .and_then(super::super::table_fns::text_scalar);
        systems.push((args[i].clone(), spec.clone()));
        i += if spec.is_some() { 2 } else { 1 };
    }
    if systems.is_empty() {
        return Err(format!(
            "{name} requires an LTI model created by tf, zpk or ss"
        ));
    }
    if systems.len() > 16 {
        return Err("At most 16 systems may be overlaid".into());
    }
    let mut w = None;
    if let Some(v @ (Value::Matrix(_) | Value::Container(Array::Cell(_)))) = args.get(i) {
        w = Some(v.clone());
        i += 1;
    }
    let mut kind = 0;
    if name == "sigma"
        && let Some(v) = args.get(i)
    {
        kind = match v {
            Value::Matrix(m) if m.len() == 1 && [0., 1., 2., 3.].contains(&m.data()[0]) => {
                m.data()[0] as u8
            }
            _ => return Err("sigma type must be 0, 1, 2 or 3".into()),
        };
        i += 1;
    }
    if i != args.len() {
        return Err(usage.into());
    }
    Ok(FqArgs { systems, w, kind })
}

fn fq_cell_scalar(c: &CellArray, i: usize) -> Result<f64> {
    match c.get_linear(i)?.as_ref() {
        Array::Numeric(m) if m.len() == 1 && m.data()[0].is_finite() => Ok(m.data()[0]),
        _ => Err("Frequency range {wmin, wmax} needs two finite real scalars".into()),
    }
}

/// Explicit vector, `{wmin, wmax}` range (clipped at the Nyquist frequency
/// pi/Ts of discrete models) or, for `[]` or none, bode's automatic grid.
pub(super) fn fq_grid(
    e: &mut Engine,
    value: &Value,
    s: &Lti,
    arg: Option<&Value>,
    out: &mut Execution,
) -> Result<Vec<f64>> {
    match arg {
        Some(Value::Container(Array::Cell(c))) => {
            if c.len() != 2 {
                return Err("Frequency range must be a cell {wmin, wmax}".into());
            }
            let (lo, hi) = (fq_cell_scalar(c, 0)?, fq_cell_scalar(c, 1)?);
            let hi = if s.ts != 0. {
                hi.min(PI / s.sample_period())
            } else {
                hi
            };
            if !(lo > 0. && hi > lo) {
                return Err(
                    "Frequency range needs 0 < wmin < wmax (wmin below the Nyquist frequency)"
                        .into(),
                );
            }
            let count = 1000;
            let (a, b) = (lo.log10(), hi.log10());
            let mut w: Vec<f64> = (0..count)
                .map(|k| 10f64.powf(a + (b - a) * k as f64 / (count - 1) as f64))
                .collect();
            w[0] = lo;
            w[count - 1] = hi;
            w.dedup();
            if w.windows(2).any(|v| v[1] <= v[0]) {
                return Err("Frequency range is too narrow to sample".into());
            }
            Ok(w)
        }
        Some(Value::Matrix(m)) if m.is_empty() => ctr_frequency_grid(e, value, s, None, out),
        _ => ctr_frequency_grid(e, value, s, arg, out),
    }
}

/// Singular values (descending) of H, inv(H), I + H or I + inv(H) at every
/// frequency: min(ny, nu) values per frequency, frequency-major.
fn fq_singular(e: &Engine, r: &CtrSpectrum, kind: u8) -> Result<(usize, Vec<f64>)> {
    let (ny, nu, nw) = (r.outputs, r.inputs, r.w.len());
    if kind > 0 && ny != nu {
        return Err("sigma types 1, 2 and 3 require a square system".into());
    }
    let k = ny.min(nu);
    if nw
        .saturating_mul(ny)
        .saturating_mul(nu)
        .saturating_mul(k)
        .saturating_mul(4)
        > FQ_SVD_WORK
    {
        return Err("sigma exceeds its work bound; use fewer frequencies".into());
    }
    let at = |f: usize, row: usize, col: usize| r.h[f + (row + col * ny) * nw];
    let mut sv = Vec::with_capacity(k * nw);
    for f in 0..nw {
        if f.is_multiple_of(128) {
            e.check_cancel()?;
        }
        if ny == 1 && nu == 1 {
            let h = at(f, 0, 0);
            let one = C::new(1., 0.);
            sv.push(match kind {
                0 => h.norm(),
                1 => 1. / h.norm(),
                2 => (one + h).norm(),
                _ => (one + h).norm() / h.norm(),
            });
            continue;
        }
        let h = DMatrix::from_fn(ny, nu, |row, col| at(f, row, col));
        let mut values: Vec<f64> = match kind {
            0 => h.svd(false, false).singular_values.as_slice().to_vec(),
            1 => h
                .svd(false, false)
                .singular_values
                .iter()
                .map(|s| 1. / s)
                .collect(),
            2 => (h + DMatrix::<C>::identity(ny, ny))
                .svd(false, false)
                .singular_values
                .as_slice()
                .to_vec(),
            _ => {
                let sum = &h + DMatrix::<C>::identity(ny, ny);
                match h.lu().solve(&sum) {
                    Some(x) if x.iter().all(|v| v.re.is_finite() && v.im.is_finite()) => {
                        x.svd(false, false).singular_values.as_slice().to_vec()
                    }
                    _ => vec![f64::INFINITY; k],
                }
            }
        };
        values.sort_unstable_by(|a, b| b.total_cmp(a));
        sv.extend(values);
    }
    Ok((k, sv))
}

fn fq_plot(e: &mut Engine, name: &str, a: &FqArgs, out: &mut Execution) -> Result<()> {
    let mut data = Vec::with_capacity(a.systems.len());
    let mut entries = 0usize;
    for (system, spec) in &a.systems {
        let r = ctr_spectrum(e, system, a.w.as_ref(), name == "nichols", out)?;
        entries += 2 * r.h.len() + r.w.len();
        if entries > CTR_ENTRIES {
            return Err("Overlaid response data exceed 32 MiB".into());
        }
        data.push((r, spec.as_deref()));
    }
    if name == "nichols" {
        fq_nichols_plot(e, &data, out)
    } else {
        fq_sigma_plot(e, &data, a.kind, out)
    }
}

fn fq_nichols_plot(
    e: &mut Engine,
    data: &[(CtrSpectrum, Option<&str>)],
    out: &mut Execution,
) -> Result<()> {
    let (p, m) = (data[0].0.outputs, data[0].0.inputs);
    if data.iter().any(|(r, _)| (r.outputs, r.inputs) != (p, m)) {
        return Err("Overlaid systems must have the same input/output dimensions".into());
    }
    if p > 16 || m > 16 {
        return Err("Response plot exceeds 16 rows or columns".into());
    }
    let polar: Vec<_> = data
        .iter()
        .map(|(r, _)| {
            e.check_cancel()?;
            Ok(ctr_polar(r))
        })
        .collect::<Result<_>>()?;
    let host = ctr_host_cell(e, p, m);
    for i in 0..p {
        for j in 0..m {
            let ax = ctr_select_axes(e, p, m, i * m + j + 1, host, out)?;
            let old_hold = e.figure.as_ref().is_some_and(|f| f.hold);
            let trace = i + p * j;
            for (system, ((r, spec), (mag, phase))) in data.iter().zip(&polar).enumerate() {
                if system > 0 {
                    ctr_hold(e, &ax, true, out)?;
                }
                let range = trace * r.w.len()..(trace + 1) * r.w.len();
                let gain = mag[range.clone()].iter().map(|v| 20. * v.log10()).collect();
                ctr_draw_line(e, &ax, &phase[range], gain, false, *spec, out)?;
            }
            ctr_hold(e, &ax, old_hold, out)?;
            e.call(
                "xlabel",
                vec![ax.clone(), Value::Text("Open-loop phase (deg)".into())],
                out,
            )?;
            e.call(
                "ylabel",
                vec![ax.clone(), Value::Text("Open-loop gain (dB)".into())],
                out,
            )?;
            let title = if p * m > 1 {
                format!("Nichols chart: output {}, input {}", i + 1, j + 1)
            } else {
                "Nichols chart".into()
            };
            e.call("title", vec![ax, Value::Text(title)], out)?;
        }
    }
    Ok(())
}

fn fq_sigma_plot(
    e: &mut Engine,
    data: &[(CtrSpectrum, Option<&str>)],
    kind: u8,
    out: &mut Execution,
) -> Result<()> {
    let mut curves = Vec::with_capacity(data.len());
    let mut lines = 0;
    for (r, spec) in data {
        let (k, sv) = fq_singular(e, r, kind)?;
        lines += k;
        curves.push((k, sv, &r.w, *spec));
    }
    if lines > 64 {
        return Err("sigma plot exceeds 64 singular-value lines".into());
    }
    let ax = ctr_select_axes(e, 1, 1, 1, None, out)?;
    let old_hold = e.figure.as_ref().is_some_and(|f| f.hold);
    let mut first = true;
    for (k, sv, w, spec) in &curves {
        for row in 0..*k {
            if !first {
                ctr_hold(e, &ax, true, out)?;
            }
            first = false;
            let db = (0..w.len())
                .map(|f| 20. * sv[row + f * k].log10())
                .collect();
            ctr_draw_line(e, &ax, w, db, true, *spec, out)?;
        }
    }
    ctr_hold(e, &ax, old_hold, out)?;
    e.call(
        "xlabel",
        vec![
            ax.clone(),
            Value::Text("Frequency (rad / time unit)".into()),
        ],
        out,
    )?;
    e.call(
        "ylabel",
        vec![ax.clone(), Value::Text("Singular values (dB)".into())],
        out,
    )?;
    e.call(
        "title",
        vec![ax, Value::Text("Singular values".into())],
        out,
    )?;
    Ok(())
}

// ------------------------------------------------------------------ allmargin

const FQ_FIELDS: [&str; 7] = [
    "GainMargin",
    "GMFrequency",
    "PhaseMargin",
    "PMFrequency",
    "DelayMargin",
    "DMFrequency",
    "Stable",
];

fn fq_row(v: Vec<f64>) -> Result<Arc<Array>> {
    Ok(Arc::new(Array::Numeric(Matrix::from_column_vec(
        1,
        v.len(),
        v,
    )?)))
}

/// Sorted by frequency, without the repeat a crossing on a grid point gives
/// in both adjacent intervals.
fn fq_sorted(mut v: Vec<(f64, f64)>, focus: Option<[f64; 2]>) -> Vec<(f64, f64)> {
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    v.dedup_by(|b, a| (b.0 - a.0).abs() <= 1e-12 * a.0.abs().max(b.0.abs()));
    if let Some([lo, hi]) = focus {
        v.retain(|x| x.0 >= lo && x.0 <= hi);
    }
    v
}

/// The seven fields of one allmargin element.
fn fq_margin_slots(
    c: &CtrCrossings,
    ts: f64,
    stable: f64,
    focus: Option<[f64; 2]>,
) -> Result<Vec<Arc<Array>>> {
    let gain = fq_sorted(c.gain.clone(), focus);
    let phase = fq_sorted(c.phase.clone(), focus);
    let delay: Vec<f64> = phase
        .iter()
        .map(|&(wc, pm)| {
            let seconds = pm.rem_euclid(360.) * PI / 180. / wc;
            if ts > 0. { seconds / ts } else { seconds }
        })
        .collect();
    Ok(vec![
        fq_row(gain.iter().map(|x| x.1).collect())?,
        fq_row(gain.iter().map(|x| x.0).collect())?,
        fq_row(phase.iter().map(|x| x.1).collect())?,
        fq_row(phase.iter().map(|x| x.0).collect())?,
        fq_row(delay)?,
        fq_row(phase.iter().map(|x| x.0).collect())?,
        Arc::new(Array::Numeric(Matrix::scalar(stable))),
    ])
}

fn fq_struct(e: &Engine, rows: usize, slots: Vec<Arc<Array>>) -> Result<Value> {
    Ok(Value::Container(Array::Struct(StructArray::from_slots(
        rows,
        1,
        FQ_FIELDS.iter().map(|s| (*s).into()).collect(),
        slots,
        &e.cancel,
    )?)))
}

fn fq_allmargin(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Value> {
    let usage = "Use allmargin(sys[, 'Focus', [fmin fmax]]) or allmargin(mag, phase, w[, ts])";
    if let Some(first) = args.first()
        && !control_lti::is_lti(first)
    {
        if !(3..=4).contains(&args.len()) {
            return Err(usage.into());
        }
        let (mag, phase, w) = (
            ctr_vector(&args[0])?,
            ctr_vector(&args[1])?,
            ctr_vector(&args[2])?,
        );
        let ts = match args.get(3) {
            None => 0.,
            Some(v) => match ctr_real(v)?.data() {
                [t] if *t >= 0. => *t,
                _ => return Err("allmargin sample time must be a nonnegative scalar".into()),
            },
        };
        let crossings = ctr_crossings(e, &mag, &phase, &w, None, None)?;
        let slots = fq_margin_slots(&crossings, ts, f64::NAN, None)?;
        return fq_struct(e, 1, slots);
    }
    let focus = match args {
        [_] => None,
        [_, key, range]
            if super::super::table_fns::text_scalar(key)
                .is_some_and(|k| k.eq_ignore_ascii_case("focus")) =>
        {
            match ctr_real(range)?.data() {
                [lo, hi] if *lo >= 0. && hi > lo => Some([*lo, *hi]),
                _ => return Err("allmargin Focus must be [fmin fmax] with 0 <= fmin < fmax".into()),
            }
        }
        _ => return Err(usage.into()),
    };
    let mut s = ctr_lti(e, &args[0])?;
    let (inputs, outputs) = s.ports();
    if inputs != outputs || inputs == 0 {
        return Err("allmargin requires a SISO or square MIMO loop transfer".into());
    }
    if inputs > 1 && s.rational_grid() {
        let (a, b, c, d) = e.ctl_ss(&s, out)?;
        s.model = Model::Ss(a, b, c, d);
    }
    let stable = fq_stable(e, &s, out)?;
    let mut slots = Vec::with_capacity(7 * inputs);
    for j in 0..inputs {
        let channel = if inputs == 1 {
            args[0].clone()
        } else {
            fq_channel(e, &s, j, out)?
        };
        let (_, crossings) = ctr_margin_crossings(e, &[channel], false, out)?;
        slots.extend(fq_margin_slots(
            &crossings,
            s.sample_period(),
            stable,
            focus,
        )?);
    }
    fq_struct(e, inputs, slots)
}

/// Loop j of a square ss loop transfer with every other loop closed by unit
/// negative feedback (u_o = -y_o), as an ss model.
fn fq_channel(e: &mut Engine, s: &Lti, j: usize, out: &mut Execution) -> Result<Value> {
    let Model::Ss(a, b, c, d) = &s.model else {
        return Err("MIMO margins need a state-space model".into());
    };
    let (a, b, c, d) = (ctr_dm(a), ctr_dm(b), ctr_dm(c), ctr_dm(d));
    let (n, m) = (a.nrows(), d.nrows());
    let others: Vec<usize> = (0..m).filter(|&k| k != j).collect();
    let pick_rows = |x: &DMatrix<f64>, rows: &[usize]| {
        DMatrix::from_fn(rows.len(), x.ncols(), |r, col| x[(rows[r], col)])
    };
    let pick_cols = |x: &DMatrix<f64>, cols: &[usize]| {
        DMatrix::from_fn(x.nrows(), cols.len(), |r, col| x[(r, cols[col])])
    };
    let d_oo = pick_cols(&pick_rows(&d, &others), &others);
    let closing = DMatrix::<f64>::identity(m - 1, m - 1) + d_oo;
    let mut rhs = DMatrix::zeros(m - 1, n + 1);
    rhs.view_mut((0, 0), (m - 1, n))
        .copy_from(&pick_rows(&c, &others));
    rhs.view_mut((0, n), (m - 1, 1))
        .copy_from(&pick_cols(&pick_rows(&d, &others), &[j]));
    let k = closing
        .lu()
        .solve(&rhs)
        .filter(|k| k.iter().all(|v| v.is_finite()))
        .ok_or("allmargin: closing the other loops is ill-posed (I + D is singular)")?;
    let (kx, ku) = (k.columns(0, n).into_owned(), k.columns(n, 1).into_owned());
    let b_o = pick_cols(&b, &others);
    let d_jo = pick_cols(&pick_rows(&d, &[j]), &others);
    let aj = &a - &b_o * &kx;
    let bj = pick_cols(&b, &[j]) - &b_o * &ku;
    let cj = pick_rows(&c, &[j]) - &d_jo * &kx;
    let dj = pick_cols(&pick_rows(&d, &[j]), &[j]) - &d_jo * &ku;
    let call = vec![
        Value::Matrix(ctr_matrix(&aj)?),
        Value::Matrix(ctr_matrix(&bj)?),
        Value::Matrix(ctr_matrix(&cj)?),
        Value::Matrix(ctr_matrix(&dj)?),
        Value::Matrix(Matrix::scalar(s.ts)),
    ];
    e.call("ss", call, out)
}

/// 1 when the loop closed by unit negative feedback is stable, 0 when not,
/// NaN when the closed loop is ill-posed.
fn fq_stable(e: &mut Engine, s: &Lti, out: &mut Execution) -> Result<f64> {
    if s.delay() != 0. || s.has_internal_delays() {
        return Ok(f64::NAN);
    }
    let poles: Vec<C> = match &s.model {
        Model::Ss(a, b, c, d) => {
            let m = d.rows();
            let closing = DMatrix::<f64>::identity(m, m) + ctr_dm(d);
            let Some(k) = closing
                .lu()
                .solve(&ctr_dm(c))
                .filter(|k| k.iter().all(|v| v.is_finite()))
            else {
                return Ok(f64::NAN);
            };
            if a.rows() == 0 {
                return Ok(1.);
            }
            let closed = ctr_dm(a) - ctr_dm(b) * k;
            match e.call("eig", vec![Value::Matrix(ctr_matrix(&closed)?)], out)? {
                Value::Complex(m) => m.data().to_vec(),
                Value::Matrix(m) => m.data().iter().map(|&x| C::new(x, 0.)).collect(),
                _ => return Err("Invalid closed-loop eigenvalue result".into()),
            }
        }
        _ => {
            let (num, den) = e.ctl_tf(s, out)?;
            let size = num.len().max(den.len());
            let mut sum = vec![0.; size];
            for (p, offset) in [(&num, size - num.len()), (&den, size - den.len())] {
                for (slot, v) in sum[offset..].iter_mut().zip(p.data()) {
                    *slot += v;
                }
            }
            let Some(first) = sum.iter().position(|&v| v != 0.) else {
                return Ok(f64::NAN);
            };
            if first + 1 == size {
                return Ok(1.);
            }
            let row = Matrix::from_column_vec(1, size - first, sum[first..].to_vec())?;
            match e.call("roots", vec![Value::Matrix(row)], out)? {
                Value::Complex(m) => m.data().to_vec(),
                Value::Matrix(m) => m.data().iter().map(|&x| C::new(x, 0.)).collect(),
                _ => return Err("Invalid closed-loop root result".into()),
            }
        }
    };
    let stable = if s.ts != 0. {
        poles.iter().all(|p| p.norm() < 1.)
    } else {
        poles.iter().all(|p| p.re < 0.)
    };
    Ok(if stable { 1. } else { 0. })
}

// ---------------------------------------------------------------------- ngrid

/// Open-loop (phase in (-360, 0], gain dB) of closed-loop values T, with NaN
/// breaks where the phase wraps.
fn fq_locus(points: impl Iterator<Item = C>) -> (Vec<f64>, Vec<f64>) {
    let (mut x, mut y) = (Vec::new(), Vec::new());
    let one = C::new(1., 0.);
    for t in points {
        let g = t / (one - t);
        if !g.re.is_finite() || !g.im.is_finite() || g.norm() == 0. {
            continue;
        }
        let mut phase = g.arg() * 180. / PI;
        if phase > 0. {
            phase -= 360.;
        }
        if let Some(&last) = x.last()
            && f64::abs(phase - last) > 180.
        {
            x.push(f64::NAN);
            y.push(f64::NAN);
        }
        x.push(phase);
        y.push(20. * g.norm().log10());
    }
    (x, y)
}

fn fq_magnitude_locus(db: f64) -> (Vec<f64>, Vec<f64>) {
    let m = 10f64.powf(db / 20.);
    let count = 720;
    fq_locus((0..count).map(|k| {
        let psi = -2. * PI * (k as f64 + 0.5) / count as f64;
        C::from_polar(m, psi)
    }))
}

fn fq_phase_locus(degrees: f64) -> (Vec<f64>, Vec<f64>) {
    let psi = degrees * PI / 180.;
    let levels = (0..48)
        .map(|k| -300. + 5. * k as f64)
        .chain((0..20).map(|k| -60. + 2. * k as f64))
        .chain((0..400).map(|k| -20. + 0.1 * k as f64))
        .chain((0..=40).map(|k| 20. + k as f64));
    fq_locus(levels.map(|db| C::from_polar(10f64.powf(db / 20.), psi)))
}

/// Phase limits rounded out to whole turns and gain limits covering at least
/// -40..40 dB, from the current axes (explicit limits are kept).
fn fq_ngrid_limits(e: &Engine) -> ([f64; 2], [f64; 2]) {
    let default = ([-360., 0.], [-40., 40.]);
    let Some(axes) = e.figure.as_ref().map(|f| &f.axes) else {
        return default;
    };
    if axes.series.is_empty() && axes.xlim.is_none() && axes.ylim.is_none() {
        return default;
    }
    let (bx, by) = axes.bounds();
    let x = axes.xlim.unwrap_or(bx);
    let y = axes.ylim.unwrap_or(by);
    let x = if axes.xlim.is_some() {
        x
    } else if x[0].is_finite() && x[1].is_finite() {
        let lo = (x[0] / 360.).floor() * 360.;
        let hi = ((x[1] / 360.).ceil() * 360.).max(lo + 360.);
        [lo, hi]
    } else {
        default.0
    };
    let y = if axes.ylim.is_some() {
        y
    } else if y[0].is_finite() && y[1].is_finite() {
        [
            ((y[0] / 10.).floor() * 10.).min(-40.),
            ((y[1] / 10.).ceil() * 10.).max(40.),
        ]
    } else {
        default.1
    };
    (x, y)
}

fn fq_ngrid(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<()> {
    let new = match args {
        [] => false,
        [v] if super::super::table_fns::text_scalar(v).is_some_and(|t| t == "new") => true,
        _ => return Err("Use ngrid or ngrid('new')".into()),
    };
    let ax = e.call("gca", vec![], out)?;
    // 'new' draws the first contours with hold off, which clears the axes.
    let ([x0, x1], [y0, y1]) = if new {
        ([-360., 0.], [-40., 40.])
    } else {
        fq_ngrid_limits(e)
    };
    if !(x0.is_finite() && x1.is_finite() && x1 > x0 && y0.is_finite() && y1.is_finite() && y1 > y0)
    {
        return Err("ngrid needs finite axes limits".into());
    }
    // Turns q with (-360, 0] + 360 q meeting [x0, x1]; at most FQ_TURNS, rightmost kept.
    let first = (x0 / 360.).floor() as i64 + 1;
    let last = (x1 / 360.).ceil() as i64;
    let first = first.max(last - FQ_TURNS as i64 + 1);
    let old_hold = e.figure.as_ref().is_some_and(|f| f.hold);
    ctr_hold(e, &ax, !new, out)?;
    let mut labels: (Vec<f64>, Vec<f64>, Vec<String>) = (Vec::new(), Vec::new(), Vec::new());
    let inside = |x: f64, y: f64| x > x0 && x < x1 && y > y0 && y < y1;
    for (loci, magnitude) in [
        (
            FQ_MAGNITUDES
                .iter()
                .map(|&db| (db, fq_magnitude_locus(db)))
                .collect::<Vec<_>>(),
            true,
        ),
        (
            FQ_PHASES.iter().map(|&p| (p, fq_phase_locus(p))).collect(),
            false,
        ),
    ] {
        e.check_cancel()?;
        let (mut xs, mut ys) = (Vec::new(), Vec::new());
        for (level, (x, y)) in &loci {
            for turn in first..=last {
                let shift = 360. * turn as f64;
                if !xs.is_empty() {
                    xs.push(f64::NAN);
                    ys.push(f64::NAN);
                }
                xs.extend(x.iter().map(|v| v + shift));
                ys.extend_from_slice(y);
            }
            // One label per contour, on the rightmost turn, inside the view:
            // magnitude levels at their highest visible point, phase levels
            // near the bottom of the view.
            let shift = 360. * last as f64;
            let visible = x
                .iter()
                .zip(y)
                .map(|(&a, &b)| (a + shift, b))
                .filter(|&(a, b)| inside(a, b));
            let target = y0 + 0.06 * (y1 - y0);
            let best = if magnitude {
                visible.max_by(|a, b| a.1.total_cmp(&b.1))
            } else {
                visible.min_by(|a, b| (a.1 - target).abs().total_cmp(&(b.1 - target).abs()))
            };
            if let Some((a, b)) = best {
                labels.0.push(a);
                labels.1.push(b);
                labels.2.push(if magnitude {
                    format!("{level} dB")
                } else {
                    format!("{level} deg")
                });
            }
        }
        let mut call = vec![
            ax.clone(),
            ctr_column(xs)?,
            ctr_column(ys)?,
            Value::Text(":".into()),
            Value::Text("Color".into()),
            Value::Matrix(Matrix::from_column_vec(1, 3, FQ_GRID_COLOR.to_vec())?),
        ];
        call.push(Value::Text("LineWidth".into()));
        call.push(Value::Matrix(Matrix::scalar(0.5)));
        e.call("plot", call, out)?;
        ctr_hold(e, &ax, true, out)?;
    }
    if !labels.2.is_empty() {
        let n = labels.2.len();
        let texts = labels
            .2
            .iter()
            .map(|t| {
                crate::CharacterRow::from_text(t, &e.cancel).map(|c| Arc::new(Array::Character(c)))
            })
            .collect::<Result<Vec<_>>>()?;
        e.call(
            "text",
            vec![
                ctr_column(labels.0)?,
                ctr_column(labels.1)?,
                Value::Container(Array::Cell(CellArray::from_shared(n, 1, texts, &e.cancel)?)),
                Value::Text("FontSize".into()),
                Value::Matrix(Matrix::scalar(8.)),
                Value::Text("Color".into()),
                Value::Matrix(Matrix::from_column_vec(1, 3, vec![0.45, 0.45, 0.45])?),
            ],
            out,
        )?;
    }
    ctr_hold(e, &ax, old_hold || new, out)?;
    let limits = |a: f64, b: f64| -> Result<Value> {
        Ok(Value::Matrix(Matrix::from_column_vec(1, 2, vec![a, b])?))
    };
    e.call("xlim", vec![ax.clone(), limits(x0, x1)?], out)?;
    e.call("ylim", vec![ax, limits(y0, y1)?], out)?;
    Ok(())
}
