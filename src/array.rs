//! Typed immutable values. Observation never discards a complex component.
use std::borrow::Cow;

/// Canonical MATLAB array dimensions: at least two axes, bounded rank, and no
/// redundant trailing singleton axes.
pub(crate) fn normalize_dims(dims: &[usize]) -> Result<Vec<usize>> {
    if dims.is_empty() || dims.len() > 32 {
        return Err("Array rank must be between 1 and 32".into());
    }
    let mut result = dims.to_vec();
    if result.len() == 1 {
        result.push(1);
    }
    while result.len() > 2 && result.last() == Some(&1) {
        result.pop();
    }
    Ok(result)
}
pub(crate) mod coordinate_retention;
use crate::{ComplexMatrix, LogicalMatrix, Matrix, Result, Scalar, logical::ArrayRef};
use std::sync::atomic::AtomicBool;
mod powers_of_two;
pub(crate) use powers_of_two::scale_pow2_shape;
pub use powers_of_two::{nextpow2, pow2, scale_pow2};
mod near_zero;
pub use near_zero::{expm1, log1p};
mod polyval;
pub use polyval::evaluate_polynomial;
mod interp1;
pub use interp1::{InterpolationMethod, interpolate_vectors};
mod ismember;
mod set_operations;
pub use set_operations::{SetOperation, SetResult};
mod repeat;
mod sort;
mod sort_rows;
mod unique;
pub use ismember::MembershipResult;
pub use sort::{MissingPlacement, SortOrder};
pub use unique::{UniqueOrder, UniqueResult};

#[derive(Clone, Debug)]
pub enum Array {
    Numeric(Matrix),
    Logical(LogicalMatrix),
    Complex(ComplexMatrix),
    Character(crate::CharacterRow),
    Str(crate::StringArray),
    Cell(crate::CellArray),
    Graphics(crate::GraphicsArray),
    Struct(crate::StructArray),
    Handle(crate::FunctionHandle),
    /// `int8` .. `uint64` and `single`.
    Typed(crate::TypedArray),
    Categorical(crate::CategoricalArray),
    Datetime(crate::DatetimeArray),
    Duration(crate::DurationArray),
    /// `table`: named variables of a common height (src/table_array.rs).
    Table(crate::TableArray),
    /// Two-dimensional compressed sparse column double or logical array.
    Sparse(crate::sparse::SparseMatrix),
}
impl From<crate::TypedArray> for Array {
    fn from(a: crate::TypedArray) -> Self {
        Self::Typed(a)
    }
}
impl From<crate::CategoricalArray> for Array {
    fn from(a: crate::CategoricalArray) -> Self {
        Self::Categorical(a)
    }
}
impl From<crate::DatetimeArray> for Array {
    fn from(a: crate::DatetimeArray) -> Self {
        Self::Datetime(a)
    }
}
impl From<crate::TableArray> for Array {
    fn from(a: crate::TableArray) -> Self {
        Self::Table(a)
    }
}
impl From<crate::DurationArray> for Array {
    fn from(a: crate::DurationArray) -> Self {
        Self::Duration(a)
    }
}
impl From<Matrix> for Array {
    fn from(v: Matrix) -> Self {
        Self::Numeric(v)
    }
}
impl From<LogicalMatrix> for Array {
    fn from(v: LogicalMatrix) -> Self {
        Self::Logical(v)
    }
}
impl From<ComplexMatrix> for Array {
    fn from(v: ComplexMatrix) -> Self {
        Self::Complex(v)
    }
}
impl From<crate::CharacterRow> for Array {
    fn from(a: crate::CharacterRow) -> Self {
        Self::Character(a)
    }
}
impl From<crate::CellArray> for Array {
    fn from(a: crate::CellArray) -> Self {
        Self::Cell(a)
    }
}
impl From<crate::StructArray> for Array {
    fn from(a: crate::StructArray) -> Self {
        Self::Struct(a)
    }
}
impl From<crate::GraphicsArray> for Array {
    fn from(a: crate::GraphicsArray) -> Self {
        Self::Graphics(a)
    }
}
impl From<crate::FunctionHandle> for Array {
    fn from(a: crate::FunctionHandle) -> Self {
        Self::Handle(a)
    }
}
impl Array {
    /// True for a struct, or a cell holding one at any depth.
    pub fn contains_struct(&self) -> bool {
        match self {
            Self::Struct(_) => true,
            Self::Cell(c) => {
                (0..c.len()).any(|i| c.get_linear(i).is_ok_and(|v| v.contains_struct()))
            }
            _ => false,
        }
    }
    pub fn is_numeric_array(&self) -> bool {
        matches!(self, Self::Numeric(_) | Self::Logical(_) | Self::Complex(_))
    }
    pub fn require_numeric_array(&self) -> Result<()> {
        if self.is_numeric_array() {
            Ok(())
        } else if matches!(self, Self::Str(_)) {
            Err("This operation does not support string arrays yet; convert with char(...) or cellstr(...)".into())
        } else {
            Err(
                "This operation does not support character, cell, struct or graphics handle arrays"
                    .into(),
            )
        }
    }
    pub fn same_snapshot(&self, other: &Self) -> bool {
        self.dims() == other.dims()
            && match (self, other) {
                (Self::Numeric(a), Self::Numeric(b)) => {
                    std::sync::Arc::ptr_eq(&a.storage, &b.storage)
                }
                (Self::Logical(a), Self::Logical(b)) => std::sync::Arc::ptr_eq(&a.data, &b.data),
                (Self::Complex(a), Self::Complex(b)) => a.same_snapshot(b),
                (Self::Character(a), Self::Character(b)) => a.same_storage(b),
                (Self::Str(a), Self::Str(b)) => a.same_storage(b),
                (Self::Cell(a), Self::Cell(b)) => a.same_storage(b),

                (Self::Graphics(a), Self::Graphics(b)) => a.same_storage(b),
                (Self::Struct(a), Self::Struct(b)) => a.same_storage(b),
                (Self::Handle(a), Self::Handle(b)) => a.same_storage(b),
                (Self::Typed(a), Self::Typed(b)) => a.same_storage(b),
                (Self::Categorical(a), Self::Categorical(b)) => a.same_storage(b),
                (Self::Datetime(a), Self::Datetime(b)) => a.same_storage(b),
                (Self::Duration(a), Self::Duration(b)) => a.same_storage(b),
                (Self::Table(a), Self::Table(b)) => a.same_storage(b),
                (Self::Sparse(a), Self::Sparse(b)) => a.same_storage(b),
                _ => false,
            }
    }
    pub fn view(&self) -> Result<ArrayRef<'_>> {
        match self {
            Self::Numeric(v) => Ok(ArrayRef::Numeric(v)),
            Self::Logical(v) => Ok(ArrayRef::Logical(v)),
            Self::Str(_)
            | Self::Character(_)
            | Self::Cell(_)
            | Self::Graphics(_)
            | Self::Struct(_)
            | Self::Handle(_)
            | Self::Typed(_)
            | Self::Categorical(_)
            | Self::Datetime(_)
            | Self::Duration(_)
            | Self::Table(_)
            | Self::Sparse(_) => Err("This operation requires numeric or logical values".into()),
            Self::Complex(_) => Err(
                "This operation requires a real or logical array; use real, imag or abs explicitly"
                    .into(),
            ),
        }
    }
    pub fn rows(&self) -> usize {
        match self {
            Self::Numeric(a) => a.rows(),
            Self::Logical(a) => a.rows(),
            Self::Complex(a) => a.rows(),
            Self::Character(a) => a.rows(),
            Self::Str(a) => a.rows(),
            Self::Cell(a) => a.rows(),
            Self::Struct(a) => a.rows(),
            Self::Graphics(a) => a.rows(),
            Self::Typed(a) => a.rows(),
            Self::Categorical(a) => a.rows(),
            Self::Datetime(a) => a.rows(),
            Self::Duration(a) => a.rows(),
            Self::Table(a) => a.rows(),
            Self::Handle(_) => 1,
            Self::Sparse(a) => a.rows(),
        }
    }
    pub fn dims(&self) -> Cow<'_, [usize]> {
        match self {
            Self::Numeric(a) => a.dims(),
            Self::Logical(a) => a.dims(),
            Self::Complex(a) => a.dims(),
            Self::Character(a) => a.dims(),
            Self::Str(a) => Cow::Owned(vec![a.rows(), a.cols()]),
            Self::Typed(a) => a.dims(),
            Self::Cell(a) => Cow::Owned(vec![a.rows(), a.cols()]),
            Self::Graphics(a) => Cow::Owned(vec![a.rows(), a.cols()]),
            Self::Struct(a) => Cow::Owned(vec![a.rows(), a.cols()]),
            Self::Categorical(a) => a.nd_dims(),
            Self::Datetime(a) => Cow::Owned(vec![a.rows(), a.cols()]),
            Self::Duration(a) => Cow::Owned(vec![a.rows(), a.cols()]),
            Self::Table(a) => Cow::Owned(vec![a.rows(), a.cols()]),
            Self::Handle(_) => Cow::Owned(vec![1, 1]),
            Self::Sparse(a) => Cow::Owned(vec![a.rows(), a.cols()]),
        }
    }
    pub fn ndims(&self) -> usize {
        self.dims().len()
    }
    pub fn is_nd(&self) -> bool {
        matches!(self, Self::Numeric(a) if a.is_nd())
            || matches!(self, Self::Logical(a) if a.is_nd())
            || matches!(self, Self::Complex(a) if a.is_nd())
            || matches!(self, Self::Character(a) if a.is_nd())
            || matches!(self, Self::Typed(a) if a.is_nd())
            || matches!(self, Self::Categorical(a) if a.is_nd())
    }
    pub fn with_dims(&self, dims: &[usize]) -> Result<Self> {
        match self {
            Self::Numeric(a) => Ok(Self::Numeric(a.with_dims(dims)?)),
            Self::Logical(a) => Ok(Self::Logical(a.with_dims(dims)?)),
            Self::Complex(a) => Ok(Self::Complex(a.with_dims(dims)?)),
            Self::Character(a) => Ok(Self::Character(a.with_dims(dims)?)),
            Self::Typed(a) => Ok(Self::Typed(a.with_dims(dims)?)),
            Self::Categorical(a) => Ok(Self::Categorical(a.with_dims(dims)?)),
            _ => Err("N-D storage is not available for cell, struct or graphics arrays".into()),
        }
    }
    pub fn cols(&self) -> usize {
        match self {
            Self::Numeric(a) => a.cols(),
            Self::Logical(a) => a.cols(),
            Self::Complex(a) => a.cols(),
            Self::Character(a) => a.cols(),
            Self::Str(a) => a.cols(),
            Self::Cell(a) => a.cols(),
            Self::Struct(a) => a.cols(),
            Self::Graphics(a) => a.cols(),
            Self::Typed(a) => a.cols(),
            Self::Categorical(a) => a.cols(),
            Self::Datetime(a) => a.cols(),
            Self::Duration(a) => a.cols(),
            Self::Table(a) => a.cols(),
            Self::Handle(_) => 1,
            Self::Sparse(a) => a.cols(),
        }
    }
    pub fn len(&self) -> usize {
        match self {
            Self::Numeric(a) => a.len(),
            Self::Logical(a) => a.len(),
            Self::Complex(a) => a.len(),
            Self::Character(a) => a.len(),
            Self::Str(a) => a.len(),
            Self::Cell(a) => a.len(),
            Self::Struct(a) => a.len(),
            Self::Graphics(a) => a.len(),
            Self::Typed(a) => a.len(),
            Self::Categorical(a) => a.len(),
            Self::Datetime(a) => a.len(),
            Self::Duration(a) => a.len(),
            Self::Table(a) => a.len(),
            Self::Handle(_) => 1,
            // Saturates for sparse arrays larger than the address space.
            Self::Sparse(a) => usize::try_from(a.numel()).unwrap_or(usize::MAX),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Retained workspace payload; serialized logical byte counts stay separate.
    pub fn retained_bytes(&self) -> usize {
        match self {
            Self::Numeric(a) => a.retained_bytes(),
            Self::Cell(a) => a.retained_bytes(),
            Self::Struct(a) => a.retained_bytes(),
            _ => self.bytes(),
        }
    }
    pub fn bytes(&self) -> usize {
        match self {
            Self::Character(a) => return a.bytes(),
            Self::Str(a) => return a.bytes(),
            Self::Cell(a) => return a.bytes(),
            Self::Struct(a) => return a.bytes(),
            Self::Graphics(a) => return a.bytes(),
            Self::Handle(a) => return a.bytes(),
            Self::Typed(a) => return a.bytes(),
            Self::Categorical(a) => return a.bytes(),
            Self::Datetime(a) => return a.bytes(),
            Self::Duration(a) => return a.bytes(),
            Self::Table(a) => return a.bytes(),
            Self::Sparse(a) => return a.bytes(),
            _ => {}
        }
        self.len()
            * match self {
                Self::Numeric(_) => 8,
                Self::Logical(_) => 1,
                Self::Complex(_) => 16,
                Self::Str(_)
                | Self::Character(_)
                | Self::Cell(_)
                | Self::Graphics(_)
                | Self::Struct(_)
                | Self::Typed(_)
                | Self::Handle(_)
                | Self::Categorical(_)
                | Self::Datetime(_)
                | Self::Duration(_)
                | Self::Table(_)
                | Self::Sparse(_) => {
                    unreachable!("containers handled above")
                }
            }
    }
    pub fn is_logical(&self) -> bool {
        matches!(self, Self::Logical(_)) || matches!(self, Self::Sparse(a) if a.is_logical())
    }
    pub fn is_complex(&self) -> bool {
        matches!(self, Self::Complex(_))
    }
    pub fn class(&self) -> &'static str {
        if matches!(self, Self::Categorical(_)) {
            return "categorical";
        }
        if matches!(self, Self::Datetime(_)) {
            return "datetime";
        }
        if let Self::Duration(d) = self {
            return if d.is_calendar() {
                "calendarDuration"
            } else {
                "duration"
            };
        }
        if let Self::Table(t) = self {
            return if t.is_timetable() {
                "timetable"
            } else {
                "table"
            };
        }
        if matches!(self, Self::Str(_)) {
            return "string";
        }
        if matches!(self, Self::Character(_)) {
            return "char";
        }
        if matches!(self, Self::Cell(_)) {
            return "cell";
        }
        if matches!(self, Self::Struct(_)) {
            return "struct";
        }
        if matches!(self, Self::Graphics(_)) {
            return "graphics handle";
        }
        if matches!(self, Self::Handle(_)) {
            return "function_handle";
        }
        if let Self::Typed(a) = self {
            return a.class().name();
        }
        if matches!(self, Self::Complex(a) if a.is_single()) {
            return "single";
        }
        if let Self::Sparse(a) = self {
            return if a.is_logical() { "logical" } else { "double" };
        }
        if self.is_logical() {
            "logical"
        } else {
            "double"
        }
    }
    pub fn scalar(&self, r: usize, c: usize) -> Result<Scalar> {
        if r >= self.rows() || c >= self.cols() {
            return Err("Array cell index is out of bounds".into());
        }
        Ok(match self {
            Self::Numeric(a) => Scalar::Real(a.get(r, c)),
            Self::Logical(a) => Scalar::Logical(a.get(r, c)),
            Self::Complex(a) => Scalar::Complex(a.get(r, c)?),
            Self::Typed(a) if a.is_integer() => Scalar::Integer(a.int(c * a.rows() + r)),
            Self::Typed(a) => Scalar::Real(a.real(c * a.rows() + r)),
            Self::Sparse(a) if a.is_logical() => Scalar::Logical(a.get(r, c) != 0.),
            Self::Sparse(a) => Scalar::Real(a.get(r, c)),
            Self::Str(_)
            | Self::Character(_)
            | Self::Cell(_)
            | Self::Graphics(_)
            | Self::Struct(_)
            | Self::Categorical(_)
            | Self::Datetime(_)
            | Self::Duration(_)
            | Self::Table(_)
            | Self::Handle(_) => {
                return Err("Container contents are not numeric scalar cells".into());
            }
        })
    }
    /// Replaces the old f64 getter: callers must explicitly handle the type.
    pub fn get(&self, r: usize, c: usize) -> Result<Scalar> {
        self.scalar(r, c)
    }
    pub fn as_numeric(&self) -> Option<&Matrix> {
        if let Self::Numeric(a) = self {
            Some(a)
        } else {
            None
        }
    }
    pub fn as_complex(&self) -> Option<&ComplexMatrix> {
        if let Self::Complex(a) = self {
            Some(a)
        } else {
            None
        }
    }
    pub fn to_numeric(&self, cancel: &AtomicBool) -> Result<Matrix> {
        match self {
            Self::Numeric(a) => Ok(a.clone()),
            Self::Logical(a) => a.to_numeric(cancel),
            Self::Typed(a) => a.to_matrix(),
            Self::Sparse(a) => a.to_dense(cancel),
            Self::Str(_) => Err("Cannot implicitly convert string arrays to numbers; use double(...) or str2double(...)".into()),
            Self::Categorical(_) => Err("Cannot implicitly convert categorical arrays to numbers; use double(...)".into()),
            Self::Datetime(_) => Err("Cannot implicitly convert datetime arrays to numbers; use datenum(...) or posixtime(...)".into()),
            Self::Duration(_) => Err("Cannot implicitly convert duration arrays to numbers; use seconds(...), hours(...) or days(...)".into()),
            Self::Table(_) => Err("Cannot implicitly convert a table to numbers; use table2array(T) or T{:,:}".into()),
            Self::Character(_) | Self::Cell(_) | Self::Graphics(_) | Self::Struct(_) | Self::Handle(_) => Err(
                "Cannot implicitly convert character, cell, struct, function handle or graphics handle arrays to numbers"
                    .into(),
            ),
            Self::Complex(_) => {
                Err("This operation requires a real array; use real, imag or abs explicitly".into())
            }
        }
    }
    pub fn preview(&self, limit: usize) -> String {
        match self {
            Self::Numeric(a) => a.preview(limit),
            Self::Logical(a) => a.to_string(),
            Self::Complex(a) => a.preview(limit),
            Self::Character(a) => a.preview(limit),
            Self::Str(a) => a.preview(limit),
            Self::Cell(a) => a.preview(limit),
            Self::Struct(a) => a.preview(limit),
            Self::Graphics(a) => a.preview(limit),
            Self::Handle(a) => a.display(),
            Self::Typed(a) => a.preview(limit),
            Self::Categorical(a) => a.to_string(),
            Self::Datetime(a) => a.to_string(),
            Self::Duration(a) => a.to_string(),
            Self::Table(a) => a.preview(),
            Self::Sparse(a) => format!(
                "{}x{} sparse {}",
                a.rows(),
                a.cols(),
                if a.is_logical() { "logical" } else { "double" }
            ),
        }
    }
    pub fn with_scalar(
        &self,
        r: usize,
        c: usize,
        value: Scalar,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        crate::dsp::check_cancel(cancel)?;
        let output = match self {
            Self::Complex(a) => a.with_value(r, c, value.into_complex(), cancel)?.into(),
            Self::Numeric(a) => a.with_value(r, c, value.into_real()?)?.into(),
            Self::Logical(a) => a.with_value(r, c, value.into_real()?, cancel)?.into(),
            Self::Typed(a) => {
                a.with_scalar(r, c, value, cancel)?.into()
            }
            Self::Sparse(a) => {
                let v = value.into_real()?;
                let mut writes = vec![(r, c, v)];
                Self::Sparse(a.with_writes(a.rows(), a.cols(), &mut writes, cancel)?)
            }
            Self::Str(_) | Self::Character(_) | Self::Cell(_) | Self::Graphics(_) | Self::Struct(_) | Self::Handle(_) | Self::Categorical(_) | Self::Datetime(_) | Self::Duration(_) | Self::Table(_) => return Err("Use script content indexing to edit containers; numeric cell editing is unsupported".into()),
        };
        crate::dsp::check_cancel(cancel)?;
        Ok(output)
    }
    pub fn with_value(&self, r: usize, c: usize, value: f64, cancel: &AtomicBool) -> Result<Self> {
        self.with_scalar(r, c, Scalar::Real(value), cancel)
    }
}
impl std::fmt::Display for Array {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Numeric(a) => a.fmt(f),
            Self::Logical(a) => a.fmt(f),
            Self::Complex(a) => a.fmt(f),
            Self::Character(a) => a.fmt(f),
            Self::Str(a) => write!(f, "{}", a.preview(16)),
            Self::Cell(a) => a.fmt(f),
            Self::Struct(a) => a.fmt(f),
            Self::Graphics(a) => a.fmt(f),
            Self::Handle(a) => a.fmt(f),
            Self::Typed(a) => a.fmt(f),
            Self::Categorical(a) => a.fmt(f),
            Self::Datetime(a) => a.fmt(f),
            Self::Duration(a) => a.fmt(f),
            Self::Table(a) => a.fmt(f),
            Self::Sparse(a) => {
                for (r, c, v) in a.entries() {
                    writeln!(f, "({},{}) {v}", r + 1, c + 1)?;
                }
                Ok(())
            }
        }
    }
}
