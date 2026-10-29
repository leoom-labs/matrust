//! Browser build (wasm32): file queries see an empty file system; other file
//! commands fail before a handler reaches std's unsupported I/O.
use super::{Engine, Value};
use crate::{Array, LogicalMatrix, Matrix, Result};

pub(super) const NO_FILES: &str = "Files are not available in the browser version.";

const FILE_COMMANDS: &[&str] = &[
    "save",
    "load",
    "fopen",
    "fgetl",
    "fgets",
    "feof",
    "ftell",
    "fseek",
    "frewind",
    "fwrite",
    "fread",
    "fscanf",
    "dlmread",
    "dlmwrite",
    "csvread",
    "csvwrite",
    "readmatrix",
    "writematrix",
    "fileread",
    "importdata",
    "textread",
    "zip",
    "unzip",
    "gzip",
    "gunzip",
    "tar",
    "untar",
    "imfinfo",
    "audioinfo",
    "isdir",
    "which",
    "type",
    "cd",
    "dir",
    "ls",
    "pwd",
    "what",
    "mkdir",
    "rmdir",
    "copyfile",
    "movefile",
    "fileattrib",
    "tempdir",
    "tempname",
    "diary",
    "run",
    "savefig",
    "openfig",
    "saveas",
    "exportgraphics",
    "print",
    "imread",
    "imwrite",
    "audioread",
    "audiowrite",
];

/// True when calling `name` would use files. With evaluated arguments,
/// text and character-array `delete(filename)` calls count too.
pub(super) fn file_command(name: &str, args: Option<&[Value]>) -> bool {
    if FILE_COMMANDS.contains(&name) {
        return true;
    }
    match (name, args) {
        ("delete", Some([Value::Text(_) | Value::QuotedString(_), ..])) => true,
        ("delete", Some([Value::Container(Array::Character(_)), ..])) => true,
        _ => false,
    }
}

/// Queries handled here must bypass native metadata lookups entirely.
pub(super) fn query(name: &str) -> bool {
    matches!(name, "exist" | "isfile" | "isfolder")
}

impl Engine {
    pub(super) fn browser_query(&self, name: &str, args: &[Value]) -> Result<Option<Value>> {
        if !query(name) {
            return Ok(None);
        }
        if args.is_empty() {
            return Err("Not enough input arguments.".into());
        }
        if args.len() > if name == "exist" { 2 } else { 1 } {
            return Err("Too many input arguments.".into());
        }
        let target = self
            .text_value(args[0].clone())
            .map_err(|_| format!("{name}: name must be a character row"))?;
        if name != "exist" {
            return Ok(Some(Value::Logical(LogicalMatrix::scalar(false))));
        }
        let kind = args.get(1).map(|value| self.text_value(value.clone()));
        let kind = match kind {
            None => String::new(),
            Some(Ok(kind)) if matches!(kind.as_str(), "var" | "file" | "dir" | "builtin") => kind,
            _ => return Err("exist: type must be 'var', 'file', 'dir' or 'builtin'".into()),
        };
        let is_var = self.contains_array(&target);
        let builtin = crate::session::FUNCTIONS.contains(&target.as_str());
        let code = match kind.as_str() {
            "var" => f64::from(u8::from(is_var)),
            "file" | "dir" => 0.,
            "builtin" => {
                if builtin {
                    5.
                } else {
                    0.
                }
            }
            _ if is_var => 1.,
            _ if builtin => 5.,
            _ => 0.,
        };
        Ok(Some(Value::Matrix(Matrix::scalar(code))))
    }
}
