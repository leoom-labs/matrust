//! cholupdate (LINPACK dchud/dchdd and their complex forms), qrupdate
//! (Givens rotations, Golub and Van Loan 12.5.1), qrinsert and qrdelete
//! (Givens sweeps after inserting or deleting a column or row: Golub and
//! Van Loan, Matrix Computations, 4th ed., 2013, §6.5; an economy-size Q
//! gains its new direction by Gram-Schmidt with the reorthogonalization of
//! Daniel, Gragg, Kaufman and Stewart, Math. Comp. 30, 1976).
// LAPACK ports keep the reference routines' index loops.
#![allow(clippy::needless_range_loop)]
use super::cdense::{CMat, c, cabs, cdivr, cmulr};
use super::dense::Mat;
use super::{Num, text_arg};
use crate::engine::{Engine, Value};
use crate::{Complex64 as C, Result};
use rayon::prelude::*;
use std::ops::{Add, Mul, Sub};

const ZERO: C = C { re: 0., im: 0. };

/// Rotation work (matrix rows times rotations) below which the Q and R
/// sweeps stay on one thread.
const PAR_WORK: usize = 1 << 18;
/// Fewest rows (or columns) one worker takes in a parallel sweep.
const MIN_BLOCK: usize = 64;

/// A 2x2 plane rotation G = [g11 g12; g21 g22].
#[derive(Clone, Copy)]
struct Rot {
    g: [C; 4],
}

impl Rot {
    fn real(cs: f64, sn: C) -> Self {
        // [c s; -conj(s) c]
        Rot {
            g: [c(cs, 0.), sn, -sn.conj(), c(cs, 0.)],
        }
    }
    /// Rows (x, y) := G (x, y).
    fn apply(self, x: C, y: C) -> (C, C) {
        let g = self.g;
        (g[0] * x + g[1] * y, g[2] * x + g[3] * y)
    }
    /// Columns (x, y) := (x, y) G'.
    fn apply_right(self, x: C, y: C) -> (C, C) {
        let g = self.g;
        (
            x * g[0].conj() + y * g[1].conj(),
            x * g[2].conj() + y * g[3].conj(),
        )
    }
}

/// Plane rotation G with G [f; g] = [r; 0] and r = sqrt(|f|^2 + |g|^2) >= 0
/// (Golub and Van Loan, Matrix Computations, 4th ed., 2013, §5.1.8, with r
/// kept nonnegative). G = [conj(f) conj(g); -g f] / r is unitary with
/// determinant 1; for real f and g it is [c s; -s c].
fn planerot(f: C, g: C) -> (Rot, C) {
    let r = libm::hypot(libm::hypot(f.re, f.im), libm::hypot(g.re, g.im));
    if r == 0. {
        let one = c(1., 0.);
        return (
            Rot {
                g: [one, ZERO, ZERO, one],
            },
            ZERO,
        );
    }
    let (a, b) = (cdivr(f, r), cdivr(g, r));
    (
        Rot {
            g: [a.conj(), b.conj(), -b, a],
        },
        c(r, 0.),
    )
}

/// Rotation with LAPACK's dlartg sign convention (r keeps the sign of f
/// when |f| > |g|); complex values use zlartg.
fn lartg(f: C, g: C) -> (Rot, C) {
    if f.im == 0. && g.im == 0. {
        let (cs, sn, r) = super::dense::lartg(f.re, g.re);
        return (Rot::real(cs, c(sn, 0.)), c(r, 0.));
    }
    let (cs, sn, r) = super::cgen::zlartg(f, g);
    (Rot::real(cs, sn), r)
}

fn to_num(m: CMat, real: bool) -> Num {
    if real { Num::R(m.re()) } else { Num::C(m) }
}

impl Engine {
    pub(super) fn lx3_cholupdate(&mut self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        if args.len() < 2 {
            return Err("Not enough input arguments.".into());
        }
        if args.len() > 3 {
            return Err("Too many input arguments.".into());
        }
        let r = self.lx3_operand("cholupdate", &args[0])?.num;
        let x = self.lx3_operand("cholupdate", &args[1])?.num;
        let n = r.rows();
        if r.cols() != n {
            return Err("First argument must be a square matrix.".into());
        }
        if x.cols() != 1 || x.rows() != n {
            return Err(format!(
                "Second argument must be a column vector of length {n}."
            ));
        }
        let downdate = match args.get(2) {
            None => false,
            Some(v) => match text_arg(v).as_deref() {
                Some("+") => false,
                Some("-") => true,
                _ => return Err("Third argument must be '+' or '-'.".into()),
            },
        };
        let real = matches!((&r, &x), (Num::R(_), Num::R(_)));
        let mut rm = r.complex();
        let xv = x.complex().data;
        let mut status = 0.;
        if !downdate {
            // zchud: apply the previous rotations to each column, then make a new one.
            let mut rots: Vec<(f64, C)> = Vec::with_capacity(n);
            for j in 0..n {
                // Appending x' gives R'*R + x*x'.
                let mut xj = xv[j].conj();
                for (i, &(cs, sn)) in rots.iter().enumerate() {
                    let t = cmulr(rm.at(i, j), cs) + sn * xj;
                    xj = cmulr(xj, cs) - sn.conj() * rm.at(i, j);
                    rm.set(i, j, t);
                }
                let (g, r) = rotg(rm.at(j, j), xj);
                rm.set(j, j, r);
                rots.push(g);
            }
        } else {
            // zchdd: solve R' s = x, then the rotations that remove s.
            let mut s = vec![ZERO; n];
            for j in 0..n {
                let mut t = xv[j];
                for k in 0..j {
                    t -= rm.at(k, j).conj() * s[k];
                }
                s[j] = super::cdense::cdiv(t, rm.at(j, j).conj());
            }
            let norm = super::cdense::znrm2(s.iter());
            if norm >= 1. {
                status = 1.;
            } else {
                let mut alpha = (1. - norm * norm).sqrt();
                let mut cs = vec![0f64; n];
                for i in (0..n).rev() {
                    let scale = alpha + cabs(s[i]);
                    let a = alpha / scale;
                    let b = cdivr(s[i], scale);
                    let nrm = (a * a + b.re * b.re + b.im * b.im).sqrt();
                    cs[i] = a / nrm;
                    s[i] = cdivr(b.conj(), nrm);
                    alpha = scale * nrm;
                }
                for j in 0..n {
                    let mut xx = ZERO;
                    for i in (0..=j).rev() {
                        let t = cmulr(xx, cs[i]) + s[i] * rm.at(i, j);
                        rm.set(i, j, cmulr(rm.at(i, j), cs[i]) - s[i].conj() * xx);
                        xx = t;
                    }
                }
            }
        }
        if status != 0. && count < 2 {
            return Err("Downdated matrix must be positive definite.".into());
        }
        let r1 = if status != 0. { r } else { to_num(rm, real) };
        let mut out = vec![self.lx3_num_value(r1, false)?];
        if count >= 2 {
            out.push(Value::Matrix(crate::Matrix::scalar(status)));
        }
        Ok(out)
    }

    pub(super) fn lx3_qrupdate(&mut self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        if args.len() < 4 {
            return Err("Not enough input arguments.".into());
        }
        if args.len() > 4 {
            return Err("Too many input arguments.".into());
        }
        let q = self.lx3_operand("qrupdate", &args[0])?.num;
        let r = self.lx3_operand("qrupdate", &args[1])?.num;
        let u = self.lx3_operand("qrupdate", &args[2])?.num;
        let v = self.lx3_operand("qrupdate", &args[3])?.num;
        let (m, n) = (r.rows(), r.cols());
        if q.cols() != m {
            return Err("Matrix dimensions must agree.".into());
        }
        if q.rows() != q.cols() {
            return Err("Matrix dimensions must agree.".into());
        }
        if u.cols() != 1 || u.rows() != q.rows() {
            return Err(format!(
                "Third argument must be a column vector of length {}.",
                q.rows()
            ));
        }
        if v.cols() != 1 || v.rows() != n {
            return Err(format!(
                "Fourth argument must be a column vector of length {n}."
            ));
        }
        let real = [&q, &r, &u, &v].iter().all(|x| matches!(x, Num::R(_)));
        let (mut qm, mut rm) = (q.complex(), r.complex());
        let (u, v) = (u.complex().data, v.complex().data);
        // w = Q' u
        let mut w: Vec<C> = (0..m)
            .map(|j| (0..m).fold(ZERO, |s, i| s + qm.at(i, j).conj() * u[i]))
            .collect();
        for k in (0..m.saturating_sub(1)).rev() {
            let (g, r0) = lartg(w[k], w[k + 1]);
            w[k] = r0;
            w[k + 1] = ZERO;
            for j in 0..n {
                let (x, y) = g.apply(rm.at(k, j), rm.at(k + 1, j));
                rm.set(k, j, x);
                rm.set(k + 1, j, y);
            }
            for i in 0..m {
                let (x, y) = g.apply_right(qm.at(i, k), qm.at(i, k + 1));
                qm.set(i, k, x);
                qm.set(i, k + 1, y);
            }
        }
        if m > 0 {
            for j in 0..n {
                let x = rm.at(0, j) + w[0] * v[j].conj();
                rm.set(0, j, x);
            }
        }
        for k in 0..n.min(m.saturating_sub(1)) {
            let (g, r0) = lartg(rm.at(k, k), rm.at(k + 1, k));
            rm.set(k, k, r0);
            rm.set(k + 1, k, ZERO);
            for j in k + 1..n {
                let (x, y) = g.apply(rm.at(k, j), rm.at(k + 1, j));
                rm.set(k, j, x);
                rm.set(k + 1, j, y);
            }
            for i in 0..m {
                let (x, y) = g.apply_right(qm.at(i, k), qm.at(i, k + 1));
                qm.set(i, k, x);
                qm.set(i, k + 1, y);
            }
        }
        let mut out = vec![self.lx3_num_value(to_num(qm, real), false)?];
        if count >= 2 {
            out.push(self.lx3_num_value(to_num(rm, real), false)?);
        }
        Ok(out)
    }

    pub(super) fn lx3_qrinsert(&mut self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        if args.len() < 4 {
            return Err("Too few inputs.".into());
        }
        if args.len() > 5 {
            return Err("Too many input arguments.".into());
        }
        let q = self.lx3_operand("qrinsert", &args[0])?.num;
        let r = self.lx3_operand("qrinsert", &args[1])?.num;
        let x = self.lx3_operand("qrinsert", &args[3])?.num;
        let row = match args.get(4) {
            None => false,
            Some(v) => match text_arg(v).as_deref() {
                Some("col") => false,
                Some("row") => true,
                _ => return Err("Fifth input must be the string 'row' or 'col'".into()),
            },
        };
        let j = self.lx3_index(&args[2])?;
        let (m, p, n) = (q.rows(), q.cols(), r.cols());
        if r.rows() != p || p > m {
            return Err("Matrix dimensions must agree.".into());
        }
        if j == 0 {
            return Err(SUBSCRIPT.into());
        }
        let last = if row { m } else { n };
        if j > last + 1 {
            return Err("Insertion index exceeds matrix dimensions.".into());
        }
        if row && (x.rows() != 1 || x.cols() != n) {
            return Err("Inserted row has incorrect dimensions.".into());
        }
        if !row && (x.rows() != m || x.cols() != 1) {
            return Err("Inserted column has incorrect dimensions.".into());
        }
        // At most one more row and column in each factor.
        let grown = if row { (m + 1) * (p + 1) } else { m * (p + 1) };
        self.preflight_elements(grown.saturating_add((p + 1) * (n + 1)))?;
        let real = [&q, &r, &x].iter().all(|v| matches!(v, Num::R(_)));
        let (q1, r1) = if real {
            insert::<f64>(q, r, x, j - 1, row)
        } else {
            insert::<C>(q, r, x, j - 1, row)
        };
        let mut out = vec![self.lx3_num_value(q1, false)?];
        if count >= 2 {
            out.push(self.lx3_num_value(r1, false)?);
        }
        Ok(out)
    }

    pub(super) fn lx3_qrdelete(&mut self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        if args.len() < 3 {
            return Err("Too few inputs.".into());
        }
        if args.len() > 4 {
            return Err("Too many input arguments.".into());
        }
        let q = self.lx3_operand("qrdelete", &args[0])?.num;
        let r = self.lx3_operand("qrdelete", &args[1])?.num;
        let row = match args.get(3) {
            None => false,
            Some(v) => match text_arg(v).as_deref() {
                Some("col") => false,
                Some("row") => true,
                _ => return Err("Fourth input must be the string 'row' or 'col'.".into()),
            },
        };
        let j = self.lx3_index(&args[2])?;
        let (m, p, n) = (q.rows(), q.cols(), r.cols());
        if r.rows() != p || p > m {
            return Err("Matrix dimensions must agree.".into());
        }
        // A factorization without rows has no row or column to delete from.
        if j == 0 || p == 0 {
            return Err(SUBSCRIPT.into());
        }
        let last = if row { m } else { n };
        if j > last {
            return Err("Deletion index exceeds matrix dimensions.".into());
        }
        if row && p != m {
            return Err("Deleting a row needs the full factorization (square Q).".into());
        }
        let real = matches!((&q, &r), (Num::R(_), Num::R(_)));
        let (q1, r1) = if real {
            delete::<f64>(q, r, j - 1, row)
        } else {
            delete::<C>(q, r, j - 1, row)
        };
        let mut out = vec![self.lx3_num_value(q1, false)?];
        if count >= 2 {
            out.push(self.lx3_num_value(r1, false)?);
        }
        Ok(out)
    }

    /// A positive integer scalar index (0 or negative are passed through as 0).
    fn lx3_index(&self, v: &Value) -> Result<usize> {
        match v {
            Value::Matrix(m) if m.len() == 1 && m.data()[0].fract() == 0. => {
                Ok(m.data()[0].max(0.) as usize)
            }
            _ => Err("The index must be a positive integer scalar.".into()),
        }
    }
}

/// zrotg (LINPACK/BLAS): rotation (c, s) zeroing b against a, and r.
fn rotg(a: C, b: C) -> ((f64, C), C) {
    if a.im == 0. && b.im == 0. {
        // drotg
        let (sa, sb) = (a.re, b.re);
        let roe = if sa.abs() > sb.abs() { sa } else { sb };
        let scale = sa.abs() + sb.abs();
        if scale == 0. {
            return ((1., ZERO), c(0., 0.));
        }
        let r = scale * ((sa / scale) * (sa / scale) + (sb / scale) * (sb / scale)).sqrt();
        let r = if roe < 0. { -r } else { r };
        return ((sa / r, c(sb / r, 0.)), c(r, 0.));
    }
    if cabs(a) == 0. {
        return ((0., c(1., 0.)), b);
    }
    let scale = cabs(a) + cabs(b);
    let norm = scale * ((cabs(a) / scale).powi(2) + (cabs(b) / scale).powi(2)).sqrt();
    let alpha = cdivr(a, cabs(a));
    (
        (cabs(a) / norm, cdivr(alpha * b.conj(), norm)),
        cmulr(alpha, norm),
    )
}

const SUBSCRIPT: &str = "Subscript indices must either be real positive integers or logicals.";

/// Column-major matrix the insertion and deletion sweeps work on.
struct Dense<T> {
    rows: usize,
    cols: usize,
    data: Vec<T>,
}

impl<T: Elem> Dense<T> {
    fn col(&self, j: usize) -> &[T] {
        &self.data[j * self.rows..(j + 1) * self.rows]
    }
    /// The matrix without row `drop_row` and column `drop_col` (if given).
    fn without(&self, drop_row: Option<usize>, drop_col: Option<usize>) -> Self {
        let rows = self.rows - usize::from(drop_row.is_some());
        let cols = self.cols - usize::from(drop_col.is_some());
        let mut data = Vec::with_capacity(rows * cols);
        for j in (0..self.cols).filter(|&j| Some(j) != drop_col) {
            let col = self.col(j);
            match drop_row {
                Some(i) => data.extend(col[..i].iter().chain(&col[i + 1..])),
                None => data.extend_from_slice(col),
            }
        }
        Dense { rows, cols, data }
    }
}

/// Scalars of the sweeps: real factors stay real (half the storage and a
/// quarter of the arithmetic of complex ones).
trait Elem: Copy + Send + Sync + Add<Output = Self> + Sub<Output = Self> + Mul<Output = Self> {
    const ZERO: Self;
    const ONE: Self;
    fn real(x: f64) -> Self;
    fn conj(self) -> Self;
    fn abs(self) -> f64;
    fn div(self, d: f64) -> Self;
    /// planerot's G as [g11, g12, g21, g22], and r.
    fn rot(f: Self, g: Self) -> ([Self; 4], Self);
    fn take(n: Num) -> Dense<Self>;
    fn give(d: Dense<Self>) -> Num;
}

impl Elem for f64 {
    const ZERO: f64 = 0.;
    const ONE: f64 = 1.;
    fn real(x: f64) -> f64 {
        x
    }
    fn conj(self) -> f64 {
        self
    }
    fn abs(self) -> f64 {
        f64::abs(self)
    }
    fn div(self, d: f64) -> f64 {
        self / d
    }
    fn rot(f: f64, g: f64) -> ([f64; 4], f64) {
        let (rot, r) = planerot(c(f, 0.), c(g, 0.));
        (rot.g.map(|z| z.re), r.re)
    }
    fn take(n: Num) -> Dense<f64> {
        let m = match n {
            Num::R(m) => m,
            Num::C(m) => m.re(),
        };
        Dense {
            rows: m.rows,
            cols: m.cols,
            data: m.data,
        }
    }
    fn give(d: Dense<f64>) -> Num {
        Num::R(Mat::from_vec(d.rows, d.cols, d.data))
    }
}

impl Elem for C {
    const ZERO: C = ZERO;
    const ONE: C = C { re: 1., im: 0. };
    fn real(x: f64) -> C {
        c(x, 0.)
    }
    fn conj(self) -> C {
        C::conj(&self)
    }
    fn abs(self) -> f64 {
        libm::hypot(self.re, self.im)
    }
    fn div(self, d: f64) -> C {
        cdivr(self, d)
    }
    fn rot(f: C, g: C) -> ([C; 4], C) {
        let (rot, r) = planerot(f, g);
        (rot.g, r)
    }
    fn take(n: Num) -> Dense<C> {
        let m = n.complex();
        Dense {
            rows: m.rows,
            cols: m.cols,
            data: m.data,
        }
    }
    fn give(d: Dense<C>) -> Num {
        Num::C(CMat {
            rows: d.rows,
            cols: d.cols,
            data: d.data,
        })
    }
}

/// A rotation of rows (k, k+1) of R, or of columns (k, k+1) of Q.
type Plane<T> = (usize, [T; 4]);

/// (x, y) := G (x, y) on rows k and k+1 of one column.
#[inline]
fn rotate_pair<T: Elem>(col: &mut [T], k: usize, g: &[T; 4]) {
    let (x, y) = (col[k], col[k + 1]);
    col[k] = g[0] * x + g[1] * y;
    col[k + 1] = g[2] * x + g[3] * y;
}

/// Q(:, [k k+1]) := Q(:, [k k+1]) G' for each rotation in order. Rows are
/// independent, so a large sweep runs in row blocks on the rayon pool.
fn rotate_q<T: Elem>(q: &mut Dense<T>, rots: &[Plane<T>]) {
    let rows = q.rows;
    if rows == 0 || rots.is_empty() {
        return;
    }
    // [x y] G' = [x y] [conj(g11) conj(g21); conj(g12) conj(g22)]
    let rots: Vec<Plane<T>> = rots.iter().map(|&(k, g)| (k, g.map(T::conj))).collect();
    let sweep = |cols: &mut [&mut [T]]| {
        for &(k, h) in &rots {
            let (left, right) = cols.split_at_mut(k + 1);
            for (x, y) in left[k].iter_mut().zip(right[0].iter_mut()) {
                let (u, v) = (*x, *y);
                *x = u * h[0] + v * h[1];
                *y = u * h[2] + v * h[3];
            }
        }
    };
    let workers = rayon::current_num_threads().min(rows / MIN_BLOCK);
    if workers < 2 || rows.saturating_mul(rots.len()) < PAR_WORK {
        sweep(&mut q.data.chunks_mut(rows).collect::<Vec<_>>());
        return;
    }
    let block = rows.div_ceil(workers);
    let mut parts: Vec<Vec<&mut [T]>> = (0..rows.div_ceil(block))
        .map(|_| Vec::with_capacity(q.cols))
        .collect();
    for col in q.data.chunks_mut(rows) {
        for (part, piece) in parts.iter_mut().zip(col.chunks_mut(block)) {
            part.push(piece);
        }
    }
    parts.into_par_iter().for_each(|mut cols| sweep(&mut cols));
}

/// Applies rotations known in advance, listed with decreasing k, to the
/// columns of R from `first` on: column c takes those with k <= top(c)
/// (the others would rotate two structural zeros). Columns are independent.
fn rotate_r<T: Elem>(
    r: &mut Dense<T>,
    first: usize,
    rots: &[Plane<T>],
    top: impl Fn(usize) -> usize + Sync,
) {
    let rows = r.rows;
    if rows == 0 || rots.is_empty() || first >= r.cols {
        return;
    }
    let apply = |c: usize, col: &mut [T]| {
        let start = rots.partition_point(|&(k, _)| k > top(c));
        for (k, g) in &rots[start..] {
            rotate_pair(col, *k, g);
        }
    };
    let tail = &mut r.data[first * rows..];
    let work = rows.min(rots.len()).saturating_mul(r.cols - first);
    if rayon::current_num_threads() > 1 && work >= PAR_WORK {
        tail.par_chunks_mut(rows)
            .with_min_len(MIN_BLOCK)
            .enumerate()
            .for_each(|(i, col)| apply(first + i, col));
    } else {
        for (i, col) in tail.chunks_mut(rows).enumerate() {
            apply(first + i, col);
        }
    }
}

fn dot<T: Elem>(a: &[T], b: &[T]) -> T {
    a.iter()
        .zip(b)
        .fold(T::ZERO, |s, (&x, &y)| s + x.conj() * y)
}

/// Q' v, one dot product per column of Q.
fn q_adjoint_times<T: Elem>(q: &Dense<T>, v: &[T]) -> Vec<T> {
    if q.rows == 0 {
        return vec![T::ZERO; q.cols];
    }
    if rayon::current_num_threads() > 1 && q.data.len() >= PAR_WORK {
        q.data
            .par_chunks(q.rows)
            .with_min_len(MIN_BLOCK)
            .map(|col| dot(col, v))
            .collect()
    } else {
        q.data.chunks(q.rows).map(|col| dot(col, v)).collect()
    }
}

/// One classical Gram-Schmidt pass: s = Q' v, v := v - Q s; returns s.
fn project_out<T: Elem>(q: &Dense<T>, v: &mut [T]) -> Vec<T> {
    let s = q_adjoint_times(q, v);
    for (j, &sj) in s.iter().enumerate() {
        for (vi, &qi) in v.iter_mut().zip(q.col(j)) {
            *vi = *vi - qi * sj;
        }
    }
    s
}

/// 2-norm, scaled by the largest modulus against overflow and underflow.
fn norm<T: Elem>(v: &[T]) -> f64 {
    let scale = v.iter().fold(0f64, |m, z| m.max(z.abs()));
    if scale == 0. || !scale.is_finite() {
        return scale;
    }
    scale
        * v.iter()
            .map(|z| (z.abs() / scale).powi(2))
            .sum::<f64>()
            .sqrt()
}

/// The component of x orthogonal to the orthonormal columns of Q, its
/// norm, and Q' x (classical Gram-Schmidt with the "twice is enough"
/// reorthogonalization of Daniel, Gragg, Kaufman and Stewart, 1976: repeat
/// the projection while it removes more than 1 - 1/sqrt(2) of the norm).
/// When x lies in the range of Q the norm is 0 and the direction is the
/// unit vector orthogonal to Q built from the coordinate axis Q covers least.
fn new_direction<T: Elem>(q: &Dense<T>, x: &[T]) -> (Vec<T>, f64, Vec<T>) {
    let reorthogonalize = |v: &mut Vec<T>, coef: &mut Vec<T>| {
        let mut before = norm(v);
        for _ in 0..2 {
            let s = project_out(q, v);
            for (a, b) in coef.iter_mut().zip(s) {
                *a = *a + b;
            }
            let after = norm(v);
            if after > before * std::f64::consts::FRAC_1_SQRT_2 {
                return after;
            }
            before = after;
        }
        0.
    };
    let mut v = x.to_vec();
    let mut w = vec![T::ZERO; q.cols];
    let rho = reorthogonalize(&mut v, &mut w);
    if rho > 0. {
        return (v.iter().map(|&z| z.div(rho)).collect(), rho, w);
    }
    // Q has fewer columns than rows, so some row of Q has squared norm at
    // most cols/rows < 1 and its axis keeps a residual of at least 1/sqrt(rows).
    let mut weight = vec![0f64; q.rows];
    for j in 0..q.cols {
        for (s, z) in weight.iter_mut().zip(q.col(j)) {
            *s += z.abs().powi(2);
        }
    }
    let axis = (0..q.rows)
        .min_by(|&a, &b| weight[a].total_cmp(&weight[b]))
        .unwrap_or(0);
    let mut e = vec![T::ZERO; q.rows];
    e[axis] = T::ONE;
    project_out(q, &mut e);
    project_out(q, &mut e);
    let size = norm(&e);
    (e.iter().map(|&z| z.div(size)).collect(), 0., w)
}

/// qrinsert: the factors of A with x inserted before column (or row) jj,
/// 0-based (Golub and Van Loan, 4th ed., §6.5).
fn insert<T: Elem>(q: Num, r: Num, x: Num, jj: usize, row: bool) -> (Num, Num) {
    let (mut q, r, x) = (T::take(q), T::take(r), T::take(x).data);
    let (m, p, n) = (q.rows, q.cols, r.cols);
    let economy = p < m;
    if row {
        // diag(1, Q)' P A1 = [x; R] is upper Hessenberg, where P moves row
        // jj of A1 to the top; rotations on rows (k, k+1) restore R, and
        // Q1 = P' diag(1, Q) G1' G2' ... carries row jj of A1 back.
        let mut qt = Dense {
            rows: m + 1,
            cols: p + 1,
            data: Vec::with_capacity((m + 1) * (p + 1)),
        };
        qt.data
            .extend((0..=m).map(|i| if i == jj { T::ONE } else { T::ZERO }));
        for k in 0..p {
            let col = q.col(k);
            qt.data.extend_from_slice(&col[..jj]);
            qt.data.push(T::ZERO);
            qt.data.extend_from_slice(&col[jj..]);
        }
        let mut h = Dense {
            rows: p + 1,
            cols: n,
            data: Vec::with_capacity((p + 1) * n),
        };
        let mut rots: Vec<Plane<T>> = Vec::with_capacity(p.min(n));
        for k in 0..n {
            let start = h.data.len();
            h.data.push(x[k]);
            h.data.extend_from_slice(r.col(k));
            let col = &mut h.data[start..];
            // Column k is nonzero down to row k + 1: every earlier rotation reaches it.
            for (i, g) in &rots {
                rotate_pair(col, *i, g);
            }
            if k < p {
                let (g, rk) = T::rot(col[k], col[k + 1]);
                col[k] = rk;
                col[k + 1] = T::ZERO;
                rots.push((k, g));
            }
        }
        rotate_q(&mut qt, &rots);
        // An economy factorization stays economy: R1's last row is zero.
        if economy && p >= n {
            return (
                T::give(qt.without(None, Some(p))),
                T::give(h.without(Some(p), None)),
            );
        }
        return (T::give(qt), T::give(h));
    }
    // Q' A1 = [R(:, 1:jj) w R(:, jj+1:n)] with w = Q' x; rotations on rows
    // (i-1, i), bottom up, zero w below row jj. An economy Q first gains the
    // direction of x outside its range, so w gets one more entry.
    let (mut w, p) = if economy {
        let (dir, rho, mut w) = new_direction(&q, &x);
        q.data.extend(dir);
        q.cols += 1;
        w.push(T::real(rho));
        (w, p + 1)
    } else {
        (q_adjoint_times(&q, &x), p)
    };
    let mut rots: Vec<Plane<T>> = Vec::with_capacity(p.saturating_sub(jj + 1));
    for i in (jj + 1..p).rev() {
        let (g, ri) = T::rot(w[i - 1], w[i]);
        w[i - 1] = ri;
        w[i] = T::ZERO;
        rots.push((i - 1, g));
    }
    let mut r1 = Dense {
        rows: p,
        cols: n + 1,
        data: Vec::with_capacity(p * (n + 1)),
    };
    let grow = |data: &mut Vec<T>, col: &[T]| {
        data.extend_from_slice(col);
        data.extend((col.len()..p).map(|_| T::ZERO));
    };
    for k in 0..jj {
        grow(&mut r1.data, r.col(k));
    }
    r1.data.extend_from_slice(&w);
    for k in jj..n {
        grow(&mut r1.data, r.col(k));
    }
    // Column c > jj (old column c - 1) is nonzero down to row c - 1.
    rotate_r(&mut r1, jj + 1, &rots, |c| c - 1);
    rotate_q(&mut q, &rots);
    (T::give(q), T::give(r1))
}

/// qrdelete: the factors of A without column (or row) jj, 0-based (Golub
/// and Van Loan, 4th ed., §6.5).
fn delete<T: Elem>(q: Num, r: Num, jj: usize, row: bool) -> (Num, Num) {
    let (mut q, mut r) = (T::take(q), T::take(r));
    let (m, p, n) = (q.rows, q.cols, r.cols);
    if row {
        // Rotations (k, k+1), bottom up, reduce row jj of Q to [1 0 ... 0]
        // (up to rounding); the same rotations turn R upper Hessenberg, and
        // its rows 2..m are the new R. Q's row jj is [a b] with a [a b] G' =
        // [r 0], so G is planerot of the conjugated pair.
        let mut qrow: Vec<T> = (0..m).map(|k| q.col(k)[jj]).collect();
        let mut rots: Vec<Plane<T>> = Vec::with_capacity(m.saturating_sub(1));
        for k in (0..m.saturating_sub(1)).rev() {
            let (g, rk) = T::rot(qrow[k].conj(), qrow[k + 1].conj());
            qrow[k] = rk;
            qrow[k + 1] = T::ZERO;
            rots.push((k, g));
        }
        // Column c of R is nonzero down to row c.
        rotate_r(&mut r, 0, &rots, |c| c);
        rotate_q(&mut q, &rots);
        return (
            T::give(q.without(Some(jj), Some(0))),
            T::give(r.without(Some(0), None)),
        );
    }
    // Removing column jj leaves R upper Hessenberg from column jj on; each
    // rotation (k, k+1) zeroes the subdiagonal entry of column k.
    let mut h = r.without(None, Some(jj));
    let mut rots: Vec<Plane<T>> = Vec::with_capacity(n.saturating_sub(jj + 1));
    for k in jj..n - 1 {
        let col = &mut h.data[k * p..(k + 1) * p];
        for (i, g) in &rots {
            rotate_pair(col, *i, g);
        }
        if k + 1 < p {
            let (g, rk) = T::rot(col[k], col[k + 1]);
            col[k] = rk;
            col[k + 1] = T::ZERO;
            rots.push((k, g));
        }
    }
    rotate_q(&mut q, &rots);
    // An economy factorization stays economy: the last row of R is zero.
    if p < m && p >= n {
        return (
            T::give(q.without(None, Some(p - 1))),
            T::give(h.without(Some(p - 1), None)),
        );
    }
    (T::give(q), T::give(h))
}

#[cfg(test)]
mod tests {
    use crate::Engine;

    fn scalar(e: &Engine, name: &str) -> f64 {
        e.array(name).unwrap().as_numeric().unwrap().data()[0]
    }

    /// Runs `call` (which sets Q1 and R1) and checks Q1*R1 = A1, Q1'*Q1 = I,
    /// R1 upper triangular and the factor sizes.
    fn check(e: &mut Engine, call: &str, a1: &str, q1: (usize, usize), r1: (usize, usize)) {
        e.execute(&format!(
            "{call} f=norm(Q1*R1-({a1}),'fro'); o=norm(Q1'*Q1-eye(size(Q1,2)),'fro'); \
             t=norm(tril(abs(R1),-1),'fro'); qs=size(Q1); rs=size(R1);"
        ))
        .unwrap();
        assert!(
            scalar(e, "f") < 1e-13,
            "{call}: residual {}",
            scalar(e, "f")
        );
        assert!(
            scalar(e, "o") < 1e-13,
            "{call}: orthogonality {}",
            scalar(e, "o")
        );
        assert_eq!(scalar(e, "t"), 0., "{call}: R1 not triangular");
        for (name, (rows, cols)) in [("qs", q1), ("rs", r1)] {
            let size = e.array(name).unwrap().as_numeric().unwrap().data().to_vec();
            assert_eq!(size, [rows as f64, cols as f64], "{call}: {name}");
        }
    }

    #[test]
    fn economy_factors_stay_economy() {
        let mut e = Engine::default();
        e.execute("A=[1 2 3;3 4 1;5 6 2;7 9 4;1 0 1]; [Q,R]=qr(A,0); x=[1;0;2;1;3];")
            .unwrap();
        for j in 1..=4 {
            let a1 = format!("[A(:,1:{}) x A(:,{j}:3)]", j - 1);
            let call = format!("[Q1,R1]=qrinsert(Q,R,{j},x);");
            check(&mut e, &call, &a1, (5, 4), (4, 4));
        }
        // x in the range of Q: the new direction is any orthonormal one.
        check(
            &mut e,
            "[Q1,R1]=qrinsert(Q,R,2,A(:,1)-2*A(:,3));",
            "[A(:,1) A(:,1)-2*A(:,3) A(:,2:3)]",
            (5, 4),
            (4, 4),
        );
        check(
            &mut e,
            "[Q1,R1]=qrinsert(Q,R,1,zeros(5,1));",
            "[zeros(5,1) A]",
            (5, 4),
            (4, 4),
        );
        for j in 1..=3 {
            let a1 = format!("A(:,[1:{} {}:3])", j - 1, j + 1);
            check(
                &mut e,
                &format!("[Q1,R1]=qrdelete(Q,R,{j});"),
                &a1,
                (5, 2),
                (2, 2),
            );
        }
        for j in 1..=6 {
            let a1 = format!("[A(1:{},:); [2 -1 5]; A({j}:5,:)]", j - 1);
            let call = format!("[Q1,R1]=qrinsert(Q,R,{j},[2 -1 5],'row');");
            check(&mut e, &call, &a1, (6, 3), (3, 3));
        }
        assert!(
            e.execute("[Q1,R1]=qrdelete(Q,R,2,'row');")
                .unwrap_err()
                .contains("square Q")
        );
    }

    #[test]
    fn full_and_wide_factors_in_every_position() {
        let mut e = Engine::default();
        e.execute("A=[4 1 2 0;1 3 0 2;2 5 1 1]; [Q,R]=qr(A); x=[1;-2;3]; y=[1 2 -1 3];")
            .unwrap();
        for j in 1..=5 {
            let a1 = format!("[A(:,1:{}) x A(:,{j}:4)]", j - 1);
            check(
                &mut e,
                &format!("[Q1,R1]=qrinsert(Q,R,{j},x);"),
                &a1,
                (3, 3),
                (3, 5),
            );
        }
        for j in 1..=4 {
            let a1 = format!("A(:,[1:{} {}:4])", j - 1, j + 1);
            check(
                &mut e,
                &format!("[Q1,R1]=qrdelete(Q,R,{j},'col');"),
                &a1,
                (3, 3),
                (3, 3),
            );
        }
        for j in 1..=4 {
            let a1 = format!("[A(1:{},:); y; A({j}:3,:)]", j - 1);
            let call = format!("[Q1,R1]=qrinsert(Q,R,{j},y,'row');");
            check(&mut e, &call, &a1, (4, 4), (4, 4));
        }
        for j in 1..=3 {
            let a1 = format!("A([1:{} {}:3],:)", j - 1, j + 1);
            check(
                &mut e,
                &format!("[Q1,R1]=qrdelete(Q,R,{j},'row');"),
                &a1,
                (2, 2),
                (2, 4),
            );
        }
    }

    #[test]
    fn complex_factors() {
        let mut e = Engine::default();
        // A unitary Q and a complex triangular R (the engine's qr is real only).
        e.execute(
            "Q=[1 1i 0;1i 1 0;0 0 sqrt(2)]/sqrt(2); R=[2 1+1i 3i;0 3 -1;0 0 1-2i]; A=Q*R; \
             x=[1i;2;-1+1i]; y=[1 2i 3];",
        )
        .unwrap();
        for j in 1..=4 {
            let a1 = format!("[A(:,1:{}) x A(:,{j}:3)]", j - 1);
            check(
                &mut e,
                &format!("[Q1,R1]=qrinsert(Q,R,{j},x);"),
                &a1,
                (3, 3),
                (3, 4),
            );
            let a1 = format!("[A(1:{},:); y; A({j}:3,:)]", j - 1);
            let call = format!("[Q1,R1]=qrinsert(Q,R,{j},y,'row');");
            check(&mut e, &call, &a1, (4, 4), (4, 3));
        }
        for j in 1..=3 {
            let a1 = format!("A(:,[1:{} {}:3])", j - 1, j + 1);
            check(
                &mut e,
                &format!("[Q1,R1]=qrdelete(Q,R,{j});"),
                &a1,
                (3, 3),
                (3, 2),
            );
            let a1 = format!("A([1:{} {}:3],:)", j - 1, j + 1);
            check(
                &mut e,
                &format!("[Q1,R1]=qrdelete(Q,R,{j},'row');"),
                &a1,
                (2, 2),
                (2, 3),
            );
        }
        // A complex inserted column makes the factors complex.
        e.execute("[Q1,R1]=qrinsert(eye(2),eye(2),1,[1i;1]);")
            .unwrap();
        assert!(e.array("R1").unwrap().as_complex().is_some());
    }

    #[test]
    fn parallel_sweeps_match_one_worker() {
        let run = |workers: usize| {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap();
            pool.install(|| {
                let mut e = Engine::default();
                e.execute(
                    "A=reshape(sin((1:240000).^2),600,400); [Q,R]=qr(A); x=cos((1:600)'); \
                     [Q1,R1]=qrinsert(Q,R,1,x); [Q2,R2]=qrdelete(Q,R,1,'row'); \
                     [Q3,R3]=qrdelete(Q,R,1); [Q4,R4]=qrinsert(Q,R,1,x(1:400)','row');",
                )
                .unwrap();
                ["Q1", "R1", "Q2", "R2", "Q3", "R3", "Q4", "R4"]
                    .map(|name| e.array(name).unwrap().as_numeric().unwrap().data().to_vec())
            })
        };
        let (one, four) = (run(1), run(4));
        for (a, b) in one.iter().zip(&four) {
            assert!(a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits()));
        }
    }
}

#[cfg(test)]
mod bench {
    use crate::Engine;

    #[test]
    #[ignore = "release benchmark; run explicitly with --ignored --nocapture"]
    fn benchmark_qr_insert_delete() {
        for workers in [1, 4] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap();
            pool.install(|| {
                let mut e = Engine::default();
                e.execute(
                    "A=reshape(sin((1:1000000).^2),1000,1000); [Q,R]=qr(A); x=cos((1:1000)');",
                )
                .unwrap();
                for (label, expression) in [
                    ("qrinsert col 1 n=1000", "[Q1,R1]=qrinsert(Q,R,1,x);"),
                    ("qrinsert row 1 n=1000", "[Q1,R1]=qrinsert(Q,R,1,x','row');"),
                    ("qrdelete col 1 n=1000", "[Q1,R1]=qrdelete(Q,R,1);"),
                    ("qrdelete row 1 n=1000", "[Q1,R1]=qrdelete(Q,R,1,'row');"),
                ] {
                    e.execute(expression).unwrap();
                    let mut times = Vec::new();
                    for _ in 0..5 {
                        let start = std::time::Instant::now();
                        e.execute(expression).unwrap();
                        times.push(start.elapsed().as_secs_f64() * 1000.);
                    }
                    times.sort_by(f64::total_cmp);
                    eprintln!(
                        "workers={workers} {label} median_ms={:.2} all={times:.2?}",
                        times[2]
                    );
                }
            });
        }
    }
}
