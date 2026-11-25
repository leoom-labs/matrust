use super::*;
pub(super) fn design(e: &mut Engine, a: &[Value]) -> Result<Value> {
    arity(a, 3, 4, "rcosdesign")?;
    let beta = scalar(e, &a[0], "rcosdesign beta")?;
    if !(0. ..=1.).contains(&beta) {
        return Err("rcosdesign: beta must be in [0,1]".into());
    }
    let span = integer(e, &a[1], 1, MAX_SAMPLES, "rcosdesign span")?;
    let sps = integer(e, &a[2], 1, MAX_SAMPLES, "rcosdesign sps")?;
    let n = span
        .checked_mul(sps)
        .filter(|n| *n < MAX_SAMPLES)
        .ok_or("rcosdesign: filter length limit exceeded")?;
    if n % 2 != 0 {
        return Err("rcosdesign: span*sps must be even".into());
    }
    let shape = choice(e, a.get(3), "sqrt", &["sqrt", "normal"], "rcosdesign shape")?;
    reserve(e, n + 1)?;
    work(e, n + 1)?;
    let mut b = zeros::<f64>(e, n + 1)?;
    for (i, v) in b.iter_mut().enumerate() {
        if i % 4096 == 0 {
            e.check_cancel()?;
        }
        let ta = ((i as f64 - n as f64 / 2.) / sps as f64).abs();
        *v = if beta == 0. {
            sinc(ta)
        } else if shape == "normal" {
            let x = 2. * beta * ta;
            let d = 1. - x;
            if d.abs() < 1e-4 {
                sinc(ta) * (PI / 2.) * sinc_rad(PI / 2. * d) / (1. + x)
            } else {
                sinc(ta) * (PI * beta * ta).cos() / (1. - x * x)
            }
        } else if ta == 0. {
            1. + beta * (4. / PI - 1.)
        } else {
            let u = 4. * beta * ta - 1.;
            if u.abs() < 1e-4 {
                // Cancellation-free rewrite of num/den near 4*beta*|t| = 1.
                let x = PI / (4. * beta);
                ((PI / 2.) * (x * (1. + u) - PI / 4.).cos() * sinc_rad(PI / 4. * u)
                    + (x - PI / 4. + (x + PI / 4.) * u).sin())
                    / (PI * ta * (2. + u))
            } else {
                ((PI * ta * (1. - beta)).sin() + 4. * beta * ta * (PI * ta * (1. + beta)).cos())
                    / (PI * ta * (1. - (4. * beta * ta).powi(2)))
            }
        };
    }
    let norm = b.iter().map(|x| x * x).sum::<f64>().sqrt();
    for v in &mut b {
        *v /= norm;
    }
    rv(1, n + 1, b)
}
fn sinc(t: f64) -> f64 {
    sinc_rad(PI * t)
}
fn sinc_rad(x: f64) -> f64 {
    if x.abs() < 1e-8 { 1. } else { x.sin() / x }
}
