//! firls, kaiserord, firpmord, phasedelay, iirnotch and iirpeak against SciPy, the published
//! Kaiser / Herrmann-Lam-Rabiner formulas and a dense-grid least-squares solve
//! (tool/ref/filter_design_extras.py prints every constant below).
//! kaiserord: Octave signal is not installed here, so the published formula is the only reference.
use matrust::{Array, Engine};

const FIRLS_A: &[f64] = &[
    -0.007144431808656507,
    0.003307845637012877,
    0.023015383097295138,
    0.029423410552935684,
    0.002378241904414767,
    -0.04557753203172371,
    -0.06454950015302342,
    -0.004696591350626556,
    0.13198221386539036,
    0.2766823979794068,
    0.33876869041368807,
    0.2766823979794068,
    0.13198221386539036,
    -0.004696591350626556,
    -0.06454950015302342,
    -0.04557753203172371,
    0.002378241904414767,
    0.029423410552935684,
    0.023015383097295138,
    0.003307845637012877,
    -0.007144431808656507,
];
const FIRLS_B: &[f64] = &[
    -0.0066407557326329135,
    0.007929988100318811,
    0.022983811462777153,
    0.021261181959660296,
    -0.005455456166127949,
    -0.042636781541754785,
    -0.0559818728085621,
    -0.013697261875024687,
    0.08641173865865653,
    0.2069096482570798,
    0.28903909979418146,
    0.28903909979418146,
    0.2069096482570798,
    0.08641173865865653,
    -0.013697261875024687,
    -0.0559818728085621,
    -0.042636781541754785,
    -0.005455456166127949,
    0.021261181959660296,
    0.022983811462777153,
    0.007929988100318811,
    -0.0066407557326329135,
];
const FIRLS_C: &[f64] = &[
    0.0007118731187150634,
    -0.0010828800269093704,
    0.01007345238296952,
    0.017868209772282158,
    -0.010102143512273894,
    -0.03799434478062073,
    -0.011198546104667513,
    0.019347682133989148,
    -0.0013331346200208983,
    0.012666048252974807,
    0.0898348288537014,
    0.04455194763773023,
    -0.16077067750227672,
    -0.19728455859224853,
    0.0848068911579051,
    0.2848730235422748,
    0.0848068911579051,
    -0.19728455859224853,
    -0.16077067750227672,
    0.04455194763773023,
    0.0898348288537014,
    0.012666048252974807,
    -0.0013331346200208983,
    0.019347682133989148,
    -0.011198546104667513,
    -0.03799434478062073,
    -0.010102143512273894,
    0.017868209772282158,
    0.01007345238296952,
    -0.0010828800269093704,
    0.0007118731187150634,
];
const FIRLS_D: &[f64] = &[
    -9.893544276919818e-05,
    -0.0056529228142357685,
    -0.011663093701203536,
    -0.0067986962179321,
    0.0009312163003802534,
    -0.023411520262726946,
    -0.102629600535786,
    -0.20232318772288768,
    0.7514451054296625,
    -0.20232318772288768,
    -0.102629600535786,
    -0.023411520262726946,
    0.0009312163003802534,
    -0.0067986962179321,
    -0.011663093701203536,
    -0.0056529228142357685,
    -9.893544276919818e-05,
];
const FIRLS_H20: &[f64] = &[
    -3.984830702777215e-18,
    -0.017085226889717555,
    1.0552827597214606e-16,
    -0.042041966314331394,
    3.639267435211276e-17,
    -0.08795178011846137,
    1.5153070349107168e-16,
    -0.1866167289819627,
    3.7490698539076415e-17,
    -0.6277416231785854,
    0.0,
    0.6277416231785854,
    -3.7490698539076415e-17,
    0.1866167289819627,
    -1.5153070349107168e-16,
    0.08795178011846137,
    -3.639267435211276e-17,
    0.042041966314331394,
    -1.0552827597214606e-16,
    0.017085226889717555,
    3.984830702777215e-18,
];
const FIRLS_H21: &[f64] = &[
    -0.015974439045676383,
    -0.017167942291401832,
    -0.025515289280366695,
    -0.02834198598125943,
    -0.03990627620856284,
    -0.0465039910445055,
    -0.06490442894779556,
    -0.08270409651388498,
    -0.12497431135321069,
    -0.20738188255429008,
    -0.6378682318867124,
    0.6378682318867124,
    0.20738188255429008,
    0.12497431135321069,
    0.08270409651388498,
    0.06490442894779556,
    0.0465039910445055,
    0.03990627620856284,
    0.02834198598125943,
    0.025515289280366695,
    0.017167942291401832,
    0.015974439045676383,
];
const FIRLS_H31W: &[f64] = &[
    -0.0012240014760586504,
    -0.0017519001648524154,
    -0.0035335485783636045,
    -0.003988344736892584,
    -0.007573201312059281,
    -0.009389432389297651,
    -0.014953487296326392,
    -0.01730680497402802,
    -0.02581546073773284,
    -0.03161381758679479,
    -0.04563750218099814,
    -0.057045052217720944,
    -0.08272228704672269,
    -0.11827758979848337,
    -0.21019818658690095,
    -0.633490250753601,
    0.633490250753601,
    0.21019818658690095,
    0.11827758979848337,
    0.08272228704672269,
    0.057045052217720944,
    0.04563750218099814,
    0.03161381758679479,
    0.02581546073773284,
    0.01730680497402802,
    0.014953487296326392,
    0.009389432389297651,
    0.007573201312059281,
    0.003988344736892584,
    0.0035335485783636045,
    0.0017519001648524154,
    0.0012240014760586504,
];
const FIRLS_D20: &[f64] = &[
    0.002722805839986512,
    -0.007837625793038414,
    0.013324939168297533,
    -0.02054970951111173,
    0.0302562949234617,
    -0.043585138840953205,
    0.06276947980819979,
    -0.0930430696128212,
    0.15022495722440368,
    -0.3137765428551082,
    0.0,
    0.3137765428551082,
    -0.15022495722440368,
    0.0930430696128212,
    -0.06276947980819979,
    0.043585138840953205,
    -0.0302562949234617,
    0.02054970951111173,
    -0.013324939168297533,
    0.007837625793038414,
    -0.002722805839986512,
];
const FIRLS_D21: &[f64] = &[
    -5.8238998997556084e-06,
    3.071104399157739e-05,
    -9.835663221737024e-05,
    0.00025500863665417836,
    -0.000586814925049463,
    0.0012621287809399262,
    -0.0026449361913628163,
    0.005664209706237918,
    -0.013397665180138227,
    0.042069337096385565,
    -0.4022447439098515,
    0.4022447439098515,
    -0.042069337096385565,
    0.013397665180138227,
    -0.005664209706237918,
    0.0026449361913628163,
    -0.0012621287809399262,
    0.000586814925049463,
    -0.00025500863665417836,
    9.835663221737024e-05,
    -3.071104399157739e-05,
    5.8238998997556084e-06,
];
const K1_WN: &[f64] = &[0.4375];
const K2_WN: &[f64] = &[0.35];
const K3_WN: &[f64] = &[0.275, 0.5375];
const K4_WN: &[f64] = &[0.225, 0.6499999999999999];
const P1_FO: &[f64] = &[0.0, 0.5, 0.6, 1.0];
const P1_W: &[f64] = &[1.0, 17.1];
const P2_FO: &[f64] = &[0.0, 0.3, 0.4, 1.0];
const P2_W: &[f64] = &[1.0, 10.0];
const P3_FO: &[f64] = &[0.0, 0.2, 0.3, 0.5, 0.6, 1.0];
const P3_W: &[f64] = &[5.0, 1.0, 5.0];
const P4_FO: &[f64] = &[0.0, 0.25, 0.375, 1.0];
const P4_W: &[f64] = &[10.0, 1.0];
const PD1: &[f64] = &[
    2.6666666666666665,
    2.6977472406311627,
    2.8173297346914374,
    3.14524809476011,
    3.625665916378002,
    3.737682652942013,
    3.809871478798759,
    3.9649436626868173,
];
const PD2: &[f64] = &[
    0.7428571428571429,
    0.7880018259354877,
    0.9021736679549226,
    0.9928851262719852,
    0.9812812853567079,
    0.9137634069311686,
    0.8556739646956562,
    0.8611491175696115,
];
const PD3: &[f64] = &[
    -0.6885245901639341,
    -0.4679844463153131,
    1.6632735318564504,
    1.1422696229476308,
    0.7676427968583843,
    0.4971996409146161,
];
const PDS_SOS: &[f64] = &[
    0.2,
    0.30000000000000004,
    0.1,
    1.0,
    -0.4999999999999999,
    0.19999999999999993,
];
const PDS: &[f64] = &[
    0.9761904761904767,
    1.0181546930589604,
    1.1222421718390958,
    1.1941001133674918,
    1.150782604279572,
    1.0299703262088158,
    0.8771688914904152,
    0.6987356503751915,
];
const N1_B: &[f64] = &[0.9270403427317333, -1.0898012834757453, 0.9270403427317333];
const N1_A: &[f64] = &[1.0, -1.0898012834757453, 0.8540806854634666];
const K1_B: &[f64] = &[0.0729596572682667, 0.0, -0.0729596572682667];
const K1_A: &[f64] = &[1.0, -1.0898012834757453, 0.8540806854634666];
const N2_B: &[f64] = &[
    0.8632712640026805,
    -1.0572023902567724e-16,
    0.8632712640026805,
];
const N2_A: &[f64] = &[1.0, -1.0572023902567724e-16, 0.726542528005361];
const K2_B: &[f64] = &[0.1367287359973195, 0.0, -0.1367287359973195];
const K2_A: &[f64] = &[1.0, -1.0572023902567724e-16, 0.726542528005361];
const N3_B: &[f64] = &[0.7547627247472144, 1.2484991756499184, 0.7547627247472144];
const N3_A: &[f64] = &[1.0, 1.2484991756499184, 0.5095254494944288];
const K3_B: &[f64] = &[0.2452372752527856, 0.0, -0.2452372752527856];
const K3_A: &[f64] = &[1.0, 1.2484991756499184, 0.5095254494944288];
const NG1_B: &[f64] = &[0.7852602902526995, -0.48531754939177607, 0.7852602902526995];
const NG1_A: &[f64] = &[1.0, -0.48531754939177607, 0.570520580505399];
const PG1_B: &[f64] = &[0.1988886156766967, 0.0, -0.1988886156766967];
const PG1_A: &[f64] = &[1.0, -1.1329425846815004, 0.6022227686466066];
const NG2_B: &[f64] = &[0.1647524026968611, 0.19367806516993122, 0.1647524026968611];
const NG2_A: &[f64] = &[1.0, 0.19367806516993122, -0.6704951946062778];

fn run(e: &mut Engine, source: &str) -> String {
    e.execute(source)
        .unwrap_or_else(|err| panic!("{source}: {err}"))
        .output
        .join("")
}
fn err(source: &str) -> String {
    Engine::default().execute(source).expect_err(source)
}
fn numbers(e: &Engine, name: &str) -> Vec<f64> {
    match e.workspace().get(name).unwrap() {
        Array::Numeric(x) => x.data().to_vec(),
        x => panic!("unexpected {}", x.class()),
    }
}
fn close(what: &str, got: &[f64], want: &[f64], tol: f64) {
    assert_eq!(got.len(), want.len(), "{what}: length");
    for (i, (&g, &w)) in got.iter().zip(want).enumerate() {
        assert!(
            (g - w).abs() <= tol * (1. + w.abs()),
            "{what} element {i}: got {g}, expected {w}"
        );
    }
}
fn eval(code: &str, var: &str) -> Vec<f64> {
    let mut e = Engine::default();
    run(&mut e, code);
    numbers(&e, var)
}
const TOL: f64 = 1e-10;

// ---- firls ----
#[test]
fn firls_type1_matches_scipy_with_weights() {
    close(
        "A",
        &eval("h=firls(20,[0 .3 .4 1],[1 1 0 0],[1 10]);", "h"),
        FIRLS_A,
        TOL,
    );
    close(
        "C",
        &eval("h=firls(30,[0 .2 .3 .5 .6 1],[0 0 1 1 0 0],[5 1 5]);", "h"),
        FIRLS_C,
        TOL,
    );
}
#[test]
fn firls_odd_order_is_type2_dense_grid_reference() {
    // SciPy only builds odd numtaps; the type II reference is the dense-grid solve (tool/ref script).
    close(
        "B",
        &eval("h=firls(21,[0 .25 .35 1],[1 1 0 0]);", "h"),
        FIRLS_B,
        TOL,
    );
}
#[test]
fn firls_sloped_band_and_weights() {
    close(
        "D",
        &eval("h=firls(16,[0 .4 .5 1],[0 .8 1 1],[1 2]);", "h"),
        FIRLS_D,
        TOL,
    );
    // default weights equal explicit ones
    let a = eval("h=firls(20,[0 .3 .4 1],[1 1 0 0]);", "h");
    let b = eval("h=firls(20,[0 .3 .4 1],[1 1 0 0],[1 1]);", "h");
    close("default w", &a, &b, 1e-14);
}
#[test]
fn firls_hilbert_type3_and_type4() {
    close(
        "H20",
        &eval("h=firls(20,[.1 .9],[1 1],'hilbert');", "h"),
        FIRLS_H20,
        1e-8,
    );
    close(
        "H21",
        &eval("h=firls(21,[.05 .95],[1 1],'hilbert');", "h"),
        FIRLS_H21,
        1e-8,
    );
    close(
        "H31W",
        &eval("h=firls(31,[.1 .4 .5 .9],[1 1 1 1],[1 3],'hilbert');", "h"),
        FIRLS_H31W,
        1e-8,
    );
}
#[test]
fn firls_differentiator() {
    close(
        "D20",
        &eval("h=firls(20,[0 .9],[0 .9],'differentiator');", "h"),
        FIRLS_D20,
        1e-8,
    );
    close(
        "D21",
        &eval("h=firls(21,[0 .8],[0 .8],'differentiator');", "h"),
        FIRLS_D21,
        1e-8,
    );
}
#[test]
fn firls_errors() {
    assert!(err("firls(20,[0 .3 .2 1],[1 1 0 0])").contains("nondecreasing"));
    assert!(err("firls(20,[0 .3 .4 1.2],[1 1 0 0])").contains("[0, 1]"));
    assert!(err("firls(20,[0 .3 .4],[1 1 0])").contains("even length"));
    assert!(err("firls(20,[0 .3 .4 1],[1 1 0])").contains("same even length"));
    assert!(err("firls(20,[0 .3 .4 1],[1 1 0 0],[1 1 1])").contains("one weight per band"));
    assert!(err("firls(20,[0 .3 .4 1],[1 1 0 0],[1 -1])").contains("positive"));
    assert!(err("firls(0,[0 .3 .4 1],[1 1 0 0])").contains("order"));
    assert!(err("firls(20,[0 .3 .4 1],[1 1 0 0],'bogus')").contains("hilbert"));
}

// ---- kaiserord ----
#[test]
fn kaiserord_matches_published_formula() {
    let mut e = Engine::default();
    run(
        &mut e,
        "[n,wn,beta,ft]=kaiserord([1500 2000],[1 0],[.01 .1],8000);",
    );
    assert_eq!(numbers(&e, "n"), [36.]);
    close("wn", &numbers(&e, "wn"), K1_WN, TOL);
    close("beta", &numbers(&e, "beta"), &[3.3953210522614574], TOL);
    assert!(run(&mut e, "ft").contains("low"));
    // A = 60 dB: beta = 0.1102 (A - 8.7)
    run(
        &mut e,
        "[n,wn,beta,ft]=kaiserord([.3 .4],[1 0],[.001 .001]);",
    );
    assert_eq!(numbers(&e, "n"), [73.]);
    close("wn", &numbers(&e, "wn"), K2_WN, TOL);
    close("beta", &numbers(&e, "beta"), &[5.65326], TOL);
    run(
        &mut e,
        "[n,wn,beta,ft]=kaiserord([1000 1200 2000 2300],[0 1 0],[.05 .01 .05],8000);",
    );
    assert_eq!(numbers(&e, "n"), [90.]);
    close("wn", &numbers(&e, "wn"), K3_WN, TOL);
    assert!(run(&mut e, "ft").contains("bandpass"));
    // A = 20 dB < 21: beta = 0; stop type
    run(
        &mut e,
        "[n,wn,beta,ft]=kaiserord([.2 .25 .6 .7],[1 0 1],[.1 .1 .1]);",
    );
    assert_eq!(numbers(&e, "n"), [34.]);
    close("wn", &numbers(&e, "wn"), K4_WN, TOL);
    assert_eq!(numbers(&e, "beta"), [0.]);
    assert!(run(&mut e, "ft").contains("stop"));
}
#[test]
fn kaiserord_cell_form_and_types() {
    let mut e = Engine::default();
    run(
        &mut e,
        "c=kaiserord([.3 .4],[0 1],[.001 .001],'cell'); n=c{1}; w=c{4}; t=c{3}; s=c{5};",
    );
    assert_eq!(numbers(&e, "n"), [74.]);
    assert_eq!(numbers(&e, "w").len(), 75);
    assert!(run(&mut e, "t").contains("high"));
    assert!(run(&mut e, "s").contains("noscale"));
    run(
        &mut e,
        "[n,wn,b,ft]=kaiserord([.1 .15 .3 .35 .5 .55],[1 0 1 0],[.01 .01 .01 .01]);",
    );
    assert!(run(&mut e, "ft").contains("DC-1"));
}
#[test]
fn kaiserord_errors() {
    assert!(err("kaiserord([.3 .4],[1 0],[.01 0])").contains("positive"));
    assert!(err("kaiserord([.3 .4],[1 0],[.01 -.1])").contains("positive"));
    assert!(err("kaiserord([.3 .4 .5],[1 0],[.01 .1])").contains("2*length"));
    assert!(err("kaiserord([.3 .4],[1 0],[.01])").contains("same length"));
    assert!(err("kaiserord([.3 1.4],[1 0],[.01 .1])").contains("between 0"));
    assert!(err("kaiserord([.4 .3],[1 0],[.01 .1])").contains("nondecreasing"));
    assert!(err("kaiserord([.3 .4],[1 0],[.01 .1],0)").contains("positive"));
}

// ---- firpmord ----
#[test]
fn firpmord_matches_hlr_formula() {
    // firpmord.m divides edges by fs (default 2): Herrmann's width is in cycles/sample.
    let mut e = Engine::default();
    for (code, n, fo, w) in [
        (
            "[n,fo,ao,w]=firpmord([500 600],[1 0],[.1710 .01],2000);",
            22.,
            P1_FO,
            P1_W,
        ),
        (
            "[n,fo,ao,w]=firpmord([.3 .4],[1 0],[.01 .001]);",
            51.,
            P2_FO,
            P2_W,
        ),
        (
            "[n,fo,ao,w]=firpmord([.2 .3 .5 .6],[0 1 0],[.01 .05 .01]);",
            29.,
            P3_FO,
            P3_W,
        ),
        (
            "[n,fo,ao,w]=firpmord([1000 1500],[0 1],[.001 .01],8000);",
            40.,
            P4_FO,
            P4_W,
        ),
    ] {
        run(&mut e, code);
        assert_eq!(numbers(&e, "n"), [n], "{code}");
        close("fo", &numbers(&e, "fo"), fo, TOL);
        close("w", &numbers(&e, "w"), w, TOL);
    }
    run(
        &mut e,
        "[n,fo,ao,w]=firpmord([.2 .3 .5 .6],[0 1 0],[.01 .05 .01]);",
    );
    assert_eq!(numbers(&e, "ao"), [0., 0., 1., 1., 0., 0.]);
    run(
        &mut e,
        "c=firpmord([.3 .4],[1 0],[.01 .001],'cell'); n=c{1}; w=c{4}; fo=c{2};",
    );
    assert_eq!(numbers(&e, "n"), [51.]);
    close("w", &numbers(&e, "w"), P2_W, TOL);
    close("fo", &numbers(&e, "fo"), P2_FO, TOL);
}
#[test]
fn firpmord_errors() {
    assert!(err("firpmord([.3 .4],[1 0],[.01 0])").contains("positive"));
    assert!(err("firpmord([.3 .4],[1 0],[.01 .1 .1])").contains("same length"));
    assert!(err("firpmord([.3],[1 0],[.01 .1])").contains("2*length"));
    assert!(err("firpmord([.3 .4],[1 0],[.01 .1],-1)").contains("positive"));
    assert!(err("firpmord([.3 1.2],[1 0],[.01 .1])").contains("between 0"));
    assert!(err("firpmord([.3 .4],[1 0],[.01 .1],'foo')").contains("'cell'"));
}

// ---- phasedelay ----
#[test]
fn phasedelay_matches_scipy_freqz() {
    let mut e = Engine::default();
    run(&mut e, "[p,w]=phasedelay([1 2 3 4 5],1,8);");
    close("PD1", &numbers(&e, "p"), PD1, TOL);
    close(
        "w",
        &numbers(&e, "w"),
        &(0..8)
            .map(|i| i as f64 * std::f64::consts::PI / 8.)
            .collect::<Vec<_>>(),
        TOL,
    );
    run(&mut e, "p=phasedelay([.2 .3],[1 -.5 .2],8);");
    close("PD2", &numbers(&e, "p"), PD2, TOL);
    run(&mut e, "p=phasedelay([1 .5 -.25],[1 -1.2 .81],6);");
    close("PD3", &numbers(&e, "p"), PD3, TOL);
}
#[test]
fn phasedelay_fs_and_sos() {
    let mut e = Engine::default();
    run(&mut e, "[p,w]=phasedelay([1 2 3 4 5],1,8,1000);");
    close("PD1 fs", &numbers(&e, "p"), PD1, TOL);
    close(
        "w Hz",
        &numbers(&e, "w"),
        &(0..8).map(|i| i as f64 * 62.5).collect::<Vec<_>>(),
        TOL,
    );
    let sos = PDS_SOS
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    run(
        &mut e,
        &format!("s=[{sos}]; s=reshape(s,6,[])'; [p,w]=phasedelay(s,8);"),
    );
    close("PDS", &numbers(&e, "p"), PDS, 1e-9);
}
#[test]
fn phasedelay_plots_without_outputs() {
    let mut e = Engine::default();
    run(&mut e, "phasedelay([1 2 3 4 5],1,64)");
    assert!(e.figure.is_some());
}
#[test]
fn review_1111_phasedelay_default_length_is_512() {
    // Help and MathWorks: n defaults to 512 points on [0, pi). b = [1 1] is
    // linear phase with delay (length-1)/2 = 0.5 sample, including at w = 0.
    let mut e = Engine::default();
    run(&mut e, "[phi,w]=phasedelay([1 1],1);");
    let phi = numbers(&e, "phi");
    let w = numbers(&e, "w");
    assert_eq!(phi.len(), 512);
    assert_eq!(w.len(), 512);
    let pi = std::f64::consts::PI;
    close("w0", &[w[0]], &[0.], TOL);
    close("w1", &[w[1]], &[pi / 512.], TOL);
    close("w last", &[w[511]], &[511. * pi / 512.], TOL);
    for (i, &p) in phi.iter().enumerate() {
        assert!((p - 0.5).abs() <= TOL, "phi[{i}] = {p}");
    }
}
#[test]
fn phasedelay_errors() {
    assert!(err("phasedelay([1 2],1,0)").contains("point count"));
    assert!(err("phasedelay([1 2],1,70000)").contains("point count"));
    assert!(err("phasedelay([1 2])").contains("expects"));
    assert!(err("phasedelay([1 2],1,8,-1)").contains("positive"));
    assert!(err("phasedelay('a',1,8)").contains("numeric"));
}

// ---- iirnotch / iirpeak ----
#[test]
fn iirnotch_iirpeak_match_scipy_default_level() {
    let mut e = Engine::default();
    for (w0, bw, nb, na, pb, pa) in [
        (".3", ".05", N1_B, N1_A, K1_B, K1_A),
        (".5", ".1", N2_B, N2_A, K2_B, K2_A),
        (".81", ".2", N3_B, N3_A, K3_B, K3_A),
    ] {
        run(&mut e, &format!("[b,a]=iirnotch({w0},{bw});"));
        close("notch b", &numbers(&e, "b"), nb, TOL);
        close("notch a", &numbers(&e, "a"), na, TOL);
        run(&mut e, &format!("[b,a]=iirpeak({w0},{bw});"));
        close("peak b", &numbers(&e, "b"), pb, TOL);
        close("peak a", &numbers(&e, "a"), pa, TOL);
    }
}
#[test]
fn iirnotch_iirpeak_general_level() {
    let mut e = Engine::default();
    for (code, b, a) in [
        ("[b,a]=iirnotch(.4,.1,-6);", NG1_B, NG1_A),
        ("[b,a]=iirpeak(.25,.08,-1);", PG1_B, PG1_A),
        ("[b,a]=iirnotch(.7,.3,-20);", NG2_B, NG2_A),
    ] {
        run(&mut e, code);
        close(code, &numbers(&e, "b"), b, TOL);
        close(code, &numbers(&e, "a"), a, TOL);
    }
    // explicit -10*log10(2) equals the default
    let x = eval("[b,a]=iirnotch(.3,.05,-10*log10(2));", "b");
    close("default level", &x, N1_B, 1e-12);
}
#[test]
fn iirnotch_iirpeak_errors() {
    assert!(err("iirnotch(0,.1)").contains("W0"));
    assert!(err("iirnotch(1,.1)").contains("W0"));
    assert!(err("iirpeak(.3,0)").contains("BW"));
    assert!(err("iirpeak(.3,1)").contains("BW"));
    assert!(err("iirnotch(.3,.1,3)").contains("negative"));
    assert!(err("iirnotch(.3)").contains("expects"));
    assert!(err("iirnotch([.3 .4],.1)").contains("scalar"));
}

#[test]
fn kaiserord_highpass_order_is_even_and_feeds_fir1() {
    // Octave signal kaiserord: a passband at Nyquist rounds the order up to even.
    let mut e = Engine::default();
    run(
        &mut e,
        "[n,wn,beta,ft]=kaiserord([.3 .4],[0 1],[.001 .001]);",
    );
    assert_eq!(numbers(&e, "n"), [74.]);
    run(
        &mut e,
        "c=kaiserord([.3 .4],[0 1],[.001 .001],'cell'); b=fir1(c{:});",
    );
    assert_eq!(numbers(&e, "b").len(), 75);
}

#[test]
fn bilinear_rejects_more_zeros_than_poles() {
    assert!(err("[zd,pd,kd]=bilinear([-1;-2],-3,1,10);").contains("more zeros than poles"));
}

#[test]
fn top_order_band_sos_runs_through_freqz_and_sosfilt() {
    let mut e = Engine::default();
    run(
        &mut e,
        "s=butter(65,[.2 .4],'sos'); h=freqz(s,32); y=sosfilt(s,ones(1,8));",
    );
    assert_eq!(numbers(&e, "y").len(), 8);
}
