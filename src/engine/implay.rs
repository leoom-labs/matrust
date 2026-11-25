//! Clean-room Video Viewer launch hook; the engine has no desktop dependency.
//! Behavior reference: https://www.mathworks.com/help/images/ref/videoviewer-app.html
use super::{Engine, Value};
use crate::{Array, Result};
use std::{cell::RefCell, marker::PhantomData, path::PathBuf, rc::Rc};

#[derive(Clone)]
pub enum Launch {
    Empty,
    File(PathBuf),
    Array {
        name: String,
        array: Array,
        fps: f64,
    },
}
/// [height, width, channels, frames], without copying shared array storage.
pub fn shape(a: &Array) -> Result<[usize; 4]> {
    let d = a.dims();
    if a.is_empty()
        || a.is_complex()
        || !matches!(a, Array::Numeric(_) | Array::Typed(_) | Array::Logical(_))
        || !matches!(
            a.class(),
            "uint8" | "uint16" | "double" | "single" | "logical"
        )
    {
        return Err(
            "implay: choose a real uint8, uint16, double, single or logical sequence".into(),
        );
    }
    let (channels, frames) = match d.as_ref() {
        [_, _] => (1, 1),
        [_, _, n] => (1, *n),
        [_, _, c, n] if matches!(*c, 1 | 3) => (*c, *n),
        _ => return Err("implay: expected H-by-W-by-F gray or H-by-W-by-3-by-F RGB".into()),
    };
    if a.len() > crate::matrix::MAX_ELEMENTS {
        return Err("implay: sequence exceeds the element limit".into());
    }
    Ok([d[0], d[1], channels, frames])
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
pub(super) fn open(e: &Engine, args: &[Value], outputs: usize) -> Result<Vec<Value>> {
    if args.len() > 2 || outputs != 0 {
        return Err("implay: expected zero, one or two inputs and no outputs".into());
    }
    if !OPEN.with(|s| s.borrow().is_some()) {
        return Err("implay requires the desktop".into());
    }
    let launch = if let Some(value) = args.first() {
        if let Ok(file) = e.text_value(value.clone()) {
            if args.len() != 1 {
                return Err("implay: a file uses its recorded frame rate".into());
            }
            Launch::File(e.file_path(&file, "")?)
        } else {
            let array = value.clone().array(&e.cancel)?;
            shape(&array)?;
            let fps = if let Some(fps) = args.get(1) {
                let a = fps.clone().array(&e.cancel)?;
                if a.len() != 1 || a.is_complex() {
                    return Err("implay: frame rate must be a positive finite scalar".into());
                }
                match a {
                    Array::Numeric(m) => m.data()[0],
                    Array::Typed(m) => m.real(0),
                    _ => f64::NAN,
                }
            } else {
                20.
            };
            if !fps.is_finite() || fps <= 0. {
                return Err("implay: frame rate must be a positive finite scalar".into());
            }
            let name = e
                .workspace()
                .into_iter()
                .find(|(_, a)| a.same_snapshot(&array))
                .map(|(name, _)| name)
                .ok_or("implay: store the sequence in a workspace variable first")?;
            Launch::Array { name, array, fps }
        }
    } else {
        Launch::Empty
    };
    OPEN.with(|s| s.borrow_mut().as_mut().unwrap()(launch))?;
    Ok(Vec::new())
}
