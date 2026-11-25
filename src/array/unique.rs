//! Borrowed real/logical duplicate grouping with optional materialized outputs.
use super::sort::{Key, allocate, build, check, compare_keys as compare, sort_lane, typed_key};
use crate::{
    Array, LogicalMatrix, Matrix, Result,
    array::{MissingPlacement, SortOrder},
};
use std::sync::atomic::AtomicBool;

const CHUNK: usize = 8192;
// Initial measurement candidate inherited from the existing indirect sorter.
// No new pool: small calls stay serial; root must measure complete unique calls.
const PAR_ELEMENTS: usize = 512 * 1024;
const MAX_PRIVATE_BYTES: usize = 512 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UniqueOrder {
    Sorted,
    Stable,
}

#[derive(Debug)]
pub struct UniqueResult {
    pub values: Option<Array>,
    pub first_indices: Option<Matrix>,
    pub group_indices: Option<Matrix>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Initialize,
    Sort,
    Group,
    Reorder,
    Output,
}

fn checkpoint(cancel: &AtomicBool, phase: Phase) -> Result<()> {
    #[cfg(test)]
    tests::probe(cancel, phase);
    let _ = phase;
    check(cancel)
}

fn preflight(rows: usize, cols: usize, numeric: bool, outputs: [bool; 3]) -> Result<usize> {
    let n = Matrix::checked_len(rows, cols)?;
    // Data-buffer ledger, excluding borrowed input and allocator metadata:
    // indices 4n + scratch 4n + optional group map 4n; bounded stable runs
    // may additionally reserve <=4n in aggregate across the existing pool.
    // Scratch is dropped before output allocation. Retained indices keep their
    // original capacity n even after compaction. No shrink/copy is hidden here.
    if !numeric {
        // Logical grouping holds only two stack indices, no n-sized permutation,
        // scratch or group map. C/ia have at most min(n,2) elements.
        let small = n.min(2) * (usize::from(outputs[0]) + 8 * usize::from(outputs[1]));
        if n.checked_mul(8 * usize::from(outputs[2]))
            .and_then(|b| b.checked_add(small))
            .is_none_or(|b| b > MAX_PRIVATE_BYTES)
        {
            return Err("unique exceeds its 512 MiB private data-buffer budget".into());
        }
        return Ok(n);
    }
    let sorting = 12 + 4 * usize::from(outputs[2]);
    let final_buffers = 4
        + 4 * usize::from(outputs[2])
        + 8 * usize::from(outputs[0])
        + 8 * usize::from(outputs[1])
        + 8 * usize::from(outputs[2]);
    if outputs.iter().any(|&x| x)
        && n.checked_mul(sorting.max(final_buffers))
            .is_none_or(|b| b > MAX_PRIVATE_BYTES)
    {
        return Err("unique exceeds its 512 MiB private data-buffer budget".into());
    }
    Ok(n)
}

struct Groups {
    first: Vec<u32>,
    ids: Option<Vec<u32>>,
}

fn reorder_stable_last(
    first_occ: &mut [u32],
    last_occ: &[u32],
    order_idx: &mut [u32],
    merge_scratch: &mut [u32],
    ids: &mut Option<Vec<u32>>,
    parallel: bool,
    cancel: &AtomicBool,
) -> Result<()> {
    for (i, slot) in order_idx.iter_mut().enumerate() {
        if i.is_multiple_of(CHUNK) {
            checkpoint(cancel, Phase::Reorder)?;
        }
        *slot = i as u32;
    }
    sort_lane(
        order_idx,
        merge_scratch,
        parallel,
        &|a, b| first_occ[a as usize].cmp(&first_occ[b as usize]),
        cancel,
    )?;
    for (rank, &old_group) in order_idx.iter().enumerate() {
        if rank.is_multiple_of(CHUNK) {
            checkpoint(cancel, Phase::Reorder)?;
        }
        first_occ[rank] = last_occ[old_group as usize];
    }
    if let Some(ids) = ids {
        for (rank, &old_group) in order_idx.iter().enumerate() {
            if rank.is_multiple_of(CHUNK) {
                checkpoint(cancel, Phase::Reorder)?;
            }
            merge_scratch[old_group as usize] = rank as u32;
        }
        for chunk in ids.chunks_mut(CHUNK) {
            checkpoint(cancel, Phase::Reorder)?;
            for id in chunk {
                *id = merge_scratch[*id as usize];
            }
        }
    }
    Ok(())
}

fn groups_by(
    n: usize,
    order: UniqueOrder,
    last: bool,
    want_ids: bool,
    cancel: &AtomicBool,
    cmp_items: &(dyn Fn(usize, usize) -> std::cmp::Ordering + Sync),
    eq_items: impl Fn(usize, usize) -> bool,
) -> Result<Groups> {
    checkpoint(cancel, Phase::Initialize)?;
    let mut first = allocate::<u32>(n, cancel)?;
    for (chunk, values) in first.chunks_mut(CHUNK).enumerate() {
        checkpoint(cancel, Phase::Initialize)?;
        for (at, value) in values.iter_mut().enumerate() {
            *value = (chunk * CHUNK + at) as u32;
        }
    }
    let mut scratch = allocate::<u32>(n, cancel)?;
    let parallel = n >= PAR_ELEMENTS && rayon::current_num_threads() > 1;
    checkpoint(cancel, Phase::Sort)?;
    sort_lane(
        &mut first,
        &mut scratch,
        parallel,
        &|a, b| cmp_items(a as usize, b as usize),
        cancel,
    )?;
    checkpoint(cancel, Phase::Sort)?;
    let mut ids = if want_ids {
        Some(allocate::<u32>(n, cancel)?)
    } else {
        None
    };
    let mut count = 0;
    let track_last_in_stable = last && order == UniqueOrder::Stable;
    for at in 0..n {
        if at.is_multiple_of(CHUNK) {
            checkpoint(cancel, Phase::Group)?;
        }
        let index = first[at];
        // IEEE numeric equality merges signed zero but deliberately never NaN.
        // Stable sorting retains the first input index and its exact value bits;
        // 'last' updates the representative within each equal group.
        if count == 0 || !eq_items(first[count - 1] as usize, index as usize) {
            first[count] = index;
            if track_last_in_stable {
                scratch[count] = index;
            }
            count += 1;
        } else if track_last_in_stable {
            scratch[count - 1] = index;
        } else if last {
            first[count - 1] = index;
        }
        if let Some(ids) = &mut ids {
            ids[index as usize] = (count - 1) as u32;
        }
    }
    checkpoint(cancel, Phase::Group)?;
    if order == UniqueOrder::Stable {
        checkpoint(cancel, Phase::Reorder)?;
        if last {
            if count <= n / 2 {
                let (first_occ, rest_first) = first.split_at_mut(count);
                let (last_occ, rest_scratch) = scratch.split_at_mut(count);
                reorder_stable_last(
                    first_occ,
                    last_occ,
                    &mut rest_first[..count],
                    &mut rest_scratch[..count],
                    &mut ids,
                    parallel && count >= PAR_ELEMENTS,
                    cancel,
                )?;
            } else {
                let mut order_idx = allocate::<u32>(count, cancel)?;
                let mut merge_scratch = allocate::<u32>(count, cancel)?;
                reorder_stable_last(
                    &mut first[..count],
                    &scratch[..count],
                    &mut order_idx,
                    &mut merge_scratch,
                    &mut ids,
                    parallel && count >= PAR_ELEMENTS,
                    cancel,
                )?;
            }
        } else {
            first.truncate(count);
            sort_lane(
                &mut first,
                &mut scratch[..count],
                parallel && count >= PAR_ELEMENTS,
                &|a, b| a.cmp(&b),
                cancel,
            )?;
            if let Some(ids) = &mut ids {
                // The old group ID at each representative gives an inverse map.
                // Reuse dead merge scratch rather than allocate another n-vector.
                for (rank, &index) in first.iter().enumerate() {
                    if rank.is_multiple_of(CHUNK) {
                        checkpoint(cancel, Phase::Reorder)?;
                    }
                    scratch[ids[index as usize] as usize] = rank as u32;
                }
                for chunk in ids.chunks_mut(CHUNK) {
                    checkpoint(cancel, Phase::Reorder)?;
                    for id in chunk {
                        *id = scratch[*id as usize];
                    }
                }
            }
        }
        checkpoint(cancel, Phase::Reorder)?;
    }
    first.truncate(count);
    drop(scratch);
    check(cancel)?;
    Ok(Groups { first, ids })
}

fn output<T: Default + Clone>(
    n: usize,
    cancel: &AtomicBool,
    read: impl Fn(usize) -> T,
) -> Result<Vec<T>> {
    checkpoint(cancel, Phase::Output)?;
    let mut data = allocate::<T>(n, cancel)?;
    for (chunk, values) in data.chunks_mut(CHUNK).enumerate() {
        checkpoint(cancel, Phase::Output)?;
        for (at, value) in values.iter_mut().enumerate() {
            *value = read(chunk * CHUNK + at);
        }
    }
    checkpoint(cancel, Phase::Output)?;
    Ok(data)
}

fn logical_unique(
    input: &LogicalMatrix,
    order: UniqueOrder,
    last: bool,
    outputs: [bool; 3],
    cancel: &AtomicBool,
) -> Result<UniqueResult> {
    let mut first = [usize::MAX; 2];
    // At most one scan, often just the first two values. The first occurrence
    // of each boolean determines all groups; comparison sorting is unnecessary.
    for (at, &value) in input.data().iter().enumerate() {
        if at.is_multiple_of(CHUNK) {
            checkpoint(cancel, Phase::Group)?;
        }
        first[usize::from(value)] = first[usize::from(value)].min(at);
        if first.iter().all(|&i| i != usize::MAX) {
            break;
        }
    }
    let picked_indices = if last {
        let mut last_idx = [usize::MAX; 2];
        for (rev_at, &value) in input.data().iter().rev().enumerate() {
            if rev_at.is_multiple_of(CHUNK) {
                checkpoint(cancel, Phase::Group)?;
            }
            let slot = &mut last_idx[usize::from(value)];
            if *slot == usize::MAX {
                *slot = input.len() - 1 - rev_at;
            }
            if last_idx.iter().all(|&i| i != usize::MAX) {
                break;
            }
        }
        last_idx
    } else {
        first
    };
    check(cancel)?;
    let mut representatives = [0usize; 2];
    let mut count = 0;
    for &index in &picked_indices {
        if index != usize::MAX {
            representatives[count] = index;
            count += 1;
        }
    }
    if count == 2 && order == UniqueOrder::Stable && first[0] > first[1] {
        representatives.swap(0, 1);
    }
    let (rows, cols) = if input.rows() == 1 {
        (1, count)
    } else {
        (count, 1)
    };
    let values = if outputs[0] {
        Some(
            LogicalMatrix::from_column_vec(
                rows,
                cols,
                output(count, cancel, |i| input.data()[representatives[i]])?,
                cancel,
            )?
            .into(),
        )
    } else {
        None
    };
    let first_indices = if outputs[1] {
        Some(Matrix::from_column_vec(
            count,
            1,
            output(count, cancel, |i| representatives[i] as f64 + 1.)?,
        )?)
    } else {
        None
    };
    let group_indices = if outputs[2] {
        let mut ranks = [0.; 2];
        for (rank, &index) in representatives[..count].iter().enumerate() {
            ranks[usize::from(input.data()[index])] = rank as f64 + 1.;
        }
        checkpoint(cancel, Phase::Output)?;
        // Reuse the existing bounded builder and caller pool, including its
        // initial 512K candidate threshold; no grouping scratch is allocated.
        let indices = build(input.len(), cancel, |i| {
            Ok(ranks[usize::from(input.data()[i])])
        })?;
        checkpoint(cancel, Phase::Output)?;
        Some(Matrix::from_column_vec(input.len(), 1, indices)?)
    } else {
        None
    };
    check(cancel)?;
    Ok(UniqueResult {
        values,
        first_indices,
        group_indices,
    })
}

impl Array {
    /// Unique real/logical values, first one-based indices, and reconstruction
    /// indices. Input is flattened column-major; only requested results allocate.
    /// C is 1xK iff input.rows()==1, otherwise Kx1; ia/ic are columns.
    /// First signed-zero bits and distinct NaN payloads are retained. Empty/zero
    /// bit nuances are explicit MatRust choices pending authentic reference checks.
    pub fn unique_values(
        &self,
        order: UniqueOrder,
        outputs: [bool; 3],
        cancel: &AtomicBool,
    ) -> Result<UniqueResult> {
        self.unique_with_options(order, false, false, outputs, cancel)
    }

    /// Unique values or unique rows with optional 'stable'/'sorted' and 'first'/'last'.
    pub fn unique_with_options(
        &self,
        order: UniqueOrder,
        rows_mode: bool,
        last: bool,
        outputs: [bool; 3],
        cancel: &AtomicBool,
    ) -> Result<UniqueResult> {
        check(cancel)?;
        if !matches!(self, Self::Numeric(_) | Self::Logical(_) | Self::Typed(_)) {
            return Err("unique supports real double or logical arrays only".into());
        }
        if rows_mode && self.is_nd() {
            return Err("unique: 'rows' is not supported for N-D arrays".into());
        }
        let n = preflight(
            self.rows(),
            self.cols(),
            rows_mode || !matches!(self, Self::Logical(_)),
            outputs,
        )?;
        if !outputs.iter().any(|&x| x) {
            return Ok(UniqueResult {
                values: None,
                first_indices: None,
                group_indices: None,
            });
        }
        if !rows_mode {
            if let Self::Logical(input) = self {
                return logical_unique(input, order, last, outputs, cancel);
            }
            let key_at = |i: usize| -> Key {
                match self {
                    Self::Numeric(a) => a.data()[i].into(),
                    Self::Typed(a) => typed_key(a, i),
                    _ => unreachable!(),
                }
            };
            let grouped = groups_by(
                n,
                order,
                last,
                outputs[2],
                cancel,
                &|a, b| {
                    compare(
                        key_at(a),
                        key_at(b),
                        SortOrder::Ascend,
                        MissingPlacement::Last,
                    )
                },
                |a, b| key_at(a) == key_at(b),
            )?;
            let k = grouped.first.len();
            let (rows, cols) = if self.rows() == 1 { (1, k) } else { (k, 1) };
            let values = if outputs[0] {
                Some(match self {
                    Self::Typed(a) => a
                        .select_with_cancel(
                            rows,
                            cols,
                            &build(k, cancel, |i| Ok(grouped.first[i] as usize))?,
                            cancel,
                        )?
                        .into(),
                    Self::Numeric(a) => Matrix::from_column_vec(
                        rows,
                        cols,
                        output(k, cancel, |i| a.data()[grouped.first[i] as usize])?,
                    )?
                    .into(),
                    _ => unreachable!(),
                })
            } else {
                None
            };
            let first_indices = if outputs[1] {
                Some(Matrix::from_column_vec(
                    k,
                    1,
                    output(k, cancel, |i| f64::from(grouped.first[i]) + 1.)?,
                )?)
            } else {
                None
            };
            let group_indices = if outputs[2] {
                let ids = grouped.ids.as_ref().unwrap();
                Some(Matrix::from_column_vec(
                    n,
                    1,
                    output(n, cancel, |i| f64::from(ids[i]) + 1.)?,
                )?)
            } else {
                None
            };
            check(cancel)?;
            return Ok(UniqueResult {
                values,
                first_indices,
                group_indices,
            });
        }

        let (r, c) = (self.rows(), self.cols());
        let key_at = |row: usize, col: usize| -> Key {
            let idx = col * r + row;
            match self {
                Self::Numeric(a) => a.data()[idx].into(),
                Self::Logical(a) => f64::from(a.data()[idx]).into(),
                Self::Typed(a) => typed_key(a, idx),
                _ => unreachable!(),
            }
        };
        let grouped = if r == 0 {
            Groups {
                first: Vec::new(),
                ids: outputs[2].then(Vec::new),
            }
        } else if c == 0 {
            let picked = if last { r - 1 } else { 0 } as u32;
            Groups {
                first: vec![picked],
                ids: outputs[2].then(|| vec![0; r]),
            }
        } else {
            groups_by(
                r,
                order,
                last,
                outputs[2],
                cancel,
                &|a, b| {
                    for col in 0..c {
                        let ord = compare(
                            key_at(a, col),
                            key_at(b, col),
                            SortOrder::Ascend,
                            MissingPlacement::Last,
                        );
                        if ord != std::cmp::Ordering::Equal {
                            return ord;
                        }
                    }
                    std::cmp::Ordering::Equal
                },
                |a, b| (0..c).all(|col| key_at(a, col) == key_at(b, col)),
            )?
        };
        let k = grouped.first.len();
        let values = if outputs[0] {
            Some(match self {
                Self::Typed(a) => a
                    .select_with_cancel(
                        k,
                        c,
                        &build(k * c, cancel, |idx| {
                            let col = idx / k;
                            let row = grouped.first[idx % k] as usize;
                            Ok(col * r + row)
                        })?,
                        cancel,
                    )?
                    .into(),
                Self::Numeric(a) => Matrix::from_column_vec(
                    k,
                    c,
                    output(k * c, cancel, |idx| {
                        let col = idx / k;
                        let row = grouped.first[idx % k] as usize;
                        a.data()[col * r + row]
                    })?,
                )?
                .into(),
                Self::Logical(a) => LogicalMatrix::from_column_vec(
                    k,
                    c,
                    output(k * c, cancel, |idx| {
                        let col = idx / k;
                        let row = grouped.first[idx % k] as usize;
                        a.data()[col * r + row]
                    })?,
                    cancel,
                )?
                .into(),
                _ => unreachable!(),
            })
        } else {
            None
        };
        let first_indices = if outputs[1] {
            Some(Matrix::from_column_vec(
                k,
                1,
                output(k, cancel, |i| f64::from(grouped.first[i]) + 1.)?,
            )?)
        } else {
            None
        };
        let group_indices = if outputs[2] {
            let ids = grouped.ids.as_ref().unwrap();
            Some(Matrix::from_column_vec(
                r,
                1,
                output(r, cancel, |i| f64::from(ids[i]) + 1.)?,
            )?)
        } else {
            None
        };
        check(cancel)?;
        Ok(UniqueResult {
            values,
            first_indices,
            group_indices,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, sync::atomic::Ordering};
    thread_local! { static CANCEL: Cell<Option<(Phase, usize)>> = const { Cell::new(None) }; }
    pub(super) fn probe(cancel: &AtomicBool, phase: Phase) {
        CANCEL.with(|slot| {
            if let Some((selected, remaining)) = slot.get()
                && selected == phase
            {
                if remaining == 0 {
                    slot.set(None);
                    cancel.store(true, Ordering::Relaxed);
                } else {
                    slot.set(Some((selected, remaining - 1)));
                }
            }
        });
    }
    #[test]
    fn cancellation_in_each_private_phase_preserves_input_and_allows_retry() {
        let a: Array = Matrix::from_column_vec(
            1,
            20001,
            (0..20001).map(|i| ((i * 37) % 997) as f64).collect(),
        )
        .unwrap()
        .into();
        let before = a.clone();
        let cancel = AtomicBool::new(false);
        for phase in [
            Phase::Initialize,
            Phase::Sort,
            Phase::Group,
            Phase::Reorder,
            Phase::Output,
        ] {
            CANCEL.with(|slot| slot.set(Some((phase, 1))));
            assert!(
                a.unique_values(UniqueOrder::Stable, [true; 3], &cancel)
                    .unwrap_err()
                    .contains("cancelled")
            );
            CANCEL.with(|slot| slot.set(None));
            assert!(a.same_snapshot(&before));
            cancel.store(false, Ordering::Relaxed);
            let retry = a
                .unique_values(UniqueOrder::Stable, [true; 3], &cancel)
                .unwrap();
            assert_eq!(retry.values.unwrap().len(), 997);
        }
    }
    #[test]
    fn masks_skip_unrequested_outputs_and_all_ignored_work() {
        let a: Array = Matrix::from_column_slice(1, 4, &[3., 1., 3., 2.])
            .unwrap()
            .into();
        let cancel = AtomicBool::new(false);
        for bits in 0..8 {
            let mask = [bits & 1 != 0, bits & 2 != 0, bits & 4 != 0];
            if bits == 0 {
                CANCEL.with(|slot| slot.set(Some((Phase::Initialize, 0))));
            }
            let result = a.unique_values(UniqueOrder::Stable, mask, &cancel).unwrap();
            assert_eq!(result.values.is_some(), mask[0]);
            assert_eq!(result.first_indices.is_some(), mask[1]);
            assert_eq!(result.group_indices.is_some(), mask[2]);
            assert!(!cancel.load(Ordering::Relaxed));
            CANCEL.with(|slot| slot.set(None));
        }
        // 16 Mi values with every output fit the fixed 512 MiB private budget
        // (the native element limit is larger; the budget binds first there).
        assert!(preflight(16 * 1024 * 1024, 1, true, [true; 3]).is_ok());
        assert!(preflight(48 * 1024 * 1024, 1, true, [true; 3]).is_err());
        assert!(preflight(crate::matrix::MAX_ELEMENTS + 1, 1, true, [false; 3]).is_err());
    }
    #[test]
    fn logical_scan_and_final_map_cancellation_do_not_publish_partial_results() {
        let cancel = AtomicBool::new(false);
        let a: Array = LogicalMatrix::filled(1, 20001, false, &cancel)
            .unwrap()
            .into();
        let before = a.clone();
        for (phase, skip) in [(Phase::Group, 1), (Phase::Output, 1)] {
            CANCEL.with(|slot| slot.set(Some((phase, skip))));
            assert!(
                a.unique_values(UniqueOrder::Stable, [false, false, true], &cancel)
                    .unwrap_err()
                    .contains("cancelled")
            );
            CANCEL.with(|slot| slot.set(None));
            cancel.store(false, Ordering::Relaxed);
            assert!(a.same_snapshot(&before));
        }
        assert!(
            a.unique_values(UniqueOrder::Sorted, [false, false, true], &cancel)
                .unwrap()
                .group_indices
                .unwrap()
                .data()
                .iter()
                .all(|&v| v == 1.)
        );
    }
}
