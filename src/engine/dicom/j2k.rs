//! Bounded raw JPEG 2000 Part 1 (.90/.91). T.800 A.5.1 SIZ is checked
//! independently before the decoder sees a byte. No UI or native codec.
use super::{
    jpeg,
    parser::{ElementValue, ParsedFile, PixelEncoding, PixelLocation},
    pixels::{self, Image},
};
use crate::{Result, typed_array::TypedArray};
use std::{
    io::{Read, Seek},
    ops::Range,
    sync::atomic::AtomicBool,
};

const CAP: usize = 256 * 1024 * 1024;
const MAX_TILES: usize = 4096;
const FIXED: usize = 1024 * 1024;

fn bad(reason: &str) -> String {
    format!("dicomread: malformed JPEG 2000 codestream: {reason}")
}
fn memory() -> String {
    "dicomread: JPEG 2000 exceeds the memory/output budget".into()
}

pub(crate) struct Layout {
    image: Image,
    frames: Vec<Range<usize>>,
    max_stream: usize,
    base: usize,
    budget: usize,
    photo: String,
    pub(crate) units: usize,
}

pub(crate) fn layout<R: Read + Seek>(
    reader: &mut R,
    parsed: &ParsedFile,
    budget: usize,
    max_elements: usize,
    cancel: &AtomicBool,
) -> Result<Layout> {
    let image = pixels::image_with(reader, parsed, max_elements, cancel, true)?;
    let photo = pixels::ascii(
        reader,
        parsed,
        0x0028_0004,
        *b"CS",
        "PhotometricInterpretation",
        cancel,
    )?;
    let photo = photo.trim_end_matches(' ').to_owned();
    if parsed
        .dataset
        .elements
        .iter()
        .any(|e| matches!(e.tag, 0x7FE0_0001 | 0x7FE0_0002))
    {
        return Err("dicomread: Extended Offset Table is not supported yet".into());
    }
    let element = parsed
        .dataset
        .elements
        .iter()
        .find(|e| e.tag == 0x7FE0_0010)
        .ok_or("dicomread: missing PixelData")?;
    let (offset_table, fragments) = match (&element.value, &parsed.dataset.pixels) {
        (
            ElementValue::Encapsulated,
            Some(PixelLocation::Encapsulated {
                offset_table,
                fragments,
            }),
        ) if element.vr == *b"OB" => (*offset_table, fragments),
        _ => {
            return Err(
                "dicomread: JPEG 2000 requires undefined-length encapsulated OB PixelData".into(),
            );
        }
    };
    if fragments.is_empty() {
        return Err(bad("no fragments"));
    }
    let mut compressed = 0usize;
    for fragment in fragments {
        crate::dsp::check_cancel(cancel)?;
        let value = fragment.value;
        if value.backing != parsed.dataset_backing
            || value
                .offset
                .checked_add(value.length)
                .is_none_or(|n| n > parsed.dataset_size)
        {
            return Err(bad("fragment range exceeds admitted backing"));
        }
        compressed = compressed
            .checked_add(usize::try_from(value.length).map_err(|_| memory())?)
            .ok_or_else(memory)?;
    }
    let table_len = usize::try_from(offset_table.length).map_err(|_| memory())?;
    let frames = jpeg::group(
        reader,
        parsed,
        offset_table,
        table_len,
        fragments,
        image.frames,
        cancel,
    )?;
    let max_stream = frames
        .iter()
        .map(|range| {
            fragments[range.clone()]
                .iter()
                .map(|f| f.value.length as usize)
                .sum()
        })
        .max()
        .unwrap_or(0);
    // Output bytes, typed storage and transient Arc copy; scratch, decoder's
    // owned stream and compressed tile/packet copies; BOT and frame ranges.
    let base = image
        .image_bytes()?
        .checked_mul(3)
        .and_then(|n| n.checked_add(max_stream.checked_mul(4)?))
        .and_then(|n| n.checked_add(table_len))
        .and_then(|n| n.checked_add(frames.len().checked_mul(size_of::<Range<usize>>())?))
        .ok_or_else(memory)?;
    let budget = budget.min(CAP);
    if base.checked_add(FIXED).is_none_or(|n| n > budget) {
        return Err(memory());
    }
    let units = image
        .len
        .checked_add(compressed.div_ceil(8))
        .ok_or_else(memory)?;
    Ok(Layout {
        image,
        frames,
        max_stream,
        base,
        budget,
        photo,
        units,
    })
}

fn u16be(data: &[u8], at: usize) -> Result<usize> {
    let b = data
        .get(at..at + 2)
        .ok_or_else(|| bad("truncated marker"))?;
    Ok(usize::from(u16::from_be_bytes([b[0], b[1]])))
}
fn u32be(data: &[u8], at: usize) -> Result<usize> {
    let b = data
        .get(at..at + 4)
        .ok_or_else(|| bad("truncated SIZ/SOT"))?;
    usize::try_from(u32::from_be_bytes([b[0], b[1], b[2], b[3]])).map_err(|_| memory())
}
fn segment<'a>(data: &'a [u8], pos: &mut usize) -> Result<(u8, &'a [u8])> {
    if data.get(*pos) != Some(&0xff) {
        return Err(bad("expected marker"));
    }
    let code = *data.get(*pos + 1).ok_or_else(|| bad("truncated marker"))?;
    let length = u16be(data, *pos + 2)?;
    if length < 2 {
        return Err(bad("invalid marker length"));
    }
    let end = pos.checked_add(2 + length).ok_or_else(memory)?;
    let body = data
        .get(*pos + 4..end)
        .ok_or_else(|| bad("truncated marker segment"))?;
    *pos = end;
    Ok((code, body))
}

struct Header {
    end: usize,
    decoder_bytes: usize,
    tile_pixels: usize,
    packets: Vec<usize>,
    tile_data: Vec<usize>,
}

fn signed(image: &Image) -> bool {
    matches!(
        image.class,
        crate::typed_array::NumericClass::Int8 | crate::typed_array::NumericClass::Int16
    )
}

// Inspect every main/tile COD/COC so a tile override cannot bypass the .90
// wavelet/photometric checks. Walk tile data using Psot, never entropy searching.
fn inspect(data: &[u8], layout: &Layout, lossless: bool, cancel: &AtomicBool) -> Result<Header> {
    crate::dsp::check_cancel(cancel)?;
    if data.starts_with(&[0, 0, 0, 12, b'j', b'P', b' ', b' ']) {
        return Err(bad(
            "JP2 box wrapper is forbidden; require raw J2K (PS3.5 8.2.4)",
        ));
    }
    if data.get(..4) != Some(&[0xff, 0x4f, 0xff, 0x51]) {
        return Err(bad("require SOC then SIZ in a raw J2K stream"));
    }
    let image = &layout.image;
    let mut pos = 2;
    let (_, siz) = segment(data, &mut pos)?;
    if siz.len() < 36 {
        return Err(bad("truncated SIZ"));
    }
    let components = u16be(siz, 34)?;
    if components != image.samples {
        return Err(bad("SIZ components differ from SamplesPerPixel"));
    }
    if siz.len() != 36 + 3 * components {
        return Err(bad("invalid SIZ length"));
    }
    // Part 1 only: reject Part 2/HT capability flags before entering decoder.
    if u16be(siz, 0)? & 0xC000 != 0 {
        return Err(bad("Part 2/HTJ2K capabilities are not supported"));
    }
    let (xs, ys, xo, yo) = (
        u32be(siz, 2)?,
        u32be(siz, 6)?,
        u32be(siz, 10)?,
        u32be(siz, 14)?,
    );
    if xs.checked_sub(xo) != Some(image.cols) || ys.checked_sub(yo) != Some(image.rows) {
        return Err(bad("SIZ dimensions differ from Rows x Columns"));
    }
    for comp in siz[36..].as_chunks::<3>().0 {
        if u16::from(comp[0] & 127) + 1 != image.stored {
            return Err(bad("SIZ precision differs from BitsStored"));
        }
        if (comp[0] & 128 != 0) != signed(image) {
            return Err(bad("SIZ sign differs from PixelRepresentation"));
        }
        if comp[1] != 1 || comp[2] != 1 {
            return Err(bad(
                "component subsampling is not supported (XRsiz=YRsiz=1 required)",
            ));
        }
    }
    let (tw, th, tx, ty) = (
        u32be(siz, 18)?,
        u32be(siz, 22)?,
        u32be(siz, 26)?,
        u32be(siz, 30)?,
    );
    if tw == 0
        || th == 0
        || tx > xo
        || ty > yo
        || tx.checked_add(tw).is_none_or(|x| x <= xo)
        || ty.checked_add(th).is_none_or(|y| y <= yo)
    {
        return Err(bad("invalid SIZ tile origin/extent"));
    }
    let tiles = (xs - tx)
        .div_ceil(tw)
        .checked_mul((ys - ty).div_ceil(th))
        .ok_or_else(memory)?;
    if tiles == 0 || tiles > MAX_TILES {
        return Err(bad("SIZ tile count exceeds limit (4096)"));
    }
    // Include i32 image/component planes, i32 tile/DWT buffers, tier-1/packet
    // state and coding parameters. Charge nominal tile area as well as actual
    // output, so huge tiles cannot hide behind a tiny cropped image.
    let tile_pixels = tw.checked_mul(th).ok_or_else(memory)?;
    let frame_samples = image.len / image.frames;
    let decoder_bytes = frame_samples
        .checked_mul(64)
        .and_then(|n| n.checked_add(tile_pixels.checked_mul(components)?.checked_mul(64)?))
        .and_then(|n| n.checked_add(tiles.checked_mul(64 * 1024)?))
        .and_then(|n| n.checked_add(FIXED))
        .ok_or_else(memory)?;
    if layout
        .base
        .checked_add(decoder_bytes)
        .is_none_or(|n| n > layout.budget)
    {
        return Err(memory());
    }
    let mut styles = vec![[None; 3]; tiles];
    let mut defaults = [None; 3];
    let mut tile_data = vec![0usize; tiles];
    let mut current_tile = 0usize;
    let mut cod = false;
    let mut in_tile = false;
    let mut tile_end = 0;
    let mut next_part = vec![0usize; tiles];
    let mut total_parts = vec![0usize; tiles];
    let mut markers = 0usize;
    let mut precinct_bytes = 0usize;
    loop {
        markers += 1;
        if markers > 65536 {
            return Err(bad("marker count exceeds limit (65536)"));
        }
        if markers.is_multiple_of(64) {
            crate::dsp::check_cancel(cancel)?;
        }
        if data.get(pos..pos + 2) == Some(&[0xff, 0xd9]) {
            if in_tile
                || !cod
                || next_part.contains(&0)
                || next_part
                    .iter()
                    .zip(&total_parts)
                    .any(|(n, t)| *t != 0 && n != t)
            {
                return Err(bad("truncated or missing tile data"));
            }
            let end = pos + 2;
            if !(data.len() == end || (data.len() == end + 1 && data[end] == 0)) {
                return Err(bad("unexpected bytes after EOC"));
            }
            let packets = packet_counts(&styles, (xs, ys, xo, yo), (tw, th, tx, ty), components)?;
            let total = packets
                .iter()
                .try_fold(0usize, |n, p| n.checked_add(*p))
                .ok_or_else(memory)?;
            // The decoder records at most 2^20 packets per tile. Bound the
            // whole frame below that, including its index and our owned copy.
            if total > (1 << 20) {
                return Err(bad("packet count exceeds limit"));
            }
            let decoder_bytes = decoder_bytes
                .checked_add(precinct_bytes)
                .and_then(|n| n.checked_add(markers.checked_mul(128)?))
                .and_then(|n| n.checked_add(total.checked_mul(192)?))
                .ok_or_else(memory)?;
            if layout
                .base
                .checked_add(decoder_bytes)
                .is_none_or(|n| n > layout.budget)
            {
                return Err(memory());
            }
            return Ok(Header {
                end,
                decoder_bytes,
                tile_pixels,
                packets,
                tile_data,
            });
        }
        if data.get(pos..pos + 2) == Some(&[0xff, 0x93]) {
            if !in_tile || !cod {
                return Err(bad("SOD without SOT/COD"));
            }
            pos += 2;
            let data_start = pos;
            if tile_end == 0 {
                // Psot=0 denotes the last tile-part, extending to EOC.
                let end = if data.ends_with(&[0xff, 0xd9, 0]) {
                    data.len() - 3
                } else if data.ends_with(&[0xff, 0xd9]) {
                    data.len() - 2
                } else {
                    return Err(bad("truncated final tile-part/EOC"));
                };
                if end < pos {
                    return Err(bad("truncated tile data"));
                }
                pos = end;
            } else {
                if tile_end < pos || tile_end > data.len() {
                    return Err(bad("truncated tile data"));
                }
                pos = tile_end;
            }
            tile_data[current_tile] = tile_data[current_tile]
                .checked_add(pos - data_start)
                .ok_or_else(memory)?;
            in_tile = false;
            continue;
        }
        let start = pos;
        let (code, body) = segment(data, &mut pos)?;
        if in_tile && tile_end != 0 && pos > tile_end {
            return Err(bad("tile header exceeds Psot"));
        }
        match code {
            0x90 => {
                if in_tile || !cod || body.len() != 8 {
                    return Err(bad("invalid SOT"));
                }
                let tile = u16be(body, 0)?;
                if tile >= tiles || usize::from(body[6]) != next_part[tile] {
                    return Err(bad("invalid tile-part index/order"));
                }
                current_tile = tile;
                if next_part[tile] == 0 {
                    styles[tile] = defaults;
                }
                next_part[tile] += 1;
                let total = usize::from(body[7]);
                if total != 0
                    && (next_part[tile] > total
                        || (total_parts[tile] != 0 && total_parts[tile] != total))
                {
                    return Err(bad("invalid tile-part count"));
                }
                total_parts[tile] = total;
                let size = u32be(body, 2)?;
                if size != 0 && size < 14 {
                    return Err(bad("invalid Psot"));
                }
                tile_end = if size == 0 {
                    0
                } else {
                    start.checked_add(size).ok_or_else(memory)?
                };
                in_tile = true;
            }
            0x52 => {
                if body.len() < 10 {
                    return Err(bad("truncated COD"));
                }
                precinct_bytes = precinct_bytes.max(coding(
                    &body[5..],
                    body[0] & 1,
                    image,
                    (tw, th),
                    decoder_bytes,
                    layout,
                    tiles,
                )?);
                photo(body[4], body[9], layout, lossless)?;
                let layers = u16be(body, 2)?;
                if layers == 0 || layers > 1024 {
                    return Err(bad("quality layer count exceeds limit (1..1024)"));
                }
                let style = Some(Style::new(&body[5..], body[0] & 1, layers)?);
                if in_tile {
                    if next_part[current_tile] != 1 {
                        return Err(bad("COD override after first tile-part"));
                    }
                    styles[current_tile] = [style; 3];
                } else {
                    defaults = [style; 3];
                }
                if !in_tile && cod {
                    return Err(bad("duplicate main COD"));
                }
                cod = true;
            }
            0x53 => {
                if body.len() < 7 || usize::from(body[0]) >= components {
                    return Err(bad("invalid COC"));
                }
                precinct_bytes = precinct_bytes.max(coding(
                    &body[2..],
                    body[1],
                    image,
                    (tw, th),
                    decoder_bytes,
                    layout,
                    tiles,
                )?);
                let styles = if in_tile {
                    if next_part[current_tile] != 1 {
                        return Err(bad("COC override after first tile-part"));
                    }
                    &mut styles[current_tile]
                } else {
                    &mut defaults
                };
                let layers = styles[usize::from(body[0])]
                    .ok_or_else(|| bad("COC before COD"))?
                    .layers;
                styles[usize::from(body[0])] = Some(Style::new(&body[2..], body[1], layers)?);
                // Mixed wavelets are incompatible with a DICOM Part 1 MCT.
                if (lossless || image.ybr) && body[6] != u8::from(layout.photo != "YBR_ICT") {
                    return Err(bad(
                        "COC requires matching reversible 5/3 / photometric wavelet",
                    ));
                }
            }
            0x5c | 0x5d => {
                let at = usize::from(code == 0x5d);
                let q = *body
                    .get(at)
                    .ok_or_else(|| bad("truncated quantization marker"))?;
                if lossless && q & 31 != 0 {
                    return Err(bad(".90 requires no quantization and reversible 5/3"));
                }
            }
            // The packet index does not report bytes consumed from packed
            // headers. Reject these explicitly rather than accept truncation.
            0x60 | 0x61 => {
                return Err(bad("packed packet headers (PPM/PPT) are not supported yet"));
            }
            // Part 1 table/pointer/comment markers. Decoder validates contents.
            0x55 | 0x57 | 0x58 | 0x5e | 0x5f | 0x63 | 0x64 => {}
            _ => return Err(bad(&format!("unsupported marker FF{code:02X}"))),
        }
    }
}

#[derive(Clone, Copy)]
struct Style {
    levels: usize,
    layers: usize,
    precincts: [u8; 17],
}
impl Style {
    fn new(body: &[u8], explicit: u8, layers: usize) -> Result<Self> {
        let levels = usize::from(body[0]) + 1;
        let mut precincts = [0xff; 17];
        if explicit != 0 {
            precincts[..levels].copy_from_slice(
                body.get(5..5 + levels)
                    .ok_or_else(|| bad("invalid precinct lengths"))?,
            );
        }
        Ok(Self {
            levels,
            layers,
            precincts,
        })
    }
}

// T.800 B.6: count all component/resolution/precinct/layer packets. A complete
// decode must report every packet, including an empty packet's one-bit header.
fn packet_counts(
    styles: &[[Option<Style>; 3]],
    image: (usize, usize, usize, usize),
    tile: (usize, usize, usize, usize),
    components: usize,
) -> Result<Vec<usize>> {
    let (xs, ys, xo, yo) = image;
    let (tw, th, tx, ty) = tile;
    let nx = (xs - tx).div_ceil(tw);
    let mut counts = Vec::with_capacity(styles.len());
    for (index, styles) in styles.iter().enumerate() {
        let x = tx
            .checked_add((index % nx).checked_mul(tw).ok_or_else(memory)?)
            .ok_or_else(memory)?;
        let y = ty
            .checked_add((index / nx).checked_mul(th).ok_or_else(memory)?)
            .ok_or_else(memory)?;
        let (x0, y0, x1, y1) = (
            x.max(xo),
            y.max(yo),
            x.saturating_add(tw).min(xs),
            y.saturating_add(th).min(ys),
        );
        let mut count = 0usize;
        for style in &styles[..components] {
            let style = style.ok_or_else(|| bad("missing COD"))?;
            for level in 0..style.levels {
                let scale = 1usize << (style.levels - 1 - level);
                let (rx0, ry0, rx1, ry1) = (
                    x0.div_ceil(scale),
                    y0.div_ceil(scale),
                    x1.div_ceil(scale),
                    y1.div_ceil(scale),
                );
                if rx0 == rx1 || ry0 == ry1 {
                    continue;
                }
                let p = style.precincts[level];
                let (px, py) = (1usize << (p & 15), 1usize << (p >> 4));
                let precincts = (rx1.div_ceil(px) - rx0 / px)
                    .checked_mul(ry1.div_ceil(py) - ry0 / py)
                    .ok_or_else(memory)?;
                count = count
                    .checked_add(precincts.checked_mul(style.layers).ok_or_else(memory)?)
                    .ok_or_else(memory)?;
            }
        }
        counts.push(count);
    }
    Ok(counts)
}

fn decode_complete(stream: &[u8], header: &Header, image: &Image) -> Result<openjp2::CoreImage> {
    let fail = |e| bad(&format!("decoder error: {e}"));
    let mut decoder = openjp2::Decoder::new(openjp2::J2KFormat::J2K).map_err(fail)?;
    decoder
        .apply_settings(&openjp2::DecoderSettings::default())
        .map_err(fail)?;
    decoder.set_strict(true).map_err(fail)?;
    decoder
        .set_limits(
            openjp2::DecodeLimits::new()
                .max_image_size(image.cols as u32, image.rows as u32)
                .max_tile_pixels(header.tile_pixels as u64)
                .max_mem_alloc(header.decoder_bytes as u64)
                .max_metadata_len(65535),
        )
        .map_err(fail)?;
    let mut stream = openjp2::Stream::from_bytes(stream.to_vec());
    decoder.read_header(&mut stream).map_err(fail)?;
    let decoded = decoder.decode(&mut stream).map_err(fail)?;
    decoder.end_decompress(&mut stream).map_err(fail)?;
    let index = decoder
        .cstr_index()
        .ok_or_else(|| bad("decoder omitted packet index"))?;
    if index.tiles.len() != header.packets.len() {
        return Err(bad("decoder returned partial tiles"));
    }
    for (tile, (&packets, &bytes)) in index
        .tiles
        .iter()
        .zip(header.packets.iter().zip(&header.tile_data))
    {
        if tile.packet_index.len() != packets
            || tile
                .packet_index
                .iter()
                .any(|p| p.end_ph_pos <= p.start_pos || p.end_pos < p.end_ph_pos)
        {
            return Err(bad(
                "decoder error: truncated packet headers / partial image",
            ));
        }
        let consumed = tile.packet_index.iter().try_fold(0usize, |n, p| {
            n.checked_add(usize::try_from(p.end_pos - p.start_pos).ok()?)
        });
        if consumed != Some(bytes) {
            return Err(bad("decoder error: incomplete or trailing packet data"));
        }
    }
    Ok(decoded)
}

fn photo(mct: u8, wavelet: u8, layout: &Layout, lossless: bool) -> Result<()> {
    if wavelet > 1 || (lossless && wavelet != 1) {
        return Err(bad(
            ".90 requires reversible 5/3 wavelet (9/7 is forbidden)",
        ));
    }
    let expected = match layout.photo.as_str() {
        "MONOCHROME1" | "MONOCHROME2" | "RGB" => 0,
        "YBR_RCT" if wavelet == 1 => 1,
        "YBR_ICT" if wavelet == 0 && !lossless => 1,
        _ => {
            return Err(bad(
                "PhotometricInterpretation disagrees with COD MCT/wavelet",
            ));
        }
    };
    if mct != expected {
        return Err(bad("PhotometricInterpretation disagrees with COD MCT flag"));
    }
    Ok(())
}

// Bound adversarially small precincts separately from the ordinary 64-byte
// per-sample workspace. Every resolution has at most three bands; 1024 bytes
// per precinct covers packet/band/codeblock descriptors conservatively.
fn coding(
    body: &[u8],
    style: u8,
    image: &Image,
    tile_dims: (usize, usize),
    decoder_bytes: usize,
    layout: &Layout,
    tiles: usize,
) -> Result<usize> {
    if body.len() < 5
        || body[0] > 16
        || style & !1 != 0
        || body[1] > 8
        || body[2] > 8
        || u16::from(body[1]) + u16::from(body[2]) > 8
        || body[3] & !0x3f != 0
    {
        return Err(bad("unsupported coding style/resolution/codeblock limits"));
    }
    if style & 1 != 0 {
        let levels = usize::from(body[0]) + 1;
        if body.len() != 5 + levels {
            return Err(bad("invalid precinct lengths"));
        }
        let mut precincts = 0usize;
        // Nominal dimensions at every resolution overestimate edge tiles and
        // lower resolutions; add one precinct for grid-origin misalignment.
        for &p in &body[5..] {
            let nx = tile_dims
                .0
                .div_ceil(1usize << (p & 15))
                .checked_add(1)
                .ok_or_else(memory)?;
            let ny = tile_dims
                .1
                .div_ceil(1usize << (p >> 4))
                .checked_add(1)
                .ok_or_else(memory)?;
            precincts = precincts
                .checked_add(nx.checked_mul(ny).ok_or_else(memory)?)
                .ok_or_else(memory)?;
        }
        let extra = precincts
            .checked_mul(3 * 1024)
            .and_then(|n| n.checked_mul(image.samples))
            .and_then(|n| n.checked_mul(tiles))
            .ok_or_else(memory)?;
        if layout
            .base
            .checked_add(decoder_bytes)
            .and_then(|n| n.checked_add(extra))
            .is_none_or(|n| n > layout.budget)
        {
            return Err(memory());
        }
        return Ok(extra);
    }
    Ok(0)
}

pub(crate) fn decode<R: Read + Seek>(
    reader: &mut R,
    parsed: &ParsedFile,
    layout: &Layout,
    cancel: &AtomicBool,
) -> Result<TypedArray> {
    let Some(PixelLocation::Encapsulated { fragments, .. }) = &parsed.dataset.pixels else {
        return Err(bad("missing encapsulated pixels"));
    };
    let image = &layout.image;
    let mut out = Vec::new();
    out.try_reserve_exact(image.image_bytes()?)
        .map_err(|_| memory())?;
    out.resize(image.image_bytes()?, 0);
    let mut scratch = Vec::new();
    scratch
        .try_reserve_exact(layout.max_stream)
        .map_err(|_| memory())?;
    let frame_bytes = out.len() / image.frames;
    let lossless = matches!(parsed.encoding, PixelEncoding::Jpeg2000 { lossless: true });
    for (frame, range) in layout.frames.iter().enumerate() {
        crate::dsp::check_cancel(cancel)?;
        jpeg::read_frame(reader, &fragments[range.clone()], &mut scratch, cancel)?;
        let header = inspect(&scratch, layout, lossless, cancel)?;
        crate::dsp::check_cancel(cancel)?;
        // Low-level operations contain assertions/indexing on malformed input;
        // catch the complete decoder lifetime, including index extraction.
        let decoded =
            std::panic::catch_unwind(|| decode_complete(&scratch[..header.end], &header, image))
                .map_err(|_| bad("decoder panic on malformed input"))??;
        crate::dsp::check_cancel(cancel)?;
        if decoded.x1.checked_sub(decoded.x0).map(|n| n as usize) != Some(image.cols)
            || decoded.y1.checked_sub(decoded.y0).map(|n| n as usize) != Some(image.rows)
            || decoded.comps.len() != image.samples
        {
            return Err(bad("decoder output dimensions/components mismatch"));
        }
        let dst = &mut out[frame * frame_bytes..(frame + 1) * frame_bytes];
        let (min, max) = if signed(image) {
            (
                -(1i32 << (image.stored - 1)),
                (1i32 << (image.stored - 1)) - 1,
            )
        } else {
            (0, (1i32 << image.stored) - 1)
        };
        for (component, comp) in decoded.comps.iter().enumerate() {
            if comp.w as usize != image.cols
                || comp.h as usize != image.rows
                || comp.data.len() != image.plane_len
                || comp.prec != u32::from(image.stored)
                || (comp.sgnd != 0) != signed(image)
            {
                return Err(bad("decoder returned partial/mismatched component"));
            }
            for (pixel, &value) in comp.data.iter().enumerate() {
                if pixel.is_multiple_of(1024) {
                    crate::dsp::check_cancel(cancel)?;
                }
                if value < min || value > max {
                    return Err(bad("decoded sample is outside BitsStored range"));
                }
                let at = (pixel * image.samples + component) * image.class.size();
                let bytes = (value as u16).to_le_bytes();
                dst[at..at + image.class.size()].copy_from_slice(&bytes[..image.class.size()]);
            }
        }
    }
    drop(scratch);
    pixels::convert_interleaved(&out, image, cancel)
}

#[cfg(test)]
mod tests {
    use super::super::parser;
    use super::*;
    use std::io::{Cursor, SeekFrom};
    use std::sync::atomic::Ordering;
    const PUBLIC: &[u8] = include_bytes!("../../../tests/fixtures/dicom/j2k_public.dcm");
    const FRAMES: &[u8] = include_bytes!("../../../tests/fixtures/dicom/j2k_frames.dcm");

    struct Watched<'a> {
        input: Cursor<&'static [u8]>,
        forbidden: Vec<Range<u64>>,
        cancel: &'a AtomicBool,
        cancel_after_read: usize,
        cancel_at_seek: Option<u64>,
        reads: usize,
    }
    impl Read for Watched<'_> {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let start = self.input.position();
            let end = start + out.len() as u64;
            assert!(
                self.forbidden
                    .iter()
                    .all(|r| end <= r.start || start >= r.end),
                "metadata read fragment payload"
            );
            self.reads += 1;
            if self.reads == self.cancel_after_read {
                self.cancel.store(true, Ordering::Relaxed);
            }
            self.input.read(out)
        }
    }
    impl Seek for Watched<'_> {
        fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
            let at = self.input.seek(pos)?;
            if self.cancel_at_seek == Some(at) {
                self.cancel.store(true, Ordering::Relaxed);
            }
            Ok(at)
        }
    }
    fn payloads(parsed: &ParsedFile) -> Vec<Range<u64>> {
        let Some(PixelLocation::Encapsulated { fragments, .. }) = &parsed.dataset.pixels else {
            panic!("not encapsulated")
        };
        fragments
            .iter()
            .map(|f| f.value.offset..f.value.offset + f.value.length)
            .collect()
    }
    #[test]
    fn info_and_layout_never_read_fragment_payloads() {
        let cancel = AtomicBool::new(false);
        for bytes in [PUBLIC, FRAMES] {
            let parsed = parser::parse(
                &mut Cursor::new(bytes),
                bytes.len() as u64,
                1 << 20,
                &cancel,
            )
            .unwrap();
            let mut reader = Watched {
                input: Cursor::new(bytes),
                forbidden: payloads(&parsed),
                cancel: &cancel,
                cancel_after_read: usize::MAX,
                cancel_at_seek: None,
                reads: 0,
            };
            let parsed = parser::parse(&mut reader, bytes.len() as u64, 1 << 20, &cancel).unwrap();
            let plan = layout(&mut reader, &parsed, CAP, 1 << 20, &cancel).unwrap();
            assert_eq!(
                plan.image.len,
                if bytes.len() == PUBLIC.len() {
                    128
                } else {
                    256
                }
            );
        }
    }
    #[test]
    fn cancellation_in_concatenation_and_before_next_frame_payload() {
        let cancel = AtomicBool::new(false);
        let mut input = Cursor::new(FRAMES);
        let parsed = parser::parse(&mut input, FRAMES.len() as u64, 1 << 20, &cancel).unwrap();
        let plan = layout(&mut input, &parsed, CAP, 1 << 20, &cancel).unwrap();
        assert_eq!(plan.frames, [0..2, 2..4]);
        for (after, seek, expected_reads) in [
            (1, None, 1),
            (usize::MAX, Some(payloads(&parsed)[2].start), 2),
        ] {
            cancel.store(false, Ordering::Relaxed);
            let mut reader = Watched {
                input: Cursor::new(FRAMES),
                forbidden: vec![],
                cancel: &cancel,
                cancel_after_read: after,
                cancel_at_seek: seek,
                reads: 0,
            };
            let error = decode(&mut reader, &parsed, &plan, &cancel).unwrap_err();
            assert!(error.contains("cancelled"), "{error}");
            assert_eq!(reader.reads, expected_reads);
        }
    }
    #[test]
    #[ignore = "manual comparable debug/gate decoder timing"]
    fn jpeg2000_frame_timing() {
        let bytes: &[u8] = include_bytes!("../../../tests/fixtures/dicom/j2k_large.dcm");
        let cancel = AtomicBool::new(false);
        let mut input = Cursor::new(bytes);
        let parsed = parser::parse(&mut input, bytes.len() as u64, 1 << 20, &cancel).unwrap();
        let plan = layout(&mut input, &parsed, CAP, 1 << 20, &cancel).unwrap();
        decode(&mut input, &parsed, &plan, &cancel).unwrap();
        let start = std::time::Instant::now();
        for _ in 0..5 {
            decode(&mut input, &parsed, &plan, &cancel).unwrap();
        }
        eprintln!(
            "JPEG2000 full decode+conversion 512x512 uint16, iterations=5, per_frame_ms={:.3}",
            start.elapsed().as_secs_f64() * 200.
        );
    }

    #[test]
    fn siz_admission_exact_boundary_and_hard_cap() {
        let cancel = AtomicBool::new(false);
        let mut input = Cursor::new(PUBLIC);
        let parsed = parser::parse(&mut input, PUBLIC.len() as u64, 1 << 20, &cancel).unwrap();
        let mut plan = layout(&mut input, &parsed, usize::MAX, 1 << 20, &cancel).unwrap();
        assert_eq!(plan.budget, CAP);
        let range = payloads(&parsed)[0].clone();
        let stream = &PUBLIC[range.start as usize..range.end as usize];
        let header = inspect(stream, &plan, true, &cancel).unwrap();
        plan.budget = plan.base + header.decoder_bytes;
        assert!(inspect(stream, &plan, true, &cancel).is_ok());
        plan.budget -= 1;
        assert!(
            inspect(stream, &plan, true, &cancel)
                .err()
                .unwrap()
                .contains("budget")
        );
        cancel.store(true, Ordering::Relaxed);
        assert!(
            inspect(stream, &plan, true, &cancel)
                .err()
                .unwrap()
                .contains("cancelled")
        );
    }
}
