//! Phosphor Regular 2.1.2 (MIT), embedded as a dedicated icon-only font.
//! See assets/fonts/phosphor-subset.txt and tool/subset-phosphor.py.
use super::egui::{self, Color32, Rect, Ui};

pub(super) const FAMILY: &str = "phosphor";

#[derive(Clone, Copy)]
pub(super) enum Icon {
    Export,
    SlidersHorizontal,
    FlowArrow,
    ArrowUUpRight,
    Copy,
    Terminal,
    Code,
    ChartLine,
    SquaresFour,
    FilePlus,
    FolderOpen,
    FloppyDisk,
    Play,
    Table,
    Sidebar,
    ArrowUUpLeft,
    FileCode,
    FileCsv,
    Image,
    ArrowClockwise,
    Funnel,
    DotsThree,
    MagnifyingGlass,
}

impl Icon {
    fn glyph(self) -> char {
        match self {
            Self::Export => '\u{eaf0}',
            Self::SlidersHorizontal => '\u{e434}',
            Self::FlowArrow => '\u{e6ec}',
            Self::ArrowUUpRight => '\u{e08c}',
            Self::Copy => '\u{e1ca}',
            Self::Terminal => '\u{e47e}',
            Self::Code => '\u{e1bc}',
            Self::ChartLine => '\u{e154}',
            Self::SquaresFour => '\u{e464}',
            Self::FilePlus => '\u{e236}',
            Self::FolderOpen => '\u{e256}',
            Self::FloppyDisk => '\u{e248}',
            Self::Play => '\u{e3d0}',
            Self::Table => '\u{e476}',
            Self::Sidebar => '\u{eab6}',
            Self::ArrowUUpLeft => '\u{e08a}',
            Self::FileCode => '\u{e914}',
            Self::FileCsv => '\u{eb1c}',
            Self::Image => '\u{e2ca}',
            Self::ArrowClockwise => '\u{e036}',
            Self::Funnel => '\u{e266}',
            Self::DotsThree => '\u{e1fe}',
            Self::MagnifyingGlass => '\u{e30c}',
        }
    }
}

/// One glyph as button text (e.g. an atom beside a label). Bare headless
/// contexts may omit the icon family; they fall back to Proportional.
pub(super) fn text(ui: &Ui, icon: Icon, size: f32) -> egui::RichText {
    let family = egui::FontFamily::Name(FAMILY.into());
    let family = if ui.fonts(|f| f.definitions().families.contains_key(&family)) {
        family
    } else {
        egui::FontFamily::Proportional
    };
    egui::RichText::new(icon.glyph().to_string()).font(egui::FontId::new(size, family))
}

/// Centre the font's ink in the existing hit box; icons never change layout.
pub(super) fn paint(ui: &Ui, rect: Rect, icon: Icon, color: Color32) {
    let family = egui::FontFamily::Name(FAMILY.into());
    // Bare headless contexts may omit desktop font configuration.
    let family = if ui.fonts(|f| f.definitions().families.contains_key(&family)) {
        family
    } else {
        egui::FontFamily::Proportional
    };
    let galley = ui.painter().layout_no_wrap(
        icon.glyph().to_string(),
        egui::FontId::new(rect.height(), family),
        color,
    );
    let origin = rect.center() - galley.mesh_bounds.center().to_vec2();
    ui.painter().galley(origin, galley, color);
}
