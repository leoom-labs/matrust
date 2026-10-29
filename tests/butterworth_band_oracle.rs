// Copy only alongside independently audited generated CSV in a separate layer.
use matrust::dsp::{self, Band};
use nalgebra::Complex;
use std::{collections::BTreeSet, sync::atomic::AtomicBool};
#[test]
fn butterworth_band_independent_scipy_poles_and_complex_response() {
    let cancel = AtomicBool::new(false);
    let mut seen = BTreeSet::new();
    let csv = include_str!("butterworth_band_oracle.csv");
    let mut lines = csv.lines();
    assert_eq!(
        lines.next().unwrap(),
        "order,band,lower,upper,frequencies,real,imag,pole_real,pole_imag,response_tolerance,pole_tolerance"
    );
    let orders = [1, 2, 3, 4, 5, 6, 8, 10, 16, 20];
    let edges: [[f64; 2]; 6] = [
        [0.05, 0.3],
        [0.3, 0.65],
        [0.01, 0.99],
        [0.2, 0.201],
        [0.8, 0.95],
        [0.45, 0.55],
    ];
    for line in lines {
        let f: Vec<_> = line.split(',').collect();
        assert_eq!(f.len(), 11);
        let n: usize = f[0].parse().unwrap();
        assert!(orders.contains(&n));
        let band = match f[1] {
            "bandpass" => Band::Bandpass,
            "bandstop" => Band::Bandstop,
            _ => panic!("band"),
        };
        let cuts = [f[2].parse::<f64>().unwrap(), f[3].parse::<f64>().unwrap()];
        assert!(edges.contains(&cuts));
        assert!(seen.insert((n, f[1].to_owned(), cuts[0].to_bits(), cuts[1].to_bits())));
        let parse = |s: &str| {
            s.split(';')
                .map(|v| v.parse::<f64>().unwrap())
                .collect::<Vec<_>>()
        };
        let frequencies = parse(f[4]);
        let re = parse(f[5]);
        let im = parse(f[6]);
        let pr = parse(f[7]);
        let pi = parse(f[8]);
        assert!(!frequencies.is_empty());
        assert!(
            frequencies
                .iter()
                .chain(&re)
                .chain(&im)
                .chain(&pr)
                .chain(&pi)
                .all(|x| x.is_finite())
        );
        assert!(frequencies.windows(2).all(|pair| pair[0] < pair[1]));
        let mut required: Vec<f64> = (0..=32).map(|i| i as f64 / 32.).chain(cuts).collect();
        required.sort_by(f64::total_cmp);
        required.dedup();
        assert!(
            required.iter().all(|x| frequencies.contains(x)),
            "Missing grid/edge sample"
        );
        let center = 2.
            * ((0.5 * std::f64::consts::PI * cuts[0]).tan()
                * (0.5 * std::f64::consts::PI * cuts[1]).tan())
            .sqrt()
            .atan()
            / std::f64::consts::PI;
        let extra: Vec<_> = frequencies
            .iter()
            .filter(|x| !required.contains(x))
            .collect();
        // Independent Python/Rust elementary functions may move the center by
        // a few ulps, including across exact grid equality. Only this optional
        // deduplication point has a tolerance; all33 grid points/edges are exact.
        assert!(extra.len() <= 1 && extra.iter().all(|x| (**x - center).abs() <= 4e-15));
        assert!(extra.len() == 1 || required.iter().any(|x| (*x - center).abs() <= 4e-15));
        assert_eq!(frequencies.len(), required.len() + extra.len());
        let tolerance: f64 = f[9].parse().unwrap();
        let pole_tolerance: f64 = f[10].parse().unwrap();
        assert_eq!(tolerance, 2e-9);
        assert_eq!(pole_tolerance, 2e-10);
        assert_eq!(frequencies.len(), re.len());
        assert_eq!(re.len(), im.len());
        assert_eq!(pr.len(), 2 * n);
        assert_eq!(pi.len(), 2 * n);
        let sos = dsp::butter_sos_with_cutoffs(n, &cuts, band, &cancel).unwrap();
        for (value, (&real, &imag)) in dsp::sos_response(&sos, &frequencies, &cancel)
            .unwrap()
            .iter()
            .zip(re.iter().zip(&im))
        {
            let error = (value[0] - real).hypot(value[1] - imag);
            assert!(
                error <= tolerance * (1. + real.hypot(imag)),
                "{n} {band:?} {cuts:?} error={error}"
            );
        }
        let mut expected: Vec<_> = pr
            .into_iter()
            .zip(pi)
            .map(|(r, i)| Complex::new(r, i))
            .collect();
        for row in 0..sos.rows() {
            let a1 = sos.get(row, 4);
            let a2 = sos.get(row, 5);
            let delta = Complex::new(a1 * a1 - 4. * a2, 0.).sqrt();
            for actual in [
                (-Complex::new(a1, 0.) + delta) * 0.5,
                (-Complex::new(a1, 0.) - delta) * 0.5,
            ] {
                let (at, error) = expected
                    .iter()
                    .enumerate()
                    .map(|(at, p)| (at, (actual - p).norm()))
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .unwrap();
                assert!(error <= pole_tolerance, "{n} {band:?} pole error={error}");
                expected.remove(at);
            }
        }
    }
    assert_eq!(seen.len(), 120);
}
