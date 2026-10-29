//! Schur forms for funm and the reordering functions: dgees / zgees
//! (permutation-only balancing, Hessenberg QR) and MATLAB's rsf2csf.
// LAPACK ports keep the reference routines' index loops.
#![allow(clippy::needless_range_loop)]
use super::cdense::{self, CMat, c, cabs};
use super::dense::{self, Mat};
use crate::Complex64 as C;

/// dgees with vectors: (U, T) with A = U*T*U'.
pub(crate) fn gees(a: &Mat, cancel: &dyn Fn() -> bool) -> Result<(Mat, Mat), usize> {
    let n = a.rows;
    if n == 0 {
        return Ok((Mat::zeros(0, 0), Mat::zeros(0, 0)));
    }
    let mut h = a.clone();
    let (ilo, ihi, scale) = dense::gebal(&mut h, true, false);
    let tau = dense::gehd2(&mut h, ilo, ihi, cancel)?;
    let mut q = dense::orghr(&h, ilo, ihi, &tau, cancel);
    dense::hseqr(true, &mut h, ilo, ihi, Some(&mut q), cancel)?;
    dense::gebak(&mut q, ilo, ihi, &scale, false, true, true);
    Ok((q, h))
}

/// zgees with vectors.
pub(crate) fn zgees(a: &CMat, cancel: &dyn Fn() -> bool) -> Result<(CMat, CMat), usize> {
    let n = a.rows;
    if n == 0 {
        return Ok((CMat::zeros(0, 0), CMat::zeros(0, 0)));
    }
    let mut h = a.clone();
    let (ilo, ihi, scale) = cdense::zgebal(&mut h, true, false);
    let tau = cdense::zgehd2(&mut h, ilo, ihi, cancel)?;
    let mut q = cdense::zunghr(&h, ilo, ihi, &tau, cancel);
    cdense::zhseqr(true, &mut h, ilo, ihi, Some(&mut q), cancel)?;
    cdense::zgebak(&mut q, ilo, ihi, &scale, false, true, true);
    Ok((q, h))
}

/// MATLAB's rsf2csf: real Schur form to complex Schur form.
pub(crate) fn rsf2csf(u: &Mat, t: &Mat, cancel: &dyn Fn() -> bool) -> (CMat, CMat) {
    let n = t.rows;
    let mut uc = CMat::from_real(u);
    let mut tc = CMat::from_real(t);
    for m in (1..n).rev() {
        if tc.at(m, m - 1) != c(0., 0.) {
            // mu = eig(T(k,k)) - T(m,m), k = m-1:m.
            let block = Mat::from_vec(
                2,
                2,
                vec![
                    tc.at(m - 1, m - 1).re,
                    tc.at(m, m - 1).re,
                    tc.at(m - 1, m).re,
                    tc.at(m, m).re,
                ],
            );
            let Ok(e) = dense::geevx(&block, true, false, false, cancel) else {
                continue;
            };
            let mu1 = c(e.wr[0], e.wi[0]) - tc.at(m, m);
            let sub = tc.at(m, m - 1).re;
            let r = cabs(mu1).hypot(sub.abs());
            let cc = cdense::cdivr(mu1, r);
            let s = sub / r;
            // G = [c' s; -s c]
            let g = [cc.conj(), c(s, 0.), c(-s, 0.), cc];
            for j in m - 1..n {
                let (x, y) = (tc.at(m - 1, j), tc.at(m, j));
                tc.set(m - 1, j, g[0] * x + g[1] * y);
                tc.set(m, j, g[2] * x + g[3] * y);
            }
            // X(:,k) := X(:,k) * G' with G' = [c -s; s c'].
            let gh = [cc, c(-s, 0.), c(s, 0.), cc.conj()];
            for i in 0..=m {
                let (x, y) = (tc.at(i, m - 1), tc.at(i, m));
                tc.set(i, m - 1, x * gh[0] + y * gh[2]);
                tc.set(i, m, x * gh[1] + y * gh[3]);
            }
            for i in 0..n {
                let (x, y) = (uc.at(i, m - 1), uc.at(i, m));
                uc.set(i, m - 1, x * gh[0] + y * gh[2]);
                uc.set(i, m, x * gh[1] + y * gh[3]);
            }
            tc.set(m, m - 1, c(0., 0.));
        }
    }
    (uc, tc)
}

pub(crate) fn is_upper(t: &CMat) -> bool {
    (0..t.cols).all(|j| (j + 1..t.rows).all(|i| t.at(i, j) == C { re: 0., im: 0. }))
}

pub(crate) fn is_diag(t: &CMat) -> bool {
    (0..t.cols).all(|j| (0..t.rows).all(|i| i == j || t.at(i, j) == C { re: 0., im: 0. }))
}
