//! Moller (1993), SCG with finite-difference directional curvature.
use super::*;
#[derive(Default)]
pub struct Scg {
    direction: Vec<f64>,
    lambda: f64,
    success: bool,
    successes: usize,
    theta: f64,
    kappa: f64,
    slope: f64,
}
impl TrainFn for Scg {
    fn defaults(&self) -> Parameters {
        let mut p = common_defaults(1e-6);
        p.extend([("mu", 0.005), ("sigma", 5e-5), ("lambda", 5e-7)].map(|(k, v)| (k.into(), v)));
        p
    }
    fn initialize(&mut self, p: &Parameters, _: &Objective<'_>) -> Result<()> {
        if p["sigma"] <= 0. || p["lambda"] <= 0. {
            return Err("trainscg: sigma and lambda must be positive".into());
        }
        self.lambda = p["lambda"];
        self.success = true;
        Ok(())
    }
    fn step(
        &mut self,
        net: &mut Network,
        o: &Objective<'_>,
        current: &Evaluation,
        p: &Parameters,
    ) -> Result<()> {
        let g = &current.gradient;
        if self.direction.is_empty() {
            self.direction = g.iter().map(|v| -v).collect();
        }
        if self.success {
            self.slope = dot(&self.direction, g);
            if self.slope >= 0. {
                self.direction = g.iter().map(|v| -v).collect();
                self.slope = -dot(g, g);
            }
            self.kappa = dot(&self.direction, &self.direction);
            if self.kappa == 0. {
                return Ok(());
            }
            let sigma = p["sigma"] / self.kappa.sqrt();
            let probe = trial(net, &self.direction, sigma)?;
            let gp = o.evaluate(&probe, false)?.gradient;
            self.theta = self
                .direction
                .iter()
                .zip(gp.iter().zip(g))
                .map(|(d, (a, b))| d * (a - b))
                .sum::<f64>()
                / sigma;
        }
        let mut delta = self.theta + self.lambda * self.kappa;
        if delta <= 0. {
            delta = self.lambda * self.kappa;
            self.lambda -= self.theta / self.kappa;
        }
        let alpha = -self.slope / delta;
        let candidate = trial(net, &self.direction, alpha)?;
        let perf = o.performance(&candidate, o.indices)?;
        let comparison = 2. * (perf - current.perf) / (alpha * self.slope);
        self.success = comparison.is_finite() && comparison >= 0.;
        if self.success {
            let next = o.evaluate(&candidate, false)?;
            *net = candidate;
            self.successes += 1;
            if self.successes == g.len() {
                self.direction = next.gradient.iter().map(|v| -v).collect();
                self.successes = 0;
            } else {
                let gamma = g
                    .iter()
                    .zip(&next.gradient)
                    .map(|(a, b)| (a - b) * b)
                    .sum::<f64>()
                    / self.slope;
                for (d, b) in self.direction.iter_mut().zip(next.gradient) {
                    *d = gamma * (*d) - b;
                }
            }
        }
        if comparison < 0.25 || !comparison.is_finite() {
            self.lambda = (4. * self.lambda).min(1e100);
        } else if comparison > 0.75 {
            self.lambda = (0.5 * self.lambda).max(1e-100);
        }
        Ok(())
    }
    fn extra_fields(&self) -> Vec<(&'static str, f64)> {
        vec![("lambda", self.lambda)]
    }
}
