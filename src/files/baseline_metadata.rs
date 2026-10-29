//! Bounded native baseline metadata; object identities are reconstructed locally.
use super::*;
use crate::plot::{
    Axes, Baseline, BaselineStyle, BaselineValue, LinePattern, MAX_BASELINE_MEMBERS, MAX_BASELINES,
    PlotKind, Series,
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NativeBaseline {
    members: Vec<usize>,
    /// None is Automatic; Some preserves the exact finite f64 value including -0.
    value: Option<f64>,
    color: [f64; 3],
    pattern: LinePattern,
    width_points: f64,
    visible: bool,
}
impl NativeBaseline {
    fn value(&self) -> BaselineValue {
        self.value
            .map_or(BaselineValue::Automatic, BaselineValue::Value)
    }
    fn style(&self) -> BaselineStyle {
        BaselineStyle {
            color: self.color,
            pattern: self.pattern,
            width_points: self.width_points,
            visible: self.visible,
        }
    }
}
pub(super) fn encode(axes: &Axes) -> Result<Vec<NativeBaseline>> {
    axes.baselines
        .iter()
        .map(|b| {
            let members = b
                .members
                .iter()
                .map(|id| {
                    axes.series
                        .iter()
                        .position(|s| s.identity.same_object(id))
                        .ok_or("Missing baseline member".to_owned())
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(NativeBaseline {
                members,
                value: match b.value {
                    BaselineValue::Automatic => None,
                    BaselineValue::Value(v) => Some(v),
                },
                color: b.style.color,
                pattern: b.style.pattern,
                width_points: b.style.width_points,
                visible: b.style.visible,
            })
        })
        .collect()
}
/// Runs before native sample allocations. The containing JSON uses the existing 64-KiB ordinary / 320-KiB descriptor-capable cap.
pub(super) fn preflight(records: &[NativeBaseline], series: &[SeriesMetadata]) -> Result<()> {
    if records.len() > MAX_BASELINES {
        return Err("Native baseline count exceeds 64".into());
    }
    let mut total = 0usize;
    let mut seen = [false; crate::plot::MAX_SERIES];
    for record in records {
        total = total
            .checked_add(record.members.len())
            .ok_or("Native baseline membership overflow")?;
        if total > MAX_BASELINE_MEMBERS {
            return Err("Native baseline membership exceeds 64".into());
        }
        Baseline::new(record.value(), record.style(), Vec::new())?;
        for &index in &record.members {
            let s = series
                .get(index)
                .filter(|_| index < seen.len())
                .ok_or("Invalid native baseline member")?;
            if !matches!(s.kind, PlotKind::Bar | PlotKind::Stem) || seen[index] {
                return Err("Invalid or duplicate native baseline membership".into());
            }
            if s.base_value
                .is_some_and(|v| record.value.is_none_or(|b| b.to_bits() != v.to_bits()))
            {
                return Err("Native baseline conflicts with series BaseValue".into());
            }
            seen[index] = true;
        }
    }
    Ok(())
}
pub(super) fn decode(records: Vec<NativeBaseline>, series: &[Series]) -> Result<Vec<Baseline>> {
    records
        .into_iter()
        .map(|record| {
            let members = record
                .members
                .iter()
                .map(|&i| {
                    series
                        .get(i)
                        .map(|s| s.identity.clone())
                        .ok_or("Invalid native baseline member".to_owned())
                })
                .collect::<Result<Vec<_>>>()?;
            Baseline::new(record.value(), record.style(), members)
        })
        .collect()
}

// This optional P/Q field records identity equivalence, not process addresses.
// Slots 0..records.len() name decoded explicit records. Subsequent slots are
// first-use ordered fresh identities; 255 means a non-chart series. At most
// 64 records + 64 series identities exist. The surrounding ordinary 64-KiB JSON cap
// or descriptor-capable 320-KiB cap bounds serde staging; preflight precedes point allocations.
const NO_LATENT_BASELINE: u8 = u8::MAX;
fn chart(kind: PlotKind) -> bool {
    matches!(kind, PlotKind::Bar | PlotKind::Stem)
}

pub(super) fn needs_bar_visibility(axes: &Axes) -> bool {
    axes.series
        .iter()
        .any(|s| s.kind == PlotKind::Bar && s.bar_baseline_visible)
}
pub(super) fn needs_latent(axes: &Axes) -> bool {
    if !axes.baselines.is_empty() {
        return true;
    }
    for (i, a) in axes.series.iter().enumerate() {
        if !chart(a.kind) {
            continue;
        }
        for b in axes.series.iter().take(i).filter(|b| chart(b.kind)) {
            let historical = a.kind == PlotKind::Bar && b.kind == PlotKind::Bar;
            if a.baseline_identity.same_object(&b.baseline_identity) != historical {
                return true;
            }
        }
    }
    false
}
/// Called only after full Figure validation. Canonical legacy figures retain
/// exactly their former bytes. Explicit records retain even dormant latent
/// links, including links to a materialized record's own identity.
pub(super) fn encode_latent(axes: &Axes) -> Option<Vec<u8>> {
    if !needs_latent(axes) {
        return None;
    }
    let mut identities: Vec<&crate::ObjectIdentity> =
        axes.baselines.iter().map(|b| &b.identity).collect();
    let mut result = Vec::with_capacity(axes.series.len());
    for series in &axes.series {
        if !chart(series.kind) {
            result.push(NO_LATENT_BASELINE);
            continue;
        }
        let index = if let Some(index) = identities
            .iter()
            .position(|id| id.same_object(&series.baseline_identity))
        {
            index
        } else {
            let index = identities.len();
            identities.push(&series.baseline_identity);
            index
        };
        // Validated axes have <=64 records and <=64 series, below the sentinel.
        result.push(u8::try_from(index).expect("bounded native baseline identity index"));
    }
    Some(result)
}
pub(super) fn preflight_latent(
    links: Option<&[u8]>,
    records: &[NativeBaseline],
    series: &[SeriesMetadata],
) -> Result<()> {
    if series.len() > crate::plot::MAX_SERIES || links.is_some_and(|v| v.len() != series.len()) {
        return Err("Invalid native latent baseline extent".into());
    }
    // Legacy absence joins all Bars; P/Q links can preserve separate groups.
    // Reject inconsistent implicit policy before any sample allocation.
    for (index, item) in series.iter().enumerate() {
        if item.kind != PlotKind::Bar {
            continue;
        }
        if records.iter().any(|b| b.members.contains(&index)) {
            continue;
        }
        for (previous, other) in series[..index].iter().enumerate() {
            if other.kind == PlotKind::Bar
                && !records.iter().any(|b| b.members.contains(&previous))
                && links.is_none_or(|v| v[index] == v[previous])
                && item.bar_baseline_visible != other.bar_baseline_visible
            {
                return Err("Native implicit Bar group has conflicting visibility".into());
            }
        }
    }
    let Some(links) = links else {
        return Ok(());
    };
    if records.len() > MAX_BASELINES
        || series.len() > crate::plot::MAX_SERIES
        || links.len() != series.len()
    {
        return Err("Invalid native latent baseline extent".into());
    }
    let mut next = records.len();
    for (series_index, (&link, series)) in links.iter().zip(series).enumerate() {
        if !chart(series.kind) {
            if link != NO_LATENT_BASELINE {
                return Err("Native latent baseline requires a chart series".into());
            }
            continue;
        }
        let index = usize::from(link);
        if link == NO_LATENT_BASELINE || index > next {
            return Err("Invalid native latent baseline identity order".into());
        }
        if let Some(record) = records.get(index)
            && !record.members.contains(&series_index)
        {
            return Err("Native latent baseline is outside explicit membership".into());
        }
        if index == next {
            next += 1;
        }
    }
    Ok(())
}
/// Preflight has bounded all indices and payloads. Rebind records first so
/// latent/materialized identities remain equal through later record removal.
pub(super) fn decode_latent(links: Option<&[u8]>, records: &[Baseline], series: &mut [Series]) {
    let Some(links) = links else {
        let bar = crate::ObjectIdentity::default();
        for series in series {
            if series.kind == PlotKind::Bar {
                series.baseline_identity = bar.clone();
            }
        }
        return;
    };
    let mut identities: Vec<_> = records.iter().map(|b| b.identity.clone()).collect();
    for (&link, series) in links.iter().zip(series) {
        if link == NO_LATENT_BASELINE {
            continue;
        }
        let index = usize::from(link);
        if index == identities.len() {
            identities.push(crate::ObjectIdentity::default());
        }
        series.baseline_identity = identities[index].clone();
    }
}
