//! CPU interaction tests inspect shapes and commands without a GPU backend.
use eframe::egui;

pub(crate) fn run(
    ctx: &egui::Context,
    mut input: egui::RawInput,
    draw: impl FnMut(&mut egui::Ui),
) -> egui::FullOutput {
    super::input::sanitize(&mut input);
    let mut output = ctx.run_ui(input, draw);
    // A native eframe renderer consumes these uploads. Headless tests have no
    // GPU and intentionally discard them, while retaining shapes and commands.
    output.textures_delta.clear();
    output
}
