use super::*;
use rayon::prelude::*;
use std::cell::Cell;

thread_local! {
    static CANCEL_AT: Cell<Option<(Phase, usize)>> = const { Cell::new(None) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

pub(super) fn allocation_probe() {
    ALLOCATIONS.with(|count| count.set(count.get() + 1));
}

pub(super) fn checkpoint_probe(cancel: &AtomicBool, phase: Phase) {
    CANCEL_AT.with(|slot| {
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

// Measurement-only candidate. Production never creates a pool or selects this
// path. Two requested outputs share each frexp evaluation, without pair scratch.
pub(super) fn parallel_dissect(
    input: &[f64],
    fraction: &mut Option<Vec<f64>>,
    exponent: &mut Option<Vec<f64>>,
    cancel: &AtomicBool,
) -> Result<()> {
    for output in [&mut *fraction, &mut *exponent].into_iter().flatten() {
        for first in (0..input.len()).step_by(CHUNK) {
            checkpoint(cancel, Phase::Allocation)?;
            output.resize((first + CHUNK).min(input.len()), 0.);
        }
    }
    match (fraction, exponent) {
        (Some(f), Some(e)) => f
            .par_chunks_mut(CHUNK)
            .zip(e.par_chunks_mut(CHUNK))
            .zip(input.par_chunks(CHUNK))
            .try_for_each(|((f, e), input)| -> Result<()> {
                checkpoint(cancel, Phase::Mapping)?;
                for ((f, e), &x) in f.iter_mut().zip(e).zip(input) {
                    let pair = libm::frexp(x);
                    *f = pair.0;
                    *e = f64::from(pair.1);
                }
                checkpoint(cancel, Phase::Mapping)
            })?,
        (Some(f), None) => f
            .par_chunks_mut(CHUNK)
            .zip(input.par_chunks(CHUNK))
            .try_for_each(|(f, input)| -> Result<()> {
                checkpoint(cancel, Phase::Mapping)?;
                for (f, &x) in f.iter_mut().zip(input) {
                    *f = libm::frexp(x).0;
                }
                checkpoint(cancel, Phase::Mapping)
            })?,
        (None, Some(e)) => e
            .par_chunks_mut(CHUNK)
            .zip(input.par_chunks(CHUNK))
            .try_for_each(|(e, input)| -> Result<()> {
                checkpoint(cancel, Phase::Mapping)?;
                for (e, &x) in e.iter_mut().zip(input) {
                    *e = f64::from(libm::frexp(x).1);
                }
                checkpoint(cancel, Phase::Mapping)
            })?,
        (None, None) => {}
    }
    checkpoint(cancel, Phase::Mapping)
}

#[test]
fn aggregate_budget_and_omitted_slots_preflight_without_result_allocation() {
    let input = Matrix::from_column_vec(1, 3, vec![1., 2., 4.]).unwrap();
    let mut engine = Engine {
        evaluated_elements: EVALUATION_UNITS - 3,
        ..Default::default()
    };
    ALLOCATIONS.with(|n| n.set(0));
    assert!(dissect(&engine, &input, [true, true], Mode::Serial).is_err());
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
    for want in [[true, false], [false, true], [false, false]] {
        ALLOCATIONS.with(|n| n.set(0));
        let result = dissect(&engine, &input, want, Mode::Serial).unwrap();
        assert_eq!(
            ALLOCATIONS.with(Cell::get),
            usize::from(want[0]) + usize::from(want[1])
        );
        for (value, present) in result.iter().zip(want) {
            assert_eq!(matches!(value, Value::Matrix(_)), present);
            assert_eq!(
                value.allocation_units().unwrap_or(0),
                if present { 3 } else { 0 }
            );
        }
    }
    engine.evaluated_elements = EVALUATION_UNITS - 5;
    let negative = Value::Matrix(Matrix::from_column_vec(1, 3, vec![1., -1., 2.]).unwrap());
    ALLOCATIONS.with(|n| n.set(0));
    assert!(single(&engine, Base::Two, &negative, Mode::Serial).is_err());
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
    assert_eq!(engine.evaluated_elements, EVALUATION_UNITS - 5);
    assert!(preflight(&engine, usize::MAX, 2).is_err());
}

#[test]
fn cancelled_classification_allocation_mapping_publish_no_outputs_and_retry() {
    for (phase, remaining) in [
        (Phase::Classification, 1),
        (Phase::Allocation, 1),
        (Phase::Mapping, 1),
    ] {
        let mut engine = Engine::default();
        let input = Matrix::from_column_vec(1, CHUNK * 3, vec![2.; CHUNK * 3]).unwrap();
        engine
            .merge_workspace(std::collections::BTreeMap::from([
                ("A".into(), input.clone().into()),
                ("alias".into(), input.clone().into()),
            ]))
            .unwrap();
        engine.execute("result=17;F=19;E=23;").unwrap();
        let before = engine.workspace();
        CANCEL_AT.with(|slot| slot.set(Some((phase, remaining))));
        assert!(
            engine
                .execute("result=log10(A);")
                .unwrap_err()
                .contains("cancelled")
        );
        CANCEL_AT.with(|slot| slot.set(None));
        for (name, value) in &before {
            assert!(engine.array(name).unwrap().same_snapshot(value));
        }
        engine.cancel.store(false, Ordering::Relaxed);
        engine.execute("result=log2(A);").unwrap();
        assert!(engine.variables["result"].data().iter().all(|&x| x == 1.));
        assert!(engine.array("A").unwrap().same_snapshot(&before["A"]));
        CANCEL_AT.with(|slot| slot.set(Some((Phase::Mapping, 1))));
        assert!(
            engine
                .execute("[F,E]=log2(A);")
                .unwrap_err()
                .contains("cancelled")
        );
        CANCEL_AT.with(|slot| slot.set(None));
        assert!(engine.array("F").unwrap().same_snapshot(&before["F"]));
        assert!(engine.array("E").unwrap().same_snapshot(&before["E"]));
        engine.cancel.store(false, Ordering::Relaxed);
        engine.execute("[F,E]=log2(A);").unwrap();
        assert!(engine.variables["F"].data().iter().all(|&x| x == 0.5));
        assert!(engine.variables["E"].data().iter().all(|&x| x == 2.));
    }
}

#[test]
fn explicit_complex_exception_policy_and_analytic_extremes() {
    let values = [f64::NEG_INFINITY, -1., -0., 0., 1., f64::INFINITY, f64::NAN];
    for base in [Base::Two, Base::Ten] {
        for re in values {
            for im in values {
                let actual = complex_log(Complex64::new(re, im), base);
                if re.is_nan() || im.is_nan() {
                    assert!(actual.re.is_nan() && actual.im.is_nan());
                } else {
                    if re.is_infinite() || im.is_infinite() {
                        assert_eq!(actual.re, f64::INFINITY);
                    } else if re == 0. && im == 0. {
                        assert_eq!(actual.re, f64::NEG_INFINITY);
                    } else {
                        assert!(actual.re.is_finite());
                    }
                    assert!(actual.im.is_finite());
                    assert_eq!(actual.im.is_sign_negative(), im.is_sign_negative());
                    assert!(actual.im.abs() <= std::f64::consts::PI / base.divisor());
                }
            }
        }
    }
    // Analytic references, not old MATLAB's rounded-subnormal magnitude.
    let tiny = f64::from_bits(1);
    let tiny_pair = complex_log(Complex64::new(tiny, tiny), Base::Two);
    assert!((tiny_pair.re - (-1073.5)).abs() <= 5e-13);
    let huge_pair = complex_log(Complex64::new(f64::MAX, f64::MAX), Base::Two);
    assert!(huge_pair.re.is_finite());
    assert!((huge_pair.re - 1024.5).abs() <= 5e-13);
    for (x, y, angle) in [
        (0., 0., 0.),
        (0., -0., -0.),
        (-0., 0., std::f64::consts::PI),
        (-0., -0., -std::f64::consts::PI),
    ] {
        let result = complex_log(Complex64::new(x, y), Base::Two);
        assert_eq!(result.re, f64::NEG_INFINITY);
        assert_eq!(
            result.im.to_bits(),
            (angle / std::f64::consts::LN_2).to_bits()
        );
    }
}

fn same_float(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}

fn same_outputs(actual: &[Value], expected: &[Value]) {
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        match (actual, expected) {
            (Value::Matrix(a), Value::Matrix(b)) => {
                assert_eq!((a.rows(), a.cols()), (b.rows(), b.cols()));
                assert!(
                    a.data()
                        .iter()
                        .zip(b.data())
                        .all(|(&x, &y)| same_float(x, y))
                );
            }
            (Value::Complex(a), Value::Complex(b)) => {
                assert_eq!((a.rows(), a.cols()), (b.rows(), b.cols()));
                assert!(
                    a.data()
                        .iter()
                        .zip(b.data())
                        .all(|(x, y)| { same_float(x.re, y.re) && same_float(x.im, y.im) })
                );
            }
            (Value::Void, Value::Void) => {}
            _ => panic!("output storage class changed"),
        }
    }
}

#[test]
fn isolated_parallel_candidate_matches_serial_in_verified_contexts() {
    let input = Matrix::from_column_vec(
        1,
        CHUNK * 2 + 1,
        (0..CHUNK * 2 + 1).map(|i| (i as f64 - 100.) / 8.).collect(),
    )
    .unwrap();
    for workers in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            assert_eq!(rayon::current_num_threads(), workers);
            let engine = Engine::default();
            for want in [[true, true], [true, false], [false, true], [false, false]] {
                let reference = dissect(&engine, &input, want, Mode::Serial).unwrap();
                let candidate = dissect(&engine, &input, want, Mode::Parallel).unwrap();
                same_outputs(&candidate, &reference);
            }
            for base in [Base::Two, Base::Ten] {
                let value = Value::Matrix(input.clone());
                let reference = single(&engine, base, &value, Mode::Serial).unwrap();
                let candidate = single(&engine, base, &value, Mode::Parallel).unwrap();
                same_outputs(&[candidate], &[reference]);
                let complex = Value::Complex(
                    ComplexMatrix::from_column_vec(
                        1,
                        input.len(),
                        input
                            .data()
                            .iter()
                            .map(|&x| Complex64::new(x, -0.25))
                            .collect(),
                        &engine.cancel,
                    )
                    .unwrap(),
                );
                let reference = single(&engine, base, &complex, Mode::Serial).unwrap();
                let candidate = single(&engine, base, &complex, Mode::Parallel).unwrap();
                same_outputs(&[candidate], &[reference]);
            }
        });
    }
}

// Fixed measurement seam only. Root must approve the final schedule and bind
// actual hardware/source/unit/profile evidence before executing either order.
#[test]
#[ignore = "release-only forced serial/candidate strategy measurement"]
#[allow(clippy::assertions_on_constants)] // Keep the intentional release admission guard.
fn logarithm_release_measurements() {
    use std::{hint::black_box, time::Instant};
    assert!(
        !cfg!(debug_assertions),
        "measure only the release executable"
    );
    let reverse = match std::env::var("MATRUST_LOGARITHM_REVERSE") {
        Err(std::env::VarError::NotPresent) => false,
        Ok(value) if value == "0" => false,
        Ok(value) if value == "1" => true,
        _ => panic!("MATRUST_LOGARITHM_REVERSE must be absent,0,1"),
    };
    let mut groups = Vec::new();
    for workers in [1, 8] {
        for size in [1, 1024, 65_535, 65_536, 524_288, 1_000_000] {
            for scenario in 0..5 {
                groups.push((workers, size, scenario));
            }
        }
    }
    if reverse {
        groups.reverse();
    }
    let pools: Vec<_> = [1, 8]
        .into_iter()
        .map(|n| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(n)
                .build()
                .unwrap()
        })
        .collect();
    for (workers, size, scenario) in groups {
        let pool = &pools[usize::from(workers == 8)];
        pool.install(|| {
            assert_eq!(rayon::current_num_threads(), workers);
            let engine = Engine::default();
            let input = Matrix::from_column_vec(
                1,
                size,
                (0..size)
                    .map(|i| {
                        let x = 0.5 + (i % 2048) as f64 / 64.;
                        if scenario == 2 && i % 7 == 0 { -x } else { x }
                    })
                    .collect(),
            )
            .unwrap();
            let value = if scenario == 3 {
                Value::Complex(
                    ComplexMatrix::from_column_vec(
                        1,
                        size,
                        input
                            .data()
                            .iter()
                            .enumerate()
                            .map(|(i, &x)| Complex64::new(x, (i % 31) as f64 / 16. - 1.))
                            .collect(),
                        &engine.cancel,
                    )
                    .unwrap(),
                )
            } else {
                Value::Matrix(input.clone())
            };
            let name = [
                "real_log10",
                "real_log2",
                "promoted_log2",
                "complex_log2",
                "frexp_both",
            ][scenario];
            let run = |mode| -> Vec<Value> {
                if scenario == 4 {
                    dissect(&engine, black_box(&input), [true, true], mode).unwrap()
                } else {
                    vec![
                        single(
                            &engine,
                            if scenario == 0 { Base::Ten } else { Base::Two },
                            black_box(&value),
                            mode,
                        )
                        .unwrap(),
                    ]
                }
            };
            let expected = run(Mode::Serial);
            for rep in 0..9 {
                let modes = if rep % 2 == 0 {
                    [Mode::Serial, Mode::Parallel]
                } else {
                    [Mode::Parallel, Mode::Serial]
                };
                for mode in modes {
                    let start = Instant::now();
                    let output = black_box(run(mode));
                    let nanos = start.elapsed().as_nanos();
                    same_outputs(&output, &expected);
                    println!(
                        "LOGARITHM_MEASURE,{},{workers},{size},{name},{},{rep},{},{nanos}",
                        if reverse { "reverse" } else { "normal" },
                        if matches!(mode, Mode::Serial) {
                            "serial"
                        } else {
                            "parallel_candidate"
                        },
                        if rep < 2 { "warmup" } else { "retained" },
                    );
                }
            }
        });
    }
}

#[test]
fn automatic_single_dispatch_uses_only_measured_pool_and_size() {
    for workers in [1, 2, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            assert_eq!(rayon::current_num_threads(), workers);
            for len in [65_535, 65_536, 65_537] {
                let engine = Engine::default();
                let real = Value::Matrix(Matrix::from_column_vec(1, len, vec![2.; len]).unwrap());
                let complex = Value::Complex(
                    ComplexMatrix::from_column_vec(
                        1,
                        len,
                        vec![Complex64::new(2., -0.); len],
                        &engine.cancel,
                    )
                    .unwrap(),
                );
                for value in [&real, &complex] {
                    assert_eq!(
                        single_mode(value),
                        if workers == 8 && len >= 65_536 {
                            Mode::Parallel
                        } else {
                            Mode::Serial
                        }
                    );
                    if workers != 2 {
                        for (name, base) in [("log2", Base::Two), ("log10", Base::Ten)] {
                            let actual = engine
                                .logarithm_call(name, &[(*value).clone()])
                                .unwrap()
                                .unwrap();
                            let reference = single(&engine, base, value, Mode::Serial).unwrap();
                            same_outputs(&[actual], &[reference]);
                        }
                    }
                }
            }
        });
    }
}

#[test]
fn automatic_parallel_budget_and_cancellation_keep_publication_atomic() {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .build()
        .unwrap();
    pool.install(|| {
        assert_eq!(rayon::current_num_threads(), 8);
        let len = 65_536;
        let input = Matrix::from_column_vec(1, len, vec![-2.; len]).unwrap();
        let value = Value::Matrix(input.clone());
        assert_eq!(single_mode(&value), Mode::Parallel);
        let engine = Engine {
            evaluated_elements: EVALUATION_UNITS - len,
            ..Default::default()
        };
        ALLOCATIONS.with(|count| count.set(0));
        assert!(
            engine
                .logarithm_call("log2", &[value])
                .unwrap_err()
                .contains("budget")
        );
        assert_eq!(
            ALLOCATIONS.with(Cell::get),
            0,
            "promotion budget precedes allocation"
        );
        let mut engine = Engine::default();
        engine
            .merge_workspace(std::collections::BTreeMap::from([
                ("A".into(), input.clone().into()),
                ("alias".into(), input.into()),
            ]))
            .unwrap();
        engine.execute("result=17;").unwrap();
        let before = engine.workspace();
        // Allocation occurs on the initiating thread, before Rayon mapping.
        // This probe does not depend on which worker receives any map chunk.
        CANCEL_AT.with(|slot| slot.set(Some((Phase::Allocation, 1))));
        assert!(
            engine
                .execute("result=log10(A);")
                .unwrap_err()
                .contains("cancelled")
        );
        CANCEL_AT.with(|slot| slot.set(None));
        for (name, value) in &before {
            assert!(engine.array(name).unwrap().same_snapshot(value));
        }
        engine.cancel.store(false, Ordering::Relaxed);
        engine.execute("result=log10(A);").unwrap();
        assert!(engine.array("A").unwrap().same_snapshot(&before["A"]));
        assert!(
            engine
                .array("alias")
                .unwrap()
                .same_snapshot(&before["alias"])
        );

        // Any worker that encounters index zero requests cancellation. No
        // thread-local injection or timing assumption is used for map checks.
        let cancel = AtomicBool::new(false);
        let result = build(len, &cancel, Mode::Parallel, |index| {
            if index == 0 {
                cancel.store(true, Ordering::Relaxed);
            }
            index as f64
        });
        assert!(result.unwrap_err().contains("cancelled"));
        assert!(cancel.load(Ordering::Relaxed));
    });
}

#[test]
fn compensated_near_unit_guards_and_exact_axis_tail() {
    let boundary = f64::from_bits((1023_u64 - 450) << 52);
    assert!(compensated_near_unit(1., boundary.next_down()).is_none());
    assert!(compensated_near_unit(1., boundary).is_some());
    assert!(compensated_near_unit(1., boundary.next_up()).is_some());
    assert!(compensated_near_unit(0.5, 0.5).is_some());
    assert!(compensated_near_unit(0.5_f64.next_down(), 0.5_f64.next_down()).is_none());
    assert!(compensated_near_unit(0.5_f64.next_up(), 0.5_f64.next_up()).is_some());
    assert!(compensated_near_unit(1., 1.).is_some());
    assert!(compensated_near_unit(1_f64.next_down(), 1_f64.next_down()).is_some());
    assert!(compensated_near_unit(1_f64.next_up(), 1_f64.next_up()).is_none());
    // log(sqrt(1+2^-900)) rounds to 2^-901: the first omitted
    // Taylor term is vastly smaller than half a binary64 ULP here.
    assert_eq!(
        compensated_near_unit(1., boundary).unwrap().to_bits(),
        ((1023_u64 - 901) << 52)
    );
    for zero in [0., -0.] {
        assert!(compensated_near_unit(1., zero).is_none());
        assert_eq!(finite_log_magnitude(1., zero).to_bits(), 0_f64.to_bits());
        assert_eq!(finite_log_magnitude(zero, -1.).to_bits(), 0_f64.to_bits());
        for base in [Base::Two, Base::Ten] {
            let z = complex_log(Complex64::new(1., zero), base);
            assert_eq!(z.re.to_bits(), 0_f64.to_bits());
            assert_eq!(z.im.to_bits(), zero.to_bits());
        }
    }
}
