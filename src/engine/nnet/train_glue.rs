//! Interpreter adaptation; numerical training has no Engine dependency.
use super::*;
use divide::Division;
const DIVIDERS: &[&str] = &[
    "dividerand",
    "divideblock",
    "divideint",
    "divideind",
    "dividetrain",
];
pub(super) fn handles(name: &str) -> bool {
    name == "train" || DIVIDERS.contains(&name)
}
fn row(v: Vec<f64>) -> Result<Array> {
    Ok(Array::Numeric(Matrix::from_column_vec(1, v.len(), v)?))
}
fn indices(v: &[usize]) -> Result<Array> {
    row(v.iter().map(|&i| (i + 1) as f64).collect())
}
fn ids(e: &Engine, v: &Value) -> Result<Vec<usize>> {
    let m = matrix(e, v)?;
    if m.rows() > 1 && m.cols() > 1 {
        return Err("divideind: expected index vector".into());
    }
    m.data()
        .iter()
        .map(|&v| size(v, false).map(|i| i - 1))
        .collect()
}
fn division(e: &Engine, name: &str, p: &Array) -> Result<Division> {
    let get = |key| scalar(field(p, key)?);
    Ok(match name {
        "" | "dividetrain" => Division::All,
        "divideind" => Division::Indices([
            ids(e, &Value::from(field(p, "trainInd")?.clone()))?,
            ids(e, &Value::from(field(p, "valInd")?.clone()))?,
            ids(e, &Value::from(field(p, "testInd")?.clone()))?,
        ]),
        _ => {
            let ratios = [get("trainRatio")?, get("valRatio")?, get("testRatio")?];
            match name {
                "dividerand" => Division::Random(ratios),
                "divideblock" => Division::Block(ratios),
                "divideint" => Division::Interleaved(ratios),
                _ => return Err("train: unsupported divideFcn".into()),
            }
        }
    })
}
pub(super) fn call(e: &mut Engine, name: &str, args: &[Value]) -> Result<Vec<Value>> {
    if name != "train" {
        if args.is_empty() {
            return Err(format!("{name}: sample count required"));
        }
        let q = size(matrix(e, &args[0])?.as_scalar()?, true)?;
        if q.saturating_mul(64) > e.limits.memory_bytes {
            return Err("division: workspace memory limit exceeded".into());
        }
        let d = if name == "dividetrain" {
            arity(name, args, 1)?;
            Division::All
        } else if name == "divideind" {
            arity(name, args, 4)?;
            Division::Indices([ids(e, &args[1])?, ids(e, &args[2])?, ids(e, &args[3])?])
        } else {
            if args.len() != 1 && args.len() != 4 {
                return Err(format!(
                    "{name}: expected Q[,trainRatio,valRatio,testRatio]"
                ));
            }
            let r = if args.len() == 1 {
                [0.7, 0.15, 0.15]
            } else {
                [
                    matrix(e, &args[1])?.as_scalar()?,
                    matrix(e, &args[2])?.as_scalar()?,
                    matrix(e, &args[3])?.as_scalar()?,
                ]
            };
            match name {
                "dividerand" => Division::Random(r),
                "divideblock" => Division::Block(r),
                _ => Division::Interleaved(r),
            }
        };
        let groups = d.split(q, &mut || e.random.next_f64(), &e.cancel)?;
        return groups.iter().map(|v| indices(v).map(Value::from)).collect();
    }
    if args
        .first()
        .and_then(|v| super::super::objects::nnet_value(e, v))
        .is_some_and(|o| o.som.is_some())
    {
        return som::train(e, args);
    }
    if args.len() < 3 {
        return Err("train: expected net,x,t[,xi,ai,ew][,Name,Value]".into());
    }
    let mut o = super::super::objects::nnet_value(e, &args[0])
        .ok_or("train: network required")?
        .as_ref()
        .clone();
    // Resolve unsupported trainers before consuming the global stream.
    let train_fcn = text(&o.props["trainFcn"])?;
    train::algorithm(&train_fcn)?;
    let input_bytes = args[1..3]
        .iter()
        .try_fold(0usize, |sum, value| -> Result<usize> {
            Ok(sum.saturating_add(value.clone().array(&e.cancel)?.bytes()))
        })?;
    if input_bytes > e.limits.memory_bytes / 4 {
        return Err("train: input workspace memory limit exceeded".into());
    }
    let hint_start = (3..args.len())
        .find(|&i| super::super::table_fns::text_scalar(&args[i]).is_some())
        .unwrap_or(args.len());
    let (x, t) = if o.dynamic.is_some() {
        dynamic::training_inputs(e, &mut o, &args[..hint_start.min(5)])?
    } else {
        (sequence_matrix(e, &args[1])?, sequence_matrix(e, &args[2])?)
    };
    if hint_start > 6 {
        return Err("train: too many positional inputs".into());
    }
    for v in &args[3..hint_start.min(5)] {
        if o.dynamic.is_none() && !v.clone().array(&e.cancel)?.is_empty() {
            return Err("train: static networks require empty Xi and Ai".into());
        }
    }
    let ew = if hint_start == 6 {
        Some(sequence_matrix(e, &args[5])?)
    } else {
        None
    };
    if !(args.len() - hint_start).is_multiple_of(2) {
        return Err("train: expected name-value pairs".into());
    }
    for pair in args[hint_start..].chunks(2) {
        let hint = value_text(e, &pair[0])?;
        if !matches!(hint.as_str(), "useParallel" | "useGPU" | "showResources") {
            return Err(format!("train: unsupported option '{hint}'"));
        }
        let value = value_text(e, &pair[1])?;
        if !matches!(value.as_str(), "yes" | "no" | "only") {
            return Err(format!("train: invalid value for hint '{hint}'"));
        }
    }
    let mode = text(&o.props["divideMode"])?;
    let d = match mode.as_str() {
        "none" => Division::All,
        "sample" => division(e, &text(&o.props["divideFcn"])?, &o.props["divideParam"])?,
        "time" if o.dynamic.is_some() => {
            division(e, &text(&o.props["divideFcn"])?, &o.props["divideParam"])?
        }
        _ => return Err("train: supported divideMode values are sample, time and none".into()),
    };
    let Array::Struct(params) = &o.props["trainParam"] else {
        unreachable!()
    };
    let mut parameters = train::Parameters::new();
    let mut string_parameters = BTreeMap::new();
    for key in params.field_names() {
        let value = field(&o.props["trainParam"], key)?;
        if matches!(value, Array::Character(_) | Array::Str(_)) {
            string_parameters.insert(key.clone(), text(value)?);
        } else {
            parameters.insert(key.clone(), scalar(value)?);
        }
    }
    let perform_fcn = text(&o.props["performFcn"])?;
    let regularization = scalar(field(&o.props["performParam"], "regularization")?)?;
    let normalization = text(field(&o.props["performParam"], "normalization")?)?;
    budget(e, &o.core, x.cols(), x.len() + t.len())?;
    // Clone only the tiny RNG state, then restore its consumed stream even on error.
    // This leaves Engine available to the streaming console sink during training.
    let mut random = e.random.clone();
    let time_samples = if o.dynamic.is_some() && mode == "time" {
        let streams = series::read(e, &args[1])?;
        streams
            .first()
            .and_then(|r| r.first())
            .map_or(1, Matrix::cols)
    } else {
        1
    };
    let d = if time_samples > 1 {
        let groups = d.split(
            x.cols() / time_samples,
            &mut || random.next_f64(),
            &e.cancel,
        )?;
        Division::Indices(groups.map(|v| {
            v.into_iter()
                .flat_map(|t| (0..time_samples).map(move |q| t * time_samples + q))
                .collect()
        }))
    } else {
        d
    };
    let mut rng = || random.next_f64();
    let cancel = e.cancel.clone();
    let memory_bytes = e.limits.memory_bytes;
    let output_bytes = e
        .limits
        .output_bytes
        .unwrap_or(usize::MAX)
        .saturating_sub(e.emitted_bytes);
    let mut console = |line: &str| -> Result<()> {
        e.hold_output(line)?;
        if e.hostio_streams_output() {
            let mut out = Execution {
                stream: true,
                ..Default::default()
            };
            e.emit(&mut out, String::new())?;
            if !out.output.is_empty() {
                e.pending_line.push_str(&out.output.join("\n"));
            }
        } else {
            e.emitted_entries += 1;
        }
        Ok(())
    };
    let opts = train::TrainOptions {
        train_fcn,
        perform_fcn,
        parameters,
        string_parameters,
        division: d,
        error_weights: ew,
        regularization,
        normalization,
        cancel,
        memory_bytes,
        output_bytes,
        command_line: Some(&mut console),
        uniform: Some(&mut rng),
    };
    let result = train::train(
        &o.core,
        &x,
        &t,
        opts,
        &mut super::super::neural_app_hooks::report,
    );
    e.random = random;
    let (net, mut tr) = result?;
    if time_samples > 1 {
        for ids in [&mut tr.train_ind, &mut tr.val_ind, &mut tr.test_ind] {
            for i in ids.iter_mut() {
                *i /= time_samples;
            }
            ids.dedup();
        }
    }
    o.core = net;
    let record = training_record(&o, &tr, &e.cancel)?;
    Ok(vec![
        super::super::objects::nnet_new(e, o)?,
        Value::from(record),
    ])
}
fn training_record(o: &Object, tr: &train::TrainingRecord, c: &AtomicBool) -> Result<Array> {
    let mut fields = vec![
        ("trainFcn", o.props["trainFcn"].clone()),
        ("trainParam", o.props["trainParam"].clone()),
        ("performFcn", o.props["performFcn"].clone()),
        ("performParam", o.props["performParam"].clone()),
        ("divideFcn", o.props["divideFcn"].clone()),
        ("divideParam", o.props["divideParam"].clone()),
        ("divideMode", o.props["divideMode"].clone()),
        ("trainInd", indices(&tr.train_ind)?),
        ("valInd", indices(&tr.val_ind)?),
        ("testInd", indices(&tr.test_ind)?),
        ("stop", string(&tr.stop, c)?),
        ("num_epochs", num(tr.num_epochs as f64)),
        ("best_epoch", num(tr.best_epoch as f64)),
        ("best_perf", num(tr.best_perf)),
        ("best_vperf", num(tr.best_vperf)),
        ("best_tperf", num(tr.best_tperf)),
    ];
    type State = (&'static str, fn(&train::EpochReport) -> f64);
    let states: [State; 7] = [
        ("epoch", |r| r.epoch as f64),
        ("time", |r| r.time),
        ("perf", |r| r.perf),
        ("vperf", |r| r.vperf),
        ("tperf", |r| r.tperf),
        ("gradient", |r| r.gradient),
        ("val_fail", |r| r.val_fail as f64),
    ];
    for (key, get) in &states {
        // The plot record contract accepts empty optional series. Keep absent
        // divisions empty so plotperform does not select a NaN validation best.
        let absent = (*key == "vperf" && tr.val_ind.is_empty())
            || (*key == "tperf" && tr.test_ind.is_empty());
        fields.push((
            key,
            row(if absent {
                Vec::new()
            } else {
                tr.epochs.iter().map(get).collect()
            })?,
        ));
    }
    // Extra state names are owned by the algorithm; build the struct dynamically.
    let mut names: Vec<String> = fields.iter().map(|(k, _)| (*k).into()).collect();
    let mut values: Vec<Arc<Array>> = fields.into_iter().map(|(_, v)| Arc::new(v)).collect();
    if let Some(first) = tr.epochs.first() {
        for key in first.extra.keys() {
            names.push(key.clone());
            values.push(Arc::new(row(tr
                .epochs
                .iter()
                .map(|r| r.extra[key])
                .collect())?));
        }
    }
    Ok(Array::Struct(StructArray::from_slots(
        1, 1, names, values, c,
    )?))
}
