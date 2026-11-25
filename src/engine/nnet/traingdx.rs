//! Adaptive learning rate with momentum. Rejection clears momentum, then retries
//! on the next epoch. See tool/refs/nnet/trainers.py for public-doc ambiguities.
use super::*;
#[derive(Default)]
pub struct Gdx {
    velocity: Vec<f64>,
    lr: f64,
}
impl TrainFn for Gdx {
    fn defaults(&self) -> Parameters {
        let mut p = common_defaults(1e-5);
        p.extend(
            [
                ("lr", 0.01),
                ("mc", 0.9),
                ("lr_inc", 1.05),
                ("lr_dec", 0.7),
                ("max_perf_inc", 1.04),
            ]
            .map(|(k, v)| (k.into(), v)),
        );
        p
    }
    fn initialize(&mut self, p: &Parameters, _: &Objective<'_>) -> Result<()> {
        if p["mc"] > 1.
            || p["lr_inc"] <= 1.
            || p["lr_dec"] <= 0.
            || p["lr_dec"] >= 1.
            || p["max_perf_inc"] < 1.
        {
            return Err(
                "traingdx: require mc in [0,1], lr_inc > 1, 0 < lr_dec < 1, max_perf_inc >= 1"
                    .into(),
            );
        }
        self.lr = p["lr"];
        Ok(())
    }
    fn step(
        &mut self,
        net: &mut Network,
        o: &Objective<'_>,
        e: &Evaluation,
        p: &Parameters,
    ) -> Result<()> {
        self.velocity.resize(e.gradient.len(), 0.);
        let step: Vec<_> = self
            .velocity
            .iter()
            .zip(&e.gradient)
            .map(|(v, g)| p["mc"] * v - (1. - p["mc"]) * self.lr * g)
            .collect();
        let candidate =
            trial(net, &step, 1.).and_then(|n| o.evaluate(&n, false).map(|e| (n, e.perf)));
        crate::dsp::check_cancel(o.cancel)?;
        if let Ok((candidate, perf)) = candidate
            && perf <= e.perf * p["max_perf_inc"]
        {
            if perf < e.perf {
                self.lr = (self.lr * p["lr_inc"]).min(f64::MAX);
            }
            *net = candidate;
            self.velocity = step;
        } else {
            self.lr *= p["lr_dec"];
            self.velocity.fill(0.);
        }
        Ok(())
    }
    fn extra_fields(&self) -> Vec<(&'static str, f64)> {
        vec![("lr", self.lr)]
    }
}
