use matrust::matrix::{Dispersion, StatisticsAxis};
use matrust::{Engine, Matrix};
use std::sync::atomic::AtomicBool;

fn reduce(values: &[f64], kind: Dispersion, population: bool) -> f64 {
    Matrix::from_column_vec(values.len(), 1, values.to_vec())
        .unwrap()
        .dispersion(
            kind,
            population,
            StatisticsAxis::Default,
            false,
            &AtomicBool::new(false),
        )
        .unwrap()
        .data()[0]
}

#[test]
fn exact_offset_extreme_range_and_subnormal_dispersion() {
    for offset in [0., 2.0f64.powi(52)] {
        let x = [offset, offset + 1., offset + 2.];
        assert_eq!(reduce(&x, Dispersion::Variance, false), 1.);
        assert_eq!(reduce(&x, Dispersion::StandardDeviation, false), 1.);
    }
    let base = 2.0f64.powi(500);
    let step = 2.0f64.powi(448);
    assert_eq!(
        reduce(
            &[base, base + step, base + 2. * step],
            Dispersion::Variance,
            false
        ),
        2.0f64.powi(896)
    );
    assert_eq!(
        reduce(
            &[base, base + step, base + 2. * step],
            Dispersion::StandardDeviation,
            false
        ),
        step
    );
    assert_eq!(
        reduce(&[f64::MAX, f64::MAX], Dispersion::Variance, false),
        0.
    );
    assert!(reduce(&[-f64::MAX, f64::MAX], Dispersion::Variance, true).is_infinite());
    assert_eq!(
        reduce(&[-f64::MAX, f64::MAX], Dispersion::StandardDeviation, true),
        f64::MAX
    );
    let tiny = f64::from_bits(1);
    assert_eq!(
        reduce(&[0., tiny], Dispersion::StandardDeviation, true).to_bits(),
        0
    );
    assert_eq!(
        reduce(&[0., tiny], Dispersion::StandardDeviation, false).to_bits(),
        1
    );
    assert_eq!(
        reduce(&[tiny, 2. * tiny, 3. * tiny], Dispersion::Variance, false),
        0.
    );
    assert_eq!(
        reduce(
            &[tiny, 2. * tiny, 3. * tiny],
            Dispersion::StandardDeviation,
            false
        )
        .to_bits(),
        tiny.to_bits()
    );
    assert_eq!(
        reduce(&[0., 2.0f64.powi(-536)], Dispersion::Variance, false).to_bits(),
        2
    );
    assert_eq!(
        reduce(&[-0., 0.], Dispersion::StandardDeviation, false).to_bits(),
        0
    );
}

#[test]
fn script_shapes_normalizations_missing_values_and_explicit_rejections() {
    let mut e = Engine::default();
    e.execute("A=[1 10;3 14;5 18]; V=var(A); S=std(A); R=var(A,1,2); T=var(A,[],1); G=var(A,0,'all'); O=std([1 NaN 3],0,'omitnan'); N=var([1 NaN 3]); E=var([]); X=var(zeros(0,3)); Z=std(A,0,3);").unwrap();
    assert_eq!(e.variables["V"].data(), &[4., 16.]);
    assert_eq!(e.variables["S"].data(), &[2., 4.]);
    assert_eq!((e.variables["R"].rows(), e.variables["R"].cols()), (3, 1));
    assert_eq!(e.variables["R"].data(), &[20.25, 30.25, 42.25]);
    assert_eq!(e.variables["T"].data(), &[4., 16.]);
    assert!((e.variables["G"].data()[0] - 443. / 10.).abs() < 1e-13);
    assert!((e.variables["O"].data()[0] - 2.0f64.sqrt()).abs() < 1e-15);
    assert!(e.variables["N"].data()[0].is_nan());
    assert_eq!((e.variables["E"].rows(), e.variables["E"].cols()), (1, 1));
    assert!(e.variables["E"].data()[0].is_nan());
    assert_eq!((e.variables["X"].rows(), e.variables["X"].cols()), (1, 3));
    assert!(e.variables["X"].data().iter().all(|x| x.is_nan()));
    assert_eq!(e.variables["Z"].data(), &[0.; 6]);
    let original = e.variables["A"].data().as_ptr();
    e.execute("keep=42;").unwrap();
    for call in [
        "var(A,-1)",
        "std(A,2)",
        "std(A,[1 1 1])",
        "var(A,0,0)",
        "var(A,0,[1 2])",
        "std(A,0,Inf)",
        "var(A,0,'all','unknown')",
        "std(A,'all')",
    ] {
        assert!(e.execute(&format!("keep={call};")).is_err(), "{call}");
        assert_eq!(e.variables["keep"].data(), &[42.]);
        assert_eq!(e.variables["A"].data().as_ptr(), original);
    }
}

#[test]
fn singleton_nonfinite_and_empty_axis_policies() {
    for kind in [Dispersion::Variance, Dispersion::StandardDeviation] {
        for population in [false, true] {
            assert_eq!(reduce(&[5.], kind, population), 0.);
            for values in [
                &[f64::NAN][..],
                &[f64::INFINITY][..],
                &[f64::INFINITY, 2.][..],
                &[][..],
            ] {
                assert!(reduce(values, kind, population).is_nan());
            }
            for (rows, cols, dim, expected) in [
                (0, 3, 1, (1, 3)),
                (3, 0, 1, (1, 0)),
                (3, 0, 2, (3, 1)),
                (0, 3, 2, (0, 1)),
                (0, 0, 1, (1, 0)),
                (0, 0, 3, (0, 0)),
            ] {
                let result = Matrix::filled(rows, cols, 0.)
                    .unwrap()
                    .dispersion(
                        kind,
                        population,
                        StatisticsAxis::Dimension(dim),
                        true,
                        &AtomicBool::new(false),
                    )
                    .unwrap();
                assert_eq!((result.rows(), result.cols()), expected);
                assert!(result.data().iter().all(|v| v.is_nan()));
            }
        }
    }
}

#[test]
fn independent_lanes_are_bit_identical_on_one_and_eight_workers() {
    let one = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let eight = rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .build()
        .unwrap();
    let rows = 65536;
    let cols = 8;
    let input = Matrix::from_column_vec(
        rows,
        cols,
        (0..rows * cols)
            .map(|i| 2.0f64.powi(40) + (i % 29) as f64 / 8.)
            .collect(),
    )
    .unwrap();
    let pointer = input.data().as_ptr();
    for axis in [
        StatisticsAxis::Dimension(1),
        StatisticsAxis::Dimension(2),
        StatisticsAxis::All,
    ] {
        for kind in [Dispersion::Variance, Dispersion::StandardDeviation] {
            let operation = || {
                input
                    .dispersion(kind, false, axis, false, &AtomicBool::new(false))
                    .unwrap()
            };
            let a = one.install(operation);
            let b = eight.install(operation);
            assert_eq!(
                a.data().iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                b.data().iter().map(|v| v.to_bits()).collect::<Vec<_>>()
            );
        }
    }
    assert_eq!(input.data().as_ptr(), pointer);
    assert!(
        input
            .dispersion(
                Dispersion::Variance,
                false,
                StatisticsAxis::All,
                false,
                &AtomicBool::new(true)
            )
            .unwrap_err()
            .contains("cancelled")
    );
}

#[test]
fn exact_rational_reference_fixture_all_rows() {
    // Parent must generate and copy the independently reviewed reference CSV
    // from research/statistics-functions/reference before integrating this test.
    let csv = include_str!("statistics_oracle.csv");
    let number = |text: &str| match text {
        "nan" => f64::NAN,
        "inf" => f64::INFINITY,
        "-inf" => f64::NEG_INFINITY,
        _ => text.parse::<f64>().unwrap(),
    };
    let values = |text: &str| {
        if text.is_empty() {
            Vec::new()
        } else {
            text.split(';').map(number).collect::<Vec<_>>()
        }
    };
    let mut count = 0;
    for line in csv.lines().skip(1) {
        let fields: Vec<_> = line.split(',').collect();
        assert_eq!(fields.len(), 11);
        let rows = fields[1].parse().unwrap();
        let cols = fields[2].parse().unwrap();
        let input = Matrix::from_column_vec(rows, cols, values(fields[7])).unwrap();
        let axis = match fields[3] {
            "default" => StatisticsAxis::Default,
            "all" => StatisticsAxis::All,
            text => StatisticsAxis::Dimension(text.parse().unwrap()),
        };
        let kind = if fields[6] == "std" {
            Dispersion::StandardDeviation
        } else {
            Dispersion::Variance
        };
        let actual = input
            .dispersion(
                kind,
                fields[4] == "1",
                axis,
                fields[5] == "1",
                &AtomicBool::new(false),
            )
            .unwrap();
        assert_eq!(
            (actual.rows(), actual.cols()),
            (fields[8].parse().unwrap(), fields[9].parse().unwrap())
        );
        let expected = values(fields[10]);
        assert_eq!(actual.len(), expected.len());
        for (index, (&a, &e)) in actual.data().iter().zip(&expected).enumerate() {
            if e.is_nan() {
                assert!(a.is_nan(), "{} {index}: {a}", fields[0]);
            } else if e.is_infinite() {
                assert_eq!(a, e);
            } else {
                assert!(
                    a.is_finite() && (a - e).abs() <= (2e-14 * e.abs()).max(4. * f64::from_bits(1)),
                    "{} axis{} pop{} omit{} {} index{index}: actual{a:e} expected{e:e}",
                    fields[0],
                    fields[3],
                    fields[4],
                    fields[5],
                    fields[6]
                );
            }
        }
        count += 1;
    }
    assert_eq!(count, 760);
}
