use matrust::files;

#[test]
fn csv_batches_preserve_shape_bits_and_physical_error_lines() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("batched.csv");
    let row = "\u{2003}-0;1.2345678901234567;1e-300;NaN;inf;-inf;5e-324;1.7976931348623157e308\r\n";
    let rows = 70000;
    let mut text = String::from("\u{feff}\r\n");
    for i in 0..rows {
        text.push_str(row);
        if i % 17 == 0 {
            text.push_str(" \t \r\n");
        }
    }
    text.truncate(text.len() - 2); // Last record need not end in a newline.
    std::fs::write(&path, &text).unwrap();
    let expected = [
        -0.,
        1.2345678901234567,
        1e-300,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        5e-324,
        f64::MAX,
    ];
    for threads in [1, 8] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                let m = files::read_csv(&path).unwrap();
                assert_eq!((m.rows(), m.cols()), (rows, expected.len()));
                for (column, value) in expected.iter().enumerate() {
                    for &actual in &m.data()[column * rows..(column + 1) * rows] {
                        if value.is_nan() {
                            assert!(actual.is_nan());
                        } else {
                            assert_eq!(actual.to_bits(), value.to_bits());
                        }
                    }
                }
            });
    }
    let line = text.bytes().filter(|&b| b == b'\n').count() + 2;
    for (bad, error) in [
        ("1,2", "Inconsistent column count"),
        ("x,2,3,4,5,6,7,8", "Invalid numeric field"),
    ] {
        std::fs::write(&path, format!("{text}\n{bad}\n")).unwrap();
        for threads in [1, 8] {
            let e = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| files::read_csv(&path))
                .unwrap_err();
            assert_eq!(e, format!("{error} on line {line}"));
        }
    }
}

#[test]
fn csv_wide_records_cross_batch_size_and_single_column_needs_no_transpose() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wide.csv");
    let text = format!("{}-0", "1.2345678901234567,".repeat(150000));
    std::fs::write(&path, text).unwrap();
    rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .build()
        .unwrap()
        .install(|| {
            let m = files::read_csv(&path).unwrap();
            assert_eq!((m.rows(), m.cols()), (1, 150001));
            assert!(m.data()[..150000].iter().all(|&v| v == 1.2345678901234567));
            assert_eq!(m.data()[150000].to_bits(), (-0.0f64).to_bits());
        });
    std::fs::write(&path, "1\n2\n3").unwrap();
    let m = files::read_csv(&path).unwrap();
    assert_eq!((m.rows(), m.cols()), (3, 1));
    assert_eq!(m.data(), &[1., 2., 3.]);
}
