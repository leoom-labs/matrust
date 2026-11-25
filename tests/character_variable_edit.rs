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
    ] {
        assert!(
            variable_edit::paste_tsv(&mut e, &expected, rect, bad).is_err(),
            "Accepted {bad:?}"
        );
        unchanged(&e, &before);
    }
    {
        // A 1x3 block on a 2x2 selection lands at the anchor and grows the
        // char matrix like MATLAB `A(1,1:3) = char([1 2 3])`, padding char(0).
        let mut g = Engine::default();
        g.merge_workspace(BTreeMap::from([(
            "A".into(),
            chars(2, 2, vec![0, 0xd800, 0xdc00, 65535]),
        )]))
        .unwrap();
        let t = target(&g, vec![]);
        variable_edit::paste_tsv(&mut g, &t, rect, "1\t2\t3").unwrap();
        let grown = g.array("A").unwrap();
        assert_eq!((grown.rows(), grown.cols()), (2, 3));
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

/// Octave `printf('%g\n', 1)` prints `1`. MathWorks format short of 1 is the
/// same text (https://www.mathworks.com/help/matlab/ref/format.html). Copying
/// a cell writes that text. Pasting it into a character cell keeps the
/// characters: `001` stays three characters, and `1` stays the character `1`.
#[test]
fn review_1111_cell_paste_keeps_text_whose_display_is_not_the_number() {
    let stop = AtomicBool::new(false);
    let char_001: Array = CharacterRow::from_text("001", &stop).unwrap().into();
    let string_001 = Array::Str(matrust::StringArray::scalar("001").unwrap());
    let other: Array = CharacterRow::from_text("x", &stop).unwrap().into();
    let root = CellArray::from_shared(
        1,
        4,
        vec![
            Arc::new(char_001),
            Arc::new(string_001),
            Arc::new(other.clone()),
            Arc::new(other),
        ],
        &stop,
    )
    .unwrap()
    .into();
    let mut e = Engine::default();
    e.merge_workspace(BTreeMap::from([("A".into(), root)]))
        .unwrap();
    let request = target(&e, vec![]);
    let copied_char =
        variable_edit::copy_tsv(&e, &request, Rect::new(0, 0, 1, 1).unwrap()).unwrap();
    let copied_string =
        variable_edit::copy_tsv(&e, &request, Rect::new(0, 1, 1, 1).unwrap()).unwrap();
    // Reference value: the field text is 001, not the short display "1".
    assert_eq!(copied_char, "001");
    assert_eq!(copied_string, "001");
    variable_edit::paste_tsv(
        &mut e,
        &request,
        Rect::new(0, 2, 1, 1).unwrap(),
        &copied_char,
    )
    .unwrap();
    let after_char = target(&e, vec![]);
    variable_edit::paste_tsv(
        &mut e,
        &after_char,
        Rect::new(0, 3, 1, 1).unwrap(),
        &copied_string,
    )
    .unwrap();
    let result = e.array("A").unwrap();
    // '001' stays the character row whose units are 48, 48, 49.
    assert_eq!(child(&result, 2).class(), "char");
    assert_eq!(units(child(&result, 2)), &[48, 48, 49]);
    // string "001" stays text (a character row), not double 1.
    assert_eq!(child(&result, 3).class(), "char");
    assert_eq!(units(child(&result, 3)), &[48, 48, 49]);
    // A character cell keeps pasted text, including a field that parses as a number.
    let after_text = target(&e, vec![]);
    variable_edit::paste_tsv(&mut e, &after_text, Rect::new(0, 2, 1, 1).unwrap(), "1").unwrap();
    let result = e.array("A").unwrap();
    assert_eq!(child(&result, 2).class(), "char");
    assert_eq!(units(child(&result, 2)), &[49]);
}

#[test]
fn review_s1113_numeric_cell_paste_parses_every_real_literal() {
    // Paste into a numeric cell parses every real literal. A char or string
    // cell, and a quoted field, stays text. Spaces inside quotes are kept.
    let mut e = Engine::default();
    let paste = |e: &mut Engine, text: &str| {
        let req = target(e, vec![]);
        variable_edit::paste_tsv(e, &req, Rect::new(0, 0, 1, 1).unwrap(), text).unwrap();
        e.array("A").unwrap()
    };
    for (text, want) in [
        ("1.0", 1.0),
        ("+1", 1.0),
        ("1e2", 100.0),
        ("Inf", f64::INFINITY),
        ("-0.5", -0.5),
        ("001", 1.0),
    ] {
        e.execute("A={0};").unwrap();
        let result = paste(&mut e, text);
        assert_eq!(child(&result, 0).class(), "double", "{text}");
        let got = child(&result, 0).scalar(0, 0).unwrap().into_real().unwrap();
        assert!(
            got.to_bits() == want.to_bits()
                || got.is_infinite() && want.is_infinite() && got.is_sign_positive(),
            "{text}: got {got}, want {want}"
        );
    }
    e.execute("A={0};").unwrap();
    let result = paste(&mut e, "\"abc\"");
    assert_eq!(child(&result, 0).class(), "char");
    assert_eq!(units(child(&result, 0)), &[97, 98, 99]);
    e.execute("A={0};").unwrap();
    let result = paste(&mut e, "\" 1 \"");
    assert_eq!(child(&result, 0).class(), "char");
    assert_eq!(units(child(&result, 0)), &[32, 49, 32]);
    e.execute("A={0};").unwrap();
    let result = paste(&mut e, "'abc'");
    assert_eq!(child(&result, 0).class(), "char");
    assert_eq!(units(child(&result, 0)), &[97, 98, 99]);
    e.execute("A={0};").unwrap();
    let result = paste(&mut e, "' abc '");
    assert_eq!(child(&result, 0).class(), "char");
    assert_eq!(units(child(&result, 0)), &[32, 97, 98, 99, 32]);
    e.execute("A={'x'};").unwrap();
    let result = paste(&mut e, "1.0");
    assert_eq!(child(&result, 0).class(), "char");
    assert_eq!(units(child(&result, 0)), &[49, 46, 48]);
    let stop = AtomicBool::new(false);
    let root = CellArray::from_shared(
        1,
        1,
        vec![Arc::new(Array::Str(
            matrust::StringArray::scalar("x").unwrap(),
        ))],
        &stop,
    )
    .unwrap()
    .into();
    e.merge_workspace(BTreeMap::from([("A".into(), root)]))
        .unwrap();
    let result = paste(&mut e, "1.0");
    let Array::Str(text) = child(&result, 0) else {
        panic!("string cell became {}", child(&result, 0).class());
    };
    assert_eq!(text.data()[0].as_deref(), Some("1.0"));
}
