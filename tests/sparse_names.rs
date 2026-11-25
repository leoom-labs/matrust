//! Sparse constructor, transform and ordering coverage.
use matrust::{Array, Engine, sparse::SparseMatrix};
use std::collections::BTreeSet;

const POISSON: &str = "N = k*k; e = ones(N,1); s = e; s(k:k:N) = 0; w = [0; s(1:end-1)]; \
     A = spdiags([-e -s 4*e -w -e], [-k -1 0 1 k], N, N);";

fn engine() -> Engine {
    let mut e = Engine::default();
    e.set_sparse_full_limit(4);
    e
}
fn run(e: &mut Engine, source: &str) {
    if let Err(error) = e.execute(source) {
        panic!("{source}: {error}");
    }
}
fn error(e: &mut Engine, source: &str) -> String {
    match e.execute(source) {
        Ok(_) => panic!("{source}: expected an error"),
        Err(error) => error
            .split_once(": ")
            .filter(|(location, _)| location.starts_with("Line "))
            .map_or(error.clone(), |(_, rest)| rest.to_owned()),
    }
}
fn sparse(e: &Engine, name: &str) -> SparseMatrix {
    match e.array(name) {
        Some(Array::Sparse(s)) => s,
        other => panic!("{name} is not sparse: {other:?}"),
    }
}
fn values(e: &Engine, name: &str) -> Vec<f64> {
    match e.array(name) {
        Some(Array::Numeric(m)) => m.data().to_vec(),
        other => panic!("{name} is not numeric: {other:?}"),
    }
}
fn permutation(e: &Engine, name: &str, n: usize) -> Vec<usize> {
    let p: Vec<_> = values(e, name)
        .into_iter()
        .map(|v| v as usize - 1)
        .collect();
    assert_eq!(p.len(), n, "{name}");
    let mut sorted = p.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, (0..n).collect::<Vec<_>>(), "{name}");
    p
}

#[test]
fn sparse_random_constructors_are_seeded_sparse_and_pattern_preserving() {
    let mut e = engine();
    run(
        &mut e,
        "rng(17); S = sprand(10, 20, 0.25); rng(17); T = sprand(10, 20, 0.25);",
    );
    let s = sparse(&e, "S");
    assert_eq!((s.rows(), s.cols()), (10, 20));
    assert!(s.nnz() <= 50);
    assert!((0..s.nnz()).all(|k| s.value(k) > 0. && s.value(k) < 1.));
    let t = sparse(&e, "T");
    assert_eq!(s.colptr(), t.colptr());
    assert_eq!(s.rowidx(), t.rowidx());
    assert_eq!(
        (0..s.nnz()).map(|k| s.value(k)).collect::<Vec<_>>(),
        (0..t.nnz()).map(|k| t.value(k)).collect::<Vec<_>>()
    );
    run(
        &mut e,
        "Z = sprandn(12, 9, 0); P = sparse([1 0; 0 2]); Q = sprand(P);",
    );
    assert_eq!(sparse(&e, "Z").nnz(), 0);
    assert_eq!(sparse(&e, "P").colptr(), sparse(&e, "Q").colptr());
    assert_eq!(sparse(&e, "P").rowidx(), sparse(&e, "Q").rowidx());
    run(&mut e, "L = sprand(1e5, 1e5, 1e-6);");
    assert!(sparse(&e, "L").nnz() <= 10_000);
}

#[test]
fn sprandsym_is_symmetric_and_seeded() {
    let mut e = engine();
    run(
        &mut e,
        "rng(9); S = sprandsym(20, 0.3); rng(9); T = sprandsym(20, 0.3);",
    );
    let s = sparse(&e, "S");
    for c in 0..20 {
        for r in 0..20 {
            assert_eq!(s.get(r, c), s.get(c, r));
        }
    }
    let t = sparse(&e, "T");
    assert_eq!(s.colptr(), t.colptr());
    assert_eq!(s.rowidx(), t.rowidx());
    assert_eq!(
        (0..s.nnz()).map(|k| s.value(k)).collect::<Vec<_>>(),
        (0..t.nnz()).map(|k| t.value(k)).collect::<Vec<_>>()
    );
}

#[test]
fn spfun_calls_once_on_stored_values_and_drops_zeros() {
    let mut e = engine();
    run(
        &mut e,
        "S = sparse([0 2; -1 0]); T = spfun(@exp, S); U = spfun(@exp, [0 2; -1 0]); Z = spfun(@(x) x-x, S);",
    );
    let s = sparse(&e, "S");
    let t = sparse(&e, "T");
    assert_eq!(s.colptr(), t.colptr());
    assert_eq!(s.rowidx(), t.rowidx());
    for k in 0..s.nnz() {
        assert!((t.value(k) - s.value(k).exp()).abs() < 1e-14);
    }
    assert_eq!(t.colptr(), sparse(&e, "U").colptr());
    assert_eq!(t.rowidx(), sparse(&e, "U").rowidx());
    assert_eq!(sparse(&e, "Z").nnz(), 0);
    assert_eq!(
        error(&mut e, "spfun(@(x) x(1), S);"),
        "spfun: function must return a real vector of the same length"
    );
}

fn cholesky_pattern_nnz(s: &SparseMatrix, order: &[usize]) -> usize {
    let n = s.rows();
    let mut graph = vec![BTreeSet::new(); n];
    for c in 0..n {
        for k in s.column(c) {
            let r = s.rowidx()[k];
            if r != c {
                graph[r].insert(c);
                graph[c].insert(r);
            }
        }
    }
    let mut active = vec![true; n];
    let mut nnz = 0;
    for &v in order {
        let neighbours: Vec<_> = graph[v].iter().copied().filter(|&u| active[u]).collect();
        nnz += 1 + neighbours.len();
        for (i, &a) in neighbours.iter().enumerate() {
            for &b in &neighbours[i + 1..] {
                graph[a].insert(b);
                graph[b].insert(a);
            }
        }
        active[v] = false;
    }
    nnz
}
fn bandwidth(s: &SparseMatrix, order: &[usize]) -> usize {
    let mut position = vec![0; order.len()];
    for (i, &v) in order.iter().enumerate() {
        position[v] = i;
    }
    let mut width = 0;
    for c in 0..s.cols() {
        for k in s.column(c) {
            width = width.max(position[s.rowidx()[k]].abs_diff(position[c]));
        }
    }
    width
}

#[test]
fn sparse_orderings_return_permutations_and_reduce_poisson_structure() {
    let mut e = engine();
    run(
        &mut e,
        &format!("k = 30; {POISSON} p = amd(A); q = symamd(A, []); r = colamd(A, []);"),
    );
    let p = permutation(&e, "p", 900);
    permutation(&e, "q", 900);
    permutation(&e, "r", 900);
    let a = sparse(&e, "A");
    assert!(cholesky_pattern_nnz(&a, &p) < cholesky_pattern_nnz(&a, &(0..900).collect::<Vec<_>>()));
}
#[test]
fn symrcm_reduces_a_fixed_permuted_poisson_bandwidth() {
    let mut e = engine();
    run(
        &mut e,
        &format!("k = 30; {POISSON} rng(1); q = randperm(N); B = A(q,q); p = symrcm(B);"),
    );
    let p = permutation(&e, "p", 900);
    assert!(bandwidth(&sparse(&e, "B"), &p) <= 36);
}
