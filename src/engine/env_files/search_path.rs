//! User search path: path, addpath, rmpath, genpath, restoredefaultpath.
//! The path holds only user folders; the lookup order itself lives in
//! `Engine::spath_folders` (current folder first, then these folders).
use super::{Engine, Execution, Value, envx_arity, envx_path_text, envx_text};
use crate::{Array, Matrix, Result};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const NAMES: &[&str] = &[
    "path",
    "addpath",
    "rmpath",
    "genpath",
    "restoredefaultpath",
    "prefdir",
    "userpath",
    "savepath",
];
/// Folders on the path and folders one genpath walk may return.
const MAX_FOLDERS: usize = 10_000;
/// Directory entries one genpath walk may inspect.
const MAX_ENTRIES: usize = 1_000_000;
/// Text of one path argument (a genpath result of many folders).
const MAX_TEXT: usize = 8 * 1024 * 1024;

pub(super) fn spath_handles(name: &str) -> bool {
    NAMES.contains(&name)
}

fn spath_separator() -> char {
    if cfg!(windows) { ';' } else { ':' }
}

/// Folders joined by pathsep, without a trailing separator (as `path`).
fn spath_join(folders: &[PathBuf]) -> String {
    folders
        .iter()
        .map(|f| envx_path_text(f))
        .collect::<Vec<_>>()
        .join(&spath_separator().to_string())
}

/// Honor engine-local setenv without changing the host process environment.
fn spath_env(engine: &Engine, name: &str) -> Option<String> {
    engine
        .envx_child_variables()
        .get(name)
        .cloned()
        .or_else(|| std::env::var(name).ok())
        .filter(|s| !s.is_empty())
}

fn spath_home(engine: &Engine) -> Result<PathBuf> {
    spath_env(engine, if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .ok_or_else(|| "No user home folder is available".into())
}

fn spath_absolute(engine: &Engine, path: PathBuf) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path)
    } else {
        engine
            .current_folder()
            .map(|p| p.join(&path))
            .ok_or_else(|| "No current folder is available".into())
    }
}

fn spath_prefdir(engine: &Engine) -> Result<PathBuf> {
    let path = if let Some(folder) = spath_env(engine, "MATRUST_PREFDIR") {
        PathBuf::from(folder)
    } else if let Some(file) = spath_env(engine, "MATRUST_SESSION_PATH") {
        // Reuse the desktop recovery/settings location when explicitly set.
        spath_absolute(engine, PathBuf::from(file))?
            .parent()
            .ok_or("Settings file has no parent folder")?
            .to_path_buf()
    } else if let Some(folder) = spath_env(engine, "LOCALAPPDATA") {
        // The desktop stores session.json (including layout) here.
        PathBuf::from(folder).join("MatRust")
    } else if cfg!(windows) {
        PathBuf::from(spath_env(engine, "APPDATA").ok_or("APPDATA is not set")?).join("MatRust")
    } else if cfg!(target_os = "macos") {
        spath_home(engine)?.join("Library/Application Support/MatRust")
    } else {
        let config = match spath_env(engine, "XDG_CONFIG_HOME") {
            Some(folder) => PathBuf::from(folder),
            None => spath_home(engine)?.join(".config"),
        };
        config.join("matrust")
    };
    let path = spath_absolute(engine, path)?;
    fs::create_dir_all(&path).map_err(|e| format!("prefdir: {}: {e}", path.display()))?;
    Ok(path)
}

/// Read bounded UTF-8 data, never source code. None means no saved preference.
/// Non-regular files (FIFOs, devices, directories) are rejected before and after
/// opening so startup preferences cannot block.
fn spath_read(path: &Path) -> Result<Option<String>> {
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if !meta.is_file() {
        return Err("Saved path is not a regular file".into());
    }
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if !file.metadata().is_ok_and(|m| m.is_file()) {
        return Err("Saved path is not a regular file".into());
    }
    let mut text = String::new();
    file.take(MAX_TEXT as u64 + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() > MAX_TEXT || text.contains('\0') {
        return Err("Saved path exceeds 8 MiB or contains NUL".into());
    }
    Ok(Some(text))
}

fn spath_userpath(engine: &mut Engine, prefdir: &Path) -> Result<Option<PathBuf>> {
    if let Some((directory, folder)) = &engine.functions.spath_userpath
        && directory == prefdir
    {
        return Ok(folder.clone());
    }
    let folder = match spath_read(&prefdir.join("userpath.txt"))? {
        Some(text) if text.is_empty() => None,
        Some(text) if Path::new(&text).is_absolute() && !text.contains(['\r', '\n']) => {
            Some(PathBuf::from(text))
        }
        Some(_) => return Err("Invalid saved userpath".into()),
        None => Some(spath_home(engine)?.join("Documents").join("MatRust")),
    };
    engine.functions.spath_userpath = Some((prefdir.to_path_buf(), folder.clone()));
    Ok(folder)
}

/// Replace a preference atomically; failed writes leave the old file intact.
fn spath_write(path: &Path, text: &str) -> Result<()> {
    if text.len() > MAX_TEXT {
        return Err("Saved path exceeds 8 MiB".into());
    }
    let mut file = tempfile::NamedTempFile::new_in(path.parent().ok_or("Missing parent folder")?)
        .map_err(|e| e.to_string())?;
    file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    file.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn spath_pathdef(folders: &[PathBuf]) -> Result<String> {
    let mut text = String::from("function p = pathdef\np = [ ...\n");
    if folders.is_empty() {
        text.push_str("'' ...\n");
    }
    for (i, folder) in folders.iter().enumerate() {
        let name = envx_path_text(folder);
        if name.contains(['\r', '\n', '\0']) {
            return Err("savepath: folder names with line breaks or NUL are unsupported".into());
        }
        text.push('\'');
        text.push_str(&name.replace('\'', "''"));
        text.push_str(if i + 1 == folders.len() {
            "' ...\n"
        } else {
            "' pathsep ...\n"
        });
        if text.len() > MAX_TEXT {
            return Err("Saved path exceeds 8 MiB".into());
        }
    }
    text.push_str("];\nend\n");
    Ok(text)
}

/// Exactly the grammar emitted above. No evaluation, expressions or comments.
#[cfg(not(target_arch = "wasm32"))]
fn spath_parse_pathdef(text: &str) -> Result<Vec<PathBuf>> {
    let invalid = "Unparseable pathdef.m (expected the format written by savepath)";
    let mut lines = text.lines();
    if lines.next() != Some("function p = pathdef") || lines.next() != Some("p = [ ...") {
        return Err(invalid.into());
    }
    let mut folders = Vec::new();
    let mut final_folder = false;
    while let Some(line) = lines.next() {
        if line == "];" {
            if !final_folder || lines.next() != Some("end") || lines.next().is_some() {
                return Err(invalid.into());
            }
            return Ok(folders);
        }
        if final_folder || folders.len() >= MAX_FOLDERS {
            return Err(invalid.into());
        }
        let quoted = if let Some(quoted) = line.strip_suffix(" pathsep ...") {
            quoted
        } else {
            final_folder = true;
            line.strip_suffix(" ...").ok_or(invalid)?
        };
        let name = quoted
            .strip_prefix('\'')
            .and_then(|q| q.strip_suffix('\''))
            .ok_or(invalid)?;
        if name.replace("''", "").contains('\'') {
            return Err(invalid.into());
        }
        let name = name.replace("''", "'");
        if name.is_empty() && folders.is_empty() && final_folder {
            continue;
        }
        if !Path::new(&name).is_absolute() || name.contains(['\r', '\n', '\0']) {
            return Err(invalid.into());
        }
        folders.push(PathBuf::from(name));
    }
    Err(invalid.into())
}

impl Engine {
    /// CLI/desktop startup only. Default engines never read user preferences.
    /// Warnings are returned to the host so startup continues even with bad data.
    pub fn spath_load_saved_path(&mut self) -> Vec<String> {
        if self.functions.spath_loaded {
            return Vec::new();
        }
        self.functions.spath_loaded = true;
        #[cfg(target_arch = "wasm32")]
        return Vec::new();
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut warnings = Vec::new();
            let prefdir = match spath_prefdir(self) {
                Ok(folder) => folder,
                Err(e) => return vec![format!("Warning: {e}")],
            };
            let mut folders = Vec::new();
            match spath_userpath(self, &prefdir) {
                Ok(Some(folder)) => {
                    if let Ok(folder) = folder.canonicalize()
                        && folder.is_dir()
                    {
                        folders.push(folder);
                    }
                }
                Ok(None) => {}
                Err(e) => warnings.push(format!("Warning: userpath: {e}")),
            }
            let pathdef = prefdir.join("pathdef.m");
            let saved = spath_read(&pathdef)
                .and_then(|text| text.map(|text| spath_parse_pathdef(&text)).transpose());
            match saved {
                Ok(Some(saved)) => {
                    for folder in saved {
                        match folder.canonicalize().ok().filter(|p| p.is_dir()) {
                            Some(folder) if !folders.contains(&folder) => {
                                if folders.len() < MAX_FOLDERS {
                                    folders.push(folder);
                                } else {
                                    warnings.push(format!(
                                        "Warning: pathdef.m: at most {MAX_FOLDERS} folders"
                                    ));
                                    break;
                                }
                            }
                            Some(_) => {}
                            None => warnings.push(format!(
                                "Warning: Name is nonexistent or not a directory: {}",
                                folder.display()
                            )),
                        }
                    }
                }
                Ok(None) => {}
                Err(e) => warnings.push(format!("Warning: {}: {e}", pathdef.display())),
            }
            self.spath_set(folders);
            warnings
        }
    }

    fn spath_string(&self, who: &str, value: &Value) -> Result<String> {
        let text = match value {
            Value::Text(s) | Value::QuotedString(s) => s.clone(),
            Value::Container(Array::Character(c)) if c.is_text_row() || c.is_empty() => {
                c.to_utf8(&self.cancel)?
            }
            _ => {
                return Err(format!(
                    "{who}: folder names must be character vectors or strings"
                ));
            }
        };
        if text.len() > MAX_TEXT || text.contains('\0') {
            return Err(format!("{who}: path text exceeds 8 MiB or contains NUL"));
        }
        Ok(text)
    }

    /// Absolute, canonical folder for a path entry; None when it is not an
    /// existing folder. Relative names resolve against the current folder.
    fn spath_folder(&self, entry: &str) -> Option<PathBuf> {
        if entry.contains("://") {
            return None;
        }
        let path = Path::new(entry);
        let path = if path.is_relative() {
            self.current_folder()?.join(path)
        } else {
            path.to_path_buf()
        };
        path.canonicalize().ok().filter(|p| p.is_dir())
    }

    /// Existing folders named by pathsep-separated entries, in order; a
    /// missing one warns and is skipped.
    fn spath_entries(
        &mut self,
        who: &str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Vec<PathBuf>> {
        let mut folders = Vec::new();
        for arg in args {
            let text = self.spath_string(who, arg)?;
            for entry in text
                .split(spath_separator())
                .filter(|e| !e.trim().is_empty())
            {
                self.check_cancel()?;
                match self.spath_folder(entry) {
                    Some(folder) => {
                        if !folders.contains(&folder) {
                            if folders.len() >= MAX_FOLDERS {
                                return Err(format!(
                                    "{who}: the search path holds at most {MAX_FOLDERS} folders"
                                ));
                            }
                            folders.push(folder);
                        }
                    }
                    None => self.envx_warning(
                        "MATLAB:mpath:nameNonexistentOrNotADirectory",
                        format!("Name is nonexistent or not a directory: {entry}"),
                        out,
                    )?,
                }
            }
        }
        Ok(folders)
    }

    fn spath_store(&mut self, who: &str, folders: Vec<PathBuf>) -> Result<()> {
        if folders.len() > MAX_FOLDERS {
            return Err(format!(
                "{who}: the search path holds at most {MAX_FOLDERS} folders"
            ));
        }
        self.spath_set(folders);
        Ok(())
    }

    pub(super) fn spath_run(
        &mut self,
        name: &str,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        if count > 1 {
            return Err("Too many output arguments.".into());
        }
        #[cfg(target_arch = "wasm32")]
        if matches!(name, "prefdir" | "userpath") {
            return Ok(self.browser_query(name, args)?.into_iter().collect());
        }
        let old = if matches!(name, "path" | "addpath" | "rmpath") {
            spath_join(self.spath_list())
        } else {
            String::new()
        };
        let value = match name {
            "prefdir" => {
                envx_arity(args, 0, 0)?;
                envx_text(envx_path_text(&spath_prefdir(self)?))
            }
            "userpath" => {
                envx_arity(args, 0, 1)?;
                let prefdir = spath_prefdir(self)?;
                let old_userpath = spath_userpath(self, &prefdir)?;
                if let Some(arg) = args.first() {
                    let text = self.spath_string("userpath", arg)?;
                    let new = match text.as_str() {
                        "clear" => None,
                        "reset" => Some(spath_home(self)?.join("Documents").join("MatRust")),
                        _ if !text.is_empty() => Some(
                            self.spath_folder(&text)
                                .ok_or("userpath: folder must exist")?,
                        ),
                        _ => return Err("userpath: folder must exist".into()),
                    };
                    let saved = new.as_ref().map(|p| envx_path_text(p)).unwrap_or_default();
                    if saved.contains(['\r', '\n', spath_separator()]) {
                        return Err(
                            "userpath: folder names with line breaks or pathsep are unsupported"
                                .into(),
                        );
                    }
                    let old_folder = old_userpath.map(|p| p.canonicalize().unwrap_or(p));
                    let new_folder = new
                        .clone()
                        .and_then(|p| p.canonicalize().ok())
                        .filter(|p| p.is_dir());
                    let mut folders: Vec<_> = self
                        .spath_list()
                        .iter()
                        .filter(|p| {
                            Some(*p) != old_folder.as_ref() && Some(*p) != new_folder.as_ref()
                        })
                        .cloned()
                        .collect();
                    if let Some(folder) = new_folder {
                        folders.insert(0, folder);
                    }
                    if folders.len() > MAX_FOLDERS {
                        return Err(format!("userpath: at most {MAX_FOLDERS} folders"));
                    }
                    spath_write(&prefdir.join("userpath.txt"), &saved)?;
                    self.functions.spath_userpath = Some((prefdir, new));
                    self.spath_store("userpath", folders)?;
                    envx_text(saved)
                } else {
                    envx_text(
                        old_userpath
                            .as_ref()
                            .map(|p| envx_path_text(p))
                            .unwrap_or_default(),
                    )
                }
            }
            "savepath" => {
                envx_arity(args, 0, 1)?;
                #[cfg(target_arch = "wasm32")]
                {
                    self.envx_warning(
                        "MatRust:savepath:Browser",
                        super::super::browser::NO_FILES.into(),
                        out,
                    )?;
                    return Ok(vec![Value::Matrix(Matrix::scalar(1.))]);
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let file = args
                        .first()
                        .map(|v| self.spath_string("savepath", v))
                        .transpose()?;
                    let result = (|| {
                        let file = match file {
                            Some(file) if !file.is_empty() => {
                                spath_absolute(self, PathBuf::from(file))?
                            }
                            Some(_) => return Err("savepath: filename is empty".into()),
                            None => spath_prefdir(self)?.join("pathdef.m"),
                        };
                        spath_write(&file, &spath_pathdef(self.spath_list())?)
                    })();
                    if let Err(message) = &result
                        && count == 0
                    {
                        self.envx_warning(
                            "MatRust:savepath:FileError",
                            format!("savepath: {message}"),
                            out,
                        )?;
                    }
                    Value::Matrix(Matrix::scalar(if result.is_ok() { 0. } else { 1. }))
                }
            }
            "path" => {
                envx_arity(args, 0, 2)?;
                if args.is_empty() {
                    if count == 0 {
                        let lines: Vec<String> = self
                            .spath_list()
                            .iter()
                            .map(|f| envx_path_text(f))
                            .collect();
                        for line in lines {
                            self.emit(out, line)?;
                        }
                        return Ok(Vec::new());
                    }
                    return Ok(vec![envx_text(old)]);
                }
                #[cfg(target_arch = "wasm32")]
                return Err(super::super::browser::NO_FILES.into());
                let folders = self.spath_entries("path", args, out)?;
                self.spath_store("path", folders)?;
                envx_text(old)
            }
            "addpath" => {
                if args.is_empty() {
                    return Err("Not enough input arguments.".into());
                }
                let mut end = false;
                let mut names = Vec::new();
                for arg in args {
                    let option = match arg {
                        Value::Matrix(m) if m.len() == 1 && matches!(m.data()[0], 0. | 1.) => {
                            Some(m.data()[0] == 1.)
                        }
                        _ => match self
                            .spath_string("addpath", arg)?
                            .to_ascii_lowercase()
                            .as_str()
                        {
                            "-begin" => Some(false),
                            "-end" => Some(true),
                            "-frozen" => None,
                            _ => {
                                names.push(arg.clone());
                                continue;
                            }
                        },
                    };
                    if let Some(option) = option {
                        end = option;
                    }
                }
                if names.is_empty() {
                    return Err("Not enough input arguments.".into());
                }
                let added = self.spath_entries("addpath", &names, out)?;
                let mut folders: Vec<PathBuf> = self
                    .spath_list()
                    .iter()
                    .filter(|f| !added.contains(f))
                    .cloned()
                    .collect();
                if end {
                    folders.extend(added);
                } else {
                    folders.splice(0..0, added);
                }
                self.spath_store("addpath", folders)?;
                envx_text(old)
            }
            "rmpath" => {
                if args.is_empty() {
                    return Err("Not enough input arguments.".into());
                }
                let mut folders = self.spath_list().to_vec();
                for arg in args {
                    let text = self.spath_string("rmpath", arg)?;
                    for entry in text
                        .split(spath_separator())
                        .filter(|e| !e.trim().is_empty())
                    {
                        self.check_cancel()?;
                        // A folder deleted since addpath still matches by name.
                        let target = self.spath_folder(entry).or_else(|| {
                            let path = Path::new(entry);
                            if path.is_absolute() {
                                Some(path.to_path_buf())
                            } else {
                                self.current_folder().map(|c| c.join(path))
                            }
                        });
                        let before = folders.len();
                        folders.retain(|f| Some(f) != target.as_ref());
                        if folders.len() == before {
                            self.envx_warning(
                                "MATLAB:rmpath:DirNotFound",
                                format!("\"{entry}\" not found in path."),
                                out,
                            )?;
                        }
                    }
                }
                self.spath_store("rmpath", folders)?;
                envx_text(old)
            }
            "genpath" => {
                envx_arity(args, 1, 1)?;
                let text = self.spath_string("genpath", &args[0])?;
                #[cfg(target_arch = "wasm32")]
                {
                    let _ = text;
                    return Ok(vec![envx_text("")]);
                }
                envx_text(self.spath_genpath(&text)?)
            }
            _ => {
                envx_arity(args, 0, 0)?;
                self.spath_store("restoredefaultpath", Vec::new())?;
                Value::Void
            }
        };
        Ok(if count == 0 && !matches!(name, "genpath" | "prefdir") {
            Vec::new()
        } else {
            vec![value]
        })
    }

    /// The folder and its subfolders, depth first in sorted name order, each
    /// followed by pathsep; class (@), package (+), hidden (.), private and
    /// resources folders are left out. A missing folder gives ''.
    fn spath_genpath(&self, text: &str) -> Result<String> {
        if text.is_empty() || text.contains("://") {
            return Ok(String::new());
        }
        let root = Path::new(text);
        let root = if root.is_relative() {
            match self.current_folder() {
                Some(current) => current.join(root),
                None => return Ok(String::new()),
            }
        } else {
            root.to_path_buf()
        };
        if !root.is_dir() {
            return Ok(String::new());
        }
        let mut result = String::new();
        let mut seen = BTreeSet::new();
        let mut stack = vec![root];
        let (mut folders, mut entries) = (0usize, 0usize);
        while let Some(folder) = stack.pop() {
            self.check_cancel()?;
            // Symbolic links to folders are followed once: no loops.
            if let Ok(real) = folder.canonicalize()
                && !seen.insert(real)
            {
                continue;
            }
            folders += 1;
            if folders > MAX_FOLDERS {
                return Err(format!(
                    "genpath: more than {MAX_FOLDERS} folders under {text}"
                ));
            }
            result.push_str(&envx_path_text(&folder));
            result.push(spath_separator());
            let Ok(list) = fs::read_dir(&folder) else {
                continue;
            };
            let mut children = Vec::new();
            for entry in list.flatten() {
                entries += 1;
                if entries > MAX_ENTRIES {
                    return Err(format!(
                        "genpath: more than {MAX_ENTRIES} entries under {text}"
                    ));
                }
                if entries % 4096 == 0 {
                    self.check_cancel()?;
                }
                let name = entry.file_name();
                let Some(text) = name.to_str() else {
                    continue;
                };
                if text.starts_with(['@', '+', '.']) || matches!(text, "private" | "resources") {
                    continue;
                }
                if fs::metadata(entry.path()).is_ok_and(|m| m.is_dir()) {
                    children.push(entry.path());
                }
            }
            children.sort();
            stack.extend(children.into_iter().rev());
        }
        Ok(result)
    }
}
