//! Small layout snapshots, separate from numerical/block parameter storage.
use super::{Model, Pos2, Rect, Vec2};
use matrust::simulation::MAX_LAYOUT_COORDINATE;

pub(super) fn boxes(model: &Model, width: f32) -> Vec<Rect> {
    let columns = ((width - 24.) / 170.).floor().clamp(1., 5.) as usize;
    let mut boxes = Vec::with_capacity(model.nodes.len());
    let mut top = 24.;
    for row in model.nodes.chunks(columns) {
        let row_height = row
            .iter()
            .map(|node| (24. + node.inputs.len() as f32 * 16.).max(56.))
            .fold(56., f32::max);
        for (column, node) in row.iter().enumerate() {
            let height = (24. + node.inputs.len() as f32 * 16.).max(56.);
            boxes.push(Rect::from_min_size(
                Pos2::new(24. + column as f32 * 170., top),
                Vec2::new(126., height),
            ));
        }
        top += row_height + 44.;
    }
    boxes
}

#[derive(Clone)]
pub(super) struct Snapshot {
    pub positions: Vec<[f32; 2]>,
    version: u32,
    nodes: usize,
}
impl Snapshot {
    pub fn capture(model: &Model) -> Self {
        Self {
            positions: model.positions.clone(),
            version: model.version,
            nodes: model.nodes.len(),
        }
    }
    pub fn matches(&self, model: &Model) -> bool {
        self.version == model.version
            && self.nodes == model.nodes.len()
            && bytemuck::cast_slice::<[f32; 2], u32>(&self.positions)
                == bytemuck::cast_slice::<[f32; 2], u32>(&model.positions)
    }
    pub fn apply(&self, model: &mut Model) -> bool {
        self.nodes == model.nodes.len() && model.set_layout(self.positions.clone()).is_ok()
    }
}

pub(super) struct Moving {
    pub before: Snapshot,
    pub positions: Vec<[f32; 2]>,
    pub node: usize,
    origin: Pos2,
    start: [f32; 2],
    starts: Vec<(usize, [f32; 2])>,
}
impl Moving {
    pub fn new(model: &Model, positions: Vec<[f32; 2]>, node: usize, origin: Pos2) -> Self {
        let start = positions[node];
        Self {
            before: Snapshot::capture(model),
            positions,
            node,
            origin,
            start,
            starts: vec![(node, start)],
        }
    }
    pub fn changed(&self) -> bool {
        self.starts
            .iter()
            .any(|(index, start)| *start != self.positions[*index])
    }
    pub fn set_group(&mut self, indices: &[usize]) {
        self.starts = indices
            .iter()
            .copied()
            .filter(|index| *index < self.positions.len())
            .take(512)
            .map(|index| (index, self.positions[index]))
            .collect();
        if self.starts.is_empty() {
            self.starts.push((self.node, self.start));
        }
    }
    pub fn update(&mut self, pointer: Pos2, unsnapped: bool) {
        let delta = pointer - self.origin;
        let mut position = [self.start[0] + delta.x, self.start[1] + delta.y];
        for value in &mut position {
            if !unsnapped {
                *value = (*value / 8.).round() * 8.;
            }
            *value = value.clamp(0., MAX_LAYOUT_COORDINATE);
        }
        let delta = bounded_delta(
            self.starts.iter().map(|(_, start)| *start),
            [position[0] - self.start[0], position[1] - self.start[1]],
        );
        for (index, start) in &self.starts {
            self.positions[*index] = [start[0] + delta[0], start[1] + delta[1]];
        }
    }
}

/// Clamp one shared translation so group spacing survives coordinate boundaries.
pub(super) fn bounded_delta(
    starts: impl Iterator<Item = [f32; 2]>,
    mut delta: [f32; 2],
) -> [f32; 2] {
    for start in starts {
        for axis in 0..2 {
            delta[axis] = delta[axis].clamp(-start[axis], MAX_LAYOUT_COORDINATE - start[axis]);
        }
    }
    delta
}
