//! Handle classes (`classdef Name < handle`). A handle object is a 1x1
//! struct with the user-object tag, the class name and an identity slot; the
//! slot's number names an entry of [`HandleTable`] that holds the properties
//! (laid out like a value object). Every copy of the struct (`b = a`, a
//! function argument, a cell element) shares the slot, so all of them see
//! one set of properties, and the slot's reference count tells when the last
//! copy is gone. Like containers.Map (objects.rs), unreachable entries are
//! freed at statement boundaries: entries no copy refers to at once,
//! reference cycles by the shared workspace mark once the table reaches its
//! sweep threshold (inside a function, the cycles made since it was
//! entered; see `cls_collect_function_cycles`; its callers then collect at
//! their next statement boundary). Freeing is silent: a `delete` method
//! runs only on an explicit `delete(h)` (documented).
use super::super::{Engine, Execution, Value};
use super::{ClassInfo, Program, ResolvedFunction, is_object, logical, object_class};
use crate::{Array, CharacterRow, Matrix, Result, StructArray, syntax::Token};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Weak},
};

/// Hidden field holding the identity slot of a handle object.
pub(in crate::engine) const HANDLE_FIELD: &str = "MatRustHandleId";
/// Any use of a deleted handle (documented text of the reference).
pub(in crate::engine) const DELETED: &str = "Invalid or deleted object.";
const MAX_HANDLES: usize = 65536;
const MAX_BYTES: usize = 256 * 1024 * 1024;
/// Stored bytes that trigger a collection at a statement boundary (then
/// twice the survivors), as for containers.Map.
const SWEEP_BYTES: usize = 64 * 1024 * 1024;

/// How isequal treats a pair with a handle object.
pub(in crate::engine) enum HandleEquality {
    Decided(bool),
    /// Compare these stored properties.
    Compare(Array, Array),
}

#[derive(Clone, Default)]
pub(in crate::engine) struct HandleTable {
    entries: BTreeMap<u64, HandleEntry>,
    next: u64,
    /// Entry count that triggers a sweep at the next statement boundary.
    sweep_at: usize,
    /// Stored bytes that trigger one.
    sweep_bytes_at: usize,
    bytes: usize,
    /// A collection ran inside a user function at this nesting level, where
    /// it could free only the cycles made since that function was entered:
    /// every caller (level below this; the top level is 0) collects at its
    /// next statement boundary. 0: none due.
    due_below: usize,
}
/// The running user function (`cls_collect_function_cycles`).
#[derive(Clone, Copy)]
pub(in crate::engine) struct HandleFrame {
    /// The last handle id that existed when it was entered.
    mark: u64,
    /// The evaluation depth its statements run at.
    depth: usize,
    /// Nesting level: 1 for a function called from the top level.
    level: usize,
}
#[derive(Clone)]
struct HandleEntry {
    slot: Weak<Array>,
    /// The properties, laid out like a value object; None once deleted.
    storage: Option<Array>,
    bytes: usize,
    /// Its delete methods are running (a nested delete(h) does nothing).
    deleting: bool,
    /// `addlistener` callbacks: event name and function handle, in order.
    listeners: Vec<(String, crate::FunctionHandle)>,
}

/// The identity slot of a handle object (any user object with that field).
fn slot(a: &Array) -> Option<&Arc<Array>> {
    let Array::Struct(s) = a else { return None };
    if s.len() != 1 {
        return None;
    }
    s.get(s.field_index(HANDLE_FIELD)?, 0).ok()
}
/// True for a handle object of a user class (deleted ones included).
pub(in crate::engine) fn is_handle_object(a: &Array) -> bool {
    is_object(a) && slot(a).is_some()
}
fn is_handle_value(v: &Value) -> bool {
    matches!(v, Value::Container(a) if is_handle_object(a))
}
/// Same object: the copies share one slot.
fn same_handle(a: &Array, b: &Array) -> bool {
    match (slot(a), slot(b)) {
        (Some(x), Some(y)) => Arc::ptr_eq(x, y),
        _ => false,
    }
}

impl Engine {
    /// The table entry of a handle object, if it is one this engine made.
    fn cls_handle_id(&self, a: &Array) -> Option<u64> {
        let slot = slot(a)?;
        let Array::Numeric(m) = slot.as_ref() else {
            return None;
        };
        let id = *m.data().first()? as u64;
        let entry = self.functions.classes.handles.entries.get(&id)?;
        (Weak::as_ptr(&entry.slot) == Arc::as_ptr(slot)).then_some(id)
    }
    /// Live handle objects (test hook for the statement-boundary sweep).
    #[doc(hidden)]
    pub fn cls_live_handles(&self) -> usize {
        self.functions.classes.handles.entries.len()
    }
    pub(in crate::engine) fn cls_handle_bytes(&self) -> usize {
        self.functions.classes.handles.bytes
    }
    /// A new handle object of `class` whose properties are `storage`.
    pub(super) fn cls_new_handle(&mut self, class: &str, storage: Array) -> Result<Array> {
        let bytes = storage.retained_bytes();
        let table = &self.functions.classes.handles;
        if table.entries.len() >= MAX_HANDLES || table.bytes.saturating_add(bytes) > MAX_BYTES {
            // Entries no copy refers to still count: free them before
            // failing. Mid-expression (constructor arguments and the object
            // being built are Rust temporaries) only the reference-count
            // sweep is safe. Cycles go at statement boundaries
            // (cls_sweep_due), long before the caps.
            self.cls_sweep_unowned();
        }
        let table = &mut self.functions.classes.handles;
        if table.entries.len() >= MAX_HANDLES {
            return Err(format!("At most {MAX_HANDLES} handle objects can exist"));
        }
        if table.bytes.saturating_add(bytes) > MAX_BYTES {
            return Err("Handle object properties exceed 256 MiB; use clear".into());
        }
        table.next += 1;
        let id = table.next;
        let slot = Arc::new(Array::Numeric(Matrix::scalar(id as f64)));
        table.bytes += bytes;
        table.entries.insert(
            id,
            HandleEntry {
                slot: Arc::downgrade(&slot),
                storage: Some(storage),
                bytes,
                deleting: false,
                listeners: Vec::new(),
            },
        );
        let names = vec![
            super::super::lang_more::TAG.into(),
            super::CLASS_FIELD.into(),
            HANDLE_FIELD.into(),
        ];
        let slots = vec![
            Arc::new(Array::Numeric(Matrix::scalar(super::USER_TAG))),
            Arc::new(Array::Character(CharacterRow::from_text(
                class,
                &self.cancel,
            )?)),
            slot,
        ];
        Ok(Array::Struct(StructArray::from_slots(
            1,
            1,
            names,
            slots,
            &self.cancel,
        )?))
    }
    /// The properties of an object: the value object itself, or the stored
    /// properties of a handle (an error once it is deleted).
    pub(in crate::engine) fn cls_storage<'a>(&'a self, a: &'a Array) -> Result<Cow<'a, Array>> {
        if slot(a).is_none() {
            return Ok(Cow::Borrowed(a));
        }
        self.cls_handle_id(a)
            .and_then(|id| self.functions.classes.handles.entries[&id].storage.as_ref())
            .map(Cow::Borrowed)
            .ok_or_else(|| DELETED.to_owned())
    }
    /// Replace the stored properties of a live handle.
    pub(super) fn cls_store(&mut self, a: &Array, storage: Array) -> Result<()> {
        let id = self.cls_handle_id(a).ok_or(DELETED)?;
        let bytes = storage.retained_bytes();
        let table = &self.functions.classes.handles;
        if table.bytes - table.entries[&id].bytes + bytes > MAX_BYTES {
            // As in cls_new_handle; `a` keeps this entry.
            self.cls_sweep_unowned();
        }
        let table = &mut self.functions.classes.handles;
        let entry = table.entries.get_mut(&id).expect("live handle");
        if entry.storage.is_none() {
            return Err(DELETED.into());
        }
        let total = table.bytes - entry.bytes + bytes;
        if total > MAX_BYTES {
            return Err("Handle object properties exceed 256 MiB; use clear".into());
        }
        entry.storage = Some(storage);
        entry.bytes = bytes;
        table.bytes = total;
        Ok(())
    }
    /// Add a listener to a live handle.
    pub(super) fn cls_add_listener(
        &mut self,
        a: &Array,
        event: &str,
        callback: crate::FunctionHandle,
    ) -> Result<()> {
        let id = self.cls_handle_id(a).ok_or(DELETED)?;
        let entry = self
            .functions
            .classes
            .handles
            .entries
            .get_mut(&id)
            .expect("live handle");
        if entry.storage.is_none() {
            return Err(DELETED.into());
        }
        if entry.listeners.len() >= 1024 {
            return Err("A handle object supports at most 1024 listeners".into());
        }
        entry.listeners.push((event.to_owned(), callback));
        Ok(())
    }
    /// The callbacks listening to `event` on a live handle, in order.
    pub(super) fn cls_listeners(
        &self,
        a: &Array,
        event: &str,
    ) -> Result<Vec<crate::FunctionHandle>> {
        let id = self.cls_handle_id(a).ok_or(DELETED)?;
        let entry = &self.functions.classes.handles.entries[&id];
        if entry.storage.is_none() {
            return Err(DELETED.into());
        }
        Ok(entry
            .listeners
            .iter()
            .filter(|(e, _)| e == event)
            .map(|(_, h)| h.clone())
            .collect())
    }
    pub(super) fn cls_is_valid(&self, a: &Array) -> bool {
        self.cls_handle_id(a).is_some_and(|id| {
            self.functions.classes.handles.entries[&id]
                .storage
                .is_some()
        })
    }
    /// `delete(h)` / `h.delete()`: the delete methods of the class and then
    /// of its superclasses (each class's own, once) run on the live object,
    /// then it is invalid for every copy. An error in a delete method becomes
    /// a warning and the object is deleted anyway (as documented).
    pub(in crate::engine) fn cls_delete(
        &mut self,
        object: &Value,
        out: &mut Execution,
    ) -> Result<()> {
        let Value::Container(a) = object else {
            unreachable!("handle object")
        };
        let Some(id) = self.cls_handle_id(a) else {
            return Ok(()); // Deleting a deleted handle does nothing.
        };
        let entry = &self.functions.classes.handles.entries[&id];
        if entry.deleting || entry.storage.is_none() {
            return Ok(());
        }
        self.functions
            .classes
            .handles
            .entries
            .get_mut(&id)
            .expect("live handle")
            .deleting = true;
        let class_name = object_class(a).expect("user object");
        let result = self.cls_run_destructors(&class_name, object, out);
        // Delete any UI figures registered to this app via AppBase::registerApp.
        let registered_figs: Vec<usize> = self
            .ui
            .registered_apps
            .iter()
            .filter_map(|(&fig_key, app_arr)| {
                (self.cls_handle_id(app_arr) == Some(id)).then_some(fig_key)
            })
            .collect();
        for fig_key in registered_figs {
            self.ui.registered_apps.remove(&fig_key);
            self.ui.delete_subtree(fig_key);
        }
        if let Some(entry) = self.functions.classes.handles.entries.get_mut(&id) {
            entry.deleting = false;
            entry.storage = None;
            self.functions.classes.handles.bytes -= entry.bytes;
            entry.bytes = 0;
        }
        if let Err(message) = result {
            if self.check_cancel().is_err() {
                return Err(message);
            }
            self.emit(
                out,
                format!(
                    "Warning: The following error was caught while executing '{class_name}' class destructor:\n{message}"
                ),
            )?;
        }
        Ok(())
    }
    fn cls_run_destructors(
        &mut self,
        class_name: &str,
        object: &Value,
        out: &mut Execution,
    ) -> Result<()> {
        let (program, class) = self.cls_require(class_name, out)?;
        let mut programs: Vec<(Arc<Program>, Arc<ClassInfo>)> = vec![(program, class.clone())];
        for ancestor in class
            .ancestors
            .iter()
            .filter(|a| *a != "handle" && *a != super::APPBASE_CLASS)
        {
            programs.push(self.cls_require(ancestor, out)?);
        }
        for (program, info) in programs {
            if let Some(method) = info.methods.get("delete")
                && method.owner == info.name
                && !method.is_static
                && !method.is_abstract
            {
                let (target, index) = self.cls_target(&program, method)?;
                let no_outputs = target.function(index).is_none_or(|f| f.outputs.is_empty());
                let requested = usize::from(!no_outputs);
                self.invoke_function(
                    ResolvedFunction::at(target, index),
                    vec![object.clone()],
                    requested,
                    None,
                    out,
                )?;
            }
        }
        Ok(())
    }
    /// `a == b`, `a ~= b` with a handle object: identity (copies of one object
    /// are equal, deleted ones too). None when neither side is a handle.
    pub(in crate::engine) fn cls_equality(
        &mut self,
        op: &Token,
        left: &Value,
        right: &Value,
    ) -> Result<Option<Value>> {
        if !matches!(op, Token::Equal | Token::NotEqual)
            || !(is_handle_value(left)
                || is_handle_value(right)
                || [left, right].into_iter().any(
                    |v| matches!(v, Value::Container(a) if super::events::enum_name(a).is_some()),
                ))
        {
            return Ok(None);
        }
        let name = if *op == Token::Equal { "eq" } else { "ne" };
        self.cls_identity(name, left, right).map(Some)
    }
    /// eq / ne of two handle objects; anything else is an undefined function.
    pub(super) fn cls_identity(
        &mut self,
        name: &str,
        left: &Value,
        right: &Value,
    ) -> Result<Value> {
        let member = |v: &Value| match v {
            Value::Container(a) => super::events::enum_name(a).map(|n| (object_class(a), n)),
            _ => None,
        };
        let member_text = |v: &Value| match v {
            Value::Container(Array::Character(_)) | Value::Text(_) | Value::QuotedString(_) => {
                super::text(v)
            }
            _ => None,
        };
        match (member(left), member(right)) {
            (Some(a), Some(b)) => return Ok(logical((a == b) == (name == "eq"))),
            (Some((_, a)), None) | (None, Some((_, a))) => {
                if let Some(t) = member_text(if member(left).is_some() { right } else { left }) {
                    return Ok(logical((a == t) == (name == "eq")));
                }
            }
            _ => {}
        }
        match (left, right) {
            (Value::Container(a), Value::Container(b))
                if is_handle_object(a) && is_handle_object(b) =>
            {
                let same = same_handle(a, b) && object_class(a) == object_class(b);
                Ok(logical(same == (name == "eq")))
            }
            _ => {
                let class = [left, right]
                    .into_iter()
                    .find_map(|v| match v {
                        Value::Container(a) => object_class(a),
                        _ => None,
                    })
                    .unwrap_or_else(|| "double".into());
                Err(self.raise(
                    format!("Undefined function '{name}' for input arguments of type '{class}'."),
                    "MATLAB:UndefinedFunction".into(),
                ))
            }
        }
    }
    /// isequal with a handle object: the same object, or two live objects of
    /// one class whose properties are equal (documented: handles compare by
    /// property values). None when neither is a handle.
    pub(in crate::engine) fn cls_handles_equal(
        &self,
        a: &Array,
        b: &Array,
    ) -> Option<HandleEquality> {
        if !is_handle_object(a) && !is_handle_object(b) {
            return None;
        }
        if same_handle(a, b) {
            return Some(HandleEquality::Decided(true));
        }
        if !(is_handle_object(a) && is_handle_object(b)) || object_class(a) != object_class(b) {
            return Some(HandleEquality::Decided(false));
        }
        let (Ok(x), Ok(y)) = (self.cls_storage(a), self.cls_storage(b)) else {
            return Some(HandleEquality::Decided(false));
        };
        Some(HandleEquality::Compare(x.into_owned(), y.into_owned()))
    }

    /// Free entries no copy refers to any more (each freed entry may free
    /// others it referred to). Safe anywhere: an object held only by an
    /// expression temporary still counts.
    pub(in crate::engine) fn cls_sweep_unowned(&mut self) {
        let table = &mut self.functions.classes.handles;
        loop {
            let before = table.entries.len();
            // Dropping an entry's storage can release other slots: collect
            // first, drop after the retain.
            let mut freed = Vec::new();
            table.entries.retain(|_, entry| {
                let keep = entry.slot.strong_count() > 0;
                if !keep {
                    table.bytes -= entry.bytes;
                    freed.push(entry.storage.take());
                }
                keep
            });
            drop(freed);
            if table.entries.len() == before {
                break;
            }
        }
    }
    /// After a collection that also frees cycles (or could not run here):
    /// the next one runs once the table doubles. The reference-count sweep
    /// above runs at every function return and must not move this, or
    /// uncollected cycles keep the threshold ahead of the table. Inside a
    /// function the collection frees only that function's own cycles
    /// (cls_collect_function_cycles), so its callers' levels are due: each
    /// collects at its next statement boundary, the top level by the full
    /// mark. Without that, a loop calling a function that drops a cycle per
    /// call crosses every threshold inside the callee and never collects.
    pub(in crate::engine) fn cls_reset_sweep_threshold(&mut self) {
        let level = self.cls_level();
        let table = &mut self.functions.classes.handles;
        table.sweep_at = table.entries.len().saturating_mul(2).max(256);
        table.sweep_bytes_at = table.bytes.saturating_mul(2).max(SWEEP_BYTES);
        table.due_below = level;
    }
    /// At a statement boundary: the handle table reached its sweep
    /// threshold (entries or stored bytes), or a callee's collection left
    /// this level due, so the shared collection (objects.rs `obj_collect`)
    /// runs: a mark from the workspace roots, which also frees reference
    /// cycles.
    pub(in crate::engine) fn cls_sweep_due(&self) -> bool {
        let table = &self.functions.classes.handles;
        !table.entries.is_empty()
            && (table.entries.len() >= table.sweep_at.max(256)
                || table.bytes >= table.sweep_bytes_at.max(SWEEP_BYTES)
                || self.cls_level() < table.due_below)
    }
    /// Nesting level of the running user function (0: top level).
    fn cls_level(&self) -> usize {
        self.functions.classes.function_frame.map_or(0, |f| f.level)
    }
    /// On entry to a user function (user_functions.rs): record the handle
    /// objects that exist, the evaluation depth of its statements and its
    /// nesting level. Returns the caller's record for
    /// [`Self::cls_leave_function`].
    pub(in crate::engine) fn cls_enter_function(&mut self) -> Option<HandleFrame> {
        let frame = HandleFrame {
            mark: self.functions.classes.handles.next,
            depth: self.evaluation_depth,
            level: self.cls_level() + 1,
        };
        self.functions.classes.function_frame.replace(frame)
    }
    pub(in crate::engine) fn cls_leave_function(&mut self, caller: Option<HandleFrame>) {
        self.functions.classes.function_frame = caller;
    }
    pub(in crate::engine) fn cls_in_function(&self) -> bool {
        self.functions.classes.function_frame.is_some()
    }
    /// Between two statements of a running user function, where the full
    /// mark cannot run (objects.rs `obj_collect`): expressions of its callers
    /// may hold objects as Rust temporaries the mark does not see. Those
    /// temporaries were made before the call, so they reach objects made
    /// since only through older handle objects or containers.Map/inputParser
    /// objects. Every such older object still referenced counts as a root,
    /// and handle objects made since the call that no root reaches
    /// (reference cycles) are freed. The function's own expressions hold
    /// nothing between its statements; text run by eval inside one of them
    /// runs deeper and is skipped.
    pub(in crate::engine) fn cls_collect_function_cycles(&mut self) {
        let Some(HandleFrame { mark, depth, .. }) = self.functions.classes.function_frame else {
            return;
        };
        let table = &self.functions.classes.handles;
        if self.evaluation_depth != depth || table.entries.range(mark + 1..).next().is_none() {
            return;
        }
        let mut marked_handles = BTreeSet::new();
        let mut pending: Vec<&Array> = self.obj_roots().collect();
        for (id, entry) in table.entries.range(..=mark) {
            if entry.slot.strong_count() > 0 {
                marked_handles.insert(*id);
                pending.extend(entry.storage.as_ref());
            }
        }
        let mut marked = BTreeSet::new();
        self.obj_mark_referenced(&mut marked, &mut pending);
        if !self.obj_mark(
            pending,
            &mut marked,
            &mut marked_handles,
            &mut BTreeSet::new(),
        ) {
            return; // Cancelled: never sweep a partially marked graph.
        }
        let table = &mut self.functions.classes.handles;
        let mut freed = Vec::new();
        table.entries.retain(|id, entry| {
            let keep = *id <= mark || marked_handles.contains(id);
            if !keep {
                table.bytes -= entry.bytes;
                freed.push(entry.storage.take());
            }
            keep
        });
        drop(freed);
        self.cls_sweep_unowned();
    }
    /// Arrays the class registry holds during a run (cached property
    /// defaults, an object handed to a superclass constructor): roots of the
    /// shared sweep, since a default may be a handle object.
    pub(in crate::engine) fn cls_roots(&self) -> impl Iterator<Item = &Array> {
        let classes = &self.functions.classes;
        classes
            .values
            .values()
            .flat_map(|v| v.iter().flatten())
            .chain(classes.super_preset.iter())
    }
    pub(in crate::engine) fn cls_handles_live(&self) -> bool {
        !self.functions.classes.handles.entries.is_empty()
    }
    /// For the shared mark phase (objects.rs): the entry of a handle object
    /// and its stored properties, to trace further.
    /// A handle object's id and what it keeps alive: its stored properties
    /// and the values its listener callbacks captured.
    pub(in crate::engine) fn cls_mark(&self, a: &Array) -> Option<(u64, Vec<&Array>)> {
        let id = self.cls_handle_id(a)?;
        let entry = &self.functions.classes.handles.entries[&id];
        let mut children: Vec<&Array> = entry.storage.iter().collect();
        for (_, callback) in &entry.listeners {
            if let Some((_, captured)) = callback.anonymous_parts() {
                children.extend(captured.iter().map(|(_, a)| a));
            }
            if let Some((app, cb, _)) = callback.app_callback_parts() {
                children.push(app);
                if let Some((_, captured)) = cb.anonymous_parts() {
                    children.extend(captured.iter().map(|(_, a)| a));
                }
            }
        }
        Some((id, children))
    }
    /// After a full mark from the workspace roots: free every entry not
    /// reached (reference cycles included).
    pub(in crate::engine) fn cls_retain_marked(&mut self, marked: &BTreeSet<u64>) {
        let table = &mut self.functions.classes.handles;
        let mut freed = Vec::new();
        table.entries.retain(|id, entry| {
            let keep = marked.contains(id);
            if !keep {
                table.bytes -= entry.bytes;
                freed.push(entry.storage.take());
            }
            keep
        });
        drop(freed);
        self.cls_sweep_unowned();
        self.cls_reset_sweep_threshold();
    }
    /// Display text of a deleted handle object.
    pub(super) fn cls_deleted_text(&self, a: &Array) -> Option<String> {
        if !is_handle_object(a) || self.cls_is_valid(a) {
            return None;
        }
        Some(format!(
            "  handle to deleted {}\n",
            object_class(a).unwrap_or_default()
        ))
    }
}
