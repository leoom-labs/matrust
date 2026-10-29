use matrust::{
    Array, CellArray, CharacterRow, Engine,
    document::Document,
    session::{Session, SessionStore, workspace},
    simulation::Model,
    variable_edit::{self, Rect, Target},
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

fn session(values: BTreeMap<String, Array>) -> Session {
    Session {
        layout_presets: Default::default(),
        export: Default::default(),
        workspace: Some(values),
        workspace_manifest: None,
        preview: None,
        eps_renderer: None,
        figure_manifest: None,
        figures: None,
        layout: Default::default(),
        version: 1,
        documents: vec![Document::new()],
        active: 0,
        folder: std::env::current_dir().unwrap(),
        recent: vec![],
        history: vec![],
        model: Model::default(),
        model_path: None,
        model_baseline: None,
    }
}
fn character(value: &Array) -> &CharacterRow {
    let Array::Character(value) = value else {
        panic!("Expected exact character array")
    };
    value
}
fn leaf(value: &Array, nested: bool) -> &CharacterRow {
    if nested {
        let Array::Cell(cell) = value else {
            panic!("Expected cell root")
        };
        character(cell.get_linear(0).unwrap())
    } else {
        character(value)
    }
}
fn exact_values(a: &Array, b: &Array) {
    assert_eq!(
        (a.class(), a.rows(), a.cols()),
        (b.class(), b.rows(), b.cols())
    );
    match (a, b) {
        (Array::Character(a), Array::Character(b)) => assert_eq!(a.units(), b.units()),
        (Array::Cell(a), Array::Cell(b)) => {
            assert_eq!(a.len(), b.len());
            for i in 0..a.len() {
                exact_values(a.get_linear(i).unwrap(), b.get_linear(i).unwrap());
            }
        }
        _ => panic!("Unexpected fixture class"),
    }
}

#[test]
fn edited_character_shapes_aliases_and_nested_siblings_survive_two_checkpoint_generations() {
    let cancel = AtomicBool::new(false);
    for nested in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("session.json");
        let mut store = SessionStore::new(&path);
        let original_units = [65, 0, 0xd800, 0xdc00, 0xffff, 955];
        let matrix =
            CharacterRow::from_column_major(2, 3, original_units.to_vec(), &cancel).unwrap();
        let shared = Arc::new(Array::from(matrix.clone()));
        let root: Array = if nested {
            CellArray::from_shared(1, 2, vec![shared.clone(), shared], &cancel)
                .unwrap()
                .into()
        } else {
            matrix.clone().into()
        };
        let mut engine = Engine::default();
        engine
            .merge_workspace(BTreeMap::from([
                ("Edited".into(), root.clone()),
                ("RootAlias".into(), root.clone()),
                ("Original".into(), matrix.clone().into()),
                (
                    "OriginalRow".into(),
                    matrix.reshape(1, 6, &cancel).unwrap().into(),
                ),
            ]))
            .unwrap();
        let cell_path = if nested { vec![0] } else { vec![] };
        let stale_before_edit =
            Target::new("Edited".into(), root.clone(), cell_path.clone()).unwrap();
        let edited = variable_edit::paste_tsv(
            &mut engine,
            &stale_before_edit,
            Rect::new(0, 1, 2, 2).unwrap(),
            "\\u0000\t\\uDC00\r\n\\uD800\t\\uFFFF\r\n",
        )
        .unwrap()
        .root;
        let expected_units = [65, 0, 0, 0xd800, 0xdc00, 0xffff];
        assert_eq!(leaf(&edited, nested).units(), &expected_units);
        assert_eq!(
            (leaf(&edited, nested).rows(), leaf(&edited, nested).cols()),
            (2, 3)
        );
        assert!(!leaf(&edited, nested).same_storage(&matrix));
        assert!(engine.array("RootAlias").unwrap().same_snapshot(&root));
        engine
            .merge_workspace(BTreeMap::from([
                ("EditedAlias".into(), edited.clone()),
                (
                    "EditedRow".into(),
                    leaf(&edited, nested).reshape(1, 6, &cancel).unwrap().into(),
                ),
                (
                    "EditedView".into(),
                    leaf(&edited, nested).reshape(3, 2, &cancel).unwrap().into(),
                ),
            ]))
            .unwrap();
        let before = engine.workspace();
        let old_process_target =
            Target::new("Edited".into(), edited.clone(), cell_path.clone()).unwrap();
        store.save(session(before.clone())).unwrap();
        let committed = std::fs::read(&path).unwrap();
        let manifest = Session::load(&path).unwrap().workspace_manifest.unwrap();
        assert_eq!(manifest.version, 5);
        let restored = workspace::restore(&path, &manifest, &cancel).unwrap();
        assert_eq!(restored.len(), before.len());
        for (name, value) in &before {
            exact_values(value, &restored[name]);
        }
        let saved = leaf(&restored["Edited"], nested);
        let old = character(&restored["Original"]);
        assert!(restored["Edited"].same_snapshot(&restored["EditedAlias"]));
        assert!(saved.same_storage(character(&restored["EditedRow"])));
        assert!(saved.same_storage(character(&restored["EditedView"])));
        assert!(old.same_storage(character(&restored["OriginalRow"])));
        assert!(!saved.same_storage(old));
        assert_eq!(old.units(), &original_units);
        assert!(leaf(&restored["RootAlias"], nested).same_storage(old));
        if nested {
            let Array::Cell(cell) = &restored["Edited"] else {
                panic!()
            };
            assert!(character(cell.get_linear(1).unwrap()).same_storage(old));
            let Array::Cell(alias) = &restored["RootAlias"] else {
                panic!()
            };
            assert!(
                character(alias.get_linear(0).unwrap())
                    .same_storage(character(alias.get_linear(1).unwrap()))
            );
        }
        let mut reopened = Engine::default();
        reopened.merge_workspace(restored.clone()).unwrap();
        // Reader-created payload identities cannot be edited with an expectation
        // retained from the old process, even when the values match exactly.
        assert!(
            variable_edit::apply_literal(&mut reopened, &old_process_target, 0, 0, "\\uFEFF")
                .is_err()
        );
        for (name, value) in &restored {
            assert!(value.same_snapshot(&reopened.array(name).unwrap()));
        }
        let target = Target::new(
            "Edited".into(),
            reopened.array("Edited").unwrap(),
            cell_path,
        )
        .unwrap();
        reopened.cancellation().store(true, Ordering::Relaxed);
        assert!(variable_edit::apply_literal(&mut reopened, &target, 0, 0, "\\uFEFF").is_err());
        reopened.cancellation().store(false, Ordering::Relaxed);
        assert_eq!(std::fs::read(&path).unwrap(), committed);
        variable_edit::apply_literal(&mut reopened, &target, 0, 0, "\\uFEFF").unwrap();
        assert_eq!(
            leaf(&reopened.array("EditedAlias").unwrap(), nested).units(),
            &expected_units
        );
        let second_source = reopened.workspace();
        cancel.store(true, Ordering::Relaxed);
        assert!(
            store
                .save_with_cancel(session(second_source.clone()), &cancel)
                .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), committed);
        cancel.store(false, Ordering::Relaxed);
        store.save(session(second_source.clone())).unwrap();
        let second_manifest = Session::load(&path).unwrap().workspace_manifest.unwrap();
        assert_eq!(second_manifest.version, 5);
        let second = workspace::restore(&path, &second_manifest, &cancel).unwrap();
        for (name, value) in &second_source {
            exact_values(value, &second[name]);
        }
        let new_leaf = leaf(&second["Edited"], nested);
        assert_eq!(new_leaf.units(), &[0xfeff, 0, 0, 0xd800, 0xdc00, 0xffff]);
        assert!(!new_leaf.same_storage(leaf(&second["EditedAlias"], nested)));
        assert!(leaf(&second["EditedAlias"], nested).same_storage(character(&second["EditedRow"])));
        assert!(character(&second["EditedRow"]).same_storage(character(&second["EditedView"])));
        assert_eq!(character(&second["Original"]).units(), &original_units);
    }
}
