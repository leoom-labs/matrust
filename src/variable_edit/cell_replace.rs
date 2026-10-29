//! Replace an existing cell slot with an immutable workspace value.
use super::{MAX_PATH, descend};
use crate::{Array, CellArray, Engine, Result};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct Binding {
    name: String,
    expected: Array,
}
impl Binding {
    pub fn new(name: String, expected: Array) -> Result<Self> {
        if !crate::files::valid_name(&name) || crate::syntax::keyword(&name) {
            return Err("Select an existing workspace variable by its exact name".into());
        }
        Ok(Self { name, expected })
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn expected(&self) -> &Array {
        &self.expected
    }
    fn current(&self, engine: &Engine) -> Result<Array> {
        let value = engine
            .array(&self.name)
            .ok_or("Replacement source or target no longer exists")?;
        if !value.same_snapshot(&self.expected) {
            return Err("Replacement source or target changed; select the values again".into());
        }
        Ok(value)
    }
}

#[derive(Clone, Debug)]
pub struct CellTarget {
    binding: Binding,
    path: Vec<usize>,
}
impl CellTarget {
    pub fn new(binding: Binding, path: Vec<usize>) -> Result<Self> {
        if path.is_empty() || path.len() > MAX_PATH || path.capacity() > MAX_PATH {
            return Err("Replacement requires an existing cell slot within 16 levels".into());
        }
        descend(binding.expected(), &path)?;
        Ok(Self { binding, path })
    }
    pub fn binding(&self) -> &Binding {
        &self.binding
    }
    pub fn path(&self) -> &[usize] {
        &self.path
    }
}

/// Shares the selected source payload; reconstructs only ancestor cell tables.
/// The caller's operation-generation guard supplements these snapshot checks.
pub fn replace(engine: &mut Engine, target: &CellTarget, source: &Binding) -> Result<Array> {
    replace_with(engine, target, source, &mut |_| Ok(()))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Validated,
    Ancestor,
    BeforeCommit,
}
fn replace_with(
    engine: &mut Engine,
    target: &CellTarget,
    source: &Binding,
    progress: &mut impl FnMut(Stage) -> Result<()>,
) -> Result<Array> {
    let cancel = engine.cancellation();
    crate::dsp::check_cancel(&cancel)?;
    // Both bindings are checked before constructing any ancestor table.
    let root = target.binding.current(engine)?;
    let mut updated = source.current(engine)?;
    descend(&root, &target.path)?;
    progress(Stage::Validated)?;
    crate::dsp::check_cancel(&cancel)?;
    let mut ancestors: Vec<(&CellArray, usize)> = Vec::new();
    ancestors
        .try_reserve_exact(target.path.len())
        .map_err(|_| "Cannot allocate bounded replacement path")?;
    let mut value = &root;
    for &index in &target.path {
        let Array::Cell(cell) = value else {
            return Err("Replacement path no longer refers to a cell".into());
        };
        ancestors.push((cell, index));
        value = cell.get_linear(index)?;
    }
    for (cell, index) in ancestors.into_iter().rev() {
        updated = cell.replaced(index, updated, &cancel)?.into();
        progress(Stage::Ancestor)?;
        crate::dsp::check_cancel(&cancel)?;
    }
    progress(Stage::BeforeCommit)?;
    crate::dsp::check_cancel(&cancel)?;
    target.binding.current(engine)?;
    source.current(engine)?;
    engine.merge_workspace(BTreeMap::from([(target.binding.name.clone(), updated)]))?;
    // Return the actual published root, including any admission normalization.
    engine
        .array(&target.binding.name)
        .ok_or_else(|| "Committed replacement target is missing".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    #[test]
    fn cancellation_after_actual_inner_table_rebuild_keeps_root_and_retry_sharing() {
        let mut engine = Engine::default();
        engine.execute("A=[1 2];C=cell(1,2);C{1}=A;C{2}=A;D=cell(1,2);D{1}=C;D{2}=C;Alias=D;S=char([0 55296 65]);").unwrap();
        let root = engine.array("D").unwrap();
        let source = engine.array("S").unwrap();
        let target =
            CellTarget::new(Binding::new("D".into(), root.clone()).unwrap(), vec![0, 1]).unwrap();
        let binding = Binding::new("S".into(), source.clone()).unwrap();
        let cancel = engine.cancellation();
        let mut rebuilt = 0;
        let result = replace_with(&mut engine, &target, &binding, &mut |stage| {
            if stage == Stage::Ancestor {
                rebuilt += 1;
                cancel.store(true, Ordering::Relaxed);
            }
            Ok(())
        });
        assert_eq!(rebuilt, 1);
        assert!(result.unwrap_err().contains("cancelled"));
        assert!(engine.array("D").unwrap().same_snapshot(&root));
        assert!(engine.array("Alias").unwrap().same_snapshot(&root));
        cancel.store(false, Ordering::Relaxed);
        let replaced = replace(&mut engine, &target, &binding).unwrap();
        assert!(replaced.same_snapshot(&engine.array("D").unwrap()));
        assert!(descend(&replaced, &[0, 1]).unwrap().same_snapshot(&source));
        assert!(
            descend(&replaced, &[1])
                .unwrap()
                .same_snapshot(descend(&root, &[1]).unwrap())
        );
        assert!(engine.array("Alias").unwrap().same_snapshot(&root));
    }
}
