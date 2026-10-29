//! Borrowed real/logical vector membership; production strategy is serial.
use super::sort::{allocate, check, compare, sort_lane};
use crate::{
    Array, LogicalMatrix, Matrix, Result,
    array::{MissingPlacement, SortOrder},
};
#[cfg(test)]
use rayon::prelude::*;
use std::sync::atomic::AtomicBool;

const CHUNK: usize = 1024;
const MAX_PRIVATE_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug)]
pub struct MembershipResult {
    pub membership: Option<LogicalMatrix>,
    pub locations: Option<Matrix>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Initialize,
    Sort,
    Scan,
    Output,
    Query,
    Ordered,
}
fn checkpoint(cancel: &AtomicBool, phase: Phase) -> Result<()> {
    #[cfg(test)]
    tests::probe(cancel, phase);
    let _ = phase;
    check(cancel)
}

#[derive(Clone, Copy)]
enum Values<'a> {
    Real(&'a [f64]),
    Logical(&'a [u8]),
}
impl Values<'_> {
    fn get(self, i: usize) -> f64 {
        match self {
            Self::Real(a) => a[i],
            Self::Logical(a) => f64::from(a[i]),
        }
    }
    fn logical(self) -> bool {
        matches!(self, Self::Logical(_))
    }
}
fn admit(input: &Array) -> Result<Values<'_>> {
    let values = match input {
        Array::Numeric(a) => Values::Real(a.data()),
        Array::Logical(a) => Values::Logical(a.data()),
        _ => return Err("ismember supports real double or logical inputs only".into()),
    };
    Matrix::checked_len(input.rows(), input.cols())?;
    if !input.is_empty() && input.rows() != 1 && input.cols() != 1 {
        return Err("ismember supports vectors and shaped empties, not nonempty matrices".into());
    }
    Ok(values)
}
fn private_budget(m: usize, n: usize, sorted: bool, outputs: [bool; 2]) -> Result<()> {
    let output = m.checked_mul(usize::from(outputs[0]) + 8 * usize::from(outputs[1]));
    // Stable runs may hold <=4n temporary bytes in aggregate, in addition to
    // indices and scratch (8n). Scratch dies before output buffers are created.
    // Borrowed inputs, allocator bookkeeping and thread stacks are excluded.
    let peak = output.and_then(|bytes| {
        if sorted {
            n.checked_mul(4)
                .and_then(|indices| indices.checked_add(bytes))
                .zip(n.checked_mul(12))
                .map(|(output, sorting)| output.max(sorting))
        } else {
            Some(bytes)
        }
    });
    if peak.is_none_or(|bytes| bytes > MAX_PRIVATE_BYTES) {
        return Err("ismember exceeds its 256 MiB private data-buffer budget".into());
    }
    Ok(())
}

enum Set<'a> {
    Empty,
    Bits([usize; 2]),
    Borrowed(&'a [f64]),
    Sorted {
        values: &'a [f64],
        indices: Vec<u32>,
    },
}
impl Set<'_> {
    fn locate(&self, query: f64) -> usize {
        match self {
            Self::Empty => 0,
            Self::Bits(first) => {
                if query == 0. {
                    first[0]
                } else if query == 1. {
                    first[1]
                } else {
                    0
                }
            }
            Self::Borrowed(values) => {
                match_position(values.len(), query, |i| values[i]).map_or(0, |i| i + 1)
            }
            Self::Sorted { values, indices } => {
                match_position(indices.len(), query, |i| values[indices[i] as usize])
                    .map_or(0, |i| indices[i] as usize + 1)
            }
        }
    }
}

// Shared numeric lower-bound/equality policy for the borrowed and indexed views.
fn match_position(len: usize, query: f64, read: impl Fn(usize) -> f64) -> Option<usize> {
    if query.is_nan() {
        return None;
    }
    let (mut lo, mut hi) = (0, len);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        // NaNs sort last and never compare less than a real query.
        if read(mid) < query {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo < len && read(lo) == query {
        Some(lo)
    } else {
        None
    }
}

fn prepare_ordered<'a>(
    set: Values<'a>,
    n: usize,
    bits: bool,
    cancel: &AtomicBool,
    parallel: bool,
) -> Result<Set<'a>> {
    if n == 0 || bits {
        return prepare(set, n, bits, cancel, parallel);
    }
    let Values::Real(values) = set else {
        unreachable!()
    };
    checkpoint(cancel, Phase::Ordered)?;
    for i in 1..n {
        if i.is_multiple_of(CHUNK) {
            checkpoint(cancel, Phase::Ordered)?;
        }
        if compare(
            values[i - 1],
            values[i],
            SortOrder::Ascend,
            MissingPlacement::Last,
        )
        .is_gt()
        {
            // Cancellation must not become permission to allocate/sort.
            checkpoint(cancel, Phase::Ordered)?;
            return prepare(set, n, bits, cancel, parallel);
        }
    }
    checkpoint(cancel, Phase::Ordered)?;
    Ok(Set::Borrowed(values))
}

fn prepare<'a>(
    set: Values<'a>,
    n: usize,
    bits: bool,
    cancel: &AtomicBool,
    _parallel: bool,
) -> Result<Set<'a>> {
    if n == 0 {
        return Ok(Set::Empty);
    }
    if bits {
        let mut first = [0; 2];
        for i in 0..n {
            if i.is_multiple_of(CHUNK) {
                checkpoint(cancel, Phase::Scan)?;
            }
            let value = set.get(i);
            if value == 0. && first[0] == 0 {
                first[0] = i + 1;
            }
            if value == 1. && first[1] == 0 {
                first[1] = i + 1;
            }
            if first.iter().all(|&i| i != 0) {
                break;
            }
        }
        checkpoint(cancel, Phase::Scan)?;
        return Ok(Set::Bits(first));
    }
    let Values::Real(values) = set else {
        unreachable!()
    };
    checkpoint(cancel, Phase::Initialize)?;
    let mut indices = allocate::<u32>(n, cancel)?;
    for (chunk, data) in indices.chunks_mut(CHUNK).enumerate() {
        checkpoint(cancel, Phase::Initialize)?;
        for (at, index) in data.iter_mut().enumerate() {
            *index = (chunk * CHUNK + at) as u32;
        }
    }
    let mut scratch = allocate::<u32>(n, cancel)?;
    #[cfg(not(test))]
    let parallel = false;
    #[cfg(test)]
    let parallel = _parallel;
    checkpoint(cancel, Phase::Sort)?;
    sort_lane(
        &mut indices,
        &mut scratch,
        parallel,
        &|a, b| {
            compare(
                values[a as usize],
                values[b as usize],
                SortOrder::Ascend,
                MissingPlacement::Last,
            )
        },
        cancel,
    )?;
    checkpoint(cancel, Phase::Sort)?;
    drop(scratch);
    Ok(Set::Sorted { values, indices })
}

fn membership_impl(
    a: &Array,
    b: &Array,
    outputs: [bool; 2],
    cancel: &AtomicBool,
    parallel: bool,
) -> Result<MembershipResult> {
    membership_preparation(a, b, outputs, cancel, parallel, true)
}

// The baseline selector is used only by cfg(test) comparisons; public callers
// always request the detector. The unchanged prepare function is the fallback.
fn membership_preparation(
    a: &Array,
    b: &Array,
    outputs: [bool; 2],
    cancel: &AtomicBool,
    _parallel: bool,
    borrow_ordered: bool,
) -> Result<MembershipResult> {
    check(cancel)?;
    let queries = admit(a)?;
    let values = admit(b)?;
    let m = a.len();
    let n = b.len();
    if !outputs.iter().any(|&want| want) {
        check(cancel)?;
        return Ok(MembershipResult {
            membership: None,
            locations: None,
        });
    }
    let bits = queries.logical() || values.logical();
    private_budget(m, n, !bits && m != 0 && n != 0, outputs)?;
    let set = if m == 0 {
        Set::Empty
    } else if borrow_ordered {
        prepare_ordered(values, n, bits, cancel, _parallel)?
    } else {
        prepare(values, n, bits, cancel, _parallel)?
    };
    checkpoint(cancel, Phase::Output)?;
    let mut lia = if outputs[0] {
        Some(allocate::<u8>(m, cancel)?)
    } else {
        None
    };
    let mut loc = if outputs[1] {
        Some(allocate::<f64>(m, cancel)?)
    } else {
        None
    };
    checkpoint(cancel, Phase::Output)?;
    #[cfg(test)]
    let counter = tests::query_counter();
    let fill =
        |start: usize, mut lia: Option<&mut [u8]>, mut loc: Option<&mut [f64]>| -> Result<()> {
            #[cfg(test)]
            tests::query_probe(cancel, &counter);
            checkpoint(cancel, Phase::Query)?;
            let len = lia
                .as_ref()
                .map_or_else(|| loc.as_ref().unwrap().len(), |a| a.len());
            for at in 0..len {
                let index = set.locate(queries.get(start + at));
                if let Some(lia) = &mut lia {
                    lia[at] = u8::from(index != 0);
                }
                if let Some(loc) = &mut loc {
                    loc[at] = index as f64;
                }
            }
            check(cancel)
        };
    match (&mut lia, &mut loc) {
        (Some(lia), Some(loc)) => {
            #[cfg(test)]
            if _parallel {
                lia.par_chunks_mut(CHUNK)
                    .zip(loc.par_chunks_mut(CHUNK))
                    .enumerate()
                    .try_for_each(|(i, (lia, loc))| fill(i * CHUNK, Some(lia), Some(loc)))?;
            } else {
                for (i, (lia, loc)) in lia.chunks_mut(CHUNK).zip(loc.chunks_mut(CHUNK)).enumerate()
                {
                    fill(i * CHUNK, Some(lia), Some(loc))?;
                }
            }
            #[cfg(not(test))]
            for (i, (lia, loc)) in lia.chunks_mut(CHUNK).zip(loc.chunks_mut(CHUNK)).enumerate() {
                fill(i * CHUNK, Some(lia), Some(loc))?;
            }
        }
        (Some(lia), None) => {
            #[cfg(test)]
            if _parallel {
                lia.par_chunks_mut(CHUNK)
                    .enumerate()
                    .try_for_each(|(i, lia)| fill(i * CHUNK, Some(lia), None))?;
            } else {
                for (i, lia) in lia.chunks_mut(CHUNK).enumerate() {
                    fill(i * CHUNK, Some(lia), None)?;
                }
            }
            #[cfg(not(test))]
            for (i, lia) in lia.chunks_mut(CHUNK).enumerate() {
                fill(i * CHUNK, Some(lia), None)?;
            }
        }
        (None, Some(loc)) => {
            #[cfg(test)]
            if _parallel {
                loc.par_chunks_mut(CHUNK)
                    .enumerate()
                    .try_for_each(|(i, loc)| fill(i * CHUNK, None, Some(loc)))?;
            } else {
                for (i, loc) in loc.chunks_mut(CHUNK).enumerate() {
                    fill(i * CHUNK, None, Some(loc))?;
                }
            }
            #[cfg(not(test))]
            for (i, loc) in loc.chunks_mut(CHUNK).enumerate() {
                fill(i * CHUNK, None, Some(loc))?;
            }
        }
        (None, None) => unreachable!(),
    }
    checkpoint(cancel, Phase::Query)?;
    let membership = lia
        .map(|data| LogicalMatrix::from_column_vec(a.rows(), a.cols(), data, cancel))
        .transpose()?;
    let locations = loc
        .map(|data| Matrix::from_column_vec(a.rows(), a.cols(), data))
        .transpose()?;
    check(cancel)?;
    Ok(MembershipResult {
        membership,
        locations,
    })
}

impl Array {
    /// Exact real/logical membership for vectors and shaped empties. Results
    /// retain query shape; locations are the first original one-based set index.
    /// NaNs never match; signed zeros compare equal. No input payload is copied.
    pub fn membership(
        &self,
        set: &Array,
        outputs: [bool; 2],
        cancel: &AtomicBool,
    ) -> Result<MembershipResult> {
        membership_impl(self, set, outputs, cancel, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };
    thread_local! {
        static CANCEL: Cell<Option<(Phase, usize)>> = const { Cell::new(None) };
        static QUERY_COUNTER: RefCell<Option<Arc<AtomicUsize>>> = const { RefCell::new(None) };
    }
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
    pub(super) fn query_counter() -> Option<Arc<AtomicUsize>> {
        QUERY_COUNTER.with(|slot| slot.borrow().clone())
    }
    pub(super) fn query_probe(cancel: &AtomicBool, counter: &Option<Arc<AtomicUsize>>) {
        if let Some(counter) = counter
            && counter.fetch_add(1, Ordering::Relaxed) >= 1
        {
            cancel.store(true, Ordering::Relaxed);
        }
    }
    fn equal(a: &MembershipResult, b: &MembershipResult) {
        assert_eq!(
            a.membership
                .as_ref()
                .map(|a| (a.rows(), a.cols(), a.data())),
            b.membership
                .as_ref()
                .map(|a| (a.rows(), a.cols(), a.data()))
        );
        match (&a.locations, &b.locations) {
            (None, None) => (),
            (Some(a), Some(b)) => {
                assert_eq!((a.rows(), a.cols()), (b.rows(), b.cols()));
                for (a, b) in a.data().iter().zip(b.data()) {
                    assert_eq!(a.to_bits(), b.to_bits());
                }
            }
            _ => panic!("mask mismatch"),
        }
    }
    #[test]
    fn cancellation_during_preparation_and_output_preserves_sources_and_retry() {
        let cancel = AtomicBool::new(false);
        let (a, b) = corpus(4097, 20001, 2);
        let aliases = (a.clone(), b.clone());
        for phase in [Phase::Initialize, Phase::Sort, Phase::Output, Phase::Query] {
            CANCEL.with(|slot| slot.set(Some((phase, 1))));
            assert!(
                a.membership(&b, [true; 2], &cancel)
                    .unwrap_err()
                    .contains("cancelled")
            );
            CANCEL.with(|slot| slot.set(None));
            cancel.store(false, Ordering::Relaxed);
            assert!(a.same_snapshot(&aliases.0) && b.same_snapshot(&aliases.1));
            validate(
                &a.membership(&b, [true; 2], &cancel).unwrap(),
                4097,
                20001,
                2,
                true,
            );
        }
        let only_false: Array = LogicalMatrix::filled(20001, 1, false, &cancel)
            .unwrap()
            .into();
        CANCEL.with(|slot| slot.set(Some((Phase::Scan, 1))));
        assert!(
            a.membership(&only_false, [true; 2], &cancel)
                .unwrap_err()
                .contains("cancelled")
        );
        CANCEL.with(|slot| slot.set(None));
        cancel.store(false, Ordering::Relaxed);
        assert!(a.membership(&only_false, [true; 2], &cancel).is_ok());
        assert!(private_budget(usize::MAX, 1, false, [true; 2]).is_err());
        assert!(private_budget(1, usize::MAX, true, [true; 2]).is_err());
        assert!(
            private_budget(
                crate::matrix::MAX_ELEMENTS,
                crate::matrix::MAX_ELEMENTS,
                true,
                [true; 2]
            )
            .is_ok()
        );
    }
    #[test]
    fn all_masks_and_actual_one_eight_strategies_match_across_merge_boundaries() {
        let pools = [1, 8].map(|n| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(n)
                .build()
                .unwrap()
        });
        let cancel = AtomicBool::new(false);
        for n in [8191, 8192, 8193, 65537] {
            for family in 0..8 {
                let (a, b) = corpus(4097, n, family);
                let aliases = (a.clone(), b.clone());
                for bits in 0..4 {
                    let mask = [bits & 1 != 0, bits & 2 != 0];
                    let serial = a.membership(&b, mask, &cancel).unwrap();
                    assert_eq!(serial.membership.is_some(), mask[0]);
                    assert_eq!(serial.locations.is_some(), mask[1]);
                    for i in 0..a.len() {
                        let expected = analytic_index(i, n, family);
                        if let Some(lia) = &serial.membership {
                            assert_eq!(lia.data()[i], u8::from(expected != 0));
                        }
                        if let Some(loc) = &serial.locations {
                            assert_eq!(loc.data()[i].to_bits(), (expected as f64).to_bits());
                        }
                    }
                    for (slot, pool) in pools.iter().enumerate() {
                        pool.install(|| {
                            assert_eq!(rayon::current_num_threads(), if slot == 0 { 1 } else { 8 });
                            equal(
                                &serial,
                                &membership_impl(&a, &b, mask, &cancel, true).unwrap(),
                            );
                        });
                    }
                    assert!(a.same_snapshot(&aliases.0) && b.same_snapshot(&aliases.1));
                }
            }
        }
        CANCEL.with(|slot| slot.set(Some((Phase::Initialize, 0))));
        let (a, b) = corpus(32, 32, 0);
        assert!(a.membership(&b, [false; 2], &cancel).is_ok());
        assert!(!cancel.load(Ordering::Relaxed));
        CANCEL.with(|slot| slot.set(None));
        let bad: Array = Matrix::filled(2, 2, 0.).unwrap().into();
        assert!(bad.membership(&b, [false; 2], &cancel).is_err());
        assert!(a.membership(&bad, [false; 2], &cancel).is_err());
    }
    #[test]
    fn cancellation_inside_actual_query_chunks_uses_captured_atomic_not_worker_tls() {
        for workers in [1, 8] {
            rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap()
                .install(|| {
                    for family in [1, 5, 6, 7] {
                        let (a, b) = corpus(4097, 8193, family);
                        let cancel = AtomicBool::new(false);
                        for parallel in [false, true] {
                            for mask in [[true, false], [false, true], [true, true]] {
                                let count = Arc::new(AtomicUsize::new(0));
                                QUERY_COUNTER.with(|slot| *slot.borrow_mut() = Some(count.clone()));
                                assert!(
                                    membership_impl(&a, &b, mask, &cancel, parallel)
                                        .unwrap_err()
                                        .contains("cancelled")
                                );
                                QUERY_COUNTER.with(|slot| *slot.borrow_mut() = None);
                                assert!(count.load(Ordering::Relaxed) >= 2);
                                cancel.store(false, Ordering::Relaxed);
                                assert!(membership_impl(&a, &b, mask, &cancel, parallel).is_ok());
                            }
                        }
                    }
                });
        }
    }

    const SIZES: [(usize, usize); 4] =
        [(32, 32), (4096, 4096), (262144, 65536), (1048576, 1048576)];
    const FAMILIES: [&str; 8] = [
        "ordered",
        "reverse",
        "duplicates",
        "ieee",
        "disjoint",
        "logical",
        "real_logical",
        "logical_real",
    ];
    fn ieee(i: usize) -> f64 {
        [
            f64::NAN,
            -0.,
            0.,
            f64::NEG_INFINITY,
            f64::INFINITY,
            f64::from_bits(1),
            -f64::from_bits(1),
            f64::MAX,
            -f64::MAX,
            1.,
            f64::NAN,
            2.,
            1.,
            0.,
            f64::NAN,
            3.,
        ][i % 16]
    }
    fn corpus(m: usize, n: usize, family: usize) -> (Array, Array) {
        let cancel = AtomicBool::new(false);
        let a = if family == 5 || family == 7 {
            LogicalMatrix::from_column_vec(1, m, (0..m).map(|i| (i % 2) as u8).collect(), &cancel)
                .unwrap()
                .into()
        } else {
            Matrix::from_column_vec(
                1,
                m,
                (0..m)
                    .map(|i| match family {
                        0 | 1 => (i % (n + 1)) as f64,
                        2 => (i % 17) as f64,
                        3 => ieee(i),
                        4 => (n + i % n) as f64,
                        6 => (i % 3) as f64 - 1.,
                        _ => unreachable!(),
                    })
                    .collect(),
            )
            .unwrap()
            .into()
        };
        let b = if family == 5 || family == 6 {
            LogicalMatrix::from_column_vec(n, 1, (0..n).map(|i| (i % 2) as u8).collect(), &cancel)
                .unwrap()
                .into()
        } else {
            Matrix::from_column_vec(
                n,
                1,
                (0..n)
                    .map(|i| match family {
                        0 | 4 => i as f64,
                        1 => (n - 1 - i) as f64,
                        2 => (i % 16) as f64,
                        3 => ieee(i),
                        7 => [2., 3., -0., 4., 1.][i % 5],
                        _ => unreachable!(),
                    })
                    .collect(),
            )
            .unwrap()
            .into()
        };
        (a, b)
    }
    fn analytic_index(i: usize, n: usize, family: usize) -> usize {
        match family {
            0 => {
                let q = i % (n + 1);
                if q < n { q + 1 } else { 0 }
            }
            1 => {
                let q = i % (n + 1);
                n.saturating_sub(q)
            }
            2 => {
                let q = i % 17;
                if q < 16 { q + 1 } else { 0 }
            }
            3 => [0, 2, 2, 4, 5, 6, 7, 8, 9, 10, 0, 12, 10, 2, 0, 16][i % 16],
            4 => 0,
            5 => i % 2 + 1,
            6 => [0, 1, 2][i % 3],
            7 => [3, 5][i % 2],
            _ => unreachable!(),
        }
    }
    fn validate(result: &MembershipResult, m: usize, n: usize, family: usize, all: bool) -> String {
        let lia = result.membership.as_ref().unwrap();
        assert_eq!((lia.rows(), lia.cols()), (1, m));
        assert_eq!(result.locations.is_some(), all);
        if let Some(loc) = &result.locations {
            assert_eq!((loc.rows(), loc.cols()), (1, m));
        }
        let mut hash = blake3::Hasher::new();
        for (i, &actual) in lia.data().iter().enumerate() {
            let expected = analytic_index(i, n, family);
            assert_eq!(actual, u8::from(expected != 0));
            hash.update(&[actual]);
            if let Some(loc) = &result.locations {
                let bits = loc.data()[i].to_bits();
                assert_eq!(bits, (expected as f64).to_bits());
                hash.update(&bits.to_le_bytes());
            }
        }
        hash.finalize().to_hex().to_string()
    }

    #[test]
    #[ignore = "Fixed unadopted membership strategy experiment; root-owned release execution"]
    #[allow(
        clippy::assertions_on_constants,
        reason = "Ignored measurement compiles in debug but must reject execution there."
    )]
    fn ismember_strategy_measurement() {
        use std::{hint::black_box, time::Instant};
        assert!(!cfg!(debug_assertions));
        let reverse = std::env::var_os("MATRUST_ISMEMBER_REVERSE").is_some();
        let order = if reverse { "reverse" } else { "normal" };
        let workers = if reverse { [8, 1] } else { [1, 8] };
        let strategies = if reverse {
            [true, false]
        } else {
            [false, true]
        };
        let pools = [1, 8].map(|n| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(n)
                .build()
                .unwrap()
        });
        for (i, pool) in pools.iter().enumerate() {
            pool.install(|| assert_eq!(rayon::current_num_threads(), if i == 0 { 1 } else { 8 }));
        }
        let cancel = AtomicBool::new(false);
        let mut case = 0;
        for (m, n) in SIZES {
            for (family, name) in FAMILIES.iter().enumerate() {
                let (a, b) = corpus(m, n, family);
                let aliases = (a.clone(), b.clone());
                for all in [false, true] {
                    let mut checksum = None;
                    for rep in 0..9 {
                        for workers in workers {
                            for parallel in strategies {
                                let start = Instant::now();
                                let result = pools[usize::from(workers == 8)]
                                    .install(|| {
                                        membership_impl(
                                            black_box(&a),
                                            black_box(&b),
                                            [true, all],
                                            &cancel,
                                            parallel,
                                        )
                                    })
                                    .unwrap();
                                let ns = start.elapsed().as_nanos();
                                let observed = validate(&result, m, n, family, all);
                                if let Some(old) = &checksum {
                                    assert_eq!(old, &observed);
                                } else {
                                    checksum = Some(observed.clone());
                                }
                                assert!(a.same_snapshot(&aliases.0) && b.same_snapshot(&aliases.1));
                                println!(
                                    "ISMEMBER_STRATEGY,{order},{case},{m},{n},{name},{},{workers},{parallel},{rep},{},{ns},{observed}",
                                    if all { 2 } else { 1 },
                                    u8::from(rep < 2)
                                );
                            }
                        }
                    }
                    case += 1;
                }
            }
        }
        assert_eq!(case, 64);
    }

    #[test]
    fn borrowed_ordered_view_and_fallback_match_frozen_preparation_all_masks() {
        let nan = f64::from_bits(0x7ff8_0000_0000_0051);
        let tiny = f64::from_bits(1);
        let cases = [
            (
                vec![
                    f64::NEG_INFINITY,
                    -tiny,
                    -0.,
                    0.,
                    0.,
                    tiny,
                    1.,
                    1.,
                    f64::INFINITY,
                    nan,
                    f64::NAN,
                ],
                true,
            ),
            (vec![-1., 0., -0., 1., f64::INFINITY, nan], true),
            (vec![nan, f64::NAN, nan], true),
            (vec![nan], true),
            (vec![-0.], true),
            (vec![2., 1., 1., 0.], false),
            (vec![-1., 0., -1., 2.], false),
            (vec![-1., 0., 1., -1.], false),
            (vec![nan, 0.], false),
            (vec![nan, f64::INFINITY], false),
            (vec![0., nan, -0.], false),
        ];
        let queries: Array = Matrix::from_column_slice(
            1,
            12,
            &[
                nan,
                f64::NAN,
                f64::NEG_INFINITY,
                -1.,
                -tiny,
                -0.,
                0.,
                tiny,
                1.,
                2.,
                f64::INFINITY,
                f64::MAX,
            ],
        )
        .unwrap()
        .into();
        let cancel = AtomicBool::new(false);
        for workers in [1, 8] {
            rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap()
                .install(|| {
                    assert_eq!(rayon::current_num_threads(), workers);
                    for (data, borrowed) in &cases {
                        let b: Array = Matrix::from_column_slice(data.len(), 1, data)
                            .unwrap()
                            .into();
                        let aliases = (queries.clone(), b.clone());
                        let prepared =
                            prepare_ordered(admit(&b).unwrap(), b.len(), false, &cancel, false)
                                .unwrap();
                        assert_eq!(matches!(prepared, Set::Borrowed(_)), *borrowed);
                        if let Set::Borrowed(values) = prepared {
                            let Array::Numeric(source) = &b else {
                                unreachable!()
                            };
                            assert_eq!(values.as_ptr(), source.data().as_ptr());
                            assert_eq!(values.len(), source.data().len());
                        }
                        for mask in [[false, false], [true, false], [false, true], [true, true]] {
                            for parallel in [false, true] {
                                let old = membership_preparation(
                                    &queries, &b, mask, &cancel, parallel, false,
                                )
                                .unwrap();
                                let new = membership_preparation(
                                    &queries, &b, mask, &cancel, parallel, true,
                                )
                                .unwrap();
                                equal(&old, &new);
                                if let Some(loc) = &new.locations {
                                    for (i, &actual) in loc.data().iter().enumerate() {
                                        let q = admit(&queries).unwrap().get(i);
                                        let first = data
                                            .iter()
                                            .position(|&v| v == q)
                                            .map_or(0., |i| i as f64 + 1.);
                                        assert_eq!(actual.to_bits(), first.to_bits());
                                    }
                                }
                            }
                        }
                        assert!(queries.same_snapshot(&aliases.0) && b.same_snapshot(&aliases.1));
                    }
                });
        }
        // Duplicate runs cross both query and old stable-sort run boundaries.
        let b: Array = Matrix::from_column_vec(
            1,
            16385,
            (0..16385)
                .map(|i| if i < 8193 { -0. } else { 1. })
                .collect(),
        )
        .unwrap()
        .into();
        let a: Array = Matrix::from_column_vec(
            1,
            2049,
            (0..2049)
                .map(|i| if i % 2 == 0 { 0. } else { 1. })
                .collect(),
        )
        .unwrap()
        .into();
        let result = a.membership(&b, [true; 2], &cancel).unwrap();
        for (i, &v) in result.locations.unwrap().data().iter().enumerate() {
            assert_eq!(v, if i % 2 == 0 { 1. } else { 8194. });
        }
    }

    #[test]
    fn ordered_scan_cancellation_and_skipped_routes_do_not_allocate_sort_buffers() {
        let cancel = AtomicBool::new(false);
        let (a, b) = corpus(32, 2049, 0);
        let late: Array = Matrix::from_column_vec(
            1,
            2049,
            (0..2049)
                .map(|i| if i == 2048 { -1. } else { i as f64 })
                .collect(),
        )
        .unwrap()
        .into();
        for set in [&b, &late] {
            for skip in [0, 1, 3] {
                CANCEL.with(|slot| slot.set(Some((Phase::Ordered, skip))));
                assert!(
                    a.membership(set, [true; 2], &cancel)
                        .unwrap_err()
                        .contains("cancelled")
                );
                CANCEL.with(|slot| slot.set(None));
                cancel.store(false, Ordering::Relaxed);
                equal(
                    &membership_preparation(&a, set, [true; 2], &cancel, false, false).unwrap(),
                    &a.membership(set, [true; 2], &cancel).unwrap(),
                );
            }
        }
        CANCEL.with(|slot| slot.set(Some((Phase::Initialize, 0))));
        assert!(a.membership(&b, [true; 2], &cancel).is_ok());
        assert!(!cancel.load(Ordering::Relaxed));
        CANCEL.with(|slot| slot.set(None));
        let empty: Array = Matrix::from_column_vec(0, 3, vec![]).unwrap().into();
        let logical: Array = LogicalMatrix::filled(1, 3, false, &cancel).unwrap().into();
        for (query, set, mask) in [
            (&a, &b, [false; 2]),
            (&empty, &b, [true; 2]),
            (&a, &empty, [true; 2]),
            (&logical, &b, [true; 2]),
            (&a, &logical, [true; 2]),
        ] {
            CANCEL.with(|slot| slot.set(Some((Phase::Ordered, 0))));
            assert!(query.membership(set, mask, &cancel).is_ok());
            assert!(!cancel.load(Ordering::Relaxed));
            CANCEL.with(|slot| slot.set(None));
        }
        assert!(private_budget(usize::MAX, 1, true, [true; 2]).is_err());
    }

    #[test]
    #[ignore = "Fixed ordered-set before/after and strategy experiment, including two labeled late-inversion controls"]
    #[allow(
        clippy::assertions_on_constants,
        reason = "Ignored measurement compiles in debug but must reject execution there."
    )]
    fn ismember_ordered_preparation_measurement() {
        use std::{hint::black_box, time::Instant};
        assert!(!cfg!(debug_assertions));
        let reverse = std::env::var_os("MATRUST_ISMEMBER_ORDERED_REVERSE").is_some();
        let order = if reverse { "reverse" } else { "normal" };
        let workers = if reverse { [8, 1] } else { [1, 8] };
        let preparations = if reverse {
            [true, false]
        } else {
            [false, true]
        };
        let strategies = if reverse {
            [true, false]
        } else {
            [false, true]
        };
        let pools = [1, 8].map(|n| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(n)
                .build()
                .unwrap()
        });
        for (i, pool) in pools.iter().enumerate() {
            pool.install(|| assert_eq!(rayon::current_num_threads(), if i == 0 { 1 } else { 8 }));
        }
        let cancel = AtomicBool::new(false);
        let run = |suite: &str,
                   case: usize,
                   m: usize,
                   n: usize,
                   family: usize,
                   all: bool,
                   a: &Array,
                   b: &Array| {
            let aliases = (a.clone(), b.clone());
            let mut checksum = None;
            for rep in 0..9 {
                for workers in workers {
                    for borrowed in preparations {
                        for parallel in strategies {
                            let start = Instant::now();
                            let result = pools[usize::from(workers == 8)]
                                .install(|| {
                                    membership_preparation(
                                        black_box(a),
                                        black_box(b),
                                        [true, all],
                                        &cancel,
                                        parallel,
                                        borrowed,
                                    )
                                })
                                .unwrap();
                            let ns = start.elapsed().as_nanos();
                            let observed = if suite == "original64" {
                                validate(&result, m, n, family, all)
                            } else {
                                let lia = result.membership.as_ref().unwrap();
                                assert_eq!((lia.rows(), lia.cols()), (1, m));
                                assert_eq!(result.locations.is_some(), all);
                                if let Some(loc) = &result.locations {
                                    assert_eq!((loc.rows(), loc.cols()), (1, m));
                                }
                                let mut hash = blake3::Hasher::new();
                                for (i, &actual) in lia.data().iter().enumerate() {
                                    let q = i % (n + 1);
                                    let expected = if q <= n - 2 { q + 1 } else { 0 };
                                    assert_eq!(actual, u8::from(expected != 0));
                                    hash.update(&[actual]);
                                    if let Some(loc) = &result.locations {
                                        assert_eq!(
                                            loc.data()[i].to_bits(),
                                            (expected as f64).to_bits()
                                        );
                                        hash.update(&loc.data()[i].to_bits().to_le_bytes());
                                    }
                                }
                                hash.finalize().to_hex().to_string()
                            };
                            if let Some(old) = &checksum {
                                assert_eq!(old, &observed);
                            } else {
                                checksum = Some(observed.clone());
                            }
                            assert!(a.same_snapshot(&aliases.0) && b.same_snapshot(&aliases.1));
                            let name = if suite == "original64" {
                                FAMILIES[family]
                            } else {
                                "late_inversion"
                            };
                            println!(
                                "ISMEMBER_ORDERED,{suite},{order},{case},{m},{n},{name},{},{workers},{borrowed},{parallel},{rep},{},{ns},{observed}",
                                if all { 2 } else { 1 },
                                u8::from(rep < 2)
                            );
                        }
                    }
                }
            }
        };
        let mut case = 0;
        for (m, n) in SIZES {
            for family in 0..FAMILIES.len() {
                let (a, b) = corpus(m, n, family);
                for all in [false, true] {
                    run("original64", case, m, n, family, all, &a, &b);
                    case += 1;
                }
            }
        }
        assert_eq!(case, 64);
        let (m, n) = (1048576, 1048576);
        let (a, _) = corpus(m, n, 0);
        let b: Array = Matrix::from_column_vec(
            n,
            1,
            (0..n)
                .map(|i| if i == n - 1 { -1. } else { i as f64 })
                .collect(),
        )
        .unwrap()
        .into();
        for (case, all) in [false, true].into_iter().enumerate() {
            run("late2", case, m, n, 0, all, &a, &b);
        }
    }
}
