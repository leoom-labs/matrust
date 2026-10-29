//! Atomic bulk removal: move nodes, remap wires once, keep bounded undo metadata.
use super::topology;
use matrust::{
    Result,
    simulation::{Model, Node},
};

struct Removed {
    index: usize,
    hash: blake3::Hash,
    budget: usize,
    node: Option<Node>,
}

pub(super) struct Change {
    removed: Vec<Removed>,
    inputs: Vec<Vec<usize>>,
    positions: Vec<[f32; 2]>,
    version: u32,
    before: blake3::Hash,
    after: blake3::Hash,
}

impl Change {
    pub fn delete(model: &mut Model, indices: &[usize], limit: usize) -> Result<Self> {
        topology::bounds(model)?;
        if indices.is_empty()
            || indices.len() > model.nodes.len()
            || indices.windows(2).any(|pair| pair[0] >= pair[1])
            || indices
                .last()
                .is_some_and(|index| *index >= model.nodes.len())
        {
            return Err("Select valid, distinct blocks to delete".into());
        }
        let mut removed = Vec::with_capacity(indices.len());
        let mut selected_bytes = 0usize;
        for &index in indices {
            let budget = topology::node_budget(&model.nodes[index])?;
            selected_bytes += budget;
            if selected_bytes > limit {
                return Err(
                    "Selected blocks exceed the 4 MiB undo limit; delete a smaller group".into(),
                );
            }
            removed.push(Removed {
                index,
                hash: blake3::Hash::from([0; 32]),
                budget,
                node: None,
            });
        }
        let inputs = model
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| {
                if indices.binary_search(&index).is_ok() {
                    Vec::new()
                } else {
                    node.inputs.clone()
                }
            })
            .collect();
        let before = topology::topology(model);
        let mut change = Self {
            removed,
            inputs,
            positions: model.positions.clone(),
            version: model.version,
            before,
            after: before,
        };
        if change.retained_bytes() > limit {
            return Err(
                "Selected blocks exceed the 4 MiB undo limit; delete a smaller group".into(),
            );
        }
        // Reject oversized selections before scanning any coefficient buffers.
        for item in &mut change.removed {
            item.hash = topology::node_hash(&model.nodes[item.index]);
        }
        if !change.remove(model) {
            return Err("Could not reserve memory for group deletion".into());
        }
        change.after = topology::topology(model);
        Ok(change)
    }

    pub fn indices(&self) -> Vec<usize> {
        self.removed.iter().map(|item| item.index).collect()
    }
    pub fn first(&self) -> usize {
        self.removed[0].index
    }

    fn remove(&mut self, model: &mut Model) -> bool {
        // Reserve before moving any nodes; all remaining work is infallible.
        let mut kept = Vec::new();
        if kept
            .try_reserve_exact(model.nodes.len() - self.removed.len())
            .is_err()
        {
            return false;
        }
        let mut deleted = [false; 512];
        for item in &self.removed {
            deleted[item.index] = true;
        }
        let mut prefix = [0usize; 513];
        for index in 0..model.nodes.len() {
            prefix[index + 1] = prefix[index] + usize::from(deleted[index]);
        }
        let count = model.nodes.len();
        let mut removed = self.removed.iter_mut().peekable();
        for (index, mut node) in std::mem::take(&mut model.nodes).into_iter().enumerate() {
            if removed.peek().is_some_and(|item| item.index == index) {
                removed.next().unwrap().node = Some(node);
            } else {
                for source in &mut node.inputs {
                    if *source == usize::MAX {
                        continue;
                    }
                    *source = if *source < count && deleted[*source] {
                        usize::MAX
                    } else {
                        *source - prefix[(*source).min(count)]
                    };
                }
                kept.push(node);
            }
        }
        model.nodes = kept;
        let mut index = 0;
        model.positions.retain(|_| {
            let keep = !deleted[index];
            index += 1;
            keep
        });
        if model.positions.is_empty() {
            model.version = 1;
        }
        true
    }

    pub fn apply(&mut self, model: &mut Model, redo: bool) -> bool {
        if topology::bounds(model).is_err()
            || topology::topology(model) != if redo { self.before } else { self.after }
        {
            return false;
        }
        if redo {
            // Preflight every removed node before mutation: a late stale edit
            // cannot leave half the selection removed.
            if self.removed.iter().any(|item| {
                item.node.is_some()
                    || model.nodes.get(item.index).is_none_or(|node| {
                        topology::node_hash(node) != item.hash
                            || !topology::bytes(node).is_ok_and(|bytes| bytes <= item.budget)
                    })
            }) {
                return false;
            }
            self.remove(model)
        } else {
            let count = self.inputs.len();
            if model.nodes.len() + self.removed.len() != count
                || count > 512
                || self.removed.iter().any(|item| item.node.is_none())
            {
                return false;
            }
            let mut restored = Vec::new();
            if restored.try_reserve_exact(count).is_err()
                || model
                    .positions
                    .try_reserve(self.positions.len().saturating_sub(model.positions.len()))
                    .is_err()
            {
                return false;
            }
            let mut kept = std::mem::take(&mut model.nodes).into_iter();
            let mut removed = self.removed.iter_mut().peekable();
            for index in 0..count {
                let node = if removed.peek().is_some_and(|item| item.index == index) {
                    removed.next().unwrap().node.take().unwrap()
                } else {
                    let mut node = kept.next().unwrap();
                    node.inputs.copy_from_slice(&self.inputs[index]);
                    node
                };
                restored.push(node);
            }
            model.nodes = restored;
            model.positions.clear();
            model.positions.extend_from_slice(&self.positions);
            model.version = self.version;
            true
        }
    }

    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            + self.removed.capacity() * size_of::<Removed>()
            + self.removed.iter().map(|item| item.budget).sum::<usize>()
            + self.inputs.capacity() * size_of::<Vec<usize>>()
            + self
                .inputs
                .iter()
                .map(|inputs| inputs.capacity() * size_of::<usize>())
                .sum::<usize>()
            + self.positions.capacity() * size_of::<[f32; 2]>()
    }
}
