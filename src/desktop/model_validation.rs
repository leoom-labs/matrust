//! Cache only the editor's readiness hint. Model::run always validates afresh.
//! Content stamps cover public model fields, including in-place coefficient edits.
use super::topology;
use matrust::simulation::{Block, Model};
use rayon::prelude::*;

#[derive(Default)]
pub struct Cache {
    stamp: Option<blake3::Hash>,
    error: Option<String>,
    nodes: Vec<blake3::Hash>,
    #[cfg(test)]
    evaluations: usize,
}

impl Cache {
    pub fn status(&mut self, model: &Model, use_workers: bool) -> Result<(), &str> {
        let Some(coefficients) = bounded_coefficients(model) else {
            // Avoid scanning oversized externally supplied drafts on the UI thread.
            self.stamp = None;
            return Err("Model exceeds editor validation bounds");
        };
        // Small models and previously invalid drafts are cheaper to validate
        // directly. In particular, an invalid denominator fails very quickly.
        if coefficients < 4096 || self.error.is_some() {
            self.error = model.validate().err();
            self.stamp = None;
            #[cfg(test)]
            {
                self.evaluations += 1;
            }
            if coefficients >= 4096 && self.error.is_none() {
                self.stamp = Some(stamp(model, &mut self.nodes, coefficients, use_workers));
            }
        } else {
            let stamp = stamp(model, &mut self.nodes, coefficients, use_workers);
            if self.stamp != Some(stamp) {
                self.error = model.validate().err();
                self.stamp = Some(stamp);
                #[cfg(test)]
                {
                    self.evaluations += 1;
                }
            }
        }
        self.error.as_deref().map_or(Ok(()), Err)
    }
}

fn bounded_coefficients(model: &Model) -> Option<usize> {
    if model.nodes.len() > 512
        || model.positions.len() > 512
        || model.nodes.iter().any(|node| {
            node.name.len() > 512
                || node.inputs.len() > 32
                || match &node.block {
                    Block::Filter { b, a } => b.len().max(a.len()) > 4097,
                    Block::Product { operations } => !(1..=32).contains(&operations.len()),
                    Block::SosFilter { sections } => sections.len() > 64,
                    _ => false,
                }
        })
    {
        return None;
    }
    Some(
        model
            .nodes
            .iter()
            .map(|node| match &node.block {
                Block::Filter { b, a } => b.len() + a.len(),
                Block::SosFilter { sections } => sections.len() * 6,
                _ => 0,
            })
            .sum(),
    )
}

// File gestures reuse the complete storage stamp, never the readiness result.
// With use_workers=false, stamp never resizes or writes the empty scratch Vec.
pub(crate) fn gesture_stamp(model: &Model) -> Option<blake3::Hash> {
    let coefficients = bounded_coefficients(model)?;
    Some(stamp(model, &mut Vec::new(), coefficients, false))
}

fn stamp(
    model: &Model,
    nodes: &mut Vec<blake3::Hash>,
    coefficients: usize,
    use_workers: bool,
) -> blake3::Hash {
    let mut hash = blake3::Hasher::new();
    hash.update(&model.version.to_le_bytes());
    hash.update(&model.sample_time.to_bits().to_le_bytes());
    hash.update(&model.steps.to_le_bytes());
    hash.update(&model.positions.len().to_le_bytes());
    hash.update(bytemuck::cast_slice(&model.positions));
    hash.update(&model.nodes.len().to_le_bytes());
    // A conservative 8-MiB threshold keeps small work serial. Reuse at most
    // 512 digests (16 KiB) and the application's existing bounded Rayon pool.
    // A busy desktop calculation may occupy all workers. Never queue a UI
    // readiness check behind that work; the caller disables parallel dispatch.
    if use_workers && coefficients >= 1_048_576 && rayon::current_num_threads() > 1 {
        nodes.resize(model.nodes.len(), blake3::Hash::from([0; 32]));
        nodes
            .par_iter_mut()
            .zip(&model.nodes)
            .for_each(|(digest, node)| {
                *digest = topology::node_hash(node);
            });
        for digest in nodes {
            hash.update(digest.as_bytes());
        }
    } else {
        for node in &model.nodes {
            hash.update(topology::node_hash(node).as_bytes());
        }
    }
    hash.finalize()
}

#[cfg(test)]
#[path = "model_validation_tests.rs"]
mod tests;
