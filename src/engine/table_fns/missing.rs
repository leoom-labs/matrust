//! Missing data on arrays and tables: ismissing (tables, datetime, duration),
//! rmmissing, fillmissing (constant, previous, next, nearest, linear) and
//! standardizeMissing. Standard missing values: NaN (double, single,
//! duration), NaT, `<missing>` strings, '' in a cellstr, ' ' in a char array.
//! Element moves and writes go through the engine's own indexing and indexed
//! assignment, so every type keeps its rules.
use super::super::{Engine, Execution, Value};
use super::{cellstr, row_expr, split_options, table_of, text_scalar, tvalue};
use crate::syntax::Expr;
use crate::table_array::TableArray;
use crate::{Array, LogicalMatrix, Matrix, Result};

/// Missing flag of every element (column-major); None when the type has no
/// missing value (logical, integers, structs, ...).
pub(super) fn missing_flags(a: &Array) -> Result<Option<Vec<bool>>> {
    let n = a.len();
    Ok(Some(match a {
        Array::Numeric(m) => m.data().iter().map(|x| x.is_nan()).collect(),
        Array::Complex(m) => m
            .data()
            .iter()
            .map(|z| z.re.is_nan() || z.im.is_nan())
            .collect(),
        Array::Typed(t) if !t.is_integer() => (0..n).map(|i| t.real(i).is_nan()).collect(),
        Array::Str(s) => s.data().iter().map(Option::is_none).collect(),
        Array::Datetime(d) => d.data().iter().map(|v| v.is_nat()).collect(),
        Array::Duration(d) if d.is_calendar() => d
            .calendar_data()
            .unwrap()
            .iter()
            .map(|v| v.iter().any(|x| x.is_nan()))
            .collect(),
        Array::Duration(d) => d.data().iter().map(|x| x.is_nan()).collect(),
        Array::Categorical(c) => c.codes().iter().map(|&k| k == 0).collect(),
        Array::Character(c) => c.units().iter().map(|&u| u == u16::from(b' ')).collect(),
        Array::Cell(c) => {
            let mut flags = Vec::with_capacity(n);
            for i in 0..n {
                flags.push(match c.get_linear(i)?.as_ref() {
                    Array::Character(s) if s.rows() <= 1 => s.is_empty(),
                    _ => return Ok(None),
                });
            }
            flags
        }
        _ => return Ok(None),
    }))
}

fn flags_or_false(a: &Array) -> Result<Vec<bool>> {
    Ok(missing_flags(a)?.unwrap_or_else(|| vec![false; a.len()]))
}

/// Linear indices of each 1-D slice of an r-by-c array along `dim`.
fn slices(r: usize, c: usize, dim: usize) -> Vec<Vec<usize>> {
    if dim == 1 {
        (0..c)
            .map(|j| (0..r).map(|i| j * r + i).collect())
            .collect()
    } else {
        (0..r)
            .map(|i| (0..c).map(|j| j * r + i).collect())
            .collect()
    }
}

fn logical(r: usize, c: usize, flags: &[bool], engine: &Engine) -> Result<Value> {
    Ok(Value::Logical(LogicalMatrix::from_column_vec(
        r,
        c,
        flags.iter().map(|&b| u8::from(b)).collect(),
        &engine.cancel,
    )?))
}

fn dim_arg(v: &Value, who: &str) -> Result<usize> {
    match v {
        Value::Matrix(m) if m.len() == 1 && matches!(m.data()[0], 1. | 2.) => {
            Ok(m.data()[0] as usize)
        }
        _ => Err(format!("{who}: the dimension must be 1 or 2")),
    }
}

/// First non-singleton dimension of a 2-D array.
fn default_dim(a: &Array) -> usize {
    if a.rows() == 1 && a.cols() != 1 { 2 } else { 1 }
}

fn two_d(a: &Array, who: &str) -> Result<()> {
    if a.is_nd() {
        return Err(format!("{who}: N-D arrays are not supported yet"));
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq)]
enum Method {
    Constant,
    Previous,
    Next,
    Nearest,
    Linear,
}

#[derive(Clone)]
enum Ends {
    Extrap,
    None,
    Method(Method),
    Constant(Value),
}

/// Source index for each element of one slice under a moving method; None
/// keeps the element as it is.
fn sources(flags: &[bool], method: Method) -> Vec<Option<usize>> {
    let n = flags.len();
    let mut prev = vec![None; n];
    let mut next = vec![None; n];
    let mut last = None;
    for i in 0..n {
        if !flags[i] {
            last = Some(i);
        }
        prev[i] = last;
    }
    last = None;
    for i in (0..n).rev() {
        if !flags[i] {
            last = Some(i);
        }
        next[i] = last;
    }
    (0..n)
        .map(|i| {
            if !flags[i] {
                return None;
            }
            match method {
                Method::Previous => prev[i],
                Method::Next => next[i],
                Method::Nearest => match (prev[i], next[i]) {
                    (Some(p), Some(q)) => Some(if q - i <= i - p { q } else { p }),
                    (p, q) => p.or(q),
                },
                _ => None,
            }
        })
        .collect()
}

impl Engine {
    /// The standard missing value for assignments into `a`, or None.
    pub(super) fn missing_piece(
        &mut self,
        a: &Array,
        out: &mut Execution,
    ) -> Result<Option<Array>> {
        Ok(Some(match a {
            Array::Numeric(_) | Array::Complex(_) | Array::Typed(_) => {
                if matches!(a, Array::Typed(t) if t.is_integer()) {
                    return Ok(None);
                }
                Array::Numeric(Matrix::scalar(f64::NAN))
            }
            Array::Str(_) => Array::Str(crate::StringArray::new(1, 1, vec![None])?),
            Array::Cell(_) => cellstr(1, 1, &[String::new()], &self.cancel)?,
            Array::Character(_) => Value::Text(" ".into()).array(&self.cancel)?,
            Array::Datetime(_) => self.call("NaT", Vec::new(), out)?.array(&self.cancel)?,
            Array::Duration(d) if d.is_calendar() => Array::Duration(
                crate::DurationArray::new_calendar(1, 1, vec![[f64::NAN; 4]])?,
            ),
            Array::Duration(_) => self
                .call(
                    "seconds",
                    vec![Value::Matrix(Matrix::scalar(f64::NAN))],
                    out,
                )?
                .array(&self.cancel)?,
            Array::Categorical(c) => Array::Categorical(crate::CategoricalArray::new(
                1,
                1,
                vec![0],
                c.categories().clone(),
                c.is_ordinal(),
            )?),
            _ => return Ok(None),
        }))
    }

    /// `a(idx) = piece` with linear indices.
    fn mis_assign(
        &mut self,
        a: Array,
        idx: &[usize],
        piece: Array,
        out: &mut Execution,
    ) -> Result<Array> {
        if idx.is_empty() {
            return Ok(a);
        }
        self.tbl_write(a, vec![row_expr(idx)], Some(piece), out)
    }

    /// `reshape(a(src), size(a))`.
    fn mis_gather(&mut self, a: &Array, src: &[usize], out: &mut Execution) -> Result<Array> {
        if src.iter().enumerate().all(|(i, &s)| i == s) {
            return Ok(a.clone());
        }
        let picked = self.index_value(Value::from(a.clone()), &[row_expr(src)], out)?;
        // `B = a; B(:) = a(src)` keeps the shape for every type.
        self.tbl_write(
            a.clone(),
            vec![Expr::All],
            Some(picked.array(&self.cancel)?),
            out,
        )
    }

    // ---- ismissing ----

    /// ismissing for tables, datetime and duration (other inputs stay with
    /// the string module).
    pub(super) fn mis_ismissing(&mut self, args: &[Value]) -> Result<Value> {
        if args.len() != 1 {
            return Err("ismissing: indicators are not supported for tables yet".into());
        }
        if let Some(t) = table_of(&args[0]) {
            let mut flags = Vec::new();
            let mut cols = 0;
            for c in t.columns() {
                flags.extend(flags_or_false(c)?);
                cols += c.cols();
            }
            return logical(t.height(), cols, &flags, self);
        }
        let a = args[0].clone().array(&self.cancel)?;
        let flags = flags_or_false(&a)?;
        Ok(Value::Logical(
            LogicalMatrix::from_column_vec(
                a.rows(),
                a.cols(),
                flags.iter().map(|&b| u8::from(b)).collect(),
                &self.cancel,
            )?
            .with_dims(&a.dims())?,
        ))
    }

    // ---- rmmissing ----

    pub(super) fn mis_rmmissing(
        &mut self,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        let (data, options) = split_options(args, &["MinNumMissing", "DataVariables"]);
        let mut min = 1usize;
        let mut data_vars = None;
        for (o, v) in &options {
            if o == "MinNumMissing" {
                let m = (*v).clone().matrix(&self.cancel)?;
                if m.len() != 1
                    || m.data()[0].is_nan()
                    || m.data()[0] < 0.
                    || m.data()[0].fract() != 0.
                {
                    return Err("rmmissing: MinNumMissing must be a nonnegative integer".into());
                }
                min = m.data()[0] as usize;
            } else {
                data_vars = Some(*v);
            }
        }
        let (input, dim) = match data {
            [a] => (a, None),
            [a, d] => (a, Some(dim_arg(d, "rmmissing")?)),
            _ => {
                return Err(
                    "rmmissing: expected an array or table, an optional dimension and options"
                        .into(),
                );
            }
        };
        if let Some(t) = table_of(input).cloned() {
            if dim == Some(2) {
                return Err("rmmissing: for tables only dimension 1 is supported".into());
            }
            let vars = match data_vars {
                Some(v) => self.tbl_vars_of(&t, v)?,
                None => (0..t.width()).collect(),
            };
            let mut missing = vec![0usize; t.height()];
            for &v in &vars {
                let c = t.column(v);
                let f = flags_or_false(c)?;
                for (i, &b) in f.iter().enumerate() {
                    missing[i % t.height().max(1)] += usize::from(b);
                }
            }
            let gone: Vec<bool> = missing.iter().map(|&m| m >= min).collect();
            let keep: Vec<usize> = (0..t.height()).filter(|&i| !gone[i]).collect();
            let result = self.tbl_select(&t, &keep, None, out)?;
            let mut values = vec![tvalue(result)];
            if count > 1 {
                values.push(logical(t.height(), 1, &gone, self)?);
            }
            return Ok(values);
        }
        if data_vars.is_some() {
            return Err("rmmissing: DataVariables needs a table".into());
        }
        let a = input.clone().array(&self.cancel)?;
        two_d(&a, "rmmissing")?;
        let flags = flags_or_false(&a)?;
        let (r, c) = (a.rows(), a.cols());
        let dim = dim.unwrap_or_else(|| default_dim(&a));
        // Rows (dim 1) or columns (dim 2) holding at least `min` missing values.
        let lines = if dim == 1 { r } else { c };
        let mut missing = vec![0usize; lines];
        for (i, &b) in flags.iter().enumerate() {
            let line = if dim == 1 { i % r.max(1) } else { i / r.max(1) };
            missing[line] += usize::from(b);
        }
        let gone: Vec<bool> = missing.iter().map(|&m| m >= min).collect();
        let keep: Vec<usize> = (0..lines).filter(|&i| !gone[i]).collect();
        let result = if keep.len() == lines {
            a.clone()
        } else {
            let subs = if dim == 1 {
                [row_expr(&keep), Expr::All]
            } else {
                [Expr::All, row_expr(&keep)]
            };
            if keep.is_empty() {
                // `A(all, :) = []` keeps the type and the other dimension.
                let all: Vec<usize> = (0..lines).collect();
                let subs = if dim == 1 {
                    vec![row_expr(&all), Expr::All]
                } else {
                    vec![Expr::All, row_expr(&all)]
                };
                self.tbl_write(a.clone(), subs, None, out)?
            } else {
                self.index_value(Value::from(a.clone()), &subs, out)?
                    .array(&self.cancel)?
            }
        };
        let mut values = vec![Value::from(result)];
        if count > 1 {
            let (tr, tc) = if dim == 1 { (r, 1) } else { (1, c) };
            values.push(logical(tr, tc, &gone, self)?);
        }
        Ok(values)
    }

    // ---- fillmissing ----

    pub(super) fn mis_fillmissing(
        &mut self,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        let (data, options) =
            split_options(args, &["EndValues", "DataVariables", "MissingLocations"]);
        let (input, rest) = data
            .split_first()
            .ok_or("fillmissing: not enough input arguments")?;
        let name = rest
            .first()
            .and_then(text_scalar)
            .ok_or("fillmissing: the second input must be a fill method")?
            .to_ascii_lowercase();
        let method = match name.as_str() {
            "constant" => Method::Constant,
            "previous" => Method::Previous,
            "next" => Method::Next,
            "nearest" => Method::Nearest,
            "linear" => Method::Linear,
            "spline" | "pchip" | "makima" | "movmean" | "movmedian" | "knn" => {
                return Err(format!(
                    "fillmissing: the '{name}' method is not supported yet"
                ));
            }
            _ => return Err(format!("fillmissing: unknown fill method '{name}'")),
        };
        let mut rest = &rest[1..];
        let constant = if method == Method::Constant {
            let (v, r) = rest
                .split_first()
                .ok_or("fillmissing: 'constant' needs a fill value")?;
            rest = r;
            Some(v.clone())
        } else {
            None
        };
        let dim = match rest {
            [] => None,
            [d] => Some(dim_arg(d, "fillmissing")?),
            _ => return Err("fillmissing: too many input arguments".into()),
        };
        let mut ends = Ends::Extrap;
        let mut data_vars = None;
        for (o, v) in &options {
            match o.as_str() {
                "EndValues" => {
                    ends = match text_scalar(v).map(|s| s.to_ascii_lowercase()).as_deref() {
                        Some("extrap") => Ends::Extrap,
                        Some("none") => Ends::None,
                        Some("previous") => Ends::Method(Method::Previous),
                        Some("next") => Ends::Method(Method::Next),
                        Some("nearest") => Ends::Method(Method::Nearest),
                        Some(other) => {
                            return Err(format!("fillmissing: unknown EndValues '{other}'"));
                        }
                        None => Ends::Constant((*v).clone()),
                    }
                }
                "DataVariables" => data_vars = Some(*v),
                _ => return Err("fillmissing: MissingLocations is not supported yet".into()),
            }
        }
        if let Some(t) = table_of(input).cloned() {
            if dim == Some(2) {
                return Err("fillmissing: for tables only dimension 1 is supported".into());
            }
            let vars = match data_vars {
                Some(v) => self.tbl_vars_of(&t, v)?,
                None => (0..t.width()).collect(),
            };
            // A constant may give one value per filled variable.
            let per_var: Option<Vec<Value>> = match &constant {
                Some(Value::Container(Array::Cell(c)))
                    if c.len() == vars.len() && vars.len() > 1 =>
                {
                    let mut list = Vec::new();
                    for i in 0..c.len() {
                        list.push(Value::from(c.get_linear(i)?.as_ref().clone()));
                    }
                    Some(list)
                }
                Some(Value::Matrix(m)) if m.len() == vars.len() && vars.len() > 1 => Some(
                    m.data()
                        .iter()
                        .map(|&x| Value::Matrix(Matrix::scalar(x)))
                        .collect(),
                ),
                _ => None,
            };
            let mut parts = t.to_parts();
            let mut filled_all: Vec<bool> = Vec::new();
            let mut tf_cols = 0;
            for (v, column) in t.columns().iter().enumerate() {
                tf_cols += column.cols();
                let Some(k) = vars.iter().position(|&x| x == v) else {
                    filled_all.extend(vec![false; column.len()]);
                    continue;
                };
                let c = match &per_var {
                    Some(list) => Some(list[k].clone()),
                    None => constant.clone(),
                };
                let name = &t.names()[v];
                let (new, filled) = self
                    .mis_fill_array(column, method, c, 1, &ends, out)
                    .map_err(|e| format!("{e} (table variable '{name}')"))?;
                parts.columns[v] = new;
                filled_all.extend(filled);
            }
            let mut values = vec![tvalue(TableArray::new(parts)?)];
            if count > 1 {
                values.push(logical(t.height(), tf_cols, &filled_all, self)?);
            }
            return Ok(values);
        }
        if data_vars.is_some() {
            return Err("fillmissing: DataVariables needs a table".into());
        }
        let a = input.clone().array(&self.cancel)?;
        two_d(&a, "fillmissing")?;
        let dim = dim.unwrap_or_else(|| default_dim(&a));
        let (new, filled) = self.mis_fill_array(&a, method, constant, dim, &ends, out)?;
        let mut values = vec![Value::from(new)];
        if count > 1 {
            values.push(logical(a.rows(), a.cols(), &filled, self)?);
        }
        Ok(values)
    }

    /// Fill one array along `dim`; also the filled flags.
    fn mis_fill_array(
        &mut self,
        a: &Array,
        method: Method,
        constant: Option<Value>,
        dim: usize,
        ends: &Ends,
        out: &mut Execution,
    ) -> Result<(Array, Vec<bool>)> {
        let Some(flags) = missing_flags(a)? else {
            if matches!(a, Array::Cell(_)) {
                return Err(
                    "fillmissing: cell inputs must be cell arrays of character vectors".into(),
                );
            }
            return Ok((a.clone(), vec![false; a.len()]));
        };
        let (r, c) = (a.rows(), a.cols());
        // Positions in the leading and trailing missing runs of each slice.
        let mut end_positions = Vec::new();
        let mut inner = vec![true; a.len()];
        for s in slices(r, c, dim) {
            let first = s.iter().position(|&i| !flags[i]);
            let last = s.iter().rposition(|&i| !flags[i]);
            for (k, &i) in s.iter().enumerate() {
                if flags[i] && (first.is_none_or(|f| k < f) || last.is_none_or(|l| k > l)) {
                    end_positions.push(i);
                    inner[i] = false;
                }
            }
        }
        let mut result = a.clone();
        match method {
            Method::Constant => {
                let v = constant.expect("constant value");
                let idx: Vec<usize> = (0..a.len())
                    .filter(|&i| flags[i] && (inner[i] || matches!(ends, Ends::Extrap)))
                    .collect();
                result = self.mis_fill_constant(a, result, &idx, &v, dim, out)?;
            }
            Method::Linear => {
                let Array::Numeric(m) = a else {
                    return Err(format!(
                        "fillmissing: the 'linear' method needs double data, not {}",
                        a.class()
                    ));
                };
                let mut data = m.data().to_vec();
                for s in slices(r, c, dim) {
                    let known: Vec<usize> = (0..s.len()).filter(|&k| !flags[s[k]]).collect();
                    if known.len() < 2 {
                        continue;
                    }
                    let y = |k: usize| m.data()[s[k]];
                    let line = |k0: usize, k1: usize, k: usize| {
                        y(k0) + (y(k1) - y(k0)) * (k as f64 - k0 as f64) / (k1 as f64 - k0 as f64)
                    };
                    let mut seg = 0;
                    for k in 0..s.len() {
                        if !flags[s[k]] {
                            continue;
                        }
                        while seg + 1 < known.len() && known[seg + 1] < k {
                            seg += 1;
                        }
                        let i = s[k];
                        if k < known[0] || k > known[known.len() - 1] {
                            if matches!(ends, Ends::Extrap) {
                                let (k0, k1) = if k < known[0] {
                                    (known[0], known[1])
                                } else {
                                    (known[known.len() - 2], known[known.len() - 1])
                                };
                                data[i] = line(k0, k1, k);
                            }
                        } else {
                            data[i] = line(known[seg], known[seg + 1], k);
                        }
                    }
                }
                result = Array::Numeric(Matrix::from_column_vec(r, c, data)?);
            }
            _ => {
                let mut src: Vec<usize> = (0..a.len()).collect();
                for s in slices(r, c, dim) {
                    let f: Vec<bool> = s.iter().map(|&i| flags[i]).collect();
                    for (k, from) in sources(&f, method).into_iter().enumerate() {
                        if let Some(from) = from
                            && (inner[s[k]] || matches!(ends, Ends::Extrap))
                        {
                            src[s[k]] = s[from];
                        }
                    }
                }
                result = self.mis_gather(&result, &src, out)?;
            }
        }
        // End values other than the method itself.
        match ends {
            Ends::Method(m) if !end_positions.is_empty() => {
                let mut src: Vec<usize> = (0..a.len()).collect();
                for s in slices(r, c, dim) {
                    let f: Vec<bool> = s.iter().map(|&i| flags[i]).collect();
                    for (k, from) in sources(&f, *m).into_iter().enumerate() {
                        if let Some(from) = from
                            && !inner[s[k]]
                        {
                            src[s[k]] = s[from];
                        }
                    }
                }
                result = self.mis_gather(&result, &src, out)?;
            }
            Ends::Constant(v) if !end_positions.is_empty() => {
                result = self.mis_fill_constant(a, result, &end_positions, v, dim, out)?;
            }
            _ => {}
        }
        let after = flags_or_false(&result)?;
        let filled = flags.iter().zip(&after).map(|(&b, &n)| b && !n).collect();
        Ok((result, filled))
    }

    /// `result(idx) = v`; a numeric `v` with one value per column (dim 1) or
    /// row (dim 2) fills each with its own value.
    fn mis_fill_constant(
        &mut self,
        a: &Array,
        result: Array,
        idx: &[usize],
        v: &Value,
        dim: usize,
        out: &mut Execution,
    ) -> Result<Array> {
        let (r, c) = (a.rows(), a.cols());
        let lines = if dim == 1 { c } else { r };
        if let Value::Matrix(m) = v
            && m.len() > 1
        {
            if m.len() != lines {
                return Err(format!(
                    "fillmissing: the fill value must be a scalar or have one value per {} ({lines})",
                    if dim == 1 { "column" } else { "row" }
                ));
            }
            let mut result = result;
            for (line, &x) in m.data().iter().enumerate() {
                let mine: Vec<usize> = idx
                    .iter()
                    .copied()
                    .filter(|&i| if dim == 1 { i / r.max(1) } else { i % r.max(1) } == line)
                    .collect();
                let piece = Array::Numeric(Matrix::scalar(x));
                result = self.mis_assign(result, &mine, piece, out)?;
            }
            return Ok(result);
        }
        let piece = self.mis_fill_value(a, v)?;
        self.mis_assign(result, idx, piece, out)
    }

    /// A fill value as an assignable piece for `a` (text into a cellstr
    /// becomes a one-element cell).
    fn mis_fill_value(&mut self, a: &Array, v: &Value) -> Result<Array> {
        if matches!(a, Array::Cell(_))
            && let Some(text) = text_scalar(v)
        {
            return cellstr(1, 1, &[text], &self.cancel);
        }
        v.clone().array(&self.cancel)
    }

    // ---- standardizeMissing ----

    pub(super) fn mis_standardize(&mut self, args: &[Value], out: &mut Execution) -> Result<Value> {
        let (data, options) = split_options(args, &["DataVariables"]);
        let [input, indicator] = data else {
            return Err(
                "standardizeMissing: expected an array or table and the missing value indicators"
                    .into(),
            );
        };
        let numbers: Vec<f64> = match indicator {
            Value::Matrix(m) => m.data().to_vec(),
            Value::Container(Array::Cell(c)) => {
                let mut list = Vec::new();
                for i in 0..c.len() {
                    if let Array::Numeric(m) = c.get_linear(i)?.as_ref() {
                        list.extend(m.data());
                    }
                }
                list
            }
            Value::Container(Array::Typed(_)) => {
                indicator.clone().matrix(&self.cancel)?.data().to_vec()
            }
            _ => Vec::new(),
        };
        let texts: Vec<String> = match indicator {
            Value::Container(Array::Cell(c)) => {
                let mut list = Vec::new();
                for i in 0..c.len() {
                    if let Some(t) = text_scalar(&Value::from(c.get_linear(i)?.as_ref().clone())) {
                        list.push(t);
                    }
                }
                list
            }
            Value::Container(Array::Str(s)) => {
                s.data().iter().flatten().map(|t| t.to_string()).collect()
            }
            other => text_scalar(other).into_iter().collect(),
        };
        if let Some(t) = table_of(input).cloned() {
            let vars = match options.first() {
                Some((_, v)) => self.tbl_vars_of(&t, v)?,
                None => (0..t.width()).collect(),
            };
            let mut parts = t.to_parts();
            for v in vars {
                parts.columns[v] =
                    self.mis_standardize_array(t.column(v), &numbers, &texts, out)?;
            }
            return Ok(tvalue(TableArray::new(parts)?));
        }
        if !options.is_empty() {
            return Err("standardizeMissing: DataVariables needs a table".into());
        }
        let a = input.clone().array(&self.cancel)?;
        Ok(Value::from(
            self.mis_standardize_array(&a, &numbers, &texts, out)?,
        ))
    }

    fn mis_standardize_array(
        &mut self,
        a: &Array,
        numbers: &[f64],
        texts: &[String],
        out: &mut Execution,
    ) -> Result<Array> {
        let n = a.len();
        let hit: Vec<usize> = match a {
            Array::Numeric(m) => (0..n).filter(|&i| numbers.contains(&m.data()[i])).collect(),
            Array::Typed(t) if !t.is_integer() => {
                (0..n).filter(|&i| numbers.contains(&t.real(i))).collect()
            }
            Array::Duration(d) => (0..n).filter(|&i| numbers.contains(&d.data()[i])).collect(),
            Array::Str(s) => (0..n)
                .filter(|&i| {
                    s.data()[i]
                        .as_deref()
                        .is_some_and(|x| texts.iter().any(|t| t == x))
                })
                .collect(),
            Array::Categorical(c) => (0..n)
                .filter(|&i| c.codes()[i] != 0 && texts.iter().any(|t| t == c.text(i)))
                .collect(),
            Array::Cell(c) => {
                let mut list = Vec::new();
                for i in 0..n {
                    if let Array::Character(ch) = c.get_linear(i)?.as_ref()
                        && texts
                            .iter()
                            .any(|t| *t == String::from_utf16_lossy(ch.units()))
                    {
                        list.push(i);
                    }
                }
                list
            }
            _ => Vec::new(),
        };
        if hit.is_empty() {
            return Ok(a.clone());
        }
        let Some(piece) = self.missing_piece(a, out)? else {
            return Ok(a.clone());
        };
        self.mis_assign(a.clone(), &hit, piece, out)
    }
}

/// ismissing inputs this module answers (others stay with the string module).
pub(super) fn claims(v: &Value) -> bool {
    matches!(
        v,
        Value::Container(
            Array::Table(_) | Array::Datetime(_) | Array::Duration(_) | Array::Categorical(_)
        )
    )
}
