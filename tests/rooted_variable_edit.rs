use matrust::variable_edit::{self, Rect, Target};
use matrust::{
    Array, CellArray, CharacterRow, Complex64, ComplexMatrix, Engine, LogicalMatrix, Matrix, Scalar,
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

fn target(engine: &Engine, name: &str, path: Vec<usize>) -> Target {
    Target::new(name.into(), engine.array(name).unwrap(), path).unwrap()
}
fn unchanged(engine: &Engine, before: &BTreeMap<String, Array>) {
    assert_eq!(engine.workspace_count(), before.len());
    for (name, value) in before {
        assert!(
            value.same_snapshot(&engine.array(name).unwrap()),
            "Changed {name}"
        );
    }
}
fn numeric(kind: usize, rows: usize, cols: usize) -> Array {
    let stop = AtomicBool::new(false);
    match kind {
        0 => Matrix::from_column_vec(rows, cols, (0..rows * cols).map(|i| i as f64).collect())
            .unwrap()
            .into(),
        1 => LogicalMatrix::from_column_vec(
            rows,
            cols,
            (0..rows * cols)
                .map(|i| u8::from(!i.is_multiple_of(2)))
                .collect(),
            &stop,
        )
        .unwrap()
        .into(),
        _ => ComplexMatrix::from_column_vec(
            rows,
            cols,
            (0..rows * cols)
                .map(|i| Complex64::new(i as f64, -(i as f64)))
                .collect(),
            &stop,
        )
        .unwrap()
        .into(),
    }
}
fn nested(leaf: &Array) -> Array {
    let stop = AtomicBool::new(false);
    let shared = Arc::new(leaf.clone());
    let inner: Array = CellArray::from_shared(1, 2, vec![shared.clone(), shared.clone()], &stop)
        .unwrap()
        .into();
    CellArray::from_shared(1, 2, vec![Arc::new(inner), shared], &stop)
        .unwrap()
        .into()
}
fn child(value: &Array, index: usize) -> &Array {
    let Array::Cell(c) = value else {
        panic!("Expected cell")
    };
    c.get_linear(index).unwrap()
}

#[test]
fn root_and_nested_rectangles_commit_once_preserving_other_bindings_and_siblings() {
    for kind in 0..3 {
        for path in [vec![], vec![0, 0]] {
            let mut e = Engine::default();
            let leaf = numeric(kind, 4, 3);
            let root = if path.is_empty() {
                leaf.clone()
            } else {
                nested(&leaf)
            };
            e.merge_workspace(BTreeMap::from([
                ("A".into(), root.clone()),
                ("Alias".into(), root.clone()),
                ("LeafAlias".into(), leaf.clone()),
            ]))
            .unwrap();
            let expected = target(&e, "A", path.clone());
            let bytes = e.workspace_bytes();
            let text = match kind {
                0 => "11\t12\r\n21\t22\r\n",
                1 => "true\t0\n-2\tfalse",
                _ => "11-1i\t12+2i\n21-3i\t22+4i",
            };
            let committed =
                variable_edit::paste_tsv(&mut e, &expected, Rect::new(1, 1, 1, 1).unwrap(), text)
                    .unwrap();
            assert_eq!(committed.rectangle, Rect::new(1, 1, 2, 2).unwrap());
            assert!(committed.root.same_snapshot(&e.array("A").unwrap()));
            assert!(!committed.root.same_snapshot(&root));
            assert_eq!(e.workspace_bytes(), bytes);
            assert!(e.array("Alias").unwrap().same_snapshot(&root));
            assert!(e.array("LeafAlias").unwrap().same_snapshot(&leaf));
            let result = if path.is_empty() {
                &committed.root
            } else {
                child(child(&committed.root, 0), 0)
            };
            if !path.is_empty() {
                assert!(child(&committed.root, 1).same_snapshot(child(&root, 1)));
                assert!(child(child(&committed.root, 0), 1).same_snapshot(&leaf));
            }
            for col in 0..3 {
                for row in 0..4 {
                    let actual = result.scalar(row, col).unwrap();
                    if (1..3).contains(&row) && (1..3).contains(&col) {
                        let fields = match kind {
                            0 => ["11", "12", "21", "22"],
                            1 => ["1", "0", "1", "0"],
                            _ => ["11-1i", "12+2i", "21-3i", "22+4i"],
                        };
                        let expected = Scalar::parse_cell(
                            fields[(row - 1) * 2 + col - 1],
                            kind == 2,
                            kind == 1,
                        )
                        .unwrap();
                        assert_eq!(actual.into_complex(), expected.into_complex());
                    } else {
                        assert_eq!(actual, leaf.scalar(row, col).unwrap());
                    }
                }
            }
        }
    }
}

#[test]
fn bounded_scientific_copy_roundtrips_extreme_components_and_signed_zero() {
    let values = [
        f64::MAX,
        -f64::MAX,
        f64::MIN_POSITIVE,
        f64::from_bits(1),
        -f64::from_bits(1),
        0.,
        -0.,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::from_bits(0x7ff8_0000_0000_1234),
    ];
    let same = |a: f64, b: f64| {
        if a.is_nan() {
            assert!(b.is_nan());
        } else {
            assert_eq!(a.to_bits(), b.to_bits());
        }
    };
    for &re in &values {
        let literal = variable_edit::scalar_literal(Scalar::Real(re));
        assert!(literal.len() <= 64);
        let Scalar::Real(result) = Scalar::parse_cell(&literal, false, false).unwrap() else {
            panic!()
        };
        same(re, result);
        for &im in &values {
            let literal = variable_edit::scalar_literal(Scalar::Complex(Complex64::new(re, im)));
            assert!(literal.len() <= 64, "{literal}");
            let Scalar::Complex(result) = Scalar::parse_cell(&literal, true, false).unwrap() else {
                panic!()
            };
            same(re, result.re);
            same(im, result.im);
        }
    }
    let mut e = Engine::default();
    let stop = AtomicBool::new(false);
    let data = vec![
        Complex64::new(f64::MAX, -f64::from_bits(1)),
        Complex64::new(-0., -0.),
        Complex64::new(f64::from_bits(1), f64::MAX),
        Complex64::new(3., -4.),
    ];
    e.merge_workspace(BTreeMap::from([
        (
            "Z".into(),
            ComplexMatrix::from_column_vec(2, 2, data.clone(), &stop)
                .unwrap()
                .into(),
        ),
        ("Out".into(), numeric(2, 2, 2)),
    ]))
    .unwrap();
    let before = e.workspace();
    let text =
        variable_edit::copy_tsv(&e, &target(&e, "Z", vec![]), Rect::new(0, 0, 2, 2).unwrap())
            .unwrap();
    unchanged(&e, &before);
    assert_eq!(text.lines().count(), 2);
    assert!(text.lines().all(|l| l.split('\t').count() == 2));
    let out = target(&e, "Out", vec![]);
    variable_edit::paste_tsv(&mut e, &out, Rect::new(0, 0, 2, 2).unwrap(), &text).unwrap();
    let Array::Complex(result) = e.array("Out").unwrap() else {
        panic!()
    };
    for (a, b) in data.iter().zip(result.data()) {
        same(a.re, b.re);
        same(a.im, b.im);
    }
}

#[test]
fn malformed_paste_type_and_shape_failures_are_atomic_and_never_evaluate_scripts() {
    for kind in 0..3 {
        let mut e = Engine::default();
        let root = nested(&numeric(kind, 4, 4));
        e.merge_workspace(BTreeMap::from([
            ("A".into(), root.clone()),
            ("Alias".into(), root),
        ]))
        .unwrap();
        let request = target(&e, "A", vec![0, 0]);
        let before = e.workspace();
        for text in [
            "",
            "1\t",
            "\t1",
            "1\n\n",
            "1\t2\n3",
            "1\r2",
            "1\t2\n3\tclear",
            "sin(1)",
            "[1 2]",
            "x=42",
            "1,2",
        ] {
            assert!(
                variable_edit::paste_tsv(&mut e, &request, Rect::new(0, 0, 1, 1).unwrap(), text)
                    .is_err(),
                "{kind} {text:?}"
            );
            unchanged(&e, &before);
        }
        assert!(
            variable_edit::paste_tsv(&mut e, &request, Rect::new(0, 0, 2, 2).unwrap(), "1")
                .is_err()
        );
        assert!(
            variable_edit::paste_tsv(&mut e, &request, Rect::new(3, 3, 1, 1).unwrap(), "1\t2")
                .is_err()
        );
        assert!(variable_edit::apply_literal(&mut e, &request, 0, 0, &"1".repeat(257)).is_err());
        assert!(
            variable_edit::paste_tsv(
                &mut e,
                &request,
                Rect::new(0, 0, 1, 1).unwrap(),
                &"1".repeat(variable_edit::MAX_TEXT_BYTES + 1)
            )
            .is_err()
        );
        if kind < 2 {
            assert!(
                variable_edit::apply_values(
                    &mut e,
                    &request,
                    Rect::new(0, 0, 1, 1).unwrap(),
                    &[Scalar::Complex(Complex64::new(1., 0.))]
                )
                .is_err()
            );
        }
        if kind == 1 {
            assert!(variable_edit::apply_literal(&mut e, &request, 0, 0, "NaN").is_err());
        }
        unchanged(&e, &before);
    }
}

#[test]
fn stale_whole_root_shape_type_and_removed_name_reject_before_edit_or_copy() {
    let mut e = Engine::default();
    let original = numeric(0, 2, 3);
    e.merge_workspace(BTreeMap::from([("A".into(), original.clone())]))
        .unwrap();
    let request = target(&e, "A", vec![]);
    let Array::Numeric(a) = &original else {
        panic!()
    };
    for next in [
        Array::from(a.reshape(3, 2).unwrap()),
        numeric(1, 2, 3),
        numeric(0, 2, 3),
    ] {
        e.merge_workspace(BTreeMap::from([("A".into(), next)]))
            .unwrap();
        let before = e.workspace();
        assert!(
            variable_edit::paste_tsv(
                &mut e,
                &request,
                Rect::new(0, 0, 1, 1).unwrap(),
                "not valid"
            )
            .unwrap_err()
            .contains("changed")
        );
        assert!(variable_edit::copy_tsv(&e, &request, Rect::new(0, 0, 1, 1).unwrap()).is_err());
        unchanged(&e, &before);
    }
    e.remove_array("A");
    assert!(variable_edit::apply_literal(&mut e, &request, 0, 0, "1").is_err());
    assert_eq!(e.workspace_count(), 0);
    let original = nested(&numeric(0, 2, 3));
    e.merge_workspace(BTreeMap::from([("A".into(), original)]))
        .unwrap();
    let nested_request = target(&e, "A", vec![0, 0]);
    // Even an unrelated sibling replacement invalidates the whole-root expectation.
    let root = e.array("A").unwrap();
    let Array::Cell(c) = root else { panic!() };
    e.merge_workspace(BTreeMap::from([(
        "A".into(),
        c.replaced(1, numeric(0, 1, 1), &AtomicBool::new(false))
            .unwrap()
            .into(),
    )]))
    .unwrap();
    let before = e.workspace();
    assert!(variable_edit::apply_literal(&mut e, &nested_request, 0, 0, "99").is_err());
    unchanged(&e, &before);
}

#[test]
fn path_rectangle_readonly_and_pre_cancel_boundaries_are_explicit() {
    let stop = AtomicBool::new(false);
    let leaf = numeric(0, 1, 1);
    assert!(Target::new("A".into(), leaf.clone(), vec![0]).is_err());
    assert!(Target::new("A".into(), nested(&leaf), vec![2]).is_err());
    assert!(Target::new("A".into(), nested(&leaf), vec![0; 17]).is_err());
    assert!(
        Target::new(
            "A".into(),
            CharacterRow::from_units(vec![0, 0xd800], &stop)
                .unwrap()
                .into(),
            vec![]
        )
        .is_ok()
    );
    for args in [
        (0, 0, 0, 1),
        (usize::MAX, 0, 1, 1),
        (0, 0, 1, 65537),
        (0, 0, usize::MAX, 2),
    ] {
        assert!(Rect::new(args.0, args.1, args.2, args.3).is_err());
    }
    let mut e = Engine::default();
    e.merge_workspace(BTreeMap::from([("A".into(), leaf)]))
        .unwrap();
    let request = target(&e, "A", vec![]);
    let before = e.workspace();
    e.cancellation().store(true, Ordering::Relaxed);
    assert!(variable_edit::apply_literal(&mut e, &request, 0, 0, "9").is_err());
    assert!(variable_edit::copy_tsv(&e, &request, Rect::new(0, 0, 1, 1).unwrap()).is_err());
    unchanged(&e, &before);
    e.cancellation().store(false, Ordering::Relaxed);
    variable_edit::apply_literal(&mut e, &request, 0, 0, "9").unwrap();
    assert_eq!(
        e.array("A").unwrap().scalar(0, 0).unwrap(),
        Scalar::Real(9.)
    );
}

#[test]
fn near_budget_shared_workspace_keeps_logical_accounting_after_rooted_edit() {
    let stop = AtomicBool::new(false);
    let root: Array = CellArray::empty(1, 65536, &stop).unwrap().into();
    let Array::Cell(cell) = root else { panic!() };
    let root: Array = cell.replaced(0, numeric(0, 1, 1), &stop).unwrap().into();
    let count = (256 * 1024 * 1024 / root.bytes()).min(9999);
    let mut values = BTreeMap::new();
    for i in 0..count {
        values.insert(format!("A{i:04}"), root.clone());
    }
    let mut e = Engine::default();
    e.merge_workspace(values).unwrap();
    let before = e.workspace();
    let bytes = e.workspace_bytes();
    let request = target(&e, "A0000", vec![0]);
    variable_edit::apply_literal(&mut e, &request, 0, 0, "7").unwrap();
    assert_eq!(e.workspace_bytes(), bytes);
    assert_eq!(e.workspace_count(), count);
    for (name, value) in &before {
        if name != "A0000" {
            assert!(value.same_snapshot(&e.array(name).unwrap()));
        }
    }
    let changed = e.array("A0000").unwrap();
    assert_eq!(child(&changed, 0).scalar(0, 0).unwrap(), Scalar::Real(7.));
    assert!(child(&changed, 65535).same_snapshot(child(&root, 65535)));
}

#[test]
fn maximum_clipboard_cell_count_is_exact_and_one_extra_field_is_atomic() {
    let mut e = Engine::default();
    e.merge_workspace(BTreeMap::from([("A".into(), numeric(1, 1, 65536))]))
        .unwrap();
    let request = target(&e, "A", vec![]);
    let before = e.workspace();
    let text = (0..65536).map(|_| "1").collect::<Vec<_>>().join("\t");
    assert!(
        variable_edit::paste_tsv(
            &mut e,
            &request,
            Rect::new(0, 0, 1, 1).unwrap(),
            &format!("{text}\t1")
        )
        .is_err()
    );
    unchanged(&e, &before);
    variable_edit::paste_tsv(&mut e, &request, Rect::new(0, 0, 1, 1).unwrap(), &text).unwrap();
    let result = target(&e, "A", vec![]);
    assert_eq!(
        variable_edit::copy_tsv(&e, &result, Rect::new(0, 0, 1, 65536).unwrap()).unwrap(),
        text
    );
}

#[test]
fn editing_preserves_untouched_nan_payload_and_signed_zero_bits() {
    let values = vec![
        f64::from_bits(0x7ff8_0000_0000_1234),
        -0.,
        f64::from_bits(1),
        9.,
    ];
    let mut e = Engine::default();
    let a: Array = Matrix::from_column_vec(2, 2, values.clone())
        .unwrap()
        .into();
    e.merge_workspace(BTreeMap::from([
        ("A".into(), a.clone()),
        ("Alias".into(), a.clone()),
    ]))
    .unwrap();
    let request = target(&e, "A", vec![]);
    variable_edit::apply_literal(&mut e, &request, 1, 1, "7").unwrap();
    let Array::Numeric(result) = e.array("A").unwrap() else {
        panic!()
    };
    for (actual, expected) in result.data().iter().zip(&values).take(3) {
        assert_eq!(actual.to_bits(), expected.to_bits());
    }
    assert_eq!(result.data()[3], 7.);
    assert!(e.array("Alias").unwrap().same_snapshot(&a));
}

#[test]
fn maximum_cell_depth_is_editable_but_empty_numeric_and_graphics_have_no_editable_cells() {
    let stop = AtomicBool::new(false);
    let mut root = numeric(0, 1, 1);
    for _ in 0..16 {
        root = CellArray::from_shared(1, 1, vec![Arc::new(root)], &stop)
            .unwrap()
            .into();
    }
    let mut e = Engine::default();
    e.merge_workspace(BTreeMap::from([
        ("A".into(), root.clone()),
        ("Alias".into(), root.clone()),
    ]))
    .unwrap();
    let request = target(&e, "A", vec![0; 16]);
    let committed = variable_edit::apply_literal(&mut e, &request, 0, 0, "-0").unwrap();
    let mut leaf = &committed.root;
    for _ in 0..16 {
        leaf = child(leaf, 0);
    }
    assert_eq!(
        leaf.scalar(0, 0).unwrap().into_real().unwrap().to_bits(),
        (-0.0f64).to_bits()
    );
    assert!(e.array("Alias").unwrap().same_snapshot(&root));
    let excessive_capacity = Vec::with_capacity(17);
    assert!(Target::new("A".into(), numeric(0, 1, 1), excessive_capacity).is_err());
    let graphics = matrust::GraphicsArray::from_shared(0, 0, vec![], &stop).unwrap();
    assert!(Target::new("G".into(), graphics.into(), vec![]).is_err());
    for kind in 0..3 {
        e.merge_workspace(BTreeMap::from([("Empty".into(), numeric(kind, 0, 3))]))
            .unwrap();
        let request = target(&e, "Empty", vec![]);
        let before = e.workspace();
        assert!(
            variable_edit::paste_tsv(&mut e, &request, Rect::new(0, 0, 1, 1).unwrap(), "7")
                .is_err()
        );
        assert!(variable_edit::copy_tsv(&e, &request, Rect::new(0, 0, 1, 1).unwrap()).is_err());
        unchanged(&e, &before);
    }
}
