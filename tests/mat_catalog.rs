use matrust::{Matrix, files};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    sync::atomic::AtomicBool,
};
fn names(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|s| s.to_string()).collect()
}
fn plain(source: &[u8]) -> Vec<u8> {
    let mut result = source[..128].to_vec();
    let mut offset = 128;
    while offset < source.len() {
        let kind = u32::from_le_bytes(source[offset..offset + 4].try_into().unwrap());
        let len = u32::from_le_bytes(source[offset + 4..offset + 8].try_into().unwrap()) as usize;
        if kind == 15 {
            flate2::read::ZlibDecoder::new(&source[offset + 8..offset + 8 + len])
                .read_to_end(&mut result)
                .unwrap();
            offset += 8 + len;
        } else {
            let end = offset + 8 + len.next_multiple_of(8);
            result.extend(&source[offset..end]);
            offset = end;
        }
    }
    result
}

#[test]
fn matlab_numeric_types_selected_among_unsupported_arrays_match_reference() {
    let cancel = AtomicBool::new(false);
    let expected = files::load_workspace(std::path::Path::new(
        "tests/fixtures/mat/mat-import-oracle.mat",
    ))
    .unwrap();
    for version in [6, 7] {
        let catalog = files::inspect_workspace(
            std::path::Path::new(&format!("tests/fixtures/mat/mat-import-v{version}.mat")),
            &cancel,
        )
        .unwrap();
        assert_eq!(catalog.variables().len(), 21);
        for name in ["struct_data", "sparse_data", "three_d"] {
            let var = catalog.variables().find(|v| v.name == name).unwrap();
            assert!(var.unavailable.is_some(), "{name}");
            assert!(files::load_selected_workspace(&catalog, &names(&[name]), &cancel).is_err());
        }
        for name in ["cell_data", "text_data"] {
            assert!(
                catalog
                    .variables()
                    .find(|v| v.name == name)
                    .unwrap()
                    .unavailable
                    .is_none()
            );
            assert!(files::load_selected_workspace(&catalog, &names(&[name]), &cancel).is_err());
        }
        let complex = catalog
            .variables()
            .find(|v| v.name == "complex_data")
            .unwrap();
        assert!(complex.complex && complex.unavailable.is_none());
        assert_eq!(complex.storage_bytes(), Some(32));
        assert!(
            files::load_selected_workspace(&catalog, &names(&["complex_data"]), &cancel).is_err()
        );
        let typed =
            files::load_selected_arrays(&catalog, &names(&["complex_data"]), &cancel).unwrap();
        let z = typed["complex_data"].as_complex().unwrap();
        assert_eq!((z.rows(), z.cols()), (1, 2));
        assert_eq!(
            z.data(),
            &[
                matrust::Complex64::new(1., 3.),
                matrust::Complex64::new(2., 4.)
            ]
        );
        let selected = expected.keys().cloned().collect();
        let actual = files::load_selected_workspace(&catalog, &selected, &cancel).unwrap();
        for (name, expected) in &expected {
            let actual = &actual[name];
            assert_eq!(
                (actual.rows(), actual.cols()),
                (expected.rows(), expected.cols())
            );
            for (&a, &b) in actual.data().iter().zip(expected.data()) {
                assert!(
                    a.to_bits() == b.to_bits() || a.is_nan() && b.is_nan(),
                    "{name}: {a} != {b}"
                );
            }
        }
        assert!(
            files::load_selected_workspace(&catalog, &names(&["too_precise"]), &cancel)
                .unwrap_err()
                .contains("precision")
        );
        assert_eq!(files::load_selected_workspace(&catalog,&names(&["compact"]),&cancel).unwrap()["compact"].data(),&[7.]);
    }
}

#[test]
fn selection_rejects_stale_catalogs_missing_names_and_cancellation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sample.mat");
    let vars = BTreeMap::from([("A".into(), Matrix::filled(8, 2, 3.).unwrap())]);
    files::save_workspace(&path, &vars).unwrap();
    let cancel = AtomicBool::new(false);
    let catalog = files::inspect_workspace(&path, &cancel).unwrap();
    assert!(files::load_selected_workspace(&catalog, &BTreeSet::new(), &cancel).is_err());
    assert!(files::load_selected_workspace(&catalog, &names(&["missing"]), &cancel).is_err());
    let stopped = AtomicBool::new(true);
    assert!(files::inspect_workspace(&path, &stopped).is_err());
    assert!(files::load_selected_workspace(&catalog, &names(&["A"]), &stopped).is_err());
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(&[0])
        .unwrap();
    assert!(
        files::load_selected_workspace(&catalog, &names(&["A"]), &cancel)
            .unwrap_err()
            .contains("changed")
    );
    assert!(files::inspect_workspace(&path, &cancel).is_err());
}

#[test]
fn unselected_compressed_payload_is_skipped_and_selected_checksum_is_checked() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mixed.mat");
    files::save_workspace(
        &path,
        &BTreeMap::from([
            ("A".into(), Matrix::filled(100000, 1, 0.).unwrap()),
            ("B".into(), Matrix::scalar(12.)),
        ]),
    )
    .unwrap();
    let source = plain(&std::fs::read(&path).unwrap());
    assert_eq!(&source[128..132], &14u32.to_le_bytes());
    let len = u32::from_le_bytes(source[132..136].try_into().unwrap()) as usize;
    let end = 136 + len.next_multiple_of(8);
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&source[128..end]).unwrap();
    let mut compressed = encoder.finish().unwrap();
    let last = compressed.len() - 1;
    compressed[last] ^= 1;
    let mut bytes = source[..128].to_vec();
    bytes.extend(15u32.to_le_bytes());
    bytes.extend((compressed.len() as u32).to_le_bytes());
    bytes.extend(compressed);
    bytes.extend(&source[end..]);
    std::fs::write(&path, &bytes).unwrap();
    let cancel = AtomicBool::new(false);
    let catalog = files::inspect_workspace(&path, &cancel).unwrap();
    assert_eq!(
        files::load_selected_workspace(&catalog, &names(&["B"]), &cancel).unwrap()["B"].data(),
        &[12.]
    );
    assert!(files::load_selected_workspace(&catalog, &names(&["A"]), &cancel).is_err());
    // A trailing byte inside a selected compressed element is also rejected.
    bytes[136 + last] ^= 1;
    bytes.insert(136 + last + 1, 0);
    let expanded = (last + 2) as u32;
    bytes[132..136].copy_from_slice(&expanded.to_le_bytes());
    std::fs::write(&path, &bytes).unwrap();
    let catalog = files::inspect_workspace(&path, &cancel).unwrap();
    assert!(files::load_selected_workspace(&catalog, &names(&["A"]), &cancel).is_err());
}

#[test]
fn malformed_metadata_extents_and_duplicate_names_are_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sample.mat");
    files::save_workspace(&path, &BTreeMap::from([("A".into(), Matrix::scalar(1.))])).unwrap();
    let source = plain(&std::fs::read(&path).unwrap());
    let cancel = AtomicBool::new(false);
    for end in [0, 127, 129, 135, 140, source.len() - 1] {
        std::fs::write(&path, &source[..end]).unwrap();
        assert!(files::inspect_workspace(&path, &cancel).is_err());
    }
    let mut duplicate = source.clone();
    duplicate.extend(&source[128..]);
    std::fs::write(&path, duplicate).unwrap();
    assert!(
        files::inspect_workspace(&path, &cancel)
            .unwrap_err()
            .contains("Duplicate")
    );
    for (offset, value) in [
        (132, u32::MAX),
        (136, (5 << 16) | 6),
        (156, 4096),
        (160, u32::MAX),
    ] {
        let mut bytes = source.clone();
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        assert!(
            files::inspect_workspace(&path, &cancel).is_err(),
            "{offset}"
        );
    }
    let mut oversized = source;
    oversized[160..164].copy_from_slice(&20_000_000u32.to_le_bytes());
    std::fs::write(&path, oversized).unwrap();
    let catalog = files::inspect_workspace(&path, &cancel).unwrap();
    assert!(
        catalog
            .variables()
            .next()
            .unwrap()
            .unavailable
            .as_ref()
            .unwrap()
            .contains("128 MiB")
    );
    assert!(files::load_selected_workspace(&catalog, &names(&["A"]), &cancel).is_err());
}
