//! Shallow neural networks, built from MathWorks public function documentation.
//! Training API: `train::{train,TrainOptions,EpochReport,TrainingRecord,TrainFn}`.
//! `network::Network` is pure numerical state:
//! `forward(&self, x)` / `forward_cancel`, `activations`, `getwb` / `setwb`, and
//! public layers, IW/LW/b matrices and connection masks are the trainer API.
//! `process::{Process,apply_chain,reverse_chain}` fit and transform columns;
//! `transfer::{apply,derivative}` includes full softmax Jacobians. Matrices are
//! immutable Arc-backed column-major values. Clone shares their storage.
//! Engine glue (`Object`) supplies metadata, cancellation, budgets and the same
//! global RNG as rand. Mutations return a new object and rebind the variable.
//! Limits: 64 layers, 4 million weights, existing matrix/workspace limits.
//! SOM API: `som_train::{train, SomOptions, SomRecord}` uses the same epoch
//! report callback, cancellation and immutable matrices as supervised training.
//! Dynamic interpreter objects expand ascending taps for static backpropagation;
//! closed-loop simulation feeds outputs back in original data units.
//! Reference values: tool/refs/nnet/{core,dynamic}.py (NumPy), never MATLAB.
pub mod divide;
pub(super) mod dynamic;
mod dynamic_properties;
mod equality;
pub mod network;
mod perform;
pub mod process;
mod properties;
mod series;
mod settings;
mod som;
pub(super) mod som_plots;
pub mod som_train;
pub mod train;
mod train_glue;
pub(super) use equality::state as equality_state;
pub mod transfer;
use super::{Engine, Execution, Value};
use crate::{Array, CellArray, CharacterRow, LogicalMatrix, Matrix, Result, StructArray};
use network::Network;
pub(super) use properties::{assign, property};
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::AtomicBool},
};
pub(super) const NAMES: &[&str] = &[
    "selforgmap",
    "gridtop",
    "hextop",
    "randtop",
    "dist",
    "linkdist",
    "boxdist",
    "mandist",
    "narxnet",
    "narnet",
    "timedelaynet",
    "preparets",
    "closeloop",
    "openloop",
    "removedelay",
    "tonndata",
    "fromnndata",
    "con2seq",
    "seq2con",
    "train",
    "dividerand",
    "divideblock",
    "divideint",
    "divideind",
    "dividetrain",
    "cascadeforwardnet",
    "configure",
    "crossentropy",
    "feedforwardnet",
    "fitnet",
    "getwb",
    "hardlim",
    "ind2vec",
    "init",
    "isconfigured",
    "logsig",
    "mae",
    "mapminmax",
    "mapstd",
    "mse",
    "patternnet",
    "perform",
    "poslin",
    "purelin",
    "radbas",
    "removeconstantrows",
    "sae",
    "satlin",
    "setwb",
    "sim",
    "softmax",
    "sse",
    "tansig",
    "vec2ind",
];
pub(super) fn handles(name: &str) -> bool {
    NAMES.contains(&name) && name != "sim"
}
pub(super) fn constructor(name: &str) -> bool {
    matches!(
        name,
        "feedforwardnet"
            | "fitnet"
            | "patternnet"
            | "cascadeforwardnet"
            | "selforgmap"
            | "narxnet"
            | "narnet"
            | "timedelaynet"
    )
}
#[derive(Clone)]
pub(super) struct Object {
    pub core: Network,
    pub som: Option<som::Som>,
    pub dynamic: Option<dynamic::Dynamic>,
    pub props: BTreeMap<String, Array>,
}
impl Object {
    pub fn bytes(&self) -> usize {
        self.core.num_weight_elements() * 8
            + self.som.as_ref().map_or(0, |s| s.bytes())
            + self
                .dynamic
                .as_ref()
                .map_or(0, |d| (d.input_delays.len() + d.feedback_delays.len()) * 8)
            + self.props.values().map(Array::bytes).sum::<usize>()
            + self
                .core
                .input
                .process_settings
                .iter()
                .chain(&self.core.output.process_settings)
                .map(|p| p.xrows * 48)
                .sum::<usize>()
            + self
                .core
                .layers
                .iter()
                .map(|l| l.name.len() + l.transfer_fcn.len() + l.init_fcn.len() + 64)
                .sum::<usize>()
            + [&self.core.input, &self.core.output]
                .iter()
                .map(|p| {
                    p.range.len() * 16
                        + p.process_params.len() * 16
                        + p.process_fcns.iter().map(String::len).sum::<usize>()
                })
                .sum::<usize>()
            + 4096
    }
    pub fn body(&self) -> String {
        let n = self.core.layers.len();
        let inputs = self.dynamic.as_ref().map_or(1, dynamic::Dynamic::inputs);
        format!(
            "  Neural Network\n\n              name: '{}'\n\n  dimensions:\n         numInputs: {inputs}\n         numLayers: {n}\n        numOutputs: 1\n numWeightElements: {}\n\n  connections:\n       biasConnect: [{}x1 boolean]\n      inputConnect: [{n}x{inputs} boolean]\n      layerConnect: [{n}x{n} boolean]\n     outputConnect: [1x{n} boolean]\n\n  subobjects:\n            inputs: {{1x{inputs} cell}}\n            layers: {{{n}x1 cell}}\n           outputs: {{1x{n} cell}}\n\n  functions:\n          trainFcn: '{}'\n        performFcn: '{}'\n         divideFcn: '{}'\n           initFcn: 'initlay'\n\n  parameters:\n        trainParam: [1x1 struct]\n      performParam: [1x1 struct]\n       divideParam: [1x1 struct]\n\n  weight and bias values:\n                IW: {{{n}x{inputs} cell}}\n                LW: {{{n}x{n} cell}}\n                 b: {{{n}x1 cell}}\n\n  other:\n          userdata: [user data]\n",
            text(&self.props["name"]).unwrap_or_default(),
            self.core.num_weight_elements(),
            n,
            text(&self.props["trainFcn"]).unwrap_or_default(),
            text(&self.props["performFcn"]).unwrap_or_default(),
            text(&self.props["divideFcn"]).unwrap_or_default()
        )
    }
}
pub(super) fn call(e: &mut Engine, name: &str, args: &[Value], nout: usize) -> Result<Vec<Value>> {
    e.check_cancel()?;
    let mut result = if som::handles(name) {
        som::call(e, name, args)?
    } else if dynamic::handles(name) {
        dynamic::call(e, name, args)?
    } else if series::NAMES.contains(&name) {
        series::call(e, name, args)?
    } else if constructor(name) {
        if args.len() > if name == "patternnet" { 3 } else { 2 } {
            return Err(format!("{name}: too many input arguments"));
        }
        let hidden = if let Some(v) = args.first() {
            let m = matrix(e, v)?;
            if m.rows() > 1 {
                return Err(format!("{name}: hiddenSizes must be a row vector"));
            }
            m.data()
                .iter()
                .map(|&x| size(x, false))
                .collect::<Result<Vec<_>>>()?
        } else {
            vec![10]
        };
        let train = args
            .get(1)
            .map(|v| value_text(e, v))
            .transpose()?
            .unwrap_or_else(|| {
                if name == "patternnet" {
                    "trainscg"
                } else {
                    "trainlm"
                }
                .into()
            });
        let perf = args
            .get(2)
            .map(|v| value_text(e, v))
            .transpose()?
            .unwrap_or_else(|| {
                if name == "patternnet" {
                    "crossentropy"
                } else {
                    "mse"
                }
                .into()
            });
        let o = properties::new_object(name, &hidden, &train, &perf, &e.cancel)?;
        vec![super::objects::nnet_new(e, o)?]
    } else if train_glue::handles(name) {
        train_glue::call(e, name, args)?
    } else if process::NAMES.contains(&name) {
        settings::call(e, name, args)?
    } else if transfer::NAMES.contains(&name) {
        vec![transfer_call(e, name, args)?]
    } else if matches!(
        name,
        "mse" | "mae" | "sse" | "sae" | "crossentropy" | "perform"
    ) {
        vec![perform::call(e, name, args)?]
    } else if matches!(name, "vec2ind" | "ind2vec") {
        vec![indices(e, name, args)?]
    } else {
        let mut o = super::objects::nnet_value(
            e,
            args.first()
                .ok_or_else(|| format!("{name}: network required"))?,
        )
        .ok_or_else(|| format!("{name}: network required"))?
        .as_ref()
        .clone();
        if name == "configure" && o.som.is_some() {
            return som::configure_call(e, o, args);
        }
        if name == "configure" && o.dynamic.is_some() {
            return dynamic::configure_call(e, o, args);
        }
        let value = match name {
            "configure" => {
                if args.len() != 3 {
                    return Err("configure: expected net,x,t".into());
                }
                let x = sequence_matrix(e, &args[1])?;
                let t = sequence_matrix(e, &args[2])?;
                budget(e, &o.core, x.cols(), x.len() + t.len())?;
                let mut dimensions = o.core.clone();
                dimensions.input.size = x.rows();
                dimensions.input.process_settings.clear();
                dimensions.layers.last_mut().unwrap().size = t.rows();
                let upper = dimensions.check_size()?;
                if upper.saturating_mul(32) > e.limits.memory_bytes {
                    return Err("configure: workspace memory limit exceeded".into());
                }
                o.core.configure(&x, &t, &e.cancel)?;
                budget(e, &o.core, 0, o.core.num_weight_elements() * 3)?;
                o.core.init(|| e.random.next_f64(), &e.cancel)?;
                super::objects::nnet_new(e, o)?
            }
            "init" => {
                arity(name, args, 1)?;
                if o.som.is_some() {
                    som::initialize(e, &mut o)?;
                } else {
                    o.core.init(|| e.random.next_f64(), &e.cancel)?;
                }
                super::objects::nnet_new(e, o)?
            }
            "getwb" => {
                arity(name, args, 1)?;
                Value::Matrix(Matrix::from_column_vec(
                    o.core.num_weight_elements(),
                    1,
                    o.core.getwb(),
                )?)
            }
            "setwb" => {
                arity(name, args, 2)?;
                let w = matrix(e, &args[1])?;
                if w.rows() > 1 && w.cols() > 1 {
                    return Err("setwb: expected a vector".into());
                }
                o.core.setwb(w.data())?;
                super::objects::nnet_new(e, o)?
            }
            "sim" => {
                arity(name, args, 2)?;
                simulate(e, &o, &args[1])?
            }
            "isconfigured" => {
                arity(name, args, 1)?;
                Value::Logical(LogicalMatrix::scalar(o.core.configured))
            }
            _ => return Err(format!("{name}: unsupported syntax")),
        };
        vec![value]
    };
    if nout > result.len() {
        return Err("Too many output arguments.".into());
    }
    if nout > 0 {
        result.truncate(nout);
    }
    e.charge_function_outputs(&result)?;
    Ok(result)
}
pub(super) fn simulate(e: &mut Engine, o: &Object, x: &Value) -> Result<Value> {
    if o.som.is_some() {
        return som::simulate(e, o, x);
    }
    if o.dynamic.is_some() {
        return Ok(dynamic::simulate(e, o, std::slice::from_ref(x))?.remove(0));
    }
    if let Value::Container(Array::Cell(c)) = x {
        let mut values = Vec::with_capacity(c.len());
        for i in 0..c.len() {
            let x = Value::from(c.get_linear(i)?.as_ref().clone());
            values.push(Arc::new(simulate(e, o, &x)?.array(&e.cancel)?));
        }
        return Ok(Value::from(Array::Cell(CellArray::from_shared(
            c.rows(),
            c.cols(),
            values,
            &e.cancel,
        )?)));
    }
    let x = matrix(e, x)?;
    budget(e, &o.core, x.cols(), 0)?;
    Ok(Value::Matrix(o.core.forward_cancel(&x, &e.cancel)?))
}
fn budget(e: &mut Engine, n: &Network, q: usize, extra: usize) -> Result<()> {
    let units = n
        .layers
        .iter()
        .map(|l| l.size.saturating_mul(q))
        .fold(extra, usize::saturating_add)
        .saturating_mul(4)
        .saturating_add(n.num_weight_elements());
    if units > e.limits.memory_bytes / 8 {
        return Err("network: workspace memory limit exceeded".into());
    }
    e.charge(units)
}
fn transfer_call(e: &Engine, name: &str, args: &[Value]) -> Result<Value> {
    if args
        .first()
        .and_then(super::table_fns::text_scalar)
        .as_deref()
        == Some("dn")
    {
        if !(3..=4).contains(&args.len()) {
            return Err(format!("{name}: derivative expects 'dn',N,A[,FP]"));
        }
        if args.len() == 4 {
            let a = args[3].clone().array(&e.cancel)?;
            if !matches!(a,Array::Struct(ref s) if s.field_count()==0) && !a.is_empty() {
                return Err(format!("{name}: no function parameters supported"));
            }
        }
        return Ok(Value::Matrix(transfer::derivative(
            name,
            &matrix(e, &args[1])?,
            &matrix(e, &args[2])?,
            &e.cancel,
        )?));
    }
    arity(name, args, 1)?;
    Ok(Value::Matrix(transfer::apply(
        name,
        &matrix(e, &args[0])?,
        &e.cancel,
    )?))
}
fn indices(e: &Engine, name: &str, args: &[Value]) -> Result<Value> {
    if name == "vec2ind" {
        arity(name, args, 1)?;
        if let Value::Container(Array::Cell(c)) = &args[0] {
            let values = (0..c.len())
                .map(|i| {
                    Ok(Arc::new(
                        indices(e, name, &[Value::from(c.get_linear(i)?.as_ref().clone())])?
                            .array(&e.cancel)?,
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            return Ok(Value::from(Array::Cell(CellArray::from_shared(
                c.rows(),
                c.cols(),
                values,
                &e.cancel,
            )?)));
        }
        let m = matrix(e, &args[0])?;
        if m.rows() == 0 {
            return Err("vec2ind: vectors must be nonempty".into());
        }
        let v = m
            .data()
            .chunks(m.rows())
            .map(|col| {
                let mut k = 0;
                for i in 1..col.len() {
                    if col[i] > col[k] {
                        k = i;
                    }
                }
                (k + 1) as f64
            })
            .collect();
        Ok(Value::Matrix(Matrix::from_column_vec(1, m.cols(), v)?))
    } else {
        if !(1..=2).contains(&args.len()) {
            return Err("ind2vec: expected indices[,rows]".into());
        }
        let m = matrix(e, &args[0])?;
        if m.rows() != 1 {
            return Err("ind2vec: indices must be a row vector".into());
        }
        let ids = m
            .data()
            .iter()
            .map(|&v| size(v, false))
            .collect::<Result<Vec<_>>>()?;
        let max = ids.iter().copied().max().unwrap_or(0);
        let rows = if args.len() == 2 {
            size(matrix(e, &args[1])?.as_scalar()?, true)?
        } else {
            max
        };
        if rows < max {
            return Err("ind2vec: row count is less than maximum index".into());
        }
        let triplets: Vec<_> = ids
            .iter()
            .enumerate()
            .map(|(c, &r)| (r - 1, c, 1.))
            .collect();
        Ok(Value::from(Array::Sparse(
            crate::sparse::SparseMatrix::from_triplets(
                rows,
                m.cols(),
                &triplets,
                false,
                &e.cancel,
            )?,
        )))
    }
}
fn sequence_matrix(e: &Engine, v: &Value) -> Result<Matrix> {
    if let Value::Container(Array::Cell(c)) = v {
        let mut rows = None;
        let mut cols = 0;
        let mut data = Vec::new();
        for i in 0..c.len() {
            let m = matrix(e, &Value::from(c.get_linear(i)?.as_ref().clone()))?;
            if rows.is_some_and(|r| r != m.rows()) {
                return Err("network: sequence row sizes must agree".into());
            }
            rows = Some(m.rows());
            cols += m.cols();
            Matrix::checked_len(m.rows(), cols)?;
            data.extend_from_slice(m.data());
        }
        Matrix::from_column_vec(rows.unwrap_or(0), cols, data)
    } else {
        matrix(e, v)
    }
}
fn matrix(e: &Engine, v: &Value) -> Result<Matrix> {
    if v.is_character() || matches!(v, Value::QuotedString(_)) {
        return Err("network: expected a real numeric matrix".into());
    }
    let m = v.clone().matrix(&e.cancel)?;
    if m.is_nd() {
        return Err("network: expected a real two-dimensional matrix".into());
    }
    Ok(m)
}
fn arity(name: &str, args: &[Value], n: usize) -> Result<()> {
    if args.len() != n {
        Err(format!("{name}: expected {n} input arguments"))
    } else {
        Ok(())
    }
}
fn size(v: f64, zero: bool) -> Result<usize> {
    if !v.is_finite() || v.fract() != 0. || v < f64::from(!zero) || v > network::MAX_WEIGHTS as f64
    {
        Err("network: invalid size or dimension limit exceeded".into())
    } else {
        Ok(v as usize)
    }
}
fn text(a: &Array) -> Result<String> {
    super::table_fns::text_scalar(&Value::from(a.clone()))
        .ok_or_else(|| "network: expected text scalar".into())
}
fn value_text(e: &Engine, v: &Value) -> Result<String> {
    text(&v.clone().array(&e.cancel)?)
}
fn num(x: f64) -> Array {
    Array::Numeric(Matrix::scalar(x))
}
fn string(s: &str, c: &AtomicBool) -> Result<Array> {
    Ok(Array::Character(CharacterRow::from_text(s, c)?))
}
fn empty() -> Result<Array> {
    Ok(Array::Numeric(Matrix::filled(0, 0, 0.)?))
}
fn record(fields: Vec<(&str, Array)>, c: &AtomicBool) -> Result<Array> {
    let (names, values) = fields
        .into_iter()
        .map(|(k, v)| (k.to_owned(), Arc::new(v)))
        .unzip();
    Ok(Array::Struct(StructArray::from_slots(
        1, 1, names, values, c,
    )?))
}
fn field<'a>(a: &'a Array, name: &str) -> Result<&'a Array> {
    let Array::Struct(s) = a else {
        return Err("network: expected scalar struct".into());
    };
    if s.len() != 1 {
        return Err("network: expected scalar struct".into());
    }
    Ok(s.get(
        s.field_index(name)
            .ok_or_else(|| format!("Unrecognized field name '{name}'."))?,
        0,
    )?
    .as_ref())
}
fn scalar(a: &Array) -> Result<f64> {
    match a {
        Array::Numeric(m) => m.as_scalar(),
        Array::Logical(m) if m.len() == 1 => Ok(m.data()[0] as f64),
        _ => Err("network: expected real scalar".into()),
    }
}
fn cell(rows: usize, cols: usize, v: Vec<Array>, c: &AtomicBool) -> Result<Array> {
    // Collect may reuse the much wider Vec<Array> allocation for Arc<Array>,
    // retaining a capacity above the cell slot limit even for an admitted series.
    // Allocate exactly the slot count; Matrix payloads remain shared.
    let mut slots = Vec::with_capacity(v.len());
    for (i, value) in v.into_iter().enumerate() {
        if i.is_multiple_of(1024) {
            crate::dsp::check_cancel(c)?;
        }
        slots.push(Arc::new(value));
    }
    Ok(Array::Cell(CellArray::from_shared(rows, cols, slots, c)?))
}
fn strings(v: &[String], c: &AtomicBool) -> Result<Array> {
    cell(
        1,
        v.len(),
        v.iter().map(|s| string(s, c)).collect::<Result<_>>()?,
        c,
    )
}
fn text_list(a: &Array) -> Result<Vec<String>> {
    let Array::Cell(c) = a else {
        return Err("network: expected cell array of function names".into());
    };
    (0..c.len())
        .map(|i| text(c.get_linear(i)?.as_ref()))
        .collect()
}
fn matrices(a: &Array, r: usize, c: usize) -> Result<Vec<Matrix>> {
    let Array::Cell(v) = a else {
        return Err("network: expected cell array".into());
    };
    if v.rows() != r || v.cols() != c {
        return Err(format!("network: expected {r}-by-{c} cell array"));
    }
    (0..v.len())
        .map(|i| match v.get_linear(i)?.as_ref() {
            Array::Numeric(m) if !m.is_nd() => Ok(m.clone()),
            _ => Err("network: weights must be real matrices".into()),
        })
        .collect()
}

/// Keep value objects out of inputParser's handle-only method path.
pub(super) fn object_method(
    e: &Engine,
    n: &Object,
    name: &str,
    args: &[Value],
) -> Result<Option<Value>> {
    let properties = || {
        let mut names: Vec<String> = n.props.keys().cloned().collect();
        names.extend(
            [
                "numInputs",
                "numLayers",
                "numOutputs",
                "numWeightElements",
                "numInputDelays",
                "numLayerDelays",
                "inputWeights",
                "inputs",
                "outputs",
                "layers",
                "biasConnect",
                "inputConnect",
                "layerConnect",
                "outputConnect",
                "IW",
                "LW",
                "b",
            ]
            .into_iter()
            .map(str::to_owned),
        );
        names.sort();
        names
    };
    Ok(Some(match name {
        "isa" => {
            arity(name, args, 1)?;
            Value::Logical(LogicalMatrix::scalar(value_text(e, &args[0])? == "network"))
        }
        "properties" | "fieldnames" => {
            arity(name, args, 0)?;
            let names = properties();
            Value::from(cell(
                names.len(),
                1,
                names
                    .iter()
                    .map(|s| string(s, &e.cancel))
                    .collect::<Result<_>>()?,
                &e.cancel,
            )?)
        }
        "isprop" => {
            arity(name, args, 1)?;
            Value::Logical(LogicalMatrix::scalar(
                properties().contains(&value_text(e, &args[0])?),
            ))
        }
        "delete" | "parse" | "addRequired" | "addOptional" | "addParameter" | "addParamValue" => {
            return Err(format!(
                "{name}: operation is not supported for network value objects"
            ));
        }
        _ => return Ok(None),
    }))
}
