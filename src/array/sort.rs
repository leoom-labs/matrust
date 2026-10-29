//! Stable real/logical 2-D sorting with bounded cancellable runs.
use crate::{Array, LogicalMatrix, Matrix, Result, matrix::MAX_ELEMENTS};
use rayon::prelude::*;
use std::{
    cmp::Ordering,
    sync::atomic::{AtomicBool, Ordering as AtomicOrdering},
};

const RUN: usize = 8192;
// Research candidate only: release measurements must accept or replace this.
const PAR_ELEMENTS: usize = 512 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortOrder {
    Ascend,
    Descend,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissingPlacement {
    Auto,
    First,
    Last,
}

pub(super) fn check(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(AtomicOrdering::Relaxed) {
        Err("Calculation cancelled".into())
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Run,
    Merge,
    Lane,
}
fn progress(phase: Phase, cancel: &AtomicBool) -> Result<()> {
    #[cfg(test)]
    cancellation_probe::completed(phase, cancel);
    #[cfg(not(test))]
    let _ = phase;
    check(cancel)
}

pub(super) fn allocate<T: Default + Clone>(len: usize, cancel: &AtomicBool) -> Result<Vec<T>> {
    check(cancel)?;
    let mut data = Vec::new();
    data.try_reserve_exact(len)
        .map_err(|_| "Cannot allocate sort buffer")?;
    while data.len() < len {
        check(cancel)?;
        data.resize((data.len() + RUN).min(len), T::default());
    }
    Ok(data)
}

pub(super) fn build<T: Default + Clone + Send>(
    len: usize,
    cancel: &AtomicBool,
    value: impl Fn(usize) -> Result<T> + Sync,
) -> Result<Vec<T>> {
    let mut result = allocate(len, cancel)?;
    let fill = |(chunk, output): (usize, &mut [T])| -> Result<()> {
        check(cancel)?;
        for (index, item) in output.iter_mut().enumerate() {
            *item = value(chunk * RUN + index)?;
        }
        check(cancel)
    };
    if len >= PAR_ELEMENTS && rayon::current_num_threads() > 1 {
        result.par_chunks_mut(RUN).enumerate().try_for_each(fill)?;
    } else {
        result.chunks_mut(RUN).enumerate().try_for_each(fill)?;
    }
    check(cancel)?;
    Ok(result)
}

pub(super) fn compare(a: f64, b: f64, order: SortOrder, missing: MissingPlacement) -> Ordering {
    let first = missing == MissingPlacement::First
        || (missing == MissingPlacement::Auto && order == SortOrder::Descend);
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => {
            if first {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        }
        (false, true) => {
            if first {
                Ordering::Greater
            } else {
                Ordering::Less
            }
        }
        (false, false) => {
            // Unlike total_cmp, numeric equality treats -0 and +0 as a tie.
            let cmp = a.partial_cmp(&b).unwrap();
            if order == SortOrder::Descend {
                cmp.reverse()
            } else {
                cmp
            }
        }
    }
}

/// Comparison keys retain integer precision; only single/double use Real.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Key {
    Integer(i128),
    Real(f64),
}
impl From<f64> for Key {
    fn from(value: f64) -> Self {
        Self::Real(value)
    }
}
pub(super) fn typed_key(a: &crate::TypedArray, i: usize) -> Key {
    if a.is_integer() {
        Key::Integer(a.int(i))
    } else {
        Key::Real(a.real(i))
    }
}
pub(super) fn compare_keys(
    a: Key,
    b: Key,
    order: SortOrder,
    missing: MissingPlacement,
) -> Ordering {
    match (a, b) {
        (Key::Integer(a), Key::Integer(b)) => {
            let result = a.cmp(&b);
            if order == SortOrder::Descend {
                result.reverse()
            } else {
                result
            }
        }
        (Key::Real(a), Key::Real(b)) => compare(a, b, order, missing),
        _ => unreachable!("sort keys belong to the same array class"),
    }
}

fn merge(
    source: &[u32],
    output: &mut [u32],
    width: usize,
    compare: &(impl Fn(u32, u32) -> Ordering + Sync),
    cancel: &AtomicBool,
) -> Result<()> {
    let middle = width.min(source.len());
    let (mut left, mut right) = (0, middle);
    for (offset, out) in output.iter_mut().enumerate() {
        if offset.is_multiple_of(RUN) {
            check(cancel)?;
        }
        if left < middle
            && (right == source.len() || compare(source[left], source[right]) != Ordering::Greater)
        {
            *out = source[left];
            left += 1;
        } else {
            *out = source[right];
            right += 1;
        }
    }
    progress(Phase::Merge, cancel)
}

fn merge_pass(
    source: &[u32],
    output: &mut [u32],
    width: usize,
    parallel: bool,
    compare: &(impl Fn(u32, u32) -> Ordering + Sync),
    cancel: &AtomicBool,
) -> Result<()> {
    let block = width * 2; // width <= MAX_ELEMENTS and MAX_ELEMENTS << usize::MAX.
    if parallel {
        source
            .par_chunks(block)
            .zip(output.par_chunks_mut(block))
            .try_for_each(|(a, b)| merge(a, b, width, compare, cancel))
    } else {
        source
            .chunks(block)
            .zip(output.chunks_mut(block))
            .try_for_each(|(a, b)| merge(a, b, width, compare, cancel))
    }
}

pub(super) fn sort_lane(
    indices: &mut [u32],
    scratch: &mut [u32],
    parallel: bool,
    compare: &(impl Fn(u32, u32) -> Ordering + Sync),
    cancel: &AtomicBool,
) -> Result<()> {
    let run = |values: &mut [u32]| -> Result<()> {
        check(cancel)?;
        // Stable bounded runs preserve equal-key input order without turning
        // every equal value into a distinct index key. This lets the standard
        // adaptive stable kernel exploit repeated or monotonic inputs.
        values.sort_by(|a, b| compare(*a, *b));
        progress(Phase::Run, cancel)
    };
    if parallel {
        indices.par_chunks_mut(RUN).try_for_each(run)?;
    } else {
        indices.chunks_mut(RUN).try_for_each(run)?;
    }
    let mut width = RUN;
    let mut original = true;
    while width < indices.len() {
        if original {
            merge_pass(indices, scratch, width, parallel, compare, cancel)?;
        } else {
            merge_pass(scratch, indices, width, parallel, compare, cancel)?;
        }
        original = !original;
        width *= 2;
    }
    if !original {
        for (dst, src) in indices.chunks_mut(RUN).zip(scratch.chunks(RUN)) {
            check(cancel)?;
            dst.copy_from_slice(src);
        }
    }
    progress(Phase::Lane, cancel)
}

impl Array {
    /// Sort each real/logical lane. An optional double index result is one-based
    /// along the selected dimension. Input allocation is immutable and shared.
    pub fn sort_dimension(
        &self,
        dim: usize,
        order: SortOrder,
        missing: MissingPlacement,
        want_indices: bool,
        cancel: &AtomicBool,
    ) -> Result<(Self, Option<Matrix>)> {
        check(cancel)?;
        if !matches!(self, Self::Typed(_)) {
            self.require_numeric_array()?;
        }
        if self.is_complex() {
            return Err("Complex sorting is not implemented".into());
        }
        if dim == 0 || dim > MAX_ELEMENTS {
            return Err(format!(
                "sort dimension must be a positive integer <= {MAX_ELEMENTS}"
            ));
        }
        let (rows, cols) = (self.rows(), self.cols());
        let len = Matrix::checked_len(rows, cols)?;
        let lane_len = if dim == 1 {
            rows
        } else if dim == 2 {
            cols
        } else {
            1
        };
        if len == 0 || lane_len <= 1 || dim > 2 {
            let indices = if want_indices {
                Some(Matrix::from_column_vec(
                    rows,
                    cols,
                    build(len, cancel, |_| Ok(1.))?,
                )?)
            } else {
                None
            };
            check(cancel)?;
            return Ok((self.clone(), indices));
        }
        let lanes = len / lane_len;
        // Permutation lanes are contiguous even for row sorts. Final gathering
        // writes the published result in column-major order without transposing.
        let source = |lane: usize, at: usize| {
            if dim == 1 {
                lane * rows + at
            } else {
                lane + at * rows
            }
        };
        let target = |index: usize| {
            if dim == 1 {
                index
            } else {
                (index % rows) * cols + index / rows
            }
        };
        let parallel = len >= PAR_ELEMENTS && rayon::current_num_threads() > 1;
        if let Self::Logical(logical) = self {
            let first = u8::from(order == SortOrder::Descend);
            let counts = build(lanes, cancel, |lane| {
                let mut count = 0usize;
                for at in 0..lane_len {
                    if at.is_multiple_of(RUN) {
                        check(cancel)?;
                    }
                    count += usize::from(logical.data()[source(lane, at)] == first);
                }
                Ok(count)
            })?;
            let output = build(len, cancel, |index| {
                let p = target(index);
                Ok(if p % lane_len < counts[p / lane_len] {
                    first
                } else {
                    first ^ 1
                })
            })?;
            let indices = if want_indices {
                let mut permutation = allocate::<u32>(len, cancel)?;
                let fill = |(lane, output): (usize, &mut [u32])| -> Result<()> {
                    let (mut a, mut b) = (0, counts[lane]);
                    for at in 0..lane_len {
                        if at.is_multiple_of(RUN) {
                            check(cancel)?;
                        }
                        let slot = if logical.data()[source(lane, at)] == first {
                            let p = a;
                            a += 1;
                            p
                        } else {
                            let p = b;
                            b += 1;
                            p
                        };
                        output[slot] = at as u32;
                    }
                    progress(Phase::Lane, cancel)
                };
                if parallel {
                    permutation
                        .par_chunks_mut(lane_len)
                        .enumerate()
                        .try_for_each(fill)?;
                } else {
                    permutation
                        .chunks_mut(lane_len)
                        .enumerate()
                        .try_for_each(fill)?;
                }
                Some(Matrix::from_column_vec(
                    rows,
                    cols,
                    build(len, cancel, |i| Ok(f64::from(permutation[target(i)]) + 1.))?,
                )?)
            } else {
                None
            };
            check(cancel)?;
            return Ok((
                LogicalMatrix::from_column_vec(rows, cols, output, cancel)?.into(),
                indices,
            ));
        }
        let key = |i| match self {
            Self::Numeric(a) => Key::Real(a.data()[i]),
            Self::Typed(a) => typed_key(a, i),
            _ => unreachable!(),
        };
        let mut permutation = build(len, cancel, |i| Ok((i % lane_len) as u32))?;
        // Two u32 buffers total <=128MiB. Avoid merge scratch for short lanes.
        let mut scratch = if lane_len > RUN {
            allocate::<u32>(len, cancel)?
        } else {
            Vec::new()
        };
        let lane_sort = |lane: usize, output: &mut [u32], scratch: &mut [u32]| {
            let cmp = |a: u32, b: u32| {
                compare_keys(
                    key(source(lane, a as usize)),
                    key(source(lane, b as usize)),
                    order,
                    missing,
                )
            };
            sort_lane(output, scratch, parallel && lanes == 1, &cmp, cancel)
        };
        if scratch.is_empty() {
            if parallel && lanes > 1 {
                permutation
                    .par_chunks_mut(lane_len)
                    .enumerate()
                    .try_for_each(|(lane, output)| lane_sort(lane, output, &mut []))?;
            } else {
                for (lane, output) in permutation.chunks_mut(lane_len).enumerate() {
                    lane_sort(lane, output, &mut [])?;
                }
            }
        } else if parallel && lanes > 1 {
            permutation
                .par_chunks_mut(lane_len)
                .zip(scratch.par_chunks_mut(lane_len))
                .enumerate()
                .try_for_each(|(lane, (output, scratch))| lane_sort(lane, output, scratch))?;
        } else {
            for (lane, (output, scratch)) in permutation
                .chunks_mut(lane_len)
                .zip(scratch.chunks_mut(lane_len))
                .enumerate()
            {
                lane_sort(lane, output, scratch)?;
            }
        }
        drop(scratch); // Merge scratch never coexists with published buffers.
        let position = |index| {
            let p = target(index);
            source(p / lane_len, permutation[p] as usize)
        };
        let values = match self {
            Self::Numeric(a) => Matrix::from_column_vec(
                rows,
                cols,
                build(len, cancel, |i| Ok(a.data()[position(i)]))?,
            )?
            .into(),
            Self::Typed(a) => a
                .select_with_cancel(
                    rows,
                    cols,
                    &build(len, cancel, |i| Ok(position(i)))?,
                    cancel,
                )?
                .into(),
            _ => unreachable!(),
        };
        let indices = if want_indices {
            Some(Matrix::from_column_vec(
                rows,
                cols,
                build(len, cancel, |i| Ok(f64::from(permutation[target(i)]) + 1.))?,
            )?)
        } else {
            None
        };
        check(cancel)?;
        Ok((values, indices))
    }
}

#[cfg(test)]
mod cancellation_probe {
    use super::*;
    use std::cell::Cell;
    thread_local! { static ACTIVE: Cell<Option<(Phase, usize)>> = const { Cell::new(None) }; }
    pub fn completed(phase: Phase, cancel: &AtomicBool) {
        ACTIVE.with(|probe| {
            if let Some((target, left)) = probe.get()
                && target == phase
            {
                if left == 1 {
                    probe.set(None);
                    cancel.store(true, AtomicOrdering::Relaxed);
                } else {
                    probe.set(Some((target, left - 1)));
                }
            }
        });
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            ACTIVE.with(|p| p.set(None));
        }
    }
    #[test]
    fn active_cancellation_after_runs_merges_and_lanes_never_publishes_or_mutates() {
        for (rows, cols, phase, after) in [
            (RUN * 4 + 3, 1, Phase::Run, 2),
            (RUN * 4 + 3, 1, Phase::Merge, 2),
            (37, 13, Phase::Lane, 3),
        ] {
            let input: Array = Matrix::from_column_vec(
                rows,
                cols,
                (0..rows * cols).map(|i| ((i * 37) % 101) as f64).collect(),
            )
            .unwrap()
            .into();
            let alias = input.clone();
            let cancel = AtomicBool::new(false);
            let expected = input
                .sort_dimension(1, SortOrder::Ascend, MissingPlacement::Auto, true, &cancel)
                .unwrap();
            ACTIVE.with(|p| p.set(Some((phase, after))));
            let _reset = Reset;
            let error = input
                .sort_dimension(1, SortOrder::Ascend, MissingPlacement::Auto, true, &cancel)
                .unwrap_err();
            assert!(error.contains("cancelled") && cancel.load(AtomicOrdering::Relaxed));
            assert!(ACTIVE.with(|p| p.get().is_none()));
            assert!(input.same_snapshot(&alias));
            cancel.store(false, AtomicOrdering::Relaxed);
            let actual = input
                .sort_dimension(1, SortOrder::Ascend, MissingPlacement::Auto, true, &cancel)
                .unwrap();
            assert_eq!(
                actual.0.as_numeric().unwrap().data(),
                expected.0.as_numeric().unwrap().data()
            );
            assert_eq!(actual.1.unwrap().data(), expected.1.unwrap().data());
        }
    }
    #[test]
    fn sortrows_active_run_merge_and_key_cancellation_retains_input_and_retry() {
        for (phase, after, columns) in [
            (Phase::Run, 2, None),
            (Phase::Merge, 1, None),
            (Phase::Lane, 1, Some(vec![1, 2])),
        ] {
            let rows = RUN * 3 + 7;
            let a: Array = Matrix::from_column_vec(
                rows,
                2,
                (0..rows * 2).map(|i| ((i * 37) % 101) as f64).collect(),
            )
            .unwrap()
            .into();
            let alias = a.clone();
            let cancel = AtomicBool::new(false);
            let expected = a
                .sort_rows(
                    columns.as_deref(),
                    None,
                    MissingPlacement::Auto,
                    true,
                    &cancel,
                )
                .unwrap();
            ACTIVE.with(|p| p.set(Some((phase, after))));
            let _reset = Reset;
            assert!(
                a.sort_rows(
                    columns.as_deref(),
                    None,
                    MissingPlacement::Auto,
                    true,
                    &cancel
                )
                .unwrap_err()
                .contains("cancelled")
            );
            assert!(cancel.load(AtomicOrdering::Relaxed) && ACTIVE.with(|p| p.get().is_none()));
            assert!(a.same_snapshot(&alias));
            cancel.store(false, AtomicOrdering::Relaxed);
            let retry = a
                .sort_rows(
                    columns.as_deref(),
                    None,
                    MissingPlacement::Auto,
                    true,
                    &cancel,
                )
                .unwrap();
            assert_eq!(
                retry.0.as_numeric().unwrap().data(),
                expected.0.as_numeric().unwrap().data()
            );
            assert_eq!(retry.1.unwrap().data(), expected.1.unwrap().data());
        }
    }
}
