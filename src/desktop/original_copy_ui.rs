use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReviewAction {
    Commit,
    Cancel,
}
#[derive(Default)]
pub(super) struct State {
    ticket: u64,
    active: Option<(PathBuf, [u8; 32])>,
    in_flight: bool,
    review: Option<original_copy::Plan>,
    // Raw input is consumed on the first pass. Retain only this review's intent
    // through its final pass; never replay it in a later frame or ticket.
    action: Option<(u64, u64, ReviewAction)>,
    picker: Option<(u64, u64, PathBuf, [u8; 32])>,
}
impl State {
    pub(super) fn pending(&self) -> bool {
        self.review.is_some()
    }
}
impl Workspace {
    pub(super) fn clear_original_copy_picker(&mut self) {
        self.original_copy.picker = None;
    }
    pub(super) fn finish_original_copy_picker(&mut self, ctx: &egui::Context, clicked: bool) {
        let frame = ctx.cumulative_frame_nr();
        if clicked
            && !self.busy
            && !self.file_action_pending()
            && !self.allow_close
            && !self.pending_quit
            && let Some(viewer) = &self.image_view
            && let Some(original) = &viewer.original
        {
            self.original_copy.picker = Some((
                frame,
                self.original_copy.ticket,
                viewer.path.clone(),
                original.digest,
            ));
        }
        if ctx.will_discard() {
            return;
        }
        let Some((observed, ticket, path, digest)) = self.original_copy.picker.take() else {
            return;
        };
        if observed != frame
            || ticket != self.original_copy.ticket
            || self.busy
            || self.file_action_pending()
            || self.allow_close
            || self.pending_quit
            || !self.image_view.as_ref().is_some_and(|viewer| {
                viewer.path == path
                    && viewer
                        .original
                        .as_ref()
                        .is_some_and(|original| original.digest == digest)
            })
        {
            return;
        }
        self.save_original_copy();
    }
    pub(super) fn cancel_original_copy(&mut self) {
        if self.original_copy.in_flight {
            self.worker.cancel.store(true, Ordering::Relaxed);
        }
        self.original_copy.ticket = self.original_copy.ticket.wrapping_add(1);
        self.original_copy.active = None;
        self.original_copy.in_flight = false;
        self.original_copy.review = None;
        self.original_copy.action = None;
        self.original_copy.picker = None;
    }
    pub(super) fn save_original_copy(&mut self) {
        if self.busy || self.file_action_pending() {
            return;
        }
        let Some(viewer) = &self.image_view else {
            return;
        };
        if viewer.original.is_none() {
            return;
        }
        if let Some(path) = pick_original_copy(&viewer.path) {
            self.prepare_original_copy_to(path);
        }
    }
    pub(super) fn prepare_original_copy_to(&mut self, destination: PathBuf) {
        if self.busy || self.file_action_pending() || self.allow_close {
            return;
        }
        let Some(viewer) = &self.image_view else {
            return;
        };
        let Some(original) = &viewer.original else {
            return;
        };
        let source = viewer.path.clone();
        let original = original.clone();
        if let Some(viewer) = &mut self.image_view {
            viewer.error = None;
            viewer.notice = None;
        }
        self.original_copy.ticket = self.original_copy.ticket.wrapping_add(1);
        self.original_copy.active = Some((source.clone(), original.digest));
        let ticket = self.original_copy.ticket;
        self.original_copy.action = None;
        self.original_copy.picker = None;
        self.submit(Job::OriginalCopy(Box::new(original_copy::Request {
            ticket,
            action: original_copy::Action::Prepare(original_copy::Prepare {
                source,
                original,
                destination,
            }),
        })));
        self.original_copy.in_flight = self.busy;
        if !self.busy {
            self.original_copy.active = None;
        }
    }
    fn original_copy_matches(&self) -> bool {
        self.original_copy
            .active
            .as_ref()
            .is_some_and(|(path, digest)| {
                self.image_view.as_ref().is_some_and(|view| {
                    &view.path == path
                        && view
                            .original
                            .as_ref()
                            .is_some_and(|original| &original.digest == digest)
                })
            })
    }
    pub(super) fn receive_original_copy(&mut self, finished: original_copy::Finished) {
        if finished.ticket != self.original_copy.ticket {
            return;
        }
        self.original_copy.in_flight = false;
        if self.worker.cancel.load(Ordering::Relaxed) || !self.original_copy_matches() {
            self.cancel_original_copy();
            return;
        }
        match finished.result {
            Ok(original_copy::Outcome::Review(plan)) => self.original_copy.review = Some(plan),
            Ok(original_copy::Outcome::Saved(path)) => {
                self.original_copy.active = None;
                let text = format!("Saved original copy {}", path_label::display(&path));
                self.status = text.clone();
                if let Some(viewer) = &mut self.image_view {
                    viewer.notice = Some(text);
                }
            }
            Err(error) => {
                self.original_copy.active = None;
                self.status = error.clone();
                if let Some(viewer) = &mut self.image_view {
                    viewer.error = Some(error);
                }
            }
        }
    }
    pub(super) fn original_copy_ui(&mut self, ctx: &egui::Context) {
        let Some(plan) = &self.original_copy.review else {
            return;
        };
        if !self.original_copy_matches() {
            self.cancel_original_copy();
            return;
        }
        let mut commit = !plan.replaces();
        let mut cancel = false;
        if plan.replaces() {
            let shown = egui::Modal::new(egui::Id::new("original_copy_replace")).show(ctx, |ui| {
                ui.set_max_width((ctx.content_rect().width() - 48.).clamp(240., 440.));
                ui.label(crate::tokens::title(ui.ctx(), "Replace original-copy destination?"));
                ui.label(path_label::display(plan.destination()).to_string());
                ui.label("This copies the complete original document without converting pages, vectors, metadata or bit depth. The open source file is preserved.");
                ui.horizontal(|ui| { commit = ui.button("Replace destination").clicked(); cancel = ui.button("Cancel").clicked(); });
            });
            cancel |= shown.should_close();
        }
        let frame = ctx.cumulative_frame_nr();
        if cancel || commit {
            self.original_copy.action = Some((
                frame,
                self.original_copy.ticket,
                if cancel {
                    ReviewAction::Cancel
                } else {
                    ReviewAction::Commit
                },
            ));
        }
        if ctx.will_discard() {
            return;
        }
        let action = self
            .original_copy
            .action
            .take()
            .filter(|(observed, ticket, _)| {
                *observed == frame && *ticket == self.original_copy.ticket
            })
            .map(|(_, _, action)| action);
        if action == Some(ReviewAction::Cancel) || self.allow_close || self.pending_quit {
            self.cancel_original_copy();
        } else if action == Some(ReviewAction::Commit) && !self.busy {
            let plan = self.original_copy.review.take().unwrap();
            let ticket = self.original_copy.ticket;
            self.submit(Job::OriginalCopy(Box::new(original_copy::Request {
                ticket,
                action: original_copy::Action::Commit(plan),
            })));
            self.original_copy.in_flight = self.busy;
            if !self.busy {
                self.original_copy.active = None;
            }
        }
    }
}

fn pick_original_copy(source: &std::path::Path) -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(choice) = ORIGINAL_COPY_PICKER.with(|slot| {
        slot.borrow_mut().as_mut().map(|(choice, calls)| {
            *calls += 1;
            choice.clone()
        })
    }) {
        return choice;
    }
    let stem = source.file_stem().unwrap_or_default().to_string_lossy();
    let extension = source.extension().unwrap_or_default().to_string_lossy();
    rfd::FileDialog::new()
        .set_directory(source.parent().unwrap_or(std::path::Path::new(".")))
        .add_filter("Original document", &[extension.as_ref()])
        .set_file_name(format!("{stem}_copy.{extension}"))
        .save_file()
}
#[cfg(test)]
thread_local! {
    static ORIGINAL_COPY_PICKER: std::cell::RefCell<Option<(Option<PathBuf>, usize)>> = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
struct OriginalCopyPicker;
#[cfg(test)]
impl OriginalCopyPicker {
    fn install(choice: Option<PathBuf>) -> Self {
        ORIGINAL_COPY_PICKER.with(|slot| {
            assert!(slot.borrow().is_none());
            *slot.borrow_mut() = Some((choice, 0));
        });
        Self
    }
    fn calls(&self) -> usize {
        ORIGINAL_COPY_PICKER.with(|slot| slot.borrow().as_ref().unwrap().1)
    }
}
#[cfg(test)]
impl Drop for OriginalCopyPicker {
    fn drop(&mut self) {
        ORIGINAL_COPY_PICKER.with(|slot| *slot.borrow_mut() = None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn finish(app: &mut Workspace) {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.busy && std::time::Instant::now() < until {
            app.receive();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(!app.busy, "{}", app.status);
    }
    fn frame(
        app: &mut Workspace,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        discard: bool,
    ) -> egui::FullOutput {
        let mut pass = 0;
        crate::headless::run(
            ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(960., 650.),
                )),
                events,
                ..Default::default()
            },
            |_| {
                app.image_view_ui(ctx);
                if discard && pass == 0 {
                    ctx.request_discard("original copy is admitted on final pass only");
                }
                app.original_copy_ui(ctx);
                if discard && ctx.will_discard() {
                    assert!(
                        app.original_copy.pending(),
                        "Discarded pass cannot consume review"
                    );
                    assert!(
                        !app.busy,
                        "Discarded pass cannot submit original-copy publication"
                    );
                }
                pass += 1;
            },
        )
    }
    fn label(output: &egui::FullOutput, text: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::Shape::Text(t) = &shape.shape
                    && t.galley.text() == text
                    && shape.clip_rect.contains_rect(t.visual_bounding_rect())
                {
                    Some(t.visual_bounding_rect().center())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| panic!("missing full visible {text}"))
    }
    fn click(app: &mut Workspace, ctx: &egui::Context, text: &str) {
        // Area deliberately hides its initial sizing pass and then anchors
        // using measured size. Target the actual settled, fully visible label.
        for _ in 0..3 {
            frame(app, ctx, vec![], false);
        }
        let pos = label(&frame(app, ctx, vec![], false), text);
        for pressed in [true, false] {
            frame(
                app,
                ctx,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
                !pressed,
            );
        }
    }
    #[test]
    fn actual_worker_original_copy_review_pointer_commit_preserves_other_documents_and_samples() {
        for dark in [false, true] {
            let ctx = egui::Context::default();
            let mut app = Workspace::with_context(ctx.clone(), true);
            ctx.set_theme(if dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            });
            let dir = tempfile::tempdir().unwrap();
            app.folder = dir.path().to_owned();
            let path = dir.path().join("source.svg");
            let bytes=b"<svg xmlns='http://www.w3.org/2000/svg' width='64' height='32'><rect width='64' height='32' fill='red'/></svg>";
            std::fs::write(&path, bytes).unwrap();
            app.run("a=1:10;alias=a;plot(a);".into());
            finish(&mut app);
            let variables = app.variables.clone();
            let figure = app.figure.clone().unwrap();
            let documents = serde_json::to_vec(&app.documents).unwrap();
            let history = app.figure_history.clone();
            app.submit(Job::OpenImage(path.clone()));
            finish(&mut app);
            for _ in 0..3 {
                frame(&mut app, &ctx, vec![], false);
            }
            assert_eq!(ctx.global_style().visuals.dark_mode, dark);
            let output = frame(&mut app, &ctx, vec![], false);
            label(&output, "Save original copy...");
            label(&output, "Save preview...");
            let pixels = app.image_view.as_ref().unwrap().pixels.clone();
            let target = dir.path().join("target.svg");
            std::fs::write(&target, b"prior").unwrap();
            app.prepare_original_copy_to(target.clone());
            finish(&mut app);
            assert!(app.original_copy.pending());
            assert!(app.file_action_pending());
            assert_eq!(std::fs::read(&target).unwrap(), b"prior");
            click(&mut app, &ctx, "Replace destination");
            finish(&mut app);
            assert_eq!(std::fs::read(&target).unwrap(), bytes);
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
            assert!(!app.original_copy.pending());
            assert!(Arc::ptr_eq(
                &pixels,
                &app.image_view.as_ref().unwrap().pixels
            ));
            assert_eq!(serde_json::to_vec(&app.documents).unwrap(), documents);
            assert!(app.figure.as_ref().unwrap().same_snapshot(&figure));
            assert_eq!(app.figure_history, history);
            for (name, before) in variables {
                assert!(app.variables[&name].same_snapshot(&before));
            }
            assert!(app.variables["a"].same_snapshot(&app.variables["alias"]));
        }
    }
    #[test]
    fn stop_and_closed_preview_discard_actual_late_review_without_destination_publication() {
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.svg");
        std::fs::write(
            &source,
            b"<svg xmlns='http://www.w3.org/2000/svg' width='8' height='8'/>",
        )
        .unwrap();
        app.submit(Job::OpenImage(source.clone()));
        finish(&mut app);
        frame(&mut app, &ctx, vec![], false);
        for close in [false, true] {
            let target = dir
                .path()
                .join(if close { "close.svg" } else { "stop.svg" });
            app.prepare_original_copy_to(target.clone());
            if close {
                app.close_image_preview();
            } else {
                app.stop_work();
            }
            finish(&mut app);
            frame(&mut app, &ctx, vec![], false);
            assert!(!target.exists());
            assert!(!app.original_copy.pending());
            assert!(app.original_copy.active.is_none());
        }
    }

    #[test]
    fn real_window_close_during_replacement_review_does_not_open_a_second_quit_flow() {
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.svg");
        let target = dir.path().join("target.svg");
        std::fs::write(
            &source,
            b"<svg xmlns='http://www.w3.org/2000/svg' width='8' height='8'/>",
        )
        .unwrap();
        std::fs::write(&target, b"prior").unwrap();
        app.run("plot([1 2]);".into());
        finish(&mut app);
        assert!(app.figure_dirty(app.current_figure));
        app.submit(Job::OpenImage(source));
        finish(&mut app);
        frame(&mut app, &ctx, vec![], false);
        app.prepare_original_copy_to(target.clone());
        finish(&mut app);
        assert!(app.original_copy.pending());
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(960., 650.),
            )),
            ..Default::default()
        };
        input
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .events
            .push(egui::ViewportEvent::Close);
        let output = crate::headless::run(&ctx, input, |root| app.ui(root));
        assert!(
            output.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .iter()
                .any(|c| matches!(c, egui::ViewportCommand::CancelClose))
        );
        assert!(app.original_copy.pending());
        assert!(app.figure_close.is_none());
        assert!(!app.pending_quit);
        assert!(!app.allow_close);
        assert_eq!(std::fs::read(&target).unwrap(), b"prior");
        click(&mut app, &ctx, "Cancel");
        assert!(!app.original_copy.pending());
        assert_eq!(std::fs::read(target).unwrap(), b"prior");
    }

    fn original_picker_position(app: &mut Workspace, ctx: &egui::Context) -> egui::Pos2 {
        for _ in 0..3 {
            frame(app, ctx, vec![], false);
        }
        label(&frame(app, ctx, vec![], false), "Save original copy...")
    }
    fn original_picker_release(
        app: &mut Workspace,
        ctx: &egui::Context,
        picker: &OriginalCopyPicker,
        position: egui::Pos2,
        mut after_discard: impl FnMut(&mut Workspace),
    ) {
        frame(
            app,
            ctx,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            false,
        );
        let mut passes = 0;
        let _ = crate::headless::run(
            ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(960., 650.),
                )),
                events: vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
            |root| {
                passes += 1;
                if passes == 1 {
                    root.ctx()
                        .request_discard("Original picker waits for final pass");
                }
                app.image_view_ui(root.ctx());
                if passes == 1 {
                    assert!(root.ctx().will_discard());
                    assert_eq!(
                        picker.calls(),
                        0,
                        "No OS dialog admission in discarded pass"
                    );
                    assert!(!app.busy);
                    assert!(
                        app.original_copy.picker.is_some(),
                        "Actual button release must queue the bounded intent"
                    );
                    after_discard(app);
                }
            },
        );
        assert!(passes >= 2);
    }
    #[test]
    fn original_copy_button_retries_layout_then_opens_one_picker_and_cancels_or_copies() {
        for chosen in [false, true] {
            let ctx = egui::Context::default();
            let mut app = Workspace::with_context(ctx.clone(), true);
            let dir = tempfile::tempdir().unwrap();
            let source = dir.path().join("source.svg");
            let destination = dir.path().join("chosen.svg");
            let bytes=b"<svg xmlns='http://www.w3.org/2000/svg' width='64' height='32'><rect width='64' height='32' fill='red'/></svg>";
            std::fs::write(&source, bytes).unwrap();
            app.run("a=[1 2 3];alias=a;plot(a);".into());
            finish(&mut app);
            let variables = app.variables.clone();
            let figure = app.figure.clone().unwrap();
            let documents = serde_json::to_vec(&app.documents).unwrap();
            app.submit(Job::OpenImage(source.clone()));
            finish(&mut app);
            let position = original_picker_position(&mut app, &ctx);
            let pixels = app.image_view.as_ref().unwrap().pixels.clone();
            let picker = OriginalCopyPicker::install(chosen.then(|| destination.clone()));
            original_picker_release(&mut app, &ctx, &picker, position, |_| {});
            assert_eq!(picker.calls(), 1);
            assert!(app.original_copy.picker.is_none());
            if chosen {
                assert!(app.busy);
                finish(&mut app);
                assert!(app.original_copy.pending());
                frame(&mut app, &ctx, vec![], false);
                finish(&mut app);
                assert_eq!(std::fs::read(&destination).unwrap(), bytes);
            } else {
                assert!(!app.busy && !app.original_copy.pending());
                assert!(app.original_copy.active.is_none());
                assert!(!destination.exists());
            }
            for _ in 0..3 {
                frame(&mut app, &ctx, vec![], false);
            }
            assert_eq!(
                picker.calls(),
                1,
                "A later frame must not reopen the picker"
            );
            assert_eq!(std::fs::read(&source).unwrap(), bytes);
            assert_eq!(serde_json::to_vec(&app.documents).unwrap(), documents);
            assert!(app.figure.as_ref().unwrap().same_snapshot(&figure));
            assert!(Arc::ptr_eq(
                &pixels,
                &app.image_view.as_ref().unwrap().pixels
            ));
            for (name, value) in variables {
                assert!(app.variables[&name].same_snapshot(&value));
            }
            assert!(app.variables["a"].same_snapshot(&app.variables["alias"]));
        }
    }
    #[test]
    fn original_copy_button_rejects_stop_close_and_new_preview_before_final_picker() {
        for change in ["stop", "close", "new_same", "new_different"] {
            let ctx = egui::Context::default();
            let mut app = Workspace::with_context(ctx.clone(), true);
            let dir = tempfile::tempdir().unwrap();
            let source = dir.path().join("source.svg");
            let replacement = dir.path().join("replacement.svg");
            let destination = dir.path().join("never.svg");
            let bytes = b"<svg xmlns='http://www.w3.org/2000/svg' width='32' height='16'/>";
            std::fs::write(&source, bytes).unwrap();
            std::fs::write(&replacement, bytes).unwrap();
            app.submit(Job::OpenImage(source.clone()));
            finish(&mut app);
            let position = original_picker_position(&mut app, &ctx);
            let next_path = if change == "new_same" {
                &source
            } else {
                &replacement
            };
            // Produce the genuine replacement outside the UI pass; installation
            // still follows the application's actual pending-image branch.
            let mut next = Some(
                crate::svg_load::load(next_path, 64, &std::sync::atomic::AtomicBool::new(false))
                    .unwrap(),
            );
            let picker = OriginalCopyPicker::install(Some(destination.clone()));
            original_picker_release(&mut app, &ctx, &picker, position, |app| match change {
                "stop" => app.stop_work(),
                "close" => app.close_image_preview(),
                _ => app.pending_image = next.take(),
            });
            assert_eq!(picker.calls(), 0);
            assert!(!app.busy && !app.original_copy.pending());
            assert!(app.original_copy.picker.is_none());
            assert!(!destination.exists());
            assert_eq!(std::fs::read(source).unwrap(), bytes);
        }
    }
}
