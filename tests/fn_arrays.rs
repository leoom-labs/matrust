//! Replay every arrays case in a fresh engine: exact console bytes, class,
//! dimensions and error text. LONG flags come from the original case list.
//! KNOWN is explicit and checked for stale entries; HARNESS documents any
//! relaxed comparison. The fixture is reference evidence.
use matrust::Engine;

const REFERENCE: &str = include_str!("fixtures/fn/arrays-expected.txt");

/// Cases MatRust cannot pass for a reason outside this task, with the reason.
const KNOWN: &[(&str, &str)] = &[
    (
        "114",
        "table-only in the reference; MatRust says not supported yet",
    ),
    (
        "115",
        "table-only in the reference; MatRust says not supported yet",
    ),
    (
        "116",
        "table-only in the reference; MatRust says not supported yet",
    ),
];

/// Every relaxation of the verbatim comparison must be listed here.
const LOCATION: &str =
    "Remove only MatRust's exact 'Line 1, column 1: ' prefix; remaining error text is exact";
const ROUNDING: &str = "Weighted centered variance differs in the last printed digits: numeric tokens within 2 ULP; all other bytes, class and size exact";
const HARNESS: &[(&str, &str)] = &[
    ("13", LOCATION),
    ("14", LOCATION),
    ("40", LOCATION),
    ("44", ROUNDING),
    ("56", LOCATION),
    ("57", LOCATION),
    ("64", LOCATION),
    ("71", LOCATION),
    ("77", LOCATION),
    ("88", LOCATION),
    ("89", LOCATION),
    ("95", LOCATION),
    ("98", LOCATION),
    ("104", LOCATION),
    ("113", LOCATION),
    ("125", LOCATION),
    ("126", LOCATION),
    ("134", LOCATION),
    ("146", LOCATION),
];

fn rounded_console_matches(actual: &str, expected: &str) -> bool {
    let number = regex::Regex::new(r"-?(?:[0-9]+\.[0-9]+|[0-9]+)(?:[eE][+-]?[0-9]+)?").unwrap();
    let a = number.find_iter(actual).collect::<Vec<_>>();
    let b = number.find_iter(expected).collect::<Vec<_>>();
    if a.len() != b.len() {
        return false;
    }
    let (mut pa, mut pb) = (0, 0);
    for (a, b) in a.into_iter().zip(b) {
        if actual[pa..a.start()] != expected[pb..b.start()] {
            return false;
        }
        if a.as_str() != b.as_str() {
            // Integer labels and NaN/Inf spellings are not relaxed.
            if !a.as_str().contains('.') || !b.as_str().contains('.') {
                return false;
            }
            let x = a.as_str().parse::<f64>().unwrap();
            let y = b.as_str().parse::<f64>().unwrap();
            if !x.is_finite()
                || !y.is_finite()
                || x.is_sign_negative() != y.is_sign_negative()
                || x.to_bits().abs_diff(y.to_bits()) > 2
            {
                return false;
            }
        }
        (pa, pb) = (a.end(), b.end());
    }
    actual[pa..] == expected[pb..]
}

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
            long: long
                || include_str!("fixtures/fn/arrays-cases.txt")
                    .lines()
                    .nth(id.parse::<usize>().unwrap() - 1)
                    .unwrap()
                    .starts_with("LONG "),
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
    let rule = HARNESS
        .iter()
        .find(|(id, _)| *id == case.id)
        .map(|(_, rule)| *rule);
    if let Some(message) = case.expected.strip_prefix("ERR ") {
        let message = message.strip_suffix('\n').expect("error line ending");
        return match result {
            Err(e)
                if (if rule == Some(LOCATION) {
                    e.strip_prefix("Line 1, column 1: ").unwrap_or(&e)
                } else {
                    &e
                }) == message =>
            {
                None
            }
            Err(e) => Some(format!("expected error {message:?}, got error {e:?}")),
            Ok(_) => Some(format!("expected error {message:?}, got success")),
        };
    }
    let actual: String = match result {
        Ok(result) => result.output.iter().map(|e| format!("{e}\n")).collect(),
        Err(e) => return Some(format!("unexpected error {e:?}")),
    };
    if actual != case.expected
        && !(rule == Some(ROUNDING) && rounded_console_matches(&actual, &case.expected))
    {
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
    assert_eq!(all.len(), 146);
    let statements = include_str!("fixtures/fn/arrays-cases.txt");
    assert_eq!(
        statements.lines().filter(|l| !l.trim().is_empty()).count(),
        146
    );
    for (case, source) in all.iter().zip(statements.lines()) {
        assert_eq!(
            case.statement,
            source.strip_prefix("LONG ").unwrap_or(source)
        );
    }
    assert_eq!(
        all.iter()
            .map(|c| c.id.parse::<usize>().unwrap())
            .collect::<Vec<_>>(),
        (1..=146).collect::<Vec<_>>()
    );
    let mut listed = std::collections::BTreeSet::new();
    for (id, reason) in KNOWN.iter().chain(HARNESS) {
        assert!(!reason.is_empty());
        assert!(listed.insert(id), "case {id} is listed more than once");
        assert!(
            all.iter().any(|c| c.id == *id),
            "known case {id} is in the corpus"
        );
    }
}

#[test]
fn rounding_rule_keeps_layout_and_meaningful_differences_strict() {
    assert!(rounded_console_matches(
        "   6.333333333333333\n",
        "   6.333333333333334\n"
    ));
    assert!(!rounded_console_matches(
        "  6.333333333333333\n",
        "   6.333333333333334\n"
    ));
    assert!(!rounded_console_matches(
        "   6.333333333333300\n",
        "   6.333333333333334\n"
    ));
    assert!(!rounded_console_matches("   NaN\n", "   Inf\n"));
    assert!(!rounded_console_matches(
        "Columns 1 through 5",
        "Columns 1 through 6"
    ));
}

#[test]
fn matches_reference_corpus() {
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
    assert_eq!(passed + KNOWN.len(), 146);
}

fn numeric(engine: &Engine, name: &str) -> Vec<f64> {
    match engine.array(name).expect("output array") {
        matrust::Array::Numeric(a) => a.data().to_vec(),
        matrust::Array::Typed(a) => (0..a.len()).map(|i| a.real(i)).collect(),
        matrust::Array::Logical(a) => a.data().iter().map(|&x| f64::from(x)).collect(),
        a => panic!("unexpected {} output", a.class()),
    }
}

/// Independent direct definition, including large windows that exercise the
/// incremental variance path. These are mathematical checks, not reference data.
#[test]
fn moving_windows_match_direct_definitions() {
    for (rows, cols) in [(1, 79), (79, 1), (13, 7)] {
        let data = (0..rows * cols)
            .map(|i| {
                if i % 11 == 0 {
                    f64::NAN
                } else {
                    ((i * 47) % 31) as f64 - 15.
                }
            })
            .collect::<Vec<_>>();
        let mut engine = Engine::default();
        engine.variables.insert(
            "a".into(),
            matrust::Matrix::from_column_vec(rows, cols, data.clone()).unwrap(),
        );
        for dim in [1, 2] {
            let n = if dim == 1 { rows } else { cols };
            for (before, after) in [(0, 0), (1, 2), (20, 20), (100, 0), (0, 100)] {
                for name in ["movmax", "movmin", "movmedian", "movvar", "movstd"] {
                    for omit in [false, true] {
                        let norm = if matches!(name, "movstd" | "movvar") {
                            ", 0"
                        } else {
                            ""
                        };
                        let flag = if omit { "omitnan" } else { "includenan" };
                        let source =
                            format!("x = {name}(a, [{before} {after}]{norm}, {dim}, '{flag}');");
                        engine
                            .execute(&source)
                            .unwrap_or_else(|e| panic!("{source}: {e}"));
                        let got = numeric(&engine, "x");
                        for c in 0..cols {
                            for r in 0..rows {
                                let index = if dim == 1 { r } else { c };
                                let mut window = (index.saturating_sub(before)
                                    ..(index + after + 1).min(n))
                                    .map(|j| {
                                        data[if dim == 1 { j + c * rows } else { r + j * rows }]
                                    })
                                    .filter(|v| !omit || !v.is_nan())
                                    .collect::<Vec<_>>();
                                let expected = if window.is_empty()
                                    || window.iter().any(|v| v.is_nan())
                                {
                                    f64::NAN
                                } else {
                                    window.sort_by(f64::total_cmp);
                                    let count = window.len();
                                    match name {
                                        "movmax" => window[count - 1],
                                        "movmin" => window[0],
                                        "movmedian" => {
                                            if count % 2 == 1 {
                                                window[count / 2]
                                            } else {
                                                (window[count / 2 - 1] + window[count / 2]) / 2.
                                            }
                                        }
                                        _ => {
                                            let mean = window.iter().sum::<f64>() / count as f64;
                                            let var = window
                                                .iter()
                                                .map(|x| (x - mean).powi(2))
                                                .sum::<f64>()
                                                / count.saturating_sub(1).max(1) as f64;
                                            if name == "movstd" { var.sqrt() } else { var }
                                        }
                                    }
                                };
                                let actual = got[r + c * rows];
                                assert!(
                                    (actual.is_nan() && expected.is_nan())
                                        || (actual - expected).abs()
                                            <= 1e-10 * expected.abs().max(1.),
                                    "{source} ({r},{c}): {actual} != {expected}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn endpoint_padding_and_class_preservation() {
    let mut e = Engine::default();
    for (source, class, expected) in [
        (
            "x=movmax(int8([-5 -2 -4]),3,'Endpoints','fill');",
            "int8",
            vec![-2., -2., -2.],
        ),
        (
            "x=movmedian(single([1 5 3]),3);",
            "single",
            vec![3., 3., 4.],
        ),
        ("x=movvar(single([1 3]),2);", "single", vec![0., 2.]),
        (
            "x=movmedian([2 4],1000000000000,'Endpoints',7);",
            "double",
            vec![7., 7.],
        ),
        (
            "x=movmin([2 4],1000000000000,'Endpoints',-7);",
            "double",
            vec![-7., -7.],
        ),
        (
            "x=movmedian([1 2],5,'Endpoints','discard');",
            "double",
            vec![],
        ),
        (
            "x=repelem(single([1 2]),[0 3]);",
            "single",
            vec![2., 2., 2.],
        ),
    ] {
        e.execute(source)
            .unwrap_or_else(|err| panic!("{source}: {err}"));
        assert_eq!(e.array("x").unwrap().class(), class, "{source}");
        assert_eq!(numeric(&e, "x"), expected, "{source}");
    }
    e.execute("a=uint64(9007199254740993); b=a+uint64(2); x=movmax([a b],2); y=movmedian([a b],2); z=repelem([a b],2);").unwrap();
    for (name, expected) in [
        ("x", vec![9007199254740993, 9007199254740995]),
        ("y", vec![9007199254740993, 9007199254740994]),
        (
            "z",
            vec![
                9007199254740993,
                9007199254740993,
                9007199254740995,
                9007199254740995,
            ],
        ),
    ] {
        let matrust::Array::Typed(t) = e.array(name).unwrap() else {
            panic!("integer output");
        };
        assert_eq!((0..t.len()).map(|i| t.int(i)).collect::<Vec<_>>(), expected);
    }
}

#[test]
fn tolerance_indices_boundaries_and_rows() {
    let mut e = Engine::default();
    for (source, expected) in [
        ("[tf,x]=ismembertol([1.1 2], [1 2], .05);", vec![0., 2.]),
        ("[tf,x]=ismembertol([2 1], [2 1 2 1]);", vec![1., 2.]),
        (
            "x=ismembertol([Inf -Inf NaN], [Inf -Inf NaN],1e-12,'DataScale',1);",
            vec![1., 1., 0.],
        ),
        (
            "x=ismembertol([1 2; 1 3], [1 3; 1 2], 'ByRows', true);",
            vec![1., 1.],
        ),
        (
            "[c,ia,ic]=uniquetol([1 1.01 2],.01);x=[ia' ic'];",
            vec![1., 3., 1., 1., 2.],
        ),
        (
            "[tf,loc]=ismembertol([1 9],[1 1 2], 'OutputAllIndices',true);x=loc{1};",
            vec![1., 2.],
        ),
        (
            "[c,ia]=uniquetol([1 1.01 2],.01,'OutputAllIndices',true);x=ia{1};",
            vec![1., 2.],
        ),
    ] {
        e.execute(source)
            .unwrap_or_else(|err| panic!("{source}: {err}"));
        assert_eq!(numeric(&e, "x"), expected, "{source}");
    }
}

#[test]
fn recursive_nan_equality_and_complex_structure() {
    let mut e = Engine::default();
    for (source, expected) in [
        (
            "x=isequaln(struct('b',{NaN,2},'a',{1,3}),struct('a',{1,3},'b',{NaN,2}));",
            true,
        ),
        (
            "x=isequaln(uint64(9007199254740993),9007199254740992);",
            false,
        ),
        ("x=isequaln(single(NaN),NaN);", true),
        ("x=issymmetric([NaN 1;1 2]);", false),
        ("x=ishermitian([1i 2+3i;-2+3i 4i],'skew');", true),
        ("x=isdiag([1 0;0 NaN]);", true),
        ("x=isdiag([1 NaN;0 1]);", false),
    ] {
        e.execute(source)
            .unwrap_or_else(|err| panic!("{source}: {err}"));
        assert_eq!(numeric(&e, "x"), vec![f64::from(expected)], "{source}");
    }
}

#[test]
fn histogram_edges_locations_and_normalization() {
    let mut e = Engine::default();
    e.execute("[n,xe,ye,bx,by]=histcounts2([-1 0 1 2 3 NaN],[0 0 1 2 3 1],[0 1 2],[0 1 2]);")
        .unwrap();
    assert_eq!(numeric(&e, "n"), vec![1., 0., 0., 2.]);
    assert_eq!(numeric(&e, "bx"), vec![0., 1., 2., 2., 0., 0.]);
    assert_eq!(numeric(&e, "by"), vec![0., 1., 2., 2., 0., 0.]);
    e.execute("x=histcounts2([1 3],[1 3],[0 2 4],[0 2 4],'Normalization','pdf');")
        .unwrap();
    assert_eq!(numeric(&e, "x"), vec![0.125, 0., 0., 0.125]);
    // Supplemental invariants, not additional reference evidence.
    for input in ["[.1 .4 1.2 1.7]", "[100 150 200 250]", "[.2 .2 .2 .2]"] {
        for bins in ["", ",3", ",[2 4]"] {
            e.execute(&format!(
                "[n,xe,ye,bx,by]=histcounts2({input},{input}{bins});"
            ))
            .unwrap();
            assert_eq!(numeric(&e, "n").iter().sum::<f64>(), 4.);
            for edge in ["xe", "ye"] {
                assert!(numeric(&e, edge).windows(2).all(|w| w[0] < w[1]));
            }
            assert!(numeric(&e, "bx").iter().all(|&b| b >= 1.));
            assert!(numeric(&e, "by").iter().all(|&b| b >= 1.));
        }
    }
}

#[test]
fn oversized_and_unsupported_forms_fail_without_publication() {
    let mut e = Engine::default();
    e.execute("x=7;").unwrap();
    for source in [
        "x=repelem(1,1e20);",
        "x=repelem({1},65537);",
        "x=histcounts2([1 2],[1 2],[65536 65536]);",
        "x=histcounts2([1 2],[1 2],[0 1 1],[0 1 2]);",
        "x=movmedian([1 2],3,'SamplePoints',[1 2]);",
        "x=movvar([1 2],3,2);",
        "x=movstd(uint64([1 2]),3);",
        "x=movvar(int8([1 2]),3);",
        "x=movmax([1 2],3,3);",
        "x=uniquetol([1 2],.1,'bogus',true);",
        "x=ismembertol([1 2],[1 2],-1);",
    ] {
        assert!(e.execute(source).is_err(), "{source}");
        assert_eq!(numeric(&e, "x"), vec![7.]);
    }
}

#[test]
fn parallel_columns_are_identical_to_serial() {
    let a = matrust::Matrix::from_column_vec(
        32768,
        8,
        (0..262144).map(|i| ((i * 43) % 1009) as f64).collect(),
    )
    .unwrap();
    for name in ["movmax", "movmin", "movmedian", "movstd", "movvar"] {
        let run = |workers| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap()
                .install(|| {
                    let mut e = Engine::default();
                    e.variables.insert("a".into(), a.clone());
                    e.execute(&format!("x={name}(a,65);")).unwrap();
                    numeric(&e, "x")
                })
        };
        assert_eq!(run(1), run(4), "{name}");
    }
}

/// Run explicitly in release mode; prints per-call medians, excludes setup and
/// display, includes dispatch/allocation. No performance assertions on shared hosts.
#[test]
#[ignore = "release benchmark; run with --ignored --nocapture"]
fn benchmark_arrays_release() {
    fn measure(workers: usize, reps: usize, a: &matrust::Matrix, source: &str) {
        let mut e = Engine::default();
        e.variables.insert("a".into(), a.clone());
        e.execute(source).unwrap();
        let mut times = Vec::with_capacity(reps);
        for _ in 0..reps {
            let start = std::time::Instant::now();
            e.execute(source).unwrap();
            times.push(start.elapsed().as_secs_f64() * 1000.);
        }
        times.sort_by(f64::total_cmp);
        println!(
            "workers={workers} shape={}x{} reps={reps} median_ms={:.6} {source}",
            a.rows(),
            a.cols(),
            times[reps / 2]
        );
    }
    for workers in [1, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            for (rows, cols, reps) in [(32, 4, 101), (4096, 4, 11), (32768, 8, 5)] {
                let a = matrust::Matrix::from_column_vec(
                    rows,
                    cols,
                    (0..rows * cols).map(|i| ((i * 43) % 1009) as f64).collect(),
                )
                .unwrap();
                for source in [
                    "x=movmax(a,65);",
                    "x=movmin(a,65);",
                    "x=movmedian(a,65);",
                    "x=movstd(a,65);",
                    "x=movvar(a,65);",
                    "x=movmean(a,65);",
                    "x=movsum(a,65);",
                    "x=repelem(a,1,2);",
                    "x=isequaln(a,a);",
                    "x=bandwidth(a);",
                    "x=uniquetol(a);",
                    "x=ismembertol(a,a);",
                    "x=histcounts2(a,a,32);",
                    "x=histcounts2(a,a);",
                ] {
                    measure(workers, reps, &a, source);
                }
            }
            // Diagonal input forces a complete successful structural scan;
            // the square sizes also exercise the transpose comparisons.
            for (size, reps) in [(16, 101), (128, 11), (512, 5)] {
                let a = matrust::Matrix::from_column_vec(
                    size,
                    size,
                    (0..size * size)
                        .map(|i| {
                            if i % size == i / size {
                                (i % size + 1) as f64
                            } else {
                                0.
                            }
                        })
                        .collect(),
                )
                .unwrap();
                for source in [
                    "x=isdiag(a);",
                    "x=istril(a);",
                    "x=istriu(a);",
                    "x=isbanded(a,1,1);",
                    "x=issymmetric(a);",
                    "x=ishermitian(a);",
                ] {
                    measure(workers, reps, &a, source);
                }
            }
        });
    }
}

#[test]
fn shared_moving_parser_preserves_mean_and_sum() {
    let mut e = Engine::default();
    for (source, expected) in [
        ("x=movmean([1 2 3],2);", vec![1., 1.5, 2.5]),
        ("x=movsum([1 NaN 3],3,'omitnan');", vec![1., 4., 3.]),
        ("x=movsum([1 2 3],3,'Endpoints','discard');", vec![6.]),
        ("x=movmean([1 2 3],3,'Endpoints',0);", vec![1., 2., 5. / 3.]),
        ("x=movsum([1 2;3 4], [1 0], 2);", vec![1., 3., 3., 7.]),
    ] {
        e.execute(source).unwrap();
        assert_eq!(numeric(&e, "x"), expected, "{source}");
    }
}

/// Interleave worker settings to reduce run-order bias on shared/hybrid hosts.
#[test]
#[ignore = "paired release benchmark; run with --ignored --nocapture"]
fn benchmark_moving_paired_release() {
    let pools = [1, 4].map(|workers| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
    });
    for rows in [32768, 131072] {
        let a = matrust::Matrix::from_column_vec(
            rows,
            8,
            (0..rows * 8).map(|i| ((i * 43) % 1009) as f64).collect(),
        )
        .unwrap();
        for name in ["movmax", "movmin", "movmedian", "movstd", "movvar"] {
            let source = format!("x={name}(a,65);");
            let mut engines = [Engine::default(), Engine::default()];
            for (i, engine) in engines.iter_mut().enumerate() {
                engine.variables.insert("a".into(), a.clone());
                pools[i].install(|| engine.execute(&source)).unwrap();
            }
            let mut times = [Vec::new(), Vec::new()];
            for repetition in 0..21 {
                for step in 0..2 {
                    let i = (step + repetition) % 2;
                    let start = std::time::Instant::now();
                    pools[i].install(|| engines[i].execute(&source)).unwrap();
                    times[i].push(start.elapsed().as_secs_f64() * 1000.);
                }
            }
            for (i, times) in times.iter_mut().enumerate() {
                times.sort_by(f64::total_cmp);
                println!(
                    "workers={} shape={rows}x8 reps=21 median_ms={:.6} {source}",
                    [1, 4][i],
                    times[10]
                );
            }
        }
    }
}

#[test]
fn variance_handles_constant_tails_large_offsets_and_extreme_ranges() {
    let mut e = Engine::default();
    let mut data = (0..100).map(|i| (i % 17) as f64).collect::<Vec<_>>();
    data.extend(vec![7.; 100]);
    e.variables.insert(
        "a".into(),
        matrust::Matrix::from_column_vec(1, 200, data).unwrap(),
    );
    e.execute("x=movstd(a,65);").unwrap();
    assert!(numeric(&e, "x")[132..].iter().all(|&x| x == 0.));
    e.variables.insert(
        "a".into(),
        matrust::Matrix::from_column_vec(1, 3, vec![1e16, 1e16 + 2., 1e16 + 4.]).unwrap(),
    );
    e.execute("x=movvar(a,3);").unwrap();
    assert_eq!(numeric(&e, "x"), vec![2., 4., 2.]);
    for magnitude in [1e308, 1e-200] {
        e.variables.insert(
            "a".into(),
            matrust::Matrix::from_column_vec(1, 2, vec![magnitude, -magnitude]).unwrap(),
        );
        e.execute("x=movstd(a,2);").unwrap();
        let x = numeric(&e, "x");
        assert_eq!(x[0], 0.);
        assert!((x[1] / magnitude - 2f64.sqrt()).abs() < 1e-14, "{x:?}");
    }
    let tiny = f64::from_bits(1);
    e.variables.insert(
        "a".into(),
        matrust::Matrix::from_column_vec(1, 2, vec![tiny, tiny]).unwrap(),
    );
    e.execute("x=movmedian(a,2);").unwrap();
    assert_eq!(numeric(&e, "x"), vec![tiny, tiny]);
}

/// Reference answers checked for this fix round.
#[test]
fn ismembertol_all_indices_are_columns_and_zero_for_non_members() {
    let mut e = Engine::default();
    e.execute("[~,l]=ismembertol([1 9],[1 1 2],'OutputAllIndices',true);x=l{1};y=l{2};s=size(l{1});c=class(l{2});")
        .unwrap();
    assert_eq!(numeric(&e, "x"), vec![1., 2.]);
    assert_eq!(numeric(&e, "s"), vec![2., 1.]);
    assert_eq!(numeric(&e, "y"), vec![0.]);
    e.execute("s=size(l{2});").unwrap();
    assert_eq!(numeric(&e, "s"), vec![1., 1.]);
}

#[test]
fn histcounts2_automatic_edge_rule_is_chosen_per_axis() {
    let mut e = Engine::default();
    let int = vec![0.5, 1.5, 2.5, 3.5, 4.5];
    for (x, y, xe, ye) in [
        (
            "[1 2 3 4]",
            "[0.1 0.5 1.7 2.2]",
            int.clone(),
            vec![0., 2., 4.],
        ),
        (
            "[0.1 0.5 1.7 2.2]",
            "[1 2 3 4]",
            vec![0., 2., 4.],
            int.clone(),
        ),
        ("[1 2 3 4]", "[1 2 3 4]", int.clone(), int.clone()),
        (
            "[0.1 0.5 1.7 2.2]",
            "[0.3 0.9 1.1 2.5]",
            vec![0., 2., 4.],
            vec![0., 2., 4.],
        ),
    ] {
        e.execute(&format!("[~,xe,ye]=histcounts2({x},{y});"))
            .unwrap();
        assert_eq!(numeric(&e, "xe"), xe, "{x} {y}");
        assert_eq!(numeric(&e, "ye"), ye, "{x} {y}");
    }
}

#[test]
fn moving_extrema_budget_admits_two_million_elements() {
    let mut e = Engine::default();
    e.execute("x = movmax(zeros(2e6,1), 3); n = numel(x);")
        .unwrap();
    assert_eq!(numeric(&e, "n"), vec![2e6]);
}

#[test]
fn non_numeric_option_values_get_a_clear_error() {
    let mut e = Engine::default();
    let err = e.execute("x=isbanded(eye(2),'a',1);").unwrap_err();
    assert!(err.contains("nonnegative integer scalars"), "{err}");
    let err = e.execute("x=repelem([1 2],'a');").unwrap_err();
    assert!(!err.contains("array option"), "{err}");
}
