//! Function handles: creation, calls, func2str/str2func/feval, arrayfun/cellfun.
//! Handles are immutable `Array::Handle` values; anonymous handles snapshot the
//! workspace variables their body references when they are created.
use super::{Engine, Execution, Value};
use crate::{
    Array, CellArray, CharacterRow, Complex64, ComplexMatrix, FunctionHandle, LogicalMatrix,
    Matrix, Result, StructArray,
    function_handle::MAX_CAPTURE_DEPTH,
    syntax::{self, AnonymousFunction, Expr},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

const MAX_EVALUATION_DEPTH: usize = 128;
const MAX_RETAINED_BYTES: usize = 256 * 1024 * 1024;
const HANDLE_NAMES: &[&str] = &["func2str", "str2func", "feval", "arrayfun", "cellfun"];
const LEGACY_CELLFUN: &[&str] = &[
    "isempty",
    "length",
    "numel",
    "ndims",
    "prodofsize",
    "isreal",
    "islogical",
];

/// Variable names an anonymous body reads from the enclosing workspace.
fn free_names(expr: &Expr, bound: &mut Vec<String>, names: &mut BTreeSet<String>) {
    let mut mention = |name: &String, bound: &[String]| {
        if name != "end" && !bound.contains(name) {
            names.insert(name.clone());
        }
    };
    match expr {
        Expr::Number(_)
        | Expr::Imaginary(_)
        | Expr::String(_)
        | Expr::DoubleQuoted(_)
        | Expr::All
        | Expr::FunctionHandle(_) => {}
        Expr::Name(name) => mention(name, bound),
        Expr::Unary(_, e) | Expr::Not(e) | Expr::Transpose(e) | Expr::NonConjugateTranspose(e) => {
            free_names(e, bound, names)
        }
        Expr::Binary(_, a, b) => {
            free_names(a, bound, names);
            free_names(b, bound, names);
        }
        Expr::Range(a, step, b) => {
            free_names(a, bound, names);
            if let Some(step) = step {
                free_names(step, bound, names);
            }
            free_names(b, bound, names);
        }
        Expr::Call(name, args) | Expr::CellContents(name, args) => {
            mention(name, bound);
            for arg in args {
                free_names(arg, bound, names);
            }
        }
        Expr::Field(base, _) => free_names(base, bound, names),
        Expr::DynamicField(base, field) => {
            free_names(base, bound, names);
            free_names(field, bound, names);
        }
        Expr::Apply(base, args) | Expr::Index(base, args) | Expr::Brace(base, args) => {
            free_names(base, bound, names);
            for arg in args {
                free_names(arg, bound, names);
            }
        }
        Expr::Matrix(rows) | Expr::CellLiteral(rows) => {
            for e in rows.iter().flatten() {
                free_names(e, bound, names);
            }
        }
        Expr::Anonymous(inner) => {
            let outer = bound.len();
            bound.extend(inner.params.iter().cloned());
            free_names(&inner.body, bound, names);
            bound.truncate(outer);
        }
    }
}

/// What `arrayfun`/`cellfun`/`feval` call for every element.
enum Callee {
    Handle(FunctionHandle),
    Name(String),
}

/// Uniform results of one output, built element by element.
enum Column {
    Real(Vec<f64>),
    Logical(Vec<u8>),
    Complex(Vec<Complex64>),
    Char(Vec<u16>),
    /// The contents of 1x1 cell results, collected into one cell array.
    Cell(Vec<Arc<Array>>),
    Struct {
        values: Vec<StructArray>,
        retained_bytes: usize,
        nodes: usize,
    },
}
impl Column {
    fn class(&self) -> &'static str {
        match self {
            Self::Real(_) | Self::Complex(_) => "double",
            Self::Logical(_) => "logical",
            Self::Char(_) => "char",
            Self::Cell(_) => "cell",
            Self::Struct { .. } => "struct",
        }
    }
}

/// MATLAB words every wrong argument count alike. The builtin arity helpers say
/// `<name> expects 1 or 2 arguments`; recover the direction from the count given.
fn arity_message(name: &str, message: &str, given: usize) -> Option<&'static str> {
    let list = message
        .strip_prefix(name)?
        .strip_prefix(" expects ")?
        .strip_suffix(" arguments")?;
    let allowed = list
        .split(" or ")
        .map(|count| count.parse::<usize>().ok())
        .collect::<Option<Vec<_>>>()?;
    if allowed.iter().all(|&count| given > count) {
        Some("Too many input arguments.")
    } else if allowed.iter().all(|&count| given < count) {
        Some("Not enough input arguments.")
    } else {
        None
    }
}

fn text_of(value: &Value) -> Option<String> {
    match value {
        Value::Text(s) => Some(s.clone()),
        Value::Container(Array::Character(c)) if c.is_text_row() => {
            c.to_utf8(&std::sync::atomic::AtomicBool::new(false)).ok()
        }
        _ => None,
    }
}

impl Engine {
    /// `@name`: no lookup until the handle is called.
    pub(super) fn named_handle(&self, name: &str) -> Result<Value> {
        Self::validate_handle_name(name)?;
        Ok(Value::Container(FunctionHandle::named(name).into()))
    }
    fn validate_handle_name(name: &str) -> Result<()> {
        if crate::files::valid_name(name) {
            Ok(())
        } else {
            Err(format!("Invalid function handle name '{name}'"))
        }
    }
    /// `@(params) body`: snapshot referenced variables unless `capture` is off.
    pub(super) fn anonymous_handle(
        &mut self,
        function: &Arc<AnonymousFunction>,
        capture: bool,
    ) -> Result<Value> {
        let mut captured = Vec::new();
        if capture {
            let mut names = BTreeSet::new();
            free_names(&function.body, &mut function.params.clone(), &mut names);
            for name in names {
                self.check_cancel()?;
                if let Some(array) = self.array(&name) {
                    captured.push((name, array.without_coordinate_facts(&self.cancel)?));
                }
            }
        }
        let handle = FunctionHandle::anonymous(function.clone(), captured);
        if handle.depth() > MAX_CAPTURE_DEPTH {
            return Err(format!(
                "Function handles capturing function handles are limited to {MAX_CAPTURE_DEPTH} levels"
            ));
        }
        if handle.bytes() > MAX_RETAINED_BYTES {
            return Err("Function handle captures more than 256 MiB of variables".into());
        }
        Ok(Value::Container(handle.into()))
    }
    /// `base(args)` where `base` is itself a call: only handle values apply.
    pub(super) fn apply_value(
        &mut self,
        base: &Expr,
        args: &[Expr],
        end: Option<usize>,
        out: &mut Execution,
    ) -> Result<Value> {
        match self.eval(base, end, out)? {
            Value::Container(Array::Handle(handle)) => {
                self.call_handle_expression(&handle, args, out)
            }
            _ => Err(
                "Indexing the result of a call is not supported yet; assign it to a variable first"
                    .into(),
            ),
        }
    }
    pub(super) fn handle_variable(&self, name: &str) -> Option<FunctionHandle> {
        match self.container_variables.get(name) {
            Some(Array::Handle(handle)) => Some(handle.clone()),
            _ => None,
        }
    }
    /// Call `handle(args)` expression-style: one value, or `Void` for no value.
    pub(super) fn call_handle_expression(
        &mut self,
        handle: &FunctionHandle,
        args: &[Expr],
        out: &mut Execution,
    ) -> Result<Value> {
        let values = self.function_arguments(args, None, out)?;
        let mut results = self.call_handle(handle, values, 1, None, out)?;
        Ok(if results.is_empty() {
            Value::Void
        } else {
            results.remove(0)
        })
    }
    /// Standalone statement `h(args)` or `feval(...)`: the target runs with
    /// nargout=0 like a direct `f(args)`; its first value, if any, becomes `ans`.
    pub(super) fn handle_statement(
        &mut self,
        expression: &Expr,
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        if let Expr::Index(base, args) = expression {
            if let Some(value) = self.obj_eval(expression, out)? {
                return Ok(Some(value));
            }
            return self
                .obj_index_expression(base, args, None, out, true)
                .map(Some);
        }
        if let Expr::Apply(base, args) = expression {
            let base = self.eval(base, None, out)?;
            return Ok(Some(match base {
                Value::Container(Array::Handle(handle)) => {
                    let values = self.function_arguments(args, None, out)?;
                    self.call_handle(&handle, values, 0, None, out)?
                        .into_iter()
                        .next()
                        .unwrap_or(Value::Void)
                }
                _ => return Err("Indexing the result of a call is not supported yet; assign it to a variable first".into()),
            }));
        }
        let Expr::Call(name, args) = expression else {
            return Ok(None);
        };
        let results = if let Some(handle) = self.handle_variable(name) {
            let values = self.function_arguments(args, None, out)?;
            self.call_handle(&handle, values, 0, None, out)?
        } else if name == "feval"
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = self.function_arguments(args, None, out)?;
            if let Some(value) = self.lg3_feval(&values, out)? {
                return Ok(Some(value));
            }
            self.handle_builtin(name, &values, 0, None, out)?
        } else {
            return Ok(None);
        };
        Ok(Some(results.into_iter().next().unwrap_or(Value::Void)))
    }
    /// Invoke a handle. Counts against the evaluation depth and, through the
    /// target (builtin, user function or anonymous body), the usual budgets.
    pub(super) fn call_handle(
        &mut self,
        handle: &FunctionHandle,
        args: Vec<Value>,
        requested: usize,
        targets: Option<&[Option<String>]>,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        self.check_cancel()?;
        if self.evaluation_depth >= MAX_EVALUATION_DEPTH {
            return Err("Combined expression/function evaluation exceeds 128 levels".into());
        }
        self.evaluation_depth += 1;
        let result = match handle.anonymous_parts() {
            Some((function, captured)) => {
                self.call_anonymous(function, captured, args, requested, targets, out)
            }
            None => self.call_named(handle.func2str(), args, requested, targets, out),
        };
        self.evaluation_depth -= 1;
        let values = result?;
        self.charge_function_outputs(&values)?;
        Ok(values)
    }
    fn call_named(
        &mut self,
        name: &str,
        args: Vec<Value>,
        requested: usize,
        targets: Option<&[Option<String>]>,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        if let Some(function) = self.resolve_function(name)? {
            if args.len() > function.input_count() {
                return Err("Too many input arguments.".into());
            }
            // A function without outputs is called for effect, as with nargout=0.
            let requested = if requested == 1 && function.output_count() == 0 {
                0
            } else {
                requested
            };
            return self.invoke_function(function, args, requested, targets, out);
        }
        if name == "hist" && requested == 0 {
            return self.statsx_outputs(name, &args, requested);
        }
        if super::lang_utils::multiple_outputs(name) && requested > 1 {
            return self.lang_utils_outputs(name, &args, requested, out);
        }
        if name == "eval" && requested == 0 {
            self.langx_eval(&args, false, out)?;
            return Ok(Vec::new());
        }
        if requested > 1 {
            return Err(format!(
                "{name}: multiple outputs through a function handle are not supported yet"
            ));
        }
        if args.is_empty()
            && let Some(value) = self.builtin_constant(name)?
        {
            return Ok(vec![value]);
        }
        let given = args.len();
        Ok(match self.call(name, args, out) {
            Ok(Value::Void) => Vec::new(),
            Ok(value) => vec![value],
            Err(message) => {
                return Err(arity_message(name, &message, given).map_or(message, str::to_owned));
            }
        })
    }
    /// `@pi` and friends name the builtin, so workspace variables never shadow it.
    fn builtin_constant(&self, name: &str) -> Result<Option<Value>> {
        Ok(Some(match name {
            "pi" => Value::Matrix(Matrix::scalar(std::f64::consts::PI)),
            "eps" => Value::Matrix(Matrix::scalar(f64::EPSILON)),
            "Inf" | "inf" => Value::Matrix(Matrix::scalar(f64::INFINITY)),
            "NaN" | "nan" => Value::Matrix(Matrix::scalar(crate::MATLAB_NAN)),
            "i" | "j" => {
                Value::Complex(ComplexMatrix::scalar(Complex64::new(0., 1.), &self.cancel)?)
            }
            _ => return Ok(None),
        }))
    }
    fn call_anonymous(
        &mut self,
        function: &AnonymousFunction,
        captured: &[(String, Array)],
        args: Vec<Value>,
        requested: usize,
        targets: Option<&[Option<String>]>,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        let variadic = function.params.last().is_some_and(|p| p == "varargin");
        let fixed = function.params.len() - usize::from(variadic);
        if args.len() > fixed && !variadic {
            return Err("Too many input arguments.".into());
        }
        if args.len() < fixed {
            // A parameter that was not passed is unset, so it shadows any
            // function of that name; reading it is the error.
            let mut read = BTreeSet::new();
            free_names(&function.body, &mut Vec::new(), &mut read);
            if function.params[args.len()..fixed]
                .iter()
                .any(|name| name != "~" && read.contains(name))
            {
                return Err("Not enough input arguments.".into());
            }
        }
        let mut scope: Vec<(String, Array)> = captured.to_vec();
        let mut args = args.into_iter();
        for name in &function.params[..fixed] {
            let Some(value) = args.next() else { break };
            let array = value
                .array(&self.cancel)?
                .without_coordinate_facts(&self.cancel)?;
            if name != "~" {
                scope.push((name.clone(), array));
            }
        }
        if variadic {
            let mut rest = Vec::new();
            for value in args {
                let array = value
                    .array(&self.cancel)?
                    .without_coordinate_facts(&self.cancel)?;
                rest.push(Arc::new(array));
            }
            // Reference behavior: no extra inputs leave a 0x0 varargin, not 1x0.
            let rows = usize::from(!rest.is_empty());
            let cells = CellArray::from_shared(rows, rest.len(), rest, &self.cancel)?;
            scope.push(("varargin".into(), cells.into()));
        }
        let mut numeric = BTreeMap::new();
        let mut logical = BTreeMap::new();
        let mut complex = BTreeMap::new();
        let mut containers = BTreeMap::new();
        for (name, array) in scope {
            match array {
                Array::Numeric(m) => {
                    numeric.insert(name, m);
                }
                Array::Logical(m) => {
                    logical.insert(name, m);
                }
                Array::Complex(m) => {
                    complex.insert(name, m);
                }
                other => {
                    containers.insert(name, other);
                }
            }
        }
        // Frames parked on the Rust stack count against the retained budget.
        let retained = self.evaluated_elements.saturating_mul(8);
        if self.functions.retained_bytes.saturating_add(retained) > MAX_RETAINED_BYTES {
            return Err(
                "Retained function caller workspaces/evaluation estimates exceed 256 MiB".into(),
            );
        }
        self.check_cancel()?;
        self.lng_sync()?;
        let caller_numeric = std::mem::replace(&mut self.variables, numeric);
        let caller_logical = std::mem::replace(&mut self.logical_variables, logical);
        let caller_complex = std::mem::replace(&mut self.complex_variables, complex);
        let caller_containers = std::mem::replace(&mut self.container_variables, containers);
        self.lng_enter(
            super::lang_more::LngWorkspace {
                numeric: caller_numeric,
                logical: caller_logical,
                complex: caller_complex,
                containers: caller_containers,
            },
            "::<anonymous>".into(),
            Vec::new(),
        );
        let caller_arguments = self.functions.arguments.take();
        let caller_current = self.functions.current.take();
        let caller_evaluated = self.evaluated_elements;
        self.functions.retained_bytes += retained;
        let result = if requested >= 2 {
            let default;
            let targets = match targets {
                Some(targets) => targets,
                None => {
                    default = vec![Some(String::from("_")); requested];
                    &default
                }
            };
            self.eval_outputs(&function.body, targets, out)
        } else {
            self.eval(&function.body, None, out).map(|value| {
                if matches!(value, Value::Void) {
                    Vec::new()
                } else {
                    vec![value]
                }
            })
        };
        // Always restore the caller, including after cancellation and errors.
        let (caller, cleanup) = self.lng_leave(out, result.as_ref().map_or(&[], Vec::as_slice));
        self.variables = caller.numeric;
        self.logical_variables = caller.logical;
        self.complex_variables = caller.complex;
        self.container_variables = caller.containers;
        self.functions.arguments = caller_arguments;
        self.functions.current = caller_current;
        self.evaluated_elements = caller_evaluated;
        self.functions.retained_bytes -= retained;
        result.and_then(|values| cleanup.map(|()| values))
    }

    /// Multi-output forms: `[a,b] = h(x)`, `[a,b] = arrayfun(...)`, `feval`.
    pub(super) fn handle_outputs(
        &mut self,
        name: &str,
        args: &[Expr],
        targets: &[Option<String>],
        out: &mut Execution,
    ) -> Result<Option<Vec<Value>>> {
        let count = targets.len();
        if let Some(handle) = self.handle_variable(name) {
            let values = self.function_arguments(args, None, out)?;
            let results = self.call_handle(&handle, values, count, Some(targets), out)?;
            return Self::exact_outputs(name, results, count).map(Some);
        }
        if !matches!(name, "arrayfun" | "cellfun" | "feval")
            || self.contains_array(name)
            || self.resolve_function(name)?.is_some()
        {
            return Ok(None);
        }
        let values = self.function_arguments(args, None, out)?;
        let results = self.handle_builtin(name, &values, count, Some(targets), out)?;
        Self::exact_outputs(name, results, count).map(Some)
    }
    fn exact_outputs(name: &str, results: Vec<Value>, count: usize) -> Result<Vec<Value>> {
        if results.len() < count {
            return Err(format!("{name}: too many output arguments requested"));
        }
        Ok(results)
    }

    /// Single-output dispatch from `call`; `None` when `name` is not ours.
    pub(super) fn handle_call(
        &mut self,
        name: &str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        if !HANDLE_NAMES.contains(&name) {
            return Ok(None);
        }
        let mut results = self.handle_builtin(name, args, 1, None, out)?;
        Ok(Some(if results.is_empty() {
            Value::Void
        } else {
            results.remove(0)
        }))
    }
    fn handle_builtin(
        &mut self,
        name: &str,
        args: &[Value],
        requested: usize,
        targets: Option<&[Option<String>]>,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        match name {
            "func2str" => {
                let [Value::Container(Array::Handle(handle))] = args else {
                    return Err("func2str: input must be a function handle".into());
                };
                Ok(vec![Value::Text(handle.func2str().to_owned())])
            }
            "str2func" => {
                let [text] = args else {
                    return Err("str2func expects one argument".into());
                };
                let text = text_of(text).ok_or("str2func: input must be a character row")?;
                self.str2func(text.trim())
            }
            "feval" => {
                let Some((callee, rest)) = args.split_first() else {
                    return Err("feval expects a function handle or name".into());
                };
                let callee = Self::callee(name, callee)?;
                self.call_callee(&callee, rest.to_vec(), requested, targets, out)
            }
            _ => self.map_call(name, args, requested, targets, out),
        }
    }
    fn str2func(&mut self, text: &str) -> Result<Vec<Value>> {
        if text.starts_with('@') {
            return Ok(vec![match syntax::parse_expression(text)? {
                Expr::Anonymous(function) => self.anonymous_handle(&function, false)?,
                Expr::FunctionHandle(name) => self.named_handle(&name)?,
                _ => return Err("str2func: expected @name or @(args) expression".into()),
            }]);
        }
        if !crate::files::valid_name(text) {
            return Err(format!("str2func: invalid function name '{text}'"));
        }
        Ok(vec![self.named_handle(text)?])
    }
    fn callee(name: &str, value: &Value) -> Result<Callee> {
        match value {
            Value::Container(Array::Handle(handle)) => Ok(Callee::Handle(handle.clone())),
            other => match text_of(other) {
                Some(text) if crate::files::valid_name(&text) => Ok(Callee::Name(text)),
                _ => Err(format!(
                    "{name}: the first argument must be a function handle or function name"
                )),
            },
        }
    }
    fn call_callee(
        &mut self,
        callee: &Callee,
        args: Vec<Value>,
        requested: usize,
        targets: Option<&[Option<String>]>,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        match callee {
            Callee::Handle(handle) => self.call_handle(handle, args, requested, targets, out),
            Callee::Name(name) => {
                let handle = FunctionHandle::named(name);
                self.call_handle(&handle, args, requested, targets, out)
            }
        }
    }

    /// `arrayfun`/`cellfun`: call the function once per element, in order.
    fn map_call(
        &mut self,
        name: &str,
        args: &[Value],
        requested: usize,
        targets: Option<&[Option<String>]>,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        let Some((function, rest)) = args.split_first() else {
            return Err(format!("{name} expects a function and at least one array"));
        };
        let first = (!rest.is_empty()).then(|| &rest[0]);
        if name == "cellfun"
            && let Some(legacy) = text_of(function).filter(|t| LEGACY_CELLFUN.contains(&t.as_str()))
            && let Some(Value::Container(Array::Cell(cells))) = first
            && rest.len() == 1
        {
            return Ok(vec![self.legacy_cellfun(&legacy, cells)?]);
        }
        if name == "cellfun"
            && let Some(legacy) =
                text_of(function).filter(|t| matches!(t.as_str(), "isclass" | "size"))
        {
            return Err(format!(
                "cellfun: legacy form '{legacy}' is not supported yet"
            ));
        }
        let callee = Self::callee(name, function)?;
        // Data arrays run up to the first option name; the rest are name/value pairs.
        let option_at = rest
            .iter()
            .position(|v| {
                text_of(v).is_some_and(|t| {
                    t.eq_ignore_ascii_case("UniformOutput")
                        || t.eq_ignore_ascii_case("ErrorHandler")
                })
            })
            .unwrap_or(rest.len());
        let (arrays, options) = rest.split_at(option_at);
        if arrays.is_empty() {
            return Err("Not enough input arguments.".into());
        }
        if options.len() % 2 != 0 {
            return Err(format!("{name}: options must be name/value pairs"));
        }
        let mut uniform = true;
        for pair in options.chunks(2) {
            let key = text_of(&pair[0]).unwrap_or_default();
            if key.eq_ignore_ascii_case("UniformOutput") {
                uniform = match &pair[1] {
                    Value::Logical(m) if m.len() == 1 => m.data()[0] != 0,
                    Value::Matrix(m) if m.len() == 1 => m.data()[0] != 0.,
                    _ => {
                        return Err(format!(
                            "{name}: UniformOutput must be a logical or numeric scalar"
                        ));
                    }
                };
            } else if key.eq_ignore_ascii_case("ErrorHandler") {
                return Err(format!("{name}: ErrorHandler is not supported yet"));
            } else {
                return Err(format!("{name}: unknown option '{key}'"));
            }
        }
        let mut inputs = Vec::with_capacity(arrays.len());
        for (i, value) in arrays.iter().enumerate() {
            let array = value.clone().array(&self.cancel)?;
            if name == "cellfun" && !matches!(array, Array::Cell(_)) {
                return Err(format!(
                    "Input #{} expected to be a cell array, was {} instead.",
                    i + 2,
                    array.class()
                ));
            }
            if matches!(array, Array::Graphics(_)) {
                return Err(format!(
                    "{name}: graphics handle arrays are not supported yet"
                ));
            }
            if let Some(first) = inputs.first() {
                let first: &Array = first;
                if (array.rows(), array.cols()) != (first.rows(), first.cols()) {
                    let dimension = if array.rows() != first.rows() { 1 } else { 2 };
                    let expected = if dimension == 1 {
                        first.rows()
                    } else {
                        first.cols()
                    };
                    let found = if dimension == 1 {
                        array.rows()
                    } else {
                        array.cols()
                    };
                    return Err(format!(
                        "All of the input arguments must be of the same size and shape.\nPrevious inputs had size {expected} in dimension {dimension}. Input #{} has size {found}",
                        i + 2
                    ));
                }
            }
            inputs.push(array);
        }
        let (rows, cols) = (inputs[0].rows(), inputs[0].cols());
        let count = inputs[0].len();
        let outputs = requested.max(1);
        // Admit the whole result before reserving anything for it: one step per
        // element and one evaluated element per output slot.
        self.admit_steps(count)?;
        self.admit(count.saturating_mul(outputs))?;
        let mut columns: Vec<Option<Column>> = (0..outputs).map(|_| None).collect();
        let mut cells: Vec<Vec<Arc<Array>>> = if uniform {
            Vec::new()
        } else {
            (0..outputs).map(|_| Vec::with_capacity(count)).collect()
        };
        let mut silent = 0usize;
        for index in 0..count {
            self.count_step()?;
            let mut element = Vec::with_capacity(inputs.len());
            for input in &inputs {
                element.push(match input {
                    Array::Cell(c) if name == "cellfun" => {
                        Value::from(c.get_linear(index)?.as_ref().clone())
                    }
                    _ => self.element_value(input, index)?,
                });
            }
            let results = self.call_callee(&callee, element, requested, targets, out)?;
            if results.is_empty() && requested <= 1 {
                silent += 1;
                continue;
            }
            if results.len() < outputs {
                return Err("Too many output arguments.".into());
            }
            for (k, value) in results.into_iter().take(outputs).enumerate() {
                if uniform {
                    self.push_uniform(&mut columns[k], value, index, k)?;
                } else {
                    cells[k].push(Arc::new(value.array(&self.cancel)?));
                }
            }
        }
        if silent != 0 {
            if silent != count {
                return Err("The function returned a value for some elements only.".into());
            }
            return Ok(vec![Value::Void]);
        }
        let mut results = Vec::with_capacity(outputs);
        for k in 0..outputs {
            self.check_cancel()?;
            results.push(if uniform {
                self.finish_column(columns[k].take(), rows, cols)?
            } else {
                let slots = std::mem::take(&mut cells[k]);
                let cell = if slots.is_empty() {
                    CellArray::empty(rows, cols, &self.cancel)?
                } else {
                    CellArray::from_shared(rows, cols, slots, &self.cancel)?
                };
                Value::Container(cell.into())
            });
        }
        self.charge_function_outputs(&results)?;
        Ok(results)
    }
    /// Fail early when `elements` more would exceed the statement's evaluation
    /// budget; nothing is charged.
    fn admit(&mut self, elements: usize) -> Result<()> {
        let charged = self.evaluated_elements;
        let admitted = self.charge(elements);
        self.evaluated_elements = charged;
        admitted
    }
    /// Element `index` (column-major) of an array as a function argument.
    fn element_value(&self, array: &Array, index: usize) -> Result<Value> {
        Ok(match array {
            Array::Numeric(m) => Value::Matrix(Matrix::scalar(m.data()[index])),
            Array::Logical(m) => Value::Logical(LogicalMatrix::scalar(m.data()[index] != 0)),
            Array::Complex(m) => {
                Value::Complex(ComplexMatrix::scalar(m.data()[index], &self.cancel)?)
            }
            Array::Character(c) => Value::Container(
                CharacterRow::from_column_major(1, 1, vec![c.units()[index]], &self.cancel)?.into(),
            ),
            Array::Cell(c) => {
                if array.rows() == 0 || array.cols() == 0 {
                    return Err("Cell element index out of range".into());
                }
                // arrayfun over a cell passes 1x1 cells; cellfun passes contents.
                Value::Container(
                    CellArray::from_shared(1, 1, vec![c.get_linear(index)?.clone()], &self.cancel)?
                        .into(),
                )
            }
            Array::Handle(_) => Value::Container(array.clone()),
            Array::Typed(t) => Value::Container(t.select(1, 1, &[index])?.into()),
            Array::Struct(s) => Value::Container(s.select(1, 1, &[index], &self.cancel)?.into()),
            Array::Graphics(_) => return Err("graphics handle arrays are not supported yet".into()),
        })
    }
    fn push_uniform(
        &self,
        column: &mut Option<Column>,
        value: Value,
        index: usize,
        output: usize,
    ) -> Result<()> {
        let non_scalar = || {
            format!(
                "Non-scalar in Uniform output, at index {}, output {}.\nSet 'UniformOutput' to false.",
                index + 1,
                output + 1
            )
        };
        let array = value.array(&self.cancel)?;
        if matches!(array, Array::Handle(_)) {
            return Err(
                "function_handle output type is not supported. Set 'UniformOutput' to false."
                    .into(),
            );
        }
        if array.len() != 1 {
            return Err(non_scalar());
        }
        let class_error = |expected: &str, found: &str| {
            format!(
                "Mismatch in type of outputs, at index {}, output {} ({expected} versus {found}).\nSet 'UniformOutput' to false.",
                index + 1,
                output + 1
            )
        };
        match (column.as_mut(), &array) {
            (None, Array::Numeric(m)) => *column = Some(Column::Real(vec![m.data()[0]])),
            (None, Array::Logical(m)) => *column = Some(Column::Logical(vec![m.data()[0]])),
            (None, Array::Complex(m)) => *column = Some(Column::Complex(vec![m.data()[0]])),
            (None, Array::Character(c)) => *column = Some(Column::Char(vec![c.units()[0]])),
            (None, Array::Cell(c)) => *column = Some(Column::Cell(vec![c.get_linear(0)?.clone()])),
            (None, Array::Struct(s)) => {
                *column = Some(Column::Struct {
                    values: vec![s.clone()],
                    retained_bytes: s.retained_bytes(),
                    nodes: s.nodes(),
                });
            }
            (Some(Column::Real(v)), Array::Numeric(m)) => v.push(m.data()[0]),
            (Some(Column::Logical(v)), Array::Logical(m)) => v.push(m.data()[0]),
            (Some(Column::Complex(v)), Array::Complex(m)) => v.push(m.data()[0]),
            (Some(Column::Complex(v)), Array::Numeric(m)) => {
                v.push(Complex64::new(m.data()[0], 0.));
            }
            (Some(Column::Real(v)), Array::Complex(m)) => {
                let mut promoted: Vec<Complex64> =
                    v.iter().map(|&re| Complex64::new(re, 0.)).collect();
                promoted.push(m.data()[0]);
                *column = Some(Column::Complex(promoted));
            }
            (Some(Column::Char(v)), Array::Character(c)) => v.push(c.units()[0]),
            (Some(Column::Cell(v)), Array::Cell(c)) => v.push(c.get_linear(0)?.clone()),
            (
                Some(Column::Struct {
                    values,
                    retained_bytes,
                    nodes,
                }),
                Array::Struct(s),
            ) => {
                let first = &values[0];
                if first.field_count() != s.field_count()
                    || first
                        .field_names()
                        .iter()
                        .any(|name| s.field_index(name).is_none())
                {
                    return Err(
                        "Field names of the structures being concatenated must match".into(),
                    );
                }
                // Bound the aggregate before growing the result. Counting each
                // scalar's field-name metadata is deliberately conservative.
                *retained_bytes = retained_bytes.saturating_add(s.retained_bytes());
                *nodes = nodes.saturating_add(s.nodes());
                crate::containers::checked_totals(*retained_bytes, *nodes, s.depth())?;
                values.push(s.clone());
            }
            (Some(existing), other) if !matches!(other, Array::Graphics(_)) => {
                return Err(class_error(existing.class(), other.class()));
            }
            (_, _) => return Err(non_scalar()),
        }
        Ok(())
    }
    fn finish_column(&self, column: Option<Column>, rows: usize, cols: usize) -> Result<Value> {
        Ok(match column {
            None => Value::Matrix(Matrix::from_column_vec(rows, cols, Vec::new())?),
            Some(Column::Real(v)) => Value::Matrix(Matrix::from_column_vec(rows, cols, v)?),
            Some(Column::Logical(v)) => {
                Value::Logical(LogicalMatrix::from_column_vec(rows, cols, v, &self.cancel)?)
            }
            Some(Column::Complex(v)) => {
                Value::Complex(ComplexMatrix::from_column_vec(rows, cols, v, &self.cancel)?)
            }
            Some(Column::Char(v)) => Value::Container(
                CharacterRow::from_column_major(rows, cols, v, &self.cancel)?.into(),
            ),
            Some(Column::Cell(v)) => {
                Value::Container(CellArray::from_shared(rows, cols, v, &self.cancel)?.into())
            }
            Some(Column::Struct { values, .. }) => {
                let names = values[0].field_names().to_vec();
                let total = StructArray::slot_total(values.len(), names.len())?;
                let mut slots = Vec::with_capacity(total);
                for value in values {
                    self.check_cancel()?;
                    for name in &names {
                        let field = value
                            .field_index(name)
                            .expect("matching field sets checked");
                        slots.push(value.get(field, 0)?.clone());
                    }
                }
                Value::Container(
                    StructArray::from_slots(rows, cols, names, slots, &self.cancel)?.into(),
                )
            }
        })
    }
    /// `cellfun('isempty', C)` and the other legacy name forms.
    fn legacy_cellfun(&mut self, name: &str, cells: &CellArray) -> Result<Value> {
        let (rows, cols) = (cells.rows(), cells.cols());
        let mut numbers = Vec::new();
        let mut flags = Vec::new();
        for i in 0..cells.len() {
            if i.is_multiple_of(1024) {
                self.check_cancel()?;
            }
            let item = cells.get_linear(i)?;
            match name {
                "isempty" => flags.push(u8::from(item.is_empty())),
                // Characters and logicals are real; complex storage, cells and handles are not.
                "isreal" => flags.push(u8::from(matches!(
                    item.as_ref(),
                    Array::Numeric(_) | Array::Logical(_) | Array::Character(_)
                ))),
                "islogical" => flags.push(u8::from(item.is_logical())),
                "length" => numbers.push(if item.is_empty() {
                    0.
                } else {
                    item.rows().max(item.cols()) as f64
                }),
                "numel" | "prodofsize" => numbers.push(item.len() as f64),
                _ => numbers.push(2.),
            }
        }
        self.charge(cells.len())?;
        if matches!(name, "isempty" | "isreal" | "islogical") {
            Ok(Value::Logical(LogicalMatrix::from_column_vec(
                rows,
                cols,
                flags,
                &self.cancel,
            )?))
        } else {
            Ok(Value::Matrix(Matrix::from_column_vec(rows, cols, numbers)?))
        }
    }
}
