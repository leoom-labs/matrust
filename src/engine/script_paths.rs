//! Script identity and lexical path operations, independent of process CWD.
use super::{Engine, Execution, Value};
use crate::{Array, Result};
use std::path::Path;

fn bounded_path(value: &str) -> Result<&Path> {
    if value.len() > 32768
        || value.contains('\0')
        || value.match_indices("://").any(|(index, _)| {
            !(cfg!(windows) && index == 1 && value.as_bytes()[0].is_ascii_alphabetic())
        })
    {
        return Err("Path is oversized, contains NUL, or is a remote URL".into());
    }
    Ok(Path::new(value))
}
fn path_text(path: &Path) -> Result<String> {
    let text = path.to_str().ok_or("Path is not valid Unicode")?;
    bounded_path(text)?;
    Ok(text.to_owned())
}
fn separator(c: char) -> bool {
    c == '/' || (cfg!(windows) && c == '\\')
}
fn folder_part(value: &str) -> String {
    if value.ends_with(separator) {
        let folder = value.trim_end_matches(separator);
        if folder.is_empty() {
            return std::path::MAIN_SEPARATOR.to_string();
        }
        if cfg!(windows) && folder.len() == 2 && folder.as_bytes()[1] == b':' {
            return format!("{folder}{}", std::path::MAIN_SEPARATOR);
        }
        return folder.to_owned();
    }
    match value.rfind(separator) {
        None => String::new(),
        Some(0) => value[..1].to_owned(),
        Some(2) if cfg!(windows) && value.as_bytes()[1] == b':' => value[..3].to_owned(),
        Some(index) => value[..index].to_owned(),
    }
}
fn joined_path(parts: &[String]) -> Result<String> {
    let sep = std::path::MAIN_SEPARATOR;
    let mut raw = String::new();
    for part in parts {
        if part.is_empty() {
            continue;
        }
        if !raw.is_empty() && !raw.ends_with(separator) {
            raw.push(sep);
        }
        if raw.len().saturating_add(part.len()) > 32768 {
            return Err("fullfile path exceeds 32768 bytes".into());
        }
        raw.extend(part.chars().map(|c| if separator(c) { sep } else { c }));
    }
    // Preserve the initial prefix, collapse inner repeated separators and dot
    // components, but retain parent components and a final dot or separator.
    let leading = raw.chars().take_while(|&c| c == sep).count();
    let mut output = raw[..leading].to_owned();
    let rest = &raw[leading..];
    let trailing = rest.ends_with(sep);
    let mut components = rest.split(sep).filter(|p| !p.is_empty()).peekable();
    while let Some(component) = components.next() {
        if component == "." && (components.peek().is_some() || trailing) {
            continue;
        }
        if !output.is_empty() && !output.ends_with(sep) {
            output.push(sep);
        }
        output.push_str(component);
    }
    if trailing && !output.is_empty() && !output.ends_with(sep) {
        output.push(sep);
    }
    bounded_path(&output)?;
    Ok(output)
}
impl Engine {
    /// Execute a named document's current text, retaining ordinary parser and
    /// execution limits. The path identifies mfilename; no process directory is
    /// changed and no source is reread. Identity is restored even after failure.
    pub fn execute_document(&mut self, source: &str, path: &Path) -> Result<Execution> {
        self.check_cancel()?;
        bounded_path(path.to_str().ok_or("Document path is not valid Unicode")?)?;
        let path = std::path::absolute(path).map_err(|e| e.to_string())?;
        path_text(&path)?;
        let previous = self.document_path.replace(path);
        let result = self.execute(source);
        self.document_path = previous;
        result
    }

    /// run(name), run('name.m'), run('folder/name.m') or an absolute path. A
    /// folder part is resolved from the lookup folder and becomes the lookup
    /// folder while the file runs; the caller's is restored afterwards.
    pub(super) fn run_command(&mut self, args: Vec<Value>, out: &mut Execution) -> Result<()> {
        self.check_cancel()?;
        let [arg] = <[Value; 1]>::try_from(args)
            .map_err(|_| "run requires one script name or path".to_owned())?;
        let text = self.text_value(arg)?;
        bounded_path(&text)?;
        if text.is_empty() {
            return Ok(());
        }
        // Reference behavior: a missing file or folder is "<text> not found."; an existing
        // file that is not a .m/.mr script with a valid name cannot run.
        let missing = || format!("{text} not found.");
        let failed = || {
            format!(
                "RUN cannot execute the file '{text}'. RUN requires a valid script: a .m or .mr file whose name starts with a letter, then uses only letters, digits or '_'."
            )
        };
        let path = Path::new(&text);
        let folder = match path.parent().filter(|p| !p.as_os_str().is_empty()) {
            None => None,
            Some(parent) => {
                let parent = if parent.is_absolute() {
                    parent.to_owned()
                } else {
                    self.function_directory().ok_or_else(missing)?.join(parent)
                };
                Some(
                    parent
                        .canonicalize()
                        .ok()
                        .filter(|p| p.is_dir())
                        .ok_or_else(missing)?,
                )
            }
        };
        let lookup = folder
            .as_deref()
            .or(self.function_directory())
            .ok_or_else(missing)?;
        let file = path.file_name().ok_or_else(missing)?;
        let extension = path.extension().map(|e| e.to_str().unwrap_or_default());
        // An explicit extension must name an existing file; .mr still wins by name.
        if extension.is_some() && !lookup.join(file).is_file() {
            return Err(missing());
        }
        if !matches!(extension, None | Some("m" | "mr")) {
            return Err(failed());
        }
        let Some(name) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| crate::files::valid_name(s))
        else {
            return Err(if extension.is_some() {
                failed()
            } else {
                missing()
            });
        };
        self.run_file(folder, name, &text, out)
    }

    /// Inspect borrowed metadata before any UTF-8 projection, including the
    /// generic graphics dispatcher reached by a leading graphics handle.
    pub(super) fn preflight_script_path_arguments(&self, name: &str, args: &[Value]) -> Result<()> {
        if !matches!(name, "mfilename" | "fileparts" | "fullfile") {
            return Ok(());
        }
        self.check_cancel()?;
        match name {
            "mfilename" if args.len() > 1 => {
                return Err("mfilename supports no argument or 'fullpath'".into());
            }
            "fileparts" if args.len() != 1 => {
                return Err("fileparts requires one character path".into());
            }
            "fullfile" if args.is_empty() || args.len() > 64 => {
                return Err("fullfile requires 1 to 64 character paths".into());
            }
            _ => {}
        }
        let mut lower_bound = 0usize;
        for arg in args {
            self.check_cancel()?;
            let length = match arg {
                Value::Text(text) | Value::QuotedString(text) => text.len(),
                Value::Container(Array::Character(value)) => {
                    if !value.is_text_row() {
                        return Err("Text conversion requires a character row or canonical 0x0 empty; matrices are not flattened".into());
                    }
                    value.len()
                }
                _ => return Err("Expected character text or a quoted option".into()),
            };
            if length > 32768 {
                return Err("Path is oversized, contains NUL, or is a remote URL".into());
            }
            // Valid UTF-8 needs at least as many bytes as UTF-16 code units.
            // joined_path already bounds the raw join before normalization, so
            // rejecting this lower bound cannot reject a formerly valid join.
            lower_bound += length; // At most 64 admitted lengths of 32768.
            if name == "fullfile" && lower_bound > 32768 {
                return Err("fullfile path exceeds 32768 bytes".into());
            }
        }
        Ok(())
    }

    pub(super) fn script_path_call(&self, name: &str, args: &[Value]) -> Result<Option<Value>> {
        if !matches!(name, "mfilename" | "fileparts" | "fullfile") {
            return Ok(None);
        }
        self.preflight_script_path_arguments(name, args)?;
        let mut text = Vec::with_capacity(args.len());
        for arg in args {
            self.check_cancel()?;
            let value = self.text_value(arg.clone())?;
            bounded_path(&value)?;
            text.push(value);
        }
        let args = text;
        let result = match name {
            "mfilename" => {
                if args.len() > 1 || args.first().is_some_and(|a| a != "fullpath") {
                    return Err("mfilename supports no argument or 'fullpath'".into());
                }
                let path = self
                    .functions
                    .active_path()
                    .or(self.document_path.as_deref());
                match path {
                    None => String::new(),
                    Some(path) if args.is_empty() => path_text(Path::new(
                        path.file_stem().ok_or("Document has no filename")?,
                    ))?,
                    Some(path) => {
                        let mut path = path.to_owned();
                        path.set_extension("");
                        path_text(&path)?
                    }
                }
            }
            "fileparts" => {
                if args.len() != 1 {
                    return Err("fileparts requires one character path".into());
                }
                folder_part(&args[0])
            }
            _ => {
                if args.is_empty() || args.len() > 64 {
                    return Err("fullfile requires 1 to 64 character paths".into());
                }
                for part in &args[1..] {
                    self.check_cancel()?;
                    let part = Path::new(part);
                    if let Some(std::path::Component::Prefix(prefix)) = part.components().next()
                        && matches!(
                            prefix.kind(),
                            std::path::Prefix::Disk(_)
                                | std::path::Prefix::VerbatimDisk(_)
                                | std::path::Prefix::VerbatimUNC(_, _)
                                | std::path::Prefix::Verbatim(_)
                                | std::path::Prefix::DeviceNS(_)
                        )
                    {
                        return Err("fullfile does not support drive or device prefixes after the first component".into());
                    }
                }
                joined_path(&args)?
            }
        };
        self.check_cancel()?;
        Ok(Some(Value::Text(result)))
    }
}
