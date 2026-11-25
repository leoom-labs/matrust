//! Pure feedforward network. Matrices share immutable column-major storage.
use super::{
    process::{self, Process},
    transfer,
};
use crate::{Matrix, Result};
use std::sync::atomic::AtomicBool;
pub const MAX_LAYERS: usize = 64;
pub const MAX_WEIGHTS: usize = 4_000_000;
#[derive(Clone, Debug)]
pub struct Layer {
    pub size: usize,
    pub transfer_fcn: String,
    pub init_fcn: String,
    pub name: String,
}
#[derive(Clone, Debug)]
pub struct Port {
    pub size: usize,
    pub range: Vec<(f64, f64)>,
    pub process_fcns: Vec<String>,
    pub process_params: Vec<(f64, f64)>,
    pub process_settings: Vec<Process>,
}
impl Port {
    fn new(pattern_output: bool) -> Self {
        let names = if pattern_output {
            vec!["removeconstantrows"]
        } else {
            vec!["removeconstantrows", "mapminmax"]
        };
        Self {
            size: 0,
            range: Vec::new(),
            process_params: names.iter().map(|n| process::defaults(n)).collect(),
            process_fcns: names.into_iter().map(str::to_owned).collect(),
            process_settings: Vec::new(),
        }
    }
    pub fn processed_size(&self) -> usize {
        self.process_settings
            .last()
            .map_or(self.size, |p| p.keep.len())
    }
}
#[derive(Clone, Debug)]
pub struct Network {
    pub layers: Vec<Layer>,
    pub input: Port,
    pub output: Port,
    pub bias_connect: Vec<bool>,
    pub input_connect: Vec<bool>,
    /// Column-major target-by-source connectivity; strictly lower triangular.
    pub layer_connect: Vec<bool>,
    pub iw: Vec<Matrix>,
    pub lw: Vec<Matrix>,
    pub b: Vec<Matrix>,
    pub configured: bool,
}
impl Network {
    pub fn new(hidden: &[usize], pattern: bool, cascade: bool) -> Result<Self> {
        if hidden.len() >= MAX_LAYERS || hidden.iter().any(|&n| n == 0 || n > MAX_WEIGHTS) {
            return Err("network: invalid hiddenSizes or layer limit exceeded".into());
        }
        let n = hidden.len() + 1;
        let mut layers: Vec<_> = hidden
            .iter()
            .enumerate()
            .map(|(i, &size)| Layer {
                size,
                transfer_fcn: "tansig".into(),
                init_fcn: "initnw".into(),
                name: format!("Hidden {}", i + 1),
            })
            .collect();
        layers.push(Layer {
            size: 0,
            transfer_fcn: if pattern { "softmax" } else { "purelin" }.into(),
            init_fcn: "initnw".into(),
            name: "Output".into(),
        });
        let empty = Matrix::filled(0, 0, 0.)?;
        let mut net = Self {
            layers,
            input: Port::new(false),
            output: Port::new(pattern),
            bias_connect: vec![true; n],
            input_connect: (0..n).map(|i| cascade || i == 0).collect(),
            layer_connect: vec![false; n * n],
            iw: vec![empty.clone(); n],
            lw: vec![empty.clone(); n * n],
            b: vec![empty; n],
            configured: false,
        };
        for i in 0..n {
            for j in 0..i {
                net.layer_connect[i + j * n] = cascade || j + 1 == i;
            }
        }
        net.check_size()?;
        Ok(net)
    }
    pub fn check_size(&self) -> Result<usize> {
        let n = self.layers.len();
        let mut count = 0usize;
        for (i, l) in self.layers.iter().enumerate() {
            let mut inputs = usize::from(self.bias_connect[i]);
            if self.input_connect[i] {
                inputs = inputs.saturating_add(self.input.processed_size());
            }
            for j in 0..i {
                if self.layer_connect[i + j * n] {
                    inputs = inputs.saturating_add(self.layers[j].size);
                }
            }
            count = count.saturating_add(l.size.saturating_mul(inputs));
        }
        if count > MAX_WEIGHTS {
            return Err(format!("network: exceeds {MAX_WEIGHTS} weight elements"));
        }
        Ok(count)
    }
    /// Allocate connected matrices; preserve correctly shaped existing values.
    pub fn resize_weights(&mut self) -> Result<()> {
        self.check_size()?;
        let n = self.layers.len();
        for i in 0..n {
            let s = self.layers[i].size;
            resize(
                &mut self.b[i],
                if self.bias_connect[i] { s } else { 0 },
                usize::from(self.bias_connect[i]),
            )?;
            resize(
                &mut self.iw[i],
                if self.input_connect[i] { s } else { 0 },
                if self.input_connect[i] {
                    self.input.processed_size()
                } else {
                    0
                },
            )?;
            for j in 0..n {
                let on = self.layer_connect[i + j * n];
                resize(
                    &mut self.lw[i + j * n],
                    if on { s } else { 0 },
                    if on { self.layers[j].size } else { 0 },
                )?;
            }
        }
        Ok(())
    }
    pub fn configure(&mut self, x: &Matrix, t: &Matrix, cancel: &AtomicBool) -> Result<()> {
        if x.cols() != t.cols() || x.cols() == 0 {
            return Err(
                "configure: input and target sample counts must match and be nonempty".into(),
            );
        }
        self.input.size = x.rows();
        self.output.size = t.rows();
        self.input.range = data_ranges(x);
        self.output.range = data_ranges(t);
        let (settings, _) = process::fit_chain(
            &self.input.process_fcns,
            &self.input.process_params,
            x,
            cancel,
        )?;
        self.input.process_settings = settings;
        let (settings, y) = process::fit_chain(
            &self.output.process_fcns,
            &self.output.process_params,
            t,
            cancel,
        )?;
        self.output.process_settings = settings;
        self.layers.last_mut().unwrap().size = y.rows();
        self.resize_weights()?;
        self.configured = true;
        Ok(())
    }
    /// The caller supplies the random stream. Engine glue passes rand's Generator.
    pub fn init(&mut self, mut uniform: impl FnMut() -> f64, cancel: &AtomicBool) -> Result<()> {
        if !self.configured {
            return Err("init: network is not configured; use configure(net,x,t)".into());
        }
        let n = self.layers.len();
        for i in 0..n {
            crate::dsp::check_cancel(cancel)?;
            let s = self.layers[i].size;
            let mut cols = self.iw[i].cols();
            for j in 0..i {
                cols += self.lw[i + j * n].cols();
            }
            if s == 0 {
                continue;
            }
            let nw = self.layers[i].init_fcn == "initnw"
                && self.bias_connect[i]
                && cols > 0
                && matches!(
                    self.layers[i].transfer_fcn.as_str(),
                    "tansig" | "logsig" | "satlin"
                );
            let beta = 0.7 * (s as f64).powf(1. / cols.max(1) as f64);
            let mut w = vec![0.; s * cols];
            for (k, v) in w.iter_mut().enumerate() {
                if k.is_multiple_of(4096) {
                    crate::dsp::check_cancel(cancel)?;
                }
                *v = 2. * uniform() - 1.;
            }
            let mut b = vec![0.; s];
            if nw {
                // Nguyen & Widrow (IJCNN 1990), normalized random directions,
                // beta=0.7*S^(1/R). Scale the normalized input space to the
                // transfer's active interval and each source's output range.
                let (lo, hi) = match self.layers[i].transfer_fcn.as_str() {
                    "logsig" => (-4., 4.),
                    "satlin" => (0., 1.),
                    _ => (-2., 2.),
                };
                for r in 0..s {
                    let norm = (0..cols)
                        .map(|c| w[c * s + r].powi(2))
                        .sum::<f64>()
                        .sqrt()
                        .max(f64::MIN_POSITIVE);
                    b[r] = if s == 1 {
                        0.
                    } else {
                        beta * (-1. + 2. * r as f64 / (s - 1) as f64) * w[r].signum()
                    };
                    b[r] = b[r] * (hi - lo) / 2. + (hi + lo) / 2.;
                    for c in 0..cols {
                        w[c * s + r] *= beta / norm * (hi - lo) / 2.;
                    }
                }
                let mut ranges = Vec::with_capacity(cols);
                if self.input_connect[i] {
                    let input_ranges = input_ranges(&self.input, cancel)?;
                    ranges.extend(input_ranges);
                }
                for j in 0..i {
                    if self.layer_connect[i + j * n] {
                        let range = match self.layers[j].transfer_fcn.as_str() {
                            "logsig" | "satlin" | "hardlim" | "radbas" | "softmax" => (0., 1.),
                            _ => (-1., 1.),
                        };
                        ranges.extend(std::iter::repeat_n(range, self.layers[j].size));
                    }
                }
                for (c, (lo, hi)) in ranges.into_iter().enumerate() {
                    let width = if hi > lo { hi - lo } else { 2. };
                    for r in 0..s {
                        w[c * s + r] *= 2. / width;
                        b[r] -= w[c * s + r] * (lo + hi) / 2.;
                    }
                }
            } else {
                for v in &mut b {
                    *v = 2. * uniform() - 1.;
                }
            }
            let mut offset = 0;
            if self.input_connect[i] {
                let len = self.iw[i].len();
                self.iw[i] =
                    Matrix::from_column_slice(s, self.iw[i].cols(), &w[offset..offset + len])?;
                offset += len;
            }
            for j in 0..i {
                let idx = i + j * n;
                if self.layer_connect[idx] {
                    let len = self.lw[idx].len();
                    self.lw[idx] = Matrix::from_column_slice(
                        s,
                        self.lw[idx].cols(),
                        &w[offset..offset + len],
                    )?;
                    offset += len;
                }
            }
            if self.bias_connect[i] {
                self.b[i] = Matrix::from_column_vec(s, 1, b)?;
            }
        }
        Ok(())
    }
    pub fn num_weight_elements(&self) -> usize {
        self.b
            .iter()
            .chain(&self.iw)
            .chain(&self.lw)
            .map(Matrix::len)
            .sum()
    }
    /// Per layer: bias, input weights, then layer weights by source; each (:).
    pub fn getwb(&self) -> Vec<f64> {
        let n = self.layers.len();
        let mut v = Vec::with_capacity(self.num_weight_elements());
        for i in 0..n {
            v.extend_from_slice(self.b[i].data());
            v.extend_from_slice(self.iw[i].data());
            for j in 0..n {
                v.extend_from_slice(self.lw[i + j * n].data());
            }
        }
        v
    }
    pub fn setwb(&mut self, v: &[f64]) -> Result<()> {
        if v.len() != self.num_weight_elements() || v.iter().any(|x| !x.is_finite()) {
            return Err("setwb: finite vector must match net.numWeightElements".into());
        }
        let n = self.layers.len();
        let mut offset = 0;
        for i in 0..n {
            replace(&mut self.b[i], v, &mut offset)?;
            replace(&mut self.iw[i], v, &mut offset)?;
            for j in 0..n {
                replace(&mut self.lw[i + j * n], v, &mut offset)?;
            }
        }
        Ok(())
    }
    pub fn forward(&self, x: &Matrix) -> Result<Matrix> {
        self.forward_cancel(x, &AtomicBool::new(false))
    }
    pub fn forward_cancel(&self, x: &Matrix, cancel: &AtomicBool) -> Result<Matrix> {
        let a = self.activations(x, cancel)?;
        process::reverse_chain(&self.output.process_settings, a.last().unwrap(), cancel)
    }
    /// Activations in layer order, after input processing and before output reversal.
    pub fn activations(&self, x: &Matrix, cancel: &AtomicBool) -> Result<Vec<Matrix>> {
        if !self.configured {
            return Err("sim: network is not configured; use configure(net,x,t)".into());
        }
        if x.rows() != self.input.size {
            return Err("sim: input size does not match net.inputs{1}.size".into());
        }
        let x = process::apply_chain(&self.input.process_settings, x, cancel)?;
        let n = self.layers.len();
        let mut a: Vec<Matrix> = Vec::with_capacity(n);
        for i in 0..n {
            crate::dsp::check_cancel(cancel)?;
            let s = self.layers[i].size;
            let mut v = vec![0.; Matrix::checked_len(s, x.cols())?];
            if self.bias_connect[i] {
                for col in v.chunks_mut(s.max(1)) {
                    col.copy_from_slice(self.b[i].data());
                }
            }
            if self.input_connect[i] {
                add_product(&mut v, &self.iw[i], &x, cancel)?;
            }
            for (j, prev) in a.iter().enumerate() {
                if self.layer_connect[i + j * n] {
                    add_product(&mut v, &self.lw[i + j * n], prev, cancel)?;
                }
            }
            a.push(transfer::apply(
                &self.layers[i].transfer_fcn,
                &Matrix::from_column_vec(s, x.cols(), v)?,
                cancel,
            )?);
        }
        Ok(a)
    }
}
fn resize(m: &mut Matrix, r: usize, c: usize) -> Result<()> {
    if m.rows() != r || m.cols() != c {
        *m = Matrix::filled(r, c, 0.)?;
    }
    Ok(())
}
fn replace(m: &mut Matrix, v: &[f64], offset: &mut usize) -> Result<()> {
    let len = m.len();
    *m = Matrix::from_column_slice(m.rows(), m.cols(), &v[*offset..*offset + len])?;
    *offset += len;
    Ok(())
}
fn add_product(v: &mut [f64], w: &Matrix, x: &Matrix, cancel: &AtomicBool) -> Result<()> {
    // Bound cancellation latency by splitting samples; no private thread pools.
    let rows = w.rows();
    for start in (0..x.cols()).step_by(128) {
        crate::dsp::check_cancel(cancel)?;
        let end = (start + 128).min(x.cols());
        let chunk = Matrix::from_column_slice(
            x.rows(),
            end - start,
            &x.data()[start * x.rows()..end * x.rows()],
        )?;
        let y = w.mul(&chunk)?;
        for (out, &p) in v[start * rows..end * rows].iter_mut().zip(y.data()) {
            *out += p;
        }
    }
    Ok(())
}
fn data_ranges(x: &Matrix) -> Vec<(f64, f64)> {
    (0..x.rows())
        .map(|r| {
            let mut lo = f64::INFINITY;
            let mut hi = f64::NEG_INFINITY;
            for c in 0..x.cols() {
                let v = x.data()[c * x.rows() + r];
                if v.is_finite() {
                    lo = lo.min(v);
                    hi = hi.max(v);
                }
            }
            (lo, hi)
        })
        .collect()
}
fn input_ranges(p: &Port, cancel: &AtomicBool) -> Result<Vec<(f64, f64)>> {
    if p.range.len() != p.size {
        return Ok(vec![(-1., 1.); p.processed_size()]);
    }
    let data = p
        .range
        .iter()
        .map(|&(lo, _)| lo)
        .chain(p.range.iter().map(|&(_, hi)| hi))
        .collect();
    let x = Matrix::from_column_vec(p.size, 2, data)?;
    let y = process::apply_chain(&p.process_settings, &x, cancel)?;
    Ok((0..y.rows())
        .map(|r| (y.data()[r], y.data()[r + y.rows()]))
        .collect())
}
