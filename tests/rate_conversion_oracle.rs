use matrust::{Matrix, dsp};
use std::{collections::BTreeSet, sync::atomic::AtomicBool};

#[test]
fn polyphase_matches_seventy_two_independent_full_convolution_references() {
    let fixture = include_str!("rate_conversion_oracle.csv");
    let mut lines = fixture.lines();
    assert_eq!(
        lines.next().unwrap(),
        "name,xrows,xcols,hrows,hcols,p,q,x,h,rows,cols,expected,tolerance"
    );
    let parse = |text: &str| {
        text.split(';')
            .map(|v| v.parse::<f64>().unwrap())
            .collect::<Vec<_>>()
    };
    let mut keys = BTreeSet::new();
    for line in lines {
        let fields = line.split(',').collect::<Vec<_>>();
        assert_eq!(fields.len(), 13);
        let int = |i: usize| fields[i].parse::<usize>().unwrap();
        assert!(keys.insert((fields[0].to_string(), int(5), int(6))));
        let x = Matrix::from_column_vec(int(1), int(2), parse(fields[7])).unwrap();
        let h = Matrix::from_column_vec(int(3), int(4), parse(fields[8])).unwrap();
        let expected = parse(fields[11]);
        let tolerance = fields[12].parse::<f64>().unwrap();
        let output = dsp::upfirdn(&x, &h, int(5), int(6), &AtomicBool::new(false)).unwrap();
        assert_eq!((output.rows(), output.cols()), (int(9), int(10)));
        assert_eq!(output.len(), expected.len());
        for (index, (&actual, &want)) in output.data().iter().zip(&expected).enumerate() {
            assert!(
                actual.is_finite() && (actual - want).abs() / (1. + want.abs()) <= tolerance,
                "{} p{} q{} sample{index}: {actual:?} vs {want:?}",
                fields[0],
                int(5),
                int(6)
            );
        }
    }
    let mut required = BTreeSet::new();
    for name in [
        "row",
        "column",
        "shared_filter",
        "filter_bank",
        "paired_bank",
        "singleton",
        "zeros",
        "cancellation",
        "large_finite",
    ] {
        for (p, q) in [
            (1, 1),
            (2, 1),
            (1, 3),
            (3, 2),
            (2, 4),
            (7, 3),
            (3, 7),
            (8, 9),
        ] {
            required.insert((name.to_string(), p, q));
        }
    }
    assert_eq!(keys, required);
}
