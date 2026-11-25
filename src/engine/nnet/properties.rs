use super::*;
use crate::syntax::PathStep;
pub(super) fn new_object(
    kind: &str,
    hidden: &[usize],
    train: &str,
    perf: &str,
    c: &AtomicBool,
) -> Result<Object> {
    if !perform::NAMES.contains(&perf) {
        return Err(format!("Unsupported performFcn '{perf}'"));
    }
    let core = Network::new(hidden, kind == "patternnet", kind == "cascadeforwardnet")?;
    let mut props = BTreeMap::new();
    for (key, value) in [
        (
            "name",
            match kind {
                "fitnet" => "Function Fitting Neural Network",
                "patternnet" => "Pattern Recognition Neural Network",
                "cascadeforwardnet" => "Cascade-Forward Neural Network",
                _ => "Feed-Forward Neural Network",
            },
        ),
        ("trainFcn", train),
        ("performFcn", perf),
        ("divideFcn", "dividerand"),
        ("divideMode", "sample"),
        ("initFcn", "initlay"),
    ] {
        props.insert(key.into(), string(value, c)?);
    }
    props.insert("trainParam".into(), train_defaults(train, c)?);
    props.insert(
        "performParam".into(),
        record(
            vec![
                ("regularization", num(0.)),
                ("normalization", string("none", c)?),
            ],
            c,
        )?,
    );
    props.insert("divideParam".into(), divide_defaults("dividerand", c)?);
    props.insert("initParam".into(), record(vec![], c)?);
    props.insert("userdata".into(), record(vec![], c)?);
    let plots = if kind == "patternnet" {
        vec![
            "plotperform",
            "plottrainstate",
            "ploterrhist",
            "plotconfusion",
            "plotroc",
        ]
    } else {
        vec![
            "plotperform",
            "plottrainstate",
            "ploterrhist",
            "plotregression",
            "plotfit",
        ]
    };
    props.insert(
        "plotFcns".into(),
        strings(&plots.into_iter().map(str::to_owned).collect::<Vec<_>>(), c)?,
    );
    Ok(Object {
        core,
        props,
        som: None,
        dynamic: None,
    })
}
pub(super) fn train_defaults(name: &str, c: &AtomicBool) -> Result<Array> {
    if name == "trainbu" {
        return som::defaults(c);
    }
    if let Ok(algorithm) = train::algorithm(name) {
        let defaults = algorithm.defaults();
        let strings = algorithm.string_defaults();
        let mut fields = defaults
            .iter()
            .map(|(key, &v)| {
                (
                    key.as_str(),
                    if matches!(key.as_str(), "showWindow" | "showCommandLine") {
                        Array::Logical(LogicalMatrix::scalar(v != 0.))
                    } else {
                        num(v)
                    },
                )
            })
            .collect::<Vec<_>>();
        for (key, value) in &strings {
            fields.push((key.as_str(), string(value, c)?));
        }
        return record(fields, c);
    }
    let grad = match name {
        "trainlm" | "trainbr" => 1e-7,
        "trainscg" | "trainbfg" => 1e-6,
        "traincgf" => 1e-10,
        "traingd" | "traingdm" | "traingdx" | "trainrp" => 1e-5,
        _ => return Err(format!("Unsupported trainFcn '{name}'")),
    };
    let mut fields = vec![
        ("epochs", num(1000.)),
        ("goal", num(0.)),
        ("max_fail", num(if name == "trainbr" { 0. } else { 6. })),
        ("min_grad", num(grad)),
        ("show", num(25.)),
        (
            "showCommandLine",
            Array::Logical(LogicalMatrix::scalar(false)),
        ),
        ("showWindow", Array::Logical(LogicalMatrix::scalar(true))),
        ("time", num(f64::INFINITY)),
    ];
    let numeric: &[(&str, f64)] = match name {
        "trainlm" => &[
            ("mu", 0.001),
            ("mu_dec", 0.1),
            ("mu_inc", 10.),
            ("mu_max", 1e10),
        ],
        "trainbr" => &[
            ("mu", 0.005),
            ("mu_dec", 0.1),
            ("mu_inc", 10.),
            ("mu_max", 1e10),
        ],
        "trainscg" => &[("mu", 0.005), ("sigma", 5e-5), ("lambda", 5e-7)],
        "traingd" => &[("lr", 0.01)],
        "traingdm" => &[("lr", 0.01), ("mc", 0.9)],
        "traingdx" => &[
            ("lr", 0.01),
            ("mc", 0.9),
            ("lr_inc", 1.05),
            ("lr_dec", 0.7),
            ("max_perf_inc", 1.04),
        ],
        "trainrp" => &[
            ("lr", 0.01),
            ("delt_inc", 1.2),
            ("delt_dec", 0.5),
            ("delta0", 0.07),
            ("deltamax", 50.),
        ],
        _ => &[
            ("scal_tol", 20.),
            ("alpha", 0.001),
            ("beta", 0.1),
            ("delta", 0.01),
            ("low_lim", 0.1),
            ("up_lim", 0.5),
            ("maxstep", 100.),
            ("minstep", 1e-6),
            ("bmax", 26.),
        ],
    };
    fields.extend(numeric.iter().map(|&(k, v)| (k, num(v))));
    if matches!(name, "trainbfg" | "traincgf") {
        fields.push((
            "searchFcn",
            string(
                if name == "trainbfg" {
                    "srchbac"
                } else {
                    "srchcha"
                },
                c,
            )?,
        ));
        fields.push((if name == "trainbfg" { "gamma" } else { "gama" }, num(0.1)));
        if name == "trainbfg" {
            fields.push(("batch_frag", num(0.)));
        }
    }
    record(fields, c)
}
fn divide_defaults(name: &str, c: &AtomicBool) -> Result<Array> {
    match name {
        "dividerand" | "divideblock" | "divideint" => record(
            vec![
                ("trainRatio", num(0.7)),
                ("valRatio", num(0.15)),
                ("testRatio", num(0.15)),
            ],
            c,
        ),
        "divideind" => record(
            vec![
                ("trainInd", empty()?),
                ("valInd", empty()?),
                ("testInd", empty()?),
            ],
            c,
        ),
        "dividetrain" | "" => record(vec![], c),
        _ => Err(format!("Unsupported divideFcn '{name}'")),
    }
}
pub(in crate::engine) fn property(e: &Engine, o: &Object, name: &str) -> Result<Value> {
    let c = &e.cancel;
    if let Some(a) = dynamic_properties::property(o, name, c)? {
        return Ok(Value::from(a));
    }
    let n = o.core.layers.len();
    let net = &o.core;
    let a = match name {
        "numInputs" => num(o.dynamic.as_ref().map_or(1, dynamic::Dynamic::inputs) as f64),
        "numOutputs" => num(1.),
        "numInputDelays" => num(o
            .dynamic
            .as_ref()
            .map_or(0, dynamic::Dynamic::input_state_depth) as f64),
        "numLayerDelays" => num(o
            .dynamic
            .as_ref()
            .map_or(0, dynamic::Dynamic::layer_state_depth) as f64),
        "inputWeights" => weight_metadata(o, c)?,
        "numLayers" => num(n as f64),
        "numWeightElements" => num(net.num_weight_elements() as f64),
        "inputs" => cell(1, 1, vec![port(&net.input, c)?], c)?,
        "outputs" => {
            let mut v = vec![empty()?; n];
            v[n - 1] = port(&net.output, c)?;
            if let Some(d) = &o.dynamic {
                v[n - 1] = extend_record(
                    &v[n - 1],
                    vec![
                        (
                            "feedbackMode",
                            string(
                                if d.feedback_delays.is_empty() {
                                    "none"
                                } else if d.closed {
                                    "closed"
                                } else {
                                    "open"
                                },
                                c,
                            )?,
                        ),
                        (
                            "feedbackDelays",
                            Array::Numeric(Matrix::from_column_vec(
                                1,
                                d.feedback_delays.len(),
                                d.feedback_delays.iter().map(|&x| x as f64).collect(),
                            )?),
                        ),
                    ],
                    c,
                )?;
            }
            cell(1, n, v, c)?
        }
        "layers" => cell(
            n,
            1,
            net.layers
                .iter()
                .map(|l| {
                    let mut extra = vec![];
                    if let Some(s) = &o.som {
                        extra.extend([
                            (
                                "dimensions",
                                Array::Numeric(Matrix::from_column_vec(
                                    1,
                                    s.dimensions.len(),
                                    s.dimensions.iter().map(|&x| x as f64).collect(),
                                )?),
                            ),
                            ("positions", Array::Numeric(s.positions.clone())),
                            ("distances", Array::Numeric(s.distances.clone())),
                            ("topologyFcn", string(&s.topology, c)?),
                            ("distanceFcn", string(&s.distance, c)?),
                        ]);
                    }
                    let mut fields = vec![
                        ("size", num(l.size as f64)),
                        ("transferFcn", string(&l.transfer_fcn, c)?),
                        ("initFcn", string(&l.init_fcn, c)?),
                        ("name", string(&l.name, c)?),
                    ];
                    fields.extend(extra);
                    record(fields, c)
                })
                .collect::<Result<_>>()?,
            c,
        )?,
        "biasConnect" => bools(n, 1, &net.bias_connect)?,
        "inputConnect" => bools(n, 1, &net.input_connect)?,
        "layerConnect" => bools(n, n, &net.layer_connect)?,
        "outputConnect" => bools(1, n, &(0..n).map(|i| i + 1 == n).collect::<Vec<_>>())?,
        "IW" | "iw" => cell(
            n,
            1,
            net.iw.iter().cloned().map(Array::Numeric).collect(),
            c,
        )?,
        "LW" | "lw" => cell(
            n,
            n,
            net.lw.iter().cloned().map(Array::Numeric).collect(),
            c,
        )?,
        "b" => cell(n, 1, net.b.iter().cloned().map(Array::Numeric).collect(), c)?,
        _ => o
            .props
            .get(name)
            .cloned()
            .ok_or_else(|| format!("Unrecognized property '{name}' for class 'network'."))?,
    };
    Ok(Value::from(a))
}
fn bools(r: usize, c: usize, v: &[bool]) -> Result<Array> {
    Ok(Array::Logical(LogicalMatrix::from_canonical_vec(
        r,
        c,
        v.iter().map(|&x| u8::from(x)).collect(),
    )?))
}
pub(super) fn port(p: &network::Port, c: &AtomicBool) -> Result<Array> {
    record(
        vec![
            ("size", num(p.size as f64)),
            ("processedSize", num(p.processed_size() as f64)),
            ("processFcns", strings(&p.process_fcns, c)?),
            (
                "processParams",
                cell(
                    1,
                    p.process_fcns.len(),
                    p.process_fcns
                        .iter()
                        .enumerate()
                        .map(|(i, n)| {
                            settings::params(
                                n,
                                p.process_params
                                    .get(i)
                                    .copied()
                                    .unwrap_or_else(|| process::defaults(n)),
                                c,
                            )
                        })
                        .collect::<Result<_>>()?,
                    c,
                )?,
            ),
            (
                "processSettings",
                cell(
                    1,
                    p.process_settings.len(),
                    p.process_settings
                        .iter()
                        .map(|s| settings::to_array(s, c))
                        .collect::<Result<_>>()?,
                    c,
                )?,
            ),
        ],
        c,
    )
}
pub(in crate::engine) fn assign(
    e: &mut Engine,
    o: &Object,
    steps: &[PathStep],
    value: Array,
    out: &mut Execution,
) -> Result<Object> {
    let name = match steps.first() {
        Some(PathStep::Field(s)) => s.clone(),
        Some(PathStep::Dynamic(expr)) => e.dynamic_field_name(expr, out)?,
        _ => return Err("network: use property assignment".into()),
    };
    let old = property(e, o, &name)?.array(&e.cancel)?;
    let value = if steps.len() > 1 {
        e.assign_step(Some(old.clone()), &steps[1..], value, false, out)?
    } else {
        value
    };
    if name == "inputs" && o.dynamic.is_some() {
        return dynamic_properties::assign_inputs(e, o, &old, &value);
    }
    let mut o = o.clone();
    let n = o.core.layers.len();
    match name.as_str() {
        "name" => {
            text(&value)?;
            o.props.insert(name, value);
        }
        "userdata" => {
            o.props.insert(name, value);
        }
        "trainFcn" => {
            let f = text(&value)?;
            o.props
                .insert("trainParam".into(), train_defaults(&f, &e.cancel)?);
            o.props.insert(name, value);
        }
        "trainParam" => {
            validate_record(&old, &value)?;
            let Array::Struct(params) = &value else {
                unreachable!()
            };
            for key in params.field_names() {
                let a = field(&value, key)?;
                if key == "searchFcn" {
                    text(a)?;
                    continue;
                }
                let v = scalar(a)?;
                if key == "show" && v.is_nan() {
                    continue;
                }
                if v < 0. || v.is_nan() || (key != "time" && !v.is_finite()) {
                    return Err(format!("trainParam.{key}: expected nonnegative scalar"));
                }
                if matches!(key.as_str(), "epochs" | "max_fail" | "show") && v.fract() != 0. {
                    return Err(format!("trainParam.{key}: expected integer"));
                }
                if matches!(key.as_str(), "showWindow" | "showCommandLine") && v != 0. && v != 1. {
                    return Err(format!("trainParam.{key}: expected logical scalar"));
                }
            }
            o.props.insert(name, value);
        }
        "performFcn" => {
            let f = text(&value)?;
            if !perform::NAMES.contains(&f.as_str()) {
                return Err(format!("Unsupported performFcn '{f}'"));
            }
            o.props.insert(name, value);
            o.props.insert(
                "performParam".into(),
                record(
                    vec![
                        ("regularization", num(0.)),
                        ("normalization", string("none", &e.cancel)?),
                    ],
                    &e.cancel,
                )?,
            );
        }
        "performParam" => {
            validate_record(&old, &value)?;
            let r = scalar(field(&value, "regularization")?)?;
            if !(0. ..=1.).contains(&r) {
                return Err("regularization must be between 0 and 1".into());
            }
            let s = text(field(&value, "normalization")?)?;
            if !["none", "standard", "percent"].contains(&s.as_str()) {
                return Err("Unsupported normalization".into());
            }
            o.props.insert(name, value);
        }
        "divideFcn" => {
            let f = text(&value)?;
            o.props
                .insert("divideParam".into(), divide_defaults(&f, &e.cancel)?);
            o.props.insert(name, value);
        }
        "divideParam" => {
            validate_record(&old, &value)?;
            o.props.insert(name, value);
        }
        "divideMode" => {
            let s = text(&value)?;
            if !["sample", "time", "sampletime", "all", "none"].contains(&s.as_str()) {
                return Err("Unsupported divideMode".into());
            }
            o.props.insert(name, value);
        }
        "plotFcns" => {
            text_list(&value)?;
            o.props.insert(name, value);
        }
        "layers" => {
            let Array::Cell(v) = &value else {
                return Err("layers must be a cell array".into());
            };
            if v.rows() != n || v.cols() != 1 {
                return Err("layers cell dimensions cannot change".into());
            }
            let Array::Cell(prev) = &old else {
                unreachable!()
            };
            for i in 0..n {
                let a = v.get_linear(i)?.as_ref();
                let before = prev.get_linear(i)?.as_ref();
                validate_record(before, a)?;
                unchanged_fields(e, before, a, &["size", "transferFcn", "initFcn", "name"])?;
                if arrays_equal(e, before, a)? {
                    continue;
                }
                let size = size(scalar(field(a, "size")?)?, true)?;
                let tf = text(field(a, "transferFcn")?)?;
                if !transfer::NAMES.contains(&tf.as_str()) {
                    return Err(format!("Unsupported transferFcn '{tf}'"));
                }
                let init = text(field(a, "initFcn")?)?;
                if !["initnw", "initwb"].contains(&init.as_str()) {
                    return Err(format!("Unsupported initFcn '{init}'"));
                }
                o.core.layers[i] = network::Layer {
                    size,
                    transfer_fcn: tf,
                    init_fcn: init,
                    name: text(field(a, "name")?)?,
                };
            }
            if arrays_equal(e, &old, &value)? {
                return Ok(o);
            }
            if o.core.layers[n - 1].size != o.core.output.processed_size() {
                o.core.output.process_settings.clear();
                o.core.output.size = o.core.layers[n - 1].size;
                o.core.configured = false;
            }
            o.core.resize_weights()?;
        }
        "inputs" | "outputs" => {
            let Array::Cell(v) = &value else {
                return Err("network ports must be cell arrays".into());
            };
            let expected = if name == "inputs" { 1 } else { n };
            if v.rows() != 1 || v.cols() != expected {
                return Err("network port cell dimensions cannot change".into());
            }
            for i in 0..expected - 1 {
                if !v.get_linear(i)?.is_empty() {
                    return Err("network: only the last layer has an output".into());
                }
            }
            let idx = expected - 1;
            let a = v.get_linear(idx)?.as_ref();
            let Array::Cell(prev) = &old else {
                unreachable!()
            };
            let old_port = prev.get_linear(idx)?.as_ref();
            validate_record(old_port, a)?;
            if name == "outputs" && o.dynamic.is_some() {
                let mode = text(field(a, "feedbackMode")?)
                    .map_err(|_| "network: invalid feedbackMode".to_string())?;
                if !["open", "closed", "none"].contains(&mode.as_str()) {
                    return Err(
                        "network: invalid feedbackMode; expected open, closed or none".into(),
                    );
                }
                if mode != text(field(old_port, "feedbackMode")?)? {
                    return Err("network: feedbackMode cannot be set directly; use closeloop(net) or openloop(net)".into());
                }
                if !arrays_equal(
                    e,
                    field(a, "feedbackDelays")?,
                    field(old_port, "feedbackDelays")?,
                )? {
                    return Err("network: feedbackDelays cannot be set directly; create the network with narxnet(inputDelays,feedbackDelays,...)".into());
                }
            }
            // Settings and derived size are read-only; recomputed by configure.
            for k in ["processSettings", "processedSize"] {
                if !arrays_equal(e, field(a, k)?, field(old_port, k)?)? {
                    return Err(format!("network: {k} is read-only; use configure"));
                }
            }
            // A metadata no-op must not resize an unconfigured network's
            // empty weights or otherwise change its value.
            if arrays_equal(e, old_port, a)? {
                return Ok(o);
            }
            let p = if name == "inputs" {
                &mut o.core.input
            } else {
                &mut o.core.output
            };
            let names = text_list(field(a, "processFcns")?)?;
            if names.len() > 16 || names.iter().any(|s| !process::NAMES.contains(&s.as_str())) {
                return Err("network: unsupported processFcns or too many processing steps".into());
            }
            let changed = names != p.process_fcns
                || scalar(field(a, "size")?)? != p.size as f64
                || !arrays_equal(
                    e,
                    field(a, "processParams")?,
                    field(old_port, "processParams")?,
                )?;
            if names != p.process_fcns {
                p.process_params = names.iter().map(|s| process::defaults(s)).collect();
            } else {
                let Array::Cell(params) = field(a, "processParams")? else {
                    return Err("processParams must be a cell array".into());
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
            p.size = size(scalar(field(a, "size")?)?, true)?;
            p.process_fcns = names;
            if changed {
                p.process_settings.clear();
                o.core.configured = false;
            }
            if name == "outputs" {
                o.core.layers[n - 1].size = o.core.output.processed_size();
            }
            o.core.resize_weights()?;
        }
        "IW" | "iw" | "LW" | "lw" | "b" => {
            let is_lw = matches!(name.as_str(), "LW" | "lw");
            let v = matrices(&value, n, if is_lw { n } else { 1 })?;
            let dest = if is_lw {
                &mut o.core.lw
            } else if name == "b" {
                &mut o.core.b
            } else {
                &mut o.core.iw
            };
            for (a, b) in v.iter().zip(dest.iter()) {
                if a.rows() != b.rows() || a.cols() != b.cols() {
                    return Err(format!(
                        "network: {name} weight size does not match configured dimensions"
                    ));
                }
                if a.data().iter().any(|v| !v.is_finite()) {
                    return Err("network: weights must be finite".into());
                }
            }
            *dest = v;
        }
        "biasConnect" | "inputConnect" | "layerConnect" => {
            let m = matrix(e, &Value::from(value))?;
            let cols = if name == "layerConnect" { n } else { 1 };
            if m.rows() != n || m.cols() != cols || m.data().iter().any(|&x| x != 0. && x != 1.) {
                return Err(format!("{name}: invalid connection matrix"));
            }
            let v: Vec<bool> = m.data().iter().map(|&x| x != 0.).collect();
            if name == "layerConnect" {
                for j in 0..n {
                    for i in 0..=j {
                        if v[i + j * n] {
                            return Err("network: recurrent connections are not supported".into());
                        }
                    }
                }
                o.core.layer_connect = v;
            } else if name == "biasConnect" {
                o.core.bias_connect = v;
            } else {
                o.core.input_connect = v;
            }
            o.core.resize_weights()?;
        }
        _ => return Err(format!("network: property '{name}' is read-only")),
    }
    budget(e, &o.core, 0, o.core.num_weight_elements())?;
    Ok(o)
}
fn unchanged_fields(e: &mut Engine, old: &Array, new: &Array, applied: &[&str]) -> Result<()> {
    let Array::Struct(s) = old else {
        unreachable!()
    };
    for name in s.field_names() {
        if !applied.contains(&name.as_str())
            && !arrays_equal(e, field(old, name)?, field(new, name)?)?
        {
            return Err(format!(
                "network: layer {name} is read-only; create the network with selforgmap"
            ));
        }
    }
    Ok(())
}

fn validate_record(old: &Array, new: &Array) -> Result<()> {
    let (Array::Struct(a), Array::Struct(b)) = (old, new) else {
        return Err("network: expected a scalar struct".into());
    };
    if b.len() != 1 {
        return Err("network: expected a scalar struct".into());
    }
    for name in b.field_names() {
        if a.field_index(name).is_none() {
            return Err(format!(
                "Unrecognized property '{name}' for class 'network'."
            ));
        }
    }
    for name in a.field_names() {
        if b.field_index(name).is_none() {
            return Err(format!("Missing network property '{name}'"));
        }
    }
    Ok(())
}

pub(super) fn arrays_equal(e: &mut Engine, a: &Array, b: &Array) -> Result<bool> {
    match e.isequal_call(&[Value::from(a.clone()), Value::from(b.clone())])? {
        Value::Logical(m) => Ok(m.data() == [1]),
        _ => Err("network: invalid equality result".into()),
    }
}

fn extend_record(a: &Array, extra: Vec<(&str, Array)>, c: &AtomicBool) -> Result<Array> {
    let Array::Struct(s) = a else {
        return Err("network: expected struct".into());
    };
    let mut names = s.field_names().to_vec();
    let mut values = (0..names.len())
        .map(|i| s.get(i, 0).cloned())
        .collect::<Result<Vec<_>>>()?;
    for (name, value) in extra {
        names.push(name.into());
        values.push(Arc::new(value));
    }
    Ok(Array::Struct(StructArray::from_slots(
        1, 1, names, values, c,
    )?))
}
fn weight_metadata(o: &Object, c: &AtomicBool) -> Result<Array> {
    let n = o.core.layers.len();
    let inputs = o.dynamic.as_ref().map_or(1, dynamic::Dynamic::inputs);
    let mut v = vec![];
    for j in 0..inputs {
        for i in 0..n {
            if i != 0 {
                v.push(empty()?);
                continue;
            }
            let delays = if let Some(d) = &o.dynamic {
                if j == 0 && !d.input_delays.is_empty() {
                    d.input_delays.clone()
                } else {
                    d.feedback_delays.clone()
                }
            } else {
                vec![0]
            };
            let mut fields = vec![(
                "delays",
                Array::Numeric(Matrix::from_column_vec(
                    1,
                    delays.len(),
                    delays.iter().map(|&x| x as f64).collect(),
                )?),
            )];
            if let Some(s) = &o.som {
                fields.extend([
                    ("initFcn", string("initsompc", c)?),
                    ("learnFcn", string("learnsomb", c)?),
                    ("weightFcn", string("negdist", c)?),
                    (
                        "learnParam",
                        record(
                            vec![
                                ("steps", num(s.cover as f64)),
                                ("init_neighborhood", num(s.neighbor)),
                            ],
                            c,
                        )?,
                    ),
                ]);
            }
            v.push(record(fields, c)?);
        }
    }
    cell(n, inputs, v, c)
}
