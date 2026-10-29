use matrust::{
    Array, LogicalMatrix, Matrix,
    array::{MissingPlacement, SortOrder},
};
use std::{collections::HashSet, sync::atomic::AtomicBool};

// Generated independently by a separate Python sort after parent clearance.
// Exact binary64 bits preserve quiet-NaN payloads and signed-zero stable ties.
#[test]
fn independent_python_sort_fixture_all_shapes_modes_and_indices() {
    let csv = include_str!("sort_oracle.csv");
    let pools = [1, 8].map(|n| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .unwrap()
    });
    let cancel = AtomicBool::new(false);
    let shapes = [
        (0, 0),
        (0, 3),
        (3, 0),
        (1, 0),
        (0, 1),
        (1, 1),
        (1, 19),
        (19, 1),
        (3, 7),
        (7, 3),
    ];
    let mut seen = HashSet::new();
    let bits = |text: &str| {
        if text.is_empty() {
            Vec::new()
        } else {
            text.split(';')
                .map(|s| {
                    assert_eq!(s.len(), 16);
                    u64::from_str_radix(s, 16).unwrap()
                })
                .collect::<Vec<_>>()
        }
    };
    let mut lines = csv.lines();
    assert_eq!(
        lines.next().unwrap(),
        "rows,cols,logical,dim,descending,missing,input_bits,output_bits,indices"
    );
    for line in lines {
        let fields = line.split(',').collect::<Vec<_>>();
        assert_eq!(fields.len(), 9);
        let rows = fields[0].parse::<usize>().unwrap();
        let cols = fields[1].parse::<usize>().unwrap();
        let logical = fields[2].parse::<u8>().unwrap();
        let dim = fields[3].parse::<usize>().unwrap();
        let descending = fields[4].parse::<u8>().unwrap();
        assert!(
            shapes.contains(&(rows, cols))
                && logical <= 1
                && descending <= 1
                && (1..=3).contains(&dim)
        );
        let missing = match fields[5] {
            "auto" => MissingPlacement::Auto,
            "first" => MissingPlacement::First,
            "last" => MissingPlacement::Last,
            _ => panic!("Unexpected missing placement"),
        };
        assert!(seen.insert((rows, cols, logical, dim, descending, fields[5].to_owned())));
        let input = bits(fields[6]);
        let output = bits(fields[7]);
        let indices = if fields[8].is_empty() {
            Vec::new()
        } else {
            fields[8]
                .split(';')
                .map(|v| v.parse::<usize>().unwrap() as f64)
                .collect::<Vec<_>>()
        };
        assert_eq!(input.len(), rows * cols);
        assert_eq!(output.len(), input.len());
        assert_eq!(indices.len(), input.len());
        let array: Array = if logical == 1 {
            let values = input
                .iter()
                .map(|&v| {
                    let value = f64::from_bits(v);
                    assert!(value == 0. || value == 1.);
                    value as u8
                })
                .collect();
            LogicalMatrix::from_column_vec(rows, cols, values, &cancel)
                .unwrap()
                .into()
        } else {
            Matrix::from_column_vec(
                rows,
                cols,
                input.iter().map(|&v| f64::from_bits(v)).collect(),
            )
            .unwrap()
            .into()
        };
        for pool in &pools {
            for want_indices in [false, true] {
                let (actual, positions) = pool
                    .install(|| {
                        array.sort_dimension(
                            dim,
                            if descending == 1 {
                                SortOrder::Descend
                            } else {
                                SortOrder::Ascend
                            },
                            missing,
                            want_indices,
                            &cancel,
                        )
                    })
                    .unwrap();
                assert_eq!((actual.rows(), actual.cols()), (rows, cols));
                assert_eq!(actual.is_logical(), logical == 1);
                for (index, &expected) in output.iter().enumerate() {
                    assert_eq!(
                        actual
                            .scalar(index % rows, index / rows)
                            .unwrap()
                            .into_real()
                            .unwrap()
                            .to_bits(),
                        expected,
                        "{line} output {index}"
                    );
                }
                if want_indices {
                    let positions = positions.unwrap();
                    assert_eq!((positions.rows(), positions.cols()), (rows, cols));
                    assert_eq!(positions.data(), indices.as_slice());
                } else {
                    assert!(positions.is_none());
                }
                for (index, &expected) in input.iter().enumerate() {
                    assert_eq!(
                        array
                            .scalar(index % rows, index / rows)
                            .unwrap()
                            .into_real()
                            .unwrap()
                            .to_bits(),
                        expected
                    );
                }
            }
        }
    }
    // 10 shapes × 2 types × 3 axes × 2 directions × 3 missing modes.
    assert_eq!(seen.len(), 360);
}
