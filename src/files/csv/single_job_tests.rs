use super::*;
use std::cell::{Cell, RefCell};

thread_local! {
    static TRACK: Cell<bool> = const { Cell::new(false) };
    static CANCEL_AT: Cell<Option<usize>> = const { Cell::new(None) };
    static VISITS: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

pub(super) fn checkpoint(cancel: &AtomicBool, length: usize) {
    if !TRACK.with(Cell::get) {
        return;
    }
    VISITS.with(|visits| visits.borrow_mut().push(length));
    CANCEL_AT.with(|at| {
        if let Some(n) = at.get() {
            if n == 0 {
                at.set(None);
                cancel.store(true, Ordering::Relaxed);
            } else {
                at.set(Some(n - 1));
            }
        }
    });
}

struct Reset;
impl Reset {
    fn new(at: Option<usize>) -> Self {
        TRACK.with(|n| n.set(true));
        CANCEL_AT.with(|n| n.set(at));
        VISITS.with(|v| v.borrow_mut().clear());
        Self
    }
}
impl Drop for Reset {
    fn drop(&mut self) {
        TRACK.with(|n| n.set(false));
        CANCEL_AT.with(|n| n.set(None));
        VISITS.with(|v| v.borrow_mut().clear());
    }
}

fn bits(values: &[f64]) -> Vec<u64> {
    values.iter().map(|v| v.to_bits()).collect()
}

fn compare<const STRICT: bool>(
    prefix: &[f64],
    text: &str,
    line: usize,
    cols: usize,
    remaining: usize,
    delimiter: char,
) {
    let cancel = AtomicBool::new(false);
    let mut expected = prefix.to_vec();
    let reference = parse::<STRICT>(text, line, cols, remaining, delimiter, &cancel)
        .and_then(|part| append(&mut expected, part, cols));
    let mut actual = prefix.to_vec();
    let result = parse_single_job_into::<STRICT>(
        &mut actual,
        text,
        line,
        cols,
        remaining,
        delimiter,
        &cancel,
    );
    match (reference, result) {
        (Ok(()), Ok(())) => assert_eq!(bits(&actual), bits(&expected)),
        (Err(a), Err(b)) => assert_eq!(a, b),
        (a, b) => panic!("reference and accumulator differ: {a:?}, {b:?}"),
    }
}

#[test]
fn accumulator_matches_unchanged_parser_bits_prefixes_blanks_and_physical_errors() {
    let _reset = Reset::new(None);
    for prefix in [vec![], vec![7., -0.]] {
        for text in [
            "-0,5e-324\r\ninf,-inf\r\nNaN,1.25\n",
            "\n\r\n1,2\n\n3,4",
            "",
            " \n\r\n",
        ] {
            compare::<false>(&prefix, text, 17, 2, 64, ',');
            compare::<true>(&prefix, text, 17, 2, 64, ',');
        }
        for text in [
            "1,2\n\n3,nope",
            "1,2\n\n3",
            "1,2\n3,4,5",
            "1,",
            ",2",
            "1,2,",
            "1,2\n3,4",
        ] {
            compare::<false>(&prefix, text, 17, 2, 3, ',');
            compare::<true>(&prefix, text, 17, 2, 3, ',');
            compare::<false>(&prefix, text, 17, 2, 64, ',');
        }
    }
    compare::<false>(&[], "1;2\n3\t4", 5, 2, 8, ',');
    compare::<true>(&[], "1;2\n3;4", 5, 2, 8, ';');
    compare::<true>(&[], "\t\n", 5, 2, 8, '\t');
    let mut output = vec![8., 9.];
    assert_eq!(
        parse_single_job_into::<false>(
            &mut output,
            "1,2\n\n3,nope",
            17,
            2,
            8,
            ',',
            &AtomicBool::new(false)
        )
        .unwrap_err(),
        "Invalid numeric field on line 19"
    );
}

#[test]
fn growth_admission_rejects_before_allocation_and_reuses_existing_capacity() {
    let _reset = Reset::new(None);
    let mut values = Vec::with_capacity(32);
    values.extend([1., -0.]);
    let pointer = values.as_ptr();
    parse_single_job_into::<true>(
        &mut values,
        "3,4\n5,6",
        1,
        2,
        4,
        ',',
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(values.as_ptr(), pointer);
    assert_eq!(bits(&values), bits(&[1., -0., 3., 4., 5., 6.]));
    let before = bits(&values);
    let capacity = values.capacity();
    for invalid in [MAX_ELEMENTS + 1, usize::MAX] {
        assert!(
            grow_single_job(&mut values, invalid)
                .unwrap_err()
                .contains("element limit")
        );
        assert_eq!(values.capacity(), capacity);
        assert_eq!(bits(&values), before);
    }
    assert!(
        parse_single_job_into::<false>(&mut values, "1,2", 5, 2, 1, ',', &AtomicBool::new(false))
            .unwrap_err()
            .contains("element limit")
    );
    assert_eq!(values.capacity(), capacity);
    assert_eq!(bits(&values), before);
    grow_single_job(&mut values, 33).unwrap();
    assert!(values.capacity() >= 64);
    assert_eq!(bits(&values), before);
}

#[test]
fn every_single_job_checkpoint_cancels_and_retry_preserves_reference_bits() {
    let text = (0usize..20_000)
        .map(|n| if n.is_multiple_of(2) { "-0" } else { "5e-324" })
        .collect::<Vec<_>>()
        .join(",");
    let cancel = AtomicBool::new(false);
    let expected = parse::<false>(&text, 7, 20_000, 20_000, ',', &cancel).unwrap();
    {
        let _reset = Reset::new(None);
        let mut values = vec![];
        parse_single_job_into::<false>(&mut values, &text, 7, 20_000, 20_000, ',', &cancel)
            .unwrap();
        assert_eq!(bits(&values), bits(&expected));
        VISITS.with(|v| assert_eq!(*v.borrow(), [0, 0, 8192, 16384, 20_000]));
    }
    for at in 0..5 {
        let _reset = Reset::new(Some(at));
        cancel.store(false, Ordering::Relaxed);
        let prefix = vec![-0.; 20_000];
        let mut values = prefix.clone();
        assert_eq!(
            parse_single_job_into::<false>(&mut values, &text, 7, 20_000, 20_000, ',', &cancel)
                .unwrap_err(),
            "Calculation cancelled"
        );
        assert!(cancel.load(Ordering::Relaxed));
        assert_eq!(bits(&values[..prefix.len()]), bits(&prefix));
        assert!(values.len() <= 40_000);
        cancel.store(false, Ordering::Relaxed);
        let mut retry = prefix;
        parse_single_job_into::<false>(&mut retry, &text, 7, 20_000, 20_000, ',', &cancel).unwrap();
        assert_eq!(bits(&retry[20_000..]), bits(&expected));
    }
}

#[test]
fn public_wide_records_across_batches_keep_column_major_shape_bits_and_cancellation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("wide.csv");
    // Each complete row exceeds the two-MiB batch, so both batches exercise
    // the single-job branch, including a nonempty final accumulator.
    let columns = 400_000;
    let fields = ["-0", "5e-324", "1.23456789", "inf", "-inf", "NaN"];
    let row = (0..columns)
        .map(|n| fields[n % fields.len()])
        .collect::<Vec<_>>()
        .join(",");
    let second = std::iter::repeat_n("-9.87654321", columns)
        .collect::<Vec<_>>()
        .join(",");
    assert!(row.len() > BATCH_BYTES && second.len() > BATCH_BYTES);
    std::fs::write(&path, format!("\u{feff}{row}\r\n{second}\r\n")).unwrap();
    let original = Matrix::from_column_vec(1, 2, vec![7., -0.]).unwrap();
    let alias = original.clone();
    let mut reference: Option<Vec<u64>> = None;
    for workers in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            assert_eq!(rayon::current_num_threads(), workers);
            let _reset = Reset::new(None);
            let cancel = AtomicBool::new(false);
            let value = crate::files::read_csv_with_cancel(&path, &cancel).unwrap();
            assert_eq!((value.rows(), value.cols()), (2, columns));
            for column in 0..columns {
                let expected = fields[column % fields.len()].parse::<f64>().unwrap();
                assert_eq!(value.data()[2 * column].to_bits(), expected.to_bits());
                assert_eq!(
                    value.data()[2 * column + 1].to_bits(),
                    (-9.87654321_f64).to_bits()
                );
            }
            let observed = VISITS.with(|v| v.borrow().len());
            assert_eq!(observed > 0, workers == 8);
            if let Some(bits_before) = &reference {
                assert_eq!(bits(value.data()), *bits_before);
            } else {
                reference = Some(bits(value.data()));
            }
            let strict = read_strict(&path, Some(','), &cancel).unwrap();
            assert_eq!(bits(strict.data()), bits(value.data()));
            if workers == 8 {
                CANCEL_AT.with(|n| n.set(Some(3)));
                assert_eq!(
                    crate::files::read_csv_with_cancel(&path, &cancel).unwrap_err(),
                    "Calculation cancelled"
                );
                assert!(cancel.load(Ordering::Relaxed));
                cancel.store(false, Ordering::Relaxed);
                let retry = crate::files::read_csv_with_cancel(&path, &cancel).unwrap();
                assert_eq!(bits(retry.data()), bits(value.data()));
            }
        });
    }
    assert_eq!(original.data().as_ptr(), alias.data().as_ptr());
    assert_eq!(bits(original.data()), bits(&[7., -0.]));
}

#[test]
fn independent_many_job_control_never_enters_single_job_accumulator() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("many-rows.csv");
    let text = "-0,5e-324,3.25\r\n".repeat(40_000);
    assert!(text.len() > 256 * 1024 && text.len() < BATCH_BYTES);
    std::fs::write(&path, text).unwrap();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .build()
        .unwrap();
    pool.install(|| {
        let _reset = Reset::new(None);
        let value = crate::files::read_csv_with_cancel(&path, &AtomicBool::new(false)).unwrap();
        assert_eq!((value.rows(), value.cols()), (40_000, 3));
        for (column, expected) in [-0., f64::from_bits(1), 3.25].into_iter().enumerate() {
            assert!(
                value.data()[column * 40_000..(column + 1) * 40_000]
                    .iter()
                    .all(|v| v.to_bits() == expected.to_bits())
            );
        }
        VISITS.with(|v| assert!(v.borrow().is_empty()));
    });
}
