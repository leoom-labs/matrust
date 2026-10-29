//! Small scalar block controls; port edits are captured atomically by Properties.
use eframe::egui;
use matrust::simulation::{Block, Node, ProductInput, SwitchCriteria};

pub(super) fn parameters(
    ui: &mut egui::Ui,
    node: &mut Node,
    interactive: bool,
) -> Option<egui::Response> {
    let mut edited = None;
    match &mut node.block {
        Block::Product { operations } => {
            if !(1..=32).contains(&operations.len()) || operations.len() != node.inputs.len() {
                ui.label("Product requires 1–32 operations and matching input ports.");
                return None;
            }
            for (port, operation) in operations.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!("Operation {}", port + 1));
                    egui::ComboBox::from_id_salt(("product_operation", port))
                        .selected_text(operation.label())
                        .show_ui(ui, |ui| {
                            for choice in [ProductInput::Multiply, ProductInput::Divide] {
                                ui.selectable_value(operation, choice, choice.label());
                            }
                        });
                });
            }
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(operations.len() < 32, egui::Button::new("Add input"))
                    .clicked()
                    && interactive
                {
                    operations.push(ProductInput::Multiply);
                    node.inputs.push(usize::MAX);
                }
                let removable = operations.len() > 1 && node.inputs.last() == Some(&usize::MAX);
                if ui
                    .add_enabled(removable, egui::Button::new("Remove last input"))
                    .on_hover_text("Disconnect the last port before removing it.")
                    .clicked()
                    && interactive
                {
                    operations.pop();
                    node.inputs.pop();
                }
            });
        }
        Block::Switch {
            criteria,
            threshold,
        } => {
            ui.small("Input 1: true data · Input 2: control · Input 3: false data");
            egui::ComboBox::from_id_salt("switch_criteria")
                .selected_text(criteria.label())
                .show_ui(ui, |ui| {
                    for choice in [
                        SwitchCriteria::Greater,
                        SwitchCriteria::GreaterEqual,
                        SwitchCriteria::Nonzero,
                    ] {
                        ui.selectable_value(criteria, choice, choice.label());
                    }
                });
            ui.horizontal(|ui| {
                ui.label("Threshold");
                edited = Some(ui.add_enabled(
                    *criteria != SwitchCriteria::Nonzero,
                    egui::DragValue::new(threshold).speed(0.01),
                ));
            });
        }
        Block::UnaryMinus => {
            ui.small("Output = −Input 1");
        }
        _ => {}
    }
    edited
}
