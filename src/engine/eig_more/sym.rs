//! Real symmetric eigenproblem, ported from reference LAPACK 3.5 dsyev:
//! dsytd2, dorgtr (dorg2l/dorg2r), dsteqr and dsterf. 0-based, column-major.
// LAPACK ports keep the reference routines' index loops.
#![allow(clippy::needless_range_loop)]
use super::dense::{EPS, Mat, SAFMIN, lapy2, larf_left, larfg, lartg, org2r, sign};
use crate::engine::Fma;

/// Reference dsymv with beta = 0 on the `lower` or upper triangle of
/// A(o..o+n, o..o+n); x and the result have length n.
fn symv(a: &Mat, o: usize, n: usize, lower: bool, alpha: f64, x: &[f64]) -> Vec<f64> {
    let mut y = vec![0.; n];
    if lower {
        for j in 0..n {
            let temp1 = alpha * x[j];
            let mut temp2 = 0.;
            y[j] += temp1 * a.at(o + j, o + j);
            for i in j + 1..n {
                y[i] += temp1 * a.at(o + i, o + j);
                temp2 += a.at(o + i, o + j) * x[i];
            }
            y[j] += alpha * temp2;
        }
    } else {
        for j in 0..n {
            let temp1 = alpha * x[j];
            let mut temp2 = 0.;
            for i in 0..j {
                y[i] += temp1 * a.at(o + i, o + j);
                temp2 += a.at(o + i, o + j) * x[i];
            }
            y[j] = y[j] + temp1 * a.at(o + j, o + j) + alpha * temp2;
        }
    }
    y
}

/// Reference dsyr2 with alpha on A(o..o+n, o..o+n).
fn syr2(a: &mut Mat, o: usize, n: usize, lower: bool, alpha: f64, x: &[f64], y: &[f64]) {
    for j in 0..n {
        if x[j] != 0. || y[j] != 0. {
            let temp1 = alpha * y[j];
            let temp2 = alpha * x[j];
            let range = if lower { j..n } else { 0..j + 1 };
            for i in range {
                *a.get_mut(o + i, o + j) += x[i] * temp1 + y[i] * temp2;
            }
        }
    }
}

fn dot(x: &[f64], y: &[f64]) -> f64 {
    let mut s = 0.;
    for (a, b) in x.iter().zip(y) {
        s += a * b;
    }
    s
}

/// dsytd2: returns (d, e, tau); `a` holds the reflectors afterwards.
pub(crate) fn sytd2(
    a: &mut Mat,
    lower: bool,
    cancel: &dyn Fn() -> bool,
) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let n = a.rows;
    let mut d = vec![0.; n];
    let mut e = vec![0.; n.saturating_sub(1)];
    let mut tau = vec![0.; n.saturating_sub(1)];
    if n == 0 {
        return (d, e, tau);
    }
    if lower {
        for i in 0..n - 1 {
            if cancel() {
                return (d, e, tau);
            }
            let mut x: Vec<f64> = (i + 2..n).map(|r| a.at(r, i)).collect();
            let (beta, taui) = larfg(a.at(i + 1, i), &mut x);
            for (k, &v) in x.iter().enumerate() {
                a.set(i + 2 + k, i, v);
            }
            e[i] = beta;
            if taui != 0. {
                a.set(i + 1, i, 1.);
                let v: Vec<f64> = (i + 1..n).map(|r| a.at(r, i)).collect();
                let mut w = symv(a, i + 1, n - i - 1, true, taui, &v);
                let alpha = -0.5 * taui * dot(&w, &v);
                for (wk, vk) in w.iter_mut().zip(&v) {
                    *wk += alpha * vk;
                }
                syr2(a, i + 1, n - i - 1, true, -1., &v, &w);
                a.set(i + 1, i, e[i]);
            } else {
                a.set(i + 1, i, beta);
            }
            d[i] = a.at(i, i);
            tau[i] = taui;
        }
        d[n - 1] = a.at(n - 1, n - 1);
    } else {
        for i in (0..n - 1).rev() {
            if cancel() {
                return (d, e, tau);
            }
            // Reflector annihilating A(0..i, i+1); alpha is A(i, i+1).
            let mut x: Vec<f64> = (0..i).map(|r| a.at(r, i + 1)).collect();
            let (beta, taui) = larfg(a.at(i, i + 1), &mut x);
            for (k, &v) in x.iter().enumerate() {
                a.set(k, i + 1, v);
            }
            e[i] = beta;
            if taui != 0. {
                a.set(i, i + 1, 1.);
                let v: Vec<f64> = (0..=i).map(|r| a.at(r, i + 1)).collect();
                let mut w = symv(a, 0, i + 1, false, taui, &v);
                let alpha = -0.5 * taui * dot(&w, &v);
                for (wk, vk) in w.iter_mut().zip(&v) {
                    *wk += alpha * vk;
                }
                syr2(a, 0, i + 1, false, -1., &v, &w);
                a.set(i, i + 1, e[i]);
            } else {
                a.set(i, i + 1, beta);
            }
            d[i + 1] = a.at(i + 1, i + 1);
            tau[i] = taui;
        }
        d[0] = a.at(0, 0);
    }
    (d, e, tau)
}

/// dorg2l on the leading m x n block of q with k reflectors (columns n-k..n).
fn org2l(q: &mut Mat, m: usize, n: usize, k: usize, tau: &[f64], cancel: &dyn Fn() -> bool) {
    for j in 0..n - k {
        if cancel() {
            return;
        }
        for l in 0..m {
            q.set(l, j, 0.);
        }
        q.set(m - n + j, j, 1.);
    }
    for i in 0..k {
        if cancel() {
            return;
        }
        let ii = n - k + i;
        // Apply H(i) to A(0..m-n+ii+1, 0..ii) from the left.
        let rows = m - n + ii + 1;
        q.set(rows - 1, ii, 1.);
        let v: Vec<f64> = (0..rows).map(|r| q.at(r, ii)).collect();
        larf_left(q, &v, tau[i], 0, 0, ii);
        for r in 0..rows - 1 {
            *q.get_mut(r, ii) *= -tau[i];
        }
        q.set(rows - 1, ii, 1. - tau[i]);
        for l in rows..m {
            q.set(l, ii, 0.);
        }
    }
}

/// dorgtr: the orthogonal Q of sytd2.
pub(crate) fn orgtr(a: &Mat, lower: bool, tau: &[f64], cancel: &dyn Fn() -> bool) -> Mat {
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
            q.set(0, j, 0.);
            for i in j + 1..n {
                q.set(i, j, q.at(i, j - 1));
            }
        }
        q.set(0, 0, 1.);
        for i in 1..n {
            if cancel() {
                return q;
            }
            q.set(i, 0, 0.);
        }
        if n > 1 {
            org2r(&mut q, n - 1, tau, 1, 1, n - 1, n - 1, cancel);
        }
    } else {
        for j in 0..n - 1 {
            if cancel() {
                return q;
            }
            for i in 0..j {
                q.set(i, j, q.at(i, j + 1));
            }
            q.set(n - 1, j, 0.);
        }
        for i in 0..n - 1 {
            if cancel() {
                return q;
            }
            q.set(i, n - 1, 0.);
        }
        q.set(n - 1, n - 1, 1.);
        if n > 1 {
            org2l(&mut q, n - 1, n - 1, n - 1, tau, cancel);
        }
    }
    q
}

/// dlaev2 (or dlae2 when the vector is not used): (rt1, rt2, cs1, sn1).
pub(crate) fn laev2(a: f64, b: f64, c: f64) -> (f64, f64, f64, f64) {
    let sm = a + c;
    let df = a - c;
    let adf = df.abs();
    let tb = b + b;
    let ab = tb.abs();
    let (acmx, acmn) = if a.abs() > c.abs() { (a, c) } else { (c, a) };
    let rt = if adf > ab {
        adf * (1. + (ab / adf) * (ab / adf)).sqrt()
    } else if adf < ab {
        ab * (1. + (adf / ab) * (adf / ab)).sqrt()
    } else {
        ab * 2f64.sqrt()
    };
    let (rt1, rt2, sgn1);
    if sm < 0. {
        rt1 = 0.5 * (sm - rt);
        sgn1 = -1;
        rt2 = (acmx / rt1) * acmn - (b / rt1) * b;
    } else if sm > 0. {
        rt1 = 0.5 * (sm + rt);
        sgn1 = 1;
        rt2 = (acmx / rt1) * acmn - (b / rt1) * b;
    } else {
        rt1 = 0.5 * rt;
        rt2 = -0.5 * rt;
        sgn1 = 1;
    }
    let (cs, sgn2) = if df >= 0. {
        (df + rt, 1)
    } else {
        (df - rt, -1)
    };
    let acs = cs.abs();
    let (mut cs1, mut sn1);
    if acs > ab {
        let ct = -tb / cs;
        sn1 = 1. / (1. + ct * ct).sqrt();
        cs1 = ct * sn1;
    } else if ab == 0. {
        cs1 = 1.;
        sn1 = 0.;
    } else {
        let tn = -cs / tb;
        cs1 = 1. / (1. + tn * tn).sqrt();
        sn1 = tn * cs1;
    }
    if sgn1 == sgn2 {
        let tn = cs1;
        cs1 = -sn1;
        sn1 = tn;
    }
    (rt1, rt2, cs1, sn1)
}

/// dlasr('R', 'V', direct) on columns c0..c0+mm of z.
fn lasr_right(z: &mut Mat, c0: usize, mm: usize, c: &[f64], s: &[f64], forward: bool) {
    let rows = z.rows;
    let mut body = |j: usize| {
        let (ct, st) = (c[j], s[j]);
        if ct != 1. || st != 0. {
            for i in 0..rows {
                // MKL's dlasr fuses each rotation's multiply-add.
                let temp = z.at(i, c0 + j + 1);
                let zj = z.at(i, c0 + j);
                z.set(i, c0 + j + 1, ct.fma(temp, -(st * zj)));
                z.set(i, c0 + j, st.fma(temp, ct * zj));
            }
        }
    };
    if forward {
        for j in 0..mm - 1 {
            body(j);
        }
    } else {
        for j in (0..mm - 1).rev() {
            body(j);
        }
    }
}

/// dsteqr with compz = 'V' (z holds Q on entry) or 'N' (z = None).
/// Returns Err(()) when it does not converge.
pub(crate) fn steqr(
    d: &mut [f64],
    e: &mut [f64],
    mut z: Option<&mut Mat>,
    cancel: &dyn Fn() -> bool,
) -> Result<(), ()> {
    let n = d.len();
    if n <= 1 {
        return Ok(());
    }
    let eps = EPS;
    let eps2 = eps * eps;
    let safmin = SAFMIN;
    let ssfmax = (1. / safmin).sqrt() / 3.;
    let ssfmin = safmin.sqrt() / eps2;
    let nmaxit = n * 30;
    let mut jtot = 0;
    let mut work_c = vec![0f64; n];
    let mut work_s = vec![0f64; n];
    // 1-based indices below mirror the Fortran; d1/e1 access helpers.
    let mut l1 = 1usize;
    let nm1 = n - 1;
    loop {
        if cancel() {
            return Err(());
        }
        if l1 > n {
            break;
        }
        if l1 > 1 {
            e[l1 - 2] = 0.;
        }
        let mut m = n;
        if l1 <= nm1 {
            for mm in l1..=nm1 {
                let tst = e[mm - 1].abs();
                if tst == 0. {
                    m = mm;
                    break;
                }
                if tst <= (d[mm - 1].abs().sqrt() * d[mm].abs().sqrt()) * eps {
                    e[mm - 1] = 0.;
                    m = mm;
                    break;
                }
            }
        }
        let mut l = l1;
        let lsv = l;
        let mut lend = m;
        let lendsv = lend;
        l1 = m + 1;
        if lend == l {
            continue;
        }
        // Scale submatrix in rows and columns l to lend.
        let mut anorm = 0f64;
        for i in l..=lend {
            anorm = anorm.max(d[i - 1].abs());
        }
        for i in l..lend {
            anorm = anorm.max(e[i - 1].abs());
        }
        let mut iscale = 0;
        if anorm == 0. {
            continue;
        }
        if anorm > ssfmax {
            iscale = 1;
            for i in l..=lend {
                d[i - 1] = d[i - 1] / anorm * ssfmax;
            }
            for i in l..lend {
                e[i - 1] = e[i - 1] / anorm * ssfmax;
            }
        } else if anorm < ssfmin {
            iscale = 2;
            for i in l..=lend {
                d[i - 1] = d[i - 1] / anorm * ssfmin;
            }
            for i in l..lend {
                e[i - 1] = e[i - 1] / anorm * ssfmin;
            }
        }
        if d[lend - 1].abs() < d[l - 1].abs() {
            lend = lsv;
            l = lendsv;
        }
        if lend > l {
            // QL iteration.
            loop {
                let mut m = lend;
                if l != lend {
                    for mm in l..lend {
                        let tst = e[mm - 1].abs() * e[mm - 1].abs();
                        if tst <= (eps2 * d[mm - 1].abs()) * d[mm].abs() + safmin {
                            m = mm;
                            break;
                        }
                    }
                }
                if m < lend {
                    e[m - 1] = 0.;
                }
                let mut p = d[l - 1];
                if m == l {
                    d[l - 1] = p;
                    l += 1;
                    if l <= lend {
                        continue;
                    }
                    break;
                }
                if m == l + 1 {
                    let (rt1, rt2, c, s) = laev2(d[l - 1], e[l - 1], d[l]);
                    if let Some(z) = z.as_deref_mut() {
                        lasr_right(z, l - 1, 2, &[c], &[s], false);
                    }
                    d[l - 1] = rt1;
                    d[l] = rt2;
                    e[l - 1] = 0.;
                    l += 2;
                    if l <= lend {
                        continue;
                    }
                    break;
                }
                if jtot == nmaxit {
                    break;
                }
                jtot += 1;
                let mut g = (d[l] - p) / (2. * e[l - 1]);
                let mut r = lapy2(g, 1.);
                g = d[m - 1] - p + (e[l - 1] / (g + sign(r, g)));
                let mut s = 1.;
                let mut c = 1.;
                p = 0.;
                for i in (l..m).rev() {
                    let f = s * e[i - 1];
                    let b = c * e[i - 1];
                    let (cc, ss, rr) = lartg(g, f);
                    c = cc;
                    s = ss;
                    r = rr;
                    if i != m - 1 {
                        e[i] = r;
                    }
                    g = d[i] - p;
                    r = (d[i - 1] - g) * s + 2. * c * b;
                    p = s * r;
                    d[i] = g + p;
                    g = c * r - b;
                    work_c[i - 1] = c;
                    work_s[i - 1] = -s;
                }
                if let Some(z) = z.as_deref_mut() {
                    let mm = m - l + 1;
                    lasr_right(z, l - 1, mm, &work_c[l - 1..], &work_s[l - 1..], false);
                }
                d[l - 1] -= p;
                e[l - 1] = g;
            }
        } else {
            // QR iteration.
            loop {
                let mut m = lend;
                if l != lend {
                    let mut mm = l;
                    while mm > lend {
                        let tst = e[mm - 2].abs() * e[mm - 2].abs();
                        if tst <= (eps2 * d[mm - 1].abs()) * d[mm - 2].abs() + safmin {
                            m = mm;
                            break;
                        }
                        mm -= 1;
                    }
                }
                if m > lend {
                    e[m - 2] = 0.;
                }
                let mut p = d[l - 1];
                if m == l {
                    d[l - 1] = p;
                    if l == 1 || l - 1 < lend {
                        break;
                    }
                    l -= 1;
                    continue;
                }
                if m + 1 == l {
                    let (rt1, rt2, c, s) = laev2(d[l - 2], e[l - 2], d[l - 1]);
                    if let Some(z) = z.as_deref_mut() {
                        lasr_right(z, l - 2, 2, &[c], &[s], true);
                    }
                    d[l - 2] = rt1;
                    d[l - 1] = rt2;
                    e[l - 2] = 0.;
                    if l < 2 + lend || l < 3 {
                        break;
                    }
                    l -= 2;
                    continue;
                }
                if jtot == nmaxit {
                    break;
                }
                jtot += 1;
                let mut g = (d[l - 2] - p) / (2. * e[l - 2]);
                let mut r = lapy2(g, 1.);
                g = d[m - 1] - p + (e[l - 2] / (g + sign(r, g)));
                let mut s = 1.;
                let mut c = 1.;
                p = 0.;
                for i in m..l {
                    let f = s * e[i - 1];
                    let b = c * e[i - 1];
                    let (cc, ss, rr) = lartg(g, f);
                    c = cc;
                    s = ss;
                    r = rr;
                    if i != m {
                        e[i - 2] = r;
                    }
                    g = d[i - 1] - p;
                    r = (d[i] - g) * s + 2. * c * b;
                    p = s * r;
                    d[i - 1] = g + p;
                    g = c * r - b;
                    work_c[i - 1] = c;
                    work_s[i - 1] = s;
                }
                if let Some(z) = z.as_deref_mut() {
                    let mm = l - m + 1;
                    lasr_right(z, m - 1, mm, &work_c[m - 1..], &work_s[m - 1..], true);
                }
                d[l - 1] -= p;
                e[l - 2] = g;
            }
        }
        if iscale == 1 {
            for i in lsv..=lendsv {
                d[i - 1] = d[i - 1] / ssfmax * anorm;
            }
            for i in lsv..lendsv {
                e[i - 1] = e[i - 1] / ssfmax * anorm;
            }
        } else if iscale == 2 {
            for i in lsv..=lendsv {
                d[i - 1] = d[i - 1] / ssfmin * anorm;
            }
            for i in lsv..lendsv {
                e[i - 1] = e[i - 1] / ssfmin * anorm;
            }
        }
        if jtot >= nmaxit {
            if e.iter().any(|&v| v != 0.) {
                return Err(());
            }
            break;
        }
    }
    // Selection sort, swapping vectors.
    for ii in 1..n {
        let i = ii - 1;
        let mut k = i;
        let mut p = d[i];
        for (j, &dj) in d.iter().enumerate().skip(ii) {
            if dj < p {
                k = j;
                p = dj;
            }
        }
        if k != i {
            d[k] = d[i];
            d[i] = p;
            if let Some(z) = z.as_deref_mut() {
                z.swap_cols(i, k, 0, z.rows);
            }
        }
    }
    Ok(())
}

/// dsterf: eigenvalues of the symmetric tridiagonal (d, e), ascending.
pub(crate) fn sterf(d: &mut [f64], e: &mut [f64], cancel: &dyn Fn() -> bool) -> Result<(), ()> {
    let n = d.len();
    if n <= 1 {
        return Ok(());
    }
    let eps = EPS;
    let eps2 = eps * eps;
    let safmin = SAFMIN;
    let ssfmax = (1. / safmin).sqrt() / 3.;
    let ssfmin = safmin.sqrt() / eps2;
    let nmaxit = n * 30;
    let mut jtot = 0;
    let mut l1 = 1usize;
    loop {
        if cancel() {
            return Err(());
        }
        if l1 > n {
            break;
        }
        if l1 > 1 {
            e[l1 - 2] = 0.;
        }
        let mut m = n;
        for mm in l1..n {
            if e[mm - 1].abs() <= (d[mm - 1].abs().sqrt() * d[mm].abs().sqrt()) * eps {
                e[mm - 1] = 0.;
                m = mm;
                break;
            }
        }
        let mut l = l1;
        let lsv = l;
        let mut lend = m;
        let lendsv = lend;
        l1 = m + 1;
        if lend == l {
            continue;
        }
        let mut anorm = 0f64;
        for i in l..=lend {
            anorm = anorm.max(d[i - 1].abs());
        }
        for i in l..lend {
            anorm = anorm.max(e[i - 1].abs());
        }
        let mut iscale = 0;
        if anorm == 0. {
            continue;
        }
        if anorm > ssfmax {
            iscale = 1;
            for i in l..=lend {
                d[i - 1] = d[i - 1] / anorm * ssfmax;
            }
            for i in l..lend {
                e[i - 1] = e[i - 1] / anorm * ssfmax;
            }
        } else if anorm < ssfmin {
            iscale = 2;
            for i in l..=lend {
                d[i - 1] = d[i - 1] / anorm * ssfmin;
            }
            for i in l..lend {
                e[i - 1] = e[i - 1] / anorm * ssfmin;
            }
        }
        for i in l..lend {
            e[i - 1] *= e[i - 1];
        }
        if d[lend - 1].abs() < d[l - 1].abs() {
            lend = lsv;
            l = lendsv;
        }
        if lend >= l {
            loop {
                let mut m = lend;
                if l != lend {
                    for mm in l..lend {
                        if e[mm - 1].abs() <= eps2 * (d[mm - 1] * d[mm]).abs() {
                            m = mm;
                            break;
                        }
                    }
                }
                if m < lend {
                    e[m - 1] = 0.;
                }
                let mut p = d[l - 1];
                if m == l {
                    d[l - 1] = p;
                    l += 1;
                    if l <= lend {
                        continue;
                    }
                    break;
                }
                if m == l + 1 {
                    let rte = e[l - 1].sqrt();
                    let (rt1, rt2, _, _) = laev2(d[l - 1], rte, d[l]);
                    d[l - 1] = rt1;
                    d[l] = rt2;
                    e[l - 1] = 0.;
                    l += 2;
                    if l <= lend {
                        continue;
                    }
                    break;
                }
                if jtot == nmaxit {
                    break;
                }
                jtot += 1;
                let rte = e[l - 1].sqrt();
                let mut sigma = (d[l] - p) / (2. * rte);
                let r = lapy2(sigma, 1.);
                sigma = p - (rte / (sigma + sign(r, sigma)));
                let mut c = 1.;
                let mut s = 0.;
                let mut gamma = d[m - 1] - sigma;
                p = gamma * gamma;
                for i in (l..m).rev() {
                    let bb = e[i - 1];
                    let r = p + bb;
                    if i != m - 1 {
                        e[i] = s * r;
                    }
                    let oldc = c;
                    c = p / r;
                    s = bb / r;
                    let oldgam = gamma;
                    let alpha = d[i - 1];
                    gamma = c * (alpha - sigma) - s * oldgam;
                    d[i] = oldgam + (alpha - gamma);
                    p = if c != 0. {
                        (gamma * gamma) / c
                    } else {
                        oldc * bb
                    };
                }
                e[l - 1] = s * p;
                d[l - 1] = sigma + gamma;
            }
        } else {
            loop {
                let mut m = lend;
                let mut mm = l;
                while mm > lend {
                    if e[mm - 2].abs() <= eps2 * (d[mm - 1] * d[mm - 2]).abs() {
                        m = mm;
                        break;
                    }
                    mm -= 1;
                }
                if m > lend {
                    e[m - 2] = 0.;
                }
                let mut p = d[l - 1];
                if m == l {
                    d[l - 1] = p;
                    if l == 1 || l - 1 < lend {
                        break;
                    }
                    l -= 1;
                    continue;
                }
                if m + 1 == l {
                    let rte = e[l - 2].sqrt();
                    let (rt1, rt2, _, _) = laev2(d[l - 1], rte, d[l - 2]);
                    d[l - 1] = rt1;
                    d[l - 2] = rt2;
                    e[l - 2] = 0.;
                    if l < 2 + lend || l < 3 {
                        break;
                    }
                    l -= 2;
                    continue;
                }
                if jtot == nmaxit {
                    break;
                }
                jtot += 1;
                let rte = e[l - 2].sqrt();
                let mut sigma = (d[l - 2] - p) / (2. * rte);
                let r = lapy2(sigma, 1.);
                sigma = p - (rte / (sigma + sign(r, sigma)));
                let mut c = 1.;
                let mut s = 0.;
                let mut gamma = d[m - 1] - sigma;
                p = gamma * gamma;
                for i in m..l {
                    let bb = e[i - 1];
                    let r = p + bb;
                    if i != m {
                        e[i - 2] = s * r;
                    }
                    let oldc = c;
                    c = p / r;
                    s = bb / r;
                    let oldgam = gamma;
                    let alpha = d[i];
                    gamma = c * (alpha - sigma) - s * oldgam;
                    d[i - 1] = oldgam + (alpha - gamma);
                    p = if c != 0. {
                        (gamma * gamma) / c
                    } else {
                        oldc * bb
                    };
                }
                e[l - 2] = s * p;
                d[l - 1] = sigma + gamma;
            }
        }
        if iscale == 1 {
            for i in lsv..=lendsv {
                d[i - 1] = d[i - 1] / ssfmax * anorm;
            }
        } else if iscale == 2 {
            for i in lsv..=lendsv {
                d[i - 1] = d[i - 1] / ssfmin * anorm;
            }
        }
        if jtot >= nmaxit {
            if e.iter().any(|&v| v != 0.) {
                return Err(());
            }
            break;
        }
    }
    d.sort_by(f64::total_cmp);
    Ok(())
}

/// dsyev ('V' when `vectors`): ascending eigenvalues and, optionally, Q.
pub(crate) fn syev(
    a: &Mat,
    vectors: bool,
    lower: bool,
    cancel: &dyn Fn() -> bool,
) -> Result<(Vec<f64>, Option<Mat>), ()> {
    let n = a.rows;
    if n == 0 {
        return Ok((vec![], vectors.then(|| Mat::zeros(0, 0))));
    }
    if n == 1 {
        return Ok((vec![a.at(0, 0)], vectors.then(|| Mat::identity(1))));
    }
    let mut h = a.clone();
    // DSYEV: smlnum = safmin / eps, rmin = sqrt(smlnum),
    // rmax = sqrt(1/smlnum), before Householder reduction.
    let anrm = h.data.iter().fold(0f64, |m, x| m.max(x.abs()));
    let rmin = (SAFMIN / super::dense::ULP).sqrt();
    let scaled = if anrm == 0. {
        0.
    } else {
        anrm.clamp(rmin, 1. / rmin)
    };
    if scaled != anrm {
        super::dense::lascl(&mut h.data, anrm, scaled);
    }
    let (mut d, mut e, tau) = sytd2(&mut h, lower, cancel);
    if !vectors {
        sterf(&mut d, &mut e, cancel)?;
        if scaled != anrm {
            super::dense::lascl(&mut d, scaled, anrm);
        }
        return Ok((d, None));
    }
    let mut q = orgtr(&h, lower, &tau, cancel);
    steqr(&mut d, &mut e, Some(&mut q), cancel)?;
    if scaled != anrm {
        super::dense::lascl(&mut d, scaled, anrm);
    }
    Ok((d, Some(q)))
}
