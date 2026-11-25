//! `table` (part 1): construction (table, array2table, cell2table,
//! struct2table), conversion back (table2array, table2cell, table2struct),
//! queries (istable, height, width, size, numel, isempty, head, tail),
//! sortrows, and the hooks for dot access, `T(rows, vars)` / `T{rows, vars}`
//! reads and writes, concatenation and readtable/writetable. Storage lives
//! in src/table_array.rs.
//!
//! Rows of a variable are always read and written through the engine's own
//! indexing and indexed assignment for that variable's type (`A(rows, :)` and
//! `A(rows, :) = B`), and variables are joined with the ordinary
//! concatenation, so a table column behaves exactly like the array it holds.
//! The hook in `Engine::call` claims a call only for one of this module's
//! names or when an argument is a table.
mod concat;
pub(super) mod csv;
pub(in crate::engine) mod groups;
mod index;
mod joins;
mod missing;
mod sort;
mod summary;
mod timetable;
mod vars;

use super::{Engine, Execution, Value};
use crate::syntax::Expr;
use crate::table_array::{TableArray, TableParts, valid_name};
use crate::{Array, CellArray, LogicalMatrix, Matrix, Result, StructArray};
use std::sync::Arc;

/// Names this module owns whatever the argument types.
const NAMES: &[&str] = &[
    "timetable",
    "table2timetable",
    "timetable2table",
    "istimetable",
    "retime",
    "synchronize",
    "table",
    "array2table",
    "cell2table",
    "struct2table",
    "table2array",
    "table2cell",
    "table2struct",
    "istable",
    "height",
    "width",
    "head",
    "tail",
    "addvars",
    "removevars",
    "renamevars",
    "movevars",
    "innerjoin",
    "outerjoin",
    "findgroups",
    "splitapply",
    "groupsummary",
    "varfun",
    "rowfun",
    "rmmissing",
    "fillmissing",
    "standardizeMissing",
    "summary",
];

/// Calls with several outputs answered here.
const MULTI: &[&str] = &[
    "innerjoin",
    "outerjoin",
    "findgroups",
    "splitapply",
    "groupsummary",
    "rowfun",
    "rmmissing",
    "fillmissing",
];

/// Other names answered here when an argument is a table; everything else
/// with a table argument goes to the existing code (class, isequal, disp, ...).
const ON_TABLES: &[&str] = &[
    "size", "numel", "isempty", "length", "ndims", "sortrows", "horzcat", "vertcat", "cat",
];

const CANCEL_STRIDE: usize = 65_536;

/// Names whose several-output calls come here first.
pub(super) fn owns_outputs(name: &str) -> bool {
    MULTI.contains(&name) || matches!(name, "join" | "summary")
}

pub(super) fn is_table(v: &Value) -> bool {
    matches!(v, Value::Container(Array::Table(_)))
}

fn table_of(v: &Value) -> Option<&TableArray> {
    match v {
        Value::Container(Array::Table(t)) => Some(t),
        _ => None,
    }
}

fn tvalue(t: TableArray) -> Value {
    Value::Container(Array::Table(t))
}

fn scalar_number(x: f64) -> Value {
    Value::Matrix(Matrix::scalar(x))
}

pub(super) fn unknown_variable(name: &str) -> String {
    format!("Unrecognized table variable name '{name}'.")
}

/// One text value: char row or string scalar.
pub(super) fn text_scalar(v: &Value) -> Option<String> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
        Value::Container(Array::Character(c)) if c.rows() <= 1 && !c.is_nd() => {
            Some(String::from_utf16_lossy(c.units()))
        }
        Value::Container(Array::Str(s)) if s.len() == 1 => {
            s.data()[0].as_deref().map(str::to_owned)
        }
        _ => None,
    }
}

/// A list of names: char row, char matrix rows, cellstr or string array
/// (missing strings are an error).
pub(super) fn text_list(engine: &Engine, v: &Value, what: &str) -> Result<Option<Vec<String>>> {
    let Some((texts, _)) = super::datetime_fns::text_elements(v, &engine.cancel)? else {
        return Ok(None);
    };
    texts
        .into_iter()
        .map(|t| t.ok_or_else(|| format!("{what} cannot be missing strings")))
        .collect::<Result<Vec<_>>>()
        .map(Some)
}

/// `Expr` selecting the zero-based `rows` as a subscript.
pub(super) fn row_expr(rows: &[usize]) -> Expr {
    let number = |r: usize| Expr::Number((r + 1) as f64);
    match rows {
        [] => Expr::Matrix(Vec::new()),
        [first, ..] if rows.iter().enumerate().all(|(i, &r)| r == first + i) => Expr::Range(
            Box::new(number(*first)),
            None,
            Box::new(number(rows[rows.len() - 1])),
        ),
        _ => Expr::Matrix(vec![rows.iter().map(|&r| number(r)).collect()]),
    }
}

/// `Expr` for the one-based column range `first..=last` (zero-based input).
fn column_range(first: usize, last: usize) -> Expr {
    Expr::Range(
        Box::new(Expr::Number((first + 1) as f64)),
        None,
        Box::new(Expr::Number((last + 1) as f64)),
    )
}

/// Name-value options at the end of `args`: the data part ends at the first
/// option name from which only `name, value` pairs of known names follow.
fn split_options<'a>(
    args: &'a [Value],
    options: &[&str],
) -> (&'a [Value], Vec<(String, &'a Value)>) {
    let is_option = |v: &Value| {
        text_scalar(v).is_some_and(|t| options.iter().any(|o| o.eq_ignore_ascii_case(&t)))
    };
    let mut start = args.len();
    let mut i = args.len();
    while i >= 2 && is_option(&args[i - 2]) {
        i -= 2;
        start = i;
    }
    let pairs = args[start..]
        .chunks(2)
        .map(|p| {
            let name = text_scalar(&p[0]).unwrap_or_default();
            let canonical = options
                .iter()
                .find(|o| o.eq_ignore_ascii_case(&name))
                .map_or(name, |o| (*o).to_owned());
            (canonical, &p[1])
        })
        .collect();
    (&args[..start], pairs)
}

/// Unique row names for a selection that repeats rows: later copies get
/// `_1`, `_2`, ...
fn unique_row_names(names: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        let mut candidate = name.clone();
        let mut k = 1;
        while !seen.insert(candidate.clone()) {
            candidate = format!("{name}_{k}");
            k += 1;
        }
        out.push(candidate);
    }
    out
}

fn cellstr_row(names: &[String], cancel: &std::sync::atomic::AtomicBool) -> Result<Array> {
    cellstr(1, names.len(), names, cancel)
}

fn cellstr(
    rows: usize,
    cols: usize,
    names: &[String],
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Array> {
    let mut slots = Vec::with_capacity(names.len());
    for name in names {
        slots.push(Arc::new(Array::from(crate::CharacterRow::from_text(
            name, cancel,
        )?)));
    }
    Ok(CellArray::from_shared(rows, cols, slots, cancel)?.into())
}

/// Class group for brace concatenation: numbers and logicals mix, other
/// classes join only with themselves.
fn concat_group(a: &Array) -> &'static str {
    match a {
        Array::Numeric(_) | Array::Logical(_) | Array::Complex(_) | Array::Typed(_) => "numeric",
        other => other.class(),
    }
}

impl Engine {
    /// Entry from `Engine::call`: None leaves the call to the existing code.
    /// `names` are the call site's argument variable names ('' for others).
    pub(super) fn tbl_call(
        &mut self,
        name: &str,
        args: &[Value],
        names: &[String],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        let owned = NAMES.contains(&name)
            || (name == "ismissing" && args.first().is_some_and(missing::claims))
            || (name == "join" && args.first().is_some_and(is_table));
        if !owned && !(ON_TABLES.contains(&name) && args.iter().any(is_table)) {
            return Ok(None);
        }
        Ok(self.tbl_run(name, args, names, 1, out)?.map(|mut v| {
            if v.is_empty() {
                Value::Void
            } else {
                v.remove(0)
            }
        }))
    }

    /// Several outputs (`[r, c] = size(T)`, `[B, i] = sortrows(T, ...)`);
    /// None when the call is not this module's.
    pub(super) fn tbl_outputs(
        &mut self,
        name: &str,
        args: &[Value],
        names: &[String],
        count: usize,
        out: &mut Execution,
    ) -> Result<Option<Vec<Value>>> {
        let mine = MULTI.contains(&name)
            || name == "summary"
            || (matches!(name, "size" | "sortrows" | "join") && args.first().is_some_and(is_table));
        if !mine {
            return Ok(None);
        }
        self.tbl_run(name, args, names, count, out)
    }

    fn tbl_run(
        &mut self,
        name: &str,
        args: &[Value],
        names: &[String],
        count: usize,
        out: &mut Execution,
    ) -> Result<Option<Vec<Value>>> {
        self.check_cancel()?;
        let max = match name {
            "size" => usize::MAX,
            "sortrows" | "rmmissing" | "fillmissing" => 2,
            "innerjoin" | "outerjoin" | "join" => 3,
            "findgroups" | "splitapply" | "groupsummary" | "rowfun" => usize::MAX,
            _ => 1,
        };
        if count > max {
            return Err(format!("{name}: too many output arguments"));
        }
        let one = |v: Value| Ok(Some(vec![v]));
        let first = || {
            args.first()
                .ok_or_else(|| "Not enough input arguments.".to_string())
        };
        match name {
            "summary" => summary::call(self, args, count, out),
            "timetable" | "table2timetable" | "timetable2table" | "istimetable" | "retime"
            | "synchronize" => one(timetable::call(self, name, args, names, out)?),
            "table" => one(self.tbl_table(args, names)?),
            "array2table" => one(self.tbl_array2table(args, names, out)?),
            "cell2table" => one(self.tbl_cell2table(args, names, out)?),
            "struct2table" => one(self.tbl_struct2table(args, out)?),
            "table2array" => {
                let t = self.tbl_one(args, name)?;
                let all: Vec<usize> = (0..t.width()).collect();
                let pieces = t.columns().to_vec();
                one(self.tbl_join_pieces(&t, &all, pieces, t.height(), out)?)
            }
            "table2cell" => one(self.tbl_table2cell(args, out)?),
            "table2struct" => one(self.tbl_table2struct(args, out)?),
            "istable" => {
                if args.len() != 1 {
                    return Err("istable: expected one input".into());
                }
                one(Value::Logical(LogicalMatrix::scalar(
                    table_of(&args[0]).is_some_and(|t| !t.is_timetable()),
                )))
            }
            "height" | "width" => {
                if args.len() != 1 {
                    return Err(format!("{name}: expected one input"));
                }
                let dims = args[0].dims();
                one(scalar_number(
                    if name == "height" { dims[0] } else { dims[1] } as f64,
                ))
            }
            "head" | "tail" => one(self.tbl_head_tail(name, args, out)?),
            "addvars" => one(self.tbl_addvars(args, names)?),
            "removevars" => one(self.tbl_removevars(args)?),
            "renamevars" => one(self.tbl_renamevars(args)?),
            "movevars" => one(self.tbl_movevars(args)?),
            "ismissing" => one(self.mis_ismissing(args)?),
            "rmmissing" => Ok(Some(self.mis_rmmissing(args, count, out)?)),
            "fillmissing" => Ok(Some(self.mis_fillmissing(args, count, out)?)),
            "standardizeMissing" => one(self.mis_standardize(args, out)?),
            "innerjoin" | "outerjoin" | "join" => {
                Ok(Some(self.tbl_keyjoin(name, args, names, count, out)?))
            }
            "findgroups" => Ok(Some(self.grp_findgroups(args, count, out)?)),
            "splitapply" => Ok(Some(self.grp_splitapply(args, count, out)?)),
            "groupsummary" => Ok(Some(self.grp_groupsummary(args, count, out)?)),
            "varfun" => one(self.grp_varfun(args, out)?),
            "rowfun" => Ok(Some(self.grp_rowfun(args, count, out)?)),
            "size" => Ok(Some(self.tbl_size(args, count)?)),
            "numel" | "length" | "ndims" | "isempty" => {
                let t = table_of(first()?).ok_or("Expected a table")?;
                if args.len() != 1 {
                    return Err(format!("{name}: expected one input for a table"));
                }
                one(match name {
                    "numel" => scalar_number(t.len() as f64),
                    "length" => scalar_number(if t.is_empty() {
                        0.
                    } else {
                        t.height().max(t.width()) as f64
                    }),
                    "ndims" => scalar_number(2.),
                    _ => Value::Logical(LogicalMatrix::scalar(t.is_empty())),
                })
            }
            "sortrows" => Ok(Some(self.tbl_sortrows(args, count, out)?)),
            "horzcat" => one(self.tbl_concat(vec![args.to_vec()], out)?),
            "vertcat" => one(self.tbl_concat(args.iter().map(|v| vec![v.clone()]).collect(), out)?),
            "cat" => {
                let dim = match args.first().map(|v| v.clone().matrix(&self.cancel)) {
                    Some(Ok(m)) if m.len() == 1 => m.data()[0],
                    _ => return Err("cat: the first input must be the dimension".into()),
                };
                match dim {
                    1. => {
                        one(self
                            .tbl_concat(args[1..].iter().map(|v| vec![v.clone()]).collect(), out)?)
                    }
                    2. => one(self.tbl_concat(vec![args[1..].to_vec()], out)?),
                    _ => Err("cat: tables concatenate along dimension 1 or 2 only".into()),
                }
            }
            _ => Ok(None),
        }
    }

    fn tbl_one(&self, args: &[Value], who: &str) -> Result<TableArray> {
        match args {
            [] => Err("Not enough input arguments.".into()),
            [v, ..] => table_of(v)
                .cloned()
                .ok_or_else(|| format!("{who}: the first input must be a table")),
        }
    }

    fn tbl_size(&self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        let t = self.tbl_one(args, "size")?;
        let dims = [t.height() as f64, t.width() as f64];
        match args.len() {
            1 => {}
            2 => {
                if count > 1 {
                    return Err("size: too many output arguments".into());
                }
                let d = args[1].clone().matrix(&self.cancel)?;
                let mut values = Vec::with_capacity(d.len());
                for &k in d.data() {
                    if !k.is_finite() || k < 1. || k.fract() != 0. {
                        return Err("size: dimensions must be positive integers".into());
                    }
                    values.push(if k <= 2. { dims[k as usize - 1] } else { 1. });
                }
                return Ok(vec![Value::Matrix(Matrix::from_column_vec(
                    1,
                    values.len(),
                    values,
                )?)]);
            }
            _ => return Err("size: too many inputs for a table".into()),
        }
        if count <= 1 {
            return Ok(vec![Value::Matrix(Matrix::from_column_vec(
                1,
                2,
                dims.to_vec(),
            )?)]);
        }
        Ok((0..count)
            .map(|i| scalar_number(dims.get(i).copied().unwrap_or(1.)))
            .collect())
    }

    // ---- shared row/variable machinery ----

    /// Rows `rows` of one variable through the engine's own `A(rows, :)`.
    pub(super) fn tbl_take_rows(
        &mut self,
        column: &Array,
        rows: &[usize],
        out: &mut Execution,
    ) -> Result<Array> {
        if rows.len() == column.rows() && rows.iter().enumerate().all(|(i, &r)| i == r) {
            return Ok(column.clone());
        }
        if matches!(column, Array::Sparse(_)) {
            return Err("table: sparse variables are not supported yet".into());
        }
        let value = self.index_value(
            Value::from(column.clone()),
            &[row_expr(rows), Expr::All],
            out,
        )?;
        value.array(&self.cancel)
    }

    /// Columns `first..=last` of `value` through `V(:, first:last)`.
    fn tbl_take_columns(
        &mut self,
        value: &Array,
        first: usize,
        last: usize,
        out: &mut Execution,
    ) -> Result<Array> {
        if first == 0 && last + 1 == value.cols() {
            return Ok(value.clone());
        }
        self.index_value(
            Value::from(value.clone()),
            &[Expr::All, column_range(first, last)],
            out,
        )?
        .array(&self.cancel)
    }

    /// `column(args) = piece` (or `= []` when `piece` is None) through the
    /// engine's indexed assignment statement on a temporary variable, so every
    /// variable type uses its own assignment rules (growth, conversion, ...).
    pub(super) fn tbl_write(
        &mut self,
        column: Array,
        args: Vec<Expr>,
        piece: Option<Array>,
        out: &mut Execution,
    ) -> Result<Array> {
        let fresh = |e: &Engine, prefix: &str| {
            (0usize..)
                .map(|n| format!("Zq_tbl_{prefix}_{n}"))
                .find(|name| !e.contains_array(name))
                .expect("unbounded candidate names")
        };
        let target = fresh(self, "target");
        self.publish_array(target.clone(), column);
        let source = fresh(self, "value");
        let rhs = match piece {
            Some(p) => {
                self.publish_array(source.clone(), p);
                Expr::Name(source.clone())
            }
            None => Expr::Matrix(Vec::new()),
        };
        let statement = crate::syntax::Evaluation {
            targets: Vec::new(),
            indexed_target: Some((target.clone(), args)),
            cell_content_target: false,
            path_target: None,
            list_target: None,
            expr: rhs,
            quiet: true,
        };
        let result = self.execute_evaluation(&statement, out);
        let updated = self.array(&target);
        self.remove_array(&target);
        self.remove_array(&source);
        result?;
        updated.ok_or_else(|| "Indexed assignment produced no value".into())
    }

    /// Concatenate values with the engine's ordinary rules (the dispatch of a
    /// `[...]` literal).
    pub(super) fn tbl_join(&mut self, rows: Vec<Vec<Value>>, out: &mut Execution) -> Result<Value> {
        if rows.iter().flatten().any(is_table) {
            return self.tbl_concat(rows, out);
        }
        if rows.iter().flatten().any(super::categorical_fns::is_cat) {
            return super::categorical_fns::concatenate(self, rows);
        }
        if rows.iter().flatten().any(super::datetime_fns::is_dt) {
            return self.dtm_concat(rows);
        }
        if rows.iter().flatten().any(super::sparse_fns::is_sparse) {
            return super::sparse_fns::concatenate(self, rows);
        }
        if rows.iter().flatten().any(Value::is_nd) {
            return super::nd::literal(self, rows);
        }
        self.cel_concatenate_warn(rows, out)
    }

    /// `T{rows, vars}`: the selected pieces side by side.
    pub(super) fn tbl_join_pieces(
        &mut self,
        t: &TableArray,
        vars: &[usize],
        pieces: Vec<Array>,
        height: usize,
        out: &mut Execution,
    ) -> Result<Value> {
        match pieces.len() {
            0 => {
                return Ok(Value::Matrix(Matrix::from_column_vec(
                    height,
                    0,
                    Vec::new(),
                )?));
            }
            1 => return Ok(Value::from(pieces.into_iter().next().expect("one piece"))),
            _ => {}
        }
        for (k, piece) in pieces.iter().enumerate().skip(1) {
            if concat_group(piece) != concat_group(&pieces[0]) {
                return Err(format!(
                    "Unable to concatenate the table variables '{}' and '{}', because their types are {} and {}.",
                    t.names()[vars[0]],
                    t.names()[vars[k]],
                    pieces[0].class(),
                    piece.class()
                ));
            }
        }
        let row = pieces.into_iter().map(Value::from).collect();
        self.tbl_join(vec![row], out)
    }

    // ---- constructors ----

    fn tbl_table(&mut self, args: &[Value], names: &[String]) -> Result<Value> {
        let (data, options) = split_options(
            args,
            &[
                "VariableNames",
                "RowNames",
                "DimensionNames",
                "Size",
                "VariableTypes",
            ],
        );
        let mut columns = Vec::with_capacity(data.len());
        for v in data {
            if matches!(v, Value::Void) {
                return Err("table: an input has no value".into());
            }
            if is_table(v) {
                return Err("table: nested tables are not supported yet".into());
            }
            columns.push(v.clone().array(&self.cancel)?);
        }
        let mut var_names: Vec<String> = (0..columns.len())
            .map(|i| match names.get(i) {
                Some(n) if valid_name(n) => n.clone(),
                _ => format!("Var{}", i + 1),
            })
            .collect();
        let mut row_names = None;
        let mut dimension_names = None;
        for (option, value) in options {
            match option.as_str() {
                "VariableNames" => {
                    let list = text_list(self, value, "VariableNames")?.ok_or(
                        "table: VariableNames must be a cell array of names or a string array",
                    )?;
                    if list.len() != columns.len() {
                        return Err("The VariableNames property must contain one name for each variable in the table.".into());
                    }
                    var_names = list;
                }
                "RowNames" => {
                    let list = text_list(self, value, "RowNames")?
                        .ok_or("table: RowNames must be a cell array of names or a string array")?;
                    row_names = if list.is_empty() || list == [""] {
                        None
                    } else {
                        Some(list)
                    };
                }
                "DimensionNames" => {
                    let list = text_list(self, value, "DimensionNames")?
                        .filter(|l| l.len() == 2)
                        .ok_or("table: DimensionNames must be two names")?;
                    dimension_names = Some([list[0].clone(), list[1].clone()]);
                }
                _ => {
                    return Err(
                        "table: the 'Size' and 'VariableTypes' options are not supported yet"
                            .into(),
                    );
                }
            }
        }
        let height = columns
            .first()
            .map(Array::rows)
            .or_else(|| row_names.as_ref().map(Vec::len))
            .unwrap_or(0);
        let mut parts = TableParts::new(var_names, columns, height);
        parts.row_names = row_names;
        if let Some(d) = dimension_names {
            parts.dimension_names = d;
        }
        Ok(tvalue(TableArray::new(parts)?))
    }

    /// Default names for array2table/cell2table: the input's variable name
    /// and the column number (`M1`, `M2`), or `Var1`, `Var2` for an
    /// expression.
    fn tbl_default_names(names: &[String], count: usize) -> Vec<String> {
        let base = names
            .first()
            .filter(|n| valid_name(n) && n.len() + count.to_string().len() <= 63)
            .cloned()
            .unwrap_or_else(|| "Var".into());
        (1..=count).map(|k| format!("{base}{k}")).collect()
    }

    /// VariableNames / RowNames options of array2table, cell2table and
    /// struct2table applied to `parts`.
    fn tbl_apply_options(
        &self,
        who: &str,
        parts: &mut TableParts,
        options: Vec<(String, &Value)>,
    ) -> Result<()> {
        for (option, value) in options {
            let list = text_list(self, value, &option)?.ok_or_else(|| {
                format!("{who}: {option} must be a cell array of names or a string array")
            })?;
            match option.as_str() {
                "VariableNames" => {
                    if list.len() != parts.names.len() {
                        return Err("The VariableNames property must contain one name for each variable in the table.".into());
                    }
                    parts.names = list;
                }
                _ => {
                    parts.row_names = if list.is_empty() || list == [""] {
                        None
                    } else {
                        Some(list)
                    };
                }
            }
        }
        Ok(())
    }

    fn tbl_array2table(
        &mut self,
        args: &[Value],
        names: &[String],
        out: &mut Execution,
    ) -> Result<Value> {
        let (data, options) = split_options(args, &["VariableNames", "RowNames"]);
        let [a] = data else {
            return Err("array2table: expected one array, then name-value options".into());
        };
        if is_table(a) {
            return Err("array2table: the input is already a table".into());
        }
        let a = a.clone().array(&self.cancel)?;
        if a.is_nd() {
            return Err("array2table: the input must be a 2-D array".into());
        }
        if matches!(a, Array::Sparse(_)) {
            return Err("table: sparse variables are not supported yet".into());
        }
        if matches!(a, Array::Struct(_) | Array::Handle(_) | Array::Graphics(_)) {
            return Err("array2table: use struct2table for struct arrays".into());
        }
        let mut columns = Vec::with_capacity(a.cols());
        for c in 0..a.cols() {
            if c % 1024 == 0 {
                self.check_cancel()?;
            }
            columns.push(self.tbl_take_columns(&a, c, c, out)?);
        }
        let mut parts =
            TableParts::new(Self::tbl_default_names(names, a.cols()), columns, a.rows());
        self.tbl_apply_options("array2table", &mut parts, options)?;
        Ok(tvalue(TableArray::new(parts)?))
    }

    /// One variable from cell contents: numeric (or logical) scalars of one
    /// class become a column of that class; anything else stays a cell column.
    fn tbl_from_contents(
        &mut self,
        contents: Vec<Arc<Array>>,
        out: &mut Execution,
    ) -> Result<Array> {
        let n = contents.len();
        let scalar_class = |a: &Array| match a {
            Array::Numeric(m) if m.len() == 1 => Some("double".to_string()),
            Array::Logical(m) if m.len() == 1 => Some("logical".into()),
            Array::Typed(t) if t.len() == 1 => Some(t.class().name().to_string()),
            _ => None,
        };
        let first = contents.first().and_then(|a| scalar_class(a));
        if let Some(class) = first
            && contents
                .iter()
                .all(|a| scalar_class(a).as_deref() == Some(class.as_str()))
        {
            if class == "double" {
                let data = contents
                    .iter()
                    .map(|a| a.as_numeric().map_or(0., |m| m.data()[0]))
                    .collect();
                return Ok(Matrix::from_column_vec(n, 1, data)?.into());
            }
            let rows = contents
                .iter()
                .map(|a| vec![Value::from(a.as_ref().clone())])
                .collect();
            return self.tbl_join(rows, out)?.array(&self.cancel);
        }
        Ok(CellArray::from_shared(n, 1, contents, &self.cancel)?.into())
    }

    fn tbl_cell2table(
        &mut self,
        args: &[Value],
        names: &[String],
        out: &mut Execution,
    ) -> Result<Value> {
        let (data, options) = split_options(args, &["VariableNames", "RowNames"]);
        let [Value::Container(Array::Cell(c))] = data else {
            return Err("cell2table: expected one cell array, then name-value options".into());
        };
        let c = c.clone();
        let (rows, cols) = (c.rows(), c.cols());
        let mut columns = Vec::with_capacity(cols);
        for j in 0..cols {
            self.check_cancel()?;
            let contents = (0..rows)
                .map(|r| c.get_linear(j * rows + r).cloned())
                .collect::<Result<Vec<_>>>()?;
            columns.push(self.tbl_from_contents(contents, out)?);
        }
        let mut parts = TableParts::new(Self::tbl_default_names(names, cols), columns, rows);
        self.tbl_apply_options("cell2table", &mut parts, options)?;
        Ok(tvalue(TableArray::new(parts)?))
    }

    fn tbl_struct2table(&mut self, args: &[Value], out: &mut Execution) -> Result<Value> {
        let (data, options) = split_options(args, &["VariableNames", "RowNames"]);
        let [Value::Container(Array::Struct(s))] = data else {
            return Err("struct2table: expected one struct array, then name-value options".into());
        };
        let s = s.clone();
        if s.rows() > 1 && s.cols() > 1 {
            return Err(
                "struct2table: the input must be a scalar struct or a struct vector".into(),
            );
        }
        let names = s.field_names().to_vec();
        let mut columns = Vec::with_capacity(names.len());
        let height;
        if s.len() == 1 {
            for f in 0..names.len() {
                columns.push(s.get(f, 0)?.as_ref().clone());
            }
            height = columns.first().map_or(1, Array::rows);
        } else {
            height = s.len();
            for f in 0..names.len() {
                self.check_cancel()?;
                let contents = (0..s.len())
                    .map(|e| s.get(f, e).cloned())
                    .collect::<Result<Vec<_>>>()?;
                columns.push(self.tbl_from_contents(contents, out)?);
            }
        }
        let mut parts = TableParts::new(names, columns, height);
        self.tbl_apply_options("struct2table", &mut parts, options)?;
        Ok(tvalue(TableArray::new(parts)?))
    }

    // ---- conversions back ----

    /// Row `r` of variable `column` as table2cell/table2struct store it: the
    /// content for a one-column cell variable, otherwise the 1-row piece.
    fn tbl_element(&mut self, column: &Array, r: usize, out: &mut Execution) -> Result<Arc<Array>> {
        match column {
            Array::Cell(c) if c.cols() == 1 => Ok(c.get_linear(r)?.clone()),
            Array::Numeric(m) if m.cols() == 1 => Ok(Arc::new(Matrix::scalar(m.data()[r]).into())),
            _ => Ok(Arc::new(self.tbl_take_rows(column, &[r], out)?)),
        }
    }

    fn tbl_table2cell(&mut self, args: &[Value], out: &mut Execution) -> Result<Value> {
        let t = self.tbl_one(args, "table2cell")?;
        if args.len() != 1 {
            return Err("table2cell: expected one table".into());
        }
        let (h, w) = (t.height(), t.width());
        if h.saturating_mul(w) > crate::script_containers::MAX_CELL_SLOTS {
            return Err("table2cell: the result would exceed 65536 cells".into());
        }
        let mut slots = Vec::with_capacity(h * w);
        for column in t.columns() {
            for r in 0..h {
                if r % CANCEL_STRIDE == 0 {
                    self.check_cancel()?;
                }
                slots.push(self.tbl_element(column, r, out)?);
            }
        }
        Ok(CellArray::from_shared(h, w, slots, &self.cancel)?.into_array_value())
    }

    fn tbl_table2struct(&mut self, args: &[Value], out: &mut Execution) -> Result<Value> {
        let t = self.tbl_one(args, "table2struct")?;
        let (_, options) = split_options(&args[1..], &["ToScalar"]);
        if args.len() != 1 + 2 * options.len() {
            return Err("table2struct: expected a table, then 'ToScalar', true or false".into());
        }
        let mut to_scalar = false;
        for (_, value) in options {
            let m = value.clone().matrix(&self.cancel)?;
            if m.len() != 1 {
                return Err("table2struct: ToScalar must be true or false".into());
            }
            to_scalar = m.data()[0] != 0.;
        }
        let names = t.names().to_vec();
        if to_scalar {
            let slots = t.columns().iter().map(|c| Arc::new(c.clone())).collect();
            return Ok(Value::Container(
                StructArray::from_slots(1, 1, names, slots, &self.cancel)?.into(),
            ));
        }
        let h = t.height();
        if h.saturating_mul(t.width().max(1)) > crate::script_containers::MAX_CELL_SLOTS {
            return Err("table2struct: the result would exceed 65536 struct fields".into());
        }
        let mut slots = Vec::with_capacity(h * t.width());
        for r in 0..h {
            if r % CANCEL_STRIDE == 0 {
                self.check_cancel()?;
            }
            for column in t.columns() {
                slots.push(self.tbl_element(column, r, out)?);
            }
        }
        Ok(Value::Container(
            StructArray::from_slots(h, 1, names, slots, &self.cancel)?.into(),
        ))
    }

    // ---- head / tail ----

    fn tbl_head_tail(&mut self, name: &str, args: &[Value], out: &mut Execution) -> Result<Value> {
        let (a, k) = match args {
            [a] => (a, 8usize),
            [a, k] => {
                let k = k.clone().matrix(&self.cancel)?;
                if k.len() != 1
                    || k.data()[0].is_nan()
                    || k.data()[0] < 0.
                    || k.data()[0].fract() != 0.
                {
                    return Err(format!(
                        "{name}: the number of rows must be a nonnegative integer"
                    ));
                }
                (a, k.data()[0].min(usize::MAX as f64) as usize)
            }
            _ => {
                return Err(format!(
                    "{name}: expected an array or table and a row count"
                ));
            }
        };
        let height = a.dims()[0];
        let k = k.min(height);
        let rows: Vec<usize> = if name == "head" {
            (0..k).collect()
        } else {
            (height - k..height).collect()
        };
        if let Some(t) = table_of(a) {
            let t = t.clone();
            return Ok(tvalue(self.tbl_select(&t, &rows, None, out)?));
        }
        if a.is_nd() {
            return Err(format!("{name}: N-D arrays are not supported"));
        }
        let a = a.clone().array(&self.cancel)?;
        Ok(Value::from(self.tbl_take_rows(&a, &rows, out)?))
    }

    /// The rows `rows` of the variables `vars` (all when None) as a table.
    pub(super) fn tbl_select(
        &mut self,
        t: &TableArray,
        rows: &[usize],
        vars: Option<&[usize]>,
        out: &mut Execution,
    ) -> Result<TableArray> {
        let all: Vec<usize>;
        let vars = match vars {
            Some(v) => v,
            None => {
                all = (0..t.width()).collect();
                &all
            }
        };
        let mut parts = t.parts().select_variables(vars);
        for (k, &v) in vars.iter().enumerate() {
            parts.columns[k] = self.tbl_take_rows(t.column(v), rows, out)?;
        }
        parts.height = rows.len();
        if let Some(times) = t.row_times() {
            parts.row_times = Some(self.tbl_take_rows(times, rows, out)?);
        }
        parts.row_names = t
            .row_names()
            .map(|n| unique_row_names(rows.iter().map(|&r| n[r].clone()).collect()));
        TableArray::new(parts)
    }
}
