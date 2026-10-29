//! Integer and `single` semantics: arithmetic, comparison, colon ranges,
//! concatenation, indexed assignment and the functions that keep or check the
//! class. Everything else sees these values as doubles (`Value::matrix`).
use super::{Engine, Execution, Value};
use crate::syntax::{Expr, Token};
use crate::typed_array::{NumericClass, TypedArray, divide_round, exact_with_double};
use crate::{Array, CharacterRow, LogicalMatrix, Matrix, Result};

const MIXED: &str =
    "Integers can only be combined with integers of the same class, or scalar doubles.";
const COMPLEX: &str = "Complex integer and complex single values are not supported yet";

/// Where an operand's values come from.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Double,
    Logical,
    Char,
}

enum Operand {
    Typed(TypedArray),
    Real {
        rows: usize,
        cols: usize,
        data: Vec<f64>,
        kind: Kind,
    },
}

impl Operand {
    fn shape(&self) -> (usize, usize) {
        match self {
            Self::Typed(a) => (a.rows(), a.cols()),
            Self::Real { rows, cols, .. } => (*rows, *cols),
        }
    }
    fn len(&self) -> usize {
        let (r, c) = self.shape();
        r * c
    }
    fn real(&self, i: usize) -> f64 {
        match self {
            Self::Typed(a) => a.real(i),
            Self::Real { data, .. } => data[i],
        }
    }
}

pub(super) fn is_typed(value: &Value) -> bool {
    matches!(value, Value::Container(Array::Typed(_)))
}

pub(super) fn to_logical(a: &TypedArray) -> Result<LogicalMatrix> {
    if !a.is_integer() && (0..a.len()).any(|i| a.real(i).is_nan()) {
        return Err("NaN's cannot be converted to logicals.".into());
    }
    let data = (0..a.len()).map(|i| u8::from(a.is_nonzero(i))).collect();
    LogicalMatrix::from_column_vec(a.rows(), a.cols(), data, &Default::default())
}

fn operand(value: Value) -> Result<Operand> {
    Ok(match value {
        Value::Container(Array::Typed(a)) => Operand::Typed(a),
        Value::Matrix(m) => Operand::Real {
            rows: m.rows(),
            cols: m.cols(),
            data: m.data().to_vec(),
            kind: Kind::Double,
        },
        Value::Logical(m) => Operand::Real {
            rows: m.rows(),
            cols: m.cols(),
            data: m.data().iter().map(|&v| f64::from(v)).collect(),
            kind: Kind::Logical,
        },
        Value::Text(s) | Value::QuotedString(s) => {
            let c = CharacterRow::from_text(&s, &Default::default())?;
            char_operand(&c)
        }
        Value::Container(Array::Character(c)) => char_operand(&c),
        Value::Complex(_) => return Err(COMPLEX.into()),
        _ => return Err("Arithmetic requires numeric or logical operands".into()),
    })
}

fn char_operand(c: &CharacterRow) -> Operand {
    Operand::Real {
        rows: c.rows(),
        cols: c.cols(),
        data: c.units().iter().map(|&u| f64::from(u)).collect(),
        kind: Kind::Char,
    }
}

fn typed_value(a: TypedArray) -> Value {
    Value::Container(Array::Typed(a))
}

/// Result shape of an element-wise operation: equal shapes or one scalar.
fn broadcast(a: &Operand, b: &Operand) -> Result<(usize, usize)> {
    if a.len() == 1 {
        Ok(b.shape())
    } else if b.len() == 1 || a.shape() == b.shape() {
        Ok(a.shape())
    } else {
        Err("Matrix dimensions must agree.".into())
    }
}

fn pick(len: usize, i: usize) -> usize {
    if len == 1 { 0 } else { i }
}

/// `x^n` for integers, saturating at the `i128` range (the caller saturates
/// into the class).
fn int_power(x: i128, n: u128) -> i128 {
    if x == -1 {
        return if n.is_multiple_of(2) { 1 } else { -1 };
    }
    let mut result: i128 = 1;
    for _ in 0..n {
        result = match result.checked_mul(x) {
            Some(v) if v.unsigned_abs() <= u64::MAX as u128 * 2 => v,
            _ => {
                return if x < 0 && n % 2 == 1 {
                    i128::MIN / 2
                } else {
                    i128::MAX / 2
                };
            }
        };
        if result == 0 || x == 1 {
            break;
        }
    }
    result
}

/// One element of `a op b` for integers of `class`, where either side may be
/// a double (`None` for its integer form).
fn int_element(
    class: NumericClass,
    op: char,
    a: (Option<i128>, f64),
    b: (Option<i128>, f64),
) -> Result<i128> {
    let wide = matches!(class, NumericClass::Int64 | NumericClass::Uint64);
    Ok(match (a.0, b.0) {
        (Some(x), Some(y)) => match op {
            '+' => class.saturate(x + y),
            '-' => class.saturate(x - y),
            '*' => class.saturate(x.checked_mul(y).unwrap_or(if (x < 0) != (y < 0) {
                i128::MIN
            } else {
                i128::MAX
            })),
            '/' => divide_round(x, y, class),
            '^' => {
                if y < 0 {
                    return Err("Integers can only be raised to positive integral powers.".into());
                } else {
                    class.saturate(int_power(x, y as u128))
                }
            }
            _ => unreachable!(),
        },
        (Some(x), None) | (None, Some(x)) => {
            let swap = a.0.is_none();
            let d = if swap { a.1 } else { b.1 };
            if op == '^' {
                if swap {
                    // Preserve the exponent's sign/parity before converting its
                    // magnitude: wide odd integers are rounded to even doubles.
                    if x < 0 {
                        return Err(
                            "Integers can only be raised to positive integral powers.".into()
                        );
                    }
                    let magnitude = d.abs().powf(x as f64);
                    let value = if d.is_sign_negative() && x % 2 != 0 {
                        -magnitude
                    } else {
                        magnitude
                    };
                    return Ok(class.from_f64(value));
                }
                if d.fract() != 0. || !d.is_finite() || d < 0. {
                    return Err("Integers can only be raised to positive integral powers.".into());
                }
                // Above the u128 range every representable double is even.
                let exponent = if d >= u128::MAX as f64 {
                    u128::MAX - 1
                } else {
                    d as u128
                };
                return Ok(class.saturate(int_power(x, exponent)));
            }
            if wide && let Some(v) = exact_with_double(x, d, op, swap) {
                return Ok(class.saturate(v));
            }
            let (l, r) = if swap { (d, x as f64) } else { (x as f64, d) };
            class.from_f64(match op {
                '+' => l + r,
                '-' => l - r,
                '*' => l * r,
                '/' => l / r,
                _ => unreachable!(),
            })
        }
        (None, None) => unreachable!(),
    })
}

/// The class of `a op b` for an element-wise operation, or the MATLAB error.
fn result_class(a: &Operand, b: &Operand) -> Result<NumericClass> {
    match (a, b) {
        (Operand::Typed(x), Operand::Typed(y)) => {
            if x.class() == y.class() {
                Ok(x.class())
            } else {
                Err(MIXED.into())
            }
        }
        (Operand::Typed(t), Operand::Real { kind, .. })
        | (Operand::Real { kind, .. }, Operand::Typed(t)) => {
            let other = if matches!(a, Operand::Typed(_)) { b } else { a };
            if t.is_integer() && (*kind == Kind::Logical || (other.len() != 1)) {
                Err(MIXED.into())
            } else {
                Ok(t.class())
            }
        }
        _ => unreachable!("one operand is typed"),
    }
}

fn elementwise(
    op: char,
    a: &Operand,
    b: &Operand,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<TypedArray> {
    let class = result_class(a, b)?;
    let (rows, cols) = broadcast(a, b)?;
    let n = Matrix::checked_len(rows, cols)?;
    let (la, lb) = (a.len(), b.len());
    if class == NumericClass::Single {
        // Both sides in single precision; f64 then rounding is exact for + - * /.
        let to_single = |o: &Operand, i: usize| f64::from(o.real(i) as f32);
        let mut values = Vec::with_capacity(n);
        for i in 0..n {
            if i.is_multiple_of(8192) {
                crate::dsp::check_cancel(cancel)?;
            }
            let x = to_single(a, pick(la, i));
            let y = to_single(b, pick(lb, i));
            values.push(match op {
                '+' => x + y,
                '-' => x - y,
                '*' => x * y,
                '/' => x / y,
                '^' => x.powf(y),
                _ => unreachable!(),
            });
        }
        return TypedArray::from_reals(class, rows, cols, values.into_iter());
    }
    let side = |o: &Operand, i: usize| match o {
        Operand::Typed(t) => (Some(t.int(i)), 0.),
        Operand::Real { data, .. } => (None, data[i]),
    };
    let mut values = Vec::with_capacity(n);
    for i in 0..n {
        if i.is_multiple_of(8192) {
            crate::dsp::check_cancel(cancel)?;
        }
        values.push(int_element(
            class,
            op,
            side(a, pick(la, i)),
            side(b, pick(lb, i)),
        )?);
    }
    TypedArray::from_ints(class, rows, cols, values.into_iter())
}

fn real_matrix(o: &Operand) -> Result<Matrix> {
    let (rows, cols) = o.shape();
    Matrix::from_column_vec(rows, cols, (0..o.len()).map(|i| o.real(i)).collect())
}

impl Engine {
    /// `a op b` where at least one side is an integer or single array.
    pub(super) fn typed_arithmetic(&mut self, op: &Token, a: Value, b: Value) -> Result<Value> {
        let (a, b) = (operand(a)?, operand(b)?);
        self.charge(a.len().max(b.len()))?;
        let scalar = a.len() == 1 || b.len() == 1;
        let integer = [&a, &b]
            .iter()
            .any(|o| matches!(o, Operand::Typed(t) if t.is_integer()));
        let result = match op {
            Token::Plus => elementwise('+', &a, &b, &self.cancel)?,
            Token::Minus => elementwise('-', &a, &b, &self.cancel)?,
            Token::DotStar => elementwise('*', &a, &b, &self.cancel)?,
            Token::DotSlash => elementwise('/', &a, &b, &self.cancel)?,
            Token::DotPower => elementwise('^', &a, &b, &self.cancel)?,
            Token::Star if scalar => elementwise('*', &a, &b, &self.cancel)?,
            Token::Slash if b.len() == 1 => elementwise('/', &a, &b, &self.cancel)?,
            Token::Backslash if a.len() == 1 => elementwise('/', &b, &a, &self.cancel)?,
            Token::Power if a.len() == 1 && b.len() == 1 => elementwise('^', &a, &b, &self.cancel)?,
            Token::Star if integer => {
                return Err("MTIMES is not fully supported for integer classes. At least one input must be scalar.\nTo compute elementwise TIMES, use TIMES (.*) instead.".into());
            }
            Token::Slash | Token::Backslash if integer => {
                let (name, word) = if op == &Token::Slash {
                    ("RDIVIDE", "./")
                } else {
                    ("LDIVIDE", ".\\")
                };
                return Err(format!(
                    "Linear algebra is not supported for integer data types.\nTo compute elementwise {name}, use {name} ({word}) instead."
                ));
            }
            Token::Power if integer => {
                return Err("Integers can only be raised to positive integral powers.".into());
            }
            Token::Star | Token::Slash | Token::Backslash | Token::Power => {
                // single matrix algebra: double kernels, rounded to single.
                let (x, y) = (real_matrix(&a)?, real_matrix(&b)?);
                let m = match op {
                    Token::Star => x.mul(&y)?,
                    Token::Backslash => x.solve(&y)?,
                    Token::Slash => y.transpose().solve(&x.transpose())?.transpose(),
                    _ => x.power(y.as_scalar()?)?,
                };
                TypedArray::from_matrix(NumericClass::Single, &m)?
            }
            _ => return Err("Unsupported operator".into()),
        };
        self.check_cancel()?;
        Ok(typed_value(result))
    }

    pub(super) fn typed_negate(&mut self, a: &TypedArray) -> Result<Value> {
        self.charge(a.len())?;
        let class = a.class();
        Ok(typed_value(if class == NumericClass::Single {
            TypedArray::from_reals(class, a.rows(), a.cols(), (0..a.len()).map(|i| -a.real(i)))?
        } else {
            TypedArray::from_ints(class, a.rows(), a.cols(), (0..a.len()).map(|i| -a.int(i)))?
        }))
    }

    /// Comparison with exact integer semantics (`int64(2^53+1) == 2^53` is false).
    pub(super) fn typed_compare(&mut self, op: &Token, a: Value, b: Value) -> Result<Value> {
        let (a, b) = (operand(a)?, operand(b)?);
        let (rows, cols) = broadcast(&a, &b)?;
        let n = Matrix::checked_len(rows, cols)?;
        self.charge(n)?;
        let (la, lb) = (a.len(), b.len());
        let mut data = Vec::with_capacity(n);
        for i in 0..n {
            if i.is_multiple_of(8192) {
                self.check_cancel()?;
            }
            let (x, y) = (&a, &b);
            let order = compare(x, pick(la, i), y, pick(lb, i));
            data.push(u8::from(match op {
                Token::Equal => order == Some(std::cmp::Ordering::Equal),
                Token::NotEqual => order != Some(std::cmp::Ordering::Equal),
                Token::Less => order == Some(std::cmp::Ordering::Less),
                Token::LessEqual => matches!(
                    order,
                    Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
                ),
                Token::Greater => order == Some(std::cmp::Ordering::Greater),
                Token::GreaterEqual => matches!(
                    order,
                    Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
                ),
                _ => return Err("Unsupported comparison".into()),
            }));
        }
        Ok(Value::Logical(LogicalMatrix::from_column_vec(
            rows,
            cols,
            data,
            &self.cancel,
        )?))
    }

    /// `start:step:stop` when an operand is an integer or single value.
    pub(super) fn typed_range(
        &mut self,
        start: Value,
        step: Option<Value>,
        stop: Value,
    ) -> Result<Value> {
        let parts: Vec<Operand> = [Some(start), step, Some(stop)]
            .into_iter()
            .flatten()
            .map(operand)
            .collect::<Result<_>>()?;
        if parts.iter().any(|p| p.len() != 1) {
            return Err("Colon operands must be scalars".into());
        }
        let mut class = None;
        for p in &parts {
            if let Operand::Typed(t) = p {
                match class {
                    None => class = Some(t.class()),
                    Some(c) if c == t.class() => {}
                    Some(_) => {
                        return Err("Colon operands must be all the same type, or mixed with real double scalar values.".into());
                    }
                }
            }
        }
        let class = class.expect("a typed operand");
        let (first, step, last) = match parts.as_slice() {
            [a, c] => (a, None, c),
            [a, b, c] => (a, Some(b), c),
            _ => unreachable!(),
        };
        if class == NumericClass::Single {
            let s = |o: &Operand| f64::from(o.real(0) as f32);
            let (a, d, b) = (s(first), step.map_or(1., s), s(last));
            if !a.is_finite() || !b.is_finite() || !d.is_finite() || d == 0. {
                return Err("Range bounds must be finite and step must be nonzero".into());
            }
            let span = ((b - a) / d) as f32;
            if span >= crate::matrix::MAX_ELEMENTS as f32 {
                return Err(format!(
                    "Matrix exceeds the {}-element limit",
                    crate::matrix::MAX_ELEMENTS
                ));
            }
            let count = if span < 0. {
                0
            } else {
                (f64::from(span) + 4. * f64::from(f32::EPSILON) * f64::from(span.abs().max(1.)))
                    .floor() as usize
                    + 1
            };
            Matrix::checked_len(1, count)?;
            self.charge(count)?;
            return Ok(typed_value(TypedArray::from_reals(
                class,
                1,
                count,
                (0..count).map(|k| a + k as f64 * d),
            )?));
        }
        let int = |o: &Operand| -> Result<i128> {
            match o {
                Operand::Typed(t) => Ok(t.int(0)),
                Operand::Real { data, .. } => {
                    let v = data[0];
                    if v.fract() != 0. || !v.is_finite() {
                        Err(format!(
                            "Double operands interacting with {} operands must have integer values.",
                            class.name()
                        ))
                    } else {
                        Ok(class.from_f64(v))
                    }
                }
            }
        };
        let (a, b) = (int(first)?, int(last)?);
        // The step is not saturated into the class (`uint8(5):-1:uint8(1)`).
        let d = match step {
            Some(Operand::Typed(t)) => t.int(0),
            Some(Operand::Real { data, .. }) => {
                let v = data[0];
                if v.fract() != 0. || !v.is_finite() {
                    return Err(format!(
                        "Double operands interacting with {} operands must have integer values.",
                        class.name()
                    ));
                }
                v as i128
            }
            None => 1,
        };
        if d == 0 {
            return Ok(typed_value(TypedArray::from_ints(
                class,
                1,
                0,
                std::iter::empty(),
            )?));
        }
        let count = if (b - a).signum() * d.signum() < 0 {
            0
        } else {
            let count = (b - a) / d + 1;
            if count > crate::matrix::MAX_ELEMENTS as i128 {
                return Err(format!(
                    "Matrix exceeds the {}-element limit",
                    crate::matrix::MAX_ELEMENTS
                ));
            }
            count as usize
        };
        Matrix::checked_len(1, count)?;
        self.charge(count)?;
        Ok(typed_value(TypedArray::from_ints(
            class,
            1,
            count,
            (0..count).map(|k| a + k as i128 * d),
        )?))
    }

    /// `[a b; c d]` with an integer or single operand: the leftmost integer
    /// class wins, then single. Character operands make a character result.
    pub(super) fn typed_concatenate(
        &mut self,
        rows: Vec<Vec<Value>>,
    ) -> Result<std::result::Result<Value, Vec<Vec<Value>>>> {
        let values = || rows.iter().flatten();
        if !values().any(is_typed) {
            return Ok(Err(rows));
        }
        if values().any(|v| {
            matches!(
                v,
                Value::Text(_) | Value::QuotedString(_) | Value::Container(Array::Character(_))
            )
        }) {
            // Character wins: every value joins as character codes.
            let code = |v: f64| NumericClass::Uint16.from_f64(v) as u16;
            let mut blocks: Vec<Vec<(usize, usize, Vec<u16>)>> = Vec::new();
            for row in rows {
                let mut out = Vec::new();
                for v in row {
                    let block = match v {
                        Value::Text(s) | Value::QuotedString(s) => {
                            let c = CharacterRow::from_text(&s, &self.cancel)?;
                            (c.rows(), c.cols(), c.units().to_vec())
                        }
                        Value::Container(Array::Character(c)) => {
                            (c.rows(), c.cols(), c.units().to_vec())
                        }
                        Value::Container(Array::Typed(t)) => (
                            t.rows(),
                            t.cols(),
                            (0..t.len()).map(|i| code(t.real(i))).collect(),
                        ),
                        Value::Matrix(m) => (
                            m.rows(),
                            m.cols(),
                            m.data().iter().map(|&v| code(v)).collect(),
                        ),
                        Value::Logical(m) => (
                            m.rows(),
                            m.cols(),
                            m.data().iter().map(|&v| u16::from(v)).collect(),
                        ),
                        _ => {
                            return Err(
                                "Concatenation of these classes is not supported yet".into()
                            );
                        }
                    };
                    if block.0 * block.1 > 0 {
                        out.push(block);
                    }
                }
                if !out.is_empty() {
                    blocks.push(out);
                }
            }
            let mut height = 0;
            let mut width = None;
            for row in &blocks {
                let h = row[0].0;
                let w: usize = row.iter().map(|b| b.1).sum();
                if row.iter().any(|b| b.0 != h) || width.is_some_and(|n| n != w) {
                    return Err(
                        "Dimensions of arrays being concatenated are not consistent.".into(),
                    );
                }
                width = Some(w);
                height += h;
            }
            let width = width.unwrap_or(0);
            let mut units = vec![0u16; Matrix::checked_len(height, width)?];
            let mut r = 0;
            for row in &blocks {
                let h = row[0].0;
                let mut c = 0;
                for (rows, cols, data) in row {
                    for col in 0..*cols {
                        for k in 0..*rows {
                            units[(c + col) * height + r + k] = data[col * rows + k];
                        }
                    }
                    c += cols;
                }
                r += h;
            }
            return Ok(Ok(Value::Container(
                CharacterRow::from_column_major(height, width, units, &self.cancel)?.into(),
            )));
        }
        let class = values()
            .filter_map(|v| match v {
                Value::Container(Array::Typed(t)) if t.is_integer() => Some(t.class()),
                _ => None,
            })
            .next()
            .unwrap_or(NumericClass::Single);
        let mut pieces: Vec<Vec<TypedArray>> = Vec::new();
        for row in rows {
            let mut out = Vec::new();
            for v in row {
                let t = match v {
                    Value::Container(Array::Typed(t)) => t.cast(class)?,
                    Value::Matrix(m) => TypedArray::from_matrix(class, &m)?,
                    Value::Logical(m) => {
                        TypedArray::from_matrix(class, &m.to_numeric(&self.cancel)?)?
                    }
                    Value::Complex(_) => return Err(COMPLEX.into()),
                    _ => {
                        return Err("Concatenation of these classes is not supported yet".into());
                    }
                };
                if !t.is_empty() || (t.rows() > 0 && t.cols() == 0) {
                    out.push(t);
                }
            }
            if !out.is_empty() {
                pieces.push(out);
            }
        }
        let mut height = 0;
        let mut width = None;
        for row in &pieces {
            let h = row[0].rows();
            if row.iter().any(|m| m.rows() != h) {
                return Err("Dimensions of arrays being concatenated are not consistent.".into());
            }
            let w: usize = row.iter().map(TypedArray::cols).sum();
            if width.is_some_and(|n| n != w) {
                return Err("Dimensions of arrays being concatenated are not consistent.".into());
            }
            width = Some(w);
            height += h;
        }
        let width = width.unwrap_or(0);
        let n = Matrix::checked_len(height, width)?;
        self.charge(n)?;
        // Linear positions into a stacked source, then one gather.
        let mut sources: Vec<(usize, usize)> = vec![(0, 0); n];
        let flat: Vec<&TypedArray> = pieces.iter().flatten().collect();
        let mut r = 0;
        let mut piece = 0;
        for row in &pieces {
            let h = row[0].rows();
            let mut c = 0;
            for m in row {
                self.check_cancel()?;
                for col in 0..m.cols() {
                    for k in 0..h {
                        sources[(c + col) * height + r + k] = (piece, col * h + k);
                    }
                }
                c += m.cols();
                piece += 1;
            }
            r += h;
        }
        let result = if class.is_integer() {
            TypedArray::from_ints(
                class,
                height,
                width,
                sources.iter().map(|&(p, i)| flat[p].int(i)),
            )?
        } else {
            TypedArray::from_reals(
                class,
                height,
                width,
                sources.iter().map(|&(p, i)| flat[p].real(i)),
            )?
        };
        Ok(Ok(typed_value(result)))
    }

    /// Native indexed assignment. Borrow metadata before evaluating the RHS and
    /// subscripts, then mutate the workspace Arc only after validation. No
    /// artificial target clone or f64 marker matrix is created.
    pub(super) fn typed_assign_indexed(
        &mut self,
        name: &str,
        args: &[Expr],
        rhs: &Expr,
        quiet: bool,
        out: &mut Execution,
    ) -> Result<bool> {
        use super::indexing::Indices;
        let original = match self.container_variables.get(name) {
            Some(Array::Typed(t)) => Some((t.rows(), t.cols(), t.class())),
            Some(_) => return Ok(false),
            None if self.contains_array(name) => return Ok(false),
            None => None,
        };
        let value = self.eval(rhs, None, out)?;
        let deletion = matches!(rhs, Expr::Matrix(rows) if rows.is_empty());
        let (nr, nc, class) = match original {
            Some(metadata) => metadata,
            None => match &value {
                Value::Container(Array::Typed(t)) => (0, 0, t.class()),
                _ => {
                    return self
                        .assign_indexed_value(name, args, value, deletion, quiet, out)
                        .map(|_| true);
                }
            },
        };
        if args.is_empty() || args.len() > 2 {
            return Err("Indexing supports one or two subscripts".into());
        }
        let source = match value {
            Value::Container(Array::Typed(t)) => {
                if t.class() != class {
                    self.charge((t.len() * class.size() + 64).div_ceil(8))?;
                }
                t.cast(class)?
            }
            Value::Complex(_) => return Err(COMPLEX.into()),
            other => {
                // Logical/character conversion owns a temporary f64 buffer;
                // charge it before conversion as well as the native result.
                if matches!(&other, Value::Logical(_)) || other.is_character() {
                    let (rows, cols) = other.shape(&self.cancel)?;
                    self.charge(Matrix::checked_len(rows, cols)?)?;
                }
                let m = other.matrix(&self.cancel)?;
                self.charge((m.len() * class.size() + 64).div_ceil(8))?;
                TypedArray::from_matrix(class, &m)?
            }
        };
        let bound = if args.len() == 1 { nr * nc } else { nr };
        let limit = crate::matrix::MAX_ELEMENTS;
        let mut rows = self
            .subscript_with_limit(
                &args[0],
                original.map(|_| bound),
                if deletion { bound } else { limit },
                out,
            )?
            .0;
        let mut columns = if args.len() == 2 {
            Some(
                self.subscript_with_limit(
                    &args[1],
                    original.map(|_| nc),
                    if deletion { nc } else { limit },
                    out,
                )?
                .0,
            )
        } else {
            None
        };
        if deletion {
            let result = self.typed_delete_indexed(name, args, &rows, columns.as_ref())?;
            self.assign_workspace_array(name, result.into(), quiet, out)?;
            return Ok(true);
        }
        let all = |len| Indices::Stride {
            start: 0,
            step: 1,
            len,
        };
        if original.is_none() && args.len() == 1 && self.colon_subscript(&args[0]) {
            if !source.is_empty() && source.rows() != 1 && source.cols() != 1 {
                return Err("A new variable assigned through A(:) requires a vector".into());
            }
            rows = all(source.len());
        }
        if (nr, nc) == (0, 0)
            && let Some(columns) = &mut columns
        {
            let row_colon = self.colon_subscript(&args[0]);
            let col_colon = self.colon_subscript(&args[1]);
            if row_colon && col_colon {
                rows = all(source.rows());
                *columns = all(source.cols());
            } else if row_colon {
                rows = all(if columns.len() == 0 {
                    0
                } else if columns.len() == 1 && (source.rows() == 1 || source.cols() == 1) {
                    source.len()
                } else {
                    source.rows()
                });
            } else if col_colon {
                *columns = all(if rows.len() == 0 {
                    0
                } else if rows.len() == 1 && (source.rows() == 1 || source.cols() == 1) {
                    source.len()
                } else {
                    source.cols()
                });
            }
        }
        let count = match &columns {
            Some(c) => Matrix::checked_len(rows.len(), c.len())?,
            None => rows.len(),
        };
        if source.len() != 1 {
            if count != source.len() {
                return Err(
                    "Indexed assignment requires matching element counts or a scalar".into(),
                );
            }
            if count != 0
                && let Some(columns) = &columns
                && [rows.len(), columns.len()]
                    .into_iter()
                    .filter(|n| *n != 1)
                    .ne([source.rows(), source.cols()]
                        .into_iter()
                        .filter(|n| *n != 1))
            {
                return Err("Subscripted assignment dimensions must agree".into());
            }
        }
        let extent = typed_index_extent(&rows, &self.cancel)?;
        let shape = if let Some(columns) = &columns {
            (
                nr.max(extent),
                nc.max(typed_index_extent(columns, &self.cancel)?),
            )
        } else if extent > nr * nc || original.is_none() {
            if nr > 1 && nc > 1 {
                return Err("Cannot grow a matrix through one ambiguous linear subscript; use row and column indices".into());
            }
            if nc == 1 && nr != 1 {
                (extent, 1)
            } else {
                (1, extent)
            }
        } else {
            (nr, nc)
        };
        let len = Matrix::checked_len(shape.0, shape.1)?;
        let full = rows.contiguous() == Some(0)
            && match &columns {
                Some(c) => rows.len() == shape.0 && c.contiguous() == Some(0) && c.len() == shape.1,
                None => rows.len() == len,
            };
        if full && source.len() == len {
            // Complete replacement can share the RHS, even if the old target
            // has snapshots. There is no old payload to copy or undo.
            self.assign_workspace_array(
                name,
                source.reshaped(shape.0, shape.1)?.into(),
                quiet,
                out,
            )?;
            return Ok(true);
        }
        let address = |i| match &columns {
            Some(c) => rows.at(i % rows.len()) + c.at(i / rows.len()) * shape.0,
            None => rows.at(i),
        };
        // The same shape changes neither workspace size nor variable count.
        if original.is_some() && shape == (nr, nc) {
            if count != 0 {
                let Some(Array::Typed(target)) = self.container_variables.get(name) else {
                    unreachable!()
                };
                self.charge(target.write_allocation_bytes(count, true).div_ceil(8))?;
                self.check_cancel()?;
                let Some(Array::Typed(target)) = self.container_variables.get_mut(name) else {
                    unreachable!()
                };
                target.write_indices(count, address, &source, true, &self.cancel)?;
            }
            if !quiet {
                let text = self.display_text(name, &self.container_variables[name])?;
                self.emit(out, text)?;
            }
            return Ok(true);
        }
        if self.workspace_count() + usize::from(original.is_none()) > 10000 {
            return Err("Workspace exceeds 10000 variables; use clear".into());
        }
        let old_bytes = self
            .container_variables
            .get(name)
            .map_or(0, Array::retained_bytes);
        if self.workspace_bytes() - old_bytes + len * class.size() > 256 * 1024 * 1024 {
            let logical: usize = self.obj_workspace_bytes()
                + self.workspace().values().map(Array::bytes).sum::<usize>();
            if logical - old_bytes + len * class.size() > 256 * 1024 * 1024 {
                return Err(
                    "Workspace exceeds its 256 MiB logical storage limit; use clear".into(),
                );
            }
        }
        // One native allocation for the grown array, written before it is
        // published: a cancelled write just drops it, so no undo buffer.
        self.charge((len * class.size() + 64).div_ceil(8))?;
        let empty = TypedArray::from_ints(class, 0, 0, std::iter::empty())?;
        let old = match self.container_variables.get(name) {
            Some(Array::Typed(t)) => t,
            _ => &empty,
        };
        let mut result = old.remap(
            shape.0,
            shape.1,
            |i| {
                let (r, c) = (i % shape.0, i / shape.0);
                (r < nr && c < nc).then_some(c * nr + r)
            },
            &self.cancel,
        )?;
        result.write_indices(count, address, &source, false, &self.cancel)?;
        self.assign_workspace_array(name, result.into(), quiet, out)?;
        Ok(true)
    }

    fn typed_delete_indexed(
        &mut self,
        name: &str,
        args: &[Expr],
        rows: &super::indexing::Indices,
        columns: Option<&super::indexing::Indices>,
    ) -> Result<TypedArray> {
        let Some(Array::Typed(old)) = self.container_variables.get(name) else {
            return Err("Deletion requires an existing variable".into());
        };
        let (nr, nc, len, class) = (old.rows(), old.cols(), old.len(), old.class());
        if args.len() == 1 && self.colon_subscript(&args[0]) {
            return TypedArray::from_ints(class, 0, 0, std::iter::empty());
        }
        let (indices, bound, dimension) = if let Some(columns) = columns {
            if self.colon_subscript(&args[1]) {
                (rows, nr, 0)
            } else if self.colon_subscript(&args[0]) {
                (columns, nc, 1)
            } else {
                return Err("A null assignment requires at least one ':' subscript".into());
            }
        } else {
            (rows, len, 2)
        };
        self.charge(bound.div_ceil(8))?;
        let mut removed = vec![false; bound];
        let mut nremoved = 0;
        for i in 0..indices.len() {
            if i.is_multiple_of(8192) {
                self.check_cancel()?;
            }
            let at = indices.at(i);
            if !removed[at] {
                removed[at] = true;
                nremoved += 1;
            }
        }
        let shape = match dimension {
            0 => (nr - nremoved, nc),
            1 => (nr, nc - nremoved),
            _ if len == 0 => (nr, nc),
            _ if nc == 1 && nr != 1 => (len - nremoved, 1),
            _ => (1, len - nremoved),
        };
        let count = Matrix::checked_len(shape.0, shape.1)?;
        if nremoved == 0 {
            let Some(Array::Typed(old)) = self.container_variables.get(name) else {
                unreachable!()
            };
            return old.reshaped(shape.0, shape.1);
        }
        self.charge(bound - nremoved)?;
        let mut kept = Vec::with_capacity(bound - nremoved);
        for (i, skip) in removed.into_iter().enumerate() {
            if i.is_multiple_of(8192) {
                self.check_cancel()?;
            }
            if !skip {
                kept.push(i);
            }
        }
        self.charge((2 * count * class.size() + 32).div_ceil(8))?;
        let Some(Array::Typed(old)) = self.container_variables.get(name) else {
            unreachable!()
        };
        old.remap(
            shape.0,
            shape.1,
            |i| {
                Some(match dimension {
                    0 => kept[i % shape.0] + (i / shape.0) * nr,
                    1 => i % nr + kept[i / nr] * nr,
                    _ => kept[i],
                })
            },
            &self.cancel,
        )
    }
}

/// Compact ranges and masks have O(1) extents; only explicit index arrays scan.
fn typed_index_extent(
    indices: &super::indexing::Indices,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<usize> {
    use super::indexing::Indices;
    if indices.len() == 0 {
        return Ok(0);
    }
    match indices {
        Indices::Stride { len, .. } => Ok(indices.at(0).max(indices.at(len - 1)) + 1),
        Indices::LogicalStride { start, len, .. } => Ok(start + len),
        Indices::Mask(values, _) => Ok(values.last().map_or(0, |i| *i as usize + 1)),
        Indices::Explicit(values) => {
            let mut maximum = 0;
            for chunk in values.data().chunks(8192) {
                crate::dsp::check_cancel(cancel)?;
                for &value in chunk {
                    maximum = maximum.max(value as usize);
                }
            }
            Ok(maximum)
        }
    }
}

fn compare(a: &Operand, i: usize, b: &Operand, j: usize) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (Operand::Typed(x), Operand::Typed(y)) if x.is_integer() && y.is_integer() => {
            Some(x.int(i).cmp(&y.int(j)))
        }
        (Operand::Typed(x), other) if x.is_integer() => int_vs_real(x.int(i), other.real(j)),
        (other, Operand::Typed(y)) if y.is_integer() => {
            int_vs_real(y.int(j), other.real(i)).map(std::cmp::Ordering::reverse)
        }
        _ => a.real(i).partial_cmp(&b.real(j)),
    }
}

/// Exact order of an integer and a double.
pub(super) fn int_vs_real(x: i128, d: f64) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering::*;
    if d.is_nan() {
        return None;
    }
    if d >= 1e38 {
        return Some(Less);
    }
    if d <= -1e38 {
        return Some(Greater);
    }
    let floor = d.floor();
    let fi = floor as i128;
    Some(match x.cmp(&fi) {
        Less => Less,
        Greater => Greater,
        Equal if floor == d => Equal,
        Equal => Less,
    })
}
