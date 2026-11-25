//! Import Tool: reads a delimited text file (.csv .tsv .txt .dat) or an
//! .xlsx workbook into workspace variables. The file settings are
//! `detectImportOptions` arguments; column types, input formats, names, the
//! column selection, the data lines and the unimportable-cell rule become the
//! non-default `setvartype`/`setvaropts`/property lines; one read call
//! (`readtable`, `readtimetable`, `readmatrix` or `readcell`) makes the
//! output. The grid is the engine's own read of the first rows with the same
//! options, so it shows what the import creates.
use super::{App, AppKind, Outputs, Snapshot, field, name_field, unused_name};
use crate::theme::ThemedScrollArea;
use eframe::egui;
use std::path::{Path, PathBuf};

/// Rows the preview grid reads.
pub const PREVIEW_ROWS: usize = 200;
/// Files the Recent list keeps.
pub const RECENT: usize = 8;
/// Extensions the file picker offers: delimited text, then workbooks.
#[cfg_attr(test, allow(dead_code))]
pub const EXTENSIONS: [&str; 5] = ["csv", "tsv", "txt", "dat", "xlsx"];
/// Column types offered in the grid header (setvartype names).
pub const TYPES: [&str; 10] = [
    "double",
    "single",
    "int32",
    "int64",
    "string",
    "char",
    "categorical",
    "datetime",
    "duration",
    "logical",
];
/// Functions the script and its probe call: an output never takes their
/// names, so the private run measures the output and not a shadowed call.
const USED: [&str; 18] = [
    "detectImportOptions",
    "setvartype",
    "setvaropts",
    "readtable",
    "readtimetable",
    "readmatrix",
    "readcell",
    "table2array",
    "clear",
    "string",
    "cell",
    "height",
    "width",
    "min",
    "ismissing",
    "size",
    "class",
    "numel",
];
/// A name the import can create: a valid, unreserved variable name.
fn usable(name: &str) -> bool {
    crate::workspace_edit::validate_name(name).is_ok() && !USED.contains(&name)
}
/// `base` (a usable name), else `base1`, `base2`, ... with the base cut so
/// the number fits in 63 characters: the first name `taken` refuses.
fn fresh_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if usable(base) && !taken(base) {
        return base.to_owned();
    }
    let stem: String = base.chars().take(56).collect();
    unused_name(&stem, |n| n == stem || !usable(n) || taken(n))
}
/// Rows of the Output column's name and type lists (the buttons stay in view).
const LISTED: usize = 8;
/// Bytes of a text file checked for NUL (a binary file).
const SNIFF: usize = 8192;
/// Largest workbook part read for the sheet names.
const WORKBOOK_PART: u64 = 1 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delimiter {
    Auto,
    Comma,
    Tab,
    Semicolon,
    Space,
    Custom,
}
impl Delimiter {
    pub const ALL: [Self; 6] = [
        Self::Auto,
        Self::Comma,
        Self::Tab,
        Self::Semicolon,
        Self::Space,
        Self::Custom,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Automatic",
            Self::Comma => "Comma",
            Self::Tab => "Tab",
            Self::Semicolon => "Semicolon",
            Self::Space => "Space",
            Self::Custom => "Other",
        }
    }
    /// The Delimiter argument as MATLAB text (`\t` is a tab escape).
    fn text(self, custom: &str) -> Result<Option<String>, String> {
        Ok(Some(
            match self {
                Self::Auto => return Ok(None),
                Self::Comma => ",",
                Self::Tab => "\\t",
                Self::Semicolon => ";",
                Self::Space => " ",
                Self::Custom if custom.is_empty() => {
                    return Err("Import Tool: type the other delimiter".into());
                }
                Self::Custom => custom,
            }
            .to_owned(),
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decimal {
    Auto,
    Period,
    Comma,
}
impl Decimal {
    pub const ALL: [Self; 3] = [Self::Auto, Self::Period, Self::Comma];
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Automatic",
            Self::Period => "Period ( . )",
            Self::Comma => "Comma ( , )",
        }
    }
    fn text(self) -> Option<&'static str> {
        match self {
            Self::Auto => None,
            Self::Period => Some("."),
            Self::Comma => Some(","),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Thousands {
    None,
    Comma,
    Period,
    Space,
    Apostrophe,
}
impl Thousands {
    pub const ALL: [Self; 5] = [
        Self::None,
        Self::Comma,
        Self::Period,
        Self::Space,
        Self::Apostrophe,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Comma => "Comma ( , )",
            Self::Period => "Period ( . )",
            Self::Space => "Space",
            Self::Apostrophe => "Apostrophe ( ' )",
        }
    }
    fn text(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Comma => Some(","),
            Self::Period => Some("."),
            Self::Space => Some(" "),
            Self::Apostrophe => Some("'"),
        }
    }
}

/// What the import creates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    Table,
    Timetable,
    /// One column vector per selected column, named after the column.
    Columns,
    Matrix,
    Cells,
    Strings,
}
impl Output {
    pub const ALL: [Self; 6] = [
        Self::Table,
        Self::Timetable,
        Self::Columns,
        Self::Matrix,
        Self::Cells,
        Self::Strings,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Table => "Table",
            Self::Timetable => "Timetable",
            Self::Columns => "Column vectors",
            Self::Matrix => "Numeric matrix",
            Self::Cells => "Cell array",
            Self::Strings => "String array",
        }
    }
}

/// One column: as detected and as the user set it.
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    /// Name and type detectImportOptions chose.
    pub detected: String,
    pub detected_type: String,
    pub name: String,
    pub kind: String,
    /// InputFormat of a datetime column; empty: the engine reads the format
    /// from the data.
    pub format: String,
    pub selected: bool,
}
impl Column {
    pub fn new(name: String, kind: String) -> Self {
        Self {
            name: name.clone(),
            detected: name,
            kind: kind.clone(),
            detected_type: kind,
            format: String::new(),
            selected: true,
        }
    }
    fn time(&self) -> bool {
        matches!(self.kind.as_str(), "datetime" | "duration")
    }
}

/// The chosen file, inspected once when it is chosen.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub path: PathBuf,
    /// Size in bytes; None when the file could not be read.
    pub bytes: Option<u64>,
    /// Read as a workbook (.xlsx).
    pub sheet: bool,
    /// Worksheet names in workbook order (.xlsx).
    pub sheets: Vec<String>,
    /// A text file holding NUL bytes in its first 8 KiB.
    pub binary: bool,
}
impl Source {
    pub fn inspect(path: PathBuf) -> Self {
        let sheet = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("xlsx"));
        let bytes = std::fs::metadata(&path)
            .ok()
            .filter(|m| m.is_file())
            .map(|m| m.len());
        let (sheets, binary) = match (bytes, sheet) {
            (None, _) => (Vec::new(), false),
            (Some(_), true) => (sheet_names(&path).unwrap_or_default(), false),
            (Some(_), false) => (Vec::new(), looks_binary(&path)),
        };
        Self {
            path,
            bytes,
            sheet,
            sheets,
            binary,
        }
    }
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

/// A NUL byte in the first bytes: not a text file.
fn looks_binary(path: &Path) -> bool {
    use std::io::Read;
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = Vec::with_capacity(SNIFF);
    if file.take(SNIFF as u64).read_to_end(&mut head).is_err() {
        return false;
    }
    head.contains(&0)
}

/// Worksheet names of an .xlsx workbook, in tab order (xl/workbook.xml).
pub fn sheet_names(path: &Path) -> Result<Vec<String>, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let part = zip.by_name("xl/workbook.xml").map_err(|e| e.to_string())?;
    let mut text = String::new();
    part.take(WORKBOOK_PART)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    Ok(sheet_tags(&text))
}

/// The `name` attribute of every `<sheet>` element (any namespace prefix).
fn sheet_tags(xml: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = xml;
    while let Some(at) = rest.find('<') {
        rest = &rest[at + 1..];
        let end = rest.find('>').unwrap_or(rest.len());
        let tag = &rest[..end];
        let element = tag.split(|c: char| c.is_whitespace()).next().unwrap_or("");
        if (element == "sheet" || element.ends_with(":sheet"))
            && let Some(name) = attribute(tag, "name")
        {
            names.push(name);
        }
        rest = &rest[end.min(rest.len())..];
    }
    names
}
fn attribute(tag: &str, wanted: &str) -> Option<String> {
    let mut rest = tag;
    while let Some(eq) = rest.find('=') {
        let key = rest[..eq].trim();
        let key = key.rsplit(|c: char| c.is_whitespace()).next().unwrap_or("");
        let value = rest[eq + 1..].trim_start();
        let quote = value.chars().next()?;
        if quote != '"' && quote != '\'' {
            return None;
        }
        let close = value[1..].find(quote)? + 1;
        if key == wanted {
            return Some(unescape_xml(&value[1..close]));
        }
        rest = &value[close + 1..];
    }
    None
}
fn unescape_xml(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// MATLAB character vector literal.
pub fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}
fn cell_list<'a>(items: impl IntoIterator<Item = &'a str>) -> String {
    let items: Vec<String> = items.into_iter().map(quote).collect();
    format!("{{{}}}", items.join(", "))
}

/// What detection found, for the hints next to the inputs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Detected {
    /// First data line (text) or row (workbook).
    pub first: usize,
    /// Last data line; None: to the end.
    pub last: Option<usize>,
    /// Line or row of the variable names; 0: none.
    pub names_line: usize,
    /// Delimiters of a text file.
    pub delimiter: Vec<String>,
    /// Column letters of the first data cell (workbooks).
    pub start_column: String,
    /// Every column numeric and no names line: a numeric matrix by default.
    pub numeric: bool,
}

/// The engine's read of the first rows: cell text and missing cells.
#[derive(Clone, Debug)]
pub struct Preview {
    pub rows: usize,
    pub cols: usize,
    /// One string column per table variable (a string array holds at most
    /// 65536 elements, a column 200).
    pub text: Vec<matrust::StringArray>,
    /// Column-major, 1 where the cell becomes missing.
    pub missing: Vec<u8>,
}
impl Preview {
    pub fn cell(&self, row: usize, col: usize) -> &str {
        self.text
            .get(col)
            .and_then(|column| column.data().get(row))
            .and_then(|t| t.as_deref())
            .unwrap_or("")
    }
    pub fn is_missing(&self, row: usize, col: usize) -> bool {
        self.missing
            .get(col * self.rows + row)
            .is_some_and(|m| *m != 0)
    }
    /// Cells that become missing among the selected columns.
    pub fn missing_count(&self, columns: &[Column]) -> usize {
        (0..self.cols)
            .filter(|c| columns.get(*c).is_some_and(|col| col.selected))
            .map(|c| (0..self.rows).filter(|r| self.is_missing(*r, c)).count())
            .sum()
    }
}

/// Size and class of what the import created privately.
#[derive(Clone, Debug, PartialEq)]
pub struct Created {
    pub size: Vec<usize>,
    pub class: String,
}

/// Names the script uses.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub output: Output,
    /// File name as a MATLAB literal.
    pub file: String,
    /// The options variable (cleared at the end).
    pub opts: String,
    /// Output variables; for column vectors (variable, column name).
    pub names: Vec<(String, String)>,
    /// The table column vectors are taken from (cleared at the end).
    pub table: String,
    /// Row-times column of a timetable.
    pub row_times: String,
}

const PROBE_COMMON: [&str; 6] = [
    "app_dnames",
    "app_dtypes",
    "app_text",
    "app_miss",
    "app_size",
    "app_class",
];
const PROBE_TEXT: [&str; 3] = ["app_dlines", "app_dnames_line", "app_ddelim"];
const PROBE_SHEET: [&str; 2] = ["app_drange", "app_dnames_range"];

fn strings(outputs: &Outputs, name: &str) -> Result<Vec<String>, String> {
    match outputs.get(name) {
        Some(matrust::Array::Str(s)) => Ok(s
            .data()
            .iter()
            .map(|t| t.as_deref().unwrap_or("").to_owned())
            .collect()),
        _ => Err(format!("Import Tool: {name} is not a string array")),
    }
}
fn numbers(outputs: &Outputs, name: &str) -> Result<Vec<f64>, String> {
    match outputs.get(name) {
        Some(matrust::Array::Numeric(m)) => Ok(m.data().to_vec()),
        _ => Err(format!("Import Tool: {name} is not a real array")),
    }
}
/// Row number of a cell reference such as `B12`; 0 for an empty range.
fn range_row(range: &str) -> usize {
    let first = range.split(':').next().unwrap_or("");
    first
        .trim_start_matches(|c: char| c.is_ascii_alphabetic())
        .parse()
        .unwrap_or(0)
}
fn range_column(range: &str) -> String {
    range
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_ascii_uppercase()
}
fn line_number(text: &str, what: &str) -> Result<Option<usize>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    text.parse::<usize>()
        .ok()
        .filter(|n| *n <= 100_000_000)
        .map(Some)
        .ok_or_else(|| format!("Import Tool: {what} must be a whole number"))
}

pub struct ImportTool {
    pub source: Option<Source>,
    /// Recently chosen files, newest first.
    pub recent: Vec<(PathBuf, Option<u64>)>,
    pub delimiter: Delimiter,
    pub custom: String,
    /// Empty: detected.
    pub header_lines: String,
    pub names_line: String,
    pub decimal: Decimal,
    pub thousands: Thousands,
    /// Index into the workbook's sheets; 0 is the first (default) sheet.
    pub sheet: usize,
    /// Data lines (rows of a workbook); empty: detected / to the end.
    pub first: String,
    pub last: String,
    /// Omit rows with unimportable or missing cells instead of filling them.
    pub omit_rows: bool,
    /// None: a table, or a numeric matrix for a plain numeric file.
    pub output: Option<Output>,
    /// Row-times column (its current name) of a timetable.
    pub row_times: String,
    pub name: String,
    pub name_edited: bool,
    /// Columns of the current detection; None until a run delivers it.
    pub columns: Option<Vec<Column>>,
    pub detected: Option<Detected>,
    pub preview: Option<Preview>,
    pub created: Option<Created>,
    /// Choose file was asked for: the desktop shows the picker at the end of
    /// the frame.
    pub pick: bool,
    /// Horizontal scroll of the grid; the header follows it.
    scroll_x: f32,
    identity: Option<String>,
}
impl Default for ImportTool {
    fn default() -> Self {
        Self {
            source: None,
            recent: Vec::new(),
            delimiter: Delimiter::Auto,
            custom: String::new(),
            header_lines: String::new(),
            names_line: String::new(),
            decimal: Decimal::Auto,
            thousands: Thousands::None,
            sheet: 0,
            first: String::new(),
            last: String::new(),
            omit_rows: false,
            output: None,
            row_times: String::new(),
            name: String::new(),
            name_edited: false,
            columns: None,
            detected: None,
            preview: None,
            created: None,
            pick: false,
            scroll_x: 0.,
            identity: None,
        }
    }
}

impl ImportTool {
    /// A new file: fresh settings, detected again; the Recent list keeps it.
    pub fn choose(&mut self, path: PathBuf) {
        let source = Source::inspect(path);
        self.recent.retain(|(p, _)| *p != source.path);
        self.recent.insert(0, (source.path.clone(), source.bytes));
        self.recent.truncate(RECENT);
        let recent = std::mem::take(&mut self.recent);
        *self = Self {
            source: Some(source),
            recent,
            ..Self::default()
        };
    }
    fn sheet_file(&self) -> bool {
        self.source.as_ref().is_some_and(|s| s.sheet)
    }
    /// The output kind: the user's, else a numeric matrix for a plain
    /// numeric file (what the former CSV import made), else a table.
    pub fn output(&self) -> Output {
        self.output.unwrap_or_else(|| {
            if self.detected.as_ref().is_some_and(|d| d.numeric) {
                Output::Matrix
            } else {
                Output::Table
            }
        })
    }
    /// `, 'Name', value` pairs of detectImportOptions; only the settings the
    /// user changed.
    pub fn detect_arguments(&self) -> Result<String, String> {
        let mut args = String::new();
        if self.sheet_file() {
            let sheets = self
                .source
                .as_ref()
                .map(|s| s.sheets.as_slice())
                .unwrap_or_default();
            if self.sheet > 0
                && let Some(name) = sheets.get(self.sheet)
            {
                args.push_str(&format!(", 'Sheet', {}", quote(name)));
            }
            return Ok(args);
        }
        if let Some(text) = self.delimiter.text(&self.custom)? {
            args.push_str(&format!(", 'Delimiter', {}", quote(&text)));
        }
        if let Some(n) = line_number(&self.header_lines, "Header lines")? {
            args.push_str(&format!(", 'NumHeaderLines', {n}"));
        }
        if let Some(n) = line_number(&self.names_line, "Variable names line")? {
            args.push_str(&format!(", 'VariableNamesLine', {n}"));
        }
        if let Some(text) = self.decimal.text() {
            args.push_str(&format!(", 'DecimalSeparator', {}", quote(text)));
        }
        if let Some(text) = self.thousands.text() {
            args.push_str(&format!(", 'ThousandsSeparator', {}", quote(text)));
        }
        Ok(args)
    }
    /// The detection settings: a change detects again.
    fn identity(&self) -> String {
        format!(
            "{:?}|{}|{:?}|{}|{}|{}|{:?}|{:?}",
            self.source.as_ref().map(|s| &s.path),
            self.sheet,
            self.delimiter,
            self.custom,
            self.header_lines.trim(),
            self.names_line.trim(),
            self.decimal,
            self.thousands
        )
    }
    fn file_literal(&self) -> Result<String, String> {
        let source = self
            .source
            .as_ref()
            .ok_or("Import Tool: choose a file to import")?;
        if source.binary {
            return Err(format!(
                "Import Tool: {} is not a text file (it holds NUL bytes); choose a UTF-8 \
                 .csv, .tsv, .txt or .dat file or an .xlsx workbook",
                source.name()
            ));
        }
        Ok(quote(&source.path.to_string_lossy()))
    }
    /// Columns that are read.
    fn selected(&self) -> impl Iterator<Item = &Column> {
        self.columns.iter().flatten().filter(|c| c.selected)
    }
    fn check_columns(&self) -> Result<(), String> {
        let Some(columns) = &self.columns else {
            return Ok(());
        };
        for (k, column) in columns.iter().enumerate() {
            if crate::workspace_edit::validate_name(column.name.trim()).is_err() {
                return Err(format!(
                    "Import Tool: column {} needs a valid variable name (letters, digits \
                     and underscores, starting with a letter)",
                    k + 1
                ));
            }
            if columns[..k]
                .iter()
                .any(|c| c.name.trim() == column.name.trim())
            {
                return Err(format!(
                    "Import Tool: two columns are named {}",
                    column.name.trim()
                ));
            }
            if column.format.contains('\'') && column.kind == "datetime" {
                return Err("Import Tool: an input format cannot hold a quote".into());
            }
        }
        if self.selected().next().is_none() {
            return Err("Import Tool: select at least one column".into());
        }
        Ok(())
    }
    /// The default output name: the file name, else `data`, never a
    /// workspace variable.
    pub fn default_name(&self, ws: &Snapshot) -> String {
        let stem = self
            .source
            .as_ref()
            .and_then(|s| s.path.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let base = if usable(&stem) {
            stem
        } else {
            "data".to_owned()
        };
        fresh_name(&base, |n| ws.contains(n))
    }
    fn check_output_name(name: &str) -> Result<(), String> {
        super::check_export_name(name)?;
        if crate::workspace_edit::validate_name(name).is_err() {
            return Err("Import Tool: the output name is not a valid variable name".into());
        }
        if USED.contains(&name) {
            return Err(format!(
                "Import Tool: {name} is a function the import calls; choose another name"
            ));
        }
        Ok(())
    }
    /// Output names, the options variable and the temporary table.
    pub fn plan(&self, ws: &Snapshot) -> Result<Plan, String> {
        let file = self.file_literal()?;
        self.check_columns()?;
        let output = self.output();
        let mut names = Vec::new();
        let mut row_times = String::new();
        if output == Output::Columns {
            let columns: Vec<&str> = self.selected().map(|c| c.name.trim()).collect();
            for (k, column) in columns.iter().enumerate() {
                let name = fresh_name(column, |n| {
                    ws.contains(n)
                        || names.iter().any(|(v, _): &(String, String)| v == n)
                        || columns.iter().enumerate().any(|(j, c)| j != k && *c == n)
                });
                super::check_export_name(&name)?;
                names.push((name, (*column).to_owned()));
            }
        } else {
            let name = if self.name_edited {
                self.name.trim().to_owned()
            } else {
                self.default_name(ws)
            };
            Self::check_output_name(&name)?;
            names.push((name, String::new()));
        }
        if output == Output::Timetable && self.columns.is_some() {
            let first = self.selected().find(|c| c.time()).ok_or(
                "Import Tool: a timetable needs a selected datetime or duration column for \
                 its row times",
            )?;
            // readtimetable's default is the first one: named only otherwise.
            let chosen = self.row_times_column().unwrap_or(first);
            if !std::ptr::eq(chosen, first) {
                row_times = chosen.name.trim().to_owned();
            }
        }
        let taken = |n: &str| ws.contains(n) || names.iter().any(|(v, _)| v == n);
        let opts = unused_name("opts", taken);
        let table = if output == Output::Columns {
            unused_name("tbl", |n| taken(n) || n == opts)
        } else {
            String::new()
        };
        Ok(Plan {
            output,
            file,
            opts,
            names,
            table,
            row_times,
        })
    }
    /// Lines after detection, on the options variable `var`: only what the
    /// user changed.
    fn option_lines(&self, var: &str, output: Output) -> Result<String, String> {
        let mut lines = String::new();
        let columns = self.columns.as_deref().unwrap_or_default();
        if output != Output::Strings {
            for column in columns.iter().filter(|c| c.kind != c.detected_type) {
                lines.push_str(&format!(
                    "{var} = setvartype({var}, {}, {});\n",
                    quote(&column.detected),
                    quote(&column.kind)
                ));
            }
            for column in columns
                .iter()
                .filter(|c| c.kind == "datetime" && !c.format.trim().is_empty())
            {
                lines.push_str(&format!(
                    "{var} = setvaropts({var}, {}, 'InputFormat', {});\n",
                    quote(&column.detected),
                    quote(column.format.trim())
                ));
            }
        }
        if columns.iter().any(|c| c.name.trim() != c.detected) {
            lines.push_str(&format!(
                "{var}.VariableNames = {};\n",
                cell_list(columns.iter().map(|c| c.name.trim()))
            ));
        }
        if columns.iter().any(|c| !c.selected) {
            lines.push_str(&format!(
                "{var}.SelectedVariableNames = {};\n",
                cell_list(self.selected().map(|c| c.name.trim()))
            ));
        }
        let what = if self.sheet_file() {
            "Data rows"
        } else {
            "Data lines"
        };
        let first = line_number(&self.first, what)?;
        let last = line_number(&self.last, what)?;
        if first == Some(0) || last == Some(0) {
            return Err(format!("Import Tool: {what} start at 1"));
        }
        if let (Some(a), Some(b)) = (first, last)
            && b < a
        {
            return Err(format!("Import Tool: {what}: the last is before the first"));
        }
        if first.is_some() || last.is_some() {
            let detected = self.detected.as_ref();
            if self.sheet_file() {
                let start = first.or(detected.map(|d| d.first)).unwrap_or(1);
                let range = match last {
                    Some(last) => format!("{start}:{last}"),
                    None => {
                        let column = detected
                            .map(|d| d.start_column.clone())
                            .filter(|c| !c.is_empty())
                            .unwrap_or_else(|| "A".into());
                        format!("{column}{start}")
                    }
                };
                lines.push_str(&format!("{var}.DataRange = {};\n", quote(&range)));
            } else {
                let start = match (first, detected) {
                    (Some(first), _) => first.to_string(),
                    (None, Some(d)) => d.first.to_string(),
                    (None, None) => format!("{var}.DataLines(1)"),
                };
                let end = last.map_or("Inf".to_owned(), |l| l.to_string());
                lines.push_str(&format!("{var}.DataLines = [{start} {end}];\n"));
            }
        }
        if self.omit_rows {
            lines.push_str(&format!(
                "{var}.MissingRule = 'omitrow';\n{var}.ImportErrorRule = 'omitrow';\n"
            ));
        }
        if output == Output::Strings {
            lines.push_str(&format!("{var} = setvartype({var}, 'string');\n"));
        }
        Ok(lines)
    }
    /// The timetable's row-times column: the chosen one, else the first
    /// selected datetime or duration column.
    fn row_times_column(&self) -> Option<&Column> {
        let mut times = self.selected().filter(|c| c.time());
        let first = times.next()?;
        Some(
            std::iter::once(first)
                .chain(times)
                .find(|c| c.name.trim() == self.row_times.trim())
                .unwrap_or(first),
        )
    }
    /// The variable sizes and classes come from: the first output.
    fn primary(plan: &Plan) -> String {
        plan.names
            .first()
            .map(|(n, _)| n.clone())
            .unwrap_or_default()
    }
    /// Columns of the result, for the Output column: (name, type).
    pub fn result_columns(&self) -> Vec<(String, String)> {
        let output = self.output();
        let times = self
            .row_times_column()
            .filter(|_| output == Output::Timetable)
            .map(|c| c.name.trim().to_owned());
        self.selected()
            .filter(|c| times.as_deref() != Some(c.name.trim()))
            .map(|c| {
                let kind = match output {
                    Output::Matrix => "double",
                    Output::Cells => "cell",
                    Output::Strings => "string",
                    _ => c.kind.as_str(),
                };
                (c.name.trim().to_owned(), kind.to_owned())
            })
            .collect()
    }
    fn picker_row(&mut self, ui: &mut egui::Ui) {
        let p = crate::tokens::palette(ui.ctx());
        match &self.source {
            Some(source) => {
                ui.label(egui::RichText::new(source.name()).color(p.text).strong())
                    .on_hover_text(crate::path_label::display(&source.path).to_string());
                let mut detail = match source.bytes {
                    Some(bytes) => size_text(bytes),
                    None => "File not found".into(),
                };
                if source.sheet && !source.sheets.is_empty() {
                    detail.push_str(&format!(" · {} sheets", source.sheets.len()));
                }
                crate::tokens::helper(ui, detail);
            }
            None => {
                crate::tokens::helper(ui, "No file chosen");
            }
        }
        ui.add_space(crate::tokens::XS);
        if ui
            .add(
                egui::Button::new("Choose file…")
                    .min_size(egui::vec2(ui.available_width(), crate::tokens::CONTROL)),
            )
            .clicked()
        {
            self.pick = true;
        }
        if self.recent.len() > 1 {
            ui.add_space(crate::tokens::XS);
            let mut chosen = None;
            egui::ComboBox::from_id_salt("import_tool_recent")
                .width(ui.available_width())
                .selected_text("Recent files")
                .show_ui(ui, |ui| {
                    for (path, bytes) in &self.recent {
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let size = bytes.map(size_text).unwrap_or_else(|| "missing".into());
                        let current = self.source.as_ref().is_some_and(|s| s.path == *path);
                        if ui
                            .selectable_label(current, format!("{name}  ·  {size}"))
                            .on_hover_text(crate::path_label::display(path).to_string())
                            .clicked()
                        {
                            chosen = Some(path.clone());
                        }
                    }
                });
            if let Some(path) = chosen {
                self.choose(path);
            }
        }
        ui.add_space(crate::tokens::S);
    }
    fn text_settings_ui(&mut self, ui: &mut egui::Ui) {
        let detected = self.detected.clone().unwrap_or_default();
        field(ui, "Delimiter", |ui| {
            egui::ComboBox::from_id_salt("import_tool_delimiter")
                .width(ui.available_width())
                .selected_text(self.delimiter.label())
                .show_ui(ui, |ui| {
                    for d in Delimiter::ALL {
                        ui.selectable_value(&mut self.delimiter, d, d.label());
                    }
                });
            if self.delimiter == Delimiter::Custom {
                ui.add(
                    crate::tokens::input(
                        egui::TextEdit::singleline(&mut self.custom)
                            .char_limit(8)
                            .hint_text("Character(s)"),
                    )
                    .desired_width(ui.available_width()),
                );
            } else if self.delimiter == Delimiter::Auto && !detected.delimiter.is_empty() {
                let shown: Vec<String> = detected.delimiter.iter().map(|d| describe(d)).collect();
                crate::tokens::helper(ui, format!("Detected: {}", shown.join(", ")));
            }
        });
        let names_hint = match (&self.columns, detected.names_line) {
            (Some(_), 0) => "Auto (none)".to_owned(),
            (Some(_), n) => format!("Auto ({n})"),
            (None, _) => "Auto".to_owned(),
        };
        // Two settings per row keep the column short.
        ui.columns(2, |halves| {
            field(&mut halves[0], "Header lines", |ui| {
                ui.add(
                    crate::tokens::input(
                        egui::TextEdit::singleline(&mut self.header_lines)
                            .char_limit(9)
                            .hint_text("Auto"),
                    )
                    .desired_width(ui.available_width()),
                )
                .on_hover_text("Lines to skip before the data; empty: detected");
            });
            field(&mut halves[1], "Names line", |ui| {
                ui.add(
                    crate::tokens::input(
                        egui::TextEdit::singleline(&mut self.names_line)
                            .char_limit(9)
                            .hint_text(names_hint),
                    )
                    .desired_width(ui.available_width()),
                )
                .on_hover_text("Line holding the variable names; 0 reads none (Var1, Var2, …)");
            });
        });
        ui.columns(2, |halves| {
            field(&mut halves[0], "Decimal", |ui| {
                egui::ComboBox::from_id_salt("import_tool_decimal")
                    .width(ui.available_width())
                    .selected_text(self.decimal.label())
                    .show_ui(ui, |ui| {
                        for d in Decimal::ALL {
                            ui.selectable_value(&mut self.decimal, d, d.label());
                        }
                    })
                    .response
                    .on_hover_text("Decimal separator");
            });
            field(&mut halves[1], "Thousands", |ui| {
                egui::ComboBox::from_id_salt("import_tool_thousands")
                    .width(ui.available_width())
                    .selected_text(self.thousands.label())
                    .show_ui(ui, |ui| {
                        for t in Thousands::ALL {
                            ui.selectable_value(&mut self.thousands, t, t.label());
                        }
                    })
                    .response
                    .on_hover_text("Thousands separator");
            });
        });
    }
    fn sheet_ui(&mut self, ui: &mut egui::Ui) {
        let sheets = self
            .source
            .as_ref()
            .map(|s| s.sheets.clone())
            .unwrap_or_default();
        field(ui, "Sheet", |ui| {
            let shown = sheets
                .get(self.sheet)
                .cloned()
                .unwrap_or_else(|| "First sheet".into());
            egui::ComboBox::from_id_salt("import_tool_sheet")
                .width(ui.available_width())
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    for (k, name) in sheets.iter().enumerate() {
                        ui.selectable_value(&mut self.sheet, k, name);
                    }
                });
        });
    }
    fn range_ui(&mut self, ui: &mut egui::Ui) {
        let detected = self.detected.clone();
        let (label, unit) = if self.sheet_file() {
            ("Data rows", "row")
        } else {
            ("Data lines", "line")
        };
        field(ui, label, |ui| {
            let half = ((ui.available_width() - crate::tokens::L - crate::tokens::S) / 2.).max(48.);
            ui.horizontal(|ui| {
                let first = detected
                    .as_ref()
                    .map_or("First".to_owned(), |d| d.first.to_string());
                let last = detected
                    .as_ref()
                    .and_then(|d| d.last)
                    .map_or("End".to_owned(), |l| l.to_string());
                ui.add(
                    crate::tokens::input(
                        egui::TextEdit::singleline(&mut self.first)
                            .char_limit(9)
                            .hint_text(first),
                    )
                    .desired_width(half),
                );
                ui.label("to");
                ui.add(
                    crate::tokens::input(
                        egui::TextEdit::singleline(&mut self.last)
                            .char_limit(9)
                            .hint_text(last),
                    )
                    .desired_width(half),
                );
            });
            crate::tokens::helper(ui, format!("File {unit} numbers; empty reads to the end"));
        });
        let Some(columns) = &mut self.columns else {
            return;
        };
        let count = columns.iter().filter(|c| c.selected).count();
        field(
            ui,
            &format!("Columns ({count} of {})", columns.len()),
            |ui| {
                ui.horizontal(|ui| {
                    if ui.button("All").clicked() {
                        columns.iter_mut().for_each(|c| c.selected = true);
                    }
                    if ui.button("None").clicked() {
                        columns.iter_mut().for_each(|c| c.selected = false);
                    }
                });
                let row = crate::tokens::ROW;
                egui::ScrollArea::vertical()
                    .id_salt("import_tool_column_list")
                    .max_height(5. * row)
                    .auto_shrink([false, true])
                    .show_rows_themed(ui, row, columns.len(), |ui, range| {
                        for column in &mut columns[range] {
                            ui.checkbox(
                                &mut column.selected,
                                format!("{}  ({})", column.name, column.kind),
                            );
                        }
                    });
            },
        );
    }
    fn rule_ui(&mut self, ui: &mut egui::Ui) {
        field(ui, "Unimportable cells", |ui| {
            ui.horizontal(|ui| {
                if crate::tokens::toggle(ui, !self.omit_rows, "Fill with missing").clicked() {
                    self.omit_rows = false;
                }
                if crate::tokens::toggle(ui, self.omit_rows, "Omit the row").clicked() {
                    self.omit_rows = true;
                }
            });
            crate::tokens::helper(
                ui,
                if self.omit_rows {
                    "Rows with an empty or unreadable cell are left out"
                } else {
                    "NaN, NaT, <missing> or an empty text"
                },
            );
        });
    }
    fn output_ui(&mut self, ui: &mut egui::Ui) {
        let current = self.output();
        field(ui, "Output type", |ui| {
            egui::ComboBox::from_id_salt("import_tool_output")
                .width(ui.available_width())
                .selected_text(current.label())
                .show_ui(ui, |ui| {
                    for kind in Output::ALL {
                        if ui.selectable_label(current == kind, kind.label()).clicked() {
                            self.output = Some(kind);
                        }
                    }
                });
        });
        if current == Output::Timetable {
            let times: Vec<String> = self
                .selected()
                .filter(|c| c.time())
                .map(|c| c.name.clone())
                .collect();
            if !times.contains(&self.row_times)
                && let Some(first) = times.first()
            {
                self.row_times = first.clone();
            }
            field(ui, "Row times", |ui| {
                let shown = if self.row_times.is_empty() {
                    "No datetime or duration column".to_owned()
                } else {
                    self.row_times.clone()
                };
                egui::ComboBox::from_id_salt("import_tool_row_times")
                    .width(ui.available_width())
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        for name in &times {
                            ui.selectable_value(&mut self.row_times, name.clone(), name);
                        }
                    });
            });
        }
    }
    /// The preview grid: a header per column (include, name, type, input
    /// format) over the engine's cells. Rows and columns outside the view
    /// are not laid out, so wide and long files scroll freely.
    fn grid(&mut self, ui: &mut egui::Ui) {
        const COL: f32 = 152.;
        const NUM: f32 = 48.;
        let row_h = crate::tokens::ROW;
        let p = crate::tokens::palette(ui.ctx());
        let omit = self.omit_rows;
        let Some(columns) = &mut self.columns else {
            return;
        };
        let preview = self.preview.as_ref();
        let rows = preview.map_or(0, |v| v.rows);
        let count = columns.len().max(preview.map_or(0, |v| v.cols));
        let width = NUM + count as f32 * COL;
        let header_h = 3. * (crate::tokens::CONTROL + crate::tokens::XS);
        let visible = |range: egui::Rangef| {
            let lo = ((range.min - NUM) / COL).floor().max(0.) as usize;
            let hi = (((range.max - NUM) / COL).ceil().max(0.) as usize + 1).min(count);
            lo..hi.max(lo)
        };
        egui::ScrollArea::horizontal()
            .id_salt("import_tool_header")
            .scroll_source(egui::scroll_area::ScrollSource::NONE)
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .horizontal_scroll_offset(self.scroll_x)
            .auto_shrink([false, true])
            .show_viewport(ui, |ui, viewport| {
                // As wide as the grid plus its vertical scroll bar, so both
                // reach the same offset at the right end.
                ui.set_min_size(egui::vec2(width + crate::tokens::L, header_h));
                let origin = ui.max_rect().min;
                for c in visible(viewport.x_range()) {
                    let Some(column) = columns.get_mut(c) else {
                        continue;
                    };
                    let rect = egui::Rect::from_min_size(
                        origin + egui::vec2(NUM + c as f32 * COL, 0.),
                        egui::vec2(COL - crate::tokens::S, header_h),
                    );
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(rect)
                            .id_salt(("import_tool_column", c)),
                        |ui| header_cell(ui, column, c),
                    );
                }
            });
        ui.add_space(crate::tokens::XS);
        let output = egui::ScrollArea::both()
            .id_salt("import_tool_grid")
            .auto_shrink([false, false])
            .show_viewport_themed(ui, |ui, viewport| {
                ui.set_min_size(egui::vec2(width, rows as f32 * row_h));
                let Some(preview) = preview else {
                    return;
                };
                let origin = ui.max_rect().min;
                let painter = ui.painter().clone();
                let font = egui::FontId::proportional(crate::tokens::BODY);
                let lo = (viewport.min.y / row_h).floor().max(0.) as usize;
                let hi = ((viewport.max.y / row_h).ceil() as usize + 1).min(rows);
                let cols = visible(viewport.x_range());
                for r in lo..hi.max(lo) {
                    let y = origin.y + r as f32 * row_h;
                    let omitted = omit
                        && (0..preview.cols).any(|c| {
                            columns.get(c).is_some_and(|col| col.selected)
                                && preview.is_missing(r, c)
                        });
                    painter.hline(
                        egui::Rangef::new(origin.x, origin.x + width),
                        y + row_h - 0.5,
                        egui::Stroke::new(1., p.hairline),
                    );
                    painter.text(
                        egui::pos2(origin.x + NUM - crate::tokens::S, y + row_h / 2.),
                        egui::Align2::RIGHT_CENTER,
                        (r + 1).to_string(),
                        font.clone(),
                        if omitted { p.error } else { p.text_muted },
                    );
                    for c in cols.clone() {
                        let rect = egui::Rect::from_min_size(
                            egui::pos2(origin.x + NUM + c as f32 * COL, y),
                            egui::vec2(COL, row_h),
                        );
                        let included = columns.get(c).is_some_and(|col| col.selected);
                        if preview.is_missing(r, c) && included {
                            painter.rect_filled(
                                rect.shrink(1.),
                                0.,
                                p.warning.gamma_multiply(0.22),
                            );
                        }
                        if c >= preview.cols {
                            continue;
                        }
                        let ink = if !included || omitted {
                            p.text_disabled
                        } else {
                            p.text
                        };
                        painter
                            .with_clip_rect(rect.shrink(1.).intersect(ui.clip_rect()))
                            .text(
                                egui::pos2(rect.left() + crate::tokens::S, rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                preview.cell(r, c),
                                font.clone(),
                                ink,
                            );
                    }
                }
            });
        let x = output.state.offset.x;
        if (x - self.scroll_x).abs() > 0.25 {
            self.scroll_x = x;
            ui.ctx().request_repaint();
        }
    }
}

/// One header cell: include box and name, type, datetime input format.
fn header_cell(ui: &mut egui::Ui, column: &mut Column, index: usize) {
    let width = ui.available_width();
    ui.spacing_mut().item_spacing.y = crate::tokens::XS;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = crate::tokens::XS;
        ui.checkbox(&mut column.selected, "")
            .on_hover_text("Import this column");
        ui.add_enabled(
            column.selected,
            crate::tokens::input(
                egui::TextEdit::singleline(&mut column.name)
                    .char_limit(63)
                    .id_salt(("import_tool_name", index)),
            )
            .desired_width(ui.available_width()),
        )
        .on_hover_text(format!(
            "Detected as {} ({})",
            column.detected, column.detected_type
        ));
    });
    ui.add_enabled_ui(column.selected, |ui| {
        egui::ComboBox::from_id_salt(("import_tool_type", index))
            .width(width)
            .selected_text(column.kind.as_str())
            .show_ui(ui, |ui| {
                let detected = column.detected_type.clone();
                let extra = (!TYPES.contains(&detected.as_str())).then_some(detected);
                for kind in TYPES.iter().map(|t| (*t).to_owned()).chain(extra) {
                    let label = kind.clone();
                    ui.selectable_value(&mut column.kind, kind, label);
                }
            });
        if column.kind == "datetime" {
            ui.add(
                crate::tokens::input(
                    egui::TextEdit::singleline(&mut column.format)
                        .char_limit(40)
                        .hint_text("Input format (auto)")
                        .id_salt(("import_tool_format", index)),
                )
                .desired_width(width),
            )
            .on_hover_text("For example dd/MM/yyyy HH:mm:ss");
        }
    });
}

/// File size in B, KB, MB or GB.
pub fn size_text(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024. && unit + 1 < UNITS.len() {
        value /= 1024.;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// A delimiter as words.
fn describe(delimiter: &str) -> String {
    match delimiter {
        "," => "comma".into(),
        "\t" | "\\t" => "tab".into(),
        ";" => "semicolon".into(),
        " " => "space".into(),
        "|" => "bar".into(),
        other => format!("'{other}'"),
    }
}

impl App for ImportTool {
    fn validate_idle_inputs(&self, _ws: &Snapshot) -> Result<(), String> {
        if self.name_edited && self.output() != Output::Columns {
            Self::check_output_name(self.name.trim())?;
        }
        Ok(())
    }
    fn idle_hint(&self) -> Option<&'static str> {
        self.source.is_none().then_some("Choose a file to import")
    }
    fn kind(&self) -> AppKind {
        AppKind::ImportTool
    }
    fn export_label(&self) -> &'static str {
        "Import"
    }
    fn script(&self, ws: &Snapshot) -> Result<String, String> {
        let plan = self.plan(ws)?;
        let Plan {
            output,
            file,
            opts,
            names,
            table,
            row_times,
        } = &plan;
        let mut script = format!(
            "{opts} = detectImportOptions({file}{});\n",
            self.detect_arguments()?
        );
        script.push_str(&self.option_lines(opts, *output)?);
        let name = Self::primary(&plan);
        match output {
            Output::Table => script.push_str(&format!("{name} = readtable({file}, {opts});\n")),
            Output::Timetable if row_times.is_empty() => {
                script.push_str(&format!("{name} = readtimetable({file}, {opts});\n"));
            }
            Output::Timetable => script.push_str(&format!(
                "{name} = readtimetable({file}, {opts}, 'RowTimes', {});\n",
                quote(row_times)
            )),
            Output::Matrix => script.push_str(&format!("{name} = readmatrix({file}, {opts});\n")),
            Output::Cells => script.push_str(&format!("{name} = readcell({file}, {opts});\n")),
            Output::Strings => script.push_str(&format!(
                "{name} = table2array(readtable({file}, {opts}));\n"
            )),
            Output::Columns => {
                script.push_str(&format!("{table} = readtable({file}, {opts});\n"));
                for (variable, column) in names {
                    script.push_str(&format!("{variable} = {table}.{column};\n"));
                }
            }
        }
        script.push_str(&format!("clear {opts}"));
        if !table.is_empty() {
            script.push_str(&format!(" {table}"));
        }
        script.push('\n');
        Ok(script)
    }
    fn inputs(&self) -> Vec<String> {
        Vec::new()
    }
    fn probe(&self, ws: &Snapshot) -> (String, Vec<String>) {
        let (Ok(plan), Ok(arguments)) = (self.plan(ws), self.detect_arguments()) else {
            return (String::new(), Vec::new());
        };
        let Ok(options) = self.option_lines("app_o", plan.output) else {
            return (String::new(), Vec::new());
        };
        let file = &plan.file;
        let sheet = self.sheet_file();
        // The output first, before any probe name can shadow it.
        let primary = Self::primary(&plan);
        let mut code = if primary.is_empty() {
            // Column vectors before the columns are known: nothing yet.
            "app_size = [0 0];\napp_class = \"\";\n".to_owned()
        } else if plan.output == Output::Columns {
            format!(
                "app_size = [numel({primary}), {}];\napp_class = \"column vectors\";\n",
                plan.names.len()
            )
        } else {
            format!("app_size = size({primary});\napp_class = string(class({primary}));\n")
        };
        code.push_str(&format!(
            "app_d = detectImportOptions({file}{arguments});\n\
             app_dnames = string(app_d.VariableNames);\n\
             app_dtypes = string(app_d.VariableTypes);\n"
        ));
        if sheet {
            code.push_str(
                "app_drange = string(app_d.DataRange);\n\
                 app_dnames_range = string(app_d.VariableNamesRange);\n",
            );
        } else {
            code.push_str(
                "app_dlines = app_d.DataLines;\n\
                 app_dnames_line = app_d.VariableNamesLine;\n\
                 app_ddelim = string(app_d.Delimiter);\n",
            );
        }
        // The same options on a private copy; every column, filled, first rows.
        code.push_str("app_o = app_d;\n");
        code.push_str(&options);
        code.push_str(
            "app_o.SelectedVariableNames = app_o.VariableNames;\n\
             app_o.MissingRule = 'fill';\n\
             app_o.ImportErrorRule = 'fill';\n",
        );
        let rows = PREVIEW_ROWS;
        if sheet {
            code.push_str(&format!(
                "app_T = readtable({file}, app_o);\n\
                 app_T = app_T(1:min({rows}, height(app_T)), :);\n"
            ));
        } else {
            code.push_str(&format!(
                "app_l = app_o.DataLines;\n\
                 app_o.DataLines = [app_l(1, 1), min(app_l(1, 2), app_l(1, 1) + {})];\n\
                 app_T = readtable({file}, app_o);\n",
                rows - 1
            ));
        }
        code.push_str(
            "app_text = cell(1, width(app_T));\n\
             for app_k = 1:width(app_T)\n\
             app_text{app_k} = string(app_T{:, app_k});\n\
             end\n\
             app_miss = ismissing(app_T);\n",
        );
        let names = PROBE_COMMON
            .iter()
            .chain(if sheet {
                PROBE_SHEET.as_slice()
            } else {
                PROBE_TEXT.as_slice()
            })
            .map(|s| (*s).to_owned())
            .collect();
        (code, names)
    }
    fn prepare(&mut self, _ws: &Snapshot) {
        let identity = self.identity();
        if self.identity.as_ref() != Some(&identity) {
            self.identity = Some(identity);
            self.columns = None;
            self.detected = None;
            self.preview = None;
            self.created = None;
        }
    }
    fn receive(&mut self, outputs: &Outputs) -> Result<(), String> {
        let names = strings(outputs, "app_dnames")?;
        let types = strings(outputs, "app_dtypes")?;
        if names.len() != types.len() {
            return Err("Import Tool: detected names and types differ in number".into());
        }
        let same = self.columns.as_ref().is_some_and(|columns| {
            columns.len() == names.len()
                && columns
                    .iter()
                    .zip(names.iter().zip(&types))
                    .all(|(c, (n, t))| c.detected == *n && c.detected_type == *t)
        });
        if !same {
            self.columns = Some(
                names
                    .iter()
                    .zip(&types)
                    .map(|(n, t)| Column::new(n.clone(), t.clone()))
                    .collect(),
            );
        }
        let numeric_types = types.iter().all(|t| {
            matches!(
                t.as_str(),
                "double"
                    | "single"
                    | "int8"
                    | "int16"
                    | "int32"
                    | "int64"
                    | "uint8"
                    | "uint16"
                    | "uint32"
                    | "uint64"
            )
        });
        let detected = if self.sheet_file() {
            let range = strings(outputs, "app_drange")?.concat();
            let names_range = strings(outputs, "app_dnames_range")?.concat();
            Detected {
                first: range_row(&range).max(1),
                last: range
                    .split_once(':')
                    .map(|(_, end)| range_row(end))
                    .filter(|r| *r > 0),
                names_line: range_row(&names_range),
                delimiter: Vec::new(),
                start_column: range_column(&range),
                numeric: numeric_types && range_row(&names_range) == 0,
            }
        } else {
            let lines = numbers(outputs, "app_dlines")?;
            let names_line = numbers(outputs, "app_dnames_line")?
                .first()
                .copied()
                .unwrap_or(0.);
            Detected {
                first: lines.first().copied().unwrap_or(1.).max(1.) as usize,
                last: lines
                    .get(lines.len() / 2..)
                    .and_then(|l| l.first())
                    .copied()
                    .filter(|l| l.is_finite())
                    .map(|l| l as usize),
                names_line: names_line.max(0.) as usize,
                delimiter: strings(outputs, "app_ddelim")?,
                start_column: String::new(),
                numeric: numeric_types && names_line == 0.,
            }
        };
        self.detected = Some(detected);
        let text = match outputs.get("app_text") {
            Some(matrust::Array::Cell(cells)) => (0..cells.len())
                .map(|k| match cells.get_linear(k).map(|a| a.as_ref()) {
                    Ok(matrust::Array::Str(column)) => Ok(column.clone()),
                    _ => Err("Import Tool: a preview column is not a string array".to_owned()),
                })
                .collect::<Result<Vec<_>, String>>()?,
            _ => return Err("Import Tool: the preview is not a cell array".into()),
        };
        let (rows, cols) = (text.first().map_or(0, |c| c.len()), text.len());
        let missing = match outputs.get("app_miss") {
            Some(matrust::Array::Logical(m)) if m.rows() == rows && m.cols() == cols => {
                m.data().to_vec()
            }
            _ => vec![0; rows * cols],
        };
        self.preview = Some(Preview {
            rows,
            cols,
            text,
            missing,
        });
        self.created = Some(Created {
            size: numbers(outputs, "app_size")?
                .into_iter()
                .map(|v| v.max(0.) as usize)
                .collect(),
            class: strings(outputs, "app_class")?.concat(),
        });
        Ok(())
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, _ws: &Snapshot) {
        field(ui, "File", |ui| self.picker_row(ui));
        if self.source.is_none() {
            return;
        }
        if self.sheet_file() {
            self.sheet_ui(ui);
        } else {
            self.text_settings_ui(ui);
        }
        ui.separator();
        self.range_ui(ui);
        self.rule_ui(ui);
        ui.separator();
        self.output_ui(ui);
    }
    fn main_caption(&self) -> &'static str {
        "PREVIEW"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        if self.source.is_none() {
            crate::tokens::helper(ui, "Choose a text file or an .xlsx workbook under INPUTS.");
            return;
        }
        let Some(columns) = &self.columns else {
            crate::tokens::helper(ui, "No preview yet: the status line says why.");
            return;
        };
        let missing = self
            .preview
            .as_ref()
            .map_or(0, |p| p.missing_count(columns));
        let mut text = format!(
            "First {} rows as the engine reads them. Shaded: becomes missing ({missing}). \
             Grey: not imported.",
            PREVIEW_ROWS
        );
        if self.omit_rows {
            text.push_str(" Red row numbers: omitted.");
        }
        crate::tokens::helper(ui, text);
        ui.add_space(crate::tokens::S);
        self.grid(ui);
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        let output = self.output();
        let file = self.source.as_ref().map_or("—".to_owned(), Source::name);
        let mut rows = vec![("File", file), ("Output", output.label().to_owned())];
        match &self.created {
            Some(created) => {
                let size: Vec<String> = created.size.iter().map(|n| n.to_string()).collect();
                if output == Output::Columns {
                    rows.push(("Variables", size.get(1).cloned().unwrap_or_default()));
                    rows.push(("Length", size.first().cloned().unwrap_or_default()));
                } else {
                    rows.push(("Size", size.join(" × ")));
                    rows.push(("Class", created.class.clone()));
                }
            }
            None => rows.push(("Size", "—".to_owned())),
        }
        rows
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        let p = crate::tokens::palette(ui.ctx());
        if self.output() == Output::Columns {
            ui.label("Variables");
            match self.plan(ws) {
                Ok(plan) => {
                    for (variable, column) in plan.names.iter().take(LISTED) {
                        let text = if variable == column {
                            variable.clone()
                        } else {
                            format!("{variable}  (column {column} is a workspace name)")
                        };
                        ui.label(egui::RichText::new(text).color(p.text));
                    }
                    if plan.names.len() > LISTED {
                        crate::tokens::helper(
                            ui,
                            format!("… and {} more", plan.names.len() - LISTED),
                        );
                    }
                }
                Err(_) => {
                    crate::tokens::helper(ui, "Named after the columns");
                }
            }
            crate::tokens::helper(ui, "Rename columns in the preview header");
        } else {
            if !self.name_edited {
                self.name = self.default_name(ws);
            }
            let before = self.name.clone();
            name_field(ui, "Variable name", &mut self.name, ws);
            self.name_edited |= self.name != before;
        }
        let columns = self.result_columns();
        if columns.is_empty()
            || matches!(
                self.output(),
                Output::Matrix | Output::Cells | Output::Strings
            )
        {
            return;
        }
        ui.add_space(crate::tokens::S);
        ui.label("Column types");
        egui::Grid::new("import_tool_column_types")
            .num_columns(2)
            .spacing(egui::vec2(crate::tokens::S, crate::tokens::XS))
            .show(ui, |ui| {
                for (name, kind) in columns.iter().take(LISTED) {
                    ui.label(egui::RichText::new(name).color(p.text));
                    ui.label(egui::RichText::new(kind).color(p.text_muted));
                    ui.end_row();
                }
            });
        if columns.len() > LISTED {
            crate::tokens::helper(ui, format!("… and {} more", columns.len() - LISTED));
        }
    }
    fn exported(&self, ws: &Snapshot) -> Vec<String> {
        self.plan(ws)
            .map(|plan| plan.names.into_iter().map(|(n, _)| n).collect())
            .unwrap_or_default()
    }
    fn summary(&self) -> Option<String> {
        let created = self.created.as_ref()?;
        let source = self.source.as_ref()?;
        let size: Vec<String> = created.size.iter().map(|n| n.to_string()).collect();
        Some(format!(
            "{} → {} {}",
            source.name(),
            self.output().label().to_lowercase(),
            size.join("×")
        ))
    }
}

// ---------------------------------------------------------------------------
// Desktop glue: every way to import data opens this tool on the file.

impl crate::Workspace {
    /// Opens the Import Tool on `path`: drag and drop, Open, the folder
    /// browser and the file picker all land here.
    pub(crate) fn open_in_import_tool(&mut self, path: PathBuf) {
        let shown = crate::path_label::display(&path).to_string();
        self.toolkits.apps.import.app.choose(path);
        self.toolkits
            .open_tool(crate::toolkits::Tool::App(AppKind::ImportTool));
        self.status = format!("Import Tool: {shown}");
    }

    /// Import data (toolstrip, Workspace menu, search): ask for a file at the
    /// end of the frame; a chosen file opens the Import Tool.
    pub(crate) fn request_import_data(&mut self) {
        self.toolkits.apps.import.app.pick = true;
    }

    /// The file picker the Import Tool or Import data asked for, once per
    /// frame that is not discarded.
    pub(crate) fn finish_import_pick(&mut self, ctx: &egui::Context) {
        if ctx.will_discard() || !std::mem::take(&mut self.toolkits.apps.import.app.pick) {
            return;
        }
        if let Some(path) = self.pick_import_file() {
            self.open_in_import_tool(path);
        }
    }

    fn pick_import_file(&mut self) -> Option<PathBuf> {
        #[cfg(test)]
        {
            self.toolkits.apps.test_picks += 1;
            self.toolkits.apps.test_pick.take()
        }
        #[cfg(not(test))]
        {
            let folder = self
                .toolkits
                .apps
                .import
                .app
                .source
                .as_ref()
                .and_then(|s| s.path.parent().map(Path::to_path_buf))
                .filter(|p| p.is_dir())
                .unwrap_or_else(|| self.folder.clone());
            rfd::FileDialog::new()
                .set_directory(folder)
                .add_filter("Text and spreadsheet files", &EXTENSIONS)
                .add_filter("All files", &["*"])
                .pick_file()
        }
    }
}
