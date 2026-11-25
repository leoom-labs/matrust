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

// Corner radii: panes are flat (0), controls and buttons 4, chips 4.
pub(super) const PANEL_RADIUS: u8 = 0;
pub(super) const CONTROL_RADIUS: u8 = 4;
pub(super) const CHIP_RADIUS: u8 = 4;

/// Top bar (brand, mode switcher, search, Menu, Help) on surface 3.
pub(super) const TOP_BAR: f32 = 40.;
/// Action row of the current mode, on surface 2.
pub(super) const ACTION_ROW: f32 = 36.;
/// Height of a control (button, input, select).
pub(super) const CONTROL: f32 = 28.;
/// Panel header: title, count and ghost tool links.
pub(super) const PANEL_HEADER: f32 = 32.;
/// Status bar on surface 3.
pub(super) const STATUS_BAR: f32 = 24.;
/// Table and list rows (COMPACT_ROW in compact windows).
pub(super) const ROW: f32 = 24.;
/// Command window input field.
pub(super) const INPUT_HEIGHT: f32 = 32.;
/// Command window text: mono 13.5 px on a 1.6 line height.
pub(super) const CONSOLE: f32 = 13.5;
pub(super) const CONSOLE_LINE: f32 = 1.6;
/// Body columns at the 1366 px reference width.
pub(super) const LEFT_COLUMN: f32 = 240.;
pub(super) const RIGHT_COLUMN: f32 = 300.;
/// Navigator width in compact windows.
pub(super) const LEFT_COLUMN_COMPACT: f32 = 200.;
/// Inspector width in compact windows.
pub(super) const RIGHT_COLUMN_COMPACT: f32 = 264.;
pub(super) const SEARCH_WIDTH: f32 = 320.;
/// Below this width hide the folder crumb and use an icon for search.
pub(super) const SEARCH_ICON_WIDTH: f32 = 900.;
/// Minimum width for workspace SIZE chips, independent of the dimensions text.
pub(super) const SIZE_CHIP_MIN_WIDTH: f32 = 48.;
/// Below this window width the last toolstrip groups move into "More".
pub(super) const OVERFLOW_WIDTH: f32 = 1100.;

/// A work-area pane: flat surface 1, no outline, no radius. Neighbouring
/// panes are told apart by the 1 px `pane_border` on their shared edge.
pub(super) fn card(ctx: &egui::Context) -> egui::Frame {
    egui::Frame::new().fill(palette(ctx).surface)
}

/// Plan section 9 single-line input: 28 px high, 8 px inner padding, text
/// centred vertically (egui's default TextEdit is 20 px).
pub(super) fn input(edit: egui::TextEdit<'_>) -> egui::TextEdit<'_> {
    edit.min_size(egui::vec2(0., CONTROL))
        .margin(egui::Margin::symmetric(S as i8, XS as i8))
        .vertical_align(egui::Align::Center)
}

/// A side pane (Navigator, Inspector): flat surface 2.
pub(super) fn side_pane(ctx: &egui::Context) -> egui::Frame {
    egui::Frame::new().fill(palette(ctx).surface_sunken)
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

/// Toggle (plan section 9): 28 px with the normal 1 px control border; on =
/// selection tint with text 1, off = surface 1 with text 2.
/// No accent stroke and no pill: the same rule in every pane and preview.
pub(super) fn toggle(ui: &mut egui::Ui, on: bool, text: &str) -> egui::Response {
    let p = palette(ui.ctx());
    // On = selection tint + text 1 (the list rule); no accent ink or stroke.
    let (fill, ink) = if on {
        (p.accent_soft, p.text)
    } else {
        (p.surface, p.text_muted)
    };
    ui.add(
        egui::Button::selectable(on, egui::RichText::new(text).color(ink))
            .frame_when_inactive(true)
            .fill(fill)
            .stroke(egui::Stroke::new(1., p.border))
            .corner_radius(CONTROL_RADIUS)
            .min_size(egui::vec2(0., CONTROL)),
    )
}

/// Tab or segment (plan section 9): 32 px, text 2; active = text 1 with a
/// 2 px accent underline. No fill and no frame.
pub(super) fn tab(ui: &mut egui::Ui, on: bool, text: &str) -> egui::Response {
    let p = palette(ui.ctx());
    let ink = if on { p.text } else { p.text_muted };
    let response = ui.add(
        egui::Button::new(egui::RichText::new(text).color(ink))
            .frame(false)
            .selected(on)
            .min_size(egui::vec2(0., PANEL_HEADER)),
    );
    if on {
        ui.painter().hline(
            response.rect.x_range(),
            response.rect.bottom() - 1.,
            egui::Stroke::new(2., p.accent),
        );
    }
    response
}

pub(super) fn swatch_stroke(ui: &egui::Ui) -> egui::Stroke {
    egui::Stroke::new(1., palette(ui.ctx()).border)
}

pub(super) fn color_swatch(
    ui: &mut egui::Ui,
    size: f32,
    color: Color32,
    sense: egui::Sense,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(size), sense);
    ui.painter()
        .rect(rect, 2., color, swatch_stroke(ui), egui::StrokeKind::Inside);
    response
}

pub(super) fn color_edit_button(ui: &mut egui::Ui, color: &mut [u8; 3]) -> egui::Response {
    let response = ui.color_edit_button_srgb(color);
    ui.painter().rect_stroke(
        response.rect,
        2.,
        swatch_stroke(ui),
        egui::StrokeKind::Inside,
    );
    response
}

/// Helper text: body 13 in text 2 (the type scale has no 11 px regular).
pub(super) fn helper(ui: &mut egui::Ui, text: impl Into<String>) -> egui::Response {
    let ink = palette(ui.ctx()).text_muted;
    ui.label(egui::RichText::new(text.into()).size(BODY).color(ink))
}

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
    /// Surface 3: top bar, status bar and the drawer header.
    pub canvas: Color32,
    /// Surface 1: work area, console, editor, canvas and inputs.
    pub surface: Color32,
    /// Surface 2: Navigator, Inspector, action row, dialogs and hover tints.
    pub surface_sunken: Color32,
    /// 1 px outline of controls and inputs (>= 3:1 on every surface).
    pub border: Color32,
    /// 1 px divider inside a pane: header rule, table rows, action groups.
    pub hairline: Color32,
    /// 1 px edge between two panes (and around floating sheets): >= 3:1
    /// against canvas, surface 1 and surface 2, so panes never blend.
    pub pane_border: Color32,
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

/// Light: rust accent on cool neutrals (plan fable, section 7).
pub(super) static LIGHT: Palette = Palette {
    canvas: hex(0xE6E8EC),
    surface: hex(0xFFFFFF),
    surface_sunken: hex(0xF3F4F6),
    border: hex(0x7C848E),
    hairline: hex(0xD9DDE3),
    // The control outline: 3.8 / 3.4 / 3.1 : 1 on surface 1 / 2 / canvas.
    pane_border: hex(0x7C848E),
    text: hex(0x1A1D23),
    text_muted: hex(0x4B5563),
    text_disabled: hex(0x626974),
    accent: hex(0xB23A0B),
    accent_fill: hex(0xB23A0B),
    accent_hover: hex(0x9A3209),
    accent_soft: hex(0xF9E4D8),
    accent_text: hex(0x9A3209),
    on_accent: hex(0xFFFFFF),
    error: hex(0xB42318),
    // #B26A00 was 3.9:1 on surface_sunken.
    warning: hex(0x7A4A00),
    success: hex(0x15703A),
    syntax: Syntax {
        plain: Color32::from_rgb(40, 44, 52),
        comment: Color32::from_rgb(36, 110, 59),
        string: Color32::from_rgb(160, 45, 105),
        number: Color32::from_rgb(150, 75, 25),
        keyword: Color32::from_rgb(0, 80, 180),
        function: Color32::from_rgb(0, 105, 150),
        operator: Color32::from_rgb(75, 80, 90),
    },
};

pub(super) static DARK: Palette = Palette {
    canvas: hex(0x121418),
    surface: hex(0x1E2128),
    surface_sunken: hex(0x181B20),
    border: hex(0x6B7380),
    hairline: hex(0x2C3139),
    // The control outline: 3.4 / 3.6 / 3.9 : 1 on surface 1 / 2 / canvas.
    pane_border: hex(0x6B7380),
    text: hex(0xE8EAED),
    text_muted: hex(0xA8AFB8),
    text_disabled: hex(0x8E959F),
    accent: hex(0xF08A4B),
    accent_fill: hex(0xC2410C),
    accent_hover: hex(0x9A3412),
    accent_soft: hex(0x3B2318),
    accent_text: hex(0xF5A775),
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
