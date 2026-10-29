//! Ordinary script reconstruction exposes bounded source views, never an AST
//! recognizer. Every sample bit comes from the retained backing, before steps.
use super::*;

pub(crate) struct CoordinateSource(Backing);
impl CoordinateSource {
    pub(crate) fn len(&self) -> usize {
        match &self.0 {
            Backing::ColumnMajor(a) => a.len(),
            Backing::Pairs(a) => a.len() * 2,
        }
    }
    pub(crate) fn value(&self, index: usize) -> f64 {
        self.0.value(index)
    }
    pub(crate) fn matrix(&self, cancel: &AtomicBool) -> Result<Matrix> {
        crate::dsp::check_cancel(cancel)?;
        match &self.0 {
            Backing::ColumnMajor(a) => Matrix::from_shared_coordinate_storage(a.clone()),
            Backing::Pairs(_) => {
                // MAT has no interleaved pair type. One bounded copy stores the
                // original scalar order; no corners or arithmetic are expanded.
                let mut values = Vec::new();
                values
                    .try_reserve_exact(self.len())
                    .map_err(|_| "Cannot allocate coordinate companion")?;
                for index in 0..self.len() {
                    if index.is_multiple_of(1024) {
                        crate::dsp::check_cancel(cancel)?;
                    }
                    values.push(self.value(index));
                }
                crate::dsp::check_cancel(cancel)?;
                Matrix::from_column_vec(values.len(), 1, values)
            }
        }
    }
}
#[derive(Clone, Copy)]
pub(crate) enum ScriptCoordinate {
    Constant(f64),
    Lane {
        source: usize,
        offset: usize,
        stride: usize,
        steps: [Option<ScalarStep>; 2],
    },
}
pub(crate) struct CoordinateScript {
    pub(crate) sources: Vec<CoordinateSource>,
    pub(crate) corners: [[ScriptCoordinate; 2]; 4],
    pub(crate) count: usize,
}
impl CoordinateScript {
    pub(crate) fn has_steps(&self) -> bool {
        self.corners.iter().flatten().any(|coordinate| {
            matches!(coordinate,
            ScriptCoordinate::Lane { steps, .. } if steps.iter().any(Option::is_some))
        })
    }
    /// Four corners by (axis, operation slot), column-major. Padding is unused.
    pub(crate) fn step_operands(&self) -> Vec<f64> {
        (0..4)
            .flat_map(|column| {
                self.corners
                    .iter()
                    .map(move |corner| match corner[column / 2] {
                        ScriptCoordinate::Lane { steps, .. } => {
                            steps[column % 2].map_or(0., ScalarStep::operand)
                        }
                        ScriptCoordinate::Constant(_) => 0.,
                    })
            })
            .collect()
    }
}
impl PatchCoordinates {
    /// Ordinary MATLAB NaN literals do not preserve arbitrary IEEE payloads.
    /// Automatic same-engine reconstruction selects MAT for these descriptors;
    /// the explicit public standalone serializer keeps its historical literals.
    pub(crate) fn needs_exact_companion(&self, cancel: &AtomicBool) -> Result<bool> {
        crate::dsp::check_cancel(cancel)?;
        let Some(script) = self.script_coordinates() else {
            return Ok(false);
        };
        if script
            .corners
            .iter()
            .flatten()
            .any(|coordinate| matches!(coordinate, ScriptCoordinate::Constant(v) if v.is_nan()))
        {
            return Ok(true);
        }
        if script.corners.iter().flatten().any(|coordinate| matches!(coordinate,
            ScriptCoordinate::Lane { steps, .. } if steps.iter().flatten().any(|step| step.operand().is_nan()))) {
            return Ok(true);
        }
        for source in &script.sources {
            for index in 0..source.len() {
                if index.is_multiple_of(1024) {
                    crate::dsp::check_cancel(cancel)?;
                }
                if source.value(index).is_nan() {
                    return Ok(true);
                }
            }
        }
        crate::dsp::check_cancel(cancel)?;
        Ok(false)
    }
    pub(crate) fn script_coordinates(&self) -> Option<CoordinateScript> {
        let Representation::Rectangles { corners, count } = &self.representation else {
            return None;
        };
        let sources = self.backings();
        let corners = std::array::from_fn(|corner| {
            std::array::from_fn(|axis| match &corners[corner][axis] {
                Coordinate::Constant(value) => ScriptCoordinate::Constant(*value),
                Coordinate::Lane(lane) => ScriptCoordinate::Lane {
                    source: sources
                        .iter()
                        .position(|source| source.same(&lane.backing))
                        .expect("retained source"),
                    offset: lane.offset,
                    stride: lane.stride,
                    steps: lane.steps,
                },
            })
        });
        Some(CoordinateScript {
            sources: sources.into_iter().map(CoordinateSource).collect(),
            corners,
            count: *count,
        })
    }
}
