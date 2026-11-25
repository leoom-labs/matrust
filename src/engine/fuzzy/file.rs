//! Strict, bounded .fis text exchange; no expression evaluation or file includes.
use super::*;
use std::{fmt::Write as _, io::Read};
const MAX_FILE: usize = 16 * 1024 * 1024;
const METHODS: [(&str, &str); 5] = [
    ("AndMethod", "AndMethod"),
    ("OrMethod", "OrMethod"),
    ("ImpMethod", "ImplicationMethod"),
    ("AggMethod", "AggregationMethod"),
    ("DefuzzMethod", "DefuzzificationMethod"),
];
fn quote(s: &str) -> Result<String> {
    if s.len() > 65536 {
        return Err("writefis: string exceeds the 65536-byte line limit".into());
    }
    if s.contains(['\n', '\r', '\0']) {
        return Err("writefis: strings cannot contain newlines or NUL".into());
    }
    Ok(format!("'{}'", s.replace('\'', "''")))
}
fn name(a: &Array) -> Result<String> {
    let s = mf::function_name(a)?;
    if s.starts_with('@') {
        return Err("writefis: anonymous function handles cannot be stored in a FIS file".into());
    }
    quote(&s)
}
fn nums(v: &[f64]) -> String {
    v.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" ")
}
pub(super) fn write(e: &mut Engine, a: &[Value]) -> Result<Value> {
    if a.len() != 2 {
        return Err("writefis: expected fis and filename".into());
    }
    let o = eval::system(e, &a[0])?;
    let p = o.record()?;
    let path = e.file_path(&vtext(e, &a[1])?, ".fis")?;
    let ins = fis::collection(e, p, "Inputs")?;
    let outs = fis::collection(e, p, "Outputs")?;
    let rs = fis::collection(e, p, "Rules")?;
    let mut s = format!(
        "[System]\nName={}\nType={}\nVersion=2.0\nNumInputs={}\nNumOutputs={}\nNumRules={}\n",
        quote(&string(&p["Name"])?)?,
        quote(&string(&p["Type"])?)?,
        ins.len(),
        outs.len(),
        rs.len()
    );
    for (key, prop) in METHODS {
        writeln!(s, "{key}={}", name(&p[prop])?).unwrap();
    }
    // Optional property, ignored by older readers; preserves staged-edit state.
    writeln!(
        s,
        "DisableStructuralChecks={}",
        scalar(&p["DisableStructuralChecks"])?
    )
    .unwrap();
    for (kind, vars) in [("Input", &ins), ("Output", &outs)] {
        for (i, v) in vars.items.iter().enumerate() {
            e.check_cancel()?;
            let mfs = fis::collection(e, v, "MembershipFunctions")?;
            writeln!(
                s,
                "\n[{kind}{}]\nName={}\nRange=[{}]\nNumMFs={}",
                i + 1,
                quote(&string(&v["Name"])?)?,
                nums(&vecnum(&v["Range"])?),
                mfs.len()
            )
            .unwrap();
            for (j, m) in mfs.items.iter().enumerate() {
                writeln!(
                    s,
                    "MF{}={}:{},[{}]",
                    j + 1,
                    quote(&string(&m["Name"])?)?,
                    name(&m["Type"])?,
                    nums(&vecnum(&m["Parameters"])?)
                )
                .unwrap();
                if s.len() > MAX_FILE || s.len() > e.limits.memory_bytes {
                    return Err("writefis: file size limit exceeded".into());
                }
            }
        }
    }
    s.push_str("\n[Rules]\n");
    for r in rs.items.iter() {
        e.check_cancel()?;
        writeln!(
            s,
            "{} , {} ({}) : {}",
            nums(&vecnum(&r["Antecedent"])?),
            nums(&vecnum(&r["Consequent"])?),
            scalar(&r["Weight"])?,
            scalar(&r["Connection"])?
        )
        .unwrap();
        if s.len() > MAX_FILE || s.len() > e.limits.memory_bytes {
            return Err("writefis: file size limit exceeded".into());
        }
    }
    if s.lines().any(|line| line.len() > 65536) {
        return Err("writefis: line exceeds the 65536-byte size limit".into());
    }
    e.check_cancel()?;
    std::fs::write(&path, s)
        .map_err(|err| format!("writefis: cannot write '{}': {err}", path.display()))?;
    eval::value(0, 0, vec![])
}
fn err(line: usize, msg: impl std::fmt::Display) -> String {
    format!("readfis: Invalid FIS file at line {line}: {msg}")
}
fn quoted(s: &str) -> Result<(String, &str)> {
    let mut it = s.trim_start().char_indices().peekable();
    if it.next().map(|(_, c)| c) != Some('\'') {
        return Err("expected single-quoted string".into());
    }
    let mut value = String::new();
    while let Some((i, c)) = it.next() {
        if c == '\'' {
            if it.peek().is_some_and(|(_, c)| *c == '\'') {
                it.next();
                value.push('\'');
            } else {
                return Ok((value, &s.trim_start()[i + 1..]));
            }
        } else {
            value.push(c);
        }
    }
    Err("unterminated single-quoted string".into())
}
fn qonly(s: &str) -> Result<String> {
    let (v, rest) = quoted(s)?;
    if !rest.trim().is_empty() {
        return Err("unexpected text after string".into());
    }
    Ok(v)
}
fn numeric(s: &str, brackets: bool) -> Result<Vec<f64>> {
    let s = s.trim();
    let s = if brackets {
        s.strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .ok_or("expected bracketed numeric vector")?
    } else {
        s
    };
    s.split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .ok_or("expected finite numeric literal".into())
        })
        .collect()
}
struct Section {
    line: usize,
    fields: BTreeMap<String, (usize, String)>,
    rules: Vec<(usize, String)>,
}
impl Section {
    fn field(&self, key: &str) -> Result<(usize, &str)> {
        self.fields
            .get(key)
            .map(|(l, v)| (*l, v.as_str()))
            .ok_or_else(|| err(self.line, format!("missing {key}")))
    }
    fn string(&self, key: &str) -> Result<String> {
        let (l, v) = self.field(key)?;
        qonly(v).map_err(|m| err(l, m))
    }
    fn count(&self, key: &str, max: usize) -> Result<usize> {
        let (l, v) = self.field(key)?;
        let v = numeric(v, false).map_err(|m| err(l, m))?;
        if v.len() != 1 || v[0] < 0. || v[0].fract() != 0. || v[0] > max as f64 {
            return Err(err(l, format!("invalid {key}")));
        }
        Ok(v[0] as usize)
    }
    fn keys(&self, allowed: &[&str], mf_count: usize) -> Result<()> {
        for (k, (l, _)) in &self.fields {
            if !allowed.contains(&k.as_str())
                && !k
                    .strip_prefix("MF")
                    .and_then(|n| n.parse::<usize>().ok())
                    .is_some_and(|n| n > 0 && n <= mf_count)
            {
                return Err(err(*l, format!("unsupported field {k}")));
            }
        }
        Ok(())
    }
}
fn comment(s: &str) -> &str {
    let mut chars = s.char_indices().peekable();
    let mut in_quote = false;
    while let Some((i, c)) = chars.next() {
        if c == '\'' {
            if in_quote && chars.peek().is_some_and(|(_, c)| *c == '\'') {
                chars.next();
            } else {
                in_quote = !in_quote;
            }
        }
        if !in_quote && matches!(c, '%' | ';') {
            return &s[..i];
        }
    }
    s
}
fn parse(e: &mut Engine, s: &str) -> Result<Value> {
    let mut sections: BTreeMap<String, Section> = BTreeMap::new();
    let mut current = String::new();
    for (i, line) in s.lines().enumerate() {
        e.check_cancel()?;
        let l = i + 1;
        let line = comment(line).trim().trim_start_matches('\u{feff}');
        if line.is_empty() {
            continue;
        }
        if line.len() > 65536 {
            return Err(err(l, "line exceeds size limit"));
        }
        if line.starts_with('[') {
            let key = line
                .strip_prefix('[')
                .and_then(|s| s.strip_suffix(']'))
                .ok_or_else(|| err(l, "invalid section header"))?;
            if sections
                .insert(
                    key.into(),
                    Section {
                        line: l,
                        fields: BTreeMap::new(),
                        rules: vec![],
                    },
                )
                .is_some()
            {
                return Err(err(l, "duplicate section"));
            }
            current = key.into();
        } else {
            let section = sections
                .get_mut(&current)
                .ok_or_else(|| err(l, "expected section header"))?;
            if current == "Rules" {
                if section.rules.len() >= MAX_ITEMS {
                    return Err(err(l, "rule count exceeds limit"));
                }
                section.rules.push((l, line.into()));
            } else {
                let (k, v) = line
                    .split_once('=')
                    .ok_or_else(|| err(l, "expected name=value"))?;
                if section
                    .fields
                    .insert(k.trim().into(), (l, v.trim().into()))
                    .is_some()
                {
                    return Err(err(l, "duplicate field"));
                }
            }
        }
    }
    let sys = sections
        .get("System")
        .ok_or_else(|| err(1, "missing [System]"))?;
    sys.keys(
        &[
            "Name",
            "Type",
            "Version",
            "NumInputs",
            "NumOutputs",
            "NumRules",
            "AndMethod",
            "OrMethod",
            "ImpMethod",
            "AggMethod",
            "DefuzzMethod",
            "DisableStructuralChecks",
        ],
        0,
    )?;
    let class = match sys.string("Type")?.as_str() {
        "mamdani" => "mamfis",
        "sugeno" => "sugfis",
        _ => return Err(err(sys.field("Type")?.0, "unsupported FIS type")),
    };
    let (vl, v) = sys.field("Version")?;
    if numeric(v, false).map_err(|m| err(vl, m))? != [2.] {
        return Err(err(vl, "unsupported FIS version"));
    }
    let ni = sys.count("NumInputs", MAX_VARS)?;
    let no = sys.count("NumOutputs", MAX_VARS)?;
    let nr = sys.count("NumRules", MAX_ITEMS)?;
    for (key, sec) in &sections {
        if !["System", "Rules"].contains(&key.as_str())
            && !(1..=ni).any(|i| *key == format!("Input{i}"))
            && !(1..=no).any(|i| *key == format!("Output{i}"))
        {
            return Err(err(sec.line, format!("unsupported section {key}")));
        }
    }
    let base = fis::constructor(e, class, &[])?;
    let mut p = object(e, &base)?.record()?.clone();
    p.insert("Name".into(), text(&sys.string("Name")?, e)?);
    for (key, prop) in METHODS {
        let name = sys.string(key)?;
        mf::function_name(&text(&name, e)?).map_err(|m| err(sys.field(key).unwrap().0, m))?;
        p.insert(prop.into(), text(&name, e)?);
    }
    if sys.fields.contains_key("DisableStructuralChecks") {
        p.insert(
            "DisableStructuralChecks".into(),
            num(sys.count("DisableStructuralChecks", 1)? as f64),
        );
    }
    for (kind, key, n) in [("Input", "Inputs", ni), ("Output", "Outputs", no)] {
        let mut vars = Vec::with_capacity(n);
        for i in 1..=n {
            let sec = sections
                .get(&format!("{kind}{i}"))
                .ok_or_else(|| err(sys.line, format!("missing [{kind}{i}]")))?;
            let nm = sec.count("NumMFs", MAX_MFS)?;
            sec.keys(&["Name", "Range", "NumMFs"], nm)?;
            let (l, r) = sec.field("Range")?;
            let range = numbers(numeric(r, true).map_err(|m| err(l, m))?)?;
            fis::range(&range).map_err(|m| err(l, m))?;
            let mut mfs = Vec::with_capacity(nm);
            for j in 1..=nm {
                let (l, m) = sec.field(&format!("MF{j}"))?;
                let parse_mf = || -> Result<Record> {
                    let (name, rest) = quoted(m)?;
                    let rest = rest
                        .trim()
                        .strip_prefix(':')
                        .ok_or("expected ':' after MF name")?;
                    let (typ, rest) = quoted(rest)?;
                    let rest = rest
                        .trim()
                        .strip_prefix(',')
                        .ok_or("expected ',' before MF parameters")?;
                    let params = numeric(rest, true)?;
                    mf::validate(
                        &typ,
                        &params,
                        if kind == "Output" && class == "sugfis" {
                            Some(ni)
                        } else {
                            None
                        },
                    )?;
                    mf::record(e, text(&typ, e)?, numbers(params)?, &name)
                };
                mfs.push(Arc::new(parse_mf().map_err(|m| err(l, m))?));
            }
            let mut v = Record::from([
                ("Name".into(), text(&sec.string("Name")?, e)?),
                ("Range".into(), range),
            ]);
            fis::set_collection(e, &mut v, "MembershipFunctions", "fismf", mfs)?;
            fis::validate_variable(e, &v).map_err(|m| err(sec.line, m))?;
            vars.push(Arc::new(v));
        }
        fis::set_collection(e, &mut p, key, "fisvar", vars)?;
    }
    let sec = sections
        .get("Rules")
        .ok_or_else(|| err(sys.line, "missing [Rules]"))?;
    if sec.rules.len() != nr {
        return Err(err(sec.line, "NumRules does not match rule count"));
    }
    let ins = fis::collection(e, &p, "Inputs")?;
    let outs = fis::collection(e, &p, "Outputs")?;
    let mut rs = Vec::with_capacity(nr);
    for (l, s) in &sec.rules {
        let rule = || -> Result<Record> {
            let (body, c) = s
                .rsplit_once(':')
                .ok_or("expected ':' before rule connection")?;
            let (body, w) = body
                .rsplit_once('(')
                .ok_or("expected '(weight)' after consequent")?;
            let w = w
                .trim()
                .strip_suffix(')')
                .ok_or("expected ')' after rule weight")?;
            let (ant, con) = body
                .split_once(',')
                .ok_or("expected ',' between antecedent and consequent")?;
            let w = numeric(w, false)?;
            let c = numeric(c, false)?;
            if w.len() != 1 || c.len() != 1 {
                return Err("invalid rule weight or connection".into());
            }
            let r =
                rules::numeric_record(e, numeric(ant, false)?, numeric(con, false)?, w[0], c[0])?;
            rules::validate_indices(e, &r, &ins, &outs, class)?;
            Ok(r)
        };
        rs.push(Arc::new(rule().map_err(|m| err(*l, m))?));
    }
    fis::set_collection(e, &mut p, "Rules", "fisrule", rs)?;
    // Validate structure with known methods while retaining custom names even
    // when their function files are not installed on this reader's machine.
    let mut structural = p.clone();
    for (key, prop) in METHODS {
        let name = mf::function_name(&p[prop])?;
        if [
            "min", "max", "prod", "probor", "sum", "centroid", "bisector", "mom", "som", "lom",
            "wtaver", "wtsum",
        ]
        .contains(&name.as_str())
        {
            fis::method_validate(e, class, prop, &p[prop])
                .map_err(|m| err(sys.field(key).unwrap().0, m))?;
        } else {
            structural.insert(prop.into(), object(e, &base)?.record()?[prop].clone());
        }
    }
    fis::validate(e, class, &structural).map_err(|m| err(sys.line, m))?;
    let checks = p["DisableStructuralChecks"].clone();
    p.insert("DisableStructuralChecks".into(), num(0.));
    rules::refresh(e, &mut p).map_err(|m| err(sec.line, m))?;
    p.insert("DisableStructuralChecks".into(), checks);
    new(e, Object::scalar(class, p))
}
pub(super) fn read(e: &mut Engine, a: &[Value]) -> Result<Value> {
    if a.len() != 1 {
        return Err("readfis: expected filename".into());
    }
    let path = e.file_path(&vtext(e, &a[0])?, ".fis")?;
    let file = std::fs::File::open(&path)
        .map_err(|m| format!("readfis: cannot open '{}': {m}", path.display()))?;
    let limit = MAX_FILE.min(e.limits.memory_bytes / 4);
    let mut s = String::new();
    file.take(limit as u64 + 1)
        .read_to_string(&mut s)
        .map_err(|m| err(1, m))?;
    if s.len() > limit {
        return Err(err(1, "file size limit exceeded"));
    }
    e.charge(s.len())?;
    parse(e, &s)
}
