//! Figure-owned literal textbox annotations in normalized figure coordinates.
use super::{Figure, Result};
use serde::{Deserialize, Serialize};

pub const MAX_TEXTBOXES: usize = 64;
pub const MAX_TEXTBOX_BYTES: usize = 16_384;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextBox {
    /// Process-local only: clone retains identity; decoding creates a new token.
    #[serde(skip)]
    pub identity: crate::ObjectIdentity,
    pub position: [f64; 4],
    pub text: String,
    pub horizontal: TextAlign,
    pub bold: bool,
    pub font_size: f64,
    pub color: [u8; 3],
    pub visible: bool,
}
// Appearance/recovery comparisons deliberately exclude process-local identity.
impl PartialEq for TextBox {
    fn eq(&self, other: &Self) -> bool {
        self.position == other.position
            && self.text == other.text
            && self.horizontal == other.horizontal
            && self.bold == other.bold
            && self.font_size == other.font_size
            && self.color == other.color
            && self.visible == other.visible
    }
}
impl TextBox {
    pub fn new(position: [f64; 4], text: String) -> Result<Self> {
        let result = Self {
            identity: crate::ObjectIdentity::default(),
            position,
            text,
            horizontal: TextAlign::Left,
            bold: false,
            font_size: 10.,
            color: [0; 3],
            visible: true,
        };
        result.validate()?;
        Ok(result)
    }
    pub fn validate(&self) -> Result<()> {
        let [x, y, w, h] = self.position;
        if self
            .position
            .iter()
            .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
            || x + w > 1. + 1e-12
            || y + h > 1. + 1e-12
        {
            return Err(
                "Textbox position must be a finite normalized rectangle inside the figure".into(),
            );
        }
        if self.text.len() > 4096 {
            return Err("Textbox text exceeds 4096 bytes".into());
        }
        if !self.font_size.is_finite() || !(1. ..=200.).contains(&self.font_size) {
            return Err("Textbox font size must be 1..200 points".into());
        }
        Ok(())
    }
    /// Top-left pixel rectangle [left,top,right,bottom] for a figure canvas.
    pub fn rectangle(&self, canvas: [f64; 4]) -> [f64; 4] {
        let [left, top, right, bottom] = canvas;
        let [x, y, w, h] = self.position;
        [
            left + x * (right - left),
            top + (1. - y - h) * (bottom - top),
            left + (x + w) * (right - left),
            top + (1. - y) * (bottom - top),
        ]
    }
    /// Bounded renderer-independent line breaks. Literal text is retained in
    /// the model; disallowed XML controls are replaced only in display output.
    /// Font advance is an explicit approximate sans-serif metric, not MATLAB
    /// font parity. Both native and SVG clip any glyph overflow to the textbox.
    pub fn display_lines(&self, width: f64) -> Vec<String> {
        let em = self.font_size * (4. / 3.);
        let columns = ((width / (em * 0.6)).floor() as usize).clamp(1, 4096);
        let mut lines = Vec::new();
        let mut line = String::new();
        let mut count = 0;
        for c in self.text.chars() {
            if c == '\n' {
                lines.push(std::mem::take(&mut line));
                count = 0;
                continue;
            }
            if count == columns {
                lines.push(std::mem::take(&mut line));
                count = 0;
            }
            let c = if c == '\t' {
                ' '
            } else if c.is_control() || matches!(c, '\u{fffe}' | '\u{ffff}') {
                '\u{fffd}'
            } else {
                c
            };
            line.push(c);
            count += 1;
        }
        lines.push(line);
        lines
    }
}

pub(crate) fn validate_textboxes(boxes: &[TextBox]) -> Result<()> {
    if boxes.len() > MAX_TEXTBOXES {
        return Err("Figure exceeds 64 textboxes".into());
    }
    let mut bytes = 0usize;
    for textbox in boxes {
        textbox.validate()?;
        bytes = bytes
            .checked_add(textbox.text.len())
            .ok_or("Textbox text size overflow")?;
        if bytes > MAX_TEXTBOX_BYTES {
            return Err("Figure textbox text exceeds 16384 bytes".into());
        }
    }
    Ok(())
}
impl Figure {
    pub fn add_textbox(&mut self, textbox: TextBox) -> Result<usize> {
        textbox.validate()?;
        self.validate()?;
        if self.identity.same_object(&textbox.identity)
            || self
                .textboxes
                .iter()
                .any(|b| b.identity.same_object(&textbox.identity))
            || self.all_axes().any(|(_, a)| {
                a.identity.same_object(&textbox.identity)
                    || a.series
                        .iter()
                        .any(|s| s.identity.same_object(&textbox.identity))
            })
        {
            return Err("A graphics object identity appears twice in one figure".into());
        }
        if self.textboxes.len() == MAX_TEXTBOXES
            || self.textboxes.iter().map(|t| t.text.len()).sum::<usize>() + textbox.text.len()
                > MAX_TEXTBOX_BYTES
        {
            return Err("Figure exceeds textbox count or text budget".into());
        }
        let index = self.textboxes.len();
        self.textboxes.push(textbox);
        Ok(index)
    }
}
