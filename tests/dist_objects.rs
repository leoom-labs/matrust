//! Oracle: bash tool/ref-python.sh tool/refs/stats/dist_objects.py (SciPy).
use matrust::{Array, Engine};
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    points: Vec<Point>,
    fits: Vec<Fit>,
    tree: Tree,
}
#[derive(Deserialize)]
struct Point {
    name: String,
    params: Vec<f64>,
    x: f64,
    pdf: f64,
    cdf: f64,
    icdf: f64,
    mean: f64,
    var: f64,
    std: f64,
    median: f64,
}
#[derive(Deserialize)]
struct Fit {
    x: Vec<f64>,
    normal: Vec<f64>,
    exponential: Vec<f64>,
    gamma: Vec<f64>,
    weibull: Vec<f64>,
    lognormal: Vec<f64>,
}
#[derive(Deserialize)]
struct Tree {
    z: Vec<Vec<f64>>,
    leaves: Vec<f64>,
    icoord: Vec<Vec<f64>>,
    dcoord: Vec<Vec<f64>>,
}
fn fixture() -> Fixture {
    serde_json::from_str(include_str!("fixtures/stats/dist_objects.json")).unwrap()
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
fn close(got: f64, want: f64, tol: f64) {
    assert!(
        (got - want).abs() <= tol * want.abs().max(1.),
        "got {got:.17e}, want {want:.17e}"
    );
}
fn rel_close(got: f64, want: f64, tol: f64) {
    assert!(
        got.to_bits() == want.to_bits() || (got - want).abs() <= tol * got.abs().max(want.abs()),
        "got {got:.17e}, want {want:.17e}"
    );
}
fn list(v: &[f64]) -> String {
    v.iter()
        .map(|x| format!("{x:.17e}"))
        .collect::<Vec<_>>()
        .join(" ")
}
fn names(name: &str) -> &[&str] {
    match name {
        "Normal" | "Lognormal" | "ExtremeValue" => &["mu", "sigma"],
        "Uniform" => &["Lower", "Upper"],
        "Exponential" => &["mu"],
        "Gamma" | "Beta" => &["a", "b"],
        "Chisquare" | "T" => &["nu"],
        "F" => &["nu1", "nu2"],
        "Weibull" => &["A", "B"],
        "Rayleigh" => &["B"],
        "Binomial" => &["N", "p"],
        "Poisson" => &["lambda"],
        "Geometric" => &["p"],
        "NegativeBinomial" => &["R", "P"],
        _ => panic!(),
    }
}
fn construct(p: &Point) -> String {
    let mut s = format!("makedist('{}'", p.name);
    for (name, v) in names(&p.name).iter().zip(&p.params) {
        s.push_str(&format!(",'{name}',{v:.17e}"));
    }
    s + ")"
}
fn tree_code(t: &Tree) -> String {
    format!(
        "Z=[{}];",
        t.z.iter().map(|r| list(r)).collect::<Vec<_>>().join(";")
    )
}

#[test]
fn defaults_names_and_case_insensitive_parameters() {
    let mut e = Engine::default();
    e.execute("pd=makedist('nOrMaL');").unwrap();
    assert_eq!(values(&mut e, "pd.ParameterValues"), [0., 1.]);
    assert_eq!(values(&mut e, "strcmp(pd.DistributionName,'Normal')"), [1.]);
    assert_eq!(
        values(&mut e, "isequal(pd.ParameterNames,{'mu','sigma'})"),
        [1.]
    );
    e.execute("pd=makedist(\"NORMAL\",\"MU\",2,\"SIGMA\",3);")
        .unwrap();
    assert_eq!(values(&mut e, "[pd.mu pd.sigma]"), [2., 3.]);
    for p in fixture().points {
        e.execute(&format!("pd=makedist('{}');", p.name)).unwrap();
        assert_eq!(values(&mut e, "isobject(pd)"), [1.]);
    }
}
#[test]
fn class_names_and_display_are_distribution_specific() {
    let mut e = Engine::default();
    for p in fixture().points {
        e.execute(&format!("pd={};", construct(&p))).unwrap();
        let title = if p.name == "Chisquare" {
            "ChiSquare"
        } else {
            &p.name
        };
        assert_eq!(
            values(
                &mut e,
                &format!("strcmp(class(pd),'prob.{title}Distribution')")
            ),
            [1.]
        );
        assert_eq!(values(&mut e, "isa(pd,'handle')"), [0.]);
        assert_eq!(values(&mut e, "isstruct(pd)"), [0.]);
    }
    let text = e
        .execute("pd=makedist('Normal','mu',2,'sigma',3)")
        .unwrap()
        .output
        .join("");
    assert!(
        text.contains("Normal") && text.contains("mu: 2") && text.contains("sigma: 3"),
        "{text}"
    );
}
#[test]
fn densities_probabilities_and_quantiles_match_scipy_all_families() {
    let mut e = Engine::default();
    for p in fixture().points {
        e.execute(&format!("pd={};", construct(&p))).unwrap();
        for (expr, want) in [
            (format!("pdf(pd,{})", p.x), p.pdf),
            (format!("cdf(pd,{})", p.x), p.cdf),
            ("icdf(pd,.7)".into(), p.icdf),
        ] {
            close(values(&mut e, &expr)[0], want, 2e-12);
        }
        close(
            values(&mut e, &format!("cdf(pd,{},'upper')", p.x))[0],
            1. - p.cdf,
            2e-12,
        );
        assert_eq!(
            values(&mut e, &format!("size(pdf(pd,[{};{}]))", p.x, p.x)),
            [2., 1.]
        );
    }
}
#[test]
fn object_moments_match_scipy_all_families() {
    let mut e = Engine::default();
    for p in fixture().points {
        e.execute(&format!("pd={};", construct(&p))).unwrap();
        for (name, want) in [
            ("mean", p.mean),
            ("var", p.var),
            ("std", p.std),
            ("median", p.median),
        ] {
            close(values(&mut e, &format!("{name}(pd)"))[0], want, 3e-12);
        }
    }
    e.execute("pd=makedist('T','nu',1);").unwrap();
    assert!(values(&mut e, "mean(pd)")[0].is_nan());
}
#[test]
fn random_reuses_family_stream_and_supports_size_vector() {
    let mut e = Engine::default();
    for p in fixture().points {
        let prefix = match p.name.as_str() {
            "Normal" => "norm",
            "Uniform" => "unif",
            "Exponential" => "exp",
            "Gamma" => "gam",
            "Beta" => "beta",
            "Chisquare" => "chi2",
            "T" => "t",
            "F" => "f",
            "Lognormal" => "logn",
            "Weibull" => "wbl",
            "Rayleigh" => "rayl",
            "ExtremeValue" => "ev",
            "Binomial" => "bino",
            "Poisson" => "poiss",
            "Geometric" => "geo",
            "NegativeBinomial" => "nbin",
            _ => unreachable!(),
        };
        e.execute(&format!(
            "pd={}; rng(0); a=random(pd,2,3); rng(0); b={}rnd({},2,3);",
            construct(&p),
            prefix,
            p.params
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(",")
        ))
        .unwrap();
        assert_eq!(values(&mut e, "isequal(a,b)"), [1.], "{}", p.name);
        e.execute("rng(0); b=random(pd,[2 3]);").unwrap();
        assert_eq!(values(&mut e, "isequal(a,b)"), [1.]);
    }
    e.execute("pd=makedist('Normal','mu',2,'sigma',3); rng(0); a=random(pd,2,3); rng(0); b=normrnd(2,3,2,3);").unwrap();
    assert_eq!(values(&mut e, "isequal(a,b)"), [1.]);
}
fn check_fit(name: &str, extract: impl Fn(&Fit) -> &Vec<f64>, tolerance: f64) {
    let mut e = Engine::default();
    for f in fixture().fits {
        e.execute(&format!(
            "x=[{} NaN]; pd=fitdist(x,'{name}'); other=fitdist(x','{name}');",
            list(&f.x)
        ))
        .unwrap();
        let got = values(&mut e, "pd.ParameterValues");
        for (&got, &want) in got.iter().zip(extract(&f)) {
            close(got, want, tolerance);
        }
        assert_eq!(values(&mut e, "isequal(pd,other)"), [1.]);
    }
}
#[test]
fn fit_normal_matches_unbiased_scipy_sample() {
    check_fit("Normal", |f| &f.normal, 2e-13);
}
#[test]
fn fit_exponential_matches_scipy() {
    check_fit("Exponential", |f| &f.exponential, 2e-13);
}
#[test]
fn fit_gamma_matches_scipy() {
    check_fit("Gamma", |f| &f.gamma, 3e-11);
}
#[test]
fn fit_weibull_matches_scipy_score_root() {
    check_fit("Weibull", |f| &f.weibull, 3e-11);
}
#[test]
fn fit_lognormal_matches_unbiased_log_sample() {
    check_fit("Lognormal", |f| &f.lognormal, 2e-13);
}
#[test]
fn fits_poisson_and_binomial_counts() {
    let mut e = Engine::default();
    e.execute(
        "pd=fitdist([0 1 2 3 NaN],'Poisson'); b=fitdist([0;1;2;3;NaN],\"Binomial\",\"NTrials\",5);",
    )
    .unwrap();
    assert_eq!(values(&mut e, "pd.lambda"), [1.5]);
    assert_eq!(values(&mut e, "b.ParameterValues"), [5., 0.3]);
}
#[test]
fn invalid_names_parameters_samples_and_options_are_clear_errors() {
    let mut e = Engine::default();
    for (code, part) in [
        ("makedist('uifigure')", "Supported"),
        ("makedist('Normal','sigma',-1)", "invalid"),
        ("makedist('Normal','sigma')", "pairs"),
        ("makedist('Normal','bogus',1)", "unknown"),
        ("makedist('Gamma','a',[1 2])", "scalar"),
        ("makedist('Poisson','lambda',Inf)", "finite"),
        ("fitdist([NaN NaN],'Normal')", "no observations"),
        ("fitdist([1 2;3 4],'Normal')", "vector"),
        ("fitdist([1 Inf],'Normal')", "finite"),
        ("fitdist([-1 1],'Exponential')", "nonnegative"),
        ("fitdist([0 1],'Gamma')", "positive"),
        ("fitdist([1 1],'Weibull')", "constant"),
        ("fitdist([1 2],'Uniform')", "not supported"),
        (
            "fitdist([1 2],'Normal','Bogus',[0 0])",
            "unsupported option",
        ),
        ("fitdist([1 2],'Binomial')", "NTrials"),
        ("fitdist([1 6],'Binomial','NTrials',5)", "integer counts"),
        ("fitdist([1 1.5],'Poisson')", "integer counts"),
    ] {
        let err = e.execute(code).unwrap_err();
        assert!(err.contains(part), "{code}: {err}");
    }
}
#[test]
fn objects_are_immutable_values_and_survive_cells_and_function_arguments() {
    let mut e = Engine::default();
    e.execute("pd=makedist('Normal'); q=pd; c={pd}; r=c{1};")
        .unwrap();
    assert_eq!(values(&mut e, "isequal(q,r)"), [1.]);
    assert!(e.execute("q.mu=10;").unwrap_err().contains("read-only"));
    assert_eq!(values(&mut e, "pd.mu"), [0.]);
    close(
        values(&mut e, "pdf(makedist('Normal'),0)")[0],
        0.3989422804014327,
        1e-15,
    );
    assert_eq!(values(&mut e, "pdf(r,0)==pdf(pd,0)"), [1.]);
    assert_eq!(values(&mut e, "r.pdf(0)==pdf(pd,0)"), [1.]);
    assert!(
        e.execute("parse(pd,1)")
            .unwrap_err()
            .contains("unsupported operation")
    );
    let text = e.execute("c").unwrap().output.join("");
    assert!(text.contains("prob.NormalDistribution"), "{text}");
    e.execute("f=@(p) mean(p); a=f(pd);").unwrap();
    assert_eq!(values(&mut e, "a"), [0.]);
}
#[test]
fn dendrogram_leaf_order_heights_and_lines_match_scipy() {
    let t = fixture().tree;
    let mut e = Engine::default();
    e.execute(&(tree_code(&t) + "[H,T,perm]=dendrogram(Z,0);"))
        .unwrap();
    assert_eq!(values(&mut e, "perm"), t.leaves);
    assert_eq!(
        values(&mut e, "T"),
        (1..=t.z.len() + 1).map(|v| v as f64).collect::<Vec<_>>()
    );
    assert_eq!(values(&mut e, "numel(H)"), [t.z.len() as f64]);
    assert_eq!(
        values(&mut e, "numel(get(gca,'Children'))"),
        [t.z.len() as f64]
    );
    assert_eq!(
        values(&mut e, "get(gca,'XTick')"),
        (1..=t.leaves.len()).map(|i| i as f64).collect::<Vec<_>>()
    );
    let axes = &e.figure.as_ref().unwrap().axes;
    let mut got: Vec<Vec<f64>> = axes
        .series
        .iter()
        .map(|s| s.points.iter().map(|p| p[1]).collect())
        .collect();
    let mut want = t.dcoord;
    got.sort_by(|a, b| a[1].total_cmp(&b[1]));
    want.sort_by(|a, b| a[1].total_cmp(&b[1]));
    assert_eq!(got, want);
    let mut gx: Vec<Vec<f64>> = axes
        .series
        .iter()
        .map(|s| s.points.iter().map(|p| p[0]).collect())
        .collect();
    let mut wx: Vec<Vec<f64>> = t
        .icoord
        .into_iter()
        .map(|r| r.into_iter().map(|x| (x + 5.) / 10.).collect())
        .collect();
    gx.sort_by(|a, b| a[0].total_cmp(&b[0]));
    wx.sort_by(|a, b| a[0].total_cmp(&b[0]));
    assert_eq!(gx, wx);
    let svg = axes.svg();
    let tick = regex::Regex::new(
        r#"<text x="[^"]*" y="[^"]*" text-anchor="middle" font-size="12"[^>]*>([^<]*)</text>"#,
    )
    .unwrap();
    let labels: Vec<_> = tick.captures_iter(&svg).map(|m| m[1].to_string()).collect();
    assert_eq!(
        labels,
        t.leaves.iter().map(|v| v.to_string()).collect::<Vec<_>>()
    );
    assert_eq!(
        axes.extras.leaf_ticks.as_ref().unwrap().as_ref(),
        &t.leaves.iter().map(|v| v.to_string()).collect::<Vec<_>>()
    );
}
#[test]
fn dendrogram_collapses_branches_and_uses_default_thirty_leaf_limit() {
    let mut e = Engine::default();
    e.execute("Z=zeros(34,3); Z(1,:)=[1 2 1]; for k=2:34; Z(k,:)=[k+1 34+k k]; end; [H,T,perm]=dendrogram(Z);").unwrap();
    assert_eq!(values(&mut e, "numel(H)"), [29.]);
    assert_eq!(values(&mut e, "numel(unique(T))"), [30.]);
    assert_eq!(values(&mut e, "numel(perm)"), [30.]);
    e.execute("[H,T,perm]=dendrogram(Z,0);").unwrap();
    assert_eq!(values(&mut e, "numel(H)"), [34.]);
    let t = fixture().tree;
    e.execute(&(tree_code(&t) + "[H,T,perm]=dendrogram(Z,3);"))
        .unwrap();
    assert_eq!(values(&mut e, "numel(H)"), [2.]);
    assert_eq!(values(&mut e, "numel(unique(T))"), [3.]);
    e.execute("[H,T,perm]=dendrogram(Z,1);").unwrap();
    assert_eq!(values(&mut e, "numel(H)"), [0.]);
    assert_eq!(values(&mut e, "perm"), [1.]);
}
#[test]
fn review_1111_lognormal_variance_stays_finite_for_large_sigma() {
    // mpmath 60 digits: (exp(σ²)-1)*exp(2μ+σ²).
    // μ=-900, σ=30 → 1 - exp(-900), which prints as 1.0 (exp(-900) ~ 1e-391).
    // μ=-2000, σ=30 → std 1.8882952750042941876e-478, which underflows to 0.
    // The old product expm1(σ²)*mean² was Inf (μ=-900) or NaN (μ=-2000).
    let mut e = Engine::default();
    assert_eq!(
        values(&mut e, "std(makedist('Lognormal','mu',-900,'sigma',30))")[0],
        1.0
    );
    assert_eq!(
        values(&mut e, "var(makedist('Lognormal','mu',-900,'sigma',30))")[0],
        1.0
    );
    assert_eq!(
        values(&mut e, "std(makedist('Lognormal','mu',-2000,'sigma',30))")[0],
        0.0
    );
    assert_eq!(
        values(&mut e, "var(makedist('Lognormal','mu',-2000,'sigma',30))")[0],
        0.0
    );
    // Lognormal(0, 1): exp(1)*(exp(1)-1). SciPy/mpmath 4.670774270471605.
    rel_close(
        values(&mut e, "var(makedist('Lognormal','mu',0,'sigma',1))")[0],
        4.670774270471605,
        1e-12,
    );
}

#[test]
fn review_1111_beta_extreme_mean_and_variance() {
    // mpmath: mean a/(a+b), variance a*b/((a+b)^2*(a+b+1)).
    // a=b=1e200 → 0.5 and 1.25e-201. a=b=1e308 → 0.5 and 1.25e-309
    // (a+b overflows; the old mean was 0). a=b=1e-200 → 0.5 and 0.25
    // (a*b underflows; the old variance was NaN).
    let mut e = Engine::default();
    for (code, mean, var) in [
        ("makedist('Beta','a',1e200,'b',1e200)", 0.5, 1.25e-201),
        ("makedist('Beta','a',1e308,'b',1e308)", 0.5, 1.25e-309),
        ("makedist('Beta','a',1e-200,'b',1e-200)", 0.5, 0.25),
    ] {
        e.execute(&format!("pd={code};")).unwrap();
        rel_close(values(&mut e, "mean(pd)")[0], mean, 1e-12);
        rel_close(values(&mut e, "var(pd)")[0], var, 1e-12);
    }
    // SciPy fixture shape Beta(2, 3): variance 0.04. Beta(2, 5) mean is 2/7.
    rel_close(
        values(&mut e, "var(makedist('Beta','a',2,'b',3))")[0],
        0.04,
        1e-12,
    );
    rel_close(
        values(&mut e, "mean(makedist('Beta','a',2,'b',5))")[0],
        2. / 7.,
        1e-12,
    );
}

#[test]
fn review_1111_weibull_overflow_and_large_shape() {
    // mpmath 60 digits, scale A = 1.
    // B=0.005: Γ(201) and Γ(401) overflow, so the variance is +Inf, not NaN.
    // B=1e8: std = 1.2825498133863866899e-8.
    // sqrt(π²/6)/1e8 = 1.282549830161864e-8 is the leading term; the series
    // keeps the next powers of 1/B. Shapes 1..=1000 stay on libm::tgamma
    // (checksum of those std bit patterns). The direct value at B=1000 is
    // 1.28087574739601975e-3; mpmath's truth there is 1.2808757478713504e-3.
    let mut e = Engine::default();
    let tiny = values(&mut e, "var(makedist('Weibull','A',1,'B',0.005))")[0];
    assert!(tiny.is_infinite() && tiny.is_sign_positive(), "{tiny}");
    rel_close(
        values(&mut e, "std(makedist('Weibull','A',1,'B',1e8))")[0],
        1.2825498133863867e-8,
        1e-12,
    );
    rel_close(
        values(&mut e, "std(makedist('Weibull','A',1,'B',1e4))")[0],
        1.2823821100913088e-4,
        1e-12,
    );
    for (b, std) in [
        (1.0, 1.0),
        (10.0, 1.1445721940301061e-1),
        (100.0, 1.2661157774859839e-2),
        (1000.0, 1.2808757473960198e-3),
    ] {
        rel_close(
            values(&mut e, &format!("std(makedist('Weibull','A',1,'B',{b}))"))[0],
            std,
            1e-12,
        );
    }
    let mut bits = 0u64;
    for b in 1..=1000 {
        let s = values(&mut e, &format!("std(makedist('Weibull','A',1,'B',{b}))"))[0];
        bits = bits.wrapping_mul(131).wrapping_add(s.to_bits());
    }
    assert_eq!(bits, 3_644_195_178_702_580_677);
}

#[test]
fn review_s1113_weibull_tiny_scale_variance_stays_finite() {
    // mpmath, 80 digits, of the exact binary64 inputs A = 1e-200 and
    // B = 0.01: var = 7.886578673647731e-26. tgamma(201) overflows, so the
    // scale and the gamma terms have to meet in log space. Relative tolerance
    // 1e-12 covers libm lgamma (about 1e-13).
    let mut e = Engine::default();
    rel_close(
        values(&mut e, "var(makedist('Weibull','A',1e-200,'B',0.01))")[0],
        7.886578673647731e-26,
        1e-12,
    );
}

#[test]
fn dendrogram_rejects_bad_trees_and_options_before_drawing() {
    let mut e = Engine::default();
    for code in [
        "dendrogram([1 1 1])",
        "dendrogram([1 3 1])",
        "dendrogram([1 2 -1])",
        "dendrogram([1 2 NaN])",
        "dendrogram([1 2])",
        "dendrogram([1 2 1],-1)",
        "dendrogram([1 2 1],2.5)",
        "dendrogram([1 2 1],0,'Orientation','left')",
    ] {
        assert!(e.execute(code).is_err(), "{code}");
        assert!(e.figure.is_none(), "{code}");
    }
}

#[test]
#[ignore = "release-mode benchmark; run once with RAYON_NUM_THREADS=1 and N"]
fn benchmark_fit_objects_and_dendrogram() {
    let workers = std::env::var("RAYON_NUM_THREADS")
        .unwrap_or_else(|_| "1".into())
        .parse()
        .unwrap();
    matrust::initialize_worker_pool(workers).unwrap();
    let mut e = Engine::default();
    e.execute("rng(0); x=gamrnd(2,3,1,10000); Z=zeros(29,3); Z(1,:)=[1 2 1]; for k=2:29; Z(k,:)=[k+1 29+k k]; end;").unwrap();
    let start = std::time::Instant::now();
    for _ in 0..10 {
        e.execute(
            "g=fitdist(x,'Gamma'); w=fitdist(x,'Weibull'); y=pdf(g,x); [H,T,p]=dendrogram(Z);",
        )
        .unwrap();
    }
    eprintln!(
        "10 x (Gamma+Weibull fit/pdf, n=10000; dendrogram n=30): {:?}; RAYON_NUM_THREADS={:?}",
        start.elapsed(),
        std::env::var("RAYON_NUM_THREADS")
    );
}
