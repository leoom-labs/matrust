//! Riedmiller & Braun (1993), original Rprop with weight backtracking.
use super::*;
#[derive(Default)]
pub struct Rp {
    previous: Vec<f64>,
    delta: Vec<f64>,
    step: Vec<f64>,
}
impl TrainFn for Rp {
    fn defaults(&self) -> Parameters {
        let mut p = common_defaults(1e-5);
        p.extend(
            [
                ("lr", 0.01),
                ("delt_inc", 1.2),
                ("delt_dec", 0.5),
                ("delta0", 0.07),
                ("deltamax", 50.),
            ]
            .map(|(k, v)| (k.into(), v)),
        );
        p
    }
    fn initialize(&mut self, p: &Parameters, _: &Objective<'_>) -> Result<()> {
        if p["delt_inc"] <= 1.
            || p["delt_dec"] <= 0.
            || p["delt_dec"] >= 1.
            || p["delta0"] <= 0.
            || p["deltamax"] < p["delta0"]
        {
            return Err(
                "trainrp: require delt_inc > 1, 0 < delt_dec < 1, 0 < delta0 <= deltamax".into(),
            );
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
        self.previous.resize(n, 0.);
        self.delta.resize(n, p["delta0"]);
        self.step.resize(n, 0.);
        let mut change = vec![0.; n];
        for (i, (&g, change)) in e.gradient.iter().zip(&mut change).enumerate() {
            if i.is_multiple_of(256) {
                crate::dsp::check_cancel(o.cancel)?;
            }
            let product = g * self.previous[i];
            if product < 0. {
                self.delta[i] = (self.delta[i] * p["delt_dec"]).max(1e-6_f64.min(p["delta0"]));
                *change = -self.step[i];
                self.previous[i] = 0.;
                self.step[i] = 0.;
            } else {
                if product > 0. {
                    self.delta[i] = (self.delta[i] * p["delt_inc"]).min(p["deltamax"]);
                }
                *change = if g == 0. {
                    0.
                } else {
                    -g.signum() * self.delta[i]
                };
                self.step[i] = *change;
                self.previous[i] = g;
            }
        }
        *net = trial(net, &change, 1.)?;
        Ok(())
    }
}
