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

fn groups(
    n: usize,
    order: UniqueOrder,
    want_ids: bool,
    cancel: &AtomicBool,
    read: impl Fn(usize) -> Key + Sync,
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
        &|a, b| {
            compare(
                read(a as usize),
                read(b as usize),
                SortOrder::Ascend,
                MissingPlacement::Last,
            )
        },
        cancel,
    )?;
    checkpoint(cancel, Phase::Sort)?;
    let mut ids = if want_ids {
        Some(allocate::<u32>(n, cancel)?)
    } else {
        None
    };
    let mut count = 0;
    for at in 0..n {
        if at.is_multiple_of(CHUNK) {
            checkpoint(cancel, Phase::Group)?;
        }
        let index = first[at];
        // IEEE numeric equality merges signed zero but deliberately never NaN.
        // Stable sorting retains the first input index and its exact value bits.
        if count == 0 || read(first[count - 1] as usize) != read(index as usize) {
            first[count] = index;
            count += 1;
        }
        if let Some(ids) = &mut ids {
            ids[index as usize] = (count - 1) as u32;
        }
    }
    checkpoint(cancel, Phase::Group)?;
    first.truncate(count);
    if order == UniqueOrder::Stable {
        checkpoint(cancel, Phase::Reorder)?;
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
        checkpoint(cancel, Phase::Reorder)?;
    }
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
    check(cancel)?;
    let mut representatives = [0usize; 2];
    let mut count = 0;
    for &index in &first {
        if index != usize::MAX {
            representatives[count] = index;
            count += 1;
        }
    }
    if count == 2 && order == UniqueOrder::Stable && representatives[0] > representatives[1] {
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
        check(cancel)?;
        if !matches!(self, Self::Numeric(_) | Self::Logical(_) | Self::Typed(_)) {
            return Err("unique supports real double or logical arrays only".into());
        }
        let n = preflight(
            self.rows(),
            self.cols(),
            !matches!(self, Self::Logical(_)),
            outputs,
        )?;
        if !outputs.iter().any(|&x| x) {
            return Ok(UniqueResult {
                values: None,
                first_indices: None,
                group_indices: None,
            });
        }
        if let Self::Logical(input) = self {
            return logical_unique(input, order, outputs, cancel);
        }
        let grouped = match self {
            Self::Numeric(a) => groups(n, order, outputs[2], cancel, |i| a.data()[i].into())?,
            Self::Typed(a) => groups(n, order, outputs[2], cancel, |i| typed_key(a, i))?,
            _ => unreachable!(),
        };
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
        assert!(preflight(crate::matrix::MAX_ELEMENTS, 1, true, [true; 3]).is_ok());
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
