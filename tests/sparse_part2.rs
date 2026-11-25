//! Sparse storage, part 2: square solves without converting to full.
//!
//! Expected values: exact fractions, or the dense solver on full(A) (whose
//! values are checked against Octave elsewhere) at relative 1e-10, since the
//! sparse factorizations order and pivot differently.
use matrust::{Array, Engine};

fn engine() -> Engine {
    Engine::default()
}
fn run(e: &mut Engine, source: &str) -> String {
    match e.execute(source) {
        Ok(result) => result.output.join("\n"),
        Err(error) => panic!("{source}: {error}"),
    }
}
fn error(e: &mut Engine, source: &str) -> String {
    match e.execute(source) {
        Ok(_) => panic!("{source}: expected an error"),
        Err(error) => match error.split_once(": ") {
            Some((location, rest)) if location.starts_with("Line ") => rest.to_owned(),
            _ => error,
        },
    }
}
fn is_sparse(e: &Engine, name: &str) -> bool {
    matches!(e.array(name), Some(Array::Sparse(_)))
}
fn var(e: &Engine, name: &str) -> Vec<f64> {
    match e.array(name).unwrap_or_else(|| panic!("{name} missing")) {
        Array::Numeric(m) => m.data().to_vec(),
        Array::Sparse(s) => s.to_dense(&Default::default()).unwrap().data().to_vec(),
        other => panic!("{name}: {other:?}"),
    }
}
fn scalar(e: &Engine, name: &str) -> f64 {
    let v = var(e, name);
    assert_eq!(v.len(), 1, "{name}");
    v[0]
}
fn close(actual: &[f64], expected: &[f64], rel: f64) {
    assert_eq!(actual.len(), expected.len());
    let scale = expected
        .iter()
        .fold(0f64, |m, v| m.max(v.abs()))
        .max(1e-300);
    for (a, b) in actual.iter().zip(expected) {
        assert!((a - b).abs() <= rel * scale, "{actual:?} vs {expected:?}");
    }
}
/// The 2-D five-point Poisson matrix on a k-by-k grid, built with spdiags.
const POISSON: &str = "N = k*k; e = ones(N,1); s = e; s(k:k:N) = 0; w = [0; s(1:end-1)]; \
     A = spdiags([-e -s 4*e -w -e], [-k -1 0 1 k], N, N);";

#[test]
fn unsymmetric_values_with_symmetric_pattern_use_lu() {
    // Symmetric pattern, positive diagonal, unequal values: not Cholesky.
    let mut e = engine();
    run(&mut e, "x = sparse([2 1; 0.5 2]) \\ [1; 1];");
    assert!(!is_sparse(&e, "x"));
    close(&var(&e, "x"), &[2. / 7., 3. / 7.], 1e-15);
}

#[test]
fn every_path_matches_the_full_solver() {
    let mut e = engine();
    for a in [
        "[2 0 0; 1 3 0; 0 1 4]",  // lower triangular
        "[2 1 0; 0 3 1; 0 0 4]",  // upper triangular
        "[4 1 0; 1 4 1; 0 1 4]",  // Cholesky
        "[1 2 0; 2 1 3; 0 3 1]",  // symmetric indefinite: Cholesky fails, LU
        "[0 2 1; 3 0 1; 1 1 0]",  // zero diagonal, unsymmetric: LU
        "[1 2 3; 4 5 6; 7 8 10]", // dense pattern
    ] {
        run(
            &mut e,
            &format!(
                "F = {a}; S = sparse(F); b = [1 2; 3 4; 5 6]; x = S \\ b; xf = F \\ b; \
                 y = b.' / S; yf = b.' / F;"
            ),
        );
        assert!(!is_sparse(&e, "x") && !is_sparse(&e, "y"), "{a}");
        close(&var(&e, "x"), &var(&e, "xf"), 1e-10);
        close(&var(&e, "y"), &var(&e, "yf"), 1e-10);
    }
}

#[test]
fn sparse_right_hand_sides_give_sparse_results() {
    let mut e = engine();
    run(
        &mut e,
        "S = sparse([4 1 0; 1 4 1; 0 1 4]); B = speye(3); X = S \\ B; Y = B / S; \
         Xf = full(S) \\ eye(3); Yf = eye(3) / full(S); L = logical(sparse([1 0 0])).'; z = S \\ L;",
    );
    assert!(is_sparse(&e, "X") && is_sparse(&e, "Y") && is_sparse(&e, "z"));
    close(&var(&e, "X"), &var(&e, "Xf"), 1e-10);
    close(&var(&e, "Y"), &var(&e, "Yf"), 1e-10);
}

#[test]
fn poisson_solves_under_a_tiny_full_limit() {
    // A conversion to full would fail at once: 4 elements allowed.
    for (k, chol) in [(100, true), (40, false)] {
        let mut e = engine();
        e.set_sparse_full_limit(4);
        run(&mut e, &format!("k = {k}; {POISSON} b = ones(N,1);"));
        if !chol {
            // One unsymmetric value: LU.
            run(&mut e, "A(1,2) = -0.5;");
        }
        run(
            &mut e,
            "x = A \\ b; r = norm(A*x - b) / norm(b); y = b.' / A; q = norm(y*A - b.') / norm(b);",
        );
        assert!(scalar(&e, "r") < 1e-12, "k={k}: {}", scalar(&e, "r"));
        assert!(scalar(&e, "q") < 1e-12, "k={k}: {}", scalar(&e, "q"));
        assert_eq!(
            e.array("x").map(|a| a.dims().to_vec()),
            Some(vec![k * k, 1])
        );
    }
}

#[test]
fn singular_and_non_finite_systems_follow_the_full_limit() {
    let mut e = engine();
    // Within the limit: the full solver's values and warning.
    run(
        &mut e,
        "S = sparse([1 2 0; 2 4 0; 0 0 1]); lastwarn(''); x = S \\ [1; 2; 3]; [~, id] = lastwarn; \
         lastwarn(''); xf = full(S) \\ [1; 2; 3];",
    );
    assert_eq!(run(&mut e, "disp(id)").trim(), "MATLAB:singularMatrix");
    let (x, xf) = (var(&e, "x"), var(&e, "xf"));
    assert_eq!(format!("{x:?}"), format!("{xf:?}"));
    run(
        &mut e,
        "T = sparse([1 0; 1 0]); t = T \\ [1; 1]; tf = full(T) \\ [1; 1];",
    );
    assert_eq!(
        format!("{:?}", var(&e, "t")),
        format!("{:?}", var(&e, "tf"))
    );
    run(
        &mut e,
        "N = sparse([1 NaN; 0 1]); n1 = N \\ [1; 1]; n2 = full(N) \\ [1; 1];",
    );
    assert_eq!(
        format!("{:?}", var(&e, "n1")),
        format!("{:?}", var(&e, "n2"))
    );
    // Triangular: substitution keeps a NaN/Inf to the components that use
    // it (Octave: speye(2)\\[Inf;1] is [Inf;1]), also above the limit.
    run(&mut e, "I = speye(2); i1 = I \\ [Inf; 1];");
    assert_eq!(var(&e, "i1"), vec![f64::INFINITY, 1.]);
    // Above the limit: errors, never a conversion.
    e.set_sparse_full_limit(4);
    assert_eq!(
        error(&mut e, "x = S \\ [1; 2; 3];"),
        "mldivide: matrix is singular to working precision (sparse LU found no pivot)"
    );
    assert_eq!(
        error(&mut e, "x = [1 2 3] / sparse([1 0 0; 0 0 0; 0 0 1]);"),
        "mrdivide: matrix is singular to working precision (zero on the diagonal)"
    );
    assert_eq!(
        error(&mut e, "x = sparse([1 NaN 0; 0 1 0; 0 0 1]) \\ [1; 1; 1];"),
        "mldivide: sparse matrices with NaN or Inf values are not supported in a solve yet"
    );
    run(&mut e, "i3 = speye(3) \\ [1; Inf; 1];");
    assert_eq!(var(&e, "i3"), vec![1., f64::INFINITY, 1.]);
    // Non-triangular with a non-finite right-hand side: dense path or error.
    assert_eq!(
        error(&mut e, "x = sparse([2 1 0; 1 2 0; 1 0 1]) \\ [1; Inf; 1];"),
        "mldivide: right-hand sides with NaN or Inf values are not supported for sparse matrices yet"
    );
}

#[test]
fn rcond_warnings_use_the_dense_identifiers() {
    let mut e = engine();
    e.set_sparse_full_limit(4);
    for a in [
        "sparse([1 0; 0 1e-20])",
        "sparse([1 2 0; 2 4+1e-15 0; 0 0 1])",
        "sparse([1 1 0; 1 1+1e-15 0; 0 1 2])",
    ] {
        run(
            &mut e,
            &format!("lastwarn(''); x = {a} \\ ones(size({a},1),1); [~, id] = lastwarn;"),
        );
        assert_eq!(
            run(&mut e, "disp(id)").trim(),
            "MATLAB:nearlySingularMatrix",
            "{a}"
        );
    }
    let text = run(&mut e, "x = sparse([1 0; 0 1e-20]) \\ [1; 1];");
    assert!(
        text.contains("Warning: Matrix is close to singular"),
        "{text}"
    );
    // warning('off', id) silences it (lastwarn is still set, as for full input).
    let text = run(
        &mut e,
        "warning('off', 'MATLAB:nearlySingularMatrix'); x = sparse([1 0; 0 1e-20]) \\ [1; 1];",
    );
    assert!(!text.contains("Warning"), "{text}");
    run(&mut e, "warning('on', 'MATLAB:nearlySingularMatrix');");
    // Well conditioned: no warning.
    run(
        &mut e,
        "lastwarn(''); x = sparse([4 1; 1 4]) \\ [1; 1]; m = lastwarn;",
    );
    assert_eq!(run(&mut e, "disp(isempty(m))").trim(), "1");
}

#[test]
fn shapes_dimensions_and_scalars() {
    let mut e = engine();
    e.set_sparse_full_limit(4);
    assert_eq!(
        error(&mut e, "x = speye(3) \\ [1; 2];"),
        "Linear system row dimensions must agree"
    );
    assert_eq!(
        error(&mut e, "x = [1 2] / speye(3);"),
        "Matrix dimensions must agree"
    );
    // 1-by-1 sparse divisors keep the scalar rules; empty right-hand sides the dense ones.
    run(
        &mut e,
        "a = sparse(2) \\ [2 4]; b = zeros(3, 0); c = speye(3) \\ b;",
    );
    assert_eq!(var(&e, "a"), vec![1., 2.]);
    assert_eq!(e.array("c").map(|a| a.dims().to_vec()), Some(vec![3, 0]));
}

/// All five outputs of `call` with sparse operands, and with `full` ones.
fn solver_outputs(e: &mut Engine, call: &str, full_call: &str) -> (String, String) {
    run(e, &format!("[x1, f1, r1, i1, v1] = {call};"));
    run(e, &format!("[x2, f2, r2, i2, v2] = {full_call};"));
    let pick = |e: &Engine, k: u8| {
        ["x", "f", "r", "i", "v"]
            .iter()
            .map(|n| format!("{:?}", var(e, &format!("{n}{k}"))))
            .collect::<Vec<_>>()
            .join(" | ")
    };
    (pick(e, 1), pick(e, 2))
}

#[test]
fn iterative_solvers_on_sparse_operands_match_full_bit_for_bit() {
    let mut e = engine();
    run(
        &mut e,
        "k = 6; N = k*k; e1 = ones(N,1); s = e1; s(k:k:N) = 0; w = [0; s(1:end-1)]; \
         S = spdiags([-e1 -s 4*e1 -w -e1], [-k -1 0 1 k], N, N); F = full(S); \
         U = S; U(2,1) = 0.3; UF = full(U); b = (1:N)'; \
         L = sparse(tril(F)); LF = full(L); R = L'; RF = full(R); D = spdiags(4*e1, 0, N, N); DF = full(D);",
    );
    let cases = [
        ("pcg(S, b, 1e-10, 100)", "pcg(F, b, 1e-10, 100)"),
        (
            "pcg(S, b, 1e-10, 100, L, R)",
            "pcg(F, b, 1e-10, 100, LF, RF)",
        ),
        (
            "minres(S, b, 1e-10, 100, D)",
            "minres(F, b, 1e-10, 100, DF)",
        ),
        ("symmlq(S, b, 1e-10, 100)", "symmlq(F, b, 1e-10, 100)"),
        (
            "gmres(U, b, 10, 1e-10, 20, L)",
            "gmres(UF, b, 10, 1e-10, 20, LF)",
        ),
        (
            "bicg(U, b, 1e-10, 100, L, R)",
            "bicg(UF, b, 1e-10, 100, LF, RF)",
        ),
        (
            "bicgstab(U, b, 1e-10, 100, D)",
            "bicgstab(UF, b, 1e-10, 100, DF)",
        ),
        (
            "bicgstabl(U, b, 1e-10, 100)",
            "bicgstabl(UF, b, 1e-10, 100)",
        ),
        ("cgs(U, b, 1e-10, 100, L)", "cgs(UF, b, 1e-10, 100, LF)"),
        (
            "qmr(U, b, 1e-10, 100, L, R)",
            "qmr(UF, b, 1e-10, 100, LF, RF)",
        ),
        ("tfqmr(U, b, 1e-10, 100, D)", "tfqmr(UF, b, 1e-10, 100, DF)"),
        (
            "lsqr(U(:, 1:30), b, 1e-10, 100)",
            "lsqr(UF(:, 1:30), b, 1e-10, 100)",
        ),
        // A handle for A with a sparse preconditioner.
        (
            "pcg(@(x) S*x, b, 1e-10, 100, L, R)",
            "pcg(@(x) F*x, b, 1e-10, 100, LF, RF)",
        ),
    ];
    for (sparse_call, full_call) in cases {
        let (a, b) = solver_outputs(&mut e, sparse_call, full_call);
        assert_eq!(a, b, "{sparse_call}");
    }
    // A non-triangular sparse preconditioner is factorized once (LU): close, not bit-identical.
    run(
        &mut e,
        "M = S + spdiags(e1, 1, N, N); MF = full(M); \
         [y1, g1] = gmres(U, b, [], 1e-12, 50, M); [y2, g2] = gmres(UF, b, [], 1e-12, 50, MF);",
    );
    close(&var(&e, "y1"), &var(&e, "y2"), 1e-9);
    assert_eq!(scalar(&e, "g1"), 0.);
}

#[test]
fn iterative_solvers_never_convert_a_large_sparse_system() {
    let mut e = engine();
    e.set_sparse_full_limit(4);
    run(
        &mut e,
        &format!("k = 60; {POISSON} b = ones(N,1); D = spdiags(4*e, 0, N, N);"),
    );
    // Separate statements: Engine::default() has the browser's per-statement budget.
    run(
        &mut e,
        "[x, flag, relres, iter] = pcg(A, b, 1e-8, 1000, D);",
    );
    run(&mut e, "r = norm(A*x - b) / norm(b);");
    run(&mut e, "y = gmres(A, b, 20, 1e-8, 100);");
    run(&mut e, "q = norm(A*y - b) / norm(b);");
    assert_eq!(scalar(&e, "flag"), 0.);
    assert!(scalar(&e, "r") < 1e-8);
    assert!(scalar(&e, "q") < 1e-7);
    // One output prints the convergence message.
    let text = run(&mut e, "z = pcg(A, b, 1e-8, 1000);");
    assert!(text.contains("pcg converged"), "{text}");
}

// ------------------------------------------------- ichol / ilu (piece 4)

/// Matrices of tests/fixtures/sparse_precond/cases.m (generator of octave.txt).
const PRECOND_SETUP: &str = "m = 5; e = ones(m,1);
T = spdiags([-e 2*e -e],-1:1,m,m); I = speye(m);
P = kron(I,T) + kron(T,I);
T1 = spdiags([-1.3*e 4*e -0.7*e],-1:1,m,m); T2 = spdiags([-0.6*e 0*e -1.1*e],-1:1,m,m);
A = kron(I,T1) + kron(T2,I);
p = [3 1 4 15 9 2 6 25 5 8 7 10 11 12 13 14 16 17 18 19 20 21 22 23 24];
B = A(p,:);";

fn octave_factors() -> std::collections::HashMap<String, Vec<f64>> {
    let mut out = std::collections::HashMap::new();
    let mut current: Option<String> = None;
    for line in include_str!("fixtures/sparse_precond/octave.txt").lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        match t.as_slice() {
            [tag, _] if tag.parse::<f64>().is_err() => {
                current = Some(tag.to_string());
                out.insert(tag.to_string(), vec![0.; 625]);
            }
            [i, j, v] => {
                let (i, j): (usize, usize) = (i.parse().unwrap(), j.parse().unwrap());
                let m = out.get_mut(current.as_ref().unwrap()).unwrap();
                m[(j - 1) * 25 + i - 1] = v.parse().unwrap();
            }
            _ => {}
        }
    }
    out
}

#[test]
fn ichol_and_ilu_match_octave_for_every_type() {
    let reference = octave_factors();
    let cases: &[(&str, &str, &str)] = &[
        ("ic0", "R = ichol(P);", "R"),
        ("mic0", "o = struct('michol','on'); R = ichol(P, o);", "R"),
        (
            "ict",
            "o = struct('type','ict','droptol',0.05); R = ichol(P, o);",
            "R",
        ),
        (
            "mict",
            "o = struct('type','ict','droptol',0.05,'michol','on'); R = ichol(P, o);",
            "R",
        ),
        (
            "icup",
            "o = struct('diagcomp',0.1,'shape','upper'); R = ichol(P, o);",
            "R",
        ),
        ("ilu0L", "[L,U] = ilu(A);", "L"),
        ("ilu0U", "[L,U] = ilu(A);", "U"),
        ("ilu0row", "R = ilu(A, struct('milu','row'));", "R"),
        ("ilu0col", "R = ilu(A, struct('milu','col'));", "R"),
        (
            "crL",
            "s = struct('type','crout','droptol',0.05); [L,U] = ilu(A, s);",
            "L",
        ),
        ("crU", "[L,U] = ilu(A, s);", "U"),
        (
            "cr0",
            "R = ilu(A, struct('type','crout','droptol',0));",
            "R",
        ),
        (
            "crrow",
            "R = ilu(A, struct('type','crout','droptol',0.05,'milu','row'));",
            "R",
        ),
        (
            "crcol",
            "R = ilu(A, struct('type','crout','droptol',0.05,'milu','col'));",
            "R",
        ),
        (
            "tpL",
            "s = struct('type','ilutp','droptol',0.05); [L,U,Q] = ilu(B, s);",
            "L",
        ),
        ("tpU", "[L,U,Q] = ilu(B, s);", "U"),
        ("tpP", "[L,U,Q] = ilu(B, s);", "Q"),
        ("tp5L", "s.thresh = 0.5; [L,U] = ilu(B, s);", "L"),
        ("tp5U", "[L,U] = ilu(B, s);", "U"),
        (
            "tp0L",
            "s.droptol = 0; s.thresh = 1; [L,U,Q] = ilu(B, s);",
            "L",
        ),
        ("tp0U", "[L,U,Q] = ilu(B, s);", "U"),
        (
            "tpcol",
            "R = ilu(B, struct('type','ilutp','droptol',0.05,'milu','col'));",
            "R",
        ),
    ];
    let mut e = engine();
    run(&mut e, PRECOND_SETUP);
    for (tag, statement, name) in cases {
        run(&mut e, statement);
        assert!(is_sparse(&e, name), "{tag}");
        let actual = var(&e, name);
        let expected = &reference[*tag];
        // Same pattern, values at relative 1e-12 (own code, no library).
        for (k, (a, b)) in actual.iter().zip(expected).enumerate() {
            assert_eq!(*a == 0., *b == 0., "{tag}: pattern differs at {k}");
        }
        close(&actual, expected, 1e-12);
    }
    // milu keeps the documented sums of L*U equal to those of A.
    run(
        &mut e,
        "E = ones(25,1); o = struct('michol','on'); L = ichol(P, o); r1 = norm(L*L'*E - P*E);
         s = struct('type','crout','droptol',0.05,'milu','row'); [L,U] = ilu(A, s); r2 = norm(L*U*E - A*E);
         s.milu = 'col'; [L,U] = ilu(A, s); r3 = norm(E'*(L*U) - E'*A);
         s = struct('type','ilutp','droptol',0.05,'milu','col'); [L,U,Q] = ilu(B, s); r4 = norm(E'*(L*U) - E'*Q*B);",
    );
    for r in ["r1", "r2", "r3", "r4"] {
        assert!(scalar(&e, r) < 1e-12, "{r}");
    }
}

#[test]
fn ichol_and_ilu_errors_and_empty_input() {
    let mut e = engine();
    assert_eq!(
        error(&mut e, "ichol(eye(3))"),
        "ichol: A must be a sparse square matrix"
    );
    assert_eq!(
        error(&mut e, "ilu(sparse(ones(2,3)))"),
        "ilu: A must be a sparse square matrix"
    );
    assert_eq!(
        error(&mut e, "ichol(sparse([1 2; 2 1]))"),
        "ichol: encountered a nonpositive pivot"
    );
    assert_eq!(
        error(&mut e, "ilu(sparse([0 1; 1 0]))"),
        "ilu: encountered a pivot equal to 0"
    );
    assert_eq!(
        error(&mut e, "ilu(sparse([0 1; 1 0]), struct('type','crout'))"),
        "ilu: encountered a pivot equal to 0"
    );
    // ilutp pivots the zero diagonal away.
    run(
        &mut e,
        "[L,U,Q] = ilu(sparse([0 1; 1 0]), struct('type','ilutp'));",
    );
    close(&var(&e, "Q"), &[0., 1., 1., 0.], 0.);
    assert_eq!(
        error(&mut e, "ichol(speye(2), struct('type','ilutp'))"),
        "ichol: TYPE must be \"nofill\" or \"ict\""
    );
    assert_eq!(
        error(&mut e, "ilu(speye(2), struct('bogus',1))"),
        "ilu: unknown option 'bogus'"
    );
    assert_eq!(
        error(&mut e, "ilu(speye(2), struct('droptol',-1))"),
        "ilu: DROPTOL must be a non-negative real scalar"
    );
    assert_eq!(
        error(&mut e, "[a,b] = ichol(speye(2));"),
        "ichol: too many output arguments"
    );
    assert_eq!(
        error(&mut e, "ichol(speye(2), 3)"),
        "ichol: OPTS must be a structure"
    );
    run(
        &mut e,
        "L = ichol(sparse(0,0)); [L2,U2,P2] = ilu(sparse(0,0)); n = nnz(L) + numel(U2);",
    );
    assert!(is_sparse(&e, "L") && is_sparse(&e, "P2"));
    assert_eq!(scalar(&e, "n"), 0.);
    // W = ilu(A) is L+U-speye; shape 'upper' gives R with R'*R = A on the pattern.
    run(
        &mut e,
        "A = sparse([4 -1 0; -1 4 -1; 0 -1 4]); [L,U] = ilu(A); d = full(ilu(A) - (L+U-speye(3)));
         R = ichol(A, struct('shape','upper')); r = norm(full(R'*R - A));",
    );
    close(&var(&e, "d"), &[0.; 9], 0.);
    assert!(scalar(&e, "r") < 1e-14);
}

#[test]
fn preconditioned_poisson_never_converts_to_full() {
    // 100x100 grid; Octave 10.3: pcg+ichol 79 iterations (8.099e-09), plain pcg 187,
    // gmres(20)+ilu outer 1 inner 7 (1.599e-09).
    let mut e = engine();
    e.set_sparse_full_limit(4);
    run(
        &mut e,
        "A = gallery('poisson',100); b = ones(10000,1); L = ichol(A);
         [x,flag,relres,iter] = pcg(A,b,1e-8,500,L,L'); r = norm(A*x-b)/norm(b);
         m = 100; e1 = ones(m,1); I = speye(m);
         T1 = spdiags([-1.3*e1 4*e1 -0.7*e1],-1:1,m,m); T2 = spdiags([-0.6*e1 2*e1 -1.1*e1],-1:1,m,m);
         C = kron(I,T1) + kron(T2,I); [L2,U2] = ilu(C);
         [y,flag2,relres2,iter2] = gmres(C,b,20,1e-8,50,L2,U2);
         W = ilu(C, struct('type','crout','droptol',1e-3)); [L3,U3,P3] = ilu(C, struct('type','ilutp','droptol',1e-3));",
    );
    assert!(is_sparse(&e, "A") && is_sparse(&e, "L") && is_sparse(&e, "C") && is_sparse(&e, "W"));
    assert_eq!(scalar(&e, "flag"), 0.);
    assert!((scalar(&e, "iter") - 79.).abs() <= 2.);
    assert!(scalar(&e, "r") < 1e-8);
    assert_eq!(scalar(&e, "flag2"), 0.);
    assert_eq!(var(&e, "iter2"), vec![1., 7.]);
    assert!(is_sparse(&e, "P3"));
}

#[test]
fn kron_and_gallery_build_sparse_matrices() {
    let mut e = engine();
    run(
        &mut e,
        "T = gallery('tridiag',4); a = issparse(T) + issparse(kron(T, eye(2))) + issparse(kron(eye(2), T));
         d = norm(full(kron(T, [1 2; 3 4])) - kron(full(T), [1 2; 3 4]));
         P = gallery('poisson',4); d2 = norm(full(P - (kron(speye(4),T) + kron(T,speye(4)))));
         G = full(gallery('tridiag',[1 2],[3 4 5],[6 7])); H = full(gallery('tridiag',3,1,2,3));",
    );
    assert_eq!(scalar(&e, "a"), 3.);
    assert_eq!(scalar(&e, "d"), 0.);
    assert_eq!(scalar(&e, "d2"), 0.);
    close(&var(&e, "G"), &[3., 1., 0., 6., 4., 2., 0., 7., 5.], 0.);
    close(&var(&e, "H"), &[2., 1., 0., 3., 2., 1., 0., 3., 2.], 0.);
}

#[test]
fn ilu_scaled_norms_and_ilutp_diagonal_after_swaps() {
    // Octave 10.3 values (step review, 2026-11-04).
    let mut e = engine();
    run(
        &mut e,
        "A = 1e200*sparse([1 1;1 2]); [L,U] = ilu(A,struct('type','crout','droptol',0.1)); U = U/1e200;
         [L2,U2] = ilu(A,struct('type','ilutp','droptol',0.1)); U2 = U2/1e200;
         B = sparse([0 1 1;10 1 1;1 1.6 10]); [L3,U3,P3] = ilu(B,struct('type','ilutp','thresh',0.5));
         [L4,U4,P4] = ilu(sparse([0 1;1 0]),struct('type','ilutp','thresh',0,'udiag',1,'droptol',0.1));",
    );
    for (l, u) in [("L", "U"), ("L2", "U2")] {
        close(&var(&e, l), &[1., 1., 0., 1.], 1e-15);
        close(&var(&e, u), &[1., 0., 1., 1.], 1e-15);
    }
    // P3 rows [2 1 3]: the current diagonal 1 >= 0.5*1.5 stays the pivot after the first swap.
    close(&var(&e, "P3"), &[0., 1., 0., 1., 0., 0., 0., 0., 1.], 0.);
    close(
        &var(&e, "U3"),
        &[10., 0., 0., 1., 1., 0., 1., 1., 8.4],
        1e-14,
    );
    close(&var(&e, "U4"), &[0.1, 0., 1., -10.], 1e-14);
    close(&var(&e, "P4"), &[1., 0., 0., 1.], 0.);
    assert_eq!(
        error(
            &mut e,
            "ilu(sparse([0 1;1 0]),struct('type','ilutp','thresh',0))"
        ),
        "ilu: encountered a pivot equal to 0"
    );
}

#[test]
fn sparse_right_hand_sides_stay_sparse_on_the_dense_path() {
    let mut e = engine();
    // Octave 9: issparse 1, values [Inf; 1].
    run(
        &mut e,
        "X = speye(2) \\ sparse([Inf; 1]); Z = sparse([Inf 1]) / speye(2);",
    );
    assert!(is_sparse(&e, "X") && is_sparse(&e, "Z"));
    assert_eq!(var(&e, "X"), vec![f64::INFINITY, 1.]);
    assert_eq!(var(&e, "Z"), vec![f64::INFINITY, 1.]);
    // Singular sparse\sparse takes the dense path and stays sparse.
    run(&mut e, "lastwarn(''); Y = sparse([1 0; 0 0]) \\ speye(2);");
    assert!(is_sparse(&e, "Y"));
    // sparse\full stays full.
    run(&mut e, "F = sparse([1 2; 2 4]) \\ [1; 2];");
    assert!(!is_sparse(&e, "F"));
}

#[test]
fn iterative_solvers_take_sparse_b_and_x0() {
    let mut e = engine();
    // Octave 9: x = [1; 2], flag 0.
    run(&mut e, "[x, f] = pcg(eye(2), sparse([1; 2]));");
    assert_eq!(var(&e, "x"), vec![1., 2.]);
    assert_eq!(var(&e, "f"), vec![0.]);
    run(
        &mut e,
        "[y, g] = gmres(speye(2), [1; 2], [], sparse(1e-10), 5, [], [], sparse([0; 0]));",
    );
    let y = var(&e, "y");
    assert!(
        (y[0] - 1.).abs() < 1e-12 && (y[1] - 2.).abs() < 1e-12,
        "{y:?}"
    );
    assert_eq!(var(&e, "g"), vec![0.]);
}

#[test]
fn sparse_lu_fill_is_bounded_before_factoring() {
    let mut e = engine();
    // Random 12000x12000 with ~13 entries per column: the George-Ng bound is
    // above MAX_NNZ, so the solve errors at once instead of filling in.
    let text = error(
        &mut e,
        "rng(1); n = 12000; A = sprand(n, n, 0.001) + 10*speye(n); x = A \\ ones(n, 1);",
    );
    assert!(
        text.starts_with("mldivide: the sparse LU factors may need up to "),
        "{text}"
    );
    // A moderate random system still factors and solves.
    run(
        &mut e,
        "rng(2); A = sprand(2000, 2000, 0.002) + 5*speye(2000); x = A \\ ones(2000, 1); r = norm(A*x - 1, inf);",
    );
    assert!(var(&e, "r")[0] < 1e-10);
}
