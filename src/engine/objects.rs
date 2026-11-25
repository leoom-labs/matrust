//! containers.Map and inputParser. Both are handle objects: a value
//! is a one-field struct whose shared slot names an engine-owned state, so
//! every copy (`m2 = m`, a function argument, a cell element) sees the same
//! state. Unreachable states, including cycles, are collected at safe workspace
//! boundaries; active evaluations retain their temporary handles.
use super::{Engine, Execution, Value, audio};
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

/// Syntactic root only: do not evaluate a setter receiver to recognize a statement.
fn mpc_set_root(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Name(name) | Expr::Call(name, _) | Expr::CellContents(name, _) => Some(name),
        Expr::Field(base, _)
        | Expr::DynamicField(base, _)
        | Expr::Index(base, _)
        | Expr::Brace(base, _) => mpc_set_root(base),
        _ => None,
    }
}

/// Resolve existing scalar struct/cell lvalues once, freezing side-effecting
/// subscripts and dynamic fields before reusing assign_step for the writeback.
fn mpc_set_target(
    e: &mut Engine,
    expr: &Expr,
    out: &mut Execution,
) -> Result<(String, Vec<PathStep>, Value)> {
    if let Expr::Name(name) = expr {
        let value = e.array(name).ok_or("mpc: set target must be assignable")?;
        return Ok((name.clone(), Vec::new(), Value::from(value)));
    }
    let (base, step) = match expr {
        Expr::Field(base, name) => (base.as_ref().clone(), PathStep::Field(name.clone())),
        Expr::DynamicField(base, key) => (
            base.as_ref().clone(),
            PathStep::Dynamic(key.as_ref().clone()),
        ),
        Expr::Index(base, args) => (base.as_ref().clone(), PathStep::Paren(args.clone())),
        Expr::Brace(base, args) => (base.as_ref().clone(), PathStep::Brace(args.clone())),
        Expr::Call(name, args) => (Expr::Name(name.clone()), PathStep::Paren(args.clone())),
        Expr::CellContents(name, args) => (Expr::Name(name.clone()), PathStep::Brace(args.clone())),
        _ => return Err("mpc: no-output set target must be assignable".into()),
    };
    let (root, mut path, value) = mpc_set_target(e, &base, out)?;
    let step = match step {
        PathStep::Dynamic(key) => PathStep::Field(e.dynamic_field_name(&key, out)?),
        step => step,
    };
    let (step, value) = match step {
        PathStep::Field(name) => {
            let mut values = e.obj_field_values_resolved(value, &name, out)?;
            if values.len() != 1 {
                return Err("mpc: set target must select one assignable controller".into());
            }
            (PathStep::Field(name), values.remove(0))
        }
        PathStep::Paren(args) | PathStep::Brace(args) => {
            let array = value.array(&e.cancel)?;
            let (_, positions) = e.container_positions(&array, &args, out)?;
            if positions.len() != 1 {
                return Err("mpc: set target must select one assignable controller".into());
            }
            let frozen = vec![Expr::Number((positions[0] + 1) as f64)];
            if matches!(expr, Expr::Brace(..) | Expr::CellContents(..)) {
                let mut values = e.brace_values(Value::from(array), &frozen, out)?;
                (PathStep::Brace(frozen), values.remove(0))
            } else {
                let value = e.index_value(Value::from(array), &frozen, out)?;
                (PathStep::Paren(frozen), value)
            }
        }
        PathStep::Dynamic(_) => unreachable!(),
    };
    path.push(step);
    Ok((root, path, value))
}

const TAG: &str = "MatRustHandle";
pub(super) const OBJ_REPORT_HEADER: &str = "MatRustReportHeader";
const MAX_OBJECTS: usize = 65536;
const MAX_KEYS: usize = crate::script_containers::MAX_CELL_SLOTS;
const MAX_PARAMETERS: usize = 4096;
const MAP: &str = "containers.Map";
const PARSER: &str = "inputParser";
const GRIDDED: &str = "griddedInterpolant";
const SCATTERED: &str = "scatteredInterpolant";
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
const EMPTY_CELL: &str = "{1x0 cell}";
const MASK_PREFIX: char = '\u{e000}';
const MASK_DIGITS: usize = 7;

/// Replaces objects with display-only char rows of the same quoted width.
/// The marker prefix and its hex digits must be absent from user text.
#[derive(Default)]
struct ObjMasker {
    used_codes: BTreeSet<u32>,
    next_code: u32,
    by_label: Vec<(String, String)>,
    replacements: Vec<(String, String)>,
}

impl ObjMasker {
    fn scan_codes(&mut self, units: impl Iterator<Item = u32>) {
        let mut remaining = 0;
        let mut code = 0;
        for unit in units {
            if unit == u32::from(MASK_PREFIX) {
                remaining = MASK_DIGITS;
                code = 0;
            } else if remaining != 0 {
                if let Some(digit) = char::from_u32(unit).and_then(|ch| ch.to_digit(16)) {
                    remaining -= 1;
                    code = (code << 4) | digit;
                    if remaining == 0 {
                        self.used_codes.insert(code);
                    }
                } else {
                    remaining = 0;
                }
            }
        }
    }

    fn scan(&mut self, a: &Array, depth: usize) {
        if depth > 8 {
            return;
        }
        match a {
            Array::Character(c) => self.scan_codes(c.units().iter().copied().map(u32::from)),
            Array::Str(s) => {
                for item in s.data().iter().flatten() {
                    self.scan_codes(item.chars().map(u32::from));
                }
            }
            Array::Handle(handle) => self.scan_codes(handle.func2str().chars().map(u32::from)),
            Array::Cell(c) => {
                for i in 0..c.len() {
                    if let Ok(slot) = c.get_linear(i) {
                        self.scan(slot, depth + 1);
                    }
                }
            }
            Array::Struct(s) => {
                for i in 0..s.len() * s.field_count() {
                    if let Ok(slot) = s.get(i % s.field_count(), i / s.field_count()) {
                        self.scan(slot, depth + 1);
                    }
                }
            }
            _ => {}
        }
    }

    fn mask_for(&mut self, label: &str) -> Result<String> {
        if let Some((_, inner)) = self.by_label.iter().find(|(l, _)| l == label) {
            return Ok(inner.clone());
        }
        while self.used_codes.contains(&self.next_code) {
            self.next_code += 1;
        }
        if self.next_code >= 1 << (4 * MASK_DIGITS) {
            return Err("No object display markers are available".into());
        }
        // Struct field summaries use the quoted marker to decide whether a
        // row cell fits inline, so account for the quotes when padding.
        let padding = label.chars().count().saturating_sub(MASK_DIGITS + 3);
        let inner = format!(
            "{MASK_PREFIX}{:0MASK_DIGITS$x}{}",
            self.next_code,
            " ".repeat(padding)
        );
        self.next_code += 1;
        self.by_label.push((label.to_owned(), inner.clone()));
        self.replacements
            .push((format!("'{inner}'"), label.to_owned()));
        Ok(inner)
    }

    fn unmask(&self, text: &str) -> String {
        let mut out = text.to_owned();
        for (needle, label) in &self.replacements {
            out = out.replace(needle, label);
        }
        out
    }
}
/// Kept in sync with sci_more::MAX_POINTS, the shared triangulation bound.
const MAX_SCATTERED_POINTS: usize = 200_000;

fn obj_barycentric(q: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> [f64; 3] {
    let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    let u = ((b[1] - c[1]) * (q[0] - c[0]) + (c[0] - b[0]) * (q[1] - c[1])) / area;
    let v = ((c[1] - a[1]) * (q[0] - c[0]) + (a[0] - c[0]) * (q[1] - c[1])) / area;
    [u, v, 1. - u - v]
}

/// Map keys as text with their values (obj_map_text_entries).
pub(super) type ObjMapEntries = Vec<(String, Arc<Array>)>;

#[derive(Clone, Default)]
pub(super) struct ObjTable {
    entries: BTreeMap<u64, ObjEntry>,
    next: u64,
    sweep_at: usize,
    sweep_bytes_at: usize,
    bytes: usize,
    #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
    audio_entries: usize,
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
    Triangulation(Arc<super::sci_more::triangulation::Triangulation>),
    Video(super::video::Reader),
    VideoWriter(super::video::Writer),
    #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
    Audio(audio::Handle),
    EconVol(Arc<super::econ_vol::Model>),
    Network(Arc<super::nnet::Object>),
    Fis(Arc<super::fuzzy::Object>),
    Arima(Arc<super::econ::Model>),
    Mpc(Arc<super::mpc::MpcObject>),
    MpcState(Arc<super::mpc::state::MpcState>),
    Map(MapState),
    Parser(ParserState),
    Distribution(super::dist_objects::Distribution),
    Regression(Arc<super::linear_models::Model>),
    Classifier(Arc<super::classifier_objects::Classifier>),
    MlPartition(Arc<super::stats_models::partition::Partition>),
    Mlr(Arc<super::stats_models::ml_regression::Model>),
    Filter(Arc<super::digital_filter::DigitalFilter>),
    /// fittype, cfit and sfit (src/engine/curve_fit/).
    Fit(Arc<super::curve_fit::FitObj>),
    Interpolant(InterpolantState),
    Parallel(super::parallel::ParObj),
    /// Import options (import_options/): value semantics, see obj_statement.
    Import(super::import_options::IoObject),
    /// imregconfig optimizer/metric objects (image_basic/registration_config.rs):
    /// value semantics, see obj_statement.
    RegConfig(super::image_basic::registration_config::RegConfig),
    /// imref2d and the 2-D transformation objects (registration/): value semantics.
    Geo(Arc<super::registration::Geo>),
    /// cameraIntrinsics and cameraParameters (camera/): value semantics.
    Camera(Arc<super::camera::Camera>),
    /// 2-D PDEModel (pde_model.rs): mutable handle semantics.
    PdeModel(super::pde_model::PdeModel),
    /// 2-D AnalyticGeometry (pde_model.rs): value semantics.
    PdeGeometry(Arc<super::pde_model::AnalyticGeometry>),
    /// 2-D FEMesh (pde_mesh.rs): value semantics.
    PdeMesh(Arc<super::pde_mesh::FeMesh>),
    /// 2-D StationaryResults (pde_solve.rs): value semantics.
    PdeStationaryResults(Arc<super::pde_solve::StationaryResults>),
    /// 2-D EigenResults (pde_solve/eigen.rs): value semantics.
    PdeEigenResults(Arc<super::pde_solve::EigenResults>),
    /// 2-D TimeDependentResults (pde_solve/time.rs): value semantics.
    PdeTimeDependentResults(Arc<super::pde_solve::time::TimeDependentResults>),
}
pub(super) fn triangulation_value(
    e: &Engine,
    value: &Value,
) -> Option<Arc<super::sci_more::triangulation::Triangulation>> {
    let id = e.obj_value_id(value)?;
    match &e.objects.entries[&id].state {
        ObjState::Triangulation(dt) => Some(dt.clone()),
        _ => None,
    }
}

#[derive(Clone)]
enum InterpolantState {
    Gridded(GriddedState),
    Scattered(ScatteredState),
}
#[derive(Clone)]
struct GriddedState {
    grids: Vec<Arc<Array>>,
    values: Arc<Array>,
    method: String,
    extrapolation: String,
}
#[derive(Clone)]
struct ScatteredState {
    points: Vec<(f64, f64)>,
    values: Arc<Array>,
    method: String,
    extrapolation: String,
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
        Token::DotBackslash => "ldivide",
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
fn obj_is_dfilt(e: &Expr) -> bool {
    matches!(e, Expr::Name(n) if n == "dfilt")
}
/// The digitalFilter an evaluated argument refers to, if any.
pub(super) fn filter_value(
    e: &Engine,
    v: &Value,
) -> Option<Arc<super::digital_filter::DigitalFilter>> {
    let id = e.obj_value_id(v)?;
    match &e.objects.entries.get(&id)?.state {
        ObjState::Filter(f) => Some(f.clone()),
        _ => None,
    }
}
/// Register immutable numerical learner state without copying its training arrays.
pub(super) fn mlr_new(
    e: &mut Engine,
    m: super::stats_models::ml_regression::Model,
) -> Result<Value> {
    e.obj_new(ObjState::Mlr(Arc::new(m)))
}
pub(super) fn arima_value(e: &Engine, v: &Value) -> Option<Arc<super::econ::Model>> {
    let id = e.obj_value_id(v)?;
    match &e.objects.entries.get(&id)?.state {
        ObjState::Arima(m) => Some(m.clone()),
        _ => None,
    }
}
pub(super) fn arima_new(e: &mut Engine, mut m: super::econ::Model) -> Result<Value> {
    super::econ::sync_variance(e, &mut m)?;
    e.obj_new(ObjState::Arima(Arc::new(m)))
}
pub(super) fn mpc_new(e: &mut Engine, m: super::mpc::MpcObject) -> Result<Value> {
    e.obj_new(ObjState::Mpc(Arc::new(m)))
}
pub(super) fn mpc_state_new(e: &mut Engine, s: super::mpc::state::MpcState) -> Result<Value> {
    e.obj_new(ObjState::MpcState(Arc::new(s)))
}
pub(super) fn mpc_value(e: &Engine, v: &Value) -> Option<Arc<super::mpc::MpcObject>> {
    let id = e.obj_value_id(v)?;
    match &e.objects.entries.get(&id)?.state {
        ObjState::Mpc(m) if !e.objects.entries[&id].deleted => Some(m.clone()),
        _ => None,
    }
}
/// Read and replace the shared handle entry, preserving all caller aliases.
pub(super) fn mpc_state_value(
    e: &Engine,
    v: &Value,
) -> Option<(u64, Arc<super::mpc::state::MpcState>)> {
    let id = e.obj_value_id(v)?;
    match &e.objects.entries.get(&id)?.state {
        ObjState::MpcState(s) if !e.objects.entries[&id].deleted => Some((id, s.clone())),
        _ => None,
    }
}
pub(super) fn mpc_state_update(
    e: &mut Engine,
    id: u64,
    s: super::mpc::state::MpcState,
) -> Result<()> {
    let changed = ObjState::MpcState(Arc::new(s));
    let added = Engine::obj_state_bytes(&changed);
    let removed = e.objects.entries[&id].bytes;
    e.obj_check_bytes(removed, added)?;
    let entry = e.objects.entries.get_mut(&id).unwrap();
    entry.state = changed;
    entry.bytes = added;
    e.objects.bytes = e.objects.bytes - removed + added;
    Ok(())
}
/// Build the augmented controller model for host-side inspection.
pub fn mpc_prediction_model(
    e: &mut Engine,
    var: &str,
) -> Result<super::mpc::prediction::MpcPredictionModel> {
    let (_, v) = e.obj_var(var).ok_or("mpc: variable is not a controller")?;
    let m = mpc_value(e, &v).ok_or("mpc: variable is not a controller")?;
    super::mpc::prediction::build(e, &m, &mut Execution::default())
}
pub fn mpc_prediction_ss(e: &Engine, var: &str) -> Option<(Matrix, Matrix, Matrix, Matrix, f64)> {
    let (id, _) = e.obj_var(var)?;
    match &e.objects.entries.get(&id)?.state {
        ObjState::Mpc(m) => Some((m.ad.clone(), m.bd.clone(), m.cd.clone(), m.dd.clone(), m.ts)),
        _ => None,
    }
}
pub(super) fn econ_vol_value(e: &Engine, v: &Value) -> Option<Arc<super::econ_vol::Model>> {
    let id = e.obj_value_id(v)?;
    match &e.objects.entries.get(&id)?.state {
        ObjState::EconVol(m) => Some(m.clone()),
        _ => None,
    }
}
pub(super) fn econ_vol_new(e: &mut Engine, m: super::econ_vol::Model) -> Result<Value> {
    e.obj_new(ObjState::EconVol(Arc::new(m)))
}
pub(super) fn fis_value(e: &Engine, v: &Value) -> Option<Arc<super::fuzzy::Object>> {
    let id = e.obj_value_id(v)?;
    match &e.objects.entries.get(&id)?.state {
        ObjState::Fis(o) => Some(o.clone()),
        _ => None,
    }
}
pub(super) fn fis_comparison(
    e: &Engine,
    a: &Array,
    b: &Array,
) -> Option<(Arc<super::fuzzy::Object>, Arc<super::fuzzy::Object>)> {
    Some((
        fis_value(e, &Value::from(a.clone()))?,
        fis_value(e, &Value::from(b.clone()))?,
    ))
}
pub(super) fn fis_new(e: &mut Engine, o: super::fuzzy::Object) -> Result<Value> {
    e.obj_new(ObjState::Fis(Arc::new(o)))
}
pub(super) fn nnet_value(e: &Engine, v: &Value) -> Option<Arc<super::nnet::Object>> {
    let id = e.obj_value_id(v)?;
    match &e.objects.entries.get(&id)?.state {
        ObjState::Network(n) => Some(n.clone()),
        _ => None,
    }
}
/// Expand only network objects for recursive value equality. Other object
/// classes retain their existing comparison and handle identity semantics.
pub(super) fn nnet_comparison(e: &Engine, a: &Array, b: &Array) -> Result<Option<(Array, Array)>> {
    let (Some(a), Some(b)) = (
        nnet_value(e, &Value::from(a.clone())),
        nnet_value(e, &Value::from(b.clone())),
    ) else {
        return Ok(None);
    };
    Ok(Some((
        super::nnet::equality_state(&a, &e.cancel)?,
        super::nnet::equality_state(&b, &e.cancel)?,
    )))
}
pub(super) fn nnet_new(e: &mut Engine, n: super::nnet::Object) -> Result<Value> {
    e.obj_new(ObjState::Network(Arc::new(n)))
}
/// The import-options object an evaluated argument refers to, if any.
pub(super) fn import_value(e: &Engine, v: &Value) -> Option<super::import_options::IoObject> {
    let id = e.obj_value_id(v)?;
    match &e.objects.entries.get(&id)?.state {
        ObjState::Import(o) => Some(o.clone()),
        _ => None,
    }
}
/// A new import-options object.
pub(super) fn import_new(e: &mut Engine, o: super::import_options::IoObject) -> Result<Value> {
    e.obj_new(ObjState::Import(o))
}
/// The registration optimizer/metric object an evaluated argument refers to, if any.
pub(super) fn reg_config_value(
    e: &Engine,
    v: &Value,
) -> Option<super::image_basic::registration_config::RegConfig> {
    let id = e.obj_value_id(v)?;
    match &e.objects.entries.get(&id)?.state {
        ObjState::RegConfig(c) => Some(*c),
        _ => None,
    }
}
/// A new registration optimizer/metric object.
pub(super) fn reg_config_new(
    e: &mut Engine,
    c: super::image_basic::registration_config::RegConfig,
) -> Result<Value> {
    e.obj_new(ObjState::RegConfig(c))
}
/// Import-options objects as one object (one item) or a 1-by-N array of
/// object handles (a struct array whose elements are objects).
pub(super) fn import_array(
    e: &mut Engine,
    objects: Vec<super::import_options::IoObject>,
) -> Result<Value> {
    if objects.len() == 1 {
        return import_new(e, objects.into_iter().next().unwrap());
    }
    let mut slots = Vec::with_capacity(objects.len());
    for o in objects {
        let Value::Container(Array::Struct(s)) = e.obj_new(ObjState::Import(o))? else {
            unreachable!("objects are structs");
        };
        slots.push(s.get(0, 0)?.clone());
    }
    let n = slots.len();
    Ok(Value::Container(Array::Struct(StructArray::from_slots(
        1,
        n,
        vec![TAG.into()],
        slots,
        &e.cancel,
    )?)))
}
/// The import-options objects of one object or of an array of them.
pub(super) fn import_values(e: &Engine, a: &Array) -> Option<Vec<super::import_options::IoObject>> {
    let Array::Struct(s) = a else { return None };
    if s.field_count() != 1 || s.field_names()[0] != TAG || s.is_empty() {
        return None;
    }
    (0..s.len())
        .map(|i| {
            let id = e.obj_slot_id(s.get(0, i).ok()?)?;
            match &e.objects.entries[&id].state {
                ObjState::Import(o) => Some(o.clone()),
                _ => None,
            }
        })
        .collect()
}

/// The registration object (imref2d, affinetform2d, ...) an argument holds.
pub(super) fn geo_value(e: &Engine, v: &Value) -> Option<Arc<super::registration::Geo>> {
    let id = e.obj_value_id(v)?;
    match &e.objects.entries.get(&id)?.state {
        ObjState::Geo(g) => Some(g.clone()),
        _ => None,
    }
}
/// A new registration object (a fresh entry: value semantics).
pub(super) fn geo_new(e: &mut Engine, g: super::registration::Geo) -> Result<Value> {
    e.obj_new(ObjState::Geo(Arc::new(g)))
}
/// The camera object (cameraIntrinsics, cameraParameters) an argument holds.
pub(super) fn camera_value(e: &Engine, v: &Value) -> Option<Arc<super::camera::Camera>> {
    let id = e.obj_value_id(v)?;
    match &e.objects.entries.get(&id)?.state {
        ObjState::Camera(c) => Some(c.clone()),
        _ => None,
    }
}
/// A new camera object (a fresh entry: value semantics).
pub(super) fn camera_new(e: &mut Engine, c: super::camera::Camera) -> Result<Value> {
    e.obj_new(ObjState::Camera(Arc::new(c)))
}

/// A new 2-D PDEModel handle object.
pub(super) fn pde_model_new(e: &mut Engine, m: super::pde_model::PdeModel) -> Result<Value> {
    e.obj_new(ObjState::PdeModel(m))
}

/// True if `v` refers to a `pde.PDEModel` handle (even if deleted).
pub(super) fn is_pde_model(e: &Engine, v: &Value) -> bool {
    e.obj_value_id(v)
        .is_some_and(|id| matches!(e.objects.entries[&id].state, ObjState::PdeModel(_)))
}

/// Live object ID of a `pde.PDEModel` handle argument.
pub(super) fn pde_model_id(e: &Engine, v: &Value, who: &str) -> Result<u64> {
    let Some(id) = e.obj_value_id(v) else {
        return Err(format!(
            "{who}: first argument must be a pde.PDEModel object"
        ));
    };
    let entry = &e.objects.entries[&id];
    if !matches!(entry.state, ObjState::PdeModel(_)) {
        return Err(format!(
            "{who}: first argument must be a pde.PDEModel object"
        ));
    }
    if entry.deleted {
        return Err(PARSER_DELETED.into());
    }
    Ok(id)
}

/// Live object ID of a workspace variable holding a `pde.PDEModel` handle.
pub(super) fn pde_model_var_id(e: &Engine, name: &str) -> Option<u64> {
    let (id, _) = e.obj_var(name)?;
    let entry = &e.objects.entries[&id];
    (!entry.deleted && matches!(entry.state, ObjState::PdeModel(_))).then_some(id)
}

/// Borrow a `pde.PDEModel` state by handle ID.
pub(super) fn pde_model_get(e: &Engine, id: u64) -> Option<&super::pde_model::PdeModel> {
    let entry = e.objects.entries.get(&id)?;
    if entry.deleted {
        return None;
    }
    match &entry.state {
        ObjState::PdeModel(m) => Some(m),
        _ => None,
    }
}

/// Mutate a `pde.PDEModel` handle in place, updating tracked object bytes.
pub(super) fn pde_model_update<F, R>(e: &mut Engine, id: u64, f: F) -> Result<R>
where
    F: FnOnce(&mut super::pde_model::PdeModel) -> Result<R>,
{
    let mut next = match &e.objects.entries[&id].state {
        ObjState::PdeModel(m) => m.clone(),
        _ => return Err("Expected a pde.PDEModel handle".into()),
    };
    let res = f(&mut next)?;
    let changed = ObjState::PdeModel(next);
    let added = Engine::obj_state_bytes(&changed);
    let removed = e.objects.entries[&id].bytes;
    e.obj_check_bytes(removed, added)?;
    let entry = e.objects.entries.get_mut(&id).unwrap();
    entry.state = changed;
    entry.bytes = added;
    e.objects.bytes = e.objects.bytes - removed + added;
    Ok(res)
}

/// A new `pde.AnalyticGeometry` value object.
pub(super) fn pde_geometry_new(
    e: &mut Engine,
    g: Arc<super::pde_model::AnalyticGeometry>,
) -> Result<Value> {
    e.obj_new(ObjState::PdeGeometry(g))
}

/// The `pde.AnalyticGeometry` an argument holds, if any.
pub(super) fn pde_geometry_value(
    e: &Engine,
    v: &Value,
) -> Option<Arc<super::pde_model::AnalyticGeometry>> {
    let id = e.obj_value_id(v)?;
    let entry = e.objects.entries.get(&id)?;
    if entry.deleted {
        return None;
    }
    match &entry.state {
        ObjState::PdeGeometry(g) => Some(g.clone()),
        _ => None,
    }
}

/// A new `pde.FEMesh` value object.
pub(super) fn pde_mesh_new(e: &mut Engine, m: Arc<super::pde_mesh::FeMesh>) -> Result<Value> {
    e.obj_new(ObjState::PdeMesh(m))
}

/// The `pde.FEMesh` an argument holds, if any.
pub(super) fn pde_mesh_value(e: &Engine, v: &Value) -> Option<Arc<super::pde_mesh::FeMesh>> {
    let id = e.obj_value_id(v)?;
    let entry = e.objects.entries.get(&id)?;
    if entry.deleted {
        return None;
    }
    match &entry.state {
        ObjState::PdeMesh(m) => Some(m.clone()),
        _ => None,
    }
}

/// A new `pde.StationaryResults` value object.
pub(super) fn pde_stationary_results_new(
    e: &mut Engine,
    r: Arc<super::pde_solve::StationaryResults>,
) -> Result<Value> {
    e.obj_new(ObjState::PdeStationaryResults(r))
}

/// A new `pde.EigenResults` value object.
pub(super) fn pde_eigen_results_new(
    e: &mut Engine,
    r: Arc<super::pde_solve::EigenResults>,
) -> Result<Value> {
    e.obj_new(ObjState::PdeEigenResults(r))
}

/// The `pde.StationaryResults` an argument holds, if any.
pub(super) fn pde_stationary_results_value(
    e: &Engine,
    v: &Value,
) -> Option<Arc<super::pde_solve::StationaryResults>> {
    let id = e.obj_value_id(v)?;
    let entry = e.objects.entries.get(&id)?;
    if entry.deleted {
        return None;
    }
    match &entry.state {
        ObjState::PdeStationaryResults(r) => Some(r.clone()),
        _ => None,
    }
}

/// A new `pde.TimeDependentResults` value object.
pub(super) fn pde_time_dependent_results_new(
    e: &mut Engine,
    r: Arc<super::pde_solve::time::TimeDependentResults>,
) -> Result<Value> {
    e.obj_new(ObjState::PdeTimeDependentResults(r))
}

/// The `pde.TimeDependentResults` an argument holds, if any.
pub(super) fn pde_time_dependent_results_value(
    e: &Engine,
    v: &Value,
) -> Option<Arc<super::pde_solve::time::TimeDependentResults>> {
    let id = e.obj_value_id(v)?;
    let entry = e.objects.entries.get(&id)?;
    if entry.deleted {
        return None;
    }
    match &entry.state {
        ObjState::PdeTimeDependentResults(r) => Some(r.clone()),
        _ => None,
    }
}

/// The `pde.EigenResults` an argument holds, if any.
pub(super) fn pde_eigen_results_value(
    e: &Engine,
    v: &Value,
) -> Option<Arc<super::pde_solve::EigenResults>> {
    let id = e.obj_value_id(v)?;
    let entry = e.objects.entries.get(&id)?;
    if entry.deleted {
        return None;
    }
    match &entry.state {
        ObjState::PdeEigenResults(r) => Some(r.clone()),
        _ => None,
    }
}

/// The fittype / cfit / sfit an evaluated argument refers to, if any.
pub(super) fn fit_value(e: &Engine, v: &Value) -> Option<Arc<super::curve_fit::FitObj>> {
    let id = e.obj_value_id(v)?;
    match &e.objects.entries.get(&id)?.state {
        ObjState::Fit(f) => Some(f.clone()),
        _ => None,
    }
}

/// Plain structs can resemble handles; only live slot identity identifies an object.
pub(super) fn save_object_class(engine: &Engine, value: &Array) -> Option<&'static str> {
    let Array::Struct(st) = value else {
        return None;
    };
    if st.field_count() != 1 || st.field_names()[0] != TAG {
        return None;
    }
    (0..st.len()).find_map(|i| {
        let id = engine.obj_slot_id(st.get(0, i).ok()?)?;
        Some(engine.obj_class_name(id))
    })
}

#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
pub(super) fn audio_receiver(engine: &Engine, value: &Value) -> bool {
    engine
        .obj_value_id(value)
        .is_some_and(|id| matches!(engine.objects.entries[&id].state, ObjState::Audio(_)))
}

#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
fn audio_replace(engine: &mut Engine, id: u64, handle: audio::Handle) -> Result<()> {
    let added = handle.bytes();
    let removed = engine.objects.entries[&id].bytes;
    if engine.workspace_bytes() - removed + added > engine.limits.memory_bytes {
        return Err(super::limits::workspace_error(&engine.limits));
    }
    engine.obj_check_bytes(removed, added)?;
    let entry = engine.objects.entries.get_mut(&id).unwrap();
    entry.state = ObjState::Audio(handle);
    entry.bytes = added;
    engine.objects.bytes = engine.objects.bytes - removed + added;
    Ok(())
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
        self.obj_slot_id(s.get(0, 0).ok()?)
    }
    /// The live object whose identity slot is `slot`.
    fn obj_slot_id(&self, slot: &Arc<Array>) -> Option<u64> {
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
    fn obj_is_interpolant(&self, id: u64) -> bool {
        matches!(self.objects.entries[&id].state, ObjState::Interpolant(_))
    }
    fn obj_class_name(&self, id: u64) -> &'static str {
        match &self.objects.entries[&id].state {
            ObjState::Triangulation(_) => super::sci_more::triangulation::CLASS,
            ObjState::Video(_) => "VideoReader",
            ObjState::VideoWriter(_) => "VideoWriter",
            #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
            ObjState::Audio(a) => a.class(),
            ObjState::EconVol(m) => m.class(),
            ObjState::Network(_) => "network",
            ObjState::Fis(o) => o.class,
            ObjState::Arima(_) => "arima",
            ObjState::Mpc(_) => "mpc",
            ObjState::MpcState(_) => "mpcstate",
            ObjState::Map(_) => MAP,
            ObjState::Parser(_) => PARSER,
            ObjState::Interpolant(InterpolantState::Gridded(_)) => GRIDDED,
            ObjState::Interpolant(InterpolantState::Scattered(_)) => SCATTERED,
            ObjState::Distribution(d) => d.class(),
            ObjState::Regression(m) => m.class(),
            ObjState::Parallel(p) => p.class(),
            ObjState::Classifier(c) => c.class(),
            ObjState::MlPartition(_) => "cvpartition",
            ObjState::Mlr(m) => m.class(),
            ObjState::Filter(f) => f.class(),
            ObjState::Import(o) => o.class(),
            ObjState::Fit(f) => f.class(),
            ObjState::RegConfig(c) => c.class(),
            ObjState::Geo(g) => g.class(),
            ObjState::Camera(c) => c.class(),
            ObjState::PdeModel(_) => super::pde_model::MODEL_CLASS,
            ObjState::PdeGeometry(_) => super::pde_model::GEOMETRY_CLASS,
            ObjState::PdeMesh(_) => super::pde_mesh::MESH_CLASS,
            ObjState::PdeStationaryResults(_) => super::pde_solve::RESULTS_CLASS,
            ObjState::PdeEigenResults(_) => super::pde_solve::EIGEN_RESULTS_CLASS,
            ObjState::PdeTimeDependentResults(_) => super::pde_solve::time::TIME_RESULTS_CLASS,
        }
    }
    pub(super) fn lmo_new(&mut self, model: super::linear_models::Model) -> Result<Value> {
        self.obj_new(ObjState::Regression(Arc::new(model)))
    }
    pub(super) fn lmo_get(&self, value: &Value) -> Result<Arc<super::linear_models::Model>> {
        if let Some(id) = self.obj_value_id(value)
            && let ObjState::Regression(model) = &self.objects.entries[&id].state
        {
            return Ok(model.clone());
        }
        Err(format!(
            "predict: unsupported model type {}",
            self.obj_class(value)
        ))
    }
    pub(super) fn dso_new(&mut self, d: super::dist_objects::Distribution) -> Result<Value> {
        self.obj_new(ObjState::Distribution(d))
    }
    /// The distribution object held by `v`, if it is one.
    pub(super) fn dso_get(&self, v: &Value) -> Option<super::dist_objects::Distribution> {
        let id = self.obj_value_id(v)?;
        match &self.objects.entries.get(&id)?.state {
            ObjState::Distribution(d) => Some(d.clone()),
            _ => None,
        }
    }
    pub(super) fn par_new(&mut self, p: super::parallel::ParObj) -> Result<Value> {
        self.obj_new(ObjState::Parallel(p))
    }
    /// Ids of the parallel objects in `v`: one object or a struct array of them.
    pub(super) fn par_ids(&self, v: &Value) -> Option<Vec<u64>> {
        let Value::Container(Array::Struct(s)) = v else {
            return None;
        };
        if s.field_count() != 1 || s.field_names()[0] != TAG {
            return None;
        }
        (0..s.len())
            .map(|k| {
                let slot = s.get(0, k).ok()?;
                let Array::Numeric(m) = slot.as_ref() else {
                    return None;
                };
                let id = *m.data().first()? as u64;
                let entry = self.objects.entries.get(&id)?;
                (Weak::as_ptr(&entry.slot) == Arc::as_ptr(slot)
                    && matches!(entry.state, ObjState::Parallel(_)))
                .then_some(id)
            })
            .collect()
    }
    /// The parallel object `id` and whether it was deleted.
    pub(super) fn par_get(&self, id: u64) -> Option<(&super::parallel::ParObj, bool)> {
        let entry = self.objects.entries.get(&id)?;
        match &entry.state {
            ObjState::Parallel(p) => Some((p, entry.deleted)),
            _ => None,
        }
    }
    pub(super) fn par_set_read(&mut self, id: u64) {
        if let Some(ObjEntry {
            state: ObjState::Parallel(p),
            ..
        }) = self.objects.entries.get_mut(&id)
        {
            p.read = true;
        }
    }
    pub(super) fn ml_partition_new(
        &mut self,
        p: super::stats_models::partition::Partition,
    ) -> Result<Value> {
        self.obj_new(ObjState::MlPartition(Arc::new(p)))
    }
    pub(super) fn ml_partition_value(
        &self,
        v: &Value,
    ) -> Option<Arc<super::stats_models::partition::Partition>> {
        let id = self.obj_value_id(v)?;
        if let ObjState::MlPartition(p) = &self.objects.entries[&id].state {
            Some(p.clone())
        } else {
            None
        }
    }
    pub(super) fn classifier_new(
        &mut self,
        c: super::classifier_objects::Classifier,
    ) -> Result<Value> {
        self.obj_new(ObjState::Classifier(Arc::new(c)))
    }
    pub(super) fn filter_new(&mut self, f: super::digital_filter::DigitalFilter) -> Result<Value> {
        self.obj_new(ObjState::Filter(Arc::new(f)))
    }
    pub(super) fn fit_new(&mut self, f: super::curve_fit::FitObj) -> Result<Value> {
        self.obj_new(ObjState::Fit(Arc::new(f)))
    }
    /// The digitalFilter held in variable `name`, for multi-output calls like `[b,a] = tf(d)`.
    pub(super) fn filter_var(
        &self,
        name: &str,
    ) -> Option<Arc<super::digital_filter::DigitalFilter>> {
        let (id, _) = self.obj_var(name)?;
        match &self.objects.entries[&id].state {
            ObjState::Filter(f) => Some(f.clone()),
            _ => None,
        }
    }
    pub(super) fn obj_predict_outputs(
        &mut self,
        name: &str,
        model: &Value,
        rest: &[Value],
    ) -> Result<Option<Vec<Value>>> {
        let Some(id) = self.obj_value_id(model) else {
            return Err(format!(
                "{name}: unsupported model type {}",
                self.obj_class(model)
            ));
        };
        match &self.objects.entries[&id].state {
            ObjState::Classifier(c) => {
                let c = c.clone();
                if name == "kfoldPredict" {
                    super::classifier_objects::kfold_predict(self, &c, rest).map(Some)
                } else {
                    super::classifier_objects::classifier_predict(self, &c, rest).map(Some)
                }
            }
            ObjState::Mlr(m) => {
                if name == "kfoldPredict" {
                    super::stats_models::ml_regression::kfold_predict(self, m, rest).map(Some)
                } else {
                    super::stats_models::ml_regression::predict(self, m, rest).map(Some)
                }
            }
            ObjState::Regression(_) if name == "predict" => {
                let mut args = vec![model.clone()];
                args.extend_from_slice(rest);
                super::linear_models::model_predict(self, &args, 2).map(Some)
            }
            _ => Err(format!(
                "{name}: unsupported model type {}",
                self.obj_class_name(id)
            )),
        }
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
        let bytes = self.objects.bytes + self.cls_handle_bytes() + self.nested.bytes;
        #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
        let bytes = bytes + self.audio.bytes();
        bytes
    }
    /// Marks and sweeps these objects and the handle objects of user
    /// classes (classdef/handles.rs) together: either can hold the other.
    fn obj_collect(&mut self) {
        self.nested.sweep_unowned();
        if self.objects.entries.is_empty()
            && !self.cls_handles_live()
            && self.nested.roots().next().is_none()
        {
            self.obj_reset_sweep_thresholds();
            return;
        }
        // Expression temporaries are Rust locals, not workspace roots. Do not
        // trace while an expression or a function invocation can own them
        // (evalin('base', ...) inside a function clears `arguments`).
        if self.evaluation_depth != 0
            || self.functions.arguments.is_some()
            || self.cls_in_function()
        {
            self.obj_collect_unowned();
            // Between two statements of a user function: handle-object
            // cycles made since it was entered (classdef/handles.rs).
            self.cls_collect_function_cycles();
            self.cls_reset_sweep_threshold();
            self.obj_reset_sweep_thresholds();
            return;
        }
        let mut marked = BTreeSet::new();
        let mut marked_handles = BTreeSet::new();
        let mut slots = BTreeSet::new();
        if !self.obj_mark(
            self.obj_workspace_roots().collect(),
            &mut marked,
            &mut marked_handles,
            &mut slots,
        ) {
            return; // Never sweep a partially marked graph.
        }
        let mut bytes = self.objects.bytes;
        #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
        let mut audio_entries = self.objects.audio_entries;
        self.objects.entries.retain(|id, entry| {
            let keep = marked.contains(id);
            if !keep {
                bytes -= entry.bytes;
                #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
                if let ObjState::Audio(handle) = &entry.state {
                    handle.stop();
                    audio_entries -= 1;
                }
            }
            keep
        });
        self.objects.bytes = bytes;
        #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
        {
            self.objects.audio_entries = audio_entries;
        }
        self.cls_retain_marked(&marked_handles);
        self.nested.retain_marked(&slots);
        self.obj_reset_sweep_thresholds();
    }
    /// Workspace roots of the shared mark phase: every workspace, globals,
    /// persistents, cleanup tasks, the class registry and the current pool.
    pub(super) fn obj_roots(&self) -> impl Iterator<Item = &Array> {
        self.obj_workspace_roots().chain(self.nested.roots())
    }

    fn obj_workspace_roots(&self) -> impl Iterator<Item = &Array> {
        self.container_variables
            .values()
            .chain(self.functions.lng.obj_roots())
            .chain(self.cls_roots())
            .chain(self.par.roots())
            .chain(self.ui.roots())
    }
    /// Mark phase shared by [`Self::obj_collect`] and the handle-cycle
    /// collection inside functions (classdef/handles.rs): adds every object
    /// and handle object reachable from `pending` to `marked` /
    /// `marked_handles` (entries already there count as traced). False when
    /// cancelled: the marks are then incomplete.
    pub(super) fn obj_mark<'a>(
        &'a self,
        mut pending: Vec<&'a Array>,
        marked: &mut BTreeSet<u64>,
        marked_handles: &mut BTreeSet<u64>,
        slots: &mut BTreeSet<usize>,
    ) -> bool {
        let mut arrays = BTreeSet::new();
        while let Some(a) = pending.pop() {
            if self.check_cancel().is_err() {
                return false;
            }
            if !matches!(
                a,
                Array::Cell(_) | Array::Struct(_) | Array::Handle(_) | Array::Table(_)
            ) {
                continue;
            }
            if !arrays.insert(a as *const Array as usize) {
                continue;
            }
            if let Some((id, storage)) = self.cls_mark(a) {
                if marked_handles.insert(id) {
                    pending.extend(storage);
                }
            } else if let Some(id) = self.obj_id(a) {
                if marked.insert(id) {
                    self.obj_children(id, &mut pending, slots);
                }
            } else {
                match a {
                    Array::Cell(c) => pending.extend(
                        (0..c.len())
                            .filter_map(|i| c.get_linear(i).ok())
                            .map(AsRef::as_ref),
                    ),
                    // A 1-by-N array of objects (import_array): mark each.
                    Array::Struct(s) if s.field_count() == 1 && s.field_names()[0] == TAG => {
                        for i in 0..s.slot_count() {
                            if let Some(id) = s.slot(i).ok().and_then(|x| self.obj_slot_id(x))
                                && marked.insert(id)
                            {
                                self.obj_children(id, &mut pending, slots);
                            }
                        }
                    }
                    Array::Struct(s) => pending.extend(
                        (0..s.slot_count())
                            .filter_map(|i| s.slot(i).ok())
                            .map(AsRef::as_ref),
                    ),
                    Array::Handle(h) => {
                        self.nested.mark_handle(h, slots, &mut pending);
                        if let Some((app, cb, _)) = h.app_callback_parts() {
                            pending.push(app);
                            self.nested.mark_handle(cb, slots, &mut pending);
                        }
                    }
                    Array::Table(t) => pending.extend(t.columns()),
                    _ => {}
                }
            }
        }
        true
    }
    /// What the mark phase traces inside object `id`.
    fn obj_children<'a>(
        &'a self,
        id: u64,
        pending: &mut Vec<&'a Array>,
        slots: &mut BTreeSet<usize>,
    ) {
        match &self.objects.entries[&id].state {
            ObjState::Video(v) => pending.push(v.user.as_ref()),
            ObjState::VideoWriter(_) => {}
            #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
            ObjState::Audio(a) => pending.push(a.user.as_ref()),
            ObjState::EconVol(_) => {}
            ObjState::Network(n) => pending.extend(n.props.values()),
            ObjState::Fis(o) => pending.extend(o.children()),
            ObjState::Arima(m) => pending.extend(m.conditional.iter().map(|v| &v.object)),
            ObjState::Mpc(m) => {
                pending.push(&m.plant_array);
                pending.push(&m.disturbance);
                pending.push(&m.noise);
                pending.push(&m.output_disturbance);
                pending.push(&m.notes);
                pending.push(&m.user_data);
            }
            ObjState::MpcState(_) | ObjState::Distribution(_) | ObjState::Regression(_) => {}
            ObjState::Parallel(p) => pending.extend(p.outputs.iter().map(AsRef::as_ref)),
            ObjState::Classifier(_)
            | ObjState::MlPartition(_)
            | ObjState::Mlr(_)
            | ObjState::Filter(_)
            | ObjState::Import(_)
            | ObjState::RegConfig(_)
            | ObjState::Geo(_)
            | ObjState::Triangulation(_)
            | ObjState::Camera(_)
            | ObjState::PdeGeometry(_)
            | ObjState::PdeMesh(_) => {}
            ObjState::PdeModel(m) => m.push_children(pending),
            ObjState::PdeStationaryResults(r) => r.push_children(pending),
            ObjState::PdeEigenResults(r) => r.push_children(pending),
            ObjState::PdeTimeDependentResults(r) => r.push_children(pending),
            ObjState::Fit(f) => {
                for h in f.handles() {
                    self.nested.mark_handle(h, slots, pending);
                }
            }
            ObjState::Map(m) => {
                pending.extend(m.entries.values().map(|(_, v)| v.as_ref()));
            }
            ObjState::Parser(p) => {
                for arg in &p.args {
                    pending.extend(arg.default.as_deref());
                    if let Some(h) = &arg.validator {
                        self.nested.mark_handle(h, slots, pending);
                    }
                }
                pending.extend(
                    p.results
                        .iter()
                        .chain(&p.unmatched)
                        .map(|(_, a)| a.as_ref()),
                );
            }
            ObjState::Interpolant(InterpolantState::Gridded(g)) => {
                pending.extend(g.grids.iter().map(AsRef::as_ref));
                pending.push(g.values.as_ref());
            }
            ObjState::Interpolant(InterpolantState::Scattered(s)) => {
                pending.push(s.values.as_ref())
            }
        }
    }
    /// For the handle-cycle collection inside functions: every object still
    /// referenced anywhere counts as reachable (an expression of a caller may
    /// hold it), with its contents.
    pub(super) fn obj_mark_referenced<'a>(
        &'a self,
        marked: &mut BTreeSet<u64>,
        pending: &mut Vec<&'a Array>,
    ) {
        for (id, entry) in &self.objects.entries {
            if entry.slot.strong_count() > 0 && marked.insert(*id) {
                self.obj_children(*id, pending, &mut BTreeSet::new());
            }
        }
    }
    fn obj_reset_sweep_thresholds(&mut self) {
        self.objects.sweep_at = self.objects.entries.len().saturating_mul(2).max(1024);
        self.objects.sweep_bytes_at = self.objects.bytes.saturating_mul(2).max(64 * 1024 * 1024);
    }
    /// Only call where no active assignment or expression can own a temporary.
    pub(super) fn obj_collect_at_statement(&mut self, force: bool) {
        #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
        if self.objects.audio_entries > 0 {
            self.obj_collect_unowned_audio();
        }
        if force
            || self.objects.entries.len() >= self.objects.sweep_at.max(1024)
            || self.objects.bytes >= self.objects.sweep_bytes_at.max(64 * 1024 * 1024)
            || self.cls_sweep_due()
            || self.nested.sweep_due()
                && self.evaluation_depth == 0
                && self.functions.arguments.is_none()
        {
            self.obj_collect();
        }
    }
    #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
    fn obj_collect_unowned_audio(&mut self) {
        let mut bytes = self.objects.bytes;
        let mut audio_entries = self.objects.audio_entries;
        let mut maybe_cycle = false;
        self.objects.entries.retain(|_, e| {
            if let ObjState::Audio(handle) = &e.state {
                if e.slot.strong_count() == 0 {
                    handle.stop();
                    bytes -= e.bytes;
                    audio_entries -= 1;
                    return false;
                }
                if handle.active()
                    && matches!(
                        handle.user.as_ref(),
                        Array::Cell(_) | Array::Struct(_) | Array::Handle(_)
                    )
                {
                    maybe_cycle = true;
                }
            }
            true
        });
        self.objects.bytes = bytes;
        self.objects.audio_entries = audio_entries;
        if maybe_cycle {
            self.obj_collect();
        }
    }
    pub(super) fn obj_collect_unowned(&mut self) {
        self.nested.sweep_unowned();
        self.cls_sweep_unowned();
        let mut bytes = self.objects.bytes;
        #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
        let mut audio_entries = self.objects.audio_entries;
        self.objects.entries.retain(|_, e| {
            let keep = e.slot.strong_count() > 0;
            if !keep {
                bytes -= e.bytes;
                #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
                if let ObjState::Audio(handle) = &e.state {
                    handle.stop();
                    audio_entries -= 1;
                }
            }
            keep
        });
        self.objects.bytes = bytes;
        #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
        {
            self.objects.audio_entries = audio_entries;
        }
    }
    fn obj_check_bytes(&mut self, removed: usize, added: usize) -> Result<()> {
        let limit = super::limits::memory_bytes(&self.limits);
        let projected = |engine: &Self| {
            engine
                .workspace_bytes()
                .checked_sub(removed)
                .and_then(|n| n.checked_add(added))
                .ok_or("Workspace byte accounting overflow")
        };
        if projected(self)? > limit {
            // Weak ownership remains safe even when Rust locals hold handles.
            self.obj_collect_unowned();
            if projected(self)? > limit {
                return Err(super::limits::workspace_error(&self.limits));
            }
        }
        Ok(())
    }
    fn obj_state_bytes(state: &ObjState) -> usize {
        match state {
            ObjState::Triangulation(dt) => dt.bytes(),
            ObjState::Video(v) => v.bytes(),
            ObjState::VideoWriter(v) => v.bytes(),
            #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
            ObjState::Audio(a) => a.bytes(),
            ObjState::EconVol(m) => m.bytes(),
            ObjState::Network(n) => n.bytes(),
            ObjState::Fis(o) => o.bytes(),
            ObjState::Arima(m) => m.bytes(),
            ObjState::Mpc(m) => m.bytes(),
            ObjState::MpcState(s) => s.bytes(),
            ObjState::Distribution(d) => d.bytes(),
            ObjState::Regression(m) => m.bytes(),
            ObjState::Parallel(p) => {
                64 + p.outputs.iter().map(|a| a.retained_bytes()).sum::<usize>()
            }
            ObjState::Classifier(c) => c.bytes(),
            ObjState::MlPartition(p) => p.bytes(),
            ObjState::Mlr(m) => m.bytes(),
            ObjState::Filter(f) => f.bytes(),
            ObjState::Import(o) => o.bytes(),
            ObjState::Fit(f) => f.bytes(),
            ObjState::RegConfig(_) => std::mem::size_of::<ObjState>() + 64,
            ObjState::Geo(g) => g.bytes(),
            ObjState::Camera(c) => c.bytes(),
            ObjState::PdeModel(m) => m.bytes(),
            ObjState::PdeGeometry(g) => g.bytes(),
            ObjState::PdeMesh(m) => m.bytes(),
            ObjState::PdeStationaryResults(r) => r.bytes(),
            ObjState::PdeEigenResults(r) => r.bytes(),
            ObjState::PdeTimeDependentResults(r) => r.bytes(),
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
            ObjState::Interpolant(InterpolantState::Gridded(g)) => {
                g.grids.iter().map(|a| a.retained_bytes()).sum::<usize>()
                    + g.values.retained_bytes()
                    + g.method.len()
                    + g.extrapolation.len()
            }
            ObjState::Interpolant(InterpolantState::Scattered(s)) => {
                s.points.len() * std::mem::size_of::<(f64, f64)>()
                    + s.values.retained_bytes()
                    + s.method.len()
                    + s.extrapolation.len()
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
            return Err(format!("At most {MAX_OBJECTS} handle objects can exist"));
        }
        table.next += 1;
        let id = table.next;
        let slot = Arc::new(Array::Numeric(Matrix::scalar(id as f64)));
        let object =
            StructArray::from_slots(1, 1, vec![TAG.into()], vec![slot.clone()], &self.cancel)?;
        self.objects.bytes += bytes;
        #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
        if matches!(state, ObjState::Audio(_)) {
            self.objects.audio_entries += 1;
        }
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
        #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
        match name {
            "sound" | "soundsc" => {
                return audio::sound(self, args, name == "soundsc", out).map(Some);
            }
            "audioplayer" | "audiorecorder" => {
                let handle = if name == "audioplayer" {
                    audio::Handle::player(self, args)?
                } else {
                    audio::Handle::recorder(self, args)?
                };
                return self.obj_new(ObjState::Audio(handle)).map(Some);
            }
            _ if args.first().is_some_and(|v| audio_receiver(self, v)) => {
                return self.obj_method(
                    &args[0],
                    self.obj_value_id(&args[0]).unwrap(),
                    name,
                    &args[1..],
                    out,
                );
            }
            _ => {}
        }
        #[cfg(not(all(feature = "audio", not(target_arch = "wasm32"))))]
        if matches!(
            name,
            "sound"
                | "soundsc"
                | "audioplayer"
                | "audiorecorder"
                | "play"
                | "playblocking"
                | "resume"
                | "stop"
                | "isplaying"
                | "record"
                | "recordblocking"
                | "getaudiodata"
                | "getplayer"
        ) {
            let value = audio::unavailable(self, out)?;
            return Ok(Some(
                if matches!(
                    name,
                    "sound"
                        | "soundsc"
                        | "play"
                        | "playblocking"
                        | "resume"
                        | "stop"
                        | "record"
                        | "recordblocking"
                ) {
                    Value::Void
                } else {
                    value
                },
            ));
        }
        if name != "writeVideo" {
            super::nd::guard(name, args)?;
        }
        if !matches!(name, "VideoWriter" | "writeVideo") {
            super::string_class::guard(name, args)?;
        }
        match name {
            super::sci_more::triangulation::CLASS => {
                let dt = super::sci_more::triangulation::Triangulation::new(self, args)?;
                return self
                    .obj_new(ObjState::Triangulation(Arc::new(dt)))
                    .map(Some);
            }
            "VideoWriter" => {
                let writer = super::video::Writer::new(self, args)?;
                return self.obj_new(ObjState::VideoWriter(writer)).map(Some);
            }
            "VideoReader" => {
                let reader = super::video::Reader::new(self, args)?;
                return self.obj_new(ObjState::Video(reader)).map(Some);
            }
            GRIDDED => return self.obj_gridded_new(args).map(Some),
            SCATTERED => return self.obj_scattered_new(args).map(Some),
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
        if let ObjState::Triangulation(dt) = &self.objects.entries[&id].state {
            use super::sci_more::triangulation::{CLASS, METHODS, PROPERTIES};
            match name {
                name if METHODS.contains(&name) => {
                    let dt = dt.clone();
                    return dt.call(self, name, rest, 1).map(|mut v| v.pop());
                }
                "properties" | "fieldnames" => {
                    return Ok(Some(self.obj_names(&PROPERTIES.map(String::from))?.into()));
                }
                "isprop" | "isa" => {
                    let [kind] = rest else {
                        return Err(format!("{name}: expected an object and a name"));
                    };
                    let kind = self.text_value(kind.clone())?;
                    let found = if name == "isprop" {
                        PROPERTIES.contains(&kind.as_str())
                    } else {
                        matches!(kind.as_str(), CLASS | "triangulation")
                    };
                    return Ok(Some(Value::Logical(LogicalMatrix::scalar(found))));
                }
                "delete" | "eq" | "ne" => return Err(self.obj_undefined(name, Some(this))),
                _ => {}
            }
        }
        #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
        if let ObjState::Audio(handle) = &self.objects.entries[&id].state
            && (matches!(
                name,
                "properties" | "fieldnames" | "get" | "set" | "getplayer" | "delete"
            ) || (handle.methods().contains(&name) && !matches!(name, "disp" | "isvalid")))
        {
            if self.objects.entries[&id].deleted {
                return Err(PARSER_DELETED.into());
            }
            let mut handle = handle.clone();
            let value = match (name, rest) {
                ("properties" | "fieldnames", []) => {
                    let names: Vec<_> = handle.properties().iter().map(|&n| n.to_owned()).collect();
                    self.obj_names(&names)?.into()
                }
                ("get", []) => {
                    let fields = handle
                        .properties()
                        .iter()
                        .map(|&n| {
                            let value = handle.property(n)?.array(&self.cancel)?;
                            Ok((n.to_owned(), Arc::new(value)))
                        })
                        .collect::<Result<Vec<_>>>()?;
                    self.obj_struct(&fields)?.into()
                }
                ("get", [property]) => handle.property(&audio::text(self, property)?)?,
                ("set", pairs) if !pairs.is_empty() && pairs.len().is_multiple_of(2) => {
                    for [property, value] in pairs.as_chunks::<2>().0 {
                        handle.assign(self, &audio::text(self, property)?, value.clone())?;
                    }
                    audio_replace(self, id, handle)?;
                    Value::Void
                }
                ("getplayer", []) if handle.class() == "audiorecorder" => {
                    let player = handle.getplayer(self)?;
                    self.obj_new(ObjState::Audio(player))?
                }
                ("delete", []) => {
                    handle.stop();
                    self.objects.entries.get_mut(&id).unwrap().deleted = true;
                    Value::Void
                }
                ("record" | "recordblocking", _) => {
                    let result = handle.operate(self, name, rest, out);
                    // A cancelled or failed blocking capture may still retain samples.
                    let entry = self.objects.entries.get_mut(&id).unwrap();
                    let bytes = handle.bytes();
                    self.objects.bytes = self.objects.bytes - entry.bytes + bytes;
                    entry.bytes = bytes;
                    entry.state = ObjState::Audio(handle);
                    result?
                }
                _ => handle.operate(self, name, rest, out)?,
            };
            return Ok(Some(value));
        }
        if let ObjState::Geo(g) = &self.objects.entries[&id].state {
            let g = g.clone();
            if let Some(value) = super::registration::object_method(self, &g, name, rest, out)? {
                return Ok(Some(value));
            }
        }
        if let ObjState::Camera(c) = &self.objects.entries[&id].state {
            let c = c.clone();
            if let Some(value) = super::camera::object_method(self, &c, name, rest, out)? {
                return Ok(Some(value));
            }
        }
        if let ObjState::PdeModel(m) = &self.objects.entries[&id].state {
            let deleted = self.objects.entries[&id].deleted;
            match name {
                "get" => {
                    if deleted {
                        return Err(PARSER_DELETED.into());
                    }
                    let m = m.clone();
                    return m.get_method(self, rest).map(Some);
                }
                "properties" | "fieldnames" => {
                    if !rest.is_empty() {
                        return Err(MANY.into());
                    }
                    let names: Vec<String> = super::pde_model::MODEL_PROPERTIES
                        .iter()
                        .map(|s| (*s).to_owned())
                        .collect();
                    return Ok(Some(self.obj_names(&names)?.into()));
                }
                "geometryFromEdges"
                | "pdegplot"
                | "generateMesh"
                | "pdemesh"
                | "pdeplot"
                | "specifyCoefficients"
                | "applyBoundaryCondition"
                | "setInitialConditions"
                | "solvepde"
                | "solvepdeeig" => {
                    let mut all = Vec::with_capacity(rest.len() + 1);
                    all.push(this.clone());
                    all.extend_from_slice(rest);
                    let mut vals = super::pde_model::call(self, name, &all, 1, out)?;
                    return Ok(Some(vals.pop().unwrap_or(Value::Void)));
                }
                _ => {}
            }
        }
        if let ObjState::PdeGeometry(g) = &self.objects.entries[&id].state {
            let g = g.clone();
            match name {
                "isa" => {
                    let [kind] = rest else {
                        return Err(if rest.is_empty() { FEW } else { MANY }.into());
                    };
                    let kind = kind.clone().array(&self.cancel)?;
                    let ok = obj_text(&kind).is_some_and(|k| k == super::pde_model::GEOMETRY_CLASS);
                    return Ok(Some(Value::Logical(LogicalMatrix::scalar(ok))));
                }
                "get" => return g.get_method(self, rest).map(Some),
                "properties" | "fieldnames" => {
                    if !rest.is_empty() {
                        return Err(MANY.into());
                    }
                    let names: Vec<String> = super::pde_model::GEOMETRY_PROPERTIES
                        .iter()
                        .map(|s| (*s).to_owned())
                        .collect();
                    return Ok(Some(self.obj_names(&names)?.into()));
                }
                "pdegplot" => {
                    let mut all = Vec::with_capacity(rest.len() + 1);
                    all.push(this.clone());
                    all.extend_from_slice(rest);
                    let mut vals = super::pde_model::call(self, name, &all, 1, out)?;
                    return Ok(Some(vals.pop().unwrap_or(Value::Void)));
                }
                _ => {}
            }
        }
        if let ObjState::PdeMesh(m) = &self.objects.entries[&id].state {
            let m = m.clone();
            match name {
                "isa" => {
                    let [kind] = rest else {
                        return Err(if rest.is_empty() { FEW } else { MANY }.into());
                    };
                    let kind = kind.clone().array(&self.cancel)?;
                    let ok = obj_text(&kind).is_some_and(|k| k == super::pde_mesh::MESH_CLASS);
                    return Ok(Some(Value::Logical(LogicalMatrix::scalar(ok))));
                }
                "get" => return m.get_method(self, rest).map(Some),
                "properties" | "fieldnames" => {
                    if !rest.is_empty() {
                        return Err(MANY.into());
                    }
                    let names: Vec<String> = super::pde_mesh::MESH_PROPERTIES
                        .iter()
                        .map(|s| (*s).to_owned())
                        .collect();
                    return Ok(Some(self.obj_names(&names)?.into()));
                }
                "findNodes" | "findElements" | "pdemesh" => {
                    let mut all = Vec::with_capacity(rest.len() + 1);
                    all.push(this.clone());
                    all.extend_from_slice(rest);
                    let mut vals = super::pde_mesh::call(self, name, &all, 1, out)?;
                    return Ok(Some(vals.pop().unwrap_or(Value::Void)));
                }
                "pdeplot" => {
                    let mut all = Vec::with_capacity(rest.len() + 1);
                    all.push(this.clone());
                    all.extend_from_slice(rest);
                    let mut vals = super::pde_model::call(self, name, &all, 1, out)?;
                    return Ok(Some(vals.pop().unwrap_or(Value::Void)));
                }
                _ => {}
            }
        }
        if let ObjState::PdeStationaryResults(r) = &self.objects.entries[&id].state {
            let r = r.clone();
            match name {
                "isa" => {
                    let [kind] = rest else {
                        return Err(if rest.is_empty() { FEW } else { MANY }.into());
                    };
                    let kind = kind.clone().array(&self.cancel)?;
                    let ok = obj_text(&kind).is_some_and(|k| k == super::pde_solve::RESULTS_CLASS);
                    return Ok(Some(Value::Logical(LogicalMatrix::scalar(ok))));
                }
                "get" => return r.get_method(self, rest).map(Some),
                "properties" | "fieldnames" => {
                    if !rest.is_empty() {
                        return Err(MANY.into());
                    }
                    let names: Vec<String> = super::pde_solve::RESULTS_PROPERTIES
                        .iter()
                        .map(|s| (*s).to_owned())
                        .collect();
                    return Ok(Some(self.obj_names(&names)?.into()));
                }
                _ => {}
            }
        }
        if let ObjState::PdeEigenResults(r) = &self.objects.entries[&id].state {
            let r = r.clone();
            match name {
                "isa" => {
                    let [kind] = rest else {
                        return Err(if rest.is_empty() { FEW } else { MANY }.into());
                    };
                    let kind = kind.clone().array(&self.cancel)?;
                    let ok =
                        obj_text(&kind).is_some_and(|k| k == super::pde_solve::EIGEN_RESULTS_CLASS);
                    return Ok(Some(Value::Logical(LogicalMatrix::scalar(ok))));
                }
                "get" => return r.get_method(self, rest).map(Some),
                "properties" | "fieldnames" => {
                    if !rest.is_empty() {
                        return Err(MANY.into());
                    }
                    let names: Vec<String> = super::pde_solve::EIGEN_RESULTS_PROPERTIES
                        .iter()
                        .map(|s| (*s).to_owned())
                        .collect();
                    return Ok(Some(self.obj_names(&names)?.into()));
                }
                _ => {}
            }
        }
        if let ObjState::PdeTimeDependentResults(r) = &self.objects.entries[&id].state {
            let r = r.clone();
            match name {
                "isa" => {
                    let [kind] = rest else {
                        return Err(if rest.is_empty() { FEW } else { MANY }.into());
                    };
                    let kind = kind.clone().array(&self.cancel)?;
                    let ok = obj_text(&kind)
                        .is_some_and(|k| k == super::pde_solve::time::TIME_RESULTS_CLASS);
                    return Ok(Some(Value::Logical(LogicalMatrix::scalar(ok))));
                }
                "get" => return r.get_method(self, rest).map(Some),
                "properties" | "fieldnames" => {
                    if !rest.is_empty() {
                        return Err(MANY.into());
                    }
                    let names: Vec<String> = super::pde_solve::time::TIME_RESULTS_PROPERTIES
                        .iter()
                        .map(|s| (*s).to_owned())
                        .collect();
                    return Ok(Some(self.obj_names(&names)?.into()));
                }
                _ => {}
            }
        }
        if let ObjState::VideoWriter(writer) = &self.objects.entries[&id].state {
            if matches!(name, "properties" | "fieldnames") {
                let names = writer.property_names();
                return Ok(Some(self.obj_names(&names)?.into()));
            }
            if matches!(name, "open" | "writeVideo" | "close" | "delete") {
                if self.objects.entries[&id].deleted {
                    return Err(PARSER_DELETED.into());
                }
                let writer = writer.clone();
                let result = writer.call(self, name, rest);
                let entry = self.objects.entries.get_mut(&id).unwrap();
                self.objects.bytes = self.objects.bytes - entry.bytes + writer.bytes();
                entry.bytes = writer.bytes();
                if name == "delete" && result.is_ok() {
                    entry.deleted = true;
                }
                return result.map(Some);
            }
        }
        if let ObjState::Video(reader) = &self.objects.entries[&id].state
            && ["hasFrame", "readFrame", "read"].contains(&name)
        {
            if self.objects.entries[&id].deleted {
                return Err(PARSER_DELETED.into());
            }
            let reader = reader.clone();
            let (result, changed) = super::video::call(self, &reader, name, rest)?;
            if let Some(reader) = changed {
                self.objects.entries.get_mut(&id).unwrap().state = ObjState::Video(reader);
            }
            return Ok(Some(result));
        }
        if let ObjState::Video(reader) = &self.objects.entries[&id].state {
            if matches!(name, "properties" | "fieldnames") {
                let names = reader.property_names();
                return Ok(Some(self.obj_names(&names)?.into()));
            }
            if !matches!(
                name,
                "class"
                    | "isa"
                    | "isobject"
                    | "isvalid"
                    | "isstruct"
                    | "iscell"
                    | "isnumeric"
                    | "ischar"
                    | "islogical"
                    | "isfloat"
                    | "delete"
                    | "eq"
                    | "ne"
                    | "isequal"
                    | "disp"
                    | "display"
                    | "length"
                    | "size"
                    | "isempty"
                    | "numel"
                    | "ndims"
            ) {
                return Err(format!(
                    "No appropriate method '{name}' for class 'VideoReader'."
                ));
            }
        }
        if let ObjState::EconVol(m) = &self.objects.entries[&id].state {
            let m = m.clone();
            if super::econ_vol::METHODS.contains(&name) {
                let mut args = vec![this.clone()];
                args.extend_from_slice(rest);
                return Ok(super::econ_vol::call(self, name, &args, 1, out)?
                    .into_iter()
                    .next());
            }
            if matches!(name, "properties" | "fieldnames") {
                let names = super::econ_vol::property_names(&m);
                return Ok(Some(self.obj_names(&names)?.into()));
            }
        }
        if let ObjState::Fis(o) = &self.objects.entries[&id].state {
            let o = o.clone();
            if let Some(v) = super::fuzzy::properties::method(self, &o, name, rest, out)? {
                return Ok(Some(v));
            }
        }
        if let ObjState::Network(n) = &self.objects.entries[&id].state {
            let n = n.clone();
            if let Some(value) = super::nnet::object_method(self, &n, name, rest)? {
                return Ok(Some(value));
            }
        }
        if let ObjState::Import(o) = &self.objects.entries[&id].state {
            let o = o.clone();
            if let Some(v) = super::import_options::method(self, &o, name, rest, out)? {
                return Ok(Some(v));
            }
        }
        if let ObjState::MpcState(s) = &self.objects.entries[&id].state {
            if matches!(name, "properties" | "fieldnames") {
                let names = super::mpc::state::PROPERTIES
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>();
                return Ok(Some(self.obj_names(&names)?.into()));
            }
            if name == "get" {
                let [v] = rest else {
                    return Err("mpcstate: get expects one property name".into());
                };
                let a = v.clone().array(&self.cancel)?;
                return super::mpc::state::property(
                    s,
                    &obj_text(&a).ok_or("mpcstate: property name must be text")?,
                )
                .map(Some);
            }
            if name == "set" {
                if !rest.len().is_multiple_of(2) {
                    return Err("mpcstate: set expects property/value pairs".into());
                }
                let mut next = s.as_ref().clone();
                for pair in rest.as_chunks::<2>().0 {
                    let prop = pair[0].clone().array(&self.cancel)?;
                    let value = pair[1].clone().array(&self.cancel)?;
                    next = super::mpc::state::assign(
                        &next,
                        &obj_text(&prop).ok_or("mpcstate: property name must be text")?,
                        &value,
                    )?;
                }
                let changed = ObjState::MpcState(Arc::new(next));
                let added = Self::obj_state_bytes(&changed);
                let removed = self.objects.entries[&id].bytes;
                self.obj_check_bytes(removed, added)?;
                let entry = self.objects.entries.get_mut(&id).unwrap();
                entry.state = changed;
                entry.bytes = added;
                self.objects.bytes = self.objects.bytes - removed + added;
                return Ok(Some(Value::Void));
            }
        }
        if let ObjState::Mpc(m) = &self.objects.entries[&id].state {
            let m = m.clone();
            match name {
                "isa" => {
                    let [kind] = rest else {
                        return Err(if rest.is_empty() { FEW } else { MANY }.into());
                    };
                    let kind = kind.clone().array(&self.cancel)?;
                    let kind = obj_text(&kind).unwrap_or_default();
                    return Ok(Some(Value::Logical(LogicalMatrix::scalar(kind == "mpc"))));
                }
                "get" => return super::mpc::get_method(self, &m, rest).map(Some),
                "set" => return super::mpc::set_method(self, &m, rest, out).map(Some),
                "properties" | "fieldnames" => {
                    let names = super::mpc::property_names();
                    return Ok(Some(self.obj_names(&names)?.into()));
                }
                _ => {}
            }
        }
        if let ObjState::RegConfig(c) = &self.objects.entries[&id].state {
            let c = *c;
            let logical = |b: bool| -> Result<Option<Value>> {
                Ok(Some(Value::Logical(LogicalMatrix::scalar(b))))
            };
            match name {
                // Value classes: isa(obj, 'handle') is false.
                "isa" => {
                    let [kind] = rest else {
                        return Err(if rest.is_empty() { FEW } else { MANY }.into());
                    };
                    let kind = kind.clone().array(&self.cancel)?;
                    return logical(obj_text(&kind).is_some_and(|k| k == c.class()));
                }
                "properties" | "fieldnames" => {
                    if !rest.is_empty() {
                        return Err(MANY.into());
                    }
                    return Ok(Some(self.obj_names(&c.property_names())?.into()));
                }
                "isequal" => {
                    if rest.is_empty() {
                        return Err(FEW.into());
                    }
                    let same = rest
                        .iter()
                        .all(|other| reg_config_value(self, other) == Some(c));
                    return logical(same);
                }
                "isvalid" | "delete" => {
                    return Err(self.raise(
                        self.obj_undefined(name, Some(this)),
                        "MATLAB:UndefinedFunction".into(),
                    ));
                }
                _ => {}
            }
        }
        if let ObjState::Fit(f) = &self.objects.entries[&id].state {
            let f = f.clone();
            if let Some(value) = super::curve_fit::object_method(self, &f, name, rest, out)? {
                return Ok(Some(value));
            }
        }
        if let ObjState::Regression(model) = &self.objects.entries[&id].state {
            if name == "predict" {
                let mut args = vec![this.clone()];
                args.extend_from_slice(rest);
                return super::linear_models::model_predict(self, &args, 1)
                    .map(|mut v| Some(v.remove(0)));
            }
            if name == "isa" {
                let [kind] = rest else {
                    return Err("isa: expected one class name".into());
                };
                let class = model.class();
                let kind = self.text_value(kind.clone())?;
                return Ok(Some(Value::Logical(LogicalMatrix::scalar(kind == class))));
            }
            if matches!(name, "properties" | "fieldnames") {
                let names = model.property_names();
                return Ok(Some(self.obj_names(&names)?.into()));
            }
            if !matches!(
                name,
                "class"
                    | "isobject"
                    | "isstruct"
                    | "iscell"
                    | "isnumeric"
                    | "ischar"
                    | "islogical"
                    | "isfloat"
                    | "isequal"
                    | "disp"
                    | "display"
                    | "length"
                    | "size"
                    | "isempty"
                    | "numel"
                    | "ndims"
            ) {
                return Err(format!(
                    "{name}: unsupported operation on {}",
                    model.class()
                ));
            }
        }
        if matches!(name, "predict" | "kfoldPredict")
            && !matches!(
                self.objects.entries[&id].state,
                ObjState::Classifier(_) | ObjState::Mlr(_)
            )
        {
            return Err(format!(
                "{name}: unsupported model type {}",
                self.obj_class_name(id)
            ));
        }
        if let ObjState::Distribution(d) = &self.objects.entries[&id].state {
            let d = d.clone();
            if name == "isa" {
                let [kind] = rest else {
                    return Err("isa: expected one class name".into());
                };
                let kind = self.text_value(kind.clone())?;
                return Ok(Some(Value::Logical(LogicalMatrix::scalar(
                    kind == d.class(),
                ))));
            }
            if matches!(name, "properties" | "fieldnames") {
                let names = d.property_names();
                return Ok(Some(self.obj_names(&names)?.into()));
            }
            if let Some(v) = super::dist_objects::method(self, &d, name, rest, out)? {
                return Ok(Some(v));
            }
            if matches!(name, "eq" | "ne") {
                let [other] = rest else {
                    return Err("Distribution comparison requires two values".into());
                };
                let same = self.obj_equal(id, other)?;
                return Ok(Some(Value::Logical(LogicalMatrix::scalar(
                    same == (name == "eq"),
                ))));
            }
            if !matches!(
                name,
                "class"
                    | "isobject"
                    | "isstruct"
                    | "iscell"
                    | "isnumeric"
                    | "ischar"
                    | "islogical"
                    | "isfloat"
                    | "isequal"
                    | "disp"
                    | "display"
                    | "length"
                    | "size"
                    | "isempty"
                    | "numel"
                    | "ndims"
            ) {
                return Err(format!("{name}: unsupported operation on {}", d.class()));
            }
        }
        if let ObjState::Filter(f) = &self.objects.entries[&id].state {
            let f = f.clone();
            if super::digital_filter::FILTER_FUNCTIONS.contains(&name) {
                let (callee, args) = super::digital_filter::route(&f, name, rest)?;
                let handle = crate::FunctionHandle::named(callee);
                let mut v = self.call_handle(&handle, args, 1, None, out)?;
                return Ok(Some(if v.is_empty() {
                    Value::Void
                } else {
                    v.remove(0)
                }));
            }
            match name {
                "isfir" => {
                    if !rest.is_empty() {
                        return Err("isfir: too many input arguments".into());
                    }
                    return Ok(Some(Value::Logical(LogicalMatrix::scalar(
                        super::digital_filter::is_fir(&f),
                    ))));
                }
                "tf" if rest.is_empty() => {
                    return Ok(Some(Value::Matrix(crate::Matrix::from_column_vec(
                        1,
                        f.b.len(),
                        f.b.clone(),
                    )?)));
                }
                "filtord" | "order" if rest.is_empty() => {
                    return Ok(Some(Value::Matrix(crate::Matrix::scalar(f.order as f64))));
                }
                "isa" => {
                    let [kind] = rest else {
                        return Err("isa: expected one class name".into());
                    };
                    let kind = self.text_value(kind.clone())?;
                    return Ok(Some(Value::Logical(LogicalMatrix::scalar(
                        kind == "digitalFilter",
                    ))));
                }
                "properties" | "fieldnames" => {
                    return Ok(Some(self.obj_names(&f.property_names())?.into()));
                }
                "class" | "isobject" | "isstruct" | "iscell" | "isnumeric" | "ischar"
                | "islogical" | "isfloat" | "isequal" | "disp" | "display" | "length" | "size"
                | "isempty" | "numel" | "ndims" => {}
                _ => return Err(format!("{name}: unsupported operation on digitalFilter")),
            }
        }
        if let ObjState::MlPartition(p) = &self.objects.entries[&id].state {
            if matches!(name, "training" | "test" | "repartition") {
                let p = p.clone();
                return super::stats_models::partition::operation(self, name, &p, rest).map(Some);
            }
            if matches!(name, "properties" | "fieldnames") {
                let names = [
                    "NumObservations",
                    "NumTestSets",
                    "TrainSize",
                    "TestSize",
                    "Type",
                ]
                .map(String::from);
                return Ok(Some(self.obj_names(&names)?.into()));
            }
            if !matches!(
                name,
                "class"
                    | "isa"
                    | "isobject"
                    | "isstruct"
                    | "iscell"
                    | "isnumeric"
                    | "ischar"
                    | "islogical"
                    | "isfloat"
                    | "isequal"
                    | "disp"
                    | "display"
                    | "length"
                    | "size"
                    | "isempty"
                    | "numel"
                    | "ndims"
            ) {
                return Err(format!("{name}: unsupported operation on cvpartition"));
            }
        }
        if let ObjState::Mlr(m) = &self.objects.entries[&id].state {
            let m = m.clone();
            if let Some(value) = super::stats_models::ml_regression::method(self, &m, name, rest)? {
                if name == "view"
                    && let Value::Text(text) = value
                {
                    self.emit(out, text)?;
                    return Ok(Some(Value::Void));
                }
                return Ok(Some(value));
            }
            if !matches!(
                name,
                "class"
                    | "isa"
                    | "isobject"
                    | "isstruct"
                    | "iscell"
                    | "isnumeric"
                    | "ischar"
                    | "islogical"
                    | "isfloat"
                    | "isequal"
                    | "disp"
                    | "display"
                    | "length"
                    | "size"
                    | "isempty"
                    | "numel"
                    | "ndims"
            ) {
                return Err(format!("{name}: unsupported operation on {}", m.class()));
            }
        }
        if let ObjState::Classifier(c) = &self.objects.entries[&id].state {
            let c = c.clone();
            if name == "crossval" {
                return super::classifier_objects::crossval(self, &c, rest).map(Some);
            }
            if name == "predict" {
                return super::classifier_objects::classifier_predict(self, &c, rest)
                    .map(|mut v| Some(v.remove(0)));
            }
            if matches!(
                name,
                "loss" | "margin" | "edge" | "oobLoss" | "predictorImportance"
            ) {
                return super::stats_models::metrics::call(self, &c, name, rest).map(Some);
            }
            if name == "resubLoss" {
                return super::classifier_objects::resub_loss(self, &c, rest).map(Some);
            }
            if name == "kfoldPredict" {
                return super::classifier_objects::kfold_predict(self, &c, rest)
                    .map(|mut v| Some(v.remove(0)));
            }
            if name == "kfoldLoss" {
                return super::classifier_objects::kfold_loss(self, &c, rest).map(Some);
            }
            if matches!(name, "properties" | "fieldnames") {
                return super::classifier_objects::property_names(self, &c).map(Some);
            }
            if !matches!(
                name,
                "class"
                    | "isa"
                    | "isobject"
                    | "isstruct"
                    | "iscell"
                    | "isnumeric"
                    | "ischar"
                    | "islogical"
                    | "isfloat"
                    | "isequal"
                    | "disp"
                    | "display"
                    | "length"
                    | "size"
                    | "isempty"
                    | "numel"
                    | "ndims"
            ) {
                return Err(format!("{name}: unsupported operation on {}", c.class()));
            }
        }
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
        if self.obj_is_interpolant(id) {
            return self.obj_interpolant_method(this, id, name, rest, out);
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
        if let (ObjState::RegConfig(a), ObjState::RegConfig(b)) = (
            &self.objects.entries[&id].state,
            &self.objects.entries[&other].state,
        ) {
            return Ok(a == b);
        }
        if let (ObjState::Mpc(a), ObjState::Mpc(b)) = (
            &self.objects.entries[&id].state,
            &self.objects.entries[&other].state,
        ) {
            let (a, b) = (a.clone(), b.clone());
            return super::mpc::equal(&a, &b, |a, b| self.equal_arrays(a, b));
        }
        if let (ObjState::Fis(a), ObjState::Fis(b)) = (
            &self.objects.entries[&id].state,
            &self.objects.entries[&other].state,
        ) {
            let (a, b) = (a.clone(), b.clone());
            return super::fuzzy::equal(&a, &b, |a, b| self.equal_arrays(a, b));
        }
        if let (ObjState::Network(a), ObjState::Network(b)) = (
            &self.objects.entries[&id].state,
            &self.objects.entries[&other].state,
        ) {
            let a = super::nnet::equality_state(a, &self.cancel)?;
            let b = super::nnet::equality_state(b, &self.cancel)?;
            return self.equal_arrays(&a, &b);
        }
        if other == id {
            return Ok(true);
        }
        if let (ObjState::Geo(a), ObjState::Geo(b)) = (
            &self.objects.entries[&id].state,
            &self.objects.entries[&other].state,
        ) {
            return Ok(a == b);
        }
        if let (ObjState::Camera(a), ObjState::Camera(b)) = (
            &self.objects.entries[&id].state,
            &self.objects.entries[&other].state,
        ) {
            return Ok(a == b);
        }
        if let (ObjState::PdeGeometry(a), ObjState::PdeGeometry(b)) = (
            &self.objects.entries[&id].state,
            &self.objects.entries[&other].state,
        ) {
            return Ok(a.geom == b.geom);
        }
        if let (ObjState::PdeMesh(a), ObjState::PdeMesh(b)) = (
            &self.objects.entries[&id].state,
            &self.objects.entries[&other].state,
        ) {
            return Ok(a == b);
        }
        if let (ObjState::PdeStationaryResults(a), ObjState::PdeStationaryResults(b)) = (
            &self.objects.entries[&id].state,
            &self.objects.entries[&other].state,
        ) {
            return Ok(a == b);
        }
        if let (ObjState::PdeEigenResults(a), ObjState::PdeEigenResults(b)) = (
            &self.objects.entries[&id].state,
            &self.objects.entries[&other].state,
        ) {
            return Ok(a == b);
        }
        if let (ObjState::PdeTimeDependentResults(a), ObjState::PdeTimeDependentResults(b)) = (
            &self.objects.entries[&id].state,
            &self.objects.entries[&other].state,
        ) {
            return Ok(a == b);
        }
        if let (ObjState::Distribution(a), ObjState::Distribution(b)) = (
            &self.objects.entries[&id].state,
            &self.objects.entries[&other].state,
        ) {
            return Ok(a == b);
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
            ObjState::EconVol(_)
            | ObjState::Network(_)
            | ObjState::Arima(_)
            | ObjState::Mpc(_)
            | ObjState::MpcState(_) => 1.,
            ObjState::Fis(o) => o.len() as f64,
            #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
            ObjState::Audio(_) => 1.,
            ObjState::VideoWriter(_)
            | ObjState::Video(_)
            | ObjState::Parser(_)
            | ObjState::Interpolant(_)
            | ObjState::Distribution(_)
            | ObjState::Regression(_)
            | ObjState::Parallel(_)
            | ObjState::Classifier(_)
            | ObjState::MlPartition(_)
            | ObjState::Mlr(_)
            | ObjState::Filter(_)
            | ObjState::Import(_)
            | ObjState::Fit(_)
            | ObjState::RegConfig(_)
            | ObjState::Geo(_)
            | ObjState::Triangulation(_)
            | ObjState::Camera(_)
            | ObjState::PdeModel(_)
            | ObjState::PdeGeometry(_)
            | ObjState::PdeMesh(_)
            | ObjState::PdeStationaryResults(_)
            | ObjState::PdeEigenResults(_)
            | ObjState::PdeTimeDependentResults(_) => 1.,
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

    /// Keys as text (numeric keys in shortest round-trip form) and values of a
    /// live containers.Map, in key order; None when `v` is not a map. Used by
    /// jsonencode (src/engine/readers.rs).
    pub(super) fn obj_map_text_entries(&self, v: &Value) -> Option<Result<ObjMapEntries>> {
        let id = self.obj_value_id(v)?;
        if !self.obj_is_map(id) {
            return None;
        }
        if self.objects.entries[&id].deleted {
            return Some(Err(MAP_DELETED.into()));
        }
        let entries = self
            .obj_map(id)
            .entries
            .iter()
            .map(|(key, (_, value))| {
                let text = match key {
                    MapKey::Text(units) => String::from_utf16_lossy(units),
                    MapKey::Number(n) => format!("{n}"),
                    MapKey::Integer(i) => i.to_string(),
                };
                (text, value.clone())
            })
            .collect();
        Some(Ok(entries))
    }

    fn obj_map(&self, id: u64) -> &MapState {
        match &self.objects.entries[&id].state {
            ObjState::Map(m) => m,
            _ => unreachable!("map state"),
        }
    }
    fn obj_map_mut(&mut self, id: u64) -> &mut MapState {
        match &mut self.objects.entries.get_mut(&id).unwrap().state {
            ObjState::Map(m) => m,
            _ => unreachable!("map state"),
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
        let (rows, cols) = match &self.objects.entries[&id].state {
            ObjState::Triangulation(dt) => (dt.connectivity.rows() as f64, 3.),
            _ => (self.obj_size(id), 1.),
        };
        match rest {
            [] => Ok(Value::Matrix(Matrix::from_column_vec(
                1,
                2,
                vec![rows, cols],
            )?)),
            [dim] => {
                let d = obj_scalar_real(dim)
                    .filter(|d| *d >= 1. && d.fract() == 0.)
                    .ok_or("Dimension argument must be a positive integer scalar within indexing range.")?;
                Ok(Value::Matrix(Matrix::scalar(match d {
                    1. => rows,
                    2. => cols,
                    _ => 1.,
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
        if let ObjState::Fis(o) = &self.objects.entries[&id].state {
            let o = o.clone();
            return super::fuzzy::properties::index(self, &o, args, out);
        }
        if let ObjState::Triangulation(dt) = &self.objects.entries[&id].state {
            let connectivity = dt.connectivity.clone();
            return self.index_value(Value::Matrix(connectivity), args, out);
        }
        if let ObjState::Network(n) = &self.objects.entries[&id].state {
            let n = n.clone();
            let values = self.function_arguments(args, None, out)?;
            if n.dynamic.is_some() {
                return Ok(super::nnet::dynamic::simulate(self, &n, &values)?.remove(0));
            }
            if values.len() != 1 {
                return Err("network: expected one input array".into());
            }
            return super::nnet::simulate(self, &n, &values[0]);
        }
        if self.obj_is_interpolant(id) {
            let values = self.function_arguments(args, None, out)?;
            return self.obj_interpolant_value(id, &values);
        }
        if let ObjState::Fit(f) = &self.objects.entries[&id].state {
            let f = f.clone();
            let values = self.function_arguments(args, None, out)?;
            return super::curve_fit::subsref(self, &f, &values, out);
        }
        if !self.obj_is_map(id) {
            return Err(format!(
                "Array indexing of a {} is not supported",
                self.obj_class_name(id)
            ));
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
            return Err(format!(
                "Indexed assignment into a {} is not supported",
                self.obj_class_name(id)
            ));
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

    // ----- interpolation handle objects -----

    fn obj_real_vector(a: &Array, who: &str) -> Result<Vec<f64>> {
        match a {
            Array::Numeric(m) if m.rows() == 1 || m.cols() == 1 => {
                if m.len() < 2 || m.data().iter().any(|v| !v.is_finite()) {
                    Err(format!(
                        "{who}: grid vectors must contain at least two finite values"
                    ))
                } else {
                    Ok(m.data().to_vec())
                }
            }
            _ => Err(format!("{who}: grid vectors must be real double vectors")),
        }
    }
    fn obj_method_name(a: &Array, who: &str, allowed: &[&str]) -> Result<String> {
        let Some(name) = obj_text(a) else {
            return Err(format!("{who}: method must be character text"));
        };
        let name = name.to_ascii_lowercase();
        if allowed.contains(&name.as_str()) {
            Ok(name)
        } else {
            Err(format!("{who}: unsupported interpolation method '{name}'"))
        }
    }
    fn obj_gridded_new(&mut self, args: &[Value]) -> Result<Value> {
        if !(2..=5).contains(&args.len()) {
            return Err("griddedInterpolant: expected grid vectors, values, and optional method/extrapolation".into());
        }
        let arrays = args
            .iter()
            .map(|v| v.clone().array(&self.cancel))
            .collect::<Result<Vec<_>>>()?;
        let mut first_text = arrays.len();
        for (i, a) in arrays.iter().enumerate() {
            if obj_text(a).is_some() {
                first_text = i;
                break;
            }
        }
        let (grid_args, options) = arrays.split_at(first_text);
        if grid_args.len() < 2 {
            return Err("griddedInterpolant: values are required".into());
        }
        let values = Arc::new(grid_args.last().unwrap().clone());
        let grids: Vec<Arc<Array>> = if grid_args.len() == 2 {
            if let Array::Cell(c) = &grid_args[0] {
                if c.len() != 2 {
                    return Err(
                        "griddedInterpolant: GridVectors cell must contain one or two vectors"
                            .into(),
                    );
                }
                (0..c.len())
                    .map(|i| Ok(c.get_linear(i)?.clone()))
                    .collect::<Result<_>>()?
            } else {
                vec![Arc::new(grid_args[0].clone())]
            }
        } else if grid_args.len() == 3 {
            vec![
                Arc::new(grid_args[0].clone()),
                Arc::new(grid_args[1].clone()),
            ]
        } else {
            return Err("griddedInterpolant supports one or two dimensions".into());
        };
        if grids.len() == 1 {
            let x = Self::obj_real_vector(&grids[0], GRIDDED)?;
            let Array::Numeric(v) = values.as_ref() else {
                return Err("griddedInterpolant: Values must be a real double array".into());
            };
            if v.len() != x.len() {
                return Err(
                    "griddedInterpolant: grid vector and Values must have matching length".into(),
                );
            }
        } else {
            let x = Self::obj_real_vector(&grids[0], GRIDDED)?;
            let y = Self::obj_real_vector(&grids[1], GRIDDED)?;
            let Array::Numeric(v) = values.as_ref() else {
                return Err("griddedInterpolant: Values must be a real double array".into());
            };
            if v.rows() != x.len() || v.cols() != y.len() {
                return Err("griddedInterpolant: Values must have ndgrid dimensions length(x1)-by-length(x2)".into());
            }
        }
        if options.len() > 2 {
            return Err("Too many input arguments.".into());
        }
        let method = options
            .first()
            .map(|a| {
                Self::obj_method_name(
                    a,
                    GRIDDED,
                    &["linear", "nearest", "pchip", "spline", "makima"],
                )
            })
            .transpose()?
            .unwrap_or_else(|| "linear".into());
        if grids.len() == 2 && !matches!(method.as_str(), "linear" | "nearest") {
            return Err(
                "griddedInterpolant: 2-D supports only 'linear' and 'nearest' methods".into(),
            );
        }
        let extrapolation = options
            .get(1)
            .map(|a| Self::obj_method_name(a, GRIDDED, &["linear", "nearest", "none"]))
            .transpose()?
            .unwrap_or_else(|| method.clone());
        self.obj_new(ObjState::Interpolant(InterpolantState::Gridded(
            GriddedState {
                grids,
                values,
                method,
                extrapolation,
            },
        )))
    }
    fn obj_scattered_new(&mut self, args: &[Value]) -> Result<Value> {
        if !(3..=5).contains(&args.len()) {
            return Err(
                "scatteredInterpolant: expected x, y, values, and optional method/extrapolation"
                    .into(),
            );
        }
        let arrays = args
            .iter()
            .map(|v| v.clone().array(&self.cancel))
            .collect::<Result<Vec<_>>>()?;
        let x = Self::obj_real_vector(&arrays[0], SCATTERED)?;
        let y = Self::obj_real_vector(&arrays[1], SCATTERED)?;
        let values = Arc::new(arrays[2].clone());
        let Array::Numeric(v) = values.as_ref() else {
            return Err("scatteredInterpolant: Values must be a real double vector".into());
        };
        if x.len() != y.len() || x.len() != v.len() || x.len() < 3 {
            return Err("scatteredInterpolant: x, y, and values must be equal-length vectors with at least three points".into());
        }
        if x.len() > MAX_SCATTERED_POINTS {
            return Err(format!(
                "scatteredInterpolant supports up to {} points",
                MAX_SCATTERED_POINTS
            ));
        }
        if arrays.len() > 5 {
            return Err("Too many input arguments.".into());
        }
        let method = arrays
            .get(3)
            .map(|a| Self::obj_method_name(a, SCATTERED, &["linear", "nearest", "natural"]))
            .transpose()?
            .unwrap_or_else(|| "linear".into());
        if method == "natural" {
            return Err("scatteredInterpolant: the 'natural' method is not supported yet".into());
        }
        let extrapolation = arrays
            .get(4)
            .map(|a| Self::obj_method_name(a, SCATTERED, &["linear", "nearest", "none"]))
            .transpose()?
            .unwrap_or_else(|| "linear".into());
        self.obj_new(ObjState::Interpolant(InterpolantState::Scattered(
            ScatteredState {
                points: x.into_iter().zip(y).collect(),
                values,
                method,
                extrapolation,
            },
        )))
    }
    fn obj_interpolant_method(
        &mut self,
        _this: &Value,
        id: u64,
        name: &str,
        rest: &[Value],
        _out: &mut Execution,
    ) -> Result<Option<Value>> {
        if self.objects.entries[&id].deleted {
            return Err("Invalid or deleted interpolant object.".into());
        }
        let class = self.obj_class_name(id);
        match name {
            "properties" | "fieldnames" => {
                let names: Vec<String> = match &self.objects.entries[&id].state {
                    ObjState::Interpolant(InterpolantState::Gridded(_)) => {
                        ["GridVectors", "Values", "Method", "ExtrapolationMethod"]
                            .map(String::from)
                            .to_vec()
                    }
                    ObjState::Interpolant(InterpolantState::Scattered(_)) => {
                        ["Points", "Values", "Method", "ExtrapolationMethod"]
                            .map(String::from)
                            .to_vec()
                    }
                    _ => unreachable!(),
                };
                return self.obj_names(&names).map(|a| Some(a.into()));
            }
            "numel" => return Ok(Some(Value::Matrix(Matrix::scalar(1.)))),
            "ndims" => return Ok(Some(Value::Matrix(Matrix::scalar(2.)))),
            "size" => return self.obj_size_value(id, &[]).map(Some),
            "length" => return Ok(Some(Value::Matrix(Matrix::scalar(1.)))),
            "isempty" => return Ok(Some(Value::Logical(LogicalMatrix::scalar(false)))),
            _ => {}
        }
        if !rest.is_empty() {
            return Err(format!(
                "No method '{name}' with matching signature found for class '{class}'."
            ));
        }
        Ok(None)
    }
    fn obj_interpolant_value(&mut self, id: u64, args: &[Value]) -> Result<Value> {
        let state = match &self.objects.entries[&id].state {
            ObjState::Interpolant(s) => s.clone(),
            _ => unreachable!(),
        };
        match state {
            InterpolantState::Gridded(g) => self.obj_gridded_eval(&g, args),
            InterpolantState::Scattered(s) => self.obj_scattered_eval(&s, args),
        }
    }
    fn obj_gridded_eval(&mut self, g: &GriddedState, args: &[Value]) -> Result<Value> {
        let cell_grid_query =
            args.len() == 1 && matches!(args[0].clone().array(&self.cancel)?, Array::Cell(_));
        let mut queries: Vec<Value> = if args.len() == 1 {
            match args[0].clone().array(&self.cancel)? {
                Array::Cell(c) => (0..c.len())
                    .map(|i| Ok(Value::from(c.get_linear(i)?.as_ref().clone())))
                    .collect::<Result<_>>()?,
                _ => args.to_vec(),
            }
        } else {
            args.to_vec()
        };
        if queries.len() != g.grids.len() {
            return Err(format!(
                "griddedInterpolant: expected {} query arguments",
                g.grids.len()
            ));
        }
        // A GridVectors cell query is an ndgrid query, even when both input
        // vectors are row vectors. interp2 recognizes this as a row/column
        // pair, so normalize its first (dimension-one) query to a column.
        if cell_grid_query && g.grids.len() == 2 {
            if let Value::Matrix(q) = &queries[0]
                && (q.rows() == 1 || q.cols() == 1)
            {
                queries[0] = Value::Matrix(Matrix::from_column_vec(q.len(), 1, q.data().to_vec())?);
            }
            if let Value::Matrix(q) = &queries[1]
                && (q.rows() == 1 || q.cols() == 1)
            {
                queries[1] = Value::Matrix(Matrix::from_column_vec(1, q.len(), q.data().to_vec())?);
            }
        }
        if g.grids.len() == 1 {
            let interpolate = |engine: &Self, method: &str, extrapolate: bool| {
                let mut call = vec![
                    Value::from(g.grids[0].as_ref().clone()),
                    Value::from(g.values.as_ref().clone()),
                    queries[0].clone(),
                    Value::Text(method.into()),
                ];
                if extrapolate {
                    call.push(Value::Text("extrap".into()));
                } else {
                    call.push(Value::Matrix(Matrix::scalar(f64::NAN)));
                }
                engine.polyopt_interp1(&call)
            };
            if g.extrapolation == "none" {
                return interpolate(self, &g.method, false);
            }
            if g.extrapolation == g.method {
                return interpolate(self, &g.method, true);
            }
            let Value::Matrix(inside) = interpolate(self, &g.method, false)? else {
                unreachable!()
            };
            let Value::Matrix(outside) = interpolate(self, &g.extrapolation, true)? else {
                unreachable!()
            };
            let Value::Matrix(q) = &queries[0] else {
                return Err("griddedInterpolant: query points must be real double arrays".into());
            };
            let grid = Self::obj_real_vector(&g.grids[0], GRIDDED)?;
            let lo = grid.iter().copied().fold(f64::INFINITY, f64::min);
            let hi = grid.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let mut result = inside.data().to_vec();
            for (i, &x) in q.data().iter().enumerate() {
                if x < lo || x > hi {
                    result[i] = outside.data()[i];
                }
            }
            return Ok(Value::Matrix(Matrix::from_column_vec(
                inside.rows(),
                inside.cols(),
                result,
            )?));
        }
        let interpolate = |engine: &Self, method: &str, extrapolate: bool| {
            let mut call = vec![
                Value::from(g.grids[1].as_ref().clone()),
                Value::from(g.grids[0].as_ref().clone()),
                Value::from(g.values.as_ref().clone()),
                queries[1].clone(),
                queries[0].clone(),
                Value::Text(method.into()),
            ];
            if extrapolate {
                call.push(Value::Text("extrap".into()));
            } else {
                call.push(Value::Matrix(Matrix::scalar(f64::NAN)));
            }
            engine.polyopt_interp2(&call)
        };
        if g.extrapolation == "none" {
            return interpolate(self, &g.method, false);
        }
        if g.extrapolation == g.method {
            return interpolate(self, &g.method, true);
        }
        let Value::Matrix(inside) = interpolate(self, &g.method, false)? else {
            unreachable!()
        };
        let Value::Matrix(outside) = interpolate(self, &g.extrapolation, true)? else {
            unreachable!()
        };
        let (Value::Matrix(q0), Value::Matrix(q1)) = (&queries[0], &queries[1]) else {
            return Err("griddedInterpolant: query points must be real double arrays".into());
        };
        let x = Self::obj_real_vector(&g.grids[0], GRIDDED)?;
        let y = Self::obj_real_vector(&g.grids[1], GRIDDED)?;
        let (xlo, xhi) = (
            x.iter().copied().fold(f64::INFINITY, f64::min),
            x.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        );
        let (ylo, yhi) = (
            y.iter().copied().fold(f64::INFINITY, f64::min),
            y.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        );
        let grid = (q1.rows() == 1
            && q0.cols() == 1
            && q1.cols() > 0
            && q0.rows() > 0
            && q1.len() * q0.len() != 1)
            || (q1.cols() == 1
                && q0.rows() == 1
                && q1.len() * q0.len() != 1
                && !q1.is_empty()
                && !q0.is_empty());
        let mut result = inside.data().to_vec();
        for (i, value) in result.iter_mut().enumerate() {
            let (qx, qy) = if grid {
                (q1.data()[i / inside.rows()], q0.data()[i % inside.rows()])
            } else {
                (q1.data()[i], q0.data()[i])
            };
            if qx < ylo || qx > yhi || qy < xlo || qy > xhi {
                *value = outside.data()[i];
            }
        }
        Ok(Value::Matrix(Matrix::from_column_vec(
            inside.rows(),
            inside.cols(),
            result,
        )?))
    }
    fn obj_scattered_eval(&self, s: &ScatteredState, args: &[Value]) -> Result<Value> {
        let [xq, yq] = args else {
            return Err("scatteredInterpolant: expected xq and yq".into());
        };
        let (Value::Matrix(xq), Value::Matrix(yq)) = (xq, yq) else {
            return Err("scatteredInterpolant: query points must be real double arrays".into());
        };
        if xq.rows() != yq.rows() || xq.cols() != yq.cols() {
            return Err("scatteredInterpolant: xq and yq must have the same size".into());
        }
        let Array::Numeric(values) = s.values.as_ref() else {
            unreachable!()
        };
        let points: Vec<[f64; 2]> = s.points.iter().map(|&(x, y)| [x, y]).collect();
        let Some((delaunay, _)) = self.sc2_triangulate(&points)? else {
            return Err("scatteredInterpolant: data points must not be collinear".into());
        };
        let mut out = Vec::with_capacity(xq.len());
        for i in 0..xq.len() {
            let q = [xq.data()[i], yq.data()[i]];
            let value = if !q[0].is_finite() || !q[1].is_finite() {
                f64::NAN
            } else {
                let triangle = delaunay.locate(q);
                if !delaunay.is_ghost(triangle) {
                    let [a, b, c] = delaunay.tris[triangle].v;
                    if s.method == "nearest" {
                        let k = points
                            .iter()
                            .enumerate()
                            .min_by(|(_, a), (_, b)| {
                                let da = (a[0] - q[0]).powi(2) + (a[1] - q[1]).powi(2);
                                let db = (b[0] - q[0]).powi(2) + (b[1] - q[1]).powi(2);
                                da.total_cmp(&db)
                            })
                            .map(|(k, _)| k)
                            .expect("at least three points");
                        values.data()[k]
                    } else {
                        let w = obj_barycentric(q, points[a], points[b], points[c]);
                        w[0] * values.data()[a] + w[1] * values.data()[b] + w[2] * values.data()[c]
                    }
                } else {
                    match s.extrapolation.as_str() {
                        "linear" => {
                            let distance_to_segment = |a: [f64; 2], b: [f64; 2]| {
                                let dx = b[0] - a[0];
                                let dy = b[1] - a[1];
                                let length2 = dx * dx + dy * dy;
                                let u = if length2 == 0. {
                                    0.
                                } else {
                                    ((q[0] - a[0]) * dx + (q[1] - a[1]) * dy) / length2
                                }
                                .clamp(0., 1.);
                                let ex = q[0] - (a[0] + u * dx);
                                let ey = q[1] - (a[1] + u * dy);
                                ex * ex + ey * ey
                            };
                            let edge = (0..delaunay.tris.len())
                                .filter(|&i| delaunay.alive[i] && delaunay.is_ghost(i))
                                .min_by(|&i, &j| {
                                    let [ai, bi, _] = delaunay.tris[i].v;
                                    let [aj, bj, _] = delaunay.tris[j].v;
                                    distance_to_segment(points[ai], points[bi])
                                        .total_cmp(&distance_to_segment(points[aj], points[bj]))
                                });
                            let Some(edge) = edge else {
                                return Err("scatteredInterpolant: no hull edge found".into());
                            };
                            let real = delaunay.tris[edge].n[2];
                            let [a, b, c] = delaunay.tris[real].v;
                            let w = obj_barycentric(q, points[a], points[b], points[c]);
                            w[0] * values.data()[a]
                                + w[1] * values.data()[b]
                                + w[2] * values.data()[c]
                        }
                        "nearest" => points
                            .iter()
                            .enumerate()
                            .min_by(|(_, a), (_, b)| {
                                let da = (a[0] - q[0]).powi(2) + (a[1] - q[1]).powi(2);
                                let db = (b[0] - q[0]).powi(2) + (b[1] - q[1]).powi(2);
                                da.total_cmp(&db)
                            })
                            .map(|(k, _)| values.data()[k])
                            .expect("at least three points"),
                        "none" => f64::NAN,
                        _ => unreachable!(),
                    }
                }
            };
            out.push(value);
        }
        Ok(Value::Matrix(Matrix::from_column_vec(
            xq.rows(),
            xq.cols(),
            out,
        )?))
    }
    fn obj_set_interpolant_property(&mut self, id: u64, name: &str, value: Array) -> Result<()> {
        match &mut self.objects.entries.get_mut(&id).unwrap().state {
            ObjState::Interpolant(InterpolantState::Gridded(g)) => match name {
                "Values" => {
                    let Array::Numeric(v) = &value else {
                        return Err("griddedInterpolant: Values must be a real double array".into());
                    };
                    let dims: Vec<usize> = g
                        .grids
                        .iter()
                        .map(|a| Self::obj_real_vector(a, GRIDDED).map(|x| x.len()))
                        .collect::<Result<_>>()?;
                    if (dims.len() == 1 && v.len() != dims[0])
                        || (dims.len() == 2 && (v.rows() != dims[0] || v.cols() != dims[1]))
                    {
                        return Err("griddedInterpolant: Values has incompatible dimensions".into());
                    }
                    g.values = Arc::new(value);
                }
                "Method" => {
                    g.method = Self::obj_method_name(
                        &value,
                        GRIDDED,
                        &["linear", "nearest", "pchip", "spline", "makima"],
                    )?;
                    if g.grids.len() == 2 && !matches!(g.method.as_str(), "linear" | "nearest") {
                        return Err(
                            "griddedInterpolant: 2-D supports only 'linear' and 'nearest' methods"
                                .into(),
                        );
                    }
                }
                "ExtrapolationMethod" => {
                    g.extrapolation =
                        Self::obj_method_name(&value, GRIDDED, &["linear", "nearest", "none"])?
                }
                "GridVectors" => return Err("griddedInterpolant: GridVectors is read-only".into()),
                _ => {
                    return Err(format!(
                        "No public property {name} exists for class {GRIDDED}."
                    ));
                }
            },
            ObjState::Interpolant(InterpolantState::Scattered(s)) => match name {
                "Values" => {
                    let Array::Numeric(v) = &value else {
                        return Err(
                            "scatteredInterpolant: Values must be a real double vector".into()
                        );
                    };
                    if v.len() != s.points.len() {
                        return Err(
                            "scatteredInterpolant: Values must have one value per point".into()
                        );
                    }
                    s.values = Arc::new(value);
                }
                "Method" => {
                    s.method = Self::obj_method_name(&value, SCATTERED, &["linear", "nearest"])?;
                }
                "ExtrapolationMethod" => {
                    s.extrapolation =
                        Self::obj_method_name(&value, SCATTERED, &["linear", "nearest", "none"])?
                }
                "Points" => return Err("scatteredInterpolant: Points is read-only".into()),
                _ => {
                    return Err(format!(
                        "No public property {name} exists for class {SCATTERED}."
                    ));
                }
            },
            _ => unreachable!(),
        }
        Ok(())
    }

    // ----- inputParser -----

    fn obj_parser(&self, id: u64) -> &ParserState {
        match &self.objects.entries[&id].state {
            ObjState::Parser(p) => p,
            _ => unreachable!("parser state"),
        }
    }
    fn obj_parser_mut(&mut self, id: u64) -> &mut ParserState {
        match &mut self.objects.entries.get_mut(&id).unwrap().state {
            ObjState::Parser(p) => p,
            _ => unreachable!("parser state"),
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
                // One call. Explicit false (or no validator) still expands.
                // A thrown error is reported; cancellation propagates.
                // Neither is sent on into field expansion. No return value accepts.
                let arg_name = p.args[i].name.clone();
                let expand = match &p.args[i].validator {
                    None => true,
                    Some(validator) => {
                        match self.call_handle(
                            validator,
                            vec![Value::from(value.clone())],
                            1,
                            None,
                            out,
                        ) {
                            Ok(values) => matches!(values.first(), Some(v) if !obj_truthy(v)),
                            Err(error) => {
                                if !self.is_catchable(&error) {
                                    return Err(error);
                                }
                                let (message, _) = self.failure_text(&error);
                                self.raised = None;
                                self.functions.lng.thrown_stack = None;
                                self.functions.lng.thrown_object = None;
                                return Err(self.obj_parse_fail(
                                    header,
                                    format!("The value of '{arg_name}' is invalid. {message}"),
                                    "ArgumentFailedValidation",
                                ));
                            }
                        }
                    }
                };
                if expand {
                    break;
                }
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
            ObjState::Triangulation(dt) => dt.property(name),
            ObjState::Video(v) => v.property(name),
            ObjState::VideoWriter(v) => v.property(name),
            #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
            ObjState::Audio(a) => a.property(name),
            ObjState::EconVol(m) => super::econ_vol::model_property(m, name),
            ObjState::Network(n) => super::nnet::property(self, n, name),
            ObjState::Fis(o) => super::fuzzy::properties::property(o, name),
            ObjState::Arima(m) => super::econ::property(self, m, name),
            ObjState::Mpc(m) => super::mpc::property(self, m, name),
            ObjState::MpcState(s) => super::mpc::state::property(s, name),
            ObjState::Distribution(d) => d.property(self, name),
            ObjState::Regression(m) => m.property(self, name),
            ObjState::Parallel(p) => p.property(self, name),
            ObjState::Classifier(c) => super::classifier_objects::property(self, c, name),
            ObjState::MlPartition(p) => p.property(name),
            ObjState::Mlr(m) => super::stats_models::ml_regression::property(self, m, name),
            ObjState::Filter(f) => f.property(self, name),
            ObjState::Import(o) => super::import_options::property_ref(self, o, name),
            ObjState::Fit(f) => f.property(name),
            ObjState::RegConfig(c) => c.property(name),
            ObjState::Geo(g) => super::registration::property(g, name),
            ObjState::Camera(c) => super::camera::property(c, name),
            ObjState::PdeModel(m) => m.property(name),
            ObjState::PdeGeometry(g) => g.property(name),
            ObjState::PdeMesh(m) => m.property(name),
            ObjState::PdeStationaryResults(r) => r.property(name),
            ObjState::PdeEigenResults(r) => r.property(name),
            ObjState::PdeTimeDependentResults(r) => r.property(name),
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
            ObjState::Interpolant(InterpolantState::Gridded(g)) => match name {
                "GridVectors" => self
                    .obj_cell(1, g.grids.len(), g.grids.clone())
                    .map(Value::from),
                "Values" => Ok(Value::from(g.values.as_ref().clone())),
                "Method" => Ok(Value::Text(g.method.clone())),
                "ExtrapolationMethod" => Ok(Value::Text(g.extrapolation.clone())),
                _ => Err(format!(
                    "No appropriate method, property, or field '{name}' for class '{GRIDDED}'."
                )),
            },
            ObjState::Interpolant(InterpolantState::Scattered(s)) => match name {
                "Points" => {
                    let mut data = Vec::with_capacity(s.points.len() * 2);
                    data.extend(s.points.iter().map(|p| p.0));
                    data.extend(s.points.iter().map(|p| p.1));
                    Ok(Value::Matrix(Matrix::from_column_vec(
                        s.points.len(),
                        2,
                        data,
                    )?))
                }
                "Values" => Ok(Value::from(s.values.as_ref().clone())),
                "Method" => Ok(Value::Text(s.method.clone())),
                "ExtrapolationMethod" => Ok(Value::Text(s.extrapolation.clone())),
                _ => Err(format!(
                    "No appropriate method, property, or field '{name}' for class '{SCATTERED}'."
                )),
            },
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
        if let ObjState::Import(o) = &self.objects.entries[&id].state {
            let o = o.clone();
            let value = super::import_options::property(self, &o, name)?;
            return match args {
                Some(args) if !args.is_empty() => self.index_value(value, args, out),
                _ => Ok(value),
            };
        }
        if let ObjState::RegConfig(c) = &self.objects.entries[&id].state {
            let value = c.property(name)?;
            return match args {
                Some(args) if !args.is_empty() => self.index_value(value, args, out),
                _ => Ok(value),
            };
        }
        if let ObjState::Fit(f) = &self.objects.entries[&id].state {
            let f = f.clone();
            return super::curve_fit::member(self, &f, name, args, out);
        }
        if let ObjState::Geo(g) = &self.objects.entries[&id].state {
            let g = g.clone();
            return super::registration::member(self, &g, name, args, out);
        }
        if let ObjState::Camera(c) = &self.objects.entries[&id].state {
            let c = c.clone();
            return super::camera::member(self, &c, name, args, out);
        }
        let methods: &[&str] = if matches!(&self.objects.entries[&id].state, ObjState::Video(_)) {
            super::video::METHODS
        } else if matches!(&self.objects.entries[&id].state, ObjState::Triangulation(_)) {
            &super::sci_more::triangulation::METHODS
        } else if matches!(&self.objects.entries[&id].state, ObjState::PdeModel(_)) {
            &[
                "get",
                "disp",
                "display",
                "delete",
                "isvalid",
                "properties",
                "fieldnames",
                "geometryFromEdges",
                "pdegplot",
                "generateMesh",
                "pdemesh",
                "pdeplot",
                "specifyCoefficients",
                "applyBoundaryCondition",
                "setInitialConditions",
                "solvepde",
                "solvepdeeig",
            ]
        } else if matches!(&self.objects.entries[&id].state, ObjState::PdeGeometry(_)) {
            &[
                "get",
                "disp",
                "display",
                "properties",
                "fieldnames",
                "pdegplot",
            ]
        } else if matches!(&self.objects.entries[&id].state, ObjState::PdeMesh(_)) {
            &[
                "get",
                "disp",
                "display",
                "properties",
                "fieldnames",
                "findNodes",
                "findElements",
                "pdemesh",
                "pdeplot",
            ]
        } else if matches!(
            &self.objects.entries[&id].state,
            ObjState::PdeStationaryResults(_)
                | ObjState::PdeEigenResults(_)
                | ObjState::PdeTimeDependentResults(_)
        ) {
            &["get", "disp", "display", "properties", "fieldnames", "isa"]
        } else if matches!(&self.objects.entries[&id].state, ObjState::VideoWriter(_)) {
            super::video::WRITER_METHODS
        } else if matches!(&self.objects.entries[&id].state, ObjState::EconVol(_)) {
            super::econ_vol::METHODS
        } else if matches!(&self.objects.entries[&id].state, ObjState::Distribution(_)) {
            super::dist_objects::METHODS
        } else if matches!(&self.objects.entries[&id].state, ObjState::Regression(_)) {
            &["predict", "disp"]
        } else if self.obj_is_interpolant(id)
            || matches!(
                &self.objects.entries[&id].state,
                ObjState::Network(_)
                    | ObjState::Fis(_)
                    | ObjState::Arima(_)
                    | ObjState::Mpc(_)
                    | ObjState::MpcState(_)
            )
        {
            &[]
        } else if self.obj_is_map(id) {
            &MAP_METHODS
        } else {
            &PARSER_METHODS
        };
        #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
        let methods = match &self.objects.entries[&id].state {
            ObjState::Audio(a) => a.methods(),
            _ => methods,
        };
        if methods.contains(&name) {
            let rest = match args {
                Some(args) => self.function_arguments(args, None, out)?,
                None => Vec::new(),
            };
            #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
            let is_audio = audio_receiver(self, &this);
            #[cfg(not(all(feature = "audio", not(target_arch = "wasm32"))))]
            let is_audio = false;
            if name != "writeVideo" && !is_audio {
                super::nd::guard(name, &rest)?;
                super::string_class::guard(name, &rest)?;
            }
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
        if matches!(&self.objects.entries[&id].state, ObjState::Triangulation(_)) {
            return Err(format!(
                "delaunayTriangulation property '{name}' is read-only"
            ));
        }
        if self.objects.entries[&id].deleted {
            return Err(if self.obj_is_map(id) {
                MAP_DELETED
            } else {
                PARSER_DELETED
            }
            .into());
        }
        #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
        if let ObjState::Audio(handle) = &self.objects.entries[&id].state {
            if steps.len() != 1 {
                return Err("Assign the complete audio property value.".into());
            }
            let mut handle = handle.clone();
            handle.assign(self, &name, Value::from(value))?;
            return audio_replace(self, id, handle);
        }
        if let ObjState::VideoWriter(writer) = &self.objects.entries[&id].state {
            if steps.len() != 1 {
                return Err("VideoWriter: assign the complete property value".into());
            }
            let writer = writer.clone();
            let result = writer.assign(&name, value);
            let entry = self.objects.entries.get_mut(&id).unwrap();
            self.objects.bytes = self.objects.bytes - entry.bytes + writer.bytes();
            entry.bytes = writer.bytes();
            return result;
        }
        if let ObjState::Video(reader) = &self.objects.entries[&id].state {
            if steps.len() != 1 {
                return Err("VideoReader: assign the complete property value.".into());
            }
            let changed = ObjState::Video(reader.assign(&name, value)?);
            let added = Self::obj_state_bytes(&changed);
            let removed = self.objects.entries[&id].bytes;
            self.obj_check_bytes(removed, added)?;
            let entry = self.objects.entries.get_mut(&id).unwrap();
            entry.state = changed;
            entry.bytes = added;
            self.objects.bytes = self.objects.bytes - removed + added;
            return Ok(());
        }
        if let ObjState::MpcState(s) = &self.objects.entries[&id].state {
            if steps.len() != 1 {
                return Err("mpcstate: indexed property assignment is not supported yet; assign a complete vector".into());
            }
            let changed =
                ObjState::MpcState(Arc::new(super::mpc::state::assign(s, &name, &value)?));
            let added = Self::obj_state_bytes(&changed);
            let removed = self.objects.entries[&id].bytes;
            self.obj_check_bytes(removed, added)?;
            let entry = self.objects.entries.get_mut(&id).unwrap();
            entry.state = changed;
            entry.bytes = added;
            self.objects.bytes = self.objects.bytes - removed + added;
            return Ok(());
        }
        if matches!(&self.objects.entries[&id].state, ObjState::Network(_)) {
            return Err("network: extract the network to a variable before editing nested container properties".into());
        }
        if matches!(&self.objects.entries[&id].state, ObjState::RegConfig(_)) {
            // Value objects change through obj_statement / obj_assign_nested only.
            return Err(format!(
                "{}: assign the changed object to a variable (obj.{name} = value).",
                self.obj_class_name(id)
            ));
        }
        if let ObjState::Import(o) = &self.objects.entries[&id].state {
            // Reached through another value (s.opts.DataLines = ...): the
            // object changes in place.
            let o = o.clone();
            let changed =
                ObjState::Import(super::import_options::assign(self, &o, steps, value, out)?);
            let added = Self::obj_state_bytes(&changed);
            let removed = self.objects.entries[&id].bytes;
            self.obj_check_bytes(removed, added)?;
            let entry = self.objects.entries.get_mut(&id).unwrap();
            entry.state = changed;
            entry.bytes = added;
            self.objects.bytes = self.objects.bytes - removed + added;
            return Ok(());
        }
        if let ObjState::Camera(c) = &self.objects.entries[&id].state {
            let c = c.clone();
            super::camera::assign(self, &c, steps, out)?;
        }
        if matches!(&self.objects.entries[&id].state, ObjState::PdeModel(_))
            && name == "SolverOptions"
        {
            if steps.len() == 2 {
                let sub = match &steps[1] {
                    PathStep::Field(f) => f.clone(),
                    PathStep::Dynamic(e) => {
                        let v = self.eval(e, None, out)?.array(&self.cancel)?;
                        obj_text(&v)
                            .ok_or("Dynamic structure field names must be character text")?
                    }
                    _ => {
                        return Err("Indexed assignment into SolverOptions is not supported".into());
                    }
                };
                if sub == "ReportStatistics" {
                    let flag = if let Some(s) = obj_text(&value) {
                        if s.eq_ignore_ascii_case("on") {
                            Some(true)
                        } else if s.eq_ignore_ascii_case("off") {
                            Some(false)
                        } else {
                            None
                        }
                    } else if let Array::Logical(l) = &value {
                        (l.len() == 1).then_some(l.data()[0] != 0)
                    } else {
                        obj_scalar_real(&value).and_then(|x| {
                            if x == 0.0 {
                                Some(false)
                            } else if x == 1.0 {
                                Some(true)
                            } else {
                                None
                            }
                        })
                    };
                    let Some(on) = flag else {
                        return Err(
                            "SolverOptions.ReportStatistics must be 'on', 'off', true, or false"
                                .into(),
                        );
                    };
                    return pde_model_update(self, id, |m| {
                        m.solver_options.report_statistics = on;
                        Ok(())
                    });
                }
                let scalar = obj_scalar_real(&value).filter(|x| x.is_finite() && *x > 0.0);
                let Some(val) = scalar else {
                    return Err(format!(
                        "SolverOptions.{sub} must be a positive finite real scalar"
                    ));
                };
                return pde_model_update(self, id, |m| match sub.as_str() {
                    "RelativeTolerance" => {
                        m.solver_options.relative_tolerance = val;
                        Ok(())
                    }
                    "AbsoluteTolerance" => {
                        m.solver_options.absolute_tolerance = val;
                        Ok(())
                    }
                    "ResidualTolerance" => {
                        m.solver_options.residual_tolerance = val;
                        Ok(())
                    }
                    "MaxIterations" => {
                        if val.fract() != 0.0 || val > 10_000.0 {
                            return Err(
                                "SolverOptions.MaxIterations must be a positive integer <= 10000"
                                    .into(),
                            );
                        }
                        m.solver_options.max_iterations = val as usize;
                        Ok(())
                    }
                    "MinStep" => {
                        m.solver_options.min_step = val;
                        Ok(())
                    }
                    _ => Err(format!(
                        "No public property '{sub}' exists for SolverOptions."
                    )),
                });
            }
            return Err("You cannot set the read-only property 'SolverOptions' of pde.PDEModel; set its fields (RelativeTolerance, AbsoluteTolerance, ResidualTolerance, MaxIterations, MinStep, ReportStatistics) instead.".into());
        }
        if matches!(
            &self.objects.entries[&id].state,
            ObjState::PdeModel(_)
                | ObjState::PdeGeometry(_)
                | ObjState::PdeMesh(_)
                | ObjState::PdeStationaryResults(_)
                | ObjState::PdeEigenResults(_)
                | ObjState::PdeTimeDependentResults(_)
        ) {
            let class = self.obj_class_name(id);
            return Err(format!(
                "You cannot set the read-only property '{name}' of {class}."
            ));
        }
        if let ObjState::Geo(g) = &self.objects.entries[&id].state {
            let g = g.clone();
            let changed = ObjState::Geo(Arc::new(super::registration::assign(
                self, &g, steps, value, out,
            )?));
            let entry = self.objects.entries.get_mut(&id).unwrap();
            entry.state = changed;
            return Ok(());
        }
        if self.obj_is_interpolant(id) {
            if steps.len() != 1 {
                return Err(
                    "Indexed assignment into an interpolant property is not supported".into(),
                );
            }
            return self.obj_set_interpolant_property(id, &name, value);
        }
        if matches!(&self.objects.entries[&id].state, ObjState::Filter(_)) {
            return Err(format!(
                "digitalFilter property '{name}' is read-only; design a new filter to change it"
            ));
        }
        if let ObjState::Fit(f) = &self.objects.entries[&id].state {
            return Err(format!(
                "{} property '{name}' is read-only; fit again or make a new object with cfit(fittype, values...)",
                f.class()
            ));
        }
        if matches!(&self.objects.entries[&id].state, ObjState::Regression(_)) {
            return Err(format!(
                "Model property '{name}' is read-only; fit a new model to change its value"
            ));
        }
        if matches!(&self.objects.entries[&id].state, ObjState::Distribution(_)) {
            return Err(format!(
                "Probability distribution property '{name}' is read-only; use makedist to create a changed value"
            ));
        }
        if matches!(
            &self.objects.entries[&id].state,
            ObjState::Classifier(_)
                | ObjState::MlPartition(_)
                | ObjState::Mlr(_)
                | ObjState::Parallel(_)
        ) {
            return Err(format!(
                "{} property '{name}' is read-only.",
                self.obj_class_name(id)
            ));
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
        let short = if self.obj_is_map(id) {
            "Map"
        } else {
            self.obj_class_name(id)
        };
        if entry.deleted {
            return format!("  handle to deleted {short}\n");
        }
        match &entry.state {
            ObjState::Triangulation(dt) => dt.body(),
            ObjState::Video(v) => v.body(),
            ObjState::VideoWriter(v) => v.body(),
            #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
            ObjState::Audio(a) => a.body(),
            ObjState::EconVol(m) => super::econ_vol::model_body(m),
            ObjState::Network(n) => n.body(),
            ObjState::Fis(o) => o.body(self),
            ObjState::Arima(m) => super::econ::body(m),
            ObjState::Mpc(m) => super::mpc::body(m),
            ObjState::MpcState(s) => super::mpc::state::body(s),
            ObjState::Distribution(d) => d.body(),
            ObjState::Regression(m) => m.body(),
            ObjState::Parallel(p) => p.body(),
            ObjState::Classifier(c) => c.body(),
            ObjState::MlPartition(p) => format!(
                "  cvpartition: {} observations, {} test sets\n",
                p.n,
                p.tests.len()
            ),
            ObjState::Mlr(m) => m.body(),
            ObjState::Filter(f) => f.body(),
            ObjState::Import(o) => super::import_options::body(self, o),
            ObjState::Fit(f) => f.body(),
            ObjState::RegConfig(c) => c.body(),
            ObjState::Geo(g) => super::registration::body(self, g),
            ObjState::Camera(c) => super::camera::body(self, c),
            ObjState::PdeModel(m) => m.body(),
            ObjState::PdeGeometry(g) => g.body(),
            ObjState::PdeMesh(m) => m.body(),
            ObjState::PdeStationaryResults(r) => r.body(),
            ObjState::PdeEigenResults(r) => r.body(),
            ObjState::PdeTimeDependentResults(r) => r.body(),
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
                let mut masker = ObjMasker::default();
                let fields = self.obj_parser_fields(id, &mut masker);
                let body = fields
                    .ok()
                    .and_then(|f| {
                        crate::display_format::disp(&f, &self.display)
                            .ok()
                            .flatten()
                    })
                    .unwrap_or_default();
                format!("  inputParser with properties:\n\n{}", masker.unmask(&body))
            }
            ObjState::Interpolant(InterpolantState::Gridded(g)) => format!(
                "  griddedInterpolant with properties:\n\n{:>23}: {}\n{:>23}: {}\n{:>23}: {}\n",
                "GridVectors",
                g.grids.len(),
                "Method",
                g.method,
                "ExtrapolationMethod",
                g.extrapolation
            ),
            ObjState::Interpolant(InterpolantState::Scattered(s)) => format!(
                "  scatteredInterpolant with properties:\n\n{:>23}: {}-by-2\n{:>23}: {}\n{:>23}: {}\n",
                "Points",
                s.points.len(),
                "Method",
                s.method,
                "ExtrapolationMethod",
                s.extrapolation
            ),
        }
    }
    /// The public properties as a struct; an empty cell is a stand-in that
    /// `masker.unmask` shows as `{1x0 cell}`.
    fn obj_parser_fields(&self, id: u64, masker: &mut ObjMasker) -> Result<Array> {
        let mut raw = Vec::with_capacity(PARSER_PROPERTIES.len());
        for n in PARSER_PROPERTIES {
            let a = self.obj_property(id, n)?.array(&self.cancel)?;
            masker.scan(&a, 0);
            raw.push(((*n).to_owned(), a));
        }
        let empty = self.obj_char(&masker.mask_for(EMPTY_CELL)?)?;
        let fields = raw
            .into_iter()
            .map(|(n, a)| {
                let a = match a {
                    Array::Cell(c) if c.is_empty() => empty.clone(),
                    other => self.obj_mask(&other, 0, masker)?.unwrap_or(other),
                };
                Ok((n, Arc::new(a)))
            })
            .collect::<Result<Vec<_>>>()?;
        self.obj_struct(&fields)
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
            if !entry.deleted && matches!(&entry.state, ObjState::Parser(_)) {
                let mut masker = ObjMasker::default();
                let fields = self.obj_parser_fields(id, &mut masker)?;
                let text = crate::display_format::echo(name, &fields, &self.display)?;
                let header = "  inputParser with properties:\n\n";
                let text = text.replacen(&marker, &(marker.clone() + header), 1);
                return Ok(Some(masker.unmask(&text)));
            }
            return Ok(Some(marker + &self.obj_body(id)));
        }
        if let Some(objects) = import_values(self, a)
            && objects.len() > 1
        {
            return Ok(Some(marker + &super::import_options::array_body(&objects)));
        }
        let mut masker = ObjMasker::default();
        masker.scan(a, 0);
        let Some(masked) = self.obj_mask(a, 0, &mut masker)? else {
            return Ok(None);
        };
        if let Array::Cell(c) = &masked {
            return crate::display_format::echo_cell_with_stand_ins(
                name,
                c,
                &self.display,
                &masker.replacements,
            )
            .map(Some);
        }
        let text = crate::display_format::echo(name, &masked, &self.display)?;
        Ok(Some(masker.unmask(&text)))
    }
    fn obj_mask_label(&self, id: u64) -> String {
        let class = match &self.objects.entries[&id].state {
            ObjState::Distribution(d) => d.class(),
            ObjState::Regression(model) => model.class(),
            ObjState::Import(o) => o.class(),
            ObjState::RegConfig(c) => c.class(),
            ObjState::Fit(f) => f.class(),
            ObjState::Network(_) => "network",
            #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
            ObjState::Audio(a) => a.class(),
            ObjState::Geo(g) => g.class(),
            ObjState::Camera(c) => c.class(),
            ObjState::PdeModel(_) => super::pde_model::MODEL_CLASS,
            ObjState::PdeGeometry(_) => super::pde_model::GEOMETRY_CLASS,
            ObjState::PdeMesh(_) => super::pde_mesh::MESH_CLASS,
            ObjState::PdeStationaryResults(_) => super::pde_solve::RESULTS_CLASS,
            ObjState::PdeEigenResults(_) => super::pde_solve::EIGEN_RESULTS_CLASS,
            ObjState::PdeTimeDependentResults(_) => super::pde_solve::time::TIME_RESULTS_CLASS,
            _ if self.obj_is_map(id) => MAP,
            _ if self.obj_is_interpolant(id) => {
                return format!("[1x1 {}]", self.obj_class_name(id));
            }
            _ => PARSER,
        };
        format!("[1x1 {class}]")
    }
    /// A copy of `a` with every object replaced by its display stand-in.
    fn obj_mask(&self, a: &Array, depth: usize, masker: &mut ObjMasker) -> Result<Option<Array>> {
        if depth > 8 {
            return Ok(None);
        }
        if let Some(id) = self.obj_id(a) {
            let label = self.obj_mask_label(id);
            return Ok(Some(self.obj_char(&masker.mask_for(&label)?)?));
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
            match self.obj_mask(&slot, depth + 1, masker)? {
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
        if let Some(value) = self.dtm_static(e, out)? {
            return Ok(Some(value));
        }
        // registration.optimizer.<Class>(...) / registration.metric.<Class>(...).
        if let Some((index, args)) = super::image_basic::registration_config::constructor_expr(e)
            && !self.contains_array("registration")
        {
            let values = match args {
                Some(args) => self.function_arguments(args, None, out)?,
                None => Vec::new(),
            };
            return super::image_basic::registration_config::construct(self, index, &values)
                .map(Some);
        }
        match e {
            Expr::Index(base, args) => {
                let Expr::Field(inner, member) = base.as_ref() else {
                    return Ok(None);
                };
                if member == "getProfiles"
                    && matches!(inner.as_ref(), Expr::Name(n) if n == "VideoWriter")
                    && !self.contains_array("VideoWriter")
                {
                    if !args.is_empty() {
                        return Err(MANY.into());
                    }
                    return super::video::profiles(self).map(Some);
                }
                if member == "getFileFormats"
                    && matches!(inner.as_ref(), Expr::Name(n) if n == "VideoReader")
                    && !self.contains_array("VideoReader")
                {
                    if !args.is_empty() {
                        return Err(MANY.into());
                    }
                    return super::video::formats(self).map(Some);
                }
                if member == "Map" && obj_is_containers(inner) && !self.contains_array("containers")
                {
                    let values = self.function_arguments(args, None, out)?;
                    return self.obj_map_new(values, out).map(Some);
                }
                // A workspace variable named dfilt wins, as containers.Map does.
                if member == "df1" && obj_is_dfilt(inner) && !self.contains_array("dfilt") {
                    let values = self.function_arguments(args, None, out)?;
                    let filter = super::digital_filter::df1(&values)?;
                    return self.filter_new(filter).map(Some);
                }
                if member == "makeValidName"
                    && let Expr::Field(root, lang) = inner.as_ref()
                    && lang == "lang"
                    && matches!(root.as_ref(), Expr::Name(n) if n == "matlab")
                    && !self.contains_array("matlab")
                {
                    let values = self.function_arguments(args, None, out)?;
                    return self.make_valid_name_call(&values).map(Some);
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
                if name == "VideoWriter"
                    && !self.contains_array(name)
                    && self.resolve_function(name)?.is_none()
                {
                    let values = self.function_arguments(args, None, out)?;
                    let writer = super::video::Writer::new(self, &values)?;
                    return self.obj_new(ObjState::VideoWriter(writer)).map(Some);
                }
                if name == "VideoReader"
                    && !self.contains_array(name)
                    && self.resolve_function(name)?.is_none()
                {
                    let values = self.function_arguments(args, None, out)?;
                    let reader = super::video::Reader::new(self, &values)?;
                    return self.obj_new(ObjState::Video(reader)).map(Some);
                }
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
                // Futures and jobs concatenate into arrays, as fetchNext expects.
                if matches!(
                    self.objects.entries[&id].state,
                    ObjState::Parallel(_) | ObjState::Fis(_)
                ) {
                    return Ok(None);
                }
                let direction = if rows.len() > 1 {
                    "Vertical"
                } else {
                    "Horizontal"
                };
                let geo_class;
                let class = if self.obj_is_map(id) {
                    "a containers.Map"
                } else if let ObjState::Geo(g) = &self.objects.entries[&id].state {
                    geo_class = format!("a {}", g.class());
                    geo_class.as_str()
                } else if let ObjState::Camera(c) = &self.objects.entries[&id].state {
                    geo_class = format!("a {}", c.class());
                    geo_class.as_str()
                } else if matches!(
                    &self.objects.entries[&id].state,
                    ObjState::PdeModel(_)
                        | ObjState::PdeGeometry(_)
                        | ObjState::PdeMesh(_)
                        | ObjState::PdeStationaryResults(_)
                        | ObjState::PdeEigenResults(_)
                        | ObjState::PdeTimeDependentResults(_)
                ) {
                    geo_class = format!("a {}", self.obj_class_name(id));
                    geo_class.as_str()
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
        // Handle objects of user classes compare by identity too.
        if let Some(value) = self.cls_equality(op, left, right)? {
            return Ok(Some(value));
        }
        let (left, right) = (self.obj_value_id(left), self.obj_value_id(right));
        // Registration objects are value classes without eq/ne.
        let value_class = |id: u64| match &self.objects.entries[&id].state {
            ObjState::Geo(g) => Some(g.class()),
            ObjState::Camera(c) => Some(c.class()),
            ObjState::Triangulation(_) => Some(super::sci_more::triangulation::CLASS),
            ObjState::PdeGeometry(_) => Some(super::pde_model::GEOMETRY_CLASS),
            ObjState::PdeMesh(_) => Some(super::pde_mesh::MESH_CLASS),
            ObjState::PdeStationaryResults(_) => Some(super::pde_solve::RESULTS_CLASS),
            ObjState::PdeEigenResults(_) => Some(super::pde_solve::EIGEN_RESULTS_CLASS),
            ObjState::PdeTimeDependentResults(_) => {
                Some(super::pde_solve::time::TIME_RESULTS_CLASS)
            }
            _ => None,
        };
        if let Some(class) = left.or(right).and_then(value_class) {
            return Err(self.raise(
                format!(
                    "Undefined function '{}' for input arguments of type '{class}'.",
                    obj_operator(op)
                ),
                "MATLAB:UndefinedFunction".into(),
            ));
        }
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
        if self.objects.entries.is_empty()
            && !matches!(&st.expr, Expr::Call(name, _) if name == "set")
        {
            return Ok(false);
        }
        if st.targets.len() > 1
            && let Expr::Call(name, args) = &st.expr
            && let Some((id, _)) = self.obj_var(name)
            && let ObjState::Network(n) = &self.objects.entries[&id].state
        {
            let n = n.clone();
            if n.dynamic.is_none() {
                return Err("network: static simulation returns one output".into());
            }
            let args = self.function_arguments(args, None, out)?;
            let values = super::nnet::dynamic::simulate(self, &n, &args)?;
            if st.targets.len() > values.len() {
                return Err("Too many output arguments.".into());
            }
            self.charge_function_outputs(&values)?;
            for (target, value) in st.targets.iter().zip(values) {
                if let Some(target) = target {
                    let a = value.array(&self.cancel)?;
                    self.assign_workspace_array(target, a, st.quiet, out)?;
                }
            }
            return Ok(true);
        }
        if let Some((root, steps)) = &st.path_target
            && let Some((id, _)) = self.obj_var(root)
            && let ObjState::Arima(m) = &self.objects.entries[&id].state
        {
            let m = m.clone();
            let value = self.eval(&st.expr, None, out)?.array(&self.cancel)?;
            let changed = super::econ::assign(self, &m, steps, value, out)?;
            let object = arima_new(self, changed)?.array(&self.cancel)?;
            self.assign_workspace_array(root, object, true, out)?;
            self.obj_echo(root, st.quiet, out)?;
            return Ok(true);
        }
        if let Some((root, steps)) = &st.path_target
            && let Some((id, _)) = self.obj_var(root)
            && let ObjState::Mpc(m) = &self.objects.entries[&id].state
        {
            let m = m.clone();
            let value = self.eval(&st.expr, None, out)?;
            if matches!(value, Value::Void) {
                return Err("This command does not return a value".into());
            }
            let value = value.array(&self.cancel)?;
            let changed = super::mpc::assign(self, &m, steps, value, out)?;
            let object = mpc_new(self, changed)?.array(&self.cancel)?;
            self.assign_workspace_array(root, object, true, out)?;
            self.obj_echo(root, st.quiet, out)?;
            return Ok(true);
        }
        if let Some((root, steps)) = &st.path_target
            && let Some((id, _)) = self.obj_var(root)
            && let ObjState::Fis(o) = &self.objects.entries[&id].state
        {
            let o = o.clone();
            let value = self.eval(&st.expr, None, out)?.array(&self.cancel)?;
            let changed = super::fuzzy::properties::assign(self, &o, steps, value, out)?;
            let object = super::fuzzy::new_array(self, changed)?;
            self.assign_workspace_array(root, object, true, out)?;
            self.obj_echo(root, st.quiet, out)?;
            return Ok(true);
        }
        if let Some((root, steps)) = &st.path_target
            && let Some((id, _)) = self.obj_var(root)
            && let ObjState::EconVol(m) = &self.objects.entries[&id].state
        {
            let m = m.clone();
            let value = self.eval(&st.expr, None, out)?.array(&self.cancel)?;
            let changed = super::econ_vol::model_assign(self, &m, steps, value, out)?;
            let object = econ_vol_new(self, changed)?.array(&self.cancel)?;
            self.assign_workspace_array(root, object, true, out)?;
            self.obj_echo(root, st.quiet, out)?;
            return Ok(true);
        }
        if let Some((root, steps)) = &st.path_target
            && let Some((id, _)) = self.obj_var(root)
            && let ObjState::Network(n) = &self.objects.entries[&id].state
        {
            let n = n.clone();
            let value = self.eval(&st.expr, None, out)?.array(&self.cancel)?;
            let changed = super::nnet::assign(self, &n, steps, value, out)?;
            let object = nnet_new(self, changed)?.array(&self.cancel)?;
            self.assign_workspace_array(root, object, true, out)?;
            self.obj_echo(root, st.quiet, out)?;
            return Ok(true);
        }
        if let Some((root, steps)) = &st.path_target
            && let Some((id, _alive)) = self.obj_var(root)
            && let ObjState::RegConfig(c) = &self.objects.entries[&id].state
        {
            // Value semantics: the variable gets a changed copy.
            let c = *c;
            let value = self.eval(&st.expr, None, out)?;
            if matches!(value, Value::Void) {
                return Err("This command does not return a value".into());
            }
            let value = value.array(&self.cancel)?;
            let changed = self.obj_reg_config_assign(c, steps, &value, out)?;
            let object = reg_config_new(self, changed)?.array(&self.cancel)?;
            self.assign_workspace_array(root, object, true, out)?;
            self.obj_echo(root, st.quiet, out)?;
            return Ok(true);
        }
        if let Some((root, steps)) = &st.path_target
            && let Some((id, _alive)) = self.obj_var(root)
            && let ObjState::Import(o) = &self.objects.entries[&id].state
        {
            // Value semantics: the variable gets a changed copy; other
            // copies of the object keep their values.
            let o = o.clone();
            let value = self.eval(&st.expr, None, out)?;
            if matches!(value, Value::Void) {
                return Err("This command does not return a value".into());
            }
            let value = value.array(&self.cancel)?;
            let changed = super::import_options::assign(self, &o, steps, value, out)?;
            let object = self
                .obj_new(ObjState::Import(changed))?
                .array(&self.cancel)?;
            self.assign_workspace_array(root, object, true, out)?;
            self.obj_echo(root, st.quiet, out)?;
            return Ok(true);
        }
        if let Some((root, steps)) = &st.path_target
            && let Some((id, _alive)) = self.obj_var(root)
            && let ObjState::Camera(c) = &self.objects.entries[&id].state
        {
            // Read-only value object: report before evaluating the right side.
            let c = c.clone();
            super::camera::assign(self, &c, steps, out)?;
        }
        if let Some((root, steps)) = &st.path_target
            && let Some((id, _alive)) = self.obj_var(root)
            && let ObjState::Geo(g) = &self.objects.entries[&id].state
        {
            // Value semantics: the variable gets a changed copy.
            let g = g.clone();
            let value = self.eval(&st.expr, None, out)?;
            if matches!(value, Value::Void) {
                return Err("This command does not return a value".into());
            }
            let value = value.array(&self.cancel)?;
            let changed = super::registration::assign(self, &g, steps, value, out)?;
            let object = geo_new(self, changed)?.array(&self.cancel)?;
            self.assign_workspace_array(root, object, true, out)?;
            self.obj_echo(root, st.quiet, out)?;
            return Ok(true);
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
            && let ObjState::Fis(o) = &self.objects.entries[&id].state
        {
            let o = o.clone();
            let value = self.eval(&st.expr, None, out)?.array(&self.cancel)?;
            let changed = super::fuzzy::properties::assign(
                self,
                &o,
                &[PathStep::Paren(args.clone())],
                value,
                out,
            )?;
            let object = super::fuzzy::new_array(self, changed)?;
            self.assign_workspace_array(name, object, true, out)?;
            self.obj_echo(name, st.quiet, out)?;
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
                if (args.first().is_some_and(|a| self.obj_expr_var(a).is_some())
                    || name == "set")
                    && super::app_launchers::Launch::for_name(name).is_none()
                    && !self.contains_array(name)
                    && self.resolve_function(name)?.is_none() =>
            {
                // Fuzzy plots distinguish data outputs from no-output plots.
                // Let their statement dispatcher below select nargout=0.
                if matches!(
                    name.as_str(),
                    "plotmf"
                        | "gensurf"
                        | "plotfis"
                        | "writefis"
                        | "fuzzyLogicDesigner"
                        | "appdesigner"
                        | "mpcDesigner"
                        | "pdegplot"
                        | "pdemesh"
                        | "pdeplot"
                ) {
                    return Ok(false);
                }
                let mut target_path = None;
                let values = if name == "set"
                    && args.len() > 1
                    && let Some(root) = args.first().and_then(mpc_set_root)
                    && let Some(array) = self.array(root)
                    && (mpc_value(self, &Value::from(array.clone())).is_some()
                        || (matches!(array, Array::Cell(_) | Array::Struct(_))
                            && self.obj_id(&array).is_none()
                            && !super::classdef::is_object(&array)))
                {
                    let (root, steps, first) = mpc_set_target(self, &args[0], out)?;
                    target_path = Some((root, steps));
                    let mut values = vec![first];
                    values.extend(self.function_arguments(&args[1..], None, out)?);
                    values
                } else {
                    self.function_arguments(args, None, out)?
                };
                if matches!(name.as_str(), "setEstimator" | "setindist" | "setoutdist")
                    && let Some(Expr::Name(var)) = args.first()
                    && let Some(m) = values.first().and_then(|v| mpc_value(self, v))
                {
                    let updated = super::mpc::estimator::setter(self, &m, name, &values[1..], out)?
                        .array(&self.cancel)?;
                    self.assign_workspace_array(var, updated, true, out)?;
                    return Ok(true);
                }
                if name == "set"
                    && values.len() > 1
                    && values.first().and_then(|v| mpc_value(self, v)).is_some()
                {
                    let (root, steps) =
                        target_path.ok_or("mpc: no-output set target must be assignable")?;
                    let updated = self
                        .obj_call(name, &values, out)?
                        .expect("mpc set handles this")
                        .array(&self.cancel)?;
                    let updated =
                        self.assign_step(self.array(&root), &steps, updated, false, out)?;
                    self.assign_workspace_array(&root, updated, true, out)?;
                    self.obj_echo(&root, st.quiet, out)?;
                    return Ok(true);
                }
                if name == "set"
                    && let Some(Value::Container(a)) = values.first()
                    && matches!(super::control_lti::class(a), Some("tf" | "zpk" | "ss"))
                {
                    let (root, steps) =
                        target_path.ok_or("No-output LTI set requires an assignable model")?;
                    let value = super::control_lti::lti_set_options(self, a, &values[1..])?
                        .array(&self.cancel)?;
                    let updated = self.assign_step(self.array(&root), &steps, value, false, out)?;
                    self.assign_workspace_array(&root, updated, true, out)?;
                    return Ok(true);
                }
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
        let mut outputs = match &self.objects.entries[&id].state {
            ObjState::Triangulation(dt) => vec![
                Value::Matrix(Matrix::scalar(dt.connectivity.rows() as f64)),
                Value::Matrix(Matrix::scalar(dt.connectivity.cols() as f64)),
            ],
            ObjState::Fis(o) => vec![
                Value::Matrix(Matrix::scalar(o.rows as f64)),
                Value::Matrix(Matrix::scalar(o.cols as f64)),
            ],
            _ => vec![Value::Matrix(Matrix::scalar(self.obj_size(id)))],
        };
        outputs.truncate(count);
        outputs.resize(count, Value::Matrix(Matrix::scalar(1.)));
        Ok(Some(outputs))
    }
    pub(super) fn obj_assign_nested(
        &mut self,
        current: &Array,
        steps: &[PathStep],
        value: &Array,
        out: &mut Execution,
    ) -> Result<Option<Array>> {
        let Some(id) = self.obj_id(current) else {
            return Ok(None);
        };
        match self.objects.entries[&id].state.clone() {
            ObjState::MpcState(_) => {
                self.obj_set_property(id, steps, value.clone(), out)?;
                Ok(Some(current.clone()))
            }
            ObjState::Arima(m) => {
                let changed = super::econ::assign(self, &m, steps, value.clone(), out)?;
                Ok(Some(arima_new(self, changed)?.array(&self.cancel)?))
            }
            ObjState::Mpc(m) => {
                let changed = super::mpc::assign(self, &m, steps, value.clone(), out)?;
                Ok(Some(mpc_new(self, changed)?.array(&self.cancel)?))
            }
            ObjState::EconVol(m) => {
                let changed = super::econ_vol::model_assign(self, &m, steps, value.clone(), out)?;
                Ok(Some(econ_vol_new(self, changed)?.array(&self.cancel)?))
            }
            ObjState::Fis(o) => {
                let changed =
                    super::fuzzy::properties::assign(self, &o, steps, value.clone(), out)?;
                Ok(Some(super::fuzzy::new_array(self, changed)?))
            }
            ObjState::Network(n) => {
                if steps.len() > 1 {
                    return Err("network: extract the network to a variable before editing nested container properties".into());
                }
                let changed = super::nnet::assign(self, &n, steps, value.clone(), out)?;
                Ok(Some(nnet_new(self, changed)?.array(&self.cancel)?))
            }
            ObjState::RegConfig(c) => {
                let changed = self.obj_reg_config_assign(c, steps, value, out)?;
                Ok(Some(reg_config_new(self, changed)?.array(&self.cancel)?))
            }
            ObjState::Import(o) => {
                let changed = super::import_options::assign(self, &o, steps, value.clone(), out)?;
                Ok(Some(
                    self.obj_new(ObjState::Import(changed))?
                        .array(&self.cancel)?,
                ))
            }
            ObjState::Geo(g) => {
                let changed = super::registration::assign(self, &g, steps, value.clone(), out)?;
                Ok(Some(geo_new(self, changed)?.array(&self.cancel)?))
            }
            ObjState::Camera(c) => {
                let changed = super::camera::assign(self, &c, steps, out)?;
                Ok(Some(camera_new(self, changed)?.array(&self.cancel)?))
            }
            _ => {
                if self.obj_is_map(id)
                    && let [PathStep::Paren(args)] = steps
                {
                    self.obj_assign_key_value(id, args, value.clone(), out)?;
                } else {
                    self.obj_set_property(id, steps, value.clone(), out)?;
                }
                Ok(Some(current.clone()))
            }
        }
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
    /// `object.Name = value` on a registration optimizer/metric value object.
    fn obj_reg_config_assign(
        &mut self,
        c: super::image_basic::registration_config::RegConfig,
        steps: &[PathStep],
        value: &Array,
        out: &mut Execution,
    ) -> Result<super::image_basic::registration_config::RegConfig> {
        let name = match steps {
            [PathStep::Field(name)] => name.clone(),
            [PathStep::Dynamic(key)] => self.dynamic_field_name(key, out)?,
            _ => {
                return Err(format!(
                    "{}: assign the complete property value (obj.Name = value).",
                    c.class()
                ));
            }
        };
        c.assign(&name, value)
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
