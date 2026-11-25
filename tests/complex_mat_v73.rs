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
/// Packed 8-byte single-precision wire record; the caller picks the member order.
fn single_descriptor(first: &str, second: &str) -> TypeDescriptor {
    TypeDescriptor::Compound(CompoundType {
        fields: vec![
            CompoundField::typed::<f32>(first, 0, 0),
            CompoundField::typed::<f32>(second, 4, 1),
        ],
        size: 8,
    })
}
/// Independent raw fixture write (never the MatRust writer): one `[T; 2]` per
/// record, in HDF5 row-major order, laid out exactly as `desc` declares.
fn write_pairs<T: Copy>(ds: &hdf5::Dataset, desc: &TypeDescriptor, wire: &[[T; 2]]) {
    assert_eq!(desc.size(), std::mem::size_of::<[T; 2]>());
    assert_eq!(ds.size(), wire.len());
    let dtype = hdf5::Datatype::from_descriptor(desc).unwrap();
    hdf5::sync::sync(|| {
        // SAFETY: `wire` holds one `[T; 2]` per record of the whole dataset and
        // `desc` has that array's exact size (both checked above).
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
            .shape((0, 48 * 1024 * 1024 + 1))
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
        if info.name == "rank" {
            // A rank-3 complex dataset is a valid 2x2x2 array since the legacy N-D work;
            // never written, so HDF5 reads its zero fill value.
            assert!(info.unavailable.is_none());
            let loaded = files::load_selected_arrays(&cat, &names(&["rank"]), &stop).unwrap();
            let Array::Complex(rank) = &loaded["rank"] else {
                panic!("rank class lost")
            };
            assert_eq!(rank.dims().as_ref(), &[2, 2, 2]);
            assert!(rank.data().iter().all(|z| z.re == 0. && z.im == 0.));
            continue;
        }
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

#[test]
fn complex_v73_single_independent_wire_fixture_loads_exact_pairs_with_single_class() {
    let stop = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("g76.mat");
    // HDF5 shape (3,2): MATLAB reverses it to 2x3, and HDF5 row-major order is
    // MATLAB column-major order.
    let real = [[1.5_f32, 2.25], [3.5, 4.25], [5.5, 6.25]];
    let imag = [[-0.25_f32, -1.], [-2., -3.], [-4., -5.]];
    let desc = single_descriptor("imag", "real");
    synthetic(&path, |f| {
        let ds = f
            .new_dataset_builder()
            .empty_as(&desc)
            .shape((3, 2))
            .create("Z")
            .unwrap();
        class(&ds, "single");
        let wire: Vec<[f32; 2]> = (0..3)
            .flat_map(|i| (0..2).map(move |j| [imag[i][j], real[i][j]]))
            .collect();
        write_pairs(&ds, &desc, &wire);
    });
    {
        // The wire really is the packed 8-byte (imag, real) single record.
        let raw = hdf5::File::open(&path).unwrap();
        let ds = raw.dataset("Z").unwrap();
        assert_eq!(ds.shape(), vec![3, 2]);
        assert!(ds.chunk().is_none());
        assert_eq!(ds.dtype().unwrap().to_descriptor().unwrap(), desc);
        assert_eq!(ds.dtype().unwrap().size(), 8);
    }
    let cat = files::inspect_workspace(&path, &stop).unwrap();
    let info = cat.variables().find(|v| v.name == "Z").unwrap();
    assert_eq!(info.unavailable, None);
    assert_eq!(info.dimensions, vec![2, 3]);
    assert_eq!(info.class, "single");
    assert!(info.complex);
    assert_eq!(info.decoded_bytes, Some(96));
    assert_eq!(info.storage_bytes(), Some(96));
    let out = files::load_selected_arrays(&cat, &names(&["Z"]), &stop).unwrap();
    let z = &out["Z"];
    assert_eq!(z.class(), "single");
    assert_eq!((z.rows(), z.cols()), (2, 3));
    let expected = [
        Complex64::new(1.5, -0.25),
        Complex64::new(2.25, -1.),
        Complex64::new(3.5, -2.),
        Complex64::new(4.25, -3.),
        Complex64::new(5.5, -4.),
        Complex64::new(6.25, -5.),
    ];
    assert_eq!(
        bits(z),
        expected
            .iter()
            .map(|z| (z.re.to_bits(), z.im.to_bits()))
            .collect::<Vec<_>>()
    );
    // The legacy all-double API still refuses complex data.
    assert!(
        files::load_selected_workspace(&cat, &names(&["Z"]), &stop)
            .unwrap_err()
            .contains("typed")
    );
    let mut engine = Engine::default();
    engine.set_file_directory(dir.path()).unwrap();
    engine
        .execute(
            "load('g76.mat','Z');ok=isa(Z,'single')&&~isreal(Z)&&isequal(size(Z),[2 3])\
             &&Z(1)==1.5-0.25i&&Z(2,3)==6.25-5i&&Z(1,2)==3.5-2i;",
        )
        .unwrap();
    assert_eq!(engine.array("Z").unwrap().class(), "single");
    assert_eq!(bits(&engine.array("Z").unwrap()), bits(z));
    assert!(matches!(
        engine.array("ok").unwrap().scalar(0, 0).unwrap(),
        matrust::Scalar::Logical(true)
    ));
}

#[test]
fn complex_v73_single_class_and_exact_member_widths_must_agree() {
    let stop = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("widths.mat");
    let float4 = || TypeDescriptor::Float(hdf5::types::FloatSize::U4);
    let float8 = || TypeDescriptor::Float(hdf5::types::FloatSize::U8);
    let compound = |fields: &[(&str, TypeDescriptor, usize)], size| {
        TypeDescriptor::Compound(CompoundType {
            fields: fields
                .iter()
                .enumerate()
                .map(|(index, (name, ty, offset))| {
                    CompoundField::new(name, ty.clone(), *offset, index)
                })
                .collect(),
            size,
        })
    };
    let refused = [
        (
            "double_class_single_members",
            single_descriptor("real", "imag"),
            "double",
        ),
        (
            "single_class_double_members",
            descriptor("real", "imag"),
            "single",
        ),
        (
            "single_mixed_widths",
            compound(&[("real", float4(), 0), ("imag", float8(), 4)], 12),
            "single",
        ),
        (
            "double_mixed_widths",
            compound(&[("real", float8(), 0), ("imag", float4(), 8)], 12),
            "double",
        ),
        (
            "single_padded",
            compound(&[("real", float4(), 0), ("imag", float4(), 8)], 16),
            "single",
        ),
        (
            "single_trailing_padding",
            compound(&[("real", float4(), 0), ("imag", float4(), 4)], 12),
            "single",
        ),
        (
            "single_leading_padding",
            compound(&[("real", float4(), 4), ("imag", float4(), 8)], 12),
            "single",
        ),
        (
            "double_padded",
            compound(&[("real", float8(), 0), ("imag", float8(), 16)], 24),
            "double",
        ),
        (
            "single_wrong_names",
            single_descriptor("re", "im"),
            "single",
        ),
        (
            "single_extra_member",
            compound(
                &[
                    ("real", float4(), 0),
                    ("imag", float4(), 4),
                    ("extra", float4(), 8),
                ],
                12,
            ),
            "single",
        ),
        (
            "single_integer_members",
            compound(
                &[
                    ("real", TypeDescriptor::Integer(hdf5::types::IntSize::U4), 0),
                    ("imag", TypeDescriptor::Integer(hdf5::types::IntSize::U4), 4),
                ],
                8,
            ),
            "single",
        ),
        (
            "int32_class_single_members",
            single_descriptor("real", "imag"),
            "int32",
        ),
    ];
    // Binary64 values that single precision cannot hold, and their nearest singles.
    let fine = [[0.1_f64, 1. + 2f64.powi(-40)], [-1. / 3., f64::MAX]];
    let coarse = fine.map(|pair| pair.map(|value| value.min(f32::MAX as f64) as f32));
    synthetic(&path, |f| {
        for (name, desc, c) in &refused {
            // An existing datatype is stored as is: the builder would re-lay out a
            // bare descriptor and silently drop the padding under test.
            let ds = f
                .new_dataset_builder()
                .empty_as(hdf5::Datatype::from_descriptor(desc).unwrap())
                .shape((1, 2))
                .create(*name)
                .unwrap();
            assert_eq!(
                &ds.dtype().unwrap().to_descriptor().unwrap(),
                desc,
                "{name}"
            );
            class(&ds, c);
        }
        // Never written or allocated: one record past the 128 MiB decoded budget,
        // which a complex single is charged at its 16-byte backing width.
        let desc = single_descriptor("real", "imag");
        let ds = f
            .new_dataset_builder()
            .empty_as(&desc)
            .shape((1, 8_388_609))
            .create("single_over_budget")
            .unwrap();
        class(&ds, "single");
        let ds = f
            .new_dataset_builder()
            .empty_as(&desc)
            .shape((2, 1))
            .create("ok_single")
            .unwrap();
        class(&ds, "single");
        write_pairs(&ds, &desc, &coarse);
        let desc = descriptor("real", "imag");
        let ds = f
            .new_dataset_builder()
            .empty_as(&desc)
            .shape((2, 1))
            .create("ok_double")
            .unwrap();
        class(&ds, "double");
        write_pairs(&ds, &desc, &fine);
    });
    let cat = files::inspect_workspace(&path, &stop).unwrap();
    assert_eq!(cat.variables().len(), refused.len() + 3);
    for info in cat.variables() {
        if info.name.starts_with("ok_") {
            continue;
        }
        let reason = info
            .unavailable
            .as_deref()
            .unwrap_or_else(|| panic!("{}", info.name));
        assert!(
            reason.contains(if info.name == "single_over_budget" {
                "128 MiB"
            } else {
                "matching its class"
            }),
            "{}: {reason}",
            info.name
        );
        assert!(info.complex, "{}", info.name);
        assert!(files::load_selected_arrays(&cat, &names(&[&info.name]), &stop).is_err());
    }
    let out =
        files::load_selected_arrays(&cat, &names(&["ok_single", "ok_double"]), &stop).unwrap();
    for (name, class) in [("ok_single", "single"), ("ok_double", "double")] {
        let info = cat.variables().find(|v| v.name == name).unwrap();
        assert_eq!(info.unavailable, None);
        assert_eq!((info.class.as_str(), info.complex), (class, true));
        assert_eq!(info.dimensions, vec![1, 2]);
        assert_eq!(info.decoded_bytes, Some(32));
        assert_eq!(out[name].class(), class);
    }
    // Double keeps every bit; single holds exactly the widened wire values.
    assert_eq!(
        out["ok_double"].as_complex().unwrap().data(),
        &fine.map(|[re, im]| Complex64::new(re, im))
    );
    assert_eq!(
        out["ok_single"].as_complex().unwrap().data(),
        &coarse.map(|[re, im]| Complex64::new(re.into(), im.into()))
    );
    assert_ne!(bits(&out["ok_single"]), bits(&out["ok_double"]));
}

#[test]
fn complex_v73_single_zero_extents_special_values_and_nd_layouts_keep_class_and_order() {
    let stop = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("single.mat");
    let special = [
        [0.1_f32, -0.1],
        [f32::MAX, f32::MIN_POSITIVE],
        [f32::from_bits(1), -f32::from_bits(1)],
        [-0., 0.],
        [f32::INFINITY, f32::NEG_INFINITY],
        [16_777_216., -16_777_215.],
        [f32::NAN, 1.],
    ];
    // MATLAB 3x2x4 is HDF5 (4,2,3); each record encodes its own MATLAB subscripts.
    let subscripts = |scale: f64| -> Vec<[f64; 2]> {
        let mut values = Vec::new();
        for k in 0..4 {
            for j in 0..2 {
                for i in 0..3 {
                    let linear = (i + 3 * (j + 2 * k)) as f64;
                    values.push([
                        (100 * (i + 1) + 10 * (j + 1) + k + 1) as f64 + scale,
                        -linear - 0.5 - scale,
                    ]);
                }
            }
        }
        values
    };
    let single_nd: Vec<[f32; 2]> = subscripts(0.)
        .iter()
        .map(|pair| pair.map(|value| value as f32))
        .collect();
    // The double control carries a fraction that single precision would lose.
    let double_nd = subscripts(1e-9);
    synthetic(&path, |f| {
        let reversed = single_descriptor("imag", "real");
        let normal = single_descriptor("real", "imag");
        for (name, shape) in [("E03", (3, 0)), ("E30", (0, 3))] {
            let ds = f
                .new_dataset_builder()
                .empty_as(&reversed)
                .shape(shape)
                .create(name)
                .unwrap();
            class(&ds, "single");
        }
        let ds = f
            .new_dataset_builder()
            .empty_as(&normal)
            .shape((1, special.len()))
            .create("S")
            .unwrap();
        class(&ds, "single");
        write_pairs(&ds, &normal, &special);
        let ds = f
            .new_dataset_builder()
            .empty_as(&normal)
            .shape((4, 2, 3))
            .create("Nc")
            .unwrap();
        class(&ds, "single");
        write_pairs(&ds, &normal, &single_nd);
        let ds = f
            .new_dataset_builder()
            .empty_as(&reversed)
            .shape((4, 2, 3))
            .chunk((3, 2, 2))
            .deflate(1)
            .create("Nz")
            .unwrap();
        class(&ds, "single");
        let swapped: Vec<[f32; 2]> = single_nd.iter().map(|&[re, im]| [im, re]).collect();
        write_pairs(&ds, &reversed, &swapped);
        let desc = descriptor("real", "imag");
        let ds = f
            .new_dataset_builder()
            .empty_as(&desc)
            .shape((4, 2, 3))
            .create("D")
            .unwrap();
        class(&ds, "double");
        write_pairs(&ds, &desc, &double_nd);
    });
    let cat = files::inspect_workspace(&path, &stop).unwrap();
    let all = names(&["E03", "E30", "S", "Nc", "Nz", "D"]);
    for (name, dims, class, compressed) in [
        ("E03", vec![0, 3], "single", false),
        ("E30", vec![3, 0], "single", false),
        ("S", vec![special.len(), 1], "single", false),
        ("Nc", vec![3, 2, 4], "single", false),
        ("Nz", vec![3, 2, 4], "single", true),
        ("D", vec![3, 2, 4], "double", false),
    ] {
        let info = cat.variables().find(|v| v.name == name).unwrap();
        assert_eq!(info.unavailable, None, "{name}");
        assert_eq!(info.dimensions, dims, "{name}");
        assert_eq!((info.class.as_str(), info.complex), (class, true), "{name}");
        assert_eq!(info.compressed, compressed, "{name}");
        // Both precisions are charged at the 16-byte Complex64 backing.
        let count: usize = dims.iter().product();
        assert_eq!(info.decoded_bytes, Some(16 * count as u64), "{name}");
        assert_eq!(info.storage_bytes(), Some(16 * count as u64), "{name}");
    }
    let out = files::load_selected_arrays(&cat, &all, &stop).unwrap();
    for (name, rows, cols) in [("E03", 0, 3), ("E30", 3, 0)] {
        let empty = &out[name];
        assert!(empty.is_complex() && empty.class() == "single", "{name}");
        assert_eq!((empty.rows(), empty.cols()), (rows, cols), "{name}");
        assert!(empty.as_complex().unwrap().data().is_empty());
    }
    assert_eq!(out["S"].class(), "single");
    let s = out["S"].as_complex().unwrap().data();
    for (actual, [re, im]) in s.iter().zip(special) {
        if re.is_nan() {
            assert!(actual.re.is_nan() && actual.im == 1.);
        } else {
            // Widening is exact, including signed zero and subnormals.
            assert_eq!(
                (actual.re.to_bits(), actual.im.to_bits()),
                (f64::from(re).to_bits(), f64::from(im).to_bits())
            );
        }
    }
    assert_ne!(s[0].re, 0.1);
    for (name, class, expected) in [
        ("Nc", "single", subscripts(0.)),
        ("Nz", "single", subscripts(0.)),
        ("D", "double", double_nd.clone()),
    ] {
        assert_eq!(out[name].class(), class, "{name}");
        let z = out[name].as_complex().unwrap();
        assert_eq!(z.dims().as_ref(), &[3, 2, 4], "{name}");
        // HDF5 row-major order is MATLAB column-major order: no transposition.
        assert_eq!(
            z.data(),
            expected
                .iter()
                .map(|&[re, im]| Complex64::new(re, im))
                .collect::<Vec<_>>(),
            "{name}"
        );
    }
    let mut engine = Engine::default();
    engine.set_file_directory(dir.path()).unwrap();
    engine
        .execute(
            "load('single.mat');ok=isa(Nc,'single')&&isa(Nz,'single')&&isa(S,'single')\
             &&isa(E03,'single')&&isa(E30,'single')&&isa(D,'double')\
             &&isequal(size(Nz),[3 2 4])&&isequal(size(E03),[0 3])&&isequal(size(E30),[3 0])\
             &&Nz(3,2,4)==324-23.5i&&Nc(1,2,1)==121-3.5i&&isequal(Nc,Nz)&&~isreal(Nz);",
        )
        .unwrap();
    assert!(matches!(
        engine.array("ok").unwrap().scalar(0, 0).unwrap(),
        matrust::Scalar::Logical(true)
    ));
}

/// Raw HDF5 view of one saved dataset, read without the MatRust loader:
/// shape, stored type, `MATLAB_class` and every record as packed f32 pairs.
fn raw_single(path: &Path, name: &str) -> (Vec<usize>, TypeDescriptor, String, Vec<[f32; 2]>) {
    let file = hdf5::File::open(path).unwrap();
    let ds = file.dataset(name).unwrap();
    let attr = ds.attr("MATLAB_class").unwrap();
    let class = match attr.dtype().unwrap().size() {
        6 => attr
            .read_scalar::<FixedAscii<6>>()
            .unwrap()
            .as_str()
            .to_owned(),
        n => panic!("class attribute width {n}"),
    };
    let stored = ds.dtype().unwrap().to_descriptor().unwrap();
    let memory = hdf5::Datatype::from_descriptor(&single_descriptor("real", "imag")).unwrap();
    let mut records = vec![[0f32; 2]; ds.size()];
    if !records.is_empty() {
        hdf5::sync::sync(|| {
            // SAFETY: `records` holds one packed 8-byte pair per dataset record.
            hdf5::h5check(unsafe {
                hdf5_sys::h5d::H5Dread(
                    ds.id(),
                    memory.id(),
                    hdf5_sys::h5s::H5S_ALL,
                    hdf5_sys::h5s::H5S_ALL,
                    hdf5_sys::h5p::H5P_DEFAULT,
                    records.as_mut_ptr().cast(),
                )
            })
        })
        .unwrap();
    }
    (ds.shape(), stored, class, records)
}

#[test]
fn complex_v73_tagged_single_save_writes_f32_wire_records_and_single_class() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = Engine::default();
    engine.set_file_directory(dir.path()).unwrap();
    engine
        .execute(
            "a=single([0 0]); a(1)=1i; a(2)=3-2i; d=complex(0.1,-0.2); \
             Alias=a; save('t.mat','a','d','-v7.3');",
        )
        .unwrap();
    let path = dir.path().join("t.mat");
    // Baseline wrote (2,1) (real:f64, imag:f64) with MATLAB_class=double.
    let (shape, stored, class, records) = raw_single(&path, "a");
    assert_eq!(shape, [2, 1]);
    assert_eq!(stored, single_descriptor("real", "imag"));
    assert_eq!(stored.size(), 8);
    assert_eq!(class, "single");
    assert_eq!(records, [[0., 1.], [3., -2.]]);
    // The double control keeps exact 16-byte records.
    let file = hdf5::File::open(&path).unwrap();
    let d = file.dataset("d").unwrap();
    assert_eq!(
        d.dtype().unwrap().to_descriptor().unwrap(),
        descriptor("real", "imag")
    );
    let attr = d.attr("MATLAB_class").unwrap();
    assert_eq!(
        attr.read_scalar::<FixedAscii<6>>().unwrap().as_str(),
        "double"
    );
    drop((attr, d, file));
    engine.execute("clear a d; load('t.mat');").unwrap();
    assert_eq!(engine.array("a").unwrap().class(), "single");
    assert_eq!(engine.array("d").unwrap().class(), "double");
    assert_eq!(
        bits(&engine.array("a").unwrap()),
        bits(&engine.array("Alias").unwrap())
    );
    assert_eq!(
        bits(&engine.array("d").unwrap()),
        [(0.1_f64.to_bits(), (-0.2_f64).to_bits())]
    );
}
