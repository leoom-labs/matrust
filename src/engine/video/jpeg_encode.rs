//! Full JFIF baseline 4:4:4 JPEG. The image encoder is used when available;
//! the core-only fallback follows T.81 Annexes A, F and K (no optional UI).
use crate::Result;
use std::sync::atomic::AtomicBool;

// Quality is a public video quality scale, not a JPEG quantizer identifier.
// A square-root curve preserves sharp testsrc edges at the default setting.
fn quantizer_quality(quality: u8) -> u8 {
    (f64::from(quality).sqrt() * 10.).round().clamp(1., 100.) as u8
}

#[cfg(feature = "plot-export")]
pub(super) fn encode(
    rgb: &[u8],
    w: usize,
    h: usize,
    quality: u8,
    cancel: &AtomicBool,
) -> Result<Vec<u8>> {
    crate::dsp::check_cancel(cancel)?;
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quantizer_quality(quality))
        .encode(rgb, w as u32, h as u32, image::ExtendedColorType::Rgb8)
        .map_err(|e| format!("VideoWriter: JPEG encoding failed: {e}"))?;
    crate::dsp::check_cancel(cancel)?;
    Ok(out)
}

#[cfg(not(feature = "plot-export"))]
mod core {
    use super::*;
    use crate::engine::video::jpeg::{
        STD_CHROMA_AC_CODE_LENGTHS, STD_CHROMA_AC_VALUES, STD_CHROMA_DC_CODE_LENGTHS,
        STD_CHROMA_DC_VALUES, STD_LUMA_AC_CODE_LENGTHS, STD_LUMA_AC_VALUES,
        STD_LUMA_DC_CODE_LENGTHS, STD_LUMA_DC_VALUES, Z,
    };
    const QY: [u8; 64] = [
        16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69,
        56, 14, 17, 22, 29, 51, 87, 80, 62, 18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81,
        104, 113, 92, 49, 64, 78, 87, 103, 121, 120, 101, 72, 92, 95, 98, 112, 100, 103, 99,
    ];
    const QC: [u8; 64] = [
        17, 18, 24, 47, 99, 99, 99, 99, 18, 21, 26, 66, 99, 99, 99, 99, 24, 26, 56, 99, 99, 99, 99,
        99, 47, 66, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
        99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    ];
    fn segment(out: &mut Vec<u8>, marker: u8, data: &[u8]) {
        out.extend_from_slice(&[255, marker]);
        out.extend_from_slice(&((data.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(data);
    }
    fn codes(counts: &[u8], values: &[u8]) -> [(u16, u8); 256] {
        let mut out = [(0, 0); 256];
        let mut code = 0u16;
        let mut k = 0;
        for (i, &n) in counts.iter().enumerate() {
            for _ in 0..n {
                out[values[k] as usize] = (code, i as u8 + 1);
                k += 1;
                code += 1;
            }
            if i != 15 {
                code <<= 1;
            }
        }
        out
    }
    struct Bits {
        out: Vec<u8>,
        byte: u8,
        used: u8,
    }
    impl Bits {
        fn put(&mut self, code: u16, len: u8) {
            for i in (0..len).rev() {
                self.byte = (self.byte << 1) | ((code >> i) & 1) as u8;
                self.used += 1;
                if self.used == 8 {
                    self.out.push(self.byte);
                    if self.byte == 255 {
                        self.out.push(0);
                    }
                    self.byte = 0;
                    self.used = 0;
                }
            }
        }
        fn symbol(&mut self, table: &[(u16, u8); 256], symbol: usize) {
            let (code, len) = table[symbol];
            self.put(code, len);
        }
        fn signed(&mut self, n: i32, table: &[(u16, u8); 256], run: usize) {
            let len = (32 - n.unsigned_abs().leading_zeros()) as u8;
            self.symbol(table, run * 16 + len as usize);
            if len > 0 {
                self.put(if n < 0 { (n - 1) as u16 } else { n as u16 }, len);
            }
        }
    }
    pub(crate) fn encode(
        rgb: &[u8],
        w: usize,
        h: usize,
        quality: u8,
        cancel: &AtomicBool,
    ) -> Result<Vec<u8>> {
        let quality = u32::from(quantizer_quality(quality));
        let scale = if quality < 50 {
            5000 / quality
        } else {
            200 - quality * 2
        };
        let q: [[u8; 64]; 2] =
            [QY, QC].map(|q| q.map(|v| ((u32::from(v) * scale + 50) / 100).clamp(1, 255) as u8));
        let mut out = vec![255, 216];
        segment(&mut out, 0xe0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
        let mut dqt = Vec::new();
        for (i, table) in q.iter().enumerate() {
            dqt.push(i as u8);
            dqt.extend(Z.map(|z| table[z]));
        }
        segment(&mut out, 0xdb, &dqt);
        let mut sof = vec![8];
        sof.extend_from_slice(&(h as u16).to_be_bytes());
        sof.extend_from_slice(&(w as u16).to_be_bytes());
        sof.extend_from_slice(&[3, 1, 0x11, 0, 2, 0x11, 1, 3, 0x11, 1]);
        segment(&mut out, 0xc0, &sof);
        let mut tables = Vec::new();
        for (id, counts, values) in [
            (
                0,
                STD_LUMA_DC_CODE_LENGTHS.as_slice(),
                STD_LUMA_DC_VALUES.as_slice(),
            ),
            (
                1,
                STD_CHROMA_DC_CODE_LENGTHS.as_slice(),
                STD_CHROMA_DC_VALUES.as_slice(),
            ),
            (
                16,
                STD_LUMA_AC_CODE_LENGTHS.as_slice(),
                STD_LUMA_AC_VALUES.as_slice(),
            ),
            (
                17,
                STD_CHROMA_AC_CODE_LENGTHS.as_slice(),
                STD_CHROMA_AC_VALUES.as_slice(),
            ),
        ] {
            tables.push(id);
            tables.extend_from_slice(counts);
            tables.extend_from_slice(values);
        }
        segment(&mut out, 0xc4, &tables);
        segment(&mut out, 0xda, &[3, 1, 0, 2, 0x11, 3, 0x11, 0, 63, 0]);
        let dc = [
            codes(&STD_LUMA_DC_CODE_LENGTHS, &STD_LUMA_DC_VALUES),
            codes(&STD_CHROMA_DC_CODE_LENGTHS, &STD_CHROMA_DC_VALUES),
        ];
        let ac = [
            codes(&STD_LUMA_AC_CODE_LENGTHS, &STD_LUMA_AC_VALUES),
            codes(&STD_CHROMA_AC_CODE_LENGTHS, &STD_CHROMA_AC_VALUES),
        ];
        let mut basis = [[0.; 8]; 8];
        for (x, row) in basis.iter_mut().enumerate() {
            for (u, b) in row.iter_mut().enumerate() {
                *b =
                    0.5 * if u == 0 {
                        std::f64::consts::FRAC_1_SQRT_2
                    } else {
                        1.
                    } * ((2 * x + 1) as f64 * u as f64 * std::f64::consts::PI / 16.).cos();
            }
        }
        let mut bits = Bits {
            out,
            byte: 0,
            used: 0,
        };
        let mut pred = [0; 3];
        for by in (0..h).step_by(8) {
            crate::dsp::check_cancel(cancel)?;
            for bx in (0..w).step_by(8) {
                for (c, pred) in pred.iter_mut().enumerate() {
                    let table = usize::from(c != 0);
                    let mut samples = [[0.; 8]; 8];
                    for (y, row) in samples.iter_mut().enumerate() {
                        for (x, s) in row.iter_mut().enumerate() {
                            let i = ((by + y).min(h - 1) * w + (bx + x).min(w - 1)) * 3;
                            let [r, g, b] = [rgb[i], rgb[i + 1], rgb[i + 2]].map(f64::from);
                            *s = match c {
                                0 => 0.299 * r + 0.587 * g + 0.114 * b - 128.,
                                1 => -0.168736 * r - 0.331264 * g + 0.5 * b,
                                _ => 0.5 * r - 0.418688 * g - 0.081312 * b,
                            };
                        }
                    }
                    let mut tmp = [[0.; 8]; 8];
                    for y in 0..8 {
                        for u in 0..8 {
                            for x in 0..8 {
                                tmp[y][u] += samples[y][x] * basis[x][u];
                            }
                        }
                    }
                    let mut coeff = [0i32; 64];
                    for v in 0..8 {
                        for u in 0..8 {
                            let n: f64 = (0..8).map(|y| tmp[y][u] * basis[y][v]).sum();
                            coeff[v * 8 + u] = (n / f64::from(q[table][v * 8 + u])).round() as i32;
                        }
                    }
                    let diff = coeff[0] - *pred;
                    *pred = coeff[0];
                    bits.signed(diff, &dc[table], 0);
                    let mut run = 0;
                    for &z in &Z[1..] {
                        let n = coeff[z];
                        if n == 0 {
                            run += 1;
                            continue;
                        }
                        while run >= 16 {
                            bits.symbol(&ac[table], 0xf0);
                            run -= 16;
                        }
                        bits.signed(n, &ac[table], run);
                        run = 0;
                    }
                    if run > 0 {
                        bits.symbol(&ac[table], 0);
                    }
                }
            }
        }
        if bits.used > 0 {
            let n = 8 - bits.used;
            bits.put((1 << n) - 1, n);
        }
        bits.out.extend_from_slice(&[255, 217]);
        Ok(bits.out)
    }
}
#[cfg(not(feature = "plot-export"))]
pub(super) use core::encode;
