//! Type-1 fuzzy core built from public MathWorks documentation and published
//! membership/spline and polygon integration formulae. No MATLAB sources.
//! Reference generator: tool/refs/fuzzy/core.py (NumPy/scikit-fuzzy).
//! All objects have value semantics. Records and numeric payloads are shared;
//! edits allocate a new record along the affected property path only.
pub(super) mod defuzz;
mod display;
mod eval;
mod eval_options;
mod file;
pub(super) mod fis;
pub(super) mod mf;
mod plots;
pub(super) mod properties;
pub(super) mod rules;
use super::{Engine, Execution, Value};
use crate::{Array, CharacterRow, Matrix, Result};
use std::{collections::BTreeMap, sync::Arc};

pub(super) const NAMES: &[&str] = &[
    "trimf",
    "trapmf",
    "gaussmf",
    "gauss2mf",
    "gbellmf",
    "sigmf",
    "dsigmf",
    "psigmf",
    "zmf",
    "smf",
    "pimf",
    "linsmf",
    "linzmf",
    "evalmf",
    "defuzz",
    "mamfis",
    "sugfis",
    "fisvar",
    "fismf",
    "fisrule",
    "addInput",
    "addOutput",
    "addMF",
    "removeInput",
    "removeOutput",
    "removeMF",
    "addRule",
    "showrule",
    "evalfis",
    "evalfisOptions",
    "readfis",
    "writefis",
    "gensurf",
    "gensurfOptions",
    "plotmf",
    "plotfis",
];
pub(super) fn handles(n: &str) -> bool {
    NAMES.contains(&n)
}
pub(super) fn bare(n: &str) -> bool {
    matches!(
        n,
        "mamfis" | "sugfis" | "fisvar" | "fismf" | "fisrule" | "evalfisOptions" | "gensurfOptions"
    )
}
pub(super) const MAX_ITEMS: usize = 65536;
pub(super) const MAX_VARS: usize = 64;
pub(super) const MAX_MFS: usize = 256;
pub(super) type Record = BTreeMap<String, Array>;
#[derive(Clone)]
pub(super) struct Object {
    pub class: &'static str,
    pub rows: usize,
    pub cols: usize,
    pub items: Arc<Vec<Arc<Record>>>,
}
impl Object {
    pub fn scalar(class: &'static str, props: Record) -> Self {
        Self {
            class,
            rows: 1,
            cols: 1,
            items: Arc::new(vec![Arc::new(props)]),
        }
    }
    pub fn vector(class: &'static str, items: Vec<Arc<Record>>) -> Self {
        let n = items.len();
        Self {
            class,
            rows: usize::from(n > 0),
            cols: n,
            items: Arc::new(items),
        }
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn record(&self) -> Result<&Record> {
        if self.len() != 1 {
            return Err(format!(
                "{}: property access requires a scalar object",
                self.class
            ));
        }
        Ok(&self.items[0])
    }
    pub fn bytes(&self) -> usize {
        128 + self
            .items
            .iter()
            .map(|p| 64 + p.iter().map(|(k, v)| k.len() + v.bytes()).sum::<usize>())
            .sum::<usize>()
    }
    pub fn children(&self) -> impl Iterator<Item = &Array> {
        self.items.iter().flat_map(|p| p.values())
    }
    pub fn body(&self, e: &Engine) -> String {
        display::body(e, self)
    }
}
pub(super) fn object(e: &Engine, v: &Value) -> Result<Arc<Object>> {
    super::objects::fis_value(e, v).ok_or_else(|| "Expected a fuzzy object".into())
}
pub(super) fn array_object(e: &Engine, a: &Array, class: &str) -> Result<Arc<Object>> {
    let o = object(e, &Value::from(a.clone()))?;
    if o.class != class {
        return Err(format!("Expected {class} objects"));
    }
    Ok(o)
}
pub(super) fn new(e: &mut Engine, o: Object) -> Result<Value> {
    if o.len() > MAX_ITEMS || o.bytes() > e.limits.memory_bytes {
        return Err("fuzzy: object size limit exceeded".into());
    }
    e.charge(o.len().max(1))?;
    super::objects::fis_new(e, o)
}
pub(super) fn new_array(e: &mut Engine, o: Object) -> Result<Array> {
    new(e, o)?.array(&e.cancel)
}
pub(super) fn num(x: f64) -> Array {
    Array::Numeric(Matrix::scalar(x))
}
pub(super) fn numbers(x: Vec<f64>) -> Result<Array> {
    Ok(Array::Numeric(Matrix::from_column_vec(1, x.len(), x)?))
}
pub(super) fn text(s: &str, e: &Engine) -> Result<Array> {
    Ok(Array::Character(CharacterRow::from_text(s, &e.cancel)?))
}
pub(super) fn string(a: &Array) -> Result<String> {
    super::table_fns::text_scalar(&Value::from(a.clone()))
        .ok_or_else(|| "Expected scalar text".into())
}
pub(super) fn vtext(e: &Engine, v: &Value) -> Result<String> {
    string(&v.clone().array(&e.cancel)?)
}
pub(super) fn matrix(e: &Engine, v: &Value) -> Result<Matrix> {
    match v.clone().array(&e.cancel)? {
        Array::Numeric(m) => Ok(m),
        _ => Err("fuzzy: expected a real numeric array".into()),
    }
}
pub(super) fn vecnum(a: &Array) -> Result<Vec<f64>> {
    match a {
        Array::Numeric(m) if m.rows() <= 1 || m.cols() <= 1 => Ok(m.data().to_vec()),
        _ => Err("Expected a real numeric vector".into()),
    }
}
pub(super) fn scalar(a: &Array) -> Result<f64> {
    match a {
        Array::Numeric(m) => m.as_scalar(),
        Array::Logical(m) if m.len() == 1 => Ok(m.data()[0] as f64),
        _ => Err("Expected a real scalar".into()),
    }
}
pub(super) fn count(a: &Array, max: usize, positive: bool) -> Result<usize> {
    let n = scalar(a)?;
    if !n.is_finite() || n.fract() != 0. || n < if positive { 1. } else { 0. } || n > max as f64 {
        return Err(format!(
            "fuzzy: expected integer in {}..{max}",
            usize::from(positive)
        ));
    }
    Ok(n as usize)
}
pub(super) fn options(e: &Engine, args: &[Value], allowed: &[&str]) -> Result<Record> {
    if !args.len().is_multiple_of(2) {
        return Err("Expected Name,Value pairs".into());
    }
    let mut p = Record::new();
    for pair in args.as_chunks::<2>().0 {
        let key = vtext(e, &pair[0])?;
        let canonical = allowed
            .iter()
            .find(|k| k.eq_ignore_ascii_case(&key))
            .ok_or_else(|| format!("Unrecognized parameter name '{key}'"))?;
        if p.insert((*canonical).into(), pair[1].clone().array(&e.cancel)?)
            .is_some()
        {
            return Err(format!("Duplicate parameter '{key}'"));
        }
    }
    Ok(p)
}
pub(super) fn call(
    e: &mut Engine,
    n: &str,
    a: &[Value],
    nout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    e.check_cancel()?;
    let multi = match n {
        "evalfis" => Some(eval::call(e, a, nout, out)?),
        "plotmf" => Some(plots::plotmf(e, a, nout, out)?),
        "gensurf" => Some(plots::gensurf(e, a, nout, out)?),
        "plotfis" => Some(plots::plotfis(e, a, nout, out)?),
        _ => None,
    };
    if let Some(v) = multi {
        e.charge_function_outputs(&v)?;
        return Ok(v);
    }
    if nout > 1 {
        return Err(format!("{n}: too many output arguments"));
    }
    let v = if mf::NAMES.contains(&n) {
        if a.len() != 2 {
            return Err(format!("{n}: expected x and parameters"));
        }
        mf::evaluate(e, n, &a[0], &a[1].clone().array(&e.cancel)?)?
    } else {
        match n {
            "evalfisOptions" => eval_options::constructor(e, "evalfisOptions", a)?,
            "gensurfOptions" => eval_options::constructor(e, "gensurfOptions", a)?,
            "readfis" => file::read(e, a)?,
            "writefis" => file::write(e, a)?,
            "defuzz" => defuzz::call(e, a, out)?,
            "evalmf" => mf::evalmf(e, a, out)?,
            "mamfis" | "sugfis" => fis::constructor(e, n, a)?,
            "fismf" => mf::constructor(e, a)?,
            "fisvar" => fis::variable_constructor(e, a)?,
            "fisrule" => rules::constructor(e, a)?,
            "addInput" | "addOutput" | "addMF" | "removeInput" | "removeOutput" | "removeMF" => {
                fis::edit(e, n, a)?
            }
            "addRule" => rules::add(e, a)?,
            "showrule" => rules::show(e, a)?,
            _ => return Err(format!("{n}: not supported yet")),
        }
    };
    e.charge_function_outputs(std::slice::from_ref(&v))?;
    Ok(vec![v])
}
/// Compare records directly through the engine's ordinary recursive value
/// comparator. No temporary container scales with the rule count: ordinary
/// structs/cells have a 65536-total-slot limit even when nested.
pub(super) fn equal(
    a: &Object,
    b: &Object,
    mut compare: impl FnMut(&Array, &Array) -> Result<bool>,
) -> Result<bool> {
    if a.class != b.class || a.rows != b.rows || a.cols != b.cols || a.len() != b.len() {
        return Ok(false);
    }
    for (a, b) in a.items.iter().zip(b.items.iter()) {
        if a.len() != b.len() {
            return Ok(false);
        }
        for ((ka, va), (kb, vb)) in a.iter().zip(b.iter()) {
            if ka != kb {
                return Ok(false);
            }
            // FIS text properties represent names, regardless of whether the
            // caller supplied a character vector or a scalar string.
            if let (Ok(a), Ok(b)) = (string(va), string(vb)) {
                if a != b {
                    return Ok(false);
                }
            } else if !compare(va, vb)? {
                return Ok(false);
            }
        }
    }
    Ok(true)
}
