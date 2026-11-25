//! Strict bounded text-matrix script I/O; paths belong to the engine workspace.
use super::{Engine, Value};
use crate::{Result, files};
use std::{collections::BTreeSet, path::Path};

struct Options {
    delimiter: Option<char>,
    delimiter_list: bool,
    explicit_text: bool,
}

fn parse_delimiter(value: &str, writing: bool) -> Result<char> {
    match value {
        "," => Ok(','),
        ";" => Ok(';'),
        "\t" | "\\t" => Ok('\t'),
        " " => Ok(' '),
        _ if value.eq_ignore_ascii_case("space") || value.eq_ignore_ascii_case("whitespace") => {
            Ok(' ')
        }
        _ if writing && value.eq_ignore_ascii_case("comma") => Ok(','),
        _ if writing && value.eq_ignore_ascii_case("semi") => Ok(';'),
        _ if writing && value.eq_ignore_ascii_case("tab") => Ok('\t'),
        _ => Err("Supported numeric text delimiters are comma, semicolon, tab and space".into()),
    }
}

fn options(args: &[Value], writing: bool) -> Result<Options> {
    if !args.len().is_multiple_of(2) {
        return Err("Text matrix options must be quoted name/value pairs".into());
    }
    let mut options = Options {
        delimiter: if writing { Some(',') } else { None },
        delimiter_list: false,
        explicit_text: false,
    };
    let mut seen = BTreeSet::new();
    for pair in args.as_chunks::<2>().0 {
        if let [Value::Text(name), value] = pair
            && name.eq_ignore_ascii_case("delimiter")
            && let Some(list) = super::import_options::text_list(value)
        {
            if !seen.insert("delimiter".to_owned()) {
                return Err("Duplicate text matrix option: delimiter".into());
            }
            if list.is_empty() {
                return Err(
                    "Supported numeric text delimiters are comma, semicolon, tab and space".into(),
                );
            }
            let mut chars = Vec::with_capacity(list.len());
            for item in &list {
                let ch = parse_delimiter(item, writing)?;
                if !chars.contains(&ch) {
                    chars.push(ch);
                }
            }
            if writing
                && !(chars.len() == 1
                    || (chars.contains(&' ') && chars.iter().all(|&c| c == ' ' || c == '\t')))
            {
                return Err(
                    "Supported numeric text delimiters are comma, semicolon, tab and space".into(),
                );
            }
            if chars.len() == 1 || writing {
                options.delimiter = Some(if writing && chars.contains(&' ') {
                    ' '
                } else {
                    chars[0]
                });
                options.delimiter_list = false;
            } else {
                options.delimiter = None;
                options.delimiter_list = true;
            }
            continue;
        }
        let [Value::Text(name), Value::Text(value)] = pair else {
            return Err("Text matrix option names and values must be quoted text".into());
        };
        let name = name.to_ascii_lowercase();
        if !seen.insert(name.clone()) {
            return Err(format!("Duplicate text matrix option: {name}"));
        }
        match name.as_str() {
            "filetype" if value.eq_ignore_ascii_case("text") => options.explicit_text = true,
            "filetype" if writing && value.eq_ignore_ascii_case("spreadsheet") => {
                return Err(super::readers::rdx_sheet_write_error());
            }
            // readmatrix sends 'FileType','spreadsheet' to the spreadsheet
            // reader before these text options are parsed.
            "filetype" if !writing => {
                return Err("readmatrix: FileType must be 'text' or 'spreadsheet'".into());
            }
            "filetype" => {
                return Err(
                    "Only FileType 'text' is supported; spreadsheet files are not implemented here"
                        .into(),
                );
            }
            "delimiter" => {
                options.delimiter = Some(parse_delimiter(value, writing)?);
                options.delimiter_list = false;
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
        if writing
            && !explicit_text
            && extension.is_some_and(|e| {
                ["xlsx", "xlsm", "xlsb", "xls"]
                    .iter()
                    .any(|s| e.eq_ignore_ascii_case(s))
            })
        {
            return Err(super::readers::rdx_sheet_write_error());
        }
        if !explicit_text {
            match extension {
                Some(e) if ["csv","txt","dat"].iter().any(|allowed|e.eq_ignore_ascii_case(allowed)) => {},
                None if writing => {},
                _ => return Err("Use a .csv, .txt or .dat filename, or specify 'FileType','text'; spreadsheets are not supported".into()),
            }
        }
        if writing {
            self.file_path(filename, if explicit_text { "" } else { ".txt" })
        } else {
            self.files_read_path(filename, "")
        }
    }

    pub(super) fn read_matrix_command(&self, args: Vec<Value>) -> Result<Value> {
        self.check_cancel()?;
        if args
            .get(1)
            .is_some_and(|v| super::objects::import_value(self, v).is_some())
        {
            return super::import_options::readmatrix(self, &args, None);
        }
        let [Value::Text(filename), rest @ ..] = args.as_slice() else {
            return Err(
                "Use readmatrix('filename'[, 'Delimiter', character][, 'FileType', 'text'])".into(),
            );
        };
        if super::readers::rdx_sheet_request(filename, rest) {
            return self.rdx_read_matrix_sheet(filename, rest);
        }
        let options = options(rest, false)?;
        let path = self.matrix_text_path(filename, options.explicit_text, false)?;
        if options.delimiter_list {
            return super::import_options::readmatrix(
                self,
                &args,
                Some("Invalid numeric field".into()),
            );
        }
        let matrix = match files::read_matrix_text(&path, options.delimiter, &self.cancel) {
            Ok(matrix) => matrix,
            Err(original) => return super::import_options::readmatrix(self, &args, Some(original)),
        };
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
