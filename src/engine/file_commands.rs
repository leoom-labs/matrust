//! Script file commands reuse the bounded codecs and commit loads atomically.
use super::{Engine, Execution, user_functions::nested};
use crate::{Result, files, syntax::Expr};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

/// Search the user path for a relative file missing from the current folder.
/// Keep the local path on a miss so each reader reports its normal error.
/// `files::read_fallback` alone decides which names and found files qualify;
/// load, fopen, exist, which and the readers all resolve through here.
pub(super) fn resolve_read_path(engine: &Engine, name: &Path, local: PathBuf) -> PathBuf {
    // Callers pass `file_directory.join(name)`; rebuild it component by
    // component so a Windows verbatim current folder still resolves `/`, `.`
    // and `..` before the metadata check below.
    let local = match engine.file_directory.as_deref() {
        Some(base) if local == base.join(name) => files::read_fallback::local_file(base, name),
        _ => local,
    };
    let folders = engine.spath_list();
    // A name that may not search is the local path, hit or miss, untouched.
    if folders.is_empty() || !files::read_fallback::name_is_eligible(name) {
        return local;
    }
    if !local.metadata().is_err_and(|err| {
        matches!(
            err.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
        )
    }) {
        return local;
    }
    folders
        .iter()
        .find_map(|folder| files::read_fallback::contained_file(folder, name))
        .unwrap_or(local)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Version {
    V6,
    V7,
    V73,
}

impl Engine {
    /// Set the base for relative script file operations, independently of the
    /// host process CWD and function directory. Existing context survives failure.
    pub fn set_file_directory(&mut self, directory: &Path) -> Result<()> {
        let directory = directory
            .canonicalize()
            .map_err(|e| format!("File directory: {e}"))?;
        if !directory.is_dir() {
            return Err("File directory must be a directory".into());
        }
        self.file_directory = Some(directory);
        Ok(())
    }

    pub(super) fn file_path(&self, filename: &str, default_extension: &str) -> Result<PathBuf> {
        self.files_path(filename, default_extension, false)
    }

    pub(super) fn files_read_path(
        &self,
        filename: &str,
        default_extension: &str,
    ) -> Result<PathBuf> {
        self.files_path(filename, default_extension, true)
    }

    fn files_path(&self, filename: &str, default_extension: &str, read: bool) -> Result<PathBuf> {
        if filename.is_empty() || filename.len() > 32768 || filename.contains('\0') {
            return Err("Filename is empty, oversized or contains NUL".into());
        }
        if filename.contains("://") {
            return Err("Remote file URLs are not supported".into());
        }
        let mut path = PathBuf::from(filename);
        files::read_fallback::reject_drive_relative(&path)?;
        if !default_extension.is_empty()
            && path.extension().is_none_or(|s| s.is_empty())
            && (!read || files::read_fallback::can_append_suffix(filename))
        {
            let mut filename = path.into_os_string();
            filename.push(default_extension);
            path = PathBuf::from(filename);
        }
        if path.is_relative() {
            let local = self
                .file_directory
                .as_ref()
                .ok_or("No file directory is available; set_file_directory first")?
                .join(&path);
            return Ok(if read {
                resolve_read_path(self, &path, local)
            } else {
                local
            });
        }
        Ok(path)
    }

    pub(super) fn file_command(
        &mut self,
        name: &str,
        expressions: &[Expr],
        output: &mut Execution,
    ) -> Result<()> {
        self.check_cancel()?;
        let values = super::string_class::arguments(self, name, expressions, output)?;
        let mut args = Vec::with_capacity(values.len());
        for value in values {
            let text = self.text_value(value).map_err(|_| {
                format!("{name} arguments must be text filenames, variable names or options")
            })?;
            if text.is_empty() || text.len() > 32768 || text.contains('\0') {
                return Err("File command argument is empty, oversized or contains NUL".into());
            }
            args.push(text);
        }
        let (filename, options) = if args.first().is_some_and(|s| !s.starts_with('-')) {
            (args[0].as_str(), &args[1..])
        } else {
            ("matlab.mat", args.as_slice())
        };
        let mut selected = BTreeSet::new();
        let mut selection_order = Vec::new();
        let mut version = None;
        let mut uncompressed = false;
        let mut force_mat = false;
        let mut ascii = false;
        let mut double = false;
        let mut tabs = false;
        let mut struct_source = None;
        let mut options = options.iter();
        while let Some(text) = options.next() {
            self.check_cancel()?;
            match (name, text.as_str()) {
                ("save", "-v6" | "-v7" | "-v7.3") => {
                    let next = match text.as_str() {
                        "-v6" => Version::V6,
                        "-v7.3" => Version::V73,
                        _ => Version::V7,
                    };
                    if version.is_some_and(|old| old != next) {
                        return Err("Choose one MAT file version".into());
                    }
                    version = Some(next);
                }
                ("save", "-struct") => {
                    if struct_source.is_some() || !selected.is_empty() {
                        return Err("save: choose one scalar struct source".into());
                    }
                    struct_source = Some(
                        options
                            .next()
                            .ok_or("save: -struct requires a scalar struct variable")?
                            .clone(),
                    );
                }
                ("save", "-nocompression") => uncompressed = true,
                ("load", "-mat") => force_mat = true,
                ("load" | "save", "-ascii") => ascii = true,
                ("save", "-double") => double = true,
                ("save", "-tabs") => tabs = true,
                _ if text.starts_with('-') => {
                    return Err(format!("Unsupported {name} option: {text}"));
                }
                _ => {
                    if !files::valid_name(text) {
                        return Err("Select explicit variable names; wildcard/regexp selection is not supported yet".into());
                    }
                    if selected.insert(text.clone()) {
                        selection_order.push(text.clone());
                    }
                }
            }
        }
        if force_mat && ascii {
            return Err("Choose either -mat or -ascii".into());
        }
        let default_extension = if name == "save" && ascii { "" } else { ".mat" };
        if name == "save" {
            let path = self.file_path(filename, default_extension)?;
            if (double || tabs) && !ascii {
                return Err("-double and -tabs require -ascii".into());
            }
            if ascii && (version.is_some() || uncompressed || struct_source.is_some()) {
                return Err("ASCII save cannot be combined with MAT options".into());
            }
            if uncompressed && version != Some(Version::V7) {
                return Err("-nocompression currently requires explicit -v7; uncompressed v7.3 is not implemented".into());
            }
            let variables = if let Some(source) = struct_source {
                let value = self
                    .array(&source)
                    .ok_or_else(|| format!("Unknown workspace variable: {source}"))?;
                let crate::Array::Struct(st) = &value else {
                    return Err("save: -struct requires a scalar struct variable".into());
                };
                if st.len() != 1
                    || super::classdef::object_class(&value).is_some()
                    || super::lang_more::lng_class(&value) != "struct"
                    || super::objects::save_object_class(self, &value).is_some()
                {
                    return Err("save: -struct requires a scalar struct variable".into());
                }
                if selected.is_empty() {
                    selected.extend(st.field_names().iter().cloned());
                }
                selected
                    .iter()
                    .map(|field| {
                        let index = st
                            .field_index(field)
                            .ok_or_else(|| format!("save: unknown struct field: {field}"))?;
                        Ok((field.clone(), st.get(index, 0)?.as_ref().clone()))
                    })
                    .collect::<Result<BTreeMap<_, _>>>()?
            } else if selected.is_empty() {
                self.workspace()
            } else {
                selected
                    .iter()
                    .map(|name| {
                        self.array(name)
                            .map(|a| (name.clone(), a))
                            .ok_or_else(|| format!("Unknown workspace variable: {name}"))
                    })
                    .collect::<Result<BTreeMap<_, _>>>()?
            };
            if ascii {
                return save_ascii(
                    self,
                    &path,
                    &variables,
                    &selection_order,
                    double,
                    tabs,
                    output,
                );
            }
            for (name, value) in &variables {
                check_save_objects(self, value, name)?;
            }
            self.check_cancel()?;
            match version.unwrap_or(Version::V7) {
                Version::V73 => files::save_array_workspace_v73(&path, &variables, &self.cancel),
                Version::V6 => {
                    files::save_array_workspace_uncompressed(&path, &variables, &self.cancel)
                }
                Version::V7 if uncompressed => {
                    files::save_array_workspace_uncompressed(&path, &variables, &self.cancel)
                }
                Version::V7 => files::save_array_workspace(&path, &variables, &self.cancel),
            }
        } else {
            let path = self.files_read_path(filename, default_extension)?;
            if ascii || (!force_mat && !is_mat(&path)) {
                if !selected.is_empty() {
                    return Err("load: variable selection requires a MAT file".into());
                }
                let matrix = load_ascii(self, &path)?;
                let stem = path
                    .file_stem()
                    .and_then(|x| x.to_str())
                    .ok_or("load: invalid filename")?;
                let mut var: String = stem
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || c == '_' {
                            c
                        } else {
                            '_'
                        }
                    })
                    .collect();
                if !var.starts_with(|c: char| c.is_ascii_alphabetic()) {
                    var.insert(0, 'X');
                }
                Self::validate_workspace_name(&var)?;
                nested::check_write(self, &var)?;
                return self
                    .merge_workspace(BTreeMap::from([(var, crate::Array::Numeric(matrix))]));
            }
            let catalog = files::inspect_workspace(&path, &self.cancel)?;
            if selected.is_empty() {
                selected.extend(catalog.variables().map(|v| v.name.clone()));
            }
            let mut remaining = selected.clone();
            let mut bytes = self
                .workspace()
                .values()
                .map(crate::Array::bytes)
                .sum::<usize>() as u64;
            let mut count = self.workspace_count();
            // A complex sparse variable is reported by name after the others load.
            let mut skipped = None;
            for info in catalog.variables().filter(|v| selected.contains(&v.name)) {
                self.check_cancel()?;
                Self::validate_workspace_name(&info.name)?;
                if info.class.ends_with("sparse") && info.complex {
                    skipped.get_or_insert_with(|| info.name.clone());
                    remaining.remove(&info.name);
                    continue;
                }
                if let Some(reason) = &info.unavailable {
                    return Err(format!("{}: {reason}", info.name));
                }
                nested::check_write(self, &info.name)?;
                if let Some(old) = self.array(&info.name) {
                    bytes -= old.bytes() as u64;
                } else {
                    count += 1;
                }
                bytes = bytes
                    .checked_add(info.storage_bytes().ok_or("MAT size overflow")?)
                    .ok_or("Workspace size overflow")?;
                remaining.remove(&info.name);
            }
            if let Some(missing) = remaining.first() {
                return Err(format!("Unknown MAT variable: {missing}"));
            }
            if bytes > 256 * 1024 * 1024 || count > 10000 {
                return Err("Resulting workspace exceeds 256 MiB or 10000 variables".into());
            }
            let skipped_error = skipped.map(|name| {
                format!("load: variable '{name}' is a complex sparse array; complex sparse arrays are not supported yet")
            });
            if skipped_error.is_some() {
                selected.retain(|n| {
                    !catalog
                        .variables()
                        .any(|v| &v.name == n && v.class.ends_with("sparse") && v.complex)
                });
            }
            if selected.is_empty() {
                self.check_cancel()?;
                return skipped_error.map_or(Ok(()), Err);
            }
            let variables = files::load_selected_arrays(&catalog, &selected, &self.cancel)?;
            self.merge_workspace(variables)?;
            skipped_error.map_or(Ok(()), Err)
        }
    }
}

// Classdef objects use private struct storage but are not plain MAT structs.
fn check_save_objects(engine: &Engine, value: &crate::Array, path: &str) -> Result<()> {
    engine.check_cancel()?;
    if let Some(class) = super::classdef::object_class(value) {
        let suffix = if path.contains(['.', '{']) {
            format!(" (field {path})")
        } else {
            String::new()
        };
        return Err(format!(
            "save: objects of class '{class}' cannot be saved yet{suffix}"
        ));
    }
    match value {
        crate::Array::Struct(st) => {
            if super::lang_more::lng_class(value) != "struct"
                || super::objects::save_object_class(engine, value).is_some()
            {
                return Err(format!(
                    "save: variable '{path}' holds a struct; saving structs is not supported yet"
                ));
            }
            for e in 0..st.len() {
                for (f, name) in st.field_names().iter().enumerate() {
                    check_save_objects(engine, st.get(f, e)?, &format!("{path}.{name}"))?;
                }
            }
        }
        crate::Array::Cell(cell) => {
            for i in 0..cell.len() {
                check_save_objects(engine, cell.get_linear(i)?, &format!("{path}{{{}}}", i + 1))?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// The expression form returns a scalar struct without merging the workspace.
pub(super) fn load_output(engine: &Engine, args: &[super::Value]) -> Result<super::Value> {
    let text = args
        .iter()
        .map(|v| engine.text_value(v.clone()))
        .collect::<Result<Vec<_>>>()?;
    let (filename, options) = if text.first().is_some_and(|s| !s.starts_with('-')) {
        (text[0].as_str(), &text[1..])
    } else {
        ("matlab.mat", text.as_slice())
    };
    let path = engine.files_read_path(filename, ".mat")?;
    let mut names = BTreeSet::new();
    let mut force_mat = false;
    let mut ascii = false;
    for name in options {
        if name == "-mat" {
            force_mat = true;
        } else if name == "-ascii" {
            ascii = true;
        } else if name.starts_with('-') {
            return Err(format!("Unsupported load option: {name}"));
        } else if !files::valid_name(name) {
            return Err(
                "Select explicit variable names; wildcard/regexp selection is not supported yet"
                    .into(),
            );
        } else {
            names.insert(name.clone());
        }
    }
    if force_mat && ascii {
        return Err("Choose either -mat or -ascii".into());
    }
    if ascii || (!force_mat && !is_mat(&path)) {
        if !names.is_empty() {
            return Err("load: variable selection requires a MAT file".into());
        }
        return Ok(super::Value::Matrix(load_ascii(engine, &path)?));
    }
    let catalog = files::inspect_workspace(&path, &engine.cancel)?;
    if names.is_empty() {
        names.extend(catalog.variables().map(|v| v.name.clone()));
    }
    if names.len() > crate::struct_array::MAX_FIELDS {
        return Err("Struct exceeds 1024 fields".into());
    }
    let variables = if names.is_empty() {
        BTreeMap::new()
    } else {
        files::load_selected_arrays(&catalog, &names, &engine.cancel)?
    };
    let (names, slots) = variables
        .into_iter()
        .map(|(name, value)| (name, std::sync::Arc::new(value)))
        .unzip();
    Ok(super::Value::Container(
        crate::StructArray::from_slots(1, 1, names, slots, &engine.cancel)?.into(),
    ))
}

pub(super) fn whos_file(
    engine: &mut Engine,
    args: &[super::Value],
    out: &mut Execution,
) -> Result<()> {
    use std::fmt::Write;
    let text = args
        .iter()
        .map(|v| engine.text_value(v.clone()))
        .collect::<Result<Vec<_>>>()?;
    if text.len() < 2 || text[0] != "-file" {
        return Err("whos with arguments requires '-file' and a MAT filename".into());
    }
    let catalog = files::inspect_workspace(&engine.file_path(&text[1], ".mat")?, &engine.cancel)?;
    let mut display = String::new();
    for info in catalog
        .variables()
        .filter(|v| text.len() == 2 || text[2..].contains(&v.name))
    {
        engine.check_cancel()?;
        let size = info
            .dimensions
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join("x");
        let bytes = info
            .storage_bytes()
            .map(|n| n.to_string())
            .unwrap_or_else(|| "?".into());
        let class = if info.complex {
            format!("{} (complex)", info.class)
        } else {
            info.class.clone()
        };
        writeln!(
            display,
            "{:<18} {size:>18} {bytes:>10} bytes  {class}",
            info.name
        )
        .map_err(|e| e.to_string())?;
        if display.len() > 2 * 1024 * 1024 {
            return Err("Workspace display exceeds 2 MiB".into());
        }
    }
    engine.emit(out, display)
}

fn is_mat(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("mat"))
}

fn load_ascii(engine: &Engine, path: &Path) -> Result<crate::Matrix> {
    let bytes = files::read_bounded(path, 256 * 1024 * 1024)?;
    let text = std::str::from_utf8(files::strip_utf8_bom(&bytes))
        .map_err(|_| "load: ASCII file is not UTF-8")?;
    let mut rows = Vec::new();
    let mut width = None;
    for (line_no, line) in text.lines().enumerate() {
        if line_no % 1024 == 0 {
            engine.check_cancel()?;
        }
        let line = line.split('%').next().unwrap_or("");
        let fields: Vec<_> = line
            .split(|c: char| c.is_ascii_whitespace() || c == ',')
            .filter(|x| !x.is_empty())
            .collect();
        if fields.is_empty() {
            continue;
        }
        if width.is_some_and(|w| w != fields.len()) {
            return Err(format!(
                "load: inconsistent column count on line {}",
                line_no + 1
            ));
        }
        width = Some(fields.len());
        crate::Matrix::checked_len(rows.len() + 1, fields.len())?;
        let row = fields
            .iter()
            .map(|f| {
                f.parse::<f64>().map_err(|_| {
                    format!("load: invalid numeric value on line {}: {f}", line_no + 1)
                })
            })
            .collect::<Result<Vec<_>>>()?;
        rows.push(row);
    }
    let nr = rows.len();
    let nc = width.unwrap_or(0);
    let mut data = Vec::with_capacity(nr * nc);
    for c in 0..nc {
        for row in &rows {
            data.push(row[c]);
        }
    }
    crate::Matrix::from_column_vec(nr, nc, data)
}

fn save_ascii(
    engine: &mut Engine,
    path: &Path,
    vars: &BTreeMap<String, crate::Array>,
    order: &[String],
    double: bool,
    tabs: bool,
    out: &mut Execution,
) -> Result<()> {
    use std::fmt::Write;
    let mut text = String::new();
    let names: Vec<&String> = if order.is_empty() {
        vars.keys().collect()
    } else {
        order.iter().collect()
    };
    for name in names {
        let value = &vars[name];
        engine.check_cancel()?;
        let (rows, cols, nd) = match value {
            crate::Array::Numeric(m) => (m.rows(), m.cols(), m.is_nd()),
            crate::Array::Complex(m) => (m.rows(), m.cols(), m.is_nd()),
            _ => {
                engine.emit(
                    out,
                    format!("Warning: save: variable '{name}' is not numeric and was skipped.\n"),
                )?;
                continue;
            }
        };
        if nd {
            engine.emit(
                out,
                format!("Warning: save: variable '{name}' is not 2-D and was skipped.\n"),
            )?;
            continue;
        }
        for r in 0..rows {
            if r % 1024 == 0 {
                engine.check_cancel()?;
            }
            for c in 0..cols {
                if c > 0 && tabs {
                    text.push('\t');
                }
                let x = match value {
                    crate::Array::Numeric(m) => m.get(r, c),
                    crate::Array::Complex(m) => m.get(r, c)?.re,
                    _ => unreachable!(),
                };
                let formatted = if x.is_nan() {
                    "NaN".into()
                } else if x == f64::INFINITY {
                    "Inf".into()
                } else if x == f64::NEG_INFINITY {
                    "-Inf".into()
                } else {
                    let precision = if double { 16 } else { 7 };
                    let scientific = format!("{x:.precision$e}");
                    let (mantissa, exponent) = scientific.split_once('e').unwrap();
                    let exp: i32 = exponent.parse().unwrap();
                    format!("{mantissa}e{exp:+03}")
                };
                write!(
                    text,
                    "{formatted:>width$}",
                    width = if double { 26 } else { 16 }
                )
                .map_err(|e| e.to_string())?;
                if text.len() > 256 * 1024 * 1024 {
                    return Err("save: ASCII output exceeds 256 MiB".into());
                }
            }
            text.push('\n');
        }
    }
    files::atomic_write_with_cancel(path, text.as_bytes(), &engine.cancel)
}
