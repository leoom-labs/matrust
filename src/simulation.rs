//! Deterministic single-rate, fixed-step, scalar discrete simulation.
//! Stateful outputs are read before combinational evaluation and updated afterwards.
use crate::{
    Matrix, Result,
    dsp::{self, Filter},
    files,
};
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, path::Path, sync::atomic::AtomicBool};
pub mod document;
mod integrator;
mod routing;
pub mod slx;
pub use integrator::IntegratorMethod;
pub use routing::{ProductInput, SwitchCriteria};
pub const MAX_LAYOUT_COORDINATE: f32 = 1_000_000.;

fn section_filters(sections: &[[f64; 6]]) -> Result<Vec<Filter>> {
    if sections.is_empty() || sections.len() > 64 {
        return Err("SOS block requires 1..64 sections".into());
    }
    sections
        .iter()
        .map(|s| Filter::new(&s[..3], &s[3..]))
        .collect()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum Block {
    Constant {
        value: f64,
    },
    Sine {
        amplitude: f64,
        frequency_hz: f64,
        phase: f64,
    },
    Step {
        time: f64,
        before: f64,
        after: f64,
    },
    Gain {
        gain: f64,
    },
    #[serde(deserialize_with = "deserialize_parameterless_block")]
    Sum,
    #[serde(deserialize_with = "deserialize_parameterless_block")]
    Abs,
    #[serde(deserialize_with = "deserialize_parameterless_block")]
    UnaryMinus,
    Product {
        #[serde(default = "routing::default_product_inputs")]
        operations: Vec<ProductInput>,
    },
    Switch {
        #[serde(default)]
        criteria: SwitchCriteria,
        #[serde(default)]
        threshold: f64,
    },
    Bias {
        bias: f64,
    },
    Saturation {
        lower: f64,
        upper: f64,
    },
    UnitDelay {
        initial: f64,
    },
    DiscreteIntegrator {
        initial: f64,
        gain: f64,
        #[serde(default)]
        method: IntegratorMethod,
    },
    Filter {
        b: Vec<f64>,
        a: Vec<f64>,
    },
    SosFilter {
        sections: Vec<[f64; 6]>,
    },
    #[serde(deserialize_with = "deserialize_parameterless_block")]
    Scope,
}
// Internally tagged unit variants otherwise ignore unknown fields in serde's
// unit visitor. Deserialize their parameter maps through a strict empty struct,
// retaining the public unit variants and their existing serialized wire form.
fn deserialize_parameterless_block<'de, D>(deserializer: D) -> std::result::Result<(), D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Parameters {}
    Parameters::deserialize(deserializer).map(|_| ())
}
impl Block {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Constant { .. } => "Constant",
            Self::Sine { .. } => "Sine",
            Self::Step { .. } => "Step",
            Self::Gain { .. } => "Gain",
            Self::Sum => "Sum",
            Self::Abs => "Abs",
            Self::UnaryMinus => "Unary Minus",
            Self::Product { .. } => "Product",
            Self::Switch { .. } => "Switch",
            Self::Bias { .. } => "Bias",
            Self::Saturation { .. } => "Saturation",
            Self::UnitDelay { .. } => "Unit Delay",
            Self::DiscreteIntegrator { .. } => "Discrete-Time Integrator",
            Self::Filter { .. } => "Discrete Filter",
            Self::SosFilter { .. } => "SOS Filter",
            Self::Scope => "Scope",
        }
    }
    fn validate(&self, inputs: usize, dt: f64) -> Result<()> {
        let (arity, values) = match self {
            Self::Constant { value } => (0, vec![*value]),
            Self::Sine {
                amplitude,
                frequency_hz,
                phase,
            } => {
                if *frequency_hz < 0. || *frequency_hz >= 0.5 / dt {
                    return Err("Sine frequency must be nonnegative and below Nyquist".into());
                }
                (0, vec![*amplitude, *frequency_hz, *phase])
            }
            Self::Step {
                time,
                before,
                after,
            } => (0, vec![*time, *before, *after]),
            Self::Gain { gain } => (1, vec![*gain]),
            Self::Abs | Self::UnaryMinus => (1, vec![]),
            Self::Product { operations } => {
                routing::validate_product(operations)?;
                (operations.len(), vec![])
            }
            Self::Switch { threshold, .. } => (3, vec![*threshold]),
            Self::Bias { bias } => (1, vec![*bias]),
            Self::Saturation { lower, upper } => {
                if lower > upper {
                    return Err("Saturation lower limit must not exceed upper limit".into());
                }
                (1, vec![*lower, *upper])
            }
            Self::Sum => {
                if !(2..=32).contains(&inputs) {
                    return Err("Sum requires 2..32 inputs".into());
                }
                (inputs, vec![])
            }
            Self::UnitDelay { initial } => (1, vec![*initial]),
            Self::DiscreteIntegrator { initial, gain, .. } => {
                integrator::validate(*initial, *gain)?;
                if !(gain * dt).is_finite() {
                    return Err("Integrator gain times sample time must be finite".into());
                }
                (1, vec![*initial, *gain])
            }
            Self::Filter { b, a } => {
                Filter::new(b, a)?;
                (1, vec![])
            }
            Self::SosFilter { sections } => {
                section_filters(sections)?;
                (1, vec![])
            }
            Self::Scope => (1, vec![]),
        };
        if inputs != arity {
            return Err(format!("{} requires {arity} inputs", self.label()));
        }
        if values.iter().any(|v| !v.is_finite()) {
            return Err("Block parameters must be finite".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub name: String,
    pub block: Block,
    pub inputs: Vec<usize>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    pub version: u32,
    pub sample_time: f64,
    pub steps: usize,
    pub nodes: Vec<Node>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub positions: Vec<[f32; 2]>,
}
pub struct Trace {
    pub name: String,
    pub values: Matrix,
}
pub struct Simulation {
    pub time: Matrix,
    pub traces: Vec<Trace>,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            version: 1,
            positions: Vec::new(),
            sample_time: 0.001,
            steps: 1000,
            nodes: vec![
                Node {
                    name: "Signal".into(),
                    block: Block::Sine {
                        amplitude: 1.,
                        frequency_hz: 10.,
                        phase: 0.,
                    },
                    inputs: vec![],
                },
                Node {
                    name: "Noise".into(),
                    block: Block::Sine {
                        amplitude: 0.3,
                        frequency_hz: 200.,
                        phase: 0.,
                    },
                    inputs: vec![],
                },
                Node {
                    name: "Mix".into(),
                    block: Block::Sum,
                    inputs: vec![0, 1],
                },
                Node {
                    name: "Lowpass".into(),
                    block: Block::Filter {
                        b: dsp::fir1(40, &[0.1], dsp::Band::Lowpass).expect("constant FIR design"),
                        a: vec![1.],
                    },
                    inputs: vec![2],
                },
                Node {
                    name: "Filtered".into(),
                    block: Block::Scope,
                    inputs: vec![3],
                },
            ],
        }
    }
}
impl Model {
    /// Layout metadata is independent of block execution. Legacy models omit it.
    pub fn validate_layout(&self) -> Result<()> {
        validate_positions(self.version, self.nodes.len(), &self.positions)
    }
    /// Publish a complete layout only after validating every position.
    pub fn set_layout(&mut self, positions: Vec<[f32; 2]>) -> Result<()> {
        let version = if positions.is_empty() { 1 } else { 2 };
        validate_positions(version, self.nodes.len(), &positions)?;
        self.positions = positions;
        self.version = version;
        Ok(())
    }
    /// Storage validation permits incomplete diagrams; running requires full graph validation.
    pub fn validate_storage(&self) -> Result<()> {
        self.validate_layout()?;
        if self.nodes.len() > 512
            || self
                .nodes
                .iter()
                .any(|n| n.inputs.len() > 32 || n.name.len() > 128)
        {
            return Err("Model version or storage bounds are invalid".into());
        }
        if !self.sample_time.is_finite()
            || self.nodes.iter().any(|n| match &n.block {
                Block::Constant { value } => !value.is_finite(),
                Block::Sine {
                    amplitude,
                    frequency_hz,
                    phase,
                } => ![*amplitude, *frequency_hz, *phase]
                    .iter()
                    .all(|v| v.is_finite()),
                Block::Step {
                    time,
                    before,
                    after,
                } => ![*time, *before, *after].iter().all(|v| v.is_finite()),
                Block::Gain { gain } => !gain.is_finite(),
                Block::Bias { bias } => !bias.is_finite(),
                Block::Switch { threshold, .. } => !threshold.is_finite(),
                Block::Product { operations } => routing::validate_product(operations).is_err(),
                Block::Saturation { lower, upper } => !lower.is_finite() || !upper.is_finite(),
                Block::UnitDelay { initial } => !initial.is_finite(),
                Block::DiscreteIntegrator { initial, gain, .. } => {
                    !initial.is_finite() || !gain.is_finite()
                }
                Block::Filter { b, a } => {
                    b.len().max(a.len()) > 4097 || b.iter().chain(a).any(|v| !v.is_finite())
                }
                Block::SosFilter { sections } => {
                    sections.len() > 64 || sections.iter().flatten().any(|v| !v.is_finite())
                }
                _ => false,
            })
        {
            return Err("Model contains nonfinite parameters or oversized coefficients".into());
        }
        Ok(())
    }
    /// Schedule is constructed once per run, never in the sample loop.
    pub fn validate(&self) -> Result<Vec<usize>> {
        self.validate_layout()?;
        if !self.sample_time.is_finite()
            || self.sample_time <= 0.
            || !(self.sample_time * self.steps as f64).is_finite()
            || self.steps == 0
            || self.steps > 1_000_000
        {
            return Err("Sample time must be positive and finite; steps must be 1..1000000".into());
        }
        let n = self.nodes.len();
        if n == 0 || n > 512 {
            return Err("Model must have 1..512 blocks".into());
        }
        let mut indegree = vec![0; n];
        let mut edges = vec![Vec::new(); n];
        let mut scopes = 0;
        let mut work = 0usize;
        let mut names = std::collections::HashSet::new();
        for (i, node) in self.nodes.iter().enumerate() {
            if node.name.trim().is_empty() || node.name.len() > 128 || !names.insert(&node.name) {
                return Err("Block names must be unique, nonempty and at most 128 bytes".into());
            }
            node.block
                .validate(node.inputs.len(), self.sample_time)
                .map_err(|e| format!("{}: {e}", node.name))?;
            scopes += usize::from(matches!(node.block, Block::Scope));
            work = work.saturating_add(match &node.block {
                Block::Filter { b, a } => b.len().max(a.len()),
                Block::SosFilter { sections } => sections.len() * 3,
                // Nominal scalar arithmetic work, not a CPU-instruction budget.
                Block::Product { operations } => operations.len(),
                Block::DiscreteIntegrator { .. } => 3,
                _ => 1,
            });
            for &input in &node.inputs {
                if input >= n {
                    return Err(format!(
                        "{}: an input is not connected to an existing block",
                        node.name
                    ));
                }
                if !matches!(
                    node.block,
                    Block::UnitDelay { .. } | Block::DiscreteIntegrator { .. }
                ) {
                    indegree[i] += 1;
                    edges[input].push(i);
                }
            }
        }
        if scopes == 0 {
            return Err("Add at least one Scope to capture simulation output".into());
        }
        if (scopes + 1).saturating_mul(self.steps) > crate::matrix::MAX_ELEMENTS
            || work.saturating_mul(self.steps) > 256_000_000
        {
            return Err("Simulation exceeds storage/work limit; reduce steps or blocks".into());
        }
        let mut ready: VecDeque<_> = (0..n).filter(|&i| indegree[i] == 0).collect();
        let mut order = Vec::with_capacity(n);
        while let Some(i) = ready.pop_front() {
            order.push(i);
            for &next in &edges[i] {
                indegree[next] -= 1;
                if indegree[next] == 0 {
                    ready.push_back(next);
                }
            }
        }
        if order.len() != n {
            return Err("Algebraic loop: feedback requires a Unit Delay or Forward Euler integrator; implicit algebraic solving is not supported".into());
        }
        Ok(order)
    }
    pub fn run(&self, cancel: &AtomicBool) -> Result<Simulation> {
        let order = self.validate()?;
        dsp::check_cancel(cancel)?;
        let mut filters: Vec<_> = self
            .nodes
            .iter()
            .map(|n| match &n.block {
                Block::Filter { b, a } => Filter::new(b, a).map(|f| vec![f]),
                Block::SosFilter { sections } => section_filters(sections),
                _ => Ok(vec![]),
            })
            .collect::<Result<_>>()?;
        let mut values = vec![0.; self.nodes.len()];
        let mut delays = values.clone();
        let state_updates: Vec<_> = self
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| match n.block {
                Block::UnitDelay { initial } => {
                    delays[i] = initial;
                    Some((i, None))
                }
                Block::DiscreteIntegrator { initial, gain, .. } => {
                    delays[i] = initial;
                    Some((i, Some(gain * self.sample_time)))
                }
                _ => None,
            })
            .collect();
        let scope_ids: Vec<_> = self
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| matches!(n.block, Block::Scope).then_some(i))
            .collect();
        let mut traces: Vec<Vec<f64>> = scope_ids
            .iter()
            .map(|_| Vec::with_capacity(self.steps))
            .collect();
        let mut time = Vec::with_capacity(self.steps);
        for sample in 0..self.steps {
            if sample % 256 == 0 {
                #[cfg(test)]
                routing::cancellation_probe(sample, cancel);
                dsp::check_cancel(cancel)?;
            }
            let t = sample as f64 * self.sample_time;
            time.push(t);
            for &i in &order {
                let node = &self.nodes[i];
                values[i] = match &node.block {
                    Block::Constant { value } => *value,
                    Block::Sine {
                        amplitude,
                        frequency_hz,
                        phase,
                    } => amplitude * (std::f64::consts::TAU * frequency_hz * t + phase).sin(),
                    Block::Step {
                        time,
                        before,
                        after,
                    } => {
                        if t < *time {
                            *before
                        } else {
                            *after
                        }
                    }
                    Block::Gain { gain } => values[node.inputs[0]] * gain,
                    Block::Abs => values[node.inputs[0]].abs(),
                    Block::UnaryMinus => -values[node.inputs[0]],
                    Block::Product { operations } => {
                        routing::product(operations, &node.inputs, &values)
                    }
                    Block::Switch {
                        criteria,
                        threshold,
                    } => {
                        let port = if criteria.selects_first(values[node.inputs[1]], *threshold) {
                            0
                        } else {
                            2
                        };
                        values[node.inputs[port]]
                    }
                    Block::Bias { bias } => values[node.inputs[0]] + bias,
                    Block::Saturation { lower, upper } => {
                        let input = values[node.inputs[0]];
                        if input < *lower {
                            *lower
                        } else if input > *upper {
                            *upper
                        } else {
                            input
                        }
                    }
                    Block::Sum => node.inputs.iter().map(|&j| values[j]).sum(),
                    Block::UnitDelay { .. } | Block::DiscreteIntegrator { .. } => delays[i],
                    Block::Filter { .. } | Block::SosFilter { .. } => filters[i]
                        .iter_mut()
                        .fold(values[node.inputs[0]], |x, filter| filter.sample(x)),
                    Block::Scope => values[node.inputs[0]],
                };
                if !values[i].is_finite() {
                    return Err(format!(
                        "{} produced a nonfinite value at sample {sample} (t={t}); check stability and parameters",
                        node.name
                    ));
                }
            }
            // Simultaneous state update is essential for cascaded delays and feedback.
            for &(i, coefficient) in &state_updates {
                let input = values[self.nodes[i].inputs[0]];
                let next = match coefficient {
                    Some(coefficient) => delays[i] + coefficient * input,
                    None => input,
                };
                if !next.is_finite() {
                    return Err(format!(
                        "{} produced a nonfinite next state at sample {sample} (t={t}); check stability and parameters",
                        self.nodes[i].name
                    ));
                }
                delays[i] = next;
            }
            for (trace, &id) in traces.iter_mut().zip(&scope_ids) {
                trace.push(values[id]);
            }
        }
        Ok(Simulation {
            time: Matrix::from_column_vec(self.steps, 1, time)?,
            traces: traces
                .into_iter()
                .zip(scope_ids)
                .map(|(v, i)| {
                    Ok(Trace {
                        name: self.nodes[i].name.clone(),
                        values: Matrix::from_column_vec(self.steps, 1, v)?,
                    })
                })
                .collect::<Result<_>>()?,
        })
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate_storage()?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err("Model file exceeds 4 MiB storage limit".into());
        }
        files::atomic_write(path, &bytes)
    }
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = files::read_bounded(path, 4 * 1024 * 1024)?;
        let model: Self =
            serde_json::from_slice(&bytes).map_err(|e| format!("Invalid MatRust model: {e}"))?;
        model.validate_storage()?;
        Ok(model)
    }
}

fn validate_positions(version: u32, nodes: usize, positions: &[[f32; 2]]) -> Result<()> {
    if !matches!(version, 1 | 2) {
        return Err("Unsupported model version".into());
    }
    if positions.len() > 512
        || (version == 1 && !positions.is_empty())
        || (version == 2 && (positions.is_empty() || positions.len() != nodes))
        || positions
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || !(0.0..=MAX_LAYOUT_COORDINATE).contains(v))
    {
        return Err(
            "Invalid model layout: version 2 requires one finite, bounded position per block"
                .into(),
        );
    }
    Ok(())
}
