//! eig, svd, rank and polyfit on real matrices through nalgebra's dense
//! decompositions. Systems are solved from a QR factorization, never from an
//! explicit inverse or the normal equations.
use super::{Engine, Value};
use crate::{Complex64, ComplexMatrix, Matrix, Result};
use nalgebra::{
    DMatrix, DVector,
    linalg::{ColPivQR, QR, SVD, Schur, SymmetricEigen},
};

fn columns_norm(x: &Matrix, power: i32) -> f64 {
    x.data()
        .iter()
        .map(|v| v.powi(2 * power))
        .sum::<f64>()
        .sqrt()
}

fn text_of(value: &Value) -> Option<String> {
    match value {
        Value::Text(s) | Value::QuotedString(s) => Some(s.to_lowercase()),
        _ => None,
    }
}

fn to_dense(m: &Matrix) -> DMatrix<f64> {
    DMatrix::from_column_slice(m.rows(), m.cols(), m.data())
}

fn from_dense(m: &DMatrix<f64>) -> Result<Matrix> {
    Matrix::from_column_slice(m.nrows(), m.ncols(), m.as_slice())
}

/// Spacing of floating point numbers at |x| (MATLAB `eps(x)`).
fn spacing(x: f64) -> f64 {
    let x = x.abs();
    f64::from_bits(x.to_bits() + 1) - x
}

fn iteration_limit(n: usize) -> usize {
    500 * n.max(1)
}

const NONFINITE: &str = "Input must not contain NaN or Inf.";

/// eig of a 0x0 matrix: 0x1 (or 0x0 with 'matrix') values, 0x0 vectors.
fn empty_eig(count: usize, as_vector: bool) -> Result<Vec<Value>> {
    let values = Value::Matrix(Matrix::filled(0, usize::from(as_vector), 0.)?);
    Ok(if count == 2 {
        vec![Value::Matrix(Matrix::filled(0, 0, 0.)?), values]
    } else {
        vec![values]
    })
}

impl Engine {
    fn finite_square_or_matrix(&self, name: &str, value: &Value) -> Result<Matrix> {
        let m = self.real_arg(name, value)?;
        if m.data().iter().any(|v| !v.is_finite()) {
            return Err(format!("{name}: {NONFINITE}"));
        }
        Ok(m)
    }

    fn diagonal(&self, values: &[f64], rows: usize, cols: usize) -> Result<Matrix> {
        self.preflight_elements(Matrix::checked_len(rows, cols)?)?;
        let mut d = DMatrix::zeros(rows, cols);
        for (i, &v) in values.iter().enumerate() {
            d[(i, i)] = v;
        }
        from_dense(&d)
    }

    /// eig outputs of a symmetric or Hermitian decomposition: real eigenvalues
    /// in ascending order and, for [V,D], their unit eigenvectors as columns.
    fn ascending_outputs<T: nalgebra::ComplexField<RealField = f64> + Copy>(
        &self,
        eigen: &SymmetricEigen<T, nalgebra::Dyn>,
        count: usize,
        as_vector: bool,
        vectors_value: impl Fn(Vec<T>) -> Result<Value>,
    ) -> Result<Vec<Value>> {
        let n = eigen.eigenvalues.len();
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&i, &j| eigen.eigenvalues[i].total_cmp(&eigen.eigenvalues[j]));
        let values: Vec<f64> = order.iter().map(|&i| eigen.eigenvalues[i]).collect();
        let mut out = Vec::new();
        if count == 2 {
            let mut vectors = Vec::with_capacity(n * n);
            for &i in &order {
                vectors.extend_from_slice(eigen.eigenvectors.column(i).as_slice());
            }
            out.push(vectors_value(vectors)?);
        }
        out.push(Value::Matrix(if as_vector {
            Matrix::from_column_vec(n, 1, values)?
        } else {
            self.diagonal(&values, n, n)?
        }));
        Ok(out)
    }

    /// eig of a complex matrix that equals its conjugate transpose exactly:
    /// real ascending eigenvalues and unit-norm complex eigenvectors.
    fn hermitian_eig(
        &self,
        z: &ComplexMatrix,
        count: usize,
        as_vector: bool,
    ) -> Result<Vec<Value>> {
        let n = z.rows();
        if n != z.cols() {
            return Err("eig: input must be a square matrix".into());
        }
        if z.data()
            .iter()
            .any(|v| !v.re.is_finite() || !v.im.is_finite())
        {
            return Err(format!("eig: {NONFINITE}"));
        }
        if n == 0 {
            return empty_eig(count, as_vector);
        }
        // Complex elements take two budget units each.
        self.preflight_elements(2 * n * n)?;
        let dense = DMatrix::from_column_slice(n, n, z.data());
        let hermitian = (0..n).all(|j| (0..=j).all(|i| dense[(i, j)] == dense[(j, i)].conj()));
        if !hermitian {
            return Err("eig: complex non-Hermitian matrices are not supported yet".into());
        }
        self.check_cancel()?;
        let eigen = SymmetricEigen::try_new(dense, f64::EPSILON, iteration_limit(n))
            .ok_or("eig: the iteration did not converge")?;
        self.check_cancel()?;
        self.ascending_outputs(&eigen, count, as_vector, |v| {
            Ok(Value::Complex(ComplexMatrix::from_column_vec(
                n,
                n,
                v,
                &self.cancel,
            )?))
        })
    }

    pub(super) fn eig_call(&mut self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        if args.is_empty() || args.len() > 2 {
            return Err("eig expects 1 or 2 arguments".into());
        }
        if count > 2 {
            return Err("eig: three outputs [V,D,W] are not supported yet".into());
        }
        let mut as_vector = count < 2;
        if let Some(flag) = args.get(1) {
            match text_of(flag).as_deref() {
                Some("vector") => as_vector = true,
                Some("matrix") => as_vector = false,
                Some(other) => return Err(format!("eig: option '{other}' is not supported yet")),
                None => {
                    return Err("eig: the generalized problem eig(A,B) is not supported yet".into());
                }
            }
        }
        let a = match &args[0] {
            Value::Complex(z) => return self.hermitian_eig(z, count, as_vector),
            other => self.finite_square_or_matrix("eig", other)?,
        };
        if a.rows() != a.cols() {
            return Err("eig: input must be a square matrix".into());
        }
        let n = a.rows();
        if n == 0 {
            return empty_eig(count, as_vector);
        }
        self.preflight_elements(n * n)?;
        let dense = to_dense(&a);
        let symmetric = (0..n).all(|j| (0..j).all(|i| dense[(i, j)] == dense[(j, i)]));
        self.check_cancel()?;
        if symmetric {
            let eigen = SymmetricEigen::try_new(dense, f64::EPSILON, iteration_limit(n))
                .ok_or("eig: the iteration did not converge")?;
            self.check_cancel()?;
            return self.ascending_outputs(&eigen, count, as_vector, |v| {
                Ok(Value::Matrix(Matrix::from_column_vec(n, n, v)?))
            });
        }
        if count == 2 {
            return Err("eig: [V,D] for non-symmetric matrices is not supported yet".into());
        }
        let schur = Schur::try_new(dense, f64::EPSILON, iteration_limit(n))
            .ok_or("eig: the iteration did not converge")?;
        self.check_cancel()?;
        let mut values: Vec<Complex64> = schur.complex_eigenvalues().iter().copied().collect();
        // Conjugate pairs: positive imaginary part first, as MATLAB lists them.
        for i in 0..n.saturating_sub(1) {
            let (p, q) = (values[i], values[i + 1]);
            if p.im < 0. && q.im > 0. && p.re == q.re && p.im == -q.im {
                values.swap(i, i + 1);
            }
        }
        if !as_vector {
            if values.iter().any(|v| v.im != 0.) {
                return Err("eig: complex eigenvalue matrices are not supported yet; use eig(A) or eig(A,'vector')".into());
            }
            let re: Vec<f64> = values.iter().map(|v| v.re).collect();
            return Ok(vec![Value::Matrix(self.diagonal(&re, n, n)?)]);
        }
        Ok(vec![if values.iter().all(|v| v.im == 0.) {
            Value::Matrix(Matrix::from_column_vec(
                n,
                1,
                values.iter().map(|v| v.re).collect(),
            )?)
        } else {
            Value::Complex(ComplexMatrix::from_column_vec(n, 1, values, &self.cancel)?)
        }])
    }

    pub(super) fn svd_call(&mut self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        if args.is_empty() || args.len() > 2 {
            return Err("svd expects 1 or 2 arguments".into());
        }
        if matches!(args[0], Value::Complex(_)) {
            return self.cxl_svd_call(args, count);
        }
        if count == 2 || count > 3 {
            return Err(format!("svd does not support {count} outputs; use 1 or 3"));
        }
        let a = self.finite_square_or_matrix("svd", &args[0])?;
        let (m, n) = (a.rows(), a.cols());
        let k = m.min(n);
        // 'econ' or 0: economy size (svd(A,0) is the full form when m <= n).
        let economy = match args.get(1) {
            None => false,
            Some(Value::Matrix(z)) if z.len() == 1 && z.data()[0] == 0. => m > n,
            Some(v) => match text_of(v).as_deref() {
                Some("econ") => true,
                _ => return Err("svd: only the 'econ' and 0 options are supported yet".into()),
            },
        };
        let want_vectors = count == 3;
        if want_vectors {
            // Every output must fit the per-matrix limit before any work.
            let (u_cols, s_rows, s_cols, v_cols) =
                if economy { (k, k, k, k) } else { (m, m, n, n) };
            let elements = Matrix::checked_len(m, u_cols)?
                + Matrix::checked_len(s_rows, s_cols)?
                + Matrix::checked_len(n, v_cols)?;
            self.preflight_elements(elements)?;
        }
        let (u_dense, s, v_dense) = if k == 0 {
            (DMatrix::identity(m, k), Vec::new(), DMatrix::identity(n, k))
        } else {
            self.check_cancel()?;
            let svd = SVD::try_new(
                to_dense(&a),
                want_vectors,
                want_vectors,
                f64::EPSILON,
                iteration_limit(m.max(n)),
            )
            .ok_or("svd: the iteration did not converge")?;
            self.check_cancel()?;
            let mut order: Vec<usize> = (0..k).collect();
            order.sort_by(|&i, &j| svd.singular_values[j].total_cmp(&svd.singular_values[i]));
            let s: Vec<f64> = order.iter().map(|&i| svd.singular_values[i]).collect();
            if !want_vectors {
                return Ok(vec![Value::Matrix(Matrix::from_column_vec(k, 1, s)?)]);
            }
            let u = svd.u.ok_or("svd: left vectors were not computed")?;
            let vt = svd.v_t.ok_or("svd: right vectors were not computed")?;
            let u = DMatrix::from_fn(m, k, |r, c| u[(r, order[c])]);
            let v = DMatrix::from_fn(n, k, |r, c| vt[(order[c], r)]);
            (u, s, v)
        };
        if !want_vectors {
            return Ok(vec![Value::Matrix(Matrix::from_column_vec(k, 1, s)?)]);
        }
        let (u, v, s_rows, s_cols) = if economy {
            (u_dense, v_dense, k, k)
        } else {
            (
                self.complete_basis(&u_dense)?,
                self.complete_basis(&v_dense)?,
                m,
                n,
            )
        };
        Ok(vec![
            Value::Matrix(from_dense(&u)?),
            Value::Matrix(self.diagonal(&s, s_rows, s_cols)?),
            Value::Matrix(from_dense(&v)?),
        ])
    }

    /// Extend orthonormal columns (rows >= cols) to a square orthogonal matrix.
    /// The k Householder reflectors of Q's own QR, applied to the identity,
    /// give the complete orthogonal factor (transposed) in O(rows^2 * k); its
    /// trailing columns are an orthonormal basis of the complement of Q.
    fn complete_basis(&self, q: &DMatrix<f64>) -> Result<DMatrix<f64>> {
        let (n, k) = (q.nrows(), q.ncols());
        if k == n {
            return Ok(q.clone());
        }
        let mut full = DMatrix::identity(n, n);
        QR::new(q.clone()).q_tr_mul(&mut full);
        self.check_cancel()?;
        full.transpose_mut();
        full.columns_mut(0, k).copy_from(q);
        Ok(full)
    }

    pub(super) fn rank_call(&mut self, args: &[Value]) -> Result<Value> {
        Self::arity_check("rank", args, &[1, 2])?;
        if matches!(args[0], Value::Complex(_)) {
            return self.cxl_rank_call(args);
        }
        let a = self.finite_square_or_matrix("rank", &args[0])?;
        let tolerance = match args.get(1) {
            Some(v) => Some(
                self.real_arg("rank", v)?
                    .as_scalar()
                    .map_err(|_| "rank: the tolerance must be a scalar".to_string())?,
            ),
            None => None,
        };
        if a.is_empty() {
            return Ok(Value::Matrix(Matrix::scalar(0.)));
        }
        self.check_cancel()?;
        let svd = SVD::try_new(
            to_dense(&a),
            false,
            false,
            f64::EPSILON,
            iteration_limit(a.rows().max(a.cols())),
        )
        .ok_or("rank: the iteration did not converge")?;
        self.check_cancel()?;
        let largest = svd.singular_values.iter().copied().fold(0., f64::max);
        let tolerance =
            tolerance.unwrap_or_else(|| a.rows().max(a.cols()) as f64 * spacing(largest));
        let rank = svd
            .singular_values
            .iter()
            .filter(|&&s| s > tolerance)
            .count();
        Ok(Value::Matrix(Matrix::scalar(rank as f64)))
    }

    pub(super) fn polyfit_call(&mut self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        if count > 1 {
            return Err("polyfit: the [p,S] and [p,S,mu] outputs are not supported yet".into());
        }
        Self::arity_check("polyfit", args, &[3])?;
        let x = self.finite_square_or_matrix("polyfit", &args[0])?;
        let y = self.finite_square_or_matrix("polyfit", &args[1])?;
        let degree = self
            .real_arg("polyfit", &args[2])?
            .as_scalar()
            .map_err(|_| "polyfit: the degree must be a nonnegative integer scalar".to_string())?;
        if !degree.is_finite() || degree < 0. || degree.fract() != 0. || degree > 4096. {
            return Err(
                "polyfit: the degree must be a nonnegative integer scalar (at most 4096)".into(),
            );
        }
        let degree = degree as usize;
        if x.len() != y.len() {
            return Err("polyfit: X and Y must be vectors of the same length".into());
        }
        if (x.rows() != 1 && x.cols() != 1) || (y.rows() != 1 && y.cols() != 1) {
            return Err("polyfit: X and Y must be vectors".into());
        }
        let points = x.len();
        let terms = degree + 1;
        self.preflight_elements(points * terms)?;
        // Vandermonde columns x^degree .. x^0, built by repeated multiplication.
        let mut columns = vec![0.; points * terms];
        for (r, &v) in x.data().iter().enumerate() {
            let mut power = 1.;
            for c in (0..terms).rev() {
                columns[r + c * points] = power;
                power *= v;
            }
        }
        self.check_cancel()?;
        // Column-pivoted QR (MATLAB backslash): the rank comes from the pivoted
        // R diagonal and dependent columns get coefficient 0 (basic solution).
        // MATLAB's "badly conditioned" warning has no output path from here.
        let qr = ColPivQR::new(DMatrix::from_vec(points, terms, columns));
        let rhs = DVector::from_column_slice(y.data());
        let projected = qr.q().transpose() * rhs;
        let r = qr.r();
        // Rank test per column: |R_ii| against the norm of its own Vandermonde
        // column, so a large x offset (years) is not mistaken for dependence.
        let mut norms = DVector::from_iterator(
            terms,
            (0..terms).map(|c| columns_norm(&x, (terms - 1 - c) as i32)),
        );
        qr.p().permute_rows(&mut norms);
        let eps_n = points.max(terms) as f64 * f64::EPSILON;
        // Underdetermined systems have only min(points, terms) diagonal
        // entries; the remaining coefficients stay zero in the basic solution.
        let rank = (0..points.min(terms))
            .take_while(|&i| r[(i, i)].abs() > eps_n * norms[i])
            .count();
        let mut solved = DVector::zeros(terms);
        if rank > 0 {
            let head = r
                .view((0, 0), (rank, rank))
                .solve_upper_triangular(&projected.rows(0, rank).into_owned())
                .ok_or("polyfit: the polynomial is rank deficient for these data")?;
            solved.rows_mut(0, rank).copy_from(&head);
        }
        qr.p().inv_permute_rows(&mut solved);
        let p = solved;
        self.check_cancel()?;
        Ok(vec![Value::Matrix(Matrix::from_column_vec(
            1,
            terms,
            p.as_slice().to_vec(),
        )?)])
    }
}
