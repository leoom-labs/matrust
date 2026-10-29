//! containers.Map and inputParser. Both are handle objects: a value
//! is a one-field struct whose shared slot names an engine-owned state, so
//! every copy (`m2 = m`, a function argument, a cell element) sees the same
//! state. Unreachable states, including cycles, are collected at safe workspace
//! boundaries; active evaluations retain their temporary handles.
use super::{Engine, Execution, Value};
use crate::{
    Array, CellArray, CharacterRow, FunctionHandle, LogicalMatrix, Matrix, NumericClass, Result,
    StructArray, TypedArray,
    struct_array::valid_field_name,
    syntax::{Evaluation, Expr, PathStep, Token},
};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Weak},
};

const TAG: &str = "MatRustHandle";
pub(super) const OBJ_REPORT_HEADER: &str = "MatRustReportHeader";
const MAX_OBJECTS: usize = 65536;
const MAX_KEYS: usize = crate::script_containers::MAX_CELL_SLOTS;
const MAX_PARAMETERS: usize = 4096;
const MAP: &str = "containers.Map";
const PARSER: &str = "inputParser";
const FEW: &str = "Not enough input arguments.";
const MANY: &str = "Too many input arguments.";
const NO_KEY: &str = "The specified key is not present in this container.";
const NO_KEY_ID: &str = "MATLAB:Containers:Map:NoKey";
const KEY_TYPE: &str = "Specified key type does not match the type expected for this container.";
const VALUE_TYPE: &str =
    "Specified value type does not match the type expected for this container.";
const MISMATCH_ID: &str = "MATLAB:Containers:TypeMismatch";
const MAP_DELETED: &str = "Invalid or deleted containers.Map.";
const PARSER_DELETED: &str = "Invalid or deleted object.";
const SUBSREF: &str = "containers.Map/subsref";
const KEY_TYPES: [&str; 7] = [
    "char", "double", "single", "int32", "uint32", "int64", "uint64",
];
const VALUE_TYPES: [&str; 13] = [
    "any", "char", "double", "single", "int8", "uint8", "int16", "uint16", "int32", "uint32",
    "int64", "uint64", "logical",
];
const MAP_METHODS: [&str; 11] = [
    "keys", "values", "isKey", "remove", "length", "size", "numel", "isempty", "delete", "disp",
    "isvalid",
];
const MAP_ONLY: [&str; 4] = ["keys", "values", "isKey", "remove"];
const PARSER_ONLY: [&str; 6] = [
    "addRequired",
    "addOptional",
    "addParameter",
    "addParamValue",
    "parse",
    "copy",
];
const PARSER_METHODS: [&str; 9] = [
    "addRequired",
    "addOptional",
    "addParameter",
    "addParamValue",
    "parse",
    "copy",
    "delete",
    "disp",
    "isvalid",
];
const PARSER_PROPERTIES: [&str; 9] = [
    "FunctionName",
    "CaseSensitive",
    "KeepUnmatched",
    "PartialMatching",
    "StructExpand",
    "Parameters",
    "Results",
    "Unmatched",
    "UsingDefaults",
];
/// Display stand-ins for objects inside cells and structs: one char per
/// column of `[1x1 class]` minus the two quotes the char display adds.
const MAP_MASK: char = '\u{E000}';
const PARSER_MASK: char = '\u{E001}';
const EMPTY_MASK: char = '\u{E002}';
const EMPTY_CELL: &str = "{1x0 cell}";

#[derive(Clone, Default)]
pub(super) struct ObjTable {
    entries: BTreeMap<u64, ObjEntry>,
    next: u64,
    sweep_at: usize,
    sweep_bytes_at: usize,
    bytes: usize,
}
#[derive(Clone)]
struct ObjEntry {
    slot: Weak<Array>,
    deleted: bool,
    bytes: usize,
    state: ObjState,
}
#[derive(Clone)]
enum ObjState {
    Map(MapState),
    Parser(ParserState),
}
#[derive(Clone)]
struct MapState {
    key: &'static str,
    value: &'static str,
    entries: BTreeMap<MapKey, (Arc<Array>, Arc<Array>)>,
}
/// Keys order as the reference lists them: UTF-16 code units, or numeric value.
#[derive(Clone, Debug)]
enum MapKey {
    Text(Vec<u16>),
    Number(f64),
    Integer(i128),
}
impl Ord for MapKey {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Text(a), Self::Text(b)) => a.cmp(b),
            (Self::Number(a), Self::Number(b)) => a.total_cmp(b),
            (Self::Integer(a), Self::Integer(b)) => a.cmp(b),
            (Self::Text(_), _) | (Self::Number(_), Self::Integer(_)) => Ordering::Less,
            (_, Self::Text(_)) | (Self::Integer(_), Self::Number(_)) => Ordering::Greater,
        }
    }
}
impl PartialOrd for MapKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl PartialEq for MapKey {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for MapKey {}

#[derive(Clone, Copy, PartialEq)]
enum ArgKind {
    Required,
    Optional,
    Parameter,
}
#[derive(Clone)]
struct ParserArg {
    name: String,
    kind: ArgKind,
    default: Option<Arc<Array>>,
    validator: Option<FunctionHandle>,
    priority: f64,
}
#[derive(Clone)]
struct ParserState {
    function_name: String,
    case_sensitive: bool,
    keep_unmatched: bool,
    partial_matching: bool,
    struct_expand: bool,
    /// Definition order.
    args: Vec<ParserArg>,
    /// Sorted by name, as Results shows them.
    results: Vec<(String, Arc<Array>)>,
    using_defaults: Vec<String>,
    unmatched: Vec<(String, Arc<Array>)>,
}
impl ParserState {
    fn new() -> Self {
        Self {
            function_name: String::new(),
            case_sensitive: false,
            keep_unmatched: false,
            partial_matching: true,
            struct_expand: true,
            args: Vec::new(),
            results: Vec::new(),
            using_defaults: Vec::new(),
            unmatched: Vec::new(),
        }
    }
}
/// A name-value argument resolved against the defined parameters.
enum ObjMatch {
    One(usize, Option<String>),
    Ambiguous(String),
}

/// The reference's order for Parameters and Results: case-insensitive, then exact.
fn obj_sort_key(name: &str) -> (String, String) {
    (name.to_ascii_lowercase(), name.to_owned())
}
fn obj_text(a: &Array) -> Option<String> {
    match a {
        Array::Character(c) if c.rows() <= 1 => Some(String::from_utf16_lossy(c.units())),
        _ => None,
    }
}
/// Numeric, logical or integer/single scalar class, if `a` is one.
fn obj_scalar_class(a: &Array) -> Option<&'static str> {
    match a {
        Array::Numeric(m) if m.len() == 1 => Some("double"),
        Array::Logical(m) if m.len() == 1 => Some("logical"),
        Array::Typed(t) if t.len() == 1 => Some(t.class().name()),
        _ => None,
    }
}
fn obj_scalar_real(a: &Array) -> Option<f64> {
    match a {
        Array::Numeric(m) if m.len() == 1 => Some(m.data()[0]),
        Array::Logical(m) if m.len() == 1 => Some(f64::from(m.data()[0])),
        Array::Typed(t) if t.len() == 1 => Some(t.real(0)),
        _ => None,
    }
}
fn obj_numeric(a: &Array) -> bool {
    matches!(a, Array::Numeric(_) | Array::Logical(_) | Array::Typed(_))
}
/// A validator result passes when it is a nonzero real or logical scalar.
fn obj_truthy(v: &Value) -> bool {
    match v {
        Value::Matrix(m) => m.len() == 1 && m.data()[0] != 0. && !m.data()[0].is_nan(),
        Value::Logical(m) => m.len() == 1 && m.data()[0] != 0,
        Value::Container(Array::Typed(t)) => t.len() == 1 && t.is_nonzero(0),
        _ => false,
    }
}
fn obj_operator(op: &Token) -> &'static str {
    match op {
        Token::Plus => "plus",
        Token::Minus => "minus",
        Token::Star => "mtimes",
        Token::DotStar => "times",
        Token::Slash => "mrdivide",
        Token::DotSlash => "rdivide",
        Token::Backslash => "mldivide",
        Token::Power => "mpower",
        Token::DotPower => "power",
        Token::Equal => "eq",
        Token::NotEqual => "ne",
        Token::Less => "lt",
        Token::LessEqual => "le",
        Token::Greater => "gt",
        Token::GreaterEqual => "ge",
        Token::And => "and",
        Token::Or => "or",
        _ => "operator",
    }
}
fn obj_is_containers(e: &Expr) -> bool {
    matches!(e, Expr::Name(n) if n == "containers")
}

impl Engine {
    /// Any containers.Map or inputParser alive (cheap guard for object-only paths).
    pub(super) fn obj_live(&self) -> bool {
        !self.objects.entries.is_empty()
    }
    fn obj_id(&self, a: &Array) -> Option<u64> {
        let Array::Struct(s) = a else { return None };
        if s.len() != 1 || s.field_count() != 1 || s.field_names()[0] != TAG {
            return None;
        }
        let slot = s.get(0, 0).ok()?;
        let Array::Numeric(m) = slot.as_ref() else {
            return None;
        };
        let id = *m.data().first()? as u64;
        let entry = self.objects.entries.get(&id)?;
        (Weak::as_ptr(&entry.slot) == Arc::as_ptr(slot)).then_some(id)
    }
    fn obj_value_id(&self, v: &Value) -> Option<u64> {
        match v {
            Value::Container(a) => self.obj_id(a),
            _ => None,
        }
    }
    fn obj_var(&self, name: &str) -> Option<(u64, Value)> {
        if self.objects.entries.is_empty() {
            return None;
        }
        let a = self.container_variables.get(name)?;
        self.obj_id(a).map(|id| (id, Value::Container(a.clone())))
    }
    fn obj_expr_var(&self, e: &Expr) -> Option<(u64, Value)> {
        match e {
            Expr::Name(n) => self.obj_var(n),
            _ => None,
        }
    }
    fn obj_is_map(&self, id: u64) -> bool {
        matches!(self.objects.entries[&id].state, ObjState::Map(_))
    }
    fn obj_class_name(&self, id: u64) -> &'static str {
        if self.obj_is_map(id) { MAP } else { PARSER }
    }
    /// class() of any value, naming these objects.
    fn obj_class(&self, v: &Value) -> String {
        if let Some(id) = self.obj_value_id(v) {
            return self.obj_class_name(id).into();
        }
        match v {
            Value::Matrix(_) | Value::Complex(_) | Value::Void => "double".into(),
            Value::Logical(_) => "logical".into(),
            Value::Text(_) => "char".into(),
            Value::QuotedString(_) => "string".into(),
            Value::Container(a) => a.class().into(),
        }
    }
    /// Charge each retained object state once, independently of handle aliases.
    pub(super) fn obj_workspace_bytes(&self) -> usize {
        self.objects.bytes
    }
    fn obj_collect(&mut self) {
        if self.objects.entries.is_empty() {
            self.obj_reset_sweep_thresholds();
            return;
        }
        // Expression temporaries are Rust locals, not workspace roots. Do not
        // trace while an expression or a function invocation can own them.
        if self.evaluation_depth != 0 || self.functions.arguments.is_some() {
            self.obj_collect_unowned();
            self.obj_reset_sweep_thresholds();
            return;
        }
        let mut pending: Vec<&Array> = self.container_variables.values().collect();
        pending.extend(self.functions.lng.obj_roots());
        let mut arrays = BTreeSet::new();
        let mut marked = BTreeSet::new();
        while let Some(a) = pending.pop() {
            if self.check_cancel().is_err() {
                return; // Never sweep a partially marked graph.
            }
            if !matches!(a, Array::Cell(_) | Array::Struct(_) | Array::Handle(_)) {
                continue;
            }
            if !arrays.insert(a as *const Array as usize) {
                continue;
            }
            if let Some(id) = self.obj_id(a) {
                if !marked.insert(id) {
                    continue;
                }
                match &self.objects.entries[&id].state {
                    ObjState::Map(m) => {
                        pending.extend(m.entries.values().map(|(_, v)| v.as_ref()));
                    }
                    ObjState::Parser(p) => {
                        for arg in &p.args {
                            pending.extend(arg.default.as_deref());
                            if let Some((_, captured)) = arg
                                .validator
                                .as_ref()
                                .and_then(FunctionHandle::anonymous_parts)
                            {
                                pending.extend(captured.iter().map(|(_, a)| a));
                            }
                        }
                        pending.extend(
                            p.results
                                .iter()
                                .chain(&p.unmatched)
                                .map(|(_, a)| a.as_ref()),
                        );
                    }
                }
            } else {
                match a {
                    Array::Cell(c) => pending.extend(
                        (0..c.len())
                            .filter_map(|i| c.get_linear(i).ok())
                            .map(AsRef::as_ref),
                    ),
                    Array::Struct(s) => pending.extend(
                        (0..s.slot_count())
                            .filter_map(|i| s.slot(i).ok())
                            .map(AsRef::as_ref),
                    ),
                    Array::Handle(h) => {
                        if let Some((_, captured)) = h.anonymous_parts() {
                            pending.extend(captured.iter().map(|(_, a)| a));
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut bytes = self.objects.bytes;
        self.objects.entries.retain(|id, entry| {
            let keep = marked.contains(id);
            if !keep {
                bytes -= entry.bytes;
            }
            keep
        });
        self.objects.bytes = bytes;
        self.obj_reset_sweep_thresholds();
    }
    fn obj_reset_sweep_thresholds(&mut self) {
        self.objects.sweep_at = self.objects.entries.len().saturating_mul(2).max(1024);
        self.objects.sweep_bytes_at = self.objects.bytes.saturating_mul(2).max(64 * 1024 * 1024);
    }
    /// Only call where no active assignment or expression can own a temporary.
    pub(super) fn obj_collect_at_statement(&mut self, force: bool) {
        if force
            || self.objects.entries.len() >= self.objects.sweep_at.max(1024)
            || self.objects.bytes >= self.objects.sweep_bytes_at.max(64 * 1024 * 1024)
        {
            self.obj_collect();
        }
    }
    pub(super) fn obj_collect_unowned(&mut self) {
        let mut bytes = self.objects.bytes;
        self.objects.entries.retain(|_, e| {
            let keep = e.slot.strong_count() > 0;
            if !keep {
                bytes -= e.bytes;
            }
            keep
        });
        self.objects.bytes = bytes;
    }
    fn obj_check_bytes(&mut self, removed: usize, added: usize) -> Result<()> {
        let projected = |engine: &Self| {
            engine
                .workspace_bytes()
                .checked_sub(removed)
                .and_then(|n| n.checked_add(added))
                .ok_or("Workspace byte accounting overflow")
        };
        if projected(self)? > 256 * 1024 * 1024 {
            // Weak ownership remains safe even when Rust locals hold handles.
            self.obj_collect_unowned();
            if projected(self)? > 256 * 1024 * 1024 {
                return Err(
                    "Workspace exceeds its 256 MiB logical storage limit; use clear".into(),
                );
            }
        }
        Ok(())
    }
    fn obj_state_bytes(state: &ObjState) -> usize {
        match state {
            ObjState::Map(m) => m
                .entries
                .iter()
                .map(|(k, (a, v))| Self::obj_entry_bytes(k, a, v))
                .sum(),
            ObjState::Parser(p) => {
                p.function_name.len()
                    + p.using_defaults.iter().map(String::len).sum::<usize>()
                    + p.args
                        .iter()
                        .map(|a| {
                            a.name.len()
                                + a.default.as_ref().map_or(0, |v| v.retained_bytes())
                                + a.validator.as_ref().map_or(0, FunctionHandle::bytes)
                        })
                        .sum::<usize>()
                    + Self::obj_result_bytes(&p.results)
                    + Self::obj_result_bytes(&p.unmatched)
            }
        }
    }
    fn obj_entry_bytes(key: &MapKey, a: &Array, v: &Array) -> usize {
        a.retained_bytes()
            + v.retained_bytes()
            + match key {
                MapKey::Text(t) => t.len() * 2,
                _ => 0,
            }
    }
    fn obj_result_bytes(values: &[(String, Arc<Array>)]) -> usize {
        values
            .iter()
            .map(|(n, v)| n.len() + v.retained_bytes())
            .sum()
    }
    fn obj_new(&mut self, state: ObjState) -> Result<Value> {
        let bytes = Self::obj_state_bytes(&state);
        self.obj_check_bytes(0, bytes)?;
        if self.objects.entries.len() >= MAX_OBJECTS {
            self.obj_collect_unowned();
        }
        let table = &mut self.objects;
        if table.entries.len() >= MAX_OBJECTS {
            return Err(format!(
                "At most {MAX_OBJECTS} containers.Map and inputParser objects can exist"
            ));
        }
        table.next += 1;
        let id = table.next;
        let slot = Arc::new(Array::Numeric(Matrix::scalar(id as f64)));
        let object =
            StructArray::from_slots(1, 1, vec![TAG.into()], vec![slot.clone()], &self.cancel)?;
        self.objects.bytes += bytes;
        self.objects.entries.insert(
            id,
            ObjEntry {
                slot: Arc::downgrade(&slot),
                deleted: false,
                bytes,
                state,
            },
        );
        Ok(Value::Container(Array::Struct(object)))
    }
    /// Raise `message`, remembering the getReport header: None reports the
    /// calling function from the stack, Some(None) the message alone.
    fn obj_fail(&mut self, message: String, id: &str, header: Option<Option<String>>) -> String {
        let message = self.raise(message, id.to_owned());
        self.raised.as_mut().unwrap().object_report = header;
        self.raised.as_mut().unwrap().object_error = true;
        message
    }
    fn obj_warn(&mut self, id: &str, message: &str, out: &mut Execution) -> Result<()> {
        let args = [id, "%s", message].map(|s| Value::Text(s.into())).to_vec();
        self.call("warning", args, out).map(drop)
    }
    fn obj_char(&self, text: &str) -> Result<Array> {
        Ok(Array::Character(CharacterRow::from_text(
            text,
            &self.cancel,
        )?))
    }
    fn obj_cell(&self, rows: usize, cols: usize, items: Vec<Arc<Array>>) -> Result<Array> {
        Ok(Array::Cell(CellArray::from_shared(
            rows,
            cols,
            items,
            &self.cancel,
        )?))
    }
    fn obj_names(&self, names: &[String]) -> Result<Array> {
        let items = names
            .iter()
            .map(|n| self.obj_char(n).map(Arc::new))
            .collect::<Result<Vec<_>>>()?;
        self.obj_cell(1, items.len(), items)
    }
    fn obj_struct(&self, fields: &[(String, Arc<Array>)]) -> Result<Array> {
        let names = fields.iter().map(|(n, _)| n.clone()).collect();
        let slots = fields.iter().map(|(_, v)| v.clone()).collect();
        Ok(Array::Struct(StructArray::from_slots(
            1,
            1,
            names,
            slots,
            &self.cancel,
        )?))
    }
    fn obj_undefined(&self, name: &str, v: Option<&Value>) -> String {
        match v {
            None => FEW.into(),
            Some(v) => format!(
                "Undefined function '{name}' for input arguments of type '{}'.",
                self.obj_class(v)
            ),
        }
    }

    /// Builtin calls: constructors, getReport with options, and every call
    /// whose first argument is one of these objects. None leaves the call to
    /// the other builtins.
    pub(super) fn obj_call(
        &mut self,
        name: &str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        match name {
            "inputParser" => {
                if !args.is_empty() {
                    return Err(MANY.into());
                }
                return self.obj_new(ObjState::Parser(ParserState::new())).map(Some);
            }
            "getReport"
                if args.len() > 1
                    && matches!(args.first(),
                Some(Value::Container(Array::Struct(s))) if s.field_index(OBJ_REPORT_HEADER).is_some()) =>
            {
                return self.obj_report(args).map(Some);
            }
            _ => {}
        }
        let first = args.first().and_then(|v| self.obj_value_id(v));
        if let Some(id) = first {
            return self.obj_method(&args[0], id, name, &args[1..], out);
        }
        if matches!(
            name,
            "keys"
                | "values"
                | "isKey"
                | "remove"
                | "addRequired"
                | "addOptional"
                | "addParameter"
                | "addParamValue"
                | "parse"
        ) {
            return Err(self.raise(
                self.obj_undefined(name, args.first()),
                "MATLAB:UndefinedFunction".into(),
            ));
        }
        if self.objects.entries.is_empty() || !args.iter().any(|v| self.obj_value_id(v).is_some()) {
            return Ok(None);
        }
        match name {
            "isequal" => Ok(Some(Value::Logical(LogicalMatrix::scalar(false)))),
            "eq" | "ne" => Ok(Some(Value::Logical(LogicalMatrix::scalar(name == "ne")))),
            _ => Ok(None),
        }
    }
    /// `name(object, rest...)`, for builtins and for `object.name(rest...)`.
    fn obj_method(
        &mut self,
        this: &Value,
        id: u64,
        name: &str,
        rest: &[Value],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        self.check_cancel()?;
        let class = self.obj_class_name(id);
        let is_map = self.obj_is_map(id);
        let deleted = self.objects.entries[&id].deleted;
        let logical = |b: bool| -> Result<Option<Value>> {
            Ok(Some(Value::Logical(LogicalMatrix::scalar(b))))
        };
        match name {
            "class" => return Ok(Some(Value::Text(class.into()))),
            "isa" => {
                let [kind] = rest else {
                    return Err(if rest.is_empty() { FEW } else { MANY }.into());
                };
                let kind = kind.clone().array(&self.cancel)?;
                let kind = obj_text(&kind).unwrap_or_default();
                return logical(kind == class || kind == "handle");
            }
            "isobject" => return logical(true),
            "isvalid" => return logical(!deleted),
            "isstruct" | "iscell" | "isnumeric" | "ischar" | "islogical" | "isfloat" => {
                return logical(false);
            }
            "delete" => {
                if !rest.is_empty() {
                    return Err(MANY.into());
                }
                self.objects.entries.get_mut(&id).unwrap().deleted = true;
                return Ok(Some(Value::Void));
            }
            "eq" | "ne" => {
                let [other] = rest else {
                    return Err(if rest.is_empty() { FEW } else { MANY }.into());
                };
                let same = self.obj_value_id(other) == Some(id);
                return logical(same == (name == "eq"));
            }
            "isequal" => {
                if rest.is_empty() {
                    return Err(FEW.into());
                }
                for other in rest {
                    if !self.obj_equal(id, other)? {
                        return logical(false);
                    }
                }
                return logical(true);
            }
            "disp" | "display" => {
                let text = self.obj_body(id);
                self.emit(out, text)?;
                return Ok(Some(Value::Void));
            }
            "plus" | "minus" | "times" | "mtimes" | "rdivide" | "ldivide" | "mrdivide"
            | "mldivide" | "power" | "mpower" | "uminus" | "uplus" | "lt" | "gt" | "le" | "ge"
            | "and" | "or" | "not" | "horzcat" | "vertcat" | "struct2cell" | "rmfield"
            | "setfield" | "getfield" | "cell2mat" | "sum" | "abs" => {
                return Err(self.raise(
                    self.obj_undefined(name, Some(this)),
                    "MATLAB:UndefinedFunction".into(),
                ));
            }
            _ => {}
        }
        let (own, other): (&[&str], &[&str]) = if is_map {
            (&MAP_ONLY, &PARSER_ONLY)
        } else {
            (&PARSER_ONLY, &MAP_ONLY)
        };
        if other.contains(&name) {
            return Err(self.raise(
                self.obj_undefined(name, Some(this)),
                "MATLAB:UndefinedFunction".into(),
            ));
        }
        let common = matches!(
            name,
            "fieldnames" | "properties" | "length" | "size" | "isempty" | "numel" | "ndims"
        );
        if !own.contains(&name) && !common {
            return Ok(None);
        }
        if deleted && name != "numel" && name != "ndims" {
            return Err(if is_map { MAP_DELETED } else { PARSER_DELETED }.into());
        }
        let rest = rest
            .iter()
            .map(|v| v.clone().array(&self.cancel))
            .collect::<Result<Vec<_>>>()?;
        let count = self.obj_size(id);
        match name {
            "fieldnames" | "properties" => {
                let names: Vec<String> = if is_map {
                    ["Count", "KeyType", "ValueType"].map(String::from).to_vec()
                } else {
                    PARSER_PROPERTIES.map(String::from).to_vec()
                };
                let items = names
                    .iter()
                    .map(|n| self.obj_char(n).map(Arc::new))
                    .collect::<Result<Vec<_>>>()?;
                return self.obj_cell(items.len(), 1, items).map(|a| Some(a.into()));
            }
            "length" => {
                return Ok(Some(if is_map && count != 0. {
                    Array::Typed(TypedArray::scalar_int(NumericClass::Uint64, count as i128)).into()
                } else {
                    Value::Matrix(Matrix::scalar(count))
                }));
            }
            "numel" => return Ok(Some(Value::Matrix(Matrix::scalar(1.)))),
            "ndims" => return Ok(Some(Value::Matrix(Matrix::scalar(2.)))),
            "isempty" => return logical(is_map && count == 0.),
            "size" => return self.obj_size_value(id, &rest).map(Some),
            _ => {}
        }
        if is_map {
            self.obj_map_method(this, id, name, &rest, out).map(Some)
        } else {
            self.obj_parser_method(id, name, &rest, out).map(Some)
        }
    }
    fn obj_equal(&mut self, id: u64, other: &Value) -> Result<bool> {
        let Some(other) = self.obj_value_id(other) else {
            return Ok(false);
        };
        if other == id {
            return Ok(true);
        }
        let (ObjState::Map(a), ObjState::Map(b)) = (
            &self.objects.entries[&id].state,
            &self.objects.entries[&other].state,
        ) else {
            return Ok(false);
        };
        if a.key != b.key || a.value != b.value || a.entries.len() != b.entries.len() {
            return Ok(false);
        }
        let pairs: Vec<_> = a
            .entries
            .iter()
            .zip(&b.entries)
            .map(|((ka, (_, va)), (kb, (_, vb)))| (ka == kb, va.clone(), vb.clone()))
            .collect();
        for (same_key, va, vb) in pairs {
            self.check_cancel()?;
            let equal = self.isequal_call(&[
                Value::from(va.as_ref().clone()),
                Value::from(vb.as_ref().clone()),
            ])?;
            if !same_key || !matches!(equal, Value::Logical(m) if m.data() == [1]) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// `[a, b, ...] = size(object)`: only size has several outputs here.
    fn obj_size(&self, id: u64) -> f64 {
        match &self.objects.entries[&id].state {
            ObjState::Map(m) => m.entries.len() as f64,
            ObjState::Parser(_) => 1.,
        }
    }
    fn obj_count(&self, id: u64) -> Result<f64> {
        let entry = &self.objects.entries[&id];
        if entry.deleted {
            return Err(MAP_DELETED.into());
        }
        Ok(self.obj_size(id))
    }

    // ----- containers.Map -----

    fn obj_map(&self, id: u64) -> &MapState {
        match &self.objects.entries[&id].state {
            ObjState::Map(m) => m,
            ObjState::Parser(_) => unreachable!("map state"),
        }
    }
    fn obj_map_mut(&mut self, id: u64) -> &mut MapState {
        match &mut self.objects.entries.get_mut(&id).unwrap().state {
            ObjState::Map(m) => m,
            ObjState::Parser(_) => unreachable!("map state"),
        }
    }
    /// The stored key for `a` in a map of `key_type`; None if the type differs.
    fn obj_key(&self, key_type: &str, a: &Array) -> Result<Option<(MapKey, Array)>> {
        if key_type == "char" {
            return Ok(match a {
                Array::Character(c) if c.rows() <= 1 => {
                    Some((MapKey::Text(c.units().to_vec()), a.clone()))
                }
                _ => None,
            });
        }
        let Some(v) = obj_scalar_real(a) else {
            return Ok(None);
        };
        if v.is_nan() {
            return Err("NaN-valued key is not allowed for this container.".into());
        }
        let (number, array) = match NumericClass::parse(key_type) {
            None => (v, Array::Numeric(Matrix::scalar(v))),
            Some(NumericClass::Single) => {
                let s = f64::from(v as f32);
                (s, TypedArray::scalar_real(NumericClass::Single, s).into())
            }
            Some(class) => {
                let i = match a {
                    Array::Typed(t) if t.is_integer() => class.saturate(t.int(0)),
                    _ => class.from_f64(v),
                };
                return Ok(Some((
                    MapKey::Integer(i),
                    TypedArray::scalar_int(class, i).into(),
                )));
            }
        };
        Ok(Some((MapKey::Number(number + 0.), array)))
    }
    fn obj_map_new(&mut self, args: Vec<Value>, out: &mut Execution) -> Result<Value> {
        let args = args
            .into_iter()
            .map(|v| v.array(&self.cancel))
            .collect::<Result<Vec<_>>>()?;
        let mut state = MapState {
            key: "char",
            value: "any",
            entries: BTreeMap::new(),
        };
        if args.len() % 2 == 1 {
            return Err(self.raise(
                "Incorrect number of inputs specified.".into(),
                "MATLAB:Containers:Map:IncorrectNumberInputs".into(),
            ));
        }
        let option = |a: &Array| {
            obj_text(a).is_some_and(|t| {
                ["KeyType", "ValueType", "UniformValues"]
                    .iter()
                    .any(|o| o.eq_ignore_ascii_case(&t))
            })
        };
        let options = args.len() >= 4 && option(&args[0]);
        let pairs = if options || args.is_empty() {
            &args[..]
        } else {
            &args[2..]
        };
        let mut uniform = None;
        for pair in pairs.chunks(2) {
            let Some(name) = obj_text(&pair[0]) else {
                return Err("Parameter names must be character vectors.".into());
            };
            let value = obj_text(&pair[1]);
            match name.to_ascii_lowercase().as_str() {
                "keytype" if options => {
                    let value = value.unwrap_or_default();
                    let Some(key) = KEY_TYPES.iter().find(|k| **k == value).copied() else {
                        return Err(self.raise(
                            format!("Unsupported KeyType '{value}' specified.  See documentation for valid key types."),
                            "MATLAB:Containers:Map:KeyUnsupportedType".into(),
                        ));
                    };
                    state.key = key;
                }
                "valuetype" if options => {
                    let value = value.unwrap_or_default();
                    let Some(kind) = VALUE_TYPES.iter().find(|k| **k == value).copied() else {
                        return Err(self.raise(
                            format!("Unsupported ValueType '{value}' specified.  See documentation for valid value types."),
                            "MATLAB:Containers:Map:ValueUnsupportedType".into(),
                        ));
                    };
                    state.value = kind;
                }
                "uniformvalues" => {
                    let Some(flag) = obj_scalar_real(&pair[1]) else {
                        return Err("'UniformValues' must be a logical scalar.".into());
                    };
                    uniform = Some(flag != 0.);
                }
                _ => self.obj_warn(
                    "MATLAB:Containers:Map:UnknownParameter",
                    &format!("Unknown parameter name {name} will be ignored.  Check documentation for list of known parameters."),
                    out,
                )?,
            }
        }
        if !options && !args.is_empty() {
            self.obj_map_fill(&mut state, &args[0], &args[1], uniform)?;
        }
        self.obj_new(ObjState::Map(state))
    }
    /// Keys and values of the constructor's `containers.Map(keys, values)`.
    fn obj_map_fill(
        &mut self,
        state: &mut MapState,
        keys: &Array,
        values: &Array,
        uniform: Option<bool>,
    ) -> Result<()> {
        self.check_cancel()?;
        let element_count = |a: &Array| match a {
            Array::Cell(c) => c.len(),
            Array::Numeric(m) => m.len(),
            Array::Logical(m) => m.len(),
            Array::Typed(t) => t.len(),
            _ => 1,
        };
        if element_count(keys) > MAX_KEYS {
            return Err(format!("containers.Map holds at most {MAX_KEYS} keys"));
        }
        let elements = |a: &Array| -> Result<Vec<Array>> {
            self.check_cancel()?;
            let mut result = Vec::with_capacity(element_count(a));
            for i in 0..element_count(a) {
                if i % 1024 == 0 {
                    #[cfg(test)]
                    tests::obj_expansion_checkpoint(i, &self.cancel);
                    self.check_cancel()?;
                }
                result.push(match a {
                    Array::Cell(c) => c.get_linear(i)?.as_ref().clone(),
                    Array::Numeric(m) => Array::Numeric(Matrix::scalar(m.data()[i])),
                    Array::Logical(m) => Array::Logical(LogicalMatrix::scalar(m.data()[i] != 0)),
                    Array::Typed(t) => Array::Typed(t.select(1, 1, &[i])?),
                    other => other.clone(),
                });
            }
            Ok(result)
        };
        let key_class = |a: &Array| match a {
            Array::Character(_) => Some("char"),
            Array::Logical(_) => Some("double"),
            a => obj_scalar_class(a),
        };
        let (key_type, key_list) = match keys {
            Array::Cell(c) => {
                if c.is_empty() {
                    return Err(self.raise(
                        "Empty keys are not allowed in this container.".into(),
                        "MATLAB:Containers:Map:EmptyKey".into(),
                    ));
                }
                let list = elements(keys)?;
                let class = key_class(&list[0]);
                if list.iter().any(|k| key_class(k) != class) {
                    return Err(self.raise(
                        "The specified keys must all be of the same data type.".into(),
                        "MATLAB:Containers:Map:KeyTypeMismatch".into(),
                    ));
                }
                (class, list)
            }
            Array::Character(_) => (Some("char"), vec![keys.clone()]),
            a if obj_numeric(a) => {
                if a.rows() * a.cols() == 0 {
                    return Err(self.raise(
                        "When keys are specified, values must also be specified.".into(),
                        "MATLAB:Containers:Map:NoValues".into(),
                    ));
                }
                let list = elements(a)?;
                (key_class(&list[0]), list)
            }
            _ => (None, Vec::new()),
        };
        let Some(key_type) = key_type else {
            return Err(self.raise(KEY_TYPE.into(), MISMATCH_ID.into()));
        };
        let Some(key_type) = KEY_TYPES.iter().find(|k| **k == key_type).copied() else {
            return Err(self.raise(
                format!("Unsupported KeyType '{key_type}' specified.  See documentation for valid key types."),
                "MATLAB:Containers:Map:KeyUnsupportedType".into(),
            ));
        };
        let value_count = match values {
            Array::Cell(_) => element_count(values),
            v if key_list.len() == 1 || !obj_numeric(v) => 1,
            v => element_count(v),
        };
        if value_count != key_list.len() {
            return Err(self.raise(
                "The number of keys and values must be the same.".into(),
                "MATLAB:Containers:Map:NumberKeysValuesMismatch".into(),
            ));
        }
        let value_list = match values {
            Array::Cell(_) => elements(values)?,
            v if key_list.len() == 1 || !obj_numeric(v) => vec![v.clone()],
            v => elements(v)?,
        };
        let chars = value_list.iter().all(|v| matches!(v, Array::Character(_)));
        let class = obj_scalar_class(&value_list[0]);
        let inferred = if chars {
            "char"
        } else if class.is_some() && value_list.iter().all(|v| obj_scalar_class(v) == class) {
            VALUE_TYPES
                .iter()
                .find(|t| Some(**t) == class)
                .copied()
                .unwrap_or("any")
        } else {
            "any"
        };
        state.value = match uniform {
            Some(false) => "any",
            Some(true) if inferred == "any" => {
                return Err(self.raise(
                    "When 'UniformValues' is true, values must be all character arrays or all scalars of matching type.".into(),
                    "MATLAB:Containers:Map:NonUniformValues".into(),
                ));
            }
            _ => inferred,
        };
        state.key = key_type;
        for (i, (k, v)) in key_list.iter().zip(value_list).enumerate() {
            if i % 1024 == 0 {
                self.check_cancel()?;
            }
            let (key, array) = self.obj_key(key_type, k)?.ok_or(KEY_TYPE)?;
            state.entries.insert(key, (Arc::new(array), Arc::new(v)));
        }
        Ok(())
    }
    fn obj_map_method(
        &mut self,
        this: &Value,
        id: u64,
        name: &str,
        rest: &[Array],
        out: &mut Execution,
    ) -> Result<Value> {
        let signature = || {
            format!("No method '{name}' with matching signature found for class 'containers.Map'.")
        };
        let count = self.obj_map(id).entries.len();
        match name {
            "keys" | "values" if rest.is_empty() => {
                let map = self.obj_map(id);
                let items = map
                    .entries
                    .values()
                    .map(|(k, v)| if name == "keys" { k.clone() } else { v.clone() })
                    .collect();
                self.obj_cell(1, count, items).map(Value::from)
            }
            "keys" => Err(signature()),
            "values" => {
                if rest.len() > 1 {
                    return Err(signature());
                }
                let Array::Cell(c) = &rest[0] else {
                    return Err(
                        "Invalid input for argument 2 (keySet):\nValue must be 'cell'.".into(),
                    );
                };
                let mut items = Vec::with_capacity(c.len());
                for i in 0..c.len() {
                    if i % 1024 == 0 {
                        self.check_cancel()?;
                    }
                    let key = c.get_linear(i)?.clone();
                    items.push(self.obj_lookup(id, &key)?);
                }
                self.obj_cell(c.rows(), c.cols(), items).map(Value::from)
            }
            "isKey" => {
                let [key] = rest else {
                    return Err(signature());
                };
                let key_type = self.obj_map(id).key;
                let found = |k: &Array| -> Result<u8> {
                    // A key of another type, or a non-scalar number, is absent.
                    let key = self.obj_key(key_type, k).unwrap_or(None);
                    Ok(u8::from(key.is_some_and(|(key, _)| {
                        self.obj_map(id).entries.contains_key(&key)
                    })))
                };
                let (rows, cols, data) = match key {
                    Array::Cell(c) => {
                        let mut data = Vec::with_capacity(c.len());
                        for i in 0..c.len() {
                            data.push(found(c.get_linear(i)?)?);
                        }
                        (c.rows(), c.cols(), data)
                    }
                    k => (1, 1, vec![found(k)?]),
                };
                Ok(Value::Logical(LogicalMatrix::from_column_vec(
                    rows,
                    cols,
                    data,
                    &self.cancel,
                )?))
            }
            "remove" => {
                let [key] = rest else {
                    return Err(if rest.is_empty() {
                        FEW.into()
                    } else {
                        signature()
                    });
                };
                let list = match key {
                    Array::Cell(c) => (0..c.len())
                        .map(|i| c.get_linear(i).map(|a| a.as_ref().clone()))
                        .collect::<Result<Vec<_>>>()?,
                    k => vec![k.clone()],
                };
                let key_type = self.obj_map(id).key;
                let mut missing = false;
                let mut found = Vec::new();
                for k in &list {
                    self.check_cancel()?;
                    match self.obj_key(key_type, k).unwrap_or(None) {
                        Some((key, _)) if self.obj_map(id).entries.contains_key(&key) => {
                            found.push(key);
                        }
                        _ => missing = true,
                    }
                }
                if missing {
                    self.obj_warn(NO_KEY_ID, NO_KEY, out)?;
                }
                let mut removed = 0;
                let map = self.obj_map_mut(id);
                for key in found {
                    if let Some((a, v)) = map.entries.remove(&key) {
                        removed += Self::obj_entry_bytes(&key, &a, &v);
                    }
                }
                self.objects.entries.get_mut(&id).unwrap().bytes -= removed;
                self.objects.bytes -= removed;
                Ok(this.clone())
            }
            _ => Err(signature()),
        }
    }
    fn obj_size_value(&self, id: u64, rest: &[Array]) -> Result<Value> {
        let rows = self.obj_size(id);
        match rest {
            [] => Ok(Value::Matrix(Matrix::from_column_vec(
                1,
                2,
                vec![rows, 1.],
            )?)),
            [dim] => {
                let d = obj_scalar_real(dim)
                    .filter(|d| *d >= 1. && d.fract() == 0.)
                    .ok_or("Dimension argument must be a positive integer scalar within indexing range.")?;
                Ok(Value::Matrix(Matrix::scalar(if d == 1. {
                    rows
                } else {
                    1.
                })))
            }
            _ => Err(MANY.into()),
        }
    }
    /// `object(key)`: the value stored under key.
    fn obj_lookup(&mut self, id: u64, key: &Array) -> Result<Arc<Array>> {
        let key_type = self.obj_map(id).key;
        let header = Some(Some(SUBSREF.to_owned()));
        let Some((key, _)) = self.obj_key(key_type, key)? else {
            return Err(self.obj_fail(KEY_TYPE.into(), MISMATCH_ID, header));
        };
        match self.obj_map(id).entries.get(&key) {
            Some((_, v)) => Ok(v.clone()),
            None => Err(self.obj_fail(NO_KEY.into(), NO_KEY_ID, header)),
        }
    }
    fn obj_subsref(&mut self, id: u64, args: &[Expr], out: &mut Execution) -> Result<Value> {
        if self.objects.entries[&id].deleted {
            return Err(if self.obj_is_map(id) {
                MAP_DELETED
            } else {
                PARSER_DELETED
            }
            .into());
        }
        if !self.obj_is_map(id) {
            return Err("Array indexing of an inputParser is not supported".into());
        }
        let values = self.function_arguments(args, None, out)?;
        let [key] = values.as_slice() else {
            return Err("Only one-dimensional indexing is supported.".into());
        };
        let key = key.clone().array(&self.cancel)?;
        let value = self.obj_lookup(id, &key)?;
        Ok(Value::from(value.as_ref().clone()))
    }
    /// `object(key) = value`.
    fn obj_assign_key(
        &mut self,
        id: u64,
        args: &[Expr],
        rhs: &Expr,
        out: &mut Execution,
    ) -> Result<()> {
        if !self.obj_is_map(id) {
            return Err("Indexed assignment into an inputParser is not supported".into());
        }
        let value = self.eval(rhs, None, out)?;
        if matches!(value, Value::Void) {
            return Err("This command does not return a value".into());
        }
        let value = value.array(&self.cancel)?;
        self.obj_assign_key_value(id, args, value, out)
    }
    fn obj_assign_key_value(
        &mut self,
        id: u64,
        args: &[Expr],
        value: Array,
        out: &mut Execution,
    ) -> Result<()> {
        let keys = self.function_arguments(args, None, out)?;
        if self.objects.entries[&id].deleted {
            return Err(MAP_DELETED.into());
        }
        let [key] = keys.as_slice() else {
            return Err("Only one-dimensional indexing is supported.".into());
        };
        let key = key.clone().array(&self.cancel)?;
        let (key_type, value_type) = {
            let map = self.obj_map(id);
            (map.key, map.value)
        };
        let header = Some(Some("containers.Map/subsasgn".to_owned()));
        let Some((key, key_array)) = self.obj_key(key_type, &key)? else {
            return Err(self.obj_fail(KEY_TYPE.into(), MISMATCH_ID, header));
        };
        let value = match value_type {
            "any" => value,
            "char" if matches!(value, Array::Character(_)) => value,
            "char" => return Err(self.obj_fail(VALUE_TYPE.into(), MISMATCH_ID, header)),
            kind => {
                if obj_scalar_class(&value).is_none() {
                    return Err(self.obj_fail(VALUE_TYPE.into(), MISMATCH_ID, header));
                }
                self.call(kind, vec![Value::from(value)], out)?
                    .array(&self.cancel)?
            }
        };
        let added = Self::obj_entry_bytes(&key, &key_array, &value);
        let removed = self
            .obj_map(id)
            .entries
            .get(&key)
            .map_or(0, |(a, v)| Self::obj_entry_bytes(&key, a, v));
        self.obj_check_bytes(removed, added)?;
        let map = self.obj_map_mut(id);
        if map.entries.len() >= MAX_KEYS && !map.entries.contains_key(&key) {
            return Err(format!("containers.Map holds at most {MAX_KEYS} keys"));
        }
        map.entries
            .insert(key, (Arc::new(key_array), Arc::new(value)));
        self.objects.entries.get_mut(&id).unwrap().bytes += added;
        self.objects.bytes += added;
        self.objects.entries.get_mut(&id).unwrap().bytes -= removed;
        self.objects.bytes -= removed;
        Ok(())
    }

    // ----- inputParser -----

    fn obj_parser(&self, id: u64) -> &ParserState {
        match &self.objects.entries[&id].state {
            ObjState::Parser(p) => p,
            ObjState::Map(_) => unreachable!("parser state"),
        }
    }
    fn obj_parser_mut(&mut self, id: u64) -> &mut ParserState {
        match &mut self.objects.entries.get_mut(&id).unwrap().state {
            ObjState::Parser(p) => p,
            ObjState::Map(_) => unreachable!("parser state"),
        }
    }
    fn obj_parser_method(
        &mut self,
        id: u64,
        name: &str,
        rest: &[Array],
        out: &mut Execution,
    ) -> Result<Value> {
        match name {
            "copy" => {
                if !rest.is_empty() {
                    return Err(MANY.into());
                }
                let state = self.objects.entries[&id].state.clone();
                self.obj_new(state)
            }
            "parse" => {
                self.obj_parse(id, rest, out)?;
                Ok(Value::Void)
            }
            _ => {
                self.obj_add(id, name, rest)?;
                Ok(Value::Void)
            }
        }
    }
    /// addRequired / addOptional / addParameter / addParamValue.
    fn obj_add(&mut self, id: u64, method: &str, rest: &[Array]) -> Result<()> {
        let kind = match method {
            "addRequired" => ArgKind::Required,
            "addOptional" => ArgKind::Optional,
            _ => ArgKind::Parameter,
        };
        let needed = if kind == ArgKind::Required { 1 } else { 2 };
        if rest.len() < needed {
            return Err(FEW.into());
        }
        if kind != ArgKind::Parameter && rest.len() > needed + 1 {
            return Err(MANY.into());
        }
        let name = obj_text(&rest[0])
            .filter(|n| valid_field_name(n))
            .ok_or_else(|| {
                self.raise(
                    "First argument must be a string that can represent a field name.".into(),
                    "MATLAB:InputParser:ArgumentMustBeValidFieldName".into(),
                )
            })?;
        let mut options = &rest[needed..];
        let validator = match options.first() {
            Some(Array::Handle(h)) => {
                options = &options[1..];
                Some(h.clone())
            }
            Some(_) if kind != ArgKind::Parameter || options.len() == 1 => {
                return Err(self.raise(
                    "Validator must be a function handle.".into(),
                    "MATLAB:InputParser:ValidatorMustBeFunctionHandle".into(),
                ));
            }
            _ => None,
        };
        let mut priority = 1.;
        for pair in options.chunks(2) {
            let option = obj_text(&pair[0]).unwrap_or_default();
            if !option.eq_ignore_ascii_case("PartialMatchPriority") {
                return Err(self.raise(
                    format!("'{option}' is not a recognized parameter. Expected: 'PartialMatchPriority'."),
                    "MATLAB:InputParser:UnrecognizedParameter".into(),
                ));
            }
            priority = pair
                .get(1)
                .and_then(obj_scalar_real)
                .filter(|p| *p >= 1. && p.fract() == 0.)
                .ok_or("PartialMatchPriority must be a positive integer.")?;
        }
        let parser = self.obj_parser(id);
        let same = |n: &str| {
            if parser.case_sensitive {
                n == name
            } else {
                n.eq_ignore_ascii_case(&name)
            }
        };
        if parser.args.iter().any(|a| same(&a.name)) {
            return Err(self.raise(
                format!("A parameter name equivalent to '{name}' already exists in the input parser. Redefining a parameter is not allowed."),
                "MATLAB:InputParser:ParamAlreadyExists".into(),
            ));
        }
        if parser.args.len() >= MAX_PARAMETERS {
            return Err(format!(
                "inputParser holds at most {MAX_PARAMETERS} arguments"
            ));
        }
        let default = (kind != ArgKind::Required).then(|| Arc::new(rest[1].clone()));
        let added = name.len()
            + default.as_ref().map_or(0, |v| v.retained_bytes())
            + validator.as_ref().map_or(0, FunctionHandle::bytes);
        self.obj_check_bytes(0, added)?;
        self.objects.entries.get_mut(&id).unwrap().bytes += added;
        self.objects.bytes += added;
        self.obj_parser_mut(id).args.push(ParserArg {
            name,
            kind,
            default,
            validator,
            priority,
        });
        Ok(())
    }
    fn obj_match(p: &ParserState, given: &str) -> Option<ObjMatch> {
        let mut params: Vec<(usize, &ParserArg)> = p
            .args
            .iter()
            .enumerate()
            .filter(|(_, a)| a.kind == ArgKind::Parameter)
            .collect();
        params.sort_by_key(|(_, a)| obj_sort_key(&a.name));
        let same = |a: &str, b: &str| {
            if p.case_sensitive {
                a == b
            } else {
                a.eq_ignore_ascii_case(b)
            }
        };
        if let Some((i, _)) = params.iter().find(|(_, a)| same(&a.name, given)) {
            return Some(ObjMatch::One(*i, None));
        }
        if !p.partial_matching {
            return None;
        }
        let candidates: Vec<_> = params
            .into_iter()
            .filter(|(_, a)| a.name.get(..given.len()).is_some_and(|s| same(s, given)))
            .collect();
        match candidates.as_slice() {
            [] => None,
            [(i, _)] => Some(ObjMatch::One(*i, None)),
            _ => {
                let list = candidates
                    .iter()
                    .map(|(_, a)| format!("'{}'", a.name))
                    .collect::<Vec<_>>()
                    .join(", ");
                let best = candidates
                    .iter()
                    .map(|(_, a)| a.priority)
                    .fold(f64::INFINITY, f64::min);
                let first: Vec<_> = candidates
                    .iter()
                    .filter(|(_, a)| a.priority == best)
                    .collect();
                if let [(i, a)] = first.as_slice() {
                    Some(ObjMatch::One(
                        *i,
                        Some(format!(
                            "'{given}' matches multiple parameter names: {list}. Assuming the intended name is '{}'.",
                            a.name
                        )),
                    ))
                } else {
                    Some(ObjMatch::Ambiguous(list))
                }
            }
        }
    }
    /// Run a validator: Ok(None) passes, Ok(Some(reason)) fails.
    fn obj_validator(
        &mut self,
        validator: &FunctionHandle,
        value: &Array,
        out: &mut Execution,
    ) -> Result<Option<String>> {
        let result = self.call_handle(validator, vec![Value::from(value.clone())], 1, None, out);
        match result {
            Ok(values) => Ok(match values.first() {
                Some(v) if !obj_truthy(v) => Some(format!(
                    "It must satisfy the function: {}.",
                    validator.func2str()
                )),
                _ => None,
            }),
            Err(error) => {
                if !self.is_catchable(&error) {
                    return Err(error);
                }
                let (message, _) = self.failure_text(&error);
                self.raised = None;
                self.functions.lng.thrown_stack = None;
                self.functions.lng.thrown_object = None;
                Ok(Some(message))
            }
        }
    }
    fn obj_parse_fail(
        &mut self,
        header: &Option<Option<String>>,
        message: String,
        id: &str,
    ) -> String {
        self.obj_fail(message, &format!("MATLAB:InputParser:{id}"), header.clone())
    }
    fn obj_check(
        &mut self,
        arg: &ParserArg,
        header: &Option<Option<String>>,
        value: &Array,
        out: &mut Execution,
    ) -> Result<()> {
        if let Some(validator) = &arg.validator
            && let Some(reason) = self.obj_validator(validator, value, out)?
        {
            return Err(self.obj_parse_fail(
                header,
                format!("The value of '{}' is invalid. {reason}", arg.name),
                "ArgumentFailedValidation",
            ));
        }
        Ok(())
    }
    fn obj_parse(&mut self, id: u64, args: &[Array], out: &mut Execution) -> Result<()> {
        let p = self.obj_parser(id).clone();
        let header = (!p.function_name.is_empty()).then(|| Some(p.function_name.clone()));
        let header = &header;
        let mut values: Vec<Option<Arc<Array>>> = vec![None; p.args.len()];
        let mut next = 0;
        for i in (0..p.args.len()).filter(|i| p.args[*i].kind == ArgKind::Required) {
            let Some(value) = args.get(next) else {
                return Err(self.obj_parse_fail(header, FEW.into(), "notEnoughInputs"));
            };
            self.obj_check(&p.args[i], header, value, out)?;
            values[i] = Some(Arc::new(value.clone()));
            next += 1;
        }
        let has_parameters = p.args.iter().any(|a| a.kind == ArgKind::Parameter);
        for i in (0..p.args.len()).filter(|i| p.args[*i].kind == ArgKind::Optional) {
            let Some(value) = args.get(next) else { break };
            if has_parameters && let Some(text) = obj_text(value) {
                // A string goes to the optional argument only if its validator
                // accepts it; otherwise it must start the name-value pairs.
                let accepted = match &p.args[i].validator {
                    Some(validator) => self.obj_validator(validator, value, out)?.is_none(),
                    None => false,
                };
                if !accepted {
                    if Self::obj_match(&p, &text).is_some() {
                        break;
                    }
                    return Err(self.obj_parse_fail(
                        header,
                        format!("The argument '{text}' is a string and does not match any parameter names. It failed validation for the argument '{}'.", p.args[i].name),
                        "ArgumentFailedValidation",
                    ));
                }
            } else if has_parameters
                && p.struct_expand
                && matches!(value, Array::Struct(_))
                && self.obj_id(value).is_none()
            {
                break;
            } else {
                self.obj_check(&p.args[i], header, value, out)?;
            }
            values[i] = Some(Arc::new(value.clone()));
            next += 1;
        }
        let mut unmatched: BTreeMap<(String, String), (String, Arc<Array>)> = BTreeMap::new();
        let mut pairs: Vec<(String, Array)> = Vec::new();
        while next < args.len() {
            self.check_cancel()?;
            let value = &args[next];
            if p.struct_expand
                && let Array::Struct(s) = value
                && self.obj_id(value).is_none()
            {
                if s.len() != 1 {
                    return Err(self.obj_parse_fail(
                        header,
                        "Input structure to a function must be scalar.".into(),
                        "StructMustBeScalar",
                    ));
                }
                for (f, name) in s.field_names().iter().enumerate() {
                    pairs.push((name.clone(), s.get(f, 0)?.as_ref().clone()));
                }
                next += 1;
                continue;
            }
            let Some(name) = obj_text(value) else {
                let class = self.obj_class(&Value::from(value.clone()));
                return Err(self.obj_parse_fail(
                    header,
                    format!("Expected a string for the parameter name, instead the input type was '{class}'."),
                    "ParamMustBeChar",
                ));
            };
            let Some(value) = args.get(next + 1) else {
                return Err(self.obj_parse_fail(
                    header,
                    format!("No value was given for '{name}'. Name-value pair arguments require a name followed by a value."),
                    "ParamMissingValue",
                ));
            };
            pairs.push((name, value.clone()));
            next += 2;
        }
        for (name, value) in pairs {
            match Self::obj_match(&p, &name) {
                Some(ObjMatch::One(i, warning)) => {
                    if let Some(warning) = warning {
                        self.obj_warn(
                            "MATLAB:InputParser:AmbiguousParameterWithPriority",
                            &warning,
                            out,
                        )?;
                    }
                    self.obj_check(&p.args[i], header, &value, out)?;
                    values[i] = Some(Arc::new(value));
                }
                Some(ObjMatch::Ambiguous(list)) => {
                    return Err(self.obj_parse_fail(
                        header,
                        format!("'{name}' matches multiple parameter names: {list}. To avoid ambiguity, specify the complete name of the parameter."),
                        "AmbiguousParameter",
                    ));
                }
                None if p.keep_unmatched => {
                    unmatched.insert(obj_sort_key(&name), (name, Arc::new(value)));
                }
                None => {
                    let doc = if p.function_name.is_empty() {
                        "this function"
                    } else {
                        &p.function_name
                    };
                    return Err(self.obj_parse_fail(
                        header,
                        format!("'{name}' is not a recognized parameter. For a list of valid name-value pair arguments, see the documentation for {doc}."),
                        "UnmatchedParameter",
                    ));
                }
            }
        }
        let mut results: Vec<_> = p
            .args
            .iter()
            .zip(&values)
            .map(|(a, v)| {
                let value = v.clone().or_else(|| a.default.clone()).unwrap();
                (a.name.clone(), value)
            })
            .collect();
        results.sort_by_key(|(n, _)| obj_sort_key(n));
        let using_defaults: Vec<String> = p
            .args
            .iter()
            .zip(&values)
            .filter(|(_, v)| v.is_none())
            .map(|(a, _)| a.name.clone())
            .collect();
        // Struct field rules apply to Results and Unmatched as built here.
        let unmatched: Vec<_> = unmatched.into_values().collect();
        self.obj_struct(&results)?;
        self.obj_struct(&unmatched)?;
        let removed = Self::obj_result_bytes(&self.obj_parser(id).results)
            + Self::obj_result_bytes(&self.obj_parser(id).unmatched)
            + self
                .obj_parser(id)
                .using_defaults
                .iter()
                .map(String::len)
                .sum::<usize>();
        let added = Self::obj_result_bytes(&results)
            + Self::obj_result_bytes(&unmatched)
            + using_defaults.iter().map(String::len).sum::<usize>();
        self.obj_check_bytes(removed, added)?;
        self.objects.entries.get_mut(&id).unwrap().bytes += added;
        self.objects.bytes += added;
        self.objects.entries.get_mut(&id).unwrap().bytes -= removed;
        self.objects.bytes -= removed;
        let state = self.obj_parser_mut(id);
        state.results = results;
        state.using_defaults = using_defaults;
        state.unmatched = unmatched;
        Ok(())
    }
    fn obj_property(&self, id: u64, name: &str) -> Result<Value> {
        if self.objects.entries[&id].deleted {
            return Err(if self.obj_is_map(id) {
                MAP_DELETED
            } else {
                PARSER_DELETED
            }
            .into());
        }
        match &self.objects.entries[&id].state {
            ObjState::Map(m) => match name {
                "Count" => Ok(Value::Container(
                    TypedArray::scalar_int(NumericClass::Uint64, m.entries.len() as i128).into(),
                )),
                "KeyType" => Ok(Value::Text(m.key.into())),
                "ValueType" => Ok(Value::Text(m.value.into())),
                _ => Err(format!(
                    "The name '{name}' is not an accessible property for an instance of class 'containers.Map'."
                )),
            },
            ObjState::Parser(p) => {
                let flag = |b: bool| Ok(Value::Logical(LogicalMatrix::scalar(b)));
                match name {
                    "FunctionName" => Ok(Value::Container(self.obj_char(&p.function_name)?)),
                    "CaseSensitive" => flag(p.case_sensitive),
                    "KeepUnmatched" => flag(p.keep_unmatched),
                    "PartialMatching" => flag(p.partial_matching),
                    "StructExpand" => flag(p.struct_expand),
                    "Parameters" => {
                        let mut names: Vec<_> = p.args.iter().map(|a| a.name.clone()).collect();
                        names.sort_by_key(|n| obj_sort_key(n));
                        Ok(self.obj_names(&names)?.into())
                    }
                    "Results" => Ok(self.obj_struct(&p.results)?.into()),
                    "Unmatched" => Ok(self.obj_struct(&p.unmatched)?.into()),
                    "UsingDefaults" => Ok(self.obj_names(&p.using_defaults)?.into()),
                    _ => Err(format!(
                        "No appropriate method, property, or field '{name}' for class 'inputParser'."
                    )),
                }
            }
        }
    }
    /// `object.name` or `object.name(args)`: a method call or a property.
    fn obj_member(
        &mut self,
        this: Value,
        id: u64,
        name: &str,
        args: Option<&[Expr]>,
        out: &mut Execution,
    ) -> Result<Value> {
        let methods: &[&str] = if self.obj_is_map(id) {
            &MAP_METHODS
        } else {
            &PARSER_METHODS
        };
        if methods.contains(&name) {
            let rest = match args {
                Some(args) => self.function_arguments(args, None, out)?,
                None => Vec::new(),
            };
            return Ok(self
                .obj_method(&this, id, name, &rest, out)?
                .unwrap_or(Value::Void));
        }
        let value = self.obj_property(id, name)?;
        match args {
            Some(args) if !args.is_empty() => self.index_value(value, args, out),
            _ => Ok(value),
        }
    }
    /// `object.name = value`.
    fn obj_assign_property(
        &mut self,
        id: u64,
        steps: &[PathStep],
        rhs: &Expr,
        out: &mut Execution,
    ) -> Result<()> {
        let value = self.eval(rhs, None, out)?;
        if matches!(value, Value::Void) {
            return Err("This command does not return a value".into());
        }
        let value = value.array(&self.cancel)?;
        self.obj_set_property(id, steps, value, out)
    }
    fn obj_set_property(
        &mut self,
        id: u64,
        steps: &[PathStep],
        value: Array,
        out: &mut Execution,
    ) -> Result<()> {
        let name = match steps.first() {
            Some(PathStep::Field(name)) => name.clone(),
            Some(PathStep::Dynamic(e)) => {
                let v = self.eval(e, None, out)?.array(&self.cancel)?;
                obj_text(&v).ok_or("Dynamic structure field names must be character text")?
            }
            _ => {
                if self.obj_is_map(id) && matches!(steps.first(), Some(PathStep::Paren(_))) {
                    return Err("Assignment into a field of a containers.Map value is not supported; assign the value to a variable first".into());
                }
                return Err(format!(
                    "Indexed assignment into a {} is not supported",
                    self.obj_class_name(id)
                ));
            }
        };
        if self.objects.entries[&id].deleted {
            return Err(if self.obj_is_map(id) {
                MAP_DELETED
            } else {
                PARSER_DELETED
            }
            .into());
        }
        let no_header = Some(None);
        if self.obj_is_map(id) {
            let message = if ["Count", "KeyType", "ValueType"].contains(&name.as_str()) {
                format!("You cannot set the read-only property '{name}' of Map.")
            } else {
                format!(
                    "The name '{name}' is not an accessible property for an instance of class 'containers.Map'."
                )
            };
            return Err(self.obj_fail(message, "MATLAB:class:SetProhibited", no_header));
        }
        if steps.len() > 1 && PARSER_PROPERTIES.contains(&name.as_str()) {
            return Err("Indexed assignment into an inputParser property is not supported".into());
        }
        match name.as_str() {
            "FunctionName" => {
                let Some(text) = obj_text(&value) else {
                    return Err(self.obj_fail(
                        "While setting the 'FunctionName' property of 'inputParser':\nValue must be a string.".into(),
                        "MATLAB:class:RequireString",
                        no_header,
                    ));
                };
                let removed = self.obj_parser(id).function_name.len();
                self.obj_check_bytes(removed, text.len())?;
                self.objects.entries.get_mut(&id).unwrap().bytes += text.len();
                self.objects.bytes += text.len();
                self.objects.entries.get_mut(&id).unwrap().bytes -= removed;
                self.objects.bytes -= removed;
                self.obj_parser_mut(id).function_name = text;
            }
            "CaseSensitive" | "KeepUnmatched" | "PartialMatching" | "StructExpand" => {
                let flag = match &value {
                    Array::Character(c) if c.len() == 1 => Some(c.units()[0] != 0),
                    v => obj_scalar_real(v).map(|x| x != 0.),
                };
                let Some(flag) = flag else {
                    return Err(self.obj_fail(
                        format!("While setting the '{name}' property of 'inputParser':\nValue must be a scalar."),
                        "MATLAB:class:RequireScalar",
                        no_header,
                    ));
                };
                let p = self.obj_parser_mut(id);
                match name.as_str() {
                    "CaseSensitive" => p.case_sensitive = flag,
                    "KeepUnmatched" => p.keep_unmatched = flag,
                    "PartialMatching" => p.partial_matching = flag,
                    _ => p.struct_expand = flag,
                }
            }
            "Parameters" | "Results" | "Unmatched" | "UsingDefaults" => {
                return Err(self.obj_fail(
                    format!("You cannot set the read-only property '{name}' of inputParser."),
                    "MATLAB:class:SetProhibited",
                    no_header,
                ));
            }
            _ => {
                return Err(format!(
                    "No public property {name} exists for class inputParser."
                ));
            }
        }
        Ok(())
    }

    // ----- display and reports -----

    fn obj_body(&self, id: u64) -> String {
        let entry = &self.objects.entries[&id];
        let short = if self.obj_is_map(id) { "Map" } else { PARSER };
        if entry.deleted {
            return format!("  handle to deleted {short}\n");
        }
        match &entry.state {
            ObjState::Map(m) => format!(
                "  Map with properties:\n\n{:>13}: {}\n{:>13}: {}\n{:>13}: {}\n",
                "Count",
                m.entries.len(),
                "KeyType",
                m.key,
                "ValueType",
                m.value
            ),
            ObjState::Parser(_) => {
                let fields = self.obj_parser_fields(id);
                let body = fields
                    .ok()
                    .and_then(|f| {
                        crate::display_format::disp(&f, &self.display)
                            .ok()
                            .flatten()
                    })
                    .unwrap_or_default();
                format!(
                    "  inputParser with properties:\n\n{}",
                    Self::obj_unmask(&body)
                )
            }
        }
    }
    /// The public properties as a struct; an empty cell is a stand-in that
    /// [`Self::obj_unmask`] shows as `{1x0 cell}`.
    fn obj_parser_fields(&self, id: u64) -> Result<Array> {
        let empty = self.obj_char(&String::from(EMPTY_MASK).repeat(EMPTY_CELL.len() - 2))?;
        let fields = PARSER_PROPERTIES
            .iter()
            .map(|n| {
                let a = self.obj_property(id, n)?.array(&self.cancel)?;
                let a = match a {
                    Array::Cell(c) if c.is_empty() => empty.clone(),
                    a => a,
                };
                Ok(((*n).to_owned(), Arc::new(a)))
            })
            .collect::<Result<Vec<_>>>()?;
        self.obj_struct(&fields)
    }
    fn obj_unmask(text: &str) -> String {
        let quoted = |c: char, n: usize| format!("'{}'", String::from(c).repeat(n));
        text.replace(&quoted(MAP_MASK, MAP.len() + 4), "[1x1 containers.Map]")
            .replace(&quoted(PARSER_MASK, PARSER.len() + 4), "[1x1 inputParser]")
            .replace(&quoted(EMPTY_MASK, EMPTY_CELL.len() - 2), EMPTY_CELL)
    }
    /// `name = value` text for an object, or for a cell or struct holding
    /// objects, where each shows as `[1x1 class]`. None for other values.
    pub(super) fn obj_display(&self, name: &str, a: &Array) -> Result<Option<String>> {
        if self.objects.entries.is_empty() {
            return Ok(None);
        }
        let marker = if self.display.compact {
            format!("{name} =\n")
        } else {
            format!("\n{name} = \n\n")
        };
        if let Some(id) = self.obj_id(a) {
            let entry = &self.objects.entries[&id];
            if !entry.deleted && !self.obj_is_map(id) {
                let fields = self.obj_parser_fields(id)?;
                let text = crate::display_format::echo(name, &fields, &self.display)?;
                let header = "  inputParser with properties:\n\n";
                let text = text.replacen(&marker, &(marker.clone() + header), 1);
                return Ok(Some(Self::obj_unmask(&text)));
            }
            return Ok(Some(marker + &self.obj_body(id)));
        }
        let Some(masked) = self.obj_mask(a, 0)? else {
            return Ok(None);
        };
        let text = crate::display_format::echo(name, &masked, &self.display)?;
        Ok(Some(Self::obj_unmask(&text)))
    }
    /// A copy of `a` with every object replaced by its display stand-in.
    fn obj_mask(&self, a: &Array, depth: usize) -> Result<Option<Array>> {
        if depth > 8 {
            return Ok(None);
        }
        if let Some(id) = self.obj_id(a) {
            let (mask, len) = if self.obj_is_map(id) {
                (MAP_MASK, MAP.len() + 4)
            } else {
                (PARSER_MASK, PARSER.len() + 4)
            };
            return Ok(Some(self.obj_char(&String::from(mask).repeat(len))?));
        }
        let slots: Vec<Arc<Array>> = match a {
            Array::Cell(c) => (0..c.len())
                .map(|i| c.get_linear(i).cloned())
                .collect::<Result<_>>()?,
            Array::Struct(s) => (0..s.len() * s.field_count())
                .map(|i| s.get(i % s.field_count(), i / s.field_count()).cloned())
                .collect::<Result<_>>()?,
            _ => return Ok(None),
        };
        let mut changed = false;
        let mut masked = Vec::with_capacity(slots.len());
        for slot in slots {
            self.check_cancel()?;
            match self.obj_mask(&slot, depth + 1)? {
                Some(m) => {
                    changed = true;
                    masked.push(Arc::new(m));
                }
                None => masked.push(slot),
            }
        }
        if !changed {
            return Ok(None);
        }
        Ok(Some(match a {
            Array::Cell(c) => self.obj_cell(c.rows(), c.cols(), masked)?,
            Array::Struct(s) => Array::Struct(StructArray::from_slots(
                s.rows(),
                s.cols(),
                s.field_names().to_vec(),
                masked,
                &self.cancel,
            )?),
            _ => unreachable!(),
        }))
    }
    /// getReport(exception, 'basic', ...): "Error using NAME" and the message.
    fn obj_report(&self, args: &[Value]) -> Result<Value> {
        let Value::Container(Array::Struct(s)) = &args[0] else {
            return Err("Input must be an MException.".into());
        };
        let field = |name: &str| -> Option<Arc<Array>> {
            s.field_index(name).and_then(|f| s.get(f, 0).ok()).cloned()
        };
        let text = |name: &str| field(name).and_then(|a| obj_text(&a));
        let (Some(message), Some(_)) = (text("message"), text("identifier")) else {
            return Err("Input must be an MException.".into());
        };
        let header = match text(OBJ_REPORT_HEADER) {
            Some(h) => (!h.is_empty()).then_some(h),
            _ => field("stack").and_then(|stack| match stack.as_ref() {
                Array::Struct(st) if !st.is_empty() => st
                    .field_index("name")
                    .and_then(|f| st.get(f, 0).ok())
                    .and_then(|a| obj_text(a)),
                _ => None,
            }),
        };
        Ok(Value::Text(match header {
            Some(name) => format!("Error using {name}\n{message}\n"),
            None => format!("{message}\n"),
        }))
    }

    // ----- hooks from expression evaluation and statements -----

    /// Expressions that reach these objects: `containers.Map(...)`,
    /// `object(key)`, `object.name(...)`, operators and concatenation.
    pub(super) fn obj_eval(&mut self, e: &Expr, out: &mut Execution) -> Result<Option<Value>> {
        match e {
            Expr::Index(base, args) => {
                let Expr::Field(inner, member) = base.as_ref() else {
                    return Ok(None);
                };
                if member == "Map" && obj_is_containers(inner) && !self.contains_array("containers")
                {
                    let values = self.function_arguments(args, None, out)?;
                    return self.obj_map_new(values, out).map(Some);
                }
                if member == "empty"
                    && let Expr::Field(root, map) = inner.as_ref()
                    && map == "Map"
                    && obj_is_containers(root)
                    && !self.contains_array("containers")
                {
                    if !args.is_empty() {
                        return Err("containers.Map.empty takes no arguments".into());
                    }
                    return self.obj_map_new(Vec::new(), out).map(Some);
                }
                let Some((id, this)) = self.obj_expr_var(inner) else {
                    return Ok(None);
                };
                self.obj_member(this, id, member, Some(args), out).map(Some)
            }
            Expr::Field(inner, member) => {
                if member == "Map" && obj_is_containers(inner) && !self.contains_array("containers")
                {
                    return self.obj_map_new(Vec::new(), out).map(Some);
                }
                if member == "empty"
                    && let Expr::Field(root, map) = inner.as_ref()
                    && map == "Map"
                    && obj_is_containers(root)
                    && !self.contains_array("containers")
                {
                    return self.obj_map_new(Vec::new(), out).map(Some);
                }
                let Some((id, this)) = self.obj_expr_var(inner) else {
                    return Ok(None);
                };
                self.obj_member(this, id, member, None, out).map(Some)
            }
            Expr::Name(name)
                if name == PARSER
                    && !self.contains_array(name)
                    && self.resolve_function(name)?.is_none() =>
            {
                self.obj_new(ObjState::Parser(ParserState::new())).map(Some)
            }
            Expr::Call(name, args) => {
                let Some((id, _alive)) = self.obj_var(name) else {
                    return Ok(None);
                };
                self.obj_subsref(id, args, out).map(Some)
            }
            Expr::Binary(
                Token::ShortAnd | Token::ShortOr | Token::Equal | Token::NotEqual,
                _,
                _,
            ) => Ok(None),
            Expr::Binary(op, a, b) => {
                let (left, right) = (self.obj_expr_var(a), self.obj_expr_var(b));
                let Some((id, _)) = left.as_ref().or(right.as_ref()) else {
                    return Ok(None);
                };
                let id = *id;
                let class = self.obj_class_name(id);
                Err(self.raise(
                    format!(
                        "Undefined function '{}' for input arguments of type '{class}'.",
                        obj_operator(op)
                    ),
                    "MATLAB:UndefinedFunction".into(),
                ))
            }
            Expr::Matrix(rows) => {
                if self.objects.entries.is_empty() || rows.iter().map(Vec::len).sum::<usize>() < 2 {
                    return Ok(None);
                }
                let Some((id, _alive)) = rows.iter().flatten().find_map(|e| self.obj_expr_var(e))
                else {
                    return Ok(None);
                };
                let direction = if rows.len() > 1 {
                    "Vertical"
                } else {
                    "Horizontal"
                };
                let class = if self.obj_is_map(id) {
                    "a containers.Map"
                } else {
                    "an inputParser"
                };
                Err(format!(
                    "{direction} concatenation is not supported by {class}."
                ))
            }
            _ => Ok(None),
        }
    }
    /// Identity comparison after ordinary operand evaluation, including fields,
    /// cells and function results. Never evaluates an operand a second time.
    pub(super) fn obj_equality(
        &mut self,
        op: &Token,
        left: &Value,
        right: &Value,
    ) -> Result<Option<Value>> {
        if !matches!(op, Token::Equal | Token::NotEqual) {
            return Ok(None);
        }
        let (left, right) = (self.obj_value_id(left), self.obj_value_id(right));
        if let (Some(x), Some(y)) = (left, right) {
            return Ok(Some(Value::Logical(LogicalMatrix::scalar(
                (x == y) == (*op == Token::Equal),
            ))));
        }
        if let Some(id) = left.or(right) {
            return Err(self.raise(
                format!(
                    "Undefined function '{}' for input arguments of type '{}'.",
                    obj_operator(op),
                    self.obj_class_name(id)
                ),
                "MATLAB:UndefinedFunction".into(),
            ));
        }
        Ok(None)
    }
    /// Statements on these objects: `object(key) = v`, `object.name = v`,
    /// `[r, c] = size(object)` and a bare `object` echo. True if handled.
    pub(super) fn obj_statement(&mut self, st: &Evaluation, out: &mut Execution) -> Result<bool> {
        if self.objects.entries.is_empty() {
            return Ok(false);
        }
        if let Some((root, steps)) = &st.path_target
            && let Some((id, _alive)) = self.obj_var(root)
        {
            self.obj_assign_property(id, steps, &st.expr, out)?;
            self.obj_echo(root, st.quiet, out)?;
            return Ok(true);
        }
        if let Some((name, args)) = &st.indexed_target
            && !st.cell_content_target
            && let Some((id, _alive)) = self.obj_var(name)
        {
            self.obj_assign_key(id, args, &st.expr, out)?;
            self.obj_echo(name, st.quiet, out)?;
            return Ok(true);
        }
        if !st.targets.is_empty() || st.indexed_target.is_some() || st.path_target.is_some() {
            return Ok(false);
        }
        if let Expr::Name(name) = &st.expr
            && self.obj_var(name).is_some()
        {
            self.obj_echo(name, st.quiet, out)?;
            return Ok(true);
        }
        // `object.method(...)` and `f(object, ...)` run once, with nargout=0.
        let value = match &st.expr {
            Expr::Index(base, _) | Expr::Field(base, _)
                if matches!(base.as_ref(), Expr::Field(root, _) if self.obj_expr_var(root).is_some())
                    || self.obj_expr_var(base).is_some() =>
            {
                match self.obj_eval(&st.expr, out)? {
                    Some(value) => value,
                    None => return Ok(false),
                }
            }
            Expr::Call(name, args)
                if args.first().is_some_and(|a| self.obj_expr_var(a).is_some())
                    && !self.contains_array(name)
                    && self.resolve_function(name)?.is_none() =>
            {
                let values = self.function_arguments(args, None, out)?;
                match self.obj_call(name, &values, out)? {
                    Some(value) => value,
                    None => self.call(name, values, out)?,
                }
            }
            _ => return Ok(false),
        };
        if !matches!(value, Value::Void) {
            let value = value.array(&self.cancel)?;
            self.assign_workspace_array("ans", value, st.quiet, out)?;
        }
        Ok(true)
    }
    pub(super) fn obj_size_outputs(
        &self,
        values: &[Value],
        count: usize,
    ) -> Result<Option<Vec<Value>>> {
        let Some(id) = values.first().and_then(|v| self.obj_value_id(v)) else {
            return Ok(None);
        };
        if values.len() != 1 {
            return Err(MANY.into());
        }
        if self.obj_is_map(id) {
            self.obj_count(id)?;
        }
        let mut outputs = vec![Value::Matrix(Matrix::scalar(self.obj_size(id)))];
        outputs.resize(count, Value::Matrix(Matrix::scalar(1.)));
        Ok(Some(outputs))
    }
    pub(super) fn obj_assign_nested(
        &mut self,
        current: &Array,
        steps: &[PathStep],
        value: &Array,
        out: &mut Execution,
    ) -> Result<bool> {
        let Some(id) = self.obj_id(current) else {
            return Ok(false);
        };
        if self.obj_is_map(id)
            && let [PathStep::Paren(args)] = steps
        {
            self.obj_assign_key_value(id, args, value.clone(), out)?;
        } else {
            self.obj_set_property(id, steps, value.clone(), out)?;
        }
        Ok(true)
    }
    pub(super) fn obj_member_value(
        &mut self,
        base: &Value,
        member: &str,
        args: &[Expr],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        let Some(id) = self.obj_value_id(base) else {
            return Ok(None);
        };
        self.obj_member(base.clone(), id, member, Some(args), out)
            .map(Some)
    }
    /// `value.name` where value is an object reached through another value.
    pub(super) fn obj_field(
        &mut self,
        base: &Value,
        name: &str,
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        let Some(id) = self.obj_value_id(base) else {
            return Ok(None);
        };
        self.obj_member(base.clone(), id, name, None, out).map(Some)
    }
    /// `value(args)` where value is an object reached through another value.
    pub(super) fn obj_index(
        &mut self,
        base: &Value,
        args: &[Expr],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        let Some(id) = self.obj_value_id(base) else {
            return Ok(None);
        };
        self.obj_subsref(id, args, out).map(Some)
    }
    fn obj_echo(&mut self, name: &str, quiet: bool, out: &mut Execution) -> Result<()> {
        if quiet {
            return Ok(());
        }
        let Some(a) = self.container_variables.get(name).cloned() else {
            return Ok(());
        };
        let text = self.display_text(name, &a)?;
        self.emit(out, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::Cell,
        sync::atomic::{AtomicBool, Ordering},
    };

    thread_local! {
        static STOP_EXPANSION_AT: Cell<Option<usize>> = const { Cell::new(None) };
        static EXPANSION_CHECKS: Cell<usize> = const { Cell::new(0) };
    }
    pub(super) fn obj_expansion_checkpoint(index: usize, cancel: &AtomicBool) {
        EXPANSION_CHECKS.set(EXPANSION_CHECKS.get() + 1);
        if STOP_EXPANSION_AT.get() == Some(index) {
            STOP_EXPANSION_AT.set(None);
            cancel.store(true, Ordering::Relaxed);
        }
    }

    #[test]
    fn sweep_hysteresis_and_cached_bytes_follow_state_changes() {
        let mut e = Engine::default();
        let check = |e: &Engine| {
            assert_eq!(
                e.obj_workspace_bytes(),
                e.objects.entries.values().map(|e| e.bytes).sum::<usize>()
            );
            for entry in e.objects.entries.values() {
                assert_eq!(entry.bytes, Engine::obj_state_bytes(&entry.state));
            }
        };
        for source in [
            "m=containers.Map(); m('data')=zeros(1,9000000);",
            "m('other')=1; m('other')=2; remove(m,'other');",
            "p=inputParser; p.FunctionName='longName'; p.FunctionName='x'; addOptional(p,'a',42); parse(p); parse(p,7);",
            "p.KeepUnmatched=true; parse(p,8,'extra',9); delete(p);",
        ] {
            e.execute(source).unwrap();
            check(&e);
        }
        assert_eq!(e.objects.sweep_bytes_at, 2 * e.objects.bytes);
        assert_eq!(e.objects.sweep_at, (2 * e.objects.entries.len()).max(1024));
        // Below the new byte threshold, a statement must not retrace the graph.
        e.execute("orphan=containers.Map(); orphan('self')=orphan;")
            .unwrap();
        e.remove_array("orphan");
        let retained = e.objects.entries.len();
        for _ in 0..100 {
            e.obj_collect_at_statement(false);
        }
        assert_eq!(e.objects.entries.len(), retained);
        e.obj_collect_at_statement(true);
        assert_eq!(e.objects.entries.len(), retained - 1);
        check(&e);
        e.execute("clear").unwrap();
        assert_eq!(e.objects.bytes, 0);
        assert_eq!(e.objects.sweep_bytes_at, 64 * 1024 * 1024);
        check(&e);
        // Deferred tracing in functions must advance its thresholds as well.
        e.functions.arguments = Some((0, 0));
        e.obj_collect_at_statement(true);
        assert_eq!(e.objects.sweep_at, 1024);
    }

    #[test]
    fn numeric_constructor_checks_limits_before_expansion_and_cancels_during_it() {
        let mut e = Engine::default();
        let mut state = MapState {
            key: "char",
            value: "any",
            entries: BTreeMap::new(),
        };
        let oversized = Array::Numeric(Matrix::filled(1, 8000000, 0.).unwrap());
        EXPANSION_CHECKS.set(0);
        assert!(
            e.obj_map_fill(&mut state, &oversized, &oversized, None)
                .unwrap_err()
                .contains("65536 keys")
        );
        assert_eq!(EXPANSION_CHECKS.get(), 0);
        e.cancel.store(true, Ordering::Relaxed);
        assert_eq!(
            e.obj_map_fill(&mut state, &oversized, &oversized, None)
                .unwrap_err(),
            "Calculation cancelled"
        );
        assert_eq!(EXPANSION_CHECKS.get(), 0);
        e.cancel.store(false, Ordering::Relaxed);
        let small = Array::Numeric(Matrix::filled(1, 2048, 1.).unwrap());
        STOP_EXPANSION_AT.set(Some(1024));
        assert_eq!(
            e.obj_map_fill(&mut state, &small, &small, None)
                .unwrap_err(),
            "Calculation cancelled"
        );
        assert_eq!(EXPANSION_CHECKS.get(), 2);
        assert!(state.entries.is_empty());
        assert!(e.objects.entries.is_empty());
        e.cancel.store(false, Ordering::Relaxed);
        e.execute("m=containers.Map([1 2],[3 4]);").unwrap();
    }
}
