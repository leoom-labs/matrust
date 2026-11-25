//! MacKay (1992), Foresee & Hagan (1997): evidence updates after accepted LM
//! steps on beta*sum(error^2) + alpha*sum(weight^2), including biases.
//! tr.perf stays in the requested data-loss units; gradient is the MAP gradient.
use super::*;
use std::cell::Cell;
#[derive(Default)]
pub struct Br {
    mu: f64,
    alpha: f64,
    beta: f64,
    gamk: Cell<f64>,
    ss_x: Cell<f64>,
}
impl TrainFn for Br {
    fn defaults(&self) -> Parameters {
        let mut p = common_defaults(1e-7);
        p.extend(
            [
                ("max_fail", 0.),
                ("mu", 0.005),
                ("mu_dec", 0.1),
                ("mu_inc", 10.),
                ("mu_max", 1e10),
            ]
            .map(|(k, v)| (k.into(), v)),
        );
        p
    }
    fn workspace(&self, w: usize) -> usize {
        w.saturating_mul(w).saturating_mul(96)
    }
    fn initialize(&mut self, p: &Parameters, o: &Objective<'_>) -> Result<()> {
        if !matches!(o.kind, "mse" | "sse") {
            return Err("trainbr: requires mse or sse performance".into());
        }
        if p["mu"] <= 0.
            || p["mu_inc"] <= 1.
            || p["mu_dec"] <= 0.
            || p["mu_dec"] >= 1.
            || p["mu_max"] <= 0.
        {
            return Err("trainbr: require mu > 0, mu_inc > 1, 0 < mu_dec < 1, mu_max > 0".into());
        }
        self.mu = p["mu"];
        // Positive weak prior makes evidence well-defined even for N < P.
        self.alpha = 0.001;
        self.beta = 1.;
        Ok(())
    }
    fn evaluate(&self, net: &Network, o: &Objective<'_>) -> Result<Evaluation> {
        let mut e = o.data_evaluation(net, self.gamk.get() == 0.)?;
        let w = net.getwb();
        let count = (net.output.size * o.indices.len()) as f64;
        let scale = if o.kind == "mse" { count } else { 1. };
        self.ss_x.set(dot(&w, &w));
        if let Some(h) = e.curvature.take() {
            self.gamk
                .set(br_effective(&h, scale, self.alpha, self.beta, o)?);
        }
        for (g, w) in e.gradient.iter_mut().zip(w) {
            *g = self.beta * scale * *g + 2. * self.alpha * w;
        }
        Ok(e)
    }
    fn step(
        &mut self,
        net: &mut Network,
        o: &Objective<'_>,
        e: &Evaluation,
        p: &Parameters,
    ) -> Result<()> {
        let data = o.data_evaluation(net, true)?;
        let h = data.curvature.unwrap();
        let n = h.rows();
        let count = (net.output.size * o.indices.len()) as f64;
        let scale = if o.kind == "mse" { count } else { 1. };
        let energy = self.beta * scale * data.perf + self.alpha * self.ss_x.get();
        let rhs = Matrix::from_column_vec(n, 1, e.gradient.iter().map(|g| -0.5 * g).collect())?;
        let mut retries = 0;
        while self.mu <= p["mu_max"] {
            crate::dsp::check_cancel(o.cancel)?;
            if o.time_expired() {
                return Ok(());
            }
            retries += 1;
            if retries > 1024 {
                return Err("trainbr: mu retry limit exceeded; increase mu_inc".into());
            }
            let mut system: Vec<_> = h.data().iter().map(|v| self.beta * scale * v).collect();
            for i in 0..n {
                system[i * n + i] += self.alpha + self.mu;
            }
            let candidate = Matrix::from_column_vec(n, n, system)?
                .solve(&rhs)
                .and_then(|d| trial(net, d.data(), 1.));
            if let Ok(candidate) = candidate {
                let proposed = o.data_evaluation(&candidate, false);
                crate::dsp::check_cancel(o.cancel)?;
                if let Ok(next) = proposed {
                    let w = candidate.getwb();
                    let ew = dot(&w, &w);
                    if self.beta * scale * next.perf + self.alpha * ew < energy {
                        let h = o.data_evaluation(&candidate, true)?.curvature.unwrap();
                        let gamk = br_effective(&h, scale, self.alpha, self.beta, o)?.min(count);
                        self.gamk.set(gamk);
                        self.alpha = (gamk / (2. * ew.max(f64::EPSILON)))
                            .clamp(f64::EPSILON, 1. / f64::EPSILON);
                        self.beta = ((count - gamk) / (2. * (scale * next.perf).max(f64::EPSILON)))
                            .clamp(f64::EPSILON, 1. / f64::EPSILON);
                        self.mu = (self.mu * p["mu_dec"]).max(f64::MIN_POSITIVE);
                        *net = candidate;
                        return Ok(());
                    }
                }
            }
            self.mu *= p["mu_inc"];
        }
        Ok(())
    }
    fn extra_fields(&self) -> Vec<(&'static str, f64)> {
        vec![
            ("mu", self.mu),
            ("gamk", self.gamk.get()),
            ("ssX", self.ss_x.get()),
        ]
    }
    fn stop(&self, p: &Parameters) -> Option<&'static str> {
        (self.mu > p["mu_max"]).then_some("Maximum MU reached.")
    }
}
/// trace(C^-1 beta J'J) = P - alpha trace(C^-1). Factor once and solve
/// column-wise, avoiding both a full inverse and subtraction of nearly equal P.
fn br_effective(h: &Matrix, scale: f64, alpha: f64, beta: f64, o: &Objective<'_>) -> Result<f64> {
    let n = h.rows();
    let data = nalgebra::DMatrix::from_column_slice(n, n, h.data()) * (beta * scale);
    let mut c = data.clone();
    for i in 0..n {
        c[(i, i)] += alpha;
    }
    let factor = c
        .cholesky()
        .ok_or("trainbr: evidence curvature is not positive definite")?;
    let mut gamma = 0.;
    for i in 0..n {
        crate::dsp::check_cancel(o.cancel)?;
        let column = factor.solve(&data.column(i));
        gamma += column[i];
    }
    Ok(gamma.clamp(f64::EPSILON, n.max(1) as f64))
}
