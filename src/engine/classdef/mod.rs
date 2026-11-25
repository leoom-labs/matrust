//! User-defined classes from `classdef` files: value classes, handle classes
//! (`handles.rs`), inheritance (`inherit.rs`) and property get/set methods.
//!
//! A class file loads like a function file (same per-run cache, `.mr` before
//! `.m`) into a [`Program`] whose first statements are the methods (property
//! get/set methods included), a generated constructor when the file has none,
//! and one generated function per property default; local functions follow.
//! A class's [`ClassInfo`] lists every property and method it has, inherited
//! ones included (each remembers its defining class and program). A value
//! object is a 1x1 struct holding the instance properties plus two hidden
//! fields: the language-object tag and the class name. A handle object holds
//! the same two fields plus an identity slot; its properties live in the
//! engine (`handles.rs`). Calls dispatch here only when an argument carries
//! that tag.
//!
//! A class folder `@Name` holds the class file `Name.m` and, each in its own
//! file, further methods (`@Name/method.m`). They are listed when the class
//! loads and read when first called in a run (`cls_target`); the classdef
//! may declare one without a body to make it Static or to set its Access.
mod events;
mod handles;
mod inherit;

use super::user_functions::{Program, ResolvedFunction};
use super::{Engine, Execution, Value};
use crate::{
    Array, CellArray, CharacterRow, LogicalMatrix, Matrix, Result, StructArray,
    syntax::{
        self, Access, ArgDim, ArgumentSpec, ClassDefinition, Evaluation, Expr, FunctionDefinition,
        PathStep, Statement, StatementKind,
    },
};
pub(super) use events::enum_equal;
pub(super) use handles::{HandleEquality, HandleTable};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};

/// Hidden field holding the class name of a user object.
pub(super) const CLASS_FIELD: &str = "MatRustClassName";
/// Value of the language-object tag field for user objects.
const USER_TAG: f64 = 4.;
pub(super) const OBJECT_ARRAYS: &str = "object arrays are not supported yet";
/// Output name of the generated property-default functions.
const DEFAULT_OUTPUT: &str = "default_value";

/// Methods every handle class has (`methods` lists them).
const HANDLE_METHODS: [&str; 4] = ["delete", "eq", "isvalid", "ne"];
/// Built-in App Designer base class (a handle class).
pub(super) const APPBASE_CLASS: &str = "matlab.apps.AppBase";
/// Built-in methods inherited from `matlab.apps.AppBase`.
const APPBASE_METHODS: [&str; 3] = ["createCallbackFcn", "registerApp", "runStartupFcn"];

#[derive(Clone)]
pub(super) struct PropertyInfo {
    pub name: String,
    pub get: Access,
    pub set: Access,
    pub constant: bool,
    /// No storage; read through the get method.
    pub dependent: bool,
    pub hidden: bool,
    /// Set only by the defining class's constructor.
    pub immutable: bool,
    /// Statement index (in the defining program) of the generated function
    /// computing the default.
    pub default: Option<usize>,
    /// Optional size/class/validator constraints on the property value.
    pub validation: Option<crate::syntax::ArgumentSpec>,
    /// Statement indices (in the defining program) of `get.Name` / `set.Name`.
    pub getter: Option<usize>,
    pub setter: Option<usize>,
    /// The defining class.
    pub owner: String,
    /// An inherited property: the defining program and the property's index
    /// there. None for the class's own properties.
    pub inherited: Option<(Arc<Program>, usize)>,
}
impl PropertyInfo {
    /// The program defining this property (its default, get and set methods).
    fn program(&self, own: &Arc<Program>) -> Arc<Program> {
        self.inherited
            .as_ref()
            .map_or_else(|| own.clone(), |(p, _)| p.clone())
    }
}
#[derive(Clone)]
pub(super) struct MethodInfo {
    pub index: usize,
    pub is_static: bool,
    pub access: Access,
    /// Declared in a `methods (Abstract)` block and not implemented.
    pub is_abstract: bool,
    /// The defining class.
    pub owner: String,
    /// The defining program for an inherited method (None: this class).
    pub program: Option<Arc<Program>>,
    /// A method kept in its own file of the defining class's folder: it runs
    /// in that file's program (`Engine::cls_target`), `index` is not used.
    pub file: Option<Arc<MethodFile>>,
}
impl MethodInfo {
    /// The class program of the defining class; the method's statements are
    /// there unless it has its own file.
    fn target(&self, own: &Arc<Program>) -> Arc<Program> {
        self.program.clone().unwrap_or_else(|| own.clone())
    }
}
/// A method file of a class folder: `folder/name.mr`, else `folder/name.m`.
pub(super) struct MethodFile {
    folder: PathBuf,
    name: String,
}
/// Directory entries of a class folder looked at when its class loads.
const MAX_FOLDER_ENTRIES: usize = 4096;

/// The method files of the class folder `folder` by method name: every
/// `name.mr` / `name.m` whose name is a function name, except the class file
/// `class_file` itself; `.mr` wins over `.m`. Subfolders (`private`) are
/// not methods.
fn folder_methods(
    engine: &Engine,
    folder: &Path,
    class_file: &str,
) -> Result<BTreeMap<String, PathBuf>> {
    let mut found: BTreeMap<String, PathBuf> = BTreeMap::new();
    let failed = |e: std::io::Error| format!("{}: {e}", folder.display());
    for (count, entry) in std::fs::read_dir(folder).map_err(failed)?.enumerate() {
        if count >= MAX_FOLDER_ENTRIES {
            return Err(format!(
                "{}: a class folder supports at most {MAX_FOLDER_ENTRIES} files",
                folder.display()
            ));
        }
        if count % 256 == 0 {
            engine.check_cancel()?;
        }
        let entry = entry.map_err(failed)?;
        let file_name = entry.file_name();
        let Some((stem, extension)) = file_name.to_str().and_then(|n| n.rsplit_once('.')) else {
            continue;
        };
        if !matches!(extension, "mr" | "m")
            || stem == class_file
            || !crate::files::valid_name(stem)
            || entry.file_type().is_ok_and(|kind| kind.is_dir())
        {
            continue;
        }
        if extension == "mr" || !found.contains_key(stem) {
            found.insert(stem.to_owned(), entry.path());
        }
    }
    Ok(found)
}
pub(super) struct ClassInfo {
    pub name: String,
    pub path: PathBuf,
    /// Own properties first (declaration order), then each superclass's, left
    /// to right; a property reached twice through a common base appears once.
    pub properties: Vec<PropertyInfo>,
    pub methods: BTreeMap<String, MethodInfo>,
    /// Methods two superclasses define differently and this class does not:
    /// calling them errors, naming both classes.
    pub ambiguous: BTreeMap<String, (String, String)>,
    pub constructor: usize,
    /// Statements before this index run with the class's private access.
    pub inside: usize,
    /// Direct superclasses in declaration order (`handle` included).
    pub supers: Vec<String>,
    /// The loaded programs of the direct superclasses (`handle` has none).
    pub super_programs: Vec<Arc<Program>>,
    /// Every superclass, depth first, left to right, each once (`handle`
    /// last of its branch): `superclasses` and `isa`.
    pub ancestors: Vec<String>,
    /// A handle class: derives from `handle`, directly or not.
    pub is_handle: bool,
    pub sealed: bool,
    /// Direct superclasses the constructor calls itself (`obj@Super(...)`);
    /// the others get an implicit call without arguments first.
    pub explicit_supers: BTreeSet<String>,
    /// Events of the class and its superclasses, own first.
    pub events: Vec<String>,
    /// `enumeration` members: name and constructor arguments.
    pub enumeration: Vec<(String, Vec<Expr>)>,
}
impl ClassInfo {
    /// The name of the constructor: the class name without its package.
    pub(super) fn constructor_name(&self) -> &str {
        self.name.rsplit('.').next().unwrap_or(&self.name)
    }
    fn property(&self, name: &str) -> Option<usize> {
        self.properties.iter().position(|p| p.name == name)
    }
    /// Public, not Hidden properties in order (properties, fieldnames).
    fn public_properties(&self) -> Vec<String> {
        self.properties
            .iter()
            .filter(|p| p.get == Access::Public && !p.hidden)
            .map(|p| p.name.clone())
            .collect()
    }
    /// Public methods in character-code order (the reference sorts so:
    /// `Pt` before `make`), constructor and inherited methods included;
    /// a handle class adds the handle methods.
    fn public_methods(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .methods
            .iter()
            .filter(|(_, m)| m.access == Access::Public)
            .map(|(n, _)| n.clone())
            .chain(self.ambiguous.keys().cloned())
            .collect();
        if !self.methods.contains_key(self.constructor_name()) {
            names.push(self.constructor_name().to_owned());
        }
        if self.is_handle {
            names.extend(HANDLE_METHODS.iter().map(|m| (*m).to_owned()));
        }
        if self.is_a(APPBASE_CLASS) {
            names.extend(APPBASE_METHODS.iter().map(|m| (*m).to_owned()));
        }
        names.sort();
        names.dedup();
        names
    }
    /// `isa(obj, wanted)`: the class itself or any superclass.
    fn is_a(&self, wanted: &str) -> bool {
        self.name == wanted || self.ancestors.iter().any(|a| a == wanted)
    }
    /// Abstract methods not implemented, in name order.
    fn abstract_methods(&self) -> Vec<String> {
        self.methods
            .iter()
            .filter(|(_, m)| m.is_abstract)
            .map(|(n, _)| n.clone())
            .collect()
    }
    /// The method `name` callable on an object of this class (not Static),
    /// or the ambiguity error.
    fn instance_method(&self, name: &str) -> Result<Option<&MethodInfo>> {
        if let Some((a, b)) = self.ambiguous.get(name) {
            return Err(ambiguous_message(&self.name, name, a, b));
        }
        Ok(self.methods.get(name).filter(|m| !m.is_static))
    }
}
fn ambiguous_message(class: &str, method: &str, a: &str, b: &str) -> String {
    format!(
        "Method '{method}' of class '{class}' is ambiguous: the superclasses '{a}' and '{b}' both define it; define {method} in {class} to choose."
    )
}
fn abstract_call_message(class: &str, method: &str) -> String {
    format!("The abstract method '{method}' of class '{class}' has no implementation.")
}

/// Per-run evaluated property defaults; `seen` survives runs because objects do.
#[derive(Clone, Default)]
pub(super) struct ClassRunState {
    /// Some class file was loaded in this engine. Only a cheap guard for the
    /// statement forms that route method calls; display goes by the tag of the
    /// displayed value, never by this flag.
    pub seen: bool,
    /// The definition each class name had when last loaded, kept across
    /// runs: display of an object whose class is not loaded in this run
    /// still shows only the public properties.
    known: BTreeMap<String, Arc<ClassInfo>>,
    values: BTreeMap<PathBuf, Vec<Option<Array>>>,
    loading: BTreeSet<(PathBuf, usize)>,
    /// Class files whose defaults were evaluated (or are being) in this run.
    started: BTreeSet<PathBuf>,
    /// Handle objects and their properties; they outlive runs like the
    /// variables holding them.
    pub(super) handles: HandleTable,
    /// The object a superclass constructor call (`obj@Super(...)`, or the
    /// implicit one) hands to that constructor instead of a new default object.
    super_preset: Option<Array>,
    /// Classes whose files are being laid out (inheritance cycles).
    building: Vec<String>,
    /// Nesting of handle comparisons in isequal (cyclic references).
    pub(super) equal_depth: usize,
    /// The running user function: the last handle id that existed when it
    /// was entered, the evaluation depth its statements run at and its
    /// nesting level (handles.rs `cls_collect_function_cycles`).
    pub(super) function_frame: Option<handles::HandleFrame>,
    /// Class names granted private/protected access during `AppBase` callback
    /// and `runStartupFcn` dispatch (restored on every exit path).
    pub(super) privileged_classes: Vec<String>,
}
impl ClassRunState {
    pub(super) fn after_run(&mut self) -> Self {
        Self {
            seen: self.seen,
            known: self.known.clone(),
            handles: std::mem::take(&mut self.handles),
            ..Self::default()
        }
    }
    /// Classes that still have an instance, plus their superclasses: `clear
    /// classes` must not drop those definitions.
    pub(super) fn classes_to_keep(&self, live: &BTreeSet<String>) -> BTreeSet<String> {
        let mut keep = live.clone();
        for name in live {
            if let Some(info) = self.known.get(name) {
                keep.extend(info.ancestors.iter().cloned());
            }
        }
        keep
    }
    /// `clear classes` / `clear all`. Definitions in `keep` stay.
    pub(super) fn clear_cached(&mut self, keep: &BTreeSet<String>) {
        self.known.retain(|name, _| keep.contains(name));
        let paths: BTreeSet<PathBuf> = self.known.values().map(|info| info.path.clone()).collect();
        self.values.retain(|path, _| paths.contains(path));
        self.started.retain(|path| paths.contains(path));
        self.loading.retain(|(path, _)| paths.contains(path));
        if keep.is_empty() {
            self.building.clear();
        }
    }
}

/// True for a user object (a struct carrying the user-object tag or a typed empty object array).
pub(super) fn is_object(a: &Array) -> bool {
    let Array::Struct(s) = a else {
        return false;
    };
    s.empty_object_class().is_some()
        || (s.len() == 1
            && s.field_index(super::lang_more::TAG)
                .and_then(|f| s.get(f, 0).ok())
                .is_some_and(|v| matches!(v.as_ref(), Array::Numeric(m) if m.data() == [USER_TAG])))
}
pub(super) fn is_object_value(v: &Value) -> bool {
    matches!(v, Value::Container(a) if is_object(a))
}
/// Class name of a user object.
pub(super) fn object_class(a: &Array) -> Option<String> {
    let Array::Struct(s) = a else { return None };
    if let Some(class) = s.empty_object_class() {
        return Some(class.to_owned());
    }
    if !is_object(a) {
        return None;
    }
    let Array::Character(c) = s.get(s.field_index(CLASS_FIELD)?, 0).ok()?.as_ref() else {
        return None;
    };
    Some(String::from_utf16_lossy(c.units()))
}
/// Functions `cls_call` answers for a user object as first argument, besides
/// the methods of its class.
fn is_object_function(name: &str) -> bool {
    matches!(
        name,
        "class"
            | "isvalid"
            | "delete"
            | "eq"
            | "ne"
            | "superclasses"
            | "isa"
            | "isobject"
            | "isstruct"
            | "isfield"
            | "numfields"
            | "getfield"
            | "setfield"
            | "rmfield"
            | "orderfields"
            | "struct2cell"
            | "subsasgn"
            | "fieldnames"
            | "methods"
            | "properties"
            | "ismethod"
            | "isprop"
            | "struct"
            | "disp"
            | "display"
            | "horzcat"
            | "vertcat"
            | "cat"
            | "repmat"
            | "addlistener"
            | "notify"
            | "events"
            | "enumeration"
            | "metaclass"
            | "char"
            | "string"
            | "registerApp"
            | "runStartupFcn"
            | "createCallbackFcn"
    )
}
/// Hidden fields of every struct-backed object.
pub(super) fn hidden_field(name: &str) -> bool {
    name == CLASS_FIELD || name == handles::HANDLE_FIELD || name == events::ENUM_FIELD
}
fn text(v: &Value) -> Option<String> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
        Value::Container(Array::Character(c)) if c.rows() <= 1 => {
            Some(String::from_utf16_lossy(c.units()))
        }
        _ => None,
    }
}
/// Element `i` of a subscript structure: its type text and its subs.
fn subs_element(s: &StructArray, i: usize) -> Result<(String, Array)> {
    let kind = s.get(s.field_index("type").ok_or("Missing type")?, i)?;
    let kind = text(&Value::from(kind.as_ref().clone())).ok_or("Invalid subscript type.")?;
    let sub = s.get(s.field_index("subs").ok_or("Missing subs")?, i)?;
    Ok((kind, sub.as_ref().clone()))
}
fn logical(b: bool) -> Value {
    Value::Logical(LogicalMatrix::scalar(b))
}

// The builtin identity cases in cls_identity, using the declared class for
// empty operands, which have no handle slot or enumeration member field.
fn cls_empty_identity_compatible(engine: &Engine, left: &Value, right: &Value) -> bool {
    let kinds = |v: &Value| {
        let Value::Container(a) = v else {
            return (false, false);
        };
        if a.is_empty() {
            object_class(a)
                .and_then(|name| engine.functions.classes.known.get(&name))
                .map_or((false, false), |class| {
                    (
                        class.is_handle,
                        !class.is_handle && !class.enumeration.is_empty(),
                    )
                })
        } else {
            (handles::is_handle_object(a), events::enum_name(a).is_some())
        }
    };
    let (left_handle, left_enum) = kinds(left);
    let (right_handle, right_enum) = kinds(right);
    (left_handle && right_handle) || (left_enum && (right_enum || text(right).is_some()))
}

fn cls_implicit_default(
    engine: &mut Engine,
    spec: &ArgumentSpec,
    out: &mut Execution,
) -> Result<Value> {
    let dims = match spec.dims.as_deref() {
        None => vec![0, 0],
        Some(dims) => crate::array::normalize_dims(
            &dims
                .iter()
                .map(|dim| match dim {
                    ArgDim::Fixed(size) => *size,
                    ArgDim::Colon => 0,
                })
                .collect::<Vec<_>>(),
        )?,
    };
    let rows = dims[0];
    let cols = super::nd::shape_product(&dims[1..])?;
    let count = Matrix::checked_len(rows, cols)?;
    let class = spec.class_name.as_deref().unwrap_or("double");
    let sizes = || {
        dims.iter()
            .map(|&n| Value::Matrix(Matrix::scalar(n as f64)))
            .collect::<Vec<_>>()
    };
    engine.check_cancel()?;
    if class == "double" {
        let value = engine
            .shape_call("zeros", &sizes())?
            .expect("zeros constructor");
        engine.charge(count)?;
        return Ok(value);
    }
    if let Some(numeric) = crate::NumericClass::parse(class) {
        engine.charge(count.saturating_mul(numeric.size()).div_ceil(8))?;
        let zero = crate::TypedArray::scalar_int(numeric, 0);
        return Ok(Value::from(Array::Typed(
            zero.remap(rows, cols, |_| None, &engine.cancel)?
                .with_dims(&dims)?,
        )));
    }
    let array = match class {
        "logical" => {
            engine.charge(count.div_ceil(8))?;
            Array::Logical(
                LogicalMatrix::filled(rows, cols, false, &engine.cancel)?.with_dims(&dims)?,
            )
        }
        "char" => {
            engine.charge(count.div_ceil(4))?;
            Array::Character(
                CharacterRow::from_column_major(
                    rows,
                    cols,
                    vec![b' ' as u16; CharacterRow::checked_len(rows, cols)?],
                    &engine.cancel,
                )?
                .with_dims(&dims)?,
            )
        }
        "cell" | "struct" | "string" if dims.len() != 2 => {
            return Err(format!(
                "N-D implicit defaults are not supported for class '{class}'."
            ));
        }
        "cell" => Array::Cell(CellArray::empty(rows, cols, &engine.cancel)?),
        "struct" => Array::Struct(StructArray::from_slots(
            rows,
            cols,
            Vec::new(),
            Vec::new(),
            &engine.cancel,
        )?),
        "string" => Array::Str(crate::StringArray::new(
            rows,
            cols,
            vec![Some(Arc::from("")); crate::StringArray::checked_len(rows, cols)?],
        )?),
        "datetime" if count == 0 && dims.len() == 2 => {
            return engine.profiler_builtin_call("NaT", sizes(), Vec::new(), out, false);
        }
        "duration" | "calendarDuration" | "categorical" if count == 0 && dims.len() == 2 => {
            match class {
                "duration" => Array::Duration(crate::DurationArray::new(
                    rows,
                    cols,
                    Vec::new(),
                    "hh:mm:ss",
                )?),
                "calendarDuration" => {
                    Array::Duration(crate::DurationArray::new_calendar(rows, cols, Vec::new())?)
                }
                _ => Array::Categorical(crate::CategoricalArray::new(
                    rows,
                    cols,
                    Vec::new(),
                    Arc::default(),
                    false,
                )?),
            }
        }
        "table" | "timetable" if count == 0 => return engine.call(class, Vec::new(), out),
        "function_handle" => {
            return Err(
                "An implicit function_handle default is unsupported. Specify a default value."
                    .into(),
            );
        }
        _ if count == 0 && dims.len() == 2 => {
            if let Some((canonical, _)) = super::ui_components::ui_kind_for_class_name(class) {
                Array::Graphics(crate::GraphicsArray::empty_typed(
                    rows,
                    cols,
                    canonical,
                    &engine.cancel,
                )?)
            } else if engine.cls_lookup_in(class, false, out)?.is_some() {
                Array::Struct(StructArray::empty_object(
                    rows,
                    cols,
                    class.to_owned(),
                    &engine.cancel,
                )?)
            } else {
                return Err(format!(
                    "An empty implicit default for class '{class}' is unsupported. Specify a default value."
                ));
            }
        }
        _ if count == 0 => {
            return Err(format!(
                "An empty implicit default for class '{class}' is unsupported. Specify a default value."
            ));
        }
        _ => return engine.call(class, Vec::new(), out),
    };
    Ok(Value::from(array))
}
fn generated(
    name: &str,
    outputs: Vec<String>,
    body: Vec<Statement>,
    span: std::ops::Range<usize>,
) -> Statement {
    Statement {
        kind: StatementKind::Function(Box::new(FunctionDefinition {
            name: name.to_owned(),
            inputs: Vec::new(),
            outputs,
            arguments: Vec::new(),
            body,
            explicit_end: true,
        })),
        span,
    }
}

/// Parse a class file and lay out its program (see the module comment).
/// Superclasses load first: a plain name from the function folders, a dotted
/// name `pkg.A` / `a.b.A` from that package (`+pkg/A.m` or `+pkg/@A/A.m`).
pub(super) fn class_program(
    engine: &mut Engine,
    name: &str,
    package: &str,
    path: &Path,
    source: &str,
) -> Result<Arc<Program>> {
    let file = path.display().to_string();
    let definition = syntax::parse_class_file(source).map_err(|e| {
        if let syntax::ClassFileError::Syntax(d) = &e {
            engine.file_diagnostic = Some(super::FileDiagnostic {
                path: path.to_owned(),
                source: Arc::from(source),
                diagnostic: d.clone(),
            });
        }
        e.message(&file, source)
    })?;
    if definition.name != name {
        return Err(format!(
            "{file}: the classdef name '{}' must match the file name '{name}'",
            definition.name
        ));
    }
    engine.functions.classes.seen = true;
    let ClassDefinition {
        name: constructor_name,
        attributes,
        superclasses,
        properties: property_blocks,
        methods: method_blocks,
        events,
        enumeration,
        functions,
        span,
    } = definition;
    // A class of package `a.b` is the class `a.b.Name`; its constructor
    // keeps the plain name.
    let class_name = if package.is_empty() {
        constructor_name.clone()
    } else {
        format!("{package}.{constructor_name}")
    };
    // The class folder `@Name` this file is the class file of, if any.
    let class_folder = super::user_functions::cls_folder(path, name).map(Path::to_path_buf);
    let line = |byte: usize| syntax::Diagnostic::new(source, byte, String::new()).line;
    let mut statements = Vec::new();
    let mut methods = BTreeMap::new();
    // Byte of each method the classdef defines or declares Abstract.
    let mut method_bytes = BTreeMap::new();
    // Methods declared without a body (their files are in the class folder):
    // Static or not, and the access.
    let mut declared = BTreeMap::new();
    let mut getters = BTreeMap::new();
    let mut setters = BTreeMap::new();
    let mut constructor = None;
    for block in method_blocks {
        for (signature, byte) in block.signatures {
            if !block.attributes.is_abstract {
                if class_folder.is_none() {
                    return Err(format!(
                        "{file}: method declarations without a body are not supported yet (line {}); a method in its own file needs the class folder @{constructor_name} holding {constructor_name}.m",
                        line(byte)
                    ));
                }
                if signature == constructor_name {
                    return Err(format!(
                        "{file}: the constructor {class_name} must be defined, with its body, in the classdef file (line {})",
                        line(byte)
                    ));
                }
                declared.insert(
                    signature,
                    (block.attributes.is_static, block.attributes.access),
                );
                continue;
            }
            if signature == constructor_name {
                return Err(format!(
                    "{file}: the constructor {class_name} cannot be Abstract"
                ));
            }
            method_bytes.insert(signature.clone(), byte);
            methods.insert(
                signature,
                MethodInfo {
                    index: usize::MAX,
                    is_static: block.attributes.is_static,
                    access: block.attributes.access,
                    is_abstract: true,
                    owner: class_name.clone(),
                    program: None,
                    file: None,
                },
            );
        }
        for statement in block.methods {
            let StatementKind::Function(f) = &statement.kind else {
                unreachable!()
            };
            let index = statements.len();
            let accessor = f
                .name
                .strip_prefix("get.")
                .map(|p| (true, p))
                .or_else(|| f.name.strip_prefix("set.").map(|p| (false, p)));
            if let Some((get, property)) = accessor {
                let (inputs, outputs) = (f.inputs.len(), f.outputs.len());
                if get && (inputs != 1 || outputs != 1) {
                    return Err(format!(
                        "{file}: the get method {} must take the object and return one value",
                        f.name
                    ));
                }
                if !get && (inputs != 2 || outputs > 1) {
                    return Err(format!(
                        "{file}: the set method {} must take the object and the new value",
                        f.name
                    ));
                }
                let table = if get { &mut getters } else { &mut setters };
                table.insert(property.to_owned(), (index, statement.span.start));
                statements.push(statement);
                continue;
            }
            if f.name == constructor_name {
                if block.attributes.is_static {
                    return Err(format!(
                        "{file}: the constructor {class_name} cannot be Static"
                    ));
                }
                if f.outputs.len() != 1 {
                    return Err(format!(
                        "{file}: the constructor {class_name} must return exactly one output, the object"
                    ));
                }
                constructor = Some(index);
            }
            method_bytes.insert(f.name.clone(), statement.span.start);
            methods.insert(
                f.name.clone(),
                MethodInfo {
                    index,
                    is_static: block.attributes.is_static,
                    access: block.attributes.access,
                    is_abstract: false,
                    owner: class_name.clone(),
                    program: None,
                    file: None,
                },
            );
            statements.push(statement);
        }
    }
    let constructor = constructor.unwrap_or_else(|| {
        statements.push(generated(
            &constructor_name,
            vec!["obj".into()],
            Vec::new(),
            span.clone(),
        ));
        statements.len() - 1
    });
    let mut properties = Vec::new();
    for block in property_blocks {
        for property in block.properties {
            let default = property.default.map(|expr| {
                let body = vec![Statement {
                    kind: StatementKind::Evaluate(Evaluation {
                        targets: vec![Some(DEFAULT_OUTPUT.into())],
                        indexed_target: None,
                        cell_content_target: false,
                        path_target: None,
                        list_target: None,
                        expr,
                        quiet: true,
                    }),
                    span: property.span.clone(),
                }];
                statements.push(generated(
                    &class_name,
                    vec![DEFAULT_OUTPUT.into()],
                    body,
                    property.span.clone(),
                ));
                statements.len() - 1
            });
            let attributes = &block.attributes;
            let getter = getters.remove(&property.name).map(|(i, _)| i);
            let setter = setters.remove(&property.name).map(|(i, _)| i);
            if attributes.constant && (getter.is_some() || setter.is_some()) {
                return Err(format!(
                    "{file}: the Constant property '{}' cannot have get or set methods",
                    property.name
                ));
            }
            properties.push(PropertyInfo {
                name: property.name,
                get: attributes.get_access,
                set: attributes.set_access,
                constant: attributes.constant,
                dependent: attributes.dependent,
                hidden: attributes.hidden,
                immutable: attributes.immutable,
                default,
                validation: property.validation,
                getter,
                setter,
                owner: class_name.clone(),
                inherited: None,
            });
        }
    }
    if let Some((property, (_, byte))) = getters.iter().chain(setters.iter()).next() {
        return Err(format!(
            "{file}: a get/set method names '{property}', which is not a property of class '{class_name}' (line {})",
            line(*byte)
        ));
    }
    // Every other function file of the class folder is a method: public and
    // not Static unless the classdef declares it. Its file is read when the
    // method is first called.
    if let Some(folder) = &class_folder {
        let files = folder_methods(engine, folder, &constructor_name)?;
        for (method, at) in &files {
            if let Some(byte) = method_bytes.get(method) {
                return Err(format!(
                    "{file}: the method '{method}' of class '{class_name}' is defined twice, in this classdef file (line {}) and in the separate file {}; keep one of them",
                    line(*byte),
                    at.display()
                ));
            }
            if properties.iter().any(|p: &PropertyInfo| p.name == *method) {
                return Err(format!(
                    "{file}: '{method}' is a property of class '{class_name}' and also its method file {}",
                    at.display()
                ));
            }
        }
        // A declared method without a file stays a method: calling it
        // reports the missing file.
        let names: BTreeSet<String> = declared.keys().cloned().chain(files.into_keys()).collect();
        for method in names {
            let (is_static, access) = declared
                .get(&method)
                .copied()
                .unwrap_or((false, Access::Public));
            let file = Arc::new(MethodFile {
                folder: folder.clone(),
                name: method.clone(),
            });
            methods.insert(
                method,
                MethodInfo {
                    index: usize::MAX,
                    is_static,
                    access,
                    is_abstract: false,
                    owner: class_name.clone(),
                    program: None,
                    file: Some(file),
                },
            );
        }
    }
    let inside = statements.len();
    let mut names = BTreeMap::new();
    for statement in functions {
        if let StatementKind::Function(f) = &statement.kind {
            names.insert(f.name.clone(), statements.len());
        }
        statements.push(statement);
    }
    let explicit_supers = superclasses
        .iter()
        .filter(|(base, _)| {
            statements.get(constructor).is_some_and(|c| match &c.kind {
                StatementKind::Function(f) => inherit::calls_superclass(f, base),
                _ => false,
            })
        })
        .map(|(base, _)| base.clone())
        .collect();
    let mut class = ClassInfo {
        name: class_name,
        path: path.to_owned(),
        properties,
        methods,
        ambiguous: BTreeMap::new(),
        constructor,
        inside,
        supers: superclasses.iter().map(|(n, _)| n.clone()).collect(),
        super_programs: Vec::new(),
        ancestors: Vec::new(),
        is_handle: false,
        sealed: attributes.iter().any(|a| a == "Sealed"),
        explicit_supers,
        events: events.into_iter().map(|(n, _)| n).collect(),
        enumeration: enumeration.into_iter().map(|m| (m.name, m.args)).collect(),
    };
    inherit::merge_superclasses(engine, &mut class, &superclasses, &file, source)?;
    if !class.is_handle && !class.events.is_empty() {
        return Err(format!(
            "{file}: only handle classes can define events; use classdef {} < handle",
            class.name
        ));
    }
    Ok(Arc::new(Program::class_file(
        Arc::from(source),
        statements,
        path.to_owned(),
        names,
        class,
    )))
}

impl Engine {
    /// The class file `name` of the lookup folder, if it is one. Using a class
    /// loads it: every property default is evaluated once per run.
    fn cls_lookup(&mut self, name: &str, out: &mut Execution) -> Result<Option<Arc<Program>>> {
        self.cls_lookup_in(name, true, out)
    }
    /// `cls_lookup`; `imports` false for the class name stored in an existing
    /// object: an import made later never changes what that object is.
    pub(super) fn cls_lookup_in(
        &mut self,
        name: &str,
        imports: bool,
        out: &mut Execution,
    ) -> Result<Option<Arc<Program>>> {
        // A class of a package is looked up by its qualified name `a.Name`.
        if !crate::files::valid_name(name) && !super::user_functions::pkg_name(name) {
            return Ok(None);
        }
        let program = if imports {
            self.folder_program(name)?
        } else {
            self.cls_stored_program(name)?
        };
        let Some(program) = program.filter(|p| p.class.is_some()) else {
            return Ok(None);
        };
        self.cls_ensure_loaded(&program, out)?;
        Ok(Some(program))
    }
    /// Evaluate all property defaults of a class, in declaration order, the
    /// first time it is loaded in a run (later calls, and calls made by a
    /// default itself, return at once). A failing default leaves the class
    /// not loaded; defaults that already ran keep their values.
    pub(super) fn cls_ensure_loaded(
        &mut self,
        program: &Arc<Program>,
        out: &mut Execution,
    ) -> Result<()> {
        let class = program.class.as_ref().expect("class program");
        if !self.functions.classes.started.insert(class.path.clone()) {
            return Ok(());
        }
        self.functions
            .classes
            .known
            .insert(class.name.clone(), class.clone());
        for i in 0..class.properties.len() {
            if let Err(error) = self.cls_default(program, i, out) {
                self.functions.classes.started.remove(&class.path);
                return Err(error);
            }
        }
        Ok(())
    }
    /// The loaded class of an object, reloading its file in a new run.
    fn cls_require(
        &mut self,
        class: &str,
        out: &mut Execution,
    ) -> Result<(Arc<Program>, Arc<ClassInfo>)> {
        let program = self.cls_lookup_in(class, false, out)?.ok_or_else(|| {
            format!("The class definition file of '{class}' is not in the current folder.")
        })?;
        let info = program.class.clone().expect("class program");
        Ok((program, info))
    }
    /// The class whose method (or default value) is running.
    fn cls_calling_class(&self) -> Option<&ClassInfo> {
        let (program, current) = self.functions.cls_frame()?;
        // A method file of a class folder: its first function is the method.
        if let Some(class) = &program.method_of {
            return (current.is_some() && current == program.primary).then_some(class.as_ref());
        }
        let class = program.class.as_ref()?;
        (current? < class.inside).then_some(class.as_ref())
    }
    /// The program and statement that run `method` of the class file
    /// `program` (for an inherited method: of its defining class). A method
    /// kept in its own file of the class folder is read on its first call in
    /// a run.
    fn cls_target(
        &mut self,
        program: &Arc<Program>,
        method: &MethodInfo,
    ) -> Result<(Arc<Program>, usize)> {
        let owner = method.target(program);
        let Some(file) = &method.file else {
            return Ok((owner, method.index));
        };
        let class = owner.class.clone().expect("class program");
        let target = self
            .cls_method_program(&class, &file.folder, &file.name)?
            .ok_or_else(|| {
                format!(
                    "The method '{}' of class '{}' is declared without a body in {}, but the class folder has no file {}.m.",
                    file.name,
                    class.name,
                    class.path.display(),
                    file.name
                )
            })?;
        let index = target.primary.expect("method file");
        Ok((target, index))
    }
    /// Run `f` with private/protected access granted to `class_name` while no
    /// other class method is on top of the call stack (used for `AppBase`
    /// `createCallbackFcn` and `runStartupFcn` dispatch).
    pub(super) fn cls_with_privilege<T>(
        &mut self,
        class_name: Option<String>,
        f: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let pushed = if let Some(name) = class_name {
            self.functions.classes.privileged_classes.push(name);
            true
        } else {
            false
        };
        let result = f(self);
        if pushed {
            self.functions.classes.privileged_classes.pop();
        }
        result
    }
    fn cls_is_privileged(&self, owner: &str, protected: bool) -> bool {
        if self.cls_calling_class().is_some() {
            return false;
        }
        let Some(priv_class) = self.functions.classes.privileged_classes.last() else {
            return false;
        };
        if priv_class == owner {
            return true;
        }
        protected
            && self
                .functions
                .classes
                .known
                .get(priv_class)
                .is_some_and(|c| c.is_a(owner))
    }
    /// May the running code use a member of class `owner` with `access`?
    /// Private: methods of `owner`; protected: also of its subclasses.
    fn cls_allowed(&self, owner: &str, access: Access) -> bool {
        match access {
            Access::Public => true,
            Access::Private => {
                self.cls_calling_class().is_some_and(|c| c.name == owner)
                    || self.cls_is_privileged(owner, false)
            }
            Access::Protected => {
                self.cls_calling_class().is_some_and(|c| c.is_a(owner))
                    || self.cls_is_privileged(owner, true)
            }
        }
    }
    /// The constructor of the defining class is running (immutable properties).
    fn cls_in_constructor(&self, owner: &str) -> bool {
        self.functions
            .cls_frame()
            .and_then(|(program, current)| Some((program.class.as_ref()?, current?)))
            .is_some_and(|(class, current)| class.name == owner && current == class.constructor)
    }
    /// The get or set method of `property` is running: there the property
    /// reads and writes its storage (no recursive get/set calls).
    fn cls_in_accessor(&self, property: &PropertyInfo, own: &Arc<Program>) -> bool {
        let Some((program, Some(current))) = self.functions.cls_frame() else {
            return false;
        };
        let owner = property.inherited.as_ref().map_or(own, |(p, _)| p);
        Arc::ptr_eq(program, owner)
            && (property.getter == Some(current) || property.setter == Some(current))
    }
    /// `Name(...)` from outside the class needs a public constructor.
    pub(super) fn cls_constructor_access(&self, class: &ClassInfo) -> Result<()> {
        match class.methods.get(class.constructor_name()) {
            Some(constructor) if !self.cls_allowed(&class.name, constructor.access) => {
                Err(format!(
                    "Cannot access {} method '{}' of class '{}'.",
                    constructor.access.name(),
                    class.name,
                    class.name
                ))
            }
            _ => Ok(()),
        }
    }
    fn cls_invoke(
        &mut self,
        program: &Arc<Program>,
        index: usize,
        args: Vec<Value>,
        requested: usize,
        targets: Option<&[Option<String>]>,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        // A method without outputs called for a value runs for effect.
        let no_outputs = program
            .function(index)
            .is_some_and(|f| f.outputs.is_empty());
        let requested = if requested == 1 && no_outputs {
            0
        } else {
            requested
        };
        // `h.delete()` / `delete(h)` with the class's own delete method:
        // the handle deletion, which runs that method.
        if program.function(index).is_some_and(|f| f.name == "delete")
            && args
                .first()
                .is_some_and(|a| matches!(a, Value::Container(a) if handles::is_handle_object(a)))
        {
            self.cls_delete(&args[0], out)?;
            return Ok(vec![Value::Void]);
        }
        let mut values = self.invoke_function(
            ResolvedFunction::at(program.clone(), index),
            args,
            requested,
            targets,
            out,
        )?;
        self.charge_function_outputs(&values)?;
        if values.is_empty() && requested <= 1 {
            values.push(Value::Void);
        }
        Ok(values)
    }
    /// Default (or Constant) value of property `i`, evaluated once per run.
    fn cls_default(
        &mut self,
        program: &Arc<Program>,
        i: usize,
        out: &mut Execution,
    ) -> Result<Array> {
        let class = program.class.as_ref().expect("class program");
        let count = class.properties.len();
        let slot = self
            .functions
            .classes
            .values
            .entry(class.path.clone())
            .or_default();
        if slot.len() != count {
            *slot = vec![None; count];
        }
        if let Some(value) = &slot[i] {
            return Ok(value.clone());
        }
        let property = &class.properties[i];
        if let Some((owner, j)) = &property.inherited {
            // Evaluated once, in (and cached for) the defining class.
            let (owner, j) = (owner.clone(), *j);
            return self.cls_default(&owner, j, out);
        }
        let key = (class.path.clone(), i);
        if !self.functions.classes.loading.insert(key.clone()) {
            return Err(format!(
                "The default value of property '{}' of class '{}' depends on itself.",
                property.name, class.name
            ));
        }
        let validation = property.validation.as_ref();
        let result = (|| {
            let default = if let Some(index) = property.default {
                self.invoke_function(
                    ResolvedFunction::at(program.clone(), index),
                    Vec::new(),
                    1,
                    None,
                    out,
                )?
                .remove(0)
            } else if let Some(spec) = validation.filter(|_| !property.dependent) {
                cls_implicit_default(self, spec, out)?
            } else {
                return Ok(Array::Numeric(Matrix::filled(0, 0, 0.)?));
            };
            if let Some(spec) = validation {
                super::arguments::validate_property(self, spec, default, &class.name, out)?
                    .array(&self.cancel)
            } else {
                default.array(&self.cancel)
            }
        })();
        self.functions.classes.loading.remove(&key);
        let value = result?;
        if let Some(slot) = self.functions.classes.values.get_mut(&class.path)
            && slot.len() == count
        {
            slot[i] = Some(value.clone());
        }
        Ok(value)
    }
    /// The object a constructor starts from: every default evaluated (once per
    /// run, in declaration order), instance properties stored (a handle class
    /// stores them in a new handle object).
    pub(super) fn cls_default_object(
        &mut self,
        program: &Arc<Program>,
        out: &mut Execution,
    ) -> Result<Array> {
        let class = program.class.clone().expect("class program");
        self.cls_ensure_loaded(program, out)?;
        let mut names = Vec::new();
        let mut slots = Vec::new();
        for (i, property) in class.properties.iter().enumerate() {
            let value = self.cls_default(program, i, out)?;
            if !property.constant && !property.dependent {
                names.push(property.name.clone());
                slots.push(Arc::new(value));
            }
        }
        names.push(super::lang_more::TAG.into());
        slots.push(Arc::new(Array::Numeric(Matrix::scalar(USER_TAG))));
        names.push(CLASS_FIELD.into());
        slots.push(Arc::new(Array::Character(CharacterRow::from_text(
            &class.name,
            &self.cancel,
        )?)));
        let storage = Array::Struct(StructArray::from_slots(1, 1, names, slots, &self.cancel)?);
        if class.is_handle {
            return self.cls_new_handle(&class.name, storage);
        }
        Ok(storage)
    }
    /// The stored value of property `i` (no get method): Constant values and
    /// properties added to the file after the object was made read the default.
    fn cls_property_value(
        &mut self,
        program: &Arc<Program>,
        object: &Array,
        i: usize,
        out: &mut Execution,
    ) -> Result<Array> {
        let class = program.class.as_ref().expect("class program");
        let property = &class.properties[i];
        if property.dependent {
            return Err(format!(
                "The Dependent property '{}' of class '{}' has no stored value.",
                property.name, property.owner
            ));
        }
        if !property.constant {
            let storage = self.cls_storage(object)?;
            if let Array::Struct(s) = storage.as_ref()
                && let Some(field) = s.field_index(&property.name)
            {
                return Ok(s.get(field, 0)?.as_ref().clone());
            }
        }
        // Constant, or a property added to the file after the object was made.
        self.cls_default(program, i, out)
    }
    /// Property `i` as code outside its get method sees it: the get method's
    /// result when there is one, else the stored value. A Dependent property
    /// without a get method cannot be read.
    fn cls_read(
        &mut self,
        program: &Arc<Program>,
        object: &Array,
        i: usize,
        out: &mut Execution,
    ) -> Result<Array> {
        let class = program.class.clone().expect("class program");
        let property = &class.properties[i];
        if let Some(getter) = property.getter
            && !self.cls_in_accessor(property, program)
        {
            let target = property.program(program);
            let mut values = self.cls_invoke(
                &target,
                getter,
                vec![Value::from(object.clone())],
                1,
                None,
                out,
            )?;
            return values.remove(0).array(&self.cancel);
        }
        if property.dependent && property.getter.is_none() {
            return Err(format!(
                "In class '{}', no get method is defined for Dependent property '{}'. A Dependent property needs a get method to access its value.",
                property.owner, property.name
            ));
        }
        self.cls_property_value(program, object, i, out)
    }
    /// The method `member` of `class` for an object or a class name, with the
    /// ambiguity, abstract and access errors.
    fn cls_method<'a>(&self, class: &'a ClassInfo, member: &str) -> Result<Option<&'a MethodInfo>> {
        if let Some((a, b)) = class.ambiguous.get(member) {
            return Err(ambiguous_message(&class.name, member, a, b));
        }
        let Some(method) = class.methods.get(member) else {
            return Ok(None);
        };
        if method.is_abstract {
            return Err(abstract_call_message(&class.name, member));
        }
        if !self.cls_allowed(&method.owner, method.access) {
            return Err(format!(
                "Cannot access {} method '{member}' of class '{}'.",
                method.access.name(),
                class.name
            ));
        }
        Ok(Some(method))
    }

    /// `obj.member` (args None) and `obj.member(args)` on a user object:
    /// property first, then method. None for any other value.
    pub(super) fn cls_member(
        &mut self,
        base: &Value,
        member: &str,
        args: Option<&[Expr]>,
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        let Value::Container(object) = base else {
            return Ok(None);
        };
        let Some(class_name) = object_class(object) else {
            return Ok(None);
        };
        if object.is_empty() {
            return Err(format!(
                "Dot indexing is not supported for empty objects of class '{class_name}'."
            ));
        }
        let (program, class) = self.cls_require(&class_name, out)?;
        if let Some(i) = class.property(member) {
            let property = &class.properties[i];
            if !self.cls_allowed(&property.owner, property.get) {
                return Err(format!(
                    "Cannot access {} property '{member}' of class '{class_name}'.",
                    property.get.name()
                ));
            }
            let value = Value::from(self.cls_read(&program, object, i, out)?);
            return match args {
                None => Ok(Some(value)),
                Some(args) => self.index_value(value, args, out).map(Some),
            };
        }
        if let Some(method) = self.cls_method(&class, member)? {
            let mut values = if method.is_static {
                Vec::new()
            } else {
                vec![base.clone()]
            };
            let (target, index) = self.cls_target(&program, method)?;
            if let Some(args) = args {
                values.extend(self.function_arguments(args, None, out)?);
            }
            let mut result = self.cls_invoke(&target, index, values, 1, None, out)?;
            return Ok(Some(result.remove(0)));
        }
        if (class.is_handle && HANDLE_METHODS.contains(&member))
            || (class.is_a(APPBASE_CLASS) && APPBASE_METHODS.contains(&member))
        {
            let mut values = vec![base.clone()];
            if let Some(args) = args {
                values.extend(self.function_arguments(args, None, out)?);
            }
            let mut result = self
                .cls_call(member, &values, 1, None, out)?
                .expect("handle or AppBase method");
            return Ok(Some(result.remove(0)));
        }
        Err(format!(
            "Unrecognized method, property, or field '{member}' for class '{class_name}'."
        ))
    }
    /// `Name.Const`, `Name.Const(i)` and `Name.staticMethod(args)` where
    /// `Name` is a class file and not a variable.
    pub(super) fn cls_static(
        &mut self,
        inner: &Expr,
        member: &str,
        args: Option<&[Expr]>,
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        let Expr::Name(name) = inner else {
            return Ok(None);
        };
        if self.contains_array(name) || name == "end" {
            return Ok(None);
        }
        let Some(program) = self.cls_lookup(name, out)? else {
            return Ok(None);
        };
        let class = program.class.clone().expect("class program");
        if let Some(i) = class.property(member)
            && class.properties[i].constant
        {
            let property = &class.properties[i];
            if !self.cls_allowed(&property.owner, property.get) {
                return Err(format!(
                    "Cannot access {} property '{member}' of class '{name}'.",
                    property.get.name()
                ));
            }
            let value = Value::from(self.cls_default(&program, i, out)?);
            return match args {
                None => Ok(Some(value)),
                Some(args) => self.index_value(value, args, out).map(Some),
            };
        }
        if class.methods.get(member).is_some_and(|m| m.is_static)
            && let Some(method) = self.cls_method(&class, member)?
        {
            let (target, index) = self.cls_target(&program, method)?;
            let values = match args {
                Some(args) => self.function_arguments(args, None, out)?,
                None => Vec::new(),
            };
            return Ok(Some(
                self.cls_invoke(&target, index, values, 1, None, out)?
                    .remove(0),
            ));
        }
        if let Some(value) = self.cls_enum_member(&program, member, out)? {
            return match args {
                None => Ok(Some(value)),
                Some(args) => self.index_value(value, args, out).map(Some),
            };
        }
        Err(format!(
            "The class {name} has no Constant property or Static method named '{member}'."
        ))
    }
    /// Index of property `member` if code running now may assign it.
    fn cls_settable(&self, class: &ClassInfo, member: &str) -> Result<usize> {
        let Some(i) = class.property(member) else {
            return Err(format!(
                "Unrecognized property '{member}' for class '{}'.",
                class.name
            ));
        };
        let property = &class.properties[i];
        if property.constant {
            return Err(format!(
                "Cannot set Constant property '{member}' of class '{}'.",
                class.name
            ));
        }
        if property.immutable {
            if !self.cls_in_constructor(&property.owner) {
                return Err(format!(
                    "Cannot set immutable property '{member}' of class '{}' outside the constructor of '{}'.",
                    class.name, property.owner
                ));
            }
        } else if !self.cls_allowed(&property.owner, property.set) {
            return Err(format!(
                "Cannot set {} property '{member}' of class '{}'.",
                property.set.name(),
                class.name
            ));
        }
        if property.dependent && property.setter.is_none() {
            return Err(format!(
                "In class '{}', no set method is defined for Dependent property '{member}'. A Dependent property needs a set method to assign its value.",
                property.owner
            ));
        }
        Ok(i)
    }
    /// Index of property `member` if code running now may read it.
    fn cls_readable(&self, class: &ClassInfo, member: &str) -> Result<usize> {
        let Some(i) = class.property(member) else {
            return Err(format!(
                "Unrecognized property '{member}' for class '{}'.",
                class.name
            ));
        };
        let property = &class.properties[i];
        if !self.cls_allowed(&property.owner, property.get) {
            return Err(format!(
                "Cannot access {} property '{member}' of class '{}'.",
                property.get.name(),
                class.name
            ));
        }
        Ok(i)
    }
    /// `[obj.p, ...] = ...`: the same property check as `obj.p = v`; any
    /// other value passes.
    pub(super) fn cls_check_settable(
        &mut self,
        object: &Array,
        member: &str,
        out: &mut Execution,
    ) -> Result<()> {
        let Some(class_name) = object_class(object) else {
            return Ok(());
        };
        let (_, class) = self.cls_require(&class_name, out)?;
        self.cls_settable(&class, member).map(|_| ())
    }
    /// Assignment below a user object (`obj.p = v`, `obj.p(i) = v`,
    /// `obj.p.f = v`): checks the property, then runs its set method (outside
    /// that method) or assigns the storage: a copy of a value object, the
    /// shared properties of a handle (returned unchanged).
    pub(super) fn cls_assign(
        &mut self,
        object: Array,
        steps: &[PathStep],
        value: Array,
        deletion: bool,
        out: &mut Execution,
    ) -> Result<Array> {
        let class_name = object_class(&object).expect("user object");
        let member = match &steps[0] {
            PathStep::Field(name) => name.clone(),
            PathStep::Dynamic(expr) => self.dynamic_field_name(expr, out)?,
            PathStep::Paren(_) => return Err(OBJECT_ARRAYS.into()),
            PathStep::Brace(_) => {
                return Err(format!(
                    "Brace indexing is not supported for objects of class '{class_name}'."
                ));
            }
        };
        if object.is_empty() {
            return Err(format!(
                "Dot indexing is not supported for empty objects of class '{class_name}'."
            ));
        }
        let (program, class) = self.cls_require(&class_name, out)?;
        let i = self.cls_settable(&class, &member)?;
        let property = class.properties[i].clone();
        let current = if steps.len() > 1 {
            Some(self.cls_read(&program, &object, i, out)?)
        } else {
            None
        };
        let child = self.assign_step(current, &steps[1..], value, deletion, out)?;
        let child = if let Some(spec) = &property.validation {
            super::arguments::validate_property(self, spec, Value::from(child), &class_name, out)?
                .array(&self.cancel)?
        } else {
            child
        };
        if let Some(setter) = property.setter
            && !self.cls_in_accessor(&property, &program)
        {
            let target = property.program(&program);
            let args = vec![Value::from(object.clone()), Value::from(child)];
            let mut result = self.cls_invoke(&target, setter, args, 1, None, out)?;
            if class.is_handle {
                return Ok(object);
            }
            return match result.remove(0) {
                Value::Container(updated)
                    if object_class(&updated).as_deref() == Some(&*class_name) =>
                {
                    Ok(updated)
                }
                _ => Err(format!(
                    "The set method set.{member} of class '{}' must return the modified object.",
                    property.owner
                )),
            };
        }
        if class.is_handle {
            let Array::Struct(s) = self.cls_storage(&object)?.into_owned() else {
                unreachable!("handle storage")
            };
            let storage = Array::Struct(s.with_field(&member, child, &self.cancel)?);
            self.cls_store(&object, storage)?;
            return Ok(object);
        }
        let Array::Struct(s) = object else {
            unreachable!()
        };
        Ok(Array::Struct(s.with_field(&member, child, &self.cancel)?))
    }

    /// Calls whose first argument is a user object: a method of its class
    /// wins over every function and builtin; then the object forms of the
    /// class/property/method queries. `methods('Name')`, `properties('Name')`
    /// and `isprop` also answer here. None leaves the call unchanged.
    pub(super) fn cls_call(
        &mut self,
        name: &str,
        args: &[Value],
        requested: usize,
        targets: Option<&[Option<String>]>,
        out: &mut Execution,
    ) -> Result<Option<Vec<Value>>> {
        if name.contains('@') {
            return self
                .cls_super_call(name, args, requested, targets, out)
                .map(Some);
        }
        if let Some(values) = self.cls_event_call(name, args, requested, out)? {
            return Ok(Some(values));
        }
        let first = args.first();
        if !first.is_some_and(is_object_value) {
            // A user function of that name keeps precedence over these builtins.
            if matches!(
                name,
                "methods"
                    | "properties"
                    | "ismethod"
                    | "isprop"
                    | "superclasses"
                    | "handle"
                    | "registerApp"
                    | "runStartupFcn"
                    | "createCallbackFcn"
            ) && self.resolve_function(name)?.is_some()
            {
                return Ok(None);
            }
            return match name {
                "handle" => Err(
                    "Cannot create an object of the abstract class 'handle'; derive a class from it (classdef Name < handle)."
                        .into(),
                ),
                "registerApp" | "runStartupFcn" | "createCallbackFcn" => Err(format!(
                    "{name} requires a matlab.apps.AppBase subclass instance as its first argument."
                )),
                "superclasses" => {
                    let Some(class) = first.and_then(text) else {
                        return Err(if args.is_empty() {
                            "Not enough input arguments."
                        } else {
                            "superclasses: the input must be an object or a class name."
                        }
                        .into());
                    };
                    if args.len() > 1 {
                        return Err("Too many input arguments.".into());
                    }
                    if class == "handle" {
                        // The base of every handle class has no superclass.
                        if requested == 0 {
                            let text = if self.display.compact {
                                "No superclasses for class handle"
                            } else {
                                "\nNo superclasses for class handle\n"
                            };
                            self.emit(out, text.into())?;
                            return Ok(Some(vec![Value::Void]));
                        }
                        return Ok(Some(vec![self.cls_names_cell(Vec::new())?]));
                    }
                    if class == APPBASE_CLASS {
                        if requested == 0 {
                            let text = if self.display.compact {
                                "Superclasses for class matlab.apps.AppBase:\n    handle"
                            } else {
                                "\nSuperclasses for class matlab.apps.AppBase:\n\n    handle\n"
                            };
                            self.emit(out, text.into())?;
                            return Ok(Some(vec![Value::Void]));
                        }
                        return Ok(Some(vec![self.cls_names_cell(vec!["handle".into()])?]));
                    }
                    match self.cls_lookup(&class, out)? {
                        Some(program) => {
                            let info = program.class.clone().expect("class program");
                            Ok(Some(vec![self.cls_superclasses(&info, requested, out)?]))
                        }
                        None => Err(format!(
                            "superclasses: '{class}' is not a class file in the function folder."
                        )),
                    }
                }
                "eq" | "ne" if args.len() == 2 && args.iter().any(|a| matches!(a, Value::Container(a) if handles::is_handle_object(a) || events::enum_name(a).is_some())) => {
                    Ok(Some(vec![self.cls_identity(name, &args[0], &args[1])?]))
                }
                "methods" | "properties" | "ismethod" | "isprop"
                    if first.is_some_and(|v| text(v).is_some()) =>
                {
                    let class = text(first.unwrap()).unwrap();
                    match self.cls_lookup(&class, out)? {
                        Some(program) => self.cls_query(name, &program, args, requested, out),
                        None if name == "isprop" => Ok(Some(vec![self.cls_isprop_builtin(args)?])),
                        None if name == "methods" => {
                            self.cls_builtin_methods(&class, requested, out)
                        }
                        None => Ok(None),
                    }
                }
                "isprop" => Ok(Some(vec![self.cls_isprop_builtin(args)?])),
                "methods" if first.is_some() => {
                    let class =
                        super::lang_more::lng_class(&first.unwrap().clone().array(&self.cancel)?);
                    self.cls_builtin_methods(&class, requested, out)
                }
                "horzcat" | "vertcat" | "cat" | "repmat" if args.iter().any(is_object_value) => {
                    Err(OBJECT_ARRAYS.into())
                }
                _ => Ok(None),
            };
        }
        let Value::Container(object) = &args[0] else {
            unreachable!()
        };
        let class_name = object_class(object).expect("user object");
        let (program, class) = self.cls_require(&class_name, out)?;
        if name == "delete" && class.is_handle {
            if args.len() > 1 {
                return Err("Too many input arguments.".into());
            }
            // A delete method declared private or protected: from outside,
            // delete(h) errors as h.delete() does (cls_method).
            if let Some(method) = class.methods.get("delete")
                && !self.cls_allowed(&method.owner, method.access)
            {
                return Err(format!(
                    "Cannot access {} method 'delete' of class '{}'.",
                    method.access.name(),
                    class.name
                ));
            }
            if object.is_empty() {
                return Ok(Some(vec![Value::Void]));
            }
            self.cls_delete(&args[0], out)?;
            return Ok(Some(vec![Value::Void]));
        }
        if class.instance_method(name)?.is_some()
            && let Some(method) = self.cls_method(&class, name)?
        {
            let (target, index) = self.cls_target(&program, method)?;
            return self
                .cls_invoke(&target, index, args.to_vec(), requested, targets, out)
                .map(Some);
        }
        let arity = |min: usize, max: usize| -> Result<()> {
            if args.len() < min {
                Err("Not enough input arguments.".into())
            } else if args.len() > max {
                Err("Too many input arguments.".into())
            } else {
                Ok(())
            }
        };
        let value = match name {
            "class" => {
                arity(1, 1)?;
                Value::Text(class_name)
            }
            "char" | "string" | "disp" if events::enum_name(object).is_some() => {
                arity(1, 1)?;
                let member = events::enum_name(object).expect("enum");
                match name {
                    "char" => Value::Text(member),
                    "string" => Value::QuotedString(member),
                    _ => {
                        self.emit(out, format!("    {member}\n"))?;
                        Value::Void
                    }
                }
            }
            "isa" => {
                arity(2, 2)?;
                let wanted = text(&args[1]).ok_or("Second input must be a class name.")?;
                logical(class.is_a(&wanted))
            }
            "isvalid" | "delete" if !class.is_handle => {
                return Err(format!(
                    "Undefined function '{name}' for input arguments of type '{class_name}'."
                ));
            }
            "registerApp" | "runStartupFcn" | "createCallbackFcn" if !class.is_a(APPBASE_CLASS) => {
                return Err(format!(
                    "Undefined function '{name}' for input arguments of type '{class_name}'."
                ));
            }
            "registerApp" => {
                arity(2, 2)?;
                if object.is_empty() || !self.cls_is_valid(object) {
                    return Err(handles::DELETED.into());
                }
                super::ui_components::register_app(self, object.clone(), &args[1])?;
                Value::Void
            }
            "runStartupFcn" => {
                arity(2, 2)?;
                if object.is_empty() || !self.cls_is_valid(object) {
                    return Err(handles::DELETED.into());
                }
                let Value::Container(Array::Handle(fh)) = &args[1] else {
                    return Err(
                        "runStartupFcn requires a function handle as the second argument.".into(),
                    );
                };
                let fh = fh.clone();
                let run_res = self.cls_with_privilege(Some(class_name), |engine| {
                    engine.call_handle(&fh, vec![args[0].clone()], 0, None, out)
                });
                if let Err(err) = run_res {
                    let _ = self.cls_delete(&args[0], out);
                    return Err(err);
                }
                Value::Void
            }
            "createCallbackFcn" => {
                arity(2, 3)?;
                if object.is_empty() || !self.cls_is_valid(object) {
                    return Err(handles::DELETED.into());
                }
                let Value::Container(Array::Handle(fh)) = &args[1] else {
                    return Err(
                        "createCallbackFcn requires a function handle as the second argument."
                            .into(),
                    );
                };
                let requires_event = match args.get(2) {
                    None => false,
                    Some(Value::Logical(m)) if m.len() == 1 => m.data()[0] != 0,
                    Some(Value::Matrix(m)) if m.len() == 1 && m.data()[0].is_finite() => {
                        m.data()[0] != 0.0
                    }
                    _ => {
                        return Err(
                            "createCallbackFcn third argument must be a logical scalar.".into()
                        );
                    }
                };
                Value::Container(Array::Handle(crate::FunctionHandle::app_callback(
                    object.clone(),
                    fh.clone(),
                    requires_event,
                )))
            }
            "isvalid" => {
                arity(1, 1)?;
                if object.is_empty() {
                    Value::Logical(LogicalMatrix::filled(
                        object.rows(),
                        object.cols(),
                        false,
                        &self.cancel,
                    )?)
                } else {
                    logical(self.cls_is_valid(object))
                }
            }
            "eq" | "ne" => {
                arity(2, 2)?;
                let empty =
                    object.is_empty() || matches!(&args[1], Value::Container(b) if b.is_empty());
                if empty && cls_empty_identity_compatible(self, &args[0], &args[1]) {
                    let left = (object.rows(), object.cols());
                    let right = match &args[1] {
                        Value::Container(Array::Struct(s)) => (s.rows(), s.cols()),
                        // An enumeration member name compares as one value.
                        _ => (1, 1),
                    };
                    let (rows, cols) = if left == (1, 1) {
                        right
                    } else if right == (1, 1) || left == right {
                        left
                    } else {
                        return Err(
                            "Array dimensions must agree (only scalar expansion is supported)"
                                .into(),
                        );
                    };
                    Value::Logical(LogicalMatrix::filled(rows, cols, false, &self.cancel)?)
                } else {
                    self.cls_identity(name, &args[0], &args[1])?
                }
            }
            "superclasses" => {
                arity(1, 1)?;
                self.cls_superclasses(&class, requested, out)?
            }
            "isobject" => {
                arity(1, 1)?;
                logical(true)
            }
            "isstruct" => {
                arity(1, 1)?;
                logical(false)
            }
            // An object is not a struct: struct queries see no fields and the
            // struct editors follow the property rules or do not apply.
            "isfield" => {
                arity(2, 2)?;
                let (rows, cols) = match &args[1] {
                    Value::Container(Array::Cell(c)) => (c.rows(), c.cols()),
                    _ => (1, 1),
                };
                Value::Logical(LogicalMatrix::filled(rows, cols, false, &self.cancel)?)
            }
            "numfields" => {
                arity(1, 1)?;
                Value::Matrix(Matrix::scalar(0.))
            }
            "getfield" => {
                arity(2, usize::MAX)?;
                if object.is_empty() {
                    return Err(format!(
                        "Dot indexing is not supported for empty objects of class '{class_name}'."
                    ));
                }
                let member = text(&args[1])
                    .ok_or("Field names must be character vectors or string scalars.")?;
                let i = self.cls_readable(&class, &member)?;
                let value = self.cls_read(&program, object, i, out)?;
                if args.len() == 2 {
                    Value::from(value)
                } else {
                    // The rest of the path (indices, nested names) runs on a
                    // struct holding only this property.
                    let mut rest = vec![self.cls_field_struct(&member, value)?];
                    rest.extend_from_slice(&args[1..]);
                    self.call("getfield", rest, out)?
                }
            }
            "set" if class.is_handle => {
                if args.len() < 3 || !(args.len() - 1).is_multiple_of(2) {
                    return Err("set requires a handle object and name-value pairs.".into());
                }
                for pair in args[1..].as_chunks::<2>().0 {
                    let member = text(&pair[0])
                        .ok_or("Property names must be character vectors or string scalars.")?;
                    let value = pair[1].clone().array(&self.cancel)?;
                    let steps = [PathStep::Field(member)];
                    self.cls_assign(object.clone(), &steps, value, false, out)?;
                }
                Value::Void
            }
            "setfield" => {
                arity(3, usize::MAX)?;
                if object.is_empty() {
                    return Err(format!(
                        "Dot indexing is not supported for empty objects of class '{class_name}'."
                    ));
                }
                let member = text(&args[1])
                    .ok_or("Field names must be character vectors or string scalars.")?;
                let i = self.cls_settable(&class, &member)?;
                let last = args[args.len() - 1].clone();
                let new = if args.len() == 3 {
                    last
                } else {
                    let child = self.cls_read(&program, object, i, out)?;
                    let mut rest = vec![self.cls_field_struct(&member, child)?];
                    rest.extend_from_slice(&args[1..]);
                    match self.call("setfield", rest, out)?.array(&self.cancel)? {
                        Array::Struct(s) if s.field_index(&member).is_some() => {
                            Value::from(s.get(s.field_index(&member).unwrap(), 0)?.as_ref().clone())
                        }
                        _ => return Err("setfield: the field path could not be assigned.".into()),
                    }
                };
                let new = new.array(&self.cancel)?;
                let steps = [PathStep::Field(member)];
                Value::Container(self.cls_assign(object.clone(), &steps, new, false, out)?)
            }
            // `subsasgn(p, S, v)` assigns like the path S written out (p.x = v):
            // the same property rules, hidden fields unreachable.
            "subsasgn" => {
                arity(3, 3)?;
                let steps = self.cls_subs_steps(&args[1])?;
                if steps.is_empty() {
                    return Err("subsasgn: the subscript structure is empty.".into());
                }
                let value = args[2].clone().array(&self.cancel)?;
                Value::Container(self.cls_assign(object.clone(), &steps, value, false, out)?)
            }
            "rmfield" | "orderfields" | "struct2cell" => {
                return Err(format!(
                    "Undefined function '{name}' for input arguments of type '{class_name}'."
                ));
            }
            "fieldnames" => {
                arity(1, 1)?;
                self.cls_names_cell(class.public_properties())?
            }
            "methods" | "properties" | "ismethod" | "isprop" => {
                return self.cls_query(name, &program, args, requested, out);
            }
            "struct" => {
                arity(1, 1)?;
                if object.is_empty() {
                    Value::Container(Array::Struct(StructArray::from_slots(
                        object.rows(),
                        object.cols(),
                        Vec::new(),
                        Vec::new(),
                        &self.cancel,
                    )?))
                } else {
                    self.emit(
                        out,
                        format!(
                            "Warning: struct(obj) exposes every property of class '{class_name}', private ones included; use disp or properties for its public view."
                        ),
                    )?;
                    let mut names = Vec::new();
                    let mut slots = Vec::new();
                    for i in 0..class.properties.len() {
                        if class.properties[i].dependent {
                            continue;
                        }
                        names.push(class.properties[i].name.clone());
                        slots.push(Arc::new(self.cls_property_value(&program, object, i, out)?));
                    }
                    Value::Container(Array::Struct(StructArray::from_slots(
                        1,
                        1,
                        names,
                        slots,
                        &self.cancel,
                    )?))
                }
            }
            "disp" | "display" => {
                arity(1, 1)?;
                if name == "display" {
                    let label = self
                        .functions
                        .lng
                        .call_names
                        .first()
                        .filter(|n| !n.is_empty())
                        .cloned()
                        .unwrap_or_else(|| "ans".into());
                    self.cls_echo(&label, object, out)?;
                } else {
                    let shown = self.cls_display_object(&program, object, out)?;
                    let text = self
                        .cls_object_text(None, &shown)?
                        .expect("user object text");
                    self.emit(out, text)?;
                }
                Value::Void
            }
            "horzcat" | "vertcat" | "cat" | "repmat" => return Err(OBJECT_ARRAYS.into()),
            _ => return Ok(None),
        };
        Ok(Some(vec![value]))
    }
    /// A 1x1 struct with the single field `member`.
    fn cls_field_struct(&self, member: &str, value: Array) -> Result<Value> {
        Ok(Value::Container(Array::Struct(StructArray::from_slots(
            1,
            1,
            vec![member.to_owned()],
            vec![Arc::new(value)],
            &self.cancel,
        )?)))
    }
    /// One step of `subsref` on a user object, at element `i` of the
    /// subscript structure `s`: `.name` reads a property or calls a method
    /// with the property and access rules of `p.name` (a method takes the
    /// arguments of a following `()` step). Returns the value and the next
    /// element. Indexing the object itself fails as `p(1)` and `p{1}` do.
    pub(super) fn cls_subsref_step(
        &mut self,
        object: Value,
        s: &StructArray,
        i: usize,
        out: &mut Execution,
    ) -> Result<(Value, usize)> {
        let Value::Container(array) = &object else {
            unreachable!("user object")
        };
        let class_name = object_class(array).expect("user object");
        let (kind, sub) = subs_element(s, i)?;
        match kind.as_str() {
            "." => {}
            "()" => return Err(OBJECT_ARRAYS.into()),
            "{}" => {
                return Err(format!(
                    "Brace indexing is not supported for objects of class '{class_name}'."
                ));
            }
            _ => return Err("Invalid subscript type.".into()),
        }
        let member = text(&Value::from(sub))
            .ok_or("Field names must be character vectors or string scalars.")?;
        let (program, class) = self.cls_require(&class_name, out)?;
        let arguments_follow = i + 1 < s.len() && subs_element(s, i + 1)?.0 == "()";
        let call = if arguments_follow && class.property(&member).is_none() {
            self.cls_method(&class, &member)?
        } else {
            None
        };
        let Some(method) = call else {
            let value = self.cls_member(&object, &member, None, out)?;
            return Ok((value.expect("user object member"), i + 1));
        };
        let (target, index) = self.cls_target(&program, method)?;
        let mut values = if method.is_static {
            Vec::new()
        } else {
            vec![object.clone()]
        };
        let Array::Cell(arguments) = subs_element(s, i + 1)?.1 else {
            return Err("Subscripts must be a cell array.".into());
        };
        for k in 0..arguments.len() {
            values.push(Value::from(arguments.get_linear(k)?.as_ref().clone()));
        }
        let mut result = self.cls_invoke(&target, index, values, 1, None, out)?;
        Ok((result.remove(0), i + 2))
    }
    /// A subscript structure (`substruct`) as an assignment path.
    fn cls_subs_steps(&self, subs: &Value) -> Result<Vec<PathStep>> {
        let Value::Container(Array::Struct(s)) = subs else {
            return Err("Second input must be a subscript structure.".into());
        };
        let mut steps = Vec::new();
        for i in 0..s.len() {
            self.check_cancel()?;
            let (kind, sub) = subs_element(s, i)?;
            steps.push(match kind.as_str() {
                "." => PathStep::Field(
                    text(&Value::from(sub))
                        .ok_or("Field names must be character vectors or string scalars.")?,
                ),
                "()" | "{}" => {
                    let Array::Cell(c) = sub else {
                        return Err("Subscripts must be a cell array.".into());
                    };
                    let mut indices = Vec::new();
                    for k in 0..c.len() {
                        indices.push(match c.get_linear(k)?.as_ref() {
                            Array::Numeric(m) => Expr::Matrix(vec![
                                m.data().iter().copied().map(Expr::Number).collect(),
                            ]),
                            Array::Character(c) if c.units() == [u16::from(b':')] => Expr::All,
                            _ => return Err("Unsupported subscript type.".into()),
                        });
                    }
                    if kind == "()" {
                        PathStep::Paren(indices)
                    } else {
                        PathStep::Brace(indices)
                    }
                }
                _ => return Err("Invalid subscript type.".into()),
            });
        }
        Ok(steps)
    }
    /// methods / properties / ismethod / isprop against a loaded class.
    fn cls_query(
        &mut self,
        name: &str,
        program: &Arc<Program>,
        args: &[Value],
        requested: usize,
        out: &mut Execution,
    ) -> Result<Option<Vec<Value>>> {
        let class = program.class.clone().expect("class program");
        let value = match name {
            "methods" | "properties" => {
                if args.len() > 1 {
                    return Err("Too many input arguments.".into());
                }
                let (names, label) = if name == "methods" {
                    (class.public_methods(), "Methods")
                } else {
                    (class.public_properties(), "Properties")
                };
                if requested == 0 {
                    let mut listing = String::new();
                    if !self.display.compact {
                        listing.push('\n');
                    }
                    listing.push_str(&format!("{label} for class {}:\n", class.name));
                    if !self.display.compact {
                        listing.push('\n');
                    }
                    for n in &names {
                        listing.push_str(&format!("    {n}\n"));
                    }
                    if self.display.compact {
                        listing.pop();
                    }
                    self.emit(out, listing)?;
                    return Ok(Some(vec![Value::Void]));
                }
                self.cls_names_cell(names)?
            }
            _ => {
                if args.len() != 2 {
                    return Err(if args.len() < 2 {
                        "Not enough input arguments."
                    } else {
                        "Too many input arguments."
                    }
                    .into());
                }
                let Some(member) = text(&args[1]) else {
                    return Err("The name must be a character vector or string scalar.".into());
                };
                // The reference answers isprop for every property, ismethod
                // for public methods only.
                logical(if name == "isprop" {
                    class.property(&member).is_some()
                } else {
                    class.public_methods().contains(&member)
                })
            }
        };
        Ok(Some(vec![value]))
    }
    /// isprop for values that are not user objects: graphics handles answer
    /// from the property table of their class (one logical per handle), the
    /// struct-backed language objects (MException, ...) from their public
    /// fields. Other values (numbers, structs, cells, ...) are not supported yet.
    fn cls_isprop_builtin(&mut self, args: &[Value]) -> Result<Value> {
        if args.len() != 2 {
            return Err(if args.len() < 2 {
                "Not enough input arguments."
            } else {
                "Too many input arguments."
            }
            .into());
        }
        let Some(member) = text(&args[1]) else {
            return Err("The name must be a character vector or string scalar.".into());
        };
        match &args[0] {
            Value::Container(Array::Graphics(handles)) => {
                let mut found = Vec::new();
                found
                    .try_reserve_exact(handles.len())
                    .map_err(|_| "Cannot allocate the isprop result")?;
                for i in 0..handles.len() {
                    self.check_cancel()?;
                    found.push(u8::from(
                        self.gobj_is_property(handles.get_linear(i)?, &member),
                    ));
                }
                Ok(Value::Logical(LogicalMatrix::from_column_vec(
                    handles.rows(),
                    handles.cols(),
                    found,
                    &self.cancel,
                )?))
            }
            Value::Container(a @ Array::Struct(s))
                if super::lang_more::lng_class(a) != "struct" =>
            {
                Ok(logical(
                    super::lang_more::lng_public_fields(s).contains(&member),
                ))
            }
            _ => Err(crate::help::unknown_function("isprop")),
        }
    }
    /// methods of a built-in object class with a fixed method table.
    fn cls_builtin_methods(
        &mut self,
        class: &str,
        requested: usize,
        out: &mut Execution,
    ) -> Result<Option<Vec<Value>>> {
        let Some(table) = super::lang_extra::lg3_method_table(class) else {
            return Ok(None);
        };
        let mut names: Vec<String> = table.iter().map(|s| (*s).to_owned()).collect();
        names.sort();
        names.dedup();
        if requested == 0 {
            let mut listing = format!("Methods for class {class}:\n");
            if !self.display.compact {
                listing.insert(0, '\n');
                listing.push('\n');
            }
            for n in &names {
                listing.push_str(&format!("    {n}\n"));
            }
            if self.display.compact {
                listing.pop();
            }
            self.emit(out, listing)?;
            return Ok(Some(vec![Value::Void]));
        }
        Ok(Some(vec![self.cls_names_cell(names)?]))
    }
    fn cls_names_cell(&self, names: Vec<String>) -> Result<Value> {
        let slots = names
            .iter()
            .map(|n| {
                Ok(Arc::new(Array::Character(CharacterRow::from_text(
                    n,
                    &self.cancel,
                )?)))
            })
            .collect::<Result<Vec<_>>>()?;
        let rows = slots.len();
        Ok(Value::Container(Array::Cell(CellArray::from_shared(
            rows,
            1,
            slots,
            &self.cancel,
        )?)))
    }

    /// The object as displayed: its stored properties with the get-method
    /// value of each shown property that has one (Dependent ones included).
    /// A deleted handle stays as it is.
    fn cls_display_object(
        &mut self,
        program: &Arc<Program>,
        a: &Array,
        out: &mut Execution,
    ) -> Result<Array> {
        if handles::is_handle_object(a) && !self.cls_is_valid(a) {
            return Ok(a.clone());
        }
        if a.is_empty() {
            return Ok(a.clone());
        }
        let class = program.class.clone().expect("class program");
        let Array::Struct(mut shown) = self.cls_storage(a)?.into_owned() else {
            unreachable!("object storage")
        };
        for (i, p) in class.properties.iter().enumerate() {
            if p.get == Access::Public && !p.hidden && !p.constant && p.getter.is_some() {
                let value = self.cls_read(program, a, i, out)?;
                shown = shown.with_field(&p.name, value, &self.cancel)?;
            }
        }
        Ok(Array::Struct(shown))
    }
    /// Default display: `label = ` then `  Name with properties:` and the
    /// public non-Constant properties formatted like struct fields. Without a
    /// label, the `disp` text. Uses the class loaded in this run, else its
    /// last loaded definition; private, Hidden and hidden fields never show.
    /// A deleted handle shows `handle to deleted Name`.
    pub(super) fn cls_object_text(&self, label: Option<&str>, a: &Array) -> Result<Option<String>> {
        let Some(class_name) = object_class(a) else {
            return Ok(None);
        };
        if a.is_empty() {
            let body = format!("  {}x{} empty {class_name} array\n", a.rows(), a.cols());
            return Ok(Some(match label {
                None => body,
                Some(label) => crate::display_format::wrap(label, true, &body, &self.display),
            }));
        }
        if let Some(member) = events::enum_name(a) {
            let body = format!("  {class_name} enumeration\n\n    {member}\n");
            return Ok(Some(match label {
                None => body,
                Some(label) => crate::display_format::wrap(label, true, &body, &self.display),
            }));
        }
        if let Some(body) = self.cls_deleted_text(a) {
            return Ok(Some(match label {
                None => body,
                Some(label) => crate::display_format::wrap(label, true, &body, &self.display),
            }));
        }
        let storage = self.cls_storage(a)?;
        let Array::Struct(s) = storage.as_ref() else {
            unreachable!()
        };
        let mut visible = Vec::new();
        let mut slots = Vec::new();
        // The class loaded in this run, else the definition it had when last
        // loaded (Constant values are then not known yet and not shown). With
        // no definition known nothing is listed: never private or hidden fields.
        let class = match self.functions.cls_cached(&class_name) {
            Some(program) => program.class.clone(),
            None => self.functions.classes.known.get(&class_name).cloned(),
        };
        if let Some(class) = class {
            for (i, p) in class.properties.iter().enumerate() {
                if p.get != Access::Public || p.hidden {
                    continue;
                }
                let value = if p.constant {
                    // Cached under the defining class.
                    let (path, i) = match &p.inherited {
                        Some((program, j)) => {
                            (&program.class.as_ref().expect("class program").path, *j)
                        }
                        None => (&class.path, i),
                    };
                    self.functions
                        .classes
                        .values
                        .get(path)
                        .and_then(|v| v.get(i))
                        .cloned()
                        .flatten()
                        .map(Arc::new)
                } else {
                    s.field_index(&p.name)
                        .and_then(|f| s.get(f, 0).ok())
                        .cloned()
                };
                if let Some(value) = value {
                    visible.push(p.name.clone());
                    slots.push(value);
                }
            }
        }
        if visible.is_empty() {
            let body = format!("  {class_name} with no properties.\n");
            return Ok(Some(match label {
                None => body,
                Some(label) => crate::display_format::wrap(label, true, &body, &self.display),
            }));
        }
        let fields = Array::Struct(StructArray::from_slots(1, 1, visible, slots, &self.cancel)?);
        let header = format!("  {class_name} with properties:\n\n");
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
    /// `label = ...` display of a user object: the class's own `disp` method
    /// after the `label =` line (a private one too), or the default text.
    /// Chosen by the tag of the displayed value (see `assign_workspace_array`).
    pub(super) fn cls_echo(&mut self, label: &str, a: &Array, out: &mut Execution) -> Result<()> {
        let class_name = object_class(a).expect("user object");
        let (program, class) = self.cls_require(&class_name, out)?;
        let deleted = handles::is_handle_object(a) && !self.cls_is_valid(a);
        if !deleted && let Some(method) = class.instance_method("disp")?.filter(|m| !m.is_abstract)
        {
            let header = if self.display.compact {
                format!("{label} =")
            } else {
                format!("\n{label} =\n")
            };
            self.emit(out, header)?;
            let (target, index) = self.cls_target(&program, method)?;
            self.cls_invoke(&target, index, vec![Value::from(a.clone())], 0, None, out)?;
            return Ok(());
        }
        let shown = self.cls_display_object(&program, a, out)?;
        let text = self
            .cls_object_text(Some(label), &shown)?
            .expect("user object text");
        self.emit(out, text)
    }
    fn cls_object_variable(&self, e: &Expr) -> bool {
        matches!(e, Expr::Name(n) if self.container_variables.get(n).is_some_and(is_object))
    }

    /// Statements whose call forms route to class methods, once a class file
    /// has been loaded in this engine: method calls with nargout 0
    /// (`obj.show()`, `show(obj)`, `Name.staticMethod()`), listings without an
    /// output, and a bare object name. Results are assigned to `ans` through
    /// `assign_workspace_array`, which shows an object by its own tag, so a
    /// first construction in a fresh session displays like any other.
    /// True if handled.
    pub(super) fn cls_statement(&mut self, st: &Evaluation, out: &mut Execution) -> Result<bool> {
        if !self.functions.classes.seen
            && !matches!(&st.expr, Expr::Call(n, _) if matches!(n.as_str(), "methods" | "properties" | "superclasses" | "events" | "enumeration"))
        {
            return Ok(false);
        }
        let plain = st.targets.is_empty()
            && st.indexed_target.is_none()
            && st.path_target.is_none()
            && st.list_target.is_none();
        if !plain {
            return Ok(false);
        }
        let value = match &st.expr {
            Expr::Name(n) if self.cls_object_variable(&st.expr) => {
                if !st.quiet {
                    let a = self.container_variables[n].clone();
                    self.cls_echo(n, &a, out)?;
                }
                return Ok(true);
            }
            // obj.method(args) / obj.method
            Expr::Index(base, args) if matches!(base.as_ref(), Expr::Field(root, _) if self.cls_object_variable(root)) =>
            {
                let Expr::Field(root, member) = base.as_ref() else {
                    unreachable!()
                };
                match self.cls_statement_member(root, member, Some(args), out)? {
                    Some(value) => value,
                    None => return Ok(false),
                }
            }
            Expr::Field(root, member) if self.cls_object_variable(root) => {
                match self.cls_statement_member(root, member, None, out)? {
                    Some(value) => value,
                    None => return Ok(false),
                }
            }
            // Name.staticMethod(args)
            Expr::Index(base, args) if matches!(base.as_ref(), Expr::Field(root, _) if matches!(root.as_ref(), Expr::Name(n) if !self.contains_array(n))) =>
            {
                let Expr::Field(root, member) = base.as_ref() else {
                    unreachable!()
                };
                let Expr::Name(class) = root.as_ref() else {
                    unreachable!()
                };
                match self.cls_static_statement(class, member, args, out)? {
                    Some(value) => value,
                    None => return Ok(false),
                }
            }
            // method(obj, ...), methods(obj), disp(obj): the class method
            // wins over functions of the folder and builtins.
            Expr::Call(name, args)
                if !self.contains_array(name)
                    && args.first().is_some_and(|a| self.cls_object_variable(a)) =>
            {
                let Some(Expr::Name(variable)) = args.first() else {
                    unreachable!()
                };
                // An imported, local or private function comes first.
                if self.cls_shadowed(name)? {
                    return Ok(false);
                }
                let object = self.container_variables[variable].clone();
                let class_name = object_class(&object).expect("user object");
                let (_, class) = self.cls_require(&class_name, out)?;
                let own = class.ambiguous.contains_key(name.as_str())
                    || class
                        .methods
                        .get(name.as_str())
                        .is_some_and(|m| !m.is_static);
                if !own
                    && !matches!(
                        name.as_str(),
                        "disp"
                            | "display"
                            | "delete"
                            | "methods"
                            | "properties"
                            | "superclasses"
                            | "events"
                            | "enumeration"
                    )
                    && !(class.is_a(APPBASE_CLASS) && APPBASE_METHODS.contains(&name.as_str()))
                {
                    return Ok(false);
                }
                let values = self.function_arguments(args, None, out)?;
                if own {
                    // inputname inside the method; a builtin takes none.
                    self.lng_note_inputs(args, values.len());
                }
                match self.cls_call(name, &values, 0, None, out)? {
                    Some(mut result) => result.remove(0),
                    None => return self.cls_finish_call(name, values, st.quiet, out),
                }
            }
            // methods('Name') / properties('Name') / superclasses('Name')
            // without an output: a listing.
            Expr::Call(name, args)
                if matches!(
                    name.as_str(),
                    "methods" | "properties" | "superclasses" | "events" | "enumeration"
                ) && !self.contains_array(name)
                    && matches!(args.as_slice(), [Expr::String(_) | Expr::DoubleQuoted(_)]) =>
            {
                let values = self.function_arguments(args, None, out)?;
                match self.cls_call(name, &values, 0, None, out)? {
                    Some(mut result) => result.remove(0),
                    None => return self.cls_finish_call(name, values, st.quiet, out),
                }
            }
            _ => {
                return Ok(false);
            }
        };
        if !matches!(value, Value::Void) {
            let array = value.array(&self.cancel)?;
            self.assign_workspace_array("ans", array, st.quiet, out)?;
        }
        Ok(true)
    }
    /// `Cls.staticMethod(args)` (also `a.Cls.staticMethod(args)`) as a
    /// statement: the static method with nargout 0. None when `class` is no
    /// class or `member` no static method of it.
    pub(super) fn cls_static_statement(
        &mut self,
        class: &str,
        member: &str,
        args: &[Expr],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        let Some(program) = self.cls_lookup(class, out)? else {
            return Ok(None);
        };
        let info = program.class.clone().expect("class program");
        if !info.methods.get(member).is_some_and(|m| m.is_static) {
            return Ok(None);
        }
        let method = self.cls_method(&info, member)?.expect("static method");
        let (target, index) = self.cls_target(&program, method)?;
        let values = self.function_arguments(args, None, out)?;
        Ok(Some(
            self.cls_invoke(&target, index, values, 0, None, out)?
                .remove(0),
        ))
    }
    /// `name(values...)` as a statement when no class method took it: the
    /// ordinary call with the already evaluated arguments.
    fn cls_finish_call(
        &mut self,
        name: &str,
        values: Vec<Value>,
        quiet: bool,
        out: &mut Execution,
    ) -> Result<bool> {
        let value = self.call(name, values, out)?;
        if !matches!(value, Value::Void) {
            let array = value.array(&self.cancel)?;
            self.assign_workspace_array("ans", array, quiet, out)?;
        }
        Ok(true)
    }
    fn cls_statement_member(
        &mut self,
        root: &Expr,
        member: &str,
        args: Option<&[Expr]>,
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        let Expr::Name(n) = root else { return Ok(None) };
        let object = self.container_variables[n].clone();
        let class_name = object_class(&object).expect("user object");
        let (program, class) = self.cls_require(&class_name, out)?;
        // A property read keeps the ordinary path (it displays as ans).
        if class.property(member).is_some() {
            return Ok(None);
        }
        let Some(method) = self.cls_method(&class, member)? else {
            if (class.is_handle && HANDLE_METHODS.contains(&member))
                || (class.is_a(APPBASE_CLASS) && APPBASE_METHODS.contains(&member))
            {
                let mut values = vec![Value::from(object)];
                if let Some(args) = args {
                    values.extend(self.function_arguments(args, None, out)?);
                }
                let mut result = self
                    .cls_call(member, &values, 0, None, out)?
                    .expect("handle or AppBase method");
                return Ok(Some(result.remove(0)));
            }
            return Err(format!(
                "Unrecognized method, property, or field '{member}' for class '{class_name}'."
            ));
        };
        let mut values = if method.is_static {
            Vec::new()
        } else {
            vec![Value::from(object)]
        };
        let (target, index) = self.cls_target(&program, method)?;
        if let Some(args) = args {
            values.extend(self.function_arguments(args, None, out)?);
        }
        Ok(Some(
            self.cls_invoke(&target, index, values, 0, None, out)?
                .remove(0),
        ))
    }
    /// `[a, b] = obj.method(...)`, `[a, b] = method(obj, ...)` and
    /// `[a, b] = Name.staticMethod(...)`: methods with several outputs.
    pub(super) fn cls_outputs(
        &mut self,
        expr: &Expr,
        targets: &[Option<String>],
        out: &mut Execution,
    ) -> Result<Option<Vec<Value>>> {
        let count = targets.len();
        match expr {
            Expr::Call(name, args) if name.contains('@') => {
                let values = self.function_arguments(args, None, out)?;
                let values = self.cls_super_call(name, &values, count, Some(targets), out)?;
                if values.len() != count {
                    return Err("Too many output arguments.".into());
                }
                Ok(Some(values))
            }
            // [m, s] = enumeration('Name')
            Expr::Call(name, args)
                if name == "enumeration"
                    && !self.contains_array(name)
                    && matches!(args.as_slice(), [Expr::String(_) | Expr::DoubleQuoted(_)])
                    && self.resolve_function(name)?.is_none() =>
            {
                let values = self.function_arguments(args, None, out)?;
                match self.cls_event_call(name, &values, count, out)? {
                    Some(values) if values.len() == count => Ok(Some(values)),
                    _ => Err("Too many output arguments.".into()),
                }
            }
            Expr::Call(name, args)
                if !self.contains_array(name)
                    && args.first().is_some_and(|a| self.cls_object_variable(a)) =>
            {
                // Decided from the first argument, a plain variable, before any
                // argument is evaluated: a call that is not ours must run once.
                let Some(Expr::Name(variable)) = args.first() else {
                    unreachable!()
                };
                // An imported, local or private function comes first.
                if self.cls_shadowed(name)? {
                    return Ok(None);
                }
                let object = self.container_variables[variable].clone();
                let class_name = object_class(&object).expect("user object");
                let (program, class) = self.cls_require(&class_name, out)?;
                let method = match class.instance_method(name)? {
                    Some(_) => self.cls_method(&class, name)?,
                    None => None,
                };
                if method.is_none() && !is_object_function(name) {
                    return Ok(None);
                }
                let method = match method {
                    Some(method) => Some(self.cls_target(&program, method)?),
                    None => None,
                };
                let values = self.function_arguments(args, None, out)?;
                match method {
                    Some((target, index)) => {
                        self.lng_note_inputs(args, values.len());
                        self.cls_invoke(&target, index, values, count, Some(targets), out)
                            .map(Some)
                    }
                    None => match self.cls_call(name, &values, count, Some(targets), out)? {
                        Some(values) if values.len() == count => Ok(Some(values)),
                        _ => Err("Too many output arguments.".into()),
                    },
                }
            }
            Expr::Call(name, args)
                if count >= 2 && !self.contains_array(name) && !args.is_empty() =>
            {
                self.cls_expression_outputs(name, args, targets, out)
            }
            Expr::Index(base, args) => {
                let Expr::Field(root, member) = base.as_ref() else {
                    return Ok(None);
                };
                let (program, class, receiver) = if self.cls_object_variable(root) {
                    let Expr::Name(n) = root.as_ref() else {
                        unreachable!()
                    };
                    let object = self.container_variables[n].clone();
                    let class_name = object_class(&object).expect("user object");
                    let (program, class) = self.cls_require(&class_name, out)?;
                    (program, class, Some(Value::from(object)))
                } else if let Expr::Name(n) = root.as_ref() {
                    if let Some(object) = self.array(n) {
                        return super::sci_more::triangulation::method_outputs(
                            self,
                            &Value::from(object),
                            member,
                            args,
                            count,
                            out,
                        );
                    }
                    let Some(program) = self.cls_lookup(n, out)? else {
                        return Ok(None);
                    };
                    let class = program.class.clone().expect("class program");
                    (program, class, None)
                } else {
                    if count < 2 {
                        return Ok(None);
                    }
                    // Any other receiver (`s.p`, `c{1}`, `Pt(3)`, `make()`) is
                    // evaluated once, here: the ordinary path would evaluate it
                    // too and then call `receiver.member` as a function handle,
                    // which `cls_receiver_outputs` does for a value that is not
                    // an object with that method.
                    let receiver = self.eval(root, None, out)?;
                    return self
                        .cls_receiver_outputs(receiver, member, args, targets, out)
                        .map(Some);
                };
                if !class.methods.contains_key(member.as_str())
                    && !class.ambiguous.contains_key(member.as_str())
                {
                    return Ok(None);
                }
                if receiver.is_none()
                    && !class
                        .methods
                        .get(member.as_str())
                        .is_some_and(|m| m.is_static)
                {
                    return Ok(None);
                }
                let method = self.cls_method(&class, member)?.expect("method");
                let mut values: Vec<Value> = if method.is_static {
                    Vec::new()
                } else {
                    receiver.into_iter().collect()
                };
                let (target, index) = self.cls_target(&program, method)?;
                values.extend(self.function_arguments(args, None, out)?);
                self.cls_invoke(&target, index, values, count, Some(targets), out)
                    .map(Some)
            }
            _ => Ok(None),
        }
    }
    /// `[a, b] = receiver.member(args)` for an evaluated receiver: a method
    /// of a user object runs; otherwise, as the ordinary multi-output path
    /// does, `receiver.member` must be one function handle.
    fn cls_receiver_outputs(
        &mut self,
        receiver: Value,
        member: &str,
        args: &[Expr],
        targets: &[Option<String>],
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        let count = targets.len();
        if let Value::Container(object) = &receiver
            && let Some(class_name) = object_class(object)
        {
            let (program, class) = self.cls_require(&class_name, out)?;
            if class.property(member).is_none()
                && let Some(method) = self.cls_method(&class, member)?
            {
                let mut values = if method.is_static {
                    Vec::new()
                } else {
                    vec![receiver.clone()]
                };
                let (target, index) = self.cls_target(&program, method)?;
                values.extend(self.function_arguments(args, None, out)?);
                return self.cls_invoke(&target, index, values, count, Some(targets), out);
            }
        }
        if let Some(values) = super::sci_more::triangulation::method_outputs(
            self, &receiver, member, args, count, out,
        )? {
            return Ok(values);
        }
        let mut member_values = self.obj_field_values_resolved(receiver, member, out)?;
        if member_values.len() != 1 {
            return Err(format!(
                "Expected one output from a curly brace or dot indexing expression, but there were {} results",
                member_values.len()
            ));
        }
        let Value::Container(Array::Handle(handle)) = member_values.remove(0) else {
            return Err("Multiple outputs require a function handle call".into());
        };
        let args = self.function_arguments(args, None, out)?;
        let values = self.call_handle(&handle, args, count, Some(targets), out)?;
        if values.len() != count {
            return Err("Too many output arguments.".into());
        }
        self.charge_function_outputs(&values)?;
        Ok(values)
    }
    /// `[a, b] = name(expr, ...)` whose first argument is an expression, not
    /// a variable. Its class decides without running any code twice:
    /// a constructor call `Pt(...)` makes a Pt; when nothing else answers to
    /// `name` (which would fail anyway) the arguments are evaluated once and
    /// the first one's class decides; a path into a variable (`s.p`, `c{1}`)
    /// runs no code, so it is looked at first. Else the ordinary call runs.
    fn cls_expression_outputs(
        &mut self,
        name: &str,
        args: &[Expr],
        targets: &[Option<String>],
        out: &mut Execution,
    ) -> Result<Option<Vec<Value>>> {
        // An imported, local or private function comes before any method.
        if self.cls_shadowed(name)? {
            return Ok(None);
        }
        let first = &args[0];
        if let Expr::Call(class, _) | Expr::Name(class) = first
            && !self.contains_array(class)
            && let Some(program) = self.cls_lookup(class, out)?
        {
            let info = program.class.clone().expect("class program");
            if !info.methods.get(name).is_some_and(|m| !m.is_static) {
                return Ok(None);
            }
            let values = self.function_arguments(args, None, out)?;
            return self
                .cls_value_outputs(name, args, values, targets, out)
                .map(Some);
        }
        // A variable here is not an object (see `cls_outputs`).
        if matches!(first, Expr::Name(n) if self.contains_array(n)) {
            return Ok(None);
        }
        if self.resolve_function(name)?.is_none() && !crate::help::is_supported(name) {
            let values = self.function_arguments(args, None, out)?;
            return self
                .cls_value_outputs(name, args, values, targets, out)
                .map(Some);
        }
        // Peek only at reads through a struct or cell variable: ordinary
        // numeric arguments (`max(x(:,k))`, `eig(A'*A)`) are never evaluated
        // twice.
        if !self.cls_container_path(first) || !self.cls_pure(first, 0) {
            return Ok(None);
        }
        let Ok(Value::Container(value)) = self.eval(first, None, out) else {
            return Ok(None);
        };
        let Some(class_name) = object_class(&value) else {
            return Ok(None);
        };
        let (_, class) = self.cls_require(&class_name, out)?;
        if !class.methods.get(name).is_some_and(|m| !m.is_static) {
            return Ok(None);
        }
        let values = self.function_arguments(args, None, out)?;
        self.cls_value_outputs(name, args, values, targets, out)
            .map(Some)
    }
    /// Method `name` of the object `values[0]` with several outputs; any other
    /// first value fails like an unknown function.
    fn cls_value_outputs(
        &mut self,
        name: &str,
        args: &[Expr],
        values: Vec<Value>,
        targets: &[Option<String>],
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        let class_name = match values.first() {
            Some(Value::Container(a)) => object_class(a),
            _ => None,
        };
        let Some(class_name) = class_name else {
            return Err(crate::help::unknown_function(name));
        };
        let (program, class) = self.cls_require(&class_name, out)?;
        if class.instance_method(name)?.is_none() {
            return Err(crate::help::unknown_function(name));
        }
        let method = self.cls_method(&class, name)?.expect("method");
        let (target, index) = self.cls_target(&program, method)?;
        self.lng_note_inputs(args, values.len());
        self.cls_invoke(&target, index, values, targets.len(), Some(targets), out)
    }
    /// A path into a variable whose evaluation runs no code: literals, data
    /// variables, `end`, `:`, operators on them, `x(i)`, `c{i}`, and `v.name`
    /// on a variable (a property, never a method, of an object variable).
    /// A read through a struct or cell variable (`s.p`, `c{1}`, `s.c{2}.p`):
    /// the only argument form that can hold a stored object without running
    /// code. Arithmetic, ranges and numeric indexing are never objects.
    fn cls_container_path(&self, e: &Expr) -> bool {
        let root = |n: &str| {
            matches!(
                self.container_variables.get(n),
                Some(Array::Struct(_) | Array::Cell(_))
            )
        };
        let mut e = e;
        loop {
            match e {
                Expr::CellContents(n, _) => return root(n),
                Expr::Field(base, _)
                | Expr::DynamicField(base, _)
                | Expr::Brace(base, _)
                | Expr::Index(base, _) => {
                    if let Expr::Name(n) = base.as_ref() {
                        return root(n);
                    }
                    e = base;
                }
                _ => return false,
            }
        }
    }
    fn cls_pure(&self, e: &Expr, depth: usize) -> bool {
        if depth > 16 {
            return false;
        }
        let pure = |e: &Expr| self.cls_pure(e, depth + 1);
        let data = |n: &str| {
            self.contains_array(n)
                && !matches!(self.container_variables.get(n), Some(Array::Handle(_)))
        };
        match e {
            Expr::Number(_)
            | Expr::Imaginary(_)
            | Expr::NameValue(_)
            | Expr::String(_)
            | Expr::DoubleQuoted(_)
            | Expr::All => true,
            Expr::Name(n) => n == "end" || data(n),
            Expr::Unary(_, a)
            | Expr::Not(a)
            | Expr::Transpose(a)
            | Expr::NonConjugateTranspose(a) => pure(a),
            Expr::Binary(_, a, b) => pure(a) && pure(b),
            Expr::Range(a, step, b) => pure(a) && step.as_deref().is_none_or(pure) && pure(b),
            Expr::Matrix(rows) | Expr::CellLiteral(rows) => rows.iter().flatten().all(pure),
            Expr::Call(n, args) | Expr::CellContents(n, args) => data(n) && args.iter().all(pure),
            Expr::Index(base, args) | Expr::Brace(base, args) => {
                pure(base) && args.iter().all(pure)
            }
            Expr::Field(base, member) => {
                let Expr::Name(n) = base.as_ref() else {
                    return false;
                };
                match self.container_variables.get(n) {
                    Some(a) if is_object(a) => object_class(a)
                        .and_then(|c| self.functions.cls_cached(&c))
                        .and_then(|p| p.class.as_ref())
                        .is_some_and(|c| {
                            // A get method would run code.
                            c.property(member).is_some_and(|i| {
                                c.properties[i].getter.is_none() && !c.properties[i].dependent
                            })
                        }),
                    _ => data(n),
                }
            }
            _ => false,
        }
    }
}
