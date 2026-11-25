//! Type-1 inference. Public contract: MathWorks mamfis.evalfis and
//! fuzzy-inference-process. Oracle: tool/refs/fuzzy/eval.py (NumPy/skfuzzy).
//! Cache input memberships and output universes once per call. The sample loop
//! uses only reusable buffers; diagnostics retain only the last input sample.
use super::*;
#[derive(Clone)]
enum Method {
    Min,
    Max,
    Prod,
    Sum,
    Probor,
    Custom(Array),
}
impl Method {
    fn from(a: &Array) -> Result<Self> {
        Ok(match mf::function_name(a)?.as_str() {
            "min" => Self::Min,
            "max" => Self::Max,
            "prod" => Self::Prod,
            "sum" => Self::Sum,
            "probor" => Self::Probor,
            _ => Self::Custom(a.clone()),
        })
    }
    fn pair(&self, a: f64, b: f64) -> f64 {
        match self {
            Self::Min => a.min(b),
            Self::Max => a.max(b),
            Self::Prod => a * b,
            Self::Sum => a + b,
            Self::Probor => a + b - a * b,
            Self::Custom(_) => unreachable!(),
        }
    }
}
pub(super) fn custom(
    e: &mut Engine,
    f: &Array,
    args: Vec<Value>,
    out: &mut Execution,
) -> Result<Value> {
    match f {
        Array::Handle(h) => e
            .call_handle(h, args, 1, None, out)?
            .into_iter()
            .next()
            .ok_or("Custom fuzzy function returned no output".into()),
        _ => e.call(&mf::function_name(f)?, args, out),
    }
}
fn diagnostic(
    e: &mut Engine,
    out: &mut Execution,
    opts: &Record,
    key: &str,
    message: String,
) -> Result<()> {
    match string(&opts[key])?.as_str() {
        "error" => Err(format!("evalfis: {message}")),
        "warning" => {
            e.lang_utils_call(
                "warning",
                &[
                    Value::Text(format!("fuzzy:evalfis:{key}")),
                    Value::Text("%s".into()),
                    Value::Text(message),
                ],
                out,
            )?;
            Ok(())
        }
        _ => Ok(()),
    }
}
pub(super) fn grid(r: [f64; 2], n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| {
            if i + 1 == n {
                r[1]
            } else {
                r[0] + (r[1] - r[0]) * (i as f64 / (n - 1).max(1) as f64)
            }
        })
        .collect()
}
pub(super) fn value(rows: usize, cols: usize, v: Vec<f64>) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(rows, cols, v)?))
}
pub(super) fn system(e: &Engine, v: &Value) -> Result<Arc<Object>> {
    let o = object(e, v)?;
    if !fis::is_system(o.class) {
        return Err("Expected a mamfis or sugfis object".into());
    }
    o.record()?;
    Ok(o)
}
fn memberships(e: &mut Engine, r: &Record, x: &Value, out: &mut Execution) -> Result<Vec<f64>> {
    let v = mf::evaluate_record(e, r, x, out)?;
    let m = matrix(e, &v)?;
    if m.data()
        .iter()
        .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
    {
        return Err("evalfis: membership values must be finite and in [0,1]".into());
    }
    Ok(m.data().to_vec())
}
struct Rule {
    ant: Vec<i32>,
    con: Vec<i32>,
    weight: f64,
    or: bool,
}
struct Output {
    range: [f64; 2],
    x: Vec<f64>,
    mfs: Vec<Vec<f64>>,
}
pub(super) fn call(
    e: &mut Engine,
    a: &[Value],
    nout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if !(2..=3).contains(&a.len()) || nout > 5 {
        return Err(
            "evalfis: expected fis, input and optional options; at most five outputs".into(),
        );
    }
    let o = system(e, &a[0])?;
    let p = o.record()?;
    // Structural checks remain mandatory for numerical safety, even when edits
    // were staged with DisableStructuralChecks=true.
    let mut checked = p.clone();
    checked.insert("DisableStructuralChecks".into(), num(0.));
    fis::validate(e, o.class, &checked)?;
    let inputs = fis::collection(e, p, "Inputs")?;
    let outputs = fis::collection(e, p, "Outputs")?;
    let rules = fis::collection(e, p, "Rules")?;
    let (ni, no, nr) = (inputs.len(), outputs.len(), rules.len());
    if ni == 0 || no == 0 {
        return Err("evalfis: FIS must have inputs and outputs".into());
    }
    let input = matrix(e, &a[1])?;
    let vector_sample =
        !input.is_nd() && (input.rows() == 1 || input.cols() == 1) && input.len() == ni;
    if input.is_nd() || input.is_empty() || (!vector_sample && input.cols() != ni) {
        return Err(format!(
            "evalfis: input must have {ni} columns (one per input variable)"
        ));
    }
    let ns = if vector_sample { 1 } else { input.rows() };
    let opts = eval_options::get(e, a.get(2), "evalfisOptions")?;
    let sugeno = o.class == "sugfis";
    let np = if sugeno {
        1
    } else {
        count(&opts["NumSamplePoints"], MAX_ITEMS, true)?
    };
    let nmi = inputs
        .items
        .iter()
        .map(|v| fis::collection(e, v, "MembershipFunctions").map(|m| m.len()))
        .collect::<Result<Vec<_>>>()?;
    let nmo = outputs
        .items
        .iter()
        .map(|v| fis::collection(e, v, "MembershipFunctions").map(|m| m.len()))
        .collect::<Result<Vec<_>>>()?;
    let implication = Method::from(&p["ImplicationMethod"])?;
    let aggregation = Method::from(&p["AggregationMethod"])?;
    let need_rules = nout >= 3 || matches!(aggregation, Method::Custom(_));
    Matrix::checked_len(ns, no)?;
    Matrix::checked_len(np, no)?;
    if nout >= 2 {
        Matrix::checked_len(nr, ni)?;
    }
    if need_rules {
        Matrix::checked_len(np, nr * no)?;
    }
    let storage = ns
        .saturating_mul(ni + no + nmi.iter().sum::<usize>())
        .saturating_add(
            np.saturating_mul(nmo.iter().sum::<usize>() + no + nr * usize::from(need_rules) * no),
        )
        .saturating_add(nr.saturating_mul(ni + no + 8))
        // Transient MF evaluation, custom inference arrays and grid copies.
        .saturating_add(ns.saturating_mul(4))
        .saturating_add(np.saturating_mul(nr + 8));
    e.preflight_elements(storage)?;
    let work = ns
        .saturating_mul(nr.saturating_mul(ni + np.saturating_mul(no)))
        .saturating_add(storage);
    e.charge(work)?;
    let mut compiled = Vec::with_capacity(nr);
    for r in rules.items.iter() {
        compiled.push(Rule {
            ant: vecnum(&r["Antecedent"])?
                .iter()
                .map(|v| *v as i32)
                .collect(),
            con: vecnum(&r["Consequent"])?
                .iter()
                .map(|v| *v as i32)
                .collect(),
            weight: scalar(&r["Weight"])?,
            or: scalar(&r["Connection"])? == 2.,
        });
    }
    let and = Method::from(&p["AndMethod"])?;
    let or = Method::from(&p["OrMethod"])?;
    let def = mf::function_name(&p["DefuzzificationMethod"])?;
    let mut clipped = Vec::with_capacity(ni);
    let mut fuzz = Vec::with_capacity(ni);
    for (j, v) in inputs.items.iter().enumerate() {
        e.check_cancel()?;
        let range = fis::range(&v["Range"])?;
        let mut col = Vec::with_capacity(ns);
        for s in 0..ns {
            if s % 4096 == 0 {
                e.check_cancel()?;
            }
            let x = input.data()[if vector_sample { j } else { s + j * ns }];
            if !x.is_finite() {
                return Err("evalfis: input values must be finite".into());
            }
            if x < range[0] || x > range[1] {
                diagnostic(
                    e,
                    out,
                    &opts,
                    "OutOfRangeInputValueMessage",
                    format!(
                        "Input {} expects a value in range [{} {}], but has a value of {}.",
                        j + 1,
                        range[0],
                        range[1],
                        x
                    ),
                )?;
            }
            col.push(x.clamp(range[0], range[1]));
        }
        let x = value(ns, 1, col.clone())?;
        let mut mfs = Vec::new();
        for m in fis::collection(e, v, "MembershipFunctions")?.items.iter() {
            mfs.push(memberships(e, m, &x, out)?);
        }
        clipped.push(col);
        fuzz.push(mfs);
    }
    let mut output_data = Vec::with_capacity(no);
    for v in outputs.items.iter() {
        e.check_cancel()?;
        let range = fis::range(&v["Range"])?;
        let x = if sugeno { vec![] } else { grid(range, np) };
        let xv = value(x.len(), 1, x.clone())?;
        let mut mfs = Vec::new();
        for m in fis::collection(e, v, "MembershipFunctions")?.items.iter() {
            mfs.push(if sugeno {
                vecnum(&m["Parameters"])?
            } else {
                memberships(e, m, &xv, out)?
            });
        }
        output_data.push(Output { range, x, mfs });
    }
    let mut y = vec![0.; ns * no];
    let mut fi = vec![0.; if nout >= 2 { nr * ni } else { 0 }];
    let mut ro = vec![0.; if need_rules { np * nr * no } else { 0 }];
    let mut ag = vec![0.; np * no];
    let mut firing = vec![0.; nr];
    let mut terms = vec![0.; ni];
    let mut implied = vec![0.; np];
    for s in 0..ns {
        e.check_cancel()?;
        for (r, rule) in compiled.iter().enumerate() {
            if r % 256 == 0 {
                e.check_cancel()?;
            }
            let method = if rule.or { &or } else { &and };
            let mut f = if rule.or { 0. } else { 1. };
            let mut count = 0;
            for (j, &idx) in rule.ant.iter().enumerate() {
                let mu = if idx == 0 {
                    if rule.or { 0. } else { 1. }
                } else {
                    let mu = fuzz[j][idx.unsigned_abs() as usize - 1][s];
                    if idx < 0 { 1. - mu } else { mu }
                };
                if nout >= 2 && s + 1 == ns {
                    fi[r + j * nr] = mu;
                }
                if idx != 0 {
                    terms[count] = mu;
                    count += 1;
                    if !matches!(method, Method::Custom(_)) {
                        f = method.pair(f, mu);
                    }
                }
            }
            if let Method::Custom(fun) = method
                && count != 0
            {
                let v = custom(e, fun, vec![value(count, 1, terms[..count].to_vec())?], out)?;
                f = matrix(e, &v)?.as_scalar()?;
            }
            if !f.is_finite() || !(0. ..=1.).contains(&f) {
                return Err("evalfis: firing strengths must be finite and in [0,1]".into());
            }
            firing[r] = f * rule.weight;
        }
        ag.fill(0.);
        if need_rules {
            ro.fill(0.);
        }
        for (j, od) in output_data.iter().enumerate() {
            let mut total = 0.;
            for (r, rule) in compiled.iter().enumerate() {
                if r % 64 == 0 {
                    e.check_cancel()?;
                }
                let idx = rule.con[j];
                if idx == 0 {
                    continue;
                }
                let f = firing[r];
                total += f;
                let mf = &od.mfs[idx.unsigned_abs() as usize - 1];
                if sugeno {
                    let level = if mf.len() == 1 {
                        mf[0]
                    } else {
                        mf[ni] + (0..ni).map(|k| mf[k] * clipped[k][s]).sum::<f64>()
                    };
                    let v = f * level;
                    ag[j] += v;
                    if need_rules {
                        ro[r + j * nr] = v;
                    }
                } else {
                    if let Method::Custom(fun) = &implication {
                        let vals = if idx < 0 {
                            mf.iter().map(|v| 1. - v).collect()
                        } else {
                            mf.clone()
                        };
                        let v = custom(
                            e,
                            fun,
                            vec![value(1, np, vec![f; np])?, value(1, np, vals)?],
                            out,
                        )?;
                        let m = matrix(e, &v)?;
                        if m.rows() != 1 || m.cols() != np {
                            return Err(
                                "evalfis: custom implication must preserve input shape".into()
                            );
                        }
                        implied.copy_from_slice(m.data());
                    } else {
                        for k in 0..np {
                            implied[k] =
                                implication.pair(f, if idx < 0 { 1. - mf[k] } else { mf[k] });
                        }
                    }
                    if implied.iter().any(|v| !v.is_finite() || *v < 0.) {
                        return Err("evalfis: invalid implication values".into());
                    }
                    for k in 0..np {
                        if need_rules {
                            ro[k + (r + j * nr) * np] = implied[k];
                        }
                        if !matches!(aggregation, Method::Custom(_)) {
                            ag[k + j * np] = aggregation.pair(ag[k + j * np], implied[k]);
                        }
                    }
                }
            }
            if let Method::Custom(fun) = &aggregation
                && total > 0.
            {
                // Inference functions reduce columns, as min/max/prod do.
                let mut rows = vec![0.; nr * np];
                for r in 0..nr {
                    for k in 0..np {
                        rows[r + k * nr] = ro[k + (r + j * nr) * np];
                    }
                }
                let v = custom(e, fun, vec![value(nr, np, rows)?], out)?;
                let m = matrix(e, &v)?;
                if m.rows() != 1 || m.cols() != np {
                    return Err(
                        "evalfis: custom aggregation must return one value per sample point".into(),
                    );
                }
                ag[j * np..(j + 1) * np].copy_from_slice(m.data());
            }
            if !sugeno
                && ag[j * np..(j + 1) * np]
                    .iter()
                    .any(|v| !v.is_finite() || *v < 0.)
            {
                return Err(
                    "evalfis: aggregated memberships must be finite and nonnegative".into(),
                );
            }
            let mid = od.range[0] + (od.range[1] - od.range[0]) / 2.;
            y[s + j * ns] = if total == 0. {
                diagnostic(
                    e,
                    out,
                    &opts,
                    "NoRuleFiredMessage",
                    format!(
                        "No rules fired for Output {}. Defuzzified output value set to its mean range value {}.",
                        j + 1,
                        mid
                    ),
                )?;
                mid
            } else if sugeno {
                if def == "wtaver" {
                    ag[j] / total
                } else if def == "wtsum" {
                    ag[j]
                } else {
                    return Err("evalfis: unsupported Sugeno defuzzification method".into());
                }
            } else if ag[j * np..(j + 1) * np].iter().all(|v| *v == 0.) {
                diagnostic(
                    e,
                    out,
                    &opts,
                    "EmptyOutputFuzzySetMessage",
                    format!(
                        "Output {} has an empty output fuzzy set. Defuzzified output value set to its mean range value {}.",
                        j + 1,
                        mid
                    ),
                )?;
                mid
            } else if ["centroid", "bisector", "mom", "som", "lom"].contains(&def.as_str()) {
                defuzz::compute(e, &od.x, &ag[j * np..(j + 1) * np], &def)?
            } else {
                let v = custom(
                    e,
                    &p["DefuzzificationMethod"],
                    vec![
                        value(1, np, od.x.clone())?,
                        value(1, np, ag[j * np..(j + 1) * np].to_vec())?,
                    ],
                    out,
                )?;
                matrix(e, &v)?.as_scalar()?
            };
            if !y[s + j * ns].is_finite() {
                return Err("evalfis: nonfinite defuzzified output".into());
            }
        }
    }
    let mut result = vec![value(ns, no, y)?];
    if nout >= 2 {
        result.push(value(nr, ni, fi)?);
    }
    if nout >= 3 {
        result.push(if sugeno {
            value(nr, no, ro)?
        } else {
            value(np, nr * no, ro)?
        });
    }
    if nout >= 4 {
        result.push(value(np, no, ag)?);
    }
    if nout >= 5 {
        result.push(value(nr, 1, firing)?);
    }
    Ok(result)
}
