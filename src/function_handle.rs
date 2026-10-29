//! Function-handle values: immutable, cheap to clone, shared by `Arc`.
//! Anonymous handles own a snapshot of the workspace variables their body
//! references, taken when the handle is created.
use crate::{Array, syntax::AnonymousFunction};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

/// Deepest chain of handles that capture handles. Bounds the recursive drop
/// of a chain; a longer chain could not be called within the evaluation depth.
pub const MAX_CAPTURE_DEPTH: usize = 100;
const OVERHEAD: usize = 64;

#[derive(Clone, Debug)]
pub struct FunctionHandle(Arc<Inner>);

#[derive(Debug)]
enum Inner {
    /// `@name`, resolved when called. A handle made inside a file to one of
    /// its local functions keeps that file (`functions` reports it).
    Named(String, Option<PathBuf>),
    Anonymous {
        function: Arc<AnonymousFunction>,
        captured: Vec<(String, Array)>,
        depth: usize,
        bytes: usize,
    },
}
impl FunctionHandle {
    pub fn named(name: &str) -> Self {
        Self(Arc::new(Inner::Named(name.to_owned(), None)))
    }
    /// `@name` for a local function of the file at `file`.
    pub fn local(name: &str, file: &Path) -> Self {
        Self(Arc::new(Inner::Named(
            name.to_owned(),
            Some(file.to_owned()),
        )))
    }
    /// The file whose local function a `@name` handle targets.
    pub fn file(&self) -> Option<&Path> {
        match &*self.0 {
            Inner::Named(_, file) => file.as_deref(),
            Inner::Anonymous { .. } => None,
        }
    }
    pub fn anonymous(function: Arc<AnonymousFunction>, captured: Vec<(String, Array)>) -> Self {
        let depth = 1 + captured
            .iter()
            .map(|(_, value)| nesting_depth(value))
            .max()
            .unwrap_or(0);
        let bytes = captured
            .iter()
            .fold(OVERHEAD + function.text.len(), |sum, (name, value)| {
                sum.saturating_add(name.len() + value.bytes())
            });
        Self(Arc::new(Inner::Anonymous {
            function,
            captured,
            depth,
            bytes,
        }))
    }
    /// Target name of a `@name` handle.
    pub fn name(&self) -> Option<&str> {
        match &*self.0 {
            Inner::Named(name, _) => Some(name),
            Inner::Anonymous { .. } => None,
        }
    }
    /// Body and captured variables of an anonymous handle.
    pub fn anonymous_parts(&self) -> Option<(&AnonymousFunction, &[(String, Array)])> {
        match &*self.0 {
            Inner::Named(..) => None,
            Inner::Anonymous {
                function, captured, ..
            } => Some((function, captured)),
        }
    }
    /// MATLAB `func2str`: `sin` for `@sin`, `@(x)x+1` for anonymous handles.
    pub fn func2str(&self) -> &str {
        match &*self.0 {
            Inner::Named(name, _) => name,
            Inner::Anonymous { function, .. } => &function.text,
        }
    }
    /// Display text: `@sin` or `@(x)x+1`.
    pub fn display(&self) -> String {
        match &*self.0 {
            Inner::Named(name, _) => format!("@{name}"),
            Inner::Anonymous { function, .. } => function.text.clone(),
        }
    }
    pub fn depth(&self) -> usize {
        match &*self.0 {
            Inner::Named(..) => 1,
            Inner::Anonymous { depth, .. } => *depth,
        }
    }
    pub fn bytes(&self) -> usize {
        match &*self.0 {
            Inner::Named(name, file) => {
                OVERHEAD + name.len() + file.as_ref().map_or(0, |f| f.as_os_str().len())
            }
            Inner::Anonymous { bytes, .. } => *bytes,
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
