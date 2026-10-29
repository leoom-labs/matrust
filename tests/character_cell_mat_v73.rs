use hdf5::{ObjectReference1, types::FixedAscii};
use matrust::{
    Array, CellArray, CharacterRow, Complex64, ComplexMatrix, Engine, LogicalMatrix, Matrix, files,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
fn names(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|s| (*s).into()).collect()
}
fn chars(units: &[u16]) -> Array {
    CharacterRow::from_units(units.to_vec(), &AtomicBool::new(false))
        .unwrap()
        .into()
}
fn cell(rows: usize, cols: usize, items: Vec<Array>) -> Array {
    CellArray::from_shared(
        rows,
        cols,
        items.into_iter().map(Arc::new).collect(),
        &AtomicBool::new(false),
    )
    .unwrap()
    .into()
}
fn same(a: &Array, b: &Array) {
    assert_eq!(
        (a.class(), a.rows(), a.cols(), a.is_complex()),
        (b.class(), b.rows(), b.cols(), b.is_complex())
    );
    match (a, b) {
        (Array::Numeric(a), Array::Numeric(b)) => assert_eq!(
            a.data().iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
            b.data().iter().map(|x| x.to_bits()).collect::<Vec<_>>()
        ),
        (Array::Logical(a), Array::Logical(b)) => assert_eq!(a.data(), b.data()),
        (Array::Complex(a), Array::Complex(b)) => {
            for (a, b) in a.data().iter().zip(b.data()) {
                assert_eq!(
                    (a.re.to_bits(), a.im.to_bits()),
                    (b.re.to_bits(), b.im.to_bits())
                );
            }
        }
        (Array::Character(a), Array::Character(b)) => assert_eq!(a.units(), b.units()),
        (Array::Cell(a), Array::Cell(b)) => {
            for i in 0..a.len() {
                same(a.get_linear(i).unwrap(), b.get_linear(i).unwrap());
            }
        }
        _ => panic!("Typed value differs"),
    }
}
fn expected() -> BTreeMap<String, Array> {
    let stop = AtomicBool::new(false);
    let real: Array = Matrix::from_column_vec(
        2,
        3,
        vec![
            -0.,
            1.25,
            f64::from_bits(1),
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::from_bits(0x7ff8000000000042),
        ],
    )
    .unwrap()
    .into();
    let logical: Array = LogicalMatrix::from_numeric(
        &Matrix::from_column_slice(2, 3, &[0., 1., 1., 0., 1., 0.]).unwrap(),
        &stop,
    )
    .unwrap()
    .into();
    let complex: Array = ComplexMatrix::from_column_vec(
        2,
        2,
        vec![
            Complex64::new(-0., 4.),
            Complex64::new(1., -0.),
            Complex64::new(2., 5.),
            Complex64::new(3., 6.),
        ],
        &stop,
    )
    .unwrap()
    .into();
    let empty: Array = Matrix::from_column_vec(0, 3, vec![]).unwrap().into();
    let text = chars(&[65, 0, 955, 65534, 65535, 90]);
    let nested = cell(
        1,
        3,
        vec![
            chars(&[]),
            CellArray::empty(0, 3, &stop).unwrap().into(),
            empty.clone(),
        ],
    );
    let c = cell(
        2,
        3,
        vec![
            text.clone(),
            real.clone(),
            logical.clone(),
            complex.clone(),
            nested,
            empty.clone(),
        ],
    );
    BTreeMap::from([
        ("Text".into(), text),
        ("Pair".into(), chars(&[0xd83d, 0xde00])),
        ("Unpaired".into(), chars(&[0xd800, 65, 0xdc00])),
        ("Char00".into(), chars(&[])),
        ("Real".into(), real),
        ("Logical".into(), logical),
        ("Complex".into(), complex),
        ("Complex03".into(), empty.clone()),
        ("Real03".into(), empty),
        (
            "Cell00".into(),
            CellArray::empty(0, 0, &stop).unwrap().into(),
        ),
        (
            "Cell03".into(),
            CellArray::empty(0, 3, &stop).unwrap().into(),
        ),
        (
            "Cell40".into(),
            CellArray::empty(4, 0, &stop).unwrap().into(),
        ),
        ("C".into(), c),
    ])
}
#[test]
fn authentic_reference_char_cells_preserve_every_shape_and_exact_bits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("authentic.mat");
    // Exact vendor generation4child attempt,17variables; SHA46471686f734... .
    std::fs::write(&path, include_bytes!("fixtures/character-cell-v73.mat")).unwrap();
    let stop = AtomicBool::new(false);
    let catalog = files::inspect_workspace(&path, &stop).unwrap();
    assert_eq!(catalog.variables().count(), 17);
    for (name, shape) in [
        ("CharMatrix", vec![2, 3]),
        ("Char10", vec![1, 0]),
        ("Char03", vec![0, 3]),
        ("Char40", vec![4, 0]),
    ] {
        let info = catalog.variables().find(|v| v.name == name).unwrap();
        assert_eq!(info.dimensions, shape);
        assert!(info.unavailable.is_none());
        let loaded = files::load_selected_arrays(&catalog, &names(&[name]), &stop).unwrap();
        let Array::Character(text) = &loaded[name] else {
            panic!("Character lost")
        };
        assert_eq!((text.rows(), text.cols()), (shape[0], shape[1]));
        assert_eq!(
            text.units(),
            if name == "CharMatrix" {
                &[65, 66, 67, 68, 69, 70][..]
            } else {
                &[][..]
            }
        );
    }
    let want = expected();
    let loaded =
        files::load_selected_arrays(&catalog, &want.keys().cloned().collect(), &stop).unwrap();
    for (name, value) in &want {
        same(value, &loaded[name]);
    }
    assert!(
        files::load_selected_workspace(&catalog, &names(&["C"]), &stop)
            .unwrap_err()
            .contains("typed")
    );
}
#[test]
fn typed_roundtrip_chunked_utf16_references_and_shared_children_one_eight_workers() {
    let stop = AtomicBool::new(false);
    let mut values = expected();
    let text: Array = CharacterRow::from_units((0..65536).map(|i| i as u16).collect(), &stop)
        .unwrap()
        .into();
    let leaf = Arc::new(chars(&[0, 0xd800, 65, 0xdc00]));
    let repeated: Array = CellArray::from_shared(1, 2048, vec![leaf.clone(); 2048], &stop)
        .unwrap()
        .into();
    values.insert("AllUnits".into(), text.clone());
    values.insert("Repeated".into(), repeated.clone());
    let dir = tempfile::tempdir().unwrap();
    for workers in [1, 8] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
            .install(|| {
                let path = dir.path().join(format!("roundtrip{workers}.mat"));
                files::save_array_workspace_v73(&path, &values, &stop).unwrap();
                let hdf = hdf5::File::open(&path).unwrap();
                assert!(!hdf.dataset("AllUnits").unwrap().filters().is_empty());
                assert!(!hdf.dataset("Repeated").unwrap().filters().is_empty());
                drop(hdf);
                let catalog = files::inspect_workspace(&path, &stop).unwrap();
                let actual =
                    files::load_selected_arrays(&catalog, &values.keys().cloned().collect(), &stop)
                        .unwrap();
                for (name, value) in &values {
                    same(value, &actual[name]);
                }
                let Array::Cell(c) = &actual["Repeated"] else {
                    panic!("Cell lost")
                };
                for i in 1..c.len() {
                    assert!(Arc::ptr_eq(
                        c.get_linear(0).unwrap(),
                        c.get_linear(i).unwrap()
                    ));
                }
                assert!(values["AllUnits"].same_snapshot(&text));
                assert!(values["Repeated"].same_snapshot(&repeated));
            });
    }
}
fn class(ds: &hdf5::Dataset, name: &str) {
    ds.new_attr::<FixedAscii<16>>()
        .create("MATLAB_class")
        .unwrap()
        .write_scalar(&FixedAscii::<16>::from_ascii(name).unwrap())
        .unwrap();
}
fn synthetic(path: &Path, make: impl FnOnce(&hdf5::File)) {
    {
        let file = hdf5::File::with_options()
            .with_fcpl(|p| p.userblock(512))
            .create(path)
            .unwrap();
        make(&file);
    }
    let mut head = [0; 128];
    let description = b"MATLAB 7.3 MAT-file,";
    head[..description.len()].copy_from_slice(description);
    head[124..].copy_from_slice(&[0, 2, b'I', b'M']);
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .write_all(&head)
        .unwrap();
}
fn rejected(path: &Path, name: &str, reason: &str) {
    let stop = AtomicBool::new(false);
    let catalog = files::inspect_workspace(path, &stop).unwrap();
    let info = catalog.variables().find(|v| v.name == name).unwrap();
    assert!(
        info.unavailable.as_ref().unwrap().contains(reason),
        "{:?}",
        info
    );
    assert!(files::load_selected_arrays(&catalog, &names(&[name]), &stop).is_err());
}
#[test]
fn cycles_null_group_references_wrong_dtype_decode_and_oversized_chunks_reject() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.mat");
    synthetic(&path, |f| {
        let cycle = f
            .new_dataset::<ObjectReference1>()
            .shape((1, 1))
            .create("Cycle")
            .unwrap();
        class(&cycle, "cell");
        cycle
            .write_raw(&[f.reference::<ObjectReference1>("Cycle").unwrap()])
            .unwrap();
        let null = f
            .new_dataset::<ObjectReference1>()
            .shape((1, 1))
            .create("Null")
            .unwrap();
        class(&null, "cell"); // default zero/null reference
        f.create_group("#refs#").unwrap();
        let group = f
            .new_dataset::<ObjectReference1>()
            .shape((1, 1))
            .create("Group")
            .unwrap();
        class(&group, "cell");
        group
            .write_raw(&[f.reference::<ObjectReference1>("#refs#").unwrap()])
            .unwrap();
        let wrong = f
            .new_dataset::<u64>()
            .shape((1, 1))
            .create("Wrong")
            .unwrap();
        class(&wrong, "cell");
        let text = f
            .new_dataset::<u16>()
            .shape((2, 1))
            .create("Decode")
            .unwrap();
        class(&text, "char");
        text.new_attr::<i32>()
            .create("MATLAB_int_decode")
            .unwrap()
            .write_scalar(&1)
            .unwrap();
        let huge = f
            .new_dataset::<ObjectReference1>()
            .shape((2048, 1024))
            .chunk((2048, 1024))
            .create("Chunk")
            .unwrap();
        class(&huge, "cell");
    });
    for (name, reason) in [
        ("Cycle", "Cyclic"),
        ("Null", "MAT v7.3"),
        ("Group", "dataset"),
        ("Wrong", "storage"),
        ("Decode", "decoding"),
        ("Chunk", "65536"),
    ] {
        rejected(&path, name, reason);
    }
}
#[test]
fn reference_depth_and_repeated_logical_storage_are_preflighted_before_leaf_loading() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("depth.mat");
    synthetic(&path, |f| {
        f.create_group("#refs#").unwrap();
        let leaf = f
            .new_dataset::<f64>()
            .shape((1, 1))
            .create("/#refs#/leaf")
            .unwrap();
        class(&leaf, "double");
        let mut last = "/#refs#/leaf".to_owned();
        for i in 0..17 {
            let name = if i == 16 {
                "Deep".into()
            } else {
                format!("/#refs#/c{i}")
            };
            let ds = f
                .new_dataset::<ObjectReference1>()
                .shape((1, 1))
                .create(name.as_str())
                .unwrap();
            class(&ds, "cell");
            ds.write_raw(&[f.reference::<ObjectReference1>(&last).unwrap()])
                .unwrap();
            last = name;
        }
        let big = f
            .new_dataset::<f64>()
            .shape((1, 16 * 1024 * 1024))
            .create("/#refs#/big")
            .unwrap();
        class(&big, "double");
        let repeated = f
            .new_dataset::<ObjectReference1>()
            .shape((3, 1))
            .create("Budget")
            .unwrap();
        class(&repeated, "cell");
        let r = f.reference::<ObjectReference1>("/#refs#/big").unwrap();
        repeated.write_raw(&[r, r, r]).unwrap();
    });
    rejected(&path, "Deep", "16 levels");
    rejected(&path, "Budget", "256 MiB");
}
#[test]
fn nested_graphics_rejection_and_pre_cancel_leave_destination_and_workspace_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("existing.mat");
    std::fs::write(&path, b"original").unwrap();
    let mut engine = Engine::default();
    engine.execute("h=plot([1 2]);C={h};").unwrap();
    let workspace = engine.workspace();
    let error =
        files::save_array_workspace_v73(&path, &workspace, &AtomicBool::new(false)).unwrap_err();
    assert!(error.contains("graphics"));
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
    assert!(engine.array("C").unwrap().same_snapshot(&workspace["C"]));
    let cancel = AtomicBool::new(true);
    assert!(files::save_array_workspace_v73(&path, &expected(), &cancel).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
    cancel.store(false, Ordering::Relaxed);
    files::save_array_workspace_v73(&path, &expected(), &cancel).unwrap();
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}
#[test]
fn script_save_load_of_nested_types_uses_real_v73_codec() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute("A=[1 2];s=char([65 0 55296]);C={A,s,{logical([0 1]),complex(A,-A)}};save('typed.mat','A','s','C','-v7.3');").unwrap();
    let before = e.workspace();
    e.execute("clear;load('typed.mat');").unwrap();
    for (name, value) in before {
        same(&value, &e.array(&name).unwrap());
    }
}

#[test]
fn maximum_flat_cell_roundtrip_preserves_shared_empty_slots() {
    let stop = AtomicBool::new(false);
    let source: Array = CellArray::empty(1, 65536, &stop).unwrap().into();
    let values = BTreeMap::from([("Maximum".to_owned(), source.clone())]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("maximum.mat");
    files::save_array_workspace_v73(&path, &values, &stop).unwrap();
    let catalog = files::inspect_workspace(&path, &stop).unwrap();
    let actual = files::load_selected_arrays(&catalog, &names(&["Maximum"]), &stop).unwrap();
    same(&source, &actual["Maximum"]);
    let Array::Cell(cell) = &actual["Maximum"] else {
        panic!("Cell lost")
    };
    assert_eq!((cell.rows(), cell.cols(), cell.len()), (1, 65536, 65536));
    for i in 0..cell.len() {
        assert!(Arc::ptr_eq(
            cell.get_linear(0).unwrap(),
            cell.get_linear(i).unwrap()
        ));
        let Array::Numeric(empty) = cell.get_linear(i).unwrap().as_ref() else {
            panic!("Empty type lost")
        };
        assert_eq!((empty.rows(), empty.cols(), empty.len()), (0, 0, 0));
    }
    assert!(values["Maximum"].same_snapshot(&source));
}
