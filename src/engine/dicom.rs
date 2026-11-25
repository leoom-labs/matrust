//! DICOM metadata projection. The shared, UI-independent parser lives below;
//! this adapter alone knows Engine values, workspace budgets and file paths.
pub mod collection;
pub(super) mod custom_dict;
mod deflated;
mod dictionary;
mod j2k;
mod jpeg;
pub(crate) mod parser;
pub(crate) mod pixels;
pub(crate) mod rle;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod writer;

use super::{Engine, Value};
use crate::{
    Array, Matrix, Result, StructArray,
    typed_array::{NumericClass, TypedArray},
};
use custom_dict::Dictionary;
use parser::{Backing, Dataset, Element, ElementValue, TransferSyntax, read_value};
use std::{
    fs::File,
    io::{Read, Seek},
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};

pub(super) fn info(
    engine: &Engine,
    path: &Path,
    file_limit: u64,
    dictionary: &Dictionary,
) -> Result<Value> {
    crate::dsp::check_cancel(&engine.cancel)?;
    if !std::fs::metadata(path)
        .map_err(|e| e.to_string())?
        .is_file()
    {
        return Err("dicominfo: input must be a regular file".into());
    }
    let file = File::open(path).map_err(|e| e.to_string())?;
    let stat = file.metadata().map_err(|e| e.to_string())?;
    if !stat.is_file() {
        return Err("dicominfo: input must be a regular file".into());
    }
    if stat.len() > file_limit {
        return Err(format!("File exceeds the {file_limit}-byte limit"));
    }
    let fixed_cost = path
        .as_os_str()
        .len()
        .checked_mul(8)
        .and_then(|n| n.checked_add(4096))
        .ok_or("dicominfo: filename memory accounting overflow")?;
    let budget = super::limits::memory_bytes(&engine.limits)
        .saturating_sub(engine.workspace_bytes())
        .saturating_sub(dictionary.memory_bytes())
        .checked_sub(fixed_cost)
        .ok_or("dicominfo: metadata exceeds the memory/output budget")?;
    // Parsing charges descriptors and a conservative 16x encoded-value budget
    // before any value-sized allocation. Engine assignment also checks retention.
    let mut prepared = deflated::prepare(
        file,
        stat.len(),
        budget,
        inflate_work_bytes(engine),
        &engine.cancel,
        dictionary,
    )?;
    let parsed = &prepared.parsed;
    let cancel = &engine.cancel;
    let mut remaining_budget = parsed.remaining_budget;
    let mut fields = vec![
        (
            "Filename".to_owned(),
            Value::Text(super::env_files::envx_path_text(path)),
        ),
        ("FileSize".to_owned(), num(parsed.file_size as f64)),
        ("Format".to_owned(), Value::Text("DICOM".into())),
    ];
    fields.extend(project(
        &mut prepared.original,
        &parsed.meta,
        TransferSyntax::ExplicitLittle,
        Backing::Original,
        false,
        cancel,
        &mut remaining_budget,
        dictionary,
    )?);
    fields.extend(project(
        &mut prepared.dataset.reader(&mut prepared.original),
        &parsed.dataset,
        parsed.syntax,
        parsed.dataset_backing,
        false,
        cancel,
        &mut remaining_budget,
        dictionary,
    )?);
    for (source, target) in [
        ("Columns", "Width"),
        ("Rows", "Height"),
        ("BitsStored", "BitDepth"),
    ] {
        if let Some(value) = scalar(&fields, source) {
            fields.push((target.into(), num(value)));
        }
    }
    // BitsStored is the sample bit depth; BitsAllocated is only a fallback.
    if scalar(&fields, "BitDepth").is_none()
        && let Some(value) = scalar(&fields, "BitsAllocated")
    {
        fields.push(("BitDepth".into(), num(value)));
    }
    if let Some((_, Value::Text(photo))) = fields
        .iter()
        .find(|(name, _)| name == "PhotometricInterpretation")
    {
        let color = match photo.as_str() {
            "MONOCHROME1" | "MONOCHROME2" => Some("grayscale"),
            "PALETTE COLOR" => Some("indexed"),
            "RGB" | "YBR_FULL" | "YBR_FULL_422" | "YBR_PARTIAL_422" | "YBR_PARTIAL_420"
            | "YBR_ICT" | "YBR_RCT" => Some("truecolor"),
            _ => None,
        };
        if let Some(color) = color {
            fields.push(("ColorType".into(), Value::Text(color.into())));
        }
    }
    // Exercise the descriptor here without touching its bytes; decoders reuse it.
    if let Some(parser::PixelLocation::Native { value, vr }) = &parsed.dataset.pixels {
        debug_assert!(
            value
                .offset
                .checked_add(value.length)
                .is_some_and(|end| end <= parsed.dataset_size)
        );
        debug_assert!(matches!(vr, b"OB" | b"OW" | b"OF" | b"OD" | b"UN"));
    }
    make_struct(fields, cancel)
}

/// Public-call adapter: share Part 10 admission/parser, then separately admit
/// the decoded image. Caller-supplied info fields never reach this path.
pub(super) fn read(
    engine: &mut Engine,
    path: &Path,
    file_limit: u64,
    dictionary: &Dictionary,
) -> Result<Value> {
    crate::dsp::check_cancel(&engine.cancel)?;
    if !std::fs::metadata(path)
        .map_err(|e| e.to_string())?
        .is_file()
    {
        return Err("dicomread: input must be a regular file".into());
    }
    let file = File::open(path).map_err(|e| e.to_string())?;
    let stat = file.metadata().map_err(|e| e.to_string())?;
    if !stat.is_file() {
        return Err("dicomread: input must be a regular file".into());
    }
    if stat.len() > file_limit {
        return Err(format!("File exceeds the {file_limit}-byte limit"));
    }
    let fixed_cost = path
        .as_os_str()
        .len()
        .checked_mul(8)
        .and_then(|n| n.checked_add(4096))
        .ok_or("dicomread: filename memory accounting overflow")?;
    let budget = super::limits::memory_bytes(&engine.limits)
        .saturating_sub(engine.workspace_bytes())
        .saturating_sub(dictionary.memory_bytes())
        .checked_sub(fixed_cost)
        .ok_or("dicomread: insufficient memory/output budget")?;
    let mut prepared = deflated::prepare(
        file,
        stat.len(),
        budget,
        inflate_work_bytes(engine),
        &engine.cancel,
        dictionary,
    )
    .map_err(|error| {
        if error.contains("undefined length for non-SQ") {
            format!("{error}; encapsulated PixelData is not supported yet")
        } else {
            error
        }
    })?;
    engine.charge(prepared.inflate_units)?;
    let mut reader = prepared.dataset.reader(&mut prepared.original);
    let max_elements = super::limits::memory_units(&engine.limits);
    let array = match prepared.parsed.encoding {
        parser::PixelEncoding::Native => {
            let layout = pixels::layout(
                &mut reader,
                &prepared.parsed,
                prepared.pixel_budget,
                max_elements,
                &engine.cancel,
            )?;
            engine.charge(layout.len)?;
            pixels::decode(&mut reader, &layout, &engine.cancel)?
        }
        parser::PixelEncoding::Rle => {
            let layout = rle::layout(
                &mut reader,
                &prepared.parsed,
                prepared.pixel_budget,
                max_elements,
                &engine.cancel,
            )?;
            engine.charge(layout.units)?;
            rle::decode(&mut reader, &prepared.parsed, &layout, &engine.cancel)?
        }
        #[cfg(feature = "plot-export")]
        parser::PixelEncoding::Jpeg => {
            let layout = jpeg::layout(
                &mut reader,
                &prepared.parsed,
                prepared.pixel_budget,
                max_elements,
                &engine.cancel,
            )?;
            engine.charge(layout.units)?;
            jpeg::decode(&mut reader, &prepared.parsed, &layout, &engine.cancel)?
        }
        parser::PixelEncoding::JpegLossless { .. } => {
            let layout = jpeg::layout(
                &mut reader,
                &prepared.parsed,
                prepared.pixel_budget,
                max_elements,
                &engine.cancel,
            )?;
            engine.charge(layout.units)?;
            jpeg::decode(&mut reader, &prepared.parsed, &layout, &engine.cancel)?
        }
        parser::PixelEncoding::Jpeg2000 { .. } => {
            let layout = j2k::layout(
                &mut reader,
                &prepared.parsed,
                prepared.pixel_budget,
                max_elements,
                &engine.cancel,
            )?;
            engine.charge(layout.units)?;
            j2k::decode(&mut reader, &prepared.parsed, &layout, &engine.cancel)?
        }
        // The JPEG decoder is the optional `image` crate (plot-export/desktop).
        #[cfg(not(feature = "plot-export"))]
        parser::PixelEncoding::Jpeg => {
            return Err("dicomread: transfer syntax 1.2.840.10008.1.2.4.50 (JPEG Baseline) needs the JPEG decoder, which this build lacks (desktop build required)".into());
        }
    };
    Ok(Value::Container(Array::Typed(array)))
}

// Metadata's existing public path borrows Engine immutably. Bound inflation by
// the remaining per-statement work allowance; dicomread additionally charges it.
fn inflate_work_bytes(engine: &Engine) -> usize {
    engine.limits.evaluation_units.map_or(usize::MAX, |units| {
        units
            .saturating_sub(engine.evaluated_elements)
            .saturating_mul(8)
    })
}

fn num(value: f64) -> Value {
    Value::Matrix(Matrix::scalar(value))
}
fn scalar(fields: &[(String, Value)], key: &str) -> Option<f64> {
    fields.iter().find_map(|(name, value)| {
        if name != key {
            return None;
        }
        match value {
            Value::Matrix(m) if m.len() == 1 => Some(m.data()[0]),
            Value::Container(Array::Typed(a)) if a.len() == 1 => Some(a.real(0)),
            _ => None,
        }
    })
}
fn make_struct(fields: Vec<(String, Value)>, cancel: &AtomicBool) -> Result<Value> {
    let mut names = Vec::new();
    let mut values = Vec::new();
    for (name, value) in fields {
        crate::dsp::check_cancel(cancel)?;
        names.push(name);
        values.push(Arc::new(value.array(cancel)?));
    }
    Ok(Value::Container(Array::Struct(StructArray::from_slots(
        1, 1, names, values, cancel,
    )?)))
}

#[allow(clippy::too_many_arguments)]
fn project<R: Read + Seek>(
    reader: &mut R,
    data: &Dataset,
    syntax: TransferSyntax,
    backing: Backing,
    inherited_utf8: bool,
    cancel: &AtomicBool,
    remaining_budget: &mut usize,
    dictionary: &Dictionary,
) -> Result<Vec<(String, Value)>> {
    let mut utf8 = inherited_utf8;
    if let Some(element) = data
        .elements
        .iter()
        .find(|e| e.tag == parser::SPECIFIC_CHARACTER_SET)
    {
        let ElementValue::Primitive(range) = element.value else {
            return Err("dicominfo: invalid SpecificCharacterSet".into());
        };
        let bytes = read_value(reader, range, backing, cancel)?;
        let charset = decode(&bytes, false, *b"CS")?;
        utf8 = match charset.as_str() {
            "" | "ISO_IR 6" => false,
            "ISO_IR 192" => true,
            _ => return Err("dicominfo: unsupported DICOM character set (only ASCII and ISO_IR 192 UTF-8 are supported)".into()),
        };
    }
    let mut fields = Vec::new();
    for element in &data.elements {
        crate::dsp::check_cancel(cancel)?;
        if parser::is_pixel(element.tag) {
            continue;
        }
        let Some(entry) = dictionary.lookup(element.tag) else {
            continue;
        };
        if element.vr == *b"UN" {
            continue;
        }
        let value = match &element.value {
            ElementValue::Sequence(items) => {
                let mut fields = Vec::new();
                for (index, item) in items.iter().enumerate() {
                    crate::dsp::check_cancel(cancel)?;
                    let item = project(
                        reader,
                        item,
                        syntax,
                        backing,
                        utf8,
                        cancel,
                        remaining_budget,
                        dictionary,
                    )?;
                    fields.push((format!("Item_{}", index + 1), make_struct(item, cancel)?));
                }
                make_struct(fields, cancel)?
            }
            ElementValue::Primitive(range) => {
                let bytes = read_value(reader, *range, backing, cancel)?;
                primitive(element, &bytes, syntax, utf8, cancel, remaining_budget)?
            }
            // Encapsulated fragments are pixel payload, never metadata.
            ElementValue::Encapsulated => continue,
        };
        fields.push((entry.field_name(element.tag), value));
    }
    Ok(fields)
}

fn decode(bytes: &[u8], utf8: bool, vr: [u8; 2]) -> Result<String> {
    if !utf8 && !bytes.is_ascii() {
        return Err("dicominfo: non-ASCII text requires ISO_IR 192".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "dicominfo: invalid UTF-8 text")?;
    let text = if vr == *b"UI" {
        text.trim_end_matches('\0')
    } else {
        text.trim_end_matches(' ')
    };
    if text.contains('\0') || text.contains('\u{1b}') {
        return Err("dicominfo: unsupported DICOM text encoding/control character".into());
    }
    Ok(text.to_owned())
}

fn primitive(
    element: &Element,
    bytes: &[u8],
    syntax: TransferSyntax,
    utf8: bool,
    cancel: &AtomicBool,
    remaining_budget: &mut usize,
) -> Result<Value> {
    let vr = element.vr;
    let mixed_descriptor = parser::mixed_lut_descriptor(element.tag, vr);
    let class_width = match &vr {
        b"SS" if mixed_descriptor => Some((NumericClass::Int32, 2)),
        b"US" | b"OW" => Some((NumericClass::Uint16, 2)),
        b"SS" => Some((NumericClass::Int16, 2)),
        b"UL" | b"OL" | b"AT" => Some((NumericClass::Uint32, 4)),
        b"SL" => Some((NumericClass::Int32, 4)),
        b"UV" | b"OV" => Some((NumericClass::Uint64, 8)),
        b"SV" => Some((NumericClass::Int64, 8)),
        b"OB" => Some((NumericClass::Uint8, 1)),
        _ => None,
    };
    if let Some((class, width)) = class_width {
        if !bytes.len().is_multiple_of(width) {
            return Err("dicominfo: invalid numeric VR value length".into());
        }
        let mut values = Vec::new();
        values
            .try_reserve_exact(bytes.len() / width)
            .map_err(|_| "dicominfo: unable to allocate numeric metadata")?;
        for (index, b) in bytes.chunks_exact(width).enumerate() {
            if index % 1024 == 0 {
                crate::dsp::check_cancel(cancel)?;
            }
            let value = match (class, width) {
                (_, 2) if mixed_descriptor => {
                    let cell = syntax.u16(b.try_into().unwrap());
                    if index == 1 {
                        (cell as i16) as i128
                    } else {
                        cell as i128
                    }
                }
                (NumericClass::Int16, _) => (syntax.u16(b.try_into().unwrap()) as i16) as i128,
                (NumericClass::Int32, _) => (syntax.u32(b.try_into().unwrap()) as i32) as i128,
                (NumericClass::Int64, _) => (syntax.u64(b.try_into().unwrap()) as i64) as i128,
                (_, 1) => b[0] as i128,
                (_, 2) => syntax.u16(b.try_into().unwrap()) as i128,
                (_, 4) if vr == *b"AT" => {
                    ((syntax.u16(b[..2].try_into().unwrap()) as i128) << 16)
                        | syntax.u16(b[2..].try_into().unwrap()) as i128
                }
                (_, 4) => syntax.u32(b.try_into().unwrap()) as i128,
                (_, 8) => syntax.u64(b.try_into().unwrap()) as i128,
                _ => unreachable!(),
            };
            values.push(value);
        }
        return Ok(Value::Container(Array::Typed(TypedArray::from_ints(
            class,
            values.len(),
            usize::from(!values.is_empty()),
            values.into_iter(),
        )?)));
    }
    if matches!(&vr, b"FL" | b"FD" | b"OF" | b"OD") {
        let width = if matches!(&vr, b"FL" | b"OF") { 4 } else { 8 };
        if !bytes.len().is_multiple_of(width) {
            return Err("dicominfo: invalid numeric VR value length".into());
        }
        let mut values = Vec::new();
        values
            .try_reserve_exact(bytes.len() / width)
            .map_err(|_| "dicominfo: unable to allocate numeric metadata")?;
        for (index, b) in bytes.chunks_exact(width).enumerate() {
            if index % 1024 == 0 {
                crate::dsp::check_cancel(cancel)?;
            }
            values.push(if width == 4 {
                f32::from_bits(syntax.u32(b.try_into().unwrap())) as f64
            } else {
                f64::from_bits(syntax.u64(b.try_into().unwrap()))
            });
        }
        let rows = values.len();
        let cols = usize::from(rows != 0);
        return if width == 4 {
            Ok(Value::Container(Array::Typed(TypedArray::from_reals(
                NumericClass::Single,
                rows,
                cols,
                values.into_iter(),
            )?)))
        } else {
            Ok(Value::Matrix(Matrix::from_column_vec(rows, cols, values)?))
        };
    }
    // These text VRs can use an extended character repertoire.
    // PN/LO/SH and long text all inherit the dataset's character set.
    let extended = matches!(&vr, b"LO" | b"LT" | b"PN" | b"SH" | b"ST" | b"UC" | b"UT");
    let text = decode(bytes, utf8 && extended, vr)?;
    if matches!(&vr, b"DS" | b"IS") {
        let mut values = Vec::new();
        if !text.is_empty() {
            for (index, part) in text.split('\\').enumerate() {
                if index % 1024 == 0 {
                    crate::dsp::check_cancel(cancel)?;
                }
                let part = part.trim_matches(' ');
                let value = if vr == *b"IS" {
                    part.parse::<i32>()
                        .map(f64::from)
                        .map_err(|_| "dicominfo: invalid IS numeric value")?
                } else {
                    part.parse::<f64>()
                        .map_err(|_| "dicominfo: invalid DS numeric value")?
                };
                if !value.is_finite() {
                    return Err("dicominfo: non-finite DS/IS numeric value".into());
                }
                values.push(value);
            }
        }
        return Ok(Value::Matrix(Matrix::from_column_vec(
            values.len(),
            usize::from(!values.is_empty()),
            values,
        )?));
    }
    if vr == *b"PN" {
        return person_name(&text, cancel, remaining_budget);
    }
    Ok(Value::Text(text))
}

fn person_name(text: &str, cancel: &AtomicBool, remaining_budget: &mut usize) -> Result<Value> {
    const COMPONENTS: [&str; 5] = [
        "FamilyName",
        "GivenName",
        "MiddleName",
        "NamePrefix",
        "NameSuffix",
    ];
    let count = text.split('\\').count();
    if count > 1024 {
        return Err("dicominfo: PN multiplicity exceeds 1024".into());
    }
    // A short PN can expand to five component arrays per representation.
    // Charge that fan-out before allocating any slots, including empty names.
    let cost = count
        .checked_mul(17 * 512)
        .ok_or("dicominfo: PN memory accounting overflow")?;
    *remaining_budget = remaining_budget
        .checked_sub(cost)
        .ok_or("dicominfo: PN exceeds the metadata memory/output budget")?;
    let has_ideographic = text.split('\\').any(|p| p.contains('='));
    let has_phonetic = text.split('\\').any(|p| p.matches('=').count() >= 2);
    let mut names: Vec<String> = COMPONENTS.iter().map(|s| (*s).into()).collect();
    if has_ideographic {
        names.push("Ideographic".into());
    }
    if has_phonetic {
        names.push("Phonetic".into());
    }
    let mut slots = Vec::new();
    for person in text.split('\\') {
        crate::dsp::check_cancel(cancel)?;
        let groups: Vec<&str> = person.split('=').take(4).collect();
        if groups.len() > 3 {
            return Err("dicominfo: PN has more than three component groups".into());
        }
        let components: Vec<&str> = groups[0].split('^').take(6).collect();
        if components.len() > 5 {
            return Err("dicominfo: PN has more than five name components".into());
        }
        for index in 0..5 {
            slots.push(Arc::new(
                Value::Text(components.get(index).copied().unwrap_or("").into()).array(cancel)?,
            ));
        }
        for index in 1..=usize::from(has_ideographic) + usize::from(has_phonetic) {
            let components: Vec<&str> = groups
                .get(index)
                .copied()
                .unwrap_or("")
                .split('^')
                .take(6)
                .collect();
            if components.len() > 5 {
                return Err("dicominfo: PN has more than five name components".into());
            }
            let fields = COMPONENTS
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    (
                        (*name).into(),
                        Value::Text(components.get(i).copied().unwrap_or("").into()),
                    )
                })
                .collect();
            slots.push(Arc::new(make_struct(fields, cancel)?.array(cancel)?));
        }
    }
    Ok(Value::Container(Array::Struct(StructArray::from_slots(
        count, 1, names, slots, cancel,
    )?)))
}
