//! nichols, sigma, allmargin and ngrid. Expected numbers come from
//! python-control 0.10.2, NumPy (linalg.svd, linalg.solve) and SciPy (brentq on
//! a dense grid, cross-checked against control.stability_margins with
//! returnall=True): bash tool/ref-python.sh tool/ref/control_freq.py prints
//! the module below. No MATLAB evidence.
use matrust::{Array, Engine};

#[rustfmt::skip]
mod reference {
    pub const SIGMA_SISO_W: &[f64] = &[0.1, 0.5, 1.0, 2.0, 10.0];
    pub const SIGMA_SISO: &[f64] = &[1.009277530082256, 1.2883132528016616, 2.5, 0.3220783132004154, 0.010092775300822561];
    pub const NICHOLS_SISO_PHASE: &[f64] = &[-2.3137224978242164, -14.931417178137552, -90.0, -165.06858282186246, -177.68627750217578];
    pub const SIGMA_MIMO_T0: &[f64] = &[0.6528029938303898, 0.4441913427111515, 0.7499101350934265, 0.3810275043818744, 0.8573666003657296, 0.25305643012004064, 0.3868442110820607, 0.1523329574885347];
    pub const SIGMA_MIMO_T1: &[f64] = &[2.251282057629564, 1.5318557197974778, 2.6244824546780676, 1.3334931123119391, 3.9516877699003214, 1.1663622067542947, 6.564567618765391, 2.585019941756015];
    pub const SIGMA_MIMO_T2: &[f64] = &[1.5370274028262598, 1.3278375218733176, 1.6710301009498252, 1.268678522783627, 1.8541076276431394, 1.1811131478175108, 1.3221856592322385, 1.1306867121405366];
    pub const SIGMA_MIMO_T3: &[f64] = &[3.03544295703573, 2.3187410135978337, 3.3562806271647845, 2.2106095677895183, 4.673587017110895, 2.159693758207912, 7.439579551304058, 3.4100155750477046];
    pub const SIGMA_DISCRETE_W: &[f64] = &[0.5, 2.0, 10.0, 31.41592653589793];
    pub const SIGMA_DISCRETE: &[f64] = &[4.192952971899151, 1.2537129645190006, 3.9318832818143017, 1.205958874607586, 1.7251905716088796, 0.7434126324609779, 0.8255807159855013, 0.42417966620156977];
    pub const AM_CUBIC_GM: &[f64] = &[8.0];
    pub const AM_CUBIC_GMF: &[f64] = &[1.7320508075688774];
    pub const AM_CUBIC_PM: &[f64] = &[];
    pub const AM_CUBIC_PMF: &[f64] = &[];
    pub const AM_CUBIC_DM: &[f64] = &[];
    // AM_CUBIC stable 1.0
    pub const AM_TWO_GM: &[f64] = &[];
    pub const AM_TWO_GMF: &[f64] = &[];
    pub const AM_TWO_PM: &[f64] = &[163.21350452795997, 28.671181400068093];
    pub const AM_TWO_PMF: &[f64] = &[0.7220153754268751, 1.199455625543183];
    pub const AM_TWO_DM: &[f64] = &[3.9453634017799097, 0.41719468833299506];
    // AM_TWO stable 1.0
    pub const AM_COND_GM: &[f64] = &[0.017359378006885903, 6.400638955326446];
    pub const AM_COND_GMF: &[f64] = &[1.0206229412959555, 97.97937705870403];
    pub const AM_COND_PM: &[f64] = &[54.740951170469486];
    pub const AM_COND_PMF: &[f64] = &[27.873121313450003];
    pub const AM_COND_DM: &[f64] = &[0.03427710240464203];
    // AM_COND stable 1.0
    pub const AM_UNSTABLE_GM: &[f64] = &[0.8];
    pub const AM_UNSTABLE_GMF: &[f64] = &[1.7320508075688774];
    pub const AM_UNSTABLE_PM: &[f64] = &[-7.032600002712854];
    pub const AM_UNSTABLE_PMF: &[f64] = &[1.9082947449523562];
    pub const AM_UNSTABLE_DM: &[f64] = &[3.2282451641453793];
    // AM_UNSTABLE stable 0.0
    pub const AM_DISCRETE_GM: &[f64] = &[2.999999999999999];
    pub const AM_DISCRETE_GMF: &[f64] = &[8.63211890069541];
    pub const AM_DISCRETE_PM: &[f64] = &[76.61619194058324];
    pub const AM_DISCRETE_PMF: &[f64] = &[3.3791536236732806];
    pub const AM_DISCRETE_DM: &[f64] = &[3.9572181635516284];
    // AM_DISCRETE stable 1.0
    pub const AM_MIMO1_GM: &[f64] = &[];
    pub const AM_MIMO1_GMF: &[f64] = &[];
    pub const AM_MIMO1_PM: &[f64] = &[120.71707466241611];
    pub const AM_MIMO1_PMF: &[f64] = &[1.7009567186055328];
    pub const AM_MIMO1_DM: &[f64] = &[1.2386619795724474];
    pub const AM_MIMO2_GM: &[f64] = &[];
    pub const AM_MIMO2_GMF: &[f64] = &[];
    pub const AM_MIMO2_PM: &[f64] = &[132.8339266685511];
    pub const AM_MIMO2_PMF: &[f64] = &[2.192273585369728];
    pub const AM_MIMO2_DM: &[f64] = &[1.0575273972148516];
    // AM_MIMO stable 1.0
}
use reference::*;

fn run(code: &str) -> Engine {
    let mut e = Engine::default();
    e.execute(code)
        .unwrap_or_else(|err| panic!("{code}: {err}"));
    e
}

fn values(e: &Engine, name: &str) -> (usize, usize, Vec<f64>) {
    match e.array(name).unwrap_or_else(|| panic!("missing {name}")) {
        Array::Numeric(m) => (m.rows(), m.cols(), m.data().to_vec()),
        Array::Logical(l) => (
            l.rows(),
            l.cols(),
            l.data().iter().map(|&b| f64::from(b)).collect(),
        ),
        other => panic!("{name}: unexpected class {}", other.class()),
    }
}

fn scalar(e: &Engine, name: &str) -> f64 {
    let (r, c, v) = values(e, name);
    assert_eq!((r, c), (1, 1), "{name} is not a scalar");
    v[0]
}

fn close(actual: &[f64], expected: &[f64], rel: f64, what: &str) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "{what}: {actual:?} vs {expected:?}"
    );
    for (i, (a, b)) in actual.iter().zip(expected).enumerate() {
        let ok = if b.is_nan() {
            a.is_nan()
        } else if b.is_infinite() {
            a == b
        } else {
            (a - b).abs() <= rel * b.abs() + 1e-13
        };
        assert!(ok, "{what}[{i}]: {a} vs {b} (rel {rel})");
    }
}

fn axes_and_series(e: &Engine) -> Vec<usize> {
    e.figure
        .as_ref()
        .expect("figure exists")
        .all_axes()
        .map(|(_, a)| a.series.len())
        .collect()
}

#[test]
fn nichols_outputs_equal_bode_outputs() {
    for setup in [
        "G=tf(1,[1 0.4 1]); w=[];",
        "G=tf([1 2],[1 3 3 1]); w=[];",
        "G=ss([-1 2;-3 -4],[1 0;0.5 1],[1 1;0 2],[0.1 0;0 0.2]); w=logspace(-1,1,30);",
        "G=tf(0.2,conv([1 -0.8],[1 -0.5]),0.1); w=[];",
    ] {
        let code = format!(
            "{setup} if isempty(w), [m1,p1,w1]=nichols(G); [m2,p2,w2]=bode(G); else, [m1,p1,w1]=nichols(G,w); [m2,p2,w2]=bode(G,w); end; if ~issiso(G), m2=reshape(permute(m2,[3 1 2]),numel(w2),[]); p2=reshape(permute(p2,[3 1 2]),numel(w2),[]); end; ok=isequal(m1,m2)&&isequal(p1,p2)&&isequal(w1,w2); n=numel(w1);"
        );
        let e = run(&code);
        assert_eq!(scalar(&e, "ok"), 1., "{setup}");
        assert!(scalar(&e, "n") >= 30., "{setup}");
    }
    // Against python-control at fixed frequencies.
    let e = run("w=[0.1 0.5 1 2 10]; [m,p,wo]=nichols(tf(1,[1 0.4 1]),w);");
    let (r, c, m) = values(&e, "m");
    assert_eq!((r, c), (5, 1));
    close(&m, SIGMA_SISO, 1e-12, "nichols magnitude");
    close(
        &values(&e, "p").2,
        NICHOLS_SISO_PHASE,
        1e-12,
        "nichols phase",
    );
    close(&values(&e, "wo").2, SIGMA_SISO_W, 0., "nichols wout");
}

#[test]
fn nichols_and_sigma_accept_a_frequency_range() {
    let e = run(
        "G=tf(1,[1 0.4 1]); [m,p,w]=nichols(G,{0.1,10}); [sv,ws]=sigma(G,{0.1,10}); [m2,p2]=bode(G,w); same=isequal(m,m2)&&isequal(p,p2)&&isequal(w,ws); first=w(1); last=w(end); n=numel(w);",
    );
    assert_eq!(scalar(&e, "same"), 1.);
    assert_eq!(scalar(&e, "first"), 0.1);
    assert_eq!(scalar(&e, "last"), 10.);
    assert_eq!(scalar(&e, "n"), 1000.);
    // A discrete range stops at the Nyquist frequency pi/Ts.
    let e = run("[sv,w]=sigma(tf(1,[1 -0.5],0.1),{0.01,1000}); last=w(end); n=numel(w);");
    assert_eq!(scalar(&e, "last"), std::f64::consts::PI / 0.1);
}

#[test]
fn sigma_siso_is_the_response_magnitude() {
    let e = run("G=tf(1,[1 0.4 1]); [sv,w]=sigma(G,[0.1 0.5 1 2 10]);");
    let (r, c, sv) = values(&e, "sv");
    assert_eq!((r, c), (1, 5));
    close(&sv, SIGMA_SISO, 1e-12, "sigma SISO");
    let (r, c, _) = values(&e, "w");
    assert_eq!((r, c), (5, 1));
    // Automatic grid: bode's grid and |H| exactly.
    let e = run(
        "G=tf([1 2],[1 3 3 1]); [sv,w]=sigma(G); [m,p,wb]=bode(G); ok=isequal(sv',m)&&isequal(w,wb);",
    );
    assert_eq!(scalar(&e, "ok"), 1.);
    // Types on a SISO model: 1/|H|, |1+H|, |1+1/H|.
    let e = run(
        "G=tf(1,[1 0.4 1]); w=[0.1 1 10]; h=squeeze(freqresp(G,w)).'; s1=sigma(G,w,1); s2=sigma(G,w,2); s3=sigma(G,w,3); e1=max(abs(s1-1./abs(h))); e2=max(abs(s2-abs(1+h))); e3=max(abs(s3-abs(1+1./h)));",
    );
    for name in ["e1", "e2", "e3"] {
        assert!(scalar(&e, name) < 1e-12, "{name}");
    }
}

#[test]
fn sigma_mimo_matches_numpy_svd_for_every_type() {
    let prefix = "G=ss([-1 2;-3 -4],[1 0;0.5 1],[1 1;0 2],[0.1 0;0 0.2]); w=[0.1 1 3 10];";
    for (kind, expected) in [
        (0, SIGMA_MIMO_T0),
        (1, SIGMA_MIMO_T1),
        (2, SIGMA_MIMO_T2),
        (3, SIGMA_MIMO_T3),
    ] {
        let e = run(&format!("{prefix} sv=sigma(G,w,{kind});"));
        let (r, c, sv) = values(&e, "sv");
        assert_eq!((r, c), (2, 4), "type {kind}");
        close(&sv, expected, 1e-10, &format!("sigma type {kind}"));
    }
    // Type 0 by default; [] keeps the automatic grid for a type.
    let e = run(&format!(
        "{prefix} a=sigma(G,w); b=sigma(G,w,0); ok=isequal(a,b); [c,wc]=sigma(G,[],2); n=numel(wc); r=size(c,1);"
    ));
    assert_eq!(scalar(&e, "ok"), 1.);
    assert_eq!(scalar(&e, "r"), 2.);
    assert!(scalar(&e, "n") >= 100.);
}

#[test]
fn sigma_discrete_uses_the_unit_circle() {
    let e = run(
        "G=ss([0.5 0.2;-0.1 0.7],eye(2),[1 0.5;0.2 1],zeros(2),0.1); [sv,w]=sigma(G,[0.5 2 10 pi/0.1]);",
    );
    let (r, c, sv) = values(&e, "sv");
    assert_eq!((r, c), (2, 4));
    close(&sv, SIGMA_DISCRETE, 1e-10, "discrete sigma");
    close(&values(&e, "w").2, SIGMA_DISCRETE_W, 0., "discrete w");
    // Non-square systems have min(ny, nu) singular values.
    let e = run("sv=sigma(ss(-eye(2),[1 0 1;0 1 1],eye(2),zeros(2,3)),[1 2]);");
    let (r, c, _) = values(&e, "sv");
    assert_eq!((r, c), (2, 2));
}

#[test]
fn sigma_single_output_multi_input_uses_the_row_norm() {
    // H = D = [3 4] at every frequency: its one singular value is
    // norm([3 4]) = 5 (NumPy linalg.svd([[3, 4]]) -> [5.]), never |H(1,1)| = 3.
    let e = run("sv=sigma(ss(-1,[0 0],0,[3 4]),[1 2]); t=sigma(ss(-1,0,[0;0],[3;4]),[1 2]);");
    let (r, c, sv) = values(&e, "sv");
    assert_eq!((r, c), (1, 2));
    close(&sv, &[5., 5.], 1e-14, "sigma 1x2");
    close(&values(&e, "t").2, &[5., 5.], 1e-14, "sigma 2x1");
}

struct Margins {
    gm: &'static [f64],
    gmf: &'static [f64],
    pm: &'static [f64],
    pmf: &'static [f64],
    dm: &'static [f64],
    stable: f64,
}

fn check_margins(e: &mut Engine, s: &str, m: &Margins, what: &str) {
    e.execute(&format!(
        "q1={s}.GainMargin; q2={s}.GMFrequency; q3={s}.PhaseMargin; q4={s}.PMFrequency; q5={s}.DelayMargin; q6={s}.DMFrequency; q7={s}.Stable;"
    ))
    .unwrap_or_else(|err| panic!("{what}: {err}"));
    for (name, expected) in [
        ("q1", m.gm),
        ("q2", m.gmf),
        ("q3", m.pm),
        ("q4", m.pmf),
        ("q5", m.dm),
        ("q6", m.pmf),
    ] {
        let (r, c, v) = values(e, name);
        assert_eq!((r, c), (1, expected.len()), "{what} {name} shape");
        close(&v, expected, 1e-9, &format!("{what} {name}"));
    }
    let stable = scalar(e, "q7");
    assert!(
        stable == m.stable || (stable.is_nan() && m.stable.is_nan()),
        "{what} Stable {stable}"
    );
}

/// margin's outputs are the dominant entries of allmargin (bit for bit).
fn check_margin_agrees(e: &mut Engine, what: &str) {
    e.execute("[Gm,Pm,Wcg,Wcp]=margin(L); S=allmargin(L); g=S.GainMargin; gf=S.GMFrequency; p=S.PhaseMargin; pf=S.PMFrequency;")
        .unwrap();
    let (g, gf, p, pf) = (
        values(e, "g").2,
        values(e, "gf").2,
        values(e, "p").2,
        values(e, "pf").2,
    );
    let gm = g
        .iter()
        .zip(&gf)
        .fold((f64::INFINITY, f64::NAN), |best, (&v, &w)| {
            if v.ln().abs() < best.0.ln().abs() {
                (v, w)
            } else {
                best
            }
        });
    let pm = p
        .iter()
        .zip(&pf)
        .fold((f64::INFINITY, f64::NAN), |best, (&v, &w)| {
            if v.abs() < best.0.abs() { (v, w) } else { best }
        });
    for (name, expected) in [("Gm", gm.0), ("Wcg", gm.1), ("Pm", pm.0), ("Wcp", pm.1)] {
        let actual = scalar(e, name);
        assert!(
            actual == expected || (actual.is_nan() && expected.is_nan()),
            "{what}: margin {name} {actual} vs allmargin {expected}"
        );
    }
}

#[test]
fn allmargin_lists_every_crossing() {
    for (code, m, what) in [
        (
            "L=tf(1,[1 3 3 1]);",
            Margins {
                gm: AM_CUBIC_GM,
                gmf: AM_CUBIC_GMF,
                pm: AM_CUBIC_PM,
                pmf: AM_CUBIC_PMF,
                dm: AM_CUBIC_DM,
                stable: 1.,
            },
            "1/(s+1)^3",
        ),
        (
            "L=tf(0.5,[1 0.2 1]);",
            Margins {
                gm: AM_TWO_GM,
                gmf: AM_TWO_GMF,
                pm: AM_TWO_PM,
                pmf: AM_TWO_PMF,
                dm: AM_TWO_DM,
                stable: 1.,
            },
            "two gain crossovers",
        ),
        (
            "L=tf(30*conv([1 1],[1 1]),conv([1 0 0 0],conv([0.01 1],[0.01 1])));",
            Margins {
                gm: AM_COND_GM,
                gmf: AM_COND_GMF,
                pm: AM_COND_PM,
                pmf: AM_COND_PMF,
                dm: AM_COND_DM,
                stable: 1.,
            },
            "conditionally stable",
        ),
        (
            "L=tf(10,[1 3 3 1]);",
            Margins {
                gm: AM_UNSTABLE_GM,
                gmf: AM_UNSTABLE_GMF,
                pm: AM_UNSTABLE_PM,
                pmf: AM_UNSTABLE_PMF,
                dm: AM_UNSTABLE_DM,
                stable: 0.,
            },
            "unstable loop",
        ),
        (
            "L=tf(0.2,conv([1 -0.8],[1 -0.5]),0.1);",
            Margins {
                gm: AM_DISCRETE_GM,
                gmf: AM_DISCRETE_GMF,
                pm: AM_DISCRETE_PM,
                pmf: AM_DISCRETE_PMF,
                dm: AM_DISCRETE_DM,
                stable: 1.,
            },
            "discrete (delay in sample periods)",
        ),
    ] {
        let mut e = run(&format!("{code} S=allmargin(L); n=numel(S);"));
        assert_eq!(scalar(&e, "n"), 1., "{what}");
        check_margins(&mut e, "S", &m, what);
        check_margin_agrees(&mut e, what);
        // Model type does not matter.
        let mut e = run(&format!("{code} S=allmargin(ss(L));"));
        check_margins(&mut e, "S", &m, &format!("{what} (ss)"));
    }
    // The cubic loop: GM 8 at sqrt(3) exactly as published.
    let e = run("S=allmargin(tf(1,[1 3 3 1])); g=S.GainMargin; w=S.GMFrequency;");
    close(
        &[scalar(&e, "g"), scalar(&e, "w")],
        &[8., 3f64.sqrt()],
        1e-12,
        "cubic",
    );
}

#[test]
fn allmargin_mimo_is_loop_at_a_time() {
    let mut e =
        run("L=ss(diag([-1 -2]),eye(2),[2 0.5;0.3 3],zeros(2)); S=allmargin(L); [r,c]=size(S);");
    assert_eq!((scalar(&e, "r"), scalar(&e, "c")), (2., 1.));
    check_margins(
        &mut e,
        "S(1)",
        &Margins {
            gm: AM_MIMO1_GM,
            gmf: AM_MIMO1_GMF,
            pm: AM_MIMO1_PM,
            pmf: AM_MIMO1_PMF,
            dm: AM_MIMO1_DM,
            stable: 1.,
        },
        "MIMO loop 1",
    );
    check_margins(
        &mut e,
        "S(2)",
        &Margins {
            gm: AM_MIMO2_GM,
            gmf: AM_MIMO2_GMF,
            pm: AM_MIMO2_PM,
            pmf: AM_MIMO2_PMF,
            dm: AM_MIMO2_DM,
            stable: 1.,
        },
        "MIMO loop 2",
    );
}

#[test]
fn allmargin_data_form_and_focus() {
    // Frequency data: the same crossings by interpolation, Stable NaN.
    let e = run(
        "L=tf(0.5,[1 0.2 1]); w=logspace(-2,2,4000); [m,p]=bode(L,w); S=allmargin(m,p,w); f=S.PMFrequency; s=S.Stable; S2=allmargin(m,p,w,0.1); d=S2.DelayMargin; d0=S.DelayMargin;",
    );
    close(&values(&e, "f").2, AM_TWO_PMF, 1e-4, "data PMFrequency");
    assert!(scalar(&e, "s").is_nan());
    let (d, d0) = (values(&e, "d").2, values(&e, "d0").2);
    close(
        &d,
        &d0.iter().map(|v| v / 0.1).collect::<Vec<_>>(),
        1e-12,
        "data delay in samples",
    );
    // Focus keeps the crossings inside [fmin fmax].
    let e = run("S=allmargin(tf(0.5,[1 0.2 1]),'Focus',[1 10]); f=S.PMFrequency; p=S.PhaseMargin;");
    close(
        &values(&e, "f").2,
        &AM_TWO_PMF[1..],
        1e-9,
        "focus frequency",
    );
    close(&values(&e, "p").2, &AM_TWO_PM[1..], 1e-9, "focus margin");
}

#[test]
fn plot_forms_create_axes_and_lines() {
    let mut e = run("G=tf(1,[1 0.4 1]); nichols(G);");
    assert_eq!(axes_and_series(&e), vec![1]);
    {
        let a = &e.figure.as_ref().unwrap().axes;
        assert_eq!(a.title, "Nichols chart");
        assert!(a.xlabel.contains("phase") && a.ylabel.contains("dB"));
        assert!(!a.xgrid && !a.ygrid, "grid is off by default");
    }
    assert!(e.array("ans").is_none());
    e.execute("figure; nichols(G,'r--',tf(2,[1 1 1]),'b');")
        .unwrap();
    assert_eq!(axes_and_series(&e), vec![2]);
    {
        let a = &e.figure.as_ref().unwrap().axes;
        assert_eq!(a.series[0].pattern, matrust::plot::LinePattern::Dashed);
        assert_eq!(a.series[0].color, [255, 0, 0]);
    }
    e.execute("figure; nichols(ss(-eye(2),eye(2),eye(2),zeros(2)));")
        .unwrap();
    assert_eq!(axes_and_series(&e), vec![1, 1, 1, 1]);
    e.execute("figure; sigma(G);").unwrap();
    assert_eq!(axes_and_series(&e), vec![1]);
    assert_eq!(
        e.figure.as_ref().unwrap().axes.xscale,
        matrust::plot::AxisScale::Log
    );
    e.execute("figure; sigma(ss([-1 2;-3 -4],[1 0;0.5 1],[1 1;0 2],[0.1 0;0 0.2]));")
        .unwrap();
    assert_eq!(axes_and_series(&e), vec![2]);
    e.execute(
        "figure; sigma(G,'k:',ss([-1 2;-3 -4],[1 0;0.5 1],[1 1;0 2],[0.1 0;0 0.2]),{0.1,10},2);",
    )
    .unwrap();
    assert_eq!(axes_and_series(&e), vec![3]);
    // Output forms do not plot.
    let e = run("G=tf(1,[1 1]); [m,p]=nichols(G); sv=sigma(G); S=allmargin(G);");
    assert!(e.figure.is_none());
}

#[test]
fn ngrid_adds_contours_and_labels() {
    let mut e = run("G=tf(4,[1 2 1 0]); nichols(G); ngrid;");
    let a = &e.figure.as_ref().unwrap().axes;
    assert_eq!(
        a.series.len(),
        3,
        "the response plus magnitude and phase contours"
    );
    assert!(a.extras.texts.len() > 10, "contour labels");
    assert_eq!(a.xlim, Some([-360., 0.]));
    let y = a.ylim.unwrap();
    assert!(y[0] <= -40. && y[1] >= 40.);
    assert!(!a.hold, "ngrid restores hold");
    assert_eq!(a.series[1].pattern, matrust::plot::LinePattern::Dotted);
    // Alone on a fresh figure.
    e.execute("figure; ngrid;").unwrap();
    let a = &e.figure.as_ref().unwrap().axes;
    assert_eq!(a.series.len(), 2);
    assert_eq!(a.xlim, Some([-360., 0.]));
    assert_eq!(a.ylim, Some([-40., 40.]));
    // Phase over two turns: two turns of contours.
    e.execute("figure; nichols(tf(1,[1 5 10 10 5 1 0])); ngrid;")
        .unwrap();
    let a = &e.figure.as_ref().unwrap().axes;
    assert_eq!(a.xlim, Some([-720., 0.]));
    // 'new' clears the axes and leaves hold on.
    e.execute("figure; plot(1:3); ngrid('new');").unwrap();
    let a = &e.figure.as_ref().unwrap().axes;
    assert_eq!(a.series.len(), 2);
    assert!(a.hold);
}

#[test]
fn invalid_calls_are_explicit() {
    for code in [
        "nichols();",
        "nichols(3);",
        "[a,b]=nichols(tf(1,[1 1]),tf(1,[1 2]));",
        "[a,b,c,d]=nichols(tf(1,[1 1]));",
        "nichols(tf(1,[1 1]),{1});",
        "nichols(tf(1,[1 1]),{10,1});",
        "nichols(tf(1,[1 1]),[1 2],7);",
        "sv=sigma(ss(-eye(2),[1 0 1;0 1 1],eye(2),zeros(2,3)),[1 2],1);",
        "sv=sigma(tf(1,[1 1]),[1 2],4);",
        "sigma(tf(1,[1 1]),[2 1]);",
        "S=allmargin(ss(-eye(2),[1 0 1;0 1 1],eye(2),zeros(2,3)));",
        "[a,b]=allmargin(tf(1,[1 1]));",
        "S=allmargin(tf(1,[1 1]),'Focus',[2 1]);",
        "S=allmargin(tf(1,[1 1]),3);",
        "S=allmargin([1 2],[3 4]);",
        "ngrid(1);",
        "x=ngrid;",
    ] {
        let mut e = Engine::default();
        assert!(e.execute(code).is_err(), "must reject: {code}");
        assert!(e.figure.is_none(), "invalid call created a figure: {code}");
    }
    let mut e = Engine::default();
    e.execute("a=exist('nichols'); b=exist('sigma'); c=exist('allmargin'); d=exist('ngrid');")
        .unwrap();
    for name in ["a", "b", "c", "d"] {
        assert_eq!(scalar(&e, name), 5., "{name}");
    }
}
