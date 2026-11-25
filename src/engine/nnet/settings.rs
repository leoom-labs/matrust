use super::*;
use process::Process;
fn column(v: &[f64]) -> Result<Array> {
    Ok(Array::Numeric(Matrix::from_column_slice(v.len(), 1, v)?))
}
fn row(v: &[f64]) -> Result<Array> {
    Ok(Array::Numeric(Matrix::from_column_slice(1, v.len(), v)?))
}
pub(super) fn params(name: &str, (a, b): (f64, f64), c: &AtomicBool) -> Result<Array> {
    record(
        match name {
            "mapminmax" => vec![("ymin", num(a)), ("ymax", num(b))],
            "mapstd" => vec![("ymean", num(a)), ("ystd", num(b))],
            _ => vec![("max_range", num(a))],
        },
        c,
    )
}
pub(super) fn read_params(name: &str, a: &Array) -> Result<(f64, f64)> {
    let allowed: &[&str] = match name {
        "mapminmax" => &["ymin", "ymax"],
        "mapstd" => &["ymean", "ystd"],
        _ => &["max_range"],
    };
    let Array::Struct(s) = a else {
        return Err("processParams: expected scalar struct".into());
    };
    for key in s.field_names() {
        if !allowed.contains(&key.as_str()) {
            return Err(format!(
                "Unrecognized property '{key}' for process '{name}'"
            ));
        }
    }

    Ok(match name {
        "mapminmax" => (scalar(field(a, "ymin")?)?, scalar(field(a, "ymax")?)?),
        "mapstd" => (scalar(field(a, "ymean")?)?, scalar(field(a, "ystd")?)?),
        _ => (scalar(field(a, "max_range")?)?, 0.),
    })
}
pub(super) fn to_array(p: &Process, c: &AtomicBool) -> Result<Array> {
    let mut fields = vec![
        ("name", string(&p.name, c)?),
        ("xrows", num(p.xrows as f64)),
        ("xcols", num(p.xcols as f64)),
        ("yrows", num(p.keep.len() as f64)),
        ("ycols", num(p.xcols as f64)),
    ];
    match p.name.as_str() {
        "mapminmax" => {
            fields.extend(vec![
                ("xmin", column(&p.xmin)?),
                ("xmax", column(&p.xmax)?),
                (
                    "xrange",
                    column(
                        &p.xmin
                            .iter()
                            .zip(&p.xmax)
                            .map(|(a, b)| b - a)
                            .collect::<Vec<_>>(),
                    )?,
                ),
                ("ymin", num(p.yoffset)),
                ("ymax", num(p.yscale)),
                ("yrange", num(p.yscale - p.yoffset)),
                ("xoffset", column(&p.offset)?),
                ("gain", column(&p.gain)?),
                (
                    "no_change",
                    Array::Logical(LogicalMatrix::scalar(
                        p.gain
                            .iter()
                            .enumerate()
                            .all(|(r, &g)| g == 0. || (g == 1. && p.offset[r] == p.yoffset)),
                    )),
                ),
            ]);
        }
        "mapstd" => {
            fields.extend(vec![
                ("xmean", column(&p.offset)?),
                (
                    "xstd",
                    column(
                        &p.gain
                            .iter()
                            .map(|&g| if g == 0. { 0. } else { p.yscale / g })
                            .collect::<Vec<_>>(),
                    )?,
                ),
                ("ymean", num(p.yoffset)),
                ("ystd", num(p.yscale)),
                (
                    "no_change",
                    Array::Logical(LogicalMatrix::scalar(
                        p.gain
                            .iter()
                            .enumerate()
                            .all(|(r, &g)| g == 0. || (g == 1. && p.offset[r] == p.yoffset)),
                    )),
                ),
            ]);
        }
        _ => {
            let remove: Vec<usize> = (0..p.xrows)
                .filter(|i| p.keep.binary_search(i).is_err())
                .collect();
            fields.extend(vec![
                (
                    "keep",
                    row(&p.keep.iter().map(|&i| (i + 1) as f64).collect::<Vec<_>>())?,
                ),
                (
                    "remove",
                    row(&remove.iter().map(|&i| (i + 1) as f64).collect::<Vec<_>>())?,
                ),
                (
                    "value",
                    column(&remove.iter().map(|&i| p.offset[i]).collect::<Vec<_>>())?,
                ),
                (
                    "constants",
                    column(&remove.iter().map(|&i| p.offset[i]).collect::<Vec<_>>())?,
                ),
                ("max_range", num(p.yoffset)),
                (
                    "no_change",
                    Array::Logical(LogicalMatrix::scalar(remove.is_empty())),
                ),
            ]);
        }
    }
    record(fields, c)
}
fn vector(a: &Array, len: usize) -> Result<Vec<f64>> {
    let Array::Numeric(m) = a else {
        return Err("processing settings: expected numeric vector".into());
    };
    if m.len() != len || m.data().iter().any(|x| !x.is_finite()) {
        return Err("processing settings: invalid vector size or value".into());
    }
    Ok(m.data().to_vec())
}
fn from_array(name: &str, a: &Array) -> Result<Process> {
    if text(field(a, "name")?)? != name {
        return Err(format!(
            "{name}: settings belong to another process function"
        ));
    }
    let rows = size(scalar(field(a, "xrows")?)?, true)?;
    let cols = size(scalar(field(a, "xcols")?)?, true)?;
    let mut p = Process {
        name: name.into(),
        xrows: rows,
        xcols: cols,
        offset: vec![0.; rows],
        gain: vec![1.; rows],
        xmin: vec![0.; rows],
        xmax: vec![0.; rows],
        keep: (0..rows).collect(),
        yoffset: 0.,
        yscale: 1.,
    };
    match name {
        "mapminmax" => {
            p.offset = vector(field(a, "xoffset")?, rows)?;
            p.gain = vector(field(a, "gain")?, rows)?;
            p.xmin = vector(field(a, "xmin")?, rows)?;
            p.xmax = vector(field(a, "xmax")?, rows)?;
            p.yoffset = scalar(field(a, "ymin")?)?;
            p.yscale = scalar(field(a, "ymax")?)?;
        }
        "mapstd" => {
            p.offset = vector(field(a, "xmean")?, rows)?;
            let std = vector(field(a, "xstd")?, rows)?;
            p.yoffset = scalar(field(a, "ymean")?)?;
            p.yscale = scalar(field(a, "ystd")?)?;
            p.gain = std
                .into_iter()
                .map(|s| if s == 0. { 0. } else { p.yscale / s })
                .collect();
        }
        _ => {
            let ids = |key: &str| -> Result<Vec<usize>> {
                let Array::Numeric(m) = field(a, key)? else {
                    return Err("removeconstantrows: invalid settings indices".into());
                };
                m.data()
                    .iter()
                    .map(|&x| {
                        let i = size(x, false)?;
                        if i > rows {
                            return Err("removeconstantrows: settings index out of range".into());
                        }
                        Ok(i - 1)
                    })
                    .collect()
            };
            p.keep = ids("keep")?;
            let remove = ids("remove")?;
            let mut all = p.keep.clone();
            all.extend(&remove);
            all.sort_unstable();
            if all != (0..rows).collect::<Vec<_>>() {
                return Err("removeconstantrows: keep/remove must partition rows".into());
            }
            let values = vector(field(a, "value")?, remove.len())?;
            for (&i, &v) in remove.iter().zip(&values) {
                p.offset[i] = v;
            }
        }
    }
    if !p.yoffset.is_finite() || !p.yscale.is_finite() {
        return Err("processing settings: nonfinite parameter".into());
    }
    Ok(p)
}
pub(super) fn call(e: &Engine, name: &str, args: &[Value]) -> Result<Vec<Value>> {
    if let Some(command) = args.first().and_then(super::super::table_fns::text_scalar) {
        if command != "apply" && command != "reverse" {
            return Err(format!("{name}: unsupported command '{command}'"));
        }
        arity(name, args, 3)?;
        let p = from_array(name, &args[2].clone().array(&e.cancel)?)?;
        let x = matrix(e, &args[1])?;
        return Ok(vec![Value::Matrix(if command == "apply" {
            p.apply(&x, &e.cancel)?
        } else {
            p.reverse(&x, &e.cancel)?
        })]);
    }
    let max = if name == "removeconstantrows" { 2 } else { 3 };
    if args.is_empty() || args.len() > max {
        return Err(format!("{name}: incorrect number of arguments"));
    }
    let x = matrix(e, &args[0])?;
    let (mut a, mut b) = process::defaults(name);
    if let Some(v) = args.get(1) {
        let v = v.clone().array(&e.cancel)?;
        if matches!(v, Array::Struct(_)) {
            if args.len() != 2 {
                return Err(
                    "process parameter struct cannot be combined with numeric arguments".into(),
                );
            }
            (a, b) = read_params(name, &v)?;
        } else {
            a = scalar(&v)?;
        }
    }
    if let Some(v) = args.get(2) {
        b = matrix(e, v)?.as_scalar()?;
    }
    let p = Process::fit(name, &x, a, b, &e.cancel)?;
    Ok(vec![
        Value::Matrix(p.apply(&x, &e.cancel)?),
        Value::from(to_array(&p, &e.cancel)?),
    ])
}
