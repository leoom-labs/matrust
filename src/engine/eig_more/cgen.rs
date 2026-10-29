//! Complex generalized eigenproblem, ported from reference LAPACK 3.5:
//! zggev (zggbal 'P', zgeqr2, zunm2r, zgghrd, zhgeqz, ztgevc, zggbak) and
//! zhegv (zpotf2, zhegs2, zheev, ztrsm). 0-based, column-major.
// LAPACK ports keep the reference routines' index loops.
#![allow(clippy::needless_range_loop)]
use super::cdense::{
    CMat, c, cabs, cabs1, cdiv, cdivr, cmulr, zheev, zladiv, zlarf_left, zlarfg, zung2r,
};
use super::dense::{EPS, SAFMIN, ULP, lapy2};
use crate::Complex64 as C;

const ZERO: C = C { re: 0., im: 0. };
const ONE: C = C { re: 1., im: 0. };

/// zlartg (LAPACK 3.5): (cs, sn, r).
pub(crate) fn zlartg(f: C, g: C) -> (f64, C, C) {
    let abs1 = |z: C| z.re.abs().max(z.im.abs());
    let abssq = |z: C| z.re * z.re + z.im * z.im;
    let safmin = SAFMIN;
    let safmn2 = 2f64.powi(((safmin / EPS).ln() / 2f64.ln() / 2.) as i32);
    let safmx2 = 1. / safmn2;
    let mut scale = abs1(f).max(abs1(g));
    let mut fs = f;
    let mut gs = g;
    let mut count: i32 = 0;
    if scale >= safmx2 {
        loop {
            count += 1;
            fs = cmulr(fs, safmn2);
            gs = cmulr(gs, safmn2);
            scale *= safmn2;
            if !(scale >= safmx2 && count < 20) {
                break;
            }
        }
    } else if scale <= safmn2 {
        if g == ZERO || cabs(g).is_nan() {
            return (1., ZERO, f);
        }
        loop {
            count -= 1;
            fs = cmulr(fs, safmx2);
            gs = cmulr(gs, safmx2);
            scale *= safmx2;
            if scale > safmn2 {
                break;
            }
        }
    }
    let f2 = abssq(fs);
    let g2 = abssq(gs);
    if f2 <= g2.max(1.) * safmin {
        if f == ZERO {
            let r = lapy2(g.re, g.im);
            let d = lapy2(gs.re, gs.im);
            return (0., c(gs.re / d, -gs.im / d), c(r, 0.));
        }
        let f2s = lapy2(fs.re, fs.im);
        let g2s = g2.sqrt();
        let cs = f2s / g2s;
        let ff = if abs1(f) > 1. {
            let d = lapy2(f.re, f.im);
            c(f.re / d, f.im / d)
        } else {
            let dr = safmx2 * f.re;
            let di = safmx2 * f.im;
            let d = lapy2(dr, di);
            c(dr / d, di / d)
        };
        let sn = ff * c(gs.re / g2s, -gs.im / g2s);
        let r = cmulr(f, cs) + sn * g;
        return (cs, sn, r);
    }
    let f2s = (1. + g2 / f2).sqrt();
    let mut r = c(f2s * fs.re, f2s * fs.im);
    let cs = 1. / f2s;
    let d = f2 + g2;
    let sn = c(r.re / d, r.im / d) * gs.conj();
    if count > 0 {
        for _ in 0..count {
            r = cmulr(r, safmx2);
        }
    } else {
        for _ in 0..-count {
            r = cmulr(r, safmn2);
        }
    }
    (cs, sn, r)
}

/// zrot on (x, y) with real c and complex s.
#[inline]
fn zrot(x: &mut C, y: &mut C, cs: f64, s: C) {
    let t = cmulr(*x, cs) + s * *y;
    *y = cmulr(*y, cs) - s.conj() * *x;
    *x = t;
}

fn rot_rows(a: &mut CMat, r1: usize, r2: usize, cols: std::ops::Range<usize>, cs: f64, s: C) {
    for j in cols {
        let (mut x, mut y) = (a.at(r1, j), a.at(r2, j));
        zrot(&mut x, &mut y, cs, s);
        a.set(r1, j, x);
        a.set(r2, j, y);
    }
}

fn rot_cols(a: &mut CMat, c1: usize, c2: usize, rows: std::ops::Range<usize>, cs: f64, s: C) {
    for i in rows {
        let (mut x, mut y) = (a.at(i, c1), a.at(i, c2));
        zrot(&mut x, &mut y, cs, s);
        a.set(i, c1, x);
        a.set(i, c2, y);
    }
}

/// zggbal with job 'P'.
pub(crate) fn zggbal_p(a: &mut CMat, b: &mut CMat) -> (usize, usize, Vec<f64>, Vec<f64>) {
    let n = a.rows;
    let mut lscale = vec![1.; n];
    let mut rscale = vec![1.; n];
    if n <= 1 {
        return (0, n.saturating_sub(1), lscale, rscale);
    }
    let nz = |a: &CMat, b: &CMat, i: usize, j: usize| a.at(i, j) != ZERO || b.at(i, j) != ZERO;
    let swap_rows = |a: &mut CMat, b: &mut CMat, i: usize, m: usize, k: usize| {
        a.swap_rows(i, m, k, n);
        b.swap_rows(i, m, k, n);
    };
    let swap_cols = |a: &mut CMat, b: &mut CMat, j: usize, m: usize, l: usize| {
        a.swap_cols(j, m, 0, l + 1);
        b.swap_cols(j, m, 0, l + 1);
    };
    let mut k = 0usize;
    let mut l = n - 1;
    loop {
        let mut found = None;
        for i in (0..=l).rev() {
            let j = match (0..l).find(|&j| nz(a, b, i, j)) {
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
        let Some((i, j)) = found else { break };
        let m = l;
        lscale[m] = i as f64;
        if i != m {
            swap_rows(a, b, i, m, k);
        }
        rscale[m] = j as f64;
        if j != m {
            swap_cols(a, b, j, m, l);
        }
        l -= 1;
        if l == 0 {
            rscale[0] = 1.;
            lscale[0] = 1.;
            return (k, l, lscale, rscale);
        }
    }
    loop {
        let mut found = None;
        for j in k..=l {
            let i = match (k..l).find(|&i| nz(a, b, i, j)) {
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
            swap_rows(a, b, i, m, k);
        }
        rscale[m] = j as f64;
        if j != m {
            swap_cols(a, b, j, m, l);
        }
        k += 1;
    }
    for i in k..=l {
        lscale[i] = 1.;
        rscale[i] = 1.;
    }
    (k, l, lscale, rscale)
}

pub(crate) fn zggbak_p(v: &mut CMat, ilo: usize, ihi: usize, scale: &[f64]) {
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

/// zgeqr2 on the m x n block at (r0, c0).
pub(crate) fn zgeqr2(
    a: &mut CMat,
    r0: usize,
    c0: usize,
    m: usize,
    n: usize,
    cancel: &dyn Fn() -> bool,
) -> Vec<C> {
    let k = m.min(n);
    let mut tau = vec![ZERO; k];
    for i in 0..k {
        if cancel() {
            return tau;
        }
        let mut x: Vec<C> = (i + 1..m).map(|r| a.at(r0 + r, c0 + i)).collect();
        let (beta, t) = zlarfg(a.at(r0 + i, c0 + i), &mut x);
        for (q, &v) in x.iter().enumerate() {
            a.set(r0 + i + 1 + q, c0 + i, v);
        }
        a.set(r0 + i, c0 + i, beta);
        tau[i] = t;
        if i + 1 < n {
            let mut v = vec![ONE];
            v.extend_from_slice(&x);
            zlarf_left(a, &v, t.conj(), r0 + i, c0 + i + 1, n - i - 1);
        }
    }
    tau
}

/// zunm2r 'L','C' (apply Q^H from the left).
#[allow(clippy::too_many_arguments)]
pub(crate) fn zunm2r_lc(
    qr: &CMat,
    r0: usize,
    c0: usize,
    m: usize,
    k: usize,
    tau: &[C],
    cm: &mut CMat,
    cr0: usize,
    cc0: usize,
    nc: usize,
    cancel: &dyn Fn() -> bool,
) {
    for i in 0..k {
        if cancel() {
            return;
        }
        let mut v = vec![ONE];
        v.extend((i + 1..m).map(|r| qr.at(r0 + r, c0 + i)));
        zlarf_left(cm, &v, tau[i].conj(), cr0 + i, cc0, nc);
    }
}

pub(crate) fn zgghrd(
    a: &mut CMat,
    b: &mut CMat,
    ilo: usize,
    ihi: usize,
    mut q: Option<&mut CMat>,
    mut z: Option<&mut CMat>,
    cancel: &dyn Fn() -> bool,
) {
    let n = a.rows;
    for jcol in 0..n.saturating_sub(1) {
        if cancel() {
            return;
        }
        for jrow in jcol + 1..n {
            b.set(jrow, jcol, ZERO);
        }
    }
    for jcol in ilo..ihi.saturating_sub(1) {
        if cancel() {
            return;
        }
        let mut jrow = ihi;
        while jrow >= jcol + 2 {
            let (cs, s, r) = zlartg(a.at(jrow - 1, jcol), a.at(jrow, jcol));
            a.set(jrow - 1, jcol, r);
            a.set(jrow, jcol, ZERO);
            rot_rows(a, jrow - 1, jrow, jcol + 1..n, cs, s);
            rot_rows(b, jrow - 1, jrow, jrow - 1..n, cs, s);
            if let Some(q) = q.as_deref_mut() {
                rot_cols(q, jrow - 1, jrow, 0..n, cs, s.conj());
            }
            let (cs, s, r) = zlartg(b.at(jrow, jrow), b.at(jrow, jrow - 1));
            b.set(jrow, jrow, r);
            b.set(jrow, jrow - 1, ZERO);
            rot_cols(a, jrow, jrow - 1, 0..ihi + 1, cs, s);
            rot_cols(b, jrow, jrow - 1, 0..jrow, cs, s);
            if let Some(z) = z.as_deref_mut() {
                rot_cols(z, jrow, jrow - 1, 0..n, cs, s);
            }
            jrow -= 1;
        }
    }
}

/// zhgeqz. Returns (alpha, beta).
#[allow(clippy::too_many_arguments)]
pub(crate) fn zhgeqz(
    schur: bool,
    h: &mut CMat,
    t: &mut CMat,
    ilo: usize,
    ihi: usize,
    mut q: Option<&mut CMat>,
    mut z: Option<&mut CMat>,
    cancel: &dyn Fn() -> bool,
) -> Result<(Vec<C>, Vec<C>), usize> {
    let n = h.rows;
    let mut alpha = vec![ZERO; n];
    let mut beta = vec![ZERO; n];
    if n == 0 {
        return Ok((alpha, beta));
    }
    let safmin = SAFMIN;
    let ulp = ULP;
    let fro = |m: &CMat| {
        let mut scale = 0f64;
        let mut ssq = 1f64;
        for j in ilo..=ihi {
            for i in ilo..=(j + 1).min(ihi) {
                for v in [m.at(i, j).re, m.at(i, j).im] {
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
        }
        scale * ssq.sqrt()
    };
    let anorm = fro(h);
    let bnorm = fro(t);
    let atol = safmin.max(ulp * anorm);
    let btol = safmin.max(ulp * bnorm);
    let ascale = 1. / safmin.max(anorm);
    let bscale = 1. / safmin.max(bnorm);
    // Standardize column j so T(j,j) is real and nonnegative.
    let standardize =
        |h: &mut CMat, t: &mut CMat, z: &mut Option<&mut CMat>, j: usize, from: usize| {
            let absb = cabs(t.at(j, j));
            if absb > safmin {
                let signbc = cdivr(t.at(j, j), absb).conj();
                t.set(j, j, c(absb, 0.));
                if schur {
                    for i in from..j {
                        let v = t.at(i, j);
                        t.set(i, j, signbc * v);
                    }
                    for i in from..=j {
                        let v = h.at(i, j);
                        h.set(i, j, signbc * v);
                    }
                } else {
                    let v = h.at(j, j);
                    h.set(j, j, v * signbc);
                }
                if let Some(z) = z.as_deref_mut() {
                    for i in 0..n {
                        let v = z.at(i, j);
                        z.set(i, j, signbc * v);
                    }
                }
            } else {
                t.set(j, j, ZERO);
            }
        };
    for j in ihi + 1..n {
        standardize(h, t, &mut z, j, 0);
        alpha[j] = h.at(j, j);
        beta[j] = t.at(j, j);
    }
    let mut ilast = ihi as isize;
    let (mut ifrstm, mut ilastm) = if schur { (0usize, n - 1) } else { (ilo, ihi) };
    let mut iiter = 0usize;
    let mut eshift = ZERO;
    let maxit = 30 * (ihi - ilo + 1);
    let mut finished = ilast < ilo as isize;
    let mut jiter = 0;
    while !finished {
        jiter += 1;
        if jiter > maxit || cancel() {
            return Err(ilast as usize + 1);
        }
        let il = ilast as usize;
        enum Next {
            L50,
            L60,
            L70(usize),
        }
        let next = 'split: {
            if il == ilo {
                break 'split Next::L60;
            } else if cabs1(h.at(il, il - 1)) <= atol {
                h.set(il, il - 1, ZERO);
                break 'split Next::L60;
            }
            if cabs(t.at(il, il)) <= btol {
                t.set(il, il, ZERO);
                break 'split Next::L50;
            }
            let mut j = il as isize - 1;
            while j >= ilo as isize {
                let ju = j as usize;
                let ilazro = if ju == ilo {
                    true
                } else if cabs1(h.at(ju, ju - 1)) <= atol {
                    h.set(ju, ju - 1, ZERO);
                    true
                } else {
                    false
                };
                if cabs(t.at(ju, ju)) < btol {
                    t.set(ju, ju, ZERO);
                    let mut ilazr2 = false;
                    if !ilazro
                        && cabs1(h.at(ju, ju - 1)) * (ascale * cabs1(h.at(ju + 1, ju)))
                            <= cabs1(h.at(ju, ju)) * (ascale * atol)
                    {
                        ilazr2 = true;
                    }
                    if ilazro || ilazr2 {
                        for jch in ju..il {
                            let (cs, s, r) = zlartg(h.at(jch, jch), h.at(jch + 1, jch));
                            h.set(jch, jch, r);
                            h.set(jch + 1, jch, ZERO);
                            rot_rows(h, jch, jch + 1, jch + 1..ilastm + 1, cs, s);
                            rot_rows(t, jch, jch + 1, jch + 1..ilastm + 1, cs, s);
                            if let Some(q) = q.as_deref_mut() {
                                rot_cols(q, jch, jch + 1, 0..n, cs, s.conj());
                            }
                            if ilazr2 {
                                let v = h.at(jch, jch - 1);
                                h.set(jch, jch - 1, cmulr(v, cs));
                            }
                            ilazr2 = false;
                            if cabs1(t.at(jch + 1, jch + 1)) >= btol {
                                if jch + 1 >= il {
                                    break 'split Next::L60;
                                }
                                break 'split Next::L70(jch + 1);
                            }
                            t.set(jch + 1, jch + 1, ZERO);
                        }
                        break 'split Next::L50;
                    }
                    for jch in ju..il {
                        let (cs, s, r) = zlartg(t.at(jch, jch + 1), t.at(jch + 1, jch + 1));
                        t.set(jch, jch + 1, r);
                        t.set(jch + 1, jch + 1, ZERO);
                        if jch + 1 < ilastm {
                            rot_rows(t, jch, jch + 1, jch + 2..ilastm + 1, cs, s);
                        }
                        rot_rows(h, jch, jch + 1, jch - 1..ilastm + 1, cs, s);
                        if let Some(q) = q.as_deref_mut() {
                            rot_cols(q, jch, jch + 1, 0..n, cs, s.conj());
                        }
                        let (cs, s, r) = zlartg(h.at(jch + 1, jch), h.at(jch + 1, jch - 1));
                        h.set(jch + 1, jch, r);
                        h.set(jch + 1, jch - 1, ZERO);
                        rot_cols(h, jch, jch - 1, ifrstm..jch + 1, cs, s);
                        rot_cols(t, jch, jch - 1, ifrstm..jch, cs, s);
                        if let Some(z) = z.as_deref_mut() {
                            rot_cols(z, jch, jch - 1, 0..n, cs, s);
                        }
                    }
                    break 'split Next::L50;
                } else if ilazro {
                    break 'split Next::L70(ju);
                }
                j -= 1;
            }
            return Err(2 * n + 1);
        };
        let ifirst = match next {
            Next::L50 | Next::L60 => {
                if matches!(next, Next::L50) {
                    let (cs, s, r) = zlartg(h.at(il, il), h.at(il, il - 1));
                    h.set(il, il, r);
                    h.set(il, il - 1, ZERO);
                    rot_cols(h, il, il - 1, ifrstm..il, cs, s);
                    rot_cols(t, il, il - 1, ifrstm..il, cs, s);
                    if let Some(z) = z.as_deref_mut() {
                        rot_cols(z, il, il - 1, 0..n, cs, s);
                    }
                }
                standardize(h, t, &mut z, il, ifrstm);
                alpha[il] = h.at(il, il);
                beta[il] = t.at(il, il);
                ilast -= 1;
                if ilast < ilo as isize {
                    finished = true;
                    continue;
                }
                iiter = 0;
                eshift = ZERO;
                if !schur {
                    ilastm = ilast as usize;
                    if ifrstm > ilastm {
                        ifrstm = ilo;
                    }
                }
                continue;
            }
            Next::L70(f) => f,
        };
        iiter += 1;
        if !schur {
            ifrstm = ifirst;
        }
        let shift = if !iiter.is_multiple_of(10) {
            let u12 = cdiv(cmulr(t.at(il - 1, il), bscale), cmulr(t.at(il, il), bscale));
            let ad11 = cdiv(
                cmulr(h.at(il - 1, il - 1), ascale),
                cmulr(t.at(il - 1, il - 1), bscale),
            );
            let ad21 = cdiv(
                cmulr(h.at(il, il - 1), ascale),
                cmulr(t.at(il - 1, il - 1), bscale),
            );
            let ad12 = cdiv(cmulr(h.at(il - 1, il), ascale), cmulr(t.at(il, il), bscale));
            let ad22 = cdiv(cmulr(h.at(il, il), ascale), cmulr(t.at(il, il), bscale));
            let abi22 = ad22 - u12 * ad21;
            let t1 = cmulr(ad11 + abi22, 0.5);
            let rtdisc = super::cdense::csqrt(t1 * t1 + ad12 * ad21 - ad11 * ad22);
            let temp = (t1 - abi22).re * rtdisc.re + (t1 - abi22).im * rtdisc.im;
            if temp <= 0. { t1 + rtdisc } else { t1 - rtdisc }
        } else {
            eshift += cdiv(
                cmulr(h.at(il, il - 1), ascale),
                cmulr(t.at(il - 1, il - 1), bscale),
            );
            eshift
        };
        let mut istart = ifirst;
        let mut ctemp = ZERO;
        let mut found = false;
        let mut j = il - 1;
        while j > ifirst {
            let ct = cmulr(h.at(j, j), ascale) - shift * cmulr(t.at(j, j), bscale);
            let mut temp = cabs1(ct);
            let mut temp2 = ascale * cabs1(h.at(j + 1, j));
            let tempr = temp.max(temp2);
            if tempr < 1. && tempr != 0. {
                temp /= tempr;
                temp2 /= tempr;
            }
            if cabs1(h.at(j, j - 1)) * temp2 <= temp * atol {
                istart = j;
                ctemp = ct;
                found = true;
                break;
            }
            j -= 1;
        }
        if !found {
            istart = ifirst;
            ctemp =
                cmulr(h.at(ifirst, ifirst), ascale) - shift * cmulr(t.at(ifirst, ifirst), bscale);
        }
        let ctemp2 = cmulr(h.at(istart + 1, istart), ascale);
        let (mut cs, mut s, _) = zlartg(ctemp, ctemp2);
        for j in istart..il {
            if j > istart {
                let (cc, ss, r) = zlartg(h.at(j, j - 1), h.at(j + 1, j - 1));
                cs = cc;
                s = ss;
                h.set(j, j - 1, r);
                h.set(j + 1, j - 1, ZERO);
            }
            for jc in j..=ilastm {
                let ct = cmulr(h.at(j, jc), cs) + s * h.at(j + 1, jc);
                h.set(
                    j + 1,
                    jc,
                    -s.conj() * h.at(j, jc) + cmulr(h.at(j + 1, jc), cs),
                );
                h.set(j, jc, ct);
                let ct2 = cmulr(t.at(j, jc), cs) + s * t.at(j + 1, jc);
                t.set(
                    j + 1,
                    jc,
                    -s.conj() * t.at(j, jc) + cmulr(t.at(j + 1, jc), cs),
                );
                t.set(j, jc, ct2);
            }
            if let Some(q) = q.as_deref_mut() {
                for jr in 0..n {
                    let ct = cmulr(q.at(jr, j), cs) + s.conj() * q.at(jr, j + 1);
                    q.set(jr, j + 1, -s * q.at(jr, j) + cmulr(q.at(jr, j + 1), cs));
                    q.set(jr, j, ct);
                }
            }
            let (cc, ss, r) = zlartg(t.at(j + 1, j + 1), t.at(j + 1, j));
            cs = cc;
            s = ss;
            t.set(j + 1, j + 1, r);
            t.set(j + 1, j, ZERO);
            for jr in ifrstm..=(j + 2).min(il) {
                let ct = cmulr(h.at(jr, j + 1), cs) + s * h.at(jr, j);
                h.set(jr, j, -s.conj() * h.at(jr, j + 1) + cmulr(h.at(jr, j), cs));
                h.set(jr, j + 1, ct);
            }
            for jr in ifrstm..=j {
                let ct = cmulr(t.at(jr, j + 1), cs) + s * t.at(jr, j);
                t.set(jr, j, -s.conj() * t.at(jr, j + 1) + cmulr(t.at(jr, j), cs));
                t.set(jr, j + 1, ct);
            }
            if let Some(z) = z.as_deref_mut() {
                for jr in 0..n {
                    let ct = cmulr(z.at(jr, j + 1), cs) + s * z.at(jr, j);
                    z.set(jr, j, -s.conj() * z.at(jr, j + 1) + cmulr(z.at(jr, j), cs));
                    z.set(jr, j + 1, ct);
                }
            }
        }
    }
    for j in 0..ilo {
        standardize(h, t, &mut z, j, 0);
        alpha[j] = h.at(j, j);
        beta[j] = t.at(j, j);
    }
    Ok((alpha, beta))
}

/// ztgevc with howmny 'B'.
pub(crate) fn ztgevc(
    s: &CMat,
    p: &CMat,
    vl: Option<&mut CMat>,
    vr: Option<&mut CMat>,
    cancel: &dyn Fn() -> bool,
) {
    let n = s.rows;
    if n == 0 {
        return;
    }
    let safmin = SAFMIN;
    let ulp = ULP;
    let small = safmin * n as f64 / ulp;
    let big = 1. / small;
    let bignum = 1. / (safmin * n as f64);
    let mut anorm = cabs1(s.at(0, 0));
    let mut bnorm = cabs1(p.at(0, 0));
    let mut ra = vec![0f64; n];
    let mut rb = vec![0f64; n];
    for j in 1..n {
        if cancel() {
            return;
        }
        for i in 0..j {
            ra[j] += cabs1(s.at(i, j));
            rb[j] += cabs1(p.at(i, j));
        }
        anorm = anorm.max(ra[j] + cabs1(s.at(j, j)));
        bnorm = bnorm.max(rb[j] + cabs1(p.at(j, j)));
    }
    let ascale = 1. / anorm.max(safmin);
    let bscale = 1. / bnorm.max(safmin);
    let coef = |je: usize| {
        let temp = 1.
            / (cabs1(s.at(je, je)) * ascale)
                .max(p.at(je, je).re.abs() * bscale)
                .max(safmin);
        let salpha = cmulr(cmulr(s.at(je, je), temp), ascale);
        let sbeta = (temp * p.at(je, je).re) * bscale;
        let mut acoeff = sbeta * ascale;
        let mut bcoeff = cmulr(salpha, bscale);
        let lsa = sbeta.abs() >= safmin && acoeff.abs() < small;
        let lsb = cabs1(salpha) >= safmin && cabs1(bcoeff) < small;
        let mut scale = 1.;
        if lsa {
            scale = (small / sbeta.abs()) * anorm.min(big);
        }
        if lsb {
            scale = scale.max((small / cabs1(salpha)) * bnorm.min(big));
        }
        if lsa || lsb {
            scale = scale.min(1. / (safmin * 1f64.max(acoeff.abs()).max(cabs1(bcoeff))));
            acoeff = if lsa {
                ascale * (scale * sbeta)
            } else {
                scale * acoeff
            };
            bcoeff = if lsb {
                cmulr(cmulr(salpha, scale), bscale)
            } else {
                cmulr(bcoeff, scale)
            };
        }
        (acoeff, bcoeff)
    };
    if let Some(vl) = vl {
        for je in 0..n {
            if cancel() {
                return;
            }
            if cabs1(s.at(je, je)) <= safmin && p.at(je, je).re.abs() <= safmin {
                for jr in 0..n {
                    vl.set(jr, je, ZERO);
                }
                vl.set(je, je, ONE);
                continue;
            }
            let (acoeff, bcoeff) = coef(je);
            let acoefa = acoeff.abs();
            let bcoefa = cabs1(bcoeff);
            let mut xmax = 1.;
            let mut work = vec![ZERO; n];
            work[je] = ONE;
            let dmin = (ulp * acoefa * anorm).max(ulp * bcoefa * bnorm).max(safmin);
            for j in je + 1..n {
                let temp = 1. / xmax;
                if acoefa * ra[j] + bcoefa * rb[j] > bignum * temp {
                    for w in work.iter_mut().take(j).skip(je) {
                        *w = cmulr(*w, temp);
                    }
                    xmax = 1.;
                }
                let mut suma = ZERO;
                let mut sumb = ZERO;
                for jr in je..j {
                    suma += s.at(jr, j).conj() * work[jr];
                    sumb += p.at(jr, j).conj() * work[jr];
                }
                let mut sum = cmulr(suma, acoeff) - bcoeff.conj() * sumb;
                let mut d = (cmulr(s.at(j, j), acoeff) - bcoeff * p.at(j, j)).conj();
                if cabs1(d) <= dmin {
                    d = c(dmin, 0.);
                }
                if cabs1(d) < 1. && cabs1(sum) >= bignum * cabs1(d) {
                    let temp = 1. / cabs1(sum);
                    for w in work.iter_mut().take(j).skip(je) {
                        *w = cmulr(*w, temp);
                    }
                    xmax *= temp;
                    sum = cmulr(sum, temp);
                }
                work[j] = zladiv(-sum, d);
                xmax = f64::max(xmax, cabs1(work[j]));
            }
            let mut out = vec![ZERO; n];
            for (k, &xk) in work.iter().enumerate().skip(je) {
                if xk != ZERO {
                    let temp = ONE * xk;
                    for (i, o) in out.iter_mut().enumerate() {
                        *o += temp * vl.at(i, k);
                    }
                }
            }
            let xmax = out.iter().fold(0f64, |m, &v| m.max(cabs1(v)));
            if xmax > safmin {
                let temp = 1. / xmax;
                for (i, &o) in out.iter().enumerate() {
                    vl.set(i, je, cmulr(o, temp));
                }
            } else {
                for i in 0..n {
                    vl.set(i, je, ZERO);
                }
            }
        }
    }
    if let Some(vr) = vr {
        for je in (0..n).rev() {
            if cancel() {
                return;
            }
            if cabs1(s.at(je, je)) <= safmin && p.at(je, je).re.abs() <= safmin {
                for jr in 0..n {
                    vr.set(jr, je, ZERO);
                }
                vr.set(je, je, ONE);
                continue;
            }
            let (acoeff, bcoeff) = coef(je);
            let acoefa = acoeff.abs();
            let bcoefa = cabs1(bcoeff);
            let mut work = vec![ZERO; n];
            let dmin = (ulp * acoefa * anorm).max(ulp * bcoefa * bnorm).max(safmin);
            for jr in 0..je {
                work[jr] = cmulr(s.at(jr, je), acoeff) - bcoeff * p.at(jr, je);
            }
            work[je] = ONE;
            for j in (0..je).rev() {
                let mut d = cmulr(s.at(j, j), acoeff) - bcoeff * p.at(j, j);
                if cabs1(d) <= dmin {
                    d = c(dmin, 0.);
                }
                if cabs1(d) < 1. && cabs1(work[j]) >= bignum * cabs1(d) {
                    let temp = 1. / cabs1(work[j]);
                    for w in work.iter_mut().take(je + 1) {
                        *w = cmulr(*w, temp);
                    }
                }
                work[j] = zladiv(-work[j], d);
                if j > 0 {
                    if cabs1(work[j]) > 1. {
                        let temp = 1. / cabs1(work[j]);
                        if acoefa * ra[j] + bcoefa * rb[j] >= bignum * temp {
                            for w in work.iter_mut().take(je + 1) {
                                *w = cmulr(*w, temp);
                            }
                        }
                    }
                    let ca = cmulr(work[j], acoeff);
                    let cb = bcoeff * work[j];
                    for jr in 0..j {
                        work[jr] = work[jr] + ca * s.at(jr, j) - cb * p.at(jr, j);
                    }
                }
            }
            let mut out = vec![ZERO; n];
            for (k, &xk) in work.iter().enumerate().take(je + 1) {
                if xk != ZERO {
                    let temp = ONE * xk;
                    for (i, o) in out.iter_mut().enumerate() {
                        *o += temp * vr.at(i, k);
                    }
                }
            }
            let xmax = out.iter().fold(0f64, |m, &v| m.max(cabs1(v)));
            if xmax > safmin {
                let temp = 1. / xmax;
                for (i, &o) in out.iter().enumerate() {
                    vr.set(i, je, cmulr(o, temp));
                }
            } else {
                for i in 0..n {
                    vr.set(i, je, ZERO);
                }
            }
        }
    }
}

pub(crate) struct Zggev {
    pub alpha: Vec<C>,
    pub beta: Vec<C>,
    pub vl: Option<CMat>,
    pub vr: Option<CMat>,
}

/// zggev.
pub(crate) fn zggev(
    a: &CMat,
    b: &CMat,
    want_vl: bool,
    want_vr: bool,
    cancel: &dyn Fn() -> bool,
) -> Result<Zggev, usize> {
    let n = a.rows;
    if n == 0 {
        return Ok(Zggev {
            alpha: vec![],
            beta: vec![],
            vl: want_vl.then(|| CMat::zeros(0, 0)),
            vr: want_vr.then(|| CMat::zeros(0, 0)),
        });
    }
    let mut a = a.clone();
    let mut b = b.clone();
    let ilv = want_vl || want_vr;
    let (ilo, ihi, lscale, rscale) = zggbal_p(&mut a, &mut b);
    let irows = ihi + 1 - ilo;
    let icols = if ilv { n - ilo } else { irows };
    let tau = zgeqr2(&mut b, ilo, ilo, irows, icols, cancel);
    zunm2r_lc(
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
        let mut q = CMat::identity(n);
        for j in 0..irows.saturating_sub(1) {
            for i in j + 1..irows {
                q.set(ilo + i, ilo + j, b.at(ilo + i, ilo + j));
            }
        }
        zung2r(
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
    let mut vr = want_vr.then(|| CMat::identity(n));
    let (alpha, beta);
    if ilv {
        zgghrd(&mut a, &mut b, ilo, ihi, vl.as_mut(), vr.as_mut(), cancel);
        let r = zhgeqz(
            true,
            &mut a,
            &mut b,
            ilo,
            ihi,
            vl.as_mut(),
            vr.as_mut(),
            cancel,
        )?;
        alpha = r.0;
        beta = r.1;
        ztgevc(&a, &b, vl.as_mut(), vr.as_mut(), cancel);
    } else {
        let mut sa = CMat::zeros(irows, irows);
        let mut sb = CMat::zeros(irows, irows);
        for j in 0..irows {
            for i in 0..irows {
                sa.set(i, j, a.at(ilo + i, ilo + j));
                sb.set(i, j, b.at(ilo + i, ilo + j));
            }
        }
        zgghrd(&mut sa, &mut sb, 0, irows - 1, None, None, cancel);
        for j in 0..irows {
            for i in 0..irows {
                a.set(ilo + i, ilo + j, sa.at(i, j));
                b.set(ilo + i, ilo + j, sb.at(i, j));
            }
        }
        let r = zhgeqz(false, &mut a, &mut b, ilo, ihi, None, None, cancel)?;
        alpha = r.0;
        beta = r.1;
    }
    let smlnum = SAFMIN.sqrt() / EPS;
    let normalize = |v: &mut CMat| {
        for jc in 0..n {
            let temp = (0..n).fold(0f64, |m, jr| m.max(cabs1(v.at(jr, jc))));
            if temp >= smlnum {
                let temp = 1. / temp;
                for jr in 0..n {
                    let x = v.at(jr, jc);
                    v.set(jr, jc, cmulr(x, temp));
                }
            }
        }
    };
    if let Some(v) = vl.as_mut() {
        zggbak_p(v, ilo, ihi, &lscale);
        normalize(v);
    }
    if let Some(v) = vr.as_mut() {
        zggbak_p(v, ilo, ihi, &rscale);
        normalize(v);
    }
    Ok(Zggev {
        alpha,
        beta,
        vl,
        vr,
    })
}

/// zpotf2 'U' in place.
pub(crate) fn zpotf2_upper(a: &mut CMat, cancel: &dyn Fn() -> bool) -> Result<(), usize> {
    let n = a.rows;
    for j in 0..n {
        if cancel() {
            return Err(n);
        }
        let mut dot = ZERO;
        for k in 0..j {
            dot += a.at(k, j).conj() * a.at(k, j);
        }
        let ajj = (c(a.at(j, j).re, 0.) - dot).re;
        if ajj <= 0. || ajj.is_nan() {
            a.set(j, j, c(ajj, 0.));
            return Err(j + 1);
        }
        let ajj = ajj.sqrt();
        a.set(j, j, c(ajj, 0.));
        if j + 1 < n {
            // zlacgv, zgemv 'T' with alpha = -1, beta = 1, zlacgv back.
            let x: Vec<C> = (0..j).map(|k| a.at(k, j).conj()).collect();
            for jj in j + 1..n {
                let mut temp = ZERO;
                for (k, &xk) in x.iter().enumerate() {
                    temp += a.at(k, jj) * xk;
                }
                let v = a.at(j, jj) + c(-1., 0.) * temp;
                a.set(j, jj, v);
            }
            let r = 1. / ajj;
            for jj in j + 1..n {
                let v = a.at(j, jj);
                a.set(j, jj, c(r, 0.) * v);
            }
        }
    }
    Ok(())
}

/// zhegs2 itype 1, 'U'.
fn zhegs2_upper(a: &mut CMat, b: &CMat, cancel: &dyn Fn() -> bool) {
    let n = a.rows;
    for k in 0..n {
        if cancel() {
            return;
        }
        let bkk = b.at(k, k).re;
        let akk = a.at(k, k).re / (bkk * bkk);
        a.set(k, k, c(akk, 0.));
        if k + 1 < n {
            let m = n - k - 1;
            for j in k + 1..n {
                let v = a.at(k, j);
                a.set(k, j, c(1. / bkk, 0.) * v);
            }
            let ct = c(-0.5 * akk, 0.);
            // Work on conjugated rows (zlacgv).
            let mut x: Vec<C> = (k + 1..n).map(|j| a.at(k, j).conj()).collect();
            let y: Vec<C> = (k + 1..n).map(|j| b.at(k, j).conj()).collect();
            for (xi, &yi) in x.iter_mut().zip(&y) {
                *xi += ct * yi;
            }
            // zher2 'U' with alpha = -1 on A(k+1.., k+1..).
            let alpha = c(-1., 0.);
            for jj in 0..m {
                if x[jj] != ZERO || y[jj] != ZERO {
                    let temp1 = alpha * y[jj].conj();
                    let temp2 = (alpha * x[jj]).conj();
                    for ii in 0..jj {
                        let v = a.at(k + 1 + ii, k + 1 + jj) + x[ii] * temp1 + y[ii] * temp2;
                        a.set(k + 1 + ii, k + 1 + jj, v);
                    }
                    let d = a.at(k + 1 + jj, k + 1 + jj).re + (x[jj] * temp1 + y[jj] * temp2).re;
                    a.set(k + 1 + jj, k + 1 + jj, c(d, 0.));
                } else {
                    let d = a.at(k + 1 + jj, k + 1 + jj).re;
                    a.set(k + 1 + jj, k + 1 + jj, c(d, 0.));
                }
            }
            for (xi, &yi) in x.iter_mut().zip(&y) {
                *xi += ct * yi;
            }
            // ztrsv 'U','C','N' with B(k+1.., k+1..) on x.
            for jj in 0..m {
                let mut temp = x[jj];
                for ii in 0..jj {
                    temp -= b.at(k + 1 + ii, k + 1 + jj).conj() * x[ii];
                }
                temp = cdiv(temp, b.at(k + 1 + jj, k + 1 + jj).conj());
                x[jj] = temp;
            }
            for (q, &v) in x.iter().enumerate() {
                a.set(k, k + 1 + q, v.conj());
            }
        }
    }
}

/// zhegv itype 1 with 'U'. Err(None) when B is not positive definite.
pub(crate) fn zhegv(
    a: &CMat,
    b: &CMat,
    vectors: bool,
    cancel: &dyn Fn() -> bool,
) -> Result<(Vec<f64>, Option<CMat>), Option<()>> {
    let n = a.rows;
    let mut bf = b.clone();
    zpotf2_upper(&mut bf, cancel).map_err(|_| None)?;
    let mut at = a.clone();
    zhegs2_upper(&mut at, &bf, cancel);
    for j in 0..n {
        for i in j + 1..n {
            at.set(i, j, at.at(j, i).conj());
        }
    }
    let (d, v) = zheev(&at, vectors, false, cancel).map_err(|_| Some(()))?;
    let v = v.map(|mut v| {
        for j in 0..n {
            if cancel() {
                return v;
            }
            for k in (0..n).rev() {
                if v.at(k, j) != ZERO {
                    let x = cdiv(v.at(k, j), bf.at(k, k));
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
