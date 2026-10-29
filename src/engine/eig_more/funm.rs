//! funm: the Schur-Parlett algorithm of P. I. Davies and N. J. Higham, "A
//! Schur-Parlett algorithm for computing matrix functions", SIAM J. Matrix
//! Anal. Appl. 25 (2003), as presented in N. J. Higham, Functions of
//! Matrices (SIAM, 2008), ch. 9: complex Schur form, blocking of close
//! eigenvalues and reordering by swaps (§9.3), Taylor series within each
//! atomic block (§9.1) and the block Parlett recurrence (§9.2).
// LAPACK ports keep the reference routines' index loops.
#![allow(clippy::needless_range_loop)]
use super::cdense::{self, CMat, c, cabs, cdiv, cmulr};
use super::{Mat, Num, ordering, schur};
use crate::engine::{Engine, Execution, Value};
use crate::{Array, Complex64 as C, FunctionHandle, Result};
use crate::{CellArray, ComplexMatrix, Matrix, StructArray};
use std::sync::Arc;

const ZERO: C = C { re: 0., im: 0. };

/// The function to apply: a known elementary function or a handle f(x, k).
enum Fun {
    Exp,
    Sin,
    Cos,
    Log,
    Sinh,
    Cosh,
    Handle(FunctionHandle),
}

fn cexp(z: C) -> C {
    let e = libm::exp(z.re);
    c(e * libm::cos(z.im), e * libm::sin(z.im))
}
fn csin(z: C) -> C {
    c(
        libm::sin(z.re) * libm::cosh(z.im),
        libm::cos(z.re) * libm::sinh(z.im),
    )
}
fn ccos(z: C) -> C {
    c(
        libm::cos(z.re) * libm::cosh(z.im),
        -(libm::sin(z.re) * libm::sinh(z.im)),
    )
}
fn csinh(z: C) -> C {
    c(
        libm::sinh(z.re) * libm::cos(z.im),
        libm::cosh(z.re) * libm::sin(z.im),
    )
}
fn ccosh(z: C) -> C {
    c(
        libm::cosh(z.re) * libm::cos(z.im),
        libm::sinh(z.re) * libm::sin(z.im),
    )
}
fn clog(z: C) -> C {
    c(libm::log(cabs(z)), libm::atan2(z.im, z.re))
}

fn factorial(k: usize) -> f64 {
    (1..=k).fold(1., |p, i| p * i as f64)
}

/// norm(X, inf) / scale: the largest absolute row sum, in units of `scale`
/// (a power of two, so dividing is exact). NaN when any entry is NaN.
fn norm_inf(x: &CMat, scale: f64) -> f64 {
    if x.data.iter().any(|z| z.re.is_nan() || z.im.is_nan()) {
        return f64::NAN;
    }
    (0..x.rows)
        .map(|i| {
            (0..x.cols)
                .map(|j| {
                    let z = x.at(i, j);
                    cabs(c(z.re / scale, z.im / scale))
                })
                .sum::<f64>()
        })
        .fold(0., f64::max)
}

/// True when every entry is a finite number.
fn all_finite(x: &CMat) -> bool {
    x.data.iter().all(|z| z.re.is_finite() && z.im.is_finite())
}

/// The unit for norms of finite matrices `x` and `y` whose row sums could
/// overflow: 1 up to entries of 2^500, else the largest power of two not above
/// the largest entry component, which keeps every |entry| / unit below 3.
fn norm_unit(x: &CMat, y: &CMat) -> f64 {
    let big = x
        .data
        .iter()
        .chain(&y.data)
        .fold(0f64, |m, z| m.max(z.re.abs()).max(z.im.abs()));
    if big > 2f64.powi(500) {
        f64::from_bits(big.to_bits() & (0x7ff << 52))
    } else {
        1.
    }
}

/// norm(X, 1) of a real matrix.
fn norm1(x: &Mat) -> f64 {
    (0..x.cols)
        .map(|j| (0..x.rows).map(|i| x.at(i, j).abs()).sum::<f64>())
        .fold(0., f64::max)
}

const NOT_SQUARE: &str = "First input must be a single or double square matrix.";

/// funm's options: TolBlk (blocking tolerance delta), TolTay (Taylor
/// tolerance), MaxTerms and the optional ordering labels Ord.
struct Options {
    tol_blk: f64,
    tol_tay: f64,
    max_terms: usize,
    ord: Option<Vec<usize>>,
}

/// The numbers in an option field; None for other classes.
fn option_values(a: &Array) -> Option<Vec<f64>> {
    match a {
        Array::Numeric(m) => Some(m.data().to_vec()),
        Array::Logical(m) => Some(m.data().iter().map(|&b| f64::from(b)).collect()),
        Array::Typed(t) => t.to_matrix().ok().map(|m| m.data().to_vec()),
        _ => None,
    }
}

/// Options from funm's third argument (a scalar struct or []) for an n x n A.
/// Unknown fields and empty values are ignored. MaxSqrt is validated only:
/// every atomic block, the logarithm's included, uses the Taylor series.
fn parse_options(value: Option<&Value>, n: usize) -> Result<Options> {
    let mut opts = Options {
        tol_blk: 0.1,
        tol_tay: f64::EPSILON,
        max_terms: 250,
        ord: None,
    };
    let s = match value {
        None => return Ok(opts),
        Some(Value::Matrix(m)) if m.is_empty() => return Ok(opts),
        Some(Value::Container(Array::Struct(s))) if s.len() == 1 => s,
        Some(_) => return Err("Options must be a scalar structure.".into()),
    };
    for (index, name) in s.field_names().iter().enumerate() {
        let key = name.to_ascii_lowercase();
        if !matches!(
            key.as_str(),
            "tolblk" | "toltay" | "maxterms" | "maxsqrt" | "ord"
        ) {
            continue;
        }
        let values = option_values(s.get(index, 0)?)
            .ok_or_else(|| format!("Option {name} must be numeric."))?;
        if values.is_empty() {
            continue;
        }
        let scalar = (values.len() == 1).then(|| values[0]);
        let tolerance = |what: &str| match scalar {
            Some(x) if x >= 0. => Ok(x),
            _ => Err(format!("{what} must be a nonnegative real scalar.")),
        };
        let count = |what: &str| match scalar {
            Some(x) if (1.0..=1e6).contains(&x) && x.fract() == 0. => Ok(x as usize),
            _ => Err(format!("{what} must be an integer from 1 to 1000000.")),
        };
        match key.as_str() {
            "tolblk" => opts.tol_blk = tolerance("TolBlk")?,
            "toltay" => opts.tol_tay = tolerance("TolTay")?,
            "maxterms" => opts.max_terms = count("MaxTerms")?,
            "maxsqrt" => {
                count("MaxSqrt")?;
            }
            _ => {
                if values.len() != n
                    || !values
                        .iter()
                        .all(|&x| (1.0..=n as f64).contains(&x) && x.fract() == 0.)
                {
                    return Err("Ord must have one integer label in 1:n per eigenvalue.".into());
                }
                opts.ord = Some(values.iter().map(|&x| x as usize).collect());
            }
        }
    }
    Ok(opts)
}

/// The function argument: the elementary functions whose derivatives are
/// known in closed form, or any other handle called as fun(x, k).
fn function_of(value: &Value) -> Result<Fun> {
    let Value::Container(Array::Handle(h)) = value else {
        return Err("Second input must be a function handle.".into());
    };
    if h.file().is_none() {
        match h.name() {
            Some("exp") => return Ok(Fun::Exp),
            Some("sin") => return Ok(Fun::Sin),
            Some("cos") => return Ok(Fun::Cos),
            Some("log") => return Ok(Fun::Log),
            Some("sinh") => return Ok(Fun::Sinh),
            Some("cosh") => return Ok(Fun::Cosh),
            _ => {}
        }
    }
    Ok(Fun::Handle(h.clone()))
}

/// z^k by repeated squaring.
fn cpowi(z: C, mut k: usize) -> C {
    let (mut base, mut acc) = (z, c(1., 0.));
    while k > 0 {
        if k & 1 == 1 {
            acc *= base;
        }
        k >>= 1;
        if k > 0 {
            base *= base;
        }
    }
    acc
}

/// The largest modulus; NaN when any value is NaN.
fn max_abs(x: &[C]) -> f64 {
    x.iter().fold(0., |m, &z| {
        let a = cabs(z);
        if m.is_nan() || a.is_nan() {
            f64::NAN
        } else {
            m.max(a)
        }
    })
}

/// A*B for upper triangular A and B. Checks `cancel` once per column and
/// returns the columns done so far when it fires.
fn upper_mul(a: &CMat, b: &CMat, cancel: &dyn Fn() -> bool) -> CMat {
    let n = a.rows;
    let mut out = CMat::zeros(n, n);
    for j in 0..n {
        if cancel() {
            return out;
        }
        for k in 0..=j {
            let bkj = b.at(k, j);
            if bkj == ZERO {
                continue;
            }
            for i in 0..=k {
                out.data[i + j * n] += a.at(i, k) * bkj;
            }
        }
    }
    out
}

/// Index sets of the blocks for ordschur-style labels: the eigenvalues with
/// the highest label form the leading block, and so on down.
fn cluster_blocks(ord: &[usize]) -> Vec<Vec<usize>> {
    let mut labels = ord.to_vec();
    labels.sort_unstable_by(|a, b| b.cmp(a));
    labels.dedup();
    let mut start = 0;
    labels
        .iter()
        .map(|&l| {
            let size = ord.iter().filter(|&&x| x == l).count();
            start += size;
            (start - size..start).collect()
        })
        .collect()
}

/// Block pattern of the triangular Schur factor (Higham 2008, §9.3, the
/// blocking algorithm of Davies and Higham 2003, §4): eigenvalues t_ii and
/// t_jj with |t_ii - t_jj| <= delta share a set, and sets that become linked
/// are merged, so eigenvalues of different sets are more than delta apart.
/// Returns the set of each eigenvalue, numbered from 1 in order of first
/// appearance along the diagonal.
fn blocking(t: &CMat, delta: f64) -> Vec<usize> {
    let n = t.rows;
    // 0 marks an eigenvalue that no set holds yet.
    let mut m = vec![0usize; n];
    let mut sets = 0;
    for i in 0..n {
        if m[i] == 0 {
            sets += 1;
            m[i] = sets;
        }
        let ti = t.at(i, i);
        for j in i + 1..n {
            if m[j] == m[i] || cabs(ti - t.at(j, j)) > delta {
                continue;
            }
            if m[j] == 0 {
                m[j] = m[i];
            } else {
                // Merge the later-numbered set into the earlier one and
                // close the gap in the numbering.
                let (keep, gone) = (m[i].min(m[j]), m[i].max(m[j]));
                for x in m.iter_mut() {
                    if *x == gone {
                        *x = keep;
                    } else if *x > gone {
                        *x -= 1;
                    }
                }
                sets -= 1;
            }
        }
    }
    m
}

/// Confluent ordering of the sets of `blocking` (Higham 2008, §9.3; Davies
/// and Higham 2003, §4): the sets are placed in increasing order of the mean
/// position of their eigenvalues, and each set in turn is gathered below the
/// previous ones by adjacent swaps, its eigenvalues keeping their relative
/// order. Returns the label of each eigenvalue in ordschur's cluster
/// convention (the highest label ends top-left), which these swaps realise,
/// and the index sets of the blocks of the reordered factor.
fn swapping(m: &[usize]) -> (Vec<usize>, Vec<Vec<usize>>) {
    let sets = m.iter().copied().max().unwrap_or(0);
    let mut sum = vec![0u128; sets];
    let mut size = vec![0u128; sets];
    for (i, &k) in m.iter().enumerate() {
        sum[k - 1] += i as u128;
        size[k - 1] += 1;
    }
    // Compare means exactly: sum_a/size_a < sum_b/size_b. Ties keep the
    // first-appearance order (stable sort).
    let mut order: Vec<usize> = (0..sets).collect();
    order.sort_by(|&a, &b| (sum[a] * size[b]).cmp(&(sum[b] * size[a])));
    let mut rank = vec![0; sets];
    for (r, &k) in order.iter().enumerate() {
        rank[k] = r;
    }
    let ord: Vec<usize> = m.iter().map(|&k| sets - rank[k - 1]).collect();
    let blocks = cluster_blocks(&ord);
    (ord, blocks)
}

/// Solves A X - X B = C for upper triangular A (p x p) and B (q x q),
/// a column at a time by back substitution (the triangular case of the
/// Bartels-Stewart method; Higham 2008, §9.2). Checks `cancel` once per
/// column and returns the columns done so far when it fires.
fn sylvester(a: &CMat, b: &CMat, rhs: &CMat, cancel: &dyn Fn() -> bool) -> CMat {
    let (p, q) = (a.rows, b.rows);
    let mut x = CMat::zeros(p, q);
    for col in 0..q {
        if cancel() {
            return x;
        }
        let mut r: Vec<C> = (0..p).map(|i| rhs.at(i, col)).collect();
        for l in 0..col {
            let blc = b.at(l, col);
            for i in 0..p {
                r[i] += x.at(i, l) * blc;
            }
        }
        let shift = b.at(col, col);
        for i in (0..p).rev() {
            let mut s = r[i];
            for l in i + 1..p {
                s -= a.at(i, l) * x.at(l, col);
            }
            x.set(i, col, cdiv(s, a.at(i, i) - shift));
        }
    }
    x
}

/// The submatrix of `m` on the half-open row and column ranges.
fn sub(m: &CMat, rows: (usize, usize), cols: (usize, usize)) -> CMat {
    let mut s = CMat::zeros(rows.1 - rows.0, cols.1 - cols.0);
    for j in cols.0..cols.1 {
        for i in rows.0..rows.1 {
            s.set(i - rows.0, j - cols.0, m.at(i, j));
        }
    }
    s
}

/// Block Parlett recurrence (Higham 2008, §9.2; Davies and Higham 2003, §3):
/// with the diagonal blocks of F = f(T) in place, each block F_ij, i < j,
/// solves the Sylvester equation
///   T_ii F_ij - F_ij T_jj = F_ii T_ij - T_ij F_jj
///                           + sum_{k=i+1}^{j-1} (F_ik T_kj - T_ik F_kj),
/// a block column at a time from the diagonal up. Two 1x1 blocks give the
/// scalar Parlett recurrence. Returns false when cancelled.
fn parlett(t: &CMat, f: &mut CMat, ranges: &[(usize, usize)], cancel: &dyn Fn() -> bool) -> bool {
    for jb in 1..ranges.len() {
        if cancel() {
            return false;
        }
        let (sj, ej) = ranges[jb];
        for ib in (0..jb).rev() {
            let (si, ei) = ranges[ib];
            if ei - si == 1 && ej - sj == 1 {
                let (i, j) = (si, sj);
                let mut s = t.at(i, j) * (f.at(i, i) - f.at(j, j));
                for k in i + 1..j {
                    s += f.at(i, k) * t.at(k, j) - t.at(i, k) * f.at(k, j);
                }
                f.set(i, j, cdiv(s, t.at(i, i) - t.at(j, j)));
                continue;
            }
            // F_ii T_ij + sum F_ik T_kj covers columns si..sj of F's rows;
            // T_ij F_jj + sum T_ik F_kj covers rows ei..ej of F's columns.
            let mut rhs = CMat::zeros(ei - si, ej - sj);
            for col in sj..ej {
                for row in si..ei {
                    let mut s = ZERO;
                    for l in si..sj {
                        s += f.at(row, l) * t.at(l, col);
                    }
                    for l in ei..ej {
                        s -= t.at(row, l) * f.at(l, col);
                    }
                    rhs.set(row - si, col - sj, s);
                }
            }
            let x = sylvester(
                &sub(t, (si, ei), (si, ei)),
                &sub(t, (sj, ej), (sj, ej)),
                &rhs,
                cancel,
            );
            if cancel() {
                return false;
            }
            for col in sj..ej {
                for row in si..ei {
                    f.set(row, col, x.at(row - si, col - sj));
                }
            }
        }
    }
    true
}

impl Engine {
    /// The k-th derivative of fun at each point of x.
    fn lx3_funm_eval(
        &mut self,
        fun: &Fun,
        x: &[C],
        k: usize,
        out: &mut Execution,
    ) -> Result<Vec<C>> {
        if x.is_empty() {
            return Ok(Vec::new());
        }
        let map = |g: fn(C) -> C, sign: f64| x.iter().map(|&z| cmulr(g(z), sign)).collect();
        Ok(match fun {
            Fun::Exp => map(cexp, 1.),
            Fun::Sin => match k % 4 {
                0 => map(csin, 1.),
                1 => map(ccos, 1.),
                2 => map(csin, -1.),
                _ => map(ccos, -1.),
            },
            Fun::Cos => match k % 4 {
                0 => map(ccos, 1.),
                1 => map(csin, -1.),
                2 => map(ccos, -1.),
                _ => map(csin, 1.),
            },
            Fun::Sinh if k.is_multiple_of(2) => map(csinh, 1.),
            Fun::Sinh => map(ccosh, 1.),
            Fun::Cosh if k.is_multiple_of(2) => map(ccosh, 1.),
            Fun::Cosh => map(csinh, 1.),
            Fun::Log if k == 0 => map(clog, 1.),
            Fun::Log => {
                // d^k/dx^k log(x) = (-1)^(k-1) (k-1)! / x^k; 171! overflows.
                let scale = if k > 171 {
                    f64::INFINITY
                } else {
                    factorial(k - 1)
                };
                let sign = if k % 2 == 1 { scale } else { -scale };
                x.iter()
                    .map(|&z| cmulr(cpowi(cdiv(c(1., 0.), z), k), sign))
                    .collect()
            }
            Fun::Handle(h) => {
                let point = if x.iter().all(|z| z.im == 0.) {
                    Value::Matrix(Matrix::from_column_vec(
                        x.len(),
                        1,
                        x.iter().map(|z| z.re).collect(),
                    )?)
                } else {
                    Value::Complex(ComplexMatrix::from_column_vec(
                        x.len(),
                        1,
                        x.to_vec(),
                        &self.cancel,
                    )?)
                };
                let order = Value::Matrix(Matrix::scalar(k as f64));
                let mut r = self.call_handle(h, vec![point, order], 1, None, out)?;
                if r.is_empty() {
                    return Err("FUN must return a value.".into());
                }
                let v = self.lx3_operand("funm", &r.remove(0))?.num;
                if v.rows() * v.cols() != x.len() {
                    return Err(
                        "FUN(X, K) must return an array with the number of elements of X.".into(),
                    );
                }
                v.complex().data
            }
        })
    }

    pub(super) fn lx3_funm(
        &mut self,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        if args.len() < 2 {
            return Err("Not enough input arguments.".into());
        }
        if args.len() > 3 {
            return Err("Too many input arguments.".into());
        }
        let a = self.lx3_operand("funm", &args[0]).map_err(|e| {
            if e.starts_with("Undefined function") {
                NOT_SQUARE.to_string()
            } else {
                e
            }
        })?;
        let n = a.num.rows();
        if a.num.cols() != n {
            return Err(NOT_SQUARE.into());
        }
        let fun = function_of(&args[1])?;
        let opts = parse_options(args.get(2), n)?;
        if !a.num.finite() {
            return Err("Input to SCHUR must not contain NaN or Inf.".into());
        }
        self.lx3_funm_budget(n)?;
        let real_input = matches!(a.num, Num::R(_));

        // A = U T U' with T upper triangular (complex Schur form). A
        // triangular A is its own Schur form.
        let ac = a.num.complex();
        let mut unitary = !schur::is_upper(&ac);
        let (mut u, mut t) = if !unitary {
            (CMat::identity(n), ac)
        } else {
            let schur_form = {
                let cancel = self.lx3_cancelled();
                match &a.num {
                    Num::R(m) => {
                        schur::gees(m, &cancel).map(|(u, t)| schur::rsf2csf(&u, &t, &cancel))
                    }
                    Num::C(m) => schur::zgees(m, &cancel),
                }
            };
            self.check_cancel()?;
            schur_form.map_err(|_| "Schur decomposition failed to converge.".to_string())?
        };

        // Labels and blocks: given, one block per eigenvalue for a diagonal
        // T, or the blocking of close eigenvalues and its confluent order.
        let (ord, ind) = match opts.ord {
            Some(ord) => {
                let ind = cluster_blocks(&ord);
                (ord, ind)
            }
            None if schur::is_diag(&t) => {
                ((1..=n).rev().collect(), (0..n).map(|i| vec![i]).collect())
            }
            None => swapping(&blocking(&t, opts.tol_blk)),
        };
        if ord.windows(2).any(|w| w[0] < w[1]) {
            let moved = ordering::zordschur_clusters(&mut u, &mut t, &ord, &self.lx3_cancelled());
            self.check_cancel()?;
            moved?;
            unitary = true;
        }

        let mut terms = vec![0.; ind.len()];
        let mut flag = 0.;
        let mut ft = CMat::zeros(n, n);
        if schur::is_diag(&t) {
            let d: Vec<C> = (0..n).map(|i| t.at(i, i)).collect();
            let fd = self.lx3_funm_eval(&fun, &d, 0, out)?;
            for (i, &v) in fd.iter().enumerate() {
                ft.set(i, i, v);
            }
        } else {
            // All 1x1 blocks in one evaluation, then the atomic blocks.
            let singles: Vec<usize> = ind.iter().filter(|b| b.len() == 1).map(|b| b[0]).collect();
            if !singles.is_empty() {
                let d: Vec<C> = singles.iter().map(|&i| t.at(i, i)).collect();
                let fd = self.lx3_funm_eval(&fun, &d, 0, out)?;
                for (&i, &v) in singles.iter().zip(&fd) {
                    ft.set(i, i, v);
                }
            }
            for (b, block) in ind.iter().enumerate().filter(|(_, b)| b.len() > 1) {
                let (fb, used) =
                    self.lx3_funm_atom(&t, block, &fun, opts.tol_tay, opts.max_terms, out)?;
                terms[b] = used.unwrap_or(opts.max_terms) as f64;
                if used.is_none() {
                    flag = 1.;
                }
                for (q, &col) in block.iter().enumerate() {
                    for (p, &row) in block.iter().enumerate() {
                        ft.set(row, col, fb.at(p, q));
                    }
                }
            }
            let ranges: Vec<(usize, usize)> = ind.iter().map(|b| (b[0], b[0] + b.len())).collect();
            let done = parlett(&t, &mut ft, &ranges, &self.lx3_cancelled());
            self.check_cancel()?;
            if !done {
                return Err("Calculation cancelled".into());
            }
        }
        let f = if unitary {
            let f = {
                let cancel = self.lx3_cancelled();
                u.mul(&ft, &cancel).mul(&u.adjoint(), &cancel)
            };
            self.check_cancel()?;
            f
        } else {
            ft
        };

        // f(A) is real for real A when f is real on the real axis and
        // f(conj z) = conj f(z) (Higham 2008, Theorem 1.18): always so for
        // the known functions but the logarithm; otherwise drop an
        // imaginary part at rounding level.
        let real_function = matches!(fun, Fun::Exp | Fun::Sin | Fun::Cos | Fun::Sinh | Fun::Cosh);
        let negligible = || norm1(&f.im()) <= 10. * n as f64 * f64::EPSILON * cnorm1(&f);
        let result = if real_input && (real_function || negligible()) {
            Num::R(f.re())
        } else {
            Num::C(f)
        };
        let mut outputs = vec![self.lx3_num_value(result, a.single)?];
        if count >= 2 {
            outputs.push(Value::Matrix(Matrix::scalar(flag)));
        }
        if count >= 3 {
            let ord: Vec<f64> = ord.iter().map(|&l| l as f64).collect();
            outputs.push(self.lx3_funm_output(&terms, &ind, &ord, t)?);
        }
        Ok(outputs)
    }

    /// Peak live storage, in eight-byte budget elements, charged before the
    /// Schur factors or any Taylor workspace exist. A complex n x n matrix
    /// takes 2 n^2; the worst step holds U, T, F(T) and the atomic block's
    /// T, shifted T, F, power, term and product (nine, 18 n^2 elements, when
    /// one block spans the matrix), so 24 n^2 is charged.
    fn lx3_funm_budget(&self, n: usize) -> Result<()> {
        self.preflight_elements(n.saturating_mul(n).saturating_mul(24))
    }

    /// The output struct: terms per block, the blocks' index sets ind, the
    /// ordschur labels ord and the reordered Schur factor T.
    fn lx3_funm_output(
        &self,
        terms: &[f64],
        ind: &[Vec<usize>],
        ord: &[f64],
        t: CMat,
    ) -> Result<Value> {
        let row = |v: Vec<f64>| -> Result<Arc<Array>> {
            Ok(Arc::new(Array::Numeric(Matrix::from_column_vec(
                1,
                v.len(),
                v,
            )?)))
        };
        let blocks = ind
            .iter()
            .map(|b| row(b.iter().map(|&i| (i + 1) as f64).collect()))
            .collect::<Result<Vec<_>>>()?;
        let ind = Array::Cell(CellArray::from_shared(
            1,
            blocks.len(),
            blocks,
            &self.cancel,
        )?);
        let t = if t.data.iter().all(|z| z.im == 0.) {
            Array::Numeric(Matrix::from_column_vec(t.rows, t.cols, t.re().data)?)
        } else {
            Array::Complex(ComplexMatrix::from_column_vec(
                t.rows,
                t.cols,
                t.data,
                &self.cancel,
            )?)
        };
        let names = ["terms", "ind", "ord", "T"].map(String::from).to_vec();
        let slots = vec![
            row(terms.to_vec())?,
            Arc::new(ind),
            row(ord.to_vec())?,
            Arc::new(t),
        ];
        Ok(Value::Container(Array::Struct(StructArray::from_slots(
            1,
            1,
            names,
            slots,
            &self.cancel,
        )?)))
    }

    /// f on the atomic block T(j, j) by its Taylor series about the mean
    /// eigenvalue sigma (Higham 2008, §9.1; Davies and Higham 2003, §2):
    /// with M = T - sigma*I, F_s = F_{s-1} + f^(s)(sigma) M^s/s!. Once a term
    /// is below tol*||F_s||, the truncation bound
    ///   mu * max_{0<=r<m} (omega_{s+r+1}/r!) * ||M^(s+1)||/(s+1)!,
    /// with omega_k the largest |f^(k)| over the block's eigenvalues and
    /// mu = ||(I - |N|)^(-1)||, N the strictly upper part of T, must be too
    /// (and be a finite number, or zero because M^(s+1) = 0).
    /// Returns the block and the number of terms, None when MaxTerms terms
    /// did not converge (the sum stops early if a term overflows).
    fn lx3_funm_atom(
        &mut self,
        t_full: &CMat,
        j: &[usize],
        fun: &Fun,
        tol: f64,
        maxterms: usize,
        out: &mut Execution,
    ) -> Result<(CMat, Option<usize>)> {
        let m = j.len();
        let mut t = CMat::zeros(m, m);
        for (q, &col) in j.iter().enumerate() {
            for (p, &row) in j.iter().enumerate() {
                t.set(p, q, t_full.at(row, col));
            }
        }
        let diag: Vec<C> = (0..m).map(|i| t.at(i, i)).collect();
        if m == 1 {
            let f = self.lx3_funm_eval(fun, &diag, 0, out)?;
            return Ok((
                CMat {
                    rows: 1,
                    cols: 1,
                    data: f,
                },
                Some(0),
            ));
        }
        let sigma = cdense::cdivr(diag.iter().fold(ZERO, |s, &z| s + z), m as f64);
        let mut shifted = t.clone();
        for i in 0..m {
            *shifted.get_mut(i, i) -= sigma;
        }
        // (I - |N|) y = e by back substitution; mu = ||y||_inf.
        let mut y = vec![0.; m];
        for i in (0..m).rev() {
            y[i] = 1. + (i + 1..m).map(|k| cabs(t.at(i, k)) * y[k]).sum::<f64>();
        }
        let mu = y.iter().copied().fold(0., f64::max);
        let at_sigma = |values: Vec<C>| values.first().copied().unwrap_or(ZERO);
        let f0 = at_sigma(self.lx3_funm_eval(fun, &[sigma], 0, out)?);
        let mut f = CMat::zeros(m, m);
        for i in 0..m {
            f.set(i, i, f0);
        }
        // p = M^s/s! for the current s.
        let mut p = shifted.clone();
        for s in 1..=maxterms {
            self.check_cancel()?;
            let d = at_sigma(self.lx3_funm_eval(fun, &[sigma], s, out)?);
            let mut step = p.clone();
            for z in &mut step.data {
                *z = d * *z;
            }
            if !all_finite(&step) {
                return Ok((f, None));
            }
            for (x, &dz) in f.data.iter_mut().zip(&step.data) {
                *x += dz;
            }
            if !all_finite(&f) {
                return Ok((f, None));
            }
            p = {
                let cancel = self.lx3_cancelled();
                upper_mul(&p, &shifted, &cancel)
            };
            self.check_cancel()?;
            for z in &mut p.data {
                *z = cdense::cdivr(*z, (s + 1) as f64);
            }
            // Norms in units that keep their row sums finite; the unit is 1
            // unless entries are near the overflow threshold.
            let unit = norm_unit(&f, &step);
            let step_norm = norm_inf(&step, unit);
            let f_norm = norm_inf(&f, unit);
            if f_norm.is_finite() && step_norm <= tol * f_norm {
                let mut delta = 0f64;
                for r in 0..m {
                    let w =
                        max_abs(&self.lx3_funm_eval(fun, &diag, s + r + 1, out)?) / factorial(r);
                    delta = if w.is_nan() || delta.is_nan() {
                        f64::NAN
                    } else {
                        delta.max(w)
                    };
                }
                // A nilpotent remainder (M^(s+1) = 0) is exactly zero; any
                // other bound must be a finite number to be accepted.
                let bound = if p.data.iter().all(|&z| z == ZERO) {
                    0.
                } else {
                    mu * delta * norm_inf(&p, unit)
                };
                if bound.is_finite() && bound <= tol * f_norm {
                    return Ok((f, Some(s)));
                }
            }
        }
        Ok((f, None))
    }
}

fn cnorm1(x: &CMat) -> f64 {
    (0..x.cols)
        .map(|j| (0..x.rows).map(|i| cabs(x.at(i, j))).sum::<f64>())
        .fold(0., f64::max)
}

#[cfg(test)]
mod tests {
    use super::{CMat, blocking, c, cluster_blocks, swapping, sylvester, upper_mul};
    use crate::Engine;
    use std::cell::Cell;

    fn scalar(e: &Engine, name: &str) -> f64 {
        e.array(name).unwrap().as_numeric().unwrap().data()[0]
    }

    #[test]
    fn blocking_merges_linked_sets_and_swapping_orders_by_mean_position() {
        let diag = [1., 5., 1.05, 9., 5.02, 1.1, 9.2];
        let mut t = CMat::zeros(7, 7);
        for (i, &d) in diag.iter().enumerate() {
            t.set(i, i, c(d, 0.));
        }
        // 1, 1.05 and 1.1 chain; 5 and 5.02 pair; 9 and 9.2 stay apart.
        assert_eq!(blocking(&t, 0.06), vec![1, 2, 1, 3, 2, 1, 4]);
        // Mean positions: set 1 (0+2+5)/3, set 2 (1+4)/2, set 3 3, set 4 6.
        let (ord, ind) = swapping(&blocking(&t, 0.06));
        assert_eq!(ord, vec![4, 3, 4, 2, 3, 4, 1]);
        assert_eq!(ind, vec![vec![0, 1, 2], vec![3, 4], vec![5], vec![6]]);
        // Set 1 {0, 5} has mean 2.5, after set 2 {1, 2} at 1.5.
        assert_eq!(swapping(&[1, 2, 2, 3, 4, 1]).0, vec![3, 4, 4, 2, 1, 3]);
        assert_eq!(cluster_blocks(&[1, 2, 1]), vec![vec![0], vec![1, 2]]);
    }

    #[test]
    fn clustered_nonnormal_matrix_matches_expm_after_reordering() {
        let mut e = Engine::default();
        // Interleaved clusters near 1 and 3 force swaps, two atomic blocks
        // and Sylvester solves between them.
        e.execute(
            "T=diag([1 3 1.05 3.02 1.02])+triu(reshape(sin(1:25),5,5),1);\
             [Q,~]=qr(reshape(cos(1:25),5,5));A=Q*T*Q';\
             [F,flag,o]=funm(A,@exp);r=norm(F-expm(A),1)/norm(expm(A),1);\
             nb=numel(o.ind);s1=numel(o.ind{1});s2=numel(o.ind{2});\
             t1=o.terms(1);t2=o.terms(2);d=norm(diag(o.T)-sort(diag(o.T)),1);\
             S=funm(A,@sin);C=funm(A,@cos);p=norm(S*S+C*C-eye(5),1);\
             L=funm(A,@log);q=norm(expm(L)-A,1)/norm(A,1);\
             H=funm(A,@(x,k) exp(x));h=norm(H-F,1)/norm(F,1);",
        )
        .unwrap();
        assert!(scalar(&e, "r") < 1e-13, "{}", scalar(&e, "r"));
        assert_eq!(scalar(&e, "flag"), 0.);
        assert_eq!(scalar(&e, "nb"), 2.);
        assert_eq!(scalar(&e, "s1") + scalar(&e, "s2"), 5.);
        assert!(scalar(&e, "t1") > 1. && scalar(&e, "t2") > 1.);
        assert!(scalar(&e, "p") < 1e-12, "{}", scalar(&e, "p"));
        assert!(scalar(&e, "q") < 1e-12, "{}", scalar(&e, "q"));
        assert!(scalar(&e, "h") < 1e-15, "{}", scalar(&e, "h"));
    }

    #[test]
    fn real_input_keeps_a_genuinely_complex_logarithm() {
        let mut e = Engine::default();
        e.execute("L=funm([-1 1;0 -2],@log);r=double(isreal(L));x=imag(L(1,1));")
            .unwrap();
        assert_eq!(scalar(&e, "r"), 0.);
        assert!((scalar(&e, "x") - std::f64::consts::PI).abs() < 1e-15);
        e.execute("F=funm([0 1;-1 0],@(x,k) exp(x));r=double(isreal(F));")
            .unwrap();
        assert_eq!(scalar(&e, "r"), 1.);
    }

    #[test]
    fn options_and_ord_labels_set_the_block_structure() {
        let mut e = Engine::default();
        e.execute(
            "A=[1 2 3;0 2 4;0 0 3];[F,flag,o]=funm(A,@exp,struct('Ord',[1 2 1]));\
             r=norm(F-expm(A),1)/norm(expm(A),1);d=o.T(1,1);n=numel(o.ind);\
             [F,flag,o]=funm(A,@exp,struct('TolBlk',10,'MaxTerms',3));m=o.terms;",
        )
        .unwrap();
        assert!(scalar(&e, "r") < 1e-14, "{}", scalar(&e, "r"));
        assert_eq!(scalar(&e, "d"), 2.);
        assert_eq!(scalar(&e, "n"), 2.);
        assert_eq!(scalar(&e, "flag"), 1.);
        assert_eq!(scalar(&e, "m"), 3.);
    }

    #[test]
    fn taylor_sum_near_overflow_is_neither_stopped_early_nor_wrong() {
        let mut e = Engine::default();
        // exp(709) = 8.2e307, and the row sums of F reach 2.7e308, past the
        // largest double, while every entry of F is finite.
        e.execute(
            "[F,flag,o]=funm([709 1.5 0;0 709 1.5;0 0 709],@exp);x=exp(709);\
             r=[F(1,1)/x F(1,2)/x F(1,3)/x F(2,3)/x];f=double(all(isfinite(F(:))));t=o.terms;",
        )
        .unwrap();
        assert_eq!(scalar(&e, "flag"), 0.);
        assert_eq!(scalar(&e, "f"), 1.);
        let r = e.array("r").unwrap().as_numeric().unwrap().data().to_vec();
        for (got, want) in r.iter().zip([1., 1.5, 1.125, 1.5]) {
            assert!((got - want).abs() < 1e-14 * want, "{r:?}");
        }
        assert_eq!(scalar(&e, "t"), 3.);
        // An entry that really overflows is reported, not accepted.
        e.execute("[F,flag]=funm([710 1;0 710],@exp);").unwrap();
        assert_eq!(scalar(&e, "flag"), 1.);
    }

    #[test]
    fn storage_is_preflighted_before_the_schur_form_and_taylor_workspaces() {
        let mut e = Engine::default();
        e.execute("A=zeros(1500);").unwrap();
        let error = e.execute("F=funm(A,@exp);").unwrap_err();
        assert!(error.contains("budget"), "{error}");
        e.execute("F=funm(zeros(40),@exp);d=norm(F-eye(40),1);")
            .unwrap();
        assert_eq!(scalar(&e, "d"), 0.);
    }

    fn upper(rows: usize, seed: f64) -> CMat {
        let mut m = CMat::zeros(rows, rows);
        for j in 0..rows {
            for i in 0..=j {
                m.set(i, j, c(1. + (i + 2 * j) as f64 * seed, (i * j) as f64 - 1.));
            }
        }
        m
    }

    #[test]
    fn upper_mul_matches_the_full_product_and_checks_cancel_once_per_column() {
        let (a, b) = (upper(5, 1.), upper(5, 2.));
        let calls = Cell::new(0);
        let full = upper_mul(&a, &b, &|| {
            calls.set(calls.get() + 1);
            false
        });
        assert_eq!(calls.get(), 5);
        assert_eq!(full.data, a.mul(&b, &|| false).data);
        calls.set(0);
        let part = upper_mul(&a, &b, &|| {
            calls.set(calls.get() + 1);
            calls.get() > 2
        });
        assert_eq!(calls.get(), 3);
        assert_eq!(part.data[..10], full.data[..10]);
        assert!(part.data[10..].iter().all(|z| z.re == 0. && z.im == 0.));
    }

    #[test]
    fn sylvester_solves_and_checks_cancel_once_per_column() {
        let (a, mut b) = (upper(4, 1.), upper(3, 2.));
        for i in 0..3 {
            b.set(i, i, c(-5. - i as f64, 0.5));
        }
        let mut rhs = CMat::zeros(4, 3);
        for j in 0..3 {
            for i in 0..4 {
                rhs.set(i, j, c((i + j + 1) as f64, i as f64 - j as f64));
            }
        }
        let calls = Cell::new(0);
        let x = sylvester(&a, &b, &rhs, &|| {
            calls.set(calls.get() + 1);
            false
        });
        assert_eq!(calls.get(), 3);
        let axb = a.mul(&x, &|| false);
        let xb = x.mul(&b, &|| false);
        for ((r, l), want) in axb.data.iter().zip(&xb.data).zip(&rhs.data) {
            assert!((*r - *l - *want).norm() < 1e-12, "{r} {l} {want}");
        }
        calls.set(0);
        let part = sylvester(&a, &b, &rhs, &|| {
            calls.set(calls.get() + 1);
            calls.get() > 1
        });
        assert_eq!(calls.get(), 2);
        assert_eq!(part.data[..4], x.data[..4]);
        assert!(part.data[4..].iter().all(|z| z.re == 0. && z.im == 0.));
    }
}
