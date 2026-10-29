//! Operator function forms (`plus(a, b)` evaluates `a + b` through the same
//! operator code) and small core functions: nonzeros, perms, gradient, del2,
//! unwrap, hankel, cplxpair, orderfields, narginchk and optimset.
use super::{Engine, Execution, Value};
use crate::function_handle::FunctionHandle;
use crate::struct_array::StructArray;
use crate::syntax::{AnonymousFunction, Expr, Token};
use crate::{Array, Complex64, ComplexMatrix, Matrix, Result};
use std::sync::Arc;

/// Largest permutation table perms builds (rows times columns).
const MAX_PERMS_ELEMENTS: usize = 8 * 1024 * 1024;

/// The expression an operator function evaluates, over parameters `a` and `b`.
fn operator_body(name: &str) -> Option<(&'static [&'static str], Expr, &'static str)> {
    let name_expr = |n: &str| Box::new(Expr::Name(n.into()));
    let binary = |op: Token, swap: bool| {
        let (l, r) = if swap { ("b", "a") } else { ("a", "b") };
        Expr::Binary(op, name_expr(l), name_expr(r))
    };
    const AB: &[&str] = &["a", "b"];
    const A: &[&str] = &["a"];
    Some(match name {
        "plus" => (AB, binary(Token::Plus, false), "@(a,b)a+b"),
        "minus" => (AB, binary(Token::Minus, false), "@(a,b)a-b"),
        "times" => (AB, binary(Token::DotStar, false), "@(a,b)a.*b"),
        "rdivide" => (AB, binary(Token::DotSlash, false), "@(a,b)a./b"),
        "ldivide" => (AB, binary(Token::DotSlash, true), "@(a,b)b./a"),
        "mtimes" => (AB, binary(Token::Star, false), "@(a,b)a*b"),
        "power" => (AB, binary(Token::DotPower, false), "@(a,b)a.^b"),
        "mpower" => (AB, binary(Token::Power, false), "@(a,b)a^b"),
        "eq" => (AB, binary(Token::Equal, false), "@(a,b)a==b"),
        "ne" => (AB, binary(Token::NotEqual, false), "@(a,b)a~=b"),
        "lt" => (AB, binary(Token::Less, false), "@(a,b)a<b"),
        "gt" => (AB, binary(Token::Greater, false), "@(a,b)a>b"),
        "le" => (AB, binary(Token::LessEqual, false), "@(a,b)a<=b"),
        "ge" => (AB, binary(Token::GreaterEqual, false), "@(a,b)a>=b"),
        "uminus" => (A, Expr::Unary(true, name_expr("a")), "@(a)-a"),
        "uplus" => (A, Expr::Unary(false, name_expr("a")), "@(a)+a"),
        _ => return None,
    })
}

/// One-dimensional derivative with coordinates `h * (1..n)`, as gradient
/// computes it: central differences inside, one-sided at the ends.
fn derivative(f: &[f64], h: f64, check: &impl Fn() -> Result<()>) -> Result<Vec<f64>> {
    check()?;
    let n = f.len();
    if n < 2 {
        return Ok(vec![0.; n]);
    }
    let x = |k: usize| h * (k + 1) as f64;
    let mut g = vec![0.; n];
    g[0] = (f[1] - f[0]) / (x(1) - x(0));
    g[n - 1] = (f[n - 1] - f[n - 2]) / (x(n - 1) - x(n - 2));
    for k in 1..n - 1 {
        if k % 4096 == 0 {
            check()?;
        }
        g[k] = (f[k + 1] - f[k - 1]) / (x(k + 1) - x(k - 1));
    }
    Ok(g)
}

/// del2's per-dimension term: half the second derivative inside, linear
/// extrapolation at the ends, zero for fewer than three points.
fn second_derivative(f: &[f64], h: f64, check: &impl Fn() -> Result<()>) -> Result<Vec<f64>> {
    check()?;
    let n = f.len();
    let mut g = vec![0.; n];
    if n < 3 {
        return Ok(g);
    }
    let x = |k: usize| h * (k + 1) as f64;
    for k in 1..n - 1 {
        if k % 4096 == 0 {
            check()?;
        }
        let (h0, h1) = (x(k) - x(k - 1), x(k + 1) - x(k));
        g[k] = ((f[k + 1] - f[k]) / h1 - (f[k] - f[k - 1]) / h0) / (h0 + h1);
    }
    if n == 3 {
        g[0] = g[1];
        g[2] = g[1];
        return Ok(g);
    }
    g[0] = g[1] * (x(2) - x(0)) / (x(2) - x(1)) - g[2] * (x(1) - x(0)) / (x(2) - x(1));
    g[n - 1] = -g[n - 3] * (x(n - 1) - x(n - 2)) / (x(n - 2) - x(n - 3))
        + g[n - 2] * (x(n - 1) - x(n - 3)) / (x(n - 2) - x(n - 3));
    Ok(g)
}

/// Apply `f` to every column (dim 1) or row (dim 2) of `m`.
fn along(
    m: &Matrix,
    dim: usize,
    check: &impl Fn() -> Result<()>,
    f: impl Fn(&[f64]) -> Result<Vec<f64>>,
) -> Result<Matrix> {
    check()?;
    let (rows, cols) = (m.rows(), m.cols());
    let mut out = vec![0.; m.len()];
    if dim == 1 {
        for j in 0..cols {
            check()?;
            let g = f(&m.data()[j * rows..(j + 1) * rows])?;
            for (dst, src) in out[j * rows..(j + 1) * rows]
                .chunks_mut(4096)
                .zip(g.chunks(4096))
            {
                check()?;
                dst.copy_from_slice(src);
            }
        }
    } else {
        for i in 0..rows {
            check()?;
            let mut line = Vec::with_capacity(cols);
            for first in (0..cols).step_by(4096) {
                check()?;
                line.extend((first..cols.min(first + 4096)).map(|j| m.data()[j * rows + i]));
            }
            for (j, v) in f(&line)?.into_iter().enumerate() {
                if j % 4096 == 0 {
                    check()?;
                }
                out[j * rows + i] = v;
            }
        }
    }
    Matrix::from_column_vec(rows, cols, out)
}

/// Unwrap of one line.
fn unwrap_line(p: &[f64], cutoff: f64, check: &impl Fn() -> Result<()>) -> Result<Vec<f64>> {
    use std::f64::consts::PI;
    check()?;
    let mut out = Vec::with_capacity(p.len());
    if let Some(&first) = p.first() {
        out.push(first);
    }
    let mut correction = 0.;
    for k in 1..p.len() {
        if k % 4096 == 0 {
            check()?;
        }
        let dp = p[k] - p[k - 1];
        let mut dps = super::elementary::mod_value(dp + PI, 2. * PI) - PI;
        if dps == -PI && dp > 0. {
            dps = PI;
        }
        let corr = if dp.abs() < cutoff { 0. } else { dps - dp };
        correction += corr;
        out.push(p[k] + correction);
    }
    Ok(out)
}

impl Engine {
    pub(super) fn core_more_call(
        &mut self,
        name: &str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        if let Some((params, body, text)) = operator_body(name) {
            let function = AnonymousFunction {
                params: params.iter().map(|p| (*p).to_string()).collect(),
                body,
                text: text.into(),
            };
            let handle = FunctionHandle::anonymous(Arc::new(function), Vec::new());
            let mut values = self.call_handle(&handle, args.to_vec(), 1, None, out)?;
            return Ok(Some(if values.is_empty() {
                Value::Void
            } else {
                values.remove(0)
            }));
        }
        self.check_cancel()?;
        Ok(Some(match name {
            "nonzeros" => self.core_more_nonzeros(args)?,
            "perms" => self.core_more_perms(args)?,
            "gradient" => self.core_more_gradient(args, 1)?.remove(0),
            "del2" => self.core_more_del2(args)?,
            "unwrap" => self.core_more_unwrap(args)?,
            "hankel" => self.core_more_hankel(args)?,
            "cplxpair" => self.core_more_cplxpair(args)?,
            "orderfields" => self.core_more_orderfields(args)?,
            "narginchk" => self.core_more_narginchk(args)?,
            "optimset" => self.core_more_optimset(args)?,
            _ => return Ok(None),
        }))
    }

    fn core_more_real(&self, name: &str, value: &Value) -> Result<Matrix> {
        self.real_arg(name, value)
    }

    fn core_more_scalar(&self, name: &str, value: &Value) -> Result<f64> {
        let m = self.core_more_real(name, value)?;
        if m.len() != 1 {
            return Err(format!(
                "{name}: spacing must be a scalar; coordinate vectors are not supported yet"
            ));
        }
        Ok(m.data()[0])
    }

    fn core_more_nonzeros(&self, args: &[Value]) -> Result<Value> {
        Self::arity_check("nonzeros", args, &[1])?;
        Ok(match &args[0] {
            Value::Complex(z) => {
                let data: Vec<Complex64> = z
                    .data()
                    .iter()
                    .copied()
                    .filter(|v| v.re != 0. || v.im != 0.)
                    .collect();
                Value::Complex(ComplexMatrix::from_column_vec(
                    data.len(),
                    1,
                    data,
                    &self.cancel,
                )?)
            }
            value => {
                let m = self.core_more_real("nonzeros", value)?;
                let data: Vec<f64> = m.data().iter().copied().filter(|&v| v != 0.).collect();
                let column = Matrix::from_column_vec(data.len(), 1, data)?;
                match value {
                    Value::Logical(_) => self.logical_from(&column)?,
                    v if v.is_character() => {
                        Value::Container(self.characters_from_codes(&column)?.into())
                    }
                    _ => Value::Matrix(column),
                }
            }
        })
    }

    /// All permutations in reverse lexicographic order of positions.
    fn core_more_perms(&self, args: &[Value]) -> Result<Value> {
        Self::arity_check("perms", args, &[1])?;
        let (n, complex, real) = match &args[0] {
            Value::Complex(z) => (z.len(), Some(z.data().to_vec()), None),
            value => {
                let m = self.core_more_real("perms", value)?;
                (m.len(), None, Some(m))
            }
        };
        let mut rows = 1usize;
        for k in 2..=n {
            rows = rows
                .checked_mul(k)
                .filter(|r| r.saturating_mul(n) <= MAX_PERMS_ELEMENTS)
                .ok_or("perms: the result exceeds 8388608 elements")?;
        }
        self.preflight_elements(rows.saturating_mul(n))?;
        // Positions start descending; each row is the previous permutation.
        let mut index: Vec<usize> = (0..n).rev().collect();
        let mut table = vec![0usize; rows * n];
        for r in 0..rows {
            if r % 4096 == 0 {
                self.check_cancel()?;
            }
            for (c, &i) in index.iter().enumerate() {
                table[c * rows + r] = i;
            }
            // Previous permutation in lexicographic order.
            if let Some(i) = (1..n).rev().find(|&i| index[i - 1] > index[i]) {
                let j = (i..n).rev().find(|&j| index[j] < index[i - 1]).unwrap();
                index.swap(i - 1, j);
                index[i..].reverse();
            }
        }
        if let Some(z) = complex {
            let data = table.iter().map(|&i| z[i]).collect();
            return Ok(Value::Complex(ComplexMatrix::from_column_vec(
                rows,
                n,
                data,
                &self.cancel,
            )?));
        }
        let m = real.unwrap();
        let out = Matrix::from_column_vec(rows, n, table.iter().map(|&i| m.data()[i]).collect())?;
        Ok(match &args[0] {
            Value::Logical(_) => self.logical_from(&out)?,
            v if v.is_character() => Value::Container(self.characters_from_codes(&out)?.into()),
            _ => Value::Matrix(out),
        })
    }

    /// gradient(F), gradient(F, h), gradient(F, hx, hy); [FX, FY] = gradient(...).
    pub(super) fn core_more_gradient(&self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        if args.is_empty() {
            return Err("Not enough input arguments.".into());
        }
        if args.len() > 3 {
            return Err("Too many input arguments.".into());
        }
        if count > 2 {
            return Err("gradient supports at most two outputs for a matrix".into());
        }
        if matches!(args[0], Value::Complex(_)) {
            return Err("gradient: complex input is not supported yet".into());
        }
        let f = self.core_more_real("gradient", &args[0])?;
        self.preflight_elements(f.len().saturating_mul(count))?;
        let hx = match args.get(1) {
            Some(h) => self.core_more_scalar("gradient", h)?,
            None => 1.,
        };
        let hy = match args.get(2) {
            Some(h) => self.core_more_scalar("gradient", h)?,
            None => hx,
        };
        let vector = f.rows() == 1 || f.cols() == 1;
        if vector && count == 1 {
            if args.len() > 2 {
                return Err("gradient of a vector takes one spacing".into());
            }
            let g = Matrix::from_column_vec(
                f.rows(),
                f.cols(),
                derivative(f.data(), hx, &|| self.check_cancel())?,
            )?;
            return Ok(vec![Value::Matrix(g)]);
        }
        let mut outputs = vec![Value::Matrix(along(
            &f,
            2,
            &|| self.check_cancel(),
            |line| derivative(line, hx, &|| self.check_cancel()),
        )?)];
        if count == 2 {
            self.check_cancel()?;
            outputs.push(Value::Matrix(along(
                &f,
                1,
                &|| self.check_cancel(),
                |line| derivative(line, hy, &|| self.check_cancel()),
            )?));
        }
        Ok(outputs)
    }

    /// del2(U), del2(U, h), del2(U, hx, hy): (d2U/dx2 + d2U/dy2) / 4.
    fn core_more_del2(&self, args: &[Value]) -> Result<Value> {
        Self::arity_check("del2", args, &[1, 2, 3])?;
        if matches!(args[0], Value::Complex(_)) {
            return Err("del2: complex input is not supported yet".into());
        }
        let u = self.core_more_real("del2", &args[0])?;
        self.preflight_elements(u.len().saturating_mul(2))?;
        let hx = match args.get(1) {
            Some(h) => self.core_more_scalar("del2", h)?,
            None => 1.,
        };
        let hy = match args.get(2) {
            Some(h) => self.core_more_scalar("del2", h)?,
            None => hx,
        };
        let gx = along(&u, 2, &|| self.check_cancel(), |line| {
            second_derivative(line, hx, &|| self.check_cancel())
        })?;
        self.check_cancel()?;
        let gy = along(&u, 1, &|| self.check_cancel(), |line| {
            second_derivative(line, hy, &|| self.check_cancel())
        })?;
        Ok(Value::Matrix(gx.zip(&gy, |a, b| (a + b) / 2.)?))
    }

    /// unwrap(P), unwrap(P, tol), unwrap(P, tol, dim).
    fn core_more_unwrap(&self, args: &[Value]) -> Result<Value> {
        Self::arity_check("unwrap", args, &[1, 2, 3])?;
        if matches!(args[0], Value::Complex(_)) {
            return Err("unwrap: complex input is not supported yet".into());
        }
        let p = self.core_more_real("unwrap", &args[0])?;
        let mut cutoff = std::f64::consts::PI;
        if let Some(tol) = args.get(1) {
            let tol = self.core_more_real("unwrap", tol)?;
            if tol.len() == 1 {
                cutoff = tol.data()[0];
            } else if !tol.is_empty() {
                return Err("unwrap: tolerance must be a scalar".into());
            }
        }
        let dim = match args.get(2) {
            Some(d) => {
                let d = self.core_more_real("unwrap", d)?;
                match d.data() {
                    [d] if *d == 1. || *d == 2. => *d as usize,
                    [d] if d.fract() == 0. && *d > 2. => return Ok(Value::Matrix(p)),
                    _ => return Err("Dimension argument must be a positive integer scalar.".into()),
                }
            }
            None if p.rows() == 1 => 2,
            None => 1,
        };
        self.preflight_elements(p.len())?;
        Ok(Value::Matrix(along(
            &p,
            dim,
            &|| self.check_cancel(),
            |line| unwrap_line(line, cutoff, &|| self.check_cancel()),
        )?))
    }

    /// hankel(c) and hankel(c, r); the column wins the anti-diagonal conflict.
    fn core_more_hankel(&self, args: &[Value]) -> Result<Value> {
        Self::arity_check("hankel", args, &[1, 2])?;
        let c = self.core_more_real("hankel", &args[0])?;
        let r = match args.get(1) {
            Some(r) => self.core_more_real("hankel", r)?,
            None => Matrix::filled(1, c.len(), 0.)?,
        };
        let (m, n) = (c.len(), r.len());
        let count = Matrix::checked_len(m, n)?;
        self.preflight_elements(count)?;
        let mut data = Vec::new();
        data.try_reserve_exact(count)
            .map_err(|_| "Cannot allocate hankel matrix")?;
        for j in 0..n {
            self.check_cancel()?;
            for i in 0..m {
                data.push(if i + j < m {
                    c.data()[i + j]
                } else {
                    r.data()[i + j + 1 - m]
                });
            }
        }
        Ok(Value::Matrix(Matrix::from_column_vec(m, n, data)?))
    }

    /// cplxpair(z) and cplxpair(z, tol) for vectors: conjugate pairs by
    /// increasing real part, negative imaginary part first, then the reals.
    fn core_more_cplxpair(&self, args: &[Value]) -> Result<Value> {
        Self::arity_check("cplxpair", args, &[1, 2])?;
        let (rows, cols, z): (usize, usize, Vec<Complex64>) = match &args[0] {
            Value::Complex(z) => (z.rows(), z.cols(), z.data().to_vec()),
            value => {
                let m = self.core_more_real("cplxpair", value)?;
                let z = m.data().iter().map(|&x| Complex64::new(x, 0.)).collect();
                (m.rows(), m.cols(), z)
            }
        };
        if rows != 1 && cols != 1 && !z.is_empty() {
            return Err("cplxpair: matrix input is not supported yet; pass a vector".into());
        }
        let tol = match args.get(1) {
            Some(t) => {
                let t = self.core_more_real("cplxpair", t)?;
                match t.data() {
                    [] => 100. * f64::EPSILON,
                    [t] => *t,
                    _ => return Err("cplxpair: tolerance must be a scalar".into()),
                }
            }
            None => 100. * f64::EPSILON,
        };
        self.preflight_elements(z.len().saturating_mul(2))?;
        let (mut reals, mut complex): (Vec<Complex64>, Vec<Complex64>) =
            z.into_iter().partition(|v| v.im.abs() <= tol * v.norm());
        reals.sort_by(|a, b| a.re.total_cmp(&b.re));
        complex.sort_by(|a, b| a.re.total_cmp(&b.re));
        let mut out = Vec::with_capacity(rows * cols);
        let mut used = vec![false; complex.len()];
        for i in 0..complex.len() {
            if i % 1024 == 0 {
                self.check_cancel()?;
            }
            if used[i] {
                continue;
            }
            used[i] = true;
            let a = complex[i];
            let partner = (i + 1..complex.len())
                .find(|&j| !used[j] && (complex[j] - a.conj()).norm() <= tol * a.norm());
            let Some(j) = partner else {
                return Err("Complex numbers can't be paired.".into());
            };
            used[j] = true;
            let b = complex[j];
            let (low, high) = if a.im < b.im { (a, b) } else { (b, a) };
            out.push(low);
            out.push(high);
        }
        out.extend(reals.into_iter().map(|v| Complex64::new(v.re, 0.)));
        if out.iter().all(|v| v.im == 0.) {
            let re = out.iter().map(|v| v.re).collect();
            return Ok(Value::Matrix(Matrix::from_column_vec(rows, cols, re)?));
        }
        Ok(Value::Complex(ComplexMatrix::from_column_vec(
            rows,
            cols,
            out,
            &self.cancel,
        )?))
    }

    /// orderfields(s): fields in ASCII order.
    fn core_more_orderfields(&self, args: &[Value]) -> Result<Value> {
        Self::arity_check("orderfields", args, &[1])?;
        let Value::Container(Array::Struct(s)) = &args[0] else {
            return Err("orderfields: the first input must be a structure".into());
        };
        let names = s.field_names();
        let mut order: Vec<usize> = (0..names.len()).collect();
        order.sort_by(|&a, &b| names[a].cmp(&names[b]));
        let mut slots = Vec::with_capacity(s.len() * names.len());
        for element in 0..s.len() {
            self.check_cancel()?;
            for &field in &order {
                slots.push(s.get(field, element)?.clone());
            }
        }
        let sorted = order.iter().map(|&k| names[k].clone()).collect();
        Ok(Value::Container(Array::Struct(StructArray::from_slots(
            s.rows(),
            s.cols(),
            sorted,
            slots,
            &self.cancel,
        )?)))
    }

    /// narginchk(minArgs, maxArgs) against the calling function's nargin.
    fn core_more_narginchk(&self, args: &[Value]) -> Result<Value> {
        if args.len() < 2 {
            return Err("Not enough input arguments.".into());
        }
        if args.len() > 2 {
            return Err("Too many input arguments.".into());
        }
        let (inputs, _) = self
            .functions
            .arguments
            .ok_or("You can only call nargin/nargout from within a MATLAB function.")?;
        let low = self.core_more_scalar("narginchk", &args[0])?;
        let high = self.core_more_scalar("narginchk", &args[1])?;
        let n = inputs as f64;
        if n < low {
            return Err("Not enough input arguments.".into());
        }
        if n > high {
            return Err("Too many input arguments.".into());
        }
        Ok(Value::Void)
    }

    /// optimset('Name', value, ...) and optimset(oldopts, 'Name', value, ...):
    /// a structure holding exactly the given options.
    fn core_more_optimset(&self, args: &[Value]) -> Result<Value> {
        let (mut names, mut slots, pairs) = match args.first() {
            Some(Value::Container(Array::Struct(s))) => {
                if s.len() != 1 {
                    return Err("optimset: the options structure must be scalar".into());
                }
                let slots = (0..s.field_count())
                    .map(|f| s.get(f, 0).cloned())
                    .collect::<Result<Vec<_>>>()?;
                (s.field_names().to_vec(), slots, &args[1..])
            }
            _ => (Vec::new(), Vec::new(), args),
        };
        if pairs.len() % 2 != 0 {
            return Err("Arguments must occur in name-value pairs.".into());
        }
        for pair in pairs.chunks(2) {
            self.check_cancel()?;
            let name = self
                .text_value(pair[0].clone())
                .map_err(|_| String::from("optimset: option names must be character vectors"))?;
            let value = Arc::new(pair[1].clone().array(&self.cancel)?);
            match names.iter().position(|n| n.eq_ignore_ascii_case(&name)) {
                Some(k) => slots[k] = value,
                None => {
                    names.push(name);
                    slots.push(value);
                }
            }
        }
        Ok(Value::Container(Array::Struct(StructArray::from_slots(
            1,
            1,
            names,
            slots,
            &self.cancel,
        )?)))
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use std::cell::Cell;

    fn cancel_at(call: usize) -> impl Fn() -> Result<()> {
        let calls = Cell::new(0);
        move || {
            calls.set(calls.get() + 1);
            if calls.get() == call {
                Err("Calculation cancelled".into())
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn long_lines_cancel_inside_each_kernel() {
        let line = vec![1.; 16_384];
        assert_eq!(
            derivative(&line, 1., &cancel_at(2)).unwrap_err(),
            "Calculation cancelled"
        );
        assert_eq!(
            second_derivative(&line, 1., &cancel_at(2)).unwrap_err(),
            "Calculation cancelled"
        );
        assert_eq!(
            unwrap_line(&line, 3., &cancel_at(2)).unwrap_err(),
            "Calculation cancelled"
        );
    }

    #[test]
    fn traversal_cancels_between_lines_and_inside_gather_and_copy() {
        let matrix = Matrix::filled(2, 16_384, 1.).unwrap();
        let calls = Cell::new(0);
        let identity = |line: &[f64]| {
            calls.set(calls.get() + 1);
            Ok(line.to_vec())
        };
        // Cancellation in the second gather chunk, before the row callback.
        assert_eq!(
            along(&matrix, 2, &cancel_at(4), identity).unwrap_err(),
            "Calculation cancelled"
        );
        assert_eq!(calls.get(), 0);
        // Cancellation between two short columns.
        assert_eq!(
            along(&matrix, 1, &cancel_at(4), identity).unwrap_err(),
            "Calculation cancelled"
        );
        assert_eq!(calls.get(), 1);
        let column = Matrix::filled(16_384, 1, 1.).unwrap();
        assert_eq!(
            along(&column, 1, &cancel_at(4), identity).unwrap_err(),
            "Calculation cancelled"
        );
        assert_eq!(calls.get(), 2);
    }
}
