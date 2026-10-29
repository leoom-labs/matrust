//! Desktop chrome design tokens: colours, type scale and spacing. Figure and
//! plot artwork keeps its own document colours and never reads these.
use super::egui::{self, Color32};

// Type scale: the only text sizes used by chrome.
pub(super) const CAPTION: f32 = 11.;
pub(super) const BODY: f32 = 13.;
pub(super) const HEADING: f32 = 15.;
pub(super) const TITLE: f32 = 18.;
pub(super) const CODE: f32 = 13.;
pub(super) const EDITOR: f32 = 14.;
/// Paths and size chips in mono.
pub(super) const SMALL_CODE: f32 = 12.;

/// Inter SemiBold, used for HEADING and TITLE text.
pub(super) const SEMIBOLD: &str = "semibold";

// Spacing on a 4 px grid.
pub(super) const XS: f32 = 4.;
pub(super) const S: f32 = 8.;
#[allow(dead_code)]
pub(super) const M: f32 = 12.;
pub(super) const L: f32 = 16.;
#[allow(dead_code)]
pub(super) const XL: f32 = 24.;

/// Minimum interact height of normal chrome rows.
pub(super) const INTERACT_HEIGHT: f32 = 24.;

// Corner radii: panel cards, controls and buttons, chips.
pub(super) const PANEL_RADIUS: u8 = 8;
pub(super) const CONTROL_RADIUS: u8 = 6;
pub(super) const CHIP_RADIUS: u8 = 4;

/// Gap between panel cards and from the cards to the window edge.
pub(super) const GUTTER: f32 = S;
/// Top bar (brand, tabs, search, Run) on the canvas.
pub(super) const TOP_BAR: f32 = 44.;
/// Panel header: title, count and ghost tool links.
pub(super) const PANEL_HEADER: f32 = 36.;
/// Status bar on the canvas.
pub(super) const STATUS_BAR: f32 = 26.;
/// Table and list rows (COMPACT_ROW in compact windows).
pub(super) const ROW: f32 = 26.;
/// Command window input field.
pub(super) const INPUT_HEIGHT: f32 = 34.;
/// Command window text: mono 13.5 px on a 1.6 line height.
pub(super) const CONSOLE: f32 = 13.5;
pub(super) const CONSOLE_LINE: f32 = 1.6;
/// Body columns at the 1366 px reference width.
pub(super) const LEFT_COLUMN: f32 = 264.;
pub(super) const RIGHT_COLUMN: f32 = 420.;
pub(super) const SEARCH_WIDTH: f32 = 260.;
/// Below this width hide the folder crumb and use an icon for search.
pub(super) const SEARCH_ICON_WIDTH: f32 = 900.;
/// Minimum width for workspace SIZE chips, independent of the dimensions text.
pub(super) const SIZE_CHIP_MIN_WIDTH: f32 = 48.;
/// Icon buttons of the Files card header: a 16 px glyph with 3 px padding,
/// so its seven tools and the title fit a 240 px column.
pub(super) const HEADER_ICON: f32 = 22.;
/// Toolstrip icons are 20 px strokes.
pub(super) const ICON: f32 = 20.;
/// Below this window width the last toolstrip groups move into "More".
pub(super) const OVERFLOW_WIDTH: f32 = 1100.;

/// A panel card: surface fill, 1 px border, radius 8; the gutter around it
/// shows the canvas.
pub(super) fn card(ctx: &egui::Context) -> egui::Frame {
    let p = palette(ctx);
    egui::Frame::new()
        .fill(p.surface)
        .stroke(egui::Stroke::new(1., p.border))
        .corner_radius(PANEL_RADIUS)
        .outer_margin(egui::Margin::same((GUTTER / 2.) as i8))
}

/// Row height of dense chrome in compact windows: one CAPTION line plus XS
/// padding above and below (normal windows keep the 24 px interact height).
pub(super) const COMPACT_ROW: f32 = 22.;

/// Compact windows are narrower than 1100 px or shorter than 760 px.
pub(super) fn compact(ctx: &egui::Context) -> bool {
    let screen = ctx.content_rect();
    screen.width() < 1100. || screen.height() < 760.
}

/// Tiny windows retain tab menus; the toolstrip stays visible from 800 px up.
pub(super) fn short(ctx: &egui::Context) -> bool {
    let screen = ctx.content_rect();
    screen.width() < 800. && screen.height() < 680.
}

/// Denser chrome for compact windows: CAPTION text and `COMPACT_ROW` rows. The
/// change stays inside `ui`; the global style is untouched.
pub(super) fn dense(ui: &mut egui::Ui) {
    let style = ui.style_mut();
    style.override_text_style = Some(egui::TextStyle::Small);
    style.spacing.interact_size.y = COMPACT_ROW;
}

/// The active-subplot outline is drawn over figure artwork, which keeps its
/// pre-token colour.
pub(super) const FIGURE_ACCENT: Color32 = Color32::from_rgb(0, 114, 189);

pub(super) fn semibold(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(SEMIBOLD.into()))
}

/// Dialog title or welcome heading: TITLE, Inter SemiBold. Panel and tool
/// titles stay on `TextStyle::Heading` (HEADING).
pub(super) fn title(ctx: &egui::Context, text: impl Into<String>) -> egui::RichText {
    let family = egui::FontFamily::Name(SEMIBOLD.into());
    // A context that never ran `configure` (tests) has no SemiBold family.
    let font = if ctx.fonts(|f| f.definitions().families.contains_key(&family)) {
        semibold(TITLE)
    } else {
        egui::FontId::proportional(TITLE)
    };
    egui::RichText::new(text).font(font)
}

pub(super) struct Syntax {
    pub plain: Color32,
    pub comment: Color32,
    pub string: Color32,
    pub number: Color32,
    pub keyword: Color32,
    pub function: Color32,
    pub operator: Color32,
}

pub(super) struct Palette {
    /// Window background behind the panel cards, top bar and status bar.
    pub canvas: Color32,
    /// Panel cards, toolstrip, search field and editor.
    pub surface: Color32,
    /// Command window body, hover, chips and welcome cards.
    pub surface_sunken: Color32,
    /// 1 px panel outline, dividers and table row lines.
    pub border: Color32,
    pub text: Color32,
    /// Values, counts, secondary labels and ghost buttons.
    pub text_muted: Color32,
    /// Placeholders and disabled controls only, never enabled text.
    pub text_disabled: Color32,
    /// Prompt, Run icon, focus ring, selection marker and input outline.
    pub accent: Color32,
    /// Filled primary button background (on_accent label).
    pub accent_fill: Color32,
    pub accent_hover: Color32,
    /// Selected row and the "Try an example" card.
    pub accent_soft: Color32,
    /// Text on accent_soft.
    pub accent_text: Color32,
    pub on_accent: Color32,
    pub error: Color32,
    pub warning: Color32,
    pub success: Color32,
    pub syntax: Syntax,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// Warm light: rust accent on warm neutrals.
pub(super) static LIGHT: Palette = Palette {
    canvas: hex(0xE9E6E1),
    surface: hex(0xFFFFFF),
    surface_sunken: hex(0xF7F5F2),
    border: hex(0xDDD8D0),
    text: hex(0x1C1917),
    text_muted: hex(0x57534E),
    text_disabled: hex(0x8B857D),
    accent: hex(0xC2410C),
    accent_fill: hex(0xC2410C),
    accent_hover: hex(0x9A3412),
    accent_soft: hex(0xFDEEE5),
    accent_text: hex(0x9A3412),
    on_accent: hex(0xFFFFFF),
    error: hex(0xC62828),
    // #B26A00 was 3.9:1 on surface_sunken.
    warning: hex(0xA35A00),
    success: hex(0x15803D),
    syntax: Syntax {
        plain: Color32::from_rgb(40, 44, 52),
        comment: Color32::from_rgb(40, 120, 65),
        string: Color32::from_rgb(160, 45, 105),
        number: Color32::from_rgb(150, 75, 25),
        keyword: Color32::from_rgb(0, 80, 180),
        function: Color32::from_rgb(0, 105, 150),
        operator: Color32::from_rgb(75, 80, 90),
    },
};

pub(super) static DARK: Palette = Palette {
    canvas: hex(0x121418),
    surface: hex(0x1B1E23),
    surface_sunken: hex(0x16181C),
    border: hex(0x2A2E35),
    text: hex(0xE6E8EB),
    text_muted: hex(0xA0A7B2),
    text_disabled: hex(0x717884),
    accent: hex(0xEA6A2C),
    accent_fill: hex(0xC2410C),
    accent_hover: hex(0xF07A3E),
    accent_soft: hex(0x3A2117),
    accent_text: hex(0xF59E6B),
    on_accent: hex(0xFFFFFF),
    error: hex(0xF28B82),
    warning: hex(0xF2B661),
    success: hex(0x4ADE80),
    syntax: Syntax {
        plain: Color32::from_rgb(224, 227, 232),
        comment: Color32::from_rgb(133, 190, 128),
        string: Color32::from_rgb(224, 151, 204),
        number: Color32::from_rgb(226, 181, 127),
        keyword: Color32::from_rgb(114, 179, 245),
        function: Color32::from_rgb(109, 205, 211),
        operator: Color32::from_rgb(191, 198, 209),
    },
};

/// Chrome palette for the global appearance. Plot documents may scope their
/// own visuals to Light; chrome inside them still follows the desktop theme.
pub(super) fn palette(ctx: &egui::Context) -> &'static Palette {
    palette_for(ctx.global_style().visuals.dark_mode)
}

pub(super) fn palette_for(dark: bool) -> &'static Palette {
    if dark { &DARK } else { &LIGHT }
}
