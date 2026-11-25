//! Function-handle values: immutable, cheap to clone, shared by `Arc`.
//! Anonymous handles own a snapshot of the workspace variables their body
//! references, taken when the handle is created.
use crate::{Array, syntax::AnonymousFunction};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

/// What a handle keeps of the code that made it, so the names its body uses
/// (or the local function it names) are looked up as there: the file, its
/// local functions and private folder, and the import list. Made and read by
/// the engine; opaque here. Shared by every copy of the handle.
pub(crate) type Scope = Arc<dyn std::any::Any + Send + Sync>;

/// Deepest chain of handles that capture handles. Bounds the recursive drop
/// of a chain; a longer chain could not be called within the evaluation depth.
pub const MAX_CAPTURE_DEPTH: usize = 100;
const OVERHEAD: usize = 64;

#[derive(Clone, Debug)]
pub struct FunctionHandle(Arc<Inner>);

#[derive(Debug)]
enum Inner {
    /// `@name`, resolved when called. A handle made inside a file to one of
    /// its local functions keeps that file (`functions` reports it) and,
    /// when made by running code, the scope that holds the function, so it
    /// still calls it after the file returned.
    Named(String, Option<PathBuf>, Option<Scope>),
    /// A specific lexical activation. Its live array slots are accounted for
    /// by the workspace. The handle retains its code and frame metadata.
    Nested {
        name: String,
        target: Scope,
        bytes: usize,
    },
    /// `@name` made inside a file to a function of that file's private
    /// folder: bound to the private file, so it still resolves when called
    /// from the Command Window or from another folder.
    Private(String, PathBuf),
    /// `@name` made by running code to a function of its own source that
    /// `functions` does not report as a local function: the main function
    /// of the file, or a function of text run without a file (`None`).
    /// While that source runs the handle calls it; afterwards it is looked
    /// up like `Global`.
    Source(String, Option<PathBuf>),
    /// `@name` made by running code when the name was none of its imported,
    /// local or private functions: a method of an object argument, the
    /// current folder and the search path, or a builtin. Never a local,
    /// imported or private function of the code that calls the handle.
    Global(String),
    Anonymous {
        function: Arc<AnonymousFunction>,
        captured: Vec<(String, Array)>,
        depth: usize,
        bytes: usize,
        /// Where the handle was made; `None`: looked up where it is called
        /// (handles built by the engine itself).
        scope: Option<Scope>,
    },
    /// Callable created by `matlab.apps.AppBase.createCallbackFcn(app, @Fn, requiresEvent)`.
    AppCallback {
        app: Array,
        callback: Box<FunctionHandle>,
        requires_event: bool,
        depth: usize,
        bytes: usize,
    },
}
impl FunctionHandle {
    pub fn named(name: &str) -> Self {
        Self(Arc::new(Inner::Named(name.to_owned(), None, None)))
    }
    /// A nested definition and the lexical activation to use when invoking it.
    pub(crate) fn nested(name: &str, target: Scope, frame_bytes: usize) -> Self {
        Self(Arc::new(Inner::Nested {
            name: name.to_owned(),
            target,
            bytes: OVERHEAD + name.len() + frame_bytes,
        }))
    }
    /// `@name` for a local function of the file at `file`.
    pub fn local(name: &str, file: &Path) -> Self {
        Self(Arc::new(Inner::Named(
            name.to_owned(),
            Some(file.to_owned()),
            None,
        )))
    }
    /// `@name` made by running code for a local function of the file at
    /// `file`; `scope` holds that file's functions.
    pub(crate) fn local_in(name: &str, file: &Path, scope: Scope) -> Self {
        Self(Arc::new(Inner::Named(
            name.to_owned(),
            Some(file.to_owned()),
            Some(scope),
        )))
    }
    /// `@name` for the private-folder function defined by `file`.
    pub fn private(name: &str, file: &Path) -> Self {
        Self(Arc::new(Inner::Private(name.to_owned(), file.to_owned())))
    }
    /// `@name` for the function `name` of the source the running code
    /// belongs to: `file`, or text run without a file.
    pub fn source(name: &str, file: Option<&Path>) -> Self {
        Self(Arc::new(Inner::Source(
            name.to_owned(),
            file.map(Path::to_owned),
        )))
    }
    /// `@name` that is none of the functions of the code that made it.
    pub fn global(name: &str) -> Self {
        Self(Arc::new(Inner::Global(name.to_owned())))
    }
    /// The file a `@name` handle is bound to: the file whose local function
    /// it targets, or the private file that defines it.
    pub fn file(&self) -> Option<&Path> {
        match &*self.0 {
            Inner::Named(_, file, _) => file.as_deref(),
            Inner::Private(_, file) => Some(file),
            Inner::Source(..)
            | Inner::Global(_)
            | Inner::Anonymous { .. }
            | Inner::Nested { .. }
            | Inner::AppCallback { .. } => None,
        }
    }
    /// The source a `@name` handle calls a function of while that source
    /// runs: `Some(Some(file))`, or `Some(None)` for text run without a
    /// file. `None`: the handle is not bound to a source.
    pub fn bound_source(&self) -> Option<Option<&Path>> {
        match &*self.0 {
            Inner::Named(_, Some(file), _) => Some(Some(file)),
            Inner::Source(_, file) => Some(file.as_deref()),
            _ => None,
        }
    }
    /// The handle never calls a local, imported or private function of the
    /// code that calls it (see `Inner::Global`).
    pub fn is_global(&self) -> bool {
        matches!(&*self.0, Inner::Global(_))
    }
    /// The private file a `@name` handle is bound to.
    pub fn private_file(&self) -> Option<&Path> {
        match &*self.0 {
            Inner::Private(_, file) => Some(file),
            _ => None,
        }
    }
    /// The scope kept from the code that made the handle (`Scope`).
    pub(crate) fn scope(&self) -> Option<&Scope> {
        match &*self.0 {
            Inner::Named(_, _, scope) | Inner::Anonymous { scope, .. } => scope.as_ref(),
            Inner::Nested { target, .. } => Some(target),
            _ => None,
        }
    }
    pub fn anonymous(function: Arc<AnonymousFunction>, captured: Vec<(String, Array)>) -> Self {
        Self::anonymous_in(function, captured, None, 0)
    }
    /// An anonymous handle made by running code: `scope` is where it was made.
    pub(crate) fn anonymous_in(
        function: Arc<AnonymousFunction>,
        captured: Vec<(String, Array)>,
        scope: Option<Scope>,
        frame_bytes: usize,
    ) -> Self {
        let depth = 1 + captured
            .iter()
            .map(|(_, value)| nesting_depth(value))
            .max()
            .unwrap_or(0);
        let bytes = captured.iter().fold(
            OVERHEAD + function.text.len() + frame_bytes,
            |sum, (name, value)| sum.saturating_add(name.len() + value.bytes()),
        );
        Self(Arc::new(Inner::Anonymous {
            function,
            captured,
            depth,
            bytes,
            scope,
        }))
    }
    /// Callable created by `createCallbackFcn(app, callback, requiresEvent)`.
    pub(crate) fn app_callback(app: Array, callback: FunctionHandle, requires_event: bool) -> Self {
        let depth = 1 + callback.depth().max(nesting_depth(&app));
        let bytes = OVERHEAD
            .saturating_add(app.bytes())
            .saturating_add(callback.bytes());
        Self(Arc::new(Inner::AppCallback {
            app,
            callback: Box::new(callback),
            requires_event,
            depth,
            bytes,
        }))
    }
    /// Target name of a `@name` handle.
    pub fn name(&self) -> Option<&str> {
        match &*self.0 {
            Inner::Named(name, ..)
            | Inner::Nested { name, .. }
            | Inner::Private(name, _)
            | Inner::Source(name, _)
            | Inner::Global(name) => Some(name),
            Inner::Anonymous { .. } | Inner::AppCallback { .. } => None,
        }
    }
    /// Body and captured variables of an anonymous handle.
    pub fn anonymous_parts(&self) -> Option<(&AnonymousFunction, &[(String, Array)])> {
        match &*self.0 {
            Inner::Anonymous {
                function, captured, ..
            } => Some((function, captured)),
            _ => None,
        }
    }
    /// Parts of an AppBase callback handle created by `createCallbackFcn`.
    pub(crate) fn app_callback_parts(&self) -> Option<(&Array, &FunctionHandle, bool)> {
        match &*self.0 {
            Inner::AppCallback {
                app,
                callback,
                requires_event,
                ..
            } => Some((app, callback, *requires_event)),
            _ => None,
        }
    }
    /// MATLAB `func2str`: `sin` for `@sin`, `@(x)x+1` for anonymous handles.
    pub fn func2str(&self) -> &str {
        match &*self.0 {
            Inner::Named(name, ..)
            | Inner::Nested { name, .. }
            | Inner::Private(name, _)
            | Inner::Source(name, _)
            | Inner::Global(name) => name,
            Inner::Anonymous { function, .. } => &function.text,
            Inner::AppCallback { requires_event, .. } => {
                if *requires_event {
                    "@(source,event)executeCallback(app,callback,event)"
                } else {
                    "@(source,event)executeCallback(app,callback)"
                }
            }
        }
    }
    /// Display text: `@sin` or `@(x)x+1`.
    pub fn display(&self) -> String {
        match &*self.0 {
            Inner::Anonymous { function, .. } => function.text.clone(),
            Inner::AppCallback { .. } => self.func2str().to_owned(),
            _ => format!("@{}", self.func2str()),
        }
    }
    pub fn depth(&self) -> usize {
        match &*self.0 {
            Inner::Anonymous { depth, .. } | Inner::AppCallback { depth, .. } => *depth,
            _ => 1,
        }
    }
    pub fn bytes(&self) -> usize {
        match &*self.0 {
            Inner::Named(name, file, _) | Inner::Source(name, file) => {
                OVERHEAD + name.len() + file.as_ref().map_or(0, |f| f.as_os_str().len())
            }
            Inner::Global(name) => OVERHEAD + name.len(),
            Inner::Private(name, file) => OVERHEAD + name.len() + file.as_os_str().len(),
            Inner::Anonymous { bytes, .. }
            | Inner::Nested { bytes, .. }
            | Inner::AppCallback { bytes, .. } => *bytes,
        }
    }
    pub fn same_storage(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl std::fmt::Display for FunctionHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.display())
    }
}
/// Longest handle-in-handle chain reachable through cells and struct fields.
pub(crate) fn nesting_depth(array: &Array) -> usize {
    match array {
        Array::Handle(handle) => handle.depth(),
        Array::Cell(cell) => (0..cell.len())
            .filter_map(|i| cell.get_linear(i).ok())
            .map(|child| nesting_depth(child))
            .max()
            .unwrap_or(0),
        Array::Struct(s) => (0..s.slot_count())
            .filter_map(|i| s.slot(i).ok())
            .map(|child| nesting_depth(child))
            .max()
            .unwrap_or(0),
        _ => 0,
    }
}
