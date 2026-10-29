//! Real generalized eigenproblem, ported from reference LAPACK 3.5:
//! dggev (dggbal 'P', dgeqr2, dorm2r, dgghrd, dhgeqz, dtgevc, dggbak) and
//! dsygv (dpotf2, dsygs2, dsyev, dtrsm). 0-based, column-major. dtgevc's
//! accumulations are fused as MATLAB's MKL computes them.
// LAPACK ports keep the reference routines' index loops.
#![allow(clippy::needless_range_loop)]
use super::dense::{EPS, Mat, SAFMIN, ULP, laln2, lapy2, larf_left, larfg, lartg, org2r, sign};
use crate::engine::Fma;

#[inline]
fn rot(x: &mut f64, y: &mut f64, c: f64, s: f64) {
    let t = c * *x + s * *y;
    *y = c * *y - s * *x;
    *x = t;
}

/// drot on rows r1, r2 over columns cols.
fn rot_rows(a: &mut Mat, r1: usize, r2: usize, cols: std::ops::Range<usize>, c: f64, s: f64) {
    for j in cols {
        let (mut x, mut y) = (a.at(r1, j), a.at(r2, j));
        rot(&mut x, &mut y, c, s);
        a.set(r1, j, x);
        a.set(r2, j, y);
    }
}

/// drot on columns c1, c2 over rows rows.
fn rot_cols(a: &mut Mat, c1: usize, c2: usize, rows: std::ops::Range<usize>, c: f64, s: f64) {
    for i in rows {
        let (mut x, mut y) = (a.at(i, c1), a.at(i, c2));
        rot(&mut x, &mut y, c, s);
        a.set(i, c1, x);
        a.set(i, c2, y);
    }
}

/// dggbal with job 'P': (ilo, ihi, lscale, rscale), 0-based permutation entries.
pub(crate) fn ggbal_p(a: &mut Mat, b: &mut Mat) -> (usize, usize, Vec<f64>, Vec<f64>) {
    let n = a.rows;
    let mut lscale = vec![1.; n];
    let mut rscale = vec![1.; n];
    if n <= 1 {
        return (0, n.saturating_sub(1), lscale, rscale);
    }
    let nz = |a: &Mat, b: &Mat, i: usize, j: usize| a.at(i, j) != 0. || b.at(i, j) != 0.;
    let mut k = 0usize;
    let mut l = n - 1;
    // Row search (label 30): a row whose only nonzero in columns 0..=l is at column j.
    'outer: loop {
        let mut found = None;
        for i in (0..=l).rev() {
            let first = (0..l).find(|&j| nz(a, b, i, j));
            let j = match first {
                None => l,
                Some(j) => {
                    if (j + 1..=l).any(|jj| nz(a, b, i, jj)) {
                        continue;
                    }
                    j
                }
            };
            found = Some((i, j));
            break;
        }
        let Some((i, j)) = found else { break 'outer };
        let m = l;
        lscale[m] = i as f64;
        if i != m {
            for jj in k..n {
                let (x, y) = (a.at(i, jj), a.at(m, jj));
                a.set(i, jj, y);
                a.set(m, jj, x);
                let (x, y) = (b.at(i, jj), b.at(m, jj));
                b.set(i, jj, y);
                b.set(m, jj, x);
            }
        }
        rscale[m] = j as f64;
        if j != m {
            for ii in 0..=l {
                let (x, y) = (a.at(ii, j), a.at(ii, m));
                a.set(ii, j, y);
                a.set(ii, m, x);
                let (x, y) = (b.at(ii, j), b.at(ii, m));
                b.set(ii, j, y);
                b.set(ii, m, x);
            }
        }
        l -= 1;
        if l == 0 {
            rscale[0] = 1.;
            lscale[0] = 1.;
            return (k, l, lscale, rscale);
        }
    }
    // Column search (label 100).
    loop {
        let mut found = None;
        for j in k..=l {
            let first = (k..l).find(|&i| nz(a, b, i, j));
            let i = match first {
                None => l,
                Some(i) => {
                    if (i + 1..=l).any(|ii| nz(a, b, ii, j)) {
                        continue;
                    }
                    i
                }
            };
            found = Some((i, j));
            break;
        }
        let Some((i, j)) = found else { break };
        let m = k;
        lscale[m] = i as f64;
        if i != m {
            for jj in k..n {
                let (x, y) = (a.at(i, jj), a.at(m, jj));
                a.set(i, jj, y);
                a.set(m, jj, x);
                let (x, y) = (b.at(i, jj), b.at(m, jj));
                b.set(i, jj, y);
                b.set(m, jj, x);
            }
        }
        rscale[m] = j as f64;
        if j != m {
            for ii in 0..=l {
                let (x, y) = (a.at(ii, j), a.at(ii, m));
                a.set(ii, j, y);
                a.set(ii, m, x);
                let (x, y) = (b.at(ii, j), b.at(ii, m));
                b.set(ii, j, y);
                b.set(ii, m, x);
            }
        }
        k += 1;
    }
    for i in k..=l {
        lscale[i] = 1.;
        rscale[i] = 1.;
    }
    (k, l, lscale, rscale)
}

/// dggbak with job 'P' on the rows of v.
pub(crate) fn ggbak_p(v: &mut Mat, ilo: usize, ihi: usize, scale: &[f64]) {
    let n = v.rows;
    let m = v.cols;
    for i in (0..ilo).rev() {
        let k = scale[i] as usize;
        if k != i {
            v.swap_rows(i, k, 0, m);
        }
    }
    for (i, &s) in scale.iter().enumerate().take(n).skip(ihi + 1) {
        let k = s as usize;
        if k != i {
            v.swap_rows(i, k, 0, m);
        }
    }
}

/// dgeqr2 on the block (r0.., c0..) of size m x n; returns tau.
pub(crate) fn geqr2(
    a: &mut Mat,
    r0: usize,
    c0: usize,
    m: usize,
    n: usize,
    cancel: &dyn Fn() -> bool,
) -> Vec<f64> {
    let k = m.min(n);
    let mut tau = vec![0.; k];
    for i in 0..k {
        if cancel() {
            return tau;
        }
        let mut x: Vec<f64> = (i + 1..m).map(|r| a.at(r0 + r, c0 + i)).collect();
        let (beta, t) = larfg(a.at(r0 + i, c0 + i), &mut x);
        for (q, &v) in x.iter().enumerate() {
            a.set(r0 + i + 1 + q, c0 + i, v);
        }
        a.set(r0 + i, c0 + i, beta);
        tau[i] = t;
        if i + 1 < n {
            let mut v = vec![1.];
            v.extend_from_slice(&x);
            larf_left(a, &v, t, r0 + i, c0 + i + 1, n - i - 1);
        }
    }
    tau
}

/// dorm2r 'L','T': C(cr0.., cc0..cc0+nc) := Q^T C with Q from the k
/// reflectors of `qr` at (r0, c0).
#[allow(clippy::too_many_arguments)]
pub(crate) fn orm2r_lt(
    qr: &Mat,
    r0: usize,
    c0: usize,
    m: usize,
    k: usize,
    tau: &[f64],
    c: &mut Mat,
    cr0: usize,
    cc0: usize,
    nc: usize,
    cancel: &dyn Fn() -> bool,
) {
    for i in 0..k {
        if cancel() {
            return;
        }
        let mut v = vec![1.];
        v.extend((i + 1..m).map(|r| qr.at(r0 + r, c0 + i)));
        larf_left(c, &v, tau[i], cr0 + i, cc0, nc);
    }
}

/// dgghrd with Q/Z accumulated when given (compq/compz = 'V').
pub(crate) fn gghrd(
    a: &mut Mat,
    b: &mut Mat,
    ilo: usize,
    ihi: usize,
    mut q: Option<&mut Mat>,
    mut z: Option<&mut Mat>,
    cancel: &dyn Fn() -> bool,
) {
    let n = a.rows;
    for jcol in 0..n.saturating_sub(1) {
        if cancel() {
            return;
        }
        for jrow in jcol + 1..n {
            b.set(jrow, jcol, 0.);
        }
    }
    if ihi < 2 {
        return;
    }
    for jcol in ilo..ihi.saturating_sub(1) {
        if cancel() {
            return;
        }
        let mut jrow = ihi;
        while jrow >= jcol + 2 {
            let (c, s, r) = lartg(a.at(jrow - 1, jcol), a.at(jrow, jcol));
            a.set(jrow - 1, jcol, r);
            a.set(jrow, jcol, 0.);
            rot_rows(a, jrow - 1, jrow, jcol + 1..n, c, s);
            rot_rows(b, jrow - 1, jrow, jrow - 1..n, c, s);
            if let Some(q) = q.as_deref_mut() {
                rot_cols(q, jrow - 1, jrow, 0..n, c, s);
            }
            let (c, s, r) = lartg(b.at(jrow, jrow), b.at(jrow, jrow - 1));
            b.set(jrow, jrow, r);
            b.set(jrow, jrow - 1, 0.);
            rot_cols(a, jrow, jrow - 1, 0..ihi + 1, c, s);
            rot_cols(b, jrow, jrow - 1, 0..jrow, c, s);
            if let Some(z) = z.as_deref_mut() {
                rot_cols(z, jrow, jrow - 1, 0..n, c, s);
            }
            jrow -= 1;
        }
    }
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

/// dlag2: (scale1, scale2, wr1, wr2, wi) for the 2x2 pencil at (r, r).
pub(crate) fn lag2(a: &Mat, b: &Mat, r: usize, safmin: f64) -> (f64, f64, f64, f64, f64) {
    const FUZZY1: f64 = 1. + 1.0e-5;
    let rtmin = safmin.sqrt();
    let rtmax = 1. / rtmin;
    let safmax = 1. / safmin;
    let anorm = (a.at(r, r).abs() + a.at(r + 1, r).abs())
        .max(a.at(r, r + 1).abs() + a.at(r + 1, r + 1).abs())
        .max(safmin);
    let ascale = 1. / anorm;
    let a11 = ascale * a.at(r, r);
    let a21 = ascale * a.at(r + 1, r);
    let a12 = ascale * a.at(r, r + 1);
    let a22 = ascale * a.at(r + 1, r + 1);
    let mut b11 = b.at(r, r);
    let mut b12 = b.at(r, r + 1);
    let mut b22 = b.at(r + 1, r + 1);
    let bmin = rtmin * b11.abs().max(b12.abs()).max(b22.abs()).max(rtmin);
    if b11.abs() < bmin {
        b11 = sign(bmin, b11);
    }
    if b22.abs() < bmin {
        b22 = sign(bmin, b22);
    }
    let bnorm = b11.abs().max(b12.abs() + b22.abs()).max(safmin);
    let bsize = b11.abs().max(b22.abs());
    let bscale = 1. / bsize;
    b11 *= bscale;
    b12 *= bscale;
    b22 *= bscale;
    let binv11 = 1. / b11;
    let binv22 = 1. / b22;
    let s1 = a11 * binv11;
    let s2 = a22 * binv22;
    let (as12, abi22, pp, shift, ss);
    if s1.abs() <= s2.abs() {
        as12 = a12 - s1 * b12;
        let as22 = a22 - s1 * b22;
        ss = a21 * (binv11 * binv22);
        abi22 = as22 * binv22 - ss * b12;
        pp = 0.5 * abi22;
        shift = s1;
    } else {
        as12 = a12 - s2 * b12;
        let as11 = a11 - s2 * b11;
        ss = a21 * (binv11 * binv22);
        abi22 = -ss * b12;
        pp = 0.5 * (as11 * binv11 + abi22);
        shift = s2;
    }
    let qq = ss * as12;
    let (discr, rr);
    if (pp * rtmin).abs() >= 1. {
        discr = (rtmin * pp) * (rtmin * pp) + qq * safmin;
        rr = discr.abs().sqrt() * rtmax;
    } else if pp * pp + qq.abs() <= safmin {
        discr = (rtmax * pp) * (rtmax * pp) + qq * safmax;
        rr = discr.abs().sqrt() * rtmin;
    } else {
        discr = pp * pp + qq;
        rr = discr.abs().sqrt();
    }
    let (mut wr1, mut wr2, mut wi);
    if discr >= 0. || rr == 0. {
        let sum = pp + sign(rr, pp);
        let diff = pp - sign(rr, pp);
        let wbig = shift + sum;
        let mut wsmall = shift + diff;
        if 0.5 * wbig.abs() > wsmall.abs().max(safmin) {
            let wdet = (a11 * a22 - a12 * a21) * (binv11 * binv22);
            wsmall = wdet / wbig;
        }
        if pp > abi22 {
            wr1 = wbig.min(wsmall);
            wr2 = wbig.max(wsmall);
        } else {
            wr1 = wbig.max(wsmall);
            wr2 = wbig.min(wsmall);
        }
        wi = 0.;
    } else {
        wr1 = shift + pp;
        wr2 = wr1;
        wi = rr;
    }
    let c1 = bsize * (safmin * 1f64.max(ascale));
    let c2 = safmin * 1f64.max(bnorm);
    let c3 = bsize * safmin;
    let c4 = if ascale <= 1. && bsize <= 1. {
        1f64.min((ascale / safmin) * bsize)
    } else {
        1.
    };
    let c5 = if ascale <= 1. || bsize <= 1. {
        1f64.min(ascale * bsize)
    } else {
        1.
    };
    let wabs = wr1.abs() + wi.abs();
    let wsize = safmin
        .max(c1)
        .max(FUZZY1 * (wabs * c2 + c3))
        .max(c4.min(0.5 * wabs.max(c5)));
    let scale1;
    let mut scale2 = 0.;
    if wsize != 1. {
        let wscale = 1. / wsize;
        scale1 = if wsize > 1. {
            (ascale.max(bsize) * wscale) * ascale.min(bsize)
        } else {
            (ascale.min(bsize) * wscale) * ascale.max(bsize)
        };
        wr1 *= wscale;
        if wi != 0. {
            wi *= wscale;
            wr2 = wr1;
            scale2 = scale1;
        }
    } else {
        scale1 = ascale * bsize;
        scale2 = scale1;
    }
    if wi == 0. {
        let wsize = safmin
            .max(c1)
            .max(FUZZY1 * (wr2.abs() * c2 + c3))
            .max(c4.min(0.5 * wr2.abs().max(c5)));
        if wsize != 1. {
            let wscale = 1. / wsize;
            scale2 = if wsize > 1. {
                (ascale.max(bsize) * wscale) * ascale.min(bsize)
            } else {
                (ascale.min(bsize) * wscale) * ascale.max(bsize)
            };
            wr2 *= wscale;
        } else {
            scale2 = ascale * bsize;
        }
    }
    (scale1, scale2, wr1, wr2, wi)
}

/// dlasv2: (ssmin, ssmax, snr, csr, snl, csl).
pub(crate) fn lasv2(f: f64, g: f64, h: f64) -> (f64, f64, f64, f64, f64, f64) {
    let mut ft = f;
    let mut fa = ft.abs();
    let mut ht = h;
    let mut ha = h.abs();
    let mut pmax = 1;
    let swap = ha > fa;
    if swap {
        pmax = 3;
        std::mem::swap(&mut ft, &mut ht);
        std::mem::swap(&mut fa, &mut ha);
    }
    let gt = g;
    let ga = gt.abs();
    let (mut ssmin, mut ssmax, clt, crt, slt, srt);
    if ga == 0. {
        ssmin = ha;
        ssmax = fa;
        clt = 1.;
        crt = 1.;
        slt = 0.;
        srt = 0.;
    } else {
        let mut gasmal = true;
        let (mut c_lt, mut c_rt, mut s_lt, mut s_rt) = (0., 0., 0., 0.);
        ssmin = 0.;
        ssmax = 0.;
        if ga > fa {
            pmax = 2;
            if fa / ga < EPS {
                gasmal = false;
                ssmax = ga;
                ssmin = if ha > 1. {
                    fa / (ga / ha)
                } else {
                    (fa / ga) * ha
                };
                c_lt = 1.;
                s_lt = ht / gt;
                s_rt = 1.;
                c_rt = ft / gt;
            }
        }
        if gasmal {
            let d = fa - ha;
            let mut l = if d == fa { 1. } else { d / fa };
            let m = gt / ft;
            let mut t = 2. - l;
            let mm = m * m;
            let tt = t * t;
            let s = (tt + mm).sqrt();
            let r = if l == 0. {
                m.abs()
            } else {
                (l * l + mm).sqrt()
            };
            let a = 0.5 * (s + r);
            ssmin = ha / a;
            ssmax = fa * a;
            if mm == 0. {
                t = if l == 0. {
                    sign(2., ft) * sign(1., gt)
                } else {
                    gt / sign(d, ft) + m / t
                };
            } else {
                t = (m / (s + t) + m / (r + l)) * (1. + a);
            }
            l = (t * t + 4.).sqrt();
            c_rt = 2. / l;
            s_rt = t / l;
            c_lt = (c_rt + s_rt * m) / a;
            s_lt = (ht / ft) * s_rt / a;
        }
        clt = c_lt;
        crt = c_rt;
        slt = s_lt;
        srt = s_rt;
    }
    let (csl, snl, csr, snr) = if swap {
        (srt, crt, slt, clt)
    } else {
        (clt, slt, crt, srt)
    };
    let tsign = match pmax {
        1 => sign(1., csr) * sign(1., csl) * sign(1., f),
        2 => sign(1., snr) * sign(1., csl) * sign(1., g),
        _ => sign(1., snr) * sign(1., snl) * sign(1., h),
    };
    ssmax = sign(ssmax, tsign);
    ssmin = sign(ssmin, tsign * sign(1., f) * sign(1., h));
    (ssmin, ssmax, snr, csr, snl, csl)
}

/// Output of [`hgeqz`].
pub(crate) struct Hgeqz {
    pub alphar: Vec<f64>,
    pub alphai: Vec<f64>,
    pub beta: Vec<f64>,
}

/// dhgeqz. `schur` selects job 'S'; q and z are updated when given.
#[allow(clippy::too_many_arguments)]
pub(crate) fn hgeqz(
    schur: bool,
    h: &mut Mat,
    t: &mut Mat,
    ilo: usize,
    ihi: usize,
    q: Option<&mut Mat>,
    z: Option<&mut Mat>,
    cancel: &dyn Fn() -> bool,
) -> Result<Hgeqz, usize> {
    hgeqz_with_rounding(schur, h, t, ilo, ihi, q, z, cancel, false)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn hgeqz_with_rounding(
    schur: bool,
    h: &mut Mat,
    t: &mut Mat,
    ilo: usize,
    ihi: usize,
    mut q: Option<&mut Mat>,
    mut z: Option<&mut Mat>,
    cancel: &dyn Fn() -> bool,
    fused_sweeps: bool,
) -> Result<Hgeqz, usize> {
    let n = h.rows;
    // QZ's Schur factors use fused mixed-shift sweeps; GGEV retains the
    // established eigenvalue/vector rounding. The standalone 2x2 path agrees.
    let qz_madd = |a: f64, b: f64, c: f64, d: f64| {
        if fused_sweeps && n > 2 {
            a.fma(b, c * d)
        } else {
            a * b + c * d
        }
    };
    let mut alphar = vec![0.; n];
    let mut alphai = vec![0.; n];
    let mut beta = vec![0.; n];
    if n == 0 {
        return Ok(Hgeqz {
            alphar,
            alphai,
            beta,
        });
    }
    const SAFETY: f64 = 100.;
    let safmin = SAFMIN;
    let safmax = 1. / safmin;
    let ulp = ULP;
    // dlanhs 'F' of the active block.
    let fro = |m: &Mat| {
        let mut scale = 0f64;
        let mut ssq = 1f64;
        for j in ilo..=ihi {
            for i in ilo..=(j + 1).min(ihi) {
                let v = m.at(i, j);
                if v != 0. {
                    let a = v.abs();
                    if scale < a {
                        ssq = 1. + ssq * (scale / a) * (scale / a);
                        scale = a;
                    } else {
                        ssq += (a / scale) * (a / scale);
                    }
                }
            }
        }
        scale * ssq.sqrt()
    };
    let anorm = fro(h);
    let bnorm = fro(t);
    let atol = safmin.max(ulp * anorm);
    let btol = safmin.max(ulp * bnorm);
    let ascale = 1. / safmin.max(anorm);
    let bscale = 1. / safmin.max(bnorm);
    let negate_col = |h: &mut Mat, t: &mut Mat, z: &mut Option<&mut Mat>, j: usize, from: usize| {
        if schur {
            for jr in from..=j {
                *h.get_mut(jr, j) = -h.at(jr, j);
                *t.get_mut(jr, j) = -t.at(jr, j);
            }
        } else {
            *h.get_mut(j, j) = -h.at(j, j);
            *t.get_mut(j, j) = -t.at(j, j);
        }
        if let Some(z) = z.as_deref_mut() {
            for jr in 0..n {
                *z.get_mut(jr, j) = -z.at(jr, j);
            }
        }
    };
    for j in ihi + 1..n {
        if t.at(j, j) < 0. {
            negate_col(h, t, &mut z, j, 0);
        }
        alphar[j] = h.at(j, j);
        alphai[j] = 0.;
        beta[j] = t.at(j, j);
    }
    // 0-based indices; ilast etc. as in the Fortran minus one.
    let mut ilast = ihi as isize;
    let (mut ifrstm, mut ilastm) = if schur { (0usize, n - 1) } else { (ilo, ihi) };
    let mut iiter = 0usize;
    let mut eshift = 0f64;
    let maxit = 30 * (ihi - ilo + 1);
    let mut finished = ilast < ilo as isize;
    let mut jiter = 0;
    while !finished {
        jiter += 1;
        if jiter > maxit {
            return Err(ilast as usize + 1);
        }
        if cancel() {
            return Err(ilast as usize + 1);
        }
        let il = ilast as usize;
        // Labels: 70 (T(ilast,ilast)=0), 80 (deflate ilast), 110 (QZ step from ifirst).
        enum Next {
            L70,
            L80,
            L110(usize),
        }
        let next = 'split: {
            if il == ilo {
                break 'split Next::L80;
            } else if h.at(il, il - 1).abs() <= atol {
                h.set(il, il - 1, 0.);
                break 'split Next::L80;
            }
            if t.at(il, il).abs() <= btol {
                t.set(il, il, 0.);
                break 'split Next::L70;
            }
            let mut j = il as isize - 1;
            while j >= ilo as isize {
                let ju = j as usize;
                let ilazro = if ju == ilo {
                    true
                } else if h.at(ju, ju - 1).abs() <= atol {
                    h.set(ju, ju - 1, 0.);
                    true
                } else {
                    false
                };
                if t.at(ju, ju).abs() < btol {
                    t.set(ju, ju, 0.);
                    let mut ilazr2 = false;
                    if !ilazro {
                        let mut temp = h.at(ju, ju - 1).abs();
                        let mut temp2 = h.at(ju, ju).abs();
                        let tempr = temp.max(temp2);
                        if tempr < 1. && tempr != 0. {
                            temp /= tempr;
                            temp2 /= tempr;
                        }
                        if temp * (ascale * h.at(ju + 1, ju).abs()) <= temp2 * (ascale * atol) {
                            ilazr2 = true;
                        }
                    }
                    if ilazro || ilazr2 {
                        for jch in ju..il {
                            let (c, s, r) = lartg(h.at(jch, jch), h.at(jch + 1, jch));
                            h.set(jch, jch, r);
                            h.set(jch + 1, jch, 0.);
                            rot_rows(h, jch, jch + 1, jch + 1..ilastm + 1, c, s);
                            rot_rows(t, jch, jch + 1, jch + 1..ilastm + 1, c, s);
                            if let Some(q) = q.as_deref_mut() {
                                rot_cols(q, jch, jch + 1, 0..n, c, s);
                            }
                            if ilazr2 {
                                *h.get_mut(jch, jch - 1) *= c;
                            }
                            ilazr2 = false;
                            if t.at(jch + 1, jch + 1).abs() >= btol {
                                if jch + 1 >= il {
                                    break 'split Next::L80;
                                }
                                break 'split Next::L110(jch + 1);
                            }
                            t.set(jch + 1, jch + 1, 0.);
                        }
                        break 'split Next::L70;
                    }
                    for jch in ju..il {
                        let (c, s, r) = lartg(t.at(jch, jch + 1), t.at(jch + 1, jch + 1));
                        t.set(jch, jch + 1, r);
                        t.set(jch + 1, jch + 1, 0.);
                        if jch + 1 < ilastm {
                            rot_rows(t, jch, jch + 1, jch + 2..ilastm + 1, c, s);
                        }
                        rot_rows(h, jch, jch + 1, jch - 1..ilastm + 1, c, s);
                        if let Some(q) = q.as_deref_mut() {
                            rot_cols(q, jch, jch + 1, 0..n, c, s);
                        }
                        let (c, s, r) = lartg(h.at(jch + 1, jch), h.at(jch + 1, jch - 1));
                        h.set(jch + 1, jch, r);
                        h.set(jch + 1, jch - 1, 0.);
                        rot_cols(h, jch, jch - 1, ifrstm..jch + 1, c, s);
                        rot_cols(t, jch, jch - 1, ifrstm..jch, c, s);
                        if let Some(z) = z.as_deref_mut() {
                            rot_cols(z, jch, jch - 1, 0..n, c, s);
                        }
                    }
                    break 'split Next::L70;
                } else if ilazro {
                    break 'split Next::L110(ju);
                }
                j -= 1;
            }
            return Err(n + 1);
        };
        let ifirst = match next {
            Next::L70 | Next::L80 => {
                if matches!(next, Next::L70) {
                    let (c, s, r) = lartg(h.at(il, il), h.at(il, il - 1));
                    h.set(il, il, r);
                    h.set(il, il - 1, 0.);
                    rot_cols(h, il, il - 1, ifrstm..il, c, s);
                    rot_cols(t, il, il - 1, ifrstm..il, c, s);
                    if let Some(z) = z.as_deref_mut() {
                        rot_cols(z, il, il - 1, 0..n, c, s);
                    }
                }
                if t.at(il, il) < 0. {
                    negate_col(h, t, &mut z, il, ifrstm);
                }
                alphar[il] = h.at(il, il);
                alphai[il] = 0.;
                beta[il] = t.at(il, il);
                ilast -= 1;
                if ilast < ilo as isize {
                    finished = true;
                    continue;
                }
                iiter = 0;
                eshift = 0.;
                if !schur {
                    ilastm = ilast as usize;
                    if ifrstm > ilastm {
                        ifrstm = ilo;
                    }
                }
                continue;
            }
            Next::L110(f) => f,
        };
        // QZ step on ifirst..=il.
        iiter += 1;
        if !schur {
            ifrstm = ifirst;
        }
        let (mut s1, mut wr, wi);
        if iiter.is_multiple_of(10) {
            if ((maxit as f64) * safmin) * h.at(il, il - 1).abs() < t.at(il - 1, il - 1).abs() {
                eshift = h.at(il, il - 1) / t.at(il - 1, il - 1);
            } else {
                eshift += 1. / (safmin * maxit as f64);
            }
            s1 = 1.;
            wr = eshift;
            wi = 0.;
        } else {
            let (sc1, sc2, w1, w2, w_i) = lag2(h, t, il - 1, safmin * SAFETY);
            s1 = sc1;
            wr = w1;
            let (mut wr2, mut s2) = (w2, sc2);
            if ((wr / s1) * t.at(il, il) - h.at(il, il)).abs()
                > ((wr2 / s2) * t.at(il, il) - h.at(il, il)).abs()
            {
                std::mem::swap(&mut wr, &mut wr2);
                std::mem::swap(&mut s1, &mut s2);
            }
            wi = w_i;
        }
        if wi == 0. {
            // Fiddle with shift to avoid overflow.
            let temp = ascale.min(1.) * (0.5 * safmax);
            let mut scale = if s1 > temp { temp / s1 } else { 1. };
            let temp = bscale.min(1.) * (0.5 * safmax);
            if wr.abs() > temp {
                scale = scale.min(temp / wr.abs());
            }
            s1 *= scale;
            wr *= scale;
            let mut istart = ifirst;
            let mut j = il - 1;
            while j > ifirst {
                let mut temp = (s1 * h.at(j, j - 1)).abs();
                let mut temp2 = (s1 * h.at(j, j) - wr * t.at(j, j)).abs();
                let tempr = temp.max(temp2);
                if tempr < 1. && tempr != 0. {
                    temp /= tempr;
                    temp2 /= tempr;
                }
                if ((ascale * h.at(j + 1, j)) * temp).abs() <= (ascale * atol) * temp2 {
                    istart = j;
                    break;
                }
                j -= 1;
            }
            let temp = s1 * h.at(istart, istart) - wr * t.at(istart, istart);
            let temp2 = s1 * h.at(istart + 1, istart);
            let (mut c, mut s, _) = lartg(temp, temp2);
            for j in istart..il {
                if j > istart {
                    let (cc, ss, r) = lartg(h.at(j, j - 1), h.at(j + 1, j - 1));
                    c = cc;
                    s = ss;
                    h.set(j, j - 1, r);
                    h.set(j + 1, j - 1, 0.);
                }
                for jc in j..=ilastm {
                    let temp = qz_madd(c, h.at(j, jc), s, h.at(j + 1, jc));
                    h.set(j + 1, jc, qz_madd(-s, h.at(j, jc), c, h.at(j + 1, jc)));
                    h.set(j, jc, temp);
                    let temp2 = qz_madd(c, t.at(j, jc), s, t.at(j + 1, jc));
                    t.set(j + 1, jc, qz_madd(-s, t.at(j, jc), c, t.at(j + 1, jc)));
                    t.set(j, jc, temp2);
                }
                if let Some(q) = q.as_deref_mut() {
                    for jr in 0..n {
                        let temp = qz_madd(c, q.at(jr, j), s, q.at(jr, j + 1));
                        q.set(jr, j + 1, qz_madd(-s, q.at(jr, j), c, q.at(jr, j + 1)));
                        q.set(jr, j, temp);
                    }
                }
                let (cc, ss, r) = lartg(t.at(j + 1, j + 1), t.at(j + 1, j));
                c = cc;
                s = ss;
                t.set(j + 1, j + 1, r);
                t.set(j + 1, j, 0.);
                for jr in ifrstm..=(j + 2).min(il) {
                    let temp = qz_madd(c, h.at(jr, j + 1), s, h.at(jr, j));
                    h.set(jr, j, qz_madd(-s, h.at(jr, j + 1), c, h.at(jr, j)));
                    h.set(jr, j + 1, temp);
                }
                for jr in ifrstm..=j {
                    let temp = qz_madd(c, t.at(jr, j + 1), s, t.at(jr, j));
                    t.set(jr, j, qz_madd(-s, t.at(jr, j + 1), c, t.at(jr, j)));
                    t.set(jr, j + 1, temp);
                }
                if let Some(z) = z.as_deref_mut() {
                    for jr in 0..n {
                        let temp = qz_madd(c, z.at(jr, j + 1), s, z.at(jr, j));
                        z.set(jr, j, qz_madd(-s, z.at(jr, j + 1), c, z.at(jr, j)));
                        z.set(jr, j + 1, temp);
                    }
                }
            }
            continue;
        }
        // Francis double shift (label 200).
        if ifirst + 1 == il {
            // 2x2 block with complex eigenvalues: standardize B.
            let (b22_0, b11_0, sr0, cr0, sl, cl) =
                lasv2(t.at(il - 1, il - 1), t.at(il - 1, il), t.at(il, il));
            let (mut b22, mut b11, mut sr, mut cr) = (b22_0, b11_0, sr0, cr0);
            if b11 < 0. {
                cr = -cr;
                sr = -sr;
                b11 = -b11;
                b22 = -b22;
            }
            rot_rows(h, il - 1, il, il - 1..ilastm + 1, cl, sl);
            rot_cols(h, il - 1, il, ifrstm..il + 1, cr, sr);
            if il < ilastm {
                rot_rows(t, il - 1, il, il + 1..ilastm + 1, cl, sl);
            }
            if ifrstm + 1 < il {
                rot_cols(t, il - 1, il, ifrstm..ifirst, cr, sr);
            }
            if let Some(q) = q.as_deref_mut() {
                rot_cols(q, il - 1, il, 0..n, cl, sl);
            }
            if let Some(z) = z.as_deref_mut() {
                rot_cols(z, il - 1, il, 0..n, cr, sr);
            }
            t.set(il - 1, il - 1, b11);
            t.set(il - 1, il, 0.);
            t.set(il, il - 1, 0.);
            t.set(il, il, b22);
            if b22 < 0. {
                for j in ifrstm..=il {
                    *h.get_mut(j, il) = -h.at(j, il);
                    *t.get_mut(j, il) = -t.at(j, il);
                }
                if let Some(z) = z.as_deref_mut() {
                    for j in 0..n {
                        *z.get_mut(j, il) = -z.at(j, il);
                    }
                }
                b22 = -b22;
            }
            let (s1, _, wr, _, wi) = lag2(h, t, il - 1, safmin * SAFETY);
            if wi == 0. {
                continue;
            }
            let s1inv = 1. / s1;
            let a11 = h.at(il - 1, il - 1);
            let a21 = h.at(il, il - 1);
            let a12 = h.at(il - 1, il);
            let a22 = h.at(il, il);
            let c11r = s1 * a11 - wr * b11;
            let c11i = -wi * b11;
            let c12 = s1 * a12;
            let c21 = s1 * a21;
            let c22r = s1 * a22 - wr * b22;
            let c22i = -wi * b22;
            let (cz, szr, szi);
            if c11r.abs() + c11i.abs() + c12.abs() > c21.abs() + c22r.abs() + c22i.abs() {
                let t1 = lapy3(c12, c11r, c11i);
                cz = c12 / t1;
                szr = -c11r / t1;
                szi = -c11i / t1;
            } else {
                let czz = lapy2(c22r, c22i);
                if czz <= safmin {
                    cz = 0.;
                    szr = 1.;
                    szi = 0.;
                } else {
                    let tempr = c22r / czz;
                    let tempi = c22i / czz;
                    let t1 = lapy2(czz, c21);
                    cz = czz / t1;
                    szr = -c21 * tempr / t1;
                    szi = c21 * tempi / t1;
                }
            }
            let an = a11.abs() + a12.abs() + a21.abs() + a22.abs();
            let bn = b11.abs() + b22.abs();
            let wabs = wr.abs() + wi.abs();
            let (mut cq, mut sqr, mut sqi);
            if s1 * an > wabs * bn {
                cq = cz * b11;
                sqr = szr * b22;
                sqi = -szi * b22;
            } else {
                let a1r = cz * a11 + szr * a12;
                let a1i = szi * a12;
                let a2r = cz * a21 + szr * a22;
                let a2i = szi * a22;
                cq = lapy2(a1r, a1i);
                if cq <= safmin {
                    cq = 0.;
                    sqr = 1.;
                    sqi = 0.;
                } else {
                    let tempr = a1r / cq;
                    let tempi = a1i / cq;
                    sqr = tempr * a2r + tempi * a2i;
                    sqi = tempi * a2r - tempr * a2i;
                }
            }
            let t1 = lapy3(cq, sqr, sqi);
            cq /= t1;
            sqr /= t1;
            sqi /= t1;
            let tempr = sqr * szr - sqi * szi;
            let tempi = sqr * szi + sqi * szr;
            let b1r = cq * cz * b11 + tempr * b22;
            let b1i = tempi * b22;
            let b1a = lapy2(b1r, b1i);
            let b2r = cq * cz * b22 + tempr * b11;
            let b2i = -tempi * b11;
            let b2a = lapy2(b2r, b2i);
            beta[il - 1] = b1a;
            beta[il] = b2a;
            alphar[il - 1] = (wr * b1a) * s1inv;
            alphai[il - 1] = (wi * b1a) * s1inv;
            alphar[il] = (wr * b2a) * s1inv;
            alphai[il] = -(wi * b2a) * s1inv;
            ilast = ifirst as isize - 1;
            if ilast < ilo as isize {
                finished = true;
                continue;
            }
            iiter = 0;
            eshift = 0.;
            if !schur {
                ilastm = ilast as usize;
                if ifrstm > ilastm {
                    ifrstm = ilo;
                }
            }
            continue;
        }
        // Usual case: 3x3 or larger block, implicit double shift.
        let ad11 = (ascale * h.at(il - 1, il - 1)) / (bscale * t.at(il - 1, il - 1));
        let ad21 = (ascale * h.at(il, il - 1)) / (bscale * t.at(il - 1, il - 1));
        let ad12 = (ascale * h.at(il - 1, il)) / (bscale * t.at(il, il));
        let ad22 = (ascale * h.at(il, il)) / (bscale * t.at(il, il));
        let u12 = t.at(il - 1, il) / t.at(il, il);
        let f = ifirst;
        let ad11l = (ascale * h.at(f, f)) / (bscale * t.at(f, f));
        let ad21l = (ascale * h.at(f + 1, f)) / (bscale * t.at(f, f));
        let ad12l = (ascale * h.at(f, f + 1)) / (bscale * t.at(f + 1, f + 1));
        let ad22l = (ascale * h.at(f + 1, f + 1)) / (bscale * t.at(f + 1, f + 1));
        let ad32l = (ascale * h.at(f + 2, f + 1)) / (bscale * t.at(f + 1, f + 1));
        let u12l = t.at(f, f + 1) / t.at(f + 1, f + 1);
        let mut v = [0f64; 3];
        v[0] = (ad11 - ad11l) * (ad22 - ad11l) - ad12 * ad21
            + ad21 * u12 * ad11l
            + (ad12l - ad11l * u12l) * ad21l;
        v[1] =
            ((ad22l - ad11l) - ad21l * u12l - (ad11 - ad11l) - (ad22 - ad11l) + ad21 * u12) * ad21l;
        v[2] = ad32l * ad21l;
        if fused_sweeps {
            v[0] = (ad12l - ad11l * u12l).fma(
                ad21l,
                (ad21 * u12).fma(ad11l, (ad11 - ad11l).fma(ad22 - ad11l, -ad12 * ad21)),
            );
            v[1] = (ad21.fma(
                u12,
                (-ad21l).fma(u12l, ad22l - ad11l) - (ad11 - ad11l) - (ad22 - ad11l),
            )) * ad21l;
        }
        let istart = ifirst;
        let (_, mut tau) = {
            let (head, tail) = v.split_at_mut(1);
            larfg(head[0], tail)
        };
        v[0] = 1.;
        for j in istart..il - 1 {
            if j > istart {
                let mut x = [h.at(j + 1, j - 1), h.at(j + 2, j - 1)];
                let (beta, tt) = larfg(h.at(j, j - 1), &mut x);
                h.set(j, j - 1, beta);
                tau = tt;
                v = [1., x[0], x[1]];
                h.set(j + 1, j - 1, 0.);
                h.set(j + 2, j - 1, 0.);
            }
            for jc in j..=ilastm {
                let temp = tau
                    * if fused_sweeps {
                        v[2].fma(h.at(j + 2, jc), v[1].fma(h.at(j + 1, jc), h.at(j, jc)))
                    } else {
                        h.at(j, jc) + v[1] * h.at(j + 1, jc) + v[2] * h.at(j + 2, jc)
                    };
                *h.get_mut(j, jc) -= temp;
                {
                    let x = h.at(j + 1, jc);
                    h.set(
                        j + 1,
                        jc,
                        if fused_sweeps {
                            (-temp).fma(v[1], x)
                        } else {
                            x - temp * v[1]
                        },
                    );
                }
                {
                    let x = h.at(j + 2, jc);
                    h.set(
                        j + 2,
                        jc,
                        if fused_sweeps {
                            (-temp).fma(v[2], x)
                        } else {
                            x - temp * v[2]
                        },
                    );
                }
                let temp2 = tau
                    * if fused_sweeps {
                        v[2].fma(t.at(j + 2, jc), v[1].fma(t.at(j + 1, jc), t.at(j, jc)))
                    } else {
                        t.at(j, jc) + v[1] * t.at(j + 1, jc) + v[2] * t.at(j + 2, jc)
                    };
                *t.get_mut(j, jc) -= temp2;
                {
                    let x = t.at(j + 1, jc);
                    t.set(
                        j + 1,
                        jc,
                        if fused_sweeps {
                            (-temp2).fma(v[1], x)
                        } else {
                            x - temp2 * v[1]
                        },
                    );
                }
                {
                    let x = t.at(j + 2, jc);
                    t.set(
                        j + 2,
                        jc,
                        if fused_sweeps {
                            (-temp2).fma(v[2], x)
                        } else {
                            x - temp2 * v[2]
                        },
                    );
                }
            }
            if let Some(q) = q.as_deref_mut() {
                for jr in 0..n {
                    let temp = tau
                        * if fused_sweeps {
                            v[2].fma(q.at(jr, j + 2), v[1].fma(q.at(jr, j + 1), q.at(jr, j)))
                        } else {
                            q.at(jr, j) + v[1] * q.at(jr, j + 1) + v[2] * q.at(jr, j + 2)
                        };
                    *q.get_mut(jr, j) -= temp;
                    {
                        let x = q.at(jr, j + 1);
                        q.set(
                            jr,
                            j + 1,
                            if fused_sweeps {
                                (-temp).fma(v[1], x)
                            } else {
                                x - temp * v[1]
                            },
                        );
                    }
                    {
                        let x = q.at(jr, j + 2);
                        q.set(
                            jr,
                            j + 2,
                            if fused_sweeps {
                                (-temp).fma(v[2], x)
                            } else {
                                x - temp * v[2]
                            },
                        );
                    }
                }
            }
            // Zero the j-th column of B.
            let mut ilpivt = false;
            let temp = t.at(j + 1, j + 1).abs().max(t.at(j + 1, j + 2).abs());
            let temp2 = t.at(j + 2, j + 1).abs().max(t.at(j + 2, j + 2).abs());
            let (mut scale, mut u1, mut u2);
            'solve: {
                if temp.max(temp2) < safmin {
                    scale = 0.;
                    u1 = 1.;
                    u2 = 0.;
                    break 'solve;
                }
                let (mut w11, mut w21, mut w12, mut w22);
                if temp >= temp2 {
                    w11 = t.at(j + 1, j + 1);
                    w21 = t.at(j + 2, j + 1);
                    w12 = t.at(j + 1, j + 2);
                    w22 = t.at(j + 2, j + 2);
                    u1 = t.at(j + 1, j);
                    u2 = t.at(j + 2, j);
                } else {
                    w21 = t.at(j + 1, j + 1);
                    w11 = t.at(j + 2, j + 1);
                    w22 = t.at(j + 1, j + 2);
                    w12 = t.at(j + 2, j + 2);
                    u2 = t.at(j + 1, j);
                    u1 = t.at(j + 2, j);
                }
                if w12.abs() > w11.abs() {
                    ilpivt = true;
                    std::mem::swap(&mut w12, &mut w11);
                    std::mem::swap(&mut w22, &mut w21);
                }
                let temp = w21 / w11;
                u2 -= temp * u1;
                w22 -= temp * w12;
                scale = 1.;
                if w22.abs() < safmin {
                    scale = 0.;
                    u2 = 1.;
                    u1 = -w12 / w11;
                    break 'solve;
                }
                if w22.abs() < u2.abs() {
                    scale = (w22 / u2).abs();
                }
                if w11.abs() < u1.abs() {
                    scale = scale.min((w11 / u1).abs());
                }
                u2 = (scale * u2) / w22;
                u1 = (scale * u1 - w12 * u2) / w11;
            }
            if ilpivt {
                std::mem::swap(&mut u1, &mut u2);
            }
            let t1 = (scale * scale + u1 * u1 + u2 * u2).sqrt();
            tau = 1. + scale / t1;
            let vs = -1. / (scale + t1);
            v = [1., vs * u1, vs * u2];
            for jr in ifrstm..=(j + 3).min(il) {
                let temp = tau
                    * if fused_sweeps {
                        v[2].fma(h.at(jr, j + 2), v[1].fma(h.at(jr, j + 1), h.at(jr, j)))
                    } else {
                        h.at(jr, j) + v[1] * h.at(jr, j + 1) + v[2] * h.at(jr, j + 2)
                    };
                *h.get_mut(jr, j) -= temp;
                {
                    let x = h.at(jr, j + 1);
                    h.set(
                        jr,
                        j + 1,
                        if fused_sweeps {
                            (-temp).fma(v[1], x)
                        } else {
                            x - temp * v[1]
                        },
                    );
                }
                {
                    let x = h.at(jr, j + 2);
                    h.set(
                        jr,
                        j + 2,
                        if fused_sweeps {
                            (-temp).fma(v[2], x)
                        } else {
                            x - temp * v[2]
                        },
                    );
                }
            }
            for jr in ifrstm..=j + 2 {
                let temp = tau
                    * if fused_sweeps {
                        v[2].fma(t.at(jr, j + 2), v[1].fma(t.at(jr, j + 1), t.at(jr, j)))
                    } else {
                        t.at(jr, j) + v[1] * t.at(jr, j + 1) + v[2] * t.at(jr, j + 2)
                    };
                *t.get_mut(jr, j) -= temp;
                {
                    let x = t.at(jr, j + 1);
                    t.set(
                        jr,
                        j + 1,
                        if fused_sweeps {
                            (-temp).fma(v[1], x)
                        } else {
                            x - temp * v[1]
                        },
                    );
                }
                {
                    let x = t.at(jr, j + 2);
                    t.set(
                        jr,
                        j + 2,
                        if fused_sweeps {
                            (-temp).fma(v[2], x)
                        } else {
                            x - temp * v[2]
                        },
                    );
                }
            }
            if let Some(z) = z.as_deref_mut() {
                for jr in 0..n {
                    let temp = tau
                        * if fused_sweeps {
                            v[2].fma(z.at(jr, j + 2), v[1].fma(z.at(jr, j + 1), z.at(jr, j)))
                        } else {
                            z.at(jr, j) + v[1] * z.at(jr, j + 1) + v[2] * z.at(jr, j + 2)
                        };
                    *z.get_mut(jr, j) -= temp;
                    {
                        let x = z.at(jr, j + 1);
                        z.set(
                            jr,
                            j + 1,
                            if fused_sweeps {
                                (-temp).fma(v[1], x)
                            } else {
                                x - temp * v[1]
                            },
                        );
                    }
                    {
                        let x = z.at(jr, j + 2);
                        z.set(
                            jr,
                            j + 2,
                            if fused_sweeps {
                                (-temp).fma(v[2], x)
                            } else {
                                x - temp * v[2]
                            },
                        );
                    }
                }
            }
            t.set(j + 1, j, 0.);
            t.set(j + 2, j, 0.);
        }
        // Last elements: Givens rotations.
        let j = il - 1;
        let (c, s, r) = lartg(h.at(j, j - 1), h.at(j + 1, j - 1));
        h.set(j, j - 1, r);
        h.set(j + 1, j - 1, 0.);
        for jc in j..=ilastm {
            let temp = qz_madd(c, h.at(j, jc), s, h.at(j + 1, jc));
            h.set(j + 1, jc, qz_madd(-s, h.at(j, jc), c, h.at(j + 1, jc)));
            h.set(j, jc, temp);
            let temp2 = qz_madd(c, t.at(j, jc), s, t.at(j + 1, jc));
            t.set(j + 1, jc, qz_madd(-s, t.at(j, jc), c, t.at(j + 1, jc)));
            t.set(j, jc, temp2);
        }
        if let Some(q) = q.as_deref_mut() {
            for jr in 0..n {
                let temp = qz_madd(c, q.at(jr, j), s, q.at(jr, j + 1));
                q.set(jr, j + 1, qz_madd(-s, q.at(jr, j), c, q.at(jr, j + 1)));
                q.set(jr, j, temp);
            }
        }
        let (c, s, r) = lartg(t.at(j + 1, j + 1), t.at(j + 1, j));
        t.set(j + 1, j + 1, r);
        t.set(j + 1, j, 0.);
        for jr in ifrstm..=il {
            let temp = qz_madd(c, h.at(jr, j + 1), s, h.at(jr, j));
            h.set(jr, j, qz_madd(-s, h.at(jr, j + 1), c, h.at(jr, j)));
            h.set(jr, j + 1, temp);
        }
        for jr in ifrstm..il {
            let temp = qz_madd(c, t.at(jr, j + 1), s, t.at(jr, j));
            t.set(jr, j, qz_madd(-s, t.at(jr, j + 1), c, t.at(jr, j)));
            t.set(jr, j + 1, temp);
        }
        if let Some(z) = z.as_deref_mut() {
            for jr in 0..n {
                let temp = qz_madd(c, z.at(jr, j + 1), s, z.at(jr, j));
                z.set(jr, j, qz_madd(-s, z.at(jr, j + 1), c, z.at(jr, j)));
                z.set(jr, j + 1, temp);
            }
        }
    }
    for j in 0..ilo {
        if t.at(j, j) < 0. {
            negate_col(h, t, &mut z, j, 0);
        }
        alphar[j] = h.at(j, j);
        alphai[j] = 0.;
        beta[j] = t.at(j, j);
    }
    Ok(Hgeqz {
        alphar,
        alphai,
        beta,
    })
}

/// dtgevc with howmny 'B': back-transforms into vl / vr (Q and Z on entry).
pub(crate) fn tgevc(
    s: &Mat,
    p: &Mat,
    vl: Option<&mut Mat>,
    vr: Option<&mut Mat>,
    cancel: &dyn Fn() -> bool,
) -> Result<(), usize> {
    let n = s.rows;
    if n == 0 {
        return Ok(());
    }
    const SAFETY: f64 = 100.;
    let safmin = SAFMIN;
    let ulp = ULP;
    let small = safmin * n as f64 / ulp;
    let big = 1. / small;
    let bignum = 1. / (safmin * n as f64);
    let mut anorm = s.at(0, 0).abs();
    if n > 1 {
        anorm += s.at(1, 0).abs();
    }
    let mut bnorm = p.at(0, 0).abs();
    let mut wa = vec![0f64; n];
    let mut wb = vec![0f64; n];
    for j in 1..n {
        if cancel() {
            return Err(n);
        }
        let mut temp = 0.;
        let mut temp2 = 0.;
        let iend = if s.at(j, j - 1) == 0. { j } else { j - 1 };
        for i in 0..iend {
            temp += s.at(i, j).abs();
        }
        for i in 0..j {
            temp2 += p.at(i, j).abs();
        }
        wa[j] = temp;
        wb[j] = temp2;
        for i in iend..(j + 2).min(n) {
            temp += s.at(i, j).abs();
            temp2 += p.at(i, j).abs();
        }
        anorm = anorm.max(temp);
        bnorm = bnorm.max(temp2);
    }
    let ascale = 1. / anorm.max(safmin);
    let bscale = 1. / bnorm.max(safmin);
    let blk = |m: &Mat, j: usize| {
        [
            m.at(j, j),
            m.at(j + 1, j),
            m.at(j, j + 1),
            m.at(j + 1, j + 1),
        ]
    };
    // Coefficients for a real eigenvalue at je: (acoef, bcoefr).
    let real_coef = |je: usize| {
        let temp = 1.
            / (s.at(je, je).abs() * ascale)
                .max(p.at(je, je).abs() * bscale)
                .max(safmin);
        let salfar = (temp * s.at(je, je)) * ascale;
        let sbeta = (temp * p.at(je, je)) * bscale;
        let mut acoef = sbeta * ascale;
        let mut bcoefr = salfar * bscale;
        let mut scale = 1.;
        let lsa = sbeta.abs() >= safmin && acoef.abs() < small;
        let lsb = salfar.abs() >= safmin && bcoefr.abs() < small;
        if lsa {
            scale = (small / sbeta.abs()) * anorm.min(big);
        }
        if lsb {
            scale = scale.max((small / salfar.abs()) * bnorm.min(big));
        }
        if lsa || lsb {
            scale = scale.min(1. / (safmin * 1f64.max(acoef.abs()).max(bcoefr.abs())));
            acoef = if lsa {
                ascale * (scale * sbeta)
            } else {
                scale * acoef
            };
            bcoefr = if lsb {
                bscale * (scale * salfar)
            } else {
                scale * bcoefr
            };
        }
        (acoef, bcoefr)
    };
    let complex_coef = |r: usize| -> Result<(f64, f64, f64), usize> {
        let (acoef, _, bcoefr, _, bcoefi) = lag2(s, p, r, safmin * SAFETY);
        let (mut acoef, mut bcoefr, mut bcoefi) = (acoef, bcoefr, bcoefi);
        if bcoefi == 0. {
            return Err(r + 1);
        }
        let acoefa = acoef.abs();
        let bcoefa = bcoefr.abs() + bcoefi.abs();
        let mut scale = 1.;
        if acoefa * ulp < safmin && acoefa >= safmin {
            scale = (safmin / ulp) / acoefa;
        }
        if bcoefa * ulp < safmin && bcoefa >= safmin {
            scale = scale.max((safmin / ulp) / bcoefa);
        }
        if safmin * acoefa > ascale {
            scale = ascale / (safmin * acoefa);
        }
        if safmin * bcoefa > bscale {
            scale = scale.min(bscale / (safmin * bcoefa));
        }
        if scale != 1. {
            acoef *= scale;
            bcoefr *= scale;
            bcoefi *= scale;
        }
        Ok((acoef, bcoefr, bcoefi))
    };
    if let Some(vl) = vl {
        let mut ilcplx = false;
        let mut je = 0;
        while je < n {
            if cancel() {
                return Err(n);
            }
            if ilcplx {
                ilcplx = false;
                je += 1;
                continue;
            }
            let mut nw = 1;
            if je + 1 < n && s.at(je + 1, je) != 0. {
                ilcplx = true;
                nw = 2;
            }
            if !ilcplx && s.at(je, je).abs() <= safmin && p.at(je, je).abs() <= safmin {
                for jr in 0..n {
                    vl.set(jr, je, 0.);
                }
                vl.set(je, je, 1.);
                je += 1;
                continue;
            }
            let mut w = [vec![0f64; n], vec![0f64; n]];
            let (acoef, bcoefr, bcoefi, acoefa, bcoefa, mut xmax);
            if !ilcplx {
                let (a, b) = real_coef(je);
                acoef = a;
                bcoefr = b;
                bcoefi = 0.;
                acoefa = a.abs();
                bcoefa = b.abs();
                w[0][je] = 1.;
                xmax = 1.;
            } else {
                let (a, br, bi) = complex_coef(je)?;
                let bi = -bi;
                acoef = a;
                bcoefr = br;
                bcoefi = bi;
                acoefa = a.abs();
                bcoefa = br.abs() + bi.abs();
                let temp = acoef * s.at(je + 1, je);
                let temp2r = acoef * s.at(je, je) - bcoefr * p.at(je, je);
                let temp2i = -bcoefi * p.at(je, je);
                if temp.abs() > temp2r.abs() + temp2i.abs() {
                    w[0][je] = 1.;
                    w[1][je] = 0.;
                    w[0][je + 1] = -temp2r / temp;
                    w[1][je + 1] = -temp2i / temp;
                } else {
                    w[0][je + 1] = 1.;
                    w[1][je + 1] = 0.;
                    let temp = acoef * s.at(je, je + 1);
                    w[0][je] =
                        (bcoefr * p.at(je + 1, je + 1) - acoef * s.at(je + 1, je + 1)) / temp;
                    w[1][je] = bcoefi * p.at(je + 1, je + 1) / temp;
                }
                xmax =
                    (w[0][je].abs() + w[1][je].abs()).max(w[0][je + 1].abs() + w[1][je + 1].abs());
            }
            let dmin = (ulp * acoefa * anorm).max(ulp * bcoefa * bnorm).max(safmin);
            let mut il2by2 = false;
            let mut j = je + nw;
            while j < n {
                if il2by2 {
                    il2by2 = false;
                    j += 1;
                    continue;
                }
                let mut na = 1;
                let mut bdiag = [p.at(j, j), 0.];
                if j + 1 < n && s.at(j + 1, j) != 0. {
                    il2by2 = true;
                    bdiag[1] = p.at(j + 1, j + 1);
                    na = 2;
                }
                let xscale = 1. / 1f64.max(xmax);
                let mut temp = wa[j].max(wb[j]).max(acoefa * wa[j] + bcoefa * wb[j]);
                if il2by2 {
                    temp = temp
                        .max(wa[j + 1])
                        .max(wb[j + 1])
                        .max(acoefa * wa[j + 1] + bcoefa * wb[j + 1]);
                }
                if temp > bignum * xscale {
                    for wv in w.iter_mut().take(nw) {
                        for v in wv.iter_mut().take(j).skip(je) {
                            *v *= xscale;
                        }
                    }
                    xmax *= xscale;
                }
                let mut sums = [[0f64; 2]; 2];
                let mut sump = [[0f64; 2]; 2];
                for jw in 0..nw {
                    for ja in 0..na {
                        for jr in je..j {
                            sums[ja][jw] = s.at(jr, j + ja).fma(w[jw][jr], sums[ja][jw]);
                            sump[ja][jw] = p.at(jr, j + ja).fma(w[jw][jr], sump[ja][jw]);
                        }
                    }
                }
                let mut sum = [0f64; 4];
                for ja in 0..na {
                    if ilcplx {
                        sum[ja] =
                            -acoef * sums[ja][0] + bcoefr * sump[ja][0] - bcoefi * sump[ja][1];
                        sum[ja + 2] =
                            -acoef * sums[ja][1] + bcoefr * sump[ja][1] + bcoefi * sump[ja][0];
                    } else {
                        sum[ja] = -acoef * sums[ja][0] + bcoefr * sump[ja][0];
                    }
                }
                let a = if na == 2 {
                    blk(s, j)
                } else {
                    [s.at(j, j), 0., 0., 0.]
                };
                let (x, scale, temp) = laln2(
                    true, na, nw, dmin, acoef, a, bdiag[0], bdiag[1], sum, bcoefr, bcoefi,
                );
                if scale < 1. {
                    for wv in w.iter_mut().take(nw) {
                        for v in wv.iter_mut().take(j).skip(je) {
                            *v *= scale;
                        }
                    }
                    xmax *= scale;
                }
                for ja in 0..na {
                    w[0][j + ja] = x[ja];
                    if nw == 2 {
                        w[1][j + ja] = x[ja + 2];
                    }
                }
                xmax = xmax.max(temp);
                j += 1;
            }
            // Back transform: VL(:, je:n) * w(je:n).
            let mut out = [vec![0f64; n], vec![0f64; n]];
            for jw in 0..nw {
                for (k, &xk) in w[jw].iter().enumerate().skip(je) {
                    if xk != 0. {
                        for i in 0..n {
                            out[jw][i] += xk * vl.at(i, k);
                        }
                    }
                }
            }
            for jw in 0..nw {
                for i in 0..n {
                    vl.set(i, je + jw, out[jw][i]);
                }
            }
            let mut xmax = 0f64;
            for jr in 0..n {
                xmax = if ilcplx {
                    xmax.max(vl.at(jr, je).abs() + vl.at(jr, je + 1).abs())
                } else {
                    xmax.max(vl.at(jr, je).abs())
                };
            }
            if xmax > safmin {
                let xscale = 1. / xmax;
                for jw in 0..nw {
                    for jr in 0..n {
                        *vl.get_mut(jr, je + jw) = xscale * vl.at(jr, je + jw);
                    }
                }
            }
            je += 1;
        }
    }
    if let Some(vr) = vr {
        let mut ilcplx = false;
        let mut je = n as isize - 1;
        while je >= 0 {
            if cancel() {
                return Err(n);
            }
            let jeu = je as usize;
            if ilcplx {
                ilcplx = false;
                je -= 1;
                continue;
            }
            let mut nw = 1;
            if jeu > 0 && s.at(jeu, jeu - 1) != 0. {
                ilcplx = true;
                nw = 2;
            }
            if !ilcplx && s.at(jeu, jeu).abs() <= safmin && p.at(jeu, jeu).abs() <= safmin {
                for jr in 0..n {
                    vr.set(jr, jeu, 0.);
                }
                vr.set(jeu, jeu, 1.);
                je -= 1;
                continue;
            }
            let mut w = [vec![0f64; n], vec![0f64; n]];
            let (acoef, bcoefr, bcoefi, acoefa, bcoefa, mut xmax);
            if !ilcplx {
                let (a, b) = real_coef(jeu);
                acoef = a;
                bcoefr = b;
                bcoefi = 0.;
                acoefa = a.abs();
                bcoefa = b.abs();
                w[0][jeu] = 1.;
                xmax = 1.;
                for jr in 0..jeu {
                    w[0][jr] = bcoefr.fma(p.at(jr, jeu), -acoef * s.at(jr, jeu));
                }
            } else {
                let (a, br, bi) = complex_coef(jeu - 1)?;
                acoef = a;
                bcoefr = br;
                bcoefi = bi;
                acoefa = a.abs();
                bcoefa = br.abs() + bi.abs();
                let temp = acoef * s.at(jeu, jeu - 1);
                let temp2r = acoef * s.at(jeu, jeu) - bcoefr * p.at(jeu, jeu);
                let temp2i = -bcoefi * p.at(jeu, jeu);
                if temp.abs() >= temp2r.abs() + temp2i.abs() {
                    w[0][jeu] = 1.;
                    w[1][jeu] = 0.;
                    w[0][jeu - 1] = -temp2r / temp;
                    w[1][jeu - 1] = -temp2i / temp;
                } else {
                    w[0][jeu - 1] = 1.;
                    w[1][jeu - 1] = 0.;
                    let temp = acoef * s.at(jeu - 1, jeu);
                    w[0][jeu] =
                        (bcoefr * p.at(jeu - 1, jeu - 1) - acoef * s.at(jeu - 1, jeu - 1)) / temp;
                    w[1][jeu] = bcoefi * p.at(jeu - 1, jeu - 1) / temp;
                }
                xmax = (w[0][jeu].abs() + w[1][jeu].abs())
                    .max(w[0][jeu - 1].abs() + w[1][jeu - 1].abs());
                let creala = acoef * w[0][jeu - 1];
                let cimaga = acoef * w[1][jeu - 1];
                let crealb = bcoefr * w[0][jeu - 1] - bcoefi * w[1][jeu - 1];
                let cimagb = bcoefi * w[0][jeu - 1] + bcoefr * w[1][jeu - 1];
                let cre2a = acoef * w[0][jeu];
                let cim2a = acoef * w[1][jeu];
                let cre2b = bcoefr * w[0][jeu] - bcoefi * w[1][jeu];
                let cim2b = bcoefi * w[0][jeu] + bcoefr * w[1][jeu];
                for jr in 0..jeu.saturating_sub(1) {
                    w[0][jr] = -creala * s.at(jr, jeu - 1) + crealb * p.at(jr, jeu - 1)
                        - cre2a * s.at(jr, jeu)
                        + cre2b * p.at(jr, jeu);
                    w[1][jr] = -cimaga * s.at(jr, jeu - 1) + cimagb * p.at(jr, jeu - 1)
                        - cim2a * s.at(jr, jeu)
                        + cim2b * p.at(jr, jeu);
                }
            }
            let dmin = (ulp * acoefa * anorm).max(ulp * bcoefa * bnorm).max(safmin);
            let mut il2by2 = false;
            let mut j = jeu as isize - nw as isize;
            while j >= 0 {
                let mut ju = j as usize;
                if !il2by2 && ju > 0 && s.at(ju, ju - 1) != 0. {
                    il2by2 = true;
                    j -= 1;
                    continue;
                }
                // With il2by2 the block is (ju, ju+1): the iteration after the flag.
                let _ = &mut ju;
                let mut bdiag = [p.at(ju, ju), 0.];
                let na;
                if il2by2 {
                    na = 2;
                    bdiag[1] = p.at(ju + 1, ju + 1);
                } else {
                    na = 1;
                }
                let a = if na == 2 {
                    blk(s, ju)
                } else {
                    [s.at(ju, ju), 0., 0., 0.]
                };
                let b = [
                    w[0][ju],
                    if na == 2 { w[0][ju + 1] } else { 0. },
                    w[1][ju],
                    if na == 2 { w[1][ju + 1] } else { 0. },
                ];
                let (x, scale, temp) = laln2(
                    false, na, nw, dmin, acoef, a, bdiag[0], bdiag[1], b, bcoefr, bcoefi,
                );
                if scale < 1. {
                    for wv in w.iter_mut().take(nw) {
                        for v in wv.iter_mut().take(jeu + 1) {
                            *v *= scale;
                        }
                    }
                }
                xmax = (scale * xmax).max(temp);
                for ja in 0..na {
                    w[0][ju + ja] = x[ja];
                    if nw == 2 {
                        w[1][ju + ja] = x[ja + 2];
                    }
                }
                if ju > 0 {
                    let xscale = 1. / 1f64.max(xmax);
                    let mut temp = acoefa * wa[ju] + bcoefa * wb[ju];
                    if il2by2 {
                        temp = temp.max(acoefa * wa[ju + 1] + bcoefa * wb[ju + 1]);
                    }
                    temp = temp.max(acoefa).max(bcoefa);
                    if temp > bignum * xscale {
                        for wv in w.iter_mut().take(nw) {
                            for v in wv.iter_mut().take(jeu + 1) {
                                *v *= xscale;
                            }
                        }
                        xmax *= xscale;
                    }
                    for ja in 0..na {
                        if ilcplx {
                            let creala = acoef * w[0][ju + ja];
                            let cimaga = acoef * w[1][ju + ja];
                            let crealb = bcoefr * w[0][ju + ja] - bcoefi * w[1][ju + ja];
                            let cimagb = bcoefi * w[0][ju + ja] + bcoefr * w[1][ju + ja];
                            for jr in 0..ju {
                                w[0][jr] = w[0][jr] - creala * s.at(jr, ju + ja)
                                    + crealb * p.at(jr, ju + ja);
                                w[1][jr] = w[1][jr] - cimaga * s.at(jr, ju + ja)
                                    + cimagb * p.at(jr, ju + ja);
                            }
                        } else {
                            let creala = acoef * w[0][ju + ja];
                            let crealb = bcoefr * w[0][ju + ja];
                            for jr in 0..ju {
                                w[0][jr] = crealb.fma(
                                    p.at(jr, ju + ja),
                                    (-creala).fma(s.at(jr, ju + ja), w[0][jr]),
                                );
                            }
                        }
                    }
                }
                il2by2 = false;
                j -= 1;
            }
            // Back transform: VR(:, 0..=je) * w(0..=je).
            let ieig = jeu + 1 - nw;
            let mut out = [vec![0f64; n], vec![0f64; n]];
            for jw in 0..nw {
                for i in 0..n {
                    out[jw][i] = w[jw][0] * vr.at(i, 0);
                }
                for jc in 1..=jeu {
                    for i in 0..n {
                        out[jw][i] += w[jw][jc] * vr.at(i, jc);
                    }
                }
            }
            for jw in 0..nw {
                for i in 0..n {
                    vr.set(i, ieig + jw, out[jw][i]);
                }
            }
            let mut xmax = 0f64;
            for jr in 0..n {
                xmax = if ilcplx {
                    xmax.max(vr.at(jr, ieig).abs() + vr.at(jr, ieig + 1).abs())
                } else {
                    xmax.max(vr.at(jr, ieig).abs())
                };
            }
            if xmax > safmin {
                let xscale = 1. / xmax;
                for jw in 0..nw {
                    for jr in 0..n {
                        *vr.get_mut(jr, ieig + jw) = xscale * vr.at(jr, ieig + jw);
                    }
                }
            }
            je -= 1;
        }
    }
    Ok(())
}

/// Output of [`ggev`]: eigenvalue numerators/denominators and vectors.
pub(crate) struct Ggev {
    pub alphar: Vec<f64>,
    pub alphai: Vec<f64>,
    pub beta: Vec<f64>,
    pub vl: Option<Mat>,
    pub vr: Option<Mat>,
}

/// dggev.
pub(crate) fn ggev(
    a: &Mat,
    b: &Mat,
    want_vl: bool,
    want_vr: bool,
    cancel: &dyn Fn() -> bool,
) -> Result<Ggev, usize> {
    let n = a.rows;
    let mut a = a.clone();
    let mut b = b.clone();
    if n == 0 {
        return Ok(Ggev {
            alphar: vec![],
            alphai: vec![],
            beta: vec![],
            vl: want_vl.then(|| Mat::zeros(0, 0)),
            vr: want_vr.then(|| Mat::zeros(0, 0)),
        });
    }
    let ilv = want_vl || want_vr;
    let (ilo, ihi, lscale, rscale) = ggbal_p(&mut a, &mut b);
    let irows = ihi + 1 - ilo;
    let icols = if ilv { n - ilo } else { irows };
    let tau = geqr2(&mut b, ilo, ilo, irows, icols, cancel);
    orm2r_lt(
        &b,
        ilo,
        ilo,
        irows,
        irows.min(icols),
        &tau,
        &mut a,
        ilo,
        ilo,
        icols,
        cancel,
    );
    let mut vl = want_vl.then(|| {
        let mut q = Mat::identity(n);
        for j in 0..irows.saturating_sub(1) {
            for i in j + 1..irows {
                q.set(ilo + i, ilo + j, b.at(ilo + i, ilo + j));
            }
        }
        org2r(
            &mut q,
            irows.min(icols),
            &tau,
            ilo,
            ilo,
            irows,
            irows,
            cancel,
        );
        q
    });
    let mut vr = want_vr.then(|| Mat::identity(n));
    let r = if ilv {
        gghrd(&mut a, &mut b, ilo, ihi, vl.as_mut(), vr.as_mut(), cancel);
        hgeqz(
            true,
            &mut a,
            &mut b,
            ilo,
            ihi,
            vl.as_mut(),
            vr.as_mut(),
            cancel,
        )?
    } else {
        // Hessenberg-triangular reduction of the active block only.
        let mut sa = Mat::zeros(irows, irows);
        let mut sb = Mat::zeros(irows, irows);
        for j in 0..irows {
            for i in 0..irows {
                sa.set(i, j, a.at(ilo + i, ilo + j));
                sb.set(i, j, b.at(ilo + i, ilo + j));
            }
        }
        gghrd(&mut sa, &mut sb, 0, irows - 1, None, None, cancel);
        for j in 0..irows {
            for i in 0..irows {
                a.set(ilo + i, ilo + j, sa.at(i, j));
                b.set(ilo + i, ilo + j, sb.at(i, j));
            }
        }
        hgeqz(false, &mut a, &mut b, ilo, ihi, None, None, cancel)?
    };
    if ilv {
        tgevc(&a, &b, vl.as_mut(), vr.as_mut(), cancel)?;
    }
    let smlnum = SAFMIN.sqrt() / EPS;
    let normalize = |v: &mut Mat| {
        let mut jc = 0;
        while jc < n {
            if r.alphai[jc] < 0. {
                jc += 1;
                continue;
            }
            let pair = r.alphai[jc] != 0.;
            let mut temp = 0f64;
            for jr in 0..n {
                temp = if pair {
                    temp.max(v.at(jr, jc).abs() + v.at(jr, jc + 1).abs())
                } else {
                    temp.max(v.at(jr, jc).abs())
                };
            }
            if temp >= smlnum {
                let temp = 1. / temp;
                for jr in 0..n {
                    *v.get_mut(jr, jc) *= temp;
                    if pair {
                        *v.get_mut(jr, jc + 1) *= temp;
                    }
                }
            }
            jc += 1;
        }
    };
    if let Some(v) = vl.as_mut() {
        ggbak_p(v, ilo, ihi, &lscale);
        normalize(v);
    }
    if let Some(v) = vr.as_mut() {
        ggbak_p(v, ilo, ihi, &rscale);
        normalize(v);
    }
    Ok(Ggev {
        alphar: r.alphar,
        alphai: r.alphai,
        beta: r.beta,
        vl,
        vr,
    })
}

/// dpotf2 'U' in place; Err(j) if the leading minor of order j is not positive.
pub(crate) fn potf2_upper(a: &mut Mat, cancel: &dyn Fn() -> bool) -> Result<(), usize> {
    let n = a.rows;
    for j in 0..n {
        if cancel() {
            return Err(n);
        }
        let mut dot = 0.;
        for k in 0..j {
            dot += a.at(k, j) * a.at(k, j);
        }
        let ajj = a.at(j, j) - dot;
        if ajj <= 0. || ajj.is_nan() {
            a.set(j, j, ajj);
            return Err(j + 1);
        }
        let ajj = ajj.sqrt();
        a.set(j, j, ajj);
        if j + 1 < n {
            for jj in j + 1..n {
                let mut temp = 0.;
                for k in 0..j {
                    temp += a.at(k, jj) * a.at(k, j);
                }
                let v = a.at(j, jj) + -temp;
                a.set(j, jj, v);
            }
            let r = 1. / ajj;
            for jj in j + 1..n {
                *a.get_mut(j, jj) *= r;
            }
        }
    }
    Ok(())
}

/// dsygs2 itype 1, 'U': A := inv(U^T) A inv(U) (upper triangle).
fn sygs2_upper(a: &mut Mat, b: &Mat, cancel: &dyn Fn() -> bool) {
    let n = a.rows;
    for k in 0..n {
        if cancel() {
            return;
        }
        let bkk = b.at(k, k);
        let akk = a.at(k, k) / (bkk * bkk);
        a.set(k, k, akk);
        if k + 1 < n {
            let m = n - k - 1;
            for j in k + 1..n {
                *a.get_mut(k, j) *= 1. / bkk;
            }
            let ct = -0.5 * akk;
            for j in k + 1..n {
                let v = a.at(k, j) + ct * b.at(k, j);
                a.set(k, j, v);
            }
            // dsyr2 'U' on A(k+1.., k+1..) with x = A(k, k+1..), y = B(k, k+1..).
            let x: Vec<f64> = (k + 1..n).map(|j| a.at(k, j)).collect();
            let y: Vec<f64> = (k + 1..n).map(|j| b.at(k, j)).collect();
            for jj in 0..m {
                if x[jj] != 0. || y[jj] != 0. {
                    let temp1 = -y[jj];
                    let temp2 = -x[jj];
                    for ii in 0..=jj {
                        let v = a.at(k + 1 + ii, k + 1 + jj) + x[ii] * temp1 + y[ii] * temp2;
                        a.set(k + 1 + ii, k + 1 + jj, v);
                    }
                }
            }
            for j in k + 1..n {
                let v = a.at(k, j) + ct * b.at(k, j);
                a.set(k, j, v);
            }
            // dtrsv 'U','T','N' with B(k+1.., k+1..) on the row A(k, k+1..).
            for jj in 0..m {
                let mut temp = a.at(k, k + 1 + jj);
                for ii in 0..jj {
                    temp -= b.at(k + 1 + ii, k + 1 + jj) * a.at(k, k + 1 + ii);
                }
                temp /= b.at(k + 1 + jj, k + 1 + jj);
                a.set(k, k + 1 + jj, temp);
            }
        }
    }
}

/// dsygv itype 1 with 'U'. Err(None) when B is not positive definite.
pub(crate) fn sygv(
    a: &Mat,
    b: &Mat,
    vectors: bool,
    cancel: &dyn Fn() -> bool,
) -> Result<(Vec<f64>, Option<Mat>), Option<()>> {
    let n = a.rows;
    let mut bf = b.clone();
    potf2_upper(&mut bf, cancel).map_err(|_| None)?;
    let mut at = a.clone();
    sygs2_upper(&mut at, &bf, cancel);
    // dsyev reads the upper triangle only.
    for j in 0..n {
        for i in j + 1..n {
            at.set(i, j, at.at(j, i));
        }
    }
    let (d, v) = super::sym::syev(&at, vectors, false, cancel).map_err(|_| Some(()))?;
    let v = v.map(|mut v| {
        // dtrsm 'L','U','N','N': v := inv(U) v.
        for j in 0..n {
            if cancel() {
                return v;
            }
            for k in (0..n).rev() {
                if v.at(k, j) != 0. {
                    let x = v.at(k, j) / bf.at(k, k);
                    v.set(k, j, x);
                    for i in 0..k {
                        let y = v.at(i, j) - x * bf.at(i, k);
                        v.set(i, j, y);
                    }
                }
            }
        }
        v
    });
    Ok((d, v))
}
