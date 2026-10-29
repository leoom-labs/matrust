//! Complex dense eigenvalue kernels, ported from reference LAPACK 3.5 in
//! unblocked form: zgebal, zgehd2, zunghr, zlahqr, ztrevc (scaled triangular solve),
//! zgebak, the zgeevx normalisation, and zheev (zhetd2, zungtr, zsteqr).
//! 0-based, column-major. As in the real kernels, axpy-style updates are
//! fused (`madd`) like MATLAB's MKL; dot products are not.
// LAPACK ports keep the reference routines' index loops.
#![allow(clippy::needless_range_loop)]
use super::dense::{self, EPS, Mat, SAFMIN, ULP, ladiv};
use crate::Complex64 as C;
use crate::engine::Fma;

const ZERO: C = C { re: 0., im: 0. };
const ONE: C = C { re: 1., im: 0. };

#[inline]
pub(crate) fn c(re: f64, im: f64) -> C {
    C { re, im }
}
#[inline]
pub(crate) fn cabs1(z: C) -> f64 {
    z.re.abs() + z.im.abs()
}
#[inline]
pub(crate) fn cabs(z: C) -> f64 {
    z.re.hypot(z.im)
}
#[inline]
pub(crate) fn cdivr(z: C, r: f64) -> C {
    c(z.re / r, z.im / r)
}
#[inline]
pub(crate) fn cmulr(z: C, r: f64) -> C {
    c(z.re * r, z.im * r)
}
/// zladiv.
#[inline]
pub(crate) fn zladiv(x: C, y: C) -> C {
    let (p, q) = ladiv(x.re, x.im, y.re, y.im);
    c(p, q)
}
/// Fortran complex division (Smith's algorithm).
#[inline]
pub(crate) fn cdiv(x: C, y: C) -> C {
    let (a, b, cc, d) = (x.re, x.im, y.re, y.im);
    let size = cc.abs().max(d.abs());
    if !(f64::MIN_POSITIVE.sqrt()..=f64::MAX.sqrt() * 0.5).contains(&size)
        || a.abs().max(b.abs()) > f64::MAX * 0.25
    {
        return zladiv(x, y);
    }
    if d.abs() < cc.abs() {
        let e = d / cc;
        let f = cc + d * e;
        c((a + b * e) / f, (b - a * e) / f)
    } else {
        let e = cc / d;
        let f = d + cc * e;
        c((b + a * e) / f, (-a + b * e) / f)
    }
}
/// y + a*x as MKL's complex axpy-style kernels compute it (fused).
#[inline]
pub(crate) fn madd(y: C, a: C, x: C) -> C {
    c(
        a.re.fma(x.re, (-a.im).fma(x.im, y.re)),
        a.re.fma(x.im, a.im.fma(x.re, y.im)),
    )
}
/// x / y through the reciprocal of y, as MKL's ztrsv divides.
#[inline]
pub(crate) fn cdiv_recip(x: C, y: C) -> C {
    let size = y.re.abs().max(y.im.abs());
    if !(f64::MIN_POSITIVE.sqrt()..=f64::MAX.sqrt() * 0.5).contains(&size)
        || x.re.abs().max(x.im.abs()) > f64::MAX * 0.25
    {
        return zladiv(x, y);
    }
    let d = 1. / (y.re * y.re + y.im * y.im);
    x * c(y.re * d, -y.im * d)
}
/// Principal complex square root.
pub(crate) fn csqrt(z: C) -> C {
    if z.re == 0. && z.im == 0. {
        return c(0., z.im);
    }
    let t = ((z.re.abs() + cabs(z)) / 2.).sqrt();
    if z.re >= 0. {
        c(t, z.im / (2. * t))
    } else {
        c(z.im.abs() / (2. * t), t.copysign(z.im))
    }
}

/// Dense column-major complex matrix.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CMat {
    pub rows: usize,
    pub cols: usize,
    pub data: Vec<C>,
}

impl CMat {
    pub fn zeros(rows: usize, cols: usize) -> Self {
        Self {
            rows,
            cols,
            data: vec![ZERO; rows * cols],
        }
    }
    pub fn identity(n: usize) -> Self {
        let mut m = Self::zeros(n, n);
        for i in 0..n {
            m.data[i + i * n] = ONE;
        }
        m
    }
    pub fn from_real(a: &Mat) -> Self {
        Self {
            rows: a.rows,
            cols: a.cols,
            data: a.data.iter().map(|&v| c(v, 0.)).collect(),
        }
    }
    #[inline]
    pub fn at(&self, i: usize, j: usize) -> C {
        self.data[i + j * self.rows]
    }
    #[inline]
    pub fn set(&mut self, i: usize, j: usize, v: C) {
        self.data[i + j * self.rows] = v;
    }
    #[inline]
    pub fn get_mut(&mut self, i: usize, j: usize) -> &mut C {
        &mut self.data[i + j * self.rows]
    }
    pub fn col(&self, j: usize) -> &[C] {
        &self.data[j * self.rows..(j + 1) * self.rows]
    }
    pub fn swap_rows(&mut self, a: usize, b: usize, from_col: usize, to_col: usize) {
        for j in from_col..to_col {
            self.data.swap(a + j * self.rows, b + j * self.rows);
        }
    }
    pub fn swap_cols(&mut self, a: usize, b: usize, from_row: usize, to_row: usize) {
        for i in from_row..to_row {
            self.data.swap(i + a * self.rows, i + b * self.rows);
        }
    }
    pub fn adjoint(&self) -> Self {
        let mut t = Self::zeros(self.cols, self.rows);
        for j in 0..self.cols {
            for i in 0..self.rows {
                t.set(j, i, self.at(i, j).conj());
            }
        }
        t
    }
    /// Reference zgemm order: C = A*B.
    pub fn mul(&self, b: &CMat, cancel: &dyn Fn() -> bool) -> CMat {
        let mut out = CMat::zeros(self.rows, b.cols);
        for j in 0..b.cols {
            if cancel() {
                return out;
            }
            for l in 0..self.cols {
                let t = b.at(l, j);
                if t != ZERO {
                    for i in 0..self.rows {
                        out.data[i + j * out.rows] += t * self.at(i, l);
                    }
                }
            }
        }
        out
    }
    pub fn re(&self) -> Mat {
        Mat::from_vec(
            self.rows,
            self.cols,
            self.data.iter().map(|z| z.re).collect(),
        )
    }
    pub fn im(&self) -> Mat {
        Mat::from_vec(
            self.rows,
            self.cols,
            self.data.iter().map(|z| z.im).collect(),
        )
    }
    pub fn from_parts(re: &Mat, im: &Mat) -> Self {
        Self {
            rows: re.rows,
            cols: re.cols,
            data: re
                .data
                .iter()
                .zip(&im.data)
                .map(|(&a, &b)| c(a, b))
                .collect(),
        }
    }
}

/// dznrm2: the reference scaled sum of squares, updates fused (MKL).
pub(crate) fn znrm2<'a>(x: impl IntoIterator<Item = &'a C>) -> f64 {
    let mut scale = 0f64;
    let mut ssq = 1f64;
    for z in x {
        for v in [z.re, z.im] {
            if v != 0. {
                let t = v.abs();
                if scale < t {
                    let r = scale / t;
                    ssq = (ssq * r).fma(r, 1.);
                    scale = t;
                } else {
                    let r = t / scale;
                    ssq = r.fma(r, ssq);
                }
            }
        }
    }
    scale * ssq.sqrt()
}

pub(crate) fn izamax<'a>(x: impl IntoIterator<Item = &'a C>) -> usize {
    let mut best = 0;
    let mut max = -1f64;
    for (i, &v) in x.into_iter().enumerate() {
        if cabs1(v) > max {
            max = cabs1(v);
            best = i;
        }
    }
    best
}

fn lapy3(x: f64, y: f64, z: f64) -> f64 {
    let (xa, ya, za) = (x.abs(), y.abs(), z.abs());
    let w = xa.max(ya).max(za);
    if w == 0. {
        xa + ya + za
    } else {
        w * ((xa / w) * (xa / w) + (ya / w) * (ya / w) + (za / w) * (za / w)).sqrt()
    }
}

/// zlarfg on (alpha, x): returns (beta as complex, tau) and scales x.
pub(crate) fn zlarfg(alpha: C, x: &mut [C]) -> (C, C) {
    let mut xnorm = znrm2(x.iter());
    let mut alphr = alpha.re;
    let mut alphi = alpha.im;
    if xnorm == 0. && alphi == 0. {
        return (alpha, ZERO);
    }
    let mut beta = -dense::sign(lapy3(alphr, alphi, xnorm), alphr);
    let safmin = SAFMIN / EPS;
    let rsafmn = 1. / safmin;
    let mut knt = 0;
    let mut alpha = alpha;
    if beta.abs() < safmin {
        loop {
            knt += 1;
            for v in x.iter_mut() {
                *v = c(rsafmn, 0.) * *v;
            }
            beta *= rsafmn;
            alphi *= rsafmn;
            alphr *= rsafmn;
            if beta.abs() >= safmin || knt >= 20 {
                break;
            }
        }
        xnorm = znrm2(x.iter());
        alpha = c(alphr, alphi);
        beta = -dense::sign(lapy3(alphr, alphi, xnorm), alphr);
    }
    let tau = c((beta - alphr) / beta, -alphi / beta);
    let scal = zladiv(ONE, alpha - c(beta, 0.));
    for v in x.iter_mut() {
        *v = scal * *v;
    }
    for _ in 0..knt {
        beta *= safmin;
    }
    (c(beta, 0.), tau)
}

/// zlarf 'Left': C(r0.., c0..c0+n) := H C, H = I - tau v v^H.
pub(crate) fn zlarf_left(cm: &mut CMat, v: &[C], tau: C, r0: usize, c0: usize, n: usize) {
    if tau == ZERO {
        return;
    }
    let mut lastv = v.len();
    while lastv > 0 && v[lastv - 1] == ZERO {
        lastv -= 1;
    }
    let mut lastc = n;
    while lastc > 0 && (0..lastv).all(|i| cm.at(r0 + i, c0 + lastc - 1) == ZERO) {
        lastc -= 1;
    }
    if lastv == 0 || lastc == 0 {
        return;
    }
    let mut w = vec![ZERO; lastc];
    for (j, wj) in w.iter_mut().enumerate() {
        let mut t = ZERO;
        for (i, &vi) in v.iter().enumerate().take(lastv) {
            t += cm.at(r0 + i, c0 + j).conj() * vi;
        }
        *wj = ONE * t;
    }
    let alpha = -tau;
    for (j, &wj) in w.iter().enumerate() {
        if wj != ZERO {
            let t = alpha * wj.conj();
            for (i, &vi) in v.iter().enumerate().take(lastv) {
                let y = cm.at(r0 + i, c0 + j);
                cm.set(r0 + i, c0 + j, madd(y, vi, t));
            }
        }
    }
}

/// zlarf 'Right': C(r0..r0+m, c0..) := C H.
pub(crate) fn zlarf_right(cm: &mut CMat, v: &[C], tau: C, r0: usize, c0: usize, m: usize) {
    if tau == ZERO {
        return;
    }
    let mut lastv = v.len();
    while lastv > 0 && v[lastv - 1] == ZERO {
        lastv -= 1;
    }
    let mut lastc = m;
    while lastc > 0 && (0..lastv).all(|j| cm.at(r0 + lastc - 1, c0 + j) == ZERO) {
        lastc -= 1;
    }
    if lastv == 0 || lastc == 0 {
        return;
    }
    let mut w = vec![ZERO; lastc];
    for (j, &vj) in v.iter().enumerate().take(lastv) {
        if vj != ZERO {
            let t = ONE * vj;
            for (i, wi) in w.iter_mut().enumerate() {
                *wi = madd(*wi, t, cm.at(r0 + i, c0 + j));
            }
        }
    }
    let alpha = -tau;
    for (j, &vj) in v.iter().enumerate().take(lastv) {
        if vj != ZERO {
            let t = alpha * vj.conj();
            for (i, &wi) in w.iter().enumerate() {
                let y = cm.at(r0 + i, c0 + j);
                cm.set(r0 + i, c0 + j, madd(y, wi, t));
            }
        }
    }
}

/// zgebal (LAPACK 3.4 off-diagonal 1-norms); scale entries for permutations are 0-based indices.
pub(crate) fn zgebal(a: &mut CMat, permute: bool, scale_it: bool) -> (usize, usize, Vec<f64>) {
    let n = a.rows;
    let mut scale = vec![1.; n];
    if n == 0 {
        return (0, 0, scale);
    }
    if !permute && !scale_it {
        return (0, n - 1, scale);
    }
    let mut k = 0usize;
    let mut l = n - 1;
    if permute {
        'rows: loop {
            let mut found = None;
            for j in (0..=l).rev() {
                if (0..=l).all(|i| i == j || a.at(j, i) == ZERO) {
                    found = Some(j);
                    break;
                }
            }
            let Some(j) = found else { break 'rows };
            scale[l] = j as f64;
            if j != l {
                a.swap_cols(j, l, 0, l + 1);
                a.swap_rows(j, l, k, n);
            }
            if l == 0 {
                return (0, 0, scale);
            }
            l -= 1;
        }
        loop {
            let mut found = None;
            for j in k..=l {
                if (k..=l).all(|i| i == j || a.at(i, j) == ZERO) {
                    found = Some(j);
                    break;
                }
            }
            let Some(j) = found else { break };
            scale[k] = j as f64;
            if j != k {
                a.swap_cols(j, k, 0, l + 1);
                a.swap_rows(j, k, k, n);
            }
            k += 1;
        }
    }
    for s in scale.iter_mut().take(l + 1).skip(k) {
        *s = 1.;
    }
    if !scale_it {
        return (k, l, scale);
    }
    const SCLFAC: f64 = 2.;
    const FACTOR: f64 = 0.95;
    let sfmin1 = SAFMIN / ULP;
    let sfmax1 = 1. / sfmin1;
    let sfmin2 = sfmin1 * SCLFAC;
    let sfmax2 = 1. / sfmin2;
    loop {
        let mut noconv = false;
        for i in k..=l {
            let mut cn = 0.;
            let mut r = 0.;
            for j in k..=l {
                if j != i {
                    cn += cabs1(a.at(j, i));
                    r += cabs1(a.at(i, j));
                }
            }
            let ica = izamax((0..=l).map(|r| &a.data[r + i * n]));
            let mut ca = cabs(a.at(ica, i));
            let ira = izamax((k..n).map(|j| &a.data[i + j * n]));
            let mut ra = cabs(a.at(i, ira + k));
            if cn == 0. || r == 0. {
                continue;
            }
            let (mut cc, mut r) = (cn, r);
            let mut g = r / SCLFAC;
            let mut f = 1f64;
            let s = cc + r;
            while !(cc >= g || f.max(cc).max(ca) >= sfmax2 || r.min(g).min(ra) <= sfmin2) {
                if (cc + f + ca + r + g + ra).is_nan() {
                    return (k, l, scale);
                }
                f *= SCLFAC;
                cc *= SCLFAC;
                ca *= SCLFAC;
                r /= SCLFAC;
                g /= SCLFAC;
                ra /= SCLFAC;
            }
            g = cc / SCLFAC;
            while !(g < r || r.max(ra) >= sfmax2 || f.min(cc).min(g).min(ca) <= sfmin2) {
                f /= SCLFAC;
                cc /= SCLFAC;
                g /= SCLFAC;
                ca /= SCLFAC;
                r *= SCLFAC;
                ra *= SCLFAC;
            }
            if cc + r >= FACTOR * s {
                continue;
            }
            if f < 1. && scale[i] < 1. && f * scale[i] <= sfmin1 {
                continue;
            }
            if f > 1. && scale[i] > 1. && scale[i] >= sfmax1 / f {
                continue;
            }
            let g = 1. / f;
            scale[i] *= f;
            noconv = true;
            for j in k..n {
                let v = a.at(i, j);
                a.set(i, j, c(g, 0.) * v);
            }
            for j in 0..=l {
                let v = a.at(j, i);
                a.set(j, i, c(f, 0.) * v);
            }
        }
        if !noconv {
            break;
        }
    }
    (k, l, scale)
}

pub(crate) fn zgebak(
    v: &mut CMat,
    ilo: usize,
    ihi: usize,
    scale: &[f64],
    scaled: bool,
    permuted: bool,
    right: bool,
) {
    let n = v.rows;
    if n == 0 {
        return;
    }
    let m = v.cols;
    if ilo != ihi && scaled {
        for (i, &s) in scale.iter().enumerate().take(ihi + 1).skip(ilo) {
            let s = if right { s } else { 1. / s };
            for j in 0..m {
                let z = v.at(i, j);
                v.set(i, j, c(s, 0.) * z);
            }
        }
    }
    if permuted {
        for ii in 0..n {
            let mut i = ii;
            if i >= ilo && i <= ihi {
                continue;
            }
            if i < ilo {
                i = ilo - 1 - ii;
            }
            let k = scale[i] as usize;
            if k != i {
                v.swap_rows(i, k, 0, m);
            }
        }
    }
}

/// zgehd2 on ilo..=ihi; returns tau.
pub(crate) fn zgehd2(
    a: &mut CMat,
    ilo: usize,
    ihi: usize,
    cancel: &dyn Fn() -> bool,
) -> Result<Vec<C>, usize> {
    let n = a.rows;
    let mut tau = vec![ZERO; n.saturating_sub(1)];
    for i in ilo..ihi {
        if cancel() {
            return Err(i + 1);
        }
        let alpha = a.at(i + 1, i);
        let mut x: Vec<C> = (i + 2..=ihi).map(|r| a.at(r, i)).collect();
        let (beta, t) = zlarfg(alpha, &mut x);
        tau[i] = t;
        for (k, &xv) in x.iter().enumerate() {
            a.set(i + 2 + k, i, xv);
        }
        let mut v = Vec::with_capacity(x.len() + 1);
        v.push(ONE);
        v.extend_from_slice(&x);
        zlarf_right(a, &v, t, 0, i + 1, ihi + 1);
        zlarf_left(a, &v, t.conj(), i + 1, i + 1, n - i - 1);
        a.set(i + 1, i, beta);
    }
    Ok(tau)
}

/// zung2r on the m x n block at (r0, c0) with k reflectors.
#[allow(clippy::too_many_arguments)]
pub(crate) fn zung2r(
    q: &mut CMat,
    k: usize,
    tau: &[C],
    r0: usize,
    c0: usize,
    m: usize,
    n: usize,
    cancel: &dyn Fn() -> bool,
) {
    for j in k..n {
        if cancel() {
            return;
        }
        for l in 0..m {
            q.set(r0 + l, c0 + j, ZERO);
        }
        q.set(r0 + j, c0 + j, ONE);
    }
    for i in (0..k).rev() {
        if cancel() {
            return;
        }
        if i + 1 < n {
            q.set(r0 + i, c0 + i, ONE);
            let v: Vec<C> = (i..m).map(|r| q.at(r0 + r, c0 + i)).collect();
            zlarf_left(q, &v, tau[i], r0 + i, c0 + i + 1, n - i - 1);
        }
        if i + 1 < m {
            for r in i + 1..m {
                let z = q.at(r0 + r, c0 + i);
                q.set(r0 + r, c0 + i, -tau[i] * z);
            }
        }
        q.set(r0 + i, c0 + i, ONE - tau[i]);
        for l in 0..i {
            q.set(r0 + l, c0 + i, ZERO);
        }
    }
}

/// zung2l on the leading m x n block with k reflectors.
fn zung2l(q: &mut CMat, m: usize, n: usize, k: usize, tau: &[C], cancel: &dyn Fn() -> bool) {
    for j in 0..n - k {
        if cancel() {
            return;
        }
        for l in 0..m {
            q.set(l, j, ZERO);
        }
        q.set(m - n + j, j, ONE);
    }
    for i in 0..k {
        if cancel() {
            return;
        }
        let ii = n - k + i;
        let rows = m - n + ii + 1;
        q.set(rows - 1, ii, ONE);
        let v: Vec<C> = (0..rows).map(|r| q.at(r, ii)).collect();
        zlarf_left(q, &v, tau[i], 0, 0, ii);
        for r in 0..rows - 1 {
            let z = q.at(r, ii);
            q.set(r, ii, -tau[i] * z);
        }
        q.set(rows - 1, ii, ONE - tau[i]);
        for l in rows..m {
            q.set(l, ii, ZERO);
        }
    }
}

pub(crate) fn zunghr(
    a: &CMat,
    ilo: usize,
    ihi: usize,
    tau: &[C],
    cancel: &dyn Fn() -> bool,
) -> CMat {
    let n = a.rows;
    let mut q = a.clone();
    let nh = ihi - ilo;
    for j in (ilo + 1..=ihi).rev() {
        if cancel() {
            return q;
        }
        for i in 0..j {
            q.set(i, j, ZERO);
        }
        for i in j + 1..=ihi {
            q.set(i, j, q.at(i, j - 1));
        }
        for i in ihi + 1..n {
            q.set(i, j, ZERO);
        }
    }
    for j in 0..=ilo {
        if cancel() {
            return q;
        }
        for i in 0..n {
            q.set(i, j, ZERO);
        }
        q.set(j, j, ONE);
    }
    for j in ihi + 1..n {
        if cancel() {
            return q;
        }
        for i in 0..n {
            q.set(i, j, ZERO);
        }
        q.set(j, j, ONE);
    }
    if nh > 0 {
        zung2r(
            &mut q,
            nh,
            &tau[ilo..ilo + nh],
            ilo + 1,
            ilo + 1,
            nh,
            nh,
            cancel,
        );
    }
    q
}

fn zscal_row(h: &mut CMat, row: usize, from: usize, to: usize, s: C) {
    for j in from..to {
        let v = h.at(row, j);
        h.set(row, j, s * v);
    }
}
fn zscal_col(h: &mut CMat, col: usize, from: usize, to: usize, s: C) {
    for i in from..to {
        let v = h.at(i, col);
        h.set(i, col, s * v);
    }
}

/// zlahqr. Eigenvalues into w; z (all rows) updated when given.
pub(crate) fn zlahqr(
    wantt: bool,
    h: &mut CMat,
    ilo: usize,
    ihi: usize,
    w: &mut [C],
    mut z: Option<&mut CMat>,
    cancel: &dyn Fn() -> bool,
) -> Result<(), usize> {
    let n = h.rows;
    if n == 0 {
        return Ok(());
    }
    if ilo == ihi {
        w[ilo] = h.at(ilo, ilo);
        return Ok(());
    }
    for j in ilo..ihi.saturating_sub(2) {
        h.set(j + 2, j, ZERO);
        h.set(j + 3, j, ZERO);
    }
    if ilo + 2 <= ihi {
        h.set(ihi, ihi - 2, ZERO);
    }
    let (jlo, jhi) = if wantt { (0, n - 1) } else { (ilo, ihi) };
    for i in ilo + 1..=ihi {
        if h.at(i, i - 1).im != 0. {
            let sc = cdivr(h.at(i, i - 1), cabs1(h.at(i, i - 1)));
            let sc = cdivr(sc.conj(), cabs(sc));
            let a = cabs(h.at(i, i - 1));
            h.set(i, i - 1, c(a, 0.));
            zscal_row(h, i, i, jhi + 1, sc);
            zscal_col(h, i, jlo, jhi.min(i + 1) + 1, sc.conj());
            if let Some(z) = z.as_deref_mut() {
                zscal_col(z, i, ilo, ihi + 1, sc.conj());
            }
        }
    }
    let nh = ihi - ilo + 1;
    let smlnum = SAFMIN * (nh as f64 / ULP);
    let (mut i1, mut i2) = (0usize, n - 1);
    let itmax = 30 * nh.max(10);
    const DAT1: f64 = 0.75;
    let mut i = ihi as isize;
    while i >= ilo as isize {
        let iu = i as usize;
        let mut l = ilo;
        let mut converged = false;
        for its in 0..=itmax {
            if cancel() {
                return Err(iu + 1);
            }
            let mut k = iu;
            while k > l {
                if cabs1(h.at(k, k - 1)) <= smlnum {
                    break;
                }
                let mut tst = cabs1(h.at(k - 1, k - 1)) + cabs1(h.at(k, k));
                if tst == 0. {
                    if k >= ilo + 2 {
                        tst += h.at(k - 1, k - 2).re.abs();
                    }
                    if k < ihi {
                        tst += h.at(k + 1, k).re.abs();
                    }
                }
                if h.at(k, k - 1).re.abs() <= ULP * tst {
                    let ab = cabs1(h.at(k, k - 1)).max(cabs1(h.at(k - 1, k)));
                    let ba = cabs1(h.at(k, k - 1)).min(cabs1(h.at(k - 1, k)));
                    let aa = cabs1(h.at(k, k)).max(cabs1(h.at(k - 1, k - 1) - h.at(k, k)));
                    let bb = cabs1(h.at(k, k)).min(cabs1(h.at(k - 1, k - 1) - h.at(k, k)));
                    let s = aa + ab;
                    if ba * (ab / s) <= smlnum.max(ULP * (bb * (aa / s))) {
                        break;
                    }
                }
                k -= 1;
            }
            l = k;
            if l > ilo {
                h.set(l, l - 1, ZERO);
            }
            if l >= iu {
                converged = true;
                break;
            }
            if !wantt {
                i1 = l;
                i2 = iu;
            }
            let t = if its == 10 {
                let s = DAT1 * h.at(l + 1, l).re.abs();
                c(s, 0.) + h.at(l, l)
            } else if its == 20 {
                let s = DAT1 * h.at(iu, iu - 1).re.abs();
                c(s, 0.) + h.at(iu, iu)
            } else {
                let mut tt = h.at(iu, iu);
                let u = csqrt(h.at(iu - 1, iu)) * csqrt(h.at(iu, iu - 1));
                let mut s = cabs1(u);
                if s != 0. {
                    let x = cmulr(h.at(iu - 1, iu - 1) - tt, 0.5);
                    let sx = cabs1(x);
                    s = s.max(cabs1(x));
                    let xs = cdivr(x, s);
                    let us = cdivr(u, s);
                    let mut y = cmulr(csqrt(xs * xs + us * us), s);
                    if sx > 0. {
                        let xsx = cdivr(x, sx);
                        if xsx.re * y.re + xsx.im * y.im < 0. {
                            y = -y;
                        }
                    }
                    tt -= u * zladiv(u, x + y);
                }
                tt
            };
            // Look for two consecutive small subdiagonal elements.
            let mut v = [ZERO; 2];
            let mut m = iu - 1;
            let mut found = false;
            while m > l {
                let h11 = h.at(m, m);
                let h22 = h.at(m + 1, m + 1);
                let mut h11s = h11 - t;
                let mut h21 = h.at(m + 1, m).re;
                let s = cabs1(h11s) + h21.abs();
                h11s = cdivr(h11s, s);
                h21 /= s;
                v[0] = h11s;
                v[1] = c(h21, 0.);
                let h10 = h.at(m, m - 1).re;
                if h10.abs() * h21.abs() <= ULP * (cabs1(h11s) * (cabs1(h11) + cabs1(h22))) {
                    found = true;
                    break;
                }
                m -= 1;
            }
            if !found {
                m = l;
                let h11 = h.at(l, l);
                let mut h11s = h11 - t;
                let mut h21 = h.at(l + 1, l).re;
                let s = cabs1(h11s) + h21.abs();
                h11s = cdivr(h11s, s);
                h21 /= s;
                v[0] = h11s;
                v[1] = c(h21, 0.);
            }
            // Single-shift QR step.
            for k in m..iu {
                if cancel() {
                    return Err(iu + 1);
                }
                if k > m {
                    v[0] = h.at(k, k - 1);
                    v[1] = h.at(k + 1, k - 1);
                }
                let (beta, t1) = {
                    let (head, tail) = v.split_at_mut(1);
                    zlarfg(head[0], &mut tail[..1])
                };
                v[0] = beta;
                if k > m {
                    h.set(k, k - 1, v[0]);
                    h.set(k + 1, k - 1, ZERO);
                }
                let v2 = v[1];
                let t2 = (t1 * v2).re;
                for j in k..=i2 {
                    let sum = t1.conj() * h.at(k, j) + c(t2, 0.) * h.at(k + 1, j);
                    *h.get_mut(k, j) -= sum;
                    *h.get_mut(k + 1, j) -= sum * v2;
                }
                for j in i1..=(k + 2).min(iu) {
                    let sum = t1 * h.at(j, k) + c(t2, 0.) * h.at(j, k + 1);
                    *h.get_mut(j, k) -= sum;
                    *h.get_mut(j, k + 1) -= sum * v2.conj();
                }
                if let Some(z) = z.as_deref_mut() {
                    for j in ilo..=ihi {
                        let sum = t1 * z.at(j, k) + c(t2, 0.) * z.at(j, k + 1);
                        *z.get_mut(j, k) -= sum;
                        *z.get_mut(j, k + 1) -= sum * v2.conj();
                    }
                }
                if k == m && m > l {
                    let temp = ONE - t1;
                    let temp = cdivr(temp, cabs(temp));
                    let x = h.at(m + 1, m);
                    h.set(m + 1, m, x * temp.conj());
                    if m + 2 <= iu {
                        let x = h.at(m + 2, m + 1);
                        h.set(m + 2, m + 1, x * temp);
                    }
                    for j in m..=iu {
                        if j != m + 1 {
                            if i2 > j {
                                zscal_row(h, j, j + 1, i2 + 1, temp);
                            }
                            zscal_col(h, j, i1, j, temp.conj());
                            if let Some(z) = z.as_deref_mut() {
                                zscal_col(z, j, ilo, ihi + 1, temp.conj());
                            }
                        }
                    }
                }
            }
            let temp = h.at(iu, iu - 1);
            if temp.im != 0. {
                let rtemp = cabs(temp);
                h.set(iu, iu - 1, c(rtemp, 0.));
                let temp = cdivr(temp, rtemp);
                if i2 > iu {
                    zscal_row(h, iu, iu + 1, i2 + 1, temp.conj());
                }
                zscal_col(h, iu, i1, iu, temp);
                if let Some(z) = z.as_deref_mut() {
                    zscal_col(z, iu, ilo, ihi + 1, temp);
                }
            }
        }
        if !converged {
            return Err(iu + 1);
        }
        w[iu] = h.at(iu, iu);
        i = l as isize - 1;
    }
    Ok(())
}

pub(crate) fn zhseqr(
    wantt: bool,
    h: &mut CMat,
    ilo: usize,
    ihi: usize,
    z: Option<&mut CMat>,
    cancel: &dyn Fn() -> bool,
) -> Result<Vec<C>, usize> {
    let n = h.rows;
    let mut w = vec![ZERO; n];
    for i in (0..ilo).chain(ihi + 1..n) {
        w[i] = h.at(i, i);
    }
    if n == 0 {
        return Ok(w);
    }
    if ilo == ihi {
        w[ilo] = h.at(ilo, ilo);
        return Ok(w);
    }
    zlahqr(wantt, h, ilo, ihi, &mut w, z, cancel)?;
    if wantt && n > 2 {
        for j in 0..n - 2 {
            for i in j + 2..n {
                h.set(i, j, ZERO);
            }
        }
    }
    Ok(w)
}

/// ZLATRS guarded substitution: solve op(T)*x = scale*b. Scaling the
/// entire live vector preserves the homogeneous eigenvector equation.
/// The ordinary-range arithmetic order is the same as ZTRSV.
fn zlatrs(t: &CMat, o: usize, len: usize, conj_trans: bool, x: &mut [C], cnorm: &[f64]) -> f64 {
    let smlnum = SAFMIN / ULP;
    let bignum = 1. / smlnum;
    let mut scale = 1.;
    let rescale = |x: &mut [C], scale: &mut f64, rec: f64| {
        for z in x {
            *z = cmulr(*z, rec);
        }
        *scale *= rec;
    };
    let mut xmax = x.iter().fold(0f64, |s, z| s.max(cabs1(*z)));
    if xmax > bignum * 0.5 {
        rescale(x, &mut scale, (bignum * 0.5) / xmax);
    }
    if !conj_trans {
        for j in (0..len).rev() {
            if x[j] != ZERO {
                let tjj = cabs1(t.at(o + j, o + j));
                let xj = cabs1(x[j]);
                if tjj < 1. && xj > tjj * bignum * 0.5 {
                    rescale(x, &mut scale, (tjj * bignum * 0.5) / xj);
                }
                x[j] = cdiv_recip(x[j], t.at(o + j, o + j));
                let xj = cabs1(x[j]);
                xmax = x.iter().fold(0f64, |s, z| s.max(cabs1(*z)));
                if xj > 1. && cnorm[j] > (bignum - xmax) / xj {
                    rescale(x, &mut scale, 0.5 / xj);
                }
                let temp = x[j];
                for i in (0..j).rev() {
                    x[i] = madd(x[i], -temp, t.at(o + i, o + j));
                }
            }
        }
    } else {
        for j in 0..len {
            xmax = x.iter().fold(0f64, |s, z| s.max(cabs1(*z)));
            if xmax > 1. && cnorm[j] > (bignum - cabs1(x[j])) / xmax {
                rescale(x, &mut scale, 0.5 / xmax);
            }
            let mut temp = x[j];
            for i in 0..j {
                temp = madd(temp, -t.at(o + i, o + j).conj(), x[i]);
            }
            let tjj = cabs1(t.at(o + j, o + j));
            let xj = cabs1(temp);
            if tjj < 1. && xj > tjj * bignum * 0.5 {
                let rec = (tjj * bignum * 0.5) / xj;
                rescale(x, &mut scale, rec);
                temp = cmulr(temp, rec);
            }
            temp = cdiv_recip(temp, t.at(o + j, o + j).conj());
            x[j] = temp;
        }
    }
    scale
}

/// ztrevc with howmny = 'B' (back-transform the Schur vectors in vl/vr).
pub(crate) fn ztrevc(
    t: &CMat,
    vl: Option<&mut CMat>,
    vr: Option<&mut CMat>,
    cancel: &dyn Fn() -> bool,
) {
    let n = t.rows;
    if n == 0 {
        return;
    }
    let smlnum = SAFMIN * (n as f64 / ULP);
    let mut tm = t.clone();
    let diag: Vec<C> = (0..n).map(|i| t.at(i, i)).collect();
    // Norms of strictly upper columns, shared by every eigenvector solve.
    // For trailing subproblems these are conservative overestimates.
    let cnorm: Vec<f64> = (0..n)
        .map(|j| (0..j).map(|i| cabs1(t.at(i, j))).sum())
        .collect();
    if let Some(vr) = vr {
        for ki in (0..n).rev() {
            if cancel() {
                return;
            }
            let smin = (ULP * cabs1(tm.at(ki, ki))).max(smlnum);
            let mut work = vec![ZERO; n];
            work[0] = ONE;
            for k in 0..ki {
                work[k] = -tm.at(k, ki);
            }
            for k in 0..ki {
                let d = tm.at(k, k) - tm.at(ki, ki);
                tm.set(k, k, if cabs1(d) < smin { c(smin, 0.) } else { d });
            }
            let mut scale = 1.;
            if ki > 0 {
                scale = zlatrs(&tm, 0, ki, false, &mut work[..ki], &cnorm[..ki]);
                work[ki] = c(scale, 0.);
            }
            if ki > 0 {
                // zgemv 'N' with beta = scale.
                let beta = c(scale, 0.);
                for i in 0..n {
                    let v = vr.at(i, ki);
                    vr.set(i, ki, if beta == ONE { v } else { beta * v });
                }
                for (j, &xj) in work.iter().enumerate().take(ki) {
                    if xj != ZERO {
                        let temp = ONE * xj;
                        for i in 0..n {
                            let a = vr.at(i, j);
                            let y = vr.at(i, ki);
                            vr.set(i, ki, madd(y, temp, a));
                        }
                    }
                }
            }
            let ii = izamax(vr.col(ki).iter());
            let remax = 1. / cabs1(vr.at(ii, ki));
            for i in 0..n {
                let v = vr.at(i, ki);
                vr.set(i, ki, c(remax, 0.) * v);
            }
            for k in 0..ki {
                tm.set(k, k, diag[k]);
            }
        }
    }
    if let Some(vl) = vl {
        for ki in 0..n {
            if cancel() {
                return;
            }
            let smin = (ULP * cabs1(tm.at(ki, ki))).max(smlnum);
            let mut work = vec![ZERO; n];
            work[n - 1] = ONE;
            for k in ki + 1..n {
                work[k] = -tm.at(ki, k).conj();
            }
            for k in ki + 1..n {
                let d = tm.at(k, k) - tm.at(ki, ki);
                tm.set(k, k, if cabs1(d) < smin { c(smin, 0.) } else { d });
            }
            let mut scale = 1.;
            if ki + 1 < n {
                scale = zlatrs(
                    &tm,
                    ki + 1,
                    n - ki - 1,
                    true,
                    &mut work[ki + 1..],
                    &cnorm[ki + 1..],
                );
                work[ki] = c(scale, 0.);
            }
            if ki + 1 < n {
                let beta = c(scale, 0.);
                for i in 0..n {
                    let v = vl.at(i, ki);
                    vl.set(i, ki, if beta == ONE { v } else { beta * v });
                }
                for j in ki + 1..n {
                    let xj = work[j];
                    if xj != ZERO {
                        let temp = ONE * xj;
                        for i in 0..n {
                            let a = vl.at(i, j);
                            let y = vl.at(i, ki);
                            vl.set(i, ki, madd(y, temp, a));
                        }
                    }
                }
            }
            let ii = izamax(vl.col(ki).iter());
            let remax = 1. / cabs1(vl.at(ii, ki));
            for i in 0..n {
                let v = vl.at(i, ki);
                vl.set(i, ki, c(remax, 0.) * v);
            }
            for k in ki + 1..n {
                tm.set(k, k, diag[k]);
            }
        }
    }
}

/// zgeevx normalisation: unit 2-norm, largest component real (|v_k|^2 is
/// fused, as MATLAB's MKL computes it).
pub(crate) fn znormalize(v: &mut CMat) {
    let n = v.rows;
    for i in 0..v.cols {
        let scl = 1. / znrm2(v.col(i).iter());
        for k in 0..n {
            let z = v.at(k, i);
            v.set(k, i, c(scl, 0.) * z);
        }
        let work: Vec<f64> = (0..n)
            .map(|k| {
                let z = v.at(k, i);
                z.re.fma(z.re, z.im * z.im)
            })
            .collect();
        let k = dense::idamax(work.iter());
        let tmp = cdivr(v.at(k, i).conj(), work[k].sqrt());
        for row in 0..n {
            let z = v.at(row, i);
            v.set(row, i, tmp * z);
        }
        let z = v.at(k, i);
        v.set(k, i, c(z.re, 0.));
    }
}

pub(crate) struct Zgeev {
    pub w: Vec<C>,
    pub vl: Option<CMat>,
    pub vr: Option<CMat>,
}

/// zgeevx with balanc 'B' or 'N', sense 'N'.
pub(crate) fn zgeevx(
    a: &CMat,
    balance: bool,
    want_vl: bool,
    want_vr: bool,
    cancel: &dyn Fn() -> bool,
) -> Result<Zgeev, usize> {
    let n = a.rows;
    let mut h = a.clone();
    let anrm = h
        .data
        .iter()
        .fold(0f64, |s, x| s.max(x.re.abs()).max(x.im.abs()));
    let cscale = dense::eigen_scale(anrm);
    if cscale != anrm {
        zlascl(&mut h.data, anrm, cscale);
    }
    let (ilo, ihi, scale) = zgebal(&mut h, balance, balance);
    if n == 0 {
        return Ok(Zgeev {
            w: vec![],
            vl: want_vl.then(|| CMat::zeros(0, 0)),
            vr: want_vr.then(|| CMat::zeros(0, 0)),
        });
    }
    let tau = zgehd2(&mut h, ilo, ihi, cancel)?;
    let (mut w, mut vl, mut vr);
    if want_vl || want_vr {
        let mut q = zunghr(&h, ilo, ihi, &tau, cancel);
        w = zhseqr(true, &mut h, ilo, ihi, Some(&mut q), cancel)?;
        vl = want_vl.then(|| q.clone());
        vr = want_vr.then_some(q);
        ztrevc(&h, vl.as_mut(), vr.as_mut(), cancel);
    } else {
        w = zhseqr(false, &mut h, ilo, ihi, None, cancel)?;
        vl = None;
        vr = None;
    }
    if let Some(v) = vl.as_mut() {
        zgebak(v, ilo, ihi, &scale, balance, balance, false);
        znormalize(v);
    }
    if let Some(v) = vr.as_mut() {
        zgebak(v, ilo, ihi, &scale, balance, balance, true);
        znormalize(v);
    }
    if cscale != anrm {
        zlascl(&mut w, cscale, anrm);
    }
    Ok(Zgeev { w, vl, vr })
}

fn zlascl(data: &mut [C], from: f64, to: f64) {
    for z in data {
        let mut parts = [z.re, z.im];
        dense::lascl(&mut parts, from, to);
        *z = c(parts[0], parts[1]);
    }
}

// ---- Hermitian: zheev ----

fn zhemv(a: &CMat, o: usize, n: usize, lower: bool, alpha: C, x: &[C]) -> Vec<C> {
    let mut y = vec![ZERO; n];
    if lower {
        for j in 0..n {
            let temp1 = alpha * x[j];
            let mut temp2 = ZERO;
            y[j] += temp1 * c(a.at(o + j, o + j).re, 0.);
            for i in j + 1..n {
                y[i] += temp1 * a.at(o + i, o + j);
                temp2 += a.at(o + i, o + j).conj() * x[i];
            }
            y[j] += alpha * temp2;
        }
    } else {
        for j in 0..n {
            let temp1 = alpha * x[j];
            let mut temp2 = ZERO;
            for i in 0..j {
                y[i] += temp1 * a.at(o + i, o + j);
                temp2 += a.at(o + i, o + j).conj() * x[i];
            }
            y[j] = y[j] + temp1 * c(a.at(o + j, o + j).re, 0.) + alpha * temp2;
        }
    }
    y
}

fn zher2(a: &mut CMat, o: usize, n: usize, lower: bool, alpha: C, x: &[C], y: &[C]) {
    for j in 0..n {
        if x[j] != ZERO || y[j] != ZERO {
            let temp1 = alpha * y[j].conj();
            let temp2 = (alpha * x[j]).conj();
            if lower {
                let d = a.at(o + j, o + j).re + (x[j] * temp1 + y[j] * temp2).re;
                a.set(o + j, o + j, c(d, 0.));
                for i in j + 1..n {
                    let v = a.at(o + i, o + j) + x[i] * temp1 + y[i] * temp2;
                    a.set(o + i, o + j, v);
                }
            } else {
                for i in 0..j {
                    let v = a.at(o + i, o + j) + x[i] * temp1 + y[i] * temp2;
                    a.set(o + i, o + j, v);
                }
                let d = a.at(o + j, o + j).re + (x[j] * temp1 + y[j] * temp2).re;
                a.set(o + j, o + j, c(d, 0.));
            }
        } else {
            let d = a.at(o + j, o + j).re;
            a.set(o + j, o + j, c(d, 0.));
        }
    }
}

fn zdotc(x: &[C], y: &[C]) -> C {
    let mut s = ZERO;
    for (a, b) in x.iter().zip(y) {
        s += a.conj() * b;
    }
    s
}

/// zhetd2: (d, e, tau).
pub(crate) fn zhetd2(
    a: &mut CMat,
    lower: bool,
    cancel: &dyn Fn() -> bool,
) -> (Vec<f64>, Vec<f64>, Vec<C>) {
    let n = a.rows;
    let mut d = vec![0.; n];
    let mut e = vec![0.; n.saturating_sub(1)];
    let mut tau = vec![ZERO; n.saturating_sub(1)];
    if n == 0 {
        return (d, e, tau);
    }
    if lower {
        let v = a.at(0, 0).re;
        a.set(0, 0, c(v, 0.));
        for i in 0..n - 1 {
            if cancel() {
                return (d, e, tau);
            }
            let alpha = a.at(i + 1, i);
            let mut x: Vec<C> = (i + 2..n).map(|r| a.at(r, i)).collect();
            let (beta, taui) = zlarfg(alpha, &mut x);
            for (k, &v) in x.iter().enumerate() {
                a.set(i + 2 + k, i, v);
            }
            e[i] = beta.re;
            if taui != ZERO {
                a.set(i + 1, i, ONE);
                let v: Vec<C> = (i + 1..n).map(|r| a.at(r, i)).collect();
                let mut w = zhemv(a, i + 1, n - i - 1, true, taui, &v);
                let alpha = cmulr(taui, -0.5) * zdotc(&w, &v);
                for (wk, vk) in w.iter_mut().zip(&v) {
                    *wk += alpha * vk;
                }
                zher2(a, i + 1, n - i - 1, true, c(-1., 0.), &v, &w);
            } else {
                let v = a.at(i + 1, i + 1).re;
                a.set(i + 1, i + 1, c(v, 0.));
            }
            a.set(i + 1, i, c(e[i], 0.));
            d[i] = a.at(i, i).re;
            tau[i] = taui;
        }
        d[n - 1] = a.at(n - 1, n - 1).re;
    } else {
        let v = a.at(n - 1, n - 1).re;
        a.set(n - 1, n - 1, c(v, 0.));
        for i in (0..n - 1).rev() {
            if cancel() {
                return (d, e, tau);
            }
            let alpha = a.at(i, i + 1);
            let mut x: Vec<C> = (0..i).map(|r| a.at(r, i + 1)).collect();
            let (beta, taui) = zlarfg(alpha, &mut x);
            for (k, &v) in x.iter().enumerate() {
                a.set(k, i + 1, v);
            }
            e[i] = beta.re;
            if taui != ZERO {
                a.set(i, i + 1, ONE);
                let v: Vec<C> = (0..=i).map(|r| a.at(r, i + 1)).collect();
                let mut w = zhemv(a, 0, i + 1, false, taui, &v);
                let alpha = cmulr(taui, -0.5) * zdotc(&w, &v);
                for (wk, vk) in w.iter_mut().zip(&v) {
                    *wk += alpha * vk;
                }
                zher2(a, 0, i + 1, false, c(-1., 0.), &v, &w);
            } else {
                let v = a.at(i, i).re;
                a.set(i, i, c(v, 0.));
            }
            a.set(i, i + 1, c(e[i], 0.));
            d[i + 1] = a.at(i + 1, i + 1).re;
            tau[i] = taui;
        }
        d[0] = a.at(0, 0).re;
    }
    (d, e, tau)
}

pub(crate) fn zungtr(a: &CMat, lower: bool, tau: &[C], cancel: &dyn Fn() -> bool) -> CMat {
    let n = a.rows;
    let mut q = a.clone();
    if n == 0 {
        return q;
    }
    if lower {
        for j in (1..n).rev() {
            if cancel() {
                return q;
            }
            q.set(0, j, ZERO);
            for i in j + 1..n {
                q.set(i, j, q.at(i, j - 1));
            }
        }
        q.set(0, 0, ONE);
        for i in 1..n {
            if cancel() {
                return q;
            }
            q.set(i, 0, ZERO);
        }
        if n > 1 {
            zung2r(&mut q, n - 1, tau, 1, 1, n - 1, n - 1, cancel);
        }
    } else {
        for j in 0..n - 1 {
            if cancel() {
                return q;
            }
            for i in 0..j {
                q.set(i, j, q.at(i, j + 1));
            }
            q.set(n - 1, j, ZERO);
        }
        for i in 0..n - 1 {
            if cancel() {
                return q;
            }
            q.set(i, n - 1, ZERO);
        }
        q.set(n - 1, n - 1, ONE);
        if n > 1 {
            zung2l(&mut q, n - 1, n - 1, n - 1, tau, cancel);
        }
    }
    q
}

/// zheev: ascending eigenvalues and, optionally, unitary eigenvectors.
pub(crate) fn zheev(
    a: &CMat,
    vectors: bool,
    lower: bool,
    cancel: &dyn Fn() -> bool,
) -> Result<(Vec<f64>, Option<CMat>), ()> {
    let n = a.rows;
    if n == 0 {
        return Ok((vec![], vectors.then(|| CMat::zeros(0, 0))));
    }
    if n == 1 {
        return Ok((vec![a.at(0, 0).re], vectors.then(|| CMat::identity(1))));
    }
    let mut h = a.clone();
    let anrm = h.data.iter().fold(0f64, |m, z| m.max(cabs(*z)));
    let rmin = (SAFMIN / ULP).sqrt();
    let scaled = if anrm == 0. {
        0.
    } else {
        anrm.clamp(rmin, 1. / rmin)
    };
    if scaled != anrm {
        zlascl(&mut h.data, anrm, scaled);
    }
    let (mut d, mut e, tau) = zhetd2(&mut h, lower, cancel);
    if !vectors {
        super::sym::sterf(&mut d, &mut e, cancel)?;
        if scaled != anrm {
            dense::lascl(&mut d, scaled, anrm);
        }
        return Ok((d, None));
    }
    let q = zungtr(&h, lower, &tau, cancel);
    // zsteqr applies the same real rotations to both parts of Z.
    let (mut re, mut im) = (q.re(), q.im());
    let (mut d2, mut e2) = (d.clone(), e.clone());
    super::sym::steqr(&mut d, &mut e, Some(&mut re), cancel)?;
    super::sym::steqr(&mut d2, &mut e2, Some(&mut im), cancel)?;
    if scaled != anrm {
        dense::lascl(&mut d, scaled, anrm);
    }
    Ok((d, Some(CMat::from_parts(&re, &im))))
}
