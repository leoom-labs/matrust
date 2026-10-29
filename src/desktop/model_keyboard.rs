//! Keyboard commands belong to the focused diagram, never an editable text field.
use super::*;

#[derive(Clone, Copy)]
enum Command {
    Undo,
    Redo,
    Delete,
    Previous,
    Next,
    First,
    Last,
    Escape,
    Nudge([i8; 2], bool),
    StartWire,
    Connect,
    Disconnect,
    PreviousPort,
    NextPort,
    SelectAll,
    Duplicate,
    Copy,
    Cut,
    Paste,
    Range(i8),
}

fn command(event: &egui::Event) -> Option<Command> {
    let egui::Event::Key {
        key,
        pressed: true,
        repeat,
        modifiers,
        ..
    } = event
    else {
        return None;
    };
    if modifiers.alt {
        return None;
    }
    if modifiers.command || modifiers.ctrl {
        let step = if modifiers.shift { 1 } else { 8 };
        let delta = match key {
            egui::Key::ArrowLeft => Some([-step, 0]),
            egui::Key::ArrowRight => Some([step, 0]),
            egui::Key::ArrowUp => Some([0, -step]),
            egui::Key::ArrowDown => Some([0, step]),
            _ => None,
        };
        if let Some(delta) = delta {
            return Some(Command::Nudge(delta, *repeat));
        }
        if *repeat {
            return None;
        }
        return match (key, modifiers.shift) {
            (egui::Key::Z, false) => Some(Command::Undo),
            (egui::Key::Z, true) | (egui::Key::Y, false) => Some(Command::Redo),
            (egui::Key::A, false) => Some(Command::SelectAll),
            (egui::Key::D, false) => Some(Command::Duplicate),
            (egui::Key::C, false) => Some(Command::Copy),
            (egui::Key::X, false) => Some(Command::Cut),
            (egui::Key::V, false) => Some(Command::Paste),
            _ => None,
        };
    }
    if modifiers.shift {
        return match key {
            egui::Key::ArrowLeft | egui::Key::ArrowUp => Some(Command::Range(-1)),
            egui::Key::ArrowRight | egui::Key::ArrowDown => Some(Command::Range(1)),
            egui::Key::Home => Some(Command::Range(-2)),
            egui::Key::End => Some(Command::Range(2)),
            _ => None,
        };
    }
    if !modifiers.is_none() {
        return None;
    }
    match key {
        egui::Key::ArrowUp | egui::Key::ArrowLeft => Some(Command::Previous),
        egui::Key::ArrowDown | egui::Key::ArrowRight => Some(Command::Next),
        egui::Key::Home => Some(Command::First),
        egui::Key::End => Some(Command::Last),
        egui::Key::Delete if !repeat => Some(Command::Delete),
        egui::Key::Escape if !repeat => Some(Command::Escape),
        egui::Key::C if !repeat => Some(Command::StartWire),
        egui::Key::Enter if !repeat => Some(Command::Connect),
        egui::Key::Backspace if !repeat => Some(Command::Disconnect),
        egui::Key::PageUp => Some(Command::PreviousPort),
        egui::Key::PageDown => Some(Command::NextPort),
        _ => None,
    }
}

impl Canvas {
    pub(super) fn keyboard_id() -> egui::Id {
        egui::Id::new("model_diagram_keyboard")
    }

    pub fn focus(&mut self, ctx: &egui::Context) {
        self.nudge = None;
        ctx.memory_mut(|memory| memory.request_focus(Self::keyboard_id()));
        Self::lock_arrows(ctx);
    }

    pub(super) fn lock_arrows(ctx: &egui::Context) {
        ctx.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                Self::keyboard_id(),
                egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                    ..Default::default()
                },
            )
        });
    }

    pub(super) fn keyboard(
        &mut self,
        ui: &egui::Ui,
        model: &mut Model,
        selected: &mut usize,
        message: &mut String,
    ) {
        // egui's focus filter becomes active one frame after a click. Escape
        // can clear newly acquired focus before this widget is visited. Accept
        // that one transition only when no other widget/pointer claimed focus.
        let escaped_new_focus = ui.memory(|memory| {
            memory.focused().is_none() && memory.had_focus_last_frame(Self::keyboard_id())
        }) && ui.input(|input| {
            !input.pointer.any_pressed()
                && input
                    .events
                    .iter()
                    .any(|event| matches!(command(event), Some(Command::Escape)))
        });
        if (!ui.memory(|memory| memory.has_focus(Self::keyboard_id())) && !escaped_new_focus)
            || egui::Popup::is_any_open(ui.ctx())
        {
            self.nudge = None;
            return;
        }
        if ui.input(|input| {
            input.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::Key {
                        key: egui::Key::ArrowLeft
                            | egui::Key::ArrowRight
                            | egui::Key::ArrowUp
                            | egui::Key::ArrowDown,
                        pressed: false,
                        ..
                    }
                )
            })
        }) {
            self.nudge = None;
        }
        let clipboard_event = ui.input_mut(|input| {
            let index = input.events.iter().position(|event| {
                matches!(
                    event,
                    egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_)
                )
            })?;
            Some(input.events.remove(index))
        });
        if let Some(event) = clipboard_event {
            self.cancel_connection();
            self.finish_edit(None);
            match event {
                egui::Event::Cut => {
                    self.cut_blocks(ui, model, ui.available_width(), selected, message)
                }
                egui::Event::Copy => self.copy_blocks(ui, model, ui.available_width(), message),
                egui::Event::Paste(text) => self.paste_blocks(ui, model, &text, selected, message),
                _ => unreachable!(),
            }
            return;
        }
        let action = ui.input_mut(|input| {
            let (index, action) = input
                .events
                .iter()
                .enumerate()
                .find_map(|(index, event)| command(event).map(|action| (index, action)))?;
            input.events.remove(index);
            Some(action)
        });
        let Some(action) = action else {
            return;
        };
        self.finish_edit(None);
        if !matches!(action, Command::Nudge(..)) {
            self.nudge = None;
        }
        match action {
            Command::Cut => self.cut_blocks(ui, model, ui.available_width(), selected, message),
            Command::Copy => self.copy_blocks(ui, model, ui.available_width(), message),
            Command::Paste => ui
                .ctx()
                .send_viewport_cmd(egui::ViewportCommand::RequestPaste),
            Command::Duplicate => self.duplicate_blocks(ui, model, selected, message),
            Command::SelectAll => {
                self.cancel_connection();
                self.selection.all(model.nodes.len(), selected);
                self.reveal(*selected);
            }
            Command::Range(direction) if !model.nodes.is_empty() => {
                self.cancel_connection();
                let last = model.nodes.len() - 1;
                let next = match direction {
                    -2 => 0,
                    2 => last,
                    -1 => {
                        if *selected == usize::MAX {
                            last
                        } else {
                            selected.saturating_sub(1)
                        }
                    }
                    _ => {
                        if *selected == usize::MAX {
                            0
                        } else {
                            selected.saturating_add(1).min(last)
                        }
                    }
                };
                self.select(next, egui::Modifiers::SHIFT, model.nodes.len(), selected);
                self.reveal(*selected);
            }
            Command::Nudge(delta, repeat) => {
                self.nudge_block(ui, model, *selected, delta, repeat, message)
            }
            Command::StartWire if *selected < model.nodes.len() => {
                self.cancel_connection();
                self.source = Some(*selected);
                self.keyboard_wiring = true;
                *message = "Choose the target block and input, then press Enter".into();
            }
            Command::PreviousPort | Command::NextPort => {
                if let Some(node) = model.nodes.get(*selected)
                    && !node.inputs.is_empty()
                {
                    let last = node.inputs.len() - 1;
                    self.keyboard_port = if matches!(action, Command::NextPort) {
                        self.keyboard_port.saturating_add(1).min(last)
                    } else {
                        self.keyboard_port.min(last).saturating_sub(1)
                    };
                    self.reveal_input(*selected, self.keyboard_port);
                    *message = format!("Selected input {}", self.keyboard_port + 1);
                }
            }
            Command::Connect | Command::Disconnect => {
                if let Some(node) = model.nodes.get(*selected)
                    && !node.inputs.is_empty()
                {
                    let port = self.keyboard_port.min(node.inputs.len() - 1);
                    let source = if matches!(action, Command::Disconnect) {
                        Some(usize::MAX)
                    } else {
                        self.keyboard_wiring.then_some(self.source).flatten()
                    };
                    if let Some(source) = source {
                        if self.connect(model, *selected, port, source) {
                            *message = if source == usize::MAX {
                                "Input disconnected; Ctrl+Z restores it"
                            } else {
                                "Blocks connected; Ctrl+Z undoes the wire"
                            }
                            .into();
                        } else {
                            *message = if source == usize::MAX {
                                "Input is already disconnected"
                            } else {
                                "Input is already connected to this block"
                            }
                            .into();
                        }
                        self.cancel_connection();
                        self.keyboard_port = port;
                        self.reveal_input(*selected, port);
                    }
                } else {
                    *message = "Selected block has no input ports".into();
                }
            }
            Command::Escape => {
                if self.source.is_some()
                    || self.moving.is_some()
                    || self.marquee.is_some()
                    || self.view.pan.is_some()
                {
                    self.cancel_connection();
                    self.focus(ui.ctx());
                } else {
                    ui.memory_mut(|memory| memory.surrender_focus(Self::keyboard_id()));
                }
            }
            Command::Undo | Command::Redo => {
                self.cancel_connection();
                self.travel(model, selected, matches!(action, Command::Redo), message);
            }
            Command::Delete if self.selection_len() > 0 => {
                match self.delete_selection(model, selected) {
                    Ok(()) => {
                        *message = "Selected blocks deleted; Ctrl+Z restores them".into();
                    }
                    Err(error) => *message = error,
                }
            }
            Command::Previous | Command::Next | Command::First | Command::Last
                if !model.nodes.is_empty() =>
            {
                if self.keyboard_wiring {
                    self.moving = None;
                    self.keyboard_port = 0;
                } else {
                    self.cancel_connection();
                }
                let last = model.nodes.len() - 1;
                let next = match action {
                    Command::Previous => {
                        if *selected == usize::MAX {
                            last
                        } else {
                            (*selected).min(last).saturating_sub(1)
                        }
                    }
                    Command::Next => {
                        if *selected == usize::MAX {
                            0
                        } else {
                            selected.saturating_add(1).min(last)
                        }
                    }
                    Command::First => 0,
                    _ => last,
                };
                self.select(next, egui::Modifiers::NONE, model.nodes.len(), selected);
                if self.keyboard_wiring && !model.nodes[*selected].inputs.is_empty() {
                    self.reveal_input(*selected, 0);
                } else {
                    self.reveal(*selected);
                }
            }
            _ => {}
        }
    }

    fn nudge_block(
        &mut self,
        ui: &egui::Ui,
        model: &mut Model,
        selected: usize,
        delta: [i8; 2],
        repeat: bool,
        message: &mut String,
    ) {
        if selected >= model.nodes.len() {
            return;
        }
        let group = (selected, delta);
        let merge = repeat && self.nudge == Some(group) && self.cursor == self.history.len();
        let mut positions = if model.positions.is_empty() {
            layout::boxes(model, ui.available_width())
                .iter()
                .map(|rect| [rect.left(), rect.top()])
                .collect()
        } else {
            model.positions.clone()
        };
        let delta = layout::bounded_delta(
            self.selection
                .indices()
                .iter()
                .map(|index| positions[*index]),
            [f32::from(delta[0]), f32::from(delta[1])],
        );
        if delta == [0., 0.] {
            return;
        }
        for &index in self.selection.indices() {
            positions[index][0] += delta[0];
            positions[index][1] += delta[1];
        }
        let merge = merge
            && matches!(self.history.back(), Some(Change::Layout { after, .. }) if after.matches(model));
        let before = (!merge).then(|| layout::Snapshot::capture(model));
        if let Err(error) = model.set_layout(positions) {
            *message = error;
            return;
        }
        self.changed = true;
        self.cancel_connection();
        let next = layout::Snapshot::capture(model);
        if merge {
            if let Some(Change::Layout { after, .. }) = self.history.back_mut() {
                self.history_bytes -= after.positions.capacity() * size_of::<[f32; 2]>();
                self.history_bytes += next.positions.capacity() * size_of::<[f32; 2]>();
                *after = next;
            }
            while self.history_bytes > HISTORY_BYTES {
                self.history_bytes -= self.history.pop_front().unwrap().retained_bytes();
            }
            self.cursor = self.history.len();
        } else {
            self.push(Change::Layout {
                before: before.unwrap(),
                after: next,
            });
        }
        self.nudge = Some(group);
        self.reveal(selected);
        *message = "Block moved; Ctrl+Z undoes this key hold".into();
    }
}
