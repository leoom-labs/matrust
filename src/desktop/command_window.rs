use super::*;
use crate::theme::ThemedScrollArea;

/// Dimensions and entry-to-row offsets for immutable console entries. Length changes
/// invalidate naturally; replacement/retention also explicitly invalidates the cache.
#[derive(Default)]
pub(super) struct ConsoleRows {
    key: Option<(usize, usize)>,
    first_rows: Vec<usize>,
    rows: usize,
    columns: usize,
    #[cfg(test)]
    rebuilds: usize,
}

impl ConsoleRows {
    pub(super) fn has_length(&self, len: usize) -> bool {
        self.key.is_some_and(|(cached, _)| cached == len)
    }

    pub(super) fn invalidate(&mut self) {
        self.key = None;
    }

    fn update(&mut self, console: &[String], skip: usize) {
        if self.key == Some((console.len(), skip)) {
            return;
        }
        #[cfg(test)]
        {
            self.rebuilds += 1;
        }
        self.first_rows.clear();
        self.rows = 0;
        self.columns = 0;
        for entry in &console[skip..] {
            self.first_rows.push(self.rows);
            for line in entry.split('\n') {
                self.rows += 1;
                self.columns = self.columns.max(line.chars().count());
            }
        }
        self.key = Some((console.len(), skip));
    }

    fn entry_at(&self, row: usize) -> usize {
        self.first_rows
            .partition_point(|&first| first <= row)
            .saturating_sub(1)
    }
}

impl Workspace {
    pub(super) fn command_window(&mut self, ui: &mut egui::Ui) {
        let collapsed = self.navigation.drawer_collapsed && self.mode() != modes::Mode::Command;
        ui.scope_builder(
            egui::UiBuilder::new().id(egui::Id::new("command_window_contents")),
            |ui| {
                // Fill the requested panel so the visible border and resize
                // handle coincide, including when the output log is short.
                ui.set_min_height(ui.available_height());

                if !collapsed && panels::header(ui, "Command Window", None, |ui| {
                    let clear = panels::tool_link(ui, "Clear").clicked();
                    self.command_history_menu(ui);
                    clear
                }) {
                    self.console.clear();
                    self.console_rows.invalidate();
                }
                if self.welcome_visible() {
                    self.welcome(ui);
                }
                let palette = tokens::palette(ui.ctx());
                // The body is sunken; it keeps the card's bottom corners.
                let body = ui.available_rect_before_wrap();
                ui.painter().rect_filled(
                    body,
                    egui::CornerRadius {
                        nw: 0,
                        ne: 0,
                        sw: tokens::PANEL_RADIUS,
                        se: tokens::PANEL_RADIUS,
                    },
                    palette.surface,
                );
                let console_font = egui::FontId::monospace(tokens::CONSOLE);
                let line_height = tokens::CONSOLE * tokens::CONSOLE_LINE;
                let completion_height = if self.suggestions.is_empty() {
                    0.
                } else {
                    (body.height() - 120.).clamp(20., 74.)
                };
                let glyph = ui.painter().layout_no_wrap(
                    "0".into(),
                    console_font.clone(),
                    egui::Color32::WHITE,
                );
                let row_height = ui.painter().layout_job(console_line(
                    "0", 0, "0", &console_font, line_height, palette,
                )).size().y;
                // One line is the 34 px field; a multi-line block grows it
                // upward by the rows the input painted last frame (up to 8),
                // within the room the card leaves above for the history.
                let text_height = ui
                    .data(|d| d.get_temp::<f32>(egui::Id::new("command_input").with("height")))
                    .unwrap_or(0.);
                let rows = (text_height / glyph.size().y).round().max(1.);
                let field_height = (tokens::INPUT_HEIGHT
                    + text_height / rows * (rows.min(8.) - 1.))
                    .min(body.height() - completion_height - 2. * tokens::M - tokens::S - 40.)
                    .max(tokens::INPUT_HEIGHT);
                // The input sits at the bottom of the card; the history
                // scrolls above it. Both get explicit rects, so the card's
                // content height never depends on its own size.
                let input_margin = if collapsed { 2. } else { tokens::M };
                let input_rect = egui::Rect::from_min_max(
                    egui::pos2(
                        body.left() + tokens::M,
                        body.bottom() - input_margin - completion_height - field_height,
                    ),
                    egui::pos2(
                        body.right() - tokens::M,
                        body.bottom() - input_margin - completion_height,
                    ),
                );
                let history_rect = egui::Rect::from_min_max(
                    egui::pos2(body.left(), body.top() + tokens::M),
                    egui::pos2(
                        body.right(),
                        (input_rect.top() - tokens::S).max(body.top() + tokens::M + 40.),
                    ),
                );
                // The engine wraps wide matrices to the characters that fit this pane:
                // 16 px output indent, the vertical scrollbar and 12 px right padding,
                // so wrapped rows never reach under the scrollbar or scroll sideways.
                self.worker.console_width = ((history_rect.width() - 40.0) / glyph.size().x.max(1.0))
                    .floor()
                    .clamp(20.0, 400.0) as usize;
                #[cfg(test)]
                let trace_command_scroll = ui.ctx().data(|data| {
                    data.get_temp::<bool>(egui::Id::new("flowing_command_scroll_diagnostic")).unwrap_or(false)
                });
                #[cfg(test)]
                if trace_command_scroll {
                    eprintln!("COMMAND_SCROLL before frame={} time={:.6} scrolling={} raw={:?} smooth={:?} since_scroll={:.6} pointer={:?} wanted={} focused={} viewport={:?} available={:?}",
                        ui.ctx().cumulative_frame_nr(), ui.input(|i| i.time), ui.input(|i| i.is_scrolling()),
                        ui.input(|i| i.raw.events.iter().filter_map(|event| {
                            if let egui::Event::MouseWheel { unit, delta, phase, .. } = event { Some((*unit, *delta, *phase)) } else { None }
                        }).collect::<Vec<_>>()), ui.input(|i| i.smooth_scroll_delta()), ui.input(|i| i.time_since_last_scroll()),
                        ui.input(|i| i.pointer.hover_pos()), self.navigation.command_focus_requested(ui.ctx().cumulative_frame_nr()),
                        ui.memory(|m| m.has_focus(egui::Id::new("command_input"))), ui.ctx().content_rect(), ui.available_rect_before_wrap());
                }
                // Lines keep the width they were printed at, like MATLAB: a pane
                // that narrows later scrolls sideways instead of re-wrapping rows.
                // Count text rows without copying the output or laying out hidden text.
                // An entry can contain many matrix rows, including trailing blank lines.
                let banner = chrome::banner();
                let skip = if self.welcome_visible() && self.console.starts_with(&banner) {
                    banner.len()
                } else {
                    0
                };
                self.console_rows.update(&self.console, skip);
                let (rows, columns) = (self.console_rows.rows, self.console_rows.columns);
                let _command_scroll = if collapsed { None } else { Some(ui.scope_builder(egui::UiBuilder::new().max_rect(history_rect), |ui| {
                    ui.spacing_mut().item_spacing.y = 0.;
                    egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .max_height(history_rect.height())
                    .show_rows_themed(ui, row_height, rows, |ui, visible| {
                        // Keep the horizontal extent stable when wide rows scroll out of view.
                        ui.set_min_width(tokens::L + columns as f32 * glyph.size().x);
                        // Output is indented under the prompt.
                        egui::Frame::new()
                            .inner_margin(egui::Margin {
                                left: tokens::L as i8,
                                ..Default::default()
                            })
                            .show(ui, |ui| {
                                let first_entry = self.console_rows.entry_at(visible.start);
                                let first_row = self.console_rows.first_rows.get(first_entry).copied().unwrap_or(0);
                                for (entry, row, line) in self.console.iter().skip(skip + first_entry)
                                    .flat_map(|entry| entry.split('\n').enumerate()
                                        .map(move |(row, line)| (entry.as_str(), row, line)))
                                    .skip(visible.start - first_row).take(visible.len()) {
                                    ui.add(
                                        egui::Label::new(console_line(
                                            entry,
                                            row,
                                            line,
                                            &console_font,
                                            line_height,
                                            palette,
                                        ))
                                        .extend(),
                                    );
                                }
                            });
                    })}).inner) };
                ui.scope_builder(egui::UiBuilder::new().max_rect(input_rect).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                        // 34 px input: border colour by default, the 1.5 px
                        // accent frame only while the prompt has focus.
                        let prompt_focused = ui.memory(|m| m.has_focus(egui::Id::new("command_input")));
                        egui::Frame::new()
                            .stroke(if prompt_focused {
                                egui::Stroke::new(1.5, palette.accent)
                            } else {
                                egui::Stroke::new(1.5, palette.border)
                            })
                            .corner_radius(tokens::CONTROL_RADIUS)
                            .fill(palette.surface)
                            .inner_margin(egui::Margin::symmetric(tokens::M as i8, 0))
                            .show(ui, |ui| {
                        ui.set_min_height(field_height - 3.);
                        ui.horizontal_centered(|ui| {
                            // `>>` stays beside the first line of a multi-line block.
                            ui.allocate_ui_with_layout(
                                egui::vec2(0., ui.available_height()),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.add_space(
                                        ((tokens::INPUT_HEIGHT - 3. - glyph.size().y) / 2.).max(0.),
                                    );
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(">>")
                                                .color(palette.accent)
                                                .font(console_font.clone()),
                                        )
                                        .extend(),
                                    );
                                },
                            );
                            let command_id = egui::Id::new("command_input");
                            let focused = ui.memory(|m| m.has_focus(command_id));
                            let mut moved = false;
                            let mut cursor_to = None;
                            let mut consumed_navigation = false;
                            let set_cursor = |ctx: &egui::Context, at: usize| {
                                let mut state =
                                    egui::text_edit::TextEditState::load(ctx, command_id)
                                        .unwrap_or_default();
                                state.cursor.set_char_range(Some(
                                    egui::text::CCursorRange::one(egui::text::CCursor::new(at)),
                                ));
                                state.store(ctx, command_id);
                            };
                            // Keys typed after an Enter wait one pass, so they land after
                            // its newline; the TextEdit applies the keys before it first.
                            let deferred_id = command_id.with("after_enter");
                            let pass = ui.ctx().cumulative_pass_nr();
                            if let Some((at, events)) = ui.data_mut(|d| {
                                d.remove_temp::<(u64, Vec<egui::Event>)>(deferred_id)
                            }) && at + 1 == pass
                            {
                                ui.input_mut(|i| {
                                    i.events.splice(0..0, events);
                                });
                            }
                            let mut enter = None;
                            let answering = self.answer.waiting();
                            if focused && (!self.busy || answering) {
                                let mut after = Vec::new();
                                ui.input_mut(|i| {
                                    let Some(at) = i.events.iter().position(|e| {
                                        matches!(
                                            e,
                                            egui::Event::Key {
                                                key: egui::Key::Enter,
                                                pressed: true,
                                                ..
                                            }
                                        )
                                    }) else {
                                        return;
                                    };
                                    let mut tail = i.events.split_off(at);
                                    if let egui::Event::Key { modifiers, .. } = tail.remove(0) {
                                        enter = Some(modifiers);
                                    }
                                    for event in tail {
                                        if matches!(
                                            event,
                                            egui::Event::Text(_)
                                                | egui::Event::Paste(_)
                                                | egui::Event::Key { .. }
                                                | egui::Event::Cut
                                                | egui::Event::Copy
                                                | egui::Event::Ime(_)
                                        ) {
                                            after.push(event);
                                        } else {
                                            i.events.push(event);
                                        }
                                    }
                                });
                                if !after.is_empty() {
                                    ui.data_mut(|d| d.insert_temp(deferred_id, (pass, after)));
                                    ui.ctx().request_repaint();
                                }
                            }
                            let cursor_char = egui::text_edit::TextEditState::load(
                                ui.ctx(),
                                command_id,
                            )
                            .and_then(|state| state.cursor.char_range())
                            .map_or(self.command.chars().count(), |r| r.primary.index.0);
                            let byte_at = |text: &str, chars: usize| {
                                text.char_indices().nth(chars).map_or(text.len(), |(i, _)| i)
                            };
                            let cursor_byte = byte_at(&self.command, cursor_char);
                            // Arrows recall history only from the first/last line of a block.
                            let on_first_line = !self.command[..cursor_byte].contains('\n');
                            let on_last_line = !self.command[cursor_byte..].contains('\n');
                            let mut run_now = false;
                            if focused
                                && on_first_line
                                && !ui.input(|i| i.modifiers.any())
                                && ui.input_mut(|i| {
                                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)
                                })
                            {
                                consumed_navigation = true;
                                if let Some(command) = self.history.previous(&self.command) {
                                    self.command = command;
                                    moved = true;
                                }
                            }
                            if focused
                                && on_last_line
                                && !ui.input(|i| i.modifiers.any())
                                && ui.input_mut(|i| {
                                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
                                })
                            {
                                // Only replace the text while browsing history.
                                if self.history.browsing() {
                                    self.command = self.history.next_entry();
                                    moved = true;
                                }
                                consumed_navigation = true;
                            }
                            if focused
                                && ui.input_mut(|i| {
                                    i.consume_key(egui::Modifiers::NONE, egui::Key::Tab)
                                })
                            {
                                consumed_navigation = true;
                                let cursor =
                                    egui::text_edit::TextEditState::load(ui.ctx(), command_id)
                                        .and_then(|state| state.cursor.char_range())
                                        .map_or(self.command.chars().count(), |r| {
                                            r.primary.index.0
                                        });
                                if let Ok((prefix, range)) =
                                    session::completion_at(&self.command, cursor)
                                {
                                    self.suggestions = session::completions(
                                        &prefix,
                                        self.variables.keys().map(String::as_str),
                                    );
                                    ui.ctx().request_repaint();
                                    self.completion_range = Some(range.clone());
                                    if self.suggestions.len() == 1 {
                                        let completion = self.suggestions.remove(0);
                                        cursor_to = Some(
                                            self.command[..range.start].chars().count()
                                                + completion.chars().count(),
                                        );
                                        self.command.replace_range(range, &completion);
                                        moved = true;
                                    }
                                }
                            }
                            // Focus traversal is computed at frame start, before key consumption.
                            if consumed_navigation {
                                ui.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
                            }
                            if moved {
                                self.suggestions.clear();
                                set_cursor(
                                    ui.ctx(),
                                    cursor_to.unwrap_or_else(|| self.command.chars().count()),
                                );
                            }
                            let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
                            let input = egui::ScrollArea::vertical()
                                .id_salt("command_input_scroll")
                                .auto_shrink([false, true])
                                .max_height(row_height * 8. + 4.)
                                // The default 64 px minimum would centre one line
                                // below the 34 px field and grow the card.
                                .min_scrolled_height(0.)
                                .show_themed(ui, |ui| {
                                    ui.add_enabled(
                                        !self.busy || answering,
                                        egui::TextEdit::multiline(&mut self.command)
                                            .id(command_id)
                                            .lock_focus(true)
                                            .return_key(None)
                                            .desired_rows(1)
                                            .char_limit(1_000_000)
                                            .font(console_font.clone())
                                            .frame(egui::Frame::NONE)
                                            .desired_width(f32::INFINITY)
                                            .hint_text(if answering {
                                                RichText::new("Answer for input")
                                                    .font(console_font.clone())
                                                    .color(palette.text_disabled)
                                            } else if self.history.entries.is_empty() {
                                                RichText::new("A = [1 2; 3 4]; A * A")
                                                    .font(console_font.clone())
                                                    .color(palette.text_disabled)
                                            } else {
                                                RichText::new("")
                                            }),
                                    )
                                })
                                .inner;
                            #[cfg(test)]
                            if trace_command_scroll {
                                eprintln!("COMMAND_SCROLL input frame={} rect={:?} clip={:?} gained={} has_focus={} moved={} wanted={}",
                                    ui.ctx().cumulative_frame_nr(), input.rect, ui.clip_rect(), input.gained_focus(), input.has_focus(), moved,
                                    self.navigation.command_focus_requested(ui.ctx().cumulative_frame_nr()));
                            }
                            let height_id = command_id.with("height");
                            if ui.data(|d| d.get_temp::<f32>(height_id)) != Some(input.rect.height()) {
                                ui.data_mut(|d| d.insert_temp(height_id, input.rect.height()));
                                ui.ctx().request_repaint();
                            }
                            focus_areas::register(ui, &input, focus_areas::Area::Command);
                            if !self.busy
                                && ui.data_mut(|d| d.remove_temp::<bool>(command_id.with("refocus")))
                                    == Some(true)
                                && ui.memory(|m| m.focused().is_none())
                            {
                                // Unless the user moved focus elsewhere during the run.
                                input.request_focus();
                            }
                            // A script's prompt moves focus to the line once.
                            if answering && self.answer.take_focus() {
                                input.request_focus();
                                input.scroll_to_me(None);
                            }
                            // Reveal the prompt for explicit focus/history actions, not
                            // every focused frame: users can still inspect older output.
                            if input.gained_focus()
                                || moved
                                || self
                                    .navigation
                                    .command_focus_requested(ui.ctx().cumulative_frame_nr())
                            {
                                input.scroll_to_me(None);
                            }
                            // Enter is decided on the text as edited by the keys before it.
                            if let Some(modifiers) = enter {
                                // An answer is one line: Enter always sends it.
                                if answering
                                    || modifiers.command
                                    || (!modifiers.shift
                                        && !matrust::syntax::is_incomplete_in_workspace(
                                            &self.command,
                                            &|name| self.variables.contains_key(name),
                                        ))
                                {
                                    run_now = true;
                                } else {
                                    let end = self.command.chars().count();
                                    let (from, to) = egui::text_edit::TextEditState::load(
                                        ui.ctx(),
                                        command_id,
                                    )
                                    .and_then(|state| state.cursor.char_range())
                                    .map_or((end, end), |r| {
                                        let (a, b) = (r.primary.index.0, r.secondary.index.0);
                                        (a.min(b), a.max(b))
                                    });
                                    let bytes =
                                        byte_at(&self.command, from)..byte_at(&self.command, to);
                                    self.command.replace_range(bytes, "\n");
                                    set_cursor(ui.ctx(), from + 1);
                                    self.suggestions.clear();
                                    ui.ctx().request_repaint();
                                }
                            }
                            if run_now && answering {
                                let text = std::mem::take(&mut self.command);
                                if let Some(text) = self.answer.answer(&mut self.console, text) {
                                    // The echo extends the last line in place.
                                    self.console_rows.invalidate();
                                    let _ = self.worker.answers.send(text);
                                }
                                self.suggestions.clear();
                                input.scroll_to_me(None);
                            } else if run_now && !self.command.trim().is_empty() {
                                let command = std::mem::take(&mut self.command);
                                for (n, line) in command.lines().enumerate() {
                                    self.console.push(if n == 0 {
                                        format!(">> {line}")
                                    } else {
                                        line.to_owned()
                                    });
                                }
                                self.history.push(command.clone());
                                self.recovery_dirty = true;
                                self.suggestions.clear();
                                self.run(command);
                                input.scroll_to_me(None);
                                input.request_focus();
                                // The field is disabled while the run is busy and
                                // egui drops its focus; hand it back afterwards.
                                ui.data_mut(|d| d.insert_temp(command_id.with("refocus"), true));
                            }
                            if input.changed()
                                || input.clicked()
                                || (input.has_focus()
                                    && ui.input(|i| {
                                        [
                                            egui::Key::ArrowLeft,
                                            egui::Key::ArrowRight,
                                            egui::Key::Home,
                                            egui::Key::End,
                                        ]
                                        .iter()
                                        .any(|key| i.key_pressed(*key))
                                    }))
                            {
                                self.suggestions.clear();
                            }
                        });
                            });
                        });
                if !self.suggestions.is_empty() {
                    let choices = egui::Rect::from_min_max(
                        egui::pos2(body.left() + tokens::M, input_rect.bottom() + tokens::XS),
                        egui::pos2(body.right() - tokens::M, body.bottom() - tokens::M),
                    );
                    ui.scope_builder(egui::UiBuilder::new().max_rect(choices), |ui| {
                            egui::ScrollArea::vertical()
                                .id_salt("completion_choices")
                                .max_height((completion_height - 10.).max(20.))
                                .show_themed(ui, |ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        let mut chosen = None;
                                        for item in &self.suggestions {
                                            if ui.small_button(item).clicked() {
                                                chosen = Some(item.clone());
                                            }
                                        }
                                        if let Some(item) = chosen {
                                            if let Some(range) = self.completion_range.take()
                                                && self.command.get(range.clone()).is_some()
                                            {
                                                self.command.replace_range(range, &item);
                                            }
                                            self.suggestions.clear();
                                            ui.memory_mut(|m| {
                                                m.request_focus(egui::Id::new("command_input"))
                                            });
                                        }
                                    });
                                });
                    });
                }
                ui.allocate_rect(body, egui::Sense::hover());
                #[cfg(test)]
                if trace_command_scroll && let Some(_command_scroll) = _command_scroll {
                    eprintln!("COMMAND_SCROLL after frame={} discard={} scroll_id={:?} inner={:?} content={:?} state={:?} remaining_smooth={:?}",
                        ui.ctx().cumulative_frame_nr(), ui.ctx().will_discard(), _command_scroll.id,
                        _command_scroll.inner_rect, _command_scroll.content_size, _command_scroll.state,
                        ui.input(|i| i.smooth_scroll_delta()));
                }
            },
        );
    }
}

impl Workspace {
    /// "History" in the Command Window header: earlier commands in a menu;
    /// picking one fills the input.
    fn command_history_menu(&mut self, ui: &mut egui::Ui) {
        let link = panels::tool_link(ui, "History");
        egui::Popup::menu(&link).show(|ui| {
            ui.set_min_width(260.);
            if self.history.entries.is_empty() {
                ui.weak("No commands yet");
                return;
            }
            egui::ScrollArea::vertical()
                .id_salt("command_history")
                .max_height(280.)
                .show_themed(ui, |ui| {
                    for item in self.history.entries.iter().rev() {
                        if ui.selectable_label(false, item).clicked() {
                            self.command = item.clone();
                            self.suggestions.clear();
                            ui.ctx()
                                .memory_mut(|m| m.request_focus(egui::Id::new("command_input")));
                            ui.close();
                        }
                    }
                });
            ui.separator();
            if ui.button("Clear history").clicked() {
                self.history.clear();
                self.recovery_dirty = true;
                ui.close();
            }
        });
    }
}

/// One Command Window line: `>>` in accent, errors in the error colour,
/// `name =` headers muted and values in text.
fn console_line(
    entry: &str,
    row: usize,
    line: &str,
    font: &egui::FontId,
    line_height: f32,
    palette: &tokens::Palette,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    // Empty labels still need the same explicit height as every virtual row.
    let line = if line.is_empty() { " " } else { line };
    let format = |color| egui::TextFormat {
        font_id: font.clone(),
        color,
        line_height: Some(line_height),
        ..Default::default()
    };
    if row == 0
        && let Some(command) = line.strip_prefix(">>")
    {
        job.append(">>", 0., format(palette.accent));
        job.append(command, 0., format(palette.text));
    } else if entry.starts_with(">>") {
        job.append(line, 0., format(palette.text));
    } else if entry.starts_with("Error:") {
        job.append(line, 0., format(palette.error));
    } else if entry.trim_end().ends_with(" =") || entry.trim().is_empty() {
        job.append(line, 0., format(palette.text_muted));
    } else {
        job.append(line, 0., format(palette.text));
    }
    job
}

/// The command line answers a running script's `input` or `pause` instead of
/// running commands. The prompt is the last console line; the answer is
/// appended to it as MATLAB echoes typed text after the prompt.
#[derive(Default)]
pub(super) struct AnswerState {
    prompt: Option<String>,
    focus: bool,
}

impl AnswerState {
    pub(super) fn waiting(&self) -> bool {
        self.prompt.is_some()
    }
    /// Show the script's output so far, then its prompt, and wait.
    pub(super) fn begin(&mut self, console: &mut Vec<String>, request: worker::InputRequest) {
        if request.clear {
            console.clear();
        }
        console.extend(request.output);
        let mut lines = request.prompt.split('\n');
        let mut last = lines.next().unwrap_or_default().to_owned();
        for line in lines {
            console.push(std::mem::replace(&mut last, line.to_owned()));
        }
        console.push(last.clone());
        self.prompt = Some(last);
        self.focus = true;
    }
    /// Echo the answer after its prompt; the text to send to the worker.
    pub(super) fn answer(&mut self, console: &mut Vec<String>, text: String) -> Option<String> {
        let prompt = self.prompt.take()?;
        self.focus = false;
        let echo = text.replace('\n', " ");
        match console.last_mut() {
            Some(line) if *line == prompt => line.push_str(&echo),
            _ => console.push(prompt + &echo),
        }
        Some(text)
    }
    /// The run ended or was stopped: the command line runs commands again.
    pub(super) fn cancel(&mut self) {
        self.prompt = None;
        self.focus = false;
    }
    /// Whether the command line should take focus this frame (once per prompt).
    pub(super) fn take_focus(&mut self) -> bool {
        std::mem::take(&mut self.focus)
    }
}

#[cfg(test)]
mod answer_tests {
    use super::{AnswerState, worker::InputRequest};

    fn request(output: &[&str], prompt: &str) -> InputRequest {
        InputRequest {
            output: output.iter().map(|s| (*s).to_owned()).collect(),
            clear: false,
            prompt: prompt.into(),
        }
    }

    #[test]
    fn answering_input_shows_output_then_prompt_and_echoes_the_answer() {
        let mut state = AnswerState::default();
        let mut console = vec![">> x = input('n? ')".to_owned()];
        assert!(!state.waiting());
        state.begin(&mut console, request(&["Hello"], "n? "));
        assert!(state.waiting());
        assert!(state.take_focus());
        assert!(!state.take_focus(), "focus moves once per prompt");
        assert_eq!(console, [">> x = input('n? ')", "Hello", "n? "]);
        assert_eq!(state.answer(&mut console, "3".into()).as_deref(), Some("3"));
        assert!(!state.waiting());
        assert_eq!(console.last().unwrap(), "n? 3");
        // No prompt is open any more: a second Enter sends nothing.
        assert_eq!(state.answer(&mut console, "4".into()), None);
    }

    #[test]
    fn answering_input_splits_multiline_prompts_clears_and_cancels() {
        let mut state = AnswerState::default();
        let mut console = vec!["old".to_owned()];
        let mut first = request(&["after clc"], "Line one\nValue: ");
        first.clear = true;
        state.begin(&mut console, first);
        assert_eq!(console, ["after clc", "Line one", "Value: "]);
        state.cancel();
        assert!(!state.waiting() && !state.take_focus());
        // A bare pause has an empty prompt; any answer, even empty, ends it.
        state.begin(&mut console, request(&[], ""));
        assert_eq!(console.last().unwrap(), "");
        assert_eq!(
            state.answer(&mut console, String::new()).as_deref(),
            Some("")
        );
        assert!(!state.waiting());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_window_cache_updates_after_push_and_twenty_thousand_entry_trim() {
        let mut console = vec!["one\ntwo".into()];
        let mut cache = ConsoleRows::default();
        cache.update(&console, 0);
        assert_eq!((cache.rows, cache.columns), (2, 3));
        console.push("longer".into());
        cache.update(&console, 0);
        assert_eq!((cache.rows, cache.columns), (3, 6));
        assert_eq!(cache.first_rows, [0, 2]);
        assert_eq!(cache.entry_at(2), 1);
        console.extend((0..19_998).map(|_| "x".into()));
        cache.update(&console, 0);
        // Retention can restore the previous length, so invalidate explicitly.
        console.extend(["new".into(), "newest".into()]);
        worker::keep_newest_output(&mut console);
        cache.invalidate();
        cache.update(&console, 0);
        assert_eq!(console.len(), 20_001);
        assert!(console[0].starts_with("... 3 earlier output lines"));
        assert_eq!(cache.rows, 20_001);
        assert_eq!(cache.columns, console[0].len());
        assert_eq!(cache.entry_at(20_000), 20_000);
        assert_eq!(console[cache.entry_at(20_000)], "newest");
        // A later trim keeps the same length but changes the entries and dimensions.
        console.push("z".repeat(100));
        worker::keep_newest_output(&mut console);
        assert_eq!(console.len(), 20_001);
        cache.invalidate();
        cache.update(&console, 0);
        assert_eq!((cache.rows, cache.columns), (20_001, 100));
        assert!(console[0].starts_with("... 4 earlier output lines"));
        // An unchanged frame reuses the index rather than rebuilding it.
        let rebuilds = cache.rebuilds;
        cache.update(&console, 0);
        assert_eq!(cache.rebuilds, rebuilds);
    }
}
