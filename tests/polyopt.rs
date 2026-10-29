//! conv/roots/poly/interp/optimisation/array utilities, reference values.
use matrust::{Array, Engine};

const TOL: f64 = 1e-12;

fn num(v: f64) -> String {
    if v.is_nan() {
        "NaN".into()
    } else if v.is_infinite() {
        if v > 0. { "Inf".into() } else { "-Inf".into() }
    } else {
        format!("{v:.12e}")
    }
}
fn show(a: &Array) -> String {
    match a {
        Array::Numeric(m) => format!(
            "double {}x{} {}",
            m.rows(),
            m.cols(),
            m.data()
                .iter()
                .map(|&v| num(v))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        Array::Logical(m) => format!(
            "logical {}x{} {}",
            m.rows(),
            m.cols(),
            m.data()
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        ),
        Array::Complex(m) => format!(
            "double {}x{} C {}",
            m.rows(),
            m.cols(),
            m.data()
                .iter()
                .map(|z| format!("{},{}", num(z.re), num(z.im)))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        Array::Character(c) => format!(
            "char {}x{} {}",
            c.rows(),
            c.cols(),
            c.units()
                .iter()
                .map(|u| u.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        ),
        _ => "other".into(),
    }
}
/// Run `expr` (optionally `N|expr` for N outputs) and describe the outputs.
fn run(e: &mut Engine, case: &str) -> String {
    let (n, expr) = match case.split_once('|') {
        Some((k, rest)) if k.len() <= 2 && k.chars().all(|c| c.is_ascii_digit()) => {
            (k.parse().unwrap(), rest)
        }
        _ => (1usize, case),
    };
    let names: Vec<String> = (1..=n).map(|i| format!("o{i}")).collect();
    let source = format!("[{}] = {};", names.join(","), expr);
    let source = if n == 1 {
        format!("o1 = {expr};")
    } else {
        source
    };
    match e.execute(&source) {
        Err(err) => format!("ERROR {err}"),
        Ok(_) => names
            .iter()
            .map(|nm| e.array(nm).map_or("missing".into(), |a| show(&a)))
            .collect::<Vec<_>>()
            .join(" | "),
    }
}

/// Exploration helper: POLYOPT_CASES=file cargo test --test polyopt dump_cases -- --ignored --nocapture
#[test]
#[ignore]
fn dump_cases() {
    let path = std::env::var("POLYOPT_CASES").unwrap();
    let dir = std::env::var("POLYOPT_DIR").ok();
    let mut e = Engine::default();
    if let Some(dir) = dir {
        e.set_function_directory(std::path::Path::new(&dir))
            .unwrap();
    }
    for line in std::fs::read_to_string(path).unwrap().lines() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        println!("{line} => {}", run(&mut e, line));
    }
}

/// Command-window output of help examples:
/// POLYOPT_SHOW=tool/polyopt/help_display.txt [POLYOPT_DIR=tool/polyopt/fns] cargo test --test polyopt show_examples -- --ignored --nocapture
#[test]
#[ignore]
fn show_examples() {
    let path = std::env::var("POLYOPT_SHOW").unwrap();
    let mut e = Engine::default();
    if let Ok(dir) = std::env::var("POLYOPT_DIR") {
        e.set_function_directory(std::path::Path::new(&dir))
            .unwrap();
    }
    for line in std::fs::read_to_string(path).unwrap().lines() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        println!(">> {line}");
        match e.execute(line) {
            Ok(out) => out.output.iter().for_each(|l| println!("{l}")),
            Err(err) => println!("ERROR {err}"),
        }
    }
}

fn real(e: &mut Engine, source: &str) -> Vec<f64> {
    e.execute(&format!("o = {source};"))
        .unwrap_or_else(|err| panic!("{source}: {err}"));
    e.variables["o"].data().to_vec()
}
fn shape(e: &Engine, name: &str) -> (usize, usize) {
    let a = e.array(name).unwrap();
    (a.rows(), a.cols())
}
fn close(got: &[f64], want: &[f64], tol: f64) {
    assert_eq!(got.len(), want.len(), "{got:?} vs {want:?}");
    for (g, w) in got.iter().zip(want) {
        assert!(
            (g.is_nan() && w.is_nan()) || g == w || (g - w).abs() <= tol * w.abs().max(1.),
            "{got:?} vs {want:?}"
        );
    }
}
fn error(e: &mut Engine, source: &str) -> String {
    e.execute(source).expect_err(source)
}

#[test]
fn conv_shapes_orientation_and_errors() {
    let mut e = Engine::default();
    close(&real(&mut e, "conv([1 2 3],[1 1])"), &[1., 3., 5., 3.], TOL);
    close(
        &real(&mut e, "conv([1 2 3 4 5],[1 1 1],'same')"),
        &[3., 6., 9., 12., 9.],
        TOL,
    );
    close(
        &real(&mut e, "conv([1 2 3 4 5],[1 1 1 1],'same')"),
        &[6., 10., 14., 12., 9.],
        TOL,
    );
    close(
        &real(&mut e, "conv([1 2 3 4 5],[1 1 1],'valid')"),
        &[6., 9., 12.],
        TOL,
    );
    assert_eq!(real(&mut e, "conv([1 2],[1 1 1],'valid')").len(), 0);
    e.execute("c = conv([1 2 3]',[1 1]);").unwrap();
    assert_eq!(shape(&e, "c"), (4, 1));
    e.execute("c = conv([1 2 3],[1 1]');").unwrap();
    assert_eq!(shape(&e, "c"), (1, 4));
    e.execute("z = conv([1+2i 3],[1 1i]);").unwrap();
    let Array::Complex(z) = e.array("z").unwrap() else {
        panic!("complex")
    };
    let got: Vec<(f64, f64)> = z.data().iter().map(|c| (c.re, c.im)).collect();
    assert_eq!(got, vec![(1., 2.), (1., 1.), (0., 3.)]);
    // conv: 'full' follows the longer input (the second on a tie); 'same'/'valid' the first.
    for (case, want) in [
        ("conv([1 0 1],[2;7;4])", (5, 1)),
        ("conv([1;0;1],[2 7 4])", (1, 5)),
        ("conv([1 1],[1 2 3]')", (4, 1)),
        ("conv([1;1],[1 2 3])", (1, 4)),
        ("conv(5,[1;2])", (2, 1)),
        ("conv([1;2;3],[1 1 1],'same')", (3, 1)),
        ("conv([1 2 3],[1;1;1],'same')", (1, 3)),
        ("conv([1;2;3;4],[1 1],'valid')", (3, 1)),
        ("conv([1 2 3 4],[1;1],'valid')", (1, 3)),
    ] {
        e.execute(&format!("c = {case};")).unwrap();
        assert_eq!(shape(&e, "c"), want, "{case}");
    }
    close(
        &real(&mut e, "conv([1 0 1],[2;7;4])"),
        &[2., 7., 6., 7., 4.],
        TOL,
    );
    assert!(error(&mut e, "conv([1 2;3 4],[1 1]);").contains("must be vectors"));
    assert!(error(&mut e, "conv([1 2]);").contains("Not enough input arguments"));
    assert!(error(&mut e, "conv([1 2],[1],'full','x');").contains("Too many input arguments"));
    assert!(error(&mut e, "conv([1 2],[1],'bogus');").contains("shape"));
}

#[test]
fn deconv_quotient_and_remainder() {
    let mut e = Engine::default();
    e.execute("[q,r] = deconv([1 5 7],[1 2]);").unwrap();
    close(e.variables["q"].data(), &[1., 3.], TOL);
    close(e.variables["r"].data(), &[0., 0., 1.], TOL);
    e.execute("[q,r] = deconv([1 2],[1 2 3]);").unwrap();
    close(e.variables["q"].data(), &[0.], TOL);
    close(e.variables["r"].data(), &[1., 2.], TOL);
    e.execute("q = deconv(conv([1 2 3],[1 4]),[1 4]);").unwrap();
    close(e.variables["q"].data(), &[1., 2., 3.], TOL);
    assert!(error(&mut e, "[q,r,s] = deconv([1 2],[1]);").contains("does not support"));
    // deconv: the quotient is a column when the dividend is not a row.
    e.execute("[q,r] = deconv([1;2;3],[1 2]); c = conv([1 2],q) + r;")
        .unwrap();
    assert_eq!(shape(&e, "q"), (2, 1));
    close(e.variables["q"].data(), &[1., 0.], TOL);
    close(e.variables["r"].data(), &[0., 0., 3.], TOL);
    close(e.variables["c"].data(), &[1., 2., 3.], TOL);
    assert_eq!(shape(&e, "c"), (3, 1));
    e.execute("q = deconv([1 2 3],[1 2]);").unwrap();
    assert_eq!(shape(&e, "q"), (1, 2));
    assert!(
        error(&mut e, "deconv([1 2 3],[0 1]);")
            .contains("First coefficient of A must be non-zero.")
    );
}

#[test]
fn roots_poly_polyder_polyint() {
    let mut e = Engine::default();
    close(&real(&mut e, "roots([1 -3 2])"), &[2., 1.], TOL);
    close(&real(&mut e, "roots([1 -6 11 -6])"), &[3., 2., 1.], 1e-12);
    close(
        &real(&mut e, "roots([0 0 1 -3 2 0 0])"),
        &[0., 0., 2., 1.],
        TOL,
    );
    e.execute("r = roots([1 2 3 4 5]);").unwrap();
    let Array::Complex(r) = e.array("r").unwrap() else {
        panic!("complex")
    };
    let want = [
        (0.2878154795576, 1.416093080172),
        (0.2878154795576, -1.416093080172),
        (-1.287815479558, 0.8578967583285),
        (-1.287815479558, -0.8578967583285),
    ];
    for (z, w) in r.data().iter().zip(want) {
        assert!(
            (z.re - w.0).abs() < 1e-11 && (z.im - w.1).abs() < 1e-11,
            "{z:?}"
        );
    }
    assert_eq!(real(&mut e, "roots([0 0])").len(), 0);
    assert!(error(&mut e, "roots([1 NaN]);").contains("NaN or Inf"));
    close(&real(&mut e, "poly([1 2 3])"), &[1., -6., 11., -6.], TOL);
    close(&real(&mut e, "poly([1+1i 1-1i])"), &[1., -2., 2.], TOL);
    close(&real(&mut e, "poly([1 2;3 4])"), &[1., -5., -2.], 1e-12);
    close(&real(&mut e, "poly([])"), &[1.], TOL);
    assert!(error(&mut e, "poly([1 2 3;4 5 6]);").contains("vector or a square matrix"));
    close(&real(&mut e, "polyder([1 2 3])"), &[2., 2.], TOL);
    close(&real(&mut e, "polyder([0 0 1 2 3])"), &[2., 2.], TOL);
    close(&real(&mut e, "polyder([1 2],[1 3])"), &[2., 5.], TOL);
    close(&real(&mut e, "polyder(5)"), &[0.], TOL);
    close(&real(&mut e, "polyint([3 2 1])"), &[1., 1., 1., 0.], TOL);
    close(&real(&mut e, "polyint([3 2 1],5)"), &[1., 1., 1., 5.], TOL);
}

#[test]
fn conv2_shapes_and_separable_form() {
    let mut e = Engine::default();
    close(
        &real(&mut e, "conv2([1 2;3 4],[1 1;1 1])"),
        &[1., 4., 3., 3., 10., 7., 2., 6., 4.],
        TOL,
    );
    close(
        &real(&mut e, "conv2([1 2 3;4 5 6;7 8 9],[1 0;0 -1],'same')"),
        &[4., 4., -7., 4., 4., -8., -3., -6., -9.],
        TOL,
    );
    close(
        &real(&mut e, "conv2([1 2 3;4 5 6;7 8 9],[1 0;0 -1],'valid')"),
        &[4., 4., 4., 4.],
        TOL,
    );
    close(
        &real(&mut e, "conv2([1 2 3;4 5 6;7 8 9],ones(3),'same')"),
        &[12., 27., 24., 21., 45., 39., 16., 33., 28.],
        TOL,
    );
    let full = real(&mut e, "conv2([1 2 3],[1;2],[1 2 3;4 5 6;7 8 9])");
    let kernel = real(&mut e, "conv2([1 2 3;4 5 6;7 8 9],[1 2;2 4;3 6])");
    close(&full, &kernel, 1e-12);
    assert!(error(&mut e, "conv2([1 2]);").contains("Not enough input arguments"));
}

#[test]
fn interp1_methods_and_extrapolation() {
    let mut e = Engine::default();
    let nan = f64::NAN;
    close(
        &real(&mut e, "interp1([1 2 3],[4 5 7],[0.5 1 1.5 2.5 3 3.5])"),
        &[nan, 4., 4.5, 6., 7., nan],
        TOL,
    );
    close(
        &real(&mut e, "interp1([1 2 3],[4 5 7],[0 4],'linear','extrap')"),
        &[3., 9.],
        TOL,
    );
    close(
        &real(&mut e, "interp1([1 2 3],[4 5 7],[0 4],'linear',0)"),
        &[0., 0.],
        TOL,
    );
    close(
        &real(&mut e, "interp1([1 2 3],[4 5 7],[1.5 2.5],'nearest')"),
        &[5., 7.],
        TOL,
    );
    close(
        &real(&mut e, "interp1([1 2 3],[4 5 7],[1.2 2.5 3.5],'next')"),
        &[5., 7., nan],
        TOL,
    );
    close(
        &real(&mut e, "interp1([1 2 3],[4 5 7],[0.5 1.2 2.5],'previous')"),
        &[nan, 4., 5.],
        TOL,
    );
    close(
        &real(
            &mut e,
            "interp1([1 2 3 4],[1 4 9 16],[1.5 2.5 3.5],'spline')",
        ),
        &[2.25, 6.25, 12.25],
        1e-13,
    );
    close(
        &real(
            &mut e,
            "interp1([1 2 3 4],[1 4 9 16],[0 5],'spline','extrap')",
        ),
        &[0., 25.],
        1e-12,
    );
    close(
        &real(&mut e, "interp1([3 1 2],[30 10 20],[1.5 2.5])"),
        &[15., 25.],
        TOL,
    );
    close(&real(&mut e, "interp1([4 5 7],[1.5 2.5])"), &[4.5, 6.], TOL);
    // pchip: end slopes 1.5 and 6, interior slopes 0 (hand-derived).
    close(
        &real(&mut e, "interp1([0 1 2 3],[0 1 1 5],[0.5 1.5 2.5],'pchip')"),
        &[0.6875, 1., 2.25],
        TOL,
    );
    e.execute("m = interp1([1 2 3]',[4 5;5 6;7 9],[1.5 2.5]');")
        .unwrap();
    assert_eq!(shape(&e, "m"), (2, 2));
    close(e.variables["m"].data(), &[4.5, 6., 5.5, 7.5], TOL);
    e.execute("g = interp1([1 2 3],[4 5 7],[1.5 2.5;2 3]);")
        .unwrap();
    assert_eq!(shape(&e, "g"), (2, 2));
    assert!(error(&mut e, "interp1([1 2 2 3],[4 5 6 7],1.5);").contains("strictly monotonic"));
    assert!(
        error(&mut e, "interp1([1 2 3],[4 5 7],2,'bogus');")
            .contains("Invalid interpolation method")
    );
}

#[test]
fn spline_pchip_interp2() {
    let mut e = Engine::default();
    close(
        &real(&mut e, "spline([0 1 2 3],[0 1 8 27],[0.5 1.5 2.5 -1 4])"),
        &[0.125, 3.375, 15.625, -1., 64.],
        1e-12,
    );
    close(
        &real(&mut e, "spline([0 1 2],[0 1 4],[0.5 1.5 3])"),
        &[0.25, 2.25, 9.],
        1e-13,
    );
    close(
        &real(&mut e, "spline([0 1],[0 1],[0.5 2])"),
        &[0.5, 2.],
        TOL,
    );
    close(
        &real(&mut e, "pchip([0 1 2 3],[0 1 1 5],[0.5 1.5 2.5])"),
        &[0.6875, 1., 2.25],
        TOL,
    );
    close(&real(&mut e, "pchip([0 1],[0 1],[0.5 2])"), &[0.5, 2.], TOL);
    assert!(error(&mut e, "spline([0 1 2],[0 1 4]);").contains("piecewise polynomial"));
    close(
        &real(&mut e, "interp2([1 2 3;4 5 6;7 8 9],[1.5 2.5],[1.5 2.5])"),
        &[3., 7.],
        TOL,
    );
    e.execute("g = interp2([1 2 3;4 5 6;7 8 9],[1.5 2.5],[1.5 2.5]');")
        .unwrap();
    assert_eq!(shape(&e, "g"), (2, 2));
    close(e.variables["g"].data(), &[3., 6., 4., 7.], TOL);
    close(
        &real(&mut e, "interp2([1 2 3;4 5 6;7 8 9],1.4,1.6,'nearest')"),
        &[4.],
        TOL,
    );
    close(
        &real(&mut e, "interp2([1 2 3;4 5 6;7 8 9],[0 4],[2 2])"),
        &[f64::NAN, f64::NAN],
        TOL,
    );
    close(
        &real(&mut e, "interp2([0 1 2],[0 10],[1 2 3;4 5 6],0.5,5)"),
        &[3.],
        TOL,
    );
}

fn with_functions() -> (Engine, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    for (name, body) in [
        ("fcos", "y = cos(x) - x;"),
        ("fcubic", "y = x.^3 - 2*x - 5;"),
        ("frosen", "y = 100*(x(2)-x(1)^2)^2 + (1-x(1))^2;"),
        ("fquad", "y = (x-3)^2 + 1;"),
        ("fsinx", "y = sin(x);"),
        ("fsq1", "y = x.^2 + 1;"),
        ("fnanneg", "y = x - 5 + 0./(x >= 0);"),
        ("fone", "y = 1;"),
        ("fsign", "y = x./abs(x);"),
        ("fnan1", "y = x.^2 - 2 + 0./(x ~= 1);"),
        ("fhole", "y = x - 1 + 0./(abs(x - 1) >= 0.8);"),
        ("fcplx", "y = x - 5 + 1i*(x < 0);"),
    ] {
        std::fs::write(
            dir.path().join(format!("{name}.m")),
            format!("function y = {name}(x)\n{body}\nend\n"),
        )
        .unwrap();
    }
    let mut e = Engine::default();
    e.set_function_directory(dir.path()).unwrap();
    (e, dir)
}

#[test]
fn fzero_finds_brackets_and_roots() {
    let (mut e, _dir) = with_functions();
    close(
        &real(&mut e, "fzero('fcos',0.5)"),
        &[0.7390851332151607],
        1e-13,
    );
    close(
        &real(&mut e, "fzero('fcos',[0 1])"),
        &[0.7390851332151607],
        1e-13,
    );
    close(
        &real(&mut e, "fzero('fcubic',2)"),
        &[2.0945514815423265],
        1e-13,
    );
    close(
        &real(&mut e, "fzero('fcubic',[3 2])"),
        &[2.0945514815423265],
        1e-13,
    );
    close(
        &real(&mut e, "fzero('cos',1)"),
        &[std::f64::consts::FRAC_PI_2],
        1e-13,
    );
    close(&real(&mut e, "fzero('fsinx',0)"), &[0.], TOL);
    e.execute("[x,fv,flag] = fzero('fcubic',[2 3]);").unwrap();
    assert!(e.variables["fv"].data()[0].abs() < 1e-10);
    assert_eq!(e.variables["flag"].data(), &[1.]);
    assert!(error(&mut e, "fzero('fsinx',[1 2]);").contains("must differ in sign"));
    assert!(error(&mut e, "fzero('fcos');").contains("Not enough input arguments"));
    let out = e.execute("fzero('fcos',0.5,'options');").unwrap();
    assert!(
        out.output.iter().any(|l| l.contains("ignores options")),
        "{:?}",
        out.output
    );
}

#[test]
fn fzero_search_follows_fzero_m_and_stops_on_bad_values() {
    let (mut e, _dir) = with_functions();
    // fzero probes x - dx before x + dx (dx = 1/50 at x = 0): the reference gives -pi/2.
    close(
        &real(&mut e, "fzero('cos',0)"),
        &[-std::f64::consts::FRAC_PI_2],
        1e-13,
    );
    // x - dx reaches x < 0 (NaN) before x + dx reaches the root at 5: exitflag -3.
    let out = e.execute("[x,fv,flag] = fzero('fnanneg',1);").unwrap();
    assert!(e.variables["x"].data()[0].is_nan());
    assert!(e.variables["fv"].data()[0].is_nan());
    assert_eq!(e.variables["flag"].data(), &[-3.]);
    let text = out.output.join("\n");
    assert!(
        text.contains("aborting search for an interval containing a sign change")
            && text.contains("NaN or Inf function value encountered"),
        "{text}"
    );
    // x^2 + 1 overflows to Inf before any sign change.
    e.execute("[x,fv,flag] = fzero('fsq1',1);").unwrap();
    assert_eq!(e.variables["flag"].data(), &[-3.]);
    // A constant never changes sign; the probe runs off to Inf: exitflag -6.
    let out = e.execute("[x,fv,flag] = fzero('fone',0);").unwrap();
    assert_eq!(e.variables["flag"].data(), &[-6.]);
    assert!(out.output.iter().any(|l| l.contains("no sign change")));
    // A complex value at a negative probe: exitflag -4.
    let out = e.execute("[x,fv,flag] = fzero('fcplx',1);").unwrap();
    assert_eq!(e.variables["flag"].data(), &[-4.]);
    assert!(
        out.output
            .iter()
            .any(|l| l.contains("complex function value"))
    );
    // NaN at an interval end is an error, not a sign change.
    assert!(error(&mut e, "fzero('fnanneg',[-1 6]);").contains("must be finite and real"));
    // NaN inside Brent (0/0 at x = 0) forces bisection and the loop still ends.
    e.execute("[x,fv,flag] = fzero('fsign',[-1 1]);").unwrap();
    assert!(e.variables["x"].data()[0].abs() < 1e-14);
    // A pole: |f| grows as the interval shrinks, exitflag -5 with fzero's message.
    let out = e.execute("[x,fv,flag] = fzero('tan',[1 2]);").unwrap();
    close(
        e.variables["x"].data(),
        &[std::f64::consts::FRAC_PI_2],
        1e-13,
    );
    assert_eq!(e.variables["flag"].data(), &[-5.]);
    assert!(out.output.iter().any(|l| l.contains("singular point")));
}

#[test]
fn fzero_keeps_its_bracket_over_nan_and_ends_from_subnormal_start() {
    let (mut e, _dir) = with_functions();
    // NaN at x = 1 inside [0 2]: the root sqrt(2) is on the far side of the NaN from 0.
    // x must not settle on the NaN point (f = -1 beside it).
    for interval in ["[2 0]", "[0 2]"] {
        e.execute(&format!("[x,fv,flag] = fzero('fnan1',{interval});"))
            .unwrap();
        close(e.variables["x"].data(), &[std::f64::consts::SQRT_2], 1e-13);
        assert!(e.variables["fv"].data()[0].abs() < 1e-10);
        assert_eq!(e.variables["flag"].data(), &[1.]);
    }
    // NaN on all of (0.2, 1.8) hides the root: no finite value next to it, flag -3 and a message.
    let out = e.execute("[x,fv,flag] = fzero('fhole',[0 2]);").unwrap();
    assert!(e.variables["x"].data()[0].is_nan());
    assert_eq!(e.variables["flag"].data(), &[-3.]);
    assert!(
        out.output.iter().any(|l| l.contains("returned NaN inside")),
        "{:?}",
        out.output
    );
    // dx = |x0|/50 is 0 or stuck at one subnormal step: the search must still end.
    for start in ["1e-323", "5e-323", "-1e-323", "1e-310"] {
        close(
            &real(&mut e, &format!("fzero('cos',{start})")),
            &[-std::f64::consts::FRAC_PI_2],
            1e-13,
        );
    }
}

#[test]
fn fminsearch_and_fminbnd_follow_the_reference_algorithms() {
    let (mut e, _dir) = with_functions();
    e.execute("[x,fv,flag] = fminsearch('frosen',[-1.2 1]);")
        .unwrap();
    // MATLAB documentation: x = 1.0000 1.0000, fval = 8.1777e-10.
    close(e.variables["x"].data(), &[1.000022, 1.000042], 1e-5);
    assert!((e.variables["fv"].data()[0] - 8.1777e-10).abs() < 1e-14);
    assert_eq!(e.variables["flag"].data(), &[1.]);
    close(&real(&mut e, "fminsearch('fquad',10)"), &[3.], 1e-4);
    e.execute("[x,fv,flag] = fminbnd('fsinx',0,6);").unwrap();
    assert!((e.variables["x"].data()[0] - 4.7124).abs() < 1e-4);
    assert!((e.variables["fv"].data()[0] + 1.).abs() < 1e-8);
    assert_eq!(e.variables["flag"].data(), &[1.]);
    close(
        &real(&mut e, "fminbnd('cos',3,4)"),
        &[std::f64::consts::PI],
        1e-5,
    );
    // fminbnd: reversed bounds give empty x and fval with exitflag -2.
    e.execute("[x,fv,flag] = fminbnd('fquad',10,0);").unwrap();
    assert_eq!(shape(&e, "x"), (0, 0));
    assert_eq!(shape(&e, "fv"), (0, 0));
    assert_eq!(e.variables["flag"].data(), &[-2.]);
    assert!(error(&mut e, "fminsearch('fquad',[]);").contains("must not be empty"));
}

#[test]
fn fft2_shifts_mode_and_histograms() {
    let mut e = Engine::default();
    e.execute("r = ifft2(fft2([1 2 3;4 5 6]));").unwrap();
    close(e.variables["r"].data(), &[1., 4., 2., 5., 3., 6.], 1e-12);
    close(&real(&mut e, "ifft2([10 -2;-4 0])"), &[1., 3., 2., 4.], TOL);
    // Symmetry is judged on the padded array [0 1 1 0], which is not symmetric.
    e.execute("z = ifft2([0 1 1],1,4);").unwrap();
    let Array::Complex(z) = e.array("z").unwrap() else {
        panic!("ifft2([0 1 1],1,4) must be complex")
    };
    let got: Vec<(f64, f64)> = z.data().iter().map(|c| (c.re, c.im)).collect();
    assert_eq!(
        got,
        vec![(0.5, 0.), (-0.25, 0.25), (0., 0.), (-0.25, -0.25)]
    );
    e.execute("z = ifft2([0 1 1]);").unwrap();
    assert!(matches!(e.array("z").unwrap(), Array::Numeric(_)));
    e.execute("f = fft2([1 2;3 4],3,3);").unwrap();
    assert_eq!(shape(&e, "f"), (3, 3));
    close(
        &real(&mut e, "real(fft2([1 2;3 4]))"),
        &[10., -4., -2., 0.],
        TOL,
    );
    close(
        &real(&mut e, "fftshift([1 2 3 4 5])"),
        &[4., 5., 1., 2., 3.],
        TOL,
    );
    close(
        &real(&mut e, "ifftshift([1 2 3 4 5])"),
        &[3., 4., 5., 1., 2.],
        TOL,
    );
    close(
        &real(&mut e, "fftshift([1 2 3;4 5 6;7 8 9])"),
        &[9., 3., 6., 7., 1., 4., 8., 2., 5.],
        TOL,
    );
    close(
        &real(&mut e, "ifftshift([1 2 3;4 5 6;7 8 9],1)"),
        &[4., 7., 1., 5., 8., 2., 6., 9., 3.],
        TOL,
    );
    e.execute("l = fftshift(true(1,3));").unwrap();
    assert!(e.logical_variables.contains_key("l"));
    close(&real(&mut e, "mode([1 2 2 3 3])"), &[2.], TOL);
    close(&real(&mut e, "mode([1 2;1 3;2 3])"), &[1., 3.], TOL);
    close(&real(&mut e, "mode([1 2;1 3;2 3],2)"), &[1., 1., 2.], TOL);
    e.execute("[m,f] = mode([1 2;1 3;2 3]);").unwrap();
    close(e.variables["f"].data(), &[2., 2.], TOL);
    close(&real(&mut e, "mode([])"), &[f64::NAN], TOL);
    close(
        &real(&mut e, "histc([1 2 2 3 5],[1 2 3 4 5])"),
        &[1., 2., 1., 0., 1.],
        TOL,
    );
    e.execute("h = histc([1 2 2 3 5]',[1 2 3 4 5]);").unwrap();
    assert_eq!(shape(&e, "h"), (5, 1));
    e.execute("[n,bin] = histc([1 2 2 3 5 7 0],[1 2 3 4 5]);")
        .unwrap();
    close(
        e.variables["bin"].data(),
        &[1., 2., 2., 3., 5., 0., 0.],
        TOL,
    );
    close(
        &real(&mut e, "histc([1 2;3 4;2 2],[1 2 3])"),
        &[1., 1., 1., 0., 2., 0.],
        TOL,
    );
    assert!(error(&mut e, "histc([1 2 3],[3 2 1]);").contains("monotonically"));
    assert!(error(&mut e, "histc([1 2;3 4],[1 2 3],3);").contains("dimension must be 1 or 2"));
    close(
        &real(&mut e, "histcounts([1 2 2 3 5],[1 2 3 4 5])"),
        &[1., 2., 1., 1.],
        TOL,
    );
    e.execute("[N,edges] = histcounts([1 2 2 3 5 7],3);")
        .unwrap();
    // Reference bins use rounded, readable edges rather than equal subdivisions.
    close(e.variables["N"].data(), &[3., 2., 1.], TOL);
    close(e.variables["edges"].data(), &[0., 3., 6., 9.], TOL);
    close(
        &real(&mut e, "histcounts([1 2 NaN 3],[0 2 4])"),
        &[1., 2.],
        TOL,
    );
}

#[test]
fn registered_in_the_function_list() {
    for name in [
        "conv",
        "deconv",
        "roots",
        "poly",
        "polyder",
        "polyint",
        "conv2",
        "interp2",
        "spline",
        "pchip",
        "fzero",
        "fminsearch",
        "fminbnd",
        "fft2",
        "ifft2",
        "fftshift",
        "ifftshift",
        "mode",
        "histc",
        "histcounts",
    ] {
        assert!(matrust::session::FUNCTIONS.contains(&name), "{name}");
    }
}

/// Fix round 2: reference values for polynomial interpolation and optimization.
#[test]
fn reference_differences_fixed_in_round_two() {
    let mut e = Engine::default();
    close(&real(&mut e, "conv(zeros(1,0),[1 2])"), &[0., 0.], TOL);
    assert!(error(&mut e, "conv(true,[1 2]);").contains("single or double"));
    e.execute("r = roots([]);").unwrap();
    assert_eq!(shape(&e, "r"), (0, 1));
    e.execute("r = roots([1+1i 2 3]);").unwrap();
    let Array::Complex(r) = e.array("r").unwrap() else {
        panic!("complex roots")
    };
    let mut got: Vec<(f64, f64)> = r.data().iter().map(|z| (z.re, z.im)).collect();
    got.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let want = [
        (-0.889085875953, -0.785063352083),
        (-0.110914124047, 1.78506335208),
    ];
    for (g, w) in got.iter().zip(want) {
        assert!(
            (g.0 - w.0).abs() < 1e-10 && (g.1 - w.1).abs() < 1e-10,
            "{got:?}"
        );
    }
    e.execute("[q,d] = polyder([1 2],[1 3]);").unwrap();
    close(e.variables["q"].data(), &[1.], TOL);
    close(e.variables["d"].data(), &[1., 6., 9.], TOL);
    close(&real(&mut e, "polyder([])"), &[0.], TOL);
    close(&real(&mut e, "polyder([1 2;3 4])"), &[3., 6., 2.], TOL);
    assert!(error(&mut e, "polyint([3 2 1]');").contains("Matrix dimensions"));
    close(
        &real(&mut e, "polyint([3 2 1],[1 2])"),
        &[1., 1., 1., 1., 2.],
        TOL,
    );
    e.execute("p = polyint([1 2 3],2+1i);").unwrap();
    let Array::Complex(p) = e.array("p").unwrap() else {
        panic!("complex polyint")
    };
    assert_eq!(p.data()[3].im, 1.);
    close(&real(&mut e, "conv2([],[1 2])"), &[0., 0.], TOL);
    e.execute("z = conv2(zeros(2,0),[1 1]);").unwrap();
    assert_eq!(shape(&e, "z"), (2, 2));
    e.execute("c = conv2([1+1i 2;3 4],[1 1]);").unwrap();
    let Array::Complex(c) = e.array("c").unwrap() else {
        panic!("complex conv2")
    };
    let got: Vec<(f64, f64)> = c.data().iter().map(|z| (z.re, z.im)).collect();
    assert_eq!(
        got,
        vec![(1., 1.), (3., 0.), (3., 1.), (7., 0.), (2., 0.), (4., 0.)]
    );
    close(
        &real(&mut e, "interp1([1 2 3 4],[1 4 9 16],[0 5],'spline')"),
        &[0., 25.],
        1e-12,
    );
    assert!(error(&mut e, "interp1([1 2 3],[4 5 7],'extrap');").contains("Wrong number"));
    assert!(error(&mut e, "interp1([1 2 3],[4 5 7],[0 4],'extrap');").contains("method must be"));
    close(
        &real(
            &mut e,
            "interp1([0 1 2 3],[0 1 4 9],[0.5 1.5 2.5],'v5cubic')",
        ),
        &[0.25, 2.25, 6.25],
        1e-12,
    );
    close(
        &real(
            &mut e,
            "interp1([0 1 2 3],[0 1 4 9],[-1 4],'v5cubic','extrap')",
        ),
        &[f64::NAN, f64::NAN],
        TOL,
    );
    close(
        &real(&mut e, "interp1([0 0.5 1],[1 3 2],[0.1 0.7],'v5cubic')"),
        &[1.64, 2.96],
        1e-12,
    );
    e.execute("z = conv2([],[]);").unwrap();
    assert_eq!(shape(&e, "z"), (0, 0));
    e.execute("z = conv2(zeros(0,2),zeros(0,3));").unwrap();
    assert_eq!(shape(&e, "z"), (0, 4));
    e.execute("[q,d] = polyder(5,[1 1]);").unwrap();
    close(e.variables["q"].data(), &[-5.], TOL);
    close(e.variables["d"].data(), &[1., 2., 1.], TOL);
    e.execute("[N,edges] = histcounts([5 5 5],3);").unwrap();
    close(e.variables["N"].data(), &[0., 3., 0.], TOL);
    close(
        e.variables["edges"].data(),
        &[4.5, 4.5 + 1. / 3., 5.5 - 1. / 3., 5.5],
        1e-12,
    );
    e.execute("[N,edges] = histcounts([-3.2 0.1 7.9 12.4 15],5);")
        .unwrap();
    close(
        e.variables["edges"].data(),
        &[-6., -1., 4., 9., 14., 19.],
        TOL,
    );
    e.execute("f = fft2([1 2;3 4]);").unwrap();
    assert!(matches!(e.array("f").unwrap(), Array::Numeric(_)));
    close(
        &real(&mut e, "double(strcmp(fftshift('abc'),'cab'))"),
        &[1.],
        TOL,
    );
    close(&real(&mut e, "double(strcmp(mode('aab'),'a'))"), &[1.], TOL);
    e.execute("m = mode(true);").unwrap();
    assert!(e.logical_variables.contains_key("m"));
    e.execute("h = histc([1 2 3],[]);").unwrap();
    assert_eq!(shape(&e, "h"), (1, 0));
    e.execute("[N,edges] = histcounts([1 2 3 4 5 6 7 8 9 10]);")
        .unwrap();
    close(e.variables["N"].data(), &[1.; 10], TOL);
    let half: Vec<f64> = (0..=10).map(|i| 0.5 + i as f64).collect();
    close(e.variables["edges"].data(), &half, TOL);
    close(&real(&mut e, "histcounts([])"), &[0.], TOL);
    e.execute("[N,edges] = histcounts([1 2 2 3 5 7],2);")
        .unwrap();
    close(e.variables["N"].data(), &[4., 2.], TOL);
    close(e.variables["edges"].data(), &[0., 4., 8.], TOL);
}
