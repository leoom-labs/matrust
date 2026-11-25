use super::*;
use std::sync::{Arc, atomic::Ordering};

fn fixture() -> (Engine, Target) {
    let mut e = Engine::default();
    let stop = AtomicBool::new(false);
    let leaf: Array = Matrix::from_column_vec(128, 128, (0..16384).map(|i| i as f64).collect())
        .unwrap()
        .into();
    let shared = Arc::new(leaf.clone());
    let inner: Array = CellArray::from_shared(1, 2, vec![shared.clone(), shared.clone()], &stop)
        .unwrap()
        .into();
    let root: Array = CellArray::from_shared(1, 2, vec![Arc::new(inner), shared], &stop)
        .unwrap()
        .into();
    e.merge_workspace(BTreeMap::from([
        ("A".into(), root.clone()),
        ("Alias".into(), root),
        ("Leaf".into(), leaf),
    ]))
    .unwrap();
    let target = Target::new("A".into(), e.array("A").unwrap(), vec![0, 0]).unwrap();
    (e, target)
}
fn unchanged(e: &Engine, before: &BTreeMap<String, Array>) {
    assert_eq!(e.workspace_count(), before.len());
    for (name, value) in before {
        assert!(value.same_snapshot(&e.array(name).unwrap()));
    }
}
#[test]
fn stop_after_actual_parse_copy_ancestor_and_precommit_work_is_atomic_then_retries() {
    let text = (0..64)
        .map(|_| (0..64).map(|_| "7").collect::<Vec<_>>().join("\t"))
        .collect::<Vec<_>>()
        .join("\r\n");
    for (stage, occurrence) in [
        (Stage::ParseProgress, 2),
        (Stage::Parsed, 1),
        (Stage::CopyProgress, 1),
        (Stage::CopyProgress, 3),
        (Stage::PatchProgress, 1),
        (Stage::Ancestor, 1),
        (Stage::Ancestor, 2),
        (Stage::BeforeCommit, 1),
    ] {
        let (mut e, target) = fixture();
        let before = e.workspace();
        let cancel = e.cancellation();
        let mut seen = 0;
        let result = apply(
            &mut e,
            &target,
            Rect::new(0, 0, 1, 1).unwrap(),
            Input::Tsv(&text),
            &mut |at| {
                if at == stage {
                    seen += 1;
                    if seen == occurrence {
                        cancel.store(true, Ordering::Relaxed);
                    }
                }
                Ok(())
            },
        );
        assert!(
            result.unwrap_err().contains("cancelled"),
            "{stage:?}/{occurrence}"
        );
        assert_eq!(seen, occurrence);
        unchanged(&e, &before);
        cancel.store(false, Ordering::Relaxed);
        let result = paste_tsv(&mut e, &target, Rect::new(0, 0, 1, 1).unwrap(), &text).unwrap();
        let leaf = descend(&result.root, &[0, 0]).unwrap();
        assert_eq!(leaf.scalar(63, 63).unwrap(), Scalar::Real(7.));
        assert_eq!(leaf.scalar(64, 64).unwrap(), Scalar::Real(8256.));
        assert!(before["Alias"].same_snapshot(&e.array("Alias").unwrap()));
        assert!(before["Leaf"].same_snapshot(&e.array("Leaf").unwrap()));
    }
}
#[test]
fn invalid_final_literal_and_stale_target_never_enter_payload_copy() {
    let (mut e, target) = fixture();
    let before = e.workspace();
    let mut copied = false;
    assert!(
        apply(
            &mut e,
            &target,
            Rect::new(0, 0, 1, 1).unwrap(),
            Input::Tsv("1\t2\n3\tx=9"),
            &mut |at| {
                copied |= at == Stage::CopyProgress;
                Ok(())
            }
        )
        .is_err()
    );
    assert!(!copied);
    unchanged(&e, &before);
    e.merge_workspace(BTreeMap::from([("A".into(), Matrix::scalar(1.).into())]))
        .unwrap();
    let mut entered = false;
    assert!(
        apply(
            &mut e,
            &target,
            Rect::new(0, 0, 1, 1).unwrap(),
            Input::Tsv("garbage"),
            &mut |_| {
                entered = true;
                Ok(())
            }
        )
        .unwrap_err()
        .contains("changed")
    );
    assert!(!entered);
}
#[test]
fn copy_cancellation_returns_no_partial_text_and_changes_no_bindings() {
    let (e, target) = fixture();
    let before = e.workspace();
    let cancel = e.cancellation();
    let mut visits = 0;
    let result = copy_checked(
        &e,
        &target,
        Rect::new(0, 0, 64, 64).unwrap(),
        &mut |stage| {
            if stage == Stage::FormatProgress {
                visits += 1;
                if visits == 3 {
                    cancel.store(true, Ordering::Relaxed);
                }
            }
            Ok(())
        },
    );
    assert!(result.unwrap_err().contains("cancelled"));
    assert_eq!(visits, 3);
    unchanged(&e, &before);
    cancel.store(false, Ordering::Relaxed);
    let text = copy_tsv(&e, &target, Rect::new(0, 0, 64, 64).unwrap()).unwrap();
    assert_eq!(text.lines().count(), 64);
}

#[test]
fn maximum_rectangular_complex_clipboard_keeps_row_major_order_and_rejects_last_field() {
    let cancel = AtomicBool::new(false);
    let row = (0..256)
        .map(|col| format!("{col}e0-0e0i"))
        .collect::<Vec<_>>()
        .join("\t");
    let text = (0..256)
        .map(|_| row.as_str())
        .collect::<Vec<_>>()
        .join("\r\n");
    let parsed = parse_tsv(&text, Kind::Complex, &cancel, &mut |_| Ok(())).unwrap();
    assert_eq!(
        (parsed.rows, parsed.cols, parsed.values.len()),
        (256, 256, MAX_CELLS)
    );
    for (index, value) in parsed.values.iter().enumerate() {
        let Scalar::Complex(value) = value else {
            panic!("Expected complex storage")
        };
        assert_eq!(value.re, (index % 256) as f64);
        assert_eq!(value.im.to_bits(), (-0.0f64).to_bits());
    }
    let last = text.rfind('\t').unwrap();
    let malformed = format!("{}clear", &text[..last + 1]);
    assert!(parse_tsv(&malformed, Kind::Complex, &cancel, &mut |_| Ok(())).is_err());
}

#[test]
fn character_parse_copy_patch_ancestor_and_commit_stop_preserve_all_aliases() {
    let text = (0..64)
        .map(|_| (0..64).map(|_| "\\uD800").collect::<Vec<_>>().join("\t"))
        .collect::<Vec<_>>()
        .join("\n");
    for stage in [
        Stage::ParseProgress,
        Stage::Parsed,
        Stage::CopyProgress,
        Stage::PatchProgress,
        Stage::Ancestor,
        Stage::BeforeCommit,
    ] {
        let stop = AtomicBool::new(false);
        let leaf: Array = CharacterRow::from_column_major(128, 128, vec![0; 16384], &stop)
            .unwrap()
            .into();
        let shared = Arc::new(leaf.clone());
        let root: Array = CellArray::from_shared(1, 2, vec![shared.clone(), shared], &stop)
            .unwrap()
            .into();
        let mut e = Engine::default();
        e.merge_workspace(BTreeMap::from([
            ("C".into(), root.clone()),
            ("Alias".into(), root),
        ]))
        .unwrap();
        let target = Target::new("C".into(), e.array("C").unwrap(), vec![0]).unwrap();
        let before = e.workspace();
        let cancel = e.cancellation();
        let mut reached = false;
        let result = apply(
            &mut e,
            &target,
            Rect::new(0, 0, 1, 1).unwrap(),
            Input::Tsv(&text),
            &mut |at| {
                if at == stage {
                    reached = true;
                    cancel.store(true, Ordering::Relaxed);
                }
                Ok(())
            },
        );
        assert!(reached, "Did not reach {stage:?}");
        assert!(result.unwrap_err().contains("cancelled"));
        unchanged(&e, &before);
        cancel.store(false, Ordering::Relaxed);
        let out = paste_tsv(&mut e, &target, Rect::new(0, 0, 1, 1).unwrap(), &text).unwrap();
        assert_eq!(
            cell_literal(descend(&out.root, &[0]).unwrap(), 63, 63).unwrap(),
            "\\uD800"
        );
        assert_eq!(
            cell_literal(descend(&out.root, &[0]).unwrap(), 64, 64).unwrap(),
            "\\u0000"
        );
        assert!(descend(&out.root, &[1]).unwrap().same_snapshot(&leaf));
        assert!(e.array("Alias").unwrap().same_snapshot(&before["Alias"]));
        let current = Target::new("C".into(), out.root, vec![0]).unwrap();
        let mut formatted = false;
        let result = copy_checked(
            &e,
            &current,
            Rect::new(0, 0, 128, 128).unwrap(),
            &mut |at| {
                if at == Stage::FormatProgress {
                    formatted = true;
                    cancel.store(true, Ordering::Relaxed);
                }
                Ok(())
            },
        );
        assert!(formatted);
        assert!(result.unwrap_err().contains("cancelled"));
        cancel.store(false, Ordering::Relaxed);
        assert!(
            copy_tsv(&e, &current, Rect::new(0, 0, 128, 128).unwrap())
                .unwrap()
                .starts_with("\\uD800")
        );
    }
}

#[test]
fn character_copy_all_utf16_units_and_offset_rectangles_match_literal_reference() {
    let stop = AtomicBool::new(false);
    for (rows, cols) in [(256, 256), (1, MAX_CELLS), (MAX_CELLS, 1)] {
        let units: Vec<u16> = (u16::MIN..=u16::MAX).collect();
        let leaf: Array = CharacterRow::from_column_major(rows, cols, units, &stop)
            .unwrap()
            .into();
        let mut engine = Engine::default();
        engine
            .merge_workspace(BTreeMap::from([
                ("A".into(), leaf.clone()),
                ("Alias".into(), leaf),
            ]))
            .unwrap();
        let target = Target::new("A".into(), engine.array("A").unwrap(), vec![]).unwrap();
        let before = engine.workspace();
        let rectangle = Rect::new(0, 0, rows, cols).unwrap();
        let expected = (0..rows)
            .map(|row| {
                (0..cols)
                    .map(|col| format!("\\u{:04X}", col * rows + row))
                    .collect::<Vec<_>>()
                    .join("\t")
            })
            .collect::<Vec<_>>()
            .join("\n");
        let actual = copy_tsv(&engine, &target, rectangle).unwrap();
        assert_eq!(actual, expected);
        assert_eq!(actual.len(), 458_751);
        assert!(actual.len() < MAX_TEXT_BYTES);
        assert!(actual.contains("\\u0000"));
        assert!(actual.contains("\\uD800"));
        assert!(actual.contains("\\uFFFF"));
        unchanged(&engine, &before);
        if rows == 256 {
            let expected = (3..20)
                .map(|row| {
                    (5..24)
                        .map(|col| format!("\\u{:04X}", col * rows + row))
                        .collect::<Vec<_>>()
                        .join("\t")
                })
                .collect::<Vec<_>>()
                .join("\n");
            assert_eq!(
                copy_tsv(&engine, &target, Rect::new(3, 5, 17, 19).unwrap()).unwrap(),
                expected
            );
            assert!(copy_tsv(&engine, &target, Rect::new(255, 0, 2, 1).unwrap()).is_err());
            assert!(copy_tsv(&engine, &target, Rect::new(0, 255, 1, 2).unwrap()).is_err());
        }
    }
    assert!(Rect::new(0, 0, 1, MAX_CELLS + 1).is_err());
}

#[test]
fn character_copy_active_cancellation_and_stale_root_preserve_nested_aliases() {
    let stop = AtomicBool::new(false);
    let leaf: Array =
        CharacterRow::from_column_major(256, 256, (u16::MIN..=u16::MAX).collect(), &stop)
            .unwrap()
            .into();
    let shared = Arc::new(leaf.clone());
    let root: Array = CellArray::from_shared(1, 2, vec![shared.clone(), shared], &stop)
        .unwrap()
        .into();
    let mut engine = Engine::default();
    engine
        .merge_workspace(BTreeMap::from([
            ("C".into(), root.clone()),
            ("Alias".into(), root),
            ("Leaf".into(), leaf),
        ]))
        .unwrap();
    let target = Target::new("C".into(), engine.array("C").unwrap(), vec![1]).unwrap();
    let rectangle = Rect::new(0, 0, 256, 256).unwrap();
    let before = engine.workspace();
    let expected = copy_tsv(&engine, &target, rectangle).unwrap();
    let cancel = engine.cancellation();
    // Stop after 512 units and after 65,280 units have actually been emitted.
    for occurrence in [3, 256] {
        let mut seen = 0;
        let result = copy_checked(&engine, &target, rectangle, &mut |stage| {
            if stage == Stage::FormatProgress {
                seen += 1;
                if seen == occurrence {
                    cancel.store(true, Ordering::Relaxed);
                }
            }
            Ok(())
        });
        assert_eq!(seen, occurrence);
        assert!(result.unwrap_err().contains("cancelled"));
        unchanged(&engine, &before);
        cancel.store(false, Ordering::Relaxed);
        assert_eq!(copy_tsv(&engine, &target, rectangle).unwrap(), expected);
    }
    let replacement: Array = CharacterRow::from_column_major(1, 1, vec![7], &stop)
        .unwrap()
        .into();
    engine
        .merge_workspace(BTreeMap::from([("C".into(), replacement)]))
        .unwrap();
    let changed = engine.workspace();
    let mut formatted = false;
    assert!(
        copy_checked(&engine, &target, rectangle, &mut |stage| {
            formatted |= stage == Stage::FormatProgress;
            Ok(())
        })
        .is_err()
    );
    assert!(!formatted);
    unchanged(&engine, &changed);
    assert!(
        engine
            .array("Alias")
            .unwrap()
            .same_snapshot(&before["Alias"])
    );
    assert!(engine.array("Leaf").unwrap().same_snapshot(&before["Leaf"]));
}

fn put(e: &mut Engine, array: Array) -> Target {
    e.merge_workspace(BTreeMap::from([("X".into(), array)]))
        .unwrap();
    Target::new("X".into(), e.array("X").unwrap(), vec![]).unwrap()
}
fn real(rows: usize, cols: usize, row_major: &[f64]) -> Array {
    let mut column = Vec::new();
    for c in 0..cols {
        for r in 0..rows {
            column.push(row_major[r * cols + c]);
        }
    }
    Matrix::from_column_vec(rows, cols, column).unwrap().into()
}
/// Row-major reals of any grid kind.
fn grid(a: &Array) -> (usize, usize, Vec<f64>) {
    let mut v = Vec::new();
    for r in 0..a.rows() {
        for c in 0..a.cols() {
            v.push(match a.scalar(r, c).unwrap() {
                Scalar::Real(x) => x,
                Scalar::Integer(x) => x as f64,
                Scalar::Logical(x) => f64::from(u8::from(x)),
                other => panic!("{other:?}"),
            });
        }
    }
    (a.rows(), a.cols(), v)
}
fn block(rows: usize, cols: usize) -> (String, Vec<f64>) {
    let v: Vec<f64> = (0..rows * cols).map(|i| (i + 1) as f64 * 10.).collect();
    let text = v
        .chunks(cols)
        .map(|r| {
            r.iter()
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
                .join("\t")
        })
        .collect::<Vec<_>>()
        .join("\n");
    (text, v)
}

#[test]
fn paste_grows_like_matlab_indexed_assignment() {
    let mut e = Engine::default();
    let target = put(&mut e, real(2, 2, &[1., 2., 3., 4.]));
    let (text, v) = block(3, 4);
    let done = paste_tsv(&mut e, &target, Rect::new(0, 0, 1, 1).unwrap(), &text).unwrap();
    assert_eq!(grid(&done.root), (3, 4, v));
    assert_eq!(grid(&e.array("X").unwrap()).0, 3);
    // x(2:4,2:5) = block on a 2x2: 4x5, untouched cells are zero.
    let target = put(&mut e, real(2, 2, &[1., 2., 3., 4.]));
    let done = paste_tsv(&mut e, &target, Rect::new(1, 1, 1, 1).unwrap(), &text).unwrap();
    let mut want = vec![0.; 20];
    want[0] = 1.;
    want[1] = 2.;
    want[5] = 3.;
    for r in 0..3 {
        for c in 0..4 {
            want[(r + 1) * 5 + c + 1] = (r * 4 + c + 1) as f64 * 10.;
        }
    }
    want[5] = 3.;
    // The block overwrites x(2,2)=4; x(2,1)=3 stays.
    assert_eq!(grid(&done.root), (4, 5, want));
}
#[test]
fn multi_cell_selection_pastes_at_top_left_without_broadcast() {
    let mut e = Engine::default();
    let target = put(&mut e, real(3, 3, &[0.; 9]));
    let done = paste_tsv(&mut e, &target, Rect::new(1, 1, 2, 2).unwrap(), "7").unwrap();
    let (_, _, v) = grid(&done.root);
    assert_eq!(v.iter().filter(|x| **x == 7.).count(), 1);
    assert_eq!(v[4], 7.);
}
#[test]
fn logical_char_complex_and_int8_grow_with_class_zero() {
    let mut e = Engine::default();
    let stop = AtomicBool::new(false);
    let logical: Array = LogicalMatrix::from_canonical_vec(1, 1, vec![1])
        .unwrap()
        .into();
    let target = put(&mut e, logical);
    let done = paste_tsv(
        &mut e,
        &target,
        Rect::new(0, 0, 1, 1).unwrap(),
        "1\t1\n0\t1",
    )
    .unwrap();
    assert_eq!(grid(&done.root), (2, 2, vec![1., 1., 0., 1.]));
    let target = put(&mut e, done.root.clone());
    let done = paste_tsv(&mut e, &target, Rect::new(2, 2, 1, 1).unwrap(), "true").unwrap();
    assert_eq!(
        grid(&done.root),
        (3, 3, vec![1., 1., 0., 0., 1., 0., 0., 0., 1.])
    );
    assert!(done.root.is_logical());
    let text: Array = CharacterRow::from_column_major(1, 1, vec![65], &stop)
        .unwrap()
        .into();
    let target = put(&mut e, text);
    let done = paste_tsv(
        &mut e,
        &target,
        Rect::new(0, 0, 1, 1).unwrap(),
        "\\u0041\t\\u0042",
    )
    .unwrap();
    let Array::Character(c) = &done.root else {
        panic!()
    };
    assert_eq!(c.units(), [65, 66]);
    let done = paste_tsv(
        &mut e,
        &Target::new("X".into(), done.root.clone(), vec![]).unwrap(),
        Rect::new(1, 0, 1, 1).unwrap(),
        "\\u0043",
    )
    .unwrap();
    let Array::Character(c) = &done.root else {
        panic!()
    };
    assert_eq!(
        (c.rows(), c.cols(), c.units()),
        (2, 2, &[65, 67, 66, 0][..])
    );
    let cx: Array = ComplexMatrix::from_column_vec(1, 1, vec![Complex64 { re: 1., im: 2. }], &stop)
        .unwrap()
        .into();
    let target = put(&mut e, cx);
    let done = paste_tsv(
        &mut e,
        &target,
        Rect::new(0, 0, 1, 1).unwrap(),
        "1+2i\t3-4i",
    )
    .unwrap();
    assert_eq!(
        done.root.scalar(0, 1).unwrap(),
        Scalar::Complex(Complex64 { re: 3., im: -4. })
    );
    let i8: Array =
        crate::TypedArray::from_ints(crate::NumericClass::Int8, 1, 1, std::iter::once(5))
            .unwrap()
            .into();
    let target = put(&mut e, i8);
    let done = paste_tsv(&mut e, &target, Rect::new(1, 1, 1, 1).unwrap(), "300\t-300").unwrap();
    assert_eq!(grid(&done.root), (2, 3, vec![5., 0., 0., 0., 127., -128.]));
    assert!(matches!(done.root, Array::Typed(_)));
}
#[test]
fn excel_text_with_crlf_and_trailing_newline_pastes() {
    let mut e = Engine::default();
    let target = put(&mut e, real(2, 2, &[1., 2., 3., 4.]));
    let done = paste_tsv(
        &mut e,
        &target,
        Rect::new(0, 0, 1, 1).unwrap(),
        "5\t6\t7\r\n8\t9\t10\r\n",
    )
    .unwrap();
    assert_eq!(grid(&done.root), (2, 3, vec![5., 6., 7., 8., 9., 10.]));
}
#[test]
fn growth_is_bounded() {
    let mut e = Engine::default();
    let target = put(&mut e, real(2, 2, &[1., 2., 3., 4.]));
    let rows = MAX_GROWN_CELLS;
    assert!(
        reshape(&mut e, &target, Shape::InsertRows { at: 0, count: rows })
            .unwrap_err()
            .contains("cannot grow")
    );
    assert!(paste_tsv(&mut e, &target, Rect::new(rows, 0, 1, 1).unwrap(), "1").is_err());
    assert_eq!(grid(&e.array("X").unwrap()).0, 2);
}
#[test]
fn insert_and_delete_match_matlab() {
    let mut e = Engine::default();
    let m = real(3, 3, &[1., 2., 3., 4., 5., 6., 7., 8., 9.]);
    let t = put(&mut e, m.clone());
    // x(2,:) = []
    let r = reshape(&mut e, &t, Shape::DeleteRows { at: 1, count: 1 }).unwrap();
    assert_eq!(grid(&r.root), (2, 3, vec![1., 2., 3., 7., 8., 9.]));
    let t = put(&mut e, m.clone());
    let r = reshape(&mut e, &t, Shape::DeleteCols { at: 0, count: 2 }).unwrap();
    assert_eq!(grid(&r.root), (3, 1, vec![3., 6., 9.]));
    let t = put(&mut e, m.clone());
    let r = reshape(&mut e, &t, Shape::InsertRows { at: 3, count: 1 }).unwrap();
    assert_eq!(grid(&r.root).2[9..], [0., 0., 0.]);
    let t = put(&mut e, m.clone());
    let r = reshape(&mut e, &t, Shape::InsertCols { at: 1, count: 1 }).unwrap();
    assert_eq!(
        grid(&r.root),
        (3, 4, vec![1., 0., 2., 3., 4., 0., 5., 6., 7., 0., 8., 9.])
    );
    let t = put(&mut e, m.clone());
    let r = reshape(&mut e, &t, Shape::DeleteRows { at: 0, count: 3 }).unwrap();
    assert_eq!((r.root.rows(), r.root.cols()), (0, 3));
    let t = put(&mut e, m);
    assert!(reshape(&mut e, &t, Shape::DeleteRows { at: 2, count: 2 }).is_err());
    // Logical and int8 keep their class.
    let i8: Array =
        crate::TypedArray::from_ints(crate::NumericClass::Int8, 2, 1, [1, 2].into_iter())
            .unwrap()
            .into();
    let t = put(&mut e, i8);
    let r = reshape(&mut e, &t, Shape::InsertRows { at: 1, count: 2 }).unwrap();
    assert_eq!(grid(&r.root), (4, 1, vec![1., 0., 0., 2.]));
    assert!(matches!(r.root, Array::Typed(_)));
}
#[test]
fn restore_requires_the_current_binding() {
    let mut e = Engine::default();
    let first = real(1, 2, &[1., 2.]);
    let t = put(&mut e, first.clone());
    let edited = paste_tsv(&mut e, &t, Rect::new(0, 0, 1, 1).unwrap(), "9").unwrap();
    let t2 = Target::new("X".into(), edited.root.clone(), vec![]).unwrap();
    let back = restore(&mut e, &t2, &first).unwrap();
    assert_eq!(grid(&back.root), grid(&first));
    // Stale target (changed elsewhere) is refused.
    assert!(
        restore(&mut e, &t2, &first)
            .unwrap_err()
            .contains("changed")
    );
}

fn strings(rows: usize, cols: usize, data: &[Option<&str>]) -> Array {
    Array::Str(
        crate::StringArray::new(rows, cols, data.iter().map(|s| s.map(Arc::from)).collect())
            .unwrap(),
    )
}

fn cells(rows: usize, cols: usize, data: Vec<Array>) -> Array {
    CellArray::from_shared(
        rows,
        cols,
        data.into_iter().map(Arc::new).collect(),
        &AtomicBool::new(false),
    )
    .unwrap()
    .into()
}
fn char_row(text: &str) -> Array {
    CharacterRow::from_text(text, &AtomicBool::new(false))
        .unwrap()
        .into()
}
fn retarget(done: &Committed) -> Target {
    Target::new("X".into(), done.root.clone(), vec![]).unwrap()
}

#[test]
fn string_grid_paste_growth_missing_defaults_structure_and_exact_undo() {
    let mut e = Engine::default();
    let original = strings(1, 1, &[Some("a")]);
    let t = put(&mut e, original.clone());
    // s = "a"; s(3,2) = "x"; intervening elements are <missing>.
    let done = paste_tsv(&mut e, &t, Rect::new(2, 1, 1, 1).unwrap(), "x").unwrap();
    assert_eq!(
        copy_tsv(&e, &retarget(&done), Rect::new(0, 0, 3, 2).unwrap()).unwrap(),
        "a\t<missing>\n<missing>\t<missing>\n<missing>\tx"
    );
    let added = reshape(
        &mut e,
        &retarget(&done),
        Shape::InsertCols { at: 1, count: 1 },
    )
    .unwrap();
    assert_eq!((added.root.rows(), added.root.cols()), (3, 3));
    assert_eq!(cell_literal(&added.root, 2, 1).unwrap(), "<missing>");
    assert_eq!(cell_literal(&added.root, 2, 2).unwrap(), "x");
    let deleted = reshape(
        &mut e,
        &retarget(&added),
        Shape::DeleteRows { at: 0, count: 3 },
    )
    .unwrap();
    assert_eq!((deleted.root.rows(), deleted.root.cols()), (0, 3));
    let back = reshape(
        &mut e,
        &retarget(&deleted),
        Shape::InsertRows { at: 0, count: 1 },
    )
    .unwrap();
    assert_eq!(cell_literal(&back.root, 0, 2).unwrap(), "<missing>");
    let undo = restore(&mut e, &retarget(&back), &original).unwrap();
    assert!(undo.root.same_snapshot(&original));
    assert!(
        paste_tsv(
            &mut e,
            &retarget(&back),
            Rect::new(0, 0, 1, 1).unwrap(),
            "stale"
        )
        .unwrap_err()
        .contains("changed")
    );
}

#[test]
fn text_tsv_round_trips_empty_unicode_quotes_tabs_and_newlines() {
    let mut e = Engine::default();
    let original = strings(
        2,
        3,
        &[
            Some(""),
            Some("λ"),
            Some("a\tb"),
            Some("line\nnext"),
            Some("\"quoted\""),
            None,
        ],
    );
    let t = put(&mut e, original);
    let text = copy_tsv(&e, &t, Rect::new(0, 0, 2, 3).unwrap()).unwrap();
    let t = put(&mut e, strings(0, 0, &[]));
    let done = paste_tsv(&mut e, &t, Rect::new(0, 0, 1, 1).unwrap(), &text).unwrap();
    assert_eq!(
        copy_tsv(&e, &retarget(&done), Rect::new(0, 0, 2, 3).unwrap()).unwrap(),
        text
    );
    let done = paste_tsv(
        &mut e,
        &retarget(&done),
        Rect::new(0, 0, 1, 1).unwrap(),
        "a\t\r\nb\t\r\n",
    )
    .unwrap();
    assert_eq!(cell_literal(&done.root, 0, 1).unwrap(), "");
    assert_eq!(cell_literal(&done.root, 1, 1).unwrap(), "");
    let done = apply_literal(&mut e, &retarget(&done), 0, 0, "").unwrap();
    assert_eq!(cell_literal(&done.root, 0, 0).unwrap(), "");
    let long = "λ".repeat(4096);
    for value in [
        strings(1, 1, &[Some(&long)]),
        cells(1, 1, vec![char_row(&long)]),
    ] {
        assert_eq!(
            cell_preview(&value, 0, 0).unwrap(),
            format!("{}…", "λ".repeat(128))
        );
        assert_eq!(cell_literal(&value, 0, 0).unwrap(), long);
    }
    let t = put(&mut e, strings(1, 2, &[Some("<missing>"), None]));
    let text = copy_tsv(&e, &t, Rect::new(0, 0, 1, 2).unwrap()).unwrap();
    assert_eq!(text, "\"<missing>\"\t<missing>");
    let done = paste_tsv(&mut e, &t, Rect::new(0, 0, 1, 1).unwrap(), &text).unwrap();
    let Array::Str(s) = done.root else { panic!() };
    assert_eq!(s.data()[0].as_deref(), Some("<missing>"));
    assert!(s.data()[1].is_none());
}

#[test]
fn cell_grid_paste_classes_growth_defaults_inline_and_undo() {
    let mut e = Engine::default();
    let original = cells(1, 1, vec![char_row("old")]);
    let t = put(&mut e, original.clone());
    // c = {'old'}; c(3,2:3) = {5,'hello'}; growth pads c with [].
    let done = paste_tsv(&mut e, &t, Rect::new(2, 1, 1, 1).unwrap(), "5\thello").unwrap();
    let Array::Cell(c) = &done.root else { panic!() };
    assert_eq!((c.rows(), c.cols()), (3, 3));
    assert!(matches!(
        c.get_linear(5).unwrap().as_ref(),
        Array::Numeric(_)
    ));
    assert_eq!(
        c.get_linear(5).unwrap().scalar(0, 0).unwrap(),
        Scalar::Real(5.)
    );
    assert!(matches!(
        c.get_linear(8).unwrap().as_ref(),
        Array::Character(_)
    ));
    assert!(c.get_linear(1).unwrap().is_empty());
    assert_eq!(cell_literal(&done.root, 2, 2).unwrap(), "hello");
    let done = apply_literal(&mut e, &retarget(&done), 2, 2, "7").unwrap();
    let Array::Cell(c) = &done.root else { panic!() };
    assert!(matches!(
        c.get_linear(8).unwrap().as_ref(),
        Array::Numeric(_)
    ));
    let inserted = reshape(
        &mut e,
        &retarget(&done),
        Shape::InsertRows { at: 1, count: 1 },
    )
    .unwrap();
    let deleted = reshape(
        &mut e,
        &retarget(&inserted),
        Shape::DeleteCols { at: 1, count: 1 },
    )
    .unwrap();
    assert_eq!((deleted.root.rows(), deleted.root.cols()), (4, 2));
    let undo = restore(&mut e, &retarget(&deleted), &original).unwrap();
    assert!(undo.root.same_snapshot(&original));

    let t = put(
        &mut e,
        cells(
            1,
            3,
            vec![
                LogicalMatrix::from_canonical_vec(1, 1, vec![1])
                    .unwrap()
                    .into(),
                strings(1, 1, &[Some("old")]),
                real(1, 2, &[1., 2.]),
            ],
        ),
    );
    let done = apply_literal(&mut e, &t, 0, 0, "0").unwrap();
    let Array::Cell(c) = &done.root else { panic!() };
    assert!(matches!(
        c.get_linear(0).unwrap().as_ref(),
        Array::Logical(_)
    ));
    let done = apply_literal(&mut e, &retarget(&done), 0, 1, "5").unwrap();
    let Array::Cell(c) = &done.root else { panic!() };
    assert!(matches!(c.get_linear(1).unwrap().as_ref(), Array::Str(_)));
    assert!(!cell_editable(&done.root, 0, 2));
    assert_eq!(cell_literal(&done.root, 0, 2).unwrap(), "1×2 double");
    assert!(apply_literal(&mut e, &retarget(&done), 0, 2, "3").is_err());
}

fn edit_table() -> Array {
    let mut parts = crate::table_array::TableParts::new(
        vec![
            "Amount".into(),
            "Flag".into(),
            "Count".into(),
            "Label".into(),
            "Text".into(),
        ],
        vec![
            real(2, 1, &[1., 2.]),
            LogicalMatrix::from_canonical_vec(2, 1, vec![1, 0])
                .unwrap()
                .into(),
            crate::TypedArray::from_ints(crate::NumericClass::Int16, 2, 1, [3, 4].into_iter())
                .unwrap()
                .into(),
            strings(2, 1, &[Some("a"), Some("b")]),
            cells(2, 1, vec![char_row("first"), char_row("last")]),
        ],
        2,
    );
    parts.row_names = Some(vec!["Row1".into(), "second".into()]);
    parts.description = "metadata stays".into();
    parts.units = vec!["kg".into(), "".into(), "".into(), "".into(), "".into()];
    crate::TableArray::new(parts).unwrap().into()
}

#[test]
fn table_grid_paste_growth_defaults_types_atomic_refusal_and_exact_undo() {
    let mut e = Engine::default();
    let original = edit_table();
    let t = put(&mut e, original.clone());
    // T.Amount(4) = 9; new rows default to 0, false, 0, <missing>, {''}.
    let done = paste_tsv(&mut e, &t, Rect::new(3, 0, 1, 1).unwrap(), "9").unwrap();
    let Array::Table(table) = &done.root else {
        panic!()
    };
    assert_eq!((table.rows(), table.cols()), (4, 5));
    for (c, expected) in ["0", "0", "0", "<missing>", ""].iter().enumerate() {
        assert_eq!(cell_literal(&done.root, 2, c).unwrap(), *expected);
    }
    assert_eq!(
        table.row_names().unwrap(),
        ["Row1", "second", "Row2", "Row3"]
    );
    assert_eq!(table.parts().description, "metadata stays");
    assert_eq!(table.parts().units[0], "kg");
    let t = retarget(&done);
    for bad in ["abc", "7\ttrue\tnot_an_integer"] {
        assert!(paste_tsv(&mut e, &t, Rect::new(0, 0, 1, 1).unwrap(), bad).is_err());
        assert!(e.array("X").unwrap().same_snapshot(&done.root));
    }
    let pasted = paste_tsv(
        &mut e,
        &t,
        Rect::new(0, 0, 1, 1).unwrap(),
        "5\tfalse\t17\tnew\t23",
    )
    .unwrap();
    let Array::Table(table) = &pasted.root else {
        panic!()
    };
    assert!(matches!(table.column(0), Array::Numeric(_)));
    assert!(matches!(table.column(1), Array::Logical(_)));
    assert!(matches!(table.column(2), Array::Typed(a) if a.class() == crate::NumericClass::Int16));
    assert!(matches!(table.column(3), Array::Str(_)));
    let Array::Cell(c) = table.column(4) else {
        panic!()
    };
    assert!(matches!(
        c.get_linear(0).unwrap().as_ref(),
        Array::Character(_)
    ));
    assert_eq!(cell_literal(&pasted.root, 0, 4).unwrap(), "23");
    assert!(
        reshape(
            &mut e,
            &retarget(&pasted),
            Shape::InsertCols { at: 0, count: 1 }
        )
        .is_err()
    );
    let undo = restore(&mut e, &retarget(&pasted), &original).unwrap();
    assert!(undo.root.same_snapshot(&original));
    assert!(
        apply_literal(&mut e, &t, 0, 0, "4")
            .unwrap_err()
            .contains("changed")
    );
}

#[test]
fn table_row_insert_delete_keep_names_metadata_and_defaults() {
    let mut e = Engine::default();
    let original = edit_table();
    let t = put(&mut e, original.clone());
    // T = [T(1,:); defaultRow; T(2,:)]; then T(2,:) = [].
    let inserted = reshape(&mut e, &t, Shape::InsertRows { at: 1, count: 1 }).unwrap();
    let Array::Table(table) = &inserted.root else {
        panic!()
    };
    assert_eq!(table.row_names().unwrap(), ["Row1", "Row2", "second"]);
    assert_eq!(cell_literal(&inserted.root, 1, 4).unwrap(), "");
    let deleted = reshape(
        &mut e,
        &retarget(&inserted),
        Shape::DeleteRows { at: 1, count: 1 },
    )
    .unwrap();
    assert_eq!(
        copy_tsv(&e, &retarget(&deleted), Rect::new(0, 0, 2, 5).unwrap()).unwrap(),
        "1\t1\t3\ta\tfirst\n2\t0\t4\tb\tlast"
    );
    let empty = reshape(
        &mut e,
        &retarget(&deleted),
        Shape::DeleteRows { at: 0, count: 2 },
    )
    .unwrap();
    let regrown = reshape(
        &mut e,
        &retarget(&empty),
        Shape::InsertRows { at: 0, count: 1 },
    )
    .unwrap();
    assert_eq!((regrown.root.rows(), regrown.root.cols()), (1, 5));
    assert_eq!(cell_literal(&regrown.root, 0, 4).unwrap(), "");
    let undo = restore(&mut e, &retarget(&regrown), &original).unwrap();
    assert!(undo.root.same_snapshot(&original));
}

#[test]
fn container_edits_cancel_atomically_preserve_aliases_and_enforce_limits() {
    for original in [
        strings(1, 1, &[Some("old")]),
        cells(1, 1, vec![char_row("old")]),
        edit_table(),
    ] {
        let mut e = Engine::default();
        let t = put(&mut e, original.clone());
        e.merge_workspace(BTreeMap::from([("Alias".into(), original.clone())]))
            .unwrap();
        let cancel = e.cancellation();
        let result = apply(
            &mut e,
            &t,
            Rect::new(0, 0, 1, 1).unwrap(),
            Input::Tsv("1"),
            &mut |stage| {
                if stage == Stage::BeforeCommit {
                    cancel.store(true, Ordering::Relaxed);
                }
                Ok(())
            },
        );
        assert!(result.unwrap_err().contains("cancelled"));
        cancel.store(false, Ordering::Relaxed);
        assert!(e.array("X").unwrap().same_snapshot(&original));
        assert!(e.array("Alias").unwrap().same_snapshot(&original));
        assert!(
            reshape(
                &mut e,
                &t,
                Shape::InsertRows {
                    at: 0,
                    count: usize::MAX
                }
            )
            .is_err()
        );
        assert!(
            paste_tsv(
                &mut e,
                &t,
                Rect::new(MAX_GROWN_CELLS, 0, 1, 1).unwrap(),
                "1"
            )
            .is_err()
        );
    }
}

#[test]
fn nested_container_grids_rebuild_only_the_path_and_refuse_stale_roots() {
    for leaf in [
        strings(1, 1, &[Some("old")]),
        cells(1, 1, vec![char_row("old")]),
        edit_table(),
    ] {
        let mut e = Engine::default();
        let root = cells(1, 2, vec![leaf.clone(), leaf.clone()]);
        let root_target = put(&mut e, root.clone());
        let target = Target::new("X".into(), root_target.expected_root().clone(), vec![0]).unwrap();
        let done = paste_tsv(&mut e, &target, Rect::new(2, 0, 1, 1).unwrap(), "8").unwrap();
        let Array::Cell(c) = &done.root else { panic!() };
        assert_eq!(cell_literal(c.get_linear(0).unwrap(), 2, 0).unwrap(), "8");
        assert!(c.get_linear(1).unwrap().same_snapshot(&leaf));
        assert!(
            copy_tsv(&e, &target, Rect::new(0, 0, 1, 1).unwrap())
                .unwrap_err()
                .contains("changed")
        );
        let current = Target::new("X".into(), done.root, vec![0]).unwrap();
        let undo = restore(&mut e, &current, &root).unwrap();
        assert!(undo.root.same_snapshot(&root));
    }
}

#[test]
fn table_unsupported_columns_stay_read_only_and_survive_row_operations() {
    let mut e = Engine::default();
    let mut parts = crate::table_array::TableParts::new(
        vec!["Amount".into(), "Wave".into()],
        vec![
            real(2, 1, &[1., 2.]),
            ComplexMatrix::from_column_vec(
                2,
                1,
                vec![Complex64 { re: 3., im: 4. }, Complex64 { re: 5., im: 6. }],
                &AtomicBool::new(false),
            )
            .unwrap()
            .into(),
        ],
        2,
    );
    parts.description = "readonly column".into();
    let original: Array = crate::TableArray::new(parts).unwrap().into();
    let t = put(&mut e, original.clone());
    assert!(!cell_editable(&original, 0, 1));
    assert!(
        paste_tsv(&mut e, &t, Rect::new(0, 0, 1, 1).unwrap(), "7\t8")
            .unwrap_err()
            .contains("read-only")
    );
    assert!(e.array("X").unwrap().same_snapshot(&original));
    let inserted = reshape(&mut e, &t, Shape::InsertRows { at: 1, count: 1 }).unwrap();
    assert_eq!(cell_literal(&inserted.root, 1, 1).unwrap(), "0+0i");
    assert_eq!(cell_literal(&inserted.root, 2, 1).unwrap(), "5+6i");
    let deleted = reshape(
        &mut e,
        &retarget(&inserted),
        Shape::DeleteRows { at: 1, count: 1 },
    )
    .unwrap();
    assert_eq!(cell_literal(&deleted.root, 1, 1).unwrap(), "5+6i");
}

#[test]
fn cell_self_copy_paste_keeps_values_and_types() {
    let mut e = Engine::default();
    // C = {[1 2], true, '001'}
    let original = cells(
        1,
        3,
        vec![
            real(1, 2, &[1., 2.]),
            Array::Logical(crate::LogicalMatrix::scalar(true)),
            char_row("001"),
        ],
    );
    let t = put(&mut e, original.clone());
    let all = Rect::new(0, 0, 1, 3).unwrap();
    let text = copy_tsv(&e, &t, all).unwrap();
    let done = paste_tsv(&mut e, &t, Rect::new(0, 0, 1, 1).unwrap(), &text).unwrap();
    let (Array::Cell(after), Array::Cell(before)) = (&done.root, &original) else {
        panic!()
    };
    for i in 0..3 {
        let (a, b) = (after.get_linear(i).unwrap(), before.get_linear(i).unwrap());
        assert_eq!(a.class(), b.class(), "element {i}");
        assert_eq!(
            cell_literal(&done.root, 0, i).unwrap(),
            cell_literal(&original, 0, i).unwrap()
        );
    }
    // A summary pasted elsewhere is refused, not stored as text.
    let err = paste_tsv(
        &mut e,
        &retarget(&done),
        Rect::new(0, 2, 1, 1).unwrap(),
        "1×2 double",
    )
    .unwrap_err();
    assert!(err.contains("open the cell"), "{err}");
}

#[test]
fn single_column_text_copy_keeps_a_trailing_empty_row() {
    let mut e = Engine::default();
    let t = put(&mut e, strings(2, 1, &[Some("a"), Some("")]));
    let text = copy_tsv(&e, &t, Rect::new(0, 0, 2, 1).unwrap()).unwrap();
    let t = put(&mut e, strings(2, 1, &[Some("x"), Some("y")]));
    let done = paste_tsv(&mut e, &t, Rect::new(0, 0, 1, 1).unwrap(), &text).unwrap();
    let Array::Str(s) = done.root else { panic!() };
    assert_eq!(s.data()[0].as_deref(), Some("a"));
    assert_eq!(s.data()[1].as_deref(), Some(""));
}
