//! Product chrome: banner, status texts, window title, Help menu, About box,
//! the generated window icon and first-run empty states.
use super::*;

pub(super) const RUNNING: &str = "Running... Press Stop to cancel";
pub(super) const NO_SCRIPT: &str = "No script open";
pub(super) const NO_FIGURE: &str = "No figures yet";
pub(super) const NO_FIGURE_HINT: &str =
    "Run plot(1:10) in the Command Window, or open a figure file.";
pub(super) const NO_VARIABLES: &str = "No variables yet. Try x = 1:10";
/// The desktop does not install licence files beside the executable; About
/// names where the distribution keeps them.
const NOTICES: &str =
    "Licence texts are in the MatRust folder: LICENSE, THIRD-PARTY-LICENSES.txt and notices/.";
const MAX_RECENT: usize = 5;

#[derive(Default)]
pub(super) struct State {
    pub(super) about: bool,
    notices: bool,
    /// Last title sent to the root viewport; avoids one command per frame.
    title: String,
}

/// The "My script does not run" issue form, with the version and system filled in.
pub(super) fn report_url() -> String {
    format!(
        "https://github.com/leoom-labs/matrust/issues/new?template=script.yml&version={}&os={}",
        matrust::VERSION,
        std::env::consts::OS
    )
}

pub(super) fn banner() -> Vec<String> {
    vec![
        format!("MatRust {}", matrust::VERSION),
        "Type help to list functions, or help plot for one function.".into(),
    ]
}

pub(super) fn variable_count(count: usize) -> String {
    match count {
        0 => "No variables".into(),
        1 => "1 variable".into(),
        n => format!("{n} variables"),
    }
}

/// Centres `add` vertically in `height`, using the content height measured
/// on the previous pass (one repaint settles a changed height).
pub(super) fn empty_state(
    ui: &mut egui::Ui,
    salt: &str,
    height: f32,
    add: impl FnOnce(&mut egui::Ui),
) {
    let id = ui.id().with(("empty_state", salt));
    let measured = ui.data(|d| d.get_temp::<f32>(id)).unwrap_or(0.);
    if height.is_finite() {
        ui.add_space(((height - measured) * 0.5).max(0.));
    }
    let content = ui.vertical_centered(add).response.rect.height();
    if (content - measured).abs() > 0.5 {
        ui.data_mut(|d| d.insert_temp(id, content));
        ui.ctx().request_repaint();
    }
}

pub(super) const ICON_SIZE: u32 = 256;
/// The window icon uses the brand mark's accent fill.
pub(super) fn icon_accent() -> [u8; 3] {
    let accent = tokens::LIGHT.accent_fill;
    [accent.r(), accent.g(), accent.b()]
}

/// Rounded accent square with one white sine period, anti-aliased by the
/// pixel-centre distance to the square edge and to the sampled curve.
pub(super) fn window_icon() -> egui::IconData {
    let accent = icon_accent();
    const SEGMENTS: usize = 128;
    let size = ICON_SIZE as f32;
    let centre = size * 0.5;
    let radius = 0.22 * size;
    let half_stroke = 0.04 * size;
    let (left, width, amplitude) = (0.15 * size, 0.7 * size, 0.2 * size);
    let wave: Vec<egui::Pos2> = (0..=SEGMENTS)
        .map(|i| {
            let t = i as f32 / SEGMENTS as f32;
            egui::pos2(
                left + t * width,
                centre - amplitude * (std::f32::consts::TAU * t).sin(),
            )
        })
        .collect();
    let reach = half_stroke + 1.;
    let distance = |p: egui::Pos2| {
        if (p.y - centre).abs() > amplitude + reach
            || p.x < left - reach
            || p.x > left + width + reach
        {
            return f32::INFINITY;
        }
        wave.windows(2)
            .map(|s| {
                let (a, b) = (s[0], s[1]);
                let t = ((p - a).dot(b - a) / (b - a).length_sq()).clamp(0., 1.);
                (p - (a + (b - a) * t)).length()
            })
            .fold(f32::INFINITY, f32::min)
    };
    let mut rgba = Vec::with_capacity((ICON_SIZE * ICON_SIZE * 4) as usize);
    for y in 0..ICON_SIZE {
        for x in 0..ICON_SIZE {
            let p = egui::pos2(x as f32 + 0.5, y as f32 + 0.5);
            let q = egui::vec2(
                (p.x - centre).abs() - (centre - radius),
                (p.y - centre).abs() - (centre - radius),
            );
            let edge = q.max(egui::Vec2::ZERO).length() + q.x.max(q.y).min(0.) - radius;
            let alpha = (0.5 - edge).clamp(0., 1.);
            let ink = (half_stroke + 0.5 - distance(p)).clamp(0., 1.);
            for channel in accent {
                let channel = f32::from(channel);
                rgba.push((channel + (255. - channel) * ink).round() as u8);
            }
            rgba.push((alpha * 255.).round() as u8);
        }
    }
    egui::IconData {
        rgba,
        width: ICON_SIZE,
        height: ICON_SIZE,
    }
}

impl Workspace {
    pub(super) fn window_title(&self) -> String {
        if self.has_startup_placeholder() {
            return "MatRust".into();
        }
        let document = &self.documents[self.active];
        let name = document
            .path
            .as_ref()
            .and_then(|path| path.file_name())
            .map_or_else(
                || "Untitled.mr".into(),
                |name| name.to_string_lossy().into_owned(),
            );
        format!("{}{name} - MatRust", if document.dirty { "*" } else { "" })
    }

    pub(super) fn update_window_title(&mut self, ctx: &egui::Context) {
        let title = self.window_title();
        if self.chrome.title != title {
            ctx.send_viewport_cmd_to(
                egui::ViewportId::ROOT,
                egui::ViewportCommand::Title(title.clone()),
            );
            self.chrome.title = title;
        }
    }

    pub(super) fn help_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Help", |ui| {
            if ui
                .add_enabled(!self.busy, egui::Button::new("Function help"))
                .on_hover_text("Type help and a function name in the Command Window")
                .clicked()
            {
                self.function_help(ui.ctx());
                ui.close();
            }
            if ui
                .button("Keyboard shortcuts")
                .on_hover_text("Ctrl+/")
                .clicked()
            {
                self.queue_navigation_help(ui.ctx());
                ui.close();
            }
            if ui
                .button("Report a problem")
                .on_hover_text("Opens a GitHub issue form in your browser")
                .clicked()
            {
                let (program, args) = matrust::bench::share::opener(&report_url());
                if std::process::Command::new(program)
                    .args(args)
                    .spawn()
                    .is_err()
                {
                    self.status = format!("Could not open a browser. Report at {}", report_url());
                }
                ui.close();
            }
            if ui.button("About MatRust").clicked() {
                self.chrome.about = true;
                ui.close();
            }
        });
    }

    fn function_help(&mut self, ctx: &egui::Context) {
        self.command = "help ".into();
        let id = egui::Id::new("command_input");
        let mut state = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(self.command.chars().count()),
            )));
        state.store(ctx, id);
        self.queue_pane_focus(ctx, navigation::FocusPane::Command);
    }

    pub(super) fn about_ui(&mut self, ctx: &egui::Context) {
        if !self.chrome.about {
            return;
        }
        let response = egui::Modal::new(egui::Id::new("about_matrust")).show(ctx, |ui| {
            ui.set_max_width(400.);
            ui.heading("MatRust");
            ui.label(format!("Version {}", matrust::VERSION));
            ui.add_space(8.);
            ui.label("Numerical workspace for a subset of the MATLAB language.");
            ui.label("Free software under the GNU GPL, version 3 or later. No warranty.");
            ui.label("Fonts: Inter and JetBrains Mono (SIL Open Font License 1.1).");
            ui.add_space(8.);
            if ui.button("Third-party notices").clicked() {
                self.chrome.notices = !self.chrome.notices;
            }
            if self.chrome.notices {
                ui.weak(NOTICES);
            }
            ui.add_space(8.);
            ui.button("Close").clicked()
        });
        if response.inner || response.should_close() {
            self.chrome.about = false;
            self.chrome.notices = false;
        }
    }

    pub(super) fn editor_empty_state(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let height = ui.available_height();
        let mut open = None;
        empty_state(ui, "editor", height, |ui| {
            ui.label(
                RichText::new(NO_SCRIPT)
                    .text_style(egui::TextStyle::Heading)
                    .color(ui.visuals().weak_text_color()),
            );
            ui.add_space(12.);
            let new = toolstrip::Action::NewScript;
            let visuals = &ui.visuals().selection;
            let primary = egui::Button::new(RichText::new(new.label()).color(visuals.stroke.color))
                .fill(visuals.bg_fill);
            if ui.add_enabled(new.enabled(self), primary).clicked() {
                new.invoke(self, &ctx);
            }
            ui.add_space(4.);
            let open_script = toolstrip::Action::OpenScript;
            if ui
                .add_enabled(
                    open_script.enabled(self),
                    egui::Link::new(format!("{}...", open_script.label())),
                )
                .clicked()
            {
                open_script.invoke(self, &ctx);
            }
            if !self.recent.is_empty() {
                ui.add_space(12.);
                ui.weak("Recent");
                for path in self.recent.iter().take(MAX_RECENT) {
                    let full = crate::path_label::display(path).to_string();
                    let label = path
                        .file_name()
                        .map_or_else(|| full.clone(), |n| n.to_string_lossy().into_owned());
                    if ui
                        .add_enabled(!self.busy, egui::Link::new(label))
                        .on_hover_text(full)
                        .clicked()
                    {
                        open = Some(path.clone());
                    }
                }
            }
        });
        if let Some(path) = open {
            self.open_path(path);
        }
    }

    pub(super) fn figure_empty_state(&mut self, ui: &mut egui::Ui) {
        let height = ui.available_height();
        empty_state(ui, "figure", height, |ui| {
            ui.label(
                RichText::new(NO_FIGURE)
                    .text_style(egui::TextStyle::Heading)
                    .color(ui.visuals().weak_text_color()),
            );
            ui.add_space(4.);
            ui.weak(NO_FIGURE_HINT);
        });
    }
}
