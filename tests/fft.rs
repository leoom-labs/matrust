use matrust::{
    Array, Complex64, ComplexMatrix, Engine, LogicalMatrix, Matrix,
    dsp::{self, FftOptions, TransformDirection},
};
use std::sync::atomic::{AtomicBool, Ordering};
const BUDGET: usize = 256 * 1024 * 1024;

fn values(a: &Array) -> Vec<Complex64> {
    match a {
        Array::Numeric(a) => a.data().iter().map(|&x| Complex64::new(x, 0.)).collect(),
        Array::Complex(a) => a.data().to_vec(),
        Array::Logical(a) => a
            .data()
            .iter()
            .map(|&x| Complex64::new(f64::from(x), 0.))
            .collect(),
        _ => panic!("numeric result expected"),
    }
}
fn compare(a: &[Complex64], b: &[Complex64], tolerance: f64) {
    assert_eq!(a.len(), b.len());
    for (a, b) in a.iter().zip(b) {
        assert!(
            (a.re - b.re).abs() <= tolerance * (1. + b.re.abs()),
            "real {a:?} versus {b:?}"
        );
        assert!(
            (a.im - b.im).abs() <= tolerance * (1. + b.im.abs()),
            "imaginary {a:?} versus {b:?}"
        );
    }
}
// Independent O(N^2) definition ONLY for small test vectors; not a production
// transform and not a RustFFT-internal reference. Includes explicit coordinate
// gathering so an identical production scatter bug cannot validate itself.
fn definition(a: &Array, n: usize, dim: usize, inverse: bool) -> Vec<Complex64> {
    let input = values(a);
    let (rows, cols) = if dim == 1 {
        (n, a.cols())
    } else {
        (a.rows(), n)
    };
    let mut output = vec![Complex64::new(0., 0.); rows * cols];
    for column in 0..cols {
        for row in 0..rows {
            let frequency = if dim == 1 { row } else { column };
            let mut sum = Complex64::new(0., 0.);
            for t in 0..n {
                let (r, c) = if dim == 1 { (t, column) } else { (row, t) };
                if r < a.rows() && c < a.cols() {
                    let angle = (if inverse { 1. } else { -1. })
                        * std::f64::consts::TAU
                        * (frequency * t) as f64
                        / n as f64;
                    sum += input[c * a.rows() + r] * Complex64::new(angle.cos(), angle.sin());
                }
            }
            output[column * rows + row] = if inverse { sum / n as f64 } else { sum };
        }
    }
    output
}

#[test]
fn typed_two_axis_padded_truncated_prime_transforms_match_independent_definition() {
    let cancel = AtomicBool::new(false);
    let inputs = [
        Array::Numeric(
            Matrix::from_column_vec(3, 5, (0..15).map(|i| (i as f64 - 4.) / 8.).collect()).unwrap(),
        ),
        Array::Logical(
            LogicalMatrix::from_column_vec(
                3,
                5,
                (0usize..15)
                    .map(|i| u8::from(i.is_multiple_of(3)))
                    .collect(),
                &cancel,
            )
            .unwrap(),
        ),
        Array::Complex(
            ComplexMatrix::from_column_vec(
                3,
                5,
                (0..15)
                    .map(|i| Complex64::new((i as f64 - 4.) / 8., ((i * 7) % 11) as f64 / 16.))
                    .collect(),
                &cancel,
            )
            .unwrap(),
        ),
    ];
    for input in inputs {
        let alias = input.clone();
        let original = values(&input);
        for dim in [1, 2] {
            for n in [0, 1, 2, 3, 5, 8, 17, 31] {
                for direction in [TransformDirection::Forward, TransformDirection::Inverse] {
                    let out = dsp::fourier(
                        &input,
                        FftOptions {
                            length: Some(n),
                            dimension: Some(dim),
                        },
                        direction,
                        BUDGET,
                        &cancel,
                    )
                    .unwrap();
                    assert_eq!(
                        (out.rows(), out.cols()),
                        if dim == 1 { (n, 5) } else { (3, n) }
                    );
                    compare(
                        &values(&out),
                        &definition(&input, n, dim, direction == TransformDirection::Inverse),
                        2e-12,
                    );
                }
            }
        }
        assert!(input.same_snapshot(&alias));
        for (a, b) in values(&input).iter().zip(original) {
            assert_eq!(a.re.to_bits(), b.re.to_bits());
            assert_eq!(a.im.to_bits(), b.im.to_bits());
        }
    }
}

#[test]
fn inverse_real_class_requires_exact_effective_symmetry_in_every_lane() {
    let mut e = Engine::default();
    e.execute("A=[10 4; -2+2i 1+3i; -2 8; -2-2i 1-3i]; R=ifft(A); C=ifft([A [1;2;3;4]]); Row=ifft(A.',[],2); Pad=ifft([2 0],4); Trunc=ifft([2 0 3i],2); Almost=ifft([2 1+1i 0 1-1.0000000000000002i]);").unwrap();
    assert!(e.array("R").unwrap().is_numeric_array());
    assert!(matches!(e.array("R"), Some(Array::Numeric(_))));
    assert!(matches!(e.array("Row"), Some(Array::Numeric(_))));
    assert!(matches!(e.array("Pad"), Some(Array::Numeric(_))));
    assert!(matches!(e.array("Trunc"), Some(Array::Numeric(_))));
    assert!(matches!(e.array("C"), Some(Array::Complex(_))));
    assert!(matches!(e.array("Almost"), Some(Array::Complex(_))));
    compare(
        &values(&e.array("R").unwrap()),
        &definition(&e.array("A").unwrap(), 4, 1, true),
        1e-13,
    );
    e.execute("Special=ifft([Inf 0]);").unwrap();
    assert!(matches!(e.array("Special"), Some(Array::Complex(_))));
}

#[test]
fn default_dimension_empty_shapes_known_quarter_turn_and_round_trip() {
    let mut e = Engine::default();
    e.execute("A=[1 2 3 4]; Alias=A; F=fft(A); C=fft(A.'); R=ifft(F); E=fft([]); E03=fft(zeros(0,3)); E30=ifft(zeros(3,0)); EP=fft(zeros(0,3),4); Z=fft(A,0); S=fft(3); L=fft(logical([1 0 1 0])); LS=fft(A,true); LD=fft(A,[],true);").unwrap();
    compare(
        &values(&e.array("F").unwrap()),
        &[
            Complex64::new(10., 0.),
            Complex64::new(-2., 2.),
            Complex64::new(-2., 0.),
            Complex64::new(-2., -2.),
        ],
        1e-14,
    );
    compare(
        &values(&e.array("R").unwrap()),
        &values(&e.array("A").unwrap()),
        1e-14,
    );
    assert_eq!(
        (e.array("C").unwrap().rows(), e.array("C").unwrap().cols()),
        (4, 1)
    );
    compare(
        &values(&e.array("LS").unwrap()),
        &[Complex64::new(1., 0.)],
        0.,
    );
    compare(
        &values(&e.array("LD").unwrap()),
        &values(&e.array("A").unwrap()),
        0.,
    );
    for (name, shape) in [
        ("E", (0, 0)),
        ("E03", (0, 3)),
        ("E30", (3, 0)),
        ("EP", (4, 3)),
        ("Z", (1, 0)),
    ] {
        let a = e.array(name).unwrap();
        assert_eq!((a.rows(), a.cols()), shape);
        assert!(values(&a).iter().all(|z| z.re == 0. && z.im == 0.));
    }
    assert!(
        e.array("A")
            .unwrap()
            .same_snapshot(&e.array("Alias").unwrap())
    );
}

#[test]
fn unsupported_shapes_options_and_budget_errors_preserve_destination_and_aliases() {
    let mut e = Engine::default();
    e.execute("saved=[7 8];Alias=saved;").unwrap();
    for expression in [
        "fft([1 2],-1)",
        "fft([1 2],1.5)",
        "fft([1 2],Inf)",
        "fft([1 2],1048577)",
        "fft([1 2],[],3)",
        "fft([1 2],[],0)",
        "ifft([1 2],'symmetric')",
        "ifft([1 2],[],1,'nonsymmetric')",
        "fft('abc')",
        "fft({1})",
        "fft([1 2],1i)",
    ] {
        assert!(
            e.execute(&format!("saved={expression};")).is_err(),
            "{expression}"
        );
        assert!(
            e.array("saved")
                .unwrap()
                .same_snapshot(&e.array("Alias").unwrap())
        );
    }
    let input: Array = Matrix::scalar(1.).into();
    assert!(
        dsp::fourier(
            &input,
            FftOptions::default(),
            TransformDirection::Forward,
            1,
            &AtomicBool::new(false)
        )
        .unwrap_err()
        .contains("budget")
    );
    let stopped = AtomicBool::new(true);
    assert_eq!(
        dsp::fourier(
            &input,
            FftOptions::default(),
            TransformDirection::Forward,
            BUDGET,
            &stopped
        )
        .unwrap_err(),
        "Calculation cancelled"
    );
    stopped.store(false, Ordering::Relaxed);
    assert!(
        dsp::fourier(
            &input,
            FftOptions::default(),
            TransformDirection::Forward,
            BUDGET,
            &stopped
        )
        .is_ok()
    );
}

#[test]
fn local_functions_keep_precedence_over_builtins() {
    let mut e = Engine::default();
    e.execute("R=fft(2);S=ifft(3);function y=fft(x);y=x+7;end;function y=ifft(x);y=x+9;end")
        .unwrap();
    assert_eq!(e.variables["R"].data(), &[9.]);
    assert_eq!(e.variables["S"].data(), &[12.]);
}

#[test]
fn large_prime_and_power_of_two_round_trips_match_in_one_and_eight_worker_pools() {
    let pools = [1, 8].map(|workers| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
    });
    let cancel = AtomicBool::new(false);
    for (rows, cols, dim) in [(65521, 1, 1), (32768, 9, 1), (9, 32768, 2)] {
        let input: Array = ComplexMatrix::from_column_vec(
            rows,
            cols,
            (0..rows * cols)
                .map(|i| Complex64::new(((i * 3) % 31) as f64 / 16., ((i * 7) % 17) as f64 / 8.))
                .collect(),
            &cancel,
        )
        .unwrap()
        .into();
        let alias = input.clone();
        let mut first: Option<Vec<Complex64>> = None;
        for pool in &pools {
            let options = FftOptions {
                length: None,
                dimension: Some(dim),
            };
            let transformed = pool
                .install(|| {
                    dsp::fourier(
                        &input,
                        options,
                        TransformDirection::Forward,
                        BUDGET,
                        &cancel,
                    )
                })
                .unwrap();
            if let Some(expected) = &first {
                compare(&values(&transformed), expected, 0.);
            } else {
                first = Some(values(&transformed));
            }
            let restored = pool
                .install(|| {
                    dsp::fourier(
                        &transformed,
                        options,
                        TransformDirection::Inverse,
                        BUDGET,
                        &cancel,
                    )
                })
                .unwrap();
            compare(&values(&restored), &values(&input), 2e-10);
            assert!(input.same_snapshot(&alias));
        }
    }
}
