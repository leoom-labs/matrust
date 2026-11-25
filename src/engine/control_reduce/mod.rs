//! Model order reduction on state-space realizations, independent of the UI.
//!
//! Gramians: lyap/dlyap (Bartels-Stewart, see control_design.rs).
//! Hankel singular values and square-root balancing: Moore, "Principal
//! component analysis in linear systems", IEEE TAC 26 (1981); Laub, Heath,
//! Paige & Ward, "Computation of system balancing transformations", IEEE TAC
//! 32 (1987) (sigma = svd(Lo' Lc), T = S^-1/2 U' Lo', T^-1 = Lc V S^-1/2).
//! Singular perturbation (MatchDC): Liu & Anderson, "Singular perturbation
//! approximation of balanced systems", IJC 50 (1989); discrete form with
//! (I - A22) in place of -A22. Unstable modes are split off with an ordered
//! real Schur form and one Sylvester solve (additive decomposition
//! G = Gs + Gu); they are kept unchanged and reported as Inf.
//! Error bound used in the tests: Glover, IJC 39 (1984): for the stable part
//! ||G - Gr||_inf <= 2 * sum of the discarded Hankel singular values.
use super::{
    Engine, Execution, Value,
    control_lti::{self, Lti, Model},
};
use crate::{Complex64 as C, LogicalMatrix, Matrix, Result};
use nalgebra::DMatrix;

mod filtord;

const MRD_MAX_STATES: usize = 512;
const MRD_DEFAULT_OFFSET: f64 = 1e-8;
/// Real n-by-n blocks in one Lyapunov solve (a complex n-by-n counts as 2):
/// both complex Schur pairs (8), the complex right-hand side (2), the real
/// solution (1) and the A and Q copies passed into the solver (2).
const MRD_LYAP_BLOCKS: usize = 13;
/// T and T^-1 from the stable/unstable split stay live for that solve.
const MRD_SPLIT_BLOCKS: usize = 2;

pub(super) fn handles(name: &str) -> bool {
    matches!(
        name,
        "gram" | "hsvd" | "balreal" | "balred" | "balredOptions" | "modred" | "filtord"
    )
}

/// `blocks` dense n-by-n double matrices, checked before any of them is allocated.
fn mrd_admit(e: &Engine, n: usize, blocks: usize) -> Result<()> {
    if n == 0 || blocks == 0 {
        return Ok(());
    }
    let elements = n
        .checked_mul(n)
        .and_then(|square| square.checked_mul(blocks))
        .ok_or_else(|| "model reduction scratch size overflow".to_string())?;
    mrd_admit_elements(e, elements)
}

fn mrd_admit_elements(e: &Engine, elements: usize) -> Result<()> {
    if elements > crate::matrix::MAX_ELEMENTS {
        return Err(format!(
            "Matrix exceeds the {}-element limit",
            crate::matrix::MAX_ELEMENTS
        ));
    }
    e.preflight_elements(elements)
}

fn mrd_add(total: &mut usize, rows: usize, cols: usize) -> Result<()> {
    let part = rows
        .checked_mul(cols)
        .ok_or_else(|| "model reduction scratch size overflow".to_string())?;
    *total = total
        .checked_add(part)
        .ok_or_else(|| "model reduction scratch size overflow".to_string())?;
    Ok(())
}

/// Gramians plus the split that is still held while they are solved.
fn mrd_admit_grams(e: &Engine, n: usize, both: bool) -> Result<()> {
    mrd_admit(e, n, MRD_LYAP_BLOCKS + MRD_SPLIT_BLOCKS + usize::from(both))
}

/// A real state-space realization (A, B, C, D) with sampling time ts.
#[derive(Clone)]
struct MrdSys {
    a: DMatrix<f64>,
    b: DMatrix<f64>,
    c: DMatrix<f64>,
    d: DMatrix<f64>,
    ts: f64,
}

fn mrd_dm(m: &Matrix) -> DMatrix<f64> {
    DMatrix::from_column_slice(m.rows(), m.cols(), m.data())
}
fn mrd_mat(m: &DMatrix<f64>) -> Result<Matrix> {
    Matrix::from_column_slice(m.nrows(), m.ncols(), m.as_slice())
}
fn mrd_text(v: &Value) -> Option<String> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
        Value::Container(crate::Array::Character(c)) if c.rows() <= 1 => {
            Some(String::from_utf16_lossy(c.units()))
        }
        _ => None,
    }
}
fn mrd_scalar(v: &Value, what: &str) -> Result<f64> {
    match v {
        Value::Matrix(m) if m.len() == 1 && m.data()[0].is_finite() => Ok(m.data()[0]),
        _ => Err(format!("{what} must be a finite real scalar")),
    }
}

fn mrd_load(e: &mut Engine, name: &str, v: &Value, out: &mut Execution) -> Result<(Lti, MrdSys)> {
    let lti = match v {
        Value::Container(a) => control_lti::decode(e, a)?,
        _ => None,
    }
    .ok_or_else(|| format!("{name}: the first input must be a tf, zpk or ss model"))?;
    lti.require_delay_free(name)?;
    let (a, b, c, d) = e.ctl_ss(&lti, out)?;
    if a.rows() > MRD_MAX_STATES {
        return Err(format!(
            "{name}: models are limited to {MRD_MAX_STATES} states"
        ));
    }
    let ts = lti.ts;
    let sys = MrdSys {
        a: mrd_dm(&a),
        b: mrd_dm(&b),
        c: mrd_dm(&c),
        d: mrd_dm(&d),
        ts,
    };
    Ok((lti, sys))
}

/// Encodes a realization as an ss model keeping names and sampling time.
fn mrd_encode(e: &Engine, template: &Lti, s: &MrdSys) -> Result<Value> {
    let mut r = template.clone();
    r.model = Model::Ss(
        mrd_mat(&s.a)?,
        mrd_mat(&s.b)?,
        mrd_mat(&s.c)?,
        mrd_mat(&s.d)?,
    );
    control_lti::encode(e, &r)
}

fn mrd_stable(z: C, discrete: bool, offset: f64) -> bool {
    if discrete {
        z.norm() < 1. - offset
    } else {
        z.re < -offset * z.im.abs().max(1.)
    }
}

fn mrd_values(v: &Value) -> Result<Vec<C>> {
    match v {
        Value::Matrix(m) => Ok(m.data().iter().map(|&x| C::new(x, 0.)).collect()),
        Value::Complex(m) => Ok(m.data().to_vec()),
        _ => Err("Invalid eigenvalue result".into()),
    }
}

fn mrd_real(v: &Value) -> Result<DMatrix<f64>> {
    match v {
        Value::Matrix(m) => Ok(mrd_dm(m)),
        _ => Err("Expected a real matrix from an internal factorization".into()),
    }
}

/// Additive split x = q [xs; xu], [xs; xu] = qi x with A block diagonal.
struct MrdSplit {
    stable: MrdSys,
    unstable: MrdSys,
    q: DMatrix<f64>,
    qi: DMatrix<f64>,
}

fn mrd_split(e: &mut Engine, s: &MrdSys, offset: f64, out: &mut Execution) -> Result<MrdSplit> {
    let n = s.a.nrows();
    let (p, m) = (s.c.nrows(), s.b.ncols());
    let discrete = s.ts != 0.;
    let all_stable = |q: DMatrix<f64>| MrdSplit {
        stable: s.clone(),
        unstable: MrdSys {
            a: DMatrix::zeros(0, 0),
            b: DMatrix::zeros(0, m),
            c: DMatrix::zeros(p, 0),
            d: DMatrix::zeros(p, m),
            ts: s.ts,
        },
        qi: q.clone(),
        q,
    };
    if n == 0 {
        return Ok(all_stable(DMatrix::identity(0, 0)));
    }
    let factors = e
        .lax_outputs("schur", &[Value::Matrix(mrd_mat(&s.a)?)], 2, out)?
        .ok_or("schur unavailable")?;
    let eigs = mrd_values(
        &e.eigx_outputs("ordeig", &[factors[1].clone()], 1, out)?
            .remove(0),
    )?;
    let select: Vec<u8> = eigs
        .iter()
        .map(|&z| u8::from(mrd_stable(z, discrete, offset)))
        .collect();
    let k = select.iter().filter(|&&x| x == 1).count();
    if k == n {
        return Ok(all_stable(DMatrix::identity(n, n)));
    }
    let (u, t) = if k == 0 {
        (mrd_real(&factors[0])?, mrd_real(&factors[1])?)
    } else {
        let mut args = factors;
        args.push(Value::Logical(LogicalMatrix::from_column_vec(
            n, 1, select, &e.cancel,
        )?));
        let r = e.eigx_outputs("ordschur", &args, 2, out)?;
        (mrd_real(&r[0])?, mrd_real(&r[1])?)
    };
    e.check_cancel()?;
    let nu = n - k;
    let t11 = t.view((0, 0), (k, k)).into_owned();
    let t12 = t.view((0, k), (k, nu)).into_owned();
    let t22 = t.view((k, k), (nu, nu)).into_owned();
    // T11 X - X T22 = -T12 decouples the blocks: S = [I X; 0 I].
    let x = if k == 0 {
        DMatrix::zeros(0, nu)
    } else {
        let r = e
            .lax_outputs(
                "sylvester",
                &[
                    Value::Matrix(mrd_mat(&t11)?),
                    Value::Matrix(mrd_mat(&(-&t22))?),
                    Value::Matrix(mrd_mat(&(-&t12))?),
                ],
                1,
                out,
            )?
            .ok_or("sylvester unavailable")?;
        mrd_real(&r[0])?
    };
    let mut sinv = DMatrix::identity(n, n);
    let mut smat = DMatrix::identity(n, n);
    sinv.view_mut((0, k), (k, nu)).copy_from(&(-&x));
    smat.view_mut((0, k), (k, nu)).copy_from(&x);
    let qi = &sinv * u.transpose();
    let q = &u * &smat;
    let bt = &qi * &s.b;
    let ct = &s.c * &q;
    Ok(MrdSplit {
        stable: MrdSys {
            a: t11,
            b: bt.rows(0, k).into_owned(),
            c: ct.columns(0, k).into_owned(),
            d: s.d.clone(),
            ts: s.ts,
        },
        unstable: MrdSys {
            a: t22,
            b: bt.rows(k, nu).into_owned(),
            c: ct.columns(k, nu).into_owned(),
            d: DMatrix::zeros(p, m),
            ts: s.ts,
        },
        q,
        qi,
    })
}

/// Controllability and observability gramians of a stable realization.
fn mrd_grams(
    e: &mut Engine,
    s: &MrdSys,
    which: (bool, bool),
    out: &mut Execution,
) -> Result<(DMatrix<f64>, DMatrix<f64>)> {
    let n = s.a.nrows();
    let solves = usize::from(which.0) + usize::from(which.1);
    // The second gramian is solved while the first result is still live.
    mrd_admit(e, n, MRD_LYAP_BLOCKS + usize::from(solves == 2))?;
    let solver = if s.ts != 0. { "dlyap" } else { "lyap" };
    let mut solve = |a: DMatrix<f64>, q: DMatrix<f64>| -> Result<DMatrix<f64>> {
        if n == 0 {
            return Ok(DMatrix::zeros(0, 0));
        }
        let r = super::control_design::cds_call(
            e,
            solver,
            &[Value::Matrix(mrd_mat(&a)?), Value::Matrix(mrd_mat(&q)?)],
            1,
            out,
        )?;
        let w = mrd_real(&r[0])?;
        Ok((&w + w.transpose()) * 0.5)
    };
    let wc = if which.0 {
        solve(s.a.clone(), &s.b * s.b.transpose())?
    } else {
        DMatrix::zeros(0, 0)
    };
    let wo = if which.1 {
        solve(s.a.transpose(), s.c.transpose() * &s.c)?
    } else {
        DMatrix::zeros(0, 0)
    };
    Ok((wc, wo))
}

/// L with W = L L' from the symmetric eigendecomposition (semidefinite safe).
fn mrd_factor(w: &DMatrix<f64>) -> DMatrix<f64> {
    if w.nrows() == 0 {
        return w.clone();
    }
    let eig = w.clone().symmetric_eigen();
    let mut l = eig.eigenvectors;
    for (j, &lambda) in eig.eigenvalues.iter().enumerate() {
        let scale = lambda.max(0.).sqrt();
        l.column_mut(j).scale_mut(scale);
    }
    l
}

/// Square-root balancing data of a stable realization.
struct MrdBalance {
    sigma: Vec<f64>,
    u: DMatrix<f64>,
    v: DMatrix<f64>,
    lc: DMatrix<f64>,
    lo: DMatrix<f64>,
}

fn mrd_balance(e: &mut Engine, s: &MrdSys, out: &mut Execution) -> Result<MrdBalance> {
    // Split transforms are still held by the caller. Factors and the SVD are
    // fewer blocks than the two Lyapunov solves, so this admits them too.
    mrd_admit_grams(e, s.a.nrows(), true)?;
    let (wc, wo) = mrd_grams(e, s, (true, true), out)?;
    let (lc, lo) = (mrd_factor(&wc), mrd_factor(&wo));
    let n = s.a.nrows();
    if n == 0 {
        return Ok(MrdBalance {
            sigma: vec![],
            u: DMatrix::zeros(0, 0),
            v: DMatrix::zeros(0, 0),
            lc,
            lo,
        });
    }
    e.check_cancel()?;
    let svd = (lo.transpose() * &lc).svd(true, true);
    let (u, vt) = (
        svd.u.ok_or("hsvd: SVD failed")?,
        svd.v_t.ok_or("hsvd: SVD failed")?,
    );
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&i, &j| svd.singular_values[j].total_cmp(&svd.singular_values[i]));
    let sigma = order.iter().map(|&i| svd.singular_values[i]).collect();
    let u = DMatrix::from_fn(n, n, |r, c| u[(r, order[c])]);
    let v = DMatrix::from_fn(n, n, |r, c| vt[(order[c], r)]);
    Ok(MrdBalance {
        sigma,
        u,
        v,
        lc,
        lo,
    })
}

impl MrdBalance {
    /// Number of Hankel singular values that are nonzero to working precision.
    fn rank(&self) -> usize {
        let n = self.sigma.len();
        let tol = self.sigma.first().copied().unwrap_or(0.) * n as f64 * f64::EPSILON;
        self.sigma.iter().filter(|&&x| x > tol).count()
    }
    /// The first r rows of T and columns of T^-1.
    fn transforms(&self, r: usize) -> (DMatrix<f64>, DMatrix<f64>) {
        let n = self.sigma.len();
        let mut t = self.u.columns(0, r).transpose() * self.lo.transpose();
        let mut ti = &self.lc * self.v.columns(0, r);
        for i in 0..r {
            let w = self.sigma[i].sqrt().recip();
            t.row_mut(i).scale_mut(w);
            ti.column_mut(i).scale_mut(w);
        }
        debug_assert!(t.ncols() == n);
        (t, ti)
    }
}

fn mrd_transform(s: &MrdSys, t: &DMatrix<f64>, ti: &DMatrix<f64>) -> MrdSys {
    MrdSys {
        a: t * &s.a * ti,
        b: t * &s.b,
        c: &s.c * ti,
        d: s.d.clone(),
        ts: s.ts,
    }
}

/// Removes the states not in `keep` (kept in their order) by truncation or
/// by matching the DC gain (singular perturbation / residualization).
fn mrd_admit_elim(
    e: &Engine,
    keep: usize,
    drop: usize,
    outputs: usize,
    inputs: usize,
    match_dc: bool,
) -> Result<()> {
    let mut total = 0usize;
    mrd_add(&mut total, keep, keep)?;
    mrd_add(&mut total, keep, inputs)?;
    mrd_add(&mut total, outputs, keep)?;
    mrd_add(&mut total, outputs, inputs)?;
    if match_dc && drop > 0 {
        mrd_add(&mut total, keep, drop)?;
        mrd_add(&mut total, drop, keep)?;
        mrd_add(&mut total, drop, drop)?;
        mrd_add(&mut total, drop, inputs)?;
        mrd_add(&mut total, outputs, drop)?;
        let rhs_cols = keep
            .checked_add(inputs)
            .ok_or_else(|| "model reduction scratch size overflow".to_string())?;
        mrd_add(&mut total, drop, rhs_cols)?;
        mrd_add(&mut total, drop, drop)?;
        mrd_add(&mut total, keep, keep)?;
        mrd_add(&mut total, keep, inputs)?;
        mrd_add(&mut total, outputs, keep)?;
        mrd_add(&mut total, outputs, inputs)?;
    }
    mrd_admit_elements(e, total)
}

fn mrd_eliminate(
    e: &Engine,
    name: &str,
    s: &MrdSys,
    keep: &[usize],
    match_dc: bool,
) -> Result<MrdSys> {
    let n = s.a.nrows();
    let drop: Vec<usize> = (0..n).filter(|i| !keep.contains(i)).collect();
    mrd_admit_elim(
        e,
        keep.len(),
        drop.len(),
        s.c.nrows(),
        s.b.ncols(),
        match_dc,
    )?;
    let pick = |m: &DMatrix<f64>, rows: &[usize], cols: &[usize]| {
        DMatrix::from_fn(rows.len(), cols.len(), |i, j| m[(rows[i], cols[j])])
    };
    let all_in: Vec<usize> = (0..s.b.ncols()).collect();
    let all_out: Vec<usize> = (0..s.c.nrows()).collect();
    let a11 = pick(&s.a, keep, keep);
    let b1 = pick(&s.b, keep, &all_in);
    let c1 = pick(&s.c, &all_out, keep);
    if !match_dc || drop.is_empty() {
        return Ok(MrdSys {
            a: a11,
            b: b1,
            c: c1,
            d: s.d.clone(),
            ts: s.ts,
        });
    }
    let a12 = pick(&s.a, keep, &drop);
    let a21 = pick(&s.a, &drop, keep);
    let a22 = pick(&s.a, &drop, &drop);
    let b2 = pick(&s.b, &drop, &all_in);
    let c2 = pick(&s.c, &all_out, &drop);
    // Steady state of the eliminated states: M x2 = A21 x1 + B2 u with
    // M = -A22 (continuous) or I - A22 (discrete).
    let mm = if s.ts != 0. {
        DMatrix::identity(drop.len(), drop.len()) - &a22
    } else {
        -&a22
    };
    let mut rhs = DMatrix::zeros(drop.len(), keep.len() + all_in.len());
    rhs.columns_mut(0, keep.len()).copy_from(&a21);
    rhs.columns_mut(keep.len(), all_in.len()).copy_from(&b2);
    let singular = || {
        format!(
            "{name}: cannot match the DC gain because the eliminated block {} is singular; \
             use 'Truncate'",
            if s.ts != 0. { "I-A22" } else { "A22" }
        )
    };
    let lu = mm.clone().lu();
    let x = lu.solve(&rhs).ok_or_else(singular)?;
    // Reject numerically singular blocks as well as exactly singular ones.
    let scale = mm.amax().max(f64::MIN_POSITIVE);
    let diag_min = (0..drop.len())
        .map(|i| lu.u()[(i, i)].abs())
        .fold(f64::INFINITY, f64::min);
    if !x.iter().all(|v| v.is_finite()) || diag_min <= scale * drop.len() as f64 * f64::EPSILON {
        return Err(singular());
    }
    let xa = x.columns(0, keep.len());
    let xb = x.columns(keep.len(), all_in.len());
    Ok(MrdSys {
        a: &a11 + &a12 * xa,
        b: &b1 + &a12 * xb,
        c: &c1 + &c2 * xa,
        d: &s.d + &c2 * xb,
        ts: s.ts,
    })
}

/// [A_u 0; 0 A_s] with unstable states first; D comes from the stable part.
fn mrd_join(u: &MrdSys, s: &MrdSys) -> MrdSys {
    let (nu, ns) = (u.a.nrows(), s.a.nrows());
    let n = nu + ns;
    let (p, m) = s.d.shape();
    let mut a = DMatrix::zeros(n, n);
    a.view_mut((0, 0), (nu, nu)).copy_from(&u.a);
    a.view_mut((nu, nu), (ns, ns)).copy_from(&s.a);
    let mut b = DMatrix::zeros(n, m);
    b.rows_mut(0, nu).copy_from(&u.b);
    b.rows_mut(nu, ns).copy_from(&s.b);
    let mut c = DMatrix::zeros(p, n);
    c.columns_mut(0, nu).copy_from(&u.c);
    c.columns_mut(nu, ns).copy_from(&s.c);
    MrdSys {
        a,
        b,
        c,
        d: &s.d + &u.d,
        ts: s.ts,
    }
}

fn mrd_column(v: &[f64]) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_slice(v.len(), 1, v)?))
}

fn mrd_struct(v: &Value) -> Option<&crate::StructArray> {
    match v {
        Value::Container(crate::Array::Struct(s)) => Some(s),
        _ => None,
    }
}

fn mrd_slot(array: &crate::Array) -> Value {
    match array {
        crate::Array::Numeric(m) => Value::Matrix(m.clone()),
        other => Value::Container(other.clone()),
    }
}

/// `balredOptions(Name,Value,...)`: a struct with StateProjection
/// ('MatchDC' or 'Truncate'). 'StateElimMethod' is the older name of the
/// same field. Other names are rejected.
fn mrd_balred_options(e: &Engine, args: &[Value]) -> Result<Value> {
    if !args.len().is_multiple_of(2) {
        return Err("balredOptions: arguments must be name-value pairs".into());
    }
    let mut match_dc = true;
    for pair in args.chunks(2) {
        e.check_cancel()?;
        let name = mrd_text(&pair[0])
            .ok_or_else(|| "balredOptions: option names must be text".to_string())?;
        match name.to_ascii_lowercase().as_str() {
            "stateprojection" | "stateelimmethod" => {
                match_dc = mrd_method("balredOptions", &pair[1])?;
            }
            _ => {
                return Err(format!(
                    "balredOptions: unknown option '{name}'; accepted: StateProjection (StateElimMethod)"
                ));
            }
        }
    }
    let text = if match_dc { "MatchDC" } else { "Truncate" };
    let slots = vec![std::sync::Arc::new(
        crate::CharacterRow::from_text(text, &e.cancel)?.into(),
    )];
    Ok(Value::Container(crate::Array::Struct(
        crate::StructArray::from_slots(1, 1, vec!["StateProjection".into()], slots, &e.cancel)?,
    )))
}

/// StateProjection of a 1x1 struct from `balredOptions`.
fn mrd_balred_struct(v: &Value) -> Result<bool> {
    let Some(s) = mrd_struct(v) else {
        return Err("balred: options must be a balredOptions struct".into());
    };
    if s.len() != 1 {
        return Err("balred: options must be a 1x1 balredOptions struct".into());
    }
    let mut match_dc = None;
    for (index, name) in s.field_names().iter().enumerate() {
        let value = mrd_slot(s.get(index, 0)?.as_ref());
        match name.to_ascii_lowercase().as_str() {
            "stateprojection" | "stateelimmethod" => {
                match_dc = Some(mrd_method("balred", &value)?);
            }
            _ => {
                return Err(format!(
                    "balred: unknown option '{name}'; accepted: StateProjection"
                ));
            }
        }
    }
    match_dc.ok_or_else(|| "balred: options must be a balredOptions struct".into())
}

/// Options tail of balred: one balredOptions struct, or the older name/value pairs.
fn mrd_balred_tail(args: &[Value]) -> Result<(f64, bool)> {
    if args.iter().any(|v| mrd_struct(v).is_some()) {
        if args.len() != 1 {
            return Err(
                "balred: pass one balredOptions value, or name-value pairs, not both".into(),
            );
        }
        return Ok((MRD_DEFAULT_OFFSET, mrd_balred_struct(&args[0])?));
    }
    mrd_options("balred", args, true)
}

/// Name-value options: returns (offset, match_dc).
fn mrd_options(name: &str, args: &[Value], allow_method: bool) -> Result<(f64, bool)> {
    if !args.len().is_multiple_of(2) {
        return Err(format!("{name}: options must be name-value pairs"));
    }
    let (mut offset, mut match_dc) = (MRD_DEFAULT_OFFSET, true);
    for pair in args.chunks(2) {
        let key = mrd_text(&pair[0]).ok_or_else(|| format!("{name}: option names must be text"))?;
        if key.eq_ignore_ascii_case("Offset") {
            offset = mrd_scalar(&pair[1], &format!("{name}: Offset"))?;
            if !(0. ..1.).contains(&offset) {
                return Err(format!("{name}: Offset must be in [0, 1)"));
            }
        } else if allow_method && key.eq_ignore_ascii_case("StateElimMethod") {
            match_dc = mrd_method(name, &pair[1])?;
        } else {
            return Err(format!(
                "{name}: unknown option '{key}'; valid options are {}",
                if allow_method {
                    "'StateElimMethod' and 'Offset'"
                } else {
                    "'Offset'"
                }
            ));
        }
    }
    Ok((offset, match_dc))
}

fn mrd_method(name: &str, v: &Value) -> Result<bool> {
    let m = mrd_text(v).ok_or_else(|| format!("{name}: the method must be text"))?;
    match m.to_ascii_lowercase().as_str() {
        "matchdc" | "mdc" => Ok(true),
        "truncate" | "del" => Ok(false),
        _ => Err(format!(
            "{name}: unknown state elimination method '{m}'; use 'MatchDC' or 'Truncate'"
        )),
    }
}

fn mrd_gram(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Value> {
    if args.len() < 2 {
        return Err("gram: not enough input arguments; use gram(sys,'c') or gram(sys,'o')".into());
    }
    if args.len() > 2 {
        return Err("gram: too many input arguments".into());
    }
    let (_, s) = mrd_load(e, "gram", &args[0], out)?;
    let kind = mrd_text(&args[1]).unwrap_or_default();
    let controllability = match kind.as_str() {
        "c" => true,
        "o" => false,
        "cf" | "of" => {
            return Err(
                "gram: the Cholesky factor types 'cf' and 'of' are not supported yet".into(),
            );
        }
        _ => return Err("gram: TYPE must be 'c' or 'o'".into()),
    };
    mrd_admit_grams(e, s.a.nrows(), false)?;
    let split = mrd_split(e, &s, 0., out)?;
    if split.unstable.a.nrows() > 0 {
        return Err(
            "gram: the model must be stable; gramians of unstable models are undefined".into(),
        );
    }
    let (wc, wo) = mrd_grams(e, &s, (controllability, !controllability), out)?;
    Ok(Value::Matrix(mrd_mat(if controllability {
        &wc
    } else {
        &wo
    })?))
}

fn mrd_hsvd(e: &mut Engine, args: &[Value], count: usize, out: &mut Execution) -> Result<Value> {
    if count > 1 {
        return Err("hsvd: the BALDATA output is not supported yet".into());
    }
    if args.is_empty() {
        return Err("hsvd: not enough input arguments".into());
    }
    let (offset, _) = mrd_options("hsvd", &args[1..], false)?;
    let (_, s) = mrd_load(e, "hsvd", &args[0], out)?;
    mrd_admit_grams(e, s.a.nrows(), true)?;
    let split = mrd_split(e, &s, offset, out)?;
    let bal = mrd_balance(e, &split.stable, out)?;
    let mut g = vec![f64::INFINITY; split.unstable.a.nrows()];
    g.extend(bal.sigma);
    mrd_column(&g)
}

fn mrd_balreal(
    e: &mut Engine,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if args.is_empty() {
        return Err("balreal: not enough input arguments".into());
    }
    let (offset, _) = mrd_options("balreal", &args[1..], false)?;
    let (lti, s) = mrd_load(e, "balreal", &args[0], out)?;
    mrd_admit_grams(e, s.a.nrows(), true)?;
    let split = mrd_split(e, &s, offset, out)?;
    let bal = mrd_balance(e, &split.stable, out)?;
    let ns = bal.sigma.len();
    if bal.rank() < ns {
        return Err(format!(
            "balreal: the stable part is not minimal ({} of {ns} Hankel singular values are \
             zero to working precision); apply minreal or use balred",
            ns - bal.rank()
        ));
    }
    let (tb, tib) = bal.transforms(ns);
    let balanced = mrd_transform(&split.stable, &tb, &tib);
    let sysb = mrd_join(&split.unstable, &balanced);
    let nu = split.unstable.a.nrows();
    let n = nu + ns;
    let mut g = vec![f64::INFINITY; nu];
    g.extend(&bal.sigma);
    let mut outputs = vec![mrd_encode(e, &lti, &sysb)?, mrd_column(&g)?];
    if count > 2 {
        // xb = [xu; Tb xs] with [xs; xu] = qi x.
        let mut t = DMatrix::zeros(n, n);
        t.rows_mut(0, nu).copy_from(&split.qi.rows(ns, nu));
        t.rows_mut(nu, ns).copy_from(&(&tb * split.qi.rows(0, ns)));
        let mut ti = DMatrix::zeros(n, n);
        ti.columns_mut(0, nu).copy_from(&split.q.columns(ns, nu));
        ti.columns_mut(nu, ns)
            .copy_from(&(split.q.columns(0, ns) * &tib));
        outputs.push(Value::Matrix(mrd_mat(&t)?));
        outputs.push(Value::Matrix(mrd_mat(&ti)?));
    }
    Ok(outputs)
}

fn mrd_balred(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Value> {
    if args.len() < 2 {
        return Err("balred: not enough input arguments; use balred(sys,r)".into());
    }
    let Value::Matrix(orders) = &args[1] else {
        return Err("balred: ORDERS must be a nonnegative integer".into());
    };
    if orders.len() != 1 {
        return Err(
            "balred: a vector of orders returns a model array, which is not supported yet; \
             call balred once per order"
                .into(),
        );
    }
    let r = orders.data()[0];
    if !(r >= 0. && r.fract() == 0.) {
        return Err("balred: ORDERS must be a nonnegative integer".into());
    }
    let (offset, match_dc) = mrd_balred_tail(&args[2..])?;
    let (lti, s) = mrd_load(e, "balred", &args[0], out)?;
    let n = s.a.nrows();
    if r > n as f64 {
        return Err(format!(
            "balred: ORDERS must not exceed the model order {n}"
        ));
    }
    let r = r as usize;
    mrd_admit_grams(e, n, true)?;
    let split = mrd_split(e, &s, offset, out)?;
    let nu = split.unstable.a.nrows();
    if r < nu {
        return Err(format!(
            "balred: order {r} is below the number of unstable modes ({nu}), which are always \
             kept"
        ));
    }
    let bal = mrd_balance(e, &split.stable, out)?;
    // States with zero Hankel singular value are removed exactly by
    // truncation; the remaining minimal balanced part is then reduced.
    let rank = bal.rank();
    let (tb, tib) = bal.transforms(rank);
    let balanced = mrd_transform(&split.stable, &tb, &tib);
    let keep: Vec<usize> = (0..(r - nu).min(rank)).collect();
    let reduced = mrd_eliminate(e, "balred", &balanced, &keep, match_dc)?;
    mrd_encode(e, &lti, &mrd_join(&split.unstable, &reduced))
}

fn mrd_modred(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Value> {
    if args.len() < 2 {
        return Err("modred: not enough input arguments; use modred(sys,elim)".into());
    }
    if args.len() > 3 {
        return Err("modred: too many input arguments".into());
    }
    let (lti, s) = mrd_load(e, "modred", &args[0], out)?;
    let n = s.a.nrows();
    let mut eliminated = vec![false; n];
    match &args[1] {
        Value::Logical(m) if m.len() == n && (m.rows() == 1 || m.cols() == 1 || n == 0) => {
            for (x, &b) in eliminated.iter_mut().zip(m.data()) {
                *x = b != 0;
            }
        }
        Value::Matrix(m) if m.rows() <= 1 || m.cols() <= 1 => {
            for &x in m.data() {
                if !(x >= 1. && x <= n as f64 && x.fract() == 0.) {
                    return Err(format!(
                        "modred: ELIM must contain state indices between 1 and {n}"
                    ));
                }
                eliminated[x as usize - 1] = true;
            }
        }
        _ => {
            return Err(format!(
                "modred: ELIM must be a vector of state indices or a logical vector of length {n}"
            ));
        }
    }
    let match_dc = match args.get(2) {
        Some(v) => mrd_method("modred", v)?,
        None => true,
    };
    let keep: Vec<usize> = (0..n).filter(|&i| !eliminated[i]).collect();
    let reduced = mrd_eliminate(e, "modred", &s, &keep, match_dc)?;
    mrd_encode(e, &lti, &reduced)
}

pub(super) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    e.check_cancel()?;
    let max = if name == "balreal" { 4 } else { 1 };
    if count > max && name != "hsvd" {
        return Err(format!("{name}: too many output arguments"));
    }
    let mut results = match name {
        "gram" => vec![mrd_gram(e, args, out)?],
        "hsvd" => vec![mrd_hsvd(e, args, count, out)?],
        "balreal" => mrd_balreal(e, args, count, out)?,
        "balred" => vec![mrd_balred(e, args, out)?],
        "balredOptions" => vec![mrd_balred_options(e, args)?],
        "modred" => vec![mrd_modred(e, args, out)?],
        "filtord" => vec![filtord::filtord(e, args, out)?],
        _ => return Err(format!("{name}: unknown model reduction function")),
    };
    results.truncate(count.max(1));
    e.charge_function_outputs(&results)?;
    e.check_cancel()?;
    Ok(results)
}
