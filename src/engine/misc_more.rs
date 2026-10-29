//! Bit/hex, two-dimensional shape, geometry and miscellaneous functions.
//! Immutable inputs are shared; only transformations allocate new payloads.
use super::{Engine, Value};
use crate::typed_array::{NumericClass, TypedArray, TypedData};
use crate::{Array, CharacterRow, Complex64, ComplexMatrix, LogicalMatrix, Matrix, Result};

const MSC_ND: &str = "N-D arrays are not supported yet";
const MSC_BUDGET: usize = 32 * 1024 * 1024;

/// Smith's hexcone (SIGGRAPH 1978), HSV to RGB: the hue circle `6 h` splits into
/// six sectors `k` with fraction `f`; with `p = v (1 - s)`, `q = v (1 - s f)` and
/// `t = v (1 - s (1 - f))` each sector is one ordering of `v, p, q, t`. Hue is
/// periodic, so `h = 1` is sector 0 again.
fn hexcone_to_rgb(h: f64, s: f64, v: f64) -> [f64; 3] {
    let h6 = 6. * h;
    let sector = h6.floor();
    let f = h6 - sector;
    let p = v * (1. - s);
    let q = v * (1. - s * f);
    let t = v * (1. - s * (1. - f));
    match (sector as i64).rem_euclid(6) {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

/// Smith's hexcone (SIGGRAPH 1978), RGB to HSV: `v` is the largest component,
/// `s` the spread over `v`, and the hue places the remaining two components in
/// the sector of the largest one. Black has `s = 0`; grays have `h = 0`.
fn rgb_to_hexcone(r: f64, g: f64, b: f64) -> [f64; 3] {
    let v = r.max(g).max(b);
    let spread = v - r.min(g).min(b);
    let s = if v == 0. { 0. } else { spread / v };
    if spread == 0. {
        return [0., s, v];
    }
    let sector = if r == v {
        (g - b) / spread
    } else if g == v {
        2. + (b - r) / spread
    } else {
        4. + (r - g) / spread
    };
    let h = sector / 6.;
    [if h < 0. { h + 1. } else { h }, s, v]
}

impl Engine {
    pub(super) fn msc_call(&mut self, name: &str, args: &[Value]) -> Result<Option<Value>> {
        Ok(Some(match name {
            "bitget" | "bitset" => self.msc_bits(name, args)?,
            "swapbytes" => self.msc_swapbytes(args)?,
            "num2hex" => self.msc_num2hex(args)?,
            "hex2num" => self.msc_hex2num(args)?,
            "flipdim" | "permute" | "ipermute" => self.msc_shape(name, args)?,
            "shiftdim" | "inpolygon" | "peaks" | "hsv2rgb" | "rgb2hsv" => {
                self.msc_outputs(name, args, 1)?.remove(0)
            }
            "filter2" => self.msc_filter2(args)?,
            "interpft" => self.msc_interpft(args)?,
            "interp1q" => self.msc_interp1q(args)?,
            "rectint" => self.msc_rectint(args)?,
            "polyarea" => self.msc_polyarea(args)?,
            "atan2d" => self.msc_atan2d(args)?,
            "full" => {
                self.msc_arity(args, 1, 1)?;
                let a = args[0].clone().array(&self.cancel)?;
                if matches!(
                    a,
                    Array::Cell(_) | Array::Struct(_) | Array::Handle(_) | Array::Graphics(_)
                ) {
                    return Err(format!(
                        "Undefined function 'full' for input arguments of type '{}'.",
                        a.class()
                    ));
                }
                args[0].clone()
            }
            _ => return Ok(None),
        }))
    }

    pub(super) fn msc_outputs(
        &self,
        name: &str,
        args: &[Value],
        count: usize,
    ) -> Result<Vec<Value>> {
        self.check_cancel()?;
        let max = match name {
            "shiftdim" | "inpolygon" => 2,
            _ => 3,
        };
        if count == 0 || count > max {
            return Err("Too many output arguments.".into());
        }
        let mut outputs = match name {
            "shiftdim" => self.msc_shiftdim(args)?,
            "inpolygon" => self.msc_inpolygon(args, count)?,
            "peaks" => self.msc_peaks(args, count)?,
            "hsv2rgb" | "rgb2hsv" => self.msc_colors(name, args, count)?,
            _ => return Err("Unsupported miscellaneous output form".into()),
        };
        outputs.truncate(count);
        Ok(outputs)
    }

    fn msc_arity(&self, args: &[Value], min: usize, max: usize) -> Result<()> {
        self.check_cancel()?;
        if args.len() < min {
            Err("Not enough input arguments.".into())
        } else if args.len() > max {
            Err("Too many input arguments.".into())
        } else {
            Ok(())
        }
    }

    fn msc_admit(&self, rows: usize, cols: usize, buffers: usize) -> Result<usize> {
        self.check_cancel()?;
        let n = Matrix::checked_len(rows, cols)?;
        if n.checked_mul(buffers)
            .and_then(|v| v.checked_add(self.evaluated_elements))
            .is_none_or(|v| v > MSC_BUDGET)
        {
            return Err("Miscellaneous function exceeds the evaluation element budget".into());
        }
        Ok(n)
    }

    fn msc_real(&self, value: &Value) -> Result<Matrix> {
        match value {
            Value::Matrix(_) | Value::Logical(_) | Value::Container(Array::Typed(_)) => {
                value.clone().matrix(&self.cancel)
            }
            Value::Complex(_) => Err("Inputs must be real.".into()),
            _ => Err("Inputs must be numeric.".into()),
        }
    }

    fn msc_scalar(&self, value: &Value) -> Result<f64> {
        let a = self.msc_real(value)?;
        if a.len() != 1 {
            return Err("Input must be a scalar.".into());
        }
        Ok(a.data()[0])
    }

    fn msc_integer(&self, value: &Value, min: f64) -> Result<usize> {
        let v = self.msc_scalar(value)?;
        if !v.is_finite() || v < min || v.fract() != 0. || v > crate::matrix::MAX_ELEMENTS as f64 {
            return Err("Input must be a bounded integer.".into());
        }
        Ok(v as usize)
    }

    fn msc_text(&self, value: &Value) -> Result<String> {
        if !value.is_character() {
            return Err("Input must be a character vector.".into());
        }
        self.character_argument(value)?.to_utf8(&self.cancel)
    }

    fn msc_broadcast(&self, shapes: &[(usize, usize)]) -> Result<(usize, usize)> {
        let mut shape = (1, 1);
        for &s in shapes {
            if s == (1, 1) {
                continue;
            }
            if shape != (1, 1) && shape != s {
                return Err("Matrix dimensions must agree.".into());
            }
            shape = s;
        }
        self.msc_admit(shape.0, shape.1, 1)?;
        Ok(shape)
    }

    fn msc_bits(&self, name: &str, args: &[Value]) -> Result<Value> {
        let set = name == "bitset";
        self.msc_arity(args, 2, if set { 4 } else { 3 })?;
        let typed = match &args[0] {
            Value::Container(Array::Typed(a)) if a.is_integer() => Some(a),
            Value::Matrix(_) => None,
            _ => return Err("Input must be a double or integer array.".into()),
        };
        let source_class = typed.map(TypedArray::class);
        let option = args.last().filter(|v| v.is_character());
        let assumed = option.map(|v| self.msc_text(v)).transpose()?;
        let class = if let Some(s) = &assumed {
            let c = NumericClass::parse(s)
                .filter(|c| c.is_integer())
                .ok_or("Invalid ASSUMEDTYPE.")?;
            if source_class.is_some_and(|source| source != c) {
                return Err("ASSUMEDTYPE must match the integer input class.".into());
            }
            c
        } else {
            source_class.unwrap_or(NumericClass::Uint64)
        };
        if (!set && args.len() == 3 && option.is_none())
            || (set && args.len() == 4 && option.is_none())
        {
            return Err("Invalid ASSUMEDTYPE.".into());
        }
        let width = class.size() * 8;
        let a = if typed.is_none() {
            Some(self.msc_real(&args[0])?)
        } else {
            None
        };
        let bit = self.msc_real(&args[1])?;
        let state = if set && args.len() >= 3 && !args[2].is_character() {
            self.msc_real(&args[2])?
        } else {
            Matrix::scalar(1.)
        };
        let shape_a = args[0].shape(&self.cancel)?;
        let (rows, cols) = self.msc_broadcast(&[
            shape_a,
            (bit.rows(), bit.cols()),
            (state.rows(), state.cols()),
        ])?;
        let n = self.msc_admit(rows, cols, 3)?;
        let mut result = Vec::with_capacity(n);
        for i in 0..n {
            if i.is_multiple_of(4096) {
                self.check_cancel()?;
            }
            let b = bit.data()[if bit.len() == 1 { 0 } else { i }];
            if !b.is_finite() || b.fract() != 0. || b < 1. || b > width as f64 {
                return Err("The BIT input is out of range for the datatype specified.".into());
            }
            let k = if shape_a == (1, 1) { 0 } else { i };
            let v = if let Some(t) = typed {
                t.int(k)
            } else {
                let v = a.as_ref().unwrap().data()[k];
                if !v.is_finite()
                    || v.fract() != 0.
                    || (v as i128) < class.min()
                    || (v as i128) > class.max()
                {
                    return Err(
                        "Double inputs must have integer values in the range of ASSUMEDTYPE."
                            .into(),
                    );
                }
                v as i128
            };
            let mask = 1i128 << (b as usize - 1);
            let mut r = if set {
                if state.data()[if state.len() == 1 { 0 } else { i }] == 0. {
                    v & !mask
                } else {
                    v | mask
                }
            } else {
                i128::from(v & mask != 0)
            };
            if set {
                r &= (1i128 << width) - 1;
                if class.is_signed() && r & (1i128 << (width - 1)) != 0 {
                    r -= 1i128 << width;
                }
            }
            result.push(r);
        }
        if let Some(c) = source_class {
            Ok(Value::Container(Array::Typed(TypedArray::from_ints(
                c,
                rows,
                cols,
                result.into_iter(),
            )?)))
        } else {
            Ok(Value::Matrix(Matrix::from_column_vec(
                rows,
                cols,
                result.into_iter().map(|v| v as f64).collect(),
            )?))
        }
    }

    fn msc_swapbytes(&self, args: &[Value]) -> Result<Value> {
        self.msc_arity(args, 1, 1)?;
        match &args[0] {
            Value::Container(Array::Typed(a)) => {
                self.msc_admit(a.rows(), a.cols(), 2)?;
                let width = a.class().size();
                if width == 1 {
                    return Ok(args[0].clone());
                }
                let mut bytes = a.to_le_bytes();
                for (i, chunk) in bytes.chunks_exact_mut(width).enumerate() {
                    if i.is_multiple_of(4096) {
                        self.check_cancel()?;
                    }
                    chunk.reverse();
                }
                Ok(Value::Container(Array::Typed(TypedArray::from_le_bytes(
                    a.class(),
                    a.rows(),
                    a.cols(),
                    &bytes,
                )?)))
            }
            Value::Matrix(a) => {
                self.msc_admit(a.rows(), a.cols(), 1)?;
                let mut data = Vec::with_capacity(a.len());
                for (i, &v) in a.data().iter().enumerate() {
                    if i.is_multiple_of(4096) {
                        self.check_cancel()?;
                    }
                    data.push(f64::from_bits(v.to_bits().swap_bytes()));
                }
                Ok(Value::Matrix(Matrix::from_column_vec(
                    a.rows(),
                    a.cols(),
                    data,
                )?))
            }
            _ => Err("The first input argument must be a full, non-complex numeric value.".into()),
        }
    }

    fn msc_num2hex(&self, args: &[Value]) -> Result<Value> {
        self.msc_arity(args, 1, 1)?;
        let (rows, width) = match &args[0] {
            Value::Matrix(a) => (a.len(), 16),
            Value::Container(Array::Typed(a)) if a.class() == NumericClass::Single => (a.len(), 8),
            Value::Complex(_) => return Err("Input must be real.".into()),
            other => {
                return Err(format!(
                    "Inputs must be floating point, and may not be of class {}.",
                    other.clone().array(&self.cancel)?.class()
                ));
            }
        };
        CharacterRow::checked_len(rows, width)?;
        let mut units = vec![0; rows * width];
        for i in 0..rows {
            if i.is_multiple_of(1024) {
                self.check_cancel()?;
            }
            let bits = match &args[0] {
                Value::Matrix(a) => a.data()[i].to_bits(),
                Value::Container(Array::Typed(a)) => match a.data() {
                    TypedData::Single(data) => u64::from(data[i].to_bits()),
                    _ => unreachable!("floating-point admission above"),
                },
                _ => unreachable!(),
            };
            for j in 0..width {
                let digit = ((bits >> (4 * (width - 1 - j))) & 15) as u8;
                units[j * rows + i] = u16::from(if digit < 10 {
                    b'0' + digit
                } else {
                    b'a' + digit - 10
                });
            }
        }
        Ok(Value::Container(Array::Character(
            CharacterRow::from_column_major(rows, width, units, &self.cancel)?,
        )))
    }

    fn msc_hex2num(&self, args: &[Value]) -> Result<Value> {
        self.msc_arity(args, 1, 1)?;
        let width = 16;
        let a = args[0].clone().array(&self.cancel)?;
        let n = match &a {
            Array::Character(c) => c.rows(),
            Array::Cell(c) => c.len(),
            _ => {
                return Err(
                    "Input must be a character array or cell array of character vectors.".into(),
                );
            }
        };
        if n == 0 {
            return Ok(Value::Matrix(Matrix::from_column_vec(0, 0, Vec::new())?));
        }
        self.msc_admit(n, 1, 1)?;
        let mut data = Vec::with_capacity(n);
        for i in 0..n {
            if i.is_multiple_of(1024) {
                self.check_cancel()?;
            }
            let units: Vec<u16> = match &a {
                Array::Character(c) => (0..c.cols()).map(|j| c.units()[j * c.rows() + i]).collect(),
                Array::Cell(c) => match c.get_linear(i)?.as_ref() {
                    Array::Character(s) if s.rows() <= 1 => s.units().to_vec(),
                    _ => return Err("Input must contain character vectors.".into()),
                },
                _ => unreachable!(),
            };
            if units.len() > width {
                return Err(
                    "Input strings must not exceed the floating-point hexadecimal width.".into(),
                );
            }
            let mut bits = 0u64;
            for j in 0..width {
                // Blanks and missing trailing digits decode as '0'.
                let digit = match units.get(j).copied().unwrap_or(u16::from(b'0')) {
                    32 => 0,
                    u @ 48..=57 => u - 48,
                    u @ 65..=70 => u - 65 + 10,
                    u @ 97..=102 => u - 97 + 10,
                    _ => {
                        return Err(
                            "Input string to hex2num should have just 0-9, a-f, or A-F.".into()
                        );
                    }
                };
                bits = (bits << 4) | u64::from(digit);
            }
            data.push(f64::from_bits(bits));
        }
        Ok(Value::Matrix(Matrix::from_column_vec(n, 1, data)?))
    }

    fn msc_reshape(&self, a: &Array, rows: usize, cols: usize) -> Result<Value> {
        self.check_cancel()?;
        Ok(match a {
            Array::Numeric(a) => Value::Matrix(a.reshape(rows, cols)?),
            Array::Logical(a) => Value::Logical(a.reshape(rows, cols)?),
            Array::Complex(a) => Value::Complex(a.reshape(rows, cols, &self.cancel)?),
            Array::Typed(a) => Value::Container(Array::Typed(a.reshaped(rows, cols)?)),
            Array::Character(a) => {
                Value::Container(Array::Character(a.reshape(rows, cols, &self.cancel)?))
            }
            Array::Cell(a) => Value::Container(Array::Cell(a.reshape(rows, cols, &self.cancel)?)),
            Array::Struct(a) => {
                Value::Container(Array::Struct(a.reshape(rows, cols, &self.cancel)?))
            }
            _ if (a.rows(), a.cols()) == (rows, cols) => Value::from(a.clone()),
            _ => return Err("Shape operations do not support this class yet".into()),
        })
    }

    fn msc_gather(
        &self,
        a: &Array,
        rows: usize,
        cols: usize,
        positions: &[usize],
    ) -> Result<Value> {
        self.msc_admit(rows, cols, 3)?;
        macro_rules! gather {
            ($values:expr) => {{
                let mut data = Vec::with_capacity(positions.len());
                for chunk in positions.chunks(4096) {
                    self.check_cancel()?;
                    data.extend(chunk.iter().map(|&i| $values[i]));
                }
                data
            }};
        }
        Ok(match a {
            Array::Numeric(a) => {
                Value::Matrix(Matrix::from_column_vec(rows, cols, gather!(a.data()))?)
            }
            Array::Logical(a) => Value::Logical(LogicalMatrix::from_canonical_vec(
                rows,
                cols,
                gather!(a.data()),
            )?),
            Array::Complex(a) => Value::Complex(ComplexMatrix::from_column_vec(
                rows,
                cols,
                gather!(a.data()),
                &self.cancel,
            )?),
            Array::Typed(a) => Value::Container(Array::Typed(a.select_with_cancel(
                rows,
                cols,
                positions,
                &self.cancel,
            )?)),
            Array::Character(a) => Value::Container(Array::Character(
                CharacterRow::from_column_major(rows, cols, gather!(a.units()), &self.cancel)?,
            )),
            Array::Cell(a) => {
                let mut slots = Vec::with_capacity(positions.len());
                for (i, &p) in positions.iter().enumerate() {
                    if i.is_multiple_of(1024) {
                        self.check_cancel()?;
                    }
                    slots.push(a.get_linear(p)?.clone());
                }
                Value::Container(Array::Cell(crate::CellArray::from_shared(
                    rows,
                    cols,
                    slots,
                    &self.cancel,
                )?))
            }
            Array::Struct(a) => Value::Container(Array::Struct(a.select(
                rows,
                cols,
                positions,
                &self.cancel,
            )?)),
            _ => return Err("Shape operations do not support this class yet".into()),
        })
    }

    fn msc_shape(&self, name: &str, args: &[Value]) -> Result<Value> {
        if name == "flipdim" && args.len() != 2 {
            return Err("Requires two arguments.".into());
        }
        self.msc_arity(args, 2, 2)?;
        let a = args[0].clone().array(&self.cancel)?;
        let (r, c) = (a.rows(), a.cols());
        if name == "flipdim" {
            let dim = self.msc_integer(&args[1], 1.)?;
            if dim > 2 || (dim == 1 && r <= 1) || (dim == 2 && c <= 1) {
                return Ok(args[0].clone());
            }
            self.msc_admit(r, c, 3)?;
            let mut positions = Vec::with_capacity(a.len());
            for i in 0..a.len() {
                if i.is_multiple_of(4096) {
                    self.check_cancel()?;
                }
                positions.push(if dim == 1 {
                    (i / r) * r + r - 1 - i % r
                } else {
                    (c - 1 - i / r) * r + i % r
                });
            }
            return self.msc_gather(&a, r, c, &positions);
        }
        let order = self.msc_real(&args[1])?;
        if order.len() < 2 {
            return Err("ORDER must have at least N elements for an N-D array.".into());
        }
        if order.rows() != 1 && order.cols() != 1 {
            return Err("ORDER must be a vector.".into());
        }
        self.msc_admit(order.len(), 1, 3)?;
        // Only axes 1 and 2 carry data, but trailing singleton axes can move.
        let mut seen = vec![false; order.len()];
        let mut axes = Vec::with_capacity(order.len());
        for (i, &v) in order.data().iter().enumerate() {
            if i.is_multiple_of(4096) {
                self.check_cancel()?;
            }
            if !v.is_finite() || v.fract() != 0. || v < 1. || v > order.len() as f64 {
                return Err("ORDER contains an invalid permutation index.".into());
            }
            let k = v as usize - 1;
            if seen[k] {
                return Err("ORDER cannot contain repeated permutation indices.".into());
            }
            seen[k] = true;
            axes.push(k);
        }
        if name == "ipermute" {
            let mut inverse = vec![0; axes.len()];
            for (i, &k) in axes.iter().enumerate() {
                if i.is_multiple_of(4096) {
                    self.check_cancel()?;
                }
                inverse[k] = i;
            }
            axes = inverse;
        }
        let size = |k| match k {
            0 => r,
            1 => c,
            _ => 1,
        };
        for (i, &k) in axes.iter().enumerate().skip(2) {
            if i.is_multiple_of(4096) {
                self.check_cancel()?;
            }
            if size(k) != 1 {
                return Err(MSC_ND.into());
            }
        }
        let (rows, cols) = (size(axes[0]), size(axes[1]));
        if axes[0] != 1 && axes[1] != 0 || r <= 1 || c <= 1 {
            return self.msc_reshape(&a, rows, cols);
        }
        self.msc_admit(rows, cols, 3)?;
        let mut positions = Vec::with_capacity(a.len());
        for i in 0..a.len() {
            if i.is_multiple_of(4096) {
                self.check_cancel()?;
            }
            positions.push((i % rows) * r + i / rows);
        }
        self.msc_gather(&a, rows, cols, &positions)
    }

    fn msc_shiftdim(&self, args: &[Value]) -> Result<Vec<Value>> {
        self.msc_arity(args, 1, 2)?;
        let a = args[0].clone().array(&self.cancel)?;
        let (r, c) = (a.rows(), a.cols());
        let shift = if args.len() == 1 {
            f64::from(r == 1 && c != 1)
        } else {
            self.msc_scalar(&args[1])?
        };
        if !shift.is_finite()
            || shift.fract() != 0.
            || shift.abs() > crate::matrix::MAX_ELEMENTS as f64
        {
            return Err("N must be a bounded integer.".into());
        }
        let value = if args.len() == 1 {
            if shift == 1. {
                self.msc_reshape(&a, c, 1)?
            } else {
                args[0].clone()
            }
        } else if shift < 0. {
            if shift == -1. && c == 1 {
                self.msc_reshape(&a, 1, r)?
            } else if r == 1 && c == 1 {
                args[0].clone()
            } else {
                return Err(MSC_ND.into());
            }
        } else if (shift as usize).is_multiple_of(2) {
            args[0].clone()
        } else {
            self.msc_shape(
                "permute",
                &[
                    args[0].clone(),
                    Value::Matrix(Matrix::from_column_vec(1, 2, vec![2., 1.])?),
                ],
            )?
        };
        Ok(vec![value, Value::Matrix(Matrix::scalar(shift))])
    }

    fn msc_filter2(&mut self, args: &[Value]) -> Result<Value> {
        self.msc_arity(args, 2, 3)?;
        let shape = if args.len() == 3 {
            self.msc_text(&args[2])?
        } else {
            "same".into()
        };
        if !matches!(shape.as_str(), "full" | "same" | "valid") {
            return Err("SHAPE must be 'full', 'same', or 'valid'.".into());
        }
        // The reference's filter2 converts non-floating inputs to double first.
        let to_double = |v: &Value| -> Result<Value> {
            match v {
                Value::Logical(_) => Ok(Value::Matrix(self.msc_real(v)?)),
                Value::Container(Array::Typed(t)) if t.is_integer() => {
                    Ok(Value::Matrix(self.msc_real(v)?))
                }
                _ => Ok(v.clone()),
            }
        };
        let (kernel_arg, data_arg) = (to_double(&args[0])?, to_double(&args[1])?);
        let kernel = kernel_arg.array(&self.cancel)?;
        self.msc_admit(kernel.rows(), kernel.cols(), 3)?;
        let mut positions = Vec::with_capacity(kernel.len());
        for i in (0..kernel.len()).rev() {
            if i.is_multiple_of(4096) {
                self.check_cancel()?;
            }
            positions.push(i);
        }
        let rotated = self.msc_gather(&kernel, kernel.rows(), kernel.cols(), &positions)?;
        // Correlation is convolution with the kernel rotated (not conjugated).
        self.polyopt_call(
            "conv2",
            &[data_arg, rotated, Value::Text(shape)],
            &mut super::Execution::default(),
        )?
        .ok_or_else(|| "Internal conv2 dispatch failed".into())
    }

    fn msc_interpft(&self, args: &[Value]) -> Result<Value> {
        self.msc_arity(args, 2, 3)?;
        let input = match &args[0] {
            Value::Matrix(_) | Value::Complex(_) | Value::Logical(_) => args[0].clone(),
            Value::Container(Array::Typed(a)) if a.class() == NumericClass::Single => {
                Value::Matrix(a.to_matrix()?)
            }
            _ => return Err("interpft requires floating-point input.".into()),
        };
        let (r, c) = input.shape(&self.cancel)?;
        if r == 0 || c == 0 {
            return Err("Index exceeds matrix dimensions.".into());
        }
        let m = self.msc_integer(&args[1], 0.)?;
        let dim = if args.len() == 3 {
            self.msc_integer(&args[2], 1.)?
        } else if r != 1 {
            1
        } else {
            2
        };
        if dim > 2 {
            return Err(MSC_ND.into());
        }
        if m == 0 {
            return Ok(Value::Matrix(Matrix::filled(0, 0, 0.)?));
        }
        let n = if dim == 1 { r } else { c };
        let lanes = if dim == 1 { c } else { r };
        let stride = n.div_ceil(m).max(1);
        let padded = m.checked_mul(stride).ok_or("Interpolation size overflow")?;
        let (pr, pc) = if dim == 1 {
            (padded, lanes)
        } else {
            (lanes, padded)
        };
        self.msc_admit(pr, pc, 8)?;
        if padded > crate::dsp::MAX_FFT_LENGTH {
            return Err("FFT length exceeds the 1048576-point transform limit".into());
        }
        let spectrum = self.fourier_with_retained(
            "fft",
            &[
                input.clone(),
                Value::Matrix(Matrix::scalar(n as f64)),
                Value::Matrix(Matrix::scalar(dim as f64)),
            ],
            pr * pc * 2,
        )?;
        let spectrum = spectrum.complex(&self.cancel)?;
        let mut values = vec![Complex64::new(0., 0.); pr * pc];
        let index = |lane, k, rows| {
            if dim == 1 {
                lane * rows + k
            } else {
                k * rows + lane
            }
        };
        for lane in 0..lanes {
            self.check_cancel()?;
            for k in 0..n {
                if k.is_multiple_of(4096) {
                    self.check_cancel()?;
                }
                let value = spectrum.data()[index(lane, k, r)];
                if n.is_multiple_of(2) && k == n / 2 && padded > n {
                    values[index(lane, k, pr)] = value * 0.5;
                    values[index(lane, padded - k, pr)] = value * 0.5;
                } else {
                    let destination = if k <= n / 2 { k } else { padded - (n - k) };
                    values[index(lane, destination, pr)] = value;
                }
            }
        }
        let padded_spectrum = Value::Complex(ComplexMatrix::from_column_vec(
            pr,
            pc,
            values,
            &self.cancel,
        )?);
        let interpolated = self.fourier_with_retained(
            "ifft",
            &[
                padded_spectrum,
                Value::Matrix(Matrix::scalar(padded as f64)),
                Value::Matrix(Matrix::scalar(dim as f64)),
            ],
            spectrum.len() * 2,
        )?;
        let (rows, cols) = if dim == 1 { (m, lanes) } else { (lanes, m) };
        self.msc_admit(rows, cols, 2)?;
        let scale = padded as f64 / n as f64;
        let z = interpolated.complex(&self.cancel)?;
        let mut data = Vec::with_capacity(rows * cols);
        for j in 0..cols {
            for i in 0..rows {
                if data.len().is_multiple_of(4096) {
                    self.check_cancel()?;
                }
                data.push(
                    z.data()[if dim == 1 {
                        j * pr + i * stride
                    } else {
                        j * stride * pr + i
                    }] * scale,
                );
            }
        }
        if matches!(input, Value::Complex(_)) {
            Ok(Value::Complex(ComplexMatrix::from_column_vec(
                rows,
                cols,
                data,
                &self.cancel,
            )?))
        } else {
            let a = Matrix::from_column_vec(rows, cols, data.into_iter().map(|z| z.re).collect())?;
            if matches!(&args[0], Value::Container(Array::Typed(_))) {
                Ok(Value::Container(Array::Typed(TypedArray::from_matrix(
                    NumericClass::Single,
                    &a,
                )?)))
            } else {
                Ok(Value::Matrix(a))
            }
        }
    }

    fn msc_interp1q(&self, args: &[Value]) -> Result<Value> {
        self.msc_arity(args, 3, 3)?;
        let x = self.msc_real(&args[0])?;
        let y = self.msc_real(&args[1])?;
        let xi = self.msc_real(&args[2])?;
        if x.cols() != 1
            || x.rows() < 2
            || y.rows() != x.rows()
            || (xi.cols() != 1 && !xi.is_empty())
        {
            return Err("Index exceeds matrix dimensions.".into());
        }
        for (i, pair) in x.data().windows(2).enumerate() {
            if i.is_multiple_of(4096) {
                self.check_cancel()?;
            }
            if pair[0].is_nan() || pair[1] <= pair[0] || pair[1].is_nan() {
                return Err("X must be strictly increasing.".into());
            }
        }
        let n = self.msc_admit(xi.len(), y.cols(), 1)?;
        let mut data = vec![f64::NAN; n];
        for (i, &q) in xi.data().iter().enumerate() {
            if i.is_multiple_of(1024) {
                self.check_cancel()?;
            }
            if q.is_nan() || q < x.data()[0] || q > x.data()[x.len() - 1] {
                continue;
            }
            let hi = x.data().partition_point(|&v| v <= q).min(x.len() - 1);
            let lo = hi - 1;
            let weight = (q - x.data()[lo]) / (x.data()[hi] - x.data()[lo]);
            for col in 0..y.cols() {
                if col.is_multiple_of(4096) {
                    self.check_cancel()?;
                }
                let offset = col * y.rows();
                data[col * xi.len() + i] = y.data()[offset + lo]
                    + weight * (y.data()[offset + hi] - y.data()[offset + lo]);
            }
        }
        Ok(Value::Matrix(Matrix::from_column_vec(
            xi.len(),
            y.cols(),
            data,
        )?))
    }

    fn msc_rectint(&self, args: &[Value]) -> Result<Value> {
        self.msc_arity(args, 2, 2)?;
        let a = self.msc_real(&args[0])?;
        let b = self.msc_real(&args[1])?;
        if a.cols() < 4 || b.cols() < 4 {
            return Err("Index exceeds matrix dimensions.".into());
        }
        let n = self.msc_admit(a.rows(), b.rows(), 1)?;
        let mut data = Vec::with_capacity(n);
        for j in 0..b.rows() {
            for i in 0..a.rows() {
                if data.len().is_multiple_of(4096) {
                    self.check_cancel()?;
                }
                let (ax, ay, aw, ah) = (
                    a.data()[i],
                    a.data()[a.rows() + i],
                    a.data()[2 * a.rows() + i],
                    a.data()[3 * a.rows() + i],
                );
                let (bx, by, bw, bh) = (
                    b.data()[j],
                    b.data()[b.rows() + j],
                    b.data()[2 * b.rows() + j],
                    b.data()[3 * b.rows() + j],
                );
                data.push(
                    ((ax + aw).min(bx + bw) - ax.max(bx)).max(0.)
                        * ((ay + ah).min(by + bh) - ay.max(by)).max(0.),
                );
            }
        }
        Ok(Value::Matrix(Matrix::from_column_vec(
            a.rows(),
            b.rows(),
            data,
        )?))
    }

    fn msc_polyarea(&self, args: &[Value]) -> Result<Value> {
        self.msc_arity(args, 2, 3)?;
        let x = self.msc_real(&args[0])?;
        let y = self.msc_real(&args[1])?;
        if (x.rows(), x.cols()) != (y.rows(), y.cols()) {
            return Err("X and Y must be the same size.".into());
        }
        let dim = if args.len() == 3 {
            self.msc_integer(&args[2], 1.)?
        } else if x.rows() == 1 {
            2
        } else {
            1
        };
        if dim > 2 {
            // Every lane along a trailing singleton axis is one vertex.
            self.msc_admit(x.rows(), x.cols(), 1)?;
            return Ok(Value::Matrix(Matrix::filled(x.rows(), x.cols(), 0.)?));
        }
        if x.is_empty() {
            // The result has the shape of sum(x, dim); sum([]) is the scalar 0.
            let (rows, cols) = if x.rows() == 0 && x.cols() == 0 && args.len() == 2 {
                (1, 1)
            } else if dim == 1 {
                (1, x.cols())
            } else {
                (x.rows(), 1)
            };
            self.msc_admit(rows, cols, 1)?;
            return Ok(Value::Matrix(Matrix::filled(rows, cols, 0.)?));
        }
        let (n, lanes) = if dim == 1 {
            (x.rows(), x.cols())
        } else {
            (x.cols(), x.rows())
        };
        let (rows, cols) = if dim == 1 { (1, lanes) } else { (lanes, 1) };
        self.msc_admit(rows, cols, 1)?;
        let mut areas = Vec::with_capacity(lanes);
        for lane in 0..lanes {
            self.check_cancel()?;
            let index = |k| {
                if dim == 1 {
                    lane * n + k
                } else {
                    k * lanes + lane
                }
            };
            let mut twice_area = 0.;
            for k in 0..n {
                if k.is_multiple_of(4096) {
                    self.check_cancel()?;
                }
                let (i, j) = (index(k), index((k + 1) % n));
                twice_area += x.data()[i] * y.data()[j] - x.data()[j] * y.data()[i];
            }
            areas.push(twice_area.abs() * 0.5);
        }
        Ok(Value::Matrix(Matrix::from_column_vec(rows, cols, areas)?))
    }

    fn msc_inpolygon(&self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        self.msc_arity(args, 4, 4)?;
        let x = self.msc_real(&args[0])?;
        let y = self.msc_real(&args[1])?;
        let xv = self.msc_real(&args[2])?;
        let yv = self.msc_real(&args[3])?;
        if xv.len() != yv.len()
            || (xv.rows() > 1 && xv.cols() > 1)
            || (yv.rows() > 1 && yv.cols() > 1)
        {
            return Err("Polygon vertices must be vectors of equal length.".into());
        }
        let (rows, cols) = self.msc_broadcast(&[(x.rows(), x.cols()), (y.rows(), y.cols())])?;
        let n = self.msc_admit(rows, cols, if count == 2 { 2 } else { 1 })?;
        if (n as u128) * (xv.len() as u128) > (1u128 << 32) {
            return Err("inpolygon exceeds the bounded point-edge work limit".into());
        }
        // NaN-separated loops use a nonzero winding number. Equal orientation
        // overlaps stay inside; reversing an inner loop makes a hole (case 97).
        self.msc_admit(xv.len(), 1, 2)?;
        let mut edges = Vec::with_capacity(xv.len());
        let mut start = 0;
        for end in 0..=xv.len() {
            if end.is_multiple_of(4096) {
                self.check_cancel()?;
            }
            if end == xv.len() || xv.data()[end].is_nan() || yv.data()[end].is_nan() {
                if end > start {
                    for i in start..end {
                        if i.is_multiple_of(4096) {
                            self.check_cancel()?;
                        }
                        edges.push((i, if i + 1 == end { start } else { i + 1 }));
                    }
                }
                start = end + 1;
            }
        }
        let mut inside = Vec::with_capacity(n);
        let mut boundary = Vec::with_capacity(if count == 2 { n } else { 0 });
        for k in 0..n {
            self.check_cancel()?;
            let px = x.data()[if x.len() == 1 { 0 } else { k }];
            let py = y.data()[if y.len() == 1 { 0 } else { k }];
            let mut winding = 0i64;
            let mut on = false;
            if px.is_finite() && py.is_finite() {
                for (e, &(i, j)) in edges.iter().enumerate() {
                    if e.is_multiple_of(4096) {
                        self.check_cancel()?;
                    }
                    let (ax, ay, bx, by) = (xv.data()[i], yv.data()[i], xv.data()[j], yv.data()[j]);
                    let cross = (bx - ax) * (py - ay) - (by - ay) * (px - ax);
                    // Scale the rounding allowance to the local cross products.
                    let tolerance = 3.
                        * f64::EPSILON
                        * ((bx - ax).abs() * (py - ay).abs() + (by - ay).abs() * (px - ax).abs());
                    if cross.abs() <= tolerance
                        && px >= ax.min(bx)
                        && px <= ax.max(bx)
                        && py >= ay.min(by)
                        && py <= ay.max(by)
                    {
                        on = true;
                        break;
                    }
                    if ay <= py && by > py && cross > 0. {
                        winding += 1;
                    } else if ay > py && by <= py && cross < 0. {
                        winding -= 1;
                    }
                }
            }
            inside.push(u8::from(on || winding != 0));
            if count == 2 {
                boundary.push(u8::from(on));
            }
        }
        let mut outputs = vec![Value::Logical(LogicalMatrix::from_canonical_vec(
            rows, cols, inside,
        )?)];
        if count == 2 {
            outputs.push(Value::Logical(LogicalMatrix::from_canonical_vec(
                rows, cols, boundary,
            )?));
        }
        Ok(outputs)
    }

    fn msc_peaks(&self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        self.msc_arity(args, 0, 2)?;
        let (x, y) = if args.len() == 2 {
            let x = self.msc_real(&args[0])?;
            let y = self.msc_real(&args[1])?;
            let (r, c) = self.msc_broadcast(&[(x.rows(), x.cols()), (y.rows(), y.cols())])?;
            let n = self.msc_admit(r, c, 3)?;
            let expand = |a: Matrix| -> Result<Matrix> {
                if a.len() == 1 && n != 1 {
                    Matrix::filled(r, c, a.data()[0])
                } else {
                    Ok(a)
                }
            };
            (expand(x)?, expand(y)?)
        } else {
            let coordinates = if args.is_empty() {
                None
            } else {
                let a = self.msc_real(&args[0])?;
                if a.len() == 1 { None } else { Some(a) }
            };
            let n = if let Some(a) = &coordinates {
                a.len()
            } else if args.is_empty() {
                49
            } else {
                self.msc_integer(&args[0], 0.)?
            };
            self.msc_admit(n, n, 3)?;
            let mut xs = Vec::with_capacity(n * n);
            let mut ys = Vec::with_capacity(n * n);
            let coordinate = |i| {
                coordinates.as_ref().map_or_else(
                    || {
                        if n <= 1 {
                            3.
                        } else {
                            -3. + 6. * i as f64 / (n - 1) as f64
                        }
                    },
                    |a| a.data()[i],
                )
            };
            for j in 0..n {
                for i in 0..n {
                    if xs.len().is_multiple_of(4096) {
                        self.check_cancel()?;
                    }
                    xs.push(coordinate(j));
                    ys.push(coordinate(i));
                }
            }
            (
                Matrix::from_column_vec(n, n, xs)?,
                Matrix::from_column_vec(n, n, ys)?,
            )
        };
        self.msc_admit(x.rows(), x.cols(), 3)?;
        let mut z = Vec::with_capacity(x.len());
        for (i, (&x, &y)) in x.data().iter().zip(y.data()).enumerate() {
            if i.is_multiple_of(4096) {
                self.check_cancel()?;
            }
            z.push(
                3. * (1. - x).powi(2) * (-x.powi(2) - (y + 1.).powi(2)).exp()
                    - 10. * (x / 5. - x.powi(3) - y.powi(5)) * (-x.powi(2) - y.powi(2)).exp()
                    - (1. / 3.) * (-(x + 1.).powi(2) - y.powi(2)).exp(),
            );
        }
        let z = Value::Matrix(Matrix::from_column_vec(x.rows(), x.cols(), z)?);
        if count == 1 {
            Ok(vec![z])
        } else {
            Ok(vec![Value::Matrix(x), Value::Matrix(y), z])
        }
    }

    /// hsv2rgb and rgb2hsv by the hexcone model of A. R. Smith, "Color gamut
    /// transform pairs", SIGGRAPH 1978: an M-by-3 colormap, or three equal-size
    /// channel arrays for three outputs. One output is the M-by-3 map; three
    /// outputs are its columns (or the converted channels).
    fn msc_colors(&self, name: &str, args: &[Value], count: usize) -> Result<Vec<Value>> {
        self.msc_arity(args, 1, 3)?;
        if args.len() == 2 {
            return Err("Not enough input arguments.".into());
        }
        let forward = name == "hsv2rgb";
        let input = |value: &Value| -> Result<Matrix> {
            if forward {
                return self.msc_real(value);
            }
            match value {
                Value::Matrix(m) | Value::Container(Array::Numeric(m)) => Ok(m.clone()),
                Value::Complex(_) | Value::Container(Array::Complex(_)) => {
                    Err("Inputs must be real.".into())
                }
                other => Err(format!(
                    "Expected M to be one of these types:\n\ndouble\n\nInstead its type was {}.",
                    other.clone().array(&self.cancel)?.class()
                )),
            }
        };
        let convert = |a: f64, b: f64, c: f64| {
            if forward {
                hexcone_to_rgb(a, b, c)
            } else {
                rgb_to_hexcone(a, b, c)
            }
        };
        if args.len() == 3 {
            if count == 1 {
                return Err(MSC_ND.into());
            }
            let channels = [input(&args[0])?, input(&args[1])?, input(&args[2])?];
            let (rows, cols) = (channels[0].rows(), channels[0].cols());
            if channels
                .iter()
                .any(|m| (m.rows(), m.cols()) != (rows, cols))
            {
                return Err(if forward {
                    "H, S and V must have the same size."
                } else {
                    "R, G and B must have the same size."
                }
                .into());
            }
            let n = self.msc_admit(rows, cols, 3)?;
            let mut out = [Vec::new(), Vec::new(), Vec::new()];
            for channel in &mut out {
                channel.reserve_exact(n);
            }
            for i in 0..n {
                if i.is_multiple_of(4096) {
                    self.check_cancel()?;
                }
                let triple = convert(
                    channels[0].data()[i],
                    channels[1].data()[i],
                    channels[2].data()[i],
                );
                for (channel, value) in out.iter_mut().zip(triple) {
                    channel.push(value);
                }
            }
            return out
                .into_iter()
                .map(|data| Matrix::from_column_vec(rows, cols, data).map(Value::Matrix))
                .collect();
        }
        let map = input(&args[0])?;
        if map.cols() != 3 {
            return Err("MAP must be a Mx3 array.".into());
        }
        let m = map.rows();
        if forward {
            for (i, &v) in map.data().iter().enumerate() {
                if i.is_multiple_of(4096) {
                    self.check_cancel()?;
                }
                // NaN is not outside the range.
                if !v.is_nan() && !(0. ..=1.).contains(&v) {
                    return Err(
                        "Valid colormaps cannot have values outside the range [0,1].".into(),
                    );
                }
            }
        }
        let n = self.msc_admit(m, 3, 1)?;
        let mut data = vec![0.; n];
        let column = map.data();
        for i in 0..m {
            if i.is_multiple_of(4096) {
                self.check_cancel()?;
            }
            let triple = convert(column[i], column[m + i], column[2 * m + i]);
            for (k, value) in triple.into_iter().enumerate() {
                data[k * m + i] = value;
            }
        }
        if count == 1 {
            return Ok(vec![Value::Matrix(Matrix::from_column_vec(m, 3, data)?)]);
        }
        (0..3)
            .map(|k| Matrix::from_column_vec(m, 1, data[k * m..(k + 1) * m].to_vec()))
            .map(|channel| channel.map(Value::Matrix))
            .collect()
    }

    fn msc_atan2d(&self, args: &[Value]) -> Result<Value> {
        self.msc_arity(args, 2, 2)?;
        for v in args {
            if let Value::Container(Array::Typed(a)) = v
                && a.is_integer()
            {
                return Err(format!(
                    "Undefined function 'atan2d' for input arguments of type '{}'.",
                    a.class().name()
                ));
            }
        }
        let y = self.msc_real(&args[0])?;
        let x = self.msc_real(&args[1])?;
        let (r, c) = self.msc_broadcast(&[(y.rows(), y.cols()), (x.rows(), x.cols())])?;
        let n = self.msc_admit(r, c, 1)?;
        let mut data = Vec::with_capacity(n);
        for i in 0..n {
            if i.is_multiple_of(4096) {
                self.check_cancel()?;
            }
            let y = y.data()[if y.len() == 1 { 0 } else { i }];
            let x = x.data()[if x.len() == 1 { 0 } else { i }];
            data.push(if x == 0. && y == 0. {
                0.
            } else {
                y.atan2(x).to_degrees()
            });
        }
        let a = Matrix::from_column_vec(r, c, data)?;
        if args.iter().any(
            |v| matches!(v, Value::Container(Array::Typed(t)) if t.class() == NumericClass::Single),
        ) {
            Ok(Value::Container(Array::Typed(TypedArray::from_matrix(
                NumericClass::Single,
                &a,
            )?)))
        } else {
            Ok(Value::Matrix(a))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hexcone_sectors_and_inverse() {
        assert_eq!(hexcone_to_rgb(0., 1., 1.), [1., 0., 0.]);
        assert_eq!(hexcone_to_rgb(1. / 3., 1., 1.), [0., 1., 0.]);
        assert_eq!(hexcone_to_rgb(0.5, 0., 0.25), [0.25; 3]);
        assert_eq!(rgb_to_hexcone(0., 0., 0.), [0.; 3]);
        assert_eq!(rgb_to_hexcone(0.5, 0.5, 0.5), [0., 0., 0.5]);
        let [h, s, v] = rgb_to_hexcone(1., 0., 1.);
        assert!((h - 5. / 6.).abs() <= f64::EPSILON && s == 1. && v == 1.);
        for rgb in [
            [0.1, 0.3, 0.9],
            [0.9, 0.9, 0.2],
            [0.25, 0.5, 0.25],
            [0.7, 0.2, 0.4],
        ] {
            let [h, s, v] = rgb_to_hexcone(rgb[0], rgb[1], rgb[2]);
            let back = hexcone_to_rgb(h, s, v);
            for (a, b) in back.iter().zip(rgb) {
                assert!((a - b).abs() <= 4. * f64::EPSILON, "{rgb:?} -> {back:?}");
            }
        }
    }

    #[test]
    fn msc_polygon_edge_budget_precedes_allocation() {
        let engine = Engine {
            evaluated_elements: MSC_BUDGET - 3,
            ..Engine::default()
        };
        let vertices = Value::Matrix(Matrix::from_column_vec(1, 2, vec![0., 1.]).unwrap());
        let args = [
            Value::Matrix(Matrix::scalar(0.)),
            Value::Matrix(Matrix::scalar(0.)),
            vertices.clone(),
            vertices,
        ];
        // One result element fits, but two pairs of vertex indices do not.
        assert!(
            engine
                .msc_inpolygon(&args, 1)
                .unwrap_err()
                .contains("budget")
        );
    }
}
