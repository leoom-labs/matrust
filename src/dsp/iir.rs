//! IIR design and representation conversions, derived from published filter mathematics.
//! Butterworth/Chebyshev/bilinear formulas follow Oppenheim & Schafer, *Discrete-Time
//! Signal Processing*, 3rd ed., ch. 7. Elliptic formulas follow S. J. Orfanidis,
//! *Lecture Notes on Elliptic Filter Design* (2006), §§2, 3, 5, 8, 9; the legal author-posted
//! snapshot used here is https://web.archive.org/web/20231207232045/http://eceweb1.rutgers.edu/~orfanidi/ece521/notes.pdf.
//! Bessel roots use the reverse Bessel polynomials (Thomson response), as in standard filter texts.
use crate::{Complex64 as C, Matrix, Result};
use nalgebra::{DMatrix, linalg::Schur};
use std::{
    f64::consts::PI,
    sync::atomic::{AtomicBool, Ordering},
};

pub const MAX_DESIGN_ORDER: usize = 100;
const ROOT_EPS: f64 = 1e-12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IirKind {
    Butter,
    Cheby2,
    Ellip,
    Bessel,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IirBand {
    Low,
    High,
    Bandpass,
    Stop,
}
#[derive(Clone, Debug)]
pub struct Zpk {
    pub zeros: Vec<C>,
    pub poles: Vec<C>,
    pub gain: f64,
}
pub trait EllipticMath {
    fn jacobi(&self, u: f64, m: f64) -> Result<(f64, f64, f64)>;
    fn complete(&self, m: f64) -> Result<(f64, f64)>;
}

fn cancel(c: &AtomicBool) -> Result<()> {
    if c.load(Ordering::Relaxed) {
        Err("Calculation cancelled".into())
    } else {
        Ok(())
    }
}
fn finite(z: C) -> bool {
    z.re.is_finite() && z.im.is_finite()
}
fn pair_count(b: IirBand) -> usize {
    if matches!(b, IirBand::Bandpass | IirBand::Stop) {
        2
    } else {
        1
    }
}
fn validate_edges(w: &[f64], b: IirBand, analog: bool) -> Result<()> {
    let n = pair_count(b);
    if w.len() != n
        || w.iter()
            .any(|v| !v.is_finite() || *v <= 0. || (!analog && *v >= 1.))
        || (n == 2 && w[0] >= w[1])
    {
        return Err("IIR edge frequencies must be positive, finite, increasing for a band, and below Nyquist for digital designs".into());
    }
    Ok(())
}
fn cprod(xs: &[C]) -> C {
    xs.iter().copied().fold(C::new(1., 0.), |a, b| a * b)
}
fn poly(roots: &[C], gain: f64) -> Vec<C> {
    // Expand the monic polynomial before scaling, so a small gain does not
    // introduce roundoff into structural cancellations (e.g. roots at +/-1).
    let mut a = vec![C::new(1., 0.)];
    for &r in roots {
        let mut b = vec![C::new(0., 0.); a.len() + 1];
        for (i, &v) in a.iter().enumerate() {
            b[i] += v;
            b[i + 1] -= v * r;
        }
        a = b;
    }
    a.iter_mut().for_each(|x| *x *= gain);
    a
}
fn real_vec(xs: Vec<C>, label: &str) -> Result<Vec<f64>> {
    if xs
        .iter()
        .any(|z| !finite(*z) || z.im.abs() > 2e-8 * (1. + z.re.abs()))
    {
        return Err(format!("{label} is not a finite real coefficient vector"));
    }
    Ok(xs
        .into_iter()
        .map(|z| if z.re.abs() < 1e-300 { 0. } else { z.re })
        .collect())
}
fn roots(coeff: &[C]) -> Result<Vec<C>> {
    roots_up_to(coeff, MAX_DESIGN_ORDER * 2)
}
/// Polynomial roots (companion matrix eigenvalues) for degrees up to `max_degree`.
pub(crate) fn roots_up_to(coeff: &[C], max_degree: usize) -> Result<Vec<C>> {
    if coeff.len() <= 1 {
        return Ok(Vec::new());
    }
    let first = coeff
        .iter()
        .position(|x| x.norm() != 0.)
        .ok_or("Polynomial is zero")?;
    let last = coeff.iter().rposition(|x| x.norm() != 0.).unwrap_or(first);
    let c = &coeff[first..=last];
    let n = c.len() - 1;
    if n > max_degree {
        return Err(format!("IIR polynomial degree exceeds {max_degree}"));
    }
    if n == 0 {
        return Ok(vec![C::new(0., 0.); coeff.len() - 1 - last]);
    }
    let mut a = DMatrix::<C>::zeros(n, n);
    for j in 0..n {
        a[(0, j)] = -c[j + 1] / c[0];
    }
    for i in 1..n {
        a[(i, i - 1)] = C::new(1., 0.);
    }
    let s = Schur::try_new(a, f64::EPSILON, 0).ok_or("Polynomial root solve did not converge")?;
    let mut r = s
        .eigenvalues()
        .ok_or("Polynomial root solve did not converge")?
        .iter()
        .copied()
        .collect::<Vec<_>>();
    r.extend(std::iter::repeat_n(C::new(0., 0.), coeff.len() - 1 - last));
    Ok(r)
}
fn butter(n: usize) -> Result<Zpk> {
    let p = (0..n)
        .map(|k| {
            let a = PI * (2 * k + 1 + n) as f64 / (2 * n) as f64;
            C::from_polar(1., a)
        })
        .collect::<Vec<_>>();
    let g = cprod(&p.iter().map(|p| -*p).collect::<Vec<_>>()).re;
    Ok(Zpk {
        zeros: vec![],
        poles: p,
        gain: g,
    })
}
fn cheby2(n: usize, rs: f64) -> Result<Zpk> {
    if !rs.is_finite() || rs <= 0. {
        return Err("Stopband attenuation must be positive and finite".into());
    }
    let ep = (10f64.powf(rs / 10.) - 1.).sqrt();
    let mu = ep.asinh() / n as f64;
    let mut p = Vec::with_capacity(n);
    let mut z = Vec::with_capacity(n - n % 2);
    for k in 0..n {
        let t = PI * (2 * k + 1) as f64 / (2 * n) as f64;
        let q = C::new(-mu.sinh() * t.sin(), mu.cosh() * t.cos());
        p.push(C::new(1., 0.) / q);
        if (2 * k + 1) != n {
            z.push(C::new(0., 1. / t.cos()));
        }
    }
    let g = (cprod(&p.iter().map(|x| -*x).collect::<Vec<_>>())
        / cprod(&z.iter().map(|x| -*x).collect::<Vec<_>>()))
    .re;
    Ok(Zpk {
        zeros: z,
        poles: p,
        gain: g,
    })
}
// Real Jacobi sn/cn/dn via descending Landen AGM; parameter is m=k^2.
fn jacobi(u: f64, m: f64, math: &dyn EllipticMath) -> Result<(f64, f64, f64)> {
    math.jacobi(u, m)
}
fn elliptic_k(m: f64, math: &dyn EllipticMath) -> Result<f64> {
    Ok(math.complete(m)?.0)
}
// K(1-mc) via AGM(1,sqrt(mc)), retaining the small complementary
// parameter directly (Orfanidis 2006, complete elliptic integral AGM identity).
fn elliptic_k_complement(mc: f64) -> Result<f64> {
    if mc <= 0. || mc > 1. {
        return Err("Elliptic complementary parameter is outside (0,1]".into());
    }
    let (mut a, mut b) = (1., mc.sqrt());
    for _ in 0..64 {
        let next = (a + b) * 0.5;
        b = (a * b).sqrt();
        a = next;
        if (a - b).abs() <= 2. * f64::EPSILON * a {
            return Ok(PI / (2. * a));
        }
    }
    Err("Elliptic AGM did not converge".into())
}
// Descending Landen, as in the repository's ellipj kernel (DLMF 22.20.2-3),
// initialized with k'=sqrt(mc). This also preserves the complementary
// parameter in the inverse-sc step when 1-mc would round to one.
fn jacobi_complement(u: f64, mc: f64) -> Result<(f64, f64, f64)> {
    if mc <= 0. || mc > 1. {
        return Err("Jacobi complementary parameter is outside (0,1]".into());
    }
    let (mut a, mut b) = (1., mc.sqrt());
    let mut ratios = [0.; 64];
    let mut steps = 0;
    for ratio in &mut ratios {
        let c = (a - b) * 0.5;
        let next = (a + b) * 0.5;
        b = (a * b).sqrt();
        a = next;
        *ratio = c / a;
        steps += 1;
        if c.abs() <= f64::EPSILON * a {
            let mut phi = libm::scalbn(a * u, steps as i32);
            for &r in ratios[..steps].iter().rev() {
                phi = (phi + (r * phi.sin()).clamp(-1., 1.).asin()) * 0.5;
            }
            let (sn, cn) = phi.sin_cos();
            return Ok((sn, cn, (cn * cn + mc * sn * sn).sqrt()));
        }
    }
    Err("Jacobi complementary AGM did not converge".into())
}
pub fn elliptic_order(selectivity: f64, k1: f64, math: &dyn EllipticMath) -> Result<usize> {
    let k = 1. / selectivity;
    let m = k * k;
    let m1 = k1 * k1;
    Ok(((elliptic_k_complement(m1)? / elliptic_k(m1, math)?)
        / (elliptic_k_complement(m)? / elliptic_k(m, math)?))
    .ceil()
    .max(1.) as usize)
}
fn cd_complex(x: f64, y: f64, m: f64, math: &dyn EllipticMath) -> Result<C> {
    let (su, cu, du) = jacobi(x, m, math)?;
    let (sv, cv, dv) = jacobi_complement(y.abs(), m)?;
    // Imaginary-argument transforms followed by Jacobi addition formulas.
    let s_i = C::new(0., y.signum() * sv / cv);
    let c_i = C::new(1. / cv, 0.);
    let d_i = C::new(dv / cv, 0.);
    let den = C::new(1., 0.) - m * su * su * s_i * s_i;
    let sn = (su * c_i * d_i + s_i * cu * du) / den;
    let cn = (cu * c_i - sn * 0. + C::new(0., 0.) - su * du * s_i * d_i) / den;
    let dn = (du * d_i - m * su * cu * s_i * c_i) / den;
    Ok(cn / dn)
}
fn ellip(n: usize, rp: f64, rs: f64, math: &dyn EllipticMath) -> Result<Zpk> {
    if !rp.is_finite() || !rs.is_finite() || rp <= 0. || rs <= rp {
        return Err("ellip requires 0 < Rp < Rs".into());
    }
    let ep = (10f64.powf(rp / 10.) - 1.).sqrt();
    let es = (10f64.powf(rs / 10.) - 1.).sqrt();
    let k1 = ep / es;
    if !k1.is_finite() || k1 <= 0. || k1 >= 1. {
        return Err("Elliptic ripple specifications exceed the stable f64 range".into());
    }
    let m1 = k1 * k1;
    let k1c = elliptic_k_complement(m1)?;
    let k1v = elliptic_k(m1, math)?;
    // Degree equation K'(k)/K(k)=K'(k1)/(n K(k1)); solve monotonically for k.
    let target = k1c / (n as f64 * k1v);
    let (mut lo, mut hi) = (1e-14, 1. - 1e-14);
    for _ in 0..90 {
        let mid = (lo + hi) * 0.5;
        if elliptic_k_complement(mid * mid)? / elliptic_k(mid * mid, math)? > target {
            lo = mid
        } else {
            hi = mid
        }
    }
    let k = (lo + hi) * 0.5;
    let m = k * k;
    let kk = elliptic_k(m, math)?;
    let k1q = elliptic_k(m1, math)?;
    let mut xlo = 0.;
    let mut xhi = elliptic_k_complement(m1)?;
    for _ in 0..80 {
        let mid = (xlo + xhi) * 0.5;
        let (s, c, _) = jacobi_complement(mid, m1)?;
        if s / c < 1. / ep {
            xlo = mid
        } else {
            xhi = mid
        }
    }
    let v0 = (xlo + xhi) * 0.5 / (n as f64 * k1q);
    let mut zeros = Vec::new();
    let mut poles = Vec::new();
    for i in 0..n / 2 {
        let u = (2 * i + 1) as f64 / n as f64;
        let (_sn, cn, dn) = jacobi(u * kk, m, math)?;
        let cd = cn / dn;
        zeros.push(C::new(0., 1. / (k * cd)));
        poles.push(C::new(0., 1.) * cd_complex(u * kk, -v0 * kk, m, math)?);
    }
    if n % 2 == 1 {
        let z = C::new(0., 1.) * cd_complex(kk, -v0 * kk, m, math)?;
        poles.push(z);
    }
    let g = if n.is_multiple_of(2) {
        10f64.powf(-rp / 20.)
    } else {
        1.
    };
    // Keep only left-half-plane poles and add conjugates; zpk vectors are real-system roots.
    let poles = poles
        .into_iter()
        .flat_map(|p| {
            if p.im.abs() < 1e-10 {
                vec![C::new(p.re, 0.)]
            } else {
                vec![
                    if p.re > 0. { p.conj() } else { p },
                    if p.re > 0. { p } else { p.conj() },
                ]
            }
        })
        .collect::<Vec<_>>();
    let zeros: Vec<C> = zeros.into_iter().flat_map(|z| vec![z, z.conj()]).collect();
    let gain = (C::new(g, 0.) * cprod(&poles.iter().map(|p| -*p).collect::<Vec<_>>())
        / cprod(&zeros.iter().map(|z| -*z).collect::<Vec<_>>()))
    .re;
    Ok(Zpk { zeros, poles, gain })
}
fn bessel(n: usize) -> Result<Zpk> {
    // Reverse Bessel polynomial roots give the Thomson delay response: W. E. Thomson,
    // "Delay Networks Having Maximally Flat Frequency Characteristics", Proc. IEE, 96(44), 487–490 (1949).
    // theta_n(s) = sum a_k s^k, a_n=1,
    // a_(k-1)/a_k = k(2n-k+1)/(2(n-k+1)). No integer factorials.
    // Solve in s=n*x to reduce companion-matrix dynamic range.
    let scale = n as f64;
    let mut c = vec![C::new(1., 0.)];
    for k in (1..=n).rev() {
        let ratio = (k * (2 * n - k + 1)) as f64 / (2 * (n - k + 1)) as f64;
        c.push(*c.last().unwrap() * (ratio / scale));
    }
    let p = roots(&c)?
        .into_iter()
        .map(|x| x * scale)
        .collect::<Vec<_>>();
    let g = c.last().unwrap().re * scale.powi(n as i32);
    Ok(Zpk {
        zeros: vec![],
        poles: p,
        gain: g,
    })
}
fn lp_transform(mut z: Zpk, b: IirBand, w: &[f64], analog: bool) -> Result<Zpk> {
    let (w1, w2) = if analog {
        (w[0], if w.len() == 2 { w[1] } else { w[0] })
    } else {
        let f = |v: f64| 2. * (PI * v / 2.).tan();
        (f(w[0]), if w.len() == 2 { f(w[1]) } else { f(w[0]) })
    };
    let degree = z.poles.len();
    let nz = z.zeros.len();
    match b {
        IirBand::Low => {
            z.zeros.iter_mut().for_each(|v| *v *= w1);
            z.poles.iter_mut().for_each(|v| *v *= w1);
            z.gain *= w1.powi((degree - nz) as i32);
        }
        IirBand::High => {
            let ratio = cprod(&z.zeros.iter().map(|x| -*x).collect::<Vec<_>>())
                / cprod(&z.poles.iter().map(|x| -*x).collect::<Vec<_>>());
            z.zeros = z.zeros.iter().map(|x| C::new(w1, 0.) / *x).collect();
            z.zeros
                .extend(std::iter::repeat_n(C::new(0., 0.), degree - nz));
            z.poles = z.poles.iter().map(|x| C::new(w1, 0.) / *x).collect();
            z.gain *= ratio.re;
        }
        IirBand::Bandpass | IirBand::Stop => {
            let bw = w2 - w1;
            let wo = (w1 * w2).sqrt();
            let oldp = z.poles.clone();
            let oldz = z.zeros.clone();
            let mut pp = Vec::new();
            let mut zz = Vec::new();
            if b == IirBand::Bandpass {
                for &r in &oldp {
                    let q = bw * r;
                    let d = (q * q - C::new(4. * wo * wo, 0.)).sqrt();
                    pp.extend([(q + d) / 2., (q - d) / 2.]);
                }
                for &r in &oldz {
                    let q = bw * r;
                    let d = (q * q - C::new(4. * wo * wo, 0.)).sqrt();
                    zz.extend([(q + d) / 2., (q - d) / 2.]);
                }
                zz.extend(std::iter::repeat_n(C::new(0., 0.), degree - nz));
                z.gain *= bw.powi((degree - nz) as i32);
            } else {
                for &r in &oldp {
                    let d = (C::new(bw * bw, 0.) - C::new(4. * wo * wo, 0.) * r * r).sqrt();
                    pp.extend([
                        (C::new(bw, 0.) + d) / (2. * r),
                        (C::new(bw, 0.) - d) / (2. * r),
                    ]);
                }
                for &r in &oldz {
                    let d = (C::new(bw * bw, 0.) - C::new(4. * wo * wo, 0.) * r * r).sqrt();
                    zz.extend([
                        (C::new(bw, 0.) + d) / (2. * r),
                        (C::new(bw, 0.) - d) / (2. * r),
                    ]);
                }
                let j = C::new(0., wo);
                for _ in 0..degree - nz {
                    zz.extend([j, -j]);
                }
                z.gain *= (cprod(&oldz.iter().map(|x| -*x).collect::<Vec<_>>())
                    / cprod(&oldp.iter().map(|x| -*x).collect::<Vec<_>>()))
                .re;
            }
            z.poles = pp;
            z.zeros = zz;
        }
    }
    if !analog {
        let new_degree = z.poles.len();
        let missing = new_degree.saturating_sub(z.zeros.len());
        // Finite roots supply the leading gain factors. The zeros at infinity map to -1
        // without an additional gain multiplier.
        z.gain *= (cprod(
            &z.zeros
                .iter()
                .map(|x| C::new(2., 0.) - *x)
                .collect::<Vec<_>>(),
        ) / cprod(
            &z.poles
                .iter()
                .map(|x| C::new(2., 0.) - *x)
                .collect::<Vec<_>>(),
        ))
        .re;
        for r in z.poles.iter_mut().chain(z.zeros.iter_mut()) {
            *r = (C::new(2., 0.) + *r) / (C::new(2., 0.) - *r)
        }
        z.zeros
            .extend(std::iter::repeat_n(C::new(-1., 0.), missing));
    }
    if !z.gain.is_finite()
        || z.poles.iter().any(|p| !finite(*p))
        || z.zeros.iter().any(|p| !finite(*p))
    {
        return Err("IIR transform overflowed; move edges away from zero and Nyquist".into());
    }
    Ok(z)
}
pub struct DesignSpec<'a> {
    pub kind: IirKind,
    pub order: usize,
    pub rp: f64,
    pub rs: f64,
    pub edges: &'a [f64],
    pub band: IirBand,
    pub analog: bool,
}
pub fn design(spec: DesignSpec<'_>, math: &dyn EllipticMath, c: &AtomicBool) -> Result<Zpk> {
    cancel(c)?;
    if spec.order == 0 || spec.order > MAX_DESIGN_ORDER {
        return Err("IIR prototype order must be in 1..100".into());
    }
    // besself documents orders up to 25; higher Bessel poles lose accuracy.
    if spec.kind == IirKind::Bessel && spec.order > 25 {
        return Err("besself order must be in 1..25".into());
    }
    validate_edges(spec.edges, spec.band, spec.analog)?;
    let p = match spec.kind {
        IirKind::Butter => butter(spec.order)?,
        IirKind::Cheby2 => cheby2(spec.order, spec.rs)?,
        IirKind::Ellip => ellip(spec.order, spec.rp, spec.rs, math)?,
        IirKind::Bessel => {
            if !spec.analog || spec.band != IirBand::Low {
                return Err("besself supports analog lowpass only".into());
            }
            bessel(spec.order)?
        }
    };
    let result = lp_transform(p, spec.band, spec.edges, spec.analog)?;
    cancel(c)?;
    Ok(result)
}
pub fn zpk2tf(z: &Zpk) -> Result<(Vec<f64>, Vec<f64>)> {
    if z.zeros.len().max(z.poles.len()) > MAX_DESIGN_ORDER * 2 {
        return Err("IIR conversion degree exceeds 40".into());
    }
    let mut b = real_vec(poly(&z.zeros, z.gain), "Numerator")?;
    let a = real_vec(poly(&z.poles, 1.), "Denominator")?;
    if b.len() < a.len() {
        let mut padded = vec![0.; a.len() - b.len()];
        padded.extend(b);
        b = padded;
    }
    Ok((b, a))
}
pub fn tf2zpk(b: &[f64], a: &[f64]) -> Result<Zpk> {
    if b.is_empty() || a.is_empty() || a[0] == 0. || b.iter().chain(a).any(|x| !x.is_finite()) {
        return Err(
            "Transfer function coefficients must be finite nonempty vectors and a(1) nonzero"
                .into(),
        );
    }
    let bb = b.iter().map(|x| C::new(*x, 0.)).collect::<Vec<_>>();
    let aa = a.iter().map(|x| C::new(*x, 0.)).collect::<Vec<_>>();
    let lead = bb.iter().position(|x| x.norm() != 0.).unwrap_or(bb.len());
    let z = if lead == bb.len() {
        Vec::new()
    } else {
        roots(&bb[lead..])?
    };
    let p = roots(&aa)?;
    let g = if lead == bb.len() {
        0.
    } else {
        bb[lead].re / aa[0].re
    };
    Ok(Zpk {
        zeros: z,
        poles: p,
        gain: g,
    })
}
fn factor2(r: &[C]) -> Result<[f64; 3]> {
    let p = real_vec(poly(r, 1.), "SOS factor (roots must be conjugate paired)")?;
    Ok([p[0], *p.get(1).unwrap_or(&0.), *p.get(2).unwrap_or(&0.)])
}
fn real_root(r: C) -> bool {
    r.im == 0.
}
// Normalize roundoff in real roots and conjugate pairs; reject unpaired roots.
fn sos_roots(rs: &[C]) -> Result<Vec<C>> {
    let mut remaining = rs.to_vec();
    let mut out = Vec::new();
    while let Some(r) = remaining.pop() {
        if !finite(r) {
            return Err("SOS roots must be finite".into());
        }
        if r.im.abs() <= ROOT_EPS * r.norm().max(1.) {
            out.push(C::new(r.re, 0.));
        } else {
            let i = remaining
                .iter()
                .position(|q| (*q - r.conj()).norm() <= ROOT_EPS * r.norm().max(1.))
                .ok_or("SOS roots must contain conjugate pairs; unpaired complex root")?;
            let q = remaining.remove(i);
            let pair = (r + q.conj()) * 0.5;
            out.extend([
                C::new(pair.re, pair.im.abs()),
                C::new(pair.re, -pair.im.abs()),
            ]);
        }
    }
    out.sort_by(|a, b| a.re.total_cmp(&b.re).then_with(|| b.im.total_cmp(&a.im)));
    Ok(out)
}
fn take_nearest(rs: &mut Vec<C>, target: C, only_real: bool) -> Result<C> {
    let i = rs
        .iter()
        .enumerate()
        .filter(|(_, r)| !only_real || real_root(**r))
        .min_by(|a, b| (*a.1 - target).norm().total_cmp(&(*b.1 - target).norm()))
        .map(|(i, _)| i)
        .ok_or("Cannot form a real SOS section")?;
    Ok(rs.remove(i))
}
fn take_worst(rs: &mut Vec<C>, only_real: bool) -> Result<C> {
    let i = rs
        .iter()
        .enumerate()
        .filter(|(_, r)| !only_real || real_root(**r))
        .min_by(|a, b| (1. - a.1.norm()).abs().total_cmp(&(1. - b.1.norm()).abs()))
        .map(|(i, _)| i)
        .ok_or("Cannot form a real SOS pole pair")?;
    Ok(rs.remove(i))
}
/// Move `count` exact factors of z out of section coefficients so the constant
/// term sits at index 2. Padding a short zero list up to the pole count would
/// otherwise turn a degree deficit into roots at the origin.
fn shed_origin(rows: &mut [[f64; 6]], at: usize, mut count: usize) -> Result<()> {
    for row in rows.iter_mut() {
        let c = &mut row[at..at + 3];
        while count > 0 && c[2] == 0. {
            c[2] = c[1];
            c[1] = c[0];
            c[0] = 0.;
            count -= 1;
        }
    }
    if count > 0 {
        return Err("SOS degree deficit did not factor as zeros at the origin".into());
    }
    Ok(())
}
pub fn zpk2sos(z: &Zpk) -> Result<Matrix> {
    if z.zeros.len().max(z.poles.len()) > MAX_DESIGN_ORDER * 2 {
        return Err("IIR conversion degree exceeds 40".into());
    }
    if !z.gain.is_finite() {
        return Err("SOS gain must be finite".into());
    }
    let mut ps = sos_roots(&z.poles)?;
    let mut zs = sos_roots(&z.zeros)?;
    let npoles = ps.len();
    let nzeros = zs.len();
    let sections = npoles.max(nzeros).div_ceil(2).max(1);
    ps.resize(2 * sections, C::new(0., 0.));
    zs.resize(2 * sections, C::new(0., 0.));
    // SciPy's public zpk2sos nearest-pairing algorithm (documentation only):
    // https://docs.scipy.org/doc/scipy/reference/generated/scipy.signal.zpk2sos.html
    // Consume the most critical pole first; store it last. Complex roots consume
    // conjugates. Real poles take the next most critical real pole, except that
    // a complex zero selects its nearest remaining real pole.
    let mut rows = Vec::<[f64; 6]>::new();
    for _ in 0..sections {
        let p1 = take_worst(&mut ps, false)?;
        let z1 = take_nearest(&mut zs, p1, false)?;
        let p2 = if !real_root(p1) {
            take_nearest(&mut ps, p1.conj(), false)?
        } else if !real_root(z1) {
            take_nearest(&mut ps, z1, true)?
        } else {
            take_worst(&mut ps, true)?
        };
        let z2 = if !real_root(z1) {
            take_nearest(&mut zs, z1.conj(), false)?
        } else {
            take_nearest(&mut zs, if real_root(p1) { p2 } else { p1 }, true)?
        };
        let a = factor2(&[p1, p2])?;
        let b = factor2(&[z1, z2])?;
        rows.push([b[0], b[1], b[2], a[0], a[1], a[2]]);
    }
    // Fewer zeros than poles is a degree deficit (zp2tf left-pads the numerator).
    // The shared origin pad on an odd, equal count stays, so that first-order
    // section keeps its trailing zeros. Explicit zeros at 0 are not in this count.
    if nzeros < npoles {
        shed_origin(&mut rows, 0, npoles - nzeros)?;
    }
    rows.reverse();
    for b in &mut rows[0][..3] {
        *b *= z.gain;
    }
    let data = (0..6)
        .flat_map(|j| rows.iter().map(move |r| r[j]))
        .collect();
    Matrix::from_column_vec(rows.len(), 6, data)
}
pub fn sos2zpk(s: &Matrix) -> Result<Zpk> {
    if s.cols() != 6 || s.rows() == 0 || s.rows() > MAX_DESIGN_ORDER {
        return Err("SOS must be a nonempty matrix with six columns".into());
    }
    let mut z = Vec::new();
    let mut p = Vec::new();
    let mut k = 1.;
    for i in 0..s.rows() {
        let r = |j| {
            [
                C::new(s.get(i, j), 0.),
                C::new(s.get(i, j + 1), 0.),
                C::new(s.get(i, j + 2), 0.),
            ]
        };
        let bb = r(0);
        let aa = r(3);
        if aa[0].re == 0. {
            return Err("SOS denominator leading coefficient must be nonzero".into());
        }
        let section = tf2zpk(&bb.map(|x| x.re), &aa.map(|x| x.re))?;
        k *= section.gain;
        z.extend(section.zeros);
        p.extend(section.poles);
    }
    Ok(Zpk {
        zeros: z,
        poles: p,
        gain: k,
    })
}
pub fn sos2tf(s: &Matrix) -> Result<(Vec<f64>, Vec<f64>)> {
    let z = sos2zpk(s)?;
    zpk2tf(&z)
}
/// Digital `b,a` as zeros, poles and gain. Coefficients are powers of
/// `z^{-1}`, so a shorter numerator is zeros at the origin, not a delay.
/// `zp2sos` still sheds a positive-power degree deficit (no zeros at 0).
pub(crate) fn digital_tf_zpk(b: &[f64], a: &[f64]) -> Result<Zpk> {
    let mut z = tf2zpk(b, a)?;
    let extra = a.len().saturating_sub(b.len());
    z.zeros.extend(std::iter::repeat_n(C::new(0., 0.), extra));
    Ok(z)
}
pub fn tf2sos(b: &[f64], a: &[f64]) -> Result<Matrix> {
    zpk2sos(&digital_tf_zpk(b, a)?)
}
pub fn zpk2ss(z: &Zpk) -> Result<(Matrix, Matrix, Matrix, Matrix)> {
    let (b, a) = zpk2tf(z)?;
    tf2ss(&b, &a)
}
pub fn tf2ss(b: &[f64], a: &[f64]) -> Result<(Matrix, Matrix, Matrix, Matrix)> {
    if a.is_empty() || b.is_empty() || a[0] == 0. || b.len() > a.len() || a.len() > 41 {
        return Err("Invalid SISO transfer function".into());
    }
    let n = a.len() - 1;
    if n > 40 {
        return Err("State-space order exceeds 40".into());
    }
    let aa = a.iter().map(|x| x / a[0]).collect::<Vec<_>>();
    let bb = b.iter().map(|x| x / a[0]).collect::<Vec<_>>();
    let m = bb.len().max(aa.len());
    let mut bpad = vec![0.; m];
    let mut apad = vec![0.; m];
    for (i, x) in bb.iter().enumerate() {
        bpad[m - bb.len() + i] = *x
    }
    for (i, x) in aa.iter().enumerate() {
        apad[m - aa.len() + i] = *x
    }
    let n = m - 1;
    let mut amat = vec![0.; n * n];
    for j in 0..n {
        amat[j * n] = -apad[j + 1]
    }
    for i in 1..n {
        amat[(i - 1) * n + i] = 1.
    }
    let mut bmat = vec![0.; n];
    if n > 0 {
        bmat[0] = 1.
    }
    let d = bpad[0];
    let c = (0..n)
        .map(|i| bpad[i + 1] - d * apad[i + 1])
        .collect::<Vec<_>>();
    Ok((
        Matrix::from_column_vec(n, n, amat)?,
        Matrix::from_column_vec(n, 1, bmat)?,
        Matrix::from_column_vec(1, n, c)?,
        Matrix::scalar(d),
    ))
}
pub fn ss2tf(a: &Matrix, b: &Matrix, c: &Matrix, d: &Matrix) -> Result<(Vec<f64>, Vec<f64>)> {
    if a.rows() != a.cols()
        || b.rows() != a.rows()
        || b.cols() != 1
        || c.rows() != 1
        || c.cols() != a.rows()
        || d.len() != 1
    {
        return Err("State-space inputs must form a SISO A,B,C,D system".into());
    }
    let n = a.rows();
    if n > MAX_DESIGN_ORDER * 2 {
        return Err("State-space order exceeds 40".into());
    }
    if n == 0 {
        return Ok((vec![d.data()[0]], vec![1.]));
    }
    let mat = DMatrix::<f64>::from_column_slice(n, n, a.data());
    let poles = Schur::try_new(mat.clone(), f64::EPSILON, 0)
        .ok_or("State-space eigenvalue solve did not converge")?
        .complex_eigenvalues()
        .iter()
        .copied()
        .collect::<Vec<_>>();
    let den = real_vec(poly(&poles, 1.), "State-space denominator")?;
    let mut state = vec![0.; n];
    for (i, value) in state.iter_mut().enumerate() {
        *value = b.get(i, 0)
    }
    let mut h = vec![d.data()[0]];
    let mut cur = state;
    for _ in 0..n {
        let y = (0..n).map(|i| c.get(0, i) * cur[i]).sum();
        h.push(y);
        let mut next = vec![0.; n];
        for (i, value) in next.iter_mut().enumerate() {
            for (j, x) in cur.iter().enumerate() {
                *value += a.get(i, j) * x
            }
        }
        cur = next;
    }
    let num = (0..=n)
        .map(|k| h[k] + (1..=k.min(n)).map(|i| den[i] * h[k - i]).sum::<f64>())
        .collect();
    Ok((num, den))
}
pub fn ss2zpk(a: &Matrix, b: &Matrix, c: &Matrix, d: &Matrix) -> Result<Zpk> {
    let (x, y) = ss2tf(a, b, c, d)?;
    tf2zpk(&x, &y)
}
pub fn zpk2tf_matrix(z: &Zpk) -> Result<(Matrix, Matrix)> {
    let (b, a) = zpk2tf(z)?;
    Ok((
        Matrix::from_column_vec(1, b.len(), b)?,
        Matrix::from_column_vec(1, a.len(), a)?,
    ))
}

fn infer_band(wp: &[f64], ws: &[f64]) -> IirBand {
    match (wp.len(), ws.len()) {
        (1, 1) if wp[0] > ws[0] => IirBand::High,
        (1, 1) => IirBand::Low,
        (2, 2) if ws[0] < wp[0] && ws[1] > wp[1] => IirBand::Bandpass,
        (2, 2) => IirBand::Stop,
        _ => IirBand::Low,
    }
}
fn cheby2_pass_edge(
    n: usize,
    rp: f64,
    rs: f64,
    math: &dyn EllipticMath,
    c: &AtomicBool,
) -> Result<f64> {
    let prototype = design(
        DesignSpec {
            kind: IirKind::Cheby2,
            order: n,
            rp,
            rs,
            edges: &[1.],
            band: IirBand::Low,
            analog: true,
        },
        math,
        c,
    )?;
    let magnitude = |w: f64| {
        let s = C::new(0., w);
        let numerator = prototype
            .zeros
            .iter()
            .fold(C::new(prototype.gain, 0.), |v, z| v * (s - z));
        let denominator = prototype
            .poles
            .iter()
            .fold(C::new(1., 0.), |v, p| v * (s - p));
        (numerator / denominator).norm()
    };
    let target = 10f64.powf(-rp / 20.);
    let (mut lo, mut hi) = (0., 1.);
    for _ in 0..80 {
        let mid = (lo + hi) * 0.5;
        if magnitude(mid) > target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Ok((lo + hi) * 0.5)
}

/// Select an order and critical frequencies independently of the script engine.
pub struct OrderSpec<'a> {
    pub name: &'a str,
    pub wp: &'a [f64],
    pub ws: &'a [f64],
    pub rp: f64,
    pub rs: f64,
    pub analog: bool,
}
pub fn select_order(
    spec: OrderSpec<'_>,
    math: &dyn EllipticMath,
    c: &AtomicBool,
) -> Result<(usize, Vec<f64>)> {
    let OrderSpec {
        name,
        wp,
        ws,
        rp,
        rs,
        analog,
    } = spec;
    cancel(c)?;
    let band = infer_band(wp, ws);
    if !rp.is_finite() || !rs.is_finite() || rp <= 0. || rs <= rp {
        return Err("Order selection requires 0 < Rp < Rs".into());
    }
    if wp.len() != ws.len()
        || wp.len()
            != if matches!(band, IirBand::Bandpass | IirBand::Stop) {
                2
            } else {
                1
            }
    {
        return Err("Passband and stopband edges must have matching one/two element shapes".into());
    }
    validate_edges(wp, band, analog)?;
    validate_edges(ws, band, analog)?;
    let scale = |v: f64| {
        if analog {
            Ok(v)
        } else {
            if !(0. < v && v < 1.) {
                return Err("Digital edges must lie strictly between zero and Nyquist".into());
            }
            Ok(2. * (std::f64::consts::PI * v / 2.).tan())
        }
    };
    let p = wp.iter().map(|v| scale(*v)).collect::<Result<Vec<_>>>()?;
    let s = ws.iter().map(|v| scale(*v)).collect::<Result<Vec<_>>>()?;
    let ep = (10f64.powf(rp / 10.) - 1.).sqrt();
    let es = (10f64.powf(rs / 10.) - 1.).sqrt();
    let ratio = match band {
        IirBand::Low => s[0] / p[0],
        IirBand::High => p[0] / s[0],
        IirBand::Bandpass => {
            let w0 = (p[0] * p[1]).sqrt();
            s.iter()
                .map(|x| ((x * x - w0 * w0) / ((p[1] - p[0]) * x)).abs())
                .fold(f64::INFINITY, f64::min)
        }
        IirBand::Stop => {
            let w0 = (s[0] * s[1]).sqrt();
            let bw = s[1] - s[0];
            1. / p
                .iter()
                .map(|x| (bw * x / (w0 * w0 - x * x)).abs())
                .fold(0., f64::max)
        }
    };
    if ratio <= 1. || !ratio.is_finite() {
        return Err("Stopband must lie outside the passband".into());
    }
    let n = match name {
        "buttord" => ((es / ep).ln() / ratio.ln()).ceil() as usize,
        "cheb1ord" | "cheb2ord" => ((es / ep).acosh() / ratio.acosh()).ceil() as usize,
        "ellipord" => elliptic_order(ratio, ep / es, math)?,
        _ => return Err("Unknown IIR order-selection family".into()),
    };
    if !(1..=MAX_DESIGN_ORDER).contains(&n) {
        return Err("Order selection requires a prototype order in 1..100".into());
    }
    let cheb2_pass = if name == "cheb2ord" {
        Some(cheby2_pass_edge(n, rp, rs, math, c)?)
    } else {
        None
    };
    let factor = match name {
        "buttord" => ep.powf(-1. / n as f64),
        _ => 1.,
    };
    let mut wn = match band {
        IirBand::Low => vec![if let Some(pass) = cheb2_pass {
            p[0] / pass
        } else {
            p[0] * factor
        }],
        IirBand::High => vec![if let Some(pass) = cheb2_pass {
            p[0] * pass
        } else {
            p[0] / factor
        }],
        IirBand::Bandpass => {
            let w0 = (p[0] * p[1]).sqrt();
            let pass = cheb2_pass.unwrap_or(1. / factor);
            let bw = (w0 * w0 - p[0] * p[0]).abs() / (pass * p[0]);
            let half = bw / 2.;
            vec![
                (w0 * w0 + half * half).sqrt() - half,
                (w0 * w0 + half * half).sqrt() + half,
            ]
        }
        IirBand::Stop => {
            let w0 = (s[0] * s[1]).sqrt();
            let bw = {
                let transformed = |x: f64| ((s[1] - s[0]) * x / (w0 * w0 - x * x)).abs();
                let pass_edge = p
                    .iter()
                    .max_by(|x, y| transformed(**x).total_cmp(&transformed(**y)))
                    .copied()
                    .unwrap();
                let pass_target =
                    cheb2_pass.unwrap_or_else(|| if name == "buttord" { 1. / factor } else { 1. });
                pass_target * (w0 * w0 - pass_edge * pass_edge).abs() / pass_edge
            };
            let low = (-bw + (bw * bw + 4. * w0 * w0).sqrt()) / 2.;
            let high = (bw + (bw * bw + 4. * w0 * w0).sqrt()) / 2.;
            vec![low, high]
        }
    };
    if !analog {
        wn.iter_mut()
            .for_each(|v| *v = 2. * (*v / 2.).atan() / std::f64::consts::PI);
    }
    cancel(c)?;
    Ok((n, wn))
}
