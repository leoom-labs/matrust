//! Additional array functions. Corpus: tests/fixtures/fn/arrays-expected.txt.
use super::{
    Engine, Value,
    stats_more::{arx_moving_options, arx_window_kept, arx_window_missing, arx_window_range},
};
use crate::{
    Array, CellArray, CharacterRow, Complex64, ComplexMatrix, LogicalMatrix, Matrix, NumericClass,
    Result, TypedArray,
};
use std::cmp::Ordering;
use std::collections::VecDeque;

const ARX_NAMES: &[&str] = &[
    "movmax",
    "movmin",
    "movmedian",
    "movstd",
    "movvar",
    "repelem",
    "isequaln",
    "isdiag",
    "istril",
    "istriu",
    "isbanded",
    "issymmetric",
    "ishermitian",
    "bandwidth",
    "uniquetol",
    "ismembertol",
    "histcounts2",
];
const ARX_WINDOW_ERROR: &str = "Window length must be a finite scalar positive integer or 2-element vector of finite nonnegative integers.";
const ARX_TOL_ERROR: &str = "Tolerance value TOL must be a real, full (non-sparse), positive numeric scalar of type single or double.";
const ARX_FLOAT_ERROR: &str = "Input A must be a real full matrix of type single or double.";

pub(super) fn arx_has_outputs(name: &str) -> bool {
    matches!(
        name,
        "bandwidth" | "uniquetol" | "ismembertol" | "histcounts2"
    )
}

fn arx_text(value: &Value) -> Option<String> {
    match value {
        Value::Text(s) => Some(s.clone()),
        Value::Container(Array::Character(c)) if c.rows() <= 1 => {
            Some(String::from_utf16_lossy(c.units()))
        }
        _ => None,
    }
}
fn arx_bool(value: bool) -> Value {
    Value::Logical(LogicalMatrix::scalar(value))
}
fn arx_scalar(value: f64) -> Value {
    Value::Matrix(Matrix::scalar(value))
}
fn arx_value(array: Array) -> Value {
    match array {
        Array::Numeric(m) => Value::Matrix(m),
        Array::Logical(m) => Value::Logical(m),
        Array::Complex(m) => Value::Complex(m),
        a => Value::Container(a),
    }
}
fn arx_number(a: &Array, i: usize) -> Complex64 {
    match a {
        Array::Numeric(m) => Complex64::new(m.data()[i], 0.),
        Array::Logical(m) => Complex64::new(f64::from(m.data()[i]), 0.),
        Array::Typed(m) => Complex64::new(m.real(i), 0.),
        Array::Complex(m) => m.data()[i],
        Array::Character(m) => Complex64::new(f64::from(m.units()[i]), 0.),
        _ => unreachable!("validated numeric input"),
    }
}
fn arx_is_numeric(a: &Array) -> bool {
    matches!(
        a,
        Array::Numeric(_) | Array::Logical(_) | Array::Complex(_) | Array::Typed(_)
    )
}
fn arx_is_float(a: &Array) -> bool {
    matches!(a, Array::Numeric(_))
        || matches!(a, Array::Typed(t) if t.class() == NumericClass::Single)
}

/// Integer ordering and selection never pass through double (including uint64).
#[derive(Clone, Copy, Debug)]
enum ArxNumber {
    Real(f64),
    Int(i128),
}
impl ArxNumber {
    fn arx_real(self) -> f64 {
        match self {
            Self::Real(x) => x,
            Self::Int(x) => x as f64,
        }
    }
    fn arx_nan(self) -> bool {
        matches!(self, Self::Real(x) if x.is_nan())
    }
    fn arx_average(self, other: Self) -> Self {
        match (self, other) {
            (Self::Int(a), Self::Int(b)) => {
                let sum = a + b;
                Self::Int(sum / 2 + if sum % 2 == 0 { 0 } else { sum.signum() })
            }
            _ => Self::Real(self.arx_real().midpoint(other.arx_real())),
        }
    }
}
impl PartialEq for ArxNumber {
    fn eq(&self, b: &Self) -> bool {
        self.cmp(b) == Ordering::Equal
    }
}
impl Eq for ArxNumber {}
impl PartialOrd for ArxNumber {
    fn partial_cmp(&self, b: &Self) -> Option<Ordering> {
        Some(self.cmp(b))
    }
}
impl Ord for ArxNumber {
    fn cmp(&self, b: &Self) -> Ordering {
        match (*self, *b) {
            (Self::Int(a), Self::Int(b)) => a.cmp(&b),
            _ => {
                let (a, b) = (self.arx_real(), b.arx_real());
                if a == b {
                    Ordering::Equal
                } else {
                    a.total_cmp(&b)
                }
            }
        }
    }
}
fn arx_key(a: &Array, i: usize) -> ArxNumber {
    match a {
        Array::Typed(t) if t.is_integer() => ArxNumber::Int(t.int(i)),
        _ => ArxNumber::Real(arx_number(a, i).re),
    }
}

impl Engine {
    pub(super) fn arx_call(&self, name: &str, args: &[Value]) -> Result<Option<Value>> {
        if !ARX_NAMES.contains(&name) {
            return Ok(None);
        }
        Ok(Some(self.arx_outputs(name, args, 1)?.remove(0)))
    }
    pub(super) fn arx_outputs(
        &self,
        name: &str,
        args: &[Value],
        count: usize,
    ) -> Result<Vec<Value>> {
        self.check_cancel()?;
        let max = match name {
            "uniquetol" => 3,
            "histcounts2" => 5,
            "bandwidth" | "ismembertol" => 2,
            _ => 1,
        };
        if count > max {
            return Err("Too many output arguments.".into());
        }
        let mut result = match name {
            "movmax" | "movmin" | "movmedian" | "movstd" | "movvar" => {
                vec![self.arx_moving(name, args)?]
            }
            "repelem" => vec![self.arx_repelem(args)?],
            "isequaln" => vec![self.arx_isequaln(args)?],
            "isdiag" | "istril" | "istriu" | "isbanded" | "issymmetric" | "ishermitian"
            | "bandwidth" => self.arx_structure(name, args)?,
            "uniquetol" | "ismembertol" => self.arx_tolerance(name, args, count)?,
            "histcounts2" => self.arx_histcounts2(args, count)?,
            _ => unreachable!("registered array function"),
        };
        result.truncate(count);
        Ok(result)
    }
    fn arx_poll(&self, i: usize) -> Result<()> {
        if i.is_multiple_of(4096) {
            #[cfg(test)]
            tests::arx_cancel_after_progress(self, i);
            self.check_cancel()?;
        }
        Ok(())
    }
    fn arx_budget(&self, elements: usize, buffers: usize) -> Result<()> {
        if self
            .evaluated_elements
            .checked_add(elements.saturating_mul(buffers))
            .is_none_or(|n| n > 32 * 1024 * 1024)
        {
            return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
        }
        Ok(())
    }
    fn arx_array(&self, value: &Value, name: &str, real: bool) -> Result<Array> {
        let a = value.clone().array(&self.cancel)?;
        if !arx_is_numeric(&a) {
            return Err(format!(
                "Undefined function '{name}' for input arguments of type '{}'.",
                a.class()
            ));
        }
        if real && matches!(a, Array::Complex(_)) {
            return Err(format!("{name} of complex input is not supported yet"));
        }
        Ok(a)
    }
    fn arx_real_values(&self, value: &Value) -> Result<Vec<f64>> {
        let a = value.clone().array(&self.cancel)?;
        if !arx_is_numeric(&a) {
            return Err("Option values must be numeric.".into());
        }
        if matches!(a, Array::Complex(_)) {
            return Err("Option values must be real.".into());
        }
        if a.rows() > 1 && a.cols() > 1 {
            return Err("Array option must be a scalar or vector.".into());
        }
        self.arx_budget(a.len(), 1)?;
        let mut data = Vec::with_capacity(a.len());
        for i in 0..a.len() {
            self.arx_poll(i)?;
            data.push(arx_number(&a, i).re);
        }
        Ok(data)
    }
    /// Cancellable merge sort: bounded 4096-element runs, then linear merges.
    fn arx_sort<T: Copy>(
        &self,
        data: &mut [T],
        compare: impl Fn(&T, &T) -> Ordering,
    ) -> Result<()> {
        for chunk in data.chunks_mut(4096) {
            self.check_cancel()?;
            chunk.sort_by(&compare);
        }
        if data.len() <= 4096 {
            return Ok(());
        }
        let mut scratch = data.to_vec();
        let mut width = 4096;
        while width < data.len() {
            for start in (0..data.len()).step_by(width * 2) {
                let middle = (start + width).min(data.len());
                let end = (middle + width).min(data.len());
                let (mut left, mut right) = (start, middle);
                for (i, slot) in scratch.iter_mut().enumerate().take(end).skip(start) {
                    self.arx_poll(i)?;
                    if left < middle
                        && (right == end || compare(&data[left], &data[right]) != Ordering::Greater)
                    {
                        *slot = data[left];
                        left += 1;
                    } else {
                        *slot = data[right];
                        right += 1;
                    }
                }
            }
            for (a, b) in data.chunks_mut(4096).zip(scratch.chunks(4096)) {
                self.check_cancel()?;
                a.copy_from_slice(b);
            }
            width *= 2;
        }
        Ok(())
    }

    fn arx_select(
        &self,
        a: &Array,
        rows: usize,
        cols: usize,
        positions: &[usize],
    ) -> Result<Value> {
        self.check_cancel()?;
        macro_rules! collect {
            ($read:expr) => {{
                let mut values = Vec::with_capacity(positions.len());
                for (i, &p) in positions.iter().enumerate() {
                    self.arx_poll(i)?;
                    values.push(($read)(p));
                }
                values
            }};
        }
        Ok(match a {
            Array::Numeric(a) => Value::Matrix(Matrix::from_column_vec(
                rows,
                cols,
                collect!(|p| a.data()[p]),
            )?),
            Array::Logical(a) => Value::Logical(LogicalMatrix::from_column_vec(
                rows,
                cols,
                collect!(|p| a.data()[p]),
                &self.cancel,
            )?),
            Array::Complex(a) => Value::Complex(ComplexMatrix::from_column_vec(
                rows,
                cols,
                collect!(|p| a.data()[p]),
                &self.cancel,
            )?),
            Array::Typed(a) => arx_value(Array::Typed(a.select_with_cancel(
                rows,
                cols,
                positions,
                &self.cancel,
            )?)),
            Array::Character(a) => arx_value(Array::Character(CharacterRow::from_column_major(
                rows,
                cols,
                collect!(|p| a.units()[p]),
                &self.cancel,
            )?)),
            Array::Cell(a) => arx_value(Array::Cell(CellArray::from_shared(
                rows,
                cols,
                collect!(|p| a.get_linear(p).expect("validated index").clone()),
                &self.cancel,
            )?)),
            Array::Struct(a) => arx_value(Array::Struct(a.select(
                rows,
                cols,
                positions,
                &self.cancel,
            )?)),
            _ => return Err("repelem of handles is not supported yet".into()),
        })
    }
    fn arx_repelem(&self, args: &[Value]) -> Result<Value> {
        if args.len() < 2 {
            return Err("Not enough input arguments.".into());
        }
        if args.len() > 3 {
            return Err("repelem in more than two dimensions is not supported yet".into());
        }
        let a = args[0].clone().array(&self.cancel)?;
        self.arx_budget(a.rows().saturating_add(a.cols()), 2)?;
        let vector = args.len() == 2;
        if vector && a.rows() != 1 && a.cols() != 1 && !a.is_empty() {
            return Err("With two inputs, the first argument must be a vector. Use three-input syntax for matrices.".into());
        }
        let counts = |v: &Value, n: usize| -> Result<Vec<usize>> {
            let data = self.arx_real_values(v)?;
            if data.len() != 1 && data.len() != n {
                return Err(if vector { "In repelem(v, N), N must be a scalar or a vector of length equal to v." } else { "Repetition factors must be scalars or vectors matching the corresponding dimension." }.into());
            }
            if data.iter().any(|x| {
                !x.is_finite()
                    || *x < 0.
                    || x.fract() != 0.
                    || *x > crate::matrix::MAX_ELEMENTS as f64
            }) {
                return Err("Repetition factors must be finite nonnegative integers within the element limit.".into());
            }
            let mut result = Vec::with_capacity(n);
            for i in 0..n {
                self.arx_poll(i)?;
                result.push(data[if data.len() == 1 { 0 } else { i }] as usize);
            }
            Ok(result)
        };
        let (rc, cc) = if vector {
            if a.rows() == 1 {
                (vec![1], counts(&args[1], a.cols())?)
            } else {
                (counts(&args[1], a.rows())?, vec![1; a.cols()])
            }
        } else {
            (counts(&args[1], a.rows())?, counts(&args[2], a.cols())?)
        };
        let sum = |v: &[usize]| -> Result<usize> {
            let mut n = 0usize;
            for (i, &k) in v.iter().enumerate() {
                self.arx_poll(i)?;
                n = n.checked_add(k).ok_or("repelem size overflow")?;
            }
            Ok(n)
        };
        let (rows, cols) = (sum(&rc)?, sum(&cc)?);
        let n = Matrix::checked_len(rows, cols)?;
        if matches!(a, Array::Cell(_) | Array::Struct(_) | Array::Character(_)) && n > 65_536 {
            return Err("repelem exceeds the 65536-element container limit".into());
        }
        let units = n.saturating_mul(if matches!(a, Array::Complex(_)) { 3 } else { 2 });
        self.arx_budget(units.saturating_add(a.rows()).saturating_add(a.cols()), 1)?;
        let mut positions = Vec::with_capacity(n);
        if n == 0 {
            return self.arx_select(&a, rows, cols, &positions);
        }
        for (c, &copies) in cc.iter().enumerate() {
            for _ in 0..copies {
                self.check_cancel()?;
                for (r, &copies) in rc.iter().enumerate() {
                    for _ in 0..copies {
                        self.arx_poll(positions.len())?;
                        positions.push(r + c * a.rows());
                    }
                }
            }
        }
        self.arx_select(&a, rows, cols, &positions)
    }

    fn arx_isequaln(&self, args: &[Value]) -> Result<Value> {
        if args.len() < 2 {
            return Err("Not enough input arguments.".into());
        }
        let a = args[0].clone().array(&self.cancel)?;
        for b in &args[1..] {
            if !self.arx_equal(&a, &b.clone().array(&self.cancel)?)? {
                return Ok(arx_bool(false));
            }
        }
        Ok(arx_bool(true))
    }
    fn arx_equal(&self, a: &Array, b: &Array) -> Result<bool> {
        self.check_cancel()?;
        if (a.rows(), a.cols()) != (b.rows(), b.cols()) {
            return Ok(false);
        }
        match (a, b) {
            (Array::Cell(a), Array::Cell(b)) => {
                for i in 0..a.len() {
                    if !self.arx_equal(a.get_linear(i)?, b.get_linear(i)?)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            (Array::Struct(a), Array::Struct(b)) => {
                if a.field_count() != b.field_count() {
                    return Ok(false);
                }
                for (f, name) in a.field_names().iter().enumerate() {
                    let Some(g) = b.field_index(name) else {
                        return Ok(false);
                    };
                    for i in 0..a.len() {
                        if !self.arx_equal(a.get(f, i)?, b.get(g, i)?)? {
                            return Ok(false);
                        }
                    }
                }
                Ok(true)
            }
            (Array::Handle(a), Array::Handle(b)) => {
                Ok(a.same_storage(b) || a.name().is_some_and(|n| b.name() == Some(n)))
            }
            (Array::Graphics(a), Array::Graphics(b)) => {
                for i in 0..a.len() {
                    self.arx_poll(i)?;
                    if !a.get_linear(i)?.same_object(b.get_linear(i)?) {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            (a, b)
                if (arx_is_numeric(a) || matches!(a, Array::Character(_)))
                    && (arx_is_numeric(b) || matches!(b, Array::Character(_))) =>
            {
                let eq = |x: f64, y: f64| x == y || (x.is_nan() && y.is_nan());
                for i in 0..a.len() {
                    self.arx_poll(i)?;
                    let equal = match (a, b) {
                        (Array::Typed(x), Array::Typed(y)) if x.is_integer() && y.is_integer() => {
                            x.int(i) == y.int(i)
                        }
                        (Array::Typed(x), y) | (y, Array::Typed(x)) if x.is_integer() => {
                            let y = arx_number(y, i);
                            y.im == 0.
                                && super::typed_values::int_vs_real(x.int(i), y.re)
                                    == Some(Ordering::Equal)
                        }
                        _ => {
                            let (x, y) = (arx_number(a, i), arx_number(b, i));
                            eq(x.re, y.re) && eq(x.im, y.im)
                        }
                    };
                    if !equal {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn arx_structure(&self, name: &str, args: &[Value]) -> Result<Vec<Value>> {
        let required = if name == "isbanded" { 3 } else { 1 };
        if args.len() < required {
            return Err("Not enough input arguments.".into());
        }
        let max = if matches!(name, "issymmetric" | "ishermitian" | "bandwidth") {
            2
        } else {
            required
        };
        if args.len() > max {
            return Err("Too many input arguments.".into());
        }
        let a = self.arx_array(&args[0], name, false)?;
        if matches!(name, "issymmetric" | "ishermitian") {
            let skew = match args.get(1) {
                None => false,
                Some(v) => match arx_text(v).as_deref() {
                    Some("skew") => true,
                    Some("nonskew") => false,
                    _ => return Err("Second input must be 'skew' or 'nonskew'.".into()),
                },
            };
            if a.rows() != a.cols() {
                return Ok(vec![arx_bool(false)]);
            }
            for c in 0..a.cols() {
                for r in 0..=c {
                    self.arx_poll(r + c * a.rows())?;
                    if let Array::Typed(t) = &a
                        && t.is_integer()
                    {
                        let x = t.int(r + c * a.rows());
                        let y = t.int(c + r * a.rows());
                        if x != if skew { -y } else { y } {
                            return Ok(vec![arx_bool(false)]);
                        }
                    } else {
                        let x = arx_number(&a, r + c * a.rows());
                        let mut y = arx_number(&a, c + r * a.rows());
                        if name == "ishermitian" {
                            y.im = -y.im;
                        }
                        if skew {
                            y.re = -y.re;
                            y.im = -y.im;
                        }
                        if x != y {
                            return Ok(vec![arx_bool(false)]);
                        }
                    }
                }
            }
            return Ok(vec![arx_bool(true)]);
        }
        let mut limits = match name {
            "isdiag" => (0, 0),
            "istril" => (usize::MAX, 0),
            "istriu" => (0, usize::MAX),
            _ => (usize::MAX, usize::MAX),
        };
        if name == "isbanded" {
            let valid = |v: &Value| -> Result<usize> {
                let d = self.arx_real_values(v).unwrap_or_default();
                match d.as_slice() {
                    [x] if x.is_finite()
                        && *x >= 0.
                        && x.fract() == 0.
                        && *x <= usize::MAX as f64 =>
                    {
                        Ok(*x as usize)
                    }
                    _ => Err("Second and third inputs must be nonnegative integer scalars.".into()),
                }
            };
            limits = (valid(&args[1])?, valid(&args[2])?);
        }
        let direction = if name == "bandwidth" {
            match args.get(1) {
                None => None,
                Some(v) => match arx_text(v).as_deref() {
                    Some("lower") => Some(false),
                    Some("upper") => Some(true),
                    _ => return Err("Second input must be 'lower' or 'upper'.".into()),
                },
            }
        } else {
            None
        };
        if a.is_empty() {
            return Ok(if name == "bandwidth" {
                vec![arx_scalar(0.), arx_scalar(0.)]
            } else {
                vec![arx_bool(true)]
            });
        }
        let (mut lower, mut upper) = (0, 0);
        for c in 0..a.cols() {
            for r in 0..a.rows() {
                self.arx_poll(r + c * a.rows())?;
                let z = arx_number(&a, r + c * a.rows());
                if z.re != 0. || z.im != 0. {
                    lower = lower.max(r.saturating_sub(c));
                    upper = upper.max(c.saturating_sub(r));
                    if lower > limits.0 || upper > limits.1 {
                        return Ok(vec![arx_bool(false)]);
                    }
                }
            }
        }
        if name == "bandwidth" {
            Ok(vec![
                arx_scalar(if direction == Some(true) {
                    upper
                } else {
                    lower
                } as f64),
                arx_scalar(upper as f64),
            ])
        } else {
            Ok(vec![arx_bool(true)])
        }
    }

    fn arx_moving(&self, name: &str, args: &[Value]) -> Result<Value> {
        if args.len() < 2 {
            return Err("Not enough input arguments.".into());
        }
        let a = self.arx_array(&args[0], name, true)?;
        let variance = matches!(name, "movstd" | "movvar");
        if variance && matches!(&a, Array::Typed(t) if t.is_integer()) {
            return Err(format!(
                "{name} requires real double, single or logical input; integer input is not supported"
            ));
        }
        let mut normalized = vec![
            args[0].clone(),
            Value::Matrix(
                args[1]
                    .clone()
                    .matrix(&self.cancel)
                    .map_err(|_| ARX_WINDOW_ERROR)?,
            ),
        ];
        let mut at = 2;
        let mut population = false;
        if variance && args.get(at).is_some_and(|v| arx_text(v).is_none()) {
            let v = self.arx_real_values(&args[at])?;
            population = match v.as_slice() {
                [] | [0.] => false,
                [1.] => true,
                _ => return Err("Normalization argument must be 0 or 1.".into()),
            };
            at += 1;
        }
        normalized.extend(self.text_arguments(args[at..].to_vec())?);
        let options = arx_moving_options(
            name,
            &normalized,
            if a.rows() == 1 { 2 } else { 1 },
            matches!(name, "movmax" | "movmin"),
        )?;
        let n = if options.dim == 1 { a.rows() } else { a.cols() };
        let lanes = if options.dim == 1 { a.cols() } else { a.rows() };
        let kept = arx_window_kept(options.before, options.after, n, options.discard);
        let (rows, cols) = if options.dim == 1 {
            (kept, a.cols())
        } else {
            (a.rows(), kept)
        };
        let length = Matrix::checked_len(rows, cols)?;
        // Output keys, final output, and per-lane sorted keys/Fenwick scratch.
        let buffers = match name {
            "movmax" | "movmin" => 5,
            "movstd" | "movvar" => 7,
            _ => 9,
        };
        self.arx_budget(length.saturating_add(a.len()), buffers)?;
        let mut result = vec![ArxNumber::Real(0.); length];
        if length == 0 {
            return self.arx_moving_value(&a, rows, cols, result, variance);
        }
        let fill = options.fill.map(|x| match &a {
            Array::Typed(t) if t.is_integer() && !variance && !x.is_nan() => {
                ArxNumber::Int(t.class().from_f64(x))
            }
            _ => ArxNumber::Real(x),
        });
        let lane = |l: usize, out: &mut [ArxNumber]| -> Result<()> {
            let mut samples = Vec::with_capacity(n);
            for i in 0..n {
                self.arx_poll(i)?;
                samples.push(arx_key(
                    &a,
                    if options.dim == 1 {
                        i + l * a.rows()
                    } else {
                        l + i * a.rows()
                    },
                ));
            }
            self.arx_moving_lane(name, &samples, &options, fill, population, out)
        };
        // Parallelize independent columns only; row lanes stay strided and serial.
        // The threshold is measured by the release benchmark in fn_arrays.
        if options.dim == 1
            && length >= 262_144
            && lanes > 1
            && crate::parallel::current_worker_count() > 1
        {
            use rayon::prelude::*;
            result
                .par_chunks_mut(kept)
                .enumerate()
                .try_for_each(|(l, out)| lane(l, out))?;
        } else if options.dim == 1 {
            for (l, out) in result.chunks_mut(kept).enumerate() {
                lane(l, out)?;
            }
        } else {
            let mut scratch = vec![ArxNumber::Real(0.); kept];
            for l in 0..lanes {
                self.check_cancel()?;
                lane(l, &mut scratch)?;
                for (i, &x) in scratch.iter().enumerate() {
                    self.arx_poll(i)?;
                    result[l + i * rows] = x;
                }
            }
        }
        self.arx_moving_value(&a, rows, cols, result, variance)
    }
    fn arx_moving_value(
        &self,
        a: &Array,
        rows: usize,
        cols: usize,
        result: Vec<ArxNumber>,
        variance: bool,
    ) -> Result<Value> {
        self.check_cancel()?;
        match a {
            Array::Typed(t) if t.is_integer() && !variance => {
                Ok(arx_value(Array::Typed(TypedArray::from_ints(
                    t.class(),
                    rows,
                    cols,
                    result.iter().map(|x| match x {
                        ArxNumber::Int(i) => *i,
                        ArxNumber::Real(x) => t.class().from_f64(*x),
                    }),
                )?)))
            }
            Array::Typed(t) if t.class() == NumericClass::Single => Ok(arx_value(Array::Typed(
                TypedArray::from_reals(t.class(), rows, cols, result.iter().map(|x| x.arx_real()))?,
            ))),
            Array::Logical(_) if !variance => Ok(Value::Logical(LogicalMatrix::from_column_vec(
                rows,
                cols,
                result
                    .iter()
                    .map(|x| u8::from(x.arx_real() != 0.))
                    .collect(),
                &self.cancel,
            )?)),
            _ => Ok(Value::Matrix(Matrix::from_column_vec(
                rows,
                cols,
                result.iter().map(|x| x.arx_real()).collect(),
            )?)),
        }
    }
    fn arx_moving_lane(
        &self,
        name: &str,
        samples: &[ArxNumber],
        options: &super::stats_more::ArxMovingOptions,
        fill: Option<ArxNumber>,
        population: bool,
        out: &mut [ArxNumber],
    ) -> Result<()> {
        let n = samples.len();
        let (before, after) = (options.before, options.after);
        let mut order = VecDeque::<usize>::new();
        let mut median = if name == "movmedian" {
            Some(self.arx_median_tree(samples, fill)?)
        } else {
            None
        };
        let (mut lo, mut hi, mut nan, mut infinities) = (0, 0, 0usize, 0usize);
        let variance = matches!(name, "movstd" | "movvar");
        let mut moments = ArxMoments::default();
        let width = before.saturating_add(after).saturating_add(1);
        let start = if options.discard { before } else { 0 };
        for (o, slot) in out.iter_mut().enumerate() {
            self.arx_poll(o)?;
            let i = o + start;
            let range = arx_window_range(before, after, i, n);
            let (new_lo, new_hi) = (range.start, range.end);
            while lo < new_lo {
                self.arx_poll(lo)?;
                let x = samples[lo];
                if x.arx_nan() {
                    nan -= 1;
                } else {
                    if let Some(tree) = &mut median {
                        tree.arx_change(x, 1, false);
                    }
                    if variance {
                        if x.arx_real().is_infinite() {
                            infinities -= 1;
                        } else {
                            moments.arx_remove(x.arx_real());
                        }
                    }
                }
                lo += 1;
            }
            while hi < new_hi {
                self.arx_poll(hi)?;
                let x = samples[hi];
                if x.arx_nan() {
                    nan += 1;
                } else {
                    if let Some(tree) = &mut median {
                        tree.arx_change(x, 1, true);
                    }
                    if variance {
                        if x.arx_real().is_infinite() {
                            infinities += 1;
                        } else {
                            moments.arx_add(x.arx_real());
                        }
                    }
                    if matches!(name, "movmin" | "movmax") {
                        while order.back().is_some_and(|&p| {
                            if name == "movmax" {
                                samples[p] <= x
                            } else {
                                samples[p] >= x
                            }
                        }) {
                            order.pop_back();
                        }
                        order.push_back(hi);
                    }
                }
                hi += 1;
            }
            while order.front().is_some_and(|&p| p < lo) {
                order.pop_front();
            }
            let missing = arx_window_missing(before, after, hi - lo);
            let padding = fill.filter(|x| missing > 0 && !(options.omit_nan && x.arx_nan()));
            let bad_nan = (!options.omit_nan && nan > 0) || padding.is_some_and(ArxNumber::arx_nan);
            let value = if bad_nan {
                ArxNumber::Real(f64::NAN)
            } else if matches!(name, "movmin" | "movmax") {
                let selected = order.front().map(|&p| samples[p]);
                match (selected, padding) {
                    (Some(x), Some(y)) => {
                        if name == "movmax" {
                            x.max(y)
                        } else {
                            x.min(y)
                        }
                    }
                    (Some(x), None) | (None, Some(x)) => x,
                    _ => ArxNumber::Real(f64::NAN),
                }
            } else if let Some(tree) = &mut median {
                if let Some(x) = padding {
                    tree.arx_change(x, missing, true);
                }
                let value = tree.arx_median();
                if let Some(x) = padding {
                    tree.arx_change(x, missing, false);
                }
                value
            } else {
                // Small windows use a fresh centered reduction. For long windows,
                // remove/add moments and rebuild once per width to bound drift.
                if width <= 32
                    || o.is_multiple_of(width.min(n).max(1))
                    || !moments.m2.is_finite()
                    || moments.m2 < 0.
                    || (moments.error > 0. && moments.m2 <= moments.error)
                {
                    moments = ArxMoments::default();
                    for (j, x) in samples[lo..hi].iter().enumerate() {
                        self.arx_poll(j)?;
                        if x.arx_real().is_finite() {
                            moments.arx_add(x.arx_real());
                        }
                    }
                }
                let mut m = moments;
                if let Some(x) = padding {
                    m.arx_merge_constant(x.arx_real(), missing);
                }
                let invalid = infinities > 0
                    || padding.is_some_and(|x| !x.arx_real().is_finite())
                    || m.n == 0;
                let v = if invalid {
                    f64::NAN
                } else if m.n == 1 {
                    0.
                } else {
                    m.m2.max(0.)
                        / if population {
                            m.n
                        } else {
                            m.n.saturating_sub(1).max(1)
                        } as f64
                };
                ArxNumber::Real(if !invalid && m.n > 1 && (m.lost || !m.m2.is_finite()) {
                    self.arx_scaled_stat(name, &samples[lo..hi], padding, missing, population)?
                } else if name == "movstd" {
                    v.sqrt()
                } else {
                    v
                })
            };
            *slot = value;
        }
        Ok(())
    }
    /// Exceptional-range fallback: normalize before squaring so finite std
    /// results survive overflow/underflow in the unscaled second moment.
    fn arx_scaled_stat(
        &self,
        name: &str,
        samples: &[ArxNumber],
        fill: Option<ArxNumber>,
        missing: usize,
        population: bool,
    ) -> Result<f64> {
        let mut scale = fill.map_or(0., |x| x.arx_real().abs());
        let mut count = if fill.is_some() { missing } else { 0 };
        for (i, &x) in samples.iter().enumerate() {
            self.arx_poll(i)?;
            if !x.arx_nan() {
                scale = scale.max(x.arx_real().abs());
                count += 1;
            }
        }
        if scale == 0. {
            return Ok(0.);
        }
        let mut sum = fill.map_or(0., |x| (x.arx_real() / scale) * missing as f64);
        for (i, &x) in samples.iter().enumerate() {
            self.arx_poll(i)?;
            if !x.arx_nan() {
                sum += x.arx_real() / scale;
            }
        }
        let mean = sum / count as f64;
        let mut squares = fill.map_or(0., |x| {
            (x.arx_real() / scale - mean).powi(2) * missing as f64
        });
        for (i, &x) in samples.iter().enumerate() {
            self.arx_poll(i)?;
            if !x.arx_nan() {
                squares += (x.arx_real() / scale - mean).powi(2);
            }
        }
        let divisor = if population {
            count
        } else {
            count.saturating_sub(1).max(1)
        } as f64;
        let deviation = scale * (squares / divisor).sqrt();
        Ok(if name == "movstd" {
            deviation
        } else {
            deviation * deviation
        })
    }
    fn arx_median_tree(&self, samples: &[ArxNumber], fill: Option<ArxNumber>) -> Result<ArxMedian> {
        let mut keys = Vec::with_capacity(samples.len() + 1);
        for (i, &x) in samples.iter().enumerate() {
            self.arx_poll(i)?;
            if !x.arx_nan() {
                keys.push(x);
            }
        }
        if let Some(x) = fill
            && !x.arx_nan()
        {
            keys.push(x);
        }
        self.arx_sort(&mut keys, Ord::cmp)?;
        keys.dedup();
        let tree = vec![0; keys.len() + 1];
        Ok(ArxMedian {
            keys,
            tree,
            count: 0,
        })
    }

    fn arx_tolerance(&self, name: &str, args: &[Value], count: usize) -> Result<Vec<Value>> {
        let member = name == "ismembertol";
        let required = if member { 2 } else { 1 };
        if args.len() < required {
            return Err("Not enough input arguments.".into());
        }
        let a = args[0].clone().array(&self.cancel)?;
        if !arx_is_float(&a) {
            return Err(ARX_FLOAT_ERROR.into());
        }
        let b = if member {
            args[1].clone().array(&self.cancel)?
        } else {
            a.clone()
        };
        if !arx_is_float(&b) {
            return Err("Input B must be a real full matrix of type single or double.".into());
        }
        let mut tol = if a.class() == "single" || b.class() == "single" {
            1e-6
        } else {
            1e-12
        };
        let mut at = required;
        if let Some(v) = args.get(at) {
            let text = arx_text(v);
            if !text.as_deref().is_some_and(|s| {
                matches!(
                    s.to_ascii_lowercase().as_str(),
                    "byrows" | "datascale" | "outputallindices"
                )
            }) {
                let values = self.arx_real_values(v).map_err(|_| ARX_TOL_ERROR)?;
                tol = match values.as_slice() {
                    [t] if t.is_finite() && *t > 0. => *t,
                    _ => return Err(ARX_TOL_ERROR.into()),
                };
                at += 1;
            }
        }
        let (mut byrows, mut all) = (false, false);
        let mut scale = None;
        while at < args.len() {
            let flag = arx_text(&args[at])
                .ok_or("Expected a tolerance option name")?
                .to_ascii_lowercase();
            let v = args
                .get(at + 1)
                .ok_or("Tolerance options require name-value pairs")?;
            match flag.as_str() {
                "byrows" | "outputallindices" => {
                    let data = self.arx_real_values(v)?;
                    let truth = match data.as_slice() {
                        [0.] => false,
                        [1.] => true,
                        _ => return Err("Option value must be a logical scalar.".into()),
                    };
                    if flag == "byrows" {
                        byrows = truth;
                    } else {
                        all = truth;
                    }
                }
                "datascale" => {
                    scale = Some(self.arx_real_values(v)?);
                }
                _ => return Err(format!("{name}: option '{flag}' is not supported yet")),
            }
            at += 2;
        }
        if byrows && a.cols() != b.cols() {
            return Err("A and B must have the same number of columns.".into());
        }
        let cols = if byrows { a.cols() } else { 1 };
        let na = if byrows { a.rows() } else { a.len() };
        let nb = if byrows { b.rows() } else { b.len() };
        all &= count > 1;
        if all && member && na > 65_536 {
            return Err("OutputAllIndices exceeds the cell slot limit".into());
        }
        self.arx_budget(a.len().saturating_add(b.len()), 8)?;
        let read = |a: &Array, r: usize, c: usize| {
            arx_number(a, if byrows { r + c * a.rows() } else { r }).re
        };
        let scales = if let Some(s) = scale {
            if (s.len() != 1 && s.len() != cols) || s.iter().any(|x| x.is_nan() || *x < 0.) {
                return Err("DataScale must be nonnegative and scalar, or one value per column with ByRows.".into());
            }
            (0..cols)
                .map(|i| s[if s.len() == 1 { 0 } else { i }])
                .collect::<Vec<_>>()
        } else {
            let mut s = vec![0f64; cols];
            for (c, sc) in s.iter_mut().enumerate() {
                self.arx_poll(c)?;
                for data in [&a, &b] {
                    let nr = if byrows { data.rows() } else { data.len() };
                    for r in 0..nr {
                        self.arx_poll(r)?;
                        *sc = sc.max(read(data, r, c).abs());
                    }
                }
            }
            s
        };
        let compare = |a: &Array, i: usize, b: &Array, j: usize| -> Ordering {
            for c in 0..cols {
                let (x, y) = (read(a, i, c), read(b, j, c));
                let order = if x == y || (x.is_nan() && y.is_nan()) {
                    Ordering::Equal
                } else if x.is_nan() {
                    Ordering::Greater
                } else if y.is_nan() {
                    Ordering::Less
                } else {
                    x.total_cmp(&y)
                };
                if order != Ordering::Equal {
                    return order;
                }
            }
            i.cmp(&j)
        };
        let matches = |a: &Array, i: usize, b: &Array, j: usize| -> Result<bool> {
            for (c, &s) in scales.iter().enumerate() {
                self.arx_poll(c)?;
                let (x, y) = (read(a, i, c), read(b, j, c));
                if x.is_nan() || y.is_nan() || !(x == y || (x - y).abs() <= tol * s) {
                    return Ok(false);
                }
            }
            Ok(true)
        };
        if member {
            let mut order = (0..nb).collect::<Vec<_>>();
            self.arx_sort(&mut order, |&i, &j| compare(&b, i, &b, j))?;
            let (rows, columns) = if byrows {
                (a.rows(), 1)
            } else {
                (a.rows(), a.cols())
            };
            let mut logical = vec![0u8; na];
            let mut locations = if count > 1 && !all {
                vec![0.; na]
            } else {
                Vec::new()
            };
            let mut slots = Vec::new();
            // Range minimum avoids scanning all duplicate values for the usual
            // scalar membership form. ByRows validates the remaining columns.
            let minimum = if count > 1 && !byrows && !all {
                Some(ArxRangeMin::arx_new(&order, &self.cancel)?)
            } else {
                None
            };
            let mut all_count = 0usize;
            for i in 0..na {
                self.arx_poll(i)?;
                let (lo, hi) = if cols == 0 {
                    (0, nb)
                } else {
                    let x = read(&a, i, 0);
                    let d = tol * scales[0];
                    if x.is_nan() {
                        (0, 0)
                    } else {
                        (
                            order.partition_point(|&j| {
                                let y = read(&b, j, 0);
                                y < x && x - y > d
                            }),
                            order.partition_point(|&j| {
                                let y = read(&b, j, 0);
                                !y.is_nan() && (y <= x || y - x <= d)
                            }),
                        )
                    }
                };
                let mut found = Vec::new();
                let first = if !byrows && !all && lo < hi {
                    minimum
                        .as_ref()
                        .map_or(Some(order[lo]), |index| index.arx_get(lo, hi))
                } else {
                    let mut first = None;
                    for (k, &j) in order[lo..hi].iter().enumerate() {
                        self.arx_poll(k)?;
                        if matches(&a, i, &b, j)? {
                            first = Some(first.map_or(j, |f: usize| f.min(j)));
                            if all {
                                found.push(j);
                            } else if j == 0 || count == 1 {
                                break;
                            }
                        }
                    }
                    first
                };
                if let Some(j) = first {
                    logical[i] = 1;
                    if !locations.is_empty() {
                        locations[i] = (j + 1) as f64;
                    }
                }
                if all {
                    all_count = all_count
                        .checked_add(found.len())
                        .ok_or("OutputAllIndices size overflow")?;
                    self.arx_budget(all_count, 1)?;
                    self.arx_sort(&mut found, Ord::cmp)?;
                    // Reference behavior: a column of matches, or the scalar 0 for a non-member.
                    let column = if found.is_empty() {
                        vec![0.]
                    } else {
                        found.iter().map(|i| (i + 1) as f64).collect()
                    };
                    slots.push(std::sync::Arc::new(Array::Numeric(
                        Matrix::from_column_vec(column.len(), 1, column)?,
                    )));
                }
            }
            let mut outputs = vec![Value::Logical(LogicalMatrix::from_column_vec(
                rows,
                columns,
                logical,
                &self.cancel,
            )?)];
            if count == 1 {
                return Ok(outputs);
            }
            let loc = if all {
                arx_value(Array::Cell(CellArray::from_shared(
                    rows,
                    columns,
                    slots,
                    &self.cancel,
                )?))
            } else {
                Value::Matrix(Matrix::from_column_vec(rows, columns, locations)?)
            };
            outputs.push(loc);
            Ok(outputs)
        } else {
            let mut order = (0..na).collect::<Vec<_>>();
            self.arx_sort(&mut order, |&i, &j| compare(&a, i, &a, j))?;
            let mut reps: Vec<usize> = Vec::new();
            let mut groups = if count >= 3 { vec![0.; na] } else { Vec::new() };
            for (k, &i) in order.iter().enumerate() {
                self.arx_poll(k)?;
                let mut group = None;
                if byrows {
                    for (g, &j) in reps.iter().enumerate().rev() {
                        self.arx_poll(g)?;
                        if cols > 0 && read(&a, i, 0) - read(&a, j, 0) > tol * scales[0] {
                            break;
                        }
                        if matches(&a, i, &a, j)? {
                            group = Some(g);
                        }
                    }
                } else if let Some(&j) = reps.last()
                    && matches(&a, i, &a, j)?
                {
                    group = Some(reps.len() - 1);
                }
                let g = group.unwrap_or_else(|| {
                    reps.push(i);
                    reps.len() - 1
                });
                if count >= 3 {
                    groups[i] = (g + 1) as f64;
                }
            }
            let (rows, columns) = if byrows {
                (reps.len(), a.cols())
            } else if a.rows() == 1 {
                (1, reps.len())
            } else {
                (reps.len(), 1)
            };
            let mut positions = Vec::with_capacity(rows * columns);
            if byrows {
                for c in 0..a.cols() {
                    self.arx_poll(c)?;
                    for &i in &reps {
                        self.arx_poll(positions.len())?;
                        positions.push(i + c * a.rows());
                    }
                }
            } else {
                positions.extend_from_slice(&reps);
            }
            let values = self.arx_select(&a, rows, columns, &positions)?;
            if count == 1 {
                return Ok(vec![values]);
            }
            let ia = if all {
                if reps.len() > 65_536 {
                    return Err("OutputAllIndices exceeds the cell slot limit".into());
                }
                let mut slots = Vec::with_capacity(reps.len());
                let mut total = 0usize;
                for &i in &reps {
                    let mut indices = Vec::new();
                    for j in 0..na {
                        self.arx_poll(j)?;
                        if matches(&a, i, &a, j)? || i == j {
                            total += 1;
                            self.arx_budget(total, 1)?;
                            indices.push((j + 1) as f64);
                        }
                    }
                    slots.push(std::sync::Arc::new(Array::Numeric(
                        Matrix::from_column_vec(indices.len(), 1, indices)?,
                    )));
                }
                arx_value(Array::Cell(CellArray::from_shared(
                    reps.len(),
                    1,
                    slots,
                    &self.cancel,
                )?))
            } else {
                Value::Matrix(Matrix::from_column_vec(
                    reps.len(),
                    1,
                    reps.iter().map(|i| (i + 1) as f64).collect(),
                )?)
            };
            let mut outputs = vec![values, ia];
            if count >= 3 {
                outputs.push(Value::Matrix(Matrix::from_column_vec(na, 1, groups)?));
            }
            Ok(outputs)
        }
    }

    fn arx_histcounts2(&self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        if args.len() < 2 {
            return Err("Not enough input arguments.".into());
        }
        let x = self.arx_array(&args[0], "histcounts2", true)?;
        let y = self.arx_array(&args[1], "histcounts2", true)?;
        if (x.rows(), x.cols()) != (y.rows(), y.cols()) {
            return Err(format!(
                "Expected input number 2, y, to be of size {}x{} when it is actually size {}x{}.",
                x.rows(),
                x.cols(),
                y.rows(),
                y.cols()
            ));
        }
        let mut at = 2;
        let mut explicit = None;
        let mut bins = None;
        let mut width = None;
        let mut normalization = "count".to_owned();
        if args.get(at).is_some_and(|v| arx_text(v).is_none()) {
            let v = self.arx_real_values(&args[at])?;
            at += 1;
            if args.get(at).is_some_and(|v| arx_text(v).is_none()) {
                explicit = Some((v, self.arx_real_values(&args[at])?));
                at += 1;
            } else {
                bins = Some(v);
            }
        }
        while at < args.len() {
            let name = arx_text(&args[at])
                .ok_or("histcounts2 expects an option name")?
                .to_ascii_lowercase();
            let value = args
                .get(at + 1)
                .ok_or("histcounts2 options require name-value pairs")?;
            match name.as_str() {
                "normalization" => {
                    normalization = arx_text(value)
                        .ok_or("Normalization must be text")?
                        .to_ascii_lowercase()
                }
                "binwidth" => width = Some(self.arx_real_values(value)?),
                "numbins" => bins = Some(self.arx_real_values(value)?),
                _ => return Err(format!("histcounts2 option '{name}' is not supported yet")),
            }
            at += 2;
        }
        if !matches!(
            normalization.as_str(),
            "count" | "probability" | "countdensity" | "pdf" | "cumcount" | "cdf"
        ) {
            return Err("Unsupported histcounts2 normalization.".into());
        }
        if explicit.is_some() && (bins.is_some() || width.is_some()) {
            return Err("Bin edges cannot be combined with NumBins or BinWidth.".into());
        }
        if bins.is_some() && width.is_some() {
            return Err("Specify either NumBins or BinWidth.".into());
        }
        let pair = |v: &[f64], integer: bool| -> Result<[f64; 2]> {
            if !(v.len() == 1 || v.len() == 2)
                || v.iter()
                    .any(|x| !x.is_finite() || *x <= 0. || (integer && x.fract() != 0.))
            {
                return Err("Bin counts or widths must be one or two finite positive values; counts must be integers.".into());
            }
            Ok([v[0], v[v.len() - 1]])
        };
        let bins = bins.as_deref().map(|v| pair(v, true)).transpose()?;
        let width = width.as_deref().map(|v| pair(v, false)).transpose()?;
        let (xe, ye) = if let Some(edges) = explicit {
            edges
        } else {
            (
                self.arx_hist_edges(&x, bins.map(|v| v[0]), width.map(|v| v[0]))?,
                self.arx_hist_edges(&y, bins.map(|v| v[1]), width.map(|v| v[1]))?,
            )
        };
        for edges in [&xe, &ye] {
            if edges.len() < 2
                || edges.iter().any(|v| v.is_nan())
                || edges.windows(2).any(|w| w[0] >= w[1])
            {
                return Err(
                    "Bin edges must be a strictly increasing vector with at least two elements."
                        .into(),
                );
            }
        }
        let (nx, ny) = (xe.len() - 1, ye.len() - 1);
        let length = Matrix::checked_len(nx, ny)?;
        let indices = usize::from(count >= 4) + usize::from(count >= 5);
        self.arx_budget(
            length
                .saturating_add(x.len() * indices)
                .saturating_add(xe.len())
                .saturating_add(ye.len()),
            1,
        )?;
        let mut counts = vec![0.; length];
        let mut bx = if count >= 4 {
            vec![0.; x.len()]
        } else {
            Vec::new()
        };
        let mut by = if count >= 5 {
            vec![0.; x.len()]
        } else {
            Vec::new()
        };
        let locate = |v: f64, edges: &[f64]| -> usize {
            if v.is_nan() || v < edges[0] || v > edges[edges.len() - 1] {
                0
            } else {
                edges.partition_point(|e| *e <= v).min(edges.len() - 1)
            }
        };
        for i in 0..x.len() {
            self.arx_poll(i)?;
            let (a, b) = (
                locate(arx_number(&x, i).re, &xe),
                locate(arx_number(&y, i).re, &ye),
            );
            if a > 0 && b > 0 {
                counts[a - 1 + (b - 1) * nx] += 1.;
                if count >= 4 {
                    bx[i] = a as f64;
                }
                if count >= 5 {
                    by[i] = b as f64;
                }
            }
        }
        if matches!(normalization.as_str(), "cumcount" | "cdf") {
            for c in 0..ny {
                for r in 0..nx {
                    self.arx_poll(r + c * nx)?;
                    let i = r + c * nx;
                    if r > 0 {
                        counts[i] += counts[i - 1];
                    }
                }
            }
            for c in 1..ny {
                for r in 0..nx {
                    self.arx_poll(r + c * nx)?;
                    counts[r + c * nx] += counts[r + (c - 1) * nx];
                }
            }
        }
        for c in 0..ny {
            for r in 0..nx {
                self.arx_poll(r + c * nx)?;
                let density = if matches!(normalization.as_str(), "countdensity" | "pdf") {
                    (xe[r + 1] - xe[r]) * (ye[c + 1] - ye[c])
                } else {
                    1.
                };
                let total = if matches!(normalization.as_str(), "probability" | "pdf" | "cdf") {
                    x.len() as f64
                } else {
                    1.
                };
                counts[r + c * nx] /= density * total;
            }
        }
        let mut outputs = vec![Value::Matrix(Matrix::from_column_vec(nx, ny, counts)?)];
        if count >= 2 {
            outputs.push(Value::Matrix(Matrix::from_column_vec(1, xe.len(), xe)?));
        }
        if count >= 3 {
            outputs.push(Value::Matrix(Matrix::from_column_vec(1, ye.len(), ye)?));
        }
        if count >= 4 {
            outputs.push(Value::Matrix(Matrix::from_column_vec(
                x.rows(),
                x.cols(),
                bx,
            )?));
        }
        if count >= 5 {
            outputs.push(Value::Matrix(Matrix::from_column_vec(
                y.rows(),
                y.cols(),
                by,
            )?));
        }
        Ok(outputs)
    }
    fn arx_hist_edges(&self, a: &Array, bins: Option<f64>, width: Option<f64>) -> Result<Vec<f64>> {
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        let mut integer = true;
        let mut moments = ArxMoments::default();
        for i in 0..a.len() {
            self.arx_poll(i)?;
            let x = arx_number(a, i).re;
            if x.is_finite() {
                lo = lo.min(x);
                hi = hi.max(x);
                integer &= x.fract() == 0.;
                if bins.is_none() && width.is_none() {
                    moments.arx_add(x);
                }
            }
        }
        if lo > hi {
            if bins.is_none() && width.is_none() {
                return Ok(vec![0., 1.]);
            }
            (lo, hi) = (0., 1.);
        }
        let (start, step, n) = if let Some(width) = width {
            let start = (lo / width).floor() * width;
            (start, width, ((hi - start) / width).ceil().max(1.))
        } else if let Some(n) = bins {
            return self.arx_hist_bin_picker(lo, hi, Some(n), (hi - lo) / n);
        } else if a.is_empty() {
            (0., 1., 1.)
        } else if integer && hi - lo <= 50. && lo >= -2f64.powi(52) && hi <= 2f64.powi(52) {
            (lo - 0.5, 1., hi - lo + 1.)
        } else {
            let deviation = (moments.m2 / moments.n.saturating_sub(1).max(1) as f64).sqrt();
            // Scott's bivariate exponent is 1/4 (the 1-D histcounts rule is 1/3).
            let raw = 3.5 * deviation / (moments.n as f64).sqrt().sqrt();
            return self.arx_hist_bin_picker(lo, hi, None, raw);
        };
        if !start.is_finite()
            || !step.is_finite()
            || !n.is_finite()
            || !(1. ..=65_536.).contains(&n)
        {
            return Err("histcounts2 exceeds the 65536-bin edge limit; specify fewer bins".into());
        }
        let mut edges = Vec::with_capacity(n as usize + 1);
        for i in 0..=n as usize {
            self.arx_poll(i)?;
            edges.push(start + i as f64 * step);
        }
        Ok(edges)
    }

    /// Nice-edge construction used by the existing histcounts bin picker
    /// in poly_interp/array_ops.rs, with allocation bounds and cancellation.
    fn arx_hist_bin_picker(
        &self,
        lo: f64,
        hi: f64,
        bins: Option<f64>,
        raw: f64,
    ) -> Result<Vec<f64>> {
        if bins.is_some_and(|n| !(1. ..=65_536.).contains(&n)) {
            return Err("histcounts2 exceeds the 65536-bin edge limit; specify fewer bins".into());
        }
        let scale = lo.abs().max(hi.abs());
        if !raw.is_finite() || !(hi - lo).is_finite() || scale > f64::MAX / 4. {
            return Err(
                "histcounts2 automatic edge range is too large; specify explicit edges".into(),
            );
        }
        let eps = f64::from_bits(scale.to_bits() + 1) - scale;
        let raw = raw.max(eps);
        let (left, right, width, count) = if hi - lo > eps.sqrt().max(f64::MIN_POSITIVE) {
            let power = 10f64.powf(raw.log10().floor());
            let relative = raw / power;
            if let Some(n) = bins {
                let mut width = power * relative.floor();
                let left = (width * (lo / width).floor()).min(lo);
                if n > 1. {
                    let lower = (hi - left) / n;
                    let upper = (hi - left) / (n - 1.);
                    let power = 10f64.powf((upper - lower).log10().floor());
                    width = power * (lower / power).ceil();
                }
                (left, (left + n * width).max(hi), width, n)
            } else {
                let factor = if relative < 1.5 {
                    1.
                } else if relative < 2.5 {
                    2.
                } else if relative < 4. {
                    3.
                } else if relative < 7.5 {
                    5.
                } else {
                    10.
                };
                let width = factor * power;
                let left = (width * (lo / width).floor()).min(lo);
                let count = ((hi - left) / width).ceil().max(1.);
                (left, (left + count * width).max(hi), width, count)
            }
        } else {
            let count = bins.unwrap_or(1.);
            let range = (count * eps).ceil().max(1.);
            let left = (2. * (lo - range / 4.)).floor() / 2.;
            let right = (2. * (hi + range / 4.)).ceil() / 2.;
            (left, right, (right - left) / count, count)
        };
        if !left.is_finite()
            || !right.is_finite()
            || !width.is_finite()
            || width <= 0.
            || !(1. ..=65_536.).contains(&count)
        {
            return Err(
                "histcounts2 cannot form bounded automatic edges; specify explicit edges".into(),
            );
        }
        let mut edges = Vec::with_capacity(count as usize + 1);
        for i in 0..count as usize {
            self.arx_poll(i)?;
            edges.push(left + i as f64 * width);
        }
        edges.push(right);
        Ok(edges)
    }
}

#[derive(Clone, Copy, Default)]
struct ArxMoments {
    n: usize,
    origin: f64,
    mean: f64,
    m2: f64,
    error: f64,
    lost: bool,
}
impl ArxMoments {
    fn arx_add(&mut self, x: f64) {
        if self.n == 0 {
            self.n = 1;
            self.origin = x;
            return;
        }
        let x = x - self.origin;
        let previous = self.n;
        self.n += 1;
        let d = x - self.mean;
        self.mean += d / self.n as f64;
        // Weighted incremental variance; centering at the first sample keeps
        // small spreads accurate even when the whole lane has a large offset.
        let term = d * d * (previous as f64 / self.n as f64);
        self.lost |= (d != 0. && term == 0.) || !term.is_finite();
        self.m2 += term;
        self.error += term.abs() * (8. * f64::EPSILON);
    }
    fn arx_remove(&mut self, x: f64) {
        if self.n <= 1 {
            *self = Self::default();
        } else {
            let d = (x - self.origin) - self.mean;
            let previous = self.n;
            self.n -= 1;
            self.mean -= d / self.n as f64;
            let term = d * d * (previous as f64 / self.n as f64);
            self.m2 -= term;
            self.error += term.abs() * (8. * f64::EPSILON);
            if self.n == 1 {
                self.m2 = 0.;
                self.error = 0.;
            }
        }
    }
    fn arx_merge_constant(&mut self, x: f64, n: usize) {
        if n == 0 {
            return;
        }
        if self.n == 0 {
            self.n = n;
            self.origin = x;
            self.mean = 0.;
            self.m2 = 0.;
            return;
        }
        let total = self.n + n;
        let delta = (x - self.origin) - self.mean;
        let term = delta * delta * (self.n as f64 / total as f64) * n as f64;
        self.lost |= (delta != 0. && term == 0.) || !term.is_finite();
        self.m2 += term;
        self.mean += delta * (n as f64 / total as f64);
        self.n = total;
    }
}
/// Coordinate-compressed frequency tree: O(log n) updates and median queries,
/// with a weighted endpoint fill (even a huge window never allocates padding).
struct ArxMedian {
    keys: Vec<ArxNumber>,
    tree: Vec<usize>,
    count: usize,
}
impl ArxMedian {
    fn arx_change(&mut self, x: ArxNumber, n: usize, add: bool) {
        let mut i = self.keys.binary_search(&x).expect("compressed key") + 1;
        if add {
            self.count += n;
        } else {
            self.count -= n;
        }
        while i < self.tree.len() {
            if add {
                self.tree[i] += n;
            } else {
                self.tree[i] -= n;
            }
            i += i.isolate_lowest_one();
        }
    }
    fn arx_rank(&self, mut rank: usize) -> ArxNumber {
        let mut i = 0;
        let mut bit = self.tree.len().next_power_of_two() / 2;
        while bit > 0 {
            let next = i + bit;
            if next < self.tree.len() && self.tree[next] <= rank {
                rank -= self.tree[next];
                i = next;
            }
            bit /= 2;
        }
        self.keys[i]
    }
    fn arx_median(&self) -> ArxNumber {
        if self.count == 0 {
            ArxNumber::Real(f64::NAN)
        } else if self.count % 2 == 1 {
            self.arx_rank(self.count / 2)
        } else {
            self.arx_rank(self.count / 2 - 1)
                .arx_average(self.arx_rank(self.count / 2))
        }
    }
}
struct ArxRangeMin {
    tree: Vec<usize>,
    base: usize,
}
impl ArxRangeMin {
    fn arx_new(order: &[usize], cancel: &std::sync::atomic::AtomicBool) -> Result<Self> {
        let base = order.len().max(1).next_power_of_two();
        let mut tree = vec![usize::MAX; base * 2];
        for (source, target) in order.chunks(4096).zip(tree[base..].chunks_mut(4096)) {
            crate::dsp::check_cancel(cancel)?;
            target[..source.len()].copy_from_slice(source);
        }
        for i in (1..base).rev() {
            if i.is_multiple_of(4096) {
                crate::dsp::check_cancel(cancel)?;
            }
            tree[i] = tree[i * 2].min(tree[i * 2 + 1]);
        }
        Ok(Self { tree, base })
    }
    fn arx_get(&self, mut lo: usize, mut hi: usize) -> Option<usize> {
        lo += self.base;
        hi += self.base;
        let mut value = usize::MAX;
        while lo < hi {
            if lo % 2 == 1 {
                value = value.min(self.tree[lo]);
                lo += 1;
            }
            if hi % 2 == 1 {
                hi -= 1;
                value = value.min(self.tree[hi]);
            }
            lo /= 2;
            hi /= 2;
        }
        (value != usize::MAX).then_some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::sync::atomic::Ordering;

    thread_local! {static CANCEL_AT: Cell<Option<usize>> = const {Cell::new(None)};}
    pub(super) fn arx_cancel_after_progress(engine: &Engine, index: usize) {
        CANCEL_AT.with(|at| {
            if at.get().is_some_and(|n| index >= n) {
                at.set(None);
                engine.cancel.store(true, Ordering::Relaxed);
            }
        });
    }

    #[test]
    fn arx_moving_cancels_after_progress_in_a_single_lane() {
        let e = Engine::default();
        let samples = (0..16384)
            .map(|i| ArxNumber::Real((i % 79) as f64))
            .collect::<Vec<_>>();
        let options = super::super::stats_more::ArxMovingOptions {
            before: 32,
            after: 32,
            dim: 1,
            omit_nan: true,
            fill: None,
            discard: false,
        };
        for name in ["movmax", "movmin", "movmedian", "movstd", "movvar"] {
            e.cancel.store(false, Ordering::Relaxed);
            CANCEL_AT.with(|at| at.set(Some(4096)));
            let mut out = vec![ArxNumber::Real(0.); samples.len()];
            let result = e.arx_moving_lane(name, &samples, &options, None, false, &mut out);
            assert!(result.unwrap_err().contains("cancelled"), "{name}");
            assert!(CANCEL_AT.with(|at| at.get().is_none()));
        }
    }

    #[test]
    fn arx_budget_rejects_before_repetition_or_moving_materialization() {
        let e = Engine {
            evaluated_elements: 32 * 1024 * 1024,
            ..Engine::default()
        };
        let a = Matrix::from_column_vec(1, 3, vec![1., 2., 3.]).unwrap();
        let pointer = a.data().as_ptr();
        for name in ["repelem", "movmax", "movmedian", "movvar"] {
            let result = e.arx_call(name, &[Value::Matrix(a.clone()), arx_scalar(2.)]);
            assert!(result.unwrap_err().contains("evaluation budget"), "{name}");
            assert_eq!(a.data().as_ptr(), pointer);
        }
    }

    #[test]
    fn arx_sort_cancels_during_a_merge() {
        let e = Engine::default();
        let mut values = (0..32768).rev().collect::<Vec<_>>();
        CANCEL_AT.with(|at| at.set(Some(4096)));
        assert!(
            e.arx_sort(&mut values, Ord::cmp)
                .unwrap_err()
                .contains("cancelled")
        );
        assert!(CANCEL_AT.with(|at| at.get().is_none()));
    }

    #[test]
    fn arx_histogram_does_not_budget_unrequested_index_outputs() {
        let e = Engine {
            evaluated_elements: 32 * 1024 * 1024 - 128,
            ..Engine::default()
        };
        let a = Matrix::from_column_vec(1, 256, (0..256).map(|i| i as f64).collect()).unwrap();
        let args = [Value::Matrix(a.clone()), Value::Matrix(a), arx_scalar(8.)];
        assert_eq!(e.arx_outputs("histcounts2", &args, 1).unwrap().len(), 1);
        assert!(
            e.arx_outputs("histcounts2", &args, 5)
                .unwrap_err()
                .contains("evaluation budget")
        );
    }
}
