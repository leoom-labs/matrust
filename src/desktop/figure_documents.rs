use super::figure_history::{History, METADATA_BUDGET, SAMPLE_BUDGET};
use super::{Job, perform};
use matrust::{
    Engine, Execution, Result,
    document::same_path,
    files::figure_version::{self, DiskVersion, SaveOutcome, Version},
    plot::Figure,
};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FigureStatus {
    pub path: Option<PathBuf>,
    pub source_version: Option<Version>,
    pub dirty: bool,
    pub revision: u64,
}

#[cfg(test)]
mod history_tests {
    use super::super::FigureEdit;
    use super::*;
    fn run(d: &mut Documents, e: &mut Engine, s: &str) {
        d.perform(e, Job::Script(s.into())).0.unwrap();
    }
    fn edit(d: &mut Documents, e: &mut Engine, change: impl FnOnce(&mut Figure)) {
        let original = e.figure.clone().unwrap();
        let mut updated = original.clone();
        change(&mut updated);
        d.perform(
            e,
            Job::EditFigure(Box::new(FigureEdit {
                view_before: None,
                id: e.current_figure,
                original,
                updated,
            })),
        )
        .0
        .unwrap();
    }
    #[test]
    fn marker_styles_history_save_reopen_retains_metadata_without_sample_copies() {
        let (mut d, mut e) = (Documents::default(), Engine::default());
        run(&mut d, &mut e, "plot([1 2 3],'ro-');");
        let original = e.figure.clone().unwrap();
        edit(&mut d, &mut e, |f| {
            f.series[0].marker_shape = Some(matrust::plot::MarkerShape::Square);
            f.series[0].marker_size = Some(12.);
        });
        d.perform(&mut e, Job::UndoFigure(1, None)).0.unwrap();
        assert!(original.same_snapshot(e.figure.as_ref().unwrap()));
        d.perform(&mut e, Job::RedoFigure(1, None)).0.unwrap();
        assert_eq!(e.figure.as_ref().unwrap().series[0].marker_size, Some(12.));
        assert!(
            (original.series[0].points).same_storage(&e.figure.as_ref().unwrap().series[0].points)
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("marker.mfig");
        d.perform(&mut e, Job::SaveFigure(path.clone(), None))
            .0
            .unwrap();
        assert!(!d.statuses()[&1].dirty);
        d.perform(&mut e, Job::OpenFigure(path)).0.unwrap();
        let s = &e.figure.as_ref().unwrap().series[0];
        assert_eq!(s.marker_shape, Some(matrust::plot::MarkerShape::Square));
        assert_eq!(s.marker_size, Some(12.));
    }
    #[test]
    fn line_width_history_save_reopen_preserves_metadata_and_shared_samples() {
        let (mut d, mut e) = (Documents::default(), Engine::default());
        run(&mut d, &mut e, "plot([1 2 3]);");
        let samples = e.figure.as_ref().unwrap().series[0].points.clone();
        edit(&mut d, &mut e, |f| f.series[0].line_width = Some(4.));
        assert_eq!(d.history_counts()[&1], (1, 0));
        d.perform(&mut e, Job::UndoFigure(1, None)).0.unwrap();
        assert_eq!(e.figure.as_ref().unwrap().series[0].line_width, None);
        d.perform(&mut e, Job::RedoFigure(1, None)).0.unwrap();
        assert_eq!(e.figure.as_ref().unwrap().series[0].line_width, Some(4.));
        assert!((samples).same_storage(&e.figure.as_ref().unwrap().series[0].points));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("width.mfig");
        d.perform(&mut e, Job::SaveFigure(path.clone(), None))
            .0
            .unwrap();
        assert!(!d.statuses()[&1].dirty);
        d.perform(&mut e, Job::OpenFigure(path)).0.unwrap();
        assert_eq!(e.figure.as_ref().unwrap().series[0].line_width, Some(4.));
    }
    #[test]
    fn history_captures_gui_manual_views_and_rejects_invalid_view_atomically() {
        let (mut d, mut e) = (Documents::default(), Engine::default());
        run(&mut d, &mut e, "plot(1:100);");
        let original = e.figure.clone().unwrap();
        let mut updated = original.clone();
        updated.title = "Edited".into();
        let mut edit = FigureEdit {
            view_before: None,
            id: 1,
            original,
            updated,
        };
        edit.preserve_manual_view(Some([20., 40., 10., 80.]), [false, true]);
        d.perform(&mut e, Job::EditFigure(Box::new(edit)))
            .0
            .unwrap();
        let current = e.figure.clone().unwrap();
        let counts = d.history_counts();
        assert!(
            d.perform(
                &mut e,
                Job::UndoFigure(1, Some(([40., 20., 0., 1.], [false, true])))
            )
            .0
            .is_err()
        );
        assert!(current.same_snapshot(e.figure.as_ref().unwrap()));
        assert_eq!(d.history_counts(), counts);
        d.perform(
            &mut e,
            Job::UndoFigure(1, Some(([30., 60., 10., 90.], [false, false]))),
        )
        .0
        .unwrap();
        let restored = e.figure.as_ref().unwrap();
        assert_eq!(restored.title, "");
        assert_eq!(restored.xlim, Some([20., 40.]));
        assert!(restored.ylim.is_none());
        d.perform(&mut e, Job::RedoFigure(1, None)).0.unwrap();
        let redone = e.figure.as_ref().unwrap();
        assert_eq!(redone.title, "Edited");
        assert_eq!(redone.xlim, Some([30., 60.]));
        assert_eq!(redone.ylim, Some([10., 90.]));
        assert!((current.series[0].points).same_storage(&redone.series[0].points));
    }
    #[test]
    fn undo_redo_restore_saved_layout_samples_and_branch_correctly() {
        let (mut d, mut e) = (Documents::default(), Engine::default());
        run(&mut d, &mut e, include_str!("../../examples/fig_layouts.m"));
        let original = e.figure.clone().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("saved.mfig");
        d.perform(&mut e, Job::SaveFigure(path.clone(), None))
            .0
            .unwrap();
        edit(&mut d, &mut e, |f| {
            f.reposition_axes([0.15, 0.1, 0.7, 0.3]).unwrap();
            f.title = "Edited".into();
            f.legend_location = matrust::plot::LegendLocation::Southwest;
            f.series[0].visible = false;
        });
        let updated = e.figure.clone().unwrap();
        assert_eq!(d.history_counts()[&1], (1, 0));
        assert!(d.statuses()[&1].dirty);
        d.perform(&mut e, Job::UndoFigure(1, None)).0.unwrap();
        assert!(original.same_snapshot(e.figure.as_ref().unwrap()));
        assert!(!d.statuses()[&1].dirty);
        d.perform(&mut e, Job::RedoFigure(1, None)).0.unwrap();
        assert!(updated.same_snapshot(e.figure.as_ref().unwrap()));
        d.perform(&mut e, Job::SaveFigure(path.clone(), None))
            .0
            .unwrap();
        d.perform(&mut e, Job::UndoFigure(1, None)).0.unwrap();
        assert!(d.statuses()[&1].dirty);
        d.perform(&mut e, Job::RedoFigure(1, None)).0.unwrap();
        assert!(!d.statuses()[&1].dirty);
        d.perform(&mut e, Job::UndoFigure(1, None)).0.unwrap();
        let mut view = original.clone();
        view.xlim = Some([0.2, 0.8]);
        d.perform(
            &mut e,
            Job::ViewFigure(Box::new(FigureEdit {
                view_before: None,
                id: 1,
                original: original.clone(),
                updated: view,
            })),
        )
        .0
        .unwrap();
        assert_eq!(d.history_counts()[&1], (0, 1));
        edit(&mut d, &mut e, |f| f.title = "Branch".into());
        assert_eq!(d.history_counts()[&1], (1, 0));
        assert!(d.perform(&mut e, Job::RedoFigure(1, None)).0.is_err());
        let before = e.figure.clone().unwrap();
        assert!(
            d.perform(
                &mut e,
                Job::EditFigure(Box::new(FigureEdit {
                    view_before: None,
                    id: 1,
                    original,
                    updated
                }))
            )
            .0
            .is_err()
        );
        assert!(before.same_snapshot(e.figure.as_ref().unwrap()));
        assert_eq!(d.history_counts()[&1], (1, 0));
        let saved = matrust::files::load_figure(&path).unwrap();
        assert_eq!(saved.title, "Edited");
        assert_eq!(
            saved.legend_location,
            matrust::plot::LegendLocation::Southwest
        );
        assert!(
            (before.series[0].points).same_storage(&e.figure.as_ref().unwrap().series[0].points)
        );
    }
    #[test]
    fn switching_preserves_histories_but_scripts_reload_and_close_release_them() {
        let (mut d, mut e) = (Documents::default(), Engine::default());
        run(&mut d, &mut e, "figure(1);plot([1 2]);");
        let weak = (e.figure.as_ref().unwrap().series[0].points).downgrade();
        edit(&mut d, &mut e, |f| f.title = "First edit".into());
        run(&mut d, &mut e, "figure(2);plot([3 4]);");
        edit(&mut d, &mut e, |f| f.title = "Second edit".into());
        run(&mut d, &mut e, "figure(1);");
        assert_eq!(d.history_counts()[&1], (1, 0));
        assert_eq!(d.history_counts()[&2], (1, 0));
        assert!(d.perform(&mut e, Job::UndoFigure(2, None)).0.is_err());
        assert!(
            d.perform(&mut e, Job::Script("unknown_function(1)".into()))
                .0
                .is_err()
        );
        assert_eq!(d.history_counts()[&1], (1, 0));
        assert!(
            d.perform(
                &mut e,
                Job::Script("plot([8 9]);unknown_function(1)".into())
            )
            .0
            .is_err()
        );
        assert_eq!(d.history_counts()[&1], (0, 0));
        assert!(weak.upgrade().is_none());
        assert_eq!(d.history_counts()[&2], (1, 0));
        run(&mut d, &mut e, "figure(2);");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("saved.mfig");
        d.perform(&mut e, Job::SaveFigure(path.clone(), None))
            .0
            .unwrap();
        d.perform(&mut e, Job::ReloadFigure(2, path)).0.unwrap();
        assert_eq!(d.history_counts()[&2], (0, 0));
        edit(&mut d, &mut e, |f| f.title = "Again".into());
        run(&mut d, &mut e, "close(2);");
        assert!(!d.history_counts().contains_key(&2));
    }
    #[test]
    fn history_caps_steps_and_global_retained_samples_and_metadata() {
        use matrust::plot::{PlotKind, Series};
        let (mut d, mut e) = (Documents::default(), Engine::default());
        run(&mut d, &mut e, "plot([1 2]);");
        for k in 0..40 {
            edit(&mut d, &mut e, |f| f.title = format!("Edit {k}"));
        }
        assert_eq!(d.history_counts()[&1], (32, 0));
        for _ in 0..32 {
            d.perform(&mut e, Job::UndoFigure(1, None)).0.unwrap();
        }
        assert!(d.perform(&mut e, Job::UndoFigure(1, None)).0.is_err());
        assert_eq!(e.figure.as_ref().unwrap().title, "Edit 7");
        run(&mut d, &mut e, "close all;");
        for id in 1..=5 {
            e.select_figure(id).unwrap();
            let mut f = Figure::default();
            f.add(vec![Series::new(vec![[1., 2.]; 1_000_000], PlotKind::Line)])
                .unwrap();
            e.figure = Some(f);
            run(&mut d, &mut e, "1;");
            edit(&mut d, &mut e, |f| f.title = "Keep".into());
        }
        assert_eq!(d.history_counts()[&1], (0, 0));
        for id in 2..=5 {
            assert_eq!(d.history_counts()[&id], (1, 0));
        }
        let mut seen = std::collections::BTreeSet::new();
        let (mut samples, mut metadata) = (0, 0);
        for entry in d.0.values() {
            entry
                .history
                .account(&mut seen, &mut samples, &mut metadata);
        }
        assert!(samples <= SAMPLE_BUDGET && samples > 48 * 1024 * 1024);
        run(&mut d, &mut e, "close all;");
        e.select_figure(1).unwrap();
        let mut f = Figure::default();
        f.add(
            (0..64)
                .map(|_| {
                    let mut s = Series::new(vec![], PlotKind::Line);
                    s.label = "a".repeat(4096);
                    s
                })
                .collect(),
        )
        .unwrap();
        f.xlabel = "x".repeat(4096);
        f.ylabel = "y".repeat(4096);
        e.figure = Some(f);
        run(&mut d, &mut e, "1;");
        for k in 0..40 {
            edit(&mut d, &mut e, |f| {
                f.title = format!("{k}{}", "t".repeat(4090))
            });
        }
        let count = d.history_counts()[&1].0;
        assert!(count > 0 && count < 32);
        let mut seen = std::collections::BTreeSet::new();
        let (mut samples, mut metadata) = (0, 0);
        d.0[&1]
            .history
            .account(&mut seen, &mut samples, &mut metadata);
        assert!(metadata <= METADATA_BUDGET);
    }
}

#[derive(Clone, Debug)]
pub struct FigureConflict {
    pub reload_requested: bool,
    pub id: u32,
    pub path: PathBuf,
    pub view: Option<[f64; 4]>,
    pub observed: DiskVersion,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn script(d: &mut Documents, e: &mut Engine, s: &str) {
        d.perform(e, Job::Script(s.into())).0.unwrap();
    }
    #[test]
    fn relative_figure_paths_are_resolved_before_worker_dispatch() {
        let dir = tempfile::tempdir_in(".").unwrap();
        let relative = dir.path().join("relative.mfig");
        let job = Job::SaveFigure(relative.clone(), None)
            .absolute_figure_paths()
            .unwrap();
        let Job::SaveFigure(path, _) = &job else {
            panic!("Wrong job")
        };
        assert!(path.is_absolute());
        let expected = std::path::absolute(relative).unwrap();
        assert_eq!(*path, expected);
        let mut d = Documents::default();
        let mut e = Engine::default();
        script(&mut d, &mut e, "plot([1 2 3]);");
        d.perform(&mut e, job).0.unwrap();
        assert_eq!(d.statuses()[&1].path, Some(expected));
    }
    #[test]
    fn identity_dirty_state_and_failed_saves_follow_worker_results() {
        let mut d = Documents::default();
        let mut e = Engine::default();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("original.mfig");
        script(&mut d, &mut e, "plot(1:10000,sin(1:10000));");
        assert!(d.statuses()[&1].dirty);
        let points = e.figure.as_ref().unwrap().series[0].points.clone();
        d.perform(&mut e, Job::SaveFigure(path.clone(), None))
            .0
            .unwrap();
        assert!(!d.statuses()[&1].dirty);
        script(&mut d, &mut e, "a=1;");
        assert!(!d.statuses()[&1].dirty);
        assert!((points).same_storage(&d.0[&1].latest.series[0].points));
        assert!(
            d.perform(
                &mut e,
                Job::Script("title('Changed'); bad=unknown_name;".into())
            )
            .0
            .is_err()
        );
        assert!(d.statuses()[&1].dirty);
        let bad = dir.path().join("missing").join("failed.mfig");
        assert!(d.perform(&mut e, Job::SaveFigure(bad, None)).0.is_err());
        assert_eq!(d.statuses()[&1].path, Some(path.clone()));
        assert!(d.statuses()[&1].dirty);
        d.perform(
            &mut e,
            Job::ExportFigure(dir.path().join("image.png"), None),
        )
        .0
        .unwrap();
        assert!(d.statuses()[&1].dirty);
        let saved = dir.path().join("new.mfig");
        d.perform(&mut e, Job::SaveFigure(saved.clone(), None))
            .0
            .unwrap();
        assert_eq!(d.statuses()[&1].path, Some(saved));
        assert!(!d.statuses()[&1].dirty);
        script(&mut d, &mut e, "plot([1 2 3]);");
        // Only the test keeps the replaced buffer alive; tracking releases it.
        assert_eq!(points.strong_count(), 1);
        let old_revision = d.statuses()[&1].revision;
        script(&mut d, &mut e, "close(1); figure(1); plot([3 2 1]);");
        assert!(d.statuses()[&1].path.is_none());
        assert!(d.statuses()[&1].revision > old_revision);
        script(&mut d, &mut e, "close('all');");
        assert!(d.statuses().is_empty());
    }
    #[test]
    fn opening_preserves_figures_and_background_saves_do_not_change_selection() {
        let mut d = Documents::default();
        let mut e = Engine::default();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plot.mfig");
        script(&mut d, &mut e, "plot([1 2 3]);");
        d.perform(&mut e, Job::SaveFigure(path.clone(), None))
            .0
            .unwrap();
        script(&mut d, &mut e, "title('Unsaved original');");
        d.perform(&mut e, Job::OpenFigure(path.clone())).0.unwrap();
        assert_eq!(e.current_figure, 2);
        assert!(d.statuses()[&1].dirty);
        assert!(!d.statuses()[&2].dirty);
        assert_eq!(d.statuses()[&2].path, Some(path));
        let copy = dir.path().join("other.mfig");
        d.perform(
            &mut e,
            Job::SaveFigureById(1, copy.clone(), Some([1., 2., -1., 4.])),
        )
        .0
        .unwrap();
        assert_eq!(e.current_figure, 2);
        let f = matrust::files::load_figure(&copy).unwrap();
        assert_eq!(f.title, "Unsaved original");
        assert_eq!(f.xlim, Some([1., 2.]));
        for id in 3..=16 {
            script(&mut d, &mut e, &format!("figure({id});"));
        }
        let selected = e.current_figure;
        assert!(d.perform(&mut e, Job::OpenFigure(copy)).0.is_err());
        assert_eq!(e.current_figure, selected);
        assert_eq!(e.figure_ids().len(), 16);
    }
}
struct Entry {
    status: FigureStatus,
    latest: Figure,
    history: History,
}
#[derive(Default)]
pub struct Documents(
    BTreeMap<u32, Entry>,
    u64,
    Option<FigureConflict>,
    Option<u32>,
);
impl Documents {
    pub fn history_counts(&self) -> BTreeMap<u32, (usize, usize)> {
        self.0
            .iter()
            .map(|(&id, e)| (id, e.history.counts()))
            .collect()
    }
    fn trim_history(&mut self) {
        loop {
            let (mut samples, mut metadata) = (0, 0);
            let mut seen = std::collections::BTreeSet::new();
            for entry in self.0.values() {
                entry
                    .history
                    .account(&mut seen, &mut samples, &mut metadata);
            }
            if samples <= SAMPLE_BUDGET && metadata <= METADATA_BUDGET {
                break;
            }
            let Some(id) = self
                .0
                .iter()
                .filter_map(|(&id, e)| e.history.oldest().map(|stamp| (stamp, id)))
                .min()
                .map(|(_, id)| id)
            else {
                break;
            };
            self.0.get_mut(&id).unwrap().history.discard_oldest();
        }
    }
    fn navigate(
        &mut self,
        engine: &mut Engine,
        id: u32,
        redo: bool,
        view: Option<super::HistoryView>,
    ) -> Result<Execution> {
        engine.diagnostic = None;
        engine.failed_output = Execution::default();
        if id != engine.current_figure {
            return Err("Select the figure before undo or redo".into());
        }
        let figure = engine.figure.as_ref().ok_or("Figure no longer exists")?;
        let entry = self.0.get_mut(&id).ok_or("No figure edit history")?;
        if !entry.latest.same_snapshot(figure) {
            entry.history.invalidate();
            return Err("Figure changed outside its edit history".into());
        }
        let counts = entry.history.counts();
        if if redo { counts.1 == 0 } else { counts.0 == 0 } {
            return Err(if redo {
                "No figure edit to redo"
            } else {
                "No figure edit to undo"
            }
            .into());
        }
        let current = super::with_history_view(figure, view)?;
        if !current.same_snapshot(figure) {
            entry.history.amend_view();
        }
        engine.figure = Some(
            entry
                .history
                .navigate(&current, redo, self.1.wrapping_add(1))?,
        );
        Ok(Execution {
            output: vec![format!(
                "{} edit in figure {id}",
                if redo { "Redid" } else { "Undid" }
            )],
            clear_console: false,
        })
    }
    pub fn take_conflict(&mut self) -> Option<FigureConflict> {
        self.2.take()
    }
    pub fn take_reloaded(&mut self) -> Option<u32> {
        self.3.take()
    }
    fn save(
        &mut self,
        engine: &Engine,
        id: u32,
        path: &std::path::Path,
        view: Option<[f64; 4]>,
        confirmed: Option<DiskVersion>,
    ) -> Result<Version> {
        let mut figure = engine
            .figures()
            .find(|(n, _)| *n == id)
            .map(|(_, f)| f.clone())
            .ok_or("Figure no longer exists")?;
        if let Some([a, b, c, d]) = view {
            figure.xlim = Some([a, b]);
            figure.ylim = Some([c, d]);
        }
        let expected = confirmed.or_else(|| {
            self.0
                .get(&id)
                .filter(|e| e.status.path.as_ref().is_some_and(|p| same_path(p, path)))
                .map(|e| {
                    e.status
                        .source_version
                        .map_or(DiskVersion::Unknown, DiskVersion::Present)
                })
        });
        match figure_version::save_with_theme(
            path,
            &figure,
            expected,
            engine.graphics_theme_context(),
            &engine.cancellation(),
        )? {
            SaveOutcome::Saved(version) => Ok(version),
            SaveOutcome::Conflict(observed) => {
                self.2 = Some(FigureConflict {
                    reload_requested: false,
                    id,
                    path: path.to_owned(),
                    view,
                    observed,
                });
                Err(if confirmed.is_some() {
                    "The figure changed again. Review the latest disk version before overwriting."
                } else {
                    "Figure changed or was deleted on disk. Review the disk version before saving."
                }
                .into())
            }
        }
    }
    fn open(engine: &mut Engine, path: &std::path::Path, reload: Option<u32>) -> Result<Version> {
        let ids = engine.figure_ids();
        if let Some(id) = reload {
            if !ids.contains(&id) {
                return Err("Figure no longer exists".into());
            }
        } else if ids.len() >= 16 {
            return Err("At most 16 figures can be open; close unused figures".into());
        }
        let (figure, version) = figure_version::load(path, &engine.cancellation())?;
        if let Some(id) = reload {
            let figures = engine
                .figures()
                .map(|(n, f)| (n, if n == id { figure.clone() } else { f.clone() }))
                .collect();
            engine.restore_figures(engine.current_figure, figures)?;
        } else {
            let ids = engine.figure_ids();
            let id = (1..=17).find(|id| !ids.contains(id)).unwrap();
            engine.select_figure(id)?;
            engine.figure = Some(figure);
        }
        Ok(version)
    }

    pub fn restore(
        &mut self,
        engine: &mut Engine,
        recovered: matrust::session::figures::FigureSession,
    ) -> Result<()> {
        recovered.validate()?;
        engine.restore_figures(
            recovered.current,
            recovered
                .figures
                .iter()
                .map(|d| (d.id, d.figure.clone()))
                .collect(),
        )?;
        self.0.clear();
        for doc in recovered.figures {
            self.1 = self.1.wrapping_add(1);
            self.0.insert(
                doc.id,
                Entry {
                    history: History::new(!doc.dirty),
                    status: FigureStatus {
                        path: doc.path,
                        source_version: doc.source_version,
                        dirty: doc.dirty,
                        revision: self.1,
                    },
                    latest: doc.figure,
                },
            );
        }
        Ok(())
    }

    pub fn statuses(&self) -> BTreeMap<u32, FigureStatus> {
        self.0
            .iter()
            .map(|(&id, e)| (id, e.status.clone()))
            .collect()
    }
    pub fn perform(&mut self, engine: &mut Engine, job: Job) -> (Result<Execution>, Option<u32>) {
        engine.take_figure_file_events();
        let job = if let Job::ScriptViewContext(job, views) = job {
            if !matches!(
                &*job,
                Job::ScriptInFolder(..) | Job::DocumentInFolder(..) | Job::SelectionInFolder(..)
            ) {
                return (Err("Figure views require a script job".into()), None);
            }
            if let Err(error) = self.sync_script_views(engine, views) {
                return (Err(error), None);
            }
            *job
        } else {
            job
        };
        let job = match job.absolute_figure_paths() {
            Ok(job) => job,
            Err(e) => return (Err(e), None),
        };
        self.2 = None;
        self.3 = None;
        engine.take_closed_figures();
        let job = if let Job::OpenFigureOrSelect(path) = job {
            if engine
                .cancellation()
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                return (Err("Figure opening cancelled".into()), None);
            }
            let ids = engine.figure_ids();
            let matches = |id: &u32, entry: &Entry| {
                ids.contains(id)
                    && entry
                        .status
                        .path
                        .as_ref()
                        .is_some_and(|saved| same_path(saved, &path))
            };
            let existing = self
                .0
                .get(&engine.current_figure)
                .filter(|entry| matches(&engine.current_figure, entry))
                .map(|_| engine.current_figure)
                .or_else(|| {
                    self.0
                        .iter()
                        .find(|(id, entry)| matches(id, entry))
                        .map(|(&id, _)| id)
                });
            if engine
                .cancellation()
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                return (Err("Figure opening cancelled".into()), None);
            }
            if let Some(id) = existing {
                engine.diagnostic = None;
                engine.failed_output = Execution::default();
                return (engine.select_figure(id).map(|_| Execution {
                    output: vec![format!("Selected open figure {id}; local edits retained. Use Reload figure to read disk changes, or Open separate disk copy to compare.")],
                    clear_console: false,
                }), None);
            }
            Job::OpenFigure(path)
        } else {
            job
        };
        let edit_id = if let Job::EditFigure(edit) = &job {
            self.0
                .get(&edit.id)
                .filter(|e| e.latest.same_snapshot(&edit.original))
                .map(|_| edit.id)
        } else {
            None
        };
        let navigation = match &job {
            Job::UndoFigure(id, view) => Some((*id, false, *view)),
            Job::RedoFigure(id, view) => Some((*id, true, *view)),
            _ => None,
        };
        let view_id = if let Job::ViewFigure(edit) = &job {
            self.0
                .get(&edit.id)
                .filter(|e| e.latest.same_snapshot(&edit.original))
                .map(|_| edit.id)
        } else {
            None
        };
        let before_view = if let Job::EditFigure(edit) = &job {
            edit.view_before
        } else {
            None
        };
        let open = match &job {
            Job::OpenFigure(path) | Job::ReloadFigure(_, path) => Some(path.clone()),
            _ => None,
        };
        let reload = match &job {
            Job::ReloadFigure(id, _) => Some(*id),
            _ => None,
        };
        let save = match &job {
            Job::SaveFigure(path, view) => Some((engine.current_figure, path.clone(), *view, None)),
            Job::SaveFigureById(id, path, view) => Some((*id, path.clone(), *view, None)),
            Job::SaveFigureConfirmed(id, path, view, expected) => {
                Some((*id, path.clone(), *view, Some(*expected)))
            }
            _ => None,
        };
        let mut source_version = None;
        let result = if let Some((id, path, view, confirmed)) = &save {
            engine.diagnostic = None;
            engine.failed_output = Execution::default();
            self.save(engine, *id, path, *view, *confirmed).map(|v| {
                source_version = Some(v);
                Execution {
                    output: vec![format!(
                        "Saved editable figure {}",
                        crate::path_label::display(path)
                    )],
                    clear_console: false,
                }
            })
        } else if let Some(path) = &open {
            engine.diagnostic = None;
            engine.failed_output = Execution::default();
            Self::open(engine,path,reload).map(|v| {
                source_version=Some(v);
                Execution { output:vec![format!("{} figure {}{}",if reload.is_some(){"Reloaded"}else{"Opened"},crate::path_label::display(path),if path.extension().is_some_and(|s|s.eq_ignore_ascii_case("fig")){" (supported MATLAB 2-D data/styles imported; fonts and unspecified marker sizes use MatRust defaults; normalized axes positions are retained; callbacks are not executed)"}else{""})], clear_console:false }
            })
        } else if let Some((id, redo, view)) = navigation {
            self.navigate(engine, id, redo, view)
        } else {
            perform(engine, job)
        };
        for id in engine.take_closed_figures() {
            self.0.remove(&id);
        }
        // Reconcile even after an error: earlier script statements may succeed.
        let ids = engine.figure_ids();
        self.0.retain(|id, _| ids.contains(id));
        for (id, figure) in engine.figures() {
            match self.0.get_mut(&id) {
                Some(entry) if !entry.latest.same_snapshot(figure) => {
                    self.1 = self.1.wrapping_add(1);
                    if result.is_ok() && edit_id == Some(id) {
                        // perform validated this view before publishing the edit.
                        let previous =
                            super::with_history_view(&entry.latest, before_view).unwrap();
                        if !previous.same_snapshot(&entry.latest) {
                            entry.history.amend_view();
                        }
                        entry.history.record(&previous, self.1);
                    } else if result.is_ok() && view_id == Some(id) {
                        entry.history.amend_view();
                    } else if !(result.is_ok() && navigation.is_some_and(|(n, _, _)| n == id)) {
                        entry.history.invalidate();
                    }
                    entry.status.dirty = entry.history.dirty();
                    entry.status.revision = self.1;
                    // The document baseline follows current buffers. Only the
                    // separately bounded edit history retains previous states.
                    entry.latest = figure.clone();
                }
                None => {
                    self.1 = self.1.wrapping_add(1);
                    self.0.insert(
                        id,
                        Entry {
                            history: History::new(figure.same_snapshot(&Figure::default())),
                            status: FigureStatus {
                                path: None,
                                source_version: None,
                                dirty: !figure.same_snapshot(&Figure::default()),
                                revision: self.1,
                            },
                            latest: figure.clone(),
                        },
                    );
                }
                _ => {}
            }
        }
        if result.is_ok()
            && let Some((id, _, _)) = navigation
            && let Some(entry) = self.0.get_mut(&id)
        {
            entry.status.dirty = entry.history.dirty();
        }
        let mut saved = None;
        if result.is_ok() {
            if let Some(path) = open {
                let entry = self
                    .0
                    .get_mut(&reload.unwrap_or(engine.current_figure))
                    .unwrap();
                entry.status.path = Some(path);
                entry.status.source_version = source_version;
                entry.status.dirty = false;
                entry.history = History::new(true);
            }
            if let Some((id, path, view, _)) = save {
                let entry = self.0.get_mut(&id).unwrap();
                entry.status.path = Some(path);
                entry.status.source_version = source_version;
                entry.status.dirty = false;
                let exact = view.is_none_or(|v| {
                    entry.latest.xlim == Some([v[0], v[1]])
                        && entry.latest.ylim == Some([v[2], v[3]])
                });
                entry.history.saved(exact);
                saved = Some(id);
            }
        }
        if result.is_ok() {
            self.3 = reload;
        }
        // Successful file statements remain real even if a later statement fails.
        for event in engine.take_figure_file_events() {
            if let Some(entry) = self.0.get_mut(&event.id) {
                let clean = event.matches(&entry.latest);
                entry.status.path = Some(event.path);
                entry.status.source_version = Some(event.version);
                if event.opened {
                    entry.history = History::new(clean);
                } else {
                    entry.history.saved(clean);
                }
                entry.status.dirty = !clean;
                self.1 = self.1.wrapping_add(1);
                entry.status.revision = self.1;
            }
        }
        self.trim_history();
        (result, saved)
    }

    fn sync_script_views(
        &mut self,
        engine: &mut Engine,
        views: Vec<(u32, super::HistoryView)>,
    ) -> Result<()> {
        if engine
            .cancellation()
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Err("Script cancelled".into());
        }
        let count = views.len();
        let views: BTreeMap<_, _> = views.into_iter().collect();
        let ids = engine.figure_ids();
        if count > 16 || count != views.len() || views.keys().any(|id| !ids.contains(id)) {
            return Err("Invalid script figure view context".into());
        }
        let figures = engine
            .figures()
            .map(|(id, figure)| {
                let view = views.get(&id).copied();
                let mut figure = super::with_history_view(figure, view)?;
                if let Some((_, auto)) = view {
                    if auto[0] {
                        figure.xlim = None;
                    }
                    if auto[1] {
                        figure.ylim = None;
                    }
                }
                Ok((id, figure))
            })
            .collect::<Result<Vec<_>>>()?;
        engine.restore_figures(engine.current_figure, figures)?;
        for (id, figure) in engine.figures() {
            if let Some(entry) = self.0.get_mut(&id)
                && !entry.latest.same_snapshot(figure)
            {
                // This synchronizes an existing GUI view, without adding an
                // undo step or invalidating earlier edit history.
                entry.history.amend_view();
                entry.status.dirty = entry.history.dirty();
                entry.latest = figure.clone();
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "figure_reopen_worker_tests.rs"]
mod reopen_tests;

#[cfg(test)]
#[path = "script_figure_document_tests.rs"]
mod script_file_tests;
