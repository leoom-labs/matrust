//! Model structures and their sampled simulation. Every model is simulated
//! through one discrete state-space realisation (controller canonical form for
//! polynomial and rational models) plus an integer input shift, so
//! simulation, initial-state estimation and k-step prediction share one path.
//! Continuous models are sampled with a zero-order hold; a fractional input
//! delay uses the two-integral formula of Astrom & Wittenmark,
//! Computer-Controlled Systems, 3rd ed., sec. 2.3 (matrix exponentials of
//! [[A B],[0 0]], Van Loan 1978). k-step predictors follow Ljung, System
//! Identification, 2nd ed., eq. (3.31): yhat_k = y - Hbar_k(q) eps, with Hbar_k
//! the first k impulse-response terms of the noise model H.
use super::num::{conv, lfilter, shift};
use crate::Result;
use nalgebra::{DMatrix, DVector};

/// Multiply-adds allowed for one k-step prediction (a few seconds).
pub(super) const PREDICT_WORK: u64 = 10_000_000_000;

pub(super) type Realization = (DMatrix<f64>, DVector<f64>, DVector<f64>, f64);

#[derive(Clone, Debug)]
pub(super) struct Proc {
    pub poles: usize,
    pub delay: bool,
    pub zero: bool,
    pub integrator: bool,
    pub under: bool,
    pub kp: f64,
    pub tp: [f64; 3],
    pub tw: f64,
    pub zeta: f64,
    pub td: f64,
    pub tz: f64,
}
impl Proc {
    pub fn parse(t: &str) -> Result<Self> {
        let up = t.trim().to_ascii_uppercase();
        let bad = || {
            format!("Invalid process model type '{t}'; use e.g. 'P1', 'P1D', 'P2', 'P2DZ' or 'P2U'")
        };
        let rest = up.strip_prefix('P').ok_or_else(bad)?;
        let mut chars = rest.chars();
        let poles = chars
            .next()
            .and_then(|c| c.to_digit(10))
            .filter(|n| *n <= 3)
            .ok_or_else(bad)? as usize;
        let mut p = Proc {
            poles,
            delay: false,
            zero: false,
            integrator: false,
            under: false,
            kp: f64::NAN,
            tp: [0.; 3],
            tw: 0.,
            zeta: 0.,
            td: 0.,
            tz: 0.,
        };
        for c in chars {
            let flag = match c {
                'D' => &mut p.delay,
                'Z' => &mut p.zero,
                'I' => &mut p.integrator,
                'U' => &mut p.under,
                _ => return Err(bad()),
            };
            if *flag {
                return Err(bad());
            }
            *flag = true;
        }
        if p.under && p.poles < 2 {
            return Err(bad());
        }
        if p.zero && p.poles == 0 {
            return Err(bad());
        }
        Ok(p)
    }
    pub fn type_name(&self) -> String {
        let mut s = format!("P{}", self.poles);
        if self.delay {
            s.push('D');
        }
        if self.zero {
            s.push('Z');
        }
        if self.integrator {
            s.push('I');
        }
        if self.under {
            s.push('U');
        }
        s
    }
    /// Free parameter names in a fixed order.
    pub fn names(&self) -> Vec<&'static str> {
        let mut v = vec!["Kp"];
        if self.under {
            v.extend(["Tw", "Zeta"]);
            if self.poles == 3 {
                v.push("Tp3");
            }
        } else {
            v.extend(["Tp1", "Tp2", "Tp3"].iter().take(self.poles));
        }
        if self.delay {
            v.push("Td");
        }
        if self.zero {
            v.push("Tz");
        }
        v
    }
    pub fn get(&self, n: &str) -> f64 {
        match n {
            "Kp" => self.kp,
            "Tp1" => self.tp[0],
            "Tp2" => self.tp[1],
            "Tp3" => self.tp[2],
            "Tw" => self.tw,
            "Zeta" => self.zeta,
            "Td" => self.td,
            _ => self.tz,
        }
    }
    pub fn set(&mut self, n: &str, v: f64) {
        match n {
            "Kp" => self.kp = v,
            "Tp1" => self.tp[0] = v,
            "Tp2" => self.tp[1] = v,
            "Tp3" => self.tp[2] = v,
            "Tw" => self.tw = v,
            "Zeta" => self.zeta = v,
            "Td" => self.td = v,
            _ => self.tz = v,
        }
    }
    /// Continuous transfer function (descending powers of s) without delay.
    pub fn tf(&self) -> (Vec<f64>, Vec<f64>) {
        let num = if self.zero {
            vec![self.kp * self.tz, self.kp]
        } else {
            vec![self.kp]
        };
        let mut den = vec![1.];
        if self.under {
            den = conv(&den, &[self.tw * self.tw, 2. * self.zeta * self.tw, 1.]);
            if self.poles == 3 {
                den = conv(&den, &[self.tp[2], 1.]);
            }
        } else {
            for t in &self.tp[..self.poles] {
                den = conv(&den, &[*t, 1.]);
            }
        }
        if self.integrator {
            den = conv(&den, &[1., 0.]);
        }
        (num, den)
    }
}

#[derive(Clone, Debug)]
pub(super) enum Structure {
    /// A(q) y = B(q)/F(q) u + C(q)/D(q) e, coefficients of q^0, q^-1, ...
    Poly {
        a: Vec<f64>,
        b: Vec<f64>,
        c: Vec<f64>,
        d: Vec<f64>,
        f: Vec<f64>,
    },
    /// Numerator/denominator in descending powers of s (Ts = 0) or z.
    Tf {
        num: Vec<f64>,
        den: Vec<f64>,
    },
    Ss {
        a: DMatrix<f64>,
        b: DMatrix<f64>,
        c: DMatrix<f64>,
        d: DMatrix<f64>,
        k: DMatrix<f64>,
    },
    Proc(Proc),
}
#[derive(Clone, Debug)]
pub(super) struct Model {
    pub s: Structure,
    pub ts: f64,
    /// Total input delay: seconds for continuous, samples for discrete.
    pub delay: f64,
    pub noise_variance: f64,
}

/// Single-input single-output discrete realisation with an input shift.
#[derive(Clone, Debug)]
pub(super) struct Dss {
    pub a: DMatrix<f64>,
    pub b: DVector<f64>,
    pub c: DVector<f64>,
    pub d: f64,
    pub shift: usize,
}
impl Dss {
    pub fn order(&self) -> usize {
        self.b.len()
    }
    /// Output for the (unshifted) input u from initial state x0.
    pub fn simulate(&self, u: &[f64], x0: Option<&DVector<f64>>) -> Vec<f64> {
        let u = shift(u, self.shift);
        let n = self.order();
        let mut x = x0.cloned().unwrap_or_else(|| DVector::zeros(n));
        let mut y = Vec::with_capacity(u.len());
        let mut next = DVector::zeros(n);
        for &ut in &u {
            y.push(self.c.dot(&x) + self.d * ut);
            self.a.mul_to(&x, &mut next);
            next.axpy(ut, &self.b, 1.);
            std::mem::swap(&mut x, &mut next);
        }
        y
    }
    /// Free responses C A^k e_i (columns), for initial-state estimation.
    pub fn free(&self, len: usize) -> DMatrix<f64> {
        let n = self.order();
        let mut o = DMatrix::zeros(len, n);
        let mut row = self.c.transpose();
        for t in 0..len {
            o.row_mut(t).copy_from(&row);
            row = &row * &self.a;
        }
        o
    }
}

/// Controller canonical realisation of num/den (descending powers, same
/// length after padding, den[0] != 0).
pub(super) fn realize(num: &[f64], den: &[f64]) -> Result<Realization> {
    let den = trim(den);
    let num = trim(num);
    if den.is_empty() {
        return Err("Model denominator is zero".into());
    }
    if num.len() > den.len() {
        return Err("Improper models (more zeros than poles) cannot be simulated".into());
    }
    let n = den.len() - 1;
    let lead = den[0];
    let a: Vec<f64> = den.iter().map(|v| v / lead).collect();
    let mut b = vec![0.; n + 1 - num.len()];
    b.extend(num.iter().map(|v| v / lead));
    let mut am = DMatrix::zeros(n, n);
    for j in 0..n {
        am[(0, j)] = -a[j + 1];
    }
    for j in 1..n {
        am[(j, j - 1)] = 1.;
    }
    let mut bm = DVector::zeros(n);
    if n > 0 {
        bm[0] = 1.;
    }
    let cm = DVector::from_iterator(n, (0..n).map(|j| b[j + 1] - b[0] * a[j + 1]));
    Ok((am, bm, cm, b[0]))
}
pub(super) fn trim(p: &[f64]) -> Vec<f64> {
    match p.iter().position(|v| *v != 0.) {
        Some(i) => p[i..].to_vec(),
        None => Vec::new(),
    }
}
/// [[e^{Ah}, int_0^h e^{As} ds B], ...] blocks of the augmented exponential.
fn hold(a: &DMatrix<f64>, b: &DVector<f64>, h: f64) -> (DMatrix<f64>, DVector<f64>) {
    let n = a.nrows();
    let mut m = DMatrix::zeros(n + 1, n + 1);
    m.view_mut((0, 0), (n, n)).copy_from(&(a * h));
    m.view_mut((0, n), (n, 1)).copy_from(&(b * h));
    let e = m.exp();
    (
        e.view((0, 0), (n, n)).into_owned(),
        e.view((0, n), (n, 1)).column(0).into_owned(),
    )
}
/// Zero-order-hold sampling of a continuous SISO realisation with input
/// delay `td` seconds.
pub(super) fn sample(
    a: &DMatrix<f64>,
    b: &DVector<f64>,
    c: &DVector<f64>,
    d: f64,
    td: f64,
    t: f64,
) -> Dss {
    let ratio = td / t;
    let mut whole = ratio.floor();
    let mut frac = td - whole * t;
    if frac <= 1e-10 * t {
        frac = 0.;
    } else if t - frac <= 1e-10 * t {
        frac = 0.;
        whole += 1.;
    }
    let shift = whole.max(0.) as usize;
    let n = a.nrows();
    if frac == 0. {
        let (phi, gamma) = hold(a, b, t);
        return Dss {
            a: phi,
            b: gamma,
            c: c.clone(),
            d,
            shift,
        };
    }
    // u' = u shifted by `shift`; w = u'(k-1) is one extra state.
    let (phi, _) = hold(a, b, t);
    let (e1, g_new) = hold(a, b, t - frac);
    let (_, g_old) = hold(a, b, frac);
    let g_old = e1 * g_old;
    let mut aa = DMatrix::zeros(n + 1, n + 1);
    aa.view_mut((0, 0), (n, n)).copy_from(&phi);
    aa.view_mut((0, n), (n, 1)).copy_from(&g_old);
    let mut bb = DVector::zeros(n + 1);
    bb.rows_mut(0, n).copy_from(&g_new);
    bb[n] = 1.;
    let mut cc = DVector::zeros(n + 1);
    cc.rows_mut(0, n).copy_from(c);
    // y(kT) = C x + D u(kT - td): the older held sample when frac > 0.
    cc[n] = d;
    Dss {
        a: aa,
        b: bb,
        c: cc,
        d: 0.,
        shift,
    }
}

impl Model {
    pub fn continuous(&self) -> bool {
        self.ts == 0.
    }
    /// The whole-sample delay of a discrete model (bounded: model decoding
    /// limits delays to `MAX_SAMPLES`; saturating for safety).
    pub fn delay_samples(&self) -> usize {
        self.delay.round().clamp(0., super::obj::MAX_SAMPLES as f64) as usize
    }
    /// A discrete model runs only on data with its own sample time.
    pub fn check_ts(&self, t: f64) -> Result<()> {
        if !self.continuous() && (self.ts - t).abs() > 1e-9 * t.max(self.ts) {
            return Err(format!(
                "The model sample time ({}) differs from the data sample time ({t})",
                self.ts
            ));
        }
        Ok(())
    }
    /// Measured transfer function u -> y as (num, den) in descending powers
    /// (s for continuous, z for discrete) and the delay, if rational.
    pub fn rational(&self) -> Option<(Vec<f64>, Vec<f64>)> {
        match &self.s {
            Structure::Poly { a, b, f, .. } => {
                let den = conv(a, f);
                let mut num = b.clone();
                if num.is_empty() {
                    num = vec![0.];
                }
                let len = num.len().max(den.len());
                let mut n = num.clone();
                n.resize(len, 0.);
                let mut d = den;
                d.resize(len, 0.);
                Some((n, d))
            }
            Structure::Tf { num, den } => Some((num.clone(), den.clone())),
            Structure::Proc(p) => Some(p.tf()),
            Structure::Ss { .. } => None,
        }
    }
    /// Sampled realisation for data with sample time `t`.
    pub fn dss(&self, t: f64) -> Result<Dss> {
        self.check_ts(t)?;
        let (a, b, c, d) = match &self.s {
            Structure::Ss { a, b, c, d, .. } => {
                if b.ncols() != 1 || c.nrows() != 1 {
                    return Err("Only single-input single-output models are supported yet".into());
                }
                (
                    a.clone(),
                    b.column(0).into_owned(),
                    c.row(0).transpose(),
                    d[(0, 0)],
                )
            }
            Structure::Poly { a, b, f, .. } => {
                let nk = b.iter().position(|v| *v != 0.).unwrap_or(b.len());
                let den = conv(a, f);
                let num: Vec<f64> = if nk < b.len() {
                    b[nk..].to_vec()
                } else {
                    vec![0.]
                };
                let len = num.len().max(den.len());
                let mut n = num;
                n.resize(len, 0.);
                let mut dd = den;
                dd.resize(len, 0.);
                let (a, b, c, d) = realize(&n, &dd)?;
                return Ok(Dss {
                    a,
                    b,
                    c,
                    d,
                    shift: nk.saturating_add(self.delay_samples()),
                });
            }
            _ => {
                let (num, den) = self.rational().expect("rational model");
                let num = trim(&num);
                let den = trim(&den);
                if self.continuous() {
                    let (a, b, c, d) = realize(&num, &den)?;
                    (a, b, c, d)
                } else {
                    // z-domain descending: the degree gap is a pure delay.
                    let gap = den.len().saturating_sub(num.len().max(1));
                    let mut n = num.clone();
                    if n.is_empty() {
                        n = vec![0.];
                    }
                    let len = n.len().max(den.len());
                    n.resize(len, 0.);
                    let mut dd = den.clone();
                    dd.resize(len, 0.);
                    let (a, b, c, d) = realize(&n, &dd)?;
                    return Ok(Dss {
                        a,
                        b,
                        c,
                        d,
                        shift: gap.saturating_add(self.delay_samples()),
                    });
                }
            }
        };
        if self.continuous() {
            let td = self.delay
                + match &self.s {
                    Structure::Proc(p) if p.delay => p.td,
                    _ => 0.,
                };
            Ok(sample(&a, &b, &c, d, td.max(0.), t))
        } else {
            Ok(Dss {
                a,
                b,
                c,
                d,
                shift: self.delay_samples(),
            })
        }
    }
    /// Zero-state simulation.
    pub fn simulate(&self, u: &[f64], t: f64) -> Result<Vec<f64>> {
        if let Structure::Poly { a, b, f, .. } = &self.s
            && !self.continuous()
        {
            self.check_ts(t)?;
            let x = shift(u, self.delay_samples());
            if b.is_empty() {
                return Ok(vec![0.; u.len()]);
            }
            return Ok(lfilter(b, &conv(a, f), &x));
        }
        Ok(self.dss(t)?.simulate(u, None))
    }
    /// One-step prediction errors with zero initial conditions.
    pub fn innovations(&self, y: &[f64], u: &[f64], t: f64) -> Result<Vec<f64>> {
        match &self.s {
            Structure::Poly { a, b, c, d, f } if !self.continuous() => {
                self.check_ts(t)?;
                let ay = lfilter(a, &[1.], y);
                let w = if b.is_empty() {
                    vec![0.; y.len()]
                } else {
                    lfilter(b, f, &shift(u, self.delay_samples()))
                };
                let v: Vec<f64> = ay.iter().zip(&w).map(|(p, q)| p - q).collect();
                Ok(lfilter(d, c, &v))
            }
            Structure::Ss { a, b, c, d, k } if !self.continuous() && k.iter().any(|v| *v != 0.) => {
                self.check_ts(t)?;
                // One output; one input, or none (a time-series model).
                if c.nrows() != 1 || b.ncols() > 1 {
                    return Err("Only single-input single-output models are supported yet".into());
                }
                let n = a.nrows();
                let mut x = DVector::zeros(n);
                let cc = c.row(0);
                let kk = k.column(0);
                let mut out = Vec::with_capacity(y.len());
                if b.ncols() == 0 {
                    for yt in y {
                        let e = yt - (cc * &x)[(0, 0)];
                        out.push(e);
                        x = a * &x + kk * e;
                    }
                    return Ok(out);
                }
                let u = shift(u, self.delay_samples());
                let bb = b.column(0);
                for (yt, ut) in y.iter().zip(&u) {
                    let e = yt - (cc * &x)[(0, 0)] - d[(0, 0)] * ut;
                    out.push(e);
                    x = a * &x + bb * *ut + kk * e;
                }
                Ok(out)
            }
            _ => {
                let s = self.simulate(u, t)?;
                Ok(y.iter().zip(&s).map(|(p, q)| p - q).collect())
            }
        }
    }
    /// First `k` impulse-response coefficients of the noise model H.
    pub fn noise_impulse(&self, k: usize) -> Vec<f64> {
        let mut delta = vec![0.; k];
        if k > 0 {
            delta[0] = 1.;
        }
        match &self.s {
            Structure::Poly { a, c, d, .. } if !self.continuous() => {
                lfilter(c, &conv(a, d), &delta)
            }
            Structure::Ss { a, c, k: kg, .. }
                if !self.continuous() && c.nrows() == 1 && kg.ncols() == 1 =>
            {
                let mut h = delta;
                let mut x = kg.column(0).into_owned();
                for v in h.iter_mut().skip(1) {
                    *v = (c.row(0) * &x)[(0, 0)];
                    x = a * &x;
                }
                h
            }
            _ => delta,
        }
    }
    /// k-step-ahead prediction (k = usize::MAX: pure simulation). The
    /// k-term FIR filter of the innovations costs N k operations: bounded by
    /// `PREDICT_WORK` and polling `cancel` every block of output samples.
    pub fn predict(
        &self,
        y: &[f64],
        u: &[f64],
        t: f64,
        k: usize,
        cancel: &dyn Fn() -> Result<()>,
    ) -> Result<Vec<f64>> {
        if k >= y.len() {
            return self.simulate(u, t);
        }
        if (y.len() as u64).saturating_mul(k as u64) > PREDICT_WORK {
            return Err(format!(
                "predict: samples times prediction horizon exceeds the work bound ({PREDICT_WORK})"
            ));
        }
        let eps = self.innovations(y, u, t)?;
        let h = self.noise_impulse(k);
        let mut out = Vec::with_capacity(y.len());
        let block = (1 << 20) / k.max(1) + 1;
        for (i, yt) in y.iter().enumerate() {
            if i % block == 0 {
                cancel()?;
            }
            let lo = (i + 1).saturating_sub(k);
            let s: f64 = (lo..=i).map(|j| h[i - j] * eps[j]).sum();
            out.push(yt - s);
        }
        Ok(out)
    }
    /// Number of free parameters (for FPE/AIC).
    pub fn parameters(&self) -> usize {
        match &self.s {
            Structure::Poly { a, b, c, d, f } => {
                let nb = b.iter().skip_while(|v| **v == 0.).count();
                a.len() - 1 + nb + c.len() - 1 + d.len() - 1 + f.len().saturating_sub(1)
            }
            Structure::Tf { num, den } => trim(num).len() + trim(den).len() - 1,
            Structure::Ss { a, b, c, d, k } => {
                let n = a.nrows();
                n * n
                    + b.len()
                    + c.len()
                    + d.iter().filter(|v| **v != 0.).count()
                    + k.iter().filter(|v| **v != 0.).count()
            }
            Structure::Proc(p) => p.names().len(),
        }
    }
}
