//! Bounded native Part 10 encoder. No Engine, file publication, parser or pool.
//! The immutable Plan borrows pixels; metadata is validated before serialization.
use super::{custom_dict::Dictionary, dictionary, parser::TransferSyntax};
use crate::{
    Array, Matrix, Result, StructArray,
    typed_array::{NumericClass, TypedArray, TypedData},
};
use std::sync::atomic::AtomicBool;

#[path = "writer_meta.rs"]
mod meta;

/// What the public call selected. `copy` writes every attribute of the info
/// struct (CreateMode 'copy'); otherwise only the verified ALLOWED subset.
pub(crate) struct Options<'a> {
    pub(crate) syntax: TransferSyntax,
    pub(crate) copy: bool,
    pub(crate) write_private: bool,
    /// Session dictionary: read only for the VR of private tags.
    pub(crate) dictionary: &'a Dictionary,
}
#[cfg(test)]
impl Options<'static> {
    pub(crate) fn create(syntax: TransferSyntax) -> Self {
        Self {
            syntax,
            copy: false,
            write_private: false,
            dictionary: &Dictionary::Factory,
        }
    }
}

pub(crate) const FILE_CAP: usize = 256 * 1024 * 1024;
const META_CAP: usize = 32 * 1024;
const SC: &str = "1.2.840.10008.5.1.4.1.1.7";
// UUIDv5(URL namespace, https://github.com/matrust/matrust/dicom/writer/1).
// Deterministic project identity, not a registered organizational root. SOURCES.md.
const IMPLEMENTATION: &str = "2.25.96426248368486820571789363904423688279";
const ALLOWED: &[u32] = &[
    0x0008_0005,
    0x0008_0016,
    0x0008_0018,
    0x0008_0020,
    0x0008_0030,
    0x0008_0060,
    0x0008_1030,
    0x0008_103E,
    0x0010_0010,
    0x0010_0020,
    0x0020_000D,
    0x0020_000E,
    0x0020_0011,
    0x0020_0013,
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

pub(crate) fn syntax(uid: &str) -> Result<TransferSyntax> {
    match uid {
        "1.2.840.10008.1.2" => Ok(TransferSyntax::ImplicitLittle),
        "1.2.840.10008.1.2.1" => Ok(TransferSyntax::ExplicitLittle),
        "1.2.840.10008.1.2.2" => Ok(TransferSyntax::ExplicitBig),
        _ => Err(format!(
            "dicomwrite: unsupported TransferSyntax UID '{uid}' (only native implicit LE, explicit LE and explicit BE are supported)"
        )),
    }
}
fn syntax_uid(s: TransferSyntax) -> &'static str {
    match s {
        TransferSyntax::ImplicitLittle => "1.2.840.10008.1.2",
        TransferSyntax::ExplicitLittle => "1.2.840.10008.1.2.1",
        TransferSyntax::ExplicitBig => "1.2.840.10008.1.2.2",
    }
}
fn fail(name: &str, reason: &str) -> String {
    format!("dicomwrite: {name} {reason}")
}

/// Admitted pixel geometry. Public dims are [R,C] gray, [R,C,3] RGB,
/// [R,C,1,F] gray frames or [R,C,3,F] RGB frames; storage is column-major
/// r + R*(c + C*(s + S*f)).
#[derive(Clone, Copy)]
struct Shape {
    rows: usize,
    cols: usize,
    samples: usize,
    frames: usize,
    /// Bytes per sample (BitsAllocated / 8).
    width: usize,
    signed: bool,
    /// Even-padded PixelData value length.
    payload: usize,
}

/// Class and rank rules only; every product is checked in `pixel_size`.
fn shape(pixels: &TypedArray, max_elements: usize) -> Result<Shape> {
    let (width, signed) = match pixels.class() {
        NumericClass::Uint8 => (1, false),
        NumericClass::Int8 => (1, true),
        NumericClass::Uint16 => (2, false),
        NumericClass::Int16 => (2, true),
        class => {
            return Err(format!(
                "dicomwrite: X of class {} is not supported; X must be int8, int16, uint8 or uint16",
                class.name()
            ));
        }
    };
    let (rows, cols, samples, frames) = match *pixels.dims() {
        [r, c] => (r, c, 1, 1),
        [r, c, 3] => (r, c, 3, 1),
        [_, _, n] => {
            return Err(format!(
                "dicomwrite: 3-D X must be R-by-C-by-3 truecolor (third dimension is {n}); write grayscale frames as 4-D R-by-C-by-1-by-F"
            ));
        }
        [r, c, 1, f] => (r, c, 1, f),
        [r, c, 3, f] => (r, c, 3, f),
        [_, _, n, _] => {
            return Err(format!(
                "dicomwrite: 4-D X must be R-by-C-by-1-by-F or R-by-C-by-3-by-F (third dimension is {n})"
            ));
        }
        _ => {
            return Err(
                "dicomwrite: X must be 2-D grayscale, R-by-C-by-3 truecolor or 4-D multiframe; more than 4 dimensions are not supported"
                    .into(),
            );
        }
    };
    if signed && samples == 3 {
        return Err(
            "dicomwrite: signed truecolor X is not supported yet; RGB requires uint8 or uint16"
                .into(),
        );
    }
    let payload = pixel_size(rows, cols, samples, frames, width, max_elements)?;
    // `pixel_size` proved the product; typed storage must hold exactly that.
    if rows * cols * samples * frames != pixels.len() {
        return Err("dicomwrite: X dimensions disagree with its storage".into());
    }
    Ok(Shape {
        rows,
        cols,
        samples,
        frames,
        width,
        signed,
        payload,
    })
}

/// Conservative peak includes borrowed inputs (also if temporary), metadata text
/// projections/capacities, element descriptors and the single serialized buffer.
/// Called before any metadata- or output-sized allocation.
pub(crate) fn admission(
    pixels: &TypedArray,
    metadata: Option<&StructArray>,
    options: &Options,
    budget: usize,
    max_elements: usize,
) -> Result<usize> {
    admit(pixels, metadata, options.copy, budget, max_elements).map(|(_, units)| units)
}
fn admit(
    pixels: &TypedArray,
    metadata: Option<&StructArray>,
    copy: bool,
    budget: usize,
    max_elements: usize,
) -> Result<(Shape, usize)> {
    let shape = shape(pixels, max_elements)?;
    let mut copied = 0;
    if let Some(m) = metadata {
        if m.len() != 1 {
            return Err("dicomwrite: metadata must be a scalar struct".into());
        }
        if copy {
            // Element values, one nested sequence copy and the serialized bytes.
            copied = meta::estimate(m)?
                .min(meta::TOTAL_CAP)
                .checked_mul(3)
                .ok_or("dicomwrite: memory accounting overflow")?;
        }
    } else if copy {
        return Err(
            "dicomwrite: CreateMode 'copy' needs a metadata struct (dicominfo output) to copy"
                .into(),
        );
    }
    let metadata_bytes = metadata.map_or(0, StructArray::bytes);
    let peak = metadata_bytes
        .checked_mul(4)
        .and_then(|n| n.checked_add(META_CAP * 3))
        .and_then(|n| n.checked_add(copied))
        .and_then(|n| n.checked_add(pixels.bytes()))
        .and_then(|n| n.checked_add(shape.payload))
        .ok_or("dicomwrite: memory accounting overflow")?;
    if peak > budget {
        return Err(
            "dicomwrite: input, metadata and serialized output exceed the memory budget".into(),
        );
    }
    // Charge traversal plus all conservative transient allocation units.
    let units = pixels
        .len()
        .checked_add(peak.div_ceil(8))
        .ok_or("dicomwrite: evaluation accounting overflow")?;
    Ok((shape, units))
}
/// Checked R*C*S*F*bytes (plus even padding) before any allocation.
fn pixel_size(
    rows: usize,
    cols: usize,
    samples: usize,
    frames: usize,
    width: usize,
    max_elements: usize,
) -> Result<usize> {
    if rows == 0 || cols == 0 || rows > 65535 || cols > 65535 {
        return Err("dicomwrite: nonempty Rows and Columns must each be in 1..65535".into());
    }
    if frames == 0 {
        return Err("dicomwrite: multiframe X must contain at least one frame".into());
    }
    let folded = cols
        .checked_mul(samples)
        .and_then(|n| n.checked_mul(frames))
        .ok_or("dicomwrite: sample count overflow")?;
    let n = rows
        .checked_mul(folded)
        .ok_or("dicomwrite: sample count overflow")?;
    let bytes = n
        .checked_mul(width)
        .and_then(|b| b.checked_add(b % 2))
        .ok_or("dicomwrite: pixel byte length overflow")?;
    if bytes > FILE_CAP - META_CAP {
        return Err("dicomwrite: encoded file exceeds the 256 MiB file cap".into());
    }
    Matrix::checked_len(rows, folded)?;
    if n > max_elements {
        return Err("dicomwrite: samples exceed the engine element/memory limit".into());
    }
    Ok(bytes)
}

pub(crate) fn text(value: &Array, name: &str, cancel: &AtomicBool) -> Result<String> {
    // Maximum PN is three 64-character groups (UTF-16/UTF-8 admitted below).
    if value.bytes() > 2048 {
        return Err(fail(name, "text exceeds the supported VR length"));
    }
    match value {
        Array::Character(c) if c.is_text_row() && c.ndims() == 2 => c.to_utf8(cancel),
        Array::Str(s) if s.len() == 1 => s
            .data()
            .first()
            .and_then(|v| v.as_ref())
            .map(|v| v.to_string())
            .ok_or_else(|| fail(name, "must be nonmissing scalar text")),
        _ => Err(fail(
            name,
            "must be a character vector or nonmissing string scalar",
        )),
    }
}
fn number(value: &Array, name: &str) -> Result<f64> {
    if value.len() != 1 || !matches!(value, Array::Numeric(_) | Array::Typed(_)) {
        return Err(fail(name, "must be one real numeric integer"));
    }
    let n = value.scalar(0, 0)?.into_real()?;
    if !n.is_finite() || n.fract() != 0. {
        return Err(fail(name, "must be one finite integer"));
    }
    Ok(n)
}
/// IS count supplied as a numeric scalar or as decimal integer text.
fn count(value: &Array, name: &str, cancel: &AtomicBool) -> Result<f64> {
    if matches!(value, Array::Numeric(_) | Array::Typed(_)) {
        return number(value, name);
    }
    let text = text(value, name, cancel)?;
    validate_text(&text, *b"IS", name)?;
    text.parse::<i32>()
        .map(f64::from)
        .map_err(|_| fail(name, "must be one integer"))
}
fn valid_uid(s: &str) -> bool {
    if s.is_empty() || s.len() > 64 {
        return false;
    }
    let mut parts = s.split('.');
    let root = parts.next().unwrap_or("");
    let second = parts.next().unwrap_or("");
    let component = |p: &str| {
        !p.is_empty()
            && p.bytes().all(|c| c.is_ascii_digit())
            && (p.len() == 1 || !p.starts_with('0'))
    };
    matches!(root, "0" | "1" | "2")
        && component(second)
        && (root == "2" || second.parse::<u8>().is_ok_and(|v| v <= 39))
        && parts.all(component)
}
fn valid_date(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    if s.len() != 8 || !s.bytes().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let year = s.get(..4).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
    let month = s
        .get(4..6)
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    let day = s.get(6..).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = [
        0,
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    year > 0 && day > 0 && days.get(month).is_some_and(|limit| day <= *limit)
}
fn valid_time(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    let (base, fraction) = s.split_once('.').map_or((s, None), |(a, b)| (a, Some(b)));
    if !matches!(base.len(), 2 | 4 | 6) || !base.bytes().all(|c| c.is_ascii_digit()) {
        return false;
    }
    if fraction.is_some_and(|f| {
        base.len() != 6 || f.is_empty() || f.len() > 6 || !f.bytes().all(|c| c.is_ascii_digit())
    }) {
        return false;
    }
    for (i, &[tens, units]) in base.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let n = u16::from(tens - b'0') * 10 + u16::from(units - b'0');
        if n > if i == 0 {
            23
        } else if i == 1 {
            59
        } else {
            60
        } {
            return false;
        }
    }
    true
}
fn validate_text(s: &str, vr: [u8; 2], name: &str) -> Result<()> {
    if s.contains('\\') || s.chars().any(char::is_control) || s.ends_with(' ') {
        return Err(fail(
            name,
            "must have VM=1 without controls, NUL or caller-supplied trailing padding",
        ));
    }
    let valid = match &vr {
        b"UI" => valid_uid(s),
        b"PN" => {
            s.split('=').count() <= 3
                && s.split('=')
                    .all(|g| g.chars().count() <= 64 && g.split('^').count() <= 5)
        }
        b"LO" => s.chars().count() <= 64,
        b"CS" => {
            s.len() <= 16
                && s.bytes()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_' || c == b' ')
        }
        b"DA" => valid_date(s),
        b"TM" => valid_time(s),
        b"IS" => s.is_empty() || (s.len() <= 12 && s.parse::<i32>().is_ok()),
        _ => false,
    };
    if !valid {
        return Err(fail(
            name,
            "has invalid VR length, value, multiplicity or UID syntax",
        ));
    }
    Ok(())
}
struct Attribute {
    tag: u32,
    vr: [u8; 2],
    value: Vec<u8>,
}
impl Attribute {
    fn text(tag: u32, vr: [u8; 2], text: &str) -> Self {
        Self {
            tag,
            vr,
            value: text.as_bytes().to_vec(),
        }
    }
    fn length(&self) -> usize {
        self.value.len() + self.value.len() % 2
    }
}
fn vr(tag: u32) -> Result<[u8; 2]> {
    dictionary::lookup(tag)
        .map(|entry| entry.vrs[0])
        .ok_or_else(|| "dicomwrite: missing encoder VR".into())
}
fn put_default(fields: &mut Vec<Attribute>, tag: u32, text: &str) -> Result<()> {
    if !fields.iter().any(|f| f.tag == tag) {
        fields.push(Attribute::text(tag, vr(tag)?, text));
    }
    Ok(())
}
fn u16_bytes(s: TransferSyntax, n: u16) -> [u8; 2] {
    if s.little() {
        n.to_le_bytes()
    } else {
        n.to_be_bytes()
    }
}
fn u32_bytes(s: TransferSyntax, n: u32) -> [u8; 4] {
    if s.little() {
        n.to_le_bytes()
    } else {
        n.to_be_bytes()
    }
}
/// PS3.5 7.1.2: VRs with a reserved field and a 32-bit length in explicit syntaxes.
fn long_vr(vr: [u8; 2]) -> bool {
    matches!(
        &vr,
        b"OB"
            | b"OD"
            | b"OF"
            | b"OL"
            | b"OV"
            | b"OW"
            | b"SQ"
            | b"SV"
            | b"UC"
            | b"UN"
            | b"UR"
            | b"UT"
            | b"UV"
    )
}
/// Character-string VRs (PS3.5 6.2); all but UI pad with a space.
fn text_vr(vr: [u8; 2]) -> bool {
    matches!(
        &vr,
        b"AE"
            | b"AS"
            | b"CS"
            | b"DA"
            | b"DS"
            | b"DT"
            | b"IS"
            | b"LO"
            | b"LT"
            | b"PN"
            | b"SH"
            | b"ST"
            | b"TM"
            | b"UC"
            | b"UI"
            | b"UR"
            | b"UT"
    )
}
fn header_size(s: TransferSyntax, vr: [u8; 2]) -> usize {
    if s != TransferSyntax::ImplicitLittle && long_vr(vr) {
        12
    } else {
        8
    }
}
fn header(out: &mut Vec<u8>, tag: u32, vr: [u8; 2], len: usize, s: TransferSyntax) -> Result<()> {
    let group = u16::try_from(tag >> 16).map_err(|_| "dicomwrite: tag overflow")?;
    let element = u16::try_from(tag & 0xffff).map_err(|_| "dicomwrite: tag overflow")?;
    out.extend_from_slice(&u16_bytes(s, group));
    out.extend_from_slice(&u16_bytes(s, element));
    if s == TransferSyntax::ImplicitLittle {
        out.extend_from_slice(&u32_bytes(
            s,
            u32::try_from(len).map_err(|_| "dicomwrite: VL overflow")?,
        ));
    } else {
        out.extend_from_slice(&vr);
        if header_size(s, vr) == 12 {
            out.extend_from_slice(&[0, 0]);
            out.extend_from_slice(&u32_bytes(
                s,
                u32::try_from(len).map_err(|_| "dicomwrite: VL overflow")?,
            ));
        } else {
            out.extend_from_slice(&u16_bytes(
                s,
                u16::try_from(len).map_err(|_| "dicomwrite: short VR VL overflow")?,
            ));
        }
    }
    Ok(())
}
fn emit(out: &mut Vec<u8>, a: &Attribute, s: TransferSyntax) -> Result<()> {
    header(out, a.tag, a.vr, a.length(), s)?;
    out.extend_from_slice(&a.value);
    if !a.value.len().is_multiple_of(2) {
        out.push(if text_vr(a.vr) && a.vr != *b"UI" {
            b' '
        } else {
            0
        });
    }
    Ok(())
}

pub(crate) struct Plan<'a> {
    pixels: &'a TypedArray,
    shape: Shape,
    syntax: TransferSyntax,
    fields: Vec<Attribute>,
    /// Attributes whose tags follow PixelData (copy mode only).
    trailer: Vec<Attribute>,
    meta: Vec<Attribute>,
    pixel_vr: [u8; 2],
    pub(crate) encoded_len: usize,
}
/// Admission must precede this call (the public adapter charges evaluation then).
/// Rechecks admission itself so reuse cannot bypass allocation limits.
pub(crate) fn prepare<'a>(
    pixels: &'a TypedArray,
    metadata: Option<&StructArray>,
    options: &Options,
    budget: usize,
    max_elements: usize,
    cancel: &AtomicBool,
) -> Result<Plan<'a>> {
    crate::dsp::check_cancel(cancel)?;
    let syntax = options.syntax;
    let (shape, _) = admit(pixels, metadata, options.copy, budget, max_elements)?;
    let bits: u16 = if shape.width == 1 { 8 } else { 16 };
    let rgb = shape.samples == 3;
    let mut photometric = if rgb { "RGB" } else { "MONOCHROME2" };
    // IS admits signed 32-bit decimal text; the file cap bounds frames far below.
    let frames = i32::try_from(shape.frames).map_err(|_| "dicomwrite: NumberOfFrames overflow")?;
    let derived = [
        (
            0x0028_0010,
            u16::try_from(shape.rows).map_err(|_| "dicomwrite: Rows overflow")?,
        ),
        (
            0x0028_0011,
            u16::try_from(shape.cols).map_err(|_| "dicomwrite: Columns overflow")?,
        ),
        (0x0028_0002, if rgb { 3 } else { 1 }),
        (0x0028_0100, bits),
        (0x0028_0101, bits),
        (0x0028_0102, bits - 1),
        (0x0028_0103, u16::from(shape.signed)),
    ];
    let mut fields = Vec::with_capacity(40);
    // Copy mode: every attribute of the info struct; pixel attributes still come from X.
    let mut copied = None;
    if options.copy {
        let m = metadata.ok_or("dicomwrite: CreateMode 'copy' needs a metadata struct")?;
        let mut walk = meta::root(
            m,
            shape.signed,
            &meta::Settings {
                syntax,
                write_private: options.write_private,
                dictionary: options.dictionary,
                cancel,
            },
        )?;
        if walk.unicode && walk.ascii_charset {
            return Err("dicomwrite: SpecificCharacterSet supports only ASCII/ISO_IR 6 or ISO_IR 192; UTF-8 text requires ISO_IR 192".into());
        }
        // X cannot tell MONOCHROME1 from MONOCHROME2: keep the declared polarity.
        if walk.monochrome1 && !rgb {
            photometric = "MONOCHROME1";
        }
        fields = std::mem::take(&mut walk.fields);
        fields.push(Attribute::text(0x0028_0004, vr(0x0028_0004)?, photometric));
        copied = Some(walk);
    } else if let Some(m) = metadata {
        let settings = meta::Settings {
            syntax,
            write_private: options.write_private,
            dictionary: options.dictionary,
            cancel,
        };
        for (index, name) in m.field_names().iter().enumerate() {
            crate::dsp::check_cancel(cancel)?;
            let value = m.get(index, 0)?;
            // Validate the field name, then silently drop recognized extras
            // (MathWorks dicomwrite CreateMode and meta_struct contract).
            let Some(tag) = meta::create_tag(name, value, &settings)? else {
                continue;
            };
            if !ALLOWED.contains(&tag) {
                continue;
            }
            let vr = vr(tag)?;
            if let Some((_, expected)) = derived.iter().find(|(t, _)| *t == tag) {
                if number(value, name)? != f64::from(*expected) {
                    return Err(fail(
                        name,
                        &format!(
                            "contradicts the pixel array (X requires {expected}; narrower stored-bit writing is not supported)"
                        ),
                    ));
                }
                continue;
            }
            if tag == 0x0028_0006 {
                // Type 1C: present only with more than one sample (PS3.3 C.7.6.3).
                if !rgb {
                    return Err(fail(
                        name,
                        "contradicts the pixel array (PlanarConfiguration applies only to RGB X)",
                    ));
                }
                if number(value, name)? != 0. {
                    return Err(fail(
                        name,
                        "contradicts the pixel array (only color-by-pixel PlanarConfiguration 0 is written)",
                    ));
                }
                continue;
            }
            if tag == 0x0028_0008 {
                if count(value, name, cancel)? != f64::from(frames) {
                    return Err(fail(
                        name,
                        &format!("contradicts the pixel array (X has {frames} frame(s))"),
                    ));
                }
                continue;
            }
            let text = if vr == *b"IS" && value.is_empty() {
                // dicominfo projects empty IS values as empty numeric arrays.
                String::new()
            } else if vr == *b"IS" && matches!(value.as_ref(), Array::Numeric(_) | Array::Typed(_))
            {
                let n = number(value, name)?;
                if n < f64::from(i32::MIN) || n > f64::from(i32::MAX) {
                    return Err(fail(name, "is outside signed 32-bit IS range"));
                }
                format!("{n:.0}")
            } else if vr == *b"PN" {
                match value.as_ref() {
                    Array::Struct(people) => meta::person_names(people, name, cancel)?,
                    _ => text(value, name, cancel)?,
                }
            } else {
                text(value, name, cancel)?
            };
            validate_text(&text, vr, name)?;
            if tag == 0x0028_0004 && text != photometric {
                return Err(fail(name, &format!("contradicts {photometric} pixels")));
            }
            if tag == 0x0008_0016 && text != SC {
                return Err(format!(
                    "dicomwrite: unsupported SOPClassUID '{text}'; only Secondary Capture Image Storage ({SC}) is supported"
                ));
            }
            fields.push(Attribute::text(tag, vr, &text));
        }
    }
    let unicode = copied.as_ref().map_or_else(
        || fields.iter().any(|f| !f.value.is_ascii()),
        |walk| walk.unicode,
    );
    if let Some(charset) = fields.iter().find(|f| f.tag == 0x0008_0005) {
        match charset.value.as_slice() {
            b"ISO_IR 192" => {},
            b""|b"ISO_IR 6" if !unicode => {},
            _ => return Err("dicomwrite: SpecificCharacterSet supports only ASCII/ISO_IR 6 or ISO_IR 192; UTF-8 text requires ISO_IR 192".into()),
        }
    } else if unicode {
        put_default(&mut fields, 0x0008_0005, "ISO_IR 192")?;
    }
    for (tag, value) in derived.into_iter().chain(rgb.then_some((0x0028_0006, 0))) {
        fields.push(Attribute {
            tag,
            vr: *b"US",
            value: u16_bytes(syntax, value).to_vec(),
        });
    }
    if frames > 1 {
        fields.push(Attribute::text(
            0x0028_0008,
            vr(0x0028_0008)?,
            &frames.to_string(),
        ));
    }
    // "Copy" generates no missing values (MathWorks CreateMode); "Create" does.
    let defaults: &[(u32, &str)] = if options.copy {
        &[]
    } else {
        &[
            (0x0008_0016, SC),
            (0x0028_0004, photometric),
            (0x0008_0060, "OT"),
            (0x0008_0064, "WSD"),
            (0x0010_0010, ""),
            (0x0010_0020, ""),
            (0x0010_0030, ""),
            (0x0010_0040, ""),
            (0x0008_0020, ""),
            (0x0008_0030, ""),
            (0x0008_0050, ""),
            (0x0008_0090, ""),
            (0x0020_0010, ""),
            (0x0020_0011, ""),
            (0x0020_0013, ""),
            (0x0020_0020, ""),
        ]
    };
    for &(tag, value) in defaults {
        put_default(&mut fields, tag, value)?;
    }
    for tag in [0x0008_0018, 0x0020_000D, 0x0020_000E] {
        if !options.copy && !fields.iter().any(|f| f.tag == tag) {
            let uid = format!("2.25.{}", uuid::Uuid::new_v4().as_u128());
            put_default(&mut fields, tag, &uid)?;
        }
    }
    fields.sort_unstable_by_key(|f| f.tag);
    // File meta storage UIDs follow the dataset; a copied struct may carry them
    // only in its (regenerated) file meta fields.
    let mut storage = [Vec::new(), Vec::new()];
    for (slot, (tag, name)) in storage.iter_mut().zip([
        (0x0008_0016, "SOPClassUID"),
        (0x0008_0018, "SOPInstanceUID"),
    ]) {
        let fallback = copied.as_mut().and_then(|walk| {
            if tag == 0x0008_0016 {
                walk.media_class.take()
            } else {
                walk.media_instance.take()
            }
        });
        *slot = match fields.iter().find(|f| f.tag == tag) {
            Some(field) => field.value.clone(),
            None => fallback.map(String::into_bytes).ok_or_else(|| {
                format!("dicomwrite: missing {name}; copy mode generates no values, so the metadata must supply {name} or MediaStorage{name}")
            })?,
        };
        if slot.is_empty()
            || slot.len() > 64
            || !slot.iter().all(|c| c.is_ascii_digit() || *c == b'.')
        {
            return Err(fail(name, "must be one UID for the file meta information"));
        }
    }
    let [class, instance] = storage;
    let mut meta = vec![
        Attribute {
            tag: 0x0002_0001,
            vr: *b"OB",
            value: vec![0, 1],
        },
        Attribute {
            tag: 0x0002_0002,
            vr: *b"UI",
            value: class,
        },
        Attribute {
            tag: 0x0002_0003,
            vr: *b"UI",
            value: instance,
        },
        Attribute::text(0x0002_0010, *b"UI", syntax_uid(syntax)),
        Attribute::text(0x0002_0012, *b"UI", IMPLEMENTATION),
        Attribute::text(0x0002_0013, *b"SH", "MATRUST_7A4"),
    ];
    let meta_len = encoded_fields_len(&meta, TransferSyntax::ExplicitLittle)?;
    meta.insert(
        0,
        Attribute {
            tag: 0x0002_0000,
            vr: *b"UL",
            value: u32::try_from(meta_len)
                .map_err(|_| "dicomwrite: meta length overflow")?
                .to_le_bytes()
                .to_vec(),
        },
    );
    // Tags above PixelData (7FE0,0010) are written after the pixels.
    let trailer = fields.split_off(fields.partition_point(|f| f.tag < 0x7FE0_0010));
    let trailer_len = encoded_fields_len(&trailer, syntax)?;
    let metadata_len = 132usize
        .checked_add(encoded_fields_len(&meta, TransferSyntax::ExplicitLittle)?)
        .and_then(|n| n.checked_add(encoded_fields_len(&fields, syntax).ok()?))
        .ok_or("dicomwrite: encoded metadata length overflow")?;
    // Copy mode: the walk capped the copied attributes; allow the generated ones on top.
    let cap = if options.copy {
        meta::TOTAL_CAP + META_CAP
    } else {
        META_CAP
    };
    if metadata_len.saturating_add(trailer_len) > cap {
        return Err("dicomwrite: serialized metadata exceeds budget".into());
    }
    let pixel_vr = if bits == 8 && syntax != TransferSyntax::ImplicitLittle {
        *b"OB"
    } else {
        *b"OW"
    };
    let encoded_len = metadata_len
        .checked_add(header_size(syntax, pixel_vr))
        .and_then(|n| n.checked_add(shape.payload))
        .and_then(|n| n.checked_add(trailer_len))
        .ok_or("dicomwrite: encoded file length overflow")?;
    if encoded_len > FILE_CAP {
        return Err("dicomwrite: encoded file exceeds 256 MiB".into());
    }
    Ok(Plan {
        pixels,
        shape,
        syntax,
        fields,
        trailer,
        meta,
        pixel_vr,
        encoded_len,
    })
}
fn encoded_fields_len(fields: &[Attribute], s: TransferSyntax) -> Result<usize> {
    fields.iter().try_fold(0usize, |n, a| {
        n.checked_add(header_size(s, a.vr))
            .and_then(|n| n.checked_add(a.length()))
            .ok_or_else(|| "dicomwrite: encoded length overflow".into())
    })
}
/// Serialize once into an admitted exact-capacity buffer; caller publishes atomically.
pub(crate) fn encode(plan: &Plan<'_>, cancel: &AtomicBool) -> Result<Vec<u8>> {
    encode_checked(plan, || crate::dsp::check_cancel(cancel))
}
fn encode_checked(plan: &Plan<'_>, mut checkpoint: impl FnMut() -> Result<()>) -> Result<Vec<u8>> {
    checkpoint()?;
    let mut out = Vec::new();
    out.try_reserve_exact(plan.encoded_len)
        .map_err(|_| "dicomwrite: unable to allocate serialized output")?;
    out.resize(128, 0);
    out.extend_from_slice(b"DICM");
    for a in &plan.meta {
        checkpoint()?;
        emit(&mut out, a, TransferSyntax::ExplicitLittle)?;
    }
    for a in &plan.fields {
        checkpoint()?;
        emit(&mut out, a, plan.syntax)?;
    }
    header(
        &mut out,
        0x7FE0_0010,
        plan.pixel_vr,
        plan.shape.payload,
        plan.syntax,
    )?;
    let little = plan.syntax.little();
    let shape = &plan.shape;
    // Only 16-bit cells follow the transfer syntax byte order (PS3.5 8.2).
    match plan.pixels.data() {
        TypedData::Uint8(v) => put_pixels(&mut out, v.as_slice(), shape, &mut checkpoint, |v| [v])?,
        TypedData::Int8(v) => put_pixels(
            &mut out,
            v.as_slice(),
            shape,
            &mut checkpoint,
            i8::to_le_bytes,
        )?,
        TypedData::Uint16(v) => {
            let bytes: fn(u16) -> [u8; 2] = if little {
                u16::to_le_bytes
            } else {
                u16::to_be_bytes
            };
            put_pixels(&mut out, v.as_slice(), shape, &mut checkpoint, bytes)?;
        }
        TypedData::Int16(v) => {
            let bytes: fn(i16) -> [u8; 2] = if little {
                i16::to_le_bytes
            } else {
                i16::to_be_bytes
            };
            put_pixels(&mut out, v.as_slice(), shape, &mut checkpoint, bytes)?;
        }
        _ => return Err("dicomwrite: unsupported pixel storage".into()),
    }
    if !plan.pixels.bytes().is_multiple_of(2) {
        out.push(0);
    }
    for a in &plan.trailer {
        checkpoint()?;
        emit(&mut out, a, plan.syntax)?;
    }
    if out.len() != plan.encoded_len {
        return Err("dicomwrite: encoded length mismatch".into());
    }
    checkpoint()?;
    Ok(out)
}
/// Native PS3.5 8.2 order: frames concatenated without padding; inside a frame
/// rows top to bottom, pixels left to right, the samples of one pixel adjacent
/// (PlanarConfiguration 0). Two's complement cells keep their bit pattern.
/// Polls at every frame start and every 4096 pixels inside a frame.
fn put_pixels<T: Copy, const N: usize>(
    out: &mut Vec<u8>,
    data: &[T],
    shape: &Shape,
    checkpoint: &mut impl FnMut() -> Result<()>,
    bytes: impl Fn(T) -> [u8; N],
) -> Result<()> {
    // Admission proved R*C*S*F == data.len(): no index below can overflow.
    let plane = shape.rows * shape.cols;
    for frame in 0..shape.frames {
        checkpoint()?;
        let first = frame * shape.samples * plane;
        for row in 0..shape.rows {
            for col in 0..shape.cols {
                let pixel = row * shape.cols + col;
                if pixel != 0 && pixel.is_multiple_of(4096) {
                    checkpoint()?;
                }
                let cell = first + col * shape.rows + row;
                for sample in 0..shape.samples {
                    let value = data
                        .get(cell + sample * plane)
                        .ok_or("dicomwrite: missing pixel")?;
                    out.extend_from_slice(&bytes(*value));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oversized_admission_without_pixel_allocation() {
        assert!(
            pixel_size(65535, 65535, 1, 1, 2, usize::MAX)
                .unwrap_err()
                .contains("256 MiB")
        );
        assert!(
            pixel_size(65536, 1, 1, 1, 1, usize::MAX)
                .unwrap_err()
                .contains("65535")
        );
        assert!(pixel_size(2, 3, 1, 1, 2, 5).is_err());
        // Samples and frames enter the same checked product, still unallocated.
        assert!(
            pixel_size(4096, 4096, 3, 6, 1, usize::MAX)
                .unwrap_err()
                .contains("256 MiB")
        );
        assert!(
            pixel_size(65535, 65535, 3, usize::MAX, 2, usize::MAX)
                .unwrap_err()
                .contains("overflow")
        );
        assert!(
            pixel_size(1, 1, 1, 0, 1, 100)
                .unwrap_err()
                .contains("at least one frame")
        );
        assert!(pixel_size(2, 3, 3, 2, 1, 35).unwrap_err().contains("limit"));
        assert_eq!(pixel_size(2, 3, 3, 2, 1, 36), Ok(36));
        assert_eq!(pixel_size(2, 3, 3, 2, 2, 36), Ok(72));
        assert_eq!(pixel_size(1, 1, 3, 1, 1, 100), Ok(4));
    }
    /// Checkpoints of one full encode; every one of them must abort cleanly.
    fn polls(pixels: &TypedArray) -> usize {
        let cancel = AtomicBool::new(false);
        let plan = prepare(
            pixels,
            None,
            &Options::create(TransferSyntax::ExplicitBig),
            1024 * 1024,
            100_000,
            &cancel,
        )
        .unwrap();
        let mut total = 0;
        encode_checked(&plan, || {
            total += 1;
            Ok(())
        })
        .unwrap();
        for stop in 1..=total {
            let mut calls = 0;
            let result = encode_checked(&plan, || {
                calls += 1;
                if calls == stop {
                    Err("Calculation cancelled".into())
                } else {
                    Ok(())
                }
            });
            assert_eq!(result.unwrap_err(), "Calculation cancelled");
            assert_eq!(calls, stop);
        }
        total
    }
    #[test]
    fn cancellation_between_frames_and_inside_frames() {
        let frames = |f: usize, class| {
            TypedArray::from_ints(class, 2, 3 * f, (0..6 * f as i128).map(|v| v - 3))
                .unwrap()
                .with_dims(&[2, 3, 1, f])
                .unwrap()
        };
        let one = polls(&frames(1, NumericClass::Int16));
        // One poll per frame start, plus the NumberOfFrames element itself.
        assert_eq!(polls(&frames(3, NumericClass::Int16)), one + 2 + 1);
        assert_eq!(polls(&frames(5, NumericClass::Int8)), one + 4 + 1);
        let rgb = TypedArray::from_ints(NumericClass::Uint16, 2, 27, 0..54)
            .unwrap()
            .with_dims(&[2, 3, 3, 3])
            .unwrap();
        // RGB adds PlanarConfiguration; samples do not add polls.
        assert_eq!(polls(&rgb), one + 2 + 1 + 1);
        // Inside one frame: pixels 4096 and 8192 of 8193.
        let wide = TypedArray::from_ints(NumericClass::Uint8, 1, 8193, (0..8193).map(|v| v % 251))
            .unwrap();
        assert_eq!(polls(&wide), one + 2);
    }
    /// Copy mode: a tag above PixelData is serialized after the pixels, and
    /// every checkpoint (also the trailer's) aborts without a result.
    #[test]
    fn copy_mode_trailer_follows_pixels_and_aborts_at_every_checkpoint() {
        use std::sync::Arc;
        let cancel = AtomicBool::new(false);
        let text = |s: &str| {
            Arc::new(Array::Character(
                crate::CharacterRow::from_units(s.encode_utf16().collect(), &cancel).unwrap(),
            ))
        };
        let item = StructArray::from_slots(
            1,
            1,
            vec!["Item_1".into()],
            vec![Arc::new(Array::Struct(
                StructArray::scalar_empty(&cancel).unwrap(),
            ))],
            &cancel,
        )
        .unwrap();
        let info = StructArray::from_slots(
            1,
            1,
            vec![
                "DigitalSignaturesSequence".into(),
                "SOPInstanceUID".into(),
                "SOPClassUID".into(),
            ],
            vec![Arc::new(Array::Struct(item)), text("2.25.5"), text(SC)],
            &cancel,
        )
        .unwrap();
        let pixels =
            TypedArray::from_ints(NumericClass::Uint8, 1, 3, [1, 2, 3].into_iter()).unwrap();
        let options = Options {
            copy: true,
            ..Options::create(TransferSyntax::ExplicitLittle)
        };
        let plan = prepare(&pixels, Some(&info), &options, 1024 * 1024, 100, &cancel).unwrap();
        assert_eq!(plan.trailer.len(), 1);
        let mut total = 0;
        let bytes = encode_checked(&plan, || {
            total += 1;
            Ok(())
        })
        .unwrap();
        // Odd pixel bytes are padded before the trailer; the item has a defined length.
        assert!(bytes.ends_with(&[
            1, 2, 3, 0, 0xfa, 0xff, 0xfa, 0xff, b'S', b'Q', 0, 0, 8, 0, 0, 0, 0xfe, 0xff, 0, 0xe0,
            0, 0, 0, 0
        ]));
        for stop in 1..=total {
            let mut calls = 0;
            let result = encode_checked(&plan, || {
                calls += 1;
                if calls == stop {
                    Err("Calculation cancelled".into())
                } else {
                    Ok(())
                }
            });
            assert_eq!(result.unwrap_err(), "Calculation cancelled");
        }
        // Create mode drops the trailer; copy mode still needs a struct.
        let create = Options::create(TransferSyntax::ExplicitLittle);
        let created = prepare(&pixels, Some(&info), &create, 1024 * 1024, 100, &cancel).unwrap();
        assert!(created.trailer.is_empty());
        assert!(prepare(&pixels, None, &options, 1024 * 1024, 100, &cancel).is_err());
        // A raised flag stops before the metadata walk.
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(prepare(&pixels, Some(&info), &options, 1024 * 1024, 100, &cancel).is_err());
    }
    #[test]
    fn cancellation_after_encoding_before_publication() {
        let cancel = AtomicBool::new(false);
        let pixels = TypedArray::from_ints(
            NumericClass::Uint16,
            2,
            3,
            [0, 32768, 1, 42, 65535, 256].into_iter(),
        )
        .unwrap();
        let plan = prepare(
            &pixels,
            None,
            &Options::create(TransferSyntax::ExplicitLittle),
            1024 * 1024,
            100,
            &cancel,
        )
        .unwrap();
        let bytes = encode(&plan, &cancel).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.dcm");
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(encode(&plan, &cancel).is_err());
        for exists in [false, true] {
            if exists {
                std::fs::write(&path, b"sentinel").unwrap();
            }
            assert!(crate::files::atomic_write_with_cancel(&path, &bytes, &cancel).is_err());
            if exists {
                assert_eq!(std::fs::read(&path).unwrap(), b"sentinel");
            } else {
                assert!(!path.exists());
            }
            assert_eq!(
                std::fs::read_dir(dir.path()).unwrap().count(),
                usize::from(exists)
            );
        }
    }
}
