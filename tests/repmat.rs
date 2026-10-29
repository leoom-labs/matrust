use matrust::{Array, Complex64, ComplexMatrix, Engine, LogicalMatrix, Matrix};
use std::sync::atomic::{AtomicBool, Ordering};

fn verify(input: &Array, output: &Array, rr: usize, cc: usize) {
    assert_eq!(
        (output.rows(), output.cols()),
        (input.rows() * rr, input.cols() * cc)
    );
    assert_eq!(output.is_complex(), input.is_complex());
    assert_eq!(output.is_logical(), input.is_logical());
    for column in 0..output.cols() {
        for row in 0..output.rows() {
            let i = (column % input.cols()) * input.rows() + row % input.rows();
            let j = column * output.rows() + row;
            match (input, output) {
                (Array::Numeric(a), Array::Numeric(b)) => {
                    assert_eq!(a.data()[i].to_bits(), b.data()[j].to_bits())
                }
                (Array::Logical(a), Array::Logical(b)) => assert_eq!(a.data()[i], b.data()[j]),
                (Array::Complex(a), Array::Complex(b)) => {
                    assert_eq!(a.data()[i].re.to_bits(), b.data()[j].re.to_bits());
                    assert_eq!(a.data()[i].im.to_bits(), b.data()[j].im.to_bits());
                }
                _ => panic!("type changed"),
            }
        }
    }
}

#[test]
fn repetition_preserves_exact_typed_bits_and_column_major_block_order() {
    let cancel = AtomicBool::new(false);
    let values = [
        -0.,
        f64::from_bits(0x7ff8000000001234),
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::from_bits(1),
        3.,
    ];
    let sources = [
        Array::Numeric(Matrix::from_column_slice(2, 3, &values).unwrap()),
        Array::Logical(
            LogicalMatrix::from_column_vec(2, 3, vec![0, 1, 1, 0, 1, 0], &cancel).unwrap(),
        ),
        Array::Complex(
            ComplexMatrix::from_column_vec(
                2,
                3,
                values
                    .iter()
                    .enumerate()
                    .map(|(i, &re)| Complex64::new(re, values[5 - i]))
                    .collect(),
                &cancel,
            )
            .unwrap(),
        ),
    ];
    for a in sources {
        for (rr, cc) in [(1, 1), (2, 3), (3, 1), (1, 4), (0, 4), (4, 0)] {
            let out = a.repeated(rr, cc, &cancel).unwrap();
            verify(&a, &out, rr, cc);
            if (rr, cc) == (1, 1) {
                assert!(a.same_snapshot(&out));
            }
        }
    }
}

#[test]
fn repetition_handles_non_power_periods_and_chunk_boundaries_in_both_axes() {
    let cancel = AtomicBool::new(false);
    for rows in [1, 3, 4095, 4096, 4097, 8193] {
        let source: Array =
            Matrix::from_column_vec(rows, 3, (0..rows * 3).map(|i| i as f64).collect())
                .unwrap()
                .into();
        for (rr, cc) in [(1, 3), (2, 3), (3, 2)] {
            verify(&source, &source.repeated(rr, cc, &cancel).unwrap(), rr, cc);
        }
    }
}

#[test]
fn script_repetition_shapes_negative_factors_aliases_and_atomic_errors() {
    let mut e = Engine::default();
    e.execute("A=[1 2;3 4];B=repmat(A,2);C=repmat(A,[2 3]);D=repmat(A,2,3,1);L=repmat(logical(A),[1 2]);Z=repmat(complex(A,-A),2,1);Alias=repmat(A,1,1);N=repmat(A,-2,3);E=repmat(zeros(0,0),[16777216 16777216]);saved=9;").unwrap();
    assert_eq!(
        e.variables["B"].data(),
        &[
            1., 3., 1., 3., 2., 4., 2., 4., 1., 3., 1., 3., 2., 4., 2., 4.
        ]
    );
    assert_eq!((e.variables["C"].rows(), e.variables["C"].cols()), (4, 6));
    assert_eq!(e.variables["C"].data(), e.variables["D"].data());
    assert_eq!((e.variables["N"].rows(), e.variables["N"].cols()), (0, 6));
    assert_eq!((e.variables["E"].rows(), e.variables["E"].cols()), (0, 0));
    assert_eq!(
        e.variables["A"].data().as_ptr(),
        e.variables["Alias"].data().as_ptr()
    );
    assert_eq!(
        (
            e.logical_variables["L"].rows(),
            e.logical_variables["L"].cols()
        ),
        (2, 4)
    );
    assert_eq!(e.complex_variables["Z"].data()[0].im, -1.);
    e.execute("Alias(1)=100;").unwrap();
    assert_eq!(e.variables["A"].data()[0], 1.);
    for rhs in [
        "repmat(A)",
        "repmat(A,[])",
        "repmat(A,[2;3])",
        "repmat(A,[],2)",
        "repmat(A,2,3,2)",
        "repmat(A,-0.5,2)",
        "repmat(A,Inf,0)",
        "repmat(A,NaN,1)",
        "repmat(A,16777217,0)",
        "repmat(A,4096,4096)",
        "repmat(zeros(0,4),1,16777216)",
        "repmat(complex(1),4096,4096)",
        "repmat('text',4096,4096)",
    ] {
        assert!(
            e.execute(&format!("saved={rhs};")).is_err(),
            "accepted {rhs}"
        );
        assert_eq!(e.variables["saved"].data(), &[9.]);
    }
}

#[test]
fn repeated_empty_shapes_and_typed_limits_are_checked_before_copying() {
    let cancel = AtomicBool::new(false);
    for (r, c) in [(0, 0), (0, 3), (4, 0)] {
        let sources = [
            Array::Numeric(Matrix::filled(r, c, 0.).unwrap()),
            Array::Logical(LogicalMatrix::from_column_vec(r, c, vec![], &cancel).unwrap()),
            Array::Complex(ComplexMatrix::from_column_vec(r, c, vec![], &cancel).unwrap()),
        ];
        for a in sources {
            for (rr, cc) in [(2, 3), (0, 2), (2, 0)] {
                verify(&a, &a.repeated(rr, cc, &cancel).unwrap(), rr, cc);
            }
            assert!(a.repeated(usize::MAX, 1, &cancel).is_err());
        }
    }
    let complex: Array =
        ComplexMatrix::from_column_vec(1, 1, vec![Complex64::new(1., 2.)], &cancel)
            .unwrap()
            .into();
    assert!(complex.repeated_shape(4096, 4096).is_err());
    let real: Array = Matrix::scalar(1.).into();
    assert_eq!(real.repeated_shape(4096, 4096).unwrap(), (4096, 4096));
}

#[test]
fn repetition_is_deterministic_in_one_and_eight_worker_contexts_and_cancellable() {
    let cancel = AtomicBool::new(false);
    let a: Array = Matrix::from_column_vec(4097, 2, (0..8194).map(|i| i as f64).collect())
        .unwrap()
        .into();
    let mut previous = None;
    for workers in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        let out = pool.install(|| a.repeated(8, 8, &cancel)).unwrap();
        verify(&a, &out, 8, 8);
        let data = out.as_numeric().unwrap().data();
        if let Some(ref old) = previous {
            assert_eq!(data, old);
        } else {
            previous = Some(data.to_vec());
        }
    }
    cancel.store(true, Ordering::Relaxed);
    assert!(a.repeated(1, 1, &cancel).unwrap_err().contains("cancelled"));
    assert!(a.repeated(8, 8, &cancel).unwrap_err().contains("cancelled"));
    cancel.store(false, Ordering::Relaxed);
    verify(&a, &a.repeated(2, 3, &cancel).unwrap(), 2, 3);
}

#[test]
fn complex_v73_fixture_repetition_has_exact_large_shape_and_pattern() {
    let mut e = Engine::default();
    e.execute("Z=complex([1 2 4;-0 3 5],[7 8 10;-0 9 11]);LargeZ=complex(repmat(real(Z),1000,100),repmat(imag(Z),1000,100));").unwrap();
    let a: Array = e.complex_variables["Z"].clone().into();
    let b: Array = e.complex_variables["LargeZ"].clone().into();
    verify(&a, &b, 1000, 100);
    assert_eq!((b.rows(), b.cols(), b.bytes()), (2000, 300, 9_600_000));
}
