//! quadl, quadv and quad2d, written from the published methods:
//!
//! - quadl: adaptive Gauss-Lobatto quadrature with a Kronrod extension, W. Gander and
//!   W. Gautschi, "Adaptive Quadrature - Revisited", BIT 40 (2000) 84-101: a 4-point
//!   Lobatto rule is compared with its 7-point extension, a 13-point rule fixes the
//!   scale of the error test, and an interval that fails is cut into six.
//! - quadv: adaptive Simpson quadrature (J. N. Lyness, "Notes on the adaptive Simpson
//!   quadrature routine", J. ACM 16 (1969) 483-495; Gander and Gautschi, same paper,
//!   Sec. 2) with array-valued integrands: the error estimate is the infinity norm of
//!   the difference of the two Simpson sums and the accepted value is extrapolated.
//! - quad2d: L. F. Shampine, "MATLAB program for quadrature in 2D", Appl. Math.
//!   Comput. 202 (2008) 266-274: the region is mapped onto a rectangle, tiled, and
//!   each tile gets the tensor product of the Gauss-Kronrod (7, 15) pair; tiles whose
//!   error estimate is too large are split. The substitution used for singular
//!   integrands is the cubic of Shampine, "Vectorized adaptive quadrature in MATLAB",
//!   J. Comput. Appl. Math. 211 (2008) 131-140.
use super::{NOT_ENOUGH, sc2_is_empty, sc2_num, sc2_scalar, sc2_text};
use crate::engine::{Engine, Execution, Value};
use crate::{Array, FunctionHandle, Matrix, Result};

const MAX_FCNT: usize = 10_000;
/// Smallest half-length quadl or quadv still bisects, as a fraction of the spacing of
/// doubles at the length of the whole interval. Chosen, not published.
const QUADL_HMIN: f64 = 1. / 32.;
const QUADV_HMIN: f64 = 1. / 1024.;
/// Where quadv first cuts the interval into three parts, as fractions of its length: not
/// at the thirds, so that a periodic integrand (cos 24 pi t on [0, 1]) is not sampled
/// at the same phase in every part and mistaken for a constant. Gander and Gautschi,
/// Sec. 2, place the points of their first estimate irregularly for the same reason.
/// Chosen, not published: three unequal parts, the first one 0.272 long so that a
/// 1/t singularity at the lower limit is cut off at the same step as in the reference
/// (tool/quad_prototypes/singular.py).
const QUADV_CUTS: [f64; 2] = [0.272, 0.618];
/// The three extra nodes of the 13-point Lobatto-Kronrod rule on [-1, 1] and its
/// weights for the node pairs +-1, +-x1, +-sqrt(2/3), +-x2, +-1/sqrt(5), +-x3 and the
/// centre. Solved from the moment equations by tool/lobatto_kronrod_constants.py
/// (exact to degree 19).
const LOBATTO_X: [f64; 3] = [0.9428824156954797, 0.6418533423457813, 0.23638319966214988];
const LOBATTO_W13: [f64; 7] = [
    0.015827191973480183,
    0.09427384021885005,
    0.1550719873365854,
    0.18882157396018245,
    0.19977340522685852,
    0.22492646533333951,
    0.24261107190140774,
];
/// Positive nodes of the Gauss-Kronrod (7, 15) pair, its Kronrod weights and the
/// Gauss weights at the same nodes (zero where the node belongs to the 15-point rule
/// only); the rule is symmetric about 0. The centre has its own weights.
const GK_NODES: [f64; 7] = [
    0.2077849550078985,
    0.4058451513773972,
    0.5860872354676911,
    0.7415311855993944,
    0.8648644233597691,
    0.9491079123427585,
    0.9914553711208126,
];
const GK_KRONROD: [f64; 7] = [
    0.2044329400752989,
    0.1903505780647854,
    0.1690047266392679,
    0.1406532597155259,
    0.1047900103222502,
    0.06309209262997855,
    0.02293532201052922,
];
const GK_GAUSS: [f64; 7] = [
    0.,
    0.3818300505051189,
    0.,
    0.2797053914892767,
    0.,
    0.1294849661688697,
    0.,
];
const GK_CENTER: (f64, f64) = (0.2094821410847278, 0.4179591836734694);
const MIN_STEP: &str = "Warning: Minimum step size reached; singularity possible.";
const MAX_COUNT: &str = "Warning: Maximum function count exceeded; singularity likely.";
const NON_FINITE: &str = "Warning: Infinite or Not-a-Number function value encountered.";
const SIZE_ERROR: &str =
    "The integrand function must return an output vector of the same length as the input vector.";
const SAME_SIZE: &str = "The integrand must return values of the same size at every point.";
const TILE_SPLIT: usize = 2;

/// The distance from |x| to the next larger double.
fn spacing(x: f64) -> f64 {
    let a = x.abs();
    if !a.is_finite() {
        return f64::NAN;
    }
    f64::from_bits(a.to_bits() + 1) - a
}

fn warning_text(code: u8) -> Option<&'static str> {
    match code {
        1 => Some(MIN_STEP),
        2 => Some(MAX_COUNT),
        3 => Some(NON_FINITE),
        _ => None,
    }
}

/// One adaptive integration: the integrand, its extra arguments and counters.
/// `tol` is quadl's error scale `|I| tol` (see `sc2_quadl`) or quadv's absolute tolerance
/// of one accepted interval; `hmin` is the half-length below which an interval is not
/// split.
struct Run<'a> {
    f: FunctionHandle,
    extra: &'a [Value],
    tol: f64,
    hmin: f64,
    fcnt: usize,
}

/// A value of the integrand of quadv: its shape and real data.
#[derive(Clone)]
struct Block {
    rows: usize,
    cols: usize,
    data: Vec<f64>,
}

impl Block {
    fn combine(&self, other: &Block, f: impl Fn(f64, f64) -> f64) -> Block {
        Block {
            rows: self.rows,
            cols: self.cols,
            data: self
                .data
                .iter()
                .zip(&other.data)
                .map(|(a, b)| f(*a, *b))
                .collect(),
        }
    }
    fn scale(&self, s: f64) -> Block {
        Block {
            rows: self.rows,
            cols: self.cols,
            data: self.data.iter().map(|v| s * v).collect(),
        }
    }
    /// Infinity norm: largest magnitude of a vector, largest absolute row sum of a
    /// matrix.
    fn norm_inf(&self) -> f64 {
        if self.rows == 1 || self.cols == 1 {
            return self.data.iter().fold(0., |m: f64, v| {
                if v.is_nan() || m.is_nan() {
                    f64::NAN
                } else {
                    m.max(v.abs())
                }
            });
        }
        (0..self.rows)
            .map(|r| {
                (0..self.cols)
                    .map(|c| self.data[c * self.rows + r].abs())
                    .sum::<f64>()
            })
            .fold(0., |m: f64, v| {
                if v.is_nan() || m.is_nan() {
                    f64::NAN
                } else {
                    m.max(v)
                }
            })
    }
}

impl Engine {
    fn sc2_function(&self, value: &Value) -> Result<FunctionHandle> {
        if let Some(name) = sc2_text(value) {
            return match self.named_handle(name.trim())? {
                Value::Container(Array::Handle(h)) => Ok(h),
                _ => Err("FUN must be a function handle or a function name.".into()),
            };
        }
        match value {
            Value::Container(Array::Handle(h)) => Ok(h.clone()),
            _ => Err("FUN must be a function handle or a function name.".into()),
        }
    }

    fn sc2_limit(&self, value: &Value) -> Result<f64> {
        sc2_scalar(value).ok_or_else(|| "The limits of integration must be real scalars.".into())
    }

    /// Calls the integrand; real data with its shape.
    fn sc2_apply(
        &mut self,
        f: &FunctionHandle,
        args: Vec<Value>,
        out: &mut Execution,
    ) -> Result<Block> {
        self.check_cancel()?;
        let points: usize = args
            .iter()
            .map(|a| match a {
                Value::Matrix(m) => m.len(),
                _ => 1,
            })
            .sum();
        self.charge(points)?;
        let value = self
            .call_handle(f, args, 1, None, out)?
            .into_iter()
            .next()
            .ok_or("The integrand must return a value.")?;
        let g = match value {
            Value::Complex(_) => {
                return Err("Complex integrands are not supported by this function yet.".into());
            }
            v => self.sc2_grid(&v, "The integrand value")?,
        };
        Ok(Block {
            rows: g.rows,
            cols: g.cols,
            data: g.data,
        })
    }

    /// The integrand at a row of points (quadl).
    fn sc2_row(&mut self, run: &Run, x: &[f64], out: &mut Execution) -> Result<Vec<f64>> {
        let mut args = vec![Value::Matrix(Matrix::from_column_slice(1, x.len(), x)?)];
        args.extend(run.extra.iter().cloned());
        let y = self.sc2_apply(&run.f, args, out)?;
        if y.data.len() != x.len() {
            return Err(SIZE_ERROR.into());
        }
        Ok(y.data)
    }

    /// The integrand at one point (quadv).
    fn sc2_point(&mut self, run: &Run, x: f64, out: &mut Execution) -> Result<Block> {
        let mut args = vec![Value::Matrix(Matrix::scalar(x))];
        args.extend(run.extra.iter().cloned());
        self.sc2_apply(&run.f, args, out)
    }

    fn sc2_tolerance(&self, value: Option<&Value>) -> Result<f64> {
        match value {
            None => Ok(1e-6),
            Some(v) if sc2_is_empty(v) => Ok(1e-6),
            Some(v) => sc2_scalar(v).ok_or_else(|| "The tolerance must be a scalar.".into()),
        }
    }

    // -- quadl ----------------------------------------------------------------

    /// Gander and Gautschi, Sec. 4-5: the 13 points of the Lobatto-Kronrod rule give an
    /// estimate I of the integral. The scale `|I| tol / eps` is where a rounding error
    /// of the sum `scale + (Q7 - Q4)` appears, so `scale + (Q7 - Q4) == scale` is the
    /// test that the two local rules agree to `tol` relative to the whole integral. The
    /// test is made in the relative form `s + (Q7 - Q4) eps == s` with `s = |I| tol`:
    /// eps is a power of two, so this is the same test without overflowing to Inf for a
    /// large integral (1e300 sin t). The tolerance is first divided by
    /// `R = |Q7 - I| / |Q4 - I|` when that lies in (0, 1), the observed convergence
    /// factor of the 7-point rule.
    pub(super) fn sc2_quadl(&mut self, args: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
        if args.len() < 3 {
            return Err(NOT_ENOUGH.into());
        }
        let f = self.sc2_function(&args[0])?;
        let a = self.sc2_limit(&args[1])?;
        let b = self.sc2_limit(&args[2])?;
        let tol = self.sc2_tolerance(args.get(3))?;
        // args[4] is the trace flag, which has no effect here.
        let extra = args.get(5..).unwrap_or(&[]);
        if !a.is_finite() || !b.is_finite() {
            return Err("The limits of integration must be finite.".into());
        }
        let (lo, hi, sign) = if a > b { (b, a, -1.) } else { (a, b, 1.) };
        let mut run = Run {
            f,
            extra,
            tol,
            hmin: spacing(hi - lo) * QUADL_HMIN,
            fcnt: 0,
        };
        let m = 0.5 * (lo + hi);
        let h = 0.5 * (hi - lo);
        let alpha = (2. / 3_f64).sqrt();
        let beta = 1. / 5_f64.sqrt();
        let [x1, x2, x3] = LOBATTO_X;
        let x = [
            lo,
            m - x1 * h,
            m - alpha * h,
            m - x2 * h,
            m - beta * h,
            m - x3 * h,
            m,
            m + x3 * h,
            m + beta * h,
            m + x2 * h,
            m + alpha * h,
            m + x1 * h,
            hi,
        ];
        let mut y = self.sc2_row(&run, &x, out)?;
        run.fcnt += x.len();
        // The integrand may be infinite or undefined at an end point (1/sqrt(t) at 0):
        // that value is dropped; the interior points still describe the singularity.
        for k in [0, 12] {
            if !y[k].is_finite() {
                y[k] = 0.;
            }
        }
        let ends = y[0] + y[12];
        let q4 = (h / 6.) * (ends + 5. * (y[4] + y[8]));
        let q7 =
            (h / 1470.) * (77. * ends + 432. * (y[2] + y[10]) + 625. * (y[4] + y[8]) + 672. * y[6]);
        let w = LOBATTO_W13;
        let i13 = h
            * (w[0] * ends
                + w[1] * (y[1] + y[11])
                + w[2] * (y[2] + y[10])
                + w[3] * (y[3] + y[9])
                + w[4] * (y[4] + y[8])
                + w[5] * (y[5] + y[7])
                + w[6] * y[6]);
        let (code, total) = if y.iter().any(|v| !v.is_finite()) {
            (3, q7)
        } else {
            let (e7, e4) = ((q7 - i13).abs(), (q4 - i13).abs());
            let ratio = e7 / e4;
            let tol = if ratio > 0. && ratio < 1. {
                tol / ratio
            } else {
                tol
            };
            let scale = i13.abs() * tol;
            run.tol = if scale == 0. {
                (hi - lo) * f64::EPSILON
            } else {
                scale
            };
            let (q, code) = self.sc2_lobatto(&mut run, lo, hi, y[0], y[12], out)?;
            (code, q)
        };
        if let Some(text) = warning_text(code) {
            self.emit(out, text.into())?;
        }
        Ok(vec![
            sc2_num(1, 1, vec![sign * total + 0.])?,
            sc2_num(1, 1, vec![run.fcnt as f64])?,
        ])
    }

    /// The adaptive step of Gander and Gautschi on [a, b] with the end values known:
    /// five new points give the 4-point rule (Q4) and its 7-point extension (Q7). The
    /// interval is accepted with Q7 when `run.tol + (Q7 - Q4) eps == run.tol`, when its
    /// inner nodes no longer differ from the ends in floating point, or when it is
    /// shorter than `run.hmin`; otherwise it is cut at the five nodes into six. The code
    /// is 0, or the first warning met (1 minimum step, 2 function count, 3 non-finite).
    fn sc2_lobatto(
        &mut self,
        run: &mut Run,
        a: f64,
        b: f64,
        fa: f64,
        fb: f64,
        out: &mut Execution,
    ) -> Result<(f64, u8)> {
        let m = 0.5 * (a + b);
        let h = 0.5 * (b - a);
        let alpha = (2. / 3_f64).sqrt();
        let beta = 1. / 5_f64.sqrt();
        let x = [m - alpha * h, m - beta * h, m, m + beta * h, m + alpha * h];
        let y = self.sc2_row(run, &x, out)?;
        run.fcnt += x.len();
        let q4 = (h / 6.) * (fa + fb + 5. * (y[1] + y[3]));
        let q7 = (h / 1470.)
            * (77. * (fa + fb) + 432. * (y[0] + y[4]) + 625. * (y[1] + y[3]) + 672. * y[2]);
        if y.iter().any(|v| !v.is_finite()) || !(q7 - q4).is_finite() {
            return Ok((q7, 3));
        }
        let too_short = h.abs() < run.hmin;
        if run.tol + (q7 - q4) * f64::EPSILON == run.tol || x[0] <= a || b <= x[4] || too_short {
            let code = u8::from(too_short || m <= a || b <= m);
            return Ok((q7, code));
        }
        if run.fcnt > MAX_FCNT {
            return Ok((q7, 2));
        }
        let points = [a, x[0], x[1], x[2], x[3], x[4], b];
        let values = [fa, y[0], y[1], y[2], y[3], y[4], fb];
        let mut total = 0.;
        let mut code = 0;
        for k in 0..6 {
            let (q, c) =
                self.sc2_lobatto(run, points[k], points[k + 1], values[k], values[k + 1], out)?;
            total += q;
            if code == 0 {
                code = c;
            }
        }
        Ok((total, code))
    }

    // -- quadv ----------------------------------------------------------------

    /// Adaptive Simpson (Lyness 1969): the interval is first cut into three unequal
    /// parts (`QUADV_CUTS`), so that an integrand that happens to vanish or repeat at the
    /// points of one Simpson sum is not mistaken for a simple one (Gander and Gautschi,
    /// Sec. 2), and each part is refined on its own.
    pub(super) fn sc2_quadv(&mut self, args: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
        if args.len() < 3 {
            return Err(NOT_ENOUGH.into());
        }
        let f = self.sc2_function(&args[0])?;
        let a = self.sc2_limit(&args[1])?;
        let b = self.sc2_limit(&args[2])?;
        let tol = self.sc2_tolerance(args.get(3))?;
        // args[4] is the trace flag, which has no effect here.
        let extra = args.get(5..).unwrap_or(&[]);
        if !a.is_finite() || !b.is_finite() {
            return Err("The limits of integration must be finite.".into());
        }
        let (lo, hi, sign) = if a > b { (b, a, -1.) } else { (a, b, 1.) };
        let mut run = Run {
            f,
            extra,
            tol,
            hmin: spacing(hi - lo) * QUADV_HMIN,
            fcnt: 0,
        };
        let cuts = [
            lo,
            lo + (hi - lo) * QUADV_CUTS[0],
            lo + (hi - lo) * QUADV_CUTS[1],
            hi,
        ];
        let mut x = Vec::with_capacity(7);
        for k in 0..3 {
            x.push(cuts[k]);
            x.push(0.5 * (cuts[k] + cuts[k + 1]));
        }
        x.push(hi);
        let mut y: Vec<Block> = Vec::with_capacity(x.len());
        for point in &x {
            let v = self.sc2_point(&run, *point, out)?;
            run.fcnt += 1;
            if y.first()
                .is_some_and(|first| (first.rows, first.cols) != (v.rows, v.cols))
            {
                return Err(SAME_SIZE.into());
            }
            y.push(v);
        }
        // An end value that is infinite or undefined is dropped, entry by entry.
        for k in [0, 6] {
            for v in &mut y[k].data {
                if !v.is_finite() {
                    *v = 0.;
                }
            }
        }
        let shape = (y[0].rows, y[0].cols);
        let mut total = Block {
            rows: shape.0,
            cols: shape.1,
            data: vec![0.; y[0].data.len()],
        };
        let mut code = 0;
        for k in 0..3 {
            let (q, c) = self.sc2_simpson(
                &mut run,
                x[2 * k],
                x[2 * k + 2],
                &y[2 * k],
                &y[2 * k + 1],
                &y[2 * k + 2],
                out,
            )?;
            total = total.combine(&q, |s, v| s + v);
            if code == 0 {
                code = c;
            }
        }
        if let Some(text) = warning_text(code) {
            self.emit(out, text.into())?;
        }
        let data = total.scale(sign).data.iter().map(|v| v + 0.).collect();
        Ok(vec![
            sc2_num(shape.0, shape.1, data)?,
            sc2_num(1, 1, vec![run.fcnt as f64])?,
        ])
    }

    /// One Simpson step on [a, b] with the values at a, the midpoint and b: the sum S1
    /// of one Simpson panel against the sum S2 of the two half panels. S2 + (S2 - S1)/15
    /// (one Romberg step) is returned when the estimated error of S2, the infinity norm
    /// of (S2 - S1)/15, is within `run.tol`; the tolerance is not divided between the
    /// halves. Otherwise both halves are refined. Codes as in `sc2_lobatto`.
    #[allow(clippy::too_many_arguments)]
    fn sc2_simpson(
        &mut self,
        run: &mut Run,
        a: f64,
        b: f64,
        fa: &Block,
        fc: &Block,
        fb: &Block,
        out: &mut Execution,
    ) -> Result<(Block, u8)> {
        let h = b - a;
        let c = 0.5 * (a + b);
        let d = 0.5 * (a + c);
        let e = 0.5 * (c + b);
        let fd = self.sc2_point(run, d, out)?;
        let fe = self.sc2_point(run, e, out)?;
        run.fcnt += 2;
        if [&fd, &fe]
            .iter()
            .any(|v| (v.rows, v.cols) != (fa.rows, fa.cols))
        {
            return Err(SAME_SIZE.into());
        }
        let n = fa.data.len();
        let s1: Vec<f64> = (0..n)
            .map(|i| h / 6. * (fa.data[i] + 4. * fc.data[i] + fb.data[i]))
            .collect();
        let s2: Vec<f64> = (0..n)
            .map(|i| {
                h / 12.
                    * (fa.data[i]
                        + 4. * fd.data[i]
                        + 2. * fc.data[i]
                        + 4. * fe.data[i]
                        + fb.data[i])
            })
            .collect();
        let block = |data: Vec<f64>| Block {
            rows: fa.rows,
            cols: fa.cols,
            data,
        };
        let s1 = block(s1);
        let s2 = block(s2);
        let diff = s2.combine(&s1, |x, y| x - y);
        let err = diff.norm_inf();
        if !err.is_finite() {
            return Ok((s2, 3));
        }
        let too_short = h.abs() < run.hmin;
        if err <= 15. * run.tol || d <= a || b <= e || too_short {
            let code = u8::from(too_short || c <= a || b <= c);
            return Ok((s2.combine(&diff, |s, v| s + v / 15.), code));
        }
        if run.fcnt > MAX_FCNT {
            return Ok((s2.combine(&diff, |s, v| s + v / 15.), 2));
        }
        let (left, lc) = self.sc2_simpson(run, a, c, fa, &fd, fc, out)?;
        let (right, rc) = self.sc2_simpson(run, c, b, fc, &fe, fb, out)?;
        let code = if lc == 0 { rc } else { lc };
        Ok((left.combine(&right, |x, y| x + y), code))
    }

    // -- quad2d ---------------------------------------------------------------

    /// Shampine's tiled method. The region is mapped onto a rectangle in (theta, phi),
    /// x = X(theta) and y = lower + (upper - lower) * Y(phi), and `sc2_tensor` applies the
    /// Gauss-Kronrod (7, 15) product rule to the four quarters of a rectangle. A tile
    /// is kept when its error estimate is within its share (by area) of the tolerance
    /// `max(AbsTol, RelTol |q|)`, the others are split again, until the summed estimate
    /// meets the tolerance or MaxFunEvals tile rules (225 integrand values each) are
    /// spent. Reaching that limit is not reported. A tile whose value or error is not
    /// finite cannot be improved by splitting: the integration stops with a warning.
    pub(super) fn sc2_quad2d(&mut self, args: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
        if args.len() < 5 {
            return Err(NOT_ENOUGH.into());
        }
        let f = self.sc2_function(&args[0])?;
        let a = self.sc2_limit(&args[1])?;
        let b = self.sc2_limit(&args[2])?;
        let c = Edge::new(self, &args[3])?;
        let d = Edge::new(self, &args[4])?;
        let options = &args[5..];
        if !options.len().is_multiple_of(2) {
            return Err("Parameter names and values must come in pairs.".into());
        }
        let mut abs_tol = 1e-10;
        let mut rel_tol = 1e-6;
        let mut max_evals = 2000_usize;
        let mut singular = true;
        for pair in options.chunks(2) {
            let name = sc2_text(&pair[0]).ok_or("Parameter names must be character vectors.")?;
            let number = sc2_scalar(&pair[1]);
            let flag = || match number {
                Some(v) if v == 0. || v == 1. => Some(v == 1.),
                _ => None,
            };
            match name.to_ascii_lowercase().as_str() {
                "abstol" => match number {
                    Some(v) if v.is_finite() && v >= 0. => abs_tol = v,
                    _ => return Err("AbsTol must be a nonnegative scalar.".into()),
                },
                "reltol" => match number {
                    Some(v) if v.is_finite() && v >= 0. => rel_tol = v,
                    _ => return Err("RelTol must be a nonnegative scalar.".into()),
                },
                "maxfunevals" => match number {
                    Some(v) if v.is_finite() && v >= 1. && v.fract() == 0. => {
                        max_evals = v as usize;
                    }
                    _ => return Err("MaxFunEvals must be a positive integer.".into()),
                },
                "singular" => {
                    singular = flag().ok_or("Singular must be true or false.")?;
                }
                "failureplot" => {
                    flag().ok_or("FailurePlot must be true or false.")?;
                }
                _ => {
                    return Err(format!(
                        "'{name}' is not a recognized parameter. For a list of valid name-value pair arguments, see the documentation for this function."
                    ));
                }
            }
        }
        if !a.is_finite() || !b.is_finite() {
            return Err("The limits of integration must be finite.".into());
        }
        if a == b {
            return Ok(vec![sc2_num(1, 1, vec![0.])?, sc2_num(1, 1, vec![0.])?]);
        }
        let (lo, hi, sign) = if a > b { (b, a, -1.) } else { (a, b, 1.) };
        let quad = Quad2d {
            f,
            a: lo,
            b: hi,
            c,
            d,
            singular,
        };
        let (theta, phi) = quad.rectangle();
        let area = (theta.1 - theta.0) * (phi.1 - phi.0);
        let mut ok = (0., 0.);
        // The region as a single tile whose error is not yet known.
        let mut tiles = vec![Tile {
            theta,
            phi,
            q: 0.,
            err: f64::INFINITY,
        }];
        let mut spent = 0;
        let mut nonfinite = false;
        loop {
            let q = ok.0 + tiles.iter().map(|t| t.q).sum::<f64>();
            let err = ok.1 + tiles.iter().map(|t| t.err).sum::<f64>();
            let tol = abs_tol.max(rel_tol * q.abs());
            if err <= tol || tiles.is_empty() || spent >= max_evals || nonfinite {
                break;
            }
            for tile in std::mem::take(&mut tiles) {
                if spent >= max_evals || nonfinite {
                    tiles.push(tile);
                    continue;
                }
                let new = self.sc2_tensor(&quad, tile.theta, tile.phi, out)?;
                spent += new.len();
                if new.iter().any(|t| !t.q.is_finite() || !t.err.is_finite()) {
                    nonfinite = true;
                    tiles.extend(new);
                    continue;
                }
                save_tiles(new, tol, area, &mut tiles, &mut ok);
            }
        }
        if nonfinite {
            self.emit(out, NON_FINITE.into())?;
        }
        let q = ok.0 + tiles.iter().map(|t| t.q).sum::<f64>();
        let err = ok.1 + tiles.iter().map(|t| t.err).sum::<f64>();
        Ok(vec![
            sc2_num(1, 1, vec![sign * q + 0.])?,
            sc2_num(1, 1, vec![err])?,
        ])
    }

    /// Splits the rectangle into `TILE_SPLIT` by `TILE_SPLIT` tiles and applies the
    /// Gauss-Kronrod product rule to each, with one call of the integrand for all of
    /// them (15 rows, 15 columns per tile). A tile's value is the 15 by 15 rule; its error
    /// estimate is the difference to the 7 by 7 Gauss rule at the same points.
    fn sc2_tensor(
        &mut self,
        quad: &Quad2d,
        theta: (f64, f64),
        phi: (f64, f64),
        out: &mut Execution,
    ) -> Result<Vec<Tile>> {
        let (nodes, kronrod, gauss) = gk_rule();
        let cut = |(lo, hi): (f64, f64), k: usize| {
            let at = |i: usize| match i {
                0 => lo,
                i if i == TILE_SPLIT => hi,
                i => lo + (hi - lo) * i as f64 / TILE_SPLIT as f64,
            };
            (at(k), at(k + 1))
        };
        let mut rectangles = Vec::new();
        for i in 0..TILE_SPLIT {
            for j in 0..TILE_SPLIT {
                rectangles.push((cut(theta, i), cut(phi, j)));
            }
        }
        let n = nodes.len();
        let tiles = rectangles.len();
        // x, its derivative with respect to theta, per tile and row of the grid.
        let mut xs = Vec::with_capacity(n * tiles);
        let mut dxs = Vec::with_capacity(n * tiles);
        for (th, _) in &rectangles {
            for node in nodes {
                let (x, dx) = quad.x_at(0.5 * (th.0 + th.1) + 0.5 * (th.1 - th.0) * node);
                xs.push(x);
                dxs.push(dx);
            }
        }
        let row = Matrix::from_column_slice(1, xs.len(), &xs)?;
        let lower = quad.c.at(self, &row, out)?;
        let upper = quad.d.at(self, &row, out)?;
        let mut gx = Vec::with_capacity(n * n * tiles);
        let mut gy = Vec::with_capacity(n * n * tiles);
        let mut fractions = Vec::with_capacity(n * tiles);
        for (t, (_, ph)) in rectangles.iter().enumerate() {
            for node in nodes {
                let (fraction, dfraction) =
                    quad.fraction(0.5 * (ph.0 + ph.1) + 0.5 * (ph.1 - ph.0) * node);
                fractions.push(dfraction);
                for i in 0..n {
                    gx.push(xs[t * n + i]);
                    gy.push(lower[t * n + i] + (upper[t * n + i] - lower[t * n + i]) * fraction);
                }
            }
        }
        let rows = n;
        let cols = n * tiles;
        let g = self.sc2_apply(
            &quad.f,
            vec![
                Value::Matrix(Matrix::from_column_slice(rows, cols, &gx)?),
                Value::Matrix(Matrix::from_column_slice(rows, cols, &gy)?),
            ],
            out,
        )?;
        if g.data.len() != rows * cols {
            return Err(SAME_SIZE.into());
        }
        let mut result = Vec::with_capacity(tiles);
        for (t, (th, ph)) in rectangles.iter().enumerate() {
            let (mut qk, mut qg) = (0., 0.);
            for j in 0..n {
                for i in 0..n {
                    let at = t * n + i;
                    let v = g.data[(t * n + j) * n + i]
                        * dxs[at]
                        * (upper[at] - lower[at])
                        * fractions[t * n + j];
                    qk += kronrod[i] * kronrod[j] * v;
                    qg += gauss[i] * gauss[j] * v;
                }
            }
            let scale = 0.25 * (th.1 - th.0) * (ph.1 - ph.0);
            result.push(Tile {
                theta: *th,
                phi: *ph,
                q: qk * scale,
                err: (qk - qg).abs() * scale,
            });
        }
        Ok(result)
    }
}

/// Keeps the tiles whose error estimate is within their area's share of `tol` (adding
/// them to `ok`: value and error) and queues the rest in `tiles` for splitting.
fn save_tiles(new: Vec<Tile>, tol: f64, area: f64, tiles: &mut Vec<Tile>, ok: &mut (f64, f64)) {
    for tile in new {
        let share = (tile.theta.1 - tile.theta.0) * (tile.phi.1 - tile.phi.0) / area;
        if tile.err <= tol * share {
            ok.0 += tile.q;
            ok.1 += tile.err;
        } else {
            tiles.push(tile);
        }
    }
}

/// Nodes of the 15-point Gauss-Kronrod rule on [-1, 1] in increasing order, with the
/// Kronrod weights and the Gauss weights (zero at the nodes of the 15-point rule only).
fn gk_rule() -> ([f64; 15], [f64; 15], [f64; 15]) {
    let mut nodes = [0.; 15];
    let mut kronrod = [0.; 15];
    let mut gauss = [0.; 15];
    for k in 0..7 {
        nodes[k] = -GK_NODES[6 - k];
        nodes[14 - k] = GK_NODES[6 - k];
        kronrod[k] = GK_KRONROD[6 - k];
        kronrod[14 - k] = GK_KRONROD[6 - k];
        gauss[k] = GK_GAUSS[6 - k];
        gauss[14 - k] = GK_GAUSS[6 - k];
    }
    kronrod[7] = GK_CENTER.0;
    gauss[7] = GK_CENTER.1;
    (nodes, kronrod, gauss)
}

struct Quad2d {
    f: FunctionHandle,
    a: f64,
    b: f64,
    c: Edge,
    d: Edge,
    singular: bool,
}

impl Quad2d {
    /// The rectangle in (theta, phi) the region is mapped onto: with `Singular` both
    /// variables are substituted by the cubic t(3 - t^2)/4 (zero slope at the ends, which
    /// weakens end point singularities), without it x = theta and phi is the fraction of
    /// the way from the lower to the upper edge.
    fn rectangle(&self) -> ((f64, f64), (f64, f64)) {
        if self.singular {
            ((-1., 1.), (-1., 1.))
        } else {
            ((self.a, self.b), (0., 1.))
        }
    }

    /// x and dx/dtheta.
    fn x_at(&self, theta: f64) -> (f64, f64) {
        if self.singular {
            let w = self.b - self.a;
            (
                0.25 * w * theta * (3. - theta * theta) + 0.5 * (self.a + self.b),
                0.75 * w * (1. - theta * theta),
            )
        } else {
            (theta, 1.)
        }
    }

    /// The fraction of the way from the lower to the upper edge, and its derivative with
    /// respect to phi.
    fn fraction(&self, phi: f64) -> (f64, f64) {
        if self.singular {
            (0.5 + 0.25 * phi * (3. - phi * phi), 0.75 * (1. - phi * phi))
        } else {
            (phi, 1.)
        }
    }
}

struct Tile {
    theta: (f64, f64),
    phi: (f64, f64),
    q: f64,
    err: f64,
}

/// A lower or upper limit of the inner variable: a constant or a function of x.
enum Edge {
    Constant(f64),
    Function(FunctionHandle),
}

impl Edge {
    fn new(engine: &Engine, value: &Value) -> Result<Self> {
        match value {
            Value::Container(Array::Handle(h)) => Ok(Edge::Function(h.clone())),
            v => sc2_scalar(v).map(Edge::Constant).ok_or_else(|| {
                let _ = engine;
                "C and D must be scalars or function handles.".into()
            }),
        }
    }

    fn at(&self, engine: &mut Engine, x: &Matrix, out: &mut Execution) -> Result<Vec<f64>> {
        match self {
            Edge::Constant(v) => Ok(vec![*v; x.len()]),
            Edge::Function(h) => {
                let y = engine.sc2_apply(h, vec![Value::Matrix(x.clone())], out)?;
                if y.data.len() == 1 {
                    return Ok(vec![y.data[0]; x.len()]);
                }
                if y.data.len() != x.len() {
                    return Err(
                        "The limit function must return an array the size of its input.".into(),
                    );
                }
                Ok(y.data)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Engine;

    fn run(source: &str) -> (Engine, Vec<String>) {
        let mut engine = Engine::default();
        let result = engine.execute(source).unwrap();
        (engine, result.output)
    }

    fn numbers(engine: &Engine, name: &str) -> Vec<f64> {
        match engine.array(name) {
            Some(Array::Numeric(m)) => m.data().to_vec(),
            _ => panic!("{name} is not a double matrix"),
        }
    }

    fn values(source: &str) -> Vec<f64> {
        numbers(&run(source).0, "x")
    }

    #[test]
    fn lobatto_kronrod_constants_satisfy_their_moment_equations() {
        // +-1, +-x1, +-sqrt(2/3), +-x2, +-1/sqrt(5), +-x3 and 0: exact to degree 19.
        let [x1, x2, x3] = LOBATTO_X;
        let nodes = [1., x1, (2. / 3_f64).sqrt(), x2, 1. / 5_f64.sqrt(), x3];
        for k in 0..10_i32 {
            let power = 2 * k;
            let rule: f64 = nodes
                .iter()
                .zip(&LOBATTO_W13)
                .map(|(x, w)| 2. * w * x.powi(power))
                .sum::<f64>()
                + if k == 0 { LOBATTO_W13[6] } else { 0. };
            let exact = 2. / f64::from(2 * k + 1);
            assert!((rule - exact).abs() < 1e-15, "moment {power}: {rule}");
        }
    }

    #[test]
    fn gauss_kronrod_rule_integrates_polynomials() {
        let (nodes, kronrod, gauss) = gk_rule();
        for degree in 0..=22 {
            let exact = if degree % 2 == 0 {
                2. / f64::from(degree + 1)
            } else {
                0.
            };
            let k: f64 = nodes
                .iter()
                .zip(&kronrod)
                .map(|(x, w)| w * x.powi(degree))
                .sum();
            assert!((k - exact).abs() < 1e-15, "Kronrod degree {degree}: {k}");
            if degree <= 13 {
                let g: f64 = nodes
                    .iter()
                    .zip(&gauss)
                    .map(|(x, w)| w * x.powi(degree))
                    .sum();
                assert!((g - exact).abs() < 1e-15, "Gauss degree {degree}: {g}");
            }
        }
    }

    #[test]
    fn quadl_accepts_a_low_degree_polynomial_on_the_first_step() {
        // 13 points for the scale and 5 for the first interval.
        let x = values("[q, n] = quadl(@(t) t.^5 + 3*t.^2 - 1, -1, 2); x = [q n];");
        assert!((x[0] - (63. / 6. + 9. - 3.)).abs() < 1e-13, "{x:?}");
        assert_eq!(x[1], 18.);
        let x = values("[q, n] = quadl(@(t) t.^5 + 3*t.^2 - 1, 2, -1); x = [q n];");
        assert!((x[0] + (63. / 6. + 9. - 3.)).abs() < 1e-13, "{x:?}");
        assert_eq!(x[1], 18.);
    }

    #[test]
    fn quadl_refines_a_peak_and_passes_extra_arguments() {
        let x = values("x = quadl(@(t, w) 1 ./ (w^2 + t.^2), -1, 1, 1e-10, [], 0.05);");
        let exact = 2. / 0.05 * (1. / 0.05_f64).atan();
        assert!((x[0] - exact).abs() < 1e-7 * exact, "{x:?}");
    }

    #[test]
    fn quadl_integrates_an_integrable_end_point_singularity() {
        let (engine, output) = run("x = quadl(@(t) 1 ./ sqrt(t), 0, 1, 1e-8);");
        let x = numbers(&engine, "x");
        assert!((x[0] - 2.).abs() < 1e-3, "{x:?} {output:?}");
    }

    #[test]
    fn quadl_reports_a_nonfinite_value() {
        let (engine, output) = run("x = quadl(@(t) 0 ./ (t - 0.5), 0, 1);");
        assert!(numbers(&engine, "x")[0].is_nan());
        assert!(output.iter().any(|line| line == NON_FINITE), "{output:?}");
    }

    #[test]
    fn quadl_meets_its_tolerance_on_a_huge_integrand() {
        // |I| tol / eps overflows to Inf here; the stop test must not accept Q7 at once.
        let x = values("x = quadl(@(t) 1e300*sin(t), 0, 10);");
        let exact = 1e300 * (1. - 10_f64.cos());
        assert!((x[0] - exact).abs() < 1e-6 * exact, "{x:?} {exact}");
    }

    #[test]
    fn quadl_stops_at_the_function_count() {
        let (engine, output) = run("[x, n] = quadl(@(t) sin(1e4 * t.^2) ./ t.^2, 1e-3, 1, 1e-14);");
        assert!(numbers(&engine, "n")[0] > MAX_FCNT as f64);
        assert!(output.iter().any(|line| line == MAX_COUNT), "{output:?}");
    }

    #[test]
    fn quadv_counts_three_simpson_panels_for_a_polynomial() {
        let x = values("[q, n] = quadv(@(t) [t t.^2], 0, 1); x = [q n];");
        assert!(
            (x[0] - 0.5).abs() < 1e-15 && (x[1] - 1. / 3.).abs() < 1e-15,
            "{x:?}"
        );
        assert_eq!(x[2], 13.);
    }

    #[test]
    fn quadv_keeps_the_shape_and_reverses_limits() {
        let (engine, _) = run("x = quadv(@(t) [1 t; t.^2 t.^3], 1, 0);");
        let x = numbers(&engine, "x");
        let expected = [-1., -1. / 3., -0.5, -0.25];
        for (a, e) in x.iter().zip(expected) {
            assert!((a - e).abs() < 1e-14, "{x:?}");
        }
        assert_eq!(
            (
                engine.array("x").unwrap().rows(),
                engine.array("x").unwrap().cols()
            ),
            (2, 2)
        );
    }

    #[test]
    fn quadv_meets_its_tolerance_on_a_smooth_integrand() {
        let x = values("x = quadv(@(t) exp(-t.^2) .* [1 t], 0, 3, 1e-9);");
        let erf3 = 0.886_226_925_452_758 * 0.999_977_909_503_001_4;
        assert!((x[0] - erf3).abs() < 1e-8, "{x:?}");
        assert!((x[1] - 0.5 * (1. - (-9_f64).exp())).abs() < 1e-8, "{x:?}");
    }

    #[test]
    fn quadv_rejects_a_changing_shape() {
        let error = Engine::default()
            .execute("quadv(@(t) ones(1, 1 + (t > 0.3)), 0, 1)")
            .unwrap_err();
        assert!(
            error.ends_with("The integrand must return values of the same size at every point."),
            "{error}"
        );
    }

    #[test]
    fn quadv_does_not_mistake_a_periodic_integrand_for_a_constant() {
        // At the thirds and their quarters cos(24 pi t) is 1 at all 13 points.
        let x = values("[q, n] = quadv(@(t) cos(24*pi*t), 0, 1); x = [q n];");
        assert!(x[0].abs() < 1e-6, "{x:?}");
        assert!(x[1] > 13., "{x:?}");
    }

    #[test]
    fn warnings_continue_held_fprintf_text() {
        for call in ["quadl(@(t) 1./t, 0, 1)", "quadv(@(t) [t 1./t], 0, 1)"] {
            let (_, output) = run(&format!("fprintf('held '); x = {call};"));
            let line = "held Warning: Minimum step size reached; singularity possible.";
            assert!(output.iter().any(|l| l == line), "{call}: {output:?}");
        }
        let (_, output) = run("fprintf('held '); x = quad2d(@(u, v) 0./(u - u), 0, 1, 0, 1);");
        let line = "held Warning: Infinite or Not-a-Number function value encountered.";
        assert!(output.iter().any(|l| l == line), "{output:?}");
    }

    #[test]
    fn quad2d_integrates_polynomials_over_curved_regions() {
        let x = values("x = quad2d(@(u, v) u .* v, 0, 1, 0, @(u) u);");
        assert!((x[0] - 0.125).abs() < 1e-14, "{x:?}");
        // The inner integral is 2u((2u+1)^3 - u^3); the outer one gives 221.6.
        let x = values("x = quad2d(@(u, v) 6*u.*v.^2, 0, 2, @(u) u, @(u) 2*u + 1);");
        assert!((x[0] - 221.6).abs() < 1e-11 * 221.6, "{x:?}");
    }

    #[test]
    fn quad2d_options_and_orientation() {
        let x = values("x = quad2d(@(u, v) u + 2*v, 1, 0, 0, 1);");
        assert!((x[0] + 1.5).abs() < 1e-14, "{x:?}");
        let x = values("x = quad2d(@(u, v) u + 2*v, 0, 1, 1, 0);");
        assert!((x[0] + 1.5).abs() < 1e-14, "{x:?}");
        let x = values("x = quad2d(@(u, v) u + 2*v, 0, 1, 0, 1, 'singular', 0, 'AbsTol', 1e-12);");
        assert!((x[0] - 1.5).abs() < 1e-14, "{x:?}");
        assert_eq!(values("x = quad2d(@(u, v) u, 2, 2, 0, 1);"), [0.]);
    }

    #[test]
    fn quad2d_max_fun_evals_counts_tile_rules() {
        // One round of four 15 by 15 rules is done before the limit is looked at.
        let source = |limit: u32| {
            format!(
                "[q, e] = quad2d(@(u, v) sin(20*u).*cos(20*v), 0, 1, 0, 1, 'MaxFunEvals', {limit}); x = [q e];"
            )
        };
        let exact = (1. - 20_f64.cos()) / 20. * (20_f64.sin() / 20.);
        let few = values(&source(4));
        assert!((few[0] - exact).abs() < 1e-8, "{few:?} {exact}");
        assert!(few[1] > 1e-6, "{few:?}");
        let many = values(&source(2000));
        assert!((many[0] - exact).abs() < 1e-9 * exact, "{many:?}");
        assert!(many[1] < 1e-8, "{many:?}");
    }

    #[test]
    fn quad2d_handles_end_point_singularities_with_and_without_the_substitution() {
        let exact = 4. / 3. * (2_f64.powf(1.5) - 2.);
        for flag in ["true", "false"] {
            let x = values(&format!(
                "x = quad2d(@(u, v) 1 ./ sqrt(u + v), 0, 1, 0, 1, 'Singular', {flag});"
            ));
            assert!((x[0] - exact).abs() < 2e-6 * exact, "{flag}: {x:?}");
        }
    }

    #[test]
    fn quad2d_stops_at_a_nonfinite_value() {
        let (engine, output) = run("[q, e] = quad2d(@(u, v) 0./(u - u), 0, 1, 0, 1); x = [q e];");
        let x = numbers(&engine, "x");
        assert!(x[0].is_nan() && x[1].is_nan(), "{x:?}");
        let warning = "Warning: Infinite or Not-a-Number function value encountered.";
        assert_eq!(
            output.iter().filter(|l| *l == warning).count(),
            1,
            "{output:?}"
        );
    }

    #[test]
    fn quad2d_rejects_wrong_options() {
        for (source, message) in [
            (
                "quad2d(@(u, v) u, 0, 1, 0, 1, 'AbsTol')",
                "Parameter names and values must come in pairs.",
            ),
            (
                "quad2d(@(u, v) u, 0, 1, 0, 1, 3, 1)",
                "Parameter names must be character vectors.",
            ),
        ] {
            let error = Engine::default().execute(source).unwrap_err();
            assert!(error.ends_with(message), "{source}: {error}");
        }
    }

    #[test]
    fn quad2d_rejects_a_wrong_size_integrand_and_a_wrong_limit() {
        let error = Engine::default()
            .execute("quad2d(@(u, v) 1, 0, 1, 0, 1)")
            .unwrap_err();
        assert!(
            error.ends_with("The integrand must return values of the same size at every point."),
            "{error}"
        );
        let error = Engine::default()
            .execute("quad2d(@(u, v) u, 0, 1, 'a', 1)")
            .unwrap_err();
        assert!(
            error.ends_with("C and D must be scalars or function handles."),
            "{error}"
        );
    }
}
