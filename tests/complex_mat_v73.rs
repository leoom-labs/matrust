use hdf5::types::{CompoundField, CompoundType, FixedAscii, TypeDescriptor};
use matrust::{Array, Complex64, ComplexMatrix, Engine, files};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

fn names(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|s| (*s).into()).collect()
}
fn bits(value: &Array) -> Vec<(u64, u64)> {
    value
        .as_complex()
        .unwrap()
        .data()
        .iter()
        .map(|z| (z.re.to_bits(), z.im.to_bits()))
        .collect()
}
fn matrix(rows: usize, cols: usize, values: Vec<Complex64>) -> Array {
    ComplexMatrix::from_column_vec(rows, cols, values, &AtomicBool::new(false))
        .unwrap()
        .into()
}
fn descriptor(real: &str, imag: &str) -> TypeDescriptor {
    TypeDescriptor::Compound(CompoundType {
        fields: vec![
            CompoundField::typed::<f64>(real, 0, 0),
            CompoundField::typed::<f64>(imag, 8, 1),
        ],
        size: 16,
    })
}
fn synthetic(path: &Path, body: impl FnOnce(&hdf5::File)) {
    {
        let file = hdf5::File::with_options()
            .with_fcpl(|p| p.userblock(512))
            .create(path)
            .unwrap();
        body(&file);
    }
    let mut header = [0; 128];
    let description = b"MATLAB 7.3 MAT-file,";
    header[..description.len()].copy_from_slice(description);
    header[124..].copy_from_slice(&[0, 2, b'I', b'M']);
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .write_all(&header)
        .unwrap();
}
fn class(ds: &hdf5::Dataset, value: &str) {
    ds.new_attr::<FixedAscii<16>>()
        .create("MATLAB_class")
        .unwrap()
        .write_scalar(&FixedAscii::<16>::from_ascii(value).unwrap())
        .unwrap();
}

#[test]
fn complex_v73_authentic_matlab_c_api_fixture_loads_components_and_rejects_legacy_api() {
    let stop = AtomicBool::new(false);
    // MAT C-API fixture metadata remains in source-evidence.json.
    // Embed bytes so the build freeze pins the fixture and execution is cwd-independent.
    let temporary = tempfile::tempdir().unwrap();
    let fixture = temporary.path().join("vendor-types-v73.mat");
    std::fs::write(
        &fixture,
        include_bytes!("../tests/fixtures/mat/vendor-types-v73.mat"),
    )
    .unwrap();
    let catalog = files::inspect_workspace(&fixture, &stop).unwrap();
    let info = catalog
        .variables()
        .find(|v| v.name == "complex_data")
        .unwrap();
    assert!(info.complex && info.unavailable.is_none());
    assert_eq!(info.decoded_bytes, Some(96));
    let loaded = files::load_selected_arrays(&catalog, &names(&["complex_data"]), &stop).unwrap();
    let z = loaded["complex_data"].as_complex().unwrap();
    assert_eq!((z.rows(), z.cols()), (2, 3));
    for (i, z) in z.data().iter().enumerate() {
        assert_eq!(*z, Complex64::new(i as f64 + 1., i as f64 + 0.5));
    }
    assert!(
        files::load_selected_workspace(&catalog, &names(&["complex_data"]), &stop)
            .unwrap_err()
            .contains("typed")
    );
}

#[test]
fn complex_v73_roundtrip_preserves_exact_bits_type_empty_shapes_and_borrowed_inputs() {
    let stop = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("complex.mat");
    let values = BTreeMap::from([
        (
            "Z".into(),
            matrix(
                2,
                3,
                vec![
                    Complex64::new(1., 2.),
                    Complex64::new(-0., 0.),
                    Complex64::new(f64::from_bits(1), -f64::from_bits(1)),
                    Complex64::new(f64::MAX, -f64::MAX),
                    Complex64::new(f64::INFINITY, f64::NEG_INFINITY),
                    Complex64::new(
                        f64::from_bits(0x7ff8000000000042),
                        f64::from_bits(0xfff8000000000017),
                    ),
                ],
            ),
        ),
        (
            "ZeroImag".into(),
            matrix(1, 3, vec![Complex64::new(1., 0.); 3]),
        ),
        ("E00".into(), matrix(0, 0, vec![])),
        ("E03".into(), matrix(0, 3, vec![])),
        ("E40".into(), matrix(4, 0, vec![])),
    ]);
    let before = values.clone();
    files::save_array_workspace_v73(&path, &values, &stop).unwrap();
    let catalog = files::inspect_workspace(&path, &stop).unwrap();
    let selected = values.keys().cloned().collect();
    let loaded = files::load_selected_arrays(&catalog, &selected, &stop).unwrap();
    let raw = hdf5::File::open(&path).unwrap();
    for (name, original) in &values {
        assert!(original.same_snapshot(&before[name]));
        let actual = &loaded[name];
        assert!(actual.is_complex());
        assert_eq!(
            (actual.rows(), actual.cols()),
            (original.rows(), original.cols())
        );
        assert_eq!(bits(actual), bits(original));
        let ds = raw.dataset(name).unwrap();
        assert_eq!(ds.shape(), vec![original.cols(), original.rows()]);
        assert_eq!(
            ds.dtype().unwrap().to_descriptor().unwrap(),
            descriptor("real", "imag")
        );
        assert!(
            ds.attr("MATLAB_empty").is_err(),
            "Compound zero extent carries type without dimension marker"
        );
    }
}

#[test]
fn complex_v73_chunk_edges_and_long_columns_keep_column_major_order() {
    let stop = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    for (rows, cols) in [(3, 174765), (524291, 2)] {
        let source = matrix(
            rows,
            cols,
            (0..rows * cols)
                .map(|i| Complex64::new(i as f64, -(i as f64) - 0.25))
                .collect(),
        );
        let values = BTreeMap::from([("Z".into(), source.clone())]);
        let path = dir.path().join("chunks.mat");
        files::save_array_workspace_v73(&path, &values, &stop).unwrap();
        let raw = hdf5::File::open(&path).unwrap();
        let chunk = raw.dataset("Z").unwrap().chunk().unwrap();
        assert!(chunk.iter().product::<usize>() * 16 <= 256 * 1024);
        drop(raw);
        let cat = files::inspect_workspace(&path, &stop).unwrap();
        let out = files::load_selected_arrays(&cat, &names(&["Z"]), &stop).unwrap();
        assert_eq!(bits(&out["Z"]), bits(&source));
        assert!(source.same_snapshot(&values["Z"]));
    }
}

#[test]
fn complex_v73_invalid_members_class_rank_and_payload_budget_are_unavailable() {
    let stop = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.mat");
    synthetic(&path, |f| {
        for (name, desc, c) in [
            ("wrong_members", descriptor("r", "i"), "double"),
            ("wrong_class", descriptor("real", "imag"), "single"),
        ] {
            let ds = f
                .new_dataset_builder()
                .empty_as(&desc)
                .shape((1, 2))
                .create(name)
                .unwrap();
            class(&ds, c);
        }
        let desc = descriptor("real", "imag");
        let ds = f
            .new_dataset_builder()
            .empty_as(&desc)
            .shape((2, 2, 2))
            .create("rank")
            .unwrap();
        class(&ds, "double");
        let ds = f
            .new_dataset_builder()
            .empty_as(&desc)
            .shape((1, 8_388_609))
            .create("large")
            .unwrap();
        class(&ds, "double");
        let ds = f
            .new_dataset_builder()
            .empty_as(&desc)
            .shape((0, 16_777_217))
            .create("empty_dimension")
            .unwrap();
        class(&ds, "double");
        let ds = f
            .new_dataset_builder()
            .empty_as(&desc)
            .shape((1, 524289))
            .chunk((1, 524289))
            .create("chunk_budget")
            .unwrap();
        class(&ds, "double");
        let single = TypeDescriptor::Compound(CompoundType {
            fields: vec![
                CompoundField::typed::<f32>("real", 0, 0),
                CompoundField::typed::<f32>("imag", 4, 1),
            ],
            size: 8,
        });
        let ds = f
            .new_dataset_builder()
            .empty_as(&single)
            .shape((1, 1))
            .create("single_components")
            .unwrap();
        class(&ds, "double");
    });
    let cat = files::inspect_workspace(&path, &stop).unwrap();
    for info in cat.variables() {
        assert!(info.unavailable.is_some(), "{}", info.name);
        assert!(files::load_selected_arrays(&cat, &names(&[&info.name]), &stop).is_err());
    }
}

#[test]
fn complex_v73_reversed_compound_offsets_map_by_member_names() {
    let stop = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reversed.mat");
    synthetic(&path, |f| {
        let desc = descriptor("imag", "real");
        let dtype = hdf5::Datatype::from_descriptor(&desc).unwrap();
        let ds = f
            .new_dataset_builder()
            .empty_as(&desc)
            .shape((1, 2))
            .create("Z")
            .unwrap();
        class(&ds, "double");
        let wire = [7_f64, 1., 8., 2.];
        // Test fixture deliberately uses imag before real, unlike the writer.
        // Four f64 values match the two fixed 16-byte records and dataset shape.
        hdf5::sync::sync(|| {
            hdf5::h5check(unsafe {
                hdf5_sys::h5d::H5Dwrite(
                    ds.id(),
                    dtype.id(),
                    hdf5_sys::h5s::H5S_ALL,
                    hdf5_sys::h5s::H5S_ALL,
                    hdf5_sys::h5p::H5P_DEFAULT,
                    wire.as_ptr().cast(),
                )
            })
        })
        .unwrap();
    });
    let cat = files::inspect_workspace(&path, &stop).unwrap();
    let out = files::load_selected_arrays(&cat, &names(&["Z"]), &stop).unwrap();
    assert_eq!(
        out["Z"].as_complex().unwrap().data(),
        &[Complex64::new(1., 7.), Complex64::new(2., 8.)]
    );
}

#[test]
fn complex_v73_precancel_atomic_destination_and_script_typed_save_load() {
    let stop = AtomicBool::new(true);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.mat");
    std::fs::write(&path, b"preserved").unwrap();
    let values = BTreeMap::from([("Z".into(), matrix(1, 1, vec![Complex64::new(2., -3.)]))]);
    assert!(
        files::save_array_workspace_v73(&path, &values, &stop)
            .unwrap_err()
            .contains("cancel")
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"preserved");
    stop.store(false, Ordering::Relaxed);
    let mut engine = Engine::default();
    engine.set_file_directory(dir.path()).unwrap();
    engine.execute("Z=complex([1 2],[3 -4]);Alias=Z;save('data.mat','Z','-v7.3');clear Z;load('data.mat','Z');").unwrap();
    assert_eq!(
        bits(&engine.array("Z").unwrap()),
        bits(&engine.array("Alias").unwrap())
    );
}
