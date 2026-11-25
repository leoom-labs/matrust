//! Desktop launch hooks for PDE Modeler. The engine has no UI dependency.
use super::{Engine, Value, image_basic};
use crate::Result;
use std::{cell::RefCell, marker::PhantomData, rc::Rc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Launch {
    Open,
    Close,
}

type Open = Box<dyn FnMut(Launch) -> Result<()>>;
thread_local! { static OPEN: RefCell<Option<Open>> = RefCell::new(None); }

pub struct Presentation(Option<Open>, PhantomData<Rc<()>>);

impl Drop for Presentation {
    fn drop(&mut self) {
        OPEN.with(|slot| slot.replace(self.0.take()));
    }
}

pub fn present(open: impl FnMut(Launch) -> Result<()> + 'static) -> Presentation {
    Presentation(
        OPEN.with(|slot| slot.replace(Some(Box::new(open)))),
        PhantomData,
    )
}

pub(super) fn open(engine: &mut Engine, args: &[Value], outputs: usize) -> Result<Vec<Value>> {
    engine.check_cancel()?;
    let usage = "pdeModeler: expected no inputs or close, and no outputs";
    if outputs != 0 {
        return Err(usage.into());
    }
    let launch = match args {
        [] => Launch::Open,
        [command]
            if image_basic::text(engine, command)
                .is_some_and(|s| s.eq_ignore_ascii_case("close")) =>
        {
            Launch::Close
        }
        _ => return Err(usage.into()),
    };
    OPEN.with(|slot| {
        let mut slot = slot.borrow_mut();
        slot.as_mut().ok_or("pdeModeler requires the desktop")?(launch)
    })?;
    Ok(Vec::new())
}
