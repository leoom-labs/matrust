//! Batch steepest descent (MathWorks public traingd contract).
use super::*;
#[derive(Default)]
pub struct Gd;
impl TrainFn for Gd {
    fn defaults(&self) -> Parameters {
        let mut p = common_defaults(1e-5);
        p.insert("lr".into(), 0.01);
        p
    }
    fn step(
        &mut self,
        net: &mut Network,
        _: &Objective<'_>,
        e: &Evaluation,
        p: &Parameters,
    ) -> Result<()> {
        *net = trial(net, &e.gradient, -p["lr"])?;
        Ok(())
    }
}
