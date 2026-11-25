//! Script dialogs in the desktop app: native pickers for `uigetfile`,
//! `uiputfile` and `uigetdir`; small app windows for `inputdlg`, `questdlg`,
//! `listdlg`, `msgbox` (errordlg, warndlg, helpdlg) and `waitbar`. Requests arrive from
//! the worker as [`DialogTicket`]s; a blocking ticket carries the channel its
//! answer goes back on.
use super::{tokens, worker::DialogTicket};
use eframe::egui;
use matrust::engine::{DialogReply, DialogRequest, FileFilter, MessageIcon};
use std::{collections::BTreeMap, path::Path, sync::mpsc::Sender};

/// An answer the script waits for.
struct Pending {
    form: Form,
    reply: Sender<DialogReply>,
    focus_on_open: bool,
}
enum Form {
    Input {
        title: String,
        prompts: Vec<String>,
        answers: Vec<String>,
        lines: Vec<usize>,
        focus: bool,
    },
    Question {
        title: String,
        text: Vec<String>,
        buttons: Vec<String>,
        default: String,
    },
    List {
        title: String,
        prompts: Vec<String>,
        items: Vec<String>,
        multiple: bool,
        selected: Vec<bool>,
        last_clicked: Option<usize>,
        ok_label: String,
        cancel_label: String,
        list_size: [f64; 2],
    },
}
/// A window that stays after the call returns.
#[derive(Clone, Debug, PartialEq)]
enum Shown {
    Message {
        title: String,
        text: Vec<String>,
        icon: MessageIcon,
    },
    Progress {
        title: String,
        text: Vec<String>,
        fraction: f64,
    },
}

#[derive(Default)]
pub(super) struct DialogWindows {
    pending: Option<Pending>,
    shown: BTreeMap<u32, Shown>,
    /// `uiwait` callers per window.
    waiters: Vec<(u32, Sender<DialogReply>)>,
}

impl DialogWindows {
    /// Take one request from the running script. File and folder pickers are
    /// native and modal: they block this frame until the user picks.
    pub(super) fn receive(&mut self, ticket: DialogTicket) {
        let DialogTicket { request, reply, .. } = ticket;
        let answer = |reply: Option<Sender<DialogReply>>, value: DialogReply| {
            if let Some(reply) = reply {
                let _ = reply.send(value);
            }
        };
        match request {
            DialogRequest::OpenFile {
                title,
                filters,
                folder,
                name,
                multiple,
            } => run_picker(reply, move || {
                let dialog = picker(&title, &filters, folder.as_deref(), &name);
                let picked = if multiple {
                    dialog.pick_files()
                } else {
                    dialog.pick_file().map(|p| vec![p])
                };
                files_reply(picked, &filters)
            }),
            DialogRequest::SaveFile {
                title,
                filters,
                folder,
                name,
            } => run_picker(reply, move || {
                let picked = picker(&title, &filters, folder.as_deref(), &name)
                    .save_file()
                    .map(|p| vec![p]);
                files_reply(picked, &filters)
            }),
            DialogRequest::Folder { title, folder } => run_picker(reply, move || {
                let mut dialog = rfd::FileDialog::new().set_title(&title);
                if let Some(folder) = folder.filter(|f| f.is_dir()) {
                    dialog = dialog.set_directory(folder);
                }
                match dialog.pick_folder() {
                    None => DialogReply::Cancelled,
                    Some(path) if path.to_str().is_none() => {
                        DialogReply::Failed("The picked folder is not valid Unicode.".to_owned())
                    }
                    Some(path) => DialogReply::Folder(path),
                }
            }),
            DialogRequest::Input {
                title,
                prompts,
                defaults,
                lines,
            } => self.begin(
                Form::Input {
                    title,
                    prompts,
                    answers: defaults,
                    lines,
                    focus: true,
                },
                reply,
            ),
            DialogRequest::Question {
                title,
                text,
                buttons,
                default,
            } => self.begin(
                Form::Question {
                    title,
                    text,
                    buttons,
                    default,
                },
                reply,
            ),
            DialogRequest::List {
                title,
                prompts,
                items,
                multiple,
                initial,
                ok_label,
                cancel_label,
                list_size,
            } => {
                let mut selected = vec![false; items.len()];
                for idx in initial {
                    if let Some(slot) = selected.get_mut(idx) {
                        *slot = true;
                        if !multiple {
                            break;
                        }
                    }
                }
                self.begin(
                    Form::List {
                        title,
                        prompts,
                        items,
                        multiple,
                        selected,
                        last_clicked: None,
                        ok_label,
                        cancel_label,
                        list_size,
                    },
                    reply,
                );
            }
            DialogRequest::Message {
                id,
                title,
                text,
                icon,
            } => {
                self.shown.insert(id, Shown::Message { title, text, icon });
                answer(reply, DialogReply::Done);
            }
            DialogRequest::Progress {
                id,
                title,
                text,
                fraction,
            } => {
                self.shown.insert(
                    id,
                    Shown::Progress {
                        title,
                        text,
                        fraction,
                    },
                );
                answer(reply, DialogReply::Done);
            }
            DialogRequest::Close { id } => {
                self.close(id);
                answer(reply, DialogReply::Done);
            }
            DialogRequest::WaitClosed { id, .. } => match reply {
                Some(reply) if self.shown.contains_key(&id) => self.waiters.push((id, reply)),
                reply => answer(reply, DialogReply::Done),
            },
            DialogRequest::UiAlert { .. } | DialogRequest::UiConfirm { .. } => {
                answer(reply, DialogReply::Cancelled);
            }
        }
    }

    fn begin(&mut self, form: Form, reply: Option<Sender<DialogReply>>) {
        let Some(reply) = reply else {
            return;
        };
        // One script waits for one answer at a time; an older form is stale.
        if let Some(old) = self.pending.take() {
            let _ = old.reply.send(DialogReply::Cancelled);
        }
        self.pending = Some(Pending {
            form,
            reply,
            focus_on_open: true,
        });
    }

    fn close(&mut self, id: u32) {
        self.shown.remove(&id);
        self.waiters.retain(|(waiting, reply)| {
            if *waiting == id {
                let _ = reply.send(DialogReply::Done);
            }
            *waiting != id
        });
    }

    /// The run ended or was stopped: forms it waited for go away; message and
    /// progress windows stay, as in MATLAB, until the user closes them.
    pub(super) fn end_run(&mut self) {
        self.pending = None;
        self.waiters.clear();
    }

    pub(super) fn show(&mut self, ctx: &egui::Context) {
        let mut closed = Vec::new();
        for (index, (&id, shown)) in self.shown.iter().enumerate() {
            let offset = egui::vec2(tokens::XL, tokens::XL) * (index % 8) as f32;
            if !show_window(ctx, id, shown, offset) {
                closed.push(id);
            }
        }
        for id in closed {
            self.close(id);
        }
        let Some(pending) = &mut self.pending else {
            return;
        };
        let focus_on_open = std::mem::take(&mut pending.focus_on_open);
        if let Some(reply) = show_form(ctx, &mut pending.form, focus_on_open) {
            let _ = pending.reply.send(reply);
            self.pending = None;
        }
    }
}

/// Native pickers run on their own thread so the app keeps drawing and
/// Stop stays usable while one is open; a stopped script ignores the answer.
/// macOS pickers must run on the main thread.
fn run_picker(
    reply: Option<Sender<DialogReply>>,
    pick: impl FnOnce() -> DialogReply + Send + 'static,
) {
    let Some(reply) = reply else {
        return;
    };
    if cfg!(target_os = "macos") {
        let _ = reply.send(pick());
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("script-file-dialog".into())
        .spawn({
            let reply = reply.clone();
            move || {
                #[cfg(target_os = "linux")]
                if let Some(why) = linux_picker_missing() {
                    let _ = reply.send(DialogReply::Failed(why));
                    return;
                }
                let _ = reply.send(pick());
            }
        });
    if spawned.is_err() {
        let _ = reply.send(DialogReply::Cancelled);
    }
}

/// rfd asks the XDG desktop portal's FileChooser (Wayland and X11) and falls
/// back to zenity; with neither it returns "nothing picked", which a script
/// would read as Cancel. Name the missing packages instead. A found picker is
/// remembered; a missing one is checked again on the next call.
#[cfg(target_os = "linux")]
fn linux_picker_missing() -> Option<String> {
    use std::sync::atomic::{AtomicBool, Ordering};
    static FOUND: AtomicBool = AtomicBool::new(false);
    if FOUND.load(Ordering::Relaxed) {
        return None;
    }
    let on_path = |tool: &str| {
        std::env::var_os("PATH")
            .is_some_and(|path| std::env::split_paths(&path).any(|d| d.join(tool).is_file()))
    };
    let found = on_path("zenity")
        || match std::process::Command::new("busctl")
            .args([
                "--user",
                "--timeout=3",
                "introspect",
                "org.freedesktop.portal.Desktop",
                "/org/freedesktop/portal/desktop",
                "org.freedesktop.portal.FileChooser",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
        {
            Ok(status) => status.success(),
            // No busctl to ask: let rfd try.
            Err(_) => true,
        };
    if found {
        FOUND.store(true, Ordering::Relaxed);
        return None;
    }
    Some(
        "no file picker found: install xdg-desktop-portal with a FileChooser backend \
         (xdg-desktop-portal-gtk, -gnome or -kde; xdg-desktop-portal-hyprland and -wlr have none) \
         or zenity, then try again."
            .to_owned(),
    )
}

fn picker(
    title: &str,
    filters: &[FileFilter],
    folder: Option<&Path>,
    name: &str,
) -> rfd::FileDialog {
    let mut dialog = rfd::FileDialog::new().set_title(title);
    for (label, extensions) in native_filters(filters, picker_platform()) {
        dialog = dialog.add_filter(&label, &extensions);
    }
    if let Some(folder) = folder.filter(|f| f.is_dir()) {
        dialog = dialog.set_directory(folder);
    }
    if !name.is_empty() {
        dialog = dialog.set_file_name(name);
    }
    dialog
}

/// Desktop the native picker is running on. macOS `setAllowedFileTypes`
/// treats `"*"` as a literal extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PickerPlatform {
    Macos,
    Other,
}

fn picker_platform() -> PickerPlatform {
    if cfg!(target_os = "macos") {
        PickerPlatform::Macos
    } else {
        PickerPlatform::Other
    }
}

/// `*.csv` -> `csv`. `*.*` and `*` are every file.
fn extensions_for(filter: &FileFilter, platform: PickerPlatform) -> Vec<String> {
    filter
        .patterns
        .iter()
        .filter_map(|p| match p.as_str() {
            "*" | "*.*" => match platform {
                // macOS passes this list to setAllowedFileTypes. "*" is not
                // "every file" there, and an empty list is not added.
                PickerPlatform::Macos => None,
                PickerPlatform::Other => Some("*".to_owned()),
            },
            p => p
                .strip_prefix("*.")
                .filter(|e| !e.contains(['*', '?']))
                .map(str::to_owned),
        })
        .collect()
}

/// `*.csv` -> `csv`; `*.*` and `*` -> `*` (every file) on this desktop.
/// The picker itself uses [`native_filters`].
#[cfg_attr(not(test), allow(dead_code))]
fn extensions(filter: &FileFilter) -> Vec<String> {
    extensions_for(filter, picker_platform())
}

/// Filters actually passed to the native picker. On macOS, rfd 0.15 unions
/// every filter's extensions into one `allowedFileTypes` list, so a wildcard
/// in any filter (`*` or `*.*`) would hide ordinary files. Pass no extension
/// filter at all in that case. Windows and Linux keep each filter.
fn native_filters(filters: &[FileFilter], platform: PickerPlatform) -> Vec<(String, Vec<String>)> {
    if platform == PickerPlatform::Macos
        && filters
            .iter()
            .any(|f| f.patterns.iter().any(|p| p == "*" || p == "*.*"))
    {
        return Vec::new();
    }
    filters
        .iter()
        .filter_map(|f| {
            let extensions = extensions_for(f, platform);
            (!extensions.is_empty()).then(|| (f.label.clone(), extensions))
        })
        .collect()
}

/// rfd does not report the chosen filter: the first filter matching the
/// first picked name stands in (zero-based).
fn files_reply(picked: Option<Vec<std::path::PathBuf>>, filters: &[FileFilter]) -> DialogReply {
    let Some(paths) = picked.filter(|p| !p.is_empty()) else {
        return DialogReply::Cancelled;
    };
    for path in &paths {
        if path.to_str().is_none() {
            return DialogReply::Failed("The picked path is not valid Unicode.".to_owned());
        }
    }
    let folder = paths[0].parent().map(Path::to_path_buf).unwrap_or_default();
    if folder.to_str().is_none() {
        return DialogReply::Failed("The picked folder is not valid Unicode.".to_owned());
    }
    let names: Vec<String> = paths
        .iter()
        .filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(str::to_owned))
        .collect();
    // `usize::MAX`: no filter matched. The engine leaves that index at 0.
    let filter = filters
        .iter()
        .position(|f| names.first().is_some_and(|n| matches_filter(f, n)))
        .unwrap_or(usize::MAX);
    DialogReply::Files {
        folder,
        names,
        filter,
    }
}

fn matches_filter(filter: &FileFilter, name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    // Index matching keeps `*` as every file. The macOS panel list omits it.
    extensions_for(filter, PickerPlatform::Other)
        .iter()
        .any(|e| e == "*" || lower.ends_with(&format!(".{}", e.to_ascii_lowercase())))
}

fn icon_label(ui: &mut egui::Ui, icon: MessageIcon) {
    let (text, color) = match icon {
        MessageIcon::None => return,
        MessageIcon::Error => ("Error", ui.visuals().error_fg_color),
        MessageIcon::Warn => ("Warning", ui.visuals().warn_fg_color),
        MessageIcon::Help => ("Help", tokens::FIGURE_ACCENT),
    };
    ui.label(
        egui::RichText::new(text)
            .font(tokens::semibold(tokens::HEADING))
            .color(color),
    );
}

fn window_title(title: &str, fallback: &str) -> String {
    if title.is_empty() {
        fallback.to_owned()
    } else {
        title.to_owned()
    }
}

/// Dialog window frame: surface 2, no margin; [`window_chrome`] draws the
/// title bar and pads the body.
pub(super) fn dialog_frame(ctx: &egui::Context) -> egui::Frame {
    egui::Frame::window(&ctx.global_style()).inner_margin(0)
}

/// Height of the dialog title bar (plan section 9).
pub(super) const TITLE_BAR: f32 = 24.;

/// Plan section 9 window chrome for a `title_bar(false)` window framed with
/// [`dialog_frame`]: a 24 px title bar on surface 3 with a 1 px divider and
/// an optional close mark, then the body on surface 2 with 16 px padding.
/// egui's own title bar takes its height from the body margin (50 px) and
/// paints on surface 2.
pub(super) fn window_chrome<R>(
    ui: &mut egui::Ui,
    title: &str,
    open: Option<&mut bool>,
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let top = ui.cursor().min;
    ui.add_space(TITLE_BAR);
    let inner = egui::Frame::NONE
        .inner_margin(tokens::L)
        .show(ui, body)
        .inner;
    let p = tokens::palette(ui.ctx());
    let width = ui.min_rect().width();
    let bar = egui::Rect::from_min_size(top, egui::vec2(width, TITLE_BAR));
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
        title,
        tokens::semibold(tokens::BODY),
        p.text,
    );
    if let Some(open) = open {
        let mark = egui::Rect::from_min_size(
            egui::pos2(bar.right() - TITLE_BAR, bar.top()),
            egui::vec2(TITLE_BAR, TITLE_BAR),
        );
        let response = ui
            .interact(
                mark,
                ui.id().with("window_chrome_close"),
                egui::Sense::click(),
            )
            .on_hover_text("Close");
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Close"));
        if response.hovered() {
            ui.painter().rect_filled(mark.shrink(2.), 2., p.hairline);
        }
        let ink = if response.hovered() {
            p.text
        } else {
            p.text_muted
        };
        let r = 4.;
        let c = mark.center();
        let stroke = egui::Stroke::new(1.5, ink);
        ui.painter()
            .line_segment([c + egui::vec2(-r, -r), c + egui::vec2(r, r)], stroke);
        ui.painter()
            .line_segment([c + egui::vec2(-r, r), c + egui::vec2(r, -r)], stroke);
        if response.clicked() {
            *open = false;
        }
    }
    inner
}

/// Buttons right-aligned in one control-high row. A bare right-to-left
/// layout claims the window's whole free height, so the row is bounded.
fn button_row(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let size = egui::vec2(ui.available_width(), tokens::CONTROL);
    ui.allocate_ui_with_layout(size, egui::Layout::right_to_left(egui::Align::Center), add);
}

/// One message or progress window; false once the user closed it.
fn show_window(ctx: &egui::Context, id: u32, shown: &Shown, offset: egui::Vec2) -> bool {
    let mut open = true;
    let mut ok = false;
    let (title, fallback) = match shown {
        Shown::Message { title, .. } => (title, "Message"),
        Shown::Progress { title, .. } => (title, "Progress"),
    };
    let title = window_title(title, fallback);
    egui::Window::new(title.as_str())
        .id(egui::Id::new(("script_dialog", id)))
        .default_pos(ctx.content_rect().center() - egui::vec2(160., 60.) + offset)
        .collapsible(false)
        .resizable(false)
        .title_bar(false)
        .frame(dialog_frame(ctx))
        .show(ctx, |ui| {
            window_chrome(ui, &title, Some(&mut open), |ui| {
                ui.set_min_width(280.);
                ui.spacing_mut().item_spacing = egui::vec2(tokens::S, tokens::S);
                match shown {
                    Shown::Message { text, icon, .. } => {
                        icon_label(ui, *icon);
                        for line in text {
                            ui.label(line);
                        }
                        ui.add_space(tokens::XS);
                        button_row(ui, |ui| {
                            ok = ui
                                .add(
                                    egui::Button::new("OK")
                                        .min_size(egui::vec2(72., tokens::CONTROL)),
                                )
                                .clicked();
                        });
                    }
                    Shown::Progress { text, fraction, .. } => {
                        for line in text {
                            ui.label(line);
                        }
                        ui.add(
                            egui::ProgressBar::new(*fraction as f32)
                                .desired_width(280.)
                                .fill(tokens::FIGURE_ACCENT)
                                .show_percentage(),
                        );
                    }
                }
            })
        });
    open && !ok
}

/// Route the shared Enter shortcut to the form's window or one of its controls.
fn form_enter(ui: &egui::Ui, focus_on_open: bool) -> (bool, bool) {
    let ctx = ui.ctx();
    let previous_rect = ui.response().rect;
    let focus_rect = if previous_rect.is_positive() {
        previous_rect
    } else {
        ui.min_rect()
    };
    let window_focus = ui.interact(
        focus_rect,
        ui.id().with("form_keyboard_focus"),
        egui::Sense::focusable_noninteractive(),
    );
    let (enter_pressed, viewport_focused, pointer_pos) = ui.input(|i| {
        let pointer_pos = (i.pointer.any_pressed() || i.pointer.any_click())
            .then(|| i.pointer.interact_pos())
            .flatten();
        (i.key_pressed(egui::Key::Enter), i.focused, pointer_pos)
    });
    let pointer_in_window = pointer_pos.map(|pos| ctx.layer_id_at(pos) == Some(ui.layer_id()));
    let focused = ui
        .memory(|m| m.focused())
        .and_then(|id| ctx.read_response(id))
        .filter(|response| response.layer_id == ui.layer_id());

    if pointer_in_window == Some(false) {
        if let Some(response) = &focused {
            response.surrender_focus();
        }
    } else if focus_on_open || (pointer_in_window == Some(true) && focused.is_none()) {
        window_focus.request_focus();
    }

    let submit = enter_pressed
        && viewport_focused
        && !focus_on_open
        && pointer_in_window != Some(false)
        && focused.is_some();
    (enter_pressed, submit)
}

/// The waiting form; `Some(reply)` once the user answered.
fn show_form(ctx: &egui::Context, form: &mut Form, focus_on_open: bool) -> Option<DialogReply> {
    let mut reply = None;
    let mut open = true;
    let title = match form {
        Form::Input { title, .. } => window_title(title, "Input"),
        Form::Question { title, .. } => window_title(title, "Question"),
        Form::List { title, .. } => window_title(title, "Select"),
    };
    egui::Window::new(title.as_str())
        .id(egui::Id::new("script_dialog_form"))
        .default_pos(ctx.content_rect().center() - egui::vec2(180., 100.))
        .collapsible(false)
        .resizable(false)
        .title_bar(false)
        .frame(dialog_frame(ctx))
        .show(ctx, |ui| {
            let (enter_pressed, enter) = match form {
                Form::Input { .. } => (false, false),
                Form::Question { .. } | Form::List { .. } => form_enter(ui, focus_on_open),
            };
            // egui also reports Enter on a focused button as a click.
            let clicked = |response: &egui::Response| {
                response.clicked_by(egui::PointerButton::Primary)
                    || (response.clicked() && (!enter_pressed || enter))
            };
            window_chrome(ui, &title, Some(&mut open), |ui| {
                ui.set_min_width(320.);
                ui.spacing_mut().item_spacing = egui::vec2(tokens::S, tokens::S);
                match form {
                    Form::Input {
                        prompts,
                        answers,
                        lines,
                        focus,
                        ..
                    } => {
                        let mut submit = false;
                        for (index, prompt) in prompts.iter().enumerate() {
                            ui.label(prompt);
                            let height = lines.get(index).copied().unwrap_or(1);
                            let edit = if height > 1 {
                                egui::TextEdit::multiline(&mut answers[index]).desired_rows(height)
                            } else {
                                egui::TextEdit::singleline(&mut answers[index])
                            };
                            let response = ui.add(edit.desired_width(320.));
                            if index == 0 && std::mem::take(focus) {
                                response.request_focus();
                            }
                            if height <= 1
                                && response.lost_focus()
                                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                            {
                                submit = true;
                            }
                        }
                        ui.add_space(tokens::XS);
                        button_row(ui, |ui| {
                            let size = egui::vec2(72., tokens::CONTROL);
                            if ui.add(egui::Button::new("Cancel").min_size(size)).clicked() {
                                reply = Some(DialogReply::Cancelled);
                            }
                            if ui.add(egui::Button::new("OK").min_size(size)).clicked() || submit {
                                reply = Some(DialogReply::Answers(answers.clone()));
                            }
                        });
                    }
                    Form::Question {
                        text,
                        buttons,
                        default,
                        ..
                    } => {
                        for line in text.iter() {
                            ui.label(line.as_str());
                        }
                        ui.add_space(tokens::XS);
                        button_row(ui, |ui| {
                            let size = egui::vec2(72., tokens::CONTROL);
                            // Right to left: the last button is drawn first.
                            for button in buttons.iter().rev() {
                                let mut widget = egui::Button::new(button.as_str()).min_size(size);
                                if button == default {
                                    // The UI accent marks the default answer (one accent).
                                    let accent = tokens::palette(ui.ctx()).accent;
                                    widget = widget.stroke(egui::Stroke::new(1.5, accent));
                                }
                                if clicked(&ui.add(widget)) {
                                    reply = Some(DialogReply::Button(button.clone()));
                                }
                            }
                        });
                        if reply.is_none() && enter && buttons.contains(default) {
                            reply = Some(DialogReply::Button(default.clone()));
                        }
                    }
                    Form::List {
                        prompts,
                        items,
                        multiple,
                        selected,
                        last_clicked,
                        ok_label,
                        cancel_label,
                        list_size,
                        ..
                    } => {
                        let list_width = (list_size[0] as f32).clamp(200., 640.);
                        let list_height = (list_size[1] as f32).clamp(120., 480.);
                        ui.set_min_width(list_width.max(320.));
                        for line in prompts.iter() {
                            ui.label(line.as_str());
                        }
                        let modifiers = ui.input(|i| i.modifiers);
                        let mut confirm = false;
                        egui::Frame::group(ui.style())
                            .inner_margin(egui::Margin::same(tokens::XS as i8))
                            .show(ui, |ui| {
                                ui.set_min_width(list_width);
                                egui::ScrollArea::vertical()
                                    .id_salt("listdlg_scroll")
                                    .max_height(list_height)
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        ui.spacing_mut().item_spacing.y = tokens::XS;
                                        for (index, item) in items.iter().enumerate() {
                                            let is_selected = selected[index];
                                            let response = ui.add_sized(
                                                [ui.available_width(), tokens::CONTROL],
                                                egui::Button::selectable(
                                                    is_selected,
                                                    item.as_str(),
                                                ),
                                            );
                                            if response.clicked_by(egui::PointerButton::Primary)
                                                || (response.clicked() && !enter_pressed)
                                            {
                                                if *multiple && modifiers.shift {
                                                    let anchor = last_clicked.unwrap_or(index);
                                                    selected.fill(false);
                                                    selected[anchor.min(index)..=anchor.max(index)]
                                                        .fill(true);
                                                } else if *multiple
                                                    && (modifiers.ctrl || modifiers.command)
                                                {
                                                    selected[index] = !is_selected;
                                                } else {
                                                    selected.fill(false);
                                                    selected[index] = true;
                                                    confirm |= response.double_clicked();
                                                }
                                                *last_clicked = Some(index);
                                            }
                                        }
                                    });
                            });
                        ui.add_space(tokens::XS);
                        let can_confirm = *multiple || selected.iter().any(|&s| s);
                        button_row(ui, |ui| {
                            let size = egui::vec2(72., tokens::CONTROL);
                            let cancel =
                                ui.add(egui::Button::new(cancel_label.as_str()).min_size(size));
                            let ok_btn = ui.add_enabled(
                                can_confirm,
                                egui::Button::new(ok_label.as_str()).min_size(size),
                            );
                            if clicked(&cancel) || (enter && cancel.has_focus()) {
                                reply = Some(DialogReply::Cancelled);
                            } else if clicked(&ok_btn) || (can_confirm && (enter || confirm)) {
                                let chosen: Vec<usize> = selected
                                    .iter()
                                    .enumerate()
                                    .filter_map(|(i, &s)| s.then_some(i))
                                    .collect();
                                reply = Some(DialogReply::Selection(chosen));
                            }
                        });
                    }
                }
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    reply = Some(DialogReply::Cancelled);
                }
            })
        });
    if !open {
        reply = Some(DialogReply::Cancelled);
    }
    reply
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn ticket(request: DialogRequest, reply: Option<Sender<DialogReply>>) -> DialogTicket {
        DialogTicket {
            request,
            reply,
            output: Vec::new(),
            clear: false,
        }
    }

    #[test]
    fn uiwait_returns_when_the_window_closes() {
        let mut windows = DialogWindows::default();
        windows.receive(ticket(
            DialogRequest::Message {
                id: 1_000_001,
                title: "t".into(),
                text: vec!["hi".into()],
                icon: MessageIcon::None,
            },
            None,
        ));
        let (send, receive) = mpsc::channel();
        windows.receive(ticket(
            DialogRequest::WaitClosed {
                id: 1_000_001,
                timeout: None,
            },
            Some(send),
        ));
        assert!(receive.try_recv().is_err(), "still open");
        windows.receive(ticket(DialogRequest::Close { id: 1_000_001 }, None));
        assert_eq!(receive.try_recv(), Ok(DialogReply::Done));
        // A closed window answers at once.
        let (send, receive) = mpsc::channel();
        windows.receive(ticket(
            DialogRequest::WaitClosed {
                id: 1_000_001,
                timeout: None,
            },
            Some(send),
        ));
        assert_eq!(receive.try_recv(), Ok(DialogReply::Done));
    }

    #[test]
    fn a_new_form_cancels_a_stale_one_and_end_run_drops_it() {
        let mut windows = DialogWindows::default();
        let question = || DialogRequest::Question {
            title: String::new(),
            text: vec!["?".into()],
            buttons: vec!["Yes".into(), "No".into()],
            default: "Yes".into(),
        };
        let (first, first_reply) = mpsc::channel();
        windows.receive(ticket(question(), Some(first)));
        let (second, _second_reply) = mpsc::channel();
        windows.receive(ticket(question(), Some(second)));
        assert_eq!(first_reply.try_recv(), Ok(DialogReply::Cancelled));
        windows.end_run();
        assert!(windows.pending.is_none());
    }

    #[test]
    fn filters_become_extensions_and_pick_the_matching_index() {
        let filters = vec![
            FileFilter {
                patterns: vec!["*.m".into(), "*.mr".into()],
                label: "Scripts".into(),
            },
            FileFilter {
                patterns: vec!["*.CSV".into()],
                label: "CSV".into(),
            },
            FileFilter {
                patterns: vec!["*.*".into()],
                label: "All".into(),
            },
        ];
        assert_eq!(extensions(&filters[0]), vec!["m", "mr"]);
        assert_eq!(extensions(&filters[2]), vec!["*"]);
        let reply = files_reply(Some(vec!["/tmp/data.csv".into()]), &filters);
        assert_eq!(
            reply,
            DialogReply::Files {
                folder: "/tmp".into(),
                names: vec!["data.csv".into()],
                filter: 1,
            }
        );
        assert_eq!(files_reply(None, &filters), DialogReply::Cancelled);
    }

    /// MathWorks `uigetfile` shows every file for the default `*.*` filter
    /// (https://www.mathworks.com/help/matlab/ref/uigetfile.html). On macOS,
    /// rfd passes extensions to `setAllowedFileTypes`, so `"*"` hides ordinary
    /// files. Reference value: macOS extension list is empty; Linux/Windows
    /// keep `"*"`.
    #[test]
    fn review_1111_macos_all_files_filter_adds_no_extension() {
        let all = FileFilter {
            patterns: vec!["*".into(), "*.*".into()],
            label: "All files".into(),
        };
        let mixed = FileFilter {
            patterns: vec!["*.csv".into(), "*.*".into()],
            label: "CSV".into(),
        };
        assert_eq!(
            extensions_for(&all, PickerPlatform::Macos),
            Vec::<String>::new()
        );
        assert_eq!(
            extensions_for(&all, PickerPlatform::Other),
            vec!["*".to_owned(), "*".to_owned()]
        );
        assert_eq!(extensions_for(&mixed, PickerPlatform::Macos), vec!["csv"]);
        assert_eq!(
            extensions_for(&mixed, PickerPlatform::Other),
            vec!["csv".to_owned(), "*".to_owned()]
        );
    }

    /// rfd on macOS unions every filter into one allowed-types list. A wildcard
    /// in any filter (`*.*` or `*`, alone or beside `*.csv`) drops the whole
    /// native list. Windows and Linux still receive each filter.
    #[test]
    fn review_s1113_macos_wildcard_drops_every_native_extension() {
        let csv = FileFilter {
            patterns: vec!["*.csv".into()],
            label: "CSV".into(),
        };
        let all = FileFilter {
            patterns: vec!["*.*".into()],
            label: "All files".into(),
        };
        let star = FileFilter {
            patterns: vec!["*".into()],
            label: "All files".into(),
        };
        let mixed = FileFilter {
            patterns: vec!["*.csv".into(), "*.*".into()],
            label: "CSV".into(),
        };
        assert!(native_filters(&[csv.clone(), all.clone()], PickerPlatform::Macos).is_empty());
        assert!(native_filters(&[csv.clone(), star], PickerPlatform::Macos).is_empty());
        assert!(native_filters(&[mixed], PickerPlatform::Macos).is_empty());
        let other = native_filters(&[csv.clone(), all], PickerPlatform::Other);
        assert_eq!(other.len(), 2);
        assert_eq!(other[0].1, vec!["csv".to_owned()]);
        assert_eq!(other[1].1, vec!["*".to_owned()]);
        let only = native_filters(&[csv], PickerPlatform::Macos);
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].1, vec!["csv".to_owned()]);
    }

    /// MathWorks `uigetfile` returns filter index 0 when no filter is selected
    /// (https://www.mathworks.com/help/matlab/ref/uigetfile.html: Cancel is 0
    /// for every output). A name that matches no pattern must not claim the
    /// first filter. Reference value: `filter` is `usize::MAX` (engine index 0)
    /// with the file and folder still returned. Cancel stays
    /// [`DialogReply::Cancelled`].
    #[test]
    fn review_1111_unmatched_name_has_no_filter_index() {
        let filters = vec![
            FileFilter {
                patterns: vec!["*.csv".into()],
                label: "CSV".into(),
            },
            FileFilter {
                patterns: vec!["*.txt".into()],
                label: "Text".into(),
            },
        ];
        let reply = files_reply(Some(vec!["/tmp/out.md".into()]), &filters);
        let DialogReply::Files {
            folder,
            names,
            filter,
        } = reply
        else {
            panic!("{reply:?}");
        };
        assert_eq!(folder, std::path::PathBuf::from("/tmp"));
        assert_eq!(names, vec!["out.md".to_owned()]);
        // The engine maps this sentinel to index 0. Reference value: usize::MAX.
        assert_eq!(filter, usize::MAX);
        assert_eq!(files_reply(None, &filters), DialogReply::Cancelled);
    }

    /// A Linux file name is raw bytes. `OsStr::to_string_lossy` inserts U+FFFD
    /// (Unicode replacement character), which is not the picked name.
    /// Reference value: [`DialogReply::Failed`], and the reply text does not
    /// contain U+FFFD. untested on macOS.
    #[cfg(unix)]
    #[test]
    fn review_1111_non_utf8_pick_is_an_error() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        let bad_name = std::path::PathBuf::from("/tmp").join(OsString::from_vec(vec![0xff, 0xfe]));
        let reply = files_reply(Some(vec![bad_name]), &[]);
        let DialogReply::Failed(why) = reply else {
            panic!("expected an error reply, got {reply:?}");
        };
        assert!(!why.contains('\u{FFFD}'));
        let bad_folder = std::path::PathBuf::from("/tmp")
            .join(OsString::from_vec(vec![0xff]))
            .join("ok.txt");
        let reply = files_reply(Some(vec![bad_folder]), &[]);
        match reply {
            DialogReply::Failed(why) => assert!(!why.contains('\u{FFFD}')),
            other => panic!("expected an error reply, got {other:?}"),
        }
    }

    fn list_req(items: Vec<String>, multiple: bool, initial: Vec<usize>) -> DialogRequest {
        DialogRequest::List {
            title: "Pick".into(),
            prompts: vec!["Choose:".into()],
            items,
            multiple,
            initial,
            ok_label: "OK".into(),
            cancel_label: "Cancel".into(),
            list_size: [200., 220.],
        }
    }

    fn key_press(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn fix15_listdlg_single_multiple_cancel_initial_and_scroll() {
        let ctx = egui::Context::default();
        crate::theme::configure(&ctx);
        let mut windows = DialogWindows::default();
        for (multiple, initial) in [(false, vec![1]), (true, vec![0, 2])] {
            let (send, recv) = mpsc::channel();
            windows.receive(ticket(
                list_req(
                    vec!["a".into(), "b".into(), "c".into()],
                    multiple,
                    initial.clone(),
                ),
                Some(send),
            ));
            let enter = egui::RawInput {
                events: vec![key_press(egui::Key::Enter)],
                ..Default::default()
            };
            list_frame(&ctx, &mut windows, enter.clone());
            assert_eq!(recv.try_recv(), Err(mpsc::TryRecvError::Empty));
            list_frame(&ctx, &mut windows, Default::default());
            list_frame(&ctx, &mut windows, enter);
            assert_eq!(recv.try_recv(), Ok(DialogReply::Selection(initial)));
        }

        // Escape cancels.
        let (send, recv) = mpsc::channel();
        windows.receive(ticket(
            list_req(vec!["a".into(), "b".into()], false, vec![0]),
            Some(send),
        ));
        let escape = egui::RawInput {
            events: vec![key_press(egui::Key::Escape)],
            ..Default::default()
        };
        list_frame(&ctx, &mut windows, escape);
        assert_eq!(recv.try_recv(), Ok(DialogReply::Cancelled));

        // A 200-item list fits in a bounded ScrollArea and does not overflow the screen height.
        let (send, _recv) = mpsc::channel();
        let items = (1..=200).map(|i| format!("Item {i}")).collect();
        windows.receive(ticket(list_req(items, true, vec![0]), Some(send)));
        list_frame(&ctx, &mut windows, Default::default());
        list_frame(&ctx, &mut windows, Default::default());
        let window_rect = ctx.globally_used_rect();
        assert!(
            window_rect.height() < 600.,
            "200-item list should scroll inside bounded height, got {}",
            window_rect.height()
        );
    }

    fn list_frame(
        ctx: &egui::Context,
        windows: &mut DialogWindows,
        mut input: egui::RawInput,
    ) -> egui::FullOutput {
        input.screen_rect = Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(800., 600.),
        ));
        crate::headless::run(ctx, input, |ctx| windows.show(ctx))
    }

    fn open_list_for_clicks() -> (
        egui::Context,
        DialogWindows,
        mpsc::Receiver<DialogReply>,
        egui::FullOutput,
    ) {
        let ctx = egui::Context::default();
        crate::theme::configure(&ctx);
        ctx.enable_accesskit();
        let (send, recv) = mpsc::channel();
        let mut windows = DialogWindows::default();
        let items = ["a", "b", "c", "d", "e"].map(str::to_owned).to_vec();
        windows.receive(ticket(list_req(items, true, vec![0]), Some(send)));
        list_frame(&ctx, &mut windows, Default::default());
        let output = list_frame(&ctx, &mut windows, Default::default());
        (ctx, windows, recv, output)
    }

    fn list_button(
        output: &egui::FullOutput,
        label: &str,
    ) -> (egui::accesskit::NodeId, egui::Pos2) {
        output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find_map(|(id, node)| {
                let bounds = node.bounds()?;
                (node.label() == Some(label)).then(|| {
                    let x = ((bounds.x0 + bounds.x1) / 2.) as f32;
                    let y = ((bounds.y0 + bounds.y1) / 2.) as f32;
                    (*id, egui::pos2(x, y))
                })
            })
            .unwrap_or_else(|| panic!("missing button {label}"))
    }

    fn list_click(
        ctx: &egui::Context,
        windows: &mut DialogWindows,
        pos: egui::Pos2,
        modifiers: egui::Modifiers,
        time: f64,
        mut on_release: Option<egui::Event>,
    ) {
        for pressed in [true, false] {
            let mut events = vec![
                egui::Event::ModifiersChanged(modifiers),
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers,
                },
            ];
            if !pressed {
                events.extend(on_release.take());
            }
            list_frame(
                ctx,
                windows,
                egui::RawInput {
                    events,
                    time: Some(if pressed { time } else { time + 0.05 }),
                    ..Default::default()
                },
            );
        }
    }

    #[test]
    fn fix15_listdlg_multiple_pointer_selection() {
        use egui::Modifiers as M;

        // (clicks with modifiers, expected selected rows)
        type Case<'a> = (&'a [(&'a str, M)], &'a [usize]);
        let cases: &[Case] = &[
            (&[("b", M::NONE)], &[1]),
            (
                &[
                    ("b", M::NONE),
                    ("d", M::CTRL),
                    ("a", M::COMMAND),
                    ("d", M::COMMAND),
                ],
                &[0, 1],
            ),
            (
                &[("a", M::NONE), ("e", M::CTRL), ("c", M::SHIFT)],
                &[2, 3, 4],
            ),
        ];
        for &(clicks, expected) in cases {
            let (ctx, mut windows, recv, output) = open_list_for_clicks();
            for (step, (label, modifiers)) in clicks.iter().copied().enumerate() {
                let (_, pos) = list_button(&output, label);
                // Space out clicks so egui reports each as a single click.
                list_click(&ctx, &mut windows, pos, modifiers, 1. + step as f64, None);
            }
            let (_, pos) = list_button(&output, "OK");
            list_click(
                &ctx,
                &mut windows,
                pos,
                M::NONE,
                1. + clicks.len() as f64,
                None,
            );
            assert_eq!(
                recv.try_recv(),
                Ok(DialogReply::Selection(expected.to_vec())),
                "{clicks:?}"
            );
        }
    }

    #[test]
    fn fix15_dialog_enter_follows_window_focus() {
        let cases = [
            (
                list_req(vec!["a".into(), "b".into()], false, vec![1]),
                DialogReply::Selection(vec![1]),
            ),
            (
                DialogRequest::Question {
                    title: "Pick".into(),
                    text: vec!["Continue?".into()],
                    buttons: vec!["Yes".into(), "No".into()],
                    default: "Yes".into(),
                },
                DialogReply::Button("Yes".into()),
            ),
        ];
        for (request, expected) in cases {
            let ctx = egui::Context::default();
            crate::theme::configure(&ctx);
            ctx.enable_accesskit();
            let mut windows = DialogWindows::default();
            let (send, recv) = mpsc::channel();
            windows.receive(ticket(request, Some(send)));
            let mut editor = String::new();
            let editor_id = egui::Id::new("test_editor");
            let mut frame = |events, focused| {
                crate::headless::run(
                    &ctx,
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(800., 600.),
                        )),
                        events,
                        focused,
                        ..Default::default()
                    },
                    |ui| {
                        windows.show(ui.ctx());
                        egui::Window::new("Editor")
                            .fixed_pos(egui::pos2(5., 5.))
                            .show(ui.ctx(), |ui| {
                                ui.add(
                                    egui::TextEdit::multiline(&mut editor)
                                        .id(editor_id)
                                        .desired_width(150.),
                                );
                            });
                    },
                )
            };
            frame(vec![], true);
            let output = frame(vec![], true);
            let editor_pos = ctx.read_response(editor_id).unwrap().rect.center();
            let (_, close_pos) = list_button(&output, "Close");
            let title_bar_pos = close_pos - egui::vec2(2. * TITLE_BAR, 0.);
            let enter = key_press(egui::Key::Enter);
            let pointer = |pos, pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };

            frame(vec![enter.clone()], false);
            assert_eq!(recv.try_recv(), Err(mpsc::TryRecvError::Empty));
            frame(
                vec![
                    egui::Event::PointerMoved(editor_pos),
                    pointer(editor_pos, true),
                    enter.clone(),
                ],
                true,
            );
            assert_eq!(recv.try_recv(), Err(mpsc::TryRecvError::Empty));
            frame(vec![pointer(editor_pos, false)], true);
            frame(vec![enter.clone()], true);
            assert_eq!(recv.try_recv(), Err(mpsc::TryRecvError::Empty));

            frame(
                vec![
                    egui::Event::PointerMoved(title_bar_pos),
                    pointer(title_bar_pos, true),
                ],
                true,
            );
            frame(vec![pointer(title_bar_pos, false)], true);
            frame(vec![enter], true);
            assert_eq!(recv.try_recv(), Ok(expected));
            assert!(editor.contains('\n'), "Enter should reach the editor");
        }
    }

    fn run_list_script(source: &str, clicks: &[(&str, egui::Modifiers)]) {
        use crate::worker::{Job, Worker};
        use std::time::Duration;

        let ctx = egui::Context::default();
        crate::theme::configure(&ctx);
        ctx.enable_accesskit();
        let worker = Worker::new(ctx.clone());
        let folder = std::env::current_dir().unwrap();
        let request = worker.request(Job::UnsavedDocumentInFolder(source.into(), folder));
        worker.send.send(request).unwrap();
        let timeout = Duration::from_secs(10);
        let mut windows = DialogWindows::default();
        windows.receive(
            worker
                .dialogs
                .recv_timeout(timeout)
                .expect("listdlg request"),
        );
        list_frame(&ctx, &mut windows, Default::default());
        let output = list_frame(&ctx, &mut windows, Default::default());
        for (step, (label, modifiers)) in clicks.iter().copied().enumerate() {
            let (_, pos) = list_button(&output, label);
            list_click(&ctx, &mut windows, pos, modifiers, 1. + step as f64, None);
        }
        let finished = worker
            .receive
            .recv_timeout(timeout)
            .expect("script finished");
        finished.result.unwrap();
    }

    #[test]
    fn fix15_listdlg_row_two_reaches_script_outputs() {
        run_list_script(
            r"[idx,tf]=listdlg('ListString',{sprintf('a\nb'),'c'},'SelectionMode','single','InitialValue',3);
              assert(isequal(idx,2) && tf==1);",
            &[("b", egui::Modifiers::NONE), ("OK", egui::Modifiers::NONE)],
        );
    }

    #[test]
    fn fix15_listdlg_empty_confirmation_reaches_script_outputs() {
        run_list_script(
            "[idx,tf]=listdlg('ListString',{'a'},'SelectionMode','multiple');
             assert(isequal(idx,[]) && tf==1);",
            &[("a", egui::Modifiers::CTRL), ("OK", egui::Modifiers::NONE)],
        );
    }

    #[test]
    fn fix15_listdlg_cancel_takes_priority_over_enter() {
        use egui::accesskit::{Action, ActionRequest, TreeId};

        for click_cancel in [false, true] {
            let (ctx, mut windows, recv, output) = open_list_for_clicks();
            let (cancel_id, pos) = list_button(&output, "Cancel");
            let enter = egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            };
            if click_cancel {
                list_click(
                    &ctx,
                    &mut windows,
                    pos,
                    egui::Modifiers::NONE,
                    1.,
                    Some(enter),
                );
            } else {
                let focus = egui::Event::AccessKitActionRequest(ActionRequest {
                    action: Action::Focus,
                    target_node: cancel_id,
                    target_tree: TreeId::ROOT,
                    data: None,
                });
                list_frame(
                    &ctx,
                    &mut windows,
                    egui::RawInput {
                        events: vec![focus],
                        ..Default::default()
                    },
                );
                assert_eq!(
                    ctx.memory(|m| m.focused().map(|id| id.accesskit_id())),
                    Some(cancel_id)
                );
                list_frame(
                    &ctx,
                    &mut windows,
                    egui::RawInput {
                        events: vec![enter],
                        ..Default::default()
                    },
                );
            }
            assert_eq!(
                recv.try_recv(),
                Ok(DialogReply::Cancelled),
                "click_cancel = {click_cancel}"
            );
        }
    }
}
