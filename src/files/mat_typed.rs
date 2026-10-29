//! MAT v5 codes of the integer and single classes.
use crate::NumericClass;

/// Array class (`mxINT8_CLASS` = 8 .. `mxUINT64_CLASS` = 15, `mxSINGLE_CLASS` = 7).
pub(super) fn mx_class(class: NumericClass) -> u32 {
    match class {
        NumericClass::Single => 7,
        NumericClass::Int8 => 8,
        NumericClass::Uint8 => 9,
        NumericClass::Int16 => 10,
        NumericClass::Uint16 => 11,
        NumericClass::Int32 => 12,
        NumericClass::Uint32 => 13,
        NumericClass::Int64 => 14,
        NumericClass::Uint64 => 15,
    }
}

/// Data element type (`miINT8` = 1 .. `miUINT64` = 13).
pub(super) fn mi_type(class: NumericClass) -> u32 {
    match class {
        NumericClass::Int8 => 1,
        NumericClass::Uint8 => 2,
        NumericClass::Int16 => 3,
        NumericClass::Uint16 => 4,
        NumericClass::Int32 => 5,
        NumericClass::Uint32 => 6,
        NumericClass::Single => 7,
        NumericClass::Int64 => 12,
        NumericClass::Uint64 => 13,
    }
}
