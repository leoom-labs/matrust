//! The two function forms of the spline builtins: the pp form (mkpp:
//! breaks, coefs, pieces, order, dim) and the B-form (spmak: knots, coefs,
//! number, order, dim), their evaluation, derivative, integral and the
//! conversion pp -> B (de Boor-Fix dual functionals; de Boor, A Practical
//! Guide to Splines, rev. ed. 2001, ch. IX-X).
use super::super::{Engine, Value};
use super::MAX_ORDER;
use crate::{Array, CharacterRow, Matrix, Result, StructArray};
use std::sync::Arc;

/// Piecewise polynomial: row r = piece * dim + component of `coefs`
/// (row-major, `order` entries per row, highest power first in x - breaks[piece]).
#[derive(Clone, Debug)]
pub(super) struct Pp {
    pub breaks: Vec<f64>,
    pub coefs: Vec<f64>,
    pub order: usize,
    pub dim: usize,
}

/// B-spline: coefficient j of component c at `coefs[j * dim + c]`
/// (the dim-by-number column-major matrix of spmak).
#[derive(Clone, Debug)]
pub(super) struct Bsp {
    pub knots: Vec<f64>,
    pub coefs: Vec<f64>,
    pub order: usize,
    pub dim: usize,
}

pub(super) enum Form {
    Pp(Pp),
    B(Bsp),
}

impl Pp {
    pub fn pieces(&self) -> usize {
        self.breaks.len() - 1
    }
    fn row(&self, piece: usize, c: usize) -> &[f64] {
        let r = piece * self.dim + c;
        &self.coefs[r * self.order..(r + 1) * self.order]
    }
    pub fn piece_of(&self, x: f64) -> usize {
        self.breaks
            .partition_point(|&v| v <= x)
            .saturating_sub(1)
            .min(self.pieces() - 1)
    }
    /// Component `c` at `x`; the end pieces extend outside the breaks.
    pub fn eval(&self, x: f64, c: usize) -> f64 {
        if x.is_nan() {
            return f64::NAN;
        }
        let k = self.piece_of(x);
        let t = x - self.breaks[k];
        self.row(k, c).iter().fold(0., |acc, v| acc * t + v)
    }
    /// Derivative of order `d` (order drops by d, at least to 1).
    pub fn der(&self, d: usize) -> Pp {
        let k = self.order;
        let nk = k.saturating_sub(d).max(1);
        let rows = self.coefs.len() / k;
        let mut coefs = vec![0.; rows * nk];
        if d < k {
            for r in 0..rows {
                let src = &self.coefs[r * k..(r + 1) * k];
                // src[m] multiplies t^(k-1-m); its d-th derivative: falling factorial.
                for (m, out) in coefs[r * nk..(r + 1) * nk].iter_mut().enumerate() {
                    let power = k - 1 - m;
                    let f: f64 = (0..d).map(|q| (power - q) as f64).product();
                    *out = src[m] * f;
                }
            }
        }
        Pp {
            breaks: self.breaks.clone(),
            coefs,
            order: nk,
            dim: self.dim,
        }
    }
    /// Indefinite integral, continuous, equal to `value` at breaks[0].
    pub fn int(&self, value: &[f64]) -> Pp {
        let k = self.order;
        let nk = k + 1;
        let (l, d) = (self.pieces(), self.dim);
        let mut coefs = vec![0.; l * d * nk];
        for (c, &start) in value.iter().enumerate().take(d) {
            let mut acc = start;
            for p in 0..l {
                let r = p * d + c;
                let src = self.row(p, c);
                let dst = &mut coefs[r * nk..(r + 1) * nk];
                for (m, v) in src.iter().enumerate() {
                    dst[m] = v / (k - m) as f64;
                }
                dst[k] = acc;
                let h = self.breaks[p + 1] - self.breaks[p];
                acc = dst.iter().fold(0., |a, v| a * h + v);
            }
        }
        Pp {
            breaks: self.breaks.clone(),
            coefs,
            order: nk,
            dim: d,
        }
    }
    /// Taylor coefficients (power 0 first) of component `c` about `tau`,
    /// using the piece `piece`.
    fn taylor(&self, piece: usize, c: usize, tau: f64) -> Vec<f64> {
        let mut a: Vec<f64> = self.row(piece, c).to_vec();
        let s = tau - self.breaks[piece];
        let k = a.len();
        // Repeated synthetic division (Horner shift to tau).
        for i in 0..k {
            for j in 1..k - i {
                a[j] += s * a[j - 1];
            }
        }
        a.reverse();
        a
    }
}

impl Bsp {
    pub fn number(&self) -> usize {
        self.knots.len() - self.order
    }
    /// Knot `i` with virtual distinct knots beyond both ends (they only
    /// enter B-splines whose coefficients do not exist).
    fn knot(&self, i: isize) -> f64 {
        let t = &self.knots;
        let n = t.len() as isize;
        let span = (t[t.len() - 1] - t[0]).max(1.);
        if i < 0 {
            t[0] + i as f64 * span
        } else if i >= n {
            t[t.len() - 1] + (i - n + 1) as f64 * span
        } else {
            t[i as usize]
        }
    }
    /// The interval index mu (knots[mu] <= x < knots[mu+1], nondegenerate)
    /// and the `order` B-spline values of indices mu-k+1..=mu at x; None
    /// outside [knots(1), knots(end)] (the function is zero there).
    /// Chunked cancellation runs inside the quadratic recurrence.
    pub fn basis_at_checked(
        &self,
        x: f64,
        check_cancel: &dyn Fn() -> Result<()>,
    ) -> Result<Option<(isize, Vec<f64>)>> {
        let t = &self.knots;
        let last = t[t.len() - 1];
        if x.is_nan() || x < t[0] || x > last || t[0] == last {
            return Ok(None);
        }
        let mu = if x == last {
            t.partition_point(|&v| v < last) - 1
        } else {
            t.partition_point(|&v| v <= x) - 1
        } as isize;
        let k = self.order;
        let mut b = vec![0.; k];
        let mut dl = vec![0.; k];
        let mut dr = vec![0.; k];
        b[0] = 1.;
        let mut updates = 0usize;
        for j in 1..k {
            dr[j - 1] = self.knot(mu + j as isize) - x;
            dl[j - 1] = x - self.knot(mu + 1 - j as isize);
            let mut saved = 0.;
            for r in 0..j {
                if updates.is_multiple_of(64) {
                    check_cancel()?;
                }
                updates += 1;
                let term = b[r] / (dr[r] + dl[j - 1 - r]);
                b[r] = saved + dr[r] * term;
                saved = dl[j - 1 - r] * term;
            }
            b[j] = saved;
        }
        Ok(Some((mu, b)))
    }
    /// All components at x into `out` (dim values).
    #[cfg(test)]
    pub fn eval_into(&self, x: f64, out: &mut [f64]) {
        self.eval_into_checked(x, out, &|| Ok(()))
            .expect("an infallible cancellation callback cannot fail");
    }
    pub fn eval_into_checked(
        &self,
        x: f64,
        out: &mut [f64],
        check_cancel: &dyn Fn() -> Result<()>,
    ) -> Result<()> {
        out.fill(0.);
        if x.is_nan() {
            out.fill(f64::NAN);
            return Ok(());
        }
        let Some((mu, b)) = self.basis_at_checked(x, check_cancel)? else {
            return Ok(());
        };
        let n = self.number() as isize;
        let k = self.order as isize;
        for (r, br) in b.iter().enumerate() {
            let j = mu - k + 1 + r as isize;
            if (0..n).contains(&j) {
                for (c, o) in out.iter_mut().enumerate() {
                    *o += br * self.coefs[j as usize * self.dim + c];
                }
            }
        }
        Ok(())
    }
    /// Derivative of order d: each step is order k-1 on the same knots
    /// with coefficients (k-1)(a_i - a_{i-1})/(t_{i+k-1} - t_i), dropping
    /// the end B-splines whose knots all coincide.
    pub fn der(&self, d: usize) -> Bsp {
        let mut f = self.clone();
        for _ in 0..d {
            let (k, n, dim) = (f.order, f.number(), f.dim);
            if k == 1 {
                f.coefs.fill(0.);
                break;
            }
            let t = &f.knots;
            let mut coefs = vec![0.; (n + 1) * dim];
            for i in 0..=n {
                let den = t[i + k - 1] - t[i];
                for c in 0..dim {
                    let hi = if i < n { f.coefs[i * dim + c] } else { 0. };
                    let lo = if i > 0 {
                        f.coefs[(i - 1) * dim + c]
                    } else {
                        0.
                    };
                    coefs[i * dim + c] = if den > 0. {
                        (k - 1) as f64 * (hi - lo) / den
                    } else {
                        0.
                    };
                }
            }
            let mut knots = t.clone();
            let (mut lo, mut hi) = (0, n + 1);
            if t[k - 1] == t[0] && hi - lo > 1 {
                lo = 1;
            }
            if t[n + k - 1] == t[n] && hi - lo > 1 {
                hi -= 1;
            }
            knots = knots[lo..hi + k - 1].to_vec();
            f = Bsp {
                knots,
                coefs: coefs[lo * dim..hi * dim].to_vec(),
                order: k - 1,
                dim,
            };
        }
        f
    }
    /// Indefinite integral (order k+1, end knots repeated once more), zero at
    /// knots(1) plus `value`.
    pub fn int(&self, value: &[f64]) -> Bsp {
        let (k, mut n, dim) = (self.order, self.number(), self.dim);
        // The B-form evaluator has implicit zero coefficients before the
        // first knot.  Make those coefficients explicit before integrating:
        // then the first coefficient of the antiderivative is its value at
        // the left end of the basic interval, also for an unclamped form.
        let left = self
            .knots
            .iter()
            .take_while(|&&v| v == self.knots[0])
            .count();
        let pad = k - left;
        let mut t = Vec::with_capacity(self.knots.len() + pad);
        t.extend(std::iter::repeat_n(self.knots[0], pad));
        t.extend_from_slice(&self.knots);
        let mut source = vec![0.; pad * dim];
        source.extend_from_slice(&self.coefs);
        n += pad;
        let mut coefs = vec![0.; (n + 1) * dim];
        for c in 0..dim {
            let mut acc = 0.;
            coefs[c] = value[c];
            for i in 0..n {
                acc += source[i * dim + c] * (t[i + k] - t[i]) / k as f64;
                coefs[(i + 1) * dim + c] = acc + value[c];
            }
        }
        let mut knots = Vec::with_capacity(t.len() + 2);
        knots.push(t[0]);
        knots.extend_from_slice(&t);
        knots.push(t[t.len() - 1]);
        Bsp {
            knots,
            coefs,
            order: k + 1,
            dim,
        }
    }
    /// Distinct knots, the breaks of the pp form.
    pub fn breaks(&self) -> Vec<f64> {
        let mut b = self.knots.clone();
        b.dedup();
        b
    }
}

/// Order-k B-form on `knots` of a pp of order <= k (exact when the pp is a
/// spline on those knots): coefficient i is the de Boor-Fix functional
/// sum_r (-1)^(k-1-r) psi_i^(k-1-r)(tau) f^(r)(tau), psi_i(t) =
/// prod_{m=1}^{k-1} (t_{i+m} - t)/(k-1)!, tau inside [t_i, t_{i+k}].
pub(super) fn pp_to_b(pp: &Pp, knots: Vec<f64>, k: usize) -> Bsp {
    let n = knots.len() - k;
    let dim = pp.dim;
    let mut coefs = vec![0.; n * dim];
    let fact: f64 = (1..k).map(|v| v as f64).product();
    for i in 0..n {
        // tau: the middle of the longest knot interval inside the support.
        let (mut a, mut b) = (knots[i], knots[i]);
        for j in i..i + k {
            if knots[j + 1] - knots[j] > b - a {
                a = knots[j];
                b = knots[j + 1];
            }
        }
        let tau = 0.5 * (a + b);
        // psi_i as polynomial in s = t - tau, power 0 first.
        let mut psi = vec![1.];
        for m in 1..k {
            let am = knots[i + m] - tau;
            let mut next = vec![0.; psi.len() + 1];
            for (j, v) in psi.iter().enumerate() {
                next[j] += am * v;
                next[j + 1] -= v;
            }
            psi = next;
        }
        let piece = pp.piece_of(tau);
        for c in 0..dim {
            let f = pp.taylor(piece, c, tau);
            let mut sum = 0.;
            for (r, fr) in f.iter().enumerate().take(k) {
                // f^(r) = r! f_r, psi^(k-1-r) = (k-1-r)! psi_{k-1-r}.
                let q = k - 1 - r;
                let rf: f64 = (1..=r).map(|v| v as f64).product();
                let qf: f64 = (1..=q).map(|v| v as f64).product();
                let sign = if q.is_multiple_of(2) { 1. } else { -1. };
                sum += sign * qf * psi[q] * rf * fr;
            }
            coefs[i * dim + c] = sum / fact;
        }
    }
    Bsp {
        knots,
        coefs,
        order: k,
        dim,
    }
}

/// Knots t(1)*k, interior breaks once, t(end)*k.
pub(super) fn clamped_knots(breaks: &[f64], k: usize) -> Vec<f64> {
    let (first, last) = (breaks[0], breaks[breaks.len() - 1]);
    let mut t = vec![first; k];
    t.extend_from_slice(&breaks[1..breaks.len() - 1]);
    t.extend(std::iter::repeat_n(last, k));
    t
}

fn text_array(e: &Engine, s: &str) -> Result<Array> {
    Ok(Array::Character(CharacterRow::from_text(s, &e.cancel)?))
}
fn row_array(v: &[f64]) -> Result<Array> {
    Ok(Array::Numeric(Matrix::from_column_vec(
        1,
        v.len(),
        v.to_vec(),
    )?))
}
fn scalar_array(v: usize) -> Array {
    Array::Numeric(Matrix::scalar(v as f64))
}
fn struct_value(e: &Engine, fields: Vec<(&str, Array)>) -> Result<Value> {
    let (names, slots) = fields
        .into_iter()
        .map(|(n, a)| (n.to_owned(), Arc::new(a)))
        .unzip();
    Ok(Value::Container(Array::Struct(StructArray::from_slots(
        1, 1, names, slots, &e.cancel,
    )?)))
}

impl Pp {
    /// The (pieces*dim)-by-order coefficient matrix of mkpp.
    pub fn coef_matrix(&self) -> Result<Matrix> {
        let k = self.order;
        let rows = self.coefs.len() / k;
        let mut data = vec![0.; rows * k];
        for r in 0..rows {
            for c in 0..k {
                data[c * rows + r] = self.coefs[r * k + c];
            }
        }
        Matrix::from_column_vec(rows, k, data)
    }
    pub fn to_value(&self, e: &Engine) -> Result<Value> {
        e.polyopt_admit(self.coefs.len() + self.breaks.len())?;
        struct_value(
            e,
            vec![
                ("form", text_array(e, "pp")?),
                ("breaks", row_array(&self.breaks)?),
                ("coefs", Array::Numeric(self.coef_matrix()?)),
                ("pieces", scalar_array(self.pieces())),
                ("order", scalar_array(self.order)),
                ("dim", scalar_array(self.dim)),
            ],
        )
    }
}
impl Bsp {
    pub fn coef_matrix(&self) -> Result<Matrix> {
        Matrix::from_column_vec(self.dim, self.number(), self.coefs.clone())
    }
    pub fn to_value(&self, e: &Engine) -> Result<Value> {
        e.polyopt_admit(self.coefs.len() + self.knots.len())?;
        struct_value(
            e,
            vec![
                ("form", text_array(e, "B-")?),
                ("knots", row_array(&self.knots)?),
                ("coefs", Array::Numeric(self.coef_matrix()?)),
                ("number", scalar_array(self.number())),
                ("order", scalar_array(self.order)),
                ("dim", scalar_array(self.dim)),
            ],
        )
    }
}
impl Form {
    pub fn to_value(&self, e: &Engine) -> Result<Value> {
        match self {
            Form::Pp(p) => p.to_value(e),
            Form::B(b) => b.to_value(e),
        }
    }
    pub fn dim(&self) -> usize {
        match self {
            Form::Pp(p) => p.dim,
            Form::B(b) => b.dim,
        }
    }
    /// The basic interval.
    pub fn interval(&self) -> (f64, f64) {
        match self {
            Form::Pp(p) => (p.breaks[0], p.breaks[p.breaks.len() - 1]),
            Form::B(b) => (b.knots[0], b.knots[b.knots.len() - 1]),
        }
    }
    pub fn eval_into_checked(
        &self,
        x: f64,
        out: &mut [f64],
        check_cancel: &dyn Fn() -> Result<()>,
    ) -> Result<()> {
        match self {
            Form::Pp(p) => {
                for (c, o) in out.iter_mut().enumerate() {
                    *o = p.eval(x, c);
                }
                Ok(())
            }
            Form::B(b) => b.eval_into_checked(x, out, check_cancel),
        }
    }
    pub fn breaks(&self) -> Vec<f64> {
        match self {
            Form::Pp(p) => p.breaks.clone(),
            Form::B(b) => b.breaks(),
        }
    }
}

fn field<'a>(s: &'a StructArray, name: &str) -> Option<&'a Array> {
    let i = s.field_index(name)?;
    s.get(i, 0).ok().map(|a| a.as_ref())
}
fn field_matrix(s: &StructArray, name: &str, who: &str) -> Result<Matrix> {
    match field(s, name) {
        Some(Array::Numeric(m)) => Ok(m.clone()),
        _ => Err(format!(
            "{who}: the function's '{name}' field is missing or not numeric"
        )),
    }
}
fn field_count(s: &StructArray, name: &str, who: &str) -> Result<usize> {
    let m = field_matrix(s, name, who)?;
    match m.data() {
        [v] if *v >= 1. && v.fract() == 0. && *v < 1e9 => Ok(*v as usize),
        _ => Err(format!(
            "{who}: the function's '{name}' must be a positive integer"
        )),
    }
}

/// A dimension product supplied by a function-form structure.  These forms
/// come from user structs too, so no product may wrap before it is compared
/// with the coefficient storage.
fn form_product(who: &str, dimensions: &[usize]) -> Result<usize> {
    dimensions.iter().try_fold(1usize, |n, &d| {
        n.checked_mul(d)
            .ok_or_else(|| format!("{who}: the function form dimensions are too large"))
    })
}

fn form_order(k: usize, who: &str) -> Result<()> {
    if k > MAX_ORDER {
        Err(format!(
            "{who}: the function form order {k} exceeds {MAX_ORDER}"
        ))
    } else {
        Ok(())
    }
}

/// The function form of a value: Some for a pp or B-form struct.
pub(super) fn parse_form(v: &Value, who: &str) -> Result<Option<Form>> {
    let Value::Container(Array::Struct(s)) = v else {
        return Ok(None);
    };
    if s.len() != 1 {
        return Ok(None);
    }
    let form = match field(s, "form") {
        Some(Array::Character(c)) => String::from_utf16_lossy(c.units()),
        _ => return Ok(None),
    };
    match form.as_str() {
        "pp" => {
            let breaks = field_matrix(s, "breaks", who)?.data().to_vec();
            let c = field_matrix(s, "coefs", who)?;
            let k = field_count(s, "order", who)?;
            let d = field_count(s, "dim", who)?;
            let l = breaks.len().saturating_sub(1);
            let expected = form_product(who, &[l, d, k])?;
            form_order(k, who)?;
            if l == 0
                || c.len() != expected
                || breaks.windows(2).any(|w| w[1] < w[0] || w[0].is_nan())
            {
                return Err(format!(
                    "{who}: the pp form is inconsistent (breaks, coefs, order and dim do not agree)"
                ));
            }
            // coefs as (l*d)-by-k (mkpp), stored column-major.
            let rows = form_product(who, &[l, d])?;
            let data = c.data();
            let mut coefs = vec![0.; rows * k];
            for r in 0..rows {
                for m in 0..k {
                    coefs[r * k + m] = data[m * rows + r];
                }
            }
            Ok(Some(Form::Pp(Pp {
                breaks,
                coefs,
                order: k,
                dim: d,
            })))
        }
        "B-" => {
            let knots = field_matrix(s, "knots", who)?.data().to_vec();
            let c = field_matrix(s, "coefs", who)?;
            let k = field_count(s, "order", who)?;
            let d = field_count(s, "dim", who)?;
            let n = field_count(s, "number", who)?;
            let knot_count = n
                .checked_add(k)
                .ok_or_else(|| format!("{who}: the function form dimensions are too large"))?;
            let coefficient_count = form_product(who, &[n, d])?;
            form_order(k, who)?;
            if knots.len() != knot_count || c.len() != coefficient_count {
                return Err(format!(
                    "{who}: the B-form is inconsistent (knots, coefs, number, order and dim do not agree)"
                ));
            }
            check_knots(&knots, k, who)?;
            Ok(Some(Form::B(Bsp {
                knots,
                coefs: c.data().to_vec(),
                order: k,
                dim: d,
            })))
        }
        other => Err(format!(
            "{who}: the function form '{other}' is not supported (pp and B- are)"
        )),
    }
}

/// Knots finite, nondecreasing, multiplicity at most k.
pub(super) fn check_knots(t: &[f64], k: usize, who: &str) -> Result<()> {
    if t.iter().any(|v| !v.is_finite()) {
        return Err(format!("{who}: the knots must be finite"));
    }
    if t.windows(2).any(|w| w[1] < w[0]) {
        return Err(format!("{who}: the knot sequence must be nondecreasing"));
    }
    let mut run = 1;
    for w in t.windows(2) {
        run = if w[1] == w[0] { run + 1 } else { 1 };
        if run > k {
            return Err(format!(
                "{who}: no knot may have multiplicity greater than the order ({k})"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pp_to_b_reproduces_a_cubic_spline() {
        // A cubic with a jump in the third derivative at 1.
        let pp = Pp {
            breaks: vec![0., 1., 3.],
            coefs: vec![1., -2., 0.5, 3., 2., 1., -1.5, 2.5],
            order: 4,
            dim: 1,
        };
        // Make it C2: second piece from the first's value/derivatives at 1.
        let mut pp = pp;
        let (a, b, c, d) = (1., -2., 0.5, 3.);
        pp.coefs[4..].copy_from_slice(&[2., 3. * a + b, 3. * a + 2. * b + c, a + b + c + d]);
        let b = pp_to_b(&pp, clamped_knots(&pp.breaks, 4), 4);
        let mut out = [0.];
        for x in [0., 0.3, 1., 1.7, 2.9, 3.] {
            b.eval_into(x, &mut out);
            assert!((out[0] - pp.eval(x, 0)).abs() < 1e-12, "{x}");
        }
        let d = b.der(2);
        let pd = pp.der(2);
        for x in [0.1, 0.9, 1.5, 2.5] {
            d.eval_into(x, &mut out);
            assert!((out[0] - pd.eval(x, 0)).abs() < 1e-11, "{x}");
        }
    }
}
