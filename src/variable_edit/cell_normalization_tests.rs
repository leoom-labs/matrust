//! Requires the separately reviewed storage/outline and cell replacement layers.
use super::cell_replace::{Binding, CellTarget, replace};
use crate::{Array, CellArray, Engine, Matrix};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[test]
fn cell_replacement_crosses_real_retention_limit_and_returns_normalized_root_after_stop_retry() {
    let cancel = AtomicBool::new(false);
    // Only 8 MiB of physical source data. Each small derived scalar retains that
    // source in its optional proof, and the workspace charges every alias.
    let backing = Matrix::from_column_vec(1024 * 1024, 1, vec![2.; 1024 * 1024]).unwrap();
    let mut scalar = Matrix::scalar(2.);
    scalar.inherit_coordinate_slice(&backing, 0);
    assert!(scalar.has_coordinate_proof());
    let shared = Arc::new(Array::Numeric(scalar.clone()));
    let source =
        CellArray::from_shared(1, 2, vec![shared.clone(), shared.clone()], &cancel).unwrap();
    let old_target = CellArray::from_shared(
        1,
        2,
        vec![Arc::new(Array::Numeric(Matrix::scalar(0.))), shared],
        &cancel,
    )
    .unwrap();
    let mut initial = (0..14)
        .map(|i| (format!("alias{i:02}"), Array::Cell(source.clone())))
        .collect::<BTreeMap<_, _>>();
    initial.insert("Source".into(), Array::Cell(source.clone()));
    initial.insert("Target".into(), Array::Cell(old_target.clone()));
    let initial_bytes: usize = initial.values().map(Array::retained_bytes).sum();
    assert!(initial_bytes < 256 * 1024 * 1024);
    let proposed = old_target
        .replaced(0, Array::Cell(source.clone()), &cancel)
        .unwrap();
    let projected = initial_bytes - old_target.retained_bytes() + proposed.retained_bytes();
    assert!(
        projected > 256 * 1024 * 1024,
        "The actual update must enter admission fallback"
    );
    assert!(initial.values().map(Array::bytes).sum::<usize>() < 16 * 1024);

    let mut engine = Engine::default();
    engine.merge_workspace(initial.clone()).unwrap();
    for (name, old) in &initial {
        assert!(engine.array(name).unwrap().same_snapshot(old));
    }
    let target = CellTarget::new(
        Binding::new("Target".into(), engine.array("Target").unwrap()).unwrap(),
        vec![0],
    )
    .unwrap();
    let selected = Binding::new("Source".into(), engine.array("Source").unwrap()).unwrap();
    let before_bytes = engine.workspace_bytes();

    // Existing hook is inside actual FactStripper checkpoints, not before the
    // transaction. It fires after real candidate normalization has begun.
    crate::array::coordinate_retention::stop_after_checkpoints(10);
    let error = replace(&mut engine, &target, &selected).unwrap_err();
    assert!(error.contains("cancelled"));
    assert!(engine.cancellation().load(Ordering::Relaxed));
    assert_eq!(engine.workspace_bytes(), before_bytes);
    for (name, old) in &initial {
        assert!(engine.array(name).unwrap().same_snapshot(old));
    }

    engine.cancellation().store(false, Ordering::Relaxed);
    let returned = replace(&mut engine, &target, &selected).unwrap();
    let committed = engine.array("Target").unwrap();
    assert!(returned.same_snapshot(&committed));
    let Array::Cell(actual) = &returned else {
        panic!("Target remains a cell")
    };
    assert!(
        !actual.same_storage(&proposed),
        "Return the normalized committed table, not the pre-admission candidate"
    );
    assert_eq!(returned.retained_bytes(), returned.bytes());
    let selected_after = engine.array("Source").unwrap();
    assert!(actual.get_linear(0).unwrap().same_snapshot(&selected_after));
    let Array::Cell(row) = &selected_after else {
        panic!("Source remains a cell")
    };
    assert!(!row.same_storage(&source));
    assert!(Arc::ptr_eq(
        row.get_linear(0).unwrap(),
        row.get_linear(1).unwrap()
    ));
    for i in 0..14 {
        let Array::Cell(alias) = engine.array(&format!("alias{i:02}")).unwrap() else {
            panic!("Alias remains a cell")
        };
        assert!(alias.same_storage(row));
    }
    for value in [row.get_linear(0).unwrap(), actual.get_linear(1).unwrap()] {
        let numeric = value.as_numeric().unwrap();
        assert!(!numeric.has_coordinate_proof());
        assert!(Arc::ptr_eq(&numeric.storage, &scalar.storage));
        assert_eq!(numeric.get(0, 0).to_bits(), 2_f64.to_bits());
    }
    assert!(
        source
            .get_linear(0)
            .unwrap()
            .as_numeric()
            .unwrap()
            .has_coordinate_proof()
    );
    assert!(
        old_target
            .get_linear(1)
            .unwrap()
            .as_numeric()
            .unwrap()
            .has_coordinate_proof()
    );
    assert_eq!(
        old_target
            .get_linear(0)
            .unwrap()
            .as_numeric()
            .unwrap()
            .get(0, 0)
            .to_bits(),
        0_f64.to_bits()
    );
}
