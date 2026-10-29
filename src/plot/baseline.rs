//! Explicit axes-owned baselines. Empty metadata retains every legacy default.
//! Rendering, persistence and user-command admission are separate follow-ons.
use super::{Axes, AxisScale, AxisTransform, LinePattern, MAX_SERIES, PlotKind};
use crate::{ObjectIdentity, Result};
use std::sync::atomic::AtomicBool;

pub const MAX_BASELINES: usize = 64;
pub const MAX_BASELINE_MEMBERS: usize = 64;

#[derive(Clone, Copy, Debug)]
pub enum BaselineValue {
    Automatic,
    Value(f64),
}

impl BaselineValue {
    pub fn resolve(self, axis: AxisTransform) -> Result<f64> {
        let value = match self {
            Self::Automatic if axis.scale == AxisScale::Linear => 0.,
            Self::Automatic if axis.negative => -1.,
            Self::Automatic => 1.,
            Self::Value(value) => value,
        };
        if !axis.accepts(value) {
            return Err("Baseline value must be finite and match the logarithmic axes sign".into());
        }
        Ok(value)
    }
    fn same_bits(self, other: Self) -> bool {
        match (self, other) {
            (Self::Automatic, Self::Automatic) => true,
            (Self::Value(a), Self::Value(b)) => a.to_bits() == b.to_bits(),
            _ => false,
        }
    }
}

/// Fully specified appearance; no MATLAB omitted-property defaults are inferred.
#[derive(Clone, Copy, Debug)]
pub struct BaselineStyle {
    pub color: [f64; 3],
    pub pattern: LinePattern,
    pub width_points: f64,
    pub visible: bool,
}
impl BaselineStyle {
    /// MatRust's explicit legacy Stem appearance, shared by native and export.
    /// This adopts the historical SVG gray (119), replacing native gray (160).
    /// It is not an inferred MATLAB default and does not create an object.
    pub const fn legacy_stem() -> Self {
        Self {
            color: [119. / 255.; 3],
            pattern: LinePattern::Solid,
            width_points: 0.75,
            visible: true,
        }
    }
    pub fn display_rgb(self) -> [u8; 3] {
        self.color.map(|v| (v * 255.).round() as u8)
    }
    pub fn display_width(self) -> f64 {
        self.width_points * 4. / 3.
    }
    fn validate(self) -> Result<()> {
        if self
            .color
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        {
            return Err("Baseline RGB components must be finite and in 0..1".into());
        }
        super::validate_line_width(self.width_points)
    }
    fn same_bits(self, other: Self) -> bool {
        self.visible == other.visible
            && self.pattern == other.pattern
            && self.width_points.to_bits() == other.width_points.to_bits()
            && self
                .color
                .iter()
                .zip(other.color)
                .all(|(a, b)| a.to_bits() == b.to_bits())
    }
}

/// Shared appearance and coordinate authority; tokens retain no sample buffers.
#[derive(Clone, Debug)]
pub struct Baseline {
    pub identity: ObjectIdentity,
    pub members: Vec<ObjectIdentity>,
    pub value: BaselineValue,
    pub style: BaselineStyle,
}
impl Baseline {
    /// No default style: callers must supply the intended visual properties.
    pub fn new(
        value: BaselineValue,
        style: BaselineStyle,
        members: Vec<ObjectIdentity>,
    ) -> Result<Self> {
        if members.len() > MAX_BASELINE_MEMBERS {
            return Err("Baseline exceeds 64 members".into());
        }
        style.validate()?;
        if matches!(value, BaselineValue::Value(v) if !v.is_finite()) {
            return Err("Baseline value must be finite".into());
        }
        Ok(Self {
            identity: ObjectIdentity::default(),
            members,
            value,
            style,
        })
    }
}

impl Axes {
    /// Existing baseline-free renderers historically omit invalid log segments.
    /// Keep that behavior; only explicit records impose the new value contract.
    pub fn render_baseline(&self, index: usize, axis: AxisTransform) -> Result<f64> {
        if self.baselines.len() > MAX_BASELINES {
            return Err("Baseline record bound exceeded".into());
        }
        let series = self
            .series
            .get(index)
            .ok_or("Baseline series index out of range")?;
        match self.baseline_for_series(&series.identity) {
            Some(record) => record.value.resolve(axis),
            None => Ok(series.stem_baseline(axis)),
        }
    }

    /// Prepare a series using its axes-owned base, without copying its samples.
    /// Only the small Series descriptor is cloned for an explicitly bound Stem.
    pub fn display_series_points_cancellable(
        &self,
        index: usize,
        max_points: usize,
        axes: [AxisTransform; 2],
        cancel: &AtomicBool,
    ) -> Result<Vec<[f64; 2]>> {
        super::display_check(cancel)?;
        let series = self
            .series
            .get(index)
            .ok_or("Display series index out of range")?;
        if series.kind == PlotKind::Stem && self.baseline_for_series(&series.identity).is_some() {
            let mut descriptor = series.clone();
            descriptor.base_value = Some(self.effective_baseline(index, axes[1])?);
            descriptor.display_points_axes_cancellable(max_points, axes, cancel)
        } else {
            series.display_points_axes_cancellable(max_points, axes, cancel)
        }
    }
    pub fn display_series_points(
        &self,
        index: usize,
        max_points: usize,
        axes: [AxisTransform; 2],
    ) -> Vec<[f64; 2]> {
        self.display_series_points_cancellable(index, max_points, axes, &AtomicBool::new(false))
            .expect("Validated axes display metadata")
    }

    fn validate_baseline_records(
        &self,
        records: &[Baseline],
        check: &impl Fn() -> Result<()>,
    ) -> Result<()> {
        check()?;
        if records.len() > MAX_BASELINES || self.series.len() > MAX_SERIES {
            return Err("Axes baseline metadata exceeds 64 objects or 64 series".into());
        }
        let mut total = 0usize;
        for baseline in records {
            total = total
                .checked_add(baseline.members.len())
                .ok_or("Baseline membership overflow")?;
            if total > MAX_BASELINE_MEMBERS {
                return Err("Axes baselines exceed 64 total members".into());
            }
        }
        let mut seen = Vec::with_capacity(total);
        for (index, baseline) in records.iter().enumerate() {
            check()?;
            baseline.style.validate()?;
            if matches!(baseline.value, BaselineValue::Value(v) if !v.is_finite()) {
                return Err("Baseline value must be finite".into());
            }
            if self.identity.same_object(&baseline.identity)
                || self
                    .series
                    .iter()
                    .any(|s| s.identity.same_object(&baseline.identity))
                || self
                    .legend_state
                    .iter()
                    .any(|s| s.identity.same_object(&baseline.identity))
                || records[..index]
                    .iter()
                    .any(|b| b.identity.same_object(&baseline.identity))
            {
                return Err("Duplicate baseline or axes object identity".into());
            }
            for member in &baseline.members {
                check()?;
                let series = self
                    .series
                    .iter()
                    .find(|s| s.identity.same_object(member))
                    .ok_or("Baseline member does not belong to its axes")?;
                if !matches!(series.kind, PlotKind::Bar | PlotKind::Stem) {
                    return Err("Baseline members must be Bar or Stem series".into());
                }
                if seen
                    .iter()
                    .any(|old: &&ObjectIdentity| old.same_object(member))
                {
                    return Err("A series belongs to more than one baseline membership".into());
                }
                seen.push(member);
                if let Some(value) = series.base_value
                    && !matches!(baseline.value, BaselineValue::Value(v) if v.to_bits() == value.to_bits())
                {
                    return Err("Baseline conflicts with the series BaseValue".into());
                }
            }
        }
        if !records.is_empty() && self.yscale == AxisScale::Log {
            // One existing cancellable sign scan, never once per baseline/member.
            let axis = self.axis_transforms_checked(check)?[1];
            for baseline in records {
                baseline.value.resolve(axis)?;
            }
        }
        check()
    }
    pub(super) fn validate_baselines(&self) -> Result<()> {
        self.validate_baseline_records(&self.baselines, &|| Ok(()))?;
        self.validate_latent_baseline_links(&self.baselines)
    }
    /// Metadata edit with final-check publication. No sample buffers are cloned.
    pub fn replace_baselines(
        &mut self,
        baselines: Vec<Baseline>,
        cancel: &AtomicBool,
    ) -> Result<()> {
        self.replace_baselines_checked(baselines, &|| super::display_check(cancel))
    }
    fn replace_baselines_checked(
        &mut self,
        baselines: Vec<Baseline>,
        check: &impl Fn() -> Result<()>,
    ) -> Result<()> {
        check()?;
        if self.series.len() > MAX_SERIES || baselines.len() > MAX_BASELINES {
            return Err("Axes baseline metadata exceeds 64 objects or 64 series".into());
        }
        self.validate_baseline_records(&baselines, check)?;
        let latent = self
            .plan_latent_baseline_transition(&baselines, check)?
            .unwrap_or_else(|| {
                self.series
                    .iter()
                    .map(|s| s.baseline_identity.clone())
                    .collect()
            });
        let (latent, visibility) = self.plan_bar_policy_transition(&baselines, latent, check)?;
        check()?;
        for ((series, identity), visible) in self.series.iter_mut().zip(latent).zip(visibility) {
            series.baseline_identity = identity;
            series.bar_baseline_visible = visible;
        }
        self.baselines = baselines;
        Ok(())
    }
    /// Change an existing chart baseline and all linked BaseValue properties in
    /// one publication. Appearance-only edits leave automatic/member values intact.
    /// Used by public property routing; it never creates or merges membership.
    pub fn edit_baseline(
        &mut self,
        identity: &ObjectIdentity,
        value: Option<f64>,
        style: Option<BaselineStyle>,
        cancel: &AtomicBool,
    ) -> Result<()> {
        self.edit_baseline_checked(identity, value, style, &|| super::display_check(cancel))
    }
    fn edit_baseline_checked(
        &mut self,
        identity: &ObjectIdentity,
        value: Option<f64>,
        style: Option<BaselineStyle>,
        check: &impl Fn() -> Result<()>,
    ) -> Result<()> {
        check()?;
        // Validate bounds before cloning any public metadata vector. Existing
        // malformed/conflicting membership cannot be repaired accidentally by set.
        self.validate_baseline_records(&self.baselines, check)?;
        if value.is_some_and(|v| !v.is_finite()) {
            return Err("Baseline BaseValue must be finite".into());
        }
        if let Some(style) = style {
            style.validate()?;
        }
        let index = self
            .baselines
            .iter()
            .position(|b| b.identity.same_object(identity))
            .ok_or("Baseline handle is stale or belongs to another axes")?;
        let mut candidate = self.clone(); // Series sample Arcs remain shared.
        if let Some(value) = value {
            candidate.baselines[index].value = BaselineValue::Value(value);
            for member in &candidate.baselines[index].members {
                check()?;
                let series = candidate
                    .series
                    .iter_mut()
                    .find(|s| s.identity.same_object(member))
                    .ok_or("Baseline member is no longer live")?;
                // Bar geometry is owned by the explicit record; the legacy
                // Series.base_value field is admitted only for Stem.
                if series.kind == PlotKind::Stem {
                    series.base_value = Some(value);
                }
            }
        }
        if let Some(style) = style {
            candidate.baselines[index].style = style;
        }
        candidate.validate_baseline_records(&candidate.baselines, check)?;
        check()?;
        *self = candidate;
        Ok(())
    }

    /// Resolve against already prepared axis transforms; no data scan/copy.
    pub fn effective_baseline(&self, series_index: usize, axis: AxisTransform) -> Result<f64> {
        let series = self
            .series
            .get(series_index)
            .ok_or("Baseline series index out of range")?;
        if !matches!(series.kind, PlotKind::Bar | PlotKind::Stem) {
            return Err("Only Bar and Stem series have baseline geometry".into());
        }
        if let Some(baseline) = self.baseline_for_series(&series.identity) {
            return baseline.value.resolve(axis);
        }
        // Preserve legacy signed-zero and automatic linear/log behavior.
        let value = series.stem_baseline(axis);
        if !axis.accepts(value) {
            return Err("Invalid series baseline for the axes".into());
        }
        Ok(value)
    }
    pub fn baseline_for_series(&self, member: &ObjectIdentity) -> Option<&Baseline> {
        self.baselines.iter().take(MAX_BASELINES).find(|b| {
            b.members
                .iter()
                .take(MAX_BASELINE_MEMBERS)
                .any(|m| m.same_object(member))
        })
    }
    pub(super) fn same_baseline_metadata(&self, other: &Self) -> bool {
        self.baselines.len() == other.baselines.len()
            && self.baselines.iter().zip(&other.baselines).all(|(a, b)| {
                a.value.same_bits(b.value)
                    && a.style.same_bits(b.style)
                    && a.members.len() == b.members.len()
                    && a.members.iter().zip(&b.members).all(|(x, y)| {
                        self.series.iter().position(|s| s.identity.same_object(x))
                            == other.series.iter().position(|s| s.identity.same_object(y))
                    })
            })
    }
    pub(super) fn remove_baseline_member(&mut self, identity: &ObjectIdentity) {
        self.baselines.retain_mut(|baseline| {
            let standalone = baseline.members.is_empty();
            baseline.members.retain(|id| !id.same_object(identity));
            standalone || !baseline.members.is_empty()
        });
    }
}

#[cfg(test)]
#[path = "baseline_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "baseline_consumer_tests.rs"]
mod consumer_tests;

#[path = "baseline_observer.rs"]
mod observer;
pub use observer::BaselineObservation;
