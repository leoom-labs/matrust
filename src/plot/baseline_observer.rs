//! Bounded, non-materializing baseline observations and private-candidate edits.
use super::*;
use crate::plot::Series;

#[derive(Clone, Copy)]
pub struct BaselineObservation<'a> {
    pub identity: &'a ObjectIdentity,
    pub value: BaselineValue,
    pub style: BaselineStyle,
}

impl Axes {
    fn baseline_observation_bounds(&self) -> Result<()> {
        if self.series.len() > MAX_SERIES
            || self.baselines.len() > MAX_BASELINES
            || self
                .baselines
                .iter()
                .try_fold(0usize, |n, b| n.checked_add(b.members.len()))
                .is_none_or(|n| n > MAX_BASELINE_MEMBERS)
        {
            return Err("Baseline observation exceeds bounded metadata".into());
        }
        self.validate_latent_baseline_links(&self.baselines)
    }

    /// An explicit record identity cannot also name an omitted latent member.
    /// Membership-changing transactions plan fresh tokens for that subgroup;
    /// direct malformed public-field/wire construction is rejected, not repaired.
    pub(super) fn validate_latent_baseline_links(&self, records: &[Baseline]) -> Result<()> {
        for (index, series) in self.series.iter().enumerate() {
            if series.kind != PlotKind::Bar && series.bar_baseline_visible {
                return Err("Implicit Bar visibility requires a Bar series".into());
            }
            if series.kind == PlotKind::Bar
                && !records
                    .iter()
                    .any(|b| b.members.iter().any(|m| m.same_object(&series.identity)))
            {
                for previous in self.series[..index]
                    .iter()
                    .filter(|s| s.kind == PlotKind::Bar)
                {
                    if previous
                        .baseline_identity
                        .same_object(&series.baseline_identity)
                        && !records
                            .iter()
                            .any(|b| b.members.iter().any(|m| m.same_object(&previous.identity)))
                        && previous.bar_baseline_visible != series.bar_baseline_visible
                    {
                        return Err(
                            "One implicit Bar baseline group has conflicting visibility".into()
                        );
                    }
                }
            }
            if !matches!(series.kind, PlotKind::Bar | PlotKind::Stem) {
                continue;
            }
            if let Some(record) = records
                .iter()
                .find(|b| b.identity.same_object(&series.baseline_identity))
                && !record
                    .members
                    .iter()
                    .any(|m| m.same_object(&series.identity))
            {
                return Err("Latent baseline identity conflicts with explicit membership".into());
            }
        }
        Ok(())
    }
    pub(super) fn plan_latent_baseline_transition(
        &self,
        records: &[Baseline],
        check: &impl Fn() -> Result<()>,
    ) -> Result<Option<Vec<ObjectIdentity>>> {
        check()?;
        if self.series.len() > MAX_SERIES
            || records.len() > MAX_BASELINES
            || records
                .iter()
                .try_fold(0usize, |n, b| n.checked_add(b.members.len()))
                .is_none_or(|n| n > MAX_BASELINE_MEMBERS)
        {
            return Err("Baseline transition exceeds bounded metadata".into());
        }
        let mut replacements: Vec<(ObjectIdentity, ObjectIdentity)> = Vec::new();
        let mut planned: Option<Vec<ObjectIdentity>> = None;
        for (index, series) in self.series.iter().enumerate() {
            check()?;
            if !matches!(series.kind, PlotKind::Bar | PlotKind::Stem) {
                continue;
            }
            let Some(record) = records
                .iter()
                .find(|b| b.identity.same_object(&series.baseline_identity))
            else {
                continue;
            };
            if record
                .members
                .iter()
                .any(|m| m.same_object(&series.identity))
            {
                continue;
            }
            let identity = if let Some((_, fresh)) = replacements
                .iter()
                .find(|(old, _)| old.same_object(&series.baseline_identity))
            {
                fresh.clone()
            } else {
                let fresh = ObjectIdentity::default();
                replacements.push((series.baseline_identity.clone(), fresh.clone()));
                fresh
            };
            planned.get_or_insert_with(|| {
                self.series
                    .iter()
                    .map(|s| s.baseline_identity.clone())
                    .collect()
            })[index] = identity;
        }
        check()?;
        Ok(planned)
    }

    /// Bounded metadata-only enumeration: one stroke per visible implicit Bar
    /// group. Explicit records are authoritative and rendered separately.
    /// Chart Visible does not rewrite the independent baseline visibility.
    pub fn implicit_bar_baselines(&self) -> impl Iterator<Item = usize> + '_ {
        self.series
            .iter()
            .take(MAX_SERIES)
            .enumerate()
            .filter_map(|(index, series)| {
                (series.kind == PlotKind::Bar
                    && series.bar_baseline_visible
                    && self.baseline_for_series(&series.identity).is_none()
                    && !self.series[..index].iter().any(|previous| {
                        previous.kind == PlotKind::Bar
                            && previous.bar_baseline_visible
                            && previous
                                .baseline_identity
                                .same_object(&series.baseline_identity)
                            && self.baseline_for_series(&previous.identity).is_none()
                    }))
                .then_some(index)
            })
    }

    /// Membership removal carries actual visibility into the surviving implicit
    /// group. Differently visible omitted subgroups receive distinct identities.
    pub(super) fn plan_bar_policy_transition(
        &self,
        records: &[Baseline],
        mut identities: Vec<ObjectIdentity>,
        check: &impl Fn() -> Result<()>,
    ) -> Result<(Vec<ObjectIdentity>, Vec<bool>)> {
        check()?;
        self.validate_latent_baseline_links(&self.baselines)?;
        let mut visibility: Vec<_> = self.series.iter().map(|s| s.bar_baseline_visible).collect();
        let mut groups: Vec<(ObjectIdentity, bool, ObjectIdentity)> = Vec::new();
        for (index, series) in self.series.iter().enumerate() {
            check()?;
            if series.kind != PlotKind::Bar
                || records
                    .iter()
                    .any(|b| b.members.iter().any(|m| m.same_object(&series.identity)))
            {
                continue;
            }
            if let Some(old) = self.baseline_for_series(&series.identity) {
                visibility[index] = old.style.visible;
            }
            let old = identities[index].clone();
            let token = if let Some((_, _, token)) = groups
                .iter()
                .find(|(id, on, _)| id.same_object(&old) && *on == visibility[index])
            {
                token.clone()
            } else {
                let token = if groups.iter().any(|(id, _, _)| id.same_object(&old)) {
                    ObjectIdentity::default()
                } else {
                    old.clone()
                };
                groups.push((old, visibility[index], token.clone()));
                token
            };
            identities[index] = token;
        }
        check()?;
        Ok((identities, visibility))
    }

    fn baseline_identities(&self) -> impl Iterator<Item = &ObjectIdentity> + Clone {
        self.series
            .iter()
            .filter(|s| matches!(s.kind, PlotKind::Bar | PlotKind::Stem))
            .map(|s| &s.baseline_identity)
            .chain(self.baselines.iter().map(|b| &b.identity))
    }
    /// Persistent equivalence classes affect recovery even when appearance and
    /// all sample Arcs are unchanged. Ordinary snapshot clones take the linear
    /// same-token path; fresh identities compare bounded relationships without
    /// allocating or depending on process address equality across reopens.
    pub(in crate::plot) fn same_latent_baseline_metadata(&self, other: &Self) -> bool {
        if self.baseline_observation_bounds().is_err()
            || other.baseline_observation_bounds().is_err()
        {
            return false;
        }
        let left = self.baseline_identities();
        let right = other.baseline_identities();
        if left.clone().count() != right.clone().count() {
            return false;
        }
        if left
            .clone()
            .zip(right.clone())
            .all(|(a, b)| a.same_object(b))
        {
            return true;
        }
        for (a, b) in left.clone().zip(right.clone()) {
            for (c, d) in left.clone().zip(right.clone()) {
                if a.same_object(c) != b.same_object(d) {
                    return false;
                }
            }
        }
        true
    }

    /// No sample traversal, allocation, record insertion, or appearance change.
    pub fn observe_series_baseline(&self, index: usize) -> Result<BaselineObservation<'_>> {
        self.baseline_observation_bounds()?;
        let series = self
            .series
            .get(index)
            .ok_or("Baseline series is no longer live")?;
        if !matches!(series.kind, PlotKind::Bar | PlotKind::Stem) {
            return Err("Only Bar and Stem series have a chart baseline".into());
        }
        if let Some(record) = self.baseline_for_series(&series.identity) {
            return Ok(BaselineObservation {
                identity: &record.identity,
                value: record.value,
                style: record.style,
            });
        }
        let mut style = BaselineStyle::legacy_stem();
        if series.kind == PlotKind::Bar {
            style.visible = series.bar_baseline_visible;
        }
        Ok(BaselineObservation {
            identity: &series.baseline_identity,
            value: series
                .base_value
                .map_or(BaselineValue::Automatic, BaselineValue::Value),
            style,
        })
    }

    pub fn observe_baseline(&self, identity: &ObjectIdentity) -> Result<BaselineObservation<'_>> {
        self.baseline_observation_bounds()?;
        if let Some(record) = self
            .baselines
            .iter()
            .find(|b| b.identity.same_object(identity))
        {
            return Ok(BaselineObservation {
                identity: &record.identity,
                value: record.value,
                style: record.style,
            });
        }
        for (index, series) in self.series.iter().enumerate() {
            if matches!(series.kind, PlotKind::Bar | PlotKind::Stem)
                && series.baseline_identity.same_object(identity)
                && self.baseline_for_series(&series.identity).is_none()
            {
                return self.observe_series_baseline(index);
            }
        }
        Err("Baseline handle is stale or belongs to another axes".into())
    }

    /// Effective Automatic queries may inspect samples. Call on the numerical
    /// worker, not from inspector liveness/metadata queries on a UI thread.
    pub fn baseline_value_cancellable(
        &self,
        identity: &ObjectIdentity,
        cancel: &AtomicBool,
    ) -> Result<f64> {
        super::super::display_check(cancel)?;
        match self.observe_baseline(identity)?.value {
            BaselineValue::Value(value) => Ok(value),
            BaselineValue::Automatic if self.yscale == AxisScale::Linear => Ok(0.),
            BaselineValue::Automatic => {
                let axis =
                    self.axis_transforms_checked(&|| super::super::display_check(cancel))?[1];
                BaselineValue::Automatic.resolve(axis)
            }
        }
    }

    /// Prepare fresh public Bar descriptors to share a live baseline group.
    /// Tokens survive removal/reordering of any member. Removing the last member
    /// destroys the group; a later Bar batch has a fresh token and cannot revive it.
    pub(in crate::plot) fn prepare_bar_baseline_join(
        &self,
        incoming: &mut [Series],
    ) -> Result<Option<(usize, Vec<ObjectIdentity>)>> {
        self.baseline_observation_bounds()?;
        if !incoming.iter().any(|s| s.kind == PlotKind::Bar) {
            return Ok(None);
        }
        let bars: Vec<_> = self
            .series
            .iter()
            .filter(|s| s.kind == PlotKind::Bar)
            .collect();
        let mut explicit = None;
        let identity = if let Some(unbound) = bars
            .iter()
            .find(|s| self.baseline_for_series(&s.identity).is_none())
        {
            unbound.baseline_identity.clone()
        } else if let Some(first) = bars.first() {
            if let Some(record) = self.baseline_for_series(&first.identity) {
                // Do not rewrite arbitrary imported or mixed membership. Only
                // the already-complete, Bar-only group accepts a new Bar member.
                let canonical = record.members.len() == bars.len()
                    && bars
                        .iter()
                        .all(|s| record.members.iter().any(|m| m.same_object(&s.identity)));
                if canonical {
                    explicit = self
                        .baselines
                        .iter()
                        .position(|b| b.identity.same_object(&record.identity));
                    record.identity.clone()
                } else {
                    ObjectIdentity::default()
                }
            } else {
                first.baseline_identity.clone()
            }
        } else {
            ObjectIdentity::default()
        };
        let visibility = explicit
            .map(|index| self.baselines[index].style.visible)
            .or_else(|| {
                bars.iter()
                    .find(|s| s.baseline_identity.same_object(&identity))
                    .map(|s| {
                        self.baseline_for_series(&s.identity)
                            .map_or(s.bar_baseline_visible, |b| b.style.visible)
                    })
            })
            .unwrap_or_else(|| {
                incoming
                    .iter()
                    .find(|s| s.kind == PlotKind::Bar)
                    .is_some_and(|s| s.bar_baseline_visible)
            });
        let mut members = Vec::new();
        for series in incoming.iter_mut().filter(|s| s.kind == PlotKind::Bar) {
            series.bar_baseline_visible = visibility;
            series.baseline_identity = identity.clone();
            if explicit.is_some() {
                members.push(series.identity.clone());
            }
        }
        if let Some(index) = explicit {
            let total: usize = self.baselines.iter().map(|b| b.members.len()).sum();
            if total + members.len() > MAX_BASELINE_MEMBERS {
                return Err("Axes baselines exceed 64 total members".into());
            }
            Ok(Some((index, members)))
        } else {
            Ok(None)
        }
    }

    /// Materialize only when a real edit succeeds. The latent handle identity
    /// remains valid across materialization; no current selection is changed.
    pub fn set_observed_baseline(
        &mut self,
        identity: &ObjectIdentity,
        value: Option<f64>,
        style: Option<BaselineStyle>,
        cancel: &AtomicBool,
    ) -> Result<()> {
        self.set_observed_baseline_checked(identity, value, style, &|| {
            super::super::display_check(cancel)
        })
    }
    fn set_observed_baseline_checked(
        &mut self,
        identity: &ObjectIdentity,
        value: Option<f64>,
        style: Option<BaselineStyle>,
        check: &impl Fn() -> Result<()>,
    ) -> Result<()> {
        check()?;
        self.baseline_observation_bounds()?;
        self.validate_baseline_records(&self.baselines, check)?;
        let observed = self.observe_baseline(identity)?;
        let initial_value = observed.value;
        let initial_style = observed.style;
        // An idempotent style-only property set must not materialize a latent
        // object. This also preserves the pure default during genuine replay.
        if value.is_none()
            && style.is_none_or(|style| style.same_bits(initial_style))
            && self
                .series
                .iter()
                .any(|s| s.kind == PlotKind::Bar && s.baseline_identity.same_object(identity))
        {
            check()?;
            return Ok(());
        }
        // Visibility is already a bounded implicit policy. A genuine style-only
        // update that changes no other property retains the latent object and
        // historical no-record topology, including after ordinary script replay.
        if value.is_none()
            && !self
                .baselines
                .iter()
                .any(|b| b.identity.same_object(identity))
            && let Some(next_style) = style
            && (BaselineStyle {
                visible: initial_style.visible,
                ..next_style
            })
            .same_bits(initial_style)
            && self
                .series
                .iter()
                .any(|s| s.kind == PlotKind::Bar && s.baseline_identity.same_object(identity))
            && !self.series.iter().any(|s| {
                s.kind == PlotKind::Stem
                    && s.baseline_identity.same_object(identity)
                    && self.baseline_for_series(&s.identity).is_none()
            })
        {
            let mut members = Vec::new();
            for (index, series) in self.series.iter().enumerate() {
                check()?;
                if series.kind == PlotKind::Bar
                    && series.baseline_identity.same_object(identity)
                    && self.baseline_for_series(&series.identity).is_none()
                {
                    members.push(index);
                }
            }
            check()?;
            for index in members {
                self.series[index].bar_baseline_visible = next_style.visible;
            }
            return Ok(());
        }
        let mut candidate = self.clone();
        if !candidate
            .baselines
            .iter()
            .any(|b| b.identity.same_object(identity))
        {
            if candidate.baselines.len() == MAX_BASELINES {
                return Err("Axes baseline metadata exceeds 64 objects".into());
            }
            let mut members = Vec::new();
            for series in &candidate.series {
                check()?;
                if matches!(series.kind, PlotKind::Bar | PlotKind::Stem)
                    && series.baseline_identity.same_object(identity)
                    && candidate.baseline_for_series(&series.identity).is_none()
                {
                    members.push(series.identity.clone());
                }
            }
            if members.is_empty() {
                return Err("Baseline has no live chart members".into());
            }
            candidate.baselines.push(Baseline {
                identity: identity.clone(),
                members,
                value: initial_value,
                style: initial_style,
            });
        }
        if let Some(latent) =
            candidate.plan_latent_baseline_transition(&candidate.baselines, check)?
        {
            for (series, token) in candidate.series.iter_mut().zip(latent) {
                series.baseline_identity = token;
            }
        }
        candidate.edit_baseline_checked(identity, value, style, check)?;
        check()?;
        *self = candidate;
        Ok(())
    }
}

#[cfg(test)]
#[path = "baseline_observer_tests.rs"]
mod tests;
