//! Current-workspace deletion, plus `clear all`, `clear functions`,
//! `clear classes`, `clear java` and `clear mex`.
//! Figures, the search path, the current folder and built-in functions stay.
use super::{Engine, Execution, Expr};
use crate::{Array, Result};
use std::collections::BTreeSet;

/// A sole argument that selects a clear mode, when it is not a variable.
pub(super) fn is_clear_mode(name: &str) -> bool {
    clear_mode(name).is_some()
}

enum ClearMode {
    All,
    Functions,
    Classes,
    Java,
    Mex,
}

fn clear_mode(name: &str) -> Option<ClearMode> {
    match name {
        "all" | "-all" => Some(ClearMode::All),
        "functions" => Some(ClearMode::Functions),
        "classes" => Some(ClearMode::Classes),
        "java" => Some(ClearMode::Java),
        "mex" => Some(ClearMode::Mex),
        _ => None,
    }
}

pub(super) fn is_variable_glob(pattern: &str) -> bool {
    pattern.contains(['*', '?'])
}

/// Match the entire variable name. Only `*` and `?` have special meaning.
pub(super) fn variable_glob(pattern: &str) -> Result<regex::Regex> {
    let mut escaped = String::from(r"\A");
    let mut start = 0;
    for (index, wildcard) in pattern.match_indices(['*', '?']) {
        escaped.push_str(&regex::escape(&pattern[start..index]));
        escaped.push_str(if wildcard == "*" { ".*" } else { "." });
        start = index + 1;
    }
    escaped.push_str(&regex::escape(&pattern[start..]));
    escaped.push_str(r"\z");
    regex::Regex::new(&escaped).map_err(|e| e.to_string())
}

pub(super) fn clear_glob_matches(engine: &Engine, globs: &[regex::Regex]) -> Result<Vec<String>> {
    let mut matched = Vec::new();
    if globs.is_empty() {
        return Ok(matched);
    }
    for name in engine
        .variables
        .keys()
        .chain(engine.logical_variables.keys())
        .chain(engine.complex_variables.keys())
        .chain(engine.container_variables.keys())
    {
        engine.check_cancel()?;
        if globs.iter().any(|glob| glob.is_match(name)) {
            matched.push(name.clone());
        }
    }
    Ok(matched)
}

impl Engine {
    pub(super) fn clear_variables_command(
        &mut self,
        args: &[Expr],
        out: &mut Execution,
    ) -> Result<()> {
        if args.len() > 255 {
            return Err("clear supports at most 255 literal variable names".into());
        }
        // `clear import`: the import list of the base workspace, no variable.
        if let [Expr::String(name) | Expr::DoubleQuoted(name)] = args
            && name == "import"
            && !self.contains_array(name)
        {
            return self.imp_clear_command();
        }
        // `clear all` / `-all` / `functions` / `classes` / `java` / `mex`.
        // A workspace variable of that name still wins, so it falls through.
        if let [Expr::String(name) | Expr::DoubleQuoted(name)] = args
            && !self.contains_array(name)
            && let Some(mode) = clear_mode(name)
        {
            return apply_clear_mode(self, mode, out);
        }
        let mut all_variables = args.is_empty();
        // Classify every item against the ORIGINAL workspace. In particular,
        // duplicate variables named all/variables must never become modes after
        // an earlier occurrence is removed. No array/string payload is cloned.
        for arg in args {
            self.check_cancel()?;
            let (Expr::String(name) | Expr::DoubleQuoted(name)) = arg else {
                return Err("clear requires literal variable names; use clear A B or clear('A','B'). Dynamic arguments and expressions are unsupported".into());
            };
            if self.contains_array(name) {
                Self::validate_workspace_name(name)?;
                continue;
            }
            match name.as_str() {
                "variables" if args.len() == 1 => all_variables = true,
                "variables" => return Err(
                    "clear variables must be used alone unless variables is a workspace variable"
                        .into(),
                ),
                "global" => {
                    return Err(
                        "clear global must come first: use clear global or clear global A B".into(),
                    );
                }
                "all" | "-all" | "functions" | "classes" | "import" | "java" | "mex" => {
                    return Err(format!(
                        "clear {name} must be used alone unless {name} is a workspace variable"
                    ));
                }
                _ => {
                    Self::validate_workspace_name(name)?;
                    if self.functions.contains_retained_function(name) {
                        return Err(format!(
                            "clear cannot unload retained function '{name}'; only variable bindings are supported"
                        ));
                    }
                }
            }
        }
        self.check_cancel()?;
        // Deliberately noncancellable commit: <=255 removals or the bounded
        // workspace maps, with no fallible validation after the first mutation.
        // Dropping the last shared owner can deallocate its existing buffer.
        if all_variables {
            self.clear_arrays();
        } else {
            for arg in args {
                if let Expr::String(name) | Expr::DoubleQuoted(name) = arg {
                    self.remove_array(name);
                }
            }
        }
        Ok(())
    }
}

/// One mode, after the argument check. `clear all` drops variables, globals,
/// persistents, file caches, class definitions and, from the command prompt
/// only, the import list. A function that is running stays loaded: its
/// program is not in those caches and its persistent variables stay.
fn apply_clear_mode(engine: &mut Engine, mode: ClearMode, out: &mut Execution) -> Result<()> {
    engine.check_cancel()?;
    match mode {
        ClearMode::All => {
            // The store is updated only on sync; copy the latest values, then drop them.
            engine.lng_sync()?;
            engine.lng_clear_all_globals();
            engine.lng_clear_all_persistents();
            engine.functions.lng_clear_file_caches();
            engine.functions.classes.clear_cached(&BTreeSet::new());
            // MathWorks clear page, `all`, import list: "From command prompt only".
            if at_command_prompt(engine) {
                engine.functions.imp_clear();
            }
            engine.clear_arrays();
            engine.cls_sweep_unowned();
        }
        ClearMode::Functions => {
            engine.lng_sync()?;
            engine.lng_clear_all_persistents();
            engine.functions.lng_clear_file_caches();
        }
        ClearMode::Classes => {
            engine.lng_sync()?;
            engine.clear_arrays();
            // The names were unbound with the variables. A later `global g`
            // must load the object that is still in the store.
            engine.lng_unbind_globals();
            engine.cls_sweep_unowned();
            let live = live_user_classes(engine);
            let keep = engine.functions.classes.classes_to_keep(&live);
            for name in &live {
                engine.lng_warn(
                    &format!(
                        "Objects of '{name}' class exist. Cannot clear this class or any of its superclasses."
                    ),
                    "MATLAB:class:clear",
                    out,
                )?;
            }
            engine.functions.lng_clear_class_caches(&keep);
            engine.functions.classes.clear_cached(&keep);
        }
        // Accepted, and they change nothing. There is no Java or MEX state.
        ClearMode::Java | ClearMode::Mex => {}
    }
    Ok(())
}

/// No function, script file or document runs: the code was typed at the
/// Command Window or given as -batch text.
fn at_command_prompt(engine: &Engine) -> bool {
    engine.functions.lng.ctx_parked().next().is_none()
        && engine.functions.active_path().is_none()
        && engine.document_path.is_none()
}

/// Class names of user objects still reachable after the workspace variables
/// are gone (globals, persistents, caller frames, handle properties).
fn live_user_classes(engine: &Engine) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut seen = BTreeSet::new();
    note_user_classes(engine, &mut names, &mut seen);
    names
}

fn note_user_classes<'a>(
    engine: &'a Engine,
    names: &mut BTreeSet<String>,
    seen: &mut BTreeSet<usize>,
) {
    let mut pending: Vec<(&'a Array, u32)> = engine.obj_roots().map(|array| (array, 0)).collect();
    while let Some((array, depth)) = pending.pop() {
        if depth > 32 || !seen.insert(array as *const Array as usize) {
            continue;
        }
        if let Some(name) = super::classdef::object_class(array) {
            names.insert(name);
        }
        // A handle object's properties live in the handle table, not in its facade.
        if let Some((_, children)) = engine.cls_mark(array) {
            pending.extend(children.into_iter().map(|child| (child, depth + 1)));
            continue;
        }
        match array {
            Array::Cell(cell) => pending.extend(
                (0..cell.len())
                    .filter_map(|i| cell.get_linear(i).ok())
                    .map(|item| (item.as_ref(), depth + 1)),
            ),
            Array::Struct(structure) => pending.extend(
                (0..structure.slot_count())
                    .filter_map(|i| structure.slot(i).ok())
                    .map(|item| (item.as_ref(), depth + 1)),
            ),
            Array::Handle(handle) => {
                if let Some((_, captured)) = handle.anonymous_parts() {
                    pending.extend(captured.iter().map(|(_, value)| (value, depth + 1)));
                }
                if let Some((app, cb, _)) = handle.app_callback_parts() {
                    pending.push((app, depth + 1));
                    if let Some((_, captured)) = cb.anonymous_parts() {
                        pending.extend(captured.iter().map(|(_, value)| (value, depth + 1)));
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn clear_preflight_cancellation_and_argument_limit_leave_bindings_shared() {
        let mut engine = Engine::default();
        engine
            .execute("A=[1 2 3];B=A;mask=logical([0 1]);")
            .unwrap();
        let original = engine.workspace();
        let pointer = engine.variables["A"].data().as_ptr();
        engine.cancel.store(true, Ordering::Relaxed);
        let mut out = super::super::Execution::default();
        for args in [vec![], vec![Expr::String("A".into())]] {
            assert!(engine.clear_variables_command(&args, &mut out).is_err());
        }
        engine.cancel.store(false, Ordering::Relaxed);
        let names = vec![Expr::String("A".into()); 256];
        assert!(engine.clear_variables_command(&names, &mut out).is_err());
        assert_eq!(engine.variables["A"].data().as_ptr(), pointer);
        for (name, value) in original {
            assert!(value.same_snapshot(&engine.array(&name).unwrap()));
        }
    }
}
