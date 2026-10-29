//! Bounded Level-5 MAT subset for MATLAB's serialized graphics hierarchy.
//! Data-only decoding: no callbacks, object construction or external runtime.
use crate::Result;
use std::{collections::BTreeMap, io::Read};
pub(super) const LIMIT: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Endian {
    Little,
    Big,
}
impl Endian {
    fn u16(self, bytes: [u8; 2]) -> u16 {
        match self {
            Self::Little => u16::from_le_bytes(bytes),
            Self::Big => u16::from_be_bytes(bytes),
        }
    }
    fn u32(self, bytes: [u8; 4]) -> u32 {
        match self {
            Self::Little => u32::from_le_bytes(bytes),
            Self::Big => u32::from_be_bytes(bytes),
        }
    }
    fn u64(self, bytes: [u8; 8]) -> u64 {
        match self {
            Self::Little => u64::from_le_bytes(bytes),
            Self::Big => u64::from_be_bytes(bytes),
        }
    }
}

#[derive(Debug)]
pub(super) struct Expanded {
    pub bytes: Vec<u8>,
    pub endian: Endian,
}
impl Expanded {
    pub fn hierarchy(&self) -> Result<Value<'_>> {
        hierarchy(&self.bytes, self.endian)
    }
}

#[derive(Debug)]
pub(super) enum Value<'a> {
    Number {
        kind: u32,
        endian: Endian,
        bytes: &'a [u8],
        count: usize,
    },
    Text(String),
    Struct(Vec<BTreeMap<String, Value<'a>>>),
    Cell(Vec<Value<'a>>),
    Opaque,
}
impl Value<'_> {
    pub fn text(&self) -> Option<&str> {
        if let Self::Text(s) = self {
            Some(s)
        } else {
            None
        }
    }
    pub fn structs(&self) -> Option<&[BTreeMap<String, Value<'_>>]> {
        if let Self::Struct(s) = self {
            Some(s)
        } else {
            None
        }
    }
    pub fn len(&self) -> usize {
        match self {
            Self::Number { count, .. } => *count,
            Self::Struct(s) => s.len(),
            Self::Cell(s) => s.len(),
            Self::Text(s) => s.len(),
            Self::Opaque => 0,
        }
    }
    pub fn number(&self, index: usize) -> Option<f64> {
        let Self::Number {
            kind,
            endian,
            bytes,
            count,
        } = self
        else {
            return None;
        };
        if index >= *count {
            return None;
        }
        let width = width(*kind)?;
        let b = &bytes[index * width..(index + 1) * width];
        Some(match kind {
            1 => b[0] as i8 as f64,
            2 => b[0] as f64,
            3 => endian.u16(b.try_into().ok()?) as i16 as f64,
            4 => endian.u16(b.try_into().ok()?) as f64,
            5 => endian.u32(b.try_into().ok()?) as i32 as f64,
            6 => endian.u32(b.try_into().ok()?) as f64,
            7 => f32::from_bits(endian.u32(b.try_into().ok()?)) as f64,
            9 => f64::from_bits(endian.u64(b.try_into().ok()?)),
            12 => {
                let n = endian.u64(b.try_into().ok()?) as i64;
                if n.unsigned_abs() > 1 << 53 {
                    return None;
                }
                n as f64
            }
            13 => {
                let n = endian.u64(b.try_into().ok()?);
                if n > 1 << 53 {
                    return None;
                }
                n as f64
            }
            _ => return None,
        })
    }
}
fn width(kind: u32) -> Option<usize> {
    match kind {
        1 | 2 => Some(1),
        3 | 4 => Some(2),
        5..=7 => Some(4),
        9 | 12 | 13 => Some(8),
        _ => None,
    }
}

fn element<'a>(bytes: &'a [u8], pos: &mut usize, endian: Endian) -> Result<(u32, &'a [u8])> {
    let header = bytes
        .get(*pos..pos.checked_add(8).ok_or("FIG offset overflow")?)
        .ok_or("Truncated FIG tag")?;
    let tag = endian.u32(header[..4].try_into().unwrap());
    let short = (tag >> 16) as usize;
    if short != 0 {
        if short > 4 {
            return Err("Invalid compact FIG tag".into());
        }
        *pos += 8;
        return Ok((tag & 65535, &header[4..4 + short]));
    }
    let len = endian.u32(header[4..].try_into().unwrap()) as usize;
    let start = *pos + 8;
    let end = start.checked_add(len).ok_or("FIG size overflow")?;
    let payload = bytes.get(start..end).ok_or("Truncated FIG data")?;
    let padded = if tag == 15 {
        end
    } else {
        end.checked_add((8 - len % 8) % 8)
            .ok_or("FIG padding overflow")?
    };
    if padded > bytes.len() {
        return Err("Truncated FIG padding".into());
    }
    *pos = padded;
    Ok((tag, payload))
}

/// Expand bounded compressed top-level elements without changing byte order.
pub(super) fn expand(bytes: &[u8]) -> Result<Expanded> {
    let endian = match bytes.get(126..128) {
        Some(b"IM") => Endian::Little,
        Some(b"MI") => Endian::Big,
        _ => {
            return Err(
                "FIG requires Level-5 MAT data; HDF5/v7.3 figures are not supported".into(),
            );
        }
    };
    let mut output = Vec::new();
    let mut pos = 128;
    while pos < bytes.len() {
        let start = pos;
        let (kind, payload) = element(bytes, &mut pos, endian)?;
        if kind == 15 {
            let remaining = LIMIT - output.len();
            let mut decoder = flate2::read::ZlibDecoder::new(payload);
            let mut expanded = Vec::new();
            decoder
                .by_ref()
                .take(remaining as u64 + 1)
                .read_to_end(&mut expanded)
                .map_err(|e| format!("FIG decompression: {e}"))?;
            if expanded.len() > remaining || decoder.total_in() != payload.len() as u64 {
                return Err("FIG compression exceeds budget or contains trailing bytes".into());
            }
            let mut p = 0;
            let (inner, _) = element(&expanded, &mut p, endian)?;
            if inner != 14 || p != expanded.len() {
                return Err("Invalid compressed FIG matrix".into());
            }
            output.extend(expanded);
        } else if kind == 14 {
            if pos - start > LIMIT - output.len() {
                return Err("Expanded FIG exceeds 64 MiB".into());
            }
            output.extend_from_slice(&bytes[start..pos]);
        } else {
            return Err("Unsupported FIG top-level element".into());
        }
    }
    Ok(Expanded {
        bytes: output,
        endian,
    })
}
pub(super) fn hierarchy(bytes: &[u8], endian: Endian) -> Result<Value<'_>> {
    let mut pos = 0;
    let mut found = None;
    let mut nodes = 0;
    while pos < bytes.len() {
        let (kind, payload) = element(bytes, &mut pos, endian)?;
        if kind != 14 {
            return Err("Invalid FIG matrix".into());
        }
        let mut p = 0;
        let (_, _, name, _) = header(payload, &mut p, endian)?;
        if name == "hgS_070000" || name == "hgS_050200" {
            if found.is_some() {
                return Err("Multiple FIG hierarchies are not supported".into());
            }
            found = Some(parse(payload, 0, &mut nodes, endian)?);
        }
        // hgM/MCOS objects and function workspace are never constructed or executed.
    }
    found.ok_or_else(|| "FIG has no supported graphics hierarchy (hgS_070000)".into())
}
fn header<'a>(
    bytes: &'a [u8],
    pos: &mut usize,
    endian: Endian,
) -> Result<(u32, usize, &'a str, usize)> {
    let (kind, flags) = element(bytes, pos, endian)?;
    if kind != 6 || flags.len() != 8 {
        return Err("Invalid FIG array flags".into());
    }
    let flags = endian.u32(flags[..4].try_into().unwrap());
    // Opaque objects have a different header; callers never deserialize them.
    if flags & 255 == 17 {
        return Ok((flags, 0, "", 0));
    }
    let (kind, dims) = element(bytes, pos, endian)?;
    if kind != 5 || dims.len() < 8 || dims.len() > 32 || dims.len() % 4 != 0 {
        return Err("Invalid FIG dimensions".into());
    }
    let mut count = 1usize;
    for d in dims.as_chunks::<4>().0 {
        let n = endian.u32(*d) as i32;
        if n < 0 {
            return Err("Negative FIG dimension".into());
        }
        count = count
            .checked_mul(n as usize)
            .ok_or("FIG dimension overflow")?;
    }
    if count > 4_000_000 {
        return Err("FIG array exceeds resource limit".into());
    }
    let (kind, name) = element(bytes, pos, endian)?;
    if kind != 1 || name.len() > 256 {
        return Err("Invalid FIG variable name".into());
    }
    let name = std::str::from_utf8(name).map_err(|_| "Invalid FIG name encoding")?;
    Ok((flags, count, name, dims.len() / 4))
}
fn parse<'a>(
    bytes: &'a [u8],
    depth: usize,
    nodes: &mut usize,
    endian: Endian,
) -> Result<Value<'a>> {
    *nodes += 1;
    if depth > 32 || *nodes > 100_000 {
        return Err("FIG structure exceeds nesting or node limit".into());
    }
    // MATLAB serializes absent structure fields as a zero-length miMATRIX.
    if bytes.is_empty() {
        return Ok(Value::Number {
            kind: 9,
            endian,
            bytes: &[],
            count: 0,
        });
    }
    let mut pos = 0;
    let (flags, count, _, _) = header(bytes, &mut pos, endian)?;
    if flags & 0x800 != 0 {
        return Ok(Value::Opaque);
    }
    let value = match flags & 255 {
        1 => {
            if count > 100_000 {
                return Err("Too many FIG cells".into());
            }
            let mut cells = Vec::new();
            for _ in 0..count {
                let (kind, payload) = element(bytes, &mut pos, endian)?;
                if kind != 14 {
                    return Err("Invalid FIG cell".into());
                }
                cells.push(parse(payload, depth + 1, nodes, endian)?);
            }
            Value::Cell(cells)
        }
        2 => {
            let (kind, size) = element(bytes, &mut pos, endian)?;
            if kind != 5 || size.len() != 4 {
                return Err("Invalid FIG field width".into());
            }
            let size = endian.u32(size.try_into().unwrap()) as usize;
            if !(1..=256).contains(&size) {
                return Err("Invalid FIG field width".into());
            }
            let (kind, names) = element(bytes, &mut pos, endian)?;
            if kind != 1
                || names.len() % size != 0
                || names.len() / size > 1024
                || count > 100_000
                || count.saturating_mul(names.len() / size) > 100_000
            {
                return Err("FIG structure is too large".into());
            }
            let names: Vec<_> = names
                .chunks_exact(size)
                .map(|s| {
                    let end = s.iter().position(|b| *b == 0).unwrap_or(s.len());
                    std::str::from_utf8(&s[..end]).map_err(|_| "Invalid FIG field name")
                })
                .collect::<std::result::Result<_, _>>()?;
            let mut structures = Vec::new();
            for _ in 0..count {
                let mut fields = BTreeMap::new();
                for &name in &names {
                    let (kind, payload) = element(bytes, &mut pos, endian)?;
                    if kind != 14 {
                        return Err("Invalid FIG field value".into());
                    }
                    let value = if name == "ApplicationData" {
                        application_data(payload, depth + 1, nodes, endian)?
                    } else {
                        parse(payload, depth + 1, nodes, endian)
                            .map_err(|e| format!("FIG field {name}: {e}"))?
                    };
                    if fields.insert(name.to_owned(), value).is_some() {
                        return Err("Duplicate FIG field".into());
                    }
                }
                structures.push(fields);
            }
            Value::Struct(structures)
        }
        4 => {
            if count == 0 && pos == bytes.len() {
                return Ok(Value::Text(String::new()));
            }
            let (kind, payload) = element(bytes, &mut pos, endian)?;
            if count > 65536 || payload.len() > 4 * 65536 {
                return Err("FIG text exceeds 65536 characters".into());
            }
            let text = match kind {
                1 | 2 | 16 => std::str::from_utf8(payload)
                    .map_err(|_| "Invalid FIG UTF-8")?
                    .to_owned(),
                4 | 17 => {
                    if payload.len() % 2 != 0 {
                        return Err("Invalid FIG UTF-16".into());
                    }
                    let words: Vec<_> = payload
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|b| endian.u16(*b))
                        .collect();
                    String::from_utf16(&words).map_err(|_| "Invalid FIG UTF-16")?
                }
                18 => {
                    if payload.len() % 4 != 0 {
                        return Err("Invalid FIG UTF-32".into());
                    }
                    payload
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|b| char::from_u32(endian.u32(*b)).ok_or("Invalid FIG UTF-32"))
                        .collect::<std::result::Result<String, _>>()?
                }
                _ => return Err("Unsupported FIG text encoding".into()),
            };
            Value::Text(text)
        }
        6..=15 => {
            if count == 0 && pos == bytes.len() {
                return Ok(Value::Number {
                    kind: 9,
                    endian,
                    bytes: &[],
                    count: 0,
                });
            }
            let (kind, payload) = element(bytes, &mut pos, endian)?;
            let width = width(kind).ok_or("Unsupported FIG numeric storage")?;
            if payload.len() != count * width {
                return Err("FIG numeric size mismatch".into());
            }
            Value::Number {
                kind,
                endian,
                bytes: payload,
                count,
            }
        }
        _ => return Ok(Value::Opaque),
    };
    if pos != bytes.len() {
        return Err("Unexpected FIG array content".into());
    }
    Ok(value)
}

// Read only the observed legend membership field. Callback/object payloads and
// unrelated application data are never recursively parsed or executed.
fn application_data<'a>(
    bytes: &'a [u8],
    depth: usize,
    nodes: &mut usize,
    endian: Endian,
) -> Result<Value<'a>> {
    *nodes += 1;
    if depth > 32 || *nodes > 100_000 {
        return Err("FIG application data exceeds nesting or node limit".into());
    }
    if bytes.is_empty() {
        return Ok(Value::Opaque);
    }
    let mut pos = 0;
    let (flags, count, _, _) = header(bytes, &mut pos, endian)?;
    if flags & 255 != 2 {
        return Ok(Value::Opaque);
    }
    if flags & (0x800 | 0x200) != 0 {
        return Err("Invalid FIG ApplicationData structure flags".into());
    }
    let (kind, size) = element(bytes, &mut pos, endian)?;
    if kind != 5 || size.len() != 4 {
        return Err("Invalid FIG application data field width".into());
    }
    let size = endian.u32(size.try_into().unwrap()) as usize;
    if !(1..=256).contains(&size) {
        return Err("Invalid FIG application data field width".into());
    }
    let (kind, names) = element(bytes, &mut pos, endian)?;
    if kind != 1 || names.len() % size != 0 || names.len() / size > 1024 {
        return Err("FIG application data field table exceeds bounds".into());
    }
    let names: Vec<_> = names
        .chunks_exact(size)
        .map(|s| {
            let end = s.iter().position(|b| *b == 0).unwrap_or(s.len());
            std::str::from_utf8(&s[..end]).map_err(|_| "Invalid FIG application data field name")
        })
        .collect::<std::result::Result<_, _>>()?;
    if !names.contains(&"PlotChildren") {
        return Ok(Value::Opaque);
    }
    if count != 1 || names.iter().filter(|&&name| name == "PlotChildren").count() != 1 {
        return Err("FIG PlotChildren requires unique metadata in scalar ApplicationData".into());
    }
    let mut selected = None;
    for name in names {
        let (kind, payload) = element(bytes, &mut pos, endian)?;
        if kind != 14 {
            return Err("Invalid FIG application data value".into());
        }
        if name != "PlotChildren" {
            continue;
        }
        let mut p = 0;
        let (flags, count, _, dimensions) = header(payload, &mut p, endian)?;
        if !(6..=15).contains(&(flags & 255))
            || flags & (0x800 | 0x200) != 0
            || count > crate::plot::MAX_SERIES
            || dimensions != 2
        {
            return Err("FIG PlotChildren must be a bounded real numeric vector".into());
        }
        let mut dims_pos = 0;
        element(payload, &mut dims_pos, endian)?; // flags
        let (_, dims) = element(payload, &mut dims_pos, endian)?;
        let dims: Vec<_> = dims
            .as_chunks::<4>()
            .0
            .iter()
            .map(|d| endian.u32(*d) as i32)
            .collect();
        if count > 0 && dims.iter().filter(|&&d| d > 1).count() > 1 {
            return Err("FIG PlotChildren must be a vector".into());
        }
        selected = Some(parse(payload, depth + 1, nodes, endian)?);
    }
    if pos != bytes.len() {
        return Err("Unexpected FIG application data content".into());
    }
    Ok(Value::Struct(vec![BTreeMap::from([(
        "PlotChildren".to_owned(),
        selected.unwrap(),
    )])]))
}

fn tag(output: &mut Vec<u8>, kind: u32, payload: &[u8]) {
    output.extend_from_slice(&kind.to_le_bytes());
    output.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    output.extend_from_slice(payload);
    if kind != 15 {
        output.resize(output.len() + (8 - payload.len() % 8) % 8, 0);
    }
}
// Keep the hierarchy as small descriptors until the final bounded allocation.
// Sample arrays are borrowed from the immutable figure; no per-axis buffers or
// intermediate serialized copies are needed at each nesting level.
pub(super) struct Encoded<'a> {
    name: String,
    rows: usize,
    cols: usize,
    body: Body<'a>,
    payload_len: usize,
}
enum Body<'a> {
    Cell(Vec<Encoded<'a>>),
    Numbers(Vec<f64>),
    Coordinate(&'a [[f64; 2]], usize),
    Text(Vec<u16>),
    Struct {
        field_size: usize,
        names: Vec<u8>,
        items: Vec<Vec<Encoded<'a>>>,
    },
}
fn tagged_len(len: usize) -> usize {
    8 + len + (8 - len % 8) % 8
}
impl<'a> Encoded<'a> {
    fn new(name: &str, rows: usize, cols: usize, body: Body<'a>, body_len: usize) -> Self {
        Self {
            name: name.to_owned(),
            rows,
            cols,
            body,
            payload_len: 16 + 16 + tagged_len(name.len()) + body_len,
        }
    }
    fn len(&self) -> usize {
        tagged_len(self.payload_len)
    }
    fn write(&self, out: &mut Vec<u8>) {
        let class: u32 = match self.body {
            Body::Numbers(_) | Body::Coordinate(..) => 6,
            Body::Text(_) => 4,
            Body::Cell(_) => 1,
            Body::Struct { .. } => 2,
        };
        out.extend(14_u32.to_le_bytes());
        out.extend((self.payload_len as u32).to_le_bytes());
        let mut flags = [0; 8];
        flags[..4].copy_from_slice(&class.to_le_bytes());
        tag(out, 6, &flags);
        let mut dims = [0; 8];
        dims[..4].copy_from_slice(&(self.rows as i32).to_le_bytes());
        dims[4..].copy_from_slice(&(self.cols as i32).to_le_bytes());
        tag(out, 5, &dims);
        tag(out, 1, self.name.as_bytes());
        match &self.body {
            Body::Numbers(values) => {
                out.extend(9_u32.to_le_bytes());
                out.extend(((values.len() * 8) as u32).to_le_bytes());
                for value in values {
                    out.extend(value.to_le_bytes());
                }
            }
            Body::Coordinate(points, axis) => {
                out.extend(9_u32.to_le_bytes());
                out.extend(((points.len() * 8) as u32).to_le_bytes());
                for point in *points {
                    out.extend(point[*axis].to_le_bytes());
                }
            }
            Body::Text(words) => {
                out.extend(4_u32.to_le_bytes());
                out.extend(((words.len() * 2) as u32).to_le_bytes());
                for word in words {
                    out.extend(word.to_le_bytes());
                }
                out.resize(out.len() + (8 - words.len() * 2 % 8) % 8, 0);
            }
            Body::Cell(values) => {
                for value in values {
                    value.write(out);
                }
            }
            Body::Struct {
                field_size,
                names,
                items,
            } => {
                tag(out, 5, &(*field_size as u32).to_le_bytes());
                tag(out, 1, names);
                for item in items {
                    for value in item {
                        value.write(out);
                    }
                }
            }
        }
        debug_assert_eq!(out.len() % 8, 0);
    }
}
pub(super) fn numbers(values: impl ExactSizeIterator<Item = f64>) -> Encoded<'static> {
    let n = values.len();
    Encoded::new("", 1, n, Body::Numbers(values.collect()), tagged_len(n * 8))
}
pub(super) fn coordinate(points: &[[f64; 2]], axis: usize) -> Encoded<'_> {
    assert!(axis < 2);
    Encoded::new(
        "",
        1,
        points.len(),
        Body::Coordinate(points, axis),
        tagged_len(points.len() * 8),
    )
}
pub(super) fn text(value: &str) -> Encoded<'static> {
    let words: Vec<_> = value.encode_utf16().collect();
    let len = words.len();
    Encoded::new("", 1, len, Body::Text(words), tagged_len(len * 2))
}
/// Bounded literal label cells, derived from the preserved declarative probe.
pub(super) fn legend_text_cells(values: &[&str]) -> Result<Encoded<'static>> {
    if values.is_empty()
        || values.len() > crate::plot::MAX_SERIES
        || values.iter().any(|s| s.len() > 4096)
    {
        return Err("FIG legend requires 1..64 labels of at most 4096 UTF-8 bytes".into());
    }
    // At most 64 bounded strings; UTF-16 storage is bounded by twice UTF-8 bytes.
    let items: Vec<_> = values.iter().map(|s| text(s)).collect();
    let bytes = items
        .iter()
        .try_fold(0usize, |n, item| n.checked_add(item.len()))
        .ok_or("Legend cell encoding size overflow")?;
    Ok(Encoded::new("", 1, items.len(), Body::Cell(items), bytes))
}
pub(super) fn structure<'a>(name: &str, fields: Vec<(&str, Encoded<'a>)>) -> Encoded<'a> {
    structures(name, vec![fields])
}
pub(super) fn structures<'a>(name: &str, items: Vec<Vec<(&str, Encoded<'a>)>>) -> Encoded<'a> {
    if items.is_empty() {
        return numbers(std::iter::empty());
    }
    let field_size = items[0].iter().map(|(n, _)| n.len() + 1).max().unwrap_or(1);
    let mut names = Vec::with_capacity(field_size * items[0].len());
    for (name, _) in &items[0] {
        names.extend(name.as_bytes());
        names.resize(names.len() + field_size - name.len(), 0);
    }
    let body_len = tagged_len(4)
        + tagged_len(names.len())
        + items
            .iter()
            .flatten()
            .map(|(_, value)| value.len())
            .sum::<usize>();
    let count = items.len();
    let items = items
        .into_iter()
        .map(|item| item.into_iter().map(|(_, value)| value).collect())
        .collect();
    Encoded::new(
        name,
        1,
        count,
        Body::Struct {
            field_size,
            names,
            items,
        },
        body_len,
    )
}
pub(super) fn file(root: Encoded<'_>) -> Result<Vec<u8>> {
    let len = root
        .len()
        .checked_add(128)
        .filter(|len| *len <= LIMIT)
        .ok_or("Exported FIG exceeds 64 MiB")?;
    let mut bytes = Vec::with_capacity(len);
    bytes.resize(128, b' ');
    let label = b"MATLAB 5.0 MAT-file, MatRust data-only graphics hierarchy";
    bytes[..label.len()].copy_from_slice(label);
    bytes[116..124].fill(0);
    bytes[124..128].copy_from_slice(&[0, 1, b'I', b'M']);
    root.write(&mut bytes);
    debug_assert_eq!(bytes.len(), len);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legend_application_data_keeps_only_bounded_numeric_membership() {
        let mut callback = text("must not be traversed");
        for _ in 0..40 {
            callback = structure("", vec![("nested", callback)]);
        }
        let bytes = file(structure(
            "hgS_070000",
            vec![(
                "ApplicationData",
                structure(
                    "",
                    vec![
                        ("PostDeserializeFcn", callback),
                        ("PeerAxes", text("opaque arbitrary metadata")),
                        ("PlotChildren", numbers([10., 11.].into_iter())),
                    ],
                ),
            )],
        ))
        .unwrap();
        let expanded = expand(&bytes).unwrap();
        let root = expanded.hierarchy().unwrap();
        let app = root.structs().unwrap()[0]["ApplicationData"]
            .structs()
            .unwrap();
        assert_eq!(app[0].len(), 1);
        assert_eq!(app[0]["PlotChildren"].number(1), Some(11.));
    }
    #[test]
    fn legend_membership_metadata_rejects_types_bounds_duplicates_and_nonscalar_container() {
        for selected in [
            text("10"),
            structure("", vec![]),
            numbers(vec![1.; 65].into_iter()),
        ] {
            let mut bytes = Vec::new();
            structure("", vec![("PlotChildren", selected)]).write(&mut bytes);
            assert!(application_data(&bytes[8..], 0, &mut 0, Endian::Little).is_err());
        }
        let mut bytes = Vec::new();
        structure(
            "",
            vec![
                ("PlotChildren", numbers([1.].into_iter())),
                ("PlotChildren", numbers([1.].into_iter())),
            ],
        )
        .write(&mut bytes);
        assert!(
            application_data(&bytes[8..], 0, &mut 0, Endian::Little)
                .unwrap_err()
                .contains("unique")
        );
        let mut bytes = Vec::new();
        structures(
            "",
            vec![
                vec![("PlotChildren", numbers([1.].into_iter()))],
                vec![("PlotChildren", numbers([2.].into_iter()))],
            ],
        )
        .write(&mut bytes);
        assert!(application_data(&bytes[8..], 0, &mut 0, Endian::Little).is_err());
        let mut bytes = Vec::new();
        structure("", vec![("unrelated", text("ignored"))]).write(&mut bytes);
        assert!(matches!(
            application_data(&bytes[8..], 0, &mut 0, Endian::Little).unwrap(),
            Value::Opaque
        ));
    }
    #[test]
    fn legend_membership_rejects_complex_logical_and_matrix_shapes_without_decoding_objects() {
        let mut selected = Vec::new();
        numbers([1., 2., 3., 4.].into_iter()).write(&mut selected);
        let mut container = Vec::new();
        structure(
            "",
            vec![("PlotChildren", numbers([1., 2., 3., 4.].into_iter()))],
        )
        .write(&mut container);
        let offset = container
            .windows(selected.len())
            .position(|w| w == selected)
            .unwrap();
        for flags in [6_u32 | 0x800, 6 | 0x200, 16, 17] {
            let mut changed = container.clone();
            changed[offset + 16..offset + 20].copy_from_slice(&flags.to_le_bytes());
            assert!(application_data(&changed[8..], 0, &mut 0, Endian::Little).is_err());
        }
        let mut changed = container;
        changed[offset + 32..offset + 36].copy_from_slice(&2_i32.to_le_bytes());
        changed[offset + 36..offset + 40].copy_from_slice(&2_i32.to_le_bytes());
        assert!(
            application_data(&changed[8..], 0, &mut 0, Endian::Little)
                .unwrap_err()
                .contains("vector")
        );
    }
    #[test]
    fn oversized_fig_is_rejected_from_borrowed_descriptors() {
        let samples = vec![[1., 2.]; 1_000_000];
        let fields = (0..9).map(|_| ("data", coordinate(&samples, 0))).collect();
        let root = structure("hgS_070000", fields);
        assert!(root.len() > LIMIT);
        assert_eq!(file(root).unwrap_err(), "Exported FIG exceeds 64 MiB");
    }
    #[test]
    fn fig_structure_depth_dimensions_and_compression_are_bounded() {
        let mut value = numbers([1.].into_iter());
        for _ in 0..34 {
            value = structure("", vec![("nested", value)]);
        }
        let bytes = file(structure("hgS_070000", vec![("root", value)])).unwrap();
        let expanded = expand(&bytes).unwrap();
        assert!(expanded.hierarchy().unwrap_err().contains("nesting"));
        let mut bytes = Vec::new();
        numbers([1.].into_iter()).write(&mut bytes);
        // Matrix tag (8), flags element (16), dimensions tag (8), then dimensions.
        bytes[32..36].copy_from_slice(&i32::MAX.to_le_bytes());
        let mut wrapped = file(structure(
            "hgS_070000",
            vec![("bad", numbers([1.].into_iter()))],
        ))
        .unwrap();
        let start = wrapped.len() - bytes.len();
        wrapped[start..].copy_from_slice(&bytes);
        let bytes = wrapped;
        assert!(expand(&bytes).unwrap().hierarchy().is_err());
        let source = std::fs::read("tests/fixtures/fig/matlab-lines.fig").unwrap();
        let len = u32::from_le_bytes(source[132..136].try_into().unwrap()) as usize;
        let mut checksum = source.clone();
        checksum[136 + len - 1] ^= 1;
        assert!(expand(&checksum).is_err());
        let mut trailing = source[..136 + len].to_vec();
        trailing.push(0);
        trailing.extend_from_slice(&source[136 + len..]);
        trailing[132..136].copy_from_slice(&((len + 1) as u32).to_le_bytes());
        assert!(expand(&trailing).is_err());
    }
}

#[cfg(test)]
#[path = "fig_endian_tests.rs"]
mod endian_tests;
