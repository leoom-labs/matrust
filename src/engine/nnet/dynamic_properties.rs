//! Object metadata for the tap expansion, kept separate from numerical kernels.
use super::*;
pub(super) fn property(o: &Object, name: &str, c: &AtomicBool) -> Result<Option<Array>> {
    let Some(d) = &o.dynamic else {
        return Ok(None);
    };
    let n = o.core.layers.len();
    let inputs = d.inputs();
    Ok(Some(match name {
        "inputs" => {
            let mut v = vec![];
            for i in 0..inputs {
                let mut p = o.core.input.clone();
                p.size = if i == 0 && !d.input_delays.is_empty() {
                    d.input_size
                } else {
                    d.feedback_size
                };
                p.process_settings.clear();
                p.range.clear();
                v.push(properties::port(&p, c)?);
            }
            cell(1, inputs, v, c)?
        }
        "inputConnect" => Array::Logical(LogicalMatrix::from_canonical_vec(
            n,
            inputs,
            (0..n * inputs).map(|i| u8::from(i % n == 0)).collect(),
        )?),
        "layerConnect" => {
            let mut v = o
                .core
                .layer_connect
                .iter()
                .map(|&x| u8::from(x))
                .collect::<Vec<_>>();
            if d.closed && !d.feedback_delays.is_empty() {
                v[(n - 1) * n] = 1;
            }
            Array::Logical(LogicalMatrix::from_canonical_vec(n, n, v)?)
        }
        _ => return Ok(None),
    }))
}
pub(super) fn assign_inputs(
    e: &mut Engine,
    o: &Object,
    old: &Array,
    value: &Array,
) -> Result<Object> {
    let d = o.dynamic.as_ref().unwrap();
    let Array::Cell(v) = value else {
        return Err("network inputs must be cells".into());
    };
    let Array::Cell(prev) = old else {
        unreachable!()
    };
    if v.rows() != 1 || v.cols() != d.inputs() {
        return Err("network: input port count cannot change".into());
    }
    let mut o = o.clone();
    for i in 0..v.len() {
        let a = v.get_linear(i)?.as_ref();
        let before = prev.get_linear(i)?.as_ref();
        let Array::Struct(s) = a else {
            return Err("network: input port must be struct".into());
        };
        let Array::Struct(b) = before else {
            unreachable!()
        };
        if s.field_names() != b.field_names() {
            return Err("network: input port fields cannot change".into());
        }
        for k in s.field_names() {
            if !matches!(k.as_str(), "processFcns" | "processParams")
                && !properties::arrays_equal(e, field(a, k)?, field(before, k)?)?
            {
                return Err(format!("network: input {k} is read-only; use configure"));
            }
        }
        if properties::arrays_equal(e, a, before)? {
            continue;
        }
        let names = text_list(field(a, "processFcns")?)?;
        if names.len() > 16 || names.iter().any(|n| !process::NAMES.contains(&n.as_str())) {
            return Err("network: unsupported processFcns".into());
        }
        let p = &mut o.core.input;
        if names != p.process_fcns {
            p.process_params = names.iter().map(|n| process::defaults(n)).collect();
        } else {
            let Array::Cell(params) = field(a, "processParams")? else {
                return Err("processParams must be cells".into());
            };
            if params.len() != names.len() {
                return Err("processParams must match processFcns".into());
            }
            p.process_params = names
                .iter()
                .enumerate()
                .map(|(i, s)| settings::read_params(s, params.get_linear(i)?.as_ref()))
                .collect::<Result<_>>()?;
        }
        p.process_fcns = names;
        p.process_settings.clear();
        o.core.configured = false;
    }
    if !properties::arrays_equal(e, old, value)? {
        o.core.resize_weights()?;
    }
    Ok(o)
}
