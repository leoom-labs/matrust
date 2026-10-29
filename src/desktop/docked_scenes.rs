//! Bounded prepared scenes. The containing Registry owns scheduling and actions.
use super::*;
use crate::figure_geometry::scene;

const MAX_SCENES: usize = 4;
struct Scene {
    key: Key,
    revision: u64,
    source: Figure,
    identity: scene::SurfaceIdentity,
    geometry: Option<Geometry>,
    requested: bool,
    visible: bool,
    preparing: bool,
    reset: bool,
    touched: u64,
    notice: String,
    frame: Option<u64>,
    view: Option<ViewRecord>,
    command: Option<Envelope>,
}
#[derive(Default)]
pub(super) struct Coordinator {
    scenes: Vec<Scene>,
}
impl Coordinator {
    /// Reordering transfers ownership, never clones display vectors or replaces IDs.
    /// Validate the complete selection before retiring any existing scene.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn configure(
        &mut self,
        app: &Workspace,
        ctx: &egui::Context,
        keys: &[Key],
        detached: &BTreeMap<u32, Arc<Mutex<Child>>>,
    ) -> matrust::Result<()> {
        if keys.len() > MAX_SCENES {
            return Err("At most four docked scenes are supported".into());
        }
        for (index, key) in keys.iter().enumerate() {
            if !live(app, key)
                || revision(app, key.number).is_none()
                || detached.contains_key(&key.number)
                || keys[..index].iter().any(|old| old.number == key.number)
            {
                return Err("Choose distinct live figures which are not detached".into());
            }
        }
        // Allocate identities first: even exhaustion must leave existing ownership intact.
        let mut additions = Vec::new();
        for key in keys {
            if !self.scenes.iter().any(|old| old.key.same(key)) {
                additions.push(Scene {
                    key: key.clone(),
                    revision: revision(app, key.number).unwrap(),
                    source: app.all_figures[&key.number].clone(),
                    identity: scene::SurfaceIdentity::new(key.clone())?,
                    geometry: None,
                    requested: true,
                    visible: true,
                    preparing: false,
                    reset: true,
                    touched: 0,
                    notice: String::new(),
                    frame: None,
                    view: None,
                    command: None,
                });
            }
        }
        let mut old = std::mem::take(&mut self.scenes);
        old.append(&mut additions);
        for key in keys {
            let index = old.iter().position(|s| s.key.same(key)).unwrap();
            self.scenes.push(old.remove(index));
        }
        for retired in old {
            let _ = retired.identity.forget(ctx, &retired.source);
        }
        self.sync(app, ctx, detached);
        Ok(())
    }
    pub fn sync(
        &mut self,
        app: &Workspace,
        ctx: &egui::Context,
        detached: &BTreeMap<u32, Arc<Mutex<Child>>>,
    ) {
        self.scenes.retain_mut(|state| {
            if !live(app, &state.key) || detached.contains_key(&state.key.number) {
                let _ = state.identity.forget(ctx, &state.source);
                return false;
            }
            let source = &app.all_figures[&state.key.number];
            let current_revision = revision(app, state.key.number).unwrap_or(u64::MAX);
            if current_revision != state.revision || !state.source.same_snapshot(source) {
                // Retire IDs using the OLD source, including axes removed by this revision.
                let _ = state.identity.forget(ctx, &state.source);
                state.geometry = None;
                state.source = source.clone();
                state.revision = current_revision;
                state.requested = true;
                state.preparing = false;
                state.reset = true;
                state.command = None;
                state.view = None;
                state.notice.clear();
            }
            true
        });
    }
    pub(super) fn has_command(&self, key: &Key) -> bool {
        self.scenes
            .iter()
            .any(|state| state.key.same(key) && state.command.is_some())
    }
    pub(super) fn has_geometry(&self, key: &Key) -> bool {
        self.scenes
            .iter()
            .any(|state| state.key.same(key) && state.geometry.is_some())
    }
    #[cfg(test)]
    pub(super) fn surface_id(&self, key: &Key) -> Option<egui::Id> {
        self.scenes
            .iter()
            .find(|state| state.key.same(key))
            .map(|state| state.identity.plot_id())
    }
    pub(super) fn show_only(&mut self, keys: &[Key]) {
        for state in &mut self.scenes {
            let visible = keys.iter().any(|key| key.same(&state.key));
            if visible && !state.visible && state.geometry.is_none() {
                state.requested = true;
            }
            state.visible = visible;
        }
    }
    pub fn cancel(&mut self) {
        for state in &mut self.scenes {
            state.command = None;
            state.view = None;
            state.requested = false;
            state.preparing = false;
            if state.geometry.is_none() {
                state.notice =
                    "Display preparation cancelled. Select this scene to prepare it again.".into();
            }
        }
    }
    pub fn retained_bytes(&self) -> usize {
        self.scenes
            .iter()
            .filter_map(|s| s.geometry.as_ref())
            .map(Geometry::retained_bytes)
            .sum()
    }
    pub fn detach(
        &mut self,
        key: &Key,
        revision: u64,
        source: &Figure,
        ctx: &egui::Context,
    ) -> Option<(Geometry, Figure)> {
        let index = self.scenes.iter().position(|state| state.key.same(key))?;
        let mut state = self.scenes.remove(index);
        let _ = state.identity.forget(ctx, &state.source);
        if state.revision != revision || !state.source.same_snapshot(source) {
            return None;
        }
        state
            .geometry
            .take()
            .map(|geometry| (geometry, state.source))
    }
    pub fn evict_oldest(&mut self) -> bool {
        let Some(index) = self
            .scenes
            .iter()
            .enumerate()
            .filter(|(_, s)| s.geometry.is_some())
            .min_by_key(|(_, s)| (s.visible, s.touched))
            .map(|(i, _)| i)
        else {
            return false;
        };
        let state = &mut self.scenes[index];
        state.geometry = None;
        state.requested = false;
        state.notice = "Display cache limit reached. Select this scene to prepare it again.".into();
        // Keep PlotMemory and the committed view. Never requeue just because it painted.
        true
    }
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn request(&mut self, key: &Key) {
        if let Some(state) = self.scenes.iter_mut().find(|s| s.key.same(key)) {
            state.requested = true;
            state.notice.clear();
        }
    }
    /// Pending visible work blocks capture; terminal notices and hidden scenes do not.
    pub fn capture_ready(&self) -> bool {
        !self.scenes.iter().any(|scene| {
            scene.visible && (scene.preparing || (scene.requested && scene.geometry.is_none()))
        })
    }
    pub fn candidate(&self) -> Option<egui::Id> {
        self.scenes
            .iter()
            .filter(|s| s.visible && s.requested && !s.preparing && s.geometry.is_none())
            .max_by_key(|s| s.touched)
            .map(|s| s.identity.plot_id())
    }
    pub fn prepare(&mut self, id: egui::Id, admitted: bool) -> Option<Prepare> {
        let state = self
            .scenes
            .iter_mut()
            .find(|s| s.identity.plot_id() == id)?;
        state.requested = false;
        if !admitted {
            state.notice =
                "Display cache limit reached. Select this scene to prepare it again.".into();
            return None;
        }
        state.preparing = true;
        Some(Prepare {
            key: state.key.clone(),
            revision: state.revision,
            original: state.source.clone(),
            surface: Some(id),
        })
    }
    pub fn accepts(&self, request: &Prepare) -> bool {
        self.scenes.iter().any(|s| {
            Some(s.identity.plot_id()) == request.surface
                && s.key.same(&request.key)
                && s.revision == request.revision
                && s.source.same_snapshot(&request.original)
                && s.preparing
        })
    }
    pub fn submission_failed(&mut self, id: egui::Id, error: &str) {
        if let Some(state) = self.scenes.iter_mut().find(|s| s.identity.plot_id() == id) {
            state.preparing = false;
            state.notice = error.into();
        }
    }
    pub fn complete(&mut self, prepared: Prepared) {
        let request = prepared.request;
        let Some(state) = self.scenes.iter_mut().find(|s| {
            Some(s.identity.plot_id()) == request.surface
                && s.key.same(&request.key)
                && s.revision == request.revision
                && s.source.same_snapshot(&request.original)
                && s.preparing
        }) else {
            return;
        };
        state.preparing = false;
        match prepared.geometry {
            Ok(geometry) => {
                state.geometry = Some(geometry);
                state.reset = true;
                state.notice.clear();
            }
            Err(error) => state.notice = error,
        }
    }
    /// Paint borrows a prepared scene, records typed intent, and never selects an engine figure.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn paint(
        &mut self,
        ui: &mut egui::Ui,
        app: &Workspace,
        key: &Key,
        sequence: &AtomicU64,
    ) -> matrust::Result<()> {
        self.paint_impl(ui, app, key, sequence, false)
    }
    pub fn paint_fitted(
        &mut self,
        ui: &mut egui::Ui,
        app: &Workspace,
        key: &Key,
        sequence: &AtomicU64,
    ) -> matrust::Result<()> {
        self.paint_impl(ui, app, key, sequence, true)
    }
    fn paint_impl(
        &mut self,
        ui: &mut egui::Ui,
        app: &Workspace,
        key: &Key,
        sequence: &AtomicU64,
        fitted: bool,
    ) -> matrust::Result<()> {
        let state = self
            .scenes
            .iter_mut()
            .find(|s| s.key.same(key))
            .ok_or("This docked scene is no longer open")?;
        let current = app
            .all_figures
            .get(&key.number)
            .ok_or("This figure is no longer open")?;
        if !live(app, key)
            || revision(app, key.number) != Some(state.revision)
            || !state.source.same_snapshot(current)
        {
            return Err("Figure changed; preparing its display".into());
        }
        let frame = ui.ctx().cumulative_frame_nr();
        if state.frame != Some(frame) {
            state.frame = Some(frame);
            state.command = None;
            state.view = None;
        }
        state.touched = frame;
        let Some(geometry) = &mut state.geometry else {
            ui.weak(if state.notice.is_empty() {
                "Preparing figure display..."
            } else {
                &state.notice
            });
            return Ok(());
        };
        let enabled = !app.busy && !app.file_action_pending();
        let outcome = ui
            .add_enabled_ui(enabled, |ui| {
                scene::show(
                    ui,
                    scene::Frame {
                        key,
                        revision: state.revision,
                        figure: current,
                        prepared_source: &state.source,
                        marker: app.sample_inspector.marker(key.number, current),
                        restored_view: app
                            .figure_views
                            .get(&key.number)
                            .map(|v| (v.bounds, v.auto)),
                    },
                    geometry,
                    state.reset,
                    if fitted {
                        scene::Identity::ExplicitFitted(&state.identity)
                    } else {
                        scene::Identity::Explicit(&state.identity)
                    },
                )
            })
            .inner?;
        state.reset = false;
        if enabled && outcome.matches(key, state.revision) {
            state.view = outcome.view.map(|view| ViewRecord {
                revision: state.revision,
                bounds: view.bounds,
                auto: view.auto,
            });
            if let Some(interaction) = outcome.interaction {
                let command = match interaction {
                    scene::Interaction::Edit(edit, undoable) => Command::Edit(edit, undoable),
                    scene::Interaction::Pick(position, query) => Command::Pick(position, query),
                };
                state.command = Some(Envelope {
                    key: key.clone(),
                    revision: state.revision,
                    sequence: next(sequence),
                    command,
                });
            }
        }
        Ok(())
    }
    /// Menus and keyboard routing supply the key belonging to their actual painted region.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn action(
        &mut self,
        app: &Workspace,
        ctx: &egui::Context,
        key: &Key,
        action: Action,
        sequence: &AtomicU64,
    ) -> bool {
        if app.busy || app.file_action_pending() || matches!(action, Action::Dock) {
            return false;
        }
        let Some(state) = self.scenes.iter_mut().find(|s| s.key.same(key)) else {
            return false;
        };
        if !live(app, key) || revision(app, key.number) != Some(state.revision) {
            return false;
        }
        let frame = ctx.cumulative_frame_nr();
        if state.frame != Some(frame) {
            state.view = None;
        }
        state.frame = Some(frame);
        state.command = Some(Envelope {
            key: key.clone(),
            revision: state.revision,
            sequence: next(sequence),
            command: Command::Action(action),
        });
        true
    }
    pub fn drain(&mut self, frame: u64) -> Vec<(Key, Option<ViewRecord>, Option<Envelope>)> {
        self.scenes
            .iter_mut()
            .filter_map(|state| {
                if state.frame == Some(frame) {
                    Some((state.key.clone(), state.view.take(), state.command.take()))
                } else {
                    state.view = None;
                    state.command = None;
                    None
                }
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "docked_scene_tests.rs"]
mod tests;

#[cfg(test)]
mod capture_tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn finish(app: &mut Workspace) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.busy && Instant::now() < deadline {
            app.receive();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!app.busy, "{}", app.status);
    }
    fn frame(app: &mut Workspace, ctx: &egui::Context, discard: bool) -> bool {
        let mut pass = 0;
        let output = crate::headless::run(
            ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1440., 900.),
                )),
                ..Default::default()
            },
            |root| {
                if discard && pass == 0 {
                    root.ctx().request_discard("capture after tile admission");
                }
                pass += 1;
                app.ui(root);
            },
        );
        output.viewport_output.values().any(|viewport| {
            viewport
                .commands
                .iter()
                .any(|command| matches!(command, egui::ViewportCommand::Screenshot(_)))
        })
    }
    fn setup() -> (Workspace, egui::Context, tempfile::TempDir, Vec<Key>) {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let dir = tempfile::tempdir().unwrap();
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.folder = dir.path().into();
        app.run("keep=[1 2 3];alias=keep;figure(1);plot([1 3 2]);figure(2);plot([4 2 1]);figure(3);plot([2 5 1]);figure(4);plot([3 1 6]);figure(1);".into());
        finish(&mut app);
        app.show_plot_pane();
        app.layout.mode = crate::layout::PaneMode::Figure;
        app.layout.figure_tiling = Some(matrust::session::FigureTiling::Four);
        let keys = app
            .all_figures
            .iter()
            .map(|(&number, figure)| Key {
                number,
                identity: figure.identity.clone(),
            })
            .collect();
        (app, ctx, dir, keys)
    }
    fn wait_for_capture(app: &mut Workspace, ctx: &egui::Context) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            finish(app);
            if frame(app, ctx, false) {
                assert!(app.screenshot_requested);
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("Settled display did not request capture: {}", app.status);
    }

    #[test]
    fn actual_four_scene_capture_waits_through_every_worker_preparation() {
        for ready_frames in [1, 3] {
            let (mut app, ctx, dir, keys) = setup();
            let sources = app.all_figures.clone();
            app.screenshot_path = Some(dir.path().join("capture.png"));
            app.screenshot_ready_target = ready_frames;
            assert!(
                !frame(&mut app, &ctx, true),
                "Initial unregistered tiles must not capture"
            );
            assert!(
                app.busy,
                "Final pass must admit the first actual Prepare job"
            );
            assert_eq!(app.screenshot_ready_frames, 0);
            for _ in 0..3 {
                finish(&mut app);
                assert!(
                    !frame(&mut app, &ctx, false),
                    "An inter-job gap is not a settled display"
                );
                assert!(!app.screenshot_requested);
                assert_eq!(app.screenshot_ready_frames, 0);
                assert!(app.busy);
            }
            finish(&mut app);
            assert!(keys.iter().all(|key| app.detached.docked.has_geometry(key)));
            wait_for_capture(&mut app, &ctx);
            assert_eq!(app.screenshot_ready_frames, ready_frames);
            for (id, source) in sources {
                assert!(app.all_figures[&id].same_snapshot(&source));
            }
            assert!(app.variables["keep"].same_snapshot(&app.variables["alias"]));
        }
    }

    #[test]
    fn stopped_or_evicted_displays_capture_their_terminal_notice_without_requeue() {
        for stop in [true, false] {
            let (mut app, ctx, dir, keys) = setup();
            let sources = app.all_figures.clone();
            assert!(!frame(&mut app, &ctx, false));
            if stop {
                app.stop_work();
                finish(&mut app);
                assert!(
                    keys.iter()
                        .any(|key| !app.detached.docked.has_geometry(key))
                );
            } else {
                for _ in 0..4 {
                    finish(&mut app);
                    frame(&mut app, &ctx, false);
                }
                finish(&mut app);
                assert!(keys.iter().all(|key| app.detached.docked.has_geometry(key)));
                // Exercise the real cache retirement operation without a giant allocation.
                assert!(app.detached.docked.evict_oldest());
            }
            app.screenshot_path = Some(dir.path().join("notice.png"));
            app.screenshot_ready_target = 1;
            wait_for_capture(&mut app, &ctx);
            assert!(!app.busy);
            assert!(
                keys.iter()
                    .any(|key| !app.detached.docked.has_geometry(key))
            );
            assert!(app.detached.docked.candidate().is_none());
            for (id, source) in sources {
                assert!(app.all_figures[&id].same_snapshot(&source));
            }
        }
    }
}
