use matrust::{Complex64, ComplexMatrix, Matrix, complex_matrix::MAX_COMPLEX_ELEMENTS};
use std::sync::atomic::AtomicBool;

fn bits(values: &[Complex64]) -> Vec<[u64; 2]> {
    values
        .iter()
        .map(|v| [v.re.to_bits(), v.im.to_bits()])
        .collect()
}

#[test]
fn complex_storage_owned_transfer_alias_reshape_and_bounds() {
    let cancel = AtomicBool::new(false);
    let values = vec![
        Complex64::new(1., 2.),
        Complex64::new(3., -4.),
        Complex64::new(5., 6.),
        Complex64::new(7., 8.),
    ];
    let pointer = values.as_ptr();
    let source = ComplexMatrix::from_column_vec(2, 2, values, &cancel).unwrap();
    assert_eq!(source.data().as_ptr(), pointer);
    assert_eq!(source.bytes(), 64);
    assert_eq!(source.get(1, 0).unwrap(), Complex64::new(3., -4.));
    let alias = source.clone();
    let row = source.reshape(1, 4, &cancel).unwrap();
    assert!(source.same_snapshot(&alias));
    assert!(source.shares_storage(&row));
    assert!(!source.same_snapshot(&row));
    assert!(source.get(2, 0).is_err());
    assert!(source.reshape(1, 3, &cancel).is_err());
    assert_eq!(
        ComplexMatrix::checked_len(MAX_COMPLEX_ELEMENTS, 1).unwrap(),
        MAX_COMPLEX_ELEMENTS
    );
    assert!(ComplexMatrix::checked_len(MAX_COMPLEX_ELEMENTS + 1, 1).is_err());
    assert!(ComplexMatrix::checked_len(usize::MAX, 2).is_err());
    assert!(ComplexMatrix::checked_len(usize::MAX, 0).is_err());
    assert!(ComplexMatrix::from_column_vec(2, 2, vec![], &cancel).is_err());
}

#[test]
fn complex_component_copy_preserves_zero_nan_payloads_and_expansion() {
    let cancel = AtomicBool::new(false);
    let re = Matrix::from_column_vec(
        2,
        2,
        vec![
            -0.,
            f64::from_bits(1),
            f64::from_bits(0x7ff8_0000_0000_0041),
            f64::INFINITY,
        ],
    )
    .unwrap();
    let im = Matrix::from_column_vec(
        2,
        2,
        vec![0., -0., f64::from_bits(0xfff8_0000_0000_0081), -3.],
    )
    .unwrap();
    let source = ComplexMatrix::from_parts(&re, &im, &cancel).unwrap();
    assert_eq!(
        source
            .real(&cancel)
            .unwrap()
            .data()
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        re.data().iter().map(|v| v.to_bits()).collect::<Vec<_>>()
    );
    assert_eq!(
        source
            .imag(&cancel)
            .unwrap()
            .data()
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        im.data().iter().map(|v| v.to_bits()).collect::<Vec<_>>()
    );
    let conjugate = source.conj(&cancel).unwrap();
    for (a, b) in source.data().iter().zip(conjugate.data()) {
        assert_eq!(a.re.to_bits(), b.re.to_bits());
        assert_eq!(a.im.to_bits() ^ (1u64 << 63), b.im.to_bits());
    }
    assert_eq!(
        bits(source.data()),
        bits(conjugate.conj(&cancel).unwrap().data())
    );
    let left = ComplexMatrix::from_parts(&Matrix::scalar(9.), &im, &cancel).unwrap();
    let right = ComplexMatrix::from_parts(&re, &Matrix::scalar(-0.), &cancel).unwrap();
    assert!(left.data().iter().all(|v| v.re == 9.));
    assert!(
        right
            .data()
            .iter()
            .all(|v| v.im.to_bits() == (-0f64).to_bits())
    );
    let real = ComplexMatrix::from_real(&re, &cancel).unwrap();
    assert!(real.data().iter().all(|v| v.im.to_bits() == 0));
    assert!(ComplexMatrix::from_parts(&re, &Matrix::filled(1, 3, 0.).unwrap(), &cancel).is_err());
}

#[test]
fn complex_empty_shapes_and_vector_transpose_share_only_valid_storage() {
    let cancel = AtomicBool::new(false);
    for (r, c) in [(0, 0), (0, 7), (7, 0), (0, 1), (1, 0)] {
        let a = ComplexMatrix::filled(r, c, Complex64::new(0., -0.), &cancel).unwrap();
        for b in [a.transpose(&cancel).unwrap(), a.adjoint(&cancel).unwrap()] {
            assert_eq!((b.rows(), b.cols()), (c, r));
            assert!(a.shares_storage(&b));
        }
        let real = Matrix::filled(r, c, 0.).unwrap();
        let expanded = ComplexMatrix::from_parts(&real, &Matrix::scalar(3.), &cancel).unwrap();
        assert_eq!(
            (expanded.rows(), expanded.cols(), expanded.len()),
            (r, c, 0)
        );
    }
    for (r, c) in [(1, 1), (1, 9), (9, 1)] {
        let a = ComplexMatrix::filled(r, c, Complex64::new(3., 4.), &cancel).unwrap();
        assert!(a.shares_storage(&a.transpose(&cancel).unwrap()));
        let b = a.adjoint(&cancel).unwrap();
        assert!(!a.shares_storage(&b));
        assert!(b.data().iter().all(|v| *v == Complex64::new(3., -4.)));
    }
}

#[test]
fn complex_rectangular_transforms_are_column_major_and_worker_bit_identical() {
    let cancel = AtomicBool::new(false);
    let pools = [1, 8].map(|n| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .unwrap()
    });
    for (r, c) in [(2_usize, 3_usize), (4097, 1), (1025, 513)] {
        let values: Vec<_> = (0..r * c)
            .map(|i| {
                Complex64::new(
                    i as f64,
                    if i.is_multiple_of(17) {
                        -0.
                    } else {
                        -(i as f64)
                    },
                )
            })
            .collect();
        let a = ComplexMatrix::from_column_vec(r, c, values, &cancel).unwrap();
        for conjugate in [false, true] {
            let outputs = pools.each_ref().map(|pool| {
                pool.install(|| {
                    if conjugate {
                        a.adjoint(&cancel)
                    } else {
                        a.transpose(&cancel)
                    }
                })
                .unwrap()
            });
            assert_eq!(bits(outputs[0].data()), bits(outputs[1].data()));
            assert_eq!((outputs[0].rows(), outputs[0].cols()), (c, r));
            for row in 0..r {
                for col in 0..c {
                    let before = a.get(row, col).unwrap();
                    let after = outputs[0].get(col, row).unwrap();
                    assert_eq!(before.re.to_bits(), after.re.to_bits());
                    assert_eq!(
                        before.im.to_bits() ^ if conjugate { 1u64 << 63 } else { 0 },
                        after.im.to_bits()
                    );
                }
            }
        }
        let copies = pools.each_ref().map(|pool| {
            pool.install(|| ComplexMatrix::from_column_slice(r, c, a.data(), &cancel))
                .unwrap()
        });
        assert_eq!(bits(copies[0].data()), bits(copies[1].data()));
        assert_eq!(bits(a.data()), bits(copies[0].data()));
        for imaginary in [false, true] {
            let projections = pools.each_ref().map(|pool| {
                pool.install(|| {
                    if imaginary {
                        a.imag(&cancel)
                    } else {
                        a.real(&cancel)
                    }
                })
                .unwrap()
            });
            assert_eq!(
                projections[0]
                    .data()
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>(),
                projections[1]
                    .data()
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>()
            );
            for (before, after) in a.data().iter().zip(projections[0].data()) {
                assert_eq!(
                    (if imaginary { before.im } else { before.re }).to_bits(),
                    after.to_bits()
                );
            }
        }
    }
}

#[test]
fn complex_precancellation_blocks_noop_and_allocation_paths() {
    let no = AtomicBool::new(false);
    let stop = AtomicBool::new(true);
    let a = ComplexMatrix::scalar(Complex64::new(2., 3.), &no).unwrap();
    let snapshot = a.clone();
    assert!(a.reshape(1, 1, &stop).is_err());
    assert!(a.transpose(&stop).is_err());
    assert!(a.adjoint(&stop).is_err());
    assert!(a.conj(&stop).is_err());
    assert!(a.real(&stop).is_err());
    assert!(a.imag(&stop).is_err());
    assert!(ComplexMatrix::from_column_vec(1, 1, vec![Complex64::new(1., 2.)], &stop).is_err());
    assert!(ComplexMatrix::from_parts(&Matrix::scalar(1.), &Matrix::scalar(2.), &stop).is_err());
    assert!(a.same_snapshot(&snapshot));
}
