//! Clean-room Registration Estimator launch hook; the engine has no desktop
//! dependency. Behavior reference:
//! https://www.mathworks.com/help/images/ref/registrationestimator-app.html
//! Also lists the workspace imref2d / transformation objects, so the desktop
//! app can recreate them in its private engines (like the MPC catalogue).
use super::registration::{self, Geo};
use super::{Engine, Value};
use crate::{Array, Result};
use std::{cell::RefCell, marker::PhantomData, rc::Rc, sync::Arc};

/// What `registrationEstimator` asked the desktop to open.
#[derive(Clone)]
pub enum Launch {
    Empty,
    /// Workspace names and their images: moving first, then fixed.
    Images {
        moving: (String, Array),
        fixed: (String, Array),
    },
}

/// [rows, columns, channels] of an image the app registers: real
/// H-by-W gray (uint8, uint16, int16, single, double) or H-by-W-by-3 RGB
/// (uint8, uint16, single, double); RGB registers on its rgb2gray version.
pub fn image_shape(a: &Array) -> Result<[usize; 3]> {
    const MESSAGE: &str = "registrationEstimator: choose a real H-by-W gray (uint8, uint16, \
                           int16, single or double) or H-by-W-by-3 RGB (uint8, uint16, single or \
                           double) image";
    if a.is_empty()
        || a.is_complex()
        || !matches!(a, Array::Numeric(_) | Array::Typed(_))
        || a.len() > crate::matrix::MAX_ELEMENTS
    {
        return Err(MESSAGE.into());
    }
    let d = a.dims();
    let shape = match d.as_ref() {
        [r, c] => [*r, *c, 1],
        [r, c, 1] => [*r, *c, 1],
        [r, c, 3] => [*r, *c, 3],
        _ => return Err(MESSAGE.into()),
    };
    let class_ok = match shape[2] {
        1 => matches!(
            a.class(),
            "uint8" | "uint16" | "int16" | "single" | "double"
        ),
        _ => matches!(a.class(), "uint8" | "uint16" | "single" | "double"),
    };
    if !class_ok {
        return Err(MESSAGE.into());
    }
    Ok(shape)
}

/// One workspace registration object (imref2d or a 2-D transformation).
#[derive(Clone)]
pub struct GeoVariable {
    pub name: String,
    pub class: &'static str,
    /// The workspace handle, to notice a reassigned name.
    pub handle: Array,
    geo: Arc<Geo>,
}
impl GeoVariable {
    pub fn is_imref(&self) -> bool {
        matches!(self.geo.as_ref(), Geo::Imref(_))
    }
    pub fn is_tform(&self) -> bool {
        matches!(self.geo.as_ref(), Geo::Tform(_))
    }
    /// Recreates the object under its name in another engine.
    pub fn install(&self, e: &mut Engine) -> Result<()> {
        let value = registration::new_object(e, self.geo.as_ref().clone())?;
        let array = value.array(&e.cancel)?;
        e.insert_array(&self.name, array)
    }
}

/// The workspace's imref2d and transformation objects, by name.
pub fn catalogue(e: &Engine) -> Vec<GeoVariable> {
    e.workspace()
        .into_iter()
        .filter_map(|(name, array)| {
            let geo = registration::geo_of(e, &Value::Container(array.clone()))?;
            Some(GeoVariable {
                name,
                class: geo.class(),
                handle: array,
                geo,
            })
        })
        .collect()
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
    if !matches!(args.len(), 0 | 2) || outputs != 0 {
        return Err(
            "registrationEstimator: expected no inputs or a moving and a fixed image, and no outputs"
                .into(),
        );
    }
    if !OPEN.with(|s| s.borrow().is_some()) {
        return Err("registrationEstimator requires the desktop".into());
    }
    let launch = if args.is_empty() {
        Launch::Empty
    } else {
        let workspace = e.workspace();
        let mut named = args.iter().zip(["moving", "fixed"]).map(|(value, role)| {
            let array = value.clone().array(&e.cancel)?;
            image_shape(&array)
                .map_err(|m| m.replacen("choose a", &format!("the {role} image must be a"), 1))?;
            let name = workspace
                .iter()
                .find(|(_, a)| a.same_snapshot(&array))
                .map(|(name, _)| name.clone())
                .ok_or_else(|| {
                    format!(
                        "registrationEstimator: store the {role} image in a workspace variable first"
                    )
                })?;
            Ok::<_, String>((name, array))
        });
        let moving = named.next().unwrap()?;
        let fixed = named.next().unwrap()?;
        Launch::Images { moving, fixed }
    };
    OPEN.with(|s| s.borrow_mut().as_mut().unwrap()(launch))?;
    Ok(Vec::new())
}
