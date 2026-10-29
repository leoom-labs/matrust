//! Stable lexicographic row ordering by bounded scalar-key passes.
use super::sort::{Key, allocate, build, check, compare_keys as compare, sort_lane, typed_key};
use crate::{
    Array, LogicalMatrix, Matrix, Result,
    array::{MissingPlacement, SortOrder},
    matrix::MAX_ELEMENTS,
};
use std::sync::atomic::AtomicBool;

const RUN: usize = 8192;
// Provisional, inherited from sort; release measurements must accept/change it.
const PAR_ROWS: usize = 512 * 1024;
const MAX_WORK: usize = 512 * 1024 * 1024;

fn validate(
    rows: usize,
    cols: usize,
    columns: Option<&[i32]>,
    cancel: &AtomicBool,
) -> Result<usize> {
    check(cancel)?;
    Matrix::checked_len(rows, cols)?;
    let keys = columns.map_or(cols, <[i32]>::len);
    if let Some(columns) = columns {
        if columns.is_empty() || columns.len() > MAX_ELEMENTS {
            return Err("sortrows column vector must be nonempty and bounded".into());
        }
        for (at, &column) in columns.iter().enumerate() {
            if at.is_multiple_of(RUN) {
                check(cancel)?;
            }
            if column == 0 || column.unsigned_abs() as usize > cols {
                return Err(
                    "sortrows column indices must be nonzero integers within the input columns"
                        .into(),
                );
            }
        }
    }
    // Nominal comparison/rank work bound, not a CPU-instruction/time estimate.
    // All default columns at the dense-array bound fit; pathological repeated
    // explicit keys cannot multiply work without a bound.
    if rows > 1 {
        let levels = usize::BITS as usize - (rows - 1).leading_zeros() as usize;
        if rows
            .checked_mul(keys)
            .and_then(|n| n.checked_mul(levels + 2))
            .is_none_or(|n| n > MAX_WORK)
        {
            return Err("sortrows exceeds its bounded row/key work budget".into());
        }
    }
    Ok(keys)
}

fn permutation<K: Into<Key>>(
    rows: usize,
    keys: usize,
    columns: Option<&[i32]>,
    direction: Option<SortOrder>,
    missing: MissingPlacement,
    cancel: &AtomicBool,
    read: impl Fn(usize) -> K + Sync,
) -> Result<Vec<u32>> {
    let mut order = build(rows, cancel, |row| Ok(row as u32))?;
    if rows == 2 {
        // Compare complete rows in priority order. The first unequal key alone
        // determines a two-row permutation; equal rows retain their input order.
        // Poll after at most RUN scalar reads, including a final decisive key.
        for key in 0..keys {
            if key.is_multiple_of(RUN / 2) {
                check(cancel)?;
            }
            let signed = columns.map_or((key + 1) as i32, |columns| columns[key]);
            let column = signed.unsigned_abs() as usize - 1;
            let direction = direction.unwrap_or(if signed < 0 {
                SortOrder::Descend
            } else {
                SortOrder::Ascend
            });
            let comparison = compare(
                read(column * rows).into(),
                read(column * rows + 1).into(),
                direction,
                missing,
            );
            if comparison != std::cmp::Ordering::Equal {
                if comparison == std::cmp::Ordering::Greater {
                    order.swap(0, 1);
                }
                break;
            }
        }
        check(cancel)?;
        return Ok(order);
    }
    let mut scratch = if rows > RUN {
        allocate::<u32>(rows, cancel)?
    } else {
        Vec::new()
    };
    let parallel = rows >= PAR_ROWS && rayon::current_num_threads() > 1;
    for key in (0..keys).rev() {
        check(cancel)?;
        let signed = columns.map_or((key + 1) as i32, |columns| columns[key]);
        let column = signed.unsigned_abs() as usize - 1;
        let direction = direction.unwrap_or(if signed < 0 {
            SortOrder::Descend
        } else {
            SortOrder::Ascend
        });
        // The first pass starts in original row order. Later passes preserve
        // the prior complete-key order for ties, yielding lexicographic order.
        // sort_lane uses stable runs and stable merges, so equal scalar keys
        // must remain equal: no rank allocation or index tie-break is needed.
        let cmp = |a: u32, b: u32| {
            compare(
                read(column * rows + a as usize).into(),
                read(column * rows + b as usize).into(),
                direction,
                missing,
            )
        };
        sort_lane(&mut order, &mut scratch, parallel, &cmp, cancel)?;
    }
    check(cancel)?;
    Ok(order) // Merge scratch is dropped before allocating B or I.
}

impl Array {
    /// Row-stable sorting. Signed columns are one-based and priority-ordered;
    /// None means all columns. A global direction overrides every column sign.
    /// Numeric/logical storage and shape are preserved; I is an optional Rx1
    /// double permutation. Explicit [] columns are unsupported in this subset.
    pub fn sort_rows(
        &self,
        columns: Option<&[i32]>,
        direction: Option<SortOrder>,
        missing: MissingPlacement,
        want_indices: bool,
        cancel: &AtomicBool,
    ) -> Result<(Self, Option<Matrix>)> {
        check(cancel)?;
        if !matches!(self, Self::Typed(_)) {
            self.require_numeric_array()?;
        }
        if self.is_complex() {
            return Err("Complex row sorting is not implemented".into());
        }
        let (rows, cols) = (self.rows(), self.cols());
        let keys = validate(rows, cols, columns, cancel)?;
        if rows <= 1 || keys == 0 {
            let indices = if want_indices {
                Some(Matrix::from_column_vec(
                    rows,
                    1,
                    build(rows, cancel, |row| Ok(row as f64 + 1.))?,
                )?)
            } else {
                None
            };
            check(cancel)?;
            return Ok((self.clone(), indices));
        }
        let order = match self {
            Self::Typed(a) => permutation(rows, keys, columns, direction, missing, cancel, |i| {
                typed_key(a, i)
            })?,
            Self::Character(_)
            | Self::Cell(_)
            | Self::Graphics(_)
            | Self::Struct(_)
            | Self::Handle(_) => {
                return Err("Row sorting of character, cell, struct, function handle or graphics handle arrays is not implemented".into());
            }
            Self::Complex(_) => return Err("Complex row sorting is not implemented".into()),
            Self::Numeric(a) => {
                permutation(rows, keys, columns, direction, missing, cancel, |i| {
                    a.data()[i]
                })?
            }
            Self::Logical(a) => {
                permutation(rows, keys, columns, direction, missing, cancel, |i| {
                    f64::from(a.data()[i])
                })?
            }
        };
        let output = match self {
            Self::Typed(a) => a
                .select_with_cancel(
                    rows,
                    cols,
                    &build(a.len(), cancel, |i| {
                        Ok(i / rows * rows + order[i % rows] as usize)
                    })?,
                    cancel,
                )?
                .into(),
            Self::Character(_)
            | Self::Cell(_)
            | Self::Graphics(_)
            | Self::Struct(_)
            | Self::Handle(_) => {
                return Err("Row sorting of character, cell, struct, function handle or graphics handle arrays is not implemented".into());
            }
            Self::Complex(_) => return Err("Complex row sorting is not implemented".into()),
            Self::Numeric(a) => Matrix::from_column_vec(
                rows,
                cols,
                build(a.len(), cancel, |i| {
                    Ok(a.data()[i / rows * rows + order[i % rows] as usize])
                })?,
            )?
            .into(),
            Self::Logical(a) => LogicalMatrix::from_column_vec(
                rows,
                cols,
                build(a.len(), cancel, |i| {
                    Ok(a.data()[i / rows * rows + order[i % rows] as usize])
                })?,
                cancel,
            )?
            .into(),
        };
        let indices = if want_indices {
            Some(Matrix::from_column_vec(
                rows,
                1,
                build(rows, cancel, |i| Ok(f64::from(order[i]) + 1.))?,
            )?)
        } else {
            None
        };
        check(cancel)?;
        Ok((output, indices))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_key_work_is_rejected_before_permutation_allocation() {
        let cancel = AtomicBool::new(false);
        assert!(
            validate(MAX_ELEMENTS, 1, Some(&[1, 1]), &cancel)
                .unwrap_err()
                .contains("work budget")
        );
        assert_eq!(validate(MAX_ELEMENTS, 1, None, &cancel).unwrap(), 1);
        assert_eq!(
            validate(0, MAX_ELEMENTS, None, &cancel).unwrap(),
            MAX_ELEMENTS
        );
    }

    #[test]
    fn cancelling_during_scalar_pass_discards_permutation_and_retry_is_stable() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let rows = 16385;
        let cancel = AtomicBool::new(false);
        let reads = AtomicUsize::new(0);
        let read = |i: usize| ((i * 37) % 101) as f64;
        let expected =
            permutation(rows, 3, None, None, MissingPlacement::Auto, &cancel, read).unwrap();
        let result = permutation(rows, 3, None, None, MissingPlacement::Auto, &cancel, |i| {
            if reads.fetch_add(1, Ordering::Relaxed) + 1 == 8192 {
                cancel.store(true, Ordering::Relaxed);
            }
            read(i)
        });
        assert!(cancel.load(Ordering::Relaxed));
        assert!(reads.load(Ordering::Relaxed) >= 8192);
        assert!(result.unwrap_err().contains("cancelled"));
        cancel.store(false, Ordering::Relaxed);
        let retry =
            permutation(rows, 3, None, None, MissingPlacement::Auto, &cancel, read).unwrap();
        assert_eq!(retry, expected);
    }

    #[test]
    fn two_rows_stop_at_first_decisive_key_and_keep_complete_ties() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let cancel = AtomicBool::new(false);
        let reads = AtomicUsize::new(0);
        let order = permutation(2, 8193, None, None, MissingPlacement::Auto, &cancel, |i| {
            reads.fetch_add(1, Ordering::Relaxed);
            assert!(
                i < 2,
                "A later key must not be read after a decisive first key"
            );
            if i == 0 { 2. } else { 1. }
        })
        .unwrap();
        assert_eq!(order, [1, 0]);
        assert_eq!(reads.load(Ordering::Relaxed), 2);
        let order = permutation(2, 8193, None, None, MissingPlacement::Auto, &cancel, |i| {
            if i / 2 == 8192 && i.is_multiple_of(2) {
                1.
            } else {
                0.
            }
        })
        .unwrap();
        assert_eq!(order, [1, 0]);
        assert_eq!(
            permutation(2, 8193, None, None, MissingPlacement::Auto, &cancel, |_| {
                -0.
            })
            .unwrap(),
            [0, 1]
        );
    }

    #[test]
    fn two_rows_cancel_within_a_key_chunk_and_after_a_decisive_read() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        for decisive in [false, true] {
            let cancel = AtomicBool::new(false);
            let reads = AtomicUsize::new(0);
            let result = permutation(2, 8193, None, None, MissingPlacement::Auto, &cancel, |i| {
                if reads.fetch_add(1, Ordering::Relaxed) == 0 {
                    cancel.store(true, Ordering::Relaxed);
                }
                if decisive && i == 0 { 1. } else { 0. }
            });
            assert!(result.unwrap_err().contains("cancelled"));
            assert!(reads.load(Ordering::Relaxed) <= RUN);
            cancel.store(false, Ordering::Relaxed);
            assert_eq!(
                permutation(2, 8193, None, None, MissingPlacement::Auto, &cancel, |_| 0.).unwrap(),
                [0, 1]
            );
        }
    }
}
