//! designfilt / digitalFilter against SciPy firwin, butter, cheby2, lfilter and
//! filtfilt (tool/refs/designfilt/generate.py).
use matrust::{Array, Engine};
use serde::Deserialize;

fn run(e: &mut Engine, source: &str) -> String {
    e.execute(source)
        .unwrap_or_else(|err| panic!("{source}: {err}"))
        .output
        .join("")
}
fn numbers(e: &Engine, name: &str) -> Vec<f64> {
    match e.workspace().get(name).unwrap() {
        Array::Numeric(x) => x.data().to_vec(),
        x => panic!("unexpected {}", x.class()),
    }
}
fn close(what: &str, a: &[f64], b: &[f64], tol: f64) {
    assert_eq!(a.len(), b.len(), "{what}: length");
    for (i, (&a, &b)) in a.iter().zip(b).enumerate() {
        assert!(
            (a - b).abs() <= tol * (1. + b.abs()),
            "{what} element {i}: got {a}, expected {b}"
        );
    }
}
#[derive(Deserialize)]
struct References {
    x: Vec<f64>,
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Case {
    name: String,
    code: String,
    b: Vec<f64>,
    a: Vec<f64>,
    y: Vec<f64>,
    yy: Vec<f64>,
}
fn references() -> References {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/designfilt/reference.json"
    ))
    .unwrap();
    serde_json::from_str(&text).unwrap()
}
fn signal_literal(x: &[f64]) -> String {
    let parts: Vec<String> = x.iter().map(|v| format!("{v:.17e}")).collect();
    format!("x=[{}];", parts.join(" "))
}

#[test]
fn coefficients_and_filtering_match_scipy() {
    let refs = references();
    for case in &refs.cases {
        let mut e = Engine::default();
        run(&mut e, &signal_literal(&refs.x));
        run(&mut e, &case.code);
        run(&mut e, "[b,a]=tf(d); y=filter(d,x); yy=filtfilt(d,x);");
        close(
            &format!("{} b", case.name),
            &numbers(&e, "b"),
            &case.b,
            1e-9,
        );
        close(
            &format!("{} a", case.name),
            &numbers(&e, "a"),
            &case.a,
            1e-9,
        );
        close(
            &format!("{} filter", case.name),
            &numbers(&e, "y"),
            &case.y,
            1e-8,
        );
        close(
            &format!("{} filtfilt", case.name),
            &numbers(&e, "yy"),
            &case.yy,
            1e-7,
        );
    }
}

#[test]
fn properties_and_coefficient_shapes() {
    let mut e = Engine::default();
    run(
        &mut e,
        "d=designfilt('lowpassfir','FilterOrder',20,'CutoffFrequency',150,'SampleRate',1000); \
         c=size(d.Coefficients); fs=d.SampleRate; n=filtord(d); k=class(d); r=d.FrequencyResponse; \
         i=d.ImpulseResponse; m=d.DesignMethod; f=d.CutoffFrequency; t=isa(d,'digitalFilter');",
    );
    assert_eq!(numbers(&e, "c"), [1., 21.]);
    assert_eq!(numbers(&e, "fs"), [1000.]);
    assert_eq!(numbers(&e, "n"), [20.]);
    assert_eq!(numbers(&e, "f"), [150.]);
    assert!(run(&mut e, "k").contains("digitalFilter"));
    assert!(run(&mut e, "r").contains("lowpass"));
    assert!(run(&mut e, "i").contains("fir"));
    assert!(run(&mut e, "m").contains("window"));
    assert!(run(&mut e, "t").contains('1'));
    run(
        &mut e,
        "g=designfilt('bandpassiir','FilterOrder',6,'HalfPowerFrequency1',0.2,'HalfPowerFrequency2',0.45); \
         s=size(g.Coefficients); fs2=g.SampleRate; [b,a]=tf(g); y=sosfilt(g.Coefficients,[1 zeros(1,30)]); \
         z=filter(b,a,[1 zeros(1,30)]);",
    );
    assert_eq!(numbers(&e, "s"), [3., 6.]);
    assert_eq!(numbers(&e, "fs2"), [2.]);
    close("sos impulse", &numbers(&e, "y"), &numbers(&e, "z"), 1e-10);
}

#[test]
fn analysis_functions_take_the_filter() {
    let mut e = Engine::default();
    run(
        &mut e,
        "d=designfilt('lowpassiir','FilterOrder',4,'HalfPowerFrequency',120,'SampleRate',1000); \
         [b,a]=tf(d); freqz(d,256); g=grpdelay(d,64); g2=grpdelay(b,a,64); \
         i=impz(d,40); i2=impz(b,a,40);",
    );
    // The tf form loses ~1e-9 near the four zeros at z=-1; the SOS route matches
    // 50-digit mpmath (sum of per-section delays) to 1e-14.
    close("grpdelay", &numbers(&e, "g"), &numbers(&e, "g2"), 1e-8);
    let g = numbers(&e, "g");
    close(
        "grpdelay vs mpmath",
        &[g[61], g[63]],
        &[0.5203035462003667, 0.5176368753018201],
        1e-13,
    );
    close("impz", &numbers(&e, "i"), &numbers(&e, "i2"), 1e-10);
    assert_eq!(numbers(&e, "g").len(), 64);
}

#[test]
fn display_lists_the_specification() {
    let mut e = Engine::default();
    let text = run(
        &mut e,
        "d=designfilt('highpassiir','FilterOrder',3,'HalfPowerFrequency',0.25)",
    );
    for want in [
        "digitalFilter with properties:",
        "Coefficients: [2\u{d7}6 double]",
        "FrequencyResponse: 'highpass'",
        "ImpulseResponse: 'iir'",
        "HalfPowerFrequency: 0.25",
        "DesignMethod: 'butter'",
    ] {
        assert!(text.contains(want), "missing {want} in {text}");
    }
}

#[test]
fn clear_errors() {
    let mut e = Engine::default();
    for (code, want) in [
        ("designfilt('notchiir','FilterOrder',2)", "not supported"),
        (
            "designfilt('lowpassfir','CutoffFrequency',0.2)",
            "FilterOrder is required",
        ),
        (
            "designfilt('lowpassfir','FilterOrder',10,'CutoffFrequency',1.2)",
            "Nyquist",
        ),
        (
            "designfilt('highpassfir','FilterOrder',11,'CutoffFrequency',0.3)",
            "even FilterOrder",
        ),
        (
            "designfilt('lowpassfir','FilterOrder',10,'PassbandRipple',1)",
            "not supported",
        ),
        (
            "designfilt('lowpassiir','FilterOrder',4,'HalfPowerFrequency',0.3,'DesignMethod','cheby2')",
            "does not match",
        ),
        ("designfilt('lowpassfir','FilterOrder')", "pairs"),
    ] {
        let err = e.execute(code).unwrap_err();
        assert!(err.contains(want), "{code}: {err}");
    }
    run(
        &mut e,
        "d=designfilt('lowpassfir','FilterOrder',4,'CutoffFrequency',0.3);",
    );
    let err = e.execute("d.FilterOrder = 5").unwrap_err();
    assert!(err.contains("read-only"), "{err}");
    let err = e.execute("d + 1").unwrap_err();
    assert!(!err.is_empty());
}

#[test]
fn narrow_high_order_iir_stays_stable() {
    let mut e = Engine::default();
    run(
        &mut e,
        "d=designfilt('lowpassiir','FilterOrder',20,'HalfPowerFrequency',.02); x=[1 zeros(1,999)]; \
         y=filter(d,x); y2=sosfilt(d.Coefficients,x); m=max(abs(y)); yy=filtfilt(d,ones(1,200)); \
         g=grpdelay(d,16); i=impz(d,50); i2=sosimpz(d.Coefficients,50);",
    );
    close("sos filter", &numbers(&e, "y"), &numbers(&e, "y2"), 1e-12);
    assert!(numbers(&e, "m")[0] < 1.);
    // Unit DC gain: a constant input comes back unchanged after forward-backward filtering.
    let yy = numbers(&e, "yy");
    assert!(yy.iter().all(|v| (v - 1.).abs() < 1e-6), "{yy:?}");
    assert!(numbers(&e, "g").iter().all(|v| v.is_finite()));
    close("sos impz", &numbers(&e, "i"), &numbers(&e, "i2"), 1e-12);
}

fn complex_values(e: &Engine, name: &str) -> ((usize, usize), Vec<(f64, f64)>) {
    match e.workspace().get(name) {
        Some(Array::Numeric(m)) => (
            (m.rows(), m.cols()),
            m.data().iter().map(|&v| (v, 0.)).collect(),
        ),
        Some(Array::Complex(m)) => (
            (m.rows(), m.cols()),
            m.data().iter().map(|z| (z.re, z.im)).collect(),
        ),
        other => panic!("{name}: expected a numeric array, got {other:?}"),
    }
}
fn logical_scalar(e: &Engine, name: &str) -> bool {
    match e.workspace().get(name) {
        Some(Array::Logical(m)) => {
            assert_eq!((m.rows(), m.cols()), (1, 1), "{name} shape");
            m.data()[0] != 0
        }
        other => panic!("{name}: expected a scalar logical, got {other:?}"),
    }
}
fn assert_response(what: &str, got: &[(f64, f64)], want: &[(f64, f64)]) {
    assert_eq!(got.len(), want.len(), "{what} length");
    for (i, (u, v)) in got.iter().zip(want).enumerate() {
        let err = (u.0 - v.0).hypot(u.1 - v.1);
        assert!(
            err <= 1e-12,
            "{what}[{i}]: got {u:?}, expected {v:?}, err {err}"
        );
    }
}

#[test]
fn direct_form_df1_matches_coefficient_filter_and_freqz() {
    let mut e = Engine::default();
    run(
        &mut e,
        "H=dfilt.df1([1 2],1); fir=isfir(H); \
         y=filter(H,[1 0 0]); yc=filter([1 2],1,[1 0 0]); \
         h=freqz(H,[0 pi]); hc=freqz([1 2],1,[0 pi]); \
         G=dfilt.df1(1,[1 -.5]); iir=isfir(G); \
         z=filter(G,[1 0 0]); zc=filter(1,[1 -.5],[1 0 0]); \
         g=freqz(G,[0 pi]); gc=freqz(1,[1 -.5],[0 pi]); \
         b=poly([1i,-1i]); a=poly([0 0]); P=dfilt.df1(b,a); pf=isfir(P); \
         py=filter(P,[1 0 0 0]); pyc=filter(b,a,[1 0 0 0]); \
         Hc=dfilt.df1([1;2],1); ycol=filter(Hc,[1 0 0]); \
         D=designfilt('lowpassfir','FilterOrder',4,'CutoffFrequency',0.3); df=isfir(D); \
         yd=filter(D,[1 0 0 0 0]); ydc=filter(D.Coefficients,1,[1 0 0 0 0]); \
         E=designfilt('lowpassiir','FilterOrder',2,'HalfPowerFrequency',0.3); di=isfir(E); \
         dfilt=struct('df1',[4 5 6]); shadow=dfilt.df1(2); \
         note=struct('label',3); tag=note.label;",
    );
    assert!(logical_scalar(&e, "fir"));
    assert!(!logical_scalar(&e, "iir"));
    // poly([0 0]) is [1 0 0]. The trailing zeros are not feedback.
    assert!(logical_scalar(&e, "pf"));
    assert!(logical_scalar(&e, "df"));
    assert!(!logical_scalar(&e, "di"));

    let (ys, y) = complex_values(&e, "y");
    let (ycs, yc) = complex_values(&e, "yc");
    assert_eq!(ys, ycs, "FIR impulse shape");
    assert_eq!(ys, (1, 3));
    assert_response("FIR impulse", &y, &[(1., 0.), (2., 0.), (0., 0.)]);
    assert_response("FIR impulse vs coefficients", &y, &yc);

    let (hs, h) = complex_values(&e, "h");
    let (hcs, hc) = complex_values(&e, "hc");
    assert_eq!(hs, hcs, "FIR freqz shape");
    assert_eq!(hs, (1, 2));
    assert_response("FIR freqz", &h, &[(3., 0.), (-1., 0.)]);
    assert_response("FIR freqz vs coefficients", &h, &hc);

    let (zs, z) = complex_values(&e, "z");
    let (zcs, zc) = complex_values(&e, "zc");
    assert_eq!(zs, zcs, "IIR impulse shape");
    assert_eq!(zs, (1, 3));
    assert_response("IIR impulse", &z, &[(1., 0.), (0.5, 0.), (0.25, 0.)]);
    assert_response("IIR impulse vs coefficients", &z, &zc);

    let (gs, g) = complex_values(&e, "g");
    let (gcs, gc) = complex_values(&e, "gc");
    assert_eq!(gs, gcs, "IIR freqz shape");
    assert_eq!(gs, (1, 2));
    assert_response("IIR freqz", &g, &[(2., 0.), (2. / 3., 0.)]);
    assert_response("IIR freqz vs coefficients", &g, &gc);

    let (pys, py) = complex_values(&e, "py");
    let (pycs, pyc) = complex_values(&e, "pyc");
    assert_eq!(pys, pycs, "polynomial impulse shape");
    assert_eq!(pys, (1, 4));
    // Octave 10.3 numeric reference for filter(poly([1i,-1i]), poly([0 0]), [1 0 0 0]).
    assert_response(
        "polynomial impulse",
        &py,
        &[(1., 0.), (0., 0.), (1., 0.), (0., 0.)],
    );
    assert_response("polynomial impulse vs coefficients", &py, &pyc);
    assert_eq!(numbers(&e, "b"), [1., 0., 1.]);
    assert_eq!(numbers(&e, "a"), [1., 0., 0.]);

    let (cols, ycol) = complex_values(&e, "ycol");
    assert_eq!(cols, ys, "column numerator keeps the row impulse shape");
    assert_response("column numerator", &ycol, &y);

    let (yds, yd) = complex_values(&e, "yd");
    let (ydcs, ydc) = complex_values(&e, "ydc");
    assert_eq!(yds, ydcs, "designed FIR shape");
    assert_response("designed FIR vs its coefficients", &yd, &ydc);
    assert_eq!(numbers(&e, "shadow"), [5.]);
    assert_eq!(numbers(&e, "tag"), [3.]);

    let mut bad = Engine::default();
    let matrix = bad.execute("dfilt.df1([1 2; 3 4], 1)").unwrap_err();
    assert!(
        matrix.contains("vector") || matrix.contains("matrix"),
        "{matrix}"
    );
    let leading = bad.execute("dfilt.df1(1, [0 1])").unwrap_err();
    assert!(
        leading.contains("leading") || leading.contains("nonzero"),
        "{leading}"
    );
    let missing = bad.execute("dfilt.df1([1 2])").unwrap_err();
    assert!(
        missing.contains("required") || missing.contains("denominator"),
        "{missing}"
    );
}
