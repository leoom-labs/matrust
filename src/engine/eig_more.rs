//! Eigenvalue functions and factorization updates: eig in
//! every form, eigs, svds, condeig, funm, polyeig, qz, ordschur, ordeig,
//! ordqz, gsvd, cholupdate, qrupdate, qrinsert and qrdelete.
//!
//! The kernels in the submodules are ports of reference LAPACK 3.5 in its
//! unblocked form, the path LAPACK takes for the small matrices these
//! functions see most; their operation order follows the Fortran so the
//! results agree with MATLAB's to the last digit as often as possible.
mod cancel_bidiagonal;
mod cancel_svd;
pub(super) mod cdense;
pub(super) mod cgen;
pub(super) mod dense;
mod eigs;
mod funm;
mod ordering;
mod polygsvd;
mod qzfun;
pub(super) mod rgen;
mod schur;
mod sym;
mod updates;

#[cfg(test)]
mod review_cancellation {
    use super::{cancel_svd, cdense, cgen, dense, ordering, rgen, sym};
    use crate::engine::{Execution, Value};
    use crate::{Engine, Matrix};
    use std::cell::Cell;

    #[test]
    fn qz_preflights_combined_storage_for_all_entry_points() {
        let mut e = Engine::default();
        assert!(
            e.lx3_qz_budget(3000, false, 1)
                .unwrap_err()
                .contains("budget")
        );
        assert!(
            e.lx3_qz_budget(2000, true, 1)
                .unwrap_err()
                .contains("budget")
        );
        e.lx3_qz_budget(16, true, 6).unwrap();
        let a = Value::Matrix(Matrix::identity(4, 4).unwrap());
        // The operands fit, but their combined factors do not. Exercise the
        // actual entry points without a test that allocates hundreds of MiB.
        e.evaluated_elements = 32 * 1024 * 1024 - 128;
        for (name, args) in [
            ("qz", vec![a.clone(), a.clone()]),
            ("eig", vec![a.clone(), a.clone()]),
            (
                "ordqz",
                vec![a.clone(), a.clone(), a.clone(), a.clone(), a.clone()],
            ),
        ] {
            let error = e
                .eigx_outputs(name, &args, 1, &mut Execution::default())
                .unwrap_err();
            assert!(error.contains("budget"), "{name}: {error}");
        }
    }

    fn sample() -> dense::Mat {
        dense::Mat::from_vec(8, 8, (0..64).map(|i| ((i * i + 3) as f64).sin()).collect())
    }

    #[test]
    fn cancellable_svd_preserves_reference_values_and_residuals() {
        for (m, n) in [
            (1, 1),
            (1, 5),
            (5, 1),
            (2, 2),
            (3, 3),
            (5, 3),
            (3, 5),
            (12, 8),
        ] {
            let a = nalgebra::DMatrix::from_fn(m, n, |i, j| ((i * i + j * 3 + 1) as f64).sin());
            let r = cancel_svd::Svd::try_new(
                a.clone(),
                true,
                true,
                f64::EPSILON,
                500 * m.max(n),
                &|| false,
            )
            .unwrap();
            let reference =
                nalgebra::linalg::SVD::try_new(a.clone(), true, true, f64::EPSILON, 500 * m.max(n))
                    .unwrap();
            assert!((&r.singular_values - reference.singular_values).norm() < 1e-13);
            let s = nalgebra::DMatrix::from_diagonal(&r.singular_values);
            assert!((r.u.unwrap() * s * r.v_t.unwrap() - a.clone()).norm() < 1e-13);
            let z = a.map(|x| cdense::c(x, (x * 3.).cos()));
            let r = cancel_svd::Svd::try_new(
                z.clone(),
                true,
                true,
                f64::EPSILON,
                500 * m.max(n),
                &|| false,
            )
            .unwrap();
            let reference =
                nalgebra::linalg::SVD::try_new(z.clone(), true, true, f64::EPSILON, 500 * m.max(n))
                    .unwrap();
            assert!((&r.singular_values - reference.singular_values).norm() < 1e-13);
            let s = nalgebra::DMatrix::from_diagonal(&r.singular_values.map(|x| cdense::c(x, 0.)));
            assert!((r.u.unwrap() * s * r.v_t.unwrap() - z).norm() < 1e-13);
        }
    }

    #[test]
    fn symmetric_and_hermitian_reduction_and_q_observe_cancellation() {
        for lower in [false, true] {
            let a = sample();
            let calls = Cell::new(0);
            let cancel = || {
                calls.set(calls.get() + 1);
                calls.get() >= 3
            };
            let mut h = a.clone();
            sym::sytd2(&mut h, lower, &cancel);
            assert_eq!(calls.get(), 3);
            assert_ne!(h, a);
            calls.set(0);
            let mut z = cdense::CMat::from_real(&a);
            cdense::zhetd2(&mut z, lower, &cancel);
            assert_eq!(calls.get(), 3);
            assert_ne!(z, cdense::CMat::from_real(&a));
            let mut h = a;
            let (_, _, tau) = sym::sytd2(&mut h, lower, &|| false);
            // Two copying loops precede the cubic reflector assembly.
            calls.set(0);
            let cancel = || {
                calls.set(calls.get() + 1);
                calls.get() >= 17
            };
            sym::orgtr(&h, lower, &tau, &cancel);
            assert_eq!(calls.get(), 17);
            let mut z = cdense::CMat::from_real(&sample());
            let (_, _, tau) = cdense::zhetd2(&mut z, lower, &|| false);
            calls.set(0);
            cdense::zungtr(&z, lower, &tau, &cancel);
            assert_eq!(calls.get(), 17);
        }
    }

    #[test]
    fn generalized_reduction_and_vector_solves_observe_cancellation() {
        let a = sample();
        let calls = Cell::new(0);
        let cancel = || {
            calls.set(calls.get() + 1);
            calls.get() >= 10
        };
        let (mut h, mut b) = (a.clone(), dense::Mat::identity(8));
        rgen::gghrd(&mut h, &mut b, 0, 7, None, None, &cancel);
        assert_eq!(calls.get(), 10);
        assert_ne!(h, a);
        calls.set(0);
        let (mut z, mut b) = (cdense::CMat::from_real(&a), cdense::CMat::identity(8));
        cgen::zgghrd(&mut z, &mut b, 0, 7, None, None, &cancel);
        assert_eq!(calls.get(), 10);
        assert_ne!(z, cdense::CMat::from_real(&a));
        let mut t = a;
        for j in 0..8 {
            t.set(j, j, (j + 1) as f64);
            for i in j + 1..8 {
                t.set(i, j, 0.);
            }
        }
        calls.set(0);
        dense::trevc(&t, None, Some(&mut dense::Mat::identity(8)), &cancel);
        assert_eq!(calls.get(), 10);
        calls.set(0);
        assert!(
            rgen::tgevc(
                &t,
                &dense::Mat::identity(8),
                None,
                Some(&mut dense::Mat::identity(8)),
                &cancel
            )
            .is_err()
        );
        assert_eq!(calls.get(), 10);
        let z = cdense::CMat::from_real(&t);
        for generalized in [false, true] {
            calls.set(0);
            let stop = if generalized { 10 } else { 3 };
            let cancel = || {
                calls.set(calls.get() + 1);
                calls.get() >= stop
            };
            if generalized {
                cgen::ztgevc(
                    &z,
                    &cdense::CMat::identity(8),
                    None,
                    Some(&mut cdense::CMat::identity(8)),
                    &cancel,
                );
            } else {
                cdense::ztrevc(&z, None, Some(&mut cdense::CMat::identity(8)), &cancel);
            }
            assert_eq!(calls.get(), stop);
        }
    }

    #[test]
    fn reordering_and_svd_observe_cancellation_mid_kernel() {
        let mut t = sample();
        for j in 0..8 {
            t.set(j, j, (j + 1) as f64);
            for i in j + 1..8 {
                t.set(i, j, 0.);
            }
        }
        let calls = Cell::new(0);
        let cancel = || {
            calls.set(calls.get() + 1);
            calls.get() >= 5
        };
        let select = [false, true, false, true, false, true, false, true];
        assert!(
            ordering::trsen(
                &mut t.clone(),
                &mut dense::Mat::identity(8),
                &select,
                &cancel
            )
            .is_err()
        );
        assert_eq!(calls.get(), 5);
        calls.set(0);
        ordering::ztrsen(
            &mut cdense::CMat::from_real(&t),
            &mut cdense::CMat::identity(8),
            &select,
            &cancel,
        );
        assert_eq!(calls.get(), 5);
        calls.set(0);
        assert!(
            ordering::tgsen(
                &mut t.clone(),
                &mut dense::Mat::identity(8),
                &mut dense::Mat::identity(8),
                &mut dense::Mat::identity(8),
                &select,
                &cancel
            )
            .is_err()
        );
        assert_eq!(calls.get(), 5);
        calls.set(0);
        assert!(
            ordering::ztgsen(
                &mut cdense::CMat::from_real(&t),
                &mut cdense::CMat::identity(8),
                &mut cdense::CMat::identity(8),
                &mut cdense::CMat::identity(8),
                &select,
                &cancel
            )
            .is_err()
        );
        assert_eq!(calls.get(), 5);
        let a = nalgebra::DMatrix::from_column_slice(8, 8, &sample().data);
        // Bidiagonalization, U/V assembly and the QR iteration respectively.
        for stop in [3, 11, 19, 27] {
            calls.set(0);
            let cancel = || {
                calls.set(calls.get() + 1);
                calls.get() >= stop
            };
            assert!(
                cancel_svd::Svd::try_new(a.clone(), true, true, f64::EPSILON, 4000, &cancel)
                    .is_none()
            );
            assert!(calls.get() >= stop && calls.get() <= stop + 2);
        }
    }

    #[test]
    fn complex_division_handles_tiny_and_huge_operands() {
        for s in [1e-300, 1e-200, 1., 1e200, 1e300] {
            for divide in [cdense::cdiv, cdense::cdiv_recip] {
                let result = divide(cdense::c(s, s), cdense::c(s, -s));
                assert!(result.re.abs() < 1e-14, "{s}: {result:?}");
                assert!((result.im - 1.).abs() < 1e-14, "{s}: {result:?}");
            }
        }
    }

    #[test]
    fn reduction_and_qr_observe_cancellation_mid_kernel() {
        let a = dense::Mat::from_vec(8, 8, (0..64).map(|i| ((i * i + 3) as f64).sin()).collect());
        for complex in [false, true] {
            let calls = Cell::new(0);
            let cancel = || {
                calls.set(calls.get() + 1);
                calls.get() == 3
            };
            if complex {
                let mut h = cdense::CMat::from_real(&a);
                assert!(cdense::zgehd2(&mut h, 0, 7, &cancel).is_err());
                assert_ne!(h, cdense::CMat::from_real(&a));
            } else {
                let mut h = a.clone();
                assert!(dense::gehd2(&mut h, 0, 7, &cancel).is_err());
                assert_ne!(h, a);
            }
            assert_eq!(calls.get(), 3);
            calls.set(0);
            if complex {
                let mut h = cdense::CMat::from_real(&a);
                cdense::zgehd2(&mut h, 0, 7, &|| false).unwrap();
                assert!(cdense::zhseqr(false, &mut h, 0, 7, None, &cancel).is_err());
            } else {
                let mut h = a.clone();
                dense::gehd2(&mut h, 0, 7, &|| false).unwrap();
                assert!(dense::hseqr(false, &mut h, 0, 7, None, &cancel).is_err());
            }
            assert_eq!(calls.get(), 3);
        }
    }
}

use super::{Engine, Execution, Value};
use crate::typed_array::{NumericClass, TypedArray};
use crate::{Array, Complex64, ComplexMatrix, Matrix, Result};
use cdense::CMat;
use dense::Mat;

/// Names this module answers for.
pub(super) fn handles(name: &str) -> bool {
    matches!(
        name,
        "eig"
            | "eigs"
            | "svds"
            | "condeig"
            | "funm"
            | "qz"
            | "ordschur"
            | "ordeig"
            | "ordqz"
            | "polyeig"
            | "gsvd"
            | "cholupdate"
            | "qrupdate"
            | "qrinsert"
            | "qrdelete"
    )
}

/// A numeric operand: real or complex, double precision (single inputs
/// are computed in double and converted back).
#[derive(Clone)]
enum Num {
    R(Mat),
    C(CMat),
}

impl Num {
    fn rows(&self) -> usize {
        match self {
            Num::R(m) => m.rows,
            Num::C(m) => m.rows,
        }
    }
    fn cols(&self) -> usize {
        match self {
            Num::R(m) => m.cols,
            Num::C(m) => m.cols,
        }
    }
    fn finite(&self) -> bool {
        match self {
            Num::R(m) => m.data.iter().all(|v| v.is_finite()),
            Num::C(m) => m.data.iter().all(|v| v.re.is_finite() && v.im.is_finite()),
        }
    }
    fn complex(&self) -> CMat {
        match self {
            Num::R(m) => CMat::from_real(m),
            Num::C(m) => m.clone(),
        }
    }
}

/// An operand with the class it came in.
struct Operand {
    num: Num,
    single: bool,
}

fn text_arg(value: &Value) -> Option<String> {
    match value {
        Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
        Value::Container(Array::Character(c)) if c.rows() <= 1 => {
            c.to_utf8(&std::sync::atomic::AtomicBool::new(false)).ok()
        }
        _ => None,
    }
}

fn diag_real(values: &[f64]) -> Mat {
    let n = values.len();
    let mut d = Mat::zeros(n, n);
    for (i, &v) in values.iter().enumerate() {
        d.set(i, i, v);
    }
    d
}

fn diag_complex(values: &[Complex64]) -> CMat {
    let n = values.len();
    let mut d = CMat::zeros(n, n);
    for (i, &v) in values.iter().enumerate() {
        d.set(i, i, v);
    }
    d
}

/// Real eigenvector pairs (columns j, j+1 for wi[j] > 0) as complex columns.
fn pair_vectors(v: &Mat, wi: &[f64]) -> CMat {
    let n = v.rows;
    let mut out = CMat::zeros(n, v.cols);
    let mut j = 0;
    while j < v.cols {
        if wi[j] == 0. {
            for i in 0..n {
                out.set(i, j, cdense::c(v.at(i, j), 0.));
            }
            j += 1;
        } else {
            for i in 0..n {
                let (re, im) = (v.at(i, j), v.at(i, j + 1));
                // MATLAB's conjugate pair has +0, never -0, imaginary zeros.
                out.set(i, j, cdense::c(re, im + 0.));
                out.set(i, j + 1, cdense::c(re, 0. - im));
            }
            j += 2;
        }
    }
    out
}

impl Engine {
    /// One-output calls of the functions in this module.
    pub(super) fn eigx_call(
        &mut self,
        name: &str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        if !handles(name) {
            return Ok(None);
        }
        Ok(Some(self.eigx_outputs(name, args, 1, out)?.remove(0)))
    }

    /// Multiple-output calls; `count` is the number of requested outputs.
    pub(super) fn eigx_outputs(
        &mut self,
        name: &str,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        self.check_cancel()?;
        let count = count.max(1);
        let max_inputs = match name {
            "ordeig" => Some(2),
            "qz" | "ordschur" | "gsvd" | "funm" => Some(3),
            "eig" => Some(4),
            "ordqz" => Some(5),
            "condeig" => Some(1),
            "svds" => Some(4),
            _ => None,
        };
        if max_inputs.is_some_and(|max| args.len() > max) {
            return Err("Too many input arguments.".into());
        }
        // Inspect shared metadata before lx3_operand creates any working
        // copies. The drivers repeat this check for internal callers.
        let pencil = match name {
            "qz" if args.len() >= 2 => Some((
                2,
                args.get(2)
                    .and_then(text_arg)
                    .is_none_or(|s| !s.eq_ignore_ascii_case("real")),
            )),
            "eig" if args.len() >= 2 && text_arg(&args[1]).is_none() => Some((2, false)),
            "ordqz" if args.len() >= 5 => Some((4, false)),
            _ => None,
        };
        if let Some((operands, force_complex)) = pencil {
            let meta = |v: &Value| -> Option<(usize, usize, bool)> {
                match v {
                    Value::Matrix(m) => Some((m.rows(), m.cols(), false)),
                    Value::Complex(m) => Some((m.rows(), m.cols(), true)),
                    Value::Container(Array::Numeric(m)) => Some((m.rows(), m.cols(), false)),
                    Value::Container(Array::Complex(m)) => Some((m.rows(), m.cols(), true)),
                    Value::Container(Array::Typed(m)) if m.class() == NumericClass::Single => {
                        Some((m.rows(), m.cols(), false))
                    }
                    _ => None,
                }
            };
            if let Some((n, cols, _)) = meta(&args[0])
                && n == cols
                && args[..operands]
                    .iter()
                    .all(|v| meta(v).is_some_and(|(r, c, _)| r == n && c == n))
            {
                let complex = force_complex
                    || args[..operands]
                        .iter()
                        .any(|v| meta(v).is_some_and(|(_, _, z)| z));
                self.lx3_qz_budget(n, complex, count)?;
            }
        }
        let result = (|| {
            Ok(match name {
                "eig" => self.lx3_eig(args, count)?,
                "eigs" => self.lx3_eigs(args, count, out)?,
                "svds" => self.lx3_svds(args, count, out)?,
                "condeig" => self.lx3_condeig(args, count)?,
                "funm" => self.lx3_funm(args, count, out)?,
                "qz" => self.lx3_qz(args, count)?,
                "ordschur" => self.lx3_ordschur(args, count)?,
                "ordeig" => self.lx3_ordeig(args)?,
                "ordqz" => self.lx3_ordqz(args, count)?,
                "polyeig" => self.lx3_polyeig(args, count)?,
                "gsvd" => self.lx3_gsvd(args, count)?,
                "cholupdate" => self.lx3_cholupdate(args, count)?,
                "qrupdate" => self.lx3_qrupdate(args, count)?,
                "qrinsert" => self.lx3_qrinsert(args, count)?,
                "qrdelete" => self.lx3_qrdelete(args, count)?,
                _ => return Err(format!("{name} is not supported")),
            })
        })();
        self.check_cancel()?;
        let mut outputs = result?;
        if outputs.len() < count {
            return Err("Too many output arguments.".into());
        }
        outputs.truncate(count);
        Ok(outputs)
    }

    fn lx3_cancelled(&self) -> impl Fn() -> bool + '_ {
        move || self.cancel.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// A double or single numeric operand; other classes give MATLAB's
    /// "Undefined function" error for `fname`.
    fn lx3_operand(&self, fname: &str, value: &Value) -> Result<Operand> {
        let undefined = |class: &str| {
            format!("Undefined function '{fname}' for input arguments of type '{class}'.")
        };
        let array = value.clone().array(&self.cancel)?;
        let (num, single) = match array {
            Array::Numeric(m) => (
                Num::R(Mat::from_vec(m.rows(), m.cols(), m.data().to_vec())),
                false,
            ),
            Array::Complex(z) => (
                Num::C(CMat {
                    rows: z.rows(),
                    cols: z.cols(),
                    data: z.data().to_vec(),
                }),
                false,
            ),
            Array::Typed(t) if t.class() == NumericClass::Single => {
                let m = t.to_matrix()?;
                (
                    Num::R(Mat::from_vec(m.rows(), m.cols(), m.data().to_vec())),
                    true,
                )
            }
            other => return Err(undefined(other.class())),
        };
        self.preflight_elements(num.rows().saturating_mul(num.cols()))?;
        Ok(Operand { num, single })
    }

    fn lx3_real_value(&self, m: Mat, single: bool) -> Result<Value> {
        if single {
            let t =
                TypedArray::from_reals(NumericClass::Single, m.rows, m.cols, m.data.into_iter())?;
            return Ok(Value::Container(Array::Typed(t)));
        }
        Ok(Value::Matrix(Matrix::from_column_vec(
            m.rows, m.cols, m.data,
        )?))
    }

    fn lx3_complex_value(&self, m: CMat, single: bool) -> Result<Value> {
        // Storage currently represents complex single results as complex double,
        // retaining single rounding. This also preserves the pre-existing eig call.
        let mut m = m;
        if single {
            for z in &mut m.data {
                *z = cdense::c(z.re as f32 as f64, z.im as f32 as f64);
            }
        }
        Ok(Value::Complex(ComplexMatrix::from_column_vec(
            m.rows,
            m.cols,
            m.data,
            &self.cancel,
        )?))
    }

    fn lx3_num_value(&self, n: Num, single: bool) -> Result<Value> {
        match n {
            Num::R(m) => self.lx3_real_value(m, single),
            Num::C(m) => self.lx3_complex_value(m, single),
        }
    }

    // ---------------------------------------------------------------- eig

    fn lx3_eig(&mut self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        if args.is_empty() {
            return Err("Not enough input arguments.".into());
        }
        if args.len() > 4 {
            return Err("Too many input arguments.".into());
        }
        if count > 3 {
            return Err("Too many output arguments.".into());
        }
        let a = self.lx3_operand("eig", &args[0])?;
        let mut rest = &args[1..];
        let mut b = None;
        if let Some(first) = rest.first()
            && text_arg(first).is_none()
        {
            b = Some(self.lx3_operand("eig", first)?);
            rest = &rest[1..];
        }
        let mut vector = None;
        let mut balance = true;
        let mut algorithm = None;
        for flag in rest {
            let text = text_arg(flag).map(|t| t.to_lowercase());
            match (text.as_deref(), b.is_some()) {
                (Some("vector"), _) => vector = Some(true),
                (Some("matrix"), _) => vector = Some(false),
                (Some("balance"), false) => balance = true,
                (Some("nobalance"), false) => balance = false,
                (Some("chol"), true) => algorithm = Some(true),
                (Some("qz"), true) => algorithm = Some(false),
                (_, false) => return Err("For standard eigenproblem EIG(A), flag must be 'vector', 'matrix', 'balance', or 'nobalance'.".into()),
                (_, true) => return Err("For generalized eigenproblem EIG(A,B), flag must be 'chol', 'qz', 'vector', or 'matrix'.".into()),
            }
        }
        let as_vector = vector.unwrap_or(count < 2);
        let single = a.single || b.as_ref().is_some_and(|b| b.single);
        if let Some(b) = b {
            return self.lx3_eig_generalized(a.num, b.num, count, as_vector, single, algorithm);
        }
        if a.num.rows() != a.num.cols() {
            return Err("For standard eigenproblem EIG(A), A must be square.".into());
        }
        if !a.num.finite() {
            return Err("Input to EIG must not contain NaN or Inf.".into());
        }
        let (values, vr, vl) = self.lx3_eig_standard(&a.num, count, balance)?;
        self.lx3_eig_outputs(values, vr, vl, count, as_vector, single)
    }

    /// Package (values, V, W) as eig's outputs.
    fn lx3_eig_outputs(
        &self,
        values: Num,
        vr: Option<Num>,
        vl: Option<Num>,
        count: usize,
        as_vector: bool,
        single: bool,
    ) -> Result<Vec<Value>> {
        // MATLAB's eig never returns a negative zero imaginary part.
        let unsigned = |v: Num| match v {
            Num::C(mut m) => {
                for z in m.data.iter_mut() {
                    *z = cdense::c(z.re, z.im + 0.);
                }
                Num::C(m)
            }
            real => real,
        };
        let values = unsigned(values);
        let vr = vr.map(unsigned);
        let vl = vl.map(unsigned);
        let d = if as_vector {
            values
        } else {
            match values {
                Num::R(v) => Num::R(diag_real(&v.data)),
                Num::C(v) => Num::C(diag_complex(&v.data)),
            }
        };
        if count < 2 {
            return Ok(vec![self.lx3_num_value(d, single)?]);
        }
        let mut out = vec![
            self.lx3_num_value(vr.ok_or("eig: vectors were not computed")?, single)?,
            self.lx3_num_value(d, single)?,
        ];
        if count > 2 {
            out.push(self.lx3_num_value(vl.ok_or("eig: vectors were not computed")?, single)?);
        }
        Ok(out)
    }

    /// Standard problem: (values, right vectors, left vectors).
    fn lx3_eig_standard(
        &self,
        a: &Num,
        count: usize,
        balance: bool,
    ) -> Result<(Num, Option<Num>, Option<Num>)> {
        let n = a.rows();
        let cancel = self.lx3_cancelled();
        let want_v = count >= 2;
        let want_w = count >= 3;
        if n == 0 {
            let v = want_v.then(|| Num::R(Mat::zeros(0, 0)));
            let w = want_w.then(|| Num::R(Mat::zeros(0, 0)));
            return Ok((Num::R(Mat::zeros(0, 1)), v, w));
        }
        match a {
            Num::R(m) => {
                let symmetric = (0..n).all(|j| (0..j).all(|i| m.at(i, j) == m.at(j, i)));
                if symmetric {
                    let (d, q) = sym::syev(m, want_v, false, &cancel)
                        .map_err(|_| "eig: the iteration did not converge".to_string())?;
                    let q = q.map(Num::R);
                    let w = if want_w { q.clone() } else { None };
                    return Ok((Num::R(Mat::from_vec(n, 1, d)), q, w));
                }
                let r = dense::geevx(m, balance, want_w, want_v, &cancel).map_err(|_| {
                    self.check_cancel()
                        .err()
                        .unwrap_or_else(|| "eig: the QR algorithm failed to converge".to_string())
                })?;
                let real = r.wi.iter().all(|&v| v == 0.);
                let values = if real {
                    Num::R(Mat::from_vec(n, 1, r.wr.clone()))
                } else {
                    Num::C(CMat {
                        rows: n,
                        cols: 1,
                        data: r
                            .wr
                            .iter()
                            .zip(&r.wi)
                            .map(|(&a, &b)| cdense::c(a, b))
                            .collect(),
                    })
                };
                let wrap = |v: Option<Mat>| {
                    v.map(|v| {
                        if real {
                            Num::R(v)
                        } else {
                            Num::C(pair_vectors(&v, &r.wi))
                        }
                    })
                };
                Ok((values, wrap(r.vr), wrap(r.vl)))
            }
            Num::C(z) => {
                let hermitian = (0..n).all(|j| (0..=j).all(|i| z.at(i, j) == z.at(j, i).conj()));
                if hermitian {
                    let (d, q) = cdense::zheev(z, want_v, false, &cancel)
                        .map_err(|_| "eig: the iteration did not converge".to_string())?;
                    let q = q.map(Num::C);
                    let w = if want_w { q.clone() } else { None };
                    return Ok((Num::R(Mat::from_vec(n, 1, d)), q, w));
                }
                let r = cdense::zgeevx(z, balance, want_w, want_v, &cancel).map_err(|_| {
                    self.check_cancel()
                        .err()
                        .unwrap_or_else(|| "eig: the QR algorithm failed to converge".to_string())
                })?;
                Ok((
                    Num::C(CMat {
                        rows: n,
                        cols: 1,
                        data: r.w,
                    }),
                    r.vr.map(Num::C),
                    r.vl.map(Num::C),
                ))
            }
        }
    }

    fn lx3_eig_generalized(
        &mut self,
        a: Num,
        b: Num,
        count: usize,
        as_vector: bool,
        single: bool,
        chol: Option<bool>,
    ) -> Result<Vec<Value>> {
        let n = a.rows();
        if n != a.cols() || b.rows() != b.cols() || b.rows() != n {
            return Err("For generalized eigenproblem EIG(A,B), A and B must be square.".into());
        }
        if !a.finite() || !b.finite() {
            return Err("Input to EIG must not contain NaN or Inf.".into());
        }
        let (values, vr, vl) = self.lx3_eig_pencil(&a, &b, count, chol.unwrap_or(true))?;
        self.lx3_eig_outputs(values, vr, vl, count, as_vector, single)
    }

    /// eig(A,B): the Cholesky path for a symmetric (Hermitian) A with a
    /// symmetric (Hermitian) positive definite B when `try_chol`, else QZ.
    fn lx3_eig_pencil(
        &self,
        a: &Num,
        b: &Num,
        count: usize,
        try_chol: bool,
    ) -> Result<(Num, Option<Num>, Option<Num>)> {
        let n = a.rows();
        self.lx3_qz_budget(n, matches!(a, Num::C(_)) || matches!(b, Num::C(_)), count)?;
        let cancel = self.lx3_cancelled();
        let want_v = count >= 2;
        let want_w = count >= 3;
        if n == 0 {
            let v = want_v.then(|| Num::R(Mat::zeros(0, 0)));
            let w = want_w.then(|| Num::R(Mat::zeros(0, 0)));
            return Ok((Num::R(Mat::zeros(0, 1)), v, w));
        }
        let failed = |_| "eig: the QZ iteration failed to converge".to_string();
        match (a, b) {
            (Num::R(a), Num::R(b)) => {
                if try_chol && symmetric(a) && symmetric(b) {
                    match rgen::sygv(a, b, want_v, &cancel) {
                        Ok((d, v)) => {
                            let v = v.map(Num::R);
                            let w = if want_w { v.clone() } else { None };
                            return Ok((Num::R(Mat::from_vec(n, 1, d)), v, w));
                        }
                        Err(Some(())) => return Err("eig: the iteration did not converge".into()),
                        Err(None) => {}
                    }
                }
                let r = rgen::ggev(a, b, want_w, want_v, &cancel).map_err(failed)?;
                let real = r.alphai.iter().all(|&v| v == 0.);
                let values = if real {
                    Num::R(Mat::from_vec(
                        n,
                        1,
                        r.alphar.iter().zip(&r.beta).map(|(&a, &b)| a / b).collect(),
                    ))
                } else {
                    Num::C(CMat {
                        rows: n,
                        cols: 1,
                        data: (0..n)
                            .map(|i| {
                                let im = if r.alphai[i] == 0. {
                                    0.
                                } else {
                                    r.alphai[i] / r.beta[i]
                                };
                                cdense::c(r.alphar[i] / r.beta[i], im)
                            })
                            .collect(),
                    })
                };
                let wrap = |v: Option<Mat>| {
                    v.map(|v| {
                        if real {
                            Num::R(v)
                        } else {
                            Num::C(pair_vectors(&v, &r.alphai))
                        }
                    })
                };
                Ok((values, wrap(r.vr), wrap(r.vl)))
            }
            _ => {
                let (a, b) = (a.complex(), b.complex());
                if try_chol && hermitian(&a) && hermitian(&b) {
                    match cgen::zhegv(&a, &b, want_v, &cancel) {
                        Ok((d, v)) => {
                            let v = v.map(Num::C);
                            let w = if want_w { v.clone() } else { None };
                            return Ok((Num::R(Mat::from_vec(n, 1, d)), v, w));
                        }
                        Err(Some(())) => return Err("eig: the iteration did not converge".into()),
                        Err(None) => {}
                    }
                }
                let r = cgen::zggev(&a, &b, want_w, want_v, &cancel).map_err(failed)?;
                let values = CMat {
                    rows: n,
                    cols: 1,
                    data: r
                        .alpha
                        .iter()
                        .zip(&r.beta)
                        .map(|(&x, &y)| cdense::cdiv(x, y))
                        .collect(),
                };
                Ok((Num::C(values), r.vr.map(Num::C), r.vl.map(Num::C)))
            }
        }
    }
}

fn symmetric(m: &Mat) -> bool {
    m.rows == m.cols && (0..m.cols).all(|j| (0..j).all(|i| m.at(i, j) == m.at(j, i)))
}

fn hermitian(m: &CMat) -> bool {
    m.rows == m.cols && (0..m.cols).all(|j| (0..=j).all(|i| m.at(i, j) == m.at(j, i).conj()))
}
