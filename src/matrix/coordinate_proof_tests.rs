use super::*;
use crate::plot::coordinates::VerifiedPatchInputs;
use crate::{Array, CellArray, Engine};
use std::sync::atomic::Ordering;

fn bits(a: &[f64], b: &[f64]) {
    assert_eq!(a.len(), b.len());
    for (a, b) in a.iter().zip(b) {
        assert_eq!(a.to_bits(), b.to_bits());
    }
}

#[test]
fn ordinary_script_column_arithmetic_concat_transpose_reshape_build_real_corners() {
    let stop = AtomicBool::new(false);
    let x = [-0.0, 1.0e16, f64::from_bits(1), 3.0];
    let y = [-0.0, 4.0, -2.0, 6.0];
    let source = Matrix::from_column_vec(4, 2, x.into_iter().chain(y).collect()).unwrap();
    let mut engine = Engine::default();
    engine.variables.insert("samples".into(), source.clone());
    engine.execute("x=samples(:,1); y=samples(:,2); left=x+0.25-0.4; right=x+0.25+0.4; bottom=y*0; X=[left right right left].'; Y=[bottom bottom y y].'; X=reshape(X,4,[]); alias=X;").unwrap();
    let a = engine.variables["X"].clone();
    let b = engine.variables["Y"].clone();
    assert!(a.has_coordinate_proof() && b.has_coordinate_proof());
    assert!(Arc::ptr_eq(&a.storage, &engine.variables["alias"].storage));
    let proof = a.rectangle_coordinates(&b, &stop).unwrap().unwrap();
    let expanded = proof.materialize(&stop).unwrap();
    for face in 0..4 {
        let left = (x[face] + 0.25) - 0.4;
        let right = (x[face] + 0.25) + 0.4;
        let expected = [
            [left, y[face] * 0.0],
            [right, y[face] * 0.0],
            [right, y[face]],
            [left, y[face]],
        ];
        for corner in 0..4 {
            bits(&expanded[face * 4 + corner], &expected[corner]);
            bits(
                &[a.get(corner, face), b.get(corner, face)],
                &expected[corner],
            );
        }
    }
    assert!(Arc::ptr_eq(
        &source.storage,
        &engine.variables["samples"].storage
    ));
    VerifiedPatchInputs::new(a.clone(), b.clone(), proof, 256 * 1024 * 1024, &stop).unwrap();
    // Same public operations, constant baseline, no exporter-source recognition.
    engine
        .execute("base=-2*ones(size(y)); C=[base base y y].';")
        .unwrap();
    let constant = a.clone();
    let proof = constant
        .rectangle_coordinates(&engine.variables["C"], &stop)
        .unwrap()
        .unwrap();
    assert_eq!(
        proof.corner(0, 0).unwrap()[1].to_bits(),
        (-2.0f64).to_bits()
    );
}

#[test]
fn ordered_scalar_sides_and_dense_fallback_keep_original_bits() {
    let source =
        Matrix::from_column_slice(4, 1, &[-0.0, f64::from_bits(1), 1.0e16, f64::INFINITY]).unwrap();
    let right = source
        .coordinate_add(&Matrix::scalar(-1.0e16))
        .unwrap()
        .coordinate_subtract(&Matrix::scalar(1.0))
        .unwrap();
    for (i, value) in source.data().iter().enumerate() {
        assert_eq!(
            right.data()[i].to_bits(),
            ((value + -1.0e16) - 1.0).to_bits()
        );
    }
    assert!(right.has_coordinate_proof());
    let third = right.coordinate_add(&Matrix::scalar(3.0)).unwrap();
    assert!(!third.has_coordinate_proof());
    let lhs = Matrix::scalar(-0.0).coordinate_subtract(&source).unwrap();
    bits(
        lhs.data(),
        &source.data().iter().map(|v| -0.0 - v).collect::<Vec<_>>(),
    );
    let product = Matrix::scalar(-1.0).mul(&source).unwrap();
    bits(
        product.data(),
        &source.data().iter().map(|v| -1.0 * v).collect::<Vec<_>>(),
    );
    assert!(product.has_coordinate_proof());
    assert!(!right.map(f64::sin).has_coordinate_proof());
    assert!(
        !right
            .zip(&source, |a, b| a / b)
            .unwrap()
            .has_coordinate_proof()
    );
    assert!(!right.with_value(0, 0, 7.0).unwrap().has_coordinate_proof());
    let four = (0..4)
        .map(|i| Matrix::from_column_vec(4, 1, vec![i as f64; 4]).unwrap())
        .collect();
    assert!(
        !Matrix::concatenate(vec![four])
            .unwrap()
            .has_coordinate_proof()
    );
}

#[test]
fn hidden_capacity_is_charged_without_retaining_intermediate_matrices() {
    let mut values = Vec::with_capacity(1024);
    values.extend([1.0, 2.0, 3.0, 4.0]);
    let source = Matrix::from_column_vec(4, 1, values).unwrap();
    let weak_source = Arc::downgrade(&source.storage);
    let first = source.coordinate_add(&Matrix::scalar(1.0)).unwrap();
    let weak_first = Arc::downgrade(&first.storage);
    let second = first.coordinate_multiply(&Matrix::scalar(2.0)).unwrap();
    drop(first);
    assert!(weak_first.upgrade().is_none());
    assert!(second.retained_bytes() >= 1024 * 8 + 4 * 8);
    drop(source);
    assert!(weak_source.upgrade().is_some());
    let pointer = second.data().as_ptr();
    let eager = second.without_coordinate_proof();
    assert!(weak_source.upgrade().is_none());
    assert_eq!(eager.data().as_ptr(), pointer);
    assert_eq!(eager.retained_bytes(), 4 * 8);
}

#[test]
fn workspace_alias_cells_and_replacement_charge_hidden_capacity() {
    let stop = AtomicBool::new(false);
    let source = Matrix::from_column_vec(1024 * 1024, 1, vec![2.0; 1024 * 1024]).unwrap();
    let mut result = Matrix::from_column_slice(1, 1, &[2.0]).unwrap();
    result.inherit_coordinate_slice(&source, 0);
    let bytes = result.retained_bytes();
    assert!(bytes > 8 * 1024 * 1024);
    let mut engine = Engine::default();
    let mut updates = std::collections::BTreeMap::new();
    for i in 0..31 {
        updates.insert(format!("v{i}"), Array::Numeric(result.clone()));
    }
    let mut functions = Engine::default();
    functions.merge_workspace(updates.clone()).unwrap();
    engine.merge_workspace(updates).unwrap();
    assert_eq!(engine.workspace_bytes(), bytes * 31);
    let before = engine.variables["v0"].clone();
    engine
        .merge_workspace([(String::from("extra"), Array::Numeric(result.clone()))].into())
        .unwrap();
    assert_eq!(engine.workspace_bytes(), 32 * 8);
    assert!(engine.variables.values().all(|m| !m.has_coordinate_proof()));
    assert!(Arc::ptr_eq(
        &before.storage,
        &engine.variables["v0"].storage
    ));
    functions
        .execute("z=retain(v0); function out=retain(a); out=a; end")
        .unwrap();
    assert!(Arc::ptr_eq(
        &before.storage,
        &functions.variables["z"].storage
    ));
    assert_eq!(functions.workspace_bytes(), 32 * 8);
    let excessive = (0..33)
        .map(|i| (format!("large{i}"), Array::Numeric(source.clone())))
        .collect();
    assert!(engine.merge_workspace(excessive).is_err());
    assert_eq!(engine.workspace_bytes(), 32 * 8);
    assert!(Arc::ptr_eq(
        &before.storage,
        &engine.variables["v0"].storage
    ));
    engine.execute("v0=1;").unwrap();
    assert_eq!(engine.workspace_bytes(), 32 * 8);
    let child = Arc::new(Array::Numeric(result));
    let cell = CellArray::from_shared(1, 2, vec![child.clone(), child], &stop).unwrap();
    assert!(cell.retained_bytes() >= bytes * 2);
    assert!(cell.bytes() < cell.retained_bytes());
    let nested =
        CellArray::from_shared(1, 1, vec![Arc::new(Array::Cell(cell.clone()))], &stop).unwrap();
    assert!(nested.retained_bytes() >= cell.retained_bytes());
}

#[test]
fn cell_retention_cache_reshape_replace_and_cancel_are_atomic() {
    let stop = AtomicBool::new(false);
    let source = Matrix::from_column_vec(1024 * 1024, 1, vec![2.0; 1024 * 1024]).unwrap();
    let mut result = Matrix::scalar(2.0);
    result.inherit_coordinate_slice(&source, 0);
    let child = Arc::new(Array::Numeric(result));
    let repeated = child.clone();
    let mut children = vec![repeated; 31];
    children.push(Arc::new(Array::Numeric(Matrix::scalar(0.0))));
    let cell = CellArray::from_shared(1, 32, children, &stop).unwrap();
    let reshaped = cell.reshape(4, 8, &stop).unwrap();
    assert!(cell.same_storage(&reshaped));
    assert_eq!(cell.retained_bytes(), reshaped.retained_bytes());
    let fallback = cell.replaced(31, (*child).clone(), &stop).unwrap();
    assert_eq!(fallback.retained_bytes(), fallback.bytes());
    for i in 0..32 {
        let value = fallback.get_linear(i).unwrap().as_numeric().unwrap();
        assert_eq!(value.get(0, 0), 2.0);
        assert!(Arc::ptr_eq(
            &value.storage,
            &child.as_numeric().unwrap().storage
        ));
    }
    let dense = Arc::new(Array::Numeric(source.clone()));
    assert!(CellArray::from_shared(1, 32, vec![dense; 32], &stop).is_err());
    assert!(cell.same_storage(&reshaped));
    assert_eq!(
        cell.get_linear(31).unwrap().as_numeric().unwrap().get(0, 0),
        0.0
    );
    stop.store(true, Ordering::Relaxed);
    assert!(
        cell.replaced(0, Array::Numeric(Matrix::scalar(0.0)), &stop)
            .is_err()
    );
    stop.store(false, Ordering::Relaxed);
    let smaller = cell
        .replaced(0, Array::Numeric(Matrix::scalar(0.0)), &stop)
        .unwrap();
    assert!(smaller.retained_bytes() < cell.retained_bytes());
    assert!(Arc::ptr_eq(
        smaller.get_linear(1).unwrap(),
        cell.get_linear(1).unwrap()
    ));
}

#[test]
fn mat_and_recovery_keep_logical_payload_contract_and_drop_optional_facts() {
    use crate::{
        document::Document,
        files,
        session::{Session, workspace},
        simulation::Model,
    };
    let stop = AtomicBool::new(false);
    let directory = tempfile::tempdir().unwrap();
    let source = Matrix::from_column_slice(4, 1, &[-0.0, 2.0, 3.0, 4.0]).unwrap();
    let result = source.coordinate_multiply(&Matrix::scalar(2.0)).unwrap();
    let numeric = Arc::new(Array::Numeric(result.clone()));
    let cell = CellArray::from_shared(1, 2, vec![numeric.clone(), numeric], &stop).unwrap();
    let values = [
        (String::from("data"), Array::Numeric(result.clone())),
        (String::from("nested"), Array::Cell(cell)),
    ]
    .into_iter()
    .collect::<std::collections::BTreeMap<_, _>>();
    assert!(values["data"].retained_bytes() > values["data"].bytes());
    let mat = directory.path().join("values.mat");
    files::save_array_workspace(&mat, &values, &stop).unwrap();
    let catalog = files::inspect_workspace(&mat, &stop).unwrap();
    let loaded =
        files::load_selected_arrays(&catalog, &values.keys().cloned().collect(), &stop).unwrap();
    let checkpoint = directory.path().join("session.json");
    Session {
        layout_presets: Default::default(),
        export: Default::default(),
        workspace: Some(values.clone()),
        workspace_manifest: None,
        preview: None,
        eps_renderer: None,
        figure_manifest: None,
        figures: None,
        layout: Default::default(),
        version: 1,
        documents: vec![Document::new()],
        active: 0,
        folder: directory.path().to_path_buf(),
        recent: vec![],
        history: vec![],
        model: Model::default(),
        model_path: None,
        model_baseline: None,
    }
    .save(&checkpoint)
    .unwrap();
    let restored = workspace::restore(
        &checkpoint,
        &Session::load(&checkpoint)
            .unwrap()
            .workspace_manifest
            .unwrap(),
        &stop,
    )
    .unwrap();
    for actual in [&loaded, &restored] {
        let matrix = actual["data"].as_numeric().unwrap();
        bits(matrix.data(), result.data());
        assert!(!matrix.has_coordinate_proof());
        let Array::Cell(cell) = &actual["nested"] else {
            panic!("cell type lost")
        };
        for i in 0..2 {
            bits(
                cell.get_linear(i).unwrap().as_numeric().unwrap().data(),
                result.data(),
            );
        }
        assert_eq!(actual["nested"].bytes(), values["nested"].bytes());
        assert_eq!(actual["nested"].retained_bytes(), actual["nested"].bytes());
    }
    assert!(Arc::ptr_eq(
        &result.storage,
        &values["data"].as_numeric().unwrap().storage
    ));
}

#[test]
fn unique_mutation_drops_facts_only_on_success_and_cancel_restores_them() {
    let source = Matrix::from_column_slice(4, 1, &[1., 2., 3., 4.]).unwrap();
    let mut result = source.coordinate_add(&Matrix::scalar(2.)).unwrap();
    let original = result.data().to_vec();
    let mut checks = 0;
    let error = result
        .write_unique_regions(std::slice::from_ref(&(0..4)), &[9.0], || {
            checks += 1;
            if checks == 3 {
                Err("test stop".into())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    assert_eq!(error, "test stop");
    assert_eq!(checks, 3);
    bits(result.data(), &original);
    assert!(result.has_coordinate_proof());
    assert!(result.write_unique(&[(1, 11.0)]));
    assert!(!result.has_coordinate_proof());
    assert_eq!(result.get(1, 0), 11.0);
    bits(source.data(), &[1., 2., 3., 4.]);
}

#[test]
fn derived_nan_geometry_accepts_payload_variance_but_source_nan_and_zero_are_exact() {
    let stop = AtomicBool::new(false);
    let source = Matrix::from_column_slice(1, 1, &[f64::from_bits(0x7ff8_0000_0000_0042)]).unwrap();
    let lane = CoordinateLane::matrix_column(&source, 0).unwrap();
    let derived = lane.clone().then(ScalarStep::Multiply(0.0)).unwrap();
    let proof = PatchCoordinates::rectangles(
        [
            [Coordinate::Constant(0.), Coordinate::Lane(derived.clone())],
            [Coordinate::Constant(1.), Coordinate::Lane(derived)],
            [Coordinate::Constant(1.), Coordinate::Lane(lane.clone())],
            [Coordinate::Constant(0.), Coordinate::Lane(lane)],
        ],
        1,
        &stop,
    )
    .unwrap();
    let x = Matrix::from_column_slice(4, 1, &[0., 1., 1., 0.]).unwrap();
    let different = f64::from_bits(0xfff8_0000_0000_0099);
    let y = Matrix::from_column_slice(
        4,
        1,
        &[different, different, source.data()[0], source.data()[0]],
    )
    .unwrap();
    VerifiedPatchInputs::new(x.clone(), y.clone(), proof.clone(), 1_000_000, &stop).unwrap();
    assert!(
        VerifiedPatchInputs::new(
            x.clone(),
            y.with_value(2, 0, different).unwrap(),
            proof.clone(),
            1_000_000,
            &stop
        )
        .is_err()
    );
    assert!(
        VerifiedPatchInputs::new(
            x.with_value(0, 0, -0.0).unwrap(),
            y,
            proof,
            1_000_000,
            &stop
        )
        .is_err()
    );
    stop.store(true, Ordering::Relaxed);
    assert!(x.rectangle_coordinates(&x, &stop).is_err());
    stop.store(false, Ordering::Relaxed);
    assert!(x.rectangle_coordinates(&x, &stop).unwrap().is_none());
}

#[test]
#[cfg(target_pointer_width = "64")]
fn original_cell_logical_slot_layout_is_preserved_and_proof_allocation_is_charged() {
    // Original x64: Matrix=24, CellArray=48, Array=56, logical SLOT=64.
    // Matrix now uses the existing largest variant's 48-byte envelope; CellArray
    // retains 48 bytes by packing checked <=65,536 nodes / <=256MiB retention.
    assert_eq!(std::mem::size_of::<Matrix>(), 48);
    assert_eq!(std::mem::size_of::<CellArray>(), 48);
    assert_eq!(std::mem::size_of::<Array>(), 56);
    assert_eq!(crate::containers::SLOT, 64);
    let source = Matrix::from_column_slice(2, 1, &[1.0, 2.0]).unwrap();
    let output = source.coordinate_add(&Matrix::scalar(1.0)).unwrap();
    assert_eq!(
        output.retained_bytes(),
        32 + std::mem::size_of::<CoordinateProof>() + 2 * std::mem::size_of::<usize>()
    );
}

#[test]
fn stripping_preserves_cross_binding_nested_and_reshaped_cell_aliases_after_stop_retry() {
    let stop = AtomicBool::new(false);
    let source = Matrix::from_column_vec(1024 * 1024, 1, vec![2.0; 1024 * 1024]).unwrap();
    let mut scalar = Matrix::scalar(2.0);
    scalar.inherit_coordinate_slice(&source, 0);
    let leaf = Arc::new(Array::Numeric(scalar.clone()));
    let row = CellArray::from_shared(1, 2, vec![leaf.clone(), leaf], &stop).unwrap();
    let col = row.reshape(2, 1, &stop).unwrap();
    // Distinct Array wrappers around one table, including a different shape.
    let nested = CellArray::from_shared(
        1,
        2,
        vec![
            Arc::new(Array::Cell(row.clone())),
            Arc::new(Array::Cell(col.clone())),
        ],
        &stop,
    )
    .unwrap();
    let mut engine = Engine::default();
    engine
        .merge_workspace(
            (0..15)
                .map(|i| {
                    (
                        format!("c{i:02}"),
                        Array::Cell(if i % 2 == 0 { row.clone() } else { col.clone() }),
                    )
                })
                .collect(),
        )
        .unwrap();
    let before_bytes = engine.workspace_bytes();
    let update = [(String::from("nested"), Array::Cell(nested.clone()))].into();
    crate::array::coordinate_retention::stop_after_checkpoints(10);
    assert_eq!(
        engine.merge_workspace(update).unwrap_err(),
        "Calculation cancelled"
    );
    assert!(engine.cancellation().load(Ordering::Relaxed));
    assert_eq!(engine.workspace_bytes(), before_bytes);
    assert!(engine.array("nested").is_none());
    for i in 0..15 {
        let Array::Cell(value) = engine.array(&format!("c{i:02}")).unwrap() else {
            panic!("cell");
        };
        assert!(value.same_storage(&row));
        assert_eq!(
            (value.rows(), value.cols()),
            if i % 2 == 0 { (1, 2) } else { (2, 1) }
        );
        assert!(
            value
                .get_linear(0)
                .unwrap()
                .as_numeric()
                .unwrap()
                .has_coordinate_proof()
        );
    }
    engine.cancellation().store(false, Ordering::Relaxed);
    engine
        .merge_workspace([(String::from("nested"), Array::Cell(nested))].into())
        .unwrap();
    let Array::Cell(converted) = engine.array("c00").unwrap() else {
        panic!("cell");
    };
    assert!(!converted.same_storage(&row));
    for i in 0..15 {
        let Array::Cell(value) = engine.array(&format!("c{i:02}")).unwrap() else {
            panic!("cell");
        };
        assert!(value.same_storage(&converted));
        assert_eq!(
            (value.rows(), value.cols()),
            if i % 2 == 0 { (1, 2) } else { (2, 1) }
        );
    }
    let Array::Cell(nested) = engine.array("nested").unwrap() else {
        panic!("cell");
    };
    for i in 0..2 {
        let Array::Cell(value) = nested.get_linear(i).unwrap().as_ref() else {
            panic!("cell");
        };
        assert!(value.same_storage(&converted));
        assert_eq!(
            (value.rows(), value.cols()),
            if i == 0 { (1, 2) } else { (2, 1) }
        );
    }
    let a = converted.get_linear(0).unwrap();
    assert!(Arc::ptr_eq(a, converted.get_linear(1).unwrap()));
    let numeric = a.as_numeric().unwrap();
    assert!(!numeric.has_coordinate_proof());
    assert!(Arc::ptr_eq(&numeric.storage, &scalar.storage));
    assert!(row.same_storage(&col));
    assert!(
        row.get_linear(0)
            .unwrap()
            .as_numeric()
            .unwrap()
            .has_coordinate_proof()
    );

    // One context must span parked bindings and inputs, not just each phase.
    let mut functions = Engine::default();
    functions
        .merge_workspace(
            [
                (String::from("a"), Array::Cell(row.clone())),
                (String::from("b"), Array::Cell(col)),
            ]
            .into(),
        )
        .unwrap();
    functions
        .execute("out=keep(b); function result=keep(arg); result=arg; end")
        .unwrap();
    let Array::Cell(a) = functions.array("a").unwrap() else {
        panic!("cell");
    };
    for name in ["b", "out"] {
        let Array::Cell(value) = functions.array(name).unwrap() else {
            panic!("cell");
        };
        assert!(a.same_storage(&value));
        assert_eq!((value.rows(), value.cols()), (2, 1));
    }
    assert_eq!((a.rows(), a.cols()), (1, 2));
    assert!(Arc::ptr_eq(
        &a.get_linear(0).unwrap().as_numeric().unwrap().storage,
        &scalar.storage
    ));
}
