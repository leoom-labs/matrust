use matrust::{Array, Complex64, LogicalMatrix, Matrix, Scalar};
use std::sync::atomic::AtomicBool;

#[test]
fn typed_scalar_observation_preserves_kind_bits_and_requires_explicit_projection() {
    for z in [
        Complex64::new(-0., 0.),
        Complex64::new(3., -4.),
        Complex64::new(f64::from_bits(0x7ff8_0000_0000_0031), -0.),
        Complex64::new(f64::INFINITY, f64::NEG_INFINITY),
    ] {
        let scalar = Scalar::Complex(z);
        assert!(scalar.is_complex());
        assert!(scalar.into_real().is_err());
        let roundtrip = scalar.into_complex();
        assert_eq!(roundtrip.re.to_bits(), z.re.to_bits());
        assert_eq!(roundtrip.im.to_bits(), z.im.to_bits());
    }
    assert_eq!(
        Scalar::Real(-0.).into_real().unwrap().to_bits(),
        (-0f64).to_bits()
    );
    assert_eq!(Scalar::Logical(true).into_real().unwrap(), 1.);
    assert_eq!(Scalar::Complex(Complex64::new(3., -4.)).to_string(), "3-4i");
    assert_eq!(Scalar::Complex(Complex64::new(3., -0.)).to_string(), "3-0i");
    assert_eq!(
        format!("{:.2}", Scalar::Complex(Complex64::new(3., 4.))),
        "3.00+4.00i"
    );
    assert_eq!(format!("{:.2}", Scalar::Logical(true)), "1.00");
}

#[test]
fn typed_array_cell_api_retains_real_and_logical_behavior_without_complex_loss() {
    let cancel = AtomicBool::new(false);
    let numeric: Array = Matrix::from_column_vec(2, 2, vec![1., -0., 3., 4.])
        .unwrap()
        .into();
    let original = numeric.clone();
    assert!(
        matches!(numeric.scalar(1, 0).unwrap(), Scalar::Real(x) if x.to_bits() == (-0f64).to_bits())
    );
    assert!(numeric.scalar(2, 0).is_err());
    assert!(numeric.scalar(0, 2).is_err());
    let edited = numeric
        .with_scalar(1, 1, Scalar::Real(9.), &cancel)
        .unwrap();
    assert_eq!(edited.scalar(1, 1).unwrap().into_real().unwrap(), 9.);
    assert!(numeric.same_snapshot(&original));
    for imaginary in [0., -0., 1.] {
        assert!(
            numeric
                .with_scalar(
                    0,
                    0,
                    Scalar::Complex(Complex64::new(2., imaginary)),
                    &cancel
                )
                .is_err()
        );
        assert!(numeric.same_snapshot(&original));
    }
    let logical: Array = LogicalMatrix::from_column_vec(1, 2, vec![0, 1], &cancel)
        .unwrap()
        .into();
    assert!(matches!(
        logical.scalar(0, 0).unwrap(),
        Scalar::Logical(false)
    ));
    assert!(matches!(
        logical.scalar(0, 1).unwrap(),
        Scalar::Logical(true)
    ));
    let logical_alias = logical.clone();
    assert!(
        logical
            .with_scalar(0, 0, Scalar::Real(f64::NAN), &cancel)
            .unwrap_err()
            .contains("NaN")
    );
    assert!(logical.same_snapshot(&logical_alias));
    let changed = logical
        .with_scalar(0, 0, Scalar::Real(-2.), &cancel)
        .unwrap();
    assert!(matches!(
        changed.scalar(0, 0).unwrap(),
        Scalar::Logical(true)
    ));
    let empty: Array = Matrix::filled(0, 4, 0.).unwrap().into();
    assert!(empty.scalar(0, 0).is_err());
    assert!(
        numeric
            .with_scalar(0, 0, Scalar::Real(2.), &AtomicBool::new(true))
            .is_err()
    );
}
