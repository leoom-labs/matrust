use matrust::{Array, files};
use std::{collections::BTreeSet, io::Write, path::Path, sync::atomic::AtomicBool};

fn names(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}
fn fixture() -> &'static Path {
    Path::new("tests/fixtures/mat/vendor-types-v73.mat")
}
fn class(dataset: &hdf5::Dataset, name: &str) {
    dataset
        .new_attr::<hdf5::types::FixedAscii<16>>()
        .create("MATLAB_class")
        .unwrap()
        .write_scalar(&hdf5::types::FixedAscii::<16>::from_ascii(name).unwrap())
        .unwrap();
}
fn create(path: &Path, body: impl FnOnce(&hdf5::File)) {
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

#[test]
fn v73_vendor_numeric_types_empty_shapes_and_chunk_order() {
    let cancel = AtomicBool::new(false);
    let catalog = files::inspect_workspace(fixture(), &cancel).unwrap();
    assert_eq!(catalog.variables().len(), 23);
    let selected = names(&[
        "double_data",
        "single_data",
        "int8_data",
        "uint8_data",
        "int16_data",
        "uint16_data",
        "int32_data",
        "uint32_data",
        "int64_data",
        "uint64_data",
        "scalar",
        "row",
        "column",
        "compressed",
        "empty_rows",
        "empty_columns",
    ]);
    let actual = files::load_selected_workspace(&catalog, &selected, &cancel).unwrap();
    let double = actual["double_data"].data();
    assert_eq!(double[0], 1.);
    assert_eq!(double[1].to_bits(), (-0.0_f64).to_bits());
    assert!(double[2].is_nan());
    assert_eq!(&double[3..], &[f64::INFINITY, f64::NEG_INFINITY, -4.25]);
    for (name, expected) in [
        ("single_data", [1., -0., 2.5, -3.25, 4., 5.]),
        ("int8_data", [-128., 127., 0., 1., -1., 42.]),
        ("uint8_data", [0., 255., 1., 2., 3., 4.]),
        ("int16_data", [-32768., 32767., 0., 1., -1., 42.]),
        ("uint16_data", [0., 65535., 1., 2., 3., 4.]),
        ("int32_data", [-2147483648., 2147483647., 0., 1., -1., 42.]),
        ("uint32_data", [0., 4294967295., 1., 2., 3., 4.]),
        (
            "int64_data",
            [-9007199254740992., 9007199254740992., 0., 1., -1., 42.],
        ),
        ("uint64_data", [0., 9007199254740992., 1., 2., 3., 4.]),
    ] {
        let a = &actual[name];
        assert_eq!((a.rows(), a.cols()), (2, 3), "{name}");
        assert_eq!(a.data(), expected, "{name}");
    }
    for (name, shape) in [
        ("empty_rows", (0, 7)),
        ("empty_columns", (5, 0)),
        ("row", (1, 5)),
        ("column", (5, 1)),
    ] {
        assert_eq!((actual[name].rows(), actual[name].cols()), shape);
    }
    assert_eq!(actual["scalar"].data(), &[12.5]);
    assert_eq!(
        (actual["compressed"].rows(), actual["compressed"].cols()),
        (1024, 128)
    );
    for (i, &value) in actual["compressed"].data().iter().enumerate() {
        assert_eq!(value, (i % 97) as f64);
    }
}

#[test]
fn v73_typed_logical_import_and_unsupported_numeric_values() {
    let cancel = AtomicBool::new(false);
    let catalog = files::inspect_workspace(fixture(), &cancel).unwrap();
    let selected = names(&["logical_data", "empty_logical"]);
    let typed = files::load_selected_arrays(&catalog, &selected, &cancel).unwrap();
    let Array::Logical(logical) = &typed["logical_data"] else {
        panic!("logical type lost")
    };
    assert_eq!(logical.data(), &[0, 1, 1, 0, 1, 0]);
    let Array::Logical(empty) = &typed["empty_logical"] else {
        panic!("empty logical type lost")
    };
    assert_eq!((empty.rows(), empty.cols()), (0, 3));
    {
        let name = "three_d";
        assert!(
            catalog
                .variables()
                .find(|v| v.name == name)
                .unwrap()
                .unavailable
                .is_some()
        );
        assert!(files::load_selected_workspace(&catalog, &names(&[name]), &cancel).is_err());
    }
    let text = catalog.variables().find(|v| v.name == "text_data").unwrap();
    assert!(text.unavailable.is_none());
    assert_eq!(text.dimensions, [1, 34]);
    assert!(
        files::load_selected_workspace(&catalog, &names(&["text_data"]), &cancel)
            .unwrap_err()
            .contains("typed")
    );
    let typed_text =
        files::load_selected_arrays(&catalog, &names(&["text_data"]), &cancel).unwrap();
    let Array::Character(text) = &typed_text["text_data"] else {
        panic!("Character class lost")
    };
    assert_eq!((text.rows(), text.cols()), (1, 34));
    for name in ["too_precise_signed", "too_precise_unsigned"] {
        assert!(
            files::load_selected_workspace(&catalog, &names(&[name]), &cancel)
                .unwrap_err()
                .contains("precision")
        );
    }
}

#[test]
fn v73_full_load_and_selective_load_preserve_orientation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("clean.mat");
    create(&path, |file| {
        let ds = file.new_dataset::<f64>().shape((3, 2)).create("x").unwrap();
        class(&ds, "double");
        ds.write_raw(&[1., 4., 2., 5., 3., 6.]).unwrap();
    });
    let actual = files::load_workspace(&path).unwrap();
    assert_eq!((actual["x"].rows(), actual["x"].cols()), (2, 3));
    assert_eq!(actual["x"].data(), &[1., 4., 2., 5., 3., 6.]);
    let empty = dir.path().join("empty.mat");
    create(&empty, |_| {});
    assert!(files::load_workspace(&empty).unwrap().is_empty());
}

#[test]
fn v73_rejects_external_storage_links_large_chunks_and_class_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("unsafe_storage.mat");
    create(&path, |file| {
        file.link_external("absent-data.h5", "/payload", "external_link")
            .unwrap();
        file.link_soft("/absent", "soft_link").unwrap();
        let ds = file
            .new_dataset::<u8>()
            .shape((2, 3))
            .create("mismatch")
            .unwrap();
        class(&ds, "double");
        let ds = file
            .new_dataset::<f64>()
            .shape((2048, 1024))
            .chunk((2048, 1024))
            .create("large_chunk")
            .unwrap();
        class(&ds, "double");
        let ds = file
            .new_dataset::<f64>()
            .shape((2, 3))
            .external("absent.raw", 0, 48)
            .create("external_data")
            .unwrap();
        class(&ds, "double");
    });
    let cancel = AtomicBool::new(false);
    let catalog = files::inspect_workspace(&path, &cancel).unwrap();
    assert_eq!(catalog.variables().len(), 5);
    for info in catalog.variables() {
        assert!(info.unavailable.is_some(), "{}", info.name);
        assert!(files::load_selected_workspace(&catalog, &names(&[&info.name]), &cancel).is_err());
    }
    assert!(!dir.path().join("absent.raw").exists());
}

#[test]
fn v73_bounds_workspace_and_rejects_stale_selection_and_cancellation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sparse_fill.mat");
    create(&path, |file| {
        for name in ["a", "b", "c"] {
            let ds = file
                .new_dataset::<f64>()
                .shape((128, 100_000))
                .chunk((1, 1024))
                .create(name)
                .unwrap();
            class(&ds, "double");
        }
    });
    let cancel = AtomicBool::new(false);
    let catalog = files::inspect_workspace(&path, &cancel).unwrap();
    assert!(catalog.file_bytes() < 65536);
    assert!(
        files::load_selected_workspace(&catalog, &names(&["a", "b", "c"]), &cancel)
            .unwrap_err()
            .contains("256 MiB")
    );
    assert!(files::load_selected_workspace(&catalog, &names(&["missing"]), &cancel).is_err());
    assert!(files::load_selected_workspace(&catalog, &BTreeSet::new(), &cancel).is_err());
    let stopped = AtomicBool::new(true);
    assert!(files::inspect_workspace(&path, &stopped).is_err());
    assert!(files::load_selected_workspace(&catalog, &names(&["a"]), &stopped).is_err());
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(&[0])
        .unwrap();
    assert!(
        files::load_selected_workspace(&catalog, &names(&["a"]), &cancel)
            .unwrap_err()
            .contains("changed")
    );
}

#[test]
fn v73_rejects_invalid_logical_payload_and_unbounded_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("logical.mat");
    create(&path, |file| {
        let ds = file
            .new_dataset::<u8>()
            .shape((2, 1))
            .create("bad")
            .unwrap();
        class(&ds, "logical");
        ds.write_raw(&[0, 2]).unwrap();
    });
    let cancel = AtomicBool::new(false);
    let catalog = files::inspect_workspace(&path, &cancel).unwrap();
    assert!(
        files::load_selected_arrays(&catalog, &names(&["bad"]), &cancel)
            .unwrap_err()
            .contains("logical")
    );
    let path = dir.path().join("large_attribute.mat");
    create(&path, |file| {
        let ds = file
            .new_dataset::<f64>()
            .shape((1, 1))
            .create("bad")
            .unwrap();
        ds.new_attr::<hdf5::types::FixedAscii<256>>()
            .create("MATLAB_class")
            .unwrap()
            .write_scalar(&hdf5::types::FixedAscii::<256>::from_ascii("double").unwrap())
            .unwrap();
    });
    assert!(
        files::inspect_workspace(&path, &cancel)
            .unwrap_err()
            .contains("MATLAB_class")
    );
}

#[test]
fn v73_rejects_non_ascii_class_without_constructing_invalid_text() {
    let error = files::inspect_workspace(
        Path::new("tests/fixtures/mat/invalid-class-v73.mat"),
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(error.contains("Invalid ASCII"), "{error}");
}

#[test]
fn v73_unicode_filename_roundtrip_and_virtual_storage_rejection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace_\u{05d0}_\u{6e2c}.mat");
    create(&path, |file| {
        let ds = file.new_dataset::<f64>().shape((1, 2)).create("x").unwrap();
        class(&ds, "double");
        ds.write_raw(&[2., 3.]).unwrap();
        let ds = file
            .new_dataset::<f64>()
            .shape((2, 3))
            .virtual_map("absent.h5", "source", (2, 3), (.., ..), (2, 3), (.., ..))
            .create("virtual_data")
            .unwrap();
        class(&ds, "double");
    });
    let cancel = AtomicBool::new(false);
    let catalog = files::inspect_workspace(&path, &cancel).unwrap();
    assert_eq!(
        files::load_selected_workspace(&catalog, &names(&["x"]), &cancel).unwrap()["x"].data(),
        &[2., 3.]
    );
    assert!(
        files::load_selected_workspace(&catalog, &names(&["virtual_data"]), &cancel)
            .unwrap_err()
            .contains("virtual")
    );
    assert!(!dir.path().join("absent.h5").exists());
}

#[test]
fn v73_inspection_skips_payload_but_selected_checksum_is_validated() {
    let cancel = AtomicBool::new(false);
    let catalog = files::inspect_workspace(
        Path::new("tests/fixtures/mat/bad-checksum-v73.mat"),
        &cancel,
    )
    .unwrap();
    assert_eq!(catalog.variables().len(), 2);
    assert!(catalog.variables().all(|info| info.unavailable.is_none()));
    assert_eq!(
        files::load_selected_workspace(&catalog, &names(&["good"]), &cancel).unwrap()["good"]
            .data(),
        &[12.5]
    );
    let error = files::load_selected_workspace(&catalog, &names(&["bad"]), &cancel).unwrap_err();
    assert!(
        error.contains("bad:") && error.contains("MAT v7.3"),
        "{error}"
    );
}

#[test]
fn v73_direct_reads_cover_partial_chunks_and_contiguous_tile_edges() {
    let dir = tempfile::tempdir().unwrap();
    for (name, columns, rows, chunked) in [
        ("chunked", 5, 210_013, true),
        ("contiguous", 2, 1_048_581, false),
    ] {
        let path = dir.path().join(format!("{name}.mat"));
        let expected: Vec<_> = (0..columns * rows).map(|i| i as f64 * 0.25 - 7.).collect();
        create(&path, |file| {
            let builder = file.new_dataset::<f64>().shape((columns, rows));
            let ds = if chunked {
                builder.chunk((2, 1024)).deflate(1).create("x")
            } else {
                builder.create("x")
            }
            .unwrap();
            class(&ds, "double");
            ds.write_raw(&expected).unwrap();
        });
        let actual = files::load_workspace(&path).unwrap();
        assert_eq!((actual["x"].rows(), actual["x"].cols()), (rows, columns));
        assert_eq!(actual["x"].data(), expected);
    }
}
