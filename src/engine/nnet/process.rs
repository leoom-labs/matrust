//! Fitted row processing; settings are immutable and independent of Engine.
use crate::{Matrix, Result};
use std::sync::atomic::AtomicBool;
pub const NAMES: &[&str] = &["mapminmax", "mapstd", "removeconstantrows"];
#[derive(Clone, Debug)]
pub struct Process {
    pub name: String,
    pub xrows: usize,
    pub xcols: usize,
    pub offset: Vec<f64>,
    /// Zero marks a constant row that passes through unchanged (derivative 1).
    pub gain: Vec<f64>,
    pub xmin: Vec<f64>,
    pub xmax: Vec<f64>,
    pub keep: Vec<usize>,
    pub yoffset: f64,
    pub yscale: f64,
}
impl Process {
    pub fn fit(name: &str, x: &Matrix, a: f64, b: f64, cancel: &AtomicBool) -> Result<Self> {
        if !NAMES.contains(&name) {
            return Err(format!("Unsupported processFcn '{name}'"));
        }
        if x.cols() == 0
            || x.data()
                .iter()
                .any(|v| v.is_infinite() || (name != "removeconstantrows" && v.is_nan()))
        {
            return Err(format!("{name}: finite, nonempty samples required"));
        }
        if !a.is_finite()
            || !b.is_finite()
            || (name == "mapminmax" && a >= b)
            || (name == "mapstd" && b <= 0.)
            || (name == "removeconstantrows" && a < 0.)
        {
            return Err(format!("{name}: invalid processing parameters"));
        }
        let mut p = Self {
            name: name.into(),
            xrows: x.rows(),
            xcols: x.cols(),
            offset: vec![0.; x.rows()],
            gain: vec![1.; x.rows()],
            xmin: vec![0.; x.rows()],
            xmax: vec![0.; x.rows()],
            keep: Vec::new(),
            yoffset: a,
            yscale: b,
        };
        for r in 0..x.rows() {
            crate::dsp::check_cancel(cancel)?;
            let mut row = Vec::with_capacity(x.cols());
            for col in 0..x.cols() {
                if col.is_multiple_of(4096) {
                    crate::dsp::check_cancel(cancel)?;
                }
                let value = x.data()[col * x.rows() + r];
                if !value.is_nan() {
                    row.push(value);
                }
            }
            if row.is_empty() {
                return Err(format!("{name}: an all-missing row is not supported"));
            }
            let min = row.iter().copied().fold(f64::INFINITY, f64::min);
            let max = row.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            p.xmin[r] = min;
            p.xmax[r] = max;
            p.offset[r] = min;
            if name == "removeconstantrows" {
                if max - min > a {
                    p.keep.push(r);
                }
            } else {
                p.keep.push(r);
                if name == "mapstd" {
                    let mean = row.iter().sum::<f64>() / row.len() as f64;
                    let std = (row.iter().map(|x| (x - mean).powi(2)).sum::<f64>()
                        / (row.len().saturating_sub(1).max(1)) as f64)
                        .sqrt();
                    p.offset[r] = mean;
                    p.gain[r] = if std > 0. { b / std } else { 0. };
                } else {
                    p.gain[r] = if max > min { (b - a) / (max - min) } else { 0. };
                }
            }
        }
        Ok(p)
    }
    pub fn apply(&self, x: &Matrix, cancel: &AtomicBool) -> Result<Matrix> {
        if x.rows() != self.xrows {
            return Err(format!(
                "{}: input row count does not match settings",
                self.name
            ));
        }
        let nr = self.keep.len();
        let mut v = vec![0.; Matrix::checked_len(nr, x.cols())?];
        for c in 0..x.cols() {
            crate::dsp::check_cancel(cancel)?;
            for (i, &r) in self.keep.iter().enumerate() {
                let x = x.data()[c * self.xrows + r];
                v[c * nr + i] = if self.name == "removeconstantrows" || self.gain[r] == 0. {
                    x
                } else {
                    (x - self.offset[r]) * self.gain[r] + self.yoffset
                };
            }
        }
        Matrix::from_column_vec(nr, x.cols(), v)
    }
    pub fn reverse(&self, y: &Matrix, cancel: &AtomicBool) -> Result<Matrix> {
        if y.rows() != self.keep.len() {
            return Err(format!(
                "{}: output row count does not match settings",
                self.name
            ));
        }
        let mut v = vec![0.; Matrix::checked_len(self.xrows, y.cols())?];
        for c in 0..y.cols() {
            crate::dsp::check_cancel(cancel)?;
            v[c * self.xrows..(c + 1) * self.xrows].copy_from_slice(&self.offset);
            for (i, &r) in self.keep.iter().enumerate() {
                let y = y.data()[c * self.keep.len() + i];
                v[c * self.xrows + r] = if self.name == "removeconstantrows" || self.gain[r] == 0. {
                    y
                } else {
                    (y - self.yoffset) / self.gain[r] + self.offset[r]
                };
            }
        }
        Matrix::from_column_vec(self.xrows, y.cols(), v)
    }
}
pub fn fit_chain(
    names: &[String],
    params: &[(f64, f64)],
    x: &Matrix,
    cancel: &AtomicBool,
) -> Result<(Vec<Process>, Matrix)> {
    let mut y = x.clone();
    let mut settings = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let (a, b) = params.get(i).copied().unwrap_or_else(|| defaults(name));
        let p = Process::fit(name, &y, a, b, cancel)?;
        y = p.apply(&y, cancel)?;
        settings.push(p);
    }
    Ok((settings, y))
}
pub fn defaults(name: &str) -> (f64, f64) {
    match name {
        "mapminmax" => (-1., 1.),
        "mapstd" => (0., 1.),
        _ => (0., 0.),
    }
}
pub fn apply_chain(settings: &[Process], x: &Matrix, cancel: &AtomicBool) -> Result<Matrix> {
    let mut y = x.clone();
    for p in settings {
        y = p.apply(&y, cancel)?;
    }
    Ok(y)
}
pub fn reverse_chain(settings: &[Process], y: &Matrix, cancel: &AtomicBool) -> Result<Matrix> {
    let mut x = y.clone();
    for p in settings.iter().rev() {
        x = p.reverse(&x, cancel)?;
    }
    Ok(x)
}
