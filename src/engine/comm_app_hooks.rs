//! Scoped communications app launch hook; the engine has no desktop dependency.
use crate::Result;
use std::{cell::RefCell, marker::PhantomData, rc::Rc};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Launch {
    Ber,
    Waveform,
}
type Open = Box<dyn FnMut(Launch) -> Result<()>>;
thread_local! { static OPEN: RefCell<Option<Open>> = RefCell::new(None); }
pub struct Presentation {
    previous: Option<Open>,
    thread: PhantomData<Rc<()>>,
}
impl Drop for Presentation {
    fn drop(&mut self) {
        OPEN.with(|s| *s.borrow_mut() = self.previous.take());
    }
}
pub fn present(open: impl FnMut(Launch) -> Result<()> + 'static) -> Presentation {
    Presentation {
        previous: OPEN.with(|s| s.replace(Some(Box::new(open)))),
        thread: PhantomData,
    }
}
pub(super) fn open(name: &str, argc: usize, outputs: usize) -> Result<()> {
    if argc != 0 || outputs != 0 {
        return Err(format!("{name}: expected no inputs or outputs"));
    }
    let launch = match name {
        "bertool" => Launch::Ber,
        "wirelessWaveformGenerator" => Launch::Waveform,
        _ => return Err(format!("Unknown communications app: {name}")),
    };
    OPEN.with(|s| match s.borrow_mut().as_mut() {
        Some(f) => f(launch),
        None => Err(format!("{name} requires the desktop")),
    })
}
