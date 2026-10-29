//! Replays the reference statistics corpus byte for byte: cov, corrcoef, movmean,
//! movsum, detrend, discretize, hist, issorted, range (plus mode and cummax).
//!
//! `tests/fixtures/fn/stats-expected.txt` holds every statement of
//! `tests/fixtures/fn/stats-cases.txt` run in the reference: `@@CASE <id>|<statement>`,
//! the `evalc` console text, `@@CLASS <class> <size>` (absent on error),
//! `@@END`; an error case has `ERR <message>`. Cases whose statement is marked
//! `LONG` ran under `format long`.
use matrust::Engine;

const REFERENCE: &str = include_str!("fixtures/fn/stats-expected.txt");

/// Cases MatRust cannot pass for a reason outside this task, with the reason.
const KNOWN: &[(&str, &str)] = &[];

struct Case {
    id: String,
    statement: String,
    long: bool,
    expected: String,
    class: Option<(String, Vec<usize>)>,
}

fn cases(reference: &str) -> Vec<Case> {
    let mut cases = Vec::new();
    let mut rest = reference;
    while let Some(start) = rest.find("@@CASE ") {
        rest = &rest[start + "@@CASE ".len()..];
        let (header, after) = rest.split_once('\n').expect("case header line");
        let (id, statement) = header.split_once('|').expect("id|statement");
        let (body, after) = after.split_once("@@END\n").expect("case end marker");
        let (expected, class) = match body.split_once("@@CLASS ") {
            Some((text, class)) => {
                let (name, size) = class.trim_end().split_once(' ').expect("class size");
                let size = size
                    .trim_matches(['[', ']'])
                    .split_whitespace()
                    .map(|n| n.parse().expect("size element"))
                    .collect();
                (text.to_string(), Some((name.to_string(), size)))
            }
            None => (body.to_string(), None),
        };
        let (statement, long) = match statement.strip_prefix("LONG ") {
            Some(s) => (s, true),
            None => (statement, false),
        };
        cases.push(Case {
            id: id.into(),
            statement: statement.into(),
            long,
            expected,
            class,
        });
        rest = after;
    }
    cases
}

fn mismatch(case: &Case) -> Option<String> {
    let mut engine = Engine::default();
    if case.long {
        engine.execute("format long").expect("format long");
    }
    let result = engine.execute(&case.statement);
    if let Some(message) = case.expected.strip_prefix("ERR ") {
        return match result {
            Err(e) if e.trim_end() == message.trim_end() => None,
            Err(e) => Some(format!("expected error {message:?}, got error {e:?}")),
            Ok(_) => Some(format!("expected error {message:?}, got success")),
        };
    }
    let actual: String = match result {
        Ok(result) => result.output.iter().map(|e| format!("{e}\n")).collect(),
        Err(e) => return Some(format!("unexpected error {e:?}")),
    };
    if actual != case.expected {
        return Some(format!(
            "expected {:?}\n  actual   {:?}",
            case.expected, actual
        ));
    }
    if let Some((class, size)) = &case.class {
        let Some(x) = engine.array("x") else {
            return Some("no variable x".into());
        };
        let got = (x.class().to_string(), vec![x.rows(), x.cols()]);
        if got != (class.clone(), size.clone()) {
            return Some(format!("class/size expected {class} {size:?}, got {got:?}"));
        }
    }
    None
}

#[test]
fn corpus_has_every_case() {
    let all = cases(REFERENCE);
    assert_eq!(all.len(), 55);
    let statements = include_str!("fixtures/fn/stats-cases.txt");
    assert_eq!(
        statements.lines().filter(|l| !l.trim().is_empty()).count(),
        55
    );
    for (id, _) in KNOWN {
        assert!(
            all.iter().any(|c| c.id == *id),
            "known case {id} is in the corpus"
        );
    }
}

#[test]
fn matches_reference_byte_for_byte() {
    let mut failures = Vec::new();
    let mut passed = 0;
    for case in cases(REFERENCE) {
        let known = KNOWN.iter().any(|(id, _)| *id == case.id);
        match (mismatch(&case), known) {
            (None, false) => passed += 1,
            (None, true) => failures.push(format!("case {} passes; remove it from KNOWN", case.id)),
            (Some(_), true) => {}
            (Some(why), false) => {
                failures.push(format!("case {} `{}`\n  {why}", case.id, case.statement))
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{passed} passed, {} failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(passed + KNOWN.len(), 55);
}

fn run(source: &str) -> Result<String, String> {
    let mut engine = Engine::default();
    let result = engine.execute(source)?;
    Ok(result.output.iter().map(|e| format!("{e}\n")).collect())
}

fn values(source: &str) -> Vec<f64> {
    let mut engine = Engine::default();
    engine
        .execute(source)
        .unwrap_or_else(|e| panic!("{source}: {e}"));
    match engine.array("x").expect("x") {
        matrust::Array::Numeric(m) => m.data().to_vec(),
        other => panic!("x is {}", other.class()),
    }
}

/// Not reference-checked: forms beyond the corpus.
#[test]
fn hist_without_outputs_is_rejected_explicitly() {
    for statement in [
        "hist([1 2 3])",
        "hist([1 2 3]);",
        "hist([1 2 3], 2)",
        "f = @hist; f([1 2 3]);",
        "feval('hist', [1 2 3]);",
        "feval(@hist, [1 2 3]);",
        "name = 'hist'; feval(name, [1 2 3]);",
        "f = str2func('hist'); f([1 2 3]);",
        "str2func('hist')([1 2 3]);",
    ] {
        let error = run(statement).unwrap_err();
        assert!(
            error.contains("plotting is not supported yet"),
            "{statement}: {error}"
        );
    }
    for source in [
        "x = hist([1 2 3], [1 2 3]);",
        "f = @hist; x = f([1 2 3], [1 2 3]);",
        "x = feval('hist', [1 2 3], [1 2 3]);",
        "x = feval(@hist, [1 2 3], [1 2 3]);",
    ] {
        assert_eq!(values(source), [1., 1., 1.]);
    }
}

#[test]
fn moving_windows_long_and_short_agree() {
    // Window 41 takes the running-sum path, window 31 the direct path.
    let long = values("y = mod((1:200)*7, 13); y(50) = NaN; x = movsum(y, 41, 'omitnan');");
    let mut engine = Engine::default();
    engine
        .execute("y = mod((1:200)*7, 13); y(50) = NaN;")
        .unwrap();
    let y = match engine.array("y").unwrap() {
        matrust::Array::Numeric(m) => m.data().to_vec(),
        _ => unreachable!(),
    };
    for (i, &v) in long.iter().enumerate() {
        let lo = i.saturating_sub(20);
        let hi = (i + 20).min(199);
        let expected: f64 = y[lo..=hi].iter().filter(|v| !v.is_nan()).sum();
        assert_eq!(v, expected, "element {i}");
    }
    let nan = values("y = 1:100; y(50) = NaN; x = movmean(y, 41);");
    assert!(nan[29].is_nan() && nan[69].is_nan() && !nan[28].is_nan() && !nan[70].is_nan());
    let inf = values("y = ones(1, 100); y(10) = Inf; x = movsum(y, 40);");
    assert_eq!(inf[0], f64::INFINITY);
    assert_eq!(inf[99], 21.);
}

#[test]
fn option_and_shape_errors() {
    assert!(
        run("x = movmean(1:5, 2.5)")
            .unwrap_err()
            .contains("not supported yet")
    );
    assert!(run("x = movmean(1:5, 3, 'SamplePoints', 1:5)").is_err());
    assert!(
        run("x = issorted([1 2; 3 4])")
            .unwrap_err()
            .contains("'rows'")
    );
    assert!(
        run("x = discretize(1:5, 3)")
            .unwrap_err()
            .contains("not supported yet")
    );
    assert!(run("x = discretize(1:5, [3 1])").is_err());
    assert!(run("x = cov([1 2 3], [1 2])").is_err());
    assert!(run("x = cov([1 2 3], 2)").is_err());
    assert!(run("x = detrend(1:5, 'linear', 2)").is_err());
    assert_eq!(values("x = range([1 5; 7 2], 2);"), vec![4., 5.]);
    assert_eq!(
        values("[y, x] = discretize([1 2], [0 1 2]);"),
        vec![0., 1., 2.]
    );
    assert_eq!(
        values("x = movsum([1 2 3], 3, 'Endpoints', 7);"),
        vec![10., 6., 12.]
    );
}

/// Additional reference results supplied by the foreman on 2026-10-23;
/// these supplement, and do not modify, the original 55-case fixture.
#[test]
fn reference_movsum_omitnan_fill() {
    assert_eq!(
        values("x = movsum([1 2 3],3,'omitnan','Endpoints','fill');"),
        [3., 6., 5.]
    );
}

#[test]
fn reference_movsum_numeric_padding() {
    assert_eq!(
        values("x = movsum([1 2 3],3,'Endpoints',7);"),
        [10., 6., 12.]
    );
}

#[test]
fn reference_movmean_numeric_padding() {
    assert_eq!(
        values("x = movmean([1 2 3],3,'Endpoints',7);"),
        [10. / 3., 2., 4.]
    );
}

#[test]
fn reference_moving_sum_retains_small_terms() {
    assert_eq!(
        values("y = [1e20 ones(1,99)]; z = movsum(y,41); x = [z(51) z(end)];"),
        [41., 21.]
    );
}

#[test]
fn reference_covariance_product_overflow_is_preserved() {
    assert_eq!(
        values("x = cov([1e154 -1e154 1e154 -1e154]);"),
        [f64::INFINITY]
    );
}

#[test]
fn reference_covariance_mean_overflow_is_preserved() {
    assert_eq!(values("x = cov([1e308 1e308]);"), [f64::INFINITY]);
}

#[test]
fn moving_sum_and_mean_recover_after_large_samples_leave() {
    // Non-reference regression cases cover both arithmetic overflow and
    // cancellation, dimensions, and both moving kernels.
    for large in ["1e20", "-1e20", "1e308"] {
        for width in [31, 41] {
            for name in ["movsum", "movmean"] {
                let source = format!("y = [{large} {large} ones(1,98)]; x = {name}(y,{width});");
                let x = values(&source);
                assert_eq!(x[50], if name == "movmean" { 1. } else { width as f64 });
                assert_eq!(
                    x[99],
                    if name == "movmean" {
                        1.
                    } else {
                        (width / 2 + 1) as f64
                    }
                );
            }
        }
    }
    assert_eq!(
        values("y = [1e20 ones(1,99)]; z = movsum(y',41,1); x = [z(51) z(end)];"),
        [41., 21.]
    );
    assert_eq!(
        values("y = [1e20 ones(1,99)]; z = movsum([y;y],41,2); x = z(:,51);"),
        [41., 41.]
    );
}

#[test]
fn padding_applies_to_short_long_and_asymmetric_windows() {
    for width in [3, 41] {
        for name in ["movsum", "movmean"] {
            let x = values(&format!("x = {name}(ones(1,100),{width},'Endpoints',7);"));
            let sum = (width / 2 + 1) as f64 + 7. * (width / 2) as f64;
            let expected = if name == "movmean" {
                sum / width as f64
            } else {
                sum
            };
            assert_eq!(x[0], expected);
            assert_eq!(x[99], expected);
            let x = values(&format!(
                "x = {name}(ones(1,100),{width},'omitnan','Endpoints','fill');"
            ));
            assert_eq!(
                x[0],
                if name == "movmean" {
                    1.
                } else {
                    (width / 2 + 1) as f64
                }
            );
        }
    }
    assert_eq!(
        values("x = movsum([1 2 3],[3 1],'Endpoints',7);"),
        [24., 20., 20.]
    );
    assert_eq!(
        values("x = movmean([NaN NaN],3,'omitnan','Endpoints',7);"),
        [7., 7.]
    );
    assert!(values("x = movsum([1 2 3],3,'Endpoints','fill');")[0].is_nan());
}

#[test]
fn character_variables_work_for_all_text_options() {
    let success = |source: &str| run(source).unwrap_or_else(|e| panic!("{source}: {e}"));
    let error = |source: &str| {
        let message = run(source).unwrap_err();
        // Assigning the option variable moves the call's source column.
        message
            .split_once(": ")
            .expect("located error")
            .1
            .to_owned()
    };
    for name in ["movsum", "movmean"] {
        for flag in ["omitnan", "includenan"] {
            for endpoint in ["shrink", "discard", "fill"] {
                let literal =
                    format!("x = {name}([1 NaN 3],3,2,'{flag}','Endpoints','{endpoint}')");
                let variables = format!(
                    "f = '{flag}'; e = 'Endpoints'; v = '{endpoint}'; x = {name}([1 NaN 3],3,2,f,e,v)"
                );
                assert_eq!(success(&variables), success(&literal), "{variables}");
            }
        }
        assert_eq!(
            success(&format!("e = 'Endpoints'; x = {name}([1 2 3],3,e,7)")),
            success(&format!("x = {name}([1 2 3],3,'Endpoints',7)"))
        );
        assert_eq!(
            error(&format!("s = 'SamplePoints'; x = {name}(1:5,3,s,1:5)")),
            error(&format!("x = {name}(1:5,3,'SamplePoints',1:5)"))
        );
    }
    for kind in ["linear", "l", "constant", "c"] {
        assert_eq!(
            success(&format!("f = '{kind}'; x = detrend([3 1 7],f)")),
            success(&format!("x = detrend([3 1 7],'{kind}')"))
        );
    }
    for edge in ["left", "right"] {
        assert_eq!(
            success(&format!(
                "k = 'IncludedEdge'; v = '{edge}'; x = discretize([0 1 2],[0 1 2],k,v)"
            )),
            success(&format!(
                "x = discretize([0 1 2],[0 1 2],'IncludedEdge','{edge}')"
            ))
        );
        assert_eq!(
            values(&format!(
                "k = 'IncludedEdge'; v = '{edge}'; x = discretize([0 2],[0 1 2],[10 20],k,v);"
            )),
            [10., 20.]
        );
    }
    assert_eq!(
        error("v = 'categorical'; x = discretize([0 1],[0 1 2],v)"),
        error("x = discretize([0 1],[0 1 2],'categorical')")
    );
    for option in ["alpha", "rows"] {
        assert_eq!(
            error(&format!("v = '{option}'; x = corrcoef([1 2 3],v)")),
            error(&format!("x = corrcoef([1 2 3],'{option}')"))
        );
    }
    for input in ["[1 2; 1 3]", "char([97 98;97 99])"] {
        assert_eq!(
            success(&format!("v = 'rows'; x = issorted({input},v)")),
            success(&format!("x = issorted({input},'rows')"))
        );
    }
    assert_eq!(
        success("a = char([97 98;97 99]); v = 'rows'; x = issorted(a,v)"),
        success("x = issorted(char([97 98;97 99]),'rows')")
    );
}

#[test]
fn covariance_and_correlation_reject_oversized_results() {
    for name in ["cov", "corrcoef"] {
        let error = run(&format!("x = {name}(zeros(2,5000));")).unwrap_err();
        assert!(error.contains("16777216-element limit"), "{name}: {error}");
    }
}

/// Explicit release measurement, not a correctness test or a reference skip.
/// Run in separate processes with MATRUST_STATS_BENCH_WORKERS=1 and =4.
#[test]
#[ignore = "release benchmark; run explicitly with --ignored --nocapture"]
fn benchmark_stats_scans() {
    let workers = std::env::var("MATRUST_STATS_BENCH_WORKERS")
        .unwrap_or_else(|_| "1".into())
        .parse()
        .unwrap();
    matrust::initialize_worker_pool(workers).unwrap();
    let mut engine = Engine::default();
    engine
        .execute("y = ones(1,1000000); z = ones(100000,10);")
        .unwrap();
    for source in [
        "x = movsum(y,31);",
        "x = movsum(y,41);",
        "x = movsum(y,1001);",
        "x = movmean(y,41);",
        "x = hist(y,50);",
        "x = detrend(z,'constant');",
        "x = detrend(z,'linear');",
        "x = issorted(y);",
    ] {
        engine.execute(source).unwrap(); // warm-up
        let mut elapsed = Vec::new();
        for _ in 0..3 {
            let start = std::time::Instant::now();
            std::hint::black_box(engine.execute(source).unwrap());
            elapsed.push(start.elapsed().as_secs_f64() * 1000.);
        }
        println!("workers={workers} input=1000000 repetitions=3 {source} ms={elapsed:?}");
    }
}
