//! NetCDF classic files (CDF-1, 64-bit offset CDF-2 and 64-bit data CDF-5)
//! for ncread, in pure Rust. Format: the NetCDF Classic Format
//! Specification (Unidata NetCDF User Guide); all numbers are big-endian.
//! Headers are parsed with bounds on every count, and only the bytes of the
//! requested variable are read.
use super::array_data::{
    H5Array, H5Data, H5Selection, NcAttributes, Span, checked_elements, matlab_dims,
    resolve_selection,
};
use crate::{NumericClass, Result, dsp::check_cancel};
use std::{
    io::{Read, Seek, SeekFrom},
    path::Path,
    sync::atomic::AtomicBool,
};

const MAX_HEADER_BYTES: u64 = 64 * 1024 * 1024;
const MAX_READ_BYTES: usize = 2 << 30;
const MAX_ITEMS: u64 = 1 << 20;
const NC_DIMENSION: u32 = 0x0A;
const NC_VARIABLE: u32 = 0x0B;
const NC_ATTRIBUTE: u32 = 0x0C;
const STREAMING: u64 = 0xFFFF_FFFF;

/// True when `head` starts a NetCDF classic file.
pub fn is_classic(head: &[u8]) -> bool {
    head.len() >= 4 && &head[..3] == b"CDF" && matches!(head[3], 1 | 2 | 5)
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum NcType {
    Byte,
    Char,
    Short,
    Int,
    Float,
    Double,
    UByte,
    UShort,
    UInt,
    Int64,
    UInt64,
}

impl NcType {
    fn from_code(code: u32) -> Result<Self> {
        Ok(match code {
            1 => Self::Byte,
            2 => Self::Char,
            3 => Self::Short,
            4 => Self::Int,
            5 => Self::Float,
            6 => Self::Double,
            7 => Self::UByte,
            8 => Self::UShort,
            9 => Self::UInt,
            10 => Self::Int64,
            11 => Self::UInt64,
            _ => return Err(format!("unknown NetCDF type code {code}")),
        })
    }
    fn size(self) -> usize {
        match self {
            Self::Byte | Self::Char | Self::UByte => 1,
            Self::Short | Self::UShort => 2,
            Self::Int | Self::UInt | Self::Float => 4,
            Self::Double | Self::Int64 | Self::UInt64 => 8,
        }
    }
    /// The value at the start of `b` (big-endian) as f64.
    fn real(self, b: &[u8]) -> f64 {
        match self {
            Self::Byte => f64::from(b[0] as i8),
            Self::Char | Self::UByte => f64::from(b[0]),
            Self::Short => f64::from(i16::from_be_bytes([b[0], b[1]])),
            Self::UShort => f64::from(u16::from_be_bytes([b[0], b[1]])),
            Self::Int => f64::from(i32::from_be_bytes(b[..4].try_into().unwrap())),
            Self::UInt => f64::from(u32::from_be_bytes(b[..4].try_into().unwrap())),
            Self::Float => f64::from(f32::from_be_bytes(b[..4].try_into().unwrap())),
            Self::Double => f64::from_be_bytes(b[..8].try_into().unwrap()),
            Self::Int64 => i64::from_be_bytes(b[..8].try_into().unwrap()) as f64,
            Self::UInt64 => u64::from_be_bytes(b[..8].try_into().unwrap()) as f64,
        }
    }
    fn int(self, b: &[u8]) -> i128 {
        match self {
            Self::Int64 => i128::from(i64::from_be_bytes(b[..8].try_into().unwrap())),
            Self::UInt64 => i128::from(u64::from_be_bytes(b[..8].try_into().unwrap())),
            _ => self.real(b) as i128,
        }
    }
}

struct Dim {
    len: u64,
}

struct Attr {
    name: String,
    kind: NcType,
    bytes: Vec<u8>,
}

struct Var {
    name: String,
    dims: Vec<usize>,
    attrs: Vec<Attr>,
    kind: NcType,
    vsize: u64,
    begin: u64,
}

struct Header {
    numrecs: u64,
    dims: Vec<Dim>,
    vars: Vec<Var>,
}

/// Bounded big-endian reader over the header bytes.
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
    cdf5: bool,
    offset64: bool,
}

impl Cursor<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.bytes.len());
        let end = end.ok_or("the NetCDF header is truncated")?;
        let out = &self.bytes[self.at..end];
        self.at = end;
        Ok(out)
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    /// NON_NEG: 4 bytes, 8 in CDF-5.
    fn count(&mut self) -> Result<u64> {
        if self.cdf5 {
            self.u64()
        } else {
            self.u32().map(u64::from)
        }
    }
    fn offset(&mut self) -> Result<u64> {
        if self.offset64 {
            self.u64()
        } else {
            self.u32().map(u64::from)
        }
    }
    fn padded(&mut self, n: u64) -> Result<&[u8]> {
        let n = usize::try_from(n).map_err(|_| "the NetCDF header is malformed")?;
        let padded = n.checked_add(3).ok_or("the NetCDF header is malformed")? & !3;
        Ok(&self.take(padded)?[..n])
    }
    fn name(&mut self) -> Result<String> {
        let n = self.count()?;
        if n > 65536 {
            return Err("a NetCDF name is too long".into());
        }
        Ok(String::from_utf8_lossy(self.padded(n)?).into_owned())
    }
    /// A list header: (tag, count); ABSENT is tag 0 with count 0.
    fn list(&mut self, tag: u32) -> Result<u64> {
        let found = self.u32()?;
        let n = self.count()?;
        if found == 0 && n == 0 {
            return Ok(0);
        }
        if found != tag || n > MAX_ITEMS {
            return Err("the NetCDF header is malformed".into());
        }
        Ok(n)
    }
    fn attrs(&mut self) -> Result<Vec<Attr>> {
        let n = self.list(NC_ATTRIBUTE)?;
        let mut attrs = Vec::new();
        for _ in 0..n {
            let name = self.name()?;
            let kind = NcType::from_code(self.u32()?)?;
            let count = self.count()?;
            let bytes = count
                .checked_mul(kind.size() as u64)
                .filter(|&b| b <= MAX_HEADER_BYTES)
                .ok_or("a NetCDF attribute is too large")?;
            let bytes = self.padded(bytes)?.to_vec();
            attrs.push(Attr { name, kind, bytes });
        }
        Ok(attrs)
    }
}

fn parse_header(bytes: &[u8]) -> Result<Header> {
    if !is_classic(bytes) {
        return Err("not a NetCDF classic file".into());
    }
    let version = bytes[3];
    let mut c = Cursor {
        bytes,
        at: 4,
        cdf5: version == 5,
        offset64: version != 1,
    };
    let numrecs = c.count()?;
    let ndims = c.list(NC_DIMENSION)?;
    let mut dims = Vec::new();
    for _ in 0..ndims {
        c.name()?;
        let len = c.count()?;
        dims.push(Dim { len });
    }
    c.attrs()?;
    let nvars = c.list(NC_VARIABLE)?;
    let mut vars = Vec::new();
    for _ in 0..nvars {
        let name = c.name()?;
        let rank = c.count()?;
        if rank > 1024 {
            return Err("a NetCDF variable has too many dimensions".into());
        }
        let mut ids = Vec::new();
        for _ in 0..rank {
            let id = usize::try_from(c.count()?).map_err(|_| "bad dimension id")?;
            if id >= dims.len() {
                return Err("a NetCDF variable names a missing dimension".into());
            }
            ids.push(id);
        }
        let attrs = c.attrs()?;
        let kind = NcType::from_code(c.u32()?)?;
        let vsize = c.count()?;
        let begin = c.offset()?;
        vars.push(Var {
            name,
            dims: ids,
            attrs,
            kind,
            vsize,
            begin,
        });
    }
    Ok(Header {
        numrecs,
        dims,
        vars,
    })
}

fn attr_number(var: &Var, name: &str) -> Option<f64> {
    let a = var.attrs.iter().find(|a| a.name == name)?;
    if a.kind == NcType::Char || a.bytes.len() < a.kind.size() {
        return None;
    }
    Some(a.kind.real(&a.bytes))
}

/// ncread on a classic file: the variable and its fill, missing, scale and
/// offset attributes.
pub fn nc3_read(
    path: &Path,
    name: &str,
    sel: Option<&H5Selection>,
    cancel: &AtomicBool,
) -> Result<(H5Array, NcAttributes)> {
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let file_len = file.metadata().map_err(|e| e.to_string())?.len();
    let mut header = Vec::new();
    (&mut file)
        .take(MAX_HEADER_BYTES)
        .read_to_end(&mut header)
        .map_err(|e| e.to_string())?;
    let h = parse_header(&header)?;
    let wanted = name.trim_start_matches('/');
    let var = h
        .vars
        .iter()
        .find(|v| v.name == wanted)
        .ok_or_else(|| format!("the file has no variable '{wanted}'"))?;
    let record = |v: &Var| v.dims.first().is_some_and(|&d| h.dims[d].len == 0);
    let records: Vec<&Var> = h.vars.iter().filter(|v| record(v)).collect();
    // A record holds one slab of every record variable; with exactly one
    // record variable the slab is not padded.
    let malformed = || "the NetCDF file is malformed".to_owned();
    let recsize: u64 = if records.len() == 1 {
        let v = records[0];
        v.dims[1..]
            .iter()
            .try_fold(v.kind.size() as u64, |n, &d| n.checked_mul(h.dims[d].len))
            .ok_or_else(malformed)?
    } else {
        records
            .iter()
            .try_fold(0u64, |n, v| n.checked_add(v.vsize))
            .ok_or_else(malformed)?
    };
    let numrecs = if h.numrecs == STREAMING {
        let first = records.iter().map(|v| v.begin).min().unwrap_or(file_len);
        file_len
            .saturating_sub(first)
            .checked_div(recsize)
            .unwrap_or(0)
    } else {
        h.numrecs
    };
    let is_record = record(var);
    let shape: Vec<usize> = var
        .dims
        .iter()
        .enumerate()
        .map(|(k, &d)| {
            let len = if k == 0 && is_record {
                numrecs
            } else {
                h.dims[d].len
            };
            usize::try_from(len).map_err(|_| "a NetCDF dimension is too large".to_owned())
        })
        .collect::<Result<_>>()?;
    let size = var.kind.size();
    let spans: Vec<Span> = match sel {
        Some(sel) => resolve_selection(&shape, sel)?,
        None => shape
            .iter()
            .map(|&n| Span {
                start: 0,
                count: n,
                step: 1,
            })
            .collect(),
    };
    let counts: Vec<usize> = spans.iter().map(|s| s.count).collect();
    let n = checked_elements(&counts, size, MAX_READ_BYTES)?;
    // Bytes of one record's slab (or of the whole non-record variable).
    let inner_len = shape
        .iter()
        .skip(usize::from(is_record))
        .try_fold(1usize, |n, &d| n.checked_mul(d))
        .ok_or_else(malformed)?;
    let slab_bytes = inner_len.checked_mul(size).ok_or_else(malformed)?;
    if n > 0 {
        // Every byte read must lie inside the file, checked before any
        // buffer is allocated.
        let last_record = if is_record {
            (spans[0].start + (spans[0].count - 1) * spans[0].step) as u64
        } else {
            0
        };
        let end = last_record
            .checked_mul(recsize)
            .and_then(|x| x.checked_add(var.begin))
            .and_then(|x| x.checked_add(slab_bytes as u64));
        if end.is_none_or(|e| e > file_len) {
            return Err("the NetCDF file is truncated".into());
        }
        if slab_bytes > MAX_READ_BYTES {
            return Err(format!(
                "the variable exceeds the {} MiB read limit",
                MAX_READ_BYTES >> 20
            ));
        }
    }
    let mut out = vec![0u8; n * size];
    let mut slab = vec![0u8; if is_record && n > 0 { slab_bytes } else { 0 }];
    let mut whole = Vec::new();
    if !is_record && n > 0 {
        whole = vec![0u8; slab_bytes];
        file.seek(SeekFrom::Start(var.begin))
            .map_err(|e| e.to_string())?;
        file.read_exact(&mut whole)
            .map_err(|_| "the NetCDF file is truncated".to_owned())?;
    }
    // Walk the selection in C order (last index fastest).
    let rank = spans.len();
    let mut index = vec![0usize; rank];
    let mut current_record = None;
    let inner_strides: Vec<usize> = (0..rank)
        .map(|k| shape[k + 1..].iter().product::<usize>())
        .collect();
    for i in 0..n {
        if i.is_multiple_of(65536) {
            check_cancel(cancel)?;
        }
        let pos: Vec<usize> = (0..rank)
            .map(|k| spans[k].start + index[k] * spans[k].step)
            .collect();
        let source = if is_record {
            let r = pos[0];
            if current_record != Some(r) {
                let at = (r as u64)
                    .checked_mul(recsize)
                    .and_then(|x| x.checked_add(var.begin))
                    .ok_or("the NetCDF file is malformed")?;
                file.seek(SeekFrom::Start(at)).map_err(|e| e.to_string())?;
                file.read_exact(&mut slab)
                    .map_err(|_| "the NetCDF file is truncated".to_owned())?;
                current_record = Some(r);
            }
            let j: usize = (1..rank).map(|k| pos[k] * inner_strides[k]).sum();
            &slab[j * size..(j + 1) * size]
        } else {
            let j: usize = (0..rank).map(|k| pos[k] * inner_strides[k]).sum();
            &whole[j * size..(j + 1) * size]
        };
        out[i * size..(i + 1) * size].copy_from_slice(source);
        for k in (0..rank).rev() {
            index[k] += 1;
            if index[k] < spans[k].count {
                break;
            }
            index[k] = 0;
        }
    }
    let values = out.chunks_exact(size);
    let data = match var.kind {
        NcType::Double => H5Data::Double(values.map(|b| var.kind.real(b)).collect()),
        NcType::Float => H5Data::Single(values.map(|b| var.kind.real(b) as f32).collect()),
        NcType::Char => H5Data::Chars(out.iter().map(|&b| u16::from(b)).collect()),
        kind => {
            let class = match kind {
                NcType::Byte => NumericClass::Int8,
                NcType::Short => NumericClass::Int16,
                NcType::Int => NumericClass::Int32,
                NcType::Int64 => NumericClass::Int64,
                NcType::UByte => NumericClass::Uint8,
                NcType::UShort => NumericClass::Uint16,
                NcType::UInt => NumericClass::Uint32,
                _ => NumericClass::Uint64,
            };
            H5Data::Int(class, values.map(|b| kind.int(b)).collect())
        }
    };
    let attributes = NcAttributes {
        fill: attr_number(var, "_FillValue"),
        missing: attr_number(var, "missing_value"),
        scale: attr_number(var, "scale_factor"),
        offset: attr_number(var, "add_offset"),
    };
    Ok((
        H5Array {
            dims: matlab_dims(&counts),
            data,
        },
        attributes,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A CDF-1 file with dims x=3 (fixed) and t (record), a fixed short
    /// variable v(x) and one record float variable r(t, x), two records.
    fn sample() -> Vec<u8> {
        let mut b = Vec::new();
        let u32b = |b: &mut Vec<u8>, v: u32| b.extend_from_slice(&v.to_be_bytes());
        let name = |b: &mut Vec<u8>, s: &str| {
            b.extend_from_slice(&(s.len() as u32).to_be_bytes());
            b.extend_from_slice(s.as_bytes());
            while !b.len().is_multiple_of(4) {
                b.push(0);
            }
        };
        b.extend_from_slice(b"CDF\x01");
        u32b(&mut b, 2); // numrecs
        u32b(&mut b, NC_DIMENSION);
        u32b(&mut b, 2);
        name(&mut b, "x");
        u32b(&mut b, 3);
        name(&mut b, "t");
        u32b(&mut b, 0);
        u32b(&mut b, 0);
        u32b(&mut b, 0); // no global attributes
        u32b(&mut b, NC_VARIABLE);
        u32b(&mut b, 2);
        // v(x): short, scale_factor 0.5
        name(&mut b, "v");
        u32b(&mut b, 1);
        u32b(&mut b, 0);
        u32b(&mut b, NC_ATTRIBUTE);
        u32b(&mut b, 1);
        name(&mut b, "scale_factor");
        u32b(&mut b, 6);
        u32b(&mut b, 1);
        b.extend_from_slice(&0.5f64.to_be_bytes());
        u32b(&mut b, 3);
        u32b(&mut b, 8); // vsize (6 padded to 8)
        let v_begin = b.len();
        u32b(&mut b, 0); // patched
        // r(t, x): float
        name(&mut b, "r");
        u32b(&mut b, 2);
        u32b(&mut b, 1);
        u32b(&mut b, 0);
        u32b(&mut b, 0);
        u32b(&mut b, 0);
        u32b(&mut b, 5);
        u32b(&mut b, 12);
        let r_begin = b.len();
        u32b(&mut b, 0);
        let data = b.len() as u32;
        b[v_begin..v_begin + 4].copy_from_slice(&data.to_be_bytes());
        for v in [2i16, 4, 6] {
            b.extend_from_slice(&v.to_be_bytes());
        }
        b.extend_from_slice(&[0, 0]);
        let rec = b.len() as u32;
        b[r_begin..r_begin + 4].copy_from_slice(&rec.to_be_bytes());
        for v in [1f32, 2., 3., 4., 5., 6.] {
            b.extend_from_slice(&v.to_be_bytes());
        }
        b
    }

    #[test]
    fn reads_fixed_and_record_variables() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.nc");
        std::fs::write(&path, sample()).unwrap();
        let cancel = AtomicBool::new(false);
        let (v, attrs) = nc3_read(&path, "v", None, &cancel).unwrap();
        assert_eq!(v.dims, [3, 1]);
        assert!(matches!(v.data, H5Data::Int(NumericClass::Int16, ref d) if d == &[2, 4, 6]));
        assert_eq!(attrs.scale, Some(0.5));
        let (r, _) = nc3_read(&path, "r", None, &cancel).unwrap();
        // C shape (t=2, x=3) is 3x2 in MATLAB, column-major = C order.
        assert_eq!(r.dims, [3, 2]);
        assert!(matches!(r.data, H5Data::Single(ref d) if d == &[1., 2., 3., 4., 5., 6.]));
        let sel = H5Selection {
            start: vec![1, 1],
            count: vec![Some(2), Some(1)],
            stride: vec![1, 1],
        };
        let (part, _) = nc3_read(&path, "r", Some(&sel), &cancel).unwrap();
        assert_eq!(part.dims, [2, 1]);
        assert!(matches!(part.data, H5Data::Single(ref d) if d == &[5., 6.]));
        assert!(nc3_read(&path, "missing", None, &cancel).is_err());
        // Truncated files fail cleanly.
        let bytes = sample();
        std::fs::write(&path, &bytes[..bytes.len() - 5]).unwrap();
        assert!(nc3_read(&path, "r", None, &cancel).is_err());
        std::fs::write(&path, &bytes[..40]).unwrap();
        assert!(nc3_read(&path, "v", None, &cancel).is_err());
    }

    /// A header-only CDF-1 file: dims `lens` (0 = record), one variable `v`
    /// over all of them of type `code`, `numrecs` records.
    fn crafted(lens: &[u32], code: u32, numrecs: u32) -> Vec<u8> {
        let mut b = Vec::new();
        let u = |b: &mut Vec<u8>, v: u32| b.extend_from_slice(&v.to_be_bytes());
        b.extend_from_slice(b"CDF\x01");
        u(&mut b, numrecs);
        u(&mut b, NC_DIMENSION);
        u(&mut b, lens.len() as u32);
        for (i, &len) in lens.iter().enumerate() {
            u(&mut b, 1);
            b.extend_from_slice(&[b'a' + i as u8, 0, 0, 0]);
            u(&mut b, len);
        }
        u(&mut b, 0);
        u(&mut b, 0);
        u(&mut b, NC_VARIABLE);
        u(&mut b, 1);
        u(&mut b, 1);
        b.extend_from_slice(b"v\0\0\0");
        u(&mut b, lens.len() as u32);
        for i in 0..lens.len() {
            u(&mut b, i as u32);
        }
        u(&mut b, 0);
        u(&mut b, 0);
        u(&mut b, code);
        u(&mut b, 0xFFFF_FFFF);
        let begin = b.len() as u32 + 4;
        u(&mut b, begin);
        b
    }

    #[test]
    fn crafted_sizes_fail_without_huge_allocations() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.nc");
        let cancel = AtomicBool::new(false);
        let one = |rank: usize| H5Selection {
            start: vec![0; rank],
            count: vec![Some(1); rank],
            stride: vec![1; rank],
        };
        // A shape whose element count overflows.
        std::fs::write(&path, crafted(&[0x8000_0000; 3], 1, 0)).unwrap();
        assert!(nc3_read(&path, "v", Some(&one(3)), &cancel).is_err());
        assert!(nc3_read(&path, "v", None, &cancel).is_err());
        // A record of 8 GiB declared by a header-only file.
        std::fs::write(&path, crafted(&[0, 0x4000_0000], 6, 1)).unwrap();
        let error = nc3_read(&path, "v", Some(&one(2)), &cancel).unwrap_err();
        assert!(error.contains("truncated"), "{error}");
        // Streaming record count with records far larger than the file.
        std::fs::write(&path, crafted(&[0, 0x4000_0000], 6, 0xFFFF_FFFF)).unwrap();
        assert!(nc3_read(&path, "v", None, &cancel).is_ok_and(|(a, _)| a.dims == [0x4000_0000, 0]));
    }
}
