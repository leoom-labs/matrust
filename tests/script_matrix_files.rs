use matrust::{Array, Engine, Matrix, files};
use std::sync::atomic::{AtomicBool, Ordering};

fn engine(directory: &std::path::Path) -> Engine {
    let mut e = Engine::default();
    e.set_file_directory(directory).unwrap();
    e
}
fn same(a: &Matrix, b: &Matrix) {
    assert_eq!((a.rows(), a.cols()), (b.rows(), b.cols()));
    for (&a, &b) in a.data().iter().zip(b.data()) {
        if a.is_nan() {
            assert!(b.is_nan());
        } else {
            assert_eq!(a.to_bits(), b.to_bits());
        }
    }
}

#[test]
fn script_numeric_text_roundtrips_quoted_paths_logical_values_and_ans() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    let before_cwd = std::env::current_dir().unwrap();
    let input = Matrix::from_column_slice(
        2,
        3,
        &[
            1.2345678901234567,
            -0.,
            f64::INFINITY,
            -f64::INFINITY,
            f64::NAN,
            f64::MIN_POSITIVE,
        ],
    )
    .unwrap();
    e.variables.insert("A".into(), input.clone());
    e.execute("ans=77;writematrix(A,'quote'' \u{5e9}\u{6570}.CSV');R=readmatrix('quote'' \u{5e9}\u{6570}.CSV');").unwrap();
    same(&input, &e.variables["R"]);
    assert_eq!(e.variables["A"].data().as_ptr(), input.data().as_ptr());
    assert_eq!(e.variables["ans"].data(), &[77.]);
    e.execute("mask=logical([0 1;1 0]);writematrix(mask,'logical.csv');readmatrix('logical.csv');")
        .unwrap();
    assert_eq!(e.variables["ans"].data(), &[0., 1., 1., 0.]);
    assert!(e.logical_variables.contains_key("mask"));
    assert_eq!(std::env::current_dir().unwrap(), before_cwd);
}

#[test]
fn script_text_delimiters_extensions_and_explicit_filetype_are_exact() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    e.execute("A=[1 2;3 4];writematrix(A,'default');B=readmatrix('default.txt');writematrix(A,'exact','FileType','text','Delimiter','semi');C=readmatrix('exact','FileType','text');writematrix(A,'tabs.dat','Delimiter','tab','WriteMode','overwrite');D=readmatrix('tabs.dat','Delimiter','\t');").unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("exact")).unwrap(),
        b"1;2\n3;4\n"
    );
    assert!(!dir.path().join("exact.txt").exists());
    assert_eq!(
        std::fs::read(dir.path().join("tabs.dat")).unwrap(),
        b"1\t2\n3\t4\n"
    );
    for name in ["B", "C", "D"] {
        same(&e.variables["A"], &e.variables[name]);
    }
    std::fs::write(
        dir.path().join("bom.csv"),
        b"\xef\xbb\xbf\r\n 1 , 2 \r\n\r\n3,4\r\n",
    )
    .unwrap();
    e.execute("E=readmatrix('bom.csv','Delimiter',',');")
        .unwrap();
    same(&e.variables["A"], &e.variables["E"]);
}

#[test]
fn script_text_rejects_unsupported_options_without_touching_destinations() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    e.execute("A=[1 2;3 4];R=42;").unwrap();
    let dest = dir.path().join("existing.csv");
    std::fs::write(&dest, b"keep").unwrap();
    for script in [
        "R=writematrix(A,'existing.csv');",
        "disp(writematrix(A,'existing.csv'));",
        "writematrix(A,'existing.csv','WriteMode','append');",
        "writematrix(A,'existing.csv','Delimiter','|');",
        "writematrix(A,'existing.csv','Delimiter',',','Delimiter',';');",
        "writematrix(A,'existing.csv','Sheet',1);",
        "writematrix(A,'existing.csv','Delimiter');",
        "writematrix(A);",
        "writematrix(A,'bad.xlsx');",
        "writematrix(A,'https://example.com/file.csv');",
        "R=readmatrix('existing.csv','NumHeaderLines',1);",
        "R=readmatrix('existing.csv','Range','A1:B2');",
        "R=readmatrix('existing.csv','FileType','spreadsheet');",
        "R=readmatrix('existing.csv','Delimiter','comma');",
        "R=readmatrix('existing.csv','WriteMode','overwrite');",
        "R=readmatrix('extensionless');",
    ] {
        assert!(e.execute(script).is_err(), "{script}");
        assert_eq!(std::fs::read(&dest).unwrap(), b"keep");
        assert_eq!(e.variables["R"].data(), &[42.]);
    }
    assert!(!dir.path().join("bad.xlsx").exists());
    assert!(!dir.path().join("A.txt").exists());
}

#[test]
fn strict_numeric_import_never_discards_headers_missing_fields_or_mixed_types() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    e.execute("R=42;").unwrap();
    let path = dir.path().join("bad.csv");
    for bytes in [
        b"time,value\n1,2\n".as_slice(),
        b"1,\n2,3\n",
        b"1,2\n3,4,5\n",
        b"\"1\",2\n",
        b"1,2;3\n",
        b"1,2\n3;4\n",
        b"1,hello\n",
        b"\xff,1\n",
    ] {
        std::fs::write(&path, bytes).unwrap();
        assert!(e.execute("R=readmatrix('bad.csv');").is_err());
        assert_eq!(e.variables["R"].data(), &[42.]);
    }
    std::fs::write(&path, b"1,2;3\n4,5;6\n").unwrap();
    assert_eq!(
        files::read_csv(&path).unwrap().data(),
        &[1., 4., 2., 5., 3., 6.]
    );
    assert!(files::read_matrix_text(&path, None, &AtomicBool::new(false)).is_err());
}

#[test]
fn streaming_text_api_handles_precancel_invalid_delimiters_and_empty_arrays() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keep.csv");
    std::fs::write(&path, b"keep").unwrap();
    let array: Array = Matrix::scalar(3.).into();
    assert!(
        files::write_matrix_text(&path, &array, ',', &AtomicBool::new(true))
            .unwrap_err()
            .contains("cancelled")
    );
    assert!(files::write_matrix_text(&path, &array, '|', &AtomicBool::new(false)).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"keep");
    assert!(
        files::read_matrix_text(
            &dir.path().join("missing.csv"),
            None,
            &AtomicBool::new(true)
        )
        .unwrap_err()
        .contains("cancelled")
    );
    for (rows, cols) in [(0, 0), (0, 3), (7, 0)] {
        files::write_matrix_text(
            &path,
            &Matrix::filled(rows, cols, 0.).unwrap().into(),
            ',',
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(std::fs::read(&path).unwrap().is_empty());
        let loaded = files::read_matrix_text(&path, None, &AtomicBool::new(false)).unwrap();
        assert_eq!((loaded.rows(), loaded.cols()), (0, 0));
    }
    let mut e = engine(dir.path());
    e.execute("A=7;").unwrap();
    std::fs::write(&path, b"keep").unwrap();
    e.cancellation().store(true, Ordering::Relaxed);
    assert!(e.execute("writematrix(A,'keep.csv');").is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"keep");
}

#[test]
fn parallel_strict_reader_preserves_row_order_and_physical_error_lines() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("large.csv");
    let mut text = String::from("\u{feff}\r\n");
    for row in 0..70000 {
        text.push_str(&format!("{row};{};{}\r\n", row + 1, row + 2));
    }
    std::fs::write(&path, &text).unwrap();
    for workers in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        let matrix = pool
            .install(|| files::read_matrix_text(&path, None, &AtomicBool::new(false)))
            .unwrap();
        assert_eq!((matrix.rows(), matrix.cols()), (70000, 3));
        for row in [0, 1, 32768, 69999] {
            for col in 0..3 {
                assert_eq!(matrix.get(row, col), (row + col) as f64);
            }
        }
        assert!(
            pool.install(|| files::read_matrix_text(&path, Some(','), &AtomicBool::new(false)))
                .is_err()
        );
    }
    text.push_str("1;bad;3\n");
    std::fs::write(&path, text).unwrap();
    for workers in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        let error = pool
            .install(|| files::read_matrix_text(&path, None, &AtomicBool::new(false)))
            .unwrap_err();
        assert!(error.contains("line 70002"), "{error}");
    }
}

#[test]
fn script_text_commands_respect_variable_and_user_function_shadowing() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    e.execute("readmatrix=[7 8];writematrix=[9 10];R=readmatrix(2);W=writematrix(1);")
        .unwrap();
    assert_eq!(e.variables["R"].data(), &[8.]);
    assert_eq!(e.variables["W"].data(), &[9.]);
    let mut e = engine(dir.path());
    e.execute("R=writematrix(4);\nfunction y=writematrix(x)\ny=x+1;\nend")
        .unwrap();
    assert_eq!(e.variables["R"].data(), &[5.]);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn stream_writer_matches_existing_numeric_csv_bytes_without_a_payload_buffer() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("old.csv");
    let b = dir.path().join("new.csv");
    let values: Vec<_> = (0..32769)
        .map(|i| {
            if i == 8192 {
                -0.
            } else {
                (i as f64 - 16384.) / 1024.
            }
        })
        .collect();
    let matrix = Matrix::from_column_vec(10923, 3, values).unwrap();
    files::write_array_csv(&a, &matrix.clone().into(), &AtomicBool::new(false)).unwrap();
    files::write_matrix_text(&b, &matrix.clone().into(), ',', &AtomicBool::new(false)).unwrap();
    assert_eq!(std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
    same(
        &matrix,
        &files::read_matrix_text(&b, Some(','), &AtomicBool::new(false)).unwrap(),
    );
}

#[test]
fn strict_tab_missing_records_reject_inferred_and_explicit_serial_and_parallel_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing-tabs.txt");
    for workers in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        for records in [2, 70000] {
            for before in [0, records / 2] {
                let text =
                    "1\t2\n".repeat(before) + " \t \r\n" + &"3\t4\n".repeat(records - before);
                if records == 70000 {
                    assert!(text.len() >= 256 * 1024);
                }
                std::fs::write(&path, text).unwrap();
                for suffix in ["", ",'Delimiter','\t'"] {
                    let mut e = engine(dir.path());
                    e.execute("R=42;").unwrap();
                    let script = format!("R=readmatrix('missing-tabs.txt'{suffix});");
                    let error = pool.install(|| e.execute(&script)).unwrap_err();
                    assert!(
                        error.contains("Invalid numeric field"),
                        "{workers}/{records}/{before}/{suffix}: {error}"
                    );
                    assert!(error.contains(&format!("line {}", before + 1)), "{error}");
                    assert_eq!(e.variables["R"].data(), &[42.]);
                }
            }
        }
        std::fs::write(&path, "\t\r\n").unwrap();
        for delimiter in [None, Some('\t')] {
            assert!(
                pool.install(|| files::read_matrix_text(&path, delimiter, &AtomicBool::new(false)))
                    .is_err()
            );
        }
    }
}

#[test]
fn strict_tab_blank_records_and_legacy_missing_record_behavior_remain_distinct() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tabs.txt");
    std::fs::write(&path, "\n \r\n1\t2\n\r\n  \n3\t4\n").unwrap();
    for delimiter in [None, Some('\t')] {
        let matrix = files::read_matrix_text(&path, delimiter, &AtomicBool::new(false)).unwrap();
        assert_eq!((matrix.rows(), matrix.cols()), (2, 2));
        assert_eq!(matrix.data(), &[1., 3., 2., 4.]);
    }
    std::fs::write(&path, "1\t2\n\t\n3\t4\n").unwrap();
    let legacy = files::read_csv(&path).unwrap();
    assert_eq!((legacy.rows(), legacy.cols()), (2, 2));
    assert_eq!(legacy.data(), &[1., 3., 2., 4.]);
    assert!(files::read_matrix_text(&path, None, &AtomicBool::new(false)).is_err());
}
