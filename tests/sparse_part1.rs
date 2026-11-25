//! Sparse storage, part 1: constructors, the result-type table, indexing,
//! display, bytes, the sparse-to-full limit and MAT v5 class 5.
//!
//! Expected values come from GNU Octave 10.3 (tests/fixtures/sparse/expected.json
//! and the .mat files, written by tool/refs/sparse/make_refs.m) and from
//! MATLAB's published sparse rules where the design keeps a documented
//! difference (rows converted to full).
use matrust::{Array, Engine, sparse::SparseMatrix};
use serde_json::Value as Json;

const EXPECTED: &str = include_str!("fixtures/sparse/expected.json");

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
        // Drop the statement location the engine puts in front.
        Err(error) => match error.split_once(": ") {
            Some((location, rest)) if location.starts_with("Line ") => rest.to_owned(),
            _ => error,
        },
    }
}
fn sparse(e: &Engine, name: &str) -> SparseMatrix {
    match e.array(name) {
        Some(Array::Sparse(s)) => s,
        other => panic!("{name} is not sparse: {other:?}"),
    }
}
fn is_sparse(e: &Engine, name: &str) -> bool {
    matches!(e.array(name), Some(Array::Sparse(_)))
}
/// Shape and column-major values of any real array.
fn values(a: &Array) -> (usize, usize, Vec<f64>) {
    match a {
        Array::Numeric(m) => (m.rows(), m.cols(), m.data().to_vec()),
        Array::Logical(m) => (
            m.rows(),
            m.cols(),
            m.data().iter().map(|&v| f64::from(v)).collect(),
        ),
        Array::Sparse(s) => (
            s.rows(),
            s.cols(),
            s.to_dense(&Default::default()).unwrap().data().to_vec(),
        ),
        other => panic!("not a real array: {other:?}"),
    }
}
fn var(e: &Engine, name: &str) -> (usize, usize, Vec<f64>) {
    values(
        &e.array(name)
            .unwrap_or_else(|| panic!("no variable {name}")),
    )
}
fn same(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| x == y || (x.is_nan() && y.is_nan()))
}
/// Octave's jsonencode writes Inf and NaN as null (read as NaN here): an
/// expected NaN matches any non-finite value.
fn same_json(actual: &[f64], expected: &[f64]) -> bool {
    actual.len() == expected.len()
        && actual
            .iter()
            .zip(expected)
            .all(|(x, y)| x == y || (y.is_nan() && !x.is_finite()))
}
fn json_values(v: &Json) -> Vec<f64> {
    match v {
        Json::Array(items) => items.iter().flat_map(json_values).collect(),
        Json::Number(n) => vec![n.as_f64().unwrap()],
        Json::Bool(b) => vec![f64::from(u8::from(*b))],
        Json::Null => vec![f64::NAN],
        other => panic!("unexpected JSON {other}"),
    }
}
fn expected() -> Json {
    serde_json::from_str(EXPECTED).unwrap()
}
/// No stored entry is zero and row indices increase in every column.
fn assert_canonical(s: &SparseMatrix) {
    for c in 0..s.cols() {
        let range = s.column(c);
        for k in range.clone() {
            assert_ne!(s.value(k), 0., "explicit zero stored");
            if k > range.start {
                assert!(s.rowidx()[k] > s.rowidx()[k - 1]);
            }
        }
    }
}

const SETUP: &str = "P = sparse([1 0 2; 0 0 3]); Q = sparse([0 1 0; 4 0 0]); F = [1 2 3; 4 5 6]; L = sparse(logical([1 0 1; 0 0 1]));";

/// MatRust expressions for the Octave table rows (same names as make_refs.m).
const OPS: &[(&str, &str)] = &[
    ("S+S", "P+Q"),
    ("S-S", "P-Q"),
    ("S+F", "P+F"),
    ("F-S", "F-P"),
    ("S+2", "P+2"),
    ("2-S", "2-P"),
    ("S.*S", "P.*Q"),
    ("S.*F", "P.*F"),
    ("F.*S", "F.*P"),
    ("S*2", "P*2"),
    ("2*S", "2*P"),
    ("S.*2", "P.*2"),
    ("S/2", "P/2"),
    ("S./2", "P./2"),
    ("2\\S", "2\\P"),
    ("S*S'", "P*Q'"),
    ("S*F'", "P*F'"),
    ("F*S'", "F*P'"),
    ("S&S", "P&Q"),
    ("S&F", "P&F"),
    ("F&S", "F&P"),
    ("-S", "-P"),
    ("+S", "+P"),
    ("S'", "P'"),
    ("S.'", "P.'"),
    ("abs", "abs(-P)"),
    ("round", "round(P/3)"),
    ("floor", "floor(P/3)"),
    ("ceil", "ceil(-P/3)"),
    ("fix", "fix(-P/3)"),
    ("sqrt", "sqrt(P)"),
    ("sum", "sum(P)"),
    ("sum2", "sum(P, 2)"),
    ("sumL", "sum(L)"),
    ("sumall", "sum(P(:))"),
    ("max", "max(P)"),
    ("min", "min(P)"),
    ("max2", "max(P, [], 2)"),
    ("min2", "min(P, [], 2)"),
    ("any", "any(P)"),
    ("all", "all(P)"),
    ("any2", "any(P, 2)"),
    ("all2", "all(L, 2)"),
    ("double", "double(L)"),
    ("logical", "logical(P)"),
    ("[S,F]", "[P, F]"),
    ("[F;S]", "[F; P]"),
    ("[S,S]", "[P, Q]"),
    ("cat1", "cat(1, P, F)"),
    ("S(2,3)", "P(2,3)"),
    ("S(:,3)", "P(:,3)"),
    ("S(5)", "P(5)"),
    ("S(:)", "P(:)"),
    ("S(end,end)", "P(end,end)"),
    ("S(S>1)", "P(P > 1)"),
];
const DENSIFIED: &[(&str, &str)] = &[
    ("S./F", "P./F"),
    ("S.^2", "P.^2"),
    ("2.^S", "2.^P"),
    ("2./S", "2./P"),
    ("S|F", "P|F"),
    ("~S", "~P"),
    ("xor", "xor(P, Q)"),
    ("S==S", "P==Q"),
    ("S<F", "P<F"),
    ("maxSF", "max(P, F)"),
];

#[test]
fn result_table_matches_octave_class_and_values() {
    let reference = expected();
    let rows = reference["ops"].as_array().unwrap();
    assert_eq!(rows.len(), OPS.len());
    for (row, &(name, expr)) in rows.iter().zip(OPS) {
        assert_eq!(row["name"], name);
        let mut e = engine();
        run(&mut e, SETUP);
        run(&mut e, &format!("x = {expr};"));
        let a = e.array("x").unwrap();
        let sparse_expected = row["sparse"].as_bool().unwrap();
        assert_eq!(is_sparse(&e, "x"), sparse_expected, "{name}: sparse");
        assert_eq!(a.class(), row["class"].as_str().unwrap(), "{name}: class");
        let (_, _, data) = values(&a);
        assert!(
            same_json(&data, &json_values(&row["value"])),
            "{name}: {data:?} vs {}",
            row["value"]
        );
        if let Array::Sparse(s) = &a {
            assert_canonical(s);
        }
    }
}

#[test]
fn rows_converted_to_full_keep_octave_values() {
    // Octave keeps several of these sparse; MatRust documents full results.
    let reference = expected();
    let rows = reference["densified"].as_array().unwrap();
    assert_eq!(rows.len(), DENSIFIED.len());
    for (row, &(name, expr)) in rows.iter().zip(DENSIFIED) {
        assert_eq!(row["name"], name);
        let mut e = engine();
        run(&mut e, SETUP);
        run(&mut e, &format!("x = {expr};"));
        assert!(!is_sparse(&e, "x"), "{name} should be full");
        let a = e.array("x").unwrap();
        assert_eq!(a.class(), row["class"].as_str().unwrap(), "{name}: class");
        let (_, _, data) = values(&a);
        assert!(same_json(&data, &json_values(&row["value"])), "{name}");
    }
}

#[test]
fn scalar_operands_and_full_operands_choose_the_documented_storage() {
    let mut e = engine();
    run(&mut e, SETUP);
    // S±scalar and scalar±S are full; a 1x1 sparse counts as a scalar.
    for (expr, sparse_result) in [
        ("P + sparse(2)", false),
        ("sparse(2) - P", false),
        ("sparse(2) + sparse(3)", true),
        ("P * sparse(2)", true),
        ("sparse(2) * P", true),
        ("sparse(2) * F", false),
        ("F * sparse(2)", false),
        ("sparse(2) .* F", true),
        ("sparse([1 0; 0 2]) \\ [1; 2]", false),
        ("[1 2] / sparse([1 0; 0 2])", false),
        ("sparse([2 0; 0 4]) \\ [2; 4]", false),
        ("2 ./ P", false),
    ] {
        run(&mut e, &format!("x = {expr};"));
        assert_eq!(is_sparse(&e, "x"), sparse_result, "{expr}");
    }
    run(&mut e, "x = sparse([2 0; 0 4]) \\ [2; 4];");
    assert_eq!(var(&e, "x").2, vec![1., 1.]);
    // Multiplying or dividing by a scalar changes stored values only (Octave).
    run(
        &mut e,
        "x = sparse([1 0]) * Inf; y = sparse([1 0]) / 0; z = 0 \\ sparse([1 0]);",
    );
    for name in ["x", "y", "z"] {
        assert_eq!(sparse(&e, name).nnz(), 1, "{name}");
        assert_eq!(var(&e, name).2, vec![f64::INFINITY, 0.]);
    }
    // A non-finite full operand of .* gives NaN at structural zeros (Octave).
    run(&mut e, "x = sparse([0 1]) .* [NaN 1];");
    assert!(var(&e, "x").2[0].is_nan());
    // Results that cancel are not stored.
    run(
        &mut e,
        "x = P - P; y = round(sparse([0.4 2.6])); z = fix(sparse([-0.5 0.7 3]));",
    );
    assert_eq!(sparse(&e, "x").nnz(), 0);
    assert_eq!(sparse(&e, "y").nnz(), 1);
    assert_eq!(sparse(&e, "z").nnz(), 1);
    // Logical inputs give double arithmetic results.
    run(&mut e, "x = L + L; y = L * 2;");
    assert_eq!(e.array("x").unwrap().class(), "double");
    assert_eq!(e.array("y").unwrap().class(), "double");
    assert!(is_sparse(&e, "x") && is_sparse(&e, "y"));
}

#[test]
fn sqrt_rejects_negative_entries_and_logical_conversion_rejects_nan() {
    let mut e = engine();
    assert_eq!(
        error(&mut e, "x = sqrt(sparse([1 -4]));"),
        "sqrt: negative entries in sparse input are not supported yet"
    );
    run(&mut e, "x = sqrt(sparse([0 4]));");
    assert_eq!(var(&e, "x").2, vec![0., 2.]);
    assert!(error(&mut e, "x = logical(sparse([NaN 1]));").contains("NaN"));
    assert!(error(&mut e, "x = sparse([NaN 1]) & sparse([1 1]);").contains("NaN"));
}

#[test]
fn reductions_include_implicit_zeros() {
    let reference = expected();
    let mut e = engine();
    run(
        &mut e,
        "a = min(sparse([1 0; 2 3])); b = max(sparse([-1 0; -2 -3])); c = min(sparse([NaN 0; NaN 1]));",
    );
    assert_eq!(var(&e, "a").2, json_values(&reference["min_hole"]));
    assert_eq!(var(&e, "b").2, json_values(&reference["max_negative_hole"]));
    assert!(same(&var(&e, "c").2, &json_values(&reference["min_nan"])));
    // A hole's minimum 0 is not stored.
    assert_eq!(sparse(&e, "a").nnz(), 1);
    run(
        &mut e,
        "S = sparse([1 2; 3 4]); s = sum(S(:)); r = sum(S, 3); t = sum(sparse(logical([1 1])));",
    );
    assert!(is_sparse(&e, "s"));
    assert_eq!(var(&e, "s"), (1, 1, vec![10.]));
    assert_eq!(var(&e, "r").2, vec![1., 3., 2., 4.]);
    assert_eq!(e.array("t").unwrap().class(), "double");
    assert!(is_sparse(&e, "t"));
    // Row vectors reduce along their first non-singleton dimension.
    run(
        &mut e,
        "x = sum(sparse([1 0 2])); y = max(sparse([0 -1 0])); z = any(sparse([0 0 1]));",
    );
    assert_eq!(var(&e, "x"), (1, 1, vec![3.]));
    assert_eq!(var(&e, "y"), (1, 1, vec![0.]));
    assert_eq!(var(&e, "z"), (1, 1, vec![1.]));
    assert_eq!(e.array("z").unwrap().class(), "logical");
    // max/min keep a logical input logical, as for full input.
    run(
        &mut e,
        "lm = max(sparse(logical([1 0; 0 0]))); lf = max(logical([1 0; 0 0]));",
    );
    assert_eq!(
        e.array("lm").unwrap().class(),
        e.array("lf").unwrap().class()
    );
    assert_eq!(var(&e, "lm").2, var(&e, "lf").2);
    // [m,i] = max(S) converts to full.
    run(&mut e, "[m, i] = max(sparse([1 5; 7 0]));");
    assert!(!is_sparse(&e, "m") && !is_sparse(&e, "i"));
    assert_eq!(var(&e, "m").2, vec![7., 5.]);
    assert_eq!(var(&e, "i").2, vec![2., 1.]);
    // Empty inputs follow the full function's shapes, stored sparse.
    run(&mut e, "x = sum(sparse(0, 3)); y = any(sparse(2, 0));");
    assert_eq!(var(&e, "x"), (1, 3, vec![0., 0., 0.]));
    assert!(is_sparse(&e, "x"));
    assert_eq!(var(&e, "y").0, 1);
}

#[test]
fn find_and_nonzeros_return_full_columns_in_column_order() {
    let mut e = engine();
    run(
        &mut e,
        "S = sparse([0 4; 5 0; 0 6]); k = find(S); [i, j, v] = find(S); n = nonzeros(S);",
    );
    for name in ["k", "i", "j", "v", "n"] {
        assert!(!is_sparse(&e, name), "{name}");
    }
    assert_eq!(var(&e, "k"), (3, 1, vec![2., 4., 6.]));
    assert_eq!(var(&e, "i").2, vec![2., 1., 3.]);
    assert_eq!(var(&e, "j").2, vec![1., 2., 2.]);
    assert_eq!(var(&e, "v").2, vec![5., 4., 6.]);
    assert_eq!(var(&e, "n"), (3, 1, vec![5., 4., 6.]));
    // Row vectors give rows; no nonzeros gives the full function's empty shapes.
    run(
        &mut e,
        "r = find(sparse([0 3 0 1])); z = find(sparse(3, 1)); w = find(sparse(1, 3)); m = find(sparse(3, 3));",
    );
    assert_eq!(var(&e, "r"), (1, 2, vec![2., 4.]));
    run(
        &mut e,
        "zf = find(zeros(3, 1)); wf = find(zeros(1, 3)); mf = find(zeros(3, 3));",
    );
    for (a, b) in [("z", "zf"), ("w", "wf"), ("m", "mf")] {
        assert_eq!(var(&e, a), var(&e, b), "{a}");
    }
    run(&mut e, "[i, j] = find(sparse(2, 2));");
    assert_eq!(var(&e, "i").2.len(), 0);
}

#[test]
fn constructors_follow_matlab_rules() {
    let reference = expected();
    let mut e = engine();
    // Scalar expansion of v and of a length-1 i or j.
    run(
        &mut e,
        "a = sparse([1 2 3], 2, 7, 3, 3); b = sparse(1, [1 3], [4 5]);",
    );
    assert_eq!(var(&e, "a").2, vec![0., 0., 0., 7., 7., 7., 0., 0., 0.]);
    assert_eq!(var(&e, "b"), (1, 3, vec![4., 0., 5.]));
    // Duplicates are summed in input order; sums of zero are not stored.
    run(
        &mut e,
        "c = sparse([1 1 2], [1 1 2], [1 -1 5]); d = sparse([2 2], [1 1], [0.1 0.2]);",
    );
    assert_eq!(
        sparse(&e, "c").nnz() as f64,
        reference["cancel"].as_f64().unwrap()
    );
    assert_eq!(var(&e, "c"), (2, 2, vec![0., 0., 0., 5.]));
    assert_eq!(var(&e, "d").2[1], 0.1 + 0.2);
    // Logical values are combined with or.
    run(&mut e, "g = sparse([1 1], [1 1], [true true]);");
    assert_eq!(e.array("g").unwrap().class(), "logical");
    assert_eq!(var(&e, "g"), (1, 1, vec![1.]));
    // sparse(2) is 1x1; sparse(m,n) is all zero; sparse(S) is S.
    run(
        &mut e,
        "h = sparse(2); k = sparse(3, 4); m = sparse(h); n = sparse([1 2], [1 2], [3 4], 4, 5, 10);",
    );
    assert_eq!(var(&e, "h"), (1, 1, vec![2.]));
    assert_eq!(
        (
            sparse(&e, "k").rows(),
            sparse(&e, "k").cols(),
            sparse(&e, "k").nnz()
        ),
        (3, 4, 0)
    );
    assert!(is_sparse(&e, "m"));
    assert_eq!(sparse(&e, "n").rows(), 4);
    // Subscripts are validated with the index error wording.
    assert_eq!(
        error(&mut e, "x = sparse([1 4], [1 1], [1 1], 3, 3);"),
        "Index must be an integer between 1 and 3"
    );
    assert_eq!(
        error(&mut e, "x = sparse([0 1], [1 1], [1 1]);"),
        "Index must be a positive integer"
    );
    assert!(error(&mut e, "x = sparse([1 2], [1 2 3], 1);").contains("same length"));
    for source in [
        "x = sparse(int8([1 0]));",
        "x = sparse([1i 0]);",
        "x = sparse('ab');",
        "x = sparse(single(1));",
    ] {
        assert_eq!(
            error(&mut e, source),
            "sparse: only double and logical input is supported yet",
            "{source}"
        );
    }
    // full keeps logical.
    run(&mut e, "f = full(sparse([true false]));");
    assert_eq!(e.array("f").unwrap().class(), "logical");
    // speye, spalloc, spones.
    run(
        &mut e,
        "i3 = speye(3); i23 = speye(2, 3); iv = speye([3 2]); z = spalloc(2, 3, 4); o = spones(sparse([0 -5; 2 0]));",
    );
    assert_eq!(var(&e, "i3").2, vec![1., 0., 0., 0., 1., 0., 0., 0., 1.]);
    assert_eq!(var(&e, "i23"), (2, 3, vec![1., 0., 0., 1., 0., 0.]));
    assert_eq!(var(&e, "iv"), (3, 2, vec![1., 0., 0., 0., 1., 0.]));
    assert_eq!((sparse(&e, "z").rows(), sparse(&e, "z").nnz()), (2, 0));
    assert_eq!(var(&e, "o").2, vec![0., 1., 1., 0.]);
    assert!(error(&mut e, "x = spalloc(2, 3, -1);").contains("nonnegative"));
    run(
        &mut e,
        "a1 = nzmax(sparse([1 0 2])); a2 = nzmax(spalloc(3, 3, 9)); a3 = nzmax([1 0 2]); s1 = issparse(speye(2)); s2 = issparse(eye(2));",
    );
    assert_eq!(var(&e, "a1").2, vec![2.]);
    assert_eq!(var(&e, "a2").2, vec![0.]);
    assert_eq!(var(&e, "a3").2, vec![3.]);
    assert_eq!(var(&e, "s1").2, vec![1.]);
    assert_eq!(var(&e, "s2").2, vec![0.]);
}

#[test]
fn spdiags_create_form_matches_octave() {
    let reference = expected();
    let mut e = engine();
    run(
        &mut e,
        "t = spdiags(reshape(1:12, 4, 3), [-1 0 1], 5, 4); w = spdiags(reshape(1:9, 3, 3), [-1 0 2], 3, 5);",
    );
    for (name, key) in [("t", "spdiags_tall"), ("w", "spdiags_wide")] {
        assert!(is_sparse(&e, name));
        let rows: Vec<Vec<f64>> = reference[key]
            .as_array()
            .unwrap()
            .iter()
            .map(json_values)
            .collect();
        let (m, n, data) = var(&e, name);
        assert_eq!((m, n), (rows.len(), rows[0].len()), "{name}");
        for (r, row) in rows.iter().enumerate() {
            for (c, &v) in row.iter().enumerate() {
                assert_eq!(data[c * m + r], v, "{name}({},{})", r + 1, c + 1);
            }
        }
    }
    // Octave's own example in the published form.
    run(&mut e, "x = full(spdiags(reshape(1:12,4,3),[-1 0 1],5,4));");
    assert_eq!(
        var(&e, "x").2,
        vec![
            5., 1., 0., 0., 0., 10., 6., 2., 0., 0., 0., 11., 7., 3., 0., 0., 0., 12., 8., 4.
        ]
    );
    assert!(error(&mut e, "x = spdiags(speye(3));").contains("not supported yet"));
}

#[test]
fn indexing_reads_give_sparse_results() {
    let mut e = engine();
    run(&mut e, "S = sparse([1 0 2; 0 3 0; 4 0 5]);");
    for (expr, rows, cols, data) in [
        ("S(2,2)", 1, 1, vec![3.]),
        ("S(1,3)", 1, 1, vec![2.]),
        ("S(2,1)", 1, 1, vec![0.]),
        ("S(:,3)", 3, 1, vec![2., 0., 5.]),
        ("S(3,:)", 1, 3, vec![4., 0., 5.]),
        ("S(7)", 1, 1, vec![2.]),
        ("S([1 3], [3 1])", 2, 2, vec![2., 5., 1., 4.]),
        ("S([3 1 3], 1)", 3, 1, vec![4., 1., 4.]),
        ("S(end, end)", 1, 1, vec![5.]),
        ("S(end)", 1, 1, vec![5.]),
        ("S(:)", 9, 1, vec![1., 0., 4., 0., 3., 0., 2., 0., 5.]),
        ("S(S > 2)", 3, 1, vec![4., 3., 5.]),
        ("S(logical([1 0 1]), 1)", 2, 1, vec![1., 4.]),
        ("S([1 2; 3 4])", 2, 2, vec![1., 4., 0., 0.]),
        ("S()", 3, 3, vec![1., 0., 4., 0., 3., 0., 2., 0., 5.]),
    ] {
        run(&mut e, &format!("x = {expr};"));
        assert!(is_sparse(&e, "x"), "{expr}");
        assert_eq!(var(&e, "x"), (rows, cols, data), "{expr}");
        assert_canonical(&sparse(&e, "x"));
    }
    // A row vector keeps its orientation under a vector index.
    run(
        &mut e,
        "R = sparse([0 7 0 8]); x = R([4 2]); y = R([4; 2]);",
    );
    assert_eq!(var(&e, "x"), (1, 2, vec![8., 7.]));
    assert_eq!(var(&e, "y"), (1, 2, vec![8., 7.]));
    // A sparse used as an index is converted to full first.
    run(
        &mut e,
        "A = [10 20 30]; x = A(sparse([3 1])); y = A(sparse(logical([1 0 1])));",
    );
    assert_eq!(var(&e, "x").2, vec![30., 10.]);
    assert_eq!(var(&e, "y").2, vec![10., 30.]);
    assert_eq!(
        error(&mut e, "x = S(1, 1, 1);"),
        "sparse: N-D indexing is not supported"
    );
    assert!(error(&mut e, "x = S(4, 1);").contains("Index"));
}

#[test]
fn indexed_writes_grow_delete_and_keep_the_last_value() {
    let mut e = engine();
    run(&mut e, "S = sparse([1 0; 0 2; 0 0]); T = S;");
    run(&mut e, "S(3, 1) = 7;");
    assert_eq!(var(&e, "S").2, vec![1., 0., 7., 0., 2., 0.]);
    // Copy-on-write: T is unchanged.
    assert_eq!(var(&e, "T").2, vec![1., 0., 0., 0., 2., 0.]);
    // Zeros in a block delete stored entries.
    run(&mut e, "S(1:2, 1:2) = [0 5; 0 0];");
    assert_eq!(var(&e, "S").2, vec![0., 0., 7., 5., 0., 0.]);
    assert_eq!(sparse(&e, "S").nnz(), 2);
    assert_canonical(&sparse(&e, "S"));
    // Growth by subscripts beyond the size stores nothing for a zero (Octave).
    let reference = expected();
    run(
        &mut e,
        "G = sparse(3, 2); G(5, 6) = 0; g = [size(G), nnz(G)];",
    );
    assert_eq!(var(&e, "g").2, json_values(&reference["growth"]));
    run(&mut e, "G(4, 7) = 3;");
    assert_eq!(
        (
            sparse(&e, "G").rows(),
            sparse(&e, "G").cols(),
            sparse(&e, "G").nnz()
        ),
        (5, 7, 1)
    );
    // Linear growth of a matrix is an error with the full-array message.
    assert_eq!(
        error(&mut e, "S(10) = 1;"),
        "Cannot grow a matrix through one ambiguous linear subscript; use row and column indices"
    );
    run(&mut e, "V = sparse([1 0]); V(4) = 2;");
    assert_eq!(var(&e, "V"), (1, 4, vec![1., 0., 0., 2.]));
    // Repeated write indices: the last value wins.
    run(&mut e, "W = sparse(2, 2); W([1 1 1], 2) = [4; 0; 6];");
    assert_eq!(var(&e, "W").2, vec![0., 0., 6., 0.]);
    run(&mut e, "W([2 2]) = [3 0];");
    assert_eq!(sparse(&e, "W").nnz(), 1);
    // Row and column deletion.
    run(
        &mut e,
        "D = sparse([1 2 3; 4 5 6]); D(:, 2) = []; E = sparse([1 2; 3 4; 5 6]); E(2, :) = [];",
    );
    assert_eq!(var(&e, "D"), (2, 2, vec![1., 4., 3., 6.]));
    assert_eq!(var(&e, "E"), (2, 2, vec![1., 5., 2., 6.]));
    assert!(is_sparse(&e, "D") && is_sparse(&e, "E"));
    // Logical sparse stays logical.
    run(&mut e, "B = sparse(logical([1 0])); B(2) = 5;");
    assert_eq!(e.array("B").unwrap().class(), "logical");
    assert_eq!(var(&e, "B").2, vec![1., 1.]);
    assert_eq!(
        error(&mut e, "S(1, 1, 1) = 2;"),
        "sparse: N-D indexing is not supported"
    );
    assert_eq!(
        error(&mut e, "S(1:2, 1) = [1 2 3];"),
        "Indexed assignment requires matching element counts or a scalar"
    );
}

#[test]
fn display_strings_are_pinned() {
    let mut e = engine();
    run(&mut e, "S = sparse([1 3 2], [1 1 3], [1 2.5 -4], 3, 3);");
    assert_eq!(
        run(&mut e, "S"),
        "\nS =\n\n   (1,1)    1.0000\n   (3,1)    2.5000\n   (2,3)   -4.0000\n"
    );
    assert_eq!(
        run(&mut e, "disp(S)"),
        "   (1,1)    1.0000\n   (3,1)    2.5000\n   (2,3)   -4.0000\n"
    );
    run(&mut e, "format long");
    assert_eq!(
        run(&mut e, "S"),
        "\nS =\n\n   (1,1)   1.000000000000000\n   (3,1)   2.500000000000000\n   (2,3)  -4.000000000000000\n"
    );
    run(&mut e, "format short");
    run(&mut e, "Z = sparse(3, 4);");
    assert_eq!(run(&mut e, "Z"), "\nZ =\n\n   All zero sparse: 3x4\n");
    assert_eq!(run(&mut e, "disp(Z)"), "   All zero sparse: 3x4\n");
    // Integer values and a wide index pair.
    run(&mut e, "I = sparse([1 12], [1 10], [3 4]);");
    assert_eq!(
        run(&mut e, "I"),
        "\nI =\n\n     (1,1)     3\n   (12,10)     4\n"
    );
}

#[test]
fn bytes_whos_and_class_queries() {
    let mut e = engine();
    run(
        &mut e,
        "S = sparse([1 0 2; 0 0 3]); L = sparse(logical([1 0 1; 0 0 1]));",
    );
    // double: 16 per entry + 8 per column pointer; logical: 9 per entry + 8 per pointer.
    assert_eq!(e.array("S").unwrap().bytes(), 16 * 3 + 8 * 4);
    assert_eq!(e.array("L").unwrap().bytes(), 9 * 3 + 8 * 4);
    let whos = run(&mut e, "whos");
    assert!(whos.contains("80 bytes  double (sparse)"), "{whos}");
    assert!(whos.contains("59 bytes  logical (sparse)"), "{whos}");
    run(
        &mut e,
        "a = class(S); b = class(L); c = [isa(S,'double') isa(S,'numeric') isa(S,'float') isa(L,'logical') isa(L,'numeric')];",
    );
    run(
        &mut e,
        "d = [isreal(S) islogical(S) islogical(L) isnumeric(S) isnumeric(L) isfloat(S) issparse(S)];",
    );
    run(
        &mut e,
        "f = [size(S) numel(S) ndims(S) length(S) isempty(S) isscalar(S) isvector(S)]; [m, n] = size(S);",
    );
    assert_eq!(run(&mut e, "disp(a)"), "double");
    assert_eq!(run(&mut e, "disp(b)"), "logical");
    assert_eq!(var(&e, "c").2, vec![1., 1., 1., 1., 0.]);
    assert_eq!(var(&e, "d").2, vec![1., 0., 1., 1., 0., 1., 1.]);
    assert_eq!(var(&e, "f").2, vec![2., 3., 6., 2., 3., 0., 0., 0.]);
    assert_eq!((var(&e, "m").2, var(&e, "n").2), (vec![2.], vec![3.]));
    // reshape keeps sparse storage.
    run(&mut e, "r = reshape(S, 3, 2); q = reshape(S, [], 1);");
    assert_eq!(var(&e, "r"), (3, 2, vec![1., 0., 0., 0., 2., 3.]));
    assert_eq!(var(&e, "q").0, 6);
    assert!(is_sparse(&e, "r") && is_sparse(&e, "q"));
}

#[test]
fn isequal_compares_values_across_storage() {
    let reference = expected();
    let mut e = engine();
    run(
        &mut e,
        "F = [1 2 3; 4 5 6]; a = isequal(sparse(F), F); b = isequal(sparse([NaN 1]), sparse([NaN 1])); c = isequaln(sparse([NaN 1]), sparse([NaN 1]));",
    );
    let got: Vec<f64> = ["a", "b", "c"].iter().map(|n| var(&e, n).2[0]).collect();
    assert_eq!(got, json_values(&reference["isequal"]));
    run(
        &mut e,
        "d = isequal(sparse([1 0]), [1 0], sparse([1 0])); g = isequal(sparse([1 0]), [1 0 0]); h = isequal(F, sparse(F));",
    );
    assert_eq!(var(&e, "d").2, vec![1.]);
    assert_eq!(var(&e, "g").2, vec![0.]);
    assert_eq!(var(&e, "h").2, vec![1.]);
}

#[test]
fn concatenation_stores_the_full_blocks_nonzeros() {
    let mut e = engine();
    run(
        &mut e,
        "S = sparse([1 0; 0 2]); x = [S, [0; 3]]; y = [[4 0]; S]; z = [S, S; S, S]; w = horzcat(S, [5; 0]); v = vertcat(S, [0 0]);",
    );
    assert_eq!(var(&e, "x"), (2, 3, vec![1., 0., 0., 2., 0., 3.]));
    assert_eq!(var(&e, "y"), (3, 2, vec![4., 1., 0., 0., 0., 2.]));
    assert_eq!(var(&e, "z").0, 4);
    assert_eq!(sparse(&e, "x").nnz(), 3);
    for name in ["x", "y", "z", "w", "v"] {
        assert!(is_sparse(&e, name), "{name}");
    }
    run(
        &mut e,
        "l = [sparse(true), sparse(false)]; m = [sparse(true), 2];",
    );
    assert_eq!(e.array("l").unwrap().class(), "logical");
    assert_eq!(e.array("m").unwrap().class(), "double");
    for source in ["x = [S, 'ab'];", "x = [S, {1}];", "x = [S; \"s\"];"] {
        assert!(error(&mut e, source).contains("not supported"), "{source}");
    }
}

#[test]
fn other_functions_receive_full_copies() {
    let mut e = engine();
    run(
        &mut e,
        "S = sparse([1 0; 0 4]); x = exp(S); y = cumsum(S); z = mean(S); xf = exp([1 0; 0 4]);",
    );
    for name in ["x", "y", "z"] {
        assert!(!is_sparse(&e, name), "{name}");
    }
    assert_eq!(var(&e, "x"), var(&e, "xf"));
    assert_eq!(var(&e, "z").2, vec![0.5, 2.]);
}

#[test]
fn sparse_to_full_limit_errors_name_the_function() {
    let mut e = engine();
    e.set_sparse_full_limit(4);
    run(&mut e, "S = speye(3); T = sparse([1 0; 0 1]);");
    assert_eq!(
        error(&mut e, "x = full(S);"),
        "full: sparse input with more than 4 elements cannot be converted to full yet"
    );
    assert_eq!(
        error(&mut e, "x = exp(S);"),
        "exp: sparse input with more than 4 elements cannot be converted to full yet"
    );
    // Square sparse solves never convert (sparse part 2); a full divisor still does.
    run(&mut e, "x = S \\ [1; 2; 3]; y = [1 2 3] / S;");
    assert_eq!(var(&e, "x").2, vec![1., 2., 3.]);
    assert_eq!(var(&e, "y").2, vec![1., 2., 3.]);
    assert_eq!(
        error(&mut e, "x = S / [1 2; 3 4; 5 6].';"),
        "mrdivide: sparse input with more than 4 elements cannot be converted to full yet"
    );
    // Within the limit the conversion works; sparse-aware functions never convert.
    run(&mut e, "x = full(T); y = sum(S); z = S * 2;");
    assert!(is_sparse(&e, "y") && is_sparse(&e, "z"));
}

#[test]
fn sparse_scalars_work_as_conditions_and_in_short_circuits() {
    let mut e = engine();
    run(
        &mut e,
        "a = 0; if sparse(1), a = 1; end; b = sparse(1) && true; c = false || sparse(2); d = 0; if sparse(0), d = 1; end",
    );
    assert_eq!(var(&e, "a").2, vec![1.]);
    assert_eq!(var(&e, "b").2, vec![1.]);
    assert_eq!(var(&e, "c").2, vec![1.]);
    assert_eq!(var(&e, "d").2, vec![0.]);
    assert_eq!(
        error(&mut e, "x = speye(2) && true;"),
        "Short-circuit operands must be logical scalars"
    );
    run(&mut e, "n = 0; while sparse(n < 3), n = n + 1; end");
    assert_eq!(var(&e, "n").2, vec![3.]);
    // for iterates columns as sparse columns.
    run(
        &mut e,
        "t = 0; for c = sparse([1 0; 0 2]), t = t + issparse(c) * sum(c); end",
    );
    assert_eq!(var(&e, "t").2, vec![3.]);
}

#[test]
fn user_functions_and_handles_receive_sparse_values() {
    let mut e = engine();
    run(
        &mut e,
        "y = f(sparse([0 3])); g = @(s) issparse(s); h = g(speye(2));\nfunction r = f(s)\nr = issparse(s) + nnz(s);\nend\n",
    );
    assert_eq!(var(&e, "y").2, vec![2.]);
    assert_eq!(var(&e, "h").2, vec![1.]);
    // Multiple outputs of a builtin after sparse values exist: arguments are
    // evaluated once.
    run(
        &mut e,
        "k = 0; [a, b] = size(ones(2, 3)); [m, i] = max([3 1 4]);",
    );
    assert_eq!((var(&e, "a").2, var(&e, "b").2), (vec![2.], vec![3.]));
    assert_eq!(var(&e, "i").2, vec![3.]);
    run(
        &mut e,
        "rng(1); [p, q] = size(rand(2, 5)); r1 = rand; rng(1); rand(2, 5); r2 = rand;",
    );
    assert_eq!(var(&e, "r1"), var(&e, "r2"));
    assert_eq!(var(&e, "q").2, vec![5.]);
    // The hidden argument variables do not stay in the workspace.
    assert!(
        e.workspace()
            .keys()
            .all(|k| k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
    );
}

fn temp_dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn mat_v5_round_trip_double_and_logical() {
    let dir = temp_dir();
    for option in ["-v6", "-v7"] {
        let mut e = engine();
        e.execute(&format!(
            "cd('{}')",
            dir.path().display().to_string().replace('\'', "''")
        ))
        .unwrap();
        run(
            &mut e,
            "S = sparse([1 3 2], [1 1 3], [1.5 -2 1e-300], 3, 4); L = sparse(logical([0 1; 1 0])); E = sparse(2, 0); D = [1 2];",
        );
        run(
            &mut e,
            &format!("save('rt.mat', 'S', 'L', 'E', 'D', '{option}');"),
        );
        let mut f = engine();
        f.execute(&format!(
            "cd('{}')",
            dir.path().display().to_string().replace('\'', "''")
        ))
        .unwrap();
        run(&mut f, "load('rt.mat');");
        for name in ["S", "L", "E"] {
            assert!(is_sparse(&f, name), "{option} {name}");
            assert_eq!(var(&f, name), var(&e, name), "{option} {name}");
            assert!(sparse(&f, name).equals(&sparse(&e, name), false));
        }
        assert_eq!(f.array("L").unwrap().class(), "logical");
        assert_eq!(var(&f, "D").2, vec![1., 2.]);
        // decoded bytes use the sparse byte count.
        let catalog =
            matrust::files::inspect_workspace(&dir.path().join("rt.mat"), &Default::default())
                .unwrap();
        let s = catalog.variables().find(|v| v.name == "S").unwrap();
        assert_eq!(s.decoded_bytes, Some(16 * 3 + 8 * 5));
        assert_eq!(s.class, "sparse");
        let l = catalog.variables().find(|v| v.name == "L").unwrap();
        assert_eq!(l.decoded_bytes, Some(9 * 2 + 8 * 3));
        assert_eq!(l.class, "logical sparse");
    }
}

#[test]
fn hand_built_logical_sparse_file_loads() {
    // MATLAB's layout for a logical sparse array: class 5 with the logical
    // flag, int32 ir and jc, uint8 pr. Built here byte by byte.
    fn element(out: &mut Vec<u8>, kind: u32, data: &[u8]) {
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
        out.resize(out.len().next_multiple_of(8), 0);
    }
    let int32 = |v: &[i32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<_>>();
    let mut body = Vec::new();
    let mut flags = 0x205u32.to_le_bytes().to_vec();
    flags.extend_from_slice(&3u32.to_le_bytes());
    element(&mut body, 6, &flags);
    element(&mut body, 5, &int32(&[3, 2]));
    element(&mut body, 1, b"M");
    element(&mut body, 5, &int32(&[0, 2, 1]));
    element(&mut body, 5, &int32(&[0, 2, 3]));
    element(&mut body, 2, &[1, 1, 1]);
    let mut file = vec![b' '; 128];
    let text = b"MATLAB 5.0 MAT-file, hand-built logical sparse";
    file[..text.len()].copy_from_slice(text);
    file[116..124].fill(0);
    file[124..128].copy_from_slice(&[0, 1, b'I', b'M']);
    element(&mut file, 14, &body);
    let dir = temp_dir();
    std::fs::write(dir.path().join("logical.mat"), &file).unwrap();
    let mut e = engine();
    e.execute(&format!(
        "cd('{}')",
        dir.path().display().to_string().replace('\'', "''")
    ))
    .unwrap();
    run(&mut e, "load('logical.mat');");
    assert_eq!(e.array("M").unwrap().class(), "logical");
    assert_eq!(var(&e, "M"), (3, 2, vec![1., 0., 1., 0., 1., 0.]));
}

#[test]
fn octave_files_load_with_the_expected_values() {
    let reference = expected();
    for file in ["octave-v6.mat", "octave-v7.mat"] {
        let path = std::path::Path::new("tests/fixtures/sparse").join(file);
        let catalog = matrust::files::inspect_workspace(&path, &Default::default()).unwrap();
        let names = ["S", "E", "V", "D"].iter().map(|s| s.to_string()).collect();
        let loaded =
            matrust::files::load_selected_arrays(&catalog, &names, &Default::default()).unwrap();
        for name in ["S", "E", "V"] {
            let Array::Sparse(s) = &loaded[name] else {
                panic!("{file} {name} is not sparse")
            };
            let r = &reference[name];
            assert_eq!(
                s.rows() as f64,
                r["rows"].as_f64().unwrap(),
                "{file} {name}"
            );
            assert_eq!(s.cols() as f64, r["cols"].as_f64().unwrap());
            let entries: Vec<(usize, usize, f64)> = s.entries().collect();
            let (i, j, v) = (
                json_values(&r["i"]),
                json_values(&r["j"]),
                json_values(&r["v"]),
            );
            assert_eq!(entries.len(), v.len(), "{file} {name}");
            for (k, &(row, col, value)) in entries.iter().enumerate() {
                assert_eq!(
                    (row as f64 + 1., col as f64 + 1., value),
                    (i[k], j[k], v[k])
                );
            }
        }
        assert!(matches!(loaded["D"], Array::Numeric(_)));
    }
}

#[test]
fn complex_sparse_variable_is_reported_and_skipped() {
    let dir = temp_dir();
    std::fs::copy(
        "tests/fixtures/sparse/octave-complex.mat",
        dir.path().join("c.mat"),
    )
    .unwrap();
    let mut e = engine();
    e.execute(&format!(
        "cd('{}')",
        dir.path().display().to_string().replace('\'', "''")
    ))
    .unwrap();
    let message = error(&mut e, "load('c.mat');");
    assert!(
        message.contains("'Z'") && message.contains("complex sparse"),
        "{message}"
    );
    assert_eq!(var(&e, "A").2, vec![1., 2., 3.]);
    assert!(is_sparse(&e, "B"));
    assert!(e.array("Z").is_none());
    let catalog =
        matrust::files::inspect_workspace(&dir.path().join("c.mat"), &Default::default()).unwrap();
    let z = catalog.variables().find(|v| v.name == "Z").unwrap();
    assert!(z.complex && z.unavailable.is_some());
}

#[test]
fn v73_save_rejects_sparse() {
    let dir = temp_dir();
    let mut e = engine();
    e.execute(&format!(
        "cd('{}')",
        dir.path().display().to_string().replace('\'', "''")
    ))
    .unwrap();
    run(&mut e, "S = speye(2);");
    let message = error(&mut e, "save('s.mat', 'S', '-v7.3');");
    assert!(
        message.contains("sparse variables cannot be saved to v7.3 yet"),
        "{message}"
    );
}

#[test]
fn storage_limits_are_checked_before_allocating() {
    let mut e = engine();
    // A huge but empty sparse array costs only its column pointers.
    run(&mut e, "H = sparse(1e9, 10); H(1e9, 10) = 1; n = nnz(H);");
    assert_eq!(var(&e, "n").2, vec![1.]);
    assert!(error(&mut e, "x = sparse(2, 1e8);").contains("columns"));
    assert!(error(&mut e, "x = full(H);").contains("cannot be converted to full yet"));
}

// ------------------------------------------------------------ fix round 1

#[test]
fn multi_output_calls_work_on_the_first_sparse_value_of_a_fresh_engine() {
    // Each case runs in a new engine: the hook decides from the arguments,
    // not from what the session did before.
    let mut e = engine();
    run(&mut e, "[i, j] = find(sparse([0 2; 3 0]));");
    assert_eq!(var(&e, "i").2, vec![2., 1.]);
    assert_eq!(var(&e, "j").2, vec![1., 2.]);

    let mut e = engine();
    run(&mut e, "[i, j, v] = find(speye(2) * 3);");
    assert_eq!(var(&e, "v").2, vec![3., 3.]);

    let mut e = engine();
    run(&mut e, "[m, k] = max(sparse([1 0 5 2]));");
    assert_eq!((var(&e, "m").2, var(&e, "k").2), (vec![5.], vec![3.]));
    assert!(!is_sparse(&e, "m"));

    let mut e = engine();
    run(&mut e, "[r, c] = size(sparse(3, 4));");
    assert_eq!((var(&e, "r").2, var(&e, "c").2), (vec![3.], vec![4.]));

    // A user function and a handle that build the first sparse value.
    let mut e = engine();
    run(
        &mut e,
        "[i, j] = find(make());\nfunction s = make()\ns = sparse([0 1]);\nend\n",
    );
    assert_eq!((var(&e, "i").2, var(&e, "j").2), (vec![1.], vec![2.]));
    let mut e = engine();
    run(&mut e, "h = @() sparse([0 1]); [i, j] = find(h());");
    assert_eq!((var(&e, "i").2, var(&e, "j").2), (vec![1.], vec![2.]));

    // Arguments are evaluated once, whether or not the value is sparse.
    let mut e = engine();
    let output = run(
        &mut e,
        "[i, j] = find(noisy());\nfunction s = noisy()\ndisp('called');\ns = sparse([0 1]);\nend\n",
    );
    assert_eq!(output.matches("called").count(), 1, "{output}");
    let mut e = engine();
    let output = run(
        &mut e,
        "[m, k] = max(noisy());\nfunction s = noisy()\ndisp('called');\ns = [3 9 4];\nend\n",
    );
    assert_eq!(output.matches("called").count(), 1, "{output}");
    assert_eq!((var(&e, "m").2, var(&e, "k").2), (vec![9.], vec![2.]));
}

#[test]
fn shape_predicates_of_empty_sparse_match_dense() {
    let mut e = engine();
    for (m, n) in [
        (0, 1),
        (1, 0),
        (0, 0),
        (0, 3),
        (3, 0),
        (1, 1),
        (2, 2),
        (1, 4),
    ] {
        for f in [
            "isscalar", "isvector", "isrow", "iscolumn", "ismatrix", "isempty",
        ] {
            run(
                &mut e,
                &format!("a = {f}(sparse({m}, {n})); b = {f}(zeros({m}, {n}));"),
            );
            assert_eq!(var(&e, "a"), var(&e, "b"), "{f} of {m}x{n}");
        }
    }
    run(
        &mut e,
        "v = isvector(sparse(0, 1)); w = isvector(sparse(1, 0));",
    );
    assert_eq!((var(&e, "v").2, var(&e, "w").2), (vec![1.], vec![1.]));
}

#[test]
fn not_and_conditions_use_the_sparse_to_full_limit() {
    let mut e = engine();
    e.set_sparse_full_limit(4);
    run(&mut e, "S = speye(3); T = sparse([1 2; 3 4]);");
    let tail = "sparse input with more than 4 elements cannot be converted to full yet";
    assert_eq!(error(&mut e, "x = ~S;"), format!("not: {tail}"));
    assert_eq!(error(&mut e, "x = not(S);"), format!("not: {tail}"));
    assert_eq!(error(&mut e, "if S, x = 1; end"), format!("if: {tail}"));
    assert_eq!(
        error(&mut e, "while S, x = 1; end"),
        format!("while: {tail}")
    );
    assert_eq!(error(&mut e, "if ~S, x = 1; end"), format!("if: {tail}"));
    assert_eq!(
        error(&mut e, "if S & true, x = 1; end"),
        format!("if: {tail}")
    );
    // Within the limit: the same operations work.
    run(
        &mut e,
        "a = ~T; b = 0; if T, b = 1; end; c = 0; while ~T, c = 1; end",
    );
    assert_eq!(var(&e, "a").2, vec![0., 0., 0., 0.]);
    assert_eq!((var(&e, "b").2, var(&e, "c").2), (vec![1.], vec![0.]));

    // A 1x1 sparse operand obeys the limit too, in conditions and short-circuits.
    let mut e = engine();
    e.set_sparse_full_limit(0);
    run(&mut e, "T = sparse(1);");
    let tail = "sparse input with more than 0 elements cannot be converted to full yet";
    assert_eq!(error(&mut e, "if T, x = 1; end"), format!("if: {tail}"));
    assert_eq!(error(&mut e, "x = T && true;"), format!("&&: {tail}"));
    assert_eq!(error(&mut e, "x = false || T;"), format!("||: {tail}"));
    assert_eq!(error(&mut e, "x = ~T;"), format!("not: {tail}"));
}

#[test]
fn hidden_argument_variables_never_show_and_are_removed_after_errors() {
    let mut e = engine();
    run(&mut e, "S = sparse([1 0; 0 1]);");
    // Four outputs of find is an error raised after the arguments went through
    // hidden variables.
    let message = error(&mut e, "[a, b, c, d] = find(S + S);");
    assert!(message.contains("find"), "{message}");
    let message = error(&mut e, "[a, b] = sort(S, 'bogus');");
    assert!(!message.is_empty());
    assert_eq!(
        e.workspace().keys().cloned().collect::<Vec<_>>(),
        ["S".to_owned()]
    );
    for k in 1..20 {
        assert!(
            !e.contains_array(&format!("\u{1}sparse_argument_{k}")),
            "{k}"
        );
    }
    let listing = run(&mut e, "who");
    assert!(!listing.contains("sparse_argument"), "{listing}");
    assert!(listing.contains('S'), "{listing}");
    // The call still works afterwards, and so does a successful one.
    run(&mut e, "[v, i] = sort(S + S, 'descend');");
    assert_eq!(var(&e, "v").2, vec![2., 0., 2., 0.]);
    assert!(!e.workspace().keys().any(|k| k.starts_with('\u{1}')));

    // While the call runs, a callback that lists or saves the caller's
    // workspace does not see the hidden argument variables either.
    let dir = temp_dir();
    run(
        &mut e,
        &format!(
            "cd('{}');",
            dir.path().display().to_string().replace('\'', "''")
        ),
    );
    let output = run(
        &mut e,
        "[p, q] = arrayfun(@peek, sparse([1 2]));\n\
         function [r, s] = peek(x)\n\
         evalin('caller', 'who'); evalin('caller', 'whos');\n\
         evalin('caller', 'save(''peek.mat'')');\n\
         r = x; s = x;\n\
         end\n",
    );
    assert!(output.contains("Your variables"), "{output}");
    assert!(!output.contains("sparse_argument"), "{output}");
    let catalog =
        matrust::files::inspect_workspace(&dir.path().join("peek.mat"), &Default::default())
            .unwrap();
    let names: Vec<_> = catalog.variables().map(|v| v.name.clone()).collect();
    assert!(names.contains(&"S".to_owned()), "{names:?}");
    assert!(names.iter().all(|n| !n.starts_with('\u{1}')), "{names:?}");
    assert!(!e.workspace().keys().any(|k| k.starts_with('\u{1}')));
}

// ------------------------------------------------------------ fix round 2
// Huge row counts with few stored entries. 1e12 rows: any work or memory per
// row (8 TB of row indices) would abort or never finish, so each of these
// tests passing in milliseconds shows the path is O(stored entries + columns).

const HUGE: usize = 1_000_000_000_000;

fn entries(e: &Engine, name: &str) -> Vec<(usize, usize, f64)> {
    sparse(e, name).entries().collect()
}
fn shape(e: &Engine, name: &str) -> (usize, usize) {
    let s = sparse(e, name);
    (s.rows(), s.cols())
}

#[test]
fn colon_and_range_row_reads_of_a_huge_sparse_array_keep_ranges() {
    let mut e = engine();
    // The review's shape first.
    run(&mut e, "G = sparse(4e8, 10); G(7, 1) = 2; x = G(:, 1);");
    assert_eq!(shape(&e, "x"), (400_000_000, 1));
    assert_eq!(entries(&e, "x"), vec![(6, 0, 2.)]);

    run(
        &mut e,
        "H = sparse(1e12, 10); H(5, 1) = 2; H(1e12, 1) = 3; H(7, 3) = 4;\n\
         x = H(:, 1); y = H(:, :); z = H(:, [3 1]); w = H(2:1e7, 1);\n\
         v = H(1e12:-1:1e12-9, 1); u = H(1:2:2e7, 1); r = H(9:-2:1, 1); t = H(end, :);",
    );
    let last = HUGE - 1;
    assert_eq!(shape(&e, "x"), (HUGE, 1));
    assert_eq!(entries(&e, "x"), vec![(4, 0, 2.), (last, 0, 3.)]);
    assert!(sparse(&e, "y").equals(&sparse(&e, "H"), false));
    assert_eq!(
        entries(&e, "z"),
        vec![(6, 0, 4.), (4, 1, 2.), (last, 1, 3.)]
    );
    // Ranges stay ranges (the engine limits a range subscript to 16777216
    // elements before the sparse code sees it).
    assert_eq!(shape(&e, "w"), (9_999_999, 1));
    assert_eq!(entries(&e, "w"), vec![(3, 0, 2.)]);
    // Descending ranges: the last row comes first.
    assert_eq!(
        (shape(&e, "v"), entries(&e, "v")),
        ((10, 1), vec![(0, 0, 3.)])
    );
    assert_eq!(
        (shape(&e, "r"), entries(&e, "r")),
        ((5, 1), vec![(2, 0, 2.)])
    );
    // Odd rows only: row 5 is output row 3.
    assert_eq!(shape(&e, "u"), (10_000_000, 1));
    assert_eq!(entries(&e, "u"), vec![(2, 0, 2.)]);
    assert_eq!(entries(&e, "t"), vec![(0, 0, 3.)]);

    // The same selections on a small matrix agree with full indexing.
    run(
        &mut e,
        "F = [1 0 2 0; 0 3 0 0; 4 0 0 5; 0 6 7 0]; S = sparse(F);",
    );
    for subscript in [
        "4:-1:1, :",
        "1:2:4, [2 2 1]",
        "[3 1 3], :",
        "logical([1 0 1 1]), 2",
        "4:-2:1, 3:4",
        "2:3, :",
        "end:-1:2, end",
    ] {
        run(&mut e, &format!("a = S({subscript}); b = F({subscript});"));
        assert!(is_sparse(&e, "a"), "{subscript}");
        assert_eq!(var(&e, "a"), var(&e, "b"), "{subscript}");
        assert_canonical(&sparse(&e, "a"));
    }
}

#[test]
fn transpose_of_a_huge_row_count_hits_the_column_limit_and_row_reductions_do_not_transpose() {
    let mut e = engine();
    run(
        &mut e,
        "G = sparse(4e8, 10); H = sparse(1e12, 10); H(3, 2) = 5; H(3, 4) = 1; H(1e12, 4) = 7; H(9, 5) = -2;",
    );
    for source in [
        "x = G';",
        "x = H';",
        "x = H.';",
        "x = transpose(H);",
        "x = ctranspose(H);",
        "x = speye(3) * G';",
    ] {
        let message = error(&mut e, source);
        assert!(
            message.contains("columns exceed the limit of 99999999 columns"),
            "{source}: {message}"
        );
    }
    run(
        &mut e,
        "s = sum(H, 2); mx = max(H, [], 2); mn = min(H, [], 2); a = any(H, 2); l = all(H, 2); g = sum(G, 2);",
    );
    let last = HUGE - 1;
    assert_eq!(shape(&e, "s"), (HUGE, 1));
    assert_eq!(
        entries(&e, "s"),
        vec![(2, 0, 6.), (8, 0, -2.), (last, 0, 7.)]
    );
    // Every row has implicit zeros: they take part in max and min.
    assert_eq!(entries(&e, "mx"), vec![(2, 0, 5.), (last, 0, 7.)]);
    assert_eq!(entries(&e, "mn"), vec![(8, 0, -2.)]);
    assert_eq!(
        entries(&e, "a"),
        vec![(2, 0, 1.), (8, 0, 1.), (last, 0, 1.)]
    );
    assert!(sparse(&e, "a").is_logical() && sparse(&e, "l").is_logical());
    assert_eq!((shape(&e, "l"), sparse(&e, "l").nnz()), ((HUGE, 1), 0));
    assert_eq!(
        (shape(&e, "g"), sparse(&e, "g").nnz()),
        ((400_000_000, 1), 0)
    );
    // A sparse product whose transposed factor is small still works.
    run(
        &mut e,
        "p = H * sparse([1 0 0 0 0 0 0 0 0 0; 0 1 0 1 0 0 0 0 0 0])';",
    );
    assert_eq!(entries(&e, "p"), vec![(2, 1, 6.), (last, 1, 7.)]);

    // Row reductions agree with the full functions (NaN, negative values,
    // empty rows, logical input) and stay sparse.
    run(
        &mut e,
        "F = [1 0 -2 NaN; 0 0 0 0; 3 4 0 -1; 0 NaN 0 0; 5 6 7 8]; S = sparse(F);\n\
         FL = logical([1 0 1; 0 0 0; 1 1 1]); L = sparse(FL);",
    );
    for (sparse_call, full_call) in [
        ("sum(S, 2)", "sum(F, 2)"),
        ("max(S, [], 2)", "max(F, [], 2)"),
        ("min(S, [], 2)", "min(F, [], 2)"),
        ("any(S, 2)", "any(F, 2)"),
        ("all(S, 2)", "all(F, 2)"),
        ("sum(L, 2)", "sum(FL, 2)"),
        ("max(L, [], 2)", "max(FL, [], 2)"),
        ("any(L, 2)", "any(FL, 2)"),
        ("all(L, 2)", "all(FL, 2)"),
        ("sum(S')", "sum(F')"),
    ] {
        run(
            &mut e,
            &format!("a = {sparse_call}; b = {full_call}; ca = class(a); cb = class(b);"),
        );
        assert!(is_sparse(&e, "a"), "{sparse_call}");
        let (a, b) = (var(&e, "a"), var(&e, "b"));
        assert!(
            a.0 == b.0 && a.1 == b.1 && same(&a.2, &b.2),
            "{sparse_call}: {a:?} {b:?}"
        );
        assert_eq!(
            run(&mut e, "disp(ca)"),
            run(&mut e, "disp(cb)"),
            "{sparse_call}"
        );
        assert_canonical(&sparse(&e, "a"));
    }
}

#[test]
fn for_loop_over_a_huge_sparse_array_extracts_each_column_directly() {
    let mut e = engine();
    run(
        &mut e,
        "G = sparse(4e8, 2); G(4e8, 2) = 1; k = 0; for c = G, k = k + nnz(c); end",
    );
    assert_eq!(var(&e, "k").2, vec![1.]);
    run(
        &mut e,
        "H = sparse(1e12, 3); H(4, 1) = 2; H(1e12, 3) = 3; H(5, 3) = 1;\n\
         n = 0; t = 0; r = 0; q = 0; for c = H, n = n + nnz(c); t = t + sum(c); r = size(c, 1); q = q + issparse(c); end",
    );
    assert_eq!(var(&e, "n").2, vec![3.]);
    assert_eq!(var(&e, "t").2, vec![6.]);
    assert_eq!(var(&e, "r").2, vec![1e12]);
    assert_eq!(var(&e, "q").2, vec![3.]);
    assert_eq!(entries(&e, "c"), vec![(4, 0, 1.), (HUGE - 1, 0, 3.)]);
}

#[test]
fn spdiags_checks_size_and_stored_entries_before_building() {
    let mut e = engine();
    // Column pointers: checked before any diagonal is built.
    let message = error(&mut e, "x = spdiags(1, 0, 2e8, 2e8);");
    assert!(message.contains("columns exceed the limit"), "{message}");
    // A scalar B is expanded along each diagonal: three diagonals of 5e7
    // entries are over the 1e8 limit, found by counting before building.
    let message = error(&mut e, "x = spdiags(1, [0 1 2], 1e11, 5e7);");
    assert!(
        message.contains("nonzeros exceed the limit of 100000000"),
        "{message}"
    );
    // A huge row count builds only the diagonal entries (values from GNU
    // Octave 10.3 with 10 rows: the same alignment, since m >= n).
    run(
        &mut e,
        "D = spdiags([1; 2; 3], 0, 1e12, 3); Z = spdiags(0, 0, 1e12, 4);\n\
         W = spdiags([1 2; 3 4; 0 6], [0 -1], 1e12, 3);",
    );
    assert_eq!(shape(&e, "D"), (HUGE, 3));
    assert_eq!(entries(&e, "D"), vec![(0, 0, 1.), (1, 1, 2.), (2, 2, 3.)]);
    assert_eq!(sparse(&e, "Z").nnz(), 0);
    // m >= n: diagonal -1 takes B(j, 2) for column j; the zero in B is not stored.
    assert_eq!(
        entries(&e, "W"),
        vec![(0, 0, 1.), (1, 0, 2.), (1, 1, 3.), (2, 1, 4.), (3, 2, 6.)]
    );
    // Small cases (GNU Octave 10.3 values).
    run(
        &mut e,
        "a = full(spdiags([1 2; 3 4; 0 6], [0 -1], 5, 3)); b = full(spdiags([1 2; 3 4; 5 6], [1 -2], 3, 5));",
    );
    assert_eq!(
        var(&e, "a"),
        (
            5,
            3,
            vec![1., 2., 0., 0., 0., 0., 3., 4., 0., 0., 0., 0., 0., 6., 0.]
        )
    );
    // m < n: B is indexed by row; diagonal 1 takes B(i, 1), diagonal -2 takes B(i, 2).
    assert_eq!(
        var(&e, "b"),
        (
            3,
            5,
            vec![0., 0., 6., 1., 0., 0., 0., 3., 0., 0., 0., 5., 0., 0., 0.]
        )
    );
}

#[test]
fn row_deletion_on_a_huge_sparse_array_renumbers_through_the_deleted_list() {
    let mut e = engine();
    run(&mut e, "G = sparse(4e8, 10); G(9, 2) = 1; G(1, :) = [];");
    assert_eq!(shape(&e, "G"), (399_999_999, 10));
    assert_eq!(entries(&e, "G"), vec![(7, 1, 1.)]);
    run(
        &mut e,
        "H = sparse(1e12, 3); H(1, 1) = 1; H(5, 2) = 2; H(1e12, 3) = 3; H(7, 3) = 4;\n\
         A = H; A(1, :) = []; B = H; B([5 1 5], :) = []; C = H; C(2:2:2e7, :) = [];\n\
         D = H; D(1e12:-1:1e12-1, :) = []; E = H; E(:, 2) = []; F = H; F([], :) = [];",
    );
    let last = HUGE - 1;
    assert_eq!(shape(&e, "A"), (HUGE - 1, 3));
    assert_eq!(
        entries(&e, "A"),
        vec![(3, 1, 2.), (5, 2, 4.), (last - 1, 2, 3.)]
    );
    // Repeated rows are deleted once.
    assert_eq!(shape(&e, "B"), (HUGE - 2, 3));
    assert_eq!(entries(&e, "B"), vec![(4, 2, 4.), (last - 2, 2, 3.)]);
    // The even rows up to 2e7: rows 1, 5, 7 become rows 1, 3, 4 and the
    // last row moves up by 1e7.
    assert_eq!(shape(&e, "C"), (HUGE - 10_000_000, 3));
    assert_eq!(
        entries(&e, "C"),
        vec![
            (0, 0, 1.),
            (2, 1, 2.),
            (3, 2, 4.),
            (last - 10_000_000, 2, 3.)
        ]
    );
    // A descending range deletes the same set as an ascending one.
    assert_eq!(shape(&e, "D"), (HUGE - 2, 3));
    assert_eq!(entries(&e, "D"), vec![(0, 0, 1.), (4, 1, 2.), (6, 2, 4.)]);
    assert_eq!(shape(&e, "E"), (HUGE, 2));
    assert_eq!(
        entries(&e, "E"),
        vec![(0, 0, 1.), (6, 1, 4.), (last, 1, 3.)]
    );
    assert!(sparse(&e, "F").equals(&sparse(&e, "H"), false));

    // Small cases agree with full deletion.
    run(
        &mut e,
        "F0 = [1 0 2 0; 0 3 0 0; 4 0 0 5; 0 6 7 0; 8 0 0 9]; S0 = sparse(F0);",
    );
    for subscript in [
        "[3 1 3], :",
        "1:2:5, :",
        "5:-2:1, :",
        "end, :",
        ":, [4 1 4]",
        ":, 2:3",
        "logical([0 1 0 1 1]), :",
    ] {
        run(
            &mut e,
            &format!("a = S0; a({subscript}) = []; b = F0; b({subscript}) = [];"),
        );
        assert!(is_sparse(&e, "a"), "{subscript}");
        assert_eq!(var(&e, "a"), var(&e, "b"), "{subscript}");
        assert_canonical(&sparse(&e, "a"));
    }
}

#[test]
fn sparse_arrays_inside_cells_round_trip_through_mat_v5() {
    let dir = temp_dir();
    let cd = format!(
        "cd('{}')",
        dir.path().display().to_string().replace('\'', "''")
    );
    for option in ["-v6", "-v7"] {
        let mut e = engine();
        e.execute(&cd).unwrap();
        run(
            &mut e,
            &format!(
                "c = {{speye(2), 3, sparse(logical([0 1; 1 0])), {{sparse([0 2.5 0])}}, sparse(4, 0)}};\n\
                 save('cell.mat', 'c', '{option}');"
            ),
        );
        let mut f = engine();
        f.execute(&cd).unwrap();
        run(
            &mut f,
            "load('cell.mat'); a = c{1}; b = c{2}; l = c{3}; inner = c{4}; d = inner{1}; z = c{5};",
        );
        assert!(
            is_sparse(&f, "a") && is_sparse(&f, "l") && is_sparse(&f, "d"),
            "{option}"
        );
        assert_eq!(entries(&f, "a"), vec![(0, 0, 1.), (1, 1, 1.)], "{option}");
        assert_eq!(var(&f, "b").2, vec![3.]);
        assert!(sparse(&f, "l").is_logical());
        assert_eq!(entries(&f, "l"), vec![(1, 0, 1.), (0, 1, 1.)]);
        assert_eq!(
            (shape(&f, "d"), entries(&f, "d")),
            ((1, 3), vec![(0, 1, 2.5)])
        );
        assert_eq!((shape(&f, "z"), sparse(&f, "z").nnz()), ((4, 0), 0));
        // The catalog charges each sparse element its sparse byte count, the
        // same count the loaded cell reports.
        let catalog =
            matrust::files::inspect_workspace(&dir.path().join("cell.mat"), &Default::default())
                .unwrap();
        let c = catalog.variables().find(|v| v.name == "c").unwrap();
        assert!(c.unavailable.is_none(), "{option}: {:?}", c.unavailable);
        assert_eq!(
            c.storage_bytes(),
            Some(f.array("c").unwrap().bytes() as u64),
            "{option}"
        );
    }
}

#[test]
fn octave_cells_with_sparse_elements_load() {
    // Written by tool/refs/sparse/make_cell_refs.m (GNU Octave 10.3):
    // c = {speye(2), 3, sparse([0 2.5 0; -1 0 0]), {sparse(4, 1)}}, d = [1 2].
    for file in ["octave-cell-v6.mat", "octave-cell-v7.mat"] {
        let path = std::path::Path::new("tests/fixtures/sparse").join(file);
        let catalog = matrust::files::inspect_workspace(&path, &Default::default()).unwrap();
        let names = ["c", "d"].iter().map(|s| s.to_string()).collect();
        let loaded =
            matrust::files::load_selected_arrays(&catalog, &names, &Default::default()).unwrap();
        let Array::Cell(c) = &loaded["c"] else {
            panic!("{file}: c is not a cell")
        };
        let element = |k: usize| c.get_linear(k).unwrap().as_ref().clone();
        let Array::Sparse(eye) = element(0) else {
            panic!("{file}: c{{1}} is not sparse")
        };
        assert_eq!(
            eye.entries().collect::<Vec<_>>(),
            vec![(0, 0, 1.), (1, 1, 1.)]
        );
        assert!(matches!(element(1), Array::Numeric(_)));
        let Array::Sparse(s) = element(2) else {
            panic!("{file}: c{{3}} is not sparse")
        };
        // Octave: find(c{3}) gives i = [2 1], j = [1 2], v = [-1 2.5].
        assert_eq!((s.rows(), s.cols()), (2, 3));
        assert_eq!(
            s.entries().collect::<Vec<_>>(),
            vec![(1, 0, -1.), (0, 1, 2.5)]
        );
        let Array::Cell(inner) = element(3) else {
            panic!("{file}: c{{4}} is not a cell")
        };
        let Array::Sparse(z) = inner.get_linear(0).unwrap().as_ref().clone() else {
            panic!("{file}: c{{4}}{{1}} is not sparse")
        };
        assert_eq!((z.rows(), z.cols(), z.nnz()), (4, 1, 0));
        assert!(matches!(loaded["d"], Array::Numeric(_)));
    }
}

#[test]
fn row_index_lists_longer_than_the_stored_entries_load() {
    // MAT files may store ir with nzmax > nnz entries: only jc[n] of them are
    // kept, at top level and inside a cell.
    fn element(out: &mut Vec<u8>, kind: u32, data: &[u8]) {
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
        out.resize(out.len().next_multiple_of(8), 0);
    }
    let int32 = |v: &[i32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<_>>();
    let sparse_array = |name: &[u8]| {
        let mut body = Vec::new();
        let mut flags = 5u32.to_le_bytes().to_vec();
        flags.extend_from_slice(&1000u32.to_le_bytes());
        element(&mut body, 6, &flags);
        element(&mut body, 5, &int32(&[3, 2]));
        element(&mut body, 1, name);
        // 1000 row indices (nzmax), of which jc says 2 are used.
        let mut ir = vec![0, 2];
        ir.extend(std::iter::repeat_n(1, 998));
        element(&mut body, 5, &int32(&ir));
        element(&mut body, 5, &int32(&[0, 1, 2]));
        let values: Vec<u8> = [4.0f64, -1.5]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        element(&mut body, 9, &values);
        body
    };
    let mut file = vec![b' '; 128];
    let text = b"MATLAB 5.0 MAT-file, hand-built sparse with nzmax > nnz";
    file[..text.len()].copy_from_slice(text);
    file[116..124].fill(0);
    file[124..128].copy_from_slice(&[0, 1, b'I', b'M']);
    element(&mut file, 14, &sparse_array(b"M"));
    let mut cell = Vec::new();
    let mut flags = 1u32.to_le_bytes().to_vec();
    flags.extend_from_slice(&0u32.to_le_bytes());
    element(&mut cell, 6, &flags);
    element(&mut cell, 5, &int32(&[1, 1]));
    element(&mut cell, 1, b"C");
    element(&mut cell, 14, &sparse_array(b""));
    element(&mut file, 14, &cell);
    let dir = temp_dir();
    std::fs::write(dir.path().join("nzmax.mat"), &file).unwrap();
    let mut e = engine();
    e.execute(&format!(
        "cd('{}')",
        dir.path().display().to_string().replace('\'', "''")
    ))
    .unwrap();
    run(&mut e, "load('nzmax.mat'); N = C{1};");
    for name in ["M", "N"] {
        assert_eq!(shape(&e, name), (3, 2), "{name}");
        assert_eq!(entries(&e, name), vec![(0, 0, 4.), (2, 1, -1.5)], "{name}");
    }
}

#[test]
fn multi_output_on_loaded_cell_element_in_fresh_session() {
    // Review rev3: a fresh session that only loads a sparse value inside a cell
    // must still route [i,j] = find(c{1}) through the sparse hook.
    for file in ["octave-cell-v6.mat", "octave-cell-v7.mat"] {
        let path = std::path::Path::new("tests/fixtures/sparse").join(file);
        let mut e = engine();
        run(&mut e, &format!("load('{}')", path.display()));
        assert_eq!(
            run(&mut e, "[i, j] = find(c{3}); disp([i j])"),
            run(&mut engine(), "disp([2 1; 1 2])"),
            "{file}"
        );
    }
}
