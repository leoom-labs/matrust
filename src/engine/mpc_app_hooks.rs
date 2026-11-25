//! Scoped MPC Designer presentation. Controller seeds carry immutable engine
//! values, never a UI dependency or a handle from another engine's object table.
use super::{Engine, Value, mpc, objects};
use crate::{Array, Result};
use std::{cell::RefCell, collections::BTreeMap, marker::PhantomData, rc::Rc, sync::Arc};

#[derive(Clone)]
pub struct Source {
    pub name: String,
    pub plant: Array,
    controller: Option<Arc<mpc::MpcObject>>,
}
impl Source {
    pub fn plant(name: String, plant: Array) -> Result<Self> {
        if !matches!(super::lti_class(&plant), Some("ss" | "tf" | "zpk")) {
            return Err("MPC Designer: choose a workspace ss, tf, zpk or mpc variable".into());
        }
        Ok(Self {
            name,
            plant,
            controller: None,
        })
    }
    pub fn is_controller(&self) -> bool {
        self.controller.is_some()
    }
    pub fn install(&self, e: &mut Engine) -> Result<()> {
        let a = if let Some(m) = &self.controller {
            objects::mpc_new(e, m.as_ref().clone())?.array(&e.cancel)?
        } else {
            self.plant.clone()
        };
        e.insert_array(&self.name, a)
    }
    pub fn properties(&self, e: &Engine) -> Result<BTreeMap<String, Array>> {
        let m = self
            .controller
            .as_ref()
            .ok_or("MPC Designer: missing controller")?;
        mpc::property_names()
            .into_iter()
            .map(|n| {
                let a = mpc::property(e, m, &n)?.array(&e.cancel)?;
                Ok((n, a))
            })
            .collect()
    }
    /// Physical channel roles, in plant order.
    pub fn roles(&self) -> (Vec<usize>, Vec<usize>) {
        let Some(m) = &self.controller else {
            return (vec![], vec![]);
        };
        let mut inputs = vec![0; m.nu()];
        for &i in &m.md_idx {
            inputs[i] = 1;
        }
        for &i in &m.ud_idx {
            inputs[i] = 2;
        }
        let mut outputs = vec![0; m.ny()];
        for &i in &m.uo_idx {
            outputs[i] = 1;
        }
        (inputs, outputs)
    }
}
pub fn snapshot(e: &Engine, name: &str) -> Result<Source> {
    let a = e
        .array(name)
        .ok_or("MPC Designer: missing source variable")?;
    if let Some(m) = objects::mpc_value(e, &Value::from(a.clone())) {
        Ok(Source {
            name: name.into(),
            plant: m.plant_array.clone(),
            controller: Some(m),
        })
    } else {
        Source::plant(name.into(), a)
    }
}
pub fn catalogue(e: &Engine) -> Vec<Source> {
    e.workspace()
        .keys()
        .filter_map(|n| snapshot(e, n).ok().filter(Source::is_controller))
        .collect()
}
type Open = Box<dyn FnMut(Option<Source>) -> Result<()>>;
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
pub fn present(open: impl FnMut(Option<Source>) -> Result<()> + 'static) -> Presentation {
    Presentation {
        previous: OPEN.with(|s| s.replace(Some(Box::new(open)))),
        thread: PhantomData,
    }
}
pub(super) fn open(e: &Engine, args: &[Value], count: usize) -> Result<()> {
    if args.len() > 1 || count != 0 {
        return Err("mpcDesigner: expected zero or one input and no outputs".into());
    }
    if !OPEN.with(|s| s.borrow().is_some()) {
        return Err("mpcDesigner requires the desktop".into());
    }
    let source = if let Some(v) = args.first() {
        let a = v.clone().array(&e.cancel)?;
        let name = e
            .workspace()
            .into_iter()
            .find(|(_, b)| a.same_snapshot(b))
            .map(|(n, _)| n)
            .ok_or("mpcDesigner: store the plant or controller in a workspace variable first")?;
        Some(snapshot(e, &name)?)
    } else {
        None
    };
    OPEN.with(|s| s.borrow_mut().as_mut().unwrap()(source))
}
