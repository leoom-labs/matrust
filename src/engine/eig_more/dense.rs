//! Real dense eigenvalue kernels, ported from reference LAPACK 3.5 in its
//! unblocked form (the path LAPACK itself takes below its block crossover):
//! dgebal, dgehd2, dorghr, dlahqr, dlanv2, dtrevc, dlaln2, dgebak and the
//! dgeevx normalisation. Column-major, 0-based. The operation order follows
//! the Fortran so small results agree with MATLAB's LAPACK to the last digit
//! as often as possible.
//!
//! MATLAB's MKL fuses the multiply-add of its axpy-style BLAS updates (dger,
//! dgemv 'N', daxpy) but not of its dot products; the ports do the same with
//! fused multiply-add (`Fma`), which reproduces the reference's eigenvalues digit for digit.
// LAPACK ports keep the reference routines' index loops.
#![allow(clippy::needless_range_loop)]
use crate::engine::Fma;

pub(crate) const EPS: f64 = f64::EPSILON * 0.5; // dlamch('E')
pub(crate) const ULP: f64 = f64::EPSILON; // dlamch('P')
pub(crate) const SAFMIN: f64 = f64::MIN_POSITIVE; // dlamch('S')

/// Dense column-major matrix.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Mat {
    pub rows: usize,
    pub cols: usize,
    pub data: Vec<f64>,
}

impl Mat {
    pub fn zeros(rows: usize, cols: usize) -> Self {
        Self {
            rows,
            cols,
            data: vec![0.; rows * cols],
        }
    }
    pub fn identity(n: usize) -> Self {
        let mut m = Self::zeros(n, n);
        for i in 0..n {
            m.data[i + i * n] = 1.;
        }
        m
    }
    pub fn from_vec(rows: usize, cols: usize, data: Vec<f64>) -> Self {
        debug_assert_eq!(data.len(), rows * cols);
        Self { rows, cols, data }
    }
    #[inline]
    pub fn at(&self, i: usize, j: usize) -> f64 {
        self.data[i + j * self.rows]
    }
    #[inline]
    pub fn set(&mut self, i: usize, j: usize, v: f64) {
        self.data[i + j * self.rows] = v;
    }
    #[inline]
    pub fn get_mut(&mut self, i: usize, j: usize) -> &mut f64 {
        &mut self.data[i + j * self.rows]
    }
    pub fn col(&self, j: usize) -> &[f64] {
        &self.data[j * self.rows..(j + 1) * self.rows]
    }
    pub fn transpose(&self) -> Self {
        let mut t = Self::zeros(self.cols, self.rows);
        for j in 0..self.cols {
            for i in 0..self.rows {
                t.set(j, i, self.at(i, j));
            }
        }
        t
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
}

/// dnrm2: the reference scaled sum of squares, with the updates fused as in
/// MATLAB's MKL.
pub(crate) fn nrm2<'a>(x: impl IntoIterator<Item = &'a f64>) -> f64 {
    let mut scale = 0f64;
    let mut ssq = 1f64;
    for &v in x {
        if v != 0. {
            let a = v.abs();
            if scale < a {
                let r = scale / a;
                ssq = (ssq * r).fma(r, 1.);
                scale = a;
            } else {
                let r = a / scale;
                ssq = r.fma(r, ssq);
            }
        }
    }
    scale * ssq.sqrt()
}

pub(crate) fn lapy2(x: f64, y: f64) -> f64 {
    let (xa, ya) = (x.abs(), y.abs());
    let w = xa.max(ya);
    let z = xa.min(ya);
    if z == 0. {
        w
    } else {
        w * (1. + (z / w) * (z / w)).sqrt()
    }
}

pub(crate) fn sign(a: f64, b: f64) -> f64 {
    if b >= 0. && !(b == 0. && b.is_sign_negative()) {
        a.abs()
    } else {
        -a.abs()
    }
}

/// Fortran SIGN for the sign test of a zero b: gfortran/ifort honour -0.
fn fsign(a: f64, b: f64) -> f64 {
    if b.is_sign_negative() {
        -a.abs()
    } else {
        a.abs()
    }
}

/// idamax: first index of the largest |x|.
pub(crate) fn idamax<'a>(x: impl IntoIterator<Item = &'a f64>) -> usize {
    let mut best = 0;
    let mut max = -1f64;
    for (i, &v) in x.into_iter().enumerate() {
        if v.abs() > max {
            max = v.abs();
            best = i;
        }
    }
    best
}

/// dlarfg on (alpha, x): returns (beta, tau) and scales x to v(2:n).
pub(crate) fn larfg(alpha: f64, x: &mut [f64]) -> (f64, f64) {
    if x.is_empty() {
        return (alpha, 0.);
    }
    let mut xnorm = nrm2(x.iter());
    if xnorm == 0. {
        return (alpha, 0.);
    }
    let mut alpha = alpha;
    let mut beta = -fsign(lapy2(alpha, xnorm), alpha);
    let safmin = SAFMIN / EPS;
    let mut knt = 0;
    if beta.abs() < safmin {
        let rsafmn = 1. / safmin;
        loop {
            knt += 1;
            for v in x.iter_mut() {
                *v *= rsafmn;
            }
            beta *= rsafmn;
            alpha *= rsafmn;
            if beta.abs() >= safmin || knt >= 20 {
                break;
            }
        }
        xnorm = nrm2(x.iter());
        beta = -fsign(lapy2(alpha, xnorm), alpha);
    }
    let tau = (beta - alpha) / beta;
    let scal = 1. / (alpha - beta);
    for v in x.iter_mut() {
        *v *= scal;
    }
    for _ in 0..knt {
        beta *= safmin;
    }
    (beta, tau)
}

/// dlarf 'Left': C(r0.., c0..c0+n) := H C with v of length m (v[0] == 1 implied by caller).
pub(crate) fn larf_left(c: &mut Mat, v: &[f64], tau: f64, r0: usize, c0: usize, n: usize) {
    if tau == 0. {
        return;
    }
    let mut lastv = v.len();
    while lastv > 0 && v[lastv - 1] == 0. {
        lastv -= 1;
    }
    // iladlc: last nonzero column of C(r0..r0+lastv, c0..c0+n).
    let mut lastc = n;
    while lastc > 0 && (0..lastv).all(|i| c.at(r0 + i, c0 + lastc - 1) == 0.) {
        lastc -= 1;
    }
    if lastv == 0 || lastc == 0 {
        return;
    }
    // w = C^T v (dgemv 'T'), then C -= tau v w^T (dger).
    let mut w = vec![0.; lastc];
    for (j, wj) in w.iter_mut().enumerate() {
        let mut t = 0.;
        for (i, &vi) in v.iter().enumerate().take(lastv) {
            t += c.at(r0 + i, c0 + j) * vi;
        }
        *wj = t;
    }
    for (j, &wj) in w.iter().enumerate() {
        if wj != 0. {
            let t = -tau * wj;
            for (i, &vi) in v.iter().enumerate().take(lastv) {
                let x = c.at(r0 + i, c0 + j);
                c.set(r0 + i, c0 + j, vi.fma(t, x));
            }
        }
    }
}

/// dlarf 'Right': C(r0..r0+m, c0..) := C H with v of length n.
pub(crate) fn larf_right(c: &mut Mat, v: &[f64], tau: f64, r0: usize, c0: usize, m: usize) {
    if tau == 0. {
        return;
    }
    let mut lastv = v.len();
    while lastv > 0 && v[lastv - 1] == 0. {
        lastv -= 1;
    }
    // iladlr: last nonzero row of C(r0..r0+m, c0..c0+lastv).
    let mut lastc = m;
    while lastc > 0 && (0..lastv).all(|j| c.at(r0 + lastc - 1, c0 + j) == 0.) {
        lastc -= 1;
    }
    if lastv == 0 || lastc == 0 {
        return;
    }
    // w = C v (dgemv 'N'), then C -= tau w v^T.
    let mut w = vec![0.; lastc];
    for (j, &vj) in v.iter().enumerate().take(lastv) {
        if vj != 0. {
            for (i, wi) in w.iter_mut().enumerate() {
                *wi = vj.fma(c.at(r0 + i, c0 + j), *wi);
            }
        }
    }
    for (j, &vj) in v.iter().enumerate().take(lastv) {
        if vj != 0. {
            let t = -tau * vj;
            for (i, &wi) in w.iter().enumerate() {
                let x = c.at(r0 + i, c0 + j);
                c.set(r0 + i, c0 + j, wi.fma(t, x));
            }
        }
    }
}

/// dgebal (the 1-norm form of LAPACK 3.4, which MATLAB's MKL uses). `permute`/`scale` select job 'P', 'S' or 'B'. Returns (ilo, ihi, scale)
/// with 0-based ilo..=ihi; permutation entries in scale are 0-based indices.
pub(crate) fn gebal(a: &mut Mat, permute: bool, scale_it: bool) -> (usize, usize, Vec<f64>) {
    let n = a.rows;
    let mut scale = vec![1.; n];
    if n == 0 {
        return (0, 0, scale);
    }
    if !permute && !scale_it {
        return (0, n - 1, scale);
    }
    let mut k = 0usize; // 0-based
    let mut l = n - 1;
    if permute {
        // Row search: rows isolating an eigenvalue go to the bottom.
        'rows: loop {
            let mut found = None;
            for j in (0..=l).rev() {
                if (0..=l).all(|i| i == j || a.at(j, i) == 0.) {
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
        // Column search: columns isolating an eigenvalue go to the left.
        loop {
            let mut found = None;
            for j in k..=l {
                if (k..=l).all(|i| i == j || a.at(i, j) == 0.) {
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
            // MKL's dgebal (as LAPACK before 3.5): off-diagonal 1-norms.
            let mut c = 0.;
            let mut r = 0.;
            for j in k..=l {
                if j != i {
                    c += a.at(j, i).abs();
                    r += a.at(i, j).abs();
                }
            }
            let ica = idamax(a.data[i * n..i * n + l + 1].iter());
            let mut ca = a.at(ica, i).abs();
            let ira = idamax((k..n).map(|j| &a.data[i + j * n]));
            let mut ra = a.at(i, ira + k).abs();
            if c == 0. || r == 0. {
                continue;
            }
            let (mut c, mut r) = (c, r);
            let mut g = r / SCLFAC;
            let mut f = 1f64;
            let s = c + r;
            while !(c >= g || f.max(c).max(ca) >= sfmax2 || r.min(g).min(ra) <= sfmin2) {
                if (c + f + ca + r + g + ra).is_nan() {
                    return (k, l, scale);
                }
                f *= SCLFAC;
                c *= SCLFAC;
                ca *= SCLFAC;
                r /= SCLFAC;
                g /= SCLFAC;
                ra /= SCLFAC;
            }
            g = c / SCLFAC;
            while !(g < r || r.max(ra) >= sfmax2 || f.min(c).min(g).min(ca) <= sfmin2) {
                f /= SCLFAC;
                c /= SCLFAC;
                g /= SCLFAC;
                ca /= SCLFAC;
                r *= SCLFAC;
                ra *= SCLFAC;
            }
            if c + r >= FACTOR * s {
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
                *a.get_mut(i, j) *= g;
            }
            for j in 0..=l {
                *a.get_mut(j, i) *= f;
            }
        }
        if !noconv {
            break;
        }
    }
    (k, l, scale)
}

/// dgebak for right (`right == true`) or left eigenvectors.
pub(crate) fn gebak(
    v: &mut Mat,
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
                *v.get_mut(i, j) *= s;
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

/// dgehd2 on rows/cols ilo..=ihi; returns tau (length n-1, zeros outside).
pub(crate) fn gehd2(
    a: &mut Mat,
    ilo: usize,
    ihi: usize,
    cancel: &dyn Fn() -> bool,
) -> Result<Vec<f64>, usize> {
    let n = a.rows;
    let mut tau = vec![0.; n.saturating_sub(1)];
    for i in ilo..ihi {
        if cancel() {
            return Err(i + 1);
        }
        // Reflector H(i) annihilates A(i+2:ihi, i).
        let alpha = a.at(i + 1, i);
        let len = ihi - i; // rows i+1..=ihi
        let mut x: Vec<f64> = (i + 2..=ihi).map(|r| a.at(r, i)).collect();
        let (beta, t) = larfg(alpha, &mut x);
        tau[i] = t;
        let mut v = Vec::with_capacity(len);
        v.push(1.);
        v.extend_from_slice(&x);
        for (k, &xv) in x.iter().enumerate() {
            a.set(i + 2 + k, i, xv);
        }
        // A(0:ihi, i+1:ihi) := A H(i)
        larf_right(a, &v, t, 0, i + 1, ihi + 1);
        // A(i+1:ihi, i+1:n) := H(i) A
        larf_left(a, &v, t, i + 1, i + 1, n - i - 1);
        a.set(i + 1, i, beta);
    }
    Ok(tau)
}

/// dorg2r on Q (m x n) with k reflectors stored below the diagonal.
#[allow(clippy::too_many_arguments)]
pub(crate) fn org2r(
    q: &mut Mat,
    k: usize,
    tau: &[f64],
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
            q.set(r0 + l, c0 + j, 0.);
        }
        q.set(r0 + j, c0 + j, 1.);
    }
    for i in (0..k).rev() {
        if cancel() {
            return;
        }
        if i + 1 < n {
            q.set(r0 + i, c0 + i, 1.);
            let v: Vec<f64> = (i..m).map(|r| q.at(r0 + r, c0 + i)).collect();
            larf_left(q, &v, tau[i], r0 + i, c0 + i + 1, n - i - 1);
        }
        if i + 1 < m {
            for r in i + 1..m {
                *q.get_mut(r0 + r, c0 + i) *= -tau[i];
            }
        }
        q.set(r0 + i, c0 + i, 1. - tau[i]);
        for l in 0..i {
            q.set(r0 + l, c0 + i, 0.);
        }
    }
}

/// dorghr: the orthogonal Q from gehd2's reflectors in `a`.
pub(crate) fn orghr(
    a: &Mat,
    ilo: usize,
    ihi: usize,
    tau: &[f64],
    cancel: &dyn Fn() -> bool,
) -> Mat {
    let n = a.rows;
    let mut q = a.clone();
    let nh = ihi - ilo;
    // Shift the vectors one column right; identity outside ilo+1..=ihi.
    for j in (ilo + 1..=ihi).rev() {
        if cancel() {
            return q;
        }
        for i in 0..j {
            q.set(i, j, 0.);
        }
        for i in j + 1..=ihi {
            q.set(i, j, q.at(i, j - 1));
        }
        for i in ihi + 1..n {
            q.set(i, j, 0.);
        }
    }
    for j in 0..=ilo {
        if cancel() {
            return q;
        }
        for i in 0..n {
            q.set(i, j, 0.);
        }
        q.set(j, j, 1.);
    }
    for j in ihi + 1..n {
        if cancel() {
            return q;
        }
        for i in 0..n {
            q.set(i, j, 0.);
        }
        q.set(j, j, 1.);
    }
    if nh > 0 {
        org2r(
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

/// dlanv2: standardise a 2x2 block. Returns (a, b, c, d, rt1r, rt1i, rt2r, rt2i, cs, sn).
#[allow(clippy::many_single_char_names)]
pub(crate) fn lanv2(mut a: f64, mut b: f64, mut c: f64, mut d: f64) -> [f64; 10] {
    const MULTPL: f64 = 4.;
    let eps = ULP;
    let (mut cs, mut sn);
    if c == 0. {
        cs = 1.;
        sn = 0.;
    } else if b == 0. {
        cs = 0.;
        sn = 1.;
        std::mem::swap(&mut a, &mut d);
        b = -c;
        c = 0.;
    } else if a - d == 0. && fsign(1., b) != fsign(1., c) {
        cs = 1.;
        sn = 0.;
    } else {
        let temp = a - d;
        let mut p = 0.5 * temp;
        let bcmax = b.abs().max(c.abs());
        let bcmis = b.abs().min(c.abs()) * fsign(1., b) * fsign(1., c);
        let scale = p.abs().max(bcmax);
        let mut z = (p / scale) * p + (bcmax / scale) * bcmis;
        if z >= MULTPL * eps {
            // Real eigenvalues.
            z = p + fsign(scale.sqrt() * z.sqrt(), p);
            a = d + z;
            d -= (bcmax / z) * bcmis;
            let tau = lapy2(c, z);
            cs = z / tau;
            sn = c / tau;
            b -= c;
            c = 0.;
        } else {
            // Complex or nearly equal real eigenvalues: equal diagonal.
            let sigma = b + c;
            let tau = lapy2(sigma, temp);
            cs = (0.5 * (1. + sigma.abs() / tau)).sqrt();
            sn = -(p / (tau * cs)) * fsign(1., sigma);
            let aa = a * cs + b * sn;
            let bb = -a * sn + b * cs;
            let cc = c * cs + d * sn;
            let dd = -c * sn + d * cs;
            a = aa * cs + cc * sn;
            b = bb * cs + dd * sn;
            c = -aa * sn + cc * cs;
            d = -bb * sn + dd * cs;
            let temp = 0.5 * (a + d);
            a = temp;
            d = temp;
            if c != 0. {
                if b != 0. {
                    if fsign(1., b) == fsign(1., c) {
                        // Real eigenvalues: reduce to upper triangular.
                        let sab = b.abs().sqrt();
                        let sac = c.abs().sqrt();
                        p = fsign(sab * sac, c);
                        let tau = 1. / (b + c).abs().sqrt();
                        a = temp + p;
                        d = temp - p;
                        b -= c;
                        c = 0.;
                        let cs1 = sab * tau;
                        let sn1 = sac * tau;
                        let temp = cs * cs1 - sn * sn1;
                        sn = cs * sn1 + sn * cs1;
                        cs = temp;
                    }
                } else {
                    b = -c;
                    c = 0.;
                    let temp = cs;
                    cs = -sn;
                    sn = temp;
                }
            }
        }
    }
    let (rt1r, rt2r) = (a, d);
    let (rt1i, rt2i) = if c == 0. {
        (0., 0.)
    } else {
        let v = b.abs().sqrt() * c.abs().sqrt();
        (v, -v)
    };
    [a, b, c, d, rt1r, rt1i, rt2r, rt2i, cs, sn]
}

/// drot on two strided sequences.
#[inline]
fn rot_pair(x: &mut f64, y: &mut f64, cs: f64, sn: f64) {
    let t = cs * *x + sn * *y;
    *y = cs * *y - sn * *x;
    *x = t;
}

/// dlahqr. `z` (if any) is updated over all its rows. Returns Err(i) on failure.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lahqr(
    wantt: bool,
    h: &mut Mat,
    ilo: usize,
    ihi: usize,
    wr: &mut [f64],
    wi: &mut [f64],
    mut z: Option<&mut Mat>,
    cancel: &dyn Fn() -> bool,
) -> Result<(), usize> {
    let n = h.rows;
    if n == 0 {
        return Ok(());
    }
    if ilo == ihi {
        wr[ilo] = h.at(ilo, ilo);
        wi[ilo] = 0.;
        return Ok(());
    }
    for j in ilo..ihi.saturating_sub(2) {
        h.set(j + 2, j, 0.);
        h.set(j + 3, j, 0.);
    }
    if ilo + 2 <= ihi {
        h.set(ihi, ihi - 2, 0.);
    }
    let nh = ihi - ilo + 1;
    let smlnum = SAFMIN * (nh as f64 / ULP);
    let (mut i1, mut i2) = (0usize, n - 1);
    let itmax = 30 * nh.max(10);
    const DAT1: f64 = 0.75;
    const DAT2: f64 = -0.4375;
    // i is the active bottom row (0-based, as signed for the exit test).
    let mut i = ihi as isize;
    while i >= ilo as isize {
        let iu = i as usize;
        let mut l = ilo;
        let mut converged = false;
        for its in 0..=itmax {
            if cancel() {
                return Err(iu + 1);
            }
            // Look for a single small subdiagonal element.
            let mut k = iu;
            while k > l {
                if h.at(k, k - 1).abs() <= smlnum {
                    break;
                }
                let mut tst = h.at(k - 1, k - 1).abs() + h.at(k, k).abs();
                if tst == 0. {
                    if k >= ilo + 2 {
                        tst += h.at(k - 1, k - 2).abs();
                    }
                    if k < ihi {
                        tst += h.at(k + 1, k).abs();
                    }
                }
                if h.at(k, k - 1).abs() <= ULP * tst {
                    let ab = h.at(k, k - 1).abs().max(h.at(k - 1, k).abs());
                    let ba = h.at(k, k - 1).abs().min(h.at(k - 1, k).abs());
                    let aa = h
                        .at(k, k)
                        .abs()
                        .max((h.at(k - 1, k - 1) - h.at(k, k)).abs());
                    let bb = h
                        .at(k, k)
                        .abs()
                        .min((h.at(k - 1, k - 1) - h.at(k, k)).abs());
                    let s = aa + ab;
                    if ba * (ab / s) <= smlnum.max(ULP * (bb * (aa / s))) {
                        break;
                    }
                }
                k -= 1;
            }
            l = k;
            if l > ilo {
                h.set(l, l - 1, 0.);
            }
            if l + 1 >= iu {
                converged = true;
                break;
            }
            if !wantt {
                i1 = l;
                i2 = iu;
            }
            let (mut h11, mut h12, mut h21, mut h22);
            if its == 10 {
                let s = h.at(l + 1, l).abs() + h.at(l + 2, l + 1).abs();
                h11 = DAT1 * s + h.at(l, l);
                h12 = DAT2 * s;
                h21 = s;
                h22 = h11;
            } else if its == 20 {
                let s = h.at(iu, iu - 1).abs() + h.at(iu - 1, iu - 2).abs();
                h11 = DAT1 * s + h.at(iu, iu);
                h12 = DAT2 * s;
                h21 = s;
                h22 = h11;
            } else {
                h11 = h.at(iu - 1, iu - 1);
                h21 = h.at(iu, iu - 1);
                h12 = h.at(iu - 1, iu);
                h22 = h.at(iu, iu);
            }
            let s = h11.abs() + h12.abs() + h21.abs() + h22.abs();
            let (rt1r, rt1i, rt2r, rt2i);
            if s == 0. {
                rt1r = 0.;
                rt1i = 0.;
                rt2r = 0.;
                rt2i = 0.;
            } else {
                h11 /= s;
                h21 /= s;
                h12 /= s;
                h22 /= s;
                let tr = (h11 + h22) / 2.;
                let det = (h11 - tr) * (h22 - tr) - h12 * h21;
                let rtdisc = det.abs().sqrt();
                if det >= 0. {
                    rt1r = tr * s;
                    rt2r = rt1r;
                    rt1i = rtdisc * s;
                    rt2i = -rt1i;
                } else {
                    let a = tr + rtdisc;
                    let b = tr - rtdisc;
                    let r = if (a - h22).abs() <= (b - h22).abs() {
                        a * s
                    } else {
                        b * s
                    };
                    rt1r = r;
                    rt2r = r;
                    rt1i = 0.;
                    rt2i = 0.;
                }
            }
            // Look for two consecutive small subdiagonal elements.
            let mut v = [0f64; 3];
            let mut m = iu - 2;
            loop {
                let h21s = h.at(m + 1, m);
                let s = (h.at(m, m) - rt2r).abs() + rt2i.abs() + h21s.abs();
                let h21s = h.at(m + 1, m) / s;
                v[0] = h21s * h.at(m, m + 1) + (h.at(m, m) - rt1r) * ((h.at(m, m) - rt2r) / s)
                    - rt1i * (rt2i / s);
                v[1] = h21s * (h.at(m, m) + h.at(m + 1, m + 1) - rt1r - rt2r);
                v[2] = h21s * h.at(m + 2, m + 1);
                let s = v[0].abs() + v[1].abs() + v[2].abs();
                v[0] /= s;
                v[1] /= s;
                v[2] /= s;
                if m == l {
                    break;
                }
                let h00 = h.at(m, m - 1).abs() * (v[1].abs() + v[2].abs());
                let h01 = ULP
                    * v[0].abs()
                    * (h.at(m - 1, m - 1).abs() + h.at(m, m).abs() + h.at(m + 1, m + 1).abs());
                if h00 <= h01 {
                    break;
                }
                m -= 1;
            }
            // Double-shift QR step.
            for k in m..iu {
                if cancel() {
                    return Err(iu + 1);
                }
                let nr = 3.min(iu - k + 1);
                if k > m {
                    for (q, vq) in v.iter_mut().enumerate().take(nr) {
                        *vq = h.at(k + q, k - 1);
                    }
                }
                let (beta, t1) = {
                    let (head, tail) = v.split_at_mut(1);
                    larfg(head[0], &mut tail[..nr - 1])
                };
                v[0] = beta;
                if k > m {
                    h.set(k, k - 1, v[0]);
                    h.set(k + 1, k - 1, 0.);
                    if k + 1 < iu {
                        h.set(k + 2, k - 1, 0.);
                    }
                } else if m > l {
                    let t = h.at(k, k - 1) * (1. - t1);
                    h.set(k, k - 1, t);
                }
                let v2 = v[1];
                let t2 = t1 * v2;
                if nr == 3 {
                    let v3 = v[2];
                    let t3 = t1 * v3;
                    for j in k..=i2 {
                        let sum = h.at(k, j) + v2 * h.at(k + 1, j) + v3 * h.at(k + 2, j);
                        *h.get_mut(k, j) -= sum * t1;
                        *h.get_mut(k + 1, j) -= sum * t2;
                        *h.get_mut(k + 2, j) -= sum * t3;
                    }
                    for j in i1..=(k + 3).min(iu) {
                        let sum = h.at(j, k) + v2 * h.at(j, k + 1) + v3 * h.at(j, k + 2);
                        *h.get_mut(j, k) -= sum * t1;
                        *h.get_mut(j, k + 1) -= sum * t2;
                        *h.get_mut(j, k + 2) -= sum * t3;
                    }
                    if let Some(z) = z.as_deref_mut() {
                        for j in 0..z.rows {
                            let sum = z.at(j, k) + v2 * z.at(j, k + 1) + v3 * z.at(j, k + 2);
                            *z.get_mut(j, k) -= sum * t1;
                            *z.get_mut(j, k + 1) -= sum * t2;
                            *z.get_mut(j, k + 2) -= sum * t3;
                        }
                    }
                } else if nr == 2 {
                    for j in k..=i2 {
                        let sum = h.at(k, j) + v2 * h.at(k + 1, j);
                        *h.get_mut(k, j) -= sum * t1;
                        *h.get_mut(k + 1, j) -= sum * t2;
                    }
                    for j in i1..=iu {
                        let sum = h.at(j, k) + v2 * h.at(j, k + 1);
                        *h.get_mut(j, k) -= sum * t1;
                        *h.get_mut(j, k + 1) -= sum * t2;
                    }
                    if let Some(z) = z.as_deref_mut() {
                        for j in 0..z.rows {
                            let sum = z.at(j, k) + v2 * z.at(j, k + 1);
                            *z.get_mut(j, k) -= sum * t1;
                            *z.get_mut(j, k + 1) -= sum * t2;
                        }
                    }
                }
            }
        }
        if !converged {
            return Err(iu + 1);
        }
        if l == iu {
            wr[iu] = h.at(iu, iu);
            wi[iu] = 0.;
        } else if l + 1 == iu {
            let r = lanv2(
                h.at(iu - 1, iu - 1),
                h.at(iu - 1, iu),
                h.at(iu, iu - 1),
                h.at(iu, iu),
            );
            h.set(iu - 1, iu - 1, r[0]);
            h.set(iu - 1, iu, r[1]);
            h.set(iu, iu - 1, r[2]);
            h.set(iu, iu, r[3]);
            wr[iu - 1] = r[4];
            wi[iu - 1] = r[5];
            wr[iu] = r[6];
            wi[iu] = r[7];
            let (cs, sn) = (r[8], r[9]);
            if wantt {
                for j in iu + 1..=i2 {
                    let (mut x, mut y) = (h.at(iu - 1, j), h.at(iu, j));
                    rot_pair(&mut x, &mut y, cs, sn);
                    h.set(iu - 1, j, x);
                    h.set(iu, j, y);
                }
                for j in i1..iu - 1 {
                    let (mut x, mut y) = (h.at(j, iu - 1), h.at(j, iu));
                    rot_pair(&mut x, &mut y, cs, sn);
                    h.set(j, iu - 1, x);
                    h.set(j, iu, y);
                }
            }
            if let Some(z) = z.as_deref_mut() {
                for j in 0..z.rows {
                    let (mut x, mut y) = (z.at(j, iu - 1), z.at(j, iu));
                    rot_pair(&mut x, &mut y, cs, sn);
                    z.set(j, iu - 1, x);
                    z.set(j, iu, y);
                }
            }
        }
        i = l as isize - 1;
    }
    Ok(())
}

/// dhseqr for small matrices: eigenvalues isolated by balancing, then dlahqr.
#[allow(clippy::type_complexity)]
pub(crate) fn hseqr(
    wantt: bool,
    h: &mut Mat,
    ilo: usize,
    ihi: usize,
    z: Option<&mut Mat>,
    cancel: &dyn Fn() -> bool,
) -> Result<(Vec<f64>, Vec<f64>), usize> {
    let n = h.rows;
    let mut wr = vec![0.; n];
    let mut wi = vec![0.; n];
    for i in (0..ilo).chain(ihi + 1..n) {
        wr[i] = h.at(i, i);
        wi[i] = 0.;
    }
    if n == 0 {
        return Ok((wr, wi));
    }
    if ilo == ihi {
        wr[ilo] = h.at(ilo, ilo);
        return Ok((wr, wi));
    }
    lahqr(wantt, h, ilo, ihi, &mut wr, &mut wi, z, cancel)?;
    if wantt && n > 2 {
        for j in 0..n - 2 {
            for i in j + 2..n {
                h.set(i, j, 0.);
            }
        }
    }
    Ok((wr, wi))
}

/// dladiv, the robust algorithm of LAPACK 3.5 (Baudin and Smith).
pub(crate) fn ladiv(a: f64, b: f64, c: f64, d: f64) -> (f64, f64) {
    let (mut aa, mut bb, mut cc, mut dd) = (a, b, c, d);
    let ab = a.abs().max(b.abs());
    let cd = c.abs().max(d.abs());
    let mut s = 1.;
    let ov = f64::MAX;
    let un = SAFMIN;
    let eps = EPS;
    let bs = 2.;
    let be = bs / (eps * eps);
    if ab >= 0.5 * ov {
        aa *= 0.5;
        bb *= 0.5;
        s *= 2.;
    }
    if cd >= 0.5 * ov {
        cc *= 0.5;
        dd *= 0.5;
        s *= 0.5;
    }
    if ab <= un * bs / eps {
        aa *= be;
        bb *= be;
        s /= be;
    }
    if cd <= un * bs / eps {
        cc *= be;
        dd *= be;
        s *= be;
    }
    let (p, q) = if d.abs() <= c.abs() {
        ladiv1(aa, bb, cc, dd)
    } else {
        let (p, q) = ladiv1(bb, aa, dd, cc);
        (p, -q)
    };
    (p * s, q * s)
}

fn ladiv1(a: f64, b: f64, c: f64, d: f64) -> (f64, f64) {
    let r = d / c;
    let t = 1. / (c + d * r);
    let p = ladiv2(a, b, c, d, r, t);
    let q = ladiv2(b, -a, c, d, r, t);
    (p, q)
}

fn ladiv2(a: f64, b: f64, c: f64, d: f64, r: f64, t: f64) -> f64 {
    if r != 0. {
        let br = b * r;
        if br != 0. {
            (a + br) * t
        } else {
            a * t + (b * t) * r
        }
    } else {
        (a + d * (b / c)) * t
    }
}

/// dlaln2 result: x (2x2, column-major: x[r + 2c]), scale, xnorm.
#[allow(clippy::too_many_arguments)]
pub(crate) fn laln2(
    ltrans: bool,
    na: usize,
    nw: usize,
    smin: f64,
    ca: f64,
    a: [f64; 4], // A(1,1), A(2,1), A(1,2), A(2,2)
    d1: f64,
    d2: f64,
    b: [f64; 4], // B(1,1), B(2,1), B(1,2), B(2,2)
    wr: f64,
    wi: f64,
) -> ([f64; 4], f64, f64) {
    const ZSWAP: [bool; 4] = [false, false, true, true];
    const RSWAP: [bool; 4] = [false, true, false, true];
    const IPIVOT: [[usize; 4]; 4] = [[0, 1, 2, 3], [1, 0, 3, 2], [2, 3, 0, 1], [3, 2, 1, 0]];
    let smlnum = 2. * SAFMIN;
    let bignum = 1. / smlnum;
    let smini = smin.max(smlnum);
    let mut scale = 1.;
    let mut x = [0f64; 4];
    if na == 1 {
        if nw == 1 {
            let mut csr = ca * a[0] - wr * d1;
            let mut cnorm = csr.abs();
            if cnorm < smini {
                csr = smini;
                cnorm = smini;
            }
            let bnorm = b[0].abs();
            if cnorm < 1. && bnorm > 1. && bnorm > bignum * cnorm {
                scale = 1. / bnorm;
            }
            x[0] = (b[0] * scale) / csr;
            let xnorm = x[0].abs();
            return (x, scale, xnorm);
        }
        let mut csr = ca * a[0] - wr * d1;
        let mut csi = -wi * d1;
        let mut cnorm = csr.abs() + csi.abs();
        if cnorm < smini {
            csr = smini;
            csi = 0.;
            cnorm = smini;
        }
        let bnorm = b[0].abs() + b[2].abs();
        if cnorm < 1. && bnorm > 1. && bnorm > bignum * cnorm {
            scale = 1. / bnorm;
        }
        let (p, q) = ladiv(scale * b[0], scale * b[2], csr, csi);
        x[0] = p;
        x[2] = q;
        let xnorm = p.abs() + q.abs();
        return (x, scale, xnorm);
    }
    // 2x2: CR column-major (CRV(1)=CR(1,1), CRV(2)=CR(2,1), CRV(3)=CR(1,2), CRV(4)=CR(2,2)).
    let mut cr = [0f64; 4];
    cr[0] = ca * a[0] - wr * d1;
    cr[3] = ca * a[3] - wr * d2;
    if ltrans {
        cr[2] = ca * a[1];
        cr[1] = ca * a[2];
    } else {
        cr[1] = ca * a[1];
        cr[2] = ca * a[2];
    }
    if nw == 1 {
        let mut cmax = 0.;
        let mut icmax = usize::MAX;
        for (j, &v) in cr.iter().enumerate() {
            if v.abs() > cmax {
                cmax = v.abs();
                icmax = j;
            }
        }
        if cmax < smini {
            let bnorm = b[0].abs().max(b[1].abs());
            if smini < 1. && bnorm > 1. && bnorm > bignum * smini {
                scale = 1. / bnorm;
            }
            let temp = scale / smini;
            x[0] = temp * b[0];
            x[1] = temp * b[1];
            return (x, scale, temp * bnorm);
        }
        let ur11 = cr[icmax];
        let cr21 = cr[IPIVOT[icmax][1]];
        let ur12 = cr[IPIVOT[icmax][2]];
        let cr22 = cr[IPIVOT[icmax][3]];
        let ur11r = 1. / ur11;
        let lr21 = ur11r * cr21;
        let mut ur22 = cr22 - ur12 * lr21;
        if ur22.abs() < smini {
            ur22 = smini;
        }
        let (br1, mut br2) = if RSWAP[icmax] {
            (b[1], b[0])
        } else {
            (b[0], b[1])
        };
        br2 -= lr21 * br1;
        let bbnd = (br1 * (ur22 * ur11r)).abs().max(br2.abs());
        if bbnd > 1. && ur22.abs() < 1. && bbnd >= bignum * ur22.abs() {
            scale = 1. / bbnd;
        }
        let xr2 = (br2 * scale) / ur22;
        let xr1 = (scale * br1) * ur11r - xr2 * (ur11r * ur12);
        if ZSWAP[icmax] {
            x[0] = xr2;
            x[1] = xr1;
        } else {
            x[0] = xr1;
            x[1] = xr2;
        }
        let mut xnorm = xr1.abs().max(xr2.abs());
        if xnorm > 1. && cmax > 1. && xnorm > bignum / cmax {
            let temp = cmax / bignum;
            x[0] *= temp;
            x[1] *= temp;
            xnorm *= temp;
            scale *= temp;
        }
        return (x, scale, xnorm);
    }
    let mut ci = [0f64; 4];
    ci[0] = -wi * d1;
    ci[3] = -wi * d2;
    let mut cmax = 0.;
    let mut icmax = usize::MAX;
    for j in 0..4 {
        if cr[j].abs() + ci[j].abs() > cmax {
            cmax = cr[j].abs() + ci[j].abs();
            icmax = j;
        }
    }
    if cmax < smini {
        let bnorm = (b[0].abs() + b[2].abs()).max(b[1].abs() + b[3].abs());
        if smini < 1. && bnorm > 1. && bnorm > bignum * smini {
            scale = 1. / bnorm;
        }
        let temp = scale / smini;
        for k in 0..4 {
            x[k] = temp * b[k];
        }
        return (x, scale, temp * bnorm);
    }
    let ur11 = cr[icmax];
    let ui11 = ci[icmax];
    let cr21 = cr[IPIVOT[icmax][1]];
    let ci21 = ci[IPIVOT[icmax][1]];
    let ur12 = cr[IPIVOT[icmax][2]];
    let ui12 = ci[IPIVOT[icmax][2]];
    let cr22 = cr[IPIVOT[icmax][3]];
    let ci22 = ci[IPIVOT[icmax][3]];
    let (ur11r, ui11r, lr21, li21, ur12s, ui12s, mut ur22, mut ui22);
    if icmax == 0 || icmax == 3 {
        if ur11.abs() > ui11.abs() {
            let temp = ui11 / ur11;
            ur11r = 1. / (ur11 * (1. + temp * temp));
            ui11r = -temp * ur11r;
        } else {
            let temp = ur11 / ui11;
            ui11r = -1. / (ui11 * (1. + temp * temp));
            ur11r = -temp * ui11r;
        }
        lr21 = cr21 * ur11r;
        li21 = cr21 * ui11r;
        ur12s = ur12 * ur11r;
        ui12s = ur12 * ui11r;
        ur22 = cr22 - ur12 * lr21;
        ui22 = ci22 - ur12 * li21;
    } else {
        ur11r = 1. / ur11;
        ui11r = 0.;
        lr21 = cr21 * ur11r;
        li21 = ci21 * ur11r;
        ur12s = ur12 * ur11r;
        ui12s = ui12 * ur11r;
        ur22 = cr22 - ur12 * lr21 + ui12 * li21;
        ui22 = -ur12 * li21 - ui12 * lr21;
    }
    let u22abs = ur22.abs() + ui22.abs();
    if u22abs < smini {
        ur22 = smini;
        ui22 = 0.;
    }
    let (mut br1, mut br2, mut bi1, mut bi2) = if RSWAP[icmax] {
        (b[1], b[0], b[3], b[2])
    } else {
        (b[0], b[1], b[2], b[3])
    };
    br2 = br2 - lr21 * br1 + li21 * bi1;
    bi2 = bi2 - li21 * br1 - lr21 * bi1;
    let bbnd = ((br1.abs() + bi1.abs()) * (u22abs * (ur11r.abs() + ui11r.abs())))
        .max(br2.abs() + bi2.abs());
    if bbnd > 1. && u22abs < 1. && bbnd >= bignum * u22abs {
        scale = 1. / bbnd;
        br1 *= scale;
        bi1 *= scale;
        br2 *= scale;
        bi2 *= scale;
    }
    let (xr2, xi2) = ladiv(br2, bi2, ur22, ui22);
    let xr1 = ur11r * br1 - ui11r * bi1 - ur12s * xr2 + ui12s * xi2;
    let xi1 = ui11r * br1 + ur11r * bi1 - ui12s * xr2 - ur12s * xi2;
    if ZSWAP[icmax] {
        x = [xr2, xr1, xi2, xi1];
    } else {
        x = [xr1, xr2, xi1, xi2];
    }
    let mut xnorm = (xr1.abs() + xi1.abs()).max(xr2.abs() + xi2.abs());
    if xnorm > 1. && cmax > 1. && xnorm > bignum / cmax {
        let temp = cmax / bignum;
        for v in x.iter_mut() {
            *v *= temp;
        }
        xnorm *= temp;
        scale *= temp;
    }
    (x, scale, xnorm)
}

/// Reference dgemv 'N' into column `dst` of `v`: v(:,dst) = V(:,c0..c0+len) x + beta v(:,dst).
fn gemv_into_col(v: &mut Mat, c0: usize, len: usize, x: &[f64], beta: f64, dst: usize) {
    let n = v.rows;
    if beta != 1. {
        for i in 0..n {
            let t = if beta == 0. { 0. } else { beta * v.at(i, dst) };
            v.set(i, dst, t);
        }
    }
    for (j, &xj) in x.iter().enumerate().take(len) {
        if xj != 0. {
            for i in 0..n {
                let y = v.at(i, dst);
                v.set(i, dst, xj.fma(v.at(i, c0 + j), y));
            }
        }
    }
}

/// dtrevc with howmny = 'B': back-transforms into `vr`/`vl`, which hold the
/// Schur vectors on entry.
pub(crate) fn trevc(
    t: &Mat,
    vl: Option<&mut Mat>,
    vr: Option<&mut Mat>,
    cancel: &dyn Fn() -> bool,
) {
    let n = t.rows;
    if n == 0 {
        return;
    }
    let unfl = SAFMIN;
    let ulp = ULP;
    let smlnum = unfl * (n as f64 / ulp);
    let bignum = (1. - ulp) / smlnum;
    let mut cnorm = vec![0f64; n];
    for j in 1..n {
        if cancel() {
            return;
        }
        for i in 0..j {
            cnorm[j] += t.at(i, j).abs();
        }
    }
    let tt = |i: usize, j: usize| t.at(i, j);
    let blk = |i: usize, j: usize| [tt(i, j), tt(i + 1, j), tt(i, j + 1), tt(i + 1, j + 1)];
    if let Some(vr) = vr {
        let mut w1 = vec![0f64; n];
        let mut w2 = vec![0f64; n];
        let mut ip = 0i32;
        for ki in (0..n).rev() {
            if cancel() {
                return;
            }
            if ip == 1 {
                ip = 0;
                continue;
            }
            if ki > 0 && tt(ki, ki - 1) != 0. {
                ip = -1;
            }
            let wr = tt(ki, ki);
            let wi = if ip != 0 {
                tt(ki, ki - 1).abs().sqrt() * tt(ki - 1, ki).abs().sqrt()
            } else {
                0.
            };
            let smin = (ulp * (wr.abs() + wi.abs())).max(smlnum);
            if ip == 0 {
                w1[ki] = 1.;
                for k in 0..ki {
                    w1[k] = -tt(k, ki);
                }
                let mut j = ki as isize - 1;
                while j >= 0 {
                    let ju = j as usize;
                    let (j1, j2) = if ju > 0 && tt(ju, ju - 1) != 0. {
                        (ju - 1, ju)
                    } else {
                        (ju, ju)
                    };
                    if j1 == j2 {
                        let (mut x, mut scale, xnorm) = laln2(
                            false,
                            1,
                            1,
                            smin,
                            1.,
                            [tt(ju, ju), 0., 0., 0.],
                            1.,
                            1.,
                            [w1[ju], 0., 0., 0.],
                            wr,
                            0.,
                        );
                        if xnorm > 1. && cnorm[ju] > bignum / xnorm {
                            x[0] /= xnorm;
                            scale /= xnorm;
                        }
                        if scale != 1. {
                            for v in w1.iter_mut().take(ki + 1) {
                                *v *= scale;
                            }
                        }
                        w1[ju] = x[0];
                        for k in 0..ju {
                            w1[k] = (-x[0]).fma(tt(k, ju), w1[k]);
                        }
                        j -= 1;
                    } else {
                        let (mut x, mut scale, xnorm) = laln2(
                            false,
                            2,
                            1,
                            smin,
                            1.,
                            blk(j1, j1),
                            1.,
                            1.,
                            [w1[j1], w1[j2], 0., 0.],
                            wr,
                            0.,
                        );
                        if xnorm > 1. {
                            let beta = cnorm[j1].max(cnorm[j2]);
                            if beta > bignum / xnorm {
                                x[0] /= xnorm;
                                x[1] /= xnorm;
                                scale /= xnorm;
                            }
                        }
                        if scale != 1. {
                            for v in w1.iter_mut().take(ki + 1) {
                                *v *= scale;
                            }
                        }
                        w1[j1] = x[0];
                        w1[j2] = x[1];
                        for k in 0..j1 {
                            w1[k] = (-x[0]).fma(tt(k, j1), w1[k]);
                        }
                        for k in 0..j1 {
                            w1[k] = (-x[1]).fma(tt(k, j2), w1[k]);
                        }
                        j -= 2;
                    }
                }
                if ki > 0 {
                    let x: Vec<f64> = w1[..ki].to_vec();
                    gemv_into_col(vr, 0, ki, &x, w1[ki], ki);
                }
                let ii = idamax(vr.col(ki).iter());
                let remax = 1. / vr.at(ii, ki).abs();
                for i in 0..n {
                    *vr.get_mut(i, ki) *= remax;
                }
            } else {
                // Complex right eigenvector for the pair (ki-1, ki).
                if tt(ki - 1, ki).abs() >= tt(ki, ki - 1).abs() {
                    w1[ki - 1] = 1.;
                    w2[ki] = wi / tt(ki - 1, ki);
                } else {
                    w1[ki - 1] = -wi / tt(ki, ki - 1);
                    w2[ki] = 1.;
                }
                w1[ki] = 0.;
                w2[ki - 1] = 0.;
                for k in 0..ki - 1 {
                    w1[k] = -w1[ki - 1] * tt(k, ki - 1);
                    w2[k] = -w2[ki] * tt(k, ki);
                }
                let mut j = ki as isize - 2;
                while j >= 0 {
                    let ju = j as usize;
                    let (j1, j2) = if ju > 0 && tt(ju, ju - 1) != 0. {
                        (ju - 1, ju)
                    } else {
                        (ju, ju)
                    };
                    if j1 == j2 {
                        let (mut x, mut scale, xnorm) = laln2(
                            false,
                            1,
                            2,
                            smin,
                            1.,
                            [tt(ju, ju), 0., 0., 0.],
                            1.,
                            1.,
                            [w1[ju], 0., w2[ju], 0.],
                            wr,
                            wi,
                        );
                        if xnorm > 1. && cnorm[ju] > bignum / xnorm {
                            x[0] /= xnorm;
                            x[2] /= xnorm;
                            scale /= xnorm;
                        }
                        if scale != 1. {
                            for k in 0..=ki {
                                w1[k] *= scale;
                                w2[k] *= scale;
                            }
                        }
                        w1[ju] = x[0];
                        w2[ju] = x[2];
                        for k in 0..ju {
                            w1[k] = (-x[0]).fma(tt(k, ju), w1[k]);
                        }
                        for k in 0..ju {
                            w2[k] = (-x[2]).fma(tt(k, ju), w2[k]);
                        }
                        j -= 1;
                    } else {
                        let (mut x, mut scale, xnorm) = laln2(
                            false,
                            2,
                            2,
                            smin,
                            1.,
                            blk(j1, j1),
                            1.,
                            1.,
                            [w1[j1], w1[j2], w2[j1], w2[j2]],
                            wr,
                            wi,
                        );
                        if xnorm > 1. {
                            let beta = cnorm[j1].max(cnorm[j2]);
                            if beta > bignum / xnorm {
                                let rec = 1. / xnorm;
                                for v in x.iter_mut() {
                                    *v *= rec;
                                }
                                scale *= rec;
                            }
                        }
                        if scale != 1. {
                            for k in 0..=ki {
                                w1[k] *= scale;
                                w2[k] *= scale;
                            }
                        }
                        w1[j1] = x[0];
                        w1[j2] = x[1];
                        w2[j1] = x[2];
                        w2[j2] = x[3];
                        for k in 0..j1 {
                            w1[k] = (-x[0]).fma(tt(k, j1), w1[k]);
                        }
                        for k in 0..j1 {
                            w1[k] = (-x[1]).fma(tt(k, j2), w1[k]);
                        }
                        for k in 0..j1 {
                            w2[k] = (-x[2]).fma(tt(k, j1), w2[k]);
                        }
                        for k in 0..j1 {
                            w2[k] = (-x[3]).fma(tt(k, j2), w2[k]);
                        }
                        j -= 2;
                    }
                }
                if ki > 1 {
                    let x1: Vec<f64> = w1[..ki - 1].to_vec();
                    let x2: Vec<f64> = w2[..ki - 1].to_vec();
                    gemv_into_col(vr, 0, ki - 1, &x1, w1[ki - 1], ki - 1);
                    gemv_into_col(vr, 0, ki - 1, &x2, w2[ki], ki);
                } else {
                    for i in 0..n {
                        *vr.get_mut(i, ki - 1) *= w1[ki - 1];
                        *vr.get_mut(i, ki) *= w2[ki];
                    }
                }
                let mut emax = 0f64;
                for k in 0..n {
                    emax = emax.max(vr.at(k, ki - 1).abs() + vr.at(k, ki).abs());
                }
                let remax = 1. / emax;
                for i in 0..n {
                    *vr.get_mut(i, ki - 1) *= remax;
                    *vr.get_mut(i, ki) *= remax;
                }
            }
            if ip == -1 {
                ip = 1;
            }
        }
    }
    if let Some(vl) = vl {
        let mut w1 = vec![0f64; n];
        let mut w2 = vec![0f64; n];
        let mut ip = 0i32;
        for ki in 0..n {
            if cancel() {
                return;
            }
            if ip == -1 {
                ip = 0;
                continue;
            }
            if ki + 1 < n && tt(ki + 1, ki) != 0. {
                ip = 1;
            }
            let wr = tt(ki, ki);
            let wi = if ip != 0 {
                tt(ki, ki + 1).abs().sqrt() * tt(ki + 1, ki).abs().sqrt()
            } else {
                0.
            };
            let smin = (ulp * (wr.abs() + wi.abs())).max(smlnum);
            let dot = |col: usize, from: usize, to: usize, w: &[f64]| {
                let mut s = 0.;
                for k in from..to {
                    s += tt(k, col) * w[k];
                }
                s
            };
            if ip == 0 {
                w1[ki] = 1.;
                for k in ki + 1..n {
                    w1[k] = -tt(ki, k);
                }
                let mut vmax = 1f64;
                let mut vcrit = bignum;
                let mut j = ki + 1;
                while j < n {
                    let (j1, j2) = if j + 1 < n && tt(j + 1, j) != 0. {
                        (j, j + 1)
                    } else {
                        (j, j)
                    };
                    if j1 == j2 {
                        if cnorm[j] > vcrit {
                            let rec = 1. / vmax;
                            for v in w1.iter_mut().skip(ki) {
                                *v *= rec;
                            }
                            vmax = 1.;
                        }
                        w1[j] -= dot(j, ki + 1, j, &w1);
                        let (x, scale, _) = laln2(
                            false,
                            1,
                            1,
                            smin,
                            1.,
                            [tt(j, j), 0., 0., 0.],
                            1.,
                            1.,
                            [w1[j], 0., 0., 0.],
                            wr,
                            0.,
                        );
                        if scale != 1. {
                            for v in w1.iter_mut().skip(ki) {
                                *v *= scale;
                            }
                        }
                        w1[j] = x[0];
                        vmax = w1[j].abs().max(vmax);
                        vcrit = bignum / vmax;
                        j += 1;
                    } else {
                        let beta = cnorm[j].max(cnorm[j + 1]);
                        if beta > vcrit {
                            let rec = 1. / vmax;
                            for v in w1.iter_mut().skip(ki) {
                                *v *= rec;
                            }
                            vmax = 1.;
                        }
                        w1[j] -= dot(j, ki + 1, j, &w1);
                        w1[j + 1] -= dot(j + 1, ki + 1, j, &w1);
                        let (x, scale, _) = laln2(
                            true,
                            2,
                            1,
                            smin,
                            1.,
                            blk(j, j),
                            1.,
                            1.,
                            [w1[j], w1[j + 1], 0., 0.],
                            wr,
                            0.,
                        );
                        if scale != 1. {
                            for v in w1.iter_mut().skip(ki) {
                                *v *= scale;
                            }
                        }
                        w1[j] = x[0];
                        w1[j + 1] = x[1];
                        vmax = w1[j].abs().max(w1[j + 1].abs()).max(vmax);
                        vcrit = bignum / vmax;
                        j += 2;
                    }
                }
                if ki + 1 < n {
                    let x: Vec<f64> = w1[ki + 1..n].to_vec();
                    gemv_into_col(vl, ki + 1, n - ki - 1, &x, w1[ki], ki);
                }
                let ii = idamax(vl.col(ki).iter());
                let remax = 1. / vl.at(ii, ki).abs();
                for i in 0..n {
                    *vl.get_mut(i, ki) *= remax;
                }
            } else {
                if tt(ki, ki + 1).abs() >= tt(ki + 1, ki).abs() {
                    w1[ki] = wi / tt(ki, ki + 1);
                    w2[ki + 1] = 1.;
                } else {
                    w1[ki] = 1.;
                    w2[ki + 1] = -wi / tt(ki + 1, ki);
                }
                w1[ki + 1] = 0.;
                w2[ki] = 0.;
                for k in ki + 2..n {
                    w1[k] = -w1[ki] * tt(ki, k);
                    w2[k] = -w2[ki + 1] * tt(ki + 1, k);
                }
                let mut vmax = 1f64;
                let mut vcrit = bignum;
                let mut j = ki + 2;
                while j < n {
                    let (j1, j2) = if j + 1 < n && tt(j + 1, j) != 0. {
                        (j, j + 1)
                    } else {
                        (j, j)
                    };
                    if j1 == j2 {
                        if cnorm[j] > vcrit {
                            let rec = 1. / vmax;
                            for k in ki..n {
                                w1[k] *= rec;
                                w2[k] *= rec;
                            }
                            vmax = 1.;
                        }
                        w1[j] -= dot(j, ki + 2, j, &w1);
                        w2[j] -= dot(j, ki + 2, j, &w2);
                        let (x, scale, _) = laln2(
                            false,
                            1,
                            2,
                            smin,
                            1.,
                            [tt(j, j), 0., 0., 0.],
                            1.,
                            1.,
                            [w1[j], 0., w2[j], 0.],
                            wr,
                            -wi,
                        );
                        if scale != 1. {
                            for k in ki..n {
                                w1[k] *= scale;
                                w2[k] *= scale;
                            }
                        }
                        w1[j] = x[0];
                        w2[j] = x[2];
                        vmax = w1[j].abs().max(w2[j].abs()).max(vmax);
                        vcrit = bignum / vmax;
                        j += 1;
                    } else {
                        let beta = cnorm[j].max(cnorm[j + 1]);
                        if beta > vcrit {
                            let rec = 1. / vmax;
                            for k in ki..n {
                                w1[k] *= rec;
                                w2[k] *= rec;
                            }
                            vmax = 1.;
                        }
                        w1[j] -= dot(j, ki + 2, j, &w1);
                        w2[j] -= dot(j, ki + 2, j, &w2);
                        w1[j + 1] -= dot(j + 1, ki + 2, j, &w1);
                        w2[j + 1] -= dot(j + 1, ki + 2, j, &w2);
                        let (x, scale, _) = laln2(
                            true,
                            2,
                            2,
                            smin,
                            1.,
                            blk(j, j),
                            1.,
                            1.,
                            [w1[j], w1[j + 1], w2[j], w2[j + 1]],
                            wr,
                            -wi,
                        );
                        if scale != 1. {
                            for k in ki..n {
                                w1[k] *= scale;
                                w2[k] *= scale;
                            }
                        }
                        w1[j] = x[0];
                        w2[j] = x[2];
                        w1[j + 1] = x[1];
                        w2[j + 1] = x[3];
                        vmax = x[0]
                            .abs()
                            .max(x[2].abs())
                            .max(x[1].abs())
                            .max(x[3].abs())
                            .max(vmax);
                        vcrit = bignum / vmax;
                        j += 2;
                    }
                }
                if ki + 2 < n {
                    let x1: Vec<f64> = w1[ki + 2..n].to_vec();
                    let x2: Vec<f64> = w2[ki + 2..n].to_vec();
                    gemv_into_col(vl, ki + 2, n - ki - 2, &x1, w1[ki], ki);
                    gemv_into_col(vl, ki + 2, n - ki - 2, &x2, w2[ki + 1], ki + 1);
                } else {
                    for i in 0..n {
                        *vl.get_mut(i, ki) *= w1[ki];
                        *vl.get_mut(i, ki + 1) *= w2[ki + 1];
                    }
                }
                let mut emax = 0f64;
                for k in 0..n {
                    emax = emax.max(vl.at(k, ki).abs() + vl.at(k, ki + 1).abs());
                }
                let remax = 1. / emax;
                for i in 0..n {
                    *vl.get_mut(i, ki) *= remax;
                    *vl.get_mut(i, ki + 1) *= remax;
                }
            }
            if ip == 1 {
                ip = -1;
            }
        }
    }
}

/// dlartg (LAPACK 3.5): (cs, sn, r) with r = sqrt(f^2+g^2) up to sign.
pub(crate) fn lartg(f: f64, g: f64) -> (f64, f64, f64) {
    let safmn2 = 2f64.powi(((SAFMIN / EPS).ln() / 2f64.ln() / 2.) as i32);
    let safmx2 = 1. / safmn2;
    if g == 0. {
        return (1., 0., f);
    }
    if f == 0. {
        return (0., 1., g);
    }
    let mut f1 = f;
    let mut g1 = g;
    let mut scale = f1.abs().max(g1.abs());
    let (mut cs, mut sn, mut r);
    if scale >= safmx2 {
        let mut count = 0;
        loop {
            count += 1;
            f1 *= safmn2;
            g1 *= safmn2;
            scale = f1.abs().max(g1.abs());
            if scale < safmx2 || count >= 20 {
                break;
            }
        }
        r = (f1 * f1 + g1 * g1).sqrt();
        cs = f1 / r;
        sn = g1 / r;
        for _ in 0..count {
            r *= safmx2;
        }
    } else if scale <= safmn2 {
        let mut count = 0;
        loop {
            count += 1;
            f1 *= safmx2;
            g1 *= safmx2;
            scale = f1.abs().max(g1.abs());
            if scale > safmn2 {
                break;
            }
        }
        r = (f1 * f1 + g1 * g1).sqrt();
        cs = f1 / r;
        sn = g1 / r;
        for _ in 0..count {
            r *= safmn2;
        }
    } else {
        r = (f1 * f1 + g1 * g1).sqrt();
        cs = f1 / r;
        sn = g1 / r;
    }
    if f.abs() > g.abs() && cs < 0. {
        cs = -cs;
        sn = -sn;
        r = -r;
    }
    (cs, sn, r)
}

/// Normalise eigenvector columns as dgeevx does: unit 2-norm, and for a
/// complex pair (wi > 0 at column i) the largest component made real.
pub(crate) fn normalize_vectors(v: &mut Mat, wi: &[f64]) {
    let n = v.rows;
    let mut i = 0;
    while i < v.cols {
        if wi[i] == 0. {
            let scl = 1. / nrm2(v.col(i).iter());
            for k in 0..n {
                *v.get_mut(k, i) *= scl;
            }
            i += 1;
        } else if wi[i] > 0. {
            let scl = 1. / lapy2(nrm2(v.col(i).iter()), nrm2(v.col(i + 1).iter()));
            for k in 0..n {
                *v.get_mut(k, i) *= scl;
                *v.get_mut(k, i + 1) *= scl;
            }
            let work: Vec<f64> = (0..n)
                .map(|k| v.at(k, i) * v.at(k, i) + v.at(k, i + 1) * v.at(k, i + 1))
                .collect();
            let k = idamax(work.iter());
            let (cs, sn, _) = lartg(v.at(k, i), v.at(k, i + 1));
            for r in 0..n {
                let (mut x, mut y) = (v.at(r, i), v.at(r, i + 1));
                rot_pair(&mut x, &mut y, cs, sn);
                v.set(r, i, x);
                v.set(r, i + 1, y);
            }
            v.set(k, i + 1, 0.);
            i += 2;
        } else {
            i += 1;
        }
    }
}

/// Output of [`geevx`].
pub(crate) struct Geev {
    pub wr: Vec<f64>,
    pub wi: Vec<f64>,
    pub vl: Option<Mat>,
    pub vr: Option<Mat>,
}

/// dgeevx with balanc 'B' (`balance`) or 'N', sense 'N'.
pub(crate) fn geevx(
    a: &Mat,
    balance: bool,
    want_vl: bool,
    want_vr: bool,
    cancel: &dyn Fn() -> bool,
) -> Result<Geev, usize> {
    let n = a.rows;
    let mut h = a.clone();
    let anrm = h.data.iter().fold(0f64, |s, x| s.max(x.abs()));
    let cscale = eigen_scale(anrm);
    if cscale != anrm {
        lascl(&mut h.data, anrm, cscale);
    }
    let (ilo, ihi, scale) = gebal(&mut h, balance, balance);
    if n == 0 {
        return Ok(Geev {
            wr: vec![],
            wi: vec![],
            vl: want_vl.then(|| Mat::zeros(0, 0)),
            vr: want_vr.then(|| Mat::zeros(0, 0)),
        });
    }
    let tau = gehd2(&mut h, ilo, ihi, cancel)?;
    let (mut wr, mut wi, mut vl, mut vr);
    if want_vl || want_vr {
        let mut q = orghr(&h, ilo, ihi, &tau, cancel);
        let r = hseqr(true, &mut h, ilo, ihi, Some(&mut q), cancel)?;
        wr = r.0;
        wi = r.1;
        vl = want_vl.then(|| q.clone());
        vr = want_vr.then_some(q);
        trevc(&h, vl.as_mut(), vr.as_mut(), cancel);
    } else {
        let r = hseqr(false, &mut h, ilo, ihi, None, cancel)?;
        wr = r.0;
        wi = r.1;
        vl = None;
        vr = None;
    }
    if let Some(v) = vl.as_mut() {
        gebak(v, ilo, ihi, &scale, balance, balance, false);
        normalize_vectors(v, &wi);
    }
    if let Some(v) = vr.as_mut() {
        gebak(v, ilo, ihi, &scale, balance, balance, true);
        normalize_vectors(v, &wi);
    }
    if cscale != anrm {
        lascl(&mut wr, cscale, anrm);
        lascl(&mut wi, cscale, anrm);
    }
    Ok(Geev { wr, wi, vl, vr })
}

/// xGEEVX's safe range before balancing, Hessenberg reduction and QR.
pub(crate) fn eigen_scale(norm: f64) -> f64 {
    let small = SAFMIN.sqrt() / ULP;
    if norm == 0. {
        0.
    } else {
        norm.clamp(small, 1. / small)
    }
}

/// DLASCL's staged multipliers avoid overflow in to/from itself.
pub(crate) fn lascl(data: &mut [f64], mut from: f64, mut to: f64) {
    let large = 1. / SAFMIN;
    loop {
        let from1 = from * SAFMIN;
        let to1 = to / large;
        let (mul, done) = if from1.abs() > to.abs() && to != 0. {
            from = from1;
            (SAFMIN, false)
        } else if to1.abs() > from.abs() {
            to = to1;
            (large, false)
        } else {
            (to / from, true)
        };
        for x in data.iter_mut() {
            *x *= mul;
        }
        if done {
            break;
        }
    }
}
