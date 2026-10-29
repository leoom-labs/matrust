use matrust::{
    Engine, Matrix,
    dsp::{self, Band},
};
use nalgebra::Complex;
use std::{f64::consts::PI, sync::atomic::AtomicBool};

fn response(sos: &Matrix, f: f64) -> Complex<f64> {
    let cancel = AtomicBool::new(false);
    (0..sos.rows()).fold(Complex::new(1., 0.), |product, row| {
        let c: [f64; 6] = std::array::from_fn(|column| sos.get(row, column));
        let h = dsp::response(&c[..3], &c[3..], &[f], &cancel).unwrap()[0];
        product * Complex::new(h[0], h[1])
    })
}

#[test]
fn butterworth_band_all_orders_stable_edge_gain_and_endpoints() {
    let cancel = AtomicBool::new(false);
    for order in 1..=20 {
        for edges in [[0.1, 0.4], [0.2, 0.201], [0.8, 0.95], [0.01, 0.99]] {
            let center = 2.
                * ((0.5 * PI * edges[0]).tan() * (0.5 * PI * edges[1]).tan())
                    .sqrt()
                    .atan()
                / PI;
            for band in [Band::Bandpass, Band::Bandstop] {
                let sos = dsp::butter_sos_with_cutoffs(order, &edges, band, &cancel).unwrap();
                assert_eq!((sos.rows(), sos.cols()), (order, 6));
                assert!(sos.data().iter().all(|x| x.is_finite()));
                for row in 0..order {
                    let a1 = sos.get(row, 4);
                    let a2 = sos.get(row, 5);
                    assert_eq!(sos.get(row, 3), 1.);
                    assert!(a2.abs() < 1. && 1. + a1 + a2 > 0. && 1. - a1 + a2 > 0.);
                }
                for edge in edges {
                    assert!(
                        (response(&sos, edge).norm() - std::f64::consts::FRAC_1_SQRT_2).abs()
                            < 2e-9,
                        "{order} {band:?} {edges:?}"
                    );
                }
                let (pass, stop) = if band == Band::Bandpass {
                    (center, 0.)
                } else {
                    (0., center)
                };
                assert!((response(&sos, pass).norm() - 1.).abs() < 2e-9);
                assert!(response(&sos, stop).norm() < 2e-9);
                assert!(
                    (response(&sos, 1.).norm() - if band == Band::Bandpass { 0. } else { 1. })
                        .abs()
                        < 2e-9
                );
            }
        }
    }
}

#[test]
fn butterworth_scalar_path_is_bit_identical_and_parameters_reject_atomically() {
    let cancel = AtomicBool::new(false);
    for order in 1..=20 {
        for band in [Band::Lowpass, Band::Highpass] {
            for cutoff in [0.01, 0.2, 0.8, 0.99] {
                let old = dsp::butter_sos(order, cutoff, band).unwrap();
                let new = dsp::butter_sos_with_cutoffs(order, &[cutoff], band, &cancel).unwrap();
                assert_eq!(
                    old.data().iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
                    new.data().iter().map(|x| x.to_bits()).collect::<Vec<_>>()
                );
            }
        }
    }
    for edges in [
        vec![],
        vec![0.2],
        vec![0.2, 0.4, 0.6],
        vec![0.4, 0.2],
        vec![0.2, 0.2],
        vec![0., 0.5],
        vec![0.2, 1.],
        vec![f64::NAN, 0.5],
        vec![0.2, f64::INFINITY],
    ] {
        assert!(dsp::butter_sos_with_cutoffs(4, &edges, Band::Bandpass, &cancel).is_err());
    }
    for n in [0, 21, usize::MAX] {
        assert!(dsp::butter_sos_with_cutoffs(n, &[0.2, 0.5], Band::Bandstop, &cancel).is_err());
    }
    assert!(dsp::butter_sos_with_cutoffs(20, &[1e-300, 2e-300], Band::Bandpass, &cancel).is_err());
    for edges in [
        [f64::from_bits(1), 0.4],
        [1e-300, 0.99],
        [0.4, f64::from_bits(1f64.to_bits() - 1)],
    ] {
        for band in [Band::Bandpass, Band::Bandstop] {
            assert!(
                dsp::butter_sos_with_cutoffs(20, &edges, band, &cancel).is_err(),
                "extreme endpoint must reject {edges:?} {band:?}"
            );
        }
    }
    assert!(
        dsp::butter_sos_with_cutoffs(20, &[0.2, 0.5], Band::Bandpass, &AtomicBool::new(true))
            .is_err()
    );
    let mut engine = Engine::default();
    engine.execute("saved=17;").unwrap();
    for script in [
        "saved=butter_sos(4,[0.3,0.2]);",
        "saved=butter_sos(4,[0.2,0.4],'low');",
        "saved=butter_sos(4,0.2,'stop');",
        "saved=butter_sos(4,[0.2,0.4;0.5,0.6]);",
    ] {
        assert!(engine.execute(script).is_err());
        assert_eq!(engine.variables["saved"].data(), &[17.]);
    }
    engine
        .execute("sos=butter_sos(3,[0.2;0.5]); stop=butter_sos(3,[0.2,0.5],'stop');")
        .unwrap();
    assert_eq!(
        engine.variables["sos"].data(),
        dsp::butter_sos_with_cutoffs(3, &[0.2, 0.5], Band::Bandpass, &cancel)
            .unwrap()
            .data()
    );
}

#[test]
fn butterworth_band_downstream_shared_pool_is_bit_deterministic() {
    let cancel = AtomicBool::new(false);
    let pools = [1, 8].map(|workers| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
    });
    let input = Matrix::from_column_vec(
        32768,
        8,
        (0..32768 * 8)
            .map(|i| ((i % 97) as f64 - 48.) / 48.)
            .collect(),
    )
    .unwrap();
    for band in [Band::Bandpass, Band::Bandstop] {
        let sos = dsp::butter_sos_with_cutoffs(10, &[0.1, 0.4], band, &cancel).unwrap();
        let results = pools.each_ref().map(|pool| {
            pool.install(|| dsp::sosfilt(&sos, &input, &cancel))
                .unwrap()
        });
        assert_eq!(
            results[0]
                .data()
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>(),
            results[1]
                .data()
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>()
        );
        assert!(results[0].data().iter().all(|x| x.is_finite()));
    }
}
