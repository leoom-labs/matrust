//! Immutable complex-double storage foundation; script integration is separate.
//! `Complex64` contains two binary64 components (16 bytes per element).
use crate::{Matrix, Result, matrix::MAX_ELEMENTS};
use nalgebra::DMatrix;
use rayon::prelude::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

mod arithmetic;
pub use arithmetic::ComplexOp;

pub type Complex64 = nalgebra::Complex<f64>;
pub const MAX_COMPLEX_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_COMPLEX_ELEMENTS: usize = MAX_COMPLEX_BYTES / 16;
const CHUNK: usize = 4096;
// Provisional admission only: release 1/8-worker measurements must determine
// the accepted threshold before publication. Reuses the caller's Rayon pool.
const PAR_ELEMENTS: usize = 512 * 1024;

#[derive(Clone, Debug)]
pub struct ComplexMatrix {
    pub(crate) storage: Arc<DMatrix<Complex64>>,
    rows: usize,
    cols: usize,
}

fn check(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err("Calculation cancelled".into())
    } else {
        Ok(())
    }
}

fn build<T: Copy + Default + Send>(
    len: usize,
    cancel: &AtomicBool,
    value: impl Fn(usize) -> T + Sync,
) -> Result<Vec<T>> {
    check(cancel)?;
    let mut out = Vec::new();
    out.try_reserve_exact(len)
        .map_err(|_| "Cannot allocate complex operation output")?;
    if len >= PAR_ELEMENTS && rayon::current_num_threads() > 1 {
        // Safe initialized output; initialization is itself cancellable. This
        // extra write pass is included in the proposed benchmark, not hidden.
        for first in (0..len).step_by(CHUNK) {
            check(cancel)?;
            out.resize((first + CHUNK).min(len), T::default());
        }
        out.par_chunks_mut(CHUNK)
            .enumerate()
            .try_for_each(|(block, chunk)| -> Result<()> {
                check(cancel)?;
                for (i, slot) in chunk.iter_mut().enumerate() {
                    *slot = value(block * CHUNK + i);
                }
                #[cfg(test)]
                cancellation_probe::completed(cancel);
                check(cancel)
            })?;
    } else {
        for first in (0..len).step_by(CHUNK) {
            check(cancel)?;
            out.extend((first..(first + CHUNK).min(len)).map(&value));
            #[cfg(test)]
            cancellation_probe::completed(cancel);
            check(cancel)?;
        }
    }
    check(cancel)?;
    Ok(out)
}

impl ComplexMatrix {
    pub fn checked_len(rows: usize, cols: usize) -> Result<usize> {
        let len = rows
            .checked_mul(cols)
            .ok_or("Complex matrix dimensions overflow")?;
        if rows > MAX_ELEMENTS || cols > MAX_ELEMENTS || len > MAX_COMPLEX_ELEMENTS {
            return Err("Complex matrix exceeds dimension or 128 MiB payload limit".into());
        }
        Ok(len)
    }
    /// Takes ownership without cloning its column-major payload. Allocation of
    /// the supplied Vec belongs to the caller; this method validates admission.
    pub fn from_column_vec(
        rows: usize,
        cols: usize,
        data: Vec<Complex64>,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        check(cancel)?;
        if Self::checked_len(rows, cols)? != data.len() {
            return Err("Complex data does not match dimensions".into());
        }
        let result = Self {
            storage: Arc::new(DMatrix::from_vec(rows, cols, data)),
            rows,
            cols,
        };
        check(cancel)?;
        Ok(result)
    }
    pub fn from_column_slice(
        rows: usize,
        cols: usize,
        data: &[Complex64],
        cancel: &AtomicBool,
    ) -> Result<Self> {
        check(cancel)?;
        let len = Self::checked_len(rows, cols)?;
        if len != data.len() {
            return Err("Complex data does not match dimensions".into());
        }
        Self::from_column_vec(rows, cols, build(len, cancel, |i| data[i])?, cancel)
    }
    pub fn filled(rows: usize, cols: usize, value: Complex64, cancel: &AtomicBool) -> Result<Self> {
        check(cancel)?;
        let len = Self::checked_len(rows, cols)?;
        Self::from_column_vec(rows, cols, build(len, cancel, |_| value)?, cancel)
    }
    pub fn scalar(value: Complex64, cancel: &AtomicBool) -> Result<Self> {
        Self::filled(1, 1, value, cancel)
    }
    pub fn from_real(real: &Matrix, cancel: &AtomicBool) -> Result<Self> {
        check(cancel)?;
        let len = Self::checked_len(real.rows(), real.cols())?;
        Self::from_column_vec(
            real.rows(),
            real.cols(),
            build(len, cancel, |i| Complex64::new(real.data()[i], 0.))?,
            cancel,
        )
    }
    /// Equal shapes or scalar expansion only. Components are copied directly,
    /// preserving explicit zero imaginary storage and each component's bits.
    pub fn from_parts(real: &Matrix, imaginary: &Matrix, cancel: &AtomicBool) -> Result<Self> {
        check(cancel)?;
        let shape = if real.rows() == imaginary.rows() && real.cols() == imaginary.cols() {
            (real.rows(), real.cols())
        } else if real.len() == 1 {
            (imaginary.rows(), imaginary.cols())
        } else if imaginary.len() == 1 {
            (real.rows(), real.cols())
        } else {
            return Err("Complex components need equal shapes or a scalar".into());
        };
        let len = Self::checked_len(shape.0, shape.1)?;
        let data = build(len, cancel, |i| {
            Complex64::new(
                real.data()[if real.len() == 1 { 0 } else { i }],
                imaginary.data()[if imaginary.len() == 1 { 0 } else { i }],
            )
        })?;
        Self::from_column_vec(shape.0, shape.1, data, cancel)
    }
    pub fn rows(&self) -> usize {
        self.rows
    }
    pub fn cols(&self) -> usize {
        self.cols
    }
    pub fn len(&self) -> usize {
        self.storage.len()
    }
    pub fn is_empty(&self) -> bool {
        self.storage.is_empty()
    }
    pub fn bytes(&self) -> usize {
        self.len() * 16
    }
    pub fn data(&self) -> &[Complex64] {
        self.storage.as_slice()
    }
    pub fn get(&self, row: usize, col: usize) -> Result<Complex64> {
        if row >= self.rows || col >= self.cols {
            return Err("Cell is outside complex matrix dimensions".into());
        }
        Ok(self.data()[col * self.rows + row])
    }
    pub fn shares_storage(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.storage, &other.storage)
    }
    pub fn same_snapshot(&self, other: &Self) -> bool {
        self.rows == other.rows && self.cols == other.cols && self.shares_storage(other)
    }
    pub fn reshape(&self, rows: usize, cols: usize, cancel: &AtomicBool) -> Result<Self> {
        check(cancel)?;
        if Self::checked_len(rows, cols)? != self.len() {
            return Err("Reshape must preserve complex element count".into());
        }
        let result = Self {
            storage: self.storage.clone(),
            rows,
            cols,
        };
        check(cancel)?;
        Ok(result)
    }
    pub fn real(&self, cancel: &AtomicBool) -> Result<Matrix> {
        let data = build(self.len(), cancel, |i| self.data()[i].re)?;
        let result = Matrix::from_column_vec(self.rows, self.cols, data)?;
        check(cancel)?;
        Ok(result)
    }
    pub fn imag(&self, cancel: &AtomicBool) -> Result<Matrix> {
        let data = build(self.len(), cancel, |i| self.data()[i].im)?;
        let result = Matrix::from_column_vec(self.rows, self.cols, data)?;
        check(cancel)?;
        Ok(result)
    }
    pub fn conj(&self, cancel: &AtomicBool) -> Result<Self> {
        if self.is_empty() {
            return self.reshape(self.rows, self.cols, cancel);
        }
        let data = build(self.len(), cancel, |i| {
            Complex64::new(self.data()[i].re, -self.data()[i].im)
        })?;
        Self::from_column_vec(self.rows, self.cols, data, cancel)
    }
    pub fn transpose(&self, cancel: &AtomicBool) -> Result<Self> {
        self.transposed(false, cancel)
    }
    pub fn adjoint(&self, cancel: &AtomicBool) -> Result<Self> {
        self.transposed(true, cancel)
    }
    fn transposed(&self, conjugate: bool, cancel: &AtomicBool) -> Result<Self> {
        check(cancel)?;
        if self.is_empty() || (!conjugate && (self.rows == 1 || self.cols == 1)) {
            return self.reshape(self.cols, self.rows, cancel);
        }
        let data = build(self.len(), cancel, |i| {
            let value = self.data()[(i % self.cols) * self.rows + i / self.cols];
            if conjugate {
                Complex64::new(value.re, -value.im)
            } else {
                value
            }
        })?;
        Self::from_column_vec(self.cols, self.rows, data, cancel)
    }
}

impl ComplexMatrix {
    /// Borrowed real/logical components avoid full f64 conversion buffers.
    pub(crate) fn from_component_views(
        real: crate::logical::ArrayRef<'_>,
        imaginary: Option<crate::logical::ArrayRef<'_>>,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        check(cancel)?;
        let shape = if let Some(imaginary) = imaginary {
            if real.shape() == imaginary.shape() || imaginary.len() == 1 {
                real.shape()
            } else if real.len() == 1 {
                imaginary.shape()
            } else {
                return Err("Complex components need equal shapes or a scalar".into());
            }
        } else {
            real.shape()
        };
        let len = Self::checked_len(shape.0, shape.1)?;
        let at = |part: crate::logical::ArrayRef<'_>, i: usize| {
            let i = if part.len() == 1 { 0 } else { i };
            match part {
                crate::logical::ArrayRef::Numeric(a) => a.data()[i],
                crate::logical::ArrayRef::Logical(a) => f64::from(a.data()[i]),
            }
        };
        Self::from_column_vec(
            shape.0,
            shape.1,
            build(len, cancel, |i| {
                Complex64::new(at(real, i), imaginary.map_or(0., |a| at(a, i)))
            })?,
            cancel,
        )
    }
    pub fn magnitude(&self, cancel: &AtomicBool) -> Result<Matrix> {
        let data = build(self.len(), cancel, |i| {
            self.data()[i].re.hypot(self.data()[i].im)
        })?;
        check(cancel)?;
        Matrix::from_column_vec(self.rows(), self.cols(), data)
    }
    pub fn angle(&self, cancel: &AtomicBool) -> Result<Matrix> {
        let data = build(self.len(), cancel, |i| {
            self.data()[i].im.atan2(self.data()[i].re)
        })?;
        check(cancel)?;
        Matrix::from_column_vec(self.rows(), self.cols(), data)
    }
    pub fn negated(&self, cancel: &AtomicBool) -> Result<Self> {
        Self::from_column_vec(
            self.rows(),
            self.cols(),
            build(self.len(), cancel, |i| {
                Complex64::new(-self.data()[i].re, -self.data()[i].im)
            })?,
            cancel,
        )
    }
    pub fn with_value(
        &self,
        row: usize,
        col: usize,
        value: Complex64,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        check(cancel)?;
        self.get(row, col)?;
        let at = col * self.rows() + row;
        Self::from_column_vec(
            self.rows(),
            self.cols(),
            build(self.len(), cancel, |i| {
                if i == at { value } else { self.data()[i] }
            })?,
            cancel,
        )
    }
    pub fn preview(&self, limit: usize) -> String {
        use std::fmt::Write;
        let mut output = String::new();
        for (i, value) in self.data().iter().take(limit.min(64)).enumerate() {
            if i > 0 {
                output.push(' ');
            }
            let _ = write!(output, "{}", crate::Scalar::Complex(*value));
        }
        if self.len() > limit.min(64) {
            output.push_str(" ...");
        }
        output
    }
}
impl std::fmt::Display for ComplexMatrix {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{}x{} complex double", self.rows(), self.cols())?;
        for row in 0..self.rows().min(8) {
            for col in 0..self.cols().min(8) {
                if col > 0 {
                    f.write_str("  ")?;
                }
                write!(
                    f,
                    "{}",
                    crate::Scalar::Complex(self.data()[col * self.rows() + row])
                )?;
            }
            if self.cols() > 8 {
                f.write_str(" ...")?;
            }
            f.write_str("\n")?;
        }
        if self.rows() > 8 {
            f.write_str("...\n")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod cancellation_probe {
    use super::*;
    use std::cell::Cell;
    thread_local! { static REMAINING: Cell<Option<usize>> = const { Cell::new(None) }; static DONE: Cell<usize> = const { Cell::new(0) }; }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            REMAINING.set(None);
        }
    }
    #[test]
    fn complex_elementwise_active_cancellation_preserves_shared_operands() {
        use crate::{Array, complex_matrix::ComplexOp};
        let cancel = AtomicBool::new(false);
        let a: Array = ComplexMatrix::filled(3 * CHUNK + 7, 1, Complex64::new(3., 4.), &cancel)
            .unwrap()
            .into();
        let b: Array = ComplexMatrix::scalar(Complex64::new(1., -2.), &cancel)
            .unwrap()
            .into();
        let aliases = (a.clone(), b.clone());
        for op in [
            ComplexOp::Add,
            ComplexOp::Subtract,
            ComplexOp::Multiply,
            ComplexOp::Divide,
        ] {
            let expected = ComplexMatrix::arithmetic(&a, &b, op, &cancel).unwrap();
            DONE.set(0);
            REMAINING.set(Some(2));
            let _reset = Reset;
            assert!(ComplexMatrix::arithmetic(&a, &b, op, &cancel).is_err());
            assert_eq!(DONE.get(), 2);
            assert!(cancel.load(Ordering::Relaxed));
            assert_eq!(REMAINING.get(), None);
            assert!(a.same_snapshot(&aliases.0) && b.same_snapshot(&aliases.1));
            cancel.store(false, Ordering::Relaxed);
            let retry = ComplexMatrix::arithmetic(&a, &b, op, &cancel).unwrap();
            assert_eq!(retry.data(), expected.data());
        }
    }

    pub(super) fn completed(cancel: &AtomicBool) {
        if let Some(left) = REMAINING.get() {
            DONE.set(DONE.get() + 1);
            if left == 1 {
                REMAINING.set(None);
                cancel.store(true, Ordering::Relaxed);
            } else {
                REMAINING.set(Some(left - 1));
            }
        }
    }
    #[test]
    fn cancellation_after_real_progress_discards_output_and_preserves_aliases() {
        let cancel = AtomicBool::new(false);
        let source =
            ComplexMatrix::filled(3 * CHUNK + 7, 1, Complex64::new(2., -0.), &cancel).unwrap();
        let alias = source.clone();
        for operation in 0..5 {
            cancel.store(false, Ordering::Relaxed);
            DONE.set(0);
            REMAINING.set(Some(2));
            let _reset = Reset;
            let failed = match operation {
                0 => source.conj(&cancel).is_err(),
                1 => source.real(&cancel).is_err(),
                2 => source.imag(&cancel).is_err(),
                3 => source.adjoint(&cancel).is_err(),
                _ => ComplexMatrix::from_column_slice(
                    source.rows(),
                    source.cols(),
                    source.data(),
                    &cancel,
                )
                .is_err(),
            };
            assert!(failed && cancel.load(Ordering::Relaxed));
            assert_eq!(DONE.get(), 2);
            assert!(source.same_snapshot(&alias));
            assert!(
                source
                    .data()
                    .iter()
                    .all(|v| v.re == 2. && v.im.to_bits() == (-0f64).to_bits())
            );
        }
        cancel.store(false, Ordering::Relaxed);
        assert!(
            source
                .conj(&cancel)
                .unwrap()
                .data()
                .iter()
                .all(|v| v.im.to_bits() == 0)
        );
    }
}
