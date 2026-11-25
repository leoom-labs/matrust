//! Balanced ANOVA via orthogonal sums of squares (standard fixed-effects model).
use super::*;
fn label(e: &Engine, v: &Value) -> Result<Vec<Option<String>>> {
    match v {
        Value::Text(s) => Ok(vec![if s.trim_end().is_empty() {
            None
        } else {
            Some(s.trim_end().to_string())
        }]),
        Value::QuotedString(s) => Ok(vec![Some(s.clone())]),
        Value::Container(Array::Str(s)) => Ok(s
            .data()
            .iter()
            .map(|s| s.as_ref().filter(|s| !s.is_empty()).map(|s| s.to_string()))
            .collect()),
        Value::Container(Array::Cell(c)) => (0..c.len())
            .map(|i| {
                poll(e, i)?;
                let v = Value::Container(c.get_linear(i)?.as_ref().clone());
                let s = text(&v).ok_or("anova1: group cells must be text")?;
                Ok(if s.is_empty() { None } else { Some(s) })
            })
            .collect(),
        Value::Container(Array::Character(c)) => Ok((0..c.rows())
            .map(|i| {
                let s = String::from_utf16_lossy(
                    &(0..c.cols())
                        .map(|j| c.units()[i + j * c.rows()])
                        .collect::<Vec<_>>(),
                )
                .trim_end()
                .to_string();
                if s.is_empty() { None } else { Some(s) }
            })
            .collect()),
        _ => Ok(mat(v, e)?
            .data()
            .iter()
            .map(|v| {
                if v.is_nan() {
                    None
                } else {
                    Some(if *v == 0. {
                        "0".to_string()
                    } else {
                        format!("{v}")
                    })
                }
            })
            .collect()),
    }
}
fn numeric_labels(v: &Value) -> bool {
    matches!(
        v,
        Value::Matrix(_) | Value::Logical(_) | Value::Container(Array::Typed(_))
    )
}
fn table(e: &Engine, rows: Vec<Vec<Value>>) -> Result<Value> {
    let r = rows.len();
    let c = 6;
    cell(
        e,
        r,
        c,
        (0..c)
            .flat_map(|j| rows.iter().map(move |row| row[j].clone()))
            .collect(),
    )
}
fn heading() -> Vec<Value> {
    ["Source", "SS", "df", "MS", "F", "Prob>F"]
        .iter()
        .map(|s| Value::Text((*s).into()))
        .collect()
}
fn entry(
    name: &str,
    ss: f64,
    df: f64,
    ms: Option<f64>,
    f: Option<f64>,
    p: Option<f64>,
) -> Vec<Value> {
    vec![
        Value::Text(name.into()),
        val(ss),
        val(df),
        ms.map_or_else(blank, val),
        f.map_or_else(blank, val),
        p.map_or_else(blank, val),
    ]
}
fn display(e: &mut Engine, out: &mut Execution, rows: &[Vec<Value>]) -> Result<()> {
    let mut s = String::new();
    for row in rows {
        for v in row {
            s.push_str(&match v {
                Value::Text(t) => t.clone(),
                Value::Matrix(m) if m.len() == 1 => format!("{:.8}", m.data()[0]),
                _ => String::new(),
            });
            s.push('\t')
        }
        s.push('\n')
    }
    e.check_cancel()?;
    out.output.push(s);
    Ok(())
}
pub(super) fn test(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if args.len() > 3 {
        return Err(format!("{name}: too many inputs"));
    }
    if name == "anova1" {
        one(e, args, out)
    } else {
        two(e, args, out)
    }
}
fn one(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
    let y = mat(&args[0], e)?;
    let vector = y.rows() <= 1 || y.cols() <= 1;
    let supplied = args.get(1).filter(|v| !empty(v));
    let labels = if let Some(v) = supplied {
        label(e, v)?
    } else if vector {
        vec![Some("1".into()); y.len()]
    } else {
        (1..=y.cols()).map(|j| Some(j.to_string())).collect()
    };
    let needed = if vector { y.len() } else { y.cols() };
    if labels.len() != needed {
        return Err("anova1: group labels must match observations or matrix columns".into());
    }
    let mut names: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (i, s) in labels.iter().flatten().enumerate() {
        poll(e, i)?;
        if seen.insert(s.clone()) {
            if names.len() == 65536 {
                return Err("anova1: too many groups".into());
            }
            names.push(s.clone());
        }
    }
    if supplied.is_some_and(numeric_labels) {
        names.sort_by(|a, b| {
            a.parse::<f64>()
                .unwrap()
                .total_cmp(&b.parse::<f64>().unwrap())
        });
    }
    let indices: std::collections::HashMap<_, _> = names
        .iter()
        .enumerate()
        .map(|(i, s)| (s.clone(), i))
        .collect();
    let mut groups = vec![Vec::new(); names.len()];
    for (i, &v) in y.data().iter().enumerate() {
        poll(e, i)?;
        if !v.is_nan()
            && let Some(s) = &labels[if vector { i } else { i / y.rows() }]
        {
            let j = indices[s];
            groups[j].push(v)
        }
    }
    let active: Vec<_> = groups
        .iter()
        .enumerate()
        .filter(|(_, g)| !g.is_empty())
        .map(|(i, _)| i)
        .collect();
    let names: Vec<_> = active.iter().map(|&i| names[i].clone()).collect();
    let groups: Vec<_> = active.iter().map(|&i| &groups[i]).collect();
    let n = groups.iter().map(|g| g.len()).sum::<usize>();
    let grand = groups.iter().flat_map(|g| g.iter()).sum::<f64>() / n as f64;
    let means: Vec<_> = groups
        .iter()
        .map(|g| g.iter().sum::<f64>() / g.len() as f64)
        .collect();
    let between = groups
        .iter()
        .zip(&means)
        .map(|(g, m)| g.len() as f64 * (m - grand).powi(2))
        .sum::<f64>();
    let within = groups
        .iter()
        .zip(&means)
        .map(|(g, m)| g.iter().map(|v| (v - m).powi(2)).sum::<f64>())
        .sum::<f64>();
    let df = groups.len() as f64 - 1.;
    let edf = n as f64 - groups.len() as f64;
    let mse = within / edf;
    let f = between / df / mse;
    let p = Family::F.cdf(f, &[df, edf], true);
    let rows = vec![
        heading(),
        entry("Groups", between, df, Some(between / df), Some(f), Some(p)),
        entry("Error", within, edf, Some(mse), None, None),
        entry("Total", between + within, n as f64 - 1., None, None, None),
    ];
    let show = args.get(2).map_or(Ok("on".into()), |v| {
        text(v).ok_or("anova1: displayopt must be text")
    })?;
    if show.eq_ignore_ascii_case("on") {
        display(e, out, &rows)?
    } else if !show.eq_ignore_ascii_case("off") {
        return Err("anova1: displayopt must be on or off".into());
    }
    let stats = structure(
        e,
        vec![
            (
                "gnames",
                cell(
                    e,
                    names.len(),
                    1,
                    names.iter().map(|s| Value::Text(s.clone())).collect(),
                )?,
            ),
            ("n", row(groups.iter().map(|g| g.len() as f64).collect())?),
            ("source", Value::Text("anova1".into())),
            ("means", row(means)?),
            ("df", val(edf)),
            ("s", val(mse.sqrt())),
        ],
    )?;
    Ok(vec![val(p), table(e, rows)?, stats])
}
fn two(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
    let y = mat(&args[0], e)?;
    let reps = if let Some(v) = args.get(1).filter(|v| !empty(v)) {
        integer(v, e)?
    } else {
        1
    };
    if reps == 0 || !y.rows().is_multiple_of(reps) {
        return Err("anova2: rows must be a multiple of positive reps".into());
    }
    if y.data().iter().any(|x| !x.is_finite()) {
        return Err("anova2: NaN or Inf data is not supported".into());
    }
    let a = y.cols();
    let b = y.rows() / reps;
    if a < 2 || b < 2 {
        return Err("anova2: at least two rows and columns of groups required".into());
    }
    let n = y.len() as f64;
    let grand = y.data().iter().sum::<f64>() / n;
    let mut cm = vec![0.; a];
    let mut rm = vec![0.; b];
    let mut means = vec![0.; a * b];
    for j in 0..a {
        for i in 0..b {
            poll(e, j * b + i)?;
            let m = (0..reps)
                .map(|r| y.data()[i * reps + r + j * y.rows()])
                .sum::<f64>()
                / reps as f64;
            means[i + j * b] = m;
            cm[j] += m / b as f64;
            rm[i] += m / a as f64;
        }
    }
    let sc = cm
        .iter()
        .map(|m| (m - grand).powi(2) * (b * reps) as f64)
        .sum::<f64>();
    let sr = rm
        .iter()
        .map(|m| (m - grand).powi(2) * (a * reps) as f64)
        .sum::<f64>();
    let mut si = 0.;
    let mut se = 0.;
    for j in 0..a {
        for i in 0..b {
            poll(e, j * b + i)?;
            si += reps as f64 * (means[i + j * b] - cm[j] - rm[i] + grand).powi(2);
            for r in 0..reps {
                poll(e, r)?;
                se += (y.data()[i * reps + r + j * y.rows()] - means[i + j * b]).powi(2)
            }
        }
    }
    let dc = (a - 1) as f64;
    let dr = (b - 1) as f64;
    let di = dc * dr;
    let de = if reps == 1 {
        di
    } else {
        (a * b * (reps - 1)) as f64
    };
    if reps == 1 {
        se = si
    }
    let mse = se / de;
    let ss = if reps == 1 {
        vec![sc, sr]
    } else {
        vec![sc, sr, si]
    };
    let dfs = if reps == 1 {
        vec![dc, dr]
    } else {
        vec![dc, dr, di]
    };
    let ps: Vec<_> = ss
        .iter()
        .zip(&dfs)
        .map(|(s, d)| Family::F.cdf(s / d / mse, &[*d, de], true))
        .collect();
    let mut rows = vec![heading()];
    for (i, label) in ["Columns", "Rows", "Interaction"]
        .iter()
        .take(ss.len())
        .enumerate()
    {
        rows.push(entry(
            label,
            ss[i],
            dfs[i],
            Some(ss[i] / dfs[i]),
            Some(ss[i] / dfs[i] / mse),
            Some(ps[i]),
        ))
    }
    rows.push(entry("Error", se, de, Some(mse), None, None));
    rows.push(entry(
        "Total",
        sc + sr + si + if reps == 1 { 0. } else { se },
        n - 1.,
        None,
        None,
        None,
    ));
    let show = args.get(2).map_or(Ok("on".into()), |v| {
        text(v).ok_or("anova2: displayopt must be text")
    })?;
    if show.eq_ignore_ascii_case("on") {
        display(e, out, &rows)?
    } else if !show.eq_ignore_ascii_case("off") {
        return Err("anova2: displayopt must be on or off".into());
    }
    let stats = structure(
        e,
        vec![
            ("source", Value::Text("anova2".into())),
            ("sigmasq", val(mse)),
            ("colmeans", row(cm)?),
            ("coln", val((b * reps) as f64)),
            ("rowmeans", row(rm)?),
            ("rown", val((a * reps) as f64)),
            ("inter", val(f64::from(reps > 1))),
            ("pval", val(if reps > 1 { ps[2] } else { f64::NAN })),
            ("df", val(de)),
        ],
    )?;
    Ok(vec![row(ps)?, table(e, rows)?, stats])
}
