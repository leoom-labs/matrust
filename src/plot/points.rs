//! Actual immutable series storage. Virtual corners are values, never fake slices.
use super::coordinates::PatchCoordinates;
use crate::Result;
use std::{
    ops::Range,
    sync::{Arc, Weak, atomic::AtomicBool},
};

#[derive(Clone, Debug)]
pub enum SeriesPoints {
    Dense(Arc<Vec<[f64; 2]>>),
    Patch(Arc<PatchCoordinates>),
}

impl From<Vec<[f64; 2]>> for SeriesPoints {
    fn from(points: Vec<[f64; 2]>) -> Self {
        Self::Dense(Arc::new(points))
    }
}
impl From<Arc<Vec<[f64; 2]>>> for SeriesPoints {
    fn from(points: Arc<Vec<[f64; 2]>>) -> Self {
        Self::Dense(points)
    }
}
impl SeriesPoints {
    pub fn len(&self) -> usize {
        match self {
            Self::Dense(points) => points.len(),
            Self::Patch(points) => points.face_count() * points.corners_per_face(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Ordered original coordinates; this never allocates or changes sample bits.
    pub fn get(&self, index: usize) -> Option<[f64; 2]> {
        match self {
            Self::Dense(points) => points.get(index).copied(),
            Self::Patch(points) => {
                let width = points.corners_per_face();
                points.corner(index / width, index % width)
            }
        }
    }
    pub fn iter(&self) -> PointIter<'_> {
        PointIter {
            points: self,
            indices: 0..self.len(),
        }
    }
    pub fn range(&self, range: Range<usize>) -> Option<PointIter<'_>> {
        (range.start <= range.end && range.end <= self.len()).then_some(PointIter {
            points: self,
            indices: range,
        })
    }
    /// A single fixed-size face scratch value, independent of source storage.
    pub fn face(&self, index: usize, width: usize) -> Option<PointFace> {
        if !matches!(width, 3 | 4) {
            return None;
        }
        let start = index.checked_mul(width)?;
        let values = self.range(start..start.checked_add(width)?)?;
        let mut face = PointFace {
            values: [[0.0; 2]; 4],
            width,
        };
        for (target, source) in face.values.iter_mut().zip(values) {
            *target = source;
        }
        Some(face)
    }
    pub fn dense(&self) -> Option<&Arc<Vec<[f64; 2]>>> {
        match self {
            Self::Dense(points) => Some(points),
            Self::Patch(_) => None,
        }
    }
    /// Explicit dense-only COW access. Compact storage is never expanded here.
    pub fn dense_mut(&mut self) -> Option<&mut Arc<Vec<[f64; 2]>>> {
        match self {
            Self::Dense(points) => Some(points),
            Self::Patch(_) => None,
        }
    }
    pub fn strong_count(&self) -> usize {
        match self {
            Self::Dense(points) => Arc::strong_count(points),
            Self::Patch(points) => Arc::strong_count(points),
        }
    }
    pub fn patch(&self) -> Option<&PatchCoordinates> {
        match self {
            Self::Patch(points) => Some(points),
            Self::Dense(_) => None,
        }
    }
    /// Snapshot identity, with no coordinate scan or retained backing copy.
    pub fn same_storage(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Dense(a), Self::Dense(b)) => Arc::ptr_eq(a, b),
            (Self::Patch(a), Self::Patch(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
    pub fn downgrade(&self) -> WeakSeriesPoints {
        match self {
            Self::Dense(points) => WeakSeriesPoints::Dense(Arc::downgrade(points)),
            Self::Patch(points) => WeakSeriesPoints::Patch(Arc::downgrade(points)),
        }
    }
    /// Process-local live allocation identities for bounded history accounting.
    /// Keys are never serialized. Shared descriptors/backings are counted once
    /// by the caller, including unused capacity and owned descriptor metadata.
    pub fn visit_retained_allocations(&self, mut visit: impl FnMut(usize, usize)) {
        let counters = 2 * std::mem::size_of::<usize>();
        match self {
            Self::Dense(points) => visit(
                Arc::as_ptr(points) as usize,
                points.capacity() * 16 + std::mem::size_of::<Vec<[f64; 2]>>() + counters,
            ),
            Self::Patch(points) => {
                visit(
                    Arc::as_ptr(points) as usize,
                    std::mem::size_of::<PatchCoordinates>()
                        + counters
                        + points.owned_descriptor_bytes(),
                );
                points.visit_retained_backings(visit);
            }
        }
    }
    /// Explicit bounded expansion for file or worker preparation consumers.
    pub fn materialize(&self, cancel: &AtomicBool) -> Result<Vec<[f64; 2]>> {
        crate::dsp::check_cancel(cancel)?;
        if self.len() > super::coordinates::MAX_EXPANDED_VERTICES {
            return Err("Series expansion exceeds the geometry limit".into());
        }
        let mut output = Vec::new();
        output
            .try_reserve_exact(self.len())
            .map_err(|_| "Cannot allocate series coordinates")?;
        for (index, point) in self.iter().enumerate() {
            if index.is_multiple_of(1024) {
                crate::dsp::check_cancel(cancel)?;
            }
            output.push(point);
        }
        crate::dsp::check_cancel(cancel)?;
        Ok(output)
    }
}

impl PartialEq for SeriesPoints {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}
impl<'a> IntoIterator for &'a SeriesPoints {
    type Item = [f64; 2];
    type IntoIter = PointIter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PointFace {
    values: [[f64; 2]; 4],
    width: usize,
}
impl PointFace {
    pub fn as_slice(&self) -> &[[f64; 2]] {
        &self.values[..self.width]
    }
}

#[derive(Clone, Debug)]
pub struct PointIter<'a> {
    points: &'a SeriesPoints,
    indices: Range<usize>,
}
impl Iterator for PointIter<'_> {
    type Item = [f64; 2];
    fn next(&mut self) -> Option<Self::Item> {
        self.indices.next().and_then(|index| self.points.get(index))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.indices.size_hint()
    }
    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        self.indices.nth(n).and_then(|index| self.points.get(index))
    }
}
impl DoubleEndedIterator for PointIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.indices
            .next_back()
            .and_then(|index| self.points.get(index))
    }
}
impl ExactSizeIterator for PointIter<'_> {}
impl std::iter::FusedIterator for PointIter<'_> {}

/// Cache identity cannot keep replaced dense or descriptor payloads alive.
#[derive(Clone, Debug)]
pub enum WeakSeriesPoints {
    Dense(Weak<Vec<[f64; 2]>>),
    Patch(Weak<PatchCoordinates>),
}
impl WeakSeriesPoints {
    pub fn upgrade(&self) -> Option<SeriesPoints> {
        match self {
            Self::Dense(points) => points.upgrade().map(SeriesPoints::Dense),
            Self::Patch(points) => points.upgrade().map(SeriesPoints::Patch),
        }
    }
    pub fn strong_count(&self) -> usize {
        match self {
            Self::Dense(points) => points.strong_count(),
            Self::Patch(points) => points.strong_count(),
        }
    }
    pub fn matches(&self, points: &SeriesPoints) -> bool {
        match (self, points) {
            (Self::Dense(a), SeriesPoints::Dense(b)) => Weak::ptr_eq(a, &Arc::downgrade(b)),
            (Self::Patch(a), SeriesPoints::Patch(b)) => Weak::ptr_eq(a, &Arc::downgrade(b)),
            _ => false,
        }
    }
}

#[cfg(test)]
#[path = "points_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "storage_tests.rs"]
mod storage_tests;
