use super::{Metadata, VideoSource, admit, buffer, jpeg};
use crate::Result;
use std::{
    collections::BTreeSet,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};
const BAD: &str = "VideoReader: truncated or corrupt AVI file.";
const MAX_FRAMES: usize = 1_000_000;
#[derive(Clone, Copy)]
struct Chunk {
    id: [u8; 4],
    data: u64,
    size: u32,
    next: u64,
}
#[derive(Clone, Copy)]
struct Frame {
    data: u64,
    size: u32,
}
struct Palette {
    data: u64,
    colors: Vec<[u8; 3]>,
}
pub(super) struct Avi {
    path: PathBuf,
    meta: Metadata,
    frames: Vec<Frame>,
    top_down: bool,
    mjpeg: bool,
    palettes: Vec<Palette>,
    max_palette_len: usize,
    palette_bytes: usize,
}
fn io<T>(r: std::io::Result<T>) -> Result<T> {
    r.map_err(|e| format!("{BAD} {e}"))
}
fn at(f: &mut File, pos: u64, n: usize) -> Result<Vec<u8>> {
    let mut b = vec![0; n];
    io(f.seek(SeekFrom::Start(pos)))?;
    io(f.read_exact(&mut b))?;
    Ok(b)
}
fn u16le(b: &[u8], p: usize) -> u16 {
    u16::from_le_bytes([b[p], b[p + 1]])
}
fn u32le(b: &[u8], p: usize) -> u32 {
    u32::from_le_bytes(b[p..p + 4].try_into().unwrap())
}
fn u64le(b: &[u8], p: usize) -> u64 {
    u64::from_le_bytes(b[p..p + 8].try_into().unwrap())
}
fn chunk(f: &mut File, pos: u64, end: u64) -> Result<Chunk> {
    if end.saturating_sub(pos) < 8 {
        return Err(BAD.into());
    }
    let b = at(f, pos, 8)?;
    let size = u32le(&b, 4);
    let next = pos
        .checked_add(8 + u64::from(size) + (u64::from(size) & 1))
        .ok_or(BAD)?;
    if next > end {
        return Err(BAD.into());
    }
    Ok(Chunk {
        id: b[..4].try_into().unwrap(),
        data: pos + 8,
        size,
        next,
    })
}
fn push(v: &mut Vec<Frame>, x: Frame, budget: usize) -> Result<()> {
    if v.len() >= MAX_FRAMES {
        return Err("VideoReader: AVI frame index exceeds its limit".into());
    }
    admit((v.len() + 1) * 64, budget)?;
    v.try_reserve(1)
        .map_err(|_| "VideoReader: unable to allocate frame index")?;
    v.push(x);
    Ok(())
}
fn is_video(id: &[u8; 4], stream: usize) -> bool {
    is_stream(id, stream) && (&id[2..] == b"db" || &id[2..] == b"dc")
}
fn is_stream(id: &[u8; 4], stream: usize) -> bool {
    id[0] == b'0' + (stream / 10) as u8 && id[1] == b'0' + (stream % 10) as u8
}
fn initial_palette(b: &[u8]) -> Result<Vec<[u8; 3]>> {
    if u16le(b, 14) != 8 || b[16..20] != [0; 4] {
        return Ok(Vec::new());
    }
    let n = match u32le(b, 32) {
        0 => 256,
        n => n as usize,
    };
    let start = u32le(b, 0) as usize;
    if n > 256 || start < 40 || start > b.len() || n * 4 > b.len() - start {
        return Err(BAD.into());
    }
    Ok(b[start..start + n * 4]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|x| [x[2], x[1], x[0]])
        .collect())
}
struct Parser<'a> {
    f: File,
    end: u64,
    budget: usize,
    cancel: &'a AtomicBool,
    stream: Option<usize>,
    streams: usize,
    header: Option<Vec<u8>>,
    format: Option<Vec<u8>>,
    avih: bool,
    frames: Vec<Frame>,
    movi: Vec<(u64, u64)>,
    idx: Vec<Chunk>,
    odml: Vec<Chunk>,
    chunks: usize,
    palette: Vec<[u8; 3]>,
    palette_dirty: bool,
    palettes: Vec<Palette>,
}
impl Parser<'_> {
    fn palette_change(&mut self, c: Chunk) -> Result<()> {
        const ERROR: &str = "VideoReader: malformed AVI palette-change chunk";
        if c.size < 4 || c.size > 4 + 256 * 4 {
            return Err(ERROR.into());
        }
        let b = at(&mut self.f, c.data, c.size as usize)?;
        let first = usize::from(b[0]);
        let count = if b[1] == 0 { 256 } else { usize::from(b[1]) };
        if first + count > 256 || b.len() != 4 + count * 4 || u16le(&b, 2) != 0 {
            return Err(ERROR.into());
        }
        // Only indexed BI_RGB uses palettes; unrelated streams are ignored.
        if self.palette.is_empty() {
            return Ok(());
        }
        if self.palette.len() < first + count {
            self.palette.resize(first + count, [0; 3]);
        }
        for (dst, entry) in self.palette[first..first + count]
            .iter_mut()
            .zip(b[4..].as_chunks::<4>().0)
        {
            *dst = [entry[0], entry[1], entry[2]];
        }
        self.palette_dirty = true;
        Ok(())
    }
    fn walk(&mut self, mut pos: u64, end: u64, depth: usize, in_movi: bool) -> Result<()> {
        if depth > 16 {
            return Err(BAD.into());
        }
        while pos < end {
            crate::dsp::check_cancel(self.cancel)?;
            self.chunks += 1;
            admit(
                self.chunks * 64 + self.palettes.capacity() * 1024,
                self.budget,
            )?;
            if self.chunks > 2_000_000 {
                return Err("VideoReader: AVI chunk limit exceeded".into());
            }
            let c = chunk(&mut self.f, pos, end)?;
            if c.id == *b"LIST" {
                if c.size < 4 {
                    return Err(BAD.into());
                }
                let kind = at(&mut self.f, c.data, 4)?;
                match kind.as_slice() {
                    b"strl" => {
                        self.strl(c)?;
                    }
                    b"movi" => {
                        self.movi.push((c.data, c.data + u64::from(c.size)));
                        self.walk(c.data + 4, c.data + u64::from(c.size), depth + 1, true)?;
                    }
                    b"hdrl" | b"rec " | b"odml" => {
                        self.walk(c.data + 4, c.data + u64::from(c.size), depth + 1, in_movi)?
                    }
                    _ => {}
                }
            } else if c.id == *b"avih" {
                if c.size < 56 {
                    return Err(BAD.into());
                }
                self.avih = true;
            } else if c.id == *b"idx1" {
                self.idx.push(c);
            } else if &c.id[..2] == b"ix" {
                self.odml.push(c);
            } else if in_movi && self.stream.is_some_and(|s| is_video(&c.id, s)) {
                push(
                    &mut self.frames,
                    Frame {
                        data: c.data,
                        size: c.size,
                    },
                    self.budget,
                )?;
                // Coalesce any number of pc chunks between frames. Never
                // retain more than one bounded palette snapshot per frame.
                if self.palette_dirty {
                    if self.palettes.len() == self.palettes.capacity() {
                        let capacity = (self.palettes.len() + 1).next_power_of_two();
                        admit(self.chunks * 64 + capacity * 1024, self.budget)?;
                        self.palettes
                            .try_reserve_exact(capacity - self.palettes.len())
                            .map_err(|_| "VideoReader: unable to allocate palette index")?;
                    }
                    self.palettes.push(Palette {
                        data: c.data,
                        colors: self.palette.clone(),
                    });
                    self.palette_dirty = false;
                }
            } else if in_movi
                && &c.id[2..] == b"pc"
                && self.stream.is_some_and(|s| is_stream(&c.id, s))
            {
                self.palette_change(c)?;
            }
            pos = c.next;
        }
        Ok(())
    }
    fn strl(&mut self, c: Chunk) -> Result<()> {
        let number = self.streams;
        self.streams += 1;
        if number >= 100 {
            return Err(BAD.into());
        }
        let mut p = c.data + 4;
        let end = c.data + u64::from(c.size);
        let mut selected = false;
        while p < end {
            crate::dsp::check_cancel(self.cancel)?;
            self.chunks += 1;
            admit(self.chunks * 64, self.budget)?;
            let x = chunk(&mut self.f, p, end)?;
            match &x.id {
                b"strh" => {
                    if x.size < 56 {
                        return Err(BAD.into());
                    }
                    let h = at(&mut self.f, x.data, 56)?;
                    if &h[..4] == b"vids" && self.stream.is_none() {
                        self.stream = Some(number);
                        selected = true;
                        self.header = Some(h);
                    }
                }
                b"strf" if selected => {
                    if x.size < 40 || x.size > 4096 {
                        return Err(BAD.into());
                    }
                    self.format = Some(at(&mut self.f, x.data, x.size as usize)?);
                    self.palette = initial_palette(self.format.as_ref().unwrap())?;
                }
                b"indx" if selected => self.odml.push(x),
                _ => {}
            }
            p = x.next;
        }
        Ok(())
    }
    fn indexed_frame(&mut self, pos: u64, id: [u8; 4], size: u32) -> Result<Option<Frame>> {
        if !self.movi.iter().any(|&(a, b)| {
            pos >= a + 4 && pos.checked_add(8 + u64::from(size)).is_some_and(|e| e <= b)
        }) {
            return Ok(None);
        }
        let Ok(c) = chunk(&mut self.f, pos, self.end) else {
            return Ok(None);
        };
        Ok((c.id == id && c.size == size).then_some(Frame { data: c.data, size }))
    }
    fn old_index(&mut self, c: Chunk) -> Result<Vec<Frame>> {
        if !c.size.is_multiple_of(16) {
            return Err(BAD.into());
        }
        let mut frames = Vec::new();
        let mut bases = None;
        for n in 0..c.size / 16 {
            crate::dsp::check_cancel(self.cancel)?;
            let b = at(&mut self.f, c.data + u64::from(n) * 16, 16)?;
            let id = b[..4].try_into().unwrap();
            if !is_video(&id, self.stream.ok_or(BAD)?) {
                continue;
            }
            let off = u64::from(u32le(&b, 8));
            let size = u32le(&b, 12);
            let candidates = if let Some(base) = bases {
                vec![base]
            } else {
                let mut v = vec![0];
                for &(m, _) in &self.movi {
                    v.push(m);
                    v.push(m + 4);
                }
                v
            };
            let mut found = None;
            for base in candidates {
                if let Some(p) = base.checked_add(off)
                    && let Some(x) = self.indexed_frame(p, id, size)?
                {
                    bases = Some(base);
                    found = Some(x);
                    break;
                }
            }
            // Preserve a physical dropped chunk's position for palette lookup.
            // Index-only dropped slots may have no meaningful offset at all.
            let frame = found.or_else(|| (size == 0).then_some(Frame { data: 0, size: 0 }));
            push(&mut frames, frame.ok_or(BAD)?, self.budget)?;
        }
        Ok(frames)
    }
    fn open_index(
        &mut self,
        c: Chunk,
        visited: &mut BTreeSet<u64>,
        out: &mut Vec<Frame>,
        depth: usize,
    ) -> Result<()> {
        admit((visited.len() + 1) * 64, self.budget)?;
        if depth > 2 || !visited.insert(c.data) {
            return Err(BAD.into());
        }
        if c.size < 24 {
            return Err(BAD.into());
        }
        let b = at(&mut self.f, c.data, 24)?;
        let longs = u16le(&b, 0);
        let count = u32le(&b, 4);
        let id: [u8; 4] = b[8..12].try_into().unwrap();
        if !is_video(&id, self.stream.ok_or(BAD)?) {
            return Ok(());
        }
        if b[2] != 0 {
            return Err("VideoReader: OpenDML field indexes are not supported; other codecs need ffmpeg (7b3).".into());
        }
        let stride = match (b[3], longs) {
            (0, 4) => 16,
            (1, 2) => 8,
            _ => return Err(BAD.into()),
        };
        if u64::from(count) * stride + 24 > u64::from(c.size) {
            return Err(BAD.into());
        }
        if count as usize > MAX_FRAMES {
            return Err(BAD.into());
        }
        for n in 0..count {
            crate::dsp::check_cancel(self.cancel)?;
            let e = at(
                &mut self.f,
                c.data + 24 + u64::from(n) * stride,
                stride as usize,
            )?;
            if b[3] == 0 {
                let pos = u64le(&e, 0);
                let sz = u32le(&e, 8);
                if pos == 0 && sz == 0 {
                    continue;
                }
                let sub = chunk(&mut self.f, pos, self.end)?;
                if sub.size.checked_add(8) != Some(sz) || &sub.id[..2] != b"ix" {
                    return Err(BAD.into());
                }
                self.open_index(sub, visited, out, depth + 1)?;
            } else {
                let size = u32le(&e, 4) & 0x7fff_ffff;
                let pos = u64le(&b, 12)
                    .checked_add(u64::from(u32le(&e, 0)))
                    .and_then(|p| p.checked_sub(8));
                let frame = match pos {
                    Some(pos) => self.indexed_frame(pos, id, size)?,
                    None => None,
                };
                let x = frame
                    .or_else(|| (size == 0).then_some(Frame { data: 0, size: 0 }))
                    .ok_or(BAD)?;
                push(out, x, self.budget)?;
            }
        }
        Ok(())
    }
}
pub(super) fn open(path: &Path, budget: usize, cancel: &AtomicBool) -> Result<Avi> {
    let f = super::file_io::open_input(path)?;
    let end = io(f.metadata())?.len();
    let mut p = Parser {
        f,
        end,
        budget,
        cancel,
        stream: None,
        streams: 0,
        header: None,
        format: None,
        avih: false,
        frames: Vec::new(),
        movi: Vec::new(),
        idx: Vec::new(),
        odml: Vec::new(),
        chunks: 0,
        palette: Vec::new(),
        palette_dirty: false,
        palettes: Vec::new(),
    };
    let mut pos = 0;
    while pos < end {
        let c = chunk(&mut p.f, pos, end)?;
        if c.id != *b"RIFF" || c.size < 4 {
            return Err(BAD.into());
        }
        let kind = at(&mut p.f, c.data, 4)?;
        if (pos == 0 && kind != b"AVI ") || (pos != 0 && kind != b"AVIX") {
            return Err("VideoReader: expected RIFF AVI or AVIX container.".into());
        }
        p.walk(c.data + 4, c.data + u64::from(c.size), 0, false)?;
        pos = c.next;
    }
    if !p.avih || p.movi.is_empty() {
        return Err(BAD.into());
    }
    let h = p
        .header
        .as_ref()
        .ok_or("VideoReader: AVI has no video stream")?;
    let b = p.format.as_ref().ok_or(BAD)?;
    let scale = u32le(h, 20);
    let rate = u32le(h, 24);
    if scale == 0 || rate == 0 {
        return Err(BAD.into());
    }
    let width = u32le(b, 4) as i32;
    let height = u32le(b, 8) as i32;
    let bits = u16le(b, 14);
    let compression = &b[16..20];
    if width <= 0
        || height == 0
        || height == i32::MIN
        || u16le(b, 12) != 1
        || u32le(b, 0) < 40
        || u32le(b, 0) as usize > b.len()
    {
        return Err(BAD.into());
    }
    let width = width as usize;
    let top_down = height < 0;
    let height = height.unsigned_abs() as usize;
    let pixels = width.checked_mul(height).ok_or(BAD)?;
    admit(pixels.checked_mul(32).ok_or(BAD)?, budget)?;
    let mjpeg = matches!(compression, b"MJPG" | b"mjpg" | b"AVRn" | b"dmb1");
    if !mjpeg && (compression != [0; 4] || ![8, 24, 32].contains(&bits)) {
        let name = if compression == [0; 4] {
            format!("BI_RGB/{bits}")
        } else {
            String::from_utf8_lossy(compression).into_owned()
        };
        return Err(format!(
            "VideoReader: unsupported codec '{name}'; other codecs need ffmpeg (7b3)."
        ));
    }
    let palette = initial_palette(b)?;
    let format = if mjpeg || bits != 8 {
        "RGB24"
    } else if p.palettes.is_empty()
        && palette.len() == 256
        && palette.iter().enumerate().all(|(i, x)| *x == [i as u8; 3])
    {
        "Grayscale"
    } else {
        "Indexed"
    };
    let mut odml = Vec::new();
    let mut visited = BTreeSet::new();
    for c in p.odml.clone() {
        if !visited.contains(&c.data) {
            p.open_index(c, &mut visited, &mut odml, 0)?;
        }
    }
    if !odml.is_empty() {
        if odml.len() < p.frames.len()
            || odml.iter().filter(|f| f.size != 0).count()
                != p.frames.iter().filter(|f| f.size != 0).count()
        {
            return Err("VideoReader: incomplete OpenDML frame index".into());
        }
        p.frames = odml;
    } else if !p.idx.is_empty() {
        let mut indexed = Vec::new();
        for c in p.idx.clone() {
            for x in p.old_index(c)? {
                push(&mut indexed, x, budget)?;
            }
        }
        // Legacy idx1 may cover only the first RIFF; append AVIX frames.
        if indexed.len() < p.frames.len() {
            let last = indexed.last().map_or(0, |f| f.data);
            for x in p.frames.iter().filter(|f| f.data > last) {
                push(&mut indexed, *x, budget)?;
            }
        }
        if indexed.len() < p.frames.len()
            || indexed.iter().filter(|f| f.size != 0).count()
                != p.frames.iter().filter(|f| f.size != 0).count()
        {
            return Err(BAD.into());
        }
        p.frames = indexed;
    }
    let frames = p.frames;
    if frames.is_empty() || frames[0].size == 0 {
        return Err("VideoReader: no decodable first frame in AVI file.".into());
    }
    Ok(Avi {
        path: path.to_owned(),
        meta: Metadata {
            width,
            height,
            rate: f64::from(rate) / f64::from(scale),
            frames: frames.len(),
            duration: None,
            bits: if mjpeg { 24 } else { bits },
            format,
            palette,
        },
        frames,
        top_down,
        mjpeg,
        max_palette_len: p.palette.len(),
        palette_bytes: p.palettes.capacity() * std::mem::size_of::<Palette>()
            + p.palettes
                .iter()
                .map(|p| p.colors.capacity() * 3)
                .sum::<usize>(),
        palettes: p.palettes,
    })
}
impl VideoSource for Avi {
    fn metadata(&self) -> &Metadata {
        &self.meta
    }
    fn bytes(&self) -> usize {
        self.frames.capacity() * std::mem::size_of::<Frame>()
            + self.palette_bytes
            + self.meta.palette.len() * 3
            + self.path.as_os_str().len()
    }
    fn palette(&self, mut i: usize) -> &[[u8; 3]] {
        // Index-only dropped slots inherit the preceding frame's palette.
        while self.frames[i].data == 0 && i > 0 {
            i -= 1;
        }
        let n = self
            .palettes
            .partition_point(|p| p.data <= self.frames[i].data);
        if n == 0 {
            &self.meta.palette
        } else {
            &self.palettes[n - 1].colors
        }
    }
    fn max_palette_len(&self) -> usize {
        self.max_palette_len
    }
    fn frame(&self, mut i: usize, budget: usize, cancel: &AtomicBool) -> Result<Vec<u8>> {
        crate::dsp::check_cancel(cancel)?;
        if i >= self.frames.len() {
            return Err("No more frames available to read from file.".into());
        }
        let palette = self.palette(i);
        while self.frames[i].size == 0 {
            i = i.checked_sub(1).ok_or(BAD)?;
        }
        let x = self.frames[i];
        let mut f = super::file_io::open_input(&self.path)?;
        let m = &self.meta;
        let pixels = m.width * m.height;
        let input = x.size as usize;
        admit(input.checked_add(pixels * 32).ok_or(BAD)?, budget)?;
        if self.mjpeg {
            let b = at(&mut f, x.data, input)?;
            return jpeg::decode(&b, m.width, m.height, budget, cancel);
        }
        let channels = if m.bits == 8 { 1 } else { 3 };
        let step = usize::from(m.bits / 8);
        let stride = (m.width * step + 3) & !3;
        if input < stride.checked_mul(m.height).ok_or(BAD)? {
            return Err(BAD.into());
        }
        let mut row = buffer(stride, budget)?;
        let mut out = buffer(pixels * channels, budget)?;
        io(f.seek(SeekFrom::Start(x.data)))?;
        for y in 0..m.height {
            crate::dsp::check_cancel(cancel)?;
            io(f.read_exact(&mut row))?;
            let dst_y = if self.top_down { y } else { m.height - 1 - y };
            for x in 0..m.width {
                let d = (dst_y * m.width + x) * channels;
                if channels == 1 {
                    let k = row[x];
                    if usize::from(k) >= palette.len() {
                        return Err(BAD.into());
                    }
                    out[d] = k;
                } else {
                    out[d..d + 3].copy_from_slice(&[
                        row[x * step + 2],
                        row[x * step + 1],
                        row[x * step],
                    ]);
                }
            }
        }
        Ok(out)
    }
}
