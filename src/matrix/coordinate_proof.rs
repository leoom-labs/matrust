//! Bounded facts carried beside the unchanged eager column-major allocation.
//! No expression node owns another Matrix or proof. Unsupported operations drop facts.
use super::Matrix;
use crate::Result;
use crate::plot::coordinates::{Coordinate, CoordinateLane, PatchCoordinates, ScalarStep};
use std::sync::{Arc, atomic::AtomicBool};

#[derive(Clone, Debug)]
pub(super) struct CoordinateProof {
    lanes: [Option<Coordinate>; 4],
    count: usize,
    len: usize,
    interleaved: bool,
}

impl CoordinateProof {
    fn one(value: Coordinate, len: usize) -> Self {
        Self {
            lanes: [Some(value), None, None, None],
            count: 1,
            len,
            interleaved: false,
        }
    }
    fn seed(matrix: &Matrix) -> Option<Self> {
        if matrix.is_empty() {
            return None;
        }
        if let Some(proof) = &matrix.coordinates {
            return Some((**proof).clone());
        }
        if let Some(value) = matrix.uniform
            && matrix.len() <= crate::plot::coordinates::MAX_SOURCE_UNITS
        {
            return Some(Self::one(Coordinate::Constant(value), matrix.len()));
        }
        if matrix.rows() != 1 && matrix.cols() != 1 {
            return None;
        }
        Some(Self::one(
            Coordinate::Lane(CoordinateLane::matrix_linear(matrix, 0, matrix.len()).ok()?),
            matrix.len(),
        ))
    }
    fn bounded(self) -> Option<Arc<Self>> {
        // At most two retained allocations, independently of aliases and step count.
        let mut identities = [0usize; 2];
        let mut count = 0;
        let mut bytes = 0usize;
        for lane in self.lanes.iter().flatten() {
            if let Coordinate::Lane(lane) = lane {
                let (identity, retained) = lane.matrix_backing()?;
                if !identities[..count].contains(&identity) {
                    if count == 2 {
                        return None;
                    }
                    identities[count] = identity;
                    count += 1;
                    bytes = bytes.checked_add(retained)?;
                }
            }
        }
        if bytes > crate::plot::coordinates::MAX_RETAINED_BYTES {
            return None;
        }
        Some(Arc::new(self))
    }
    fn step(mut self, step: ScalarStep) -> Option<Arc<Self>> {
        for value in self.lanes.iter_mut().flatten() {
            *value = match value {
                Coordinate::Constant(value) => {
                    let result = step.apply(*value);
                    if result.is_nan() {
                        return None;
                    }
                    Coordinate::Constant(result)
                }
                Coordinate::Lane(lane) => Coordinate::Lane(lane.clone().then(step).ok()?),
            };
        }
        self.bounded()
    }
    fn slice(&self, start: usize, step: usize, len: usize) -> Option<Arc<Self>> {
        let end = if len == 0 {
            start
        } else {
            start
                .checked_add((len - 1).checked_mul(step)?)?
                .checked_add(1)?
        };
        if self.count != 1 || !matches!(step, 1 | 2) || end > self.len {
            return None;
        }
        let value = match self.lanes[0].as_ref()? {
            Coordinate::Constant(v) => Coordinate::Constant(*v),
            Coordinate::Lane(lane) => {
                Coordinate::Lane(lane.clone().strided_slice(start, step, len).ok()?)
            }
        };
        Self::one(value, len).bounded()
    }
    fn transpose(&self, rows: usize, cols: usize) -> Option<Arc<Self>> {
        if self.count == 1 && (rows == 1 || cols == 1) {
            return Some(Arc::new(self.clone()));
        }
        if (!self.interleaved && rows == self.len && cols == self.count)
            || (self.interleaved && rows == self.count && cols == self.len)
        {
            let mut proof = self.clone();
            proof.interleaved = !proof.interleaved;
            Some(Arc::new(proof))
        } else {
            None
        }
    }
    pub(super) fn concatenate(blocks: &[Vec<Matrix>]) -> Option<Arc<Self>> {
        // General matrix concatenation remains eager. Recognize ordinary column
        // concatenation only; at most four lanes, with no syntax/name recognition.
        let row = blocks.first()?;
        if blocks.len() != 1 || row.is_empty() || row.len() > 4 {
            return None;
        }
        let len = row[0].rows();
        let mut result = Self {
            lanes: [None, None, None, None],
            count: 0,
            len,
            interleaved: false,
        };
        for matrix in row {
            if matrix.cols() != 1 || matrix.rows() != len {
                return None;
            }
            let proof = Self::seed(matrix)?;
            if proof.count != 1 || proof.len != len {
                return None;
            }
            result.lanes[result.count] = proof.lanes[0].clone();
            result.count += 1;
        }
        result.bounded()
    }
    fn corners(&self, rows: usize, cols: usize) -> Option<[Coordinate; 4]> {
        if rows != 4 || cols != self.len || self.count != 4 || !self.interleaved {
            return None;
        }
        Some(std::array::from_fn(|i| {
            self.lanes[i].as_ref().unwrap().clone()
        }))
    }
    fn retained_extra(&self, visible: usize) -> usize {
        let mut seen = [visible, 0, 0];
        let mut count = 1;
        // Includes the Arc counters and inline bounded descriptor allocation.
        let mut bytes = std::mem::size_of::<Self>() + 2 * std::mem::size_of::<usize>();
        for value in self.lanes.iter().flatten() {
            if let Coordinate::Lane(lane) = value {
                let (identity, retained) = lane.matrix_backing().expect("matrix-only proof");
                if !seen[..count].contains(&identity) {
                    seen[count] = identity;
                    count += 1;
                    bytes += retained;
                }
            }
        }
        bytes
    }
}

impl Matrix {
    /// Logical workspace charge, including the full capacity of hidden proof
    /// backings. Aliased variable bindings are charged independently as before.
    pub fn retained_bytes(&self) -> usize {
        match &self.coordinates {
            None => self.len() * 8,
            Some(proof) => {
                self.storage.data.as_vec().capacity() * 8
                    + proof.retained_extra(Arc::as_ptr(&self.storage) as usize)
            }
        }
    }
    pub fn has_coordinate_proof(&self) -> bool {
        self.coordinates.is_some()
    }
    /// Explicit eager fallback; retains the very same column-major sample Arc.
    pub fn without_coordinate_proof(mut self) -> Self {
        self.coordinates = None;
        self
    }
    pub(super) fn transpose_proof(&self) -> Option<Arc<CoordinateProof>> {
        self.coordinates
            .as_ref()?
            .transpose(self.rows(), self.cols())
    }
    /// Called only after the ordinary eager gather copied the selected values.
    /// Facts never change the kernel or authorize an otherwise invalid index.
    pub(crate) fn inherit_coordinate_slice(&mut self, input: &Self, start: usize) {
        self.inherit_coordinate_stride(input, start, 1);
    }
    pub(crate) fn inherit_coordinate_stride(&mut self, input: &Self, start: usize, step: usize) {
        if !matches!(step, 1 | 2) {
            return;
        }
        let end = if self.is_empty() {
            Some(start)
        } else {
            (self.len() - 1)
                .checked_mul(step)
                .and_then(|n| start.checked_add(n))
                .and_then(|n| n.checked_add(1))
        };
        if end.is_none_or(|end| end > input.len()) {
            return;
        }
        self.coordinates = if let Some(proof) = &input.coordinates {
            proof.slice(start, step, self.len())
        } else {
            CoordinateLane::matrix_strided(input, start, step, self.len())
                .ok()
                .and_then(|lane| CoordinateProof::one(Coordinate::Lane(lane), self.len()).bounded())
        };
    }
    /// Named arithmetic preserves operation order; arbitrary map/zip closures
    /// intentionally do not preserve coordinate facts.
    pub fn coordinate_add(&self, rhs: &Self) -> Result<Self> {
        self.coordinate_binary(rhs, |a, b| a + b, ScalarStep::Add, ScalarStep::LeftAdd)
    }
    pub fn coordinate_subtract(&self, rhs: &Self) -> Result<Self> {
        self.coordinate_binary(
            rhs,
            |a, b| a - b,
            ScalarStep::Subtract,
            ScalarStep::LeftSubtract,
        )
    }
    pub fn coordinate_multiply(&self, rhs: &Self) -> Result<Self> {
        self.coordinate_binary(
            rhs,
            |a, b| a * b,
            ScalarStep::Multiply,
            ScalarStep::LeftMultiply,
        )
    }
    fn coordinate_binary(
        &self,
        rhs: &Self,
        operation: impl Fn(f64, f64) -> f64 + Sync + Send,
        right: fn(f64) -> ScalarStep,
        left: fn(f64) -> ScalarStep,
    ) -> Result<Self> {
        let mut result = self.zip(rhs, operation)?;
        result.coordinates = if rhs.len() == 1 {
            CoordinateProof::seed(self).and_then(|proof| proof.step(right(rhs.data()[0])))
        } else if self.len() == 1 {
            CoordinateProof::seed(rhs).and_then(|proof| proof.step(left(self.data()[0])))
        } else {
            None
        };
        Ok(result)
    }
    /// A real future Patch consumer can request the same general geometric view
    /// produced by ordinary operations. Dense values remain authoritative.
    pub fn rectangle_coordinates(
        &self,
        y: &Self,
        cancel: &AtomicBool,
    ) -> Result<Option<PatchCoordinates>> {
        crate::dsp::check_cancel(cancel)?;
        if self.rows() != y.rows() || self.cols() != y.cols() {
            return Ok(None);
        }
        let Some(x) = self
            .coordinates
            .as_ref()
            .and_then(|p| p.corners(self.rows(), self.cols()))
        else {
            return Ok(None);
        };
        let Some(y) = y
            .coordinates
            .as_ref()
            .and_then(|p| p.corners(y.rows(), y.cols()))
        else {
            return Ok(None);
        };
        let corners = std::array::from_fn(|i| [x[i].clone(), y[i].clone()]);
        PatchCoordinates::rectangles(corners, self.cols(), cancel).map(Some)
    }
}

#[cfg(test)]
#[path = "coordinate_proof_tests.rs"]
mod tests;
