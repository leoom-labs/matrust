//! Opaque vector EPS from the same generated usvg tree as other plot exports.
//! Adobe EPSF3.0/DSC3.0 envelope; no raster fallback, external resources or PS text.
use crate::{
    Result,
    plot::{BaseTheme, Canvas, Figure},
};
use resvg::{
    tiny_skia::{PathSegment, Transform},
    usvg,
};
use std::{io::Write, path::Path, sync::atomic::AtomicBool};

const MAX_OUTPUT: usize = 128 * 1024 * 1024;
const MAX_NODES: usize = 262144;
const MAX_SEGMENTS: usize = 4194304;
const MAX_DEPTH: usize = 64;

/// True vector full-canvas EPS. Canvas units are PostScript points (1/72 inch).
/// Text uses resolved outline geometry; unsupported paints/glyphs fail atomically.
/// SVG parse/font resolution and OS I/O calls are individually nonpreemptible.
pub fn export_eps_with_cancel(
    figure: &Figure,
    path: &Path,
    canvas: Canvas,
    cancel: &AtomicBool,
) -> Result<()> {
    export_eps_in_theme_with_cancel(figure, path, canvas, BaseTheme::Light, cancel)
}
/// Explicit host context; used by the desktop without changing document intent.
pub(super) fn export_eps_in_theme_with_cancel(
    figure: &Figure,
    path: &Path,
    canvas: Canvas,
    host: BaseTheme,
    cancel: &AtomicBool,
) -> Result<()> {
    let svg = figure.svg_on_canvas_with_theme_and_cancel(canvas, host, cancel)?;
    super::check_cancel(cancel)?;
    let tree =
        usvg::Tree::from_str(&svg, &super::svg_render_options()).map_err(|e| e.to_string())?;
    drop(svg);
    super::check_cancel(cancel)?;
    let bytes = encode(&tree, figure.resolve_appearance(host).canvas, cancel)?;
    crate::files::atomic_write_with_cancel(path, &bytes, cancel)
}

#[derive(Clone, Copy)]
struct Limits {
    bytes: usize,
    nodes: usize,
    segments: usize,
    depth: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            bytes: MAX_OUTPUT,
            nodes: MAX_NODES,
            segments: MAX_SEGMENTS,
            depth: MAX_DEPTH,
        }
    }
}
struct Output {
    bytes: Vec<u8>,
    length: usize,
    line: usize,
    emit: bool,
    limit: usize,
}
impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let next = self
            .length
            .checked_add(bytes.len())
            .filter(|&n| n <= self.limit)
            .ok_or_else(|| std::io::Error::other("EPS exceeds128MiB encoded limit"))?;
        for &byte in bytes {
            if byte == b'\n' {
                self.line = 0;
            } else {
                if !(32..=126).contains(&byte) {
                    return Err(std::io::Error::other("Non-ASCII EPS token"));
                }
                self.line += 1;
                if self.line > 255 {
                    return Err(std::io::Error::other("EPS line exceeds255bytes"));
                }
            }
        }
        if self.emit {
            if next > self.bytes.capacity() {
                let capacity = next
                    .max(self.bytes.capacity().saturating_mul(2))
                    .min(self.limit);
                self.bytes
                    .try_reserve_exact(capacity - self.bytes.len())
                    .map_err(|_| std::io::Error::other("Could not allocate bounded EPS output"))?;
            }
            self.bytes.extend_from_slice(bytes);
        }
        self.length = next;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
macro_rules! put {
    ($out:expr, $($arg:tt)*) => { ($out).write_fmt(format_args!($($arg)*)).map_err(|e| e.to_string())? };
}
struct Encoder<'a> {
    out: Output,
    limits: Limits,
    nodes: usize,
    segments: usize,
    check: &'a mut dyn FnMut() -> Result<()>,
}
fn number(v: f64) -> Result<f64> {
    if !v.is_finite() || v.abs() > 1e12 || (v != 0. && v.abs() < 1e-30) {
        return Err("EPS coordinate/style outside supported finite range".into());
    }
    Ok(v)
}
fn transform(t: Transform) -> Result<Transform> {
    for n in [t.sx, t.ky, t.kx, t.sy, t.tx, t.ty] {
        number(f64::from(n))?;
    }
    let det = f64::from(t.sx) * f64::from(t.sy) - f64::from(t.kx) * f64::from(t.ky);
    if det == 0. || !det.is_finite() {
        return Err("EPS singular transform is unsupported".into());
    }
    Ok(t)
}
fn plain(group: &usvg::Group) -> Result<()> {
    if group.opacity().get() != 1.
        || group.blend_mode() != usvg::BlendMode::Normal
        || group.isolate()
        || group.mask().is_some()
        || !group.filters().is_empty()
    {
        return Err("EPS requires opaque normal groups without isolation, masks or filters".into());
    }
    Ok(())
}
fn solid(paint: &usvg::Paint, opacity: f32) -> Result<usvg::Color> {
    match paint {
        usvg::Paint::Color(c) if opacity == 1. => Ok(*c),
        _ => Err(
            "EPS supports opaque solid colors only; no gradient/pattern/transparency fallback"
                .into(),
        ),
    }
}
impl Encoder<'_> {
    fn visit(&mut self, depth: usize) -> Result<()> {
        (self.check)()?;
        self.nodes = self.nodes.checked_add(1).ok_or("EPS node overflow")?;
        if depth > self.limits.depth || self.nodes > self.limits.nodes {
            return Err("EPS tree exceeds node/depth limit".into());
        }
        Ok(())
    }
    fn concat(&mut self, t: Transform) -> Result<()> {
        transform(t)?;
        put!(
            self.out,
            "[{:.9e} {:.9e} {:.9e} {:.9e} {:.9e} {:.9e}] concat\n",
            t.sx,
            t.ky,
            t.kx,
            t.sy,
            t.tx,
            t.ty
        );
        Ok(())
    }
    fn color(&mut self, c: usvg::Color) -> Result<()> {
        put!(
            self.out,
            "{:.9e} {:.9e} {:.9e} setrgbcolor\n",
            f64::from(c.red) / 255.,
            f64::from(c.green) / 255.,
            f64::from(c.blue) / 255.
        );
        Ok(())
    }
    fn point(&self, p: [f64; 2], t: Transform) -> Result<[f64; 2]> {
        number(p[0])?;
        number(p[1])?;
        number(f64::from(t.sx) * p[0] + f64::from(t.kx) * p[1] + f64::from(t.tx))?;
        number(f64::from(t.ky) * p[0] + f64::from(t.sy) * p[1] + f64::from(t.ty))?;
        Ok(p)
    }
    fn path(&mut self, path: &usvg::Path, t: Transform) -> Result<()> {
        put!(self.out, "newpath\n");
        let mut current = [0.; 2];
        let mut start = [0.; 2];
        for segment in path.data().segments() {
            if self.segments.is_multiple_of(1024) {
                (self.check)()?;
            }
            self.segments = self.segments.checked_add(1).ok_or("EPS segment overflow")?;
            if self.segments > self.limits.segments {
                return Err("EPS segment limit exceeded".into());
            }
            let p = |p: resvg::tiny_skia::Point| [f64::from(p.x), f64::from(p.y)];
            match segment {
                PathSegment::MoveTo(q) => {
                    let q = self.point(p(q), t)?;
                    put!(self.out, "{:.9e} {:.9e} moveto\n", q[0], q[1]);
                    current = q;
                    start = q;
                }
                PathSegment::LineTo(q) => {
                    let q = self.point(p(q), t)?;
                    put!(self.out, "{:.9e} {:.9e} lineto\n", q[0], q[1]);
                    current = q;
                }
                PathSegment::QuadTo(q, end) => {
                    let q = self.point(p(q), t)?;
                    let end = self.point(p(end), t)?;
                    let (a, b) = quadratic(current, q, end);
                    let a = self.point(a, t)?;
                    let b = self.point(b, t)?;
                    put!(
                        self.out,
                        "{:.9e} {:.9e} {:.9e} {:.9e} {:.9e} {:.9e} curveto\n",
                        a[0],
                        a[1],
                        b[0],
                        b[1],
                        end[0],
                        end[1]
                    );
                    current = end;
                }
                PathSegment::CubicTo(a, b, end) => {
                    let a = self.point(p(a), t)?;
                    let b = self.point(p(b), t)?;
                    let end = self.point(p(end), t)?;
                    put!(
                        self.out,
                        "{:.9e} {:.9e} {:.9e} {:.9e} {:.9e} {:.9e} curveto\n",
                        a[0],
                        a[1],
                        b[0],
                        b[1],
                        end[0],
                        end[1]
                    );
                    current = end;
                }
                PathSegment::Close => {
                    put!(self.out, "closepath\n");
                    current = start;
                }
            }
        }
        Ok(())
    }
    fn paint(&mut self, path: &usvg::Path, t: Transform, fill: bool) -> Result<()> {
        if fill {
            if let Some(f) = path.fill() {
                self.color(solid(f.paint(), f.opacity().get())?)?;
                self.path(path, t)?;
                put!(
                    self.out,
                    "{}\n",
                    if f.rule() == usvg::FillRule::EvenOdd {
                        "eofill"
                    } else {
                        "fill"
                    }
                );
            }
        } else if let Some(s) = path.stroke() {
            self.color(solid(s.paint(), s.opacity().get())?)?;
            let join = match s.linejoin() {
                usvg::LineJoin::Miter => 0,
                usvg::LineJoin::Round => 1,
                usvg::LineJoin::Bevel => 2,
                usvg::LineJoin::MiterClip => {
                    return Err("EPS miter-clip join is unsupported".into());
                }
            };
            let cap = match s.linecap() {
                usvg::LineCap::Butt => 0,
                usvg::LineCap::Round => 1,
                usvg::LineCap::Square => 2,
            };
            let width = number(f64::from(s.width().get()))?;
            let miter = number(f64::from(s.miterlimit().get()))?;
            put!(
                self.out,
                "{width:.9e} setlinewidth\n{cap} setlinecap\n{join} setlinejoin\n{miter:.9e} setmiterlimit\n[\n"
            );
            if let Some(dash) = s.dasharray() {
                if dash.len() > 256 {
                    return Err("EPS dash array exceeds256 entries".into());
                }
                for &d in dash {
                    put!(self.out, "{:.9e}\n", number(f64::from(d))?);
                }
            }
            put!(
                self.out,
                "] {:.9e} setdash\n",
                number(f64::from(s.dashoffset()))?
            );
            self.path(path, t)?;
            put!(self.out, "stroke\n");
        }
        Ok(())
    }
    fn group(&mut self, group: &usvg::Group, parent: Transform, depth: usize) -> Result<()> {
        self.visit(depth)?;
        plain(group)?;
        let t = transform(parent.pre_concat(group.transform()))?;
        put!(self.out, "gsave\n");
        self.concat(group.transform())?;
        if let Some(clip) = group.clip_path() {
            self.clip(clip, t, depth + 1)?;
        }
        for node in group.children() {
            match node {
                usvg::Node::Group(g) => self.group(g, t, depth + 1)?,
                usvg::Node::Text(text) => self.group(text.flattened(), t, depth + 1)?,
                usvg::Node::Path(path) => {
                    self.visit(depth + 1)?;
                    if path.is_visible() {
                        let fill_first = path.paint_order() == usvg::PaintOrder::FillAndStroke;
                        self.paint(path, t, fill_first)?;
                        self.paint(path, t, !fill_first)?;
                    }
                }
                usvg::Node::Image(_) => {
                    return Err(
                        "EPS image/color-glyph nodes are unsupported; no raster fallback".into(),
                    );
                }
            }
        }
        put!(self.out, "grestore\n");
        Ok(())
    }
    fn clip_shape<'a>(
        &mut self,
        group: &'a usvg::Group,
        parent: Transform,
        depth: usize,
        found: &mut Option<(&'a usvg::Path, Transform)>,
    ) -> Result<()> {
        self.visit(depth)?;
        plain(group)?;
        if group.clip_path().is_some() {
            return Err("EPS clipped groups inside clip paths are unsupported".into());
        }
        let t = transform(parent.pre_concat(group.transform()))?;
        for child in group.children() {
            match child {
                usvg::Node::Group(g) => self.clip_shape(g, t, depth + 1, found)?,
                usvg::Node::Path(p) => {
                    self.visit(depth + 1)?;
                    if p.is_visible() {
                        if found.is_some() {
                            return Err("EPS multiple clip shapes require unsupported union".into());
                        }
                        *found = Some((p, t));
                    }
                }
                _ => return Err("EPS clip text/images are unsupported".into()),
            }
        }
        Ok(())
    }
    fn clip(&mut self, clip: &usvg::ClipPath, parent: Transform, depth: usize) -> Result<()> {
        self.visit(depth)?;
        // An additional clip is in the referring group's coordinates, not the
        // transformed coordinates of this clip. Each intersection restores CTM.
        if let Some(other) = clip.clip_path() {
            self.clip(other, parent, depth + 1)?;
        }
        let mut found = None;
        self.clip_shape(
            clip.root(),
            transform(clip.transform())?,
            depth + 1,
            &mut found,
        )?;
        let (path, relative) = found.ok_or("EPS empty clip tree is unsupported")?;
        let fill = path.fill().ok_or("EPS clip has no fill rule")?;
        let combined = transform(parent.pre_concat(relative))?;
        put!(self.out, "matrix currentmatrix\n");
        self.concat(relative)?;
        self.path(path, combined)?;
        put!(
            self.out,
            "{} newpath\nsetmatrix\n",
            if fill.rule() == usvg::FillRule::EvenOdd {
                "eoclip"
            } else {
                "clip"
            }
        );
        Ok(())
    }
}
fn quadratic(p: [f64; 2], q: [f64; 2], end: [f64; 2]) -> ([f64; 2], [f64; 2]) {
    (
        [
            p[0] + (q[0] - p[0]) * 2. / 3.,
            p[1] + (q[1] - p[1]) * 2. / 3.,
        ],
        [
            end[0] + (q[0] - end[0]) * 2. / 3.,
            end[1] + (q[1] - end[1]) * 2. / 3.,
        ],
    )
}
fn encode(tree: &usvg::Tree, background: [u8; 3], cancel: &AtomicBool) -> Result<Vec<u8>> {
    encode_checked(tree, background, Limits::default(), &mut || {
        super::check_cancel(cancel)
    })
}
fn encode_checked(
    tree: &usvg::Tree,
    background: [u8; 3],
    limits: Limits,
    checkpoint: &mut dyn FnMut() -> Result<()>,
) -> Result<Vec<u8>> {
    let width = f64::from(tree.size().width());
    let height = f64::from(tree.size().height());
    Canvas {
        width: tree.size().width(),
        height: tree.size().height(),
    }
    .validate()?;
    // First pass validates/counts actual output without allocating its byte vector.
    // Second pass borrows the same immutable tree and cannot observe new geometry.
    let mut bytes = Vec::new();
    for emit in [false, true] {
        checkpoint()?;
        let mut e = Encoder {
            out: Output {
                bytes: Vec::new(),
                length: 0,
                line: 0,
                emit,
                limit: limits.bytes,
            },
            limits,
            nodes: 0,
            segments: 0,
            check: checkpoint,
        };
        put!(
            e.out,
            "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 {:.0} {:.0}\n%%HiResBoundingBox: 0 0 {width:.9e} {height:.9e}\n%%LanguageLevel: 3\n%%Pages: 1\n%%Creator: MatRust vector EPS\n%%DocumentData: Clean7Bit\n%%EndComments\n%%Page: 1 1\ngsave\n",
            width.ceil(),
            height.ceil()
        );
        put!(
            e.out,
            "0 {height:.9e} translate\n1 -1 scale\nnewpath\n0 0 moveto\n{width:.9e} 0 lineto\n{width:.9e} {height:.9e} lineto\n0 {height:.9e} lineto\nclosepath\nclip newpath\n"
        );
        // Opaque full-canvas background makes BoundingBox exact even for empty
        // figures. No crop/transparent-tree public API in this first milestone.
        e.color(usvg::Color {
            red: background[0],
            green: background[1],
            blue: background[2],
        })?;
        put!(e.out, "0 0 {width:.9e} {height:.9e} rectfill\n");
        e.group(tree.root(), Transform::identity(), 0)?;
        put!(e.out, "grestore\nshowpage\n%%Trailer\n%%EOF\n");
        (e.check)()?;
        if emit {
            bytes = e.out.bytes;
        }
    }
    Ok(bytes)
}

#[cfg(test)]
#[path = "eps_tests.rs"]
mod tests;
