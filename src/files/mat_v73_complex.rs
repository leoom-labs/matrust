//! MATLAB compound schema and the exact in-memory num_complex layout.
use crate::Complex64;
use hdf5::types::{CompoundField, CompoundType, FloatSize, TypeDescriptor};

// num-complex 0.4.6 guarantees repr(C). Use actual offsets rather than assuming
// that a Rust tuple or a separately allocated component vector has this layout.
const _: () = assert!(std::mem::size_of::<Complex64>() == 16);
const _: () = assert!(std::mem::offset_of!(Complex64, re) == 0);
const _: () = assert!(std::mem::offset_of!(Complex64, im) == 8);

pub(super) fn descriptor() -> TypeDescriptor {
    TypeDescriptor::Compound(CompoundType {
        fields: vec![
            CompoundField::typed::<f64>("real", std::mem::offset_of!(Complex64, re), 0),
            CompoundField::typed::<f64>("imag", std::mem::offset_of!(Complex64, im), 1),
        ],
        size: std::mem::size_of::<Complex64>(),
    })
}

pub(super) fn supported(value: &TypeDescriptor) -> bool {
    let TypeDescriptor::Compound(c) = value else {
        return false;
    };
    // Admit reversed member order/offsets: HDF5 maps members by their names.
    // Reject padding, aliases, nested/vlen members and non-double components.
    c.size == 16
        && c.fields.len() == 2
        && ["real", "imag"].iter().all(|name| {
            c.fields
                .iter()
                .filter(|f| {
                    f.name == *name
                        && f.ty == TypeDescriptor::Float(FloatSize::U8)
                        && matches!(f.offset, 0 | 8)
                })
                .count()
                == 1
        })
        && c.fields[0].offset != c.fields[1].offset
}
