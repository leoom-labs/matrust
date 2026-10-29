//! Editable model file identity and bounded, semantic external-change checks.
use super::{Block, Model};
use crate::{
    Result,
    document::{absolute_path, same_path},
};
use std::path::{Path, PathBuf};

impl Model {
    /// Allocation-free storage comparison, including signed zero and NaN bits.
    /// This is also useful for incomplete diagrams which cannot yet simulate.
    pub fn same_storage(&self, other: &Self) -> bool {
        self.version == other.version
            && self.steps == other.steps
            && self.sample_time.to_bits() == other.sample_time.to_bits()
            && bytemuck::cast_slice::<[f32; 2], u32>(&self.positions)
                == bytemuck::cast_slice::<[f32; 2], u32>(&other.positions)
            && self.nodes.len() == other.nodes.len()
            && self.nodes.iter().zip(&other.nodes).all(|(a, b)| {
                a.name == b.name && a.inputs == b.inputs && same_block(&a.block, &b.block)
            })
    }
}
fn same_block(a: &Block, b: &Block) -> bool {
    use Block::*;
    match (a, b) {
        (Constant { value: a }, Constant { value: b })
        | (Gain { gain: a }, Gain { gain: b })
        | (Bias { bias: a }, Bias { bias: b })
        | (UnitDelay { initial: a }, UnitDelay { initial: b }) => a.to_bits() == b.to_bits(),
        (
            Sine {
                amplitude: a,
                frequency_hz: b,
                phase: c,
            },
            Sine {
                amplitude: x,
                frequency_hz: y,
                phase: z,
            },
        )
        | (
            Step {
                time: a,
                before: b,
                after: c,
            },
            Step {
                time: x,
                before: y,
                after: z,
            },
        ) => a.to_bits() == x.to_bits() && b.to_bits() == y.to_bits() && c.to_bits() == z.to_bits(),
        (Filter { b: a, a: b }, Filter { b: x, a: y }) => {
            bytemuck::cast_slice::<f64, u64>(a) == bytemuck::cast_slice::<f64, u64>(x)
                && bytemuck::cast_slice::<f64, u64>(b) == bytemuck::cast_slice::<f64, u64>(y)
        }
        (SosFilter { sections: a }, SosFilter { sections: b }) => {
            bytemuck::cast_slice::<[f64; 6], u64>(a) == bytemuck::cast_slice::<[f64; 6], u64>(b)
        }
        (Saturation { lower: a, upper: b }, Saturation { lower: x, upper: y }) => {
            a.to_bits() == x.to_bits() && b.to_bits() == y.to_bits()
        }
        (
            DiscreteIntegrator {
                initial: a,
                gain: b,
                method: c,
            },
            DiscreteIntegrator {
                initial: x,
                gain: y,
                method: z,
            },
        ) => a.to_bits() == x.to_bits() && b.to_bits() == y.to_bits() && c == z,
        (Product { operations: a }, Product { operations: b }) => a == b,
        (
            Switch {
                criteria: a,
                threshold: x,
            },
            Switch {
                criteria: b,
                threshold: y,
            },
        ) => a == b && x.to_bits() == y.to_bits(),
        (Sum, Sum) | (Scope, Scope) | (Abs, Abs) | (UnaryMinus, UnaryMinus) => true,
        _ => false,
    }
}

#[derive(Clone, Debug)]
pub enum DiskVersion {
    Missing,
    Model(Model),
}
impl DiskVersion {
    fn matches(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Missing, Self::Missing) => true,
            (Self::Model(a), Self::Model(b)) => a.same_storage(b),
            _ => false,
        }
    }
}
#[derive(Debug)]
pub enum SaveOutcome {
    Saved,
    Conflict(DiskVersion),
}
fn disk_version(path: &Path) -> Result<DiskVersion> {
    match std::fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(DiskVersion::Missing),
        Err(e) => Err(e.to_string()),
        Ok(_) => Model::load(path).map(DiskVersion::Model),
    }
}
#[derive(Clone, Debug)]
pub struct ModelFile {
    pub path: Option<PathBuf>,
    baseline: Option<Model>,
}
impl Default for ModelFile {
    fn default() -> Self {
        Self {
            path: None,
            baseline: Some(Model::default()),
        }
    }
}
impl ModelFile {
    /// Read-only foreign import: unsaved, with no writable source or clean baseline.
    pub fn imported() -> Self {
        Self {
            path: None,
            baseline: None,
        }
    }
    pub fn restored(path: Option<PathBuf>, baseline: Option<Model>) -> Self {
        let baseline = baseline.or_else(|| path.is_none().then(Model::default));
        Self { path, baseline }
    }
    pub fn open(path: &Path) -> Result<(Model, Self)> {
        let path = absolute_path(path)?;
        let model = Model::load(&path)?;
        let file = Self {
            path: Some(path),
            baseline: Some(model.clone()),
        };
        Ok((model, file))
    }
    pub fn baseline(&self) -> Option<&Model> {
        self.baseline.as_ref()
    }
    pub fn dirty(&self, model: &Model) -> bool {
        self.baseline
            .as_ref()
            .is_none_or(|b| !model.same_storage(b))
    }
    pub fn external_change(&self) -> Result<Option<DiskVersion>> {
        let Some(path) = &self.path else {
            return Ok(None);
        };
        let disk = disk_version(path)?;
        let unchanged = matches!(&disk,DiskVersion::Model(m) if self.baseline.as_ref().is_some_and(|b| m.same_storage(b)));
        Ok((!unchanged).then_some(disk))
    }
    /// Best-effort comparison before atomic replacement; not a filesystem lock.
    /// JSON formatting changes are ignored; all model fields and float bits count.
    pub fn try_save(
        &mut self,
        model: &Model,
        path: &Path,
        confirmed: Option<&DiskVersion>,
    ) -> Result<SaveOutcome> {
        if !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("mrsim"))
        {
            return Err(
                "Editable models must use .mrsim; Simulink .slx/.mdl export is not supported"
                    .into(),
            );
        }
        model.validate_storage()?;
        let path = absolute_path(path)?;
        if confirmed.is_some() || self.path.as_ref().is_some_and(|p| same_path(p, &path)) {
            let disk = disk_version(&path)?;
            let unchanged=confirmed.map_or_else(
                || matches!(&disk,DiskVersion::Model(m) if self.baseline.as_ref().is_some_and(|b| m.same_storage(b))),
                |old| old.matches(&disk),
            );
            if !unchanged {
                return Ok(SaveOutcome::Conflict(disk));
            }
        }
        model.save(&path)?;
        self.path = Some(path);
        self.baseline = Some(model.clone());
        Ok(SaveOutcome::Saved)
    }
}
