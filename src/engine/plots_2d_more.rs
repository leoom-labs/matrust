//! Missing 2-D charts: histogram, errorbar, area, barh, pie, fill, polarplot,
//! fplot and heatmap, plus histcounts name-value options; contourc, contour,
//! contourf and clabel (module `contour`).
//!
//! Every chart is composed from the existing series kinds (Line, Patch) and
//! figure textboxes, so the desktop and SVG renderers draw them unchanged.
//! Chart-level properties (BinEdges, CapSize, ...) live in `P2State`, keyed by
//! the identity of the chart's primary series, whose handle the call returns.
//! Helper series (error-bar segments, outlines, polar grid) carry empty
//! DisplayNames and `legend_hidden`, so legends list one entry per chart.
pub(crate) mod contour;
mod geom;
mod gscatter;

use super::poly_interp::{auto_edges, bin_picker, real_matrix};
use super::{Engine, Execution, Value};
use crate::plot::{
    AutoPalette, Axes, BaseTheme, FaceColorMode, Figure, LinePattern, PaletteCycle, PaletteVersion,
    PatchFaces, PlotKind, Series, TextAlign, TextBox, colormap,
};
use crate::{Array, GraphicsArray, GraphicsHandle, GraphicsKind, Matrix, ObjectIdentity, Result};
use geom::*;
use std::sync::Arc;

pub(super) const P2_NAMES: &[&str] = &[
    "histogram",
    "errorbar",
    "area",
    "barh",
    "pie",
    "fill",
    "polarplot",
    "fplot",
    "heatmap",
    "contourc",
    "contour",
    "contourf",
    "contour3",
    "clabel",
    "gscatter",
];

pub(super) fn handles(name: &str) -> bool {
    P2_NAMES.contains(&name)
}

/// Names a statement draws without showing a result: every chart. contourc
/// is a plain function (`contourc(Z)` displays and sets ans).
pub(super) fn statement(name: &str) -> bool {
    handles(name) && name != "contourc"
}

/// Chart metadata, bounded: dead records are pruned on every publication.
#[derive(Clone, Default)]
pub(crate) struct P2State {
    charts: Vec<P2Chart>,
    /// (axes, textbox) pairs: labels a chart owns, removed when its axes is replaced.
    texts: Vec<(ObjectIdentity, ObjectIdentity)>,
    polar: Vec<P2Polar>,
}
const P2_MAX_RECORDS: usize = 1024;

#[derive(Clone)]
struct P2Chart {
    primary: ObjectIdentity,
    helpers: Vec<ObjectIdentity>,
    kind: P2Kind,
}

#[derive(Clone)]
struct P2Hist {
    data: Matrix,
    bins: P2Bins,
    stairs: bool,
    /// Face colour before the FaceAlpha tint; bars are drawn tinted from it.
    face: [u8; 3],
    face_alpha: f64,
    edges: Vec<f64>,
    counts: Vec<f64>,
    values: Vec<f64>,
}

#[derive(Clone)]
struct P2Err {
    x: Vec<f64>,
    y: Vec<f64>,
    d: P2Deltas,
    cap: f64,
}

#[derive(Clone)]
struct P2Heat {
    cdata: Matrix,
    limits: [f64; 2],
    labels: bool,
}

#[derive(Clone)]
enum P2Kind {
    Histogram(Box<P2Hist>),
    ErrorBar(Box<P2Err>),
    Area {
        x: Vec<f64>,
        y: Vec<f64>,
        base: f64,
    },
    BarH {
        x: Vec<f64>,
        y: Vec<f64>,
        width: f64,
        stacked: bool,
        base: f64,
    },
    Fill,
    FPlot {
        function: crate::FunctionHandle,
        range: [f64; 2],
        density: usize,
    },
    Heatmap(Box<P2Heat>),
    PolarLine {
        theta: Vec<f64>,
        rho: Vec<f64>,
    },
    Contour(Box<contour::ContourRec>),
}
impl P2Kind {
    fn type_name(&self) -> &'static str {
        match self {
            Self::Histogram(_) => "histogram",
            Self::ErrorBar(_) => "errorbar",
            Self::Area { .. } => "area",
            Self::BarH { .. } => "bar",
            Self::Fill => "patch",
            Self::FPlot { .. } => "functionline",
            Self::Heatmap(_) => "heatmap",
            Self::PolarLine { .. } => "line",
            Self::Contour(_) => "contour",
        }
    }
}

#[derive(Clone)]
struct P2Polar {
    axes: ObjectIdentity,
    rlim: Option<[f64; 2]>,
    zero: &'static str,
    clockwise: bool,
    /// The polar grid series. The axes is polar only while it still holds this
    /// series: any plot that replaces the axes contents (plot, bar, histogram,
    /// cla, ...) drops the grid and so ends polar mode. None only while
    /// polarplot builds a new polar axes, before its first refresh.
    grid: Option<ObjectIdentity>,
}
impl P2Polar {
    /// Whether this record still describes `axes`.
    fn live_on(&self, axes: &Axes) -> bool {
        self.axes.same_object(&axes.identity)
            && self
                .grid
                .as_ref()
                .is_none_or(|g| axes.series.iter().any(|s| s.identity.same_object(g)))
    }
    fn orient(&self) -> P2Orient {
        let zero = match self.zero {
            "top" => 90f64,
            "left" => 180.,
            "bottom" => 270.,
            _ => 0.,
        }
        .to_radians();
        P2Orient {
            zero,
            dir: if self.clockwise { -1. } else { 1. },
        }
    }
}

// ------------------------------------------------------------------ helpers

fn p2_text(e: &Engine, value: &Value) -> Option<String> {
    match value {
        Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
        Value::Container(Array::Character(c)) => c.to_utf8(&e.cancel).ok(),
        Value::Container(Array::Str(s)) if s.len() == 1 => s.data()[0].as_deref().map(Into::into),
        _ => None,
    }
}

fn p2_vec(value: &Value, who: &str) -> Result<Vec<f64>> {
    Ok(real_matrix(value, who)?.data().to_vec())
}

fn p2_scalar(value: &Value, who: &str, what: &str) -> Result<f64> {
    let m = real_matrix(value, who)?;
    if m.len() != 1 || !m.data()[0].is_finite() {
        return Err(format!("{who}: {what} must be a finite scalar"));
    }
    Ok(m.data()[0])
}

fn p2_pair(value: &Value, who: &str, what: &str) -> Result<[f64; 2]> {
    let v = p2_vec(value, who)?;
    if v.len() != 2 || !v.iter().all(|x| x.is_finite()) || v[0] >= v[1] {
        return Err(format!(
            "{who}: {what} must be an increasing finite two-element vector"
        ));
    }
    Ok([v[0], v[1]])
}

fn p2_on_off(e: &Engine, value: &Value, who: &str) -> Result<bool> {
    match p2_text(e, value).map(|s| s.to_ascii_lowercase()).as_deref() {
        Some("on") => Ok(true),
        Some("off") => Ok(false),
        _ => Err(format!("{who}: expected 'on' or 'off'")),
    }
}

/// Colour letter, colour name or RGB triple in 0..1.
fn p2_color(e: &Engine, value: &Value) -> Result<[u8; 3]> {
    if let Some(s) = p2_text(e, value) {
        return Ok(match s.to_ascii_lowercase().as_str() {
            "r" | "red" => [255, 0, 0],
            "g" | "green" => [0, 255, 0],
            "b" | "blue" => [0, 0, 255],
            "c" | "cyan" => [0, 255, 255],
            "m" | "magenta" => [255, 0, 255],
            "y" | "yellow" => [255, 255, 0],
            "k" | "black" => [0, 0, 0],
            "w" | "white" => [255, 255, 255],
            _ => {
                return Err(format!(
                    "Unknown colour '{s}'; use a colour letter, name or RGB triple"
                ));
            }
        });
    }
    let v = p2_vec(value, "color").map_err(|_| "Colour must be a letter, name or RGB triple")?;
    if v.len() != 3 || v.iter().any(|c| !c.is_finite() || !(0. ..=1.).contains(c)) {
        return Err("Colour must be a letter, name or RGB triple with values in 0..1".into());
    }
    Ok([0, 1, 2].map(|k| (v[k] * 255.).round() as u8))
}

/// 'none' or a colour.
fn p2_edge(e: &Engine, value: &Value) -> Result<Option<[u8; 3]>> {
    if p2_text(e, value).is_some_and(|s| s.eq_ignore_ascii_case("none")) {
        return Ok(None);
    }
    p2_color(e, value).map(Some)
}

#[derive(Clone, Copy)]
enum P2FillFace {
    Rgb([u8; 3]),
    Flat,
    Interp,
    None,
}

fn p2_fill_face(e: &Engine, value: &Value) -> Result<P2FillFace> {
    if let Some(s) = p2_text(e, value) {
        if s.eq_ignore_ascii_case("flat") {
            return Ok(P2FillFace::Flat);
        }
        if s.eq_ignore_ascii_case("none") {
            return Ok(P2FillFace::None);
        }
        if s.eq_ignore_ascii_case("interp") {
            return Ok(P2FillFace::Interp);
        }
    }
    p2_color(e, value).map(P2FillFace::Rgb)
}

#[derive(Clone, Copy)]
enum P2FillColor {
    Rgb([u8; 3]),
    Scalar(f64),
}

fn p2_fill_color(e: &Engine, value: &Value) -> Result<P2FillColor> {
    if p2_text(e, value).is_none()
        && let Ok(m) = real_matrix(value, "fill")
        && m.len() == 1
    {
        let v = m.data()[0];
        if !v.is_finite() {
            return Err("fill CData must be finite".into());
        }
        return Ok(P2FillColor::Scalar(v));
    }
    p2_color(e, value).map(P2FillColor::Rgb)
}

/// Per-vertex fill CData: a numeric column with one value per vertex of a
/// vector polygon. A 3x1 column for a polygon of another size stays an RGB
/// triple; any other column length is rejected before an object is created.
fn p2_fill_vertex_cdata(e: &Engine, value: &Value, x: &Matrix) -> Result<Option<Vec<f64>>> {
    if p2_text(e, value).is_some() || (x.rows() > 1 && x.cols() > 1) {
        return Ok(None);
    }
    let Ok(m) = real_matrix(value, "fill") else {
        return Ok(None);
    };
    if m.cols() != 1 || m.rows() < 2 {
        return Ok(None);
    }
    if m.rows() != x.len() {
        if m.rows() == 3 {
            return Ok(None);
        }
        return Err(format!(
            "fill per-vertex CData has {} values for {} vertices",
            m.rows(),
            x.len()
        ));
    }
    if m.data().iter().any(|v| !v.is_finite()) {
        return Err("fill CData must be finite".into());
    }
    Ok(Some(m.data().to_vec()))
}

fn p2_rgb_value(rgb: [u8; 3]) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_slice(
        1,
        3,
        &rgb.map(|c| f64::from(c) / 255.),
    )?))
}

fn p2_row(values: &[f64]) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_slice(
        1,
        values.len(),
        values,
    )?))
}

fn p2_str(s: &str) -> Value {
    Value::Text(s.into())
}

fn p2_line_width(value: &Value) -> Result<f64> {
    let w = p2_scalar(value, "LineWidth", "LineWidth")?;
    crate::plot::validate_line_width(w)?;
    Ok(w)
}

fn p2_line_style(e: &Engine, value: &Value) -> Result<LinePattern> {
    Ok(match p2_text(e, value).as_deref() {
        Some("-") => LinePattern::Solid,
        Some("--") => LinePattern::Dashed,
        Some(":") => LinePattern::Dotted,
        Some("-.") => LinePattern::DashDot,
        Some("none") => LinePattern::None,
        _ => return Err("LineStyle must be '-', '--', ':', '-.' or 'none'".into()),
    })
}

/// Split positional arguments from trailing Name,Value pairs: the pairs start
/// at the first text argument that names one of `names` (case-insensitive).
fn p2_split<'a>(e: &Engine, args: &'a [Value], names: &[&str]) -> (&'a [Value], &'a [Value]) {
    let start = args
        .iter()
        .position(|a| {
            p2_text(e, a).is_some_and(|s| names.iter().any(|n| n.eq_ignore_ascii_case(&s)))
        })
        .unwrap_or(args.len());
    (&args[..start], &args[start..])
}

fn p2_pairs<'a>(e: &Engine, rest: &'a [Value], who: &str) -> Result<Vec<(String, &'a Value)>> {
    if !rest.len().is_multiple_of(2) || rest.len() > 64 {
        return Err(format!("{who}: name-value arguments must come in pairs"));
    }
    rest.chunks(2)
        .map(|p| {
            let name = p2_text(e, &p[0]).ok_or(format!("{who}: property names must be text"))?;
            Ok((name.to_ascii_lowercase(), &p[1]))
        })
        .collect()
}

/// Plot-box geometry (normalized figure coordinates and nominal pixels) and
/// automatic colours of the axes a chart draws into.
struct P2Ctx {
    norm: [f64; 4],
    px: [f64; 2],
    palette: Option<PaletteCycle>,
    used: u8,
    base_index: usize,
    grid: [u8; 3],
    text: [u8; 3],
}
impl P2Ctx {
    fn of(figure: &Figure, theme: BaseTheme) -> Self {
        let norm = figure
            .normalized_axes_positions()
            .into_iter()
            .find(|(p, _)| *p == figure.active_position())
            .map_or(
                [70. / 960., 70. / 560., 850. / 960., 430. / 560.],
                |(_, r)| r,
            );
        let appearance = figure.resolve_appearance(theme);
        let colors = figure.axes.resolve_colors(appearance);
        let alpha = colors.grid_alpha.clamp(0., 1.);
        let grid = [0, 1, 2].map(|k| {
            (f64::from(colors.grid_x[k]) * alpha + f64::from(colors.background[k]) * (1. - alpha))
                .round() as u8
        });
        Self {
            norm,
            px: [norm[2] * 960., norm[3] * 560.],
            palette: figure.axes.palette_cycle,
            used: 0,
            base_index: figure.axes.series.len(),
            grid,
            text: appearance.text,
        }
    }
    /// Next automatic colour: its palette slot (for line-like series) and RGB.
    fn next_color(&mut self) -> (Option<AutoPalette>, [u8; 3]) {
        let k = self.used;
        self.used += 1;
        match self.palette {
            Some(cycle) => {
                let slot = (cycle.next_slot + k) % 7;
                (
                    Some(AutoPalette {
                        version: cycle.version,
                        slot,
                    }),
                    cycle.version.color(slot, BaseTheme::Light),
                )
            }
            None => (
                None,
                crate::plot::COLORS[(self.base_index + usize::from(k)) % 7],
            ),
        }
    }
    /// Limits that give equal data units per pixel in both directions at the
    /// nominal 960-by-560 figure size, covering [-rx, rx] x [-ry, ry].
    fn equal_limits(&self, rx: f64, ry: f64) -> ([f64; 2], [f64; 2]) {
        let unit = (rx / self.px[0]).max(ry / self.px[1]);
        let (hx, hy) = (unit * self.px[0], unit * self.px[1]);
        ([-hx, hx], [-hy, hy])
    }
    /// A label centred (or aligned) on a data point, in normalized figure units.
    #[allow(clippy::too_many_arguments)]
    fn label(
        &self,
        text: &str,
        at: [f64; 2],
        xlim: [f64; 2],
        ylim: [f64; 2],
        align: TextAlign,
        color: [u8; 3],
        size: f64,
    ) -> Result<TextBox> {
        let font = size * 4. / 3.;
        let chars = text.chars().count().max(1) as f64;
        let w = (chars * 0.62 * font + 8.) / 960.;
        let h = font * 1.45 / 560.;
        let nx = self.norm[0] + self.norm[2] * (at[0] - xlim[0]) / (xlim[1] - xlim[0]);
        let ny = self.norm[1] + self.norm[3] * (at[1] - ylim[0]) / (ylim[1] - ylim[0]);
        let left = match align {
            TextAlign::Left => nx - 4. / 960.,
            TextAlign::Center => nx - w / 2.,
            TextAlign::Right => nx - w + 4. / 960.,
        };
        // The first baseline sits one em below the box top; centre the glyphs.
        let bottom = ny + 0.35 * font / 560. + font / 560. - h;
        let left = left.clamp(0., 1. - w.min(1.));
        let bottom = bottom.clamp(0., 1. - h.min(1.));
        let mut tb = TextBox::new([left, bottom, w.min(1.), h.min(1.)], text.into())?;
        tb.horizontal = align;
        tb.color = color;
        tb.font_size = size;
        Ok(tb)
    }
}

fn p2_patch(points: Vec<[f64; 2]>, vertices: usize, color: [u8; 3]) -> Result<Series> {
    let mut s = Series::new(points, PlotKind::Patch);
    s.patch = Some(PatchFaces::new(vertices)?);
    s.color = color;
    s.explicit_color = true;
    s.pattern = LinePattern::None;
    Ok(s)
}

fn p2_line(points: Vec<[f64; 2]>, color: (Option<AutoPalette>, [u8; 3])) -> Series {
    let mut s = Series::new(points, PlotKind::Line);
    s.color = color.1;
    s.auto_palette = color.0;
    s.explicit_color = color.0.is_none();
    s
}

fn p2_decor(points: Vec<[f64; 2]>, rgb: [u8; 3], width: f64) -> Series {
    let mut s = Series::new(points, PlotKind::Line);
    s.color = rgb;
    s.explicit_color = true;
    s.line_width = Some(width);
    s.legend_hidden = true;
    s
}

/// Blend toward white: an opaque stand-in for FaceAlpha on a white background.
fn p2_tint(rgb: [u8; 3], alpha: f64) -> [u8; 3] {
    rgb.map(|c| (f64::from(c) * alpha + 255. * (1. - alpha)).round() as u8)
}

/// One new chart object: series (primary first) and an optional record.
struct P2New {
    series: Vec<Series>,
    kind: Option<P2Kind>,
}

#[derive(Default)]
struct P2Draw {
    charts: Vec<P2New>,
    decor: Vec<Series>,
    texts: Vec<TextBox>,
    /// Chart i's handle is followed by text handle `text_after[i]`, if any (pie).
    text_after: Vec<Option<usize>>,
    xlim: Option<[f64; 2]>,
    ylim: Option<[f64; 2]>,
    title: Option<String>,
    xlabel: Option<String>,
    ylabel: Option<String>,
    ticks_hidden: bool,
}

impl Engine {
    /// The figure a new chart draws into: the current axes when held, else a
    /// fresh axes that keeps identity, placement and layout (like plot).
    fn p2_target(&self) -> Figure {
        if let Some(f) = self.figure.as_ref().filter(|f| f.hold) {
            return f.clone();
        }
        let old = self.figure.as_ref();
        Figure {
            identity: old.map_or_else(ObjectIdentity::default, |f| f.identity.clone()),
            axes: Axes {
                palette_cycle: old.map_or(
                    Some(PaletteCycle::new(PaletteVersion::MatRustModern1)),
                    |f| f.axes.palette_cycle.map(PaletteCycle::reset),
                ),
                identity: old.map_or_else(ObjectIdentity::default, |f| f.axes.identity.clone()),
                placement: old.and_then(|f| f.axes.placement),
                ..Axes::default()
            },
            layout: old.and_then(|f| f.layout.clone()),
            textboxes: p2_kept_textboxes(self),
            annotations: old.map_or_else(Vec::new, |f| f.annotations.clone()),
            name: old.map_or_else(String::new, |f| f.name.clone()),
            color: old.map_or([255; 3], |f| f.color),
            invert_hardcopy: old.and_then(|f| f.invert_hardcopy),
            alphamap: old.and_then(|f| f.alphamap.clone()),
            slice_viewers: Vec::new(),
            orthoslice_viewers: Vec::new(),
            appearance: old.map_or(Some(crate::plot::FigureAppearance::default()), |f| {
                f.appearance
            }),
            position: old.map_or(crate::plot::DEFAULT_FIGURE_POSITION, |f| f.position),
            units: old.map_or(Default::default(), |f| f.units),
            number_title: old.is_none_or(|f| f.number_title),
            visible: old.is_none_or(|f| f.visible),
            resize: old.is_none_or(|f| f.resize),
            window_state: old.map_or(Default::default(), |f| f.window_state),
        }
    }

    /// Admit, publish and record a drawn chart set; returns the handles.
    fn p2_publish(&mut self, mut figure: Figure, ctx: &P2Ctx, draw: P2Draw) -> Result<Value> {
        self.check_cancel()?;
        let axes_id = figure.axes.identity.clone();
        let mut series = Vec::new();
        let mut roles = Vec::new(); // (chart index, is primary)
        for (i, chart) in draw.charts.iter().enumerate() {
            for (k, s) in chart.series.iter().enumerate() {
                let mut s = s.clone();
                // Helpers are never legend members (IconDisplayStyle 'off').
                s.legend_hidden = k != 0;
                series.push(s);
                roles.push((Some(i), k == 0));
            }
        }
        for s in &draw.decor {
            let mut s = s.clone();
            s.legend_hidden = true;
            series.push(s);
            roles.push((None, false));
        }
        let first = figure.axes.series.len();
        let labels: Vec<String> = series.iter().map(|s| s.label.clone()).collect();
        figure.add(series)?;
        for (offset, (chart, primary)) in roles.iter().enumerate() {
            let s = &mut figure.axes.series[first + offset];
            if !primary || chart.is_none() {
                s.label = labels[offset].clone();
            }
        }
        if let Some(cycle) = &mut figure.axes.palette_cycle
            && let Some(start) = ctx.palette
        {
            cycle.next_slot = (start.next_slot + ctx.used) % 7;
        }
        if draw.xlim.is_some() {
            figure.axes.xlim = draw.xlim;
        }
        if draw.ylim.is_some() {
            figure.axes.ylim = draw.ylim;
        }
        if draw.ticks_hidden {
            figure.axes.ticks_hidden = true;
        }
        if let Some(t) = draw.title {
            figure.axes.title = t;
        }
        if let Some(t) = draw.xlabel {
            figure.axes.xlabel = t;
        }
        if let Some(t) = draw.ylabel {
            figure.axes.ylabel = t;
        }
        if figure.textboxes.len() + draw.texts.len() > crate::plot::MAX_TEXTBOXES {
            return Err("Chart labels exceed the figure's 64 textboxes".into());
        }
        let text_ids: Vec<ObjectIdentity> = draw.texts.iter().map(|t| t.identity.clone()).collect();
        figure.textboxes.extend(draw.texts);
        super::plot_properties::refresh_scatter_clim(&mut figure.axes);
        super::plot_properties::refresh_scalar_patch_colors(&mut figure.axes);
        figure.validate()?;
        // Records and handles.
        let mut handles = Vec::new();
        let mut offset = first;
        let mut records = Vec::new();
        for (i, chart) in draw.charts.into_iter().enumerate() {
            let ids: Vec<ObjectIdentity> = figure.axes.series[offset..offset + chart.series.len()]
                .iter()
                .map(|s| s.identity.clone())
                .collect();
            offset += chart.series.len();
            handles.push(Arc::new(GraphicsHandle::series(
                &figure.identity,
                &axes_id,
                &ids[0],
            )));
            if let Some(Some(t)) = draw.text_after.get(i) {
                handles.push(Arc::new(GraphicsHandle::textbox(
                    &figure.identity,
                    &text_ids[*t],
                )));
            }
            if let Some(kind) = chart.kind {
                records.push(P2Chart {
                    primary: ids[0].clone(),
                    helpers: ids[1..].to_vec(),
                    kind,
                });
            }
        }
        self.check_cancel()?;
        self.figure = Some(figure);
        self.p2.charts.extend(records);
        self.p2
            .texts
            .extend(text_ids.into_iter().map(|t| (axes_id.clone(), t)));
        self.p2_prune();
        let count = handles.len();
        Ok(Value::Container(
            GraphicsArray::from_shared(count, 1, handles, &self.cancel)?.into(),
        ))
    }

    /// Forget records whose objects no longer exist in any figure.
    fn p2_prune(&mut self) {
        let live_series: Vec<ObjectIdentity> = p2_all_figures(self)
            .flat_map(|(_, f)| f.all_axes().flat_map(|(_, a)| a.series.iter()))
            .map(|s| s.identity.clone())
            .collect();
        let live_texts: Vec<ObjectIdentity> = p2_all_figures(self)
            .flat_map(|(_, f)| f.textboxes.iter().map(|t| t.identity.clone()))
            .collect();
        let has =
            |list: &[ObjectIdentity], id: &ObjectIdentity| list.iter().any(|x| x.same_object(id));
        self.p2.charts.retain(|c| has(&live_series, &c.primary));
        self.p2.texts.retain(|(_, t)| has(&live_texts, t));
        // A polar record goes with its axes and with its grid (replaced contents).
        let mut polar = std::mem::take(&mut self.p2.polar);
        polar
            .retain(|p| p2_all_figures(self).any(|(_, f)| f.all_axes().any(|(_, a)| p.live_on(a))));
        self.p2.polar = polar;
        for list_len in [self.p2.charts.len(), self.p2.texts.len()] {
            debug_assert!(list_len <= P2_MAX_RECORDS * 4);
        }
        if self.p2.charts.len() > P2_MAX_RECORDS {
            let excess = self.p2.charts.len() - P2_MAX_RECORDS;
            self.p2.charts.drain(..excess);
        }
    }
}

/// Classic figures plus uiaxes backing figures.
fn p2_all_figures(e: &Engine) -> impl Iterator<Item = (u32, &Figure)> {
    e.figures()
        .chain(e.ui.uiaxes_figures.iter().map(|(&id, f)| (id, f)))
        .chain(e.ui.held_classic.iter().map(|(id, f)| (*id, f)))
}

/// Figure textboxes to keep when the current axes is replaced by a new plot:
/// every textbox except the labels a chart placed on that axes.
/// Whether `id` is a series of a contour chart: contourf bands are not
/// low-level `patch` objects and do not count against their face bound.
pub(super) fn p2_contour_series(e: &Engine, id: &ObjectIdentity) -> bool {
    e.p2.charts.iter().any(|c| {
        matches!(c.kind, P2Kind::Contour(_))
            && (c.primary.same_object(id) || c.helpers.iter().any(|h| h.same_object(id)))
    })
}

pub(super) fn p2_kept_textboxes(e: &Engine) -> Vec<TextBox> {
    let Some(f) = e.figure.as_ref() else {
        return Vec::new();
    };
    f.textboxes
        .iter()
        .filter(|t| {
            !e.p2
                .texts
                .iter()
                .any(|(a, id)| a.same_object(&f.axes.identity) && id.same_object(&t.identity))
        })
        .cloned()
        .collect()
}

// ---------------------------------------------------------------- dispatch

pub(super) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    e.check_cancel()?;
    if contour::CONTOUR_NAMES.contains(&name) {
        return contour::call(e, name, args, count, out);
    }
    if count > 1 {
        return Err(format!("{name} returns at most one output"));
    }
    let mut args = if name == "gscatter" {
        args.to_vec()
    } else {
        e.text_arguments(args.to_vec())?
    };
    // A leading axes handle selects that axes first.
    if let Some(Value::Container(Array::Graphics(h))) = args.first()
        && h.len() == 1
        && h.scalar_handle()?.kind() == GraphicsKind::Axes
    {
        let target = args.remove(0);
        let Value::Container(Array::Graphics(h)) = &target else {
            unreachable!()
        };
        let h = h.scalar_handle()?;
        if super::ui_components::is_uiaxes_fig_id(e.locate_graphics(h)?.figure) {
            // axes(uiaxes) cannot select a uiaxes host; draw through it instead.
            return e.with_axes(h, |e| call(e, name, &args, count, out));
        }
        e.call("axes", vec![target], out)?;
    }
    if args.is_empty() {
        return Err(format!("{name} requires input data"));
    }
    let old = (e.figure.clone(), e.p2.clone());
    let result = match name {
        "histogram" => e.p2_histogram(&args),
        "errorbar" => e.p2_errorbar(&args),
        "area" => e.p2_area(&args),
        "barh" => e.p2_barh(&args),
        "pie" => e.p2_pie(&args),
        "fill" => e.p2_fill(&args),
        "polarplot" => e.p2_polarplot(&args),
        "fplot" => e.p2_fplot(&args, out),
        "heatmap" => e.p2_heatmap(&args),
        "gscatter" => gscatter::run(e, &args),
        _ => Err(format!("{name} is not a chart builtin")),
    };
    match result {
        Ok(v) => Ok(if count == 0 { vec![] } else { vec![v] }),
        Err(error) => {
            (e.figure, e.p2) = old;
            Err(error)
        }
    }
}

// ---------------------------------------------------------------- histogram

const HIST_NAMES: &[&str] = &[
    "NumBins",
    "BinEdges",
    "BinWidth",
    "BinLimits",
    "BinMethod",
    "Normalization",
    "DisplayStyle",
    "FaceColor",
    "EdgeColor",
    "FaceAlpha",
    "DisplayName",
    "LineWidth",
];

fn p2_rules() -> P2Rules {
    P2Rules {
        auto: auto_edges,
        picker: bin_picker,
    }
}

/// Apply one binning name-value pair; Ok(false) when the name is not binning.
fn p2_bin_option(e: &Engine, bins: &mut P2Bins, name: &str, value: &Value) -> Result<bool> {
    match name {
        "numbins" => {
            let n = p2_scalar(value, "histogram", "NumBins")?;
            if n < 1. || n.fract() != 0. || n > P2_MAX_BINS as f64 {
                return Err(format!("NumBins must be an integer in 1..{P2_MAX_BINS}"));
            }
            bins.nbins = Some(n as usize);
            bins.edges = None;
            bins.width = None;
        }
        "binedges" => {
            bins.edges = Some(p2_vec(value, "histogram")?);
            bins.nbins = None;
            bins.width = None;
            bins.limits = None;
        }
        "binwidth" => {
            let w = p2_scalar(value, "histogram", "BinWidth")?;
            if w <= 0. {
                return Err("BinWidth must be positive".into());
            }
            bins.width = Some(w);
            bins.nbins = None;
            bins.edges = None;
        }
        "binlimits" => {
            let v = p2_vec(value, "histogram")?;
            if v.len() != 2 || !v.iter().all(|x| x.is_finite()) || v[0] > v[1] {
                return Err("BinLimits must be a finite nondecreasing two-element vector".into());
            }
            bins.limits = Some([v[0], v[1]]);
            bins.edges = None;
        }
        "binmethod" => {
            bins.method = P2Method::parse(&p2_text(e, value).ok_or("BinMethod must be text")?)?;
            bins.nbins = None;
            bins.edges = None;
            bins.width = None;
        }
        "normalization" => {
            bins.norm = P2Norm::parse(&p2_text(e, value).ok_or("Normalization must be text")?)?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn p2_bin(data: &Matrix, bins: &P2Bins) -> Result<(Vec<f64>, Vec<f64>, Vec<f64>)> {
    let edges = p2_edges(data.data(), bins, &p2_rules())?;
    let counts = p2_counts(data.data(), &edges);
    let values = p2_normalize(&counts, &edges, data.len(), bins.norm);
    Ok((edges, counts, values))
}

/// histcounts(X, ..., Name, Value): NumBins, BinEdges, BinWidth, BinLimits,
/// BinMethod and Normalization.
pub(super) fn p2_histcounts(e: &Engine, args: &[Value], nout: usize) -> Result<Vec<Value>> {
    let args = e.text_arguments(args.to_vec())?;
    let x = real_matrix(&args[0], "histcounts")?;
    let (positional, rest) = p2_split(e, &args[1..], HIST_NAMES);
    let mut bins = P2Bins::default();
    match positional {
        [] => {}
        [v] => {
            let m = real_matrix(v, "histcounts")?;
            if m.len() == 1 {
                p2_bin_option(e, &mut bins, "numbins", v)?;
            } else {
                p2_bin_option(e, &mut bins, "binedges", v)?;
            }
        }
        _ => return Err("histcounts: too many positional arguments".into()),
    }
    for (name, value) in p2_pairs(e, rest, "histcounts")? {
        if !p2_bin_option(e, &mut bins, &name, value)? {
            return Err(format!(
                "histcounts: unsupported option '{name}'; use NumBins, BinEdges, BinWidth, BinLimits, BinMethod or Normalization"
            ));
        }
    }
    let (edges, _, values) = p2_bin(&x, &bins)?;
    let mut out = vec![p2_row(&values)?];
    if nout > 1 {
        out.push(p2_row(&edges)?);
    }
    Ok(out)
}

impl Engine {
    fn p2_hist_series(
        h: &P2Hist,
        face: [u8; 3],
        edge: Option<[u8; 3]>,
        auto: Option<AutoPalette>,
    ) -> Result<Series> {
        if h.stairs {
            let mut s = p2_line(p2_bin_outline(&h.edges, &h.values), (auto, face));
            if auto.is_none() {
                s.explicit_color = true;
            }
            return Ok(s);
        }
        let mut s = p2_patch(
            p2_bin_quads(&h.edges, &h.values, false),
            4,
            p2_tint(face, h.face_alpha),
        )?;
        s.face_edge_color = edge;
        s.pattern = if edge.is_some() {
            LinePattern::Solid
        } else {
            LinePattern::None
        };
        Ok(s)
    }

    fn p2_histogram(&mut self, args: &[Value]) -> Result<Value> {
        let data = real_matrix(&args[0], "histogram")?;
        if data.len() > crate::plot::MAX_POINTS {
            return Err("histogram data exceeds 1000000 elements".into());
        }
        let (positional, rest) = p2_split(self, &args[1..], HIST_NAMES);
        let mut bins = P2Bins::default();
        match positional {
            [] => {}
            [v] => {
                let m = real_matrix(v, "histogram")?;
                let key = if m.len() == 1 { "numbins" } else { "binedges" };
                p2_bin_option(self, &mut bins, key, v)?;
            }
            _ => return Err("Use histogram(X), histogram(X,nbins) or histogram(X,edges) followed by Name,Value pairs".into()),
        }
        let figure = self.p2_target();
        let mut ctx = P2Ctx::of(&figure, self.graphics_theme_context);
        let (auto, rgb) = ctx.next_color();
        let (mut face, mut edge, mut alpha, mut stairs, mut label, mut width) =
            (rgb, Some([0u8; 3]), 0.6, false, String::new(), None);
        for (name, value) in p2_pairs(self, rest, "histogram")? {
            if p2_bin_option(self, &mut bins, &name, value)? {
                continue;
            }
            match name.as_str() {
                "displaystyle" => {
                    stairs = match p2_text(self, value)
                        .map(|s| s.to_ascii_lowercase())
                        .as_deref()
                    {
                        Some("bar") => false,
                        Some("stairs") => true,
                        _ => return Err("DisplayStyle must be 'bar' or 'stairs'".into()),
                    }
                }
                "facecolor" => face = p2_color(self, value)?,
                "edgecolor" => edge = p2_edge(self, value)?,
                "facealpha" => {
                    alpha = p2_scalar(value, "histogram", "FaceAlpha")?;
                    if !(0. ..=1.).contains(&alpha) {
                        return Err("FaceAlpha must be in 0..1".into());
                    }
                }
                "displayname" => label = p2_text(self, value).ok_or("DisplayName must be text")?,
                "linewidth" => width = Some(p2_line_width(value)?),
                _ => return Err(format!("histogram: unsupported property '{name}'")),
            }
        }
        let (edges, counts, values) = p2_bin(&data, &bins)?;
        if edges.len() - 1 > crate::plot::MAX_PATCH_FACES {
            return Err("histogram draws at most 10000 bins".into());
        }
        let hist = P2Hist {
            data,
            bins,
            stairs,
            face,
            face_alpha: alpha,
            edges,
            counts,
            values,
        };
        let explicit_face = face != rgb;
        let mut s =
            Self::p2_hist_series(&hist, face, edge, if explicit_face { None } else { auto })?;
        s.label = label;
        s.line_width = width;
        figure.axes.validate()?;
        let draw = P2Draw {
            charts: vec![P2New {
                series: vec![s],
                kind: Some(P2Kind::Histogram(Box::new(hist))),
            }],
            ..P2Draw::default()
        };
        self.p2_publish(figure, &ctx, draw)
    }
}

// ---------------------------------------------------------------- errorbar

const ERR_NAMES: &[&str] = &["CapSize", "Color", "LineWidth", "DisplayName", "LineStyle"];

fn p2_columns(m: &Matrix) -> Vec<Vec<f64>> {
    if m.rows() == 1 || m.cols() == 1 {
        return vec![m.data().to_vec()];
    }
    m.data().chunks(m.rows()).map(<[f64]>::to_vec).collect()
}

impl Engine {
    fn p2_errorbar(&mut self, args: &[Value]) -> Result<Value> {
        let (positional, rest) = p2_split(self, args, ERR_NAMES);
        let mut numeric = Vec::new();
        let mut orientation: Option<String> = None;
        let mut spec = None;
        for a in positional {
            match p2_text(self, a) {
                Some(t)
                    if matches!(
                        t.to_ascii_lowercase().as_str(),
                        "vertical" | "horizontal" | "both"
                    ) =>
                {
                    if orientation.is_some() {
                        return Err("errorbar accepts one orientation".into());
                    }
                    orientation = Some(t.to_ascii_lowercase());
                }
                Some(t) => {
                    if spec.is_some() {
                        return Err("errorbar accepts one line specification".into());
                    }
                    spec = Some(t);
                }
                None => {
                    if orientation.is_some() || spec.is_some() {
                        return Err(
                            "errorbar data must precede the orientation and line specification"
                                .into(),
                        );
                    }
                    numeric.push(real_matrix(a, "errorbar")?);
                }
            }
        }
        let n_args = numeric.len();
        let y = numeric
            .get(if n_args == 2 { 0 } else { 1 })
            .ok_or("errorbar requires Y and error data")?
            .clone();
        let x = if n_args == 2 {
            Matrix::from_column_vec(y.rows(), y.cols(), {
                let rows = if y.rows() == 1 { y.cols() } else { y.rows() };
                (0..y.len()).map(|i| (i % rows + 1) as f64).collect()
            })?
        } else {
            numeric[0].clone()
        };
        let empty = || Matrix::from_column_vec(0, 0, vec![]);
        // (neg, pos) as given: one err for both directions, or a neg/pos pair.
        let (neg, pos) = match n_args {
            2 => (numeric[1].clone(), numeric[1].clone()),
            3 => (numeric[2].clone(), numeric[2].clone()),
            4 | 6 => (numeric[2].clone(), numeric[3].clone()),
            _ => return Err("Use errorbar(y,err), errorbar(x,y,err), errorbar(x,y,neg,pos) or errorbar(x,y,yneg,ypos,xneg,xpos), each optionally followed by 'vertical', 'horizontal' or 'both'".into()),
        };
        // The orientation picks the directions that get bars. The six-input form
        // gives both directions; there an orientation keeps only the named bars.
        let (yneg, ypos, xneg, xpos) = match (n_args, orientation.as_deref()) {
            (6, None | Some("both")) => (neg, pos, numeric[4].clone(), numeric[5].clone()),
            (6, Some("horizontal")) => (empty()?, empty()?, numeric[4].clone(), numeric[5].clone()),
            (_, None | Some("vertical")) => (neg, pos, empty()?, empty()?),
            (_, Some("horizontal")) => (empty()?, empty()?, neg, pos),
            _ => (neg.clone(), pos.clone(), neg, pos),
        };
        let shape = (y.rows(), y.cols());
        let same = |m: &Matrix| {
            m.is_empty()
                || (m.rows(), m.cols()) == shape
                || (m.len() == y.len()
                    && (m.rows() == 1 || m.cols() == 1)
                    && (y.rows() == 1 || y.cols() == 1))
        };
        if !same(&x) || x.is_empty() || ![&yneg, &ypos, &xneg, &xpos].iter().all(|m| same(m)) {
            return Err("errorbar inputs must all have the same size".into());
        }
        if y.len() > 100_000 {
            return Err("errorbar draws at most 100000 points".into());
        }
        let figure = self.p2_target();
        let mut ctx = P2Ctx::of(&figure, self.graphics_theme_context);
        let (mut cap, mut color, mut width, mut label, mut style) = (6., None, None, None, None);
        for (name, value) in p2_pairs(self, rest, "errorbar")? {
            match name.as_str() {
                "capsize" => {
                    cap = p2_scalar(value, "errorbar", "CapSize")?;
                    if !(0. ..=100.).contains(&cap) {
                        return Err("CapSize must be in 0..100 points".into());
                    }
                }
                "color" => color = Some(p2_color(self, value)?),
                "linewidth" => width = Some(p2_line_width(value)?),
                "displayname" => {
                    label = Some(p2_text(self, value).ok_or("DisplayName must be text")?)
                }
                "linestyle" => style = Some(p2_line_style(self, value)?),
                _ => return Err(format!("errorbar: unsupported property '{name}'")),
            }
        }
        let (xs, ys) = (p2_columns(&x), p2_columns(&y));
        let col = |m: &Matrix, j: usize| -> Vec<f64> {
            if m.is_empty() {
                Vec::new()
            } else {
                let c = p2_columns(m);
                c[j.min(c.len() - 1)].clone()
            }
        };
        let mut charts = Vec::new();
        for j in 0..ys.len() {
            let xj = xs[j.min(xs.len() - 1)].clone();
            let yj = ys[j].clone();
            let mut d = P2Deltas {
                yneg: col(&yneg, j),
                ypos: col(&ypos, j),
                xneg: col(&xneg, j),
                xpos: col(&xpos, j),
            };
            for v in [&mut d.yneg, &mut d.ypos, &mut d.xneg, &mut d.xpos] {
                for e in v.iter_mut() {
                    *e = e.abs();
                }
            }
            let auto = ctx.next_color();
            let c = color.map_or(auto, |rgb| (None, rgb));
            let mut line = p2_line(xj.iter().zip(&yj).map(|(&a, &b)| [a, b]).collect(), c);
            if let Some(spec) = &spec {
                line.line_spec(spec)?;
            }
            if let Some(p) = style {
                line.pattern = p;
            }
            line.line_width = width;
            if let Some(l) = &label {
                line.label = l.clone();
            }
            let record = P2Err {
                x: xj,
                y: yj,
                d,
                cap,
            };
            let mut bars = p2_decor(Vec::new(), line.color, width.unwrap_or(0.5));
            bars.line_width = width;
            bars.auto_palette = line.auto_palette;
            bars.explicit_color = line.explicit_color;
            charts.push((line, bars, record));
        }
        // Caps are CapSize points long on the nominal plot box over the data span.
        let mut bx = [f64::INFINITY, f64::NEG_INFINITY];
        let mut by = bx;
        for (_, _, r) in &charts {
            let (a, b) = p2_errorbar_bounds(&r.x, &r.y, &r.d);
            bx = [bx[0].min(a[0]), bx[1].max(a[1])];
            by = [by[0].min(b[0]), by[1].max(b[1])];
        }
        let span = |b: [f64; 2]| if b[1] > b[0] { b[1] - b[0] } else { 1. };
        let unit = [span(bx) / ctx.px[0], span(by) / ctx.px[1]];
        let draw = P2Draw {
            charts: charts
                .into_iter()
                .map(|(line, mut bars, r)| {
                    let half = r.cap * 4. / 3. / 2.;
                    bars.points =
                        p2_errorbar_segments(&r.x, &r.y, &r.d, [half * unit[0], half * unit[1]])
                            .into();
                    P2New {
                        series: vec![line, bars],
                        kind: Some(P2Kind::ErrorBar(Box::new(r))),
                    }
                })
                .collect(),
            ..P2Draw::default()
        };
        figure.axes.validate()?;
        self.p2_publish(figure, &ctx, draw)
    }
}

// ---------------------------------------------------------------- area / barh

const AREA_NAMES: &[&str] = &[
    "BaseValue",
    "FaceColor",
    "EdgeColor",
    "DisplayName",
    "LineWidth",
];
const BAR_NAMES: &[&str] = &[
    "BarWidth",
    "BaseValue",
    "FaceColor",
    "EdgeColor",
    "DisplayName",
    "LineWidth",
    "BarLayout",
];

/// (x, Y column-major with n rows, m columns) from y or (x, Y).
fn p2_xy_columns(
    x: Option<&Matrix>,
    y: &Matrix,
    who: &str,
) -> Result<(Vec<f64>, Vec<f64>, usize, usize)> {
    let (n, m, data) = if y.rows() == 1 || y.cols() == 1 {
        (y.len(), 1, y.data().to_vec())
    } else {
        (y.rows(), y.cols(), y.data().to_vec())
    };
    let x = match x {
        Some(x) if x.len() == n && (x.rows() == 1 || x.cols() == 1) => x.data().to_vec(),
        Some(x) if (x.rows(), x.cols()) == (y.rows(), y.cols()) && m > 1 => x.data()[..n].to_vec(),
        Some(_) => {
            return Err(format!(
                "{who}: X must be a vector with one value per row of Y"
            ));
        }
        None => (1..=n).map(|i| i as f64).collect(),
    };
    if n == 0 {
        return Err(format!("{who} requires nonempty data"));
    }
    Ok((x, data, n, m))
}

impl Engine {
    fn p2_area(&mut self, args: &[Value]) -> Result<Value> {
        let (positional, rest) = p2_split(self, args, AREA_NAMES);
        let existing_x = super::plot_time_axes::existing_ruler_for_plot(self, 0).cloned();
        let existing_y = super::plot_time_axes::existing_ruler_for_plot(self, 1).cloned();
        if let Some(ey) = &existing_y
            && !ey.is_numeric()
        {
            return Err(format!(
                "Values plotted against y-axis must be {} values",
                ey.kind_word()
            ));
        }
        let mut base = 0.;
        let (x_mat, y_mat, x_kind) = match positional {
            [y] => {
                if let Some(ex) = &existing_x
                    && !ex.is_numeric()
                {
                    return Err(format!(
                        "Values plotted against x-axis must be {} values",
                        ex.kind_word()
                    ));
                }
                (
                    None,
                    real_matrix(y, "area")?,
                    crate::plot::RulerKind::Numeric,
                )
            }
            [x, y] => {
                let y_m = real_matrix(y, "area")?;
                if y_m.len() == 1 && x.dims().iter().product::<usize>() > 1 {
                    if let Some(ex) = &existing_x
                        && !ex.is_numeric()
                    {
                        return Err(format!(
                            "Values plotted against x-axis must be {} values",
                            ex.kind_word()
                        ));
                    }
                    base = y_m.as_scalar()?;
                    (
                        None,
                        real_matrix(x, "area")?,
                        crate::plot::RulerKind::Numeric,
                    )
                } else {
                    let (xm, xk) = super::plot_time_axes::extract_axis_data(
                        x,
                        existing_x.as_ref(),
                        "x",
                        &self.cancel,
                    )?;
                    (Some(xm), y_m, xk)
                }
            }
            [x, y, b] => {
                base = real_matrix(b, "area")?.as_scalar()?;
                let (xm, xk) = super::plot_time_axes::extract_axis_data(
                    x,
                    existing_x.as_ref(),
                    "x",
                    &self.cancel,
                )?;
                (Some(xm), real_matrix(y, "area")?, xk)
            }
            _ => {
                return Err(
                    "Use area(Y), area(X,Y) or area(...,basevalue) followed by Name,Value pairs"
                        .into(),
                );
            }
        };
        let (x, data, n, m) = p2_xy_columns(x_mat.as_ref(), &y_mat, "area")?;
        if n > 3000 || m > 24 {
            return Err("area draws at most 3000 rows and 24 columns".into());
        }
        let mut figure = self.p2_target();
        figure.axes.extras.rulers[0].kind = x_kind;
        let mut ctx = P2Ctx::of(&figure, self.graphics_theme_context);
        let (mut face, mut edge, mut label, mut width) = (None, Some([0u8; 3]), None, None);
        for (name, value) in p2_pairs(self, rest, "area")? {
            match name.as_str() {
                "basevalue" => base = p2_scalar(value, "area", "BaseValue")?,
                "facecolor" => face = Some(p2_color(self, value)?),
                "edgecolor" => edge = p2_edge(self, value)?,
                "displayname" => {
                    label = Some(p2_text(self, value).ok_or("DisplayName must be text")?)
                }
                "linewidth" => width = Some(p2_line_width(value)?),
                _ => return Err(format!("area: unsupported property '{name}'")),
            }
        }
        let tops = p2_area_tops(&data, n, m);
        let mut charts = Vec::new();
        for j in 0..m {
            let lower = if j == 0 {
                vec![base; n]
            } else {
                tops[j - 1].clone()
            };
            let rgb = face.unwrap_or_else(|| ctx.next_color().1);
            let mut s = p2_patch(p2_band_triangles(&x, &lower, &tops[j]), 3, rgb)?;
            if let Some(l) = &label {
                s.label = l.clone();
            }
            let mut outline = p2_decor(
                p2_band_outline(&x, &lower, &tops[j]),
                edge.unwrap_or([0; 3]),
                width.unwrap_or(0.5),
            );
            outline.visible = edge.is_some();
            charts.push(P2New {
                series: vec![s, outline],
                kind: Some(P2Kind::Area {
                    x: x.clone(),
                    y: data[j * n..(j + 1) * n].to_vec(),
                    base,
                }),
            });
        }
        let draw = P2Draw {
            charts,
            ..P2Draw::default()
        };
        self.p2_publish(figure, &ctx, draw)
    }

    fn p2_barh(&mut self, args: &[Value]) -> Result<Value> {
        let (positional, rest) = p2_split(self, args, BAR_NAMES);
        let mut numeric = Vec::new();
        let (mut stacked, mut letter) = (false, None);
        for a in positional {
            match p2_text(self, a) {
                Some(t) if t.eq_ignore_ascii_case("grouped") => stacked = false,
                Some(t) if t.eq_ignore_ascii_case("stacked") => stacked = true,
                Some(t) if t.len() == 1 => letter = Some(p2_color(self, a)?),
                Some(t) => return Err(format!("barh: unsupported option '{t}'")),
                None => numeric.push(real_matrix(a, "barh")?),
            }
        }
        let mut width = 0.8;
        if numeric.len() == 3
            || (numeric.len() == 2 && numeric[1].len() == 1 && numeric[0].len() > 1)
        {
            width = numeric.pop().unwrap().as_scalar()?;
        }
        let (x, y) = match numeric.as_slice() {
            [y] => (None, y),
            [x, y] => (Some(x), y),
            _ => {
                return Err(
                    "Use barh(y), barh(x,y), barh(...,width) or barh(...,'stacked')".into(),
                );
            }
        };
        let (x, data, n, m) = p2_xy_columns(x, y, "barh")?;
        if n > 2500 || m > 32 {
            return Err("barh draws at most 2500 groups and 32 series".into());
        }
        let figure = self.p2_target();
        let mut ctx = P2Ctx::of(&figure, self.graphics_theme_context);
        let (mut face, mut edge, mut label, mut lw, mut base) =
            (letter, Some([0u8; 3]), None, None, 0.);
        for (name, value) in p2_pairs(self, rest, "barh")? {
            match name.as_str() {
                "barwidth" => width = p2_scalar(value, "barh", "BarWidth")?,
                "basevalue" => base = p2_scalar(value, "barh", "BaseValue")?,
                "facecolor" => face = Some(p2_color(self, value)?),
                "edgecolor" => edge = p2_edge(self, value)?,
                "displayname" => {
                    label = Some(p2_text(self, value).ok_or("DisplayName must be text")?)
                }
                "linewidth" => lw = Some(p2_line_width(value)?),
                "barlayout" => {
                    stacked = match p2_text(self, value)
                        .map(|s| s.to_ascii_lowercase())
                        .as_deref()
                    {
                        Some("stacked") => true,
                        Some("grouped") => false,
                        _ => return Err("BarLayout must be 'grouped' or 'stacked'".into()),
                    }
                }
                _ => return Err(format!("barh: unsupported property '{name}'")),
            }
        }
        if !(width > 0. && width <= 1.) {
            return Err("BarWidth must be in (0, 1]".into());
        }
        let rects = p2_bar_rects(&x, &data, m, width, stacked, base, true);
        let mut charts = Vec::new();
        for (j, quads) in rects.into_iter().enumerate() {
            let rgb = face.unwrap_or_else(|| ctx.next_color().1);
            let mut s = p2_patch(quads, 4, rgb)?;
            s.face_edge_color = edge;
            s.pattern = if edge.is_some() {
                LinePattern::Solid
            } else {
                LinePattern::None
            };
            s.line_width = lw;
            if let Some(l) = &label {
                s.label = l.clone();
            }
            charts.push(P2New {
                series: vec![s],
                kind: Some(P2Kind::BarH {
                    x: x.clone(),
                    y: data[j * n..(j + 1) * n].to_vec(),
                    width,
                    stacked,
                    base,
                }),
            });
        }
        let draw = P2Draw {
            charts,
            ..P2Draw::default()
        };
        self.p2_publish(figure, &ctx, draw)
    }
}

// ---------------------------------------------------------------- pie / fill

fn p2_cell_texts(e: &Engine, value: &Value) -> Option<Vec<String>> {
    match value {
        Value::Container(Array::Cell(c)) => (0..c.len())
            .map(|i| match c.get_linear(i).ok()?.as_ref() {
                Array::Character(s) => s.to_utf8(&e.cancel).ok(),
                Array::Str(s) if s.len() == 1 => s.data()[0].as_deref().map(Into::into),
                _ => None,
            })
            .collect(),
        Value::Container(Array::Str(s)) => s
            .data()
            .iter()
            .map(|v| v.as_deref().map(Into::into))
            .collect(),
        _ => None,
    }
}

impl Engine {
    fn p2_pie(&mut self, args: &[Value]) -> Result<Value> {
        let x = p2_vec(&args[0], "pie")?;
        let fractions = p2_pie_fractions(&x)?;
        let mut explode = vec![false; x.len()];
        let mut labels = None;
        for a in &args[1..] {
            if let Some(t) = p2_cell_texts(self, a) {
                if t.len() != x.len() {
                    return Err("pie labels must have one entry per value".into());
                }
                labels = Some(t);
            } else if p2_text(self, a).is_some() {
                return Err(
                    "pie labels must be a cell array of character vectors or a string array".into(),
                );
            } else {
                let v = p2_vec(a, "pie")?;
                if v.len() != x.len() {
                    return Err("pie explode must have one entry per value".into());
                }
                explode = v.iter().map(|&v| v != 0.).collect();
            }
        }
        let shown: Vec<usize> = (0..x.len()).filter(|&i| fractions[i] > 0.).collect();
        if shown.len() > 32 {
            return Err("pie draws at most 32 nonzero slices".into());
        }
        let figure = self.p2_target();
        let mut ctx = P2Ctx::of(&figure, self.graphics_theme_context);
        let (xlim, ylim) = ctx.equal_limits(1.45, 1.3);
        let mut draw = P2Draw {
            xlim: Some(xlim),
            ylim: Some(ylim),
            ticks_hidden: true,
            ..P2Draw::default()
        };
        let mut outline = Vec::new();
        // Slices run clockwise from 12 o'clock.
        let mut angle = std::f64::consts::FRAC_PI_2;
        for &i in &shown {
            let end = angle - std::f64::consts::TAU * fractions[i];
            let slice = p2_pie_slice(angle, end, explode[i]);
            angle = end;
            let rgb = ctx.next_color().1;
            let s = p2_patch(slice.triangles, 3, rgb)?;
            outline.extend(slice.outline);
            let text = labels
                .as_ref()
                .map_or_else(|| p2_pie_label(fractions[i]), |l| l[i].clone());
            let (dir, inside) = (
                [slice.middle.cos(), slice.middle.sin()],
                fractions[i] >= 0.06 && labels.is_none(),
            );
            let r = if inside { 0.62 } else { 1.12 };
            let at = [slice.center[0] + r * dir[0], slice.center[1] + r * dir[1]];
            let (align, color) = if inside {
                (TextAlign::Center, p2_text_on(rgb))
            } else if dir[0] > 0.2 {
                (TextAlign::Left, ctx.text)
            } else if dir[0] < -0.2 {
                (TextAlign::Right, ctx.text)
            } else {
                (TextAlign::Center, ctx.text)
            };
            draw.text_after.push(Some(draw.texts.len()));
            draw.texts
                .push(ctx.label(&text, at, xlim, ylim, align, color, 10.)?);
            draw.charts.push(P2New {
                series: vec![s],
                kind: None,
            });
        }
        draw.decor.push(p2_decor(outline, [0; 3], 0.5));
        self.p2_publish(figure, &ctx, draw)
    }

    fn p2_fill(&mut self, args: &[Value]) -> Result<Value> {
        const FILL_NAMES: &[&str] = &[
            "FaceColor",
            "EdgeColor",
            "DisplayName",
            "LineWidth",
            "LineStyle",
        ];
        let (positional, rest) = p2_split(self, args, FILL_NAMES);
        if positional.is_empty() || !positional.len().is_multiple_of(3) {
            return Err("Use fill(x,y,c) or fill(x1,y1,c1,x2,y2,c2,...) with a colour letter, name or RGB triple".into());
        }
        let figure = self.p2_target();
        let ctx = P2Ctx::of(&figure, self.graphics_theme_context);
        let (mut face, mut edge, mut label, mut lw, mut style) =
            (None, Some([0u8; 3]), None, None, None);
        for (name, value) in p2_pairs(self, rest, "fill")? {
            match name.as_str() {
                "facecolor" => face = Some(p2_fill_face(self, value)?),
                "edgecolor" => edge = p2_edge(self, value)?,
                "displayname" => {
                    label = Some(p2_text(self, value).ok_or("DisplayName must be text")?)
                }
                "linewidth" => lw = Some(p2_line_width(value)?),
                "linestyle" => style = Some(p2_line_style(self, value)?),
                _ => return Err(format!("fill: unsupported property '{name}'")),
            }
        }
        let mut charts = Vec::new();
        for triple in positional.chunks(3) {
            let x = real_matrix(&triple[0], "fill")?;
            let y = real_matrix(&triple[1], "fill")?;
            if x.len() != y.len() {
                return Err("fill X and Y must have the same number of elements".into());
            }
            let vertex_cdata = p2_fill_vertex_cdata(self, &triple[2], &x)?;
            let pos_color = match &vertex_cdata {
                Some(_) => P2FillColor::Scalar(0.),
                None => p2_fill_color(self, &triple[2])?,
            };
            let (rgb, face_mode, face_none, patch_cdata) = match (pos_color, face) {
                _ if vertex_cdata.is_some() => match face {
                    None | Some(P2FillFace::Interp) => ([0; 3], FaceColorMode::Flat, false, None),
                    Some(P2FillFace::None) => ([0; 3], FaceColorMode::None, true, None),
                    Some(P2FillFace::Rgb(rgb)) => (rgb, FaceColorMode::Color, false, None),
                    Some(P2FillFace::Flat) => {
                        return Err(
                            "fill FaceColor 'flat' with per-vertex CData is not supported yet"
                                .into(),
                        );
                    }
                },
                (_, Some(P2FillFace::Interp)) => {
                    return Err("fill FaceColor 'interp' requires per-vertex CData".into());
                }
                (P2FillColor::Scalar(c), None | Some(P2FillFace::Flat)) => {
                    ([0; 3], FaceColorMode::Flat, false, Some(c))
                }
                (P2FillColor::Scalar(c), Some(P2FillFace::None)) => {
                    ([0; 3], FaceColorMode::None, true, Some(c))
                }
                (P2FillColor::Scalar(c), Some(P2FillFace::Rgb(rgb))) => {
                    (rgb, FaceColorMode::Color, false, Some(c))
                }
                (P2FillColor::Rgb(rgb), None) => (rgb, FaceColorMode::Color, false, None),
                (P2FillColor::Rgb(_), Some(P2FillFace::Rgb(rgb))) => {
                    (rgb, FaceColorMode::Color, false, None)
                }
                (P2FillColor::Rgb(rgb), Some(P2FillFace::None)) => {
                    (rgb, FaceColorMode::None, true, None)
                }
                (P2FillColor::Rgb(_), Some(P2FillFace::Flat)) => {
                    return Err("fill FaceColor 'flat' requires scalar CData".into());
                }
            };
            let (xs, ys) = if x.rows() > 1 && x.cols() > 1 {
                (p2_columns(&x), p2_columns(&y))
            } else {
                (vec![x.data().to_vec()], vec![y.data().to_vec()])
            };
            for (xc, yc) in xs.iter().zip(&ys) {
                if xc.len() > 3000 {
                    return Err("fill polygons have at most 3000 vertices".into());
                }
                let poly: Vec<[f64; 2]> = xc.iter().zip(yc).map(|(&a, &b)| [a, b]).collect();
                if poly.iter().flatten().any(|v| !v.is_finite()) {
                    return Err("fill vertices must be finite".into());
                }
                let corners = p2_triangulate_indices(&poly);
                let tri = corners.iter().map(|&i| poly[i]).collect();
                let mut s = p2_patch(tri, 3, rgb)?;
                s.face_color_mode = face_mode;
                s.face_color_none = face_none;
                s.patch_cdata = patch_cdata;
                if let Some(values) = &vertex_cdata {
                    s.patch_vertex_cdata = Some(crate::plot::VertexCData {
                        source: values.as_slice().into(),
                        corners: corners.iter().map(|&i| i as u32).collect(),
                    });
                }
                if let Some(l) = &label {
                    s.label = l.clone();
                }
                let mut ring = poly.clone();
                if let Some(&first) = poly.first() {
                    ring.push(first);
                }
                let mut outline = p2_decor(ring, edge.unwrap_or([0; 3]), lw.unwrap_or(0.5));
                outline.visible = edge.is_some();
                if let Some(p) = style {
                    outline.pattern = p;
                }
                charts.push(P2New {
                    series: vec![s, outline],
                    kind: Some(P2Kind::Fill),
                });
            }
        }
        if charts.len() > 24 {
            return Err("fill draws at most 24 polygons per call".into());
        }
        let draw = P2Draw {
            charts,
            ..P2Draw::default()
        };
        self.p2_publish(figure, &ctx, draw)
    }
}

// ---------------------------------------------------------------- polarplot

/// Theta columns, rho columns and an optional line specification.
type P2PolarGroup = (Vec<Vec<f64>>, Vec<Vec<f64>>, Option<String>);

impl Engine {
    fn p2_polarplot(&mut self, args: &[Value]) -> Result<Value> {
        const POLAR_NAMES: &[&str] = &["LineWidth", "Color", "DisplayName", "LineStyle"];
        let (positional, rest) = p2_split(self, args, POLAR_NAMES);
        // Groups: (theta, rho[, spec]) or a lone rho.
        let mut groups: Vec<P2PolarGroup> = Vec::new();
        let mut i = 0;
        while i < positional.len() {
            let a = real_matrix(&positional[i], "polarplot")?;
            let b = positional.get(i + 1).filter(|v| p2_text(self, v).is_none());
            let (theta, rho) = if let Some(b) = b {
                i += 2;
                let b = real_matrix(b, "polarplot")?;
                if a.len() != b.len() && !(a.rows() == 1 || a.cols() == 1) {
                    return Err("polarplot theta and rho must have the same size".into());
                }
                let (t, r) = (p2_columns(&a), p2_columns(&b));
                if t.iter()
                    .zip(r.iter().cycle())
                    .any(|(t, r)| t.len() != r.len())
                    && t.len() == r.len()
                {
                    return Err("polarplot theta and rho must have the same size".into());
                }
                (t, r)
            } else {
                i += 1;
                let r = p2_columns(&a);
                let n = r[0].len();
                let t = vec![
                    (0..n)
                        .map(|k| {
                            if n > 1 {
                                std::f64::consts::TAU * k as f64 / (n - 1) as f64
                            } else {
                                0.
                            }
                        })
                        .collect(),
                ];
                (t, r)
            };
            let spec = positional.get(i).and_then(|v| p2_text(self, v));
            if spec.is_some() {
                i += 1;
            }
            groups.push((theta, rho, spec));
        }
        let figure = self.p2_target();
        let held_polar = self.p2.polar.iter().any(|p| p.live_on(&figure.axes));
        if figure.axes.hold && !held_polar && !figure.axes.series.is_empty() {
            return Err(
                "polarplot cannot add to Cartesian axes; use a new figure, subplot or hold off"
                    .into(),
            );
        }
        let mut ctx = P2Ctx::of(&figure, self.graphics_theme_context);
        let (mut width, mut color, mut label, mut style) = (None, None, None, None);
        for (name, value) in p2_pairs(self, rest, "polarplot")? {
            match name.as_str() {
                "linewidth" => width = Some(p2_line_width(value)?),
                "color" => color = Some(p2_color(self, value)?),
                "displayname" => {
                    label = Some(p2_text(self, value).ok_or("DisplayName must be text")?)
                }
                "linestyle" => style = Some(p2_line_style(self, value)?),
                _ => return Err(format!("polarplot: unsupported property '{name}'")),
            }
        }
        let mut charts = Vec::new();
        for (theta, rho, spec) in groups {
            let count = theta.len().max(rho.len());
            for k in 0..count {
                let t = theta[k.min(theta.len() - 1)].clone();
                let r = rho[k.min(rho.len() - 1)].clone();
                if t.len() != r.len() {
                    return Err("polarplot theta and rho must have the same size".into());
                }
                if t.len() > 100_000 {
                    return Err("polarplot draws at most 100000 points per line".into());
                }
                let auto = ctx.next_color();
                let mut s = p2_line(vec![[0., 0.]; t.len()], color.map_or(auto, |c| (None, c)));
                if let Some(spec) = &spec {
                    s.line_spec(spec)?;
                }
                if let Some(p) = style {
                    s.pattern = p;
                }
                s.line_width = width;
                if let Some(l) = &label {
                    s.label = l.clone();
                }
                charts.push(P2New {
                    series: vec![s],
                    kind: Some(P2Kind::PolarLine { theta: t, rho: r }),
                });
            }
        }
        if !held_polar {
            self.p2
                .polar
                .retain(|p| !p.axes.same_object(&figure.axes.identity));
            self.p2.polar.push(P2Polar {
                axes: figure.axes.identity.clone(),
                rlim: None,
                zero: "right",
                clockwise: false,
                grid: None,
            });
        }
        let handles = self.p2_publish(
            figure.clone(),
            &ctx,
            P2Draw {
                charts,
                ..P2Draw::default()
            },
        )?;
        let mut figure = self.figure.take().ok_or("Polar figure vanished")?;
        let refreshed = self.p2_polar_refresh(&mut figure);
        self.figure = Some(figure);
        refreshed?;
        Ok(handles)
    }

    /// Recompute every polar line, the grid and its labels of the current axes.
    fn p2_polar_refresh(&mut self, figure: &mut Figure) -> Result<()> {
        let axes_id = figure.axes.identity.clone();
        let index = self
            .p2
            .polar
            .iter()
            .position(|p| p.live_on(&figure.axes))
            .ok_or("The current axes is not a polar axes")?;
        let polar = self.p2.polar[index].clone();
        // Remove the old grid and labels.
        if let Some(grid) = &polar.grid {
            figure.axes.series.retain(|s| !s.identity.same_object(grid));
        }
        let owned: Vec<ObjectIdentity> = self
            .p2
            .texts
            .iter()
            .filter(|(a, _)| a.same_object(&axes_id))
            .map(|(_, t)| t.clone())
            .collect();
        figure
            .textboxes
            .retain(|t| !owned.iter().any(|o| o.same_object(&t.identity)));
        self.p2.texts.retain(|(a, _)| !a.same_object(&axes_id));
        let lines: Vec<(usize, Vec<f64>, Vec<f64>)> = figure
            .axes
            .series
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                self.p2
                    .charts
                    .iter()
                    .find(|c| c.primary.same_object(&s.identity))
                    .and_then(|c| match &c.kind {
                        P2Kind::PolarLine { theta, rho } => Some((i, theta.clone(), rho.clone())),
                        _ => None,
                    })
            })
            .collect();
        let all_rho: Vec<f64> = lines
            .iter()
            .flat_map(|(_, _, r)| r.iter().copied())
            .collect();
        let rlim = polar.rlim.unwrap_or_else(|| p2_auto_rlim(&all_rho));
        let orient = polar.orient();
        for (i, theta, rho) in &lines {
            let points: Vec<[f64; 2]> = theta
                .iter()
                .zip(rho)
                .map(|(&t, &r)| orient.point(t, r, rlim))
                .collect();
            figure.axes.series[*i].points = points.into();
        }
        let ctx = P2Ctx::of(figure, self.graphics_theme_context);
        let radius = rlim[1] - rlim[0];
        let step = p2_nice_step(radius, 4.);
        let (grid_points, ticks) = p2_polar_grid(rlim, step, orient);
        let mut grid = p2_decor(grid_points, ctx.grid, 0.5);
        grid.label = String::new();
        let grid_id = grid.identity.clone();
        figure.axes.series.insert(0, grid);
        let (xlim, ylim) = ctx.equal_limits(radius * 1.2, radius * 1.12);
        figure.axes.xlim = Some(xlim);
        figure.axes.ylim = Some(ylim);
        figure.axes.ticks_hidden = true;
        let mut texts = Vec::new();
        for k in 0..12 {
            let deg = 30. * k as f64;
            let phi = orient.zero + orient.dir * deg.to_radians();
            let at = [1.09 * radius * phi.cos(), 1.07 * radius * phi.sin()];
            let align = if phi.cos() > 0.3 {
                TextAlign::Left
            } else if phi.cos() < -0.3 {
                TextAlign::Right
            } else {
                TextAlign::Center
            };
            texts.push(ctx.label(&format!("{deg}°"), at, xlim, ylim, align, ctx.text, 9.)?);
        }
        let label_angle = orient.zero + orient.dir * 75f64.to_radians();
        for &r in &ticks {
            let rr = r - rlim[0];
            let at = [rr * label_angle.cos(), rr * label_angle.sin()];
            texts.push(ctx.label(&p2_short(r), at, xlim, ylim, TextAlign::Left, ctx.text, 9.)?);
        }
        if rlim[0] != 0. {
            texts.push(ctx.label(
                &p2_short(rlim[0]),
                [0., 0.],
                xlim,
                ylim,
                TextAlign::Left,
                ctx.text,
                9.,
            )?);
        }
        if figure.textboxes.len() + texts.len() > crate::plot::MAX_TEXTBOXES {
            return Err("Polar labels exceed the figure's 64 textboxes".into());
        }
        for t in &texts {
            self.p2.texts.push((axes_id.clone(), t.identity.clone()));
        }
        figure.textboxes.extend(texts);
        figure.validate()?;
        self.p2.polar[index].grid = Some(grid_id);
        Ok(())
    }
}

// ---------------------------------------------------------------- fplot

impl Engine {
    fn p2_sample(
        &mut self,
        function: &crate::FunctionHandle,
        range: [f64; 2],
        density: usize,
        out: &mut Execution,
    ) -> Result<(Vec<f64>, Vec<f64>)> {
        // Set once a vector call fails or returns the wrong shape; later rounds
        // then go straight to point-by-point evaluation.
        let mut pointwise = false;
        let mut eval = |xs: &[f64]| -> Result<Vec<f64>> {
            self.check_cancel()?;
            if !pointwise {
                let input = Value::Matrix(Matrix::from_column_slice(1, xs.len(), xs)?);
                // An error on the vector (x^2 on a row) means "not vectorized",
                // like a wrong-shape result. As in eval(try, catch), the failed
                // call leaves no error state; cancellation and run limits stop.
                let failure = self.functions.failure.clone();
                let file_diagnostic = self.file_diagnostic.clone();
                let value = match self.call_handle(function, vec![input], 1, None, out) {
                    Ok(r) => r.into_iter().next(),
                    Err(message) if self.is_catchable(&message) => {
                        self.functions.failure = failure;
                        self.file_diagnostic = file_diagnostic;
                        self.raised = None;
                        self.functions.lng.thrown_stack = None;
                        self.functions.lng.thrown_object = None;
                        None
                    }
                    Err(message) => return Err(message),
                };
                match &value {
                    Some(Value::Matrix(m)) if m.len() == xs.len() => {
                        return Ok(m.data().to_vec());
                    }
                    Some(v @ Value::Logical(_)) => {
                        let m = real_matrix(v, "fplot")?;
                        if m.len() == xs.len() {
                            return Ok(m.data().to_vec());
                        }
                    }
                    Some(Value::Complex(_)) => {
                        return Err("fplot requires a real-valued function".into());
                    }
                    _ => {}
                }
                pointwise = true;
            }
            // Not vectorized: evaluate point by point.
            xs.iter()
                .map(|&x| {
                    self.check_cancel()?;
                    let r = self.call_handle(
                        function,
                        vec![Value::Matrix(Matrix::scalar(x))],
                        1,
                        None,
                        out,
                    )?;
                    let v = r
                        .into_iter()
                        .next()
                        .ok_or("fplot function returned no value")?;
                    let m = real_matrix(&v, "fplot")
                        .map_err(|_| "fplot requires a real numeric function".to_string())?;
                    if m.len() != 1 {
                        return Err("fplot function must return one value per input".into());
                    }
                    Ok(m.data()[0])
                })
                .collect()
        };
        p2_adaptive_samples(range[0], range[1], density, &mut eval)
    }

    fn p2_fplot(&mut self, args: &[Value], out: &mut Execution) -> Result<Value> {
        const FPLOT_NAMES: &[&str] = &[
            "MeshDensity",
            "LineWidth",
            "Color",
            "DisplayName",
            "LineStyle",
        ];
        let Value::Container(Array::Handle(function)) = &args[0] else {
            return Err("fplot requires a function handle, for example fplot(@sin)".into());
        };
        let function = function.clone();
        let (positional, rest) = p2_split(self, &args[1..], FPLOT_NAMES);
        let mut range = [-5., 5.];
        let mut spec = None;
        for a in positional {
            if let Some(t) = p2_text(self, a) {
                spec = Some(t);
            } else if matches!(a, Value::Container(Array::Handle(_))) {
                return Err("Parametric fplot(funx,funy) is not supported yet".into());
            } else {
                range = p2_pair(a, "fplot", "the interval")?;
            }
        }
        let (mut density, mut width, mut color, mut label, mut style) =
            (23usize, None, None, None, None);
        for (name, value) in p2_pairs(self, rest, "fplot")? {
            match name.as_str() {
                "meshdensity" => {
                    let d = p2_scalar(value, "fplot", "MeshDensity")?;
                    if !(2. ..=5000.).contains(&d) || d.fract() != 0. {
                        return Err("MeshDensity must be an integer in 2..5000".into());
                    }
                    density = d as usize;
                }
                "linewidth" => width = Some(p2_line_width(value)?),
                "color" => color = Some(p2_color(self, value)?),
                "displayname" => {
                    label = Some(p2_text(self, value).ok_or("DisplayName must be text")?)
                }
                "linestyle" => style = Some(p2_line_style(self, value)?),
                _ => return Err(format!("fplot: unsupported property '{name}'")),
            }
        }
        let (xs, ys) = self.p2_sample(&function, range, density, out)?;
        let figure = self.p2_target();
        let mut ctx = P2Ctx::of(&figure, self.graphics_theme_context);
        let auto = ctx.next_color();
        let mut s = p2_line(
            xs.iter().zip(&ys).map(|(&a, &b)| [a, b]).collect(),
            color.map_or(auto, |c| (None, c)),
        );
        if let Some(spec) = &spec {
            s.line_spec(spec)?;
        }
        if let Some(p) = style {
            s.pattern = p;
        }
        s.line_width = width;
        s.label = label.unwrap_or_default();
        let draw = P2Draw {
            charts: vec![P2New {
                series: vec![s],
                kind: Some(P2Kind::FPlot {
                    function,
                    range,
                    density,
                }),
            }],
            xlim: if figure.axes.hold && !figure.axes.series.is_empty() {
                None
            } else {
                Some(range)
            },
            ..P2Draw::default()
        };
        self.p2_publish(figure, &ctx, draw)
    }
}

// ---------------------------------------------------------------- heatmap

impl Engine {
    fn p2_heatmap(&mut self, args: &[Value]) -> Result<Value> {
        const HEAT_NAMES: &[&str] = &[
            "ColorLimits",
            "CellLabelColor",
            "Title",
            "XLabel",
            "YLabel",
            "ColorbarVisible",
        ];
        let (positional, rest) = p2_split(self, args, HEAT_NAMES);
        let (xv, yv, cdata) = match positional {
            [c] => (None, None, real_matrix(c, "heatmap")?),
            [x, y, c] => (Some(x), Some(y), real_matrix(c, "heatmap")?),
            _ => return Err(
                "Use heatmap(cdata) or heatmap(xvalues,yvalues,cdata) followed by Name,Value pairs"
                    .into(),
            ),
        };
        let (rows, cols) = (cdata.rows(), cdata.cols());
        if rows == 0 || cols == 0 || rows > 100 || cols > 100 {
            return Err(
                "heatmap color data must be a nonempty matrix of at most 100 by 100".into(),
            );
        }
        let category = |v: Option<&Value>, n: usize, who: &str| -> Result<Vec<String>> {
            let Some(v) = v else {
                return Ok((1..=n).map(|i| i.to_string()).collect());
            };
            let labels = match p2_cell_texts(self, v) {
                Some(t) => t,
                None => p2_vec(v, "heatmap")?.iter().map(|&x| p2_short(x)).collect(),
            };
            if labels.len() != n {
                return Err(format!(
                    "heatmap {who} must have one entry per {}",
                    if who == "xvalues" { "column" } else { "row" }
                ));
            }
            Ok(labels)
        };
        let xlabels = category(xv, cols, "xvalues")?;
        let ylabels = category(yv, rows, "yvalues")?;
        let finite: Vec<f64> = cdata
            .data()
            .iter()
            .copied()
            .filter(|v| v.is_finite())
            .collect();
        let mut limits = finite
            .iter()
            .fold([f64::INFINITY, f64::NEG_INFINITY], |l, &v| {
                [l[0].min(v), l[1].max(v)]
            });
        if !limits[0].is_finite() {
            limits = [0., 1.];
        }
        let (mut show_labels, mut colorbar, mut title, mut xl, mut yl) =
            (true, true, None, None, None);
        for (name, value) in p2_pairs(self, rest, "heatmap")? {
            match name.as_str() {
                "colorlimits" => limits = p2_pair(value, "heatmap", "ColorLimits")?,
                "celllabelcolor" => {
                    show_labels =
                        !p2_text(self, value).is_some_and(|s| s.eq_ignore_ascii_case("none"))
                }
                "colorbarvisible" => colorbar = p2_on_off(self, value, "ColorbarVisible")?,
                "title" => title = Some(p2_text(self, value).ok_or("Title must be text")?),
                "xlabel" => xl = Some(p2_text(self, value).ok_or("XLabel must be text")?),
                "ylabel" => yl = Some(p2_text(self, value).ok_or("YLabel must be text")?),
                _ => return Err(format!("heatmap: unsupported property '{name}'")),
            }
        }
        let figure = self.p2_target();
        let ctx = P2Ctx::of(&figure, self.graphics_theme_context);
        let base = crate::plot::PaletteVersion::MatRustModern1.color(0, BaseTheme::Light);
        let (c0, c1) = (cols as f64 + 0.5, rows as f64 + 0.5);
        let bar = [
            c0 + 0.08 * cols as f64 + 0.2,
            c0 + 0.13 * cols as f64 + 0.45,
        ];
        let xlim = [
            0.5,
            if colorbar {
                bar[1] + 0.12 * cols as f64 + 0.3
            } else {
                c0
            },
        ];
        let ylim = [0.5, c1];
        let mut quads: Vec<Vec<[f64; 2]>> = vec![Vec::new(); P2_LEVELS + 1];
        let mut cell_text = Vec::new();
        for j in 0..cols {
            for i in 0..rows {
                let v = cdata.data()[j * rows + i];
                let level = p2_level(v, limits[0], limits[1]);
                let (x, y) = (j as f64 + 1., (rows - i) as f64);
                quads[level.unwrap_or(P2_LEVELS)].extend([
                    [x - 0.5, y - 0.5],
                    [x - 0.5, y + 0.5],
                    [x + 0.5, y + 0.5],
                    [x + 0.5, y - 0.5],
                ]);
                let rgb = level.map_or([180; 3], |l| p2_ramp(l, base));
                cell_text.push((p2_short(v), [x, y], p2_text_on(rgb)));
            }
        }
        if colorbar {
            for (l, q) in quads.iter_mut().take(P2_LEVELS).enumerate() {
                let h = (c1 - 0.5) / P2_LEVELS as f64;
                let (y0, y1) = (0.5 + h * l as f64, 0.5 + h * (l + 1) as f64);
                q.extend([[bar[0], y0], [bar[0], y1], [bar[1], y1], [bar[1], y0]]);
            }
        }
        let mut series = Vec::new();
        for (l, q) in quads.into_iter().enumerate() {
            if q.is_empty() {
                continue;
            }
            let rgb = if l == P2_LEVELS {
                [180; 3]
            } else {
                p2_ramp(l, base)
            };
            series.push(p2_patch(q, 4, rgb)?);
        }
        // Grid lines between cells, in the background colour.
        let mut lines = Vec::new();
        for j in 0..=cols {
            lines.extend([[j as f64 + 0.5, 0.5], [j as f64 + 0.5, c1], [f64::NAN; 2]]);
        }
        for i in 0..=rows {
            lines.extend([[0.5, i as f64 + 0.5], [c0, i as f64 + 0.5], [f64::NAN; 2]]);
        }
        let mut texts = Vec::new();
        let budget =
            crate::plot::MAX_TEXTBOXES - figure.textboxes.len().min(crate::plot::MAX_TEXTBOXES);
        let axis_labels = cols + rows + if colorbar { 2 } else { 0 };
        let ticks_hidden = axis_labels <= budget;
        if ticks_hidden {
            for (j, t) in xlabels.iter().enumerate() {
                texts.push(ctx.label(
                    t,
                    [j as f64 + 1., 0.5 - 0.035 * (c1 - 0.5) - 0.02],
                    xlim,
                    ylim,
                    TextAlign::Center,
                    ctx.text,
                    9.,
                )?);
            }
            for (i, t) in ylabels.iter().enumerate() {
                texts.push(ctx.label(
                    t,
                    [0.5 - 0.01 * cols as f64 - 0.02, (rows - i) as f64],
                    xlim,
                    ylim,
                    TextAlign::Right,
                    ctx.text,
                    9.,
                )?);
            }
            if colorbar {
                texts.push(ctx.label(
                    &p2_short(limits[0]),
                    [bar[1] + 0.05, 0.5 + 0.02 * c1],
                    xlim,
                    ylim,
                    TextAlign::Left,
                    ctx.text,
                    9.,
                )?);
                texts.push(ctx.label(
                    &p2_short(limits[1]),
                    [bar[1] + 0.05, c1 - 0.02 * c1],
                    xlim,
                    ylim,
                    TextAlign::Left,
                    ctx.text,
                    9.,
                )?);
            }
        }
        if show_labels && texts.len() + cell_text.len() <= budget {
            for (t, at, color) in &cell_text {
                texts.push(ctx.label(t, *at, xlim, ylim, TextAlign::Center, *color, 9.)?);
            }
        }
        let primary_rest = series.split_off(1);
        let mut chart_series = series;
        chart_series.extend(primary_rest);
        chart_series.push(p2_decor(lines, [255; 3], 1.));
        let draw = P2Draw {
            charts: vec![P2New {
                series: chart_series,
                kind: Some(P2Kind::Heatmap(Box::new(P2Heat {
                    cdata,
                    limits,
                    labels: show_labels,
                }))),
            }],
            texts,
            xlim: Some(xlim),
            ylim: Some(ylim),
            title,
            xlabel: xl,
            ylabel: yl,
            ticks_hidden,
            ..P2Draw::default()
        };
        self.p2_publish(figure, &ctx, draw)
    }
}

// ---------------------------------------------------------------- get / set

/// Located chart: figure number, the chart record index and the series index
/// of its primary in the figure's current axes.
impl Engine {
    fn p2_find(&self, h: &GraphicsHandle) -> Option<usize> {
        if h.kind() != GraphicsKind::Series {
            return None;
        }
        self.p2
            .charts
            .iter()
            .position(|c| c.primary.same_object(&h.object))
    }

    /// The live polar record of axes `axes` in figure `figure`, if that axes
    /// is (still) polar.
    fn p2_polar_of(&self, figure: u32, axes: &ObjectIdentity) -> Option<usize> {
        let f = self.graphics_figure(figure).ok()?;
        let (_, a) = f.all_axes().find(|(_, a)| a.identity.same_object(axes))?;
        self.p2.polar.iter().position(|p| p.live_on(a))
    }

    /// get(h, Name) for chart objects and polar axes; None falls through to
    /// the generic series/axes queries.
    pub(super) fn p2_get(
        &self,
        h: &GraphicsHandle,
        figure: u32,
        name: &str,
    ) -> Result<Option<Value>> {
        let name = name.to_ascii_lowercase();
        if h.kind() == GraphicsKind::Axes {
            let Some(index) = self.p2_polar_of(figure, &h.object) else {
                return Ok(None);
            };
            let p = &self.p2.polar[index];
            return Ok(Some(match name.as_str() {
                "type" => p2_str("polaraxes"),
                "rlim" => {
                    let f = self.graphics_figure(figure).ok();
                    p2_row(&p.rlim.unwrap_or_else(|| {
                        let rho: Vec<f64> = f
                            .into_iter()
                            .flat_map(|f| {
                                f.all_axes()
                                    .filter(|(_, a)| a.identity.same_object(&p.axes))
                                    .flat_map(|(_, a)| a.series.iter())
                            })
                            .filter_map(|s| {
                                self.p2
                                    .charts
                                    .iter()
                                    .find(|c| c.primary.same_object(&s.identity))
                            })
                            .flat_map(|c| match &c.kind {
                                P2Kind::PolarLine { rho, .. } => rho.clone(),
                                _ => Vec::new(),
                            })
                            .collect();
                        p2_auto_rlim(&rho)
                    }))?
                }
                "rlimmode" => p2_str(if p.rlim.is_some() { "manual" } else { "auto" }),
                "thetazerolocation" => p2_str(p.zero),
                "thetadir" => p2_str(if p.clockwise {
                    "clockwise"
                } else {
                    "counterclockwise"
                }),
                _ => return Ok(None),
            }));
        }
        let Some(index) = self.p2_find(h) else {
            return Ok(None);
        };
        let chart = &self.p2.charts[index];
        let primary = self
            .graphics_figure(figure)
            .ok()
            .and_then(|f| {
                f.all_axes()
                    .flat_map(|(_, a)| a.series.iter())
                    .find(|s| s.identity.same_object(&chart.primary))
                    .cloned()
            })
            .ok_or("Chart is no longer live")?;
        if name == "type" {
            return Ok(Some(p2_str(chart.kind.type_name())));
        }
        if let P2Kind::Contour(rec) = &chart.kind {
            return contour::get(rec, &name);
        }
        let v = match (&chart.kind, name.as_str()) {
            (P2Kind::Histogram(h), "values") => p2_row(&h.values)?,
            (P2Kind::Histogram(h), "bincounts") => p2_row(&h.counts)?,
            (P2Kind::Histogram(h), "binedges") => p2_row(&h.edges)?,
            (P2Kind::Histogram(h), "numbins") => {
                Value::Matrix(Matrix::scalar((h.edges.len() - 1) as f64))
            }
            (P2Kind::Histogram(h), "binwidth") => {
                let w = h.edges[1] - h.edges[0];
                let uniform = h
                    .edges
                    .windows(2)
                    .all(|e| ((e[1] - e[0]) - w).abs() <= 1e-9 * w.abs().max(1e-300));
                if uniform {
                    Value::Matrix(Matrix::scalar(w))
                } else {
                    p2_str("nonuniform")
                }
            }
            (P2Kind::Histogram(h), "binlimits") => p2_row(&[h.edges[0], *h.edges.last().unwrap()])?,
            (P2Kind::Histogram(h), "normalization") => p2_str(h.bins.norm.name()),
            (P2Kind::Histogram(h), "binmethod") => p2_str(
                if h.bins.edges.is_some() || h.bins.nbins.is_some() || h.bins.width.is_some() {
                    "manual"
                } else {
                    h.bins.method.name()
                },
            ),
            (P2Kind::Histogram(h), "displaystyle") => {
                p2_str(if h.stairs { "stairs" } else { "bar" })
            }
            (P2Kind::Histogram(h), "data") => Value::Matrix(h.data.clone()),
            (P2Kind::Histogram(h), "facealpha") => Value::Matrix(Matrix::scalar(h.face_alpha)),
            // The drawn colour is the tinted face; report the face itself.
            (P2Kind::Histogram(h), "facecolor") if !h.stairs => p2_rgb_value(h.face)?,
            (P2Kind::ErrorBar(r), "xdata") => p2_row(&r.x)?,
            (P2Kind::ErrorBar(r), "ydata") => p2_row(&r.y)?,
            (P2Kind::ErrorBar(r), "ynegativedelta") => p2_row(&r.d.yneg)?,
            (P2Kind::ErrorBar(r), "ypositivedelta") => p2_row(&r.d.ypos)?,
            (P2Kind::ErrorBar(r), "xnegativedelta") => p2_row(&r.d.xneg)?,
            (P2Kind::ErrorBar(r), "xpositivedelta") => p2_row(&r.d.xpos)?,
            (P2Kind::ErrorBar(r), "capsize") => Value::Matrix(Matrix::scalar(r.cap)),
            (P2Kind::ErrorBar(_) | P2Kind::FPlot { .. } | P2Kind::PolarLine { .. }, "color") => {
                p2_rgb_value(
                    self.graphics_figure(figure)
                        .ok()
                        .map_or(primary.color, |f| {
                            f.resolve_appearance(self.graphics_theme_context)
                                .series_color(&primary)
                        }),
                )?
            }
            (P2Kind::Area { x, .. }, "xdata") => p2_row(x)?,
            (P2Kind::Area { y, .. }, "ydata") => p2_row(y)?,
            (P2Kind::Area { base, .. }, "basevalue") => Value::Matrix(Matrix::scalar(*base)),
            (P2Kind::BarH { x, .. }, "xdata") => p2_row(x)?,
            (P2Kind::BarH { y, .. }, "ydata") => p2_row(y)?,
            (P2Kind::BarH { width, .. }, "barwidth") => Value::Matrix(Matrix::scalar(*width)),
            (P2Kind::BarH { base, .. }, "basevalue") => Value::Matrix(Matrix::scalar(*base)),
            (P2Kind::BarH { stacked, .. }, "barlayout") => {
                p2_str(if *stacked { "stacked" } else { "grouped" })
            }
            (P2Kind::BarH { .. }, "horizontal") => p2_str("on"),
            (P2Kind::FPlot { function, .. }, "function") => p2_str(function.func2str()),
            (P2Kind::FPlot { range, .. }, "xrange") => p2_row(range)?,
            (P2Kind::FPlot { density, .. }, "meshdensity") => {
                Value::Matrix(Matrix::scalar(*density as f64))
            }
            (P2Kind::FPlot { .. }, "xdata") => {
                p2_row(&primary.points.iter().map(|p| p[0]).collect::<Vec<_>>())?
            }
            (P2Kind::FPlot { .. }, "ydata") => {
                p2_row(&primary.points.iter().map(|p| p[1]).collect::<Vec<_>>())?
            }
            (P2Kind::PolarLine { theta, .. }, "thetadata") => p2_row(theta)?,
            (P2Kind::PolarLine { rho, .. }, "rdata") => p2_row(rho)?,
            (P2Kind::Heatmap(m), "colordata") => Value::Matrix(m.cdata.clone()),
            (P2Kind::Heatmap(m), "colorlimits") => p2_row(&m.limits)?,
            (P2Kind::Heatmap(m), "celllabelcolor") => {
                p2_str(if m.labels { "auto" } else { "none" })
            }
            (P2Kind::Fill, "facecolor") => {
                if primary.face_color_none || primary.face_color_mode == FaceColorMode::None {
                    p2_str("none")
                } else if primary.face_color_mode == FaceColorMode::Flat {
                    p2_str(if primary.patch_vertex_cdata.is_some() {
                        "interp"
                    } else {
                        "flat"
                    })
                } else {
                    p2_rgb_value(primary.color)?
                }
            }
            (P2Kind::Fill, "cdata") => {
                if let Some(vertex) = &primary.patch_vertex_cdata {
                    Value::Matrix(Matrix::from_column_slice(
                        vertex.source.len(),
                        1,
                        &vertex.source,
                    )?)
                } else if let Some(c) = primary.patch_cdata {
                    Value::Matrix(Matrix::scalar(c))
                } else {
                    // An RGB colour argument sets FaceColor and leaves CData empty.
                    Value::Matrix(Matrix::from_column_slice(0, 0, &[])?)
                }
            }
            (P2Kind::Fill, "cdatamapping") => p2_str("scaled"),
            (P2Kind::Fill, "edgecolor") => {
                let outline = chart.helpers.first().and_then(|id| {
                    self.figures()
                        .find(|(n, _)| *n == figure)
                        .and_then(|(_, f)| {
                            f.all_axes()
                                .flat_map(|(_, a)| a.series.iter())
                                .find(|s| s.identity.same_object(id))
                        })
                });
                match outline {
                    Some(s) if s.visible => p2_rgb_value(s.color)?,
                    _ => p2_str("none"),
                }
            }
            _ => return Ok(None),
        };
        Ok(Some(v))
    }

    /// set(h, ...) for chart objects and polar axes. Ok(false) when none of the
    /// pairs is chart-specific, so the generic series/axes setter runs.
    pub(super) fn p2_set(
        &mut self,
        h: &GraphicsHandle,
        figure_number: u32,
        args: &[Value],
    ) -> Result<bool> {
        let args = self.text_arguments(args.to_vec())?;
        let pairs: Vec<(String, Value)> = p2_pairs(self, &args, "set")?
            .into_iter()
            .map(|(n, v)| (n, v.clone()))
            .collect();
        if h.kind() == GraphicsKind::Axes {
            let Some(index) = self.p2_polar_of(figure_number, &h.object) else {
                return Ok(false);
            };
            const POLAR: &[&str] = &["rlim", "rlimmode", "thetazerolocation", "thetadir"];
            let mine = pairs
                .iter()
                .filter(|(n, _)| POLAR.contains(&n.as_str()))
                .count();
            if mine == 0 {
                return Ok(false);
            }
            if mine != pairs.len() {
                return Err("Set polar-axes properties (RLim, ThetaZeroLocation, ThetaDir) in their own set call".into());
            }
            let old = self.p2.polar[index].clone();
            for (name, value) in &pairs {
                let p = &mut self.p2.polar[index];
                match name.as_str() {
                    "rlim" => p.rlim = Some(p2_pair(value, "set", "RLim")?),
                    "rlimmode" => {
                        if p2_text(self, value).is_some_and(|s| s.eq_ignore_ascii_case("auto")) {
                            self.p2.polar[index].rlim = None;
                        }
                    }
                    "thetazerolocation" => {
                        let z = p2_text(self, value).map(|s| s.to_ascii_lowercase());
                        self.p2.polar[index].zero =
                            match z.as_deref() {
                                Some("right") => "right",
                                Some("top") => "top",
                                Some("left") => "left",
                                Some("bottom") => "bottom",
                                _ => return Err(
                                    "ThetaZeroLocation must be 'right', 'top', 'left' or 'bottom'"
                                        .into(),
                                ),
                            };
                    }
                    _ => {
                        let d = p2_text(self, value).map(|s| s.to_ascii_lowercase());
                        self.p2.polar[index].clockwise = match d.as_deref() {
                            Some("clockwise") => true,
                            Some("counterclockwise") => false,
                            _ => {
                                return Err(
                                    "ThetaDir must be 'clockwise' or 'counterclockwise'".into()
                                );
                            }
                        };
                    }
                }
            }
            let mut figure = self.p2_owned_figure(figure_number, &h.object)?;
            let saved = self.p2.clone();
            if let Err(error) = self.p2_polar_refresh(&mut figure) {
                self.p2 = saved;
                self.p2.polar[index] = old;
                return Err(error);
            }
            self.p2_store_figure(figure_number, figure);
            return Ok(true);
        }
        let Some(index) = self.p2_find(h) else {
            return Ok(false);
        };
        if matches!(self.p2.charts[index].kind, P2Kind::Contour(_)) {
            return contour::set(self, index, h, figure_number, &pairs);
        }
        const COMMON: &[&str] = &["visible"];
        let specific: &[&str] = match &self.p2.charts[index].kind {
            P2Kind::Histogram(_) => &[
                "numbins",
                "binedges",
                "binwidth",
                "binlimits",
                "binmethod",
                "normalization",
                "displaystyle",
                "facecolor",
                "edgecolor",
                "facealpha",
                "data",
            ],
            P2Kind::ErrorBar(_) => &[
                "xdata",
                "ydata",
                "capsize",
                "color",
                "linewidth",
                "ynegativedelta",
                "ypositivedelta",
                "xnegativedelta",
                "xpositivedelta",
            ],
            P2Kind::Area { .. } => &["edgecolor"],
            P2Kind::Fill => &["edgecolor", "facecolor", "cdata", "cdatamapping"],
            P2Kind::FPlot { .. } => &["xrange", "meshdensity"],
            _ => &[],
        };
        let mine = pairs
            .iter()
            .filter(|(n, _)| COMMON.contains(&n.as_str()) || specific.contains(&n.as_str()))
            .count();
        if mine == 0 {
            return Ok(false);
        }
        if mine != pairs.len() {
            return Err("Set chart-specific properties in their own set call, separate from generic series properties".into());
        }
        let axes_id = h.axes.clone().ok_or("Chart handle has no axes")?;
        let mut figure = self.p2_owned_figure(figure_number, &axes_id)?;
        let mut chart = self.p2.charts[index].clone();
        let find = |figure: &Figure, id: &ObjectIdentity| {
            figure
                .axes
                .series
                .iter()
                .position(|s| s.identity.same_object(id))
        };
        let p = find(&figure, &chart.primary).ok_or("Chart is no longer live")?;
        let members: Vec<usize> = std::iter::once(p)
            .chain(chart.helpers.iter().filter_map(|id| find(&figure, id)))
            .collect();
        let mut rebuild = false;
        for (name, value) in &pairs {
            match (&mut chart.kind, name.as_str()) {
                (_, "visible") => {
                    let on = p2_on_off(self, value, "Visible")?;
                    for &m in &members {
                        figure.axes.series[m].visible = on;
                    }
                }
                (P2Kind::Histogram(hist), n) => {
                    if !p2_bin_option(self, &mut hist.bins, n, value)? {
                        match n {
                            "displaystyle" => {
                                hist.stairs = match p2_text(self, value)
                                    .map(|s| s.to_ascii_lowercase())
                                    .as_deref()
                                {
                                    Some("bar") => false,
                                    Some("stairs") => true,
                                    _ => {
                                        return Err("DisplayStyle must be 'bar' or 'stairs'".into());
                                    }
                                }
                            }
                            "facecolor" => {
                                let rgb = p2_color(self, value)?;
                                hist.face = rgb;
                                let s = &mut figure.axes.series[p];
                                s.color = if hist.stairs {
                                    rgb
                                } else {
                                    p2_tint(rgb, hist.face_alpha)
                                };
                                s.explicit_color = true;
                                s.auto_palette = None;
                            }
                            "edgecolor" => {
                                let s = &mut figure.axes.series[p];
                                s.face_edge_color = p2_edge(self, value)?;
                                s.pattern = if s.face_edge_color.is_some() {
                                    LinePattern::Solid
                                } else {
                                    LinePattern::None
                                };
                            }
                            "facealpha" => {
                                let a = p2_scalar(value, "set", "FaceAlpha")?;
                                if !(0. ..=1.).contains(&a) {
                                    return Err("FaceAlpha must be in 0..1".into());
                                }
                                hist.face_alpha = a;
                            }
                            _ => hist.data = real_matrix(value, "set")?,
                        }
                    }
                    rebuild = true;
                }
                (P2Kind::ErrorBar(r), n) => {
                    match n {
                        "capsize" => {
                            r.cap = p2_scalar(value, "set", "CapSize")?;
                            if !(0. ..=100.).contains(&r.cap) {
                                return Err("CapSize must be in 0..100 points".into());
                            }
                        }
                        "color" => {
                            let rgb = p2_color(self, value)?;
                            for &m in &members {
                                let s = &mut figure.axes.series[m];
                                s.color = rgb;
                                s.explicit_color = true;
                                s.auto_palette = None;
                            }
                        }
                        "linewidth" => {
                            let w = p2_line_width(value)?;
                            for &m in &members {
                                figure.axes.series[m].line_width = Some(w);
                            }
                        }
                        // Lengths are checked after all pairs, so one set call can
                        // change the number of points together with the deltas.
                        "xdata" => r.x = p2_vec(value, "set")?,
                        "ydata" => r.y = p2_vec(value, "set")?,
                        _ => {
                            let v: Vec<f64> =
                                p2_vec(value, "set")?.iter().map(|v| v.abs()).collect();
                            let (neg, pos) = if n.starts_with('y') {
                                (&mut r.d.yneg, &mut r.d.ypos)
                            } else {
                                (&mut r.d.xneg, &mut r.d.xpos)
                            };
                            let target_neg = n.contains("negative");
                            if v.is_empty() {
                                neg.clear();
                                pos.clear();
                            } else {
                                let other = if target_neg { pos.clone() } else { neg.clone() };
                                let other = if other.is_empty() {
                                    vec![0.; v.len()]
                                } else {
                                    other
                                };
                                if target_neg {
                                    *neg = v;
                                    *pos = other;
                                } else {
                                    *pos = v;
                                    *neg = other;
                                }
                            }
                        }
                    }
                    rebuild = true;
                }
                (P2Kind::Area { .. }, _) => {
                    let edge = p2_edge(self, value)?;
                    if let Some(&outline) = members.get(1) {
                        let shown = edge.is_some() && figure.axes.series[p].visible;
                        let s = &mut figure.axes.series[outline];
                        s.visible = shown;
                        if let Some(rgb) = edge {
                            figure.axes.series[outline].color = rgb;
                        }
                    }
                }
                (P2Kind::Fill, n) => match n {
                    "edgecolor" => {
                        let edge = p2_edge(self, value)?;
                        if let Some(&outline) = members.get(1) {
                            let shown = edge.is_some() && figure.axes.series[p].visible;
                            let s = &mut figure.axes.series[outline];
                            s.visible = shown;
                            if let Some(rgb) = edge {
                                s.color = rgb;
                            }
                        }
                    }
                    "facecolor" => match p2_fill_face(self, value)? {
                        P2FillFace::Rgb(rgb) => {
                            let s = &mut figure.axes.series[p];
                            s.color = rgb;
                            s.face_color_mode = FaceColorMode::Color;
                            s.face_color_none = false;
                            s.explicit_color = true;
                        }
                        P2FillFace::Interp => {
                            let s = &mut figure.axes.series[p];
                            if s.patch_vertex_cdata.is_none() {
                                return Err(
                                    "Patch FaceColor 'interp' requires per-vertex CData".into()
                                );
                            }
                            s.face_color_mode = FaceColorMode::Flat;
                            s.face_color_none = false;
                        }
                        P2FillFace::Flat => {
                            let s = &mut figure.axes.series[p];
                            if s.patch_vertex_cdata.is_some() {
                                return Err(
                                    "Patch FaceColor 'flat' with per-vertex CData is not supported yet"
                                        .into(),
                                );
                            }
                            let Some(c) = s.patch_cdata else {
                                return Err("Patch FaceColor 'flat' requires CData".into());
                            };
                            s.face_color_mode = FaceColorMode::Flat;
                            s.face_color_none = false;
                            let map = figure.axes.extras.effective_colormap();
                            let clim = figure.axes.color_limits();
                            figure.axes.series[p].color = colormap::scaled_rgb(&map, clim, c);
                        }
                        P2FillFace::None => {
                            let s = &mut figure.axes.series[p];
                            s.face_color_mode = FaceColorMode::None;
                            s.face_color_none = true;
                        }
                    },
                    "cdata" => {
                        if figure.axes.series[p].patch_vertex_cdata.is_some() {
                            return Err(
                                "set CData of a per-vertex fill is not supported yet".into()
                            );
                        }
                        let c = p2_scalar(value, "set", "CData")?;
                        figure.axes.series[p].patch_cdata = Some(c);
                    }
                    _ => {
                        if !p2_text(self, value).is_some_and(|s| s.eq_ignore_ascii_case("scaled")) {
                            return Err("Patch CDataMapping currently supports 'scaled'".into());
                        }
                    }
                },
                (P2Kind::FPlot { range, density, .. }, n) => {
                    if n == "xrange" {
                        *range = p2_pair(value, "set", "XRange")?;
                    } else {
                        let d = p2_scalar(value, "set", "MeshDensity")?;
                        if !(2. ..=5000.).contains(&d) || d.fract() != 0. {
                            return Err("MeshDensity must be an integer in 2..5000".into());
                        }
                        *density = d as usize;
                    }
                    rebuild = true;
                }
                _ => return Err(format!("Unsupported chart property '{name}'")),
            }
        }
        if rebuild {
            let ctx = P2Ctx::of(&figure, self.graphics_theme_context);
            match &mut chart.kind {
                P2Kind::Histogram(hist) => {
                    let (edges, counts, values) = p2_bin(&hist.data, &hist.bins)?;
                    if edges.len() - 1 > crate::plot::MAX_PATCH_FACES {
                        return Err("histogram draws at most 10000 bins".into());
                    }
                    (hist.edges, hist.counts, hist.values) = (edges, counts, values);
                    let old = figure.axes.series[p].clone();
                    // Bars tint the stored face (never the drawn, already tinted
                    // colour); a stairs line draws its colour untinted, which a
                    // generic set may have changed.
                    if old.kind != PlotKind::Patch {
                        hist.face = old.color;
                    }
                    let face = hist.face;
                    let edge = if old.kind == PlotKind::Patch {
                        old.face_edge_color
                    } else {
                        Some([0; 3])
                    };
                    let mut s = Self::p2_hist_series(hist, face, edge, old.auto_palette)?;
                    s.identity = old.identity;
                    s.label = old.label;
                    s.visible = old.visible;
                    s.line_width = old.line_width;
                    s.explicit_color = old.explicit_color;
                    if s.kind == PlotKind::Patch {
                        s.auto_palette = None;
                        s.explicit_color = true;
                    }
                    figure.axes.series[p] = s;
                }
                P2Kind::ErrorBar(r) => {
                    if r.x.len() != r.y.len() || r.x.is_empty() {
                        return Err(
                            "errorbar XData and YData must be nonempty with the same length".into(),
                        );
                    }
                    if r.x.len() > 100_000 {
                        return Err("errorbar draws at most 100000 points".into());
                    }
                    let d = &r.d;
                    if [&d.yneg, &d.ypos, &d.xneg, &d.xpos]
                        .iter()
                        .any(|v| !v.is_empty() && v.len() != r.x.len())
                    {
                        return Err("Error deltas must have one value per point".into());
                    }
                    figure.axes.series[p].points =
                        r.x.iter()
                            .zip(&r.y)
                            .map(|(&a, &b)| [a, b])
                            .collect::<Vec<_>>()
                            .into();
                    let (bx, by) = p2_errorbar_bounds(&r.x, &r.y, &r.d);
                    let span = |b: [f64; 2]| if b[1] > b[0] { b[1] - b[0] } else { 1. };
                    let half = r.cap * 4. / 3. / 2.;
                    let points = p2_errorbar_segments(
                        &r.x,
                        &r.y,
                        &r.d,
                        [half * span(bx) / ctx.px[0], half * span(by) / ctx.px[1]],
                    );
                    if let Some(&bars) = members.get(1) {
                        figure.axes.series[bars].points = points.into();
                    }
                }
                P2Kind::FPlot {
                    function,
                    range,
                    density,
                } => {
                    let (function, range, density) = (function.clone(), *range, *density);
                    let mut out = Execution::default();
                    let (xs, ys) = self.p2_sample(&function, range, density, &mut out)?;
                    figure.axes.series[p].points = xs
                        .iter()
                        .zip(&ys)
                        .map(|(&a, &b)| [a, b])
                        .collect::<Vec<_>>()
                        .into();
                }
                _ => {}
            }
        }
        super::plot_properties::refresh_scatter_clim(&mut figure.axes);
        super::plot_properties::refresh_scalar_patch_colors(&mut figure.axes);
        figure.validate()?;
        self.p2.charts[index] = chart;
        self.p2_store_figure(figure_number, figure);
        Ok(true)
    }

    /// A copy of figure `number` whose current axes is `axes`.
    fn p2_owned_figure(&self, number: u32, axes: &ObjectIdentity) -> Result<Figure> {
        let figure = self
            .graphics_figure(number)
            .cloned()
            .map_err(|_| "Figure is no longer live".to_string())?;
        if !super::ui_components::is_uiaxes_fig_id(number)
            && !figure.axes.identity.same_object(axes)
        {
            return Err("Make the chart's axes current (axes(ax) or subplot) before setting chart properties".into());
        }
        Ok(figure)
    }

    fn p2_store_figure(&mut self, number: u32, figure: Figure) {
        if super::ui_components::is_uiaxes_fig_id(number) {
            self.publish_graphics_figure(number, figure);
        } else if number == self.current_figure {
            self.figure = Some(figure);
        } else {
            self.other_figures.insert(number, figure);
        }
    }
}
