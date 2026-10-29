//! Bounded explicit-coordinate interpolation; source samples remain borrowed.
use super::sort::{allocate, check, sort_lane};
use crate::{Matrix, Result};
#[cfg(test)]
use rayon::prelude::*;
use std::sync::atomic::AtomicBool;

const CHECK_CHUNK: usize = 1024;
const SORT_PAR: usize = 512 * 1024;
const MAX_PRIVATE_BYTES: usize = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterpolationMethod {
    Linear,
    Nearest,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Validate,
    Index,
    Sort,
    Ordered,
    Query,
}
fn checkpoint(cancel: &AtomicBool, phase: Phase) -> Result<()> {
    #[cfg(test)]
    tests::probe(cancel, phase);
    let _ = phase;
    check(cancel)
}

struct Order {
    indices: Option<Vec<u32>>,
    reverse: bool,
    len: usize,
}
impl Order {
    fn at(&self, rank: usize) -> usize {
        match &self.indices {
            Some(indices) => indices[rank] as usize,
            None if self.reverse => self.len - 1 - rank,
            None => rank,
        }
    }
}

fn private_budget(n: usize, m: usize, indexed: bool) -> Result<()> {
    // Sorting peak: permutation + merge scratch + conservative stable-run
    // temporary <=12N bytes. Scratch dies before output, whose peak is 4N+8M.
    // Monotonic inputs need no indices. Input storage/allocator metadata excluded.
    let output = m.checked_mul(8);
    let bytes = if indexed {
        n.checked_mul(4)
            .and_then(|indices| output.and_then(|out| indices.checked_add(out)))
            .zip(n.checked_mul(12))
            .map(|(gather, sort)| gather.max(sort))
    } else {
        output
    };
    if bytes.is_none_or(|bytes| bytes > MAX_PRIVATE_BYTES) {
        return Err("interp1 exceeds its 256 MiB private data-buffer budget".into());
    }
    Ok(())
}

fn coordinate_order(x: &[f64], v: &[f64], m: usize, cancel: &AtomicBool) -> Result<Order> {
    let (mut ascending, mut descending) = (true, true);
    for i in 0..x.len() {
        if i.is_multiple_of(CHECK_CHUNK) {
            checkpoint(cancel, Phase::Validate)?;
        }
        if !x[i].is_finite() || !v[i].is_finite() {
            return Err("interp1 requires finite coordinates and finite sample values".into());
        }
        if i > 0 {
            ascending &= x[i - 1] < x[i];
            descending &= x[i - 1] > x[i];
        }
    }
    checkpoint(cancel, Phase::Validate)?;
    private_budget(x.len(), m, !ascending && !descending)?;
    if ascending || descending {
        return Ok(Order {
            indices: None,
            reverse: descending,
            len: x.len(),
        });
    }
    let mut indices = allocate::<u32>(x.len(), cancel)?;
    for (chunk, values) in indices.chunks_mut(CHECK_CHUNK).enumerate() {
        checkpoint(cancel, Phase::Index)?;
        for (offset, value) in values.iter_mut().enumerate() {
            *value = (chunk * CHECK_CHUNK + offset) as u32;
        }
    }
    let mut scratch = allocate::<u32>(x.len(), cancel)?;
    checkpoint(cancel, Phase::Sort)?;
    // Only the existing sort candidate uses the caller pool; queries stay serial.
    sort_lane(
        &mut indices,
        &mut scratch,
        x.len() >= SORT_PAR && rayon::current_num_threads() > 1,
        &|a, b| x[a as usize].partial_cmp(&x[b as usize]).unwrap(),
        cancel,
    )?;
    checkpoint(cancel, Phase::Sort)?;
    drop(scratch);
    for i in 1..indices.len() {
        if i.is_multiple_of(CHECK_CHUNK) {
            checkpoint(cancel, Phase::Ordered)?;
        }
        if x[indices[i - 1] as usize] == x[indices[i] as usize] {
            return Err("interp1 coordinates must be distinct, including signed zeros".into());
        }
    }
    checkpoint(cancel, Phase::Ordered)?;
    Ok(Order {
        indices: Some(indices),
        reverse: false,
        len: x.len(),
    })
}

// Error-free TwoSum residual for a finite rounded sum (IEEE binary64,
// round-to-nearest, no fast-math). Standard arithmetic identity; see Shewchuk
// 1997 / public-domain predicates reference in the private source packet.
fn sum_residual(a: f64, b: f64, sum: f64) -> f64 {
    let virtual_b = sum - a;
    (a - (sum - virtual_b)) + (b - virtual_b)
}

fn nearest_upper(lo: f64, hi: f64, query: f64) -> bool {
    let left = query - lo;
    let right = hi - query;
    // Both cannot overflow: their exact sum is at most 2*MAX finite magnitude.
    if left != right {
        return left > right;
    }
    // Rounded distances may tie without an exact midpoint (e.g. +/-MAX and
    // a subnormal query). Compare their residuals before choosing upper on tie.
    let left_tail = sum_residual(query, -lo, left);
    let right_tail = sum_residual(hi, -query, right);
    left_tail >= right_tail
}

fn fraction(lo: f64, hi: f64, query: f64) -> f64 {
    let span = hi - lo;
    if span.is_finite() {
        (query - lo) / span
    } else {
        (query * 0.5 - lo * 0.5) / (hi * 0.5 - lo * 0.5)
    }
}

fn blend(a: f64, b: f64, t: f64) -> f64 {
    if a.is_sign_negative() != b.is_sign_negative() {
        // Opposite-sign difference can overflow; weighted terms cannot.
        (1. - t) * a + t * b
    } else if t <= 0.5 {
        // Same-sign difference is finite. Use the nearer endpoint to avoid
        // rounding a large sum beyond the finite segment's upper magnitude.
        a + t * (b - a)
    } else {
        b - (1. - t) * (b - a)
    }
}

/// Real finite vectors x/v, N>=2, distinct x; real queries retain their 2-D shape.
/// NaN queries yield NaN, out-of-range queries copy fill bits, exact knots copy
/// sample bits. Nearest midpoint chooses the higher coordinate. No extrapolation.
pub fn interpolate_vectors(
    x: &Matrix,
    v: &Matrix,
    xq: &Matrix,
    method: InterpolationMethod,
    fill: f64,
    cancel: &AtomicBool,
) -> Result<Matrix> {
    interpolate_impl(x, v, xq, method, fill, cancel, false)
}

fn interpolate_impl(
    x: &Matrix,
    v: &Matrix,
    xq: &Matrix,
    method: InterpolationMethod,
    fill: f64,
    cancel: &AtomicBool,
    _parallel: bool,
) -> Result<Matrix> {
    check(cancel)?;
    Matrix::checked_len(x.rows(), x.cols())?;
    Matrix::checked_len(v.rows(), v.cols())?;
    let m = Matrix::checked_len(xq.rows(), xq.cols())?;
    if x.len() < 2
        || x.len() != v.len()
        || (x.rows() != 1 && x.cols() != 1)
        || (v.rows() != 1 && v.cols() != 1)
    {
        return Err(
            "interp1 x and v must be equal-length real vectors with at least two samples".into(),
        );
    }
    let (x, v) = (x.data(), v.data());
    let order = coordinate_order(x, v, m, cancel)?;
    let (minimum, maximum) = (x[order.at(0)], x[order.at(x.len() - 1)]);
    let mut output = allocate::<f64>(m, cancel)?;
    checkpoint(cancel, Phase::Query)?;
    #[cfg(test)]
    let query_counter = tests::query_counter();
    let fill_chunk = |(chunk, values): (usize, &mut [f64])| -> Result<()> {
        #[cfg(test)]
        tests::query_probe(cancel, &query_counter);
        checkpoint(cancel, Phase::Query)?;
        for (offset, value) in values.iter_mut().enumerate() {
            let q = xq.data()[chunk * CHECK_CHUNK + offset];
            if q.is_nan() {
                *value = f64::NAN;
                continue;
            }
            if q < minimum || q > maximum {
                *value = fill;
                continue;
            }
            // Lower_bound over ordered coordinates; each search is <=25 steps
            // under MAX_ELEMENTS. No sorted sample or query payload copy.
            let (mut low, mut high) = (0, x.len());
            while low < high {
                let middle = low + (high - low) / 2;
                if x[order.at(middle)] < q {
                    low = middle + 1;
                } else {
                    high = middle;
                }
            }
            let right = order.at(low);
            if x[right] == q {
                *value = v[right];
                continue;
            }
            let left = order.at(low - 1);
            *value = match method {
                InterpolationMethod::Nearest => {
                    if nearest_upper(x[left], x[right], q) {
                        v[right]
                    } else {
                        v[left]
                    }
                }
                InterpolationMethod::Linear => {
                    blend(v[left], v[right], fraction(x[left], x[right], q))
                }
            };
        }
        Ok(())
    };
    #[cfg(test)]
    if _parallel {
        output
            .par_chunks_mut(CHECK_CHUNK)
            .enumerate()
            .try_for_each(fill_chunk)?;
        checkpoint(cancel, Phase::Query)?;
        return Matrix::from_column_vec(xq.rows(), xq.cols(), output);
    }
    for chunk in output.chunks_mut(CHECK_CHUNK).enumerate() {
        fill_chunk(chunk)?;
    }
    checkpoint(cancel, Phase::Query)?;
    Matrix::from_column_vec(xq.rows(), xq.cols(), output)
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
    fn caller_phase_cancellation_preserves_inputs_and_retry() {
        let n = 20_001;
        let x =
            Matrix::from_column_vec(1, n, (0..n).map(|i| ((i * 37) % n) as f64).collect()).unwrap();
        let v = x.clone();
        let q = Matrix::from_column_vec(1, 2049, (0..2049).map(|i| i as f64).collect()).unwrap();
        let pointers = (x.data().as_ptr(), v.data().as_ptr(), q.data().as_ptr());
        let cancel = AtomicBool::new(false);
        for phase in [
            Phase::Validate,
            Phase::Index,
            Phase::Sort,
            Phase::Ordered,
            Phase::Query,
        ] {
            CANCEL.with(|slot| slot.set(Some((phase, 1))));
            assert!(
                interpolate_vectors(&x, &v, &q, InterpolationMethod::Linear, 0., &cancel)
                    .unwrap_err()
                    .contains("cancelled")
            );
            CANCEL.with(|slot| slot.set(None));
            cancel.store(false, Ordering::Relaxed);
            assert_eq!(
                pointers,
                (x.data().as_ptr(), v.data().as_ptr(), q.data().as_ptr())
            );
            assert_eq!(
                interpolate_vectors(&x, &v, &q, InterpolationMethod::Linear, 0., &cancel)
                    .unwrap()
                    .data(),
                q.data()
            );
        }
        assert!(private_budget(usize::MAX, 1, true).is_err());
        assert!(private_budget(1, usize::MAX, false).is_err());
    }
    #[test]
    fn nearest_compares_residuals_and_monotonic_sources_need_no_indices() {
        let max = f64::MAX;
        let tiny = f64::from_bits(1);
        assert!(!nearest_upper(-max, max, -tiny));
        assert!(nearest_upper(-max, max, 0.));
        assert!(nearest_upper(-max, max, tiny));
        assert!(!nearest_upper(-max, max, -max / 2.));
        assert!(nearest_upper(-max, max, max / 2.));
        let cancel = AtomicBool::new(false);
        assert!(
            coordinate_order(&[0., 1., 2.], &[2., 3., 4.], 0, &cancel)
                .unwrap()
                .indices
                .is_none()
        );
        let descending = coordinate_order(&[2., 1., 0.], &[4., 3., 2.], 0, &cancel).unwrap();
        assert!(descending.indices.is_none() && descending.reverse);
        assert!(
            coordinate_order(&[1., 0., 2.], &[3., 2., 4.], 0, &cancel)
                .unwrap()
                .indices
                .is_some()
        );
    }

    #[test]
    fn test_parallel_query_strategy_matches_bits_and_cancels_inside_chunks() {
        for workers in [1, 8] {
            rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap()
                .install(|| {
                    assert_eq!(rayon::current_num_threads(), workers);
                    let x = Matrix::from_column_vec(
                        1,
                        33,
                        (0..33).map(|i| ((i * 2) % 33) as f64).collect(),
                    )
                    .unwrap();
                    let v = Matrix::from_column_vec(
                        33,
                        1,
                        x.data().iter().map(|x| 2. * x + 1.).collect(),
                    )
                    .unwrap();
                    let q = Matrix::from_column_vec(
                        1,
                        4097,
                        (0..4097)
                            .map(|i| match i % 19 {
                                0 => f64::NAN,
                                1 => f64::NEG_INFINITY,
                                2 => f64::INFINITY,
                                _ => (i % 129) as f64 / 4.,
                            })
                            .collect(),
                    )
                    .unwrap();
                    let cancel = AtomicBool::new(false);
                    for method in [InterpolationMethod::Linear, InterpolationMethod::Nearest] {
                        let serial =
                            interpolate_impl(&x, &v, &q, method, -0., &cancel, false).unwrap();
                        let parallel =
                            interpolate_impl(&x, &v, &q, method, -0., &cancel, true).unwrap();
                        for (serial, parallel) in serial.data().iter().zip(parallel.data()) {
                            assert_eq!(serial.to_bits(), parallel.to_bits());
                        }
                        for strategy in [false, true] {
                            let count = Arc::new(AtomicUsize::new(0));
                            QUERY_COUNTER.with(|slot| *slot.borrow_mut() = Some(count.clone()));
                            assert!(
                                interpolate_impl(&x, &v, &q, method, -0., &cancel, strategy)
                                    .unwrap_err()
                                    .contains("cancelled")
                            );
                            QUERY_COUNTER.with(|slot| *slot.borrow_mut() = None);
                            assert!(count.load(Ordering::Relaxed) >= 2);
                            cancel.store(false, Ordering::Relaxed);
                            assert_eq!(
                                interpolate_impl(
                                    &x,
                                    &v,
                                    &row_query(),
                                    method,
                                    -7.,
                                    &cancel,
                                    strategy
                                )
                                .unwrap()
                                .data(),
                                &[1., 3.]
                            );
                        }
                    }
                });
        }
    }
    fn row_query() -> Matrix {
        Matrix::from_column_slice(1, 2, &[0., 1.]).unwrap()
    }

    #[test]
    #[ignore = "Fixed unadopted query-strategy experiment; root-owned release execution"]
    #[allow(
        clippy::assertions_on_constants,
        reason = "Ignored measurement compiles in debug but must reject execution there."
    )]
    fn interp1_query_strategy_measurement() {
        use std::{hint::black_box, time::Instant};
        assert!(!cfg!(debug_assertions));
        let reverse = std::env::var_os("MATRUST_INTERP1_REVERSE").is_some();
        let workers = if reverse { [8, 1] } else { [1, 8] };
        let strategies = if reverse {
            [true, false]
        } else {
            [false, true]
        };
        let pools = [1, 8].map(|workers| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap()
        });
        for (slot, pool) in pools.iter().enumerate() {
            pool.install(|| {
                assert_eq!(rayon::current_num_threads(), if slot == 0 { 1 } else { 8 })
            });
        }
        let cancel = AtomicBool::new(false);
        let mut case = 0;
        for n in [2usize, 32, 4096, 65536] {
            for m in [32usize, 4096, 262144, 1048576] {
                for method in [InterpolationMethod::Linear, InterpolationMethod::Nearest] {
                    for unordered in [false, true] {
                        let x = Matrix::from_column_vec(
                            1,
                            n,
                            (0..n)
                                .map(|i| {
                                    if unordered {
                                        ((i * 37 + 1) % n) as f64
                                    } else {
                                        i as f64
                                    }
                                })
                                .collect(),
                        )
                        .unwrap();
                        let v = Matrix::from_column_vec(
                            n,
                            1,
                            x.data().iter().map(|x| 2. * x + 1.).collect(),
                        )
                        .unwrap();
                        let q = Matrix::from_column_vec(
                            1,
                            m,
                            (0..m)
                                .map(|i| match i % 16 {
                                    0 => -1.,
                                    1 => n as f64,
                                    2 => (i % n) as f64,
                                    _ => ((i * 37) % ((n - 1) * 4 + 1)) as f64 / 4.,
                                })
                                .collect(),
                        )
                        .unwrap();
                        for rep in 0..9 {
                            for workers in workers {
                                for parallel in strategies {
                                    let pool = &pools[usize::from(workers == 8)];
                                    let start = Instant::now();
                                    let result = pool
                                        .install(|| {
                                            interpolate_impl(
                                                black_box(&x),
                                                &v,
                                                &q,
                                                method,
                                                -7.,
                                                &cancel,
                                                parallel,
                                            )
                                        })
                                        .unwrap();
                                    let ns = start.elapsed().as_nanos();
                                    assert_eq!((result.rows(), result.cols()), (1, m));
                                    let mut hash = blake3::Hasher::new();
                                    for (&query, &actual) in q.data().iter().zip(result.data()) {
                                        let expected = if query < 0. || query > (n - 1) as f64 {
                                            -7.
                                        } else {
                                            2. * match method {
                                                InterpolationMethod::Linear => query,
                                                InterpolationMethod::Nearest => {
                                                    (query + 0.5).floor()
                                                }
                                            } + 1.
                                        };
                                        assert_eq!(actual.to_bits(), expected.to_bits());
                                        hash.update(&actual.to_bits().to_le_bytes());
                                    }
                                    let checksum = hash.finalize().to_hex();
                                    println!(
                                        "INTERP1_STRATEGY,{case},{n},{m},{method:?},{unordered},{workers},{parallel},{rep},{},{ns},{checksum}",
                                        u8::from(rep < 2)
                                    );
                                }
                            }
                        }
                        case += 1;
                    }
                }
            }
        }
        assert_eq!(case, 64);
    }
}
