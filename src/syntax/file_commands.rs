//! Recognize file command arguments without rewriting source byte positions.
use super::{Result, Spanned, Token, line_break_len};

pub(super) fn arguments(source: &str, after_name: usize) -> bool {
    let bytes = source.as_bytes();
    if !bytes
        .get(after_name)
        .is_some_and(|c| matches!(c, b' ' | b'\t'))
    {
        return false;
    }
    let mut at = after_name;
    while bytes.get(at).is_some_and(|c| matches!(c, b' ' | b'\t')) {
        at += 1;
    }
    let Some(&first) = bytes.get(at) else {
        return false;
    };
    if b"(=\r\n;,%<>:&|^[]~".contains(&first) {
        return false;
    }
    if first == b'.'
        && (bytes.get(at + 1).is_some_and(|c| b"*^'".contains(c))
            || bytes.get(at + 1) == Some(&b'/')
                && bytes
                    .get(at + 2)
                    .is_none_or(|c| c.is_ascii_whitespace() || c.is_ascii_digit()))
    {
        return false;
    }
    if first == b'\'' {
        // A balanced quoted argument may itself start with a delimiter.
        // Otherwise leave the quote to the ordinary transpose lexer.
        let mut cursor = at + 1;
        while cursor < bytes.len() && line_break_len(bytes, cursor) == 0 {
            if bytes[cursor] == b'\'' {
                if bytes.get(cursor + 1) != Some(&b'\'') {
                    return true;
                }
                cursor += 1;
            }
            cursor += 1;
        }
        return false;
    }
    // Retain ordinary arithmetic/indexing for variables named save or load.
    // Quote filenames beginning with an ambiguous arithmetic operator.
    if b"+-*/\\".contains(&first) {
        return first == b'-' && bytes.get(at + 1).is_some_and(u8::is_ascii_alphabetic)
            || matches!(first, b'/' | b'\\')
                && bytes.get(at + 1).is_some_and(|c| !c.is_ascii_whitespace());
    }
    true
}

pub(super) fn lex(
    source: &str,
    at: &mut usize,
    tokens: &mut Vec<Spanned>,
    error: &mut usize,
) -> Result<()> {
    let bytes = source.as_bytes();
    tokens.push(Spanned {
        token: Token::LParen,
        start: *at,
        end: *at,
    });
    let mut count = 0;
    loop {
        while bytes.get(*at).is_some_and(|c| matches!(c, b' ' | b'\t')) {
            *at += 1;
        }
        if bytes.get(*at..).is_some_and(|s| s.starts_with(b"...")) {
            while *at < bytes.len() && line_break_len(bytes, *at) == 0 {
                *at += 1;
            }
            *at += line_break_len(bytes, *at);
            continue;
        }
        if *at == bytes.len() || b"\r\n;,%".contains(&bytes[*at]) {
            break;
        }
        let start = *at;
        *error = start;
        let mut text = String::new();
        if matches!(bytes[*at], b'\'' | b'"') {
            let quote = bytes[*at];
            *at += 1;
            loop {
                let Some(&next) = bytes.get(*at) else {
                    return Err("Unterminated file command argument".into());
                };
                if matches!(next, b'\r' | b'\n') {
                    return Err("A quoted file command argument cannot cross a newline".into());
                }
                if next == quote {
                    *at += 1;
                    if bytes.get(*at) == Some(&quote) {
                        text.push(quote as char);
                        *at += 1;
                    } else {
                        break;
                    }
                } else {
                    let ch = source[*at..].chars().next().ok_or("Invalid command text")?;
                    text.push(ch);
                    *at += ch.len_utf8();
                }
            }
            if bytes
                .get(*at)
                .is_some_and(|c| !c.is_ascii_whitespace() && !b";,%".contains(c))
            {
                return Err("Separate file command arguments with spaces".into());
            }
        } else {
            while *at < bytes.len()
                && !bytes[*at].is_ascii_whitespace()
                && !b";,%".contains(&bytes[*at])
            {
                if matches!(bytes[*at], b'\'' | b'"' | b'(' | b')' | b'[' | b']') {
                    return Err("Quote the complete file command argument".into());
                }
                let ch = source[*at..].chars().next().ok_or("Invalid command text")?;
                text.push(ch);
                *at += ch.len_utf8();
            }
        }
        count += 1;
        if count > 255 {
            return Err("File command exceeds 255 arguments".into());
        }
        if count > 1 {
            tokens.push(Spanned {
                token: Token::Comma,
                start,
                end: start,
            });
        }
        tokens.push(Spanned {
            token: Token::String(text),
            start,
            end: *at,
        });
        if tokens.len() > 100_000 {
            return Err("Script exceeds 100000 tokens".into());
        }
    }
    let end = tokens.last().map_or(*at, |t| t.end);
    tokens.push(Spanned {
        token: Token::RParen,
        start: end,
        end,
    });
    Ok(())
}

/// A wildcard filename in an environment/file command. Keep spaced arithmetic
/// (`delete * 2`) available to variables named after a command.
pub(super) fn envx_wildcard_argument(source: &str, after_name: usize) -> bool {
    let rest = &source[after_name..];
    if !rest.starts_with([' ', '\t']) {
        return false;
    }
    let rest = rest.trim_start_matches([' ', '\t']);
    if rest.starts_with('?') {
        return true;
    }
    let Some(rest) = rest.strip_prefix('*') else {
        return false;
    };
    rest.is_empty()
        || rest.starts_with(['.', '*', '?', ';', ',', '\n', '\r'])
        || rest.chars().next().is_some_and(char::is_alphabetic)
}
