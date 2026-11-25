//! Wavelet builtins: wfilters, dwt/idwt in every dwtmode, wavedec/waverec, appcoef,
//! detcoef, wrcoef, wmaxlev, wthresh, thselect, wden and wdenoise. Expected values:
//! PyWavelets 1.10 for the transforms and the published threshold methods
//! (tool/refs/wavelet_dwt/wavelet_dwt.py prints the constants in `refs`). Never MATLAB.
use matrust::{Array, Engine};
use refs::*;

const X13: &str = "[0.5 -1.25 2 3.5 -0.75 1 0 2.25 -2 1.5 0.25 -0.5 3]";
const X5: &str = "[1 -2 0.5 3 -1]";
const X40: &str = "cos((1:40)/3) + 0.1*(1:40)";

fn values(e: &mut Engine, code: &str) -> Vec<f64> {
    e.execute(&format!("zz={code};"))
        .unwrap_or_else(|s| panic!("{code}: {s}"));
    match e.workspace().get("zz").unwrap() {
        Array::Numeric(m) => m.data().to_vec(),
        a => panic!("{code}: got {}", a.class()),
    }
}
fn dims(e: &mut Engine, name: &str) -> Vec<usize> {
    e.workspace().get(name).unwrap().dims().to_vec()
}
fn run(e: &mut Engine, code: &str) -> String {
    e.execute(code)
        .unwrap_or_else(|s| panic!("{code}: {s}"))
        .output
        .join("")
}
fn error(code: &str) -> String {
    Engine::default()
        .execute(code)
        .err()
        .unwrap_or_else(|| panic!("{code}: expected an error"))
}
/// |got - want| <= tol * max(1, |want|) element by element.
fn all_close(got: &[f64], want: &[f64], tol: f64, what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: lengths {got:?} vs {want:?}");
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        assert!(
            (g - w).abs() <= tol * w.abs().max(1.),
            "{what}[{i}]: got {g:.17e}, want {w:.17e}"
        );
    }
}
/// Every 16th sample, the sum and the sum of squares (as the reference script).
fn probe(v: &[f64]) -> Vec<f64> {
    let mut p: Vec<f64> = v.iter().step_by(16).copied().collect();
    p.push(v.iter().sum());
    p.push(v.iter().map(|x| x * x).sum());
    p
}
fn noise(n: usize) -> String {
    format!(
        "m = 1:12*{n}; u = mod(m.^2*7919 + m*104729 + 12345, 1000003)/1000003; \
         e{n} = sum(reshape(u, 12, {n}), 1) - 6;"
    )
}

#[test]
fn wfilters_match_pywavelets() {
    let mut e = Engine::default();
    let cases: [(&str, [&[f64]; 4]); 5] = [
        ("db2", [&WF_DB2_0, &WF_DB2_1, &WF_DB2_2, &WF_DB2_3]),
        ("sym4", [&WF_SYM4_0, &WF_SYM4_1, &WF_SYM4_2, &WF_SYM4_3]),
        (
            "coif1",
            [&WF_COIF1_0, &WF_COIF1_1, &WF_COIF1_2, &WF_COIF1_3],
        ),
        (
            "bior2.2",
            [&WF_BIOR2_2_0, &WF_BIOR2_2_1, &WF_BIOR2_2_2, &WF_BIOR2_2_3],
        ),
        (
            "rbio1.3",
            [&WF_RBIO1_3_0, &WF_RBIO1_3_1, &WF_RBIO1_3_2, &WF_RBIO1_3_3],
        ),
    ];
    for (name, want) in cases {
        run(&mut e, &format!("[f1, f2, f3, f4] = wfilters('{name}');"));
        for (i, w) in want.iter().enumerate() {
            let got = values(&mut e, &format!("f{}", i + 1));
            all_close(&got, w, 1e-12, &format!("{name} filter {i}"));
            assert_eq!(dims(&mut e, &format!("f{}", i + 1)), [1, w.len()]);
        }
    }
    run(
        &mut e,
        "[a, b] = wfilters('db2', 'h'); [c, d] = wfilters('db2', 'l');",
    );
    all_close(&values(&mut e, "a"), &WF_DB2_1, 1e-12, "type h");
    all_close(&values(&mut e, "d"), &WF_DB2_2, 1e-12, "type l");
    all_close(
        &values(&mut e, "wfilters('haar')"),
        &values(&mut e, "wfilters('db1')"),
        0.,
        "haar",
    );
    // Orthonormal families: sum(LoR) = sqrt(2) and unit energy.
    for name in (1..=20)
        .map(|i| format!("db{i}"))
        .chain((2..=20).map(|i| format!("sym{i}")))
        .chain((1..=5).map(|i| format!("coif{i}")))
    {
        // PyWavelets' sym3 table carries ~5e-12 of rounding in its energy.
        let s = values(
            &mut e,
            &format!("[sum(wfilters('{name}')) sum(wfilters('{name}').^2)]"),
        );
        all_close(&s, &[std::f64::consts::SQRT_2, 1.], 1e-10, &name);
    }
}

#[test]
fn dwt_matches_pywavelets_in_every_extension_mode() {
    let mut e = Engine::default();
    let modes: [(&str, &[f64], &[f64]); 9] = [
        ("zpd", &DWT_DB2_ZPD_A, &DWT_DB2_ZPD_D),
        ("sym", &DWT_DB2_SYM_A, &DWT_DB2_SYM_D),
        ("symw", &DWT_DB2_SYMW_A, &DWT_DB2_SYMW_D),
        ("asym", &DWT_DB2_ASYM_A, &DWT_DB2_ASYM_D),
        ("asymw", &DWT_DB2_ASYMW_A, &DWT_DB2_ASYMW_D),
        ("sp0", &DWT_DB2_SP0_A, &DWT_DB2_SP0_D),
        ("sp1", &DWT_DB2_SP1_A, &DWT_DB2_SP1_D),
        ("ppd", &DWT_DB2_PPD_A, &DWT_DB2_PPD_D),
        ("per", &DWT_DB2_PER_A, &DWT_DB2_PER_D),
    ];
    for (mode, a, d) in modes {
        run(
            &mut e,
            &format!("[a, d] = dwt({X13}, 'db2', 'mode', '{mode}');"),
        );
        all_close(&values(&mut e, "a"), a, 1e-10, &format!("db2 {mode} cA"));
        all_close(&values(&mut e, "d"), d, 1e-10, &format!("db2 {mode} cD"));
    }
    let families: [(&str, &str, &[f64], &[f64]); 15] = [
        ("sym4", "sym", &DWT_SYM4_SYM_A, &DWT_SYM4_SYM_D),
        ("sym4", "per", &DWT_SYM4_PER_A, &DWT_SYM4_PER_D),
        ("sym4", "asymw", &DWT_SYM4_ASYMW_A, &DWT_SYM4_ASYMW_D),
        ("coif2", "sym", &DWT_COIF2_SYM_A, &DWT_COIF2_SYM_D),
        ("coif2", "per", &DWT_COIF2_PER_A, &DWT_COIF2_PER_D),
        ("coif2", "asymw", &DWT_COIF2_ASYMW_A, &DWT_COIF2_ASYMW_D),
        ("bior2.4", "sym", &DWT_BIOR2_4_SYM_A, &DWT_BIOR2_4_SYM_D),
        ("bior2.4", "per", &DWT_BIOR2_4_PER_A, &DWT_BIOR2_4_PER_D),
        (
            "bior2.4",
            "asymw",
            &DWT_BIOR2_4_ASYMW_A,
            &DWT_BIOR2_4_ASYMW_D,
        ),
        ("rbio3.3", "sym", &DWT_RBIO3_3_SYM_A, &DWT_RBIO3_3_SYM_D),
        ("rbio3.3", "per", &DWT_RBIO3_3_PER_A, &DWT_RBIO3_3_PER_D),
        (
            "rbio3.3",
            "asymw",
            &DWT_RBIO3_3_ASYMW_A,
            &DWT_RBIO3_3_ASYMW_D,
        ),
        ("haar", "sym", &DWT_HAAR_SYM_A, &DWT_HAAR_SYM_D),
        ("haar", "per", &DWT_HAAR_PER_A, &DWT_HAAR_PER_D),
        ("haar", "asymw", &DWT_HAAR_ASYMW_A, &DWT_HAAR_ASYMW_D),
    ];
    for (w, mode, a, d) in families {
        run(
            &mut e,
            &format!("[a, d] = dwt({X13}, '{w}', 'mode', '{mode}');"),
        );
        all_close(&values(&mut e, "a"), a, 1e-10, &format!("{w} {mode} cA"));
        all_close(&values(&mut e, "d"), d, 1e-10, &format!("{w} {mode} cD"));
    }
    // Extensions longer than the signal (db10 has 20 taps, x has 5 samples).
    let short: [(&str, &[f64], &[f64]); 5] = [
        ("sym", &DWT_DB10_X5_SYM_A, &DWT_DB10_X5_SYM_D),
        ("symw", &DWT_DB10_X5_SYMW_A, &DWT_DB10_X5_SYMW_D),
        ("asym", &DWT_DB10_X5_ASYM_A, &DWT_DB10_X5_ASYM_D),
        ("sp1", &DWT_DB10_X5_SP1_A, &DWT_DB10_X5_SP1_D),
        ("per", &DWT_DB10_X5_PER_A, &DWT_DB10_X5_PER_D),
    ];
    for (mode, a, d) in short {
        run(
            &mut e,
            &format!("[a, d] = dwt({X5}, 'db10', 'mode', '{mode}');"),
        );
        all_close(
            &values(&mut e, "a"),
            a,
            1e-10,
            &format!("db10 x5 {mode} cA"),
        );
        all_close(
            &values(&mut e, "d"),
            d,
            1e-10,
            &format!("db10 x5 {mode} cD"),
        );
    }
    // Filter form and column orientation.
    run(
        &mut e,
        &format!("[lo, hi] = wfilters('db2', 'd'); [a, d] = dwt({X13}', lo, hi);"),
    );
    all_close(&values(&mut e, "a"), &DWT_DB2_SYM_A, 1e-10, "filter form");
    assert_eq!(dims(&mut e, "a"), [DWT_DB2_SYM_A.len(), 1]);
}

#[test]
fn dwtmode_sets_the_global_extension_mode() {
    let mut e = Engine::default();
    let text = run(&mut e, "dwtmode");
    assert!(text.contains("Symmetrization (half-point)"), "{text}");
    let text = run(&mut e, "dwtmode('per')");
    assert!(text.contains("Periodization"), "{text}");
    run(
        &mut e,
        &format!("[a, d] = dwt({X13}, 'db2'); st = dwtmode('status', 'nodisp');"),
    );
    all_close(&values(&mut e, "a"), &DWT_DB2_PER_A, 1e-10, "global per");
    all_close(
        &values(&mut e, "double(st)"),
        &[112., 101., 114.],
        0.,
        "status",
    );
    assert_eq!(run(&mut e, "dwtmode('zpd', 'nodisp')"), "");
    run(&mut e, &format!("[a, d] = dwt({X13}, 'db2');"));
    all_close(&values(&mut e, "d"), &DWT_DB2_ZPD_D, 1e-10, "global zpd");
    // A fresh engine starts in 'sym'.
    let mut f = Engine::default();
    run(&mut f, &format!("[a, d] = dwt({X13}, 'db2');"));
    all_close(&values(&mut f, "a"), &DWT_DB2_SYM_A, 1e-10, "default sym");
}

#[test]
fn wavedec_matches_pywavelets() {
    let mut e = Engine::default();
    let cases: [(&str, &str, &[f64], &[f64]); 3] = [
        ("db3", "sym", &WAVEDEC_DB3_SYM_C, &WAVEDEC_DB3_SYM_L),
        ("sym4", "per", &WAVEDEC_SYM4_PER_C, &WAVEDEC_SYM4_PER_L),
        (
            "bior3.5",
            "zpd",
            &WAVEDEC_BIOR3_5_ZPD_C,
            &WAVEDEC_BIOR3_5_ZPD_L,
        ),
    ];
    for (w, mode, c, l) in cases {
        run(
            &mut e,
            &format!(
                "dwtmode('{mode}', 'nodisp'); [C, L] = wavedec({X40}, 3, '{w}'); x = waverec(C, L, '{w}');"
            ),
        );
        all_close(&values(&mut e, "C"), c, 1e-10, &format!("{w} {mode} C"));
        all_close(&values(&mut e, "L"), l, 0., &format!("{w} {mode} L"));
        all_close(
            &values(&mut e, "x"),
            &values(&mut e, X40),
            1e-10,
            &format!("{w} {mode} waverec"),
        );
    }
    run(
        &mut e,
        &format!("dwtmode('sym', 'nodisp'); [C, L] = wavedec(({X40})', 3, 'db3');"),
    );
    assert_eq!(dims(&mut e, "C"), [WAVEDEC_DB3_SYM_C.len(), 1]);
    assert_eq!(dims(&mut e, "L"), [5, 1]);
}

#[test]
fn appcoef_detcoef_and_wrcoef_split_the_decomposition() {
    let mut e = Engine::default();
    run(
        &mut e,
        &format!("x = {X40}; [C, L] = wavedec(x, 3, 'db3');"),
    );
    all_close(
        &values(&mut e, "appcoef(C, L, 'db3', 2)"),
        &APPCOEF_A2,
        1e-10,
        "appcoef",
    );
    let n3 = WAVEDEC_DB3_SYM_L[0] as usize;
    all_close(
        &values(&mut e, "appcoef(C, L, 'db3')"),
        &WAVEDEC_DB3_SYM_C[..n3],
        1e-12,
        "appcoef top",
    );
    all_close(
        &values(&mut e, "appcoef(C, L, 'db3', 0)"),
        &values(&mut e, "x"),
        1e-10,
        "appcoef 0",
    );
    all_close(
        &values(&mut e, "wrcoef('d', C, L, 'db3', 2)"),
        &WRCOEF_D2,
        1e-10,
        "wrcoef d2",
    );
    all_close(
        &values(&mut e, "wrcoef('a', C, L, 'db3', 2)"),
        &WRCOEF_A2,
        1e-10,
        "wrcoef a2",
    );
    // A3 + D3 + D2 + D1 = x, and A1 + D1 = x.
    let total = values(
        &mut e,
        "wrcoef('a', C, L, 'db3') + wrcoef('d', C, L, 'db3', 3) + wrcoef('d', C, L, 'db3', 2) + wrcoef('d', C, L, 'db3', 1)",
    );
    all_close(&total, &values(&mut e, "x"), 1e-10, "additive branches");
    let total = values(
        &mut e,
        "wrcoef('a', C, L, 'db3', 1) + wrcoef('d', C, L, 'db3', 1)",
    );
    all_close(&total, &values(&mut e, "x"), 1e-10, "A1 + D1");
    // detcoef forms.
    let off = n3 + WAVEDEC_DB3_SYM_L[1] as usize;
    let d2 = &WAVEDEC_DB3_SYM_C[off..off + WAVEDEC_DB3_SYM_L[2] as usize];
    all_close(&values(&mut e, "detcoef(C, L, 2)"), d2, 1e-12, "detcoef 2");
    run(
        &mut e,
        "[p, q] = detcoef(C, L, [2 1]); k = detcoef(C, L, 'cells'); v = detcoef(C, L, [1 3]);",
    );
    all_close(&values(&mut e, "p"), d2, 1e-12, "detcoef outputs");
    all_close(
        &values(
            &mut e,
            "[numel(k) numel(v) isequal(k{2}, p) isequal(v{1}, q)]",
        ),
        &[3., 2., 1., 1.],
        0.,
        "detcoef cells",
    );
    all_close(
        &values(&mut e, "detcoef(C, L)"),
        &values(&mut e, "q"),
        0.,
        "detcoef default 1",
    );
}

#[test]
fn perfect_reconstruction_for_every_wavelet_and_mode() {
    let mut names: Vec<String> = (1..=20).map(|i| format!("db{i}")).collect();
    names.extend((2..=20).map(|i| format!("sym{i}")));
    names.extend((1..=5).map(|i| format!("coif{i}")));
    for o in [
        "1.1", "1.3", "1.5", "2.2", "2.4", "2.6", "2.8", "3.1", "3.3", "3.5", "3.7", "3.9", "4.4",
        "5.5", "6.8",
    ] {
        names.push(format!("bior{o}"));
        names.push(format!("rbio{o}"));
    }
    names.push("haar".into());
    let mut e = Engine::default();
    run(&mut e, "x = sin((1:61).^1.3) + (1:61)/30; y = x(1:50);");
    for (i, w) in names.iter().enumerate() {
        let mode = [
            "sym", "symw", "asym", "asymw", "zpd", "sp0", "sp1", "ppd", "per",
        ][i % 9];
        run(
            &mut e,
            &format!(
                "dwtmode('{mode}', 'nodisp'); [a, d] = dwt(x, '{w}'); r1 = idwt(a, d, '{w}', 61); \
             [C, L] = wavedec(y, 3, '{w}'); r2 = waverec(C, L, '{w}');"
            ),
        );
        let err = values(&mut e, "[max(abs(r1 - x)) max(abs(r2 - y))]");
        assert!(err.iter().all(|&v| v < 1e-10), "{w} {mode}: {err:?}");
    }
    // Every mode once with a long filter on an odd-length signal.
    for mode in [
        "sym", "symw", "asym", "asymw", "zpd", "sp0", "sp1", "ppd", "per",
    ] {
        run(
            &mut e,
            &format!("dwtmode('{mode}', 'nodisp'); [C, L] = wavedec(x, 4, 'sym8');"),
        );
        let err = values(&mut e, "max(abs(waverec(C, L, 'sym8') - x))");
        assert!(err[0] < 1e-10, "sym8 {mode}: {err:?}");
    }
}

#[test]
fn wmaxlev_wthresh_and_thselect() {
    let mut e = Engine::default();
    let got = values(
        &mut e,
        "[wmaxlev(1024, 'sym4') wmaxlev(1000, 'db10') wmaxlev([1 40], 'db3') wmaxlev(5, 'db10') wmaxlev(64, 'haar')]",
    );
    all_close(&got, &WMAXLEV, 0., "wmaxlev");
    all_close(
        &values(&mut e, "wthresh([-3 -1 0.5 2 4], 's', 1)"),
        &[-2., 0., 0., 1., 3.],
        0.,
        "soft",
    );
    all_close(
        &values(&mut e, "wthresh([-3 -1 0.5 2 4], 'h', 1)"),
        &[-3., 0., 0., 2., 4.],
        0.,
        "hard",
    );
    run(&mut e, &noise(64));
    run(&mut e, "s64 = e64 + 4*(mod(0:63, 3) == 0);");
    for (x, want) in [
        ("e64", &THSELECT_NOISE),
        ("s64", &THSELECT_SIGNAL),
        (X13, &THSELECT_SMALL),
    ] {
        let got = values(
            &mut e,
            &format!(
                "[thselect({x}, 'rigrsure') thselect({x}, 'heursure') thselect({x}, 'sqtwolog') thselect({x}, 'minimaxi')]"
            ),
        );
        all_close(&got, want, 1e-10, &format!("thselect {x}"));
    }
}

fn blocks(e: &mut Engine) {
    run(e, &noise(512));
    run(
        e,
        "t = (1:512)/512; pos = [.1 .13 .15 .23 .25 .40 .44 .65 .76 .78 .81]; \
             hgt = [4 -5 3 -4 5 -4.2 2.1 4.3 -3.1 2.1 -4.2]; b = zeros(1, 512); \
             for j = 1:11, b = b + hgt(j)*(1 + sign(t - pos(j)))/2; end; xb = b + 0.5*e512;",
    );
}

#[test]
fn wden_matches_the_published_rules() {
    let mut e = Engine::default();
    blocks(&mut e);
    let cases: [(&str, &str, &[f64], &[f64]); 4] = [
        (
            "'rigrsure', 's', 'mln', 4, 'sym4'",
            "rig",
            &WDEN_RIG_S_MLN,
            &WDEN_RIG_S_MLN_THR,
        ),
        (
            "'heursure', 'h', 'sln', 3, 'db4'",
            "heu",
            &WDEN_HEU_H_SLN,
            &WDEN_HEU_H_SLN_THR,
        ),
        (
            "'sqtwolog', 's', 'one', 3, 'coif2'",
            "sqt",
            &WDEN_SQT_S_ONE,
            &WDEN_SQT_S_ONE_THR,
        ),
        (
            "'minimaxi', 's', 'mln', 4, 'db3'",
            "min",
            &WDEN_MIN_S_MLN,
            &WDEN_MIN_S_MLN_THR,
        ),
    ];
    for (args, tag, xd, thr) in cases {
        run(&mut e, &format!("[xd, cxd, lxd, thr] = wden(xb, {args});"));
        all_close(
            &probe(&values(&mut e, "xd")),
            xd,
            1e-10,
            &format!("wden {tag}"),
        );
        all_close(
            &values(&mut e, "thr"),
            thr,
            1e-10,
            &format!("wden {tag} thr"),
        );
        if tag == "rig" {
            all_close(
                &probe(&values(&mut e, "cxd")),
                &WDEN_RIG_S_MLN_C,
                1e-10,
                "wden cxd",
            );
            // The C, L form gives the same result.
            run(
                &mut e,
                &format!("[C, L] = wavedec(xb, 4, 'sym4'); xd2 = wden(C, L, {args});"),
            );
            all_close(
                &values(&mut e, "xd2"),
                &values(&mut e, "xd"),
                1e-12,
                "wden C,L form",
            );
        }
    }
}

fn doppler(e: &mut Engine) {
    run(e, &noise(1024));
    run(
        e,
        "t = (1:1024)/1024; xn = sqrt(t.*(1-t)).*sin(2*pi*1.05./(t+0.05)) + 0.05*e1024;",
    );
}

#[test]
fn wdenoise_matches_the_reference_methods() {
    let mut e = Engine::default();
    doppler(&mut e);
    let cases: [(&str, &[f64]); 11] = [
        ("", &WDENOISE_BAYES),
        (", 'ThresholdRule', 'Mean'", &WDENOISE_BAYES_MEAN),
        (
            ", 'DenoisingMethod', 'Bayes', 'ThresholdRule', 'Soft'",
            &WDENOISE_BAYES_SOFT,
        ),
        (
            ", 'ThresholdRule', 'Hard', 'NoiseEstimate', 'LevelDependent'",
            &WDENOISE_BAYES_HARD_LD,
        ),
        (", 'DenoisingMethod', 'SURE'", &WDENOISE_SURE),
        (
            ", 'DenoisingMethod', 'UniversalThreshold', 'ThresholdRule', 'Hard'",
            &WDENOISE_UNIV_HARD,
        ),
        (
            ", 'DenoisingMethod', 'Minimax', 'NoiseEstimate', 'LevelDependent'",
            &WDENOISE_MINIMAX_LD,
        ),
        (", 'DenoisingMethod', 'FDR'", &WDENOISE_FDR),
        (", 'DenoisingMethod', {'FDR', 0.1}", &WDENOISE_FDR_Q10),
        (", 'DenoisingMethod', 'BlockJS'", &WDENOISE_BLOCKJS),
        (", 5, 'Wavelet', 'db4'", &WDENOISE_DB4_L5),
    ];
    for (opts, want) in cases {
        let got = values(&mut e, &format!("wdenoise(xn{opts})"));
        all_close(&probe(&got), want, 1e-8, &format!("wdenoise{opts}"));
    }
    run(
        &mut e,
        "[xd, dc, oc] = wdenoise(xn'); a = dc{end}; d1 = dc{1};",
    );
    assert_eq!(dims(&mut e, "xd"), [1024, 1]);
    all_close(
        &probe(&values(&mut e, "xd")),
        &WDENOISE_BAYES,
        1e-8,
        "column input",
    );
    all_close(
        &values(&mut e, "a"),
        &WDENOISE_BAYES_A,
        1e-8,
        "approximation cell",
    );
    all_close(
        &probe(&values(&mut e, "d1")),
        &WDENOISE_BAYES_D1,
        1e-8,
        "level-1 cell",
    );
    all_close(
        &values(&mut e, "[numel(dc) numel(oc) isequal(oc{end}, a)]"),
        &[8., 8., 1.],
        0.,
        "cells",
    );
    // A matrix is denoised column by column.
    let both = values(&mut e, "max(max(abs(wdenoise([xn' xn']) - [xd xd])))");
    assert!(both[0] < 1e-12, "{both:?}");
}

#[test]
fn errors_name_the_problem() {
    for (code, part) in [
        ("dwt(1:8, 'db99')", "Invalid wavelet name"),
        ("dwt(magic(4), 'db2')", "must be a vector"),
        ("dwt(1:8, 'db2', 'mode', 'xyz')", "invalid extension mode"),
        ("dwt(1:8, 'db2', 'extmode', 'per')", "unknown option"),
        ("[a, b, c] = dwt(1:8, 'db2')", "too many output"),
        ("dwtmode('nope')", "invalid extension mode"),
        ("idwt([1 2], [1 2 3], 'db1')", "same length"),
        ("waverec(1:10, [3 3 5 9], 'db1')", "bookkeeping"),
        ("wavedec(1:8, 0, 'db1')", "level N"),
        ("thselect(1:8, 'nope')", "unknown threshold rule"),
        ("wthresh(1:4, 'x', 1)", "SORH"),
        (
            "wden(1:64, 'modwtsqtwolog', 's', 'mln', 2, 'db2')",
            "not supported",
        ),
        ("wden(1:64, 'rigrsure', 's', 'all', 2, 'db2')", "SCAL"),
        ("wdenoise(1:64, 'Color', 'red')", "unknown option"),
        (
            "wdenoise(1:64, 'DenoisingMethod', 'SURE', 'ThresholdRule', 'Median')",
            "not valid",
        ),
        (
            "wdenoise(1:64, 'DenoisingMethod', 'Magic')",
            "unknown DenoisingMethod",
        ),
        ("wdenoise(1:64, 9)", "floor(log2(N))"),
        ("wdenoise(1:64, 'Wavelet')", "pairs"),
        ("detcoef(1:10, [3 3 4 9], 4)", "levels N"),
    ] {
        let message = error(code);
        assert!(message.contains(part), "{code}: {message}");
    }
}

#[test]
fn review_b7_oversized_bookkeeping_is_rejected() {
    for code in [
        "waverec(1, [2^64 2 1], 'haar')",
        "appcoef(1, [2^64 2 1], 'haar')",
        "detcoef(1, [2^64 2 1])",
        "wrcoef('a', 1, [2^64 2 1], 'haar')",
        "wden(1, [2^64 2 1], 'sqtwolog', 's', 'one', 1, 'haar')",
    ] {
        let message = error(code);
        assert!(message.contains("invalid bookkeeping"), "{code}: {message}");
    }
    // idwt's length is a scalar, parsed before the reconstruction filters run.
    let message = error("idwt(1, 1, 'haar', 2^64)");
    assert!(
        message.contains("integer") || message.contains("length"),
        "{message}"
    );
}

#[test]
fn review_b7_wdenoise_admits_coefficient_cells_before_allocation() {
    // db20, level 1, symmetric: 20 detail + 20 approximation coefficients
    // per column. zeros(2, 300) with a coefficient cell is 12 000 doubles
    // plus scratch, over 64 KiB (8192 doubles). One output keeps only the
    // denoised matrix, and a 2-by-2 with two outputs still fits.
    let mut e = Engine::with_limits(matrust::engine::Limits {
        evaluation_units: None,
        memory_bytes: 64 * 1024,
        ..matrust::engine::Limits::browser()
    });
    let started = std::time::Instant::now();
    let err = e
        .execute("[a, b] = wdenoise(zeros(2, 300), 1, 'Wavelet', 'db20');")
        .unwrap_err();
    assert!(started.elapsed().as_millis() < 1000, "{err}");
    assert!(err.contains("memory limit"), "{err}");
    assert!(!err.contains("Workspace"), "{err}");
    run(&mut e, "y = wdenoise(zeros(2, 300), 1, 'Wavelet', 'db20');");
    assert_eq!(dims(&mut e, "y"), [2, 300]);
    run(
        &mut e,
        "[a, b] = wdenoise(zeros(2, 2), 1, 'Wavelet', 'db20');",
    );
    assert_eq!(dims(&mut e, "a"), [2, 2]);
    assert_eq!(dims(&mut e, "b").len(), 2);
}

#[rustfmt::skip]
#[allow(clippy::approx_constant, clippy::excessive_precision)]
mod refs {
pub const WF_DB2_0: [f64; 4] = [-0.12940952255126037, 0.2241438680420134, 0.8365163037378079, 0.48296291314453416];
pub const WF_DB2_1: [f64; 4] = [-0.48296291314453416, 0.8365163037378079, -0.2241438680420134, -0.12940952255126037];
pub const WF_DB2_2: [f64; 4] = [0.48296291314453416, 0.8365163037378079, 0.2241438680420134, -0.12940952255126037];
pub const WF_DB2_3: [f64; 4] = [-0.12940952255126037, -0.2241438680420134, 0.8365163037378079, -0.48296291314453416];
pub const WF_SYM4_0: [f64; 8] = [-0.07576571478927333, -0.02963552764599851, 0.49761866763201545, 0.8037387518059161, 0.29785779560527736, -0.09921954357684722, -0.012603967262037833, 0.0322231006040427];
pub const WF_SYM4_1: [f64; 8] = [-0.0322231006040427, -0.012603967262037833, 0.09921954357684722, 0.29785779560527736, -0.8037387518059161, 0.49761866763201545, 0.02963552764599851, -0.07576571478927333];
pub const WF_SYM4_2: [f64; 8] = [0.0322231006040427, -0.012603967262037833, -0.09921954357684722, 0.29785779560527736, 0.8037387518059161, 0.49761866763201545, -0.02963552764599851, -0.07576571478927333];
pub const WF_SYM4_3: [f64; 8] = [-0.07576571478927333, 0.02963552764599851, 0.49761866763201545, -0.8037387518059161, 0.29785779560527736, 0.09921954357684722, -0.012603967262037833, -0.0322231006040427];
pub const WF_COIF1_0: [f64; 6] = [-0.015655728135791993, -0.07273261951252645, 0.3848648468648578, 0.8525720202116004, 0.3378976624574818, -0.07273261951252645];
pub const WF_COIF1_1: [f64; 6] = [0.07273261951252645, 0.3378976624574818, -0.8525720202116004, 0.3848648468648578, 0.07273261951252645, -0.015655728135791993];
pub const WF_COIF1_2: [f64; 6] = [-0.07273261951252645, 0.3378976624574818, 0.8525720202116004, 0.3848648468648578, -0.07273261951252645, -0.015655728135791993];
pub const WF_COIF1_3: [f64; 6] = [-0.015655728135791993, 0.07273261951252645, 0.3848648468648578, -0.8525720202116004, 0.3378976624574818, 0.07273261951252645];
pub const WF_BIOR2_2_0: [f64; 6] = [0.0, -0.1767766952966369, 0.3535533905932738, 1.0606601717798212, 0.3535533905932738, -0.1767766952966369];
pub const WF_BIOR2_2_1: [f64; 6] = [-0.0, 0.3535533905932738, -0.7071067811865476, 0.3535533905932738, -0.0, 0.0];
pub const WF_BIOR2_2_2: [f64; 6] = [0.0, 0.3535533905932738, 0.7071067811865476, 0.3535533905932738, 0.0, 0.0];
pub const WF_BIOR2_2_3: [f64; 6] = [0.0, 0.1767766952966369, 0.3535533905932738, -1.0606601717798212, 0.3535533905932738, 0.1767766952966369];
pub const WF_RBIO1_3_0: [f64; 6] = [0.0, 0.0, 0.7071067811865476, 0.7071067811865476, 0.0, 0.0];
pub const WF_RBIO1_3_1: [f64; 6] = [0.08838834764831845, 0.08838834764831845, -0.7071067811865476, 0.7071067811865476, -0.08838834764831845, -0.08838834764831845];
pub const WF_RBIO1_3_2: [f64; 6] = [-0.08838834764831845, 0.08838834764831845, 0.7071067811865476, 0.7071067811865476, 0.08838834764831845, -0.08838834764831845];
pub const WF_RBIO1_3_3: [f64; 6] = [0.0, -0.0, 0.7071067811865476, -0.7071067811865476, 0.0, -0.0];
pub const DWT_DB2_ZPD_A: [f64; 8] = [0.27383383721008214, -0.8088095159453775, 3.596215465788626, 0.18312269313907148, 1.2397596634991506, 0.4095893576037771, 0.37491418054326975, 1.4488887394336025];
pub const DWT_DB2_ZPD_D: [f64; 8] = [1.0219617932995717, 0.198137485246633, -2.153672724197458, -1.21375328070377, -2.901800680286947, 0.3732137755462197, 2.5892684645966155, -0.3882285676537811];
pub const DWT_DB2_SYM_A: [f64; 8] = [0.08838834764831838, -0.8088095159453775, 3.596215465788626, 0.18312269313907148, 1.2397596634991506, 0.4095893576037771, -0.013314387110511347, 3.814013335988205];
pub const DWT_DB2_SYM_D: [f64; 8] = [1.0716517624676405, 0.198137485246633, -2.153672724197458, -1.21375328070377, -2.901800680286947, 0.3732137755462197, 1.140379725163013, -1.5996590519348588];
pub const DWT_DB2_SYMW_A: [f64; 8] = [0.19411428382689044, -0.8088095159453775, 3.596215465788626, 0.18312269313907148, 1.2397596634991506, 0.4095893576037771, 0.43961894181889993, 0.8925522707483113];
pub const DWT_DB2_SYMW_D: [f64; 8] = [1.0433225832495676, 0.198137485246633, -2.153672724197458, -1.21375328070377, -2.901800680286947, 0.3732137755462197, 2.8307499211688825, -0.7914719274151236];
pub const DWT_DB2_ASYM_A: [f64; 8] = [0.45927932677184585, -0.8088095159453775, 3.596215465788626, 0.18312269313907148, 1.2397596634991506, 0.4095893576037771, 0.7631427481970511, -0.9162358571209999];
pub const DWT_DB2_ASYM_D: [f64; 8] = [0.972271824131503, 0.198137485246633, -2.153672724197458, -1.21375328070377, -2.901800680286947, 0.3732137755462197, 4.0381572040302185, 0.8232019166272966];
pub const DWT_DB2_ASYMW_A: [f64; 8] = [1.6730326074756159, -0.8088095159453775, 3.596215465788626, 0.18312269313907148, 1.2397596634991506, 0.4095893576037771, -0.4662477160399227, 7.59272910349026];
pub const DWT_DB2_ASYMW_D: [f64; 8] = [0.647047612756302, 0.198137485246633, -2.153672724197458, -1.21375328070377, -2.901800680286947, 0.3732137755462197, -0.5499904708428565, 0.7914719274151234];
pub const DWT_DB2_SP0_A: [f64; 8] = [0.9335734456512531, -0.8088095159453775, 3.596215465788626, 0.18312269313907148, 1.2397596634991506, 0.4095893576037771, -0.013314387110511347, 4.242640687119286];
pub const DWT_DB2_SP0_D: [f64; 8] = [0.8451850980029348, 0.198137485246633, -2.153672724197458, -1.21375328070377, -2.901800680286947, 0.3732137755462197, 1.140379725163013, 1.1102230246251565e-16];
pub const DWT_DB2_SP1_A: [f64; 8] = [4.087847173198287, -0.8088095159453775, 3.596215465788626, 0.18312269313907148, 1.2397596634991506, 0.4095893576037771, -0.4662477160399227, 7.380654839707473];
pub const DWT_DB2_SP1_D: [f64; 8] = [1.1102230246251565e-16, 0.198137485246633, -2.153672724197458, -1.21375328070377, -2.901800680286947, 0.3732137755462197, -0.5499904708428565, 6.661338147750939e-16];
pub const DWT_DB2_PPD_A: [f64; 8] = [2.541901291851239, -0.8088095159453775, 3.596215465788626, 0.18312269313907148, 1.2397596634991506, 0.4095893576037771, 0.31020941926763956, 1.328148011147469];
pub const DWT_DB2_PPD_D: [f64; 8] = [0.4142349504491617, 0.198137485246633, -2.153672724197458, -1.21375328070377, -2.901800680286947, 0.3732137755462197, 2.3477870080243486, -2.511871707636116];
pub const DWT_DB2_PER_A: [f64; 7] = [1.328148011147469, 1.9508896461054404, 1.2871268362445267, 1.246105661341585, -0.28250263147520904, 0.43327294397646543, 2.875794297491567];
pub const DWT_DB2_PER_D: [f64; 7] = [-2.511871707636116, 3.003503415045777, 0.5516908758399066, 2.718677987147876, 1.2911500376642695, -2.1172971421399005, 1.6603406117907469];
pub const DWT_SYM4_SYM_A: [f64; 10] = [-0.3858514615356344, -0.1840787708277435, 2.8270908955880194, 0.6002714523460585, 1.4578692543589875, -0.19716677928386794, 0.2527884810516813, 3.6428626576414826, 0.4398859439819146, 0.32662247595710425];
pub const DWT_SYM4_SYM_D: [f64; 10] = [-0.2525401080176892, -1.3428859752200826, 2.2832215494549994, -2.0894513424950176, -1.0246425786815616, -2.1558780951841303, -2.2437920522547543, 1.9117369898848882, -1.0307032066354178, -0.3681459032826747];
pub const DWT_SYM4_PER_A: [f64; 7] = [-0.2589893501784418, 3.220343441813419, 0.25244168467891376, 0.7727944201167949, 0.3307969504176392, 0.9875422310796761, 3.533905386907518];
pub const DWT_SYM4_PER_D: [f64; 7] = [1.0239463500087176, -1.486496559022875, 2.8323926698188573, 0.6549912470491721, 3.0325780521156127, 0.41055568076845017, -1.8717733630324487];
pub const DWT_SYM4_ASYMW_A: [f64; 10] = [-0.49026295963892214, 0.31269491220937556, 2.813089728030464, 0.6002714523460585, 1.4578692543589875, -0.19716677928386794, -0.012391520710775546, 4.877301658711635, 7.992350975086115, 8.523929497510789];
pub const DWT_SYM4_ASYMW_D: [f64; 10] = [2.7637769026482073, -2.625793997498197, 2.3161422941381784, -2.0894513424950176, -1.0246425786815616, -2.1558780951841303, -2.3565729043689037, 2.0432824194489356, -1.8277031708980311, 1.7280130877515916];
pub const DWT_COIF2_SYM_A: [f64; 12] = [0.04250802805287534, 3.4230104862935415, -0.11980377419625579, -0.2379657257530925, 2.5405482690968086, 0.9113612603695096, 1.3218571628048494, -0.08574593612065573, 0.19579242370382488, 3.4278970726184466, 0.7350648459565292, 0.27960599539115133];
pub const DWT_COIF2_SYM_D: [f64; 12] = [-0.7240311109201765, -0.9923574490208509, 2.3298716225514733, -2.4126617006482243, -0.7324809236928016, -2.4551946522097587, -2.0146606704050907, 2.070381582732544, -1.4885337918630812, 0.008021780113481106, 3.084069669358947, 0.8483937558053656];
pub const DWT_COIF2_PER_A: [f64; 7] = [3.9064954752716172, -0.525660516728122, 3.3792981712975183, 0.16636566173729248, 0.8852724502725035, 0.3505408930168981, 0.6765226299641365];
pub const DWT_COIF2_PER_D: [f64; 7] = [-1.1018430819788052, 2.632743845331882, 0.7637018901406644, 3.113800470500234, 0.013818484935916333, -1.4463644780808476, 0.6203369468635159];
pub const DWT_COIF2_ASYMW_A: [f64; 12] = [0.16465699548788518, 0.3547926353997314, -0.837555825119301, 0.5787981292361277, 2.472081463835584, 0.9113612603695096, 1.3193352397455282, -0.08056330483952023, 0.1436276141187684, 4.332512281985561, 8.203353616159841, 8.60055079752255];
pub const DWT_COIF2_ASYMW_D: [f64; 12] = [2.5123461290810303, -2.5711405303054384, 2.5469762357569596, -2.4596426212398055, -0.7294704455300874, -2.4551946522097587, -2.0720163480263034, 1.97738348937902, -1.9972945144960144, 2.0876162576152937, 2.485266586178469, 0.675170413796638];
pub const DWT_BIOR2_4_SYM_A: [f64; 11] = [4.107296029782798, -0.6684368790904083, -0.22373300498480603, 2.8367135323382207, 0.6739611508184283, 1.5937523935337419, -0.4170825154655027, 0.12705824974445773, 3.94433001380621, 0.32040776022515416, 0.38117474923337324];
pub const DWT_BIOR2_4_SYM_D: [f64; 11] = [-0.6187184335382292, -0.6187184335382291, 1.7677669529663689, -2.032931995911324, -0.9722718241315029, -2.29809703885628, -1.6793786053180506, 1.5026019100214136, -1.2374368670764584, 0.17677669529663692, 2.740038777097872];
pub const DWT_BIOR2_4_PER_A: [f64; 7] = [0.1325825214724775, 2.811854309562131, 0.6739611508184283, 1.5937523935337419, -0.4170825154655027, 0.16020388011257708, 3.8835630247979913];
pub const DWT_BIOR2_4_PER_D: [f64; 7] = [1.7677669529663689, -2.032931995911324, -0.9722718241315029, -2.29809703885628, -1.6793786053180506, 1.5026019100214136, -0.8838834764831844];
pub const DWT_BIOR2_4_ASYMW_A: [f64; 11] = [0.7402524115546669, -1.314776671268737, 0.7071067811865475, 2.7289902336418326, 0.6739611508184283, 1.5937523935337419, -0.4170825154655027, 0.10219902696836819, 4.242640687119286, 8.3830823472702, 8.902363889704075];
pub const DWT_BIOR2_4_ASYMW_D: [f64; 11] = [2.032931995911324, -1.7677669529663689, 1.7677669529663689, -2.032931995911324, -0.9722718241315029, -2.29809703885628, -1.6793786053180506, 1.5026019100214136, -1.502601910021414, 1.6793786053180502, 2.29809703885628];
pub const DWT_RBIO3_3_SYM_A: [f64; 10] = [1.1048543456039805, 0.08838834764831838, 1.1048543456039803, 1.9887378220871648, 0.795495128834866, 0.397747564417433, 0.4861359120657514, 1.9003494744388467, 1.9003494744388465, 0.4861359120657514];
pub const DWT_RBIO3_3_SYM_D: [f64; 10] = [-3.7951746771496726, 0.0, 3.795174677149673, -4.745349414369096, 0.5966213466261493, -3.8614659378859124, -1.3589708450928961, 4.1542523394709665, -4.1542523394709665, 1.358970845092896];
pub const DWT_RBIO3_3_PER_A: [f64; 7] = [0.4861359120657516, 2.5632620818012346, 0.7513009550107067, 1.016465997955662, 0.17677669529663703, 0.6629126073623884, 3.181980515339464];
pub const DWT_RBIO3_3_PER_D: [f64; 7] = [-1.9776892786311253, 1.6683300618620107, 1.1987669649803188, 1.5412718121175533, 3.1377863415153047, -1.3921164754610156, 0.41984465132951243];
pub const DWT_RBIO3_3_ASYMW_A: [f64; 10] = [-1.1490485194281395, 1.060660171779821, 1.1048543456039803, 1.9887378220871648, 0.795495128834866, 0.397747564417433, 0.4861359120657514, 2.5190679079770755, 7.822368766876181, 8.308504678941933];
pub const DWT_RBIO3_3_ASYMW_D: [f64; 10] = [1.6186116163098316, -2.789757222650051, 4.1597766111989865, -4.745349414369096, 0.5966213466261493, -3.8614659378859124, -1.590990257669732, 3.170931971883424, -2.4362038320567776, 3.1377863415153042];
pub const DWT_HAAR_SYM_A: [f64; 7] = [-0.5303300858899107, 3.8890872965260117, 0.17677669529663687, 1.5909902576697321, -0.35355339059327373, -0.1767766952966369, 4.242640687119286];
pub const DWT_HAAR_SYM_D: [f64; 7] = [1.2374368670764582, -1.0606601717798212, -1.2374368670764584, -1.5909902576697321, -2.474873734152917, 0.5303300858899107, 0.0];
pub const DWT_HAAR_PER_A: [f64; 7] = [-0.5303300858899107, 3.8890872965260117, 0.17677669529663687, 1.5909902576697321, -0.35355339059327373, -0.1767766952966369, 4.242640687119286];
pub const DWT_HAAR_PER_D: [f64; 7] = [1.2374368670764582, -1.0606601717798212, -1.2374368670764584, -1.5909902576697321, -2.474873734152917, 0.5303300858899107, 0.0];
pub const DWT_HAAR_ASYMW_A: [f64; 7] = [-0.5303300858899107, 3.8890872965260117, 0.17677669529663687, 1.5909902576697321, -0.35355339059327373, -0.1767766952966369, 6.717514421272202];
pub const DWT_HAAR_ASYMW_D: [f64; 7] = [1.2374368670764582, -1.0606601717798212, -1.2374368670764584, -1.5909902576697321, -2.474873734152917, 0.5303300858899107, -2.474873734152917];
pub const DWT_DB10_X5_SYM_A: [f64; 12] = [0.9345000648261218, 0.2809101189134852, 0.7584174257505786, -0.8868293511260625, 1.03432208519552, 0.9345000648261218, 0.2809101189134852, 0.7584174257505786, -0.8868293511260625, 1.0343220851955204, 0.9345000648261214, 0.28091011891348516];
pub const DWT_DB10_X5_SYM_D: [f64; 12] = [-3.194332429004365, 2.7161719160780886, -1.4791272034199439, -0.6693335745381086, 2.6266212908843287, -3.194332429004365, 2.716171916078089, -1.4791272034199436, -0.6693335745381085, 2.6266212908843296, -3.194332429004365, 2.7161719160780886];
pub const DWT_DB10_X5_SYMW_A: [f64; 12] = [-1.1760044383425883, 0.8770865127804828, 2.286931682565838, 0.13330658655591054, -1.1760044383425883, 0.8770865127804828, 2.286931682565838, 0.13330658655591032, -1.1760044383425883, 0.8770865127804828, 2.2869316825658372, 0.1333065865559105];
pub const DWT_DB10_X5_SYMW_D: [f64; 12] = [1.765829572724373, 2.3121003432807075, -1.6219161589698925, -3.163120538221734, 1.765829572724373, 2.312100343280707, -1.621916158969893, -3.1631205382217336, 1.7658295727243731, 2.3121003432807066, -1.6219161589698925, -3.163120538221734];
pub const DWT_DB10_X5_ASYM_A: [f64; 12] = [-0.19863463189765934, -1.5129044320800313, 1.1787172444570657, -1.5737507763981438, 2.1065725959187684, -0.19863463189765934, -1.5129044320800313, 1.1787172444570657, -1.5737507763981438, 2.1065725959187684, -0.19863463189765923, -1.5129044320800313];
pub const DWT_DB10_X5_ASYM_D: [f64; 12] = [2.741142583619176, 1.1434399522607221, -0.8362095923096784, -3.161401122208369, -0.5940786025483976, 2.7411425836191765, 1.1434399522607221, -0.8362095923096784, -3.161401122208368, -0.5940786025483978, 2.741142583619176, 1.1434399522607221];
pub const DWT_DB10_X5_SP1_A: [f64; 12] = [68.74081326416058, 60.25590072137694, 51.766013677331905, 43.30730912559724, 34.72753713609699, 26.47794305008967, 17.529079026436854, 9.930113775303976, -0.9354422732073524, 2.9121396074429224, -2.3284373282214657, -13.468790450852612];
pub const DWT_DB10_X5_SP1_D: [f64; 12] = [-1.0755285551056204e-16, 0.7416017635211919, -0.37095088158224726, -1.008547424598438, 0.6398995578497804, -0.17862142887062427, -0.037200628393839634, 0.053416921525960975, -0.018620178180254236, 0.0021593861130248804, 8.621731882517249e-05, -1.6315701095942839e-15];
pub const DWT_DB10_X5_PER_A: [f64; 3] = [-1.4145575399948906, 0.5064336807476283, 1.261677249840536];
pub const DWT_DB10_X5_PER_D: [f64; 3] = [-0.834397252884363, 2.6475881307161475, -2.1667442684250577];
pub const WAVEDEC_DB3_SYM_C: [f64; 53] = [2.5278510014997986, 2.649546740639171, 2.7745513350788014, 1.5214880253792864, 1.3831706159508452, 7.543701319509281, 5.265273162560881, 11.268817784240762, 13.867881437718683, 0.12214684968537406, 0.39086624913698814, -1.0967592314752448, 1.1499787649001005, -0.5663586255992472, -0.1375148492056638, 0.2640496415484753, 0.45377806890129213, -1.1904633752450915, -0.13463832200052048, 0.1458051198079785, -0.17583023950271606, -0.09747764568183856, 0.15100343537020985, 0.1685210090756446, -0.07171848891915775, -0.20226277566424444, -0.023441120114409603, 0.19123431124181023, 0.12143082466458262, -0.13023470806816614, 0.0890400865414365, -0.004293043963981287, -0.0037485078916026024, -0.019491415056760747, -0.01808197052787803, -0.00892936551844637, 0.004047021312342009, 0.015290370505370725, 0.019985993473119433, 0.016123104823621268, 0.005355891896989776, -0.007704850399734592, -0.0174661794476777, -0.019748045445013615, -0.013573295233282448, -0.0015861141761891462, 0.01108028138287348, 0.019001818145436444, 0.018786292241324165, 0.010525997353942773, -0.002241797786452726, -0.03614050212883435, 0.039006553986926984];
pub const WAVEDEC_DB3_SYM_L: [f64; 5] = [9.0, 9.0, 13.0, 22.0, 40.0];
pub const WAVEDEC_SYM4_PER_C: [f64; 40] = [-0.3492902046731739, 6.6939090821714196, 5.62095985200113, 9.994309744859498, 7.711214769856872, 2.1172226709537294, 0.21588701660455284, -1.0778031985202856, 1.6841785064235968, -3.0276259623884347, 1.7886552838168934, -0.1726989743233347, 0.10166436684783067, -0.015280729660172753, -0.11793565463917807, -0.040205064750462205, 0.09902017090122761, 0.07822622475383126, 0.14737159456973453, -1.0261154091716236, 1.6440701341871355, -0.28472808210562583, 0.0010845507054213083, 0.004582647129902813, 0.006118337294313983, 0.005033999543158121, 0.0017939749286245911, -0.0022142754586485305, -0.005274316678936852, -0.006075761116712375, -0.004275409844259365, -0.0006442191466912994, 0.0032628426020996915, 0.005772672015321445, 0.005810496191583303, 0.0033601178548106903, -0.0005291485594533241, -0.004191820080253927, 0.10699128316755913, -0.20599609883188816];
pub const WAVEDEC_SYM4_PER_L: [f64; 5] = [5.0, 5.0, 10.0, 20.0, 40.0];
pub const WAVEDEC_BIOR3_5_ZPD_C: [f64; 71] = [0.0002823017108202698, -0.006857107983390297, 0.1321666050322188, -0.7500373757107228, 2.751219022767056, -2.1004662657876163, 9.752872358916076, 3.5312409954108945, 12.872637157891074, 4.540284384923112, -1.0626585692843027, -0.048190774628772415, 0.057645047839069166, 0.0009654631193391351, 0.0, 0.0, 0.0036134618984994538, -0.06319944127759948, 1.1694717191231383, 0.2090643598312359, 0.3609631769842584, -0.6736202846508719, 1.0406925922527974, -4.655650681088737, -0.8218905222473636, -0.012357927927540929, 0.0, 0.0, 0.0, 0.0, 0.0514821156195163, 0.2428821436836701, -0.3253079419395307, -0.04897976504682966, 0.046597081660292146, 0.06564119829208034, -0.01571452927043382, -0.07303449375443194, -0.01864638488597592, 0.06426183309152966, -0.17427542372627802, -1.0683049640359954, 1.1185782639921626, 0.022833540914360486, 0.0, 0.0, 0.0, 0.0, 0.37989021519515104, -0.0047792448810400345, -0.0064407212789833745, -0.005344116725695944, -0.0019590252306755265, 0.002264970781038867, 0.005519048596376128, 0.006409729185963137, 0.004555600428181483, 0.0007506474974320332, -0.0033757518170496326, -0.006056568194560019, -0.0061438077592133045, -0.0036001123066966567, 0.00048524296081475793, 0.004362804829269207, 0.006372102512342326, 0.00565270354835945, 0.0025126729028674966, -0.0017033482986341664, -1.6533249657901852, 0.0, 0.0];
pub const WAVEDEC_BIOR3_5_ZPD_L: [f64; 5] = [14.0, 14.0, 18.0, 25.0, 40.0];
pub const WRCOEF_D2: [f64; 40] = [0.04023990721913322, 0.11160942608511497, 0.07462070080528548, -0.0005518296115745995, -0.04178788811522005, -0.09795150328267545, -0.020135900543252424, 0.11128575135435803, 0.033018401083266445, -0.09516355048669432, -0.06027083288175014, -0.0015847876369329966, 0.02967843609792306, 0.07769107763035651, 0.001808711880129568, -0.12184542411382851, -0.023723501704943563, 0.13325786323715375, 0.061719653146777305, -0.05598694838836416, -0.04083975404056356, -0.014996564887456626, 0.027228850982570833, 0.09550495636279119, 0.00450941243535718, -0.14031337430116578, -0.048909155488839874, 0.10091965673474584, 0.04305528982109254, -0.05078946817004808, -0.049881569399396954, -0.04751007768773381, 0.01572316296821212, 0.11557207309350122, 0.023830610986432087, -0.12582882014780747, -0.03195774219445025, 0.11810637011255372, 0.07476212199329425, 0.0012432592063754307];
pub const APPCOEF_A2: [f64; 13] = [1.9326488980086545, 1.6559969634061127, 2.0596980793420236, 1.419015325404479, -0.13739918620549196, 0.7960078665319943, 4.015187080836905, 5.819943723615658, 4.673477538125412, 3.5529569313645553, 5.395016805862133, 8.610668141947546, 9.455698677756914];
pub const WRCOEF_A2: [f64; 40] = [1.0083615802213335, 0.8727911822688478, 0.7667116491966015, 0.6271236680486679, 0.45966508056594574, 0.2677678918359345, 0.04261982498322821, -0.21107985566781595, -0.11579809904242251, 0.11107371412913343, 0.29307881888913934, 0.5546115602434337, 0.8900709489726378, 1.2895027237826073, 1.767715073761187, 2.3172547044191862, 2.527604735858158, 2.635397472030586, 2.8322766034092304, 2.9830854442240864, 2.8992965104620314, 2.703531024345246, 2.4708978979050937, 2.145262564078344, 2.048444407083113, 2.0015013604689016, 1.8481266101623222, 1.6970669232646411, 1.8881638364557491, 2.2146623181902654, 2.527959020439132, 2.9353201465706515, 3.2755356939193465, 3.629862897270786, 4.08435283194044, 4.580964636537199, 4.69740430596724, 4.682426823182414, 4.728240124744087, 4.69807970050178];
pub const WMAXLEV: [f64; 5] = [7.0, 5.0, 3.0, 0.0, 6.0];
pub const THSELECT_NOISE: [f64; 4] = [1.2137933586199239, 2.884053773201766, 2.884053773201766, 1.4909999999999999];
pub const THSELECT_SIGNAL: [f64; 4] = [0.5469033592899226, 0.5469033592899226, 2.884053773201766, 1.4909999999999999];
pub const THSELECT_SMALL: [f64; 4] = [0.5, 0.5, 2.2649279712439143, 0.0];
pub const WDEN_RIG_S_MLN: [f64; 34] = [-0.11907890204731096, -0.030996911874345306, -0.18054720566022367, 0.8699649195742747, 3.56109961059121, 1.7683467460108024, 2.1779998837556334, 1.7497509300948029, 2.7615973234315914, 2.8929368656633896, 3.0331039998352605, 3.0609875334595134, 3.0520922314645857, -1.3293388420737944, -0.5105856340258725, 0.9132093020374644, 0.9332385092184319, 0.7750972459249799, 0.9456725485674489, 1.23522856283171, -0.08168030311644028, 5.0626602533356655, 5.183578164667863, 5.201483460887206, 4.843981718062553, 3.7875238411079346, 0.39081395496232385, -0.05942746886753681, 0.1695248696442055, 0.06557328444265126, 0.22614349673037398, -0.1523102041044739, 775.8904583324859, 2932.377516334265];
pub const WDEN_RIG_S_MLN_THR: [f64; 4] = [0.6790770761101318, 0.7474068834155747, 0.8480509047824314, 0.6388534375080499];
pub const WDEN_RIG_S_MLN_C: [f64; 36] = [-1.6080157704678928, 2.3182383630315693, 0.15266626766687183, 8.203930473631546, -1.5089964710753043, -0.0, -0.09163704694590535, -0.0, -0.0, 0.0, -0.0, 0.0, 0.0, 0.0, -0.0, 0.0, -0.0, -0.0, 0.0, -0.0, 0.0, -0.0, -0.0, 0.11231120005493256, -0.0, -0.0, 0.0, 0.4786847183439932, 0.0, -0.0, -0.29778291608250884, 0.0, 0.0, -0.0, 199.93776263360292, 2939.038057590441];
pub const WDEN_HEU_H_SLN: [f64; 34] = [-0.3111119134678701, -0.03241175375100356, -0.021162109834735227, -0.24609549742480533, 3.041022834743217, 2.4170356969349838, 2.0089940783506597, 2.314591806631036, 2.149828460977489, 3.09269314357451, 2.800211898979334, 2.9708869639480766, 2.893146941842967, -1.4097966021848976, -0.13142054188373312, 0.7885936149193195, 1.0468816590618044, 0.7314800163221172, 0.8431802657932042, 0.9582210054991817, 0.5113629835493354, 5.539448529756585, 5.156470183883736, 5.243198137389085, 5.3135996120695745, 3.6947875837557542, -0.8931307934125629, -0.04925206501429985, 0.3015331926717533, 0.12108107361969656, 0.2680430166701712, -0.5271659472924423, 777.0172081856279, 3044.2606847319885];
pub const WDEN_HEU_H_SLN_THR: [f64; 3] = [1.8384439441364768, 1.7246733210745389, 0.35676473579362955];
pub const WDEN_SQT_S_ONE: [f64; 34] = [-0.09711753203835047, 0.03500243863329842, 0.05151219355125839, 0.517935601939319, 2.514166040035487, 2.1328841916529164, 1.866801871093733, 1.671950540574004, 0.36960089797001644, 2.8553674441160233, 2.903067520613971, 2.9387677757424875, 2.9250497539581697, -1.1072378375425735, -0.22861438420787658, 0.8078467997921028, 1.0146010136104577, 0.6060290111476371, 0.8402195144113258, 1.110345319708397, 0.5867157253364308, 4.913780748760959, 5.149228178127166, 5.228055993050898, 5.04412494573454, 3.420561607148711, 0.7649240325915109, 0.013284702258329471, 0.05520028990034631, 0.1737911001544952, 0.22405862067892077, -0.4601779925665913, 777.3948295438017, 2915.87357172686];
pub const WDEN_SQT_S_ONE_THR: [f64; 3] = [3.5488334195715456, 3.5488334195715456, 3.5488334195715456];
pub const WDEN_MIN_S_MLN: [f64; 34] = [-0.049319253357364645, -0.108058179888318, -0.4173199649106956, 0.7887953444339294, 2.356834503629732, 1.670919683167417, 2.1147115697143573, 1.360120340366152, 1.9820103853738922, 2.9145795116764406, 3.082659586701137, 2.888459728660017, 3.089692427115301, -1.120591390221219, 0.15190741769979293, 0.9056990192071602, 0.8340764017146688, 0.8121544933197519, 0.8684462150337972, 1.318449764237605, 0.12121932631935994, 4.608476081649617, 5.245313258020793, 5.330469220791601, 4.421000130973118, 3.831626971951322, 0.19754128922444475, -0.10233374693825681, 0.18483198241732574, 0.06362350291808436, 0.06339847243407065, -0.08993587241468783, 775.4036036573809, 2822.5979863462803];
pub const WDEN_MIN_S_MLN_THR: [f64; 4] = [1.0810444094571061, 1.155286147289864, 1.3414552300646383, 1.7427141350106385];
pub const WDENOISE_BAYES: [f64; 66] = [-0.019453050655966756, -0.13052666723190814, -0.19814260520653917, -0.10854043396663311, 0.22085587401575588, 0.19534715183566434, 0.26215049885417857, -0.08041863552805618, -0.08648002223433855, 0.0406936802341339, 0.1333550296381828, -0.3401388558528315, 0.21025032214892728, 0.29315044438746307, -0.25233924445987677, -0.36790735744725434, 0.034350695595606544, 0.38762003417217944, 0.36758561255727407, 0.0701564609099073, -0.3696679279958225, -0.47607412528662524, -0.3895815107864352, -0.1751404272480415, 0.10619005285732624, 0.33714494443927545, 0.45434251865266606, 0.4787940780094457, 0.40890051449389975, 0.2603031718392726, 0.08383904972662524, -0.12520604527512555, -0.28447114752871216, -0.3987575123986777, -0.4821693221654181, -0.4988447573941871, -0.48420013583927757, -0.4370576235033861, -0.3641309459182432, -0.2686827853777887, -0.1609786608202965, -0.052381545057348544, 0.056975158829057515, 0.14232097659710125, 0.1980889188397835, 0.2668615135686461, 0.3307421898936762, 0.3788002887668495, 0.4129154561613015, 0.3964755188193267, 0.41770366986380675, 0.42163707303103726, 0.41259717233097204, 0.3571773346832596, 0.3210429918174283, 0.287190940765603, 0.25896848412494955, 0.21775785832012837, 0.17039273992833712, 0.12977011083492782, 0.09594565990747231, 0.05881481494516742, 0.04221152087508824, 0.007300664554427449, 48.2621227246987, 87.57432840114964];
pub const WDENOISE_BAYES_A: [f64; 14] = [-0.41916625254670653, -0.43089872692618025, -0.16298702945877308, -0.07195267400230465, -0.5839803521007344, 2.1793197110408524, -3.4067023706284862, -1.0250590943718365, 4.632174383349634, 2.3941940432705904, 0.15541482306181645, 0.1347717055393754, 0.058347551598923644, -0.04729622150230036];
pub const WDENOISE_BAYES_D1: [f64; 35] = [0.0, -0.0, -0.0, 0.0, 0.0, 0.0, -0.0, -0.0, -0.0, -0.0, 0.0, 0.0, -0.0, -0.0, 0.0, -0.0, 0.0, -0.0, -0.0, -0.0, -0.0, -0.0, -0.0, 0.0, -0.0, -0.0, 0.0, 0.0, 0.0, 0.0, -0.0, -0.0, 0.0, 0.1328475086612268, 0.03793722197438844];
pub const WDENOISE_BAYES_MEAN: [f64; 66] = [-0.020414927825255873, -0.12811647010287514, -0.1875119362301026, -0.12167910200503385, 0.21529590435574747, 0.1874783125416223, 0.26676812750594525, -0.08044953607161996, -0.08223517706461295, 0.049412182119940994, 0.1371141589182669, -0.3510081797807295, 0.2112895918711764, 0.2915468991993042, -0.2475844052285337, -0.37383897906762775, 0.03806132322304024, 0.3840938855750107, 0.3694679223014152, 0.08101096862330401, -0.37996412654466083, -0.4747904045665757, -0.3939233773258755, -0.17576623992169574, 0.1099417192987394, 0.3401843570689401, 0.45219000639389023, 0.4798449134849022, 0.40881884136095503, 0.2596856569617058, 0.08400829038569577, -0.1259324799486861, -0.28886529726730853, -0.3973520186235312, -0.4664732733582568, -0.486693394601633, -0.4759135280541561, -0.4364748528805401, -0.36780152441922853, -0.269353089715372, -0.15767919776273204, -0.07286042455733695, 0.05653913225834479, 0.1689483568243439, 0.2104514107780778, 0.2559923049219016, 0.33800158452977264, 0.3840640167292989, 0.42378189892618107, 0.4011649662411508, 0.4240783972568523, 0.41504670398208593, 0.41443318757113906, 0.34935239935612605, 0.3267359592393065, 0.29256238578034255, 0.26057407146150535, 0.21759639238426884, 0.17512653789709884, 0.1299953052117204, 0.0970948565177555, 0.05791588588182162, 0.046382974464162355, 0.0076744578880297626, 48.31162655246874, 87.6105871228642];
pub const WDENOISE_BAYES_SOFT: [f64; 66] = [-0.015157875607235477, -0.06935146148422462, -0.13718378308391044, -0.08847022783010366, 0.14901668581357735, 0.16307910840116058, 0.2316111591486163, -0.06689406621019126, -0.08532070129401284, 0.047096094505106156, 0.1084570247637558, -0.33163115662429077, 0.22756099811068817, 0.26391372608390645, -0.2621172902977701, -0.35870594125072425, 0.03987119850911967, 0.3797681790300795, 0.3693183886780641, 0.08485053398177803, -0.33180386624231406, -0.47154166320236496, -0.3980679573049216, -0.18321789576834682, 0.10985059262046498, 0.3295661873558742, 0.46091465290159717, 0.48016599806943966, 0.4143882288831188, 0.2625109201378987, 0.06624735785899259, -0.12340210670008556, -0.2796315600370843, -0.3957067043977234, -0.47883730239397665, -0.5014398020700072, -0.4847582449389376, -0.4381439177664663, -0.3672070145175395, -0.2718977560368155, -0.16406377110505205, -0.052588212319766195, 0.06194550239497052, 0.14371893966249852, 0.19896195637799463, 0.26737555109704136, 0.3342164855565643, 0.3805616734732852, 0.41222622067982473, 0.41267993931290514, 0.413590515676318, 0.40874118264813547, 0.4068797295055953, 0.362806121759852, 0.31959128705034473, 0.2869410709065403, 0.26223631889476495, 0.22014222550140303, 0.16917852081186444, 0.12832007156112668, 0.09651580262537351, 0.05792950882417771, 0.025324098454397006, 0.004095861687475141, 48.24750730926788, 86.49844687611491];
pub const WDENOISE_BAYES_HARD_LD: [f64; 66] = [-0.0115995264272386, -0.15099883514390144, -0.20960505117196182, -0.11579325633598236, 0.26154248517374784, 0.18631913042539602, 0.25614815944482927, -0.09551138787658632, -0.08815830540469975, 0.03967122986719983, 0.1405446210497153, -0.3386136296689069, 0.2238097387328292, 0.3024595212672758, -0.23794581017234576, -0.37212295785449295, 0.024689315581231584, 0.39113068675079626, 0.35364887376244314, 0.08116229033811054, -0.40221032163219017, -0.48787450556535816, -0.3951519036818619, -0.17862230730728268, 0.09855387067417873, 0.34342773248331343, 0.4592184288192613, 0.4744018149221271, 0.4159576823952247, 0.26454650086930137, 0.07014024051912802, -0.12099770680047262, -0.2796029462349198, -0.39552071406140354, -0.4880743595159795, -0.5043844514607496, -0.48920451994955005, -0.44003089409022506, -0.36576338698330524, -0.265744189213275, -0.148203847907595, -0.04940032425830025, 0.03709717711502875, 0.12618240063969458, 0.21237068729213024, 0.27997500831804534, 0.3311990204814874, 0.37702763272159956, 0.4124411194424046, 0.38854694324937594, 0.4200971104724429, 0.4039781866913394, 0.4054301184610231, 0.3494936417062023, 0.32612837621894314, 0.2925373702758479, 0.2584340819320122, 0.2167493343704283, 0.1738243960462502, 0.1276970658732337, 0.08103801199677164, 0.05371410374529305, 0.050997544467355126, 0.01974002691279293, 48.11836486081396, 88.91965760037397];
pub const WDENOISE_SURE: [f64; 66] = [-0.021343807762767814, -0.1004582251616359, -0.170531757659506, -0.12948621496908322, 0.20624332861823227, 0.16067283751764688, 0.2588586834044952, -0.0626925319993806, -0.0754265717860052, 0.05777063067102261, 0.132392769112532, -0.34790760267618814, 0.20679611145116392, 0.28589041649562746, -0.259702388591549, -0.37105543180954853, 0.037163686440162796, 0.3891763886532461, 0.37757756673876836, 0.09324861474340326, -0.40315370942688844, -0.4778624764882681, -0.39118407261480687, -0.1777529254604883, 0.11592985958360884, 0.34455335167268447, 0.45366966303855505, 0.4794420779895206, 0.41156977201537553, 0.25803945198259476, 0.08824187037968684, -0.11438946002967024, -0.2932123296593173, -0.39247463986194614, -0.44742692692798103, -0.4662492777251243, -0.46462084318562585, -0.44072185893243315, -0.3628325932995244, -0.2777499754755956, -0.16192791221089534, -0.09245555797415286, 0.05552338369753612, 0.18592434170262587, 0.22049712051806178, 0.2478477521902893, 0.34188242140488995, 0.3850278961864297, 0.43099475976370427, 0.41128383489364195, 0.42394601950533617, 0.4130482659269151, 0.4132288068620507, 0.32934288849972304, 0.3307898800718606, 0.29289933431481896, 0.2597282650759729, 0.21392106124795726, 0.18052941579975884, 0.12724818711913957, 0.10066140048165478, 0.05001884880340282, 0.046366149406965276, 0.011228589753100182, 48.33524885411273, 87.50138924909201];
pub const WDENOISE_UNIV_HARD: [f64; 66] = [-0.017977853967655767, -0.15099883514390144, -0.1990892203982815, -0.11579325633598236, 0.16220763075912853, 0.18631913042539602, 0.2548208270252678, -0.0756386799041653, -0.08539879391115618, 0.03967122986719983, 0.14047407128591322, -0.34135223035863405, 0.2483900416006588, 0.2820566947473884, -0.23992842512209797, -0.37273156823294473, 0.024689315581231584, 0.39113068675079626, 0.35364887376244314, 0.08116051343813402, -0.311119754701069, -0.48754594735011214, -0.39700518624240383, -0.18080025443496583, 0.11679082455045331, 0.32119842861179315, 0.46284087861366074, 0.48028535197858624, 0.4178094658143551, 0.26460675000239686, 0.06052371618103395, -0.125348241372199, -0.29959306730079804, -0.40156101863021987, -0.4479023968194642, -0.48239157742279676, -0.497077364984118, -0.4515041148992741, -0.3696455858820414, -0.2667996853299802, -0.14793318706976732, -0.048468957018135105, 0.038183078863101114, 0.12692125213745808, 0.21240252992023684, 0.2798469993756416, 0.3311847872731809, 0.37703139845711037, 0.41450814110361134, 0.42672301027754245, 0.4217234663380926, 0.40655719365394904, 0.38224592722815065, 0.35582796761783514, 0.3257374400104669, 0.2931105134907137, 0.2584340819320122, 0.2167493343704283, 0.1738243960462502, 0.1276970658732337, 0.08103801199677164, 0.05371410374529305, 0.050997544467355126, 0.02649150864112467, 48.13118476225691, 88.54764764616222];
pub const WDENOISE_MINIMAX_LD: [f64; 66] = [-0.016532611013516284, -0.07997378652390637, -0.13693786677879924, -0.09215139752211657, 0.15183596444920708, 0.14236673156923962, 0.20738085873193793, -0.08530268541906302, -0.11201145020067192, 0.051100890281712906, 0.13722344689564725, -0.28375258263235914, 0.22630093278282742, 0.22825846098867292, -0.23982069292855787, -0.3325204973331569, 0.021597674421700987, 0.32334445866917483, 0.31396095006592345, 0.07603980023917208, -0.3337659927219516, -0.4072994831519958, -0.3213590006018906, -0.11729656845029235, 0.16299668352301683, 0.32207753140247397, 0.41323966910162996, 0.41545640617000895, 0.3518783363878104, 0.22218813176711946, 0.06647418761673358, -0.1283490903014833, -0.3330943724439964, -0.42499567053949616, -0.4467540848985432, -0.4541456867889833, -0.44493734078433017, -0.3969449498464667, -0.32600175942520093, -0.23452453387339203, -0.12914045326694173, -0.04642759503773157, 0.02561089901454189, 0.09405517059135049, 0.15559836657276743, 0.228341846332005, 0.29838741471510255, 0.3686845061874127, 0.43485980337126584, 0.4482424572987291, 0.44349058289687504, 0.4235198960197064, 0.39439353648537984, 0.35572179180794994, 0.324999685226942, 0.27650111445023207, 0.22700612273243434, 0.19240889541039366, 0.1652271996199178, 0.1414025997890456, 0.12086627813987133, 0.09275953583237387, 0.0686886317335126, 0.03353449066377298, 49.024666166242675, 76.45291548125623];
pub const WDENOISE_FDR: [f64; 66] = [-0.02917002542678059, -0.15957878102986228, -0.21230089475660008, -0.1089676207340821, 0.2507775275488757, 0.20633772350030669, 0.26708138001117043, -0.08556004746505684, -0.08103700880922096, 0.041992671568817364, 0.13791271094793425, -0.34834345877137296, 0.20401169181766238, 0.3065904407092215, -0.2541571785844051, -0.37495080575982515, 0.029854137075565813, 0.39903481718627715, 0.3602429510941174, 0.08598504271720588, -0.3086997992432504, -0.4866219585013829, -0.3942288159957153, -0.17825579946513995, 0.09835190395685839, 0.3429410440702018, 0.45863135088329987, 0.4737711900307663, 0.4152901307559456, 0.26171147919432836, 0.07870781573673398, -0.1269170755719226, -0.2791583276343385, -0.3959079781476112, -0.48836854727270146, -0.5037401617719013, -0.48744554361435205, -0.4389911072554012, -0.3636163233398314, -0.2696584162083436, -0.16604244642103427, -0.05482541141394708, 0.060570654390403135, 0.14363827951835692, 0.20534760779334066, 0.269738065765748, 0.32772435924921683, 0.3761021210545904, 0.4127414415411653, 0.389413081084844, 0.4210976853262476, 0.40457076744410314, 0.40514039399491913, 0.3493420953287094, 0.3268313758237302, 0.2928938397269798, 0.2582842502571917, 0.21712516450296887, 0.17277491449304172, 0.13148513932576023, 0.09589396305076237, 0.05727195938406966, 0.030619523920564027, 0.006406797923756576, 47.91570795592926, 88.92511609608295];
pub const WDENOISE_FDR_Q10: [f64; 66] = [-0.0191167979606221, -0.15453970601761258, -0.20819924489458458, -0.10608500085985781, 0.25199902399177354, 0.20687708042372124, 0.26747226986850786, -0.08543488716410845, -0.0811659919366973, 0.041712959191493565, 0.13755923496963202, -0.34873972608338194, 0.20360697767387742, 0.30623994957206635, -0.2544908709613091, -0.3747876170075577, 0.028144813795515897, 0.39708795533321833, 0.3770604047984262, 0.06541541694537664, -0.39614013547701155, -0.4818106821323247, -0.39433772843155007, -0.17928499297422218, 0.09822597712641835, 0.34297436102355094, 0.45863135088329987, 0.4737711900307663, 0.4152901307559456, 0.26171147919432836, 0.0905950904895684, -0.1269170755719226, -0.2791583276343385, -0.3959079781476112, -0.48836854727270146, -0.5037401617719013, -0.48744554361435205, -0.4389911072554012, -0.3636163233398314, -0.2696584162083436, -0.16604244642103427, -0.05482541141394708, 0.060570654390403135, 0.14363827951835692, 0.20534760779334066, 0.269738065765748, 0.32772435924921683, 0.3761021210545904, 0.4127414415411653, 0.389413081084844, 0.4228579372999696, 0.4370810689065683, 0.406468801817295, 0.3493420953287094, 0.3268313758237302, 0.2928938397269798, 0.2582842502571917, 0.21712516450296887, 0.17277491449304172, 0.13148513932576023, 0.09589396305076237, 0.05727195938406966, 0.030619523920564027, -0.0003446838045751625, 48.13520166331807, 89.07697226928272];
pub const WDENOISE_BLOCKJS: [f64; 66] = [-0.015534909761889959, -0.13118020103505368, -0.15238084664874155, -0.14434424834105053, 0.13695878308632822, 0.2008436208072807, 0.25774385873287486, -0.092478209938183, -0.09746901460559017, 0.04389510083943187, 0.14229098165177528, -0.34354876001771695, 0.20380898467695646, 0.2791801631992115, -0.2480554973641434, -0.37586855236419997, 0.029805754299971527, 0.3952787744231005, 0.36527184038216753, 0.08448577997452911, -0.3303959892984926, -0.4533695441047699, -0.39907022804568965, -0.1844701507700801, 0.11353205885247078, 0.3268325586953412, 0.4606195058827222, 0.4760559622999249, 0.4145720407569223, 0.25988737385950866, 0.05697690219071578, -0.1214006768386769, -0.27980769228714625, -0.39523827348647445, -0.4734662992923868, -0.5009649589489756, -0.48813300507234586, -0.43900474852521326, -0.36486857547318624, -0.2651102070812281, -0.1479002672816642, -0.04933164323193984, 0.036904898593358215, 0.1259432829274895, 0.21218826327032855, 0.27931123043859873, 0.330062532445039, 0.3749469837546954, 0.41128992225420913, 0.42445076593969033, 0.4212066202910582, 0.4083994505561699, 0.3867646469109732, 0.3604200059563127, 0.3291434331327577, 0.2947093155171974, 0.25781225055258683, 0.2155572554997067, 0.17253402855409272, 0.12734673146258044, 0.08216691477764458, 0.05317554623613143, 0.03519207041423304, 0.019540652427866994, 48.45854710869819, 87.53317690072629];
pub const WDENOISE_DB4_L5: [f64; 66] = [-0.021289580233048895, 0.0006232176413157094, -0.10964841904003018, -0.19325198319335077, 0.14449781643473936, 0.20363541579843453, 0.29066601582519846, -0.0975221388277794, -0.06113436282929557, 0.0509747218299679, 0.13065545471763765, -0.3602380246256038, 0.20602255117650697, 0.3201104103992481, -0.1945875848234157, -0.39832403154225815, 0.033367758386656565, 0.397153619832211, 0.3931534586267253, 0.07237954718355327, -0.3069293056872461, -0.48738187907113917, -0.4056274683370646, -0.169855718397893, 0.08849520258137265, 0.34277542931709576, 0.4643914004950766, 0.47466114007669546, 0.4119711979919415, 0.2622446583960087, 0.07666219471244683, -0.1668350892395457, -0.28567125189927345, -0.38973121359118373, -0.4724778023537127, -0.5039093261541232, -0.4891591693772204, -0.4369398220977641, -0.3649099401875969, -0.2710202129984263, -0.16409718719049113, -0.05252408590414876, 0.055412482475346844, 0.14705344556532154, 0.2074729141300774, 0.25926192256413916, 0.3294741914457492, 0.38627750953399276, 0.4158071088271135, 0.410572228841458, 0.4187127707804338, 0.40401686818169447, 0.3900248862709444, 0.3664295673761081, 0.3252440459206421, 0.28433099919501287, 0.25794652541299057, 0.22614791905082454, 0.17319807242062296, 0.12286700682124982, 0.09116989219932967, 0.06012414310608777, 0.025236464140317435, 0.003649853846847814, 48.382801955055946, 88.03147365690822];
}
