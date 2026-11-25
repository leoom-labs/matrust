//! Lexical function scopes and the shared bindings of each invocation.
use super::super::handles::{NameUse, visit_names};
use super::{Engine, FunctionState, Program, ResolvedFunction, Statement, StatementKind};
use crate::{
    Array, CellArray, FunctionHandle, Result, StructArray,
    syntax::{Evaluation, Expr, PathStep},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Weak},
};

#[derive(Default)]
pub(super) struct Scope {
    pub(super) parent: Option<usize>,
    children: BTreeMap<String, usize>,
    /// For each name shared with another definition, its lexical owner's index.
    shared: BTreeMap<String, usize>,
    names: Option<Arc<BTreeSet<String>>>,
}

struct Names {
    references: BTreeSet<String>,
    variables: BTreeSet<String>,
}

impl Names {
    fn declare(&mut self, name: &str) {
        self.references.insert(name.to_owned());
        self.variables.insert(name.to_owned());
    }

    fn read(&mut self, expr: &Expr) {
        visit_names(expr, &mut Vec::new(), &mut |name, usage| {
            if usage != NameUse::Handle {
                self.references.insert(name.to_owned());
            }
            if usage == NameUse::Value {
                self.variables.insert(name.to_owned());
            }
        });
    }
}

/// Keep top-level indices stable. Move nested definitions out of executable
/// bodies and index each name only among the children of its lexical parent.
pub(super) fn index(statements: &mut Vec<Statement>) -> BTreeMap<usize, Scope> {
    let mut definitions: Vec<_> = statements
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            matches!(&s.kind, StatementKind::Function(f)
            if f.body.iter().any(|s| matches!(s.kind, StatementKind::Function(_))))
            .then_some(i)
        })
        .collect();
    let mut scopes: BTreeMap<_, _> = definitions.iter().map(|&i| (i, Scope::default())).collect();
    let mut uses = BTreeMap::<usize, Names>::new();
    let mut next = 0;
    while let Some(&i) = definitions.get(next) {
        let StatementKind::Function(f) = &mut statements[i].kind else {
            unreachable!()
        };
        let children: Vec<_> = f
            .body
            .extract_if(.., |s| matches!(s.kind, StatementKind::Function(_)))
            .collect();
        let arguments: BTreeSet<_> = f
            .inputs
            .iter()
            .flatten()
            .chain(&f.outputs)
            .cloned()
            .collect();
        let mut names = Names {
            references: arguments.clone(),
            variables: arguments.clone(),
        };
        for spec in f.arguments.iter().flat_map(|b| &b.specs) {
            for e in spec
                .default
                .iter()
                .chain(spec.validators.iter().flat_map(|v| &v.args))
            {
                names.read(e);
            }
        }
        body_names(&f.body, &mut names);
        for name in names.references.difference(&arguments) {
            let mut enclosing = scopes[&i].parent;
            while let Some(j) = enclosing {
                let scope = scopes.get_mut(&j).unwrap();
                let enclosing_names = &uses[&j].references;
                if enclosing_names.contains(name) {
                    let owner = *scope.shared.entry(name.clone()).or_insert(j);
                    scopes
                        .get_mut(&i)
                        .unwrap()
                        .shared
                        .insert(name.clone(), owner);
                    break;
                }
                enclosing = scope.parent;
            }
        }
        uses.insert(i, names);
        for s in children {
            let StatementKind::Function(child) = &s.kind else {
                unreachable!()
            };
            let child_index = statements.len();
            scopes
                .get_mut(&i)
                .unwrap()
                .children
                .insert(child.name.clone(), child_index);
            statements.push(s);
            definitions.push(child_index);
            scopes.insert(
                child_index,
                Scope {
                    parent: Some(i),
                    ..Default::default()
                },
            );
        }
        next += 1;
    }
    for (&index, scope) in &mut scopes {
        let mut names = uses[&index].variables.clone();
        // A call-like expression can index a statically referenced parent
        // variable when this definition already shares its slot.
        for (name, owner) in &scope.shared {
            if uses[owner].variables.contains(name) {
                names.insert(name.clone());
            }
        }
        scope.names = Some(Arc::new(names));
    }
    scopes
}

fn body_names(body: &[Statement], names: &mut Names) {
    for s in body {
        match &s.kind {
            StatementKind::Evaluate(e) => {
                for name in e.targets.iter().flatten() {
                    names.declare(name);
                }
                names.read(&e.expr);
                if let Some((name, indices)) = &e.indexed_target {
                    names.declare(name);
                    for index in indices {
                        names.read(index);
                    }
                }
                if let Some((name, steps)) = &e.path_target {
                    names.declare(name);
                    for step in steps {
                        match step {
                            PathStep::Dynamic(e) => names.read(e),
                            PathStep::Paren(args) | PathStep::Brace(args) => {
                                for e in args {
                                    names.read(e);
                                }
                            }
                            PathStep::Field(_) => {}
                        }
                    }
                }
                for e in e.list_target.iter().flatten().flatten() {
                    if let Some(root) = target_root(e) {
                        names.declare(root);
                    }
                    names.read(e);
                }
                if e.targets.is_empty()
                    && e.indexed_target.is_none()
                    && e.path_target.is_none()
                    && e.list_target.is_none()
                    && let Expr::Call(name, args) = &e.expr
                    && name == "load"
                {
                    // Literal load selections declare variables for dynamic
                    // writes. They do not participate in lexical sharing.
                    names
                        .variables
                        .extend(args.iter().skip(1).filter_map(|arg| match arg {
                            Expr::String(name) | Expr::DoubleQuoted(name)
                                if crate::files::valid_name(name) =>
                            {
                                Some(name.clone())
                            }
                            _ => None,
                        }));
                }
            }
            StatementKind::Declaration {
                names: declared, ..
            } => {
                for name in declared {
                    names.declare(name);
                }
            }
            StatementKind::If {
                branches,
                otherwise,
            } => {
                for branch in branches {
                    names.read(&branch.condition);
                    body_names(&branch.body, names);
                }
                body_names(otherwise, names);
            }
            StatementKind::Switch {
                subject,
                cases,
                otherwise,
            } => {
                names.read(subject);
                for case in cases {
                    names.read(&case.value);
                    body_names(&case.body, names);
                }
                body_names(otherwise, names);
            }
            StatementKind::While { condition, body } => {
                names.read(condition);
                body_names(body, names);
            }
            StatementKind::For { name, values, body } => {
                names.declare(name);
                names.read(values);
                body_names(body, names);
            }
            StatementKind::SerialBlock { arguments, body } => {
                for e in arguments {
                    names.read(e);
                }
                body_names(body, names);
            }
            StatementKind::Try {
                body,
                error,
                handler,
            } => {
                if let Some(name) = error {
                    names.declare(name);
                }
                body_names(body, names);
                body_names(handler, names);
            }
            StatementKind::Function(_)
            | StatementKind::Break
            | StatementKind::Continue
            | StatementKind::Return => {}
        }
    }
}

/// A lexical activation, retained by its children and by handles to them.
/// Only statically shared names have slots. An ordinary local, including
/// a returned handle, can be released as soon as the invocation finishes.
pub(in crate::engine) struct Frame {
    index: usize,
    parent: Option<Arc<Frame>>,
    bindings: BTreeMap<String, Arc<Slot>>,
    names: Arc<BTreeSet<String>>,
    /// Logical charge for the retained source and enclosing frame metadata.
    pub(in crate::engine) bytes: usize,
}
struct Slot;
impl Frame {
    fn chain(self: &Arc<Self>) -> impl Iterator<Item = &Arc<Self>> {
        std::iter::successors(Some(self), |f| f.parent.as_ref())
    }
}

pub(super) fn frame(
    program: &Program,
    index: usize,
    parent: Option<Arc<Frame>>,
) -> Option<Arc<Frame>> {
    let scope = program.scopes.get(&index)?;
    let names = scope.names.clone()?;
    let bindings = scope
        .shared
        .iter()
        .map(|(name, &owner)| {
            let slot = if owner == index {
                Arc::new(Slot)
            } else {
                parent
                    .as_ref()
                    .unwrap()
                    .chain()
                    .find(|f| f.index == owner)
                    .unwrap()
                    .bindings[name]
                    .clone()
            };
            (name.clone(), slot)
        })
        .collect();
    let bytes = parent.as_ref().map_or(program.source.len(), |f| f.bytes)
        + std::mem::size_of::<Frame>()
        + scope.shared.keys().map(|n| 64 + n.len()).sum::<usize>();
    Some(Arc::new(Frame {
        index,
        parent,
        bindings,
        names,
        bytes,
    }))
}

pub(in crate::engine) fn lookup_frame(functions: &FunctionState) -> Option<&Arc<Frame>> {
    if let Some(parent) = &functions.anonymous_parent {
        return Some(parent);
    }
    functions
        .lng
        .nested_frame()
        .as_ref()
        .filter(|f| Some(f.index) == functions.current)
}

pub(in crate::engine) fn resolve(
    functions: &FunctionState,
    name: &str,
) -> Option<ResolvedFunction> {
    let frame = lookup_frame(functions)?;
    let program = functions.scope.as_ref()?;
    frame.chain().find_map(|f| {
        let &index = program.scopes.get(&f.index)?.children.get(name)?;
        Some(ResolvedFunction {
            program: program.clone(),
            index,
            parent: Some(f.clone()),
        })
    })
}

/// A nested write may grow a slot in a suspended parent. Include parked
/// caller locals in the active workspace's usual admission checks.
pub(in crate::engine) fn caller_bytes(engine: &Engine) -> usize {
    if engine.functions.lng.nested_frame().is_some() {
        engine.functions.retained_bytes
    } else {
        0
    }
}

pub(in crate::engine) fn bound(handle: &FunctionHandle) -> Option<&ResolvedFunction> {
    handle.scope()?.downcast_ref()
}

pub(in crate::engine) fn handle(functions: &FunctionState, name: &str) -> Option<FunctionHandle> {
    let function = resolve(functions, name)?;
    let bytes = function.parent.as_ref()?.bytes;
    Some(FunctionHandle::nested(name, Arc::new(function), bytes))
}

pub(in crate::engine) fn qualified_name(function: &ResolvedFunction) -> String {
    let program = &function.program;
    let indices = std::iter::successors(Some(function.index), |index| {
        program.scopes.get(index)?.parent
    });
    let mut names: Vec<_> = indices
        .map(|index| program.function(index).unwrap().name.as_str())
        .collect();
    names.reverse();
    names.join("/")
}

pub(in crate::engine) fn check_write(engine: &mut Engine, name: &str) -> Result<()> {
    if engine
        .functions
        .lng
        .nested_frame()
        .as_ref()
        .is_some_and(|frame| !frame.names.contains(name))
    {
        return Err(engine.raise(
            format!(r#"Attempt to add "{name}" to a static workspace."#),
            "MATLAB:err_static_workspace_violation".into(),
        ));
    }
    Ok(())
}

fn target_root(mut expr: &Expr) -> Option<&str> {
    loop {
        match expr {
            Expr::Name(name) | Expr::Call(name, _) | Expr::CellContents(name, _) => {
                return Some(name);
            }
            Expr::Field(base, _)
            | Expr::DynamicField(base, _)
            | Expr::Index(base, _)
            | Expr::Brace(base, _) => expr = base,
            _ => return None,
        }
    }
}

/// Targets written by eval text, including the roots of subscripted writes.
pub(in crate::engine) fn check_eval_targets(engine: &mut Engine, e: &Evaluation) -> Result<()> {
    let names = e
        .targets
        .iter()
        .flatten()
        .map(String::as_str)
        .chain(e.indexed_target.iter().map(|(name, _)| name.as_str()))
        .chain(e.path_target.iter().map(|(name, _)| name.as_str()))
        .chain(
            e.list_target
                .iter()
                .flatten()
                .flatten()
                .filter_map(target_root),
        );
    for name in names {
        check_write(engine, name)?;
    }
    Ok(())
}

/// Read the captured slots without moving them between workspaces. An active
/// binding is usable only if it belongs to this exact lexical activation.
pub(in crate::engine) fn workspace(engine: &Engine, function: &ResolvedFunction) -> Result<Array> {
    let mut seen = BTreeSet::new();
    let mut values = BTreeMap::new();
    let active = engine.functions.lng.nested_frame().as_ref();
    if let Some(parent) = &function.parent {
        for frame in parent.chain() {
            for (name, slot) in &frame.bindings {
                if !seen.insert(name) {
                    continue;
                }
                engine.check_cancel()?;
                let in_workspace = active
                    .and_then(|f| f.bindings.get(name))
                    .is_some_and(|current| Arc::ptr_eq(current, slot));
                let array = if in_workspace {
                    engine.array(name)
                } else {
                    engine
                        .nested
                        .values
                        .get(&(Arc::as_ptr(slot) as usize))
                        .map(|(_, a)| a.clone())
                };
                if let Some(array) = array {
                    values.insert(name.clone(), Arc::new(array));
                }
            }
        }
    }
    let (names, slots) = values.into_iter().unzip();
    let fields = StructArray::from_slots(1, 1, names, slots, &engine.cancel)?;
    Ok(CellArray::from_shared(1, 1, vec![Arc::new(fields.into())], &engine.cancel)?.into())
}

/// While a scope is executing, its slots live in the evaluator's typed
/// workspace maps. On suspension, move them here. Every activation that
/// binds the same slot takes the same value, including after its owner has
/// returned. The engine owns parked values so a slot containing its own
/// closure cannot form a reference-counting cycle.
#[derive(Clone, Default)]
pub(in crate::engine) struct Store {
    values: BTreeMap<usize, (Weak<Slot>, Array)>,
    pub(in crate::engine) bytes: usize,
    sweep_at: usize,
    sweep_bytes_at: usize,
}

impl Store {
    pub(in crate::engine) fn park(
        &mut self,
        frame: &Frame,
        mut take: impl FnMut(&str) -> Option<Array>,
    ) {
        for (name, slot) in &frame.bindings {
            if let Some(array) = take(name) {
                self.bytes += array.retained_bytes();
                self.values
                    .insert(Arc::as_ptr(slot) as usize, (Arc::downgrade(slot), array));
            }
        }
    }

    pub(in crate::engine) fn activate(
        &mut self,
        frame: &Frame,
        mut put: impl FnMut(String, Array),
    ) {
        for (name, slot) in &frame.bindings {
            if let Some((_, array)) = self.values.remove(&(Arc::as_ptr(slot) as usize)) {
                self.bytes -= array.retained_bytes();
                put(name.clone(), array);
            }
        }
    }
}

pub(in crate::engine) fn resume(engine: &mut Engine, frame: Option<Arc<Frame>>) {
    if let Some(f) = &frame {
        let mut store = std::mem::take(&mut engine.nested);
        store.activate(f, |name, array| engine.publish_array(name, array));
        engine.nested = store;
    }
    *engine.functions.lng.nested_frame_mut() = frame;
}

pub(in crate::engine) fn suspend(engine: &mut Engine) -> Option<Arc<Frame>> {
    let f = engine.functions.lng.nested_frame_mut().take()?;
    let mut store = std::mem::take(&mut engine.nested);
    store.park(&f, |name| {
        let array = engine.array(name)?;
        engine.remove_array(name);
        Some(array)
    });
    engine.nested = store;
    Some(f)
}

impl Store {
    pub(in crate::engine) fn mark_handle<'a>(
        &'a self,
        h: &'a FunctionHandle,
        marked: &mut BTreeSet<usize>,
        pending: &mut Vec<&'a Array>,
    ) {
        if let Some((_, captured)) = h.anonymous_parts() {
            pending.extend(captured.iter().map(|(_, a)| a));
        }
        let parent = bound(h).and_then(|f| f.parent.as_ref()).or_else(|| {
            h.scope()?
                .downcast_ref::<super::HandleScope>()?
                .lookup
                .anonymous_parent
                .as_ref()
        });
        if let Some(parent) = parent {
            for frame in parent.chain() {
                for slot in frame.bindings.values() {
                    let id = Arc::as_ptr(slot) as usize;
                    if marked.insert(id)
                        && let Some((_, array)) = self.values.get(&id)
                    {
                        pending.push(array);
                    }
                }
            }
        }
    }

    pub(in crate::engine) fn sweep_due(&self) -> bool {
        self.values.len() >= self.sweep_at.max(256)
            || self.bytes >= self.sweep_bytes_at.max(1 << 20)
    }

    pub(in crate::engine) fn retain_marked(&mut self, marked: &BTreeSet<usize>) {
        self.values.retain(|id, (_, a)| {
            if marked.contains(id) {
                true
            } else {
                self.bytes -= a.retained_bytes();
                false
            }
        });
        self.sweep_at = self.values.len().saturating_mul(2);
        self.sweep_bytes_at = self.bytes.saturating_mul(2);
    }

    pub(in crate::engine) fn roots(&self) -> impl Iterator<Item = &Array> {
        self.values.values().map(|(_, a)| a)
    }
    pub(in crate::engine) fn sweep_unowned(&mut self) {
        self.values.retain(|_, (slot, a)| {
            if slot.strong_count() == 0 {
                self.bytes -= a.retained_bytes();
                false
            } else {
                true
            }
        });
    }
}
