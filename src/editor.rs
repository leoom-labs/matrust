//! Bounded, UTF-8-safe editor operations. Independent of the desktop renderer.
use crate::Result;
use std::ops::Range;

pub const MAX_BYTES: usize = 1_000_000;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Plain,
    Comment,
    String,
    Number,
    Keyword,
    Function,
    Operator,
}

/// Tolerant highlighting also works while an expression is incomplete. This is
/// presentation only; the execution parser remains the authority on syntax.
pub fn highlights(source: &str) -> Vec<(Range<usize>, Style)> {
    let b = source.as_bytes();
    let mut spans: Vec<(Range<usize>, Style)> = Vec::new();
    let mut i = 0;
    let mut can_end = false;
    while i < b.len() {
        let start = i;
        let style;
        // Bound layout sections for adversarial alternating token styles.
        if spans.len() >= 20_000 || source.len() > MAX_BYTES {
            spans.push((i..b.len(), Style::Plain));
            break;
        }
        if b[i] == b'%' || b[i..].starts_with(b"...") {
            i += b[i..]
                .iter()
                .position(|c| matches!(c, b'\r' | b'\n'))
                .unwrap_or(b.len() - i);
            style = Style::Comment;
        } else if b[i] == b'"' || (b[i] == b'\'' && !can_end) {
            let quote = b[i];
            i += 1;
            while i < b.len() {
                if b[i] == quote {
                    i += 1;
                    if b.get(i) == Some(&quote) {
                        i += 1;
                    } else {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            style = Style::String;
            can_end = true;
        } else if b[i].is_ascii_digit()
            || (b[i] == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit))
        {
            i += 1;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            if b.get(i) == Some(&b'.') && !b.get(i + 1).is_some_and(|c| b"*/^'".contains(c)) {
                i += 1;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
            }
            if b.get(i).is_some_and(|c| b"eE".contains(c)) {
                i += 1;
                if b.get(i).is_some_and(|c| b"+-".contains(c)) {
                    i += 1;
                }
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
            }
            style = Style::Number;
            can_end = true;
        } else if b[i].is_ascii_alphabetic() || b[i] == b'_' {
            i += 1;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            let name = &source[start..i];
            style = if matches!(
                name,
                "if" | "else"
                    | "elseif"
                    | "end"
                    | "for"
                    | "while"
                    | "function"
                    | "return"
                    | "break"
                    | "continue"
                    | "switch"
                    | "case"
                    | "otherwise"
                    | "try"
                    | "catch"
                    | "clear"
                    | "clc"
                    | "hold"
                    | "grid"
            ) {
                Style::Keyword
            } else if source[i..].trim_start().starts_with('(') {
                Style::Function
            } else {
                Style::Plain
            };
            can_end = true;
        } else if b[i..].starts_with(b".'") {
            i += 2;
            style = Style::Operator;
            can_end = true;
        } else {
            let c = source[i..].chars().next().unwrap();
            i += c.len_utf8();
            style = if c.is_whitespace() {
                Style::Plain
            } else {
                Style::Operator
            };
            if !c.is_whitespace() {
                can_end = matches!(c, ')' | ']' | '\'');
            }
            if matches!(c, '\r' | '\n') {
                can_end = false;
            }
        }
        if let Some((range, old)) = spans.last_mut()
            && *old == style
        {
            range.end = i;
        } else {
            spans.push((start..i, style));
        }
    }
    spans
}

/// Display bare CR as a logical break without changing source bytes/cursors.
/// Replacing one ASCII byte with one ASCII byte preserves every byte and
/// character index; existing CRLF is retained, including its invisible CR.
pub fn layout_text(source: &str) -> std::borrow::Cow<'_, str> {
    let bytes = source.as_bytes();
    if !bytes
        .iter()
        .enumerate()
        .any(|(i, b)| *b == b'\r' && bytes.get(i + 1) != Some(&b'\n'))
    {
        return std::borrow::Cow::Borrowed(source);
    }
    std::borrow::Cow::Owned(
        source
            .char_indices()
            .map(|(i, c)| {
                if c == '\r' && bytes.get(i + 1) != Some(&b'\n') {
                    '\n'
                } else {
                    c
                }
            })
            .collect(),
    )
}
/// Check the retained layout without allocating another editor snapshot.
pub fn layout_matches(display: &str, source: &str) -> bool {
    let bytes = source.as_bytes();
    display.len() == bytes.len()
        && display.bytes().enumerate().all(|(i, b)| {
            b == if bytes[i] == b'\r' && bytes.get(i + 1) != Some(&b'\n') {
                b'\n'
            } else {
                bytes[i]
            }
        })
}

pub fn char_byte(text: &str, character: usize) -> usize {
    text.char_indices()
        .nth(character)
        .map_or(text.len(), |(byte, _)| byte)
}
pub fn byte_chars(text: &str, range: Range<usize>) -> Range<usize> {
    let start = text[..range.start].chars().count();
    start..start + text[range].chars().count()
}
/// Literal, case-sensitive, non-overlapping search with wraparound. `from` is
/// a byte boundary; previous searches end before it, next searches start at it.
pub fn find(text: &str, needle: &str, from: usize, backwards: bool) -> Option<Range<usize>> {
    if needle.is_empty() || !text.is_char_boundary(from) {
        return None;
    }
    let start = if backwards {
        text[..from].rfind(needle).or_else(|| text.rfind(needle))
    } else {
        text[from..]
            .find(needle)
            .map(|p| from + p)
            .or_else(|| text.find(needle))
    }?;
    Some(start..start + needle.len())
}
/// Validate the complete output size before allocating or modifying the buffer.
pub fn replace_all(text: &str, needle: &str, replacement: &str) -> Result<(String, usize)> {
    if needle.is_empty() {
        return Err("Enter text to find".into());
    }
    let count = text.matches(needle).count();
    let size = text
        .len()
        .saturating_sub(count * needle.len())
        .checked_add(
            count
                .checked_mul(replacement.len())
                .ok_or("Replacement exceeds 1 MB")?,
        )
        .filter(|n| *n <= MAX_BYTES)
        .ok_or("Replacement exceeds 1 MB")?;
    let mut result = String::with_capacity(size);
    let mut previous = 0;
    for (start, _) in text.match_indices(needle) {
        result.push_str(&text[previous..start]);
        result.push_str(replacement);
        previous = start + needle.len();
    }
    result.push_str(&text[previous..]);
    Ok((result, count))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tolerant_highlighting_preserves_every_utf8_byte_and_distinguishes_transpose() {
        let text = "% αβ\nx=1e-3; y=A'; title('It''s a filter');\nplot(x, ... ignored\n";
        let spans = highlights(text);
        assert_eq!(
            spans
                .iter()
                .map(|(r, _)| &text[r.clone()])
                .collect::<String>(),
            text
        );
        assert!(
            spans
                .iter()
                .any(|(r, s)| *s == Style::String && &text[r.clone()] == "'It''s a filter'")
        );
        assert!(
            spans
                .iter()
                .any(|(r, s)| *s == Style::Number && &text[r.clone()] == "1e-3")
        );
        assert!(
            spans
                .iter()
                .any(|(r, s)| *s == Style::Operator && text[r.clone()].starts_with('\''))
        );
        assert_eq!(highlights(&"x=1;".repeat(250_000)).len(), 20_001);
        assert!(
            !highlights("A.'; b=2;")
                .iter()
                .any(|(_, style)| *style == Style::String)
        );
    }
    #[test]
    fn search_wraps_and_replacement_is_bounded_and_literal() {
        let text = "α.x α.x";
        assert_eq!(find(text, "α.x", 0, false), Some(0..4));
        assert_eq!(find(text, "α.x", 4, false), Some(5..9));
        assert_eq!(find(text, "α.x", 9, false), Some(0..4));
        assert_eq!(find(text, "α.x", 0, true), Some(5..9));
        assert_eq!(find(text, "α.x", 5, true), Some(0..4));
        assert_eq!(find("abc", "abc", 1, false), Some(0..3));
        assert_eq!(byte_chars(text, 5..9), 4..7);
        assert_eq!(replace_all(text, "α.x", "β").unwrap(), ("β β".into(), 2));
        assert!(replace_all(&"x".repeat(MAX_BYTES), "x", "xx").is_err());
        assert!(replace_all(text, "", "x").is_err());
    }
}
