//! Element text of datetime and duration arrays for display, char, string
//! and cellstr. The console layout lives in display_format.rs.
use super::{dur_text, polled_map, text};
use crate::{Array, Result};
use std::sync::atomic::AtomicBool;

/// Text of every element, column-major, in the array's own format (or
/// `format` when given). `cancel` is polled every 65536 elements; the
/// console display passes None (it bounds the element count by the output
/// limit before it asks for the text).
pub(crate) fn element_texts(
    array: &Array,
    format: Option<&str>,
    cancel: Option<&AtomicBool>,
) -> Result<Vec<String>> {
    let never = AtomicBool::new(false);
    let cancel = cancel.unwrap_or(&never);
    match array {
        Array::Datetime(t) => {
            let tokens = text::tokens(format.unwrap_or(t.format()))?;
            polled_map(cancel, t.data(), |v| text::render(&tokens, v))
        }
        Array::Duration(d) => {
            if let Some(values) = d.calendar_data() {
                return polled_map(cancel, values, super::calendar_duration::render);
            }
            let f = dur_text::parse_format(format.unwrap_or(d.format()))?;
            polled_map(cancel, d.data(), |ms| dur_text::render(f, ms))
        }
        _ => Err("Expected a datetime or duration array".into()),
    }
}
