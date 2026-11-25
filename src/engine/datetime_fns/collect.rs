//! Shape queries, conversions to text, and the collection functions of
//! datetime and duration arrays: sort, unique, min, max, mean, diff, sum,
//! issorted, isequal(n), reshape, repmat, cat, flips. NaT/NaN sort last
//! ascending and first descending; min/max omit them unless asked to
//! include them; mean propagates them unless asked to omit them.
use super::super::{Engine, Execution, Value};
use super::{
    CANCEL_STRIDE, CANCELLED, DATETIME_ND, DURATION_ND, calendar, datetime_of, display, dt_value,
    du_value, duration_of, is_dt, number_matrix, option_text, polled, polled_map, scalar_bool,
    text_elements,
};
use crate::datetime_array::DateTimeValue;
use crate::matrix::{MAX_ELEMENTS, MedianAxis};
use crate::syntax::Token;
use crate::{
    Array, CellArray, CharacterRow, DatetimeArray, DurationArray, Matrix, Result, StringArray,
};
use std::cmp::Ordering;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// A datetime or duration array seen as sortable keys.
#[derive(Clone)]
enum Keys {
    Dt(DatetimeArray),
    Du(DurationArray),
}

impl Keys {
    fn of(v: &Value) -> Option<Self> {
        match v {
            Value::Container(Array::Datetime(t)) => Some(Self::Dt(t.clone())),
            Value::Container(Array::Duration(d)) => Some(Self::Du(d.clone())),
            _ => None,
        }
    }
    fn shape(&self) -> (usize, usize) {
        match self {
            Self::Dt(t) => (t.rows(), t.cols()),
            Self::Du(d) => (d.rows(), d.cols()),
        }
    }
    fn len(&self) -> usize {
        let (r, c) = self.shape();
        r * c
    }
    fn missing(&self, i: usize) -> bool {
        match self {
            Self::Dt(t) => t.data()[i].is_nat(),
            Self::Du(d) => d.data()[i].is_nan(),
        }
    }
    /// Order of two non-missing elements.
    fn cmp(&self, i: usize, j: usize) -> Ordering {
        match self {
            Self::Dt(t) => t.data()[i].total_nanos().cmp(&t.data()[j].total_nanos()),
            Self::Du(d) => d.data()[i]
                .partial_cmp(&d.data()[j])
                .unwrap_or(Ordering::Equal),
        }
    }
    /// Elements at `positions` (usize::MAX = missing), new shape; polls
    /// `cancel` every 65536 elements.
    fn pick(
        &self,
        rows: usize,
        cols: usize,
        positions: &[usize],
        cancel: &AtomicBool,
    ) -> Result<Value> {
        match self {
            Self::Dt(t) => Ok(dt_value(t.with_data(
                rows,
                cols,
                polled_map(cancel, positions, |i| {
                    t.data().get(i).copied().unwrap_or(DateTimeValue::NAT)
                })?,
            )?)),
            Self::Du(d) => Ok(du_value(d.with_data(
                rows,
                cols,
                polled_map(cancel, positions, |i| {
                    d.data().get(i).copied().unwrap_or(f64::NAN)
                })?,
            )?)),
        }
    }
    /// The array read as one column (a reduction over all dimensions).
    fn as_column(&self) -> Result<Self> {
        let n = self.len();
        Ok(match self {
            Self::Dt(t) => Self::Dt(t.reshaped(n, 1)?),
            Self::Du(d) => Self::Du(d.reshaped(n, 1)?),
        })
    }
    /// The array itself (a reduction along a dimension above 2).
    fn value(&self) -> Value {
        match self {
            Self::Dt(t) => dt_value(t.clone()),
            Self::Du(d) => du_value(d.clone()),
        }
    }
    fn nd_error(&self) -> &'static str {
        match self {
            Self::Dt(_) => DATETIME_ND,
            Self::Du(_) => DURATION_ND,
        }
    }
}

/// Lanes of a 2-D array along `dim` (1 = down columns, 2 = along rows):
/// (lane count, lane length, linear index of element k of lane l).
type LaneIndex = Box<dyn Fn(usize, usize) -> usize>;

fn lanes(shape: (usize, usize), dim: usize) -> (usize, usize, LaneIndex) {
    let rows = shape.0;
    if dim == 1 {
        (shape.1, shape.0, Box::new(move |l, k| l * rows + k))
    } else {
        (shape.0, shape.1, Box::new(move |l, k| k * rows + l))
    }
}

fn first_dim(shape: (usize, usize)) -> usize {
    if shape.0 != 1 { 1 } else { 2 }
}

/// Shape with dimension `dim` set to `n`.
fn with_dim(shape: (usize, usize), dim: usize, n: usize) -> (usize, usize) {
    if dim == 1 { (n, shape.1) } else { (shape.0, n) }
}

/// Linear index of element k of lane l in an output of `shape`.
fn out_index(shape: (usize, usize), dim: usize, l: usize, k: usize) -> usize {
    if dim == 1 {
        l * shape.0 + k
    } else {
        k * shape.0 + l
    }
}

/// Axis of a reduction: one dimension (3 = any dimension a 2-D array has as
/// a singleton) or all of them.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Axis {
    Dim(usize),
    All,
}

/// Flags of min and max.
const EXTREME_FLAGS: &[&str] = &[
    "includenat",
    "includenan",
    "includemissing",
    "omitnat",
    "omitnan",
    "omitmissing",
    "linear",
];
/// Flags of mean and median.
const MEAN_FLAGS: &[&str] = &[
    "includenat",
    "includenan",
    "includemissing",
    "omitnat",
    "omitnan",
    "omitmissing",
];
/// Flags of sum (durations only).
const SUM_FLAGS: &[&str] = &["includenan", "includemissing", "omitnan", "omitmissing"];

/// The last of the include/omit flags decides; `include` is the default.
fn includes_missing(flags: &[String], include: bool) -> bool {
    flags.iter().fold(include, |acc, f| match f.as_str() {
        "includenat" | "includenan" | "includemissing" => true,
        "omitnat" | "omitnan" | "omitmissing" => false,
        _ => acc,
    })
}

/// The array a reduction along `axis` reads, with its dimension: 'all' reads
/// the array as one column, down that column.
fn along(keys: Keys, axis: Option<Axis>) -> Result<(Keys, Option<usize>)> {
    Ok(match axis {
        Some(Axis::All) => (keys.as_column()?, Some(1)),
        Some(Axis::Dim(d)) => (keys, Some(d)),
        None => (keys, None),
    })
}

/// Counts elements of a lane loop; every `CANCEL_STRIDE` of them it polls the
/// cancellation flag, so one huge lane stays cancellable.
#[derive(Default)]
struct Tick(usize);

impl Tick {
    fn poll(&mut self, engine: &Engine) -> Result<()> {
        self.0 += 1;
        engine.dtm_check(self.0)
    }
}

/// Work bound of repeated differences: the numeric diff limit.
const MAX_DIFF_WORK: u128 = 16 * MAX_ELEMENTS as u128;

/// Length of the runs sorted with the std stable sort before merging.
const SORT_RUN: usize = 8192;

impl Engine {
    /// Stable sort of `items` by `cmp` that polls for cancellation between
    /// runs and every `CANCEL_STRIDE` merged elements (std's sort cannot be
    /// interrupted). One scratch buffer of the same length.
    pub(in crate::engine) fn dtm_stable_sort(
        &self,
        items: &mut Vec<usize>,
        cmp: impl Fn(usize, usize) -> Ordering,
    ) -> Result<()> {
        let n = items.len();
        for run in items.chunks_mut(SORT_RUN) {
            self.check_cancel()?;
            run.sort_by(|&a, &b| cmp(a, b));
        }
        if n <= SORT_RUN {
            return Ok(());
        }
        let mut scratch = vec![0usize; n];
        let mut tick = Tick::default();
        let mut width = SORT_RUN;
        while width < n {
            let mut start = 0;
            while start < n {
                let mid = (start + width).min(n);
                let end = (start + 2 * width).min(n);
                let (mut i, mut j, mut o) = (start, mid, start);
                while i < mid && j < end {
                    tick.poll(self)?;
                    // Take from the right run only when strictly smaller: stable.
                    if cmp(items[j], items[i]) == Ordering::Less {
                        scratch[o] = items[j];
                        j += 1;
                    } else {
                        scratch[o] = items[i];
                        i += 1;
                    }
                    o += 1;
                }
                scratch[o..o + (mid - i)].copy_from_slice(&items[i..mid]);
                o += mid - i;
                scratch[o..o + (end - j)].copy_from_slice(&items[j..end]);
                start = end;
            }
            std::mem::swap(items, &mut scratch);
            width *= 2;
        }
        Ok(())
    }

    fn dtm_dim(&self, v: &Value, who: &str) -> Result<usize> {
        let (x, _) = self.dtm_numbers(v, who)?;
        match x.as_slice() {
            [d] if *d == 1.0 || *d == 2.0 => Ok(*d as usize),
            [d] if d.fract() == 0.0 && *d > 2.0 => Ok(3),
            _ => Err(format!(
                "{who}: dimension must be a positive integer scalar"
            )),
        }
    }

    /// A dimension or a vector of dimensions of a reduction, for a 2-D array.
    fn dtm_vecdim(&self, v: &Value, who: &str) -> Result<Axis> {
        let message = || {
            format!(
                "{who}: dimension must be a positive integer scalar or a vector of unique positive integers"
            )
        };
        let (x, _) = self.dtm_numbers(v, who).map_err(|_| message())?;
        if x.is_empty() || x.len() > 32 {
            return Err(message());
        }
        let mut dims = Vec::with_capacity(x.len());
        for d in x {
            if !(d.is_finite() && d >= 1.0 && d.fract() == 0.0 && d <= 1e9) {
                return Err(message());
            }
            dims.push(d as usize);
        }
        let mut unique = dims.clone();
        unique.sort_unstable();
        unique.dedup();
        if unique.len() != dims.len() {
            return Err(format!(
                "{who}: the vector of dimensions must not repeat a dimension"
            ));
        }
        Ok(match (dims.contains(&1), dims.contains(&2)) {
            (true, true) => Axis::All,
            (true, false) => Axis::Dim(1),
            (false, true) => Axis::Dim(2),
            // Only dimensions a 2-D array has as singletons.
            (false, false) => Axis::Dim(3),
        })
    }

    /// Options of min/max/mean/median/sum after the data argument (and the
    /// `[]` of min/max): an optional dimension, vector of dimensions or
    /// 'all' first, then flags from `allowed`. Anything else is an error
    /// that names it; nothing is ignored.
    fn dtm_reduce_options(
        &self,
        args: &[Value],
        who: &str,
        allowed: &[&str],
    ) -> Result<(Option<Axis>, Vec<String>)> {
        let mut axis = None;
        let mut flags: Vec<String> = Vec::new();
        for (k, a) in args.iter().enumerate() {
            match option_text(a, &self.cancel) {
                Some(t) => {
                    let lower = t.to_ascii_lowercase();
                    if lower == "all" && k == 0 {
                        axis = Some(Axis::All);
                    } else if allowed.contains(&lower.as_str()) {
                        flags.push(lower);
                    } else {
                        return Err(format!("{who}: invalid option '{t}'"));
                    }
                }
                None if k == 0 => axis = Some(self.dtm_vecdim(a, who)?),
                None => {
                    return Err(format!(
                        "{who}: invalid option of class {}; expected a dimension first, then text flags",
                        super::ops::class_of(a)
                    ));
                }
            }
        }
        Ok((axis, flags))
    }

    /// Text flags and an optional dimension after the data arguments.
    fn dtm_flags(&self, args: &[Value], who: &str) -> Result<(Option<usize>, Vec<String>)> {
        let mut dim = None;
        let mut flags = Vec::new();
        for a in args {
            if let Some(t) = option_text(a, &self.cancel) {
                flags.push(t.to_ascii_lowercase());
            } else if a.dims().iter().product::<usize>() == 0 {
                // the [] placeholder of min(t, [], dim)
            } else {
                dim = Some(self.dtm_dim(a, who)?);
            }
        }
        Ok((dim, flags))
    }

    pub(super) fn dtm_collection(
        &mut self,
        name: &str,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Option<Vec<Value>>> {
        let one = |v: Value| Ok(Some(vec![v]));
        let first = &args[0];
        let class = || {
            args.iter()
                .find(|v| is_dt(v))
                .and_then(|v| match v {
                    Value::Container(a) => Some(a.class()),
                    _ => None,
                })
                .unwrap_or("datetime")
        };
        let shape = first.dims();
        let shape = (shape[0], shape.get(1).copied().unwrap_or(1));
        match name {
            "class" => one(Value::Text(class().into())),
            "isa" => {
                let want = args
                    .get(1)
                    .and_then(|v| option_text(v, &self.cancel))
                    .unwrap_or_default();
                one(scalar_bool(is_dt(first) && want == class()))
            }
            "isnumeric" | "ischar" | "iscell" | "isstruct" | "islogical" | "isfloat"
            | "isinteger" | "isstring" | "is_function_handle" | "issparse" => {
                one(scalar_bool(false))
            }
            // MathWorks isreal: false for datetime, true for duration (real numeric storage)
            "isreal" => one(scalar_bool(duration_of(first).is_some())),
            "size" => {
                if args.len() == 2 {
                    let d = self.dtm_dim(&args[1], "size")?;
                    let n = match d {
                        1 => shape.0,
                        2 => shape.1,
                        _ => 1,
                    };
                    return one(number_matrix(1, 1, vec![n as f64])?);
                }
                if count <= 1 {
                    return one(number_matrix(1, 2, vec![shape.0 as f64, shape.1 as f64])?);
                }
                let mut outs = vec![
                    number_matrix(1, 1, vec![shape.0 as f64])?,
                    number_matrix(1, 1, vec![shape.1 as f64])?,
                ];
                while outs.len() < count {
                    outs.push(number_matrix(1, 1, vec![1.0])?);
                }
                Ok(Some(outs))
            }
            "numel" => one(number_matrix(1, 1, vec![(shape.0 * shape.1) as f64])?),
            "length" => one(number_matrix(
                1,
                1,
                vec![if shape.0 * shape.1 == 0 {
                    0.0
                } else {
                    shape.0.max(shape.1) as f64
                }],
            )?),
            "ndims" => one(number_matrix(1, 1, vec![2.0])?),
            "isempty" => one(scalar_bool(shape.0 * shape.1 == 0)),
            "isscalar" => one(scalar_bool(shape == (1, 1))),
            "isvector" => one(scalar_bool(shape.0 == 1 || shape.1 == 1)),
            "isrow" => one(scalar_bool(shape.0 == 1)),
            "iscolumn" => one(scalar_bool(shape.1 == 1)),
            "ismatrix" => one(scalar_bool(true)),
            "transpose" | "ctranspose" => one(self.dtm_transpose(first)?),
            "reshape" => one(self.dtm_reshape(args)?),
            "repmat" => one(self.dtm_repmat(args)?),
            "horzcat" => one(self.dtm_concat(vec![args.to_vec()])?),
            "vertcat" => one(self.dtm_concat(args.iter().map(|v| vec![v.clone()]).collect())?),
            "cat" => {
                let d = self.dtm_dim(first, "cat")?;
                match d {
                    1 => one(self.dtm_concat(args[1..].iter().map(|v| vec![v.clone()]).collect())?),
                    2 => one(self.dtm_concat(vec![args[1..].to_vec()])?),
                    _ => Err(if args[1..].iter().any(|v| datetime_of(v).is_some()) {
                        DATETIME_ND.into()
                    } else {
                        DURATION_ND.into()
                    }),
                }
            }
            "fliplr" | "flipud" => {
                let keys = Keys::of(first).ok_or_else(|| format!("{name}: unsupported input"))?;
                let (r, c) = keys.shape();
                let positions = polled(&self.cancel, r * c, |i| {
                    let (row, col) = (i % r, i / r);
                    if name == "fliplr" {
                        (c - 1 - col) * r + row
                    } else {
                        col * r + (r - 1 - row)
                    }
                })?;
                one(keys.pick(r, c, &positions, &self.cancel)?)
            }
            "char" | "string" | "cellstr" => one(self.dtm_text_out(name, args)?),
            "num2str" | "mat2str" | "int2str" => Err(format!(
                "{name}: {} input is not supported, use char or string",
                class()
            )),
            "double" | "single" => Err(match class() {
                "datetime" => {
                    format!("{name}: datetime input is not supported; use datenum or posixtime")
                }
                _ => format!(
                    "{name}: duration input is not supported; use seconds, minutes, hours or days"
                ),
            }),
            "abs" => match duration_of(first) {
                Some(d) => one(du_value(d.with_data(
                    d.rows(),
                    d.cols(),
                    polled_map(&self.cancel, d.data(), f64::abs)?,
                )?)),
                None => Err("abs: datetime input is not supported".into()),
            },
            "uminus" => one(self.dtm_unary(true, first.clone())?),
            "uplus" => one(self.dtm_unary(false, first.clone())?),
            "plus" | "minus" | "times" | "mtimes" | "rdivide" | "mrdivide" if args.len() == 2 => {
                let op = match name {
                    "plus" => Token::Plus,
                    "minus" => Token::Minus,
                    "times" => Token::DotStar,
                    "mtimes" => Token::Star,
                    "rdivide" => Token::DotSlash,
                    _ => Token::Slash,
                };
                one(self.dtm_binary(&op, &args[0], &args[1])?)
            }
            "eq" | "ne" | "lt" | "le" | "gt" | "ge" if args.len() == 2 => {
                let op = match name {
                    "eq" => Token::Equal,
                    "ne" => Token::NotEqual,
                    "lt" => Token::Less,
                    "le" => Token::LessEqual,
                    "gt" => Token::Greater,
                    _ => Token::GreaterEqual,
                };
                one(self.dtm_compare(&op, &args[0], &args[1])?)
            }
            "colon" if matches!(args.len(), 2 | 3) => {
                let (step, last) = if args.len() == 3 {
                    (Some(args[1].clone()), args[2].clone())
                } else {
                    (None, args[1].clone())
                };
                one(self.dtm_range(args[0].clone(), step, last)?)
            }
            "isequal" | "isequaln" => one(scalar_bool(self.dtm_isequal(args, name == "isequaln")?)),
            "sort" => self.dtm_sort(args, count).map(Some),
            "unique" => self.dtm_unique(args, count).map(Some),
            "min" | "max" => self.dtm_extreme(name, args, count).map(Some),
            "mean" => one(self.dtm_mean(args)?),
            "median" => one(self.dtm_median(args)?),
            "sum" => one(self.dtm_sum(args)?),
            "diff" => one(self.dtm_diff(args)?),
            "issorted" => one(self.dtm_issorted(args)?),
            "mode" | "ismember" | "linspace" | "dateshift" | "between" | "caldays"
            | "calmonths" | "calyears" | "std" | "var" | "cumsum" => {
                Err(format!("{name}: {} input is not supported yet", class()))
            }
            "plot" | "semilogx" | "semilogy" | "loglog" | "line" | "scatter" | "bar" | "area"
            | "stairs" | "stem" | "plot3" | "scatter3" | "xlim" | "ylim" | "zlim" | "xticks"
            | "yticks" | "zticks" | "set" => Ok(None),
            _ => {
                let _ = out;
                Err(format!("{name}: {} input is not supported", class()))
            }
        }
    }

    pub(super) fn dtm_size_args(&self, args: &[Value], who: &str) -> Result<Vec<Option<usize>>> {
        let mut sizes = Vec::new();
        for a in args {
            if a.dims().iter().product::<usize>() == 0 && args.len() > 1 {
                sizes.push(None);
                continue;
            }
            let (x, _) = self.dtm_numbers(a, who)?;
            for v in x {
                if !(v.is_finite() && v >= 0.0 && v.fract() == 0.0) {
                    return Err(format!(
                        "{who}: size arguments must be nonnegative integers"
                    ));
                }
                sizes.push(Some(v as usize));
            }
        }
        Ok(sizes)
    }

    fn dtm_reshape(&self, args: &[Value]) -> Result<Value> {
        let keys = Keys::of(&args[0]).ok_or("reshape: unsupported input")?;
        let mut sizes = self.dtm_size_args(&args[1..], "reshape")?;
        while sizes.len() > 2 && sizes.last() == Some(&Some(1)) {
            sizes.pop();
        }
        if sizes.len() != 2 {
            return Err(keys.nd_error().into());
        }
        let n = keys.len();
        let (r, c) = match (sizes[0], sizes[1]) {
            (Some(r), Some(c)) => (r, c),
            (None, Some(c)) if c > 0 && n % c == 0 => (n / c, c),
            (Some(r), None) if r > 0 && n % r == 0 => (r, n / r),
            _ => {
                return Err(
                    "reshape: the size is not compatible with the number of elements".into(),
                );
            }
        };
        match keys {
            Keys::Dt(t) => Ok(dt_value(t.reshaped(r, c)?)),
            Keys::Du(d) => Ok(du_value(d.reshaped(r, c)?)),
        }
    }

    fn dtm_repmat(&self, args: &[Value]) -> Result<Value> {
        let keys = Keys::of(&args[0]).ok_or("repmat: unsupported input")?;
        let sizes = self.dtm_size_args(&args[1..], "repmat")?;
        let (m, n) = match sizes.as_slice() {
            [Some(m)] => (*m, *m),
            [Some(m), Some(n)] => (*m, *n),
            [Some(m), Some(n), rest @ ..] if rest.iter().all(|s| *s == Some(1)) => (*m, *n),
            _ => return Err(keys.nd_error().into()),
        };
        let (r, c) = keys.shape();
        let (rows, cols) = (r.saturating_mul(m), c.saturating_mul(n));
        crate::datetime_array::checked_len(rows, cols, "repmat")?;
        let positions = polled(&self.cancel, rows * cols, |i| {
            (i / rows % c) * r + (i % rows) % r
        })?;
        keys.pick(rows, cols, &positions, &self.cancel)
    }

    fn dtm_text_out(&self, name: &str, args: &[Value]) -> Result<Value> {
        let array = match &args[0] {
            Value::Container(a @ (Array::Datetime(_) | Array::Duration(_))) => a,
            _ => return Err(format!("{name}: unsupported input")),
        };
        let format = match args.get(1) {
            Some(v) => Some(
                option_text(v, &self.cancel)
                    .ok_or_else(|| format!("{name}: the format must be text"))?,
            ),
            None => None,
        };
        if args.len() > 2 {
            return Err(format!("{name}: too many input arguments"));
        }
        let texts = display::element_texts(array, format.as_deref(), Some(&*self.cancel))?;
        let (r, c) = (array.rows(), array.cols());
        match name {
            "char" => self.dtm_char_rows(&texts),
            "string" => {
                // NaT and NaN become missing strings; char and cellstr keep
                // the 'NaT'/'NaN' text.
                let missing = |i: usize| match array {
                    Array::Datetime(t) => t.data()[i].is_nat(),
                    Array::Duration(d) => d.data()[i].is_nan(),
                    _ => false,
                };
                let data = super::polled(&self.cancel, texts.len(), |i| {
                    (!missing(i)).then(|| Arc::<str>::from(texts[i].as_str()))
                })?;
                Ok(Value::Container(Array::Str(StringArray::new(r, c, data)?)))
            }
            _ => {
                let mut slots = Vec::with_capacity(texts.len());
                for (i, t) in texts.iter().enumerate() {
                    self.dtm_check(i)?;
                    slots.push(Arc::new(Array::Character(CharacterRow::from_text(
                        t,
                        &self.cancel,
                    )?)));
                }
                Ok(Value::Container(Array::Cell(CellArray::from_shared(
                    r,
                    c,
                    slots,
                    &self.cancel,
                )?)))
            }
        }
    }

    fn dtm_isequal(&self, args: &[Value], nan_equal: bool) -> Result<bool> {
        if args.len() < 2 {
            return Err("Not enough input arguments.".into());
        }
        let like = args.iter().find_map(|v| datetime_of(v)).cloned();
        let mut converted = Vec::with_capacity(args.len());
        for v in args {
            match (&like, v) {
                (Some(t), v)
                    if datetime_of(v).is_none() && text_elements(v, &self.cancel)?.is_some() =>
                {
                    match self.dtm_text_as_datetime(v, t) {
                        Ok(Some(t)) => converted.push(dt_value(t)),
                        _ => return Ok(false),
                    }
                }
                _ => converted.push(v.clone()),
            }
        }
        let first = &converted[0];
        for other in &converted[1..] {
            if !equal_values(first, other, nan_equal, &self.cancel)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn dtm_sort(&self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        let keys = Keys::of(&args[0]).ok_or("sort: unsupported input")?;
        let (dim, flags) = self.dtm_flags(&args[1..], "sort")?;
        let descending = match flags.as_slice() {
            [] => false,
            [f] if f == "ascend" => false,
            [f] if f == "descend" => true,
            _ => return Err("sort: expected 'ascend' or 'descend'".into()),
        };
        let shape = keys.shape();
        let dim = dim.unwrap_or_else(|| first_dim(shape));
        if dim > 2 {
            return Ok(vec![
                keys.value(),
                number_matrix(shape.0, shape.1, vec![1.0; keys.len()])?,
            ]);
        }
        let (count_lanes, len, at) = lanes(shape, dim);
        let mut positions = vec![0; keys.len()];
        let mut order_out = vec![0.0; keys.len()];
        let mut tick = Tick::default();
        for l in 0..count_lanes {
            tick.poll(self)?;
            let mut order: Vec<usize> = (0..len).collect();
            self.dtm_stable_sort(&mut order, |a, b| {
                let (i, j) = (at(l, a), at(l, b));
                match (keys.missing(i), keys.missing(j)) {
                    (true, true) => Ordering::Equal,
                    (true, false) => {
                        if descending {
                            Ordering::Less
                        } else {
                            Ordering::Greater
                        }
                    }
                    (false, true) => {
                        if descending {
                            Ordering::Greater
                        } else {
                            Ordering::Less
                        }
                    }
                    _ if descending => keys.cmp(j, i),
                    _ => keys.cmp(i, j),
                }
            })?;
            for (k, &src) in order.iter().enumerate() {
                tick.poll(self)?;
                let o = out_index(shape, dim, l, k);
                positions[o] = at(l, src);
                order_out[o] = (src + 1) as f64;
            }
        }
        let mut outs = vec![keys.pick(shape.0, shape.1, &positions, &self.cancel)?];
        if count > 1 {
            outs.push(number_matrix(shape.0, shape.1, order_out)?);
        }
        Ok(outs)
    }

    fn dtm_unique(&self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        let keys = Keys::of(&args[0]).ok_or("unique: unsupported input")?;
        let (_, flags) = self.dtm_flags(&args[1..], "unique")?;
        if flags.iter().any(|f| f != "sorted") {
            return Err(
                "unique: only the 'sorted' option is supported for datetime and duration".into(),
            );
        }
        let n = keys.len();
        let mut order: Vec<usize> = (0..n).collect();
        self.dtm_stable_sort(&mut order, |i, j| {
            match (keys.missing(i), keys.missing(j)) {
                (true, true) => i.cmp(&j),
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                _ => keys.cmp(i, j).then(i.cmp(&j)),
            }
        })?;
        let mut picks: Vec<usize> = Vec::new();
        let mut ic = vec![0.0; n];
        let mut tick = Tick::default();
        for &i in &order {
            tick.poll(self)?;
            let same = picks.last().is_some_and(|&p| {
                !keys.missing(i) && !keys.missing(p) && keys.cmp(p, i) == Ordering::Equal
            });
            if !same {
                picks.push(i);
            }
            ic[i] = picks.len() as f64;
        }
        let row = keys.shape().0 == 1;
        let m = picks.len();
        let (r, c) = if row { (1, m) } else { (m, 1) };
        let mut outs = vec![keys.pick(r, c, &picks, &self.cancel)?];
        if count > 1 {
            outs.push(number_matrix(
                m,
                1,
                polled_map(&self.cancel, &picks, |p| (p + 1) as f64)?,
            )?);
        }
        if count > 2 {
            outs.push(number_matrix(n, 1, ic)?);
        }
        Ok(outs)
    }

    fn dtm_extreme(&self, name: &str, args: &[Value], count: usize) -> Result<Vec<Value>> {
        let is_max = name == "max";
        let better = |o: Ordering| {
            if is_max {
                o == Ordering::Greater
            } else {
                o == Ordering::Less
            }
        };
        // Two-array form: min(a, b).
        if args.len() >= 2
            && args[1].dims().iter().product::<usize>() != 0
            && option_text(&args[1], &self.cancel).is_none()
        {
            if count > 1 {
                return Err(format!(
                    "{name}: two outputs are not supported with two input arrays"
                ));
            }
            let flags: &[&str] = &EXTREME_FLAGS[..6];
            let mut include = false;
            for v in &args[2..] {
                match option_text(v, &self.cancel) {
                    Some(t) if flags.contains(&t.to_ascii_lowercase().as_str()) => {
                        include = includes_missing(&[t.to_ascii_lowercase()], include);
                    }
                    Some(t) => return Err(format!("{name}: invalid option '{t}'")),
                    None => {
                        return Err(format!(
                            "{name}: a dimension cannot be given with two input arrays"
                        ));
                    }
                }
            }
            let (a, b) = match (&args[0], &args[1]) {
                (x, y) if datetime_of(x).is_some() && datetime_of(y).is_none() => {
                    let t = datetime_of(x).unwrap();
                    (
                        x.clone(),
                        self.dtm_text_as_datetime(y, t)?
                            .map(dt_value)
                            .unwrap_or_else(|| y.clone()),
                    )
                }
                (x, y) if datetime_of(y).is_some() && datetime_of(x).is_none() => {
                    let t = datetime_of(y).unwrap();
                    (
                        self.dtm_text_as_datetime(x, t)?
                            .map(dt_value)
                            .unwrap_or_else(|| x.clone()),
                        y.clone(),
                    )
                }
                (x, y) => (x.clone(), y.clone()),
            };
            let (Some(ka), Some(kb)) = (Keys::of(&a), Keys::of(&b)) else {
                return Err(format!(
                    "{name}: both inputs must be datetimes or both durations"
                ));
            };
            let shape = super::broadcast(ka.shape(), kb.shape(), name)?;
            let mut data_dt = Vec::new();
            let mut data_du = Vec::new();
            for i in 0..shape.0 * shape.1 {
                self.dtm_check(i)?;
                let ia = super::expand_index(i, shape.0, ka.shape());
                let ib = super::expand_index(i, shape.0, kb.shape());
                match (&ka, &kb) {
                    (Keys::Dt(p), Keys::Dt(q)) => {
                        if p.zone() != q.zone() {
                            return Err(format!(
                                "{name}: cannot compare datetimes with and without a time zone"
                            ));
                        }
                        let (u, v) = (p.data()[ia], q.data()[ib]);
                        data_dt.push(match (u.is_nat(), v.is_nat()) {
                            (true, true) => u,
                            (true, false) => {
                                if include {
                                    u
                                } else {
                                    v
                                }
                            }
                            (false, true) => {
                                if include {
                                    v
                                } else {
                                    u
                                }
                            }
                            _ => {
                                if better(v.total_nanos().cmp(&u.total_nanos())) {
                                    v
                                } else {
                                    u
                                }
                            }
                        });
                    }
                    (Keys::Du(p), Keys::Du(q)) => {
                        let (u, v) = (p.data()[ia], q.data()[ib]);
                        data_du.push(match (u.is_nan(), v.is_nan()) {
                            (true, true) => u,
                            (true, false) => {
                                if include {
                                    u
                                } else {
                                    v
                                }
                            }
                            (false, true) => {
                                if include {
                                    v
                                } else {
                                    u
                                }
                            }
                            _ => {
                                if better(v.partial_cmp(&u).unwrap_or(Ordering::Equal)) {
                                    v
                                } else {
                                    u
                                }
                            }
                        });
                    }
                    _ => {
                        return Err(format!(
                            "{name}: both inputs must be datetimes or both durations"
                        ));
                    }
                }
            }
            return Ok(vec![match ka {
                Keys::Dt(t) => dt_value(t.with_data(shape.0, shape.1, data_dt)?),
                Keys::Du(d) => du_value(d.with_data(shape.0, shape.1, data_du)?),
            }]);
        }
        let keys = Keys::of(&args[0]).ok_or_else(|| format!("{name}: unsupported input"))?;
        // min(t, [], ...): the [] placeholder before a dimension or flags.
        let rest = match args.get(1) {
            Some(v)
                if v.dims().iter().product::<usize>() == 0
                    && !matches!(v, Value::Text(_) | Value::QuotedString(_)) =>
            {
                &args[2..]
            }
            Some(v) if option_text(v, &self.cancel).is_some() => &args[1..],
            Some(_) => unreachable!("the two-array form is handled above"),
            None => &[][..],
        };
        let (axis, flags) = self.dtm_reduce_options(rest, name, EXTREME_FLAGS)?;
        let include = includes_missing(&flags, false);
        let linear = flags.iter().any(|f| f == "linear");
        let all = axis == Some(Axis::All);
        let keys = if all { keys.as_column()? } else { keys };
        let dim = match axis {
            Some(Axis::Dim(d)) => Some(d),
            Some(Axis::All) => Some(1),
            None => None,
        };
        let shape = keys.shape();
        if keys.len() == 0 && (shape == (0, 0) || all) {
            let empty = keys.pick(0, 0, &[], &self.cancel)?;
            return Ok(vec![empty, number_matrix(0, 0, vec![])?]
                .into_iter()
                .take(count.max(1))
                .collect());
        }
        let dim = dim.unwrap_or_else(|| first_dim(shape));
        if dim > 2 {
            let index = if linear {
                polled(&self.cancel, keys.len(), |i| (i + 1) as f64)?
            } else {
                vec![1.0; keys.len()]
            };
            return Ok(vec![keys.value(), number_matrix(shape.0, shape.1, index)?]
                .into_iter()
                .take(count.max(1))
                .collect());
        }
        let (count_lanes, len, at) = lanes(shape, dim);
        let out_shape = with_dim(shape, dim, if len == 0 { 0 } else { 1 });
        let mut picks = Vec::with_capacity(count_lanes);
        let mut index = Vec::with_capacity(count_lanes);
        if len > 0 {
            let mut tick = Tick::default();
            for l in 0..count_lanes {
                let mut best: Option<usize> = None;
                let mut first_missing = None;
                for k in 0..len {
                    tick.poll(self)?;
                    let i = at(l, k);
                    if keys.missing(i) {
                        first_missing.get_or_insert(k);
                        continue;
                    }
                    match best {
                        None => best = Some(k),
                        Some(b) if better(keys.cmp(i, at(l, b))) => best = Some(k),
                        _ => {}
                    }
                }
                let chosen = match (include, first_missing, best) {
                    (true, Some(k), _) => k,
                    (_, _, Some(k)) => k,
                    (_, Some(k), None) => k,
                    (_, None, None) => 0,
                };
                picks.push(at(l, chosen));
                // Position in the lane; 'linear' (and 'all', where the lane is
                // the whole array) gives the linear index.
                index.push(if linear {
                    (at(l, chosen) + 1) as f64
                } else {
                    (chosen + 1) as f64
                });
            }
        }
        let mut outs = vec![keys.pick(out_shape.0, out_shape.1, &picks, &self.cancel)?];
        if count > 1 {
            outs.push(number_matrix(out_shape.0, out_shape.1, index)?);
        }
        Ok(outs)
    }

    fn dtm_mean(&self, args: &[Value]) -> Result<Value> {
        let keys = Keys::of(&args[0]).ok_or("mean: unsupported input")?;
        let (axis, flags) = self.dtm_reduce_options(&args[1..], "mean", MEAN_FLAGS)?;
        let omit = !includes_missing(&flags, true);
        let (keys, dim) = along(keys, axis)?;
        let shape = keys.shape();
        let dim = dim.unwrap_or_else(|| first_dim(shape));
        if dim > 2 {
            return Ok(keys.value());
        }
        let (count_lanes, len, at) = lanes(shape, dim);
        let out_shape = with_dim(shape, dim, 1);
        let out_shape = if shape == (0, 0) { (1, 1) } else { out_shape };
        let lanes_out = if shape == (0, 0) { 1 } else { count_lanes };
        match &keys {
            Keys::Dt(t) => {
                let mut data = Vec::with_capacity(lanes_out);
                let mut tick = Tick::default();
                for l in 0..lanes_out {
                    let mut sum: i128 = 0;
                    let mut n: i128 = 0;
                    let mut nat = false;
                    for k in 0..len {
                        tick.poll(self)?;
                        match t.data()[at(l, k)].total_nanos() {
                            Some(v) => {
                                sum += v;
                                n += 1;
                            }
                            None => nat = true,
                        }
                    }
                    data.push(if n == 0 || (nat && !omit) {
                        DateTimeValue::NAT
                    } else {
                        // Round half away from zero.
                        let q = sum.div_euclid(n);
                        let r = sum.rem_euclid(n);
                        calendar::from_total(if 2 * r >= n { q + 1 } else { q })?
                    });
                }
                Ok(dt_value(t.with_data(out_shape.0, out_shape.1, data)?))
            }
            Keys::Du(d) => {
                let mut data = Vec::with_capacity(lanes_out);
                let mut tick = Tick::default();
                for l in 0..lanes_out {
                    // Started at -0.0 like std's float sum.
                    let (mut sum, mut n) = (-0.0f64, 0usize);
                    for k in 0..len {
                        tick.poll(self)?;
                        let v = d.data()[at(l, k)];
                        if !(omit && v.is_nan()) {
                            sum += v;
                            n += 1;
                        }
                    }
                    data.push(if n == 0 { f64::NAN } else { sum / n as f64 });
                }
                Ok(du_value(d.with_data(out_shape.0, out_shape.1, data)?))
            }
        }
    }

    /// median: NaT/NaN propagate unless omitted; an even count averages the
    /// two middle values (datetimes to the nearest nanosecond, halves up).
    fn dtm_median(&self, args: &[Value]) -> Result<Value> {
        let keys = Keys::of(&args[0]).ok_or("median: unsupported input")?;
        let (axis, flags) = self.dtm_reduce_options(&args[1..], "median", MEAN_FLAGS)?;
        let omit = !includes_missing(&flags, true);
        if let Keys::Du(d) = &keys {
            // The numeric median kernel on the milliseconds.
            let m = Matrix::from_column_vec(d.rows(), d.cols(), d.data().to_vec())?;
            let axis = match axis {
                None => MedianAxis::Default,
                Some(Axis::All) => MedianAxis::All,
                Some(Axis::Dim(k)) => MedianAxis::Dimension(k),
            };
            let r = m.median(axis, omit, &self.cancel)?;
            return Ok(du_value(d.with_data(
                r.rows(),
                r.cols(),
                r.data().to_vec(),
            )?));
        }
        let (keys, dim) = along(keys, axis)?;
        let Keys::Dt(t) = &keys else {
            unreachable!("durations are handled above");
        };
        let shape = (t.rows(), t.cols());
        let dim = dim.unwrap_or_else(|| first_dim(shape));
        if dim > 2 {
            return Ok(keys.value());
        }
        let (count_lanes, len, at) = lanes(shape, dim);
        let (out_shape, lanes_out) = if shape == (0, 0) {
            ((1, 1), 1)
        } else {
            (with_dim(shape, dim, 1), count_lanes)
        };
        let mut data = Vec::with_capacity(lanes_out);
        let mut lane: Vec<i128> = Vec::with_capacity(len);
        let mut tick = Tick::default();
        for l in 0..lanes_out {
            lane.clear();
            let mut nat = false;
            for k in 0..len {
                tick.poll(self)?;
                match t.data()[at(l, k)].total_nanos() {
                    Some(v) => lane.push(v),
                    None => nat = true,
                }
            }
            if lane.is_empty() || (nat && !omit) {
                data.push(DateTimeValue::NAT);
                continue;
            }
            let n = lane.len();
            let mid = n / 2;
            let (lower, &mut upper, _) = lane.select_nth_unstable(mid);
            self.check_cancel()?;
            let value = if n % 2 == 1 {
                upper
            } else {
                // The largest of the lower half is the other middle value.
                let mut below = i128::MIN;
                for &v in lower.iter() {
                    tick.poll(self)?;
                    below = below.max(v);
                }
                let sum = below + upper;
                sum.div_euclid(2) + sum.rem_euclid(2)
            };
            data.push(calendar::from_total(value)?);
        }
        Ok(dt_value(t.with_data(out_shape.0, out_shape.1, data)?))
    }

    fn dtm_sum(&self, args: &[Value]) -> Result<Value> {
        let Some(d) = duration_of(&args[0]) else {
            return Err(
                "sum: datetime values cannot be added; subtract them to get durations".into(),
            );
        };
        let (axis, flags) = self.dtm_reduce_options(&args[1..], "sum", SUM_FLAGS)?;
        let omit = !includes_missing(&flags, true);
        let (Keys::Du(d), dim) = along(Keys::Du(d.clone()), axis)? else {
            unreachable!("a duration stays a duration");
        };
        let shape = (d.rows(), d.cols());
        let dim = dim.unwrap_or_else(|| first_dim(shape));
        if dim > 2 {
            return Ok(du_value(d));
        }
        let (count_lanes, len, at) = lanes(shape, dim);
        let (out_shape, lanes_out) = if shape == (0, 0) {
            ((1, 1), 1)
        } else {
            (with_dim(shape, dim, 1), count_lanes)
        };
        let mut data = Vec::with_capacity(lanes_out);
        let mut tick = Tick::default();
        for l in 0..lanes_out {
            // Started at -0.0 like std's float sum.
            let mut sum = -0.0f64;
            for k in 0..len {
                tick.poll(self)?;
                let v = d.data()[at(l, k)];
                if !(omit && v.is_nan()) {
                    sum += v;
                }
            }
            data.push(sum);
        }
        Ok(du_value(d.with_data(out_shape.0, out_shape.1, data)?))
    }

    fn dtm_diff(&self, args: &[Value]) -> Result<Value> {
        let keys = Keys::of(&args[0]).ok_or("diff: unsupported input")?;
        let order = match args.get(1) {
            Some(v) => {
                let (x, _) = self.dtm_numbers(v, "diff")?;
                match x.as_slice() {
                    // [] means the default order, like numeric diff.
                    [] => 1,
                    [n] if n.fract() == 0.0 && *n >= 0.0 && *n <= MAX_ELEMENTS as f64 => {
                        *n as usize
                    }
                    _ => {
                        return Err(format!(
                            "diff: the order must be a nonnegative integer <= {MAX_ELEMENTS}"
                        ));
                    }
                }
            }
            None => 1,
        };
        let shape = keys.shape();
        let dim = match args.get(2) {
            Some(v) => self.dtm_dim(v, "diff")?,
            None => first_dim(shape),
        };
        if order == 0 {
            return Ok(match keys {
                Keys::Dt(t) => dt_value(t),
                Keys::Du(d) => du_value(d),
            });
        }
        if dim > 2 {
            return Err(keys.nd_error().into());
        }
        let (count_lanes, len, at) = lanes(shape, dim);
        let remaining = len.saturating_sub(order);
        let out_shape = with_dim(shape, dim, remaining);
        let format = match &keys {
            Keys::Du(d) => d.format().to_string(),
            Keys::Dt(_) => super::dur_text::DEFAULT.to_string(),
        };
        // An order >= the length gives the empty result at constant cost.
        if remaining == 0 || count_lanes == 0 {
            return Ok(du_value(DurationArray::new(
                out_shape.0,
                out_shape.1,
                Vec::new(),
                format,
            )?));
        }
        // Same work bound as numeric diff: lanes * (order*len - order*(order+1)/2).
        let n = order as u128;
        let work = count_lanes as u128 * (n * len as u128 - n * (n + 1) / 2);
        if work > MAX_DIFF_WORK {
            return Err(format!(
                "diff: repeated differences exceed the {MAX_DIFF_WORK}-subtraction work limit"
            ));
        }
        // One lane buffer is differenced in place, order times; no allocation
        // per step.
        let mut data = vec![0.0; out_shape.0 * out_shape.1];
        let mut lane = vec![0.0; len];
        let mut tick = Tick::default();
        for l in 0..count_lanes {
            tick.poll(self)?;
            for (k, slot) in lane.iter_mut().enumerate().take(len - 1) {
                tick.poll(self)?;
                let (i, j) = (at(l, k), at(l, k + 1));
                *slot = match &keys {
                    Keys::Dt(t) => match (t.data()[j].total_nanos(), t.data()[i].total_nanos()) {
                        (Some(b), Some(a)) => calendar::nanos_to_ms(b - a),
                        _ => f64::NAN,
                    },
                    Keys::Du(d) => d.data()[j] - d.data()[i],
                };
            }
            let mut current = len - 1;
            for _ in 1..order {
                for k in 0..current - 1 {
                    tick.poll(self)?;
                    lane[k] = lane[k + 1] - lane[k];
                }
                current -= 1;
            }
            for (k, &v) in lane[..remaining].iter().enumerate() {
                data[out_index(out_shape, dim, l, k)] = v;
            }
        }
        Ok(du_value(DurationArray::new(
            out_shape.0,
            out_shape.1,
            data,
            format,
        )?))
    }

    /// issorted(t) / issorted(t, 'ascend'|'descend'): NaT/NaN last when
    /// ascending, first when descending; equal neighbours are allowed.
    fn dtm_issorted(&self, args: &[Value]) -> Result<Value> {
        let keys = Keys::of(&args[0]).ok_or("issorted: unsupported input")?;
        let descending = match &args[1..] {
            [] => false,
            [v] => match option_text(v, &self.cancel)
                .map(|t| t.to_ascii_lowercase())
                .as_deref()
            {
                Some("ascend") => false,
                Some("descend") => true,
                _ => return Err("issorted: expected 'ascend' or 'descend'".into()),
            },
            _ => return Err("issorted: expected 'ascend' or 'descend'".into()),
        };
        let shape = keys.shape();
        let (count_lanes, len, at) = if shape.0 == 1 {
            lanes(shape, 2)
        } else {
            lanes(shape, 1)
        };
        let mut tick = Tick::default();
        for l in 0..count_lanes {
            tick.poll(self)?;
            for k in 1..len {
                tick.poll(self)?;
                let (i, j) = (at(l, k - 1), at(l, k));
                let ordered = match (keys.missing(i), keys.missing(j), descending) {
                    (true, false, false) | (false, true, true) => false,
                    (_, true, false) | (true, _, true) => true,
                    (false, false, false) => keys.cmp(i, j) != Ordering::Greater,
                    (false, false, true) => keys.cmp(i, j) != Ordering::Less,
                };
                if !ordered {
                    return Ok(scalar_bool(false));
                }
            }
        }
        Ok(scalar_bool(true))
    }
}

/// isequal / isequaln of two values where one may be a datetime/duration.
fn equal_values(a: &Value, b: &Value, nan_equal: bool, cancel: &AtomicBool) -> Result<bool> {
    match (a, b) {
        (Value::Container(p), Value::Container(q)) => {
            Ok(arrays_equal(p, q, nan_equal, cancel)?.unwrap_or(false))
        }
        _ => Ok(false),
    }
}

/// isequal / isequaln of two datetime or two duration arrays: same shape,
/// same time zone, equal elements (NaT/NaN equal only for isequaln). None
/// when the pair is not two datetimes or two durations. Polls `cancel`
/// every 65536 elements.
pub(crate) fn arrays_equal(
    a: &Array,
    b: &Array,
    nan_equal: bool,
    cancel: &AtomicBool,
) -> Result<Option<bool>> {
    let all = |n: usize, eq: &dyn Fn(usize) -> bool| -> Result<bool> {
        for start in (0..n).step_by(CANCEL_STRIDE) {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(CANCELLED.into());
            }
            if !(start..(start + CANCEL_STRIDE).min(n)).all(eq) {
                return Ok(false);
            }
        }
        Ok(true)
    };
    match (a, b) {
        (Array::Datetime(p), Array::Datetime(q)) => Ok(Some(
            (p.rows(), p.cols()) == (q.rows(), q.cols())
                && p.zone() == q.zone()
                && all(p.len(), &|i| {
                    let (u, v) = (p.data()[i], q.data()[i]);
                    if u.is_nat() || v.is_nat() {
                        nan_equal && u.is_nat() && v.is_nat()
                    } else {
                        u == v
                    }
                })?,
        )),
        (Array::Duration(p), Array::Duration(q)) if p.is_calendar() || q.is_calendar() => Ok(Some(
            p.is_calendar()
                && q.is_calendar()
                && (p.rows(), p.cols()) == (q.rows(), q.cols())
                && all(p.len(), &|i| {
                    p.calendar_data().unwrap()[i]
                        .iter()
                        .zip(q.calendar_data().unwrap()[i])
                        .all(|(u, v)| *u == v || (nan_equal && u.is_nan() && v.is_nan()))
                })?,
        )),
        (Array::Duration(p), Array::Duration(q)) => Ok(Some(
            (p.rows(), p.cols()) == (q.rows(), q.cols())
                && all(p.len(), &|i| {
                    let (u, v) = (p.data()[i], q.data()[i]);
                    u == v || (nan_equal && u.is_nan() && v.is_nan())
                })?,
        )),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::sync::atomic::Ordering as AtomicOrdering;

    /// Pseudo-random keys with many ties.
    fn key(i: usize) -> usize {
        i.wrapping_mul(2_654_435_761) % 100_003
    }

    #[test]
    fn stable_sort_matches_std_with_ties() {
        let e = Engine::default();
        for n in [0, 1, 5, SORT_RUN, SORT_RUN + 1, 3 * SORT_RUN + 17, 100_000] {
            let mut items: Vec<usize> = (0..n).collect();
            e.dtm_stable_sort(&mut items, |a, b| (key(a) % 13).cmp(&(key(b) % 13)))
                .unwrap();
            let mut expected: Vec<usize> = (0..n).collect();
            expected.sort_by_key(|&a| key(a) % 13);
            assert_eq!(items, expected, "n = {n}");
        }
    }

    #[test]
    fn sort_stops_within_one_stride_of_a_cancellation() {
        let e = Engine::default();
        let stop = e.cancellation();
        let n = 1_000_000;
        // Comparisons of a complete sort.
        let calls = Cell::new(0usize);
        let mut items: Vec<usize> = (0..n).collect();
        e.dtm_stable_sort(&mut items, |a, b| {
            calls.set(calls.get() + 1);
            key(a).cmp(&key(b))
        })
        .unwrap();
        let total = calls.get();
        // Cancel in the merge phase (the last third of the comparisons).
        for trigger in [total / 100, total - total / 3] {
            calls.set(0);
            let mut items: Vec<usize> = (0..n).collect();
            let error = e
                .dtm_stable_sort(&mut items, |a, b| {
                    calls.set(calls.get() + 1);
                    if calls.get() == trigger {
                        stop.store(true, AtomicOrdering::Relaxed);
                    }
                    key(a).cmp(&key(b))
                })
                .unwrap_err();
            stop.store(false, AtomicOrdering::Relaxed);
            assert!(error.contains("cancelled"), "{error}");
            // One run of the std sort or one merge stride after the request.
            assert!(
                calls.get() <= trigger + 2 * SORT_RUN * 14 + super::super::CANCEL_STRIDE,
                "{} comparisons after a request at {trigger} of {total}",
                calls.get()
            );
            assert!(calls.get() < total, "ran to completion");
        }
    }
}
