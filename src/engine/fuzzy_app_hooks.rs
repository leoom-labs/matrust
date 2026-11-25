//! UI-independent, scoped desktop presentation for the type-1 fuzzy designer.
//! Portable records contain values, never engine-local object handles.
use super::{Engine, Execution, Value, fuzzy};
use crate::Result;
use std::{cell::RefCell, marker::PhantomData, rc::Rc};

#[derive(Clone, Debug, PartialEq)]
pub struct Membership {
    pub name: String,
    pub kind: String,
    pub parameters: Vec<f64>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Variable {
    pub name: String,
    pub range: [f64; 2],
    pub members: Vec<Membership>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub antecedent: Vec<f64>,
    pub consequent: Vec<f64>,
    pub weight: f64,
    pub connection: f64,
    pub description: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct System {
    pub sugeno: bool,
    pub name: String,
    pub methods: [String; 5],
    pub inputs: Vec<Variable>,
    pub outputs: Vec<Variable>,
    pub rules: Vec<Rule>,
    pub structural_checks_disabled: bool,
}

/// Read an existing system without executing user code or copying numeric storage.
/// Only the small editable metadata crosses the host boundary.
pub fn snapshot(engine: &Engine, name: &str) -> Result<System> {
    let value = engine.array(name).ok_or("Fuzzy designer: missing system")?;
    system(engine, &Value::from(value))
}
fn collection(e: &Engine, p: &fuzzy::Record, key: &str) -> Result<std::sync::Arc<fuzzy::Object>> {
    fuzzy::object(e, &Value::from(p[key].clone()))
}
fn system(e: &Engine, value: &Value) -> Result<System> {
    let o = fuzzy::object(e, value)?;
    if !matches!(o.class, "mamfis" | "sugfis") {
        return Err("Fuzzy designer: expected a scalar mamfis or sugfis".into());
    }
    let p = o.record()?;
    let variables = |key| -> Result<Vec<Variable>> {
        collection(e, p, key)?
            .items
            .iter()
            .map(|v| {
                let members = collection(e, v, "MembershipFunctions")?
                    .items
                    .iter()
                    .map(|m| {
                        Ok(Membership {
                            name: fuzzy::string(&m["Name"])?,
                            kind: fuzzy::string(&m["Type"])?,
                            parameters: fuzzy::vecnum(&m["Parameters"])?,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(Variable {
                    name: fuzzy::string(&v["Name"])?,
                    range: fuzzy::vecnum(&v["Range"])?
                        .try_into()
                        .map_err(|_| "Invalid fuzzy range")?,
                    members,
                })
            })
            .collect()
    };
    let rules = collection(e, p, "Rules")?
        .items
        .iter()
        .map(|r| {
            Ok(Rule {
                antecedent: fuzzy::vecnum(&r["Antecedent"])?,
                consequent: fuzzy::vecnum(&r["Consequent"])?,
                weight: fuzzy::scalar(&r["Weight"])?,
                connection: fuzzy::scalar(&r["Connection"])?,
                description: fuzzy::string(&r["Description"])?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let methods = [
        "AndMethod",
        "OrMethod",
        "ImplicationMethod",
        "AggregationMethod",
        "DefuzzificationMethod",
    ]
    .iter()
    .map(|k| fuzzy::string(&p[*k]))
    .collect::<Result<Vec<_>>>()?
    .try_into()
    .unwrap();
    Ok(System {
        sugeno: o.class == "sugfis",
        name: fuzzy::string(&p["Name"])?,
        methods,
        inputs: variables("Inputs")?,
        outputs: variables("Outputs")?,
        rules,
        structural_checks_disabled: fuzzy::scalar(&p["DisableStructuralChecks"])? != 0.,
    })
}
/// Catalogue for the desktop picker; unrelated workspace values are ignored.
pub fn catalogue(e: &Engine) -> Vec<(String, System)> {
    e.workspace()
        .into_iter()
        .filter_map(|(name, value)| system(e, &Value::from(value)).ok().map(|s| (name, s)))
        .collect()
}
type Open = Box<dyn FnMut(System) -> Result<()>>;
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
pub fn present(open: impl FnMut(System) -> Result<()> + 'static) -> Presentation {
    Presentation {
        previous: OPEN.with(|s| s.replace(Some(Box::new(open)))),
        thread: PhantomData,
    }
}
pub(super) fn open(
    e: &mut Engine,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<()> {
    if args.len() > 1 || count != 0 {
        return Err("fuzzyLogicDesigner: expected zero or one input and no outputs".into());
    }
    if !OPEN.with(|s| s.borrow().is_some()) {
        return Err("fuzzyLogicDesigner requires the desktop".into());
    }
    let value = match args.first() {
        None => fuzzy::call(e, "mamfis", &[], 1, out)?.remove(0),
        Some(v) if fuzzy::vtext(e, v).is_ok() => {
            fuzzy::call(e, "readfis", std::slice::from_ref(v), 1, out)?.remove(0)
        }
        Some(v) => v.clone(),
    };
    let system = system(e, &value)?;
    OPEN.with(|s| s.borrow_mut().as_mut().unwrap()(system))
}
