//! `visdiff` glue. The desktop can replace the single presentation hook below.
use super::{
    Engine, Execution, Value,
    compare::{self, Comparison, ComparisonType},
};
use crate::Result;
use std::cell::RefCell;
use std::path::Path;

type Presentation = Box<dyn FnMut(&Path, &Path, &Comparison) -> Result<()>>;
thread_local! {
    static PRESENTATION: RefCell<Option<Presentation>> = RefCell::new(None);
}

/// A scoped, thread-local presentation hook. Private app engines and the
/// desktop worker cannot steal each other's results; CLI engines have no hook.
pub struct ComparisonPresentation {
    previous: Option<Presentation>,
    // A thread-local installation must be removed on its installing thread.
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl Drop for ComparisonPresentation {
    fn drop(&mut self) {
        PRESENTATION.with(|slot| *slot.borrow_mut() = self.previous.take());
    }
}
pub fn present_comparisons(
    present: impl FnMut(&Path, &Path, &Comparison) -> Result<()> + 'static,
) -> ComparisonPresentation {
    ComparisonPresentation {
        previous: PRESENTATION.with(|slot| slot.replace(Some(Box::new(present)))),
        _thread: std::marker::PhantomData,
    }
}

/// Return true after opening the Comparison Tool. The engine-only host leaves
/// presentation to the command window. No UI dependency belongs in this module.
pub fn open_comparison_tool(
    _engine: &mut Engine,
    left: &Path,
    right: &Path,
    comparison: &Comparison,
) -> Result<bool> {
    PRESENTATION.with(|slot| match slot.borrow_mut().as_mut() {
        Some(present) => {
            present(left, right, comparison)?;
            Ok(true)
        }
        None => Ok(false),
    })
}

pub(super) fn visdiff(
    engine: &mut Engine,
    args: &[Value],
    outputs: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if outputs != 0 {
        return Err("Too many output arguments.".into());
    }
    if args.len() < 2 {
        return Err("Not enough input arguments.".into());
    }
    if args.len() > 3 {
        return Err("Too many input arguments.".into());
    }
    let mut texts = Vec::new();
    for arg in args {
        texts.push(
            engine
                .text_value(arg.clone())
                .map_err(|_| "visdiff arguments must be text filenames or a comparison type")?,
        );
    }
    let kind = match texts.get(2).map(String::as_str) {
        None => ComparisonType::Auto,
        Some("text") => ComparisonType::Text,
        Some("binary") => ComparisonType::Binary,
        Some(other) => return Err(format!("Unsupported visdiff comparison type: {other}")),
    };
    let left = engine.file_path(&texts[0], "")?;
    let right = engine.file_path(&texts[1], "")?;
    let comparison = compare::compare_paths(&left, &right, kind, &engine.cancel)?;
    if !open_comparison_tool(engine, &left, &right, &comparison)? {
        let report = compare::render_report(&comparison, &texts[0], &texts[1], &engine.cancel)?;
        // Execution stores output records without their final line terminator.
        if !report.is_empty() {
            engine.emit(out, report.strip_suffix('\n').unwrap_or(&report).to_owned())?;
        }
    }
    Ok(Vec::new())
}
