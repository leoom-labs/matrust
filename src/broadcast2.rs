//! Column-major singleton expansion for two-dimensional arrays.
use crate::{Matrix, Result};
use std::sync::atomic::AtomicBool;

pub(crate) const SIZE_MISMATCH: &str = "Arrays have incompatible sizes for this operation.";

#[derive(Clone, Copy)]
enum Index {
    Linear,
    Scalar,
    Column,
    Row,
}

impl Index {
    fn new(shape: (usize, usize), output: (usize, usize)) -> Self {
        match shape {
            _ if shape == output => Self::Linear,
            (1, 1) => Self::Scalar,
            (_, 1) => Self::Column,
            (1, _) => Self::Row,
            _ => unreachable!("a compatible nonsingleton operand has the output shape"),
        }
    }

    #[inline]
    fn at(self, linear: usize, rows: usize) -> usize {
        match self {
            Self::Linear => linear,
            Self::Scalar => 0,
            Self::Column => linear % rows,
            Self::Row => linear / rows,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Broadcast2 {
    pub(crate) rows: usize,
    pub(crate) cols: usize,
    len: usize,
    left: Index,
    right: Index,
}

impl Broadcast2 {
    pub(crate) fn new(left: (usize, usize), right: (usize, usize)) -> Result<Self> {
        let dimension = |a, b| {
            if a == 1 {
                Ok(b)
            } else if b == 1 || a == b {
                Ok(a)
            } else {
                Err(SIZE_MISMATCH.to_owned())
            }
        };
        let rows = dimension(left.0, right.0)?;
        let cols = dimension(left.1, right.1)?;
        Ok(Self {
            rows,
            cols,
            len: Matrix::checked_len(rows, cols)?,
            left: Index::new(left, (rows, cols)),
            right: Index::new(right, (rows, cols)),
        })
    }

    pub(crate) fn len(self) -> usize {
        self.len
    }

    pub(crate) fn needs_expansion(self) -> bool {
        matches!(self.left, Index::Column | Index::Row)
            || matches!(self.right, Index::Column | Index::Row)
    }

    #[inline]
    pub(crate) fn indices(self, i: usize) -> (usize, usize) {
        debug_assert!(i < self.len);
        (self.left.at(i, self.rows), self.right.at(i, self.rows))
    }

    pub(crate) fn checked_indices(
        self,
        cancel: &AtomicBool,
    ) -> impl Iterator<Item = Result<(usize, usize)>> + '_ {
        (0..self.len).map(move |i| {
            if i.is_multiple_of(8192) {
                crate::dsp::check_cancel(cancel)?;
            }
            Ok(self.indices(i))
        })
    }

    pub(crate) fn map<T>(
        self,
        cancel: &AtomicBool,
        mut f: impl FnMut(usize, usize) -> T,
    ) -> Result<Vec<T>> {
        crate::dsp::check_cancel(cancel)?;
        let mut data = Vec::new();
        data.try_reserve_exact(self.len)
            .map_err(|_| "Cannot allocate operation output".to_owned())?;
        for first in (0..self.len).step_by(8192) {
            crate::dsp::check_cancel(cancel)?;
            data.extend((first..(first + 8192).min(self.len)).map(|i| {
                let (a, b) = self.indices(i);
                f(a, b)
            }));
        }
        crate::dsp::check_cancel(cancel)?;
        Ok(data)
    }

    pub(crate) fn zip_real(
        self,
        left: &Matrix,
        right: &Matrix,
        cancel: &AtomicBool,
        f: impl Fn(f64, f64) -> f64 + Send + Sync,
    ) -> Result<Matrix> {
        if !self.needs_expansion() {
            crate::dsp::check_cancel(cancel)?;
            let output = left.zip(right, f)?;
            crate::dsp::check_cancel(cancel)?;
            return Ok(output);
        }
        let data = self.map(cancel, |a, b| f(left.data()[a], right.data()[b]))?;
        Matrix::from_column_vec(self.rows, self.cols, data)
    }
}
