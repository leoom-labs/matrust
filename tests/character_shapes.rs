use matrust::{Array, CellArray, CharacterRow, Engine, files};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
fn character(rows: usize, cols: usize, units: Vec<u16>) -> CharacterRow {
    CharacterRow::from_column_major(rows, cols, units, &AtomicBool::new(false)).unwrap()
}
fn get(e: &Engine, name: &str) -> CharacterRow {
    let Array::Character(c) = e.array(name).unwrap() else {
        panic!("Character lost")
    };
    c
}
fn values() -> BTreeMap<String, Array> {
    let mut values = BTreeMap::new();
    for (name, r, c) in [
        ("Empty00", 0, 0),
        ("Empty10", 1, 0),
        ("Empty03", 0, 3),
        ("Empty40", 4, 0),
    ] {
        values.insert(name.into(), character(r, c, vec![]).into());
    }
    values.insert(
        "Matrix".into(),
        character(2, 3, vec![65, 0, 955, 0xd800, 0xdc00, 0xffff]).into(),
    );
    values
}
#[test]
fn character_shapes_aliases_text_boundary_and_pre_cancel() {
    let stop = AtomicBool::new(false);
    let original = character(2, 3, vec![65, 0, 955, 0xd800, 0xdc00, 0xffff]);
    let view = original.reshape(3, 2, &stop).unwrap();
    assert!(original.same_storage(&view));
    assert!(!Array::from(original.clone()).same_snapshot(&view.clone().into()));
    assert_eq!(view.units(), original.units());
    assert!(
        original
            .to_utf8(&stop)
            .unwrap_err()
            .contains("not flattened")
    );
    let ascii = character(2, 2, vec![65, 66, 67, 68]);
    assert!(ascii.to_utf8(&stop).is_err());
    assert_eq!(
        ascii.reshape(1, 4, &stop).unwrap().to_utf8(&stop).unwrap(),
        "ABCD"
    );
    for (r, c) in [(0, 0), (1, 0), (0, 3), (4, 0)] {
        let empty = character(r, c, vec![]);
        assert_eq!((empty.rows(), empty.cols()), (r, c));
        assert_eq!(empty.to_utf8(&stop).is_ok(), r == 1 || (r, c) == (0, 0));
    }
    assert!(CharacterRow::from_column_major(65537, 0, vec![], &stop).is_err());
    assert!(CharacterRow::from_column_major(2, 3, vec![65; 5], &stop).is_err());
    stop.store(true, Ordering::Relaxed);
    assert!(original.reshape(1, 6, &stop).is_err());
    assert!(original.replaced(1, 90, &stop).is_err());
    assert_eq!(original.units()[1], 0);
    assert!(original.same_storage(&view));
}
#[test]
fn actual_engine_matrix_index_single_edit_reshape_and_empty_shapes() {
    let mut e = Engine::default();
    e.execute("C=char([65 67 69;66 68 70]);Alias=C;Flat=reshape(C,1,[]);Column=C(:);Pick=C([2 1],[3 1]);C(2,3)=char(55296);E10=char(zeros(1,0));E03=char(zeros(0,3));E40=reshape(E03,4,0);T=reshape('abcd',2,2);").unwrap();
    assert_eq!(get(&e, "Alias").units(), &[65, 66, 67, 68, 69, 70]);
    assert_eq!((get(&e, "Column").rows(), get(&e, "Column").cols()), (6, 1));
    assert_eq!(get(&e, "Pick").units(), &[70, 69, 66, 65]);
    assert_eq!(get(&e, "C").units(), &[65, 66, 67, 68, 69, 55296]);
    assert_eq!((get(&e, "C").rows(), get(&e, "C").cols()), (2, 3));
    assert!(get(&e, "Alias").same_storage(&get(&e, "Flat")));
    for (name, r, c) in [("E10", 1, 0), ("E03", 0, 3), ("E40", 4, 0), ("T", 2, 2)] {
        let a = get(&e, name);
        assert_eq!((a.rows(), a.cols()), (r, c));
    }
    let before = e.workspace();
    for source in [
        "load(T);",
        "save(T);",
        "C(2,3)='two';",
        "C=[C Flat];",
        "C=[C;Flat];",
        "C=reshape(C,7,1);",
    ] {
        assert!(e.execute(source).is_err(), "{source}");
        for (name, value) in &before {
            assert!(
                value.same_snapshot(&e.array(name).unwrap()),
                "{source} changed {name}"
            );
        }
    }
    // Char matrices of matching sizes concatenate (MATLAB), keeping every unit.
    e.execute("Wide=[C C];Tall=[Flat;Flat];").unwrap();
    let wide = get(&e, "Wide");
    assert_eq!((wide.rows(), wide.cols()), (2, 6));
    assert_eq!(
        wide.units(),
        &[65, 66, 67, 68, 69, 55296, 65, 66, 67, 68, 69, 55296]
    );
    let tall = get(&e, "Tall");
    assert_eq!((tall.rows(), tall.cols()), (2, 6));
    assert_eq!(
        tall.units(),
        &[65, 65, 66, 66, 67, 67, 68, 68, 69, 69, 70, 70]
    );
}
#[test]
fn v5_and_v73_preserve_matrix_utf16_and_every_empty_shape() {
    let stop = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    let mut source = values();
    let slots = source.values().cloned().map(Arc::new).collect::<Vec<_>>();
    source.insert(
        "Cell".into(),
        CellArray::from_shared(1, slots.len(), slots, &stop)
            .unwrap()
            .into(),
    );
    for format in ["v5", "v73"] {
        let path = dir.path().join(format!("{format}.mat"));
        if format == "v5" {
            files::save_array_workspace(&path, &source, &stop).unwrap();
        } else {
            files::save_array_workspace_v73(&path, &source, &stop).unwrap();
        }
        let catalog = files::inspect_workspace(&path, &stop).unwrap();
        assert!(catalog.variables().all(|x| x.unavailable.is_none()));
        let loaded = files::load_selected_arrays(
            &catalog,
            &source.keys().cloned().collect::<BTreeSet<_>>(),
            &stop,
        )
        .unwrap();
        for (name, a) in &source {
            equal(a, &loaded[name]);
        }
    }
}
fn equal(a: &Array, b: &Array) {
    assert_eq!(
        (a.class(), a.rows(), a.cols()),
        (b.class(), b.rows(), b.cols())
    );
    match (a, b) {
        (Array::Character(a), Array::Character(b)) => assert_eq!(a.units(), b.units()),
        (Array::Cell(a), Array::Cell(b)) => {
            for i in 0..a.len() {
                equal(a.get_linear(i).unwrap(), b.get_linear(i).unwrap());
            }
        }
        _ => panic!("Unexpected type"),
    }
}
