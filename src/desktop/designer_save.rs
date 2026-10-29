//! Retain a Designer save gesture until root admission; the existing worker owns saving.
use super::*;
use designer_apply::Design;

#[cfg(test)]
type Picker = fn(&mut Workspace, &egui::Context) -> Option<PathBuf>;

#[derive(Default)]
pub(super) struct State {
    action: Option<Action>,
    finished: Option<(egui::ViewportId, u64)>,
    generation: u64,
    #[cfg(test)]
    picker: Option<Picker>,
    #[cfg(test)]
    calls: usize,
}

struct Action {
    design: Design,
    stamp: (egui::ViewportId, u64),
    generation: u64,
    document: u64,
    interaction: u64,
    focus: Option<egui::Id>,
    folder: PathBuf,
}

fn same_design(a: Design, b: Design) -> bool {
    match (a, b) {
        (Design::Fir(a), Design::Fir(b)) => {
            a.order == b.order
                && a.band == b.band
                && a.sample_rate.to_bits() == b.sample_rate.to_bits()
                && a.cutoff.to_bits() == b.cutoff.to_bits()
                && a.upper.to_bits() == b.upper.to_bits()
                && a.window == b.window
                && a.scale == b.scale
        }
        (Design::Iir(a), Design::Iir(b)) => {
            a.family == b.family
                && a.ripple_db.to_bits() == b.ripple_db.to_bits()
                && a.order == b.order
                && a.band == b.band
                && a.sample_rate.to_bits() == b.sample_rate.to_bits()
                && a.cutoff.to_bits() == b.cutoff.to_bits()
                && a.upper.to_bits() == b.upper.to_bits()
        }
        _ => false,
    }
}

impl Workspace {
    fn designer_save_context(&self, ctx: &egui::Context) -> bool {
        self.root_open_context(ctx)
            && !egui::Popup::is_any_open(ctx)
            && self.toolkits.design_save_available()
    }

    fn valid_designer_save(&self, ctx: &egui::Context, action: &Action) -> bool {
        self.designer_save_context(ctx)
            && action.stamp == (ctx.viewport_id(), ctx.cumulative_frame_nr())
            && action.generation == self.designer_save.generation
            && action.document == self.documents[self.active].id
            && action.interaction == self.document_save.interaction_generation()
            && action.focus == ctx.memory(|m| m.focused())
            && action.folder == self.folder
            && same_design(action.design, self.toolkits.selected_save_design())
    }

    pub(super) fn queue_designer_save(&mut self, ctx: &egui::Context, design: Design) {
        let stamp = (ctx.viewport_id(), ctx.cumulative_frame_nr());
        if !self.designer_save_context(ctx)
            || !same_design(design, self.toolkits.selected_save_design())
            || self.designer_save.action.is_some()
            || self.designer_save.finished == Some(stamp)
        {
            return;
        }
        self.designer_save.action = Some(Action {
            design,
            stamp,
            generation: self.designer_save.generation,
            document: self.documents[self.active].id,
            interaction: self.document_save.interaction_generation(),
            focus: ctx.memory(|m| m.focused()),
            folder: self.folder.clone(),
        });
    }

    pub(super) fn cancel_designer_save(&mut self) {
        self.designer_save.generation = self.designer_save.generation.wrapping_add(1);
        if let Some(action) = self.designer_save.action.take() {
            self.designer_save.finished = Some(action.stamp);
        }
    }

    pub(super) fn observe_designer_save(&mut self, ctx: &egui::Context) {
        if self
            .designer_save
            .action
            .as_ref()
            .is_some_and(|action| !self.valid_designer_save(ctx, action))
        {
            self.cancel_designer_save();
        }
    }

    pub(super) fn finish_designer_save(&mut self, ctx: &egui::Context) {
        if ctx.will_discard() {
            return;
        }
        let Some(action) = self.designer_save.action.take() else {
            return;
        };
        // Cancel and post-picker invalidation consume this release too.
        self.designer_save.finished = Some(action.stamp);
        if !self.valid_designer_save(ctx, &action) {
            return;
        }
        if let Some(path) = self.pick_designer_save_path(ctx)
            && self.valid_designer_save(ctx, &action)
        {
            match action.design {
                Design::Fir(design) => self.submit_fir_save(fir_designer::Save { design, path }),
                Design::Iir(design) => self.submit_iir_save(iir_designer::Save { design, path }),
            }
        }
    }

    fn pick_designer_save_path(&mut self, _ctx: &egui::Context) -> Option<PathBuf> {
        #[cfg(test)]
        {
            self.designer_save.calls += 1;
            assert!(self.designer_save.calls <= 128);
            let picker = self.designer_save.picker.expect("headless Designer picker");
            picker(self, _ctx)
        }
        #[cfg(not(test))]
        {
            rfd::FileDialog::new()
                .add_filter("MatRust script (*.mr)", &["mr"])
                .add_filter("MATLAB script (*.m)", &["m"])
                .set_file_name("filter_design.mr")
                .save_file()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDE: egui::Vec2 = egui::vec2(1440., 900.);
    const COMPACT: egui::Vec2 = egui::vec2(960., 650.);

    fn finish(app: &mut Workspace) {
        let until = Instant::now() + Duration::from_secs(10);
        while (app.busy || app.folder_browser.loading) && Instant::now() < until {
            app.receive();
            app.receive_folder();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!app.busy && !app.folder_browser.loading, "{}", app.status);
    }

    fn frame(
        app: &mut Workspace,
        ctx: &egui::Context,
        size: egui::Vec2,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        headless::run(
            ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |root| app.ui(root),
        )
    }

    fn setup(iir: bool, size: egui::Vec2) -> (egui::Context, Workspace, tempfile::TempDir) {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let mut app = Workspace::with_context(ctx.clone(), true);
        assert!(app.change_folder(dir.path().to_owned()));
        finish(&mut app);
        app.run("keep=[1 2 3]; alias=keep; plot(keep);".into());
        finish(&mut app);
        app.toolkits.open_designer_demo(iir);
        app.designer_save.picker = Some(|_, _| None);
        for _ in 0..5 {
            frame(&mut app, &ctx, size, vec![]);
        }
        assert!(app.designer_save_context(&ctx));
        (ctx, app, dir)
    }

    fn text_rect(shape: &egui::epaint::Shape) -> Option<egui::Rect> {
        match shape {
            egui::epaint::Shape::Text(text)
                if text.galley.text() == "Save design script\u{2026}" =>
            {
                Some(text.galley.rect.translate(text.pos.to_vec2()))
            }
            egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(text_rect),
            _ => None,
        }
    }

    fn button(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2) -> egui::Pos2 {
        let output = frame(app, ctx, size, vec![]);
        output
            .shapes
            .iter()
            .find_map(|shape| {
                text_rect(&shape.shape).filter(|rect| shape.clip_rect.contains_rect(*rect))
            })
            .expect("visible actual Designer save button")
            .center()
    }

    fn pointer(pos: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    fn release_discarded(
        app: &mut Workspace,
        ctx: &egui::Context,
        size: egui::Vec2,
        pos: egui::Pos2,
        mut between: impl FnMut(&mut Workspace, &egui::Context),
    ) {
        let calls = app.designer_save.calls;
        let mut passes = 0;
        headless::run(
            ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events: pointer(pos, false),
                ..Default::default()
            },
            |root| {
                passes += 1;
                if passes == 1 {
                    root.ctx()
                        .request_discard("Designer picker waits for final authority");
                }
                app.ui(root);
                if passes == 1 {
                    assert!(app.designer_save.action.is_some(), "real release retained");
                    assert_eq!(app.designer_save.calls, calls);
                    assert!(!app.busy);
                    between(app, ctx);
                }
            },
        );
        assert!(passes >= 2);
    }

    #[test]
    fn designer_save_cancel_waits_for_final_pass_and_does_not_replay() {
        for iir in [false, true] {
            for size in [COMPACT, WIDE] {
                let (ctx, mut app, _dir) = setup(iir, size);
                for expected in 1..=2 {
                    let pos = button(&mut app, &ctx, size);
                    frame(&mut app, &ctx, size, pointer(pos, true));
                    release_discarded(&mut app, &ctx, size, pos, |_, _| {});
                    assert_eq!(app.designer_save.calls, expected);
                    assert!(!app.busy && app.toolkits.design_save_available());
                }
                let pos = button(&mut app, &ctx, size);
                frame(&mut app, &ctx, size, pointer(pos, true));
                let mut passes = 0;
                headless::run(
                    &ctx,
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        events: pointer(pos, false),
                        ..Default::default()
                    },
                    |root| {
                        passes += 1;
                        app.ui(root);
                        if passes == 1 {
                            assert_eq!(app.designer_save.calls, 3);
                            root.ctx()
                                .request_discard("Completed Cancel cannot reopen picker");
                        }
                    },
                );
                assert!(passes >= 2);
                assert_eq!(app.designer_save.calls, 3);
                assert!(!app.busy && app.toolkits.design_save_available());
            }
        }
    }

    fn change_settings(app: &mut Workspace) {
        let mut design = app.toolkits.selected_save_design();
        match &mut design {
            Design::Fir(d) => d.cutoff += 1.,
            Design::Iir(d) => d.ripple_db += 1.,
        }
        app.toolkits.set_save_design_for_test(design);
    }

    fn invalidate(app: &mut Workspace, ctx: &egui::Context, case: usize) {
        match case {
            0 => app.stop_work(),
            1 => app.busy = true,
            2 => app.pending_quit = true,
            3 => app.allow_close = true,
            4 => app.recovering_session = true,
            5 => app.recovery_blocked = true,
            6 => app.folder.push("different"),
            7 => app.new_document("different_owner=1;"),
            8 => {
                app.begin_workspace_action(false, true);
                assert!(app.workspace_tools.dialog.is_some());
            }
            9 => ctx.memory_mut(|m| m.request_focus(egui::Id::new("late_designer_owner"))),
            10 => app.toolkits.designer_open = false,
            11 => change_settings(app),
            12 => {
                let iir = matches!(app.toolkits.selected_save_design(), Design::Iir(_));
                app.toolkits.open_designer_demo(!iir);
            }
            13 => {
                let Design::Fir(d) = app.toolkits.selected_save_design() else {
                    unreachable!()
                };
                app.toolkits.begin_fir_save(d).unwrap();
            }
            14 => ctx.input_mut(|i| i.focused = false),
            _ => unreachable!(),
        }
    }

    #[test]
    fn designer_save_rejects_late_authority_before_and_after_picker() {
        for after in [false, true] {
            for case in 0..15 {
                let (ctx, mut app, dir) = setup(false, WIDE);
                let path = dir.path().join("design.m");
                std::fs::write(&path, "existing destination").unwrap();
                let values = app.variables.clone();
                let model = app.toolkits.model.clone();
                if after {
                    app.designer_save.picker = Some(|app, ctx| {
                        let path = app.folder.join("design.m");
                        let case = app
                            .status
                            .strip_prefix("designer test ")
                            .unwrap()
                            .parse()
                            .unwrap();
                        invalidate(app, ctx, case);
                        Some(path)
                    });
                } else {
                    app.designer_save.picker = Some(|_, _| panic!("stale gesture opened picker"));
                }
                let pos = button(&mut app, &ctx, WIDE);
                frame(&mut app, &ctx, WIDE, pointer(pos, true));
                let (send, receive) = std::sync::mpsc::sync_channel(8);
                let original_send = std::mem::replace(&mut app.worker.send, send);
                if after {
                    app.status = format!("designer test {case}");
                    frame(&mut app, &ctx, WIDE, pointer(pos, false));
                } else {
                    release_discarded(&mut app, &ctx, WIDE, pos, |app, ctx| {
                        invalidate(app, ctx, case);
                        if case == 14 {
                            // egui retries restore RawInput.focused. Observe this
                            // injected focus loss now, not after it is overwritten.
                            assert!(!ctx.input(|i| i.focused));
                            assert!(app.designer_save.action.is_some());
                            let generation = app.designer_save.generation;
                            app.observe_designer_save(ctx);
                            assert!(app.designer_save.action.is_none());
                            assert_eq!(app.designer_save.generation, generation.wrapping_add(1));
                        }
                    });
                }
                assert_eq!(app.designer_save.calls, usize::from(after), "case {case}");
                assert_eq!(app.busy, case == 1, "no submitted worker: case {case}");
                assert!(app.designer_save.action.is_none());
                assert!(matches!(
                    receive.try_recv(),
                    Err(std::sync::mpsc::TryRecvError::Empty)
                ));
                app.worker.send = original_send;
                assert_eq!(std::fs::read(&path).unwrap(), b"existing destination");
                assert!(app.toolkits.model.same_storage(&model));
                for (name, value) in &values {
                    assert!(value.same_snapshot(&app.variables[name]));
                }
                if case == 1 {
                    app.busy = false;
                }
                if case == 13 {
                    app.toolkits
                        .finish_design_save(Err("fixture pending operation retired".into()));
                }
                frame(&mut app, &ctx, WIDE, vec![]);
                assert_eq!(app.designer_save.calls, usize::from(after));
            }
        }
    }

    #[test]
    fn designer_actual_save_captures_same_frame_settings_and_preserves_worker_state() {
        for iir in [false, true] {
            let (ctx, mut app, dir) = setup(iir, WIDE);
            let before = app.variables.clone();
            let model = app.toolkits.model.clone();
            let documents: Vec<_> = app
                .documents
                .iter()
                .map(|d| (d.id, d.text.clone()))
                .collect();
            let figures = app.all_figures.clone();
            app.designer_save.picker = Some(|app, _| Some(app.folder.join("design.m")));
            let pos = button(&mut app, &ctx, WIDE);
            frame(&mut app, &ctx, WIDE, pointer(pos, true));
            // A settings edit before the release's UI pass must be in its snapshot.
            change_settings(&mut app);
            let design = app.toolkits.selected_save_design();
            let expected = match design {
                Design::Fir(d) => d.exported_script().unwrap(),
                Design::Iir(d) => d
                    .portable_script(&std::sync::atomic::AtomicBool::new(false))
                    .unwrap(),
            };
            release_discarded(&mut app, &ctx, WIDE, pos, |app, _| {
                assert!(same_design(
                    app.designer_save.action.as_ref().unwrap().design,
                    design
                ));
            });
            assert_eq!(app.designer_save.calls, 1);
            assert!(app.busy);
            change_settings(&mut app);
            app.toolkits.designer_open = false;
            finish(&mut app);
            assert_eq!(
                std::fs::read_to_string(dir.path().join("design.m")).unwrap(),
                expected
            );
            assert!(app.toolkits.message.starts_with("Saved"));
            assert!(app.toolkits.model.same_storage(&model));
            for (name, value) in &before {
                assert!(value.same_snapshot(&app.variables[name]));
            }
            assert!(app.variables["keep"].same_snapshot(&app.variables["alias"]));
            assert_eq!(
                documents,
                app.documents
                    .iter()
                    .map(|d| (d.id, d.text.clone()))
                    .collect::<Vec<_>>()
            );
            for (id, figure) in figures {
                assert!(figure.same_snapshot(&app.all_figures[&id]));
            }
        }
    }
}
