//! Grouping: findgroups, splitapply, groupsummary, varfun, rowfun. Group
//! keys sort like `unique` (numbers ascending, text in code-point order,
//! datetimes by time); rows with a missing key (NaN, NaT, '', <missing>)
//! belong to no group, except in groupsummary (IncludeMissingGroups).
use super::super::{Engine, Execution, Value};
use super::{row_expr, split_options, table_of, text_list, text_scalar, tvalue};
use crate::table_array::{TableArray, TableParts};
use crate::{Array, CellArray, FunctionHandle, Matrix, Result};
use std::collections::BTreeMap;
use std::sync::Arc;

/// One comparable key element.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub(super) enum K {
    Num(i64),
    Int(i128),
    Text(Vec<u16>),
    /// A missing element (NaN, NaT, '', <missing>, undefined); sorts last.
    Missing,
}

/// f64 → i64 with the same order (NaN excluded by the caller).
fn ordered(x: f64) -> i64 {
    let x = if x == 0. { 0. } else { x };
    let b = x.to_bits() as i64;
    if b < 0 { b ^ i64::MAX } else { b }
}

/// Class of a join key variable.
///
/// Double, single, and every integer class compare by value. An integer paired
/// with a float converts to double only when that integer is an exact binary64
/// value; `uint64(2^53)+1` does not match `2^53`. Logical joins only logical.
/// Duration, datetime, and categorical join only their own class. Char, string,
/// and cellstr join each other.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum KeyKind {
    Float,
    Integer,
    Logical,
    Duration,
    Datetime,
    Categorical,
    Text,
}

pub(super) fn key_kind(a: &Array, who: &str) -> Result<KeyKind> {
    match a {
        Array::Numeric(_) => Ok(KeyKind::Float),
        Array::Logical(_) => Ok(KeyKind::Logical),
        Array::Typed(t) if t.is_integer() => Ok(KeyKind::Integer),
        Array::Typed(_) => Ok(KeyKind::Float),
        Array::Duration(d) if d.is_calendar() => {
            Err("table: calendarDuration grouping keys are not supported".into())
        }
        Array::Duration(_) => Ok(KeyKind::Duration),
        Array::Datetime(_) => Ok(KeyKind::Datetime),
        Array::Categorical(_) => Ok(KeyKind::Categorical),
        Array::Str(_) | Array::Character(_) | Array::Cell(_) => Ok(KeyKind::Text),
        other => Err(format!(
            "{who}: cannot group by values of class {}",
            other.class()
        )),
    }
}

pub(super) fn key_kinds_compatible(a: KeyKind, b: KeyKind) -> bool {
    matches!(
        (a, b),
        (KeyKind::Float, KeyKind::Float | KeyKind::Integer)
            | (KeyKind::Integer, KeyKind::Float | KeyKind::Integer)
            | (KeyKind::Logical, KeyKind::Logical)
            | (KeyKind::Duration, KeyKind::Duration)
            | (KeyKind::Datetime, KeyKind::Datetime)
            | (KeyKind::Categorical, KeyKind::Categorical)
            | (KeyKind::Text, KeyKind::Text)
    )
}

/// An integer that survives the round trip through f64. Values outside the
/// i64/u64 span, and integers that f64 would round, stay integer keys.
fn exact_f64(i: i128) -> Option<f64> {
    if i < i128::from(i64::MIN) || i > i128::from(u64::MAX) {
        return None;
    }
    let f = i as f64;
    (f as i128 == i).then_some(if f == 0. { 0. } else { f })
}

/// Rewrite exact integer elements as the ordered-float keys a double uses.
/// An integer that is not an exact f64 stays an integer key and does not match
/// the rounded double.
pub(super) fn promote_int_keys(rows: &mut [Vec<K>], start: usize, width: usize) {
    for row in rows {
        for slot in row.iter_mut().skip(start).take(width) {
            if let K::Int(i) = *slot
                && let Some(f) = exact_f64(i)
            {
                *slot = K::Num(ordered(f));
            }
        }
    }
}

/// Keys of every row of `a` (all its columns), None for a row holding a
/// missing value. A row vector of `n` elements counts as `n` rows when
/// `rows` is n.
/// Categorical values key by category order, or by their text when
/// `by_text` (joins compare arrays with different category lists).
pub(super) fn row_keys(a: &Array, rows: usize, who: &str, by_text: bool) -> Result<Vec<Vec<K>>> {
    let by_element = a.rows() == 1 && a.cols() == rows && rows != 1;
    let (r, c) = if by_element {
        (rows, 1)
    } else {
        (a.rows(), a.cols())
    };
    if r != rows {
        return Err(format!(
            "{who}: every grouping variable needs one value per row"
        ));
    }
    let num = |x: f64| (!x.is_nan()).then(|| K::Num(ordered(x)));
    let element = |i: usize| -> Result<Option<K>> {
        Ok(match a {
            Array::Numeric(m) => num(m.data()[i]),
            Array::Logical(m) => Some(K::Num(ordered(f64::from(m.data()[i])))),
            Array::Typed(t) if t.is_integer() => Some(K::Int(t.int(i))),
            Array::Typed(t) => num(t.real(i)),
            Array::Duration(d) if d.is_calendar() => {
                return Err("table: calendarDuration grouping keys are not supported".into());
            }
            Array::Duration(d) => num(d.data()[i]),
            Array::Datetime(d) => d.data()[i].total_nanos().map(K::Int),
            Array::Categorical(c) => match c.codes()[i] {
                0 => None,
                _ if by_text => Some(K::Text(c.text(i).encode_utf16().collect())),
                k => Some(K::Int(i128::from(k))),
            },
            Array::Str(s) => s.data()[i]
                .as_deref()
                .map(|t| K::Text(t.encode_utf16().collect())),
            Array::Cell(cell) => match cell.get_linear(i)?.as_ref() {
                Array::Character(ch) if ch.rows() <= 1 => {
                    (!ch.is_empty()).then(|| K::Text(ch.units().to_vec()))
                }
                _ => {
                    return Err(format!(
                        "{who}: cell grouping variables must be cell arrays of character vectors"
                    ));
                }
            },
            other => {
                return Err(format!(
                    "{who}: cannot group by values of class {}",
                    other.class()
                ));
            }
        })
    };
    if let Array::Character(ch) = a {
        // Each row of a char matrix is one text value.
        let (cr, cc) = (ch.rows(), ch.cols());
        if cr != rows {
            return Err(format!(
                "{who}: every grouping variable needs one value per row"
            ));
        }
        return Ok((0..cr)
            .map(|i| vec![K::Text((0..cc).map(|k| ch.units()[k * cr + i]).collect())])
            .collect());
    }
    let mut keys = Vec::with_capacity(r);
    for i in 0..r {
        let mut row = Vec::with_capacity(c);
        for j in 0..c {
            row.push(element(j * r + i)?.unwrap_or(K::Missing));
        }
        keys.push(row);
    }
    Ok(keys)
}

/// Groups of `n` rows keyed by the combined keys of `vars`.
pub(in crate::engine) struct Groups {
    /// Zero-based group of each row; None = in no group.
    pub of_row: Vec<Option<usize>>,
    /// First row of each group, in group order.
    pub firsts: Vec<usize>,
    pub counts: Vec<usize>,
    /// Rows of each group, ascending.
    pub members: Vec<Vec<usize>>,
}

pub(in crate::engine) fn group_rows(
    vars: &[Array],
    n: usize,
    include_missing: bool,
    who: &str,
) -> Result<Groups> {
    let mut combined: Vec<Vec<K>> = vec![Vec::new(); n];
    for a in vars {
        for (row, k) in combined.iter_mut().zip(row_keys(a, n, who, false)?) {
            row.extend(k);
        }
    }
    // A key with a missing element is its own group (missing sorts last) or,
    // without include_missing, in no group.
    let usable = |k: &Vec<K>| include_missing || !k.contains(&K::Missing);
    let mut first_of: BTreeMap<&Vec<K>, usize> = BTreeMap::new();
    for (i, k) in combined.iter().enumerate() {
        if usable(k) {
            first_of.entry(k).or_insert(i);
        }
    }
    let mut number: BTreeMap<&Vec<K>, usize> = BTreeMap::new();
    let mut firsts = Vec::with_capacity(first_of.len());
    for (g, (k, &first)) in first_of.iter().enumerate() {
        number.insert(k, g);
        firsts.push(first);
    }
    let of_row: Vec<Option<usize>> = combined
        .iter()
        .map(|k| {
            if usable(k) {
                number.get(k).copied()
            } else {
                None
            }
        })
        .collect();
    let mut members = vec![Vec::new(); firsts.len()];
    for (i, g) in of_row.iter().enumerate() {
        if let Some(g) = g {
            members[*g].push(i);
        }
    }
    let counts = members.iter().map(Vec::len).collect();
    Ok(Groups {
        of_row,
        firsts,
        counts,
        members,
    })
}

fn column(values: &[f64]) -> Result<Array> {
    Ok(Array::Numeric(Matrix::from_column_vec(
        values.len(),
        1,
        values.to_vec(),
    )?))
}

/// The function argument of splitapply/varfun/rowfun.
fn handle_of(v: &Value, who: &str) -> Result<FunctionHandle> {
    match v {
        Value::Container(Array::Handle(h)) => Ok(h.clone()),
        other => match text_scalar(other) {
            Some(t) if crate::files::valid_name(&t) => Ok(FunctionHandle::named(&t)),
            _ => Err(format!("{who}: the first input must be a function handle")),
        },
    }
}

/// Name prefix of results: the function name, or 'Fun' for an anonymous one.
fn fun_name(h: &FunctionHandle) -> String {
    h.name().map_or_else(|| "Fun".to_owned(), str::to_owned)
}

/// Summary statistics of groupsummary, omitting missing values.
fn summarize(method: &str, x: &[f64]) -> Option<f64> {
    let present: Vec<f64> = x.iter().copied().filter(|v| !v.is_nan()).collect();
    let n = present.len() as f64;
    let mean = present.iter().sum::<f64>() / n;
    let sorted = || {
        let mut s = present.clone();
        s.sort_by(f64::total_cmp);
        s
    };
    let var = || {
        if present.len() < 2 {
            if present.len() == 1 { 0. } else { f64::NAN }
        } else {
            present.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.)
        }
    };
    Some(match method {
        "mean" => mean,
        "sum" => present.iter().sum(),
        "min" => present.iter().copied().reduce(f64::min).unwrap_or(f64::NAN),
        "max" => present.iter().copied().reduce(f64::max).unwrap_or(f64::NAN),
        "range" => match (
            present.iter().copied().reduce(f64::min),
            present.iter().copied().reduce(f64::max),
        ) {
            (Some(a), Some(b)) => b - a,
            _ => f64::NAN,
        },
        "median" => {
            let s = sorted();
            match s.len() {
                0 => f64::NAN,
                l if l % 2 == 1 => s[l / 2],
                l => (s[l / 2 - 1] + s[l / 2]) / 2.,
            }
        }
        "mode" => {
            let s = sorted();
            let (mut best, mut best_n, mut i) = (f64::NAN, 0, 0);
            while i < s.len() {
                let mut j = i;
                while j < s.len() && s[j] == s[i] {
                    j += 1;
                }
                if j - i > best_n {
                    best = s[i];
                    best_n = j - i;
                }
                i = j;
            }
            best
        }
        "var" => var(),
        "std" => var().sqrt(),
        "numel" => x.len() as f64,
        "nnz" => present.iter().filter(|v| **v != 0.).count() as f64,
        "nummissing" => (x.len() - present.len()) as f64,
        _ => return None,
    })
}

const METHODS: &[&str] = &[
    "mean",
    "sum",
    "min",
    "max",
    "range",
    "median",
    "mode",
    "var",
    "std",
    "numel",
    "nnz",
    "nummissing",
];

/// A groupsummary method: a name or a function handle.
enum Summary {
    Named(String),
    Handle(FunctionHandle),
}

impl Engine {
    /// Rows `rows` of a grouping/data input: elements of a vector that runs
    /// along the rows, rows of anything else.
    fn grp_take(
        &mut self,
        a: &Array,
        rows: &[usize],
        n: usize,
        out: &mut Execution,
    ) -> Result<Array> {
        if a.rows() == 1 && a.cols() == n && n != 1 {
            return self
                .index_value(Value::from(a.clone()), &[row_expr(rows)], out)?
                .array(&self.cancel);
        }
        self.tbl_take_rows(a, rows, out)
    }

    /// Vertical concatenation of results.
    fn grp_stack(&mut self, values: Vec<Value>, out: &mut Execution) -> Result<Value> {
        if values.is_empty() {
            return Ok(Value::Matrix(Matrix::from_column_vec(0, 1, Vec::new())?));
        }
        self.tbl_join(values.into_iter().map(|v| vec![v]).collect(), out)
    }

    pub(super) fn grp_findgroups(
        &mut self,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        if args.is_empty() {
            return Err("findgroups: not enough input arguments".into());
        }
        let n = match table_of(&args[0]) {
            Some(t) => t.height(),
            None => {
                let a = &args[0];
                if a.dims()[0] == 1 {
                    a.dims()[1]
                } else {
                    a.dims()[0]
                }
            }
        };
        let mut vars = Vec::new();
        for v in args {
            match table_of(v) {
                Some(t) => vars.extend(t.columns().iter().cloned()),
                None => vars.push(v.clone().array(&self.cancel)?),
            }
        }
        let groups = group_rows(&vars, n, false, "findgroups")?;
        let g: Vec<f64> = groups
            .of_row
            .iter()
            .map(|g| g.map_or(f64::NAN, |g| (g + 1) as f64))
            .collect();
        let row_shaped = table_of(&args[0]).is_none() && args[0].dims()[0] == 1 && n != 1;
        let (gr, gc) = if row_shaped { (1, n) } else { (n, 1) };
        let mut values = vec![Value::Matrix(Matrix::from_column_vec(gr, gc, g)?)];
        for v in args.iter().take(count.saturating_sub(1)) {
            values.push(match table_of(v) {
                Some(t) => tvalue(self.tbl_select(t, &groups.firsts, None, out)?),
                None => {
                    let a = v.clone().array(&self.cancel)?;
                    Value::from(self.grp_take(&a, &groups.firsts, n, out)?)
                }
            });
        }
        if count > args.len() + 1 {
            return Err("findgroups: too many output arguments".into());
        }
        Ok(values)
    }

    pub(super) fn grp_splitapply(
        &mut self,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        if args.len() < 3 {
            return Err("splitapply: expected a function, data and group numbers".into());
        }
        let h = handle_of(&args[0], "splitapply")?;
        let g = args[args.len() - 1].clone().matrix(&self.cancel)?;
        let n = g.len();
        let mut data = Vec::new();
        for v in &args[1..args.len() - 1] {
            match table_of(v) {
                Some(t) => data.extend(t.columns().iter().cloned()),
                None => data.push(v.clone().array(&self.cancel)?),
            }
        }
        let mut groups: Vec<Vec<usize>> = Vec::new();
        for (i, &x) in g.data().iter().enumerate() {
            if x.is_nan() {
                continue;
            }
            if x > n as f64 {
                return Err(format!(
                    "splitapply: group number {x} is larger than the number of rows ({n})"
                ));
            }
            if x < 1. || x.fract() != 0. {
                return Err("splitapply: group numbers must be positive integers or NaN".into());
            }
            let k = x as usize - 1;
            if groups.len() <= k {
                groups.resize(k + 1, Vec::new());
            }
            groups[k].push(i);
        }
        if let Some(k) = groups.iter().position(Vec::is_empty) {
            return Err(format!(
                "splitapply: group number {} has no rows; group numbers must run from 1 to the number of groups",
                k + 1
            ));
        }
        for d in &data {
            let rows = if d.rows() == 1 && d.cols() == n {
                n
            } else {
                d.rows()
            };
            if rows != n {
                return Err("splitapply: every data input needs one row per group number".into());
            }
        }
        let want = count.max(1);
        let mut results: Vec<Vec<Value>> = vec![Vec::new(); want];
        for rows in &groups {
            let mut call_args = Vec::with_capacity(data.len());
            for d in &data {
                call_args.push(Value::from(self.grp_take(d, rows, n, out)?));
            }
            let outs = self.call_handle(&h, call_args, want, None, out)?;
            for (k, v) in outs.into_iter().take(want).enumerate() {
                results[k].push(v);
            }
        }
        results
            .into_iter()
            .map(|r| self.grp_stack(r, out))
            .collect()
    }

    pub(super) fn grp_groupsummary(
        &mut self,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        let (data, options) = split_options(args, &["IncludeMissingGroups", "IncludeEmptyGroups"]);
        let mut include_missing = true;
        for (o, v) in &options {
            let flag = (*v).clone().matrix(&self.cancel)?;
            let on = flag.data().first().is_some_and(|x| *x != 0.);
            if o == "IncludeMissingGroups" {
                include_missing = on;
            } else if on {
                return Err("groupsummary: 'IncludeEmptyGroups' is not supported yet".into());
            }
        }
        let [input, group_arg, rest @ ..] = data else {
            return Err("groupsummary: expected data and grouping variables".into());
        };
        let methods = match rest.first() {
            None => Vec::new(),
            Some(m) => self.grp_methods(m)?,
        };
        let Some(t) = table_of(input).cloned() else {
            if rest.len() > 1 {
                return Err("groupsummary: for arrays give exactly one method".into());
            }
            return self.grp_summary_array(input, group_arg, &methods, include_missing, count, out);
        };
        if count > 1 {
            return Err("groupsummary: a table input has one output".into());
        }
        let gvars = self.tbl_vars_of(&t, group_arg)?;
        let dvars = match rest.get(1) {
            Some(v) => self.tbl_vars_of(&t, v)?,
            None if methods.is_empty() => Vec::new(),
            None => (0..t.width()).filter(|v| !gvars.contains(v)).collect(),
        };
        if rest.len() > 2 {
            return Err("groupsummary: too many input arguments".into());
        }
        let keys: Vec<Array> = gvars.iter().map(|&v| t.column(v).clone()).collect();
        let groups = group_rows(&keys, t.height(), include_missing, "groupsummary")?;
        let mut parts = self
            .tbl_select(&t, &groups.firsts, Some(&gvars), out)?
            .to_parts();
        parts.row_names = None;
        parts.push_variable(
            "GroupCount".into(),
            column(&groups.counts.iter().map(|&c| c as f64).collect::<Vec<_>>())?,
        );
        let members = |g: usize| -> Vec<usize> { groups.members[g].clone() };
        let mut handle_no = 0;
        let handle_names: Vec<usize> = methods
            .iter()
            .map(|m| {
                if matches!(m, Summary::Handle(..)) {
                    handle_no += 1;
                }
                handle_no
            })
            .collect();
        for &v in &dvars {
            let col = t.column(v).clone();
            let name = t.names()[v].clone();
            for (mi, method) in methods.iter().enumerate() {
                let (label, values) = match method {
                    Summary::Named(m) => {
                        let x = col.clone();
                        let numeric = Value::from(x).matrix(&self.cancel).map_err(|_| {
                            format!(
                                "groupsummary: '{m}' needs numeric data; variable '{name}' is {}",
                                col.class()
                            )
                        })?;
                        let (r, c) = (numeric.rows(), numeric.cols());
                        let mut data = vec![0.; groups.firsts.len() * c];
                        for g in 0..groups.firsts.len() {
                            let rows = members(g);
                            for j in 0..c {
                                let xs: Vec<f64> =
                                    rows.iter().map(|&i| numeric.data()[j * r + i]).collect();
                                data[j * groups.firsts.len() + g] =
                                    summarize(m, &xs).expect("known method");
                            }
                        }
                        (
                            format!("{m}_{name}"),
                            Value::Matrix(Matrix::from_column_vec(groups.firsts.len(), c, data)?),
                        )
                    }
                    Summary::Handle(h) => {
                        let mut each = Vec::new();
                        for g in 0..groups.firsts.len() {
                            let rows = members(g);
                            let piece = self.tbl_take_rows(&col, &rows, out)?;
                            let r = self.call_handle(h, vec![Value::from(piece)], 1, None, out)?;
                            each.push(r.into_iter().next().unwrap_or(Value::Void));
                        }
                        (
                            format!("fun{}_{name}", handle_names[mi]),
                            self.grp_stack(each, out)?,
                        )
                    }
                };
                parts.push_variable(label, values.array(&self.cancel)?);
            }
        }
        Ok(vec![tvalue(TableArray::new(parts)?)])
    }

    /// Method list: a name, a handle, or a cell of them.
    fn grp_methods(&mut self, v: &Value) -> Result<Vec<Summary>> {
        let one = |e: &Engine, v: &Value| -> Result<Summary> {
            if let Value::Container(Array::Handle(h)) = v {
                return Ok(Summary::Handle(h.clone()));
            }
            let _ = e;
            let name = text_scalar(v)
                .ok_or("groupsummary: methods must be names or function handles")?
                .to_ascii_lowercase();
            if !METHODS.contains(&name.as_str()) {
                return Err(format!(
                    "groupsummary: the method '{name}' is not supported"
                ));
            }
            Ok(Summary::Named(name))
        };
        match v {
            Value::Container(Array::Cell(c)) => {
                let mut list = Vec::with_capacity(c.len());
                for i in 0..c.len() {
                    list.push(one(self, &Value::from(c.get_linear(i)?.as_ref().clone()))?);
                }
                Ok(list)
            }
            Value::Container(Array::Str(s)) if s.len() > 1 => {
                let names = text_list(self, v, "Methods")?.unwrap_or_default();
                let _ = s;
                names
                    .iter()
                    .map(|n| one(self, &Value::Text(n.clone())))
                    .collect()
            }
            _ => Ok(vec![one(self, v)?]),
        }
    }

    /// `[B, BG, BC] = groupsummary(A, groupvars, method)` on arrays.
    fn grp_summary_array(
        &mut self,
        input: &Value,
        group_arg: &Value,
        methods: &[Summary],
        include_missing: bool,
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        if methods.len() != 1 {
            return Err("groupsummary: for arrays give exactly one method".into());
        }
        let a = input.clone().matrix(&self.cancel)?;
        let n = if a.rows() == 1 { a.cols() } else { a.rows() };
        let a = if a.rows() == 1 { a.transpose() } else { a };
        let gv = group_arg.clone().array(&self.cancel)?;
        let groups = group_rows(
            std::slice::from_ref(&gv),
            n,
            include_missing,
            "groupsummary",
        )?;
        let ng = groups.firsts.len();
        let c = a.cols();
        let mut data = vec![0.; ng * c];
        let mut stacked = Vec::new();
        for g in 0..ng {
            let rows = groups.members[g].clone();
            match &methods[0] {
                Summary::Named(m) => {
                    for j in 0..c {
                        let xs: Vec<f64> = rows.iter().map(|&i| a.data()[j * n + i]).collect();
                        data[j * ng + g] = summarize(m, &xs).expect("known method");
                    }
                }
                Summary::Handle(h) => {
                    let piece = self.tbl_take_rows(&Array::Numeric(a.clone()), &rows, out)?;
                    let r = self.call_handle(h, vec![Value::from(piece)], 1, None, out)?;
                    stacked.push(r.into_iter().next().unwrap_or(Value::Void));
                }
            }
        }
        let b = if stacked.is_empty() {
            Value::Matrix(Matrix::from_column_vec(ng, c, data)?)
        } else {
            self.grp_stack(stacked, out)?
        };
        let mut values = vec![b];
        if count > 1 {
            let source = if gv.rows() == 1 && gv.cols() == n && n != 1 {
                self.index_value(Value::from(gv.clone()), &[super::super::Expr::All], out)?
                    .array(&self.cancel)?
            } else {
                gv.clone()
            };
            let ids = self.tbl_take_rows(&source, &groups.firsts, out)?;
            values.push(Value::from(ids));
        }
        if count > 2 {
            values.push(Value::from(column(
                &groups.counts.iter().map(|&c| c as f64).collect::<Vec<_>>(),
            )?));
        }
        Ok(values)
    }

    pub(super) fn grp_varfun(&mut self, args: &[Value], out: &mut Execution) -> Result<Value> {
        let (data, options) = split_options(
            args,
            &[
                "InputVariables",
                "GroupingVariables",
                "OutputFormat",
                "ErrorHandler",
            ],
        );
        let [f, input] = data else {
            return Err("varfun: expected a function and a table".into());
        };
        let h = handle_of(f, "varfun")?;
        let t = table_of(input)
            .cloned()
            .ok_or("varfun: the second input must be a table")?;
        let mut format = "table".to_owned();
        let mut gvars = Vec::new();
        let mut ivars = None;
        for (o, v) in &options {
            match o.as_str() {
                "InputVariables" => ivars = Some(self.tbl_vars_of(&t, v)?),
                "GroupingVariables" => gvars = self.tbl_vars_of(&t, v)?,
                "OutputFormat" => {
                    format = text_scalar(v)
                        .ok_or("varfun: OutputFormat must be text")?
                        .to_ascii_lowercase()
                }
                _ => return Err("varfun: ErrorHandler is not supported yet".into()),
            }
        }
        let ivars: Vec<usize> = ivars
            .unwrap_or_else(|| (0..t.width()).collect())
            .into_iter()
            .filter(|v| !gvars.contains(v))
            .collect();
        let prefix = fun_name(&h);
        if gvars.is_empty() {
            let mut results = Vec::with_capacity(ivars.len());
            for &v in &ivars {
                let r =
                    self.call_handle(&h, vec![Value::from(t.column(v).clone())], 1, None, out)?;
                results.push(r.into_iter().next().unwrap_or(Value::Void));
            }
            return match format.as_str() {
                "uniform" => {
                    let row = results.into_iter().collect();
                    self.tbl_join(vec![row], out)
                }
                "cell" => self.grp_cell_row(results),
                "table" => {
                    let names = ivars
                        .iter()
                        .map(|&v| format!("{prefix}_{}", t.names()[v]))
                        .collect();
                    let columns = results
                        .into_iter()
                        .map(|v| v.array(&self.cancel))
                        .collect::<Result<Vec<_>>>()?;
                    let height = columns.first().map_or(0, Array::rows);
                    Ok(tvalue(TableArray::new(TableParts::new(
                        names, columns, height,
                    ))?))
                }
                other => Err(format!("varfun: unknown OutputFormat '{other}'")),
            };
        }
        let keys: Vec<Array> = gvars.iter().map(|&v| t.column(v).clone()).collect();
        let groups = group_rows(&keys, t.height(), false, "varfun")?;
        let mut columns = Vec::with_capacity(ivars.len());
        for &v in &ivars {
            let mut each = Vec::new();
            for g in 0..groups.firsts.len() {
                let rows = groups.members[g].clone();
                let piece = self.tbl_take_rows(t.column(v), &rows, out)?;
                let r = self.call_handle(&h, vec![Value::from(piece)], 1, None, out)?;
                each.push(r.into_iter().next().unwrap_or(Value::Void));
            }
            columns.push(self.grp_stack(each, out)?);
        }
        match format.as_str() {
            "uniform" => self.tbl_join(vec![columns], out),
            "cell" => Err(
                "varfun: OutputFormat 'cell' with GroupingVariables is not supported yet".into(),
            ),
            "table" => {
                let mut parts = self
                    .tbl_select(&t, &groups.firsts, Some(&gvars), out)?
                    .to_parts();
                parts.row_names = None;
                parts.push_variable(
                    "GroupCount".into(),
                    column(&groups.counts.iter().map(|&c| c as f64).collect::<Vec<_>>())?,
                );
                for (&v, c) in ivars.iter().zip(columns) {
                    parts.push_variable(
                        format!("{prefix}_{}", t.names()[v]),
                        c.array(&self.cancel)?,
                    );
                }
                Ok(tvalue(TableArray::new(parts)?))
            }
            other => Err(format!("varfun: unknown OutputFormat '{other}'")),
        }
    }

    fn grp_cell_row(&mut self, values: Vec<Value>) -> Result<Value> {
        let n = values.len();
        let slots = values
            .into_iter()
            .map(|v| v.array(&self.cancel).map(Arc::new))
            .collect::<Result<Vec<_>>>()?;
        Ok(Value::from(Array::from(CellArray::from_shared(
            1,
            n,
            slots,
            &self.cancel,
        )?)))
    }

    pub(super) fn grp_rowfun(
        &mut self,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        let (data, options) = split_options(
            args,
            &[
                "InputVariables",
                "GroupingVariables",
                "OutputVariableNames",
                "NumOutputs",
                "OutputFormat",
                "SeparateInputs",
                "ExtractCellContents",
                "ErrorHandler",
            ],
        );
        let [f, input] = data else {
            return Err("rowfun: expected a function and a table".into());
        };
        let h = handle_of(f, "rowfun")?;
        let t = table_of(input)
            .cloned()
            .ok_or("rowfun: the second input must be a table")?;
        let mut format = "table".to_owned();
        let (mut gvars, mut ivars, mut out_names, mut nout) = (Vec::new(), None, None, None);
        let (mut separate, mut extract) = (true, false);
        for (o, v) in &options {
            let flag = |e: &Engine| -> Result<bool> {
                Ok((*v)
                    .clone()
                    .matrix(&e.cancel)?
                    .data()
                    .first()
                    .is_some_and(|x| *x != 0.))
            };
            match o.as_str() {
                "InputVariables" => ivars = Some(self.tbl_vars_of(&t, v)?),
                "GroupingVariables" => gvars = self.tbl_vars_of(&t, v)?,
                "OutputVariableNames" => {
                    out_names = Some(
                        text_list(self, v, "OutputVariableNames")?
                            .ok_or("rowfun: OutputVariableNames must be text")?,
                    )
                }
                "NumOutputs" => {
                    let m = (*v).clone().matrix(&self.cancel)?;
                    let k = m.data().first().copied().unwrap_or(f64::NAN);
                    if m.len() != 1 || !(0. ..=1024.).contains(&k) || k.fract() != 0. {
                        return Err("rowfun: NumOutputs must be an integer from 0 to 1024".into());
                    }
                    nout = Some(k as usize);
                }
                "OutputFormat" => {
                    format = text_scalar(v)
                        .ok_or("rowfun: OutputFormat must be text")?
                        .to_ascii_lowercase()
                }
                "SeparateInputs" => separate = flag(self)?,
                "ExtractCellContents" => extract = flag(self)?,
                _ => return Err("rowfun: ErrorHandler is not supported yet".into()),
            }
        }
        let ivars: Vec<usize> =
            ivars.unwrap_or_else(|| (0..t.width()).filter(|v| !gvars.contains(v)).collect());
        let nout = nout.unwrap_or_else(|| match (&out_names, format.as_str()) {
            (Some(n), _) => n.len(),
            (None, "uniform" | "cell") => count.max(1),
            _ => 1,
        });
        let out_names =
            out_names.unwrap_or_else(|| (1..=nout).map(|k| format!("Var{k}")).collect());
        if out_names.len() != nout {
            return Err("rowfun: give one OutputVariableNames entry per output".into());
        }
        // Row sets the function is called on: each row, or each group.
        let groups = if gvars.is_empty() {
            None
        } else {
            let keys: Vec<Array> = gvars.iter().map(|&v| t.column(v).clone()).collect();
            Some(group_rows(&keys, t.height(), false, "rowfun")?)
        };
        let sets: Vec<Vec<usize>> = match &groups {
            None => (0..t.height()).map(|i| vec![i]).collect(),
            Some(g) => g.members.clone(),
        };
        let mut results: Vec<Vec<Value>> = vec![Vec::new(); nout];
        for rows in &sets {
            let mut pieces = Vec::with_capacity(ivars.len());
            for &v in &ivars {
                let p = self.tbl_take_rows(t.column(v), rows, out)?;
                let p = match (&p, extract && rows.len() == 1) {
                    (Array::Cell(c), true) if c.len() == 1 => c.get_linear(0)?.as_ref().clone(),
                    _ => p,
                };
                pieces.push(Value::from(p));
            }
            let call_args = if separate {
                pieces
            } else {
                vec![self.tbl_join(vec![pieces], out)?]
            };
            let outs = self.call_handle(&h, call_args, nout, None, out)?;
            if outs.len() < nout && nout > 0 {
                return Err(format!(
                    "rowfun: the function returned {} outputs, {nout} needed",
                    outs.len()
                ));
            }
            for (k, v) in outs.into_iter().take(nout).enumerate() {
                results[k].push(v);
            }
        }
        if format == "cell" {
            // One cell per call and output, each result as returned.
            let n = sets.len();
            let mut slots = Vec::with_capacity(n * nout);
            for r in results {
                for v in r {
                    slots.push(Arc::new(v.array(&self.cancel)?));
                }
            }
            return Ok(vec![Value::from(Array::from(CellArray::from_shared(
                n,
                nout,
                slots,
                &self.cancel,
            )?))]);
        }
        let stacked: Vec<Value> = results
            .into_iter()
            .map(|r| self.grp_stack(r, out))
            .collect::<Result<_>>()?;
        match format.as_str() {
            "uniform" => Ok(stacked),
            "table" => {
                let mut parts = match &groups {
                    None => {
                        let mut p = TableParts::new(Vec::new(), Vec::new(), t.height());
                        p.row_names = t.row_names().map(<[String]>::to_vec);
                        p
                    }
                    Some(g) => {
                        let mut p = self
                            .tbl_select(&t, &g.firsts, Some(&gvars), out)?
                            .to_parts();
                        p.row_names = None;
                        p.push_variable(
                            "GroupCount".into(),
                            column(&g.counts.iter().map(|&c| c as f64).collect::<Vec<_>>())?,
                        );
                        p
                    }
                };
                for (name, v) in out_names.into_iter().zip(stacked) {
                    parts.push_variable(name, v.array(&self.cancel)?);
                }
                Ok(vec![tvalue(TableArray::new(parts)?)])
            }
            other => Err(format!("rowfun: unknown OutputFormat '{other}'")),
        }
    }
}
