//! Bounded ordered legend membership. Tokens retain no graph or sample buffers.
use super::{Axes, MAX_SERIES, Series};
use crate::{ObjectIdentity, Result};

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
}

impl Axes {
    /// Includes empty labels for String queries and exact persistence. Renderer
    /// callers use legend_entries, which omits current-MATLAB empty labels.
    pub fn legend_members(&self) -> impl Iterator<Item = (usize, &Series)> {
        let count = self
            .legend_state
            .as_ref()
            .map_or(self.series.len(), |s| s.members.len())
            .min(MAX_SERIES);
        (0..count).filter_map(move |index| {
            if let Some(state) = &self.legend_state {
                let token = &state.members[index];
                self.series
                    .iter()
                    .take(MAX_SERIES)
                    .enumerate()
                    .find(|(_, s)| s.identity.same_object(token))
            } else {
                self.series.get(index).map(|s| (index, s))
            }
        })
    }
    pub fn legend_entries(&self) -> impl Iterator<Item = (usize, &Series)> {
        self.legend_members().filter(|(_, s)| !s.label.is_empty())
    }
    pub(super) fn validate_legend_state(&self) -> Result<()> {
        if let Some(state) = &self.legend_state {
            if state.members.len() > MAX_SERIES {
                return Err("Legend exceeds 64 members".into());
            }
            for (i, token) in state.members.iter().enumerate() {
                if !self.series.iter().any(|s| s.identity.same_object(token)) {
                    return Err("Legend member does not belong to its axes".into());
                }
                if state.members[..i].iter().any(|old| old.same_object(token)) {
                    return Err("Legend contains a duplicate series".into());
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
        })
    }
    pub(crate) fn restore_legend_metadata(
        &mut self,
        metadata: Option<LegendMetadata>,
    ) -> Result<()> {
        self.legend_state = metadata
            .map(|metadata| -> Result<LegendState> {
                if metadata.members.len() > MAX_SERIES {
                    return Err("Legend exceeds 64 members".into());
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
