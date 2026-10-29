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
