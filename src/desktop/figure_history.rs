//! Bounded, metadata-only snapshots for native figure editing.
use matrust::{Result, plot::Figure};
use std::collections::{BTreeSet, VecDeque};

pub const MAX_STATES: usize = 32;
pub const SAMPLE_BUDGET: usize = 64 * 1024 * 1024;
pub const METADATA_BUDGET: usize = 8 * 1024 * 1024;

struct State {
    token: u64,
    stamp: u64,
    figure: Figure,
}
pub struct History {
    undo: VecDeque<State>,
    redo: VecDeque<State>,
    current: u64,
    next: u64,
    saved: Option<u64>,
}
impl History {
    pub fn new(clean: bool) -> Self {
        Self {
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            current: 0,
            next: 0,
            saved: clean.then_some(0),
        }
    }
    pub fn counts(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }
    pub fn dirty(&self) -> bool {
        self.saved != Some(self.current)
    }
    pub fn saved(&mut self, exact: bool) {
        self.saved = exact.then_some(self.current);
    }
    pub fn amend_view(&mut self) {
        if self.saved == Some(self.current) {
            self.saved = None;
        }
    }
    fn advance(&mut self) {
        self.next = self.next.wrapping_add(1);
        self.current = self.next;
    }
    pub fn invalidate(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.advance();
        self.saved = None;
    }
    pub fn record(&mut self, previous: &Figure, stamp: u64) {
        self.redo.clear();
        self.undo.push_back(State {
            token: self.current,
            stamp,
            figure: previous.clone(),
        });
        if self.undo.len() > MAX_STATES {
            self.undo.pop_front();
        }
        self.advance();
    }
    pub fn navigate(&mut self, current: &Figure, redo: bool, stamp: u64) -> Result<Figure> {
        let (from, to) = if redo {
            (&mut self.redo, &mut self.undo)
        } else {
            (&mut self.undo, &mut self.redo)
        };
        let state = from.back().ok_or(if redo {
            "No figure edit to redo"
        } else {
            "No figure edit to undo"
        })?;
        state.figure.validate()?;
        let state = from.pop_back().unwrap();
        to.push_back(State {
            token: self.current,
            stamp,
            figure: current.clone(),
        });
        self.current = state.token;
        Ok(state.figure)
    }
    pub fn oldest(&self) -> Option<u64> {
        self.undo
            .front()
            .into_iter()
            .chain(self.redo.front())
            .map(|s| s.stamp)
            .min()
    }
    pub fn discard_oldest(&mut self) {
        if self
            .undo
            .front()
            .is_some_and(|s| self.redo.front().is_none_or(|r| s.stamp <= r.stamp))
        {
            self.undo.pop_front();
        } else {
            self.redo.pop_front();
        }
    }
    /// Account retained capacities, deduplicating shared samples across figures.
    /// This bounds payloads rather than claiming an OS working-set measurement.
    pub fn account(&self, seen: &mut BTreeSet<usize>, samples: &mut usize, metadata: &mut usize) {
        for state in self.undo.iter().chain(&self.redo) {
            let f = &state.figure;
            *metadata += std::mem::size_of::<State>();
            if let Some(layout) = &f.layout {
                *metadata += layout.title.capacity()
                    + layout.others.capacity() * std::mem::size_of::<matrust::plot::Panel>();
            }
            for (_, a) in f.all_axes() {
                *metadata += a.title.capacity()
                    + a.xlabel.capacity()
                    + a.ylabel.capacity()
                    + a.series.capacity() * std::mem::size_of::<matrust::plot::Series>();
                // Series size already includes the new token pointer. Charge
                // its Arc control allocation conservatively per snapshot (even
                // when Bar tokens are shared), plus actual record/member capacities.
                let token_allowance = 3 * std::mem::size_of::<usize>();
                *metadata += a.series.len() * token_allowance
                    + a.baselines.capacity() * std::mem::size_of::<matrust::plot::Baseline>();
                for baseline in &a.baselines {
                    *metadata += token_allowance
                        + baseline.members.capacity()
                            * std::mem::size_of::<matrust::ObjectIdentity>();
                }
                for s in &a.series {
                    *metadata += s.label.capacity();
                    s.points.visit_retained_allocations(|identity, bytes| {
                        if seen.insert(identity) {
                            *samples += bytes;
                        }
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod baseline_account_tests {
    use super::*;
    #[test]
    fn record_member_capacities_and_latent_tokens_are_charged_with_shared_samples() {
        let mut figure = Figure::default();
        figure
            .add(vec![matrust::plot::Series::new(
                vec![[1., 2.]],
                matrust::plot::PlotKind::Stem,
            )])
            .unwrap();
        let before = figure.clone();
        let token = figure
            .axes
            .observe_series_baseline(0)
            .unwrap()
            .identity
            .clone();
        figure
            .axes
            .set_observed_baseline(
                &token,
                None,
                Some(matrust::plot::BaselineStyle::legacy_stem()),
                &std::sync::atomic::AtomicBool::new(false),
            )
            .unwrap();
        let mut history = History::new(true);
        history.record(&before, 1);
        let mut seen = BTreeSet::new();
        let (mut samples, mut metadata) = (0, 0);
        history.account(&mut seen, &mut samples, &mut metadata);
        let samples_before = samples;
        let metadata_before = metadata;
        history.record(&figure, 2);
        let (mut next_samples, mut next_metadata) = (0, 0);
        history.account(&mut BTreeSet::new(), &mut next_samples, &mut next_metadata);
        assert_eq!(samples_before, next_samples);
        let retained = &history.undo.back().unwrap().figure;
        let charged = retained.baselines.capacity()
            * std::mem::size_of::<matrust::plot::Baseline>()
            + retained.baselines[0].members.capacity()
                * std::mem::size_of::<matrust::ObjectIdentity>()
            + 3 * std::mem::size_of::<usize>();
        assert!(next_metadata >= 2 * metadata_before + charged);
        assert!(
            before.series[0]
                .points
                .same_storage(&figure.series[0].points)
        );
    }
}

#[cfg(test)]
mod implicit_bar_history_tests {
    use super::*;
    #[test]
    fn visibility_edit_undo_redo_retains_group_tokens_and_shared_samples() {
        let mut engine = matrust::Engine::default();
        engine
            .execute("b=bar([1 2],[3 4;5 6]);bl=get(b(1),'BaseLine');")
            .unwrap();
        let original = engine.figure.as_ref().unwrap().clone();
        let token = original
            .observe_series_baseline(0)
            .unwrap()
            .identity
            .clone();
        let mut history = History::new(true);
        history.record(&original, 1);
        engine.execute("set(bl,'Visible','off');").unwrap();
        let hidden = engine.figure.unwrap();
        assert!(!hidden.observe_baseline(&token).unwrap().style.visible);
        let restored = history.navigate(&hidden, false, 2).unwrap();
        assert!(restored.same_snapshot(&original));
        assert!(restored.baselines.is_empty());
        assert!(restored.observe_baseline(&token).unwrap().style.visible);
        let redone = history.navigate(&restored, true, 3).unwrap();
        assert!(redone.same_snapshot(&hidden));
        for (a, b) in original.series.iter().zip(&redone.series) {
            assert!(a.points.same_storage(&b.points));
        }
        let mut sample_bytes = 0;
        let mut metadata = 0;
        history.account(&mut BTreeSet::new(), &mut sample_bytes, &mut metadata);
        assert!(metadata >= original.series.len() * std::mem::size_of::<matrust::plot::Series>());
        assert!(sample_bytes > 0);
    }
}
