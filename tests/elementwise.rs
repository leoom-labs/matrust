use matrust::{Engine, Matrix};

type Kernel = fn(&Matrix) -> Matrix;
type Operation = (&'static str, Kernel, fn(f64) -> f64);
const OPS: [Operation; 8] = [
    ("tan", Matrix::tan, libm::tan),
    ("exp", Matrix::exp, libm::exp),
    ("log", Matrix::ln, libm::log),
    ("sqrt", Matrix::sqrt, f64::sqrt),
    ("abs", Matrix::abs, f64::abs),
    ("floor", Matrix::floor, f64::floor),
    ("ceil", Matrix::ceil, f64::ceil),
    ("round", Matrix::round, f64::round),
];

fn check(name: &str, x: f64, actual: f64, expected: f64) {
    if expected.is_nan() {
        assert!(actual.is_nan(), "{name}({x}) = {actual}, expected NaN");
    } else if expected == 0. || !expected.is_finite() || expected.abs() < f64::MIN_POSITIVE {
        assert_eq!(actual.to_bits(), expected.to_bits(), "{name}({x})");
    } else {
        let tolerance = if name == "log" {
            2e-15 * expected.abs().max(1.)
        } else {
            2e-15 * expected.abs()
        };
        assert!(
            (actual - expected).abs() <= tolerance,
            "{name}({x}) = {actual}, expected {expected}, relative error {}",
            (actual - expected) / expected
        );
    }
}

#[test]
fn transcendentals_match_scalar_over_dense_ranges_and_boundaries() {
    let mut angles: Vec<_> = (0..200_001)
        .map(|i| -8192. + i as f64 * 16384. / 200000.)
        .collect();
    for k in -5215..=5215 {
        let x = k as f64 * std::f64::consts::FRAC_PI_2;
        angles.extend([x.next_down(), x, x.next_up()]);
    }
    let mut exponents: Vec<_> = (0..200_001)
        .map(|i| -746. + i as f64 * 1457. / 200000.)
        .collect();
    for x in [
        -745.1332191019411_f64,
        -708.3964185322641,
        -700.,
        0.,
        700.,
        709.782712893384,
    ] {
        exponents.extend([x.next_down(), x, x.next_up()]);
    }
    let mut positive: Vec<_> = (0..200_001)
        .map(|i| libm::exp(-744. + i as f64 * 1453. / 200000.))
        .collect();
    for k in -1074..=1023 {
        let x = libm::scalbn(1., k);
        positive.extend([x.next_down(), x, x.next_up()]);
    }
    for (name, kernel, reference, values) in [
        (
            "tan",
            Matrix::tan as Kernel,
            libm::tan as fn(f64) -> f64,
            angles,
        ),
        ("exp", Matrix::exp, libm::exp, exponents),
        ("log", Matrix::ln, libm::log, positive),
    ] {
        let input = Matrix::from_column_vec(1, values.len(), values).unwrap();
        let output = kernel(&input);
        for (&x, &actual) in input.data().iter().zip(output.data()) {
            check(name, x, actual, reference(x));
        }
    }
}

#[test]
fn elementwise_special_values_tails_and_threads_preserve_semantics() {
    let special = [
        0.,
        -0.,
        f64::from_bits(1),
        -f64::from_bits(1),
        f64::MIN_POSITIVE,
        0.5,
        -0.5,
        1.5,
        -1.5,
        1_f64.next_down(),
        1.,
        1_f64.next_up(),
        -745.,
        -710.,
        -700.,
        700.,
        710.,
        8192.,
        8192_f64.next_up(),
        1e20,
        -1e100,
        f64::MAX,
        -f64::MAX,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
    ];
    let serial = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let parallel = rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .build()
        .unwrap();
    for len in [
        0, 1, 31, 32, 33, 8191, 8192, 8193, 65535, 65536, 65537, 262143, 262144, 262145,
    ] {
        let input =
            Matrix::from_column_vec(1, len, special.into_iter().cycle().take(len).collect())
                .unwrap();
        for (name, kernel, reference) in OPS {
            let a = serial.install(|| kernel(&input));
            let b = parallel.install(|| kernel(&input));
            assert_eq!((a.rows(), a.cols()), (1, len));
            for ((&x, &actual), &other) in input.data().iter().zip(a.data()).zip(b.data()) {
                check(name, x, actual, reference(x));
                assert_eq!(
                    actual.to_bits(),
                    other.to_bits(),
                    "{name} differs across threads"
                );
            }
        }
    }
    // Verify vectorized lanes separately from mixed groups requiring scalar fallback.
    for x in special {
        let input = Matrix::filled(3, 11, x).unwrap();
        for (name, kernel, reference) in OPS {
            let actual = kernel(&input);
            assert_eq!((actual.rows(), actual.cols()), (3, 11));
            for &v in actual.data() {
                check(name, x, v, reference(x));
            }
        }
    }
}

#[test]
fn script_math_dispatch_preserves_rounding_and_large_angle_behavior() {
    let mut engine = Engine::default();
    engine.execute("x=[-1.5 -0.5 0.5 1.5]; r=round(x); t=tan(1e20); e=exp(710); l=log(0); s=sqrt(4); a=abs(-2); f=floor(-0.5); c=ceil(-0.5);").unwrap();
    assert_eq!(engine.variables["r"].data(), &[-2., -1., 1., 2.]);
    assert!((engine.variables["t"].data()[0] - libm::tan(1e20)).abs() < 1e-15);
    assert_eq!(engine.variables["e"].data()[0], f64::INFINITY);
    assert_eq!(engine.variables["l"].data()[0], f64::NEG_INFINITY);
    assert_eq!(engine.variables["s"].data()[0], 2.);
    assert_eq!(engine.variables["a"].data()[0], 2.);
    assert_eq!(engine.variables["f"].data()[0], -1.);
    assert_eq!(
        engine.variables["c"].data()[0].to_bits(),
        (-0_f64).to_bits()
    );
}
