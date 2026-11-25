use super::*;
use crate::syntax::{Expr, PathStep};

pub(in crate::engine) fn names(class: &str) -> &'static [&'static str] {
    match class {
        "mamfis" | "sugfis" => &[
            "Name",
            "Type",
            "AndMethod",
            "OrMethod",
            "ImplicationMethod",
            "AggregationMethod",
            "DefuzzificationMethod",
            "DisableStructuralChecks",
            "Inputs",
            "Outputs",
            "Rules",
        ],
        "evalfisOptions" => eval_options::EVAL,
        "gensurfOptions" => eval_options::SURF,
        "fisvar" => &["Name", "Range", "MembershipFunctions"],
        "fismf" => &["Name", "Type", "Parameters"],
        "fisrule" => &[
            "Description",
            "Antecedent",
            "Consequent",
            "Weight",
            "Connection",
        ],
        _ => &[],
    }
}
pub(in crate::engine) fn property(o: &Object, name: &str) -> Result<Value> {
    o.record()?
        .get(name)
        .cloned()
        .map(Value::from)
        .ok_or_else(|| format!("Unrecognized property '{name}' for class '{}'", o.class))
}
fn proxy(o: &Object) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(
        o.rows,
        o.cols,
        (1..=o.len()).map(|i| i as f64).collect(),
    )?))
}
fn selected(e: &mut Engine, o: &Object, args: &[Expr], out: &mut Execution) -> Result<Matrix> {
    let p = proxy(o)?;
    let v = e.index_value(p, args, out)?;
    matrix(e, &v)
}
pub(in crate::engine) fn index(
    e: &mut Engine,
    o: &Object,
    args: &[Expr],
    out: &mut Execution,
) -> Result<Value> {
    let ids = selected(e, o, args, out)?;
    let items = ids
        .data()
        .iter()
        .map(|i| o.items[*i as usize - 1].clone())
        .collect();
    new(
        e,
        Object {
            class: o.class,
            rows: ids.rows(),
            cols: ids.cols(),
            items: Arc::new(items),
        },
    )
}
pub(in crate::engine) fn assign(
    e: &mut Engine,
    o: &Object,
    steps: &[PathStep],
    value: Array,
    out: &mut Execution,
) -> Result<Object> {
    e.check_cancel()?;
    let Some((step, rest)) = steps.split_first() else {
        return Ok(array_object(e, &value, o.class)?.as_ref().clone());
    };
    if let PathStep::Paren(args) = step {
        let ids = selected(e, o, args, out)?;
        let mut items = o.items.as_ref().clone();
        let deleting = matches!(&value,Array::Numeric(m) if m.is_empty());
        if rest.is_empty() && deleting {
            let mut remove = ids
                .data()
                .iter()
                .map(|v| *v as usize - 1)
                .collect::<Vec<_>>();
            remove.sort_unstable();
            remove.dedup();
            for i in remove.into_iter().rev() {
                items.remove(i);
            }
            let mut result = Object::vector(o.class, items);
            if o.cols == 1 {
                result.rows = result.len();
                result.cols = usize::from(result.len() > 0);
            }
            return Ok(result);
        }
        if !rest.is_empty() {
            if ids.len() != 1 {
                return Err("fuzzy: nested property assignment requires one object index".into());
            }
            let i = ids.data()[0] as usize - 1;
            let child = Object {
                class: o.class,
                rows: 1,
                cols: 1,
                items: Arc::new(vec![items[i].clone()]),
            };
            items[i] = assign(e, &child, rest, value, out)?.items[0].clone();
        } else {
            let rhs = array_object(e, &value, o.class)?;
            if rhs.len() != ids.len() && rhs.len() != 1 {
                return Err("fuzzy: assignment dimensions do not match".into());
            }
            for (j, &id) in ids.data().iter().enumerate() {
                items[id as usize - 1] = rhs.items[if rhs.len() == 1 { 0 } else { j }].clone();
            }
        }
        return Ok(Object {
            class: o.class,
            rows: o.rows,
            cols: o.cols,
            items: Arc::new(items),
        });
    }
    let name = match step {
        PathStep::Field(n) => n.clone(),
        PathStep::Dynamic(expr) => e.dynamic_field_name(expr, out)?,
        _ => return Err("fuzzy: expected property or parentheses indexing".into()),
    };
    let mut props = o.record()?.clone();
    let old = props
        .get(&name)
        .ok_or_else(|| format!("Unrecognized property '{name}' for class '{}'", o.class))?
        .clone();
    if matches!(o.class, "mamfis" | "sugfis") && name == "Type" {
        return Err("FIS Type is read-only".into());
    }
    let value = if rest.is_empty() {
        value
    } else {
        e.assign_step(Some(old), rest, value, false, out)?
    };
    let changed_rule_description = name == "Rules"
        && rest
            .iter()
            .any(|step| matches!(step, PathStep::Field(field) if field == "Description"));
    let value = if rest.is_empty() && matches!(&value, Array::Numeric(m) if m.is_empty()) {
        let class = match name.as_str() {
            "Inputs" | "Outputs" => Some("fisvar"),
            "MembershipFunctions" => Some("fismf"),
            "Rules" => Some("fisrule"),
            _ => None,
        };
        if let Some(class) = class {
            new_array(e, Object::vector(class, vec![]))?
        } else {
            value
        }
    } else {
        value
    };
    props.insert(name.clone(), value);
    match o.class {
        "mamfis" | "sugfis" => {
            if changed_rule_description {
                rules::resolve_descriptions(e, &mut props, o.class)?;
            }
            fis::validate(e, o.class, &props)?;
            rules::refresh(e, &mut props)?;
        }
        "evalfisOptions" | "gensurfOptions" => eval_options::validate(o.class, &props)?,
        "fisvar" => {
            let r = fis::range(&props["Range"])?;
            props.insert("Range".into(), numbers(r.to_vec())?);
            fis::validate_variable(e, &props)?;
        }
        "fismf" => {
            string(&props["Name"])?;
            let typ = mf::function_name(&props["Type"])?;
            if name == "Type" {
                // Public fismf API converts parameters when Type changes. Use
                // support endpoints/center to retain the represented interval.
                let oldp = vecnum(&props["Parameters"])?;
                let oldt = mf::function_name(&o.record()?["Type"])?;
                props.insert(
                    "Parameters".into(),
                    numbers(convert_parameters(&oldt, &oldp, &typ))?,
                );
            }
            let params = vecnum(&props["Parameters"])?;
            mf::validate(&typ, &params, None)?;
            props.insert("Parameters".into(), numbers(params)?);
        }
        "fisrule" => rules::validate_property(e, &mut props, &name)?,
        _ => unreachable!(),
    }
    Ok(Object::scalar(o.class, props))
}
fn convert_parameters(old: &str, p: &[f64], t: &str) -> Vec<f64> {
    if old == t {
        return p.to_vec();
    }
    let (a, b) = match old {
        "trimf" => (p[0], p[2]),
        "trapmf" | "pimf" => (p[0], p[3]),
        "gaussmf" => (p[1] - 2. * p[0], p[1] + 2. * p[0]),
        "gauss2mf" => (p[1] - 2. * p[0], p[3] + 2. * p[2]),
        "gbellmf" => (p[2] - p[0].abs(), p[2] + p[0].abs()),
        "smf" | "zmf" | "linsmf" | "linzmf" => (p[0], p[1]),
        _ => (0., 1.),
    };
    let c = (a + b) / 2.;
    let d = (b - a).abs().max(f64::EPSILON);
    match t {
        "trimf" => vec![a, c, b],
        "trapmf" | "pimf" => vec![a, a + d / 3., b - d / 3., b],
        "gaussmf" => vec![d / 4., c],
        "gauss2mf" => vec![d / 4., a, d / 4., b],
        "gbellmf" => vec![d / 2., 2., c],
        "sigmf" => vec![8. / d, c],
        "dsigmf" => vec![8. / d, a, 8. / d, b],
        "psigmf" => vec![8. / d, a, -8. / d, b],
        "smf" | "zmf" | "linsmf" | "linzmf" => vec![a, b],
        "constant" => vec![c],
        "linear" => vec![0., c],
        _ => p.to_vec(),
    }
}
pub(in crate::engine) fn method(
    e: &mut Engine,
    o: &Object,
    n: &str,
    a: &[Value],
    out: &mut Execution,
) -> Result<Option<Value>> {
    let numeric = |v: f64| Some(Value::Matrix(Matrix::scalar(v)));
    let boolean = |v: bool| Some(Value::Logical(crate::LogicalMatrix::scalar(v)));
    match n {
        "numel" => {
            if !a.is_empty() {
                return Err("numel: too many inputs".into());
            }
            Ok(numeric(o.len() as f64))
        }
        "length" => Ok(numeric(o.rows.max(o.cols) as f64)),
        "isempty" => Ok(boolean(o.len() == 0)),
        "isscalar" => Ok(boolean(o.len() == 1)),
        "isvector" => Ok(boolean(o.rows == 1 || o.cols == 1)),
        "isrow" => Ok(boolean(o.rows == 1)),
        "iscolumn" => Ok(boolean(o.cols == 1)),
        "ismatrix" => Ok(boolean(true)),
        "size" => {
            let v = match a {
                [] => Matrix::from_column_vec(1, 2, vec![o.rows as f64, o.cols as f64])?,
                [v] => {
                    let d = count(&v.clone().array(&e.cancel)?, MAX_ITEMS, true)?;
                    Matrix::scalar(match d {
                        1 => o.rows as f64,
                        2 => o.cols as f64,
                        _ => 1.,
                    })
                }
                _ => return Err("size: too many inputs".into()),
            };
            Ok(Some(Value::Matrix(v)))
        }
        "isprop" => {
            let [v] = a else {
                return Err("isprop: expected a property name".into());
            };
            Ok(boolean(names(o.class).contains(&vtext(e, v)?.as_str())))
        }
        "properties" | "fieldnames" => {
            let v = names(o.class)
                .iter()
                .map(|s| text(s, e))
                .collect::<Result<Vec<_>>>()?;
            Ok(Some(Value::from(Array::Cell(
                crate::CellArray::from_shared(
                    v.len(),
                    1,
                    v.into_iter().map(Arc::new).collect(),
                    &e.cancel,
                )?,
            ))))
        }
        "addInput" | "addOutput" | "addMF" | "removeInput" | "removeOutput" | "removeMF"
        | "addRule" | "showrule" | "evalmf" => {
            let mut args = vec![new(e, o.clone())?];
            args.extend_from_slice(a);
            Ok(Some(call(e, n, &args, 1, out)?.remove(0)))
        }
        _ => Ok(None),
    }
}
pub(in crate::engine) fn concatenate(e: &mut Engine, values: Vec<Vec<Value>>) -> Result<Value> {
    let class = values
        .iter()
        .flatten()
        .find_map(|v| super::super::objects::fis_value(e, v))
        .ok_or("Expected fuzzy objects")?
        .class;
    let mut blocks = Vec::new();
    let mut cols = None;
    for row in values {
        let mut rowobjects = Vec::new();
        let mut rows = None;
        let mut width = 0;
        for v in row {
            if matches!(&v,Value::Matrix(m) if m.is_empty()) {
                continue;
            }
            let o = object(e, &v)?;
            if o.class != class {
                return Err("Cannot concatenate different fuzzy object classes".into());
            }
            if o.len() == 0 {
                continue;
            }
            if rows.is_some_and(|r| r != o.rows) {
                return Err("Fuzzy object concatenation dimensions do not match".into());
            }
            rows = Some(o.rows);
            width += o.cols;
            rowobjects.push(o);
        }
        if width == 0 {
            continue;
        }
        if cols.is_some_and(|c| c != width) {
            return Err("Fuzzy object concatenation dimensions do not match".into());
        }
        cols = Some(width);
        blocks.push((rows.unwrap(), rowobjects));
    }
    let cols = cols.unwrap_or(0);
    let rows: usize = blocks.iter().map(|(r, _)| r).sum();
    if rows.saturating_mul(cols) > MAX_ITEMS {
        return Err("fuzzy: array size limit exceeded".into());
    }
    let mut items = Vec::with_capacity(rows * cols);
    for c in 0..cols {
        for (height, blocks) in &blocks {
            let mut offset = c;
            for b in blocks {
                if offset < b.cols {
                    items.extend((0..*height).map(|r| b.items[r + offset * b.rows].clone()));
                    break;
                }
                offset -= b.cols;
            }
        }
    }
    new(
        e,
        Object {
            class,
            rows,
            cols,
            items: Arc::new(items),
        },
    )
}
