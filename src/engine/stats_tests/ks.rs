//! MTW (JSS 8(18), 2003), full matrix power; Pelz-Good and Smirnov sums
//! as printed in Simard & L'Ecuyer (JSS 39(11), 2011), equations (6),(7).
use super::*;
use std::f64::consts::PI;
#[path = "ks_precision.rs"]
mod precision;
fn multiply(e: &Engine, a: &(Matrix, f64), b: &(Matrix, f64)) -> Result<(Matrix, f64)> {
    e.check_cancel()?;
    let m = a.0.mul(&b.0)?;
    e.check_cancel()?;
    let largest = m.data().iter().copied().fold(0., f64::max);
    let scale = if largest > 1e100 { largest.ln() } else { 0. };
    let m = if scale != 0. {
        Matrix::from_column_vec(
            m.rows(),
            m.cols(),
            m.data().iter().map(|v| v / largest).collect(),
        )?
    } else {
        m
    };
    Ok((m, a.1 + b.1 + scale))
}
fn mtw(e: &Engine, n: usize, d: f64) -> Result<f64> {
    let nf = n as f64;
    let k = (nf * d).floor() as usize + 1;
    let m = 2 * k - 1;
    let h = k as f64 - nf * d;
    let mut invfact = vec![1.; m + 1];
    for i in 1..=m {
        invfact[i] = invfact[i - 1] / i as f64;
    }
    let mut data = vec![0.; m * m];
    for j in 0..m {
        for i in 0..m {
            poll(e, i + j * m)?;
            if i + 1 >= j {
                data[i + j * m] = invfact[i + 1 - j]
            }
        }
    }
    for i in 0..m {
        data[i] -= h.powi((i + 1) as i32) * invfact[i + 1];
        data[m - 1 + i * m] -= h.powi((m - i) as i32) * invfact[m - i];
    }
    if 2. * h > 1. {
        data[m - 1] += (2. * h - 1.).powi(m as i32) * invfact[m];
    }
    let mut base = (Matrix::from_column_vec(m, m, data)?, 0.);
    let mut power = n;
    let mut result: Option<(Matrix, f64)> = None;
    while power > 0 {
        e.check_cancel()?;
        if power & 1 == 1 {
            result = Some(if let Some(r) = result {
                multiply(e, &r, &base)?
            } else {
                base.clone()
            })
        }
        power >>= 1;
        if power > 0 {
            base = multiply(e, &base, &base)?
        }
    }
    let (r, log) = result.unwrap();
    let pref = if n < 100 {
        (1..=n).map(|i| (i as f64 / nf).ln()).sum::<f64>()
    } else {
        0.5 * (2. * PI * nf).ln() - nf + 1. / (12. * nf) - 1. / (360. * nf.powi(3))
            + 1. / (1260. * nf.powi(5))
    };
    let logcdf = r.data()[k - 1 + (k - 1) * m].ln() + log + pref;
    Ok((-logcdf.exp_m1()).clamp(0., 1.))
}
fn pelz(e: &Engine, n: usize, d: f64) -> Result<f64> {
    let nf = n as f64;
    let z = nf.sqrt() * d;
    if z < 0.05 {
        return Ok(1.);
    }
    if z > 20. {
        return Ok(0.);
    }
    if z > 1.2 {
        // Poisson-transform equation (6) before taking the complement, avoiding
        // cancellation of probabilities near one. K and L below omit their
        // constant terms; primes are derivatives of the theta series.
        let mut k = [0.; 4];
        let mut l = [0.; 3];
        for j in 1..=32 {
            poll(e, j)?;
            let b = (j * j) as f64;
            let q = (-2. * b * z * z).exp();
            let derivatives = [
                q,
                -4. * b * z * q,
                (16. * b * b * z * z - 4. * b) * q,
                (48. * b * b * z - 64. * b * b * b * z.powi(3)) * q,
            ];
            let sign = if j % 2 == 1 { -2. } else { 2. };
            for i in 0..4 {
                k[i] += sign * derivatives[i];
                if i < 3 {
                    l[i] += 2. * derivatives[i];
                }
            }
        }
        let k1 = k[1] / 6.;
        let k2 =
            (k[0] - l[0]) / 36. - z * k[1] / 9. - z * l[1] / 36. + (1. - 2. * z * z) * k[2] / 72.;
        let k3 = (-26. * k[1] - 148. * z * k[2] + (5. - 30. * z * z) * k[3]) / 6480.
            - (2. * l[1] + z * l[2]) / 216.;
        return Ok((-k[0] - k1 / nf.sqrt() - k2 / nf - k3 / nf.powf(1.5)).clamp(0., 1.));
    }
    let z2 = z * z;
    let mut sums = [0.; 4];
    let mut integers = [0.; 2];
    // Symmetric half-integer and integer sums; exponential truncation < 1e-32.
    let limit = ((80. * 2. * z2).sqrt() / PI).ceil() as usize + 2;
    for k in 0..limit {
        poll(e, k)?;
        let u = PI.powi(2) * (k as f64 + 0.5).powi(2);
        let q = (-u / (2. * z2)).exp();
        sums[0] += q;
        sums[1] += (u - z2) * q;
        sums[2] += ((6. * z.powi(6) + 2. * z.powi(4))
            + u * (2. * z.powi(4) - 5. * z2)
            + u * u * (1. - 2. * z2))
            * q;
        sums[3] += (u.powi(3) * (5. - 30. * z2)
            + u * u * (-60. * z2 + 212. * z.powi(4))
            + u * (135. * z.powi(4) - 96. * z.powi(6))
            - (30. * z.powi(6) + 90. * z.powi(8)))
            * q;
        let v = PI.powi(2) * (k as f64 + 1.).powi(2);
        let q = (-v / (2. * z2)).exp();
        integers[0] += v * q;
        integers[1] += (-v * v + 3. * v * z2) * q;
    }
    let root = (2. * PI).sqrt();
    let cdf = root / z * sums[0]
        + root / (6. * z.powi(4) * nf.sqrt()) * sums[1]
        + (root / (72. * z.powi(7)) * sums[2] - root / (36. * z.powi(3)) * integers[0]) / nf
        + (root / (6480. * z.powi(10)) * sums[3] + root / (216. * z.powi(6)) * integers[1])
            / nf.powf(1.5);
    Ok((1. - cdf).clamp(0., 1.))
}
fn one(e: &Engine, n: usize, d: f64) -> Result<f64> {
    if d <= 0. {
        return Ok(1.);
    }
    if d >= 1. {
        return Ok(0.);
    }
    let nf = n as f64;
    let max = (nf * (1. - d)).floor() as usize;
    let mut logchoose = 0.;
    let mut log_error = 0.;
    let mut total = 0.;
    let mut sum_error = 0.;
    for j in 0..=max {
        poll(e, j)?;
        if j > 0 {
            let term = ((n - j + 1) as f64 / j as f64).ln() - log_error;
            let next = logchoose + term;
            log_error = (next - logchoose) - term;
            logchoose = next;
        }
        let a = j as f64 / nf + d;
        let b = 1. - d - j as f64 / nf;
        let exponent = logchoose
            + (j as f64 - 1.) * a.ln()
            + if n == j { 0. } else { (n - j) as f64 * b.ln() };
        let term = (d.ln() + exponent).exp() - sum_error;
        let next = total + term;
        sum_error = (next - total) - term;
        total = next;
    }
    Ok(total.clamp(0., 1.))
}
pub(super) fn sf(e: &Engine, n: usize, d: f64, two: bool) -> Result<f64> {
    if n == 0 || d.is_nan() {
        return Ok(f64::NAN);
    }
    if d <= 0. {
        return Ok(1.);
    }
    if d >= 1. {
        return Ok(0.);
    }
    if !two {
        return one(e, n, d);
    }
    if d <= 0.5 / n as f64 {
        return Ok(1.);
    }
    if d >= 0.5 {
        // D+ >= d and D- >= d cannot both hold, so the two tails add exactly.
        return Ok((2. * one(e, n, d)?).min(1.));
    }
    let k = (n as f64 * d).floor() as usize + 1;
    if 2 * k - 1 <= 1001 {
        let p = mtw(e, n, d)?;
        // The multi-limb matrix power costs ~limbs^2 * m^3 * log2(n); past the
        // budget, a tail below 1e-10 is twice the one-sided tail to well
        // within 1e-9 relative (the joint term is O(p^3)), else the f64 MTW value.
        let m = (2 * k - 1) as f64;
        let budget_ok = m.powi(3) * (n as f64).log2().max(1.) <= 4e7;
        if p < 0.001 && !budget_ok {
            let two_one = 2. * one(e, n, d)?;
            return Ok(if two_one < 1e-10 { two_one } else { p });
        }
        if p < 0.001 {
            // Smirnov's one-sided sum selects working precision only. The
            // returned probability still comes from the full MTW matrix.
            let digits = -(2. * one(e, n, d)?).log10();
            match digits {
                x if x <= 12. => precision::sf::<2>(e, n, d),
                x if x <= 27. => precision::sf::<3>(e, n, d),
                x if x <= 42. => precision::sf::<4>(e, n, d),
                x if x <= 72. => precision::sf::<6>(e, n, d),
                x if x <= 102. => precision::sf::<8>(e, n, d),
                x if x <= 162. => precision::sf::<12>(e, n, d),
                x if x <= 222. => precision::sf::<16>(e, n, d),
                _ => precision::sf::<24>(e, n, d),
            }
        } else {
            Ok(p)
        }
    } else {
        pelz(e, n, d)
    }
}
pub(super) fn test(e: &Engine, args: &[Value], count: usize) -> Result<Vec<Value>> {
    let (end, o) = Opts::parse(args, 1, &["Alpha", "Tail", "CDF"])?;
    if end > 4 {
        return Err("kstest: too many inputs".into());
    }
    let mut a = o.num("alpha", 0.05, e)?;
    if end > 2 && !empty(&args[2]) {
        a = scalar(&args[2], e)?
    }
    alpha(a)?;
    let mut t = o.word("tail", "unequal")?;
    if end > 3 && !empty(&args[3]) {
        t = text(&args[3])
            .ok_or("kstest: Tail must be text")?
            .to_ascii_lowercase()
    }
    if !["unequal", "larger", "smaller"].contains(&t.as_str()) {
        return Err("kstest: Tail must be unequal, larger or smaller".into());
    }
    let two = t == "unequal";
    let cdf = o.get("cdf").or_else(|| {
        if end > 1 && !empty(&args[1]) {
            Some(&args[1])
        } else {
            None
        }
    });
    let cdf = if let Some(c) = cdf {
        let m = mat(c, e).map_err(|_| "kstest: distribution objects are not supported yet")?;
        if m.cols() != 2 || m.rows() < 2 {
            return Err("kstest: CDF must have two columns".into());
        }
        for i in 0..m.rows() {
            let f = m.data()[i + m.rows()];
            if !m.data()[i].is_finite()
                || !f.is_finite()
                || !(0. ..=1.).contains(&f)
                || i > 0 && (m.data()[i] <= m.data()[i - 1] || f < m.data()[i - 1 + m.rows()])
            {
                return Err("kstest: invalid CDF table".into());
            }
        }
        Some(m)
    } else {
        None
    };
    let mut x = vector(&args[0], e)?;
    sort(e, &mut x, f64::total_cmp)?;
    let n = x.len();
    let mut plus: f64 = 0.;
    let mut minus: f64 = 0.;
    for (i, &x) in x.iter().enumerate() {
        poll(e, i)?;
        let f = if let Some(m) = &cdf {
            let xs = &m.data()[..m.rows()];
            if x < xs[0] || x > xs[xs.len() - 1] {
                return Err("kstest: observation outside CDF range".into());
            }
            let j = xs.partition_point(|v| *v < x).clamp(1, xs.len() - 1);
            let f0 = m.data()[m.rows() + j - 1];
            f0 + (x - xs[j - 1]) / (xs[j] - xs[j - 1]) * (m.data()[m.rows() + j] - f0)
        } else {
            Family::Norm.cdf(x, &[0., 1.], false)
        };
        plus = plus.max((i + 1) as f64 / n as f64 - f);
        minus = minus.max(f - i as f64 / n as f64);
    }
    let stat = if two {
        plus.max(minus)
    } else if t == "larger" {
        plus
    } else {
        minus
    };
    let p = sf(e, n, stat, two)?;
    let cv = if n == 0
        || count < 4
        || if two {
            !(0.01..=0.2).contains(&a)
        } else {
            !(0.005..=0.1).contains(&a)
        } {
        f64::NAN
    } else {
        let mut lo = 0.;
        let mut hi = (-((if two { a / 2. } else { a }).ln()) / (2. * n as f64))
            .sqrt()
            .min(1.);
        for _ in 0..45 {
            e.check_cancel()?;
            let mid = (lo + hi) / 2.;
            if sf(e, n, mid, two)? > a {
                lo = mid
            } else {
                hi = mid
            }
        }
        (lo + hi) / 2.
    };
    Ok(vec![
        logical(p < a),
        val(p),
        val(if n == 0 { f64::NAN } else { stat }),
        val(cv),
    ])
}
#[cfg(test)]
mod stats_tests {
    use super::*;
    #[test]
    fn mtw_published_example() {
        let e = Engine::default();
        assert!((sf(&e, 10, 0.274, true).unwrap() - 0.3715203845434957).abs() < 1e-13);
    }
}
