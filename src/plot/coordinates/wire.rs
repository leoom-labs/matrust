//! Fixed-size descriptor metadata; raw backing payloads retain IEEE bits.
//! Native N/O envelope integration remains in the parent migration layer.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum SourceKind {
    Scalars,
    Pairs,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    kind: SourceKind,
    len: u32,
    capacity: u32,
}
impl Source {
    fn width(self) -> usize {
        if self.kind == SourceKind::Pairs {
            16
        } else {
            8
        }
    }
    fn scalar_len(self) -> usize {
        self.len as usize * (self.width() / 8)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
enum Operation {
    Add,
    Subtract,
    Multiply,
    LeftAdd,
    LeftSubtract,
    LeftMultiply,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    operation: Operation,
    bits: u64,
}
impl Step {
    fn from_step(step: ScalarStep) -> Self {
        let (operation, value) = match step {
            ScalarStep::Add(v) => (Operation::Add, v),
            ScalarStep::Subtract(v) => (Operation::Subtract, v),
            ScalarStep::Multiply(v) => (Operation::Multiply, v),
            ScalarStep::LeftAdd(v) => (Operation::LeftAdd, v),
            ScalarStep::LeftSubtract(v) => (Operation::LeftSubtract, v),
            ScalarStep::LeftMultiply(v) => (Operation::LeftMultiply, v),
        };
        Self {
            operation,
            bits: value.to_bits(),
        }
    }
    fn into_step(self) -> ScalarStep {
        let value = f64::from_bits(self.bits);
        match self.operation {
            Operation::Add => ScalarStep::Add(value),
            Operation::Subtract => ScalarStep::Subtract(value),
            Operation::Multiply => ScalarStep::Multiply(value),
            Operation::LeftAdd => ScalarStep::LeftAdd(value),
            Operation::LeftSubtract => ScalarStep::LeftSubtract(value),
            Operation::LeftMultiply => ScalarStep::LeftMultiply(value),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum Corner {
    Constant {
        bits: u64,
    },
    Lane {
        source: u8,
        offset: u32,
        stride: u8,
        steps: [Option<Step>; 2],
    },
}

/// Arrays bound metadata parsing independently of a claimed count. The outer
/// native reader must also bound its complete metadata buffer before serde.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CoordinateDescriptor {
    count: u32,
    sources: [Option<Source>; 2],
    corners: [[Corner; 2]; 4],
}

impl PatchCoordinates {
    pub(crate) fn native_descriptor(&self) -> Option<CoordinateDescriptor> {
        let Representation::Rectangles { corners, count } = &self.representation else {
            return None;
        };
        let backings = self.backings();
        let sources = std::array::from_fn(|index| {
            backings.get(index).map(|source| match source {
                Backing::ColumnMajor(values) => Source {
                    kind: SourceKind::Scalars,
                    len: values.len() as u32,
                    capacity: values.data.as_vec().capacity() as u32,
                },
                Backing::Pairs(values) => Source {
                    kind: SourceKind::Pairs,
                    len: values.len() as u32,
                    capacity: values.capacity() as u32,
                },
            })
        });
        let corners = std::array::from_fn(|corner| {
            std::array::from_fn(|axis| match &corners[corner][axis] {
                Coordinate::Constant(value) => Corner::Constant {
                    bits: value.to_bits(),
                },
                Coordinate::Lane(lane) => Corner::Lane {
                    source: backings
                        .iter()
                        .position(|source| source.same(&lane.backing))
                        .expect("retained source") as u8,
                    offset: lane.offset as u32,
                    stride: lane.stride as u8,
                    steps: lane.steps.map(|step| step.map(Step::from_step)),
                },
            })
        });
        // All narrowing follows the safe constructors' <=2M-scalar, <=10K-face,
        // two-source/stride1-or2 limits. Decode rechecks rather than trusting it.
        Some(CoordinateDescriptor {
            count: *count as u32,
            sources,
            corners,
        })
    }
    pub(crate) fn write_native_payload(
        &self,
        out: &mut Vec<u8>,
        cancel: &AtomicBool,
    ) -> Result<()> {
        crate::dsp::check_cancel(cancel)?;
        if !matches!(self.representation, Representation::Rectangles { .. }) {
            return Err("Native descriptor requires compact coordinates".into());
        }
        for source in self.backings() {
            let scalars = match &source {
                Backing::ColumnMajor(a) => a.len(),
                Backing::Pairs(a) => a.len() * 2,
            };
            for index in 0..scalars {
                if index.is_multiple_of(1024) {
                    crate::dsp::check_cancel(cancel)?;
                }
                out.extend_from_slice(&source.value(index).to_bits().to_le_bytes());
            }
        }
        crate::dsp::check_cancel(cancel)
    }
}

impl CoordinateDescriptor {
    pub(crate) fn vertex_count(&self) -> usize {
        self.count as usize * 4
    }
    /// All resource/index checks happen before any backing allocation.
    pub(crate) fn preflight(&self) -> Result<(GeometryCost, usize)> {
        let count = self.count as usize;
        if count > MAX_RECTANGLES {
            return Err("Native compact rectangle limit".into());
        }
        if self.sources[0].is_none() && self.sources[1].is_some() {
            return Err("Native coordinate source table has a gap".into());
        }
        let mut retained = 0usize;
        let mut payload = 0usize;
        for source in self.sources.iter().flatten() {
            if source.capacity < source.len {
                return Err("Native coordinate capacity is shorter than data".into());
            }
            retained = retained
                .checked_add(
                    (source.capacity as usize)
                        .checked_mul(source.width())
                        .ok_or("Native coordinate capacity overflow")?,
                )
                .ok_or("Native coordinate capacity overflow")?;
            payload = payload
                .checked_add(
                    (source.len as usize)
                        .checked_mul(source.width())
                        .ok_or("Native coordinate payload overflow")?,
                )
                .ok_or("Native coordinate payload overflow")?;
        }
        let cost = GeometryCost::from_counts(
            count.max(retained.div_ceil(16)),
            count * 4,
            count,
            0,
            retained,
        )?;
        let mut lanes: [Option<(u8, u32, u8)>; 2] = [None; 2];
        let mut used = [false; 2];
        for corner in self.corners.iter().flatten() {
            if let Corner::Lane {
                source,
                offset,
                stride,
                steps,
            } = corner
            {
                let index = usize::from(*source);
                let source_info = self
                    .sources
                    .get(index)
                    .and_then(|s| *s)
                    .ok_or("Unknown native coordinate source")?;
                if !matches!(*stride, 1 | 2) || (steps[0].is_none() && steps[1].is_some()) {
                    return Err("Invalid native coordinate stride/step sequence".into());
                }
                let end = if count == 0 {
                    *offset as usize
                } else {
                    (*offset as usize)
                        .checked_add(
                            (count - 1)
                                .checked_mul(usize::from(*stride))
                                .ok_or("Native coordinate extent overflow")?,
                        )
                        .and_then(|n| n.checked_add(1))
                        .ok_or("Native coordinate extent overflow")?
                };
                if end > source_info.scalar_len() {
                    return Err("Native coordinate lane exceeds its source".into());
                }
                used[index] = true;
                let key = (*source, *offset, *stride);
                if !lanes.iter().flatten().any(|old| *old == key) {
                    *lanes
                        .iter_mut()
                        .find(|entry| entry.is_none())
                        .ok_or("Native rectangle has more than two lanes")? = Some(key);
                }
            }
        }
        for (index, source) in self.sources.iter().enumerate() {
            if source.is_some() != used[index] {
                return Err("Unused native coordinate source".into());
            }
        }
        Ok((cost, payload))
    }
    pub(crate) fn decode(&self, bytes: &[u8], cancel: &AtomicBool) -> Result<PatchCoordinates> {
        crate::dsp::check_cancel(cancel)?;
        let (_, payload) = self.preflight()?;
        if payload != bytes.len() {
            return Err("Native coordinate payload is truncated or trailing".into());
        }
        let mut cursor = 0usize;
        let mut sources: [Option<Backing>; 2] = [None, None];
        for (target, info) in sources.iter_mut().zip(self.sources) {
            let Some(info) = info else {
                continue;
            };
            let mut scalar = || {
                let value = f64::from_bits(u64::from_le_bytes(
                    bytes[cursor..cursor + 8]
                        .try_into()
                        .expect("preflight length"),
                ));
                cursor += 8;
                value
            };
            *target = Some(match info.kind {
                SourceKind::Scalars => {
                    let mut values = Vec::new();
                    values
                        .try_reserve_exact(info.capacity as usize)
                        .map_err(|_| "Cannot allocate coordinate source")?;
                    for index in 0..info.len as usize {
                        if index.is_multiple_of(1024) {
                            crate::dsp::check_cancel(cancel)?;
                        }
                        values.push(scalar());
                    }
                    let matrix = Matrix::from_column_vec(info.len as usize, 1, values)?;
                    Backing::ColumnMajor(matrix.storage)
                }
                SourceKind::Pairs => {
                    let mut values = Vec::new();
                    values
                        .try_reserve_exact(info.capacity as usize)
                        .map_err(|_| "Cannot allocate coordinate source")?;
                    for index in 0..info.len as usize {
                        if index.is_multiple_of(1024) {
                            crate::dsp::check_cancel(cancel)?;
                        }
                        values.push([scalar(), scalar()]);
                    }
                    Backing::Pairs(Arc::new(values))
                }
            });
        }
        let corners = self.corners.map(|corner| {
            corner.map(|axis| match axis {
                Corner::Constant { bits } => Coordinate::Constant(f64::from_bits(bits)),
                Corner::Lane {
                    source,
                    offset,
                    stride,
                    steps,
                } => Coordinate::Lane(CoordinateLane {
                    backing: sources[usize::from(source)]
                        .as_ref()
                        .expect("preflight source")
                        .clone(),
                    offset: offset as usize,
                    stride: usize::from(stride),
                    len: self.count as usize,
                    steps: steps.map(|step| step.map(Step::into_step)),
                }),
            })
        });
        PatchCoordinates::rectangles(corners, self.count as usize, cancel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    fn sample(cancel: &AtomicBool) -> PatchCoordinates {
        let mut values = Vec::with_capacity(64);
        values.extend([
            [-0.0, -0.0],
            [1.0e16, f64::from_bits(1)],
            [3.0, f64::from_bits(0x7ff8_1234_0000_0001)],
        ]);
        let values = Arc::new(values);
        let x = CoordinateLane::pair_component(&values, 0).unwrap();
        let y = CoordinateLane::pair_component(&values, 1).unwrap();
        let a = Coordinate::Lane(
            x.clone()
                .then(ScalarStep::Add(0.25))
                .unwrap()
                .then(ScalarStep::Subtract(0.4))
                .unwrap(),
        );
        let b = Coordinate::Lane(
            x.then(ScalarStep::Add(0.25))
                .unwrap()
                .then(ScalarStep::Add(0.4))
                .unwrap(),
        );
        let c = Coordinate::Lane(y.clone().then(ScalarStep::Multiply(0.0)).unwrap());
        let d = Coordinate::Lane(y);
        PatchCoordinates::rectangles(
            [
                [a.clone(), c.clone()],
                [b.clone(), c],
                [b, d.clone()],
                [a, d],
            ],
            3,
            cancel,
        )
        .unwrap()
    }
    #[test]
    fn descriptor_roundtrip_preserves_capacity_source_bits_and_ordered_operations() {
        let cancel = AtomicBool::new(false);
        let source = sample(&cancel);
        let descriptor = source.native_descriptor().unwrap();
        let json = serde_json::to_vec(&descriptor).unwrap();
        let descriptor: CoordinateDescriptor = serde_json::from_slice(&json).unwrap();
        let (cost, bytes) = descriptor.preflight().unwrap();
        assert_eq!(cost, source.cost());
        assert_eq!(bytes, 3 * 16);
        assert_eq!(cost.retained_bytes(), 64 * 16);
        let mut payload = Vec::new();
        source.write_native_payload(&mut payload, &cancel).unwrap();
        let restored = descriptor.decode(&payload, &cancel).unwrap();
        assert_eq!(restored.cost(), source.cost());
        for face in 0..3 {
            for corner in 0..4 {
                let a = source.corner(face, corner).unwrap();
                let b = restored.corner(face, corner).unwrap();
                for axis in 0..2 {
                    if a[axis].is_nan() && b[axis].is_nan() && corner < 2 {
                        continue;
                    }
                    assert_eq!(a[axis].to_bits(), b[axis].to_bits());
                }
            }
        }
        let mut rewritten = Vec::new();
        restored
            .write_native_payload(&mut rewritten, &cancel)
            .unwrap();
        assert_eq!(rewritten, payload);
        assert_eq!(
            serde_json::to_vec(&restored.native_descriptor().unwrap()).unwrap(),
            json
        );
        cancel.store(true, Ordering::Relaxed);
        assert!(descriptor.decode(&payload, &cancel).is_err());
        cancel.store(false, Ordering::Relaxed);
        assert_eq!(descriptor.decode(&payload, &cancel).unwrap().cost(), cost);
    }
    #[test]
    fn malformed_descriptor_preflight_rejects_extents_steps_sources_and_resource_claims() {
        let cancel = AtomicBool::new(false);
        let source = sample(&cancel);
        let good = source.native_descriptor().unwrap();
        let mut invalid = good.clone();
        invalid.sources[0].as_mut().unwrap().capacity = u32::MAX;
        assert!(invalid.preflight().is_err());
        let mut invalid = good.clone();
        invalid.count = 10_001;
        assert!(invalid.preflight().is_err());
        let mut invalid = good.clone();
        if let Corner::Lane { offset, .. } = &mut invalid.corners[0][0] {
            *offset = u32::MAX;
        }
        assert!(invalid.preflight().is_err());
        let mut invalid = good.clone();
        if let Corner::Lane { steps, .. } = &mut invalid.corners[0][0] {
            steps[0] = None;
        }
        assert!(invalid.preflight().is_err());
        let mut invalid = good.clone();
        invalid.sources[1] = invalid.sources[0];
        assert!(invalid.preflight().is_err());
        let mut invalid = good.clone();
        if let Corner::Lane { source, .. } = &mut invalid.corners[0][0] {
            *source = 2;
        }
        assert!(invalid.preflight().is_err());
        assert!(good.decode(&[], &cancel).is_err());
        assert!(good.decode(&[0; 49], &cancel).is_err());
    }
}
