//! Single-output assignments borrow their parsed name and avoid temporary maps.
use super::{Engine, Execution};
use crate::{Array, Result, syntax};

impl Engine {
    pub(super) fn validate_workspace_name(name: &str) -> Result<()> {
        if !crate::files::valid_name(name) || syntax::keyword(name) {
            return Err(format!(
                "Invalid workspace name '{name}': use an ASCII letter followed by letters, digits or underscores, at most 63 characters, and no reserved keyword"
            ));
        }
        Ok(())
    }

    pub(super) fn assign_workspace_array(
        &mut self,
        name: &str,
        array: Array,
        quiet: bool,
        execution: &mut Execution,
    ) -> Result<()> {
        Self::validate_workspace_name(name)?;
        let numeric = self.variables.get(name);
        let logical = self.logical_variables.get(name);
        let complex = self.complex_variables.get(name);
        let container = self.container_variables.get(name);
        let added = usize::from(
            numeric.is_none() && logical.is_none() && complex.is_none() && container.is_none(),
        );
        if self.workspace_count() + added > 10000 {
            return Err("Workspace exceeds 10000 variables; use clear".into());
        }
        // Public workspace maps can change between calls. Recount payloads rather
        // than trusting a cached total; only references and lengths are read.
        let replaced = numeric.map_or(0, |m| m.retained_bytes())
            + logical.map_or(0, |m| m.len())
            + complex.map_or(0, |m| m.bytes())
            + container.map_or(0, Array::retained_bytes);
        let used = self.workspace_bytes() - replaced + array.retained_bytes();
        if used > 256 * 1024 * 1024 {
            // An echo that is too large fails after the assignment, as an oversized emit does.
            let output = (!quiet).then(|| self.display_text(name, &array));
            self.merge_workspace([(name.to_owned(), array)].into())?;
            if let Some(text) = output {
                self.emit(execution, text?)?;
            }
            return Ok(());
        }
        self.check_cancel()?;
        let output = (!quiet).then(|| self.display_text(name, &array));
        self.container_variables.remove(name);
        match array {
            a @ (Array::Character(_)
            | Array::Cell(_)
            | Array::Graphics(_)
            | Array::Struct(_)
            | Array::Handle(_)
            | Array::Typed(_)) => {
                self.variables.remove(name);
                self.logical_variables.remove(name);
                self.complex_variables.remove(name);
                self.container_variables.insert(name.into(), a);
            }
            Array::Numeric(m) => {
                self.logical_variables.remove(name);
                self.complex_variables.remove(name);
                if let Some(existing) = self.variables.get_mut(name) {
                    *existing = m;
                } else {
                    self.variables.insert(name.into(), m);
                }
            }
            Array::Logical(m) => {
                self.variables.remove(name);
                self.complex_variables.remove(name);
                if let Some(existing) = self.logical_variables.get_mut(name) {
                    *existing = m;
                } else {
                    self.logical_variables.insert(name.into(), m);
                }
            }
            Array::Complex(m) => {
                self.variables.remove(name);
                self.logical_variables.remove(name);
                if let Some(existing) = self.complex_variables.get_mut(name) {
                    *existing = m;
                } else {
                    self.complex_variables.insert(name.into(), m);
                }
            }
        }
        if let Some(text) = output {
            self.emit(execution, text?)?;
        }
        Ok(())
    }
}
