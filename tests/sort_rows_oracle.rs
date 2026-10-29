use matrust::{
    Array, LogicalMatrix, Matrix,
    array::{MissingPlacement as M, SortOrder as O},
};
use std::{collections::BTreeSet, sync::atomic::AtomicBool};

#[test]
fn sortrows_independent_python_complete_tuple_references() {
    let csv = include_str!("sort_rows_oracle.csv");
    let mut lines = csv.lines();
    assert_eq!(
        lines.next().unwrap(),
        "rows,cols,logical,keys,direction,missing,input_bits,output_bits,indices"
    );
    let pools = [1, 8].map(|n| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .unwrap()
    });
    let cancel = AtomicBool::new(false);
    let mut seen = BTreeSet::new();
    let mut expected = BTreeSet::new();
    for (r, c) in [
        (0, 0),
        (0, 3),
        (3, 0),
        (1, 0),
        (0, 1),
        (1, 1),
        (1, 7),
        (19, 1),
        (3, 7),
        (17, 3),
    ] {
        for logical in [0, 1] {
            for key in if c == 0 {
                vec!["all"]
            } else {
                vec!["all", "first", "mixed"]
            } {
                for d in ["signed", "ascend", "descend"] {
                    for m in ["auto", "first", "last"] {
                        expected.insert((
                            r,
                            c,
                            logical,
                            key.to_owned(),
                            d.to_owned(),
                            m.to_owned(),
                        ));
                    }
                }
            }
        }
    }
    for line in lines {
        let fields: Vec<_> = line.split(',').collect();
        assert_eq!(fields.len(), 9);
        let r: usize = fields[0].parse().unwrap();
        let c: usize = fields[1].parse().unwrap();
        let logical: u8 = fields[2].parse().unwrap();
        let key = (
            r,
            c,
            logical,
            fields[3].to_owned(),
            fields[4].to_owned(),
            fields[5].to_owned(),
        );
        assert!(expected.contains(&key) && seen.insert(key));
        let decode = |s: &str| -> Vec<u64> {
            if s.is_empty() {
                vec![]
            } else {
                s.split(';')
                    .map(|s| {
                        assert_eq!(s.len(), 16);
                        u64::from_str_radix(s, 16).unwrap()
                    })
                    .collect()
            }
        };
        let input = decode(fields[6]);
        let output = decode(fields[7]);
        assert_eq!(input.len(), r * c);
        assert_eq!(output.len(), r * c);
        let indices: Vec<usize> = if fields[8].is_empty() {
            vec![]
        } else {
            fields[8].split(';').map(|s| s.parse().unwrap()).collect()
        };
        assert_eq!(indices.len(), r);
        let a: Array = if logical == 1 {
            assert!(input.iter().all(|&b| b == 0 || b == 1f64.to_bits()));
            LogicalMatrix::from_column_vec(
                r,
                c,
                input.iter().map(|&b| u8::from(b != 0)).collect(),
                &cancel,
            )
            .unwrap()
            .into()
        } else {
            Matrix::from_column_vec(r, c, input.iter().map(|&b| f64::from_bits(b)).collect())
                .unwrap()
                .into()
        };
        let columns = match fields[3] {
            "all" => None,
            "first" => Some(vec![1]),
            "mixed" => Some(vec![c as i32, -1]),
            _ => unreachable!(),
        };
        let direction = match fields[4] {
            "signed" => None,
            "ascend" => Some(O::Ascend),
            "descend" => Some(O::Descend),
            _ => unreachable!(),
        };
        let missing = match fields[5] {
            "auto" => M::Auto,
            "first" => M::First,
            "last" => M::Last,
            _ => unreachable!(),
        };
        for pool in &pools {
            for want in [false, true] {
                let (b, i) = pool
                    .install(|| a.sort_rows(columns.as_deref(), direction, missing, want, &cancel))
                    .unwrap();
                assert_eq!((b.rows(), b.cols(), b.is_logical()), (r, c, logical == 1));
                for (at, &bits) in output.iter().enumerate() {
                    assert_eq!(
                        b.scalar(at % r, at / r)
                            .unwrap()
                            .into_real()
                            .unwrap()
                            .to_bits(),
                        bits
                    );
                }
                if want {
                    let i = i.unwrap();
                    assert_eq!((i.rows(), i.cols()), (r, 1));
                    assert_eq!(
                        i.data(),
                        indices.iter().map(|&v| v as f64).collect::<Vec<_>>()
                    );
                } else {
                    assert!(i.is_none());
                }
            }
        }
    }
    assert_eq!(seen, expected);
    assert_eq!(seen.len(), 432);
}
