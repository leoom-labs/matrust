//! Desktop rendering for MATLAB-style `uifigure` and classic `figure + uicontrol`
//! windows. Each visible UI figure opens as its own egui viewport window sized
//! from `Position`, laid out by `uigridlayout` or MATLAB bottom-left `Position`
//! coordinates, and styled with the existing desktop design tokens.
//!
//! User interactions emit [`UiEvent`] values that are forwarded to the engine
//! worker thread (never executed on the UI thread). `uialert`, `uiconfirm`, and
//! `uiprogressdlg` render as modal sheets over their parent figure.
use super::{figure_geometry, figure_view, theme, tokens, worker::DialogTicket};
use eframe::egui::{self, Color32, RichText, Stroke};
use matrust::engine::{
    DialogReply, DialogRequest, GridSpan, GridTrack, MessageIcon, UiEvent, UiKind, UiNodeSnapshot,
    UiSnapshot, UiTableCell, UiWidgetSnapshot, UiWindowSnapshot,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{atomic::AtomicBool, mpsc::Sender};

/// Modal dialog sheet (`uialert` or `uiconfirm`) waiting over a specific UI figure.
enum PendingSheet {
    Alert {
        id: u32,
        title: String,
        text: Vec<String>,
        icon: MessageIcon,
    },
    Confirm {
        title: String,
        text: Vec<String>,
        options: Vec<String>,
        default: String,
        icon: MessageIcon,
        reply: Sender<DialogReply>,
    },
}

/// Ephemeral UI edit state for one widget (buffers while focused, error state).
#[derive(Default)]
struct WidgetState {
    text_buf: String,
    last_synced: String,
    error: bool,
    error_msg: String,
    table_bufs: BTreeMap<(usize, usize), (String, String)>,
}

/// Cached prepared plot geometry for one `uiaxes`.
struct CachedAxes {
    figure: matrust::plot::Figure,
    geometry: Option<figure_geometry::Geometry>,
    reset: bool,
}

/// Manages all open `uifigure` / classic-control windows and their modal sheets.
#[derive(Default)]
pub(super) struct UiWindows {
    snapshot: UiSnapshot,
    revision: u64,
    pending_sheets: BTreeMap<usize, VecDeque<PendingSheet>>,
    widget_states: BTreeMap<usize, WidgetState>,
    axes_cache: BTreeMap<usize, CachedAxes>,
    opened_windows: BTreeSet<usize>,
}

impl UiWindows {
    pub(super) fn set_snapshot(&mut self, snapshot: UiSnapshot) {
        let live_nodes: BTreeSet<usize> = snapshot.nodes.keys().copied().collect();
        let live_figs: BTreeSet<usize> = snapshot.windows.iter().map(|w| w.fig_key).collect();
        self.widget_states.retain(|k, _| live_nodes.contains(k));
        self.axes_cache.retain(|k, _| live_nodes.contains(k));
        self.opened_windows.retain(|k| live_figs.contains(k));
        let mut kept_sheets = BTreeMap::new();
        for (fig_key, mut queue) in std::mem::take(&mut self.pending_sheets) {
            if live_figs.contains(&fig_key) {
                kept_sheets.insert(fig_key, queue);
            } else {
                while let Some(sheet) = queue.pop_front() {
                    cancel_sheet(sheet);
                }
            }
        }
        self.pending_sheets = kept_sheets;
        self.snapshot = snapshot;
        self.revision = self.revision.wrapping_add(1);
    }

    /// Intercept `UiAlert` / `UiConfirm` tickets targeted at a UI figure.
    /// Returns `None` if handled here, or `Some(ticket)` for `DialogWindows`.
    pub(super) fn receive_dialog(&mut self, ticket: DialogTicket) -> Option<DialogTicket> {
        if !matches!(
            ticket.request,
            DialogRequest::UiAlert { .. } | DialogRequest::UiConfirm { .. }
        ) {
            return Some(ticket);
        }
        let DialogTicket { request, reply, .. } = ticket;
        match request {
            DialogRequest::UiAlert {
                id,
                parent_fig,
                title,
                text,
                icon,
            } => {
                self.pending_sheets
                    .entry(parent_fig)
                    .or_default()
                    .push_back(PendingSheet::Alert {
                        id,
                        title,
                        text,
                        icon,
                    });
            }
            DialogRequest::UiConfirm {
                parent_fig,
                title,
                text,
                options,
                default,
                icon,
            } => {
                if let Some(reply) = reply {
                    self.pending_sheets
                        .entry(parent_fig)
                        .or_default()
                        .push_back(PendingSheet::Confirm {
                            title,
                            text,
                            options,
                            default,
                            icon,
                            reply,
                        });
                }
            }
            _ => unreachable!(),
        }
        None
    }

    /// Cancel any blocking sheets (`uiconfirm`) if the running script ended or was stopped.
    /// Non-blocking `uialert` sheets remain visible until dismissed by the user.
    pub(super) fn end_run(&mut self) {
        self.pending_sheets.retain(|_, queue| {
            let mut kept = VecDeque::new();
            while let Some(sheet) = queue.pop_front() {
                match sheet {
                    PendingSheet::Alert { .. } => kept.push_back(sheet),
                    PendingSheet::Confirm { .. } => cancel_sheet(sheet),
                }
            }
            *queue = kept;
            !queue.is_empty()
        });
    }

    #[cfg(test)]
    pub(super) fn pending_sheet_count(&self, fig_key: usize) -> usize {
        self.pending_sheets.get(&fig_key).map_or(0, VecDeque::len)
    }

    #[cfg(test)]
    pub(super) fn snapshot(&self) -> &UiSnapshot {
        &self.snapshot
    }

    #[cfg(test)]
    pub(super) fn window_count(&self) -> usize {
        self.snapshot.windows.len()
    }

    #[cfg(test)]
    pub(super) fn viewport_id(fig_key: usize) -> egui::ViewportId {
        viewport_id_for(fig_key)
    }

    /// Render every visible UI figure in its own egui viewport window and return
    /// any user interaction events to dispatch on the engine thread.
    pub(super) fn show(&mut self, ctx: &egui::Context) -> Vec<UiEvent> {
        let mut events = Vec::new();
        let windows = self.snapshot.windows.clone();
        for win in &windows {
            let vp_id = viewport_id_for(win.fig_key);
            let w = (win.position[2] as f32).clamp(200., 2400.);
            let h = (win.position[3] as f32).clamp(120., 1800.);
            let mut builder = egui::ViewportBuilder::default()
                .with_title(win.title.clone())
                .with_inner_size([w, h])
                .with_min_inner_size([200., 120.]);
            if self.opened_windows.insert(win.fig_key) {
                let x = win.position[0] as f32;
                let y = win.position[1] as f32;
                if x.is_finite() && y.is_finite() && x >= 0. && y >= 0. {
                    builder = builder.with_position([x, y]);
                }
            }
            let fig_key = win.fig_key;
            let is_classic = win.is_classic;
            let classic_fig_id = win.classic_fig_id;
            let mut closed_sheet = false;
            ctx.show_viewport_immediate(vp_id, builder, |ui, _class| {
                let vctx = ui.ctx().clone();
                theme::window_root(ui);
                if vctx.input(|i| i.viewport().close_requested()) {
                    vctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    events.push(UiEvent::CloseFigure {
                        fig_key,
                        is_classic,
                        classic_fig_id,
                    });
                    closed_sheet = true;
                    return;
                }
                let has_modal = self
                    .pending_sheets
                    .get(&fig_key)
                    .is_some_and(|q| !q.is_empty())
                    || !win.progress_dialogs.is_empty();
                egui::CentralPanel::default()
                    .frame(tokens::card(&vctx).inner_margin(egui::Margin::same(tokens::S as i8)))
                    .show(ui, |ui| {
                        let content_rect = ui.available_rect_before_wrap();
                        ui.add_enabled_ui(!has_modal, |ui| {
                            self.render_container_children(
                                ui,
                                content_rect,
                                &win.root_children,
                                false,
                                &mut events,
                            );
                        });
                        if has_modal {
                            self.render_modal_sheet(ui, content_rect, win, &mut events);
                        }
                    });
            });
            if closed_sheet && let Some(mut queue) = self.pending_sheets.remove(&fig_key) {
                while let Some(sheet) = queue.pop_front() {
                    match sheet {
                        PendingSheet::Alert { id, .. } => {
                            events.push(UiEvent::AlertClosed { id });
                        }
                        other => cancel_sheet(other),
                    }
                }
            }
        }
        events
    }

    fn render_modal_sheet(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        win: &UiWindowSnapshot,
        events: &mut Vec<UiEvent>,
    ) {
        let p = tokens::palette(ui.ctx());
        // Translucent backdrop over the parent figure.
        ui.painter()
            .rect_filled(rect, 0.0, Color32::from_black_alpha(96));

        let sheet_w = (rect.width() - tokens::XL * 2.0).clamp(240.0, 400.0);
        let center = rect.center();
        let area_id = egui::Id::new(("ui_modal_sheet", win.fig_key));

        // Priority: blocking uialert/uiconfirm sheet, else active uiprogressdlg.
        let mut finished_reply: Option<DialogReply> = None;
        if let Some(sheet) = self
            .pending_sheets
            .get(&win.fig_key)
            .and_then(VecDeque::front)
        {
            egui::Area::new(area_id)
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ui.ctx(), |ui| {
                    egui::Frame::new()
                        .fill(p.surface)
                        .stroke(Stroke::new(1.0, p.pane_border))
                        .corner_radius(tokens::CONTROL_RADIUS)
                        .inner_margin(egui::Margin::same(tokens::L as i8))
                        .show(ui, |ui| {
                            ui.set_width(sheet_w);
                            match sheet {
                                PendingSheet::Alert {
                                    title, text, icon, ..
                                } => {
                                    sheet_header(ui, title, *icon);
                                    ui.add_space(tokens::S);
                                    for line in text {
                                        ui.label(line);
                                    }
                                    ui.add_space(tokens::M);
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if primary_button(ui, "OK").clicked() {
                                                finished_reply = Some(DialogReply::Done);
                                            }
                                        },
                                    );
                                }
                                PendingSheet::Confirm {
                                    title,
                                    text,
                                    options,
                                    default,
                                    icon,
                                    ..
                                } => {
                                    sheet_header(ui, title, *icon);
                                    ui.add_space(tokens::S);
                                    for line in text {
                                        ui.label(line);
                                    }
                                    ui.add_space(tokens::M);
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            for opt in options.iter().rev() {
                                                let resp =
                                                    if opt == default {
                                                        primary_button(ui, opt)
                                                    } else {
                                                        ui.add(egui::Button::new(opt).min_size(
                                                            egui::vec2(64., tokens::CONTROL),
                                                        ))
                                                    };
                                                if resp.clicked() {
                                                    finished_reply =
                                                        Some(DialogReply::Button(opt.clone()));
                                                }
                                            }
                                        },
                                    );
                                }
                            }
                        });
                });
            if let Some(reply) = finished_reply
                && let Some(queue) = self.pending_sheets.get_mut(&win.fig_key)
                && let Some(sheet) = queue.pop_front()
            {
                if queue.is_empty() {
                    self.pending_sheets.remove(&win.fig_key);
                }
                match sheet {
                    PendingSheet::Alert { id, .. } => {
                        events.push(UiEvent::AlertClosed { id });
                    }
                    PendingSheet::Confirm { reply: tx, .. } => {
                        let _ = tx.send(reply);
                    }
                }
            }
            return;
        }

        // Otherwise render the topmost progress dialog sheet attached to this figure.
        if let Some(&pd_key) = win.progress_dialogs.last()
            && let Some(pd_node) = self.snapshot.nodes.get(&pd_key)
            && let UiWidgetSnapshot::ProgressDialog {
                title,
                message,
                value,
                indeterminate,
                cancelable,
                cancel_text,
                cancel_requested,
                show_percentage,
                ..
            } = &pd_node.widget
        {
            let _ = center;
            egui::Area::new(area_id.with(pd_key))
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ui.ctx(), |ui| {
                    egui::Frame::new()
                        .fill(p.surface)
                        .stroke(Stroke::new(1.0, p.pane_border))
                        .corner_radius(tokens::CONTROL_RADIUS)
                        .inner_margin(egui::Margin::same(tokens::L as i8))
                        .show(ui, |ui| {
                            ui.set_width(sheet_w);
                            if !title.is_empty() {
                                ui.label(tokens::title(ui.ctx(), title));
                                ui.add_space(tokens::S);
                            }
                            if !message.is_empty() {
                                ui.label(message);
                                ui.add_space(tokens::S);
                            }
                            let frac = if *indeterminate {
                                let t = ui.ctx().input(|i| i.time);
                                ui.ctx().request_repaint();
                                ((t * 0.6).fract() as f32).clamp(0.05, 0.95)
                            } else {
                                (*value as f32).clamp(0.0, 1.0)
                            };
                            // Own track + fill: the stock bar's track matched the sheet and its
                            // in-bar percent text was unreadable on the accent fill.
                            let (track, _) = ui.allocate_exact_size(
                                egui::vec2(sheet_w, tokens::S),
                                egui::Sense::hover(),
                            );
                            let r = track.height() / 2.0;
                            ui.painter().rect_filled(track, r, p.surface_sunken);
                            ui.painter().rect_stroke(
                                track,
                                r,
                                egui::Stroke::new(1.0, p.border),
                                egui::StrokeKind::Inside,
                            );
                            let mut fill = track;
                            fill.set_width(track.width() * frac);
                            ui.painter().rect_filled(fill, r, p.accent_fill);
                            if *show_percentage && !*indeterminate {
                                ui.add_space(tokens::XS);
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{}%",
                                        (frac * 100.0).round() as u32
                                    ))
                                    .color(p.text_muted),
                                );
                            }
                            if *cancelable {
                                ui.add_space(tokens::M);
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let label = if cancel_text.is_empty() {
                                            "Cancel"
                                        } else {
                                            cancel_text.as_str()
                                        };
                                        ui.add_enabled_ui(!*cancel_requested, |ui| {
                                            if ui
                                                .add(
                                                    egui::Button::new(label)
                                                        .min_size(egui::vec2(64., tokens::CONTROL)),
                                                )
                                                .clicked()
                                            {
                                                events
                                                    .push(UiEvent::CancelProgress { key: pd_key });
                                            }
                                        });
                                    },
                                );
                            }
                        });
                });
        }
    }

    fn render_container_children(
        &mut self,
        ui: &mut egui::Ui,
        container_rect: egui::Rect,
        children: &[usize],
        is_button_group: bool,
        events: &mut Vec<UiEvent>,
    ) {
        // If there is a single visible GridLayout child, let it fill the container rect.
        let visible_children: Vec<usize> = children
            .iter()
            .copied()
            .filter(|k| self.snapshot.nodes.get(k).is_some_and(|n| n.visible))
            .collect();
        if visible_children.len() == 1
            && let Some(node) = self.snapshot.nodes.get(&visible_children[0])
            && node.kind == UiKind::GridLayout
        {
            self.render_node(ui, container_rect, visible_children[0], events);
            return;
        }

        // Clip all children strictly to the container's inner area.
        ui.scope_builder(egui::UiBuilder::new().max_rect(container_rect), |ui| {
            ui.set_clip_rect(container_rect.intersect(ui.clip_rect()));

            // For ButtonGroup where radio buttons share identical default positions, stack vertically.
            if is_button_group && visible_children.len() > 1 {
                let first_pos = self
                    .snapshot
                    .nodes
                    .get(&visible_children[0])
                    .map(|n| n.position);
                let all_same_pos = visible_children
                    .iter()
                    .all(|k| self.snapshot.nodes.get(k).map(|n| n.position) == first_pos);
                if all_same_pos {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = tokens::XS;
                        for key in visible_children {
                            let row_rect = egui::Rect::from_min_size(
                                ui.cursor().min,
                                egui::vec2(ui.available_width(), tokens::CONTROL),
                            );
                            ui.allocate_rect(row_rect, egui::Sense::hover());
                            self.render_node(ui, row_rect, key, events);
                        }
                    });
                    return;
                }
            }

            // Position-based placement (MATLAB y from bottom; pixels or normalized units).
            for key in visible_children {
                let Some(node) = self.snapshot.nodes.get(&key) else {
                    continue;
                };
                let raw_rect = position_to_rect(container_rect, node.position, &node.units);
                let rect = raw_rect.intersect(container_rect);
                if rect.width() > 0.0 && rect.height() > 0.0 {
                    self.render_node(ui, rect, key, events);
                }
            }
        });
    }

    fn render_node(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        key: usize,
        events: &mut Vec<UiEvent>,
    ) {
        let Some(node) = self.snapshot.nodes.get(&key).cloned() else {
            return;
        };
        if !node.visible {
            return;
        }
        let enabled = node.enable;
        let tooltip = node.tooltip.clone();
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.set_clip_rect(rect.intersect(ui.clip_rect()));
            ui.add_enabled_ui(enabled, |ui| {
                let resp = self.render_widget(ui, rect, &node, events);
                if !tooltip.is_empty()
                    && let Some(r) = resp
                {
                    r.on_hover_text(&tooltip);
                }
            });
        });
    }

    fn render_widget(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        node: &UiNodeSnapshot,
        events: &mut Vec<UiEvent>,
    ) -> Option<egui::Response> {
        let p = tokens::palette(ui.ctx());
        let key = node.key;
        match &node.widget {
            UiWidgetSnapshot::GridLayout {
                row_height,
                column_width,
                padding,
                row_spacing,
                column_spacing,
                scrollable,
            } => {
                self.render_grid_layout(
                    ui,
                    rect,
                    &node.children,
                    row_height,
                    column_width,
                    *padding,
                    *row_spacing as f32,
                    *column_spacing as f32,
                    *scrollable,
                    events,
                );
                None
            }
            UiWidgetSnapshot::Panel { title, .. } => {
                egui::Frame::new()
                    .fill(p.surface_sunken)
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(tokens::CONTROL_RADIUS)
                    .inner_margin(egui::Margin::same(tokens::S as i8))
                    .show(ui, |ui| {
                        let min_sz =
                            (rect.size() - egui::vec2(tokens::L, tokens::L)).max(egui::Vec2::ZERO);
                        ui.set_min_size(min_sz);
                        if !title.is_empty() {
                            ui.label(
                                RichText::new(title)
                                    .strong()
                                    .size(tokens::BODY)
                                    .color(p.text),
                            );
                            ui.add_space(tokens::XS);
                        }
                        let inner = ui.available_rect_before_wrap();
                        self.render_container_children(ui, inner, &node.children, false, events);
                    });
                None
            }
            UiWidgetSnapshot::ButtonGroup { title, .. } => {
                egui::Frame::new()
                    .fill(p.surface_sunken)
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(tokens::CONTROL_RADIUS)
                    .inner_margin(egui::Margin::same(tokens::S as i8))
                    .show(ui, |ui| {
                        let min_sz =
                            (rect.size() - egui::vec2(tokens::L, tokens::L)).max(egui::Vec2::ZERO);
                        ui.set_min_size(min_sz);
                        if !title.is_empty() {
                            ui.label(
                                RichText::new(title)
                                    .strong()
                                    .size(tokens::BODY)
                                    .color(p.text),
                            );
                            ui.add_space(tokens::XS);
                        }
                        let inner = ui.available_rect_before_wrap();
                        self.render_container_children(ui, inner, &node.children, true, events);
                    });
                None
            }
            UiWidgetSnapshot::TabGroup { selected_tab, .. } => {
                let mut active = *selected_tab;
                let tabs: Vec<(usize, String)> = node
                    .children
                    .iter()
                    .filter_map(|&ck| {
                        let child = self.snapshot.nodes.get(&ck)?;
                        if !child.visible {
                            return None;
                        }
                        if let UiWidgetSnapshot::Tab { title } = &child.widget {
                            Some((ck, title.clone()))
                        } else {
                            None
                        }
                    })
                    .collect();
                if active.is_none() {
                    active = tabs.first().map(|(k, _)| *k);
                }
                egui::Frame::new()
                    .fill(p.surface)
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(tokens::CONTROL_RADIUS)
                    .inner_margin(egui::Margin::same(tokens::XS as i8))
                    .show(ui, |ui| {
                        let min_sz =
                            (rect.size() - egui::vec2(tokens::S, tokens::S)).max(egui::Vec2::ZERO);
                        ui.set_min_size(min_sz);
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                for (tk, title) in &tabs {
                                    if tokens::tab(ui, active == Some(*tk), title).clicked()
                                        && active != Some(*tk)
                                    {
                                        events.push(UiEvent::ChooseChild { key: *tk });
                                    }
                                }
                            });
                            ui.separator();
                            let inner = ui.available_rect_before_wrap();
                            if let Some(sel) = active
                                && let Some(tab_node) = self.snapshot.nodes.get(&sel).cloned()
                            {
                                self.render_container_children(
                                    ui,
                                    inner,
                                    &tab_node.children,
                                    false,
                                    events,
                                );
                            }
                        });
                    });
                None
            }
            UiWidgetSnapshot::Tab { .. }
            | UiWidgetSnapshot::Figure
            | UiWidgetSnapshot::ProgressDialog { .. } => None,
            UiWidgetSnapshot::Button { text } => {
                let resp = ui.add_sized(
                    egui::vec2(rect.width(), rect.height().min(tokens::CONTROL)),
                    egui::Button::new(text).corner_radius(tokens::CONTROL_RADIUS),
                );
                if resp.clicked() {
                    events.push(UiEvent::Press { key });
                }
                Some(resp)
            }
            UiWidgetSnapshot::StateButton { text, value } => {
                let resp = tokens::toggle(ui, *value, text);
                if resp.clicked() {
                    events.push(UiEvent::Toggle {
                        key,
                        value: !*value,
                    });
                }
                Some(resp)
            }
            UiWidgetSnapshot::Label {
                text,
                horizontal_alignment,
                word_wrap,
                ..
            } => {
                let align = match horizontal_alignment.to_ascii_lowercase().as_str() {
                    "center" => egui::Align::Center,
                    "right" => egui::Align::Max,
                    _ => egui::Align::Min,
                };
                let mut resp_out = None;
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.with_layout(egui::Layout::top_down(align), |ui| {
                        let mut lbl = egui::Label::new(RichText::new(text).color(p.text));
                        lbl = if *word_wrap {
                            lbl.wrap()
                        } else {
                            lbl.truncate()
                        };
                        resp_out = Some(ui.add(lbl));
                    });
                });
                resp_out
            }
            UiWidgetSnapshot::EditField {
                value,
                editable,
                placeholder,
            } => {
                let state = self.widget_states.entry(key).or_default();
                if state.last_synced != *value {
                    state.text_buf = value.clone();
                    state.last_synced = value.clone();
                }
                let margin_x = if rect.width() < 80.0 {
                    tokens::XS
                } else {
                    tokens::S
                };
                let desired_w = (rect.width() - margin_x * 2.0 - 2.0).max(8.0);
                let edit = tokens::input(
                    egui::TextEdit::singleline(&mut state.text_buf)
                        .id(egui::Id::new(("ui_edit", key)))
                        .hint_text(placeholder)
                        .interactive(*editable)
                        .desired_width(desired_w),
                )
                .margin(egui::Margin::symmetric(margin_x as i8, tokens::XS as i8));
                let resp = ui.add(edit);
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (enter || resp.lost_focus()) && state.text_buf != state.last_synced {
                    let committed = state.text_buf.clone();
                    state.last_synced = committed.clone();
                    events.push(UiEvent::TypeText {
                        key,
                        text: committed,
                    });
                }
                Some(resp)
            }
            UiWidgetSnapshot::NumericEditField {
                value,
                limits,
                value_display_format,
                editable,
                lower_limit_inclusive,
                upper_limit_inclusive,
                round_fractional_values,
            } => {
                let formatted = format_numeric(*value, value_display_format);
                let state = self.widget_states.entry(key).or_default();
                if state.last_synced != formatted {
                    state.text_buf = formatted.clone();
                    state.last_synced = formatted.clone();
                    state.error = false;
                    state.error_msg.clear();
                }
                let margin_x = if rect.width() < 80.0 {
                    tokens::XS
                } else {
                    tokens::S
                };
                let desired_w = (rect.width() - margin_x * 2.0 - 2.0).max(8.0);
                let edit = tokens::input(
                    egui::TextEdit::singleline(&mut state.text_buf)
                        .id(egui::Id::new(("ui_num_edit", key)))
                        .interactive(*editable)
                        .desired_width(desired_w),
                )
                .margin(egui::Margin::symmetric(margin_x as i8, tokens::XS as i8));
                let mut resp = ui.add(edit);
                if resp.changed() {
                    state.error = false;
                    state.error_msg.clear();
                }
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (enter || resp.lost_focus()) && state.text_buf != state.last_synced {
                    match parse_valid_numeric(
                        &state.text_buf,
                        *limits,
                        *lower_limit_inclusive,
                        *upper_limit_inclusive,
                        *round_fractional_values,
                    ) {
                        Some(v) => {
                            state.error = false;
                            state.error_msg.clear();
                            let new_fmt = format_numeric(v, value_display_format);
                            state.text_buf = new_fmt.clone();
                            state.last_synced = new_fmt;
                            events.push(UiEvent::TypeNumeric { key, value: v });
                        }
                        None => {
                            // Reject bad input: turn field to error style and restore old value.
                            state.error = true;
                            state.error_msg = format_numeric_error(
                                *limits,
                                *lower_limit_inclusive,
                                *upper_limit_inclusive,
                            );
                            state.text_buf = formatted;
                        }
                    }
                }
                if state.error {
                    ui.painter().rect_stroke(
                        resp.rect,
                        tokens::CONTROL_RADIUS,
                        Stroke::new(1.5, p.error),
                        egui::StrokeKind::Inside,
                    );
                    if !state.error_msg.is_empty() {
                        resp = resp.on_hover_text(RichText::new(&state.error_msg).color(p.error));
                    }
                }
                Some(resp)
            }
            UiWidgetSnapshot::TextArea {
                value, editable, ..
            } => {
                let joined = value.join("\n");
                let state = self.widget_states.entry(key).or_default();
                if state.last_synced != joined {
                    state.text_buf = joined.clone();
                    state.last_synced = joined;
                }
                let mut resp_out = None;
                let inner_w = (rect.width() - 10.0).max(16.0);
                let inner_h = (rect.height() - 4.0).max(tokens::CONTROL);
                egui::ScrollArea::vertical()
                    .id_salt(("ui_textarea_scroll", key))
                    .max_width(rect.width())
                    .max_height(rect.height())
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let resp = ui.add(
                            egui::TextEdit::multiline(&mut state.text_buf)
                                .id(egui::Id::new(("ui_textarea", key)))
                                .interactive(*editable)
                                .desired_width(inner_w)
                                .min_size(egui::vec2(inner_w, inner_h)),
                        );
                        if resp.lost_focus() && state.text_buf != state.last_synced {
                            let committed = state.text_buf.clone();
                            state.last_synced = committed.clone();
                            let lines: Vec<String> =
                                committed.split('\n').map(str::to_owned).collect();
                            events.push(UiEvent::TypeTextArea { key, lines });
                        }
                        resp_out = Some(resp);
                    });
                resp_out
            }
            UiWidgetSnapshot::DropDown {
                items,
                selected_index,
                display_text,
                ..
            } => {
                let mut chosen = *selected_index;
                let cb = egui::ComboBox::from_id_salt(("ui_dropdown", key))
                    .width((rect.width() - 2.0).max(24.0))
                    .selected_text(display_text)
                    .show_ui(ui, |ui| {
                        for (idx, item) in items.iter().enumerate() {
                            if ui.selectable_label(chosen == Some(idx), item).clicked() {
                                chosen = Some(idx);
                            }
                        }
                    });
                if chosen != *selected_index
                    && let Some(idx) = chosen
                {
                    events.push(UiEvent::ChooseIndex { key, index: idx });
                }
                Some(cb.response)
            }
            UiWidgetSnapshot::ListBox {
                items,
                selected_indices,
                multiselect,
            } => {
                let mut resp_out = None;
                let inner_w = (rect.width() - tokens::S - 2.0).max(16.0);
                let inner_h = (rect.height() - tokens::S - 2.0).max(24.0);
                egui::Frame::new()
                    .fill(p.surface)
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(tokens::CONTROL_RADIUS)
                    .inner_margin(egui::Margin::same(tokens::XS as i8))
                    .show(ui, |ui| {
                        ui.set_min_size(egui::vec2(inner_w, inner_h));
                        ui.set_max_size(egui::vec2(inner_w, inner_h));
                        egui::ScrollArea::vertical()
                            .id_salt(("ui_listbox", key))
                            .max_width(inner_w)
                            .max_height(inner_h)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                for (idx, item) in items.iter().enumerate() {
                                    let is_sel = selected_indices.contains(&idx);
                                    let r = ui.add_sized(
                                        egui::vec2(ui.available_width(), tokens::ROW),
                                        egui::Button::selectable(is_sel, item),
                                    );
                                    if r.clicked() {
                                        let next = if *multiselect {
                                            let ctrl = ui
                                                .input(|i| i.modifiers.ctrl || i.modifiers.command);
                                            if ctrl {
                                                let mut v = selected_indices.clone();
                                                if let Some(pos) = v.iter().position(|&x| x == idx)
                                                {
                                                    v.remove(pos);
                                                } else {
                                                    v.push(idx);
                                                    v.sort_unstable();
                                                }
                                                v
                                            } else {
                                                vec![idx]
                                            }
                                        } else {
                                            vec![idx]
                                        };
                                        if !next.is_empty() {
                                            events.push(UiEvent::ChooseIndices {
                                                key,
                                                indices: next,
                                            });
                                        }
                                    }
                                    if resp_out.is_none() {
                                        resp_out = Some(r);
                                    }
                                }
                            });
                    });
                resp_out
            }
            UiWidgetSnapshot::CheckBox { text, value } => {
                let mut checked = *value;
                let resp = ui.checkbox(&mut checked, text);
                if resp.changed() {
                    events.push(UiEvent::Toggle {
                        key,
                        value: checked,
                    });
                }
                Some(resp)
            }
            UiWidgetSnapshot::RadioButton { text, value } => {
                let resp = ui.radio(*value, text);
                if resp.clicked() {
                    let in_button_group = node
                        .parent
                        .and_then(|pk| self.snapshot.nodes.get(&pk))
                        .is_some_and(|p| p.kind == UiKind::ButtonGroup);
                    if node.is_classic && !in_button_group {
                        events.push(UiEvent::Toggle {
                            key,
                            value: !*value,
                        });
                    } else if !*value {
                        events.push(UiEvent::ChooseChild { key });
                    }
                }
                Some(resp)
            }
            UiWidgetSnapshot::Slider {
                limits,
                value,
                major_ticks,
                orientation,
            } => {
                let mut v = *value;
                let vertical = orientation.eq_ignore_ascii_case("vertical");
                let mut slider = egui::Slider::new(&mut v, limits[0]..=limits[1]);
                if vertical {
                    slider = slider.orientation(egui::SliderOrientation::Vertical);
                }
                let resp = ui.add(slider);
                if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
                    events.push(UiEvent::Slide { key, value: v });
                }
                // Draw major tick marks & labels below horizontal slider.
                if !vertical && !major_ticks.is_empty() && limits[1] > limits[0] {
                    let span = limits[1] - limits[0];
                    let tick_y = resp.rect.bottom() + 2.0;
                    let left = resp.rect.left() + 6.0;
                    let width = (resp.rect.width() - 60.0).max(20.0);
                    let font = egui::FontId::proportional(tokens::SMALL_CODE);
                    for &t in major_ticks {
                        let frac = ((t - limits[0]) / span).clamp(0.0, 1.0) as f32;
                        let x = left + frac * width;
                        ui.painter().line_segment(
                            [egui::pos2(x, tick_y), egui::pos2(x, tick_y + 4.0)],
                            Stroke::new(1.0, p.border),
                        );
                        ui.painter().text(
                            egui::pos2(x, tick_y + 5.0),
                            egui::Align2::CENTER_TOP,
                            format_numeric(t, "%g"),
                            font.clone(),
                            p.text,
                        );
                    }
                }
                Some(resp)
            }
            UiWidgetSnapshot::Spinner {
                limits,
                step,
                value,
                value_display_format,
                editable,
            } => {
                let formatted = format_numeric(*value, value_display_format);
                let state = self.widget_states.entry(key).or_default();
                if state.last_synced != formatted {
                    state.text_buf = formatted.clone();
                    state.last_synced = formatted.clone();
                    state.error = false;
                    state.error_msg.clear();
                }
                let mut resp_out = None;
                let spacing = tokens::XS;
                let btn_w = ((rect.width() - spacing * 2.0) / 3.0).clamp(18.0, tokens::CONTROL);
                let margin_x = if rect.width() < 120.0 {
                    tokens::XS
                } else {
                    tokens::S
                };
                let field_w =
                    (rect.width() - btn_w * 2.0 - spacing * 2.0 - margin_x * 2.0 - 2.0).max(8.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = spacing;
                    if ui
                        .add(egui::Button::new("-").min_size(egui::vec2(btn_w, tokens::CONTROL)))
                        .clicked()
                    {
                        let next = (*value - *step).clamp(limits[0], limits[1]);
                        if (next - *value).abs() > f64::EPSILON {
                            events.push(UiEvent::TypeNumeric { key, value: next });
                        }
                    }
                    let mut resp = ui.add(
                        tokens::input(
                            egui::TextEdit::singleline(&mut state.text_buf)
                                .id(egui::Id::new(("ui_spinner", key)))
                                .interactive(*editable)
                                .desired_width(field_w),
                        )
                        .margin(egui::Margin::symmetric(margin_x as i8, tokens::XS as i8)),
                    );
                    if resp.changed() {
                        state.error = false;
                        state.error_msg.clear();
                    }
                    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (enter || resp.lost_focus()) && state.text_buf != state.last_synced {
                        match parse_valid_numeric(&state.text_buf, *limits, true, true, false) {
                            Some(v) => {
                                state.error = false;
                                state.error_msg.clear();
                                let nf = format_numeric(v, value_display_format);
                                state.text_buf = nf.clone();
                                state.last_synced = nf;
                                events.push(UiEvent::TypeNumeric { key, value: v });
                            }
                            None => {
                                state.error = true;
                                state.error_msg = format_numeric_error(*limits, true, true);
                                state.text_buf = formatted;
                            }
                        }
                    }
                    if state.error {
                        ui.painter().rect_stroke(
                            resp.rect,
                            tokens::CONTROL_RADIUS,
                            Stroke::new(1.5, p.error),
                            egui::StrokeKind::Inside,
                        );
                        if !state.error_msg.is_empty() {
                            resp =
                                resp.on_hover_text(RichText::new(&state.error_msg).color(p.error));
                        }
                    }
                    if ui
                        .add(egui::Button::new("+").min_size(egui::vec2(btn_w, tokens::CONTROL)))
                        .clicked()
                    {
                        let next = (*value + *step).clamp(limits[0], limits[1]);
                        if (next - *value).abs() > f64::EPSILON {
                            events.push(UiEvent::TypeNumeric { key, value: next });
                        }
                    }
                    resp_out = Some(resp);
                });
                resp_out
            }
            UiWidgetSnapshot::Switch {
                items,
                selected_index,
                ..
            } => {
                let is_on = *selected_index == 1;
                let mut resp_out = None;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = tokens::XS;
                    ui.label(
                        RichText::new(&items[0])
                            .size(tokens::SMALL_CODE)
                            .color(if !is_on { p.text } else { p.text_muted }),
                    );
                    let desired = egui::vec2(36.0, 20.0);
                    let (sw_rect, resp) = ui.allocate_exact_size(desired, egui::Sense::click());
                    let fill = if is_on {
                        p.accent_fill
                    } else {
                        p.surface_sunken
                    };
                    ui.painter().rect(
                        sw_rect,
                        tokens::CONTROL_RADIUS,
                        fill,
                        Stroke::new(1.0, p.border),
                        egui::StrokeKind::Inside,
                    );
                    let thumb_x = if is_on {
                        sw_rect.right() - 16.0
                    } else {
                        sw_rect.left() + 2.0
                    };
                    let thumb = egui::Rect::from_min_size(
                        egui::pos2(thumb_x, sw_rect.top() + 2.0),
                        egui::vec2(14.0, 16.0),
                    );
                    ui.painter().rect_filled(thumb, 2.0, p.surface);
                    if resp.clicked() {
                        events.push(UiEvent::ChooseIndex {
                            key,
                            index: usize::from(!is_on),
                        });
                    }
                    ui.label(
                        RichText::new(&items[1])
                            .size(tokens::SMALL_CODE)
                            .color(if is_on { p.text } else { p.text_muted }),
                    );
                    resp_out = Some(resp);
                });
                resp_out
            }
            UiWidgetSnapshot::Table {
                rows,
                cols,
                cells,
                col_names,
                row_names,
                col_editable,
                ..
            } => {
                let state = self.widget_states.entry(key).or_default();
                let mut resp_out = None;
                let inner_w = (rect.width() - tokens::S - 2.0).max(24.0);
                let inner_h = (rect.height() - tokens::S - 2.0).max(24.0);
                let has_row_hdr = !row_names.is_empty();
                let row_hdr_w = if has_row_hdr { 24.0 } else { 0.0 };
                let ncols = (*cols).max(1);
                let total_col_spacing =
                    tokens::S * (ncols + usize::from(has_row_hdr)).saturating_sub(1) as f32;
                let avail_per_col = (inner_w - row_hdr_w - total_col_spacing - 16.0) / ncols as f32;
                let num_col_w = avail_per_col.max(64.0);
                let text_col_w = avail_per_col.max(80.0);
                egui::Frame::new()
                    .fill(p.surface)
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(tokens::CONTROL_RADIUS)
                    .inner_margin(egui::Margin::same(tokens::XS as i8))
                    .show(ui, |ui| {
                        ui.set_min_size(egui::vec2(inner_w, inner_h));
                        ui.set_max_size(egui::vec2(inner_w, inner_h));
                        // Only the visible rows are laid out (a 10000-row
                        // table took 2.3 s per frame when every row was).
                        let header = usize::from(!col_names.is_empty());
                        let row_h = ui.spacing().interact_size.y.max(20.0);
                        ui.spacing_mut().item_spacing.y = tokens::XS;
                        egui::ScrollArea::both()
                            .id_salt(("ui_table_scroll", key))
                            .max_width(inner_w)
                            .max_height(inner_h)
                            .auto_shrink([false, false])
                            .show_rows(ui, row_h, *rows + header, |ui, range| {
                                let first = range.start.saturating_sub(header);
                                let last = range.end.saturating_sub(header).min(*rows);
                                egui::Grid::new(("ui_table_grid", key))
                                    .striped(true)
                                    .min_row_height(row_h)
                                    .spacing([tokens::S, tokens::XS])
                                    .show(ui, |ui| {
                                        if header == 1 && range.start == 0 {
                                            if has_row_hdr {
                                                ui.label("");
                                            }
                                            for name in col_names {
                                                ui.label(
                                                    RichText::new(name).strong().color(p.text),
                                                );
                                            }
                                            ui.end_row();
                                        }
                                        for r in first..last {
                                            if has_row_hdr {
                                                let rh = row_names
                                                    .get(r)
                                                    .map(String::as_str)
                                                    .unwrap_or("");
                                                ui.label(
                                                    RichText::new(rh)
                                                        .size(tokens::SMALL_CODE)
                                                        .color(p.text_muted),
                                                );
                                            }
                                            for c in 0..*cols {
                                                let cell = cells
                                                    .get(r)
                                                    .and_then(|row| row.get(c))
                                                    .cloned()
                                                    .unwrap_or(UiTableCell::Text(String::new()));
                                                let editable =
                                                    col_editable.get(c).copied().unwrap_or(false);
                                                let resp = match cell {
                                                    UiTableCell::Logical(b) if editable => {
                                                        let mut cur = b;
                                                        let resp = ui.checkbox(&mut cur, "");
                                                        if resp.changed() {
                                                            events.push(UiEvent::EditTableCell {
                                                                key,
                                                                row: r + 1,
                                                                col: c + 1,
                                                                value: UiTableCell::Logical(cur),
                                                            });
                                                        }
                                                        resp
                                                    }
                                                    UiTableCell::Numeric(num) if editable => {
                                                        let fmt = format_numeric(num, "%g");
                                                        let entry = state
                                                            .table_bufs
                                                            .entry((r, c))
                                                            .or_insert_with(|| {
                                                                (fmt.clone(), fmt.clone())
                                                            });
                                                        if entry.1 != fmt {
                                                            entry.0 = fmt.clone();
                                                            entry.1 = fmt.clone();
                                                        }
                                                        let resp = ui.add(
                                                            egui::TextEdit::singleline(
                                                                &mut entry.0,
                                                            )
                                                            .id(egui::Id::new((
                                                                "ui_tbl_cell",
                                                                key,
                                                                r,
                                                                c,
                                                            )))
                                                            .desired_width(num_col_w),
                                                        );
                                                        let enter = resp.lost_focus()
                                                            && ui.input(|i| {
                                                                i.key_pressed(egui::Key::Enter)
                                                            });
                                                        if (enter || resp.lost_focus())
                                                            && entry.0 != entry.1
                                                        {
                                                            if let Ok(v) =
                                                                entry.0.trim().parse::<f64>()
                                                                && v.is_finite()
                                                            {
                                                                entry.1 = format_numeric(v, "%g");
                                                                entry.0 = entry.1.clone();
                                                                events.push(
                                                                    UiEvent::EditTableCell {
                                                                        key,
                                                                        row: r + 1,
                                                                        col: c + 1,
                                                                        value: UiTableCell::Numeric(
                                                                            v,
                                                                        ),
                                                                    },
                                                                );
                                                            } else {
                                                                entry.0 = entry.1.clone();
                                                            }
                                                        }
                                                        resp
                                                    }
                                                    UiTableCell::Text(ref s) if editable => {
                                                        let entry = state
                                                            .table_bufs
                                                            .entry((r, c))
                                                            .or_insert_with(|| {
                                                                (s.clone(), s.clone())
                                                            });
                                                        if entry.1 != *s {
                                                            entry.0 = s.clone();
                                                            entry.1 = s.clone();
                                                        }
                                                        let resp = ui.add(
                                                            egui::TextEdit::singleline(
                                                                &mut entry.0,
                                                            )
                                                            .id(egui::Id::new((
                                                                "ui_tbl_cell",
                                                                key,
                                                                r,
                                                                c,
                                                            )))
                                                            .desired_width(text_col_w),
                                                        );
                                                        let enter = resp.lost_focus()
                                                            && ui.input(|i| {
                                                                i.key_pressed(egui::Key::Enter)
                                                            });
                                                        if (enter || resp.lost_focus())
                                                            && entry.0 != entry.1
                                                        {
                                                            let committed = entry.0.clone();
                                                            entry.1 = committed.clone();
                                                            events.push(UiEvent::EditTableCell {
                                                                key,
                                                                row: r + 1,
                                                                col: c + 1,
                                                                value: UiTableCell::Text(committed),
                                                            });
                                                        }
                                                        resp
                                                    }
                                                    other => {
                                                        let text = match other {
                                                            UiTableCell::Numeric(v) => {
                                                                format_numeric(v, "%g")
                                                            }
                                                            UiTableCell::Logical(b) => {
                                                                if b { "true" } else { "false" }
                                                                    .to_string()
                                                            }
                                                            UiTableCell::Text(s) => s,
                                                        };
                                                        let resp = ui.add(
                                                            egui::Label::new(text)
                                                                .sense(egui::Sense::click()),
                                                        );
                                                        if resp.clicked() {
                                                            events.push(UiEvent::SelectTableCell {
                                                                key,
                                                                row: r + 1,
                                                                col: c + 1,
                                                            });
                                                        }
                                                        resp
                                                    }
                                                };
                                                if resp_out.is_none() {
                                                    resp_out = Some(resp);
                                                }
                                            }
                                            ui.end_row();
                                        }
                                    });
                            });
                    });
                resp_out
            }
            UiWidgetSnapshot::Axes { fig_id, figure } => {
                if let Some(fig) = figure.as_deref() {
                    let needs_rebuild = self
                        .axes_cache
                        .get(&key)
                        .is_none_or(|c| !c.figure.same_snapshot(fig));
                    if needs_rebuild {
                        let never = AtomicBool::new(false);
                        let geometry =
                            figure_geometry::Geometry::prepare(*fig_id, fig, &never).ok();
                        self.axes_cache.insert(
                            key,
                            CachedAxes {
                                figure: fig.clone(),
                                geometry,
                                reset: true,
                            },
                        );
                    }
                    if let Some(cached) = self.axes_cache.get_mut(&key)
                        && let Some(figure_geometry::Geometry::Ordinary(ord)) = &cached.geometry
                    {
                        let prepared = figure_view::PreparedView {
                            points: &ord.points,
                            point_indices: Some(&ord.point_indices),
                            axes: ord.transforms,
                            bounds: ord.bounds,
                            legend: ord.legend.as_deref(),
                        };
                        let reset = std::mem::replace(&mut cached.reset, false);
                        let plot_id = egui::Id::new(("uiaxes_plot", key));
                        figure_view::show_interactive_prepared_fitted_with_id(
                            ui,
                            plot_id,
                            &cached.figure,
                            prepared,
                            reset,
                            None,
                        );
                    }
                }
                None
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_grid_layout(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        children: &[usize],
        row_height: &[GridTrack],
        column_width: &[GridTrack],
        padding: [f64; 4],
        row_spacing: f32,
        column_spacing: f32,
        scrollable: bool,
        events: &mut Vec<UiEvent>,
    ) {
        let nrows = row_height.len().max(1);
        let ncols = column_width.len().max(1);
        let pad_l = padding[0].max(0.0) as f32;
        let pad_b = padding[1].max(0.0) as f32;
        let pad_r = padding[2].max(0.0) as f32;
        let pad_t = padding[3].max(0.0) as f32;

        let inner = egui::Rect::from_min_max(
            egui::pos2(rect.left() + pad_l, rect.top() + pad_t),
            egui::pos2(
                (rect.right() - pad_r).max(rect.left() + pad_l + 1.0),
                (rect.bottom() - pad_b).max(rect.top() + pad_t + 1.0),
            ),
        );

        // Estimate intrinsic sizes for 'fit' tracks from children occupying single tracks.
        let mut fit_rows = vec![tokens::CONTROL; nrows];
        let mut fit_cols = vec![80.0_f32; ncols];
        for &ck in children {
            let Some(child) = self.snapshot.nodes.get(&ck) else {
                continue;
            };
            if !child.visible {
                continue;
            }
            let (pref_w, pref_h) = intrinsic_size(child);
            if let Some(layout) = child.layout {
                if let GridSpan::Single(r) = layout.row
                    && r >= 1
                    && r <= nrows
                {
                    fit_rows[r - 1] = fit_rows[r - 1].max(pref_h);
                }
                if let GridSpan::Single(c) = layout.column
                    && c >= 1
                    && c <= ncols
                {
                    fit_cols[c - 1] = fit_cols[c - 1].max(pref_w);
                }
            }
        }

        let col_sizes = resolve_tracks(column_width, &fit_cols, inner.width(), column_spacing);
        let row_sizes = resolve_tracks(row_height, &fit_rows, inner.height(), row_spacing);

        let mut col_starts = Vec::with_capacity(ncols);
        let mut x = inner.left();
        for &w in &col_sizes {
            col_starts.push(x);
            x += w + column_spacing;
        }
        let mut row_starts = Vec::with_capacity(nrows);
        let mut y = inner.top();
        for &h in &row_sizes {
            row_starts.push(y);
            y += h + row_spacing;
        }

        let draw_cells = |this: &mut Self, ui: &mut egui::Ui, events: &mut Vec<UiEvent>| {
            for &ck in children {
                let Some(child) = this.snapshot.nodes.get(&ck) else {
                    continue;
                };
                if !child.visible {
                    continue;
                }
                let layout = child.layout.unwrap_or_default();
                let (r1, r2) = span_bounds(layout.row, nrows);
                let (c1, c2) = span_bounds(layout.column, ncols);
                let cell_left = col_starts[c1];
                let cell_right = col_starts[c2] + col_sizes[c2];
                let cell_top = row_starts[r1];
                let cell_bottom = row_starts[r2] + row_sizes[r2];
                let cell_rect = egui::Rect::from_min_max(
                    egui::pos2(cell_left, cell_top),
                    egui::pos2(
                        cell_right.max(cell_left + 1.0),
                        cell_bottom.max(cell_top + 1.0),
                    ),
                );
                this.render_node(ui, cell_rect, ck, events);
            }
        };

        if scrollable {
            egui::ScrollArea::both()
                .id_salt(("ui_grid_scroll", children.first().copied().unwrap_or(0)))
                .show(ui, |ui| {
                    let total_w = col_sizes.iter().sum::<f32>()
                        + column_spacing * ncols.saturating_sub(1) as f32;
                    let total_h = row_sizes.iter().sum::<f32>()
                        + row_spacing * nrows.saturating_sub(1) as f32;
                    ui.allocate_space(egui::vec2(total_w, total_h));
                    draw_cells(self, ui, events);
                });
        } else {
            draw_cells(self, ui, events);
        }
    }
}

fn viewport_id_for(fig_key: usize) -> egui::ViewportId {
    egui::ViewportId::from_hash_of(("matrust_ui_figure", fig_key))
}

fn cancel_sheet(sheet: PendingSheet) {
    if let PendingSheet::Confirm { reply, .. } = sheet {
        let _ = reply.send(DialogReply::Cancelled);
    }
}

fn primary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let p = tokens::palette(ui.ctx());
    ui.add(
        egui::Button::new(RichText::new(label).color(p.on_accent))
            .fill(p.accent_fill)
            .corner_radius(tokens::CONTROL_RADIUS)
            .min_size(egui::vec2(64.0, tokens::CONTROL)),
    )
}

fn sheet_header(ui: &mut egui::Ui, title: &str, icon: MessageIcon) {
    let p = tokens::palette(ui.ctx());
    ui.horizontal(|ui| {
        let badge = match icon {
            MessageIcon::Error => Some(("!", p.error)),
            MessageIcon::Warn => Some(("!", p.warning)),
            MessageIcon::Help => Some(("i", p.accent)),
            MessageIcon::None => None,
        };
        if let Some((sym, color)) = badge {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
            ui.painter()
                .rect_filled(rect, tokens::CONTROL_RADIUS, color);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                sym,
                egui::FontId::proportional(tokens::BODY),
                p.on_accent,
            );
            ui.add_space(tokens::XS);
        }
        ui.label(tokens::title(ui.ctx(), title));
    });
}

fn span_bounds(span: GridSpan, limit: usize) -> (usize, usize) {
    let last = limit.saturating_sub(1);
    match span {
        GridSpan::Single(i) => {
            let idx = i.saturating_sub(1).min(last);
            (idx, idx)
        }
        GridSpan::Range(a, b) => {
            let lo = a.min(b).saturating_sub(1).min(last);
            let hi = a.max(b).saturating_sub(1).min(last);
            (lo, hi)
        }
    }
}

fn resolve_tracks(tracks: &[GridTrack], fit_sizes: &[f32], avail: f32, spacing: f32) -> Vec<f32> {
    let n = tracks.len().max(1);
    let total_spacing = spacing * n.saturating_sub(1) as f32;
    let mut fixed_sum = 0.0_f32;
    let mut weight_sum = 0.0_f64;
    for (i, t) in tracks.iter().enumerate() {
        match t {
            GridTrack::Pixels(px) => fixed_sum += (*px as f32).max(0.0),
            GridTrack::Fit => fixed_sum += fit_sizes.get(i).copied().unwrap_or(tokens::CONTROL),
            GridTrack::Weight(w) => weight_sum += (*w).max(0.0),
        }
    }
    let remaining = (avail - fixed_sum - total_spacing).max(0.0);
    tracks
        .iter()
        .enumerate()
        .map(|(i, t)| match t {
            GridTrack::Pixels(px) => (*px as f32).max(0.0),
            GridTrack::Fit => fit_sizes.get(i).copied().unwrap_or(tokens::CONTROL),
            GridTrack::Weight(w) => {
                if weight_sum > 0.0 {
                    (remaining * (*w / weight_sum) as f32).max(20.0)
                } else {
                    20.0
                }
            }
        })
        .collect()
}

fn intrinsic_size(node: &UiNodeSnapshot) -> (f32, f32) {
    match &node.widget {
        UiWidgetSnapshot::Button { text }
        | UiWidgetSnapshot::StateButton { text, .. }
        | UiWidgetSnapshot::Label { text, .. }
        | UiWidgetSnapshot::CheckBox { text, .. }
        | UiWidgetSnapshot::RadioButton { text, .. } => {
            let w = (text.len() as f32 * 7.5 + 28.0).clamp(64.0, 240.0);
            (w, tokens::CONTROL)
        }
        UiWidgetSnapshot::EditField { .. }
        | UiWidgetSnapshot::NumericEditField { .. }
        | UiWidgetSnapshot::DropDown { .. }
        | UiWidgetSnapshot::Spinner { .. }
        | UiWidgetSnapshot::Switch { .. } => (110.0, tokens::CONTROL),
        UiWidgetSnapshot::Slider { .. } => (160.0, 44.0),
        UiWidgetSnapshot::TextArea { .. } | UiWidgetSnapshot::ListBox { .. } => (160.0, 84.0),
        UiWidgetSnapshot::Table { .. } | UiWidgetSnapshot::Axes { .. } => (220.0, 160.0),
        _ => (140.0, 100.0),
    }
}

fn position_to_rect(container: egui::Rect, pos: [f64; 4], units: &str) -> egui::Rect {
    let (x, y_bottom, w, h) = if units.eq_ignore_ascii_case("normalized") {
        (
            pos[0] as f32 * container.width(),
            pos[1] as f32 * container.height(),
            pos[2] as f32 * container.width(),
            pos[3] as f32 * container.height(),
        )
    } else {
        (pos[0] as f32, pos[1] as f32, pos[2] as f32, pos[3] as f32)
    };
    let left = container.left() + x;
    let top = container.bottom() - (y_bottom + h);
    egui::Rect::from_min_size(egui::pos2(left, top), egui::vec2(w.max(1.0), h.max(1.0)))
}

fn format_numeric(v: f64, fmt: &str) -> String {
    if !v.is_finite() {
        return format!("{v}");
    }
    if fmt.contains('f')
        && let Some(dot) = fmt.find('.')
        && let Some(f_pos) = fmt[dot + 1..].find('f')
        && let Ok(prec) = fmt[dot + 1..dot + 1 + f_pos].parse::<usize>()
    {
        return format!("{v:.prec$}");
    }
    if v.fract() == 0.0 && v.abs() < 1e12 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

fn parse_valid_numeric(
    text: &str,
    limits: [f64; 2],
    low_inc: bool,
    high_inc: bool,
    round: bool,
) -> Option<f64> {
    let raw = text.trim().parse::<f64>().ok()?;
    if !raw.is_finite() {
        return None;
    }
    let v = if round { raw.round() } else { raw };
    let low_ok = if low_inc {
        v >= limits[0]
    } else {
        v > limits[0]
    };
    let high_ok = if high_inc {
        v <= limits[1]
    } else {
        v < limits[1]
    };
    (low_ok && high_ok).then_some(v)
}

fn format_numeric_error(limits: [f64; 2], low_inc: bool, high_inc: bool) -> String {
    if limits[0].is_finite() && limits[1].is_finite() {
        let lo = format_numeric(limits[0], "%g");
        let hi = format_numeric(limits[1], "%g");
        let lo_br = if low_inc { "[" } else { "(" };
        let hi_br = if high_inc { "]" } else { ")" };
        format!("Enter a number in {lo_br}{lo}, {hi}{hi_br}")
    } else if limits[0].is_finite() {
        let lo = format_numeric(limits[0], "%g");
        let cmp = if low_inc { ">=" } else { ">" };
        format!("Enter a number {cmp} {lo}")
    } else if limits[1].is_finite() {
        let hi = format_numeric(limits[1], "%g");
        let cmp = if high_inc { "<=" } else { "<" };
        format!("Enter a number {cmp} {hi}")
    } else {
        "Enter a valid number".to_owned()
    }
}
