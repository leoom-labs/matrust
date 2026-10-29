//! Reversible node insertion/removal. Coefficient buffers move; they never clone.
use matrust::{
    Result,
    simulation::{Block, Model, Node},
};

const MAX_NODE_BYTES: usize = 256 * 1024;

struct Edge {
    source: usize,
    target: u16,
    port: u8,
}

pub(super) struct Change {
    pub index: usize,
    inserted: bool,
    node: Option<Node>,
    node_hash: blake3::Hash,
    position: Option<[f32; 2]>,
    edges: Vec<Edge>,
    before: blake3::Hash,
    after: blake3::Hash,
    node_bytes: usize,
}

pub(super) fn bounds(model: &Model) -> Result<()> {
    model.validate_layout()?;
    if model.nodes.len() > 512
        || model
            .nodes
            .iter()
            .any(|n| n.inputs.len() > 32 || n.name.len() > 512)
    {
        return Err("Model exceeds undoable diagram bounds".into());
    }
    Ok(())
}

pub(super) fn bytes(node: &Node) -> Result<usize> {
    let parameters = match &node.block {
        Block::Product { operations } => {
            if !(1..=32).contains(&operations.len()) {
                return Err("Product requires 1..32 operations".into());
            }
            operations
                .capacity()
                .saturating_mul(size_of::<matrust::simulation::ProductInput>())
        }
        Block::Filter { b, a } => {
            if b.len().max(a.len()) > 4097 {
                return Err("Filter exceeds the supported coefficient limit".into());
            }
            b.capacity().saturating_add(a.capacity()).saturating_mul(8)
        }
        Block::SosFilter { sections } => {
            if sections.len() > 64 {
                return Err("SOS block exceeds the supported section limit".into());
            }
            sections.capacity().saturating_mul(48)
        }
        _ => 0,
    };
    let total = parameters
        .saturating_add(node.name.capacity())
        .saturating_add(node.inputs.capacity().saturating_mul(size_of::<usize>()));
    if total > MAX_NODE_BYTES || node.inputs.len() > 32 || node.name.len() > 512 {
        return Err("Block exceeds the undo storage limit".into());
    }
    Ok(total)
}

fn identity(hash: &mut blake3::Hasher, node: &Node) {
    hash.update(&node.name.len().to_le_bytes());
    hash.update(node.name.as_bytes());
    hash.update(node.block.label().as_bytes());
    hash.update(&node.inputs.len().to_le_bytes());
    hash.update(bytemuck::cast_slice(&node.inputs));
}

pub(super) fn node_budget(node: &Node) -> Result<usize> {
    // TextEdit permits 128 Unicode characters. Reserve accounting space for
    // ordinary String growth even when typing returns to the original value.
    // This charges potential bytes; it does not allocate a larger name buffer.
    Ok(bytes(node)? + 1024_usize.saturating_sub(node.name.capacity()))
}

pub(super) fn topology(model: &Model) -> blake3::Hash {
    let mut hash = blake3::Hasher::new();
    hash.update(&model.version.to_le_bytes());
    hash.update(&model.nodes.len().to_le_bytes());
    hash.update(bytemuck::cast_slice(&model.positions));
    for node in &model.nodes {
        identity(&mut hash, node);
    }
    hash.finalize()
}

pub(super) fn node_hash(node: &Node) -> blake3::Hash {
    let mut hash = blake3::Hasher::new();
    identity(&mut hash, node);
    match &node.block {
        Block::Constant { value } => {
            hash.update(&value.to_bits().to_le_bytes());
        }
        Block::Gain { gain } => {
            hash.update(&gain.to_bits().to_le_bytes());
        }
        Block::Bias { bias } => {
            hash.update(&bias.to_bits().to_le_bytes());
        }
        Block::Saturation { lower, upper } => {
            hash.update(bytemuck::cast_slice(&[*lower, *upper]));
        }
        Block::DiscreteIntegrator { initial, gain, .. } => {
            hash.update(bytemuck::cast_slice(&[*initial, *gain]));
        }
        Block::UnitDelay { initial } => {
            hash.update(&initial.to_bits().to_le_bytes());
        }
        Block::Sine {
            amplitude,
            frequency_hz,
            phase,
        } => {
            hash.update(bytemuck::cast_slice(&[*amplitude, *frequency_hz, *phase]));
        }
        Block::Step {
            time,
            before,
            after,
        } => {
            hash.update(bytemuck::cast_slice(&[*time, *before, *after]));
        }
        Block::Filter { b, a } => {
            hash.update(&b.len().to_le_bytes());
            hash.update(bytemuck::cast_slice(b));
            hash.update(bytemuck::cast_slice(a));
        }
        Block::SosFilter { sections } => {
            hash.update(bytemuck::cast_slice(sections));
        }
        Block::Product { operations } => {
            hash.update(&operations.len().to_le_bytes());
            for operation in operations {
                hash.update(&[*operation as u8]);
            }
        }
        Block::Switch {
            criteria,
            threshold,
        } => {
            hash.update(&[*criteria as u8]);
            hash.update(&threshold.to_bits().to_le_bytes());
        }
        Block::Sum | Block::Scope | Block::Abs | Block::UnaryMinus => {}
    }
    hash.finalize()
}

impl Change {
    pub fn insert(model: &mut Model, node: Node) -> Result<Self> {
        bounds(model)?;
        if model.nodes.len() == 512 {
            return Err("Model block limit reached".into());
        }
        let node_bytes = node_budget(&node)?;
        let position = (!model.positions.is_empty()).then(|| {
            let bottom = model
                .positions
                .iter()
                .zip(&model.nodes)
                .map(|(p, n)| p[1] + (24. + n.inputs.len() as f32 * 16.).max(56.))
                .fold(0., f32::max);
            [
                24.,
                (bottom + 44.).min(matrust::simulation::MAX_LAYOUT_COORDINATE),
            ]
        });
        let before = topology(model);
        let index = model.nodes.len();
        let node_hash = node_hash(&node);
        model.nodes.push(node);
        if let Some(position) = position {
            model.positions.push(position);
        }
        Ok(Self {
            index,
            inserted: true,
            node: None,
            node_hash,
            position,
            edges: Vec::new(),
            before,
            after: topology(model),
            node_bytes,
        })
    }

    pub fn delete(model: &mut Model, index: usize) -> Result<Self> {
        bounds(model)?;
        let node = model.nodes.get(index).ok_or("Select a block to delete")?;
        let node_bytes = node_budget(node)?;
        let node_hash = node_hash(node);
        let before = topology(model);
        let mut edges = Vec::new();
        for (target, node) in model.nodes.iter().enumerate() {
            if target == index {
                continue;
            }
            for (port, source) in node.inputs.iter().copied().enumerate() {
                if source >= index && source != usize::MAX {
                    edges.push(Edge {
                        source,
                        target: target as u16,
                        port: port as u8,
                    });
                }
            }
        }
        let position = (!model.positions.is_empty()).then(|| model.positions[index]);
        let mut change = Self {
            index,
            inserted: false,
            node: None,
            node_hash,
            position,
            edges,
            before,
            after: before,
            node_bytes,
        };
        change.remove(model);
        change.after = topology(model);
        Ok(change)
    }

    fn remove(&mut self, model: &mut Model) {
        // Edges address the original indices, before removing the node.
        for edge in &self.edges {
            model.nodes[edge.target as usize].inputs[edge.port as usize] =
                if edge.source == self.index {
                    usize::MAX
                } else {
                    edge.source - 1
                };
        }
        self.node = Some(model.nodes.remove(self.index));
        if self.position.is_some() {
            model.positions.remove(self.index);
            if model.positions.is_empty() {
                model.version = 1;
            }
        }
    }

    pub fn apply(&mut self, model: &mut Model, redo: bool) -> bool {
        let expected = if redo { self.before } else { self.after };
        if bounds(model).is_err() || topology(model) != expected {
            return false;
        }
        let inserting = self.inserted == redo;
        if inserting {
            if self.node.is_none()
                || model.nodes.len() >= 512
                || self.index > model.nodes.len()
                || model.nodes.try_reserve(1).is_err()
                || (self.position.is_some() && model.positions.try_reserve(1).is_err())
            {
                return false;
            }
            model.nodes.insert(self.index, self.node.take().unwrap());
            if let Some(position) = self.position {
                model.positions.insert(self.index, position);
                model.version = 2;
            }
            for edge in &self.edges {
                model.nodes[edge.target as usize].inputs[edge.port as usize] = edge.source;
            }
        } else {
            let Some(node) = model.nodes.get(self.index) else {
                return false;
            };
            // Untracked parameter changes to the node being removed must survive.
            if !bytes(node).is_ok_and(|bytes| bytes <= self.node_bytes)
                || node_hash(node) != self.node_hash
            {
                return false;
            }
            self.remove(model);
        }
        true
    }

    /// Charge the potential retained node even while it is owned by the model.
    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>() + self.node_bytes + self.edges.capacity() * size_of::<Edge>()
    }
}
