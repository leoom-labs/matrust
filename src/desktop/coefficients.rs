use crate::theme::ThemedScrollArea;
use eframe::egui;
use matrust::{
    Result,
    dsp::Filter,
    simulation::{Block, Model},
};

pub struct Editor {
    node: usize,
    original: String,
    sos: bool,
    numerator: String,
    denominator: String,
    message: String,
}
fn coefficient_text(ui: &mut egui::Ui, id: &str, text: &mut String, rows: usize, height: f32) {
    egui::ScrollArea::both()
        .id_salt(id)
        .max_height(height)
        .auto_shrink([false, true])
        .show_themed(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(text)
                    .id_salt(id)
                    .desired_width(f32::INFINITY)
                    .desired_rows(rows)
                    .char_limit(262144)
                    .code_editor(),
            );
        });
}
fn vector(text: &str) -> Result<Vec<f64>> {
    if text.len() > 262144 {
        return Err("Coefficient text is too long".into());
    }
    let mut values = Vec::new();
    for token in text
        .split(|c: char| c.is_whitespace() || c == ',' || c == '[' || c == ']')
        .filter(|s| !s.is_empty())
    {
        if values.len() > matrust::dsp::MAX_ORDER {
            return Err("Coefficient limit exceeded".into());
        }
        let value = token.parse::<f64>().map_err(|_| {
            format!(
                "Invalid coefficient: {}",
                token.chars().take(40).collect::<String>()
            )
        })?;
        if !value.is_finite() {
            return Err("Coefficients must be finite".into());
        }
        values.push(value);
    }
    if values.is_empty() {
        return Err("Enter at least one coefficient".into());
    }
    Ok(values)
}
fn parse(numerator: &str, denominator: &str, sos: bool) -> Result<Block> {
    if sos {
        if numerator.len() > 262144 {
            return Err("Coefficient text is too long".into());
        }
        let mut sections = Vec::new();
        for row in numerator
            .split([';', '\n'])
            .filter(|s| !s.trim().is_empty())
        {
            if sections.len() >= 64 {
                return Err("SOS limit is 64 sections".into());
            }
            let values = vector(row)?;
            let section: [f64; 6] = values
                .try_into()
                .map_err(|_| "Each SOS row needs six numbers: b0 b1 b2 a0 a1 a2")?;
            Filter::new(&section[..3], &section[3..])?;
            sections.push(section);
        }
        if sections.is_empty() {
            return Err("Enter at least one SOS row".into());
        }
        Ok(Block::SosFilter { sections })
    } else {
        let b = vector(numerator)?;
        let a = vector(denominator)?;
        Filter::new(&b, &a)?;
        Ok(Block::Filter { b, a })
    }
}
fn format(values: &[f64]) -> String {
    values
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}
impl Editor {
    pub fn new(node: usize, block: &Block) -> Option<Self> {
        let (sos, numerator, denominator) = match block {
            Block::Filter { b, a } => (false, format(b), format(a)),
            Block::SosFilter { sections } => (
                true,
                sections
                    .iter()
                    .map(|s| format(s))
                    .collect::<Vec<_>>()
                    .join("\n"),
                String::new(),
            ),
            _ => return None,
        };
        Some(Self {
            node,
            original: serde_json::to_string(block).ok()?,
            sos,
            numerator,
            denominator,
            message: String::new(),
        })
    }
    fn apply(&self, model: &mut Model, canvas: &mut super::model_canvas::Canvas) -> Result<()> {
        let block = parse(&self.numerator, &self.denominator, self.sos)?;
        let node = model
            .nodes
            .get_mut(self.node)
            .ok_or("Block was removed; reopen the coefficient editor")?;
        if serde_json::to_string(&node.block).ok().as_deref() != Some(&self.original) {
            return Err("Block changed; reopen the coefficient editor".into());
        }
        canvas.replace_coefficients(model, self.node, block)
    }
    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        model: &mut Model,
        canvas: &mut super::model_canvas::Canvas,
    ) -> bool {
        let mut close = false;
        let width = (ctx.content_rect().width() - 48.).clamp(180., 580.);
        let body_height = (ctx.content_rect().height() - 150.).clamp(60., 420.);
        let shown = egui::Modal::new(egui::Id::new("filter_coefficients")).show(ctx, |ui| {
            ui.set_width(width);
            ui.label(crate::tokens::title(ui.ctx(), "Filter coefficients"));
            egui::ScrollArea::vertical().id_salt("coefficient_body")
                .max_height(body_height).auto_shrink([false, true]).show_themed(ui, |ui| {
                ui.label(if self.sos {"One SOS row per line: b0 b1 b2 a0 a1 a2. Up to 64 sections."}else{"Numerator b, in ascending delay order: b0 b1 b2 ..."});
                coefficient_text(ui, "numerator", &mut self.numerator, 4, (body_height * 0.55).clamp(48., 200.));
                if !self.sos {
                    ui.label("Denominator a: a0 a1 a2 ... (a0 must be nonzero)");
                    coefficient_text(ui, "denominator", &mut self.denominator, 2, (body_height * 0.3).clamp(36., 120.));
                }
                ui.small("Use spaces or commas between finite numbers. Apply validates all coefficients; each simulation run starts with zero filter state.");
            });
            ui.separator();
            if !self.message.is_empty() {
                ui.colored_label(crate::tokens::palette(ui.ctx()).error, &self.message);
            }
            ui.horizontal(|ui| {
                if ui.button("Apply coefficients").clicked() {
                    match self.apply(model, canvas) { Ok(()) => close = true, Err(e) => self.message = e }
                }
                if ui.button("Cancel").clicked() { close = true; }
            });
        });
        if shown.is_top_modal
            && !shown.any_popup_open
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            close = true;
        }
        close
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn coefficient_edits_are_validated_and_transactional() {
        let mut model = Model::default();
        let mut canvas = super::super::model_canvas::Canvas::default();
        let before = serde_json::to_string(&model).unwrap();
        let mut editor = Editor::new(3, &model.nodes[3].block).unwrap();
        editor.numerator = "[0.25, 0.5, 0.25]".into();
        editor.denominator = "0".into();
        assert!(editor.apply(&mut model, &mut canvas).is_err());
        assert_eq!(serde_json::to_string(&model).unwrap(), before);
        editor.denominator = "1 -0.25".into();
        editor.apply(&mut model, &mut canvas).unwrap();
        let Block::Filter { b, a } = &model.nodes[3].block else {
            panic!()
        };
        assert_eq!(b, &[0.25, 0.5, 0.25]);
        assert_eq!(a, &[1., -0.25]);
        assert!(editor.apply(&mut model, &mut canvas).is_err());
        assert!(parse("1 0 0 1 -0.5 0;1 0 0 1 -0.25 0", "", true).is_ok());
        for text in ["1 0 0 0 0 0", "1 2 3", "NaN 0 0 1 0 0"] {
            assert!(parse(text, "", true).is_err());
        }
        assert!(parse(&"1 ".repeat(4098), "1", false).is_err());
    }
}

#[cfg(test)]
#[path = "coefficient_ui_tests.rs"]
mod ui_tests;
