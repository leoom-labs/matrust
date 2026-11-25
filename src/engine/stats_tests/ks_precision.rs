//! Compensated MTW evaluation when subtracting a CDF from one would lose
//! relative tail accuracy. Error-free transforms: Knuth, TAOCP vol. 2,
//! sec. 4.2.2; fused product residual. Same full MTW matrix, no tail shortcut.
//! The ordinary path uses the existing dense GEMM kernel. This precision
//! fallback keeps a floating-point expansion because GEMM's f64 output cannot retain
//! the residual needed for a small complementary probability.
use super::*;
use crate::engine::Fma;
#[derive(Clone, Copy)]
struct D<const N: usize>([f64; N]);
impl<const N: usize> Default for D<N> {
    fn default() -> Self {
        Self([0.; N])
    }
}
fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let s = a + b;
    let v = s - a;
    (s, (a - (s - v)) + (b - v))
}
impl<const N: usize> D<N> {
    fn from(x: f64) -> Self {
        let mut a = [0.; N];
        a[0] = x;
        Self(a)
    }
    fn hi(self) -> f64 {
        self.0[0]
    }
    fn value(self) -> f64 {
        self.0.into_iter().sum()
    }
    fn normalized(mut self) -> Self {
        for _ in 0..2 {
            for i in (0..N - 1).rev() {
                let (s, e) = two_sum(self.0[i], self.0[i + 1]);
                self.0[i] = s;
                self.0[i + 1] = e;
            }
        }
        self
    }
    fn add_scalar(mut self, mut x: f64) -> Self {
        for a in &mut self.0 {
            let (s, e) = two_sum(*a, x);
            *a = s;
            x = e;
        }
        self
    }
    fn add(self, b: Self) -> Self {
        if N == 2 {
            let (s, e) = two_sum(self.0[0], b.0[0]);
            let e = e + self.0[1] + b.0[1];
            let hi = s + e;
            let mut a = [0.; N];
            a[0] = hi;
            a[1] = e - (hi - s);
            Self(a)
        } else {
            let mut out = self;
            for v in b.0 {
                out = out.add_scalar(v)
            }
            out.normalized()
        }
    }
    fn neg(mut self) -> Self {
        for a in &mut self.0 {
            *a = -*a;
        }
        self
    }
    fn mul(self, b: Self) -> Self {
        if N == 2 {
            let p = self.0[0] * b.0[0];
            let e = self.0[0].fma(b.0[0], -p)
                + self.0[0] * b.0[1]
                + self.0[1] * b.0[0]
                + self.0[1] * b.0[1];
            let hi = p + e;
            let mut a = [0.; N];
            a[0] = hi;
            a[1] = e - (hi - p);
            Self(a)
        } else {
            let mut out = Self::default();
            for i in 0..N {
                for j in 0..N - i {
                    let p = self.0[i] * b.0[j];
                    let residual = self.0[i].fma(b.0[j], -p);
                    out = out.add_scalar(p).add_scalar(residual);
                }
            }
            out.normalized()
        }
    }
    fn div(self, b: f64) -> Self {
        let mut out = Self::from(self.hi() / b);
        let mut r = self.add(out.mul(Self::from(b)).neg());
        for _ in 1..N {
            let q = Self::from(r.hi() / b);
            out = out.add(q);
            r = r.add(q.mul(Self::from(b)).neg());
        }
        out
    }
    fn pow(self, n: usize) -> Self {
        let mut x = Self::from(1.);
        for _ in 0..n {
            x = x.mul(self)
        }
        x
    }
    fn scale(mut self, s: f64) -> Self {
        for a in &mut self.0 {
            *a *= s;
        }
        self
    }
}
struct Dense<const N: usize> {
    m: usize,
    data: Vec<D<N>>,
    scale: i32,
}
fn product<const N: usize>(e: &Engine, a: &Dense<N>, b: &Dense<N>) -> Result<Dense<N>> {
    let m = a.m;
    let mut c = Dense {
        m,
        data: vec![D::default(); m * m],
        scale: a.scale + b.scale,
    };
    // Precision-specific compensated dots; bounded chunks make cancellation
    // responsive even at the maximum permitted matrix order.
    let cancel = &e.cancel;
    let fill = |(j, col): (usize, &mut [D<N>])| -> Result<()> {
        for (i, slot) in col.iter_mut().enumerate() {
            crate::dsp::check_cancel(cancel)?;
            let mut s = D::default();
            for k in 0..m {
                if k.is_multiple_of(256) {
                    crate::dsp::check_cancel(cancel)?;
                }
                s = s.add(a.data[i + k * m].mul(b.data[k + j * m]));
            }
            *slot = s;
        }
        Ok(())
    };
    // Existing Rayon pool only; no pool nesting, shared mutable sums or locks.
    // Eight million products amortize column scheduling on the benchmark host.
    if m * m * m >= 8_000_000 && rayon::current_num_threads() > 1 {
        use rayon::prelude::*;
        c.data.par_chunks_mut(m).enumerate().try_for_each(fill)?;
    } else {
        c.data.chunks_mut(m).enumerate().try_for_each(fill)?;
    }
    while c.data.iter().any(|x| x.hi().abs() > 2f64.powi(256)) {
        for (i, x) in c.data.iter_mut().enumerate() {
            poll(e, i)?;
            *x = x.scale(2f64.powi(-256));
        }
        c.scale += 256;
    }
    Ok(c)
}
pub(super) fn sf<const N: usize>(e: &Engine, n: usize, d: f64) -> Result<f64> {
    let nf = n as f64;
    let k = (nf * d).floor() as usize + 1;
    let m = 2 * k - 1;
    let h = D::<N>::from(k as f64).add(D::from(nf).mul(D::from(d)).neg());
    let mut invfact = vec![D::from(1.); m + 1];
    for i in 1..=m {
        invfact[i] = invfact[i - 1].div(i as f64);
    }
    let mut data = vec![D::default(); m * m];
    for j in 0..m {
        for i in 0..m {
            poll(e, i + j * m)?;
            if i + 1 >= j {
                data[i + j * m] = invfact[i + 1 - j]
            }
        }
    }
    for i in 0..m {
        data[i] = data[i].add(h.pow(i + 1).mul(invfact[i + 1]).neg());
        data[m - 1 + i * m] = data[m - 1 + i * m].add(h.pow(m - i).mul(invfact[m - i]).neg());
    }
    if h.hi() > 0.5 {
        data[m - 1] = data[m - 1].add(h.scale(2.).add(D::from(-1.)).pow(m).mul(invfact[m]));
    }
    let mut base = Dense { m, data, scale: 0 };
    let mut result: Option<Dense<N>> = None;
    let mut power = n;
    while power > 0 {
        e.check_cancel()?;
        if power & 1 == 1 {
            result = Some(if let Some(r) = result {
                product(e, &r, &base)?
            } else {
                Dense {
                    m,
                    data: base.data.clone(),
                    scale: base.scale,
                }
            })
        }
        power >>= 1;
        if power > 0 {
            base = product(e, &base, &base)?
        }
    }
    let r = result.unwrap();
    let mut p = r.data[k - 1 + (k - 1) * m];
    let mut scale = r.scale;
    for i in 1..=n {
        poll(e, i)?;
        p = p.mul(D::from(i as f64).div(nf));
        if p.hi().abs() < 2f64.powi(-256) && p.hi() != 0. {
            p = p.scale(2f64.powi(256));
            scale -= 256;
        }
    }
    p = p.scale(2f64.powi(scale));
    let q = D::from(1.).add(p.neg());
    Ok(q.value().clamp(0., 1.))
}
