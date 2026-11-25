//! A bounded direct sum of SISO paths represents parallel transport delays.
//! Each path retains its own input/output delay and state coordinates. The
//! public A/B/C/D matrices are their direct sum with internal delays zeroed.
use super::*;

pub(super) const PATHS: &str = "MatRustLTIPaths";
const MAX_PATHS: usize = 32;

pub(super) fn decode_paths(e: &Engine, s: &StructArray, system: &Lti) -> Result<Vec<Lti>> {
    let Some(index) = s.field_index(PATHS) else {
        return Ok(Vec::new());
    };
    let Array::Cell(paths) = s.get(index, 0)?.as_ref() else {
        return Err("Invalid internal delay paths".into());
    };
    if !(2..=MAX_PATHS).contains(&paths.len()) || system.kind() != "ss" {
        return Err("Invalid internal delay path count or representation".into());
    }
    system.require_siso("Internal delays")?;
    let mut result = Vec::with_capacity(paths.len());
    let mut states = 0;
    for i in 0..paths.len() {
        let a = paths.get_linear(i)?;
        // Serialized children are flat; reject nesting before decoding.
        let Array::Struct(fields) = a.as_ref() else {
            return Err("Invalid internal delay path model".into());
        };
        if fields.field_index(PATHS).is_some() {
            return Err("Internal delay paths cannot be nested".into());
        }
        let path = decode(e, a)?.ok_or("Invalid internal delay path model")?;
        path.require_siso("Internal delays")?;
        if path.ts != system.ts || path.kind() != "ss" {
            return Err("Internal delay paths must share the model's timebase".into());
        }
        states += path.states();
        result.push(path);
    }
    if states != system.states() {
        return Err("Internal delay paths must match the model's state dimension".into());
    }
    Ok(result)
}

pub(super) fn encode_paths(e: &Engine, paths: &[Lti]) -> Result<Array> {
    let slots = paths
        .iter()
        .map(|path| encode(e, path)?.array(&e.cancel).map(Arc::new))
        .collect::<Result<Vec<_>>>()?;
    Ok(CellArray::from_shared(slots.len(), 1, slots, &e.cancel)?.into())
}

pub(super) fn full_paths(s: &Lti) -> Vec<Lti> {
    if s.props.paths.is_empty() {
        return vec![s.clone()];
    }
    s.props
        .paths
        .iter()
        .cloned()
        .map(|mut path| {
            for (part, outside) in path.props.delay.iter_mut().zip(s.props.delay) {
                *part += outside;
            }
            path
        })
        .collect()
}

pub(super) fn interconnect(
    e: &mut Engine,
    op: &Token,
    a: &Lti,
    b: &Lti,
    ts: f64,
    out: &mut Execution,
) -> Result<Lti> {
    let mut left = full_paths(a);
    let mut right = full_paths(b);
    if *op == Token::Slash && right.len() > 1 {
        return Err(
            "Division by a model with internal delays requires a Padé approximation".into(),
        );
    }
    let mut paths = if matches!(op, Token::Plus | Token::Minus) {
        if left.len() + right.len() > MAX_PATHS {
            return Err("Interconnection exceeds 32 internal delay paths".into());
        }
        if *op == Token::Minus {
            for path in &mut right {
                path.model = neg_model(&path.model);
            }
        }
        left.extend(right);
        left
    } else {
        if left.len() * right.len() > MAX_PATHS {
            return Err("Interconnection exceeds 32 internal delay paths".into());
        }
        let mut product = Vec::new();
        for l in &left {
            for r in &right {
                product.push(e.ctl_combine(op, l, r, out)?);
            }
        }
        product
    };
    for path in &mut paths {
        *path = e.ctl_convert(path, "ss", out)?;
        // A nondelayed static gain and unspecified sample periods can adopt
        // the common timebase selected by the interconnection.
        if path.ts != ts {
            path.ts = ts;
            path.variable = default_variable(ts).into();
        }
    }
    let mut result = direct_sum(e, paths, ts)?;
    result.input = if *op == Token::Star {
        b.input.clone()
    } else if *op == Token::Slash {
        b.output.clone()
    } else {
        a.input.clone()
    };
    result.output = a.output.clone();
    copy_metadata(&mut result, a, b);
    Ok(result)
}

pub(super) fn direct_sum(e: &Engine, mut paths: Vec<Lti>, ts: f64) -> Result<Lti> {
    let states = paths.iter().map(Lti::states).sum::<usize>();
    if states > MAX_STATES || states * (states + 2) + 1 > MAX_ENTRIES {
        return Err("Internal delay interconnection exceeds state-space bounds".into());
    }
    let mut a = vec![0.; states * states];
    let mut b = Vec::with_capacity(states);
    let mut c = Vec::with_capacity(states);
    let mut d = 0.;
    let mut names = Vec::with_capacity(states);
    let empty_name = Arc::new(char_array(e, "")?);
    let mut common = [f64::INFINITY; 3];
    for path in &mut paths {
        e.check_cancel()?;
        delay_check(path)?;
        let Model::Ss(pa, pb, pc, pd) = &path.model else {
            return Err("Internal delay paths require ordinary state-space realizations".into());
        };
        let start = b.len();
        for j in 0..pa.cols() {
            let offset = start + states * (start + j);
            a[offset..offset + pa.rows()].copy_from_slice(&pa.data()[j * pa.rows()..][..pa.rows()]);
        }
        b.extend_from_slice(pb.data());
        c.extend_from_slice(pc.data());
        d += pd.data()[0];
        for i in 0..pa.rows() {
            names.push(match &path.props.state {
                Some(Array::Cell(c)) => c.get_linear(i)?.clone(),
                Some(value) => Arc::new(value.clone()),
                None => empty_name.clone(),
            });
        }
        for (common, delay) in common.iter_mut().zip(path.props.delay) {
            *common = common.min(delay);
        }
        // Paths are implementation data. Only state names matter internally.
        path.props.name = None;
        path.props.notes = None;
        path.props.user_data = None;
        path.input = empty_name.as_ref().clone();
        path.output = empty_name.as_ref().clone();
    }
    let model = ss_model(
        Matrix::from_column_vec(states, states, a)?,
        Matrix::from_column_vec(states, 1, b)?,
        Matrix::from_column_vec(1, states, c)?,
        Matrix::scalar(d),
    )?;
    let mut result = new(e, model, ts)?;
    result.props.state = Some(CellArray::from_shared(states, 1, names, &e.cancel)?.into());
    result.props.delay = common;
    for path in &mut paths {
        for (delay, outside) in path.props.delay.iter_mut().zip(common) {
            *delay -= outside;
        }
    }
    if paths.iter().any(|path| path.delay() != 0.) {
        result.props.paths = paths;
    }
    Ok(result)
}

pub(super) fn keep_outer_properties(result: &mut Lti, original: &Lti) {
    result.input = original.input.clone();
    result.output = original.output.clone();
    result.props.name = original.props.name.clone();
    result.props.notes = original.props.notes.clone();
    result.props.user_data = original.props.user_data.clone();
}
