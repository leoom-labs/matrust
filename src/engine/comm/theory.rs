//! Proakis/Simon analytical expressions from the public berawgn expressions
//! page. Gray BER uses averaged Hamming weights, not the nearest-neighbor SER/k
//! approximation. Noncoherent MSK is the documented optimum-block upper bound.
use super::*;
use std::f64::consts::{PI, SQRT_2};
fn q(x: f64) -> f64 {
    0.5 * libm::erfc(x / SQRT_2)
}
// Composite 16-point Gauss-Legendre, recursively refined. Local relative error
// controls small tail probabilities; a 16-panel start avoids missing narrow peaks.
fn quad(e: &Engine, f: &impl Fn(f64) -> f64, a: f64, b: f64) -> Result<f64> {
    if a == b {
        return Ok(0.);
    }
    let mut sum = 0.;
    for j in 0..16 {
        e.check_cancel()?;
        let l = a + (b - a) * j as f64 / 16.;
        let r = a + (b - a) * (j + 1) as f64 / 16.;
        let whole = gauss(f, l, r);
        sum += refine(e, f, l, r, whole, 0)?;
    }
    Ok(sum)
}
const X: [f64; 8] = [
    0.09501250983763744,
    0.2816035507792589,
    0.4580167776572274,
    0.6178762444026438,
    0.755404408355003,
    0.8656312023878318,
    0.9445750230732326,
    0.9894009349916499,
];
const W: [f64; 8] = [
    0.1894506104550685,
    0.18260341504492358,
    0.16915651939500254,
    0.14959598881657673,
    0.12462897125553388,
    0.09515851168249279,
    0.06225352393864789,
    0.027152459411754096,
];
fn gauss(f: &impl Fn(f64) -> f64, a: f64, b: f64) -> f64 {
    let mid = (a + b) / 2.;
    let h = (b - a) / 2.;
    h * X
        .iter()
        .zip(W)
        .map(|(&x, w)| w * (f(mid - h * x) + f(mid + h * x)))
        .sum::<f64>()
}
fn refine(
    e: &Engine,
    f: &impl Fn(f64) -> f64,
    a: f64,
    b: f64,
    whole: f64,
    depth: usize,
) -> Result<f64> {
    e.check_cancel()?;
    let mid = (a + b) / 2.;
    let l = gauss(f, a, mid);
    let r = gauss(f, mid, b);
    let sum = l + r;
    if depth >= 12 || (sum - whole).abs() <= 2e-13 * sum.abs().max(1e-300) {
        Ok(sum)
    } else {
        Ok(refine(e, f, a, mid, l, depth + 1)? + refine(e, f, mid, b, r, depth + 1)?)
    }
}
fn weights(m: usize) -> Vec<f64> {
    (1..=m / 2)
        .map(|d| {
            let w = (0..m)
                .map(|i| {
                    let j = (i + d) % m;
                    ((i ^ (i >> 1)) ^ (j ^ (j >> 1))).count_ones() as usize
                })
                .sum::<usize>() as f64
                / m as f64;
            w * if d == m / 2 { 1. } else { 2. }
        })
        .collect()
}
fn psk(e: &Engine, g: f64, m: usize, differential: bool) -> Result<(f64, f64)> {
    let k = m.ilog2() as f64;
    let es = k * g;
    if m == 2 {
        let p = if differential {
            0.5 * (-g).exp()
        } else {
            q((2. * g).sqrt())
        };
        return Ok((p, p));
    }
    if m == 4 && !differential {
        let p = q((2. * g).sqrt());
        return Ok((p, 2. * p - p * p));
    }
    let mut ber = 0.;
    let mut ser = 0.;
    let weights = weights(m);
    for i in 1..=m / 2 {
        let left = (2 * i - 1) as f64 * PI / m as f64;
        let right = (2 * i + 1) as f64 * PI / m as f64;
        let p = if differential {
            let f = |psi: f64| -> Result<f64> {
                Ok(-psi.sin() / (4. * PI)
                    * quad(
                        e,
                        &|t| {
                            let d = 1. - psi.cos() * t.cos();
                            (-es * d).exp() / d
                        },
                        -PI / 2.,
                        PI / 2.,
                    )?)
            };
            f(right)? - f(left)?
        } else {
            let tail = |angle: f64| -> Result<f64> {
                Ok(quad(
                    e,
                    &|theta| (-es * (angle.sin() / theta.sin()).powi(2)).exp(),
                    0.,
                    PI - angle,
                )? / (2. * PI))
            };
            tail(left)? - tail(right)?
        }
        .max(0.);
        ber += weights[i - 1] * p / k;
        ser += if i == m / 2 { p } else { 2. * p };
    }
    Ok((ber, ser))
}
fn pam_bits(m: usize, t: f64) -> f64 {
    if m == 1 {
        return 0.;
    }
    let mut sum = 0.;
    for bit in 1..=m.ilog2() {
        let scale = 1usize << (bit - 1);
        let end = m - (m >> bit);
        for i in 0..end {
            let a = i * scale / m;
            let b = (2 * i * scale + m) / (2 * m);
            let coeff = if a.is_multiple_of(2) { 1. } else { -1. } * (scale - b) as f64;
            sum += coeff * q((2 * i + 1) as f64 * t);
        }
    }
    2. * sum / m as f64
}
fn i0e(x: f64) -> f64 {
    if x > 500. {
        let mut term = 1.;
        let mut sum = 1.;
        for k in 1..=16 {
            term *= ((2 * k - 1) * (2 * k - 1)) as f64 / (8. * x * k as f64);
            sum += term;
        }
        return sum / (2. * PI * x).sqrt();
    }
    let mut term = 1.;
    let mut sum = 1.;
    for k in 1..1000 {
        term *= x * x / (4. * (k * k) as f64);
        sum += term;
        if term < sum * 2e-16 {
            break;
        }
    }
    sum * (-x).exp()
}
fn rice(r: f64, a: f64) -> f64 {
    r * (-0.5 * (r - a).powi(2)).exp() * i0e(r * a)
}
fn marcum(e: &Engine, a: f64, b: f64) -> Result<f64> {
    quad(e, &|r| rice(r, a), b, a.max(b) + 14.)
}
fn msk_bound(e: &Engine, g: f64) -> Result<f64> {
    let component = |c: f64| -> Result<f64> {
        let a = (g * (1. - c)).sqrt();
        let b = (g * (1. + c)).sqrt();
        // Marcum symmetry removes subtraction of a probability near one.
        Ok(2. * marcum(e, a, b)? - (-0.5 * (a - b).powi(2)).exp() * i0e(a * b))
    };
    Ok(0.5 * component(((3. - 4. / (PI * PI)) / 4.).sqrt())?
        + 0.25 * component((1. - 4. / (PI * PI)).sqrt())?
        + 0.5 * (-g).exp())
}
pub(super) fn call(e: &mut Engine, n: &str, a: &[Value]) -> Result<Vec<Value>> {
    if n != "berawgn" {
        arity(n, a, 1, 1)?;
        let x = numeric(e, &a[0], n)?;
        let dims = x.dims().to_vec();
        preflight(e, &dims, 1)?;
        let mut data = Vec::with_capacity(x.len());
        for i in 0..x.len() {
            tick(e, i)?;
            let v = real(&x, i)?;
            if n == "qfuncinv" && !(0. ..=1.).contains(&v) && !v.is_nan() {
                return Err(err(
                    n,
                    "InvalidProbability",
                    "Probabilities must lie in [0,1].",
                ));
            }
            data.push(if n == "qfunc" {
                q(v)
            } else {
                SQRT_2 * super::super::number_theory::erfcinv_fn(2. * v)
            });
        }
        return Ok(vec![output(
            &dims,
            data,
            if is_single(&x) { "single" } else { "double" },
        )?]);
    }
    arity(n, a, 3, 5)?;
    let x = numeric(e, &a[0], n)?;
    if x.dims().len() > 2 || (x.rows() > 1 && x.cols() > 1) || x.len() > 65536 {
        return Err(err(
            n,
            "InvalidEbNo",
            "EbNo must be a vector of at most 65536 values.",
        ));
    }
    let family = choose(
        e,
        n,
        &a[1],
        &["psk", "oqpsk", "dpsk", "fsk", "msk", "pam", "qam"],
    )?;
    let m = if family == "oqpsk" {
        4
    } else if family == "msk" {
        2
    } else {
        count(e, &a[2], n, 2, 1024)?
    };
    if !m.is_power_of_two() || (family == "qam" && m < 4) {
        return Err(err(
            n,
            "InvalidM",
            "M must be a power of 2 (at least 4 for QAM).",
        ));
    }
    let mut diff = false;
    let mut coherent = true;
    match family.as_str() {
        "psk" => {
            arity(n, a, 3, 4)?;
            if let Some(v) = a.get(3) {
                diff = choose(e, n, v, &["diff", "nondiff"])? == "diff";
            }
            if diff && m > 4 {
                return Err(err(n, "InvalidM", "Differential PSK requires M=2 or M=4."));
            }
        }
        "oqpsk" => {
            arity(n, a, 3, 3)?;
            diff = choose(e, n, &a[2], &["diff", "nondiff"])? == "diff";
        }
        "msk" => {
            arity(n, a, 3, 4)?;
            diff = choose(e, n, &a[2], &["on", "off"])? == "off";
            if let Some(v) = a.get(3) {
                coherent = choose(e, n, v, &["coherent", "noncoherent"])? == "coherent";
            }
        }
        "fsk" => {
            arity(n, a, 4, 4)?;
            coherent = choose(e, n, &a[3], &["coherent", "noncoherent"])? == "coherent";
            if !coherent && m > 64 {
                return Err(err(n, "InvalidM", "Noncoherent FSK requires M in [2,64]."));
            }
        }
        _ => arity(n, a, 3, 3)?,
    }
    let dims = x.dims().to_vec();
    preflight(e, &dims, 2)?;
    let work = x.len().saturating_mul(m).saturating_mul(m);
    if work > 16_777_216 {
        return Err(err(
            n,
            "WorkLimit",
            "Analytical BER exceeds the integration work limit.",
        ));
    }
    e.charge(work)?;
    let mut ber = Vec::with_capacity(x.len());
    let mut ser = Vec::with_capacity(x.len());
    for i in 0..x.len() {
        e.check_cancel()?;
        let db = real(&x, i)?;
        if db.is_nan() {
            return Err(err(n, "InvalidEbNo", "EbNo must not contain NaN."));
        }
        let g = 10f64.powf(db / 10.);
        let k = m.ilog2() as f64;
        let (b, s) = if g.is_infinite() {
            (0., 0.)
        } else {
            match family.as_str() {
                "psk" | "oqpsk" => {
                    if diff {
                        let q = q((2. * g).sqrt());
                        let b = 2. * q * (1. - q);
                        (
                            b,
                            if m == 2 {
                                b
                            } else {
                                4. * q - 8. * q * q + 8. * q.powi(3) - 4. * q.powi(4)
                            },
                        )
                    } else {
                        psk(e, g, m, false)?
                    }
                }
                "dpsk" => psk(e, g, m, true)?,
                "pam" => {
                    let t = (6. * k * g / (m * m - 1) as f64).sqrt();
                    (pam_bits(m, t) / k, 2. * (m - 1) as f64 / m as f64 * q(t))
                }
                "qam" => {
                    let i = 1usize << (m.ilog2() / 2);
                    let j = m / i;
                    let t = (6. * k * g / (i * i + j * j - 2) as f64).sqrt();
                    let pi = 2. * (i - 1) as f64 / i as f64 * q(t);
                    let pj = 2. * (j - 1) as f64 / j as f64 * q(t);
                    ((pam_bits(i, t) + pam_bits(j, t)) / k, pi + pj - pi * pj)
                }
                "msk" => {
                    let p = if !coherent {
                        msk_bound(e, g)?
                    } else {
                        let p = q((2. * g).sqrt());
                        if diff { 2. * p * (1. - p) } else { p }
                    };
                    (p, p)
                }
                _ => {
                    let s = if m == 2 {
                        if coherent {
                            q(g.sqrt())
                        } else {
                            0.5 * (-g / 2.).exp()
                        }
                    } else if coherent {
                        let shift = (2. * k * g).sqrt();
                        quad(
                            e,
                            &|t| {
                                let tail = q(t + shift);
                                let fail = if tail >= 1. {
                                    1.
                                } else {
                                    -((m - 1) as f64 * (-tail).ln_1p()).exp_m1()
                                };
                                fail * (-t * t / 2.).exp() / (2. * PI).sqrt()
                            },
                            -14.,
                            14.,
                        )?
                    } else {
                        let a = (2. * k * g).sqrt();
                        quad(
                            e,
                            &|r| {
                                let fail = if r == 0. {
                                    1.
                                } else {
                                    -((m - 1) as f64 * (-(-r * r / 2.).exp()).ln_1p()).exp_m1()
                                };
                                rice(r, a) * fail
                            },
                            0.,
                            a + 14.,
                        )?
                    };
                    (m as f64 / (2. * (m - 1) as f64) * s, s)
                }
            }
        };
        ber.push(b.max(0.));
        ser.push(s.max(0.));
    }
    Ok(vec![
        output(&dims, ber, "double")?,
        output(&dims, ser, "double")?,
    ])
}
