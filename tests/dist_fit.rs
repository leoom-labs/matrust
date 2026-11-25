//! fitdist families, covariances, paramci, mle, <family>fit/<family>like, ecdf and
//! probability plots. Oracle: bash tool/ref-python.sh tool/refs/stats/dist_fit.py
//! (SciPy/NumPy; the printed constants are pasted below). Never MATLAB.
use matrust::{Array, Engine};

#[rustfmt::skip]
const POS: &[f64] = &[6.46, 0.845, 6.022, 3.377, 2.533, 5.436, 6.962, 3.585, 2.101, 2.055, 1.843, 7.084, 6.598, 5.043, 5.629, 1.585, 6.801, 1.488, 3.94, 4.017, 1.68, 4.412, 5.146, 7.749, 13.827, 1.197, 8.056, 4.162, 4.452, 2.532];
#[rustfmt::skip]
const REAL: &[f64] = &[5.565, 12.639, 3.175, 8.765, 4.76, 3.678, 6.214, 6.048, 9.058, 9.27, 10.252, 6.966, 5.66, 8.729, 5.486, 3.695, 3.483, 5.195, 4.408, 8.37, 6.201, 9.477, 6.46, 3.157, 5.608, 5.17, 3.133, 10.058, 5.02, 4.009];
#[rustfmt::skip]
const UNIT: &[f64] = &[0.4557, 0.1243, 0.2887, 0.1186, 0.149, 0.3418, 0.3721, 0.1484, 0.4703, 0.3728, 0.4527, 0.0484, 0.2496, 0.2377, 0.2983, 0.2577, 0.4191, 0.3282, 0.0757, 0.1906, 0.5281, 0.0118, 0.3731, 0.2511, 0.2195];
#[rustfmt::skip]
const COUNTS: &[f64] = &[4.0, 5.0, 6.0, 4.0, 0.0, 0.0, 2.0, 9.0, 4.0, 5.0, 4.0, 3.0, 4.0, 3.0, 4.0, 3.0, 1.0, 10.0, 5.0, 1.0, 4.0, 6.0, 6.0, 2.0, 3.0, 9.0, 4.0, 5.0, 8.0, 5.0, 1.0, 2.0, 11.0, 4.0, 3.0, 4.0, 3.0, 0.0, 5.0, 6.0];
#[rustfmt::skip]
const BINO: &[f64] = &[5.0, 6.0, 5.0, 2.0, 0.0, 3.0, 3.0, 4.0, 5.0, 2.0, 6.0, 3.0, 3.0, 1.0, 1.0, 5.0, 5.0, 4.0, 4.0, 6.0];
#[rustfmt::skip]
const TDATA: &[f64] = &[3.156, -1.53, 1.653, -0.659, 4.299, 1.781, -1.417, 1.37, 0.915, 7.549, -0.558, -1.536, 2.369, 3.977, 2.383, 5.53, 0.799, -2.34, 9.775, 0.08, 0.924, 3.36, 0.802, 1.384, 0.732, 2.795, -0.152, 0.562, 1.18, 2.189, 1.475, 0.024, 7.05, -1.352, -0.325, 1.561, 1.416, 0.567, 2.193, 0.454];
#[rustfmt::skip]
const RICE: &[f64] = &[1.553, 1.905, 1.875, 2.396, 3.559, 2.333, 2.056, 1.532, 2.262, 0.963, 2.633, 1.925, 1.335, 1.577, 3.256, 0.91, 3.021, 2.467, 2.521, 2.31, 1.611, 1.473, 2.316, 1.921, 2.208, 1.553, 0.332, 2.094, 1.313, 3.553];
#[rustfmt::skip]
const GPD: &[f64] = &[3.031, 0.866, 0.734, 1.061, 1.874, 0.483, 0.025, 0.725, 3.572, 1.235, 1.309, 0.917, 4.399, 1.705, 5.55, 0.299, 0.164, 0.254, 0.088, 1.86, 3.198, 0.088, 1.237, 0.184, 7.664, 0.345, 0.772, 0.477, 0.87, 0.18];
#[rustfmt::skip]
const CENS: &[f64] = &[0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0];
#[rustfmt::skip]
const FIT_GAMMA: &[f64] = &[2.787645369655894, 1.6336009054703151];
#[rustfmt::skip]
const FIT_WEIBULL: &[f64] = &[5.134856766456421, 1.7737047517082405];
#[rustfmt::skip]
const FIT_LOGNORMAL: &[f64] = &[1.3260279701105884, 0.6604096888793667];
#[rustfmt::skip]
const FIT_EXPONENTIAL: &[f64] = &[4.5539];
#[rustfmt::skip]
const FIT_RAYLEIGH: &[f64] = &[3.747883587217369];
#[rustfmt::skip]
const FIT_LOGLOGISTIC: &[f64] = &[1.369343392293925, 0.3794025409016134];
#[rustfmt::skip]
const FIT_RICIAN: &[f64] = &[1.8525108286867396, 0.7774012195225208];
#[rustfmt::skip]
const FIT_NAKAGAMI: &[f64] = &[0.8602252561811841, 28.093262766666662];
#[rustfmt::skip]
const FIT_INVERSEGAUSSIAN: &[f64] = &[4.5539, 8.882208705158094];
#[rustfmt::skip]
const FIT_BIRNBAUMSAUNDERS: &[f64] = &[3.69377370627692, 0.6781177026181491];
#[rustfmt::skip]
const FIT_HALFNORMAL: &[f64] = &[0.0, 5.30030779923833];
#[rustfmt::skip]
const FIT_GENERALIZEDPARETO: &[f64] = &[0.23366889088374831, 1.1693984348121433, 0.0];
#[rustfmt::skip]
const FIT_NORMAL: &[f64] = &[6.323633333333333, 2.5047318320679985];
#[rustfmt::skip]
const FIT_LOGISTIC: &[f64] = &[6.107506572724759, 1.4346696160308774];
#[rustfmt::skip]
const FIT_EXTREMEVALUE: &[f64] = &[7.622655956142137, 2.6674932227625243];
#[rustfmt::skip]
const FIT_GENERALIZEDEXTREMEVALUE: &[f64] = &[0.06961808451860863, 1.8659953862993783, 5.10802942092041];
#[rustfmt::skip]
const FIT_TLOCATIONSCALE: &[f64] = &[1.1499209042932272, 1.5284058103498883, 2.563862988654142];
#[rustfmt::skip]
const FIT_BETA: &[f64] = &[1.967536039544894, 5.4288370171835805];
#[rustfmt::skip]
const FIT_NEGATIVEBINOMIAL: &[f64] = &[6.725443980142835, 0.6155762632957008];
#[rustfmt::skip]
const FIT_POISSON: &[f64] = &[4.2];
#[rustfmt::skip]
const FIT_BINOMIAL: &[f64] = &[10.0, 0.365];
#[rustfmt::skip]
const MLE_NORMAL: &[f64] = &[6.323633333333333, 2.4626325004397676];
#[rustfmt::skip]
const NLL_GAMMA_NORMAL_WEIBULL_GEV: &[f64] = &[68.75003505623043, 69.61360626832736, 69.08545608389785, 67.42973181823265];
#[rustfmt::skip]
const COV_GAMMA: &[f64] = &[0.46390768618235384, -0.2718566806208023, -0.2718566806208023, 0.19122245241851082];
#[rustfmt::skip]
const COV_NORMAL: &[f64] = &[0.20912271841884109, -1.7209167799589004e-12, -1.7209167799589004e-12, 0.11006458877469807];
#[rustfmt::skip]
const COV_GEV: &[f64] = &[0.04545167472962718, -0.032502584229392194, -0.044322049749420216, -0.03250258422939218, 0.1068988088370543, 0.07544038777781713, -0.04432204974942021, 0.07544038777781714, 0.17459341273385667];
#[rustfmt::skip]
const COV_WEIBULL: &[f64] = &[0.31172338210326334, 0.04404502747006751, 0.044045027470067506, 0.05995335887387465];
#[rustfmt::skip]
const CI_NORMAL: &[f64] = &[5.388351096305785, 7.258915570360882, 1.994785795770211, 3.3671490217757443];
#[rustfmt::skip]
const CI_EXPONENTIAL: &[f64] = &[3.2802116073815735, 6.749560313226506];
#[rustfmt::skip]
const CI_POISSON: &[f64] = &[3.588899032109375, 4.885326720257962];
#[rustfmt::skip]
const CI_BINOMIAL: &[f64] = &[10.0, 10.0, 0.29823342636701194, 0.4358340297899972];
#[rustfmt::skip]
const CI_GAMMA_WALD: &[f64] = &[1.7268823455263276, 4.499997771762194, 0.9667014585059057, 2.760575040900417];
#[rustfmt::skip]
const CI_NORMAL_90_MU: &[f64] = &[5.5466236060965, 7.1006430605701665];
#[rustfmt::skip]
const CI_RAYLEIGH: &[f64] = &[3.180865300835789, 4.5628068962098896];
#[rustfmt::skip]
const CENS_WEIBULL: &[f64] = &[6.0084558368757754, 2.1648469166856827];
#[rustfmt::skip]
const CENS_EXPONENTIAL: &[f64] = &[5.692374972856639, 5.692374999999999];
#[rustfmt::skip]
const CENS_NORMAL: &[f64] = &[5.327168866711428, 2.558216049547875];
#[rustfmt::skip]
const CENS_GAMMA: &[f64] = &[4.220562074641179, 1.2519346339960404];
#[rustfmt::skip]
const CENS_EV: &[f64] = &[8.19710525422866, 2.74276746522596];
#[rustfmt::skip]
const CENS_LOGNORMAL: &[f64] = &[1.5399397311742757, 0.5167485087792743];
#[rustfmt::skip]
const KX: &[f64] = &[1.2, 2.3, 2.9, 3.1, 4.8, 5.0, 5.6, 7.2, 8.1, 9.4];
#[rustfmt::skip]
const KPTS: &[f64] = &[0.5, 2.0, 3.0, 4.4, 6.0, 9.9];
#[rustfmt::skip]
const KPDF_NORMAL: &[f64] = &[0.03687367596803225, 0.1231952405945586, 0.15259155217106238, 0.11754028505586205, 0.09582059129901768, 0.04628206231350235];
#[rustfmt::skip]
const KCDF_NORMAL: &[f64] = &[0.01641245223285664, 0.1364922494322879, 0.28435840032651577, 0.4473747397044982, 0.6640910229024068, 0.9757353382231114];
#[rustfmt::skip]
const KPDF_EPAN: &[f64] = &[0.0383325938999964, 0.13787999336478296, 0.13377292973264046, 0.12418978125764141, 0.11265088819590775, 0.04302638090815922];
#[rustfmt::skip]
const KCDF_EPAN: &[f64] = &[0.018695048315002943, 0.13839404551786294, 0.2813049516849971, 0.45551519973459137, 0.6601456651907086, 0.9731429776096917];
#[rustfmt::skip]
const KPDF_POSITIVE: &[f64] = &[0.003764044680869456, 0.1307673896162513, 0.147798054091105, 0.12107863886696033, 0.09919146374005462, 0.03557913228001346];
#[rustfmt::skip]
const KBW_DEFAULT: &[f64] = &[2.130317098903304];
#[rustfmt::skip]
const EY: &[f64] = &[3.0, 1.0, 4.0, 1.0, 5.0, 9.0, 2.0, 6.0, 5.0, 3.0, 5.0, 8.0];
#[rustfmt::skip]
const EC: &[f64] = &[0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
#[rustfmt::skip]
const ECDF_F: &[f64] = &[0.0, 0.08333333333333337, 0.17500000000000004, 0.3583333333333334, 0.44999999999999996, 0.6333333333333333, 0.7555555555555555, 0.8777777777777778];
#[rustfmt::skip]
const ECDF_X: &[f64] = &[1.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0];
#[rustfmt::skip]
const ECDF_LO: &[f64] = &[f64::NAN, 0.0, 0.0, 0.07590091618362083, 0.15627774568238406, 0.34805799879200255, 0.4827437365449332, 0.6602945605877415];
#[rustfmt::skip]
const ECDF_HI: &[f64] = &[f64::NAN, 0.23971022075206438, 0.3960397122141255, 0.640765750483046, 0.7437222543176158, 0.9186086678746641, 1.0, 1.0];
#[rustfmt::skip]
const LIKE_NORM: &[f64] = &[77.67468653793853];
#[rustfmt::skip]
const LIKE_GAM: &[f64] = &[70.11649173027905];
#[rustfmt::skip]
const LIKE_WBL: &[f64] = &[71.40761515971744];
#[rustfmt::skip]
const LIKE_EV: &[f64] = &[100.4780505012724];
#[rustfmt::skip]
const LIKE_BETA: &[f64] = &[-13.26776469720659];
#[rustfmt::skip]
const LIKE_EXP: &[f64] = &[75.74308083359672];
#[rustfmt::skip]
const LIKE_LOGN: &[f64] = &[69.61924224015708];
#[rustfmt::skip]
const LIKE_WBL_ACOV: &[f64] = &[0.2520403250802034, 0.07564224796151832, 0.07564224796151832, 0.05433126317882221];
#[rustfmt::skip]
const PX: &[f64] = &[2.1, 0.7, 3.3, 1.5, 2.8];
#[rustfmt::skip]
const NORMPLOT_Y: &[f64] = &[-1.2815515655446004, -0.5244005127080409, 0.0, 0.5244005127080407, 1.2815515655446004];
#[rustfmt::skip]
const WBLPLOT_Y: &[f64] = &[-2.2503673273124454, -1.0309304331587232, -0.36651292058166435, 0.18562675886236557, 0.834032445247956];
#[rustfmt::skip]
#[allow(clippy::approx_constant)]
const NEW_FAMILY_POINTS: &[f64] = &[0.108947496880907, 0.679178699175393, 2.694595720774407, 1.0, 13.159472534785811, 1.0, 0.32692842051033444, 0.43069138136386487, 3.504995925894142, 3.166707602701757, 4.616773625632246, 2.718281828459045, 0.3043591652233475, 0.48564105207440955, 3.253835436896882, 2.6347331436945662, 1.5581812615173458, 2.5472546674340357, 0.08437262935359277, 0.9753009851112622, 1.5630889893989435, 1.30294003174112, 0.3023472736864494, 1.2559123866404425, 0.13148065215253227, 0.6082454758656212, 3.2898187981638554, 2.642297137253033, 13.376142249191547, 1.7605608513900515, 0.21616379795577814, 0.5028232647017103, 3.722596365393921, 3.5, 10.416666666666666, 2.4869835499703496, 0.13784905803596834, 0.7564878759703453, 2.1188592889387206, 1.0, 6.666666666666667, 1.0, 0.16837384790974963, 0.7455990771901392, 2.253125822882857, 2.0, 2.666666666666666, 1.512250663605367, 0.2905816065687652, 0.6726395769907115, 2.5976367878802247, 2.25, 1.3125, 2.0, 0.30113743215480443, 0.5467452952462635, 3.0728667789875797, 2.5957691216057306, 1.4535209105296745, 2.348979500392163];

fn list(v: &[f64]) -> String {
    format!(
        "[{}]",
        v.iter()
            .map(|x| format!("{x:.17e}"))
            .collect::<Vec<_>>()
            .join(" ")
    )
}
fn values(e: &mut Engine, code: &str) -> Vec<f64> {
    e.execute(&format!("zz={code};"))
        .unwrap_or_else(|s| panic!("{code}: {s}"));
    match e.workspace().get("zz").unwrap() {
        Array::Numeric(m) => m.data().to_vec(),
        Array::Logical(m) => m.data().iter().map(|&v| f64::from(v)).collect(),
        a => panic!("{code}: got {}", a.class()),
    }
}
fn run(e: &mut Engine, code: &str) {
    e.execute(code).unwrap_or_else(|s| panic!("{code}: {s}"));
}
fn close(got: f64, want: f64, tol: f64, what: &str) {
    let ok = if want.is_nan() {
        got.is_nan()
    } else {
        (got - want).abs() <= tol * want.abs().max(1e-300)
    };
    assert!(ok, "{what}: got {got:.17e}, want {want:.17e}");
}
fn all_close(got: &[f64], want: &[f64], tol: f64, what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: {got:?} vs {want:?}");
    for (g, w) in got.iter().zip(want) {
        close(*g, *w, tol, what);
    }
}
fn engine() -> Engine {
    let mut e = Engine::default();
    run(
        &mut e,
        &format!(
            "pos={}; real={}; unit={}; counts={}; bino={}; tdata={}; rice={}; gpd={}; cens={};",
            list(POS),
            list(REAL),
            list(UNIT),
            list(COUNTS),
            list(BINO),
            list(TDATA),
            list(RICE),
            list(GPD),
            list(CENS)
        ),
    );
    e
}

#[test]
fn new_families_match_scipy_densities_quantiles_and_moments() {
    let mut e = Engine::default();
    let objects = [
        "makedist('Logistic','mu',1,'sigma',2)",
        "makedist('Loglogistic','mu',1,'sigma',0.3)",
        "makedist('Rician','s',2,'sigma',1.5)",
        "makedist('Nakagami','mu',1.5,'omega',2)",
        "makedist('GeneralizedExtremeValue','k',0.2,'sigma',2,'mu',1)",
        "makedist('GeneralizedPareto','k',0.2,'sigma',2,'theta',1)",
        "makedist('tLocationScale','mu',1,'sigma',2,'nu',5)",
        "makedist('InverseGaussian','mu',2,'lambda',3)",
        "makedist('BirnbaumSaunders','beta',2,'gamma',0.5)",
        "makedist('HalfNormal','mu',1,'sigma',2)",
    ];
    for (k, code) in objects.iter().enumerate() {
        run(&mut e, &format!("pd={code};"));
        let want = &NEW_FAMILY_POINTS[6 * k..6 * k + 6];
        let got = [
            values(&mut e, "pdf(pd,2.5)")[0],
            values(&mut e, "cdf(pd,2.5)")[0],
            values(&mut e, "icdf(pd,0.7)")[0],
            values(&mut e, "mean(pd)")[0],
            values(&mut e, "var(pd)")[0],
            values(&mut e, "median(pd)")[0],
        ];
        all_close(&got, want, 1e-9, code);
        close(
            values(&mut e, "cdf(pd,2.5,'upper')")[0],
            1. - want[1],
            1e-9,
            code,
        );
        // Random draws come from the object's own law (rough check).
        run(&mut e, "rng(3); r=random(pd,4000,1);");
        let p = values(&mut e, "mean(r <= median(pd))")[0];
        assert!((p - 0.5).abs() < 0.04, "{code}: {p}");
    }
    run(&mut e, "pd=makedist('tLocationScale');");
    assert_eq!(values(&mut e, "pd.ParameterValues"), [0., 1., 5.]);
    assert_eq!(
        values(&mut e, "isequal(pd.ParameterNames,{'mu','sigma','nu'})"),
        [1.]
    );
    assert_eq!(
        values(
            &mut e,
            "strcmp(class(pd),'prob.tLocationScaleDistribution')"
        ),
        [1.]
    );
    // The generic functions take the new names too.
    close(
        values(&mut e, "pdf('Logistic',2.5,1,2)")[0],
        NEW_FAMILY_POINTS[0],
        1e-12,
        "pdf",
    );
}

fn fit_case(e: &mut Engine, code: &str, want: &[f64], tol: f64) {
    run(e, &format!("pd={code};"));
    let got = values(e, "pd.ParameterValues");
    all_close(&got, want, tol, code);
}

#[test]
fn fitdist_every_family_matches_the_scipy_mle() {
    let mut e = engine();
    for (code, want, tol) in [
        ("fitdist(real,'Normal')", FIT_NORMAL, 1e-12),
        ("fitdist(pos,'Lognormal')", FIT_LOGNORMAL, 1e-12),
        ("fitdist(pos,'Exponential')", FIT_EXPONENTIAL, 1e-12),
        ("fitdist(pos,'Rayleigh')", FIT_RAYLEIGH, 1e-12),
        ("fitdist(counts,'Poisson')", FIT_POISSON, 1e-12),
        ("fitdist(bino,'Binomial','NTrials',10)", FIT_BINOMIAL, 1e-12),
        ("fitdist(pos,'InverseGaussian')", FIT_INVERSEGAUSSIAN, 1e-12),
        ("fitdist(pos,'HalfNormal')", FIT_HALFNORMAL, 1e-12),
        ("fitdist(pos,'Gamma')", FIT_GAMMA, 1e-8),
        ("fitdist(pos,'Weibull')", FIT_WEIBULL, 1e-8),
        ("fitdist(real,'ExtremeValue')", FIT_EXTREMEVALUE, 1e-8),
        (
            "fitdist(counts,'NegativeBinomial')",
            FIT_NEGATIVEBINOMIAL,
            1e-8,
        ),
        ("fitdist(pos,'Nakagami')", FIT_NAKAGAMI, 1e-8),
        ("fitdist(unit,'Beta')", FIT_BETA, 1e-6),
        ("fitdist(real,'Logistic')", FIT_LOGISTIC, 1e-6),
        ("fitdist(pos,'Loglogistic')", FIT_LOGLOGISTIC, 1e-6),
        ("fitdist(rice,'Rician')", FIT_RICIAN, 1e-6),
        (
            "fitdist(pos,'BirnbaumSaunders')",
            FIT_BIRNBAUMSAUNDERS,
            1e-6,
        ),
        (
            "fitdist(gpd,'GeneralizedPareto')",
            FIT_GENERALIZEDPARETO,
            1e-6,
        ),
        (
            "fitdist(real,'GeneralizedExtremeValue')",
            FIT_GENERALIZEDEXTREMEVALUE,
            1e-6,
        ),
        ("fitdist(tdata,'tLocationScale')", FIT_TLOCATIONSCALE, 1e-6),
    ] {
        fit_case(&mut e, code, want, tol);
    }
    // Names, classes and fixed parameters of the fitted objects.
    run(&mut e, "pd=fitdist(gpd,'GeneralizedPareto','Theta',0);");
    assert_eq!(values(&mut e, "pd.ParameterIsFixed"), [0., 0., 1.]);
    assert_eq!(
        values(&mut e, "strcmp(pd.DistributionName,'GeneralizedPareto')"),
        [1.]
    );
    run(&mut e, "pd=fitdist(pos-1,'GeneralizedPareto','Theta',-1);");
    let shifted = values(&mut e, "pd.ParameterValues");
    assert_eq!(shifted[2], -1.);
    run(&mut e, "pd=fitdist(pos+2,'HalfNormal','mu',2);");
    close(
        values(&mut e, "pd.sigma")[0],
        FIT_HALFNORMAL[1],
        1e-12,
        "mu",
    );
    run(&mut e, "pd=fitdist(bino,'Binomial','NTrials',10);");
    assert_eq!(values(&mut e, "pd.ParameterIsFixed"), [1., 0.]);
    assert_eq!(
        values(&mut e, "pd.ParameterCovariance(1,:)"),
        [0., 0.],
        "fixed N has no variance"
    );
    run(&mut e, "pd=fitdist(real,'ExtremeValue');");
    assert_eq!(
        values(&mut e, "strcmp(pd.DistributionName,'ExtremeValue')"),
        [1.]
    );
    assert_eq!(
        values(
            &mut e,
            "iscell(pd.ParameterDescription) && numel(pd.ParameterDescription)==2"
        ),
        [1.]
    );
    assert_eq!(values(&mut e, "pd.Truncation"), Vec::<f64>::new());
    assert_eq!(values(&mut e, "pd.IsTruncated"), [0.]);
    assert_eq!(values(&mut e, "pd.InputData.data'"), REAL);
    // Statset options are accepted.
    run(
        &mut e,
        "o=statset('MaxIter',500,'TolX',1e-8); pd=fitdist(real,'Logistic','Options',o);",
    );
    all_close(
        &values(&mut e, "pd.ParameterValues"),
        FIT_LOGISTIC,
        1e-6,
        "statset",
    );
    assert_eq!(values(&mut e, "o.MaxIter"), [500.]);
}

#[test]
fn parameter_covariance_is_the_inverse_observed_information() {
    let mut e = engine();
    for (code, want, tol) in [
        ("fitdist(pos,'Gamma')", COV_GAMMA, 2e-5),
        ("fitdist(real,'Normal')", COV_NORMAL, 2e-5),
        ("fitdist(real,'GeneralizedExtremeValue')", COV_GEV, 1e-4),
        ("fitdist(pos,'Weibull')", COV_WEIBULL, 2e-5),
    ] {
        run(&mut e, &format!("pd={code}; c=pd.ParameterCovariance;"));
        let got = values(&mut e, "c");
        for (g, w) in got.iter().zip(want) {
            // Off-diagonal zeros (Normal) are compared absolutely.
            let scale = want.iter().fold(0f64, |m, v| m.max(v.abs()));
            assert!((g - w).abs() <= tol * scale, "{code}: {got:?} vs {want:?}");
        }
    }
    run(&mut e, "pd=fitdist(counts,'Poisson');");
    close(
        values(&mut e, "pd.ParameterCovariance")[0],
        4.2 / 40.,
        1e-12,
        "poisson",
    );
}

#[test]
fn paramci_exact_intervals_and_one_wald_case() {
    let mut e = engine();
    run(&mut e, "pd=fitdist(real,'Normal');");
    all_close(&values(&mut e, "paramci(pd)"), CI_NORMAL, 1e-10, "normal");
    all_close(
        &values(&mut e, "paramci(pd,'Alpha',0.1,'Parameter','mu')"),
        CI_NORMAL_90_MU,
        1e-10,
        "alpha",
    );
    assert_eq!(values(&mut e, "size(paramci(pd))"), [2., 2.]);
    // normfit gives the same exact intervals.
    run(&mut e, "[m,s,mci,sci]=normfit(real);");
    all_close(&values(&mut e, "[mci; sci]"), CI_NORMAL, 1e-10, "normfit");
    run(&mut e, "pd=fitdist(pos,'Exponential');");
    all_close(&values(&mut e, "paramci(pd)"), CI_EXPONENTIAL, 1e-10, "exp");
    run(&mut e, "pd=fitdist(counts,'Poisson');");
    all_close(&values(&mut e, "paramci(pd)"), CI_POISSON, 1e-10, "poisson");
    run(&mut e, "pd=fitdist(bino,'Binomial','NTrials',10);");
    all_close(
        &values(&mut e, "paramci(pd)"),
        CI_BINOMIAL,
        1e-10,
        "binomial",
    );
    run(&mut e, "pd=fitdist(pos,'Gamma');");
    all_close(&values(&mut e, "paramci(pd)"), CI_GAMMA_WALD, 1e-6, "gamma");
    all_close(
        &values(&mut e, "pd.paramci('Parameter',{'b'})"),
        &CI_GAMMA_WALD[2..],
        1e-6,
        "method form",
    );
    // A makedist object has no estimation error.
    assert_eq!(
        values(&mut e, "paramci(makedist('Normal','mu',2,'sigma',3))"),
        [2., 2., 3., 3.]
    );
}

#[test]
fn nlogl_property_and_negloglik_agree_with_scipy() {
    let mut e = engine();
    for (k, code) in [
        "fitdist(pos,'Gamma')",
        "fitdist(real,'Normal')",
        "fitdist(pos,'Weibull')",
        "fitdist(real,'GeneralizedExtremeValue')",
    ]
    .iter()
    .enumerate()
    {
        run(&mut e, &format!("pd={code};"));
        let tol = if k == 3 { 1e-9 } else { 1e-11 };
        close(
            values(&mut e, "pd.NLogL")[0],
            NLL_GAMMA_NORMAL_WEIBULL_GEV[k],
            tol,
            code,
        );
        close(
            values(&mut e, "negloglik(pd)")[0],
            NLL_GAMMA_NORMAL_WEIBULL_GEV[k],
            tol,
            code,
        );
    }
    assert_eq!(
        values(&mut e, "negloglik(makedist('Normal'))"),
        Vec::<f64>::new()
    );
}

#[test]
fn right_censored_fits_match_a_scipy_minimisation() {
    let mut e = engine();
    fit_case(
        &mut e,
        "fitdist(pos,'Weibull','Censoring',cens)",
        CENS_WEIBULL,
        1e-8,
    );
    fit_case(
        &mut e,
        "fitdist(pos,'Exponential','Censoring',logical(cens))",
        &CENS_EXPONENTIAL[1..],
        1e-12,
    );
    close(
        CENS_EXPONENTIAL[0],
        CENS_EXPONENTIAL[1],
        1e-8,
        "scipy minimise",
    );
    fit_case(
        &mut e,
        "fitdist(pos,'Normal','Censoring',cens)",
        CENS_NORMAL,
        1e-6,
    );
    fit_case(
        &mut e,
        "fitdist(pos,'Gamma','Censoring',cens)",
        CENS_GAMMA,
        1e-6,
    );
    fit_case(
        &mut e,
        "fitdist(real,'ExtremeValue','Censoring',cens)",
        CENS_EV,
        1e-6,
    );
    fit_case(
        &mut e,
        "fitdist(pos,'Lognormal','Censoring',cens)",
        CENS_LOGNORMAL,
        1e-6,
    );
    all_close(
        &values(&mut e, "evfit(real,[],cens)"),
        CENS_EV,
        1e-6,
        "evfit",
    );
    run(&mut e, "pd=fitdist(pos,'Weibull','Censoring',cens);");
    assert_eq!(values(&mut e, "pd.InputData.cens'"), CENS);
    let text = e.execute("pd").unwrap().output.join("\n");
    assert!(text.contains("6 right-censored"), "{text}");
    // wblfit and expfit-style intervals with censoring.
    run(&mut e, "[p,ci]=wblfit(pos,0.05,cens);");
    all_close(&values(&mut e, "p"), CENS_WEIBULL, 1e-8, "wblfit");
    assert_eq!(values(&mut e, "size(ci)"), [2., 2.]);
}

#[test]
fn frequencies_equal_repeated_observations() {
    let mut e = engine();
    for name in [
        "Gamma",
        "Weibull",
        "Normal",
        "Logistic",
        "ExtremeValue",
        "Lognormal",
    ] {
        run(
            &mut e,
            &format!(
                "a=fitdist([1.5 2.25 3 4.5 7],'{name}','Frequency',[1 2 3 1 2]); b=fitdist([1.5 2.25 2.25 3 3 3 4.5 7 7],'{name}');"
            ),
        );
        let a = values(&mut e, "[a.ParameterValues a.NLogL]");
        let b = values(&mut e, "[b.ParameterValues b.NLogL]");
        all_close(&a, &b, 1e-8, name);
        // Numerical Hessians sum in a different order: looser.
        let a = values(&mut e, "a.ParameterCovariance(:)'");
        let b = values(&mut e, "b.ParameterCovariance(:)'");
        let scale = b.iter().fold(0f64, |m, v| m.max(v.abs()));
        for (x, y) in a.iter().zip(&b) {
            assert!((x - y).abs() <= 1e-5 * scale, "{name}: {a:?} vs {b:?}");
        }
    }
    run(&mut e, "a=fitdist([1 2 3],'Poisson','Frequency',[2 0 1]);");
    assert_eq!(values(&mut e, "a.lambda"), [5. / 3.]);
    assert_eq!(values(&mut e, "a.InputData.freq'"), [2., 1.]);
}

#[test]
fn by_groups_fit_each_group_separately() {
    let mut e = engine();
    run(
        &mut e,
        "g=[1 2 1 2 1 2 1 2 1 2 1 2 1 2 1 2 1 2 1 2 1 2 1 2 1 2 1 2 1 2]; [pd,gn,gl]=fitdist(real','Normal','By',g');",
    );
    assert_eq!(values(&mut e, "iscell(pd) && numel(pd)==2"), [1.]);
    assert_eq!(
        values(&mut e, "isequal(gn,{'1';'2'}) && isequal(gl,gn)"),
        [1.]
    );
    run(
        &mut e,
        "one=fitdist(real(1:2:end)','Normal'); two=fitdist(real(2:2:end)','Normal');",
    );
    assert_eq!(
        values(&mut e, "[pd{1}.ParameterValues pd{2}.ParameterValues]"),
        values(&mut e, "[one.ParameterValues two.ParameterValues]")
    );
    run(
        &mut e,
        "c=repmat({'b','a'},1,15); [pd,gn]=fitdist(pos,'Gamma','By',c);",
    );
    assert_eq!(values(&mut e, "numel(pd)"), [2.]);
    assert_eq!(values(&mut e, "isequal(gn,{'a';'b'})"), [1.]);
    run(&mut e, "b=fitdist(pos(1:2:end),'Gamma');");
    assert_eq!(
        values(&mut e, "pd{2}.ParameterValues"),
        values(&mut e, "b.ParameterValues")
    );
}

#[test]
fn kernel_pdf_and_cdf_match_a_numpy_kde() {
    let mut e = Engine::default();
    run(&mut e, &format!("kx={}; t={};", list(KX), list(KPTS)));
    run(&mut e, "pd=fitdist(kx,'Kernel','Bandwidth',0.7);");
    all_close(
        &values(&mut e, "pdf(pd,t)"),
        KPDF_NORMAL,
        1e-12,
        "normal pdf",
    );
    all_close(
        &values(&mut e, "cdf(pd,t)"),
        KCDF_NORMAL,
        1e-12,
        "normal cdf",
    );
    all_close(
        &values(&mut e, "cdf(pd,icdf(pd,[0.1 0.5 0.9]))"),
        &[0.1, 0.5, 0.9],
        1e-12,
        "icdf",
    );
    close(values(&mut e, "mean(pd)")[0], 4.96, 1e-12, "mean");
    close(
        values(&mut e, "var(pd)")[0],
        values(&mut e, "var(kx,1)+0.49")[0],
        1e-12,
        "var",
    );
    assert_eq!(values(&mut e, "pd.BandWidth"), [0.7]);
    assert_eq!(values(&mut e, "strcmp(pd.Kernel,'normal')"), [1.]);
    assert_eq!(
        values(&mut e, "strcmp(class(pd),'prob.KernelDistribution')"),
        [1.]
    );
    run(
        &mut e,
        "pd=fitdist(kx,'Kernel','Kernel','epanechnikov','Width',0.7);",
    );
    all_close(&values(&mut e, "pdf(pd,t)"), KPDF_EPAN, 1e-12, "epan pdf");
    all_close(&values(&mut e, "cdf(pd,t)"), KCDF_EPAN, 1e-12, "epan cdf");
    run(
        &mut e,
        "pd=fitdist(kx,'Kernel','Support','positive','Bandwidth',0.3);",
    );
    all_close(
        &values(&mut e, "pdf(pd,t)"),
        KPDF_POSITIVE,
        1e-12,
        "positive",
    );
    assert_eq!(values(&mut e, "[pdf(pd,-1) cdf(pd,0)]"), [0., 0.]);
    close(
        values(&mut e, "integral(@(x) pdf(pd,x),0,60)")[0],
        1.,
        1e-6,
        "mass",
    );
    run(&mut e, "pd=fitdist(kx,'Kernel');");
    close(
        values(&mut e, "pd.BandWidth")[0],
        KBW_DEFAULT[0],
        1e-12,
        "default h",
    );
    run(
        &mut e,
        "pd=fitdist(kx,'Kernel','Support',[0 10],'Kernel','box');",
    );
    close(
        values(&mut e, "integral(@(x) pdf(pd,x),0,10)")[0],
        1.,
        1e-6,
        "bounded mass",
    );
    let m = values(&mut e, "mean(pd)")[0];
    assert!(m > 0. && m < 10., "{m}");
    run(&mut e, "rng(1); r=random(pd,1000,1);");
    assert_eq!(values(&mut e, "all(r>0 & r<10)"), [1.]);
    let text = e.execute("pd").unwrap().output.join("\n");
    assert!(
        text.contains("Kernel = box") && text.contains("Support = [0, 10]"),
        "{text}"
    );
}

#[test]
fn kernel_negloglik_groups_repeated_values() {
    // tool/refs/stats/kernel_nll.py (NumPy direct double sums).
    let mut e = Engine::default();
    run(&mut e, "x=[1 1 2 2 2 3.5 4 4 7];");
    let nll = values(&mut e, "negloglik(fitdist(x,'Kernel','Bandwidth',0.7))")[0];
    close(nll, 15.408178094314653, 1e-12, "normal kernel with ties");
    run(
        &mut e,
        "pd=fitdist(x,'Kernel','Kernel','epanechnikov','Width',0.7);",
    );
    let nll = values(&mut e, "pd.NLogL")[0];
    close(nll, 16.147803516282814, 1e-12, "epanechnikov with ties");
    // 1e6 equal points: one distinct value, so O(n log n), not O(n^2).
    let start = std::time::Instant::now();
    let nll = values(&mut e, "negloglik(fitdist(ones(1000000,1),'Kernel'))")[0];
    assert!(start.elapsed().as_secs_f64() < 5., "{:?}", start.elapsed());
    close(nll, -1786627.163897826, 1e-10, "constant data");
}

#[test]
fn mle_named_and_custom_distributions_reproduce_the_normal_mle() {
    let mut e = engine();
    all_close(
        &values(&mut e, "mle(real)"),
        MLE_NORMAL,
        1e-12,
        "default normal",
    );
    all_close(
        &values(&mut e, "mle(pos,'distribution','gamma')"),
        FIT_GAMMA,
        1e-8,
        "gamma",
    );
    run(&mut e, "[p,ci]=mle(pos,'Distribution','Gamma');");
    all_close(&values(&mut e, "ci"), CI_GAMMA_WALD, 1e-6, "gamma ci");
    run(&mut e, "[p,ci]=mle(real);");
    all_close(&values(&mut e, "ci"), CI_NORMAL, 1e-10, "normal ci");
    all_close(
        &values(&mut e, "mle(bino,'distribution','binomial','NTrials',10)"),
        &FIT_BINOMIAL[1..],
        1e-12,
        "binomial",
    );
    all_close(
        &values(
            &mut e,
            "mle(real,'pdf',@(x,m,s) normpdf(x,m,s),'Start',[5 2])",
        ),
        MLE_NORMAL,
        1e-6,
        "pdf",
    );
    all_close(
        &values(
            &mut e,
            "mle(real,'nloglf',@(p,x,c,f) -sum(f.*log(normpdf(x,p(1),p(2)))),'Start',[5 2],'LowerBound',[-Inf 0])",
        ),
        MLE_NORMAL,
        1e-6,
        "nloglf",
    );
    all_close(
        &values(
            &mut e,
            "mle(real,'logpdf',@(x,m,s) -0.5*((x-m)/s).^2-log(s)-0.5*log(2*pi),'Start',[5 2])",
        ),
        MLE_NORMAL,
        1e-6,
        "logpdf",
    );
    run(
        &mut e,
        "[p,ci]=mle(real,'pdf',@(x,m,s) normpdf(x,m,s),'Start',[5 2]);",
    );
    let ci = values(&mut e, "ci");
    let se = MLE_NORMAL[1] / (REAL.len() as f64).sqrt();
    close(
        ci[1] - ci[0],
        2. * 1.959963984540054 * se,
        1e-4,
        "wald width",
    );
    // Censored custom fit with a cdf matches the censored normal fit.
    all_close(
        &values(
            &mut e,
            "mle(pos,'pdf',@(x,m,s) normpdf(x,m,s),'cdf',@(x,m,s) normcdf(x,m,s),'Censoring',cens,'Start',[5 2])",
        ),
        CENS_NORMAL,
        1e-6,
        "censored custom",
    );
}

#[test]
fn family_fit_functions_return_estimates_and_intervals() {
    let mut e = engine();
    for (code, want, tol) in [
        ("gamfit(pos)", FIT_GAMMA, 1e-8),
        ("wblfit(pos)", FIT_WEIBULL, 1e-8),
        ("lognfit(pos)", FIT_LOGNORMAL, 1e-12),
        ("betafit(unit)", FIT_BETA, 1e-6),
        ("raylfit(pos)", FIT_RAYLEIGH, 1e-12),
        ("evfit(real)", FIT_EXTREMEVALUE, 1e-8),
        ("poissfit(counts)", FIT_POISSON, 1e-12),
        ("nbinfit(counts)", FIT_NEGATIVEBINOMIAL, 1e-8),
        ("gevfit(real)", FIT_GENERALIZEDEXTREMEVALUE, 1e-6),
        ("gpfit(gpd)", &FIT_GENERALIZEDPARETO[..2], 1e-6),
    ] {
        all_close(&values(&mut e, code), want, tol, code);
    }
    run(&mut e, "[p,ci]=gamfit(pos);");
    all_close(&values(&mut e, "ci"), CI_GAMMA_WALD, 1e-6, "gamfit ci");
    run(&mut e, "[p,ci]=raylfit(pos);");
    all_close(&values(&mut e, "ci"), CI_RAYLEIGH, 1e-10, "raylfit ci");
    run(&mut e, "[p,ci]=poissfit(counts);");
    all_close(&values(&mut e, "ci"), CI_POISSON, 1e-10, "poissfit ci");
    run(&mut e, "[p,ci]=binofit(sum(bino),200);");
    all_close(
        &values(&mut e, "[p ci]"),
        &[0.365, CI_BINOMIAL[2], CI_BINOMIAL[3]],
        1e-10,
        "binofit",
    );
    run(&mut e, "[p,ci]=binofit([0 3],[5 3]);");
    assert_eq!(values(&mut e, "p"), [0., 1.]);
    let ci = values(&mut e, "ci");
    assert_eq!((ci[0], ci[3]), (0., 1.));
    close(
        ci[2],
        1. - 0.025f64.powf(1. / 5.),
        1e-10,
        "clopper-pearson x=0",
    );
    run(&mut e, "[p,ci]=lognfit(pos);");
    assert_eq!(values(&mut e, "size(ci)"), [2., 2.]);
    run(&mut e, "[p,ci]=gevfit(real,0.1);");
    assert_eq!(values(&mut e, "size(ci)"), [2., 3.]);
    assert_eq!(values(&mut e, "all(ci(1,:)<p & p<ci(2,:))"), [1.]);
}

#[test]
fn likelihood_functions_match_scipy() {
    let mut e = engine();
    for (code, want) in [
        ("normlike([5 2],real)", LIKE_NORM),
        ("gamlike([2 2],pos)", LIKE_GAM),
        ("wbllike([4 1.5],pos)", LIKE_WBL),
        ("evlike([6 2],real)", LIKE_EV),
        ("betalike([2 5],unit)", LIKE_BETA),
        ("explike(4,pos)", LIKE_EXP),
        ("lognlike([1.3 0.6],pos)", LIKE_LOGN),
    ] {
        close(values(&mut e, code)[0], want[0], 1e-11, code);
    }
    run(&mut e, "[nll,acov]=wbllike([4 1.5],pos);");
    all_close(&values(&mut e, "acov"), LIKE_WBL_ACOV, 2e-5, "acov");
    run(&mut e, "[nll,acov]=normlike(mle(real),real);");
    let acov = values(&mut e, "acov");
    let s2 = MLE_NORMAL[1] * MLE_NORMAL[1];
    close(acov[0], s2 / 30., 1e-10, "var mu");
    close(acov[3], s2 / 60., 1e-10, "var sigma");
    // With censoring the likelihood uses the survival function.
    let a = values(&mut e, "wbllike([4 1.5],pos,cens)")[0];
    let b = values(
        &mut e,
        "-sum(log(wblpdf(pos(~cens),4,1.5)))-sum(log(1-wblcdf(pos(logical(cens)),4,1.5)))",
    )[0];
    close(a, b, 1e-12, "censored wbllike");
}

#[test]
fn rician_cdf_terminates_for_large_noncentrality() {
    // tool/refs/stats/rician_large.py: x = s gives (1 - e^(-a^2) I0(a^2))/2
    // exactly (mpmath; SciPy rice.cdf returns NaN there); s/sigma = 1500 against
    // mpmath quadrature of the density (SciPy rice.cdf agrees to 1e-13).
    let mut e = Engine::default();
    let start = std::time::Instant::now();
    let got = values(&mut e, "cdf(makedist('Rician','s',2e8,'sigma',1),2e8)")[0];
    assert!(start.elapsed().as_secs_f64() < 1., "{:?}", start.elapsed());
    close(got, 0.4999999990026443, 1e-12, "s=2e8");
    all_close(
        &values(
            &mut e,
            "cdf(makedist('Rician','s',3000,'sigma',2),[2990 3001 3010])",
        ),
        &[2.86155585012604e-07, 0.6913451159365372, 0.9999997128532672],
        1e-9,
        "s=3000 sigma=2",
    );
    let upper = values(
        &mut e,
        "cdf(makedist('Rician','s',3000,'sigma',2),3010,'upper')",
    )[0];
    close(upper, 1. - 0.9999997128532672, 1e-6, "upper tail");
}

#[test]
fn like_variances_are_the_inverse_information_at_the_given_parameters() {
    // explike: 1/(2 sum(x)/mu^3 - n/mu^2) at mu = 1, x = [1 2 3] is 1/9 (the
    // MLE-only mu^2/n would give 1/3). With censoring n counts the uncensored
    // observations only: mu = 2, x = [1 2 3 4], last censored -> 1/(20/8 - 3/4).
    let mut e = Engine::default();
    run(&mut e, "[nll,v]=explike(1,[1 2 3]);");
    close(values(&mut e, "v")[0], 1. / 9., 1e-14, "explike variance");
    close(values(&mut e, "nll")[0], 6., 1e-14, "explike nll");
    run(&mut e, "[~,v]=explike(2,[1 2 3 4],[0 0 0 1]);");
    close(
        values(&mut e, "v")[0],
        1. / (20. / 8. - 3. / 4.),
        1e-14,
        "censored",
    );
    // At the MLE it is still mu^2/n.
    run(&mut e, "[~,v]=explike(2,[1 2 3]);");
    close(values(&mut e, "v")[0], 4. / 3., 1e-14, "at the MLE");
    // normlike's analytic Hessian holds away from the MLE too: at (mu, sigma)
    // = (1, 2) with x = [1 2 3], H = [3/4 3/4; 3/4 3*5/16-3/4] (Normal NLL).
    run(&mut e, "[~,c]=normlike([1 2],[1 2 3]);");
    let h = [0.75, 0.75, 0.75, 15. / 16. - 0.75];
    let det = h[0] * h[3] - h[1] * h[2];
    all_close(
        &values(&mut e, "c"),
        &[h[3] / det, -h[1] / det, -h[2] / det, h[0] / det],
        1e-12,
        "normlike away from the MLE",
    );
}

#[test]
fn ecdf_handles_ties_censoring_and_greenwood_bounds() {
    let mut e = Engine::default();
    run(
        &mut e,
        &format!(
            "y={}; c={}; [f,x,flo,fup]=ecdf(y,'Censoring',c,'Bounds','on');",
            list(EY),
            list(EC)
        ),
    );
    all_close(&values(&mut e, "f"), ECDF_F, 1e-14, "f");
    all_close(&values(&mut e, "x"), ECDF_X, 0., "x");
    all_close(&values(&mut e, "flo"), ECDF_LO, 1e-12, "flo");
    all_close(&values(&mut e, "fup"), ECDF_HI, 1e-12, "fup");
    assert_eq!(values(&mut e, "size(f,2)"), [1.]);
    run(&mut e, "[f,x]=ecdf([3 1 2 2]);");
    assert_eq!(
        values(&mut e, "[f x]"),
        [0., 0.25, 0.75, 1., 1., 1., 2., 3.]
    );
    run(&mut e, "[f,x]=ecdf([3 1 2],'Frequency',[1 2 1]);");
    assert_eq!(values(&mut e, "f'"), [0., 0.5, 0.75, 1.]);
    run(&mut e, "[s,x]=ecdf([3 1 2 2],'Function','survivor');");
    assert_eq!(values(&mut e, "s'"), [1., 0.75, 0.25, 0.]);
    run(&mut e, "figure; ecdf([3 1 2 2]);");
    assert_eq!(values(&mut e, "numel(get(gca,'Children'))"), [1.]);
    run(&mut e, "[h,stats]=cdfplot([4 1 2 2 NaN]);");
    assert_eq!(
        values(&mut e, "[stats.min stats.max stats.mean stats.median]"),
        [1., 4., 2.25, 2.]
    );
    close(
        values(&mut e, "stats.std")[0],
        (4.75f64 / 3.).sqrt(),
        1e-14,
        "std",
    );
    assert_eq!(values(&mut e, "isgraphics(h)"), [1.]);
}

#[test]
fn probability_and_quantile_plots_draw_the_expected_lines() {
    let mut e = Engine::default();
    run(&mut e, &format!("px={};", list(PX)));
    run(&mut e, "h=normplot(px);");
    assert_eq!(values(&mut e, "numel(h)"), [2.]);
    assert_eq!(
        values(&mut e, "get(h(1),'XData')"),
        [0.7, 1.5, 2.1, 2.8, 3.3]
    );
    all_close(
        &values(&mut e, "get(h(1),'YData')"),
        NORMPLOT_Y,
        1e-12,
        "normplot",
    );
    // The reference line passes through the quartiles (prctile rule).
    let q = values(&mut e, "prctile(px,[25 75])");
    let lx = values(&mut e, "get(h(2),'XData')");
    let ly = values(&mut e, "get(h(2),'YData')");
    let slope = (ly[1] - ly[0]) / (lx[1] - lx[0]);
    let z = 0.6744897501960817;
    close(slope, 2. * z / (q[1] - q[0]), 1e-10, "slope");
    close(ly[0] + slope * (q[0] - lx[0]), -z, 1e-10, "through q1");
    run(&mut e, "h=wblplot(px);");
    all_close(
        &values(&mut e, "get(h(1),'YData')"),
        WBLPLOT_Y,
        1e-12,
        "wblplot",
    );
    assert_eq!(values(&mut e, "strcmp(get(gca,'XScale'),'log')"), [1.]);
    run(&mut e, "figure; h=probplot('exponential',px);");
    let y = values(&mut e, "get(h(1),'YData')");
    close(y[0], -(1f64 - 0.1).ln(), 1e-12, "exponential");
    // probplot(ax, pd) adds the fitted line to the current probability plot.
    run(
        &mut e,
        "figure; probplot(px); n0=numel(get(gca,'Children')); pd=fitdist(px','Normal'); h=probplot(gca,pd); n1=numel(get(gca,'Children'));",
    );
    assert_eq!(values(&mut e, "n1-n0"), [1.]);
    let x = values(&mut e, "get(h,'XData')");
    let y = values(&mut e, "get(h,'YData')");
    let want = values(
        &mut e,
        &format!("norminv(normcdf({},pd.mu,pd.sigma))", x[50]),
    )[0];
    close(y[50], want, 1e-9, "fitted line");
    run(&mut e, "figure; h=probplot(fitdist(px','Weibull'),px);");
    assert_eq!(values(&mut e, "numel(h)"), [2.]);
    // qqplot(x): normal quantiles against the sorted sample.
    run(&mut e, "figure; h=qqplot(px);");
    assert_eq!(values(&mut e, "numel(h)"), [3.]);
    all_close(
        &values(&mut e, "get(h(1),'XData')"),
        NORMPLOT_Y,
        1e-12,
        "qq x",
    );
    assert_eq!(
        values(&mut e, "get(h(1),'YData')"),
        [0.7, 1.5, 2.1, 2.8, 3.3]
    );
    run(
        &mut e,
        "pd=makedist('Exponential','mu',2); h=qqplot(px,pd);",
    );
    let want: Vec<f64> = (0..5)
        .map(|i| -2. * (1. - (i as f64 + 0.5) / 5.).ln())
        .collect();
    all_close(&values(&mut e, "get(h(1),'XData')"), &want, 1e-12, "qq pd");
    run(&mut e, "h=qqplot(px,[5 4 3 2 1]);");
    assert_eq!(
        values(&mut e, "get(h(1),'XData')"),
        [0.7, 1.5, 2.1, 2.8, 3.3]
    );
    assert_eq!(values(&mut e, "get(h(1),'YData')"), [1., 2., 3., 4., 5.]);
    run(&mut e, "h=qqplot(px,1:10);");
    // The larger sample is interpolated at the smaller one's positions.
    assert_eq!(
        values(&mut e, "get(h(1),'YData')"),
        [1.5, 3.5, 5.5, 7.5, 9.5]
    );
    // Statement forms draw without assigning ans.
    run(
        &mut e,
        "clear; figure; normplot([1 2 3 4]); qqplot([1 2 3]); probplot('weibull',[1 2 3]);",
    );
    assert!(!e.workspace().contains_key("ans"));
}

#[test]
fn display_shows_each_parameter_with_its_confidence_interval() {
    let mut e = engine();
    let text = e
        .execute("pd=fitdist(real,'Normal')")
        .unwrap()
        .output
        .join("\n");
    assert!(
        text.contains("Normal distribution fitted to 30 observations"),
        "{text}"
    );
    assert!(text.contains("mu = 6.32363"), "{text}");
    assert!(text.contains("[5.38835, 7.25892]"), "{text}");
    assert!(text.contains("[1.99479, 3.36715]"), "{text}");
    let text = e
        .execute("pd=fitdist(bino,'Binomial','NTrials',10)")
        .unwrap()
        .output
        .join("\n");
    assert!(
        text.contains("N = 10") && text.contains("(fixed)"),
        "{text}"
    );
    // makedist objects keep the plain parameter listing.
    let text = e
        .execute("pd=makedist('Logistic')")
        .unwrap()
        .output
        .join("\n");
    assert!(
        text.contains("mu: 0") && text.contains("sigma: 1"),
        "{text}"
    );
    let text = e
        .execute("c={fitdist(pos,'Gamma')}")
        .unwrap()
        .output
        .join("\n");
    assert!(text.contains("prob.GammaDistribution"), "{text}");
}

#[test]
fn errors_name_the_problem() {
    let mut e = engine();
    for (code, part) in [
        ("fitdist([-1 2 3],'Weibull')", "positive"),
        ("fitdist([1 2.5 3],'Poisson')", "integer counts"),
        ("fitdist(5,'Normal')", "at least two"),
        ("fitdist([1 2 3],'uifigure')", "unknown distribution"),
        ("fitdist([1 2 3],'Uniform')", "not supported"),
        ("fitdist([1 2 3],'Normal','Theta',1)", "does not apply"),
        ("fitdist([1 2 3],'Normal','Bogus',1)", "unsupported option"),
        ("fitdist([1 2],'GeneralizedExtremeValue')", "distinct"),
        ("fitdist([1 2 1 2],'NegativeBinomial')", "overdispersion"),
        (
            "fitdist([1 2 3],'Poisson','Censoring',[0 1 0])",
            "censoring is not supported",
        ),
        (
            "fitdist([1 2 3],'Normal','Censoring',[0 2 0])",
            "right-censored",
        ),
        (
            "fitdist([1 2 3],'Normal','Frequency',[1 -1 1])",
            "nonnegative",
        ),
        (
            "fitdist([1 2 3],'Normal','Censoring',[1 1 1])",
            "uncensored",
        ),
        ("fitdist([0.5 1.5],'Beta')", "between 0 and 1"),
        (
            "fitdist([1 2 3],'Kernel','Support','positive','Censoring',[0 1 0])",
            "censoring",
        ),
        (
            "fitdist([1 2 3],'Kernel','Support',[1 5])",
            "inside the support",
        ),
        (
            "fitdist([1 2 3],'Kernel','Kernel','cosine')",
            "unknown kernel",
        ),
        ("fitdist([1 2 3],'Normal','Options',5)", "statset"),
        ("[a,b]=fitdist([1 2 3],'Normal')", "By"),
        ("paramci(fitdist([1 2 3],'Kernel'))", "no parameters"),
        (
            "paramci(fitdist([1 2 3],'Normal'),'Parameter','nu')",
            "unknown parameter",
        ),
        ("paramci(fitdist([1 2 3],'Normal'),'Alpha',2)", "alpha"),
        ("mle([1 2 3],'pdf',@(x,m) normpdf(x,m,1))", "Start"),
        (
            "mle([1 2 3],'pdf',@(x,m) normpdf(x,m,1),'Start',5,'LowerBound',6)",
            "within the bounds",
        ),
        ("mle([1 2 3],'distribution','binomial')", "NTrials"),
        ("statset('Bogus',1)", "unknown parameter"),
        ("gamfit([1 2;3 4])", "vector"),
        ("binofit(3,2)", "between 0 and n"),
        ("probplot('cauchy',[1 2 3])", "unknown distribution"),
        ("wblplot([-1 2 3])", "positive"),
        ("ecdf([1 2 3],'Function','pdf')", "Function"),
        ("qqplot([1 2],[3 4],[5 6])", "qqplot"),
        ("normlike([1 2 3],[1 2])", "2 parameters"),
    ] {
        let err = e.execute(code).unwrap_err();
        assert!(err.contains(part), "{code}: {err}");
    }
}

#[test]
fn new_builtins_are_registered() {
    for name in [
        "fitdist",
        "paramci",
        "negloglik",
        "mle",
        "statset",
        "normlike",
        "explike",
        "gamlike",
        "wbllike",
        "lognlike",
        "evlike",
        "betalike",
        "gamfit",
        "wblfit",
        "lognfit",
        "betafit",
        "raylfit",
        "evfit",
        "poissfit",
        "binofit",
        "nbinfit",
        "gevfit",
        "gpfit",
        "probplot",
        "normplot",
        "wblplot",
        "qqplot",
        "ecdf",
        "cdfplot",
    ] {
        assert!(matrust::session::FUNCTIONS.contains(&name), "{name}");
        assert!(matrust::help::summary(name).is_some(), "{name}");
    }
}

#[test]
#[ignore = "release-mode benchmark; run once with RAYON_NUM_THREADS=1 and N"]
fn benchmark_distribution_fits() {
    let workers = std::env::var("RAYON_NUM_THREADS")
        .unwrap_or_else(|_| "1".into())
        .parse()
        .unwrap();
    matrust::initialize_worker_pool(workers).unwrap();
    let mut e = Engine::default();
    run(
        &mut e,
        "rng(0); g=gamrnd(2,3,100000,1); r=normrnd(5,2,100000,1); t=trnd(4,100000,1); c=rand(100000,1)<0.2; s=sqrt((1+randn(10000,1)).^2+randn(10000,1).^2);",
    );
    for code in [
        "fitdist(g,'Gamma')",
        "fitdist(g,'Weibull','Censoring',c)",
        "fitdist(r,'GeneralizedExtremeValue')",
        "fitdist(t,'tLocationScale')",
        "fitdist(s,'Rician')",
        "fitdist(g,'Gamma','Censoring',c)",
        "pdf(fitdist(r,'Kernel'),linspace(0,10,1000))",
        "ecdf(g,'Censoring',c)",
    ] {
        let start = std::time::Instant::now();
        run(&mut e, &format!("q={code};"));
        eprintln!(
            "{code}: {:?} (RAYON_NUM_THREADS={workers})",
            start.elapsed()
        );
    }
}
