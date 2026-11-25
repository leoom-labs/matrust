//! `table`: named column variables of a common height. Storage is one shared
//! (Arc'd) record; every write builds a new record, so `B = A; A.x(1) = 9`
//! leaves B unchanged. Each variable is an ordinary 2-D array whose row count
//! is the table height (a variable may have several columns). Behaviour lives
//! in src/engine/table_fns.rs.
use crate::{Array, Result};
use std::collections::HashSet;
use std::sync::Arc;

/// `T.Properties` is the table's properties: no variable or dimension may
/// carry this name.
pub const RESERVED_NAME: &str = "Properties";
/// Dimension names of a new table.
pub const DEFAULT_DIMENSION_NAMES: [&str; 2] = ["Row", "Variables"];
/// Most variables a table may hold.
pub const MAX_VARIABLES: usize = 10_000;
/// Longest variable or dimension name.
pub const MAX_NAME: usize = 63;

/// Every field of a table; `TableArray` shares one of these.
#[derive(Clone, Debug)]
pub struct TableParts {
    pub names: Vec<String>,
    pub columns: Vec<Array>,
    pub height: usize,
    pub row_names: Option<Vec<String>>,
    /// A timetable shares the table machinery, with a typed time column.
    pub row_times: Option<Array>,
    pub description: String,
    /// One entry per variable ('' when unset), or empty when none is set.
    pub units: Vec<String>,
    /// One entry per variable ('' when unset), or empty when none is set.
    pub descriptions: Vec<String>,
    pub dimension_names: [String; 2],
}

impl TableParts {
    /// Variables only, default properties.
    pub fn new(names: Vec<String>, columns: Vec<Array>, height: usize) -> Self {
        Self {
            names,
            columns,
            height,
            row_names: None,
            row_times: None,
            description: String::new(),
            units: Vec::new(),
            descriptions: Vec::new(),
            dimension_names: DEFAULT_DIMENSION_NAMES.map(String::from),
        }
    }

    /// The same properties for the variables at `keep` (in that order).
    pub fn select_variables(&self, keep: &[usize]) -> Self {
        let pick = |v: &Vec<String>| {
            if v.is_empty() {
                Vec::new()
            } else {
                keep.iter().map(|&i| v[i].clone()).collect()
            }
        };
        Self {
            names: keep.iter().map(|&i| self.names[i].clone()).collect(),
            columns: keep.iter().map(|&i| self.columns[i].clone()).collect(),
            height: self.height,
            row_names: self.row_names.clone(),
            row_times: self.row_times.clone(),
            description: self.description.clone(),
            units: pick(&self.units),
            descriptions: pick(&self.descriptions),
            dimension_names: self.dimension_names.clone(),
        }
    }

    /// Append one variable (properties extended with '').
    pub fn push_variable(&mut self, name: String, column: Array) {
        self.names.push(name);
        self.columns.push(column);
        if !self.units.is_empty() {
            self.units.push(String::new());
        }
        if !self.descriptions.is_empty() {
            self.descriptions.push(String::new());
        }
    }

    /// Remove the variables at `drop` (any order).
    pub fn remove_variables(&mut self, drop: &[usize]) {
        let keep: Vec<usize> = (0..self.names.len())
            .filter(|i| !drop.contains(i))
            .collect();
        let height = self.height;
        *self = self.select_variables(&keep);
        self.height = height;
    }
}

/// A function or graphics handle cannot be a variable: selecting rows of
/// one would call the function with the subscripts.
pub fn reject_handle(column: &Array) -> Result<()> {
    match column {
        Array::Handle(_) => Err("table: function handle variables are not supported yet".into()),
        Array::Graphics(_) => Err("table: graphics handle variables are not supported yet".into()),
        _ => Ok(()),
    }
}

/// Normalize vector orientation without copying shared time elements.
pub(crate) fn row_times_column(times: Array) -> Result<Array> {
    let n = times.len();
    if times.rows() != 1 && times.cols() != 1 && n != 0 {
        return Err("timetable: RowTimes must be a vector".into());
    }
    match times {
        Array::Datetime(t) => Ok(t.reshaped(n, 1)?.into()),
        Array::Duration(d) if !d.is_calendar() => Ok(d.reshaped(n, 1)?.into()),
        _ => Err("timetable: RowTimes must be a datetime or duration vector".into()),
    }
}

/// A valid MATLAB identifier of at most MAX_NAME characters.
pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && name.len() <= MAX_NAME
}

/// A valid table variable label: nonempty text of at most MAX_NAME characters.
pub fn valid_variable_name(name: &str) -> bool {
    !name.is_empty() && name.chars().count() <= MAX_NAME
}

#[derive(Clone, Debug)]
pub struct TableArray {
    inner: Arc<TableParts>,
}

impl TableArray {
    /// Checks the variable count, names, heights and row names.
    pub fn new(mut parts: TableParts) -> Result<Self> {
        if let Some(times) = parts.row_times.take() {
            parts.row_times = Some(row_times_column(times)?);
        }
        if parts.names.len() != parts.columns.len() {
            return Err("table: every variable needs one name".into());
        }
        if parts.names.len() > MAX_VARIABLES {
            return Err(format!(
                "table: a table holds at most {MAX_VARIABLES} variables"
            ));
        }
        let [row_dim, var_dim] = &parts.dimension_names;
        let mut seen = HashSet::with_capacity(parts.names.len());
        for name in &parts.names {
            if !valid_variable_name(name) {
                return Err(format!(
                    "Invalid table variable name: '{name}'. Variable names must be nonempty and at most {MAX_NAME} characters."
                ));
            }
            if name == RESERVED_NAME {
                return Err(format!(
                    "Invalid table variable name: '{name}'. T.{name} is the table's properties, so no variable can use that name."
                ));
            }
            if name == row_dim || name == var_dim {
                return Err(format!(
                    "Invalid table variable name: '{name}'. It is also a table dimension name ('{row_dim}', '{var_dim}'); choose another variable name or change DimensionNames."
                ));
            }
            if !seen.insert(name.as_str()) {
                return Err(format!("Duplicate table variable name: '{name}'."));
            }
        }
        for dim in [row_dim, var_dim] {
            if !valid_name(dim) || dim == RESERVED_NAME {
                return Err(format!("Invalid table dimension name: '{dim}'."));
            }
        }
        for column in &parts.columns {
            reject_handle(column)?;
            if column.rows() != parts.height {
                return Err("All table variables must have the same number of rows.".into());
            }
            if column.is_nd() {
                return Err("table: N-D variables are not supported yet".into());
            }
            if matches!(column, Array::Sparse(_)) {
                return Err("table: sparse variables are not supported yet".into());
            }
        }
        if let Some(times) = &parts.row_times {
            if !matches!(times, Array::Datetime(_) | Array::Duration(_))
                || times.cols() != 1
                || times.rows() != parts.height
                || matches!(times, Array::Duration(d) if d.is_calendar())
            {
                return Err(
                    "timetable: RowTimes must be a datetime or duration column matching the height"
                        .into(),
                );
            }
            if matches!(times,Array::Datetime(t) if t.data().iter().any(|v|v.is_nat()))
                || matches!(times,Array::Duration(d) if d.data().iter().any(|v|!v.is_finite() || v.abs()>=1e18))
            {
                return Err("timetable: RowTimes must be finite, nonmissing times".into());
            }
            if parts.row_names.is_some() {
                return Err("timetable: RowNames is not supported".into());
            }
        }
        if let Some(rows) = &parts.row_names {
            if rows.len() != parts.height {
                return Err(
                    "The number of row names must match the number of rows in the table.".into(),
                );
            }
            let mut seen = HashSet::with_capacity(rows.len());
            for name in rows {
                if name.is_empty() {
                    return Err("Row names must be nonempty text.".into());
                }
                if !seen.insert(name.as_str()) {
                    return Err(format!("Duplicate row name: '{name}'."));
                }
            }
        }
        for (what, list) in [
            ("VariableUnits", &parts.units),
            ("VariableDescriptions", &parts.descriptions),
        ] {
            if !list.is_empty() && list.len() != parts.names.len() {
                return Err(format!(
                    "The {what} property must have one entry per table variable."
                ));
            }
        }
        if parts.dimension_names[0] == parts.dimension_names[1] {
            return Err("Duplicate table dimension name.".into());
        }
        Ok(Self {
            inner: Arc::new(parts),
        })
    }
    /// 0x0 table.
    pub fn empty() -> Self {
        Self {
            inner: Arc::new(TableParts::new(Vec::new(), Vec::new(), 0)),
        }
    }
    pub fn parts(&self) -> &TableParts {
        &self.inner
    }
    /// A copy of the fields to edit and rebuild with [`TableArray::new`].
    pub fn to_parts(&self) -> TableParts {
        self.inner.as_ref().clone()
    }
    pub fn height(&self) -> usize {
        self.inner.height
    }
    pub fn width(&self) -> usize {
        self.inner.names.len()
    }
    pub fn rows(&self) -> usize {
        self.height()
    }
    pub fn cols(&self) -> usize {
        self.width()
    }
    pub fn len(&self) -> usize {
        self.height().saturating_mul(self.width())
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn names(&self) -> &[String] {
        &self.inner.names
    }
    pub fn columns(&self) -> &[Array] {
        &self.inner.columns
    }
    pub fn column(&self, i: usize) -> &Array {
        &self.inner.columns[i]
    }
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.inner.names.iter().position(|n| n == name)
    }
    pub fn is_timetable(&self) -> bool {
        self.inner.row_times.is_some()
    }
    pub fn row_times(&self) -> Option<&Array> {
        self.inner.row_times.as_ref()
    }
    pub fn row_names(&self) -> Option<&[String]> {
        self.inner.row_names.as_deref()
    }
    pub fn same_storage(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
    /// Sum of the variables' bytes.
    pub fn bytes(&self) -> usize {
        self.inner
            .columns
            .iter()
            .fold(self.row_times().map_or(0, Array::bytes), |sum, c| {
                sum.saturating_add(c.bytes())
            })
    }
    pub fn preview(&self) -> String {
        format!(
            "{}x{} {}",
            self.height(),
            self.width(),
            if self.is_timetable() {
                "timetable"
            } else {
                "table"
            }
        )
    }
}

impl std::fmt::Display for TableArray {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.preview())
    }
}
