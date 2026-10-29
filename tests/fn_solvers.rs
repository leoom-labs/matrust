//! Replays the reference corpus of the iterative solvers (pcg, gmres, bicg,
//! bicgstab, bicgstabl, cgs, qmr, tfqmr, minres, symmlq, lsqr) through MatRust.
//! `tests/fixtures/fn/solvers-expected.txt` holds every case of
//! `tests/fixtures/fn/solvers-cases.txt`, where `LONG` marks `format long` cases:
//! `@@CASE <id>|<statement>`, the `evalc` console text, `@@CLASS <class> <size>`
//! of `x` (absent on error), `@@END`; an error case holds `ERR <message>`.
//!
//! Each case runs in a fresh engine. The console text must match byte for byte,
//! then the class and size of `x`; for an error, the message without MatRust's
//! `Line 1, column N: ` location. The reference's desktop wraps a warning in
//! `[\x08...]\x08` markup and adds `> In ...` stack lines that name the reference's
//! snippet path (cases 33, 34, 61, 62, 75); only the `Warning: ...` text is
//! compared. Cases compared with a relaxed rule are listed in `HARNESS`.
use matrust::Engine;

const CORPUS: &str = include_str!("fixtures/fn/solvers-expected.txt");
const CASE_LIST: &str = include_str!("fixtures/fn/solvers-cases.txt");

/// The reference's `gallery('poisson', 3)` is sparse and MatRust has no sparse type
/// (nor `gallery` or `ichol`): the same matrix as a dense literal, and its
/// zero-fill incomplete Cholesky factor (`ichol` default, computed in double
/// precision by `scratchpad/solvers-proto2.py`).
const POISSON: &str = "[4 -1 0 -1 0 0 0 0 0; -1 4 -1 0 -1 0 0 0 0; 0 -1 4 0 0 -1 0 0 0; -1 0 0 4 -1 0 -1 0 0; 0 -1 0 -1 4 -1 0 -1 0; 0 0 -1 0 -1 4 0 0 -1; 0 0 0 -1 0 0 4 -1 0; 0 0 0 0 -1 0 -1 4 -1; 0 0 0 0 0 -1 0 -1 4]";
const ICHOL: &str = "[2 0 0 0 0 0 0 0 0; -0.5 1.9364916731037085 0 0 0 0 0 0 0; 0 -0.5163977794943222 1.9321835661585918 0 0 0 0 0 0; -0.5 0 0 1.9364916731037085 0 0 0 0 0; 0 -0.5163977794943222 0 -0.5163977794943222 1.8618986725025255 0 0 0 0; 0 0 -0.5175491695067657 0 -0.5370861555295746 1.8557158507382856 0 0 0; 0 0 0 -0.5163977794943222 0 0 1.9321835661585918 0 0; 0 0 0 0 -0.5370861555295746 0 -0.5175491695067657 1.8557158507382856 0; 0 0 0 0 0 -0.5388756040436664 0 -0.5388756040436664 1.8491149685006467]";

/// Every number token within relative 1e-12 (absolute 1e-14), every other
/// token equal: the last bits of residuals near 1e-16 and of the last digit.
const NUMERIC: &str = "numbers within relative 1e-12 or absolute 1e-14, text equal";
/// As NUMERIC, for function handles whose body multiplies with the engine's `*`.
const HANDLE: &str = "anonymous A*v uses the engine's matrix product; numbers within relative 1e-12 or absolute 1e-14";
/// The Poisson matrix is sparse in the reference, whose sparse kernels round
/// differently from dense ones; `gallery`/`ichol` are replaced by literals.
const SPARSE: &str =
    "gallery/ichol replaced by dense literals; numbers within relative 1e-12 or absolute 1e-14";

/// Only warning markup/backtraces are removed; all remaining bytes stay exact.
const WARNING: &str = "warning markup and backtraces removed; warning and other text exact";
/// As WARNING, then numbers as NUMERIC: the residual of a stagnated solve is
/// rounding error, whose value depends on the operation order.
const WARNING_NUMERIC: &str =
    "warning markup and backtraces removed; numbers within relative 1e-12 or absolute 1e-14";

/// Cases compared with a relaxed rule, with the rule.
const HARNESS: &[(&str, &str)] = &[
    ("33", WARNING),
    ("34", WARNING),
    ("61", WARNING),
    ("62", WARNING),
    // Clean-room gmres (2026-10-27): flag 3 and iteration 3 as before; the
    // stagnated relative residual is 4.3e-16 instead of 4.7e-16.
    ("75", WARNING_NUMERIC),
    ("19", SPARSE),
    ("20", SPARSE),
    ("26", SPARSE),
    ("27", SPARSE),
    ("55", SPARSE),
    ("56", SPARSE),
    ("57", SPARSE),
    ("58", SPARSE),
    ("59", SPARSE),
    ("60", SPARSE),
    ("183", SPARSE),
    ("184", SPARSE),
    ("185", SPARSE),
    ("186", SPARSE),
    ("200", SPARSE),
    ("220", SPARSE),
    // The engine's own `*` (nalgebra) rounds differently from MKL's dgemv
    // inside these anonymous functions.
    ("125", HANDLE),
    ("142", HANDLE),
    ("199", HANDLE),
    ("219", HANDLE),
    // Last-bit differences in a residual near 1e-16 or a 15th decimal:
    // the reference's operation order is not fully reproduced.
    // Clean-room gmres (2026-10-27): the second Arnoldi residual estimate
    // ends in ...472 instead of ...473.
    ("54", NUMERIC),
    ("66", NUMERIC),
    ("122", NUMERIC),
    ("123", NUMERIC),
    ("124", NUMERIC),
    ("126", NUMERIC),
    ("148", NUMERIC),
    ("150", NUMERIC),
    // Clean-room QMR (Templates Fig. 2.8) and TFQMR (Freund) may round
    // the quasi-minimization rotations differently. Preserve text, shapes,
    // flags and counts; allow only the existing tight numeric tolerance.
    ("152", NUMERIC),
    ("155", NUMERIC),
    ("157", NUMERIC),
    ("159", NUMERIC),
    ("166", NUMERIC),
    ("168", NUMERIC),
    ("177", NUMERIC),
    ("187", NUMERIC),
    ("189", NUMERIC),
    // Clean-room Lanczos/rotation evaluation changes only final roundoff.
    ("191", NUMERIC),
    ("197", NUMERIC),
    ("198", NUMERIC),
    ("207", NUMERIC),
    ("209", NUMERIC),
    ("211", NUMERIC),
    ("212", NUMERIC),
    ("215", NUMERIC),
    ("216", NUMERIC),
    ("217", NUMERIC),
    ("218", NUMERIC),
    ("236", NUMERIC),
    ("237", NUMERIC),
    ("238", NUMERIC),
    ("239", NUMERIC),
    ("243", NUMERIC),
];

/// Cases that cannot pass for a reason outside this task, with the reason.
const KNOWN: &[(&str, &str)] = &[];

struct Case {
    id: String,
    statement: String,
    long: bool,
    text: String,
    class: Option<String>,
    error: Option<String>,
}

fn cases() -> Vec<Case> {
    let mut list = CASE_LIST
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'));
    let mut cases = Vec::new();
    let mut rest = CORPUS;
    while let Some(start) = rest.find("@@CASE ") {
        rest = &rest[start + "@@CASE ".len()..];
        let (header, after) = rest.split_once('\n').expect("case header line");
        let (id, statement) = header.split_once('|').expect("id|statement");
        let (body, after) = after.split_once("@@END\n").expect("case end marker");
        let listed = list.next().expect("case list line");
        let long = listed.starts_with("LONG ");
        assert_eq!(
            listed.trim_start_matches("LONG "),
            statement,
            "case list order"
        );
        let (text, class, error) = if let Some(message) = body.strip_prefix("ERR ") {
            (
                String::new(),
                None,
                Some(message.trim_end_matches('\n').to_string()),
            )
        } else {
            let (text, class) = body.split_once("@@CLASS ").expect("class line");
            (
                if HARNESS.contains(&(id, WARNING)) || HARNESS.contains(&(id, WARNING_NUMERIC)) {
                    plain_warnings(text)
                } else {
                    text.to_string()
                },
                Some(class.trim_end().to_string()),
                None,
            )
        };
        cases.push(Case {
            id: id.into(),
            statement: statement.into(),
            long,
            text,
            class,
            error,
        });
        rest = after;
    }
    cases
}

/// Drop the reference's `[\x08` `]\x08 ` warning markup and the `> In` stack lines.
fn plain_warnings(text: &str) -> String {
    let text = text.replace("]\u{8} ", "").replace("[\u{8}", "");
    text.lines()
        .filter(|line| !line.starts_with("> In ") && !line.starts_with("  In "))
        .map(|line| format!("{line}\n"))
        .collect()
}

fn unplaced(error: &str) -> &str {
    match error.split_once(": ") {
        Some((place, text)) if place.starts_with("Line ") && place.contains(", column ") => text,
        _ => error,
    }
}

fn console(engine: &mut Engine, source: &str) -> Result<String, String> {
    engine.execute(source).map(|result| {
        result
            .output
            .iter()
            .map(|entry| format!("{entry}\n"))
            .collect()
    })
}

fn number(token: &str) -> Option<f64> {
    token
        .trim_end_matches(['.', ','])
        .trim_start_matches('(')
        .parse()
        .ok()
}

fn numerically_equal(actual: &str, expected: &str) -> bool {
    let a: Vec<&str> = actual.split_whitespace().collect();
    let e: Vec<&str> = expected.split_whitespace().collect();
    a.len() == e.len()
        && a.iter().zip(&e).all(|(a, e)| {
            a == e
                || match (number(a), number(e)) {
                    (Some(x), Some(y)) => {
                        (x - y).abs() <= (1e-12 * x.abs().max(y.abs())).max(1e-14)
                    }
                    _ => false,
                }
        })
}

/// What the case gives in MatRust; `None` when it matches.
fn mismatch(case: &Case) -> Option<String> {
    let rule = HARNESS.iter().find(|(id, _)| *id == case.id).map(|r| r.1);
    let mut engine = Engine::default();
    if case.long {
        console(&mut engine, "format long").expect("format long");
    }
    let mut statement = case.statement.clone();
    if rule == Some(SPARSE) {
        statement = statement
            .replace("gallery('poisson', 3)", POISSON)
            .replace("ichol(P_sv)", ICHOL);
    }
    let result = console(&mut engine, &statement);
    match (&case.error, result) {
        (Some(expected), Err(error)) => {
            let actual = unplaced(&error);
            (actual != expected).then(|| format!("error {expected:?}, got {actual:?}"))
        }
        (Some(expected), Ok(_)) => Some(format!("error {expected:?}, got success")),
        (None, Err(error)) => Some(format!("success, got error {error:?}")),
        (None, Ok(text)) => {
            let same = match rule {
                None | Some(WARNING) => text == case.text,
                Some(_) => numerically_equal(&text, &case.text),
            };
            if !same {
                return Some(format!(
                    "console\n  expected {:?}\n  actual   {text:?}",
                    case.text
                ));
            }
            let Some(x) = engine.array("x") else {
                return Some("no variable x".into());
            };
            let actual = format!("{} [{} {}]", x.class(), x.rows(), x.cols());
            (Some(&actual) != case.class.as_ref())
                .then(|| format!("class/size {:?}, got {actual}", case.class))
        }
    }
}

#[test]
fn corpus_has_every_case() {
    let cases = cases();
    assert_eq!(cases.len(), 246);
    for (i, case) in cases.iter().enumerate() {
        assert_eq!(case.id, (i + 1).to_string());
    }
    for (id, _) in HARNESS.iter().chain(KNOWN) {
        assert!(cases.iter().any(|c| c.id == *id), "listed case {id} exists");
    }
}

#[test]
fn matches_reference() {
    let mut failures = Vec::new();
    let mut passed = 0;
    for case in cases() {
        if KNOWN.iter().any(|(id, _)| *id == case.id) {
            continue;
        }
        match mismatch(&case) {
            None => passed += 1,
            Some(e) => failures.push(format!("case {} `{}`: {e}", case.id, case.statement)),
        }
    }
    assert!(
        failures.is_empty(),
        "{passed} passed, {} failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(passed + KNOWN.len(), 246);
}

/// Not reference-checked: extra arguments reach function handles, and a
/// restarted gmres reports [outer inner].
#[test]
fn extra_arguments_and_gmres_counts() {
    let mut e = Engine::default();
    console(
        &mut e,
        "A = [4 1 0; 1 3 1; 0 1 2]; b = [1; 2; 3]; [x, f] = pcg(@(v, s) s * (A * v), b, 1e-10, 10, [], [], [], 2);",
    )
    .unwrap();
    let text = console(&mut e, "fprintf('%.15g %d', x(3), f)").unwrap();
    assert_eq!(text.trim(), "0.722222222222222 0", "{text}");
    console(&mut e, "[x, f, r, it] = gmres(A, b, 2, 1e-12, 4);").unwrap();
    let text = console(&mut e, "fprintf('%d ', size(it))").unwrap();
    assert_eq!(text.trim(), "1 2");
    let error = e.execute("[a, b, c, d, f, g] = pcg(A, b)").unwrap_err();
    assert!(error.contains("Too many output arguments."), "{error}");
}

/// Not reference-checked: a larger symmetric positive definite system converges
/// and cancellation-friendly loops stay within the iteration bound.
#[test]
fn larger_tridiagonal_system() {
    let mut e = Engine::default();
    console(
        &mut e,
        "n = 200; A = diag(4 * ones(n, 1)) + diag(-ones(n - 1, 1), 1) + diag(-ones(n - 1, 1), -1); b = ones(n, 1);",
    )
    .unwrap();
    for solver in [
        "pcg", "minres", "bicgstab", "cgs", "gmres", "qmr", "tfqmr", "bicg",
    ] {
        let statement = if solver == "gmres" {
            "[x, f] = gmres(A, b, 20, 1e-10, 20);".to_string()
        } else {
            format!("[x, f] = {solver}(A, b, 1e-10, 200);")
        };
        console(&mut e, &statement).unwrap();
        let text = console(&mut e, "fprintf('%d %.3g', f, norm(b - A * x) / norm(b))").unwrap();
        let mut parts = text.split_whitespace();
        assert_eq!(parts.next(), Some("0"), "{solver}: {text}");
        let relres: f64 = parts.next().unwrap().parse().unwrap();
        assert!(relres <= 1e-9, "{solver}: {text}");
    }
}

fn printed(statement: &str, show: &str) -> String {
    let mut e = Engine::default();
    console(&mut e, statement).unwrap_or_else(|error| panic!("{statement}: {error}"));
    console(&mut e, show).unwrap().trim().to_string()
}

fn failure(statement: &str) -> String {
    let error = Engine::default().execute(statement).expect_err(statement);
    unplaced(&error).to_string()
}

/// The reference's post-loop truncation reads initer before it has been assigned.
/// The reviewer supplied the control flow; the wording is not reference-checked.
#[test]
fn gmres_zero_outer_iterations_errors() {
    for lhs in ["x", "[x, f, r, it, rv]"] {
        let error = failure(&format!("{lhs} = gmres(eye(3), ones(3, 1), 2, [], 0);"));
        assert_eq!(error, "Undefined function or variable 'initer'.");
    }
    // The early initial-residual return still precedes the empty loop.
    assert_eq!(
        printed(
            "[x,f] = gmres(eye(3), zeros(3,1), 2, [], 0);",
            "fprintf('%g',f)"
        ),
        "0"
    );
}

/// Findings 2 and 9: lsqr checks the length of a callback's result, also for a
/// preconditioner, and operator calls pass the extra arguments on.
#[test]
fn lsqr_callbacks() {
    for statement in [
        "[x, f] = lsqr(eye(2), [1; 1], [], [], @(v, t) 1);",
        "[x, f] = lsqr(@(v, t) [v; 1], [1; 2]);",
    ] {
        assert_eq!(printed(statement, "fprintf('%g %g', f,norm(x))"), "2 0");
    }
    let text = printed(
        "[x, f] = lsqr(@(v, s, t) s * v, [1; 2], [], [], [], [], [], 2);",
        "fprintf('%g %g %g', f, x)",
    );
    assert_eq!(text, "0 0.5 1", "{text}");
}

/// Finding 3: a huge maxit on data that never converges is stopped by the
/// evaluation budget instead of growing the residual history without bound.
#[test]
fn unbounded_maxit_is_charged() {
    // Invalid RHS data exits before an operator is fed a NaN direction.
    assert_eq!(
        printed("[x, f] = pcg(1, NaN, [], 1e12);", "fprintf('%g',f)"),
        "1"
    );
    // Every solver ends: by its own breakdown test or by the budget.
    for solver in [
        "pcg",
        "bicg",
        "bicgstab",
        "bicgstabl",
        "cgs",
        "qmr",
        "tfqmr",
        "minres",
        "symmlq",
        "lsqr",
    ] {
        let statement = format!("[x, f] = {solver}(1, NaN, [], 1e12);");
        if let Err(error) = Engine::default().execute(&statement) {
            assert!(error.contains("evaluation budget"), "{statement}: {error}");
        }
    }
    let statement = "[x, f] = gmres(1, NaN, [], [], 1e12);";
    if let Err(error) = Engine::default().execute(statement) {
        assert!(error.contains("evaluation budget"), "{statement}: {error}");
    }
    let error = failure("[x, f] = gmres(@(v) v, ones(100000, 1), [], 1e-6, 100000);");
    assert!(error.contains("element limit"), "{error}");
}

/// Finding 4: the adjoint of the preconditioner M = M1*M2 applies M2' first,
/// so non-commuting factors still converge.
#[test]
fn lsqr_split_preconditioner() {
    let text = printed(
        "A = [4 1; 2 5]; b = [1; 2]; [x, f, r, it] = lsqr(A, b, 1e-10, 20, [1 1; 0 1], [1 0; 1 1]);",
        "fprintf('%d %d %d', f, it < 20, norm(A * x - b) < 1e-8)",
    );
    assert_eq!(text, "0 1 1", "{text}");
}

/// Finding 5: with a preconditioner minres still reports the Euclidean
/// relative residual of the returned x.
#[test]
fn minres_preconditioned_relres_is_euclidean() {
    let text = printed(
        "A = diag([2 3]); b = [1; 1]; [x, f, r] = minres(A, b, 1e-6, 1, 100 * eye(2));",
        "fprintf('%d %.6f %.6f', f, r, norm(b - A * x) / norm(b))",
    );
    assert_eq!(text, "1 0.196116 0.196116", "{text}");
}

/// Fix round 1: a NaN right-hand side stops before the first iteration, so
/// the one-output call prints the flag-1 message instead of failing.
#[test]
fn pcg_nan_rhs_prints_stop_message() {
    let mut e = Engine::default();
    let text = console(&mut e, "x = pcg([4 1 0;1 3 1;0 1 2], [NaN;1;1]);").unwrap();
    assert!(
        text.starts_with(
            "pcg stopped at iteration 0 without converging to the desired tolerance 1e-06"
        ),
        "{text}"
    );
    assert!(
        text.contains("The iterate returned (number 0) has relative residual NaN."),
        "{text}"
    );
}

/// Fix round 1: without any iteration there is no residual history and no
/// iteration count to print, so the one-output call reports that.
#[test]
fn zero_maxit_has_no_history_or_iteration_count() {
    for solver in ["pcg", "minres", "symmlq", "lsqr"] {
        let text = printed(
            &format!("A=[4 1 0;1 3 1;0 1 2]; b=[1;2;3]; [x,f,r,it,rv]={solver}(A,b,1e-10,0);"),
            "fprintf('%g %g %d %d', f, it, numel(rv), norm(x))",
        );
        assert_eq!(text, "1 0 0 0", "{solver}: {text}");
        assert_eq!(
            failure(&format!(
                "x = {solver}([4 1 0;1 3 1;0 1 2], [1;2;3], 1e-10, 0);"
            )),
            "In 'MATLAB:itermsg:IterationInteger', parameter {1} must be a scalar.",
            "{solver}"
        );
    }
}

/// Fix round 1: with a preconditioner minres's sixth output holds the
/// residual norms of the CG points (Paige and Saunders 1975, section 6): they
/// decrease on this SPD system and equal symmlq's, apart from minres's
/// reserved zero in the third slot.
#[test]
fn minres_preconditioned_cg_residuals_decrease_and_match_symmlq() {
    let text = printed(
        "A=[4 1 0;1 3 1;0 1 2]; b=[1;2;3]; M=diag([4 3 2]); [~,~,~,~,~,rm]=minres(A,b,1e-10,10,M); [~,~,~,~,~,rs]=symmlq(A,b,1e-10,10,M);",
        "fprintf('%d %d %d %d %d', numel(rm), rm(1)>rm(2) && rm(2)>rm(4) && rm(4)>rm(5), rm(3)==0, norm(rm([1 2 4])-rs([1 2 3]))<1e-12, rm(5)<1e-12)",
    );
    assert_eq!(text, "5 1 1 1 1", "{text}");
}

/// Fix round 1: a preconditioner with r'*inv(M)*r = 0 is not positive definite.
#[test]
fn minres_symmlq_zero_energy_preconditioner_is_flag_five() {
    for solver in ["minres", "symmlq"] {
        let text = printed(
            &format!("[x,f]={solver}(eye(2),[1;1],[],[],@(v) [v(1);-v(2)]);"),
            "fprintf('%g %g', f, norm(x))",
        );
        assert_eq!(text, "5 0", "{solver}: {text}");
    }
}

/// Fix round 1: a start that already satisfies the normal equations keeps its
/// true relative residual (the zero start reports 0, see the test below).
#[test]
fn lsqr_orthogonal_residual_with_nonzero_start() {
    let text = printed(
        "[x, f, r, it] = lsqr([1; 0], [3; 1], [], [], [], [], 3);",
        "fprintf('%g %g %.12g %g', x, f, r, it)",
    );
    assert_eq!(text, "3 0 0.316227766017 0", "{text}");
}

/// Fix round 1: an overflowing first alpha is reported at iteration 0.
#[test]
fn lsqr_overflowing_first_alpha_prints_stop_message() {
    let mut e = Engine::default();
    let text = console(&mut e, "x = lsqr(1.5e308*ones(1,3), 1);").unwrap();
    assert!(
        text.starts_with("lsqr stopped at iteration 0 without converging"),
        "{text}"
    );
    assert!(
        text.contains("a scalar quantity became too small or too large"),
        "{text}"
    );
}

/// Finding 6: a residual orthogonal to the columns of A leaves x as it is.
#[test]
fn lsqr_orthogonal_residual_uses_zero_solution_message() {
    let text = printed(
        "[x, f, r, it, rv] = lsqr([1; 0], [0; 1]);",
        "fprintf('%g %g %g %d %d', x, f, r, it, numel(rv))",
    );
    assert_eq!(text, "0 0 0 0 1", "{text}");
    let mut e = Engine::default();
    let text = console(&mut e, "x = lsqr([1; 0], [0; 1]);").unwrap();
    assert_eq!(
        text.lines().next(),
        Some("The right hand side vector is all zero so lsqr")
    );
}

/// Finding 7: symmlq's first CG point is the solution when the Krylov space
/// is exhausted after one step.
#[test]
fn symmlq_exhausted_krylov_space() {
    let text = printed(
        "[x, f, r, it] = symmlq(eye(2), [1; 0]);",
        "fprintf('%g %g %g %g', x, f, it)",
    );
    assert_eq!(text, "1 0 0 1", "{text}");
    let mut e = Engine::default();
    let text = console(&mut e, "x = symmlq(eye(2), [1; 0]);").unwrap();
    assert_eq!(
        text.lines().next(),
        Some("symmlq converged at iteration 1 to a solution with relative residual 0.")
    );
}

/// Finding 10: a function handle is limited by its vectors, not by an
/// imagined n-by-n matrix.
#[test]
fn handle_operator_size() {
    let text = printed(
        "[x, f] = pcg(@(v) v, ones(5000, 1));",
        "fprintf('%d %g', f, norm(x - 1))",
    );
    assert_eq!(text, "0 0", "{text}");
}

/// Rev2 findings 1 and 2: infinity reaches the initial-residual return;
/// NaN relres selects itermsg's all-zero text without replacing x0.
#[test]
fn infinite_rhs_preserves_initial_guess_and_residual() {
    assert_eq!(
        printed(
            "[x,f,r,it,rv] = pcg(eye(3), [Inf;1;1], [], [], [], [], [1;1;1]);",
            "fprintf('%g %g %g %g %g', norm(x-1), f, isnan(r), it, isinf(rv))"
        ),
        "0 0 1 0 1"
    );
    assert_eq!(
        printed(
            "[x,f,r,it,rv] = pcg(eye(3), [Inf;1;1]);",
            "fprintf('%g %g %g %g %g', norm(x), f, isnan(r), it, isinf(rv))"
        ),
        "0 0 1 0 1"
    );
    let mut e = Engine::default();
    let text = console(&mut e, "x=pcg(eye(3),[Inf;1;1],[],[],[],[],[1;1;1]);").unwrap();
    assert!(text.starts_with("The right hand side vector is all zero so pcg\nreturned an all zero solution without iterating."), "{text}");
}

/// Reference behavior (checked 2026-10-24): a zero RHS bypasses the handle;
/// otherwise LSQR probes it before validating the preconditioner dimensions.
#[test]
fn lsqr_setup_probes_operator_before_preconditioner_validation() {
    assert_eq!(
        printed(
            "R=eye(3); x=lsqr(@(v) R*v, zeros(3,1));",
            "fprintf('%g %g', numel(x),norm(x))"
        ),
        "3 0"
    );
    for statement in [
        "[x,f]=lsqr(@(v) v,ones(3,1),[],[],eye(2));",
        "[x,f]=lsqr(@(v) v,ones(3,1),[],[],[],[],ones(2,1));",
    ] {
        assert_eq!(
            failure(statement),
            "user supplied function ==> @(v)v failed with the following error:\n \n Too many input arguments."
        );
    }
}

/// Rev2 finding 4: tiny updates relative to x trigger three-step stagnation.
#[test]
fn lsqr_stagnation_and_breakdown_flags() {
    assert_eq!(
        printed(
            "[x,f,r,it]=lsqr(diag([1 2 3 4 0]),[1;1;1;1;0],1e-12,10,[],[],[0;0;0;0;1e30]);",
            "fprintf('%g %g', f,it)"
        ),
        "3 3"
    );
    assert_eq!(
        printed("[x,f]=lsqr(1e308*ones(2),[1;0]);", "fprintf('%g', f)"),
        "4"
    );
}

/// Rev2 finding 6: inline-truncated histories keep the preallocated first entry.
#[test]
fn zero_iterations_keep_initial_residual_for_inline_histories() {
    for solver in ["bicgstab", "bicgstabl", "tfqmr"] {
        assert_eq!(
            printed(
                &format!("[x,f,r,it,rv]={solver}([4 1;2 5],[3;4],[],0);"),
                "fprintf('%g %g %g %g %g',norm(x),f,r,it,rv)"
            ),
            "0 1 1 0 5",
            "{solver}"
        );
    }
}

/// Rev2 finding 7: every preconditioner solve is checked immediately.
#[test]
fn qmr_rejects_nonfinite_preconditioner_results() {
    for m in ["@(v,t) v*Inf", "@(v,t) v*NaN"] {
        for factors in [format!("{m},[]"), format!("[],{m}")] {
            assert_eq!(
                printed(
                    &format!("[x,f,r,it,rv]=qmr([4 1;2 5],[1;2],[],5,{factors});"),
                    "fprintf('%g %g %g %g %g',norm(x),f,r,it,numel(rv))"
                ),
                "0 2 1 0 1"
            );
        }
    }
    for factors in ["@bad,[]", "[],@bad"] {
        let mut e = Engine::default();
        console(
            &mut e,
            &format!("[x,f,r,it,rv]=qmr([4 1;2 5],[1;2],[],5,{factors}); function y=bad(v,t), persistent calls; if isempty(calls), calls=0; end; calls=calls+1; if calls==3, y=v*Inf; else, y=v; end; end"),
        )
        .unwrap();
        assert_eq!(
            console(&mut e, "fprintf('%g %g',f,it)").unwrap().trim(),
            "2 0"
        );
    }
}

/// Rev2 finding 8: output arity cannot change a stateful operator's call stream.
#[test]
fn minres_operator_calls_independent_of_output_arity() {
    let mut results = Vec::new();
    for lhs in ["x", "[x,f,r,it,rv]", "[x,f,r,it,rv,cg]"] {
        let mut e = Engine::default();
        console(&mut e, &format!("global calls; calls=0; {lhs}=minres(@counted,[1;2;3],1e-10,5); function y=counted(v), global calls; calls=calls+1; y=[4 1 0;1 3 1;0 1 2]*v; end")).unwrap();
        results.push(console(&mut e, "fprintf('%.15g ',calls,x)").unwrap());
    }
    assert_eq!(results[0], results[1]);
    assert_eq!(results[1], results[2]);
}

/// Rev2 finding 9: user help contains no internal test or corpus identifiers.
#[test]
fn solver_help_has_no_test_identifiers() {
    for solver in [
        "pcg",
        "gmres",
        "bicg",
        "bicgstab",
        "bicgstabl",
        "cgs",
        "qmr",
        "tfqmr",
        "minres",
        "symmlq",
        "lsqr",
    ] {
        let mut e = Engine::default();
        let help = console(&mut e, &format!("help {solver}")).unwrap();
        assert!(!help.contains("tests/fn_solvers.rs"));
        assert!(!help.contains("case 192"));
    }
}

/// Release-only timing evidence; run separately with one and four workers.
#[test]
#[ignore]
fn benchmark_round2_solvers() {
    let workers: usize = std::env::var("SLV_BENCH_WORKERS").unwrap().parse().unwrap();
    matrust::initialize_worker_pool(workers).unwrap();
    for solver in ["minres", "lsqr", "qmr"] {
        let mut e = Engine::default();
        e.execute(
            "n=1000; A=diag(4*ones(n,1))+diag(-ones(n-1,1),1)+diag(-ones(n-1,1),-1); b=ones(n,1);",
        )
        .unwrap();
        let command = if solver == "qmr" {
            format!("[x,f,r,it]={solver}(A,b,1e-10,20,@(v,t) v);")
        } else {
            format!("[x,f,r,it]={solver}(A,b,1e-10,20);")
        };
        e.execute(&command).unwrap();
        let start = std::time::Instant::now();
        for _ in 0..5 {
            e.execute(&command).unwrap();
        }
        eprintln!(
            "{solver}: workers={workers}, n=1000 dense, 5 repeats, {:.6} seconds",
            start.elapsed().as_secs_f64()
        );
    }
}

/// Rev3's examples are reviewer-derived regressions, not new reference evidence.
#[test]
fn gmres_failed_second_preconditioner_preserves_iterate() {
    for factors in ["@bad,[]", "[],@bad"] {
        let mut e = Engine::default();
        console(&mut e, &format!("[x,f,r,it,rv]=gmres(eye(2),ones(2,1),[],[],2,{factors}); function y=bad(v), persistent calls; if isempty(calls), calls=0; end; calls=calls+1; if calls==2, y=Inf*ones(size(v)); else, y=v; end; end")).unwrap();
        assert_eq!(
            console(&mut e, "fprintf('%g ',f,norm(x),r,it,numel(rv))")
                .unwrap()
                .trim(),
            "2 0 1 0 0 1"
        );
    }
}

#[test]
fn minres_scaled_preconditioner_uses_euclidean_tolerance() {
    assert_eq!(
        printed(
            "[x,f,r,it]=minres(diag([2 3]),[1;1],1e-6,2,1e12*eye(2));",
            "fprintf('%g %g %g %g',f,it,norm(x-[0.5;1/3])<1e-12,r<1e-6)"
        ),
        "0 2 1 1"
    );
}

#[test]
fn lsqr_bounds_both_scratch_vector_spaces() {
    // The input is valid at MAX_ELEMENTS; rejection precedes x0/scratch allocation.
    for statement in [
        "A=zeros(1,16777216); [x,f]=lsqr(A,1);",
        "A=zeros(16777216,1); b=ones(16777216,1); [x,f]=lsqr(A,b);",
    ] {
        assert_eq!(
            failure(statement),
            "Iterative solver problem exceeds the element limit."
        );
    }
}

#[test]
fn callback_results_must_be_columns() {
    assert_eq!(
        printed("[x,f]=pcg(@(v) v',[1;2]);", "fprintf('%g %g',f,norm(x))"),
        "2 0"
    );
    for solver in [
        "pcg",
        "gmres",
        "bicg",
        "bicgstab",
        "bicgstabl",
        "cgs",
        "qmr",
        "tfqmr",
        "minres",
        "symmlq",
        "lsqr",
    ] {
        let two_way = matches!(solver, "bicg" | "qmr" | "lsqr");
        let args = if two_way { "v,t" } else { "v" };
        let options = if solver == "gmres" { "[],[],5" } else { "[],5" };
        for bad in [
            "v'",
            "1",
            "[v;v]",
            "Inf*ones(size(v))",
            "NaN*ones(size(v))",
            "char(98)",
        ] {
            let handle = format!("@({args}) {bad}");
            let statements = [
                format!("[x,f]={solver}({handle},[1;2],{options});"),
                format!("[x,f]={solver}(eye(2),[1;2],{options},{handle});"),
                format!("[x,f]={solver}(eye(2),[1;2],{options},[],{handle});"),
            ];
            for statement in statements {
                assert_eq!(
                    printed(&statement, "fprintf('%g %g',f,norm(x))"),
                    "2 0",
                    "{statement}"
                );
            }
        }
    }
}

#[test]
fn tfqmr_initial_singular_preconditioner_is_flag_two() {
    assert_eq!(
        printed(
            "[x,f,r,it,rv]=tfqmr(1,1,[],1,0);",
            "fprintf('%g ',f,x,r,it,rv)"
        ),
        "2 0 1 0 1"
    );
}

#[test]
fn bicgstabl_failed_solution_preconditioner_preserves_iterate() {
    let mut e = Engine::default();
    console(&mut e,"[x,f,r,it,rv]=bicgstabl(1,1,[],2,@bad); function y=bad(v), persistent calls; if isempty(calls), calls=0; end; calls=calls+1; if calls==2, y=Inf*ones(size(v)); else, y=v; end; end").unwrap();
    assert_eq!(
        console(&mut e, "fprintf('%g ',f,x,r,it,rv)")
            .unwrap()
            .trim(),
        "2 0 1 0 1"
    );
}

#[test]
fn warning_normalization_is_enumerated_and_exact() {
    let ids: Vec<_> = HARNESS
        .iter()
        .filter(|(_, r)| *r == WARNING || *r == WARNING_NUMERIC)
        .map(|(id, _)| *id)
        .collect();
    assert_eq!(ids, ["33", "34", "61", "62", "75"]);
    for case in cases().iter().filter(|c| ids.contains(&c.id.as_str())) {
        assert!(case.text.contains("Warning:"));
        assert_eq!(mismatch(case), None, "{}", case.id);
        let changed = Case {
            text: case.text.replace("Warning:", "Warning: changed"),
            id: case.id.clone(),
            statement: case.statement.clone(),
            long: case.long,
            class: case.class.clone(),
            error: case.error.clone(),
        };
        assert!(mismatch(&changed).is_some(), "{}", case.id);
    }
}

/// A failed first factor must never reach a second factor that would mask it.
#[test]
fn split_preconditioners_stop_before_masking_invalid_values() {
    for solver in [
        "pcg",
        "gmres",
        "bicg",
        "bicgstab",
        "bicgstabl",
        "cgs",
        "qmr",
        "tfqmr",
        "minres",
        "symmlq",
        "lsqr",
    ] {
        let args = if matches!(solver, "bicg" | "qmr" | "lsqr") {
            "v,t"
        } else {
            "v"
        };
        let options = if solver == "gmres" { "[],[],5" } else { "[],5" };
        let statement = format!(
            "global masked; masked=0; [x,f]={solver}(diag([2 3]),[1;2],{options},@({args}) Inf*ones(size(v)),@mask); function y=mask({args}), global masked; masked=masked+1; y=ones(size(v)); end"
        );
        // LSQR applies the adjoint factors in reverse order, so M2 is called once.
        let expected = if solver == "lsqr" { "2 0 1" } else { "2 0 0" };
        assert_eq!(
            printed(&statement, "fprintf('%g %g %g',f,norm(x),masked)"),
            expected,
            "{solver}"
        );
    }
}

/// Sweep actual call positions, including transpose and final-residual calls.
#[test]
fn every_operator_and_preconditioner_call_checks_its_result() {
    for solver in [
        "pcg",
        "gmres",
        "bicg",
        "bicgstab",
        "bicgstabl",
        "cgs",
        "qmr",
        "tfqmr",
        "minres",
        "symmlq",
        "lsqr",
    ] {
        let args = if matches!(solver, "bicg" | "qmr" | "lsqr") {
            "v,t"
        } else {
            "v"
        };
        let options = if solver == "gmres" {
            "[],1e-10,3"
        } else {
            "1e-10,3"
        };
        for target in ["A", "M1", "M2"] {
            let call = match target {
                "A" => format!("{solver}(@checked,[1;2;3],{options})"),
                "M1" => format!("{solver}(diag([2 3 4]),[1;2;3],{options},@checked)"),
                _ => format!("{solver}(diag([2 3 4]),[1;2;3],{options},[],@checked)"),
            };
            let body = if target == "A" {
                "diag([2 3 4])*v"
            } else {
                "v"
            };
            let program = |fail: usize, bad: &str| {
                format!(
                    "global calls; calls=0; [x,f,r,it,rv]={call}; function y=checked({args}), global calls; calls=calls+1; if calls=={fail}, y={bad}; else, y={body}; end; end"
                )
            };
            let count: usize = printed(&program(0, "v"), "fprintf('%g',calls)")
                .parse()
                .unwrap();
            for fail in 1..=count {
                // LSQR's first adjoint result discovers n, so any column
                // length is valid at that call; subsequent lengths are fixed.
                let bad_values = if solver == "lsqr" && target == "A" && fail == 1 {
                    &["Inf*ones(size(v))", "v'"][..]
                } else {
                    &["Inf*ones(size(v))", "v'", "[v;v]"][..]
                };
                for bad in bad_values {
                    let result = printed(
                        &program(fail, bad),
                        "fprintf('%g %g %g %g %g',f,all(isfinite(x)),calls,abs(norm([1;2;3]-diag([2 3 4])*x)-r*norm([1;2;3]))<1e-10,abs(rv(end)-r*norm([1;2;3]))<1e-10)",
                    );
                    assert_eq!(
                        result,
                        format!("2 1 {fail} 1 1"),
                        "{solver} {target} call {fail}/{count}: {bad}"
                    );
                }
            }
        }
    }
}

#[test]
fn failed_later_solve_retains_nonzero_last_iterate() {
    let mut e = Engine::default();
    console(&mut e, "A=diag([2 3]); b=[1;2]; [expected,unused]=pcg(A,b,1e-12,1); [x,f,r,it]=pcg(A,b,1e-12,2,@bad); function y=bad(v), persistent calls; if isempty(calls), calls=0; end; calls=calls+1; if calls==2, y=Inf*ones(size(v)); else, y=v; end; end").unwrap();
    assert_eq!(
        console(&mut e, "fprintf('%g %g %g',f,it,norm(x-expected))")
            .unwrap()
            .trim(),
        "2 1 0"
    );
}

#[test]
#[ignore]
fn benchmark_round3_solvers() {
    let workers: usize = std::env::var("SLV_BENCH_WORKERS").unwrap().parse().unwrap();
    matrust::initialize_worker_pool(workers).unwrap();
    for solver in [
        "pcg",
        "gmres",
        "bicg",
        "bicgstab",
        "bicgstabl",
        "cgs",
        "qmr",
        "tfqmr",
        "minres",
        "symmlq",
        "lsqr",
    ] {
        let mut e = Engine::default();
        e.execute(
            "n=512; A=diag(4*ones(n,1))+diag(-ones(n-1,1),1)+diag(-ones(n-1,1),-1); b=ones(n,1);",
        )
        .unwrap();
        let args = if matches!(solver, "bicg" | "qmr" | "lsqr") {
            "v,t"
        } else {
            "v"
        };
        let options = if solver == "gmres" {
            "[],1e-10,40"
        } else {
            "1e-10,40"
        };
        let command = format!("[x,f]={solver}(A,b,{options},@({args}) v);");
        e.execute(&command).unwrap();
        let start = std::time::Instant::now();
        for _ in 0..5 {
            e.execute(&command).unwrap();
        }
        eprintln!(
            "{solver}: workers={workers}, n=512 dense, 5 repeats, {:.6} seconds",
            start.elapsed().as_secs_f64()
        );
        assert_eq!(
            console(&mut e, "fprintf('%g',f)").unwrap().trim(),
            "0",
            "{solver}"
        );
    }
}

/// Release-only timing of one mid-size gmres solve; run with SLV_BENCH_WORKERS=1 and 4.
#[test]
#[ignore]
fn benchmark_gmres_mid_size() {
    let workers: usize = std::env::var("SLV_BENCH_WORKERS").unwrap().parse().unwrap();
    matrust::initialize_worker_pool(workers).unwrap();
    for (label, operator) in [("dense", "A"), ("handle", "@(v) A*v")] {
        let mut e = Engine::default();
        e.execute("n=2000; A=diag(2.05*ones(n,1))+diag(-ones(n-1,1),1)+diag(-0.95*ones(n-1,1),-1); b=ones(n,1);").unwrap();
        let command = format!("[x,f,r,it]=gmres({operator},b,50,1e-10,4);");
        e.execute(&command).unwrap();
        let start = std::time::Instant::now();
        for _ in 0..5 {
            e.execute(&command).unwrap();
        }
        eprintln!(
            "gmres-{label}: workers={workers}, n=2000 dense, restart 50 x 4 cycles, 5 repeats, {:.6} seconds",
            start.elapsed().as_secs_f64()
        );
        assert_eq!(
            console(&mut e, "fprintf('%g',f)").unwrap().trim(),
            "1",
            "{label}"
        );
    }
}

#[test]
fn gmres_failure_after_restart_keeps_completed_iterate() {
    let mut e = Engine::default();
    console(&mut e, "A=diag([2 3]); b=[1;2]; [expected,unused,expected_r,expected_it]=gmres(A,b,1,1e-12,1); [x,f,r,it]=gmres(A,b,1,1e-12,3,@bad); function y=bad(v), persistent calls; if isempty(calls), calls=0; end; calls=calls+1; if calls==4, y=Inf*ones(size(v)); else, y=v; end; end").unwrap();
    assert_eq!(
        console(
            &mut e,
            "fprintf('%g %g %g %g',f,norm(x-expected),norm(it-expected_it),abs(r-expected_r)<1e-14)"
        )
        .unwrap()
        .trim(),
        "2 0 0 1"
    );
}

// Round 4: reviewer examples, independent of the saved MATLAB corpus.
#[test]
fn round4_rectangular_lsqr_validates_column_space_once() {
    for factors in ["diag([2 3]),[]", "[],diag([2 3])", "diag([2 3]),eye(2)"] {
        for x0 in ["[0;0]", "[1;1]"] {
            let setup = format!(
                "R=[1 2;3 4;5 6]; op={{R,R'}}; af=@(v,t) op{{1+strcmp(t,'transp')}}*v; [xd,fd,rd]=lsqr(R,[1;2;4],1e-10,10,{factors},{x0}); [x,f,r]=lsqr(af,[1;2;4],1e-10,10,{factors},{x0});"
            );
            assert_eq!(
                printed(
                    &setup,
                    "fprintf('%g %g %g', f==fd,norm(x-xd)<1e-10,abs(r-rd)<1e-10)"
                ),
                "1 1 1"
            );
        }
    }
}

#[test]
fn round4_transpose_flag_follows_extra_parameters() {
    for solver in ["bicg", "qmr", "lsqr"] {
        for factors in ["[],[]", "@(v,s,t) v/s,[]", "[],@(v,s,t) v/s"] {
            let command = format!("[x,f]= {solver}(@(v,s,t) s*v,[1;2],[],[],{factors},[],2);");
            assert_eq!(
                printed(&command, "fprintf('%g %g',f,norm(x-[0.5;1])<1e-12)"),
                "0 1"
            );
        }
    }
}

#[test]
fn round5_tfqmr_floors_odd_half_step() {
    assert_eq!(
        printed(
            "[x,f,r,it,rv]=tfqmr(2*eye(3),[1;2;3]);",
            "fprintf('%g %g %g %g',f,r,it,numel(rv))"
        ),
        "0 0 0 2"
    );
    let mut e = Engine::default();
    let text = console(&mut e, "x=tfqmr(2*eye(3),[1;2;3]);").unwrap();
    assert_eq!(
        text.trim(),
        "The initial guess has relative residual 0 which is within\nthe desired tolerance 1e-06 so tfqmr returned it without iterating."
    );
}

#[test]
fn round4_symmlq_setup_breakdown_retains_best_initial_point() {
    for (a, b, x0) in [
        ("[2 1;1 3]", "[1;0.1]", "[0;0]"),
        ("[2 1 0;1 -3 1;0 1 1]", "[1;2;3]", "[0;0;0]"),
    ] {
        let factor = if b == "[1;0.1]" {
            "diag([1 -1])"
        } else {
            "diag([1 -1 1])"
        };
        let cmd = format!("[x,f,r,it]=symmlq({a},{b},[],[],{factor},[],{x0});");
        assert_eq!(
            printed(&cmd, "fprintf('%g %g %g %g',f,r,it,norm(x))"),
            "5 1 0 0"
        );
        let mut e = Engine::default();
        let text = console(&mut e, &format!("x=symmlq({a},{b},[],[],{factor});")).unwrap();
        assert!(text.starts_with("symmlq stopped at iteration 0"), "{text}");
        assert!(
            text.contains("because the preconditioner is not symmetric positive definite."),
            "{text}"
        );
    }
    assert_eq!(
        printed(
            "[x,f,r,it]=symmlq([2 1 0;1 -3 1;0 1 1],[1;2;3],[],0);",
            "fprintf('%g %g %g %g',f,r,it,norm(x))"
        ),
        "1 1 0 0"
    );
}

#[test]
fn round4_lsqr_probe_failure_uses_tolerance_and_initial_residual() {
    let mut e = Engine::default();
    let text = console(&mut e, "x=lsqr(@(v,t) Inf*v,[1;2],1e-10);").unwrap();
    assert!(text.contains("desired tolerance 1e-10"), "{text}");
    assert_eq!(
        printed(
            "op={2*eye(2),NaN*eye(2)}; af=@(v,t) op{1+strcmp(t,'transp')}*v; [x,f,r,it]=lsqr(af,[1;2],1e-10,[],[],[],[1;1]);",
            "fprintf('%g %g %g',f,r,it)"
        ),
        "2 0.447214 0"
    );
    let text=console(&mut Engine::default(),"op={2*eye(2),NaN*eye(2)}; af=@(v,t) op{1+strcmp(t,'transp')}*v; x=lsqr(af,[1;2],1e-10,[],[],[],[1;1]);").unwrap();
    assert!(text.contains("relative residual 0.45."), "{text}");
}

#[test]
fn round4_restarted_gmres_initial_guess_uses_plain_label() {
    let text = console(
        &mut Engine::default(),
        "A=[4 1 0;1 3 1;0 1 2]; x=gmres(A,A*[1;1;1],2,1e-10,3,[],[],[1;1;1]);",
    )
    .unwrap();
    assert!(
        text.contains("so gmres returned it without iterating."),
        "{text}"
    );
}

#[test]
fn round5_failure_messages_split_loop_and_returned_indices() {
    for solver in ["pcg", "bicg", "cgs", "bicgstab", "bicgstabl"] {
        let text = console(
            &mut Engine::default(),
            &format!("x={solver}([4 1;1 3],[1;2],[],[],zeros(2));"),
        )
        .unwrap();
        assert!(
            text.starts_with(&format!("{solver} stopped at iteration 1 without")),
            "{text}"
        );
        assert_eq!(
            printed(
                &format!("[x,f,r,it]={solver}([4 1;1 3],[1;2],[],[],zeros(2));"),
                "fprintf('%g %g',f,it)"
            ),
            "2 0"
        );
        assert!(
            text.contains("The iterate returned (number 0) has relative residual 1."),
            "{text}"
        );
    }
    let text=console(&mut Engine::default(),"x=pcg([4 1 0;1 3 1;0 1 2],[1;2;3],[],5,@bad); function y=bad(v), persistent calls; if isempty(calls), calls=0; end; calls=calls+1; if calls==3, y=Inf*v; else, y=v; end; end").unwrap();
    assert!(
        text.starts_with("pcg stopped at iteration 3 without"),
        "{text}"
    );
    assert!(text.contains("The iterate returned (number 2)"), "{text}");
    let text=console(&mut Engine::default(),"x=gmres([4 1;1 3],[1;2],1,1e-10,3,@bad); function y=bad(v), persistent calls; if isempty(calls), calls=0; end; calls=calls+1; if calls==2, y=Inf*v; else, y=v; end; end").unwrap();
    assert!(
        text.starts_with("gmres(1) stopped at outer iteration 1 (inner iteration 1)"),
        "{text}"
    );
    assert!(
        text.contains("The iterate returned (number 0(0)) has relative residual 1."),
        "{text}"
    );
}

/// Reference facts, checked 2026-10-24.
/// Compare the supplied four significant digits; the final residual is at
/// roundoff and retains the corpus's absolute 1e-14 allowance.
#[test]
fn round5_minres_cg_residual_uses_running_residual_with_x0() {
    for (x0, residuals, cg_residuals) in [
        (
            "zeros(3,1)",
            [3.742_f64, 3., 1.557, 9.155e-16],
            [3.742_f64, 3.493, 0., 3.170, 3.742],
        ),
        (
            "S\\d+[1e-3;0;0]",
            [0.002236, 0.001414, 0.0001622, 6.28e-16],
            [0.002236, 0.001528, 0., 0.002224, 0.002236],
        ),
    ] {
        let mut e = Engine::default();
        console(&mut e, &format!(
            "S=[2 1 0;1 -3 1;0 1 1]; d=[1;2;3]; [x,f,r,it,rv,rvc]=minres(S,d,1e-12,10,[],[],{x0});"
        )).unwrap();
        assert_eq!(
            console(&mut e, "fprintf('%g %g',f,it)").unwrap().trim(),
            "0 3"
        );
        for (variable, expected) in [("rv", &residuals[..]), ("rvc", &cg_residuals[..])] {
            let values = console(&mut e, &format!("fprintf('%.16g ',{variable})")).unwrap();
            let actual: Vec<f64> = values
                .split_whitespace()
                .map(|s| s.parse().unwrap())
                .collect();
            assert_eq!(actual.len(), expected.len(), "{x0}: {variable}");
            for (&actual, &expected) in actual.iter().zip(expected) {
                let tolerance = if expected == 0. {
                    0.
                } else {
                    (0.5 * 10f64.powf(expected.abs().log10().floor() - 3.)).max(1e-14)
                };
                assert!(
                    (actual - expected).abs() <= tolerance,
                    "{x0}: {variable}: {actual:e} != {expected:e} (tolerance {tolerance:e})"
                );
            }
        }
    }
}

#[test]
fn round5_minres_cg_zero_cosine_is_infinite() {
    assert_eq!(
        printed(
            "[x,f,r,it,rv,rvc]=minres([0 1;1 0],[1;0],1e-12,2);",
            "fprintf('%g %g %g %g',f,it,isinf(rvc(2)),norm(x-[0;1])<1e-12)"
        ),
        "0 2 1 1"
    );
}

#[test]
fn round5_lsqr_reuses_setup_residual() {
    for x0 in ["[0;0]", "[0.1;0.2]"] {
        let mut e = Engine::default();
        console(&mut e, &format!(
            "global order; order=[]; A=diag([2 3]); b=[1;2]; [xd,fd,rd,itd,rvd]=lsqr(A,b,1e-12,1,[],[],{x0}); [x,f,r,it,rv]=lsqr(@op,b,1e-12,1,[],[],{x0}); function y=op(v,t), global order; order=[order strcmp(t,'transp')]; y=diag([2 3])*v; end"
        )).unwrap();
        assert_eq!(
            console(&mut e, "fprintf('%g ',order)").unwrap().trim(),
            "0 1 1 0 1",
            "{x0}"
        );
        assert_eq!(console(&mut e,
            "fprintf('%g %g %g %g %g',norm(x-xd)<1e-12,f==fd,abs(r-rd)<1e-12,it==itd,norm(rv-rvd)<1e-12)"
        ).unwrap().trim(), "1 1 1 1 1", "{x0}");
    }
}

#[test]
#[ignore]
fn benchmark_round5_changed_paths() {
    let workers: usize = std::env::var("SLV_BENCH_WORKERS").unwrap().parse().unwrap();
    matrust::initialize_worker_pool(workers).unwrap();
    for (label, command) in [
        (
            "minres-x0",
            "[x,f,r,it,rv,rvc]=minres(A,b,1e-10,40,[],[],x0);",
        ),
        (
            "lsqr-handle-x0",
            "[x,f,r,it,rv]=lsqr(@(v,t) A*v,b,1e-10,40,[],[],x0);",
        ),
    ] {
        let mut e = Engine::default();
        e.execute("n=512; A=diag(4*ones(n,1))+diag(-ones(n-1,1),1)+diag(-ones(n-1,1),-1); b=ones(n,1); x0=0.1*ones(n,1);").unwrap();
        e.execute(command).unwrap();
        let start = std::time::Instant::now();
        for _ in 0..5 {
            e.execute(command).unwrap();
        }
        eprintln!(
            "{label}: workers={workers}, n=512 dense, 5 repeats, {:.6} seconds",
            start.elapsed().as_secs_f64()
        );
        assert_eq!(
            console(&mut e, "fprintf('%g',f)").unwrap().trim(),
            "0",
            "{label}"
        );
    }
}
