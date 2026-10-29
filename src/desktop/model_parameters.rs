//! Small property snapshots and ownership-swapping coefficient history.
use matrust::{
    Result,
    simulation::{Block, Model, Node, ProductInput, SwitchCriteria},
};

#[derive(Clone, PartialEq, Eq)]
pub struct Properties {
    name: String,
    kind: &'static str,
    values: [u64; 3],
    product: Option<(Vec<ProductInput>, Vec<usize>)>,
}
fn values(block: &Block) -> [u64; 3] {
    let v = match block {
        Block::Constant { value } => [*value, 0., 0.],
        Block::Gain { gain } => [*gain, 0., 0.],
        Block::Switch {
            criteria,
            threshold,
        } => [*threshold, *criteria as u8 as f64, 0.],
        Block::Bias { bias } => [*bias, 0., 0.],
        Block::Saturation { lower, upper } => [*lower, *upper, 0.],
        Block::UnitDelay { initial } => [*initial, 0., 0.],
        Block::DiscreteIntegrator { initial, gain, .. } => [*initial, *gain, 0.],
        Block::Sine {
            amplitude,
            frequency_hz,
            phase,
        } => [*amplitude, *frequency_hz, *phase],
        Block::Step {
            time,
            before,
            after,
        } => [*time, *before, *after],
        _ => [0.; 3],
    };
    v.map(f64::to_bits)
}
impl Properties {
    pub fn capture(node: &Node) -> Self {
        Self {
            name: node.name.clone(),
            kind: node.block.label(),
            values: values(&node.block),
            product: match &node.block {
                Block::Product { operations }
                    if operations.len() <= 32 && node.inputs.len() <= 32 =>
                {
                    Some((operations.clone(), node.inputs.clone()))
                }
                _ => None,
            },
        }
    }
    pub fn matches(&self, node: &Node) -> bool {
        self.name == node.name
            && self.kind == node.block.label()
            && self.values == values(&node.block)
            && match (&self.product, &node.block) {
                (
                    Some((operations, inputs)),
                    Block::Product {
                        operations: current,
                    },
                ) => operations == current && inputs == &node.inputs,
                (None, Block::Product { .. }) => false,
                (None, _) => true,
                _ => false,
            }
    }
    fn apply(&self, node: &mut Node) {
        if node.name != self.name {
            node.name = self.name.clone();
        }
        let [x, y, z] = self.values.map(f64::from_bits);
        match &mut node.block {
            Block::Constant { value } => *value = x,
            Block::Gain { gain } => *gain = x,
            Block::Switch {
                criteria,
                threshold,
            } => {
                *threshold = x;
                *criteria = match y as u8 {
                    0 => SwitchCriteria::Greater,
                    1 => SwitchCriteria::GreaterEqual,
                    _ => SwitchCriteria::Nonzero,
                };
            }
            Block::Product { operations } => {
                if let Some((saved, inputs)) = &self.product {
                    operations.clone_from(saved);
                    node.inputs.clone_from(inputs);
                }
            }
            Block::Bias { bias } => *bias = x,
            Block::Saturation { lower, upper } => {
                *lower = x;
                *upper = y;
            }
            Block::UnitDelay { initial } => *initial = x,
            Block::DiscreteIntegrator { initial, gain, .. } => {
                *initial = x;
                *gain = y;
            }
            Block::Sine {
                amplitude,
                frequency_hz,
                phase,
            } => {
                *amplitude = x;
                *frequency_hz = y;
                *phase = z;
            }
            Block::Step {
                time,
                before,
                after,
            } => {
                *time = x;
                *before = y;
                *after = z;
            }
            _ => {}
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    time: u64,
    steps: usize,
}
impl Settings {
    pub fn capture(model: &Model) -> Self {
        Self {
            time: model.sample_time.to_bits(),
            steps: model.steps,
        }
    }
}

// Keep property snapshots inline: the shared history is capped at 128 entries,
// charges size_of::<Change>() plus these heap buffers, and has a 4 MiB budget.
// Boxing here would add an allocation to every ordinary property edit.
#[allow(clippy::large_enum_variant)]
pub enum Edit {
    Node {
        index: usize,
        count: usize,
        before: Properties,
        after: Properties,
    },
    Settings {
        before: Settings,
        after: Settings,
    },
}
impl Edit {
    pub fn node(model: &Model, index: usize, before: Properties) -> Option<Self> {
        let node = model.nodes.get(index)?;
        if model.nodes.len() > 512
            || before.name.len() > 512
            || node.name.len() > 512
            || before.kind != node.block.label()
            || matches!(&node.block, Block::Product { operations } if operations.is_empty() || operations.len() > 32 || node.inputs.len() > 32)
            || matches!(&node.block, Block::Product { .. }) && before.product.is_none()
        {
            return None;
        }
        if before.matches(node) {
            return None;
        }
        Some(Self::Node {
            index,
            count: model.nodes.len(),
            before,
            after: Properties::capture(node),
        })
    }
    pub fn settings(model: &Model, before: Settings) -> Option<Self> {
        let after = Settings::capture(model);
        (before != after).then_some(Self::Settings { before, after })
    }
    pub fn index(&self) -> Option<usize> {
        match self {
            Self::Node { index, .. } => Some(*index),
            _ => None,
        }
    }
    pub fn apply(&self, model: &mut Model, redo: bool) -> bool {
        match self {
            Self::Node {
                index,
                count,
                before,
                after,
            } => {
                if model.nodes.len() != *count {
                    return false;
                }
                let (expected, replacement) = if redo {
                    (before, after)
                } else {
                    (after, before)
                };
                let Some(node) = model.nodes.get_mut(*index) else {
                    return false;
                };
                if !expected.matches(node) {
                    return false;
                }
                replacement.apply(node);
            }
            Self::Settings { before, after } => {
                let (expected, replacement) = if redo {
                    (before, after)
                } else {
                    (after, before)
                };
                if Settings::capture(model) != *expected {
                    return false;
                }
                model.sample_time = f64::from_bits(replacement.time);
                model.steps = replacement.steps;
            }
        }
        true
    }
    pub fn merge(&mut self, next: Self) -> Option<Self> {
        match (self, next) {
            (
                Self::Node {
                    index,
                    count,
                    after,
                    ..
                },
                Self::Node {
                    index: ni,
                    count: nc,
                    before: nb,
                    after: na,
                },
            ) if *index == ni && *count == nc && *after == nb => {
                *after = na;
                None
            }
            (
                Self::Settings { after, .. },
                Self::Settings {
                    before: nb,
                    after: na,
                },
            ) if *after == nb => {
                *after = na;
                None
            }
            (_, next) => Some(next),
        }
    }
    pub fn unchanged(&self) -> bool {
        match self {
            Self::Node { before, after, .. } => before == after,
            Self::Settings { before, after } => before == after,
        }
    }
    pub fn heap_bytes(&self) -> usize {
        match self {
            Self::Node { before, after, .. } => {
                let product_bytes = |p: &Properties| {
                    p.product.as_ref().map_or(0, |(operations, inputs)| {
                        operations.capacity() * size_of::<ProductInput>()
                            + inputs.capacity() * size_of::<usize>()
                    })
                };
                before.name.capacity()
                    + after.name.capacity()
                    + product_bytes(before)
                    + product_bytes(after)
            }
            _ => 0,
        }
    }
}

fn coefficient_bytes(block: &Block) -> Result<usize> {
    let bytes = match block {
        Block::Filter { b, a } if b.len().max(a.len()) <= 4097 => {
            b.capacity().saturating_add(a.capacity()).saturating_mul(8)
        }
        Block::SosFilter { sections } if sections.len() <= 64 => {
            sections.capacity().saturating_mul(48)
        }
        _ => return Err("Expected bounded filter coefficients".into()),
    };
    if bytes > 256 * 1024 {
        return Err("Filter buffer exceeds undo storage limit".into());
    }
    Ok(bytes)
}
fn equal_coefficients(a: &Block, b: &Block) -> bool {
    match (a, b) {
        (Block::Filter { b: ab, a: aa }, Block::Filter { b: bb, a: ba }) => {
            bytemuck::cast_slice::<f64, u64>(ab) == bytemuck::cast_slice::<f64, u64>(bb)
                && bytemuck::cast_slice::<f64, u64>(aa) == bytemuck::cast_slice::<f64, u64>(ba)
        }
        (Block::SosFilter { sections: a }, Block::SosFilter { sections: b }) => {
            bytemuck::cast_slice::<[f64; 6], u64>(a) == bytemuck::cast_slice::<[f64; 6], u64>(b)
        }
        _ => false,
    }
}
pub(super) struct Coefficients {
    pub index: usize,
    count: usize,
    stored: Block,
    expected: blake3::Hash,
    capacity: usize,
}
impl Coefficients {
    pub fn replace(model: &mut Model, index: usize, replacement: Block) -> Result<Option<Self>> {
        let count = model.nodes.len();
        let node = model.nodes.get_mut(index).ok_or("Block was removed")?;
        let capacity = coefficient_bytes(&node.block)?.max(coefficient_bytes(&replacement)?);
        match &replacement {
            Block::Filter { b, a } => {
                matrust::dsp::Filter::new(b, a)?;
            }
            Block::SosFilter { sections } => {
                if sections.is_empty() {
                    return Err("SOS requires at least one section".into());
                }
                for s in sections {
                    matrust::dsp::Filter::new(&s[..3], &s[3..])?;
                }
            }
            _ => unreachable!(),
        }
        if node.block.label() != replacement.label() {
            return Err("Filter type changed".into());
        }
        if equal_coefficients(&node.block, &replacement) {
            return Ok(None);
        }
        let stored = std::mem::replace(&mut node.block, replacement);
        let expected = super::topology::node_hash(node);
        Ok(Some(Self {
            index,
            count,
            stored,
            expected,
            capacity,
        }))
    }
    pub fn apply(&mut self, model: &mut Model) -> bool {
        if model.nodes.len() != self.count {
            return false;
        }
        let Some(node) = model.nodes.get_mut(self.index) else {
            return false;
        };
        if !coefficient_bytes(&node.block).is_ok_and(|b| b <= self.capacity)
            || super::topology::node_hash(node) != self.expected
        {
            return false;
        }
        std::mem::swap(&mut node.block, &mut self.stored);
        self.expected = super::topology::node_hash(node);
        true
    }
    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>() + self.capacity
    }
}
