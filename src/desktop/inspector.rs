//! Compact Command/Editor Inspector. The sheet never consumes work-area width.
use super::*;

pub(crate) fn uses_sheet(app: &Workspace, ctx: &egui::Context) -> bool {
    !app.editor_window.in_callback
        && ctx.viewport_id() == egui::ViewportId::ROOT
        && tokens::compact(ctx)
        && app.layout.center.is_some()
        && !app.workspace_on_left()
        && matches!(app.mode(), modes::Mode::Command | modes::Mode::Editor)
}

pub(super) fn sheet(app: &mut Workspace, root: &mut egui::Ui) {
    let ctx = root.ctx().clone();
    // Variable Editor and other modal work own the foreground while open.
    // Retain the sheet intent so returning to the desktop restores it.
    if !app.navigation.inspector_open || app.editing_variable.is_some() || app.file_action_pending()
    {
        return;
    }
    if !app.file_action_pending()
        && app.editing_variable.is_none()
        && !egui::Popup::is_any_open(&ctx)
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
    {
        app.navigation.inspector_open = false;
        return;
    }
    let available = root.available_rect_before_wrap();
    let width = 320_f32.min(available.width());
    let p = tokens::palette(&ctx);
    egui::Area::new(egui::Id::new("workspace_inspector_sheet"))
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(available.right() - width, available.top()))
        .movable(false)
        .show(&ctx, |ui| {
            if !root.is_enabled() {
                ui.disable();
            }
            egui::Frame::new()
                .fill(p.surface_sunken)
                .stroke(egui::Stroke::new(1., p.border))
                .inner_margin(12)
                .show(ui, |ui| {
                    let body = egui::vec2(width - 26., (available.height() - 26.).max(80.));
                    ui.set_min_size(body);
                    ui.set_max_size(body);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("INSPECTOR").font(panels::caption_semibold(ui)));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Close").clicked() {
                                app.navigation.inspector_open = false;
                            }
                        });
                    });
                    ui.add_space(tokens::S);
                    if app.figure_column() {
                        let height = (ui.available_height() * 0.45).clamp(120., 240.);
                        egui::Panel::bottom("workspace_sheet_figure")
                            .exact_size(height)
                            .frame(tokens::side_pane(&ctx))
                            .show_pane(ui, |ui| app.figure_pane(ui));
                    }
                    // The figure panel draws the pane_border between them.
                    egui::Panel::top("workspace_sheet_values")
                        .show_separator_line(false)
                        .exact_size(ui.available_height())
                        .frame(tokens::side_pane(&ctx))
                        .show(ui, |ui| app.workspace_dock_contents(ui));
                });
        });
}
