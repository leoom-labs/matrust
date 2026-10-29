use matrust::{
    Array, Complex64 as C, Engine, Matrix,
    dsp::{self, Band},
};
use std::{
    f64::consts::PI,
    sync::atomic::{AtomicBool, Ordering},
};

fn response(sos: &Matrix, f: f64) -> C {
    let z = C::from_polar(1., -PI * f);
    (0..sos.rows()).fold(C::new(1., 0.), |h, r| {
        let p = |offset| {
            C::new(sos.get(r, offset), 0.)
                + z * sos.get(r, offset + 1)
                + z * z * sos.get(r, offset + 2)
        };
        h * p(0) / p(3)
    })
}
fn close(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance * (1. + expected.abs()),
        "{actual} != {expected}"
    );
}
// Independent magnitude definition through T_n recurrence, not poles, section
// pairing, coefficient expansion or the production response implementation.
fn analytic(n: usize, rp: f64, edges: &[f64], band: Band, f: f64) -> f64 {
    let t = (PI * f / 2.).tan();
    let lo = (PI * edges[0] / 2.).tan();
    let hi = (PI * edges[edges.len() - 1] / 2.).tan();
    let x = match band {
        Band::Lowpass => t / lo,
        Band::Highpass => lo / t,
        Band::Bandpass => (t * t - lo * hi) / ((hi - lo) * t),
        Band::Bandstop => (hi - lo) * t / (lo * hi - t * t),
    };
    let (mut previous, mut current) = (1., x);
    for _ in 1..n {
        (previous, current) = (current, 2. * x * current - previous);
    }
    1. / (1. + (10f64.powf(rp / 10.) - 1.) * current * current).sqrt()
}
#[test]
fn all_orders_four_forms_match_independent_chebyshev_magnitude() {
    let cancel = AtomicBool::new(false);
    for n in 1..=20 {
        for rp in [0.1, 1., 6.] {
            for (band, edges) in [
                (Band::Lowpass, vec![0.3]),
                (Band::Highpass, vec![0.6]),
                (Band::Bandpass, vec![0.2, 0.65]),
                (Band::Bandstop, vec![0.2, 0.65]),
            ] {
                let design = dsp::cheby1(n, rp, &edges, band, &cancel).unwrap();
                let sos = design.sos(&cancel).unwrap();
                assert_eq!(design.order(), if edges.len() == 2 { 2 * n } else { n });
                assert_eq!((sos.rows(), sos.cols()), (design.order().div_ceil(2), 6));
                for r in 0..sos.rows() {
                    let (a, b) = (sos.get(r, 4), sos.get(r, 5));
                    assert_eq!(sos.get(r, 3), 1.);
                    assert!(b.abs() < 1. && 1. + a + b > 0. && 1. - a + b > 0.);
                }
                for f in [0.03, 0.11, 0.2, 0.3, 0.47, 0.6, 0.65, 0.83, 0.97] {
                    close(
                        response(&sos, f).norm(),
                        analytic(n, rp, &edges, band, f),
                        2e-8,
                    );
                }
                for edge in edges {
                    close(response(&sos, edge).norm(), 10f64.powf(-rp / 20.), 2e-8);
                }
            }
        }
    }
}
#[test]
fn narrow_bands_and_near_endpoint_designs_have_correct_gain_and_stable_poles() {
    let cancel = AtomicBool::new(false);
    for n in [1, 2, 7, 20] {
        for (band, edges) in [
            (Band::Lowpass, vec![0.01]),
            (Band::Highpass, vec![0.99]),
            (Band::Bandpass, vec![0.2, 0.201]),
            (Band::Bandstop, vec![0.2, 0.201]),
            (Band::Bandpass, vec![0.01, 0.99]),
            (Band::Bandstop, vec![0.01, 0.99]),
        ] {
            let d = dsp::cheby1(n, 1., &edges, band, &cancel).unwrap();
            let sos = d.sos(&cancel).unwrap();
            let (z, p, k) = d.zero_pole_gain(&cancel).unwrap();
            assert_eq!((p.rows(), p.cols()), (d.order(), 1));
            assert_eq!(z.len(), d.order());
            assert!(p.data().iter().all(|p| p.norm() < 1.));
            assert!(k > 0. && k.is_finite());
            for &edge in &edges {
                close(response(&sos, edge).norm(), 10f64.powf(-0.05), 2e-6);
            }
            let anchor = match band {
                Band::Highpass => 1.,
                Band::Bandpass => {
                    2. * ((PI * edges[0] / 2.).tan() * (PI * edges[1] / 2.).tan())
                        .sqrt()
                        .atan()
                        / PI
                }
                _ => 0.,
            };
            close(
                response(&sos, anchor).norm(),
                if n.is_multiple_of(2) {
                    10f64.powf(-0.05)
                } else {
                    1.
                },
                2e-6,
            );
        }
    }
}
#[test]
fn first_order_closed_forms_and_zpk_tf_complex_phase_agree() {
    let cancel = AtomicBool::new(false);
    let rp = 2.;
    let cutoff = 0.4;
    let epsilon = (10f64.powf(rp / 10.) - 1.).sqrt();
    let u = (PI * cutoff / 2.).tan();
    for band in [Band::Lowpass, Band::Highpass] {
        let d = dsp::cheby1(1, rp, &[cutoff], band, &cancel).unwrap();
        let (b, a) = d.transfer_function(&cancel).unwrap();
        let (gain, a1) = if band == Band::Lowpass {
            (u / (epsilon + u), (u - epsilon) / (u + epsilon))
        } else {
            (
                1. / (1. + u * epsilon),
                (u * epsilon - 1.) / (u * epsilon + 1.),
            )
        };
        close(b.data()[0], gain, 1e-14);
        close(
            b.data()[1],
            if band == Band::Lowpass { gain } else { -gain },
            1e-14,
        );
        close(a.data()[1], a1, 1e-14);
    }
    for n in 1..=4 {
        for band in [
            Band::Lowpass,
            Band::Highpass,
            Band::Bandpass,
            Band::Bandstop,
        ] {
            let edges = if matches!(band, Band::Bandpass | Band::Bandstop) {
                vec![0.2, 0.65]
            } else {
                vec![0.4]
            };
            let d = dsp::cheby1(n, 1., &edges, band, &cancel).unwrap();
            let sos = d.sos(&cancel).unwrap();
            let (b, a) = d.transfer_function(&cancel).unwrap();
            let (zeros, poles, k) = d.zero_pole_gain(&cancel).unwrap();
            assert_eq!((b.rows(), b.cols()), (1, d.order() + 1));
            assert_eq!(a.data()[0], 1.);
            for f in [0.04, 0.2, 0.43, 0.65, 0.96] {
                let z = C::from_polar(1., PI * f);
                let hz = zeros
                    .data()
                    .iter()
                    .zip(poles.data())
                    .fold(C::new(k, 0.), |v, (&zero, &pole)| {
                        v * (z - zero) / (z - pole)
                    });
                let value = dsp::response(b.data(), a.data(), &[f], &cancel).unwrap()[0];
                let hs = response(&sos, f);
                assert!((hz - hs).norm() < 2e-9);
                assert!((C::new(value[0], value[1]) - hs).norm() < 2e-8);
            }
        }
    }
}
#[test]
fn engine_outputs_defaults_complex_poles_shared_inputs_and_filter_use() {
    let completions = matrust::session::completions("cheby1", std::iter::empty());
    assert!(completions.contains(&"cheby1".to_owned()));
    assert!(completions.contains(&"cheby1_sos".to_owned()));
    let mut e = Engine::default();
    e.execute("Wp=[0.2;0.65];Alias=Wp;b=cheby1(3,1,Wp);[bb,a]=cheby1(3,1,Wp,'bandpass');[z,p,k]=cheby1(3,1,Wp);s=cheby1_sos(3,1,Wp);y=sosfilt(s,[1 0 0 0 0]);").unwrap();
    let get = |name| match e.array(name).unwrap() {
        Array::Numeric(a) => a,
        _ => panic!("real output"),
    };
    assert_eq!(get("b").data(), get("bb").data());
    assert_eq!((get("b").rows(), get("b").cols()), (1, 7));
    assert_eq!((get("s").rows(), get("s").cols()), (3, 6));
    assert_eq!((get("z").rows(), get("z").cols()), (6, 1));
    assert!(matches!(e.array("p"), Some(Array::Complex(_))));
    assert!(get("y").data().iter().all(|x| x.is_finite()));
    assert!(
        e.array("Wp")
            .unwrap()
            .same_snapshot(&e.array("Alias").unwrap())
    );
    e.execute("s1=cheby1_sos(3,1,0.4);s2=cheby1_sos(3,1,0.4,char([108 111 119]));")
        .unwrap();
    assert_eq!(e.variables["s1"].data(), e.variables["s2"].data());
}
#[test]
fn invalid_options_multiple_output_publication_and_cancel_are_atomic() {
    let mut e = Engine::default();
    e.execute("b=[7 8];a=b;Alias=b;").unwrap();
    let original = e.array("b").unwrap();
    for args in [
        "0,1,0.3",
        "21,1,0.3",
        "1.5,1,0.3",
        "3,0,0.3",
        "3,-1,0.3",
        "3,Inf,0.3",
        "3,1,0",
        "3,1,1",
        "3,1,[.4 .2]",
        "3,1,[.2 .4],'high'",
        "3,1,.3,'stop'",
        "3,1,.3,'s'",
        "3,1,.3,'ctf'",
        "3,1,.3,'low','s'",
        "3,1,1i",
        "true,1,.3",
        "3,1,{.3}",
    ] {
        assert!(
            e.execute(&format!("[b,a]=cheby1({args});")).is_err(),
            "{args}"
        );
        assert!(e.array("b").unwrap().same_snapshot(&original));
        assert!(e.array("a").unwrap().same_snapshot(&original));
    }
    assert!(e.execute("[b,a,z,p]=cheby1(3,1,.3);").is_err());
    let flag = e.cancellation();
    flag.store(true, Ordering::Relaxed);
    assert!(e.execute("[b,a]=cheby1(3,1,.3);").is_err());
    flag.store(false, Ordering::Relaxed);
    assert!(e.array("b").unwrap().same_snapshot(&original));
    e.execute("[b,a]=cheby1(3,1,.3);").unwrap();
    assert!(e.array("Alias").unwrap().same_snapshot(&original));
    let cancel = AtomicBool::new(false);
    let d = dsp::cheby1(4, 1., &[0.2, 0.6], Band::Bandpass, &cancel).unwrap();
    cancel.store(true, Ordering::Relaxed);
    assert!(d.sos(&cancel).is_err());
    assert!(d.transfer_function(&cancel).is_err());
    assert!(d.zero_pole_gain(&cancel).is_err());
    assert!(dsp::cheby1(4, 1., &[0.3], Band::Lowpass, &cancel).is_err());
}
#[test]
fn user_function_precedence_and_unsupported_outputs_precede_argument_effects() {
    let mut e = Engine::default();
    e.execute("[b,a]=cheby1(3,1,.3);\nfunction [x,y]=cheby1(n,r,w)\nx=n;y=r;\nend")
        .unwrap();
    match e.array("b").unwrap() {
        Array::Numeric(a) => assert_eq!(a.data(), &[3.]),
        _ => panic!(),
    }
    let mut e = Engine::default();
    let before = e.current_figure;
    assert!(e.figure.is_none());
    assert!(e.execute("[a,b,c,d]=cheby1(figure(9),1,.3);").is_err());
    assert_eq!(e.current_figure, before);
    assert!(e.figure.is_none());
}
