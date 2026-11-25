use super::*;
#[derive(Clone)]
struct Term {
    var: String,
    mf: String,
    neg: bool,
}
struct Parsed {
    ant: Vec<Term>,
    con: Vec<Term>,
    weight: f64,
    connection: f64,
}
fn regex(pattern: &str) -> Result<regex::Regex> {
    regex::RegexBuilder::new(pattern)
        .unicode(false)
        .build()
        .map_err(|e| e.to_string())
}
fn weight_connection(w: f64, c: f64) -> Result<()> {
    if !w.is_finite() || !(0.0..=1.0).contains(&w) {
        return Err("Rule Weight must be in [0,1]".into());
    }
    if c != 1. && c != 2. {
        return Err("Rule Connection must be 1 (AND) or 2 (OR)".into());
    }
    Ok(())
}
fn parse(s: &str) -> Result<Parsed> {
    if s.len() > 16384 {
        return Err("Fuzzy rule text exceeds 16384-byte limit".into());
    }
    let mut s = regex(r"^\s*\d+\.\s+")?.replace(s.trim(), "").into_owned();
    let mut weight = 1.;
    if s.ends_with(')')
        && let Some(pos) = s.rfind('(')
        && let Ok(w) = s[pos + 1..s.len() - 1].trim().parse::<f64>()
    {
        weight = w;
        s.truncate(pos);
    }
    weight_connection(weight, 1.)?;
    let verbose = s
        .trim_start()
        .get(..3)
        .is_some_and(|p| p.eq_ignore_ascii_case("if "));
    if verbose {
        s = s.trim_start()[3..].into();
        for (pattern, to) in [
            (r"(?i)\s+then\s+", " => "),
            (r"(?i)\s+is\s+not\s+", "~="),
            (r"(?i)\s+is\s+", "=="),
            (r"(?i)\s+and\s+", " & "),
            (r"(?i)\s+or\s+", " | "),
        ] {
            s = regex(pattern)?.replace_all(&s, to).into_owned();
        }
    }
    let (left, right) = s
        .split_once("=>")
        .ok_or("Fuzzy rule must contain THEN or =>")?;
    if right.contains("=>") {
        return Err("Fuzzy rule has more than one consequent separator".into());
    }
    let has_or = left.contains('|');
    let has_and = left.contains('&');
    if has_or && has_and {
        return Err("A fuzzy rule cannot mix AND and OR connections".into());
    }
    let parse_term = |term: &str, output: bool| -> Result<Term> {
        let term = term
            .trim()
            .trim_start_matches('(')
            .trim_end_matches(')')
            .trim();
        let (var, mf, neg) = if let Some((a, b)) = term.split_once("~=") {
            (a, b, true)
        } else if let Some((a, b)) = term.split_once("==") {
            (a, b, false)
        } else if output {
            let (a, b) = term.split_once('=').ok_or("Invalid fuzzy rule condition")?;
            (a, b, false)
        } else {
            return Err("Antecedent conditions must use == or ~= (IS or IS NOT)".into());
        };
        let var = var.trim();
        let mf = mf.trim();
        if var.is_empty()
            || mf.is_empty()
            || var
                .chars()
                .chain(mf.chars())
                .any(|c| "()=~&|,<>".contains(c))
        {
            return Err("Unsupported fuzzy rule syntax".into());
        }
        Ok(Term {
            var: var.into(),
            mf: mf.into(),
            neg,
        })
    };
    let ant = left
        .split(['&', '|'])
        .map(|t| parse_term(t, false))
        .collect::<Result<Vec<_>>>()?;
    let con = right
        .split(',')
        .map(|t| parse_term(t, true))
        .collect::<Result<Vec<_>>>()?;
    Ok(Parsed {
        ant,
        con,
        weight,
        connection: if has_or { 2. } else { 1. },
    })
}
fn canonical(p: &Parsed, format: &str) -> String {
    let verbose = format == "verbose";
    let term = |t: &Term, output: bool| {
        if verbose {
            format!("({} is {}{})", t.var, if t.neg { "not " } else { "" }, t.mf)
        } else {
            format!(
                "{}{}{}",
                t.var,
                if t.neg {
                    "~="
                } else if output {
                    "="
                } else {
                    "=="
                },
                t.mf
            )
        }
    };
    let ant = p
        .ant
        .iter()
        .map(|t| term(t, false))
        .collect::<Vec<_>>()
        .join(if p.connection == 2. {
            if verbose { " or " } else { " | " }
        } else if verbose {
            " and "
        } else {
            " & "
        });
    let con = p
        .con
        .iter()
        .map(|t| term(t, true))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{}{}{}{} ({})",
        if verbose { "If " } else { "" },
        ant,
        if verbose { " then " } else { " => " },
        con,
        p.weight
    )
}
pub(super) fn numeric_record(
    e: &Engine,
    ant: Vec<f64>,
    con: Vec<f64>,
    weight: f64,
    connection: f64,
) -> Result<Record> {
    weight_connection(weight, connection)?;
    for v in ant.iter().chain(&con) {
        if !v.is_finite() || v.fract() != 0. || v.abs() > MAX_MFS as f64 {
            return Err("Rule indices must be integers in membership function range".into());
        }
    }
    let terms = |v: &[f64], prefix: &str| {
        v.iter()
            .enumerate()
            .filter(|&(_, &v)| v != 0.)
            .map(|(i, &v)| Term {
                var: format!("{prefix}{}", i + 1),
                mf: format!("mf{}", v.abs()),
                neg: v < 0.,
            })
            .collect()
    };
    let parsed = Parsed {
        ant: terms(&ant, "input"),
        con: terms(&con, "output"),
        weight,
        connection,
    };
    Ok(BTreeMap::from([
        (
            "Description".into(),
            text(&canonical(&parsed, "symbolic"), e)?,
        ),
        ("Antecedent".into(), numbers(ant)?),
        ("Consequent".into(), numbers(con)?),
        ("Weight".into(), num(weight)),
        ("Connection".into(), num(connection)),
    ]))
}
fn text_record(e: &Engine, s: &str) -> Result<Record> {
    let parsed = parse(s)?;
    let mut r = numeric_record(e, vec![], vec![], parsed.weight, parsed.connection)?;
    r.insert(
        "Description".into(),
        text(&canonical(&parsed, "symbolic"), e)?,
    );
    Ok(r)
}
fn texts(e: &Engine, a: &Array) -> Result<Vec<String>> {
    match a {
        Array::Str(s) => s
            .data()
            .iter()
            .map(|s| {
                s.as_ref()
                    .map(|s| s.to_string())
                    .ok_or_else(|| "Missing rule text".into())
            })
            .collect(),
        Array::Cell(c) => (0..c.len()).map(|i| string(c.get_linear(i)?)).collect(),
        Array::Character(c) => {
            let mut rows = Vec::new();
            for r in 0..c.rows() {
                let line = (0..c.cols())
                    .map(|k| c.units()[r + k * c.rows()])
                    .collect::<Vec<_>>();
                rows.push(String::from_utf16_lossy(&line).trim_end().into());
            }
            Ok(rows)
        }
        _ => {
            let _ = e;
            Err("Expected numeric rows, rule text, string array, or cells of rule text".into())
        }
    }
}
fn numeric_rows(e: &Engine, m: &Matrix, ni: usize, no: usize) -> Result<Vec<Arc<Record>>> {
    if m.is_nd() || m.cols() != ni + no + 2 {
        return Err("Vector description: Rule length must be m+n+2, where m is the number of inputs and n is the number of outputs".into());
    }
    if m.rows() > MAX_ITEMS {
        return Err("fuzzy: rule count exceeds limit".into());
    }
    (0..m.rows())
        .map(|r| {
            e.check_cancel()?;
            numeric_record(
                e,
                (0..ni).map(|c| m.get(r, c)).collect(),
                (ni..ni + no).map(|c| m.get(r, c)).collect(),
                m.get(r, ni + no),
                m.get(r, ni + no + 1),
            )
            .map(Arc::new)
        })
        .collect()
}
pub(super) fn constructor(e: &mut Engine, a: &[Value]) -> Result<Value> {
    let records = match a {
        [] => vec![Arc::new(numeric_record(e, vec![1.], vec![1.], 1., 1.)?)],
        [v] => texts(e, &v.clone().array(&e.cancel)?)?
            .iter()
            .map(|s| text_record(e, s).map(Arc::new))
            .collect::<Result<Vec<_>>>()?,
        [v, ni] => {
            let m = matrix(e, v)?;
            let ni = count(&ni.clone().array(&e.cancel)?, MAX_VARS, true)?;
            let no = m
                .cols()
                .checked_sub(ni + 2)
                .filter(|&n| n > 0 && n <= MAX_VARS)
                .ok_or("fisrule: invalid number of input or output variables")?;
            numeric_rows(e, &m, ni, no)?
        }
        _ => return Err("fisrule: expected text or (numericRules,numInputs)".into()),
    };
    new(e, Object::vector("fisrule", records))
}
fn resolve(e: &Engine, parsed: &Parsed, inputs: &Object, outputs: &Object) -> Result<Record> {
    let indices = |terms: &[Term], vars: &Object| -> Result<Vec<f64>> {
        let mut result = vec![0.; vars.len()];
        for term in terms {
            let i = vars
                .items
                .iter()
                .position(|v| string(&v["Name"]).is_ok_and(|s| s == term.var))
                .ok_or_else(|| {
                    format!("Unknown variable name '{}' in rule description", term.var)
                })?;
            let mfs = fis::collection(e, &vars.items[i], "MembershipFunctions")?;
            let j = mfs
                .items
                .iter()
                .position(|v| string(&v["Name"]).is_ok_and(|s| s == term.mf))
                .ok_or_else(|| {
                    format!(
                        "Unknown membership function '{}' for variable '{}'",
                        term.mf, term.var
                    )
                })?;
            if result[i] != 0. {
                return Err(format!(
                    "Repeated variable '{}' in rule description",
                    term.var
                ));
            }
            result[i] = (j + 1) as f64 * if term.neg { -1. } else { 1. };
        }
        Ok(result)
    };
    numeric_record(
        e,
        indices(&parsed.ant, inputs)?,
        indices(&parsed.con, outputs)?,
        parsed.weight,
        parsed.connection,
    )
}
pub(super) fn resolve_descriptions(e: &mut Engine, p: &mut Record, class: &str) -> Result<()> {
    let inputs = fis::collection(e, p, "Inputs")?;
    let outputs = fis::collection(e, p, "Outputs")?;
    let old = fis::collection(e, p, "Rules")?;
    let mut rules = Vec::with_capacity(old.len());
    for rule in old.items.iter() {
        e.check_cancel()?;
        let parsed = parse(&string(&rule["Description"])?)?;
        let mut resolved = resolve(e, &parsed, &inputs, &outputs)?;
        resolved.insert(
            "Description".into(),
            text(&canonical(&parsed, "symbolic"), e)?,
        );
        validate_indices(e, &resolved, &inputs, &outputs, class)?;
        rules.push(Arc::new(resolved));
    }
    fis::set_collection(e, p, "Rules", "fisrule", rules)
}
pub(super) fn validate_indices(
    e: &Engine,
    r: &Record,
    inputs: &Object,
    outputs: &Object,
    class: &str,
) -> Result<()> {
    weight_connection(scalar(&r["Weight"])?, scalar(&r["Connection"])?)?;
    for (key, vars) in [("Antecedent", inputs), ("Consequent", outputs)] {
        let v = vecnum(&r[key])?;
        if v.len() != vars.len() {
            return Err(format!(
                "Rule {key} length must match the number of variables"
            ));
        }
        if !v.iter().any(|&v| v != 0.) {
            return Err(format!(
                "Rule {key} must contain at least one nonzero membership function index"
            ));
        }
        for (&v, var) in v.iter().zip(vars.items.iter()) {
            if !v.is_finite()
                || v.fract() != 0.
                || v.abs() > fis::collection(e, var, "MembershipFunctions")?.len() as f64
            {
                return Err(format!("Rule {key} membership function index out of range"));
            }
            if key == "Consequent" && class == "sugfis" && v < 0. {
                return Err("Sugeno rule consequents do not support NOT".into());
            }
        }
    }
    Ok(())
}
fn parsed_indices(e: &Engine, r: &Record, inputs: &Object, outputs: &Object) -> Result<Parsed> {
    let terms = |key: &str, vars: &Object| -> Result<Vec<Term>> {
        let v = vecnum(&r[key])?;
        let mut terms = Vec::new();
        for (i, &v) in v.iter().enumerate().filter(|(_, v)| **v != 0.) {
            let var = vars
                .items
                .get(i)
                .ok_or("Rule variable index out of range")?;
            let mfs = fis::collection(e, var, "MembershipFunctions")?;
            let j = v.abs() as usize;
            let mf = j
                .checked_sub(1)
                .and_then(|j| mfs.items.get(j))
                .ok_or("Rule membership function index out of range")?;
            terms.push(Term {
                var: string(&var["Name"])?,
                mf: string(&mf["Name"])?,
                neg: v < 0.,
            });
        }
        Ok(terms)
    };
    Ok(Parsed {
        ant: terms("Antecedent", inputs)?,
        con: terms("Consequent", outputs)?,
        weight: scalar(&r["Weight"])?,
        connection: scalar(&r["Connection"])?,
    })
}
pub(super) fn refresh(e: &mut Engine, p: &mut Record) -> Result<()> {
    if scalar(&p["DisableStructuralChecks"])? != 0. {
        return Ok(());
    }
    let inputs = fis::collection(e, p, "Inputs")?;
    let outputs = fis::collection(e, p, "Outputs")?;
    let mut rules = fis::collection(e, p, "Rules")?.items.as_ref().clone();
    for r in &mut rules {
        e.check_cancel()?;
        let s = canonical(&parsed_indices(e, r, &inputs, &outputs)?, "symbolic");
        if string(&r["Description"])? != s {
            Arc::make_mut(r).insert("Description".into(), text(&s, e)?);
        }
    }
    fis::set_collection(e, p, "Rules", "fisrule", rules)
}
pub(super) fn add(e: &mut Engine, a: &[Value]) -> Result<Value> {
    if a.is_empty() || a.len() > 2 {
        return Err("addRule: expected FIS and rules".into());
    }
    let o = object(e, &a[0])?;
    if !fis::is_system(o.class) {
        return Err("addRule: expected a FIS".into());
    }
    let mut p = o.record()?.clone();
    let inputs = fis::collection(e, &p, "Inputs")?;
    let outputs = fis::collection(e, &p, "Outputs")?;
    let mut rules = fis::collection(e, &p, "Rules")?.items.as_ref().clone();
    let arg = if let Some(v) = a.get(1) {
        v.clone().array(&e.cancel)?
    } else {
        text("input1==mf1 => output1=mf1", e)?
    };
    let more = if let Some(o) = super::super::objects::fis_value(e, &Value::from(arg.clone())) {
        if o.class != "fisrule" {
            return Err("addRule: expected fisrule objects".into());
        }
        o.items
            .iter()
            .map(|r| {
                if vecnum(&r["Antecedent"])?.is_empty() {
                    resolve(e, &parse(&string(&r["Description"])?)?, &inputs, &outputs)
                        .map(Arc::new)
                } else {
                    Ok(r.clone())
                }
            })
            .collect::<Result<Vec<_>>>()?
    } else if let Array::Numeric(m) = &arg {
        numeric_rows(e, m, inputs.len(), outputs.len())?
    } else {
        texts(e, &arg)?
            .iter()
            .map(|s| {
                // The indexed showrule format is accepted as well for round trips.
                if s.contains(':') && !s.contains("=>") {
                    parse_indexed(e, s, inputs.len(), outputs.len()).map(Arc::new)
                } else {
                    resolve(e, &parse(s)?, &inputs, &outputs).map(Arc::new)
                }
            })
            .collect::<Result<Vec<_>>>()?
    };
    if rules.len().saturating_add(more.len()) > MAX_ITEMS {
        return Err("fuzzy: rule count exceeds 65536 limit".into());
    }
    for r in &more {
        validate_indices(e, r, &inputs, &outputs, o.class)?;
    }
    rules.extend(more);
    fis::set_collection(e, &mut p, "Rules", "fisrule", rules)?;
    refresh(e, &mut p)?;
    new(e, Object::scalar(o.class, p))
}
fn parse_indexed(e: &Engine, s: &str, ni: usize, no: usize) -> Result<Record> {
    let (body, connection) = s.split_once(':').ok_or("Invalid indexed rule")?;
    let connection = connection
        .trim()
        .parse::<f64>()
        .map_err(|_| "Invalid indexed rule connection")?;
    let start = body.rfind('(').ok_or("Invalid indexed rule weight")?;
    let weight = body[start + 1..]
        .trim()
        .trim_end_matches(')')
        .parse::<f64>()
        .map_err(|_| "Invalid indexed rule weight")?;
    let (ant, con) = body[..start]
        .split_once(',')
        .ok_or("Invalid indexed rule separator")?;
    let nums = |s: &str| {
        s.split_whitespace()
            .map(|v| {
                v.parse::<f64>()
                    .map_err(|_| "Invalid indexed rule index".into())
            })
            .collect::<Result<Vec<_>>>()
    };
    let ant = nums(ant)?;
    let con = nums(con)?;
    if ant.len() != ni || con.len() != no {
        return Err("Indexed rule dimensions do not match FIS".into());
    }
    numeric_record(e, ant, con, weight, connection)
}
pub(super) fn show(e: &mut Engine, a: &[Value]) -> Result<Value> {
    if a.is_empty() {
        return Err("showrule: expected a FIS".into());
    }
    let o = object(e, &a[0])?;
    if !fis::is_system(o.class) {
        return Err("showrule: expected a FIS".into());
    }
    let p = o.record()?;
    let opts = options(e, &a[1..], &["Format", "RuleIndex", "Language"])?;
    let format = opts
        .get("Format")
        .map(string)
        .transpose()?
        .unwrap_or("verbose".into());
    if !["verbose", "symbolic", "indexed"].contains(&format.as_str()) {
        return Err("showrule: Format must be verbose, symbolic or indexed".into());
    }
    if opts
        .get("Language")
        .map(string)
        .transpose()?
        .is_some_and(|s| !matches!(s.as_str(), "english" | "none"))
    {
        return Err("showrule: non-English languages are not supported yet".into());
    }
    let inputs = fis::collection(e, p, "Inputs")?;
    let outputs = fis::collection(e, p, "Outputs")?;
    let rules = fis::collection(e, p, "Rules")?;
    let ids = opts
        .get("RuleIndex")
        .map(vecnum)
        .transpose()?
        .unwrap_or_else(|| (1..=rules.len()).map(|i| i as f64).collect());
    let mut lines = Vec::new();
    let mut chars = 0usize;
    for id in ids {
        if !id.is_finite() || id.fract() != 0. || id < 1. || id > rules.len() as f64 {
            return Err("showrule: rule index out of range".into());
        }
        let r = &rules.items[id as usize - 1];
        let line = if format == "indexed" {
            format!(
                "{}, {} ({}) : {}",
                vecnum(&r["Antecedent"])?
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" "),
                vecnum(&r["Consequent"])?
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" "),
                scalar(&r["Weight"])?,
                scalar(&r["Connection"])?
            )
        } else {
            format!(
                "{}. {}",
                id,
                canonical(&parsed_indices(e, r, &inputs, &outputs)?, &format)
            )
        };
        chars = chars.saturating_add(line.len());
        if chars > e.limits.memory_bytes / 2 {
            return Err("showrule: output size limit exceeded".into());
        }
        lines.push(line);
    }
    let rows = lines.len();
    let cols = lines
        .iter()
        .map(|s| s.encode_utf16().count())
        .max()
        .unwrap_or(0);
    CharacterRow::checked_len(rows, cols)?;
    if rows.saturating_mul(cols).saturating_mul(2) > e.limits.memory_bytes {
        return Err("showrule: output size limit exceeded".into());
    }
    let mut units = vec![32u16; rows * cols];
    for (r, line) in lines.iter().enumerate() {
        for (c, u) in line.encode_utf16().enumerate() {
            units[r + c * rows] = u;
        }
    }
    Ok(Value::from(Array::Character(
        CharacterRow::from_column_major(rows, cols, units, &e.cancel)?,
    )))
}
pub(super) fn validate_property(e: &Engine, r: &mut Record, name: &str) -> Result<()> {
    if name == "Description" {
        let parsed = parse(&string(&r["Description"])?)?;
        r.insert(
            "Description".into(),
            text(&canonical(&parsed, "symbolic"), e)?,
        );
        r.insert("Weight".into(), num(parsed.weight));
        r.insert("Connection".into(), num(parsed.connection));
        return Ok(());
    }
    weight_connection(scalar(&r["Weight"])?, scalar(&r["Connection"])?)?;
    for key in ["Antecedent", "Consequent"] {
        let v = vecnum(&r[key])?;
        for &x in &v {
            if !x.is_finite() || x.fract() != 0. || x.abs() > MAX_MFS as f64 {
                return Err("Rule indices must be integers in membership function range".into());
            }
        }
        r.insert(key.into(), numbers(v)?);
    }
    if matches!(name, "Weight" | "Connection") {
        let mut parsed = parse(&string(&r["Description"])?)?;
        parsed.weight = scalar(&r["Weight"])?;
        parsed.connection = scalar(&r["Connection"])?;
        r.insert(
            "Description".into(),
            text(&canonical(&parsed, "symbolic"), e)?,
        );
    }
    Ok(())
}
pub(super) fn append_variable(e: &mut Engine, p: &mut Record, output: bool) -> Result<()> {
    let key = if output { "Consequent" } else { "Antecedent" };
    let mut rules = fis::collection(e, p, "Rules")?.items.as_ref().clone();
    for r in &mut rules {
        let mut v = vecnum(&r[key])?;
        v.push(0.);
        Arc::make_mut(r).insert(key.into(), numbers(v)?);
    }
    fis::set_collection(e, p, "Rules", "fisrule", rules)
}
pub(super) fn remove_variable(
    e: &mut Engine,
    p: &mut Record,
    output: bool,
    i: usize,
) -> Result<()> {
    edit_indices(e, p, output, |v| {
        if i >= v.len() {
            return Err("Rule indices are inconsistent with FIS variables".into());
        }
        v.remove(i);
        Ok(())
    })
}
pub(super) fn remove_mf(
    e: &mut Engine,
    p: &mut Record,
    output: bool,
    var: usize,
    mf: usize,
) -> Result<()> {
    edit_indices(e, p, output, |v| {
        if var >= v.len() {
            return Err("Rule indices are inconsistent with FIS variables".into());
        }
        let val = v[var];
        let index = val.abs() as usize;
        if index == mf + 1 {
            v[var] = 0.;
        } else if index > mf + 1 {
            v[var] = val.signum() * (index - 1) as f64;
        }
        Ok(())
    })
}
fn edit_indices(
    e: &mut Engine,
    p: &mut Record,
    output: bool,
    edit: impl Fn(&mut Vec<f64>) -> Result<()>,
) -> Result<()> {
    let key = if output { "Consequent" } else { "Antecedent" };
    let old = fis::collection(e, p, "Rules")?;
    let mut rules = Vec::new();
    for r in old.items.iter() {
        e.check_cancel()?;
        let mut v = vecnum(&r[key])?;
        edit(&mut v)?;
        if v.iter().all(|&v| v == 0.) {
            continue;
        }
        let mut r = r.as_ref().clone();
        r.insert(key.into(), numbers(v)?);
        rules.push(Arc::new(r));
    }
    fis::set_collection(e, p, "Rules", "fisrule", rules)
}
