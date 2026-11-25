use super::{Engine, Execution, Value};
use crate::syntax::{ArgDim, ArgumentSpec, ArgumentsBlock, ArgumentsKind, Expr};
use crate::{Array, CellArray, Matrix, Result, StructArray};
use std::{cmp::Ordering, collections::HashMap, sync::Arc};

pub(super) fn validate_inputs(
    engine: &mut Engine,
    blocks: &[ArgumentsBlock],
    inputs: &[Option<String>],
    all_args: &[Value],
    nv_start: Option<usize>,
    byte: &mut usize,
    out: &mut Execution,
) -> Result<()> {
    let mut has_input_block = false;
    let mut positional_specs: Vec<&ArgumentSpec> = Vec::new();
    let mut nv_specs: Vec<&ArgumentSpec> = Vec::new();
    let mut repeating_specs: Vec<&ArgumentSpec> = Vec::new();
    for block in blocks {
        match block.kind {
            ArgumentsKind::Input => {
                has_input_block = true;
                for spec in &block.specs {
                    if spec.group.is_some() {
                        nv_specs.push(spec);
                    } else {
                        positional_specs.push(spec);
                    }
                }
            }
            ArgumentsKind::Repeating => {
                for spec in &block.specs {
                    repeating_specs.push(spec);
                }
            }
            ArgumentsKind::Output => {}
        }
    }
    if !has_input_block && repeating_specs.is_empty() {
        return Ok(());
    }

    let pos_limit = nv_start.unwrap_or(all_args.len());
    let pos_passed = pos_limit.min(positional_specs.len());
    let rem_after_pos = &all_args[pos_passed..];
    let rep_end = if repeating_specs.is_empty() {
        0
    } else {
        let k = repeating_specs.len();
        let rep_limit = pos_limit.saturating_sub(pos_passed);
        if nv_specs.is_empty() {
            rep_limit
        } else {
            // Stop before trailing name-value pairs if any match a known name-value field.
            let mut idx = 0;
            while idx + k <= rep_limit {
                if extract_scalar_text(&rem_after_pos[idx])
                    .is_some_and(|s| match_nv_name(&s, &nv_specs).is_ok())
                    && (rem_after_pos.len() - idx).is_multiple_of(2)
                {
                    break;
                }
                idx += k;
            }
            idx
        }
    };
    let nargin = if nv_specs.is_empty() && nv_start.is_none() {
        all_args.len()
    } else {
        pos_passed + rep_end
    };
    if let Some((n, _)) = engine.functions.arguments.as_mut() {
        *n = nargin;
    }

    for (pos, spec) in positional_specs.iter().enumerate() {
        *byte = spec.byte;
        if inputs.get(pos).is_some_and(Option::is_none) {
            continue;
        }
        let val = if pos < pos_passed {
            all_args[pos].clone()
        } else if let Some(default_expr) = &spec.default {
            engine.eval(default_expr, None, out)?
        } else {
            return Err(engine.raise("Not enough input arguments.".into(), "MATLAB:minrhs".into()));
        };
        let prefix = format!("Invalid argument at position {}.", pos + 1);
        validate_one_spec(engine, spec, val, &prefix, Some(&spec.name), byte, out)?;
    }

    let mut rem = rem_after_pos;

    if !repeating_specs.is_empty() {
        let k = repeating_specs.len();
        let rep_slice = &rem[..rep_end];
        rem = &rem[rep_end..];
        if !rep_slice.len().is_multiple_of(k) {
            return Err(engine.raise(
                format!("Repeating arguments must be passed in groups of {k}."),
                "MATLAB:minrhs".into(),
            ));
        }
        let n_groups = rep_slice.len() / k;
        let mut columns: Vec<Vec<Arc<Array>>> = vec![Vec::with_capacity(n_groups); k];
        for (g, chunk) in rep_slice.chunks_exact(k).enumerate() {
            for (j, spec) in repeating_specs.iter().enumerate() {
                *byte = spec.byte;
                let pos = pos_passed + g * k + j + 1;
                let prefix = format!("Invalid argument at position {pos}.");
                let validated = validate_one_spec(
                    engine,
                    spec,
                    chunk[j].clone(),
                    &prefix,
                    Some(&spec.name),
                    byte,
                    out,
                )?;
                let arr = validated.array(&engine.cancel)?;
                columns[j].push(Arc::new(arr));
            }
        }
        for (j, spec) in repeating_specs.iter().enumerate() {
            let cell = CellArray::from_shared(
                1,
                n_groups,
                std::mem::take(&mut columns[j]),
                &engine.cancel,
            )?;
            engine.insert_array(&spec.name, Array::Cell(cell))?;
        }
    }

    if nv_specs.is_empty() && nv_start.is_some() && !rem.is_empty() {
        return Err("Too many input arguments.".into());
    }

    if !nv_specs.is_empty() {
        let mut supplied: HashMap<usize, Value> = HashMap::new();
        if rem.len() == 1
            && let Value::Container(Array::Struct(s)) = &rem[0]
            && s.rows() == 1
            && s.cols() == 1
        {
            for (f_idx, fname) in s.field_names().iter().enumerate() {
                let spec_idx = resolve_nv_index(engine, fname, &nv_specs)?;
                let arr = s.get(f_idx, 0)?.as_ref().clone();
                supplied.insert(spec_idx, Value::from(arr));
            }
        } else {
            if !rem.len().is_multiple_of(2) {
                return Err(engine.raise(
                    "Name-value arguments must be passed in pairs.".into(),
                    "MATLAB:validation:InvalidNameValue".into(),
                ));
            }
            for pair in rem.as_chunks::<2>().0 {
                let Some(name_str) = extract_scalar_text(&pair[0]) else {
                    return Err(engine.raise(
                        "Name-value argument name must be a character vector or string scalar."
                            .into(),
                        "MATLAB:validation:InvalidNameValue".into(),
                    ));
                };
                let spec_idx = resolve_nv_index(engine, &name_str, &nv_specs)?;
                supplied.insert(spec_idx, pair[1].clone());
            }
        }

        // Preserve declaration order of each opts struct and its fields.
        let mut group_order: Vec<String> = Vec::new();
        for spec in &nv_specs {
            let g = spec.group.as_ref().expect("nv spec has group");
            if !group_order.contains(g) {
                group_order.push(g.clone());
            }
        }
        for group_name in group_order {
            let mut struct_arr =
                StructArray::from_slots(1, 1, Vec::new(), Vec::new(), &engine.cancel)?;
            engine.insert_array(&group_name, Array::Struct(struct_arr.clone()))?;
            for (idx, spec) in nv_specs.iter().enumerate() {
                if spec.group.as_deref() != Some(group_name.as_str()) {
                    continue;
                }
                *byte = spec.byte;
                let val = if let Some(v) = supplied.get(&idx) {
                    v.clone()
                } else if let Some(default_expr) = &spec.default {
                    engine.eval(default_expr, None, out)?
                } else {
                    continue;
                };
                let prefix = format!("Invalid value for name-value argument '{}'.", spec.name);
                let arr = convert_and_size(engine, spec, val, &prefix, out)?;
                struct_arr = struct_arr.with_field(&spec.name, arr.clone(), &engine.cancel)?;
                engine.insert_array(&group_name, Array::Struct(struct_arr.clone()))?;
                run_validators(engine, spec, Value::from(arr), &prefix, false, byte, out)?;
            }
        }
    }
    Ok(())
}

pub(super) fn validate_property(
    engine: &mut Engine,
    spec: &ArgumentSpec,
    val: Value,
    class_name: &str,
    out: &mut Execution,
) -> Result<Value> {
    let mut byte = spec.byte;
    let prefix = format!(
        "Error setting property '{}' of class '{class_name}'.",
        spec.name
    );
    validate_one_spec(engine, spec, val, &prefix, None, &mut byte, out)
}

fn convert_and_size(
    engine: &mut Engine,
    spec: &ArgumentSpec,
    mut val: Value,
    prefix: &str,
    out: &mut Execution,
) -> Result<Array> {
    if let Some(cls) = &spec.class_name {
        val = convert_to_class(engine, val, cls, prefix, out)?;
    }
    if let Some(dims) = &spec.dims {
        val = check_and_expand_size(engine, val, dims, prefix, out)?;
    }
    val.array(&engine.cancel)
}

fn validate_one_spec(
    engine: &mut Engine,
    spec: &ArgumentSpec,
    val: Value,
    prefix: &str,
    insert_name: Option<&str>,
    byte: &mut usize,
    out: &mut Execution,
) -> Result<Value> {
    let arr = convert_and_size(engine, spec, val, prefix, out)?;
    let updated_val = if let Some(var_name) = insert_name {
        engine.insert_array(var_name, arr)?;
        Value::from(engine.array(var_name).unwrap())
    } else {
        Value::from(arr)
    };
    run_validators(
        engine,
        spec,
        updated_val.clone(),
        prefix,
        insert_name.is_none(),
        byte,
        out,
    )?;
    Ok(updated_val)
}

fn run_validators(
    engine: &mut Engine,
    spec: &ArgumentSpec,
    updated_val: Value,
    prefix: &str,
    substitute_bare_name: bool,
    byte: &mut usize,
    out: &mut Execution,
) -> Result<()> {
    for validator in &spec.validators {
        *byte = validator.byte;
        let res = if validator.args.is_empty() {
            invoke_validator(engine, &validator.name, vec![updated_val.clone()], out)
        } else {
            let mut eval_args = Vec::with_capacity(validator.args.len());
            for arg_expr in &validator.args {
                if substitute_bare_name
                    && let Expr::Name(n) = arg_expr
                    && n == &spec.name
                {
                    eval_args.push(updated_val.clone());
                } else {
                    eval_args.push(engine.eval(arg_expr, None, out)?);
                }
            }
            invoke_validator(engine, &validator.name, eval_args, out)
        };
        if let Err(msg) = res {
            return Err(prefix_validation_error(engine, prefix, msg));
        }
    }
    Ok(())
}

fn extract_scalar_text(val: &Value) -> Option<String> {
    match val {
        Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
        Value::Container(Array::Character(c)) if c.rows() <= 1 => {
            Some(String::from_utf16_lossy(c.units()))
        }
        Value::Container(Array::Str(s)) if s.rows() == 1 && s.cols() == 1 => s
            .data()
            .first()
            .and_then(|o| o.as_deref().map(str::to_owned)),
        _ => None,
    }
}

enum NvMatch {
    Ambiguous,
    Unknown,
}

fn match_nv_name(name: &str, nv_specs: &[&ArgumentSpec]) -> std::result::Result<usize, NvMatch> {
    for (i, spec) in nv_specs.iter().enumerate() {
        if spec.name.eq_ignore_ascii_case(name) {
            return Ok(i);
        }
    }
    let lower = name.to_ascii_lowercase();
    let mut matched = None;
    for (i, spec) in nv_specs.iter().enumerate() {
        if spec.name.to_ascii_lowercase().starts_with(&lower) {
            if matched.is_some() {
                return Err(NvMatch::Ambiguous);
            }
            matched = Some(i);
        }
    }
    matched.ok_or(NvMatch::Unknown)
}

fn resolve_nv_index(engine: &mut Engine, name: &str, nv_specs: &[&ArgumentSpec]) -> Result<usize> {
    match match_nv_name(name, nv_specs) {
        Ok(i) => Ok(i),
        Err(NvMatch::Ambiguous) => Err(engine.raise(
            format!("Ambiguous argument name '{name}'."),
            "MATLAB:argument:AmbiguousName".into(),
        )),
        Err(NvMatch::Unknown) => Err(engine.raise(
            format!("Invalid argument name '{name}'."),
            "MATLAB:argument:UnknownName".into(),
        )),
    }
}

pub(super) fn validate_outputs(
    engine: &mut Engine,
    blocks: &[ArgumentsBlock],
    requested: usize,
    byte: &mut usize,
    out: &mut Execution,
) -> Result<()> {
    for block in blocks {
        if block.kind != ArgumentsKind::Output {
            continue;
        }
        for (pos, spec) in block.specs.iter().enumerate() {
            *byte = spec.byte;
            if pos >= requested && !(requested == 0 && pos == 0) {
                continue;
            }
            let val = if let Some(arr) = engine.array(&spec.name) {
                Value::from(arr)
            } else if let Some(default_expr) = &spec.default {
                engine.eval(default_expr, None, out)?
            } else {
                continue;
            };
            let prefix = format!("Invalid output '{}'.", spec.name);
            validate_one_spec(engine, spec, val, &prefix, Some(&spec.name), byte, out)?;
        }
    }
    Ok(())
}

fn prefix_validation_error(engine: &mut Engine, prefix: &str, msg: String) -> String {
    let (raw, id) = engine.failure_text(&msg);
    let prefixed = format!("{prefix} {raw}");
    if id.is_empty() {
        prefixed
    } else {
        engine.raise(prefixed, id)
    }
}

fn invoke_validator(
    engine: &mut Engine,
    name: &str,
    args: Vec<Value>,
    out: &mut Execution,
) -> Result<()> {
    if let Some(func) = engine.resolve_function(name)? {
        engine.call_user(func, args, 0, None, out)?;
        return Ok(());
    }
    engine.call(name, args, out)?;
    Ok(())
}

fn is_class(engine: &mut Engine, val: &Value, cls: &str, out: &mut Execution) -> Result<bool> {
    let res = engine.call("isa", vec![val.clone(), Value::Text(cls.to_owned())], out)?;
    let arr = res.array(&engine.cancel)?;
    Ok(matches!(arr, Array::Logical(m) if m.data().first().copied() == Some(1)))
}

fn convert_to_class(
    engine: &mut Engine,
    val: Value,
    cls: &str,
    prefix: &str,
    out: &mut Execution,
) -> Result<Value> {
    if is_class(engine, &val, cls, out)? {
        return Ok(val);
    }
    let convertible_builtin = matches!(
        cls,
        "double"
            | "single"
            | "int8"
            | "int16"
            | "int32"
            | "int64"
            | "uint8"
            | "uint16"
            | "uint32"
            | "uint64"
            | "logical"
            | "char"
            | "string"
    );
    if convertible_builtin {
        if let Ok(converted) = engine.call(cls, vec![val.clone()], out) {
            return Ok(converted);
        }
    } else if !matches!(cls, "cell" | "struct" | "function_handle")
        && let Ok(Some(func)) = engine.resolve_function(cls)
        && let Ok(mut vals) = engine.call_user(func, vec![val], 1, None, out)
        && let Some(converted) = vals.pop()
        && is_class(engine, &converted, cls, out)?
    {
        return Ok(converted);
    }
    Err(engine.raise(
        format!("{prefix} Unable to convert value to {cls}."),
        "MATLAB:validation:UnableToConvert".into(),
    ))
}

fn format_dims(dims: &[ArgDim]) -> String {
    let parts: Vec<String> = dims
        .iter()
        .map(|d| match d {
            ArgDim::Fixed(n) => n.to_string(),
            ArgDim::Colon => ":".into(),
        })
        .collect();
    format!("({})", parts.join(","))
}

fn matches_dims(actual: &[usize], spec: &[ArgDim]) -> bool {
    let len = actual.len().max(spec.len());
    for i in 0..len {
        let a = actual.get(i).copied().unwrap_or(1);
        match spec.get(i) {
            Some(ArgDim::Fixed(n)) => {
                if a != *n {
                    return false;
                }
            }
            Some(ArgDim::Colon) => {}
            None => {
                if a != 1 {
                    return false;
                }
            }
        }
    }
    true
}

fn check_and_expand_size(
    engine: &mut Engine,
    val: Value,
    dims: &[ArgDim],
    prefix: &str,
    out: &mut Execution,
) -> Result<Value> {
    let arr = val.clone().array(&engine.cancel)?;
    let actual_dims = arr.dims();
    if matches_dims(&actual_dims, dims) {
        return Ok(val);
    }
    if let [rows, cols] = *actual_dims
        && (rows == 1 || cols == 1)
        && matches_dims(&[cols, rows], dims)
    {
        if let Array::Str(strings) = arr {
            return Ok(Value::from(Array::Str(strings.reshaped(cols, rows)?)));
        }
        return engine.profiler_builtin_call(
            "reshape",
            vec![
                Value::from(arr),
                Value::Matrix(Matrix::scalar(cols as f64)),
                Value::Matrix(Matrix::scalar(rows as f64)),
            ],
            Vec::new(),
            out,
            false,
        );
    }
    if !actual_dims.is_empty() && actual_dims.iter().all(|&d| d == 1) {
        let target: Vec<f64> = dims
            .iter()
            .map(|d| match d {
                ArgDim::Fixed(n) => *n as f64,
                ArgDim::Colon => 1.0,
            })
            .collect();
        let size_vec = Value::Matrix(Matrix::from_column_vec(1, target.len(), target)?);
        if let Ok(expanded) = engine.call("repmat", vec![val, size_vec], out) {
            return Ok(expanded);
        }
    }
    Err(engine.raise(
        format!("{prefix} Value must be of size {}.", format_dims(dims)),
        "MATLAB:validation:IncompatibleSize".into(),
    ))
}

fn fail_validator(engine: &mut Engine, id: &str, msg: impl Into<String>) -> Result<Option<Value>> {
    Err(engine.raise(msg.into(), format!("MATLAB:validators:{id}")))
}

fn arity(engine: &mut Engine, name: &str, args: &[Value], min: usize, max: usize) -> Result<()> {
    if args.len() < min {
        return Err(engine.raise(
            format!("Not enough input arguments for {name}."),
            "MATLAB:minrhs".into(),
        ));
    }
    if args.len() > max {
        return Err(engine.raise(
            format!("Too many input arguments for {name}."),
            "MATLAB:TooManyInputs".into(),
        ));
    }
    Ok(())
}

fn is_real_value(engine: &mut Engine, val: &Value, out: &mut Execution) -> Result<bool> {
    let res = engine.call("isreal", vec![val.clone()], out)?;
    let arr = res.array(&engine.cancel)?;
    Ok(matches!(arr, Array::Logical(m) if m.data().first().copied() == Some(1)))
}

fn extract_real_f64s(
    engine: &mut Engine,
    arr: &Array,
    _require_real: bool,
) -> std::result::Result<Vec<f64>, (&'static str, &'static str)> {
    match arr {
        Array::Numeric(m) => Ok(m.data().to_vec()),
        Array::Logical(m) => Ok(m.data().iter().map(|&b| f64::from(b)).collect()),
        Array::Typed(t) => Ok((0..t.len()).map(|i| t.real(i)).collect()),
        Array::Complex(_) => Err(("mustBeReal", "Value must be real.")),
        Array::Sparse(s) => {
            let dense = s.to_dense(&engine.cancel).map_err(|_| {
                (
                    "mustBeNumericOrLogical",
                    "Value must be numeric or logical.",
                )
            })?;
            Ok(dense.data().to_vec())
        }
        _ => Err((
            "mustBeNumericOrLogical",
            "Value must be numeric or logical.",
        )),
    }
}

#[derive(Clone, Copy)]
enum RealOperand {
    Integer(i128),
    Float(f64),
}

impl RealOperand {
    fn partial_cmp(self, other: Self) -> Option<Ordering> {
        use super::typed_values::int_vs_real;
        match (self, other) {
            (Self::Integer(x), Self::Integer(y)) => Some(x.cmp(&y)),
            (Self::Integer(x), Self::Float(y)) => int_vs_real(x, y),
            (Self::Float(x), Self::Integer(y)) => int_vs_real(y, x).map(Ordering::reverse),
            (Self::Float(x), Self::Float(y)) => x.partial_cmp(&y),
        }
    }
}

impl std::fmt::Display for RealOperand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Integer(value) => write!(f, "{value}"),
            Self::Float(value) => write!(f, "{value}"),
        }
    }
}

fn comparison_values<'a>(
    engine: &mut Engine,
    array: &'a Array,
) -> Result<impl Iterator<Item = RealOperand> + use<'a>> {
    if !matches!(
        array,
        Array::Numeric(_) | Array::Logical(_) | Array::Typed(_) | Array::Sparse(_)
    ) {
        let (id, message) = if matches!(array, Array::Complex(_)) {
            ("mustBeReal", "Value must be real.")
        } else {
            (
                "mustBeNumericOrLogical",
                "Value must be numeric or logical.",
            )
        };
        return Err(engine.raise(message.into(), format!("MATLAB:validators:{id}")));
    }
    Ok(real_operands(array))
}

fn real_operands(array: &Array) -> impl Iterator<Item = RealOperand> {
    // For a sparse array, one implicit zero suffices for an all-elements check.
    let len = match array {
        Array::Sparse(a) => a.nnz() + usize::from((a.nnz() as u64) < a.numel()),
        _ => array.len(),
    };
    (0..len).map(move |i| match array {
        Array::Numeric(a) => RealOperand::Float(a.data()[i]),
        Array::Logical(a) => RealOperand::Integer(i128::from(a.data()[i])),
        Array::Typed(a) if a.is_integer() => RealOperand::Integer(a.int(i)),
        Array::Typed(a) => RealOperand::Float(a.real(i)),
        Array::Sparse(a) => RealOperand::Float(if i < a.nnz() { a.value(i) } else { 0. }),
        _ => unreachable!("real numeric or logical array"),
    })
}

fn scalar_bound(engine: &Engine, val: &Value) -> Result<RealOperand> {
    let arr = val.clone().array(&engine.cancel)?;
    if arr.len() == 1
        && matches!(
            arr,
            Array::Numeric(_) | Array::Logical(_) | Array::Typed(_) | Array::Sparse(_)
        )
        && let Some(value) = real_operands(&arr).next()
        && !matches!(value, RealOperand::Float(x) if x.is_nan())
    {
        return Ok(value);
    }
    Err("Bound must be a real scalar.".into())
}

fn all_comparisons(
    engine: &Engine,
    values: impl Iterator<Item = RealOperand>,
    passes: impl Fn(RealOperand) -> bool,
) -> Result<bool> {
    for (i, value) in values.enumerate() {
        if i.is_multiple_of(8192) {
            engine.check_cancel()?;
        }
        if !passes(value) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn member_strings(engine: &Engine, val: &Value) -> Result<crate::StringArray> {
    match val {
        Value::Text(_)
        | Value::QuotedString(_)
        | Value::Container(Array::Character(_) | Array::Str(_) | Array::Cell(_)) => {
            engine.str_convert(val)
        }
        _ => Err("String membership requires string, character, or cell text inputs.".into()),
    }
}

fn member_preview(array: &Array) -> String {
    const LIMIT: usize = 16;
    let mut items: Vec<String> = match array {
        Array::Numeric(a) => a.data().iter().take(LIMIT).map(f64::to_string).collect(),
        Array::Logical(a) => a.data().iter().take(LIMIT).map(u8::to_string).collect(),
        Array::Typed(a) => (0..a.len().min(LIMIT)).map(|i| a.element_text(i)).collect(),
        Array::Cell(a) => (0..a.len().min(LIMIT))
            .map(|i| a.get_linear(i).expect("cell element in bounds").preview(64))
            .collect(),
        _ => return array.preview(LIMIT),
    };
    if array.len() > LIMIT {
        items.push("...".to_owned());
    }
    format!("[{}]", items.join(", "))
}

fn must_be_member(engine: &mut Engine, args: &[Value], out: &mut Execution) -> Result<bool> {
    if args.iter().any(super::string_class::is_string)
        && !args.iter().any(super::categorical_fns::is_cat)
    {
        use std::collections::BTreeSet;
        let input = member_strings(engine, &args[0])?;
        let allowed = member_strings(engine, &args[1])?;
        engine.preflight_elements(allowed.len().saturating_mul(4))?;
        let mut values = BTreeSet::new();
        for (i, text) in allowed.data().iter().enumerate() {
            if i.is_multiple_of(128) {
                engine.check_cancel()?;
            }
            if let Some(text) = text {
                values.insert(text.as_ref());
            }
        }
        for (i, text) in input.data().iter().enumerate() {
            if i.is_multiple_of(128) {
                engine.check_cancel()?;
            }
            if !text.as_deref().is_some_and(|text| values.contains(text)) {
                return Ok(false);
            }
        }
        return Ok(true);
    }
    let res = engine.call("ismember", args.to_vec(), out)?;
    Ok(matches!(res.array(&engine.cancel)?, Array::Logical(m) if m.data().iter().all(|&b| b != 0)))
}

fn is_text(engine: &Engine, val: &Value) -> Result<bool> {
    match val {
        Value::Text(_)
        | Value::QuotedString(_)
        | Value::Container(Array::Character(_) | Array::Str(_)) => Ok(true),
        Value::Container(Array::Cell(c)) => {
            for i in 0..c.len() {
                if i.is_multiple_of(128) {
                    engine.check_cancel()?;
                }
                if !matches!(c.get_linear(i)?.as_ref(), Array::Character(ch) if ch.is_text_row() && !ch.is_nd())
                {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn text_items(_engine: &mut Engine, val: &Value) -> std::result::Result<Vec<String>, ()> {
    match val {
        Value::Text(s) | Value::QuotedString(s) => Ok(vec![s.clone()]),
        Value::Container(Array::Character(c)) => {
            if c.rows() <= 1 {
                Ok(vec![String::from_utf16_lossy(c.units())])
            } else {
                Err(())
            }
        }
        Value::Container(Array::Str(s)) => {
            let mut out = Vec::with_capacity(s.len());
            for item in s.data() {
                match item {
                    Some(t) => out.push(t.as_ref().to_owned()),
                    None => return Err(()),
                }
            }
            Ok(out)
        }
        Value::Container(Array::Cell(c)) => {
            let mut out = Vec::with_capacity(c.len());
            for i in 0..c.len() {
                let Ok(slot) = c.get_linear(i) else {
                    return Err(());
                };
                match slot.as_ref() {
                    Array::Character(ch) if ch.rows() <= 1 => {
                        out.push(String::from_utf16_lossy(ch.units()));
                    }
                    _ => return Err(()),
                }
            }
            Ok(out)
        }
        _ => Err(()),
    }
}

fn text_list(engine: &mut Engine, val: &Value) -> Result<Vec<String>> {
    text_items(engine, val)
        .map_err(|()| "Class names must be a character vector, string array, or cellstr.".into())
}

pub(super) fn call_validator(
    engine: &mut Engine,
    name: &str,
    args: &[Value],
    out: &mut Execution,
) -> Result<Option<Value>> {
    if !name.starts_with("mustBe") {
        return Ok(None);
    }
    match name {
        "mustBePositive" | "mustBeNonnegative" | "mustBeNegative" | "mustBeNonpositive" => {
            arity(engine, name, args, 1, 1)?;
            let arr = args[0].clone().array(&engine.cancel)?;
            let vals = match extract_real_f64s(engine, &arr, true) {
                Ok(v) => v,
                Err((id, msg)) => return fail_validator(engine, id, msg),
            };
            let ok = match name {
                "mustBePositive" => vals.iter().all(|&x| x > 0.0),
                "mustBeNonnegative" => vals.iter().all(|&x| x >= 0.0),
                "mustBeNegative" => vals.iter().all(|&x| x < 0.0),
                _ => vals.iter().all(|&x| x <= 0.0),
            };
            if !ok {
                let msg = match name {
                    "mustBePositive" => "Value must be positive.",
                    "mustBeNonnegative" => "Value must be nonnegative.",
                    "mustBeNegative" => "Value must be negative.",
                    _ => "Value must be nonpositive.",
                };
                return fail_validator(engine, name, msg);
            }
        }
        "mustBeNonzero" => {
            arity(engine, name, args, 1, 1)?;
            let arr = args[0].clone().array(&engine.cancel)?;
            let ok = match &arr {
                Array::Numeric(m) => m.data().iter().all(|&x| x != 0.0),
                Array::Logical(m) => m.data().iter().all(|&b| b != 0),
                Array::Complex(m) => m.data().iter().all(|z| z.re != 0.0 || z.im != 0.0),
                Array::Typed(t) => (0..t.len()).all(|i| t.real(i) != 0.0),
                Array::Sparse(s) => {
                    let dense = s.to_dense(&engine.cancel)?;
                    dense.data().iter().all(|&x| x != 0.0)
                }
                _ => {
                    return fail_validator(
                        engine,
                        "mustBeNumericOrLogical",
                        "Value must be numeric or logical.",
                    );
                }
            };
            if !ok {
                return fail_validator(engine, name, "Value must be nonzero.");
            }
        }
        "mustBeFinite" => {
            arity(engine, name, args, 1, 1)?;
            let arr = args[0].clone().array(&engine.cancel)?;
            let ok = match &arr {
                Array::Numeric(m) => m.data().iter().all(|x| x.is_finite()),
                Array::Logical(_) => true,
                Array::Complex(m) => m
                    .data()
                    .iter()
                    .all(|z| z.re.is_finite() && z.im.is_finite()),
                Array::Typed(t) => (0..t.len()).all(|i| t.real(i).is_finite()),
                Array::Sparse(s) => s
                    .to_dense(&engine.cancel)?
                    .data()
                    .iter()
                    .all(|x| x.is_finite()),
                _ => {
                    return fail_validator(
                        engine,
                        "mustBeNumericOrLogical",
                        "Value must be numeric or logical.",
                    );
                }
            };
            if !ok {
                return fail_validator(engine, name, "Value must be finite.");
            }
        }
        "mustBeNonNan" => {
            arity(engine, name, args, 1, 1)?;
            let arr = args[0].clone().array(&engine.cancel)?;
            let ok = match &arr {
                Array::Numeric(m) => m.data().iter().all(|x| !x.is_nan()),
                Array::Logical(_) => true,
                Array::Complex(m) => m.data().iter().all(|z| !z.re.is_nan() && !z.im.is_nan()),
                Array::Typed(t) => (0..t.len()).all(|i| !t.real(i).is_nan()),
                Array::Sparse(s) => s
                    .to_dense(&engine.cancel)?
                    .data()
                    .iter()
                    .all(|x| !x.is_nan()),
                _ => {
                    return fail_validator(
                        engine,
                        "mustBeNumericOrLogical",
                        "Value must be numeric or logical.",
                    );
                }
            };
            if !ok {
                return fail_validator(engine, name, "Value must not be NaN.");
            }
        }
        "mustBeInteger" => {
            arity(engine, name, args, 1, 1)?;
            let arr = args[0].clone().array(&engine.cancel)?;
            let vals = match extract_real_f64s(engine, &arr, true) {
                Ok(v) => v,
                Err((id, msg)) => return fail_validator(engine, id, msg),
            };
            if !vals.iter().all(|&x| x.is_finite() && x.fract() == 0.0) {
                return fail_validator(engine, name, "Value must be integer.");
            }
        }
        "mustBeReal" => {
            arity(engine, name, args, 1, 1)?;
            if !is_real_value(engine, &args[0], out)? {
                return fail_validator(engine, name, "Value must be real.");
            }
        }
        "mustBeNumeric" => {
            arity(engine, name, args, 1, 1)?;
            if !args[0].clone().array(&engine.cancel)?.is_empty() {
                let res = engine.call("isnumeric", vec![args[0].clone()], out)?;
                let ok = matches!(
                    res.array(&engine.cancel)?,
                    Array::Logical(m) if m.data().first().copied() == Some(1)
                );
                if !ok {
                    return fail_validator(engine, name, "Value must be numeric.");
                }
            }
        }
        "mustBeNumericOrLogical" => {
            arity(engine, name, args, 1, 1)?;
            let num_res = engine.call("isnumeric", vec![args[0].clone()], out)?;
            let is_num = matches!(
                num_res.array(&engine.cancel)?,
                Array::Logical(m) if m.data().first().copied() == Some(1)
            );
            let log_res = engine.call("islogical", vec![args[0].clone()], out)?;
            let is_log = matches!(
                log_res.array(&engine.cancel)?,
                Array::Logical(m) if m.data().first().copied() == Some(1)
            );
            if !is_num && !is_log {
                return fail_validator(engine, name, "Value must be numeric or logical.");
            }
        }
        "mustBeNonempty" => {
            arity(engine, name, args, 1, 1)?;
            let arr = args[0].clone().array(&engine.cancel)?;
            if arr.is_empty() {
                return fail_validator(engine, name, "Value must not be empty.");
            }
        }
        "mustBeMember" => {
            arity(engine, name, args, 2, 2)?;
            if !must_be_member(engine, args, out)? {
                let allowed = args[1].clone().array(&engine.cancel)?;
                return fail_validator(
                    engine,
                    name,
                    format!(
                        "Value must be a member of the specified set: {}.",
                        member_preview(&allowed)
                    ),
                );
            }
        }
        "mustBeInRange" => {
            arity(engine, name, args, 3, 4)?;
            let arr = args[0].clone().array(&engine.cancel)?;
            let vals = comparison_values(engine, &arr)?;
            let lo = scalar_bound(engine, &args[1])?;
            let hi = scalar_bound(engine, &args[2])?;
            let bounds = if args.len() == 4 {
                let list = text_list(engine, &args[3])?;
                if list.len() != 1 {
                    return Err("Bounds flag must be a text scalar.".into());
                }
                list[0].clone()
            } else {
                "inclusive".into()
            };
            let (inc_lo, inc_hi) = match bounds.as_str() {
                "inclusive" => (true, true),
                "exclusive" => (false, false),
                "exclude-lower" => (false, true),
                "exclude-upper" => (true, false),
                _ => {
                    return Err(
                        "Bounds flag must be 'inclusive', 'exclusive', 'exclude-lower', or 'exclude-upper'."
                            .into(),
                    );
                }
            };
            let ok = all_comparisons(engine, vals, |x| {
                let above = x.partial_cmp(lo);
                let below = x.partial_cmp(hi);
                (above == Some(Ordering::Greater) || (inc_lo && above == Some(Ordering::Equal)))
                    && (below == Some(Ordering::Less) || (inc_hi && below == Some(Ordering::Equal)))
            })?;
            if !ok {
                let lb = if inc_lo { '[' } else { '(' };
                let rb = if inc_hi { ']' } else { ')' };
                return fail_validator(
                    engine,
                    name,
                    format!("Value must be in the range {lb}{lo}, {hi}{rb}."),
                );
            }
        }
        "mustBeGreaterThan"
        | "mustBeGreaterThanOrEqual"
        | "mustBeLessThan"
        | "mustBeLessThanOrEqual" => {
            arity(engine, name, args, 2, 2)?;
            let arr = args[0].clone().array(&engine.cancel)?;
            let vals = comparison_values(engine, &arr)?;
            let c = scalar_bound(engine, &args[1])?;
            let (direction, inclusive, description) = match name {
                "mustBeGreaterThan" => (Ordering::Greater, false, "greater than"),
                "mustBeGreaterThanOrEqual" => (Ordering::Greater, true, "greater than or equal to"),
                "mustBeLessThan" => (Ordering::Less, false, "less than"),
                _ => (Ordering::Less, true, "less than or equal to"),
            };
            let ok = all_comparisons(engine, vals, |x| {
                let order = x.partial_cmp(c);
                order == Some(direction) || (inclusive && order == Some(Ordering::Equal))
            })?;
            if !ok {
                return fail_validator(engine, name, format!("Value must be {description} {c}."));
            }
        }
        "mustBeText" => {
            arity(engine, name, args, 1, 1)?;
            if !is_text(engine, &args[0])? {
                return fail_validator(
                    engine,
                    name,
                    "Value must be a character array, string array, or cell array of character vectors.",
                );
            }
        }
        "mustBeTextScalar" => {
            arity(engine, name, args, 1, 1)?;
            let ok = match &args[0] {
                Value::Text(_) | Value::QuotedString(_) => true,
                Value::Container(Array::Character(c)) => c.rows() <= 1,
                Value::Container(Array::Str(s)) => {
                    s.len() == 1 && s.data().first().is_some_and(Option::is_some)
                }
                _ => false,
            };
            if !ok {
                return fail_validator(
                    engine,
                    name,
                    "Value must be a character vector or a string scalar.",
                );
            }
        }
        "mustBeNonzeroLengthText" => {
            arity(engine, name, args, 1, 1)?;
            let Ok(items) = text_items(engine, &args[0]) else {
                return fail_validator(engine, name, "Value must be text with nonzero length.");
            };
            if items.is_empty() || items.iter().any(String::is_empty) {
                return fail_validator(engine, name, "Value must be text with nonzero length.");
            }
        }
        "mustBeFile" => {
            arity(engine, name, args, 1, 1)?;
            let Ok(items) = text_items(engine, &args[0]) else {
                return fail_validator(engine, name, "Value must be the name of an existing file.");
            };
            for item in items {
                let res = engine.call("isfile", vec![Value::Text(item.clone())], out)?;
                let ok = matches!(
                    res.array(&engine.cancel)?,
                    Array::Logical(m) if m.data().first().copied() == Some(1)
                );
                if !ok {
                    return fail_validator(engine, name, format!("File '{item}' does not exist."));
                }
            }
        }
        "mustBeFolder" => {
            arity(engine, name, args, 1, 1)?;
            let Ok(items) = text_items(engine, &args[0]) else {
                return fail_validator(
                    engine,
                    name,
                    "Value must be the name of an existing folder.",
                );
            };
            for item in items {
                let res = engine.call("isfolder", vec![Value::Text(item.clone())], out)?;
                let ok = matches!(
                    res.array(&engine.cancel)?,
                    Array::Logical(m) if m.data().first().copied() == Some(1)
                );
                if !ok {
                    return fail_validator(
                        engine,
                        name,
                        format!("Folder '{item}' does not exist."),
                    );
                }
            }
        }
        "mustBeVector" => {
            arity(engine, name, args, 1, 2)?;
            let allow_all_empties = if args.len() == 2 {
                let list = text_list(engine, &args[1])?;
                list.len() == 1 && list[0] == "allow-all-empties"
            } else {
                false
            };
            let arr = args[0].clone().array(&engine.cancel)?;
            let dims = arr.dims();
            let is_vec = dims.len() == 2 && (dims[0] == 1 || dims[1] == 1);
            if !is_vec && !(allow_all_empties && arr.is_empty()) {
                return fail_validator(engine, name, "Value must be a vector.");
            }
        }
        "mustBeRow" => {
            arity(engine, name, args, 1, 1)?;
            let arr = args[0].clone().array(&engine.cancel)?;
            let dims = arr.dims();
            if !(dims.len() == 2 && dims[0] == 1) {
                return fail_validator(engine, name, "Value must be a row vector.");
            }
        }
        "mustBeColumn" => {
            arity(engine, name, args, 1, 1)?;
            let arr = args[0].clone().array(&engine.cancel)?;
            let dims = arr.dims();
            if !(dims.len() == 2 && dims[1] == 1) {
                return fail_validator(engine, name, "Value must be a column vector.");
            }
        }
        "mustBeScalarOrEmpty" => {
            arity(engine, name, args, 1, 1)?;
            let arr = args[0].clone().array(&engine.cancel)?;
            let dims = arr.dims();
            let is_scalar = dims.iter().all(|&d| d == 1);
            if !is_scalar && !arr.is_empty() {
                return fail_validator(engine, name, "Value must be scalar or empty.");
            }
        }
        "mustBeA" => {
            arity(engine, name, args, 2, 2)?;
            let classes = text_list(engine, &args[1])?;
            let mut matched = false;
            for cls in &classes {
                if is_class(engine, &args[0], cls, out)? {
                    matched = true;
                    break;
                }
            }
            if !matched {
                return fail_validator(
                    engine,
                    name,
                    format!("Value must be of type {}.", classes.join(", ")),
                );
            }
        }
        "mustBeUnderlyingType" => {
            arity(engine, name, args, 2, 2)?;
            let types = text_list(engine, &args[1])?;
            let arr = args[0].clone().array(&engine.cancel)?;
            let actual = arr.class();
            if !types.iter().any(|t| t == actual) {
                return fail_validator(
                    engine,
                    name,
                    format!("Value must have underlying type {}.", types.join(", ")),
                );
            }
        }
        _ => return Ok(None),
    }
    Ok(Some(Value::Void))
}
