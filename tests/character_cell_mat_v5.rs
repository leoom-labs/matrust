use matrust::{
    Array, CellArray, CharacterRow, Complex64, ComplexMatrix, Engine, LogicalMatrix, Matrix, files,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

fn names(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|v| (*v).into()).collect()
}
fn same(a: &Array, b: &Array) {
    assert_eq!(
        (a.class(), a.rows(), a.cols(), a.is_complex()),
        (b.class(), b.rows(), b.cols(), b.is_complex())
    );
    match (a, b) {
        (Array::Numeric(a), Array::Numeric(b)) => assert_eq!(
            a.data().iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            b.data().iter().map(|v| v.to_bits()).collect::<Vec<_>>()
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
        _ => panic!("Different typed arrays"),
    }
}
fn cell(rows: usize, cols: usize, values: Vec<Array>) -> Array {
    CellArray::from_shared(
        rows,
        cols,
        values.into_iter().map(Arc::new).collect(),
        &AtomicBool::new(false),
    )
    .unwrap()
    .into()
}
fn chars(units: &[u16]) -> Array {
    CharacterRow::from_units(units.to_vec(), &AtomicBool::new(false))
        .unwrap()
        .into()
}
fn element(kind: u32, data: &[u8]) -> Vec<u8> {
    let mut out = kind.to_le_bytes().to_vec();
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(data);
    out.resize(out.len() + (8 - data.len() % 8) % 8, 0);
    out
}
fn matrix(name: &str, class: u32, rows: i32, cols: i32, payload: &[u8]) -> Vec<u8> {
    let mut body = element(6, &[class.to_le_bytes(), 0u32.to_le_bytes()].concat());
    body.extend(element(
        5,
        &[rows.to_le_bytes(), cols.to_le_bytes()].concat(),
    ));
    body.extend(element(1, name.as_bytes()));
    body.extend(payload);
    element(14, &body)
}
fn file(path: &Path, elements: &[Vec<u8>]) {
    let mut bytes = vec![b' '; 128];
    bytes[116..124].fill(0);
    bytes[124..128].copy_from_slice(&[0, 1, b'I', b'M']);
    for item in elements {
        bytes.extend(item);
    }
    std::fs::write(path, bytes).unwrap();
}
fn typed(path: &Path, wanted: &[&str]) -> BTreeMap<String, Array> {
    let stop = AtomicBool::new(false);
    let catalog = files::inspect_workspace(path, &stop).unwrap();
    files::load_selected_arrays(&catalog, &names(wanted), &stop).unwrap()
}

#[test]
fn nested_typed_roundtrip_preserves_shapes_bits_utf16_and_source_aliases() {
    let stop = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    let real: Array = Matrix::from_column_slice(
        2,
        2,
        &[
            -0.,
            f64::from_bits(1),
            f64::INFINITY,
            f64::from_bits(0x7ff8000000000031),
        ],
    )
    .unwrap()
    .into();
    let z: Array = ComplexMatrix::from_column_vec(
        1,
        2,
        vec![Complex64::new(-0., 0.), Complex64::new(2., -0.)],
        &stop,
    )
    .unwrap()
    .into();
    let logical: Array = LogicalMatrix::from_numeric(
        &Matrix::from_column_slice(2, 2, &[1., 0., 0., 1.]).unwrap(),
        &stop,
    )
    .unwrap()
    .into();
    let c = cell(
        2,
        3,
        vec![
            real.clone(),
            chars(&[65, 0, 955, 0xd83d, 0xde00, 0xd800]),
            z.clone(),
            logical,
            cell(1, 2, vec![chars(&[]), real.clone()]),
            CellArray::empty(0, 3, &stop).unwrap().into(),
        ],
    );
    let workspace = BTreeMap::from([
        ("C".into(), c),
        ("Text".into(), chars(&[0xdfff, 66])),
        ("Empty".into(), chars(&[])),
        ("C40".into(), CellArray::empty(4, 0, &stop).unwrap().into()),
        ("Z".into(), z),
    ]);
    let aliases = workspace.clone();
    for compressed in [false, true] {
        let path = dir.path().join(if compressed {
            "compressed.mat"
        } else {
            "plain.mat"
        });
        if compressed {
            files::save_array_workspace(&path, &workspace, &stop).unwrap();
        } else {
            files::save_array_workspace_uncompressed(&path, &workspace, &stop).unwrap();
        }
        let catalog = files::inspect_workspace(&path, &stop).unwrap();
        for info in catalog.variables() {
            assert!(info.unavailable.is_none(), "{:?}", info);
            assert_eq!(
                info.storage_bytes(),
                Some(workspace[&info.name].bytes() as u64)
            );
        }
        let loaded =
            files::load_selected_arrays(&catalog, &workspace.keys().cloned().collect(), &stop)
                .unwrap();
        for (name, a) in &workspace {
            same(a, &loaded[name]);
            assert!(a.same_snapshot(&aliases[name]));
        }
        assert!(files::load_selected_workspace(&catalog, &names(&["C"]), &stop).is_err());
    }
}

#[test]
fn independent_wire_accepts_utf16_utf8_compact_and_column_major_cells() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wire.mat");
    let raw = element(
        17,
        &[
            65u16.to_le_bytes(),
            0u16.to_le_bytes(),
            0xd800u16.to_le_bytes(),
        ]
        .concat(),
    );
    let compact = [((2u32 << 16) | 4).to_le_bytes(), [0xbb, 0x03, 0, 0]].concat();
    let children = [
        matrix("", 4, 1, 1, &compact),
        matrix("", 6, 1, 1, &element(9, &7f64.to_le_bytes())),
    ]
    .concat();
    file(
        &path,
        &[
            matrix("Raw", 4, 1, 3, &raw),
            matrix("Utf8", 4, 1, 2, &element(16, "A\u{03bb}".as_bytes())),
            matrix("C", 1, 2, 1, &children),
        ],
    );
    let loaded = typed(&path, &["Raw", "Utf8", "C"]);
    same(&loaded["Raw"], &chars(&[65, 0, 0xd800]));
    same(&loaded["Utf8"], &chars(&[65, 955]));
    same(
        &loaded["C"],
        &cell(2, 1, vec![chars(&[955]), Matrix::scalar(7.).into()]),
    );
}

#[test]
fn authentic_reference_character_and_cell_inputs_load_through_typed_path() {
    let dir = tempfile::tempdir().unwrap();
    // Original reference-generated files; literal dependencies are part of the
    // compiled-input freeze, and all test I/O stays inside this private folder.
    for (version, bytes) in [
        (
            6,
            include_bytes!("../tests/fixtures/mat/mat-import-v6.mat").as_slice(),
        ),
        (
            7,
            include_bytes!("../tests/fixtures/mat/mat-import-v7.mat").as_slice(),
        ),
    ] {
        let path = dir.path().join(format!("vendor-v{version}.mat"));
        std::fs::write(&path, bytes).unwrap();
        let loaded = typed(&path, &["text_data", "cell_data"]);
        same(
            &loaded["text_data"],
            &chars(&"MatRust".encode_utf16().collect::<Vec<_>>()),
        );
        same(
            &loaded["cell_data"],
            &cell(
                1,
                2,
                vec![Matrix::scalar(1.).into(), Matrix::scalar(2.).into()],
            ),
        );
    }
}

#[test]
fn malformed_nested_cells_and_character_shapes_are_not_silently_canonicalized() {
    let dir = tempfile::tempdir().unwrap();
    let stop = AtomicBool::new(false);
    let path = dir.path().join("bad.mat");
    let leaf = matrix("", 6, 1, 1, &element(9, &1f64.to_le_bytes()));
    let mut deep = leaf.clone();
    for _ in 0..17 {
        deep = matrix("", 1, 1, 1, &deep);
    }
    let cases = vec![
        matrix("Bad", 1, 1, 2, &leaf),
        matrix("Bad", 1, 1, 0, &leaf),
        matrix("Bad", 1, 65537, 0, &[]),
        matrix(
            "Bad",
            1,
            1,
            1,
            &matrix("named", 6, 1, 1, &element(9, &1f64.to_le_bytes())),
        ),
        matrix("Bad", 1, 1, 1, &deep),
        matrix("Bad", 0x804, 1, 1, &element(17, &[65, 0])),
        matrix("Bad", 4, 1, 1, &element(16, "\u{1f600}".as_bytes())), // Two UTF-16 units, declared one.
        matrix("Bad", 4, 1, 1, &element(16, &[0xff])),
        matrix("Bad", 4, 1, 2, &element(17, &[65, 0])),
        matrix(
            "Bad",
            4,
            1,
            1,
            &[element(17, &[65, 0]), element(17, &[66, 0])].concat(),
        ),
    ];
    for (i, bad) in cases.into_iter().enumerate() {
        file(
            &path,
            &[
                matrix("Good", 6, 1, 1, &element(9, &3f64.to_le_bytes())),
                bad,
            ],
        );
        let catalog = files::inspect_workspace(&path, &stop).unwrap();
        assert!(
            files::load_selected_arrays(&catalog, &names(&["Bad"]), &stop).is_err(),
            "case {i}"
        );
        assert_eq!(
            files::load_selected_arrays(&catalog, &names(&["Good"]), &stop).unwrap()["Good"]
                .as_numeric()
                .unwrap()
                .data(),
            &[3.]
        );
    }
}

#[test]
fn failed_multi_variable_script_load_preserves_all_original_bindings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.mat");
    let bad = matrix(
        "Bad",
        1,
        1,
        1,
        &matrix(
            "",
            6,
            1,
            1,
            &element(13, &9_007_199_254_740_993u64.to_le_bytes()),
        ),
    );
    file(
        &path,
        &[
            matrix("A", 6, 1, 1, &element(9, &123f64.to_le_bytes())),
            bad,
        ],
    );
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute("A=[4 5];C={A,'retained'};filename='bad.mat';")
        .unwrap();
    let before = e.workspace();
    assert!(
        e.execute("load(filename);")
            .unwrap_err()
            .contains("precision")
    );
    assert_eq!(before.len(), e.workspace_count());
    for (name, value) in before {
        assert!(value.same_snapshot(&e.array(&name).unwrap()));
    }
}

#[test]
fn save_preflights_aggregate_budget_and_cancellation_before_atomic_publication() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("kept.mat");
    let stop = AtomicBool::new(false);
    let text = chars(&vec![65; 65536]);
    let mut workspace = BTreeMap::new();
    for i in 0..2049 {
        workspace.insert(format!("s{i}"), text.clone());
    }
    std::fs::write(&path, b"original").unwrap();
    assert!(files::save_array_workspace(&path, &workspace, &stop).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
    workspace.clear();
    workspace.insert("s".into(), text.clone());
    stop.store(true, Ordering::Relaxed);
    assert!(files::save_array_workspace_uncompressed(&path, &workspace, &stop).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    assert!(text.same_snapshot(&workspace["s"]));
}

#[test]
fn script_compressed_plain_save_reopen_retains_nested_values() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute("A=[1 2];s=char([65 0 955 55296]);C={A,s;logical([1 0]),{complex(A,A)}};save('mixed.mat','C','s','-v7');save('plain.mat','C','s','-v6');").unwrap();
    let c = e.array("C").unwrap();
    let s = e.array("s").unwrap();
    e.execute("clear C s;load('mixed.mat');").unwrap();
    same(&c, &e.array("C").unwrap());
    same(&s, &e.array("s").unwrap());
    e.execute("clear C s;load('plain.mat');").unwrap();
    same(&c, &e.array("C").unwrap());
    same(&s, &e.array("s").unwrap());
}
