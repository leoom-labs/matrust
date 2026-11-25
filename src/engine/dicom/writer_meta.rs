//! dicomwrite copy mode: a dicominfo-style struct becomes a bounded, sorted
//! element list. No Engine, file access or pool. Standard tags use the factory
//! dictionary; the session dictionary is read only for private-tag VRs.
use super::super::{custom_dict::Dictionary, parser::TransferSyntax};
use super::{Attribute, fail, header_size, long_vr, text_vr, u16_bytes, u32_bytes};
use crate::{Array, Result, StructArray, typed_array::NumericClass};
use std::sync::atomic::AtomicBool;

/// Reader limits (parser.rs): 16 MiB of metadata, 1 MiB per value.
pub(super) const TOTAL_CAP: usize = 16 * 1024 * 1024;
pub(super) const VALUE_CAP: usize = 1024 * 1024;
/// Engine containers nest 16 levels; an info struct can hold 7 sequence levels.
pub(super) const MAX_SEQUENCE_DEPTH: usize = 6;
pub(super) const MAX_ITEMS: usize = 1024;
pub(super) const MAX_ELEMENTS: usize = 10_000;

/// dicominfo bookkeeping fields: never DICOM attributes, ignored silently.
const BOOKKEEPING: &[&str] = &[
    "Filename",
    "FileModDate",
    "FileSize",
    "Format",
    "FormatVersion",
    "Width",
    "Height",
    "BitDepth",
    "ColorType",
    "StartOfPixelData",
];
/// Option names the MathWorks page also admits as struct fields; not supported here.
const OPTION_FIELDS: &[&str] = &[
    "CompressionMode",
    "CreateMode",
    "Dictionary",
    "Endian",
    "MultiframeSingleFile",
    "ObjectType",
    "TransferSyntax",
    "UseMetadataBitDepths",
    "VR",
    "WritePrivate",
];
/// Root attributes that always come from X.
const DERIVED: &[u32] = &[
    0x0028_0002,
    0x0028_0004,
    0x0028_0006,
    0x0028_0008,
    0x0028_0010,
    0x0028_0011,
    0x0028_0100,
    0x0028_0101,
    0x0028_0102,
    0x0028_0103,
];
const COMPONENTS: [&str; 5] = [
    "FamilyName",
    "GivenName",
    "MiddleName",
    "NamePrefix",
    "NameSuffix",
];
const CHARSET: u32 = 0x0008_0005;
const PHOTOMETRIC: u32 = 0x0028_0004;

pub(super) struct Settings<'a> {
    pub(super) syntax: TransferSyntax,
    pub(super) write_private: bool,
    pub(super) dictionary: &'a Dictionary,
    pub(super) cancel: &'a AtomicBool,
}
/// Result of the root walk; `fields` is unsorted and excludes derived tags.
#[derive(Default)]
pub(super) struct Copied {
    pub(super) fields: Vec<Attribute>,
    /// Some extended-repertoire text is not ASCII.
    pub(super) unicode: bool,
    /// Some SpecificCharacterSet (any level) declares ASCII.
    pub(super) ascii_charset: bool,
    /// Root PhotometricInterpretation was MONOCHROME1 (X cannot tell it from 2).
    pub(super) monochrome1: bool,
    pub(super) media_class: Option<String>,
    pub(super) media_instance: Option<String>,
    total: usize,
    elements: usize,
}
impl Copied {
    /// Charge one encoded element (header plus even-padded value) before it is kept.
    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.total = self
            .total
            .checked_add(bytes)
            .filter(|total| *total <= TOTAL_CAP)
            .ok_or("dicomwrite: serialized metadata exceeds the 16 MiB metadata limit")?;
        self.elements += 1;
        if self.elements > MAX_ELEMENTS {
            return Err("dicomwrite: metadata exceeds 10000 elements".into());
        }
        Ok(())
    }
}

/// Upper bound of the serialized size of `m`, without allocation. Struct
/// construction already bounds nodes (65536) and nesting (16).
pub(super) fn estimate(m: &StructArray) -> Result<usize> {
    let mut total = 0usize;
    for index in 0..m.slot_count() {
        let value = m.slot(index)?;
        let bytes = match value.as_ref() {
            Array::Struct(inner) => estimate(inner)?,
            // UTF-16 unit -> at most three UTF-8 bytes.
            Array::Character(c) => c.units().len().saturating_mul(3),
            Array::Str(s) => s
                .data()
                .iter()
                .map(|v| v.as_ref().map_or(0, |v| v.len()))
                .fold(0usize, usize::saturating_add),
            // Longest DS/IS text plus its separator.
            other => other.len().saturating_mul(17),
        };
        // Element, sequence and item headers plus padding.
        total = total
            .checked_add(bytes)
            .and_then(|n| n.checked_add(32))
            .ok_or("dicomwrite: metadata size overflow")?;
    }
    Ok(total)
}

enum Target {
    Skip,
    Element(u32, [u8; 2]),
}

fn hex4(text: &str) -> Option<u32> {
    (text.len() == 4 && text.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| u32::from_str_radix(text, 16).ok())
        .flatten()
}
/// `Private_gggg_eeee` data element or `Private_gggg_eexx_Creator` (gggg,00ee).
fn private_name(name: &str) -> Option<u32> {
    let rest = name.strip_prefix("Private_")?;
    let group = hex4(rest.get(..4)?)?;
    let rest = rest.get(4..)?.strip_prefix('_')?;
    if let Some(block) = rest.strip_suffix("xx_Creator") {
        let block = (block.len() == 2).then(|| u32::from_str_radix(block, 16).ok())??;
        return Some((group << 16) | block);
    }
    Some((group << 16) | hex4(rest)?)
}

/// Create mode shares Copy's field-name validation without serializing extras.
pub(super) fn create_tag(name: &str, value: &Array, settings: &Settings) -> Result<Option<u32>> {
    match resolve(name, true, 0, value, settings, true)? {
        Target::Skip => Ok(None),
        Target::Element(tag, _) => Ok(Some(tag)),
    }
}

fn resolve(
    name: &str,
    root: bool,
    representation: u16,
    value: &Array,
    settings: &Settings,
    create: bool,
) -> Result<Target> {
    // Create validates names before skipping known group-length tags below.
    // Copy retains its documented broad GroupLength-name removal.
    if (!create && name.contains("GroupLength")) || (root && BOOKKEEPING.contains(&name)) {
        return Ok(Target::Skip);
    }
    if root && OPTION_FIELDS.contains(&name) {
        return Err(fail(
            name,
            "is an option; option fields inside the metadata struct are not supported yet, pass it as a 'Name',value pair",
        ));
    }
    let factory = Dictionary::Factory;
    if let Some(tag) = factory.tag_by_name(name)
        && let Some(entry) = factory.lookup(tag)
        && (entry.keyword == name || entry.field_name(tag) == name)
    {
        // File meta is regenerated; the pixel group and group lengths come from X.
        if tag & 0xFFFF == 0 || (root && matches!(tag >> 16, 0x0002 | 0x7FE0)) {
            return Ok(Target::Skip);
        }
        let mut vr = entry.implicit_vr(tag, representation);
        // "OB or OW": the value class decides, as an explicit VR source did.
        if let Array::Typed(typed) = value
            && entry.vrs.contains(b"OB")
            && entry.vrs.contains(b"OW")
        {
            vr = match typed.class() {
                NumericClass::Uint8 | NumericClass::Int8 => *b"OB",
                NumericClass::Uint16 | NumericClass::Int16 => *b"OW",
                _ => vr,
            };
        }
        return Ok(Target::Element(tag, vr));
    }
    let custom = settings.dictionary.is_custom();
    let private = private_name(name).or_else(|| {
        custom
            .then(|| settings.dictionary.tag_by_name(name))
            .flatten()
            .filter(|tag| tag & 0x0001_0000 != 0)
    });
    let Some(tag) = private else {
        return Err(fail(
            name,
            "is not a DICOM attribute name of the factory dictionary (metadata field names must be attribute names)",
        ));
    };
    let (group, element) = (tag >> 16, tag & 0xFFFF);
    // PS3.5 7.8: groups 0001-0007 and FFFF are never private.
    if group & 1 == 0 || group < 9 || group == 0xFFFF {
        return Err(fail(name, "is not a valid private attribute tag"));
    }
    // Private group lengths are removed like every other group length.
    if element == 0 {
        return Ok(Target::Skip);
    }
    if !settings.write_private {
        return Ok(Target::Skip);
    }
    let vr = match settings.dictionary.lookup(tag).filter(|_| custom) {
        Some(entry) => entry.implicit_vr(tag, representation),
        // PS3.5 7.8.1: private creators are LO; other private data is unknown.
        None if (0x10..=0xFF).contains(&element) => *b"LO",
        None => *b"UN",
    };
    Ok(Target::Element(tag, vr))
}

/// Root dataset of a scalar info struct. `signed` is PixelRepresentation of X.
pub(super) fn root(m: &StructArray, signed: bool, settings: &Settings) -> Result<Copied> {
    let mut state = Copied::default();
    state.fields = dataset(m, 0, u16::from(signed), settings, &mut state)?;
    Ok(state)
}

fn dataset(
    m: &StructArray,
    depth: usize,
    inherited: u16,
    settings: &Settings,
    state: &mut Copied,
) -> Result<Vec<Attribute>> {
    let root = depth == 0;
    // The reader resolves US/SS from the nearest enclosing PixelRepresentation.
    let representation = if root {
        inherited
    } else {
        match m.field_index("PixelRepresentation") {
            Some(index) => match m.get(index, 0)?.as_ref() {
                value @ (Array::Numeric(_) | Array::Typed(_)) if value.len() == 1 => {
                    u16::from(value.scalar(0, 0)?.into_real()? != 0.)
                }
                _ => inherited,
            },
            None => inherited,
        }
    };
    let mut fields: Vec<Attribute> = Vec::new();
    for (index, name) in m.field_names().iter().enumerate() {
        crate::dsp::check_cancel(settings.cancel)?;
        let value = m.get(index, 0)?.as_ref();
        let Target::Element(tag, vr) = resolve(name, root, representation, value, settings, false)?
        else {
            if root
                && matches!(
                    name.as_str(),
                    "MediaStorageSOPClassUID" | "MediaStorageSOPInstanceUID"
                )
            {
                let text = text_of(value, name, settings.cancel)?;
                if name == "MediaStorageSOPClassUID" {
                    state.media_class = Some(text);
                } else {
                    state.media_instance = Some(text);
                }
            }
            continue;
        };
        if root && DERIVED.contains(&tag) {
            if tag == PHOTOMETRIC {
                state.monochrome1 = matches!(value, Array::Character(_) | Array::Str(_))
                    && text_of(value, name, settings.cancel)? == "MONOCHROME1";
            }
            continue;
        }
        if fields.iter().any(|f| f.tag == tag) {
            return Err(fail(
                name,
                "names an attribute that another field already supplies",
            ));
        }
        let bytes = if vr == *b"SQ" {
            sequence(value, name, depth, representation, settings, state)?
        } else {
            primitive(value, vr, tag, name, settings, state)?
        };
        let padded = bytes.len() + bytes.len() % 2;
        if settings.syntax != TransferSyntax::ImplicitLittle && !long_vr(vr) && padded > 0xFFFE {
            return Err(fail(
                name,
                &format!(
                    "value exceeds the 65534-byte limit of VR {}",
                    String::from_utf8_lossy(&vr)
                ),
            ));
        }
        // A sequence charged its items and their elements; add only the header.
        state.charge(if vr == *b"SQ" {
            header_size(settings.syntax, vr)
        } else {
            header_size(settings.syntax, vr) + padded
        })?;
        fields.push(Attribute {
            tag,
            vr,
            value: bytes,
        });
    }
    fields.sort_unstable_by_key(|f| f.tag);
    Ok(fields)
}

/// Items with defined lengths (PS3.5 7.5), each a nested dataset.
fn sequence(
    value: &Array,
    name: &str,
    depth: usize,
    representation: u16,
    settings: &Settings,
    state: &mut Copied,
) -> Result<Vec<u8>> {
    let items = match value {
        Array::Struct(items) if items.len() == 1 => items,
        other if other.is_empty() => return Ok(Vec::new()),
        _ => {
            return Err(fail(
                name,
                "is a sequence: its value must be a scalar struct with fields Item_1, Item_2, ...",
            ));
        }
    };
    if depth >= MAX_SEQUENCE_DEPTH {
        return Err(fail(
            name,
            &format!("exceeds {MAX_SEQUENCE_DEPTH} nested sequence levels"),
        ));
    }
    let count = items.field_count();
    if count > MAX_ITEMS {
        return Err(fail(name, &format!("exceeds {MAX_ITEMS} sequence items")));
    }
    let mut out = Vec::new();
    for number in 1..=count {
        crate::dsp::check_cancel(settings.cancel)?;
        let item = items
            .field_index(&format!("Item_{number}"))
            .ok_or_else(|| {
                fail(
                    name,
                    "is a sequence: its fields must be exactly Item_1 ... Item_n",
                )
            })?;
        let Array::Struct(item) = items.get(item, 0)?.as_ref() else {
            return Err(fail(
                name,
                &format!("Item_{number} must be a scalar struct"),
            ));
        };
        if item.len() != 1 {
            return Err(fail(
                name,
                &format!("Item_{number} must be a scalar struct"),
            ));
        }
        state.charge(8)?;
        let fields = dataset(item, depth + 1, representation, settings, state)?;
        let length = super::encoded_fields_len(&fields, settings.syntax)?;
        let total = out
            .len()
            .checked_add(length)
            .and_then(|n| n.checked_add(8))
            .filter(|n| *n <= TOTAL_CAP)
            .ok_or("dicomwrite: serialized metadata exceeds the 16 MiB metadata limit")?;
        out.try_reserve(total - out.len())
            .map_err(|_| "dicomwrite: unable to allocate sequence metadata")?;
        out.extend_from_slice(&u16_bytes(settings.syntax, 0xFFFE));
        out.extend_from_slice(&u16_bytes(settings.syntax, 0xE000));
        out.extend_from_slice(&u32_bytes(
            settings.syntax,
            u32::try_from(length).map_err(|_| "dicomwrite: item length overflow")?,
        ));
        for field in &fields {
            super::emit(&mut out, field, settings.syntax)?;
        }
    }
    Ok(out)
}

fn text_of(value: &Array, name: &str, cancel: &AtomicBool) -> Result<String> {
    match value {
        other if other.is_empty() => Ok(String::new()),
        Array::Character(c) if c.is_text_row() && c.ndims() == 2 => c.to_utf8(cancel),
        Array::Str(s) if s.len() == 1 => s
            .data()
            .first()
            .and_then(|v| v.as_ref())
            .filter(|v| v.len() <= VALUE_CAP)
            .map(|v| v.to_string())
            .ok_or_else(|| fail(name, "must be nonmissing text within the 1 MiB value limit")),
        _ => Err(fail(
            name,
            "must be a character row vector or a nonmissing string scalar",
        )),
    }
}

/// Real values of a numeric array in storage order, polled every 4096 values.
fn reals(
    value: &Array,
    name: &str,
    cancel: &AtomicBool,
    mut each: impl FnMut(f64) -> Result<()>,
) -> Result<()> {
    for index in 0..value.len() {
        if index % 4096 == 0 {
            crate::dsp::check_cancel(cancel)?;
        }
        let number = match value {
            Array::Numeric(m) => m.data()[index],
            Array::Typed(t) => t.real(index),
            _ => return Err(fail(name, "must be a real numeric array")),
        };
        each(number)?;
    }
    Ok(())
}
/// Integer values with exact 64-bit classes; doubles must hold whole numbers.
fn integers(
    value: &Array,
    name: &str,
    cancel: &AtomicBool,
    mut each: impl FnMut(i128) -> Result<()>,
) -> Result<()> {
    if let Array::Typed(t) = value
        && t.class() != NumericClass::Single
    {
        for index in 0..t.len() {
            if index % 4096 == 0 {
                crate::dsp::check_cancel(cancel)?;
            }
            each(t.int(index))?;
        }
        return Ok(());
    }
    reals(value, name, cancel, |number| {
        if !number.is_finite() || number.fract() != 0. || number.abs() > 1.8e19 {
            return Err(fail(name, "must hold finite integers"));
        }
        each(number as i128)
    })
}

/// DS text: shortest decimal that fits 16 bytes (PS3.5 6.2).
fn decimal(number: f64) -> String {
    let plain = format!("{number}");
    if plain.len() <= 16 {
        return plain;
    }
    let exponent = format!("{number:e}");
    if exponent.len() <= 16 {
        return exponent;
    }
    (0..=15usize)
        .rev()
        .map(|digits| format!("{number:.digits$e}"))
        .find(|text| text.len() <= 16)
        .unwrap_or(exponent)
}

fn name_group(
    person: &StructArray,
    element: usize,
    name: &str,
    cancel: &AtomicBool,
) -> Result<String> {
    let mut parts = [const { String::new() }; 5];
    for (slot, component) in parts.iter_mut().zip(COMPONENTS) {
        if let Some(index) = person.field_index(component) {
            *slot = text_of(person.get(index, element)?, name, cancel)?;
            if slot.contains(['^', '=', '\\']) {
                return Err(fail(
                    name,
                    "name components must not contain '^', '=' or a backslash",
                ));
            }
        }
    }
    Ok(parts.join("^").trim_end_matches('^').to_owned())
}
/// The struct form dicominfo produces: one element per person, five component
/// fields and optional Ideographic/Phonetic component structs.
pub(super) fn person_names(
    people: &StructArray,
    name: &str,
    cancel: &AtomicBool,
) -> Result<String> {
    if let Some(unknown) = people.field_names().iter().find(|field| {
        !COMPONENTS.contains(&field.as_str())
            && !matches!(field.as_str(), "Ideographic" | "Phonetic")
    }) {
        return Err(fail(
            name,
            &format!("has the unknown person-name field '{unknown}'"),
        ));
    }
    let mut out = String::new();
    for element in 0..people.len() {
        crate::dsp::check_cancel(cancel)?;
        let mut groups = vec![name_group(people, element, name, cancel)?];
        for extra in ["Ideographic", "Phonetic"] {
            let group = match people.field_index(extra) {
                Some(index) => match people.get(index, element)?.as_ref() {
                    Array::Struct(group) if group.len() == 1 => name_group(group, 0, name, cancel)?,
                    other if other.is_empty() => String::new(),
                    _ => {
                        return Err(fail(
                            name,
                            &format!("{extra} must be a scalar struct of name components"),
                        ));
                    }
                },
                None => String::new(),
            };
            groups.push(group);
        }
        if element != 0 {
            out.push('\\');
        }
        out.push_str(groups.join("=").trim_end_matches('='));
        if out.len() > VALUE_CAP {
            return Err(fail(name, "exceeds the 1 MiB value limit"));
        }
    }
    Ok(out)
}

fn primitive(
    value: &Array,
    vr: [u8; 2],
    tag: u32,
    name: &str,
    settings: &Settings,
    state: &mut Copied,
) -> Result<Vec<u8>> {
    let cancel = settings.cancel;
    let little = settings.syntax.little();
    let numeric = matches!(value, Array::Numeric(_) | Array::Typed(_));
    let width = match &vr {
        b"OB" | b"UN" => 1,
        b"US" | b"SS" | b"OW" => 2,
        b"UL" | b"SL" | b"OL" | b"AT" | b"FL" | b"OF" => 4,
        b"UV" | b"SV" | b"OV" | b"FD" | b"OD" => 8,
        _ => 0,
    };
    if text_vr(vr) || (vr == *b"UN" && !numeric) {
        let text = match value {
            Array::Struct(people) if vr == *b"PN" => person_names(people, name, cancel)?,
            _ if numeric && !value.is_empty() && matches!(&vr, b"DS" | b"IS") => {
                if value.len() > VALUE_CAP / 17 {
                    return Err(fail(name, "exceeds the 1 MiB value limit"));
                }
                let mut out = String::new();
                reals(value, name, cancel, |number| {
                    if !out.is_empty() {
                        out.push('\\');
                    }
                    if vr == *b"IS" {
                        if number.fract() != 0.
                            || number < f64::from(i32::MIN)
                            || number > f64::from(i32::MAX)
                        {
                            return Err(fail(name, "must hold signed 32-bit integers (VR IS)"));
                        }
                        out.push_str(&format!("{number:.0}"));
                    } else if number.is_finite() {
                        out.push_str(&decimal(number));
                    } else {
                        return Err(fail(name, "must hold finite numbers (VR DS)"));
                    }
                    Ok(())
                })?;
                out
            }
            _ => text_of(value, name, cancel)?,
        };
        if text.len() > VALUE_CAP {
            return Err(fail(name, "exceeds the 1 MiB value limit"));
        }
        if vr == *b"UN" {
            return Ok(text.into_bytes());
        }
        // The reader rejects NUL and ESC (ISO 2022) in every text value.
        if text.contains(['\0', '\u{1b}']) {
            return Err(fail(name, "text must not contain NUL or ESC characters"));
        }
        if !text.is_ascii() {
            if !matches!(&vr, b"LO" | b"LT" | b"PN" | b"SH" | b"ST" | b"UC" | b"UT") {
                return Err(fail(
                    name,
                    &format!("VR {} text must be ASCII", String::from_utf8_lossy(&vr)),
                ));
            }
            state.unicode = true;
        }
        if tag == CHARSET {
            match text.as_str() {
                "ISO_IR 192" => {}
                "" | "ISO_IR 6" => state.ascii_charset = true,
                _ => return Err("dicomwrite: SpecificCharacterSet supports only ASCII/ISO_IR 6 or ISO_IR 192; UTF-8 text requires ISO_IR 192".into()),
            }
        }
        return Ok(text.into_bytes());
    }
    if width == 0 {
        return Err(fail(
            name,
            &format!(
                "has VR {}, which is not supported yet",
                String::from_utf8_lossy(&vr)
            ),
        ));
    }
    if !numeric && !value.is_empty() {
        return Err(fail(
            name,
            &format!("must be numeric for VR {}", String::from_utf8_lossy(&vr)),
        ));
    }
    let size = value
        .len()
        .checked_mul(width)
        .filter(|size| *size <= VALUE_CAP)
        .ok_or_else(|| fail(name, "exceeds the 1 MiB value limit"))?;
    let mut out = Vec::new();
    out.try_reserve_exact(size + size % 2)
        .map_err(|_| "dicomwrite: unable to allocate metadata value")?;
    if value.is_empty() {
        return Ok(out);
    }
    if matches!(&vr, b"FL" | b"OF" | b"FD" | b"OD") {
        reals(value, name, cancel, |number| {
            if width == 4 {
                let bits = (number as f32).to_bits();
                out.extend_from_slice(&if little {
                    bits.to_le_bytes()
                } else {
                    bits.to_be_bytes()
                });
            } else {
                let bits = number.to_bits();
                out.extend_from_slice(&if little {
                    bits.to_le_bytes()
                } else {
                    bits.to_be_bytes()
                });
            }
            Ok(())
        })?;
        return Ok(out);
    }
    // Two's complement bit patterns of the signed classes are kept for O* data.
    let signed_class = matches!(
        value,
        Array::Typed(t) if matches!(t.class(), NumericClass::Int8 | NumericClass::Int16 | NumericClass::Int32 | NumericClass::Int64)
    );
    let bits = 8 * width as u32;
    let (low, high): (i128, i128) = match &vr {
        b"SS" | b"SL" | b"SV" => (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1),
        b"OB" | b"UN" | b"OW" | b"OL" | b"OV" if signed_class => {
            (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1)
        }
        _ => (0, (1i128 << bits) - 1),
    };
    let mixed_descriptor = super::super::parser::mixed_lut_descriptor(tag, vr);
    let mut index = 0;
    integers(value, name, cancel, |number| {
        let (low, high) = if mixed_descriptor && index != 1 {
            (0, i128::from(u16::MAX))
        } else {
            (low, high)
        };
        index += 1;
        if number < low || number > high {
            return Err(fail(
                name,
                &format!(
                    "value {number} is outside the range of VR {}",
                    String::from_utf8_lossy(&vr)
                ),
            ));
        }
        // Truncation keeps the two's complement pattern of admitted negatives.
        let cell = number as u64;
        if vr == *b"AT" {
            out.extend_from_slice(&u16_bytes(settings.syntax, (cell >> 16) as u16));
            out.extend_from_slice(&u16_bytes(settings.syntax, cell as u16));
        } else if little {
            out.extend_from_slice(&cell.to_le_bytes()[..width]);
        } else {
            out.extend_from_slice(&cell.to_be_bytes()[8 - width..]);
        }
        Ok(())
    })?;
    Ok(out)
}
