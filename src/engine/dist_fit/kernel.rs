//! Kernel (nonparametric) distribution objects: fitdist(x, 'Kernel').
//!
//! The kernels are scaled to unit variance (normal; box on |z| <= sqrt 3;
//! triangle on |z| <= sqrt 6; Epanechnikov on |z| <= sqrt 5) so one
//! bandwidth gives comparable smoothing. The default bandwidth is
//! Silverman's normal-reference rule sigma (4/(3n))^(1/5) (B. W. Silverman,
//! "Density Estimation for Statistics and Data Analysis", 1986, eq. 3.28)
//! with the robust sigma = MAD/0.6745. Bounded supports are mapped to the
//! real line by log(x) or log((x-L)/(U-x)), as the public ksdensity page
//! describes, and the density is transformed back with the Jacobian.
use super::super::{Engine, Value};
use super::FitInfo;
use super::sample::Sample;
use crate::{Array, Matrix, Result};
use std::f64::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::engine) enum Smoother {
    Normal,
    Box,
    Triangle,
    Epanechnikov,
}

impl Smoother {
    pub fn parse(who: &str, name: &str) -> Result<Self> {
        Ok(match name.to_ascii_lowercase().as_str() {
            "normal" => Self::Normal,
            "box" => Self::Box,
            "triangle" => Self::Triangle,
            "epanechnikov" => Self::Epanechnikov,
            _ => {
                return Err(format!(
                    "{who}: unknown kernel '{name}'; expected normal, box, triangle or epanechnikov"
                ));
            }
        })
    }
    fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Box => "box",
            Self::Triangle => "triangle",
            Self::Epanechnikov => "epanechnikov",
        }
    }
    /// Half-width of the support (infinite for the normal kernel, where the
    /// terms beyond 40 bandwidths underflow).
    fn reach(self) -> f64 {
        match self {
            Self::Normal => 40.,
            Self::Box => 3f64.sqrt(),
            Self::Triangle => 6f64.sqrt(),
            Self::Epanechnikov => 5f64.sqrt(),
        }
    }
    fn density(self, z: f64) -> f64 {
        let a = self.reach();
        match self {
            Self::Normal => (-0.5 * z * z).exp() / (2. * PI).sqrt(),
            _ if z.abs() > a => 0.,
            Self::Box => 1. / (2. * a),
            Self::Triangle => (1. - z.abs() / a) / a,
            Self::Epanechnikov => 0.75 * (1. - (z / a).powi(2)) / a,
        }
    }
    fn cdf(self, z: f64) -> f64 {
        let a = self.reach();
        if self == Self::Normal {
            return 0.5 * libm::erfc(-z / std::f64::consts::SQRT_2);
        }
        if z <= -a {
            return 0.;
        }
        if z >= a {
            return 1.;
        }
        let t = z / a;
        match self {
            Self::Box => (t + 1.) / 2.,
            Self::Triangle => {
                if t <= 0. {
                    (1. + t).powi(2) / 2.
                } else {
                    1. - (1. - t).powi(2) / 2.
                }
            }
            _ => 0.25 * (2. + 3. * t - t * t * t),
        }
    }
    fn draw(self, g: &mut super::super::random::Generator) -> f64 {
        match self {
            Self::Normal => g.next_normal(),
            Self::Box => (2. * g.next_f64() - 1.) * self.reach(),
            Self::Triangle => (g.next_f64() + g.next_f64() - 1.) * self.reach(),
            Self::Epanechnikov => {
                // Median of three uniforms on (-1, 1) by the rule of
                // Devroye (1986, p. 236).
                let u: Vec<f64> = (0..3).map(|_| 2. * g.next_f64() - 1.).collect();
                let t = if u[2].abs() >= u[1].abs() && u[2].abs() >= u[0].abs() {
                    u[1]
                } else {
                    u[2]
                };
                t * self.reach()
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::engine) struct KernelFit {
    pub kind: Smoother,
    pub bandwidth: f64,
    /// (lower, upper) support; infinite for unbounded sides.
    pub support: (f64, f64),
    pub input: FitInfo,
    /// Distinct transformed data, sorted, with weights normalised to sum 1
    /// (equal values merged) and their running sums.
    y: Vec<f64>,
    w: Vec<f64>,
    cum: Vec<f64>,
}

/// The fitdist kernel options.
pub(super) struct KernelOptions {
    pub kind: Smoother,
    pub bandwidth: Option<f64>,
    pub support: (f64, f64),
}

fn weighted_median(points: &[(f64, f64)]) -> f64 {
    // points sorted by value
    let total: f64 = points.iter().map(|p| p.1).sum();
    let mut acc = 0.;
    for (i, &(v, w)) in points.iter().enumerate() {
        acc += w;
        if acc > total / 2. * (1. + 1e-15) {
            return v;
        }
        if (acc - total / 2.).abs() <= 1e-15 * total {
            return match points.get(i + 1) {
                Some(next) => (v + next.0) / 2.,
                None => v,
            };
        }
    }
    points.last().map_or(f64::NAN, |p| p.0)
}

impl KernelFit {
    fn transform(&self, x: f64) -> f64 {
        let (lo, hi) = self.support;
        match (lo.is_finite(), hi.is_finite()) {
            (false, false) => x,
            (true, false) => (x - lo).ln(),
            _ => ((x - lo) / (hi - x)).ln(),
        }
    }
    fn untransform(&self, y: f64) -> f64 {
        let (lo, hi) = self.support;
        match (lo.is_finite(), hi.is_finite()) {
            (false, false) => y,
            (true, false) => lo + y.exp(),
            _ => {
                if y >= 0. {
                    lo + (hi - lo) / (1. + (-y).exp())
                } else {
                    let t = y.exp();
                    (lo + hi * t) / (1. + t)
                }
            }
        }
    }
    fn jacobian(&self, x: f64) -> f64 {
        let (lo, hi) = self.support;
        match (lo.is_finite(), hi.is_finite()) {
            (false, false) => 1.,
            (true, false) => 1. / (x - lo),
            _ => (hi - lo) / ((x - lo) * (hi - x)),
        }
    }
    fn inside(&self, x: f64) -> bool {
        x > self.support.0 && x < self.support.1
    }
    /// Index range of the points within `reach` bandwidths of y.
    fn window(&self, y: f64) -> (usize, usize) {
        let r = self.kind.reach() * self.bandwidth;
        let a = self.y.partition_point(|v| *v < y - r);
        let b = self.y.partition_point(|v| *v <= y + r);
        (a, b)
    }
    fn density_y(&self, y: f64) -> f64 {
        let (a, b) = self.window(y);
        let h = self.bandwidth;
        (a..b)
            .map(|i| self.w[i] * self.kind.density((y - self.y[i]) / h))
            .sum::<f64>()
            / h
    }
    fn cdf_y(&self, y: f64) -> f64 {
        let (a, b) = self.window(y);
        let below = if a == 0 { 0. } else { self.cum[a - 1] };
        let h = self.bandwidth;
        let inner: f64 = (a..b)
            .map(|i| self.w[i] * self.kind.cdf((y - self.y[i]) / h))
            .sum();
        (below + inner).clamp(0., 1.)
    }
    pub fn pdf(&self, x: f64) -> f64 {
        if x.is_nan() {
            return f64::NAN;
        }
        if !self.inside(x) {
            return 0.;
        }
        self.density_y(self.transform(x)) * self.jacobian(x)
    }
    pub fn cdf(&self, x: f64) -> f64 {
        if x.is_nan() {
            return f64::NAN;
        }
        if x <= self.support.0 {
            return 0.;
        }
        if x >= self.support.1 {
            return 1.;
        }
        if x.is_infinite() {
            return if x > 0. { 1. } else { 0. };
        }
        self.cdf_y(self.transform(x))
    }
    pub fn icdf(&self, p: f64) -> f64 {
        if p.is_nan() || !(0. ..=1.).contains(&p) {
            return f64::NAN;
        }
        let r = self.kind.reach() * self.bandwidth;
        let (first, last) = (self.y[0], self.y[self.y.len() - 1]);
        let unbounded_kernel = self.kind == Smoother::Normal;
        if p == 0. {
            return self.untransform(if unbounded_kernel {
                f64::NEG_INFINITY
            } else {
                first - r
            });
        }
        if p == 1. {
            return self.untransform(if unbounded_kernel {
                f64::INFINITY
            } else {
                last + r
            });
        }
        let (mut lo, mut hi) = (first - r, last + r);
        for _ in 0..300 {
            let mid = lo + (hi - lo) / 2.;
            if mid <= lo || mid >= hi {
                break;
            }
            if self.cdf_y(mid) < p {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        self.untransform(lo + (hi - lo) / 2.)
    }
    /// E[g(X)] by Gauss-Legendre quadrature of each kernel term.
    fn expect(&self, g: impl Fn(f64) -> f64) -> f64 {
        let (nodes, weights) = gauss_legendre(16);
        let a = if self.kind == Smoother::Normal {
            8.
        } else {
            self.kind.reach()
        };
        let pieces = if self.kind == Smoother::Normal { 4 } else { 2 };
        let width = 2. * a / pieces as f64;
        let mut total = 0.;
        for (yi, wi) in self.y.iter().zip(&self.w) {
            let mut part = 0.;
            for piece in 0..pieces {
                let left = -a + piece as f64 * width;
                for (t, c) in nodes.iter().zip(&weights) {
                    let z = left + (t + 1.) / 2. * width;
                    part += c * width / 2. * self.kind.density(z) * g(yi + self.bandwidth * z);
                }
            }
            total += wi * part;
        }
        total
    }
    pub fn moments(&self) -> (f64, f64) {
        let (lo, hi) = self.support;
        if !lo.is_finite() && !hi.is_finite() {
            let m: f64 = self.y.iter().zip(&self.w).map(|(y, w)| y * w).sum();
            let v: f64 = self
                .y
                .iter()
                .zip(&self.w)
                .map(|(y, w)| w * (y - m) * (y - m))
                .sum();
            return (m, v + self.bandwidth * self.bandwidth);
        }
        let m = self.expect(|y| self.untransform(y));
        let v = self.expect(|y| (self.untransform(y) - m).powi(2));
        (m, v)
    }
    /// Negative log-likelihood of a sample. Equal observations share one
    /// density evaluation (sorted, merged), so repeated values cost one
    /// kernel sum each; cancellable between evaluations.
    pub(in crate::engine::dist_fit) fn nll(&self, e: &Engine, s: &Sample) -> Result<f64> {
        let mut obs: Vec<(f64, f64)> = s.iter().map(|o| (o.0, o.2)).collect();
        obs.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut total = 0.;
        let mut i = 0;
        let mut evaluations = 0usize;
        while i < obs.len() {
            if evaluations.is_multiple_of(16) {
                e.check_cancel()?;
            }
            evaluations += 1;
            let x = obs[i].0;
            let mut freq = 0.;
            while i < obs.len() && obs[i].0.total_cmp(&x).is_eq() {
                freq += obs[i].1;
                i += 1;
            }
            total += freq * self.pdf(x).ln();
        }
        Ok(-total)
    }
    pub fn bytes(&self) -> usize {
        self.input.bytes() + self.y.len() * 24
    }
    fn support_text(&self) -> String {
        let (lo, hi) = self.support;
        match (lo.is_finite(), hi.is_finite()) {
            (false, false) => "unbounded".into(),
            (true, false) if lo == 0. => "positive".into(),
            _ => format!("[{}, {}]", super::short(lo, 6), super::short(hi, 6)),
        }
    }
    pub fn body(&self) -> String {
        let n: f64 = self.input.sample().weight();
        format!(
            "  Kernel distribution fitted to {} observation{}\n    Kernel = {}\n    Bandwidth = {}\n    Support = {}\n",
            super::short(n, 10),
            if n == 1. { "" } else { "s" },
            self.kind.name(),
            super::short(self.bandwidth, 6),
            self.support_text()
        )
    }
    pub fn property(&self, e: &Engine, name: &str) -> Result<Value> {
        match name {
            "Kernel" => Ok(Value::Text(self.kind.name().into())),
            "BandWidth" | "Bandwidth" => Ok(Value::Matrix(Matrix::scalar(self.bandwidth))),
            "Support" => {
                let (lo, hi) = self.support;
                super::super::optim_nl::onl_struct(
                    e,
                    vec![
                        (
                            "range",
                            Array::Numeric(Matrix::from_column_vec(1, 2, vec![lo, hi])?),
                        ),
                        (
                            "closedbound",
                            Array::Logical(crate::LogicalMatrix::from_column_vec(
                                1,
                                2,
                                vec![0, 0],
                                &e.cancel,
                            )?),
                        ),
                        (
                            "iscontinuous",
                            Array::Logical(crate::LogicalMatrix::scalar(true)),
                        ),
                    ],
                )
            }
            _ => Err(format!(
                "prob.KernelDistribution: unknown property '{name}'"
            )),
        }
    }
    pub fn method(&self, e: &mut Engine, name: &str, rest: &[Value]) -> Result<Option<Value>> {
        match name {
            "mean" | "std" | "var" | "median" => {
                if !rest.is_empty() {
                    return Err(format!(
                        "{name}: distribution objects take no additional arguments"
                    ));
                }
                let v = if name == "median" {
                    self.icdf(0.5)
                } else {
                    let (m, v) = self.moments();
                    match name {
                        "mean" => m,
                        "var" => v,
                        _ => v.sqrt(),
                    }
                };
                Ok(Some(Value::Matrix(Matrix::scalar(v))))
            }
            "pdf" | "cdf" | "icdf" => {
                let Some(x) = rest.first() else {
                    return Err(format!("{name}: expected evaluation points"));
                };
                let rest = e.text_arguments(rest[1..].to_vec())?;
                let upper = match rest.as_slice() {
                    [] => false,
                    [Value::Text(t)] if name == "cdf" && t.eq_ignore_ascii_case("upper") => true,
                    _ => return Err(format!("{name}: too many input arguments")),
                };
                let m = e.real_arg(name, x)?;
                e.preflight_elements(m.len())?;
                let mut out = Vec::with_capacity(m.len());
                for (i, &v) in m.data().iter().enumerate() {
                    if i.is_multiple_of(256) {
                        e.check_cancel()?;
                    }
                    out.push(match name {
                        "pdf" => self.pdf(v),
                        "icdf" => self.icdf(v),
                        _ if upper => 1. - self.cdf(v),
                        _ => self.cdf(v),
                    });
                }
                let dims = m.dims().into_owned();
                let rows = dims[0];
                let cols = dims[1..].iter().product();
                Ok(Some(Value::Matrix(
                    Matrix::from_column_vec(rows, cols, out)?.with_dims(&dims)?,
                )))
            }
            "random" => {
                let dims = if rest.is_empty() {
                    vec![1, 1]
                } else {
                    super::super::shape_arguments::shape_dims(rest, None)?
                };
                let total = dims.iter().product::<usize>();
                e.preflight_elements(total)?;
                let mut out = Vec::with_capacity(total);
                for i in 0..total {
                    if i.is_multiple_of(4096) {
                        e.check_cancel()?;
                    }
                    let u = e.random.next_f64();
                    let k = self.cum.partition_point(|c| *c <= u).min(self.y.len() - 1);
                    let z = self.kind.draw(&mut e.random);
                    out.push(self.untransform(self.y[k] + self.bandwidth * z));
                }
                let rows = dims[0];
                let cols = dims[1..].iter().product();
                Ok(Some(Value::Matrix(
                    Matrix::from_column_vec(rows, cols, out)?.with_dims(&dims)?,
                )))
            }
            "delete" | "isvalid" => Err(format!(
                "{name}: probability distributions are value objects"
            )),
            _ => Ok(None),
        }
    }
}

/// Fits a kernel estimate to the (uncensored) sample.
pub(super) fn fit(who: &str, s: &Sample, o: &KernelOptions) -> Result<KernelFit> {
    if s.has_cens() {
        return Err(format!(
            "{who}: censoring is not supported for the Kernel distribution"
        ));
    }
    let (lo, hi) = o.support;
    if s.x.iter().any(|&v| v <= lo || v >= hi) {
        return Err(format!(
            "{who}: Kernel observations must lie strictly inside the support"
        ));
    }
    let mut fit = KernelFit {
        kind: o.kind,
        bandwidth: 1.,
        support: o.support,
        input: FitInfo {
            data: s.x.clone(),
            cens: Vec::new(),
            freq: if s.has_freq() {
                s.freq.clone()
            } else {
                Vec::new()
            },
            cov: Vec::new(),
            fixed: Vec::new(),
        },
        y: Vec::new(),
        w: Vec::new(),
        cum: Vec::new(),
    };
    let total = s.weight();
    let mut points: Vec<(f64, f64)> = s
        .iter()
        .map(|(x, _, w)| (fit.transform(x), w / total))
        .collect();
    points.sort_by(|a, b| a.0.total_cmp(&b.0));
    // Equal values share one kernel term, so repeated observations do not make
    // every density sum O(n); `cum` keeps the running sum at each group's last
    // member, as before the merge, so cdf and random draws are unchanged.
    let mut acc = 0.;
    for &(y, w) in &points {
        acc += w;
        if fit.y.last() == Some(&y) {
            *fit.w.last_mut().expect("nonempty") += w;
            *fit.cum.last_mut().expect("nonempty") = acc;
        } else {
            fit.y.push(y);
            fit.w.push(w);
            fit.cum.push(acc);
        }
    }
    fit.bandwidth = match o.bandwidth {
        Some(h) => {
            if h.is_nan() || h <= 0. || !h.is_finite() {
                return Err(format!(
                    "{who}: the bandwidth must be a positive finite number"
                ));
            }
            h
        }
        None => {
            let med = weighted_median(&points);
            let mut dev: Vec<(f64, f64)> =
                points.iter().map(|p| ((p.0 - med).abs(), p.1)).collect();
            dev.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut sigma = weighted_median(&dev) / 0.6745;
            if sigma.is_nan() || sigma <= 0. {
                sigma = fit.y[fit.y.len() - 1] - fit.y[0];
            }
            if sigma.is_nan() || sigma <= 0. {
                sigma = 1.;
            }
            sigma * (4. / (3. * total)).powf(0.2)
        }
    };
    Ok(fit)
}

/// Gauss-Legendre nodes and weights on [-1, 1] (Newton on P_n).
fn gauss_legendre(n: usize) -> (Vec<f64>, Vec<f64>) {
    let mut nodes = vec![0.; n];
    let mut weights = vec![0.; n];
    for i in 0..n.div_ceil(2) {
        let mut x = (PI * (i as f64 + 0.75) / (n as f64 + 0.5)).cos();
        let mut dp = 0.;
        for _ in 0..100 {
            let (mut p0, mut p1) = (1., x);
            for k in 2..=n {
                let p2 = ((2 * k - 1) as f64 * x * p1 - (k - 1) as f64 * p0) / k as f64;
                p0 = p1;
                p1 = p2;
            }
            dp = n as f64 * (x * p1 - p0) / (x * x - 1.);
            let dx = p1 / dp;
            x -= dx;
            if dx.abs() < 1e-16 {
                break;
            }
        }
        nodes[i] = -x;
        nodes[n - 1 - i] = x;
        let w = 2. / ((1. - x * x) * dp * dp);
        weights[i] = w;
        weights[n - 1 - i] = w;
    }
    (nodes, weights)
}
