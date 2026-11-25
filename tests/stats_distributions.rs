//! Probability distribution functions against 50-digit mpmath values
//! (cross-checked with scipy.stats); regenerate the fixture with
//! `tool/ref-python.sh tool/refs/stats/distributions.py`.
//!
//! Tolerances (task brief): density and distribution function relative
//! error <= 1e-13 in the body and <= 1e-11 in the far tails (down to 1e-300,
//! upper flag); continuous inverses satisfy |cdf(inv(p)) - p| <= 4 eps
//! max(p, 1-p) and match the 50-digit quantile to its conditioning; discrete
//! inverses are exact integers.
use matrust::{Array, Engine};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct Fixture {
    source: BTreeMap<String, serde_json::Value>,
    pdf: Vec<Point>,
    cdf: Vec<Point>,
    inv: Vec<Quantile>,
    moments: Vec<Moments>,
    fits: Vec<Fit>,
}

#[derive(Deserialize)]
struct Point {
    f: String,
    x: String,
    q: Vec<String>,
    v: String,
    tail: bool,
    #[serde(default)]
    upper: bool,
}

#[derive(Deserialize)]
struct Quantile {
    f: String,
    p: String,
    q: Vec<String>,
    v: String,
    kappa: String,
}

#[derive(Deserialize)]
struct Moments {
    f: String,
    q: Vec<String>,
    mean: String,
    var: String,
    kurt: String,
}

#[derive(Deserialize)]
struct Fit {
    x: Vec<String>,
    alpha: String,
    muhat: String,
    sigmahat: String,
    muci: Vec<String>,
    sigmaci: Vec<String>,
    expci: Vec<String>,
}

fn fixture() -> Fixture {
    serde_json::from_str(include_str!("fixtures/stats/distributions.json")).unwrap()
}

fn num(s: &str) -> f64 {
    s.parse().unwrap()
}

/// A MATLAB literal that reads back as exactly `v`.
fn lit(v: f64) -> String {
    if v.is_nan() {
        "NaN".into()
    } else if v.is_infinite() {
        if v > 0. { "Inf".into() } else { "-Inf".into() }
    } else {
        format!("{v:e}")
    }
}

fn values(e: &mut Engine, code: &str) -> Vec<f64> {
    e.execute(&format!("zz = {code};"))
        .unwrap_or_else(|err| panic!("{code}: {err}"));
    match e.workspace().get("zz").expect("zz") {
        Array::Numeric(m) => m.data().to_vec(),
        Array::Logical(m) => m.data().iter().map(|&b| f64::from(b)).collect(),
        other => panic!("{code}: unexpected class {}", other.class()),
    }
}

fn value(e: &mut Engine, code: &str) -> f64 {
    let v = values(e, code);
    assert_eq!(v.len(), 1, "{code}");
    v[0]
}

fn size(e: &mut Engine, code: &str) -> Vec<f64> {
    values(e, &format!("size({code})"))
}

fn error(e: &mut Engine, code: &str) -> String {
    match e.execute(code) {
        Ok(_) => panic!("{code} should fail"),
        Err(err) => err.to_string(),
    }
}

fn args(x: f64, q: &[String]) -> String {
    let mut parts = vec![lit(x)];
    parts.extend(q.iter().map(|v| lit(num(v))));
    parts.join(", ")
}

fn relative(got: f64, want: f64) -> f64 {
    if got == want || (got.is_nan() && want.is_nan()) {
        0.
    } else if want == 0. || !want.is_finite() {
        f64::INFINITY
    } else {
        (got - want).abs() / want.abs()
    }
}

/// Worst error per (family, kind), printed for the report.
#[derive(Default)]
struct Worst(BTreeMap<String, (f64, String)>);

impl Worst {
    fn note(&mut self, key: String, err: f64, case: String) {
        let slot = self.0.entry(key).or_insert((0., String::new()));
        if err > slot.0 {
            *slot = (err, case);
        }
    }
    fn print(&self, title: &str) {
        println!("{title}");
        for (k, (e, c)) in &self.0 {
            println!("  {k:<12} {e:.3e}  {c}");
        }
    }
}

#[test]
fn fixture_records_its_sources() {
    let f = fixture();
    assert!(f.source.contains_key("mpmath") && f.source.contains_key("scipy"));
    assert!(f.pdf.len() > 500 && f.cdf.len() > 1000 && f.inv.len() > 300);
}

#[test]
fn densities_match_mpmath() {
    let mut e = Engine::default();
    let mut worst = Worst::default();
    let mut failures = Vec::new();
    for c in fixture().pdf {
        let code = format!("{}pdf({})", c.f, args(num(&c.x), &c.q));
        let got = value(&mut e, &code);
        let want = num(&c.v);
        let err = relative(got, want);
        let tol = if c.tail { 1e-11 } else { 1e-13 };
        let kind = if c.tail { "tail" } else { "body" };
        worst.note(format!("{}pdf {kind}", c.f), err, code.clone());
        if err > tol {
            failures.push(format!("{code} = {got:e}, want {want:e} (rel {err:.2e})"));
        }
    }
    worst.print("worst relative pdf error");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn distribution_functions_match_mpmath_in_both_tails() {
    let mut e = Engine::default();
    let mut worst = Worst::default();
    let mut failures = Vec::new();
    for c in fixture().cdf {
        let flag = if c.upper { ", 'upper'" } else { "" };
        let code = format!("{}cdf({}{flag})", c.f, args(num(&c.x), &c.q));
        let got = value(&mut e, &code);
        let want = num(&c.v);
        let err = relative(got, want);
        let tol = if c.tail { 1e-11 } else { 1e-13 };
        let kind = if c.tail { "tail" } else { "body" };
        worst.note(format!("{}cdf {kind}", c.f), err, code.clone());
        if err > tol {
            failures.push(format!("{code} = {got:e}, want {want:e} (rel {err:.2e})"));
        }
    }
    worst.print("worst relative cdf error");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn inverses_meet_the_residual_bound_and_the_reference_quantile() {
    const DISCRETE: &[&str] = &["bino", "poiss", "geo", "nbin"];
    let eps = f64::EPSILON;
    let mut e = Engine::default();
    let mut worst = Worst::default();
    let mut failures = Vec::new();
    for c in fixture().inv {
        let p = num(&c.p);
        let code = format!("{}inv({})", c.f, args(p, &c.q));
        let got = value(&mut e, &code);
        let want = num(&c.v);
        if DISCRETE.contains(&c.f.as_str()) {
            if got != want {
                failures.push(format!("{code} = {got}, want {want}"));
            }
            continue;
        }
        // The task's bound, with this engine's own cdf.
        let back = value(&mut e, &format!("{}cdf({})", c.f, args(got, &c.q)));
        // Where the doubles around x are coarser than 4 eps in probability
        // (cdf(next double) - cdf(x) = pdf(x) ulp(x)), no x can do better
        // than that step; it is added to the bound.
        let density = value(&mut e, &format!("{}pdf({})", c.f, args(got, &c.q)));
        let grid = density * 2. * eps * got.abs();
        let residual = ((back - p).abs() - grid).max(0.) / p.max(1. - p);
        worst.note(format!("{}inv resid", c.f), residual / eps, code.clone());
        if residual > 4. * eps && !grid.is_infinite() {
            failures.push(format!(
                "{code}: |cdf(x) - p| = {:.2} eps * max(p, 1-p)",
                residual / eps
            ));
        }
        // Agreement with the 50-digit quantile, allowing for its conditioning
        // (kappa = min(p, 1-p) / |x f(x)|): a relative error d of the cdf
        // moves x by d kappa, so the bound is the cdf target (1e-13 body,
        // 1e-11 far tail) plus 8 eps, times kappa.
        let kappa = num(&c.kappa);
        let err = relative(got, want);
        worst.note(format!("{}inv x", c.f), err, code.clone());
        let target = if p.min(1. - p) < 1e-15 { 1e-11 } else { 1e-13 };
        let tol = ((8. * eps + target) * kappa).max(4e-15);
        if err > tol {
            failures.push(format!(
                "{code} = {got:e}, want {want:e} (rel {err:.2e}, kappa {kappa:.2e})"
            ));
        }
    }
    worst.print("worst inverse error (resid in eps units; x relative)");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn edge_cases_and_nan_rules() {
    let mut e = Engine::default();
    let nan = |e: &mut Engine, code: &str| {
        let v = value(e, code);
        assert!(v.is_nan(), "{code} = {v}, expected NaN");
    };
    // NaN inputs propagate.
    for code in [
        "normpdf(NaN)",
        "normcdf(1, NaN, 1)",
        "gaminv(NaN, 2)",
        "binopdf(NaN, 3, 0.5)",
        "poisscdf(2, NaN)",
        "tcdf(1, NaN)",
    ] {
        nan(&mut e, code);
    }
    // Parameters outside the domain give NaN.
    for code in [
        "normpdf(0, 0, 0)",
        "normcdf(0, 0, -1)",
        "unifpdf(0.5, 1, 0)",
        "exppdf(1, 0)",
        "gampdf(1, 0, 1)",
        "gamcdf(1, 2, -1)",
        "betacdf(0.5, -1, 1)",
        "chi2pdf(1, -2)",
        "tpdf(0, 0)",
        "fcdf(1, 0, 3)",
        "lognpdf(1, 0, 0)",
        "wblpdf(1, -1, 2)",
        "raylcdf(1, 0)",
        "evpdf(0, 0, 0)",
        "binopdf(1, 2.5, 0.5)",
        "binocdf(1, 3, 1.5)",
        "binopdf(1, -1, 0.5)",
        "poisspdf(1, -1)",
        "geopdf(1, -0.1)",
        "geocdf(1, 1.5)",
        "nbinpdf(1, 0, 0.5)",
        "nbincdf(1, 2, 1.5)",
        "norminv(1.5)",
        "norminv(-0.1)",
        "binoinv(2, 10, 0.5)",
    ] {
        nan(&mut e, code);
    }
    // Ends of the inverses.
    assert_eq!(value(&mut e, "norminv(0)"), f64::NEG_INFINITY);
    assert_eq!(value(&mut e, "norminv(1)"), f64::INFINITY);
    assert_eq!(value(&mut e, "norminv(0.5, 3, 2)"), 3.);
    assert_eq!(value(&mut e, "expinv(1, 2)"), f64::INFINITY);
    assert_eq!(value(&mut e, "unifinv(1, 2, 5)"), 5.);
    assert_eq!(value(&mut e, "tinv(0.5, 3)"), 0.);
    assert_eq!(value(&mut e, "binoinv(1, 10, 0.3)"), 10.);
    assert_eq!(value(&mut e, "binoinv(0, 10, 0.3)"), 0.);
    assert_eq!(value(&mut e, "poissinv(1, 3)"), f64::INFINITY);
    assert_eq!(value(&mut e, "geoinv(0, 0.3)"), 0.);
    assert_eq!(value(&mut e, "nbininv(1, 3, 0.5)"), f64::INFINITY);
    // A degenerate law has cdf(0) = 1: the smallest x with cdf(x) >= 1 is 0.
    assert_eq!(value(&mut e, "binoinv(1, 10, 0)"), 0.);
    assert_eq!(value(&mut e, "binoinv(1, 0, 0.3)"), 0.);
    assert_eq!(value(&mut e, "poissinv(1, 0)"), 0.);
    assert_eq!(value(&mut e, "geoinv(1, 1)"), 0.);
    assert_eq!(value(&mut e, "nbininv(1, 3, 1)"), 0.);
    assert_eq!(value(&mut e, "binoinv(1, 10, 1)"), 10.);
    // n = 0 and p in {0, 1} for the binomial.
    assert_eq!(value(&mut e, "binopdf(0, 0, 0.3)"), 1.);
    assert_eq!(value(&mut e, "binopdf(1, 0, 0.3)"), 0.);
    assert_eq!(value(&mut e, "binocdf(0, 0, 0.3)"), 1.);
    assert_eq!(value(&mut e, "binoinv(0.5, 0, 0.3)"), 0.);
    assert_eq!(values(&mut e, "binopdf(0:3, 3, 0)"), [1., 0., 0., 0.]);
    assert_eq!(values(&mut e, "binopdf(0:3, 3, 1)"), [0., 0., 0., 1.]);
    assert_eq!(values(&mut e, "binocdf(0:3, 3, 1)"), [0., 0., 0., 1.]);
    // Integer steps: the cdf is constant between integers and exactly 1 at n.
    assert_eq!(
        value(&mut e, "binocdf(2.75, 10, 0.3)"),
        value(&mut e, "binocdf(2, 10, 0.3)")
    );
    assert_eq!(value(&mut e, "binocdf(10, 10, 0.3)"), 1.);
    assert_eq!(value(&mut e, "binocdf(10, 10, 0.3, 'upper')"), 0.);
    assert_eq!(value(&mut e, "binocdf(-0.5, 10, 0.3)"), 0.);
    assert_eq!(value(&mut e, "binopdf(2.5, 10, 0.3)"), 0.);
    assert_eq!(value(&mut e, "poisspdf(1.5, 2)"), 0.);
    assert_eq!(value(&mut e, "poisscdf(Inf, 2)"), 1.);
    assert_eq!(
        value(&mut e, "poisscdf(3.999, 2.5)"),
        value(&mut e, "poisscdf(3, 2.5)")
    );
    // Densities at the ends of the support.
    assert_eq!(value(&mut e, "gampdf(0, 0.5, 1)"), f64::INFINITY);
    assert_eq!(value(&mut e, "gampdf(0, 1, 4)"), 0.25);
    assert_eq!(value(&mut e, "gampdf(0, 3, 1)"), 0.);
    assert_eq!(value(&mut e, "betapdf(0, 1, 3)"), 3.);
    assert_eq!(value(&mut e, "betapdf(1, 0.5, 0.5)"), f64::INFINITY);
    assert_eq!(value(&mut e, "betapdf(1.5, 2, 2)"), 0.);
    assert_eq!(value(&mut e, "exppdf(-1)"), 0.);
    assert_eq!(value(&mut e, "wblpdf(0, 2, 1)"), 0.5);
    assert_eq!(value(&mut e, "fpdf(0, 2, 7)"), 1.);
    assert_eq!(value(&mut e, "lognpdf(0)"), 0.);
    assert_eq!(values(&mut e, "unifpdf([0 0.5 1 1.5])"), [1., 1., 1., 0.]);
    // Student t: nu = 1 is the Cauchy law, nu = Inf the normal.
    let cauchy = value(&mut e, "tcdf(-3, 1)");
    assert!(relative(cauchy, 0.5 - (3f64).atan() / std::f64::consts::PI) < 1e-15);
    assert_eq!(
        value(&mut e, "tcdf(1.5, Inf)"),
        value(&mut e, "normcdf(1.5)")
    );
    assert_eq!(
        value(&mut e, "tpdf(1.5, Inf)"),
        value(&mut e, "normpdf(1.5)")
    );
    // Defaults.
    assert_eq!(
        value(&mut e, "normpdf(1)"),
        value(&mut e, "normpdf(1, 0, 1)")
    );
    assert_eq!(
        value(&mut e, "normpdf(1, 2)"),
        value(&mut e, "normpdf(1, 2, 1)")
    );
    assert_eq!(value(&mut e, "expcdf(3)"), value(&mut e, "expcdf(3, 1)"));
    assert_eq!(
        value(&mut e, "gamcdf(3, 2)"),
        value(&mut e, "gamcdf(3, 2, 1)")
    );
    assert_eq!(value(&mut e, "wblcdf(2)"), value(&mut e, "expcdf(2)"));
    assert_eq!(value(&mut e, "evcdf(0)"), -(-1f64).exp_m1());
    // Logical input is numeric.
    assert_eq!(value(&mut e, "normpdf(true)"), value(&mut e, "normpdf(1)"));
}

/// Review rev2: overflow in a density, quantiles beyond the largest double,
/// and success probability 0 of the geometric and negative binomial laws.
#[test]
fn overflow_edges_and_zero_success_probability() {
    let mut e = Engine::default();
    // Densities whose intermediate overflows or underflows are 0 (or the
    // small true value), never NaN.
    assert_eq!(value(&mut e, "wblpdf(1e150, 1, 3)"), 0.);
    // n1 x overflows: the cdf is 1, not the central 0.5 (rev3 finding).
    for code in [
        "fcdf(1e305, 1e4, 1e4)",
        "fcdf(2, 1e308, 1e308)",
        "fcdf(1e10, 1e300, 1e300)",
    ] {
        assert_eq!(value(&mut e, code), 1., "{code}");
    }
    assert_eq!(value(&mut e, "wblpdf(1e200, 1, 3)"), 0.);
    assert_eq!(value(&mut e, "wblpdf(1e300, 1, 0.5)"), 0.);
    assert_eq!(value(&mut e, "raylpdf(1, 1e-310)"), 0.);
    assert_eq!(value(&mut e, "lognpdf(1e-300, 0, 1e-30)"), 0.);
    // b z^(b-1) e^(-z^b) with z^b = 1e-330 below the doubles (mpmath, 40 digits).
    let tiny = value(&mut e, "wblpdf(1e-300, 1, 1.1)");
    assert!(
        relative(tiny, 1.099_999_999_999_932_7e-30) < 1e-14,
        "{tiny:e}"
    );
    // A quantile beyond the largest double is +-Inf, not +-realmax.
    for (code, want) in [
        ("norminv(0.99, 0, 1e308)", f64::INFINITY),
        ("norminv(0.01, 0, 1e308)", f64::NEG_INFINITY),
        ("tinv(1e-300, 0.5)", f64::NEG_INFINITY),
        ("tinv(1 - 1e-16, 0.01)", f64::INFINITY),
        ("logninv(0.99, 800, 1)", f64::INFINITY),
        ("expinv(0.99, 1e308)", f64::INFINITY),
        ("gaminv(0.99, 1, 1e308)", f64::INFINITY),
        ("wblinv(0.99, 1e308, 1)", f64::INFINITY),
        ("raylinv(0.99, 1e308)", f64::INFINITY),
        ("evinv(0.01, 0, 1e308)", f64::NEG_INFINITY),
    ] {
        assert_eq!(value(&mut e, code), want, "{code}");
    }
    // ... while one just inside stays finite (mpmath: 1e308 Phi^-1(0.9)).
    let inside = value(&mut e, "norminv(0.9, 0, 1e308)");
    assert!(
        relative(inside, 1.281_551_565_544_600_5e308) < 1e-15,
        "{inside:e}"
    );
    assert!(value(&mut e, "tinv(0.999, 0.5)").is_finite());
    // p = 0: success never comes, all the mass sits at +Inf.
    for (code, want) in [
        ("geopdf(2, 0)", 0.),
        ("geopdf(0, 0)", 0.),
        ("geocdf(2, 0)", 0.),
        ("geocdf(2, 0, 'upper')", 1.),
        ("geoinv(0.5, 0)", f64::INFINITY),
        ("geoinv(0, 0)", 0.),
        ("geornd(0)", f64::INFINITY),
        ("nbinpdf(1, 2, 0)", 0.),
        ("nbinpdf(0, 0.5, 0)", 0.),
        ("nbincdf(1, 2, 0)", 0.),
        ("nbincdf(100, 2, 0, 'upper')", 1.),
        ("nbininv(0.5, 2, 0)", f64::INFINITY),
        ("nbinrnd(2, 0)", f64::INFINITY),
        ("nbinrnd(0.5, 0)", f64::INFINITY),
    ] {
        assert_eq!(value(&mut e, code), want, "{code}");
    }
}

#[derive(Deserialize)]
struct LargeCase {
    #[serde(rename = "fn")]
    f: String,
    x: String,
    a: String,
    b: String,
    lower: String,
    upper: String,
    #[serde(default)]
    pdf: Option<String>,
}

/// Review rev2 #6: both shapes large (1e3 .. 1e9), where the continued
/// fraction loses about eps sqrt(a + b) near the mean; betacdf, betapdf,
/// fcdf with equal large degrees of freedom and binocdf with a huge n
/// against 40-digit mpmath quadrature (tool/refs/stats/large_shapes.py).
#[test]
fn large_shapes_match_mpmath() {
    #[derive(Deserialize)]
    struct Large {
        cases: Vec<LargeCase>,
    }
    let large: Large =
        serde_json::from_str(include_str!("fixtures/stats/beta_large_shapes.json")).unwrap();
    assert!(large.cases.len() >= 200);
    let mut e = Engine::default();
    let mut worst = Worst::default();
    for c in &large.cases {
        let call = format!(
            "{}cdf({}, {}, {})",
            c.f,
            lit(num(&c.x)),
            lit(num(&c.a)),
            lit(num(&c.b))
        );
        let lower = value(&mut e, &call);
        let upper = value(&mut e, &format!("{}, 'upper')", &call[..call.len() - 1]));
        for (got, want, side) in [(lower, &c.lower, "lower"), (upper, &c.upper, "upper")] {
            let err = relative(got, num(want));
            worst.note(format!("{} cdf", c.f), err, format!("{call} {side}"));
            assert!(err <= 1e-13, "{call} {side}: {got:e} vs {want} ({err:.2e})");
        }
        if let Some(want) = &c.pdf {
            let code = call.replacen("cdf", "pdf", 1);
            let got = value(&mut e, &code);
            let err = relative(got, num(want));
            worst.note(format!("{} pdf", c.f), err, code.clone());
            assert!(err <= 1e-13, "{code}: {got:e} vs {want} ({err:.2e})");
        }
    }
    worst.print("large shapes: worst relative error");
}

#[test]
fn scalar_expansion_and_size_errors() {
    let mut e = Engine::default();
    let grid = values(&mut e, "normpdf([0 1; 2 3], [0 1; 0 1], 2)");
    let single: Vec<f64> = ["(0,0,2)", "(2,0,2)", "(1,1,2)", "(3,1,2)"]
        .iter()
        .map(|a| value(&mut e, &format!("normpdf{a}")))
        .collect();
    assert_eq!(grid, single);
    assert_eq!(size(&mut e, "normcdf(1, [0 1 2], 1)"), [1., 3.]);
    assert_eq!(size(&mut e, "binopdf((0:4)', 4, 0.5)"), [5., 1.]);
    assert_eq!(size(&mut e, "gaminv(zeros(0, 3), 2)"), [0., 3.]);
    assert_eq!(size(&mut e, "normpdf([])"), [0., 0.]);
    for code in [
        "normpdf([1 2], [1 2 3])",
        "gamcdf([1 2 3], [1; 2; 3])",
        "binoinv([0.1 0.2], 10, [0.1 0.2 0.3])",
        "normrnd([0 1], 1, 1, 3)",
    ] {
        let msg = error(&mut e, code);
        assert!(msg.contains("size"), "{code}: {msg}");
    }
    let msg = error(&mut e, "betapdf(0.5, 2)");
    assert!(msg.contains("requires 2 distribution parameters"), "{msg}");
    let msg = error(&mut e, "normpdf(1, 2, 3, 4)");
    assert!(msg.contains("too many"), "{msg}");
    let msg = error(&mut e, "normcdf(1, 0, 1, 'lower')");
    assert!(msg.contains("'upper'"), "{msg}");
    let msg = error(&mut e, "normpdf(1+2i)");
    assert!(msg.contains("complex"), "{msg}");
}

#[test]
fn upper_flag_is_computed_in_the_upper_tail() {
    let mut e = Engine::default();
    // 1 - cdf would give 0 here.
    let upper = value(&mut e, "normcdf(30, 0, 1, 'upper')");
    assert!(relative(upper, 4.906713927148187e-198) < 1e-12, "{upper:e}");
    assert_eq!(
        value(&mut e, "normcdf(2, 'upper')"),
        value(&mut e, "normcdf(-2)")
    );
    assert_eq!(
        value(&mut e, "normcdf(2, 0, 1, \"upper\")"),
        value(&mut e, "normcdf(-2)")
    );
    for code in [
        "expcdf(700, 1, 'upper')",
        "gamcdf(650, 2, 1, 'upper')",
        "chi2cdf(1300, 3, 'upper')",
        "betacdf(1 - 1e-6, 30, 2, 'upper')",
        "tcdf(1e40, 2, 'upper')",
        "fcdf(1e60, 3, 4, 'upper')",
        "poisscdf(150, 3, 'upper')",
        "binocdf(400, 1000, 0.1, 'upper')",
        "wblcdf(700, 1, 1, 'upper')",
        "logncdf(exp(36), 0, 1, 'upper')",
        "evcdf(6.5, 0, 1, 'upper')",
        "raylcdf(37, 1, 'upper')",
        "geocdf(2000, 0.3, 'upper')",
        "nbincdf(500, 2, 0.4, 'upper')",
        "unifcdf(0.25, 0, 1, 'upper')",
    ] {
        let v = value(&mut e, code);
        assert!(v > 0. && v < 0.8, "{code} = {v:e}");
    }
}

/// Above 2^53 the doubles are more than 1 apart: the integer bisection of the
/// discrete inverses (and the draws that fall back to it) must stop at adjacent
/// doubles instead of looping on a midpoint that cannot move.
#[test]
fn discrete_inverses_with_huge_parameters_terminate() {
    let start = std::time::Instant::now();
    let mut e = Engine::default();
    let prev = |x: f64| f64::from_bits(x.to_bits() - 1);
    // Poisson median at lambda = 1e20 (about lambda + 1/3; the doubles near it
    // are 16384 apart, and a unit of x moves the cdf by only 4e-11): the answer
    // is the first double whose cdf reaches 0.5.
    let x = value(&mut e, "poissinv(0.5, 1e20)");
    assert!(x.is_finite() && (x - 1e20).abs() <= 1e5, "poissinv = {x:e}");
    let up = value(&mut e, &format!("poisscdf({}, 1e20)", lit(x)));
    let down = value(&mut e, &format!("poisscdf({}, 1e20)", lit(prev(x))));
    assert!(up >= 0.5 && down < 0.5, "cdf {up:e} at x, {down:e} below");
    // Geometric law with p = 1e-18 (also nbin with r = 1): the median is
    // ln(2)/p - 1 to within the spacing of the doubles (128).
    let want = (0.5f64.ln() / (-1e-18f64).ln_1p()).ceil() - 1.;
    for (inv, cdf) in [
        ("geoinv(0.5, 1e-18)", "geocdf({}, 1e-18)"),
        ("nbininv(0.5, 1, 1e-18)", "nbincdf({}, 1, 1e-18)"),
    ] {
        let x = value(&mut e, inv);
        assert!(relative(x, want) < 1e-12, "{inv} = {x:e}, want {want:e}");
        let below = value(&mut e, &cdf.replace("{}", &lit(x * (1. - 1e-9))));
        let above = value(&mut e, &cdf.replace("{}", &lit(x * (1. + 1e-9))));
        assert!(
            below < 0.5 && above > 0.5,
            "{inv}: cdf {below:e} / {above:e}"
        );
    }
    // The quantile of a huge law can be beyond the largest double: Inf.
    assert_eq!(value(&mut e, "geoinv(0.5, 5e-324)"), f64::INFINITY);
    assert!(value(&mut e, "geoinv(0.5, 1e-300)").is_finite());
    // Binomial with n = 9e15 (n is capped at 2^53): the median of an even n
    // at p = 1/2 is exactly n/2, found among 4.7e7-wide mass of neighbours.
    assert_eq!(value(&mut e, "binoinv(0.5, 9e15, 0.5)"), 4.5e15);
    // Draws: the walk from the mode cannot move, so they use the bisection.
    // 200 draws of poissrnd(1e20): mean 1e20, sd 1e10.
    e.execute("rng(0);").expect("rng");
    let draws = values(&mut e, "poissrnd(1e20, 1, 200)");
    assert_eq!(draws.len(), 200);
    let mean = draws.iter().sum::<f64>() / 200.;
    let sd = (draws.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / 199.).sqrt();
    assert!(draws.iter().all(|d| d.is_finite()));
    assert!(
        (mean - 1e20).abs() < 5. * 1e10 / 200f64.sqrt(),
        "mean {mean:e}"
    );
    assert!(sd > 0.8e10 && sd < 1.25e10, "sd {sd:e}");
    for code in ["nbinrnd(1, 1e-18, 1, 5)", "binornd(9e15, 0.5, 1, 5)"] {
        let d = values(&mut e, code);
        assert!(
            d.len() == 5 && d.iter().all(|v| v.is_finite() && *v >= 0.),
            "{code}"
        );
    }
    assert!(start.elapsed().as_secs() < 30, "took {:?}", start.elapsed());
}

#[test]
fn generic_names_match_the_family_functions() {
    let mut e = Engine::default();
    let pairs = [
        ("pdf('Normal', 1, 0, 2)", "normpdf(1, 0, 2)"),
        ("pdf('norm', 1)", "normpdf(1)"),
        ("pdf(\"Normal\", 1, 0, 2)", "normpdf(1, 0, 2)"),
        (
            "cdf('normal', 2, 0, 1, 'upper')",
            "normcdf(2, 0, 1, 'upper')",
        ),
        ("cdf('Exponential', 2, 3)", "expcdf(2, 3)"),
        ("cdf('GAMMA', 2, 3, 0.5)", "gamcdf(2, 3, 0.5)"),
        ("icdf('Beta', 0.3, 2, 5)", "betainv(0.3, 2, 5)"),
        ("icdf('Poisson', 0.5, 3)", "poissinv(0.5, 3)"),
        ("pdf('Binomial', 3, 10, 0.4)", "binopdf(3, 10, 0.4)"),
        ("cdf('Chisquare', 3, 4)", "chi2cdf(3, 4)"),
        ("cdf('chi2', 3, 4)", "chi2cdf(3, 4)"),
        ("pdf('T', 0.5, 3)", "tpdf(0.5, 3)"),
        ("pdf('F', 0.5, 3, 7)", "fpdf(0.5, 3, 7)"),
        ("pdf('Lognormal', 2, 0, 1)", "lognpdf(2, 0, 1)"),
        ("pdf('Weibull', 2, 1, 3)", "wblpdf(2, 1, 3)"),
        ("pdf('Rayleigh', 2, 1)", "raylpdf(2, 1)"),
        ("pdf('Extreme Value', 0.5, 0, 1)", "evpdf(0.5, 0, 1)"),
        ("pdf('ev', 0.5)", "evpdf(0.5)"),
        ("pdf('Geometric', 2, 0.3)", "geopdf(2, 0.3)"),
        ("pdf('Negative Binomial', 2, 3, 0.3)", "nbinpdf(2, 3, 0.3)"),
        ("pdf('nbin', 2, 3, 0.3)", "nbinpdf(2, 3, 0.3)"),
        ("pdf('Uniform', 0.5, 0, 2)", "unifpdf(0.5, 0, 2)"),
    ];
    for (generic, family) in pairs {
        assert_eq!(values(&mut e, generic), values(&mut e, family), "{generic}");
    }
    e.execute("rng(3); a = random('Normal', 1, 2, 2, 3); rng(3); b = normrnd(1, 2, 2, 3);")
        .unwrap();
    assert_eq!(value(&mut e, "isequal(a, b)"), 1.);
    assert_eq!(size(&mut e, "random('Exponential', 2, 1, 3)"), [1., 3.]);
    assert_eq!(size(&mut e, "random('Poisson', 4, [2 2])"), [2., 2.]);
    let msg = error(&mut e, "pdf('Cauchy', 1)");
    assert!(
        msg.contains("unknown distribution 'Cauchy'")
            && msg.contains("'Normal'")
            && msg.contains("'Poisson'"),
        "{msg}"
    );
    let msg = error(&mut e, "pdf(1, 2)");
    assert!(msg.contains("distribution name"), "{msg}");
}

#[test]
fn random_draws_are_reproducible_and_take_every_size_form() {
    let mut e = Engine::default();
    // normrnd uses the same stream as randn.
    e.execute("rng(0); a = normrnd(0, 1, 1, 5); rng(0); b = randn(1, 5);")
        .unwrap();
    assert_eq!(value(&mut e, "isequal(a, b)"), 1.);
    e.execute("rng(0); a = unifrnd(2, 4, 1, 5); rng(0); b = 2 + 2*rand(1, 5);")
        .unwrap();
    assert_eq!(value(&mut e, "max(abs(a - b))"), 0.);
    // Pinned sequences under rng(0) (this implementation's stream; MATLAB's
    // numbers differ except for normrnd/unifrnd, which share rand/randn).
    let pinned: &[(&str, &[f64])] = &[
        ("exprnd(2, 1, 4)", &PIN_EXP),
        ("gamrnd(2.5, 1.5, 1, 4)", &PIN_GAM),
        ("betarnd(2, 3, 1, 4)", &PIN_BETA),
        ("trnd(5, 1, 4)", &PIN_T),
        ("binornd(100, 0.3, 1, 4)", &PIN_BINO),
        ("poissrnd(4, 1, 4)", &PIN_POISS),
        ("geornd(0.2, 1, 4)", &PIN_GEO),
        ("nbinrnd(3, 0.4, 1, 4)", &PIN_NBIN),
        ("wblrnd(2, 1.5, 1, 4)", &PIN_WBL),
        ("evrnd(0, 1, 1, 4)", &PIN_EV),
    ];
    let mut wrong = Vec::new();
    for (code, want) in pinned {
        e.execute("rng(0);").unwrap();
        let got = values(&mut e, code);
        println!("{code}: {got:?}");
        let ok = got.len() == want.len()
            && got
                .iter()
                .zip(*want)
                .all(|(g, w)| relative(*g, *w) <= 1e-12);
        if !ok {
            wrong.push(format!("{code} = {got:?}, pinned {want:?}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    // Same seed, same numbers.
    e.execute("rng(5); a = gamrnd(0.3, 1, 3, 3); rng(5); b = gamrnd(0.3, 1, 3, 3);")
        .unwrap();
    assert_eq!(value(&mut e, "isequal(a, b)"), 1.);
    // Size forms.
    assert_eq!(size(&mut e, "normrnd(0, 1, 2, 3)"), [2., 3.]);
    assert_eq!(size(&mut e, "normrnd(0, 1, [2 3])"), [2., 3.]);
    assert_eq!(size(&mut e, "normrnd([0 1 2], 1)"), [1., 3.]);
    assert_eq!(size(&mut e, "normrnd([0 1 2], 1, 1, 3)"), [1., 3.]);
    assert_eq!(size(&mut e, "exprnd(2, 3)"), [3., 3.]);
    assert_eq!(size(&mut e, "chi2rnd(3, [4 1])"), [4., 1.]);
    assert_eq!(size(&mut e, "normrnd()"), [1., 1.]);
    assert_eq!(size(&mut e, "binornd(5, 0.5, 0, 3)"), [0., 3.]);
    // Each element uses its own parameters; invalid ones give NaN.
    let v = values(&mut e, "normrnd([0 1e6], [1 1e-6])");
    assert!(v[0].abs() < 10. && (v[1] - 1e6).abs() < 1e-3, "{v:?}");
    let v = values(&mut e, "poissrnd([3 -1 0])");
    assert!(v[0] >= 0. && v[1].is_nan() && v[2] == 0., "{v:?}");
    let v = values(&mut e, "binornd(10, [0 1])");
    assert_eq!(v, [0., 10.]);
    let v = values(&mut e, "geornd(1, 1, 3)");
    assert_eq!(v, [0., 0., 0.]);
}

const PIN_EXP: [f64; 4] = [
    0.4098125166541227,
    0.19789129867934677,
    4.127344013249187,
    0.18121562307093547,
];
const PIN_GAM: [f64; 4] = [
    4.58754627885813,
    4.0058427116937585,
    2.383579128902462,
    1.0877901657295075,
];
const PIN_BETA: [f64; 4] = [
    0.4331213217699411,
    0.5351443298718083,
    0.41010698502177095,
    0.2098015117913031,
];
const PIN_T: [f64; 4] = [
    0.34301872478157464,
    0.83418671124066,
    -0.416366146751949,
    5.142029495791799,
];
const PIN_BINO: [f64; 4] = [34.0, 36.0, 25.0, 36.0];
const PIN_POISS: [f64; 4] = [6.0, 7.0, 2.0, 7.0];
const PIN_GEO: [f64; 4] = [0.0, 0.0, 9.0, 0.0];
const PIN_NBIN: [f64; 4] = [7.0, 9.0, 1.0, 9.0];
const PIN_WBL: [f64; 4] = [
    0.6951312469642973,
    0.4278528845307061,
    3.2418315486345914,
    0.4034661365097726,
];
const PIN_EV: [f64; 4] = [
    -1.585202680879849,
    -2.3131845761702974,
    0.7244869234724508,
    -2.401214849611662,
];

#[test]
fn sample_moments_of_1e5_draws_are_within_5_standard_errors() {
    const N: f64 = 1e5;
    let mut e = Engine::default();
    e.execute("rng(0);").unwrap();
    let mut failures = Vec::new();
    for m in fixture().moments {
        let params: Vec<String> = m.q.iter().map(|v| lit(num(v))).collect();
        let code = format!("{}rnd({}, 1, 1e5)", m.f, params.join(", "));
        let draws = values(&mut e, &code);
        assert_eq!(draws.len(), N as usize);
        let mean = draws.iter().sum::<f64>() / N;
        let var = draws.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / (N - 1.);
        let (mu, sigma2, kurt) = (num(&m.mean), num(&m.var), num(&m.kurt));
        let se_mean = (sigma2 / N).sqrt();
        // Var(s^2) ~ sigma^4 (excess kurtosis + 2) / N.
        let se_var = sigma2 * ((kurt + 2.) / N).sqrt();
        let zm = (mean - mu) / se_mean;
        let zv = (var - sigma2) / se_var;
        println!("{code}: mean z {zm:+.2}, variance z {zv:+.2}");
        if zm.abs() > 5. || zv.abs() > 5. {
            failures.push(format!("{code}: mean z {zm:.2}, variance z {zv:.2}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn normfit_and_expfit_match_scipy() {
    let mut e = Engine::default();
    let close = |got: f64, want: &str, what: &str| {
        let want = num(want);
        assert!(relative(got, want) < 1e-12, "{what}: {got:e} vs {want:e}");
    };
    for f in fixture().fits {
        let data: Vec<String> = f.x.iter().map(|v| lit(num(v))).collect();
        let x = format!("[{}]", data.join(" "));
        let alpha = lit(num(&f.alpha));
        e.execute(&format!(
            "[mu, sg, muci, sgci] = normfit({x}', {alpha}); [emu, eci] = expfit({x}, {alpha});"
        ))
        .unwrap();
        close(value(&mut e, "mu"), &f.muhat, "muhat");
        close(value(&mut e, "sg"), &f.sigmahat, "sigmahat");
        close(value(&mut e, "emu"), &f.muhat, "expfit muhat");
        let muci = values(&mut e, "muci");
        let sgci = values(&mut e, "sgci");
        let eci = values(&mut e, "eci");
        assert_eq!(size(&mut e, "muci"), [2., 1.]);
        for i in 0..2 {
            close(muci[i], &f.muci[i], "muci");
            close(sgci[i], &f.sigmaci[i], "sigmaci");
            close(eci[i], &f.expci[i], "expci");
        }
    }
    // Default alpha and the column-wise matrix form.
    e.execute("x = [4.1 5.3 2.2 6.0 4.8 3.9 5.1 4.4]';")
        .unwrap();
    e.execute("[m1, s1, c1, d1] = normfit(x); [m2, s2, c2, d2] = normfit(x, 0.05);")
        .unwrap();
    assert_eq!(
        value(
            &mut e,
            "isequal(m1, m2) && isequal(c1, c2) && isequal(d1, d2)"
        ),
        1.
    );
    e.execute("[M, S, C, D] = normfit([x, 2*x + 1]); [ma, sa, ca, da] = normfit(2*x + 1);")
        .unwrap();
    assert_eq!(size(&mut e, "M"), [1., 2.]);
    assert_eq!(size(&mut e, "C"), [2., 2.]);
    assert_eq!(
        value(
            &mut e,
            "isequal(M(2), ma) && isequal(S(2), sa) && isequal(C(:, 2), ca) && isequal(D(:, 2), da)"
        ),
        1.
    );
    e.execute("[E, EC] = expfit([x, 2*x]); [eb, ebc] = expfit(2*x);")
        .unwrap();
    assert_eq!(
        value(&mut e, "isequal(E(2), eb) && isequal(EC(:, 2), ebc)"),
        1.
    );
    // A row vector is one sample.
    assert_eq!(value(&mut e, "isequal(normfit(x'), normfit(x))"), 1.);
    assert_eq!(value(&mut e, "normfit(5)"), 5.);
    let msg = error(&mut e, "normfit(x, 0.05, [], [])");
    assert!(msg.contains("not supported"), "{msg}");
    let msg = error(&mut e, "normfit(x, 2)");
    assert!(msg.contains("alpha"), "{msg}");
}

#[test]
fn every_new_name_is_registered() {
    let families = [
        "norm", "unif", "exp", "gam", "beta", "chi2", "t", "f", "logn", "wbl", "rayl", "ev",
        "bino", "poiss", "geo", "nbin",
    ];
    for fam in families {
        for op in ["pdf", "cdf", "inv", "rnd"] {
            let name = format!("{fam}{op}");
            assert!(
                matrust::session::FUNCTIONS.contains(&name.as_str()),
                "{name} missing from FUNCTIONS"
            );
        }
    }
    for name in ["pdf", "cdf", "icdf", "random", "normfit", "expfit"] {
        assert!(matrust::session::FUNCTIONS.contains(&name));
    }
}
