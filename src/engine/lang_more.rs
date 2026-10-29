//! Language/workspace utilities. Array payloads stay shared; saved
//! frames contain only the existing immutable array handles and bounded maps.
use super::{Engine, Execution, Value};
use crate::{
    Array, CellArray, CharacterRow, ComplexMatrix, LogicalMatrix, Matrix, Result, StructArray,
    syntax::{self, Expr},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Weak},
};

const FEW: &str = "Not enough input arguments.";
const MANY: &str = "Too many input arguments.";
const LIMIT: usize = 65536;
pub(super) const TAG: &str = "MatRustObjectKind";
const OBJECT_ID: &str = "MatRustObjectId";
const CAUGHT: &str = "MatRustCaught";

trait LngCells {
    fn lng_contents(&self) -> impl ExactSizeIterator<Item = &Arc<Array>>;
}
impl LngCells for CellArray {
    fn lng_contents(&self) -> impl ExactSizeIterator<Item = &Arc<Array>> {
        (0..self.len()).map(|i| self.get_linear(i).expect("bounded cell index"))
    }
}

impl Engine {
    pub(super) fn lng_note_inputs(&mut self, args: &[Expr], count: usize) {
        self.functions.lng.pending_names = if args.iter().any(|a| {
            matches!(
                a,
                Expr::Field(_, _)
                    | Expr::DynamicField(_, _)
                    | Expr::CellContents(_, _)
                    | Expr::Brace(_, _)
            )
        }) {
            vec![String::new(); count]
        } else {
            args.iter()
                .map(|a| {
                    if let Expr::Name(n) = a {
                        n.clone()
                    } else {
                        String::new()
                    }
                })
                .collect()
        };
    }
    fn lng_outputs(
        &mut self,
        name: &str,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        let mut values = match name {
            "evalc" => {
                lng_arity(args, 1, 2)?;
                if count > 2 {
                    return Err("evalc currently supports at most two outputs.".into());
                }
                let pending = std::mem::take(&mut self.pending_line);
                let mut capture = Execution::default();
                let result = self.langx_eval(args, count > 1, &mut capture);
                let trailing = std::mem::replace(&mut self.pending_line, pending);
                let v = result?;
                let text = capture
                    .output
                    .iter()
                    .map(|s| format!("{s}\n"))
                    .collect::<String>()
                    + &trailing;
                let mut values = vec![Value::Text(text)];
                if count > 1 {
                    values.push(v);
                }
                values
            }
            "lasterr" | "lastwarn" => {
                lng_arity(args, 0, 2)?;
                if count > 2 {
                    return Err("Too many output arguments.".into());
                }
                let update = args.first().map(|v| self.lng_text(v)).transpose()?;
                let id = args
                    .get(1)
                    .map(|v| self.lng_text(v))
                    .transpose()?
                    .unwrap_or_default();
                let state = if name == "lasterr" {
                    &mut self.functions.lng.last_error
                } else {
                    &mut self.functions.lng.last_warning
                };
                let old = state.clone();
                if let Some(message) = update {
                    *state = (message, id);
                }
                vec![Value::Text(old.0), Value::Text(old.1)]
            }
            "sscanf" => self.lng_sscanf(args)?,
            "strread" | "textread" => self.lng_textread(name, args, count, out)?,
            _ => unreachable!(),
        };
        if values.len() < count {
            return Err("Too many output arguments.".into());
        }
        values.truncate(count);
        Ok(values)
    }
    fn lng_sscanf(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        lng_arity(args, 2, 3)?;
        let text = self
            .lng_text(&args[0])
            .map_err(|_| "First argument must be a string.".to_owned())?;
        let fmt = self.lng_text(&args[1])?;
        let items = super::file_io::scan_format("sscanf", &fmt)?;
        let spec = super::file_io::size_spec("sscanf", args.get(2))?;
        let scanned =
            super::file_io::scan(text.as_bytes(), &items, spec.limit, false, &self.cancel)?;
        let n = scanned.values.len();
        // The count output is the number of successful conversions.
        let conversions = scanned.fields;
        // Padding to a requested row count is charged with the values.
        self.charge(
            spec.rows
                .filter(|&r| r > 0 && r <= n)
                .map_or(n, |r| r * n.div_ceil(r)),
        )?;
        let position = scanned.consumed;
        let message = if position < text.len() && n < spec.limit {
            "Matching failure in format."
        } else {
            ""
        };
        let value = if scanned.all_text && n > 0 {
            let mut units: Vec<_> = scanned.values.iter().map(|n| *n as u16).collect();
            let (rows, cols) = spec
                .rows
                .map_or((1, n), |r| if r > n { (n, 1) } else { (r, n.div_ceil(r)) });
            units.resize(rows * cols, 0);
            Value::Container(
                CharacterRow::from_column_major(rows, cols, units, &self.cancel)?.into(),
            )
        } else {
            let (rows, cols, data) = super::file_io::shape_values(scanned.values, spec);
            Value::Matrix(Matrix::from_column_vec(rows, cols, data)?)
        };
        Ok(vec![
            value,
            lng_number(conversions as f64),
            Value::Text(message.into()),
            lng_number((position + 1) as f64),
        ])
    }
    fn lng_textread(
        &mut self,
        name: &str,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        lng_arity(args, 1, 255)?;
        let source = self.lng_text(&args[0])?;
        let default = args.len() == 1;
        let mut scan_args = args.to_vec();
        scan_args[0] = Value::Text(source.clone());
        let mut fid = None;
        if name == "textread" {
            let opened = self
                .fileio_outputs(
                    "fopen",
                    &[Value::Text(source), Value::Text("r".into())],
                    1,
                    out,
                )?
                .remove(0);
            if self.lng_scalar(&opened)? < 0. {
                return Err("File not found.".into());
            }
            fid = Some(opened.clone());
            scan_args[0] = opened;
        }
        let result = self.fileio_outputs("textscan", &scan_args, 1, out);
        if let Some(fid) = fid {
            self.fileio_outputs("fclose", &[fid], 1, out)?;
        }
        let Value::Container(Array::Cell(c)) = result?.remove(0) else {
            return Err("Invalid textscan result".into());
        };
        let mut columns: Vec<_> = c
            .lng_contents()
            .map(|v| Value::from(v.as_ref().clone()))
            .collect();
        if default
            && name == "strread"
            && let Some(Value::Matrix(m)) = columns.first_mut()
        {
            *m = m.reshape(1, m.len())?;
        }
        if columns.len() != count {
            return Err("Number of outputs must match the format conversions.".into());
        }
        Ok(columns)
    }
    fn lng_nargchk(&mut self, name: &str, args: &[Value], out: &mut Execution) -> Result<Value> {
        lng_arity(args, if name == "nargchk" { 3 } else { 2 }, 4)?;
        let lo = self.lng_scalar(&args[0])?;
        let hi = self.lng_scalar(&args[1])?;
        if lo > hi {
            return Err("Minimum must be less than or equal to maximum.".into());
        }
        let n = if args.len() >= 3 {
            self.lng_scalar(&args[2])?
        } else {
            self.functions
                .arguments
                .ok_or("nargoutchk must be called from a function.")?
                .1 as f64
        };
        let input = name == "nargchk";
        let message = if n < lo {
            if input {
                "Not enough input arguments."
            } else {
                "Not enough output arguments."
            }
        } else if n > hi {
            if input {
                "Too many input arguments."
            } else {
                "Too many output arguments."
            }
        } else {
            ""
        };
        let suffix = if n < lo { "notEnough" } else { "tooMany" };
        let id = if message.is_empty() {
            String::new()
        } else if args.len() == 2 {
            format!("MATLAB:nargoutchk:{suffix}Outputs")
        } else {
            format!(
                "MATLAB:{name}:{suffix}{}",
                if input { "Inputs" } else { "Outputs" }
            )
        };
        if args.len() == 2 {
            if message.is_empty() {
                return Ok(Value::Void);
            }
            return Err(self.raise(message.into(), id));
        }
        if input {
            self.lng_warn(
                "NARGCHK will be removed in a future release. Use NARGINCHK or NARGOUTCHK instead.",
                "MATLAB:nargchk:deprecated",
                out,
            )?;
        }
        let mode = args
            .get(3)
            .map(|v| self.lng_text(v))
            .transpose()?
            .unwrap_or_else(|| "string".into());
        match mode.as_str() {
            "string" if message.is_empty() => {
                Ok(Value::Matrix(Matrix::from_column_vec(0, 0, Vec::new())?))
            }
            "string" => Ok(Value::Text(message.into())),
            "struct" => Ok(Value::Container(Array::Struct(StructArray::from_slots(
                1,
                1,
                vec!["message".into(), "identifier".into()],
                vec![
                    Arc::new(self.lng_char(message)?),
                    Arc::new(self.lng_char(&id)?),
                ],
                &self.cancel,
            )?))),
            _ => Err("Invalid output format.".into()),
        }
    }
    fn lng_validate(&mut self, args: &[Value], out: &mut Execution) -> Result<()> {
        lng_arity(args, 3, 6)?;
        let a = args[0].clone().array(&self.cancel)?;
        let types = self.lng_strings(&args[1])?;
        let numeric = matches!(a, Array::Numeric(_) | Array::Complex(_) | Array::Typed(_));
        let accepted = types.iter().any(|t| {
            t == a.class()
                || t == "numeric" && numeric
                || t == "float" && matches!(a.class(), "double" | "single")
        });
        let argument = args
            .get(4)
            .map(|v| self.lng_text(v))
            .transpose()?
            .unwrap_or_default();
        let position = args.get(5).map(|n| self.lng_scalar(n)).transpose()?;
        let prefix = match (position, argument.is_empty()) {
            (Some(n), false) => format!("Expected input number {n}, {argument},"),
            (Some(n), true) => format!("Expected input number {n}"),
            (None, false) => format!("Expected {argument}"),
            (None, true) => "Expected input".into(),
        };
        if !accepted {
            let mut expanded = Vec::new();
            for t in types {
                if t == "numeric" {
                    expanded.extend(
                        [
                            "double", "single", "uint8", "uint16", "uint32", "uint64", "int8",
                            "int16", "int32", "int64",
                        ]
                        .map(String::from),
                    );
                } else {
                    expanded.push(t);
                }
            }
            return Err(format!(
                "{prefix} to be one of these types:\n\n{}\n\nInstead its type was {}.",
                expanded.join(", "),
                a.class()
            ));
        }
        let Value::Container(Array::Cell(attributes)) = &args[2] else {
            return Err("Attributes must be a cell array.".into());
        };
        let list: Vec<_> = attributes
            .lng_contents()
            .map(|a| Value::from(a.as_ref().clone()))
            .collect();
        let mut i = 0;
        while i < list.len() {
            self.check_cancel()?;
            let attr = self.lng_text(&list[i])?;
            i += 1;
            let (valid, description, id) = match attr.as_str() {
                "positive" => (
                    self.lng_all(&a, |n| n > 0.)?,
                    "positive".into(),
                    "expectedPositive",
                ),
                "nonnegative" => (
                    self.lng_all(&a, |n| n >= 0.)?,
                    "nonnegative".into(),
                    "expectedNonnegative",
                ),
                "negative" => (
                    self.lng_all(&a, |n| n < 0.)?,
                    "negative".into(),
                    "expectedNegative",
                ),
                "nonzero" => (
                    self.lng_all(&a, |n| n != 0.)?,
                    "nonzero".into(),
                    "expectedNonzero",
                ),
                "integer" => (
                    self.lng_all(&a, |n| n.is_finite() && n.fract() == 0.)?,
                    "integer-valued".into(),
                    "expectedInteger",
                ),
                "even" => (
                    self.lng_all(&a, |n| n.is_finite() && n % 2. == 0.)?,
                    "even".into(),
                    "expectedEven",
                ),
                "odd" => (
                    self.lng_all(&a, |n| n.is_finite() && n.abs() % 2. == 1.)?,
                    "odd".into(),
                    "expectedOdd",
                ),
                "binary" => (
                    self.lng_all(&a, |n| n == 0. || n == 1.)?,
                    "binary".into(),
                    "expectedBinary",
                ),
                "finite" => (
                    self.lng_all(&a, |n| n.is_finite())?,
                    "finite".into(),
                    "expectedFinite",
                ),
                "nonnan" => (
                    self.lng_all(&a, |n| !n.is_nan())?,
                    "non-NaN".into(),
                    "expectedNonNaN",
                ),
                "real" => (
                    !matches!(a, Array::Complex(_)),
                    "real".into(),
                    "expectedReal",
                ),
                "scalar" => (a.len() == 1, "a scalar".into(), "expectedScalar"),
                "vector" => (
                    a.rows() == 1 || a.cols() == 1,
                    "a vector".into(),
                    "expectedVector",
                ),
                "row" => (a.rows() == 1, "a row vector".into(), "expectedRow"),
                "column" => (a.cols() == 1, "a column vector".into(), "expectedColumn"),
                "nonempty" => (!a.is_empty(), "nonempty".into(), "expectedNonempty"),
                // Arrays here are two-dimensional and never sparse.
                "2d" => (true, "two-dimensional".into(), "expected2D"),
                "3d" => (true, "three-dimensional".into(), "expected3D"),
                "nonsparse" => (true, "nonsparse".into(), "expectedNonsparse"),
                "square" => (a.rows() == a.cols(), "square".into(), "expectedSquare"),
                "diag" => (self.lng_diagonal(&a)?, "diagonal".into(), "expectedDiag"),
                "increasing" => (
                    self.lng_ordered(&a, |o| o.is_lt())?,
                    "increasing valued".into(),
                    "expectedIncreasing",
                ),
                "decreasing" => (
                    self.lng_ordered(&a, |o| o.is_gt())?,
                    "decreasing valued".into(),
                    "expectedDecreasing",
                ),
                "nondecreasing" => (
                    self.lng_ordered(&a, |o| o.is_le())?,
                    "nondecreasing valued".into(),
                    "expectedNondecreasing",
                ),
                "nonincreasing" => (
                    self.lng_ordered(&a, |o| o.is_ge())?,
                    "nonincreasing valued".into(),
                    "expectedNonincreasing",
                ),
                "size" => {
                    let expected = list.get(i).ok_or(FEW)?.clone().matrix(&self.cancel)?;
                    i += 1;
                    let shape = [a.rows() as f64, a.cols() as f64];
                    let valid = expected
                        .data()
                        .iter()
                        .enumerate()
                        .all(|(j, n)| n.is_nan() || *n == *shape.get(j).unwrap_or(&1.));
                    let desc = expected
                        .data()
                        .iter()
                        .map(|n| {
                            if n.is_nan() {
                                "NaN".into()
                            } else {
                                n.to_string()
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("x");
                    (
                        valid,
                        format!(
                            "of size {desc} when it is actually size {}x{}",
                            a.rows(),
                            a.cols()
                        ),
                        "incorrectSize",
                    )
                }
                ">" | ">=" | "<" | "<=" => {
                    let n = self.lng_scalar(list.get(i).ok_or(FEW)?)?;
                    i += 1;
                    let valid = match attr.as_str() {
                        ">" => self.lng_all(&a, |x| x > n)?,
                        ">=" => self.lng_all(&a, |x| x >= n)?,
                        "<" => self.lng_all(&a, |x| x < n)?,
                        _ => self.lng_all(&a, |x| x <= n)?,
                    };
                    (
                        valid,
                        format!("an array with all of the values {attr} {n}"),
                        "expectedValue",
                    )
                }
                "numel" | "nrows" | "ncols" | "ndims" => {
                    let n = self.lng_scalar(list.get(i).ok_or(FEW)?)?;
                    i += 1;
                    let (actual, what, id) = match attr.as_str() {
                        "numel" => (a.len(), "elements", "incorrectNumel"),
                        "nrows" => (a.rows(), "rows", "incorrectNumrows"),
                        "ncols" => (a.cols(), "columns", "incorrectNumcols"),
                        _ => (2, "dimensions", "incorrectNumdims"),
                    };
                    (
                        actual as f64 == n,
                        format!("an array with number of {what} equal to {n}"),
                        id,
                    )
                }
                _ => {
                    self.lng_warn(
                        &format!("Attribute {attr} not supported."),
                        "MATLAB:validateattributes:unsupportedAttribute",
                        out,
                    )?;
                    (true, String::new(), "")
                }
            };
            if !valid {
                let function = args.get(3).map(|v| self.lng_text(v)).transpose()?;
                // The size check names its own identifier: MATLAB:incorrectSize
                // without a function name (checked against the reference by the orchestrator).
                let identifier = match function {
                    Some(f) if id == "incorrectSize" && f.is_empty() => format!("MATLAB:{id}"),
                    None if id == "incorrectSize" => format!("MATLAB:{id}"),
                    f => format!(
                        "MATLAB:{}:{id}",
                        f.unwrap_or_else(|| "validateattributes".into())
                    ),
                };
                return Err(self.raise(format!("{prefix} to be {description}."), identifier));
            }
        }
        Ok(())
    }
    fn lng_diagonal(&self, a: &Array) -> Result<bool> {
        for i in 0..a.len() {
            if i.is_multiple_of(4096) {
                self.check_cancel()?;
            }
            if i % a.rows().max(1) != i / a.rows().max(1) && self.lng_element(a, i)? != 0. {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn lng_element(&self, a: &Array, index: usize) -> Result<f64> {
        match a {
            Array::Numeric(m) => Ok(m.data()[index]),
            Array::Logical(m) => Ok(f64::from(m.data()[index])),
            Array::Typed(m) => Ok(m.real(index)),
            Array::Character(m) => Ok(f64::from(m.units()[index])),
            _ => Err("validateattributes: value predicates require real numeric, logical or character data.".into()),
        }
    }
    fn lng_all(&self, a: &Array, predicate: impl Fn(f64) -> bool) -> Result<bool> {
        for i in 0..a.len() {
            if i.is_multiple_of(4096) {
                self.check_cancel()?;
            }
            if !predicate(self.lng_element(a, i)?) {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn lng_ordered(
        &self,
        a: &Array,
        predicate: impl Fn(std::cmp::Ordering) -> bool,
    ) -> Result<bool> {
        for i in 1..a.len() {
            if i.is_multiple_of(4096) {
                self.check_cancel()?;
            }
            let order = match a {
                Array::Typed(m) if m.is_integer() => Some(m.int(i - 1).cmp(&m.int(i))),
                _ => self
                    .lng_element(a, i - 1)?
                    .partial_cmp(&self.lng_element(a, i)?),
            };
            if !order.is_some_and(&predicate) {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

#[derive(Clone, Default)]
pub(super) struct LngWorkspace {
    pub numeric: BTreeMap<String, Matrix>,
    pub logical: BTreeMap<String, LogicalMatrix>,
    pub complex: BTreeMap<String, ComplexMatrix>,
    pub containers: BTreeMap<String, Array>,
}
#[derive(Clone, Default)]
struct LngContext {
    globals: BTreeSet<String>,
    persistent: BTreeSet<String>,
    /// `path::name` of the running function; empty in the base workspace.
    function: String,
    /// `path::name::hash` of the definition text, set by the first persistent
    /// declaration: an edited function starts with fresh persistent variables.
    persistent_key: Option<String>,
    input_names: Vec<String>,
    arguments: Option<(usize, usize)>,
}
#[derive(Clone)]
struct LngCleanup {
    task: Array,
    /// The task slot is shared by all copies of the cleanup object, including
    /// values temporarily held by the evaluator while it computes later inputs.
    owner: Weak<Array>,
}
#[derive(Clone, Default)]
pub(super) struct LngState {
    globals: BTreeMap<String, Array>,
    persistent: BTreeMap<String, BTreeMap<String, Array>>,
    frames: Vec<(LngWorkspace, LngContext)>,
    context: LngContext,
    /// Argument names of the call about to enter a user function.
    pub pending_names: Vec<String>,
    /// Argument names of the builtin call being dispatched (celldisp).
    pub call_names: Vec<String>,
    pub last_error: (String, String),
    pub last_warning: (String, String),
    pub thrown_stack: Option<StructArray>,
    pub thrown_object: Option<StructArray>,
    cleanup: BTreeMap<u64, LngCleanup>,
    next_id: u64,
}

impl LngState {
    /// Stored workspaces and cleanup callbacks are roots for handle objects.
    pub(super) fn obj_roots(&self) -> impl Iterator<Item = &Array> {
        self.globals
            .values()
            .chain(self.persistent.values().flat_map(|values| values.values()))
            .chain(
                self.frames
                    .iter()
                    .flat_map(|(workspace, _)| workspace.containers.values()),
            )
            .chain(self.cleanup.values().map(|cleanup| &cleanup.task))
    }
}

fn lng_number(n: f64) -> Value {
    Value::Matrix(Matrix::scalar(n))
}
fn lng_arity(args: &[Value], min: usize, max: usize) -> Result<()> {
    if args.len() < min {
        Err(FEW.into())
    } else if args.len() > max {
        Err(MANY.into())
    } else {
        Ok(())
    }
}
fn lng_tag(s: &StructArray) -> Option<&'static str> {
    let Array::Numeric(n) = s.get(s.field_index(TAG)?, 0).ok()?.as_ref() else {
        return None;
    };
    match n.data() {
        [1.] => Some("MException"),
        [2.] => Some("onCleanup"),
        [3.] => Some("inline"),
        _ => None,
    }
}
/// Field names a language object shows; the facade's tag fields stay hidden.
fn lng_public_fields(s: &StructArray) -> Vec<String> {
    s.field_names()
        .iter()
        .filter(|n| {
            !matches!(
                n.as_str(),
                TAG | OBJECT_ID | CAUGHT | super::objects::OBJ_REPORT_HEADER
            )
        })
        .cloned()
        .collect()
}
/// class() of a value, naming the struct-backed language objects.
pub(super) fn lng_class(a: &Array) -> &'static str {
    match a {
        Array::Struct(s) => lng_tag(s).unwrap_or("struct"),
        a => a.class(),
    }
}
fn lng_id(s: &StructArray) -> Option<u64> {
    let Array::Numeric(n) = s.get(s.field_index(OBJECT_ID)?, 0).ok()?.as_ref() else {
        return None;
    };
    n.data().first().map(|n| *n as u64)
}

impl LngWorkspace {
    fn lng_put(&mut self, name: String, a: Array) {
        self.numeric.remove(&name);
        self.logical.remove(&name);
        self.complex.remove(&name);
        self.containers.remove(&name);
        match a {
            Array::Numeric(m) => {
                self.numeric.insert(name, m);
            }
            Array::Logical(m) => {
                self.logical.insert(name, m);
            }
            Array::Complex(m) => {
                self.complex.insert(name, m);
            }
            a => {
                self.containers.insert(name, a);
            }
        }
    }
    fn lng_remove(&mut self, name: &str) {
        self.numeric.remove(name);
        self.logical.remove(name);
        self.complex.remove(name);
        self.containers.remove(name);
    }
}

impl Engine {
    pub(super) fn lng_enter(
        &mut self,
        caller: LngWorkspace,
        function: String,
        input_names: Vec<String>,
    ) {
        let mut context = std::mem::replace(
            &mut self.functions.lng.context,
            LngContext {
                function,
                input_names,
                ..Default::default()
            },
        );
        context.arguments = self.functions.arguments;
        self.lg3_enter(&self.functions.lng.context.function.clone());
        self.functions.lng.frames.push((caller, context));
    }
    /// Names held in the global store (declared anywhere), for `clearvars -global`.
    pub(super) fn lg3_global_names(&self) -> Vec<String> {
        self.functions.lng.globals.keys().cloned().collect()
    }
    /// Clear already selected variables without resolving the name `clear`.
    pub(super) fn lg3_clear_variables(&mut self, names: &[String], global: bool) -> Result<()> {
        if !global {
            self.lng_sync()?;
        }
        for n in names {
            self.check_cancel()?;
            if global {
                self.functions.lng.globals.remove(n);
                if self.functions.lng.context.globals.remove(n) {
                    self.remove_array(n);
                }
                for (frame, context) in &mut self.functions.lng.frames {
                    if context.globals.remove(n) {
                        frame.lng_remove(n);
                    }
                }
            } else {
                self.remove_array(n);
                self.functions.lng.context.globals.remove(n);
                if self.functions.lng.context.persistent.remove(n)
                    && let Some(key) = &self.functions.lng.context.persistent_key
                    && let Some(saved) = self.functions.lng.persistent.get_mut(key)
                {
                    saved.remove(n);
                }
            }
        }
        Ok(())
    }
    pub(super) fn lng_sync(&mut self) -> Result<()> {
        if self.functions.lng.context.globals.is_empty()
            && self.functions.lng.context.persistent.is_empty()
        {
            return Ok(());
        }
        let globals: Vec<_> = self
            .functions
            .lng
            .context
            .globals
            .iter()
            .filter_map(|n| self.array(n).map(|v| (n.clone(), v)))
            .collect();
        let persistent: Vec<_> = self
            .functions
            .lng
            .context
            .persistent
            .iter()
            .filter_map(|n| self.array(n).map(|v| (n.clone(), v)))
            .collect();
        let retained = &self.functions.lng.globals;
        let saved = &self.functions.lng.persistent;
        let mut count = retained.len() + saved.values().map(BTreeMap::len).sum::<usize>();
        let mut bytes = retained
            .values()
            .chain(saved.values().flat_map(BTreeMap::values))
            .map(Array::retained_bytes)
            .sum::<usize>();
        let local_saved = self
            .functions
            .lng
            .context
            .persistent_key
            .as_ref()
            .and_then(|key| saved.get(key));
        for (name, value) in &globals {
            self.check_cancel()?;
            let old = retained.get(name);
            count += usize::from(old.is_none());
            bytes = bytes - old.map_or(0, Array::retained_bytes) + value.retained_bytes();
        }
        for (name, value) in &persistent {
            self.check_cancel()?;
            let old = local_saved.and_then(|m| m.get(name));
            count += usize::from(old.is_none());
            bytes = bytes - old.map_or(0, Array::retained_bytes) + value.retained_bytes();
        }
        if count > 10000 || bytes > 256 * 1024 * 1024 {
            return Err(
                "Retained global/persistent workspace exceeds 256 MiB or 10000 variables".into(),
            );
        }
        self.functions.lng.globals.extend(globals);
        if !persistent.is_empty()
            && let Some(key) = self.functions.lng.context.persistent_key.clone()
        {
            self.functions
                .lng
                .persistent
                .entry(key)
                .or_default()
                .extend(persistent);
        }
        Ok(())
    }
    pub(super) fn lng_leave(
        &mut self,
        out: &mut Execution,
        keep: &[Value],
    ) -> (LngWorkspace, Result<()>) {
        let sync = self.lng_sync();
        self.clear_arrays();
        let cleanup = self.lng_sweep_with(out, keep);
        let (mut caller, context) = self.functions.lng.frames.pop().expect("function frame");
        self.lg3_leave();
        for name in &context.globals {
            if let Some(a) = self.functions.lng.globals.get(name) {
                caller.lng_put(name.clone(), a.clone());
            }
        }
        if let Some(saved) = context
            .persistent_key
            .as_ref()
            .and_then(|key| self.functions.lng.persistent.get(key))
        {
            for name in &context.persistent {
                if let Some(a) = saved.get(name) {
                    caller.lng_put(name.clone(), a.clone());
                }
            }
        }
        self.functions.lng.context = context;
        (caller, sync.and(cleanup))
    }
    pub(super) fn lng_varargout(
        &self,
        names: &[String],
        count: usize,
        targets: Option<&[Option<String>]>,
    ) -> Result<Vec<Value>> {
        let fixed = names.len() - 1;
        if count == 0 {
            // A statement call returns its first output when one was set.
            let first = match (fixed, self.array("varargout")) {
                (0, Some(Array::Cell(c))) if !c.is_empty() => {
                    Some(c.get_linear(0)?.as_ref().clone())
                }
                (0, _) => None,
                _ => self.array(&names[0]),
            };
            return Ok(first.map(Value::from).into_iter().collect());
        }
        let mut values = Vec::new();
        for (i, name) in names.iter().take(fixed.min(count)).enumerate() {
            values.push(if targets.is_some_and(|t| t[i].is_none()) {
                Value::Void
            } else {
                Value::from(
                    self.array(name)
                        .ok_or_else(|| format!("Output '{name}' was not assigned"))?,
                )
            });
        }
        if count > fixed {
            let Some(Array::Cell(c)) = self.array("varargout") else {
                return Err("varargout must be a cell array.".into());
            };
            if c.len() < count - fixed {
                return Err("Too many output arguments.".into());
            }
            values.extend(
                c.lng_contents()
                    .take(count - fixed)
                    .map(|v| Value::from(v.as_ref().clone())),
            );
        }
        Ok(values)
    }
    pub(super) fn lng_declare(
        &mut self,
        persistent: bool,
        names: &[String],
        out: &mut Execution,
    ) -> Result<()> {
        if persistent && self.functions.arguments.is_none() {
            return Err("A PERSISTENT declaration is only allowed in a function.".into());
        }
        let key = if persistent {
            self.lng_persistent_key()
        } else {
            String::new()
        };
        for name in names {
            self.check_cancel()?;
            let exists = if persistent {
                &self.functions.lng.context.persistent
            } else {
                &self.functions.lng.context.globals
            };
            if exists.contains(name) {
                continue;
            }
            let saved = if persistent {
                self.functions
                    .lng
                    .persistent
                    .get(&key)
                    .and_then(|v| v.get(name))
                    .cloned()
            } else {
                self.functions.lng.globals.get(name).cloned()
            };
            let local = self.array(name);
            if !persistent && local.is_some() {
                self.lng_warn("The value of local variables may have been changed to match the globals.  Future versions of\nMATLAB will require that you declare a variable to be global before you use that variable.", "MATLAB:declareGlobalBeforeUse", out)?;
            }
            let value = saved
                .or(local)
                .unwrap_or(Matrix::from_column_vec(0, 0, Vec::new())?.into());
            self.assign_workspace_array(name, value, true, out)?;
            if persistent {
                self.functions.lng.context.persistent.insert(name.clone());
            } else {
                self.functions.lng.context.globals.insert(name.clone());
            }
        }
        self.lng_sync()
    }
    fn lng_persistent_key(&mut self) -> String {
        if let Some(key) = &self.functions.lng.context.persistent_key {
            return key.clone();
        }
        let text = self.functions.lng_definition().unwrap_or_default();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(text, &mut hasher);
        let function = &self.functions.lng.context.function;
        let key = format!("{function}::{:016x}", std::hash::Hasher::finish(&hasher));
        let prefix = format!("{function}::");
        self.functions
            .lng
            .persistent
            .retain(|k, _| !k.starts_with(&prefix) || *k == key);
        self.functions.lng.context.persistent_key = Some(key.clone());
        key
    }
    /// Record lasterr and, inside a function, the stack where the error arose.
    pub(super) fn lng_note_failure(&mut self, message: &str) {
        self.functions.lng.last_error = self.failure_text(message);
        if self.functions.lng.thrown_stack.is_none() && !self.functions.lng.frames.is_empty() {
            self.functions.lng.thrown_stack = self.lng_stack(false).ok();
        }
    }
    /// A new statement starts with no error in flight.
    pub(super) fn lng_statement_start(&mut self) {
        self.functions.lng.thrown_stack = None;
        self.functions.lng.thrown_object = None;
    }
    fn lng_warn(&mut self, message: &str, identifier: &str, out: &mut Execution) -> Result<()> {
        self.lang_utils_call(
            "warning",
            &[Value::Text(identifier.into()), Value::Text(message.into())],
            out,
        )?;
        Ok(())
    }
    pub(super) fn lng_statement(
        &mut self,
        e: &syntax::Evaluation,
        out: &mut Execution,
    ) -> Result<bool> {
        if e.targets.is_empty()
            && e.list_target.is_none()
            && e.path_target.is_none()
            && e.indexed_target.is_none()
            && let Expr::Name(name) = &e.expr
            && let Some(a) = self.array(name)
        {
            if !e.quiet {
                self.emit(out, self.display_text(name, &a)?)?;
            }
            return Ok(true);
        }
        if !e.targets.is_empty()
            || e.path_target.is_some()
            || e.indexed_target.is_some()
            || e.list_target.is_some()
        {
            return Ok(false);
        }
        let (name, args) = match &e.expr {
            Expr::Name(n) => (n, &[][..]),
            Expr::Call(n, a) => (n, a.as_slice()),
            _ => return Ok(false),
        };
        if self.contains_array(name) {
            return Ok(false);
        }
        match name.as_str() {
            "clear" => {
                // A function or folder script named clear keeps precedence, and
                // non-literal arguments keep the variable-clearing diagnostic.
                if self.resolve_function(name)?.is_some()
                    || (args.is_empty() && self.resolve_script(name)?.is_some())
                {
                    return Ok(false);
                }
                let Some(names) = args
                    .iter()
                    .map(|e| match e {
                        Expr::String(n) | Expr::DoubleQuoted(n) => Some(n.clone()),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()
                else {
                    return Ok(false);
                };
                if names.len() > 255 {
                    return Err("clear supports at most 255 literal variable names".into());
                }
                if names.first().is_some_and(|n| n == "global") && !self.contains_array("global") {
                    let selected: Vec<_> = if names.len() == 1 {
                        self.functions.lng.globals.keys().cloned().collect()
                    } else {
                        names[1..].to_vec()
                    };
                    for n in &selected {
                        Self::validate_workspace_name(n)?;
                    }
                    for n in selected {
                        self.functions.lng.globals.remove(&n);
                        if self.functions.lng.context.globals.remove(&n) {
                            self.remove_array(&n);
                        }
                        for (frame, context) in &mut self.functions.lng.frames {
                            if context.globals.remove(&n) {
                                frame.lng_remove(&n);
                            }
                        }
                    }
                    return Ok(true);
                }
                for n in &names {
                    Self::validate_workspace_name(n)?;
                    if !self.contains_array(n)
                        && matches!(
                            n.as_str(),
                            "all" | "functions" | "classes" | "import" | "java" | "mex"
                        )
                    {
                        return Err(format!(
                            "clear {n} is unsupported; select explicit variables or a function name"
                        ));
                    }
                    if !self.contains_array(n) && n == "variables" && names.len() != 1 {
                        return Err("clear variables must be used alone unless variables is a workspace variable".into());
                    }
                }
                let mut variable_args = Vec::new();
                for (arg, n) in args.iter().zip(&names) {
                    let function =
                        !self.contains_array(n) && self.functions.contains_retained_function(n);
                    if !self.contains_array(n) {
                        self.functions.lng_forget(n);
                        self.functions
                            .lng
                            .persistent
                            .retain(|key, _| key.rsplit("::").nth(1) != Some(n.as_str()));
                    }
                    if !function {
                        variable_args.push(arg.clone());
                    }
                }
                self.lng_sync()?;
                if args.is_empty() || !variable_args.is_empty() {
                    self.clear_variables_command(&variable_args)?;
                }
                self.functions.lng.context.globals.retain(|n| {
                    self.variables.contains_key(n)
                        || self.logical_variables.contains_key(n)
                        || self.complex_variables.contains_key(n)
                        || self.container_variables.contains_key(n)
                });
                let removed: Vec<_> = self
                    .functions
                    .lng
                    .context
                    .persistent
                    .iter()
                    .filter(|n| !self.contains_array(n))
                    .cloned()
                    .collect();
                for n in removed {
                    self.functions.lng.context.persistent.remove(&n);
                    if let Some(key) = &self.functions.lng.context.persistent_key
                        && let Some(saved) = self.functions.lng.persistent.get_mut(key)
                    {
                        saved.remove(&n);
                    }
                }
                Ok(true)
            }
            "who" | "evalin" | "lasterr" | "lastwarn" => {
                if self.resolve_function(name)?.is_some() {
                    return Ok(false);
                }
                let values = self.function_arguments(args, None, out)?;
                match name.as_str() {
                    "who" => {
                        let v = self.lng_who(&values)?;
                        let names = self.lng_strings(&v)?;
                        if !names.is_empty() {
                            self.emit(
                                out,
                                format!(
                                    "\nYour variables are:\n\n{}\n",
                                    names.iter().map(|n| format!("{n}  ")).collect::<String>()
                                ),
                            )?;
                        }
                    }
                    "evalin" => {
                        self.lng_in_workspace(&values, false, false, out)?;
                    }
                    _ => {
                        if values.is_empty() {
                            let value = self.lng_outputs(name, &values, 1, out)?.remove(0);
                            self.assign_workspace_array(
                                "ans",
                                value.array(&self.cancel)?,
                                e.quiet,
                                out,
                            )?;
                        } else {
                            self.lng_outputs(name, &values, 0, out)?;
                        }
                    }
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }
    fn lng_swap_frame(&mut self, index: usize) {
        let arguments = self.functions.arguments;
        let (w, c) = &mut self.functions.lng.frames[index];
        std::mem::swap(&mut self.variables, &mut w.numeric);
        std::mem::swap(&mut self.logical_variables, &mut w.logical);
        std::mem::swap(&mut self.complex_variables, &mut w.complex);
        std::mem::swap(&mut self.container_variables, &mut w.containers);
        std::mem::swap(&mut self.functions.lng.context, c);
        self.functions.arguments = self.functions.lng.context.arguments;
        c.arguments = arguments;
    }
    /// Assigning `object.name = ...` to a language object is an error: its
    /// public properties are read-only and it has no others.
    pub(super) fn lng_check_field_assign(&self, current: &Option<Array>, name: &str) -> Result<()> {
        let Some(Array::Struct(object)) = current else {
            return Ok(());
        };
        let Some(class) = lng_tag(object) else {
            return Ok(());
        };
        Err(if lng_public_fields(object).iter().any(|n| n == name) {
            format!("You cannot set the read-only property '{name}' of {class}.")
        } else {
            format!("No public property {name} exists for class {class}.")
        })
    }
    /// Echo (`label = ...`) or, without a label, disp text of a language
    /// object: only its public fields show. None for any other value.
    pub(super) fn lng_object_text(&self, label: Option<&str>, a: &Array) -> Result<Option<String>> {
        let Array::Struct(object) = a else {
            return Ok(None);
        };
        let Some(class) = lng_tag(object) else {
            return Ok(None);
        };
        if class == "inline" {
            return self.lg3_inline_display(label, a).map(Some);
        }
        let names = lng_public_fields(object);
        let slots = names
            .iter()
            .map(|n| object.get(object.field_index(n).unwrap(), 0).cloned())
            .collect::<Result<Vec<_>>>()?;
        let fields = Array::Struct(StructArray::from_slots(1, 1, names, slots, &self.cancel)?);
        let header = format!("  {class} with properties:\n\n");
        let Some(label) = label else {
            let body = crate::display_format::disp(&fields, &self.display)?.unwrap_or_default();
            return Ok(Some(header + &body));
        };
        let text = crate::display_format::echo(label, &fields, &self.display)?;
        let marker = if self.display.compact {
            format!("{label} =\n")
        } else {
            format!("\n{label} = \n\n")
        };
        Ok(Some(text.replacen(&marker, &(marker.clone() + &header), 1)))
    }
    fn lng_in_workspace(
        &mut self,
        args: &[Value],
        assign: bool,
        requested: bool,
        out: &mut Execution,
    ) -> Result<Value> {
        lng_arity(args, if assign { 3 } else { 2 }, 3)?;
        let scope = self.lng_text(&args[0])?;
        if !matches!(scope.as_str(), "base" | "caller") {
            return Err("Unknown command option.".into());
        }
        let name = self.lng_text(&args[1])?;
        if assign && (!crate::files::valid_name(&name) || syntax::keyword(&name)) {
            return Err(format!("Invalid variable name \"{name}\" in ASSIGNIN."));
        }
        self.lng_sync()?;
        let index = if self.functions.lng.frames.is_empty() {
            None
        } else {
            Some(if scope == "base" {
                0
            } else {
                self.functions.lng.frames.len() - 1
            })
        };
        if let Some(index) = index {
            let lng = &mut self.functions.lng;
            let (workspace, context) = &mut lng.frames[index];
            for name in &context.globals {
                if let Some(a) = lng.globals.get(name) {
                    workspace.lng_put(name.clone(), a.clone());
                }
            }
            self.lng_swap_frame(index);
        }
        let result = if assign {
            args[2]
                .clone()
                .array(&self.cancel)
                .and_then(|a| self.assign_workspace_array(&name, a, true, out))
                .map(|()| Value::Void)
        } else {
            self.langx_eval(&args[1..], requested, out)
        };
        let sync = self.lng_sync();
        if let Some(index) = index {
            self.lng_swap_frame(index);
        }
        let refresh: Vec<_> = self
            .functions
            .lng
            .context
            .globals
            .iter()
            .filter_map(|n| {
                self.functions
                    .lng
                    .globals
                    .get(n)
                    .map(|a| (n.clone(), a.clone()))
            })
            .collect();
        let refreshed = refresh
            .into_iter()
            .try_for_each(|(n, a)| self.assign_workspace_array(&n, a, true, out));
        result.and_then(|v| sync.and(refreshed).map(|()| v))
    }
    pub(super) fn lng_sweep_cleanup(&mut self, out: &mut Execution) -> Result<()> {
        self.lng_sweep_with(out, &[])
    }
    fn lng_sweep_with(&mut self, out: &mut Execution, keep: &[Value]) -> Result<()> {
        // Copies of a cleanup object share its task slot, so while every slot
        // is alive nothing can be released: O(live objects) per statement.
        if self
            .functions
            .lng
            .cleanup
            .values()
            .all(|entry| entry.owner.strong_count() > 0)
        {
            return Ok(());
        }
        fn lng_visit(
            a: &Array,
            ids: &mut BTreeSet<u64>,
            owners: &mut Vec<(u64, Weak<Array>)>,
            depth: usize,
            cancel: &std::sync::atomic::AtomicBool,
        ) -> Result<()> {
            crate::dsp::check_cancel(cancel)?;
            if depth > 128 {
                return Err("Cleanup references exceed 128 levels".into());
            }
            match a {
                Array::Struct(s) => {
                    if lng_tag(s) == Some("onCleanup")
                        && let Some(id) = lng_id(s)
                    {
                        ids.insert(id);
                        if let Some(task) = s.field_index("task").and_then(|f| s.get(f, 0).ok()) {
                            owners.push((id, Arc::downgrade(task)));
                        }
                    }
                    for e in 0..s.len() {
                        for f in 0..s.field_count() {
                            if let Ok(a) = s.get(f, e) {
                                lng_visit(a, ids, owners, depth + 1, cancel)?;
                            }
                        }
                    }
                }
                Array::Cell(c) => {
                    for a in c.lng_contents() {
                        lng_visit(a, ids, owners, depth + 1, cancel)?;
                    }
                }
                Array::Handle(h) => {
                    if let Some((_, captures)) = h.anonymous_parts() {
                        for (_, a) in captures {
                            lng_visit(a, ids, owners, depth + 1, cancel)?;
                        }
                    }
                }
                _ => (),
            }
            Ok(())
        }
        loop {
            let mut owners = Vec::new();
            let mut ids: BTreeSet<_> = self
                .functions
                .lng
                .cleanup
                .iter()
                .filter(|(_, entry)| entry.owner.strong_count() > 0)
                .map(|(id, _)| *id)
                .collect();
            for a in self
                .container_variables
                .values()
                .chain(
                    self.functions
                        .lng
                        .frames
                        .iter()
                        .flat_map(|(w, _)| w.containers.values()),
                )
                .chain(self.functions.lng.globals.values())
                .chain(
                    self.functions
                        .lng
                        .persistent
                        .values()
                        .flat_map(BTreeMap::values),
                )
            {
                lng_visit(a, &mut ids, &mut owners, 0, &self.cancel)?;
            }
            for v in keep {
                if let Value::Container(a) = v {
                    lng_visit(a, &mut ids, &mut owners, 0, &self.cancel)?;
                }
            }
            let mut visited = BTreeSet::new();
            while let Some(id) = ids.iter().find(|id| !visited.contains(*id)).copied() {
                visited.insert(id);
                if let Some(entry) = self.functions.lng.cleanup.get(&id) {
                    lng_visit(&entry.task, &mut ids, &mut owners, 0, &self.cancel)?;
                }
            }
            // A reachable copy with its own task slot becomes the owner, so the
            // fast path holds again on later statements.
            for (id, owner) in owners {
                if let Some(entry) = self.functions.lng.cleanup.get_mut(&id)
                    && entry.owner.strong_count() == 0
                {
                    entry.owner = owner;
                }
            }
            let removed: Vec<_> = self
                .functions
                .lng
                .cleanup
                .keys()
                .copied()
                .filter(|id| !ids.contains(id))
                .collect();
            if removed.is_empty() {
                break;
            }
            for id in removed {
                let Some(entry) = self.functions.lng.cleanup.remove(&id) else {
                    continue;
                };
                if let Array::Handle(h) = entry.task {
                    self.count_step()?;
                    // A failure may be in flight: the task must not consume it.
                    let raised = self.raised.take();
                    let stack = self.functions.lng.thrown_stack.take();
                    let object = self.functions.lng.thrown_object.take();
                    let result = self.call_handle(&h, vec![], 0, None, out);
                    self.raised = raised;
                    self.functions.lng.thrown_stack = stack;
                    self.functions.lng.thrown_object = object;
                    result?;
                }
            }
        }
        Ok(())
    }
}

impl Engine {
    fn lng_text(&self, v: &Value) -> Result<String> {
        match v {
            Value::Text(s) => Ok(s.clone()),
            Value::Container(Array::Character(c)) => c.to_utf8(&self.cancel),
            _ => Err("Argument must contain a string.".into()),
        }
    }
    fn lng_char(&self, s: &str) -> Result<Array> {
        Ok(CharacterRow::from_text(s, &self.cancel)?.into())
    }
    fn lng_scalar(&self, v: &Value) -> Result<f64> {
        let a = v.clone().matrix(&self.cancel)?;
        if a.len() != 1 {
            return Err("Input must be a scalar.".into());
        }
        Ok(a.data()[0])
    }
    fn lng_cell(&self, rows: usize, cols: usize, values: Vec<Array>) -> Result<Value> {
        Ok(Value::Container(
            CellArray::from_shared(
                rows,
                cols,
                values.into_iter().map(Arc::new).collect(),
                &self.cancel,
            )?
            .into(),
        ))
    }
    fn lng_strings(&self, value: &Value) -> Result<Vec<String>> {
        match value.clone().array(&self.cancel)? {
            Array::Cell(c) => c
                .lng_contents()
                .map(|a| self.lng_text(&Value::from(a.as_ref().clone())))
                .collect(),
            Array::Character(c) => (0..c.rows())
                .map(|r| {
                    let row: Vec<u16> = (0..c.cols())
                        .map(|col| c.units()[r + col * c.rows()])
                        .collect();
                    Ok(String::from_utf16_lossy(&row).trim_end().to_owned())
                })
                .collect(),
            _ => Err("Requires character array or cell array of strings as inputs.".into()),
        }
    }
    fn lng_string_cell(&self, names: Vec<String>, empty_column: bool) -> Result<Value> {
        let n = names.len();
        let data = names
            .iter()
            .map(|s| self.lng_char(s))
            .collect::<Result<_>>()?;
        self.lng_cell(n, usize::from(n > 0 || empty_column), data)
    }
    pub(super) fn lng_eval_outputs(
        &mut self,
        expr: &Expr,
        targets: &[Option<String>],
        out: &mut Execution,
    ) -> Result<Option<Vec<Value>>> {
        let (Expr::Call(name, _) | Expr::Name(name)) = expr else {
            return Ok(None);
        };
        if !matches!(
            name.as_str(),
            "evalc" | "lasterr" | "lastwarn" | "sscanf" | "strread" | "textread"
        ) || self.contains_array(name)
            || self.resolve_function(name)?.is_some()
        {
            return Ok(None);
        }
        let args = if let Expr::Call(_, args) = expr {
            args.as_slice()
        } else {
            &[]
        };
        let args = self.function_arguments(args, None, out)?;
        let values = self.lng_outputs(name, &args, targets.len(), out)?;
        self.charge_function_outputs(&values)?;
        Ok(Some(values))
    }
    pub(super) fn lng_call(
        &mut self,
        name: &str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        self.check_cancel()?;
        let value = match name {
            "evalc" | "lasterr" | "lastwarn" | "sscanf" | "strread" | "textread" => self.lng_outputs(name, args, 1, out)?.remove(0),
            "evalin" => self.lng_in_workspace(args, false, true, out)?,
            "assignin" => self.lng_in_workspace(args, true, false, out)?,
            "inputname" => {
                lng_arity(args, 1, 1)?;
                if self.functions.arguments.is_none() { return Err("Cannot return input name if not in an active function.".into()); }
                let n = self.lng_scalar(&args[0])?;
                if !n.is_finite() || n < 1. || n.fract() != 0. { return Err("Input must be a positive integer.".into()); }
                if n as usize > self.functions.arguments.unwrap().0 { return Err("Argument number is not valid.".into()); }
                Value::Text(self.functions.lng.context.input_names.get(n as usize - 1).cloned().unwrap_or_default())
            }
            "nargoutchk" | "nargchk" => self.lng_nargchk(name, args, out)?,
            "validateattributes" => { self.lng_validate(args, out)?; Value::Void }
            "MException" => {
                lng_arity(args, 2, LIMIT)?;
                let id = self.lng_text(&args[0])?;
                if !id.is_empty() && (!id.contains(':') || !id.split(':').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))) {
                    return Err("Identifier must be two or more parts separated by ':', each part letters, digits or underscore. See help MException.".into());
                }
                let template = self.lng_text(&args[1])?;
                let text = self.lng_format(&template, &args[2..])?;
                Value::Container(self.lng_exception(&id, &text, false)?)
            }
            "throw" | "throwAsCaller" => {
                lng_arity(args, 1, 1)?;
                let Value::Container(Array::Struct(s)) = &args[0] else { return Err(self.lng_undefined(name, &args[0])?); };
                if lng_tag(s) != Some("MException") { return Err(self.lng_undefined(name, &args[0])?); }
                let message = self.lng_struct_text(s, "message")?;
                let id = self.lng_struct_text(s, "identifier")?;
                self.functions.lng.thrown_stack = Some(self.lng_stack(name == "throwAsCaller")?);
                self.functions.lng.thrown_object = Some(s.clone());
                return Err(self.raise(message, id));
            }
            "rethrow" if args.first().is_some_and(|v| matches!(v, Value::Container(Array::Struct(s)) if lng_tag(s) == Some("MException"))) => {
                lng_arity(args, 1, 1)?;
                let Value::Container(Array::Struct(s)) = &args[0] else { unreachable!() };
                let caught = s.field_index(CAUGHT).is_some();
                if !caught { return Err("RETHROW can only throw a previously caught exception.".into()); }
                let message = self.lng_struct_text(s, "message")?;
                let id = self.lng_struct_text(s, "identifier")?;
                self.functions.lng.thrown_object = Some(s.clone());
                if let Array::Struct(stack) = s.get(s.field_index("stack").ok_or("Missing exception stack")?, 0)?.as_ref() { self.functions.lng.thrown_stack = Some(stack.clone()); }
                return Err(self.raise(message, id));
            }
            "fieldnames" | "struct2cell" | "disp" if args.first().is_some_and(|v| matches!(v, Value::Container(Array::Struct(s)) if lng_tag(s).is_some())) => {
                lng_arity(args, 1, 1)?;
                let Value::Container(a @ Array::Struct(s)) = &args[0] else { unreachable!() };
                match name {
                    "fieldnames" => self.lng_string_cell(lng_public_fields(s), true)?,
                    "struct2cell" => return Err(self.lng_undefined(name, &args[0])?),
                    _ => {
                        let text = self.lng_object_text(None, a)?.unwrap_or_default();
                        self.emit(out, text)?;
                        Value::Void
                    }
                }
            }
            // Language objects are not structs: struct queries see no fields and
            // struct editors do not apply.
            "isstruct" | "isfield" | "numfields" | "rmfield" | "setfield" if args.first().is_some_and(|v| matches!(v, Value::Container(Array::Struct(s)) if lng_tag(s).is_some())) => {
                match name {
                    "isstruct" => { lng_arity(args, 1, 1)?; Value::Logical(LogicalMatrix::scalar(false)) }
                    "numfields" => { lng_arity(args, 1, 1)?; lng_number(0.) }
                    "isfield" => {
                        lng_arity(args, 2, 2)?;
                        let (rows, cols) = match &args[1] { Value::Container(Array::Cell(c)) => (c.rows(), c.cols()), _ => (1, 1) };
                        Value::Logical(LogicalMatrix::filled(rows, cols, false, &self.cancel)?)
                    }
                    _ => return Err(self.lng_undefined(name, &args[0])?),
                }
            }
            "class" | "isa" if args.first().is_some_and(|v| matches!(v, Value::Container(Array::Struct(s)) if lng_tag(s).is_some())) => {
                lng_arity(args, if name == "isa" { 2 } else { 1 }, if name == "isa" { 2 } else { 1 })?;
                let Value::Container(Array::Struct(s)) = &args[0] else { unreachable!() };
                let class = lng_tag(s).unwrap();
                if name == "class" { Value::Text(class.into()) } else { Value::Logical(LogicalMatrix::scalar(self.lng_text(&args[1])? == class)) }
            }
            "addCause" => {
                lng_arity(args, 2, 2)?;
                let (Value::Container(Array::Struct(s)), Value::Container(Array::Struct(cause))) = (&args[0], &args[1]) else { return Err("Inputs must be MException objects.".into()) };
                if lng_tag(s) != Some("MException") || lng_tag(cause) != Some("MException") { return Err("Inputs must be MException objects.".into()); }
                let Some(Array::Cell(c)) = s.field_index("cause").and_then(|f| s.get(f, 0).ok()).map(AsRef::as_ref) else { return Err("The cause property of the MException must be a cell array.".into()) };
                let mut values: Vec<_> = c.lng_contents().map(|a| a.as_ref().clone()).collect();
                values.push(Array::Struct(cause.clone()));
                let cell = self.lng_cell(values.len(), 1, values)?.array(&self.cancel)?;
                Value::Container(Array::Struct(s.with_field("cause", cell, &self.cancel)?))
            }
            "getReport" => {
                lng_arity(args, 1, 1)?;
                let Value::Container(Array::Struct(s)) = &args[0] else { return Err("Input must be an MException.".into()) };
                Value::Text(format!("{}\n", self.lng_struct_text(s, "message")?))
            }
            "properties" => {
                lng_arity(args, 1, 1)?;
                if self.lng_text(&args[0])? != "MException" { return Err("Only MException properties are supported.".into()); }
                self.lng_string_cell(["identifier", "message", "cause", "stack"].map(String::from).to_vec(), true)?
            }
            "onCleanup" => {
                lng_arity(args, 1, 1)?;
                if self.functions.lng.cleanup.len() >= 1024 { return Err("onCleanup exceeds 1024 live tasks".into()); }
                let task = args[0].clone().array(&self.cancel)?;
                self.functions.lng.next_id += 1;
                let id = self.functions.lng.next_id;
                let slot = Arc::new(task.clone());
                let owner = Arc::downgrade(&slot);
                let object = StructArray::from_slots(1, 1, vec!["task".into(), TAG.into(), OBJECT_ID.into()], vec![slot, Arc::new(Matrix::scalar(2.).into()), Arc::new(Matrix::scalar(id as f64).into())], &self.cancel)?;
                self.functions.lng.cleanup.insert(id, LngCleanup { task, owner });
                Value::Container(Array::Struct(object))
            }
            "cell2struct" => self.lng_cell2struct(args)?,
            "struct2cell" => self.lng_struct2cell(args)?,
            "substruct" => self.lng_substruct(args)?,
            "subsref" => self.lng_subsref(args, out)?,
            "celldisp" => {
                lng_arity(args, 1, 2)?;
                let Value::Container(Array::Cell(c)) = &args[0] else { return Err("Must be a cell array.".into()) };
                // The reference labels with inputname(1), or ans for an unnamed input.
                let label = if args.len() == 2 { self.lng_text(&args[1])? } else { self.functions.lng.call_names.first().filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| "ans".into()) };
                self.lng_celldisp(c, &label, 0, out)?;
                Value::Void
            }
            "mat2cell" => self.lng_mat2cell(args, out)?,
            "str2mat" | "setstr" => self.lng_chars(name, args)?,
            "strmatch" => {
                lng_arity(args, 2, 3)?;
                let pattern = self.lng_text(&args[0]).map_err(|_| "Requires character array or cell array of strings as inputs.".to_owned())?;
                let mode = args.get(2).map(|v| self.lng_text(v)).transpose()?;
                if mode.as_ref().is_some_and(|s| s != "exact") { return Err("Third input must be 'exact'.".into()); }
                let exact = mode.is_some();
                let strings = self.lng_strings(&args[1])?;
                let mut hits = Vec::new();
                for (i, s) in strings.iter().enumerate() {
                    if i % 1024 == 0 { self.check_cancel()?; }
                    if if exact { s.trim_end() == pattern.trim_end() } else { s.starts_with(&pattern) } { hits.push((i+1) as f64); }
                }
                Value::Matrix(Matrix::from_column_vec(hits.len(), 1, hits)?)
            }
            "findstr" => {
                lng_arity(args, 2, 2)?;
                let mut a: Vec<_> = self.lng_text(&args[0])?.encode_utf16().collect();
                let mut b: Vec<_> = self.lng_text(&args[1])?.encode_utf16().collect();
                if a.len() < b.len() { std::mem::swap(&mut a, &mut b); }
                let mut hits = Vec::new();
                if !b.is_empty() {
                    for (i, window) in a.windows(b.len()).enumerate() {
                        if i % 1024 == 0 { self.check_cancel()?; }
                        if window == b { hits.push((i+1) as f64); }
                    }
                }
                Value::Matrix(Matrix::from_column_vec(usize::from(!hits.is_empty()), hits.len(), hits)?)
            }
            "regexptranslate" => {
                lng_arity(args, 2, 2)?;
                let mode = self.lng_text(&args[0])?;
                if !matches!(mode.as_str(), "escape" | "wildcard") { return Err(format!("Invalid option for regexptranslate: {mode}.")); }
                self.lng_translate(&args[1], &mode)?
            }
            "isstr" => { lng_arity(args, 1, 1)?; Value::Logical(LogicalMatrix::scalar(args[0].is_character())) }
            "symvar" => {
                lng_arity(args, 1, 1)?;
                let text = if args[0].is_character() { self.lng_text(&args[0])? } else { String::new() };
                let re = regex::Regex::new(r"[A-Za-z][A-Za-z0-9_]*").unwrap();
                let mut names = BTreeSet::new();
                for m in re.find_iter(&text) {
                    self.check_cancel()?;
                    if !matches!(m.as_str(), "i" | "j" | "eps" | "inf" | "Inf" | "NaN" | "nan" | "pi") && !text[m.end()..].trim_start().starts_with('(') && (m.start() == 0 || !text.as_bytes()[m.start()-1].is_ascii_digit()) { names.insert(m.as_str().to_owned()); }
                }
                self.lng_string_cell(names.into_iter().collect(), true)?
            }
            "vectorize" => {
                lng_arity(args, 1, 1)?;
                let text = match &args[0] {
                    Value::Container(Array::Handle(_)) => { let v = self.call("func2str", args.to_vec(), out)?; self.lng_text(&v)?.chars().filter(|c| !c.is_whitespace()).collect() },
                    v if v.is_character() => self.lng_text(v)?,
                    _ => { let v = self.lng_chars("setstr", args)?; self.lng_text(&v)? }
                };
                let mut result = String::new();
                let mut previous = '\0';
                for (i, c) in text.chars().enumerate() {
                    if i % 4096 == 0 { self.check_cancel()?; }
                    if matches!(c, '*' | '/' | '\\' | '^') && previous != '.' { result.push('.'); }
                    result.push(c);
                    previous = c;
                }
                Value::Text(result)
            }
            "who" => self.lng_who(args)?,
            "rehash" => { lng_arity(args, 0, 0)?; self.functions.lng_rehash(); Value::Void }
            _ => return Ok(None),
        };
        Ok(Some(value))
    }
    fn lng_undefined(&self, name: &str, v: &Value) -> Result<String> {
        Ok(format!(
            "Undefined function '{name}' for input arguments of type '{}'.",
            lng_class(&v.clone().array(&self.cancel)?)
        ))
    }
    fn lng_format(&mut self, template: &str, args: &[Value]) -> Result<String> {
        let mut values = vec![Value::Text(template.into())];
        values.extend_from_slice(args);
        let value = self.call("sprintf", values, &mut Execution::default())?;
        self.lng_text(&value)
    }
    fn lng_struct_text(&self, s: &StructArray, name: &str) -> Result<String> {
        self.lng_text(&Value::from(
            s.get(s.field_index(name).ok_or("Missing exception field")?, 0)?
                .as_ref()
                .clone(),
        ))
    }
    pub(super) fn lng_exception(&mut self, id: &str, text: &str, caught: bool) -> Result<Array> {
        let thrown = caught && self.functions.lng.thrown_object.is_some();
        let object = if caught {
            self.functions.lng.thrown_object.take().filter(|object| {
                self.lng_struct_text(object, "identifier")
                    .is_ok_and(|s| s == id)
                    && self
                        .lng_struct_text(object, "message")
                        .is_ok_and(|s| s == text)
            })
        } else {
            None
        };
        // A stack recorded for a different thrown exception is stale.
        if thrown && object.is_none() {
            self.functions.lng.thrown_stack = None;
        }
        let thrown_stack = if caught {
            self.functions.lng.thrown_stack.take()
        } else {
            None
        };
        let stack = match thrown_stack {
            Some(stack) => stack,
            None => StructArray::filled_empty(
                0,
                1,
                vec!["file".into(), "name".into(), "line".into()],
                &self.cancel,
            )?,
        };
        if let Some(object) = object {
            return Ok(Array::Struct(
                object
                    .with_field("stack", Array::Struct(stack), &self.cancel)?
                    .with_field(CAUGHT, Matrix::scalar(1.).into(), &self.cancel)?,
            ));
        }
        let mut fields = vec![
            "identifier".into(),
            "message".into(),
            "cause".into(),
            "stack".into(),
            TAG.into(),
        ];
        let mut slots = vec![
            Arc::new(self.lng_char(id)?),
            Arc::new(self.lng_char(text)?),
            Arc::new(self.lng_cell(0, 0, vec![])?.array(&self.cancel)?),
            Arc::new(Array::Struct(stack)),
            Arc::new(Matrix::scalar(1.).into()),
        ];
        if caught {
            fields.push(CAUGHT.into());
            slots.push(Arc::new(Matrix::scalar(1.).into()));
        }
        Ok(Array::Struct(StructArray::from_slots(
            1,
            1,
            fields,
            slots,
            &self.cancel,
        )?))
    }
    /// Function frames, innermost first, ending with the submitted source.
    /// Top-level code has an empty stack; `caller` drops the innermost frame.
    fn lng_stack(&self, caller: bool) -> Result<StructArray> {
        let lng = &self.functions.lng;
        let mut slots = Vec::new();
        if !lng.frames.is_empty() {
            let contexts =
                std::iter::once(&lng.context).chain(lng.frames.iter().rev().map(|(_, c)| c));
            for context in contexts.skip(usize::from(caller)) {
                self.check_cancel()?;
                let name = context
                    .function
                    .rsplit("::")
                    .next()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("submitted source");
                slots.push(Arc::new(self.lng_char("")?));
                slots.push(Arc::new(self.lng_char(name)?));
                slots.push(Arc::new(Matrix::scalar(1.).into()));
            }
        }
        let fields = vec!["file".into(), "name".into(), "line".into()];
        if slots.is_empty() {
            return StructArray::filled_empty(0, 1, fields, &self.cancel);
        }
        StructArray::from_slots(slots.len() / 3, 1, fields, slots, &self.cancel)
    }

    fn lng_chars(&self, name: &str, args: &[Value]) -> Result<Value> {
        lng_arity(args, 1, if name == "setstr" { 1 } else { 255 })?;
        let mut rows: Vec<Vec<u16>> = Vec::new();
        for v in args {
            self.check_cancel()?;
            let a = v.clone().array(&self.cancel)?;
            if a.len() > LIMIT || a.rows() > LIMIT || a.cols() > LIMIT {
                return Err("Character result exceeds 65536 characters".into());
            }
            let (r, c, units) = match a {
                Array::Character(c) => (c.rows(), c.cols(), c.units().to_vec()),
                Array::Numeric(m) => (
                    m.rows(),
                    m.cols(),
                    m.data()
                        .iter()
                        .map(|n| n.clamp(0., 65535.) as u16)
                        .collect(),
                ),
                Array::Typed(m) => (
                    m.rows(),
                    m.cols(),
                    (0..m.len())
                        .map(|i| m.real(i).clamp(0., 65535.) as u16)
                        .collect(),
                ),
                Array::Logical(m) => (
                    m.rows(),
                    m.cols(),
                    m.data().iter().map(|n| u16::from(*n)).collect(),
                ),
                _ => {
                    return Err(if name == "setstr" {
                        "Conversion to char from cell is not possible."
                    } else {
                        "Inputs must be character arrays."
                    }
                    .into());
                }
            };
            if r == 0 && name == "str2mat" && args.len() > 1 {
                rows.push(Vec::new());
            }
            for row in 0..r {
                rows.push((0..c).map(|col| units[row + col * r]).collect());
            }
        }
        let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
        if rows.len().saturating_mul(cols) > LIMIT {
            return Err("Character result exceeds 65536 characters".into());
        }
        let mut units = Vec::new();
        for col in 0..cols {
            for row in &rows {
                units.push(row.get(col).copied().unwrap_or(32));
            }
        }
        Ok(Value::Container(
            CharacterRow::from_column_major(rows.len(), cols, units, &self.cancel)?.into(),
        ))
    }
    fn lng_translate(&self, value: &Value, mode: &str) -> Result<Value> {
        if let Value::Container(Array::Cell(c)) = value {
            let mut values = Vec::new();
            for v in c.lng_contents() {
                self.check_cancel()?;
                values.push(
                    self.lng_translate(&Value::from(v.as_ref().clone()), mode)?
                        .array(&self.cancel)?,
                );
            }
            return self.lng_cell(c.rows(), c.cols(), values);
        }
        let mut translated = String::new();
        for c in self.lng_text(value)?.chars() {
            match c {
                '*' if mode == "wildcard" => translated.push_str(".*"),
                '?' if mode == "wildcard" => translated.push('.'),
                c if ".*+?^$()[]{}|\\".contains(c) => {
                    translated.push('\\');
                    translated.push(c);
                }
                c => translated.push(c),
            }
        }
        Ok(Value::Text(translated))
    }
    fn lng_cell2struct(&self, args: &[Value]) -> Result<Value> {
        lng_arity(args, 3, 3)?;
        let Value::Container(Array::Cell(c)) = &args[0] else {
            return Err("First input must be a cell array.".into());
        };
        let fields = self.lng_strings(&args[1])?;
        for f in &fields {
            if !crate::struct_array::valid_field_name(f) {
                return Err(format!("Invalid field name \"{f}\""));
            }
        }
        let dim = self.lng_scalar(&args[2])?;
        if dim != 1. && dim != 2. {
            return Err("cell2struct supports dimensions 1 and 2.".into());
        }
        let count = if dim == 1. { c.rows() } else { c.cols() };
        if count != fields.len() {
            return Err(
                "Number of field names must match number of fields in new structure.".into(),
            );
        }
        let n = if dim == 1. { c.cols() } else { c.rows() };
        let mut slots = Vec::new();
        for element in 0..n {
            for field in 0..count {
                self.check_cancel()?;
                slots.push(
                    c.get_linear(if dim == 1. {
                        field + element * c.rows()
                    } else {
                        element + field * c.rows()
                    })?
                    .clone(),
                );
            }
        }
        Ok(Value::Container(Array::Struct(StructArray::from_slots(
            n,
            1,
            fields,
            slots,
            &self.cancel,
        )?)))
    }
    fn lng_struct2cell(&self, args: &[Value]) -> Result<Value> {
        lng_arity(args, 1, 1)?;
        let Value::Container(Array::Struct(s)) = &args[0] else {
            return Err(self.lng_undefined("struct2cell", &args[0])?);
        };
        if s.cols() != 1 {
            return Err("struct2cell: N-D cell arrays are not supported.".into());
        }
        let mut values = Vec::new();
        for element in 0..s.len() {
            for field in 0..s.field_count() {
                self.check_cancel()?;
                values.push(s.get(field, element)?.as_ref().clone());
            }
        }
        self.lng_cell(s.field_count(), s.rows(), values)
    }
    fn lng_substruct(&self, args: &[Value]) -> Result<Value> {
        if args.len() < 2 {
            return Err("SUBSTRUCT takes at least two arguments.".into());
        }
        if !args.len().is_multiple_of(2) {
            return Err("SUBSTRUCT requires pairs of inputs.".into());
        }
        let mut slots = Vec::new();
        for pair in args.chunks(2) {
            self.check_cancel()?;
            slots.push(Arc::new(pair[0].clone().array(&self.cancel)?));
            slots.push(Arc::new(pair[1].clone().array(&self.cancel)?));
        }
        Ok(Value::Container(Array::Struct(StructArray::from_slots(
            1,
            args.len() / 2,
            vec!["type".into(), "subs".into()],
            slots,
            &self.cancel,
        )?)))
    }
    fn lng_subsref(&mut self, args: &[Value], out: &mut Execution) -> Result<Value> {
        lng_arity(args, 2, 2)?;
        let Value::Container(Array::Struct(s)) = &args[1] else {
            return Err("Second input must be a subscript structure.".into());
        };
        let mut value = args[0].clone();
        for i in 0..s.len() {
            self.check_cancel()?;
            let kind = self.lng_text(&Value::from(
                s.get(s.field_index("type").ok_or("Missing type")?, i)?
                    .as_ref()
                    .clone(),
            ))?;
            let sub = s.get(s.field_index("subs").ok_or("Missing subs")?, i)?;
            if kind == "." {
                let field = self.lng_text(&Value::from(sub.as_ref().clone()))?;
                let Value::Container(Array::Struct(current)) = value else {
                    return Err("Dot indexing requires a structure.".into());
                };
                value = Value::from(
                    current
                        .get(
                            current
                                .field_index(&field)
                                .ok_or("Reference to non-existent field.")?,
                            0,
                        )?
                        .as_ref()
                        .clone(),
                );
            } else {
                let Array::Cell(c) = sub.as_ref() else {
                    return Err("Subscripts must be a cell array.".into());
                };
                let mut indices = Vec::new();
                for a in c.lng_contents() {
                    match a.as_ref() {
                        Array::Numeric(m) => indices.push(Expr::Matrix(vec![
                            m.data().iter().copied().map(Expr::Number).collect(),
                        ])),
                        Array::Character(c) if c.to_utf8(&self.cancel)? == ":" => {
                            indices.push(Expr::All)
                        }
                        _ => return Err("Unsupported subscript type.".into()),
                    }
                }
                let a = value.array(&self.cancel)?;
                value = if kind == "()" {
                    self.index_value(Value::from(a), &indices, out)?
                } else if kind == "{}" {
                    self.brace_values(Value::from(a), &indices, out)?
                        .into_iter()
                        .next()
                        .ok_or("Empty subscript")?
                } else {
                    return Err("Invalid subscript type.".into());
                };
            }
        }
        Ok(value)
    }
    fn lng_celldisp(
        &mut self,
        c: &CellArray,
        label: &str,
        depth: usize,
        out: &mut Execution,
    ) -> Result<()> {
        if depth > 32 {
            return Err("celldisp nesting exceeds 32 levels".into());
        }
        for (i, a) in c.lng_contents().enumerate() {
            self.check_cancel()?;
            let index = if c.rows() == 1 || c.cols() == 1 {
                (i + 1).to_string()
            } else {
                format!("{},{}", i % c.rows() + 1, i / c.rows() + 1)
            };
            let name = format!("{label}{{{index}}}");
            if let Array::Cell(nested) = a.as_ref() {
                self.lng_celldisp(nested, &name, depth + 1, out)?;
            } else {
                self.emit(out, format!(" \n{name} =\n "))?;
                self.call("disp", vec![Value::from(a.as_ref().clone())], out)?;
                self.emit(out, " ".into())?;
            }
        }
        Ok(())
    }
    fn lng_mat2cell(&mut self, args: &[Value], out: &mut Execution) -> Result<Value> {
        lng_arity(args, 2, 3)?;
        let a = args[0].clone().array(&self.cancel)?;
        let r = args[1].clone().matrix(&self.cancel)?;
        let c = if args.len() == 3 {
            args[2].clone().matrix(&self.cancel)?
        } else {
            Matrix::scalar(a.cols() as f64)
        };
        for (parts, n) in [(&r, a.rows()), (&c, a.cols())] {
            if parts
                .data()
                .iter()
                .any(|x| !x.is_finite() || *x < 0. || x.fract() != 0.)
                || parts.data().iter().sum::<f64>() != n as f64
            {
                return Err(format!(
                    "Input arguments, D1 through D2, must sum to each dimension of the input matrix size, [{}  {}].'",
                    a.rows(),
                    a.cols()
                ));
            }
        }
        if r.len().saturating_mul(c.len()) > LIMIT {
            return Err("Cell result exceeds 65536 elements".into());
        }
        let mut values = Vec::new();
        let mut col = 1.;
        for &nc in c.data() {
            let mut row = 1.;
            for &nr in r.data() {
                self.check_cancel()?;
                let indices = [
                    Expr::Range(
                        Box::new(Expr::Number(row)),
                        None,
                        Box::new(Expr::Number(row + nr - 1.)),
                    ),
                    Expr::Range(
                        Box::new(Expr::Number(col)),
                        None,
                        Box::new(Expr::Number(col + nc - 1.)),
                    ),
                ];
                values.push(
                    self.index_value(Value::from(a.clone()), &indices, out)?
                        .array(&self.cancel)?,
                );
                row += nr;
            }
            col += nc;
        }
        self.lng_cell(r.len(), c.len(), values)
    }
    fn lng_who(&mut self, args: &[Value]) -> Result<Value> {
        let patterns = args
            .iter()
            .map(|v| self.lng_text(v))
            .collect::<Result<Vec<_>>>()?;
        if patterns
            .iter()
            .enumerate()
            .any(|(i, p)| p.starts_with('-') && !(i == 0 && p == "-file"))
        {
            return Err(
                "who supports wildcard patterns and '-file'; other options are unsupported.".into(),
            );
        }
        let (names, patterns) = if patterns.first().is_some_and(|p| p == "-file") {
            if patterns.len() < 2 {
                return Err(FEW.into());
            }
            let catalog = crate::files::inspect_workspace(
                &self.file_path(&patterns[1], ".mat")?,
                &self.cancel,
            )?;
            (
                catalog
                    .variables()
                    .map(|v| v.name.clone())
                    .collect::<Vec<_>>(),
                &patterns[2..],
            )
        } else {
            (self.workspace().into_keys().collect(), patterns.as_slice())
        };
        let regexes = patterns
            .iter()
            .map(|p| {
                let v = self.lng_translate(&Value::Text(p.clone()), "wildcard")?;
                regex::Regex::new(&format!("^{}$", self.lng_text(&v)?)).map_err(|e| e.to_string())
            })
            .collect::<Result<Vec<_>>>()?;
        self.lng_string_cell(
            names
                .into_iter()
                .filter(|n| regexes.is_empty() || regexes.iter().any(|r| r.is_match(n)))
                .collect(),
            false,
        )
    }
}
