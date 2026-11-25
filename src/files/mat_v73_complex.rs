//! MATLAB compound schema and the exact in-memory num_complex layout.
use crate::Complex64;
use hdf5::types::{CompoundField, CompoundType, FloatSize, TypeDescriptor};

// num-complex 0.4.6 guarantees repr(C). Use actual offsets rather than assuming
// that a Rust tuple or a separately allocated component vector has this layout.
const _: () = assert!(std::mem::size_of::<Complex64>() == 16);
const _: () = assert!(std::mem::offset_of!(Complex64, re) == 0);
const _: () = assert!(std::mem::offset_of!(Complex64, im) == 8);

/// Component width shared by a MATLAB class and its real/imag wire compound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Precision {
    Double,
    Single,
}

/// Layout of the `Complex64` backing, which is also the complex-double wire
/// record. Both wire precisions decode into it: HDF5 maps the members by name
/// and widens single components exactly.
pub(super) fn descriptor() -> TypeDescriptor {
    TypeDescriptor::Compound(CompoundType {
        fields: vec![
            CompoundField::typed::<f64>("real", std::mem::offset_of!(Complex64, re), 0),
            CompoundField::typed::<f64>("imag", std::mem::offset_of!(Complex64, im), 1),
        ],
        size: std::mem::size_of::<Complex64>(),
    })
}

/// Packed complex-single wire record written by MatRust: `real` f32 at 0 and
/// `imag` f32 at 4. HDF5 maps members by name to and from `descriptor()`.
pub(super) fn single_wire_descriptor() -> TypeDescriptor {
    TypeDescriptor::Compound(CompoundType {
        fields: vec![
            CompoundField::typed::<f32>("real", 0, 0),
            CompoundField::typed::<f32>("imag", 4, 1),
        ],
        size: 2 * std::mem::size_of::<f32>(),
    })
}

/// Exactly two packed `real`/`imag` members of one float width, in either order.
fn packed(c: &CompoundType, component: FloatSize) -> bool {
    let width = component as usize;
    // Admit reversed member order/offsets: HDF5 maps members by their names.
    // Reject padding, aliases, nested/vlen members and any other component type.
    c.size == 2 * width
        && c.fields.len() == 2
        && ["real", "imag"].iter().all(|name| {
            c.fields
                .iter()
                .filter(|f| {
                    f.name == *name
                        && f.ty == TypeDescriptor::Float(component)
                        && (f.offset == 0 || f.offset == width)
                })
                .count()
                == 1
        })
        && c.fields[0].offset != c.fields[1].offset
}

/// Wire precision of a complex dataset, only when the MATLAB class and both
/// named member widths agree: `double` with 8-byte, `single` with 4-byte floats.
pub(super) fn precision(class: &str, value: &TypeDescriptor) -> Option<Precision> {
    let TypeDescriptor::Compound(c) = value else {
        return None;
    };
    match class {
        "double" if packed(c, FloatSize::U8) => Some(Precision::Double),
        "single" if packed(c, FloatSize::U4) => Some(Precision::Single),
        _ => None,
    }
}
