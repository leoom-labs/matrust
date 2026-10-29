use super::*;

enum Owner {
    Preview(PathBuf, Arc<egui::ColorImage>),
    Figure,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum ReviewAction {
    Commit,
    Cancel,
}
#[derive(Default)]
pub(super) struct State {
    ticket: u64,
    owner: Option<Owner>,
    in_flight: bool,
    closed_preview: bool,
    review: Option<single_export::Plan>,
    action: Option<(egui::ViewportId, u64, u64, ReviewAction)>,
}
impl State {
    pub(super) fn pending(&self) -> bool {
        self.owner.is_some()
    }
    pub(super) fn reviewing(&self) -> bool {
        self.review.is_some()
    }
}
impl Workspace {
    pub(super) fn close_preview_export(&mut self) {
        if matches!(self.single_export.owner, Some(Owner::Preview(..))) {
            if self.single_export.in_flight {
                // Dispatched work owns its pixels; Close retires only its UI authority.
                // A late replacement review is never approved by closing the preview.
                self.single_export.closed_preview = true;
            } else {
                self.cancel_preview_export();
            }
        }
    }
    pub(super) fn cancel_preview_export(&mut self) {
        if matches!(self.single_export.owner, Some(Owner::Preview(..))) {
            self.cancel_single_export();
        }
    }
    pub(super) fn cancel_single_export(&mut self) {
        let figure = matches!(self.single_export.owner, Some(Owner::Figure));
        if self.single_export.pending() {
            self.image_action = false;
        }
        if self.single_export.in_flight {
            self.worker.cancel.store(true, Ordering::Relaxed);
        }
        self.single_export.ticket = self.single_export.ticket.wrapping_add(1);
        self.single_export.owner = None;
        self.single_export.closed_preview = false;
        self.single_export.in_flight = false;
        self.single_export.review = None; // drops only this attempt's staged output
        self.single_export.action = None;
        if figure && let Some(dialog) = &mut self.figure_export {
            dialog.saving = false;
        }
    }
    pub(super) fn bind_single_export(&mut self, job: Job) -> Job {
        let (input, owner) = match job {
            Job::ExportPreview(request) => {
                let owner = Owner::Preview(request.source.clone(), request.pixels.clone());
                (single_export::Input::Preview(request), owner)
            }
            Job::ExportSnapshot(request) => (single_export::Input::Figure(request), Owner::Figure),
            Job::SingleExport(request) => return Job::SingleExport(request),
            other => {
                if self.single_export.pending() {
                    self.cancel_single_export();
                }
                return other;
            }
        };
        // New requests invalidate earlier prepared output and review decisions.
        self.single_export.ticket = self.single_export.ticket.wrapping_add(1);
        self.single_export.owner = Some(owner);
        self.single_export.closed_preview = false;
        self.single_export.in_flight = true;
        self.single_export.review = None;
        self.single_export.action = None;
        Job::SingleExport(Box::new(single_export::Request {
            ticket: self.single_export.ticket,
            action: single_export::Action::Prepare(input),
        }))
    }
    fn single_export_matches(&self) -> bool {
        match &self.single_export.owner {
            Some(Owner::Figure) => true, // captured figure/view/theme, independent of later selection
            Some(Owner::Preview(path, pixels)) => self
                .image_view
                .as_ref()
                .is_some_and(|v| &v.path == path && Arc::ptr_eq(&v.pixels, pixels)),
            None => false,
        }
    }
    fn single_export_result(&mut self, result: Result<PathBuf, String>) {
        let notify_preview = !self.single_export.closed_preview && self.single_export_matches();
        let figure = matches!(self.single_export.owner, Some(Owner::Figure));
        match result {
            Ok(path) => {
                let message = format!(
                    "{} {}",
                    if figure {
                        "Exported"
                    } else {
                        "Saved preview to"
                    },
                    path_label::display(&path)
                );
                self.status = message.clone();
                if figure {
                    self.figure_export = None;
                } else if notify_preview && let Some(viewer) = &mut self.image_view {
                    viewer.notice = Some(message);
                    viewer.error = None;
                }
            }
            Err(error) => {
                self.status = error.clone();
                if figure {
                    if let Some(dialog) = &mut self.figure_export {
                        dialog.saving = false;
                        dialog.error = Some(error);
                    }
                } else if notify_preview && let Some(viewer) = &mut self.image_view {
                    viewer.error = Some(error);
                }
            }
        }
        self.single_export.owner = None;
        self.single_export.closed_preview = false;
        self.single_export.review = None;
        self.single_export.action = None;
        self.single_export.in_flight = false;
        self.image_action = false;
    }
    pub(super) fn receive_single_export(&mut self, finished: single_export::Finished) {
        if finished.ticket != self.single_export.ticket {
            return;
        }
        self.image_action = false;
        self.single_export.in_flight = false;
        if self.worker.cancel.load(Ordering::Relaxed)
            || (!self.single_export_matches() && !self.single_export.closed_preview)
            || self.allow_close
            || self.pending_quit
        {
            self.cancel_single_export();
            return;
        }
        match finished.result {
            Ok(single_export::Outcome::Review(plan)) => {
                if self.single_export.closed_preview {
                    drop(plan);
                    self.cancel_single_export();
                    self.status = "Preview closed; replacement cancelled".into();
                    return;
                }
                self.status = "Export prepared; review replacement".into();
                self.single_export.review = Some(plan);
            }
            Ok(single_export::Outcome::Saved(path)) => self.single_export_result(Ok(path)),
            Err(error) => self.single_export_result(Err(error)),
        }
    }
    pub(super) fn single_export_ui(&mut self, ctx: &egui::Context) {
        if self.single_export.review.is_none() {
            return;
        }
        if !self.single_export_matches() || self.allow_close || self.pending_quit {
            self.cancel_single_export();
            return;
        }
        let plan = self.single_export.review.as_ref().unwrap();
        let mut commit = false;
        let mut cancel = false;
        let shown = egui::Modal::new(egui::Id::new("single_export_replace")).show(ctx, |ui| {
            ui.set_max_width((ctx.content_rect().width() - 48.).clamp(240., 440.));
            ui.label(crate::tokens::title(ui.ctx(), "Replace export destination?"));
            ui.label(path_label::display(plan.destination()).to_string());
            ui.label("The selected output is ready. Replace the existing file with this prepared preview or figure? Changes to the destination since preparation will stop the save.");
            ui.horizontal(|ui| {
                commit = ui.add_enabled(!self.busy, egui::Button::new("Replace destination")).clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        cancel |= shown.should_close();
        let stamp = (
            ctx.viewport_id(),
            ctx.cumulative_frame_nr(),
            self.single_export.ticket,
        );
        if cancel || commit {
            self.single_export.action = Some((
                stamp.0,
                stamp.1,
                stamp.2,
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
            .single_export
            .action
            .take()
            .filter(|(v, f, t, _)| (*v, *f, *t) == stamp)
            .map(|(_, _, _, a)| a);
        if action == Some(ReviewAction::Cancel) {
            self.cancel_single_export();
        } else if action == Some(ReviewAction::Commit)
            && !self.busy
            && self.single_export_matches()
            && !egui::Popup::is_any_open(ctx)
            && shown.is_top_modal
        {
            let plan = self.single_export.review.take().unwrap();
            self.single_export.in_flight = true;
            self.submit(Job::SingleExport(Box::new(single_export::Request {
                ticket: self.single_export.ticket,
                action: single_export::Action::Commit(plan),
            })));
        }
    }
}
#[cfg(test)]
#[path = "single_export_ui_tests.rs"]
mod tests;
