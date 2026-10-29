//! Public vector parity and opt-in release measurement; no production control path.
use super::*;
use crate::files;

// Exact predecessor serial implementation, with function/parent-path naming only.
fn serial_before<const STRICT: bool>(
    file: std::fs::File,
    mut delimiter: char,
    cancel: &AtomicBool,
) -> Result<Matrix> {
    let mut reader =
        BufReader::with_capacity(64 * 1024, file.take(super::super::MAX_CSV_BYTES as u64 + 1));
    let mut line = Vec::new();
    let mut line_number = 0;
    let mut bytes_read = 0;
    let mut values = Vec::new();
    let mut rows = 0;
    let mut cols = None;
    loop {
        check(cancel)?;
        line.clear();
        let count = read_record(
            &mut reader,
            &mut line,
            super::super::MAX_CSV_BYTES - bytes_read,
            cancel,
        )?;
        if count == 0 {
            break;
        }
        bytes_read += count;
        if bytes_read > super::super::MAX_CSV_BYTES {
            return Err("CSV file exceeds 512 MiB".into());
        }
        line_number += 1;
        let line = std::str::from_utf8(&line).map_err(|_| "CSV is not valid UTF-8")?;
        let line = if line_number == 1 {
            line.trim_start_matches('\u{feff}')
        } else {
            line
        };
        if blank_record::<STRICT>(line, delimiter) {
            continue;
        }
        if STRICT && delimiter == '\0' {
            delimiter = detect(line)?;
        }
        let mut columns: usize = 0;
        for cell in line
            .trim_end_matches(['\r', '\n'])
            .split(|c| separator::<STRICT>(c, delimiter))
        {
            if columns.is_multiple_of(8192) {
                check(cancel)?;
            }
            columns += 1;
            Matrix::checked_len(rows + 1, columns)?;
            let value = cell
                .trim()
                .parse::<f64>()
                .map_err(|_| format!("Invalid numeric field on line {line_number}"))?;
            values.push(value);
        }
        if cols.is_some_and(|n| n != columns) {
            return Err(format!("Inconsistent column count on line {}", line_number));
        }
        cols = Some(columns);
        rows += 1;
    }
    check(cancel)?;
    let cols = cols.unwrap_or(0);
    if STRICT && (cols <= 1 || rows <= 1) {
        return Matrix::from_column_vec(rows, cols, values);
    }
    let mut column_major = vec![0.0; values.len()];
    for r in 0..rows {
        check(cancel)?;
        for c in 0..cols {
            if c.is_multiple_of(8192) {
                check(cancel)?;
            }
            column_major[r + c * rows] = values[r * cols + c];
        }
    }
    check(cancel)?;
    Matrix::from_column_vec(rows, cols, column_major)
}

// Only used for the predecessor's serial route. The caller chooses that route
// outside timing from the authored file length and requested/observed context.
fn legacy_serial_before(path: &Path, cancel: &AtomicBool) -> Result<Matrix> {
    check(cancel)?;
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let length = file.metadata().map_err(|e| e.to_string())?.len();
    if length > super::super::MAX_CSV_BYTES as u64 {
        return Err("CSV file exceeds 512 MiB".into());
    }
    serial_before::<false>(file, '\0', cancel)
}
fn assert_bits(actual: &Matrix, rows: usize, cols: usize, expected: &[f64]) {
    assert_eq!((actual.rows(), actual.cols()), (rows, cols));
    assert_eq!(actual.len(), expected.len());
    for (&a, &b) in actual.data().iter().zip(expected) {
        if b.is_nan() {
            assert!(a.is_nan());
        } else {
            assert_eq!(a.to_bits(), b.to_bits());
        }
    }
}
fn authored_vector(count: usize, row: bool) -> (String, Vec<f64>) {
    let words = [
        "-0",
        "inf",
        "-inf",
        "NaN",
        "5e-324",
        "1e-300",
        "1.2345678901234567",
        "-9",
    ];
    let expected = [
        -0.,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
        f64::from_bits(1),
        1e-300,
        1.2345678901234567,
        -9.,
    ];
    let mut text = String::new();
    let mut values = Vec::with_capacity(count);
    for i in 0..count {
        if i != 0 {
            text.push_str(if row { "," } else { "\r\n" });
        }
        text.push_str(words[i % words.len()]);
        values.push(expected[i % expected.len()]);
    }
    text.push_str("\r\n");
    (text, values)
}

#[test]
fn public_vector_apis_keep_shapes_bits_and_legacy_nonfinite_handling() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("vector.csv");
    for workers in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            assert_eq!(rayon::current_num_threads(), workers);
            for count in [1, 8, 32768] {
                for row in [false, true] {
                    let (text, expected) = authored_vector(count, row);
                    std::fs::write(&path, text).unwrap();
                    let cancel = AtomicBool::new(false);
                    let (rows, cols) = if row { (1, count) } else { (count, 1) };
                    for actual in [
                        files::read_csv(&path).unwrap(),
                        files::read_csv_with_cancel(&path, &cancel).unwrap(),
                        files::read_matrix_text(&path, None, &cancel).unwrap(),
                        legacy_serial_before(&path, &cancel).unwrap(),
                    ] {
                        assert_bits(&actual, rows, cols, &expected);
                        let shared = actual.clone();
                        assert!(std::sync::Arc::ptr_eq(&actual.storage, &shared.storage));
                    }
                }
            }
        });
    }
}

#[test]
fn empty_mixed_delimiters_and_bad_vectors_keep_existing_contracts() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("edge.csv");
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    pool.install(|| {
        let cancel = AtomicBool::new(false);
        for text in ["", "\r\n \n", "\u{feff}\r\n"] {
            std::fs::write(&path, text).unwrap();
            assert_bits(
                &files::read_csv_with_cancel(&path, &cancel).unwrap(),
                0,
                0,
                &[],
            );
            assert_bits(
                &files::read_matrix_text(&path, None, &cancel).unwrap(),
                0,
                0,
                &[],
            );
        }
        std::fs::write(&path, "-0;inf,NaN\t5e-324\r\n").unwrap();
        assert_bits(
            &files::read_csv_with_cancel(&path, &cancel).unwrap(),
            1,
            4,
            &[-0., f64::INFINITY, f64::NAN, f64::from_bits(1)],
        );
        assert!(
            files::read_matrix_text(&path, None, &cancel)
                .unwrap_err()
                .contains("Mixed delimiters")
        );
        for text in [
            "1,",
            ",1",
            "1\ntext\n",
            "1,2\n3\n",
            "1\n2,3\n",
            "'1'",
            "[1 2]",
        ] {
            std::fs::write(&path, text).unwrap();
            let original = legacy_serial_before(&path, &cancel).unwrap_err();
            assert_eq!(
                files::read_csv_with_cancel(&path, &cancel).unwrap_err(),
                original
            );
            assert!(files::read_matrix_text(&path, None, &cancel).is_err());
        }
        std::fs::write(&path, [0xff, b'\n']).unwrap();
        assert_eq!(
            files::read_csv_with_cancel(&path, &cancel).unwrap_err(),
            "CSV is not valid UTF-8"
        );
    });
}

#[test]
fn vector_precancel_preserves_file_and_retry_keeps_exact_values() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cancel.csv");
    let (text, expected) = authored_vector(32768, false);
    std::fs::write(&path, text.as_bytes()).unwrap();
    let cancel = AtomicBool::new(true);
    assert_eq!(
        files::read_csv_with_cancel(&path, &cancel).unwrap_err(),
        "Calculation cancelled"
    );
    assert_eq!(
        files::read_matrix_text(&path, None, &cancel).unwrap_err(),
        "Calculation cancelled"
    );
    assert_eq!(std::fs::read(&path).unwrap(), text.as_bytes());
    cancel.store(false, Ordering::Relaxed);
    assert_bits(
        &files::read_csv_with_cancel(&path, &cancel).unwrap(),
        32768,
        1,
        &expected,
    );
    // Existing record/cell/transpose cancellation regressions remain unchanged.
}

#[test]
#[ignore = "opt-in release measurements; parent owns accepted binaries and quiet lane"]
fn csv_vector_release_measurements() {
    if cfg!(debug_assertions) {
        panic!("release required");
    }
    let directory = tempfile::tempdir().unwrap();
    let pools: Vec<_> = [1, 8]
        .into_iter()
        .map(|n| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(n)
                .build()
                .unwrap()
        })
        .collect();
    let mut fixtures = Vec::new();
    for count in [64, 2_097_152] {
        for row in [false, true] {
            let (text, expected) = authored_vector(count, row);
            let path = directory.path().join(format!("vector_{count}_{row}.csv"));
            std::fs::write(&path, text.as_bytes()).unwrap();
            fixtures.push((count, row, path, text.len(), expected));
        }
    }
    let mut schedule = Vec::new();
    for index in 0..fixtures.len() {
        for worker in [0, 1] {
            for before in [true, false] {
                schedule.push((index, worker, before));
            }
        }
    }
    let reverse = std::env::var_os("MATRUST_CSV_VECTOR_BENCH_REVERSE").is_some();
    if reverse {
        schedule.reverse();
    }
    println!(
        "CSV_VECTOR\telements\trow\tworkers\tvariant\torder\troute\tphase\tsample\tinput_bytes\telapsed_ns"
    );
    for (index, worker, before) in schedule {
        let (count, row, path, encoded, expected) = &fixtures[index];
        let workers = [1, 8][worker];
        let serial = *encoded < 256 * 1024 || workers == 1;
        let cancel = AtomicBool::new(false);
        for sample in 0..9 {
            let (matrix, elapsed) = pools[worker].install(|| {
                assert_eq!(rayon::current_num_threads(), workers);
                let start = std::time::Instant::now();
                let matrix = if before && serial {
                    legacy_serial_before(path, &cancel).unwrap()
                } else {
                    // Predecessor and candidate already share this batched path.
                    files::read_csv_with_cancel(path, &cancel).unwrap()
                };
                (matrix, start.elapsed().as_nanos())
            });
            let (rows, cols) = if *row { (1, *count) } else { (*count, 1) };
            assert_bits(&matrix, rows, cols, expected);
            println!(
                "CSV_VECTOR\t{count}\t{row}\t{workers}\t{}\t{}\t{}\t{}\t{}\t{encoded}\t{elapsed}",
                if before {
                    "before_copy_control"
                } else {
                    "public_owned_return"
                },
                if reverse { "reverse" } else { "normal" },
                if serial {
                    "serial"
                } else {
                    "unchanged_batched"
                },
                if sample < 2 { "warmup" } else { "retained" },
                if sample < 2 { sample } else { sample - 2 }
            );
            drop(matrix);
        }
    }
}
