//! Square `\` and `/` with a sparse coefficient matrix, without converting it
//! to full.
//!
//! Path choice for a square sparse `A` (public mldivide documentation for
//! sparse input, simplified):
//! 1. triangular pattern: substitution (own code, cancellable per column);
//! 2. numerically symmetric with a positive diagonal: sparse Cholesky with an
//!    AMD ordering (faer); a non-positive pivot falls through to 3 (MATLAB
//!    tries LDL' first, listed under Differences);
//! 3. otherwise sparse LU with a COLAMD ordering and partial pivoting (faer).
//!
//! Storage: sparse\full gives full, sparse\sparse gives sparse, also on the
//! dense path. Singular systems, stored NaN/Inf and non-finite right-hand
//! sides of a non-triangular matrix take the dense path when `n*n` fits the
//! sparse-to-full limit (same values and warnings as `full(A)\b`); above it
//! they are errors. The warnings use the dense path's
//! identifiers; RCOND comes from the same 1-norm estimator.
//!
//! faer runs sequentially (`Par::Seq`, no rayon feature). Its numeric
//! factorizations cannot be interrupted, so they are bounded instead: the
//! Cholesky factor size is known from the symbolic step, and LU fill has the
//! George–Ng upper bound (`lu_fill_bound`). Both are checked against
//! `MAX_NNZ` and charged to the evaluation budget before any numeric work;
//! cancellation is polled before and after.
use super::sparse_fns::full_limit;
use super::*;
use crate::sparse::{MAX_NNZ, SparseMatrix, SparseValues};
use faer::Side;
use faer::dyn_stack::{MemBuffer, MemStack};
use faer::linalg::solvers::Solve;
use faer::prelude::*;
use faer::sparse::linalg::LuError;
use faer::sparse::linalg::cholesky::{LltRef, SymbolicCholesky, factorize_symbolic_cholesky};
use faer::sparse::linalg::solvers::Lu;
use faer::sparse::{SparseColMatRef, SymbolicSparseColMatRef};
use faer::{Conj, Par};
use nalgebra::DMatrix;

/// Right-hand-side columns solved together (bounds the dense block for sparse B).
const BLOCK: usize = 64;
/// The browser build aborts on allocation failure: refuse large LU there.
#[cfg(target_arch = "wasm32")]
const WASM_LU_NNZ: usize = 1_000_000;

enum Factor {
    Lower,
    Upper,
    Cholesky(SymbolicCholesky<usize>, Vec<f64>),
    Lu(Lu<usize, f64>),
}

/// Why a sparse solve could not finish on its own.
enum Fallback {
    /// Singular (no pivot or a zero diagonal) or non-finite input.
    Dense(&'static str),
}

/// `a \ b` (`backslash`) or `a / b` where the divisor is sparse, square and
/// not 1-by-1, and the other operand is real (full or sparse). `None`: the
/// caller keeps its dense path.
pub(super) fn divide(
    engine: &mut Engine,
    backslash: bool,
    a: &Value,
    b: &Value,
    out: &mut Execution,
) -> Result<Option<Value>> {
    let name = if backslash { "mldivide" } else { "mrdivide" };
    let (divisor, other) = if backslash { (a, b) } else { (b, a) };
    let Value::Container(Array::Sparse(s)) = divisor else {
        return Ok(None);
    };
    let n = s.rows();
    if n != s.cols() || n <= 1 {
        return Ok(None);
    }
    // a / b = (b' \ a')'.
    let s = if backslash { s.clone() } else { s.transpose()? };
    let rhs = match other {
        Value::Container(Array::Sparse(o)) => {
            let o = if backslash { o.clone() } else { o.transpose()? };
            Rhs::Sparse(o)
        }
        Value::Matrix(m) if !m.is_nd() => {
            Rhs::Full(if backslash { m.clone() } else { m.transpose() })
        }
        Value::Logical(m) if !m.is_nd() => {
            let m = m.to_numeric(&engine.cancel)?;
            Rhs::Full(if backslash { m } else { m.transpose() })
        }
        _ => return Ok(None),
    };
    if rhs.rows() != n {
        return Err(if backslash {
            "Linear system row dimensions must agree".into()
        } else {
            "Matrix dimensions must agree".into()
        });
    }
    if rhs.cols() == 0 {
        // n-by-0 (or 0-by-n for /), with the right-hand side's storage.
        let (r, c) = if backslash { (n, 0) } else { (0, n) };
        return Ok(Some(match rhs {
            Rhs::Sparse(_) => Value::Container(Array::Sparse(SparseMatrix::zeros(r, c, false)?)),
            Rhs::Full(_) => Value::Matrix(Matrix::from_column_vec(r, c, Vec::new())?),
        }));
    }
    let limit = full_limit(engine) as u64;
    let fits = (n as u64).checked_mul(n as u64).is_some_and(|e| e <= limit);
    let result = match solve(engine, name, &s, &rhs, out)? {
        Ok(v) => v,
        Err(Fallback::Dense(why)) => {
            if fits {
                return Ok(None);
            }
            return Err(format!("{name}: {why}"));
        }
    };
    Ok(Some(match result {
        Solved::Full(x) => Value::Matrix(if backslash { x } else { x.transpose() }),
        Solved::Sparse(x) => {
            let x = if backslash { x } else { x.transpose()? };
            Value::Container(Array::Sparse(x))
        }
    }))
}

enum Rhs {
    Full(Matrix),
    Sparse(SparseMatrix),
}
impl Rhs {
    fn rows(&self) -> usize {
        match self {
            Self::Full(m) => m.rows(),
            Self::Sparse(s) => s.rows(),
        }
    }
    fn cols(&self) -> usize {
        match self {
            Self::Full(m) => m.cols(),
            Self::Sparse(s) => s.cols(),
        }
    }
    /// Columns `c0..c1` as a dense column-major block.
    fn block(&self, c0: usize, c1: usize) -> Vec<f64> {
        let n = self.rows();
        match self {
            Self::Full(m) => m.data()[c0 * n..c1 * n].to_vec(),
            Self::Sparse(s) => {
                let mut v = vec![0.; n * (c1 - c0)];
                for c in c0..c1 {
                    for k in s.column(c) {
                        v[(c - c0) * n + s.rowidx()[k]] = s.value(k);
                    }
                }
                v
            }
        }
    }
    fn all_finite(&self) -> bool {
        match self {
            Self::Full(m) => m.data().iter().all(|v| v.is_finite()),
            Self::Sparse(s) => (0..s.nnz()).all(|k| s.value(k).is_finite()),
        }
    }
}

enum Solved {
    Full(Matrix),
    Sparse(SparseMatrix),
}

/// Stored values as doubles (logical sparse arrays hold booleans).
fn doubles(s: &SparseMatrix) -> std::borrow::Cow<'_, [f64]> {
    match s.values() {
        SparseValues::Double(v) => std::borrow::Cow::Borrowed(v),
        SparseValues::Logical(v) => {
            std::borrow::Cow::Owned(v.iter().map(|&b| f64::from(u8::from(b))).collect())
        }
    }
}

pub(super) fn faer_ref<'a>(
    s: &'a SparseMatrix,
    values: &'a [f64],
) -> SparseColMatRef<'a, usize, f64> {
    let symbolic =
        SymbolicSparseColMatRef::new_checked(s.rows(), s.cols(), s.colptr(), None, s.rowidx());
    SparseColMatRef::new(symbolic, values)
}

/// Upper bound on nnz(L) + nnz(U) for faer's sparse LU of the square `s`
/// (its default COLAMD column order `Q`, any row pivoting). George and Ng
/// (1985): struct(U) lies in struct(R) and struct(L) in struct(R') where
/// R'R is the Cholesky factorization of (AQ)'(AQ); R's column counts come
/// from the column elimination tree without forming A'A. O(nnz(A)) memory.
pub(super) fn lu_fill_bound(s: &SparseMatrix) -> Result<u64> {
    use faer::perm::PermRef;
    use faer::sparse::linalg::{colamd, qr};
    let n = s.cols();
    let nnz = s.nnz();
    let a = SymbolicSparseColMatRef::new_checked(n, n, s.colptr(), None, s.rowidx());
    let t = s.transpose()?;
    let at = SymbolicSparseColMatRef::new_checked(n, n, t.colptr(), None, t.rowidx());
    let oom = || "sparse LU analysis: out of memory".to_string();
    let req = faer::dyn_stack::StackReq::any_of(&[
        colamd::order_scratch::<usize>(n, n, nnz),
        qr::col_etree_scratch::<usize>(n, n),
        qr::postorder_scratch::<usize>(n),
        qr::column_counts_aat_scratch::<usize>(n, n),
    ]);
    let mut mem = MemBuffer::try_new(req).map_err(|_| oom())?;
    let (mut fwd, mut inv) = (vec![0usize; n], vec![0usize; n]);
    colamd::order(
        &mut fwd,
        &mut inv,
        a,
        Default::default(),
        MemStack::new(&mut mem),
    )
    .map_err(|_| oom())?;
    let perm = PermRef::new_checked(&fwd, &inv, n);
    let mut etree = vec![0usize; n];
    let etree = qr::col_etree(a, Some(perm), &mut etree, MemStack::new(&mut mem));
    let mut post = vec![0usize; n];
    qr::postorder(&mut post, etree, MemStack::new(&mut mem));
    let (mut counts, mut min_col) = (vec![0usize; n], vec![0usize; n]);
    qr::column_counts_ata(
        &mut counts,
        &mut min_col,
        at,
        Some(perm),
        etree,
        &post,
        MemStack::new(&mut mem),
    );
    let r: u64 = counts.iter().map(|&c| c as u64).sum();
    Ok(r.saturating_mul(2))
}

/// Rejects an LU whose fill bound is above `MAX_NNZ`; returns the bound
/// (callers with a mutable engine charge it).
pub(super) fn check_lu_fill(engine: &Engine, name: &str, s: &SparseMatrix) -> Result<u64> {
    let bound = lu_fill_bound(s)?;
    if bound > MAX_NNZ {
        return Err(format!(
            "{name}: the sparse LU factors may need up to {bound} nonzeros, more than {MAX_NNZ}"
        ));
    }
    engine.check_cancel()?;
    Ok(bound)
}

/// Triangular pattern: Some(true) lower, Some(false) upper, None neither.
/// A diagonal matrix counts as lower.
fn triangular(s: &SparseMatrix) -> Option<bool> {
    let rows = s.rowidx();
    let lower = (0..s.cols()).all(|c| s.column(c).all(|k| rows[k] >= c));
    if lower {
        return Some(true);
    }
    let upper = (0..s.cols()).all(|c| s.column(c).all(|k| rows[k] <= c));
    upper.then_some(false)
}

/// Equal to its transpose (pattern and values) with a positive diagonal; O(nnz).
fn cholesky_candidate(s: &SparseMatrix, values: &[f64]) -> Result<bool> {
    let n = s.cols();
    let rows = s.rowidx();
    for c in 0..n {
        let range = s.column(c);
        match range.clone().find(|&k| rows[k] == c) {
            Some(k) if values[k] > 0. => {}
            _ => return Ok(false),
        }
    }
    let t = s.transpose()?;
    if t.colptr() != s.colptr() || t.rowidx() != s.rowidx() {
        return Ok(false);
    }
    let tv = doubles(&t);
    Ok(tv.iter().zip(values).all(|(a, b)| a == b))
}

fn norm1(s: &SparseMatrix, values: &[f64]) -> f64 {
    (0..s.cols())
        .map(|c| s.column(c).map(|k| values[k].abs()).sum::<f64>())
        .fold(0., f64::max)
}

/// Substitution with a triangular sparse matrix, in place on `x` (n rows).
/// `transpose` solves with the transpose (an upper/lower solve read by rows).
fn substitute(s: &SparseMatrix, values: &[f64], lower: bool, transpose: bool, x: &mut [f64]) {
    let n = s.cols();
    let rows = s.rowidx();
    let diag = |c: usize| {
        s.column(c)
            .find(|&k| rows[k] == c)
            .map_or(0., |k| values[k])
    };
    if !transpose {
        // Column-oriented: x(c) /= a(c,c); x(rows) -= a(rows,c) * x(c).
        let order: Box<dyn Iterator<Item = usize>> = if lower {
            Box::new(0..n)
        } else {
            Box::new((0..n).rev())
        };
        for c in order {
            let v = x[c] / diag(c);
            x[c] = v;
            for k in s.column(c) {
                if rows[k] != c {
                    x[rows[k]] -= values[k] * v;
                }
            }
        }
    } else {
        // A' lower is upper: x(c) = (x(c) - a(:,c)' x) / a(c,c), column c read whole.
        let order: Box<dyn Iterator<Item = usize>> = if lower {
            Box::new((0..n).rev())
        } else {
            Box::new(0..n)
        };
        for c in order {
            let mut v = x[c];
            for k in s.column(c) {
                if rows[k] != c {
                    v -= values[k] * x[rows[k]];
                }
            }
            x[c] = v / diag(c);
        }
    }
}

fn solve(
    engine: &mut Engine,
    name: &str,
    s: &SparseMatrix,
    rhs: &Rhs,
    out: &mut Execution,
) -> Result<std::result::Result<Solved, Fallback>> {
    let n = s.cols();
    let values = doubles(s);
    if !values.iter().all(|v| v.is_finite()) {
        return Ok(Err(Fallback::Dense(
            "sparse matrices with NaN or Inf values are not supported in a solve yet",
        )));
    }
    let tri = triangular(s);
    // Substitution touches only stored entries, so a NaN/Inf in the
    // right-hand side stays in the components that depend on it (as Octave).
    if tri.is_none() && !rhs.all_finite() {
        return Ok(Err(Fallback::Dense(
            "right-hand sides with NaN or Inf values are not supported for sparse matrices yet",
        )));
    }
    engine.check_cancel()?;
    let factor = match tri {
        Some(lower) => {
            let rows = s.rowidx();
            if (0..n).any(|c| !s.column(c).any(|k| rows[k] == c && values[k] != 0.)) {
                return Ok(Err(Fallback::Dense(
                    "matrix is singular to working precision (zero on the diagonal)",
                )));
            }
            if lower { Factor::Lower } else { Factor::Upper }
        }
        None => {
            let a = faer_ref(s, &values);
            let mut chol = None;
            if cholesky_candidate(s, &values)? {
                let symbolic = factorize_symbolic_cholesky(
                    a.symbolic(),
                    Side::Lower,
                    Default::default(),
                    Default::default(),
                )
                .map_err(|e| format!("{name}: sparse Cholesky analysis failed ({e:?})"))?;
                let factor_nnz = symbolic.len_val();
                if factor_nnz as u64 > MAX_NNZ {
                    return Err(format!(
                        "{name}: the sparse Cholesky factor needs {factor_nnz} nonzeros, more than {MAX_NNZ}"
                    ));
                }
                engine.charge(factor_nnz)?;
                engine.check_cancel()?;
                let mut l = Vec::new();
                l.try_reserve_exact(factor_nnz)
                    .map_err(|_| format!("{name}: out of memory for the sparse Cholesky factor"))?;
                l.resize(factor_nnz, 0.);
                let mut mem = MemBuffer::try_new(
                    symbolic.factorize_numeric_llt_scratch::<f64>(Par::Seq, Default::default()),
                )
                .map_err(|_| format!("{name}: out of memory for the sparse Cholesky workspace"))?;
                let ok = symbolic
                    .factorize_numeric_llt(
                        &mut l,
                        a,
                        Side::Lower,
                        Default::default(),
                        Par::Seq,
                        MemStack::new(&mut mem),
                        Default::default(),
                    )
                    .is_ok();
                if ok {
                    chol = Some((symbolic, l));
                }
                engine.check_cancel()?;
            }
            match chol {
                Some((symbolic, l)) => Factor::Cholesky(symbolic, l),
                None => {
                    #[cfg(target_arch = "wasm32")]
                    if s.nnz() > WASM_LU_NNZ {
                        return Err(format!(
                            "{name}: sparse LU with more than {WASM_LU_NNZ} nonzeros is not supported in the browser"
                        ));
                    }
                    let bound = check_lu_fill(engine, name, s)?;
                    engine.charge(bound as usize)?;
                    match a.sp_lu() {
                        Ok(f) => {
                            engine.check_cancel()?;
                            Factor::Lu(f)
                        }
                        Err(LuError::SymbolicSingular { .. }) => {
                            return Ok(Err(Fallback::Dense(
                                "matrix is singular to working precision (sparse LU found no pivot)",
                            )));
                        }
                        Err(LuError::Generic(e)) => {
                            return Err(format!("{name}: sparse LU failed ({e:?})"));
                        }
                    }
                }
            }
        }
    };
    let apply = |x: &mut [f64], k: usize, transpose: bool| match &factor {
        Factor::Lower | Factor::Upper => {
            let lower = matches!(factor, Factor::Lower);
            for c in 0..k {
                substitute(s, &values, lower, transpose, &mut x[c * n..(c + 1) * n]);
            }
        }
        Factor::Cholesky(symbolic, l) => {
            // Symmetric: the transpose solve is the same solve.
            let m = MatMut::from_column_major_slice_mut(x, n, k);
            let mut mem = MemBuffer::new(symbolic.solve_in_place_scratch::<f64>(k, Par::Seq));
            LltRef::new(symbolic, l).solve_in_place_with_conj(
                Conj::No,
                m,
                Par::Seq,
                MemStack::new(&mut mem),
            );
        }
        Factor::Lu(f) => {
            let m = MatMut::from_column_major_slice_mut(x, n, k);
            if transpose {
                f.solve_transpose_in_place(m);
            } else {
                f.solve_in_place(m);
            }
        }
    };
    let total = rhs.cols();
    let mut full: Vec<f64> = Vec::new();
    let mut sparse = match rhs {
        Rhs::Sparse(_) => Some(crate::sparse::Builder::new(n, total, false, 0)?),
        Rhs::Full(_) => {
            engine.charge(n.saturating_mul(total))?;
            full.reserve_exact(n * total);
            None
        }
    };
    let mut c0 = 0;
    while c0 < total {
        engine.check_cancel()?;
        let c1 = (c0 + BLOCK).min(total);
        let mut x = rhs.block(c0, c1);
        apply(&mut x, c1 - c0, false);
        if tri.is_none() && x.iter().any(|v| !v.is_finite()) {
            return Ok(Err(Fallback::Dense(
                "matrix is singular to working precision (sparse LU found no pivot)",
            )));
        }
        match &mut sparse {
            Some(builder) => {
                for c in 0..c1 - c0 {
                    for (r, &v) in x[c * n..(c + 1) * n].iter().enumerate() {
                        builder.push(r, v)?;
                    }
                    builder.end_column();
                }
            }
            None => full.extend_from_slice(&x),
        }
        c0 = c1;
    }
    // RCOND from the 1-norm estimate, as the dense path.
    let estimate = engine.lax_inverse_norm1(n, |v: &DMatrix<f64>, transpose: bool| {
        let mut x: Vec<f64> = v.iter().copied().collect();
        apply(&mut x, 1, transpose);
        Some(DMatrix::from_column_slice(n, 1, &x))
    })?;
    engine.real_condition_warning(norm1(s, &values), estimate, out)?;
    Ok(Ok(match sparse {
        Some(builder) => Solved::Sparse(builder.finish()),
        None => Solved::Full(Matrix::from_column_vec(n, total, full)?),
    }))
}
