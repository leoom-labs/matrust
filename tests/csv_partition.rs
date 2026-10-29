use matrust::files;
use std::sync::atomic::AtomicBool;

#[test]
fn one_wide_record_and_blank_lines_have_independent_expected_values_on_one_and_eight_workers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wide.csv");
    let fields = 262144;
    let text = format!("\u{feff}\n\n{}2\r\n\n", "1,".repeat(fields - 1));
    std::fs::write(&path, text).unwrap();
    let pools = [1, 8].map(|n| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .unwrap()
    });
    for pool in &pools {
        pool.install(|| {
            let cancel = AtomicBool::new(false);
            for result in [
                files::read_csv(&path),
                files::read_csv_with_cancel(&path, &cancel),
                files::read_matrix_text(&path, None, &cancel),
            ] {
                let matrix = result.unwrap();
                assert_eq!((matrix.rows(), matrix.cols()), (1, fields));
                assert!(
                    matrix.data()[..fields - 1]
                        .iter()
                        .all(|x| x.to_bits() == 1f64.to_bits())
                );
                assert_eq!(matrix.data()[fields - 1], 2.);
            }
        });
    }
}

#[test]
fn partitioning_preserves_first_physical_error_and_utf8_admission() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("errors.csv");
    let text = format!(
        "{}bad,2\n{}1,2,3\n{}",
        "1,2\n".repeat(20000),
        "1,2\n".repeat(20000),
        "1,2\n".repeat(40000)
    );
    std::fs::write(&path, text).unwrap();
    for workers in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            let cancel = AtomicBool::new(false);
            assert_eq!(
                files::read_csv_with_cancel(&path, &cancel).unwrap_err(),
                "Invalid numeric field on line 20001"
            );
            assert_eq!(
                files::read_matrix_text(&path, None, &cancel).unwrap_err(),
                "Invalid numeric field on line 20001"
            );
        });
    }
    let mut invalid = b"1,2\n".repeat(80000);
    invalid[200000] = 0xff;
    std::fs::write(&path, invalid).unwrap();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .build()
        .unwrap();
    assert!(
        pool.install(|| files::read_csv_with_cancel(&path, &AtomicBool::new(false)))
            .unwrap_err()
            .contains("UTF-8")
    );
}
