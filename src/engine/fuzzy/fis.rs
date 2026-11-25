use super::*;
const METHODS: &[&str] = &[
    "AndMethod",
    "OrMethod",
    "ImplicationMethod",
    "AggregationMethod",
    "DefuzzificationMethod",
];
pub(super) fn is_system(class: &str) -> bool {
    matches!(class, "mamfis" | "sugfis")
}
pub(super) fn collection(e: &Engine, p: &Record, key: &str) -> Result<Arc<Object>> {
    array_object(
        e,
        p.get(key)
            .ok_or_else(|| format!("Missing property {key}"))?,
        match key {
            "Inputs" | "Outputs" => "fisvar",
            "MembershipFunctions" => "fismf",
            "Rules" => "fisrule",
            _ => return Err("Unknown fuzzy collection".into()),
        },
    )
}
pub(super) fn set_collection(
    e: &mut Engine,
    p: &mut Record,
    key: &str,
    class: &'static str,
    items: Vec<Arc<Record>>,
) -> Result<()> {
    p.insert(key.into(), new_array(e, Object::vector(class, items))?);
    Ok(())
}
pub(super) fn range(a: &Array) -> Result<[f64; 2]> {
    let v = vecnum(a)?;
    if v.len() != 2
        || v.iter().any(|v| !v.is_finite())
        || v[0] >= v[1]
        || !(v[1] - v[0]).is_finite()
    {
        return Err("Range must be a finite two-element vector with increasing bounds".into());
    }
    Ok([v[0], v[1]])
}
fn default_mfs(
    e: &Engine,
    r: [f64; 2],
    n: usize,
    t: &str,
    inputs: usize,
) -> Result<Vec<Arc<Record>>> {
    if n > MAX_MFS {
        return Err("fuzzy: membership function count exceeds limit".into());
    }
    if !["trimf", "gaussmf", "constant", "linear"].contains(&t) {
        return Err(
            "MFType must be trimf or gaussmf (constant or linear for Sugeno outputs)".into(),
        );
    }
    let width = r[1] - r[0];
    let step = if n > 1 { width / (n - 1) as f64 } else { width };
    (0..n)
        .map(|i| {
            let c = if n == 1 {
                (r[0] + r[1]) / 2.
            } else {
                r[0] + i as f64 * step
            };
            let p = match t {
                "trimf" => vec![c - step, c, c + step],
                "gaussmf" => vec![step / (8. * 2f64.ln()).sqrt(), c],
                "linear" => {
                    let mut p = vec![0.; inputs + 1];
                    p[inputs] = c;
                    p
                }
                _ => vec![c],
            };
            Ok(Arc::new(mf::record(
                e,
                text(t, e)?,
                numbers(p)?,
                &format!("mf{}", i + 1),
            )?))
        })
        .collect()
}
pub(super) fn variable(
    e: &mut Engine,
    r: [f64; 2],
    name: &str,
    n: usize,
    t: &str,
    inputs: usize,
) -> Result<Record> {
    let mfs = default_mfs(e, r, n, t, inputs)?;
    Ok(BTreeMap::from([
        ("Name".into(), text(name, e)?),
        ("Range".into(), numbers(r.to_vec())?),
        (
            "MembershipFunctions".into(),
            new_array(e, Object::vector("fismf", mfs))?,
        ),
    ]))
}
pub(super) fn variable_constructor(e: &mut Engine, a: &[Value]) -> Result<Value> {
    let positional = a.first().is_some_and(|v| matches!(v, Value::Matrix(_)));
    let r = if positional {
        range(&a[0].clone().array(&e.cancel)?)?
    } else {
        [0., 1.]
    };
    let opts = options(e, &a[usize::from(positional)..], &["Name"])?;
    let name = opts
        .get("Name")
        .map(string)
        .transpose()?
        .unwrap_or("var".into());
    let p = variable(e, r, &name, 0, "trimf", 0)?;
    new(e, Object::scalar("fisvar", p))
}
pub(super) fn constructor(e: &mut Engine, class: &str, a: &[Value]) -> Result<Value> {
    let opts = options(
        e,
        a,
        &[
            "Name",
            "NumInputs",
            "NumInputMFs",
            "NumOutputs",
            "NumOutputMFs",
            "AndMethod",
            "OrMethod",
            "ImplicationMethod",
            "AggregationMethod",
            "DefuzzificationMethod",
            "MFType",
            "DisableStructuralChecks",
            "AddRules",
        ],
    )?;
    let size = |k: &str, d: usize, m: usize, pos: bool| {
        opts.get(k)
            .map(|v| count(v, m, pos))
            .transpose()
            .map(|n| n.unwrap_or(d))
    };
    let ni = size("NumInputs", 0, MAX_VARS, false)?;
    let no = size("NumOutputs", 0, MAX_VARS, false)?;
    let nmi = size("NumInputMFs", 3, MAX_MFS, true)?;
    let nmo = size("NumOutputMFs", 3, MAX_MFS, true)?;
    let t = opts
        .get("MFType")
        .map(string)
        .transpose()?
        .unwrap_or("trimf".into());
    if !["trimf", "gaussmf"].contains(&t.as_str()) {
        return Err("MFType must be trimf or gaussmf".into());
    }
    let addrules = opts
        .get("AddRules")
        .map(string)
        .transpose()?
        .unwrap_or("allcombinations".into());
    if !["allcombinations", "none"].contains(&addrules.as_str()) {
        return Err("AddRules must be allcombinations or none".into());
    }
    let nr = if ni > 0 && no > 0 && addrules == "allcombinations" {
        (0..ni)
            .try_fold(1usize, |a, _| {
                a.checked_mul(nmi).filter(|&n| n <= MAX_ITEMS)
            })
            .ok_or("fuzzy: generated rule count exceeds 65536 limit")?
    } else {
        0
    };
    let estimated =
        (ni * nmi + no * nmo).saturating_mul(1024) + nr.saturating_mul(512 + (ni + no) * 8);
    if estimated > e.limits.memory_bytes {
        return Err("fuzzy: workspace memory limit exceeded".into());
    }
    e.charge(estimated / 8)?;
    let mut p = Record::new();
    for (k, d) in [
        ("Name", "fis"),
        ("AndMethod", if class == "mamfis" { "min" } else { "prod" }),
        ("OrMethod", if class == "mamfis" { "max" } else { "probor" }),
        (
            "ImplicationMethod",
            if class == "mamfis" { "min" } else { "prod" },
        ),
        (
            "AggregationMethod",
            if class == "mamfis" { "max" } else { "sum" },
        ),
        (
            "DefuzzificationMethod",
            if class == "mamfis" {
                "centroid"
            } else {
                "wtaver"
            },
        ),
    ] {
        p.insert(k.into(), opts.get(k).cloned().unwrap_or(text(d, e)?));
    }
    p.insert(
        "Type".into(),
        text(
            if class == "mamfis" {
                "mamdani"
            } else {
                "sugeno"
            },
            e,
        )?,
    );
    p.insert(
        "DisableStructuralChecks".into(),
        opts.get("DisableStructuralChecks")
            .cloned()
            .unwrap_or(Array::Logical(crate::LogicalMatrix::scalar(false))),
    );
    let inputs = (0..ni)
        .map(|i| variable(e, [0., 1.], &format!("input{}", i + 1), nmi, &t, ni).map(Arc::new))
        .collect::<Result<Vec<_>>>()?;
    let outputs = (0..no)
        .map(|i| {
            variable(
                e,
                [0., 1.],
                &format!("output{}", i + 1),
                nmo,
                if class == "mamfis" { &t } else { "constant" },
                ni,
            )
            .map(Arc::new)
        })
        .collect::<Result<Vec<_>>>()?;
    set_collection(e, &mut p, "Inputs", "fisvar", inputs)?;
    set_collection(e, &mut p, "Outputs", "fisvar", outputs)?;
    let mut rules = Vec::with_capacity(nr);
    for i in 0..nr {
        if i % 256 == 0 {
            e.check_cancel()?;
        }
        let mut k = i;
        let mut ant = vec![0.; ni];
        for j in (0..ni).rev() {
            ant[j] = (k % nmi + 1) as f64;
            k /= nmi;
        }
        rules.push(Arc::new(rules::numeric_record(
            e,
            ant,
            vec![1.; no],
            1.,
            1.,
        )?));
    }
    set_collection(e, &mut p, "Rules", "fisrule", rules)?;
    validate(e, class, &p)?;
    rules::refresh(e, &mut p)?;
    new(
        e,
        Object::scalar(
            if class == "mamfis" {
                "mamfis"
            } else {
                "sugfis"
            },
            p,
        ),
    )
}
pub(super) fn method_validate(e: &mut Engine, class: &str, key: &str, a: &Array) -> Result<()> {
    let t = mf::function_name(a)?;
    if matches!(a, Array::Handle(_)) {
        return Ok(());
    }
    let allowed: &[&str] = match key {
        "AndMethod" => &["min", "prod"],
        "OrMethod" => &["max", "probor"],
        "ImplicationMethod" if class == "sugfis" => &["prod"],
        "ImplicationMethod" => &["min", "prod"],
        "AggregationMethod" if class == "sugfis" => &["sum"],
        "AggregationMethod" => &["max", "probor", "sum"],
        "DefuzzificationMethod" if class == "sugfis" => &["wtaver", "wtsum"],
        "DefuzzificationMethod" => &["centroid", "bisector", "mom", "som", "lom"],
        _ => &[],
    };
    if allowed.contains(&t.as_str()) {
        return Ok(());
    }
    let reserved = [
        "min", "max", "prod", "probor", "sum", "wtaver", "wtsum", "centroid", "bisector", "mom",
        "som", "lom",
    ];
    if reserved.contains(&t.as_str())
        || !(crate::session::FUNCTIONS.contains(&t.as_str()) || e.resolve_function(&t)?.is_some())
    {
        return Err(format!("Invalid {key} '{t}' for {class}"));
    }
    Ok(())
}
pub(super) fn validate_variable(e: &Engine, p: &Record) -> Result<()> {
    string(&p["Name"])?;
    range(&p["Range"])?;
    let mfs = collection(e, p, "MembershipFunctions")?;
    if mfs.len() > MAX_MFS || (mfs.rows > 1 && mfs.cols > 1) {
        return Err("MembershipFunctions must be a vector of at most 256 fismf objects".into());
    }
    unique_names(&mfs.items, "membership function")?;
    Ok(())
}
fn unique_names(items: &[Arc<Record>], kind: &str) -> Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    for p in items {
        let name = string(&p["Name"])?;
        if !seen.insert(name.clone()) {
            return Err(format!("Duplicate {kind} name '{name}'"));
        }
    }
    Ok(())
}
pub(super) fn validate(e: &mut Engine, class: &str, p: &Record) -> Result<()> {
    string(&p["Name"])?;
    for key in METHODS {
        method_validate(e, class, key, &p[*key])?;
    }
    let checks = scalar(&p["DisableStructuralChecks"])?;
    if checks != 0. && checks != 1. {
        return Err("DisableStructuralChecks must be a logical scalar".into());
    }
    let inputs = collection(e, p, "Inputs")?;
    let outputs = collection(e, p, "Outputs")?;
    let rules = collection(e, p, "Rules")?;
    if inputs.len() > MAX_VARS || outputs.len() > MAX_VARS || rules.len() > MAX_ITEMS {
        return Err("fuzzy: object dimension limit exceeded".into());
    }
    for vars in [&inputs, &outputs] {
        if vars.rows > 1 && vars.cols > 1 {
            return Err("Inputs and Outputs must be vectors of fisvar objects".into());
        }
    }
    if rules.rows > 1 && rules.cols > 1 {
        return Err("Rules must be a vector of fisrule objects".into());
    }
    if checks == 1. {
        return Ok(());
    }
    unique_names(&inputs.items, "input")?;
    unique_names(&outputs.items, "output")?;
    for (output, vars) in [(false, &inputs), (true, &outputs)] {
        for v in vars.items.iter() {
            validate_variable(e, v)?;
            for m in collection(e, v, "MembershipFunctions")?.items.iter() {
                let t = mf::function_name(&m["Type"])?;
                if class == "sugfis" && output {
                    if !["constant", "linear"].contains(&t.as_str()) {
                        return Err(
                            "Sugeno output membership functions must be constant or linear".into(),
                        );
                    }
                    mf::validate(&t, &vecnum(&m["Parameters"])?, Some(inputs.len()))?;
                } else if ["constant", "linear"].contains(&t.as_str()) {
                    return Err("constant and linear membership functions are only valid for Sugeno outputs".into());
                }
            }
        }
    }
    for r in rules.items.iter() {
        rules::validate_indices(e, r, &inputs, &outputs, class)?;
    }
    Ok(())
}
pub(super) fn locate(
    e: &Engine,
    p: &Record,
    name: &Array,
    kind: Option<&str>,
) -> Result<(bool, usize)> {
    let name = string(name)?;
    if kind.is_some_and(|k| !matches!(k, "input" | "output")) {
        return Err("VariableType must be input or output".into());
    }
    let mut found = Vec::new();
    for (output, key) in [(false, "Inputs"), (true, "Outputs")] {
        if kind.is_some_and(|k| (k == "output") != output) {
            continue;
        }
        for (i, v) in collection(e, p, key)?.items.iter().enumerate() {
            if string(&v["Name"])? == name {
                found.push((output, i));
            }
        }
    }
    match found.as_slice() {
        [v] => Ok(*v),
        [] => Err(format!("Unknown variable name '{name}'")),
        _ => Err(format!(
            "Variable name '{name}' is ambiguous; specify VariableType"
        )),
    }
}
fn unique_default(items: &[Arc<Record>], prefix: &str) -> String {
    let mut i = items.len() + 1;
    loop {
        let name = format!("{prefix}{i}");
        if !items
            .iter()
            .any(|r| string(&r["Name"]).is_ok_and(|n| n == name))
        {
            return name;
        }
        i += 1;
    }
}
pub(super) fn edit(e: &mut Engine, n: &str, a: &[Value]) -> Result<Value> {
    if a.is_empty() {
        return Err(format!("{n}: expected a fuzzy object"));
    }
    let o = object(e, &a[0])?;
    let mut p = o.record()?.clone();
    if matches!(n, "addInput" | "addOutput") {
        if !is_system(o.class) {
            return Err(format!("{n}: expected a FIS"));
        }
        let output = n == "addOutput";
        let key = if output { "Outputs" } else { "Inputs" };
        let positional = a.get(1).is_some_and(|v| matches!(v, Value::Matrix(_)));
        let r = if positional {
            range(&a[1].clone().array(&e.cancel)?)?
        } else {
            [0., 1.]
        };
        let opts = options(
            e,
            &a[1 + usize::from(positional)..],
            &["Name", "NumMFs", "MFType"],
        )?;
        let vars = collection(e, &p, key)?;
        if vars.len() >= MAX_VARS {
            return Err("fuzzy: variable count exceeds 64 limit".into());
        }
        let name = opts
            .get("Name")
            .map(string)
            .transpose()?
            .unwrap_or_else(|| {
                unique_default(&vars.items, if output { "output" } else { "input" })
            });
        let count = opts
            .get("NumMFs")
            .map(|v| count(v, MAX_MFS, false))
            .transpose()?
            .unwrap_or(0);
        let t = opts.get("MFType").map(string).transpose()?.unwrap_or(
            if output && o.class == "sugfis" {
                "constant"
            } else {
                "trimf"
            }
            .into(),
        );
        if (!output || o.class == "mamfis") && !matches!(t.as_str(), "trimf" | "gaussmf") {
            return Err("MFType must be trimf or gaussmf for this variable".into());
        }
        if output && o.class == "sugfis" && !matches!(t.as_str(), "constant" | "linear") {
            return Err("Sugeno output MFType must be constant or linear".into());
        }
        let ni = collection(e, &p, "Inputs")?.len();
        if output && t == "linear" && ni == 0 {
            return Err("Linear output membership functions require at least one input".into());
        }
        let var = variable(e, r, &name, count, &t, ni)?;
        let mut items = vars.items.as_ref().clone();
        items.push(Arc::new(var));
        set_collection(e, &mut p, key, "fisvar", items)?;
        rules::append_variable(e, &mut p, output)?;
        if !output && o.class == "sugfis" {
            adjust_linear(e, &mut p, ni, None)?;
        }
    } else if matches!(n, "removeInput" | "removeOutput") {
        if !is_system(o.class) || a.len() != 2 {
            return Err(format!("{n}: expected FIS and variable name"));
        }
        let output = n == "removeOutput";
        let key = if output { "Outputs" } else { "Inputs" };
        let (_, i) = locate(
            e,
            &p,
            &a[1].clone().array(&e.cancel)?,
            Some(if output { "output" } else { "input" }),
        )?;
        let mut vars = collection(e, &p, key)?.items.as_ref().clone();
        vars.remove(i);
        set_collection(e, &mut p, key, "fisvar", vars)?;
        rules::remove_variable(e, &mut p, output, i)?;
        if !output && o.class == "sugfis" {
            let ni = collection(e, &p, "Inputs")?.len();
            adjust_linear(e, &mut p, ni, Some(i))?;
        }
    } else if matches!(n, "addMF" | "removeMF") {
        let system = is_system(o.class);
        if !system && o.class != "fisvar" {
            return Err(format!("{n}: expected a FIS or fisvar"));
        }
        let option_start = if n == "addMF" {
            let start = if system { 2 } else { 1 };
            if a.get(start).is_some_and(|v| {
                !vtext(e, v).is_ok_and(|s| {
                    s.eq_ignore_ascii_case("Name") || s.eq_ignore_ascii_case("VariableType")
                })
            }) {
                start + 2
            } else {
                start
            }
        } else {
            if system { 3 } else { 2 }
        };
        if a.len() < option_start {
            return Err(format!("{n}: too few inputs"));
        }
        let opts = options(e, &a[option_start..], &["Name", "VariableType"])?;
        let kind = opts.get("VariableType").map(string).transpose()?;
        let (output, idx) = if system {
            if a.len() < 2 {
                return Err(format!("{n}: expected variable name"));
            }
            locate(e, &p, &a[1].clone().array(&e.cancel)?, kind.as_deref())?
        } else {
            (false, 0)
        };
        let key = if output { "Outputs" } else { "Inputs" };
        let vars = if system {
            Some(collection(e, &p, key)?)
        } else {
            None
        };
        let mut var = if let Some(vars) = &vars {
            vars.items[idx].as_ref().clone()
        } else {
            p.clone()
        };
        let mut mfs = collection(e, &var, "MembershipFunctions")?
            .items
            .as_ref()
            .clone();
        if n == "addMF" {
            if mfs.len() >= MAX_MFS {
                return Err("fuzzy: membership function count exceeds 256 limit".into());
            }
            let start = if system { 2 } else { 1 };
            let (t, params) = if option_start == start + 2 {
                (
                    a[start].clone().array(&e.cancel)?,
                    a[start + 1].clone().array(&e.cancel)?,
                )
            } else {
                (
                    text(
                        if output && o.class == "sugfis" {
                            "constant"
                        } else {
                            "trimf"
                        },
                        e,
                    )?,
                    numbers(if output && o.class == "sugfis" {
                        vec![0.]
                    } else {
                        vec![0., 0.5, 1.]
                    })?,
                )
            };
            let name = opts
                .get("Name")
                .map(string)
                .transpose()?
                .unwrap_or_else(|| unique_default(&mfs, "mf"));
            mfs.push(Arc::new(mf::record(e, t, params, &name)?));
        } else {
            let name = vtext(e, &a[if system { 2 } else { 1 }])?;
            let i = mfs
                .iter()
                .position(|p| string(&p["Name"]).is_ok_and(|s| s == name))
                .ok_or_else(|| format!("Unknown membership function name '{name}'"))?;
            mfs.remove(i);
            if system {
                rules::remove_mf(e, &mut p, output, idx, i)?;
            }
        }
        set_collection(e, &mut var, "MembershipFunctions", "fismf", mfs)?;
        if let Some(vars) = vars {
            let mut items = vars.items.as_ref().clone();
            items[idx] = Arc::new(var);
            set_collection(e, &mut p, key, "fisvar", items)?;
        } else {
            p = var;
        }
    }
    if is_system(o.class) {
        validate(e, o.class, &p)?;
        rules::refresh(e, &mut p)?;
    } else {
        validate_variable(e, &p)?;
    }
    new(e, Object::scalar(o.class, p))
}
fn adjust_linear(
    e: &mut Engine,
    p: &mut Record,
    inputs: usize,
    removed: Option<usize>,
) -> Result<()> {
    let outputs = collection(e, p, "Outputs")?;
    let mut vars = outputs.items.as_ref().clone();
    for v in &mut vars {
        let mfs = collection(e, v, "MembershipFunctions")?;
        let mut members = mfs.items.as_ref().clone();
        let mut changed = false;
        for m in &mut members {
            if mf::function_name(&m["Type"])? == "linear" {
                let mut params = vecnum(&m["Parameters"])?;
                if let Some(i) = removed {
                    params.remove(i);
                } else {
                    params.insert(inputs, 0.);
                }
                Arc::make_mut(m).insert("Parameters".into(), numbers(params)?);
                changed = true;
            }
        }
        if changed {
            set_collection(e, Arc::make_mut(v), "MembershipFunctions", "fismf", members)?;
        }
    }
    set_collection(e, p, "Outputs", "fisvar", vars)
}
