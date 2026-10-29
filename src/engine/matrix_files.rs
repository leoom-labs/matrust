//! Strict bounded text-matrix script I/O; paths belong to the engine workspace.
use super::{Engine, Value};
use crate::{Result, files};
use std::{collections::BTreeSet, path::Path};

struct Options {
    delimiter: Option<char>,
    explicit_text: bool,
}

fn options(args: &[Value], writing: bool) -> Result<Options> {
    if !args.len().is_multiple_of(2) {
        return Err("Text matrix options must be quoted name/value pairs".into());
    }
    let mut options = Options {
        delimiter: if writing { Some(',') } else { None },
        explicit_text: false,
    };
    let mut seen = BTreeSet::new();
    for pair in args.as_chunks::<2>().0 {
        let [Value::Text(name), Value::Text(value)] = pair else {
            return Err("Text matrix option names and values must be quoted text".into());
        };
        let name = name.to_ascii_lowercase();
        if !seen.insert(name.clone()) {
            return Err(format!("Duplicate text matrix option: {name}"));
        }
        match name.as_str() {
            "filetype" if value.eq_ignore_ascii_case("text") => options.explicit_text = true,
            "filetype" => {
                return Err(
                    "Only FileType 'text' is supported; spreadsheet files are not implemented here"
                        .into(),
                );
            }
            "delimiter" => {
                options.delimiter = Some(match value.as_str() {
                    "," => ',',
                    ";" => ';',
                    "\t" | "\\t" => '\t',
                    _ if writing && value.eq_ignore_ascii_case("comma") => ',',
                    _ if writing && value.eq_ignore_ascii_case("semi") => ';',
                    _ if writing && value.eq_ignore_ascii_case("tab") => '\t',
                    _ => {
                        return Err(
                            "Supported numeric text delimiters are comma, semicolon and tab".into(),
                        );
                    }
                });
            }
            "writemode" if writing && value.eq_ignore_ascii_case("overwrite") => {}
            "writemode" if writing => {
                return Err(
                    "writematrix supports only WriteMode 'overwrite'; append is not implemented"
                        .into(),
                );
            }
            _ => {
                return Err(format!(
                    "Unsupported text matrix option: {name}; headers, ranges, missing-value rules and import-option objects are not implemented"
                ));
            }
        }
    }
    Ok(options)
}

impl Engine {
    fn matrix_text_path(
        &self,
        filename: &str,
        explicit_text: bool,
        writing: bool,
    ) -> Result<std::path::PathBuf> {
        let extension = Path::new(filename)
            .extension()
            .and_then(|e| e.to_str())
            .filter(|e| !e.is_empty());
        if !explicit_text {
            match extension {
                Some(e) if ["csv","txt","dat"].iter().any(|allowed|e.eq_ignore_ascii_case(allowed)) => {},
                None if writing => {},
                _ => return Err("Use a .csv, .txt or .dat filename, or specify 'FileType','text'; spreadsheets are not supported".into()),
            }
        }
        self.file_path(
            filename,
            if writing && !explicit_text {
                ".txt"
            } else {
                ""
            },
        )
    }

    pub(super) fn read_matrix_command(&self, args: Vec<Value>) -> Result<Value> {
        self.check_cancel()?;
        let [Value::Text(filename), rest @ ..] = args.as_slice() else {
            return Err(
                "Use readmatrix('filename'[, 'Delimiter', character][, 'FileType', 'text'])".into(),
            );
        };
        let options = options(rest, false)?;
        let path = self.matrix_text_path(filename, options.explicit_text, false)?;
        let matrix = files::read_matrix_text(&path, options.delimiter, &self.cancel)?;
        self.check_cancel()?;
        Ok(Value::Matrix(matrix))
    }

    pub(super) fn write_matrix_command(&self, args: Vec<Value>) -> Result<()> {
        self.check_cancel()?;
        let [array, Value::Text(filename), rest @ ..] = args.as_slice() else {
            return Err("Use writematrix(A,'filename'[, 'Delimiter', character][, 'FileType', 'text']); an explicit filename is required".into());
        };
        let options = options(rest, true)?;
        let path = self.matrix_text_path(filename, options.explicit_text, true)?;
        let array = array.clone().array(&self.cancel)?;
        files::write_matrix_text(
            &path,
            &array,
            options.delimiter.unwrap_or(','),
            &self.cancel,
        )
    }
}
