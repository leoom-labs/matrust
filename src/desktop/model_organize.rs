//! Bounded spatial selection and geometry-only organization of selected blocks.
use super::*;
use matrust::simulation::MAX_LAYOUT_COORDINATE;

pub(super) struct Marquee {
    pub before: layout::Snapshot,
    pub start: Pos2,
    pub end: Pos2,
    pub base: Vec<usize>,
}
impl Marquee {
    pub fn contains(&self, index: usize, block: Rect) -> bool {
        self.base.binary_search(&index).is_ok()
            || Rect::from_two_pos(self.start, self.end).intersects(block)
    }
}

#[derive(Clone, Copy)]
pub(super) enum Operation {
    Left,
    CenterX,
    Right,
    Top,
    CenterY,
    Bottom,
    SpaceX,
    SpaceY,
}
impl Operation {
    pub const ALL: [(Self, &'static str); 8] = [
        (Self::Left, "Align left"),
        (Self::CenterX, "Align horizontal centers"),
        (Self::Right, "Align right"),
        (Self::Top, "Align top"),
        (Self::CenterY, "Align vertical centers"),
        (Self::Bottom, "Align bottom"),
        (Self::SpaceX, "Distribute horizontal centers"),
        (Self::SpaceY, "Distribute vertical centers"),
    ];
    pub fn minimum(self) -> usize {
        if matches!(self, Self::SpaceX | Self::SpaceY) {
            3
        } else {
            2
        }
    }
}

impl Canvas {
    pub(super) fn organize(
        &mut self,
        model: &mut Model,
        width: f32,
        operation: Operation,
    ) -> matrust::Result<bool> {
        model.validate_layout()?;
        let indices = self.selection.indices();
        if model.nodes.len() > 512
            || indices.len() < operation.minimum()
            || indices.iter().any(|index| *index >= model.nodes.len())
        {
            return Ok(false);
        }
        let mut boxes = layout::boxes(model, width);
        for (rect, position) in boxes.iter_mut().zip(&model.positions) {
            *rect = Rect::from_min_size(Pos2::new(position[0], position[1]), rect.size());
        }
        let bounds = indices
            .iter()
            .fold(Rect::NOTHING, |area, index| area.union(boxes[*index]));
        let axis = usize::from(matches!(
            operation,
            Operation::Top | Operation::CenterY | Operation::Bottom | Operation::SpaceY
        ));
        let mut positions: Vec<_> = boxes.iter().map(|rect| [rect.left(), rect.top()]).collect();
        if matches!(operation, Operation::SpaceX | Operation::SpaceY) {
            let mut ordered = indices.to_vec();
            ordered.sort_by(|a, b| {
                boxes[*a].center()[axis]
                    .total_cmp(&boxes[*b].center()[axis])
                    .then(a.cmp(b))
            });
            let first = boxes[ordered[0]].center()[axis];
            let last = boxes[*ordered.last().unwrap()].center()[axis];
            // Leave endpoints bit-for-bit unchanged; only interior centers move.
            for (rank, index) in ordered.iter().enumerate().skip(1).take(ordered.len() - 2) {
                positions[*index][axis] = first
                    + (last - first) * (rank as f32 / (ordered.len() - 1) as f32)
                    - boxes[*index].size()[axis] * 0.5;
            }
        } else {
            for index in indices {
                positions[*index][axis] = match operation {
                    Operation::Left | Operation::Top => bounds.min[axis],
                    Operation::Right | Operation::Bottom => {
                        bounds.max[axis] - boxes[*index].size()[axis]
                    }
                    _ => bounds.center()[axis] - boxes[*index].size()[axis] * 0.5,
                };
            }
        }
        if positions
            .iter()
            .flatten()
            .any(|value| !value.is_finite() || !(0. ..=MAX_LAYOUT_COORDINATE).contains(value))
        {
            return Err("Organized blocks would exceed the layout bounds".into());
        }
        if positions
            .iter()
            .zip(&boxes)
            .all(|(position, rect)| *position == [rect.left(), rect.top()])
        {
            return Ok(false);
        }
        let before = layout::Snapshot::capture(model);
        model.set_layout(positions)?;
        self.cancel_connection();
        self.push(Change::Layout {
            before,
            after: layout::Snapshot::capture(model),
        });
        Ok(true)
    }
}
