//! Analytic reverse-mode derivatives, including processing and softmax coupling.
use super::{TrainOptions, dot};
use crate::engine::nnet::{network::Network, process, transfer};
use crate::{Matrix, Result};
use std::sync::atomic::AtomicBool;

pub struct Evaluation {
    pub perf: f64,
    pub gradient: Vec<f64>,
    /// Half the squared-error Gauss-Newton Hessian, including regularization.
    pub curvature: Option<Matrix>,
}
pub struct Objective<'a> {
    pub kind: &'a str,
    pub started: std::time::Instant,
    time_limit: f64,
    pub cancel: &'a AtomicBool,
    pub indices: &'a [usize],
    pub string_parameters: &'a std::collections::BTreeMap<String, String>,
    x: Matrix,
    t: &'a Matrix,
    ew: Option<&'a Matrix>,
    scales: Vec<f64>,
    reg: f64,
}
struct Cache {
    a: Vec<Matrix>,
    derivative: Vec<Matrix>,
    y: Matrix,
}
impl<'a> Objective<'a> {
    pub fn new(
        net: &Network,
        x: &Matrix,
        t: &'a Matrix,
        indices: &'a [usize],
        opts: &'a TrainOptions<'_>,
    ) -> Result<Self> {
        if !matches!(
            opts.perform_fcn.as_str(),
            "mse" | "sse" | "mae" | "sae" | "crossentropy"
        ) {
            return Err("train: unsupported performFcn".into());
        }
        if !opts.regularization.is_finite() || !(0. ..=1.).contains(&opts.regularization) {
            return Err("train: regularization must be in [0,1]".into());
        }
        if !matches!(opts.normalization.as_str(), "none" | "standard" | "percent")
            || (opts.perform_fcn == "crossentropy" && opts.normalization != "none")
        {
            return Err("train: unsupported performance normalization".into());
        }
        if let Some(w) = &opts.error_weights
            && (w.is_nd()
                || ![1, t.rows()].contains(&w.rows())
                || ![1, t.cols()].contains(&w.cols())
                || w.data().iter().any(|v| !v.is_finite() || *v < 0.))
        {
            return Err("train: EW must be finite nonnegative scalar, output column, sample row or target-sized matrix".into());
        }
        if opts.perform_fcn == "crossentropy" && t.data().iter().any(|v| !(0. ..=1.).contains(v)) {
            return Err("train: crossentropy targets must be in [0,1]".into());
        }
        let mut scales = vec![1.; t.rows()];
        if opts.normalization != "none" {
            for (r, scale) in scales.iter_mut().enumerate() {
                let (lo, hi) = net.output.range.get(r).copied().unwrap_or((0., 0.));
                if hi > lo {
                    *scale = if opts.normalization == "standard" {
                        2.
                    } else {
                        1.
                    } / (hi - lo);
                }
            }
        }
        Ok(Self {
            kind: &opts.perform_fcn,
            started: std::time::Instant::now(),
            time_limit: opts
                .parameters
                .get("time")
                .copied()
                .unwrap_or(f64::INFINITY),
            cancel: &opts.cancel,
            indices,
            string_parameters: &opts.string_parameters,
            x: process::apply_chain(&net.input.process_settings, x, &opts.cancel)?,
            t,
            ew: opts.error_weights.as_ref(),
            scales,
            reg: opts.regularization,
        })
    }
    pub fn time_expired(&self) -> bool {
        self.started.elapsed().as_secs_f64() >= self.time_limit
    }
    /// Data objective without the configured weight penalty, for Bayesian trainers.
    pub fn data_evaluation(&self, net: &Network, curvature: bool) -> Result<Evaluation> {
        let data = Self {
            reg: 0.,
            x: self.x.clone(),
            scales: self.scales.clone(),
            ..*self
        };
        data.evaluate(net, curvature)
    }
    fn mean(&self) -> bool {
        matches!(self.kind, "mse" | "mae" | "crossentropy")
    }
    fn weight(&self, r: usize, q: usize) -> f64 {
        self.ew.map_or(1., |w| {
            w.data()
                [if w.cols() == 1 { 0 } else { q * w.rows() } + if w.rows() == 1 { 0 } else { r }]
        })
    }
    fn cache(&self, net: &Network) -> Result<Cache> {
        let n = net.layers.len();
        let mut a: Vec<Matrix> = Vec::with_capacity(n);
        let mut derivative = Vec::with_capacity(n);
        for i in 0..n {
            crate::dsp::check_cancel(self.cancel)?;
            let s = net.layers[i].size;
            let mut z = vec![0.; s * self.x.cols()];
            if net.bias_connect[i] {
                for col in z.chunks_mut(s.max(1)) {
                    col.copy_from_slice(net.b[i].data());
                }
            }
            if net.input_connect[i] {
                add_product(&mut z, &net.iw[i], &self.x, self.cancel)?;
            }
            for (j, prev) in a.iter().enumerate() {
                if net.layer_connect[i + j * n] {
                    add_product(&mut z, &net.lw[i + j * n], prev, self.cancel)?;
                }
            }
            let z = Matrix::from_column_vec(s, self.x.cols(), z)?;
            let y = transfer::apply(&net.layers[i].transfer_fcn, &z, self.cancel)?;
            // Softmax is differentiated as a vector-Jacobian product, not S^2 storage.
            derivative.push(if net.layers[i].transfer_fcn == "softmax" {
                Matrix::filled(0, 0, 0.)?
            } else {
                transfer::derivative(&net.layers[i].transfer_fcn, &z, &y, self.cancel)?
            });
            a.push(y);
        }
        let y =
            process::reverse_chain(&net.output.process_settings, a.last().unwrap(), self.cancel)?;
        if y.data().iter().any(|v| !v.is_finite()) {
            return Err("train: nonfinite network output".into());
        }
        if self.kind == "crossentropy" && y.data().iter().any(|v| !(0. ..=1.).contains(v)) {
            return Err("train: crossentropy outputs must be in [0,1]".into());
        }
        Ok(Cache { a, derivative, y })
    }
    fn loss(&self, t: f64, y: f64, r: usize) -> (f64, f64) {
        if self.kind == "crossentropy" {
            let eps = f64::EPSILON;
            let loss = -t * y.max(eps).ln();
            let grad = if y > eps { -t / y } else { 0. };
            if self.t.rows() == 1 {
                (
                    loss - (1. - t) * (1. - y).max(eps).ln(),
                    grad + if 1. - y > eps {
                        (1. - t) / (1. - y)
                    } else {
                        0.
                    },
                )
            } else {
                (loss, grad)
            }
        } else {
            let e = (y - t) * self.scales[r];
            if matches!(self.kind, "mse" | "sse") {
                (e * e, 2. * e * self.scales[r])
            } else {
                (
                    e.abs(),
                    if e == 0. {
                        0.
                    } else {
                        e.signum() * self.scales[r]
                    },
                )
            }
        }
    }
    fn penalty(&self, w: &[f64]) -> f64 {
        let total = if matches!(self.kind, "mae" | "sae") {
            w.iter().map(|v| v.abs()).sum()
        } else {
            dot(w, w)
        };
        self.reg * total
            / if self.mean() {
                w.len().max(1) as f64
            } else {
                1.
            }
    }
    pub fn performance(&self, net: &Network, indices: &[usize]) -> Result<f64> {
        if indices.is_empty() {
            return Ok(f64::NAN);
        }
        let cache = self.cache(net)?;
        let mut sum = 0.;
        for &q in indices {
            crate::dsp::check_cancel(self.cancel)?;
            for r in 0..self.t.rows() {
                let k = q * self.t.rows() + r;
                sum += self.weight(r, q) * self.loss(self.t.data()[k], cache.y.data()[k], r).0;
            }
        }
        let div = if self.mean() {
            (indices.len() * self.t.rows()) as f64
        } else {
            1.
        };
        Ok((1. - self.reg) * sum / div + self.penalty(&net.getwb()))
    }
    pub fn evaluate(&self, net: &Network, curvature: bool) -> Result<Evaluation> {
        crate::dsp::check_cancel(self.cancel)?;
        if curvature && !matches!(self.kind, "mse" | "sse") {
            return Err("trainlm: requires mse or sse performance".into());
        }
        let cache = self.cache(net)?;
        let w = net.getwb();
        let p = w.len();
        let mut gradient = vec![0.; p];
        let mut h = if curvature {
            vec![0.; Matrix::checked_len(p, p)?]
        } else {
            vec![]
        };
        let mut perf = 0.;
        let scale = (1. - self.reg)
            / if self.mean() {
                (self.indices.len() * self.t.rows()) as f64
            } else {
                1.
            };
        for &q in self.indices {
            crate::dsp::check_cancel(self.cancel)?;
            let mut seed = vec![0.; self.t.rows()];
            for r in 0..self.t.rows() {
                let k = q * self.t.rows() + r;
                let (loss, d) = self.loss(self.t.data()[k], cache.y.data()[k], r);
                let ew = self.weight(r, q);
                perf += scale * ew * loss;
                seed[r] = scale * ew * d;
                if curvature && ew > 0. {
                    let mut unit = vec![0.; self.t.rows()];
                    unit[r] = self.scales[r] * (scale * ew).sqrt();
                    let j = self.backprop(net, &cache, q, unit)?;
                    for col in 0..p {
                        if col.is_multiple_of(64) {
                            crate::dsp::check_cancel(self.cancel)?;
                        }
                        for row in 0..=col {
                            h[col * p + row] += j[col] * j[row];
                        }
                    }
                }
            }
            let g = self.backprop(net, &cache, q, seed)?;
            for (a, b) in gradient.iter_mut().zip(g) {
                *a += b;
            }
        }
        let reg = self.reg / if self.mean() { p.max(1) as f64 } else { 1. };
        perf += self.penalty(&w);
        for i in 0..p {
            gradient[i] += if matches!(self.kind, "mae" | "sae") {
                reg * if w[i] == 0. { 0. } else { w[i].signum() }
            } else {
                2. * reg * w[i]
            };
            if curvature {
                h[i * p + i] += reg;
                for j in 0..i {
                    h[j * p + i] = h[i * p + j];
                }
            }
        }
        if !perf.is_finite() || gradient.iter().any(|v| !v.is_finite()) {
            return Err("train: nonfinite performance or gradient".into());
        }
        Ok(Evaluation {
            perf,
            gradient,
            curvature: if curvature {
                Some(Matrix::from_column_vec(p, p, h)?)
            } else {
                None
            },
        })
    }
    fn backprop(
        &self,
        net: &Network,
        cache: &Cache,
        q: usize,
        mut seed: Vec<f64>,
    ) -> Result<Vec<f64>> {
        // VJP of output reversal: traverse fitted processing in forward order.
        for proc in &net.output.process_settings {
            seed = proc
                .keep
                .iter()
                .map(|&r| {
                    seed[r]
                        / if proc.name == "removeconstantrows" || proc.gain[r] == 0. {
                            1.
                        } else {
                            proc.gain[r]
                        }
                })
                .collect();
        }
        let n = net.layers.len();
        let mut adj: Vec<Vec<f64>> = net.layers.iter().map(|l| vec![0.; l.size]).collect();
        adj[n - 1] = seed;
        let mut offsets = vec![0; n];
        let mut offset = 0;
        for (i, o) in offsets.iter_mut().enumerate() {
            *o = offset;
            offset += net.b[i].len() + net.iw[i].len();
            for j in 0..n {
                offset += net.lw[i + j * n].len();
            }
        }
        let mut g = vec![0.; offset];
        for i in (0..n).rev() {
            crate::dsp::check_cancel(self.cancel)?;
            let s = net.layers[i].size;
            let a = &cache.a[i].data()[q * s..(q + 1) * s];
            if net.layers[i].transfer_fcn == "softmax" {
                let sum = dot(a, &adj[i]);
                for (r, &v) in a.iter().enumerate() {
                    adj[i][r] = v * (adj[i][r] - sum);
                }
            } else {
                for (value, derivative) in adj[i]
                    .iter_mut()
                    .zip(&cache.derivative[i].data()[q * s..(q + 1) * s])
                {
                    *value *= derivative;
                }
            }
            let mut k = offsets[i];
            if net.bias_connect[i] {
                g[k..k + s].copy_from_slice(&adj[i]);
                k += s;
            }
            if net.input_connect[i] {
                for c in 0..self.x.rows() {
                    for &value in &adj[i] {
                        g[k] = value * self.x.data()[q * self.x.rows() + c];
                        k += 1;
                    }
                }
            }
            for j in 0..i {
                if net.layer_connect[i + j * n] {
                    for c in 0..net.layers[j].size {
                        for r in 0..s {
                            g[k] = adj[i][r] * cache.a[j].data()[q * net.layers[j].size + c];
                            k += 1;
                            adj[j][c] += net.lw[i + j * n].data()[c * s + r] * adj[i][r];
                        }
                    }
                }
            }
        }
        Ok(g)
    }
}
fn add_product(out: &mut [f64], w: &Matrix, x: &Matrix, cancel: &AtomicBool) -> Result<()> {
    for start in (0..x.cols()).step_by(128) {
        crate::dsp::check_cancel(cancel)?;
        let end = (start + 128).min(x.cols());
        let chunk = Matrix::from_column_slice(
            x.rows(),
            end - start,
            &x.data()[start * x.rows()..end * x.rows()],
        )?;
        let y = w.mul(&chunk)?;
        for (a, b) in out[start * w.rows()..end * w.rows()]
            .iter_mut()
            .zip(y.data())
        {
            *a += b;
        }
    }
    Ok(())
}
