//! Public PS3.6 dictionary; provenance and regeneration in tool/refs/dicom/SOURCES.md.
//! Immutable sorted factory tables; no byte-based VR inference.

#[derive(Debug)]
pub(crate) struct Entry {
    pub(crate) value: u32,
    pub(crate) mask: u32,
    pub(crate) keyword: &'static str,
    pub(crate) vrs: &'static [[u8; 2]],
    // Retained for the next dictionary API slice; validated by integration tests.
    #[allow(dead_code)]
    pub(crate) vm: &'static str,
    #[allow(dead_code)]
    pub(crate) retired: bool,
    #[allow(dead_code)]
    pub(crate) name: &'static str,
}

include!("dictionary_data.rs");

pub(crate) fn lookup(tag: u32) -> Option<&'static Entry> {
    // Odd groups are private, even where a repeater's wildcard would match.
    if tag & 0x0001_0000 != 0 {
        return None;
    }
    if let Ok(index) = EXACT.binary_search_by_key(&tag, |entry| entry.value) {
        return Some(&EXACT[index]);
    }
    REPEATERS
        .iter()
        .find(|entry| tag & entry.mask == entry.value)
}

pub(crate) fn by_keyword(keyword: &str) -> Option<&'static Entry> {
    let index = KEYWORDS
        .binary_search_by(|index| indexed(*index).keyword.cmp(keyword))
        .ok()?;
    Some(indexed(KEYWORDS[index]))
}

fn indexed(index: u16) -> &'static Entry {
    let index = usize::from(index);
    if index < EXACT.len() {
        &EXACT[index]
    } else {
        &REPEATERS[index - EXACT.len()]
    }
}

/// Overlay/curve base spellings follow the public examples in SOURCES.md.
/// Other repeaters keep the PS3.6 keyword at their base; non-base tags append
/// the wildcard value in uppercase hex (group wildcards shifted to group bits).
/// This deterministic extension avoids collisions; see the task notes.
#[allow(dead_code)]
pub(crate) fn field_name(entry: &Entry, tag: u32) -> String {
    field_name_parts(entry.keyword, entry.value, entry.mask, tag)
}

pub(crate) fn field_name_parts(keyword: &str, value: u32, mask: u32, tag: u32) -> String {
    let wildcard = tag & !mask;
    let suffix = if mask & 0xFFFF == 0xFFFF {
        wildcard >> 16
    } else {
        wildcard
    };
    if mask == u32::MAX || (wildcard == 0 && !matches!(value >> 24, 0x50 | 0x60)) {
        keyword.into()
    } else {
        format!("{}_{suffix:X}", keyword)
    }
}

/// PS3.5 A.1: native implicit PixelData is OW. Resolve the US/SS attributes
/// identified by pydicom's correct_ambiguous_vr from the nearest enclosing
/// PixelRepresentation. Remaining ambiguous entries use the first listed VR.
#[allow(dead_code)]
pub(crate) fn implicit_vr(entry: &Entry, tag: u32, pixel_representation: u16) -> [u8; 2] {
    implicit_vr_parts(entry.vrs, tag, pixel_representation)
}

pub(crate) fn implicit_vr_parts(vrs: &[[u8; 2]], tag: u32, pixel_representation: u16) -> [u8; 2] {
    if tag == 0x7FE0_0010 && vrs.contains(b"OW") {
        return *b"OW";
    }
    if matches!(
        tag,
        0x0018_9810
            | 0x0022_1452
            | 0x0028_0104..=0x0028_0109
            | 0x0028_0110..=0x0028_0111
            | 0x0028_0120..=0x0028_0121
            | 0x0028_1101..=0x0028_1103
            | 0x0028_3002
            | 0x0040_9211
            | 0x0040_9216
            | 0x0060_3004
            | 0x0060_3006
    ) && vrs == [*b"US", *b"SS"]
    {
        return if pixel_representation == 0 {
            *b"US"
        } else {
            *b"SS"
        };
    }
    vrs[0]
}
