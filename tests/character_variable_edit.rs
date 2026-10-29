use matrust::{
    Array, CellArray, CharacterRow, Engine, Scalar,
    variable_edit::{self, Rect, Target},
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

fn chars(rows: usize, cols: usize, units: Vec<u16>) -> Array {
    CharacterRow::from_column_major(rows, cols, units, &AtomicBool::new(false))
        .unwrap()
        .into()
}
fn units(value: &Array) -> &[u16] {
    let Array::Character(text) = value else {
        panic!("Expected character array")
    };
    text.units()
}
fn child(value: &Array, index: usize) -> &Array {
    let Array::Cell(cell) = value else {
        panic!("Expected cell")
    };
    cell.get_linear(index).unwrap()
}
fn target(e: &Engine, path: Vec<usize>) -> Target {
    Target::new("A".into(), e.array("A").unwrap(), path).unwrap()
}
fn unchanged(e: &Engine, before: &BTreeMap<String, Array>) {
    assert_eq!(e.workspace_count(), before.len());
    for (name, value) in before {
        assert!(
            value.same_snapshot(&e.array(name).unwrap()),
            "Changed {name}"
        );
    }
}

#[test]
fn exact_units_root_and_nested_rectangles_preserve_shape_aliases_and_siblings() {
    for nested in [false, true] {
        let stop = AtomicBool::new(false);
        let leaf = chars(
            3,
            3,
            vec![65, 0, 0xd800, 0xdc00, 65535, 66, 0xd83d, 0xde00, 67],
        );
        let shared = Arc::new(leaf.clone());
        let root = if nested {
            CellArray::from_shared(1, 2, vec![shared.clone(), shared], &stop)
                .unwrap()
                .into()
        } else {
            leaf.clone()
        };
        let mut e = Engine::default();
        e.merge_workspace(BTreeMap::from([
            ("A".into(), root.clone()),
            ("Alias".into(), root.clone()),
            ("Leaf".into(), leaf.clone()),
        ]))
        .unwrap();
        let path = if nested { vec![0] } else { vec![] };
        let before_bytes = e.workspace_bytes();
        let expected = target(&e, path.clone());
        let rect = Rect::new(1, 1, 2, 2).unwrap();
        let out = variable_edit::paste_tsv(
            &mut e,
            &expected,
            rect,
            "\\u0000\t0xD800\r\n65535\t\\uDC00\r\n",
        )
        .unwrap();
        assert_eq!(out.rectangle, rect);
        let result = if nested {
            child(&out.root, 0)
        } else {
            &out.root
        };
        assert_eq!((result.rows(), result.cols()), (3, 3));
        assert_eq!(
            units(result),
            &[65, 0, 0xd800, 0xdc00, 0, 65535, 0xd83d, 0xd800, 0xdc00]
        );
        assert_eq!(e.workspace_bytes(), before_bytes);
        assert!(e.array("Alias").unwrap().same_snapshot(&root));
        assert!(e.array("Leaf").unwrap().same_snapshot(&leaf));
        if nested {
            assert!(child(&out.root, 1).same_snapshot(&leaf));
        }
        assert!(
            variable_edit::copy_tsv(&e, &expected, rect).is_err(),
            "Old root is stale"
        );
        assert_eq!(
            variable_edit::copy_tsv(&e, &target(&e, path), rect).unwrap(),
            "\\u0000\t\\uD800\n\\uFFFF\t\\uDC00"
        );
    }
}

#[test]
fn invalid_units_shapes_and_prepared_values_are_atomic_before_publication() {
    let mut e = Engine::default();
    let a = chars(2, 2, vec![0, 0xd800, 0xdc00, 65535]);
    e.merge_workspace(BTreeMap::from([
        ("A".into(), a.clone()),
        ("Alias".into(), a),
    ]))
    .unwrap();
    let expected = target(&e, vec![]);
    let before = e.workspace();
    let rect = Rect::new(0, 0, 2, 2).unwrap();
    for bad in [
        "\\u0000\t\\uD800\n\\uDC00\t65536",
        "1\t2\n3\t-1",
        "1\t2\n3\t1.5",
        "1\t2\n3\tNaN",
        "1\t2\n3\ttrue",
        "1\t2\n3\tA=4",
        "1\t2\n3\t\\uD80",
        "1\t2\n3\t\\uGGGG",
        "1\t2\n3\tλ",
        "1\t2\n3",
        "1\t2\t3",
    ] {
        assert!(
            variable_edit::paste_tsv(&mut e, &expected, rect, bad).is_err(),
            "Accepted {bad:?}"
        );
        unchanged(&e, &before);
    }
    for value in [
        Scalar::Real(65536.),
        Scalar::Real(-1.),
        Scalar::Real(1.5),
        Scalar::Real(f64::NAN),
        Scalar::Logical(true),
    ] {
        assert!(
            variable_edit::apply_values(
                &mut e,
                &expected,
                Rect::new(0, 0, 1, 1).unwrap(),
                &[value]
            )
            .is_err()
        );
        unchanged(&e, &before);
    }
    e.cancellation().store(true, Ordering::Relaxed);
    assert!(variable_edit::paste_tsv(&mut e, &expected, rect, "1\t2\n3\t4").is_err());
    assert!(variable_edit::copy_tsv(&e, &expected, rect).is_err());
    unchanged(&e, &before);
    e.cancellation().store(false, Ordering::Relaxed);
    variable_edit::apply_literal(&mut e, &expected, 0, 0, "\\uDFFF").unwrap();
    assert_eq!(units(&e.array("A").unwrap())[0], 0xdfff);
}

#[test]
fn text_row_copy_is_exact_and_never_flattens_or_replaces_invalid_units() {
    for (rows, cols, data, expected) in [
        (1, 4, vec![65, 0xd83d, 0xde00, 0xffff], Some("A😀\u{ffff}")),
        (1, 3, vec![65, 0, 66], None),
        (1, 0, vec![], Some("")),
        (0, 0, vec![], Some("")),
        (0, 3, vec![], None),
        (3, 0, vec![], None),
        (2, 2, vec![65, 66, 67, 68], None),
        (1, 1, vec![0xd800], None),
        (1, 1, vec![0xdc00], None),
    ] {
        let mut e = Engine::default();
        let value = chars(rows, cols, data.clone());
        e.merge_workspace(BTreeMap::from([("A".into(), value.clone())]))
            .unwrap();
        let target = target(&e, vec![]);
        let out = variable_edit::copy_text_row(&e, &target);
        if let Some(expected) = expected {
            assert_eq!(out.unwrap(), expected);
        } else {
            assert!(out.is_err());
        }
        assert!(e.array("A").unwrap().same_snapshot(&value));
        assert_eq!(units(&e.array("A").unwrap()), data);
    }
}

#[test]
fn maximum_character_grid_copy_and_paste_uses_row_major_tsv_without_shape_change() {
    let mut e = Engine::default();
    let value = chars(256, 256, (0..=u16::MAX).collect());
    e.merge_workspace(BTreeMap::from([
        ("A".into(), value.clone()),
        ("Alias".into(), value.clone()),
    ]))
    .unwrap();
    let rect = Rect::new(0, 0, 256, 256).unwrap();
    let expected = target(&e, vec![]);
    let text = variable_edit::copy_tsv(&e, &expected, rect).unwrap();
    assert_eq!(text.lines().count(), 256);
    assert_eq!(
        text.lines().next().unwrap().split('\t').nth(1),
        Some("\\u0100")
    );
    assert!(text.ends_with("\\uFFFF"));
    let out = variable_edit::paste_tsv(&mut e, &expected, rect, &text).unwrap();
    assert_eq!(units(&out.root), units(&value));
    assert!(!out.root.same_snapshot(&value));
    assert!(e.array("Alias").unwrap().same_snapshot(&value));
}
