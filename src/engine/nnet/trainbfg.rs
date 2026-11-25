//! BFGS inverse-Hessian rank-two update; Dennis & Schnabel (1983) backtracking
//! with quadratic, then cubic interpolation (MathWorks public srchbac page).
use super::*;
#[derive(Default)]
pub struct Bfg {
    inverse: Vec<f64>,
    step_size: f64,
}
impl TrainFn for Bfg {
    fn defaults(&self) -> Parameters {
        let mut p = common_defaults(1e-6);
        p.extend(
            [
                ("scal_tol", 20.),
                ("alpha", 0.001),
                ("beta", 0.1),
                ("delta", 0.01),
                ("gamma", 0.1),
                ("low_lim", 0.1),
                ("up_lim", 0.5),
                ("maxstep", 100.),
                ("minstep", 1e-6),
                ("bmax", 26.),
                ("batch_frag", 0.),
            ]
            .map(|(k, v)| (k.into(), v)),
        );
        p
    }
    fn string_defaults(&self) -> BTreeMap<String, String> {
        [("searchFcn".into(), "srchbac".into())].into()
    }
    fn workspace(&self, w: usize) -> usize {
        w.saturating_mul(w).saturating_mul(24)
    }
    fn initialize(&mut self, p: &Parameters, o: &Objective<'_>) -> Result<()> {
        if o.string_parameters
            .get("searchFcn")
            .is_some_and(|s| s != "srchbac")
        {
            return Err("trainbfg: only searchFcn srchbac is supported".into());
        }
        if p["alpha"] <= 0.
            || p["alpha"] >= p["beta"]
            || p["beta"] >= 1.
            || p["low_lim"] <= 0.
            || p["low_lim"] >= p["up_lim"]
            || p["up_lim"] >= 1.
            || p["minstep"] <= 0.
            || p["maxstep"] < p["minstep"]
            || p["scal_tol"] <= 0.
            || p["delta"] <= 0.
        {
            return Err("trainbfg: invalid srchbac parameters".into());
        }
        if p["batch_frag"] != 0. {
            return Err("trainbfg: fragmented batches are not supported".into());
        }
        Ok(())
    }
    fn step(
        &mut self,
        net: &mut Network,
        o: &Objective<'_>,
        e: &Evaluation,
        p: &Parameters,
    ) -> Result<()> {
        let n = e.gradient.len();
        if self.inverse.is_empty() {
            self.inverse = bfg_identity(n);
        }
        let mut direction = bfg_product(&self.inverse, &e.gradient);
        for d in &mut direction {
            *d = -*d;
        }
        if dot(&direction, &e.gradient) >= 0. || direction.iter().any(|d| !d.is_finite()) {
            self.inverse = bfg_identity(n);
            direction = e.gradient.iter().map(|g| -g).collect();
        }
        let Some((candidate, next, step)) = bfg_search(net, o, e, &direction, p)? else {
            self.step_size = 0.;
            return Ok(());
        };
        self.step_size = step;
        let s: Vec<_> = direction.iter().map(|d| step * d).collect();
        let y: Vec<_> = next
            .gradient
            .iter()
            .zip(&e.gradient)
            .map(|(a, b)| a - b)
            .collect();
        let sy = dot(&s, &y);
        if sy > f64::EPSILON * (dot(&s, &s) * dot(&y, &y)).sqrt() {
            let hy = bfg_product(&self.inverse, &y);
            let coefficient = (sy + dot(&y, &hy)) / (sy * sy);
            for j in 0..n {
                crate::dsp::check_cancel(o.cancel)?;
                for i in 0..n {
                    self.inverse[j * n + i] +=
                        coefficient * s[i] * s[j] - (hy[i] * s[j] + s[i] * hy[j]) / sy;
                }
            }
        } else {
            self.inverse = bfg_identity(n);
        }
        *net = candidate;
        Ok(())
    }
    fn extra_fields(&self) -> Vec<(&'static str, f64)> {
        vec![("stepsize", self.step_size)]
    }
}
fn bfg_identity(n: usize) -> Vec<f64> {
    let mut a = vec![0.; n * n];
    for i in 0..n {
        a[i * n + i] = 1.;
    }
    a
}
fn bfg_product(a: &[f64], x: &[f64]) -> Vec<f64> {
    let n = x.len();
    (0..n)
        .map(|i| (0..n).map(|j| a[j * n + i] * x[j]).sum())
        .collect()
}
fn bfg_search(
    net: &Network,
    o: &Objective<'_>,
    e: &Evaluation,
    d: &[f64],
    p: &Parameters,
) -> Result<Option<(Network, Evaluation, f64)>> {
    let norm = dot(d, d).sqrt();
    if norm == 0. {
        return Ok(None);
    }
    let slope = dot(&e.gradient, d);
    let maximum = p["maxstep"] / norm;
    let minimum = p["minstep"] / norm;
    let mut step = 1_f64.min(maximum);
    let mut previous: Option<(f64, f64)> = None;
    let mut lower = 0.;
    let mut upper = f64::INFINITY;
    let mut accepted = None;
    // The docs leave retry bounds unspecified. This also bounds cancellation latency.
    for _ in 0..128 {
        crate::dsp::check_cancel(o.cancel)?;
        if o.time_expired() {
            return Ok(accepted);
        }
        let candidate = trial(net, d, step).and_then(|n| o.evaluate(&n, false).map(|e| (n, e)));
        crate::dsp::check_cancel(o.cancel)?;
        let perf = candidate.as_ref().map_or(f64::INFINITY, |(_, e)| e.perf);
        if perf <= e.perf + p["alpha"] * step * slope {
            let (candidate, next) = candidate?;
            let sufficient = dot(&next.gradient, d) >= p["beta"] * slope;
            accepted = Some((candidate, next, step));
            if sufficient || step >= maximum {
                return Ok(accepted);
            }
            lower = step;
            step = if upper.is_finite() {
                (lower + upper) * 0.5
            } else {
                (step / p["up_lim"]).min(maximum)
            };
            previous = None;
        } else {
            upper = step;
            let proposal = bfg_interpolate(step, perf, e.perf, slope, previous);
            previous = Some((step, perf));
            let next = proposal.clamp(p["low_lim"] * step, p["up_lim"] * step);
            step = if next > lower {
                next
            } else {
                (lower + upper) * 0.5
            };
        }
        if step < minimum || (upper - lower).abs() < minimum.min(p["delta"] / p["scal_tol"]) {
            return Ok(accepted);
        }
    }
    Ok(accepted)
}
fn bfg_interpolate(
    step: f64,
    perf: f64,
    initial: f64,
    slope: f64,
    previous: Option<(f64, f64)>,
) -> f64 {
    let value = if let Some((old, old_perf)) = previous {
        let a1 = (perf - initial - step * slope) / (step * step);
        let a2 = (old_perf - initial - old * slope) / (old * old);
        let a = (a1 - a2) / (step - old);
        let b = (-old * a1 + step * a2) / (step - old);
        if a == 0. {
            -slope / (2. * b)
        } else {
            let disc = b * b - 3. * a * slope;
            if b <= 0. {
                (-b + disc.sqrt()) / (3. * a)
            } else {
                -slope / (b + disc.sqrt())
            }
        }
    } else {
        -slope * step * step / (2. * (perf - initial - step * slope))
    };
    if value.is_finite() && value > 0. {
        value
    } else {
        step * 0.5
    }
}
