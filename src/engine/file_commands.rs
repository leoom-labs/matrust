//! Script file commands reuse the bounded codecs and commit loads atomically.
use super::{Engine, Execution};
use crate::{Result, files, syntax::Expr};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Version {
    V6,
    V7,
    V73,
}

impl Engine {
    /// Set the base for relative script file operations, independently of the
    /// host process CWD and .m function search path. Existing context survives failure.
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
        if filename.is_empty() || filename.len() > 32768 || filename.contains('\0') {
            return Err("Filename is empty, oversized or contains NUL".into());
        }
        if filename.contains("://") {
            return Err("Remote file URLs are not supported".into());
        }
        let mut path = PathBuf::from(filename);
        #[cfg(windows)]
        if !path.has_root()
            && matches!(
                path.components().next(),
                Some(std::path::Component::Prefix(_))
            )
        {
            return Err("Drive-relative filenames are unsupported; use a drive-absolute or workspace-relative path".into());
        }
        if path.extension().is_none_or(|s| s.is_empty()) {
            let mut filename = path.into_os_string();
            filename.push(default_extension);
            path = PathBuf::from(filename);
        }
        if path.is_relative() {
            path = self
                .file_directory
                .as_ref()
                .ok_or("No file directory is available; set_file_directory first")?
                .join(path);
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
        let values = self.function_arguments(expressions, None, output)?;
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
        let path = self.file_path(filename, ".mat")?;
        let mut selected = BTreeSet::new();
        let mut version = None;
        let mut uncompressed = false;
        let mut force_mat = false;
        for text in options {
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
                ("save", "-nocompression") => uncompressed = true,
                ("load", "-mat") => force_mat = true,
                _ if text.starts_with('-') => {
                    return Err(format!("Unsupported {name} option: {text}"));
                }
                _ => {
                    if !files::valid_name(text) {
                        return Err("Select explicit variable names; wildcard/regexp selection is not supported yet".into());
                    }
                    selected.insert(text.clone());
                }
            }
        }
        if name == "save" {
            if uncompressed && version != Some(Version::V7) {
                return Err("-nocompression currently requires explicit -v7; uncompressed v7.3 is not implemented".into());
            }
            let variables = if selected.is_empty() {
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
            if let Some((name, _)) = variables.iter().find(|(_, a)| a.contains_struct()) {
                return Err(format!(
                    "save: variable '{name}' holds a struct; saving structs is not supported yet"
                ));
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
            if !force_mat
                && !path
                    .extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.eq_ignore_ascii_case("mat"))
            {
                return Err("ASCII load is not supported yet; use -mat to read MAT data with another extension".into());
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
            for info in catalog.variables().filter(|v| selected.contains(&v.name)) {
                self.check_cancel()?;
                Self::validate_workspace_name(&info.name)?;
                if let Some(reason) = &info.unavailable {
                    return Err(format!("{}: {reason}", info.name));
                }
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
            if selected.is_empty() {
                return self.check_cancel();
            }
            let variables = files::load_selected_arrays(&catalog, &selected, &self.cancel)?;
            self.merge_workspace(variables)
        }
    }
}
