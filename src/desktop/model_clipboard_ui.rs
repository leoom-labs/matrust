use super::*;

#[derive(Clone, Copy)]
pub(super) enum Action {
    Cut,
    Copy,
    Paste,
    Duplicate,
    Delete,
}

impl Canvas {
    pub(super) fn edit_menu(&self, ui: &mut egui::Ui, interactive: bool) -> Option<Action> {
        let mut chosen = None;
        for (action, label) in [
            (Action::Cut, "Cut blocks (Ctrl+X)"),
            (Action::Copy, "Copy blocks (Ctrl+C)"),
            (Action::Paste, "Paste blocks (Ctrl+V)"),
            (Action::Duplicate, "Duplicate blocks (Ctrl+D)"),
            (Action::Delete, "Delete blocks (Delete)"),
        ] {
            if ui
                .add_enabled(
                    self.selection_len() > 0 || matches!(action, Action::Paste),
                    egui::Button::new(label),
                )
                .clicked()
                && interactive
            {
                chosen = Some(action);
                ui.close();
            }
        }
        chosen
    }
    pub(super) fn edit_action(
        &mut self,
        action: Action,
        ui: &egui::Ui,
        model: &mut Model,
        width: f32,
        selected: &mut usize,
        message: &mut String,
    ) {
        self.cancel_connection();
        self.finish_edit(None);
        match action {
            Action::Cut => self.cut_blocks(ui, model, width, selected, message),
            Action::Copy => self.copy_blocks(ui, model, width, message),
            Action::Paste => ui
                .ctx()
                .send_viewport_cmd(egui::ViewportCommand::RequestPaste),
            Action::Duplicate => {
                let result = clipboard::Fragment::capture(model, self.selection.indices(), width)
                    .and_then(|fragment| {
                        self.insert_fragment(model, fragment, width, 32., selected)
                    });
                *message = result.map_or_else(
                    |error| error,
                    |()| {
                        "Blocks duplicated; internal wires preserved, outside inputs disconnected"
                            .into()
                    },
                );
            }
            Action::Delete => {
                *message = self.delete_selection(model, selected).map_or_else(
                    |error| error,
                    |()| "Selected blocks deleted; Ctrl+Z restores them".into(),
                );
            }
        }
    }
    pub(super) fn cut_blocks(
        &mut self,
        ui: &egui::Ui,
        model: &mut Model,
        width: f32,
        selected: &mut usize,
        message: &mut String,
    ) {
        // Prepare the complete payload before deletion; publish it only after
        // the bounded undoable removal succeeds. Failures issue no CopyText.
        let result = clipboard::Fragment::capture(model, self.selection.indices(), width)
            .and_then(|fragment| fragment.encode())
            .and_then(|text| {
                self.delete_selection(model, selected)?;
                Ok(text)
            });
        match result {
            Ok(text) => {
                self.selection
                    .replace(model.nodes.len(), Vec::new(), selected);
                ui.ctx().copy_text(text);
                *message =
                    "Blocks cut; Ctrl+Z restores them. Paste preserves internal wires only".into();
            }
            Err(error) => *message = error,
        }
    }
    pub(super) fn copy_blocks(
        &self,
        ui: &egui::Ui,
        model: &Model,
        width: f32,
        message: &mut String,
    ) {
        match clipboard::Fragment::capture(model, self.selection.indices(), width)
            .and_then(|fragment| fragment.encode())
        {
            Ok(text) => {
                ui.ctx().copy_text(text);
                *message =
                    "Blocks copied; inputs from outside the group will be disconnected on paste"
                        .into();
            }
            Err(error) => *message = error,
        }
    }
    pub(super) fn insert_fragment(
        &mut self,
        model: &mut Model,
        fragment: clipboard::Fragment,
        width: f32,
        offset: f32,
        selected: &mut usize,
    ) -> matrust::Result<()> {
        let change = clipboard::Change::insert(
            model,
            fragment,
            width,
            offset,
            self.selection.indices().to_vec(),
            HISTORY_BYTES - size_of::<Change>(),
        )?;
        *selected = change.first;
        self.selection
            .replace(model.nodes.len(), change.indices(), selected);
        self.cancel_connection();
        self.push(Change::Paste(Box::new(change)));
        self.reveal(*selected);
        Ok(())
    }
    pub(super) fn duplicate_blocks(
        &mut self,
        ui: &egui::Ui,
        model: &mut Model,
        selected: &mut usize,
        message: &mut String,
    ) {
        let result =
            clipboard::Fragment::capture(model, self.selection.indices(), ui.available_width())
                .and_then(|fragment| {
                    self.insert_fragment(model, fragment, ui.available_width(), 32., selected)
                });
        *message = result.map_or_else(
            |error| error,
            |()| "Blocks duplicated; internal wires preserved, outside inputs disconnected".into(),
        );
    }
    pub(super) fn paste_blocks(
        &mut self,
        ui: &egui::Ui,
        model: &mut Model,
        text: &str,
        selected: &mut usize,
        message: &mut String,
    ) {
        let result = clipboard::Fragment::decode(text).and_then(|fragment| {
            let hash = blake3::hash(text.as_bytes());
            let number = self
                .last_paste
                .filter(|(previous, _)| *previous == hash)
                .map_or(1, |(_, count)| count.saturating_add(1).min(512));
            self.insert_fragment(
                model,
                fragment,
                ui.available_width(),
                32. * number as f32,
                selected,
            )?;
            self.last_paste = Some((hash, number));
            Ok(())
        });
        *message = result.map_or_else(
            |error| error,
            |()| "Blocks pasted; internal wires preserved, outside inputs disconnected".into(),
        );
    }
}
