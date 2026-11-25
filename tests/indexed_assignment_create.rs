use matrust::{Array, Complex64, Engine, GraphicsArray, GraphicsKind, StructArray};

fn structure(engine: &Engine, name: &str, shape: (usize, usize), fields: &[&str]) -> StructArray {
    let Array::Struct(a) = engine.array(name).unwrap() else {
        panic!("expected struct {name}");
    };
    assert_eq!((a.rows(), a.cols()), shape);
    assert_eq!(a.field_names(), fields);
    a
}

fn handles(engine: &Engine, name: &str, shape: (usize, usize)) -> GraphicsArray {
    let Array::Graphics(a) = engine.array(name).unwrap() else {
        panic!("expected graphics array {name}");
    };
    assert_eq!((a.rows(), a.cols()), shape);
    a
}

fn assert_complex(
    engine: &Engine,
    name: &str,
    class: &str,
    shape: (usize, usize),
    values: &[Complex64],
) {
    let array = engine.array(name).unwrap();
    assert_eq!(array.class(), class);
    let Array::Complex(a) = array else {
        panic!("expected complex storage for {name}");
    };
    assert_eq!((a.rows(), a.cols()), shape);
    assert_eq!(a.data(), values);
}

#[test]
fn indexed_assignment_create_struct_keeps_field_names_and_fills_empty_fields() {
    let mut e = Engine::default();
    e.execute("clear s; s(3)=struct('a',1); t(2,3)=struct('b',2,'a',3);")
        .unwrap();
    let s = structure(&e, "s", (1, 3), &["a"]);
    for i in [0, 1] {
        let a = s.get(0, i).unwrap();
        assert_eq!((a.class(), a.rows(), a.cols()), ("double", 0, 0));
    }
    let Array::Numeric(field) = s.get(0, 2).unwrap().as_ref() else {
        panic!("expected double field");
    };
    assert_eq!(field.data(), &[1.]);
    let t = structure(&e, "t", (2, 3), &["b", "a"]);
    for index in 0..5 {
        for field in 0..2 {
            let value = t.get(field, index).unwrap();
            assert_eq!(
                (value.class(), value.rows(), value.cols()),
                ("double", 0, 0)
            );
        }
    }
    e.execute("b=t(2,3).b; a=t(2,3).a; alias=s; s(end+1)=struct('a',4); last=s(4).a;")
        .unwrap();
    assert_eq!(e.variables["b"].data(), &[2.]);
    assert_eq!(e.variables["a"].data(), &[3.]);
    assert_eq!(e.variables["last"].data(), &[4.]);
    let before = structure(&e, "s", (1, 4), &["a"]);
    assert!(s.same_storage(&structure(&e, "alias", (1, 3), &["a"])));
    let error = e.execute("s(5)=struct('b',2);").unwrap_err();
    assert!(
        error.contains("Subscripted assignment between dissimilar structures."),
        "{error}"
    );
    assert!(before.same_storage(&structure(&e, "s", (1, 4), &["a"])));
}

#[test]
fn indexed_assignment_create_graphics_preserves_handle_identity_and_fills_gaps() {
    let mut e = Engine::default();
    e.execute("clear s; s(1)=subplot(2,1,1); first=s(1); s(3)=subplot(2,1,2); s(2)=first; set(s(1),'XLim',[2 4]); limits=get(s(2),'XLim');")
        .unwrap();
    let s = handles(&e, "s", (1, 3));
    let first = handles(&e, "first", (1, 1));
    assert_eq!(s.get_linear(0).unwrap().kind(), GraphicsKind::Axes);
    assert!(
        s.get_linear(0)
            .unwrap()
            .same_object(first.scalar_handle().unwrap())
    );
    assert!(
        s.get_linear(1)
            .unwrap()
            .same_object(first.scalar_handle().unwrap())
    );
    assert!(
        !s.get_linear(2)
            .unwrap()
            .same_object(first.scalar_handle().unwrap())
    );
    assert_eq!(e.variables["limits"].data(), &[2., 4.]);
    e.execute("g(2,3)=s(3); live=isgraphics(g); g(1,2:3)=s(1:2);")
        .unwrap();
    assert_eq!(e.logical_variables["live"].data(), &[0, 0, 0, 0, 0, 1]);
    let g = handles(&e, "g", (2, 3));
    assert_eq!(g.get_linear(0).unwrap().kind(), GraphicsKind::Placeholder);
    for i in [2, 4] {
        assert!(
            g.get_linear(i)
                .unwrap()
                .same_object(first.scalar_handle().unwrap())
        );
    }
    assert!(
        g.get_linear(5)
            .unwrap()
            .same_object(s.get_linear(2).unwrap())
    );
}

#[test]
fn indexed_assignment_create_double_targets_store_the_explicit_numeric_handle_value() {
    let mut e = Engine::default();
    e.execute("h=zeros(1,2); h(1)=plot(1:3); ax=gca; expectedFirst=double(get(ax,'Children')); p=plot(2:4); expected=double(p); h(2)=p;")
        .unwrap();
    let h = &e.variables["h"];
    assert_eq!((h.rows(), h.cols()), (1, 2));
    assert_eq!(e.array("h").unwrap().class(), "double");
    assert!(h.data()[0].is_finite() && h.data()[0] != 0.);
    assert_eq!(h.data()[0], e.variables["expectedFirst"].data()[0]);
    assert_ne!(h.data()[0], h.data()[1]);
    assert_eq!(h.data()[1], e.variables["expected"].data()[0]);
    e.execute(
        "f=figure(2); hold on; both=[plot(1:3) plot(2:4)]; h(1:2)=both; expected=double(both);",
    )
    .unwrap();
    assert_eq!(e.variables["h"].data(), e.variables["expected"].data());
    assert!(
        e.execute("get(h(1),'Type');")
            .unwrap_err()
            .contains("typed graphics handle")
    );
    assert!(
        e.execute("set(h(1),'Visible','off');")
            .unwrap_err()
            .contains("typed graphics handle")
    );
}

#[test]
fn indexed_assignment_create_undefined_target_end_errors_as_matlab() {
    // Recorded MATLAB fixture (indexed-assignment-oracle.mat): end on an
    // undefined assignment target is an error and creates nothing.
    let mut e = Engine::default();
    let error = e.execute("x=R(end);").unwrap_err();
    assert!(
        error.contains("Undefined function or variable 'R'"),
        "{error}"
    );
    for code in [
        "clear R; R(end+1)=5;",
        "clear T; T(1+end,:)=[1 2];",
        "clear U; U(end+1)=uint16(7);",
    ] {
        assert!(e.execute(code).is_err(), "{code}");
    }
    for name in ["R", "T", "U"] {
        assert!(e.array(name).is_none(), "{name}");
    }
}

#[test]
fn indexed_assignment_create_promotes_float_logical_and_character_targets() {
    let mut e = Engine::default();
    e.execute("x=[1 2]; alias=x; x(2)=1i; q=single([3 4]); q(2)=complex(1/3,1/7); l=[true false]; l(2)=1i; c='ab'; c(2)=1i;")
        .unwrap();
    let imaginary = Complex64::new(0., 1.);
    assert_complex(
        &e,
        "x",
        "double",
        (1, 2),
        &[Complex64::new(1., 0.), imaginary],
    );
    assert_complex(
        &e,
        "l",
        "double",
        (1, 2),
        &[Complex64::new(1., 0.), imaginary],
    );
    assert_complex(
        &e,
        "c",
        "double",
        (1, 2),
        &[Complex64::new(97., 0.), imaginary],
    );
    let expected = Complex64::new(f64::from(1_f32 / 3.), f64::from(1_f32 / 7.));
    assert_complex(
        &e,
        "q",
        "single",
        (1, 2),
        &[Complex64::new(3., 0.), expected],
    );
    assert_eq!(e.variables["alias"].data(), &[1., 2.]);
    e.execute("q(3)=single(5); z=q(2); kind=class(q); d=double(q); q(1)=[];")
        .unwrap();
    let extended = [Complex64::new(3., 0.), expected, Complex64::new(5., 0.)];
    assert_complex(&e, "z", "single", (1, 1), &[expected]);
    let Array::Character(kind) = e.array("kind").unwrap() else {
        panic!("expected class name");
    };
    assert_eq!(kind.units(), &[115, 105, 110, 103, 108, 101]);
    assert_complex(&e, "d", "double", (1, 3), &extended);
    assert_complex(&e, "q", "single", (1, 2), &extended[1..]);
    e.execute("m=[1 2;3 4]; m(3,2)=1i; blocked=single([5 6]);")
        .unwrap();
    assert_complex(
        &e,
        "m",
        "double",
        (3, 2),
        &[
            Complex64::new(1., 0.),
            Complex64::new(3., 0.),
            Complex64::new(0., 0.),
            Complex64::new(2., 0.),
            Complex64::new(4., 0.),
            imaginary,
        ],
    );
    let before = e.array("blocked").unwrap();
    assert!(e.execute("blocked(1:2)=complex([1 2 3],1);").is_err());
    assert!(e.array("blocked").unwrap().same_snapshot(&before));
}

#[test]
fn indexed_assignment_create_char_uses_nul_padding_and_double_keeps_its_class() {
    let mut e = Engine::default();
    e.execute("clear c; c(3)='a'; codes=double(c); grid(2,3)='b'; d=zeros(1,2); d(2)='a';")
        .unwrap();
    for (name, shape, expected) in [
        ("c", (1, 3), &[0, 0, 97][..]),
        ("grid", (2, 3), &[0, 0, 0, 0, 0, 98][..]),
    ] {
        let Array::Character(a) = e.array(name).unwrap() else {
            panic!("expected char {name}");
        };
        assert_eq!((a.rows(), a.cols()), shape);
        assert_eq!(a.units(), expected);
    }
    assert_eq!(e.variables["codes"].data(), &[0., 0., 97.]);
    assert_eq!(e.array("d").unwrap().class(), "double");
    assert_eq!(e.variables["d"].data(), &[0., 97.]);
}
