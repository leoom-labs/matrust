//! Bounded workspace metadata layout; reads at most ten array elements.
use super::super::*;

/// Most elements a vector shows in the VALUE column (MATLAB's workspace
/// browser: `[1 2 3]`); longer vectors and matrices show their size and class.
const VECTOR_ELEMENTS: usize = 10;

/// One real element as `format short` prints it alone (`16`, `0.8147`,
/// `1.2346e+05`).
pub(super) fn format_short(value: f64) -> String {
    matrust::display_format::disp(
        &matrust::Matrix::scalar(value).into(),
        &matrust::display_format::DisplayFormat::default(),
    )
    .ok()
    .flatten()
    .map_or_else(|| value.to_string(), |text| text.trim().to_owned())
}

/// Bounded VALUE column text: a scalar's value, a vector of up to ten
/// elements in `format short`, otherwise `<rows>x<cols> <class>`. Reads at
/// most ten elements or 32 characters.
pub(super) fn value_preview(value: &matrust::Array) -> String {
    // A column vector reads as MATLAB writes it: [1;2;3].
    let separator = if value.cols() == 1 { ";" } else { " " };
    let list = |items: &mut dyn Iterator<Item = String>| {
        format!("[{}]", items.collect::<Vec<_>>().join(separator))
    };
    let vector =
        (value.rows() == 1 || value.cols() == 1) && (2..=VECTOR_ELEMENTS).contains(&value.len());
    match value {
        matrust::Array::Numeric(m) if m.len() == 1 => format_short(m.data()[0]),
        matrust::Array::Numeric(m) if vector => {
            list(&mut m.data().iter().map(|v| format_short(*v)))
        }
        matrust::Array::Typed(t) if t.len() == 1 || vector => {
            let one = |i: usize| {
                if t.is_integer() {
                    t.int(i).to_string()
                } else {
                    format_short(t.real(i))
                }
            };
            if t.len() == 1 {
                one(0)
            } else {
                list(&mut (0..t.len()).map(one))
            }
        }
        matrust::Array::Logical(m) if m.len() == 1 => (m.data()[0] != 0).to_string(),
        matrust::Array::Logical(m) if vector => {
            list(&mut m.data().iter().map(|v| u8::from(*v != 0).to_string()))
        }
        matrust::Array::Complex(m) if m.len() == 1 => {
            let z = m.data()[0];
            format!(
                "{}{}{}i",
                format_short(z.re),
                if z.im < 0. { "-" } else { "+" },
                format_short(z.im.abs())
            )
        }
        matrust::Array::Character(c) if c.rows() <= 1 => {
            let units = c.units();
            let text = String::from_utf16_lossy(&units[..units.len().min(32)]);
            format!("'{text}{}'", if units.len() > 32 { "..." } else { "" })
        }
        _ if value.rows() == 0 || value.cols() == 0 => "[]".into(),
        matrust::Array::Complex(_) => {
            format!("{}x{} double (complex)", value.rows(), value.cols())
        }
        _ => format!(
            "{}x{} {}",
            value.rows(),
            value.cols(),
            workspace_type_label(value)
        ),
    }
}

/// Row tooltip: full name, class, size and the untruncated value preview.
pub(super) fn hover_text(name: &str, value: &matrust::Array) -> String {
    format!(
        "{name}\n{}, {} x {}\n{}",
        workspace_type_label(value),
        value.rows(),
        value.cols(),
        value_preview(value)
    )
}

/// Narrowest NAME and VALUE columns; both stay whenever the list is shown.
const MIN_COLUMN: f32 = 96.;

/// Includes both text columns, their gap, row padding and the scroll gutter.
pub(super) fn minimum_table_width() -> f32 {
    2. * MIN_COLUMN + tokens::S + 2. * tokens::M + theme::scroll_style().allocated_width()
}

/// Shortens `text` to fit `width`, keeping its start and end around one `…`,
/// so `var_long_name_12` and `var_long_name_13` stay distinguishable.
fn truncate_middle(ui: &egui::Ui, text: &str, font: &egui::FontId, width: f32) -> String {
    let measure = |text: String| {
        ui.painter()
            .layout_no_wrap(text, font.clone(), Color32::WHITE)
            .size()
            .x
    };
    if measure(text.to_owned()) <= width {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let shortened = |keep: usize| {
        let tail = keep / 2;
        let head = keep - tail;
        chars[..head]
            .iter()
            .chain(std::iter::once(&'…'))
            .chain(&chars[chars.len() - tail..])
            .collect::<String>()
    };
    // The widest kept prefix and suffix that still fit; at least the ellipsis.
    let (mut fits, mut over) = (0, chars.len());
    while over - fits > 1 {
        let keep = (fits + over) / 2;
        if measure(shortened(keep)) <= width {
            fits = keep;
        } else {
            over = keep;
        }
    }
    shortened(fits)
}

/// One line per variable like MATLAB: NAME | VALUE | SIZE chip. When space is
/// short the SIZE column goes first; NAME and VALUE never disappear.
#[derive(Clone, Copy)]
pub(super) struct Layout {
    width: f32,
    pub(super) height: f32,
    show_size: bool,
    size_width: f32,
}
impl Layout {
    pub(super) fn new(ui: &egui::Ui) -> Self {
        let width = (ui.available_width() - ui.spacing().scroll.allocated_width()).max(1.);
        let size_width = ui
            .painter()
            .layout_no_wrap(
                "1x1000".into(),
                egui::FontId::monospace(tokens::CAPTION),
                Color32::WHITE,
            )
            .size()
            .x
            + 12.;
        let height = if tokens::compact(ui.ctx()) {
            tokens::COMPACT_ROW
        } else {
            tokens::ROW
        }
        .max(ui.text_style_height(&egui::TextStyle::Body) + 4.);
        let inner = width - 2. * tokens::M;
        Self {
            width,
            height,
            show_size: inner >= 2. * MIN_COLUMN + size_width + 2. * tokens::S,
            size_width,
        }
    }
    fn columns(self, rect: egui::Rect) -> [egui::Rect; 3] {
        let inner = rect.shrink2(egui::vec2(tokens::M, 0.));
        let size_left = if self.show_size {
            inner.right() - self.size_width
        } else {
            inner.right() + tokens::S
        };
        // NAME takes a third of the text columns, at least MIN_COLUMN while
        // VALUE keeps its own minimum; below both minimums they share evenly.
        let text = size_left - tokens::S - inner.left();
        let name = if text >= 2. * MIN_COLUMN + tokens::S {
            (text / 3.).clamp(MIN_COLUMN, text - tokens::S - MIN_COLUMN)
        } else {
            (text - tokens::S) * 0.5
        };
        let name_right = inner.left() + name.max(0.) + tokens::S;
        let row = |left: f32, right: f32| {
            egui::Rect::from_min_max(
                egui::pos2(left, rect.top()),
                egui::pos2(right.max(left), rect.bottom()),
            )
        };
        [
            row(inner.left(), name_right - tokens::S),
            row(name_right, size_left - tokens::S),
            row(size_left, inner.right()),
        ]
    }
    fn text(ui: &egui::Ui, text: &str, rect: egui::Rect, font: egui::FontId, color: Color32) {
        let mut job = egui::text::LayoutJob::single_section(
            text.to_owned(),
            egui::TextFormat::simple(font, color),
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width().max(0.));
        job.wrap.overflow_character = Some('…');
        let galley = ui.painter().layout_job(job);
        let origin = egui::pos2(rect.left(), rect.center().y - galley.size().y * 0.5)
            - galley.mesh_bounds.min.to_vec2().min(egui::Vec2::ZERO);
        ui.painter()
            .with_clip_rect(ui.clip_rect().intersect(rect))
            .galley(origin, galley, color);
    }
    pub(super) fn header(self, ui: &mut egui::Ui) {
        let p = tokens::palette(ui.ctx());
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(self.width, self.height), egui::Sense::hover());
        let [name, value, size] = self.columns(rect);
        let font = panels::caption_semibold(ui);
        Self::text(ui, "NAME", name, font.clone(), p.text_muted);
        Self::text(ui, "VALUE", value, font.clone(), p.text_muted);
        if self.show_size {
            Self::text(ui, "SIZE", size, font, p.text_muted);
        }
    }
    pub(super) fn row(
        self,
        ui: &mut egui::Ui,
        name: &str,
        value: &matrust::Array,
        selected: bool,
    ) -> egui::Response {
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(self.width, self.height), egui::Sense::click());
        let dimensions = format!("{}x{}", value.rows(), value.cols());
        let description = format!(
            "{name}, {} x {}, {}",
            value.rows(),
            value.cols(),
            workspace_type_label(value)
        );
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Button,
                ui.is_enabled(),
                selected,
                &description,
            )
        });
        if ui.is_rect_visible(rect) {
            let p = tokens::palette(ui.ctx());
            let fill = if selected {
                p.accent_soft
            } else if response.hovered() {
                p.surface_sunken
            } else {
                Color32::TRANSPARENT
            };
            ui.painter().rect_filled(rect, 0., fill);
            ui.painter().line_segment(
                [rect.left_top(), rect.right_top()],
                egui::Stroke::new(1., p.border),
            );
            panels::focus_ring(ui, &response);
            let [name_rect, value_rect, size_rect] = self.columns(rect);
            let body = ui.text_style_height(&egui::TextStyle::Body);
            let name_color = if selected { p.accent_text } else { p.text };
            // A large dimension label may exceed the usual SIZE column. Drop
            // that chip before it can take VALUE below its minimum.
            let chip_fits = self.show_size && {
                let chip_width = (ui
                    .painter()
                    .layout_no_wrap(
                        dimensions.clone(),
                        egui::FontId::monospace(tokens::CAPTION),
                        p.text_muted,
                    )
                    .size()
                    .x
                    + tokens::M)
                    .max(tokens::SIZE_CHIP_MIN_WIDTH);
                size_rect.right() - chip_width - tokens::S >= value_rect.left() + MIN_COLUMN
            };
            let chip_left = if chip_fits {
                panels::chip(
                    ui,
                    egui::pos2(size_rect.right(), size_rect.center().y),
                    &dimensions,
                    rect,
                )
                .left()
                    - tokens::S
            } else {
                f32::INFINITY
            };
            let name_rect = name_rect.with_max_x(name_rect.right().min(chip_left));
            let name_font = panels::semibold_font(ui, body.min(tokens::BODY));
            let shown = truncate_middle(ui, name, &name_font, name_rect.width());
            Self::text(ui, &shown, name_rect, name_font, name_color);
            let value_rect = value_rect.with_max_x(value_rect.right().min(chip_left));
            Self::text(
                ui,
                &value_preview(value),
                value_rect,
                egui::FontId::monospace(tokens::SMALL_CODE),
                if selected {
                    p.accent_text
                } else {
                    p.text_muted
                },
            );
        }
        // The full name, class, size and the whole bounded preview; built
        // only while the tooltip is actually shown.
        response.on_hover_ui(|ui| {
            ui.label(hover_text(name, value));
        })
    }
}
