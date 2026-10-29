//! Serial real-double powers of two; immutable inputs and one bounded result.
use super::sort::check;
use crate::{Matrix, Result};
use std::sync::atomic::AtomicBool;

const CHUNK: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Validate,
    Allocate,
    Map,
    Publish,
}

fn checkpoint(cancel: &AtomicBool, phase: Phase) -> Result<()> {
    #[cfg(test)]
    tests::probe(cancel, phase);
    let _ = phase;
    check(cancel)
}

// For a finite positive binary64, exponent and fraction determine ceil(log2(x))
// exactly. Subnormal bit k represents 2^(k-1074). No floating logarithm, input
// multiplication, rounding tolerance or vendor source code is used here.
fn next_exponent(value: f64) -> f64 {
    let magnitude = value.to_bits() & 0x7fff_ffff_ffff_ffff;
    if magnitude == 0 {
        return 0.;
    }
    let exponent = (magnitude >> 52) as i32;
    let fraction = magnitude & 0x000f_ffff_ffff_ffff;
    if exponent == 0x7ff {
        return if fraction == 0 {
            f64::INFINITY
        } else {
            f64::NAN
        };
    }
    if exponent == 0 {
        let highest_bit = 63 - fraction.leading_zeros() as i32;
        return f64::from(highest_bit - 1074 + i32::from(!fraction.is_power_of_two()));
    }
    f64::from(exponent - 1023 + i32::from(fraction != 0))
}

/// Smallest binary exponent p for which 2^p >= abs(x), elementwise.
/// Signed zeros map to +0; either infinity maps to +Inf; NaNs remain NaN.
/// Shape, including all admitted 2-D empties, is retained. Serial execution.
pub fn nextpow2(input: &Matrix, cancel: &AtomicBool) -> Result<Matrix> {
    unary(input, cancel, next_exponent)
}

/// Elementwise 2^x using the locked libm kernel, including fractional exponents.
/// +/-0 map to 1, -Inf to +0, +Inf to +Inf, and NaN to NaN. Finite overflow and
/// rounded underflow are allowed. No NaN payload or exception-flag guarantee.
pub fn pow2(input: &Matrix, cancel: &AtomicBool) -> Result<Matrix> {
    unary(input, cancel, libm::exp2)
}

fn unary(input: &Matrix, cancel: &AtomicBool, operation: fn(f64) -> f64) -> Result<Matrix> {
    check(cancel)?;
    Matrix::checked_len(input.rows(), input.cols())?;
    let mut output = allocate(input.len(), cancel)?;
    for chunk in input.data().chunks(CHUNK) {
        checkpoint(cancel, Phase::Map)?;
        output.extend(chunk.iter().copied().map(operation));
    }
    checkpoint(cancel, Phase::Publish)?;
    Matrix::from_column_vec(input.rows(), input.cols(), output)
}

pub(crate) fn scale_pow2_shape(f: &Matrix, e: &Matrix) -> Result<(usize, usize)> {
    let shape = if f.rows() == e.rows() && f.cols() == e.cols() {
        (f.rows(), f.cols())
    } else if f.len() == 1 {
        (e.rows(), e.cols())
    } else if e.len() == 1 {
        (f.rows(), f.cols())
    } else {
        return Err("pow2(F,E) requires equal 2-D shapes or either input scalar; row/column broadcasting is unsupported".into());
    };
    Matrix::checked_len(shape.0, shape.1)?;
    Ok(shape)
}

fn allocate(len: usize, cancel: &AtomicBool) -> Result<Vec<f64>> {
    checkpoint(cancel, Phase::Allocate)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(len)
        .map_err(|_| "Cannot allocate powers-of-two output")?;
    checkpoint(cancel, Phase::Allocate)?;
    Ok(output)
}

/// Scale F by 2^fix(E), with finite real exponents and real IEEE significands.
/// Equal shapes or either scalar are admitted. A scalar with an empty array
/// produces that empty shape; different nonscalar empty shapes reject. Even
/// when output is empty, every supplied exponent must be finite.
///
/// Finite E truncates toward zero and saturates to i32 for libm::scalbn. Outside
/// i32 range every finite nonzero binary64 necessarily over/underflows; zero,
/// infinity and NaN classifications are retained by scalbn. This avoids forming
/// an overflowing/underflowing 2^E intermediate. No pool or input copy is made.
pub fn scale_pow2(f: &Matrix, e: &Matrix, cancel: &AtomicBool) -> Result<Matrix> {
    check(cancel)?;
    let (rows, cols) = scale_pow2_shape(f, e)?;
    let len = Matrix::checked_len(rows, cols)?;
    for chunk in e.data().chunks(CHUNK) {
        checkpoint(cancel, Phase::Validate)?;
        if chunk.iter().any(|value| !value.is_finite()) {
            return Err(
                "pow2(F,E) requires finite exponents; NaN/Inf exponent behavior is unsupported"
                    .into(),
            );
        }
    }
    let mut output = allocate(len, cancel)?;
    for start in (0..len).step_by(CHUNK) {
        checkpoint(cancel, Phase::Map)?;
        for index in start..(start + CHUNK).min(len) {
            let significand = f.data()[if f.len() == 1 { 0 } else { index }];
            let exponent = e.data()[if e.len() == 1 { 0 } else { index }];
            // Rust's finite float-to-integer cast truncates and saturates. The
            // explicit clamp documents the finite-exponent range contract.
            let integer = exponent
                .trunc()
                .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
            output.push(libm::scalbn(significand, integer));
        }
    }
    checkpoint(cancel, Phase::Publish)?;
    Matrix::from_column_vec(rows, cols, output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        sync::atomic::Ordering,
    };

    thread_local! {
        static ACTIVE: Cell<bool> = const { Cell::new(false) };
        static TARGET: Cell<Option<(Phase, usize)>> = const { Cell::new(None) };
        static TRACE: RefCell<Vec<Phase>> = const { RefCell::new(Vec::new()) };
    }
    pub(super) fn probe(cancel: &AtomicBool, phase: Phase) {
        if !ACTIVE.get() {
            return;
        }
        TRACE.with_borrow_mut(|trace| trace.push(phase));
        if let Some((target, remaining)) = TARGET.get()
            && target == phase
        {
            if remaining == 1 {
                cancel.store(true, Ordering::Relaxed);
            } else {
                TARGET.set(Some((target, remaining - 1)));
            }
        }
    }
    struct Observe;
    impl Observe {
        fn new(target: Option<(Phase, usize)>) -> Self {
            assert!(!ACTIVE.replace(true));
            TARGET.set(target);
            TRACE.with_borrow_mut(Vec::clear);
            Self
        }
    }
    impl Drop for Observe {
        fn drop(&mut self) {
            ACTIVE.set(false);
            TARGET.set(None);
            TRACE.with_borrow_mut(Vec::clear);
        }
    }

    #[test]
    fn cancel_during_validation_reservation_after_completed_chunk_and_publication() {
        let input = Matrix::from_column_vec(1, 2 * CHUNK + 1, vec![1.; 2 * CHUNK + 1]).unwrap();
        let alias = input.clone();
        for operation in 0..3 {
            for (phase, count) in [
                (Phase::Validate, 2),
                (Phase::Allocate, 1),
                (Phase::Allocate, 2),
                (Phase::Map, 2),
                (Phase::Publish, 1),
            ] {
                if operation != 2 && phase == Phase::Validate {
                    continue;
                }
                let cancel = AtomicBool::new(false);
                {
                    let _observe = Observe::new(Some((phase, count)));
                    let result = match operation {
                        0 => nextpow2(&input, &cancel),
                        1 => pow2(&input, &cancel),
                        _ => scale_pow2(&input, &input, &cancel),
                    };
                    assert_eq!(result.unwrap_err(), "Calculation cancelled");
                    TRACE.with_borrow(|trace| {
                        assert_eq!(trace.last(), Some(&phase));
                        if phase == Phase::Validate {
                            assert!(!trace.contains(&Phase::Allocate));
                        }
                        if phase == Phase::Map {
                            assert_eq!(trace.iter().filter(|&&x| x == Phase::Map).count(), 2);
                        }
                    });
                }
                cancel.store(false, Ordering::Relaxed);
                let output = scale_pow2(&input, &input, &cancel).unwrap();
                assert!(output.data().iter().all(|&x| x == 2.));
                assert_eq!(input.data().as_ptr(), alias.data().as_ptr());
            }
        }
    }

    #[test]
    fn nonfinite_last_exponent_rejects_before_reserving_any_result() {
        let f = Matrix::scalar(1.);
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut values = vec![0.; CHUNK + 1];
            values[CHUNK] = bad;
            let e = Matrix::from_column_vec(1, values.len(), values).unwrap();
            let _observe = Observe::new(None);
            assert!(
                scale_pow2(&f, &e, &AtomicBool::new(false))
                    .unwrap_err()
                    .contains("finite exponents")
            );
            TRACE.with_borrow(|trace| {
                assert_eq!(trace.as_slice(), &[Phase::Validate, Phase::Validate])
            });
        }
    }
}
