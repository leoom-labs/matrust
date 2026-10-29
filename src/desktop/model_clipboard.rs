//! Versioned native clipboard fragments and atomic, ownership-moving paste history.
use super::{Model, layout, topology};
use matrust::{Result, simulation::Node};
use serde::{Deserialize, Serialize};

const PREFIX: &str = "MATRUST_BLOCKS_V1\n";
const TEXT_LIMIT: usize = 16 * 1024 * 1024;
const DATA_LIMIT: usize = 4 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Fragment {
    nodes: Vec<Node>,
    positions: Vec<[f32; 2]>,
}
impl Fragment {
    fn check(&self) -> Result<()> {
        if self.nodes.is_empty()
            || self.nodes.len() > 512
            || self.positions.len() != self.nodes.len()
        {
            return Err("Clipboard must contain 1 to 512 positioned blocks".into());
        }
        let mut bytes = self.nodes.capacity() * size_of::<Node>() + self.positions.capacity() * 8;
        for node in &self.nodes {
            bytes += topology::node_budget(node)?;
            if bytes > DATA_LIMIT
                || node
                    .inputs
                    .iter()
                    .any(|source| *source != usize::MAX && *source >= self.nodes.len())
            {
                return Err(
                    "Clipboard blocks exceed the storage limit or contain invalid internal wires"
                        .into(),
                );
            }
        }
        if self.positions.iter().flatten().any(|value| {
            !value.is_finite()
                || !(0. ..=matrust::simulation::MAX_LAYOUT_COORDINATE).contains(value)
        }) {
            return Err("Clipboard contains invalid block positions".into());
        }
        Ok(())
    }
    pub fn capture(model: &Model, indices: &[usize], width: f32) -> Result<Self> {
        topology::bounds(model)?;
        if indices.is_empty()
            || indices.windows(2).any(|p| p[0] >= p[1])
            || indices
                .last()
                .is_none_or(|index| *index >= model.nodes.len())
        {
            return Err("Select distinct blocks to copy".into());
        }
        let mut budget = indices.len() * (size_of::<Node>() + 8);
        for index in indices {
            budget += topology::node_budget(&model.nodes[*index])?;
            if budget > DATA_LIMIT {
                return Err(
                    "Copied blocks exceed the 4 MiB storage limit; select a smaller group".into(),
                );
            }
        }
        let mut mapping = [usize::MAX; 512];
        for (local, index) in indices.iter().enumerate() {
            mapping[*index] = local;
        }
        let boxes = layout::boxes(model, width);
        let positions = indices
            .iter()
            .map(|index| {
                model
                    .positions
                    .get(*index)
                    .copied()
                    .unwrap_or([boxes[*index].left(), boxes[*index].top()])
            })
            .collect();
        let nodes = indices
            .iter()
            .map(|index| {
                let mut node = model.nodes[*index].clone();
                for source in &mut node.inputs {
                    *source = mapping.get(*source).copied().unwrap_or(usize::MAX);
                }
                node
            })
            .collect();
        let fragment = Self { nodes, positions };
        fragment.check()?;
        Ok(fragment)
    }
    pub fn encode(&self) -> Result<String> {
        use matrust::simulation::Block;
        for node in &self.nodes {
            let finite = match &node.block {
                Block::Constant { value } => value.is_finite(),
                Block::Gain { gain } => gain.is_finite(),
                Block::Bias { bias } => bias.is_finite(),
                Block::Saturation { lower, upper } => lower.is_finite() && upper.is_finite(),
                Block::UnitDelay { initial } => initial.is_finite(),
                Block::DiscreteIntegrator { initial, gain, .. } => {
                    initial.is_finite() && gain.is_finite()
                }
                Block::Sine {
                    amplitude,
                    frequency_hz,
                    phase,
                } => [amplitude, frequency_hz, phase]
                    .iter()
                    .all(|v| v.is_finite()),
                Block::Step {
                    time,
                    before,
                    after,
                } => [time, before, after].iter().all(|v| v.is_finite()),
                Block::Filter { b, a } => b.iter().chain(a).all(|v| v.is_finite()),
                Block::SosFilter { sections } => sections.iter().flatten().all(|v| v.is_finite()),
                Block::Switch { threshold, .. } => threshold.is_finite(),
                Block::Product { operations } => (1..=32).contains(&operations.len()),
                Block::Sum | Block::Scope | Block::Abs | Block::UnaryMinus => true,
            };
            if !finite {
                return Err("Clipboard export requires finite block parameters".into());
            }
        }
        let body = serde_json::to_string(self).map_err(|error| error.to_string())?;
        if body.len() + PREFIX.len() > TEXT_LIMIT {
            return Err("Clipboard text exceeds 16 MiB".into());
        }
        Ok(format!("{PREFIX}{body}"))
    }
    pub fn decode(text: &str) -> Result<Self> {
        if text.len() > TEXT_LIMIT {
            return Err("Clipboard text exceeds 16 MiB".into());
        }
        let body = text
            .strip_prefix(PREFIX)
            .ok_or("Clipboard does not contain MatRust blocks")?;
        let value: Self =
            serde_json::from_str(body).map_err(|_| "Invalid MatRust block clipboard")?;
        value.check()?;
        Ok(value)
    }
}

struct InvalidEdge {
    target: usize,
    port: usize,
    source: usize,
}
pub(super) struct Change {
    pub first: usize,
    count: usize,
    nodes: Vec<Node>,
    hashes: Vec<blake3::Hash>,
    budgets: Vec<usize>,
    edges: Vec<InvalidEdge>,
    before_positions: Vec<[f32; 2]>,
    after_positions: Vec<[f32; 2]>,
    before_version: u32,
    before: blake3::Hash,
    after: blake3::Hash,
    pub previous_selection: Vec<usize>,
}
impl Change {
    pub fn insert(
        model: &mut Model,
        mut fragment: Fragment,
        width: f32,
        offset: f32,
        previous_selection: Vec<usize>,
        limit: usize,
    ) -> Result<Self> {
        topology::bounds(model)?;
        fragment.check()?;
        let first = model.nodes.len();
        let count = fragment.nodes.len();
        if first + count > 512 {
            return Err("Paste would exceed the 512-block model limit".into());
        }
        if !offset.is_finite() || offset < 0. {
            return Err("Invalid paste offset".into());
        }
        let delta = layout::bounded_delta(fragment.positions.iter().copied(), [offset, offset]);
        for position in &mut fragment.positions {
            position[0] += delta[0];
            position[1] += delta[1];
        }
        let mut names: std::collections::HashSet<String> =
            model.nodes.iter().map(|node| node.name.clone()).collect();
        for node in &mut fragment.nodes {
            let base: String = node.name.chars().take(100).collect();
            let mut name = format!("{base} copy");
            let mut suffix = 2;
            while names.contains(&name) {
                name = format!("{base} copy {suffix}");
                suffix += 1;
            }
            names.insert(name.clone());
            node.name = name;
            for source in &mut node.inputs {
                if *source != usize::MAX {
                    *source += first;
                }
            }
        }
        let mut edges = Vec::new();
        for (target, node) in model.nodes.iter().enumerate() {
            for (port, source) in node.inputs.iter().copied().enumerate() {
                if source >= first && source != usize::MAX {
                    if source
                        .checked_add(count)
                        .is_none_or(|shifted| shifted == usize::MAX)
                    {
                        return Err("An invalid draft wire cannot be preserved during paste".into());
                    }
                    edges.push(InvalidEdge {
                        target,
                        port,
                        source,
                    });
                }
            }
        }
        let mut after_positions = if model.positions.is_empty() {
            layout::boxes(model, width)
                .iter()
                .map(|rect| [rect.left(), rect.top()])
                .collect::<Vec<_>>()
        } else {
            model.positions.clone()
        };
        after_positions.extend_from_slice(&fragment.positions);
        let budgets = fragment
            .nodes
            .iter()
            .map(topology::node_budget)
            .collect::<Result<Vec<_>>>()?;
        let before = topology::topology(model);
        let mut change = Self {
            first,
            count,
            nodes: fragment.nodes,
            hashes: Vec::new(),
            budgets,
            edges,
            before_positions: model.positions.clone(),
            after_positions,
            before_version: model.version,
            before,
            after: before,
            previous_selection,
        };
        // Charge the hash storage before hashing any coefficient buffers.
        change.hashes.reserve_exact(count);
        if change.retained_bytes() > limit {
            return Err("Pasted blocks exceed the 4 MiB undo limit; select a smaller group".into());
        }
        change
            .hashes
            .extend(change.nodes.iter().map(topology::node_hash));
        if !change.append(model) {
            return Err("Could not reserve memory for pasted blocks".into());
        }
        change.after = topology::topology(model);
        Ok(change)
    }
    pub fn indices(&self) -> Vec<usize> {
        (self.first..self.first + self.count).collect()
    }
    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            + self.nodes.capacity() * size_of::<Node>()
            + self.budgets.iter().sum::<usize>()
            + self.budgets.capacity() * size_of::<usize>()
            + self.hashes.capacity() * size_of::<blake3::Hash>()
            + self.edges.capacity() * size_of::<InvalidEdge>()
            + (self.before_positions.capacity() + self.after_positions.capacity()) * 8
            + self.previous_selection.capacity() * size_of::<usize>()
    }
    fn append(&mut self, model: &mut Model) -> bool {
        if model.nodes.try_reserve(self.count).is_err()
            || model
                .positions
                .try_reserve(
                    self.after_positions
                        .len()
                        .saturating_sub(model.positions.len()),
                )
                .is_err()
        {
            return false;
        }
        for edge in &self.edges {
            model.nodes[edge.target].inputs[edge.port] = edge.source + self.count;
        }
        model.nodes.append(&mut self.nodes);
        model.positions.clear();
        model.positions.extend_from_slice(&self.after_positions);
        model.version = 2;
        true
    }
    pub fn apply(&mut self, model: &mut Model, redo: bool) -> bool {
        if topology::bounds(model).is_err()
            || topology::topology(model) != if redo { self.before } else { self.after }
        {
            return false;
        }
        if redo {
            self.nodes.len() == self.count && self.append(model)
        } else {
            if !self.nodes.is_empty()
                || model.nodes[self.first..]
                    .iter()
                    .enumerate()
                    .any(|(index, node)| {
                        topology::node_hash(node) != self.hashes[index]
                            || !topology::bytes(node)
                                .is_ok_and(|bytes| bytes <= self.budgets[index])
                    })
            {
                return false;
            }
            // Preflight all nodes before moving any owned buffers.
            self.nodes.extend(model.nodes.drain(self.first..));
            for edge in &self.edges {
                model.nodes[edge.target].inputs[edge.port] = edge.source;
            }
            model.positions.clear();
            model.positions.extend_from_slice(&self.before_positions);
            model.version = self.before_version;
            true
        }
    }
}
