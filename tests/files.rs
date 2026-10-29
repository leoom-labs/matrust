use matrust::{Matrix, document::Document, files, plot::Figure};
use std::collections::BTreeMap;

#[test]
fn mat_workspace_roundtrip_preserves_shapes_and_special_values() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.mat");
    let vars = BTreeMap::from([
        (
            "A".into(),
            Matrix::from_column_slice(2, 2, &[1., f64::NAN, f64::INFINITY, f64::NEG_INFINITY])
                .unwrap(),
        ),
        ("empty".into(), Matrix::filled(0, 3, 0.).unwrap()),
    ]);
    files::save_workspace(&path, &vars).unwrap();
    let loaded = files::load_workspace(&path).unwrap();
    assert_eq!((loaded["A"].rows(), loaded["A"].cols()), (2, 2));
    assert_eq!((loaded["empty"].rows(), loaded["empty"].cols()), (0, 3));
    for (a, b) in vars["A"].data().iter().zip(loaded["A"].data()) {
        assert_eq!(a.to_bits(), b.to_bits());
    }
}
#[test]
fn csv_roundtrip_and_ragged_input() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.csv");
    let m = Matrix::from_column_slice(2, 3, &[1.12345678901234, 2., 3., 4., 5., 6.]).unwrap();
    files::write_csv(&path, &m).unwrap();
    assert_eq!(files::read_csv(&path).unwrap().data(), m.data());
    std::fs::write(&path, "1,2\n3\n").unwrap();
    assert!(files::read_csv(&path).is_err());
}
#[test]
fn streaming_csv_handles_bom_crlf_blanks_and_invalid_fields() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("stream.csv");
    std::fs::write(&path, "\u{feff}1;2\r\n\r\n3\t4\r\n5,6").unwrap();
    let matrix = files::read_csv(&path).unwrap();
    assert_eq!((matrix.rows(), matrix.cols()), (3, 2));
    assert_eq!(matrix.data(), &[1., 3., 5., 2., 4., 6.]);
    for bad in [b"1,\n".as_slice(), b"1,2\n3,no", b"1,2\n\xff"] {
        std::fs::write(&path, bad).unwrap();
        assert!(files::read_csv(&path).is_err());
    }
    std::fs::write(&path, "12345").unwrap();
    assert!(files::read_bounded(&path, 4).is_err());
    assert_eq!(files::read_bounded(&path, 5).unwrap(), b"12345");
}

#[test]
fn mixed_compressed_mat_arrays_preserve_order_and_validation() {
    use std::io::{Read, Write};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mixed.mat");
    let vars: BTreeMap<_, _> = ["a", "b", "c"]
        .into_iter()
        .enumerate()
        .map(|(i, name)| (name.to_owned(), Matrix::filled(2, 3, i as f64).unwrap()))
        .collect();
    files::save_workspace(&path, &vars).unwrap();
    let original = std::fs::read(&path).unwrap();
    let mut mixed = original[..128].to_vec();
    let mut pos = 128;
    for index in 0..3 {
        let kind = u32::from_le_bytes(original[pos..pos + 4].try_into().unwrap());
        let len = u32::from_le_bytes(original[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let end = pos
            + 8
            + if kind == 15 {
                len
            } else {
                len.next_multiple_of(8)
            };
        let mut matrix = Vec::new();
        if kind == 15 {
            flate2::read::ZlibDecoder::new(&original[pos + 8..end])
                .read_to_end(&mut matrix)
                .unwrap();
        } else {
            matrix.extend_from_slice(&original[pos..end]);
        }
        if index == 1 {
            let mut encoder =
                flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
            encoder.write_all(&matrix).unwrap();
            let compressed = encoder.finish().unwrap();
            mixed.extend_from_slice(&15u32.to_le_bytes());
            mixed.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
            mixed.extend(compressed);
        } else {
            mixed.extend_from_slice(&matrix);
        }
        pos = end;
    }
    std::fs::write(&path, &mixed).unwrap();
    let loaded = files::load_workspace(&path).unwrap();
    for (name, expected) in vars {
        assert_eq!(loaded[&name].data(), expected.data());
    }
    mixed.pop();
    std::fs::write(&path, mixed).unwrap();
    assert!(files::load_workspace(&path).is_err());
}

fn plain_double_fixture() -> Vec<u8> {
    let mut bytes = vec![0; 128];
    bytes[124..128].copy_from_slice(&[0, 1, b'I', b'M']);
    let mut matrix = Vec::new();
    for word in [6u32, 8, 6, 0, 5, 8, 3, 1, 0x00010001, b'x' as u32, 9, 24] {
        matrix.extend_from_slice(&word.to_le_bytes());
    }
    for value in [-0_f64, f64::from_bits(0x7ff8000000000042), f64::INFINITY] {
        matrix.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(&14u32.to_le_bytes());
    bytes.extend_from_slice(&(matrix.len() as u32).to_le_bytes());
    bytes.extend(matrix);
    bytes
}

#[test]
fn streamed_plain_mat_preserves_bits_and_rejects_truncation_and_bad_lengths() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plain.mat");
    let valid = plain_double_fixture();
    std::fs::write(&path, &valid).unwrap();
    let loaded = files::load_workspace(&path).unwrap();
    assert_eq!((loaded["x"].rows(), loaded["x"].cols()), (3, 1));
    assert_eq!(
        loaded["x"]
            .data()
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        vec![
            (-0_f64).to_bits(),
            0x7ff8000000000042,
            f64::INFINITY.to_bits()
        ]
    );
    for end in 129..valid.len() {
        std::fs::write(&path, &valid[..end]).unwrap();
        assert!(
            files::load_workspace(&path).is_err(),
            "accepted truncated file length {end}"
        );
    }
    for (offset, value) in [
        (132, 71u32),
        (160, u32::MAX),
        (160, 16777217),
        (180, 16),
        (172, b'!' as u32),
    ] {
        let mut bad = valid.clone();
        bad[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        std::fs::write(&path, bad).unwrap();
        assert!(
            files::load_workspace(&path).is_err(),
            "accepted invalid word at {offset}"
        );
    }
    let mut duplicate = valid.clone();
    duplicate.extend_from_slice(&valid[128..]);
    std::fs::write(&path, duplicate).unwrap();
    assert!(
        files::load_workspace(&path)
            .unwrap_err()
            .contains("Duplicate")
    );
}

#[test]
fn non_double_mat_storage_uses_checked_general_conversion() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("integers.mat");
    for (values, expected) in [
        (mat5::Values::I16(vec![-3, 0, 7]), Some(vec![-3., 0., 7.])),
        (
            mat5::Values::F32(vec![-1.5, 0., 3.25]),
            Some(vec![-1.5, 0., 3.25]),
        ),
        (
            mat5::Values::Bool(vec![true, false, true]),
            Some(vec![1., 0., 1.]),
        ),
        (mat5::Values::U64(vec![0, 1, 9_007_199_254_740_993]), None),
        (mat5::Values::I64(vec![0, 1, i64::MIN]), None),
    ] {
        let file = mat5::MatFile {
            header_text: "fixture".into(),
            arrays: vec![mat5::NamedArray {
                name: "v".into(),
                matrix: mat5::Matrix::Dense(mat5::DenseMatrix {
                    rows: 3,
                    cols: 1,
                    values,
                }),
            }],
        };
        std::fs::write(&path, file.write_bytes().unwrap()).unwrap();
        let loaded = files::load_workspace(&path);
        if let Some(expected) = expected {
            assert_eq!(loaded.unwrap()["v"].data(), expected);
        } else {
            assert!(loaded.unwrap_err().contains("exact f64 precision"));
        }
    }
}

#[test]
fn streamed_compressed_mat_checks_checksum_complete_stream_and_extra_data() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("compressed.mat");
    let plain = plain_double_fixture();
    let compress = |payload: &[u8]| {
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(payload).unwrap();
        encoder.finish().unwrap()
    };
    let wrap = |compressed: &[u8]| {
        let mut file = plain[..128].to_vec();
        file.extend_from_slice(&15u32.to_le_bytes());
        file.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
        file.extend_from_slice(compressed);
        file
    };
    let compressed = compress(&plain[128..]);
    std::fs::write(&path, wrap(&compressed)).unwrap();
    assert_eq!(
        files::load_workspace(&path).unwrap()["x"].data()[1].to_bits(),
        0x7ff8000000000042
    );
    for cut in 1..=8 {
        std::fs::write(&path, wrap(&compressed[..compressed.len() - cut])).unwrap();
        assert!(
            files::load_workspace(&path).is_err(),
            "accepted truncated stream, {cut} bytes missing"
        );
    }
    let mut corrupt = compressed.clone();
    *corrupt.last_mut().unwrap() ^= 1;
    std::fs::write(&path, wrap(&corrupt)).unwrap();
    assert!(files::load_workspace(&path).is_err());
    let mut extra = plain[128..].to_vec();
    extra.push(0);
    std::fs::write(&path, wrap(&compress(&extra))).unwrap();
    assert!(files::load_workspace(&path).is_err());
    let mut extra = compressed;
    extra.push(0);
    std::fs::write(&path, wrap(&extra)).unwrap();
    assert!(files::load_workspace(&path).is_err());
}
#[test]
fn figure_roundtrip_preserves_exact_samples_and_labels() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plot.mfig");
    let x = Matrix::from_column_slice(1, 3, &[0., 1., 2.]).unwrap();
    let y = Matrix::from_column_slice(1, 3, &[f64::NAN, 1e-200, f64::INFINITY]).unwrap();
    let mut f = Figure::new(&x, &y).unwrap();
    f.title = "שלום & data <test>".into();
    f.xlabel = "seconds".into();
    files::save_figure(&path, &f).unwrap();
    let loaded = files::load_figure(&path).unwrap();
    assert_eq!(loaded.title, f.title);
    assert_eq!(loaded.xlabel, f.xlabel);
    for (a, b) in f.series[0]
        .points
        .iter()
        .flatten()
        .zip(loaded.series[0].points.iter().flatten())
    {
        assert_eq!(a.to_bits(), b.to_bits());
    }
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.pop();
    std::fs::write(&path, bytes).unwrap();
    assert!(files::load_figure(&path).is_err());
}
#[test]
fn document_failure_keeps_dirty_buffer_and_saved_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("script.m");
    std::fs::write(&path, "A=1;").unwrap();
    let mut doc = Document::open(&path).unwrap();
    doc.text = "A=2;".into();
    doc.changed();
    assert!(doc.dirty);
    assert!(doc.save(&dir.path().join("missing/file.m")).is_err());
    assert!(doc.dirty);
    assert!(matrust::document::same_path(
        doc.path.as_ref().unwrap(),
        &path
    ));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "A=1;");
    doc.save(&path).unwrap();
    assert!(!doc.dirty);
    assert_eq!(std::fs::read_to_string(path).unwrap(), "A=2;");
}
#[test]
fn malformed_mat_and_figure_inputs_fail_without_panics() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("input.mat");
    for n in [0, 12, 127, 128, 256] {
        std::fs::write(&path, vec![255; n]).unwrap();
        assert!(files::load_workspace(&path).is_err());
        assert!(files::load_figure(&path).is_err());
    }
    // Compact tags may have at most four bytes. Reject before dependency parsing.
    let mut bytes = vec![0; 128];
    bytes[126] = b'I';
    bytes[127] = b'M';
    bytes.extend_from_slice(&0x00050001u32.to_le_bytes());
    bytes.extend_from_slice(&[0; 8]);
    std::fs::write(&path, bytes).unwrap();
    assert!(files::load_workspace(&path).is_err());
}
#[cfg(feature = "plot-export")]
#[test]
fn exported_formats_are_decodable_and_have_visible_plot_geometry() {
    let dir = tempfile::tempdir().unwrap();
    let x = Matrix::from_column_slice(1, 3, &[0., 1., 2.]).unwrap();
    let y = Matrix::from_column_slice(1, 3, &[0., 1., 0.]).unwrap();
    let f = Figure::new(&x, &y).unwrap();
    for ext in ["png", "jpg", "bmp", "tiff"] {
        let p = dir.path().join(format!("plot.{ext}"));
        matrust::export::export_figure(&f, &p, 960, 560).unwrap();
        let img = image::open(p).unwrap().to_rgb8();
        assert_eq!(img.dimensions(), (960, 560));
        assert!(
            img.pixels()
                .filter(|p| p.0[2] > p.0[0].saturating_add(40))
                .count()
                > 500
        );
    }
    for (ext, header) in [("svg", b"<svg".as_slice()), ("pdf", b"%PDF".as_slice())] {
        let p = dir.path().join(format!("plot.{ext}"));
        matrust::export::export_figure(&f, &p, 960, 560).unwrap();
        assert!(std::fs::read(p).unwrap().starts_with(header));
    }
    assert!(
        matrust::export::export_figure(&f, &dir.path().join("bad.png"), 100000, 100000).is_err()
    );
}
