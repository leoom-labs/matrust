use matrust::{
    Array, CellArray, CharacterRow, Engine, Matrix,
    variable_edit::cell_replace::{Binding, CellTarget, replace},
};
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::AtomicBool},
};
fn binding(engine: &Engine, name: &str) -> Binding {
    Binding::new(name.into(), engine.array(name).unwrap()).unwrap()
}
fn leaf<'a>(mut value: &'a Array, path: &[usize]) -> &'a Array {
    for &index in path {
        let Array::Cell(cell) = value else {
            panic!("cell")
        };
        value = cell.get_linear(index).unwrap();
    }
    value
}
#[test]
fn all_typed_values_replace_nested_slot_without_losing_shape_bits_or_other_aliases() {
    for setup in [
        "S=[-0 2;3 4];",
        "S=logical([0 1;1 0]);",
        "S=complex([1 2],[3 -0]);",
        "S=reshape(char([0 55296 65535 56320 65 66]),2,3);",
        "S=cell(1,1);S{1}=char([0 55296]);",
        "S=figure;close(gcf);",
    ] {
        let mut engine = Engine::default();
        engine.execute(&format!("A=[11 12];C=cell(1,2);C{{1}}=A;C{{2}}=A;D=cell(1,2);D{{1}}=C;D{{2}}=C;Alias=D;{setup}")).unwrap();
        let before = engine.array("D").unwrap();
        let source = engine.array("S").unwrap();
        let target = CellTarget::new(binding(&engine, "D"), vec![0, 1]).unwrap();
        let input = binding(&engine, "S");
        let after = replace(&mut engine, &target, &input).unwrap();
        assert!(after.same_snapshot(&engine.array("D").unwrap()));
        assert!(leaf(&after, &[0, 1]).same_snapshot(&source));
        assert_eq!(
            (leaf(&after, &[0, 1]).rows(), leaf(&after, &[0, 1]).cols()),
            (source.rows(), source.cols())
        );
        assert!(leaf(&after, &[0, 0]).same_snapshot(leaf(&before, &[0, 0])));
        assert!(leaf(&after, &[1]).same_snapshot(leaf(&before, &[1])));
        let (Array::Cell(old), Array::Cell(new)) = (&before, &after) else {
            panic!()
        };
        assert!(Arc::ptr_eq(
            old.get_linear(1).unwrap(),
            new.get_linear(1).unwrap()
        ));
        assert!(engine.array("Alias").unwrap().same_snapshot(&before));
        assert!(engine.array("S").unwrap().same_snapshot(&source));
        if let Array::Character(text) = leaf(&after, &[0, 1]) {
            assert_eq!(text.units(), [0, 55296, 65535, 56320, 65, 66]);
        }
    }
}
#[test]
fn stale_source_target_removed_bindings_and_invalid_paths_are_atomic() {
    let mut e = Engine::default();
    e.execute("C=cell(1,1);C{1}=1;S=[2 3];").unwrap();
    let original = e.array("C").unwrap();
    let source = binding(&e, "S");
    let target = CellTarget::new(binding(&e, "C"), vec![0]).unwrap();
    e.execute("S=[4 5];").unwrap();
    assert!(
        replace(&mut e, &target, &source)
            .unwrap_err()
            .contains("changed")
    );
    assert!(e.array("C").unwrap().same_snapshot(&original));
    let source = binding(&e, "S");
    e.execute("C{1}=9;").unwrap();
    let changed = e.array("C").unwrap();
    assert!(replace(&mut e, &target, &source).is_err());
    assert!(e.array("C").unwrap().same_snapshot(&changed));
    let target = CellTarget::new(binding(&e, "C"), vec![0]).unwrap();
    e.execute("clear S;").unwrap();
    assert!(
        replace(&mut e, &target, &source)
            .unwrap_err()
            .contains("no longer exists")
    );
    assert!(CellTarget::new(binding(&e, "C"), vec![]).is_err());
    assert!(CellTarget::new(binding(&e, "C"), vec![1]).is_err());
    assert!(CellTarget::new(binding(&e, "C"), vec![0, 0]).is_err());
    assert!(CellTarget::new(binding(&e, "C"), vec![0; 17]).is_err());
}
#[test]
fn inserting_own_old_root_is_finite_and_depth_or_workspace_failure_publishes_nothing() {
    let stop = AtomicBool::new(false);
    let mut e = Engine::default();
    e.execute("C=cell(1,1);C{1}=7;").unwrap();
    let before = e.array("C").unwrap();
    let source = binding(&e, "C");
    let target = CellTarget::new(source.clone(), vec![0]).unwrap();
    let after = replace(&mut e, &target, &source).unwrap();
    assert!(leaf(&after, &[0]).same_snapshot(&before));
    let mut deep: Array = Matrix::scalar(1.).into();
    for _ in 0..16 {
        deep = CellArray::from_shared(1, 1, vec![Arc::new(deep)], &stop)
            .unwrap()
            .into();
    }
    e.merge_workspace(BTreeMap::from([("Deep".into(), deep)]))
        .unwrap();
    let current = e.array("C").unwrap();
    let target = CellTarget::new(binding(&e, "C"), vec![0]).unwrap();
    let deep_source = binding(&e, "Deep");
    assert!(replace(&mut e, &target, &deep_source).is_err());
    assert!(e.array("C").unwrap().same_snapshot(&current));
    // Aggregate workspace limit is stricter than this individual resulting cell.
    let large: Array = Matrix::from_column_vec(1, 1_000_000, vec![2.; 1_000_000])
        .unwrap()
        .into();
    let mut updates = BTreeMap::new();
    for i in 0..33 {
        updates.insert(format!("V{i}"), large.clone());
    }
    e.merge_workspace(updates).unwrap();
    let source = binding(&e, "V0");
    assert!(
        replace(&mut e, &target, &source)
            .unwrap_err()
            .contains("Workspace exceeds")
    );
    assert!(e.array("C").unwrap().same_snapshot(&current));
}
#[test]
fn shaped_empty_character_and_cell_sources_are_not_normalized() {
    let stop = AtomicBool::new(false);
    let mut e = Engine::default();
    e.execute("C=cell(1,1);").unwrap();
    for value in [
        Array::Character(CharacterRow::from_column_major(0, 7, vec![], &stop).unwrap()),
        Array::Cell(CellArray::from_shared(3, 0, vec![], &stop).unwrap()),
    ] {
        e.merge_workspace(BTreeMap::from([("S".into(), value.clone())]))
            .unwrap();
        let target = CellTarget::new(binding(&e, "C"), vec![0]).unwrap();
        let source = binding(&e, "S");
        let result = replace(&mut e, &target, &source).unwrap();
        assert!(leaf(&result, &[0]).same_snapshot(&value));
    }
}

#[test]
fn literal_and_prepared_leaf_edits_return_the_actual_committed_root() {
    use matrust::{
        Scalar,
        variable_edit::{Rect, Target, apply_literal, apply_values},
    };
    for nested in [false, true] {
        for setup in [
            "A=[1 2];",
            "A=logical([1 0]);",
            "A=complex([1 2],[3 4]);",
            "A=char([0 55296]);",
        ] {
            let mut e = Engine::default();
            e.execute(&format!("{setup}C=cell(1,2);C{{1}}=A;C{{2}}=A;Alias=C;"))
                .unwrap();
            let name = if nested { "C" } else { "A" };
            let path = if nested { vec![0] } else { vec![] };
            let old = e.array("C").unwrap();
            let target = Target::new(name.into(), e.array(name).unwrap(), path.clone()).unwrap();
            let changed = apply_literal(&mut e, &target, 0, 0, "9").unwrap();
            assert!(changed.root.same_snapshot(&e.array(name).unwrap()));
            let target = Target::new(name.into(), changed.root, path).unwrap();
            let prepared = apply_values(
                &mut e,
                &target,
                Rect::new(0, 1, 1, 1).unwrap(),
                &[Scalar::Real(7.)],
            )
            .unwrap();
            assert!(prepared.root.same_snapshot(&e.array(name).unwrap()));
            assert!(e.array("Alias").unwrap().same_snapshot(&old));
            if nested {
                assert!(leaf(&prepared.root, &[1]).same_snapshot(leaf(&old, &[1])));
            }
        }
    }
}
