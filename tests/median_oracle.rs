use matrust::{Matrix, matrix::MedianAxis};
use std::sync::atomic::AtomicBool;

fn values(text: &str) -> Vec<f64> {
    if text.is_empty() {
        Vec::new()
    } else {
        text.split(';')
            .map(|v| f64::from_bits(u64::from_str_radix(v, 16).unwrap()))
            .collect()
    }
}

#[test]
fn median_matches_310_independent_exact_midpoint_and_sorted_references() {
    let pools = [1, 8].map(|workers| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
    });
    let cancel = AtomicBool::new(false);
    let mut count = 0;
    let mut keys = std::collections::BTreeSet::new();
    for line in include_str!("median_oracle.csv").lines().skip(1) {
        let fields: Vec<_> = line.split(',').collect();
        assert_eq!(fields.len(), 9);
        assert!(matches!(fields[4], "0" | "1"));
        assert!(
            keys.insert((
                fields[0].to_owned(),
                fields[3].to_owned(),
                fields[4].to_owned()
            )),
            "Duplicate reference case"
        );
        let rows: usize = fields[1].parse().unwrap();
        let cols: usize = fields[2].parse().unwrap();
        let input = Matrix::from_column_vec(rows, cols, values(fields[5])).unwrap();
        let axis = match fields[3] {
            "default" => MedianAxis::Default,
            "all" => MedianAxis::All,
            dim => MedianAxis::Dimension(dim.parse().unwrap()),
        };
        let expected = values(fields[8]);
        let shape = (
            fields[6].parse::<usize>().unwrap(),
            fields[7].parse::<usize>().unwrap(),
        );
        for pool in &pools {
            let actual = pool
                .install(|| input.median(axis, fields[4] == "1", &cancel))
                .unwrap();
            assert_eq!(
                (actual.rows(), actual.cols()),
                shape,
                "{} axis{}",
                fields[0],
                fields[3]
            );
            assert_eq!(actual.len(), expected.len());
            for (i, (&actual, &expected)) in actual.data().iter().zip(&expected).enumerate() {
                if expected.is_nan() {
                    assert!(actual.is_nan(), "{} index{i}", fields[0]);
                } else {
                    assert_eq!(
                        actual.to_bits(),
                        expected.to_bits(),
                        "{} axis{} omit{} index{i}",
                        fields[0],
                        fields[3],
                        fields[4]
                    );
                }
            }
        }
        count += 1;
    }
    assert_eq!(count, 310);
    let names = [
        "row_even",
        "column_odd",
        "matrix",
        "missing_matrix",
        "all_missing",
        "nan_single",
        "singleton",
        "negative_zero",
        "mixed_zero",
        "half_subnormal",
        "subnormal_tie",
        "negative_subnormal",
        "max_same",
        "max_neighbors",
        "opposite_max",
        "mixed_extremes",
        "infinite_same",
        "opposite_infinite",
        "negative_infinite",
        "nan_payloads",
        "empty_0_0",
        "empty_0_3",
        "empty_3_0",
        "empty_1_0",
        "empty_0_1",
        "boundary_8191",
        "boundary_8192",
        "boundary_8193",
        "boundary_8194",
        "boundary_16385",
        "strided_radix",
    ];
    let mut expected_keys = std::collections::BTreeSet::new();
    for name in names {
        for axis in ["default", "1", "2", "3", "all"] {
            for omit in ["0", "1"] {
                expected_keys.insert((name.to_owned(), axis.to_owned(), omit.to_owned()));
            }
        }
    }
    assert_eq!(keys, expected_keys);
}
