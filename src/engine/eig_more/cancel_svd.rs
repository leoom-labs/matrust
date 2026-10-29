//! Dynamic f64/complex SVD port of nalgebra 0.34.2's implicit-shift algorithm.
//! Only callback plumbing and dynamic specialisation differ; arithmetic order
//! and the bidiagonal Householder/Givens kernels are retained. See nalgebra-LICENSE.
#![allow(
    clippy::needless_range_loop,
    clippy::too_many_arguments,
    clippy::type_complexity
)]
use super::cancel_bidiagonal::Bidiagonal;
use nalgebra::linalg::givens::GivensRotation;
use nalgebra::{ComplexField, DMatrix, DVector, Matrix2x3, Vector2};
pub(super) struct Svd<T: ComplexField<RealField = f64> + Copy> {
    pub u: Option<DMatrix<T>>,
    pub v_t: Option<DMatrix<T>>,
    pub singular_values: DVector<f64>,
}
impl<T: ComplexField<RealField = f64> + Copy> Svd<T> {
    pub fn try_new(
        matrix: DMatrix<T>,
        compute_u: bool,
        compute_v: bool,
        eps: f64,
        max_niter: usize,
        cancel: &dyn Fn() -> bool,
    ) -> Option<Self> {
        let raw = Self::try_new_unordered(matrix, compute_u, compute_v, eps, max_niter, cancel)?;
        let mut sorted = nalgebra::linalg::SVD {
            u: raw.u,
            v_t: raw.v_t,
            singular_values: raw.singular_values,
        };
        sorted.sort_by_singular_values();
        if cancel() {
            return None;
        }
        Some(Self {
            u: sorted.u,
            v_t: sorted.v_t,
            singular_values: sorted.singular_values,
        })
    }
    pub fn try_new_unordered(
        mut matrix: DMatrix<T>,
        compute_u: bool,
        compute_v: bool,
        eps: f64,
        max_niter: usize,
        cancel: &dyn Fn() -> bool,
    ) -> Option<Self> {
        assert!(
            !matrix.is_empty(),
            "Cannot compute the SVD of an empty matrix."
        );
        let (nrows, ncols) = matrix.shape();
        let min_nrows_ncols = nrows.min(ncols);

        let dim = min_nrows_ncols;

        let m_amax = matrix.camax();

        if !(m_amax == 0.) {
            matrix.unscale_mut(m_amax);
        }

        let bi_matrix = Bidiagonal::new(matrix, cancel)?;
        let mut u = if compute_u {
            Some(bi_matrix.u(cancel)?)
        } else {
            None
        };
        let mut v_t = if compute_v {
            Some(bi_matrix.v_t(cancel)?)
        } else {
            None
        };
        let mut diagonal = bi_matrix.diagonal();
        let mut off_diagonal = bi_matrix.off_diagonal();

        let mut niter = 0;
        let (mut start, mut end) = Self::delimit_subproblem(
            &mut diagonal,
            &mut off_diagonal,
            &mut u,
            &mut v_t,
            bi_matrix.is_upper_diagonal(),
            dim - 1,
            eps,
            cancel,
        );

        while end != start {
            if cancel() {
                return None;
            }
            let subdim = end - start + 1;

            // Solve the subproblem.
            #[allow(clippy::comparison_chain)]
            if subdim > 2 {
                let m = end - 1;
                let n = end;

                let mut vec;
                {
                    let dm = diagonal[m];
                    let dn = diagonal[n];
                    let fm = off_diagonal[m];

                    let tmm = dm * dm + off_diagonal[m - 1] * off_diagonal[m - 1];
                    let tmn = dm * fm;
                    let tnn = dn * dn + fm * fm;

                    let shift = wilkinson_shift(tmm, tnn, tmn);

                    vec = Vector2::new(
                        diagonal[start] * diagonal[start] - shift,
                        diagonal[start] * off_diagonal[start],
                    );
                }

                for k in start..n {
                    if cancel() {
                        return None;
                    }
                    let m12 = if k == n - 1 { 0.0 } else { off_diagonal[k + 1] };

                    let mut subm = Matrix2x3::new(
                        diagonal[k],
                        off_diagonal[k],
                        0.0,
                        0.0,
                        diagonal[k + 1],
                        m12,
                    );

                    match GivensRotation::cancel_y(&vec) {
                        Some((rot1, norm1)) => {
                            rot1.inverse()
                                .rotate_rows(&mut subm.fixed_columns_mut::<2>(0));
                            let rot1 =
                                GivensRotation::new_unchecked(rot1.c(), T::from_real(rot1.s()));

                            if k > start {
                                // This is not the first iteration.
                                off_diagonal[k - 1] = norm1;
                            }

                            let v = Vector2::new(subm[(0, 0)], subm[(1, 0)]);
                            // TODO: does the case `v.y == 0` ever happen?
                            let (rot2, norm2) = GivensRotation::cancel_y(&v)
                                .unwrap_or((GivensRotation::identity(), subm[(0, 0)]));

                            rot2.rotate(&mut subm.fixed_columns_mut::<2>(1));
                            let rot2 =
                                GivensRotation::new_unchecked(rot2.c(), T::from_real(rot2.s()));

                            subm[(0, 0)] = norm2;

                            if let Some(ref mut v_t) = v_t {
                                if bi_matrix.is_upper_diagonal() {
                                    rot1.rotate(&mut v_t.fixed_rows_mut::<2>(k));
                                } else {
                                    rot2.rotate(&mut v_t.fixed_rows_mut::<2>(k));
                                }
                            }

                            if let Some(ref mut u) = u {
                                if bi_matrix.is_upper_diagonal() {
                                    rot2.inverse().rotate_rows(&mut u.fixed_columns_mut::<2>(k));
                                } else {
                                    rot1.inverse().rotate_rows(&mut u.fixed_columns_mut::<2>(k));
                                }
                            }

                            diagonal[k] = subm[(0, 0)];
                            diagonal[k + 1] = subm[(1, 1)];
                            off_diagonal[k] = subm[(0, 1)];

                            if k != n - 1 {
                                off_diagonal[k + 1] = subm[(1, 2)];
                            }

                            vec.x = subm[(0, 1)];
                            vec.y = subm[(0, 2)];
                        }
                        None => {
                            break;
                        }
                    }
                }
            } else if subdim == 2 {
                // Solve the remaining 2x2 subproblem.
                let (u2, s, v2) = compute_2x2_uptrig_svd(
                    diagonal[start],
                    off_diagonal[start],
                    diagonal[start + 1],
                    compute_u && bi_matrix.is_upper_diagonal()
                        || compute_v && !bi_matrix.is_upper_diagonal(),
                    compute_v && bi_matrix.is_upper_diagonal()
                        || compute_u && !bi_matrix.is_upper_diagonal(),
                );
                let u2 = u2.map(|u2| GivensRotation::new_unchecked(u2.c(), T::from_real(u2.s())));
                let v2 = v2.map(|v2| GivensRotation::new_unchecked(v2.c(), T::from_real(v2.s())));

                diagonal[start] = s[0];
                diagonal[start + 1] = s[1];
                off_diagonal[start] = 0.0;

                if let Some(ref mut u) = u {
                    let rot = if bi_matrix.is_upper_diagonal() {
                        u2.unwrap()
                    } else {
                        v2.unwrap()
                    };
                    rot.rotate_rows(&mut u.fixed_columns_mut::<2>(start));
                }

                if let Some(ref mut v_t) = v_t {
                    let rot = if bi_matrix.is_upper_diagonal() {
                        v2.unwrap()
                    } else {
                        u2.unwrap()
                    };
                    rot.inverse().rotate(&mut v_t.fixed_rows_mut::<2>(start));
                }

                end -= 1;
            }

            // Re-delimit the subproblem in case some decoupling occurred.
            let sub = Self::delimit_subproblem(
                &mut diagonal,
                &mut off_diagonal,
                &mut u,
                &mut v_t,
                bi_matrix.is_upper_diagonal(),
                end,
                eps,
                cancel,
            );
            start = sub.0;
            end = sub.1;

            niter += 1;
            if niter == max_niter {
                return None;
            }
        }

        diagonal *= m_amax;

        // Ensure all singular value are non-negative.
        for i in 0..dim {
            if cancel() {
                return None;
            }
            let sval = diagonal[i];

            if sval < 0.0 {
                diagonal[i] = -sval;

                if let Some(ref mut u) = u {
                    u.column_mut(i).neg_mut();
                }
            }
        }

        if cancel() {
            return None;
        }
        Some(Self {
            u,
            v_t,
            singular_values: diagonal,
        })
    }
    fn delimit_subproblem(
        diagonal: &mut DVector<f64>,
        off_diagonal: &mut DVector<f64>,
        u: &mut Option<DMatrix<T>>,
        v_t: &mut Option<DMatrix<T>>,
        is_upper_diagonal: bool,
        end: usize,
        eps: f64,
        cancel: &dyn Fn() -> bool,
    ) -> (usize, usize) {
        let mut n = end;

        while n > 0 {
            if cancel() {
                return (0, 0);
            }
            let m = n - 1;

            if (off_diagonal[m] == 0.)
                || off_diagonal[m].norm1() <= eps * (diagonal[n].norm1() + diagonal[m].norm1())
            {
                off_diagonal[m] = 0.0;
            } else if diagonal[m].norm1() <= eps {
                diagonal[m] = 0.0;
                Self::cancel_horizontal_off_diagonal_elt(
                    diagonal,
                    off_diagonal,
                    u,
                    v_t,
                    is_upper_diagonal,
                    m,
                    m + 1,
                    cancel,
                );

                if m != 0 {
                    Self::cancel_vertical_off_diagonal_elt(
                        diagonal,
                        off_diagonal,
                        u,
                        v_t,
                        is_upper_diagonal,
                        m - 1,
                        cancel,
                    );
                }
            } else if diagonal[n].norm1() <= eps {
                diagonal[n] = 0.0;
                Self::cancel_vertical_off_diagonal_elt(
                    diagonal,
                    off_diagonal,
                    u,
                    v_t,
                    is_upper_diagonal,
                    m,
                    cancel,
                );
            } else {
                break;
            }

            n -= 1;
        }

        if n == 0 {
            return (0, 0);
        }

        let mut new_start = n - 1;
        while new_start > 0 {
            if cancel() {
                return (0, 0);
            }
            let m = new_start - 1;

            if off_diagonal[m].norm1() <= eps * (diagonal[new_start].norm1() + diagonal[m].norm1())
            {
                off_diagonal[m] = 0.0;
                break;
            }
            // TODO: write a test that enters this case.
            else if diagonal[m].norm1() <= eps {
                diagonal[m] = 0.0;
                Self::cancel_horizontal_off_diagonal_elt(
                    diagonal,
                    off_diagonal,
                    u,
                    v_t,
                    is_upper_diagonal,
                    m,
                    n,
                    cancel,
                );

                if m != 0 {
                    Self::cancel_vertical_off_diagonal_elt(
                        diagonal,
                        off_diagonal,
                        u,
                        v_t,
                        is_upper_diagonal,
                        m - 1,
                        cancel,
                    );
                }
                break;
            }

            new_start -= 1;
        }

        (new_start, n)
    }
    fn cancel_horizontal_off_diagonal_elt(
        diagonal: &mut DVector<f64>,
        off_diagonal: &mut DVector<f64>,
        u: &mut Option<DMatrix<T>>,
        v_t: &mut Option<DMatrix<T>>,
        is_upper_diagonal: bool,
        i: usize,
        end: usize,
        cancel: &dyn Fn() -> bool,
    ) {
        let mut v = Vector2::new(off_diagonal[i], diagonal[i + 1]);
        off_diagonal[i] = 0.0;

        for k in i..end {
            if cancel() {
                return;
            }
            match GivensRotation::cancel_x(&v) {
                Some((rot, norm)) => {
                    let rot = GivensRotation::new_unchecked(rot.c(), T::from_real(rot.s()));
                    diagonal[k + 1] = norm;

                    if is_upper_diagonal {
                        if let Some(ref mut u) = *u {
                            rot.inverse()
                                .rotate_rows(&mut u.fixed_columns_with_step_mut::<2>(i, k - i));
                        }
                    } else if let Some(ref mut v_t) = *v_t {
                        rot.rotate(&mut v_t.fixed_rows_with_step_mut::<2>(i, k - i));
                    }

                    if k + 1 != end {
                        v.x = -rot.s().real() * off_diagonal[k + 1];
                        v.y = diagonal[k + 2];
                        off_diagonal[k + 1] *= rot.c();
                    }
                }
                None => {
                    break;
                }
            }
        }
    }
    fn cancel_vertical_off_diagonal_elt(
        diagonal: &mut DVector<f64>,
        off_diagonal: &mut DVector<f64>,
        u: &mut Option<DMatrix<T>>,
        v_t: &mut Option<DMatrix<T>>,
        is_upper_diagonal: bool,
        i: usize,
        cancel: &dyn Fn() -> bool,
    ) {
        let mut v = Vector2::new(diagonal[i], off_diagonal[i]);
        off_diagonal[i] = 0.0;

        for k in (0..i + 1).rev() {
            if cancel() {
                return;
            }
            match GivensRotation::cancel_y(&v) {
                Some((rot, norm)) => {
                    let rot = GivensRotation::new_unchecked(rot.c(), T::from_real(rot.s()));
                    diagonal[k] = norm;

                    if is_upper_diagonal {
                        if let Some(ref mut v_t) = *v_t {
                            rot.rotate(&mut v_t.fixed_rows_with_step_mut::<2>(k, i - k));
                        }
                    } else if let Some(ref mut u) = *u {
                        rot.inverse()
                            .rotate_rows(&mut u.fixed_columns_with_step_mut::<2>(k, i - k));
                    }

                    if k > 0 {
                        v.x = diagonal[k - 1];
                        v.y = rot.s().real() * off_diagonal[k - 1];
                        off_diagonal[k - 1] *= rot.c();
                    }
                }
                None => {
                    break;
                }
            }
        }
    }
}
fn compute_2x2_uptrig_svd(
    m11: f64,
    m12: f64,
    m22: f64,
    compute_u: bool,
    compute_v: bool,
) -> (
    Option<GivensRotation<f64>>,
    Vector2<f64>,
    Option<GivensRotation<f64>>,
) {
    let two: f64 = nalgebra::convert(2.0f64);
    let half: f64 = nalgebra::convert(0.5f64);

    let denom = (m11 + m22).hypot(m12) + (m11 - m22).hypot(m12);

    // NOTE: v1 is the singular value that is the closest to m22.
    // This prevents cancellation issues when constructing the vector `csv` below. If we chose
    // otherwise, we would have v1 ~= m11 when m12 is small. This would cause catastrophic
    // cancellation on `v1 * v1 - m11 * m11` below.
    let mut v1 = m11 * m22 * two / denom;
    let mut v2 = half * denom;

    let mut u = None;
    let mut v_t = None;

    if compute_u || compute_v {
        let (csv, sgn_v) = GivensRotation::new(m11 * m12, v1 * v1 - m11 * m11);
        v1 *= sgn_v;
        v2 *= sgn_v;

        if compute_v {
            v_t = Some(csv);
        }

        let cu = (m11.scale(csv.c()) + m12 * csv.s()) / v1;
        let su = (m22 * csv.s()) / v1;
        let (csu, sgn_u) = GivensRotation::new(cu, su);
        v1 *= sgn_u;
        v2 *= sgn_u;

        if compute_u {
            u = Some(csu);
        }
    }

    (u, Vector2::new(v1, v2), v_t)
}

fn wilkinson_shift(tmm: f64, tnn: f64, tmn: f64) -> f64 {
    let sq_tmn = tmn * tmn;
    if sq_tmn != 0. {
        // We have the guarantee that the denominator won't be zero.
        let d = (tmm - tnn) * 0.5;
        tnn - sq_tmn / (d + d.signum() * (d * d + sq_tmn).sqrt())
    } else {
        tnn
    }
}
