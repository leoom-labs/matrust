//! Bounded ordered legend membership. Tokens retain no graph or sample buffers.
use super::{Axes, Series};
use crate::{ObjectIdentity, Result};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub struct LegendState {
    pub identity: ObjectIdentity,
    pub members: Vec<ObjectIdentity>,
    pub auto_update: bool,
}

/// On-disk indices are local to one axes; process-local identities never escape.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct LegendMetadata {
    pub members: Vec<usize>,
    pub auto_update: bool,
    #[serde(default, skip_serializing_if = "interpreter_is_default")]
    pub interpreter: super::text_markup::Interpreter,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub num_columns: Option<usize>,
    #[serde(default, skip_serializing_if = "super::LegendOrientation::is_default")]
    pub orientation: super::LegendOrientation,
    #[serde(default, skip_serializing_if = "super::LegendBox::is_default")]
    pub r#box: super::LegendBox,
    #[serde(default, skip_serializing_if = "super::LabelStyle::is_default")]
    pub style: super::LabelStyle,
}
fn interpreter_is_default(value: &super::text_markup::Interpreter) -> bool {
    *value == super::text_markup::Interpreter::Tex
}

impl Axes {
    /// Includes empty labels for String queries and exact persistence. Renderer
    /// callers use legend_entries, which omits current-MATLAB empty labels.
    /// Without explicit legend state, series marked `legend_hidden` (chart
    /// helpers) are never members.
    pub fn legend_members(&self) -> impl Iterator<Item = (usize, &Series)> {
        let explicit = self.legend_state.as_ref().map(|state| {
            let mut indices = HashMap::with_capacity(self.series.len());
            for (index, series) in self.series.iter().enumerate() {
                // Snapshot clones retain identity. Keep the first matching series.
                indices
                    .entry(series.identity.recovery_key())
                    .or_insert(index);
            }
            state.members.iter().filter_map(move |token| {
                let &index = indices.get(&token.recovery_key())?;
                Some((index, &self.series[index]))
            })
        });
        let implicit = explicit.is_none().then(|| {
            self.series
                .iter()
                .enumerate()
                .filter(|(_, s)| !s.legend_hidden)
        });
        explicit
            .into_iter()
            .flatten()
            .chain(implicit.into_iter().flatten())
    }
    pub fn legend_entries(&self) -> impl Iterator<Item = (usize, &Series)> {
        self.legend_members()
            .filter(|(_, s)| !s.label.is_empty() && !s.legend_hidden)
    }
    pub(super) fn validate_legend_state(&self) -> Result<()> {
        if let Some(state) = &self.legend_state {
            if state.members.len() > self.series.len() {
                return Err("Legend exceeds axes membership".into());
            }
            let mut members: HashSet<_> = self
                .series
                .iter()
                .map(|s| s.identity.recovery_key())
                .collect();
            let mut seen = HashSet::new();
            for token in &state.members {
                if !seen.insert(token.recovery_key()) {
                    return Err("Legend contains a duplicate series".into());
                }
                if !members.remove(&token.recovery_key()) {
                    return Err("Legend member does not belong to its axes".into());
                }
            }
        }
        Ok(())
    }
    pub(super) fn same_legend_metadata(&self, other: &Self) -> bool {
        match (&self.legend_state, &other.legend_state) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                a.auto_update == b.auto_update
                    && a.members.len() == b.members.len()
                    && self
                        .legend_members()
                        .map(|(i, _)| i)
                        .eq(other.legend_members().map(|(i, _)| i))
            }
            _ => false,
        }
    }
    /// GUI series deletion is an atomic metadata edit; identity-based order
    /// survives unrelated swaps. Removed objects always leave the legend.
    pub fn remove_series(&mut self, index: usize) -> Result<Series> {
        let series = self.series.get(index).ok_or("Series index out of range")?;
        if let Some(state) = &mut self.legend_state {
            state.members.retain(|id| !id.same_object(&series.identity));
        }
        let identity = series.identity.clone();
        self.remove_baseline_member(&identity);
        Ok(self.series.remove(index))
    }
    pub(crate) fn legend_metadata(&self) -> Option<LegendMetadata> {
        self.legend_state.as_ref().map(|state| LegendMetadata {
            members: self.legend_members().map(|(i, _)| i).collect(),
            auto_update: state.auto_update,
            interpreter: self.extras.legend_interpreter,
            num_columns: self.extras.legend_num_columns,
            orientation: self.extras.legend_orientation,
            r#box: self.extras.legend_box,
            style: self.extras.legend_style,
        })
    }
    pub(crate) fn restore_legend_metadata(
        &mut self,
        metadata: Option<LegendMetadata>,
    ) -> Result<()> {
        self.extras.legend_interpreter = metadata
            .as_ref()
            .map_or(Default::default(), |m| m.interpreter);
        self.extras.legend_num_columns = metadata
            .as_ref()
            .and_then(|m| m.num_columns.filter(|&n| n >= 1));
        self.extras.legend_orientation = metadata
            .as_ref()
            .map_or(Default::default(), |m| m.orientation);
        self.extras.legend_box = metadata.as_ref().map_or(Default::default(), |m| m.r#box);
        self.extras.legend_style = metadata.as_ref().map_or(Default::default(), |m| m.style);
        self.legend_state = metadata
            .map(|metadata| -> Result<LegendState> {
                if metadata.members.len() > self.series.len() {
                    return Err("Legend exceeds axes membership".into());
                }
                let members = metadata
                    .members
                    .into_iter()
                    .map(|index| {
                        self.series
                            .get(index)
                            .map(|s| s.identity.clone())
                            .ok_or_else(|| "Invalid native legend series index".to_owned())
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(LegendState {
                    identity: ObjectIdentity::default(),
                    members,
                    auto_update: metadata.auto_update,
                })
            })
            .transpose()?;
        self.validate_legend_state()
    }
}
