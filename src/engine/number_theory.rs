//! Number theory, special functions, bit operations and base conversion.
//! Real double (plus logical) inputs only; character arguments are accepted
//! where MATLAB takes text (`nchoosek`, `bin2dec`, ...). User functions resolve first.
use super::{Engine, Value};
use crate::engine::Fma;
use crate::{Array, CharacterRow, LogicalMatrix, Matrix, Result};

const NAMES: &[&str] = &[
    "gcd",
    "lcm",
    "factorial",
    "nchoosek",
    "primes",
    "isprime",
    "factor",
    "gamma",
    "gammaln",
    "beta",
    "betaln",
    "erf",
    "erfc",
    "erfinv",
    "erfcinv",
    "bitand",
    "bitor",
    "bitxor",
    "bitshift",
    "bitcmp",
    "dec2bin",
    "bin2dec",
    "dec2hex",
    "hex2dec",
    "dec2base",
    "base2dec",
    "nthroot",
];

const EVALUATION_UNITS: usize = 32 * 1024 * 1024;
const FLINTMAX: f64 = 9_007_199_254_740_992.0;
/// Largest `primes` argument: the primes below it fit one matrix (16 Mi elements).
const PRIMES_LIMIT: f64 = 300_000_000.0;
const SYMBOLS: &[u8; 36] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";

pub(super) fn handles(name: &str) -> bool {
    NAMES.contains(&name)
}

fn is_int(x: f64) -> bool {
    x.is_finite() && x == x.trunc()
}

fn arity(args: &[Value], min: usize, max: usize) -> Result<()> {
    if args.len() < min {
        Err("Not enough input arguments.".into())
    } else if args.len() > max {
        Err("Too many input arguments.".into())
    } else {
        Ok(())
    }
}

// ---------- number theory kernels ----------

fn gcd_f(mut a: f64, mut b: f64) -> f64 {
    a = a.abs();
    b = b.abs();
    while b != 0.0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Coefficients with g = a*c + b*d (the classical Euclid recurrence). The remainder is
/// exact (`%`), so the loop ends even where q * v would overflow; remainders shrink at
/// least as fast as Fibonacci numbers, so doubles need fewer than 1500 steps.
fn gcd_bezout(a: f64, b: f64) -> Result<(f64, f64, f64)> {
    let mut u = [1.0, 0.0, a.abs()];
    let mut v = [0.0, 1.0, b.abs()];
    for _ in 0..1500 {
        if v[2] == 0.0 {
            if !(u[0].is_finite() && u[1].is_finite()) {
                break;
            }
            let sign = |x: f64| {
                if x > 0.0 {
                    1.0
                } else if x < 0.0 {
                    -1.0
                } else {
                    0.0
                }
            };
            return Ok((u[2], u[0] * sign(a), u[1] * sign(b)));
        }
        let r = u[2] % v[2];
        let q = ((u[2] - r) / v[2]).round();
        let t = [u[0] - q * v[0], u[1] - q * v[1], r];
        u = v;
        v = t;
    }
    Err("gcd coefficients overflow for these inputs.".into())
}

fn mulmod(a: u64, b: u64, m: u64) -> u64 {
    (u128::from(a) * u128::from(b) % u128::from(m)) as u64
}

fn powmod(mut base: u64, mut exp: u64, m: u64) -> u64 {
    let mut result = 1 % m;
    base %= m;
    while exp > 0 {
        if exp & 1 == 1 {
            result = mulmod(result, base, m);
        }
        base = mulmod(base, base, m);
        exp >>= 1;
    }
    result
}

/// Deterministic Miller-Rabin for every u64.
fn is_prime_u64(n: u64) -> bool {
    const SMALL: [u64; 12] = [2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37];
    if n < 2 {
        return false;
    }
    for p in SMALL {
        if n.is_multiple_of(p) {
            return n == p;
        }
    }
    let s = (n - 1).trailing_zeros();
    let d = (n - 1) >> s;
    'bases: for a in SMALL {
        let mut x = powmod(a, d, n);
        if x == 1 || x == n - 1 {
            continue;
        }
        for _ in 1..s {
            x = mulmod(x, x, n);
            if x == n - 1 {
                continue 'bases;
            }
        }
        return false;
    }
    true
}

fn gcd_u64(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// A nontrivial divisor of an odd composite `n` (Pollard rho, Floyd cycle).
fn rho_divisor(n: u64) -> u64 {
    let mut c = 1u64;
    loop {
        let f = |v: u64| (mulmod(v, v, n) + c) % n;
        let (mut x, mut y, mut d) = (2u64, 2u64, 1u64);
        while d == 1 {
            x = f(x);
            y = f(f(y));
            d = gcd_u64(x.abs_diff(y), n);
        }
        if d != n {
            return d;
        }
        c += 1;
    }
}

fn split_factors(n: u64, out: &mut Vec<u64>) {
    if n == 1 {
        return;
    }
    if is_prime_u64(n) {
        out.push(n);
        return;
    }
    let d = rho_divisor(n);
    split_factors(d, out);
    split_factors(n / d, out);
}

fn prime_factors(mut n: u64) -> Vec<u64> {
    let mut out = Vec::new();
    while n.is_multiple_of(2) && n > 1 {
        out.push(2);
        n /= 2;
    }
    let mut p = 3u64;
    while p < 1000 && p * p <= n {
        while n.is_multiple_of(p) {
            out.push(p);
            n /= p;
        }
        p += 2;
    }
    split_factors(n, &mut out);
    out.sort_unstable();
    out
}

/// Exact C(n, k) as f64 while it stays within flintmax.
fn binomial(n: f64, k: f64) -> Result<f64> {
    if !is_int(n) || !is_int(k) || n < 0.0 || k < 0.0 {
        return Err("N and K must be nonnegative integers.".into());
    }
    if k > n {
        return Err("K must be an integer between 0 and N.".into());
    }
    let k = k.min(n - k);
    if k == 0.0 {
        return Ok(1.0);
    }
    let limit = "nchoosek result exceeds flintmax (2^53); MatRust does not return inexact counts";
    if n >= 18_446_744_073_709_551_616.0 {
        return Err(limit.into());
    }
    let (n, k) = (n as u128, k as u128);
    let mut c: u128 = 1;
    for i in 1..=k {
        c = c.checked_mul(n - k + i).ok_or(limit)? / i;
        if c as f64 > FLINTMAX {
            return Err(limit.into());
        }
    }
    Ok(c as f64)
}

// ---------- special function kernels ----------

fn gamma_fn(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    if x == 0.0 || (x < 0.0 && x == x.floor()) {
        return f64::INFINITY;
    }
    libm::tgamma(x)
}

fn gammaln_fn(x: f64) -> f64 {
    libm::lgamma(x)
}

const FRAC_2_SQRT_PI: f64 = std::f64::consts::FRAC_2_SQRT_PI;

/// Giles' single-precision erfinv; `w = -ln(1 - y^2)`, `y` the signed argument.
fn erfinv_guess(w: f64, y: f64) -> f64 {
    let p = if w < 5.0 {
        let w = w - 2.5;
        [
            2.810_226_36e-08,
            3.432_739_39e-07,
            -3.523_387_7e-06,
            -4.391_506_54e-06,
            0.000_218_580_87,
            -0.001_253_725_03,
            -0.004_177_681_64,
            0.246_640_727,
            1.501_409_41,
        ]
        .iter()
        .fold(0.0, |p, &c| if p == 0.0 { c } else { c + p * w })
    } else {
        let w = w.sqrt() - 3.0;
        [
            -0.000_200_214_257,
            0.000_100_950_558,
            0.001_349_343_22,
            -0.003_673_428_44,
            0.005_739_507_73,
            -0.007_622_461_3,
            0.009_438_870_47,
            1.001_674_06,
            2.832_976_82,
        ]
        .iter()
        .fold(0.0, |p, &c| if p == 0.0 { c } else { c + p * w })
    };
    p * y
}

const SQRT_PI: f64 = 1.772_453_850_905_516;

/// ln(erfc(t) / x) and the slope -d/dt ln(erfc(t)) = 2/sqrt(pi) exp(-t^2) / erfc(t), for t > 0.
/// Above t = 26 erfc(t) nears the subnormal range, so ln(erfc) comes from the asymptotic
/// series erfc(t) = exp(-t^2) / (t sqrt(pi)) * sum (-1)^n (2n-1)!! / (2t^2)^n instead.
fn erfc_log_ratio(t: f64, x: f64, ln_x: f64) -> (f64, f64) {
    if t < 26.0 {
        let e = libm::erfc(t);
        let r = e / x;
        let g = if r.is_finite() {
            (r - 1.0).ln_1p()
        } else {
            e.ln() - ln_x
        };
        return (g, FRAC_2_SQRT_PI * (-t * t).exp() / e);
    }
    let z = 0.5 / (t * t);
    let (mut term, mut series) = (1.0, 1.0);
    for n in 1..12 {
        term *= -f64::from(2 * n - 1) * z;
        series += term;
    }
    let square = t * t;
    let square_low = t.fma(t, -square);
    let g = (-square - ln_x) - square_low - (t * SQRT_PI).ln() + series.ln();
    (g, 2.0 * t / series)
}

/// erfcinv for 0 < x <= 1.
fn erfcinv_core(x: f64) -> f64 {
    let w = -(x * (2.0 - x)).ln();
    if x >= 0.5 {
        // central: Halley on erf(t) = 1 - x (1 - x is exact here)
        let mut t = erfinv_guess(w, 1.0 - x);
        let y = 1.0 - x;
        for _ in 0..3 {
            let f = libm::erf(t) - y;
            let d = f / (FRAC_2_SQRT_PI * (-t * t).exp());
            t -= d / (1.0 + t * d);
        }
        return t;
    }
    let ln_x = x.ln();
    // Giles' fit covers w <= 16 (x >~ 1e-7); below that solve t^2 = -ln(x t sqrt(pi)),
    // the leading term of the asymptotic series, by fixed-point steps.
    let mut t = if w <= 16.0 {
        erfinv_guess(w, 1.0 - x)
    } else {
        let mut t = (-ln_x).sqrt();
        for _ in 0..3 {
            t = (-ln_x - (t * SQRT_PI).ln()).sqrt();
        }
        t
    };
    // tail: Newton on ln(erfc(t)) = ln(x); steps are bounded and never leave t > 0
    for _ in 0..8 {
        let (g, slope) = erfc_log_ratio(t, x, ln_x);
        let step = g / slope;
        if !step.is_finite() {
            break;
        }
        t += step.clamp(-0.5 * t, 0.5 * t);
        if step.abs() <= 1e-16 * t {
            break;
        }
    }
    t
}

fn erfcinv_fn(x: f64) -> f64 {
    if x.is_nan() || !(0.0..=2.0).contains(&x) {
        return f64::NAN;
    }
    if x == 0.0 {
        return f64::INFINITY;
    }
    if x == 2.0 {
        return f64::NEG_INFINITY;
    }
    if x == 1.0 {
        return 0.0;
    }
    if x < 1.0 {
        erfcinv_core(x)
    } else {
        -erfcinv_core(2.0 - x)
    }
}

fn erfinv_fn(y: f64) -> f64 {
    if y.is_nan() || y.abs() > 1.0 {
        return f64::NAN;
    }
    if y.abs() == 1.0 {
        return y * f64::INFINITY;
    }
    if y == 0.0 {
        return y;
    }
    let a = y.abs();
    let r = if a <= 0.5 {
        let mut t = erfinv_guess(-(1.0 - a * a).ln(), a);
        for _ in 0..3 {
            let f = libm::erf(t) - a;
            let d = f / (FRAC_2_SQRT_PI * (-t * t).exp());
            t -= d / (1.0 + t * d);
        }
        t
    } else {
        erfcinv_core(1.0 - a)
    };
    r.copysign(y)
}

/// Real n-th root. The power `|x|^(1/n)` starts one Newton step for `y^n = |x|`
/// (Burden & Faires, *Numerical Analysis*, sec. 2.3), written as
/// `y - y (1 - |x| / y^n) / n` so that `y^(n-1)` cannot overflow. For integer
/// orders a rounded candidate `r` with `r^|n| == |x|`, evaluated exactly in
/// integers below 2^53, proves the root is `r` (or `1/r` for negative `n`).
/// The caller has rejected negative `x` unless `n` is odd or not finite.
/// `n = -0` is `n = 0`, which evaluates `|x|^Inf`.
fn nthroot_fn(x: f64, n: f64) -> f64 {
    if x.is_nan() || n.is_nan() {
        return f64::NAN;
    }
    let n = if n == 0.0 { 0.0 } else { n };
    let a = x.abs();
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    if n.is_infinite() {
        return sign;
    }
    if n == 1.0 {
        return x;
    }
    if n == -1.0 {
        return 1.0 / x;
    }
    let mut y = libm::pow(a, 1.0 / n);
    if n != 0.0 && y != 0.0 && y.is_finite() {
        let step = y * (1.0 - power_ratio(a, y, n)) / n;
        if step.is_finite() {
            y -= step;
        }
    }
    if is_int(n) && (2.0..=53.0).contains(&n.abs()) && y != 0.0 && y.is_finite() {
        let root = if n > 0.0 { y } else { 1.0 / y }.round();
        if integer_power_is(root, n.abs() as u32, a) {
            y = if n > 0.0 { root } else { 1.0 / root };
        }
    }
    sign * y
}

/// `a / y^n` for `y^n` near `a`. Where `y^n` itself would overflow or fall into the
/// subnormals (a near `f64::MAX` or `f64::MIN_POSITIVE`), it is formed as two
/// factors `y^(n/2)` of about `sqrt(a)`, which stay normal.
fn power_ratio(a: f64, y: f64, n: f64) -> f64 {
    let whole = libm::pow(y, n);
    if whole.is_finite() && whole >= f64::MIN_POSITIVE {
        return a / whole;
    }
    let half = libm::pow(y, n / 2.0);
    a / half / half
}

/// Whether `base^power == target` exactly; false once a partial product leaves the
/// integers that binary64 holds exactly.
fn integer_power_is(base: f64, power: u32, target: f64) -> bool {
    const EXACT: f64 = 9_007_199_254_740_992.0; // 2^53
    if base < 1.0 {
        return false;
    }
    let mut product = 1.0;
    for _ in 0..power {
        product *= base;
        if product >= EXACT {
            return false;
        }
    }
    product == target
}

// ---------- base conversion ----------

fn digit_count(mut d: u64, base: u64) -> usize {
    let mut count = 1;
    while d >= base {
        d /= base;
        count += 1;
    }
    count
}

#[derive(Clone, Copy)]
struct AssumedType {
    bits: u32,
    signed: bool,
}

impl AssumedType {
    fn mask(self) -> u64 {
        u64::MAX >> (64 - self.bits)
    }

    fn encode(self, value: f64) -> u64 {
        if self.signed {
            (value as i64) as u64
        } else {
            value as u64
        }
    }

    fn decode(self, bits: u64) -> f64 {
        let bits = bits & self.mask();
        if self.signed {
            (((bits << (64 - self.bits)) as i64) >> (64 - self.bits)) as f64
        } else {
            bits as f64
        }
    }
}

impl Engine {
    fn numtheory_assumed_type(&self, value: Option<&Value>) -> Result<AssumedType> {
        let Some(value) = value else {
            return Ok(AssumedType {
                bits: 64,
                signed: false,
            });
        };
        let message = "ASSUMEDTYPE must be an integer type name.";
        let Some((1, _, units)) = self.numtheory_chars(value)? else {
            return Err(message.into());
        };
        let name = String::from_utf16_lossy(&units);
        let (bits, signed) = match name.as_str() {
            "uint8" => (8, false),
            "uint16" => (16, false),
            "uint32" => (32, false),
            "uint64" => (64, false),
            "int8" => (8, true),
            "int16" => (16, true),
            "int32" => (32, true),
            "int64" => (64, true),
            _ => return Err(message.into()),
        };
        Ok(AssumedType { bits, signed })
    }

    fn numtheory_bit_input(&self, a: &Matrix, assumed: AssumedType) -> Result<()> {
        let (min, max) = if assumed.signed {
            let bound = 2.0f64.powi(assumed.bits as i32 - 1);
            (-bound, bound - 1.0)
        } else {
            (0.0, assumed.mask() as f64)
        };
        self.numtheory_require(
            a,
            |v| is_int(v) && (min..=max).contains(&v),
            "Double inputs must have integer values in the range of ASSUMEDTYPE.",
        )?;
        self.numtheory_require(
            a,
            |v| v.abs() <= FLINTMAX,
            "Input must be within flintmax (2^53).",
        )
    }

    fn numtheory_charge(&self, len: usize) -> Result<()> {
        self.check_cancel()?;
        if self
            .evaluated_elements
            .checked_add(len)
            .is_none_or(|n| n > EVALUATION_UNITS)
        {
            return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
        }
        Ok(())
    }

    /// Real double view of a numeric or logical argument.
    fn numtheory_real(&self, value: &Value) -> Result<Matrix> {
        match value {
            Value::Matrix(m) => Ok(m.clone()),
            Value::Logical(m) => m.to_numeric(&self.cancel),
            Value::Complex(_) => Err("Input must be real.".into()),
            _ => Err("Input must be numeric or logical.".into()),
        }
    }

    /// Character argument as (rows, cols, UTF-16 units, is text); None for non-char values.
    fn numtheory_chars(&self, value: &Value) -> Result<Option<(usize, usize, Vec<u16>)>> {
        match value {
            Value::Text(s) => {
                let units: Vec<u16> = s.encode_utf16().collect();
                let rows = usize::from(!units.is_empty());
                let cols = units.len();
                Ok(Some((rows, cols, units)))
            }
            Value::Container(Array::Character(c)) => {
                Ok(Some((c.rows(), c.cols(), c.units().to_vec())))
            }
            Value::QuotedString(_) => {
                Err("String arrays are not supported; use a single-quoted character vector".into())
            }
            _ => Ok(None),
        }
    }

    fn numtheory_char_value(&self, rows: usize, cols: usize, units: Vec<u16>) -> Result<Value> {
        Ok(Value::Container(
            CharacterRow::from_column_major(rows, cols, units, &self.cancel)?.into(),
        ))
    }

    fn numtheory_map(&self, x: &Matrix, f: impl Fn(f64) -> f64 + Sync + Send) -> Result<Value> {
        self.numtheory_charge(x.len())?;
        Ok(Value::Matrix(x.map(f)))
    }

    fn numtheory_zip(
        &self,
        a: &Matrix,
        b: &Matrix,
        f: impl Fn(f64, f64) -> f64 + Sync + Send,
    ) -> Result<Value> {
        self.numtheory_charge(a.len().max(b.len()))?;
        if a.len() != 1 && b.len() != 1 && (a.rows(), a.cols()) != (b.rows(), b.cols()) {
            return Err("Matrix dimensions must agree.".into());
        }
        Ok(Value::Matrix(a.zip(b, f)?))
    }

    /// Every element of `x` must satisfy `ok` (cancel checked in blocks).
    fn numtheory_require(&self, x: &Matrix, ok: impl Fn(f64) -> bool, message: &str) -> Result<()> {
        for chunk in x.data().chunks(65_536) {
            self.check_cancel()?;
            if !chunk.iter().all(|&v| ok(v)) {
                return Err(message.into());
            }
        }
        Ok(())
    }

    pub(super) fn numtheory_outputs(
        &self,
        name: &str,
        args: &[Value],
        count: usize,
    ) -> Result<Vec<Value>> {
        self.check_cancel()?;
        let max_outputs = match name {
            "gcd" => 3,
            _ => 1,
        };
        if count > max_outputs {
            return Err("Too many output arguments.".into());
        }
        match name {
            "gcd" | "lcm" => {
                arity(args, 2, 2)?;
                let (a, b) = (
                    self.numtheory_real(&args[0])?,
                    self.numtheory_real(&args[1])?,
                );
                let message = if name == "lcm" {
                    "Input arguments must contain positive integers."
                } else {
                    "Inputs must be real integers."
                };
                let valid = |v| is_int(v) && (name != "lcm" || v > 0.0);
                self.numtheory_require(&a, valid, message)?;
                self.numtheory_require(&b, valid, message)?;
                if a.len() != 1 && b.len() != 1 && (a.rows(), a.cols()) != (b.rows(), b.cols()) {
                    return Err("Inputs must be the same size.".into());
                }
                if name == "lcm" {
                    return Ok(vec![self.numtheory_zip(&a, &b, |x, y| {
                        let g = gcd_f(x, y);
                        if g == 0.0 { 0.0 } else { (x / g * y).abs() }
                    })?]);
                }
                if count <= 1 {
                    return Ok(vec![self.numtheory_zip(&a, &b, gcd_f)?]);
                }
                if a.len() != 1 && b.len() != 1 && (a.rows(), a.cols()) != (b.rows(), b.cols()) {
                    return Err("Matrix dimensions must agree.".into());
                }
                let (rows, cols) = if a.len() == 1 {
                    (b.rows(), b.cols())
                } else {
                    (a.rows(), a.cols())
                };
                let len = rows * cols;
                self.numtheory_charge(len.saturating_mul(3))?;
                let pick = |m: &Matrix, i: usize| {
                    if m.len() == 1 {
                        m.data()[0]
                    } else {
                        m.data()[i]
                    }
                };
                let (mut g, mut c, mut d) = (
                    Vec::with_capacity(len),
                    Vec::with_capacity(len),
                    Vec::with_capacity(len),
                );
                for i in 0..len {
                    if i % 4096 == 0 {
                        self.check_cancel()?;
                    }
                    let (gi, ci, di) = gcd_bezout(pick(&a, i), pick(&b, i))?;
                    g.push(gi);
                    c.push(ci);
                    d.push(di);
                }
                let mut outputs = vec![Value::Matrix(Matrix::from_column_vec(rows, cols, g)?)];
                outputs.push(Value::Matrix(Matrix::from_column_vec(rows, cols, c)?));
                if count == 3 {
                    outputs.push(Value::Matrix(Matrix::from_column_vec(rows, cols, d)?));
                }
                Ok(outputs)
            }
            "factorial" => {
                arity(args, 1, 1)?;
                let n = self.numtheory_real(&args[0])?;
                self.numtheory_require(
                    &n,
                    |v| is_int(v) && v >= 0.0,
                    "N must be an array of real non-negative integers.",
                )?;
                // Running product, as a cumulative table: n > 170 overflows to Inf.
                let mut table = vec![1.0f64];
                for i in 1..=171u32 {
                    table.push(table[i as usize - 1] * f64::from(i));
                }
                Ok(vec![self.numtheory_map(&n, move |v| {
                    if v > 171.0 {
                        f64::INFINITY
                    } else {
                        table[v as usize]
                    }
                })?])
            }
            "nchoosek" => Ok(vec![self.numtheory_nchoosek(args)?]),
            "primes" => Ok(vec![self.numtheory_primes(args)?]),
            "isprime" => {
                arity(args, 1, 1)?;
                let x = self.numtheory_real(&args[0])?;
                self.numtheory_require(
                    &x,
                    |v| is_int(v) && v >= 0.0,
                    "All entries of X must be nonnegative integers.",
                )?;
                self.numtheory_charge(x.len())?;
                let mut flags = Vec::with_capacity(x.len());
                for (i, &v) in x.data().iter().enumerate() {
                    if i % 4096 == 0 {
                        self.check_cancel()?;
                    }
                    // Every finite double above flintmax is even. Avoid saturating it
                    // to u64::MAX (an unrelated integer) before testing primality.
                    flags.push(u8::from(v <= FLINTMAX && is_prime_u64(v as u64)));
                }
                Ok(vec![Value::Logical(LogicalMatrix::from_canonical_vec(
                    x.rows(),
                    x.cols(),
                    flags,
                )?)])
            }
            "factor" => {
                arity(args, 1, 1)?;
                let x = self.numtheory_real(&args[0])?;
                if x.len() != 1 {
                    return Err("N must be a scalar.".into());
                }
                let n = x.data()[0];
                if !is_int(n) || n < 0.0 {
                    return Err("N must be a real nonnegative integer.".into());
                }
                if n > FLINTMAX {
                    return Err("Input must not exceed flintmax (2^53).".into());
                }
                let factors = if n < 2.0 {
                    vec![n]
                } else {
                    prime_factors(n as u64)
                        .into_iter()
                        .map(|p| p as f64)
                        .collect()
                };
                self.check_cancel()?;
                let len = factors.len();
                Ok(vec![Value::Matrix(Matrix::from_column_vec(
                    1, len, factors,
                )?)])
            }
            "gamma" | "gammaln" | "erf" | "erfc" | "erfinv" | "erfcinv" => {
                arity(args, 1, 1)?;
                if matches!(
                    &args[0],
                    Value::Text(_) | Value::Container(Array::Character(_))
                ) {
                    return Err(format!(
                        "Undefined function '{name}' for input arguments of type 'char'."
                    ));
                }
                let x = self.numtheory_real(&args[0])?;
                let f: fn(f64) -> f64 = match name {
                    "gamma" => gamma_fn,
                    "gammaln" => {
                        self.numtheory_require(
                            &x,
                            |v| v.is_nan() || v >= 0.0,
                            "Input must be nonnegative.",
                        )?;
                        gammaln_fn
                    }
                    "erf" => libm::erf,
                    "erfc" => libm::erfc,
                    "erfinv" => erfinv_fn,
                    _ => erfcinv_fn,
                };
                Ok(vec![self.numtheory_map(&x, f)?])
            }
            "beta" | "betaln" => {
                arity(args, 2, 2)?;
                let (z, w) = (
                    self.numtheory_real(&args[0])?,
                    self.numtheory_real(&args[1])?,
                );
                let message = "Input must be nonnegative.";
                self.numtheory_require(&z, |v| v.is_nan() || v >= 0.0, message)?;
                self.numtheory_require(&w, |v| v.is_nan() || v >= 0.0, message)?;
                let ln = |a: f64, b: f64| gammaln_fn(a) + gammaln_fn(b) - gammaln_fn(a + b);
                Ok(vec![if name == "beta" {
                    self.numtheory_zip(&z, &w, move |a, b| ln(a, b).exp())?
                } else {
                    self.numtheory_zip(&z, &w, ln)?
                }])
            }
            "bitand" | "bitor" | "bitxor" => {
                arity(args, 2, 2)?;
                let (a, b) = (
                    self.numtheory_real(&args[0])?,
                    self.numtheory_real(&args[1])?,
                );
                let message = "Double inputs must have integer values in the range of ASSUMEDTYPE.";
                let ok = |v: f64| is_int(v) && v >= 0.0;
                self.numtheory_require(&a, ok, message)?;
                self.numtheory_require(&b, ok, message)?;
                for input in [&a, &b] {
                    self.numtheory_require(
                        input,
                        |v| v <= FLINTMAX,
                        "Input must not exceed flintmax (2^53).",
                    )?;
                }
                if a.len() != 1 && b.len() != 1 && (a.rows(), a.cols()) != (b.rows(), b.cols()) {
                    return Err("Inputs must have the same size.".into());
                }
                let op: fn(u64, u64) -> u64 = match name {
                    "bitand" => |x, y| x & y,
                    "bitor" => |x, y| x | y,
                    _ => |x, y| x ^ y,
                };
                Ok(vec![self.numtheory_zip(&a, &b, move |x, y| {
                    op(x as u64, y as u64) as f64
                })?])
            }
            "bitshift" => {
                arity(args, 2, 3)?;
                let (a, k) = (
                    self.numtheory_real(&args[0])?,
                    self.numtheory_real(&args[1])?,
                );
                let assumed = self.numtheory_assumed_type(args.get(2))?;
                self.numtheory_bit_input(&a, assumed)?;
                self.numtheory_require(&k, is_int, "Shift counts must be integers.")?;
                Ok(vec![self.numtheory_zip(&a, &k, move |x, s| {
                    let bits = assumed.encode(x);
                    let shifted = if s >= 0.0 {
                        if s >= 64.0 { 0 } else { bits << s as u32 }
                    } else if assumed.signed {
                        ((x as i64) >> ((-s).min(63.0) as u32)) as u64
                    } else if -s >= 64.0 {
                        0
                    } else {
                        bits >> (-s) as u32
                    };
                    assumed.decode(shifted)
                })?])
            }
            "bitcmp" => {
                arity(args, 1, 2)?;
                let a = self.numtheory_real(&args[0])?;
                let assumed = self.numtheory_assumed_type(args.get(1))?;
                self.numtheory_bit_input(&a, assumed)?;
                Ok(vec![self.numtheory_map(&a, |v| {
                    assumed.decode(!assumed.encode(v))
                })?])
            }
            "nthroot" => {
                arity(args, 2, 2)?;
                let (x, n) = (
                    self.numtheory_real(&args[0])?,
                    self.numtheory_real(&args[1])?,
                );
                let odd_or_nonfinite = |v: f64| !v.is_finite() || (is_int(v) && v % 2.0 != 0.0);
                self.numtheory_charge(x.len().max(n.len()))?;
                if x.len() != 1 && n.len() != 1 && (x.rows(), x.cols()) != (n.rows(), n.cols()) {
                    return Err("Matrix dimensions must agree.".into());
                }
                let at = |m: &Matrix, i: usize| {
                    if m.len() == 1 {
                        m.data()[0]
                    } else {
                        m.data()[i]
                    }
                };
                if (0..if x.is_empty() || n.is_empty() {
                    0
                } else {
                    x.len().max(n.len())
                })
                    .any(|i| at(&x, i) < 0.0 && !odd_or_nonfinite(at(&n, i)))
                {
                    return Err("If X is negative, N must be an odd integer.".into());
                }
                Ok(vec![self.numtheory_zip(&x, &n, nthroot_fn)?])
            }
            "dec2bin" | "dec2hex" | "dec2base" => Ok(vec![self.numtheory_dec2base(name, args)?]),
            "bin2dec" | "hex2dec" | "base2dec" => Ok(vec![self.numtheory_base2dec(name, args)?]),
            _ => Err(format!("{name} is not a number-theory function")),
        }
    }

    fn numtheory_primes(&self, args: &[Value]) -> Result<Value> {
        arity(args, 1, 1)?;
        let x = self.numtheory_real(&args[0])?;
        if x.len() != 1 {
            return Err("N must be a scalar".into());
        }
        let n = x.data()[0];
        if n.is_nan() || n.is_infinite() {
            return Err("Input must be finite.".into());
        }
        if n < 2.0 {
            return Ok(Value::Matrix(Matrix::from_column_vec(1, 0, Vec::new())?));
        }
        if n > PRIMES_LIMIT {
            return Err(format!(
                "primes supports N up to {PRIMES_LIMIT:.0}; larger limits exceed the matrix size limit"
            ));
        }
        let n = n as usize;
        // Odd-only bit sieve: bit i stands for 2i + 3.
        let half = (n - 1) / 2;
        let mut composite = vec![0u64; half.div_ceil(64)];
        let mut i = 0usize;
        while (2 * i + 3) * (2 * i + 3) <= n {
            self.check_cancel()?;
            if composite[i / 64] >> (i % 64) & 1 == 0 {
                let p = 2 * i + 3;
                let mut j = (p * p - 3) / 2;
                while j < half {
                    composite[j / 64] |= 1 << (j % 64);
                    j += p;
                }
            }
            i += 1;
        }
        let mut primes = vec![2.0f64];
        for j in 0..half {
            if j % 65_536 == 0 {
                self.check_cancel()?;
            }
            if composite[j / 64] >> (j % 64) & 1 == 0 {
                primes.push((2 * j + 3) as f64);
            }
        }
        let len = primes.len();
        Ok(Value::Matrix(Matrix::from_column_vec(1, len, primes)?))
    }

    fn numtheory_nchoosek(&self, args: &[Value]) -> Result<Value> {
        arity(args, 2, 2)?;
        let k = self.numtheory_real(&args[1])?;
        if k.len() != 1 {
            return Err("K must be a scalar.".into());
        }
        let k = k.data()[0];
        if !is_int(k) || k < 0.0 {
            return Err("The second input has to be a non-negative integer.".into());
        }
        let chars = self.numtheory_chars(&args[0])?;
        let numeric = if chars.is_none() {
            Some(self.numtheory_real(&args[0])?)
        } else {
            None
        };
        let n_elements = match (&chars, &numeric) {
            (Some((r, c, _)), _) => r * c,
            (_, Some(m)) => m.len(),
            _ => unreachable!(),
        };
        let is_count =
            numeric.as_ref().is_some_and(|m| m.len() == 1) && !matches!(args[0], Value::Logical(_));
        if is_count {
            let n = numeric.as_ref().unwrap().data()[0];
            return Ok(Value::Matrix(Matrix::scalar(binomial(n, k)?)));
        }
        let (rows_v, cols_v) = match (&chars, &numeric) {
            (Some((r, c, _)), _) => (*r, *c),
            (_, Some(m)) => (m.rows(), m.cols()),
            _ => unreachable!(),
        };
        if rows_v > 1 && cols_v > 1 {
            return Err("First argument must be a scalar or a vector.".into());
        }
        if !is_int(k) || k < 0.0 {
            return Err("K must be a nonnegative integer.".into());
        }
        let n = n_elements;
        let combos = if k > n as f64 {
            0.0
        } else {
            binomial(n as f64, k)?
        };
        let k = k as usize;
        let rows = combos as usize;
        let total = rows.checked_mul(k).ok_or("nchoosek result is too large")?;
        if total > crate::matrix::MAX_ELEMENTS {
            return Err(format!(
                "nchoosek result exceeds the {}-element limit",
                crate::matrix::MAX_ELEMENTS
            ));
        }
        self.numtheory_charge(total)?;
        // Lexicographic combinations of positions, written column-major.
        let mut index = vec![0usize; total];
        if rows > 0 && k > 0 {
            let mut comb: Vec<usize> = (0..k).collect();
            for r in 0..rows {
                if r % 4096 == 0 {
                    self.check_cancel()?;
                }
                for (c, &p) in comb.iter().enumerate() {
                    index[c * rows + r] = p;
                }
                let mut i = k;
                while i > 0 && comb[i - 1] == n - k + i - 1 {
                    i -= 1;
                }
                if i == 0 {
                    break;
                }
                comb[i - 1] += 1;
                for j in i..k {
                    comb[j] = comb[j - 1] + 1;
                }
            }
        }
        let (out_rows, out_cols) = if k == 0 { (1, 0) } else { (rows, k) };
        if let Some((_, _, units)) = chars {
            let gathered: Vec<u16> = index.iter().map(|&p| units[p]).collect();
            return self.numtheory_char_value(out_rows, out_cols, gathered);
        }
        let v = numeric.unwrap();
        if let Value::Logical(l) = &args[0] {
            let gathered: Vec<u8> = index.iter().map(|&p| l.data()[p]).collect();
            return Ok(Value::Logical(LogicalMatrix::from_canonical_vec(
                out_rows, out_cols, gathered,
            )?));
        }
        let gathered: Vec<f64> = index.iter().map(|&p| v.data()[p]).collect();
        Ok(Value::Matrix(Matrix::from_column_vec(
            out_rows, out_cols, gathered,
        )?))
    }

    /// Symbols for a base given as an integer 2..36 or a character vector of symbols.
    fn numtheory_symbols(&self, value: &Value) -> Result<Vec<u16>> {
        let message =
            "Base must be an integer between 2 and 36 or a character vector of unique symbols.";
        if let Some((rows, _, units)) = self.numtheory_chars(value)? {
            let mut sorted = units.clone();
            sorted.sort_unstable();
            sorted.dedup();
            if rows != 1 || units.len() < 2 || sorted.len() != units.len() || units.contains(&32) {
                return Err(message.into());
            }
            return Ok(units);
        }
        let b = self.numtheory_real(value)?;
        if b.len() != 1 || !is_int(b.data()[0]) || !(2.0..=36.0).contains(&b.data()[0]) {
            return Err(message.into());
        }
        Ok(SYMBOLS[..b.data()[0] as usize]
            .iter()
            .map(|&c| u16::from(c))
            .collect())
    }

    fn numtheory_dec2base(&self, name: &str, args: &[Value]) -> Result<Value> {
        let (symbols, width_arg) = match name {
            "dec2bin" => {
                arity(args, 1, 2)?;
                (
                    SYMBOLS[..2].iter().map(|&c| u16::from(c)).collect(),
                    args.get(1),
                )
            }
            "dec2hex" => {
                arity(args, 1, 2)?;
                (
                    SYMBOLS[..16].iter().map(|&c| u16::from(c)).collect(),
                    args.get(1),
                )
            }
            _ => {
                arity(args, 2, 3)?;
                let message = "Second argument must be an integer, 2 <= B <= 36.";
                let b = self.numtheory_real(&args[1]).map_err(|_| message)?;
                if b.len() != 1 || !is_int(b.data()[0]) || !(2.0..=36.0).contains(&b.data()[0]) {
                    return Err(message.into());
                }
                (self.numtheory_symbols(&args[1])?, args.get(2))
            }
        };
        let d = self.numtheory_real(&args[0])?;
        self.numtheory_require(
            &d,
            |v| (name == "dec2bin" || is_int(v)) && (0.0..=FLINTMAX).contains(&v),
            if name == "dec2bin" {
                // The reference keeps this historical text although it accepts 2^53.
                "D must be a non-negative integer smaller than 2^52."
            } else {
                "First argument must be an array of nonnegative integers up to 2^53."
            },
        )?;
        let min_width = match width_arg {
            Some(w) => {
                let w = self.numtheory_real(w)?;
                if w.len() != 1 || !is_int(w.data()[0]) || w.data()[0] < 0.0 {
                    return Err("Length argument must be a nonnegative integer.".into());
                }
                w.data()[0] as usize
            }
            None => 0,
        };
        if d.is_empty() {
            return self.numtheory_char_value(0, 0, Vec::new());
        }
        let base = symbols.len() as u64;
        // Size the result from the largest value and reject it before allocating.
        let rows = d.len();
        let mut largest = 0.0f64;
        for chunk in d.data().chunks(65_536) {
            self.check_cancel()?;
            largest = chunk.iter().fold(largest, |m, &v| m.max(v));
        }
        let width = min_width.max(digit_count(largest as u64, base));
        let total = CharacterRow::checked_len(rows, width)?;
        let mut units = vec![symbols[0]; total];
        for (r, &v) in d.data().iter().enumerate() {
            if r % 4096 == 0 {
                self.check_cancel()?;
            }
            let (mut value, mut column) = (v as u64, width);
            while value > 0 {
                column -= 1;
                units[column * rows + r] = symbols[(value % base) as usize];
                value /= base;
            }
        }
        self.numtheory_char_value(rows, width, units)
    }

    fn numtheory_base2dec(&self, name: &str, args: &[Value]) -> Result<Value> {
        let symbols: Vec<u16> = match name {
            "bin2dec" => {
                arity(args, 1, 1)?;
                SYMBOLS[..2].iter().map(|&c| u16::from(c)).collect()
            }
            "hex2dec" => {
                arity(args, 1, 1)?;
                SYMBOLS[..16].iter().map(|&c| u16::from(c)).collect()
            }
            _ => {
                arity(args, 2, 2)?;
                self.numtheory_symbols(&args[1])?
            }
        };
        let custom = name == "base2dec" && self.numtheory_chars(&args[1])?.is_some();
        let Some((rows, cols, units)) = self.numtheory_chars(&args[0])? else {
            return Err(if matches!(args[0], Value::Container(_)) {
                "Cell array input is not supported yet; pass a character array.".to_string()
            } else {
                "Input must be a string.".to_string()
            });
        };
        let base = symbols.len() as f64;
        self.numtheory_charge(rows)?;
        let mut out = Vec::with_capacity(rows);
        for r in 0..rows {
            if r % 1024 == 0 {
                self.check_cancel()?;
            }
            let mut value = 0.0f64;
            for c in 0..cols {
                let u = units[c * rows + r];
                if u == 32 && !custom && name != "hex2dec" {
                    continue;
                }
                let digit = if custom {
                    symbols.iter().position(|&s| s == u)
                } else {
                    let up = if (u16::from(b'a')..=u16::from(b'z')).contains(&u) {
                        u - 32
                    } else {
                        u
                    };
                    symbols.iter().position(|&s| s == up)
                };
                match digit {
                    Some(dv) => {
                        value = value * base + dv as f64;
                    }
                    None => {
                        return Err(match name {
                            "bin2dec" => {
                                "Binary string may consist only of characters 0 and 1".into()
                            }
                            "hex2dec" => {
                                "Input string found with characters other than 0-9, a-f, or A-F."
                                    .into()
                            }
                            _ => {
                                let row: Vec<u16> =
                                    (0..cols).map(|c| units[c * rows + r]).collect();
                                format!(
                                    "String {} contains characters which cannot be converted to base {}.",
                                    String::from_utf16_lossy(&row),
                                    base
                                )
                            }
                        });
                    }
                }
            }
            out.push(value);
        }
        Ok(Value::Matrix(Matrix::from_column_vec(
            rows,
            usize::from(rows != 0),
            out,
        )?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nthroot_exact_integer_roots_and_newton_accuracy() {
        for (x, n, root) in [
            (205_891_132_094_649., 30., 3.), // 3^30
            (1e15, 5., 1e3),
            (4_503_599_627_370_496., 52., 2.), // 2^52
            (-343., 3., -7.),
            (8., -3., 0.5),
        ] {
            assert_eq!(nthroot_fn(x, n), root, "nthroot({x}, {n})");
        }
        assert!((nthroot_fn(2., 3.) - libm::cbrt(2.)).abs() <= 2. * f64::EPSILON);
        assert!((nthroot_fn(10., 2.) - 10_f64.sqrt()).abs() <= 4. * f64::EPSILON);
        assert!(!integer_power_is(3., 34, 3_f64.powi(34)));
        assert!(nthroot_fn(1., f64::NAN).is_nan());
    }

    #[test]
    fn nthroot_correction_survives_extreme_powers_and_negative_zero() {
        let close = |got: f64, want: f64| (got - want).abs() <= 4. * f64::EPSILON * want.abs();
        // y^n rounds past f64::MAX (or into the subnormals), so a / y^n is formed in halves.
        let big = nthroot_fn(f64::MAX, -2.);
        assert!(close(big, 1. / libm::sqrt(f64::MAX)), "{big:e}");
        assert!(close(big, 7.458340731200207e-155), "{big:e}");
        let big = nthroot_fn(f64::MAX, -3.);
        assert!(close(big, 1. / libm::cbrt(f64::MAX)), "{big:e}");
        for tiny in [1e-320, f64::MIN_POSITIVE * 0.9, 5e-324] {
            let root = nthroot_fn(tiny, 3.);
            assert!(
                close(root, libm::cbrt(tiny)),
                "nthroot({tiny:e}, 3) = {root:e}"
            );
        }
        // n = -0 is n = 0, which evaluates x.^Inf.
        assert_eq!(nthroot_fn(4., -0.), f64::INFINITY);
        assert_eq!(nthroot_fn(0.25, -0.), 0.);
        assert_eq!(nthroot_fn(1., -0.), 1.);
    }
}
