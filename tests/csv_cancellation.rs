use matrust::files;
use std::sync::atomic::AtomicBool;

#[test]
fn cancellable_permissive_import_matches_independent_expected_bits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.csv");
    let cancel = AtomicBool::new(false);
    let row = "-0;1e-300,2\t5e-324\r\n";
    let cases = [
        (String::new(), 0, 0, Vec::new()),
        ("\u{feff} \r\n1;2\n3,4".into(), 2, 2, vec![1., 3., 2., 4.]),
        (
            row.repeat(20000),
            20000,
            4,
            [-0., 1e-300, 2., 5e-324]
                .into_iter()
                .flat_map(|v| std::iter::repeat_n(v, 20000))
                .collect(),
        ),
        (
            format!("{}-0", "1.2345678901234567,".repeat(150000)),
            1,
            150001,
            std::iter::repeat_n(1.2345678901234567, 150000)
                .chain([-0.])
                .collect(),
        ),
    ];
    for (text, rows, cols, expected) in cases {
        std::fs::write(&path, text).unwrap();
        let expected_bits: Vec<_> = expected.iter().map(|v: &f64| v.to_bits()).collect();
        let mut one_worker_bits = None;
        for workers in [1, 8] {
            rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap()
                .install(|| {
                    let previous = files::read_csv(&path).unwrap();
                    let actual = files::read_csv_with_cancel(&path, &cancel).unwrap();
                    assert_eq!((actual.rows(), actual.cols()), (rows, cols));
                    assert_eq!((previous.rows(), previous.cols()), (rows, cols));
                    let actual_bits: Vec<_> = actual.data().iter().map(|v| v.to_bits()).collect();
                    assert_eq!(actual_bits, expected_bits);
                    assert_eq!(
                        previous
                            .data()
                            .iter()
                            .map(|v| v.to_bits())
                            .collect::<Vec<_>>(),
                        expected_bits
                    );
                    if workers == 1 {
                        one_worker_bits = Some(actual_bits);
                    } else {
                        assert_eq!(actual_bits, *one_worker_bits.as_ref().unwrap());
                    }
                });
        }
    }
    std::fs::write(&path, "1,2\n3,4\nx,6").unwrap();
    assert_eq!(
        files::read_csv_with_cancel(&path, &cancel).unwrap_err(),
        "Invalid numeric field on line 3"
    );
    assert!(
        files::read_csv_with_cancel(&path, &AtomicBool::new(true))
            .unwrap_err()
            .contains("cancelled")
    );
    std::fs::write(&path, [0xff]).unwrap();
    assert!(
        files::read_csv_with_cancel(&path, &cancel)
            .unwrap_err()
            .contains("UTF-8")
    );
}
