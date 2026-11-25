use matrust::{Array, Complex64, Engine};

fn eval(code: &str) -> Engine {
    let mut engine = Engine::default();
    engine
        .execute(code)
        .unwrap_or_else(|err| panic!("{code}: {err}"));
    engine
}

#[test]
fn w8_logical_constructors_accept_column_size() {
    // GNU Octave 10.3: size(true([2;3])) and size(false([2;3])) are [2 3].
    let mut e = eval("t=true([2;3]); f=false([2;3]);");
    for (name, fill) in [("t", 1), ("f", 0)] {
        let a = &e.logical_variables[name];
        assert_eq!((a.rows(), a.cols()), (2, 3));
        assert_eq!(a.data(), &[fill; 6]);
    }
    assert!(e.execute("true([2 3;4 5])").is_err());
}

#[test]
fn w9_zero_single_colon_step_returns_empty_row() {
    // GNU Octave 10.3 returns an empty single row.
    let e = eval("a=colon(single(1),0,single(2));");
    let a = e.array("a").unwrap();
    assert_eq!((a.rows(), a.cols(), a.class()), (1, 0, "single"));
}

#[test]
fn w10_single_colon_stops_at_exact_bound() {
    // GNU Octave 10.3 includes the endpoint exactly once in both directions.
    let e = eval(
        "a=colon(single(1),single(1),single(2097153)); b=single(2097153):single(-1):single(1);",
    );
    for (name, first, last) in [("a", 1., 2097153.), ("b", 2097153., 1.)] {
        let Some(Array::Typed(a)) = e.array(name) else {
            panic!("expected single row")
        };
        assert_eq!((a.rows(), a.cols()), (1, 2097153));
        assert_eq!((a.real(0), a.real(a.len() - 1)), (first, last));
    }
}

#[test]
fn f6_single_decimal_ranges_keep_their_endpoint() {
    // GNU Octave 10.3 counts single ranges with a 3-eps tolerant floor:
    // single(0):single(0.3):single(0.9) has 4 elements, 0:0.05:0.45 has 10.
    let mut e = eval("a=single(0):single(0.3):single(0.9); b=single(0):single(0.05):single(0.45);");
    assert_eq!(e.array("a").unwrap().cols(), 4);
    assert_eq!(e.array("b").unwrap().cols(), 10);
    for d in ["0.1", "0.3", "0.05", "0.7", "0.01"] {
        for k in 1..=60 {
            e.execute(&format!("c=single(0):single({d}):single({k}*single({d}));"))
                .unwrap();
            assert_eq!(e.array("c").unwrap().cols(), k + 1, "d={d} k={k}");
        }
    }
}

#[test]
fn w11_zeros_and_ones_keep_sparse_and_complex_prototypes() {
    // MathWorks zeros and ones document preservation of class, sparsity and complexity.
    let e = eval(
        "p=single(0); p(1)=1i; s=sparse(1); t=sparse(true); \
                  z=zeros([2 3],'like',s); o=ones([2 3],'like',s); \
                  l=ones(2,3,'like',t); c=zeros(2,3,'like',1i); d=ones([2 3],'like',p);",
    );
    for (name, nnz, logical) in [("z", 0, false), ("o", 6, false), ("l", 6, true)] {
        let Some(Array::Sparse(s)) = e.array(name) else {
            panic!("{name} lost sparse storage")
        };
        assert_eq!(
            (s.rows(), s.cols(), s.nnz(), s.is_logical()),
            (2, 3, nnz, logical)
        );
        assert!((0..nnz).all(|i| s.value(i) == 1.));
    }
    for (name, class, fill) in [("c", "double", 0.), ("d", "single", 1.)] {
        let c = &e.complex_variables[name];
        assert_eq!((c.rows(), c.cols()), (2, 3));
        assert_eq!(e.array(name).unwrap().class(), class);
        assert_eq!(c.data(), &[Complex64::new(fill, 0.); 6]);
    }
}

#[test]
fn w16_dfilt_rejects_nd_coefficients() {
    // isvector(reshape(1:4,1,2,2)) is false in GNU Octave 10.3.
    let mut e = eval("nd=reshape(1:4,1,2,2);");
    for (code, arg) in [
        ("dfilt.df1(nd,1)", "numerator"),
        ("dfilt.df1(1,nd)", "denominator"),
    ] {
        let err = e.execute(code).unwrap_err();
        assert!(err.contains(arg) && err.contains("vector"), "{err}");
    }
}

#[test]
fn w17_translated_tetrahedron_keeps_faces_and_volume() {
    // tests/fixtures/fn/sci2-expected.txt, cases 122/123 (unit tetrahedron).
    let e = eval("p=[0 0 0;eye(3)]; [k,v]=convhulln(p+1e13);");
    let k = &e.variables["k"];
    assert_eq!((k.rows(), k.cols()), (4, 3));
    let mut faces: Vec<_> = (0..4)
        .map(|r| {
            let mut face = [k.data()[r], k.data()[r + 4], k.data()[r + 8]];
            face.sort_by(f64::total_cmp);
            face
        })
        .collect();
    faces.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(
        faces,
        [[1., 2., 3.], [1., 2., 4.], [1., 3., 4.], [2., 3., 4.]]
    );
    assert!((e.variables["v"].data()[0] - 1. / 6.).abs() < 1e-15);
}

#[test]
fn r1_9_fractional_power_returns_principal_complex_single() {
    // mpmath.power(-4, 0.5) = 2i. Broadcast alongside a real result.
    let e = eval("z=power(single(-4),0.5); a=single([-4;9]).^[0.5 2];");
    for name in ["z", "a"] {
        assert_eq!(e.array(name).unwrap().class(), "single");
    }
    let i2 = Complex64::new(0., 2.);
    assert_eq!(e.complex_variables["z"].data(), &[i2]);
    assert_eq!(
        e.complex_variables["a"].data(),
        &[i2, 3.0.into(), 16.0.into(), 81.0.into()]
    );
}

#[test]
fn r1_10_mixed_plus_rounds_to_single_after_adding() {
    // NumPy float32(float64(1) + (-1.0 + 1e-8)), widened for exact comparison.
    let e = eval("a=plus(single(1),-1+1e-8); b=(-1+1e-8)+single(1);");
    for name in ["a", "b"] {
        let a = e.array(name).unwrap();
        assert_eq!(a.class(), "single");
        let Array::Typed(a) = a else {
            panic!("expected typed single")
        };
        assert_eq!(a.real(0), f64::from(1e-8_f32));
    }
}

#[test]
fn r1_11_complex_single_plus_double_keeps_single_precision() {
    // GNU Octave 10.3 preserves single under broadcast in either order.
    let e = eval("a=single([1;2]); a(1)=1i; x=a+[1 2]; y=[1 2]+a; z=a+(1+1e-8); r=a+[-1i;0];");
    for name in ["x", "y", "z", "r"] {
        assert_eq!(e.array(name).unwrap().class(), "single");
    }
    for name in ["x", "y"] {
        let a = &e.complex_variables[name];
        assert_eq!((a.rows(), a.cols()), (2, 2));
        assert_eq!(
            a.data(),
            &[
                Complex64::new(1., 1.),
                3.0.into(),
                Complex64::new(2., 1.),
                4.0.into()
            ]
        );
    }
    assert_eq!(e.complex_variables["z"].data()[0], Complex64::new(1., 1.));
}
