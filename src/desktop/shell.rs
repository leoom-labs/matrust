//! Native desktop chrome; no model, file, worker or document ownership.
use super::egui::{self, Rect, Response, Ui, Vec2};
use super::tokens::{self, palette};

pub(super) fn title(ui: &mut Ui, title: &str) -> Response {
    let font = tokens::semibold(tokens::BODY);
    let text = egui::RichText::new(title)
        .font(font.clone())
        .color(palette(ui.ctx()).text);
    let width = ui
        .painter()
        .layout_no_wrap(title.to_owned(), font, palette(ui.ctx()).text)
        .size()
        .x
        + 4.;
    ui.add_sized(Vec2::new(width, 20.), egui::Label::new(text))
}

#[derive(Clone, Copy)]
pub(super) enum Icon {
    Document,
    Folder,
    Save,
    Run,
    Table,
    Plot,
    Tools,
    Layout,
    Undo,
}

pub(super) fn active_header(ui: &Ui, active: bool) {
    let rect = Rect::from_min_size(
        ui.cursor().min,
        Vec2::new(ui.available_width(), ui.spacing().interact_size.y),
    );
    let palette = palette(ui.ctx());
    ui.painter().rect_filled(
        rect,
        0.,
        if active {
            palette.accent_soft
        } else {
            palette.surface
        },
    );
}
