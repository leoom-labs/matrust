//! Parsing and validation for the LTI value properties.
use super::*;

pub(super) fn props_decode(s: &StructArray) -> Result<LtiProperties> {
    let optional = |name| {
        s.field_index(name)
            .map(|i| s.get(i, 0).map(|a| a.as_ref().clone()))
            .transpose()
    };
    let mut props = LtiProperties::default();
    for (index, name) in DELAYS.into_iter().enumerate() {
        if let Some(a) = optional(name)? {
            props.delay[index] = scalar(&a.into())
                .map_err(|_| format!("{name} must be a real scalar for a SISO model"))?;
        }
    }
    props.name = optional("Name")?;
    props.state = optional("StateName")?;
    props.notes = optional("Notes")?;
    props.user_data = optional("UserData")?;
    Ok(props)
}
pub(super) fn delay_check(sys: &Lti) -> Result<()> {
    for (name, delay) in DELAYS.into_iter().zip(sys.props.delay) {
        if !delay.is_finite() || delay < 0. {
            return Err(format!("{name} must be finite and nonnegative"));
        }
        if sys.ts != 0. && (delay.fract() != 0. || delay > (1_u64 << 53) as f64) {
            return Err(format!(
                "{name} must be a nonnegative integer number of sample periods"
            ));
        }
    }
    if sys.delay() != 0. {
        sys.require_siso("Transport delay properties")?;
        let total = sys.delay();
        if !total.is_finite()
            || !(total * sys.sample_period()).is_finite()
            || sys.ts != 0. && total > (1_u64 << 53) as f64
        {
            return Err("Total LTI delay exceeds the representable range".into());
        }
    }
    Ok(())
}
fn name_cells(e: &Engine, array: Array) -> Result<Array> {
    let items = match array {
        Array::Character(_) => return cell(e, array),
        Array::Str(s) if s.rows() <= 1 || s.cols() <= 1 => s
            .data()
            .iter()
            .map(|item| {
                char_array(e, item.as_deref().ok_or("Names must be nonmissing text")?).map(Arc::new)
            })
            .collect::<Result<Vec<_>>>()?,
        Array::Cell(c) if (c.rows() <= 1 || c.cols() <= 1) && c.cols() != 1 => (0..c.len())
            .map(|i| c.get_linear(i).cloned())
            .collect::<Result<Vec<_>>>()?,
        _ => return Ok(array),
    };
    Ok(CellArray::from_shared(items.len(), 1, items, &e.cancel)?.into())
}
fn notes_check(a: &Array) -> Result<()> {
    let good = match a {
        Array::Character(_) => true,
        Array::Str(s) => (s.rows() <= 1 || s.cols() <= 1) && s.data().iter().all(Option::is_some),
        Array::Cell(c) => (c.rows() <= 1 || c.cols() <= 1)
            && (0..c.len()).all(|i| matches!(c.get_linear(i).map(AsRef::as_ref), Ok(Array::Character(ch)) if ch.rows() <= 1)),
        _ => false,
    };
    if good {
        Ok(())
    } else {
        Err("Notes must be text or a vector of text values".into())
    }
}
pub(super) fn names_check(a: &Array, n: usize) -> Result<()> {
    let valid_text = |a: &Array| matches!(a, Array::Character(s) if s.rows()<=1 && s.len()<=1024);
    let valid = if n == 1 && valid_text(a) {
        true
    } else if let Array::Cell(c) = a {
        c.len() == n
            && (n == 0 || c.cols() == 1 || c.rows() == 1)
            && (0..c.len()).all(|i| c.get_linear(i).is_ok_and(|s| valid_text(s)))
    } else {
        false
    };
    if valid {
        Ok(())
    } else {
        Err("Signal names must be a character row for SISO or one character cell per port (at most 1024 units each)".into())
    }
}

pub(in crate::engine) fn property_facade(e: &Engine, a: &Array) -> Result<StructArray> {
    let Array::Struct(s) = a else {
        return Err("Expected an LTI model".into());
    };
    if pid_kind(a).is_some() || s.field_index("Name").is_some() {
        return Ok(s.clone());
    }
    let system = decode(e, a)?.ok_or("Expected an LTI model")?;
    let Value::Container(Array::Struct(s)) = encode(e, &system)? else {
        unreachable!()
    };
    Ok(s)
}
pub(in crate::engine) fn private_property(name: &str) -> bool {
    matches!(name, TAG | delays::PATHS)
}
pub(super) fn public_properties(e: &Engine, s: StructArray) -> Result<StructArray> {
    let s = s.without_field(TAG, &e.cancel)?;
    if s.field_index(delays::PATHS).is_some() {
        s.without_field(delays::PATHS, &e.cancel)
    } else {
        Ok(s)
    }
}
pub(super) fn property_name(s: &StructArray, key: &str) -> Result<String> {
    let query = key.to_ascii_lowercase();
    let aliases = match query.as_str() {
        "num" => "numerator",
        "den" => "denominator",
        "u" => "inputname",
        "y" => "outputname",
        _ => &query,
    };
    let names = s
        .field_names()
        .iter()
        .filter(|name| !private_property(name));
    if let Some(name) = names
        .clone()
        .find(|name| name.eq_ignore_ascii_case(aliases))
    {
        return Ok(name.clone());
    }
    let mut matches =
        names.filter(|name| !aliases.is_empty() && name.to_ascii_lowercase().starts_with(aliases));
    if let Some(name) = matches.next() {
        if matches.next().is_none() {
            return Ok(name.clone());
        }
        return Err(format!("Ambiguous LTI property '{key}'"));
    }
    Err(format!("Unknown LTI property '{key}'"))
}
pub(super) fn replace_properties(
    e: &Engine,
    s: &StructArray,
    updates: &[(String, Array)],
) -> Result<Lti> {
    let mut edited = s.clone();
    let update = |name| updates.iter().rfind(|(key, _)| key == name).map(|(_, a)| a);
    let old_ts = scalar(&field(s, "Ts")?.into())?;
    let ts = match update("Ts") {
        Some(a) => scalar(&a.clone().into()).map_err(|_| "Ts must be a real scalar")?,
        None => old_ts,
    };
    ts_check(ts)?;
    if s.field_index(delays::PATHS).is_some() {
        if (ts == 0.) != (old_ts == 0.)
            || updates
                .iter()
                .any(|(k, _)| matches!(k.as_str(), "A" | "B" | "C" | "D"))
        {
            return Err("Changing the time domain or state matrices of a model with internal delays requires a Padé approximation".into());
        }
        if ts != old_ts {
            let Array::Cell(paths) = field(s, delays::PATHS)? else {
                unreachable!()
            };
            let mut converted_paths = Vec::with_capacity(paths.len());
            for i in 0..paths.len() {
                let mut path =
                    decode(e, paths.get_linear(i)?)?.ok_or("Invalid internal delay path")?;
                path.ts = ts;
                converted_paths.push(path);
            }
            edited = edited.with_field(
                delays::PATHS,
                delays::encode_paths(e, &converted_paths)?,
                &e.cancel,
            )?;
        }
    }
    let previous = text(&field(s, "Variable")?.into())?;
    let variable = match update("Variable") {
        Some(a) => text(&a.clone().into()).map_err(|_| "Variable must be scalar text")?,
        None if (ts == 0.) != (old_ts == 0.) => default_variable(ts).into(),
        None => previous.clone(),
    };
    variable_check(&variable, ts)?;
    let tf = s.field_index("Numerator").is_some();
    let zpk = s.field_index("Z").is_some();
    if tf && inverse_variable(&previous) != inverse_variable(&variable) {
        let old = decode(e, &s.clone().into())?.ok_or("Invalid transfer function")?;
        let (n, d) = mimo::tf_fields(e, &old, &variable)?;
        edited = edited.with_field("Numerator", n, &e.cancel)?.with_field(
            "Denominator",
            d,
            &e.cancel,
        )?;
    }
    for (key, a) in updates {
        let value = match key.as_str() {
            "Name" => char_array(
                e,
                &text(&a.clone().into()).map_err(|_| "Name must be scalar text")?,
            )?,
            "InputName" | "OutputName" | "StateName" => name_cells(e, a.clone())?,
            "Notes" => {
                notes_check(a)?;
                a.clone()
            }
            "Ts" | "Variable" => continue,
            _ => a.clone(),
        };
        edited = edited.with_field(key, value, &e.cancel)?;
    }
    if tf && (update("Numerator").is_some() || update("Denominator").is_some()) {
        let model = mimo::tf_model(
            e,
            &field(&edited, "Numerator")?.into(),
            &field(&edited, "Denominator")?.into(),
            &variable,
            false,
        )?;
        if let Model::TfArray(n, d) = model {
            edited = edited
                .with_field("Numerator", n.into(), &e.cancel)?
                .with_field("Denominator", d.into(), &e.cancel)?;
        }
    }
    if zpk && ["Z", "P", "K"].iter().any(|name| update(name).is_some()) {
        let model = mimo::zpk_model(
            e,
            &field(&edited, "Z")?.into(),
            &field(&edited, "P")?.into(),
            &field(&edited, "K")?.into(),
            false,
        )?;
        if let Model::ZpkArray(z, p, k) = model {
            edited = edited
                .with_field("Z", z.into(), &e.cancel)?
                .with_field("P", p.into(), &e.cancel)?
                .with_field("K", k.into(), &e.cancel)?;
        }
    }
    edited = edited
        .with_field("Ts", Matrix::scalar(ts).into(), &e.cancel)?
        .with_field("Variable", char_array(e, &variable)?, &e.cancel)?;
    decode(e, &edited.into())?.ok_or_else(|| "Invalid LTI model".into())
}
pub(in crate::engine) fn lti_set_options(e: &Engine, a: &Array, args: &[Value]) -> Result<Value> {
    let facade = property_facade(e, a)?;
    let kind = class(a).ok_or("Expected an LTI model")?;
    let updates = property_updates(e, kind, &facade, args)?;
    encode(e, &replace_properties(e, &facade, &updates)?)
}
fn property_updates(
    e: &Engine,
    name: &str,
    facade: &StructArray,
    options: &[Value],
) -> Result<Vec<(String, Array)>> {
    if !options.len().is_multiple_of(2) {
        return Err(format!("{name}: expected property name-value pairs"));
    }
    options
        .as_chunks::<2>()
        .0
        .iter()
        .map(|[key, value]| {
            let key = property_name(facade, &text(key)?)?;
            Ok((key, value.clone().array(&e.cancel)?))
        })
        .collect()
}
pub(super) fn apply_options(e: &Engine, name: &str, s: Lti, options: &[Value]) -> Result<Lti> {
    if options.is_empty() {
        return Ok(s);
    }
    let Value::Container(Array::Struct(facade)) = encode(e, &s)? else {
        unreachable!()
    };
    let updates = property_updates(e, name, &facade, options)?;
    replace_properties(e, &facade, &updates)
}

pub(super) fn copy_metadata(result: &mut Lti, a: &Lti, b: &Lti) {
    result.variable = [a, b]
        .into_iter()
        .find(|s| !static_model(s) && (s.ts == 0.) == (result.ts == 0.))
        .map_or_else(
            || default_variable(result.ts).into(),
            |s| s.variable.clone(),
        );
    result.props.name = a
        .props
        .name
        .as_ref()
        .filter(|v| nonempty_model_name(v))
        .or_else(|| b.props.name.as_ref().filter(|v| nonempty_model_name(v)))
        .cloned();
    result.props.notes = a
        .props
        .notes
        .as_ref()
        .filter(|v| !v.is_empty())
        .or_else(|| b.props.notes.as_ref().filter(|v| !v.is_empty()))
        .cloned();
    result.props.user_data = a
        .props
        .user_data
        .as_ref()
        .filter(|v| !v.is_empty())
        .or_else(|| b.props.user_data.as_ref().filter(|v| !v.is_empty()))
        .cloned();
}
fn nonempty_model_name(a: &Array) -> bool {
    matches!(a, Array::Character(c) if !c.is_empty())
}
pub(super) fn joined_states(e: &Engine, children: [&Lti; 2]) -> Result<Option<Array>> {
    state_names(
        e,
        children.into_iter().map(|s| {
            let count = match &s.model {
                Model::Tf(_, d) => d.len() - 1,
                Model::Zpk(_, p, k) if *k != 0. => p.len(),
                _ => s.states(),
            };
            (s, count)
        }),
    )
}

pub(super) fn state_names<'a>(
    e: &Engine,
    children: impl Iterator<Item = (&'a Lti, usize)> + Clone,
) -> Result<Option<Array>> {
    if children.clone().all(|(s, _)| s.props.state.is_none()) {
        return Ok(None);
    }
    let mut names = Vec::new();
    let empty = Arc::new(char_array(e, "")?);
    for (s, count) in children {
        for index in 0..count {
            names.push(match &s.props.state {
                Some(Array::Cell(c)) => c.get_linear(index)?.clone(),
                Some(a) => Arc::new(a.clone()),
                None => empty.clone(),
            });
        }
    }
    Ok(Some(
        CellArray::from_shared(names.len(), 1, names, &e.cancel)?.into(),
    ))
}
