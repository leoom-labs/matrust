//! Bounded, atomic workspace metadata edits; numerical storage stays shared.
use matrust::{Array, Engine, Result, files, syntax};
use std::collections::BTreeMap;
use std::sync::atomic::Ordering;

pub enum Action {
    Rename(String),
    Remove { all: bool },
}
pub struct Request {
    pub targets: BTreeMap<String, Array>,
    pub action: Action,
}
pub enum Change {
    Renamed(String, String),
    Removed(Vec<String>),
}
impl Change {
    pub fn message(&self) -> String {
        match self {
            Self::Renamed(from, to) => format!("Renamed {from} to {to}"),
            Self::Removed(names) => format!("Removed {} workspace variable(s)", names.len()),
        }
    }
}
pub fn validate_name(name: &str) -> Result<()> {
    if !files::valid_name(name) {
        return Err(
            "Use 1..63 ASCII letters, digits or underscores, starting with a letter".into(),
        );
    }
    syntax::parse(&format!("{name}=0;"))
        .map(|_| ())
        .map_err(|_| "Reserved language keywords cannot be variable names".into())
}
pub fn perform(engine: &mut Engine, request: Request) -> Result<Change> {
    let cancel = engine.cancellation();
    let check = || {
        if cancel.load(Ordering::Relaxed) {
            Err("Workspace edit cancelled".to_owned())
        } else {
            Ok(())
        }
    };
    check()?;
    if request.targets.is_empty() || request.targets.len() > 10000 {
        return Err("Select 1..10000 variables".into());
    }
    for (name, expected) in &request.targets {
        check()?;
        if engine
            .array(name)
            .is_none_or(|value| !value.same_snapshot(expected))
        {
            return Err(format!(
                "Variable {name} changed; reopen the workspace action"
            ));
        }
    }
    match request.action {
        Action::Rename(to) => {
            if request.targets.len() != 1 {
                return Err("Select one variable to rename".into());
            }
            validate_name(&to)?;
            let from = request.targets.keys().next().unwrap();
            if engine.array(&to).is_some() {
                return Err("That variable name already exists; choose another name".into());
            }
            check()?;
            engine.rename_array(from, &to)?;
            Ok(Change::Renamed(from.clone(), to))
        }
        Action::Remove { all } => {
            if all && engine.workspace_count() != request.targets.len() {
                return Err("Workspace changed; review Clear workspace again".into());
            }
            check()?;
            // Commit the bounded map update together, with no partial cancellation.
            let names: Vec<_> = request.targets.into_keys().collect();
            for name in &names {
                engine.remove_array(name);
            }
            Ok(Change::Removed(names))
        }
    }
}
