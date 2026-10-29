//! Lossless scalar observations for typed array consumers.
use crate::{Complex64, Result};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scalar {
    Real(f64),
    Integer(i128),
    Logical(bool),
    Complex(Complex64),
}
impl Scalar {
    /// Only an explicitly real/logical value can enter a real-only consumer.
    /// Even complex storage with an all-zero imaginary component is not demoted.
    pub fn into_real(self) -> Result<f64> {
        match self {
            Self::Real(x) => Ok(x),
            Self::Integer(x) => Ok(x as f64),
            Self::Logical(x) => Ok(f64::from(x)),
            Self::Complex(_) => Err(
                "This operation requires a real or logical value; use real, imag or abs explicitly"
                    .into(),
            ),
        }
    }
    pub fn into_complex(self) -> Complex64 {
        match self {
            Self::Real(re) => Complex64::new(re, 0.),
            Self::Integer(x) => Complex64::new(x as f64, 0.),
            Self::Logical(x) => Complex64::new(f64::from(x), 0.),
            Self::Complex(x) => x,
        }
    }
    pub fn is_complex(self) -> bool {
        matches!(self, Self::Complex(_))
    }
}
impl From<f64> for Scalar {
    fn from(value: f64) -> Self {
        Self::Real(value)
    }
}
impl From<bool> for Scalar {
    fn from(value: bool) -> Self {
        Self::Logical(value)
    }
}
impl From<Complex64> for Scalar {
    fn from(value: Complex64) -> Self {
        Self::Complex(value)
    }
}
impl std::fmt::Display for Scalar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::Real(x) => std::fmt::Display::fmt(&x, f),
            Self::Integer(x) => std::fmt::Display::fmt(&x, f),
            // Preserve the desktop's existing numeric logical cell display.
            Self::Logical(x) => std::fmt::Display::fmt(&f64::from(x), f),
            Self::Complex(x) => {
                let sign = if x.im.is_sign_negative() { '-' } else { '+' };
                if let Some(precision) = f.precision() {
                    write!(
                        f,
                        "{:.*}{sign}{:.*}i",
                        precision,
                        x.re,
                        precision,
                        x.im.abs()
                    )
                } else {
                    write!(f, "{}{sign}{}i", x.re, x.im.abs())
                }
            }
        }
    }
}

impl Scalar {
    /// Preserve exact integer observations until a consumer explicitly requests
    /// floating point. Integer destinations round/saturate real edits as before.
    pub fn into_integer(self, class: crate::NumericClass) -> Result<i128> {
        Ok(match self {
            Self::Integer(value) => class.saturate(value),
            Self::Real(value) => class.from_f64(value),
            Self::Logical(value) => i128::from(value),
            Self::Complex(_) => {
                return Err(
                    "Complex integer and complex single values are not supported yet".into(),
                );
            }
        })
    }

    /// Bounded decimal editor literal, rounded and saturated directly in the
    /// destination integer class. Decimal fractions/exponents never visit f64.
    pub fn parse_typed_cell(text: &str, class: crate::NumericClass) -> Result<Self> {
        if text.len() > 256 {
            return Err("Invalid or oversized cell literal".into());
        }
        if !class.is_integer() {
            return Self::parse_cell(text, false, false);
        }
        let text = text.trim();
        let (negative, unsigned) = if let Some(t) = text.strip_prefix('-') {
            (true, t)
        } else {
            (false, text.strip_prefix('+').unwrap_or(text))
        };
        if unsigned.eq_ignore_ascii_case("inf") || unsigned.eq_ignore_ascii_case("infinity") {
            return Ok(Self::Integer(if negative {
                class.min()
            } else {
                class.max()
            }));
        }
        if unsigned.eq_ignore_ascii_case("nan") {
            return Ok(Self::Integer(0));
        }
        let invalid = || "Enter a decimal number, NaN or Inf".to_owned();
        let mut pieces = unsigned.split(['e', 'E']);
        let mantissa = pieces.next().unwrap_or_default();
        let exponent = match pieces.next() {
            None => 0,
            Some(e) => {
                let digits = e.strip_prefix(['+', '-']).unwrap_or(e);
                if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(invalid());
                }
                // Larger exponents already saturate every supported class.
                let magnitude = digits
                    .bytes()
                    .fold(0i32, |n, b| (n * 10 + i32::from(b - b'0')).min(10000));
                if e.starts_with('-') {
                    -magnitude
                } else {
                    magnitude
                }
            }
        };
        if pieces.next().is_some() {
            return Err(invalid());
        }
        let mut digits = [0u8; 256];
        let mut len = 0;
        let mut point = None;
        for byte in mantissa.bytes() {
            if byte == b'.' && point.is_none() {
                point = Some(len);
            } else if byte.is_ascii_digit() {
                digits[len] = byte - b'0';
                len += 1;
            } else {
                return Err(invalid());
            }
        }
        if len == 0 {
            return Err(invalid());
        }
        let integer_digits = point.unwrap_or(len) as i32 + exponent;
        let mut magnitude = 0i128;
        if integer_digits >= 0 {
            for &digit in &digits[..len.min(integer_digits as usize)] {
                magnitude = magnitude
                    .saturating_mul(10)
                    .saturating_add(i128::from(digit));
            }
            for _ in len..(integer_digits as usize).min(len + 39) {
                magnitude = magnitude.saturating_mul(10);
            }
            if (integer_digits as usize) < len && digits[integer_digits as usize] >= 5 {
                magnitude = magnitude.saturating_add(1);
            }
        }
        Ok(Self::Integer(class.saturate(if negative {
            -magnitude
        } else {
            magnitude
        })))
    }

    /// Bounded literal entry for the desktop cell editor. This is not an
    /// expression evaluator; no functions, operators or workspace reads run.
    pub fn parse_cell(text: &str, complex: bool, logical: bool) -> Result<Self> {
        if text.len() > 256 {
            return Err("Invalid or oversized cell literal".into());
        }
        let text = text.trim();
        if text.is_empty() || complex && logical {
            return Err("Invalid or oversized cell literal".into());
        }
        if logical && text.eq_ignore_ascii_case("true") {
            return Ok(Self::Logical(true));
        }
        if logical && text.eq_ignore_ascii_case("false") {
            return Ok(Self::Logical(false));
        }
        if !complex {
            return text
                .parse::<f64>()
                .map(Self::Real)
                .map_err(|_| "Enter a number, NaN or Inf".into());
        }
        let body = text.strip_suffix('i').or_else(|| text.strip_suffix('j'));
        let number = |value: &str| {
            value
                .trim()
                .parse::<f64>()
                .map_err(|_| "Enter a complex literal such as 3-4i, or a real number".to_owned())
        };
        let imaginary = |value: &str| match value.trim() {
            "" | "+" => Ok(1.),
            "-" => Ok(-1.),
            other => number(other),
        };
        let Some(body) = body else {
            return Ok(Self::Complex(Complex64::new(number(text)?, 0.)));
        };
        // A component sign is distinct from the sign in scientific notation.
        let split = body
            .as_bytes()
            .iter()
            .enumerate()
            .skip(1)
            .filter_map(|(i, &c)| {
                ((c == b'+' || c == b'-') && !matches!(body.as_bytes()[i - 1], b'e' | b'E'))
                    .then_some(i)
            })
            .next_back();
        let (re, im) = if let Some(i) = split {
            (number(&body[..i])?, imaginary(&body[i..])?)
        } else {
            (0., imaginary(body)?)
        };
        Ok(Self::Complex(Complex64::new(re, im)))
    }
}
