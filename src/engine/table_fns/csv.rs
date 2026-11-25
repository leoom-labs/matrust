//! readtable / writetable for delimited text. readtable takes readcell's
//! cell grid (same delimiter detection, quoting, number parsing and empty
//! fields) and turns its columns into variables; writetable builds the cell
//! grid writecell writes (same number text and quoting rules).
use super::super::{Engine, Value};
use super::{text_scalar, tvalue};
use crate::table_array::{
    DEFAULT_DIMENSION_NAMES, MAX_NAME, RESERVED_NAME, TableArray, TableParts,
};
use crate::{Array, CellArray, CharacterRow, Matrix, Result, StringArray};
use std::sync::Arc;

/// Number text of a header or text cell that readcell parsed as a number.
fn number_text(x: f64) -> String {
    if x.is_nan() {
        "NaN".into()
    } else if x.is_infinite() {
        if x > 0. { "Inf".into() } else { "-Inf".into() }
    } else {
        format!("{x}")
    }
}

/// Text of one readcell grid cell; None for an empty field.
fn cell_text(a: &Array) -> Option<String> {
    match a {
        Array::Character(c) => Some(String::from_utf16_lossy(c.units())),
        Array::Numeric(m) if m.len() == 1 => Some(number_text(m.data()[0])),
        Array::Str(s) => s
            .data()
            .first()
            .and_then(|t| t.as_deref().map(str::to_owned)),
        _ => None,
    }
}

/// Header texts made valid and unique: whitespace removed (the next letter
/// capitalized), other invalid characters '_', a leading non-letter gets an
/// 'x' prefix, an empty header is Var<k>, later duplicates get _1, _2, ...
/// A header equal to 'Properties' or a default dimension name ('Row',
/// 'Variables') is renamed the same way ('Row' -> 'Row_1').
pub(in crate::engine) fn variable_names(headers: Vec<Option<String>>) -> Vec<String> {
    let mut names: Vec<String> = Vec::with_capacity(headers.len());
    for (k, header) in headers.into_iter().enumerate() {
        let base = match header.filter(|h| !h.trim().is_empty()) {
            Some(h) => super::super::readers::rdx_make_valid_name(&h),
            None => format!("Var{}", k + 1),
        };
        let mut candidate = base.clone();
        let mut n = 1;
        while names.contains(&candidate)
            || candidate == RESERVED_NAME
            || DEFAULT_DIMENSION_NAMES.contains(&candidate.as_str())
        {
            let suffix = format!("_{n}");
            let mut stem = base.clone();
            stem.truncate(MAX_NAME - suffix.len());
            candidate = format!("{stem}{suffix}");
            n += 1;
        }
        names.push(candidate);
    }
    names
}

fn flag(
    who: &str,
    option: &str,
    v: &Value,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<bool> {
    if let Some(t) = text_scalar(v) {
        return match t.to_ascii_lowercase().as_str() {
            "true" | "on" => Ok(true),
            "false" | "off" => Ok(false),
            _ => Err(format!("{who}: {option} must be true or false")),
        };
    }
    let m = v.clone().matrix(cancel)?;
    if m.len() != 1 {
        return Err(format!("{who}: {option} must be true or false"));
    }
    Ok(m.data()[0] != 0.)
}

impl Engine {
    pub(in crate::engine) fn tbl_readtable(&mut self, args: &[Value]) -> Result<Value> {
        let [file, rest @ ..] = args else {
            return Err("Not enough input arguments.".into());
        };
        // Import options, spreadsheets and the options this reader lacks.
        if crate::engine::import_options::readtable_takes_over(self, args) {
            return crate::engine::import_options::readtable(self, args);
        }
        let filename = text_scalar(file).ok_or("readtable: the file name must be text")?;
        if let Some((_, ext)) = filename.rsplit_once('.')
            && ["xlsx", "xlsm", "xlsb", "xls"].contains(&ext.to_ascii_lowercase().as_str())
        {
            return Err(
                "readtable: spreadsheet files are not supported yet; read a .csv or .txt file"
                    .into(),
            );
        }
        if rest.len() % 2 != 0 {
            return Err("readtable: options must be name-value pairs".into());
        }
        let mut cell_args = vec![file.clone()];
        let mut read_names = true;
        let mut strings = false;
        for pair in rest.chunks(2) {
            let option = text_scalar(&pair[0]).ok_or("readtable: option names must be text")?;
            match option.to_ascii_lowercase().as_str() {
                "delimiter" | "numheaderlines" => {
                    cell_args.push(pair[0].clone());
                    cell_args.push(pair[1].clone());
                }
                "readvariablenames" => {
                    read_names = flag("readtable", "ReadVariableNames", &pair[1], &self.cancel)?;
                }
                "texttype" => {
                    strings = match text_scalar(&pair[1])
                        .map(|t| t.to_ascii_lowercase())
                        .as_deref()
                    {
                        Some("char") => false,
                        Some("string") => true,
                        _ => return Err("readtable: TextType must be 'char' or 'string'".into()),
                    };
                }
                _ => {
                    return Err(format!(
                        "readtable: unsupported option '{option}'; supported are Delimiter, ReadVariableNames, NumHeaderLines and TextType"
                    ));
                }
            }
        }
        let grid = self
            .rdx_readcell(&cell_args)
            .map_err(|e| e.replacen("readcell", "readtable", 1))?;
        let Value::Container(Array::Cell(grid)) = grid else {
            return Err("readtable: unexpected reader result".into());
        };
        let (rows, cols) = (grid.rows(), grid.cols());
        if rows == 0 || cols == 0 {
            return Ok(tvalue(TableArray::empty()));
        }
        let at = |r: usize, c: usize| grid.get_linear(c * rows + r);
        let first = usize::from(read_names);
        let names = if read_names {
            variable_names(
                (0..cols)
                    .map(|c| at(0, c).ok().and_then(|a| cell_text(a)))
                    .collect(),
            )
        } else {
            (1..=cols).map(|k| format!("Var{k}")).collect()
        };
        let height = rows - first;
        let mut columns = Vec::with_capacity(cols);
        for c in 0..cols {
            self.check_cancel()?;
            let items = (first..rows)
                .map(|r| at(r, c))
                .collect::<Result<Vec<_>>>()?;
            let numeric = items.iter().all(|a| match a.as_ref() {
                Array::Numeric(m) => m.len() == 1,
                Array::Str(_) => true,
                _ => false,
            });
            columns.push(if numeric {
                let data = items
                    .iter()
                    .map(|a| a.as_numeric().map_or(f64::NAN, |m| m.data()[0]))
                    .collect();
                Matrix::from_column_vec(height, 1, data)?.into()
            } else if strings {
                let data = items.iter().map(|a| cell_text(a).map(Arc::from)).collect();
                Array::Str(StringArray::new(height, 1, data)?)
            } else {
                let mut slots = Vec::with_capacity(height);
                for a in &items {
                    let text = cell_text(a).unwrap_or_default();
                    slots.push(Arc::new(Array::from(if text.is_empty() {
                        CharacterRow::from_column_major(0, 0, Vec::new(), &self.cancel)?
                    } else {
                        CharacterRow::from_text(&text, &self.cancel)?
                    })));
                }
                CellArray::from_shared(height, 1, slots, &self.cancel)?.into()
            });
        }
        Ok(tvalue(TableArray::new(TableParts::new(
            names, columns, height,
        ))?))
    }

    pub(in crate::engine) fn tbl_writetable(&mut self, args: &[Value]) -> Result<()> {
        let [table, rest @ ..] = args else {
            return Err("Not enough input arguments.".into());
        };
        let Value::Container(Array::Table(t)) = table else {
            return Err("writetable: the first input must be a table".into());
        };
        let t = t.clone();
        let Some((file, options)) = rest.split_first() else {
            return Err("writetable: give a file name, as in writetable(T, 'data.csv')".into());
        };
        if options.len() % 2 != 0 {
            return Err("writetable: options must be name-value pairs".into());
        }
        let mut cell_args = vec![Value::Void, file.clone()];
        let mut write_names = true;
        let mut write_rows = false;
        // A spreadsheet file name writes the same grid through xlswrite.
        let spreadsheet = text_scalar(file)
            .and_then(|f| f.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()))
            .is_some_and(|e| ["xlsx", "xlsm", "xlsb", "xls"].contains(&e.as_str()));
        let mut sheet = Value::Text(String::new());
        let mut range = Value::Text(String::new());
        for pair in options.chunks(2) {
            let option = text_scalar(&pair[0]).ok_or("writetable: option names must be text")?;
            match option.to_ascii_lowercase().as_str() {
                "delimiter" | "quotestrings" if spreadsheet => {
                    return Err(format!("writetable: {option} applies to text files only"));
                }
                "delimiter" | "quotestrings" => {
                    cell_args.push(pair[0].clone());
                    cell_args.push(pair[1].clone());
                }
                "sheet" if spreadsheet => sheet = pair[1].clone(),
                "range" if spreadsheet => range = pair[1].clone(),
                "writevariablenames" => {
                    write_names = flag("writetable", "WriteVariableNames", &pair[1], &self.cancel)?;
                }
                "writerownames" => {
                    write_rows = flag("writetable", "WriteRowNames", &pair[1], &self.cancel)?;
                }
                _ => {
                    return Err(format!(
                        "writetable: unsupported option '{option}'; supported are Delimiter, WriteVariableNames, WriteRowNames and QuoteStrings (and Sheet and Range for spreadsheets)"
                    ));
                }
            }
        }
        let row_names = if write_rows { t.row_names() } else { None };
        let h = t.height();
        let text = |s: &str, e: &Engine| -> Result<Arc<Array>> {
            Ok(Arc::new(CharacterRow::from_text(s, &e.cancel)?.into()))
        };
        // Columns of the output grid, each `header + h` cells.
        let mut grid: Vec<Vec<Arc<Array>>> = Vec::new();
        if let Some(names) = row_names {
            let mut col = vec![text(&t.parts().dimension_names[0], self)?];
            for n in names {
                col.push(text(n, self)?);
            }
            grid.push(col);
        }
        for (name, column) in t.names().iter().zip(t.columns()) {
            self.check_cancel()?;
            let width = column.cols();
            let texts = match column {
                Array::Datetime(_) | Array::Duration(_) => {
                    Some(super::super::datetime_fns::display::element_texts(
                        column,
                        None,
                        Some(&self.cancel),
                    )?)
                }
                _ => None,
            };
            // A char matrix writes one text per row.
            let out_cols = if matches!(column, Array::Character(_)) {
                1
            } else {
                width
            };
            for c in 0..out_cols {
                let header = if out_cols == 1 {
                    name.clone()
                } else {
                    format!("{name}_{}", c + 1)
                };
                let mut cells = vec![text(&header, self)?];
                for r in 0..h {
                    let i = c * h + r;
                    cells.push(match column {
                        Array::Numeric(m) => Arc::new(Matrix::scalar(m.data()[i]).into()),
                        Array::Logical(m) => {
                            Arc::new(Matrix::scalar(f64::from(m.data()[i])).into())
                        }
                        Array::Typed(a) => Arc::new(Array::Typed(a.select(1, 1, &[i])?)),
                        Array::Str(s) => Arc::new(Array::Str(StringArray::new(
                            1,
                            1,
                            vec![s.data()[i].clone()],
                        )?)),
                        Array::Cell(cell) => cell.get_linear(i)?.clone(),
                        Array::Character(ch) => {
                            let row: Vec<u16> = (0..width).map(|k| ch.units()[k * h + r]).collect();
                            text(String::from_utf16_lossy(&row).trim_end(), self)?
                        }
                        Array::Datetime(_) | Array::Duration(_) => {
                            text(&texts.as_ref().expect("time texts")[i], self)?
                        }
                        other => {
                            return Err(format!(
                                "writetable: variables of class {} cannot be written to text yet",
                                other.class()
                            ));
                        }
                    });
                }
                grid.push(cells);
            }
        }
        let skip = usize::from(!write_names);
        let rows = h + 1 - skip;
        let cols = grid.len();
        if rows.saturating_mul(cols) > crate::script_containers::MAX_CELL_SLOTS {
            return Err("writetable: tables of more than 65536 cells cannot be written yet".into());
        }
        let mut slots = Vec::with_capacity(rows * cols);
        for col in grid {
            slots.extend(col.into_iter().skip(skip));
        }
        cell_args[0] = Value::Container(Array::Cell(CellArray::from_shared(
            rows,
            cols,
            slots,
            &self.cancel,
        )?));
        if spreadsheet {
            let grid = cell_args.swap_remove(0);
            return self
                .rdx_write_sheet(&[file.clone(), grid, sheet, range])
                .map_err(|e| e.replace("xlswrite", "writetable"));
        }
        self.rdx_writecell(&cell_args)
            .map_err(|e| e.replace("writecell", "writetable"))
    }
}
