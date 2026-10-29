//! Exact immutable coordinate views and explicit geometry resource accounting.
//!
//! Matrix propagation retains bounded facts beside eager values. The migrated
//! SeriesPoints consumer stores validated views with the same geometry ledger.
use super::patch::validate_face;
use crate::{Matrix, Result};
use std::sync::{Arc, atomic::AtomicBool};

pub const MAX_SOURCE_UNITS: usize = super::MAX_POINTS;
pub const MAX_RECTANGLES: usize = 10_000;
pub const MAX_GENERAL_FACES: usize = super::MAX_PATCH_FACES;
pub const MAX_EXPANDED_VERTICES: usize = MAX_SOURCE_UNITS + 3 * MAX_RECTANGLES;
pub const MAX_RETAINED_BYTES: usize = MAX_SOURCE_UNITS * 16;
const CHUNK: usize = 256;
mod script;
mod wire;
pub(crate) use script::{CoordinateScript, ScriptCoordinate};
pub(crate) use wire::CoordinateDescriptor;

/// Order is significant. No fused operation or algebraic simplification is used.
#[derive(Clone, Copy, Debug)]
pub enum ScalarStep {
    Add(f64),
    Subtract(f64),
    Multiply(f64),
    LeftAdd(f64),
    LeftSubtract(f64),
    LeftMultiply(f64),
}
impl ScalarStep {
    pub(crate) fn operand(self) -> f64 {
        match self {
            Self::Add(v)
            | Self::Subtract(v)
            | Self::Multiply(v)
            | Self::LeftAdd(v)
            | Self::LeftSubtract(v)
            | Self::LeftMultiply(v) => v,
        }
    }
    pub(crate) fn apply(self, value: f64) -> f64 {
        match self {
            Self::Add(v) => value + v,
            Self::Subtract(v) => value - v,
            Self::Multiply(v) => value * v,
            Self::LeftAdd(v) => v + value,
            Self::LeftSubtract(v) => v - value,
            Self::LeftMultiply(v) => v * value,
        }
    }
    fn same_bits(self, other: Self) -> bool {
        match (self, other) {
            (Self::Add(a), Self::Add(b))
            | (Self::Subtract(a), Self::Subtract(b))
            | (Self::Multiply(a), Self::Multiply(b))
            | (Self::LeftAdd(a), Self::LeftAdd(b))
            | (Self::LeftSubtract(a), Self::LeftSubtract(b))
            | (Self::LeftMultiply(a), Self::LeftMultiply(b)) => a.to_bits() == b.to_bits(),
            _ => false,
        }
    }
}

#[derive(Clone, Debug)]
enum Backing {
    ColumnMajor(Arc<nalgebra::DMatrix<f64>>),
    Pairs(Arc<Vec<[f64; 2]>>),
}
impl Backing {
    fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::ColumnMajor(a), Self::ColumnMajor(b)) => Arc::ptr_eq(a, b),
            (Self::Pairs(a), Self::Pairs(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
    fn bytes(&self) -> Result<usize> {
        let (capacity, width) = match self {
            Self::ColumnMajor(m) => (m.data.as_vec().capacity(), 8),
            Self::Pairs(p) => (p.capacity(), 16),
        };
        capacity
            .checked_mul(width)
            .ok_or_else(|| "Coordinate backing capacity overflow".into())
    }
    fn value(&self, offset: usize) -> f64 {
        match self {
            Self::ColumnMajor(m) => m.as_slice()[offset],
            Self::Pairs(p) => p[offset / 2][offset % 2],
        }
    }
}

/// A complete column/lane view retains and charges the entire backing allocation.
#[derive(Clone, Debug)]
pub struct CoordinateLane {
    backing: Backing,
    offset: usize,
    stride: usize,
    len: usize,
    steps: [Option<ScalarStep>; 2],
}
impl CoordinateLane {
    pub(crate) fn matrix_linear(matrix: &Matrix, offset: usize, len: usize) -> Result<Self> {
        Self::matrix_strided(matrix, offset, 1, len)
    }
    pub(crate) fn matrix_strided(
        matrix: &Matrix,
        offset: usize,
        stride: usize,
        len: usize,
    ) -> Result<Self> {
        if !matches!(stride, 1 | 2) {
            return Err("Unsupported coordinate stride".into());
        }
        let end = if len == 0 {
            Some(offset)
        } else {
            (len - 1)
                .checked_mul(stride)
                .and_then(|n| offset.checked_add(n))
                .and_then(|n| n.checked_add(1))
        };
        if end.is_none_or(|end| end > matrix.len()) {
            return Err("Coordinate interval is outside the matrix".into());
        }
        let lane = Self {
            backing: Backing::ColumnMajor(matrix.storage.clone()),
            offset,
            stride,
            len,
            steps: [None; 2],
        };
        lane.check_backing()?;
        Ok(lane)
    }
    pub(crate) fn strided_slice(mut self, start: usize, step: usize, len: usize) -> Result<Self> {
        if !matches!(step, 1 | 2) {
            return Err("Unsupported coordinate stride".into());
        }
        let end = if len == 0 {
            start
        } else {
            start
                .checked_add(
                    (len - 1)
                        .checked_mul(step)
                        .ok_or("Coordinate extent overflow")?,
                )
                .and_then(|n| n.checked_add(1))
                .ok_or("Coordinate extent overflow")?
        };
        if end > self.len {
            return Err("Coordinate interval is outside the lane".into());
        }
        self.offset = self
            .offset
            .checked_add(
                start
                    .checked_mul(self.stride)
                    .ok_or("Coordinate offset overflow")?,
            )
            .ok_or("Coordinate offset overflow")?;
        self.stride = self
            .stride
            .checked_mul(step)
            .filter(|s| matches!(*s, 1 | 2))
            .ok_or("Unsupported coordinate stride")?;
        self.len = len;
        Ok(self)
    }
    pub(crate) fn matrix_backing(&self) -> Option<(usize, usize)> {
        match &self.backing {
            Backing::ColumnMajor(storage) => {
                Some((Arc::as_ptr(storage) as usize, self.backing.bytes().ok()?))
            }
            Backing::Pairs(_) => None,
        }
    }
    fn derived(&self) -> bool {
        self.steps.iter().any(Option::is_some)
    }

    pub fn matrix_column(matrix: &Matrix, column: usize) -> Result<Self> {
        if column >= matrix.cols() {
            return Err("Coordinate column is outside the matrix".into());
        }
        let lane = Self {
            backing: Backing::ColumnMajor(matrix.storage.clone()),
            offset: column
                .checked_mul(matrix.rows())
                .ok_or("Coordinate column overflow")?,
            stride: 1,
            len: matrix.rows(),
            steps: [None; 2],
        };
        lane.check_backing()?;
        Ok(lane)
    }
    pub fn pair_component(points: &Arc<Vec<[f64; 2]>>, component: usize) -> Result<Self> {
        if component > 1 {
            return Err("Coordinate component must be zero or one".into());
        }
        let lane = Self {
            backing: Backing::Pairs(points.clone()),
            offset: component,
            stride: 2,
            len: points.len(),
            steps: [None; 2],
        };
        lane.check_backing()?;
        Ok(lane)
    }
    fn check_backing(&self) -> Result<()> {
        if self.len > MAX_SOURCE_UNITS || self.backing.bytes()? > MAX_RETAINED_BYTES {
            return Err("Coordinate backing exceeds the retained payload limit".into());
        }
        Ok(())
    }
    /// Consumes metadata only. A third operation must use the ordinary dense path.
    pub fn then(mut self, step: ScalarStep) -> Result<Self> {
        let slot = self
            .steps
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or("Coordinate expressions support at most two ordered steps")?;
        *slot = Some(step);
        Ok(self)
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn value(&self, index: usize) -> Option<f64> {
        if index >= self.len {
            return None;
        }
        let mut value = self.backing.value(self.offset + index * self.stride);
        for step in self.steps.iter().flatten() {
            value = step.apply(value);
        }
        Some(value)
    }
    fn same_source(&self, other: &Self) -> bool {
        self.backing.same(&other.backing)
            && self.offset == other.offset
            && self.stride == other.stride
            && self.len == other.len
    }
    pub fn same_snapshot(&self, other: &Self) -> bool {
        self.same_source(other)
            && self
                .steps
                .iter()
                .zip(other.steps)
                .all(|(a, b)| match (a, b) {
                    (None, None) => true,
                    (Some(a), Some(b)) => a.same_bits(b),
                    _ => false,
                })
    }
}

#[derive(Clone, Debug)]
pub enum Coordinate {
    Constant(f64),
    Lane(CoordinateLane),
}
impl Coordinate {
    fn value(&self, index: usize) -> f64 {
        match self {
            Self::Constant(value) => *value,
            Self::Lane(lane) => lane.value(index).expect("validated coordinate length"),
        }
    }
    fn matches_eager(&self, actual: f64, index: usize) -> bool {
        let expected = self.value(index);
        actual.to_bits() == expected.to_bits()
            || (actual.is_nan()
                && expected.is_nan()
                && matches!(self, Self::Lane(lane) if lane.derived()))
    }
    fn same_snapshot(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Constant(a), Self::Constant(b)) => a.to_bits() == b.to_bits(),
            (Self::Lane(a), Self::Lane(b)) => a.same_snapshot(b),
            _ => false,
        }
    }
}

#[derive(Clone, Debug)]
enum Representation {
    Dense {
        points: Arc<Vec<[f64; 2]>>,
        width: usize,
    },
    Rectangles {
        corners: Box<[[Coordinate; 2]; 4]>,
        count: usize,
    },
}

/// Real face count and corner access are independent of retained storage size.
#[derive(Clone, Debug)]
pub struct PatchCoordinates {
    representation: Representation,
    cost: GeometryCost,
}

impl PatchCoordinates {
    pub fn dense(points: Arc<Vec<[f64; 2]>>, width: usize, cancel: &AtomicBool) -> Result<Self> {
        Self::dense_checked(points, width, || crate::dsp::check_cancel(cancel))
    }
    fn dense_checked(
        points: Arc<Vec<[f64; 2]>>,
        width: usize,
        mut check: impl FnMut() -> Result<()>,
    ) -> Result<Self> {
        check()?;
        if !matches!(width, 3 | 4) || !points.len().is_multiple_of(width) {
            return Err("Patch needs complete three- or four-corner faces".into());
        }
        let faces = points.len() / width;
        if faces > MAX_GENERAL_FACES {
            return Err("Patch exceeds the general-face limit".into());
        }
        let cost = GeometryCost::from_counts(
            points.capacity(),
            points.len(),
            0,
            faces,
            points
                .capacity()
                .checked_mul(16)
                .ok_or("Patch capacity overflow")?,
        )?;
        for (index, face) in points.chunks_exact(width).enumerate() {
            if index.is_multiple_of(CHUNK) {
                check()?;
            }
            validate_face(face)?;
        }
        check()?;
        Ok(Self {
            representation: Representation::Dense { points, width },
            cost,
        })
    }
    pub fn rectangles(
        corners: [[Coordinate; 2]; 4],
        count: usize,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        Self::rectangles_checked(corners, count, || crate::dsp::check_cancel(cancel))
    }
    fn rectangles_checked(
        corners: [[Coordinate; 2]; 4],
        count: usize,
        mut check: impl FnMut() -> Result<()>,
    ) -> Result<Self> {
        check()?;
        if count > MAX_RECTANGLES {
            return Err("Patch exceeds the compact-rectangle limit".into());
        }
        // Eight coordinates bound this stack registry. No growing DAG or hidden nodes.
        let mut lanes: [Option<&CoordinateLane>; 2] = [None; 2];
        let mut backings: [Option<&Backing>; 2] = [None; 2];
        let mut bytes = 0usize;
        for coordinate in corners.iter().flatten() {
            if let Coordinate::Lane(lane) = coordinate {
                if lane.len != count {
                    return Err("Coordinate lane length differs from face count".into());
                }
                if !lanes.iter().flatten().any(|old| lane.same_source(old)) {
                    let slot = lanes
                        .iter_mut()
                        .find(|slot| slot.is_none())
                        .ok_or("Compact rectangles support at most two independent source lanes")?;
                    *slot = Some(lane);
                }
                if !backings.iter().flatten().any(|old| lane.backing.same(old)) {
                    bytes = bytes
                        .checked_add(lane.backing.bytes()?)
                        .ok_or("Coordinate bytes overflow")?;
                    let slot = backings
                        .iter_mut()
                        .find(|slot| slot.is_none())
                        .ok_or("Too many coordinate backings")?;
                    *slot = Some(&lane.backing);
                }
            }
        }
        // Capacity/unused backing is charged even if the visible lane is tiny.
        let units = count.max(bytes.div_ceil(16));
        let cost = GeometryCost::from_counts(units, count * 4, count, 0, bytes)?;
        for index in 0..count {
            if index.is_multiple_of(CHUNK) {
                check()?;
            }
            let face = std::array::from_fn::<_, 4, _>(|corner| {
                std::array::from_fn(|axis| corners[corner][axis].value(index))
            });
            validate_face(&face)?;
        }
        check()?;
        Ok(Self {
            representation: Representation::Rectangles {
                corners: Box::new(corners),
                count,
            },
            cost,
        })
    }
    pub fn face_count(&self) -> usize {
        match &self.representation {
            Representation::Dense { points, width } => points.len() / width,
            Representation::Rectangles { count, .. } => *count,
        }
    }
    pub fn corners_per_face(&self) -> usize {
        match &self.representation {
            Representation::Dense { width, .. } => *width,
            _ => 4,
        }
    }
    pub fn corner(&self, face: usize, corner: usize) -> Option<[f64; 2]> {
        if face >= self.face_count() || corner >= self.corners_per_face() {
            return None;
        }
        Some(match &self.representation {
            Representation::Dense { points, width } => points[face * width + corner],
            Representation::Rectangles { corners, .. } => {
                std::array::from_fn(|axis| corners[corner][axis].value(face))
            }
        })
    }
    /// Verify a descriptor against an ordinary eager input without keeping or
    /// cloning either Matrix. Derived NaN equivalence retains the existing rule.
    pub(crate) fn matches_matrices(
        &self,
        x: &Matrix,
        y: &Matrix,
        cancel: &AtomicBool,
    ) -> Result<bool> {
        crate::dsp::check_cancel(cancel)?;
        if x.rows() != self.corners_per_face()
            || y.rows() != x.rows()
            || x.cols() != self.face_count()
            || y.cols() != x.cols()
        {
            return Ok(false);
        }
        for face in 0..self.face_count() {
            if face.is_multiple_of(CHUNK) {
                crate::dsp::check_cancel(cancel)?;
            }
            for corner in 0..self.corners_per_face() {
                let matched = match &self.representation {
                    Representation::Rectangles { corners, .. } => {
                        corners[corner][0].matches_eager(x.get(corner, face), face)
                            && corners[corner][1].matches_eager(y.get(corner, face), face)
                    }
                    Representation::Dense { points, width } => {
                        let actual = [x.get(corner, face), y.get(corner, face)];
                        points[face * width + corner]
                            .iter()
                            .zip(actual)
                            .all(|(a, b)| a.to_bits() == b.to_bits())
                    }
                };
                if !matched {
                    return Ok(false);
                }
            }
        }
        crate::dsp::check_cancel(cancel)?;
        Ok(true)
    }
    pub fn cost(&self) -> GeometryCost {
        self.cost
    }
    /// Bounded metadata/Arc identity comparison; never scans coordinate payload.
    pub fn same_snapshot(&self, other: &Self) -> bool {
        match (&self.representation, &other.representation) {
            (
                Representation::Dense {
                    points: a,
                    width: wa,
                },
                Representation::Dense {
                    points: b,
                    width: wb,
                },
            ) => wa == wb && Arc::ptr_eq(a, b),
            (
                Representation::Rectangles {
                    corners: a,
                    count: na,
                },
                Representation::Rectangles {
                    corners: b,
                    count: nb,
                },
            ) => {
                na == nb
                    && a.iter()
                        .flatten()
                        .zip(b.iter().flatten())
                        .all(|(a, b)| a.same_snapshot(b))
            }
            _ => false,
        }
    }
    /// Fixed descriptor allocation, separate from scalar/pair backing payload.
    pub fn descriptor_bytes(&self) -> usize {
        match &self.representation {
            Representation::Rectangles { .. } => std::mem::size_of::<[[Coordinate; 2]; 4]>(),
            Representation::Dense { .. } => 0,
        }
    }
    /// Materialize for an explicitly bounded consumer; never done by inspection.
    pub fn materialize(&self, cancel: &AtomicBool) -> Result<Vec<[f64; 2]>> {
        self.materialize_checked(|| crate::dsp::check_cancel(cancel))
    }
    fn materialize_checked(&self, mut check: impl FnMut() -> Result<()>) -> Result<Vec<[f64; 2]>> {
        check()?;
        let count = self.face_count() * self.corners_per_face();
        let mut result = Vec::new();
        result
            .try_reserve_exact(count)
            .map_err(|_| "Cannot allocate expanded patch coordinates")?;
        for face in 0..self.face_count() {
            if face.is_multiple_of(CHUNK) {
                check()?;
            }
            for corner in 0..self.corners_per_face() {
                result.push(self.corner(face, corner).unwrap());
            }
        }
        check()?;
        Ok(result)
    }
    pub(crate) fn owned_descriptor_bytes(&self) -> usize {
        match self.representation {
            Representation::Rectangles { .. } => std::mem::size_of::<[[Coordinate; 2]; 4]>(),
            Representation::Dense { .. } => 0,
        }
    }
    pub(crate) fn visit_retained_backings(&self, mut visit: impl FnMut(usize, usize)) {
        let counters = 2 * std::mem::size_of::<usize>();
        for backing in self.backings() {
            let bytes = backing.bytes().expect("validated retained capacity");
            match backing {
                Backing::ColumnMajor(value) => visit(
                    Arc::as_ptr(&value) as usize,
                    bytes + counters + std::mem::size_of::<nalgebra::DMatrix<f64>>(),
                ),
                Backing::Pairs(value) => visit(
                    Arc::as_ptr(&value) as usize,
                    bytes + counters + std::mem::size_of::<Vec<[f64; 2]>>(),
                ),
            }
        }
    }
    fn backings(&self) -> Vec<Backing> {
        // At most eight descriptor slots, at most two distinct shared backings.
        let mut result = Vec::new();
        match &self.representation {
            Representation::Dense { points, .. } => result.push(Backing::Pairs(points.clone())),
            Representation::Rectangles { corners, .. } => {
                for coordinate in corners.iter().flatten() {
                    if let Coordinate::Lane(lane) = coordinate
                        && !result.iter().any(|old| lane.backing.same(old))
                    {
                        result.push(lane.backing.clone());
                    }
                }
            }
        }
        result
    }
}

/// Counts are private: costs originate from checked geometry/backing constructors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GeometryCost {
    source_units: usize,
    vertices: usize,
    rectangles: usize,
    general_faces: usize,
    retained_bytes: usize,
}
impl GeometryCost {
    fn from_counts(
        source_units: usize,
        vertices: usize,
        rectangles: usize,
        general_faces: usize,
        retained_bytes: usize,
    ) -> Result<Self> {
        let cost = Self {
            source_units,
            vertices,
            rectangles,
            general_faces,
            retained_bytes,
        };
        if general_faces > MAX_GENERAL_FACES {
            return Err("Patch face budget exceeds 10,000 faces".into());
        }
        if rectangles > MAX_RECTANGLES {
            return Err("Compact rectangle face budget exceeds 10,000 faces".into());
        }
        if source_units > MAX_SOURCE_UNITS
            || retained_bytes > MAX_RETAINED_BYTES
            || retained_bytes > source_units * 16
            || vertices < rectangles * 4 + general_faces * 3
            || vertices > source_units + 3 * rectangles
        {
            return Err("Geometry exceeds the retained/expanded resource budget limits".into());
        }
        Ok(cost)
    }
    pub fn ordinary(points: &Arc<Vec<[f64; 2]>>) -> Result<Self> {
        Self::from_counts(
            points.capacity(),
            points.len(),
            0,
            0,
            points
                .capacity()
                .checked_mul(16)
                .ok_or("Point capacity overflow")?,
        )
    }
    pub(crate) fn ordinary_allocation(count: usize, bars: bool) -> Result<Self> {
        Self::from_counts(
            count,
            if bars {
                count.checked_mul(4).ok_or("Bar geometry overflow")?
            } else {
                count
            },
            if bars { count } else { 0 },
            0,
            count.checked_mul(16).ok_or("Point capacity overflow")?,
        )
    }
    /// Preflight for an exact-size dense face allocation before pair copying.
    pub(crate) fn dense_face_allocation(vertices: usize, width: usize) -> Result<Self> {
        if !matches!(width, 3 | 4) || !vertices.is_multiple_of(width) {
            return Err("Patch needs complete three- or four-corner faces".into());
        }
        Self::from_counts(
            vertices,
            vertices,
            0,
            vertices / width,
            vertices.checked_mul(16).ok_or("Point capacity overflow")?,
        )
    }
    /// Dense faces use the general bank; no compact provenance is inferred.
    pub fn dense_faces(points: &Arc<Vec<[f64; 2]>>, width: usize) -> Result<Self> {
        if !matches!(width, 3 | 4) || !points.len().is_multiple_of(width) {
            return Err("Patch needs complete three- or four-corner faces".into());
        }
        Self::from_counts(
            points.capacity(),
            points.len(),
            0,
            points.len() / width,
            points
                .capacity()
                .checked_mul(16)
                .ok_or("Point capacity overflow")?,
        )
    }
    /// Ordinary Bar samples are one retained pair and four expanded corners.
    pub fn bars(points: &Arc<Vec<[f64; 2]>>) -> Result<Self> {
        Self::from_counts(
            points.capacity(),
            points.len().checked_mul(4).ok_or("Bar geometry overflow")?,
            points.len(),
            0,
            points
                .capacity()
                .checked_mul(16)
                .ok_or("Point capacity overflow")?,
        )
    }
    pub fn source_units(self) -> usize {
        self.source_units
    }
    pub fn expanded_vertices(self) -> usize {
        self.vertices
    }
    pub fn compact_rectangles(self) -> usize {
        self.rectangles
    }
    pub fn general_faces(self) -> usize {
        self.general_faces
    }
    pub fn retained_bytes(self) -> usize {
        self.retained_bytes
    }
}

/// Logical per-series charges are added even when two objects share a backing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GeometryLedger {
    cost: GeometryCost,
    series: usize,
}
impl GeometryLedger {
    pub fn cost(self) -> GeometryCost {
        self.cost
    }
    pub fn series(self) -> usize {
        self.series
    }
    pub(crate) fn merge(&mut self, other: Self) -> Result<()> {
        let series = self
            .series
            .checked_add(other.series)
            .ok_or("Series count overflow")?;
        if series > super::MAX_SERIES {
            return Err("Figure exceeds 64 series".into());
        }
        let add = |a: usize, b: usize| {
            a.checked_add(b)
                .ok_or_else(|| "Geometry accounting overflow".to_owned())
        };
        let cost = GeometryCost::from_counts(
            add(self.cost.source_units, other.cost.source_units)?,
            add(self.cost.vertices, other.cost.vertices)?,
            add(self.cost.rectangles, other.cost.rectangles)?,
            add(self.cost.general_faces, other.cost.general_faces)?,
            add(self.cost.retained_bytes, other.cost.retained_bytes)?,
        )?;
        *self = Self { series, cost };
        Ok(())
    }

    /// Failure leaves the existing ledger unchanged; allocation policy is external.
    pub fn add(&mut self, cost: GeometryCost) -> Result<()> {
        let mut candidate = *self;
        candidate.series = candidate
            .series
            .checked_add(1)
            .ok_or("Series count overflow")?;
        if candidate.series > super::MAX_SERIES {
            return Err("Figure exceeds 64 series".into());
        }
        let sum = |a: usize, b: usize| {
            a.checked_add(b)
                .ok_or_else(|| "Geometry accounting overflow".to_owned())
        };
        candidate.cost = GeometryCost::from_counts(
            sum(self.cost.source_units, cost.source_units)?,
            sum(self.cost.vertices, cost.vertices)?,
            sum(self.cost.rectangles, cost.rectangles)?,
            sum(self.cost.general_faces, cost.general_faces)?,
            sum(self.cost.retained_bytes, cost.retained_bytes)?,
        )?;
        *self = candidate;
        Ok(())
    }
}

/// Eager proof envelope; Matrix facts and their hidden retention are charged separately.
/// Admission must charge `workspace_bytes`, including every retained proof backing.
#[derive(Clone, Debug)]
pub struct VerifiedPatchInputs {
    x: Matrix,
    y: Matrix,
    proof: PatchCoordinates,
    workspace_bytes: usize,
}
impl VerifiedPatchInputs {
    pub fn new(
        x: Matrix,
        y: Matrix,
        proof: PatchCoordinates,
        remaining_workspace_bytes: usize,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        Self::new_checked(x, y, proof, remaining_workspace_bytes, || {
            crate::dsp::check_cancel(cancel)
        })
    }
    fn new_checked(
        x: Matrix,
        y: Matrix,
        proof: PatchCoordinates,
        remaining_workspace_bytes: usize,
        mut check: impl FnMut() -> Result<()>,
    ) -> Result<Self> {
        check()?;
        if x.rows() != proof.corners_per_face()
            || y.rows() != x.rows()
            || x.cols() != proof.face_count()
            || y.cols() != x.cols()
        {
            return Err("Proof does not match Patch XData/YData shape".into());
        }
        let mut backings = vec![Backing::ColumnMajor(x.storage.clone())];
        let y_backing = Backing::ColumnMajor(y.storage.clone());
        // Charge both visible matrix bindings even if they alias. Only hidden
        // proof references are deduplicated against already charged inputs.
        let mut bytes = x
            .retained_bytes()
            .max(backings[0].bytes()?)
            .checked_add(y.retained_bytes().max(y_backing.bytes()?))
            .ok_or("Proof workspace bytes overflow")?;
        if !backings[0].same(&y_backing) {
            backings.push(y_backing);
        }
        for backing in proof.backings() {
            if !backings.iter().any(|old| backing.same(old)) {
                bytes = bytes
                    .checked_add(backing.bytes()?)
                    .ok_or("Proof workspace bytes overflow")?;
                backings.push(backing);
            }
        }
        bytes = bytes
            .checked_add(proof.descriptor_bytes())
            .ok_or("Proof metadata bytes overflow")?;
        if bytes > remaining_workspace_bytes.min(256 * 1024 * 1024) {
            return Err("Coordinate proof exceeds remaining workspace bytes".into());
        }
        for face in 0..proof.face_count() {
            if face.is_multiple_of(CHUNK) {
                check()?;
            }
            for corner in 0..proof.corners_per_face() {
                let expected = proof.corner(face, corner).unwrap();
                let matches = match &proof.representation {
                    Representation::Rectangles { corners, .. } => {
                        corners[corner][0].matches_eager(x.get(corner, face), face)
                            && corners[corner][1].matches_eager(y.get(corner, face), face)
                    }
                    Representation::Dense { .. } => {
                        x.get(corner, face).to_bits() == expected[0].to_bits()
                            && y.get(corner, face).to_bits() == expected[1].to_bits()
                    }
                };
                if !matches {
                    return Err("Coordinate proof does not match exact eager values".into());
                }
            }
        }
        check()?;
        Ok(Self {
            x,
            y,
            proof,
            workspace_bytes: bytes,
        })
    }
    pub fn workspace_bytes(&self) -> usize {
        self.workspace_bytes
    }
    pub fn x(&self) -> &Matrix {
        &self.x
    }
    pub fn y(&self) -> &Matrix {
        &self.y
    }
    pub fn proof(&self) -> &PatchCoordinates {
        &self.proof
    }
    /// Unsupported operations take ordinary inputs; retained proof is dropped.
    pub fn into_dense(self) -> (Matrix, Matrix) {
        (
            self.x.without_coordinate_proof(),
            self.y.without_coordinate_proof(),
        )
    }
}

#[cfg(test)]
#[path = "coordinate_tests.rs"]
mod tests;
