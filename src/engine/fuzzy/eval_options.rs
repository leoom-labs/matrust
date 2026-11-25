//! Bounded value-semantic option sets for evaluation and surface generation.
use super::*;
pub(super) const EVAL: &[&str] = &[
    "NumSamplePoints",
    "OutOfRangeInputValueMessage",
    "NoRuleFiredMessage",
    "EmptyOutputFuzzySetMessage",
];
pub(super) const SURF: &[&str] = &[
    "InputIndex",
    "OutputIndex",
    "NumGridPoints",
    "ReferenceInputs",
    "NumSamplePoints",
];
pub(super) fn validate(class: &str, p: &Record) -> Result<()> {
    let n = count(&p["NumSamplePoints"], MAX_ITEMS, true)?;
    if n < 2 {
        return Err("NumSamplePoints must be an integer greater than 1".into());
    }
    if class == "evalfisOptions" {
        for k in &EVAL[1..] {
            if !["error", "warning", "none"].contains(&string(&p[*k])?.as_str()) {
                return Err(format!("{k} must be 'error', 'warning' or 'none'"));
            }
        }
    } else {
        for (key, max_len, min) in [
            ("InputIndex", 2, 1.),
            ("NumGridPoints", 2, 2.),
            ("OutputIndex", 1, 1.),
        ] {
            let v = vecnum(&p[key])?;
            if v.is_empty()
                || v.len() > max_len
                || v.iter()
                    .any(|x| !x.is_finite() || x.fract() != 0. || *x < min || *x > MAX_ITEMS as f64)
            {
                return Err(format!(
                    "{key}: expected one or two bounded positive integers"
                ));
            }
            if key == "InputIndex" && v.len() == 2 && v[0] == v[1] {
                return Err("InputIndex must contain distinct indices".into());
            }
        }
        if string(&p["ReferenceInputs"]).is_ok_and(|s| s == "auto") {
            return Ok(());
        }
        let v = vecnum(&p["ReferenceInputs"])?;
        if v.len() > MAX_VARS || v.iter().any(|x| x.is_infinite()) {
            return Err("ReferenceInputs must be 'auto' or a finite/NaN vector".into());
        }
    }
    Ok(())
}
pub(super) fn defaults(e: &Engine, class: &str) -> Result<Record> {
    let mut p = Record::new();
    p.insert("NumSamplePoints".into(), num(101.));
    if class == "evalfisOptions" {
        for k in &EVAL[1..] {
            p.insert((*k).into(), text("warning", e)?);
        }
    } else {
        p.insert("InputIndex".into(), numbers(vec![1., 2.])?);
        p.insert("OutputIndex".into(), num(1.));
        p.insert("NumGridPoints".into(), num(15.));
        p.insert("ReferenceInputs".into(), text("auto", e)?);
    }
    Ok(p)
}
pub(super) fn constructor(e: &mut Engine, class: &'static str, a: &[Value]) -> Result<Value> {
    let mut p = defaults(e, class)?;
    p.extend(options(
        e,
        a,
        if class == "evalfisOptions" {
            EVAL
        } else {
            SURF
        },
    )?);
    validate(class, &p)?;
    new(e, Object::scalar(class, p))
}
pub(super) fn get(e: &Engine, v: Option<&Value>, class: &str) -> Result<Record> {
    if let Some(v) = v {
        let o = object(e, v)?;
        if o.class != class {
            return Err(format!("Expected {class} object"));
        }
        let p = o.record()?;
        validate(class, p)?;
        Ok(p.clone())
    } else {
        defaults(e, class)
    }
}
