//! Bounded discovery of existing desktop actions. No document contents are retained.
use super::*;
use crate::theme::ThemedScrollArea;
use matrust::session::DesktopTheme;

const MAX_QUERY_CHARS: usize = 128;
const MAX_QUERY_BYTES: usize = 512;
/// Palette card width before clamping to the window.
const WIDTH: f32 = 640.;
/// Result row height and the rows shown before the list scrolls.
const ROW: f32 = 28.;
const MAX_ROWS: usize = 10;
/// Query characters quoted in the empty-result line.
const QUOTED_QUERY_CHARS: usize = 24;

pub(super) fn popup_id() -> egui::Id {
    egui::Id::new("desktop_action_search")
}
pub(super) fn query_id() -> egui::Id {
    popup_id().with("query")
}
pub(super) fn list_id() -> egui::Id {
    popup_id().with("results")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Entry {
    Action(toolstrip::Action),
    Theme(DesktopTheme),
    Scale(u16),
    Help,
    UndockFigure,
    Settings,
}
impl Entry {
    fn label(self) -> &'static str {
        match self {
            Self::Action(action) => action.label(),
            Self::Theme(DesktopTheme::Light) => "Appearance: Light",
            Self::Theme(DesktopTheme::Dark) => "Appearance: Dark",
            Self::Theme(DesktopTheme::System) => "Appearance: Use system setting",
            Self::Scale(75) => "Desktop scale: 75%",
            Self::Scale(100) => "Desktop scale: 100%",
            Self::Scale(125) => "Desktop scale: 125%",
            Self::Scale(_) => "Desktop scale: 150%",
            Self::UndockFigure => "Undock figure",
            Self::Settings => "Settings",
            Self::Help => "Help: MatRust syntax and supported commands",
        }
    }
    /// Extra words the entry answers to; the label stays as shown.
    fn keywords(self) -> &'static str {
        match self {
            Self::Theme(_) => "theme dark light appearance mode colour color",
            Self::Scale(_) => "zoom size",
            Self::Settings => "preferences options theme scale workspace",
            _ => "",
        }
    }
    /// Existing keyboard shortcut, shown right-aligned in the result row.
    fn shortcut(self) -> Option<&'static str> {
        match self {
            Self::Action(toolstrip::Action::NewScript) => Some("Ctrl+N"),
            Self::Action(toolstrip::Action::RunScript) => Some("F5"),
            Self::Action(toolstrip::Action::RunSelection) => Some("F9"),
            _ => None,
        }
    }
    fn category(self) -> &'static str {
        match self {
            Self::Action(_) | Self::UndockFigure => "Action",
            Self::Theme(_) | Self::Scale(_) | Self::Settings => "Setting",
            Self::Help => "Help",
        }
    }
    fn enabled(self, app: &Workspace) -> bool {
        match self {
            Self::Action(action) => action.enabled_for_search(app),
            Self::UndockFigure => {
                !app.busy && app.figure.is_some() && !app.detached.contains(app.current_figure)
            }
            _ => !app.busy,
        }
    }
    fn invoke(self, app: &mut Workspace, ctx: &egui::Context) {
        match self {
            Self::Action(action) => action.invoke(app, ctx),
            Self::Theme(theme) => app.set_desktop_theme(ctx, theme),
            Self::Scale(scale) => app.set_desktop_scale(ctx, scale),
            Self::UndockFigure => app.undock_figure(app.current_figure, ctx),
            Self::Settings => theme::open_settings(ctx),
            Self::Help => {
                app.console.push(matrust::engine::HELP.into());
                app.queue_pane_focus(ctx, navigation::FocusPane::Command);
            }
        }
    }
}

#[cfg(test)]
pub(super) fn matching_labels(query: &str) -> Vec<&'static str> {
    matches(query).into_iter().map(Entry::label).collect()
}

fn matches(query: &str) -> Vec<Entry> {
    let query = query.trim().to_lowercase();
    let mut found = toolstrip::Action::ALL
        .into_iter()
        .chain(crate::apps::AppKind::built().map(toolstrip::Action::App))
        .map(Entry::Action)
        .chain([
            Entry::Theme(DesktopTheme::Light),
            Entry::Theme(DesktopTheme::Dark),
            Entry::Theme(DesktopTheme::System),
            Entry::Scale(75),
            Entry::Scale(100),
            Entry::Scale(125),
            Entry::Scale(150),
            Entry::Settings,
            Entry::UndockFigure,
            Entry::Help,
        ])
        .filter(|entry| {
            let label = entry.label().to_lowercase();
            // "dark mode" finds "Appearance: Dark": every word is in the label
            // or starts one of the entry's keywords.
            label.contains(&query)
                || entry.category().to_lowercase().contains(&query)
                || (!query.is_empty()
                    && query.split_whitespace().all(|word| {
                        label.contains(word)
                            || entry.keywords().split(' ').any(|k| k.starts_with(word))
                    }))
        })
        .collect::<Vec<_>>();
    // Entries whose label holds the query come first: "dark" selects
    // "Appearance: Dark", not "Appearance: Light" matched by a keyword.
    found.sort_by_key(|entry| !entry.label().to_lowercase().contains(&query));
    found
}

/// The palette dialog (plan sections 5, 9): surface 2 with a 1 px border,
/// 16 px padding and room above the body for the 24 px title bar on
/// surface 3 that [`paint_title_bar`] draws.
pub(super) fn card_frame(ctx: &egui::Context) -> egui::Frame {
    let pad = tokens::L as i8;
    crate::dialog_windows::dialog_frame(ctx).inner_margin(egui::Margin {
        left: pad,
        right: pad,
        top: (crate::dialog_windows::TITLE_BAR + tokens::L) as i8,
        bottom: pad,
    })
}

/// Title bar of the palette dialog, painted into the frame's top margin
/// above `body` (the padded content rect).
fn paint_title_bar(ui: &egui::Ui, body: egui::Rect) {
    let p = tokens::palette(ui.ctx());
    let bar = egui::Rect::from_min_max(
        egui::pos2(
            body.left() - tokens::L,
            body.top() - tokens::L - crate::dialog_windows::TITLE_BAR,
        ),
        egui::pos2(body.right() + tokens::L, body.top() - tokens::L),
    );
    let painter = ui.painter();
    painter.rect_filled(bar, 0., p.canvas);
    painter.hline(
        bar.x_range(),
        bar.bottom() - 0.5,
        egui::Stroke::new(1., p.hairline),
    );
    painter.text(
        bar.left_center() + egui::vec2(tokens::M, 0.),
        egui::Align2::LEFT_CENTER,
        "Search actions",
        tokens::semibold(tokens::BODY),
        p.text,
    );
    painter.text(
        bar.right_center() - egui::vec2(tokens::M, 0.),
        egui::Align2::RIGHT_CENTER,
        "Ctrl+K",
        egui::FontId::proportional(tokens::CAPTION),
        p.text_muted,
    );
}

/// "No matches for '<query>'", quoting at most 24 characters of the query.
pub(super) fn no_matches(query: &str) -> String {
    let query = query.trim();
    let mut quoted: String = query.chars().take(QUOTED_QUERY_CHARS).collect();
    if query.chars().nth(QUOTED_QUERY_CHARS).is_some() {
        quoted.push('…');
    }
    format!("No matches for '{quoted}'")
}

/// One result: BODY label at S padding, the shortcut right-aligned in CAPTION;
/// the selected or hovered row is filled with the full selection tint.
/// Disabled rows keep full-strength paint (no egui fade) in text 3.
fn result_row(
    ui: &mut egui::Ui,
    entry: Entry,
    selected: bool,
    enabled: bool,
    width: f32,
) -> egui::Response {
    let palette = tokens::palette(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, ROW), egui::Sense::CLICK);
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            entry.label(),
        )
    });
    if !ui.is_rect_visible(rect) {
        return response;
    }
    // A disabled Ui fades its painter; the row picks its own disabled ink.
    let mut painter = ui.painter().clone();
    painter.set_opacity(1.0);
    let highlighted = selected || (enabled && response.hovered());
    if highlighted {
        painter.rect_filled(rect, tokens::CONTROL_RADIUS, palette.accent_soft);
    }
    if selected {
        // 2 px accent bar at the left of the keyboard-selected row.
        let bar = egui::Rect::from_min_size(rect.min, egui::vec2(2., rect.height()));
        painter.rect_filled(bar, 0., palette.accent);
    }
    let muted = if enabled {
        palette.text_muted
    } else {
        palette.text_disabled
    };
    let inner = rect.shrink2(egui::vec2(tokens::S, 0.));
    let mut right = inner.right();
    if let Some(shortcut) = entry.shortcut() {
        let galley = painter.layout_no_wrap(
            shortcut.into(),
            egui::FontId::proportional(tokens::CAPTION),
            muted,
        );
        right -= galley.size().x;
        let at = egui::pos2(right, inner.center().y - galley.size().y / 2.);
        painter.galley(at, galley, muted);
        right -= tokens::S;
    }
    // Selected rows keep text 1 on the selection tint (plus the accent bar);
    // disabled rows are text 3.
    let color = if enabled {
        palette.text
    } else {
        palette.text_disabled
    };
    let mut job = egui::text::LayoutJob::single_section(
        entry.label().into(),
        egui::TextFormat::simple(egui::FontId::proportional(tokens::BODY), color),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width((right - inner.left()).max(0.));
    let galley = painter.layout_job(job);
    let at = egui::pos2(inner.left(), inner.center().y - galley.size().y / 2.);
    painter.galley(at, galley, color);
    response
}

struct Authority {
    document: u64,
    interaction: u64,
    figure: Option<(figure_target::Key, Option<u64>)>,
    selected: Option<String>,
    selected_value: Option<matrust::Array>,
    selection: Option<(usize, std::ops::Range<usize>)>,
    folder: PathBuf,
}
impl Authority {
    fn capture(app: &Workspace) -> Self {
        Self {
            document: app.documents[app.active].id,
            interaction: app.document_save.interaction_generation(),
            figure: app.figure.as_ref().map(|figure| {
                (
                    figure_target::Key {
                        number: app.current_figure,
                        identity: figure.identity.clone(),
                    },
                    app.figure_documents
                        .get(&app.current_figure)
                        .map(|d| d.revision),
                )
            }),
            selected_value: app
                .selected
                .as_ref()
                .and_then(|name| app.variables.get(name))
                .cloned(),
            selected: app.selected.clone(),
            selection: app.selection.clone(),
            folder: app.folder.clone(),
        }
    }
    fn valid(&self, app: &Workspace) -> bool {
        self.document == app.documents[app.active].id
            && self.interaction == app.document_save.interaction_generation()
            && self.selected == app.selected
            && self.selection == app.selection
            && self.folder == app.folder
            && match (
                &self.selected_value,
                app.selected
                    .as_ref()
                    .and_then(|name| app.variables.get(name)),
            ) {
                (None, None) => true,
                (Some(before), Some(now)) => before.same_snapshot(now),
                _ => false,
            }
            && match (&self.figure, &app.figure) {
                (None, None) => true,
                (Some((key, revision)), Some(figure)) => {
                    key.number == app.current_figure
                        && key.identity.same_object(&figure.identity)
                        && *revision
                            == app
                                .figure_documents
                                .get(&app.current_figure)
                                .map(|d| d.revision)
                }
                _ => false,
            }
    }
}

#[derive(Clone, Copy)]
enum Intent {
    Close,
    Execute(Entry),
}
#[derive(Default)]
pub(super) struct State {
    authority: Option<Authority>,
    anchor: Option<(egui::Rect, egui::LayerId)>,
    query: String,
    generation: u64,
    selected: usize,
    reveal: bool,
    /// Move a fresh selection off disabled rows (set when the list resets).
    preselect: bool,
    focus_query: bool,
    key_frame: Option<u64>,
    finished_frame: Option<u64>,
    pending: Option<(u64, u64, Intent)>,
}
impl State {
    #[cfg(test)]
    pub(super) fn anchor_rect(&self) -> Option<egui::Rect> {
        self.anchor.map(|(rect, _)| rect)
    }
    #[cfg(test)]
    pub(super) fn selected(&self) -> usize {
        self.selected
    }
    pub(super) fn is_open(&self) -> bool {
        self.authority.is_some()
    }
    fn queue(&mut self, ctx: &egui::Context, intent: Intent) -> bool {
        let frame = ctx.cumulative_frame_nr();
        if self.finished_frame == Some(frame) {
            return false;
        }
        if let Some((queued, _, action)) = self.pending
            && queued == frame
            && (matches!(action, Intent::Close) || !matches!(intent, Intent::Close))
        {
            return false;
        }
        // Preserve the first gesture through retries; Close may replace an Execute.
        self.pending = Some((frame, self.generation, intent));
        true
    }
    pub(super) fn cancel(&mut self) {
        if let Some((frame, _, _)) = self.pending {
            self.finished_frame = Some(frame);
        }
        self.authority = None;
        self.pending = None;
        self.anchor = None;
        self.focus_query = false;
    }
    pub(super) fn close(&mut self, ctx: &egui::Context) {
        self.cancel();
        egui::Popup::close_id(ctx, popup_id());
        // Never request the old editor's focus: another surface may now own it.
        ctx.memory_mut(|memory| {
            memory.surrender_focus(query_id());
            memory.surrender_focus(list_id());
        });
    }
}

impl Workspace {
    pub(super) fn observe_global_search(&mut self, ctx: &egui::Context) {
        if !self.global_search.is_open() {
            if egui::Popup::is_id_open(ctx, popup_id()) {
                self.global_search.close(ctx);
            }
            return;
        }
        let frame = ctx.cumulative_frame_nr();
        if let Some((queued, _, Intent::Execute(_))) = self.global_search.pending
            && (queued != frame
                || !ctx
                    .memory(|memory| memory.has_focus(query_id()) || memory.has_focus(list_id())))
        {
            // Preserve a newer owner's choice before its response can surrender focus
            // on the replayed pointer release. That release cannot renew consent.
            self.global_search.pending = None;
            self.global_search.finished_frame = Some(frame);
        }
    }
    fn global_search_available(&self, ctx: &egui::Context) -> bool {
        ctx.viewport_id() == egui::ViewportId::ROOT
            && !self.busy
            && !self.recovering_session
            && !self.allow_close
            && !self.file_action_pending_without_global_search()
            && !ctx.input(|input| input.viewport().close_requested())
            && ctx.memory(|memory| memory.top_modal_layer().is_none())
            && (!egui::Popup::is_any_open(ctx) || egui::Popup::is_id_open(ctx, popup_id()))
    }
    pub(super) fn global_search_launcher(&mut self, ui: &mut egui::Ui) {
        let enabled = self.global_search.is_open() || self.global_search_available(ui.ctx());
        let response = if ui.ctx().content_rect().width() < tokens::SEARCH_ICON_WIDTH {
            // Toolstrip buttons always carry text: icon and "Search".
            ui.add_enabled_ui(enabled, |ui| {
                panels::strip_action(ui, "Search", shell::Icon::Search, false)
            })
            .inner
        } else {
            panels::search_field(ui, "Search functions and actions", enabled)
        };
        response
            .clone()
            .on_hover_text("Search MatRust actions, appearance, scale and syntax help · Ctrl+K");
        let shortcut = enabled
            && !self.global_search.is_open()
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::K));
        if response.clicked() || shortcut {
            if self.global_search.is_open() {
                self.global_search.queue(ui.ctx(), Intent::Close);
            } else if self.global_search_available(ui.ctx()) {
                // Capture before moving focus to the palette; no text/sample snapshot.
                self.global_search.authority = Some(Authority::capture(self));
                self.global_search.anchor = Some((response.rect, response.layer_id));
                self.global_search.query.clear();
                self.global_search.selected = 0;
                self.global_search.preselect = true;
                self.global_search.generation = self.global_search.generation.wrapping_add(1);
                self.global_search.focus_query = true;
                self.global_search.reveal = true;
                self.global_search.pending = None;
                egui::Popup::open_id(ui.ctx(), popup_id());
            }
        }
    }

    pub(super) fn global_search_ui(&mut self, ctx: &egui::Context) {
        if !self.global_search.is_open() {
            return;
        }
        if !egui::Popup::is_id_open(ctx, popup_id())
            || ctx.memory(|memory| memory.top_modal_layer().is_some())
        {
            self.global_search.close(ctx);
            return;
        }
        let Some((anchor, layer)) = self.global_search.anchor else {
            return;
        };
        let allowed = self.global_search_available(ctx)
            && self
                .global_search
                .authority
                .as_ref()
                .is_some_and(|a| a.valid(self));
        // A centred 640 px command dialog (plan 5); the top-bar field only
        // opens it. It sits below the launcher so the bars stay readable.
        let width = WIDTH
            .min(ctx.content_rect().width() - 2. * tokens::L)
            .max(120.);
        let content = ctx.content_rect();
        let top = anchor
            .bottom()
            .max(content.top() + (content.height() * 0.12).round());
        let anchor = egui::Rect::from_min_size(
            egui::pos2((content.center().x - width / 2.).round(), top),
            egui::vec2(width, 0.),
        );
        let palette = tokens::palette(ctx);
        let frame = card_frame(ctx);
        egui::Popup::new(popup_id(), ctx.clone(), anchor, layer)
            .open_memory(None)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .frame(frame)
            .width(width)
            .layout(egui::Layout::top_down(egui::Align::Min))
            .show(|ui| {
                let width = width - frame.total_margin().sum().x;
                ui.set_width(width);
                ui.spacing_mut().item_spacing.y = tokens::XS;
                // Consume only palette navigation before TextEdit handles Enter/arrows.
                // A same-frame text edit below invalidates the retained intent.
                let prior_entries = matches(&self.global_search.query);
                self.global_search_keys(ctx, &prior_entries, allowed);
                let body = egui::FontId::proportional(tokens::BODY);
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.global_search.query)
                        .id(query_id())
                        .char_limit(MAX_QUERY_CHARS)
                        .font(body.clone())
                        .desired_width(width)
                        .margin(egui::Margin::symmetric(tokens::S as i8, tokens::XS as i8))
                        .hint_text(
                            RichText::new("Search actions and settings")
                                .font(body.clone())
                                .color(palette.text_disabled),
                        ),
                );
                // 128 Unicode scalars fit the explicit UTF-8 ceiling; protect programmatic state too.
                if self.global_search.query.len() > MAX_QUERY_BYTES {
                    let end = self
                        .global_search
                        .query
                        .char_indices()
                        .map(|(i, _)| i)
                        .take_while(|&i| i <= MAX_QUERY_BYTES)
                        .last()
                        .unwrap_or(0);
                    self.global_search.query.truncate(end);
                }
                if response.changed() {
                    self.global_search.generation = self.global_search.generation.wrapping_add(1);
                    self.global_search.pending = None;
                    self.global_search.selected = 0;
                    self.global_search.preselect = true;
                    self.global_search.reveal = true;
                }
                if self.global_search.focus_query {
                    response.request_focus();
                    self.global_search.focus_query = false;
                }
                let entries = matches(&self.global_search.query);
                if std::mem::take(&mut self.global_search.preselect) {
                    // Do not pre-select a disabled row.
                    self.global_search.selected = entries
                        .iter()
                        .position(|entry| allowed && entry.enabled(self))
                        .unwrap_or(0);
                }
                self.global_search.selected = self
                    .global_search
                    .selected
                    .min(entries.len().saturating_sub(1));
                // Up to ten rows; more scroll. Rows touch, so the stride is the row.
                let height = (ctx.content_rect().height() - 170.)
                    .max(ROW)
                    .min(MAX_ROWS as f32 * ROW);
                let list_height = height.min(entries.len().max(1) as f32 * ROW);
                let list = ui.interact(
                    egui::Rect::from_min_size(
                        ui.next_widget_position(),
                        egui::vec2(width, list_height),
                    ),
                    list_id(),
                    egui::Sense::focusable_noninteractive(),
                );
                list.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Other,
                        true,
                        "Action search results",
                    )
                });
                ui.memory_mut(|memory| {
                    memory.set_focus_lock_filter(
                        list_id(),
                        egui::EventFilter {
                            vertical_arrows: true,
                            ..Default::default()
                        },
                    )
                });
                if entries.is_empty() {
                    ui.label(
                        RichText::new(no_matches(&self.global_search.query))
                            .size(tokens::CAPTION)
                            .color(palette.text_muted),
                    );
                } else {
                    let mut area = egui::ScrollArea::vertical()
                        .id_salt("global_action_results")
                        .auto_shrink([false, true])
                        .max_height(height);
                    if self.global_search.reveal {
                        // Keyboard selection must be visible in this paint, without depending
                        // on wall-clock animation or a later input frame. ScrollArea clamps
                        // the bounded row offset against its finite content and viewport.
                        area =
                            area.vertical_scroll_offset(self.global_search.selected as f32 * ROW);
                    }
                    area.show_themed(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.;
                        let row_width = ui.available_width();
                        for (index, entry) in entries.iter().copied().enumerate() {
                            let enabled = allowed && entry.enabled(self);
                            let selected = index == self.global_search.selected;
                            let response = ui
                                .add_enabled_ui(enabled, |ui| {
                                    result_row(ui, entry, selected, enabled, row_width)
                                })
                                .inner;
                            response.clone().on_hover_text(format!(
                                "{}: {}",
                                entry.category(),
                                entry.label()
                            ));
                            if response.clicked() {
                                self.global_search.selected = index;
                                if self.global_search.queue(ctx, Intent::Execute(entry)) {
                                    // The actual pointer release gives this list ownership once.
                                    // A retry must not reclaim focus from a later surface.
                                    ctx.memory_mut(|memory| memory.request_focus(list_id()));
                                }
                            }
                        }
                    });
                }
                self.global_search.reveal = false;
                let caption = |text: &str| {
                    RichText::new(text)
                        .size(tokens::CAPTION)
                        .color(palette.text_muted)
                };
                if !allowed {
                    ui.label(caption("Action unavailable. Close search and try again."));
                }
                ui.label(caption("Up/Down to select, Enter to run, Esc to close"));
                paint_title_bar(ui, ui.min_rect());
            });
    }

    fn global_search_keys(&mut self, ctx: &egui::Context, entries: &[Entry], allowed: bool) {
        let frame = ctx.cumulative_frame_nr();
        if self.global_search.key_frame == Some(frame)
            || !ctx.input(|input| input.modifiers.is_none())
        {
            return;
        }
        let focused = ctx.memory(|memory| memory.focused());
        if focused != Some(query_id()) && focused != Some(list_id()) {
            return;
        }
        self.global_search.key_frame = Some(frame);
        let key = ctx.input_mut(|input| {
            [
                egui::Key::Escape,
                egui::Key::ArrowDown,
                egui::Key::ArrowUp,
                egui::Key::Home,
                egui::Key::End,
                egui::Key::Enter,
            ]
            .into_iter()
            .find(|key| {
                (focused == Some(list_id()) || !matches!(key, egui::Key::Home | egui::Key::End))
                    && input.consume_key(egui::Modifiers::NONE, *key)
            })
        });
        if matches!(key, Some(egui::Key::ArrowUp | egui::Key::ArrowDown)) {
            ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
        }
        match key {
            Some(egui::Key::Escape) => {
                self.global_search.queue(ctx, Intent::Close);
            }
            Some(egui::Key::Enter) if allowed => {
                if let Some(entry) = entries.get(self.global_search.selected).copied()
                    && entry.enabled(self)
                {
                    self.global_search.queue(ctx, Intent::Execute(entry));
                }
            }
            Some(
                key
                @ (egui::Key::ArrowDown | egui::Key::ArrowUp | egui::Key::Home | egui::Key::End),
            ) => {
                let last = entries.len().saturating_sub(1);
                self.global_search.selected = match key {
                    egui::Key::ArrowDown => (self.global_search.selected + 1).min(last),
                    egui::Key::ArrowUp => self.global_search.selected.saturating_sub(1),
                    egui::Key::Home => 0,
                    _ => last,
                };
                self.global_search.reveal = true;
                ctx.memory_mut(|memory| memory.request_focus(list_id()));
            }
            _ => {}
        }
    }

    pub(super) fn finish_global_search(&mut self, ctx: &egui::Context) {
        if ctx.will_discard() {
            return;
        }
        let Some((frame, generation, intent)) = self.global_search.pending.take() else {
            return;
        };
        if frame != ctx.cumulative_frame_nr() || self.global_search.finished_frame == Some(frame) {
            return;
        }
        self.global_search.finished_frame = Some(frame);
        if matches!(intent, Intent::Close) {
            self.global_search.close(ctx);
            return;
        }
        let Intent::Execute(entry) = intent else {
            return;
        };
        if generation != self.global_search.generation
            || !egui::Popup::is_id_open(ctx, popup_id())
            || !self.global_search_available(ctx)
            || !entry.enabled(self)
            || !self
                .global_search
                .authority
                .as_ref()
                .is_some_and(|a| a.valid(self))
            || !matches(&self.global_search.query).contains(&entry)
            || !ctx.memory(|memory| memory.has_focus(query_id()) || memory.has_focus(list_id()))
        {
            return;
        }
        self.global_search.close(ctx);
        entry.invoke(self, ctx);
    }
}
