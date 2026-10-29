//! Function help: embedded pages, user-file help comments and honest messages
//! for names MatRust does not support. The engine and the editor share this API.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    path::PathBuf,
    sync::OnceLock,
};

const PAGES: &str = include_str!("../assets/help/functions.md");
const MATLAB_NAMES: &str = include_str!("../assets/help/matlab-names.txt");
/// Help reads at most this many bytes of a user file; help text sits at the top.
const MAX_USER_SOURCE: u64 = 1_000_000;

/// Where help looks for user functions and scripts (the Current Folder).
#[derive(Clone, Debug, Default)]
pub struct PathContext {
    pub directory: Option<PathBuf>,
}

/// Split help markdown into `name -> page` without copying. A page runs from
/// its `## name` heading to the next `## ` heading; text before the first
/// heading is ignored, surrounding blank lines are dropped and the first
/// heading of a repeated name wins.
pub fn parse_pages(text: &str) -> BTreeMap<&str, &str> {
    let mut pages = BTreeMap::new();
    let mut current: Option<(&str, usize)> = None;
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        if let Some(heading) = line.strip_prefix("## ") {
            if let Some((name, start)) = current.take() {
                pages
                    .entry(name)
                    .or_insert_with(|| page_body(&text[start..at]));
            }
            current = heading
                .split_whitespace()
                .next()
                .map(|name| (name, at + line.len()));
        }
        at += line.len();
    }
    if let Some((name, start)) = current {
        pages
            .entry(name)
            .or_insert_with(|| page_body(&text[start..]));
    }
    pages
}

fn page_body(mut body: &str) -> &str {
    while let Some((line, rest)) = body.split_once('\n')
        && line.trim().is_empty()
    {
        body = rest;
    }
    body.trim_end()
}

fn pages() -> &'static BTreeMap<&'static str, &'static str> {
    static PARSED: OnceLock<BTreeMap<&'static str, &'static str>> = OnceLock::new();
    PARSED.get_or_init(|| parse_pages(PAGES))
}

fn matlab_names() -> &'static BTreeSet<&'static str> {
    static NAMES: OnceLock<BTreeSet<&'static str>> = OnceLock::new();
    NAMES.get_or_init(|| {
        MATLAB_NAMES
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .collect()
    })
}

/// The help page of a built-in function, as written (line endings normalized).
pub fn page(name: &str) -> Option<String> {
    pages().get(name).map(|page| page.replace('\r', ""))
}

/// The first line of a built-in function's help page.
pub fn summary(name: &str) -> Option<&'static str> {
    pages().get(name).and_then(|page| page.lines().next())
}

/// MatRust implements `name` (case-sensitive).
pub fn is_supported(name: &str) -> bool {
    crate::session::FUNCTIONS.contains(&name)
}

/// Whether `name` is in the MATLAB function-name list (case-sensitive).
pub fn is_matlab_name(name: &str) -> bool {
    matlab_names().contains(name)
}

/// Error text for a call that no user function or built-in handles.
pub fn unknown_function(name: &str) -> String {
    if is_supported(name) {
        format!("{name}: this call form is not supported yet")
    } else if is_matlab_name(name) {
        format!("'{name}' is a MATLAB function that MatRust does not support yet.")
    } else {
        format!("Undefined function or variable '{name}'.")
    }
}

/// The help comment block of MATLAB source: consecutive `%` lines at the top
/// of the file or after the `function` line (blank lines before the block
/// skipped). As in MATLAB, each line prints as one space plus the text after `%`.
pub fn h1_text(source: &str) -> Option<String> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let mut lines = source
        .lines()
        .skip_while(|l| l.trim().is_empty())
        .peekable();
    let top = comment_block(&mut lines);
    if !top.is_empty() {
        return Some(top);
    }
    let mut line = lines.next()?.trim_start();
    if !line
        .strip_prefix("function")
        .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '\t', '[']))
    {
        return None;
    }
    // A signature continued with ... ends on its first line without one.
    while continues(line) {
        line = lines.next()?;
    }
    while lines.next_if(|l| l.trim().is_empty()).is_some() {}
    let after = comment_block(&mut lines);
    (!after.is_empty()).then_some(after)
}

/// Whether the line ends in a `...` continuation: one before any `%` comment
/// and outside quotes.
fn continues(line: &str) -> bool {
    let mut quoted = None;
    let mut dots = 0;
    for c in line.chars() {
        match (quoted, c) {
            (Some(q), c) if c == q => quoted = None,
            (Some(_), _) => {}
            (None, '"') => quoted = Some('"'),
            (None, '\'') => quoted = Some('\''),
            (None, '%') => return false,
            (None, '.') => {
                dots += 1;
                if dots == 3 {
                    return true;
                }
                continue;
            }
            _ => {}
        }
        dots = 0;
    }
    false
}

fn comment_block<'a>(lines: &mut std::iter::Peekable<impl Iterator<Item = &'a str>>) -> String {
    let mut block = String::new();
    while let Some(text) = lines
        .peek()
        .and_then(|line| line.trim_start().strip_prefix('%'))
        // A %{ block comment is not help text.
        .filter(|text| text.trim_end() != "{")
    {
        if !block.is_empty() {
            block.push('\n');
        }
        block.push(' ');
        block.push_str(text.trim_end());
        lines.next();
    }
    block
}

fn is_identifier(name: &str) -> bool {
    name.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

/// The text `help name` prints. User files (.mr, then .m) in the path context
/// come first, as for calls; then built-in pages and MATLAB name knowledge.
/// Unsupported name forms and unreadable user files become the returned text.
pub fn help_text(name: &str, path: &PathContext) -> String {
    help(name, path).unwrap_or_else(|error| error)
}

/// Like [`help_text`], but unsupported name forms and unreadable user files
/// are errors.
pub fn help(name: &str, path: &PathContext) -> crate::Result<String> {
    if !is_identifier(name) {
        // Dotted built-in class names such as containers.Map have pages too.
        if let Some(page) = page(name).filter(|_| is_supported(name)) {
            return Ok(page);
        }
        return Err(format!(
            "help: '{name}' is not a function name; help for folders, packages, methods and operators is not supported yet"
        ));
    }
    if let Some(text) = user_help(name, path)? {
        return Ok(text);
    }
    if let Some(page) = page(name) {
        return Ok(page);
    }
    if is_supported(name) {
        return Ok(format!("{name} is supported. No help page yet."));
    }
    if is_matlab_name(name) {
        return Ok(format!(
            "{name} is a MATLAB function that MatRust does not support yet."
        ));
    }
    // The final line points to MatRust help; no help browser is available.
    let mut text = format!("\n{name} not found.\n\nType help to list the supported functions.");
    if let Some(similar) = crate::session::FUNCTIONS
        .iter()
        .find(|f| f.eq_ignore_ascii_case(name))
    {
        text.push_str(&format!("\nDid you mean {similar}?"));
    }
    Ok(text)
}

fn user_help(name: &str, path: &PathContext) -> crate::Result<Option<String>> {
    let Some(directory) = &path.directory else {
        return Ok(None);
    };
    if !crate::files::valid_name(name) {
        return Ok(None);
    }
    for extension in ["mr", "m"] {
        let file = directory.join(format!("{name}.{extension}"));
        let mut source = Vec::new();
        let read = File::open(&file).and_then(|f| {
            if !f.metadata()?.is_file() {
                return Ok(false);
            }
            f.take(MAX_USER_SOURCE).read_to_end(&mut source)?;
            Ok(true)
        });
        match read {
            Ok(true) => {}
            Ok(false) => continue,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("{}: {e}", file.display())),
        }
        return Ok(Some(
            h1_text(&String::from_utf8_lossy(&source))
                .unwrap_or_else(|| format!("\nNo help found for {name}.{extension}.")),
        ));
    }
    Ok(None)
}
