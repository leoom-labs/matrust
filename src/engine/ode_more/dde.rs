//! Delay equations with continuous Runge-Kutta approximations. Shampine &
//! Thompson, "Solving DDEs in MATLAB" (2001), sections 2, 4-7: BS(2,3),
//! Hermite history, propagated discontinuities and iteration for short delays.
//! Shampine, "Solving ODEs and DDEs with residual control" (2005), sections
//! 2.1 and 3: RK4
//! with control of the continuous solution's defect for general delays.
use super::bvp::hermite;
use super::ivp::format_e;
use super::num::{R, eps_of, norm_inf, odezero, validate_events};
use crate::engine::Fma;
use std::collections::BTreeSet;

const MAX_VALUES: usize = 32 * 1024 * 1024;
const MAX_BREAKS: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy)]
struct Break(f64);

impl PartialEq for Break {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}
impl Eq for Break {}
impl PartialOrd for Break {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Break {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

fn insert_break(points: &mut BTreeSet<Break>, point: f64) -> R<bool> {
    let key = Break(point);
    let coincides = |&Break(t): &Break| (t - point).abs() <= 10. * eps_of(t.abs().max(point.abs()));
    if points.range(..=key).next_back().is_some_and(coincides)
        || points.range(key..).next().is_some_and(coincides)
    {
        return Ok(false);
    }
    if points.len() >= MAX_BREAKS {
        return Err("DDE discontinuities exceed the generation limit.".into());
    }
    Ok(points.insert(key))
}

pub trait DdeModel {
    /// f(t, y, Z) with Z neq-by-ndelays, column-major.
    fn f(&mut self, t: f64, y: &[f64], z: &[f64]) -> R<Vec<f64>>;
    /// The history function at t (only called for a function history).
    fn history(&mut self, t: f64) -> R<Vec<f64>>;
    /// Delayed times d(t, y) (ddesd with a delay function).
    fn delays(&mut self, t: f64, y: &[f64]) -> R<Vec<f64>>;
    fn events(&mut self, t: f64, y: &[f64], z: &[f64]) -> R<(Vec<f64>, Vec<f64>, Vec<f64>)>;
    fn initial_events(&mut self, _t: f64, _y: &[f64], _z: &[f64]) -> R<()> {
        Ok(())
    }
    fn check_cancel(&self) -> R<()>;
}

pub enum History {
    Constant(Vec<f64>),
    Function,
    /// A previous solution: x, y, yp.
    Solution(Vec<f64>, Vec<Vec<f64>>, Vec<Vec<f64>>),
}

pub enum Delays {
    /// Constant lags (dde23, or ddesd with numeric delays).
    Lags(Vec<f64>),
    /// ddesd delay function d(t, y).
    Function(usize),
}

pub struct DdeOpts {
    pub rtol: f64,
    pub threshold: Vec<f64>,
    pub hmax: f64,
    pub htry: Option<f64>,
    pub jumps: Vec<f64>,
    pub initial_y: Option<Vec<f64>>,
    pub events: bool,
}

pub struct DdeOut {
    pub x: Vec<f64>,
    pub y: Vec<Vec<f64>>,
    pub yp: Vec<Vec<f64>>,
    pub discont: Vec<f64>,
    pub te: Vec<f64>,
    pub ye: Vec<Vec<f64>>,
    pub ie: Vec<usize>,
    pub stats: [usize; 3],
    pub warnings: Vec<String>,
}

struct Past<'a> {
    history: &'a History,
    x: &'a [f64],
    y: &'a [Vec<f64>],
    yp: &'a [Vec<f64>],
    trial: Option<(f64, &'a [f64], &'a [f64])>,
}

impl Past<'_> {
    fn value(&self, m: &mut dyn DdeModel, s: f64) -> R<Vec<f64>> {
        if s < self.x[0] {
            return match self.history {
                History::Constant(v) => Ok(v.clone()),
                History::Function => m.history(s),
                History::Solution(x, y, yp) if s >= x[0] => Ok(interp(x, y, yp, s)),
                // The adapter resolves the saved history, including earlier restarts.
                History::Solution(..) => m.history(s),
            };
        }
        let last = self.x.len() - 1;
        if s > self.x[last]
            && let Some((end, y, yp)) = self.trial
        {
            return Ok(hermite(s, self.x[last], &self.y[last], &self.yp[last], end, y, yp).0);
        }
        Ok(interp(self.x, self.y, self.yp, s))
    }
}

pub fn interp(x: &[f64], y: &[Vec<f64>], yp: &[Vec<f64>], s: f64) -> Vec<f64> {
    // Exact knots also preserve InitialY on the right side of the initial jump.
    let upper = x.partition_point(|&t| t <= s);
    if upper > 0 && x[upper - 1] == s {
        return y[upper - 1].clone();
    }
    if x.len() == 1 {
        return y[0].clone();
    }
    let b = upper.clamp(1, x.len() - 1);
    hermite(s, x[b - 1], &y[b - 1], &yp[b - 1], x[b], &y[b], &yp[b]).0
}

fn discontinuities(
    m: &dyn DdeModel,
    t0: f64,
    tfinal: f64,
    lags: &[f64],
    jumps: &[f64],
    from_t0: bool,
) -> R<Vec<f64>> {
    // Deduplicate each candidate before storing or propagating it. Section 5 of
    // Shampine & Thompson (2001) accounts for roundoff in coincident sums.
    let mut unique_lags = BTreeSet::new();
    for &lag in lags {
        m.check_cancel()?;
        if !lag.is_finite() || lag <= 0. {
            return Err("The lags must all be positive.".into());
        }
        insert_break(&mut unique_lags, lag)?;
    }
    let unique_lags: Vec<_> = unique_lags.into_iter().map(|Break(t)| t).collect();
    let mut all = BTreeSet::new();
    for &jump in jumps {
        m.check_cancel()?;
        if !jump.is_finite() {
            return Err("DDE jumps must be finite.".into());
        }
        if jump <= tfinal {
            insert_break(&mut all, jump)?;
        }
    }
    if from_t0 {
        insert_break(&mut all, t0)?;
    }
    let mut level: Vec<_> = all.iter().map(|Break(t)| *t).collect();
    for _ in 0..3 {
        let mut next = Vec::new();
        for &base in &level {
            for &lag in &unique_lags {
                m.check_cancel()?;
                let point = base + lag;
                if point > tfinal {
                    break;
                }
                if point > base && insert_break(&mut all, point)? {
                    next.push(point);
                }
            }
        }
        next.sort_by(f64::total_cmp);
        level = next;
    }
    Ok(all
        .into_iter()
        .map(|Break(t)| t)
        .filter(|&t| t >= t0 && t <= tfinal)
        .collect())
}

#[derive(Clone, Copy, PartialEq)]
pub enum Scheme {
    /// dde23: Bogacki-Shampine 2(3).
    Bs23,
    /// ddesd: classical Runge-Kutta 4 with residual control.
    Rk4,
}

/// Mutable user callbacks remain serial. Each delay column is filled separately;
/// neither a stage nor a correction copies the accepted history mesh.
struct Evaluator<'a> {
    model: &'a mut dyn DdeModel,
    delays: &'a Delays,
    calls: usize,
}

impl Evaluator<'_> {
    fn delayed(&mut self, past: &Past<'_>, t: f64, y: &[f64]) -> R<(Vec<f64>, bool)> {
        self.model.check_cancel()?;
        let times = match self.delays {
            Delays::Lags(lags) => lags.iter().map(|&lag| t - lag).collect(),
            Delays::Function(count) => {
                let times = self.model.delays(t, y)?;
                if times.len() != *count {
                    return Err(
                        "The delay function must return a vector of constant length.".into(),
                    );
                }
                times
            }
        };
        let size = y.len().saturating_mul(times.len());
        if size > MAX_VALUES {
            return Err("DDE delayed states exceed the 33554432-element limit.".into());
        }
        let mut z = Vec::with_capacity(size);
        let mut overlaps = false;
        for s in times {
            self.model.check_cancel()?;
            if !s.is_finite() || s > t {
                return Err(
                    "The delay function must return finite delayed times no greater than t.".into(),
                );
            }
            if s == t {
                // A vanishing delay is the current Runge-Kutta stage, not an
                // extrapolation of a previous stage or of the initial history.
                z.extend_from_slice(y);
                continue;
            }
            overlaps |= s > *past.x.last().unwrap();
            let value = past.value(self.model, s)?;
            if value.len() != y.len() || value.iter().any(|v| !v.is_finite()) {
                return Err(
                    "The history must return a finite vector matching the initial conditions."
                        .into(),
                );
            }
            z.extend(value);
        }
        Ok((z, overlaps))
    }

    fn rhs(&mut self, past: &Past<'_>, t: f64, y: &[f64], overlap: &mut bool) -> R<Vec<f64>> {
        let (z, uses_trial) = self.delayed(past, t, y)?;
        *overlap |= uses_trial;
        self.calls += 1;
        let f = self.model.f(t, y, &z)?;
        if f.len() != y.len() {
            return Err(
                "DDE derivative must have the same number of elements as the initial conditions."
                    .into(),
            );
        }
        Ok(f)
    }
}

fn combine(y: &[f64], h: f64, terms: &[(f64, &[f64])]) -> Vec<f64> {
    y.iter()
        .enumerate()
        .map(|(i, &v)| {
            let slope = terms.iter().fold(0., |sum, (a, f)| a.fma(f[i], sum));
            h.fma(slope, v)
        })
        .collect()
}

fn scaled_norm(values: impl Iterator<Item = f64>, a: &[f64], b: &[f64], o: &DdeOpts) -> f64 {
    norm_inf(values.enumerate().map(|(i, e)| {
        let scale = a[i].abs().max(b[i].abs()).max(o.threshold[i]);
        if scale == 0. && e == 0. {
            0.
        } else {
            e / scale
        }
    }))
}

struct Attempt {
    y: Vec<f64>,
    yp: Vec<f64>,
    error: f64,
}

/// The tableau coefficients are BS(2,3) and classical RK4. Iteration evaluates
/// the same tableau against successive Hermite extensions of the current step;
/// see Shampine & Thompson (2001), section 2 and section 7.1.
fn attempt(
    eval: &mut Evaluator<'_>,
    scheme: Scheme,
    past: &Past<'_>,
    end: f64,
    o: &DdeOpts,
) -> R<Attempt> {
    let last = past.x.len() - 1;
    let t = past.x[last];
    let y = &past.y[last];
    let f = &past.yp[last];
    let h = end - t;
    let mut guess = interp(past.x, past.y, past.yp, end);
    let mut slope = f.clone();
    for _ in 0..5 {
        eval.model.check_cancel()?;
        let trial = Past {
            trial: Some((end, &guess, &slope)),
            ..*past
        };
        let mut overlap = false;
        let mid = t + 0.5 * h;
        let y2 = combine(y, h, &[(0.5, f)]);
        let k2 = eval.rhs(&trial, mid, &y2, &mut overlap)?;
        let (next, k3) = match scheme {
            Scheme::Bs23 => {
                let y3 = combine(y, h, &[(0.75, &k2)]);
                let k3 = eval.rhs(&trial, t + 0.75 * h, &y3, &mut overlap)?;
                (
                    combine(y, h, &[(2. / 9., f), (1. / 3., &k2), (4. / 9., &k3)]),
                    k3,
                )
            }
            Scheme::Rk4 => {
                let y3 = combine(y, h, &[(0.5, &k2)]);
                let k3 = eval.rhs(&trial, mid, &y3, &mut overlap)?;
                let y4 = combine(y, h, &[(1., &k3)]);
                let k4 = eval.rhs(&trial, end, &y4, &mut overlap)?;
                (
                    combine(
                        y,
                        h,
                        &[(1. / 6., f), (1. / 3., &k2), (1. / 3., &k3), (1. / 6., &k4)],
                    ),
                    k3,
                )
            }
        };
        let next_f = eval.rhs(&trial, end, &next, &mut overlap)?;
        let change =
            scaled_norm(next.iter().zip(&guess).map(|(a, b)| a - b), y, &next, o).max(scaled_norm(
                next_f.iter().zip(&slope).map(|(a, b)| h * (a - b)),
                y,
                &next,
                o,
            ));
        if !overlap || change <= 0.1 * o.rtol {
            let error = match scheme {
                Scheme::Bs23 => {
                    // Difference between order-three weights (2/9,1/3,4/9,0)
                    // and order-two weights (7/24,1/4,1/3,1/8).
                    scaled_norm(
                        (0..y.len()).map(|i| {
                            h * ((-5. / 72.).fma(
                                f[i],
                                (1. / 12.).fma(k2[i], (1. / 9.).fma(k3[i], -next_f[i] / 8.)),
                            ))
                        }),
                        y,
                        &next,
                        o,
                    ) / o.rtol
                }
                Scheme::Rk4 => {
                    let extension = Past {
                        trial: Some((end, &next, &next_f)),
                        ..*past
                    };
                    let mut defect = 0f64;
                    // A cubic Hermite interpolation remainder is proportional
                    // to s^2(1-s)^2. Its derivative has extrema at these two
                    // Gauss points. The midpoint is an additional nonsmoothness
                    // check; the endpoint checks the remaining iteration defect.
                    // Shampine (2005), sections 2.1 and 3, bounds the residual
                    // by C5 times the larger Gauss sample, C5 approximately
                    // 2.1342, and controls h times this bound. Round C5 upward.
                    let offset = libm::sqrt(3.) / 6.;
                    for s in [0.5 - offset, 0.5, 0.5 + offset, 1.] {
                        let at = t + s * h;
                        let (p, dp) = hermite(at, t, y, f, end, &next, &next_f);
                        let fp = eval.rhs(&extension, at, &p, &mut overlap)?;
                        let residual =
                            scaled_norm(dp.iter().zip(&fp).map(|(a, b)| a - b), y, &next, o);
                        if !residual.is_finite() {
                            defect = f64::INFINITY;
                            break;
                        }
                        defect = defect.max(residual);
                    }
                    2.14 * h * defect / o.rtol
                }
            };
            return Ok(Attempt {
                y: next,
                yp: next_f,
                error,
            });
        }
        guess = next;
        slope = next_f;
    }
    // Failure of simple iteration rejects the entire step. No trial values
    // become part of the accepted history.
    Ok(Attempt {
        y: guess,
        yp: slope,
        error: f64::INFINITY,
    })
}

fn check_output_size(points: usize, events: usize, neq: usize) -> R<()> {
    let elements = points
        .saturating_mul(neq.saturating_mul(2).saturating_add(1))
        .saturating_add(events.saturating_mul(neq.saturating_add(2)));
    if elements > MAX_VALUES {
        Err("DDE solution exceeds the 33554432-element limit; shorten tspan.".into())
    } else {
        Ok(())
    }
}

pub fn solve(
    m: &mut dyn DdeModel,
    scheme: Scheme,
    delays: &Delays,
    history: &History,
    t0: f64,
    tfinal: f64,
    o: &DdeOpts,
) -> R<DdeOut> {
    m.check_cancel()?;
    if !t0.is_finite() || !tfinal.is_finite() || tfinal <= t0 || !(tfinal - t0).is_finite() {
        return Err("Must have tspan(1) < tspan(end).".into());
    }
    let y0 = match &o.initial_y {
        Some(y) => y.clone(),
        None => match history {
            History::Constant(y) => y.clone(),
            History::Function => m.history(t0)?,
            History::Solution(x, y, yp) => interp(x, y, yp, t0),
        },
    };
    let neq = y0.len();
    if neq == 0 || y0.iter().any(|y| !y.is_finite()) {
        return Err("DDE initial conditions must be nonempty and finite.".into());
    }
    check_output_size(1, 0, neq)?;
    let ndel = match delays {
        Delays::Lags(lags) => lags.len(),
        Delays::Function(count) => *count,
    };
    // Bound stage vectors and delayed-state workspace before allocating them.
    if neq.saturating_mul(ndel.saturating_add(16)) > MAX_VALUES {
        return Err("DDE workspace exceeds the 33554432-element limit.".into());
    }
    if o.threshold.len() != neq
        || o.threshold.iter().any(|x| !x.is_finite() || *x < 0.)
        || !o.rtol.is_finite()
        || o.rtol <= 0.
        || o.hmax <= 0.
    {
        return Err(
            "DDE tolerances and step bounds must be positive and match the initial conditions."
                .into(),
        );
    }
    let from_t0 = scheme == Scheme::Bs23
        && match history {
            History::Solution(x, y, yp) => o.initial_y.is_some() && y0 != interp(x, y, yp, t0),
            _ => true,
        };
    let mut breaks = match delays {
        Delays::Lags(lags) => discontinuities(m, t0, tfinal, lags, &o.jumps, from_t0)?,
        Delays::Function(_) => discontinuities(m, t0, tfinal, &[], &o.jumps, false)?,
    };
    let discont = breaks.clone();
    breaks.retain(|&point| point > t0 && point < tfinal);
    breaks.push(tfinal);
    let mut next_break = 0;
    let mut out = DdeOut {
        x: vec![t0],
        y: vec![y0],
        yp: vec![vec![0.; neq]],
        discont,
        te: Vec::new(),
        ye: Vec::new(),
        ie: Vec::new(),
        stats: [0; 3],
        warnings: Vec::new(),
    };
    let mut eval = Evaluator {
        model: m,
        delays,
        calls: 0,
    };
    let past = Past {
        history,
        x: &out.x,
        y: &out.y,
        yp: &out.yp,
        trial: None,
    };
    let (z0, _) = eval.delayed(&past, t0, &out.y[0])?;
    let f0 = eval.model.f(t0, &out.y[0], &z0)?;
    eval.calls += 1;
    if f0.len() != neq || f0.iter().any(|v| !v.is_finite()) {
        return Err("DDE derivative must be finite and match the initial conditions.".into());
    }
    out.yp[0] = f0;
    let mut event_values = if o.events {
        eval.model.initial_events(t0, &out.y[0], &z0)?;
        let (v, term, dir) = eval.model.events(t0, &out.y[0], &z0)?;
        validate_events(&v, &term, &dir, v.len())?;
        v
    } else {
        Vec::new()
    };
    let hmax = o.hmax.min(tfinal - t0);
    let rate = scaled_norm(out.yp[0].iter().copied(), &out.y[0], &out.y[0], o);
    let initial = if rate > 0. {
        0.5 * libm::cbrt(o.rtol) / rate
    } else {
        hmax
    };
    let mut h = o.htry.unwrap_or(initial).min(hmax).max(16. * eps_of(t0));
    let min_lag = match delays {
        Delays::Lags(lags) => lags.iter().copied().fold(f64::INFINITY, f64::min),
        Delays::Function(_) => f64::INFINITY,
    };
    let power = match scheme {
        Scheme::Bs23 => 1. / 3.,
        Scheme::Rk4 => 1. / 4.,
    };
    while *out.x.last().unwrap() < tfinal {
        eval.model.check_cancel()?;
        let last = out.x.len() - 1;
        let t = out.x[last];
        let hmin = 16. * eps_of(t);
        h = h.min(hmax);
        // For these short explicit steps an iteration costs more than taking
        // two steps; Shampine & Thompson (2001), section 2.
        if h > min_lag && h < 2. * min_lag {
            h = min_lag;
        }
        // Stretch to a nearby discontinuity (or final time), avoiding a tiny
        // leftover interval. Shampine & Thompson (2001), section 5.
        let end = if breaks[next_break] - t <= 1.1 * h {
            breaks[next_break]
        } else {
            t + h
        };
        let used = end - t;
        if end <= t {
            out.warnings.push(format!(
                "Warning: Integration stopped at t={}; meeting the tolerances would require a step below the minimum ({}).",
                format_e(t), format_e(hmin)));
            break;
        }
        let past = Past {
            history,
            x: &out.x,
            y: &out.y,
            yp: &out.yp,
            trial: None,
        };
        let step = attempt(&mut eval, scheme, &past, end, o)?;
        let valid = step.error.is_finite()
            && step.error <= 1.
            && step.y.iter().chain(&step.yp).all(|v| v.is_finite());
        if !valid {
            out.stats[1] += 1;
            if used <= hmin || h <= hmin {
                out.warnings.push(format!(
                    "Warning: Integration stopped at t={}; meeting the tolerances would require a step below the minimum ({}).",
                    format_e(t), format_e(hmin)));
                break;
            }
            let factor = if step.error.is_finite() {
                (0.8 / libm::pow(step.error, power)).clamp(0.1, 0.5)
            } else {
                0.5
            };
            h = (used * factor).max(hmin);
            continue;
        }
        let mut accepted_t = end;
        let mut accepted_y = step.y;
        let mut accepted_f = step.yp;
        let mut stop = false;
        if o.events {
            let tail = Past {
                trial: Some((end, &accepted_y, &accepted_f)),
                ..past
            };
            let interpolate = |s| {
                hermite(
                    s,
                    t,
                    &out.y[last],
                    &out.yp[last],
                    end,
                    &accepted_y,
                    &accepted_f,
                )
            };
            let (found, values) = odezero(
                &mut |s| interpolate(s).0,
                &mut |s, y| {
                    let (z, _) = eval.delayed(&tail, s, y)?;
                    eval.model.events(s, y, &z)
                },
                t0,
                &event_values,
                t,
                end,
                &accepted_y,
            )?;
            check_output_size(out.x.len() + 1, out.te.len() + found.t.len(), neq)?;
            stop = found.stop;
            let truncated = if stop {
                accepted_t = *found.t.last().unwrap();
                Some(interpolate(accepted_t))
            } else {
                None
            };
            out.te.extend(found.t);
            out.ye.extend(found.y);
            out.ie.extend(found.i);
            event_values = values;
            if let Some((y, yp)) = truncated {
                // Restricting a cubic using its value AND derivative preserves
                // the whole polynomial on the retained terminal interval.
                accepted_y = y;
                accepted_f = yp;
            }
        }
        check_output_size(out.x.len() + 1, out.te.len(), neq)?;
        out.x.push(accepted_t);
        out.y.push(accepted_y);
        out.yp.push(accepted_f);
        out.stats[0] += 1;
        if stop {
            break;
        }
        if end == breaks[next_break] {
            next_break += 1;
        }
        let factor = if step.error == 0. {
            5.
        } else {
            (0.8 / libm::pow(step.error, power)).clamp(0.2, 5.)
        };
        h = (used * factor).max(hmin);
    }
    out.stats[2] = eval.calls;
    out.discont.retain(|&point| point <= *out.x.last().unwrap());
    Ok(out)
}

#[cfg(test)]
mod regression_tests {
    use super::*;
    use std::cell::Cell;

    struct CancelAfter(Cell<usize>);
    impl DdeModel for CancelAfter {
        fn f(&mut self, _: f64, _: &[f64], _: &[f64]) -> R<Vec<f64>> {
            Ok(vec![0.])
        }
        fn history(&mut self, _: f64) -> R<Vec<f64>> {
            Ok(vec![0.])
        }
        fn delays(&mut self, _: f64, _: &[f64]) -> R<Vec<f64>> {
            Ok(vec![0.])
        }
        fn events(&mut self, _: f64, _: &[f64], _: &[f64]) -> R<(Vec<f64>, Vec<f64>, Vec<f64>)> {
            Ok((vec![], vec![], vec![]))
        }
        fn check_cancel(&self) -> R<()> {
            let left = self.0.get();
            if left == 0 {
                return Err("cancelled".into());
            }
            self.0.set(left - 1);
            Ok(())
        }
    }

    #[test]
    fn duplicate_lags_are_bounded_and_generation_is_cancellable() {
        // Total ordering distinguishes signed zeros; interval membership must
        // still use numerical comparison when a supplied jump is -0.
        assert_eq!(
            discontinuities(&CancelAfter(Cell::new(20)), 0., 4., &[1.], &[-0.], true).unwrap(),
            vec![0., 1., 2., 3.]
        );
        assert_eq!(
            discontinuities(
                &CancelAfter(Cell::new(2000)),
                0.,
                4.,
                &vec![1.; 1000],
                &[],
                true
            )
            .unwrap(),
            vec![0., 1., 2., 3.]
        );
        // Allow the 1000 lag inputs, then cancel during combination generation.
        assert_eq!(
            discontinuities(
                &CancelAfter(Cell::new(1001)),
                0.,
                4.,
                &vec![1.; 1000],
                &[],
                true
            )
            .unwrap_err(),
            "cancelled"
        );
    }

    #[test]
    fn trial_history_borrows_mesh_and_interpolates_only_the_tail() {
        let x = vec![0., 1.];
        let y = vec![vec![0.], vec![1.]];
        let yp = vec![vec![0.], vec![2.]];
        let past = Past {
            history: &History::Constant(vec![0.]),
            x: &x,
            y: &y,
            yp: &yp,
            trial: Some((2., &[4.], &[4.])),
        };
        assert_eq!(past.x.as_ptr(), x.as_ptr());
        assert_eq!(past.y.as_ptr(), y.as_ptr());
        for t in [0.25, 1.25, 2.25] {
            assert!(
                (past.value(&mut CancelAfter(Cell::new(10)), t).unwrap()[0] - t * t).abs() < 1e-12
            );
        }
    }

    type RightHandSide = fn(f64, &[f64], &[f64]) -> Vec<f64>;
    type EventFunction = fn(f64, &[f64], &[f64]) -> (Vec<f64>, Vec<f64>, Vec<f64>);

    struct Equation {
        rhs: RightHandSide,
        past: fn(f64) -> Vec<f64>,
        lag: fn(f64, &[f64]) -> Vec<f64>,
        event: EventFunction,
        checks: Cell<usize>,
        calls: usize,
        initial_events: usize,
    }

    impl Default for Equation {
        fn default() -> Self {
            Self {
                rhs: |_, _, z| vec![-z[0]],
                past: |_| vec![1.],
                lag: |t, _| vec![t - 1.],
                event: |t, _, _| (vec![t - 0.5], vec![1.], vec![0.]),
                checks: Cell::new(usize::MAX),
                calls: 0,
                initial_events: 0,
            }
        }
    }

    impl DdeModel for Equation {
        fn f(&mut self, t: f64, y: &[f64], z: &[f64]) -> R<Vec<f64>> {
            self.calls += 1;
            Ok((self.rhs)(t, y, z))
        }
        fn history(&mut self, t: f64) -> R<Vec<f64>> {
            Ok((self.past)(t))
        }
        fn delays(&mut self, t: f64, y: &[f64]) -> R<Vec<f64>> {
            Ok((self.lag)(t, y))
        }
        fn events(&mut self, t: f64, y: &[f64], z: &[f64]) -> R<(Vec<f64>, Vec<f64>, Vec<f64>)> {
            Ok((self.event)(t, y, z))
        }
        fn initial_events(&mut self, _: f64, _: &[f64], _: &[f64]) -> R<()> {
            self.initial_events += 1;
            Ok(())
        }
        fn check_cancel(&self) -> R<()> {
            let left = self.checks.get();
            if left == 0 {
                return Err("cancelled".into());
            }
            self.checks.set(left - 1);
            Ok(())
        }
    }

    fn options() -> DdeOpts {
        DdeOpts {
            rtol: 1e-7,
            threshold: vec![1e-3],
            hmax: 0.2,
            htry: None,
            jumps: vec![],
            initial_y: None,
            events: false,
        }
    }

    #[test]
    fn valid_small_maxstep_run_can_exceed_a_million_steps() {
        let mut opts = options();
        opts.hmax = 1e-6;
        let out = solve(
            &mut Equation::default(),
            Scheme::Bs23,
            &Delays::Lags(vec![1.]),
            &History::Constant(vec![1.]),
            0.,
            1.5,
            &opts,
        )
        .unwrap();
        assert_eq!(out.x.last(), Some(&1.5));
        assert!(out.stats[0] > 1_000_000);
        assert!(out.warnings.is_empty());
        assert!((out.y.last().unwrap()[0] + 0.375).abs() < 1e-8);
    }

    #[test]
    fn rejected_minimum_step_stops_even_when_stretched() {
        let mut model = Equation {
            rhs: |t, _, _| vec![if t == 1. { 0. } else { 1. }],
            checks: Cell::new(100),
            ..Equation::default()
        };
        let mut opts = options();
        opts.rtol = 1e-30;
        opts.htry = Some(16. * eps_of(1.));
        let out = solve(
            &mut model,
            Scheme::Bs23,
            &Delays::Lags(vec![1.]),
            &History::Constant(vec![1.]),
            1.,
            1. + 17. * eps_of(1.),
            &opts,
        )
        .unwrap();
        assert_eq!(out.x, [1.]);
        assert_eq!(out.stats[1], 1);
        assert_eq!(out.warnings.len(), 1);
        assert!(out.warnings[0].contains("below the minimum"));
    }

    #[test]
    fn discontinuity_limit_counts_distinct_points_instead_of_pairs() {
        let model = Equation::default();
        let lags: Vec<_> = (1..=600).map(|i| i as f64 / 600.).collect();
        let breaks = discontinuities(&model, 0., 4., &lags, &[], true).unwrap();
        assert_eq!(breaks.len(), 1801);
        assert_eq!(breaks.first(), Some(&0.));
        assert_eq!(breaks.last(), Some(&3.));
        // Square roots of distinct primes are incommensurate. Three additions
        // produce C(122,3) distinct triple sums, well above the removed cap.
        let lags: Vec<_> = (2usize..)
            .filter(|n| (2..=*n / 2).all(|d| !n.is_multiple_of(d)))
            .take(120)
            .map(|n| libm::sqrt(n as f64) / 10.)
            .collect();
        let breaks = discontinuities(&model, 0., 10., &lags, &[], true).unwrap();
        assert_eq!(breaks.len(), 1 + 120 + 120 * 121 / 2 + 120 * 121 * 122 / 6);
        assert!(breaks.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn steps_stretch_to_nearby_interior_breaks_and_final_time() {
        let mut opts = options();
        opts.rtol = 1e-3;
        let out = solve(
            &mut Equation::default(),
            Scheme::Bs23,
            &Delays::Lags(vec![1.]),
            &History::Constant(vec![1.]),
            0.,
            3.,
            &opts,
        )
        .unwrap();
        assert!(out.x.contains(&1.) && out.x.contains(&2.));
        assert_eq!(out.x.last(), Some(&3.));
        assert!(out.x.windows(2).all(|p| p[1] - p[0] > 0.01), "{:?}", out.x);
    }

    #[test]
    fn solution_history_restart_only_propagates_an_actual_initial_jump() {
        let first = solve(
            &mut Equation::default(),
            Scheme::Bs23,
            &Delays::Lags(vec![1.]),
            &History::Constant(vec![1.]),
            0.,
            0.5,
            &options(),
        )
        .unwrap();
        let initial = first.y.last().unwrap().clone();
        let history = History::Solution(first.x, first.y, first.yp);
        for initial_y in [None, Some(initial), Some(vec![2.])] {
            let jump = initial_y.as_ref().is_some_and(|y| y == &[2.]);
            let mut opts = options();
            opts.initial_y = initial_y;
            let out = solve(
                &mut Equation::default(),
                Scheme::Bs23,
                &Delays::Lags(vec![1.]),
                &history,
                0.5,
                2.,
                &opts,
            )
            .unwrap();
            if jump {
                assert_eq!(out.discont, [0.5, 1.5]);
                assert!(out.x.contains(&1.5));
            } else {
                assert!(out.discont.is_empty());
                assert!(!out.x.contains(&1.5));
                assert!((out.y.last().unwrap()[0] + 0.5).abs() < 2e-6);
            }
        }
    }

    #[test]
    fn terminal_event_discards_unreached_discontinuities() {
        let mut model = Equation {
            event: |_, y, _| (vec![y[0] + 0.25], vec![1.], vec![0.]),
            ..Equation::default()
        };
        let mut opts = options();
        opts.events = true;
        let out = solve(
            &mut model,
            Scheme::Bs23,
            &Delays::Lags(vec![1.]),
            &History::Constant(vec![1.]),
            0.,
            3.,
            &opts,
        )
        .unwrap();
        assert!((out.x.last().unwrap() - (2. - libm::sqrt(0.5))).abs() < 2e-7);
        assert_eq!(out.discont, [0., 1.]);
        assert_eq!(out.te.last(), out.x.last());
    }

    #[test]
    fn method_of_steps_matches_piecewise_polynomials_and_counts_callbacks() {
        for scheme in [Scheme::Bs23, Scheme::Rk4] {
            let mut model = Equation::default();
            let out = solve(
                &mut model,
                scheme,
                &Delays::Lags(vec![1.]),
                &History::Constant(vec![1.]),
                0.,
                3.,
                &options(),
            )
            .unwrap();
            assert!(out.warnings.is_empty());
            assert_eq!(out.x.last(), Some(&3.));
            assert_eq!(out.stats[0], out.x.len() - 1);
            assert_eq!(out.stats[2], model.calls);
            for i in 0..=60 {
                let t = i as f64 / 20.;
                let exact = 1. - t + 0.5 * (t - 1.).max(0.).powi(2) - (t - 2.).max(0.).powi(3) / 6.;
                let actual = interp(&out.x, &out.y, &out.yp, t)[0];
                assert!((actual - exact).abs() < 2e-7, "t={t}: {actual} vs {exact}");
            }
        }
    }

    #[test]
    fn short_delays_allow_steps_longer_than_the_lag() {
        for scheme in [Scheme::Bs23, Scheme::Rk4] {
            // exp(-t/10) solves y'=-exp(-lag/10)*y(t-lag)/10.
            let mut model = Equation {
                rhs: |_, _, z| vec![-0.1 * libm::exp(-0.0001) * z[0]],
                past: |t| vec![libm::exp(-0.1 * t)],
                ..Equation::default()
            };
            let mut opts = options();
            opts.htry = Some(0.2);
            let out = solve(
                &mut model,
                scheme,
                &Delays::Lags(vec![0.001]),
                &History::Function,
                0.,
                1.,
                &opts,
            )
            .unwrap();
            assert!(out.warnings.is_empty());
            assert!(out.x.windows(2).any(|x| x[1] - x[0] > 0.01));
            assert!((out.y.last().unwrap()[0] - libm::exp(-0.1)).abs() < 2e-7);
        }
    }

    #[test]
    fn residual_control_handles_time_state_and_zero_delays() {
        for lag in [
            (|t, _: &[f64]| vec![t / 2.]) as fn(f64, &[f64]) -> Vec<f64>,
            |t, y| vec![t - 0.02 / (1. + y[0] * y[0])],
            |t, _| vec![t],
        ] {
            let zero = lag(1., &[1.])[0] == 1.;
            let time = lag(1., &[1.])[0] == 0.5;
            let mut model = Equation {
                lag,
                rhs: if zero {
                    |_, y, z| {
                        assert_eq!(y, z);
                        vec![-z[0]]
                    }
                } else if time {
                    |_, _, z| vec![-z[0]]
                } else {
                    |_, y, z| vec![-libm::exp(-0.02 / (1. + y[0] * y[0])) * z[0]]
                },
                past: |t| vec![libm::exp(-t)],
                ..Equation::default()
            };
            let out = solve(
                &mut model,
                Scheme::Rk4,
                &Delays::Function(1),
                &History::Function,
                0.,
                2.,
                &options(),
            )
            .unwrap();
            assert!(out.warnings.is_empty());
            let exact = if time {
                // Taylor recurrence for y'=-y(t/2), y(0)=1.
                let mut term = 1.;
                let mut sum = term;
                for k in 1..20 {
                    term *= -2. / (k as f64 * (2f64).powi(k - 1));
                    sum += term;
                }
                sum
            } else {
                libm::exp(-2.)
            };
            assert!(
                (out.y.last().unwrap()[0] - exact).abs() < 2e-7,
                "time={time}, zero={zero}: {:?} vs {exact}",
                out.y.last()
            );
        }
    }

    #[test]
    fn rejected_steps_still_land_on_sorted_jumps() {
        for scheme in [Scheme::Bs23, Scheme::Rk4] {
            let mut model = Equation {
                rhs: |_, y, _| vec![10. * y[0]],
                ..Equation::default()
            };
            let mut opts = options();
            opts.htry = Some(0.95);
            opts.hmax = 1.;
            opts.jumps = vec![0.97, 0.3, 0.3];
            let out = solve(
                &mut model,
                scheme,
                &Delays::Function(1),
                &History::Constant(vec![1.]),
                0.,
                1.,
                &opts,
            )
            .unwrap();
            assert!(out.stats[1] > 0);
            assert!(out.x.contains(&0.3) && out.x.contains(&0.97));
            assert_eq!(out.x.last(), Some(&1.));
        }
    }

    #[test]
    fn terminal_event_restricts_the_same_dense_cubic() {
        for scheme in [Scheme::Bs23, Scheme::Rk4] {
            let mut model = Equation {
                rhs: |t, _, _| vec![2. * t],
                ..Equation::default()
            };
            let mut opts = options();
            opts.htry = Some(1.);
            opts.hmax = 1.;
            opts.events = true;
            let out = solve(
                &mut model,
                scheme,
                &Delays::Lags(vec![2.]),
                &History::Constant(vec![0.]),
                0.,
                1.,
                &opts,
            )
            .unwrap();
            assert_eq!(model.initial_events, 1);
            assert_eq!(out.te, [0.5]);
            assert_eq!(out.ie, [1]);
            assert_eq!(out.x.last(), Some(&0.5));
            assert!((out.yp.last().unwrap()[0] - 1.).abs() < 1e-12);
            assert!((interp(&out.x, &out.y, &out.yp, 0.25)[0] - 0.0625).abs() < 1e-12);
        }
    }

    #[test]
    fn initial_terminal_event_is_recorded_without_stopping() {
        let mut model = Equation {
            rhs: |_, _, _| vec![1.],
            event: |_, y, _| (vec![y[0]], vec![1.], vec![]),
            ..Equation::default()
        };
        let mut opts = options();
        opts.events = true;
        let out = solve(
            &mut model,
            Scheme::Bs23,
            &Delays::Lags(vec![2.]),
            &History::Constant(vec![0.]),
            0.,
            1.,
            &opts,
        )
        .unwrap();
        assert_eq!(out.te, [0.]);
        assert_eq!(out.ie, [1]);
        assert_eq!(out.x.last(), Some(&1.));
    }

    #[test]
    fn initial_y_keeps_the_left_history_and_propagates_its_jump() {
        for scheme in [Scheme::Bs23, Scheme::Rk4] {
            let mut model = Equation::default();
            let mut opts = options();
            opts.initial_y = Some(vec![2.]);
            let out = solve(
                &mut model,
                scheme,
                &Delays::Lags(vec![1.]),
                &History::Constant(vec![1.]),
                0.,
                2.,
                &opts,
            )
            .unwrap();
            assert!(out.warnings.is_empty());
            assert_eq!(out.y[0], [2.]);
            assert_eq!(out.yp[0], [-1.]);
            for t in [0.5, 1., 1.5, 2.] {
                let exact = if t <= 1. {
                    2. - t
                } else {
                    3.5 - 3. * t + 0.5 * t * t
                };
                let value = interp(&out.x, &out.y, &out.yp, t)[0];
                assert!((value - exact).abs() < 2e-7, "t={t}: {value} vs {exact}");
            }
        }
    }

    #[test]
    fn callback_validation_and_cancellation_are_not_bypassed_by_iteration() {
        for (lag, message) in [
            (
                (|_, _: &[f64]| vec![f64::NAN]) as fn(f64, &[f64]) -> Vec<f64>,
                "delay function",
            ),
            (|t, _| vec![t + 1.], "delay function"),
            (
                |t, _| if t == 0. { vec![-1.] } else { vec![-1., -2.] },
                "constant length",
            ),
        ] {
            let mut model = Equation {
                lag,
                ..Equation::default()
            };
            let error = solve(
                &mut model,
                Scheme::Rk4,
                &Delays::Function(1),
                &History::Constant(vec![1.]),
                0.,
                1.,
                &options(),
            )
            .err()
            .unwrap();
            assert!(error.contains(message), "{error}");
        }
        let mut model = Equation {
            checks: Cell::new(30),
            ..Equation::default()
        };
        let error = solve(
            &mut model,
            Scheme::Rk4,
            &Delays::Function(1),
            &History::Constant(vec![1.]),
            0.,
            1.,
            &options(),
        )
        .err()
        .unwrap();
        assert_eq!(error, "cancelled");
    }
}
