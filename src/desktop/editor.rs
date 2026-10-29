use crate::theme::ThemedScrollArea;
use eframe::egui::{
    self, FontId,
    text::{CCursor, CCursorRange, CharIndex},
    text_edit::TextEditState,
};
use matrust::{
    document::Document,
    editor::{self, Style},
};
use std::{ops::Range, sync::Arc};

pub fn id(document: u64) -> egui::Id {
    egui::Id::new(("script_buffer", document))
}
fn cursor(range: Range<usize>) -> CCursorRange {
    CCursorRange::two(CCursor::new(range.start), CCursor::new(range.end))
}
#[derive(Default)]
pub struct Editor {
    cache: Option<Arc<egui::Galley>>,
    scale: f32,
    dark: bool,
    pub jump: Option<(u64, Range<usize>)>,
    search_open: bool,
    replace_open: bool,
    focus_search: bool,
    query: String,
    replacement: String,
    search_direction: Option<bool>,
    message: String,
    context: Option<egui::Context>,
    #[cfg(test)]
    search_observation: Option<(egui::Id, egui::Rect, bool, egui::ViewportId, u64)>,
}
impl Editor {
    #[cfg(test)]
    pub fn input_diagnostic(&self) -> String {
        format!(
            "search_open={} focus_search={} query={:?} field={:?}",
            self.search_open, self.focus_search, self.query, self.search_observation
        )
    }
    pub fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::F)) {
            self.open(false);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::H)) {
            self.open(true);
        }
        if self.search_open
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.search_open = false;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::F3)) {
            self.search_direction = Some(true);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F3)) {
            self.search_direction = Some(false);
        }
    }
    pub fn open(&mut self, replace: bool) {
        self.search_open = true;
        self.replace_open = replace;
        self.focus_search = true;
    }
    pub fn forget(&mut self, document: u64) {
        if let Some(ctx) = &self.context {
            ctx.data_mut(|data| data.remove::<TextEditState>(id(document)));
        }
        if self.jump.as_ref().is_some_and(|(doc, _)| *doc == document) {
            self.jump = None;
        }
    }
    fn selection(&self, ctx: &egui::Context, doc: &Document) -> Range<usize> {
        self.jump
            .as_ref()
            .filter(|(d, _)| *d == doc.id)
            .map(|(_, r)| r.clone())
            .unwrap_or_else(|| {
                TextEditState::load(ctx, id(doc.id))
                    .and_then(|s| s.cursor.char_range())
                    .map(|r| {
                        let r = r.as_sorted_char_range();
                        r.start.0..r.end.0
                    })
                    .unwrap_or(0..0)
            })
    }
    fn find(&mut self, ctx: &egui::Context, doc: &Document, backwards: bool) {
        let selected = self.selection(ctx, doc);
        let from = editor::char_byte(
            &doc.text,
            if backwards {
                selected.start
            } else {
                selected.end
            },
        );
        if let Some(range) = editor::find(&doc.text, &self.query, from, backwards) {
            self.jump = Some((doc.id, editor::byte_chars(&doc.text, range)));
            self.message.clear();
        } else {
            self.message = if self.query.is_empty() {
                "Enter text to find"
            } else {
                "No matches"
            }
            .into();
        }
    }
    // Record replacement as a separate undo step, preserving the editor's
    // existing history. The normal text widget handles subsequent Ctrl+Z/Y.
    fn replace_text(
        &mut self,
        ctx: &egui::Context,
        doc: &mut Document,
        text: String,
        range: Range<usize>,
    ) {
        let mut state = TextEditState::load(ctx, id(doc.id)).unwrap_or_default();
        let before = cursor(self.selection(ctx, doc));
        let mut undo = state.undoer();
        undo.add_undo(&(before, doc.text.clone()));
        doc.text = text;
        doc.changed();
        let after = cursor(range.clone());
        state.set_undoer(undo);
        state.cursor.set_char_range(Some(after));
        state.store(ctx, id(doc.id));
        self.jump = Some((doc.id, range));
    }
    fn replace(&mut self, ctx: &egui::Context, doc: &mut Document, all: bool) -> bool {
        if all {
            match editor::replace_all(&doc.text, &self.query, &self.replacement) {
                Ok((text, count)) if count > 0 => {
                    self.replace_text(ctx, doc, text, 0..0);
                    self.message = format!("Replaced {count} matches");
                    return true;
                }
                Ok(_) => self.message = "No matches".into(),
                Err(error) => self.message = error,
            }
        } else {
            let range = self.selection(ctx, doc);
            let bytes =
                editor::char_byte(&doc.text, range.start)..editor::char_byte(&doc.text, range.end);
            if !self.query.is_empty() && doc.text[bytes.clone()] == self.query {
                if doc.text.len() - bytes.len() + self.replacement.len() > editor::MAX_BYTES {
                    self.message = "Replacement exceeds 1 MB".into();
                    return false;
                }
                let mut text = doc.text.clone();
                text.replace_range(bytes, &self.replacement);
                let end = range.start + self.replacement.chars().count();
                self.replace_text(ctx, doc, text, end..end);
                self.find(ctx, doc, false);
                return true;
            }
            self.find(ctx, doc, false);
        }
        false
    }
    /// Returns the current character selection and whether content changed.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        doc: &mut Document,
        error_line: Option<usize>,
    ) -> (Option<Range<usize>>, bool) {
        self.context = Some(ui.ctx().clone());
        let ctx = ui.ctx().clone();
        let editor_id = id(doc.id);
        if TextEditState::load(&ctx, editor_id).is_none() {
            let mut state = TextEditState::default();
            state.set_undoer(egui::util::undoer::Undoer::with_settings(
                egui::util::undoer::Settings {
                    max_undos: 16,
                    ..Default::default()
                },
            ));
            state.store(&ctx, editor_id);
        }
        let mut changed = false;
        if self.search_open {
            ui.horizontal_wrapped(|ui| {
                ui.label("Find");
                let field = ui.add(
                    egui::TextEdit::singleline(&mut self.query)
                        .id_salt("editor_search")
                        .desired_width(140.)
                        .char_limit(4096)
                        .hint_text("Match case, literal"),
                );
                if field.changed() {
                    self.message.clear();
                }
                if self.focus_search {
                    field.request_focus();
                    self.focus_search = false;
                }
                #[cfg(test)]
                {
                    self.search_observation = Some((
                        field.id,
                        field.interact_rect,
                        field.has_focus(),
                        ctx.viewport_id(),
                        ctx.cumulative_frame_nr(),
                    ));
                }
                let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui
                    .small_button("Previous")
                    .on_hover_text("Shift+F3")
                    .clicked()
                {
                    self.search_direction = Some(true);
                }
                if ui.small_button("Next").on_hover_text("F3").clicked() || enter {
                    self.search_direction = Some(false);
                }
                ui.checkbox(&mut self.replace_open, "Replace");
                if ui
                    .small_button("×")
                    .on_hover_text("Close search (Escape)")
                    .clicked()
                {
                    self.search_open = false;
                }
            });
            if self.replace_open {
                ui.horizontal_wrapped(|ui| {
                    ui.label("With");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.replacement)
                            .desired_width(140.)
                            .char_limit(65_536),
                    );
                    if ui.button("Replace next").clicked() {
                        changed |= self.replace(&ctx, doc, false);
                    }
                    if ui.button("Replace all").clicked() {
                        changed |= self.replace(&ctx, doc, true);
                    }
                });
            }
            if !self.message.is_empty() {
                ui.label(&self.message);
            }
        }
        if let Some(backwards) = self.search_direction.take() {
            self.find(&ctx, doc, backwards);
        }
        let jump = self
            .jump
            .take()
            .filter(|(d, _)| *d == doc.id)
            .map(|(_, r)| r);
        if let Some(range) = &jump {
            let mut state = TextEditState::load(&ctx, editor_id).unwrap_or_default();
            state.cursor.set_char_range(Some(cursor(range.clone())));
            state.store(&ctx, editor_id);
            ctx.memory_mut(|m| m.request_focus(editor_id));
        }
        let mut selection = None;
        let font = FontId::monospace(super::tokens::EDITOR);
        let scale = ctx.pixels_per_point();
        // Only the active document holds a layout cache. Unchanged frames share
        // the galley without retokenizing or reconstructing the layout job.
        let cache = &mut self.cache;
        let old_scale = &mut self.scale;
        let dark = ui.visuals().dark_mode;
        let palette = super::tokens::palette_for(dark);
        let old_dark = &mut self.dark;
        let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, _width: f32| {
            let text = buf.as_str();
            if let Some(galley) = cache
                && (galley.job.text == text || editor::layout_matches(&galley.job.text, text))
                && *old_scale == scale
                && *old_dark == dark
            {
                return galley.clone();
            }
            let display = editor::layout_text(text);
            let mut job = egui::text::LayoutJob::default();
            for (range, style) in editor::highlights(text) {
                let syntax = &palette.syntax;
                let color = match style {
                    Style::Plain => syntax.plain,
                    Style::Comment => syntax.comment,
                    Style::String => syntax.string,
                    Style::Number => syntax.number,
                    Style::Keyword => syntax.keyword,
                    Style::Function => syntax.function,
                    Style::Operator => syntax.operator,
                };
                job.append(
                    &display[range],
                    0.,
                    egui::TextFormat {
                        font_id: font.clone(),
                        color,
                        ..Default::default()
                    },
                );
            }
            if text.is_empty() {
                job.append(
                    "",
                    0.,
                    egui::TextFormat {
                        font_id: font.clone(),
                        ..Default::default()
                    },
                );
            }
            job.wrap.max_width = f32::INFINITY;
            let galley = ui.fonts_mut(|f| f.layout_job(job));
            *old_scale = scale;
            *old_dark = dark;
            *cache = Some(galley.clone());
            galley
        };
        egui::ScrollArea::both()
            .id_salt(("script_scroll", doc.id))
            .auto_shrink([false, false])
            .show_themed(ui, |ui| {
                ui.spacing_mut().item_spacing.x = super::tokens::S;
                ui.horizontal_top(|ui| {
                    let gutter = ui.allocate_space(egui::vec2(50., 0.)).1;
                    let mut buffer = BoundedBuffer(&mut doc.text, false);
                    let output = egui::TextEdit::multiline(&mut buffer)
                        .id(editor_id)
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        .desired_rows(28)
                        .lock_focus(true)
                        .layouter(&mut layouter)
                        .show(ui);
                    let clip = ui.clip_rect();
                    let first = output.galley.rows.partition_point(|row| {
                        output.galley_pos.y + row.pos.y + row.size.y < clip.top()
                    });
                    for (i, row) in output.galley.rows.iter().enumerate().skip(first) {
                        let y = output.galley_pos.y + row.pos.y;
                        if y > clip.bottom() {
                            break;
                        }
                        let color = if error_line == Some(i + 1) {
                            palette.error
                        } else {
                            palette.text_muted
                        };
                        ui.painter().text(
                            egui::pos2(gutter.right() - 4., y),
                            egui::Align2::RIGHT_TOP,
                            (i + 1).to_string(),
                            FontId::monospace(super::tokens::CODE),
                            color,
                        );
                        if error_line == Some(i + 1) {
                            ui.painter().circle_filled(
                                egui::pos2(gutter.left() + 4., y + 7.),
                                3.,
                                color,
                            );
                        }
                    }
                    if let Some(range) = output.cursor_range {
                        let r = range.as_sorted_char_range();
                        selection = Some(r.start.0..r.end.0);
                    }
                    if let Some(range) = &jump {
                        let rect = output
                            .galley
                            .pos_from_cursor(CCursor::new(range.start))
                            .translate(output.galley_pos.to_vec2());
                        ui.scroll_to_rect(rect, Some(egui::Align::Center));
                    }
                    // TextEdit can flag cursor-only IME mutations as changed.
                    // Track actual bounded-buffer writes without copying text.
                    changed |= buffer.1;
                });
            });
        if changed {
            doc.changed();
        }
        (selection, changed)
    }
}

/// TextEdit limits characters by default; scripts are bounded in UTF-8 bytes.
struct BoundedBuffer<'a>(&'a mut String, bool);
impl egui::TextBuffer for BoundedBuffer<'_> {
    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<BoundedBuffer<'static>>()
    }
    fn is_mutable(&self) -> bool {
        true
    }
    fn as_str(&self) -> &str {
        self.0
    }
    fn insert_text(&mut self, text: &str, char_index: CharIndex) -> usize {
        let mut end = text
            .len()
            .min(editor::MAX_BYTES.saturating_sub(self.0.len()));
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let inserted = egui::TextBuffer::insert_text(self.0, &text[..end], char_index);
        self.1 |= inserted != 0;
        inserted
    }
    fn delete_char_range(&mut self, range: Range<CharIndex>) {
        let before = self.0.len();
        egui::TextBuffer::delete_char_range(self.0, range);
        self.1 |= self.0.len() != before;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(
        editor: &mut Editor,
        ctx: &egui::Context,
        doc: &mut Document,
        events: Vec<egui::Event>,
    ) {
        let _ = crate::headless::run(
            ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(700., 400.),
                )),
                events,
                ..Default::default()
            },
            |root| {
                let ctx = &root.ctx().clone();
                editor.shortcuts(ctx);
                egui::CentralPanel::default().show(root, |ui| {
                    editor.show(ui, doc, Some(2));
                });
            },
        );
    }
    fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers,
        }
    }
    #[test]
    fn search_replace_and_undo_use_the_actual_text_widget() {
        let ctx = egui::Context::default();
        let mut editor = Editor::default();
        let mut doc = Document::new();
        doc.text = "% α\na=1; a=2;".into();
        frame(&mut editor, &ctx, &mut doc, vec![]);
        let original = doc.text.clone();
        let cached = editor.cache.as_ref().unwrap().clone();
        frame(&mut editor, &ctx, &mut doc, vec![]);
        assert!(
            Arc::ptr_eq(&cached, editor.cache.as_ref().unwrap()),
            "Unchanged frame must share cached layout"
        );
        frame(
            &mut editor,
            &ctx,
            &mut doc,
            vec![key(egui::Key::H, egui::Modifiers::CTRL)],
        );
        assert!(editor.search_open && editor.replace_open);
        editor.query = "a=".into();
        editor.replacement = "value=".into();
        editor.find(&ctx, &doc, false);
        frame(&mut editor, &ctx, &mut doc, vec![]);
        assert_eq!(editor.selection(&ctx, &doc), 4..6);
        assert!(editor.replace(&ctx, &mut doc, false));
        frame(&mut editor, &ctx, &mut doc, vec![]);
        assert_eq!(doc.text, "% α\nvalue=1; a=2;");
        assert!(doc.dirty);
        frame(
            &mut editor,
            &ctx,
            &mut doc,
            vec![key(egui::Key::Z, egui::Modifiers::COMMAND)],
        );
        assert_eq!(doc.text, original, "One Ctrl+Z must undo a replacement");
        frame(
            &mut editor,
            &ctx,
            &mut doc,
            vec![key(egui::Key::Y, egui::Modifiers::COMMAND)],
        );
        assert_eq!(doc.text, "% α\nvalue=1; a=2;");
        assert!(editor.replace(&ctx, &mut doc, true));
        frame(&mut editor, &ctx, &mut doc, vec![]);
        assert_eq!(doc.text, "% α\nvalue=1; value=2;");
        frame(
            &mut editor,
            &ctx,
            &mut doc,
            vec![key(egui::Key::Z, egui::Modifiers::COMMAND)],
        );
        assert_eq!(doc.text, "% α\nvalue=1; a=2;");
    }
    #[test]
    fn text_buffer_enforces_byte_limit_without_splitting_utf8() {
        use egui::TextBuffer;
        let mut text = "x".repeat(editor::MAX_BYTES - 1);
        let mut buffer = BoundedBuffer(&mut text, false);
        assert_eq!(buffer.insert_text("α", CharIndex(0)), 0);
        assert_eq!(buffer.insert_text("aα", CharIndex(0)), 1);
        assert_eq!(buffer.as_str().len(), editor::MAX_BYTES);
        buffer.delete_char_range(CharIndex(0)..CharIndex(2));
        assert_eq!(buffer.insert_text("β", CharIndex(0)), 1);
        assert!(buffer.as_str().starts_with('β'));
    }
    #[test]
    fn shaped_accents_preserve_original_text_and_match_composed_glyphs() {
        let ctx = egui::Context::default();
        let _ = crate::headless::run(&ctx, Default::default(), |_| {});
        let size = crate::tokens::TITLE;
        for font in [FontId::proportional(size), FontId::monospace(size)] {
            let original = "Cafe\u{301} a\u{301} e\u{302}";
            let composed = "Caf\u{e9} \u{e1} \u{ea}";
            let (a, b) = ctx.fonts_mut(|f| {
                (
                    f.layout_no_wrap(original.into(), font.clone(), egui::Color32::BLACK),
                    f.layout_no_wrap(composed.into(), font.clone(), egui::Color32::BLACK),
                )
            });
            assert_eq!(a.text(), original);
            assert_eq!(a.rows[0].glyphs.len(), original.chars().count());
            assert_eq!(a.size(), b.size());
            assert_eq!(a.rows[0].visuals.mesh, b.rows[0].visuals.mesh);
            let end = a.pos_from_cursor(CCursor::new(original.chars().count()));
            assert_eq!(end.left(), a.size().x);
        }
    }
    #[test]
    fn shaped_editor_selection_replace_and_undo_preserve_utf8() {
        let ctx = egui::Context::default();
        let mut editor = Editor::default();
        let mut doc = Document::new();
        let original = "% Cafe\u{301} a\u{301}\nx=1;";
        doc.text = original.into();
        frame(&mut editor, &ctx, &mut doc, vec![]);
        assert_eq!(doc.text, original);
        editor.query = "a\u{301}".into();
        editor.replacement = "\u{3a9}".into();
        editor.find(&ctx, &doc, false);
        frame(&mut editor, &ctx, &mut doc, vec![]);
        let selected = editor.selection(&ctx, &doc);
        assert_eq!(
            doc.text
                .chars()
                .skip(selected.start)
                .take(selected.len())
                .collect::<String>(),
            editor.query
        );
        assert!(editor.replace(&ctx, &mut doc, false));
        frame(&mut editor, &ctx, &mut doc, vec![]);
        assert_eq!(doc.text, "% Cafe\u{301} \u{3a9}\nx=1;");
        frame(
            &mut editor,
            &ctx,
            &mut doc,
            vec![key(egui::Key::Z, egui::Modifiers::COMMAND)],
        );
        assert_eq!(doc.text, original);
    }
}

#[cfg(test)]
mod theme_cache_tests {
    use super::*;
    #[test]
    fn switching_theme_rebuilds_syntax_colors_without_editing_or_copying_text() {
        let ctx = egui::Context::default();
        let mut editor = Editor::default();
        let mut doc = Document::new();
        doc.text = "% comment\nA = sin(1); s = 'text';".into();
        let original = doc.text.clone();
        for dark in [false, true, false] {
            ctx.set_visuals(if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            crate::headless::run(&ctx, egui::RawInput::default(), |root| {
                egui::CentralPanel::default().show(root, |ui| {
                    editor.show(ui, &mut doc, None);
                });
            });
            assert_eq!(editor.dark, dark);
            assert_eq!(doc.text, original);
            assert!(!doc.dirty);
            let galley = editor.cache.clone().unwrap();
            let plain = galley
                .job
                .sections
                .iter()
                .find(|s| &original[s.byte_range.start.0..s.byte_range.end.0] == "A")
                .map(|s| s.format.color);
            // Plain identifiers may be grouped with punctuation; the entire
            // syntax job must still use the selected palette's plain color.
            assert!(
                galley
                    .job
                    .sections
                    .iter()
                    .any(|s| s.format.color == crate::tokens::palette_for(dark).syntax.plain),
                "{plain:?}"
            );
            crate::headless::run(&ctx, egui::RawInput::default(), |root| {
                egui::CentralPanel::default().show(root, |ui| {
                    editor.show(ui, &mut doc, None);
                });
            });
            assert!(
                Arc::ptr_eq(&galley, editor.cache.as_ref().unwrap()),
                "Unchanged same-theme frames must reuse the galley"
            );
        }
    }
}

#[cfg(test)]
mod logical_line_tests {
    use super::*;

    fn frame(
        editor: &mut Editor,
        ctx: &egui::Context,
        doc: &mut Document,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        crate::headless::run(
            ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(960., 650.),
                )),
                events,
                ..Default::default()
            },
            |root| {
                egui::CentralPanel::default().show(root, |ui| {
                    editor.show(ui, doc, Some(2));
                });
            },
        )
    }

    #[test]
    fn cr_editor_rows_cursor_copy_edit_undo_keep_original_buffer() {
        for dark in [false, true] {
            let ctx = egui::Context::default();
            ctx.set_visuals(if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            let mut editor = Editor::default();
            let mut doc = Document::new();
            let raw = "% \u{3bb}\rA=1;\r\nB=2;\nC=3;";
            doc.text = raw.into();
            frame(&mut editor, &ctx, &mut doc, vec![]);
            let galley = editor.cache.as_ref().unwrap().clone();
            assert_eq!(galley.rows.len(), 4);
            assert_eq!(doc.text, raw);
            let start = raw.chars().position(|c| c == 'A').unwrap();
            let cursor = galley.pos_from_cursor(CCursor::new(start));
            assert!(cursor.top() > galley.pos_from_cursor(CCursor::new(0)).top());
            frame(&mut editor, &ctx, &mut doc, vec![]);
            assert!(Arc::ptr_eq(&galley, editor.cache.as_ref().unwrap()));
            editor.jump = Some((doc.id, 0..raw.chars().count()));
            frame(&mut editor, &ctx, &mut doc, vec![]);
            let copied = frame(&mut editor, &ctx, &mut doc, vec![egui::Event::Copy]);
            assert!(
                copied
                    .platform_output
                    .commands
                    .iter()
                    .any(|c| matches!(c,egui::OutputCommand::CopyText(text) if text==raw))
            );
            editor.jump = Some((doc.id, start..start + 1));
            frame(&mut editor, &ctx, &mut doc, vec![]);
            frame(
                &mut editor,
                &ctx,
                &mut doc,
                vec![egui::Event::Text("Z".into())],
            );
            assert_eq!(doc.text, raw.replacen('A', "Z", 1));
            frame(
                &mut editor,
                &ctx,
                &mut doc,
                vec![egui::Event::Key {
                    key: egui::Key::Z,
                    physical_key: Some(egui::Key::Z),
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                }],
            );
            assert_eq!(doc.text, raw);
        }
    }
}
