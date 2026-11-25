//! Ordinary editable plot/surf/line/text objects; no UI dependency.
use super::*;
use eval::{grid, value};
fn txt(s: &str) -> Value {
    Value::Text(s.into())
}
fn scalar_value(x: f64) -> Value {
    Value::Matrix(Matrix::scalar(x))
}
fn label(e: &mut Engine, out: &mut Execution, key: &str, s: &str) -> Result<()> {
    e.call(key, vec![txt(s)], out)?;
    Ok(())
}
fn annotation(e: &mut Engine, out: &mut Execution, x: f64, y: f64, s: &str) -> Result<()> {
    e.call(
        "text",
        vec![
            scalar_value(x),
            scalar_value(y),
            txt(s),
            txt("HorizontalAlignment"),
            txt("center"),
            txt("Interpreter"),
            txt("none"),
        ],
        out,
    )?;
    Ok(())
}
pub(super) fn plotmf(
    e: &mut Engine,
    a: &[Value],
    nout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if !(3..=4).contains(&a.len()) || nout > 2 {
        return Err(
            "plotmf: expected fis, 'input' or 'output', index and optional number of points".into(),
        );
    }
    let o = eval::system(e, &a[0])?;
    let p = o.record()?;
    let typ = vtext(e, &a[1])?.to_ascii_lowercase();
    let key = match typ.as_str() {
        "input" => "Inputs",
        "output" => "Outputs",
        _ => return Err("plotmf: variable type must be 'input' or 'output'".into()),
    };
    if typ == "output" && o.class == "sugfis" {
        return Err("plotmf: Sugeno output membership functions are not supported".into());
    }
    let vars = fis::collection(e, p, key)?;
    let idx = count(&a[2].clone().array(&e.cancel)?, vars.len(), true)? - 1;
    let v = &vars.items[idx];
    let mfs = fis::collection(e, v, "MembershipFunctions")?;
    let np = if let Some(n) = a.get(3) {
        count(&n.clone().array(&e.cancel)?, MAX_ITEMS, true)?
    } else {
        181
    };
    e.preflight_elements(np.saturating_mul(mfs.len()).saturating_mul(3))?;
    let x = grid(fis::range(&v["Range"])?, np);
    let xv = value(np, 1, x.clone())?;
    let mut xs = Vec::with_capacity(np * mfs.len());
    let mut ys = Vec::with_capacity(np * mfs.len());
    for m in mfs.items.iter() {
        e.check_cancel()?;
        xs.extend_from_slice(&x);
        let y = mf::evaluate_record(e, m, &xv, out)?;
        ys.extend_from_slice(matrix(e, &y)?.data());
    }
    if nout > 0 {
        return Ok(vec![value(np, mfs.len(), xs)?, value(np, mfs.len(), ys)?]);
    }
    e.call("gca", vec![], out)?;
    let was_hold = e.figure.as_ref().is_some_and(|f| f.hold);
    e.call(
        "plot",
        vec![value(np, mfs.len(), xs)?, value(np, mfs.len(), ys.clone())?],
        out,
    )?;
    e.call("hold", vec![txt("on")], out)?;
    for (i, m) in mfs.items.iter().enumerate() {
        let col = &ys[i * np..(i + 1) * np];
        let peak = col
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map_or(0, |(i, _)| i);
        annotation(e, out, x[peak], col[peak] + 0.04, &string(&m["Name"])?)?;
    }
    label(e, out, "xlabel", &string(&v["Name"])?)?;
    label(e, out, "ylabel", "Degree of membership")?;
    if !was_hold {
        e.call("hold", vec![txt("off")], out)?;
    }
    Ok(vec![value(0, 0, vec![])?])
}
pub(super) fn gensurf(
    e: &mut Engine,
    a: &[Value],
    nout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if !(1..=2).contains(&a.len()) || nout > 3 {
        return Err(
            "gensurf: expected fis and optional gensurfOptions; at most three outputs".into(),
        );
    }
    let o = eval::system(e, &a[0])?;
    let p = o.record()?;
    let ins = fis::collection(e, p, "Inputs")?;
    let outs = fis::collection(e, p, "Outputs")?;
    if ins.len() == 0 || outs.len() == 0 {
        return Err("gensurf: FIS must have inputs and outputs".into());
    }
    let opts = eval_options::get(e, a.get(1), "gensurfOptions")?;
    let mut ids = vecnum(&opts["InputIndex"])?;
    if ins.len() == 1 && ids == [1., 2.] {
        ids.truncate(1);
    }
    if ids.iter().any(|i| *i > ins.len() as f64) {
        return Err("gensurf: InputIndex exceeds number of inputs".into());
    }
    let ids = ids.iter().map(|i| *i as usize - 1).collect::<Vec<_>>();
    let oi = count(&opts["OutputIndex"], outs.len(), true)? - 1;
    let ng = vecnum(&opts["NumGridPoints"])?;
    let nx = ng[0] as usize;
    let ny = if ids.len() == 2 {
        *ng.get(1).unwrap_or(&ng[0]) as usize
    } else {
        1
    };
    let n = Matrix::checked_len(nx, ny)?;
    e.preflight_elements(n.saturating_mul(ins.len() + outs.len() + 5))?;
    let mut refs = if string(&opts["ReferenceInputs"]).is_ok_and(|s| s == "auto") {
        ins.items
            .iter()
            .map(|v| fis::range(&v["Range"]).map(|r| r[0] + (r[1] - r[0]) / 2.))
            .collect::<Result<Vec<_>>>()?
    } else {
        vecnum(&opts["ReferenceInputs"])?
    };
    if refs.len() != ins.len()
        || refs
            .iter()
            .enumerate()
            .any(|(i, x)| !ids.contains(&i) && !x.is_finite())
    {
        return Err(
            "gensurf: ReferenceInputs must have one finite reference per unused input".into(),
        );
    }
    for &i in &ids {
        refs[i] = 0.;
    }
    let x = grid(fis::range(&ins.items[ids[0]]["Range"])?, nx);
    let y = if ids.len() == 2 {
        grid(fis::range(&ins.items[ids[1]]["Range"])?, ny)
    } else {
        vec![0.]
    };
    let mut xs = vec![0.; n];
    let mut ys = if ids.len() == 2 { vec![0.; n] } else { vec![] };
    let mut input = vec![0.; n * ins.len()];
    for (j, &xj) in x.iter().enumerate() {
        for (i, &yi) in y.iter().enumerate() {
            let k = if ids.len() == 2 { i + j * ny } else { j };
            if k % 4096 == 0 {
                e.check_cancel()?;
            }
            xs[k] = xj;
            if ids.len() == 2 {
                ys[k] = yi;
            }
            for v in 0..ins.len() {
                input[k + v * n] = refs[v];
            }
            input[k + ids[0] * n] = xj;
            if ids.len() == 2 {
                input[k + ids[1] * n] = yi;
            }
        }
    }
    let opt = eval_options::constructor(
        e,
        "evalfisOptions",
        &[
            txt("NumSamplePoints"),
            Value::from(opts["NumSamplePoints"].clone()),
        ],
    )?;
    let result = eval::call(e, &[a[0].clone(), value(n, ins.len(), input)?, opt], 1, out)?;
    let z = matrix(e, &result[0])?.data()[oi * n..(oi + 1) * n].to_vec();
    let rows = if ids.len() == 2 { ny } else { nx };
    let cols = if ids.len() == 2 { nx } else { 1 };
    let xv = value(rows, cols, xs)?;
    let yv = if ids.len() == 2 {
        value(rows, cols, ys)?
    } else {
        value(0, 0, vec![])?
    };
    let zv = value(rows, cols, z)?;
    if nout > 0 {
        return Ok(vec![xv, yv, zv]);
    }
    if ids.len() == 2 {
        e.call("surf", vec![xv, yv, zv], out)?;
        label(e, out, "ylabel", &string(&ins.items[ids[1]]["Name"])?)?;
        label(e, out, "zlabel", &string(&outs.items[oi]["Name"])?)?;
    } else {
        e.call("plot", vec![xv, zv], out)?;
        label(e, out, "ylabel", &string(&outs.items[oi]["Name"])?)?;
    }
    label(e, out, "xlabel", &string(&ins.items[ids[0]]["Name"])?)?;
    label(e, out, "title", &string(&p["Name"])?)?;
    Ok(vec![value(0, 0, vec![])?])
}
fn segment(e: &mut Engine, out: &mut Execution, x: Vec<f64>, y: Vec<f64>) -> Result<()> {
    e.call(
        "line",
        vec![value(1, x.len(), x)?, value(1, y.len(), y)?],
        out,
    )?;
    Ok(())
}
fn box_at(e: &mut Engine, out: &mut Execution, x: f64, y: f64, w: f64, h: f64) -> Result<()> {
    segment(
        e,
        out,
        vec![x, x + w, x + w, x, x],
        vec![y, y, y + h, y + h, y],
    )
}

pub(super) fn plotfis(
    e: &mut Engine,
    a: &[Value],
    nout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if a.len() != 1 || nout > 0 {
        return Err("plotfis: expected fis and no output arguments".into());
    }
    let o = eval::system(e, &a[0])?;
    let p = o.record()?;
    let ins = fis::collection(e, p, "Inputs")?;
    let outs = fis::collection(e, p, "Outputs")?;
    let nr = fis::collection(e, p, "Rules")?.len();
    if 1 + 2 * (ins.len() + outs.len()) > crate::plot::MAX_SERIES {
        return Err("plotfis: diagram exceeds the 64-line graphics limit".into());
    }
    e.call("figure", vec![], out)?;
    e.call("hold", vec![txt("on")], out)?;
    box_at(e, out, 0.38, 0.25, 0.24, 0.5)?;
    annotation(
        e,
        out,
        0.5,
        0.5,
        &format!(
            "{}\n{}\n{} rules",
            string(&p["Name"])?,
            string(&p["Type"])?,
            nr
        ),
    )?;
    for (left, vars) in [(true, ins), (false, outs)] {
        for (i, v) in vars.items.iter().enumerate() {
            e.check_cancel()?;
            let y = 1. - (i + 1) as f64 / (vars.len() + 1) as f64;
            let x = if left { 0.02 } else { 0.78 };
            box_at(e, out, x, y - 0.04, 0.2, 0.08)?;
            let nm = fis::collection(e, v, "MembershipFunctions")?.len();
            annotation(
                e,
                out,
                x + 0.1,
                y,
                &format!("{} ({} MFs)", string(&v["Name"])?, nm),
            )?;
            segment(
                e,
                out,
                if left {
                    vec![0.22, 0.38]
                } else {
                    vec![0.62, 0.78]
                },
                if left { vec![y, 0.5] } else { vec![0.5, y] },
            )?;
        }
    }
    e.call("axis", vec![value(1, 4, vec![0., 1., 0., 1.])?], out)?;
    e.call("hold", vec![txt("off")], out)?;
    Ok(vec![value(0, 0, vec![])?])
}
