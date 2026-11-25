//! Momentum filters the negative gradient with coefficient (1-mc).
use super::*;
#[derive(Default)]
pub struct Gdm {
    velocity: Vec<f64>,
}
impl TrainFn for Gdm {
    fn defaults(&self) -> Parameters {
        let mut p = common_defaults(1e-5);
        p.extend([("lr", 0.01), ("mc", 0.9)].map(|(k, v)| (k.into(), v)));
        p
    }
    fn initialize(&mut self, p: &Parameters, _: &Objective<'_>) -> Result<()> {
        if p["mc"] > 1. {
            return Err("traingdm: mc must be in [0,1]".into());
        }
        Ok(())
    }
    fn step(
        &mut self,
        net: &mut Network,
        _: &Objective<'_>,
        e: &Evaluation,
        p: &Parameters,
    ) -> Result<()> {
        self.velocity.resize(e.gradient.len(), 0.);
        for (v, g) in self.velocity.iter_mut().zip(&e.gradient) {
            *v = p["mc"] * *v - (1. - p["mc"]) * p["lr"] * g;
        }
        *net = trial(net, &self.velocity, 1.)?;
        Ok(())
    }
}
