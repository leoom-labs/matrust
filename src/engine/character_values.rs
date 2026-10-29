//! Character operands share one UTF-16-to-double conversion. Numeric/logical
//! views borrow their immutable storage; only character views own a conversion.
use super::{Engine, Value};
use crate::{Array, CharacterRow, Matrix, Result, logical::ArrayRef};
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) enum NumericView<'a> {
    Borrowed(ArrayRef<'a>),
    Converted(Matrix),
}
impl NumericView<'_> {
    pub(super) fn as_ref(&self) -> ArrayRef<'_> {
        match self {
            Self::Borrowed(view) => *view,
            Self::Converted(matrix) => ArrayRef::Numeric(matrix),
        }
    }
}
impl Value {
    /// Double-quoted literals are not char arrays here; they keep their own rules.
    pub(super) fn is_character(&self) -> bool {
        matches!(self, Self::Text(_) | Self::Container(Array::Character(_)))
    }
    /// A 1x1 char ':' subscript means all elements, as the bare colon does.
    pub(super) fn is_colon_character(&self) -> bool {
        match self {
            Self::Text(s) => s == ":",
            Self::Container(Array::Character(a)) => a.units() == [u16::from(b':')],
            _ => false,
        }
    }
    pub(super) fn character_codes(&self, cancel: &AtomicBool) -> Result<Matrix> {
        let text;
        let character = match self {
            Self::Container(Array::Character(character)) => character,
            Self::Text(s) => {
                text = CharacterRow::from_text(s, cancel)?;
                &text
            }
            _ => return Err("Expected a character value".into()),
        };
        if cancel.load(Ordering::Relaxed) {
            return Err("Calculation cancelled".into());
        }
        let mut codes = Vec::new();
        codes
            .try_reserve_exact(character.len())
            .map_err(|_| "Cannot allocate character codes")?;
        for chunk in character.units().chunks(1024) {
            if cancel.load(Ordering::Relaxed) {
                return Err("Calculation cancelled".into());
            }
            codes.extend(chunk.iter().map(|&unit| f64::from(unit)));
        }
        if cancel.load(Ordering::Relaxed) {
            return Err("Calculation cancelled".into());
        }
        Matrix::from_column_vec(character.rows(), character.cols(), codes)
    }
    pub(super) fn numeric_operand(self, cancel: &AtomicBool) -> Result<Self> {
        if self.is_character() {
            Ok(Self::Matrix(self.matrix(cancel)?))
        } else {
            Ok(self)
        }
    }
}
pub(super) const CHAR_TO_LOGICAL: &str = "Conversion to logical from char is not possible.";

/// Builtins whose result has the class of their first argument.
pub(super) fn keeps_character_class(name: &str) -> bool {
    matches!(
        name,
        "flip"
            | "flipud"
            | "fliplr"
            | "diag"
            | "tril"
            | "triu"
            | "transpose"
            | "ctranspose"
            | "rot90"
            | "circshift"
    )
}
impl Engine {
    /// Legacy option consumers receive UTF-8 rows when representable. Keep the
    /// original arguments separately for numeric consumers and shape-aware disp.
    pub(super) fn call_text_arguments<'a>(
        &self,
        args: &'a [Value],
    ) -> Result<std::borrow::Cow<'a, [Value]>> {
        if !args.iter().any(|v| {
            matches!(
                v,
                Value::QuotedString(_) | Value::Container(Array::Character(_))
            )
        }) {
            return Ok(std::borrow::Cow::Borrowed(args));
        }
        args.iter()
            .map(|value| {
                self.check_cancel()?;
                Ok(match value {
                    Value::QuotedString(s) => Value::Text(s.clone()),
                    Value::Container(Array::Character(a)) if a.is_text_row() => {
                        match a.to_utf8(&self.cancel) {
                            Ok(text) => Value::Text(text),
                            Err(_) => {
                                self.check_cancel()?;
                                value.clone()
                            }
                        }
                    }
                    value => value.clone(),
                })
            })
            .collect::<Result<Vec<_>>>()
            .map(std::borrow::Cow::Owned)
    }

    pub(super) fn character_argument(&self, value: &Value) -> Result<CharacterRow> {
        match value {
            Value::Text(s) | Value::QuotedString(s) => CharacterRow::from_text(s, &self.cancel),
            Value::Container(Array::Character(a)) => Ok(a.clone()),
            Value::Matrix(a) => self.characters_from_codes(a),
            Value::Logical(_) => self.characters_from_codes(&value.clone().matrix(&self.cancel)?),
            _ => Err("char supports a bounded real 2-D array of exact UTF-16 units or an existing character array".into()),
        }
    }

    /// The inverse of `character_codes`: exact integer UTF-16 units become char.
    pub(super) fn characters_from_codes(&self, a: &Matrix) -> Result<CharacterRow> {
        CharacterRow::checked_len(a.rows(), a.cols())?;
        let mut units = Vec::new();
        units
            .try_reserve_exact(a.len())
            .map_err(|_| "Cannot allocate character row")?;
        for chunk in a.data().chunks(1024) {
            self.check_cancel()?;
            for &v in chunk {
                if !v.is_finite() || v.fract() != 0. || !(0. ..=65535.).contains(&v) {
                    return Err("char supports exact integer UTF-16 units 0..65535; rounding/out-of-range conversion is unsupported".into());
                }
                units.push(v as u16);
            }
        }
        CharacterRow::from_column_major(a.rows(), a.cols(), units, &self.cancel)
    }

    /// Rearranging builtins keep char (MATLAB): they run on the codes of the
    /// first argument and the result converts back. Everything else that
    /// computes numbers from char returns double through `character_codes`.
    pub(super) fn character_class_call(
        &mut self,
        name: &str,
        mut args: Vec<Value>,
        out: &mut super::Execution,
    ) -> Result<Value> {
        args[0] = Value::Matrix(args[0].character_codes(&self.cancel)?);
        let result = self.call(name, args, out)?.matrix(&self.cancel)?;
        Ok(Value::Container(
            self.characters_from_codes(&result)?.into(),
        ))
    }

    /// Stack blocks in column-major order, optionally padding the right margin.
    /// All shapes are bounded before allocating the result. Inputs stay shared.
    pub(super) fn character_blocks(
        &self,
        rows: Vec<Vec<CharacterRow>>,
        padded: bool,
    ) -> Result<Value> {
        const DIMENSIONS: &str = "Dimensions of matrices being concatenated are not consistent.";
        let mut height = 0usize;
        let mut width = None;
        for row in &rows {
            self.check_cancel()?;
            let Some(first) = row.first() else { continue };
            let h = first.rows();
            if row.iter().any(|part| part.rows() != h) {
                return Err(DIMENSIONS.into());
            }
            let w = row
                .iter()
                .try_fold(0usize, |n, part| n.checked_add(part.cols()))
                .ok_or("Character concatenation overflow")?;
            if !padded && width.is_some_and(|old| old != w) {
                return Err(DIMENSIONS.into());
            }
            width = Some(width.unwrap_or(0).max(w));
            height = height
                .checked_add(h)
                .ok_or("Character concatenation overflow")?;
        }
        let width = width.unwrap_or(0);
        let len = CharacterRow::checked_len(height, width)?;
        self.preflight_elements(len.div_ceil(4))?;
        let mut units = Vec::new();
        units
            .try_reserve_exact(len)
            .map_err(|_| "Cannot allocate character concatenation")?;
        for start in (0..len).step_by(1024) {
            self.check_cancel()?;
            units.resize((start + 1024).min(len), u16::from(b' '));
        }
        let mut r = 0;
        for row in rows {
            let h = row.first().map_or(0, CharacterRow::rows);
            let mut c = 0;
            for part in row {
                for col in 0..part.cols() {
                    self.check_cancel()?;
                    for start in (0..h).step_by(1024) {
                        self.check_cancel()?;
                        let n = (h - start).min(1024);
                        let target = (c + col) * height + r + start;
                        let source = col * h + start;
                        units[target..target + n]
                            .copy_from_slice(&part.units()[source..source + n]);
                    }
                }
                c += part.cols();
            }
            r += h;
        }
        Ok(Value::Container(
            CharacterRow::from_column_major(height, width, units, &self.cancel)?.into(),
        ))
    }

    pub(super) fn padded_characters(&self, args: &[Value], omit_empty: bool) -> Result<Value> {
        let mut rows = Vec::new();
        let mut height = 0usize;
        let mut width = 0usize;
        let mut push = |character: CharacterRow| -> Result<()> {
            height = height
                .checked_add(character.rows())
                .ok_or("Character concatenation overflow")?;
            width = width.max(character.cols());
            CharacterRow::checked_len(height, width)?;
            rows.push(vec![character]);
            Ok(())
        };
        for value in args {
            self.check_cancel()?;
            if let Value::Container(Array::Cell(cell)) = value {
                // MATLAB visits cells in linear (column-major) order.
                for index in 0..cell.len() {
                    let content = cell.get_linear(index)?;
                    self.check_cancel()?;
                    let Array::Character(character) = content.as_ref() else {
                        return Err(
                            "Character conversion requires a cell array of character rows".into(),
                        );
                    };
                    if !character.is_text_row() {
                        return Err(
                            "Character conversion requires a cell array of character rows".into(),
                        );
                    }
                    let character = if character.is_empty() && !omit_empty {
                        character.reshape(1, 0, &self.cancel)?
                    } else {
                        character.clone()
                    };
                    if !omit_empty || !character.is_empty() {
                        push(character)?;
                    }
                }
            } else {
                let character = self.character_argument(value)?;
                if !omit_empty || !character.is_empty() {
                    push(character)?;
                }
            }
        }
        self.character_blocks(rows, true)
    }
}

impl Engine {
    pub(super) fn character_range(&self, start: f64, step: f64, count: usize) -> Result<Value> {
        CharacterRow::checked_len(1, count)?;
        let mut units = Vec::new();
        units
            .try_reserve_exact(count)
            .map_err(|_| "Cannot allocate character range")?;
        for first in (0..count).step_by(1024) {
            self.check_cancel()?;
            units.extend(
                (first..(first + 1024).min(count)).map(|i| (start + i as f64 * step) as u16),
            );
        }
        Ok(Value::Container(
            CharacterRow::from_column_major(1, count, units, &self.cancel)?.into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancelled_character_numeric_conversion_never_returns_a_partial_result() {
        let cancel = AtomicBool::new(false);
        let character = CharacterRow::from_units(vec![55296, 0, 65535], &cancel).unwrap();
        let values = [
            Value::Text("abc".into()),
            Value::Container(character.into()),
        ];
        cancel.store(true, Ordering::Relaxed);
        for value in values {
            assert!(
                value
                    .clone()
                    .matrix(&cancel)
                    .unwrap_err()
                    .contains("cancel")
            );
            assert!(matches!(value.view(&cancel), Err(error) if error.contains("cancel")));
        }
    }
}
