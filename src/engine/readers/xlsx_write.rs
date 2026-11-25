//! xlswrite: writes .xlsx workbooks (Office Open XML SpreadsheetML) with the
//! zip crate. An existing .xlsx keeps its other sheets and the cells outside
//! the written area as values (formatting, formulas and charts are not kept).
//! Behaviour follows the published MathWorks xlswrite page (behaviour only);
//! differences are listed on the help page.
use super::{
    CellRange, MAX_GRID_CELLS, MAX_SHEET_COLS, MAX_SHEET_ROWS, SheetCell, SheetChoice,
    rdx_extension, rdx_range, rdx_sheet_choice, rdx_text,
};
use crate::{Array, Result};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};

/// Longest text one cell holds (UTF-16 units), as in Excel.
const MAX_CELL_TEXT: usize = 32_767;
/// Text held by all cells of a workbook (bytes), and the estimated size of
/// the worksheet XML built in memory.
const MAX_TEXT_BYTES: usize = 256 * 1024 * 1024;
const MAX_XML_BYTES: usize = 1024 * 1024 * 1024;
/// Longest sheet name, as in Excel.
const MAX_SHEET_NAME: usize = 31;

/// One worksheet: its name and its non-empty cells keyed by zero-based (row, column).
pub(super) struct Sheet {
    pub(super) name: String,
    pub(super) cells: BTreeMap<(u32, u32), SheetCell>,
}

/// The cells of `value` as a row-major grid, for xlswrite.
fn xw_grid(value: &Array, cancel: &AtomicBool) -> Result<(usize, usize, Vec<SheetCell>)> {
    let dims = value.dims();
    if dims.len() > 2 {
        return Err("xlswrite: the data must be two-dimensional".into());
    }
    let (rows, cols) = (dims[0], dims.get(1).copied().unwrap_or(1));
    let number = |x: f64| {
        if x.is_finite() {
            SheetCell::Number(x)
        } else {
            SheetCell::Empty
        }
    };
    let text = |t: &str| -> Result<SheetCell> {
        if t.encode_utf16().count() > MAX_CELL_TEXT {
            return Err(format!(
                "xlswrite: a cell can hold at most {MAX_CELL_TEXT} characters"
            ));
        }
        Ok(if t.is_empty() {
            SheetCell::Empty
        } else {
            SheetCell::Text(t.to_owned())
        })
    };
    let mut column_major = Vec::new();
    match value {
        // A character vector goes to one cell.
        Array::Character(ch) if ch.rows() <= 1 && !ch.is_nd() => {
            let t = String::from_utf16_lossy(ch.units());
            return Ok(if t.is_empty() {
                (0, 0, Vec::new())
            } else {
                (1, 1, vec![text(&t)?])
            });
        }
        Array::Numeric(m) => column_major.extend(m.data().iter().map(|&x| number(x))),
        Array::Logical(m) => column_major.extend(m.data().iter().map(|&b| SheetCell::Bool(b != 0))),
        Array::Typed(t) => {
            for i in 0..t.len() {
                column_major.push(number(t.real(i)));
            }
        }
        Array::Str(s) => {
            for item in s.data() {
                column_major.push(match item {
                    Some(t) => text(t)?,
                    None => SheetCell::Empty,
                });
            }
        }
        Array::Cell(cell) => {
            for i in 0..cell.len() {
                if i.is_multiple_of(65536) {
                    super::rdx_check(cancel)?;
                }
                let item = cell.get_linear(i)?;
                column_major.push(match &**item {
                    _ if item.is_empty() => SheetCell::Empty,
                    Array::Numeric(m) if m.len() == 1 => number(m.data()[0]),
                    Array::Logical(m) if m.len() == 1 => SheetCell::Bool(m.data()[0] != 0),
                    Array::Typed(t) if t.len() == 1 => number(t.real(0)),
                    Array::Character(ch) if ch.rows() <= 1 && !ch.is_nd() => {
                        text(&String::from_utf16_lossy(ch.units()))?
                    }
                    Array::Str(s) if s.len() == 1 => match &s.data()[0] {
                        Some(t) => text(t)?,
                        None => SheetCell::Empty,
                    },
                    _ => {
                        return Err(format!(
                            "xlswrite: cell ({},{}) must hold a real scalar, text or an empty value",
                            i % rows.max(1) + 1,
                            i / rows.max(1) + 1
                        ));
                    }
                });
            }
        }
        Array::Complex(_) => return Err("xlswrite: complex data cannot be written".into()),
        _ => {
            return Err(
                "xlswrite: the data must be a numeric, logical, character, string or cell array"
                    .into(),
            );
        }
    }
    let mut grid = Vec::with_capacity(column_major.len());
    for r in 0..rows {
        for c in 0..cols {
            grid.push(column_major[c * rows + r].clone());
        }
    }
    Ok((rows, cols, grid))
}

fn xw_sheet_name_check(name: &str) -> Result<()> {
    if name.is_empty()
        || name.chars().count() > MAX_SHEET_NAME
        || name.contains(['[', ']', ':', '*', '?', '/', '\\'])
        || name.starts_with('\'')
        || name.ends_with('\'')
    {
        return Err(format!(
            "xlswrite: '{name}' is not a valid sheet name (1 to {MAX_SHEET_NAME} characters, none of [ ] : * ? / \\)"
        ));
    }
    Ok(())
}

/// Index into `sheets` of the sheet `choice` names; missing sheets are added
/// (a name, or `SheetN` names up to an index).
fn xw_target(sheets: &mut Vec<Sheet>, choice: &SheetChoice) -> Result<usize> {
    let add = |sheets: &mut Vec<Sheet>, name: String| {
        sheets.push(Sheet {
            name,
            cells: BTreeMap::new(),
        });
        sheets.len() - 1
    };
    let fresh = |sheets: &Vec<Sheet>| {
        (1..)
            .map(|k| format!("Sheet{k}"))
            .find(|n| !sheets.iter().any(|s| s.name.eq_ignore_ascii_case(n)))
            .unwrap_or_default()
    };
    match choice {
        SheetChoice::First if sheets.is_empty() => Ok(add(sheets, "Sheet1".into())),
        SheetChoice::First => Ok(0),
        SheetChoice::Name(name) => {
            if let Some(i) = sheets.iter().position(|s| s.name == *name) {
                return Ok(i);
            }
            if let Some(i) = sheets
                .iter()
                .position(|s| s.name.eq_ignore_ascii_case(name))
            {
                return Ok(i);
            }
            xw_sheet_name_check(name)?;
            Ok(add(sheets, name.clone()))
        }
        SheetChoice::Index(i) => {
            if *i > 255 {
                return Err("xlswrite: the sheet index must be at most 255".into());
            }
            while sheets.len() < *i {
                let name = fresh(sheets);
                add(sheets, name);
            }
            Ok(i - 1)
        }
    }
}

fn xw_escape(text: &str, attribute: bool, out: &mut String) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attribute => out.push_str("&quot;"),
            '\t' | '\n' | '\r' => out.push(ch),
            // Characters XML 1.0 cannot hold use Excel's _xHHHH_ form.
            c if (c as u32) < 0x20 || c == '\u{FFFE}' || c == '\u{FFFF}' => {
                out.push_str(&format!("_x{:04X}_", c as u32));
            }
            c => out.push(c),
        }
    }
}

fn xw_number(x: f64) -> String {
    let a = x.abs();
    if a == 0. || (1e-5..1e15).contains(&a) {
        format!("{x}")
    } else {
        format!("{x:e}")
    }
}

fn xw_sheet_xml(sheet: &Sheet, cancel: &AtomicBool) -> Result<String> {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">",
    );
    let bounds = sheet
        .cells
        .keys()
        .fold(None, |acc: Option<(u32, u32, u32, u32)>, &(r, c)| {
            Some(match acc {
                None => (r, c, r, c),
                Some((r0, c0, r1, c1)) => (r0.min(r), c0.min(c), r1.max(r), c1.max(c)),
            })
        });
    match bounds {
        Some((r0, c0, r1, c1)) => xml.push_str(&format!(
            "<dimension ref=\"{}:{}\"/>",
            super::rdx_cell_name(r0 as usize, c0 as usize),
            super::rdx_cell_name(r1 as usize, c1 as usize)
        )),
        None => xml.push_str("<dimension ref=\"A1\"/>"),
    }
    xml.push_str("<sheetData>");
    let mut row = None;
    for (count, (&(r, c), cell)) in sheet.cells.iter().enumerate() {
        if count.is_multiple_of(65536) {
            super::rdx_check(cancel)?;
        }
        if row != Some(r) {
            if row.is_some() {
                xml.push_str("</row>");
            }
            xml.push_str(&format!("<row r=\"{}\">", r + 1));
            row = Some(r);
        }
        let at = super::rdx_cell_name(r as usize, c as usize);
        match cell {
            SheetCell::Empty => {}
            SheetCell::Number(x) => {
                xml.push_str(&format!("<c r=\"{at}\"><v>{}</v></c>", xw_number(*x)))
            }
            SheetCell::Bool(b) => xml.push_str(&format!(
                "<c r=\"{at}\" t=\"b\"><v>{}</v></c>",
                u8::from(*b)
            )),
            SheetCell::Error => xml.push_str(&format!("<c r=\"{at}\" t=\"e\"><v>#N/A</v></c>")),
            SheetCell::Text(t) => {
                xml.push_str(&format!(
                    "<c r=\"{at}\" t=\"inlineStr\"><is><t xml:space=\"preserve\">"
                ));
                xw_escape(t, false, &mut xml);
                xml.push_str("</t></is></c>");
            }
        }
    }
    if row.is_some() {
        xml.push_str("</row>");
    }
    xml.push_str("</sheetData></worksheet>");
    Ok(xml)
}

/// The bytes of an .xlsx package holding `sheets` (at least one).
pub(super) fn xw_package(sheets: &[Sheet], cancel: &AtomicBool) -> Result<Vec<u8>> {
    use std::io::Write;
    const HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";
    const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
    const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    const PKG_REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
    let mut types = format!(
        "{HEAD}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/><Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/>"
    );
    let mut book = format!("{HEAD}<workbook xmlns=\"{MAIN}\" xmlns:r=\"{REL}\"><sheets>");
    let mut book_rels = format!("{HEAD}<Relationships xmlns=\"{PKG_REL}\">");
    for (i, sheet) in sheets.iter().enumerate() {
        let n = i + 1;
        types.push_str(&format!(
            "<Override PartName=\"/xl/worksheets/sheet{n}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>"
        ));
        book.push_str("<sheet name=\"");
        xw_escape(&sheet.name, true, &mut book);
        book.push_str(&format!("\" sheetId=\"{n}\" r:id=\"rId{n}\"/>"));
        book_rels.push_str(&format!(
            "<Relationship Id=\"rId{n}\" Type=\"{REL}/worksheet\" Target=\"worksheets/sheet{n}.xml\"/>"
        ));
    }
    types.push_str("</Types>");
    book.push_str("</sheets></workbook>");
    book_rels.push_str(&format!(
        "<Relationship Id=\"rId{}\" Type=\"{REL}/styles\" Target=\"styles.xml\"/></Relationships>",
        sheets.len() + 1
    ));
    let rels = format!(
        "{HEAD}<Relationships xmlns=\"{PKG_REL}\"><Relationship Id=\"rId1\" Type=\"{REL}/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>"
    );
    let styles = format!(
        "{HEAD}<styleSheet xmlns=\"{MAIN}\"><fonts count=\"1\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts><fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill></fills><borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs><cellXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/></cellXfs><cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles></styleSheet>"
    );
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let part = |zip: &mut zip::ZipWriter<_>, name: &str, text: &str| -> Result<()> {
        zip.start_file(name, options).map_err(|e| e.to_string())?;
        zip.write_all(text.as_bytes()).map_err(|e| e.to_string())
    };
    part(&mut zip, "[Content_Types].xml", &types)?;
    part(&mut zip, "_rels/.rels", &rels)?;
    part(&mut zip, "xl/workbook.xml", &book)?;
    part(&mut zip, "xl/_rels/workbook.xml.rels", &book_rels)?;
    part(&mut zip, "xl/styles.xml", &styles)?;
    for (i, sheet) in sheets.iter().enumerate() {
        let xml = xw_sheet_xml(sheet, cancel)?;
        part(&mut zip, &format!("xl/worksheets/sheet{}.xml", i + 1), &xml)?;
    }
    Ok(zip.finish().map_err(|e| e.to_string())?.into_inner())
}

/// Parsed xlswrite arguments after the data: the sheet and the range.
fn xw_place(args: &[crate::engine::Value]) -> Result<(SheetChoice, Option<CellRange>)> {
    let mut sheet = SheetChoice::First;
    let mut range = None;
    match args {
        [] => {}
        [one] => match rdx_text(one) {
            Some(t) if t.contains(':') => range = Some(rdx_range("xlswrite", &t)?),
            _ => sheet = rdx_sheet_choice("xlswrite", one)?,
        },
        [s, r] => {
            sheet = rdx_sheet_choice("xlswrite", s)?;
            let t = rdx_text(r).ok_or("xlswrite: the range must be text such as 'A1:C10'")?;
            if !t.is_empty() {
                range = Some(rdx_range("xlswrite", &t)?);
            }
        }
        _ => return Err("Too many input arguments.".into()),
    }
    Ok((sheet, range))
}

impl crate::engine::Engine {
    pub(super) fn rdx_xlswrite(&mut self, args: &[crate::engine::Value]) -> Result<()> {
        let [file, data, rest @ ..] = args else {
            return Err(
                "xlswrite: give a file name and data, as in xlswrite('book.xlsx', A)".into(),
            );
        };
        let filename = rdx_text(file).ok_or("xlswrite: the file name must be text")?;
        match rdx_extension(&filename).as_deref() {
            None | Some("xlsx") => {}
            Some(e) => {
                return Err(format!(
                    "xlswrite: only .xlsx workbooks can be written, not .{e}"
                ));
            }
        }
        let (choice, range) = xw_place(rest)?;
        let data = data.clone().array(&self.cancel)?;
        let (rows, cols, grid) = xw_grid(&data, &self.cancel)?;
        // A rectangle range clips the data; a single cell is the top-left corner.
        let (r0, c0) = range.map_or((0, 0), |r| (r.row, r.col));
        let (keep_rows, keep_cols) = match range {
            Some(CellRange {
                last_row: Some(lr),
                last_col: Some(lc),
                ..
            }) => (rows.min(lr - r0 + 1), cols.min(lc - c0 + 1)),
            _ => (rows, cols),
        };
        if r0 + keep_rows > MAX_SHEET_ROWS || c0 + keep_cols > MAX_SHEET_COLS {
            return Err(format!(
                "xlswrite: the data does not fit on a sheet ({MAX_SHEET_ROWS} rows, {MAX_SHEET_COLS} columns)"
            ));
        }
        if keep_rows.saturating_mul(keep_cols) > MAX_GRID_CELLS {
            return Err(format!(
                "xlswrite: at most {MAX_GRID_CELLS} cells can be written"
            ));
        }
        let path = self.file_path(&filename, ".xlsx")?;
        let mut sheets = if path.exists() {
            super::rdx_read_workbook(&path, MAX_GRID_CELLS, MAX_TEXT_BYTES, &self.cancel)
                .map_err(|e| format!("xlswrite: cannot update {filename}: {e}"))?
        } else {
            Vec::new()
        };
        let target = xw_target(&mut sheets, &choice)?;
        let cells = &mut sheets[target].cells;
        for r in 0..keep_rows {
            self.check_cancel()?;
            for c in 0..keep_cols {
                let at = ((r0 + r) as u32, (c0 + c) as u32);
                match &grid[r * cols + c] {
                    SheetCell::Empty => {
                        cells.remove(&at);
                    }
                    cell => {
                        cells.insert(at, cell.clone());
                    }
                }
            }
        }
        let total: usize = sheets.iter().map(|s| s.cells.len()).sum();
        if total > MAX_GRID_CELLS {
            return Err(format!(
                "xlswrite: the workbook would hold more than {MAX_GRID_CELLS} cells"
            ));
        }
        // About 40 bytes of markup per cell (escaping can grow text up to 6x).
        let text: usize = sheets
            .iter()
            .flat_map(|s| s.cells.values())
            .map(|c| match c {
                SheetCell::Text(t) => t.len(),
                _ => 0,
            })
            .sum();
        if text > MAX_TEXT_BYTES
            || total
                .saturating_mul(40)
                .saturating_add(text.saturating_mul(6))
                > MAX_XML_BYTES
        {
            return Err(format!(
                "xlswrite: the workbook would hold more than {} MiB of text and markup",
                MAX_XML_BYTES >> 20
            ));
        }
        let bytes = xw_package(&sheets, &self.cancel)?;
        crate::files::atomic_write_with_cancel(&path, &bytes, &self.cancel).map_err(|e| {
            format!(
                "xlswrite: unable to write {filename}: {}",
                e.split(" at path").next().unwrap_or("")
            )
        })
    }
}
