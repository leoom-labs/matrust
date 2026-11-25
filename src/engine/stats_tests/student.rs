//! Student tests, Welch-Satterthwaite degrees of freedom, and trimmed means.
use super::*;
pub(super) fn test(e: &Engine, name: &str, args: &[Value]) -> Result<Vec<Value>> {
    let two = name == "ttest2";
    if two && args.len() < 2 {
        return Err("ttest2 needs two samples".into());
    }
    let (end, o) = Opts::parse(
        args,
        if two { 2 } else { 1 },
        if two {
            &["Alpha", "Tail", "Dim", "Vartype"]
        } else {
            &["Alpha", "Tail", "Dim"]
        },
    )?;
    let x = mat(&args[0], e)?;
    let mut a = o.num("alpha", 0.05, e)?;
    let mut t = tail(&o.word("tail", "both")?)?;
    let mut vartype = o.word("vartype", "equal")?;
    let mut dv = o.get("dim");
    let offset = 2;
    if end > offset && !empty(&args[offset]) {
        a = scalar(&args[offset], e)?;
    }
    if end > offset + 1 && !empty(&args[offset + 1]) {
        t = tail(&text(&args[offset + 1]).ok_or("Tail must be text")?)?;
    }
    if two {
        if end > 4 && !empty(&args[4]) {
            vartype = text(&args[4])
                .ok_or("Vartype must be text")?
                .to_ascii_lowercase();
        }
        if end > 5 {
            dv = Some(&args[5]);
        }
        if end > 6 {
            return Err("ttest2: too many inputs".into());
        }
    } else {
        if end > 4 {
            dv = Some(&args[4]);
        }
        if end > 5 {
            return Err("ttest: too many inputs".into());
        }
    }
    alpha(a)?;
    if vartype != "equal" && vartype != "unequal" {
        return Err("Vartype must be equal or unequal".into());
    }
    let d = dimension(&x, dv, e)?;
    let channels = if d == 1 { x.cols() } else { x.rows() };
    Matrix::checked_len(2, channels)?;
    if channels
        .checked_mul(9)
        .and_then(|n| n.checked_add(e.evaluated_elements))
        .is_none_or(|n| n > 32 * 1024 * 1024)
    {
        return Err(format!("{name}: outputs exceed evaluation budget"));
    }
    let mut xx = lanes(e, &x, d)?;
    let mut center = 0.;
    let mut yy = Vec::new();
    if end >= 2 && !empty(&args[1]) {
        let y = mat(&args[1], e)?;
        if two {
            yy = lanes(e, &y, d)?;
            if xx.len() != yy.len() {
                // MathWorks ttest2: two vectors are one test whatever their
                // orientation. A row otherwise selects Dim 2 and splits the
                // column into one-point samples.
                let vectors = (x.rows() <= 1 || x.cols() <= 1) && (y.rows() <= 1 || y.cols() <= 1);
                if vectors {
                    xx = vec![x.data().to_vec()];
                    yy = vec![y.data().to_vec()];
                } else {
                    return Err("ttest2: samples must have the same unreduced dimensions".into());
                }
            }
        } else if y.len() == 1 {
            center = y.data()[0];
        } else {
            if x.rows() != y.rows() || x.cols() != y.cols() {
                return Err("ttest: paired inputs must have the same size".into());
            }
            for (a, b) in xx.iter_mut().zip(lanes(e, &y, d)?) {
                for (v, w) in a.iter_mut().zip(b) {
                    *v -= w;
                }
            }
        }
    }
    if two && yy.is_empty() {
        yy = lanes(e, &mat(&args[1], e)?, d)?;
        if yy.is_empty() {
            // an empty y is an empty sample in every lane: NaN results, no error
            yy = vec![Vec::new(); xx.len()];
        }
    }
    let mut hs = Vec::new();
    let mut ps = Vec::new();
    let mut los = Vec::new();
    let mut his = Vec::new();
    let mut ts = Vec::new();
    let mut dfs = Vec::new();
    let mut sds = Vec::new();
    let mut sy = Vec::new();
    for (j, x) in xx.iter().enumerate() {
        poll(e, j)?;
        let x: Vec<_> = x.iter().copied().filter(|v| !v.is_nan()).collect();
        let (mx, sx) = mean_sd(e, &x)?;
        let nx = x.len() as f64;
        let (delta, se, df, sd) = if two {
            let y: Vec<_> = yy[j].iter().copied().filter(|v| !v.is_nan()).collect();
            let (my, s_y) = mean_sd(e, &y)?;
            let ny = y.len() as f64;
            sy.push(s_y);
            if vartype == "equal" {
                let df = nx + ny - 2.;
                let sd = (((nx - 1.) * sx * sx + (ny - 1.) * s_y * s_y) / df).sqrt();
                (mx - my, sd * (1. / nx + 1. / ny).sqrt(), df, sd)
            } else {
                let vx = sx * sx / nx;
                let vy = s_y * s_y / ny;
                (
                    mx - my,
                    (vx + vy).sqrt(),
                    (vx + vy).powi(2) / (vx * vx / (nx - 1.) + vy * vy / (ny - 1.)),
                    sx,
                )
            }
        } else {
            (mx - center, sx / nx.sqrt(), nx - 1., sx)
        };
        let stat = delta / se;
        let p = if stat.is_nan() || df <= 0. {
            f64::NAN
        } else {
            probability(Family::T, stat, &[df], t)
        };
        let critical = -Family::T.inv(if t == Tail::Both { a / 2. } else { a }, &[df]);
        let ci_center = if two { delta } else { delta + center };
        los.push(if t == Tail::Left {
            f64::NEG_INFINITY
        } else {
            ci_center - critical * se
        });
        his.push(if t == Tail::Right {
            f64::INFINITY
        } else {
            ci_center + critical * se
        });
        hs.push(decision(p, a));
        ps.push(p);
        ts.push(stat);
        dfs.push(df);
        sds.push(sd);
    }
    let sd = if two && vartype == "unequal" {
        interval(d, sds, sy)?
    } else {
        reduced(d, sds)?
    };
    Ok(vec![
        reduced(d, hs)?,
        reduced(d, ps)?,
        interval(d, los, his)?,
        structure(
            e,
            vec![
                ("tstat", reduced(d, ts)?),
                ("df", reduced(d, dfs)?),
                ("sd", sd),
            ],
        )?,
    ])
}
pub(super) fn trim(e: &Engine, args: &[Value]) -> Result<Value> {
    if args.len() < 2 || args.len() > 4 {
        return Err("trimmean: expected x, percent, optional flag and dimension".into());
    }
    let m = mat(&args[0], e)?;
    let p = scalar(&args[1], e)?;
    if !(0. ..=100.).contains(&p) {
        return Err("trimmean: percent must be in [0,100]".into());
    }
    let mut flag = "round".to_string();
    let mut dv = None;
    let mut all = false;
    for v in &args[2..] {
        if let Some(s) = text(v) {
            if s.eq_ignore_ascii_case("all") {
                all = true
            } else {
                flag = s.to_ascii_lowercase()
            }
        } else {
            if mat(v, e)?.len() > 1 {
                return Err("trimmean: vecdim not supported yet".into());
            }
            dv = Some(v)
        }
    }
    if !["round", "floor", "weighted"].contains(&flag.as_str()) {
        return Err("trimmean: unknown flag".into());
    }
    let d = dimension(&m, dv, e)?;
    let xs = if all {
        vec![m.data().to_vec()]
    } else {
        lanes(e, &m, d)?
    };
    let mut out = Vec::new();
    for (j, mut x) in xs.into_iter().enumerate() {
        poll(e, j)?;
        x.retain(|v| !v.is_nan());
        sort(e, &mut x, f64::total_cmp)?;
        let k = x.len() as f64 * p / 200.;
        let cut = if flag == "round" {
            k.round()
        } else {
            k.floor()
        } as usize;
        if cut * 2 >= x.len() {
            out.push(f64::NAN);
            continue;
        }
        if flag == "weighted" {
            let f = k - k.floor();
            let mut sum = 0.;
            let mut weight = 0.;
            for (i, &v) in x.iter().enumerate().take(x.len() - cut).skip(cut) {
                poll(e, i)?;
                let w = if i == cut && i == x.len() - cut - 1 {
                    1. - 2. * f
                } else if i == cut || i == x.len() - cut - 1 {
                    1. - f
                } else {
                    1.
                };
                sum += w * v;
                weight += w;
            }
            out.push(sum / weight)
        } else {
            out.push(x[cut..x.len() - cut].iter().sum::<f64>() / (x.len() - 2 * cut) as f64)
        }
    }
    if all {
        Ok(val(out[0]))
    } else {
        reduced(d, out)
    }
}
