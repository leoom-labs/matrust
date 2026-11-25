//! Parallel Computing Toolbox names, run serially: MatRust has one worker, the
//! client itself. `parfor` runs as `for`, `spmd` runs its body once (both in
//! syntax.rs), a pool reports NumWorkers 1, and parfeval/batch run at once and
//! return a finished future. gpuArray and gather return their input.
use super::{Engine, Execution, Value};
use crate::function_handle::FunctionHandle;
use crate::{Array, CellArray, LogicalMatrix, Matrix, Result};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum ParKind {
    Pool { threads: bool },
    Future,
    Job,
}

#[derive(Clone)]
pub(super) struct ParObj {
    pub kind: ParKind,
    pub id: u64,
    pub function: String,
    pub outputs: Vec<Arc<Array>>,
    /// The MException of a failed evaluation and its message.
    pub error: Option<(Arc<Array>, String)>,
    pub read: bool,
}

/// The open pool, kept so gcp returns the same object.
#[derive(Clone, Default)]
pub(super) struct ParState {
    pool: Option<Value>,
    next_id: u64,
}

impl ParState {
    pub(super) fn roots(&self) -> impl Iterator<Item = &Array> {
        self.pool.iter().filter_map(|v| match v {
            Value::Container(a) => Some(a),
            _ => None,
        })
    }
}

const NAMES: &[&str] = &[
    "parpool",
    "gcp",
    "parfeval",
    "batch",
    "fetchOutputs",
    "fetchNext",
    "gpuArray",
    "gpuDeviceCount",
    "gpuDevice",
    "canUseGPU",
    "gather",
    "spmdIndex",
    "spmdSize",
    "labindex",
    "numlabs",
];

pub(super) fn handles(name: &str) -> bool {
    NAMES.contains(&name)
}

/// Methods of futures and jobs that share a name with other builtins.
pub(super) fn method(e: &Engine, name: &str, args: &[Value]) -> bool {
    matches!(name, "wait" | "cancel" | "load") && args.first().and_then(|v| e.par_ids(v)).is_some()
}

impl ParObj {
    pub fn class(&self) -> &'static str {
        match self.kind {
            ParKind::Pool { threads: true } => "parallel.ThreadPool",
            ParKind::Pool { threads: false } => "parallel.ProcessPool",
            ParKind::Future => "parallel.FevalFuture",
            ParKind::Job => "parallel.job.CJSIndependentJob",
        }
    }
    pub fn body(&self) -> String {
        match self.kind {
            ParKind::Pool { .. } => format!(
                "  {} with properties:\n\n{:>16}: 1\n{:>16}: true\n{:>16}: false\n\
                 {:>16}: 30\n  (runs serially in MatRust)\n",
                self.class().trim_start_matches("parallel."),
                "NumWorkers",
                "Connected",
                "Busy",
                "IdleTimeout"
            ),
            _ => format!(
                "  {} with properties:\n\n{:>14}: {}\n{:>14}: finished\n{:>14}: {}\n{:>14}: {}\n",
                self.class().trim_start_matches("parallel."),
                "ID",
                self.id,
                "State",
                "Function",
                self.function,
                "Error",
                if self.error.is_some() {
                    "MException"
                } else {
                    "none"
                }
            ),
        }
    }
    pub fn property(&self, e: &Engine, name: &str) -> Result<Value> {
        let number = |x: f64| Ok(Value::Matrix(Matrix::scalar(x)));
        let logical = |b: bool| Ok(Value::Logical(LogicalMatrix::scalar(b)));
        match (&self.kind, name) {
            (ParKind::Pool { .. }, "NumWorkers") => number(1.),
            (ParKind::Pool { .. }, "Connected") => logical(true),
            (ParKind::Pool { .. }, "Busy") => logical(false),
            (ParKind::Pool { .. }, "IdleTimeout") => number(30.),
            (ParKind::Pool { .. }, "SpmdEnabled") => logical(true),
            (ParKind::Future | ParKind::Job, "ID") => number(self.id as f64),
            (ParKind::Future | ParKind::Job, "State") => Ok(Value::Text("finished".into())),
            (ParKind::Future | ParKind::Job, "Function") => Ok(Value::Text(self.function.clone())),
            (ParKind::Future, "Read") => logical(self.read),
            (ParKind::Future, "NumOutputArguments") => number(self.outputs.len() as f64),
            (ParKind::Future | ParKind::Job, "OutputArguments") => cell(e, &self.outputs),
            (ParKind::Future | ParKind::Job, "Error") => Ok(match &self.error {
                Some((err, _)) => Value::from(err.as_ref().clone()),
                None => Value::Matrix(empty()),
            }),
            (ParKind::Future | ParKind::Job, "Diary") => Ok(Value::Text(String::new())),
            _ => Err(format!(
                "No appropriate method, property, or field '{name}' for class '{}'.",
                self.class()
            )),
        }
    }
}

fn empty() -> Matrix {
    Matrix::from_column_vec(0, 0, Vec::new()).expect("empty matrix")
}

fn cell(e: &Engine, items: &[Arc<Array>]) -> Result<Value> {
    Ok(Value::Container(Array::Cell(CellArray::from_shared(
        1,
        items.len(),
        items.to_vec(),
        &e.cancel,
    )?)))
}

fn count(e: &Engine, who: &str, v: &Value) -> Result<usize> {
    let x = e.real_arg(who, v)?.as_scalar()?;
    if !(x >= 0. && x.fract() == 0. && x <= 64.) {
        return Err(format!(
            "{who}: the number of outputs must be an integer from 0 to 64"
        ));
    }
    Ok(x as usize)
}

fn handle(e: &Engine, who: &str, v: &Value) -> Result<FunctionHandle> {
    super::optim_nl::onl_handle(e, who, v)
}

fn is_pool(e: &Engine, v: &Value) -> bool {
    e.par_ids(v)
        .and_then(|ids| {
            ids.first()
                .and_then(|id| e.par_get(*id))
                .map(|(p, _)| p.kind.clone())
        })
        .is_some_and(|k| matches!(k, ParKind::Pool { .. }))
}

/// The open pool, or None after delete(pool) or before the first parpool.
fn open_pool(e: &Engine) -> Option<Value> {
    let pool = e.par.pool.clone()?;
    let ids = e.par_ids(&pool)?;
    let (_, deleted) = e.par_get(*ids.first()?)?;
    (!deleted).then_some(pool)
}

fn new_pool(e: &mut Engine, threads: bool) -> Result<Value> {
    let pool = e.par_new(ParObj {
        kind: ParKind::Pool { threads },
        id: 0,
        function: String::new(),
        outputs: Vec::new(),
        error: None,
        read: false,
    })?;
    e.par.pool = Some(pool.clone());
    Ok(pool)
}

impl Engine {
    /// Run `f` now with `nout` outputs; a failure is kept in the future, as a
    /// parallel worker would keep it, and raised by fetchOutputs.
    fn par_evaluate(
        &mut self,
        f: &FunctionHandle,
        args: Vec<Value>,
        nout: usize,
        kind: ParKind,
        out: &mut Execution,
    ) -> Result<Value> {
        let failure = self.functions.failure.clone();
        let file_diagnostic = self.file_diagnostic.clone();
        let (outputs, error) = match self.call_handle(f, args, nout, None, out) {
            Ok(values) => {
                let outputs = values
                    .into_iter()
                    .take(nout)
                    .map(|v| v.array(&self.cancel).map(Arc::new))
                    .collect::<Result<Vec<_>>>()?;
                if outputs.len() < nout {
                    return Err(format!(
                        "{}: the function returned {} of {nout} outputs",
                        f.display(),
                        outputs.len()
                    ));
                }
                (outputs, None)
            }
            Err(message) if self.is_catchable(&message) => {
                self.functions.failure = failure;
                self.file_diagnostic = file_diagnostic;
                let value = Arc::new(self.error_value(&message)?);
                (Vec::new(), Some((value, message)))
            }
            Err(message) => return Err(message),
        };
        self.par.next_id += 1;
        self.par_new(ParObj {
            kind,
            id: self.par.next_id,
            function: f.display(),
            outputs,
            error,
            read: false,
        })
    }

    fn par_fetch(&mut self, v: &Value, nargout: usize, rest: &[Value]) -> Result<Vec<Value>> {
        let ids = self
            .par_ids(v)
            .ok_or("fetchOutputs: the input must be a future or a job")?;
        let mut uniform = true;
        for pair in rest.chunks(2) {
            let key = self.text_value(pair[0].clone())?;
            if !key.eq_ignore_ascii_case("UniformOutput") || pair.len() < 2 {
                return Err(format!("fetchOutputs: unknown option '{key}'"));
            }
            uniform = self.real_arg("fetchOutputs", &pair[1])?.as_scalar()? != 0.;
        }
        let mut objects = Vec::new();
        for id in &ids {
            let (p, _) = self.par_get(*id).ok_or("fetchOutputs: invalid future")?;
            if let Some((_, message)) = &p.error {
                return Err(format!(
                    "One or more futures resulted in an error.\nCaused by:\n    {message}"
                ));
            }
            objects.push(p.clone());
        }
        for id in &ids {
            self.par_set_read(*id);
        }
        if let [p] = objects.as_slice() {
            if p.kind == ParKind::Job {
                return Ok(vec![cell(self, &p.outputs)?]);
            }
            return Ok(p
                .outputs
                .iter()
                .map(|a| Value::from(a.as_ref().clone()))
                .collect());
        }
        let n = objects.iter().map(|p| p.outputs.len()).min().unwrap_or(0);
        let want = nargout.max(1).min(n.max(1));
        let mut values = Vec::new();
        for k in 0..want.min(n) {
            let column: Vec<Value> = objects
                .iter()
                .map(|p| Value::from(p.outputs[k].as_ref().clone()))
                .collect();
            values.push(if uniform {
                self.par_vertcat(column)?
            } else {
                let items = column
                    .into_iter()
                    .map(|v| v.array(&self.cancel).map(Arc::new))
                    .collect::<Result<Vec<_>>>()?;
                Value::Container(Array::Cell(CellArray::from_shared(
                    items.len(),
                    1,
                    items,
                    &self.cancel,
                )?))
            });
        }
        Ok(values)
    }

    fn par_vertcat(&mut self, values: Vec<Value>) -> Result<Value> {
        let f = self.named_handle("vertcat")?;
        let Value::Container(Array::Handle(f)) = f else {
            return Err("vertcat is unavailable".into());
        };
        let mut out = Execution::default();
        Ok(self.call_handle(&f, values, 1, None, &mut out)?.remove(0))
    }
}

pub(super) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    nargout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    let one = |v: Value| Ok(vec![v]);
    match name {
        "gpuDeviceCount" | "gpuDevice" | "canUseGPU" => {
            if nargout > 1 {
                return Err(format!("{name}: too many output arguments"));
            }
            match name {
                "gpuDeviceCount" => {
                    match args {
                        [] => {}
                        [option] if e.text_value(option.clone())? == "available" => {}
                        _ => return Err("Use gpuDeviceCount or gpuDeviceCount('available')".into()),
                    }
                    one(Value::Matrix(Matrix::scalar(0.)))
                }
                "canUseGPU" if args.is_empty() => one(Value::Logical(LogicalMatrix::scalar(false))),
                "canUseGPU" => Err("canUseGPU takes no arguments".into()),
                _ if args.is_empty() => Err(e.raise(
                    "No supported GPU device was found.".into(),
                    "parallel:gpu:device:NoDevicesAvailable".into(),
                )),
                _ => Err("gpuDevice takes no arguments in MatRust".into()),
            }
        }
        "gpuArray" | "gather" => {
            if name == "gather" {
                return Ok(args.to_vec());
            }
            let [x] = args else {
                return Err("gpuArray requires one input".into());
            };
            one(x.clone())
        }
        "spmdIndex" | "spmdSize" | "labindex" | "numlabs" => {
            if !args.is_empty() {
                return Err(format!("{name}: too many input arguments"));
            }
            one(Value::Matrix(Matrix::scalar(1.)))
        }
        "parpool" => {
            if open_pool(e).is_some() {
                return Err(
                    "Found an interactive session. You cannot have multiple interactive \
                            sessions open simultaneously. To terminate the existing session, use \
                            delete(gcp('nocreate'))."
                        .into(),
                );
            }
            let mut threads = false;
            for a in args {
                if let Ok(text) = e.text_value(a.clone()) {
                    threads = text.eq_ignore_ascii_case("threads");
                } else {
                    e.real_arg("parpool", a)?;
                }
            }
            e.emit(
                out,
                "Starting parallel pool (parpool) ...\nConnected to parallel pool with 1 worker \
                 (runs serially in MatRust).\n"
                    .into(),
            )?;
            one(new_pool(e, threads)?)
        }
        "gcp" => {
            let nocreate = match args {
                [] => false,
                [a] => {
                    let text = e.text_value(a.clone())?;
                    if !text.eq_ignore_ascii_case("nocreate") {
                        return Err("gcp: the only option is 'nocreate'".into());
                    }
                    true
                }
                _ => return Err("gcp: too many input arguments".into()),
            };
            match open_pool(e) {
                Some(pool) => one(pool),
                None if nocreate => one(Value::Matrix(empty())),
                None => one(new_pool(e, false)?),
            }
        }
        "parfeval" => {
            let rest = match args.first() {
                Some(p) if is_pool(e, p) => &args[1..],
                _ => args,
            };
            let [f, n, inputs @ ..] = rest else {
                return Err("parfeval requires a function and the number of outputs".into());
            };
            let f = handle(e, "parfeval", f)?;
            let n = count(e, "parfeval", n)?;
            one(e.par_evaluate(&f, inputs.to_vec(), n, ParKind::Future, out)?)
        }
        "batch" => {
            let rest = match args.first() {
                Some(p) if is_pool(e, p) => &args[1..],
                _ => args,
            };
            match rest {
                [script] if !matches!(script, Value::Container(Array::Handle(_))) => {
                    // A script runs in the client workspace, so load(job) has nothing to copy.
                    let text = e.text_value(script.clone())?;
                    e.run_command(vec![Value::Text(text.clone())], out)?;
                    e.par.next_id += 1;
                    let id = e.par.next_id;
                    one(e.par_new(ParObj {
                        kind: ParKind::Job,
                        id,
                        function: text,
                        outputs: Vec::new(),
                        error: None,
                        read: false,
                    })?)
                }
                [f, n, inputs, options @ ..] if options.len() % 2 == 0 => {
                    let f = handle(e, "batch", f)?;
                    let n = count(e, "batch", n)?;
                    let inputs = match inputs {
                        Value::Container(Array::Cell(c)) => (0..c.len())
                            .map(|k| c.get_linear(k).map(|a| Value::from(a.as_ref().clone())))
                            .collect::<Result<Vec<_>>>()?,
                        _ => return Err("batch: the inputs must be a cell array".into()),
                    };
                    one(e.par_evaluate(&f, inputs, n, ParKind::Job, out)?)
                }
                [_, _] => Err("batch: the inputs must be a cell array".into()),
                _ => Err("batch requires a script name or a function, outputs and inputs".into()),
            }
        }
        "fetchOutputs" => {
            let [f, rest @ ..] = args else {
                return Err("fetchOutputs requires a future or a job".into());
            };
            e.par_fetch(f, nargout, rest)
        }
        "fetchNext" => {
            let [f, ..] = args else {
                return Err("fetchNext requires futures".into());
            };
            let ids = e.par_ids(f).ok_or("fetchNext: the input must be futures")?;
            let Some((k, id)) = ids
                .iter()
                .enumerate()
                .find(|(_, id)| e.par_get(**id).is_some_and(|(p, _)| !p.read))
            else {
                return Err("fetchNext: all futures have been read".into());
            };
            let id = *id;
            let single = e
                .par_get(id)
                .map(|(p, _)| p.clone())
                .ok_or("fetchNext: invalid future")?;
            let value = e.par_new(single)?;
            let mut values = vec![Value::Matrix(Matrix::scalar((k + 1) as f64))];
            values.extend(e.par_fetch(&value, nargout.saturating_sub(1), &[])?);
            e.par_set_read(id);
            Ok(values)
        }
        "wait" if args.len() > 1 => one(Value::Logical(LogicalMatrix::scalar(true))),
        "wait" | "cancel" => one(Value::Void),
        "load" => one(Value::Void),
        _ => Err(format!("{name}: not a parallel function")),
    }
}
