//! Library models of the curve fitting functions, as the public "List of
//! Library Models" names them: coefficient names and order, formulas, values
//! and analytic derivatives with respect to the coefficients.
use crate::Result;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine) enum Lib {
    /// poly1..poly9: p1*x^n + ... + p(n+1).
    Poly(usize),
    /// polyij surfaces: terms x^a*y^b with a <= i, b <= j, a+b <= max(i, j).
    PolySurf(usize, usize),
    Exp(usize),
    Fourier(usize),
    Gauss(usize),
    Power(usize),
    /// ratmn: numerator degree m, monic denominator of degree n.
    Rat(usize, usize),
    Sin(usize),
    Weibull,
}

/// The library model `name` names (case-insensitive). Ok(None) for text that
/// is not a library-style name (a custom equation); an error for a library
/// family with a number outside its range.
pub(super) fn parse(name: &str) -> Result<Option<Lib>> {
    let lower = name.trim().to_ascii_lowercase();
    if lower == "weibull" {
        return Ok(Some(Lib::Weibull));
    }
    for family in ["fourier", "gauss", "power", "poly", "exp", "rat", "sin"] {
        let Some(digits) = lower.strip_prefix(family) else {
            continue;
        };
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let d: Vec<usize> = digits.bytes().map(|b| (b - b'0') as usize).collect();
        let lib = match (family, d.as_slice()) {
            ("poly", &[n]) if (1..=9).contains(&n) => Some(Lib::Poly(n)),
            ("poly", &[i, j]) if i <= 5 && j <= 5 => Some(Lib::PolySurf(i, j)),
            ("exp", &[n]) if (1..=2).contains(&n) => Some(Lib::Exp(n)),
            ("fourier", &[n]) if (1..=8).contains(&n) => Some(Lib::Fourier(n)),
            ("gauss", &[n]) if (1..=8).contains(&n) => Some(Lib::Gauss(n)),
            ("power", &[n]) if (1..=2).contains(&n) => Some(Lib::Power(n)),
            ("rat", &[m, n]) if m <= 5 && (1..=5).contains(&n) => Some(Lib::Rat(m, n)),
            ("sin", &[n]) if (1..=8).contains(&n) => Some(Lib::Sin(n)),
            _ => None,
        };
        return match lib {
            Some(lib) => Ok(Some(lib)),
            None => Err(format!(
                "Unknown library model '{name}'. Library models are poly1..poly9, poly00..poly55, exp1, exp2, fourier1..fourier8, gauss1..gauss8, power1, power2, rat01..rat55, sin1..sin8 and weibull."
            )),
        };
    }
    Ok(None)
}

/// Surface polynomial terms (x power, y power) in coefficient order: by total
/// degree, then by decreasing x power.
pub(super) fn surf_terms(i: usize, j: usize) -> Vec<(usize, usize)> {
    let mut terms = Vec::new();
    for degree in 0..=i.max(j) {
        for xp in (0..=degree).rev() {
            let yp = degree - xp;
            if xp <= i && yp <= j {
                terms.push((xp, yp));
            }
        }
    }
    terms
}

fn power_text(var: &str, p: usize) -> String {
    match p {
        0 => String::new(),
        1 => var.into(),
        _ => format!("{var}^{p}"),
    }
}

impl Lib {
    pub(super) fn name(self) -> String {
        match self {
            Lib::Poly(n) => format!("poly{n}"),
            Lib::PolySurf(i, j) => format!("poly{i}{j}"),
            Lib::Exp(n) => format!("exp{n}"),
            Lib::Fourier(n) => format!("fourier{n}"),
            Lib::Gauss(n) => format!("gauss{n}"),
            Lib::Power(n) => format!("power{n}"),
            Lib::Rat(m, n) => format!("rat{m}{n}"),
            Lib::Sin(n) => format!("sin{n}"),
            Lib::Weibull => "weibull".into(),
        }
    }
    pub(super) fn surface(self) -> bool {
        matches!(self, Lib::PolySurf(..))
    }
    /// Linear in the coefficients: solved by linear least squares.
    pub(super) fn linear(self) -> bool {
        matches!(self, Lib::Poly(_) | Lib::PolySurf(..))
    }
    pub(super) fn coeffs(self) -> Vec<String> {
        let names = |list: &[&str]| list.iter().map(|s| (*s).to_owned()).collect();
        match self {
            Lib::Poly(n) => (1..=n + 1).map(|k| format!("p{k}")).collect(),
            Lib::PolySurf(i, j) => surf_terms(i, j)
                .into_iter()
                .map(|(a, b)| format!("p{a}{b}"))
                .collect(),
            Lib::Exp(1) | Lib::Power(1) | Lib::Weibull => names(&["a", "b"]),
            Lib::Exp(_) => names(&["a", "b", "c", "d"]),
            Lib::Power(_) => names(&["a", "b", "c"]),
            Lib::Fourier(n) => {
                let mut v = vec!["a0".to_owned()];
                for k in 1..=n {
                    v.push(format!("a{k}"));
                    v.push(format!("b{k}"));
                }
                v.push("w".into());
                v
            }
            Lib::Gauss(n) | Lib::Sin(n) => (1..=n)
                .flat_map(|k| [format!("a{k}"), format!("b{k}"), format!("c{k}")])
                .collect(),
            Lib::Rat(m, n) => (1..=m + 1)
                .map(|k| format!("p{k}"))
                .chain((1..=n).map(|k| format!("q{k}")))
                .collect(),
        }
    }
    pub(super) fn formula(self) -> String {
        match self {
            Lib::Poly(n) => (0..=n)
                .map(|k| {
                    let p = power_text("x", n - k);
                    if p.is_empty() {
                        format!("p{}", k + 1)
                    } else {
                        format!("p{}*{p}", k + 1)
                    }
                })
                .collect::<Vec<_>>()
                .join(" + "),
            Lib::PolySurf(i, j) => surf_terms(i, j)
                .into_iter()
                .map(|(a, b)| {
                    let mut t = format!("p{a}{b}");
                    for part in [power_text("x", a), power_text("y", b)] {
                        if !part.is_empty() {
                            t.push('*');
                            t.push_str(&part);
                        }
                    }
                    t
                })
                .collect::<Vec<_>>()
                .join(" + "),
            Lib::Exp(1) => "a*exp(b*x)".into(),
            Lib::Exp(_) => "a*exp(b*x) + c*exp(d*x)".into(),
            Lib::Fourier(n) => {
                let mut t = "a0".to_owned();
                for k in 1..=n {
                    let arg = if k == 1 {
                        "x*w".to_owned()
                    } else {
                        format!("{k}*x*w")
                    };
                    t.push_str(&format!(" + a{k}*cos({arg}) + b{k}*sin({arg})"));
                }
                t
            }
            Lib::Gauss(n) => (1..=n)
                .map(|k| format!("a{k}*exp(-((x-b{k})/c{k})^2)"))
                .collect::<Vec<_>>()
                .join(" + "),
            Lib::Power(1) => "a*x^b".into(),
            Lib::Power(_) => "a*x^b + c".into(),
            Lib::Rat(m, n) => {
                let num = (0..=m)
                    .map(|k| {
                        let p = power_text("x", m - k);
                        if p.is_empty() {
                            format!("p{}", k + 1)
                        } else {
                            format!("p{}*{p}", k + 1)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" + ");
                let mut den = power_text("x", n);
                for k in 1..=n {
                    let p = power_text("x", n - k);
                    den.push_str(&if p.is_empty() {
                        format!(" + q{k}")
                    } else {
                        format!(" + q{k}*{p}")
                    });
                }
                format!("({num}) / ({den})")
            }
            Lib::Sin(n) => (1..=n)
                .map(|k| format!("a{k}*sin(b{k}*x+c{k})"))
                .collect::<Vec<_>>()
                .join(" + "),
            Lib::Weibull => "a*b*x^(b-1)*exp(-a*x^b)".into(),
        }
    }
    /// Default bounds of the library model: Gaussian widths and sine
    /// frequencies are nonnegative; everything else is unbounded.
    pub(super) fn default_lower(self) -> Vec<f64> {
        let p = self.coeffs().len();
        match self {
            Lib::Gauss(_) | Lib::Sin(_) => (0..p)
                .map(|k| match (self, k % 3) {
                    (Lib::Gauss(_), 2) | (Lib::Sin(_), 1) => 0.,
                    _ => f64::NEG_INFINITY,
                })
                .collect(),
            _ => Vec::new(),
        }
    }
    /// Model value at one point.
    pub(super) fn value(self, c: &[f64], x: f64, y: f64) -> f64 {
        match self {
            Lib::Poly(_) => c.iter().fold(0., |acc, &p| acc * x + p),
            Lib::PolySurf(i, j) => surf_terms(i, j)
                .iter()
                .zip(c)
                .map(|(&(a, b), &p)| p * x.powi(a as i32) * y.powi(b as i32))
                .sum(),
            Lib::Exp(1) => c[0] * (c[1] * x).exp(),
            Lib::Exp(_) => c[0] * (c[1] * x).exp() + c[2] * (c[3] * x).exp(),
            Lib::Fourier(n) => {
                let w = c[2 * n + 1];
                let mut v = c[0];
                for k in 1..=n {
                    let t = k as f64 * w * x;
                    v += c[2 * k - 1] * t.cos() + c[2 * k] * t.sin();
                }
                v
            }
            Lib::Gauss(n) => (0..n)
                .map(|k| {
                    let u = (x - c[3 * k + 1]) / c[3 * k + 2];
                    c[3 * k] * (-u * u).exp()
                })
                .sum(),
            Lib::Power(n) => c[0] * x.powf(c[1]) + if n == 2 { c[2] } else { 0. },
            Lib::Rat(m, n) => {
                let p = c[..=m].iter().fold(0., |acc, &v| acc * x + v);
                let q = c[m + 1..].iter().fold(1., |acc, &v| acc * x + v);
                debug_assert_eq!(c.len(), m + 1 + n);
                p / q
            }
            Lib::Sin(n) => (0..n)
                .map(|k| c[3 * k] * (c[3 * k + 1] * x + c[3 * k + 2]).sin())
                .sum(),
            Lib::Weibull => {
                let (a, b) = (c[0], c[1]);
                a * b * x.powf(b - 1.) * (-a * x.powf(b)).exp()
            }
        }
    }
    /// Derivatives of the model value with respect to each coefficient.
    pub(super) fn gradient(self, c: &[f64], x: f64, y: f64, g: &mut [f64]) {
        // x*ln(x) style factors are 0 where the power itself vanishes.
        let log_times = |base: f64, factor: f64| {
            if factor == 0. { 0. } else { factor * base.ln() }
        };
        match self {
            Lib::Poly(n) => {
                let mut power = 1.;
                for k in (0..=n).rev() {
                    g[k] = power;
                    power *= x;
                }
            }
            Lib::PolySurf(i, j) => {
                for (k, (a, b)) in surf_terms(i, j).into_iter().enumerate() {
                    g[k] = x.powi(a as i32) * y.powi(b as i32);
                }
            }
            Lib::Exp(n) => {
                for k in 0..n {
                    let e = (c[2 * k + 1] * x).exp();
                    g[2 * k] = e;
                    g[2 * k + 1] = c[2 * k] * x * e;
                }
            }
            Lib::Fourier(n) => {
                let w = c[2 * n + 1];
                g[0] = 1.;
                let mut dw = 0.;
                for k in 1..=n {
                    let kf = k as f64;
                    let (s, co) = (kf * w * x).sin_cos();
                    g[2 * k - 1] = co;
                    g[2 * k] = s;
                    dw += kf * x * (-c[2 * k - 1] * s + c[2 * k] * co);
                }
                g[2 * n + 1] = dw;
            }
            Lib::Gauss(n) => {
                for k in 0..n {
                    let (a, b, w) = (c[3 * k], c[3 * k + 1], c[3 * k + 2]);
                    let u = (x - b) / w;
                    let e = (-u * u).exp();
                    g[3 * k] = e;
                    g[3 * k + 1] = a * e * 2. * u / w;
                    g[3 * k + 2] = a * e * 2. * u * u / w;
                }
            }
            Lib::Power(n) => {
                let xb = x.powf(c[1]);
                g[0] = xb;
                g[1] = log_times(x, c[0] * xb);
                if n == 2 {
                    g[2] = 1.;
                }
            }
            Lib::Rat(m, n) => {
                let p = c[..=m].iter().fold(0., |acc, &v| acc * x + v);
                let q = c[m + 1..].iter().fold(1., |acc, &v| acc * x + v);
                let mut power = 1.;
                for k in (0..=m).rev() {
                    g[k] = power / q;
                    power *= x;
                }
                let mut power = 1.;
                for k in (0..n).rev() {
                    g[m + 1 + k] = -p * power / (q * q);
                    power *= x;
                }
            }
            Lib::Sin(n) => {
                for k in 0..n {
                    let (s, co) = (c[3 * k + 1] * x + c[3 * k + 2]).sin_cos();
                    g[3 * k] = s;
                    g[3 * k + 1] = c[3 * k] * x * co;
                    g[3 * k + 2] = c[3 * k] * co;
                }
            }
            Lib::Weibull => {
                let (a, b) = (c[0], c[1]);
                let xb = x.powf(b);
                let base = x.powf(b - 1.) * (-a * xb).exp();
                g[0] = b * base * (1. - a * xb);
                g[1] = a * base * (1. + log_times(x, b) - log_times(x, a * b * xb));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_counts_follow_the_library_list() {
        assert_eq!(parse("poly3").unwrap(), Some(Lib::Poly(3)));
        assert_eq!(parse("Poly22").unwrap(), Some(Lib::PolySurf(2, 2)));
        assert_eq!(parse("rat23").unwrap(), Some(Lib::Rat(2, 3)));
        assert!(parse("gauss9").is_err() && parse("rat60").is_err() && parse("poly0").is_err());
        assert_eq!(parse("a*exp(b*x)").unwrap(), None);
        assert_eq!(
            Lib::Fourier(2).coeffs(),
            ["a0", "a1", "b1", "a2", "b2", "w"]
        );
        assert_eq!(
            Lib::PolySurf(2, 1).coeffs(),
            ["p00", "p10", "p01", "p20", "p11"]
        );
        assert_eq!(
            Lib::PolySurf(1, 3).coeffs(),
            ["p00", "p10", "p01", "p11", "p02", "p12", "p03"]
        );
        assert_eq!(Lib::Rat(1, 2).coeffs(), ["p1", "p2", "q1", "q2"]);
        assert_eq!(Lib::Rat(1, 2).formula(), "(p1*x + p2) / (x^2 + q1*x + q2)");
        assert_eq!(Lib::Poly(2).formula(), "p1*x^2 + p2*x + p3");
    }

    #[test]
    fn analytic_gradients_match_central_differences() {
        let cases: Vec<(Lib, Vec<f64>)> = vec![
            (Lib::Exp(2), vec![1.5, -0.4, 0.7, 0.2]),
            (Lib::Fourier(2), vec![0.3, 1.1, -0.6, 0.25, 0.4, 1.7]),
            (Lib::Gauss(2), vec![2., 1., 0.8, -1., 3., 1.2]),
            (Lib::Power(2), vec![1.3, 0.7, 0.2]),
            (Lib::Rat(2, 2), vec![0.5, -1., 2., 0.3, 4.]),
            (Lib::Sin(2), vec![1.2, 2.1, 0.3, 0.4, 5., -1.]),
            (Lib::Weibull, vec![0.4, 1.8]),
            (Lib::PolySurf(2, 2), vec![1., 2., 3., 4., 5., 6.]),
        ];
        for (lib, c) in cases {
            for &x in &[0.4, 1.3, 2.7] {
                let mut g = vec![0.; c.len()];
                lib.gradient(&c, x, 0.6, &mut g);
                for k in 0..c.len() {
                    let h = 1e-6 * c[k].abs().max(1.);
                    let (mut up, mut dn) = (c.clone(), c.clone());
                    up[k] += h;
                    dn[k] -= h;
                    let d = (lib.value(&up, x, 0.6) - lib.value(&dn, x, 0.6)) / (2. * h);
                    assert!(
                        (d - g[k]).abs() < 1e-6 * (1. + d.abs()),
                        "{lib:?} {k}: {d} {}",
                        g[k]
                    );
                }
            }
        }
    }
}
