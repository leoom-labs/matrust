//! Scenario snapshots and strict trajectory decoding. Numerical work stays in sim.
use super::*;

#[derive(Clone, Debug, PartialEq)]
pub struct Scenario {
    pub samples: String,
    pub reference: String,
    pub disturbance: String,
    pub initial: String,
    pub reference_preview: bool,
    pub disturbance_preview: bool,
}
impl Default for Scenario {
    fn default() -> Self {
        Self {
            samples: "30".into(),
            reference: String::new(),
            disturbance: String::new(),
            initial: String::new(),
            reference_preview: false,
            disturbance_preview: false,
        }
    }
}
impl Scenario {
    pub fn count(&self) -> Result<usize, String> {
        let n = script::scalar(&self.samples, "Stored samples")?;
        if !(1. ..=10000.).contains(&n) || n.fract() != 0. {
            return Err("Stored samples must be an integer from 1 to 10000".into());
        }
        Ok(n as usize)
    }
    pub fn validate(&self, specs: &[Spec]) -> Result<(), String> {
        self.count()?;
        let first = specs.first().ok_or("Create a controller first")?;
        for s in specs {
            if s.inputs != first.inputs || s.outputs != first.outputs {
                return Err("Compare requires matching physical input/output channel counts and MV/MD/UD/MO/UO roles".into());
            }
            if script::scalar(&s.ts, "Sample time")? != script::scalar(&first.ts, "Sample time")? {
                return Err(
                    "Compare requires the same sample time; no resampling is performed".into(),
                );
            }
        }
        for (label, value, cols) in [
            ("Reference", &self.reference, first.outputs.len()),
            (
                "Measured disturbances",
                &self.disturbance,
                first.inputs.iter().filter(|r| **r == 1).count(),
            ),
        ] {
            if let Some(rows) = finite_literal(value).map_err(|e| format!("{label}: {e}"))?
                && rows[0].len() != cols
            {
                return Err(format!("{label} requires {cols} physical channel columns"));
            }
        }
        if let Some(rows) = finite_literal(&self.initial)?
            && rows.len() != 1
            && rows[0].len() != 1
        {
            return Err("Plant initial state must be a vector".into());
        }
        Ok(())
    }
}
pub(super) fn finite_literal(value: &str) -> Result<Option<Vec<Vec<f64>>>, String> {
    if value.len() > 32_768 {
        return Err("Numeric field exceeds 32768 bytes".into());
    }
    if value.trim().is_empty() || value.trim() == "[]" {
        return Ok(None);
    }
    let rows = script::literal(value)?;
    if rows.iter().flatten().any(|x| !x.is_finite()) {
        return Err("Scenario values must be finite numeric literals".into());
    }
    Ok(Some(rows))
}
#[derive(Clone)]
pub struct RunSpec {
    pub scenario: Scenario,
    pub controllers: Vec<Controller>,
}
#[derive(Clone)]
pub struct Trajectory {
    pub y: matrust::Matrix,
    pub t: matrust::Matrix,
    pub u: matrust::Matrix,
}
#[derive(Clone)]
pub struct ScenarioResult {
    pub run: RunSpec,
    pub trajectories: Vec<Trajectory>,
    pub reference: matrust::Matrix,
}
pub fn output_names(prefix: &str, count: usize) -> Vec<String> {
    let mut names = ["r", "v", "x0", "options"]
        .map(|s| format!("{prefix}_{s}"))
        .to_vec();
    for i in 0..count {
        for suffix in ["y", "t", "u"] {
            names.push(format!(
                "{prefix}_{suffix}{}",
                if i == 0 {
                    String::new()
                } else {
                    (i + 1).to_string()
                }
            ));
        }
    }
    names
}
pub(super) fn collect(
    outputs: &Outputs,
    run: RunSpec,
    prefix: &str,
) -> Result<ScenarioResult, String> {
    let numeric = |name: &str| -> Result<matrust::Matrix, String> {
        match outputs.get(name) {
            Some(Array::Numeric(m)) if m.data().iter().all(|v| v.is_finite()) => Ok(m.clone()),
            _ => Err(format!("Missing or nonfinite simulation array: {name}")),
        }
    };
    let n = run.scenario.count()?;
    let mut trajectories = Vec::new();
    for (i, c) in run.controllers.iter().enumerate() {
        let suffix = if i == 0 {
            String::new()
        } else {
            (i + 1).to_string()
        };
        let y = numeric(&format!("{prefix}_y{suffix}"))?;
        let t = numeric(&format!("{prefix}_t{suffix}"))?;
        let u = numeric(&format!("{prefix}_u{suffix}"))?;
        if (y.rows(), y.cols()) != (n, c.spec.outputs.len())
            || (t.rows(), t.cols()) != (n, 1)
            || (u.rows(), u.cols()) != (n, c.spec.mv.len())
            || t.data().windows(2).any(|w| w[1] <= w[0])
        {
            return Err("Simulation returned inconsistent y/t/u shapes or time values".into());
        }
        if let Some(first) = trajectories.first() {
            let first: &Trajectory = first;
            if first.t.data() != t.data() {
                return Err("Compared simulation time vectors differ".into());
            }
        }
        trajectories.push(Trajectory { y, t, u });
    }
    let reference = numeric(&format!("{prefix}_r"))?;
    if reference.rows() == 0 || reference.cols() != run.controllers[0].spec.outputs.len() {
        return Err("Simulation reference columns differ from physical outputs".into());
    }
    Ok(ScenarioResult {
        run,
        trajectories,
        reference,
    })
}
