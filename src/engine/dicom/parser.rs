//! Own structural Part 10 reader. Values are checked backing ranges, not copies.
//! Independent of Engine/UI; reusable envelope and dataset stages use Read + Seek.
use super::custom_dict::Dictionary;
use crate::Result;
use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::AtomicBool;

pub(crate) const MAX_DEPTH: usize = 32;
pub(crate) const MAX_NODES: usize = 10_000;
pub(crate) const MAX_VALUE: u64 = 1024 * 1024;
pub(crate) const MAX_METADATA_BUDGET: usize = 16 * 1024 * 1024;
pub(crate) const SPECIFIC_CHARACTER_SET: u32 = 0x0008_0005;
const UNDEFINED: u32 = u32::MAX;
const ITEM: u32 = 0xFFFE_E000;
const ITEM_END: u32 = 0xFFFE_E00D;
const SEQUENCE_END: u32 = 0xFFFE_E0DD;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TransferSyntax {
    ImplicitLittle,
    ExplicitLittle,
    ExplicitBig,
}
impl TransferSyntax {
    pub(crate) fn little(self) -> bool {
        self != Self::ExplicitBig
    }
    fn explicit(self) -> bool {
        self != Self::ImplicitLittle
    }
    pub(crate) fn u16(self, b: [u8; 2]) -> u16 {
        if self.little() {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        }
    }
    pub(crate) fn u32(self, b: [u8; 4]) -> u32 {
        if self.little() {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        }
    }
    pub(crate) fn u64(self, b: [u8; 8]) -> u64 {
        if self.little() {
            u64::from_le_bytes(b)
        } else {
            u64::from_be_bytes(b)
        }
    }
}

/// Offset domain is explicit: meta uses Original, prepared datasets may use
/// Inflated. Every range is checked against that backing and its enclosing item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Backing {
    Original,
    Inflated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ValueRange {
    pub(crate) backing: Backing,
    pub(crate) offset: u64,
    pub(crate) length: u64,
}
/// One checked encapsulated fragment item. `item_offset` is the offset of its
/// Item TAG relative to the first fragment item tag (the PS3.5 A.4 BOT domain);
/// `value` is the payload range, which structural parsing never reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Fragment {
    pub(crate) item_offset: u64,
    pub(crate) value: ValueRange,
}
#[derive(Debug)]
pub(crate) enum PixelLocation {
    Native {
        value: ValueRange,
        vr: [u8; 2],
    },
    /// PS3.5 A.4 encapsulation: Basic Offset Table item value (possibly empty)
    /// plus every fragment in stream order. Codec-neutral; frame mapping and
    /// payload decoding belong to the codec (RLE now, JPEG later).
    Encapsulated {
        offset_table: ValueRange,
        fragments: Vec<Fragment>,
    },
}
/// Pixel transport named by the transfer syntax, separate from dataset VR/endian.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PixelEncoding {
    Native,
    /// RLE Lossless 1.2.840.10008.1.2.5 (explicit LE dataset, encapsulated pixels).
    Rle,
    /// JPEG Baseline Process 1, 1.2.840.10008.1.2.4.50 (explicit LE dataset,
    /// encapsulated pixels; a frame may span several fragments).
    Jpeg,
    /// Huffman SOF3, Process 14 (.57) or Selection Value 1 (.70).
    JpegLossless {
        selection_one: bool,
    },
    /// JPEG 2000 Part 1 raw codestream (.90 reversible only, .91 either).
    Jpeg2000 {
        lossless: bool,
    },
}
#[derive(Debug)]
pub(crate) enum ElementValue {
    Primitive(ValueRange),
    Sequence(Vec<Dataset>),
    /// Undefined-length encapsulated pixel value; see Dataset::pixels.
    Encapsulated,
}
#[derive(Debug)]
pub(crate) struct Element {
    pub(crate) tag: u32,
    pub(crate) vr: [u8; 2],
    pub(crate) value: ElementValue,
}
#[derive(Debug, Default)]
pub(crate) struct Dataset {
    pub(crate) elements: Vec<Element>,
    pub(crate) pixels: Option<PixelLocation>,
}
#[derive(Debug)]
pub(crate) struct ParsedFile {
    pub(crate) meta: Dataset,
    pub(crate) dataset: Dataset,
    pub(crate) syntax: TransferSyntax,
    pub(crate) encoding: PixelEncoding,
    pub(crate) file_size: u64,
    pub(crate) dataset_size: u64,
    pub(crate) dataset_backing: Backing,
    pub(crate) remaining_budget: usize,
}
/// Validated uncompressed Part 10 envelope; no synthetic header or UID patching.
pub(crate) struct Envelope {
    pub(crate) meta: Dataset,
    pub(crate) syntax: TransferSyntax,
    pub(crate) deflated: bool,
    pub(crate) encoding: PixelEncoding,
    pub(crate) file_size: u64,
    pub(crate) dataset_start: u64,
    pub(crate) metadata_cost: usize,
    nodes: usize,
}
struct Header {
    tag: u32,
    vr: [u8; 2],
    length: u32,
}
struct Parser<'a, R> {
    reader: &'a mut R,
    backing: Backing,
    position: u64,
    file_size: u64,
    nodes: usize,
    budget: usize,
    encapsulated: bool,
    cancel: &'a AtomicBool,
    dictionary: &'a Dictionary,
}

fn io_error(_: std::io::Error) -> String {
    "dicominfo: truncated or unreadable DICOM file".into()
}
fn length_error() -> String {
    "dicominfo: invalid length outside the enclosing DICOM value/file".into()
}

pub(crate) fn is_pixel(tag: u32) -> bool {
    matches!(tag, 0x7FE0_0010 | 0x7FE0_0008 | 0x7FE0_0009)
}

#[cfg(test)]
pub(crate) fn envelope<R: Read + Seek>(
    reader: &mut R,
    file_size: u64,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Envelope> {
    envelope_with_dictionary(reader, file_size, budget, cancel, &Dictionary::Factory)
}

pub(crate) fn envelope_with_dictionary<R: Read + Seek>(
    reader: &mut R,
    file_size: u64,
    budget: usize,
    cancel: &AtomicBool,
    dictionary: &Dictionary,
) -> Result<Envelope> {
    let mut p = Parser {
        reader,
        backing: Backing::Original,
        position: 0,
        file_size,
        nodes: 0,
        budget: budget.min(MAX_METADATA_BUDGET),
        encapsulated: false,
        cancel,
        dictionary,
    };
    if file_size < 132 {
        return Err("dicominfo: Part 10 preamble and DICM marker required; raw datasets are not supported yet".into());
    }
    p.seek(128)?;
    if p.read::<4>(file_size)? != *b"DICM" {
        return Err(
            "dicominfo: Part 10 DICM marker required; raw datasets are not supported yet".into(),
        );
    }
    let syntax = TransferSyntax::ExplicitLittle;
    let first = p.header(syntax, file_size)?;
    if first.tag != 0x0002_0000 || first.vr != *b"UL" || first.length != 4 {
        return Err("dicominfo: invalid Part 10 meta group length header".into());
    }
    let first_range = ValueRange {
        backing: Backing::Original,
        offset: p.position,
        length: 4,
    };
    let meta_length = u32::from_le_bytes(p.read::<4>(file_size)?) as u64;
    let end = p
        .end(meta_length, file_size)
        .map_err(|_| "dicominfo: invalid meta group length")?;
    let mut meta = Dataset::default();
    p.charge_value(first.tag, 4)?;
    meta.elements
        .try_reserve(1)
        .map_err(|_| "dicominfo: unable to allocate meta descriptors")?;
    meta.elements.push(Element {
        tag: first.tag,
        vr: first.vr,
        value: ElementValue::Primitive(first_range),
    });
    let mut previous = first.tag;
    while p.position < end {
        let h = p
            .header(syntax, end)
            .map_err(|error| format!("dicominfo: meta {error}"))?;
        if h.tag >> 16 != 2 || h.tag <= previous || h.length == UNDEFINED || h.vr == *b"SQ" {
            return Err("dicominfo: invalid explicit little-endian meta group".into());
        }
        previous = h.tag;
        let value_end = p.end(h.length as u64, end)?;
        p.charge_value(h.tag, h.length as u64)?;
        let value = ElementValue::Primitive(ValueRange {
            backing: Backing::Original,
            offset: p.position,
            length: h.length as u64,
        });
        meta.elements
            .try_reserve(1)
            .map_err(|_| "dicominfo: unable to allocate meta descriptors")?;
        meta.elements.push(Element {
            tag: h.tag,
            vr: h.vr,
            value,
        });
        p.seek(value_end)?;
    }
    if let Some(version) = meta.elements.iter().find(|e| e.tag == 0x0002_0001) {
        let ElementValue::Primitive(range) = version.value else {
            unreachable!()
        };
        if range.length != 2 || read_value(p.reader, range, Backing::Original, cancel)?[1] & 1 == 0
        {
            return Err("dicominfo: unsupported Part 10 meta information version".into());
        }
    }
    let transfer = meta
        .elements
        .iter()
        .find(|e| e.tag == 0x0002_0010)
        .ok_or("dicominfo: Part 10 meta group is missing TransferSyntaxUID")?;
    if transfer.vr != *b"UI" {
        return Err("dicominfo: invalid TransferSyntaxUID VR in meta group".into());
    }
    let ElementValue::Primitive(range) = transfer.value else {
        unreachable!()
    };
    if range.length > 64 {
        return Err("dicominfo: invalid TransferSyntaxUID length".into());
    }
    let uid = read_value(p.reader, range, Backing::Original, cancel)?;
    let uid = std::str::from_utf8(&uid)
        .map_err(|_| "dicominfo: invalid TransferSyntaxUID text")?
        .trim_end_matches('\0');
    if uid.is_empty() || !uid.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return Err("dicominfo: invalid TransferSyntaxUID text".into());
    }
    let syntax = match uid {
        "1.2.840.10008.1.2" => TransferSyntax::ImplicitLittle,
        "1.2.840.10008.1.2.1"
        | "1.2.840.10008.1.2.1.99"
        | "1.2.840.10008.1.2.5"
        | "1.2.840.10008.1.2.4.50"
        | "1.2.840.10008.1.2.4.57"
        | "1.2.840.10008.1.2.4.70"
        | "1.2.840.10008.1.2.4.90"
        | "1.2.840.10008.1.2.4.91" => TransferSyntax::ExplicitLittle,
        "1.2.840.10008.1.2.2" => TransferSyntax::ExplicitBig,
        _ => {
            return Err(format!(
                "dicominfo: transfer syntax {uid} is not supported yet (supports native implicit LE, explicit LE, explicit BE, deflated explicit LE, RLE Lossless, JPEG Baseline Process 1, JPEG Lossless Process 14 and JPEG 2000 Part 1)"
            ));
        }
    };
    Ok(Envelope {
        meta,
        syntax,
        deflated: uid == "1.2.840.10008.1.2.1.99",
        encoding: match uid {
            "1.2.840.10008.1.2.5" => PixelEncoding::Rle,
            "1.2.840.10008.1.2.4.50" => PixelEncoding::Jpeg,
            "1.2.840.10008.1.2.4.57" => PixelEncoding::JpegLossless {
                selection_one: false,
            },
            "1.2.840.10008.1.2.4.70" => PixelEncoding::JpegLossless {
                selection_one: true,
            },
            "1.2.840.10008.1.2.4.90" => PixelEncoding::Jpeg2000 { lossless: true },
            "1.2.840.10008.1.2.4.91" => PixelEncoding::Jpeg2000 { lossless: false },
            _ => PixelEncoding::Native,
        },
        file_size,
        dataset_start: end,
        metadata_cost: budget.min(MAX_METADATA_BUDGET) - p.budget,
        nodes: p.nodes,
    })
}

/// Reuse the structural dataset parser with its real reader, length and domain.
/// `budget` includes envelope descriptors/projection, but excludes retained input.
#[cfg(test)]
pub(crate) fn dataset<R: Read + Seek>(
    reader: &mut R,
    envelope: Envelope,
    backing: Backing,
    backing_size: u64,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<ParsedFile> {
    dataset_with_dictionary(
        reader,
        envelope,
        backing,
        backing_size,
        budget,
        cancel,
        &Dictionary::Factory,
    )
}

pub(crate) fn dataset_with_dictionary<R: Read + Seek>(
    reader: &mut R,
    envelope: Envelope,
    backing: Backing,
    backing_size: u64,
    budget: usize,
    cancel: &AtomicBool,
    dictionary: &Dictionary,
) -> Result<ParsedFile> {
    let budget = budget
        .min(MAX_METADATA_BUDGET)
        .checked_sub(envelope.metadata_cost)
        .ok_or("dicominfo: metadata exceeds the memory/output budget")?;
    let mut p = Parser {
        reader,
        backing,
        position: 0,
        file_size: backing_size,
        nodes: envelope.nodes,
        budget,
        encapsulated: envelope.encoding != PixelEncoding::Native,
        cancel,
        dictionary,
    };
    p.seek(if backing == Backing::Original {
        envelope.dataset_start
    } else {
        0
    })?;
    let mut dataset = p.dataset(backing_size, false, 0, envelope.syntax)?;
    if envelope.syntax == TransferSyntax::ImplicitLittle {
        resolve_implicit_vrs(p.reader, &mut dataset, 0, cancel, dictionary)?;
    }
    Ok(ParsedFile {
        meta: envelope.meta,
        dataset,
        syntax: envelope.syntax,
        encoding: envelope.encoding,
        file_size: envelope.file_size,
        dataset_size: backing_size,
        dataset_backing: backing,
        remaining_budget: p.budget,
    })
}

pub(crate) fn mixed_lut_descriptor(tag: u32, vr: [u8; 2]) -> bool {
    // PS3.3 C.11.1.1.1 and C.7.6.3.1.5: SS applies only to cell 2.
    vr == *b"SS" && matches!(tag, 0x0028_3002 | 0x0028_1101..=0x0028_1103)
}

/// Resolve after parsing so an ancestor's PixelRepresentation can follow a
/// sequence or an ambiguous tag in tag order. Only this explicit context value
/// is read, never the ambiguous payload; ranges and budget remain unchanged.
fn resolve_implicit_vrs<R: Read + Seek>(
    reader: &mut R,
    data: &mut Dataset,
    inherited: u16,
    cancel: &AtomicBool,
    dictionary: &Dictionary,
) -> Result<()> {
    crate::dsp::check_cancel(cancel)?;
    let mut representation = inherited;
    if let Some(element) = data.elements.iter().find(|e| e.tag == 0x0028_0103) {
        let ElementValue::Primitive(range) = element.value else {
            return Err("dicominfo: invalid PixelRepresentation".into());
        };
        // Empty attributes retain their existing metadata representation and
        // provide no signedness context, just like a missing attribute.
        if range.length != 0 {
            if range.length != 2 {
                return Err("dicominfo: invalid PixelRepresentation value length".into());
            }
            reader
                .seek(SeekFrom::Start(range.offset))
                .map_err(io_error)?;
            let mut bytes = [0; 2];
            reader.read_exact(&mut bytes).map_err(io_error)?;
            representation = u16::from_le_bytes(bytes);
        }
    }
    for element in &mut data.elements {
        crate::dsp::check_cancel(cancel)?;
        if let Some(entry) = dictionary.lookup(element.tag) {
            element.vr = entry.implicit_vr(element.tag, representation);
        }
        if let ElementValue::Sequence(items) = &mut element.value {
            for item in items {
                resolve_implicit_vrs(reader, item, representation, cancel, dictionary)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn parse<R: Read + Seek>(
    reader: &mut R,
    file_size: u64,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<ParsedFile> {
    let env = envelope(reader, file_size, budget, cancel)?;
    if env.deflated {
        return Err("dicominfo: deflated input requires prepared backing".into());
    }
    dataset(reader, env, Backing::Original, file_size, budget, cancel)
}

impl<R: Read + Seek> Parser<'_, R> {
    fn charge(&mut self, amount: usize) -> Result<()> {
        self.budget = self
            .budget
            .checked_sub(amount)
            .ok_or("dicominfo: metadata exceeds the memory/output budget")?;
        Ok(())
    }
    fn node(&mut self) -> Result<()> {
        crate::dsp::check_cancel(self.cancel)?;
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err("dicominfo: element/item count exceeds 10000".into());
        }
        self.charge(512)
    }
    fn charge_value(&mut self, tag: u32, length: u64) -> Result<()> {
        // Projection reads SpecificCharacterSet in every dataset it visits, so
        // it stays admitted even when a custom dictionary omits that tag.
        if (tag == SPECIFIC_CHARACTER_SET || self.dictionary.lookup(tag).is_some())
            && !is_pixel(tag)
        {
            if length > MAX_VALUE {
                return Err("dicominfo: metadata value exceeds the 1048576-byte limit".into());
            }
            // Read buffer + decoded text/numeric payload + container overhead.
            self.charge((length as usize).checked_mul(16).ok_or_else(length_error)?)?;
        }
        Ok(())
    }
    fn end(&self, length: u64, limit: u64) -> Result<u64> {
        self.position
            .checked_add(length)
            .filter(|end| *end <= limit && *end <= self.file_size)
            .ok_or_else(length_error)
    }
    fn read<const N: usize>(&mut self, limit: u64) -> Result<[u8; N]> {
        crate::dsp::check_cancel(self.cancel)?;
        let end = self
            .end(N as u64, limit)
            .map_err(|_| "dicominfo: truncated DICOM header/value")?;
        let mut b = [0; N];
        self.reader.read_exact(&mut b).map_err(io_error)?;
        self.position = end;
        Ok(b)
    }
    fn seek(&mut self, position: u64) -> Result<()> {
        if position > self.file_size {
            return Err(length_error());
        }
        self.reader
            .seek(SeekFrom::Start(position))
            .map_err(io_error)?;
        self.position = position;
        Ok(())
    }
    fn header(&mut self, syntax: TransferSyntax, end: u64) -> Result<Header> {
        self.node()?;
        let b = self.read::<8>(end)?;
        let tag = ((syntax.u16([b[0], b[1]]) as u32) << 16) | syntax.u16([b[2], b[3]]) as u32;
        if tag >> 16 == 0xFFFE {
            return Ok(Header {
                tag,
                vr: *b"UN",
                length: syntax.u32(b[4..8].try_into().unwrap()),
            });
        }
        let (vr, length) = if syntax.explicit() {
            let vr = [b[4], b[5]];
            let long = match &vr {
                b"OB" | b"OD" | b"OF" | b"OL" | b"OV" | b"OW" | b"SQ" | b"SV" | b"UC" | b"UN"
                | b"UR" | b"UT" | b"UV" => true,
                b"AE" | b"AS" | b"AT" | b"CS" | b"DA" | b"DS" | b"DT" | b"FD" | b"FL" | b"IS"
                | b"LO" | b"LT" | b"PN" | b"SH" | b"SL" | b"SS" | b"ST" | b"TM" | b"UI" | b"UL"
                | b"US" => false,
                _ => return Err("dicominfo: invalid or unsupported explicit VR".into()),
            };
            let length = if long {
                if b[6..8] != [0, 0] {
                    return Err("dicominfo: nonzero reserved explicit VR header bytes".into());
                }
                syntax.u32(self.read::<4>(end)?)
            } else {
                syntax.u16([b[6], b[7]]) as u32
            };
            if let Some(entry) = self.dictionary.lookup(tag)
                && !entry.vrs.contains(&vr)
                && vr != *b"UN"
            {
                return Err(
                    "dicominfo: explicit VR disagrees with the supported dictionary entry".into(),
                );
            }
            (vr, length)
        } else {
            (
                self.dictionary
                    .lookup(tag)
                    .map_or(*b"UN", |entry| entry.implicit_vr(tag, 0)),
                syntax.u32(b[4..8].try_into().unwrap()),
            )
        };
        if length != UNDEFINED && length % 2 != 0 {
            return Err("dicominfo: odd DICOM value length".into());
        }
        Ok(Header { tag, vr, length })
    }
    fn dataset(
        &mut self,
        end: u64,
        undefined: bool,
        depth: usize,
        syntax: TransferSyntax,
    ) -> Result<Dataset> {
        let mut data = Dataset::default();
        let mut previous = None;
        while self.position < end {
            let h = self.header(syntax, end)?;
            if h.tag >> 16 == 0xFFFE {
                if h.tag == ITEM_END && undefined && h.length == 0 {
                    return Ok(data);
                }
                return Err("dicominfo: mismatched or invalid item/sequence delimiter".into());
            }
            if h.tag >> 16 == 2 {
                return Err("dicominfo: group 0002 element outside the declared meta group".into());
            }
            if previous.is_some_and(|tag| tag >= h.tag) {
                return Err("dicominfo: duplicate or out-of-order dataset tag".into());
            }
            previous = Some(h.tag);
            let value = if h.vr == *b"SQ" {
                if depth >= MAX_DEPTH {
                    return Err("dicominfo: sequence nesting exceeds 32 levels".into());
                }
                let sequence_end = if h.length == UNDEFINED {
                    end
                } else {
                    self.end(h.length as u64, end)?
                };
                ElementValue::Sequence(self.sequence(
                    sequence_end,
                    h.length == UNDEFINED,
                    depth + 1,
                    syntax,
                )?)
            } else if h.length == UNDEFINED
                && self.encapsulated
                && h.tag == 0x7FE0_0010
                && h.vr == *b"OB"
            {
                // PS3.5 A.4: fragments are opaque codec bytes, never datasets.
                if data.pixels.is_some() {
                    return Err(
                        "dicominfo: multiple pixel payloads in one dataset are not supported yet"
                            .into(),
                    );
                }
                data.pixels = Some(self.encapsulated_pixels(end)?);
                ElementValue::Encapsulated
            } else if h.length == UNDEFINED && self.encapsulated && is_pixel(h.tag) {
                return Err("dicominfo: encapsulated PixelData (7FE0,0010) must use VR OB".into());
            } else {
                if h.length == UNDEFINED {
                    return Err(
                        "dicominfo: undefined length for non-SQ value is not supported yet".into(),
                    );
                }
                let value_end = self.end(h.length as u64, end)?;
                self.charge_value(h.tag, h.length as u64)?;
                let range = ValueRange {
                    backing: self.backing,
                    offset: self.position,
                    length: h.length as u64,
                };
                if is_pixel(h.tag) {
                    if data.pixels.is_some() {
                        return Err("dicominfo: multiple pixel payloads in one dataset are not supported yet".into());
                    }
                    data.pixels = Some(PixelLocation::Native {
                        value: range,
                        vr: h.vr,
                    });
                }
                self.seek(value_end)?;
                ElementValue::Primitive(range)
            };
            data.elements
                .try_reserve(1)
                .map_err(|_| "dicominfo: unable to allocate element descriptors")?;
            data.elements.push(Element {
                tag: h.tag,
                vr: h.vr,
                value,
            });
        }
        if undefined {
            return Err("dicominfo: unterminated undefined-length item".into());
        }
        Ok(data)
    }
    /// Walk PS3.5 A.4 items by their explicit lengths: BOT item, fragment items,
    /// then a zero-length Sequence Delimitation Item. Payloads are seeked over,
    /// so delimiter-like payload bytes are never interpreted. Every item is a
    /// counted, budgeted, cancellable node.
    fn encapsulated_pixels(&mut self, end: u64) -> Result<PixelLocation> {
        let (tag, length) = self.item_header(end)?;
        if tag != ITEM {
            return Err(
                "dicominfo: encapsulated PixelData must start with a Basic Offset Table item"
                    .into(),
            );
        }
        if length == UNDEFINED || length % 4 != 0 {
            return Err(
                "dicominfo: Basic Offset Table item length must be a defined multiple of 4".into(),
            );
        }
        let offset_table = ValueRange {
            backing: self.backing,
            offset: self.position,
            length: length as u64,
        };
        let table_end = self.end(length as u64, end).map_err(
            |_| "dicominfo: truncated Basic Offset Table item in encapsulated PixelData",
        )?;
        self.seek(table_end)?;
        let first = self.position;
        let mut fragments: Vec<Fragment> = Vec::new();
        while self.position < end {
            let item_start = self.position;
            let (tag, length) = self.item_header(end)?;
            if tag == SEQUENCE_END {
                if length != 0 {
                    return Err("dicominfo: encapsulated PixelData sequence delimiter must have zero length".into());
                }
                return Ok(PixelLocation::Encapsulated {
                    offset_table,
                    fragments,
                });
            }
            if tag != ITEM {
                return Err(
                    "dicominfo: mismatched or invalid item/delimiter in encapsulated PixelData"
                        .into(),
                );
            }
            if length == UNDEFINED {
                return Err("dicominfo: encapsulated fragment must have a defined length".into());
            }
            if length == 0 || length % 2 != 0 {
                return Err(
                    "dicominfo: encapsulated fragment length must be even and at least 2".into(),
                );
            }
            let value_end = self
                .end(length as u64, end)
                .map_err(|_| "dicominfo: truncated encapsulated fragment item")?;
            fragments
                .try_reserve(1)
                .map_err(|_| "dicominfo: unable to allocate fragment descriptors")?;
            fragments.push(Fragment {
                item_offset: item_start - first,
                value: ValueRange {
                    backing: self.backing,
                    offset: self.position,
                    length: length as u64,
                },
            });
            self.seek(value_end)?;
        }
        Err("dicominfo: encapsulated PixelData is missing its sequence delimiter".into())
    }
    fn item_header(&mut self, end: u64) -> Result<(u32, u32)> {
        self.node()?;
        crate::dsp::check_cancel(self.cancel)?;
        let b = self.read::<8>(end).map_err(|error| {
            if error.contains("cancelled") {
                error
            } else {
                "dicominfo: truncated item header in encapsulated PixelData".into()
            }
        })?;
        let tag = (u32::from(u16::from_le_bytes([b[0], b[1]])) << 16)
            | u32::from(u16::from_le_bytes([b[2], b[3]]));
        Ok((tag, u32::from_le_bytes(b[4..8].try_into().unwrap())))
    }
    fn sequence(
        &mut self,
        end: u64,
        undefined: bool,
        depth: usize,
        syntax: TransferSyntax,
    ) -> Result<Vec<Dataset>> {
        let mut items = Vec::new();
        while self.position < end {
            let h = self.header(syntax, end)?;
            if h.tag == SEQUENCE_END && undefined && h.length == 0 {
                return Ok(items);
            }
            if h.tag != ITEM {
                return Err("dicominfo: mismatched or invalid sequence/item delimiter".into());
            }
            if items.len() >= crate::struct_array::MAX_FIELDS {
                return Err("dicominfo: sequence exceeds 1024 items".into());
            }
            let item_end = if h.length == UNDEFINED {
                end
            } else {
                self.end(h.length as u64, end)?
            };
            let item = self.dataset(item_end, h.length == UNDEFINED, depth, syntax)?;
            items
                .try_reserve(1)
                .map_err(|_| "dicominfo: unable to allocate item descriptors")?;
            items.push(item);
        }
        if undefined {
            return Err("dicominfo: unterminated undefined-length sequence".into());
        }
        Ok(items)
    }
}

/// Only metadata values pass here. Pixel payloads stay behind file offsets.
pub(crate) fn read_value<R: Read + Seek>(
    reader: &mut R,
    range: ValueRange,
    backing: Backing,
    cancel: &AtomicBool,
) -> Result<Vec<u8>> {
    crate::dsp::check_cancel(cancel)?;
    if range.backing != backing {
        return Err("dicominfo: mismatched DICOM value backing".into());
    }
    if range.length > MAX_VALUE {
        return Err("dicominfo: metadata value exceeds the 1048576-byte limit".into());
    }
    let length = usize::try_from(range.length).map_err(|_| length_error())?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| "dicominfo: unable to allocate metadata value")?;
    bytes.resize(length, 0);
    reader
        .seek(SeekFrom::Start(range.offset))
        .map_err(io_error)?;
    reader.read_exact(&mut bytes).map_err(io_error)?;
    crate::dsp::check_cancel(cancel)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::atomic::Ordering;

    const BASELINE: &[u8] = include_bytes!("../../../tests/fixtures/dicom/baseline.dcm");

    struct Watched<'a> {
        input: Cursor<&'a [u8]>,
        forbidden: Option<std::ops::Range<u64>>,
        cancel: &'a AtomicBool,
        cancel_after: usize,
        reads: usize,
    }
    impl Read for Watched<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let start = self.input.position();
            let end = start + buffer.len() as u64;
            if let Some(range) = &self.forbidden {
                assert!(
                    end <= range.start || start >= range.end,
                    "pixel bytes must never be read"
                );
            }
            self.reads += 1;
            let n = self.input.read(buffer)?;
            if self.reads == self.cancel_after {
                self.cancel.store(true, Ordering::Relaxed);
            }
            Ok(n)
        }
    }
    impl Seek for Watched<'_> {
        fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
            self.input.seek(position)
        }
    }
    #[test]
    fn checked_native_pixel_location_without_payload_reads() {
        let cancel = AtomicBool::new(false);
        let offset = BASELINE.len() as u64 - 12;
        let mut reader = Watched {
            input: Cursor::new(BASELINE),
            forbidden: Some(offset..offset + 12),
            cancel: &cancel,
            cancel_after: usize::MAX,
            reads: 0,
        };
        let parsed = parse(
            &mut reader,
            BASELINE.len() as u64,
            MAX_METADATA_BUDGET,
            &cancel,
        )
        .unwrap();
        let Some(PixelLocation::Native { value, vr }) = parsed.dataset.pixels else {
            panic!("native pixel descriptor required")
        };
        assert_eq!(
            value,
            ValueRange {
                backing: Backing::Original,
                offset,
                length: 12
            }
        );
        assert_eq!(vr, *b"OW");
        assert_eq!(parsed.syntax, TransferSyntax::ExplicitLittle);
    }
    #[test]
    fn parsing_polls_cancellation_between_headers() {
        let cancel = AtomicBool::new(false);
        let mut reader = Watched {
            input: Cursor::new(BASELINE),
            forbidden: None,
            cancel: &cancel,
            cancel_after: 5,
            reads: 0,
        };
        let error = parse(
            &mut reader,
            BASELINE.len() as u64,
            MAX_METADATA_BUDGET,
            &cancel,
        )
        .unwrap_err();
        assert!(error.contains("cancelled"), "{error}");
        assert_eq!(reader.reads, 5);
    }
    #[test]
    fn newly_known_metadata_is_charged_and_cancels_during_parsing() {
        let meta_length = u32::from_le_bytes(BASELINE[140..144].try_into().unwrap()) as usize;
        let start = 144 + meta_length;
        let mut bytes = BASELINE[..start].to_vec();
        for group in (0x6000u16..0x6020).step_by(2) {
            bytes.extend_from_slice(&group.to_le_bytes());
            bytes.extend_from_slice(&0x3000u16.to_le_bytes());
            bytes.extend_from_slice(b"OW\0\0");
            bytes.extend_from_slice(&65536u32.to_le_bytes());
            bytes.resize(bytes.len() + 65536, 0);
        }
        let cancel = AtomicBool::new(false);
        let mut reader = Watched {
            input: Cursor::new(&bytes),
            forbidden: Some((start + 12) as u64..(start + 12 + 65536) as u64),
            cancel: &cancel,
            cancel_after: usize::MAX,
            reads: 0,
        };
        envelope(
            &mut reader,
            bytes.len() as u64,
            MAX_METADATA_BUDGET,
            &cancel,
        )
        .unwrap();
        let meta_reads = reader.reads;
        reader.reads = 0;
        let error = parse(
            &mut reader,
            bytes.len() as u64,
            MAX_METADATA_BUDGET,
            &cancel,
        )
        .unwrap_err();
        assert!(error.contains("memory/output budget"), "{error}");
        // Cancel after four new, previously unknown element headers. The
        // watched reader also forbids reading the first large value payload.
        reader.reads = 0;
        reader.cancel_after = meta_reads + 8;
        let error = parse(
            &mut reader,
            bytes.len() as u64,
            MAX_METADATA_BUDGET,
            &cancel,
        )
        .unwrap_err();
        assert!(error.contains("cancelled"), "{error}");
        assert_eq!(reader.reads, meta_reads + 8);
        assert!(reader.input.position() > start as u64);
    }
    #[test]
    fn inflated_parser_cancels_and_checks_range_domains() {
        let cancel = AtomicBool::new(false);
        let env = envelope(
            &mut Cursor::new(BASELINE),
            BASELINE.len() as u64,
            MAX_METADATA_BUDGET,
            &cancel,
        )
        .unwrap();
        let data = &BASELINE[env.dataset_start as usize..];
        let mut reader = Watched {
            input: Cursor::new(data),
            forbidden: None,
            cancel: &cancel,
            cancel_after: 5,
            reads: 0,
        };
        let error = dataset(
            &mut reader,
            env,
            Backing::Inflated,
            data.len() as u64,
            MAX_METADATA_BUDGET,
            &cancel,
        )
        .unwrap_err();
        assert!(error.contains("cancelled"), "{error}");
        assert_eq!(reader.reads, 5);
        cancel.store(false, Ordering::Relaxed);
        let range = ValueRange {
            backing: Backing::Inflated,
            offset: 0,
            length: 2,
        };
        assert!(
            read_value(
                &mut Cursor::new(BASELINE),
                range,
                Backing::Original,
                &cancel
            )
            .unwrap_err()
            .contains("backing")
        );
    }
    #[test]
    fn encapsulated_walker_never_reads_fragment_payloads_and_cancels() {
        const RLE: &[u8] =
            include_bytes!("../../../tests/fixtures/dicom/rle_delimiter_payload.dcm");
        let cancel = AtomicBool::new(false);
        let parsed = parse(
            &mut Cursor::new(RLE),
            RLE.len() as u64,
            MAX_METADATA_BUDGET,
            &cancel,
        )
        .unwrap();
        assert_eq!(parsed.encoding, PixelEncoding::Rle);
        let Some(PixelLocation::Encapsulated {
            offset_table,
            fragments,
        }) = &parsed.dataset.pixels
        else {
            panic!("encapsulated descriptor required")
        };
        assert_eq!(offset_table.length, 4);
        assert_eq!(fragments.len(), 1);
        assert_eq!(fragments[0].item_offset, 0);
        let payload = fragments[0].value;
        assert_eq!(payload.offset, offset_table.offset + 4 + 8);
        assert_eq!(payload.offset + payload.length + 8, RLE.len() as u64);
        let bytes = &RLE[payload.offset as usize..(payload.offset + payload.length) as usize];
        assert!(bytes.windows(4).any(|w| w == [0xFE, 0xFF, 0xDD, 0xE0]));
        assert!(bytes.windows(4).any(|w| w == [0xFE, 0xFF, 0x00, 0xE0]));
        let mut reader = Watched {
            input: Cursor::new(RLE),
            forbidden: Some(payload.offset..payload.offset + payload.length),
            cancel: &cancel,
            cancel_after: usize::MAX,
            reads: 0,
        };
        let guarded = parse(&mut reader, RLE.len() as u64, MAX_METADATA_BUDGET, &cancel).unwrap();
        assert!(matches!(
            guarded.dataset.pixels,
            Some(PixelLocation::Encapsulated { ref fragments, .. }) if fragments[0].value == payload
        ));
        // Cancel right after the fragment item header: the delimiter read polls.
        let total = reader.reads;
        let mut reader = Watched {
            input: Cursor::new(RLE),
            forbidden: Some(payload.offset..payload.offset + payload.length),
            cancel: &cancel,
            cancel_after: total - 1,
            reads: 0,
        };
        let error = parse(&mut reader, RLE.len() as u64, MAX_METADATA_BUDGET, &cancel).unwrap_err();
        assert!(error.contains("cancelled"), "{error}");
        assert_eq!(reader.reads, total - 1);
    }
    fn nested(depth: usize) -> Vec<u8> {
        let meta_length = u32::from_le_bytes(BASELINE[140..144].try_into().unwrap()) as usize;
        let mut data = BASELINE[..144 + meta_length].to_vec();
        for _ in 0..depth {
            data.extend_from_slice(&[8, 0, 0x10, 0x11, b'S', b'Q', 0, 0, 255, 255, 255, 255]);
            data.extend_from_slice(&[254, 255, 0, 224, 255, 255, 255, 255]);
        }
        for _ in 0..depth {
            data.extend_from_slice(&[254, 255, 13, 224, 0, 0, 0, 0]);
            data.extend_from_slice(&[254, 255, 221, 224, 0, 0, 0, 0]);
        }
        data
    }
    #[test]
    fn parser_accepts_32_sequence_levels_and_rejects_33() {
        for depth in [32, 33] {
            let input = nested(depth);
            let result = parse(
                &mut Cursor::new(&input),
                input.len() as u64,
                MAX_METADATA_BUDGET,
                &AtomicBool::new(false),
            );
            if depth == 32 {
                assert!(result.is_ok(), "{result:?}");
            } else {
                assert!(result.unwrap_err().contains("nesting"));
            }
        }
    }
    #[test]
    fn descriptor_count_and_budget_are_enforced() {
        let meta_length = u32::from_le_bytes(BASELINE[140..144].try_into().unwrap()) as usize;
        let mut input = BASELINE[..144 + meta_length].to_vec();
        for tag in 1..=MAX_NODES as u16 {
            input.extend_from_slice(&[0x11, 0]);
            input.extend_from_slice(&tag.to_le_bytes());
            input.extend_from_slice(b"OB\0\0\0\0\0\0");
        }
        let error = parse(
            &mut Cursor::new(&input),
            input.len() as u64,
            MAX_METADATA_BUDGET,
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(error.contains("count"), "{error}");
        let error = parse(
            &mut Cursor::new(BASELINE),
            BASELINE.len() as u64,
            128,
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(error.contains("budget"), "{error}");
    }
}

#[cfg(test)]
mod custom_dictionary_tests {
    use super::*;
    use std::{io::Cursor, path::Path};

    #[test]
    fn custom_private_vrs_and_replacement_are_used_during_structural_parse() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dicom");
        let cancel = AtomicBool::new(false);
        let dictionary = crate::engine::dicom::custom_dict::load(
            &root.join("custom-dict.txt"),
            32 * 1024 * 1024,
            &cancel,
        )
        .unwrap();
        for filename in ["custom_implicit.dcm", "custom_explicit.dcm"] {
            let bytes = std::fs::read(root.join(filename)).unwrap();
            let size = bytes.len() as u64;
            let mut reader = Cursor::new(bytes);
            let envelope = envelope_with_dictionary(
                &mut reader,
                size,
                MAX_METADATA_BUDGET,
                &cancel,
                &dictionary,
            )
            .unwrap();
            let parsed = dataset_with_dictionary(
                &mut reader,
                envelope,
                Backing::Original,
                size,
                MAX_METADATA_BUDGET,
                &cancel,
                &dictionary,
            )
            .unwrap();
            let private = parsed
                .dataset
                .elements
                .iter()
                .find(|e| e.tag == 0x0011_1001)
                .unwrap();
            assert_eq!(private.vr, *b"US");
            let ElementValue::Primitive(range) = private.value else {
                panic!("primitive private value")
            };
            let value = read_value(&mut reader, range, Backing::Original, &cancel).unwrap();
            assert_eq!(value, [1, 2]); // independently generated US 513
            assert!(dictionary.lookup(0x0002_0010).is_none()); // replaces factory naming/VRs
            assert!(parsed.meta.elements.iter().any(|e| e.tag == 0x0002_0010)); // envelope is still structural
        }
    }

    /// The fixture dictionary omits (0008,0005), yet projection always reads it:
    /// its value must be charged (16x) and capped exactly like a known tag.
    #[test]
    fn specific_character_set_is_admitted_when_the_dictionary_omits_it() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dicom");
        let cancel = AtomicBool::new(false);
        let dictionary = crate::engine::dicom::custom_dict::load(
            &root.join("custom-dict.txt"),
            32 * 1024 * 1024,
            &cancel,
        )
        .unwrap();
        assert!(dictionary.lookup(SPECIFIC_CHARACTER_SET).is_none());
        let base = std::fs::read(root.join("custom_implicit.dcm")).unwrap();
        let start = 144 + u32::from_le_bytes(base[140..144].try_into().unwrap()) as usize;
        let parse = |length: usize, budget: usize, dictionary: &Dictionary| {
            let mut bytes = base[..start].to_vec();
            bytes.extend_from_slice(&[8, 0, 5, 0]);
            bytes.extend_from_slice(&(length as u32).to_le_bytes());
            bytes.resize(bytes.len() + length, b' ');
            bytes.extend_from_slice(&base[start..]);
            let size = bytes.len() as u64;
            let mut reader = Cursor::new(bytes);
            let envelope =
                envelope_with_dictionary(&mut reader, size, budget, &cancel, dictionary)?;
            dataset_with_dictionary(
                &mut reader,
                envelope,
                Backing::Original,
                size,
                budget,
                &cancel,
                dictionary,
            )
            .map(|parsed| budget - parsed.remaining_budget)
        };
        let empty = parse(0, MAX_METADATA_BUDGET, &dictionary).unwrap();
        let used = parse(4096, MAX_METADATA_BUDGET, &dictionary).unwrap();
        assert_eq!(used - empty, 16 * 4096);
        assert_eq!(parse(4096, used, &dictionary).unwrap(), used);
        let error = parse(4096, used - 1, &dictionary).unwrap_err();
        assert!(error.contains("budget"), "{error}");
        let error = parse(MAX_VALUE as usize + 2, MAX_METADATA_BUDGET, &dictionary).unwrap_err();
        assert!(error.contains("1048576-byte limit"), "{error}");
        // The factory table knows the tag: identical charge, no double counting.
        let factory = Dictionary::Factory;
        assert_eq!(
            parse(4096, MAX_METADATA_BUDGET, &factory).unwrap()
                - parse(0, MAX_METADATA_BUDGET, &factory).unwrap(),
            16 * 4096
        );
    }
}
