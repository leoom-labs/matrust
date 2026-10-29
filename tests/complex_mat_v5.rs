use matrust::{Array, Complex64, ComplexMatrix, Engine, LogicalMatrix, Matrix, files};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    sync::atomic::{AtomicBool, Ordering},
};

fn names(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|v| (*v).into()).collect()
}
fn element(kind: u32, data: &[u8]) -> Vec<u8> {
    let mut out = kind.to_le_bytes().to_vec();
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(data);
    out.resize(out.len() + (8 - data.len() % 8) % 8, 0);
    out
}
fn component(kind: u32, data: &[u8], compact: bool) -> Vec<u8> {
    if !compact {
        return element(kind, data);
    }
    assert!(!data.is_empty() && data.len() <= 4);
    let mut out = (kind | ((data.len() as u32) << 16)).to_le_bytes().to_vec();
    out.extend(data);
    out.resize(8, 0);
    out
}
fn array(name: &str, flags: u32, rows: i32, cols: i32, payload: &[u8]) -> Vec<u8> {
    let mut body = element(6, &[flags.to_le_bytes(), [0; 4]].concat());
    body.extend(element(
        5,
        &[rows.to_le_bytes(), cols.to_le_bytes()].concat(),
    ));
    body.extend(element(1, name.as_bytes()));
    body.extend(payload);
    element(14, &body)
}
fn file(elements: &[Vec<u8>], compressed: bool) -> Vec<u8> {
    let mut bytes = vec![b' '; 128];
    let text = b"MATLAB 5.0 MAT-file,";
    bytes[..text.len()].copy_from_slice(text);
    bytes[116..124].fill(0);
    bytes[124..128].copy_from_slice(&[0, 1, b'I', b'M']);
    for value in elements {
        if compressed {
            let mut encoder =
                flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(value).unwrap();
            let encoded = encoder.finish().unwrap();
            bytes.extend(15u32.to_le_bytes());
            bytes.extend((encoded.len() as u32).to_le_bytes());
            bytes.extend(encoded);
        } else {
            bytes.extend(value);
        }
    }
    bytes
}
fn bits(a: &Array) -> Vec<(u64, u64)> {
    a.as_complex()
        .unwrap()
        .data()
        .iter()
        .map(|z| (z.re.to_bits(), z.im.to_bits()))
        .collect()
}

#[test]
fn typed_complex_mat_roundtrip_preserves_bits_empty_shapes_and_real_logical_controls() {
    let cancel = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    let raw = [
        Complex64::new(-0., 0.),
        Complex64::new(1., -0.),
        Complex64::new(f64::INFINITY, f64::NEG_INFINITY),
        Complex64::new(f64::from_bits(0x7ff8_0000_0000_0042), f64::from_bits(1)),
    ];
    let mut values: BTreeMap<String, Array> = BTreeMap::from([
        (
            "Z".into(),
            ComplexMatrix::from_column_slice(2, 2, &raw, &cancel)
                .unwrap()
                .into(),
        ),
        (
            "ZeroImag".into(),
            ComplexMatrix::filled(1, 3, Complex64::new(7., 0.), &cancel)
                .unwrap()
                .into(),
        ),
        (
            "R".into(),
            Matrix::from_column_vec(2, 2, vec![1., 2., 3., 4.])
                .unwrap()
                .into(),
        ),
        (
            "L".into(),
            LogicalMatrix::from_column_vec(2, 2, vec![0, 1, 1, 0], &cancel)
                .unwrap()
                .into(),
        ),
    ]);
    for (name, rows, cols) in [("E00", 0, 0), ("E03", 0, 3), ("E40", 4, 0)] {
        values.insert(
            name.into(),
            ComplexMatrix::filled(rows, cols, Complex64::new(0., 0.), &cancel)
                .unwrap()
                .into(),
        );
    }
    let before = values.clone();
    for workers in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        for compressed in [false, true] {
            let path = dir.path().join(format!("{workers}-{compressed}.mat"));
            pool.install(|| {
                if compressed {
                    files::save_array_workspace(&path, &values, &cancel)
                } else {
                    files::save_array_workspace_uncompressed(&path, &values, &cancel)
                }
            })
            .unwrap();
            let catalog = files::inspect_workspace(&path, &cancel).unwrap();
            let loaded = pool
                .install(|| {
                    files::load_selected_arrays(
                        &catalog,
                        &values.keys().cloned().collect(),
                        &cancel,
                    )
                })
                .unwrap();
            for (name, original) in &values {
                assert!(original.same_snapshot(&before[name]));
                let actual = &loaded[name];
                assert_eq!(
                    (actual.rows(), actual.cols()),
                    (original.rows(), original.cols())
                );
                let info = catalog.variables().find(|v| &v.name == name).unwrap();
                assert_eq!(info.complex, original.is_complex());
                assert_eq!(info.storage_bytes(), Some(original.bytes() as u64));
                if original.is_complex() {
                    assert_eq!(bits(actual), bits(original));
                }
            }
            assert_eq!(loaded["R"].as_numeric().unwrap().data(), &[1., 2., 3., 4.]);
            let Array::Logical(logical) = &loaded["L"] else {
                panic!("lost logical class")
            };
            assert_eq!(logical.data(), &[0, 1, 1, 0]);
            assert!(
                files::load_selected_workspace(&catalog, &names(&["E00"]), &cancel)
                    .unwrap_err()
                    .contains("typed")
            );
        }
    }
}

#[test]
fn independent_wire_accepts_compact_and_mixed_component_storage() {
    let cancel = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    // Hand-built normative tags, independent of the MatRust writer.
    let real = component(1, &[255, 2], true);
    let imaginary = component(
        3,
        &[3i16.to_le_bytes(), (-4i16).to_le_bytes()].concat(),
        true,
    );
    let payload = [real, imaginary].concat();
    for compressed in [false, true] {
        let path = dir.path().join("packed.mat");
        std::fs::write(
            &path,
            file(&[array("Z", 0x806, 1, 2, &payload)], compressed),
        )
        .unwrap();
        let catalog = files::inspect_workspace(&path, &cancel).unwrap();
        let loaded = files::load_selected_arrays(&catalog, &names(&["Z"]), &cancel).unwrap();
        assert_eq!(
            loaded["Z"].as_complex().unwrap().data(),
            &[Complex64::new(-1., 3.), Complex64::new(2., -4.)]
        );
    }
}

#[test]
fn malformed_complex_payloads_fail_without_partial_workspace_or_destination_changes() {
    let cancel = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    let good = component(9, &1f64.to_le_bytes(), false);
    let invalid = vec![
        good.clone(),
        [good.clone(), component(9, &[], false)].concat(),
        [good.clone(), component(16, &[0; 8], false)].concat(),
        [good.clone(), good.clone(), good.clone()].concat(),
        [
            good.clone(),
            component(13, &(9_007_199_254_740_993u64).to_le_bytes(), false),
        ]
        .concat(),
    ];
    for compressed in [false, true] {
        for payload in &invalid {
            let path = dir.path().join("invalid.mat");
            std::fs::write(
                &path,
                file(
                    &[array("A", 6, 1, 1, &good), array("Z", 0x806, 1, 1, payload)],
                    compressed,
                ),
            )
            .unwrap();
            let catalog = files::inspect_workspace(&path, &cancel).unwrap();
            assert!(files::load_selected_arrays(&catalog, &names(&["A", "Z"]), &cancel).is_err());
            assert_eq!(
                files::load_selected_arrays(&catalog, &names(&["A"]), &cancel).unwrap()["A"]
                    .as_numeric()
                    .unwrap()
                    .data(),
                &[1.]
            );
            let mut engine = Engine::default();
            engine.execute("A=99;Z=complex(4,5);Alias=Z;").unwrap();
            let a = engine.array("A").unwrap();
            let z = engine.array("Z").unwrap();
            let quoted = path
                .to_string_lossy()
                .replace('\\', "/")
                .replace('\'', "''");
            assert!(engine.execute(&format!("load('{quoted}');")).is_err());
            assert!(engine.array("A").unwrap().same_snapshot(&a));
            assert!(engine.array("Z").unwrap().same_snapshot(&z));
        }
    }
    let path = dir.path().join("keep.mat");
    std::fs::write(&path, b"previous").unwrap();
    let z: Array = ComplexMatrix::scalar(Complex64::new(1., 2.), &cancel)
        .unwrap()
        .into();
    let invalid_names = BTreeMap::from([("A".into(), z.clone()), ("bad name".into(), z.clone())]);
    assert!(files::save_array_workspace(&path, &invalid_names, &cancel).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"previous");
    cancel.store(true, Ordering::Relaxed);
    assert!(
        files::save_array_workspace_uncompressed(
            &path,
            &BTreeMap::from([("Z".into(), z)]),
            &cancel
        )
        .unwrap_err()
        .contains("cancelled")
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"previous");
}

#[test]
fn metadata_rejects_non_double_logical_complex_nd_and_double_component_memory_overflow() {
    let cancel = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    for (flags, rows, cols) in [(0x807, 1, 1), (0xa06, 1, 1), (0x806, 8_388_609, 1)] {
        let path = dir.path().join("bounded.mat");
        std::fs::write(&path, file(&[array("Z", flags, rows, cols, &[])], false)).unwrap();
        let catalog = files::inspect_workspace(&path, &cancel).unwrap();
        assert!(catalog.variable(0).unwrap().unavailable.is_some());
        assert!(files::load_selected_arrays(&catalog, &names(&["Z"]), &cancel).is_err());
    }
    let mut body = element(6, &[0x806u32.to_le_bytes(), [0; 4]].concat());
    body.extend(element(
        5,
        &[1i32.to_le_bytes(), 1i32.to_le_bytes(), 2i32.to_le_bytes()].concat(),
    ));
    body.extend(element(1, b"Z"));
    let path = dir.path().join("three-d.mat");
    std::fs::write(&path, file(&[element(14, &body)], false)).unwrap();
    let catalog = files::inspect_workspace(&path, &cancel).unwrap();
    assert!(
        catalog
            .variable(0)
            .unwrap()
            .unavailable
            .as_ref()
            .unwrap()
            .contains("two-dimensional")
    );
    let empty_payload = [component(9, &[], false), component(9, &[], false)].concat();
    let path = dir.path().join("huge-empty.mat");
    std::fs::write(
        &path,
        file(&[array("Z", 0x806, i32::MAX, 0, &empty_payload)], false),
    )
    .unwrap();
    let catalog = files::inspect_workspace(&path, &cancel).unwrap();
    assert!(
        files::load_selected_arrays(&catalog, &names(&["Z"]), &cancel)
            .unwrap_err()
            .contains("dimension")
    );
}

#[test]
fn script_save_load_versions_preserve_typed_complex_and_selected_workspace() {
    let dir = tempfile::tempdir().unwrap();
    for options in ["'-v6'", "'-v7'", "'-v7','-nocompression'"] {
        let mut engine = Engine::default();
        engine.set_file_directory(dir.path()).unwrap();
        engine
            .execute("Z=complex([1 2],[-0 3]);Alias=Z;keep=17;")
            .unwrap();
        let original = engine.array("Z").unwrap();
        let keep = engine.array("keep").unwrap();
        engine
            .execute(&format!(
                "save('typed.mat','Z',{options});clear Z;load('typed.mat','Z');"
            ))
            .unwrap();
        assert_eq!(bits(&engine.array("Z").unwrap()), bits(&original));
        assert!(engine.array("Alias").unwrap().same_snapshot(&original));
        assert!(engine.array("keep").unwrap().same_snapshot(&keep));
    }
}

#[test]
fn large_split_components_cross_chunk_boundaries_with_identical_one_eight_worker_bits() {
    let cancel = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    let count = 16385;
    let source: Array = ComplexMatrix::from_column_vec(
        count,
        1,
        (0..count)
            .map(|i| {
                Complex64::new(
                    i as f64 / 8.,
                    if i % 7 == 0 { -0. } else { -(i as f64) / 16. },
                )
            })
            .collect(),
        &cancel,
    )
    .unwrap()
    .into();
    let wanted = bits(&source);
    for workers in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        for compressed in [false, true] {
            let path = dir.path().join("chunks.mat");
            let variables = BTreeMap::from([("Z".into(), source.clone())]);
            pool.install(|| {
                if compressed {
                    files::save_array_workspace(&path, &variables, &cancel)
                } else {
                    files::save_array_workspace_uncompressed(&path, &variables, &cancel)
                }
            })
            .unwrap();
            let catalog = files::inspect_workspace(&path, &cancel).unwrap();
            let loaded = pool
                .install(|| files::load_selected_arrays(&catalog, &names(&["Z"]), &cancel))
                .unwrap();
            assert_eq!(bits(&loaded["Z"]), wanted);
            assert!(variables["Z"].same_snapshot(&source));
        }
    }
}

#[test]
fn independent_writer_inspection_verifies_two_components_and_complex_flag() {
    let cancel = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    for compressed in [false, true] {
        let path = dir.path().join("written.mat");
        let z = ComplexMatrix::from_column_vec(
            2,
            1,
            vec![Complex64::new(1., 3.), Complex64::new(2., -0.)],
            &cancel,
        )
        .unwrap();
        let values = BTreeMap::from([("Z".into(), z.into())]);
        if compressed {
            files::save_array_workspace(&path, &values, &cancel)
        } else {
            files::save_array_workspace_uncompressed(&path, &values, &cancel)
        }
        .unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let mut plain = Vec::new();
        if compressed {
            flate2::read::ZlibDecoder::new(&bytes[136..])
                .read_to_end(&mut plain)
                .unwrap();
        } else {
            plain.extend(&bytes[128..]);
        }
        assert_eq!(u32::from_le_bytes(plain[..4].try_into().unwrap()), 14);
        assert_eq!(u32::from_le_bytes(plain[16..20].try_into().unwrap()), 0x806);
        // flags(16), dimensions(16), padded one-byte name(16), outer tag(8).
        let start = 56;
        assert_eq!(&plain[start..start + 8], &[9, 0, 0, 0, 16, 0, 0, 0]);
        assert_eq!(
            &plain[start + 8..start + 24],
            &[1f64.to_le_bytes(), 2f64.to_le_bytes()].concat()
        );
        assert_eq!(&plain[start + 24..start + 32], &[9, 0, 0, 0, 16, 0, 0, 0]);
        assert_eq!(
            &plain[start + 32..],
            &[3f64.to_le_bytes(), (-0.0f64).to_le_bytes()].concat()
        );
    }
}
