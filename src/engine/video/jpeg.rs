//! Baseline JPEG entropy decoding and separable inverse DCT, with Annex K
//! default tables for AVI1 frames. No feature or external codec dependency.
use super::{admit, buffer};
use crate::Result;
use std::sync::atomic::AtomicBool;
const BAD: &str = "VideoReader: corrupt or unsupported baseline MJPEG frame.";
pub(super) const Z: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];
#[derive(Clone, Default)]
struct Huff {
    entries: Vec<(u16, u8, u8)>,
}
impl Huff {
    fn new(counts: &[u8], values: &[u8]) -> Result<Self> {
        if values.len() > 256 {
            return Err(BAD.into());
        }
        let mut entries = Vec::new();
        let mut code = 0u32;
        let mut k = 0;
        for (i, &n) in counts.iter().enumerate() {
            for _ in 0..n {
                if k >= values.len() || code >= (1 << (i + 1)) {
                    return Err(BAD.into());
                }
                entries.push((code as u16, (i + 1) as u8, values[k]));
                k += 1;
                code += 1;
            }
            code <<= 1;
        }
        Ok(Self { entries })
    }
    fn symbol(&self, b: &mut Bits<'_>) -> Result<u8> {
        let mut code = 0;
        for len in 1..=16 {
            code = (code << 1) | b.bit()? as u16;
            if let Some(x) = self.entries.iter().find(|x| x.1 == len && x.0 == code) {
                return Ok(x.2);
            }
        }
        Err(BAD.into())
    }
}
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    byte: u8,
    left: u8,
}
impl Bits<'_> {
    fn bit(&mut self) -> Result<u8> {
        if self.left == 0 {
            self.byte = *self.data.get(self.pos).ok_or(BAD)?;
            self.pos += 1;
            if self.byte == 255 {
                if self.data.get(self.pos) != Some(&0) {
                    return Err(BAD.into());
                }
                self.pos += 1;
            }
            self.left = 8;
        }
        self.left -= 1;
        Ok((self.byte >> self.left) & 1)
    }
    fn signed(&mut self, n: u8) -> Result<i32> {
        if n > 16 {
            return Err(BAD.into());
        }
        if n == 0 {
            return Ok(0);
        }
        let mut v = 0;
        for _ in 0..n {
            v = (v << 1) | i32::from(self.bit()?);
        }
        Ok(if v < (1 << (n - 1)) {
            v + 1 - (1 << n)
        } else {
            v
        })
    }
    fn restart(&mut self, n: u8) -> Result<()> {
        self.left = 0;
        if self.data.get(self.pos..self.pos + 2) != Some(&[255, 0xd0 + n]) {
            return Err(BAD.into());
        }
        self.pos += 2;
        Ok(())
    }
}
struct Component {
    id: u8,
    h: usize,
    v: usize,
    q: usize,
    dc: usize,
    ac: usize,
    pred: i32,
    stride: usize,
    plane: Vec<u8>,
}
fn be(b: &[u8], p: usize) -> usize {
    usize::from(u16::from_be_bytes([b[p], b[p + 1]]))
}
fn idct(c: &[f64; 64], basis: &[[f64; 8]; 8]) -> [u8; 64] {
    let mut tmp = [[0.; 8]; 8];
    let mut out = [0; 64];
    for v in 0..8 {
        for x in 0..8 {
            for u in 0..8 {
                tmp[v][x] += c[v * 8 + u] * basis[x][u];
            }
        }
    }
    for y in 0..8 {
        for x in 0..8 {
            let mut a = 128.;
            for v in 0..8 {
                a += tmp[v][x] * basis[y][v];
            }
            out[y * 8 + x] = a.round().clamp(0., 255.) as u8;
        }
    }
    out
}
fn tables() -> Result<[[Huff; 4]; 2]> {
    let mut h: [[Huff; 4]; 2] = std::array::from_fn(|_| std::array::from_fn(|_| Huff::default()));
    h[0][0] = Huff::new(&STD_LUMA_DC_CODE_LENGTHS, &STD_LUMA_DC_VALUES)?;
    h[0][1] = Huff::new(&STD_CHROMA_DC_CODE_LENGTHS, &STD_CHROMA_DC_VALUES)?;
    h[1][0] = Huff::new(&STD_LUMA_AC_CODE_LENGTHS, &STD_LUMA_AC_VALUES)?;
    h[1][1] = Huff::new(&STD_CHROMA_AC_CODE_LENGTHS, &STD_CHROMA_AC_VALUES)?;
    Ok(h)
}
pub(super) fn decode(
    data: &[u8],
    width: usize,
    height: usize,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Vec<u8>> {
    if data.get(..2) != Some(&[255, 216]) {
        return Err(BAD.into());
    }
    admit(
        width
            .checked_mul(height)
            .and_then(|n| n.checked_mul(32))
            .ok_or(BAD)?,
        budget,
    )?;
    let mut h = tables()?;
    let mut q = [[0.; 64]; 4];
    let mut components = Vec::new();
    let mut restart = 0;
    let mut pos = 2;
    while pos < data.len() {
        if data[pos] != 255 {
            return Err(BAD.into());
        }
        while data.get(pos) == Some(&255) {
            pos += 1;
        }
        let marker = *data.get(pos).ok_or(BAD)?;
        pos += 1;
        if pos + 2 > data.len() {
            return Err(BAD.into());
        }
        let size = be(data, pos);
        if size < 2 || size > data.len() - pos {
            return Err(BAD.into());
        }
        let b = &data[pos + 2..pos + size];
        pos += size;
        match marker {
            0xdb => {
                let mut i = 0;
                while i < b.len() {
                    let spec = b[i];
                    i += 1;
                    let table = (spec & 15) as usize;
                    let step = if spec >> 4 == 0 {
                        1
                    } else if spec >> 4 == 1 {
                        2
                    } else {
                        return Err(BAD.into());
                    };
                    if table > 3 || i + 64 * step > b.len() {
                        return Err(BAD.into());
                    }
                    for k in 0..64 {
                        q[table][Z[k]] = if step == 1 {
                            f64::from(b[i + k])
                        } else {
                            be(b, i + k * 2) as f64
                        };
                    }
                    i += 64 * step;
                }
            }
            0xc4 => {
                let mut i = 0;
                while i < b.len() {
                    let spec = b[i];
                    i += 1;
                    if spec >> 4 > 1 || spec & 15 > 3 || i + 16 > b.len() {
                        return Err(BAD.into());
                    }
                    let counts = &b[i..i + 16];
                    let n = counts.iter().map(|&x| usize::from(x)).sum::<usize>();
                    i += 16;
                    if i + n > b.len() {
                        return Err(BAD.into());
                    }
                    h[(spec >> 4) as usize][(spec & 15) as usize] =
                        Huff::new(counts, &b[i..i + n])?;
                    i += n;
                }
            }
            0xc0 => {
                if b.len() < 6
                    || b[0] != 8
                    || be(b, 1) != height
                    || be(b, 3) != width
                    || ![1, 3].contains(&b[5])
                    || b.len() != 6 + usize::from(b[5]) * 3
                {
                    return Err(BAD.into());
                }
                if !components.is_empty() {
                    return Err(BAD.into());
                }
                for c in b[6..].as_chunks::<3>().0 {
                    let hh = usize::from(c[1] >> 4);
                    let vv = usize::from(c[1] & 15);
                    if hh == 0 || vv == 0 || hh > 4 || vv > 4 || c[2] > 3 {
                        return Err(BAD.into());
                    }
                    components.push(Component {
                        id: c[0],
                        h: hh,
                        v: vv,
                        q: usize::from(c[2]),
                        dc: 0,
                        ac: 0,
                        pred: 0,
                        stride: 0,
                        plane: Vec::new(),
                    });
                }
            }
            0xdd => {
                if b.len() != 2 {
                    return Err(BAD.into());
                }
                restart = be(b, 0);
            }
            0xda => {
                if components.is_empty()
                    || b.len() != 4 + components.len() * 2
                    || usize::from(b[0]) != components.len()
                    || b[b.len() - 3..] != [0, 63, 0]
                {
                    return Err(BAD.into());
                }
                for (c, s) in components
                    .iter_mut()
                    .zip(b[1..b.len() - 3].as_chunks::<2>().0)
                {
                    if c.id != s[0] || s[1] >> 4 > 3 || s[1] & 15 > 3 || q[c.q][0] == 0. {
                        return Err(BAD.into());
                    }
                    c.dc = usize::from(s[1] >> 4);
                    c.ac = usize::from(s[1] & 15);
                }
                let mh = components.iter().map(|c| c.h).max().ok_or(BAD)?;
                let mv = components.iter().map(|c| c.v).max().ok_or(BAD)?;
                let cols = width.div_ceil(mh * 8);
                let rows = height.div_ceil(mv * 8);
                let plane_bytes = components.iter().try_fold(0usize, |total, c| {
                    cols.checked_mul(c.h * 8)
                        .and_then(|n| n.checked_mul(rows))
                        .and_then(|n| n.checked_mul(c.v * 8))
                        .and_then(|n| n.checked_add(total))
                        .ok_or(BAD)
                })?;
                let required = width
                    .checked_mul(height)
                    .and_then(|n| n.checked_mul(3))
                    .and_then(|n| n.checked_add(plane_bytes))
                    .and_then(|n| n.checked_add(data.len()))
                    .and_then(|n| n.checked_add(16 * 1024))
                    .ok_or(BAD)?;
                admit(required, budget)?;
                for c in &mut components {
                    c.stride = cols * c.h * 8;
                    c.plane = buffer(c.stride * rows * c.v * 8, budget)?;
                }
                let mut bits = Bits {
                    data: &data[pos..],
                    pos: 0,
                    byte: 0,
                    left: 0,
                };
                let mut basis = [[0.; 8]; 8];
                for (x, row) in basis.iter_mut().enumerate() {
                    for (u, b) in row.iter_mut().enumerate() {
                        *b =
                            0.5 * (if u == 0 {
                                std::f64::consts::FRAC_1_SQRT_2
                            } else {
                                1.
                            }) * ((2 * x + 1) as f64 * u as f64 * std::f64::consts::PI / 16.).cos();
                    }
                }
                for m in 0..cols * rows {
                    crate::dsp::check_cancel(cancel)?;
                    if restart > 0 && m > 0 && m.is_multiple_of(restart) {
                        bits.restart(((m / restart - 1) % 8) as u8)?;
                        for c in &mut components {
                            c.pred = 0;
                        }
                    }
                    for c in &mut components {
                        for by in 0..c.v {
                            for bx in 0..c.h {
                                let mut coeff = [0.; 64];
                                let n = h[0][c.dc].symbol(&mut bits)?;
                                if n > 11 {
                                    return Err(BAD.into());
                                }
                                c.pred = c.pred.checked_add(bits.signed(n)?).ok_or(BAD)?;
                                if c.pred.abs() > 32767 {
                                    return Err(BAD.into());
                                }
                                coeff[0] = f64::from(c.pred) * q[c.q][0];
                                let mut k = 1;
                                while k < 64 {
                                    let s = h[1][c.ac].symbol(&mut bits)?;
                                    if s == 0 {
                                        break;
                                    }
                                    if s == 0xf0 {
                                        k += 16;
                                        if k > 64 {
                                            return Err(BAD.into());
                                        }
                                        continue;
                                    }
                                    let n = s & 15;
                                    if n == 0 || n > 10 {
                                        return Err(BAD.into());
                                    }
                                    k += usize::from(s >> 4);
                                    if k >= 64 {
                                        return Err(BAD.into());
                                    }
                                    coeff[Z[k]] = f64::from(bits.signed(n)?) * q[c.q][Z[k]];
                                    k += 1;
                                }
                                let block = idct(&coeff, &basis);
                                let base = ((m / cols) * c.v * 8 + by * 8) * c.stride
                                    + (m % cols) * c.h * 8
                                    + bx * 8;
                                for y in 0..8 {
                                    c.plane[base + y * c.stride..base + y * c.stride + 8]
                                        .copy_from_slice(&block[y * 8..y * 8 + 8]);
                                }
                            }
                        }
                    }
                }
                if bits.data.get(bits.pos..bits.pos + 2) != Some(&[255, 217]) {
                    return Err(BAD.into());
                }
                let mut out = buffer(width * height * 3, budget)?;
                for y in 0..height {
                    crate::dsp::check_cancel(cancel)?;
                    for x in 0..width {
                        let sample = |c: &Component| {
                            f64::from(c.plane[(y * c.v / mv) * c.stride + x * c.h / mh])
                        };
                        let l = sample(&components[0]);
                        let rgb = if components.len() == 1 {
                            [l; 3]
                        } else {
                            let cb = sample(&components[1]) - 128.;
                            let cr = sample(&components[2]) - 128.;
                            [
                                l + 1.402 * cr,
                                l - 0.344136 * cb - 0.714136 * cr,
                                l + 1.772 * cb,
                            ]
                        };
                        for c in 0..3 {
                            out[(y * width + x) * 3 + c] = rgb[c].round().clamp(0., 255.) as u8;
                        }
                    }
                }
                return Ok(out);
            }
            0xe0..=0xef | 0xfe => {}
            _ => return Err(BAD.into()),
        }
    }
    Err(BAD.into())
}

// JPEG T.81 Annex K standard Huffman tables (normative integer data).
pub(super) const STD_LUMA_DC_CODE_LENGTHS: [u8; 16] = [
    0x00, 0x01, 0x05, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];
pub(super) const STD_LUMA_DC_VALUES: [u8; 12] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B,
];
pub(super) const STD_CHROMA_DC_CODE_LENGTHS: [u8; 16] = [
    0x00, 0x03, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
];
pub(super) const STD_CHROMA_DC_VALUES: [u8; 12] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B,
];
pub(super) const STD_LUMA_AC_CODE_LENGTHS: [u8; 16] = [
    0x00, 0x02, 0x01, 0x03, 0x03, 0x02, 0x04, 0x03, 0x05, 0x05, 0x04, 0x04, 0x00, 0x00, 0x01, 0x7D,
];
pub(super) const STD_LUMA_AC_VALUES: [u8; 162] = [
    0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07,
    0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xA1, 0x08, 0x23, 0x42, 0xB1, 0xC1, 0x15, 0x52, 0xD1, 0xF0,
    0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0A, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x25, 0x26, 0x27, 0x28,
    0x29, 0x2A, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49,
    0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69,
    0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89,
    0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7,
    0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4, 0xC5,
    0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE1, 0xE2,
    0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8,
    0xF9, 0xFA,
];
pub(super) const STD_CHROMA_AC_CODE_LENGTHS: [u8; 16] = [
    0x00, 0x02, 0x01, 0x02, 0x04, 0x04, 0x03, 0x04, 0x07, 0x05, 0x04, 0x04, 0x00, 0x01, 0x02, 0x77,
];
pub(super) const STD_CHROMA_AC_VALUES: [u8; 162] = [
    0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71,
    0x13, 0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xA1, 0xB1, 0xC1, 0x09, 0x23, 0x33, 0x52, 0xF0,
    0x15, 0x62, 0x72, 0xD1, 0x0A, 0x16, 0x24, 0x34, 0xE1, 0x25, 0xF1, 0x17, 0x18, 0x19, 0x1A, 0x26,
    0x27, 0x28, 0x29, 0x2A, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48,
    0x49, 0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68,
    0x69, 0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87,
    0x88, 0x89, 0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5,
    0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3,
    0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA,
    0xE2, 0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8,
    0xF9, 0xFA,
];
