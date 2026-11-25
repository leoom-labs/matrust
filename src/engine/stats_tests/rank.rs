//! Midrank permutation tests; normal approximations with tie and continuity corrections.
use super::*;
fn ranks(e: &Engine, x: &[f64]) -> Result<(Vec<usize>, f64)> {
    let mut order: Vec<_> = (0..x.len()).collect();
    sort(e, &mut order, |&i, &j| x[i].total_cmp(&x[j]))?;
    let mut r = vec![0; x.len()];
    let mut correction = 0.;
    let mut i = 0;
    while i < x.len() {
        poll(e, i)?;
        let mut j = i + 1;
        while j < x.len() && x[order[j]] == x[order[i]] {
            poll(e, j)?;
            j += 1
        }
        for &k in &order[i..j] {
            r[k] = i + j + 1
        }
        let t = (j - i) as f64;
        correction += t * t * t - t;
        i = j
    }
    Ok((r, correction))
}
fn exact(e: &Engine, r: &[usize], choose: Option<usize>, observed: usize, t: Tail) -> Result<f64> {
    let sum = r
        .iter()
        .try_fold(0usize, |s, &x| s.checked_add(x))
        .ok_or("Exact DP too large; use approximate")?;
    let levels = choose.map_or(1, |k| k + 1);
    let states = (sum + 1)
        .checked_mul(levels)
        .ok_or("Exact DP too large; use approximate")?;
    if states > 10_000_000 {
        return Err("Exact DP too large; use method 'approximate'".into());
    }
    let mut dp = vec![0.; states];
    dp[0] = 1.;
    let width = sum + 1;
    let mut used_sum = 0;
    for (i, &v) in r.iter().enumerate() {
        e.check_cancel()?;
        if let Some(k) = choose {
            for c in (1..=k.min(i + 1)).rev() {
                for s in (v..=used_sum + v).rev() {
                    poll(e, s)?;
                    dp[c * width + s] += dp[(c - 1) * width + s - v];
                }
            }
        } else {
            for s in (v..=used_sum + v).rev() {
                poll(e, s)?;
                dp[s] += dp[s - v];
            } // Scale each Bernoulli convolution to avoid overflow.
            for (j, p) in dp.iter_mut().enumerate().take(used_sum + v + 1) {
                poll(e, j)?;
                *p *= 0.5;
            }
        }
        used_sum += v;
    }
    let distribution = &dp[choose.unwrap_or(0) * width..][..width];
    let total = distribution.iter().sum::<f64>();
    let left = distribution[..=observed.min(sum)].iter().sum::<f64>() / total;
    let right = distribution[observed.min(sum)..].iter().sum::<f64>() / total;
    Ok(match t {
        Tail::Both => (2. * left.min(right)).min(1.),
        Tail::Left => left,
        Tail::Right => right,
    })
}
pub(super) fn test(e: &Engine, name: &str, args: &[Value]) -> Result<Vec<Value>> {
    let sum = name == "ranksum";
    if sum && args.len() < 2 {
        return Err("ranksum needs two vectors".into());
    }
    let (end, o) = Opts::parse(args, if sum { 2 } else { 1 }, &["Alpha", "Tail", "Method"])?;
    if end > 3 {
        return Err(format!("{name}: too many positional inputs"));
    }
    let a = alpha(if end > 2 && !empty(&args[2]) {
        scalar(&args[2], e)?
    } else {
        o.num("alpha", 0.05, e)?
    })?;
    let t = tail(&o.word("tail", "both")?)?;
    let method = o.word("method", "default")?;
    if !["exact", "approximate", "default"].contains(&method.as_str()) {
        return Err("Method must be exact or approximate".into());
    }
    let (r, obs, corr, nx, ny, default_exact) = if sum {
        let x = vector(&args[0], e)?;
        let y = vector(&args[1], e)?;
        let nx = x.len();
        let ny = y.len();
        let (r, c) = ranks(e, &x.into_iter().chain(y).collect::<Vec<_>>())?;
        let obs = r[..nx].iter().sum();
        (r, obs, c, nx, ny, nx.min(ny) < 10 && nx + ny < 20)
    } else {
        let x = mat(&args[0], e)?;
        if x.rows() > 1 && x.cols() > 1 {
            return Err("signrank: input must be a vector".into());
        }
        let y = if end > 1 && !empty(&args[1]) {
            Some(mat(&args[1], e)?)
        } else {
            None
        };
        if let Some(y) = &y
            && y.len() != 1
            && (y.rows() != x.rows() || y.cols() != x.cols())
        {
            return Err("signrank: paired inputs must have the same size".into());
        }
        let mut differences = Vec::new();
        for (i, &v) in x.data().iter().enumerate() {
            poll(e, i)?;
            let d = v - y
                .as_ref()
                .map_or(0., |y| y.data()[if y.len() == 1 { 0 } else { i }]);
            if !d.is_nan() && d != 0. {
                differences.push(d)
            }
        }
        let (r, c) = ranks(e, &differences.iter().map(|d| d.abs()).collect::<Vec<_>>())?;
        let obs = r
            .iter()
            .zip(&differences)
            .filter(|(_, d)| **d > 0.)
            .map(|(r, _)| *r)
            .sum();
        let n = r.len();
        (r, obs, c, n, 0, n <= 15)
    };
    let use_exact = method == "exact" || method == "default" && default_exact;
    let n = (nx + ny) as f64;
    let stat = obs as f64 / 2.;
    let mut fields = vec![(if sum { "ranksum" } else { "signedrank" }, val(stat))];
    let p = if nx == 0 || sum && ny == 0 {
        f64::NAN
    } else if use_exact {
        if sum && nx > ny {
            let total: usize = r.iter().sum();
            let flipped = match t {
                Tail::Left => Tail::Right,
                Tail::Right => Tail::Left,
                Tail::Both => Tail::Both,
            };
            exact(e, &r, Some(ny), total - obs, flipped)?
        } else {
            exact(e, &r, if sum { Some(nx) } else { None }, obs, t)?
        }
    } else {
        let (mu, var) = if sum {
            (
                nx as f64 * (n + 1.) / 2.,
                nx as f64 * ny as f64 / 12. * (n + 1. - corr / (n * (n - 1.))),
            )
        } else {
            (
                n * (n + 1.) / 4.,
                n * (n + 1.) * (2. * n + 1.) / 24. - corr / 48.,
            )
        };
        let delta = stat - mu;
        let cc = match t {
            Tail::Both => 0.5 * if delta == 0. { 0. } else { delta.signum() },
            Tail::Right => 0.5,
            Tail::Left => -0.5,
        };
        let z = (delta - cc) / var.sqrt();
        fields.push(("zval", val(z)));
        probability(Family::Norm, z, &[0., 1.], t)
    };
    Ok(vec![val(p), logical(p < a), structure(e, fields)?])
}
