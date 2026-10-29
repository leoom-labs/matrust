//! Reordering of Schur forms, ported from reference LAPACK 3.5: dtrsen
//! (dtrexc, dlaexc, dlasy2), ztrsen (ztrexc), dtgsen (dtgexc, dtgex2 for
//! 1-by-1 and 2-by-2 blocks) and ztgsen (ztgexc, ztgex2). 0-based, column-major.
// LAPACK ports keep the reference routines' index loops.
#![allow(clippy::needless_range_loop)]
use super::cdense::{CMat, c, cabs, cdivr, cmulr};
use super::cgen::zlartg;
use super::dense::{Mat, SAFMIN, idamax, lanv2, larfg, lartg};
use crate::Complex64 as C;

const ULPP: f64 = f64::EPSILON; // dlamch('P')

#[inline]
fn rot(x: &mut f64, y: &mut f64, cs: f64, sn: f64) {
    let t = cs * *x + sn * *y;
    *y = cs * *y - sn * *x;
    *x = t;
}
fn rot_rows(a: &mut Mat, r1: usize, r2: usize, cols: std::ops::Range<usize>, cs: f64, sn: f64) {
    for j in cols {
        let (mut x, mut y) = (a.at(r1, j), a.at(r2, j));
        rot(&mut x, &mut y, cs, sn);
        a.set(r1, j, x);
        a.set(r2, j, y);
    }
}
fn rot_cols(a: &mut Mat, c1: usize, c2: usize, rows: std::ops::Range<usize>, cs: f64, sn: f64) {
    for i in rows {
        let (mut x, mut y) = (a.at(i, c1), a.at(i, c2));
        rot(&mut x, &mut y, cs, sn);
        a.set(i, c1, x);
        a.set(i, c2, y);
    }
}

/// dlarfx 'L' with a 3-vector on rows r0..r0+3, columns c0..c0+n.
fn larfx_l3(a: &mut Mat, v: [f64; 3], tau: f64, r0: usize, c0: usize, n: usize) {
    let (t1, t2, t3) = (tau * v[0], tau * v[1], tau * v[2]);
    for j in c0..c0 + n {
        let sum = v[0] * a.at(r0, j) + v[1] * a.at(r0 + 1, j) + v[2] * a.at(r0 + 2, j);
        *a.get_mut(r0, j) -= sum * t1;
        *a.get_mut(r0 + 1, j) -= sum * t2;
        *a.get_mut(r0 + 2, j) -= sum * t3;
    }
}

/// dlarfx 'R' with a 3-vector on columns c0..c0+3, rows r0..r0+m.
fn larfx_r3(a: &mut Mat, v: [f64; 3], tau: f64, r0: usize, m: usize, c0: usize) {
    let (t1, t2, t3) = (tau * v[0], tau * v[1], tau * v[2]);
    for j in r0..r0 + m {
        let sum = v[0] * a.at(j, c0) + v[1] * a.at(j, c0 + 1) + v[2] * a.at(j, c0 + 2);
        *a.get_mut(j, c0) -= sum * t1;
        *a.get_mut(j, c0 + 1) -= sum * t2;
        *a.get_mut(j, c0 + 2) -= sum * t3;
    }
}

/// dlasy2 with ltranl = ltranr = false: TL*X + sgn*X*TR = scale*B.
/// Matrices are column-major 2x2 arrays; returns (x, scale).
fn lasy2(
    sgn: f64,
    n1: usize,
    n2: usize,
    tl: [f64; 4],
    tr: [f64; 4],
    b: [f64; 4],
) -> ([f64; 4], f64) {
    let eps = ULPP;
    let smlnum = SAFMIN / eps;
    let (tl11, tl21, tl12, tl22) = (tl[0], tl[1], tl[2], tl[3]);
    let (tr11, tr21, tr12, tr22) = (tr[0], tr[1], tr[2], tr[3]);
    let mut x = [0f64; 4];
    match (n1, n2) {
        (1, 1) => {
            let mut tau1 = tl11 + sgn * tr11;
            let mut bet = tau1.abs();
            if bet <= smlnum {
                tau1 = smlnum;
                bet = smlnum;
            }
            let mut scale = 1.;
            let gam = b[0].abs();
            if smlnum * gam > bet {
                scale = 1. / gam;
            }
            x[0] = (b[0] * scale) / tau1;
            (x, scale)
        }
        (1, 2) | (2, 1) => {
            const LOCU12: [usize; 4] = [2, 3, 0, 1];
            const LOCL21: [usize; 4] = [1, 0, 3, 2];
            const LOCU22: [usize; 4] = [3, 2, 1, 0];
            const XSWPIV: [bool; 4] = [false, false, true, true];
            const BSWPIV: [bool; 4] = [false, true, false, true];
            let (smin, tmp, mut btmp);
            if n1 == 1 {
                smin = (eps
                    * tl11
                        .abs()
                        .max(tr11.abs())
                        .max(tr12.abs())
                        .max(tr21.abs())
                        .max(tr22.abs()))
                .max(smlnum);
                tmp = [tl11 + sgn * tr11, sgn * tr12, sgn * tr21, tl11 + sgn * tr22];
                btmp = [b[0], b[2]];
            } else {
                smin = (eps
                    * tr11
                        .abs()
                        .max(tl11.abs())
                        .max(tl12.abs())
                        .max(tl21.abs())
                        .max(tl22.abs()))
                .max(smlnum);
                tmp = [tl11 + sgn * tr11, tl21, tl12, tl22 + sgn * tr11];
                btmp = [b[0], b[1]];
            }
            let ipiv = idamax(tmp.iter());
            let mut u11 = tmp[ipiv];
            if u11.abs() <= smin {
                u11 = smin;
            }
            let u12 = tmp[LOCU12[ipiv]];
            let l21 = tmp[LOCL21[ipiv]] / u11;
            let mut u22 = tmp[LOCU22[ipiv]] - u12 * l21;
            if u22.abs() <= smin {
                u22 = smin;
            }
            if BSWPIV[ipiv] {
                let temp = btmp[1];
                btmp[1] = btmp[0] - l21 * temp;
                btmp[0] = temp;
            } else {
                btmp[1] -= l21 * btmp[0];
            }
            let mut scale = 1.;
            if (2. * smlnum) * btmp[1].abs() > u22.abs()
                || (2. * smlnum) * btmp[0].abs() > u11.abs()
            {
                scale = 0.5 / btmp[0].abs().max(btmp[1].abs());
                btmp[0] *= scale;
                btmp[1] *= scale;
            }
            let mut x2 = [0f64; 2];
            x2[1] = btmp[1] / u22;
            x2[0] = btmp[0] / u11 - (u12 / u11) * x2[1];
            if XSWPIV[ipiv] {
                x2.swap(0, 1);
            }
            x[0] = x2[0];
            if n1 == 1 {
                x[2] = x2[1];
            } else {
                x[1] = x2[1];
            }
            (x, scale)
        }
        _ => {
            let mut smin = tr11.abs().max(tr12.abs()).max(tr21.abs()).max(tr22.abs());
            smin = smin
                .max(tl11.abs())
                .max(tl12.abs())
                .max(tl21.abs())
                .max(tl22.abs());
            smin = (eps * smin).max(smlnum);
            let mut t16 = [[0f64; 4]; 4];
            t16[0][0] = tl11 + sgn * tr11;
            t16[1][1] = tl22 + sgn * tr11;
            t16[2][2] = tl11 + sgn * tr22;
            t16[3][3] = tl22 + sgn * tr22;
            t16[0][1] = tl12;
            t16[1][0] = tl21;
            t16[2][3] = tl12;
            t16[3][2] = tl21;
            t16[0][2] = sgn * tr21;
            t16[1][3] = sgn * tr21;
            t16[2][0] = sgn * tr12;
            t16[3][1] = sgn * tr12;
            let mut btmp = [b[0], b[1], b[2], b[3]];
            let mut jpiv = [0usize; 4];
            for i in 0..3 {
                let mut xmax = 0.;
                let (mut ipsv, mut jpsv) = (i, i);
                for ip in i..4 {
                    for jp in i..4 {
                        if t16[ip][jp].abs() >= xmax {
                            xmax = t16[ip][jp].abs();
                            ipsv = ip;
                            jpsv = jp;
                        }
                    }
                }
                if ipsv != i {
                    t16.swap(ipsv, i);
                    btmp.swap(i, ipsv);
                }
                if jpsv != i {
                    for row in t16.iter_mut() {
                        row.swap(jpsv, i);
                    }
                }
                jpiv[i] = jpsv;
                if t16[i][i].abs() < smin {
                    t16[i][i] = smin;
                }
                for j in i + 1..4 {
                    t16[j][i] /= t16[i][i];
                    btmp[j] -= t16[j][i] * btmp[i];
                    for k in i + 1..4 {
                        t16[j][k] -= t16[j][i] * t16[i][k];
                    }
                }
            }
            if t16[3][3].abs() < smin {
                t16[3][3] = smin;
            }
            let mut scale = 1.;
            if (8. * smlnum) * btmp[0].abs() > t16[0][0].abs()
                || (8. * smlnum) * btmp[1].abs() > t16[1][1].abs()
                || (8. * smlnum) * btmp[2].abs() > t16[2][2].abs()
                || (8. * smlnum) * btmp[3].abs() > t16[3][3].abs()
            {
                scale = (1. / 8.)
                    / btmp[0]
                        .abs()
                        .max(btmp[1].abs())
                        .max(btmp[2].abs())
                        .max(btmp[3].abs());
                for v in btmp.iter_mut() {
                    *v *= scale;
                }
            }
            let mut tmp = [0f64; 4];
            for i in 0..4 {
                let k = 3 - i;
                let temp = 1. / t16[k][k];
                tmp[k] = btmp[k] * temp;
                for j in k + 1..4 {
                    tmp[k] -= (temp * t16[k][j]) * tmp[j];
                }
            }
            for i in 0..3 {
                let k = 2 - i;
                if jpiv[k] != k {
                    tmp.swap(k, jpiv[k]);
                }
            }
            (tmp, scale)
        }
    }
}

/// dlaexc: swap adjacent blocks (j1, n1) and (j1+n1, n2). Err when rejected.
fn laexc(t: &mut Mat, q: &mut Mat, j1: usize, n1: usize, n2: usize) -> Result<(), ()> {
    let n = t.rows;
    if n == 0 || n1 == 0 || n2 == 0 || j1 + n1 >= n {
        return Ok(());
    }
    let j2 = j1 + 1;
    let j3 = j1 + 2;
    if n1 == 1 && n2 == 1 {
        let t11 = t.at(j1, j1);
        let t22 = t.at(j2, j2);
        let (cs, sn, _) = lartg(t.at(j1, j2), t22 - t11);
        if j3 < n {
            rot_rows(t, j1, j2, j3..n, cs, sn);
        }
        rot_cols(t, j1, j2, 0..j1, cs, sn);
        t.set(j1, j1, t22);
        t.set(j2, j2, t11);
        rot_cols(q, j1, j2, 0..n, cs, sn);
        return Ok(());
    }
    let nd = n1 + n2;
    let mut d = Mat::zeros(4, 4);
    for jj in 0..nd {
        for ii in 0..nd {
            d.set(ii, jj, t.at(j1 + ii, j1 + jj));
        }
    }
    let mut dnorm = 0f64;
    for jj in 0..nd {
        for ii in 0..nd {
            dnorm = dnorm.max(d.at(ii, jj).abs());
        }
    }
    let eps = ULPP;
    let smlnum = SAFMIN / eps;
    let thresh = (10. * eps * dnorm).max(smlnum);
    let blk = |m: &Mat, r: usize, cc: usize, nr: usize, nc: usize| {
        let mut v = [0f64; 4];
        for q in 0..nc {
            for p in 0..nr {
                v[p + 2 * q] = m.at(r + p, cc + q);
            }
        }
        v
    };
    let tl = blk(&d, 0, 0, n1, n1);
    let tr = blk(&d, n1, n1, n2, n2);
    let bb = blk(&d, 0, n1, n1, n2);
    let (x, scale) = lasy2(-1., n1, n2, tl, tr, bb);
    let xat = |i: usize, j: usize| x[i + 2 * j];
    match n1 + n1 + n2 - 3 {
        1 => {
            // n1 = 1, n2 = 2
            let mut u = [scale, xat(0, 0), xat(0, 1)];
            let (beta, tau) = {
                let mut xx = [u[0], u[1]];
                let r = larfg(u[2], &mut xx);
                u[0] = xx[0];
                u[1] = xx[1];
                r
            };
            let _ = beta;
            u[2] = 1.;
            let t11 = t.at(j1, j1);
            larfx_l3(&mut d, u, tau, 0, 0, 3);
            larfx_r3(&mut d, u, tau, 0, 3, 0);
            if d.at(2, 0)
                .abs()
                .max(d.at(2, 1).abs())
                .max((d.at(2, 2) - t11).abs())
                > thresh
            {
                return Err(());
            }
            larfx_l3(t, u, tau, j1, j1, n - j1);
            larfx_r3(t, u, tau, 0, j2 + 1, j1);
            t.set(j3, j1, 0.);
            t.set(j3, j2, 0.);
            t.set(j3, j3, t11);
            larfx_r3(q, u, tau, 0, n, j1);
        }
        2 => {
            // n1 = 2, n2 = 1
            let mut u = [-xat(0, 0), -xat(1, 0), scale];
            let (_, tau) = {
                let mut xx = [u[1], u[2]];
                let r = larfg(u[0], &mut xx);
                u[1] = xx[0];
                u[2] = xx[1];
                r
            };
            u[0] = 1.;
            let t33 = t.at(j3, j3);
            larfx_l3(&mut d, u, tau, 0, 0, 3);
            larfx_r3(&mut d, u, tau, 0, 3, 0);
            if d.at(1, 0)
                .abs()
                .max(d.at(2, 0).abs())
                .max((d.at(0, 0) - t33).abs())
                > thresh
            {
                return Err(());
            }
            larfx_r3(t, u, tau, 0, j3 + 1, j1);
            larfx_l3(t, u, tau, j1, j2, n - j1 - 1);
            t.set(j1, j1, t33);
            t.set(j2, j1, 0.);
            t.set(j3, j1, 0.);
            larfx_r3(q, u, tau, 0, n, j1);
        }
        _ => {
            // n1 = 2, n2 = 2
            let mut u1 = [-xat(0, 0), -xat(1, 0), scale];
            let (_, tau1) = {
                let mut xx = [u1[1], u1[2]];
                let r = larfg(u1[0], &mut xx);
                u1[1] = xx[0];
                u1[2] = xx[1];
                r
            };
            u1[0] = 1.;
            let temp = -tau1 * (xat(0, 1) + u1[1] * xat(1, 1));
            let mut u2 = [-temp * u1[1] - xat(1, 1), -temp * u1[2], scale];
            let (_, tau2) = {
                let mut xx = [u2[1], u2[2]];
                let r = larfg(u2[0], &mut xx);
                u2[1] = xx[0];
                u2[2] = xx[1];
                r
            };
            u2[0] = 1.;
            larfx_l3(&mut d, u1, tau1, 0, 0, 4);
            larfx_r3(&mut d, u1, tau1, 0, 4, 0);
            larfx_l3(&mut d, u2, tau2, 1, 0, 4);
            larfx_r3(&mut d, u2, tau2, 0, 4, 1);
            if d.at(2, 0)
                .abs()
                .max(d.at(2, 1).abs())
                .max(d.at(3, 0).abs())
                .max(d.at(3, 1).abs())
                > thresh
            {
                return Err(());
            }
            let j4 = j1 + 3;
            larfx_l3(t, u1, tau1, j1, j1, n - j1);
            larfx_r3(t, u1, tau1, 0, j4 + 1, j1);
            larfx_l3(t, u2, tau2, j2, j1, n - j1);
            larfx_r3(t, u2, tau2, 0, j4 + 1, j2);
            t.set(j3, j1, 0.);
            t.set(j3, j2, 0.);
            t.set(j4, j1, 0.);
            t.set(j4, j2, 0.);
            larfx_r3(q, u1, tau1, 0, n, j1);
            larfx_r3(q, u2, tau2, 0, n, j2);
        }
    }
    if n2 == 2 {
        standardize(t, q, j1);
    }
    if n1 == 2 {
        standardize(t, q, j1 + n2);
    }
    Ok(())
}

/// dlanv2 on the block at (j, j) plus the matching rotations.
fn standardize(t: &mut Mat, q: &mut Mat, j: usize) {
    let n = t.rows;
    let r = lanv2(
        t.at(j, j),
        t.at(j, j + 1),
        t.at(j + 1, j),
        t.at(j + 1, j + 1),
    );
    t.set(j, j, r[0]);
    t.set(j, j + 1, r[1]);
    t.set(j + 1, j, r[2]);
    t.set(j + 1, j + 1, r[3]);
    let (cs, sn) = (r[8], r[9]);
    if j + 2 < n {
        rot_rows(t, j, j + 1, j + 2..n, cs, sn);
    }
    rot_cols(t, j, j + 1, 0..j, cs, sn);
    rot_cols(q, j, j + 1, 0..n, cs, sn);
}

fn sub(t: &Mat, i: usize) -> bool {
    t.at(i + 1, i) != 0.
}

/// dtrexc (0-based ifst, ilst).
fn trexc(
    t: &mut Mat,
    q: &mut Mat,
    ifst: usize,
    ilst: usize,
    cancel: &dyn Fn() -> bool,
) -> Result<(), ()> {
    let n = t.rows;
    let mut ifst = ifst;
    let mut ilst = ilst;
    if ifst > 0 && t.at(ifst, ifst - 1) != 0. {
        ifst -= 1;
    }
    let mut nbf = 1;
    if ifst + 1 < n && sub(t, ifst) {
        nbf = 2;
    }
    if ilst > 0 && t.at(ilst, ilst - 1) != 0. {
        ilst -= 1;
    }
    let mut nbl = 1;
    if ilst + 1 < n && sub(t, ilst) {
        nbl = 2;
    }
    if ifst == ilst {
        return Ok(());
    }
    if ifst < ilst {
        if nbf == 2 && nbl == 1 {
            ilst -= 1;
        }
        if nbf == 1 && nbl == 2 {
            ilst += 1;
        }
        let mut here = ifst;
        loop {
            if cancel() {
                return Err(());
            }
            if nbf == 1 || nbf == 2 {
                let mut nbnext = 1;
                if here + nbf + 1 < n && sub(t, here + nbf) {
                    nbnext = 2;
                }
                laexc(t, q, here, nbf, nbnext)?;
                here += nbnext;
                if nbf == 2 && t.at(here + 1, here) == 0. {
                    nbf = 3;
                }
            } else {
                let mut nbnext = 1;
                if here + 3 < n && sub(t, here + 2) {
                    nbnext = 2;
                }
                laexc(t, q, here + 1, 1, nbnext)?;
                if nbnext == 1 {
                    laexc(t, q, here, 1, nbnext)?;
                    here += 1;
                } else {
                    if t.at(here + 2, here + 1) == 0. {
                        nbnext = 1;
                    }
                    if nbnext == 2 {
                        laexc(t, q, here, 1, nbnext)?;
                        here += 2;
                    } else {
                        laexc(t, q, here, 1, 1)?;
                        laexc(t, q, here + 1, 1, 1)?;
                        here += 2;
                    }
                }
            }
            if here >= ilst {
                break;
            }
        }
    } else {
        let mut here = ifst;
        loop {
            if cancel() {
                return Err(());
            }
            if nbf == 1 || nbf == 2 {
                let mut nbnext = 1;
                if here >= 2 && t.at(here - 1, here - 2) != 0. {
                    nbnext = 2;
                }
                laexc(t, q, here - nbnext, nbnext, nbf)?;
                here -= nbnext;
                if nbf == 2 && t.at(here + 1, here) == 0. {
                    nbf = 3;
                }
            } else {
                let mut nbnext = 1;
                if here >= 2 && t.at(here - 1, here - 2) != 0. {
                    nbnext = 2;
                }
                laexc(t, q, here - nbnext, nbnext, 1)?;
                if nbnext == 1 {
                    laexc(t, q, here, nbnext, 1)?;
                    here -= 1;
                } else {
                    if t.at(here, here - 1) == 0. {
                        nbnext = 1;
                    }
                    if nbnext == 2 {
                        laexc(t, q, here - 1, 2, 1)?;
                        here -= 2;
                    } else {
                        laexc(t, q, here, 1, 1)?;
                        laexc(t, q, here - 1, 1, 1)?;
                        here -= 2;
                    }
                }
            }
            if here <= ilst {
                break;
            }
        }
    }
    Ok(())
}

/// dtrsen (job 'N', compq 'V'): selected blocks to the top-left.
pub(crate) fn trsen(
    t: &mut Mat,
    q: &mut Mat,
    select: &[bool],
    cancel: &dyn Fn() -> bool,
) -> Result<(), ()> {
    let n = t.rows;
    let mut ks = 0usize;
    let mut pair = false;
    for k in 0..n {
        if cancel() {
            return Err(());
        }
        if pair {
            pair = false;
            continue;
        }
        let mut swap = select[k];
        if k + 1 < n && sub(t, k) {
            pair = true;
            swap = swap || select[k + 1];
        }
        if swap {
            if k != ks {
                trexc(t, q, k, ks, cancel)?;
            }
            ks += 1;
            if pair {
                ks += 1;
            }
        }
    }
    Ok(())
}

/// ztrexc (0-based).
fn ztrexc(t: &mut CMat, q: &mut CMat, ifst: usize, ilst: usize, cancel: &dyn Fn() -> bool) {
    let n = t.rows;
    if ifst == ilst {
        return;
    }
    let ks: Vec<usize> = if ifst < ilst {
        (ifst..ilst).collect()
    } else {
        (ilst..ifst).rev().collect()
    };
    for k in ks {
        if cancel() {
            return;
        }
        let t11 = t.at(k, k);
        let t22 = t.at(k + 1, k + 1);
        let (cs, sn, _) = zlartg(t.at(k, k + 1), t22 - t11);
        if k + 2 < n {
            for j in k + 2..n {
                let (x, y) = (t.at(k, j), t.at(k + 1, j));
                t.set(k, j, cmulr(x, cs) + sn * y);
                t.set(k + 1, j, cmulr(y, cs) - sn.conj() * x);
            }
        }
        let s2 = sn.conj();
        for i in 0..k {
            let (x, y) = (t.at(i, k), t.at(i, k + 1));
            t.set(i, k, cmulr(x, cs) + s2 * y);
            t.set(i, k + 1, cmulr(y, cs) - s2.conj() * x);
        }
        t.set(k, k, t22);
        t.set(k + 1, k + 1, t11);
        for i in 0..n {
            let (x, y) = (q.at(i, k), q.at(i, k + 1));
            q.set(i, k, cmulr(x, cs) + s2 * y);
            q.set(i, k + 1, cmulr(y, cs) - s2.conj() * x);
        }
    }
}

/// ztrsen (job 'N', compq 'V').
pub(crate) fn ztrsen(t: &mut CMat, q: &mut CMat, select: &[bool], cancel: &dyn Fn() -> bool) {
    let mut ks = 0;
    for k in 0..t.rows {
        if cancel() {
            return;
        }
        if select[k] {
            if k != ks {
                ztrexc(t, q, k, ks, cancel);
            }
            ks += 1;
        }
    }
}

/// Cluster reordering: the highest cluster index ends top-left. Each step
/// selects the clusters >= c, from the largest c down.
pub(crate) fn zordschur_clusters(
    u: &mut CMat,
    t: &mut CMat,
    clusters: &[usize],
    cancel: &dyn Fn() -> bool,
) -> Result<(), String> {
    let mut labels = clusters.to_vec();
    let mut values: Vec<usize> = labels.clone();
    values.sort_unstable();
    values.dedup();
    for &cv in values.iter().rev().take(values.len().saturating_sub(1)) {
        if cancel() {
            return Err("Calculation cancelled".into());
        }
        let select: Vec<bool> = labels.iter().map(|&l| l >= cv).collect();
        ztrsen(t, u, &select, cancel);
        let (sel, rest): (Vec<usize>, Vec<usize>) = labels.iter().partition(|&&l| l >= cv);
        labels = sel.into_iter().chain(rest).collect();
    }
    Ok(())
}

/// Real cluster reordering (blocks keep the label of their first entry).
pub(crate) fn ordschur_clusters(
    u: &mut Mat,
    t: &mut Mat,
    clusters: &[usize],
    cancel: &dyn Fn() -> bool,
) -> Result<(), ()> {
    let n = t.rows;
    let mut labels = clusters.to_vec();
    // A 2x2 block takes the larger label of its two entries.
    let mut k = 0;
    while k < n {
        if cancel() {
            return Err(());
        }
        if k + 1 < n && sub(t, k) {
            let m = labels[k].max(labels[k + 1]);
            labels[k] = m;
            labels[k + 1] = m;
            k += 2;
        } else {
            k += 1;
        }
    }
    let mut values: Vec<usize> = labels.clone();
    values.sort_unstable();
    values.dedup();
    for &cv in values.iter().rev().take(values.len().saturating_sub(1)) {
        if cancel() {
            return Err(());
        }
        let select: Vec<bool> = labels.iter().map(|&l| l >= cv).collect();
        trsen(t, u, &select, cancel)?;
        let (sel, rest): (Vec<usize>, Vec<usize>) = labels.iter().partition(|&&l| l >= cv);
        labels = sel.into_iter().chain(rest).collect();
    }
    Ok(())
}

// ---- generalized ----

fn lassq(values: impl IntoIterator<Item = f64>, scale: &mut f64, sumsq: &mut f64) {
    for v in values {
        if v != 0. {
            let a = v.abs();
            if *scale < a {
                *sumsq = 1. + *sumsq * (*scale / a) * (*scale / a);
                *scale = a;
            } else {
                *sumsq += (a / *scale) * (a / *scale);
            }
        }
    }
}

/// DTGEX2: swap adjacent blocks, rejecting numerically unstable swaps.
fn tgex2(
    a: &mut Mat,
    b: &mut Mat,
    q: &mut Mat,
    z: &mut Mat,
    j1: usize,
    n1: usize,
    n2: usize,
) -> Result<(), ()> {
    let n = a.rows;
    if n <= 1 || n1 == 0 || n2 == 0 || j1 + n1 >= n {
        return Ok(());
    }
    if n1 != 1 || n2 != 1 {
        return tgex2_blocks(a, b, q, z, j1, n1, n2);
    }
    let s = [
        a.at(j1, j1),
        a.at(j1 + 1, j1),
        a.at(j1, j1 + 1),
        a.at(j1 + 1, j1 + 1),
    ];
    let t = [
        b.at(j1, j1),
        b.at(j1 + 1, j1),
        b.at(j1, j1 + 1),
        b.at(j1 + 1, j1 + 1),
    ];
    let eps = ULPP;
    let smlnum = SAFMIN / eps;
    let (mut dscale, mut dsum) = (0., 1.);
    lassq(s, &mut dscale, &mut dsum);
    lassq(t, &mut dscale, &mut dsum);
    let dnorm = dscale * dsum.sqrt();
    let thresh = (20. * eps * dnorm).max(smlnum);
    let mut s = Mat::from_vec(2, 2, s.to_vec());
    let mut t = Mat::from_vec(2, 2, t.to_vec());
    let f = s.at(1, 1) * t.at(0, 0) - t.at(1, 1) * s.at(0, 0);
    let g = s.at(1, 1) * t.at(0, 1) - t.at(1, 1) * s.at(0, 1);
    let sb = t.at(1, 1).abs();
    let sa = s.at(1, 1).abs();
    let (ir12, ir11, _) = lartg(f, g);
    let ir21 = -ir12;
    let ir22 = ir11;
    rot_cols(&mut s, 0, 1, 0..2, ir11, ir21);
    rot_cols(&mut t, 0, 1, 0..2, ir11, ir21);
    let (li11, li21, _) = if sa >= sb {
        lartg(s.at(0, 0), s.at(1, 0))
    } else {
        lartg(t.at(0, 0), t.at(1, 0))
    };
    rot_rows(&mut s, 0, 1, 0..2, li11, li21);
    rot_rows(&mut t, 0, 1, 0..2, li11, li21);
    let li22 = li11;
    let li12 = -li21;
    let ws = s.at(1, 0).abs() + t.at(1, 0).abs();
    if ws > thresh {
        return Err(());
    }
    // Strong stability test.
    let li = [li11, li21, li12, li22];
    let ir = [ir11, ir21, ir12, ir22];
    let check = |orig: &Mat, st: &Mat, dscale: &mut f64, dsum: &mut f64| {
        let mut work = [0f64; 4];
        // work = LI * S
        for jj in 0..2 {
            for ii in 0..2 {
                let mut v = 0.;
                for l in 0..2 {
                    v += li[ii + 2 * l] * st.at(l, jj);
                }
                work[ii + 2 * jj] = v;
            }
        }
        // orig - work * IR^T
        let mut rres = [0f64; 4];
        for jj in 0..2 {
            for ii in 0..2 {
                let mut v = orig.at(j1 + ii, j1 + jj);
                for l in 0..2 {
                    v += -work[ii + 2 * l] * ir[jj + 2 * l];
                }
                rres[ii + 2 * jj] = v;
            }
        }
        lassq(rres, dscale, dsum);
    };
    let (mut dscale, mut dsum) = (0., 1.);
    check(a, &s, &mut dscale, &mut dsum);
    check(b, &t, &mut dscale, &mut dsum);
    if dscale * dsum.sqrt() > thresh {
        return Err(());
    }
    rot_cols(a, j1, j1 + 1, 0..j1 + 2, ir11, ir21);
    rot_cols(b, j1, j1 + 1, 0..j1 + 2, ir11, ir21);
    rot_rows(a, j1, j1 + 1, j1..n, li11, li21);
    rot_rows(b, j1, j1 + 1, j1..n, li11, li21);
    a.set(j1 + 1, j1, 0.);
    b.set(j1 + 1, j1, 0.);
    rot_cols(z, j1, j1 + 1, 0..n, ir11, ir21);
    rot_cols(q, j1, j1 + 1, 0..n, li11, li21);
    Ok(())
}

/// Products here are at most 4x4, local to a block swap.
fn small_mul(a: &Mat, b: &Mat) -> Mat {
    let mut out = Mat::zeros(a.rows, b.cols);
    for j in 0..b.cols {
        for k in 0..a.cols {
            for i in 0..a.rows {
                *out.get_mut(i, j) += a.at(i, k) * b.at(k, j);
            }
        }
    }
    out
}

fn small_qr(a: &Mat) -> (Mat, Mat) {
    let (m, n) = (a.rows, a.cols);
    let mut packed = a.clone();
    let tau = super::rgen::geqr2(&mut packed, 0, 0, m, n, &|| false);
    let mut q = Mat::identity(m);
    let mut r = Mat::zeros(m, n);
    for j in 0..n {
        for i in 0..m {
            if i <= j {
                r.set(i, j, packed.at(i, j));
            } else {
                q.set(i, j, packed.at(i, j));
            }
        }
    }
    super::dense::org2r(&mut q, m.min(n), &tau, 0, 0, m, m, &|| false);
    (q, r)
}

/// DGERQ2's right reflectors, accumulated as Q: A = R*Q.
fn small_rq(a: &Mat) -> (Mat, Mat) {
    let (m, n) = (a.rows, a.cols);
    let mut r = a.clone();
    let mut qt = Mat::identity(n);
    for k in (0..m.min(n)).rev() {
        let row = m - m.min(n) + k;
        let col = n - m.min(n) + k;
        let mut x: Vec<f64> = (0..col).map(|j| r.at(row, j)).collect();
        let (beta, tau) = larfg(r.at(row, col), &mut x);
        let mut v = x;
        v.push(1.);
        super::dense::larf_right(&mut r, &v, tau, 0, 0, row);
        super::dense::larf_right(&mut qt, &v, tau, 0, 0, n);
        for j in 0..col {
            r.set(row, j, 0.);
        }
        r.set(row, col, beta);
    }
    (r, qt.transpose())
}

/// Reference DTGEX2's generalized Sylvester / QR-RQ swap for m=3 or 4.
/// The Sylvester system has at most eight unknowns; full-pivot LU solves it
/// directly, without an inverse. All tentative work stays local until both
/// the weak and strong backward-error tests pass.
#[allow(clippy::too_many_arguments)]
fn tgex2_blocks(
    a: &mut Mat,
    b: &mut Mat,
    q: &mut Mat,
    z: &mut Mat,
    j: usize,
    n1: usize,
    n2: usize,
) -> Result<(), ()> {
    use nalgebra::{DMatrix, DVector};
    let m = n1 + n2;
    let extract = |x: &Mat| {
        Mat::from_vec(
            m,
            m,
            (0..m)
                .flat_map(|c| (0..m).map(move |r| x.at(j + r, j + c)))
                .collect(),
        )
    };
    let (s0, t0) = (extract(a), extract(b));
    // Scale the small pencil before the Sylvester solve and trial products.
    let norm = s0
        .data
        .iter()
        .chain(&t0.data)
        .fold(0f64, |s, x| s.max(x.abs()));
    if norm == 0. {
        return Err(());
    }
    let mut s = s0.clone();
    let mut t = t0.clone();
    for x in s.data.iter_mut().chain(&mut t.data) {
        *x /= norm;
    }
    let original_s = s.clone();
    let original_t = t.clone();
    let size = n1 * n2;
    let mut lhs = DMatrix::<f64>::zeros(2 * size, 2 * size);
    let mut rhs = DVector::<f64>::zeros(2 * size);
    for (part, mat) in [&s, &t].into_iter().enumerate() {
        for col in 0..n2 {
            for row in 0..n1 {
                let eq = part * size + row + n1 * col;
                rhs[eq] = mat.at(row, n1 + col);
                for k in 0..n1 {
                    lhs[(eq, k + n1 * col)] = mat.at(row, k);
                }
                for k in 0..n2 {
                    lhs[(eq, size + row + n1 * k)] = -mat.at(n1 + k, n1 + col);
                }
            }
        }
    }
    let solution = lhs.full_piv_lu().solve(&rhs).ok_or(())?;
    if solution.iter().any(|v| !v.is_finite()) {
        return Err(());
    }
    let scale = 1. / solution.iter().fold(1f64, |s, x| s.max(x.abs()));
    let mut left = Mat::zeros(m, n2);
    let mut right = Mat::zeros(n1, m);
    for c in 0..n2 {
        for r in 0..n1 {
            left.set(r, c, -solution[size + r + n1 * c] * scale);
            right.set(r, n1 + c, solution[r + n1 * c] * scale);
        }
        left.set(n1 + c, c, scale);
    }
    for r in 0..n1 {
        right.set(r, r, scale);
    }
    let (li, _) = small_qr(&left);
    let (_, ir) = small_rq(&right);
    s = small_mul(&small_mul(&li.transpose(), &s), &ir.transpose());
    t = small_mul(&small_mul(&li.transpose(), &t), &ir.transpose());
    let (tr, rq) = small_rq(&t);
    let sr = small_mul(&s, &rq.transpose());
    let irr = small_mul(&rq, &ir);
    let (qr, tq) = small_qr(&t);
    let sq = small_mul(&qr.transpose(), &s);
    let liq = small_mul(&li, &qr);
    let lower = |x: &Mat| {
        super::dense::nrm2((0..n2).flat_map(|c| (n2..m).map(move |r| &x.data[r + c * m])))
    };
    let thresh = (20. * ULPP * super::dense::nrm2(original_s.data.iter().chain(&original_t.data)))
        .max(SAFMIN / ULPP);
    let (mut li, ir) = if lower(&sq) <= lower(&sr) && lower(&sq) <= thresh {
        s = sq;
        t = tq;
        (liq, ir)
    } else if lower(&sr) < thresh {
        s = sr;
        t = tr;
        (li, irr)
    } else {
        return Err(());
    };
    for c in 0..m {
        for r in c + 1..m {
            t.set(r, c, 0.);
        }
    }
    let mut residual = Vec::with_capacity(2 * m * m);
    for (orig, new) in [(&original_s, &s), (&original_t, &t)] {
        let back = small_mul(&small_mul(&li, new), &ir);
        residual.extend(orig.data.iter().zip(&back.data).map(|(x, y)| x - y));
    }
    if super::dense::nrm2(residual.iter()) > thresh {
        return Err(());
    }
    for c in 0..n2 {
        for r in n2..m {
            s.set(r, c, 0.);
        }
    }
    let mut right = ir.transpose();
    // DLAGV2's complex-pair branch: diagonalize the triangular B block.
    for (start, len) in [(0, n2), (n2, n1)] {
        if len != 2 {
            continue;
        }
        let (_, _, _, _, wi) = super::rgen::lag2(&s, &t, start, SAFMIN);
        if wi == 0. {
            // A rounded real pair is handled by the existing 2x2 QZ kernel.
            let mut sb = Mat::from_vec(
                2,
                2,
                vec![
                    s.at(start, start),
                    s.at(start + 1, start),
                    s.at(start, start + 1),
                    s.at(start + 1, start + 1),
                ],
            );
            let mut tb = Mat::from_vec(
                2,
                2,
                vec![
                    t.at(start, start),
                    0.,
                    t.at(start, start + 1),
                    t.at(start + 1, start + 1),
                ],
            );
            let (mut qb, mut zb) = (Mat::identity(2), Mat::identity(2));
            super::rgen::hgeqz(
                true,
                &mut sb,
                &mut tb,
                0,
                1,
                Some(&mut qb),
                Some(&mut zb),
                &|| false,
            )
            .map_err(|_| ())?;
            let mut l = Mat::identity(m);
            let mut r = Mat::identity(m);
            for c in 0..2 {
                for i in 0..2 {
                    l.set(start + i, start + c, qb.at(i, c));
                    r.set(start + i, start + c, zb.at(i, c));
                }
            }
            s = small_mul(&small_mul(&l.transpose(), &s), &r);
            t = small_mul(&small_mul(&l.transpose(), &t), &r);
            li = small_mul(&li, &l);
            right = small_mul(&right, &r);
            s.set(start + 1, start, 0.);
            t.set(start + 1, start, 0.);
        } else {
            let (_, _, snr, csr, snl, csl) = super::rgen::lasv2(
                t.at(start, start),
                t.at(start, start + 1),
                t.at(start + 1, start + 1),
            );
            rot_rows(&mut s, start, start + 1, 0..m, csl, snl);
            rot_rows(&mut t, start, start + 1, 0..m, csl, snl);
            rot_cols(&mut s, start, start + 1, 0..m, csr, snr);
            rot_cols(&mut t, start, start + 1, 0..m, csr, snr);
            rot_cols(&mut li, start, start + 1, 0..m, csl, snl);
            rot_cols(&mut right, start, start + 1, 0..m, csr, snr);
            t.set(start + 1, start, 0.);
            t.set(start, start + 1, 0.);
        }
    }
    let n = a.rows;
    for mat in [&mut *a, &mut *b] {
        // Rows to the right, then columns above the local block.
        for col in j + m..n {
            let old: Vec<f64> = (0..m).map(|r| mat.at(j + r, col)).collect();
            for r in 0..m {
                mat.set(j + r, col, (0..m).map(|k| li.at(k, r) * old[k]).sum());
            }
        }
        for row in 0..j {
            let old: Vec<f64> = (0..m).map(|c| mat.at(row, j + c)).collect();
            for c in 0..m {
                mat.set(row, j + c, (0..m).map(|k| old[k] * right.at(k, c)).sum());
            }
        }
    }
    for c in 0..m {
        for r in 0..m {
            a.set(j + r, j + c, s.at(r, c) * norm);
            b.set(j + r, j + c, t.at(r, c) * norm);
        }
    }
    for (mat, transform) in [(q, &li), (z, &right)] {
        for row in 0..n {
            let old: Vec<f64> = (0..m).map(|c| mat.at(row, j + c)).collect();
            for c in 0..m {
                mat.set(
                    row,
                    j + c,
                    (0..m).map(|k| old[k] * transform.at(k, c)).sum(),
                );
            }
        }
    }
    Ok(())
}

/// DTGEXC (0-based), preserving conjugate pairs as blocks.
fn tgexc(
    a: &mut Mat,
    b: &mut Mat,
    q: &mut Mat,
    z: &mut Mat,
    ifst: usize,
    ilst: usize,
    cancel: &dyn Fn() -> bool,
) -> Result<(), ()> {
    let n = a.rows;
    let mut ifst = ifst;
    let mut ilst = ilst;
    if ifst > 0 && a.at(ifst, ifst - 1) != 0. {
        ifst -= 1;
    }
    let nbf = if ifst + 1 < n && a.at(ifst + 1, ifst) != 0. {
        2
    } else {
        1
    };
    if ilst > 0 && a.at(ilst, ilst - 1) != 0. {
        ilst -= 1;
    }
    let nbl = if ilst + 1 < n && a.at(ilst + 1, ilst) != 0. {
        2
    } else {
        1
    };
    if ifst == ilst {
        return Ok(());
    }
    if ifst < ilst {
        if nbf == 2 && nbl == 1 {
            ilst -= 1;
        }
        if nbf == 1 && nbl == 2 {
            ilst += 1;
        }
        let mut here = ifst;
        while here < ilst {
            if cancel() {
                return Err(());
            }
            let nbnext = if here + nbf + 1 < n && a.at(here + nbf + 1, here + nbf) != 0. {
                2
            } else {
                1
            };
            tgex2(a, b, q, z, here, nbf, nbnext)?;
            here += nbnext;
        }
    } else {
        let mut here = ifst;
        while here > ilst {
            if cancel() {
                return Err(());
            }
            let nbnext = if here >= 2 && a.at(here - 1, here - 2) != 0. {
                2
            } else {
                1
            };
            tgex2(a, b, q, z, here - nbnext, nbnext, nbf)?;
            here -= nbnext;
        }
    }
    Ok(())
}

/// dtgsen (ijob 0) with Q and Z: selected blocks to the top-left, then
/// B's diagonal made nonnegative.
pub(crate) fn tgsen(
    a: &mut Mat,
    b: &mut Mat,
    q: &mut Mat,
    z: &mut Mat,
    select: &[bool],
    cancel: &dyn Fn() -> bool,
) -> Result<(), ()> {
    let n = a.rows;
    let mut ks = 0usize;
    let mut pair = false;
    for k in 0..n {
        if cancel() {
            return Err(());
        }
        if pair {
            pair = false;
            continue;
        }
        let mut swap = select[k];
        if k + 1 < n && a.at(k + 1, k) != 0. {
            pair = true;
            swap = swap || select[k + 1];
        }
        if swap {
            if k != ks {
                tgexc(a, b, q, z, k, ks, cancel)?;
            }
            ks += 1;
            if pair {
                ks += 1;
            }
        }
    }
    let mut pair = false;
    for k in 0..n {
        if cancel() {
            return Err(());
        }
        if pair {
            pair = false;
            continue;
        }
        if k + 1 < n && a.at(k + 1, k) != 0. {
            pair = true;
            continue;
        }
        if b.at(k, k).is_sign_negative() {
            for i in 0..n {
                *a.get_mut(k, i) = -a.at(k, i);
                *b.get_mut(k, i) = -b.at(k, i);
                *q.get_mut(i, k) = -q.at(i, k);
            }
        }
    }
    Ok(())
}

fn zlassq(values: impl IntoIterator<Item = C>, scale: &mut f64, sumsq: &mut f64) {
    lassq(values.into_iter().flat_map(|z| [z.re, z.im]), scale, sumsq);
}

#[inline]
fn zrot(x: &mut C, y: &mut C, cs: f64, s: C) {
    let t = cmulr(*x, cs) + s * *y;
    *y = cmulr(*y, cs) - s.conj() * *x;
    *x = t;
}

/// ztgex2 at j1.
fn ztgex2(a: &mut CMat, b: &mut CMat, q: &mut CMat, z: &mut CMat, j1: usize) -> Result<(), ()> {
    let n = a.rows;
    if n <= 1 {
        return Ok(());
    }
    let mut s = [
        a.at(j1, j1),
        a.at(j1 + 1, j1),
        a.at(j1, j1 + 1),
        a.at(j1 + 1, j1 + 1),
    ];
    let mut t = [
        b.at(j1, j1),
        b.at(j1 + 1, j1),
        b.at(j1, j1 + 1),
        b.at(j1 + 1, j1 + 1),
    ];
    let eps = ULPP;
    let smlnum = SAFMIN / eps;
    let (mut scale, mut sum) = (0., 1.);
    zlassq(s.iter().chain(t.iter()).copied(), &mut scale, &mut sum);
    let sa0 = scale * sum.sqrt();
    let thresh = (20. * eps * sa0).max(smlnum);
    let f = s[3] * t[0] - t[3] * s[0];
    let g = s[3] * t[2] - t[3] * s[2];
    let sa = cabs(s[3]);
    let sb = cabs(t[3]);
    let (cz, sz, _) = zlartg(g, f);
    let sz = -sz;
    // zrot(2, S(1,1),1, S(1,2),1, cz, conj(sz)): columns 1 and 2.
    for m in [&mut s, &mut t] {
        for i in 0..2 {
            let (mut x, mut y) = (m[i], m[i + 2]);
            zrot(&mut x, &mut y, cz, sz.conj());
            m[i] = x;
            m[i + 2] = y;
        }
    }
    let (cq, sq, _) = if sa >= sb {
        zlartg(s[0], s[1])
    } else {
        zlartg(t[0], t[1])
    };
    for m in [&mut s, &mut t] {
        for j in 0..2 {
            let (mut x, mut y) = (m[2 * j], m[2 * j + 1]);
            zrot(&mut x, &mut y, cq, sq);
            m[2 * j] = x;
            m[2 * j + 1] = y;
        }
    }
    let ws = cabs(s[1]) + cabs(t[1]);
    if ws > thresh {
        return Err(());
    }
    let mut work = [s[0], s[1], s[2], s[3], t[0], t[1], t[2], t[3]];
    for base in [0, 4] {
        for i in 0..2 {
            let (mut x, mut y) = (work[base + i], work[base + 2 + i]);
            zrot(&mut x, &mut y, cz, -sz.conj());
            work[base + i] = x;
            work[base + 2 + i] = y;
        }
        for j in 0..2 {
            let (mut x, mut y) = (work[base + 2 * j], work[base + 2 * j + 1]);
            zrot(&mut x, &mut y, cq, -sq);
            work[base + 2 * j] = x;
            work[base + 2 * j + 1] = y;
        }
    }
    for i in 0..2 {
        work[i] -= a.at(j1 + i, j1);
        work[i + 2] -= a.at(j1 + i, j1 + 1);
        work[i + 4] -= b.at(j1 + i, j1);
        work[i + 6] -= b.at(j1 + i, j1 + 1);
    }
    let (mut scale, mut sum) = (0., 1.);
    zlassq(work, &mut scale, &mut sum);
    if scale * sum.sqrt() > thresh {
        return Err(());
    }
    for m in [&mut *a, &mut *b] {
        for i in 0..j1 + 2 {
            let (mut x, mut y) = (m.at(i, j1), m.at(i, j1 + 1));
            zrot(&mut x, &mut y, cz, sz.conj());
            m.set(i, j1, x);
            m.set(i, j1 + 1, y);
        }
        for j in j1..n {
            let (mut x, mut y) = (m.at(j1, j), m.at(j1 + 1, j));
            zrot(&mut x, &mut y, cq, sq);
            m.set(j1, j, x);
            m.set(j1 + 1, j, y);
        }
        m.set(j1 + 1, j1, c(0., 0.));
    }
    for i in 0..n {
        let (mut x, mut y) = (z.at(i, j1), z.at(i, j1 + 1));
        zrot(&mut x, &mut y, cz, sz.conj());
        z.set(i, j1, x);
        z.set(i, j1 + 1, y);
        let (mut x, mut y) = (q.at(i, j1), q.at(i, j1 + 1));
        zrot(&mut x, &mut y, cq, sq.conj());
        q.set(i, j1, x);
        q.set(i, j1 + 1, y);
    }
    Ok(())
}

fn ztgexc(
    a: &mut CMat,
    b: &mut CMat,
    q: &mut CMat,
    z: &mut CMat,
    ifst: usize,
    ilst: usize,
    cancel: &dyn Fn() -> bool,
) -> Result<(), ()> {
    if ifst == ilst {
        return Ok(());
    }
    if ifst < ilst {
        for here in ifst..ilst {
            if cancel() {
                return Err(());
            }
            ztgex2(a, b, q, z, here)?;
        }
    } else {
        for here in (ilst..ifst).rev() {
            if cancel() {
                return Err(());
            }
            ztgex2(a, b, q, z, here)?;
        }
    }
    Ok(())
}

/// ztgsen (ijob 0) with Q and Z.
pub(crate) fn ztgsen(
    a: &mut CMat,
    b: &mut CMat,
    q: &mut CMat,
    z: &mut CMat,
    select: &[bool],
    cancel: &dyn Fn() -> bool,
) -> Result<(), ()> {
    let n = a.rows;
    let mut ks = 0;
    for k in 0..n {
        if cancel() {
            return Err(());
        }
        if select[k] {
            if k != ks {
                ztgexc(a, b, q, z, k, ks, cancel)?;
            }
            ks += 1;
        }
    }
    for k in 0..n {
        if cancel() {
            return Err(());
        }
        let dscale = cabs(b.at(k, k));
        if dscale > SAFMIN {
            let temp1 = cdivr(b.at(k, k), dscale).conj();
            let temp2 = cdivr(b.at(k, k), dscale);
            b.set(k, k, c(dscale, 0.));
            for j in k + 1..n {
                let v = b.at(k, j);
                b.set(k, j, temp1 * v);
            }
            for j in k..n {
                let v = a.at(k, j);
                a.set(k, j, temp1 * v);
            }
            for i in 0..n {
                let v = q.at(i, k);
                q.set(i, k, temp2 * v);
            }
        } else {
            b.set(k, k, c(0., 0.));
        }
    }
    Ok(())
}
