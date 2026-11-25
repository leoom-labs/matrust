//! Numerical facts from python-control 0.10.2 and SciPy; reproduce with
//! bash tool/ref-python.sh tool/refs/control/responses.py. No MATLAB evidence.
use matrust::{Array, Engine};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct Fixture {
    origin: String,
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Case {
    name: String,
    code: String,
    expected: BTreeMap<String, Expected>,
    tolerance: Option<f64>,
}
#[derive(Deserialize)]
struct Expected {
    rows: usize,
    cols: usize,
    real: Vec<Number>,
    imag: Vec<Number>,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Number {
    Finite(f64),
    Special(String),
}
impl Number {
    fn value(&self) -> f64 {
        match self {
            Self::Finite(v) => *v,
            Self::Special(s) => match s.as_str() {
                "Inf" => f64::INFINITY,
                "-Inf" => f64::NEG_INFINITY,
                "NaN" => f64::NAN,
                _ => panic!("Invalid fixture number"),
            },
        }
    }
}
fn parts(a: &Array) -> Vec<(f64, f64)> {
    match a {
        Array::Numeric(m) => m.data().iter().map(|&v| (v, 0.)).collect(),
        Array::Complex(m) => m.data().iter().map(|v| (v.re, v.im)).collect(),
        _ => panic!("Expected numerical array, got {}", a.class()),
    }
}
fn close(a: f64, b: f64, tolerance: f64) -> bool {
    if b.is_nan() {
        a.is_nan()
    } else if b.is_infinite() {
        a == b
    } else {
        (a - b).abs() <= 1e-12 + tolerance * b.abs()
    }
}
#[test]
fn python_control_response_fixtures() {
    verify_reference_fixtures();
}
#[test]
fn python_control_response_fixtures_four_workers() {
    rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap()
        .install(verify_reference_fixtures);
}
fn verify_reference_fixtures() {
    verify_reference_fixtures_matching("");
}
#[test]
fn continuous_margins_cover_crossovers_outside_pole_zero_grid() {
    verify_reference_fixtures_matching("margin_extended_");
}
#[test]
fn automatic_margin_range_limit_preserves_published_value() {
    let mut e = Engine::default();
    e.execute("G=tf(1e50,[1 1 0]); gm=42;").unwrap();
    let err = e.execute("gm=margin(G);").unwrap_err();
    assert!(err.contains("10000 points"), "{err}");
    assert_eq!(scalar(&e, "gm"), 42.);
}
fn verify_reference_fixtures_matching(prefix: &str) {
    let fixture: Fixture =
        serde_json::from_str(include_str!("fixtures/control/responses.json")).unwrap();
    assert!(fixture.origin.contains("python-control 0.10.2"));
    let cases: Vec<_> = fixture
        .cases
        .into_iter()
        .filter(|c| c.name.starts_with(prefix))
        .collect();
    assert!(!cases.is_empty());
    for case in cases {
        let mut e = Engine::default();
        e.execute(&case.code)
            .unwrap_or_else(|err| panic!("{}: {err}\n{}", case.name, case.code));
        for (name, reference) in case.expected {
            let actual = e
                .array(&name)
                .unwrap_or_else(|| panic!("{}: missing {name}", case.name));
            // The reference predates multidimensional responses and stores
            // time or frequency contiguous within each of the four channels.
            let samples = reference.rows;
            let spectrum = case.name == "frequency_mimo"
                && matches!(name.as_str(), "H" | "mag" | "phase" | "re" | "im")
                || case.name == "dense_hessenberg_parallel" && name == "H";
            let dims = if spectrum {
                vec![2, 2, samples]
            } else if matches!(case.name.as_str(), "mimo_step" | "mimo_impulse") && name == "y" {
                vec![samples, 2, 2]
            } else {
                vec![samples, reference.cols]
            };
            assert_eq!(actual.dims().as_ref(), dims, "{} {name}", case.name);
            let mut actual = parts(&actual);
            if spectrum {
                actual = (0..4)
                    .flat_map(|pair| actual.iter().skip(pair).step_by(4).copied())
                    .collect();
            }
            assert_eq!(actual.len(), reference.real.len());
            assert_eq!(actual.len(), reference.imag.len());
            for (i, ((re, im), (a, b))) in actual
                .iter()
                .zip(reference.real.iter().zip(&reference.imag))
                .enumerate()
            {
                let (a, b) = (a.value(), b.value());
                let tolerance = case.tolerance.unwrap_or(1e-9);
                assert!(
                    close(*re, a, tolerance) && close(*im, b, tolerance),
                    "{} {name}[{i}]: ({re},{im}) != ({a},{b}); tolerance={tolerance}",
                    case.name
                );
            }
        }
    }
}
fn scalar(e: &Engine, name: &str) -> f64 {
    parts(&e.array(name).unwrap())[0].0
}
fn plot_shape(e: &Engine, axes: usize, lines_per_axis: usize) {
    let f = e.figure.as_ref().expect("figure exists");
    let all: Vec<_> = f.all_axes().collect();
    assert_eq!(all.len(), axes);
    for (_, a) in all {
        assert_eq!(a.series.len(), lines_per_axis);
    }
}
#[test]
fn plotting_smoke_and_overlays() {
    for (code, axes, lines) in [
        ("step(G,t);", 1, 1),
        ("impulse(G,t);", 1, 1),
        ("initial(ss(G),[1;0],t);", 1, 1),
        ("lsim(G,sin(t),t);", 1, 1),
        ("bode(G);", 2, 1),
        ("nyquist(G);", 1, 3),
        ("margin(G);", 2, 1),
        ("step(G,G,t);", 1, 2),
        ("impulse(G,G,t);", 1, 2),
        ("bode(G,G);", 2, 2),
        ("nyquist(G,G);", 1, 5),
        (
            "w=logspace(-2,2,200); [mag,phase]=bode(G,w); margin(mag,phase,w);",
            2,
            1,
        ),
    ] {
        let mut e = Engine::default();
        e.execute("G=tf(1,[1 0.4 1]); t=(0:0.02:10)';").unwrap();
        e.execute(code)
            .unwrap_or_else(|err| panic!("{code}: {err}"));
        plot_shape(&e, axes, lines);
        assert!(
            e.array("ans").is_none(),
            "{code} should plot without publishing ans"
        );
        if code.starts_with("bode") || code.starts_with("margin") {
            for (_, a) in e.figure.as_ref().unwrap().all_axes() {
                assert_eq!(a.xscale, matrust::plot::AxisScale::Log);
            }
        }
    }
}
#[test]
fn mimo_plot_pair_axes_and_return_values_do_not_plot() {
    let prefix = "G=ss([-1 0;0 -2],eye(2),eye(2),zeros(2)); t=(0:0.1:2)'; ";
    for (code, axes, lines) in [
        ("step(G,t);", 4, 1),
        ("impulse(G,t);", 4, 1),
        ("initial(G,[1;2],t);", 2, 1),
        ("lsim(G,[sin(t) cos(t)],t);", 2, 1),
        ("bode(G);", 8, 1),
        ("nyquist(G);", 4, 3),
    ] {
        let mut e = Engine::default();
        e.execute(&(prefix.to_owned() + code))
            .unwrap_or_else(|err| panic!("{code}: {err}"));
        plot_shape(&e, axes, lines);
    }
    let mut e = Engine::default();
    e.execute("G=tf(1,[1 1]); y=step(G); h=bode(G); r=nyquist(G); gm=margin(G);")
        .unwrap();
    assert!(e.figure.is_none());
}
/// Fraction of the figure an axes covers: (top, bottom, left, right).
fn span(p: matrust::plot::Position) -> (f64, f64, f64, f64) {
    let (r, c) = (f64::from(p.rows), f64::from(p.columns));
    (
        f64::from(p.top) / r,
        f64::from(p.bottom) / r,
        f64::from(p.left) / c,
        f64::from(p.right) / c,
    )
}
#[test]
fn response_grids_nest_in_the_subplot_cell_that_subplot_just_selected() {
    // A response plot draws where the current (empty) subplot cell is; it
    // used to lay its panels over the whole figure and delete the other cells.
    let mut e = Engine::default();
    e.execute(
        "G=tf(1,[1 0.4 1]); P=ss([-1 0;0 -2],eye(2),eye(2),zeros(2)); figure; \
         subplot(1,2,1); step(G); subplot(1,2,2); bode(G);",
    )
    .unwrap();
    let f = e.figure.as_ref().unwrap();
    let axes: Vec<_> = f
        .all_axes()
        .map(|(p, a)| (span(p), a.series.len()))
        .collect();
    assert_eq!(axes.len(), 3, "{axes:?}");
    assert!(
        axes.contains(&((0., 1., 0., 0.5), 1)),
        "step stays: {axes:?}"
    );
    assert!(
        axes.contains(&((0., 0.5, 0.5, 1.), 1)),
        "magnitude: {axes:?}"
    );
    assert!(axes.contains(&((0.5, 1., 0.5, 1.), 1)), "phase: {axes:?}");
    // MIMO grids and Nichols panels nest the same way, in any cell.
    for (code, panels) in [
        ("step(P);", 4),
        ("bode(P);", 8),
        ("nyquist(P);", 4),
        ("nichols(P);", 4),
        ("initial(P,[1;2]);", 2),
    ] {
        let mut e = Engine::default();
        e.execute(&format!(
            "G=tf(1,[1 0.4 1]); P=ss([-1 0;0 -2],eye(2),eye(2),zeros(2)); figure; \
             subplot(2,2,1); step(G); subplot(2,2,[3 4]); {code}"
        ))
        .unwrap_or_else(|err| panic!("{code}: {err}"));
        let f = e.figure.as_ref().unwrap();
        let spans: Vec<_> = f.all_axes().map(|(p, _)| span(p)).collect();
        assert_eq!(spans.len(), 1 + panels, "{code}: {spans:?}");
        assert!(spans.contains(&(0., 0.5, 0., 0.5)), "{code}: {spans:?}");
        assert_eq!(
            spans.iter().filter(|s| s.0 >= 0.5 && s.1 <= 1.).count(),
            panels,
            "{code}: {spans:?}"
        );
    }
    // A response plot over an earlier one (no subplot call between) still
    // replaces it rather than nesting into its last panel.
    let mut e = Engine::default();
    e.execute("G=tf(1,[1 0.4 1]); bode(G); bode(G, G);")
        .unwrap();
    plot_shape(&e, 2, 2);
}
#[test]
fn exact_integrator_static_and_feedthrough_conventions() {
    let mut e = Engine::default();
    e.execute("t=(0:0.1:2)'; G=ss(0,1,1,0); [y,t,x]=lsim(G,t,t,'foh'); E=y-t.^2/2; err=max(abs(E)); ys=step(G,t); es=max(abs(ys-t)); yi=impulse(G,t); ei=max(abs(yi-1)); H=ss([],zeros(0,1),zeros(1,0),3); yh=step(H,t); eh=max(abs(yh-3)); Q=ss(-1,1,1,2); qi=impulse(Q,t); eq=max(abs(qi-exp(-t))); R=ss(0.5,1,1,2,0.1); ir=impulse(R,t); r0=ir(1); r1=ir(2);").unwrap();
    for name in ["err", "es", "ei", "eh", "eq"] {
        assert!(scalar(&e, name) < 1e-12, "{name}");
    }
    assert_eq!(scalar(&e, "r0"), 20.);
    assert_eq!(scalar(&e, "r1"), 10.);
}
#[test]
fn automatic_grids_match_the_explicit_time_grid() {
    for system in [
        "tf(1,[1 0.4 1])",
        "ss(0.8,1,1,0,0.1)",
        "ss(0,1,1,0)",
        "ss(1,1,1,0)",
    ] {
        let mut e = Engine::default();
        e.execute(&format!("G={system}; [a,t]=step(G); b=step(G,t); err=max(abs(a-b)); n=length(t); [c,tc]=impulse(G); d=impulse(G,tc); ei=max(abs(c-d));")).unwrap();
        assert!(scalar(&e, "err") < 1e-12);
        assert!(scalar(&e, "ei") < 1e-12);
        assert!((100. ..=10_000.).contains(&scalar(&e, "n")));
    }
}
#[test]
fn default_discrete_grid_bounds_its_horizon_at_the_sample_time() {
    let mut e = Engine::default();
    e.execute("G=ss(.9999999,1,1,0,.1); [y,t]=step(G); n=length(t); dt=t(2)-t(1); yf=y(end); [q,tq]=step(G,0.2); nq=length(tq);").unwrap();
    assert_eq!(scalar(&e, "n"), 10_000.);
    assert_eq!(scalar(&e, "dt"), 0.1);
    assert_eq!(scalar(&e, "nq"), 3.);
    let expected = (1. - 0.9999999f64.powi(9999)) / (1. - 0.9999999);
    assert!(close(scalar(&e, "yf"), expected, 1e-9));
}
#[test]
fn discrete_margin_includes_exact_nyquist_endpoint_for_every_sample_time() {
    // Endpoint-inclusive convention: H(-1)=-2/3 gives Gm=3/2.
    // python-control excludes this endpoint and returns an infinite Gm.
    for ts in [1., 0.5, 0.2, 0.1, 0.05, 0.01] {
        for model in ["tf(1,[1 -0.5],Ts)", "ss(0.5,1,1,0,Ts)"] {
            let mut e = Engine::default();
            e.execute(&format!("Ts={ts}; G={model}; [gm,pm,wg,wp]=margin(G); [mag,phase,w]=bode(G); last=w(end); wn=pi/Ts; H=freqresp(G,wn); E=evalfr(G,-1); endpoint_error=abs(H-E); endpoint_imag=imag(H);")).unwrap();
            assert!(close(scalar(&e, "gm"), 1.5, 1e-12), "Ts={ts}, {model}");
            assert_eq!(scalar(&e, "wg"), scalar(&e, "wn"));
            assert_eq!(scalar(&e, "last"), scalar(&e, "wn"));
            assert_eq!(scalar(&e, "endpoint_error"), 0.);
            assert_eq!(scalar(&e, "endpoint_imag"), 0.);
        }
    }
}
#[test]
fn automatic_time_treats_near_imaginary_rates_as_undamped() {
    // Similarity preserves the oscillator poles +/-j; rounding in eig must
    // not turn their vanishing real parts into enormous decay horizons.
    for model in [
        "tf(1,[1 0 1])",
        "ss(T*[0 1;-1 0]/T,[0;1],[1 0],0)",
        "tf(1,[1 2e-12 1])",
        "tf(1,[1 -2e-12 1])",
    ] {
        let mut e = Engine::default();
        e.execute(&format!(
            "T=[1 2;3 4]; G={model}; [y,t]=step(G); last=t(end);"
        ))
        .unwrap();
        assert!(close(scalar(&e, "last"), 10., 1e-9), "{model}");
        assert!(scalar(&e, "last") < 1e3);
    }
    let mut e = Engine::default();
    // log(z)/Ts has an order-one imaginary rate and tiny negative real rate.
    e.execute("r=1-1e-12; a=r*[cos(0.1) sin(0.1);-sin(0.1) cos(0.1)]; G=ss(a,[0;1],[1 0],0,0.1); [y,t]=step(G); last=t(end);").unwrap();
    assert!((scalar(&e, "last") - 10.).abs() <= 0.1);
}
#[test]
fn control_responses_require_inputs_and_outputs() {
    for model in [
        "ss(-eye(40),zeros(40,0),ones(1,40),zeros(1,0))",
        "ss(-eye(40),ones(40,1),zeros(0,40),zeros(0,1))",
        "ss(-eye(40),zeros(40,0),zeros(0,40),zeros(0,0))",
    ] {
        let mut e = Engine::default();
        e.execute(&format!("G={model}; assert(isempty(G));"))
            .unwrap();
        for name in ["step", "impulse", "bode"] {
            let err = e.execute(&format!("y={name}(G);")).unwrap_err();
            assert!(
                err.contains("at least one input and one output"),
                "{name}: {err}"
            );
        }
    }
}
#[test]
fn automatic_time_treats_near_zero_poles_as_integrators() {
    let mut e = Engine::default();
    e.execute("T=[1 2 0;0 1 3;1 0 1]; A=T*diag([0 -1 -2])/T; G=ss(A,[1;0;0],[1 0 0],0); [y,t]=step(G); last=t(end); explicit=step(G,t); err=max(abs(y-explicit));").unwrap();
    assert!(scalar(&e, "last") < 1e3);
    assert!(close(scalar(&e, "last"), 10., 1e-12));
    assert!(scalar(&e, "err") < 1e-12);
    for pole in [-1e-11, 0., 1e-11] {
        e.execute(&format!("[y,t]=step(ss({pole},1,1,0)); last=t(end);"))
            .unwrap();
        assert!(close(scalar(&e, "last"), 10., 1e-12));
    }
}
#[test]
fn bounds_and_invalid_calls_are_explicit() {
    for code in [
        "y=step();",
        "y=step(3);",
        "y=step(tf(1,[1 1]),[0 .1 .3]);",
        "y=step(tf(1,[1 1]),[1 2 3]);",
        "y=step(tf(1,[1 1]),0);",
        "y=step(ss(.5,1,1,0,.1),[0 .2 .4]);",
        "y=initial(tf(1,[1 1]),0);",
        "y=initial(ss(-1,1,1,0),[1;2]);",
        "y=lsim(ss(-1,1,1,0),[1 2],[0 .1 .2]);",
        "y=lsim(ss(-1,1,1,0),[1 2 3],[0 .1 .2],'cubic');",
        "y=lsim(ss(-1,1,1,0),[1 2 3],[0 .1 .2],'foh','zoh');",
        "y=freqresp(tf(1,[1 1]),[1 .1]);",
        "y=bode(tf(1,[1 1]),[-1 1]);",
        "y=freqresp(ss(.5,1,1,0,.1),[1 40]);",
        "y=evalfr(tf(1,[1 1]),-1);",
        "y=margin(ss(-eye(2),eye(2),eye(2),zeros(2)));",
        "y=stepinfo([1 2 3]);",
        "y=stepinfo([1 2],[0 .1 .2]);",
        "[a,b,c,d]=step(tf(1,[1 1]));",
        "[a,b]=evalfr(tf(1,[1 1]),1i);",
        "y=step(ss(-eye(257),ones(257,1),ones(1,257),0),[0 1]);",
        "step(tf(1,[1 1]),ss(-eye(2),eye(2),eye(2),zeros(2)));",
    ] {
        let mut e = Engine::default();
        assert!(e.execute(code).is_err(), "Must reject: {code}");
        assert!(
            e.figure.is_none(),
            "Invalid call must not partially create a figure: {code}"
        );
    }
}
#[test]
fn interpolation_option_can_be_stored_character_text() {
    let mut e = Engine::default();
    e.execute(
        "G=ss(0,1,1,0); t=(0:.1:1)'; method='foh'; y=lsim(G,t,t,method); err=max(abs(y-t.^2/2));",
    )
    .unwrap();
    assert!(scalar(&e, "err") < 1e-12);
}
#[test]
fn plot_resource_failure_restores_the_previous_figure() {
    let mut e = Engine::default();
    e.execute(
        "plot([0 1],[2 3]); G=ss(-1,ones(1,16),ones(8,1),zeros(8,16)); t=linspace(0,1,10000);",
    )
    .unwrap();
    let before = e.figure.as_ref().unwrap().clone();
    assert!(e.execute("step(G,t);").is_err());
    assert!(before.same_snapshot(e.figure.as_ref().unwrap()));
}
#[test]
fn cancellation_during_large_frequency_batch_preserves_published_value() {
    let mut e = Engine::default();
    e.execute("n=128; G=ss(-diag(1:n)+.002*ones(n),ones(n,2),ones(2,n),zeros(2)); w=logspace(-2,2,10000); H=42;").unwrap();
    let stop = e.cancellation();
    let thread = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(20));
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
    });
    let result = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap()
        .install(|| e.execute("H=freqresp(G,w);"));
    thread.join().unwrap();
    assert!(result.unwrap_err().contains("cancel"));
    assert_eq!(scalar(&e, "H"), 42.);
}
#[test]
fn stepinfo_negative_and_zero_gain_and_infinite_margin() {
    let mut e = Engine::default();
    e.execute("G=tf(-1,[1 .4 1]); t=(0:.01:40)'; S=stepinfo(G,t); over=S.Overshoot; rise=S.RiseTime; G=tf(1,[1 1]); [gm,pm,wg,wp]=margin(G); Z=stepinfo([0;0;0],[0;1;2]); rz=Z.RiseTime; sz=Z.SettlingTime;").unwrap();
    assert!((scalar(&e, "over") - 52.662).abs() < 0.01);
    assert!(scalar(&e, "rise") > 0.);
    assert!(scalar(&e, "gm").is_infinite());
    assert!(scalar(&e, "pm").is_infinite());
    for name in ["wg", "wp", "rz", "sz"] {
        assert!(scalar(&e, name).is_nan(), "{name}");
    }
}
