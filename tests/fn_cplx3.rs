//! Reference cplx3 corpus (complex input to polyval, polyfit, conv,
//! deconv, filter, filtfilt, interp1, trapz, cumtrapz, meshgrid, roots, poly,
//! residue, polyder and polyint), stored as expected values. The LONG flags come from the
//! case list `fixtures/fn/cplx3-cases.txt`;
//! the captured fixture headers omit them. No reference is contacted by this test.
//! See docs/tasks/2026-10-25-complex-corpora-report.md.
use matrust::Engine;

fn scalar(engine: &Engine, name: &str) -> f64 {
    engine.array(name).unwrap().as_numeric().unwrap().data()[0]
}

#[test]
fn complex_interp1_uses_the_real_kernels_part_by_part() {
    for method in [
        "linear", "nearest", "next", "previous", "pchip", "spline", "cubic",
    ] {
        let mut e = Engine::default();
        e.execute(&format!("x=[1 2 4 5];v=[1i 2 3+1i -2i];q=[0.5 1.5 3 4.75 6];a=interp1(x,v,q,'{method}');b=interp1(x,real(v),q,'{method}')+1i*interp1(x,imag(v),q,'{method}');d=double(isequaln(a,b));"))
            .unwrap();
        assert_eq!(scalar(&e, "d"), 1., "{method}");
    }
    // KNOWN 62: the real spline kernel alone prints the same real part.
    let mut e = Engine::default();
    e.execute("format long").unwrap();
    let out = e
        .execute("x = interp1([1 2 3 4], [0 2 3 0], 5, 'spline')")
        .unwrap();
    assert!(out.output.concat().contains("-10.000000000000004"));
}

#[test]
fn complex_filter_blocks_deconv_and_polyval_agree() {
    let mut e = Engine::default();
    e.execute("b=[1 0.5i -0.25];a=[1 -0.3+0.2i 0.1i];x=sin(1:40)+1i*cos(1:40);[y,z]=filter(b,a,x);[y1,z1]=filter(b,a,x(1:17));[y2,z2]=filter(b,a,x(18:end),z1);r=max(abs([y1 y2]-y));s=max(abs(z2-z));")
        .unwrap();
    assert!(scalar(&e, "r") < 1e-14);
    assert!(scalar(&e, "s") < 1e-14);
    // FIR filtering is the leading part of the convolution.
    e.execute("c=conv(b,x);r=max(abs(filter(b,1,x)-c(1:40)));")
        .unwrap();
    assert!(scalar(&e, "r") < 1e-14);
    e.execute("p=[1i 2 -3+1i 4];u=[2 1i];[q,w]=deconv(conv(p,u)+[0 0 0 1 1i],u);r=max(abs(conv(u,q)+w-(conv(p,u)+[0 0 0 1 1i])));")
        .unwrap();
    assert!(scalar(&e, "r") < 1e-14);
    e.execute("t=[0.5+1i -2 3i];r=max(abs(polyval(p,t)-(p(1)*t.*t.*t+p(2)*t.*t+p(3)*t+p(4))));")
        .unwrap();
    assert!(scalar(&e, "r") < 1e-13);
    // Complex coefficients: filtfilt is linear in x.
    e.execute("f=@(v) filtfilt([0.5 0.5i],[1 -0.4i],v);v=(1:20)+1i*(20:-1:1);w=cos(1:20);r=max(abs(f(v+w)-f(v)-f(w)));")
        .unwrap();
    assert!(scalar(&e, "r") < 1e-12);
}

/// The error message without its "Line n, column m: " position.
fn error_of(source: &str) -> String {
    let error = Engine::default().execute(source).unwrap_err();
    match error.split_once(": ") {
        Some((position, message)) if position.starts_with("Line ") => message.to_string(),
        _ => error,
    }
}

#[test]
fn complex_filtfilt_divides_by_a1_before_the_initial_state() {
    let mut e = Engine::default();
    e.execute("x=ones(1,8);y=filtfilt([1 1i],[2 0.5],x);h=filtfilt([0.5 0.5i],[1 0.25],x);d=max(abs(y-h));c=max(abs(y-0.32i));")
        .unwrap();
    assert_eq!(scalar(&e, "d"), 0.);
    assert!(scalar(&e, "c") < 1e-14);
    // Longer filter, several columns.
    e.execute("b=[1 1i 0.5];a=[4 1 0.5i];X=[(1:12)' cos(1:12)'];y=filtfilt(b,a,X);h=filtfilt(b./4,a./4,X);d=max(max(abs(y-h)));")
        .unwrap();
    assert_eq!(scalar(&e, "d"), 0.);
}

#[test]
fn complex_filtfilt_rejects_singular_initial_conditions_as_the_real_path() {
    let message =
        "filtfilt has singular steady-state initial conditions (for example a pole at DC)";
    // One state: 1 + a(2) == 0.
    assert_eq!(error_of("y=filtfilt(1,[1 -1],ones(1,8));"), message);
    assert_eq!(error_of("y=filtfilt(1i,[1 -1],ones(1,8));"), message);
    assert_eq!(error_of("y=filtfilt(1,[2 -2],ones(1,8));"), message);
    assert_eq!(error_of("y=filtfilt(1i,[2 -2],ones(1,8));"), message);
    // Two states: the first column of the system vanishes.
    assert_eq!(error_of("y=filtfilt([1 1],[1 -1 0],ones(1,12));"), message);
    assert_eq!(error_of("y=filtfilt([1i 1],[1 -1 0],ones(1,12));"), message);
}

#[test]
fn complex_filtfilt_end_reflection_stays_finite_near_the_double_limit() {
    // 2 x(1) - x(k) overflows as a product but the reflected value is 1e308 + 1i.
    let mut e = Engine::default();
    e.execute("y=filtfilt([1 0],1,(1e308+1i)*ones(1,4));f=double(all(isfinite(real(y)))&&all(isfinite(imag(y))));d=max(abs(y-(1e308+1i)))/1e308;")
        .unwrap();
    assert_eq!(scalar(&e, "f"), 1.);
    assert!(scalar(&e, "d") < 1e-14);
    // A reflection that is itself not representable is an error.
    let error = error_of("y=filtfilt([1 0],1,[1.7e308 -1.7e308 -1.7e308 -1.7e308]);");
    assert_eq!(error, "filtfilt reflection overflowed");
}

#[test]
fn complex_filtfilt_steady_state_survives_a_sum_beyond_the_double_range() {
    // sum(b) = 2e308 overflows, the initial state b(2) = 1e308 does not.
    let mut e = Engine::default();
    e.execute("y=filtfilt([1e308 1e308],1,1i*1e-309*ones(1,8));f=double(all(isfinite(real(y)))&&all(isfinite(imag(y))));r=max(abs(real(y)));d=max(abs(imag(y)/4e307-1));")
        .unwrap();
    assert_eq!(scalar(&e, "f"), 1.);
    assert_eq!(scalar(&e, "r"), 0.);
    assert!(scalar(&e, "d") < 1e-12);
    // The same filter with a feedback term whose gain is representable.
    e.execute("y=filtfilt([1e308 1e308],[1 0.5],1i*1e-309*ones(1,8));f=double(all(isfinite(real(y)))&&all(isfinite(imag(y))));")
        .unwrap();
    assert_eq!(scalar(&e, "f"), 1.);
    // Only sum(a) == 0 is singular.
    assert_eq!(
        error_of("y=filtfilt([1e308 1e308],[1 -1],1i*ones(1,8));"),
        "filtfilt has singular steady-state initial conditions (for example a pole at DC)"
    );
}

#[test]
fn complex_filtfilt_rejects_an_overflowing_result() {
    let mut e = Engine::default();
    e.execute("x=5;").unwrap();
    let error = e.execute("x=filtfilt(1e308,1,[1i 1i]);").unwrap_err();
    assert!(error.contains("filtfilt gain overflowed"), "{error}");
    assert_eq!(scalar(&e, "x"), 5.);
    assert_eq!(
        error_of("y=filtfilt(1,[1 -1e200],1i*ones(1,8));"),
        "filtfilt recurrence overflowed; use a better-conditioned filter"
    );
    // The second pass starts from a state scaled by the first pass's output.
    assert_eq!(
        error_of("y=filtfilt([1e200 1e200],1,1i*ones(1,8));"),
        "filtfilt scaled initial state overflowed"
    );
}

#[test]
fn complex_polyfit_rejects_nonfinite_data() {
    for source in [
        "p=polyfit([NaN 1],[1i 2i],0);",
        "p=polyfit([1 Inf],[1i 2i],0);",
        "p=polyfit([1 2],[1i NaN],0);",
        "p=polyfit([1 2],[1i complex(1,Inf)],1);",
    ] {
        assert_eq!(
            error_of(source),
            "polyfit: X and Y must be finite",
            "{source}"
        );
    }
}

#[test]
fn complex_deconv_bounds_the_filter_order_before_any_work() {
    let started = std::time::Instant::now();
    let error = error_of("q=deconv(1i*ones(1,1000000),ones(1,500000));");
    assert_eq!(error, "filter supports order at most 4096");
    let error = error_of("[q,r]=deconv(1i*ones(1,1000000),[1 1i]);");
    assert_eq!(error, "filter supports order at most 4096");
    assert!(
        started.elapsed().as_secs_f64() < 5.,
        "{:?}",
        started.elapsed()
    );
    // Order 4096 is the largest accepted; the division still reproduces v.
    let mut e = Engine::default();
    e.execute("v=(1:4097)+1i;[q,r]=deconv(v,[1 1i]);d=max(abs(conv([1 1i],q)+r-v));")
        .unwrap();
    assert!(scalar(&e, "d") < 1e-9);
    assert_eq!(
        error_of("q=deconv(1i*ones(1,4098),[1 1i]);"),
        "filter supports order at most 4096"
    );
    // A divisor longer than the dividend needs no division at all.
    let mut e = Engine::default();
    e.execute("[q,r]=deconv([1i 2],1i*ones(1,5000));f=double(isequal(q,0)&&isequal(r,[1i 2]));")
        .unwrap();
    assert_eq!(scalar(&e, "f"), 1.);
}

#[test]
fn complex_deconv_rejects_matrices_as_the_real_path() {
    for (real, complex) in [
        ("deconv([1 2;3 4],1)", "deconv([1i 2;3 4],1)"),
        ("deconv([1 2 3],[1 2;3 4])", "deconv([1i 2 3],[1 2;3 4])"),
        ("deconv([1 2 3],[1 2;3 4])", "deconv([1 2 3],[1i 2;3 4])"),
    ] {
        let expected = error_of(&format!("x={real};"));
        assert_eq!(expected, "deconv: A and B must be vectors.");
        assert_eq!(error_of(&format!("x={complex};")), expected);
    }
}

#[test]
fn complex_residue_agrees_for_every_output_count() {
    let mut e = Engine::default();
    e.execute("b1=residue([1i;-1i],[1i;-1i],[]);[b2,a2]=residue([1i;-1i],[1i;-1i],[]);")
        .unwrap();
    e.execute("same=double(isequal(b1,b2));n=numel(b1);r=double(isreal(b1))+double(isreal(a2));v=b1(2);w=a2(3);")
        .unwrap();
    assert_eq!(scalar(&e, "same"), 1.);
    assert_eq!(scalar(&e, "n"), 2.);
    assert_eq!(scalar(&e, "r"), 2.);
    assert_eq!(scalar(&e, "v"), -2.);
    assert_eq!(scalar(&e, "w"), 1.);
    // With a direct term the numerator is still the same for one and two outputs.
    e.execute("c1=residue([1i;2],[1;-1i],[1 2]);[c2,d2]=residue([1i;2],[1;-1i],[1 2]);same=double(isequal(c1,c2));")
        .unwrap();
    assert_eq!(scalar(&e, "same"), 1.);
    // The two-input form narrows each output on its own.
    e.execute("r1=residue([1 1i],[1 2i 1]);[r2,p2,k2]=residue([1 1i],[1 2i 1]);same=double(isequal(r1,r2));")
        .unwrap();
    assert_eq!(scalar(&e, "same"), 1.);
}

#[test]
fn complex_meshgrid_follows_the_real_form() {
    let mut e = Engine::default();
    e.execute("[X,Y]=meshgrid([1i 2],[3 4]);a=double(isreal(X));b=double(isreal(Y));y=Y(2,1);x=abs(X(2,1)-1i);")
        .unwrap();
    assert_eq!(scalar(&e, "a"), 0.);
    assert_eq!(scalar(&e, "b"), 1.);
    assert_eq!(scalar(&e, "y"), 4.);
    assert_eq!(scalar(&e, "x"), 0.);
    e.execute("[U,V]=meshgrid([1i 2],[3 4]);W=meshgrid([1i 2],[3 4]);same=double(isequal(U,W));t=size(meshgrid(1i,[1 2 3]));")
        .unwrap();
    assert_eq!(scalar(&e, "same"), 1.);
    assert_eq!(
        e.array("t").unwrap().as_numeric().unwrap().data(),
        &[3., 1.]
    );
    // Column-major flattening of matrix inputs.
    e.execute("[X,Y]=meshgrid([1 2;3 4i]);d=max(abs(X(1,:)-[1 3 2 4i]));")
        .unwrap();
    assert_eq!(scalar(&e, "d"), 0.);
    assert_eq!(
        error_of("[X,Y,Z]=meshgrid([1i 2],[3 4],[5 6]);"),
        error_of("[X,Y,Z]=meshgrid([1 2],[3 4],[5 6]);")
    );
    assert_eq!(
        error_of("[X,Y,Z]=meshgrid([1i 2]);"),
        error_of("[X,Y,Z]=meshgrid([1 2]);")
    );
}

#[test]
fn complex_polyder_observes_cancellation_inside_the_convolution() {
    let mut e = Engine::default();
    e.execute("p=(1:65536)+1i*(1:65536);").unwrap();
    let stop = e.cancellation();
    let handle = std::thread::spawn({
        let stop = stop.clone();
        move || {
            std::thread::sleep(std::time::Duration::from_millis(150));
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    });
    let started = std::time::Instant::now();
    let error = e.execute("q=polyder(p,p);").unwrap_err();
    handle.join().unwrap();
    assert!(error.contains("cancelled"), "{error}");
    assert!(
        started.elapsed().as_secs_f64() < 5.,
        "{:?}",
        started.elapsed()
    );
}

const REFERENCE: &str = include_str!("fixtures/fn/cplx3-expected.txt");
const CASE_LIST: &str = include_str!("fixtures/fn/cplx3-cases.txt");
const TOTAL: usize = 115;

/// Cases that cannot pass for a reason outside this task, with the reason.
/// Each one still runs; a KNOWN case that starts to pass is reported as stale.
const KNOWN: &[(usize, &str)] = &[
    (
        62,
        "The real spline kernel (poly_interp, shared with real interp1) gives the real part -10.000000000000004 for MATLAB's -10.000000000000000; the imaginary part matches.",
    ),
    (
        90,
        "eig of the complex companion matrix (the engine's zgeev port, shared with eig): the second root's real part is about -1e-17 here and >= 0 in MKL, so format long prints -0.000000000000000 for MATLAB's 0.000000000000000.",
    ),
    (
        91,
        "eig of the complex companion matrix [1+1i -1i; 1 0] (the engine's zgeev port, shared with eig) returns the roots in the order [1; 1i] with -0 parts; MKL returns [1i; 1].",
    ),
    (
        93,
        "eig of the complex companion matrix (the engine's zgeev port, shared with eig) differs from MKL in the 15th digit: -0.586095029409547 + 1.640851941570563i for MATLAB's -0.586095029409546 + 1.640851941570562i.",
    ),
];

/// Comparison rules other than byte-exact console text.
const WARNING_LINE: &str = "Compare the [Warning: ...] text exactly without its backtrace (it names the oracle script and folder); the rest of the console text is exact.";
const LAST_DIGIT: &str = "Each numeric field (sign and fixed-width padding included) must agree with MATLAB within 1e-13 relative or 1e-15 absolute near zero; field end positions, other text, class and size are exact (the corpus report lists the tiny + 0.000000000000000i parts of case 52 and the filtfilt transient of case 46 under Compare with care).";
const HARNESS: &[(usize, &str)] = &[
    (15, WARNING_LINE),
    (53, WARNING_LINE),
    (46, LAST_DIGIT),
    (52, LAST_DIGIT),
    // polyfit by Householder QR (clean-room rewrite, docs/CLEAN_ROOM.md): the
    // reflector and back-substitution rounding is not MKL's, so the last digit
    // and the sign of ~1e-16 parts can differ (10: -1.333333333333335 for
    // -1.333333333333333; 11: -1.900000000000000 for -1.899999999999999;
    // 12: 0.0000 - 0.0000i for 0.0000 + 0.0000i; 13: 1.124999999999999 for 1.125).
    (10, LAST_DIGIT),
    (11, LAST_DIGIT),
    (12, LAST_DIGIT),
    (13, LAST_DIGIT),
];

fn rule(id: usize) -> Option<&'static str> {
    HARNESS.iter().find(|&&(n, _)| n == id).map(|&(_, r)| r)
}

struct Case<'a> {
    id: usize,
    statement: &'a str,
    long: bool,
    output: &'a str,
    class_size: Option<&'a str>,
}

fn cases() -> Vec<Case<'static>> {
    let list: Vec<&str> = CASE_LIST
        .lines()
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    let mut cases = Vec::new();
    let mut rest = REFERENCE;
    while let Some(start) = rest.find("@@CASE ") {
        let (header, after) = rest[start + 7..].split_once('\n').expect("header");
        let (id, statement) = header.split_once('|').expect("id|statement");
        let (block, after) = after.split_once("@@END\n").expect("end");
        let (output, class_size) = match block.split_once("@@CLASS ") {
            Some((output, class)) => (
                output,
                Some(class.strip_suffix('\n').expect("class newline")),
            ),
            None => (block, None),
        };
        let id: usize = id.parse().expect("numeric id");
        let listed = list[id - 1];
        let long = listed.starts_with("LONG ");
        assert_eq!(
            listed.strip_prefix("LONG ").unwrap_or(listed),
            statement,
            "case list and fixture disagree at {id}"
        );
        cases.push(Case {
            id,
            statement,
            long,
            output,
            class_size,
        });
        rest = after;
    }
    cases
}

/// The `[Warning: ...]` text without the backtrace, as the engine prints it.
fn warning_only(text: &str) -> String {
    let mut s = text.to_owned();
    while let Some(at) = s.find("[\u{8}> In ") {
        let end = at + s[at..].find("]\u{8} \n").unwrap() + "]\u{8} \n".len();
        s.replace_range(at..end, "");
    }
    s.replace("[\u{8}Warning: ", "Warning: ")
        .replace("]\u{8} \n", "\n")
}

fn last_digits_match(actual: &str, expected: &str) -> bool {
    // Signs (including the spaced imaginary sign) and numeric padding belong
    // to the numeric field. Field ends must stay at exactly the same columns.
    let re = regex::Regex::new(r"(?:[-+] *)?[0-9]+(?:\.[0-9]+)?(?:e[-+][0-9]+)?").unwrap();
    let scale = expected
        .lines()
        .find_map(|line| line.trim().strip_suffix(" *")?.parse::<f64>().ok())
        .unwrap_or(1.);
    let (mut ai, mut bi) = (0, 0);
    let a: Vec<_> = re.find_iter(actual).collect();
    let b: Vec<_> = re.find_iter(expected).collect();
    if a.len() != b.len() {
        return false;
    }
    for (x, y) in a.iter().zip(&b) {
        if x.end() != y.end()
            || actual[ai..x.start()].trim_end_matches(' ')
                != expected[bi..y.start()].trim_end_matches(' ')
        {
            return false;
        }
        let av: f64 = x.as_str().replace(' ', "").parse().unwrap();
        let bv: f64 = y.as_str().replace(' ', "").parse().unwrap();
        let within = av == bv || (av - bv).abs() <= (1e-13 * bv.abs()).max(1e-15 / scale.abs());
        if !within {
            return false;
        }
        ai = x.end();
        bi = y.end();
    }
    actual[ai..] == expected[bi..]
}

fn console_matches(id: usize, actual: &str, expected: &str) -> bool {
    match rule(id) {
        Some(r) if r == WARNING_LINE => actual == warning_only(expected),
        Some(r) if r == LAST_DIGIT => last_digits_match(actual, expected),
        _ => actual == expected,
    }
}

/// Run one case in a fresh engine: (console text or `ERR message`, class and size of x).
fn run(case: &Case) -> (String, Option<String>) {
    let mut engine = Engine::default();
    if case.long {
        engine.execute("format long").unwrap();
    }
    let actual = match engine.execute(case.statement) {
        Ok(result) => result
            .output
            .iter()
            .map(|s| format!("{s}\n"))
            .collect::<String>(),
        Err(error) => format!(
            "ERR {}\n",
            engine
                .diagnostic
                .as_ref()
                .map_or(error.as_str(), |d| d.message.as_str())
        ),
    };
    let class = engine
        .array("x")
        .map(|a| format!("{} [{} {}]", a.class(), a.rows(), a.cols()));
    (actual, class)
}

#[test]
fn corpus_is_complete_and_exceptions_are_explicit() {
    let cases = cases();
    assert_eq!(cases.len(), TOTAL);
    for (i, case) in cases.iter().enumerate() {
        assert_eq!(case.id, i + 1);
        assert_eq!(case.class_size.is_none(), case.output.starts_with("ERR "));
    }
    for (i, &(id, reason)) in KNOWN.iter().enumerate() {
        assert!(cases.iter().any(|c| c.id == id));
        assert!(!reason.is_empty());
        assert!(!KNOWN[..i].iter().any(|&(n, _)| n == id));
    }
    for (i, &(id, r)) in HARNESS.iter().enumerate() {
        let case = cases.iter().find(|c| c.id == id).expect("listed case");
        assert!(!r.is_empty());
        assert!(case.class_size.is_some());
        assert!(!HARNESS[..i].iter().any(|&(n, _)| n == id));
        assert!(!KNOWN.iter().any(|&(n, _)| n == id));
        assert_eq!(r == WARNING_LINE, case.output.contains("[\u{8}Warning: "));
    }
}

#[test]
fn relaxed_rules_only_allow_digit_noise() {
    let expected = "\nx =\n\n  0.937500000000000 + 0.000000000000000i\n\n";
    assert!(last_digits_match(
        "\nx =\n\n  0.937500000000000 - 0.000000000000000i\n\n",
        expected
    ));
    assert!(!last_digits_match(
        "\nx =\n\n  0.937500000001000 + 0.000000000000000i\n\n",
        expected
    ));
    assert!(!last_digits_match(
        "\nx =\n\n   0.937500000000000 + 0.000000000000000i\n\n",
        expected
    ));
    assert!(!last_digits_match(
        "\ny =\n\n  0.937500000000000 + 0.000000000000000i\n\n",
        expected
    ));
    assert_eq!(
        warning_only("[\u{8}Warning: a.]\u{8} \n[\u{8}> In <a>x</a>\n  In y]\u{8} \n\nx =\n"),
        "Warning: a.\n\nx =\n"
    );
}

#[test]
fn cplx3_matches_reference() {
    let mut failures = Vec::new();
    let mut passed = 0;
    let mut known = 0;
    for case in cases() {
        let (actual, class) = run(&case);
        let matches = if case.class_size.is_none() {
            actual == case.output && class.is_none()
        } else {
            console_matches(case.id, &actual, case.output) && class.as_deref() == case.class_size
        };
        if let Some((_, reason)) = KNOWN.iter().find(|&&(id, _)| id == case.id) {
            known += 1;
            if matches {
                failures.push(format!(
                    "KNOWN {} now passes; remove it ({reason})",
                    case.id
                ));
            }
            eprintln!("KNOWN {}: {reason}", case.id);
            continue;
        }
        if matches {
            passed += 1;
        } else {
            failures.push(format!(
                "case {} `{}`\nexpected {:?}\nactual   {:?}\nclass expected {:?}, actual {:?}",
                case.id, case.statement, case.output, actual, case.class_size, class
            ));
        }
    }
    assert_eq!(known, KNOWN.len());
    assert!(
        failures.is_empty(),
        "{passed}/{} supported cases passed; {known} KNOWN; {} failed:\n{}",
        TOTAL - known,
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(passed + known, TOTAL);
}
