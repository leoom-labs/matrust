//! Desktop-independent batch SOM training with the shallow trainer's epoch callback.
//! Kohonen batch centroids; reference: tool/refs/nnet/dynamic.py.
use super::train::EpochReport;
use crate::{Matrix, Result};
use std::{
    collections::BTreeMap,
    ops::ControlFlow,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

pub struct SomOptions<'a> {
    pub epochs: usize,
    pub cover_steps: usize,
    pub initial_neighbor: f64,
    pub time_limit: Duration,
    pub memory_bytes: usize,
    pub cancel: &'a AtomicBool,
}
pub struct SomRecord {
    pub epochs: Vec<EpochReport>,
    pub stop: String,
}
/// Immutable inputs and weights. Cancellation is an error; callback Break returns
/// current weights, so callers can use the same progress UI as supervised train.
pub fn train(
    weights: &Matrix,
    x: &Matrix,
    distances: &Matrix,
    opts: SomOptions<'_>,
    progress: &mut dyn FnMut(EpochReport) -> ControlFlow<()>,
) -> Result<(Matrix, SomRecord)> {
    crate::dsp::check_cancel(opts.cancel)?;
    let s = weights.rows();
    if s == 0
        || x.cols() == 0
        || x.rows() == 0
        || distances.data().iter().any(|&x| x < 0.)
        || weights.cols() != x.rows()
        || distances.rows() != s
        || distances.cols() != s
        || !opts.initial_neighbor.is_finite()
        || opts.initial_neighbor < 1.
        || opts.epochs > 1_000_000
        || weights
            .data()
            .iter()
            .chain(x.data())
            .chain(distances.data())
            .any(|v| !v.is_finite())
    {
        return Err("SOM: invalid weights, inputs, distances or training parameters".into());
    }
    let bytes = weights
        .len()
        .saturating_mul(24)
        .saturating_add(x.cols().saturating_mul(16))
        .saturating_add((opts.epochs + 1).saturating_mul(512));
    if bytes > opts.memory_bytes {
        return Err("SOM: training workspace memory limit exceeded".into());
    }
    let mut w = weights.clone();
    let started = Instant::now();
    let mut record = SomRecord {
        epochs: Vec::with_capacity(opts.epochs + 1),
        stop: "Maximum epoch reached.".into(),
    };
    for epoch in 0..=opts.epochs {
        crate::dsp::check_cancel(opts.cancel)?;
        let mut ids = Vec::with_capacity(x.cols());
        let mut error = 0.;
        for c in 0..x.cols() {
            crate::dsp::check_cancel(opts.cancel)?;
            let mut best = 0;
            let mut score = f64::INFINITY;
            for i in 0..s {
                let distance = (0..x.rows())
                    .map(|r| (x.data()[r + c * x.rows()] - w.data()[i + r * s]).powi(2))
                    .sum::<f64>();
                if distance < score {
                    score = distance;
                    best = i;
                }
            }
            ids.push(best);
            error += score;
        }
        let radius = 1.
            + (opts.initial_neighbor - 1.)
                * if opts.cover_steps == 0 {
                    0.
                } else {
                    (1. - epoch as f64 / opts.cover_steps as f64).max(0.)
                };
        let report = EpochReport {
            epoch,
            time: started.elapsed().as_secs_f64(),
            perf: error / x.cols() as f64,
            vperf: f64::NAN,
            tperf: f64::NAN,
            gradient: 0.,
            val_fail: 0,
            extra: BTreeMap::from([("radius".into(), radius)]),
        };
        record.epochs.push(report.clone());
        if progress(report).is_break() {
            record.stop = "User stop.".into();
            break;
        }
        if epoch == opts.epochs {
            break;
        }
        if started.elapsed() >= opts.time_limit {
            record.stop = "Maximum time elapsed.".into();
            break;
        }
        let mut values = w.data().to_vec();
        for i in 0..s {
            crate::dsp::check_cancel(opts.cancel)?;
            let mut count = 0.;
            let mut sum = vec![0.; x.rows()];
            for (c, &winner) in ids.iter().enumerate() {
                if distances.data()[i + winner * s] <= radius {
                    count += 1.;
                    for (r, v) in sum.iter_mut().enumerate() {
                        *v += x.data()[r + c * x.rows()];
                    }
                }
            }
            if count > 0. {
                for r in 0..x.rows() {
                    values[i + r * s] = sum[r] / count;
                }
            }
        }
        if values.iter().any(|x| !x.is_finite()) {
            return Err("SOM: centroid overflow".into());
        }
        w = Matrix::from_column_vec(s, x.rows(), values)?;
    }
    Ok((w, record))
}
