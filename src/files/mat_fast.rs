//! Stream double MAT arrays directly into final storage, with bounded decompression.
//! Other numeric encodings use the checked general reader.
use super::{MAX_MAT_BYTES, valid_name};
use crate::{Matrix, Result};
use std::{
    collections::BTreeMap,
    io::{BufReader, Read},
    path::Path,
};

fn tag(reader: &mut impl Read) -> Result<Option<[u8; 8]>> {
    let mut bytes = [0; 8];
    if reader.read(&mut bytes[..1]).map_err(|e| e.to_string())? == 0 {
        return Ok(None);
    }
    reader
        .read_exact(&mut bytes[1..])
        .map_err(|e| format!("Truncated MAT tag: {e}"))?;
    Ok(Some(bytes))
}
fn word(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().unwrap())
}
fn required_tag(reader: &mut impl Read) -> Result<[u8; 8]> {
    tag(reader)?.ok_or_else(|| "Truncated MAT matrix".into())
}

pub(super) fn load_doubles(path: &Path) -> Result<Option<BTreeMap<String, Matrix>>> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let file_bytes_on_disk = file.metadata().map_err(|e| e.to_string())?.len();
    let mut reader = BufReader::with_capacity(64 * 1024, file.take(MAX_MAT_BYTES as u64 + 1));
    let mut header = [0; 128];
    reader
        .read_exact(&mut header)
        .map_err(|e| format!("Truncated MAT header: {e}"))?;
    if super::mat_catalog::is_v73_header(&header) {
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let catalog = super::inspect_workspace(path, &cancel)?;
        let names = catalog.variables().map(|v| v.name.clone()).collect();
        if catalog.variables().len() == 0 {
            return Ok(Some(BTreeMap::new()));
        }
        return super::load_selected_workspace(&catalog, &names, &cancel).map(Some);
    }
    if file_bytes_on_disk > MAX_MAT_BYTES as u64 {
        return Err("File exceeds the 272 MiB limit".into());
    }
    if &header[126..] != b"IM" {
        return Ok(None);
    }
    let mut variables = BTreeMap::new();
    let mut total = 0usize;
    let mut file_bytes = 128usize;
    let mut expanded_bytes = 128usize;
    while let Some(outer) = tag(&mut reader)? {
        let kind = word(&outer[..4]);
        if kind != 14 && kind != 15 {
            return Ok(None);
        }
        let outer_len = word(&outer[4..]) as usize;
        file_bytes = file_bytes
            .checked_add(
                8 + if kind == 15 {
                    outer_len
                } else {
                    outer_len.next_multiple_of(8)
                },
            )
            .ok_or("MAT length overflow")?;
        if file_bytes > MAX_MAT_BYTES {
            return Err("File exceeds the 272 MiB limit".into());
        }
        let parsed = if kind == 15 {
            let compressed = reader.by_ref().take(outer_len as u64);
            let mut decoder = flate2::read::ZlibDecoder::new(compressed);
            let inner = required_tag(&mut decoder)?;
            if word(&inner[..4]) != 14 {
                return Err("Invalid compressed MAT matrix".into());
            }
            let decoded_len = word(&inner[4..]) as usize;
            if decoded_len > MAX_MAT_BYTES {
                return Err("Compressed MAT array exceeds memory budget".into());
            }
            expanded_bytes = expanded_bytes
                .checked_add(8 + decoded_len)
                .ok_or("MAT length overflow")?;
            if expanded_bytes > MAX_MAT_BYTES {
                return Err("Expanded MAT file exceeds 272 MiB".into());
            }
            if !read_double_matrix(&mut decoder, decoded_len, &mut total, &mut variables)? {
                return Ok(None);
            }
            let mut extra = [0; 1];
            if decoder
                .read(&mut extra)
                .map_err(|e| format!("Compressed MAT: {e}"))?
                != 0
                || decoder.total_in() != outer_len as u64
            {
                return Err("Invalid compressed MAT matrix length".into());
            }
            true
        } else {
            expanded_bytes = expanded_bytes
                .checked_add(8 + outer_len)
                .ok_or("MAT length overflow")?;
            if expanded_bytes > MAX_MAT_BYTES {
                return Err("Expanded MAT file exceeds 272 MiB".into());
            }
            read_double_matrix(&mut reader, outer_len, &mut total, &mut variables)?
        };
        if !parsed {
            return Ok(None);
        }
    }
    Ok(Some(variables))
}

fn read_double_matrix(
    reader: &mut impl Read,
    outer_len: usize,
    total: &mut usize,
    variables: &mut BTreeMap<String, Matrix>,
) -> Result<bool> {
    let flags_tag = required_tag(reader)?;
    if word(&flags_tag[..4]) != 6 || word(&flags_tag[4..]) != 8 {
        return Ok(false);
    }
    let mut flags = [0; 8];
    reader.read_exact(&mut flags).map_err(|e| e.to_string())?;
    let flags = word(&flags[..4]);
    if flags & 0xff != 6 || flags & 0xa00 != 0 {
        return Ok(false);
    }
    let dimensions_tag = required_tag(reader)?;
    if word(&dimensions_tag[..4]) != 5 || word(&dimensions_tag[4..]) != 8 {
        return Ok(false);
    }
    let mut dimensions = [0; 8];
    reader
        .read_exact(&mut dimensions)
        .map_err(|e| e.to_string())?;
    let rows = i32::from_le_bytes(dimensions[..4].try_into().unwrap());
    let cols = i32::from_le_bytes(dimensions[4..].try_into().unwrap());
    if rows < 0 || cols < 0 {
        return Err("Negative MAT dimension".into());
    }
    let len = Matrix::checked_len(rows as usize, cols as usize)?;
    *total = total.checked_add(len).ok_or("MAT size overflow")?;
    if *total > 32 * 1024 * 1024 {
        return Err("MAT workspace exceeds 256 MiB".into());
    }
    let name_tag = required_tag(reader)?;
    let name_word = word(&name_tag[..4]);
    let small = (name_word >> 16) as usize;
    let (name, name_bytes) = if small > 0 {
        if small > 4 || name_word & 65535 != 1 {
            return Ok(false);
        }
        (name_tag[4..4 + small].to_vec(), 8)
    } else {
        let len = word(&name_tag[4..]) as usize;
        if name_word != 1 || len > 64 {
            return Ok(false);
        }
        let mut name = vec![0; len.next_multiple_of(8)];
        reader.read_exact(&mut name).map_err(|e| e.to_string())?;
        name.truncate(len);
        (name, 8 + len.next_multiple_of(8))
    };
    let name = std::str::from_utf8(&name)
        .map_err(|_| "Invalid MAT variable name")?
        .trim_end_matches('\0')
        .to_owned();
    if !valid_name(&name) {
        return Err(format!("Invalid variable name: {name}"));
    }
    if variables.contains_key(&name) {
        return Err(format!("Duplicate variable: {name}"));
    }
    let real_tag = required_tag(reader)?;
    if word(&real_tag[..4]) != 9 {
        return Ok(false);
    }
    if word(&real_tag[4..]) as usize != len * 8 || outer_len != 32 + name_bytes + 8 + len * 8 {
        return Err("MAT data length does not match matrix dimensions".into());
    }
    let mut values = vec![0_f64; len];
    // bytemuck exposes the initialized f64 buffer as bytes without alignment casts
    // or unsafe application code; every f64 bit pattern is valid.
    reader
        .read_exact(bytemuck::cast_slice_mut(&mut values))
        .map_err(|e| format!("Truncated MAT values: {e}"))?;
    #[cfg(target_endian = "big")]
    for value in &mut values {
        *value = f64::from_bits(value.to_bits().swap_bytes());
    }
    variables.insert(
        name,
        Matrix::from_column_vec(rows as usize, cols as usize, values)?,
    );
    Ok(true)
}
