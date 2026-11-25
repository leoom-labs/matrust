//! UI-independent, scoped desktop presentation hook for `appdesigner`.
//! Mirrors `fuzzy_app_hooks.rs` so the engine stays independent of egui.
use super::{AppLoadResult, open_app_file};
use crate::Result;
use crate::engine::{Engine, Value};
use std::path::PathBuf;
use std::{cell::RefCell, marker::PhantomData, rc::Rc};

/// Launch payload sent from the engine worker to the desktop App Designer window.
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum AppDesignerLaunch {
    /// `appdesigner` with no arguments: open a new app named `app1`.
    New,
    /// `appdesigner('MyApp.m')` or `appdesigner('Legacy.mlapp')`.
    OpenFile {
        path: PathBuf,
        result: AppLoadResult,
    },
}

type Open = Box<dyn FnMut(AppDesignerLaunch) -> Result<()>>;

thread_local! {
    static OPEN: RefCell<Option<Open>> = RefCell::new(None);
}

pub struct Presentation {
    previous: Option<Open>,
    thread: PhantomData<Rc<()>>,
}

impl Drop for Presentation {
    fn drop(&mut self) {
        OPEN.with(|s| *s.borrow_mut() = self.previous.take());
    }
}

pub fn present(open: impl FnMut(AppDesignerLaunch) -> Result<()> + 'static) -> Presentation {
    Presentation {
        previous: OPEN.with(|s| s.replace(Some(Box::new(open)))),
        thread: PhantomData,
    }
}

pub(crate) fn open(engine: &mut Engine, args: &[Value], count: usize) -> Result<()> {
    if args.len() > 1 || count != 0 {
        return Err("appdesigner: expected zero or one input and no outputs".into());
    }
    if !OPEN.with(|s| s.borrow().is_some()) {
        return Err("appdesigner requires the desktop".into());
    }
    let launch = match args.first() {
        None => AppDesignerLaunch::New,
        Some(v) => {
            let text = engine.envx_string(v, "appdesigner")?;
            let raw = PathBuf::from(&text);
            let path = if raw.is_absolute() {
                raw
            } else if let Some(dir) = engine.current_folder() {
                dir.join(raw)
            } else {
                raw
            };
            let result = open_app_file(&path)?;
            AppDesignerLaunch::OpenFile { path, result }
        }
    };
    OPEN.with(|s| s.borrow_mut().as_mut().unwrap()(launch))
}
