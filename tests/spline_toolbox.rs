//! csapi, csape, spaps, spap2, spmak, fnbrk, aptknt, augknt and fnval/fnder/fnint/fnplt
//! for pp and B-forms. Expected values from tool/refs/spline_toolbox/spline_toolbox.py
//! (SciPy CubicSpline, make_smoothing_spline with lam = 1/rho, make_lsq_spline with
//! weights sqrt(w), BSpline.derivative/antiderivative, make_interp_spline). Data: the rows of
//! SETUP; xx = linspace(0, 5.2, 14).
use matrust::{Array, Engine, engine::Limits};

#[rustfmt::skip]
const CSAPI: &[f64] = &[1.2, 1.9, 1.3071004819747705, 1.7052313600798958, 2.9611683022824384, 2.5407792754925467, 1.8933776410281213, 2.8378130821371936, 3.876145765087483, 3.427679531738212, 3.134265004494106, 3.8302686877388332, 4.496304009062966, 3.8999999999999995];
#[rustfmt::skip]
const CSAPI_OUT: &[f64] = &[-5.003239891682726, -0.4979848419587576];
#[rustfmt::skip]
const CSAPI_Y2: &[f64] = &[0.5, -0.3, 0.44327355907549437, 0.7236644361224743, -0.12776819533827666, -0.7947497232142744, -0.8691714617027767, -0.12161324968241795, 0.795561950280179, 1.1543439567248577, 1.0299933798115726, 0.5857048283022458, 0.08428855150932558, -0.2];
#[rustfmt::skip]
const CSAPI3: &[f64] = &[0.25, 1.0, 9.0];
#[rustfmt::skip]
const CSAPE_COMPLETE: &[f64] = &[1.2, 1.9, 1.4552371184056245, 1.6846728686217787, 2.9710446219924482, 2.558600428817578, 1.8969856495401412, 2.839910355687298, 3.8614761641888395, 3.3880354925880893, 3.1664072132882866, 3.9675318140033644, 4.382413905820136, 3.9];
#[rustfmt::skip]
const CSAPE_SECOND: &[f64] = &[1.2, 1.9, 1.3941592864977312, 1.693155340857971, 2.9669471967499126, 2.551071646629043, 1.895326476350463, 2.83927356603109, 3.8664741764263986, 3.4016566944498736, 3.1553355527586024, 3.9202228477228145, 4.421674473175578, 3.9000000000000004];
#[rustfmt::skip]
const CSAPE_VARIATIONAL: &[f64] = &[1.2, 1.9, 1.3904385892687712, 1.6936825909360638, 2.9666535470590705, 2.5502984902269237, 1.894927258111435, 2.8396308756026944, 3.86495329810708, 3.397752107228563, 3.1584509210484097, 3.933477544682013, 4.41068994410642, 3.9000000000000004];
#[rustfmt::skip]
const CSAPE_PERIODIC: &[f64] = &[1.2, 1.9, 1.540104836253754, 1.673276050660089, 2.9751069667173358, 2.5574147058460808, 1.888250561469547, 2.8554630240233823, 3.7869379023756475, 3.1937929890604138, 3.3221306383903038, 4.630816332431232, 3.8109338541677684, 1.2];
#[rustfmt::skip]
const CSAPE_LAGRANGE: &[f64] = &[1.2, 1.9, 1.333672828821915, 1.7015597019002409, 2.9628726194943584, 2.543495618745277, 1.8935694819499416, 2.8387942447558667, 3.870726551119374, 3.4133375819005156, 3.145818729351677, 3.8795358159126136, 4.455445472279684, 3.9];
#[rustfmt::skip]
const CSAPE_MIXED: &[f64] = &[1.2, 1.9, 1.4552723067023061, 1.6846370755262479, 2.971176373324628, 2.55952872734637, 1.8978624553767687, 2.8387471139046285, 3.866835516480785, 3.4019355482183764, 3.1552803932365032, 3.9201551992523163, 4.421685891642686, 3.8999999999999995];
#[rustfmt::skip]
const SPAPS_RHO: &[f64] = &[169.73812329424007];
#[rustfmt::skip]
const SPAPS: &[f64] = &[1.4196199611620117, 1.687947106712696, 1.6902790205345575, 1.998202824605105, 2.5467097695845946, 2.458677231585482, 2.3131295708668502, 2.8173944458374702, 3.339564447032858, 3.2671400892445384, 3.34305447032676, 3.8814351326833934, 4.247149117124667, 4.11716145816788];
#[rustfmt::skip]
const SPAPS_SITES: &[f64] = &[1.4196199611620117, 1.687947106712696, 1.8540156057521051, 2.4697030018714865, 2.458862674255611, 3.180191543277085, 3.2686015297774462, 4.210532447436506, 4.11716145816788];
#[rustfmt::skip]
const SPAPS_W_RHO: &[f64] = &[34.171812918596075];
#[rustfmt::skip]
const SPAPS_W: &[f64] = &[1.3276904952939412, 1.7718366240483199, 2.0301550782647624, 2.288235692302936, 2.5485345199579332, 2.5725311866675, 2.6439940117316176, 3.0816318024850995, 3.4972242251180465, 3.54717209574363, 3.6391368119473375, 3.945140237535964, 4.115215488898611, 3.981767737567436];
#[rustfmt::skip]
const SPAPS_FIXED: &[f64] = &[1.4625369408497213, 1.6718563045656445, 1.8730875489629213, 2.0793279528971094, 2.281518879931718, 2.450844201283427, 2.618745839847259, 2.821844013590229, 3.0425461423306204, 3.259859406819364, 3.5032639823631966, 3.7813506126737453, 4.05783117457156, 4.3126849361679565];
#[rustfmt::skip]
const SPAPS_LINE: &[f64] = &[1.4141613526812622, 1.6267755824462753, 1.8393898122112886, 2.0520040419763017, 2.264618271741315, 2.477232501506328, 2.6898467312713414, 2.902460961036354, 3.1150751908013676, 3.3276894205663803, 3.540303650331394, 3.7529178800964074, 3.96553210986142, 4.178146339626434];
#[rustfmt::skip]
const SPAPS_LINE_E: &[f64] = &[1.3992345774126342];
#[rustfmt::skip]
const SPAP2_C: &[f64] = &[1.2787143631024873, 1.8134582596361934, 1.9376728835223922, 3.0224919820806266, 2.7835477923611687, 4.908726101335087, 3.923201437883522];
#[rustfmt::skip]
const SPAP2: &[f64] = &[1.2787143631024873, 1.6562146382665253, 1.8740179672896298, 2.040907357407777, 2.253465271002258, 2.5020842430153785, 2.722480292559167, 2.854149237411895, 2.9339153214035303, 3.0959312144157454, 3.4770286952391327, 4.0261795661983015, 4.327503609208717, 3.923201437883522];
#[rustfmt::skip]
const SPAP2_W: &[f64] = &[1.2482078897997553, 1.8250073178554467, 2.1205390640341895, 2.29944638940644, 2.510605059520871, 2.765655230755019, 3.0055753202961824, 3.1739654347745923, 3.2881561082672546, 3.4392083022979505, 3.7203063516881216, 4.093676598490967, 4.267205481449473, 3.919557407862466];
#[rustfmt::skip]
const SPAP2_K2: &[f64] = &[1.3900347473651145, 1.5832754210581426, 1.776516094751171, 1.969782800621205, 2.1630755386682448, 2.3563682767152847, 2.669585861089784, 2.9828034454642833, 3.1859227856895194, 3.278943881765493, 3.371964977841466, 3.6524347274997497, 3.932904477158033, 4.213374226816315];
#[rustfmt::skip]
const SPAP2_V: &[f64] = &[1.2787143631024873, 0.4329768514782751, 1.6562146382665253, -0.09336622214069114, 1.8740179672896298, 0.11217519304027182, 2.040907357407777, 0.4347002432416373, 2.253465271002258, 0.3328732142731719, 2.5020842430153785, -0.09435243014854235, 2.722480292559167, -0.4183424155549024, 2.854149237411895, -0.22737077237375378, 2.9339153214035303, 0.4387308559578779, 3.0959312144157454, 1.0885732129937888, 3.4770286952391327, 1.2140971465103083, 4.0261795661983015, 0.6657718184325458, 4.327503609208717, -0.009618288446921246, 3.923201437883522, -0.18528650553372228];
#[rustfmt::skip]
const DER1_T: &[f64] = &[0.0, 0.0, 0.0, 1.3, 2.6, 3.9, 5.2, 5.2, 5.2];
#[rustfmt::skip]
const DER1_C: &[f64] = &[1.2340243766162449, 0.14332456602253701, 0.8344762296601803, -0.1838032228611215, 2.452128818046828, -2.2742876848882254];
#[rustfmt::skip]
const DER1: &[f64] = &[1.2340243766162449, 0.6988032522189362, 0.43553964591157424, 0.44423355769415845, 0.6028795390197778, 0.6132420489959822, 0.46176492667311475, 0.21192323251967524, 0.24456732934665953, 0.623172277622567, 1.3011612236781451, 1.2538433867898406, 0.062027083934381944, -2.2742876848882254];
#[rustfmt::skip]
const DER2: &[f64] = &[-1.6779997086057044, -0.9981059133808383, -0.3182121181559723, 0.3616816770688939, 0.22820581531435064, -0.17639326543332853, -0.5809923461810083, -0.3508408222436933, 0.5140613063786147, 1.3789634350009226, 1.312328490238182, -1.5489176746797029, -4.410163839597587, -7.271410004515467];
#[rustfmt::skip]
const INT_T: &[f64] = &[0.0, 0.0, 0.0, 0.0, 0.0, 1.3, 2.6, 3.9, 5.2, 5.2, 5.2, 5.2, 5.2];
#[rustfmt::skip]
const INT_C: &[f64] = &[0.0, 0.4155821680083084, 1.5943300367718343, 3.4835610982061667, 7.412800674910981, 10.12675977246312, 13.317431738330928, 14.592472205643073];
#[rustfmt::skip]
const INT: &[f64] = &[0.0, 0.5941220819324335, 1.3036787844610958, 2.0865479305768093, 2.943205505300686, 3.8941772413045306, 4.941109843383745, 6.059978555501565, 7.2171562126402895, 8.418077453827134, 9.723280256741516, 11.224552813520843, 12.911180332640324, 14.592472205643073];
#[rustfmt::skip]
const APTKNT: &[f64] = &[0.0, 0.0, 0.0, 0.0, 1.0, 1.7333333333333334, 2.3666666666666667, 3.1666666666666665, 3.866666666666667, 5.2, 5.2, 5.2, 5.2];
#[rustfmt::skip]
const APT_INTERP: &[f64] = &[1.2, 1.8999999999999997, 1.2323468305534167, 1.704917630107483, 3.0615790425267004, 2.6342980643590486, 1.727497442028816, 2.842384058330261, 4.018706438214627, 3.5343247591834093, 3.1219507345461337, 3.8126768735274466, 4.501581553326384, 3.9];

const SETUP: &str = "x = [0 0.4 1.1 1.5 2.6 3.0 3.9 4.7 5.2]; \
    y = [1.2 1.9 1.4 2.8 2.1 3.6 3.1 4.4 3.9]; \
    w = [1 2 0.5 1.5 1 3 0.8 1.2 2]; \
    y2 = [0.5 -0.3 0.8 0.1 -0.6 0.4 1.1 0.2 -0.2]; xx = linspace(0, 5.2, 14); \
    t4 = augknt([0 1.3 2.6 3.9 5.2], 4); \
    wt = [diff(x) 0]/2 + [0 diff(x)]/2;";

fn run(e: &mut Engine, source: &str) {
    e.execute(source)
        .unwrap_or_else(|err| panic!("{source}: {err}"));
}
fn error(e: &mut Engine, source: &str) -> String {
    e.execute(source).expect_err(source)
}
fn numbers(e: &Engine, name: &str) -> Vec<f64> {
    match e.workspace().get(name).unwrap() {
        Array::Numeric(x) => x.data().to_vec(),
        Array::Logical(x) => x.data().iter().map(|v| f64::from(*v)).collect(),
        x => panic!("{name}: unexpected {}", x.class()),
    }
}
fn size(e: &Engine, name: &str) -> (usize, usize) {
    let ws = e.workspace();
    let a = ws.get(name).unwrap();
    (a.rows(), a.cols())
}
fn text(e: &Engine, name: &str) -> String {
    match e.workspace().get(name).unwrap() {
        Array::Character(c) => String::from_utf16_lossy(c.units()),
        x => panic!("{name}: unexpected {}", x.class()),
    }
}
/// Relative closeness |a - b| <= tol * (1 + |b|).
fn close(what: &str, got: &[f64], want: &[f64], tol: f64) {
    assert_eq!(got.len(), want.len(), "{what}: {got:?} vs {want:?}");
    for (i, (a, b)) in got.iter().zip(want).enumerate() {
        assert!(
            (a - b).abs() <= tol * (1. + b.abs()),
            "{what}[{i}]: got {a}, expected {b}"
        );
    }
}
fn engine() -> Engine {
    let mut e = Engine::default();
    run(&mut e, SETUP);
    e
}

#[test]
fn csapi_matches_scipy_not_a_knot() {
    let mut e = engine();
    run(
        &mut e,
        "v = csapi(x, y, xx); pp = csapi(x, y); a = fnval(pp, xx); o = fnval(pp, [-0.5 5.7]); \
         f = pp.form; k = pp.order; l = pp.pieces; \
         p = randperm(9); u = csapi(x(p), y(p), xx); \
         V = csapi(x, [y; y2], xx); c = csapi(x', y', xx');",
    );
    close("values", &numbers(&e, "v"), CSAPI, 1e-10);
    assert_eq!(size(&e, "v"), (1, 14));
    close("pp", &numbers(&e, "a"), CSAPI, 1e-10);
    close("extrapolated", &numbers(&e, "o"), CSAPI_OUT, 1e-10);
    assert_eq!(text(&e, "f"), "pp");
    assert_eq!((numbers(&e, "k"), numbers(&e, "l")), (vec![4.], vec![8.]));
    close("unsorted sites", &numbers(&e, "u"), CSAPI, 1e-10);
    let vv = numbers(&e, "V");
    assert_eq!(size(&e, "V"), (2, 14));
    let row = |r: usize| -> Vec<f64> { vv.iter().skip(r).step_by(2).copied().collect() };
    close("component 1", &row(0), CSAPI, 1e-10);
    close("component 2", &row(1), CSAPI_Y2, 1e-10);
    assert_eq!(size(&e, "c"), (14, 1));
    run(
        &mut e,
        "q = csapi([0 1 3], [1 0 4], [0.5 2 4]); l2 = csapi([1 3], [2 6], [0 2]);",
    );
    close(
        "three sites: the parabola",
        &numbers(&e, "q"),
        CSAPI3,
        1e-12,
    );
    close("two sites: the line", &numbers(&e, "l2"), &[0., 4.], 1e-14);
}

#[test]
fn csape_end_conditions_match_scipy() {
    let mut e = engine();
    run(
        &mut e,
        "a = fnval(csape(x, [0.5 y -1], 'complete'), xx); \
         b = fnval(csape(x, y, 'clamped', [0.5 -1]), xx); \
         c = fnval(csape(x, [1 y -2], 'second'), xx); \
         d = fnval(csape(x, y, 'variational'), xx); \
         yp = y; yp(end) = yp(1); p = fnval(csape(x, yp, 'periodic'), xx); \
         g = fnval(csape(x, y), xx); g0 = fnval(csape(x, y, [0 0]), xx); \
         m = fnval(csape(x, y, [1 2], [0.5 -2]), xx); \
         n = fnval(csape(x, y, 'not-a-knot'), xx);",
    );
    close("complete", &numbers(&e, "a"), CSAPE_COMPLETE, 1e-10);
    close(
        "clamped + valconds",
        &numbers(&e, "b"),
        CSAPE_COMPLETE,
        1e-10,
    );
    close("second", &numbers(&e, "c"), CSAPE_SECOND, 1e-10);
    close("variational", &numbers(&e, "d"), CSAPE_VARIATIONAL, 1e-10);
    close("periodic", &numbers(&e, "p"), CSAPE_PERIODIC, 1e-10);
    close("Lagrange default", &numbers(&e, "g"), CSAPE_LAGRANGE, 1e-10);
    close("[0 0]", &numbers(&e, "g0"), CSAPE_LAGRANGE, 1e-10);
    close("[1 2]", &numbers(&e, "m"), CSAPE_MIXED, 1e-10);
    close("not-a-knot", &numbers(&e, "n"), CSAPI, 1e-10);
}

#[test]
fn spaps_reaches_the_tolerance() {
    let mut e = engine();
    run(
        &mut e,
        "[sp, v, rho] = spaps(x, y, 0.5); s = fnval(sp, xx); E = sum(wt .* (y - v).^2); \
         f = sp.form; k = sp.order; n = sp.number; t = sp.knots; t0 = augknt(x, 4); \
         [sw, vw, rw] = spaps(x, y, 1.5, w); sw = fnval(sw, xx); Ew = sum(w .* (y - vw).^2); \
         [sf, ~, rf] = spaps(x, y, -4); sf = fnval(sf, xx); \
         [sl, vl, rl] = spaps(x, y, 2); sl = fnval(sl, xx); \
         [s0, v0, r0] = spaps(x, y, 0);",
    );
    let (rho, err) = (numbers(&e, "rho")[0], numbers(&e, "E")[0]);
    assert!(rho > 0. && err <= 0.5 * (1. + 1e-9), "rho {rho}, E {err}");
    close("rho", &[rho], SPAPS_RHO, 1e-6);
    close("spline", &numbers(&e, "s"), SPAPS, 1e-6);
    close("values", &numbers(&e, "v"), SPAPS_SITES, 1e-6);
    assert_eq!(size(&e, "v"), (1, 9));
    assert_eq!(text(&e, "f"), "B-");
    assert_eq!((numbers(&e, "k"), numbers(&e, "n")), (vec![4.], vec![11.]));
    assert_eq!(numbers(&e, "t"), numbers(&e, "t0"));
    assert!(numbers(&e, "Ew")[0] <= 1.5 * (1. + 1e-9));
    close("weighted rho", &numbers(&e, "rw"), SPAPS_W_RHO, 1e-6);
    close("weighted", &numbers(&e, "sw"), SPAPS_W, 1e-6);
    assert_eq!(numbers(&e, "rf"), vec![4.]);
    close("fixed rho", &numbers(&e, "sf"), SPAPS_FIXED, 1e-9);
    assert_eq!(numbers(&e, "rl"), vec![0.]);
    assert!(SPAPS_LINE_E[0] <= 2.);
    close("line", &numbers(&e, "sl"), SPAPS_LINE, 1e-9);
    assert_eq!(numbers(&e, "r0"), vec![f64::INFINITY]);
    close(
        "tol 0 interpolates",
        &numbers(&e, "v0"),
        &numbers(&e, "y"),
        1e-10,
    );
}

#[test]
fn spap2_matches_make_lsq_spline() {
    let mut e = engine();
    run(
        &mut e,
        "sp = spap2(t4, 4, x, y); c = sp.coefs; v = fnval(sp, xx); n = sp.number; \
         vw = fnval(spap2(t4, 4, x, y, w), xx); \
         v2 = fnval(spap2(augknt([0 1 2 3 4 5.2], 2), 2, x, y), xx); \
         V = fnval(spap2(t4, 4, x, [y; y2]), xx); \
         vl = fnval(spap2(4, 4, x, y), xx); \
         [ta, ka] = aptknt(x, 4); vi = fnval(spap2(ta, ka, x, y), xx);",
    );
    close("coefs", &numbers(&e, "c"), SPAP2_C, 1e-10);
    assert_eq!(size(&e, "c"), (1, 7));
    assert_eq!(numbers(&e, "n"), vec![7.]);
    close("values", &numbers(&e, "v"), SPAP2, 1e-10);
    close("weighted", &numbers(&e, "vw"), SPAP2_W, 1e-10);
    close("order 2", &numbers(&e, "v2"), SPAP2_K2, 1e-10);
    close("two components", &numbers(&e, "V"), SPAP2_V, 1e-10);
    close("l = 4 pieces", &numbers(&e, "vl"), SPAP2, 1e-10);
    close("aptknt", &numbers(&e, "ta"), APTKNT, 1e-14);
    assert_eq!(numbers(&e, "ka"), vec![4.]);
    close(
        "interpolation at aptknt",
        &numbers(&e, "vi"),
        APT_INTERP,
        1e-10,
    );
}

#[test]
fn fnder_and_fnint_match_bspline_derivative_and_antiderivative() {
    let mut e = engine();
    run(
        &mut e,
        "sp = spap2(t4, 4, x, y); d = fnder(sp); dt = d.knots; dc = d.coefs; dv = fnval(d, xx); \
         dk = d.order; d2 = fnval(fnder(sp, 2), xx); \
         i = fnint(sp); it = i.knots; ic = i.coefs; iv = fnval(i, xx); ik = i.order; \
         i5 = fnval(fnint(sp, 5), xx); m = fnval(fnder(sp, -1), xx); \
         back = fnval(fnder(fnint(sp)), xx); v = fnval(sp, xx);",
    );
    close("derivative knots", &numbers(&e, "dt"), DER1_T, 0.);
    close("derivative coefs", &numbers(&e, "dc"), DER1_C, 1e-10);
    close("derivative", &numbers(&e, "dv"), DER1, 1e-10);
    assert_eq!(numbers(&e, "dk"), vec![3.]);
    close("second derivative", &numbers(&e, "d2"), DER2, 1e-10);
    close("integral knots", &numbers(&e, "it"), INT_T, 0.);
    close("integral coefs", &numbers(&e, "ic"), INT_C, 1e-10);
    close("integral", &numbers(&e, "iv"), INT, 1e-10);
    assert_eq!(numbers(&e, "ik"), vec![5.]);
    let shifted: Vec<f64> = INT.iter().map(|v| v + 5.).collect();
    close("integral + 5", &numbers(&e, "i5"), &shifted, 1e-10);
    close("fnder(f, -1)", &numbers(&e, "m"), INT, 1e-10);
    close(
        "fnder(fnint(f))",
        &numbers(&e, "back"),
        &numbers(&e, "v"),
        1e-10,
    );
}

#[test]
fn pp_and_b_forms_agree() {
    let mut e = engine();
    // spaps with rho = 4 (B-form) and csaps with p = rho/(1 + rho) (pp) are one spline.
    run(
        &mut e,
        "sp = spaps(x, y, -4, w); pp = csaps(x, y, 4/5, [], w); \
         a = fnval(sp, xx); b = fnval(pp, xx); \
         a1 = fnval(fnder(sp), xx); b1 = fnval(fnder(pp), xx); \
         a2 = fnval(fnder(sp, 2), xx); b2 = fnval(fnder(pp, 2), xx); \
         a3 = fnval(fnder(sp, 3), xx(2:end-1)); b3 = fnval(fnder(pp, 3), xx(2:end-1)); \
         ai = fnval(fnint(sp), xx); bi = fnval(fnint(pp), xx); \
         pc = fnval(fnder(fnint(pp)), xx); po = fnder(pp).order; \
         ia = fnbrk(fnint(sp), 'interval'); ib = fnbrk(fnint(pp), 'interval');",
    );
    for (u, v) in [("a", "b"), ("a1", "b1"), ("a2", "b2"), ("ai", "bi")] {
        close(u, &numbers(&e, u), &numbers(&e, v), 1e-10);
    }
    // The third derivative jumps at the breaks; compare away from them.
    let (a3, b3) = (numbers(&e, "a3"), numbers(&e, "b3"));
    let xx: Vec<f64> = (1..13).map(|i| 5.2 * i as f64 / 13.).collect();
    let sites = [0., 0.4, 1.1, 1.5, 2.6, 3.0, 3.9, 4.7, 5.2];
    for (i, q) in xx.iter().enumerate() {
        if sites.iter().all(|s| (s - q).abs() > 1e-9) {
            close("third derivative", &[a3[i]], &[b3[i]], 1e-9);
        }
    }
    close(
        "fnder(fnint(pp))",
        &numbers(&e, "pc"),
        &numbers(&e, "b"),
        1e-12,
    );
    assert_eq!(numbers(&e, "po"), vec![3.]);
    assert_eq!(numbers(&e, "ia"), numbers(&e, "ib"));
}

#[test]
fn spmak_fnbrk_augknt_and_fnplt() {
    let mut e = engine();
    run(
        &mut e,
        "sp = spmak([0 0 1 2 2], [1 3 2]); v = fnval(sp, [-1 0 0.5 1 1.5 2 3]); \
         v2 = fnval([0.5 1.5], sp); \
         [t, c, n, k, d] = fnbrk(sp); [o, kk] = fnbrk(sp, 'ord', 'kn'); f = fnbrk(sp, 'form'); \
         iv = fnbrk(sp, 'interval'); br = fnbrk(sp, 'breaks'); pc = fnbrk(sp, 'pieces'); \
         S = spmak(0:5, [1 0; 2 1; 0 3]); S2 = fnval(S, [2 2.5]); sd = S.dim; \
         [a1, l1] = augknt([1 2 3 3 3], 2); [a2, l2] = augknt([3 0 1 2], 3, [2 1]); \
         [a3, l3] = augknt([0 0 0 0 0 1], 4); \
         [ap, kp] = aptknt([0 1 2], 4); \
         pts = fnplt(sp); [pp2, pt] = fnplt(sp, [0.5 1.5]); px = pp2(1,:); py = pp2(2,:); pv = fnval(sp, px); \
         P = fnplt(S);",
    );
    close(
        "hat functions",
        &numbers(&e, "v"),
        &[0., 1., 2., 3., 2.5, 2., 0.],
        1e-15,
    );
    close("fnval(x, f)", &numbers(&e, "v2"), &[2., 2.5], 1e-15);
    assert_eq!(numbers(&e, "t"), vec![0., 0., 1., 2., 2.]);
    assert_eq!(numbers(&e, "c"), vec![1., 3., 2.]);
    assert_eq!(
        (numbers(&e, "n"), numbers(&e, "k"), numbers(&e, "d")),
        (vec![3.], vec![2.], vec![1.])
    );
    assert_eq!((numbers(&e, "o"), numbers(&e, "kk").len()), (vec![2.], 5));
    assert_eq!(text(&e, "f"), "B-");
    assert_eq!(numbers(&e, "iv"), vec![0., 2.]);
    assert_eq!(numbers(&e, "br"), vec![0., 1., 2.]);
    assert_eq!(numbers(&e, "pc"), vec![2.]);
    // Order 4 on 0:5: two uniform cubic B-splines (centred at 2 and 3; 2/3 at the
    // centre, 1/6 one step away, 23/48 half a step away).
    assert_eq!(numbers(&e, "sd"), vec![3.]);
    assert_eq!(size(&e, "S2"), (3, 2));
    close(
        "vector B-form",
        &numbers(&e, "S2"),
        &[2. / 3., 1.5, 0.5, 23. / 48., 69. / 48., 69. / 48.],
        1e-12,
    );
    assert_eq!(numbers(&e, "a1"), vec![1., 1., 2., 3., 3.]);
    assert_eq!(numbers(&e, "l1"), vec![1.]);
    assert_eq!(numbers(&e, "a2"), vec![0., 0., 0., 1., 1., 2., 3., 3., 3.]);
    assert_eq!(numbers(&e, "l2"), vec![2.]);
    assert_eq!(numbers(&e, "a3"), vec![0., 0., 0., 0., 1., 1., 1., 1.]);
    assert_eq!(numbers(&e, "l3"), vec![-1.]);
    assert_eq!(numbers(&e, "ap"), vec![0., 0., 0., 2., 2., 2.]);
    assert_eq!(numbers(&e, "kp"), vec![3.]);
    let (rows, cols) = size(&e, "pts");
    assert_eq!((rows, cols), (2, 101));
    let px = numbers(&e, "px");
    assert_eq!((px[0], px[px.len() - 1]), (0.5, 1.5));
    assert_eq!(px.len(), 101);
    close("fnplt y", &numbers(&e, "py"), &numbers(&e, "pv"), 0.);
    assert_eq!(numbers(&e, "pt"), px);
    assert_eq!(size(&e, "P").0, 3);
    // Statement forms plot and print.
    run(&mut e, "fnplt(sp, 'r-', 2); fnbrk(sp)");
    let out = e.execute("fnbrk(sp)").unwrap().output.join("");
    assert!(out.contains("B- form") && out.contains("order k"), "{out}");
}

#[test]
fn review_b7_csape_empty_data_is_an_error() {
    let mut e = Engine::default();
    let msg = error(&mut e, "csape([], [])");
    assert!(msg.contains("at least one component"), "{msg}");
}

#[test]
fn review_b7_form_products_and_order_are_checked_before_evaluation() {
    let mut e = Engine::default();
    let msg = error(
        &mut e,
        "fnder(struct('form','pp','breaks',0:256,'coefs',[],'order',268435456,'dim',268435456))",
    );
    assert!(msg.contains("dimensions are too large"), "{msg}");
    let msg = error(
        &mut e,
        "fnval(struct('form','B-','knots',0:100000,'coefs',1,'number',1,'order',100000,'dim',1),50000)",
    );
    assert!(msg.contains("order 100000 exceeds 64"), "{msg}");
}

#[test]
fn review_b7_augknt_and_fnplt_admit_before_expanding() {
    let mut e = Engine::with_limits(Limits {
        evaluation_units: None,
        memory_bytes: 1024 * 1024,
        ..Limits::browser()
    });
    let msg = error(&mut e, "augknt(0:20000,64,64)");
    assert!(msg.contains("memory limit"), "{msg}");
    let msg = error(&mut e, "fnplt(spmak([0 0 1 1],ones(2000,2)))");
    assert!(msg.contains("memory limit"), "{msg}");
}

#[test]
fn review_b7_fnint_normalizes_unclamped_left_value_from_scipy() {
    let mut e = Engine::default();
    run(&mut e, "v = fnval(fnint(spmak([0 1 1],1),5),[0 .5 1]);");
    // tool/refs/spline_toolbox/spline_toolbox.py derives these with
    // scipy.interpolate.BSpline.antiderivative.
    close(
        "unclamped fnint",
        &numbers(&e, "v"),
        &[5., 5.125, 5.5],
        1e-14,
    );
}

#[test]
fn review_b7_fnplt_returns_component_rows_then_parameter() {
    let mut e = Engine::default();
    run(
        &mut e,
        "s=spmak([0 0 1 1],[1 2;3 4]); p=fnplt(s); [q,t]=fnplt(s);",
    );
    assert_eq!(size(&e, "p"), (2, 101));
    assert_eq!(size(&e, "q"), (2, 101));
    assert_eq!(size(&e, "t"), (1, 101));
    assert_eq!(numbers(&e, "p"), numbers(&e, "q"));
}

#[test]
fn errors_name_the_problem() {
    let mut e = engine();
    for (src, want) in [
        ("csapi([0 1 1], [1 2 3])", "distinct"),
        ("csapi([0 1 2], [1 2])", "same number"),
        ("csape(x, y, 'bogus')", "unknown end condition"),
        ("csape(x, y, [1 2 3])", "two-element"),
        ("spaps(x, y, 0.5, w, 3)", "not supported yet"),
        ("spaps(x, y, [1 2])", "scalar"),
        ("spaps(x, y, 0.5, [1 2])", "one value per data site"),
        (
            "spap2(augknt([0 0.1 0.2 0.3 5.2], 4), 4, x, y)",
            "Schoenberg-Whitney",
        ),
        ("spap2(t4, 4, [x 6], [y 1])", "basic interval"),
        ("spap2([0 1 1 1 1 1 2], 4, x, y)", "multiplicity"),
        ("spmak([0 1 2], [1 2 3])", "more knots"),
        ("spmak([2 1 0 3], [1 2])", "nondecreasing"),
        ("fnbrk(spmak(0:3, [1 2]), 'zz')", "unknown part"),
        ("fnbrk(csapi(x, y), 'knots')", "not a part of a pp form"),
        ("fnval([1 2], 3)", "(pp) structure"),
        ("fnder(struct('a', 1))", "pp or B-form"),
        ("fnder(csapi(x, y), 0.5)", "integer"),
        ("aptknt([2 1 0], 3)", "nondecreasing"),
        ("augknt([1 2 3], 2, [1 1])", "one entry per interior knot"),
        ("[a, b, c, d] = spaps(x, y, 1)", "output"),
    ] {
        let msg = error(&mut e, src);
        assert!(msg.contains(want), "{src}: {msg}");
    }
}
