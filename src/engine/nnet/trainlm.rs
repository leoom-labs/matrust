//! Hagan & Menhaj (1994), damped Gauss-Newton, strict descent acceptance.
use super::*;
#[derive(Default)]
pub struct Lm {
    mu: f64,
}
impl TrainFn for Lm {
    fn defaults(&self) -> Parameters {
        let mut p = common_defaults(1e-7);
        p.extend(
            [
                ("mu", 0.001),
                ("mu_dec", 0.1),
                ("mu_inc", 10.),
                ("mu_max", 1e10),
            ]
            .map(|(k, v)| (k.into(), v)),
        );
        p
    }
    fn workspace(&self, w: usize) -> usize {
        w.saturating_mul(w).saturating_mul(64)
    }
    fn initialize(&mut self, p: &Parameters, o: &Objective<'_>) -> Result<()> {
        if !matches!(o.kind, "mse" | "sse") {
            return Err("trainlm: requires mse or sse performance".into());
        }
        if p["mu"] <= 0.
            || p["mu_inc"] <= 1.
            || p["mu_dec"] <= 0.
            || p["mu_dec"] >= 1.
            || p["mu_max"] <= 0.
        {
            return Err("trainlm: require mu > 0, mu_inc > 1, 0 < mu_dec < 1, mu_max > 0".into());
        }
        self.mu = p["mu"];
        Ok(())
    }
    fn step(
        &mut self,
        net: &mut Network,
        o: &Objective<'_>,
        current: &Evaluation,
        p: &Parameters,
    ) -> Result<()> {
        let evaluation = o.evaluate(net, true)?;
        let h = evaluation.curvature.unwrap();
        let n = h.rows();
        let rhs =
            Matrix::from_column_vec(n, 1, current.gradient.iter().map(|v| -0.5 * v).collect())?;
        // Retry rejected steps within the epoch. Decrease mu only after acceptance.
        let mut retries = 0;
        while self.mu <= p["mu_max"] {
            if o.time_expired() {
                return Ok(());
            }
            retries += 1;
            if retries > 1024 {
                return Err("trainlm: mu retry limit exceeded; increase mu_inc".into());
            }
            crate::dsp::check_cancel(o.cancel)?;
            let mut damped = h.data().to_vec();
            for i in 0..n {
                damped[i * n + i] += self.mu;
            }
            let system = Matrix::from_column_vec(n, n, damped)?;
            if let Ok(step) = system.solve(&rhs)
                && let Ok(candidate) = trial(net, step.data(), 1.)
            {
                match o.performance(&candidate, o.indices) {
                    Ok(perf) if perf < current.perf => {
                        *net = candidate;
                        self.mu = (self.mu * p["mu_dec"]).max(f64::MIN_POSITIVE);
                        return Ok(());
                    }
                    _ => {
                        crate::dsp::check_cancel(o.cancel)?;
                    }
                }
            }
            self.mu *= p["mu_inc"];
        }
        Ok(())
    }
    fn extra_fields(&self) -> Vec<(&'static str, f64)> {
        vec![("mu", self.mu)]
    }
    fn stop(&self, p: &Parameters) -> Option<&'static str> {
        if self.mu > p["mu_max"] {
            Some("Maximum MU reached.")
        } else {
            None
        }
    }
}
