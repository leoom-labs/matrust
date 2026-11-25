//! Tapped-delay networks. Series-parallel training expands observed inputs and
//! targets into static columns; closed-loop simulation feeds predicted outputs
//! back in original data units. State cells are chronological, oldest first.
use super::*;
#[derive(Clone)]
pub(in crate::engine) struct Dynamic {
    pub input_delays: Vec<usize>,
    pub feedback_delays: Vec<usize>,
    pub closed: bool,
    pub input_size: usize,
    pub feedback_size: usize,
}
impl Dynamic {
    pub(super) fn input_state_depth(&self) -> usize {
        if self.closed {
            self.input_depth()
        } else {
            self.depth()
        }
    }
    pub(super) fn layer_state_depth(&self) -> usize {
        if self.closed {
            self.feedback_depth()
        } else {
            0
        }
    }
    fn input_depth(&self) -> usize {
        self.input_delays.iter().copied().max().unwrap_or(0)
    }
    fn feedback_depth(&self) -> usize {
        self.feedback_delays.iter().copied().max().unwrap_or(0)
    }
    fn depth(&self) -> usize {
        self.input_depth().max(self.feedback_depth())
    }
    pub(super) fn inputs(&self) -> usize {
        usize::from(!self.input_delays.is_empty())
            + usize::from(!self.closed && !self.feedback_delays.is_empty())
    }
}
pub(super) fn handles(n: &str) -> bool {
    matches!(
        n,
        "narxnet"
            | "narnet"
            | "timedelaynet"
            | "preparets"
            | "closeloop"
            | "openloop"
            | "removedelay"
    )
}
fn delays(e: &Engine, v: Option<&Value>) -> Result<Vec<usize>> {
    let Some(v) = v else {
        return Ok(vec![1, 2]);
    };
    let m = matrix(e, v)?;
    let d = m
        .data()
        .iter()
        .map(|&x| size(x, true))
        .collect::<Result<Vec<_>>>()?;
    if m.rows() != 1
        || d.is_empty()
        || d.len() > 256
        || d.iter().any(|&n| n > 4096)
        || d.windows(2).any(|x| x[0] >= x[1])
    {
        return Err(
            "network: delays must be an increasing row vector, at most 256 taps and delay 4096"
                .into(),
        );
    }
    Ok(d)
}
pub(super) fn call(e: &mut Engine, name: &str, args: &[Value]) -> Result<Vec<Value>> {
    if matches!(name, "narxnet" | "narnet" | "timedelaynet") {
        let narx = name == "narxnet";
        let td = name == "timedelaynet";
        let base = if narx { 2 } else { 1 };
        if args.len() > base + if td { 2 } else { 3 } {
            return Err(format!("{name}: too many inputs"));
        }
        let id = if name != "narnet" {
            delays(e, args.first())?
        } else {
            vec![]
        };
        let fd = if !td {
            delays(e, args.get(usize::from(narx)))?
        } else {
            vec![]
        };
        let hidden = if let Some(v) = args.get(base) {
            let m = matrix(e, v)?;
            if m.rows() != 1 {
                return Err("network: hiddenSizes must be a row vector".into());
            }
            m.data()
                .iter()
                .map(|&x| size(x, false))
                .collect::<Result<Vec<_>>>()?
        } else {
            vec![10]
        };
        let closed = if td {
            false
        } else {
            let mode = args
                .get(base + 1)
                .map(|v| value_text(e, v))
                .transpose()?
                .unwrap_or("open".into());
            if !["open", "closed"].contains(&mode.as_str()) {
                return Err("network: feedbackMode must be open or closed".into());
            }
            mode == "closed"
        };
        if closed && fd.contains(&0) {
            return Err("network: zero-delay closed feedback is an algebraic loop".into());
        }
        let train = args
            .get(base + if td { 1 } else { 2 })
            .map(|v| value_text(e, v))
            .transpose()?
            .unwrap_or("trainlm".into());
        let mut o = properties::new_object(name, &hidden, &train, "mse", &e.cancel)?;
        o.props
            .insert("divideMode".into(), string("time", &e.cancel)?);
        o.props
            .insert("divideFcn".into(), string("divideblock", &e.cancel)?);
        o.props.insert("name".into(), string(name, &e.cancel)?);
        o.dynamic = Some(Dynamic {
            input_delays: id,
            feedback_delays: fd,
            closed,
            input_size: 0,
            feedback_size: 0,
        });
        return Ok(vec![super::super::objects::nnet_new(e, o)?]);
    }
    let mut o = super::super::objects::nnet_value(e, args.first().ok_or("network required")?)
        .ok_or("network required")?
        .as_ref()
        .clone();
    if o.dynamic
        .as_ref()
        .is_none_or(|d| d.feedback_delays.is_empty())
        && matches!(name, "closeloop" | "openloop")
    {
        if args.len() != 1 && args.len() != 3 {
            return Err(format!("{name}: expected net[,Xi,Ai]"));
        }
        let xi = if args.len() == 3 {
            args[1].clone()
        } else {
            Value::from(cell(1, 0, vec![], &e.cancel)?)
        };
        let ai = if args.len() == 3 {
            args[2].clone()
        } else {
            Value::from(cell(o.core.layers.len(), 0, vec![], &e.cancel)?)
        };
        return Ok(vec![super::super::objects::nnet_new(e, o)?, xi, ai]);
    }
    if o.dynamic.is_none() {
        return Err(format!("{name}: dynamic network required"));
    }
    if name == "preparets" {
        return prepare(e, &mut o, args);
    }
    if name == "removedelay" {
        if !(1..=2).contains(&args.len()) {
            return Err("removedelay: expected net[,n]".into());
        }
        let n = args
            .get(1)
            .map(|v| matrix(e, v).and_then(|m| size(m.as_scalar()?, true)))
            .transpose()?
            .unwrap_or(1);
        let d = o.dynamic.as_mut().unwrap();
        if d.input_delays.iter().any(|&x| x < n) {
            return Err("removedelay: removal exceeds minimum input delay".into());
        }
        for x in &mut d.input_delays {
            *x -= n;
        }
        for x in &mut d.feedback_delays {
            *x = x
                .checked_add(n)
                .filter(|&x| x <= 4096)
                .ok_or("removedelay: delay limit exceeded")?;
        }
        return Ok(vec![super::super::objects::nnet_new(e, o)?]);
    }
    if args.len() != 1 && args.len() != 3 {
        return Err(format!("{name}: expected net[,Xi,Ai]"));
    }
    let old = o.dynamic.clone().unwrap();
    let closed = name == "closeloop";
    if old.feedback_delays.is_empty() {
        return Err(format!("{name}: network has no feedback"));
    }
    if closed && old.feedback_delays.contains(&0) {
        return Err("closeloop: zero-delay feedback is an algebraic loop".into());
    }
    let (xi, ai) = if args.len() == 3 {
        convert_states(e, &o, &old, &args[1], &args[2], closed)?
    } else {
        (
            Value::from(cell(0, 0, vec![], &e.cancel)?),
            Value::from(cell(0, 0, vec![], &e.cancel)?),
        )
    };
    o.dynamic.as_mut().unwrap().closed = closed;
    Ok(vec![super::super::objects::nnet_new(e, o)?, xi, ai])
}
fn zero(rows: usize, q: usize) -> Result<Matrix> {
    Matrix::filled(rows, q, 0.)
}
fn cells(e: &Engine, v: Option<&Value>) -> Result<Vec<Vec<Matrix>>> {
    match v {
        None => Ok(vec![]),
        Some(v) => {
            if v.clone().array(&e.cancel)?.is_empty() {
                Ok(vec![])
            } else {
                series::read(e, v)
            }
        }
    }
}
// Check raw cells before per-signal shape validation or concatenation can hide
// a mismatch. Numeric training targets are already flattened over timesteps.
fn validate_samples(e: &Engine, name: &str, values: &[&Value]) -> Result<()> {
    let mut samples = None;
    for value in values {
        if let Value::Container(Array::Cell(c)) = value {
            for i in 0..c.len() {
                e.check_cancel()?;
                let m = matrix(e, &Value::from(c.get_linear(i)?.as_ref().clone()))?;
                if samples.is_some_and(|q| q != m.cols()) {
                    return Err(format!(
                        "{name}: inconsistent number of samples (columns) across timesteps/signals"
                    ));
                }
                samples = Some(m.cols());
            }
        }
    }
    Ok(())
}
type StateRows = Vec<Vec<Matrix>>;
fn history(
    e: &Engine,
    d: &Dynamic,
    xi: Option<&Value>,
    ai: Option<&Value>,
    q: usize,
    layers: usize,
) -> Result<(StateRows, StateRows)> {
    let state_elements = d
        .input_state_depth()
        .saturating_mul(d.input_size.saturating_add(d.feedback_size))
        .saturating_mul(q)
        .saturating_add(
            d.layer_state_depth()
                .saturating_mul(d.feedback_size.saturating_mul(layers))
                .saturating_mul(q),
        );
    if state_elements > e.limits.memory_bytes / 32 {
        return Err("network: delay state workspace memory limit exceeded".into());
    }
    let mut inputs = cells(e, xi)?;
    let mut layers_state = cells(e, ai)?;
    let depth = if d.closed { d.input_depth() } else { d.depth() };
    if inputs.is_empty() {
        inputs = (0..d.inputs())
            .map(|r| {
                Ok(vec![
                    zero(
                        if r == 0 && !d.input_delays.is_empty() {
                            d.input_size
                        } else {
                            d.feedback_size
                        },
                        q
                    )?;
                    depth
                ])
            })
            .collect::<Result<_>>()?;
    }
    if inputs.len() != d.inputs() || inputs.iter().any(|r| r.len() != depth) {
        return Err("network: Xi must be numInputs-by-numInputDelays cells".into());
    }
    for (i, row) in inputs.iter().enumerate() {
        let size = if i == 0 && !d.input_delays.is_empty() {
            d.input_size
        } else {
            d.feedback_size
        };
        if row.iter().any(|m| m.rows() != size || m.cols() != q) {
            return Err("network: Xi sample dimensions mismatch".into());
        }
    }
    let ad = if d.closed { d.feedback_depth() } else { 0 };
    if layers_state.is_empty() {
        layers_state = (0..layers)
            .map(|i| {
                Ok(vec![
                    zero(
                        if i + 1 == layers { d.feedback_size } else { 0 },
                        q
                    )?;
                    ad
                ])
            })
            .collect::<Result<_>>()?;
    }
    if layers_state.len() != layers || layers_state.iter().any(|r| r.len() != ad) {
        return Err("network: Ai must be numLayers-by-numLayerDelays cells".into());
    }
    if ad > 0
        && layers_state[layers - 1]
            .iter()
            .any(|m| m.rows() != d.feedback_size || m.cols() != q)
    {
        return Err("network: Ai output dimensions mismatch".into());
    }
    Ok((inputs, layers_state))
}
fn convert_states(
    e: &Engine,
    o: &Object,
    d: &Dynamic,
    xi: &Value,
    ai: &Value,
    closed: bool,
) -> Result<(Value, Value)> {
    if d.closed == closed {
        return Ok((xi.clone(), ai.clone()));
    }
    let raw = series::read(e, xi)?;
    let ar = series::read(e, ai)?;
    let q = raw
        .iter()
        .chain(ar.iter())
        .find_map(|r| r.first().map(Matrix::cols))
        .unwrap_or(1);
    let (mut x, mut a) = history(e, d, Some(xi), Some(ai), q, o.core.layers.len())?;
    let mut nd = d.clone();
    nd.closed = closed;
    if closed {
        let fb = x.pop().ok_or("closeloop: feedback states missing")?;
        a = o
            .core
            .layers
            .iter()
            .map(|l| Ok(vec![zero(l.size, q)?; d.feedback_depth()]))
            .collect::<Result<_>>()?;
        let last = a.len() - 1;
        a[last] = fb[fb.len() - d.feedback_depth()..].to_vec();
        for row in &mut x {
            *row = row[row.len() - d.input_depth()..].to_vec();
        }
    } else {
        for row in &mut x {
            let mut padded = vec![zero(d.input_size, q)?; d.depth() - row.len()];
            padded.append(row);
            *row = padded;
        }
        let mut fb = vec![zero(d.feedback_size, q)?; d.depth() - d.feedback_depth()];
        fb.extend_from_slice(&a[a.len() - 1]);
        x.push(fb);
        a = vec![vec![]; o.core.layers.len()];
    }
    Ok((
        series::value(e, &x, if closed { nd.input_depth() } else { nd.depth() })?,
        series::value(e, &a, if closed { nd.feedback_depth() } else { 0 })?,
    ))
}
fn prepare(e: &mut Engine, o: &mut Object, args: &[Value]) -> Result<Vec<Value>> {
    if !(2..=5).contains(&args.len()) {
        return Err("preparets: expected net,Xnf[,Tnf,Tf,EW]".into());
    }
    let d = o.dynamic.as_mut().unwrap();
    let target_index = if d.feedback_delays.is_empty() { 2 } else { 3 };
    let mut sample_values = vec![&args[1]];
    sample_values.extend(args.get(target_index));
    validate_samples(e, "preparets", &sample_values)?;
    let external = series::read(e, &args[1])?;
    if !d.feedback_delays.is_empty()
        && args
            .get(2)
            .is_some_and(|v| !v.clone().array(&e.cancel).is_ok_and(|a| a.is_empty()))
    {
        return Err("preparets: feedback networks require empty Tnf".into());
    }
    let target = cells(e, args.get(target_index))?;
    if target.len() > 1 || external.len() != usize::from(!d.input_delays.is_empty()) {
        return Err("preparets: input/target signal count mismatch".into());
    }
    let len = external.first().or(target.first()).map_or(0, Vec::len);
    let shift = d.depth();
    if len <= shift || target.iter().chain(external.iter()).any(|r| r.len() != len) {
        return Err("preparets: series must have equal lengths greater than maximum delay".into());
    }
    if !d.feedback_delays.is_empty() && target.is_empty() {
        return Err("preparets: feedback targets required".into());
    }
    let q = external.first().or(target.first()).unwrap()[0].cols();
    let mut streams = external.clone();
    if !d.closed && !d.feedback_delays.is_empty() {
        streams.push(target[0].clone());
    }
    let depth = if d.closed { d.input_depth() } else { d.depth() };
    let xi = streams
        .iter()
        .map(|r| r[shift - depth..shift].to_vec())
        .collect::<Vec<_>>();
    let xs = streams
        .iter()
        .map(|r| r[shift..].to_vec())
        .collect::<Vec<_>>();
    let mut ai = vec![vec![]; o.core.layers.len()];
    let ad = if d.closed { d.feedback_depth() } else { 0 };
    if ad > 0 {
        for (i, row) in ai.iter_mut().enumerate() {
            *row = if i + 1 == o.core.layers.len() {
                target[0][shift - ad..shift].to_vec()
            } else {
                vec![zero(o.core.layers[i].size, q)?; ad]
            };
        }
    }
    let ts = target
        .iter()
        .map(|r| r[shift..].to_vec())
        .collect::<Vec<_>>();
    let ew = if let Some(v) = args.get(4) {
        let r = series::read(e, v)?;
        if r.iter().any(|r| r.len() != len) {
            return Err("preparets: EW time length mismatch".into());
        }
        r.iter().map(|r| r[shift..].to_vec()).collect()
    } else {
        vec![vec![Matrix::scalar(1.); len - shift]]
    };
    Ok(vec![
        series::value(e, &xs, len - shift)?,
        series::value(e, &xi, depth)?,
        series::value(e, &ai, ad)?,
        series::value(e, &ts, len - shift)?,
        series::value(e, &ew, len - shift)?,
        Value::Matrix(Matrix::scalar(shift as f64)),
    ])
}
fn tap<'a>(current: &'a Matrix, h: &'a [Matrix], delay: usize) -> Result<&'a Matrix> {
    if delay == 0 {
        Ok(current)
    } else {
        h.get(
            h.len()
                .checked_sub(delay)
                .ok_or("network: missing delay state")?,
        )
        .ok_or("network: missing delay state".into())
    }
}
fn expanded(
    d: &Dynamic,
    streams: &[Vec<Matrix>],
    t: usize,
    xh: &[Vec<Matrix>],
    ah: &[Vec<Matrix>],
    q: usize,
) -> Result<Matrix> {
    let mut taps = Vec::new();
    if !d.input_delays.is_empty() {
        for &delay in &d.input_delays {
            taps.push(tap(&streams[0][t], &xh[0], delay)?);
        }
    }
    if !d.feedback_delays.is_empty() {
        let idx = usize::from(!d.input_delays.is_empty());
        let (current, h) = if d.closed {
            let h = &ah[ah.len() - 1];
            (h.last().ok_or("network: feedback history required")?, h)
        } else {
            (&streams[idx][t], &xh[idx])
        };
        for &delay in &d.feedback_delays {
            taps.push(tap(current, h, delay)?);
        }
    }
    let rows = taps.iter().map(|m| m.rows()).sum();
    let mut data = Vec::with_capacity(Matrix::checked_len(rows, q)?);
    for c in 0..q {
        for m in &taps {
            data.extend_from_slice(&m.data()[c * m.rows()..(c + 1) * m.rows()]);
        }
    }
    Matrix::from_column_vec(rows, q, data)
}
fn push(row: &mut [Matrix], m: Matrix) {
    if !row.is_empty() {
        row.rotate_left(1);
        let n = row.len();
        row[n - 1] = m;
    }
}
fn metadata(o: &mut Object, streams: &[Vec<Matrix>], target: Option<&Matrix>) -> Result<()> {
    let d = o.dynamic.as_mut().unwrap();
    if !d.input_delays.is_empty() {
        d.input_size = streams
            .first()
            .and_then(|r| r.first())
            .ok_or("network: external input required")?
            .rows();
    }
    if !d.feedback_delays.is_empty() {
        d.feedback_size = if d.closed {
            target.map_or(o.core.output.size, Matrix::rows)
        } else {
            streams
                .get(usize::from(!d.input_delays.is_empty()))
                .and_then(|r| r.first())
                .ok_or("network: feedback input required")?
                .rows()
        };
    }
    Ok(())
}
pub(super) fn training_inputs(
    e: &mut Engine,
    o: &mut Object,
    args: &[Value],
) -> Result<(Matrix, Matrix)> {
    if o.dynamic.as_ref().unwrap().closed {
        return Err("train: closed-loop training is not supported; use openloop".into());
    }
    validate_samples(e, "train", &args[1..].iter().collect::<Vec<_>>())?;
    let streams = series::read(e, &args[1])?;
    let t = sequence_matrix(e, &args[2])?;
    metadata(o, &streams, Some(&t))?;
    let d = o.dynamic.as_ref().unwrap();
    let len = streams.first().map_or(0, Vec::len);
    let q = streams
        .first()
        .and_then(|r| r.first())
        .map_or(0, Matrix::cols);
    if len == 0 || streams.len() != d.inputs() || t.cols() != len * q {
        return Err("train: time series dimensions mismatch".into());
    }
    let (mut xh, ah) = history(e, d, args.get(3), args.get(4), q, o.core.layers.len())?;
    let rows = d.input_delays.len() * d.input_size + d.feedback_delays.len() * d.feedback_size;
    budget(
        e,
        &o.core,
        len * q,
        rows.saturating_mul(len * q).saturating_mul(4),
    )?;
    let mut data = Vec::with_capacity(Matrix::checked_len(rows, len * q)?);
    for time in 0..len {
        e.check_cancel()?;
        data.extend_from_slice(expanded(d, &streams, time, &xh, &ah, q)?.data());
        for (r, row) in xh.iter_mut().enumerate() {
            push(row, streams[r][time].clone());
        }
    }
    Ok((Matrix::from_column_vec(rows, len * q, data)?, t))
}
pub(super) fn configure_call(e: &mut Engine, mut o: Object, args: &[Value]) -> Result<Vec<Value>> {
    arity("configure", args, 3)?;
    validate_samples(e, "configure", &args[1..].iter().collect::<Vec<_>>())?;
    let closed = o.dynamic.as_ref().unwrap().closed;
    let (x, t) = if closed {
        let mut inputs = series::read(e, &args[1])?;
        let target = series::read(e, &args[2])?;
        if target.len() != 1 {
            return Err("configure: one target signal required".into());
        }
        let len = target[0].len();
        if inputs.iter().any(|r| r.len() != len) {
            return Err("configure: time lengths must agree".into());
        }
        inputs.push(target[0].clone());
        let joined = series::value(e, &inputs, len)?;
        o.dynamic.as_mut().unwrap().closed = false;
        let data = training_inputs(e, &mut o, &[args[0].clone(), joined, args[2].clone()])?;
        o.dynamic.as_mut().unwrap().closed = true;
        data
    } else {
        training_inputs(e, &mut o, args)?
    };
    budget(
        e,
        &o.core,
        x.cols(),
        x.len().saturating_mul(4) + t.len() * 4,
    )?;
    o.core.configure(&x, &t, &e.cancel)?;
    o.core.init(|| e.random.next_f64(), &e.cancel)?;
    Ok(vec![super::super::objects::nnet_new(e, o)?])
}
pub(in crate::engine) fn simulate(
    e: &mut Engine,
    o: &Object,
    args: &[Value],
) -> Result<Vec<Value>> {
    if !(1..=3).contains(&args.len()) {
        return Err("network: expected X[,Xi,Ai]".into());
    }
    if !o.core.configured {
        return Err("network: not configured; use configure or train".into());
    }
    validate_samples(e, "sim", &args.iter().collect::<Vec<_>>())?;
    let streams = series::read(e, &args[0])?;
    let d = o.dynamic.as_ref().unwrap();
    let Value::Container(Array::Cell(c)) = &args[0] else {
        unreachable!()
    };
    let len = c.cols();
    if streams.len() != d.inputs() {
        return Err("network: time series input count mismatch".into());
    }
    let raw_x = cells(e, args.get(1))?;
    let raw_a = cells(e, args.get(2))?;
    let q = streams
        .iter()
        .chain(raw_x.iter())
        .chain(raw_a.iter())
        .find_map(|r| r.first().filter(|m| m.rows() > 0).map(Matrix::cols))
        .unwrap_or(1);
    for (i, row) in streams.iter().enumerate() {
        let rows = if i == 0 && !d.input_delays.is_empty() {
            d.input_size
        } else {
            d.feedback_size
        };
        if row.iter().any(|m| m.rows() != rows || m.cols() != q) {
            return Err("network: time series input dimensions mismatch".into());
        }
    }
    budget(
        e,
        &o.core,
        len * q,
        len.saturating_mul(q).saturating_mul(o.core.output.size) * 4,
    )?;
    let (mut xh, mut ah) = history(e, d, args.get(1), args.get(2), q, o.core.layers.len())?;
    let mut out = Vec::with_capacity(len);
    for t in 0..len {
        e.check_cancel()?;
        let x = expanded(d, &streams, t, &xh, &ah, q)?;
        let acts = o.core.activations(&x, &e.cancel)?;
        let y = process::reverse_chain(
            &o.core.output.process_settings,
            acts.last().unwrap(),
            &e.cancel,
        )?;
        for (r, row) in xh.iter_mut().enumerate() {
            push(row, streams[r][t].clone());
        }
        let last = ah.len() - 1;
        for (i, row) in ah.iter_mut().enumerate() {
            push(
                row,
                if i == last {
                    y.clone()
                } else {
                    acts[i].clone()
                },
            );
        }
        out.push(y);
    }
    Ok(vec![
        series::value(e, &[out], len)?,
        series::value(e, &xh, if d.closed { d.input_depth() } else { d.depth() })?,
        series::value(e, &ah, if d.closed { d.feedback_depth() } else { 0 })?,
    ])
}
