//! Dynamic specialisation of nalgebra 0.34.2's Householder bidiagonalization,
//! with cancellation at each reflector. See nalgebra-LICENSE.
#![allow(clippy::needless_range_loop)]
use nalgebra::linalg::householder;
use nalgebra::{ComplexField, DMatrix, DVector, Reflection, Unit};

pub(super) struct Bidiagonal<T: ComplexField<RealField = f64> + Copy> {
    uv: DMatrix<T>,
    diagonal: DVector<T>,
    off_diagonal: DVector<T>,
    upper_diagonal: bool,
}
impl<T: ComplexField<RealField = f64> + Copy> Bidiagonal<T> {
    pub fn new(mut matrix: DMatrix<T>, cancel: &dyn Fn() -> bool) -> Option<Self> {
        let (m, n) = matrix.shape();
        let dim = m.min(n);
        let mut diagonal = DVector::zeros(dim);
        let mut off_diagonal = DVector::zeros(dim - 1);
        let mut axis_packed = DVector::zeros(n);
        let mut work = DVector::zeros(m);
        let upper_diagonal = m >= n;
        for ite in 0..dim {
            if cancel() {
                return None;
            }
            if upper_diagonal {
                diagonal[ite] = householder::clear_column_unchecked(&mut matrix, ite, 0, None);
                if ite + 1 < dim {
                    off_diagonal[ite] = householder::clear_row_unchecked(
                        &mut matrix,
                        &mut axis_packed,
                        &mut work,
                        ite,
                        1,
                    );
                }
            } else {
                diagonal[ite] = householder::clear_row_unchecked(
                    &mut matrix,
                    &mut axis_packed,
                    &mut work,
                    ite,
                    0,
                );
                if ite + 1 < dim {
                    off_diagonal[ite] =
                        householder::clear_column_unchecked(&mut matrix, ite, 1, None);
                }
            }
        }
        Some(Self {
            uv: matrix,
            diagonal,
            off_diagonal,
            upper_diagonal,
        })
    }
    pub fn is_upper_diagonal(&self) -> bool {
        self.upper_diagonal
    }
    pub fn diagonal(&self) -> DVector<f64> {
        self.diagonal.map(|e| e.modulus())
    }
    pub fn off_diagonal(&self) -> DVector<f64> {
        self.off_diagonal.map(|e| e.modulus())
    }
    pub fn u(&self, cancel: &dyn Fn() -> bool) -> Option<DMatrix<T>> {
        let (m, n) = self.uv.shape();
        let dim = m.min(n);
        let shift = usize::from(!self.upper_diagonal);
        let mut res = DMatrix::identity(m, dim);
        for i in (0..dim - shift).rev() {
            if cancel() {
                return None;
            }
            let axis = self.uv.view_range(i + shift.., i);
            if axis.norm_squared() == 0. {
                continue;
            }
            let refl = Reflection::new(Unit::new_unchecked(axis), T::zero());
            let mut res_rows = res.view_range_mut(i + shift.., i..);
            let sign = if self.upper_diagonal {
                self.diagonal[i].signum()
            } else {
                self.off_diagonal[i].signum()
            };
            refl.reflect_with_sign(&mut res_rows, sign);
        }
        Some(res)
    }
    pub fn v_t(&self, cancel: &dyn Fn() -> bool) -> Option<DMatrix<T>> {
        let (m, n) = self.uv.shape();
        let dim = m.min(n);
        let shift = usize::from(self.upper_diagonal);
        let mut res = DMatrix::identity(dim, n);
        let mut work = DVector::zeros(dim);
        let mut axis_packed = DVector::zeros(n);
        for i in (0..dim - shift).rev() {
            if cancel() {
                return None;
            }
            let axis = self.uv.view_range(i, i + shift..);
            let mut axis_packed = axis_packed.rows_range_mut(i + shift..);
            axis_packed.tr_copy_from(&axis);
            if axis_packed.norm_squared() == 0. {
                continue;
            }
            let refl = Reflection::new(Unit::new_unchecked(axis_packed), T::zero());
            let mut res_rows = res.view_range_mut(i.., i + shift..);
            let sign = if self.upper_diagonal {
                self.off_diagonal[i].signum()
            } else {
                self.diagonal[i].signum()
            };
            refl.reflect_rows_with_sign(&mut res_rows, &mut work.rows_range_mut(i..), sign);
        }
        Some(res)
    }
}
