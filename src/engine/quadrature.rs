//! integral, quadgk, integral2, integral3, quad, dblquad and triplequad.
//!
//! integral and quadgk follow Shampine's vectorised adaptive Gauss-Kronrod (7,15)
//! quadrature ("Vectorized adaptive quadrature in MATLAB", 2008) as MATLAB's
//! quadgk does: finite intervals are mapped with x = (b-a)/4 t (3 - t^2) + (b+a)/2
//! to weaken endpoint singularities, infinite ones with t/(1-t) and t/(1-t^2),
//! and the interval starts as 10 pieces. integral2 and integral3 nest integral
//! (MATLAB's 'iterated' method). quad is adaptive Simpson with one Romberg step;
//! dblquad and triplequad nest quad. Integrands are called with a row of points.
use super::{Engine, Execution, Value};
use crate::matrix::MAX_ELEMENTS;
use crate::{Array, Complex64, ComplexMatrix, FunctionHandle, Matrix, Result};
use std::sync::atomic::AtomicBool;

type C = Complex64;

const ZERO: C = C::new(0., 0.);

/// Positive Gauss-Kronrod nodes; the rule is symmetric about 0.
const PNODES: [f64; 7] = [
    0.2077849550078985,
    0.4058451513773972,
    0.5860872354676911,
    0.7415311855993944,
    0.8648644233597691,
    0.9491079123427585,
    0.9914553711208126,
];
const PWT: [f64; 7] = [
    0.2044329400752989,
    0.1903505780647854,
    0.1690047266392679,
    0.1406532597155259,
    0.1047900103222502,
    0.06309209262997855,
    0.02293532201052922,
];
const PWT7: [f64; 7] = [
    0.,
    0.3818300505051189,
    0.,
    0.2797053914892767,
    0.,
    0.1294849661688697,
    0.,
];
const CENTER_WT: f64 = 0.2094821410847278;
const CENTER_WT7: f64 = 0.4179591836734694;
/// Pieces the first mesh is split into.
const MIN_INTERVALS: usize = 10;
/// Integrand values one statement may request before stopping with an error.
const MAX_POINTS: usize = 20_000_000;

fn quadx_nodes() -> ([f64; 15], [f64; 15], [f64; 15]) {
    let mut nodes = [0.; 15];
    let mut wt = [0.; 15];
    let mut ewt = [0.; 15];
    for k in 0..7 {
        nodes[k] = -PNODES[6 - k];
        nodes[14 - k] = PNODES[6 - k];
        wt[k] = PWT[6 - k];
        wt[14 - k] = PWT[6 - k];
        ewt[k] = PWT[6 - k] - PWT7[6 - k];
        ewt[14 - k] = PWT[6 - k] - PWT7[6 - k];
    }
    wt[7] = CENTER_WT;
    ewt[7] = CENTER_WT - CENTER_WT7;
    (nodes, wt, ewt)
}

fn quadx_is_quadrature(name: &str) -> bool {
    matches!(
        name,
        "integral" | "integral2" | "integral3" | "quad" | "quadgk" | "dblquad" | "triplequad"
    )
}

/// Functions with a second output: [q, errbnd] = quadgk(...), [q, fcnt] = quad(...).
pub(super) fn quadx_has_outputs(name: &str) -> bool {
    matches!(name, "quadgk" | "quad")
}

/// Select warnings for the calling integration function.
#[derive(Clone, Copy, PartialEq)]
enum Caller {
    Integral,
    Quadgk,
}

#[derive(Clone, Copy)]
struct Tolerances {
    rel: f64,
    abs: f64,
    max_intervals: usize,
}

/// Why the adaptive loop stopped before meeting the tolerance.
enum Stop {
    MinStep(f64),
    MaxIntervals(f64, usize),
    NonFinite,
}

struct GkResult {
    q: Vec<C>,
    errbnd: f64,
    stop: Option<Stop>,
}

/// Evaluates m values per point (point-major) for a row of points.
type Integrand<'a> = dyn FnMut(&[f64]) -> Result<Vec<C>> + 'a;
/// Point-major values, with optional absolute errors from inner quadratures.
struct GkBatch {
    values: Vec<C>,
    errors: Vec<f64>,
}

impl GkBatch {
    fn quadx_exact(values: Vec<C>) -> Self {
        Self {
            values,
            errors: Vec::new(),
        }
    }
}

type GkIntegrand<'a> = dyn FnMut(&[f64]) -> Result<GkBatch> + 'a;
/// The integrand in the mapped variable; `None` when mesh points got too close.
type MappedIntegrand<'a> = dyn FnMut(&[f64]) -> Result<Option<GkBatch>> + 'a;

#[derive(Clone, Copy)]
enum Map {
    Finite,
    RightInfinite,
    LeftInfinite,
    BothInfinite,
}

/// Splits pieces of the mesh so that none is longer than 1/minsubs of the path,
/// drops pieces of zero length and returns the path length (quadgk's split).
fn quadx_split(mut x: Vec<f64>, minsubs: usize) -> (Vec<f64>, f64) {
    let pathlen = x[x.len() - 1] - x[0];
    if pathlen > 0. {
        let udelta = minsubs as f64 / pathlen;
        for k in (0..x.len() - 1).rev() {
            let n = ((x[k + 1] - x[k]).abs() * udelta).ceil() - 1.;
            if n > 0. {
                let n = n as usize;
                let (left, right) = (x[k], x[k + 1]);
                let new: Vec<f64> = (1..=n)
                    .map(|i| left + (i as f64 / (n + 1) as f64) * (right - left))
                    .collect();
                x.splice(k + 1..k + 1, new);
            }
        }
    }
    let mut kept = Vec::with_capacity(x.len());
    for k in 0..x.len() {
        if k + 1 < x.len() && (x[k + 1] - x[k]).abs() == 0. {
            continue;
        }
        kept.push(x[k]);
    }
    if kept.len() == 1 {
        kept.push(kept[0]);
    }
    (kept, pathlen)
}

/// First point whose neighbour is within 100 eps (quadgk's checkSpacing).
fn quadx_too_close(x: &[f64]) -> Option<f64> {
    x.windows(2)
        .find(|w| (w[1] - w[0]).abs() <= 100. * f64::EPSILON * w[0].abs().max(w[1].abs()))
        .map(|w| w[0])
}

fn quadx_finite(v: C) -> bool {
    v.re.is_finite() && v.im.is_finite()
}

fn quadx_batch_elements(points: usize, components: usize) -> Result<usize> {
    points
        .checked_mul(components)
        .filter(|n| *n <= MAX_ELEMENTS)
        .ok_or_else(|| format!("Integration batch exceeds the {MAX_ELEMENTS}-element limit"))
}

/// Adaptive Gauss-Kronrod over [a, b] with m-valued integrands (m = 1 unless
/// 'ArrayValued'). Infinite limits and waypoints as in quadgk.
fn quadx_gk(
    f: &mut GkIntegrand,
    m: usize,
    a: f64,
    b: f64,
    waypoints: &[f64],
    tol: Tolerances,
    cancel: &AtomicBool,
) -> Result<GkResult> {
    if a.is_nan() || b.is_nan() {
        return Ok(GkResult {
            q: vec![C::new(f64::NAN, 0.); m],
            errbnd: f64::NAN,
            stop: None,
        });
    }
    let (lo, hi, reverse) = if b < a { (b, a, true) } else { (a, b, false) };
    if lo == hi {
        return Ok(GkResult {
            q: vec![ZERO; m],
            errbnd: 0.,
            stop: None,
        });
    }
    let mut inner: Vec<f64> = waypoints
        .iter()
        .copied()
        .filter(|w| *w > lo && *w < hi)
        .collect();
    inner.sort_by(f64::total_cmp);
    let (map, tinterval) = match (lo.is_finite(), hi.is_finite()) {
        (true, true) => {
            let mut t = vec![-1.];
            t.extend(
                inner
                    .iter()
                    .map(|w| 2. * (((lo + hi - 2. * w) / (lo - hi)).asin() / 3.).sin()),
            );
            t.push(1.);
            (Map::Finite, t)
        }
        (true, false) => {
            let mut t = vec![0.];
            t.extend(inner.iter().map(|w| {
                let u = (w - lo).sqrt();
                u / (1. + u)
            }));
            t.push(1.);
            (Map::RightInfinite, t)
        }
        (false, true) => {
            let mut t = vec![-1.];
            t.extend(inner.iter().map(|w| {
                let u = (hi - w).sqrt();
                -u / (1. + u)
            }));
            t.push(0.);
            (Map::LeftInfinite, t)
        }
        (false, false) => {
            let mut t = vec![-1.];
            t.extend(
                inner
                    .iter()
                    .map(|w| 2. * w / (1. + (1. + 4. * w * w).sqrt())),
            );
            t.push(1.);
            (Map::BothInfinite, t)
        }
    };
    let mut first = true;
    let mut close = None;
    let mut transformed = |t: &[f64]| -> Result<Option<GkBatch>> {
        let mut x = Vec::with_capacity(t.len());
        let mut w = Vec::with_capacity(t.len());
        for &t in t {
            let (xi, wi) = match map {
                Map::Finite => (
                    0.25 * (hi - lo) * t * (3. - t * t) + 0.5 * (hi + lo),
                    (0.75 * (hi - lo), 1. - t * t, 1.),
                ),
                Map::RightInfinite => {
                    let tt = t / (1. - t);
                    (lo + tt * tt, (2. * tt, 1., (1. - t) * (1. - t)))
                }
                Map::LeftInfinite => {
                    let tt = t / (1. + t);
                    (hi - tt * tt, (-2. * tt, 1., (1. + t) * (1. + t)))
                }
                Map::BothInfinite => {
                    let tt = t / (1. - t * t);
                    (tt, (1., 1. + t * t, (1. - t * t) * (1. - t * t)))
                }
            };
            x.push(xi);
            w.push(wi);
        }
        if !first && let Some(at) = quadx_too_close(&x) {
            close = Some(at);
            return Ok(None);
        }
        first = false;
        let mut y = f(&x)?;
        for (i, (pre, post, div)) in w.into_iter().enumerate() {
            for v in &mut y.values[i * m..(i + 1) * m] {
                *v = *v * pre * post / div;
            }
            if !y.errors.is_empty() {
                for e in &mut y.errors[i * m..(i + 1) * m] {
                    *e *= (pre * post / div).abs();
                }
            }
        }
        Ok(Some(y))
    };
    let mut result = quadx_vadapt(&mut transformed, m, tinterval, tol, cancel)?;
    if let Some(Stop::MinStep(at)) = &mut result.stop {
        *at = close.unwrap_or(*at);
    }
    if reverse {
        for v in &mut result.q {
            *v = -*v;
        }
    }
    Ok(result)
}

/// quadgk's vadapt: refine the pieces whose error estimate is too large.
fn quadx_vadapt(
    f: &mut MappedIntegrand,
    m: usize,
    tinterval: Vec<f64>,
    tol: Tolerances,
    cancel: &AtomicBool,
) -> Result<GkResult> {
    let (nodes, wt, ewt) = quadx_nodes();
    let (tinterval, pathlen) = quadx_split(tinterval, MIN_INTERVALS);
    if pathlen == 0. {
        return Ok(GkResult {
            q: vec![ZERO; m],
            errbnd: 0.,
            stop: None,
        });
    }
    let mut subs: Vec<(f64, f64)> = tinterval.windows(2).map(|w| (w[0], w[1])).collect();
    // Reject oversized array-valued problems before even allocating the
    // per-component accumulators. The evaluator charges each batch separately.
    quadx_batch_elements(subs.len().saturating_mul(15), m)?;
    let mut q_ok = vec![ZERO; m];
    let mut err_ok = vec![ZERO; m];
    let mut inner_ok = vec![0.; m];
    let mut q = vec![ZERO; m];
    let mut errbnd = 0.;
    let mut evaluated = 0usize;
    loop {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("Calculation cancelled".into());
        }
        let mid: Vec<f64> = subs.iter().map(|(a, b)| (a + b) / 2.).collect();
        let halfh: Vec<f64> = subs.iter().map(|(a, b)| (b - a) / 2.).collect();
        let mut t = Vec::with_capacity(15 * subs.len());
        for (j, h) in halfh.iter().enumerate() {
            t.extend(nodes.iter().map(|n| n * h + mid[j]));
        }
        evaluated += t.len();
        if evaluated > MAX_POINTS {
            return Err(format!(
                "Integration needs more than {MAX_POINTS} integrand values; loosen the tolerances or split the interval"
            ));
        }
        let Some(fx) = f(&t)? else {
            // Mesh points too close: keep the last complete pass.
            return Ok(GkResult {
                q,
                errbnd,
                stop: Some(Stop::MinStep(f64::NAN)),
            });
        };
        let n = subs.len();
        let mut qsubs = vec![ZERO; n * m];
        let mut errsubs = vec![ZERO; n * m];
        let mut inner_subs = vec![0.; n * m];
        for j in 0..n {
            for c in 0..m {
                let (mut qs, mut es) = (ZERO, ZERO);
                for k in 0..15 {
                    let index = (j * 15 + k) * m + c;
                    let v = fx.values[index];
                    qs += v * wt[k];
                    es += v * ewt[k];
                    if !fx.errors.is_empty() {
                        inner_subs[j * m + c] += fx.errors[index] * wt[k] * halfh[j].abs();
                    }
                }
                qsubs[j * m + c] = qs * halfh[j];
                errsubs[j * m + c] = es * halfh[j];
            }
        }
        for c in 0..m {
            q[c] = (0..n).fold(ZERO, |s, j| s + qsubs[j * m + c]) + q_ok[c];
        }
        let tolerances: Vec<f64> = q.iter().map(|v| tol.abs.max(tol.rel * v.norm())).collect();
        let accurate: Vec<bool> = (0..n)
            .map(|j| {
                (0..m).all(|c| {
                    let e = errsubs[j * m + c].norm() + inner_subs[j * m + c];
                    e <= (2. * tolerances[c] / pathlen) * halfh[j].abs()
                })
            })
            .collect();
        for j in (0..n).filter(|j| accurate[*j]) {
            for c in 0..m {
                err_ok[c] += errsubs[j * m + c];
                inner_ok[c] += inner_subs[j * m + c];
            }
        }
        let errors: Vec<f64> = (0..m)
            .map(|c| {
                err_ok[c].norm()
                    + inner_ok[c]
                    + (0..n)
                        .filter(|j| !accurate[*j])
                        .map(|j| errsubs[j * m + c].norm() + inner_subs[j * m + c])
                        .sum::<f64>()
            })
            .collect();
        errbnd = errors.iter().copied().fold(0., f64::max);
        if !(q.iter().all(|v| quadx_finite(*v)) && errors.iter().all(|e| e.is_finite())) {
            return Ok(GkResult {
                q,
                errbnd,
                stop: Some(Stop::NonFinite),
            });
        }
        if errors.iter().zip(&tolerances).all(|(e, t)| e <= t) {
            break;
        }
        let remaining: Vec<usize> = (0..n).filter(|j| !accurate[*j]).collect();
        if remaining.is_empty() {
            break;
        }
        for j in (0..n).filter(|j| accurate[*j]) {
            for c in 0..m {
                q_ok[c] += qsubs[j * m + c];
            }
        }
        let nsubs = 2 * remaining.len();
        if nsubs > tol.max_intervals {
            return Ok(GkResult {
                q,
                errbnd,
                stop: Some(Stop::MaxIntervals(errbnd, nsubs)),
            });
        }
        subs = remaining
            .iter()
            .flat_map(|&j| [(subs[j].0, mid[j]), (mid[j], subs[j].1)])
            .collect();
    }
    Ok(GkResult {
        q,
        errbnd,
        stop: None,
    })
}

/// quad's recursive Simpson step; warn: 1 min step, 2 max count, 3 non-finite.
struct Simpson<'f, 'a> {
    f: &'f mut Integrand<'a>,
    tol: f64,
    hmin: f64,
    fcnt: usize,
}

impl Simpson<'_, '_> {
    fn quadx_step(&mut self, a: f64, b: f64, fa: C, fc: C, fb: C) -> Result<(C, u8)> {
        let h = b - a;
        let c = (a + b) / 2.;
        if h.abs() < self.hmin || c == a || c == b {
            return Ok((fc * h, 1));
        }
        let y = (self.f)(&[(a + c) / 2., (c + b) / 2.])?;
        self.fcnt += 2;
        if self.fcnt > 10_000 {
            return Ok((fc * h, 2));
        }
        let (fd, fe) = (y[0], y[1]);
        let q1 = (fa + fc * 4. + fb) * (h / 6.);
        let q2 = (fa + fd * 4. + fc * 2. + fe * 4. + fb) * (h / 12.);
        let q = q2 + (q2 - q1) / 15.;
        if !quadx_finite(q) {
            return Ok((q, 3));
        }
        if (q2 - q).norm() <= self.tol {
            return Ok((q, 0));
        }
        let (qac, wac) = self.quadx_step(a, c, fa, fd, fc)?;
        let (qcb, wcb) = self.quadx_step(c, b, fc, fe, fb)?;
        Ok((qac + qcb, wac.max(wcb)))
    }
}

/// quad: returns the integral, the function count and the warning code.
fn quadx_simpson(f: &mut Integrand, a: f64, b: f64, tol: f64) -> Result<(C, usize, u8)> {
    let h = 0.13579 * (b - a);
    let x = [a, a + h, a + 2. * h, (a + b) / 2., b - 2. * h, b - h, b];
    let mut y = f(&x)?;
    let mut fcnt = 7;
    if !quadx_finite(y[0]) {
        y[0] = f(&[a + f64::EPSILON * (b - a)])?[0];
        fcnt += 1;
    }
    if !quadx_finite(y[6]) {
        y[6] = f(&[b - f64::EPSILON * (b - a)])?[0];
        fcnt += 1;
    }
    let mut s = Simpson {
        f,
        tol,
        hmin: f64::EPSILON / 1024. * (b - a).abs(),
        fcnt,
    };
    let (q1, w1) = s.quadx_step(x[0], x[2], y[0], y[1], y[2])?;
    let (q2, w2) = s.quadx_step(x[2], x[4], y[2], y[3], y[4])?;
    let (q3, w3) = s.quadx_step(x[4], x[6], y[4], y[5], y[6])?;
    Ok((q1 + q2 + q3, s.fcnt, w1.max(w2).max(w3)))
}

fn quadx_simpson_warning(code: u8) -> Option<&'static str> {
    match code {
        1 => Some("Warning: Minimum step size reached; singularity possible."),
        2 => Some("Warning: Maximum function count exceeded; singularity likely."),
        3 => Some("Warning: Infinite or Not-a-Number function value encountered."),
        _ => None,
    }
}

/// C's %e with MATLAB's two-digit exponent.
fn quadx_format_e(x: f64, precision: usize) -> String {
    let text = format!("{x:.precision$e}");
    let Some((mantissa, exponent)) = text.split_once('e') else {
        return text;
    };
    let (sign, digits) = match exponent.strip_prefix('-') {
        Some(d) => ('-', d),
        None => ('+', exponent),
    };
    format!("{mantissa}e{sign}{digits:0>2}")
}

/// C's %g (precision 6).
fn quadx_format_g(x: f64) -> String {
    if !x.is_finite() {
        return if x.is_nan() {
            "NaN".into()
        } else if x > 0. {
            "Inf".into()
        } else {
            "-Inf".into()
        };
    }
    if x == 0. {
        return "0".into();
    }
    let exponent = x.abs().log10().floor() as i32;
    let trim = |s: String| {
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            s
        }
    };
    let e = quadx_format_e(x, 5);
    let exponent = e
        .split_once('e')
        .and_then(|(_, p)| p.parse::<i32>().ok())
        .unwrap_or(exponent);
    if (-4..6).contains(&exponent) {
        let decimals = (5 - exponent).max(0) as usize;
        trim(format!("{x:.decimals$}"))
    } else {
        let (mantissa, rest) = e.split_once('e').unwrap_or((&e, ""));
        format!("{}e{rest}", trim(mantissa.to_string()))
    }
}

fn quadx_stop_warning(caller: Caller, stop: &Stop) -> String {
    match (caller, stop) {
        (Caller::Integral, Stop::MinStep(at)) => format!(
            "Warning: Step size limit near x = {}; possible singularity or overly strict tolerances.",
            quadx_format_g(*at)
        ),
        (Caller::Quadgk, Stop::MinStep(_)) => {
            "Warning: Minimum step size reached; singularity possible.".into()
        }
        (Caller::Integral, Stop::MaxIntervals(bound, _)) => format!(
            "Warning: Interval limit reached; estimated error bound:{:>9}. The integral may not exist or may be hard to compute at the requested accuracy.",
            quadx_format_e(*bound, 1)
        ),
        (Caller::Quadgk, Stop::MaxIntervals(bound, count)) => format!(
            "Warning: Interval limit reached; estimated error bound:{:>9}. The integral may not exist or may be hard to compute. Set MaxIntervalCount to {count} for one more QUADGK iteration.",
            quadx_format_e(*bound, 1)
        ),
        (_, Stop::NonFinite) => "Warning: Infinite or Not-a-Number value encountered.".into(),
    }
}

/// A limit of integral2/integral3: a number or a function of the outer variables.
enum Limit {
    Value(f64),
    Function(FunctionHandle),
}

fn quadx_text(value: &Value) -> Option<String> {
    match value {
        Value::Text(s) => Some(s.clone()),
        Value::Container(Array::Character(c)) if c.is_text_row() => {
            c.to_utf8(&AtomicBool::new(false)).ok()
        }
        _ => None,
    }
}

fn quadx_scalar(value: &Value) -> Option<f64> {
    match value {
        Value::Matrix(m) if m.len() == 1 => Some(m.data()[0]),
        _ => None,
    }
}

impl Engine {
    /// Single-output calls; `None` when `name` is not a quadrature function.
    pub(super) fn quadx_call(
        &mut self,
        name: &str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        if !quadx_is_quadrature(name) {
            return Ok(None);
        }
        Ok(Some(
            self.quadx_outputs(name, args.to_vec(), 1, out)?.remove(0),
        ))
    }

    /// All forms; `count` outputs (two only for quadgk and quad).
    pub(super) fn quadx_outputs(
        &mut self,
        name: &str,
        args: Vec<Value>,
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        if count > 2 || (count == 2 && !quadx_has_outputs(name)) {
            return Err("Too many output arguments.".into());
        }
        let minimum = match name {
            "integral" | "quad" | "quadgk" => 3,
            "integral2" | "dblquad" => 5,
            _ => 7,
        };
        if args.len() < minimum {
            return Err("Not enough input arguments.".into());
        }
        let (value, second) = match name {
            "integral" => (self.quadx_integral(&args, out)?, None),
            "quadgk" => {
                let (q, e) = self.quadx_quadgk(&args, out)?;
                (q, Some(Value::Matrix(Matrix::scalar(e))))
            }
            "integral2" | "integral3" => (self.quadx_iterated(name, &args, out)?, None),
            "quad" => {
                let (q, fcnt) = self.quadx_quad(&args, out)?;
                (q, Some(Value::Matrix(Matrix::scalar(fcnt as f64))))
            }
            "dblquad" | "triplequad" => (self.quadx_nested_quad(name, &args, out)?, None),
            _ => unreachable!("quadx_is_quadrature"),
        };
        let mut outputs = vec![value];
        if count == 2 {
            outputs.extend(second);
        }
        self.check_cancel()?;
        Ok(outputs)
    }

    fn quadx_handle(&self, value: &Value) -> Result<FunctionHandle> {
        match value {
            Value::Container(Array::Handle(h)) => Ok(h.clone()),
            _ => Err("First input argument must be a function handle.".into()),
        }
    }

    /// quad, dblquad and triplequad also take a function name.
    fn quadx_handle_or_name(&self, value: &Value) -> Result<FunctionHandle> {
        if let Some(name) = quadx_text(value) {
            return match self.named_handle(name.trim())? {
                Value::Container(Array::Handle(h)) => Ok(h),
                _ => Err("FUN must be a function handle or a function name.".into()),
            };
        }
        self.quadx_handle(value)
            .map_err(|_| "FUN must be a function handle or a function name.".to_string())
    }

    fn quadx_limits(&self, a: &Value, b: &Value) -> Result<(f64, f64)> {
        match (quadx_scalar(a), quadx_scalar(b)) {
            (Some(a), Some(b)) => Ok((a, b)),
            _ if matches!(a, Value::Complex(_)) || matches!(b, Value::Complex(_)) => {
                Err("Complex limits (contour integration) are not supported yet.".into())
            }
            _ => Err("A and B must be floating-point scalars.".into()),
        }
    }

    /// Calls `handle(args...)` and returns its values with the size it returned.
    fn quadx_apply(
        &mut self,
        handle: &FunctionHandle,
        args: Vec<Value>,
        complex: &mut bool,
        out: &mut Execution,
    ) -> Result<(usize, usize, Vec<C>)> {
        self.check_cancel()?;
        let points: usize = args
            .iter()
            .map(|a| match a {
                Value::Matrix(m) => m.len(),
                _ => 1,
            })
            .sum();
        self.charge(points)?;
        let value = self
            .call_handle(handle, args, 1, None, out)?
            .into_iter()
            .next()
            .ok_or("The integrand must return a value.")?;
        match value {
            Value::Matrix(m) => Ok((
                m.rows(),
                m.cols(),
                m.data().iter().map(|v| C::new(*v, 0.)).collect(),
            )),
            Value::Logical(m) => {
                let m = m.to_numeric(&self.cancel)?;
                Ok((
                    m.rows(),
                    m.cols(),
                    m.data().iter().map(|v| C::new(*v, 0.)).collect(),
                ))
            }
            Value::Complex(m) => {
                *complex = true;
                Ok((m.rows(), m.cols(), m.data().to_vec()))
            }
            _ => Err("Supported classes are 'double' and 'single'.".into()),
        }
    }

    /// Evaluates an elementwise integrand at a row of points; `before` and `after`
    /// are the arguments around the row (extra quad parameters, outer variables).
    #[allow(clippy::too_many_arguments)]
    fn quadx_row(
        &mut self,
        handle: &FunctionHandle,
        before: &[f64],
        x: &[f64],
        after: &[Value],
        size_error: &str,
        complex: &mut bool,
        out: &mut Execution,
    ) -> Result<Vec<C>> {
        let n = x.len();
        let mut args = Vec::with_capacity(before.len() + 1 + after.len());
        for v in before {
            args.push(Value::Matrix(Matrix::from_column_vec(1, n, vec![*v; n])?));
        }
        args.push(Value::Matrix(Matrix::from_column_slice(1, n, x)?));
        args.extend(after.iter().cloned());
        let (rows, cols, y) = self.quadx_apply(handle, args, complex, out)?;
        if rows != 1 || cols != n {
            return Err(size_error.into());
        }
        Ok(y)
    }

    fn quadx_emit_stop(
        &mut self,
        caller: Caller,
        stop: &Option<Stop>,
        out: &mut Execution,
    ) -> Result<()> {
        if let Some(stop) = stop {
            self.emit(out, quadx_stop_warning(caller, stop))?;
        }
        Ok(())
    }

    fn quadx_result(&self, rows: usize, cols: usize, q: Vec<C>, complex: bool) -> Result<Value> {
        if complex {
            Ok(Value::Complex(ComplexMatrix::from_column_vec(
                rows,
                cols,
                q,
                &self.cancel,
            )?))
        } else {
            Ok(Value::Matrix(Matrix::from_column_vec(
                rows,
                cols,
                q.into_iter().map(|v| v.re).collect(),
            )?))
        }
    }

    /// Name-value pairs, matched by case-insensitive unique prefix.
    fn quadx_pairs(
        &self,
        args: &[Value],
        names: &[&'static str],
    ) -> Result<Vec<(&'static str, Value)>> {
        let mut pairs = Vec::new();
        let mut i = 0;
        while i < args.len() {
            let Some(text) = quadx_text(&args[i]) else {
                return Err("Expected a string for the parameter name.".into());
            };
            let lower = text.to_ascii_lowercase();
            let matches: Vec<&'static str> = names
                .iter()
                .copied()
                .filter(|n| !lower.is_empty() && n.to_ascii_lowercase().starts_with(&lower))
                .collect();
            let name = match matches.as_slice() {
                [name] => *name,
                [] => {
                    return Err(format!(
                        "'{text}' is not a recognized parameter. For a list of valid name-value pair arguments, see the documentation for this function."
                    ));
                }
                _ => match matches.iter().find(|n| n.eq_ignore_ascii_case(&text)) {
                    Some(name) => *name,
                    None => {
                        return Err(format!(
                            "'{text}' matches multiple parameter names. To avoid ambiguity, specify the complete name of the parameter."
                        ));
                    }
                },
            };
            let Some(value) = args.get(i + 1) else {
                return Err(format!("Expected a value for the parameter '{name}'."));
            };
            pairs.push((name, value.clone()));
            i += 2;
        }
        Ok(pairs)
    }

    /// Tolerances and waypoints shared by integral, quadgk, integral2 and integral3.
    fn quadx_tolerances(
        &self,
        pairs: &[(&'static str, Value)],
    ) -> Result<(Tolerances, Vec<f64>, bool)> {
        let mut tol = Tolerances {
            rel: 1e-6,
            abs: 1e-10,
            max_intervals: 650,
        };
        let mut waypoints = Vec::new();
        let mut array_valued = false;
        for (name, value) in pairs {
            match *name {
                "AbsTol" | "RelTol" => {
                    let v = quadx_scalar(value).filter(|v| *v >= 0.).ok_or_else(|| {
                        format!("Invalid {name} value: expected a non-negative real scalar.")
                    })?;
                    if *name == "AbsTol" {
                        tol.abs = v;
                    } else {
                        tol.rel = v;
                    }
                }
                "MaxIntervalCount" => {
                    let v = quadx_scalar(value)
                        .filter(|v| *v >= 1. && v.fract() == 0.)
                        .ok_or("Invalid MaxIntervalCount value: expected a positive integer.")?;
                    tol.max_intervals = v.min(1e7) as usize;
                }
                "Waypoints" => match value {
                    Value::Matrix(m) if m.rows() <= 1 || m.cols() <= 1 => {
                        if m.data().iter().any(|w| !w.is_finite()) {
                            return Err("Waypoints must be finite.".into());
                        }
                        waypoints = m.data().to_vec();
                    }
                    Value::Complex(_) => {
                        return Err(
                            "Complex waypoints (contour integration) are not supported yet.".into(),
                        );
                    }
                    _ => return Err("Waypoints must be a vector of real numbers.".into()),
                },
                "ArrayValued" => {
                    array_valued = match value {
                        Value::Logical(m) if m.len() == 1 => {
                            m.to_numeric(&self.cancel)?.data()[0] != 0.
                        }
                        Value::Matrix(m) if m.len() == 1 => m.data()[0] != 0.,
                        _ => {
                            return Err("Invalid ArrayValued value: expected true or false.".into());
                        }
                    };
                }
                "Method" => {
                    let method = quadx_text(value).unwrap_or_default().to_ascii_lowercase();
                    if !matches!(method.as_str(), "auto" | "tiled" | "iterated") {
                        return Err("Method must be 'auto', 'tiled' or 'iterated'.".into());
                    }
                }
                _ => {}
            }
        }
        Ok((tol, waypoints, array_valued))
    }

    /// One-dimensional adaptive Gauss-Kronrod of a scalar elementwise integrand.
    #[allow(clippy::too_many_arguments)]
    fn quadx_gk_row(
        &mut self,
        handle: &FunctionHandle,
        a: f64,
        b: f64,
        waypoints: &[f64],
        tol: Tolerances,
        size_error: &str,
        complex: &mut bool,
        out: &mut Execution,
    ) -> Result<GkResult> {
        let cancel = self.cancel.clone();
        let mut f = |x: &[f64]| {
            self.quadx_row(handle, &[], x, &[], size_error, complex, out)
                .map(GkBatch::quadx_exact)
        };
        quadx_gk(&mut f, 1, a, b, waypoints, tol, &cancel)
    }

    /// Validate and charge complex storage before reserving an ArrayValued batch.
    fn quadx_batch_len(&mut self, points: usize, components: usize) -> Result<usize> {
        self.check_cancel()?;
        let len = quadx_batch_elements(points, components)?;
        // Every value occupies two f64 allocation units, even for real integrands.
        let units = len
            .checked_mul(2)
            .ok_or("Integration batch size overflow")?;
        self.charge(units)?;
        Ok(len)
    }

    fn quadx_integral(&mut self, args: &[Value], out: &mut Execution) -> Result<Value> {
        let handle = self.quadx_handle(&args[0])?;
        let (a, b) = self.quadx_limits(&args[1], &args[2])?;
        let pairs = self.quadx_pairs(
            &args[3..],
            &["AbsTol", "ArrayValued", "RelTol", "Waypoints"],
        )?;
        let (tol, waypoints, array_valued) = self.quadx_tolerances(&pairs)?;
        let mut complex = false;
        if !array_valued {
            let result = self.quadx_gk_row(
                &handle,
                a,
                b,
                &waypoints,
                tol,
                "Output of the function must be the same size as the input. If FUN is an array-valued integrand, set the 'ArrayValued' option to true.",
                &mut complex,
                out,
            )?;
            self.quadx_emit_stop(Caller::Integral, &result.stop, out)?;
            return self.quadx_result(1, 1, result.q, complex);
        }
        // The size of the value at the midpoint fixes the size of the result.
        let probe = if a.is_finite() && b.is_finite() {
            (a + b) / 2.
        } else if a.is_finite() {
            a + 1.
        } else if b.is_finite() {
            b - 1.
        } else {
            0.
        };
        let (rows, cols, _) = self.quadx_apply(
            &handle,
            vec![Value::Matrix(Matrix::scalar(probe))],
            &mut complex,
            out,
        )?;
        let m = rows * cols;
        if m == 0 {
            return self.quadx_result(rows, cols, Vec::new(), complex);
        }
        let cancel = self.cancel.clone();
        let result = {
            let mut f = |x: &[f64]| -> Result<GkBatch> {
                let len = self.quadx_batch_len(x.len(), m)?;
                let mut values = Vec::with_capacity(len);
                for &xi in x {
                    let (r, c, y) = self.quadx_apply(
                        &handle,
                        vec![Value::Matrix(Matrix::scalar(xi))],
                        &mut complex,
                        out,
                    )?;
                    if r != rows || c != cols {
                        return Err("The size of the integrand output must not change: an array-valued FUN must return an array of the same size at every point.".into());
                    }
                    values.extend(y);
                }
                Ok(GkBatch::quadx_exact(values))
            };
            quadx_gk(&mut f, m, a, b, &waypoints, tol, &cancel)?
        };
        self.quadx_emit_stop(Caller::Integral, &result.stop, out)?;
        self.quadx_result(rows, cols, result.q, complex)
    }

    fn quadx_quadgk(&mut self, args: &[Value], out: &mut Execution) -> Result<(Value, f64)> {
        let handle = self.quadx_handle(&args[0])?;
        let (a, b) = self.quadx_limits(&args[1], &args[2])?;
        let pairs = self.quadx_pairs(
            &args[3..],
            &["AbsTol", "MaxIntervalCount", "RelTol", "Waypoints"],
        )?;
        let (tol, waypoints, _) = self.quadx_tolerances(&pairs)?;
        let mut complex = false;
        let result = self.quadx_gk_row(
            &handle,
            a,
            b,
            &waypoints,
            tol,
            "Output of the function must be the same size as the input.",
            &mut complex,
            out,
        )?;
        self.quadx_emit_stop(Caller::Quadgk, &result.stop, out)?;
        Ok((self.quadx_result(1, 1, result.q, complex)?, result.errbnd))
    }

    fn quadx_limit(&self, value: &Value, what: &str) -> Result<Limit> {
        if let Value::Container(Array::Handle(h)) = value {
            return Ok(Limit::Function(h.clone()));
        }
        quadx_scalar(value)
            .map(Limit::Value)
            .ok_or_else(|| format!("{what} must be a real scalar or a function handle."))
    }

    /// The value of a limit at the outer variables.
    fn quadx_limit_at(&mut self, limit: &Limit, outer: &[f64], out: &mut Execution) -> Result<f64> {
        match limit {
            Limit::Value(v) => Ok(*v),
            Limit::Function(h) => {
                let args = outer
                    .iter()
                    .map(|v| Value::Matrix(Matrix::scalar(*v)))
                    .collect();
                let mut limit_complex = false;
                let (_, _, y) = self.quadx_apply(h, args, &mut limit_complex, out)?;
                match y.as_slice() {
                    [v] if !limit_complex => Ok(v.re),
                    _ => Err("Limit functions must return real scalars.".into()),
                }
            }
        }
    }

    /// integral2 and integral3 as iterated one-dimensional integrals.
    fn quadx_iterated(&mut self, name: &str, args: &[Value], out: &mut Execution) -> Result<Value> {
        let handle = self.quadx_handle(&args[0])?;
        let dims = if name == "integral2" { 2 } else { 3 };
        let Some(xa) = quadx_scalar(&args[1]) else {
            return Err("XMIN must be a real scalar.".into());
        };
        let Some(xb) = quadx_scalar(&args[2]) else {
            return Err("XMAX must be a real scalar.".into());
        };
        let mut limits = vec![
            self.quadx_limit(&args[3], "YMIN")?,
            self.quadx_limit(&args[4], "YMAX")?,
        ];
        if dims == 3 {
            limits.push(self.quadx_limit(&args[5], "ZMIN")?);
            limits.push(self.quadx_limit(&args[6], "ZMAX")?);
        }
        let pairs = self.quadx_pairs(&args[1 + 2 * dims..], &["AbsTol", "Method", "RelTol"])?;
        let (mut tol, _, _) = self.quadx_tolerances(&pairs)?;
        // Reserve headroom for nested estimates and quadrature roundoff.
        tol.rel /= 10.;
        tol.abs /= 10.;
        let mut complex = false;
        let mut stops = Vec::new();
        let (q, _) = self.quadx_level(
            &handle,
            &limits,
            &[],
            xa,
            xb,
            tol,
            &mut complex,
            &mut stops,
            out,
        )?;
        if let Some(stop) = stops.into_iter().next() {
            self.quadx_emit_stop(Caller::Integral, &Some(stop), out)?;
        }
        self.quadx_result(1, 1, vec![q], complex)
    }

    /// Integrates over the variable after `outer` from a to b; deeper variables
    /// take their limits from `limits` (pairs of lower and upper).
    #[allow(clippy::too_many_arguments)]
    fn quadx_level(
        &mut self,
        handle: &FunctionHandle,
        limits: &[Limit],
        outer: &[f64],
        a: f64,
        b: f64,
        tol: Tolerances,
        complex: &mut bool,
        stops: &mut Vec<Stop>,
        out: &mut Execution,
    ) -> Result<(C, f64)> {
        let cancel = self.cancel.clone();
        let result = if limits.is_empty() {
            let mut f = |x: &[f64]| {
                self.quadx_row(
                    handle,
                    outer,
                    x,
                    &[],
                    "Integrand output size does not match the input size.",
                    complex,
                    out,
                )
                .map(GkBatch::quadx_exact)
            };
            quadx_gk(&mut f, 1, a, b, &[], tol, &cancel)?
        } else {
            let mut f = |x: &[f64]| -> Result<GkBatch> {
                let mut values = Vec::with_capacity(x.len());
                let mut errors = Vec::with_capacity(x.len());
                let mut point = outer.to_vec();
                point.push(0.);
                for &xi in x {
                    *point.last_mut().expect("point") = xi;
                    let lo = self.quadx_limit_at(&limits[0], &point, out)?;
                    let hi = self.quadx_limit_at(&limits[1], &point, out)?;
                    // A finite outer interval integrates a uniform inner error
                    // over |b-a|. On infinite intervals use an integrable error
                    // density of total mass one instead of an infinite measure.
                    let density = if a.is_finite() && b.is_finite() {
                        1. / (b - a).abs()
                    } else if a.is_finite() || b.is_finite() {
                        let endpoint = if a.is_finite() { a } else { b };
                        (1. + (xi - endpoint).abs()).recip().powi(2)
                    } else {
                        0.5 * (1. + xi.abs()).recip().powi(2)
                    };
                    let inner_tol = Tolerances {
                        abs: tol.abs * density / 10.,
                        rel: tol.rel / 10.,
                        ..tol
                    };
                    let (q, error) = self.quadx_level(
                        handle,
                        &limits[2..],
                        &point,
                        lo,
                        hi,
                        inner_tol,
                        complex,
                        stops,
                        out,
                    )?;
                    values.push(q);
                    errors.push(error);
                }
                Ok(GkBatch { values, errors })
            };
            quadx_gk(&mut f, 1, a, b, &[], tol, &cancel)?
        };
        if let Some(stop) = result.stop {
            stops.push(stop);
        }
        Ok((result.q[0], result.errbnd))
    }

    fn quadx_quad_tol(&self, value: Option<&Value>) -> Result<f64> {
        match value {
            None => Ok(1e-6),
            Some(Value::Matrix(m)) if m.is_empty() => Ok(1e-6),
            Some(v) => quadx_scalar(v)
                .filter(|t| *t >= 0.)
                .ok_or_else(|| "The tolerance must be a non-negative real scalar.".into()),
        }
    }

    fn quadx_trace(&self, value: Option<&Value>) -> Result<()> {
        match value {
            None => Ok(()),
            Some(Value::Matrix(m)) if m.is_empty() || m.data().iter().all(|v| *v == 0.) => Ok(()),
            Some(_) => Err("quad: the trace argument is not supported yet; pass [] or 0.".into()),
        }
    }

    fn quadx_quad(&mut self, args: &[Value], out: &mut Execution) -> Result<(Value, usize)> {
        let handle = self.quadx_handle_or_name(&args[0])?;
        let (Some(a), Some(b)) = (quadx_scalar(&args[1]), quadx_scalar(&args[2])) else {
            return Err("The limits of integration must be scalars.".into());
        };
        let tol = self.quadx_quad_tol(args.get(3))?;
        self.quadx_trace(args.get(4))?;
        let extra = args.get(5..).unwrap_or(&[]);
        let mut complex = false;
        let mut warnings = Vec::new();
        let (q, fcnt) = self.quadx_quad_core(
            &handle,
            a,
            b,
            tol,
            &[],
            extra,
            &mut complex,
            &mut warnings,
            out,
        )?;
        for warning in warnings {
            self.emit(out, warning.into())?;
        }
        Ok((self.quadx_result(1, 1, vec![q], complex)?, fcnt))
    }

    /// quad of handle(x, extra...) over [a, b], with `outer` scalars passed after x.
    #[allow(clippy::too_many_arguments)]
    fn quadx_quad_core(
        &mut self,
        handle: &FunctionHandle,
        a: f64,
        b: f64,
        tol: f64,
        outer: &[f64],
        extra: &[Value],
        complex: &mut bool,
        warnings: &mut Vec<&'static str>,
        out: &mut Execution,
    ) -> Result<(C, usize)> {
        let mut after: Vec<Value> = outer
            .iter()
            .map(|v| Value::Matrix(Matrix::scalar(*v)))
            .collect();
        after.extend(extra.iter().cloned());
        let mut f = |x: &[f64]| -> Result<Vec<C>> {
            let mut args = vec![Value::Matrix(Matrix::from_column_slice(1, x.len(), x)?)];
            args.extend(after.iter().cloned());
            let (_, _, y) = self.quadx_apply(handle, args, complex, out)?;
            if y.len() < x.len() {
                return Err("Index exceeds matrix dimensions.".into());
            }
            Ok(y)
        };
        let (q, fcnt, warn) = quadx_simpson(&mut f, a, b, tol)?;
        if let Some(text) = quadx_simpson_warning(warn) {
            warnings.push(text);
        }
        Ok((q, fcnt))
    }

    /// dblquad and triplequad: quad over the outer variables of inner quad calls.
    fn quadx_nested_quad(
        &mut self,
        name: &str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Value> {
        let handle = self.quadx_handle_or_name(&args[0])?;
        let dims = if name == "dblquad" { 2 } else { 3 };
        let mut bounds = Vec::new();
        for k in 0..dims {
            let (Some(lo), Some(hi)) = (
                quadx_scalar(&args[1 + 2 * k]),
                quadx_scalar(&args[2 + 2 * k]),
            ) else {
                return Err("The limits of integration must be scalars.".into());
            };
            bounds.push((lo, hi));
        }
        let tol = self.quadx_quad_tol(args.get(1 + 2 * dims))?;
        if let Some(method) = args.get(2 + 2 * dims) {
            let plain = match method {
                Value::Matrix(m) => m.is_empty(),
                Value::Container(Array::Handle(h)) => h.func2str() == "quad",
                other => quadx_text(other).as_deref() == Some("quad"),
            };
            if !plain {
                return Err(format!(
                    "{name}: only the default method @quad is supported yet."
                ));
            }
        }
        let extra = args.get(3 + 2 * dims..).unwrap_or(&[]).to_vec();
        let mut complex = false;
        let mut warnings = Vec::new();
        let q = self.quadx_nested_level(
            &handle,
            &bounds,
            &[],
            tol,
            &extra,
            &mut complex,
            &mut warnings,
            out,
        )?;
        for warning in warnings {
            self.emit(out, warning.into())?;
        }
        self.quadx_result(1, 1, vec![q], complex)
    }

    /// Integrates over the last variable of `bounds`; inner variables come first
    /// in the call, as dblquad's fun(x, y) with x a row and y a scalar.
    #[allow(clippy::too_many_arguments)]
    fn quadx_nested_level(
        &mut self,
        handle: &FunctionHandle,
        bounds: &[(f64, f64)],
        outer: &[f64],
        tol: f64,
        extra: &[Value],
        complex: &mut bool,
        warnings: &mut Vec<&'static str>,
        out: &mut Execution,
    ) -> Result<C> {
        let (last, inner) = bounds.split_last().expect("bounds");
        if inner.is_empty() {
            return Ok(self
                .quadx_quad_core(
                    handle, last.0, last.1, tol, outer, extra, complex, warnings, out,
                )?
                .0);
        }
        let mut f = |y: &[f64]| -> Result<Vec<C>> {
            let mut values = Vec::with_capacity(y.len());
            for &yi in y {
                let mut point = vec![yi];
                point.extend_from_slice(outer);
                values.push(self.quadx_nested_level(
                    handle, inner, &point, tol, extra, complex, warnings, out,
                )?);
            }
            Ok(values)
        };
        let (q, _, warn) = quadx_simpson(&mut f, last.0, last.1, tol)?;
        if let Some(text) = quadx_simpson_warning(warn) {
            warnings.push(text);
        }
        Ok(q)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kronrod_weights_integrate_polynomials_exactly() {
        let (nodes, wt, ewt) = quadx_nodes();
        let total: f64 = wt.iter().sum();
        assert!((total - 2.).abs() < 1e-14);
        // The Gauss 7-point rule is exact for degree 13: the error weights vanish on x^12.
        let e: f64 = (0..15).map(|k| ewt[k] * nodes[k].powi(12)).sum();
        assert!(e.abs() < 1e-14, "{e}");
    }

    #[test]
    fn split_makes_ten_pieces() {
        let (x, len) = quadx_split(vec![-1., 1.], 10);
        assert_eq!(x.len(), 11);
        assert_eq!(len, 2.);
        assert!((x[1] + 0.8).abs() < 1e-15);
    }

    #[test]
    fn printf_formats() {
        assert_eq!(quadx_format_e(1.234e-5, 1), "1.2e-05");
        assert_eq!(quadx_format_g(0.5), "0.5");
        assert_eq!(quadx_format_g(1e-7), "1e-07");
        assert_eq!(quadx_format_g(123456.), "123456");
        assert_eq!(quadx_format_g(1234567.), "1.23457e+06");
    }

    #[test]
    fn quadx_batch_checks_product_and_charges_complex_units() {
        let mut engine = Engine::default();
        assert!(engine.quadx_batch_len(usize::MAX, 2).is_err());
        assert!(engine.quadx_batch_len(150, MAX_ELEMENTS).is_err());
        assert_eq!(engine.evaluated_elements, 0);
        assert_eq!(engine.quadx_batch_len(150, 2).unwrap(), 300);
        assert_eq!(engine.evaluated_elements, 600);
        engine.evaluated_elements = 32 * 1024 * 1024 - 599;
        let error = engine.quadx_batch_len(150, 2).unwrap_err();
        assert!(error.contains("256 MiB evaluation budget"), "{error}");
    }

    #[test]
    fn quadx_inner_errors_contribute_to_outer_convergence() {
        // Exact constant values, but uncertain inner integrals. The outer rule's
        // own error is essentially zero; it must still refuse convergence.
        for (a, b) in [(0., 100.), (100., 0.)] {
            let mut f = |x: &[f64]| {
                Ok(GkBatch {
                    values: vec![C::new(1., 0.); x.len()],
                    errors: vec![0.01; x.len()],
                })
            };
            let result = quadx_gk(
                &mut f,
                1,
                a,
                b,
                &[],
                Tolerances {
                    abs: 0.1,
                    rel: 0.,
                    max_intervals: 10,
                },
                &AtomicBool::new(false),
            )
            .unwrap();
            assert!((result.q[0].re - (b - a)).abs() < 1e-12);
            assert!((result.errbnd - 1.).abs() < 1e-12);
            assert!(matches!(result.stop, Some(Stop::MaxIntervals(_, _))));
        }
    }
}
