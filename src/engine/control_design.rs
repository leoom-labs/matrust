//! Dense control design, independent of the optional UI.
//!
//! CARE: Laub, "A Schur Method for Solving Algebraic Riccati Equations",
//! IEEE TAC 24 (1979), sections II and IV (balance, order, solve).
//! DARE: Pappas, Laub & Sandell, "On the Numerical Solution of the
//! Discrete-Time Algebraic Riccati Equation", IEEE TAC 25 (1980):
//! stable deflating subspace of the symplectic pencil, without inverting A.
//! Pole assignment: Kautsky, Nichols & Van Dooren, "Robust pole assignment
//! in linear state feedback", IJC 41 (1985), sections 3.1--3.3, method 0.
//! Conjugate pairs use a constrained two-column determinant update; see
//! Tits & Yang, "Globally Convergent Algorithms for Robust Pole Assignment
//! by State Feedback", TR 95-3 (1995), for the two-column interpretation.
use super::{Engine, Execution, Value, control_lti};
use crate::{Complex64 as C, ComplexMatrix, Matrix, Result};
use nalgebra::{DMatrix, DVector};

const MAX_STATES: usize = 128;
const MAX_ENTRIES: usize = 1_048_576;
const MAX_GAINS: usize = 4096;
pub(super) fn cds_handles(name: &str) -> bool {
    matches!(
        name,
        "ctrb"
            | "obsv"
            | "care"
            | "dare"
            | "lqr"
            | "dlqr"
            | "place"
            | "acker"
            | "rlocus"
            | "lyap"
            | "dlyap"
            | "pidtune"
            | "pidtuneOptions"
    )
}
fn cds_real(v: &Value) -> Result<Matrix> {
    let Value::Matrix(m) = v else {
        return Err("Control design requires real double matrices".into());
    };
    if m.rows() > MAX_ENTRIES
        || m.cols() > MAX_ENTRIES
        || m.len() > MAX_ENTRIES
        || m.data().iter().any(|x| !x.is_finite())
    {
        return Err("Control design input exceeds its size bound or contains NaN/Inf".into());
    }
    Ok(m.clone())
}
fn cds_dm(m: &Matrix) -> DMatrix<f64> {
    DMatrix::from_column_slice(m.rows(), m.cols(), m.data())
}
fn cds_mat(m: &DMatrix<f64>) -> Result<Matrix> {
    Matrix::from_column_slice(m.nrows(), m.ncols(), m.as_slice())
}
fn cds_value(m: &DMatrix<f64>) -> Result<Value> {
    Ok(Value::Matrix(cds_mat(m)?))
}
fn cds_complex(v: &Value) -> Result<DMatrix<C>> {
    match v {
        Value::Matrix(m) => Ok(cds_dm(m).map(|x| C::new(x, 0.))),
        Value::Complex(m) => Ok(DMatrix::from_column_slice(m.rows(), m.cols(), m.data())),
        _ => Err("Expected a numeric matrix".into()),
    }
}
fn cds_cvalue(e: &Engine, m: &DMatrix<C>) -> Result<Value> {
    if m.iter().all(|z| z.im == 0.) {
        return cds_value(&m.map(|z| z.re));
    }
    Ok(Value::Complex(ComplexMatrix::from_column_vec(
        m.nrows(),
        m.ncols(),
        m.as_slice().to_vec(),
        &e.cancel,
    )?))
}
fn cds_square(a: &Matrix) -> Result<usize> {
    if a.rows() != a.cols() || a.rows() == 0 || a.rows() > MAX_STATES {
        return Err("Control design requires a nonempty square A with at most 128 states".into());
    }
    Ok(a.rows())
}
fn cds_ab(a: &Matrix, b: &Matrix) -> Result<usize> {
    let n = cds_square(a)?;
    if b.rows() != n || b.cols() == 0 || b.cols() > MAX_STATES {
        return Err("B must have n rows and 1..128 input columns".into());
    }
    Ok(n)
}
fn cds_shape(m: &Matrix, r: usize, c: usize, name: &str) -> Result<()> {
    if (m.rows(), m.cols()) != (r, c) {
        return Err(format!("{name} must be {r} by {c}"));
    }
    Ok(())
}
fn cds_sym(m: &Matrix, name: &str) -> Result<()> {
    if m.rows() != m.cols() {
        return Err(format!("{name} must be symmetric"));
    }
    let difference = cds_sub(m, &m.transpose())?;
    if cds_norm(&difference) > 1e-12 * cds_norm(m).max(f64::MIN_POSITIVE) {
        return Err(format!(
            "{name} must be symmetric to relative tolerance 1e-12"
        ));
    }
    Ok(())
}
fn cds_add(a: &Matrix, b: &Matrix) -> Result<Matrix> {
    a.zip(b, |x, y| x + y)
}
fn cds_sub(a: &Matrix, b: &Matrix) -> Result<Matrix> {
    a.zip(b, |x, y| x - y)
}
fn cds_norm(a: &Matrix) -> f64 {
    a.data().iter().fold(0_f64, |s, x| s.hypot(*x))
}
fn cds_pencil_scale(a: &Matrix, scale: &[f64]) -> Result<Matrix> {
    // A diagonal change of units is elementwise scaling, not a dense solve
    // with the balancing matrix. Powers-of-two factors come from balance.
    Matrix::from_column_vec(
        a.rows(),
        a.cols(),
        (0..a.len())
            .map(|k| {
                let i = k % a.rows();
                let j = k / a.rows();
                (a.data()[k] / scale[i]) * scale[j]
            })
            .collect(),
    )
}
fn cds_lti(e: &Engine, v: &Value, operation: &str) -> Result<control_lti::Lti> {
    if let Value::Container(a) = v
        && let Some(s) = control_lti::decode(e, a)?
    {
        s.require_delay_free(operation)?;
        return Ok(s);
    }
    Err("Expected an LTI model".into())
}
fn cds_ctrb(e: &Engine, a: &Matrix, b: &Matrix) -> Result<Matrix> {
    let n = cds_ab(a, b)?;
    let cols = n
        .checked_mul(b.cols())
        .ok_or("Controllability dimensions overflow")?;
    if n * cols > MAX_ENTRIES {
        return Err("Controllability output exceeds 1048576 entries".into());
    }
    let mut data = Vec::with_capacity(n * cols);
    let mut block = b.clone();
    for j in 0..n {
        e.check_cancel()?;
        data.extend_from_slice(block.data());
        if j + 1 < n {
            block = a.mul(&block)?;
        }
    }
    if data.iter().any(|x| !x.is_finite()) {
        return Err("Controllability powers overflowed".into());
    }
    Matrix::from_column_vec(n, cols, data)
}
fn cds_eigen(e: &mut Engine, a: &Matrix, out: &mut Execution) -> Result<DMatrix<C>> {
    cds_complex(
        &e.eigx_outputs("eig", &[Value::Matrix(a.clone())], 1, out)?
            .remove(0),
    )
}
fn cds_stable(z: C, discrete: bool) -> bool {
    if discrete { z.norm() < 1. } else { z.re < 0. }
}

// Newton correction: Ac' D + D Ac = -F (CARE), or
// Ac' D Ac - D = -F (DARE). See Palitta (2019), section 2.
// The discrete equation uses Popov's bilinear transform, as described by
// SciPy solve_discrete_lyapunov. Reuse the existing Bartels-Stewart solver;
// all transforms are solves, with O(n^2) storage and no Kronecker matrix.
fn cds_riccati_correction(
    e: &mut Engine,
    closed: &Matrix,
    residual: &Matrix,
    discrete: bool,
    out: &mut Execution,
) -> Result<Matrix> {
    e.check_cancel()?;
    let (a, rhs) = if discrete {
        let eye = Matrix::identity(closed.rows(), closed.cols())?;
        let plus = cds_add(closed, &eye)?;
        let a = plus.solve(&cds_sub(closed, &eye)?)?;
        let left = plus.transpose().solve(residual)?;
        let rhs = plus.transpose().solve(&left.transpose())?.transpose();
        (a, rhs.map(|x| -2. * x))
    } else {
        (closed.clone(), residual.map(|x| -x))
    };
    let result = e
        .lax_outputs(
            "sylvester",
            &[
                Value::Matrix(a.transpose()),
                Value::Matrix(a),
                Value::Matrix(rhs),
            ],
            1,
            out,
        )?
        .ok_or("Riccati refinement: Sylvester solver unavailable")?;
    let d = cds_real(&result[0])?;
    Ok(cds_add(&d, &d.transpose())?.map(|x| 0.5 * x))
}

// Both algorithms return the task's [X,K,L] order (K is the feedback gain).
fn cds_riccati(
    e: &mut Engine,
    args: &[Value],
    discrete: bool,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if !(3..=6).contains(&args.len()) {
        return Err("care/dare require A,B,Q[,R,S,E]".into());
    }
    let a = cds_real(&args[0])?;
    let b = cds_real(&args[1])?;
    let n = cds_ab(&a, &b)?;
    let m = b.cols();
    let q = cds_real(&args[2])?;
    cds_shape(&q, n, n, "Q")?;
    cds_sym(&q, "Q")?;
    let r = if let Some(v) = args.get(3) {
        let r = cds_real(v)?;
        if r.is_empty() {
            Matrix::identity(m, m)?
        } else {
            r
        }
    } else {
        Matrix::identity(m, m)?
    };
    cds_shape(&r, m, m, "R")?;
    cds_sym(&r, "R")?;
    if cds_dm(&r).cholesky().is_none() {
        return Err("R must be symmetric positive definite".into());
    }
    let s = if let Some(v) = args.get(4) {
        let s = cds_real(v)?;
        if s.is_empty() {
            Matrix::filled(n, m, 0.)?
        } else {
            s
        }
    } else {
        Matrix::filled(n, m, 0.)?
    };
    cds_shape(&s, n, m, "S")?;
    if let Some(v) = args.get(5) {
        let descriptor = cds_real(v)?;
        if !descriptor.is_empty() {
            cds_shape(&descriptor, n, n, "E")?;
            if descriptor.data() != Matrix::identity(n, n)?.data() {
                return Err("care/dare: descriptor E other than identity is not supported".into());
            }
        }
    }
    let rst = r.solve(&s.transpose())?;
    let a0 = cds_sub(&a, &b.mul(&rst)?)?;
    let q0 = cds_sub(&q, &s.mul(&rst)?)?;
    let g = b.mul(&r.solve(&b.transpose())?)?;
    e.check_cancel()?;
    let zero = Matrix::filled(n, n, 0.)?;
    let eye = Matrix::identity(n, n)?;
    let h = if discrete {
        Matrix::concatenate(vec![
            vec![a0.clone(), zero.clone()],
            vec![q0.map(|x| -x), eye.clone()],
        ])?
    } else {
        Matrix::concatenate(vec![
            vec![a0.clone(), g.map(|x| -x)],
            vec![q0.map(|x| -x), a0.transpose().map(|x| -x)],
        ])?
    };
    let u = if discrete {
        let j = Matrix::concatenate(vec![vec![eye, g], vec![zero, a0.transpose()]])?;
        // Balance the combined off-diagonal magnitudes of the pencil. Apply
        // the same diagonal similarity to H,J and undo it on the right vectors.
        let magnitude = h.zip(&j, |x, y| x.abs() + y.abs())?;
        let balanced = e
            .lax_outputs(
                "balance",
                &[Value::Matrix(magnitude), Value::Text("noperm".into())],
                2,
                out,
            )?
            .ok_or("balance unavailable")?;
        let t = cds_real(&balanced[0])?;
        let scale: Vec<_> = (0..2 * n).map(|i| t.get(i, i)).collect();
        let hb = cds_pencil_scale(&h, &scale)?;
        let jb = cds_pencil_scale(&j, &scale)?;
        let factors = e.eigx_outputs(
            "qz",
            &[
                Value::Matrix(hb),
                Value::Matrix(jb),
                Value::Text("complex".into()),
            ],
            4,
            out,
        )?;
        let eigs = e.eigx_outputs("ordeig", &factors[..2], 1, out)?;
        let roots = cds_complex(&eigs[0])?;
        if roots.iter().filter(|&&z| cds_stable(z, true)).count() != n {
            return Err(
                "dare: no separated stabilising subspace (system may be non-stabilisable)".into(),
            );
        }
        let mut ordered = factors;
        ordered.push(Value::Text("udi".into()));
        let factors = e.eigx_outputs("ordqz", &ordered, 4, out)?;
        let z = cds_complex(&factors[3])?;
        DMatrix::from_fn(2 * n, 2 * n, |i, j| z[(i, j)] * scale[i])
    } else {
        // Laub section IV.B: H_b=T^-1 H T; backtransform T U before
        // solving X U11=U21. Reuse balance and real Schur/ordschur.
        let balanced = e
            .lax_outputs("balance", &[Value::Matrix(h)], 2, out)?
            .ok_or("balance unavailable")?;
        let t = cds_real(&balanced[0])?;
        let factors = e
            .lax_outputs("schur", &[balanced[1].clone()], 2, out)?
            .ok_or("schur unavailable")?;
        let roots = e.eigx_outputs("ordeig", &[factors[1].clone()], 1, out)?;
        if cds_complex(&roots[0])?
            .iter()
            .filter(|&&z| cds_stable(z, false))
            .count()
            != n
        {
            return Err(
                "care: no separated stabilising subspace (system may be non-stabilisable)".into(),
            );
        }
        let mut ordered = factors;
        ordered.push(Value::Text("lhp".into()));
        let factors = e.eigx_outputs("ordschur", &ordered, 2, out)?;
        cds_complex(&Value::Matrix(t.mul(&cds_real(&factors[0])?)?))?
    };
    e.check_cancel()?;
    let u11 = u.view((0, 0), (n, n)).into_owned();
    let u21 = u.view((n, 0), (n, n)).into_owned();
    // Transpose solve, never form an explicit inverse.
    let xc = u11
        .transpose()
        .lu()
        .solve(&u21.transpose())
        .ok_or("Riccati stable subspace is singular; no finite stabilising solution")?
        .transpose();
    let re = xc.map(|z| z.re);
    let asym = (&re - re.transpose()).norm();
    if xc.iter().any(|z| !z.re.is_finite() || !z.im.is_finite())
        || xc.map(|z| z.im).norm() > 1e-9 * re.norm().max(f64::MIN_POSITIVE)
        || asym > 1e-8 * re.norm().max(f64::MIN_POSITIVE)
    {
        return Err("Riccati stable subspace does not yield a real symmetric solution".into());
    }
    let mut x = cds_mat(&((&re + re.transpose()) * 0.5))?;
    // One bounded Newton-Kleinman refinement if the Schur/QZ candidate
    // misses the residual target. Check stability before and after the step.
    for refinement in 0..=1 {
        e.check_cancel()?;
        let bt = b.transpose();
        let k = if discrete {
            cds_add(&r, &bt.mul(&x)?.mul(&b)?)?
                .solve(&cds_add(&bt.mul(&x)?.mul(&a)?, &s.transpose())?)?
        } else {
            r.solve(&cds_add(&bt.mul(&x)?, &s.transpose())?)?
        };
        let closed = cds_sub(&a, &b.mul(&k)?)?;
        let poles = cds_eigen(e, &closed, out)?;
        if !poles
            .iter()
            .all(|&z| z.re.is_finite() && z.im.is_finite() && cds_stable(z, discrete))
        {
            return Err(
                "Riccati solution is not stabilising (system may be non-stabilisable)".into(),
            );
        }
        let ax = if discrete {
            a.transpose().mul(&x)?.mul(&a)?
        } else {
            cds_add(&a.transpose().mul(&x)?, &x.mul(&a)?)?
        };
        let cross = if discrete {
            cds_add(&a.transpose().mul(&x)?.mul(&b)?, &s)?
        } else {
            cds_add(&x.mul(&b)?, &s)?
        };
        let feedback = cross.mul(&k)?;
        let residual = cds_sub(&cds_add(&ax, &q)?, &feedback)?;
        let residual = if discrete {
            cds_sub(&residual, &x)?
        } else {
            residual
        };
        let scale = cds_norm(&ax)
            + cds_norm(&q)
            + cds_norm(&feedback)
            + if discrete { cds_norm(&x) } else { 0. };
        if !scale.is_finite() || !cds_norm(&residual).is_finite() {
            return Err("Riccati residual calculation overflowed; rescale the problem".into());
        }
        if cds_norm(&residual) > 1e-10 * scale.max(f64::MIN_POSITIVE) {
            if refinement == 1 {
                return Err("Riccati relative residual exceeds 1e-10 after Newton refinement; problem is ill-conditioned".into());
            }
            let correction = cds_riccati_correction(e, &closed, &residual, discrete, out)?;
            x = cds_add(&x, &correction)?;
            continue;
        }
        return Ok(vec![
            Value::Matrix(x),
            Value::Matrix(k),
            cds_cvalue(e, &poles)?,
        ]);
    }
    unreachable!("bounded Riccati refinement returns on its final pass")
}

fn cds_poles(v: &Value, n: usize) -> Result<Vec<C>> {
    let (rows, cols, len) = match v {
        Value::Matrix(m) => (m.rows(), m.cols(), m.len()),
        Value::Complex(m) => (m.rows(), m.cols(), m.len()),
        _ => return Err("Poles must be a finite numeric vector".into()),
    };
    if len != n || (rows != 1 && cols != 1) {
        return Err("Pole vector must have exactly n entries".into());
    }
    let mut p = cds_complex(v)?.as_slice().to_vec();
    if p.iter().any(|z| !z.re.is_finite() || !z.im.is_finite()) {
        return Err("Poles must be finite".into());
    }
    let mut result = Vec::with_capacity(n);
    while let Some(z) = p.pop() {
        if z.im.abs() <= 1e-14 * (1. + z.norm()) {
            result.push(C::new(z.re, 0.));
            continue;
        }
        let i = p
            .iter()
            .position(|w| (*w - z.conj()).norm() <= 1e-12 * (1. + z.norm()))
            .ok_or("Complex poles must come in conjugate pairs")?;
        let w = p.remove(i);
        let z = (z + w.conj()) * 0.5;
        let z = if z.im > 0. { z } else { z.conj() };
        result.extend([z, z.conj()]);
    }
    Ok(result)
}
fn cds_svd(
    m: DMatrix<f64>,
    u: bool,
    v: bool,
) -> Result<nalgebra::linalg::SVD<f64, nalgebra::Dyn, nalgebra::Dyn>> {
    if m.iter().any(|x| !x.is_finite()) {
        return Err("Control design arithmetic overflowed before SVD".into());
    }
    let iterations = 128 * m.nrows().max(m.ncols()) + 1;
    m.try_svd(u, v, f64::EPSILON * 5., iterations)
        .ok_or_else(|| "Control design SVD did not converge within its iteration bound".into())
}
fn cds_rank(m: &DMatrix<f64>) -> Result<usize> {
    let svd = cds_svd(m.clone(), false, false)?;
    let max = svd.singular_values.max();
    Ok(svd
        .singular_values
        .iter()
        .filter(|&&s| s > 1e-12 * (m.nrows().max(m.ncols()) as f64) * max)
        .count())
}
// Orthogonal reachability expansion for place. Unlike the public ctrb
// matrix, this never forms high powers of A (KNV section 3.1 recommends
// orthogonal reductions for the preliminary controllability decision).
fn cds_reachable(e: &Engine, a: &DMatrix<f64>, b: &DMatrix<f64>, rank: usize) -> Result<bool> {
    let n = a.nrows();
    let svd = cds_svd(b.clone(), true, false)?;
    let mut basis = svd
        .u
        .ok_or("place: input SVD failed")?
        .columns(0, rank)
        .into_owned();
    let mut frontier = basis.clone();
    let tolerance = 1e-12 * n as f64 * a.norm().max(f64::MIN_POSITIVE);
    while basis.ncols() < n {
        e.check_cancel()?;
        let mut residual = a * &frontier;
        for _ in 0..2 {
            residual -= &basis * (basis.transpose() * &residual);
        }
        let svd = cds_svd(residual, true, false)?;
        let added = svd
            .singular_values
            .iter()
            .filter(|&&s| s > tolerance)
            .count()
            .min(n - basis.ncols());
        if added == 0 {
            return Ok(false);
        }
        frontier = svd
            .u
            .ok_or("place: reachability SVD failed")?
            .columns(0, added)
            .into_owned();
        let old = basis.ncols();
        basis = basis.insert_columns(old, added, 0.);
        basis.columns_mut(old, added).copy_from(&frontier);
    }
    Ok(true)
}
fn cds_acker(e: &Engine, a: &Matrix, b: &Matrix, p: &[C]) -> Result<Matrix> {
    let n = cds_ab(a, b)?;
    if b.cols() != 1 {
        return Err("acker requires a single input column".into());
    }
    let ctr = cds_ctrb(e, a, b)?;
    if cds_rank(&cds_dm(&ctr))? != n {
        return Err("acker: system is uncontrollable at numerical rank tolerance".into());
    }
    let mut coeff = vec![C::new(1., 0.)];
    for &z in p {
        let mut next = vec![C::new(0., 0.); coeff.len() + 1];
        for (i, &c) in coeff.iter().enumerate() {
            next[i] += c;
            next[i + 1] -= z * c;
        }
        coeff = next;
    }
    let eye = Matrix::identity(n, n)?;
    let mut phi = eye.clone();
    for c in &coeff[1..] {
        e.check_cancel()?;
        phi = cds_add(&a.mul(&phi)?, &eye.map(|x| x * c.re))?;
    }
    let mut last = vec![0.; n];
    last[n - 1] = 1.;
    let w = ctr
        .transpose()
        .solve(&Matrix::from_column_vec(n, 1, last)?)?;
    let gain = w.transpose().mul(&phi)?;
    if gain.data().iter().any(|x| !x.is_finite()) {
        return Err(
            "acker: polynomial calculation overflowed; use place or rescale the problem".into(),
        );
    }
    Ok(gain)
}

// Complete orthogonal complement from a square QR with zero padding. The
// supplied columns are independent; no Gram matrix or normal equations.
fn cds_complement(x: &DMatrix<f64>) -> DMatrix<f64> {
    let n = x.nrows();
    let k = x.ncols();
    if k == 0 {
        return DMatrix::identity(n, n);
    }
    let mut padded = DMatrix::zeros(n, n);
    padded.columns_mut(0, k).copy_from(x);
    padded.qr().q().columns(k, n - k).into_owned()
}
fn cds_nullspace(constraint: &DMatrix<C>) -> DMatrix<C> {
    let n = constraint.ncols();
    let k = constraint.nrows();
    if k == 0 {
        return DMatrix::identity(n, n);
    }
    let mut padded = DMatrix::zeros(n, n);
    padded.columns_mut(0, k).copy_from(&constraint.adjoint());
    padded.qr().q().columns(k, n - k).into_owned()
}
fn cds_pair_update(s: &DMatrix<C>, q: &DMatrix<f64>) -> Result<DVector<C>> {
    let qc = q.map(|x| C::new(x, 0.));
    let projections = qc.adjoint() * s;
    let a = projections.row(0);
    let b = projections.row(1);
    // Im(conj(a*w)*(b*w)) = w^H M w. The largest magnitude
    // eigenvalue gives the unit vector maximizing the real pair's area.
    let h = (a.adjoint() * b - b.adjoint() * a) * C::new(0., -0.5);
    let eig = nalgebra::linalg::SymmetricEigen::try_new(h, f64::EPSILON, 128 * s.ncols() + 1)
        .ok_or("place: conjugate-pair update did not converge within its iteration bound")?;
    let idx = eig
        .eigenvalues
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.abs().total_cmp(&b.abs()))
        .map(|(i, _)| i)
        .unwrap_or(0);
    Ok(s * eig.eigenvectors.column(idx))
}
// Ackermann in orthogonal controllability coordinates (H, beta*e1).
// Two-pass orthogonal Arnoldi constructs the form documented by SLICOT AB01MD.
// For upper Hessenberg H, the last row of ctrb(H,beta*e1)^-1 is
// e_n'/(beta*product(H[i+1,i])). Evaluate the factored polynomial on
// that row, dividing one factor at a time; never form ctrb or powers of A.
fn cds_single_input(
    e: &Engine,
    a: &DMatrix<f64>,
    b: &DVector<f64>,
    p: &[C],
) -> Result<DMatrix<f64>> {
    let n = a.nrows();
    let beta = b.norm();
    let mut q = DMatrix::zeros(n, n);
    let mut h = DMatrix::zeros(n, n);
    q.set_column(0, &(b / beta));
    let tolerance = 1e-12 * n as f64 * a.norm().max(f64::MIN_POSITIVE);
    for j in 0..n {
        e.check_cancel()?;
        let mut v = a * q.column(j);
        let basis = q.columns(0, j + 1);
        // Reorthogonalize without ever constructing controllability powers.
        // Exact coordinate chains also retain their sparse integer structure.
        for _ in 0..2 {
            let projection = basis.transpose() * &v;
            v -= basis * &projection;
            let mut column = h.view_mut((0, j), (j + 1, 1));
            column += &projection;
        }
        if j + 1 < n {
            let norm = v.norm();
            if !norm.is_finite() || norm <= tolerance {
                return Err("place: singular orthogonal controllability reduction".into());
            }
            h[(j + 1, j)] = norm;
            q.set_column(j + 1, &(v / norm));
        }
    }
    let hc = h.map(|x| C::new(x, 0.));
    let mut row = DMatrix::zeros(1, n);
    row[(0, n - 1)] = C::new(1., 0.);
    for (j, &pole) in p.iter().enumerate() {
        e.check_cancel()?;
        let divisor = if j + 1 < n {
            h[(n - 1 - j, n - 2 - j)]
        } else {
            beta
        };
        if divisor == 0. || !divisor.is_finite() {
            return Err("place: singular orthogonal controllability reduction".into());
        }
        row = (&row * &hc - &row * pole) / C::new(divisor, 0.);
    }
    if row.iter().any(|z| !z.re.is_finite() || !z.im.is_finite())
        || row.map(|z| z.im).norm() > 1e-9 * row.norm().max(f64::MIN_POSITIVE)
    {
        return Err("place: single-input gain calculation overflowed; rescale the problem".into());
    }
    Ok(row.map(|z| z.re) * q.transpose())
}

fn cds_place_check(
    e: &mut Engine,
    a: &Matrix,
    b: &Matrix,
    p: &[C],
    gain: &Matrix,
    out: &mut Execution,
) -> Result<()> {
    if gain.data().iter().any(|x| !x.is_finite()) {
        return Err("place: gain calculation overflowed; rescale the problem".into());
    }
    let actual = cds_eigen(e, &cds_sub(a, &b.mul(gain)?)?, out)?;
    if actual
        .iter()
        .any(|z| !z.re.is_finite() || !z.im.is_finite())
    {
        return Err("place: nonfinite closed-loop poles; rescale the problem".into());
    }
    let mut remaining = actual.as_slice().to_vec();
    for &z in p {
        let i = remaining
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| (**a - z).norm().total_cmp(&(**b - z).norm()))
            .map(|(i, _)| i)
            .ok_or("place: missing closed-loop pole")?;
        if (remaining.remove(i) - z).norm() > 1e-8 * (1. + z.norm()) {
            // Eigenvalue sensitivity alone must not reject a finite gain.
            e.lang_utils_call(
                "warning",
                &[
                    Value::Text("MatRust:place:PoleAccuracy".into()),
                    Value::Text("place: closed-loop pole error exceeds 1e-8*(1+abs(p)); sensitive poles or ill-conditioned data can limit accuracy. Verify the returned gain.".into()),
                ],
                out,
            )?;
            break;
        }
    }
    Ok(())
}

fn cds_place(
    e: &mut Engine,
    a: &Matrix,
    b: &Matrix,
    p: &[C],
    out: &mut Execution,
) -> Result<Matrix> {
    let n = cds_ab(a, b)?;
    let ad = cds_dm(a);
    let bd = cds_dm(b);
    let r = cds_rank(&bd)?;
    if r == 0 || !cds_reachable(e, &ad, &bd, r)? {
        return Err("place: system is uncontrollable at numerical rank tolerance".into());
    }
    for &z in p {
        if p.iter()
            .filter(|&&w| (w - z).norm() <= 1e-12 * (1. + z.norm()))
            .count()
            > r
        {
            return Err("place: pole multiplicity exceeds rank(B)".into());
        }
    }
    let svd = cds_svd(bd.clone(), true, true)?;
    if r == 1 {
        // Pick the strongest original input, preserving exact sparse SISO
        // data. With redundant columns, recover the minimum-norm input gain.
        let column = (0..bd.ncols())
            .max_by(|&i, &j| bd.column(i).norm().total_cmp(&bd.column(j).norm()))
            .unwrap_or(0);
        let input = bd.column(column).into_owned();
        let single = cds_single_input(e, &ad, &input, p)?;
        let gain = if bd.ncols() == 1 {
            single
        } else {
            svd.solve(&(input * single), 1e-12 * bd.norm())
                .map_err(|_| "place: input solve failed")?
        };
        let result = cds_mat(&gain)?;
        cds_place_check(e, a, b, p, &result, out)?;
        return Ok(result);
    }
    let u = svd
        .u
        .as_ref()
        .ok_or("place SVD failed")?
        .columns(0, r)
        .into_owned();
    let u1 = cds_complement(&u).transpose().map(|x| C::new(x, 0.));
    let ac = ad.map(|x| C::new(x, 0.));
    let mut spaces = Vec::new();
    let mut slots = Vec::new();
    let mut j = 0;
    while j < n {
        e.check_cancel()?;
        let constraint = &u1 * (&ac - DMatrix::identity(n, n) * p[j]);
        if constraint
            .iter()
            .any(|z| !z.re.is_finite() || !z.im.is_finite())
        {
            return Err("place: pole constraint overflowed; rescale the problem".into());
        }
        let s = cds_nullspace(&constraint);
        if s.iter().any(|z| !z.re.is_finite() || !z.im.is_finite()) {
            return Err(
                "place: admissible eigenspace calculation overflowed; rescale the problem".into(),
            );
        }
        slots.push((j, p[j].im != 0.));
        spaces.push(s);
        j += if p[j].im != 0. { 2 } else { 1 };
    }
    let mut best = None;
    let mut best_det = 0.;
    // Deterministic independent starts avoid dependence on an RNG or global
    // state. KNV0 real-column updates; conjugate blocks stay real throughout.
    for start in 0..4 {
        let mut x = DMatrix::<f64>::zeros(n, n);
        for (idx, &(j, pair)) in slots.iter().enumerate() {
            let s = &spaces[idx];
            let w = DVector::from_fn(r, |k, _| {
                C::new(
                    (((k + 1) * (j + 2) * (start + 1)) as f64 * 1.631).sin()
                        + (((k + 1).pow(2) + (j + 1) * (start + 1)) as f64 * 0.7548776662466927)
                            .cos(),
                    if pair {
                        (((k + 2) * (j + 3) * (start + 2)) as f64 * 0.5698402909980532).sin()
                    } else {
                        0.
                    },
                )
            });
            let v = s * w;
            let v = &v / C::new(v.norm(), 0.);
            for i in 0..n {
                x[(i, j)] = v[i].re;
                if pair {
                    x[(i, j + 1)] = v[i].im;
                }
            }
        }
        let mut previous = 0.;
        for sweep in 0..80 {
            e.check_cancel()?;
            for (idx, &(j, pair)) in slots.iter().enumerate() {
                e.check_cancel()?;
                let width = if pair { 2 } else { 1 };
                let others = DMatrix::from_fn(n, n - width, |i, k| {
                    x[(i, if k < j { k } else { k + width })]
                });
                let q = cds_complement(&others);
                let s = &spaces[idx];
                let v = if pair {
                    cds_pair_update(s, &q)?
                } else {
                    let qv = q.column(0).map(|x| C::new(x, 0.));
                    let v = s * (s.adjoint() * qv);
                    if v.norm() < 1e-14 {
                        continue;
                    }
                    let norm = v.norm();
                    v / C::new(norm, 0.)
                };
                for i in 0..n {
                    x[(i, j)] = v[i].re;
                    if pair {
                        x[(i, j + 1)] = v[i].im;
                    }
                }
            }
            let det = x.determinant().abs();
            if det > best_det {
                best_det = det;
                best = Some(x.clone());
            }
            if sweep > 1 && (det - previous).abs() < 1e-8 * det {
                break;
            }
            previous = det;
        }
    }
    let x = best.ok_or("place: failed to construct independent closed-loop eigenvectors")?;
    let mut lambda = DMatrix::zeros(n, n);
    for &(j, pair) in &slots {
        lambda[(j, j)] = p[j].re;
        if pair {
            lambda[(j + 1, j + 1)] = p[j].re;
            lambda[(j, j + 1)] = p[j].im;
            lambda[(j + 1, j)] = -p[j].im;
        }
    }
    let target = &x * lambda;
    let closed = x
        .transpose()
        .lu()
        .solve(&target.transpose())
        .ok_or("place: eigenvector matrix is singular")?
        .transpose();
    let k = svd
        .solve(&(&ad - closed), 1e-12 * bd.norm())
        .map_err(|_| "place: input solve failed")?;
    let result = cds_mat(&k)?;
    cds_place_check(e, a, b, p, &result, out)?;
    Ok(result)
}

fn cds_roots(e: &mut Engine, coefficients: &[f64], out: &mut Execution) -> Result<Vec<C>> {
    let p = Matrix::from_column_slice(1, coefficients.len(), coefficients)?;
    let v = e.call("roots", vec![Value::Matrix(p)], out)?;
    Ok(cds_complex(&v)?.as_slice().to_vec())
}
// Deterministic greedy nearest matching; branch j takes its closest unused
// root. At repeated poles the labels are inherently ambiguous.
fn cds_match(previous: &[C], mut roots: Vec<C>) -> Vec<C> {
    let mut result = Vec::with_capacity(roots.len());
    for &z in previous {
        let i = roots
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| (**a - z).norm().total_cmp(&(**b - z).norm()))
            .map(|(i, _)| i)
            .unwrap_or(0);
        result.push(roots.remove(i));
    }
    result
}
fn cds_locus_at(
    e: &mut Engine,
    num: &[f64],
    den: &[f64],
    gain: f64,
    out: &mut Execution,
) -> Result<Vec<C>> {
    e.check_cancel()?;
    let mut coefficients = den.to_vec();
    let offset = den.len() - num.len();
    for (a, &b) in coefficients[offset..].iter_mut().zip(num) {
        *a += gain * b;
    }
    if coefficients.iter().any(|x| !x.is_finite()) {
        return Err("rlocus: closed-loop coefficients overflowed".into());
    }
    if coefficients[0] == 0. {
        return Err(
            "rlocus: a gain cancels the leading coefficient; poles at infinity are not supported"
                .into(),
        );
    }
    let roots = cds_roots(e, &coefficients, out)?;
    if roots.len() + 1 != den.len() || roots.iter().any(|z| !z.re.is_finite() || !z.im.is_finite())
    {
        return Err("rlocus: closed-loop root computation failed".into());
    }
    Ok(roots)
}
fn cds_rlocus(
    e: &mut Engine,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if !(1..=2).contains(&args.len()) || count > 2 {
        return Err("rlocus requires sys[,k] and returns at most [r,k]".into());
    }
    let sys = cds_lti(e, &args[0], "rlocus")?;
    sys.require_siso("rlocus")?;
    let (num, den) = e.ctl_tf(&sys, out)?;
    if num.len() > den.len() || den.len() > MAX_STATES + 1 {
        return Err("rlocus requires a proper SISO model with at most 128 poles".into());
    }
    let degree = den.len() - 1;
    if count == 0
        && degree + usize::from(degree > 0) + usize::from(num.len() > 1) > crate::plot::MAX_SERIES
    {
        return Err("rlocus plot exceeds the graphics limit of 64 total branch/marker series; request numeric outputs instead".into());
    }
    let mut gains = if let Some(v) = args.get(1) {
        let k = cds_real(v)?;
        if k.is_empty()
            || (k.rows() != 1 && k.cols() != 1)
            || k.len() > MAX_GAINS
            || k.data().iter().any(|x| *x < 0.)
        {
            return Err(
                "rlocus gains must be a nonempty vector of at most 4096 finite nonnegative values"
                    .into(),
            );
        }
        k.data().to_vec()
    } else {
        let dn = den.data().iter().fold(0_f64, |s, x| s.max(x.abs()));
        let nn = num.data().iter().fold(0_f64, |s, x| s.max(x.abs()));
        let scale = if nn > 0. {
            (dn / nn).clamp(1e-100, 1e100)
        } else {
            1.
        };
        let mut k = vec![0.];
        k.extend((0..81).map(|i| scale * 10_f64.powf(-4. + i as f64 / 10.)));
        k
    };
    let mut roots = Vec::with_capacity(gains.len());
    for &k in &gains {
        roots.push(cds_locus_at(e, num.data(), den.data(), k, out)?);
    }
    if args.len() == 1 && degree > 0 {
        // Refine an interval if any matched branch moves >5% of 1+|r|,
        // or the midpoint deviates >1% from its straight chord. Bound both
        // the number of rounds (12) and total gains (4096).
        for _ in 0..12 {
            let mut next_gains = vec![gains[0]];
            let mut next_roots = vec![roots[0].clone()];
            let mut added = false;
            let mut capacity = MAX_GAINS - gains.len();
            for i in 1..gains.len() {
                e.check_cancel()?;
                let left = &roots[i - 1];
                let right = cds_match(left, roots[i].clone());
                let mid_gain = (gains[i - 1] + gains[i]) * 0.5;
                if capacity > 0 && mid_gain > gains[i - 1] && mid_gain < gains[i] {
                    let middle = cds_match(
                        left,
                        cds_locus_at(e, num.data(), den.data(), mid_gain, out)?,
                    );
                    let refine = left.iter().zip(&right).zip(&middle).any(|((&l, &r), &m)| {
                        (r - l).norm() > 0.05 * (1. + l.norm().min(r.norm()))
                            || (m - (l + r) * 0.5).norm() > 0.01 * (1. + m.norm())
                    });
                    if refine {
                        next_gains.push(mid_gain);
                        next_roots.push(middle);
                        capacity -= 1;
                        added = true;
                    }
                }
                next_gains.push(gains[i]);
                next_roots.push(roots[i].clone());
            }
            gains = next_gains;
            roots = next_roots;
            if !added || gains.len() == MAX_GAINS {
                break;
            }
        }
    }
    if let Some(first) = roots.first_mut() {
        first.sort_by(|a, b| a.re.total_cmp(&b.re).then(a.im.total_cmp(&b.im)));
    }
    for i in 1..roots.len() {
        roots[i] = cds_match(&roots[i - 1], roots[i].clone());
    }
    let values = DMatrix::from_fn(degree, gains.len(), |i, j| roots[j][i]);
    if count == 0 {
        let mut plot_args = Vec::new();
        for branch in 0..degree {
            let points = DMatrix::from_fn(gains.len(), 1, |i, _| values[(branch, i)]);
            plot_args.extend([
                cds_value(&points.map(|z| z.re))?,
                cds_value(&points.map(|z| z.im))?,
                Value::Text(if gains.len() == 1 { "." } else { "-" }.into()),
            ]);
        }
        for (coefficients, marker) in [(den.data(), "x"), (num.data(), "o")] {
            let z = cds_roots(e, coefficients, out)?;
            if !z.is_empty() {
                plot_args.push(Value::Matrix(Matrix::from_column_vec(
                    z.len(),
                    1,
                    z.iter().map(|z| z.re).collect(),
                )?));
                plot_args.push(Value::Matrix(Matrix::from_column_vec(
                    z.len(),
                    1,
                    z.iter().map(|z| z.im).collect(),
                )?));
                plot_args.push(Value::Text(marker.into()));
            }
        }
        if degree == 0 {
            return Err("rlocus: a static gain has no branches to plot".into());
        }
        e.call("plot", plot_args, out)?;
        for (name, text) in [
            ("title", "Root locus"),
            ("xlabel", "Real axis"),
            ("ylabel", "Imaginary axis"),
        ] {
            e.call(name, vec![Value::Text(text.into())], out)?;
        }
        return Ok(Vec::new());
    }
    Ok(vec![
        cds_cvalue(e, &values)?,
        Value::Matrix(Matrix::from_column_slice(1, gains.len(), &gains)?),
    ])
}

// Lyapunov, Sylvester and Stein equations by Bartels-Stewart: Bartels &
// Stewart, "Solution of the matrix equation AX + XB = C", CACM 15 (1972);
// discrete form after Barraud, IEEE TAC 22 (1977). The real Schur forms of
// A and B (existing `schur`) are made triangular with `rsf2csf`; the
// transformed equation is solved by complex back-substitution, O(m^3 + n^3)
// work and O(m n) storage. No Kronecker matrix and no inverse is formed.
const MAX_LYAP: usize = 512;
fn cds_lyap_square(m: &Matrix, name: &str, what: &str) -> Result<usize> {
    if m.rows() != m.cols() || m.rows() == 0 {
        return Err(format!("{name}: {what} must be a nonempty square matrix"));
    }
    if m.rows() > MAX_LYAP {
        return Err(format!("{name}: {what} is limited to {MAX_LYAP} rows"));
    }
    Ok(m.rows())
}
fn cds_complex_schur(
    e: &mut Engine,
    a: &Matrix,
    out: &mut Execution,
) -> Result<(DMatrix<C>, DMatrix<C>)> {
    let real = e
        .lax_outputs("schur", &[Value::Matrix(a.clone())], 2, out)?
        .ok_or("schur unavailable")?;
    let complex = e
        .lax_outputs("rsf2csf", &real, 2, out)?
        .ok_or("rsf2csf unavailable")?;
    Ok((cds_complex(&complex[0])?, cds_complex(&complex[1])?))
}
/// Solves A X + X B + C = 0 (continuous) or A X B - X + C = 0 (discrete).
fn cds_lyap_solve(
    e: &mut Engine,
    name: &str,
    a: &Matrix,
    b: &Matrix,
    c: &Matrix,
    discrete: bool,
    out: &mut Execution,
) -> Result<Matrix> {
    let (m, n) = (a.rows(), b.rows());
    let (u, r) = cds_complex_schur(e, a, out)?;
    e.check_cancel()?;
    let (v, s) = cds_complex_schur(e, b, out)?;
    let scale = cds_norm(a) + cds_norm(b);
    let mut y = u.adjoint() * cds_dm(c).map(|x| C::new(x, 0.)) * &v;
    let singular = |what: &str| {
        format!(
            "{name}: the equation has no unique solution because {what} for an eigenvalue \
             pair of A and B{}",
            if discrete {
                " (with dlyap(A,Q), B = A')"
            } else {
                " (with lyap(A,Q), B = A')"
            }
        )
    };
    let mut w = vec![C::new(0., 0.); m];
    for j in 0..n {
        e.check_cancel()?;
        if discrete {
            // w = (Y S)(:,j) without the unknown diagonal term S(j,j).
            for (k, wk) in w.iter_mut().enumerate() {
                *wk = (0..j).map(|l| y[(k, l)] * s[(l, j)]).sum();
            }
        }
        for i in (0..m).rev() {
            let (rhs, den) = if discrete {
                // sum_{k>=i} R(i,k) (w_k + Y(k,j) S(j,j)) - Y(i,j) = -C(i,j)
                let mut rhs = -y[(i, j)] - r[(i, i)] * w[i];
                for k in i + 1..m {
                    rhs -= r[(i, k)] * (w[k] + y[(k, j)] * s[(j, j)]);
                }
                let product = r[(i, i)] * s[(j, j)];
                let den = product - 1.;
                if den.norm() <= 1e2 * f64::EPSILON * product.norm().max(1.) {
                    return Err(singular("lambda_i(A) * lambda_j(B) = 1"));
                }
                (rhs, den)
            } else {
                let mut rhs = -y[(i, j)];
                for k in i + 1..m {
                    rhs -= r[(i, k)] * y[(k, j)];
                }
                for k in 0..j {
                    rhs -= y[(i, k)] * s[(k, j)];
                }
                let den = r[(i, i)] + s[(j, j)];
                if den.norm() <= 1e2 * f64::EPSILON * scale {
                    return Err(singular("lambda_i(A) + lambda_j(B) = 0"));
                }
                (rhs, den)
            };
            y[(i, j)] = rhs / den;
        }
    }
    let x = (u * y * v.adjoint()).map(|z| z.re);
    if x.iter().any(|v| !v.is_finite()) {
        return Err(format!(
            "{name}: the solution overflowed; the equation is too close to singular"
        ));
    }
    cds_mat(&x)
}
fn cds_lyap(e: &mut Engine, name: &str, args: &[Value], out: &mut Execution) -> Result<Value> {
    let discrete = name == "dlyap";
    if args.len() == 4 {
        return Err(format!(
            "{name}: the generalized form {name}(A,Q,[],E) is not supported yet"
        ));
    }
    if !(2..=3).contains(&args.len()) {
        return Err(format!("{name} requires (A,Q) or (A,B,C)"));
    }
    let a = cds_real(&args[0])?;
    let m = cds_lyap_square(&a, name, "A")?;
    let (b, c) = if args.len() == 2 {
        let q = cds_real(&args[1])?;
        if (q.rows(), q.cols()) != (m, m) {
            return Err(format!("{name}: Q must be {m} by {m}, the size of A"));
        }
        (a.transpose(), q)
    } else {
        let b = cds_real(&args[1])?;
        let n = cds_lyap_square(&b, name, "B")?;
        let c = cds_real(&args[2])?;
        if (c.rows(), c.cols()) != (m, n) {
            return Err(format!(
                "{name}: C must be {m} by {n} (rows of A by columns of B)"
            ));
        }
        (b, c)
    };
    Ok(Value::Matrix(cds_lyap_solve(
        e, name, &a, &b, &c, discrete, out,
    )?))
}

// pidtune: a documented loop-shaping rule (not MathWorks' algorithm).
// Target: |C(jwc) G(jwc)| = 1 and arg C(jwc) G(jwc) = -120 deg (60 deg phase
// margin at wc). Ki = Kp wc/10 fixes the integral phase lag at atan(0.1);
// Tf = 1/(10 wc) for PIDF. P and PI have one free gain, so their default wc
// is where arg G reaches the phase their fixed shape needs; PD, PID and PIDF
// solve Kp and Kd from the complex target (2x2 real system), default wc where
// arg G = -150 deg. Fallbacks: the gain crossover of G, then 1 rad/s.
fn cds_text(v: &Value) -> Result<String> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Ok(s.clone()),
        Value::Container(crate::Array::Character(c)) if c.rows() <= 1 => {
            Ok(String::from_utf16_lossy(c.units()))
        }
        _ => Err("Expected character-row text".into()),
    }
}
fn cds_polyval(c: &[f64], z: C) -> C {
    c.iter().fold(C::new(0., 0.), |acc, &x| acc * z + x)
}
fn cds_conv(a: &[f64], b: &[f64]) -> Vec<f64> {
    let mut c = vec![0.; a.len() + b.len() - 1];
    for (i, &x) in a.iter().enumerate() {
        for (j, &y) in b.iter().enumerate() {
            c[i + j] += x * y;
        }
    }
    c
}
struct CdsPlant {
    num: Vec<f64>,
    den: Vec<f64>,
    zeros: Vec<C>,
    poles: Vec<C>,
    negative: bool,
}
impl CdsPlant {
    fn at(&self, w: f64) -> C {
        let s = C::new(0., w);
        cds_polyval(&self.num, s) / cds_polyval(&self.den, s)
    }
    /// Continuous (unwrapped) phase in degrees, from root angles.
    fn phase(&self, w: f64) -> f64 {
        let s = C::new(0., w);
        let sum = |r: &[C]| r.iter().map(|z| (s - z).arg()).sum::<f64>();
        (sum(&self.zeros) - sum(&self.poles)).to_degrees() - if self.negative { 180. } else { 0. }
    }
    /// First frequency (ascending) where f changes sign, refined by bisection.
    fn crossing(&self, lo: f64, hi: f64, f: impl Fn(f64) -> f64) -> Option<f64> {
        const POINTS: usize = 4000;
        let (llo, lhi) = (lo.ln(), hi.ln());
        let at = |k: usize| (llo + (lhi - llo) * k as f64 / POINTS as f64).exp();
        let mut a = at(0);
        let mut fa = f(a);
        for k in 1..=POINTS {
            let b = at(k);
            let fb = f(b);
            if fa == 0. {
                return Some(a);
            }
            if fa.is_finite() && fb.is_finite() && (fa < 0.) != (fb < 0.) {
                let (mut x, mut y) = (a, b);
                for _ in 0..200 {
                    let m = (x * y).sqrt();
                    if m <= x || m >= y {
                        break;
                    }
                    if (f(m) < 0.) == (fa < 0.) {
                        x = m;
                    } else {
                        y = m;
                    }
                }
                return Some(if f(x).abs() <= f(y).abs() { x } else { y });
            }
            a = b;
            fa = fb;
        }
        None
    }
}
/// Target phase margin of a `pidtuneOptions` struct (degrees).
fn cds_pidtune_margin(v: &Value) -> Result<f64> {
    let pm = cds_real(v)
        .and_then(|m| m.as_scalar())
        .map_err(|_| "pidtuneOptions: PhaseMargin must be a real scalar in degrees".to_owned())?;
    if !(pm > 0. && pm < 180.) {
        return Err("pidtuneOptions: PhaseMargin must be between 0 and 180 degrees".into());
    }
    Ok(pm)
}
fn cds_design_focus(v: &Value) -> Result<String> {
    let focus = cds_text(v)
        .map_err(|_| "pidtuneOptions: DesignFocus must be text".to_owned())?
        .to_ascii_lowercase();
    if !matches!(
        focus.as_str(),
        "balanced" | "reference-tracking" | "disturbance-rejection"
    ) {
        return Err(
            "pidtuneOptions: DesignFocus must be 'balanced', 'reference-tracking' or \
             'disturbance-rejection'"
                .into(),
        );
    }
    Ok(focus)
}
/// `pidtuneOptions(Name,Value,...)`: a struct with PhaseMargin (degrees,
/// default 60) and DesignFocus (default 'balanced').
fn cds_pidtune_options(e: &Engine, args: &[Value]) -> Result<Value> {
    if !args.len().is_multiple_of(2) {
        return Err("pidtuneOptions: arguments must be name-value pairs".into());
    }
    let mut margin = 60.;
    let mut focus = "balanced".to_owned();
    for pair in args.chunks(2) {
        e.check_cancel()?;
        let name = cds_text(&pair[0])
            .map_err(|_| "pidtuneOptions: option names must be text".to_owned())?;
        match name.to_ascii_lowercase().as_str() {
            "phasemargin" => margin = cds_pidtune_margin(&pair[1])?,
            "designfocus" => focus = cds_design_focus(&pair[1])?,
            _ => {
                return Err(format!(
                    "pidtuneOptions: unknown option '{name}'; accepted: PhaseMargin, DesignFocus"
                ));
            }
        }
    }
    let slots = vec![
        std::sync::Arc::new(crate::Array::Numeric(Matrix::scalar(margin))),
        std::sync::Arc::new(crate::CharacterRow::from_text(&focus, &e.cancel)?.into()),
    ];
    Ok(Value::Container(crate::Array::Struct(
        crate::StructArray::from_slots(
            1,
            1,
            vec!["PhaseMargin".into(), "DesignFocus".into()],
            slots,
            &e.cancel,
        )?,
    )))
}
/// Phase margin of options from `pidtuneOptions` (or any 1x1 struct with
/// those fields). DesignFocus is validated but does not change this rule:
/// the published options only weight tracking against rejection, and the
/// fixed-ratio integral rule here has no such trade-off to make.
fn cds_pidtune_opts(v: &Value) -> Result<f64> {
    let Value::Container(crate::Array::Struct(s)) = v else {
        return Err("pidtune: options must be a pidtuneOptions struct".into());
    };
    if s.len() != 1 {
        return Err("pidtune: options must be a 1x1 pidtuneOptions struct".into());
    }
    let field = |name: &str| -> Result<Option<Value>> {
        let Some(index) = s.field_index(name) else {
            return Ok(None);
        };
        Ok(Some(match s.get(index, 0)?.as_ref() {
            crate::Array::Numeric(m) => Value::Matrix(m.clone()),
            other => Value::Container(other.clone()),
        }))
    };
    let focus = field("DesignFocus")?;
    let margin = field("PhaseMargin")?;
    if focus.is_none() && margin.is_none() {
        return Err("pidtune: options must be a pidtuneOptions struct".into());
    }
    if let Some(focus) = focus {
        cds_design_focus(&focus).map_err(|e| e.replacen("pidtuneOptions", "pidtune", 1))?;
    }
    match margin {
        Some(v) => cds_pidtune_margin(&v).map_err(|e| e.replacen("pidtuneOptions", "pidtune", 1)),
        None => Ok(60.),
    }
}
fn cds_pidtune(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
    if !(2..=4).contains(&args.len()) {
        return Err(
            "pidtune requires (G,type), (G,type,wc), (G,type,opts) or (G,type,wc,opts)".into(),
        );
    }
    let g = cds_lti(e, &args[0], "pidtune")?;
    g.require_siso("pidtune")?;
    if g.ts != 0. {
        return Err("pidtune: discrete-time plants are not supported yet".into());
    }
    let kind = cds_text(&args[1])?.to_ascii_uppercase();
    if !matches!(
        kind.as_str(),
        "P" | "I" | "PI" | "PD" | "PDF" | "PID" | "PIDF"
    ) {
        return Err("pidtune: type must be 'P', 'I', 'PI', 'PD', 'PDF', 'PID' or 'PIDF'".into());
    }
    // pidtune(G,type,opts) and pidtune(G,type,wc,opts); wc = [] keeps the default.
    let is_options = |v: &Value| matches!(v, Value::Container(crate::Array::Struct(_)));
    let (wc_arg, options) = match &args[2..] {
        [] => (None, None),
        [v] if is_options(v) => (None, Some(v)),
        [w] => (Some(w), None),
        [w, o] => (Some(w), Some(o)),
        _ => unreachable!("argument count checked above"),
    };
    let requested = match wc_arg {
        Some(Value::Matrix(m)) if m.is_empty() => None,
        Some(v) => {
            let w = cds_real(v)?.as_scalar()?;
            if !(w > 0. && w.is_finite()) {
                return Err("pidtune: wc must be a positive finite frequency in rad/s".into());
            }
            Some(w)
        }
        None => None,
    };
    let target_margin = match options {
        Some(v) => cds_pidtune_opts(v)?,
        None => 60.,
    };
    // Loop phase at crossover that leaves the target phase margin.
    let loop_phase = target_margin - 180.;
    let (num, den) = e.ctl_tf(&g, out)?;
    let (num, den) = (num.data().to_vec(), den.data().to_vec());
    if num.iter().all(|x| *x == 0.) {
        return Err("pidtune: the plant is identically zero".into());
    }
    let zeros = if num.len() > 1 {
        cds_roots(e, &num, out)?
    } else {
        vec![]
    };
    let poles = if den.len() > 1 {
        cds_roots(e, &den, out)?
    } else {
        vec![]
    };
    let plant = CdsPlant {
        negative: num[0] / den[0] < 0.,
        num,
        den,
        zeros,
        poles,
    };
    // Search band around the plant's own time scales.
    let scales: Vec<f64> = plant
        .zeros
        .iter()
        .chain(&plant.poles)
        .map(|z| z.norm())
        .filter(|x| *x > 0. && x.is_finite())
        .collect();
    let (lo, hi) = if scales.is_empty() {
        (1e-3, 1e3)
    } else {
        let min = scales.iter().copied().fold(f64::INFINITY, f64::min);
        let max = scales.iter().copied().fold(0., f64::max);
        ((min * 1e-3).max(1e-8), (max * 1e3).min(1e8))
    };
    let integral = kind.contains('I');
    let derivative = kind.contains('D');
    // Ki/(j wc) with Ki = Kp wc/10.
    let kp_basis = if integral {
        C::new(1., -0.1)
    } else {
        C::new(1., 0.)
    };
    // Phase the controller adds at the default crossover: none (P), -90 (I),
    // the fixed integral lag (PI) or a 30 degree lead (derivative types).
    let controller_phase = match kind.as_str() {
        "I" => -90.,
        _ if derivative => 30.,
        _ => kp_basis.arg().to_degrees(),
    };
    let target_phase = loop_phase - controller_phase;
    let wc = requested
        .or_else(|| plant.crossing(lo, hi, |w| plant.phase(w) - target_phase))
        .or_else(|| plant.crossing(lo, hi, |w| plant.at(w).norm().ln()))
        .unwrap_or(1.);
    e.check_cancel()?;
    let gw = plant.at(wc);
    if !(gw.norm() > 0. && gw.norm().is_finite()) {
        return Err(format!(
            "pidtune: the plant has no finite nonzero response at wc = {wc}"
        ));
    }
    // Controller value that gives |CG| = 1 and the target loop phase.
    let target = C::from_polar(1., loop_phase.to_radians()) / gw;
    let tf = if kind.ends_with('F') {
        1. / (10. * wc)
    } else {
        0.
    };
    let (kp, ki, kd) = if kind == "I" {
        // One gain: |Ki/(j wc) G| = 1.
        (0., wc / gw.norm(), 0.)
    } else if derivative {
        let kd_basis = C::new(0., wc) / C::new(1., wc * tf);
        let det = kp_basis.re * kd_basis.im - kd_basis.re * kp_basis.im;
        let kp = (target.re * kd_basis.im - kd_basis.re * target.im) / det;
        let kd = (kp_basis.re * target.im - target.re * kp_basis.im) / det;
        if kp < 0. || kd < 0. {
            return Err(format!(
                "pidtune: a {kind} controller cannot give a {target_margin} degree phase margin at \
                 wc = {wc} with nonnegative gains (Kp = {kp}, Kd = {kd}); try another type or wc"
            ));
        }
        (kp, if integral { kp * wc / 10. } else { 0. }, kd)
    } else if kind == "PI" && requested.is_some() && target.re > 0. && target.im <= 0. {
        // A given wc: Kp and Ki both free, Kp - j Ki/wc = target, when the
        // needed phase lies within the PI range (-90..0 degrees).
        (target.re, -wc * target.im, 0.)
    } else {
        // P, and PI with Ki = Kp wc/10: one gain, |C G| = 1 at wc.
        let kp = 1. / (gw.norm() * kp_basis.norm());
        (kp, if integral { kp * wc / 10. } else { 0. }, 0.)
    };
    let controller = control_lti::pid_value(e, [kp, ki, kd, tf], 0.)?;
    let c = cds_lti(e, &controller, "pidtune")?;
    let (cn, cd) = e.ctl_tf(&c, out)?;
    let loop_value =
        cds_polyval(cn.data(), C::new(0., wc)) / cds_polyval(cd.data(), C::new(0., wc)) * gw;
    let margin = 180. + loop_value.arg().to_degrees();
    // Closed loop of feedback(C*G, 1): den_C den_G + num_C num_G.
    let left = cds_conv(cd.data(), &plant.den);
    let right = cds_conv(cn.data(), &plant.num);
    let size = left.len().max(right.len());
    let mut characteristic = vec![0.; size];
    for (i, x) in left.iter().enumerate() {
        characteristic[size - left.len() + i] += x;
    }
    for (i, x) in right.iter().enumerate() {
        characteristic[size - right.len() + i] += x;
    }
    let first = characteristic
        .iter()
        .position(|x| *x != 0.)
        .ok_or("pidtune: the closed loop is degenerate")?;
    let characteristic = &characteristic[first..];
    let closed = if characteristic.len() > 1 {
        cds_roots(e, characteristic, out)?
    } else {
        vec![]
    };
    if let Some(worst) = closed
        .iter()
        .map(|p| p.re)
        .filter(|re| *re >= 0. || !re.is_finite())
        .reduce(f64::max)
    {
        return Err(format!(
            "pidtune: the tuned {kind} loop is unstable (closed-loop pole with real part {worst}); \
             this rule cannot stabilise the plant, try another type or crossover frequency"
        ));
    }
    let names = ["Stable", "CrossoverFrequency", "PhaseMargin"];
    let slots = vec![
        std::sync::Arc::new(Value::Logical(crate::LogicalMatrix::scalar(true)).array(&e.cancel)?),
        std::sync::Arc::new(crate::Array::Numeric(Matrix::scalar(wc))),
        std::sync::Arc::new(crate::Array::Numeric(Matrix::scalar(margin))),
    ];
    let info = Value::Container(crate::Array::Struct(crate::StructArray::from_slots(
        1,
        1,
        names.iter().map(|s| (*s).into()).collect(),
        slots,
        &e.cancel,
    )?));
    Ok(vec![controller, info])
}

pub(super) fn cds_call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    e.check_cancel()?;
    let max = match name {
        "care" | "dare" | "lqr" | "dlqr" => 3,
        "rlocus" | "pidtune" => 2,
        _ => 1,
    };
    if count > max {
        return Err(format!("{name}: too many output arguments"));
    }
    let mut results = match name {
        "ctrb" | "obsv" => {
            let (a, b) = if args.len() == 1 {
                let s = cds_lti(e, &args[0], name)?;
                let (a, b, c, _) = e.ctl_ss(&s, out)?;
                (a, if name == "ctrb" { b } else { c })
            } else if args.len() == 2 {
                (cds_real(&args[0])?, cds_real(&args[1])?)
            } else {
                return Err(format!("{name} requires sys or two matrices"));
            };
            let m = if name == "ctrb" {
                cds_ctrb(e, &a, &b)?
            } else {
                let n = cds_square(&a)?;
                if b.cols() != n || b.rows() == 0 || b.rows() > MAX_STATES {
                    return Err("C must have n columns and 1..128 output rows".into());
                }
                cds_ctrb(e, &a.transpose(), &b.transpose())?.transpose()
            };
            vec![Value::Matrix(m)]
        }
        "care" | "dare" => cds_riccati(e, args, name == "dare", out)?,
        "lqr" | "dlqr" => {
            let (a, b, tail, discrete) = if args.first().is_some_and(control_lti::is_lti) {
                if name == "dlqr" {
                    return Err(
                        "dlqr requires A,B,Q,R[,N]; use lqr(sys,Q,R[,N]) for an LTI model".into(),
                    );
                }
                if !(3..=4).contains(&args.len()) {
                    return Err("lqr(sys,Q,R[,N]) requires 3 or 4 inputs".into());
                }
                let s = cds_lti(e, &args[0], name)?;
                let (a, b, _, _) = e.ctl_ss(&s, out)?;
                (a, b, &args[1..], s.ts != 0.)
            } else {
                if !(4..=5).contains(&args.len()) {
                    return Err("lqr/dlqr require A,B,Q,R[,N]".into());
                }
                (
                    cds_real(&args[0])?,
                    cds_real(&args[1])?,
                    &args[2..],
                    name == "dlqr",
                )
            };
            let mut inputs = vec![Value::Matrix(a), Value::Matrix(b)];
            inputs.extend_from_slice(tail);
            let mut r = cds_riccati(e, &inputs, discrete, out)?;
            r.swap(0, 1);
            r
        }
        "place" | "acker" => {
            if args.len() != 3 {
                return Err(format!("{name} requires A,B,p"));
            }
            let a = cds_real(&args[0])?;
            let b = cds_real(&args[1])?;
            let n = cds_ab(&a, &b)?;
            let p = cds_poles(&args[2], n)?;
            let k = if name == "acker" {
                cds_acker(e, &a, &b, &p)?
            } else {
                cds_place(e, &a, &b, &p, out)?
            };
            vec![Value::Matrix(k)]
        }
        "rlocus" => cds_rlocus(e, args, count, out)?,
        "lyap" | "dlyap" => vec![cds_lyap(e, name, args, out)?],
        "pidtune" => cds_pidtune(e, args, out)?,
        "pidtuneOptions" => vec![cds_pidtune_options(e, args)?],
        _ => return Err("Unknown control design function".into()),
    };
    results.truncate(count.max(1));
    e.charge_function_outputs(&results)?;
    e.check_cancel()?;
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newton_corrections_match_scipy_lyapunov_solutions() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/control/design.json")).unwrap();
        let matrix = |v: &serde_json::Value| {
            Matrix::from_column_vec(
                v["rows"].as_u64().unwrap() as usize,
                v["cols"].as_u64().unwrap() as usize,
                v["real"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| x.as_f64().unwrap())
                    .collect(),
            )
            .unwrap()
        };
        for case in fixture["corrections"].as_array().unwrap() {
            let mut e = Engine::default();
            let correction = cds_riccati_correction(
                &mut e,
                &matrix(&case["closed"]),
                &matrix(&case["residual"]),
                case["discrete"].as_bool().unwrap(),
                &mut Execution::default(),
            )
            .unwrap();
            let expected = matrix(&case["expected"]);
            assert!(cds_norm(&cds_sub(&correction, &expected).unwrap()) < 1e-12);
        }
    }
}
