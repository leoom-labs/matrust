//! Typed immutable values. Observation never discards a complex component.
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
    Cell(crate::CellArray),
    Graphics(crate::GraphicsArray),
    Struct(crate::StructArray),
    Handle(crate::FunctionHandle),
    /// `int8` .. `uint64` and `single`.
    Typed(crate::TypedArray),
}
impl From<crate::TypedArray> for Array {
    fn from(a: crate::TypedArray) -> Self {
        Self::Typed(a)
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
        } else {
            Err(
                "This operation does not support character, cell, struct or graphics handle arrays"
                    .into(),
            )
        }
    }
    pub fn same_snapshot(&self, other: &Self) -> bool {
        self.rows() == other.rows()
            && self.cols() == other.cols()
            && match (self, other) {
                (Self::Numeric(a), Self::Numeric(b)) => {
                    std::sync::Arc::ptr_eq(&a.storage, &b.storage)
                }
                (Self::Logical(a), Self::Logical(b)) => std::sync::Arc::ptr_eq(&a.data, &b.data),
                (Self::Complex(a), Self::Complex(b)) => a.same_snapshot(b),
                (Self::Character(a), Self::Character(b)) => a.same_storage(b),
                (Self::Cell(a), Self::Cell(b)) => a.same_storage(b),

                (Self::Graphics(a), Self::Graphics(b)) => a.same_storage(b),
                (Self::Struct(a), Self::Struct(b)) => a.same_storage(b),
                (Self::Handle(a), Self::Handle(b)) => a.same_storage(b),
                (Self::Typed(a), Self::Typed(b)) => a.same_storage(b),
                _ => false,
            }
    }
    pub fn view(&self) -> Result<ArrayRef<'_>> {
        match self {
            Self::Numeric(v) => Ok(ArrayRef::Numeric(v)),
            Self::Logical(v) => Ok(ArrayRef::Logical(v)),
            Self::Character(_)
            | Self::Cell(_)
            | Self::Graphics(_)
            | Self::Struct(_)
            | Self::Handle(_)
            | Self::Typed(_) => Err("This operation requires numeric or logical values".into()),
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
            Self::Cell(a) => a.rows(),
            Self::Struct(a) => a.rows(),
            Self::Graphics(a) => a.rows(),
            Self::Typed(a) => a.rows(),
            Self::Handle(_) => 1,
        }
    }
    pub fn cols(&self) -> usize {
        match self {
            Self::Numeric(a) => a.cols(),
            Self::Logical(a) => a.cols(),
            Self::Complex(a) => a.cols(),
            Self::Character(a) => a.cols(),
            Self::Cell(a) => a.cols(),
            Self::Struct(a) => a.cols(),
            Self::Graphics(a) => a.cols(),
            Self::Typed(a) => a.cols(),
            Self::Handle(_) => 1,
        }
    }
    pub fn len(&self) -> usize {
        match self {
            Self::Numeric(a) => a.len(),
            Self::Logical(a) => a.len(),
            Self::Complex(a) => a.len(),
            Self::Character(a) => a.len(),
            Self::Cell(a) => a.len(),
            Self::Struct(a) => a.len(),
            Self::Graphics(a) => a.len(),
            Self::Typed(a) => a.len(),
            Self::Handle(_) => 1,
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
            Self::Cell(a) => return a.bytes(),
            Self::Struct(a) => return a.bytes(),
            Self::Graphics(a) => return a.bytes(),
            Self::Handle(a) => return a.bytes(),
            Self::Typed(a) => return a.bytes(),
            _ => {}
        }
        self.len()
            * match self {
                Self::Numeric(_) => 8,
                Self::Logical(_) => 1,
                Self::Complex(_) => 16,
                Self::Character(_)
                | Self::Cell(_)
                | Self::Graphics(_)
                | Self::Struct(_)
                | Self::Typed(_)
                | Self::Handle(_) => {
                    unreachable!("containers handled above")
                }
            }
    }
    pub fn is_logical(&self) -> bool {
        matches!(self, Self::Logical(_))
    }
    pub fn is_complex(&self) -> bool {
        matches!(self, Self::Complex(_))
    }
    pub fn class(&self) -> &'static str {
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
            Self::Character(_)
            | Self::Cell(_)
            | Self::Graphics(_)
            | Self::Struct(_)
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
            Self::Cell(a) => a.preview(limit),
            Self::Struct(a) => a.preview(limit),
            Self::Graphics(a) => a.preview(limit),
            Self::Handle(a) => a.display(),
            Self::Typed(a) => a.preview(limit),
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
            Self::Character(_) | Self::Cell(_) | Self::Graphics(_) | Self::Struct(_) | Self::Handle(_) => return Err("Use script content indexing to edit containers; numeric cell editing is unsupported".into()),
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
            Self::Cell(a) => a.fmt(f),
            Self::Struct(a) => a.fmt(f),
            Self::Graphics(a) => a.fmt(f),
            Self::Handle(a) => a.fmt(f),
            Self::Typed(a) => a.fmt(f),
        }
    }
}
