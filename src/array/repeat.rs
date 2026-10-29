//! Typed repetition with one output allocation and bounded exact slice copies.
use crate::{Array, ComplexMatrix, LogicalMatrix, Matrix, Result};
use std::sync::atomic::{AtomicBool, Ordering};

const COPY_CHUNK: usize = 4096;

fn check(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err("Calculation cancelled".into())
    } else {
        Ok(())
    }
}

fn copy_source<T: Copy>(
    out: &mut Vec<T>,
    source: &[T],
    check: &mut impl FnMut() -> Result<()>,
) -> Result<()> {
    for block in source.chunks(COPY_CHUNK) {
        check()?;
        out.extend_from_slice(block);
    }
    Ok(())
}

// Repeat a fully initialized prefix of the current column/block. Doubling tiny
// prefixes avoids a separate per-value loop for scalar/short-vector inputs.
fn repeat_prefix<T: Copy>(
    out: &mut Vec<T>,
    start: usize,
    end: usize,
    check: &mut impl FnMut() -> Result<()>,
) -> Result<()> {
    while out.len() < end {
        let count = (out.len() - start).min(end - out.len());
        // Callers enter only after a nonempty source prefix was copied.
        debug_assert!(count > 0);
        // Each doubling copies the whole existing prefix (or its final
        // truncation). Chunk offsets advance through that prefix; restarting at
        // zero every 4096 values would corrupt non-divisor period lengths.
        for first in (0..count).step_by(COPY_CHUNK) {
            check()?;
            let n = (count - first).min(COPY_CHUNK);
            out.extend_from_within(start + first..start + first + n);
        }
    }
    Ok(())
}

fn repeated<T: Copy>(
    source: &[T],
    source_rows: usize,
    rows: usize,
    count: usize,
    mut check: impl FnMut() -> Result<()>,
) -> Result<Vec<T>> {
    check()?;
    let mut out = Vec::new();
    out.try_reserve_exact(count)
        .map_err(|_| "Cannot allocate repeated output")?;
    if count == 0 {
        check()?;
        return Ok(out);
    }
    debug_assert!(source_rows > 0 && !source.is_empty());
    if rows == source_rows {
        copy_source(&mut out, source, &mut check)?;
    } else {
        // Materialize the row repeats in each original column once; then the
        // complete original-column block can be repeated horizontally.
        for column in source.chunks_exact(source_rows) {
            let start = out.len();
            copy_source(&mut out, column, &mut check)?;
            repeat_prefix(&mut out, start, start + rows, &mut check)?;
        }
    }
    repeat_prefix(&mut out, 0, count, &mut check)?;
    check()?;
    Ok(out)
}

impl Array {
    /// Validate the final typed output, including dimensions of empty arrays.
    pub fn repeated_shape(
        &self,
        row_copies: usize,
        column_copies: usize,
    ) -> Result<(usize, usize)> {
        self.require_numeric_array()?;
        if row_copies > crate::matrix::MAX_ELEMENTS || column_copies > crate::matrix::MAX_ELEMENTS {
            return Err("Repetition factors exceed the dimension limit".into());
        }
        let rows = self
            .rows()
            .checked_mul(row_copies)
            .ok_or("Repeated row dimension overflow")?;
        let cols = self
            .cols()
            .checked_mul(column_copies)
            .ok_or("Repeated column dimension overflow")?;
        if self.is_complex() {
            ComplexMatrix::checked_len(rows, cols)?;
        } else {
            Matrix::checked_len(rows, cols)?;
        }
        Ok((rows, cols))
    }

    pub fn repeated(
        &self,
        row_copies: usize,
        column_copies: usize,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        check(cancel)?;
        let (rows, cols) = self.repeated_shape(row_copies, column_copies)?;
        if row_copies == 1 && column_copies == 1 {
            check(cancel)?;
            let mut output = self.clone();
            if let (Self::Numeric(output), Self::Numeric(input)) = (&mut output, self) {
                output.inherit_scalar_repetition(input);
            }
            return Ok(output);
        }
        let count = rows * cols;
        let out = match self {
            Self::Character(_)
            | Self::Cell(_)
            | Self::Graphics(_)
            | Self::Struct(_)
            | Self::Handle(_)
            | Self::Typed(_) => {
                return Err("Repetition of character, cell, struct, function handle or graphics handle arrays is not implemented".into());
            }
            Self::Numeric(a) => {
                let mut output = Matrix::from_column_vec(
                    rows,
                    cols,
                    repeated(a.data(), a.rows(), rows, count, || check(cancel))?,
                )?;
                output.inherit_scalar_repetition(a);
                Self::Numeric(output)
            }
            Self::Logical(a) => Self::Logical(LogicalMatrix::from_column_vec(
                rows,
                cols,
                repeated(a.data(), a.rows(), rows, count, || check(cancel))?,
                cancel,
            )?),
            Self::Complex(a) => Self::Complex(ComplexMatrix::from_column_vec(
                rows,
                cols,
                repeated(a.data(), a.rows(), rows, count, || check(cancel))?,
                cancel,
            )?),
        };
        check(cancel)?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repetition_cancel_during_source_copy_and_prefix_growth_is_atomic() {
        let source: Vec<_> = (0..COPY_CHUNK + 1).map(|i| i as u64).collect();
        for (rows, copies, stop_at) in [(source.len(), 2, 3), (source.len() * 2, 2, 4)] {
            let mut checks = 0;
            let result = repeated(&source, source.len(), rows, rows * copies, || {
                checks += 1;
                if checks == stop_at {
                    Err("Calculation cancelled".into())
                } else {
                    Ok(())
                }
            });
            assert!(result.unwrap_err().contains("cancelled"));
            assert_eq!(checks, stop_at);
            let retry = repeated(&source, source.len(), rows, rows * copies, || Ok(())).unwrap();
            assert_eq!(retry.len(), rows * copies);
            assert!(
                retry
                    .iter()
                    .enumerate()
                    .all(|(i, &v)| v == source[i % source.len()])
            );
        }
        assert_eq!(source[0], 0);
        assert_eq!(source[source.len() - 1], COPY_CHUNK as u64);
    }
}
