use matrust::{Array, Engine};
use serde_json::Value as Json;
use std::{collections::BTreeMap, fs};

fn fixture() -> BTreeMap<String, Json> {
    serde_json::from_str(&fs::read_to_string("tests/fixtures/signal/iir_design.json").unwrap())
        .unwrap()
}
fn numeric(engine: &Engine, name: &str) -> Vec<f64> {
    match engine.workspace().get(name).unwrap() {
        Array::Numeric(m) => m.data().to_vec(),
        v => panic!("{name} is not numeric: {v:?}"),
    }
}
fn js_vec(v: &Json) -> Vec<f64> {
    match v {
        Json::Array(xs) => xs.iter().map(|x| x.as_f64().unwrap()).collect(),
        Json::Number(x) => vec![x.as_f64().unwrap()],
        _ => panic!("expected a numeric scalar or vector fixture, got {v}"),
    }
}
fn close(actual: &[f64], expected: &[f64], label: &str) {
    let scale = expected.iter().map(|x| x.abs()).fold(0.0, f64::max);
    assert_eq!(actual.len(), expected.len(), "{label} length");
    for (i, (&a, &e)) in actual.iter().zip(expected).enumerate() {
        // SciPy's polynomial products can leave roundoff at structural zeros
        // (e.g. the middle coefficient of an odd-order Chebyshev II bandpass
        // numerator). Only values within eight ulps of the vector scale get
        // an absolute allowance. All resolved coefficients, including small
        // analog coefficients, retain the requested relative tolerance.
        let tolerance = if e.abs() <= 8. * f64::EPSILON * scale {
            64. * f64::EPSILON * scale
        } else {
            1e-10 * e.abs()
        };
        assert!(
            (a - e).abs() <= tolerance,
            "{label}[{i}] {a} != {e} (tol {tolerance})"
        );
    }
}
// SciPy bandstop order selection optimizes the pass edges with fminbound
// (xtol=1e-5). Our exact symmetric stop-edge transform can differ within
// that optimizer tolerance; selected integer orders must still match exactly.
fn close_order_edges(actual: &[f64], expected: &[f64], label: &str) {
    assert_eq!(actual.len(), expected.len(), "{label} length");
    for (i, (&a, &e)) in actual.iter().zip(expected).enumerate() {
        assert!(
            (a - e).abs() <= 2e-5 * e.abs().max(1.0),
            "{label}[{i}] {a} != {e} (2e-5 relative edge tolerance)"
        );
    }
}
fn design_expr(name: &str, n: usize, band: &str, mode: &str) -> String {
    let w: String = match band {
        "low" => "0.2".into(),
        "high" => "0.6".into(),
        "bandpass" | "stop" => "[0.2 0.65]".into(),
        _ => unreachable!(),
    };
    match name {
        "butter" => format!(
            "butter({n}, {w}, '{}'{})",
            if band == "stop" {
                "stop"
            } else if band == "bandpass" {
                "bandpass"
            } else if band == "high" {
                "high"
            } else {
                "low"
            },
            if mode == "s" { ", 's'" } else { "" }
        ),
        "cheby1" => format!(
            "cheby1({n}, 1.25, {w}, '{}'{})",
            if band == "stop" {
                "stop"
            } else if band == "bandpass" {
                "bandpass"
            } else if band == "high" {
                "high"
            } else {
                "low"
            },
            if mode == "s" { ", 's'" } else { "" }
        ),
        "cheby2" => format!(
            "cheby2({n}, 36, {w}, '{}'{})",
            if band == "stop" {
                "stop"
            } else if band == "bandpass" {
                "bandpass"
            } else if band == "high" {
                "high"
            } else {
                "low"
            },
            if mode == "s" { ", 's'" } else { "" }
        ),
        "ellip" => format!(
            "ellip({n}, 1.25, 36, {w}, '{}'{})",
            if band == "stop" {
                "stop"
            } else if band == "bandpass" {
                "bandpass"
            } else if band == "high" {
                "high"
            } else {
                "low"
            },
            if mode == "s" { ", 's'" } else { "" }
        ),
        "besself" => format!("besself({n}, 1)"),
        _ => unreachable!(),
    }
}

#[test]
fn design_coefficients_match_committed_scipy_fixtures() {
    let refs = fixture();
    for name in ["butter", "cheby1", "cheby2", "ellip", "besself"] {
        for n in [1, 2, 5, 8] {
            for band in ["low", "high", "bandpass", "stop"] {
                if name == "besself" && band != "low" {
                    continue;
                }
                let modes: &[&str] = if name == "besself" {
                    &["s"]
                } else if name == "cheby1" {
                    &["digital"]
                } else {
                    &["digital", "s"]
                };
                for &mode in modes {
                    let key = format!("{name}:{n}:{band}:{mode}");
                    let Some(r) = refs.get(&key) else {
                        panic!("missing {key}")
                    };
                    let mut e = Engine::default();
                    let cmd = format!("[b,a] = {};", design_expr(name, n, band, mode));
                    e.execute(&cmd).unwrap_or_else(|x| panic!("{key}: {x}"));
                    let mut expected_b = js_vec(&r["b"]);
                    if name != "cheby1" {
                        let mut padded = vec![0.; js_vec(&r["a"]).len() - expected_b.len()];
                        padded.extend(expected_b);
                        expected_b = padded;
                    }
                    close(&numeric(&e, "b"), &expected_b, &format!("{key} b"));
                    close(&numeric(&e, "a"), &js_vec(&r["a"]), &format!("{key} a"));
                }
            }
        }
    }
    let r = &refs["butter:20:low:digital"];
    let mut e = Engine::default();
    e.execute("[b,a]=butter(20,0.32);").unwrap();
    close(&numeric(&e, "b"), &js_vec(&r["b"]), "order20 b");
    close(&numeric(&e, "a"), &js_vec(&r["a"]), "order20 a");
}

#[test]
fn order_selection_matches_scipy_fixtures() {
    let refs = fixture();
    let specs = [
        ("low", "0.2", "0.35"),
        ("high", "0.6", "0.4"),
        ("bandpass", "[0.2 0.5]", "[0.1 0.65]"),
        ("stop", "[0.15 0.65]", "[0.25 0.5]"),
    ];
    for name in ["buttord", "cheb1ord", "cheb2ord", "ellipord"] {
        for (band, wp, ws) in specs {
            let key = format!("{name}:{band}");
            let r = &refs[&key];
            let mut e = Engine::default();
            e.execute(&format!("[n,Wn]={name}({wp},{ws},1.25,36);"))
                .unwrap_or_else(|x| panic!("{key}: {x}"));
            assert_eq!(numeric(&e, "n")[0], r["n"].as_f64().unwrap(), "{key} order");
            if band == "stop" {
                close_order_edges(&numeric(&e, "Wn"), &js_vec(&r["wn"]), &format!("{key} Wn"));
            } else {
                close(&numeric(&e, "Wn"), &js_vec(&r["wn"]), &format!("{key} Wn"));
            }
        }
    }
}

#[test]
fn transfer_zpk_sos_and_state_space_conversions_round_trip() {
    let mut e = Engine::default();
    e.execute("b=[0.2 0.4 0.2]; a=[1 -0.3 0.1]; [z,p,k]=tf2zp(b,a); [b1,a1]=zp2tf(z,p,k); s=zp2sos(z,p,k); [b2,a2]=sos2tf(s); [z2,p2,k2]=sos2zp(s); [b3,a3]=zp2tf(z2,p2,k2); [A,B,C,D]=tf2ss(b,a); [b4,a4]=ss2tf(A,B,C,D); [z3,p3,k3]=ss2zp(A,B,C,D); [A2,B2,C2,D2]=zp2ss(z3,p3,k3); [b5,a5]=ss2tf(A2,B2,C2,D2); s2=tf2sos(b,a); [b6,a6]=sos2tf(s2);").unwrap();
    for (b, a) in [
        ("b1", "a1"),
        ("b2", "a2"),
        ("b3", "a3"),
        ("b4", "a4"),
        ("b5", "a5"),
        ("b6", "a6"),
    ] {
        close(&numeric(&e, b), &[0.2, 0.4, 0.2], b);
        close(&numeric(&e, a), &[1., -0.3, 0.1], a);
    }
    let refs = fixture();
    let rows = refs["conversion"]["sos"].as_array().unwrap();
    let expected = (0..6)
        .flat_map(|j| rows.iter().map(move |row| row[j].as_f64().unwrap()))
        .collect::<Vec<_>>();
    let actual = numeric(&e, "s");
    close(&actual, &expected, "nearest SOS coefficients");
}

#[test]
fn rejects_invalid_specifications_and_order_selection_edges() {
    for script in [
        "butter(0,0.2)",
        "butter(3,0)",
        "butter(3,1)",
        "ellip(4,4,3,0.2)",
        "cheby2(0,40,0.2)",
        "besself(26,1)",
        "butter(101,0.2)",
        "ellipord(0.2,0.3,40,20)",
    ] {
        let mut e = Engine::default();
        assert!(e.execute(script).is_err(), "accepted {script}");
    }
}

#[test]
#[ignore = "release timing probe; run with --ignored --nocapture"]
fn iir_design_timing() {
    use rayon::prelude::*;
    use std::time::Instant;
    println!(
        "arch={} workers_available={}",
        std::env::consts::ARCH,
        std::thread::available_parallelism().map_or(1, usize::from)
    );
    for workers in [1, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        for order in [5, 12, 20] {
            for (case, call) in [
                (
                    "ellip_bandpass",
                    format!("[b,a]=ellip({order},1,40,[0.2 0.65],'bandpass')"),
                ),
                ("bessel_delay", format!("[z,p,k]=besself({order},1)")),
                ("butter_sos", format!("[s,g]=butter({order},0.2,'sos')")),
            ] {
                let start = Instant::now();
                pool.install(|| {
                    (0..workers).into_par_iter().for_each(|_| {
                        let mut engine = Engine::default();
                        let reps = 100 / workers;
                        let script = format!("for i=1:{reps}; {call}; end");
                        std::hint::black_box(engine.execute(&script).unwrap());
                    });
                });
                println!(
                    "{case} order={order} repetitions=100 workers={workers} elapsed_ms={:.3}",
                    start.elapsed().as_secs_f64() * 1e3
                );
            }
        }
    }
}

fn shape(e: &Engine, name: &str) -> (usize, usize) {
    match e.workspace().get(name).unwrap() {
        Array::Numeric(m) => (m.rows(), m.cols()),
        v => panic!("unexpected {name}: {v:?}"),
    }
}
fn complex_values(e: &Engine, name: &str) -> Vec<matrust::Complex64> {
    match e.workspace().get(name).unwrap() {
        Array::Complex(m) => m.data().to_vec(),
        Array::Numeric(m) => m
            .data()
            .iter()
            .map(|x| matrust::Complex64::new(*x, 0.))
            .collect(),
        v => panic!("unexpected {v:?}"),
    }
}
fn root_fixture(e: &Engine, name: &str, expected: &Json, tolerance: f64) {
    let actual = complex_values(e, name);
    let mut remaining = expected
        .as_array()
        .unwrap()
        .iter()
        .map(|x| {
            if let Some(v) = x.as_f64() {
                matrust::Complex64::new(v, 0.)
            } else {
                matrust::Complex64::new(x[0].as_f64().unwrap(), x[1].as_f64().unwrap())
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(actual.len(), remaining.len());
    for r in actual {
        let (i, err) = remaining
            .iter()
            .enumerate()
            .map(|(i, q)| (i, (r - q).norm() / q.norm().max(1.)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        assert!(
            err <= tolerance,
            "{name}: {r} vs {} relative error {err}",
            remaining[i]
        );
        remaining.remove(i);
    }
}
#[test]
fn bessel_high_orders_are_stable_and_match_scipy_delay_normalization() {
    let refs = fixture();
    for n in 11..=20 {
        let mut e = Engine::default();
        e.execute(&format!("[z,p,k]=besself({n},1); [b,a]=besself({n},1);"))
            .unwrap();
        assert!(complex_values(&e, "p").iter().all(|p| p.re < 0.));
        assert_eq!(shape(&e, "b"), (1, n + 1));
        assert_eq!(shape(&e, "a"), (1, n + 1));
        if n != 12 && n != 20 {
            continue;
        }
        // Polynomial-root conditioning grows sharply at n=20. This checks
        // individual roots at 2e-5 relative, separately from coefficients.
        root_fixture(&e, "p", &refs[&format!("bessel_poles:{n}")]["p"], 2e-5);
        close(
            &numeric(&e, "k"),
            &js_vec(&refs[&format!("bessel_poles:{n}")]["k"]),
            "Bessel gain",
        );
    }
}
#[test]
fn row_coefficients_order_edges_and_all_output_prefixes() {
    let mut e = Engine::default();
    for name in [
        "butter(4,0.2)",
        "cheby2(4,40,0.2)",
        "ellip(4,1,40,0.2)",
        "besself(4,1)",
    ] {
        e.execute(&format!(
            "[b,a]={name}; b1={name}; [z,p,k]={name}; [A,B,C,D]={name};"
        ))
        .unwrap();
        assert_eq!(shape(&e, "b"), (1, 5));
        assert_eq!(shape(&e, "a"), (1, 5));
        close(&numeric(&e, "b1"), &numeric(&e, "b"), name);
    }
    for name in ["buttord", "cheb1ord", "cheb2ord", "ellipord"] {
        e.execute(&format!(
            "[n,Wn]={name}([0.2 0.5],[0.1 0.65],1,40); n1={name}([0.2 0.5],[0.1 0.65],1,40);"
        ))
        .unwrap();
        assert_eq!(shape(&e, "Wn"), (1, 2));
        assert_eq!(numeric(&e, "n1"), numeric(&e, "n"));
    }
    let calls = [
        ("tf2zp([1 2],[1 3 2])", 3),
        ("zp2tf([],[-1 -2],1)", 2),
        ("sos2zp([1 2 1 1 -0.5 0.2])", 3),
        ("sos2tf([1 2 1 1 -0.5 0.2])", 2),
        ("tf2ss([1 2],[1 3 2])", 4),
        ("zp2ss([],[-1 -2],1)", 4),
        ("ss2tf(A,B,C,D)", 2),
        ("ss2zp(A,B,C,D)", 3),
    ];
    e.execute("[A,B,C,D]=tf2ss([1 2],[1 3 2]);").unwrap();
    for (call, max) in calls {
        for count in 1..=max {
            let names = (0..count)
                .map(|i| format!("o{i}"))
                .collect::<Vec<_>>()
                .join(",");
            e.execute(&format!("[{names}]={call};"))
                .unwrap_or_else(|err| panic!("{call}, {count}: {err}"));
        }
    }
    for call in [
        "butter(4,0.2,'sos')",
        "cheby2(4,40,0.2,'sos')",
        "ellip(4,1,40,0.2,'sos')",
        "zp2sos([],[-1 -2],2)",
        "tf2sos([2],[1 3 2])",
    ] {
        e.execute(&format!(
            "s1={call}; [s,g]={call}; [b,a]=sos2tf(s); [b1,a1]=sos2tf(s1); b=b*g;"
        ))
        .unwrap();
        close(&numeric(&e, "b"), &numeric(&e, "b1"), call);
        close(&numeric(&e, "a"), &numeric(&e, "a1"), call);
    }
}
#[test]
fn all_iir_names_are_registered_and_their_help_examples_run() {
    let names = [
        "butter", "cheby2", "ellip", "besself", "buttord", "cheb1ord", "cheb2ord", "ellipord",
        "zp2tf", "tf2zp", "zp2sos", "sos2zp", "tf2sos", "sos2tf", "zp2ss", "ss2zp", "tf2ss",
        "ss2tf",
    ];
    let help = fs::read_to_string("assets/help/functions.md").unwrap();
    for name in names {
        assert!(matrust::session::FUNCTIONS.contains(&name));
        let mut e = Engine::default();
        e.execute(&format!("v=exist('{name}');")).unwrap();
        assert_eq!(numeric(&e, "v"), [5.]);
        let page = help
            .split(&format!("## {name}\n"))
            .nth(1)
            .unwrap()
            .split("\n## ")
            .next()
            .unwrap();
        for example in page
            .lines()
            .filter_map(|line| line.strip_prefix("Example: "))
        {
            Engine::default()
                .execute(example)
                .unwrap_or_else(|err| panic!("{name}: {err}"));
        }
    }
}
#[test]
fn empty_zeros_leading_sos_zero_and_unpaired_roots() {
    let mut e = Engine::default();
    e.execute("[b,a]=zp2tf([],[-1 -2],1); [bs,as]=sos2tf([0 1 0 1 -0.5 0]); [z,p,k]=sos2zp([0 1 0 1 -0.5 0]);").unwrap();
    close(&numeric(&e, "b"), &[0., 0., 1.], "all pole numerator");
    close(&numeric(&e, "bs"), &[0., 1., 0.], "leading SOS zero");
    close(&numeric(&e, "as"), &[1., -0.5, 0.], "SOS denominator");
    assert_eq!(numeric(&e, "k"), [1.]);
    for script in ["zp2sos(0.3,[0.5+0.5i;0.2],1)", "zp2sos([0.2+0.3i],0.5,1)"] {
        assert!(e.execute(script).unwrap_err().contains("conjugate"));
    }
}
#[test]
fn sos_sections_match_scipy_nearest_order_and_pairing() {
    let refs = fixture();
    for n in [3, 4, 5, 8] {
        for name in ["butter", "cheby2", "ellip"] {
            let args = match name {
                "butter" => format!("{n},0.2"),
                "cheby2" => format!("{n},40,0.2"),
                _ => format!("{n},1,40,0.2"),
            };
            let mut e = Engine::default();
            e.execute(&format!("s={name}({args},'sos');")).unwrap();
            let rows = refs[&format!("sos:{name}:{n}")].as_array().unwrap();
            let expected = (0..6)
                .flat_map(|j| rows.iter().map(move |r| r[j].as_f64().unwrap()))
                .collect::<Vec<_>>();
            close(
                &numeric(&e, "s"),
                &expected,
                &format!("{name} {n} nearest SOS"),
            );
        }
    }
    for (label, z, p) in [
        (
            "mixed",
            "[-1 -0.5+0.5i -0.5-0.5i]",
            "[0.75 0.8+0.1i 0.8-0.1i]",
        ),
        ("outside", "[-1 -2 -3 -4]", "[1.1 2 0.9 0.2]"),
        (
            "real_complex",
            "[0.8+0.1i 0.8-0.1i -1 -2]",
            "[0.9 0.8 0.4 0.3]",
        ),
    ] {
        let mut e = Engine::default();
        e.execute(&format!("s=zp2sos({z},{p},2);")).unwrap();
        let rows = refs[&format!("sos_custom:{label}")]["sos"]
            .as_array()
            .unwrap();
        let expected = (0..6)
            .flat_map(|j| rows.iter().map(move |r| r[j].as_f64().unwrap()))
            .collect::<Vec<_>>();
        close(&numeric(&e, "s"), &expected, label);
    }
}
#[test]
fn elliptic_high_attenuation_matches_scipy_at_one_part_in_a_billion() {
    let refs = fixture();
    for analog in [false, true] {
        let mut e = Engine::default();
        let call = format!("ellip(6,0.1,120,0.3{})", if analog { ",'s'" } else { "" });
        e.execute(&format!("[z,p,k]={call}; [b,a]={call};"))
            .unwrap();
        let r = &refs[if analog {
            "ellip_precision:True"
        } else {
            "ellip_precision:False"
        }];
        root_fixture(&e, "p", &r["p"], 1e-9);
        root_fixture(&e, "z", &r["z"], 1e-9);
        let expected = r["k"].as_f64().unwrap();
        assert!((numeric(&e, "k")[0] / expected - 1.).abs() < 1e-9);
        close(&numeric(&e, "a"), &js_vec(&r["a"]), "high Rs denominator");
    }
}
#[test]
fn analog_numerators_are_padded_and_delay_round_trip_is_preserved() {
    let mut e = Engine::default();
    e.execute("[b,a]=butter(2,1,'s'); [z,p,k]=tf2zp([0 1],[1 -0.5]); [bd,ad]=zp2tf(z,p,k);")
        .unwrap();
    close(&numeric(&e, "b"), &[0., 0., 1.], "analog padding");
    assert_eq!(shape(&e, "b"), (1, 3));
    close(&numeric(&e, "bd"), &[0., 1.], "delay round trip");
}

#[test]
fn order_selection_is_reusable_without_an_engine() {
    use matrust::dsp::iir::{self, EllipticMath, OrderSpec};
    use std::sync::atomic::{AtomicBool, Ordering};
    struct Agm;
    impl EllipticMath for Agm {
        fn jacobi(&self, _u: f64, _m: f64) -> matrust::Result<(f64, f64, f64)> {
            Err("Order selection must not need Jacobi functions".into())
        }
        fn complete(&self, m: f64) -> matrust::Result<(f64, f64)> {
            let (mut a, mut b) = (1., (1. - m).sqrt());
            for _ in 0..64 {
                let next = (a + b) * 0.5;
                b = (a * b).sqrt();
                a = next;
                if (a - b).abs() < f64::EPSILON * a {
                    break;
                }
            }
            Ok((std::f64::consts::PI / (2. * a), 0.))
        }
    }
    let refs = fixture();
    let cancel = AtomicBool::new(false);
    for name in ["buttord", "cheb1ord", "cheb2ord", "ellipord"] {
        let spec = || OrderSpec {
            name,
            wp: &[0.2],
            ws: &[0.35],
            rp: 1.25,
            rs: 36.,
            analog: false,
        };
        let (n, w) = iir::select_order(spec(), &Agm, &cancel).unwrap();
        let expected = &refs[&format!("{name}:low")];
        assert_eq!(n as f64, expected["n"].as_f64().unwrap());
        close(&w, &js_vec(&expected["wn"]), name);
        cancel.store(true, Ordering::Relaxed);
        assert!(
            iir::select_order(spec(), &Agm, &cancel)
                .unwrap_err()
                .contains("cancel")
        );
        cancel.store(false, Ordering::Relaxed);
    }
}

/// Column-major SOS rows, evaluated as descending powers of s (or z).
fn sos_at(s: &[f64], x: f64) -> f64 {
    let rows = s.len() / 6;
    assert_eq!(s.len(), rows * 6);
    let mut h = 1.;
    for r in 0..rows {
        let c = |col: usize| s[col * rows + r];
        let num = (c(0) * x + c(1)) * x + c(2);
        let den = (c(3) * x + c(4)) * x + c(5);
        h *= num / den;
    }
    h
}

#[test]
fn review_1111_zp2sos_degree_deficit_is_leading_zeros() {
    // MathWorks zp2sos: H(z) = k / ((z+1)(z+2)) = z^{-2} / (1 + 3 z^{-1} + 2 z^{-2}).
    // zp2tf of the same zeros, poles and gain is [0 0 1] / [1 3 2] (tests above).
    // SciPy zpk2sos(..., analog=True) places that constant in b2: [0 0 1 1 3 2].
    let mut e = Engine::default();
    e.execute("[b,a]=zp2tf([],[-1 -2],1); s=zp2sos([],[-1 -2],1); [bs,as]=sos2tf(s);")
        .unwrap();
    close(&numeric(&e, "b"), &[0., 0., 1.], "zp2tf numerator");
    close(&numeric(&e, "a"), &[1., 3., 2.], "zp2tf denominator");
    close(&numeric(&e, "s"), &[0., 0., 1., 1., 3., 2.], "zp2sos row");
    close(&numeric(&e, "bs"), &[0., 0., 1.], "sos2tf numerator");
    close(&numeric(&e, "as"), &[1., 3., 2.], "sos2tf denominator");
    e.execute("s=tf2sos([0 0 1],[1 3 2]);").unwrap();
    close(
        &numeric(&e, "s"),
        &[0., 0., 1., 1., 3., 2.],
        "tf2sos leading zeros",
    );
    // Analog Butterworth lowpass. H(0) = 1 (Oppenheim & Schafer, normalized prototype).
    e.execute("[b,a]=butter(2,1,'s'); s=tf2sos(b,a);").unwrap();
    let s = numeric(&e, "s");
    assert_eq!(s.len(), 6, "one section");
    assert_eq!([s[0], s[1]], [0., 0.], "not a highpass numerator");
    close(&[sos_at(&s, 0.)], &[1.], "butter(2) H(0)");
    // Order 1: H(s) = 1/(s+1). The odd-order origin pad stays on the denominator;
    // the extra numerator origin is removed, so the section is s/(s^2+s) = 1/(s+1).
    // At s = 2 that is 1/3, not the highpass value 2/3.
    e.execute("[b,a]=butter(1,1,'s'); s=tf2sos(b,a);").unwrap();
    let b = numeric(&e, "b");
    let a = numeric(&e, "a");
    let s = numeric(&e, "s");
    close(&s, &[0., 1., 0., 1., 1., 0.], "butter(1) section");
    let eval = |c: &[f64], x: f64| c.iter().fold(0., |acc, v| acc * x + *v);
    close(
        &[sos_at(&s, 2.)],
        &[eval(&b, 2.) / eval(&a, 2.)],
        "butter(1) at 2",
    );
    close(
        &[eval(&b, 2.) / eval(&a, 2.)],
        &[1. / 3.],
        "butter(1) is 1/(s+1)",
    );
    // Equal counts are unchanged, including real zeros at the origin (delays):
    // H(z) = z^2 / ((z+1)(z+2)). And the help-example quadratic is one section.
    e.execute("s=zp2sos([0 0],[-1 -2],1); t=tf2sos([1 2 1],[1 -0.5 0.2]);")
        .unwrap();
    close(
        &numeric(&e, "s"),
        &[1., 0., 0., 1., 3., 2.],
        "zeros at the origin kept",
    );
    close(
        &numeric(&e, "t"),
        &[1., 2., 1., 1., -0.5, 0.2],
        "equal-count digital section",
    );
}

#[test]
fn review_s1113_tf2sos_short_numerator_has_no_extra_delay() {
    // SciPy 1.18 signal.tf2sos([1], [1, -0.5]) is [1, 0, 0, 1, -0.5, 0].
    // H(z) = 1 / (1 - 0.5 z^{-1}). A shorter numerator is a zero at the
    // origin, not one extra sample of delay. sosfilt of the unit impulse
    // is [1, 0.5, 0.25, 0.125].
    let mut e = Engine::default();
    e.execute("s=tf2sos([1],[1 -0.5]);").unwrap();
    assert_eq!(numeric(&e, "s"), vec![1., 0., 0., 1., -0.5, 0.]);
    e.execute("y=filter([1 0 0],[1 -0.5 0],[1 0 0 0]);")
        .unwrap();
    assert_eq!(numeric(&e, "y"), vec![1., 0.5, 0.25, 0.125]);
}
