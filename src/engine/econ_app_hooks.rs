//! Scoped Econometric Modeler launch hook. The engine has no UI dependency.
use crate::Result;
use std::{cell::RefCell, marker::PhantomData, rc::Rc};

type Open = Box<dyn FnMut() -> Result<()>>;
thread_local! { static OPEN: RefCell<Option<Open>> = RefCell::new(None); }

pub struct Presentation {
    previous: Option<Open>,
    thread: PhantomData<Rc<()>>,
}
impl Drop for Presentation {
    fn drop(&mut self) {
        OPEN.with(|slot| *slot.borrow_mut() = self.previous.take());
    }
}
pub fn present(open: impl FnMut() -> Result<()> + 'static) -> Presentation {
    Presentation {
        previous: OPEN.with(|slot| slot.replace(Some(Box::new(open)))),
        thread: PhantomData,
    }
}
pub(super) fn open(argc: usize, outputs: usize) -> Result<()> {
    if argc != 0 || outputs != 0 {
        return Err("econometricModeler: expected no inputs or outputs".into());
    }
    OPEN.with(|slot| match slot.borrow_mut().as_mut() {
        Some(open) => open(),
        None => Err("econometricModeler requires the desktop".into()),
    })
}
