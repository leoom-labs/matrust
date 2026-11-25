//! Scoped host hooks for neural app launch and training progress. No UI dependency.
use super::nnet::train::EpochReport;
use crate::Result;
use std::{cell::RefCell, marker::PhantomData, ops::ControlFlow, rc::Rc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Launch {
    Picker,
    Fitting,
    PatternRecognition,
    Clustering,
    TimeSeries,
}
type Open = Box<dyn FnMut(Launch) -> Result<()>>;
type Progress = Box<dyn FnMut(EpochReport) -> ControlFlow<()>>;
thread_local! {
    static OPEN: RefCell<Option<Open>> = RefCell::new(None);
    static PROGRESS: RefCell<Option<Progress>> = RefCell::new(None);
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
pub fn present(open: impl FnMut(Launch) -> Result<()> + 'static) -> Presentation {
    Presentation {
        previous: OPEN.with(|s| s.replace(Some(Box::new(open)))),
        thread: PhantomData,
    }
}
pub struct Training {
    previous: Option<Progress>,
    thread: PhantomData<Rc<()>>,
}
impl Drop for Training {
    fn drop(&mut self) {
        PROGRESS.with(|s| *s.borrow_mut() = self.previous.take());
    }
}
pub fn progress(callback: impl FnMut(EpochReport) -> ControlFlow<()> + 'static) -> Training {
    Training {
        previous: PROGRESS.with(|s| s.replace(Some(Box::new(callback)))),
        thread: PhantomData,
    }
}
pub(super) fn report(report: EpochReport) -> ControlFlow<()> {
    PROGRESS.with(|s| {
        s.borrow_mut()
            .as_mut()
            .map_or(ControlFlow::Continue(()), |f| f(report))
    })
}
pub(super) fn open(name: &str, argc: usize, outputs: usize) -> Result<()> {
    if argc != 0 || outputs != 0 {
        return Err(format!("{name}: expected no inputs or outputs"));
    }
    let launch = match name {
        "nnstart" => Launch::Picker,
        "nftool" => Launch::Fitting,
        "nprtool" => Launch::PatternRecognition,
        "nctool" => Launch::Clustering,
        "ntstool" => Launch::TimeSeries,
        _ => return Err(format!("Unknown neural app: {name}")),
    };
    OPEN.with(|s| match s.borrow_mut().as_mut() {
        Some(f) => f(launch),
        None => Err(format!("{name} requires the desktop")),
    })
}
