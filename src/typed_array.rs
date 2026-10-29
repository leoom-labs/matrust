//! Integer (`int8` .. `uint64`) and `single` storage: immutable shared
//! column-major data at the class's own element size.
//!
//! Integer conversion rounds half away from zero and saturates; NaN becomes 0.
//! Arithmetic never panics: it works in `i128` and saturates into the class.
use crate::{Matrix, Result};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NumericClass {
    Int8,
    Uint8,
    Int16,
    Uint16,
    Int32,
    Uint32,
    Int64,
    Uint64,
    Single,
}

impl NumericClass {
    pub const INTEGERS: [Self; 8] = [
        Self::Int8,
        Self::Uint8,
        Self::Int16,
        Self::Uint16,
        Self::Int32,
        Self::Uint32,
        Self::Int64,
        Self::Uint64,
    ];
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "int8" => Self::Int8,
            "uint8" => Self::Uint8,
            "int16" => Self::Int16,
            "uint16" => Self::Uint16,
            "int32" => Self::Int32,
            "uint32" => Self::Uint32,
            "int64" => Self::Int64,
            "uint64" => Self::Uint64,
            "single" => Self::Single,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Int8 => "int8",
            Self::Uint8 => "uint8",
            Self::Int16 => "int16",
            Self::Uint16 => "uint16",
            Self::Int32 => "int32",
            Self::Uint32 => "uint32",
            Self::Int64 => "int64",
            Self::Uint64 => "uint64",
            Self::Single => "single",
        }
    }
    pub fn is_integer(self) -> bool {
        self != Self::Single
    }
    pub fn is_signed(self) -> bool {
        matches!(
            self,
            Self::Int8 | Self::Int16 | Self::Int32 | Self::Int64 | Self::Single
        )
    }
    /// Bytes per element.
    pub fn size(self) -> usize {
        match self {
            Self::Int8 | Self::Uint8 => 1,
            Self::Int16 | Self::Uint16 => 2,
            Self::Int32 | Self::Uint32 | Self::Single => 4,
            Self::Int64 | Self::Uint64 => 8,
        }
    }
    /// Range of an integer class (`single`: the largest finite magnitude, truncated).
    pub fn min(self) -> i128 {
        match self {
            Self::Int8 => i8::MIN.into(),
            Self::Int16 => i16::MIN.into(),
            Self::Int32 => i32::MIN.into(),
            Self::Int64 => i64::MIN.into(),
            Self::Single => -(f32::MAX as i128),
            _ => 0,
        }
    }
    pub fn max(self) -> i128 {
        match self {
            Self::Int8 => i8::MAX.into(),
            Self::Uint8 => u8::MAX.into(),
            Self::Int16 => i16::MAX.into(),
            Self::Uint16 => u16::MAX.into(),
            Self::Int32 => i32::MAX.into(),
            Self::Uint32 => u32::MAX.into(),
            Self::Int64 => i64::MAX.into(),
            Self::Uint64 => u64::MAX.into(),
            Self::Single => f32::MAX as i128,
        }
    }
    pub fn saturate(self, v: i128) -> i128 {
        v.clamp(self.min(), self.max())
    }
    /// Double to integer: round half away from zero, saturate, NaN to 0.
    pub fn from_f64(self, v: f64) -> i128 {
        if v.is_nan() {
            return 0;
        }
        let r = v.round();
        if r >= self.max() as f64 {
            self.max()
        } else if r <= self.min() as f64 {
            self.min()
        } else {
            r as i128
        }
    }
    /// Display width of one element: two spaces and the digits of `intmax`.
    pub fn display_width(self) -> usize {
        match self {
            Self::Int8 | Self::Uint8 => 5,
            Self::Int16 | Self::Uint16 => 7,
            Self::Int32 | Self::Uint32 => 12,
            _ => 21,
        }
    }
}

/// Elements sit behind a `Vec` header, like numeric `DMatrix` storage: a
/// filled `Vec` moves into its `Arc` without a copy, and a weak identity pins
/// only the header, never the payload.
#[derive(Clone, Debug)]
pub enum TypedData {
    Int8(Arc<Vec<i8>>),
    Uint8(Arc<Vec<u8>>),
    Int16(Arc<Vec<i16>>),
    Uint16(Arc<Vec<u16>>),
    Int32(Arc<Vec<i32>>),
    Uint32(Arc<Vec<u32>>),
    Int64(Arc<Vec<i64>>),
    Uint64(Arc<Vec<u64>>),
    Single(Arc<Vec<f32>>),
}

pub(crate) enum WeakTypedData {
    Int8(std::sync::Weak<Vec<i8>>),
    Uint8(std::sync::Weak<Vec<u8>>),
    Int16(std::sync::Weak<Vec<i16>>),
    Uint16(std::sync::Weak<Vec<u16>>),
    Int32(std::sync::Weak<Vec<i32>>),
    Uint32(std::sync::Weak<Vec<u32>>),
    Int64(std::sync::Weak<Vec<i64>>),
    Uint64(std::sync::Weak<Vec<u64>>),
    Single(std::sync::Weak<Vec<f32>>),
}
impl WeakTypedData {
    pub(crate) fn key(&self) -> (u8, usize) {
        match self {
            Self::Int8(data) => (6, data.as_ptr() as *const () as usize),
            Self::Uint8(data) => (7, data.as_ptr() as *const () as usize),
            Self::Int16(data) => (8, data.as_ptr() as *const () as usize),
            Self::Uint16(data) => (9, data.as_ptr() as *const () as usize),
            Self::Int32(data) => (10, data.as_ptr() as *const () as usize),
            Self::Uint32(data) => (11, data.as_ptr() as *const () as usize),
            Self::Int64(data) => (12, data.as_ptr() as *const () as usize),
            Self::Uint64(data) => (13, data.as_ptr() as *const () as usize),
            Self::Single(data) => (14, data.as_ptr() as *const () as usize),
        }
    }
}

/// Applies `$body` with `$d` bound to the typed slice of every integer variant;
/// `$single` handles the `f32` variant.
macro_rules! each_int {
    ($data:expr, $d:ident => $body:expr, $s:ident => $single:expr) => {
        match $data {
            TypedData::Int8($d) => $body,
            TypedData::Uint8($d) => $body,
            TypedData::Int16($d) => $body,
            TypedData::Uint16($d) => $body,
            TypedData::Int32($d) => $body,
            TypedData::Uint32($d) => $body,
            TypedData::Int64($d) => $body,
            TypedData::Uint64($d) => $body,
            TypedData::Single($s) => $single,
        }
    };
}

#[derive(Clone, Debug)]
pub struct TypedArray {
    rows: usize,
    cols: usize,
    data: TypedData,
}

fn reserve<T>(n: usize) -> Result<Vec<T>> {
    let mut v = Vec::new();
    v.try_reserve_exact(n)
        .map_err(|_| format!("Cannot allocate {n} elements"))?;
    Ok(v)
}

impl TypedArray {
    pub(crate) fn weak_storage(&self) -> WeakTypedData {
        match &self.data {
            TypedData::Int8(data) => WeakTypedData::Int8(Arc::downgrade(data)),
            TypedData::Uint8(data) => WeakTypedData::Uint8(Arc::downgrade(data)),
            TypedData::Int16(data) => WeakTypedData::Int16(Arc::downgrade(data)),
            TypedData::Uint16(data) => WeakTypedData::Uint16(Arc::downgrade(data)),
            TypedData::Int32(data) => WeakTypedData::Int32(Arc::downgrade(data)),
            TypedData::Uint32(data) => WeakTypedData::Uint32(Arc::downgrade(data)),
            TypedData::Int64(data) => WeakTypedData::Int64(Arc::downgrade(data)),
            TypedData::Uint64(data) => WeakTypedData::Uint64(Arc::downgrade(data)),
            TypedData::Single(data) => WeakTypedData::Single(Arc::downgrade(data)),
        }
    }
    /// Stream native storage with bounded endian conversion, never an f64 bridge.
    pub(crate) fn write_le_chunks(&self, mut write: impl FnMut(&[u8]) -> Result<()>) -> Result<()> {
        macro_rules! encode {
            ($data:expr) => {{
                #[cfg(target_endian = "little")]
                for chunk in bytemuck::cast_slice($data.as_slice()).chunks(65536) {
                    write(chunk)?;
                }
                #[cfg(target_endian = "big")]
                for chunk in $data.chunks(8192) {
                    let bytes: Vec<u8> = chunk.iter().flat_map(|v| v.to_le_bytes()).collect();
                    write(&bytes)?;
                }
            }};
        }
        each_int!(&self.data, d => encode!(d), s => encode!(s));
        Ok(())
    }
    pub(crate) fn from_le_reader(
        class: NumericClass,
        rows: usize,
        cols: usize,
        mut read: impl FnMut(&mut [u8]) -> Result<()>,
    ) -> Result<Self> {
        let len = Matrix::checked_len(rows, cols)?;
        macro_rules! decode {
            ($variant:ident, $t:ty) => {{
                let mut values = reserve(len)?;
                let mut buffer = [0u8; 65536];
                const WIDTH: usize = std::mem::size_of::<$t>();
                for start in (0..len).step_by(65536 / WIDTH) {
                    let bytes = &mut buffer[..(len - start).min(65536 / WIDTH) * WIDTH];
                    read(bytes)?;
                    values.extend(
                        bytes
                            .chunks_exact(WIDTH)
                            .map(|b| <$t>::from_le_bytes(b.try_into().unwrap())),
                    );
                }
                TypedData::$variant(values.into())
            }};
        }
        let data = match class {
            NumericClass::Int8 => decode!(Int8, i8),
            NumericClass::Uint8 => decode!(Uint8, u8),
            NumericClass::Int16 => decode!(Int16, i16),
            NumericClass::Uint16 => decode!(Uint16, u16),
            NumericClass::Int32 => decode!(Int32, i32),
            NumericClass::Uint32 => decode!(Uint32, u32),
            NumericClass::Int64 => decode!(Int64, i64),
            NumericClass::Uint64 => decode!(Uint64, u64),
            NumericClass::Single => decode!(Single, f32),
        };
        Self::checked(rows, cols, data)
    }

    pub fn rows(&self) -> usize {
        self.rows
    }
    pub fn cols(&self) -> usize {
        self.cols
    }
    pub fn len(&self) -> usize {
        self.rows * self.cols
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn data(&self) -> &TypedData {
        &self.data
    }
    pub fn class(&self) -> NumericClass {
        match &self.data {
            TypedData::Int8(_) => NumericClass::Int8,
            TypedData::Uint8(_) => NumericClass::Uint8,
            TypedData::Int16(_) => NumericClass::Int16,
            TypedData::Uint16(_) => NumericClass::Uint16,
            TypedData::Int32(_) => NumericClass::Int32,
            TypedData::Uint32(_) => NumericClass::Uint32,
            TypedData::Int64(_) => NumericClass::Int64,
            TypedData::Uint64(_) => NumericClass::Uint64,
            TypedData::Single(_) => NumericClass::Single,
        }
    }
    pub fn is_integer(&self) -> bool {
        self.class().is_integer()
    }
    pub fn bytes(&self) -> usize {
        self.len() * self.class().size()
    }
    pub fn same_storage(&self, other: &Self) -> bool {
        let a = each_int!(&self.data, d => Arc::as_ptr(d).cast::<()>(), s => Arc::as_ptr(s).cast::<()>());
        let b = each_int!(&other.data, d => Arc::as_ptr(d).cast::<()>(), s => Arc::as_ptr(s).cast::<()>());
        std::ptr::eq(a, b) && self.class() == other.class()
    }
    /// Element `i` as an integer (single: rounded and saturated like `int64`).
    pub fn int(&self, i: usize) -> i128 {
        each_int!(&self.data, d => i128::from(d[i]), s => NumericClass::Int64.from_f64(f64::from(s[i])))
    }
    /// Element `i` as a double (exact up to 2^53 for 64-bit integers).
    pub fn real(&self, i: usize) -> f64 {
        each_int!(&self.data, d => d[i] as f64, s => f64::from(s[i]))
    }
    pub fn is_nonzero(&self, i: usize) -> bool {
        each_int!(&self.data, d => d[i] != 0, s => s[i] != 0.)
    }
    pub fn to_f64_vec(&self) -> Result<Vec<f64>> {
        let mut out = reserve(self.len())?;
        out.extend((0..self.len()).map(|i| self.real(i)));
        Ok(out)
    }
    pub fn to_matrix(&self) -> Result<Matrix> {
        Matrix::from_column_vec(self.rows, self.cols, self.to_f64_vec()?)
    }
    /// Values already in range for `class` (saturated by the caller or not).
    pub fn from_ints(
        class: NumericClass,
        rows: usize,
        cols: usize,
        values: impl Iterator<Item = i128>,
    ) -> Result<Self> {
        let n = Matrix::checked_len(rows, cols)?;
        macro_rules! build {
            ($variant:ident, $t:ty) => {{
                let mut v: Vec<$t> = reserve(n)?;
                v.extend(values.take(n).map(|x| class.saturate(x) as $t));
                TypedData::$variant(v.into())
            }};
        }
        let data = match class {
            NumericClass::Int8 => build!(Int8, i8),
            NumericClass::Uint8 => build!(Uint8, u8),
            NumericClass::Int16 => build!(Int16, i16),
            NumericClass::Uint16 => build!(Uint16, u16),
            NumericClass::Int32 => build!(Int32, i32),
            NumericClass::Uint32 => build!(Uint32, u32),
            NumericClass::Int64 => build!(Int64, i64),
            NumericClass::Uint64 => build!(Uint64, u64),
            NumericClass::Single => {
                let mut v: Vec<f32> = reserve(n)?;
                v.extend(values.take(n).map(|x| x as f32));
                TypedData::Single(v.into())
            }
        };
        Self::checked(rows, cols, data)
    }
    /// Doubles converted to `class`: rounded and saturated, or to the nearest `f32`.
    pub fn from_reals(
        class: NumericClass,
        rows: usize,
        cols: usize,
        values: impl Iterator<Item = f64>,
    ) -> Result<Self> {
        if class == NumericClass::Single {
            let n = Matrix::checked_len(rows, cols)?;
            let mut v: Vec<f32> = reserve(n)?;
            v.extend(values.take(n).map(|x| x as f32));
            return Self::checked(rows, cols, TypedData::Single(v.into()));
        }
        Self::from_ints(class, rows, cols, values.map(|x| class.from_f64(x)))
    }
    pub fn from_matrix(class: NumericClass, m: &Matrix) -> Result<Self> {
        Self::from_reals(class, m.rows(), m.cols(), m.data().iter().copied())
    }
    pub fn scalar_int(class: NumericClass, v: i128) -> Self {
        Self::from_ints(class, 1, 1, std::iter::once(v)).expect("scalar")
    }
    pub fn scalar_real(class: NumericClass, v: f64) -> Self {
        Self::from_reals(class, 1, 1, std::iter::once(v)).expect("scalar")
    }
    pub(crate) fn checked(rows: usize, cols: usize, data: TypedData) -> Result<Self> {
        let len = each_int!(&data, d => d.len(), s => s.len());
        if len != Matrix::checked_len(rows, cols)? {
            return Err("Typed array data does not match its shape".into());
        }
        Ok(Self { rows, cols, data })
    }
    /// The same class and data under a new shape of equal length.
    pub fn reshaped(&self, rows: usize, cols: usize) -> Result<Self> {
        if Matrix::checked_len(rows, cols)? != self.len() {
            return Err("Number of elements must not change in reshape".into());
        }
        Ok(Self {
            rows,
            cols,
            data: self.data.clone(),
        })
    }
    /// Elements at linear `positions` of `self`, in a `rows`-by-`cols` result.
    pub fn select(&self, rows: usize, cols: usize, positions: &[usize]) -> Result<Self> {
        self.select_with_cancel(rows, cols, positions, &Default::default())
    }
    pub fn select_with_cancel(
        &self,
        rows: usize,
        cols: usize,
        positions: &[usize],
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<Self> {
        crate::dsp::check_cancel(cancel)?;
        if positions.len() != Matrix::checked_len(rows, cols)? {
            return Err("Selection does not match its shape".into());
        }
        if let Some(&bad) = positions.iter().find(|&&p| p >= self.len()) {
            return Err(format!(
                "Index exceeds matrix dimensions ({} > {})",
                bad + 1,
                self.len()
            ));
        }
        macro_rules! gather {
            ($variant:ident, $d:expr) => {{
                let mut v = reserve(positions.len())?;
                for chunk in positions.chunks(8192) {
                    crate::dsp::check_cancel(cancel)?;
                    v.extend(chunk.iter().map(|&p| $d[p]));
                }
                TypedData::$variant(v.into())
            }};
        }
        let data = match &self.data {
            TypedData::Int8(d) => gather!(Int8, d),
            TypedData::Uint8(d) => gather!(Uint8, d),
            TypedData::Int16(d) => gather!(Int16, d),
            TypedData::Uint16(d) => gather!(Uint16, d),
            TypedData::Int32(d) => gather!(Int32, d),
            TypedData::Uint32(d) => gather!(Uint32, d),
            TypedData::Int64(d) => gather!(Int64, d),
            TypedData::Uint64(d) => gather!(Uint64, d),
            TypedData::Single(d) => gather!(Single, d),
        };
        Ok(Self { rows, cols, data })
    }
    /// Actual scratch/payload bytes needed by an in-bounds write. Unique small
    /// writes allocate nothing; large unique writes retain an undo buffer so
    /// cancellation never exposes a partially updated workspace array.
    /// `rollback` is false for a private result the caller drops on error.
    pub(crate) fn write_allocation_bytes(&self, count: usize, rollback: bool) -> usize {
        let unique = each_int!(&self.data, d => Arc::strong_count(d) == 1 && Arc::weak_count(d) == 0,
            s => Arc::strong_count(s) == 1 && Arc::weak_count(s) == 0);
        if unique {
            if rollback && count > 8192 {
                count * self.class().size()
            } else {
                0
            }
        } else {
            // One payload copy, moved under a new Arc header.
            self.bytes() + 64
        }
    }

    /// Native class write, after the engine has validated every subscript and
    /// source dimension. The address callback is pure and always in bounds.
    pub(crate) fn write_indices(
        &mut self,
        count: usize,
        address: impl Fn(usize) -> usize,
        source: &Self,
        rollback: bool,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<()> {
        crate::dsp::check_cancel(cancel)?;
        fn write<T: Copy>(
            data: &mut [T],
            count: usize,
            address: &impl Fn(usize) -> usize,
            source: &[T],
            rollback: bool,
            cancel: &std::sync::atomic::AtomicBool,
        ) -> Result<()> {
            let mut undo = if rollback && count > 8192 {
                reserve(count)?
            } else {
                Vec::new()
            };
            if rollback && count > 8192 {
                for i in 0..count {
                    if i.is_multiple_of(8192) {
                        crate::dsp::check_cancel(cancel)?;
                    }
                    undo.push(data[address(i)]);
                }
            }
            for i in 0..count {
                if i.is_multiple_of(8192)
                    && let Err(error) = crate::dsp::check_cancel(cancel)
                {
                    // Every saved value predates the write, including repeated
                    // subscripts; restoring the visited prefix is sufficient.
                    for j in 0..i.min(undo.len()) {
                        data[address(j)] = undo[j];
                    }
                    return Err(error);
                }
                data[address(i)] = source[if source.len() == 1 { 0 } else { i }];
            }
            Ok(())
        }
        macro_rules! update {
            ($target:expr, $source:expr) => {{
                if let Some(data) = Arc::get_mut($target) {
                    write(data, count, &address, $source, rollback, cancel)?;
                } else {
                    let mut data = reserve($target.len())?;
                    for chunk in $target.chunks(8192) {
                        crate::dsp::check_cancel(cancel)?;
                        data.extend_from_slice(chunk);
                    }
                    write(&mut data, count, &address, $source, false, cancel)?;
                    *$target = Arc::new(data);
                }
            }};
        }
        match (&mut self.data, &source.data) {
            (TypedData::Int8(a), TypedData::Int8(b)) => update!(a, b),
            (TypedData::Uint8(a), TypedData::Uint8(b)) => update!(a, b),
            (TypedData::Int16(a), TypedData::Int16(b)) => update!(a, b),
            (TypedData::Uint16(a), TypedData::Uint16(b)) => update!(a, b),
            (TypedData::Int32(a), TypedData::Int32(b)) => update!(a, b),
            (TypedData::Uint32(a), TypedData::Uint32(b)) => update!(a, b),
            (TypedData::Int64(a), TypedData::Int64(b)) => update!(a, b),
            (TypedData::Uint64(a), TypedData::Uint64(b)) => update!(a, b),
            (TypedData::Single(a), TypedData::Single(b)) => update!(a, b),
            _ => return Err("Indexed assignment source class mismatch".into()),
        }
        Ok(())
    }

    /// Growth/deletion at native element width, retaining untouched bit patterns.
    pub(crate) fn remap(
        &self,
        rows: usize,
        cols: usize,
        source: impl Fn(usize) -> Option<usize>,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<Self> {
        let len = Matrix::checked_len(rows, cols)?;
        macro_rules! copy {
            ($data:expr, $variant:ident, $zero:expr) => {{
                let mut values = reserve(len)?;
                for i in 0..len {
                    if i.is_multiple_of(8192) {
                        crate::dsp::check_cancel(cancel)?;
                    }
                    values.push(source(i).map_or($zero, |j| $data[j]));
                }
                TypedData::$variant(values.into())
            }};
        }
        crate::dsp::check_cancel(cancel)?;
        let data = match &self.data {
            TypedData::Int8(d) => copy!(d, Int8, 0),
            TypedData::Uint8(d) => copy!(d, Uint8, 0),
            TypedData::Int16(d) => copy!(d, Int16, 0),
            TypedData::Uint16(d) => copy!(d, Uint16, 0),
            TypedData::Int32(d) => copy!(d, Int32, 0),
            TypedData::Uint32(d) => copy!(d, Uint32, 0),
            TypedData::Int64(d) => copy!(d, Int64, 0),
            TypedData::Uint64(d) => copy!(d, Uint64, 0),
            TypedData::Single(d) => copy!(d, Single, 0.),
        };
        Self::checked(rows, cols, data)
    }

    /// Immutable scalar edit; untouched elements keep their stored bits.
    pub fn with_value(
        &self,
        row: usize,
        col: usize,
        value: f64,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<Self> {
        self.with_scalar(row, col, crate::Scalar::Real(value), cancel)
    }
    pub fn with_scalar(
        &self,
        row: usize,
        col: usize,
        value: crate::Scalar,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<Self> {
        crate::dsp::check_cancel(cancel)?;
        if row >= self.rows || col >= self.cols {
            return Err("Index exceeds matrix dimensions".into());
        }
        let index = col * self.rows + row;
        macro_rules! update {
            ($d:expr, $variant:ident, $value:expr) => {{
                let mut values = reserve(self.len())?;
                for chunk in $d.chunks(8192) {
                    crate::dsp::check_cancel(cancel)?;
                    values.extend_from_slice(chunk);
                }
                values[index] = $value;
                TypedData::$variant(values.into())
            }};
        }
        let v = value.into_integer(self.class())?;
        let data = match &self.data {
            TypedData::Int8(d) => update!(d, Int8, v as i8),
            TypedData::Uint8(d) => update!(d, Uint8, v as u8),
            TypedData::Int16(d) => update!(d, Int16, v as i16),
            TypedData::Uint16(d) => update!(d, Uint16, v as u16),
            TypedData::Int32(d) => update!(d, Int32, v as i32),
            TypedData::Uint32(d) => update!(d, Uint32, v as u32),
            TypedData::Int64(d) => update!(d, Int64, v as i64),
            TypedData::Uint64(d) => update!(d, Uint64, v as u64),
            TypedData::Single(d) => update!(d, Single, value.into_real()? as f32),
        };
        Self::checked(self.rows, self.cols, data)
    }
    /// Native integer accumulation: each step saturates without a float conversion.
    pub fn cumulative_integer(
        &self,
        dim: usize,
        product: bool,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<Self> {
        use rayon::prelude::*;
        crate::dsp::check_cancel(cancel)?;
        if dim == 0 || dim > crate::matrix::MAX_ELEMENTS {
            return Err("Dimension must be a bounded positive integer".into());
        }
        if dim > 2 || self.is_empty() {
            return Ok(self.clone());
        }
        let class = self.class();
        let combine =
            |a: i128, b: i128| class.saturate(if product { a.saturating_mul(b) } else { a + b });
        macro_rules! accumulate {
            ($d:expr, $variant:ident, $t:ty) => {{
                let mut output = reserve(self.len())?;
                for chunk in $d.chunks(8192) {
                    crate::dsp::check_cancel(cancel)?;
                    output.extend_from_slice(chunk);
                }
                if dim == 1 {
                    let lane = |values: &mut [$t]| -> Result<()> {
                        for i in 1..values.len() {
                            if i.is_multiple_of(8192) {
                                crate::dsp::check_cancel(cancel)?;
                            }
                            values[i] =
                                combine(i128::from(values[i - 1]), i128::from(values[i])) as $t;
                        }
                        crate::dsp::check_cancel(cancel)
                    };
                    if self.len() >= 512 * 1024 && self.cols > 1 && rayon::current_num_threads() > 1
                    {
                        output.par_chunks_mut(self.rows).try_for_each(lane)?;
                    } else {
                        for values in output.chunks_mut(self.rows) {
                            lane(values)?;
                        }
                    }
                } else {
                    for i in self.rows..self.len() {
                        if i.is_multiple_of(8192) {
                            crate::dsp::check_cancel(cancel)?;
                        }
                        output[i] =
                            combine(i128::from(output[i - self.rows]), i128::from(output[i])) as $t;
                    }
                }
                TypedData::$variant(output.into())
            }};
        }
        let data = match &self.data {
            TypedData::Int8(d) => accumulate!(d, Int8, i8),
            TypedData::Uint8(d) => accumulate!(d, Uint8, u8),
            TypedData::Int16(d) => accumulate!(d, Int16, i16),
            TypedData::Uint16(d) => accumulate!(d, Uint16, u16),
            TypedData::Int32(d) => accumulate!(d, Int32, i32),
            TypedData::Uint32(d) => accumulate!(d, Uint32, u32),
            TypedData::Int64(d) => accumulate!(d, Int64, i64),
            TypedData::Uint64(d) => accumulate!(d, Uint64, u64),
            TypedData::Single(_) => return Err("Expected integer storage".into()),
        };
        Self::checked(self.rows, self.cols, data)
    }
    pub fn transpose(&self) -> Result<Self> {
        let positions: Vec<usize> = (0..self.len())
            .map(|i| (i % self.cols) * self.rows + i / self.cols)
            .collect();
        self.select(self.cols, self.rows, &positions)
    }
    /// The same values in another class (integers saturate, single rounds).
    pub fn cast(&self, class: NumericClass) -> Result<Self> {
        if class == self.class() {
            return Ok(self.clone());
        }
        if self.is_integer() && class.is_integer() {
            Self::from_ints(
                class,
                self.rows,
                self.cols,
                (0..self.len()).map(|i| self.int(i)),
            )
        } else if self.is_integer() {
            Self::from_ints(
                class,
                self.rows,
                self.cols,
                (0..self.len()).map(|i| self.int(i)),
            )
        } else {
            Self::from_reals(
                class,
                self.rows,
                self.cols,
                (0..self.len()).map(|i| self.real(i)),
            )
        }
    }
    /// Raw little-endian bytes, for `typecast` and MAT files.
    pub fn to_le_bytes(&self) -> Vec<u8> {
        each_int!(&self.data, d => d.iter().flat_map(|x| x.to_le_bytes()).collect(), s => s.iter().flat_map(|x| x.to_le_bytes()).collect())
    }
    pub fn from_le_bytes(
        class: NumericClass,
        rows: usize,
        cols: usize,
        bytes: &[u8],
    ) -> Result<Self> {
        let n = Matrix::checked_len(rows, cols)?;
        if bytes.len() != n * class.size() {
            return Err("Typed array byte count does not match its shape".into());
        }
        macro_rules! decode {
            ($variant:ident, $t:ty) => {{
                const W: usize = std::mem::size_of::<$t>();
                let v: Vec<$t> = bytes
                    .chunks_exact(W)
                    .map(|c| <$t>::from_le_bytes(c.try_into().expect("chunk")))
                    .collect();
                TypedData::$variant(v.into())
            }};
        }
        let data = match class {
            NumericClass::Int8 => decode!(Int8, i8),
            NumericClass::Uint8 => decode!(Uint8, u8),
            NumericClass::Int16 => decode!(Int16, i16),
            NumericClass::Uint16 => decode!(Uint16, u16),
            NumericClass::Int32 => decode!(Int32, i32),
            NumericClass::Uint32 => decode!(Uint32, u32),
            NumericClass::Int64 => decode!(Int64, i64),
            NumericClass::Uint64 => decode!(Uint64, u64),
            NumericClass::Single => decode!(Single, f32),
        };
        Self::checked(rows, cols, data)
    }
    /// Short value text for the workspace panel.
    pub fn preview(&self, limit: usize) -> String {
        if self.len() == 1 {
            return self.element_text(0);
        }
        let mut text = format!("{}x{} {}", self.rows, self.cols, self.class().name());
        if self.len() <= 10 {
            let mut values = String::from("[");
            for i in 0..self.len() {
                if i > 0 {
                    values.push(' ');
                }
                values.push_str(&self.element_text(i));
            }
            values.push(']');
            if values.len() <= limit {
                text = values;
            }
        }
        text
    }
    /// One element as text: integers exactly, single with up to 8 significant digits.
    pub fn element_text(&self, i: usize) -> String {
        if self.is_integer() {
            self.int(i).to_string()
        } else {
            let v = self.real(i) as f32;
            format!("{v}")
        }
    }
}

impl std::fmt::Display for TypedArray {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.preview(80))
    }
}

/// Integer quotient rounded half away from zero; `x/0` saturates, `0/0` is 0.
pub fn divide_round(a: i128, b: i128, class: NumericClass) -> i128 {
    if b == 0 {
        return match a.signum() {
            1 => class.max(),
            -1 => class.min(),
            _ => 0,
        };
    }
    let q = a / b;
    let r = a % b;
    let q = if 2 * r.unsigned_abs() >= b.unsigned_abs() {
        q + a.signum() * b.signum()
    } else {
        q
    };
    class.saturate(q)
}

/// `n + f` rounded half away from zero, for an integer `n` and `|f| < 1`.
fn round_sum(n: i128, f: f64) -> i128 {
    let non_negative = n > 0 || (n == 0 && f >= 0.);
    if non_negative {
        n + if f >= 0.5 {
            1
        } else if f < -0.5 {
            -1
        } else {
            0
        }
    } else {
        n - if f <= -0.5 {
            1
        } else if f > 0.5 {
            -1
        } else {
            0
        }
    }
}

/// Mantissa and exponent of a finite double: `v = m * 2^e` exactly.
fn decompose(v: f64) -> (i128, i32) {
    if v == 0. {
        return (0, 0);
    }
    let bits = v.to_bits();
    let sign: i128 = if bits >> 63 == 1 { -1 } else { 1 };
    let exp = ((bits >> 52) & 0x7ff) as i32;
    let frac = (bits & ((1u64 << 52) - 1)) as i128;
    if exp == 0 {
        (sign * frac, -1074)
    } else {
        (sign * (frac | (1 << 52)), exp - 1075)
    }
}

/// `x * 2^e` rounded half away from zero, saturating at the `i128` range.
fn scale_round(x: i128, e: i32) -> i128 {
    if x == 0 {
        return 0;
    }
    if e >= 0 {
        if e >= 127 {
            return if x > 0 { i128::MAX } else { i128::MIN };
        }
        return x
            .checked_mul(1i128 << e)
            .unwrap_or(if x > 0 { i128::MAX } else { i128::MIN });
    }
    let shift = (-e) as u32;
    if shift >= 127 {
        return 0;
    }
    let magnitude = x.unsigned_abs();
    let q = magnitude >> shift;
    let half = 1u128 << (shift - 1);
    let rem = magnitude & ((1u128 << shift) - 1);
    let q = (q + u128::from(rem >= half)) as i128;
    if x < 0 { -q } else { q }
}

/// Exact `x op d` for a 64-bit integer `x` and a double `d`, rounded half away
/// from zero (MATLAB keeps `int64` exact: `int64(2^53+1) + 1` is `2^53+2`).
/// `swap` computes `d op x`. `None` when `op` has no exact form here.
pub fn exact_with_double(x: i128, d: f64, op: char, swap: bool) -> Option<i128> {
    if !d.is_finite() {
        return None;
    }
    match op {
        '+' | '-' => {
            let n = d.trunc();
            if n.abs() >= 1e38 {
                return None;
            }
            let (ni, f) = (n as i128, d - n);
            Some(match (op, swap) {
                ('+', _) => round_sum(x + ni, f),
                ('-', false) => round_sum(x - ni, -f),
                _ => round_sum(ni - x, f),
            })
        }
        '*' => {
            let (m, e) = decompose(d);
            let product = x.checked_mul(m)?;
            Some(scale_round(product, e))
        }
        '/' => {
            let (m, e) = decompose(d);
            if !swap {
                // x / (m 2^e) = (x 2^-e) / m
                if m == 0 {
                    return None;
                }
                if -e > 60 || e > 60 {
                    return None;
                }
                let (num, den) = if e <= 0 {
                    (x.checked_mul(1i128 << (-e))?, m)
                } else {
                    (x, m.checked_mul(1i128 << e)?)
                };
                Some(round_div(num, den))
            } else {
                // (m 2^e) / x
                if x == 0 || e.abs() > 60 {
                    return None;
                }
                let (num, den) = if e >= 0 {
                    (m.checked_mul(1i128 << e)?, x)
                } else {
                    (m, x.checked_mul(1i128 << (-e))?)
                };
                Some(round_div(num, den))
            }
        }
        _ => None,
    }
}

fn round_div(num: i128, den: i128) -> i128 {
    let q = num / den;
    let r = num % den;
    if 2 * r.unsigned_abs() >= den.unsigned_abs() {
        q + num.signum() * den.signum()
    } else {
        q
    }
}

#[cfg(test)]
mod indexed_write_tests {
    use super::*;
    use std::{
        cell::Cell,
        sync::atomic::{AtomicBool, Ordering},
    };

    #[test]
    fn typed_writes_rollback_mid_write_cancellation_for_unique_and_shared_storage() {
        for class in NumericClass::INTEGERS
            .into_iter()
            .chain([NumericClass::Single])
        {
            for shared in [false, true] {
                let mut target =
                    TypedArray::from_ints(class, 1, 32768, (0..32768).map(i128::from)).unwrap();
                let alias = shared.then(|| target.clone());
                let before = target.to_le_bytes();
                let source = TypedArray::scalar_int(class, 7);
                let cancel = AtomicBool::new(false);
                let visits = Cell::new(0);
                // Unique storage first gathers its undo values; shared storage
                // edits a private native copy. Cancel during the first write chunk.
                let stop_at = if shared { 10 } else { target.len() + 10 };
                let error = target
                    .write_indices(
                        target.len(),
                        |i| {
                            visits.set(visits.get() + 1);
                            if visits.get() == stop_at {
                                cancel.store(true, Ordering::Relaxed);
                            }
                            i
                        },
                        &source,
                        true,
                        &cancel,
                    )
                    .unwrap_err();
                assert!(error.contains("cancelled"));
                assert!(visits.get() > stop_at);
                assert_eq!(target.to_le_bytes(), before);
                if let Some(alias) = alias {
                    assert!(target.same_storage(&alias));
                }
                cancel.store(false, Ordering::Relaxed);
                target
                    .write_indices(1, |_| 0, &source, true, &cancel)
                    .unwrap();
                assert_eq!(target.int(0), 7);
            }
        }
    }

    #[test]
    fn weak_recovery_identity_pins_only_a_header_and_the_old_payload_is_freed_on_write() {
        // A `Weak` keeps exactly one `ArcInner<T>` allocated. Typed storage is
        // `Arc<Vec<_>>`, so that is a fixed-size `Vec` header whatever the
        // element count: this only compiles for a sized pointee.
        fn pinned_bytes<T>(_: &std::sync::Weak<T>) -> usize {
            2 * std::mem::size_of::<usize>() + std::mem::size_of::<T>()
        }
        let n = 1 << 20;
        let mut target = TypedArray::from_ints(NumericClass::Int64, 1, n, 0..n as i128).unwrap();
        let WeakTypedData::Int64(weak) = target.weak_storage() else {
            panic!("int64")
        };
        assert!(pinned_bytes(&weak) <= 64 && target.bytes() == 8 * n);
        let before = weak.as_ptr();
        let source = TypedArray::scalar_int(NumericClass::Int64, 9007199254740993);
        // The weak identity forces a detached copy, charged once.
        assert_eq!(target.write_allocation_bytes(1, true), target.bytes() + 64);
        target
            .write_indices(1, |_| 0, &source, true, &AtomicBool::new(false))
            .unwrap();
        // No strong owner remains, so the old `Vec` has been dropped and its
        // 8 MiB buffer returned; only the header stays allocated for the cache.
        assert_eq!(weak.strong_count(), 0);
        assert!(weak.upgrade().is_none());
        let TypedData::Int64(data) = target.data() else {
            panic!("int64")
        };
        assert!(!std::ptr::eq(before, Arc::as_ptr(data)));
        assert_eq!(target.int(0), 9007199254740993);
        assert_eq!(target.int(n - 1), n as i128 - 1);
        assert_eq!(target.write_allocation_bytes(1, true), 0);
        assert_eq!(target.write_allocation_bytes(n, false), 0);
        assert_eq!(target.write_allocation_bytes(n, true), 8 * n);
    }
}
