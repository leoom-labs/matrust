//! 3-D axes view: MATLAB's `viewmtx(az, el)` orthographic projection and the
//! projected 2-D display copy every renderer draws.
//!
//! Renderers call [`display_view`] (or [`display_view_in`]) once at their entry.
//! A 2-D axes comes back borrowed, so 2-D output is unchanged and costs nothing.
//! A 3-D axes comes back as an owned plain 2-D axes: series points replaced by
//! projected (u, v) screen coordinates, the axes box, grid and tick marks as
//! extra legend-hidden line series, tick and axis labels as text objects, and
//! `frame_hidden` set so the renderer draws no 2-D frame, ticks or grid.
//!
//! Convention (MATLAB's documented `viewmtx`): azimuth is measured in degrees
//! from the negative y axis, counter-clockwise seen from above; elevation is
//! measured up from the x-y plane. Every axis is first scaled to the unit
//! interval of its limits (PlotBoxAspectRatio [1 1 1], stretch-to-fill).
use super::{
    Axes, AxisScale, AxisTransform, FaceStyle, FaceStyles, LinePattern, PlotKind,
    ResolvedAppearance, Series, TextAlign, TextObject, VerticalAlign,
};
use crate::Result;
use std::borrow::Cow;

/// `view(2)`: the 2-D view.
pub const VIEW_2D: [f64; 2] = [0., 90.];
/// `view(3)`: MATLAB's default 3-D view.
pub const VIEW_3D: [f64; 2] = [-37.5, 30.];
/// Desktop drag rotation: degrees per pixel of pointer movement.
pub const DRAG_DEGREES_PER_PIXEL: f64 = 0.5;

/// Tick-mark length and label offsets as fractions of the unit box or of the
/// projected box span.
const TICK_LENGTH: f64 = 0.02;
const TICK_LABEL_GAP: f64 = 0.05;
const AXIS_LABEL_GAP: f64 = 0.15;
/// The projected box fills this share of the display limits; the rest is room
/// for tick and axis labels.
const BOX_SHARE: f64 = 0.78;

pub fn validate_view(view: [f64; 2]) -> Result<()> {
    if !view.iter().all(|v| v.is_finite() && v.abs() <= 1e6) {
        return Err("View requires finite azimuth and elevation in degrees".into());
    }
    Ok(())
}

/// An axes is 3-D when its view is not (0, 90).
pub fn is_3d(view: [f64; 2]) -> bool {
    view != VIEW_2D
}

/// Exact sine and cosine of degrees: multiples of 90 give exact 0 and +-1
/// (as MATLAB's sind/cosd), other angles use the portable `libm` kernels.
fn sin_cos_degrees(degrees: f64) -> (f64, f64) {
    let reduced = degrees % 360.;
    if reduced % 90. == 0. {
        let quarter = ((reduced / 90.) as i64).rem_euclid(4);
        return match quarter {
            0 => (0., 1.),
            1 => (1., 0.),
            2 => (0., -1.),
            _ => (-1., 0.),
        };
    }
    let radians = reduced.to_radians();
    (libm::sin(radians), libm::cos(radians))
}

/// MATLAB's documented `viewmtx(az, el)`: the 4x4 orthographic view matrix, row
/// major. Row 1 gives screen x, row 2 screen y, row 3 the depth toward the
/// viewer.
pub fn viewmtx(azimuth: f64, elevation: f64) -> [[f64; 4]; 4] {
    let (sa, ca) = sin_cos_degrees(azimuth);
    let (se, ce) = sin_cos_degrees(elevation);
    [
        [ca, sa, 0., 0.],
        [-se * sa, se * ca, ce, 0.],
        [ce * sa, -ce * ca, se, 0.],
        [0., 0., 0., 1.],
    ]
}

/// (u, v, depth) of one point already scaled to the unit box.
pub fn project(matrix: &[[f64; 4]; 4], point: [f64; 3]) -> [f64; 3] {
    let row = |r: &[f64; 4]| r[0] * point[0] + r[1] * point[1] + r[2] * point[2] + r[3];
    [row(&matrix[0]), row(&matrix[1]), row(&matrix[2])]
}

/// Roundoff allowance when deciding whether a point sits on a box face.
const EDGE: f64 = 1e-9;

/// Liang-Barsky: the parameter range [t0, t1] within [0, 1] of the segment
/// a + t (b - a) that lies inside the unit cube; None when it misses the cube.
fn clip_unit_segment(a: [f64; 3], b: [f64; 3]) -> Option<(f64, f64)> {
    let (mut t0, mut t1) = (0_f64, 1_f64);
    for axis in 0..3 {
        let d = b[axis] - a[axis];
        // Inside the slab when -d t <= a and d t <= 1 - a.
        for (p, q) in [(-d, a[axis]), (d, 1. - a[axis])] {
            if p == 0. {
                if q < 0. {
                    return None;
                }
            } else {
                let r = q / p;
                if p < 0. {
                    if r > t1 {
                        return None;
                    }
                    t0 = t0.max(r);
                } else {
                    if r < t0 {
                        return None;
                    }
                    t1 = t1.min(r);
                }
            }
        }
    }
    Some((t0, t1))
}

/// The point at parameter `t` of a segment; the ends are returned exactly and
/// a cut point is held inside the unit cube against roundoff.
fn point_along(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    if t == 0. {
        a
    } else if t == 1. {
        b
    } else {
        std::array::from_fn(|axis| (a[axis] + t * (b[axis] - a[axis])).clamp(0., 1.))
    }
}

/// Scale one coordinate to the unit interval of its (transformed) limits.
pub fn unit(value: f64, limits: [f64; 2]) -> f64 {
    (value - limits[0]) / (limits[1] - limits[0])
}

/// Desktop left-drag rotation: azimuth follows dx, elevation follows -dy,
/// each at [`DRAG_DEGREES_PER_PIXEL`]. Elevation is clamped to [-90, 90] and
/// azimuth kept in (-180, 180].
pub fn drag_view(view: [f64; 2], [dx, dy]: [f64; 2]) -> [f64; 2] {
    let mut azimuth = view[0] + dx * DRAG_DEGREES_PER_PIXEL;
    if azimuth.is_finite() {
        azimuth = (azimuth + 180.).rem_euclid(360.) - 180.;
        if azimuth == -180. {
            azimuth = 180.;
        }
    } else {
        azimuth = view[0];
    }
    let elevation = view[1] - dy * DRAG_DEGREES_PER_PIXEL;
    let elevation = if elevation.is_finite() {
        elevation.clamp(-90., 90.)
    } else {
        view[1]
    };
    [azimuth, elevation]
}

impl Axes {
    /// True when the axes is viewed in 3-D (its view is not (0, 90)).
    pub fn is_3d(&self) -> bool {
        is_3d(self.view)
    }
    /// Resolved z limits: explicit `zlim`, else the data range of visible
    /// series (2-D series sit at z = 0) under the shared automatic rule.
    pub fn zbounds(&self) -> [f64; 2] {
        if let Some(limits) = self.zlim
            && limits.iter().all(|v| v.is_finite())
        {
            return limits;
        }
        let mut range = [f64::INFINITY, f64::NEG_INFINITY];
        for s in self.series.iter().filter(|s| s.visible) {
            if s.is_3d() {
                for (point, &z) in s.points.iter().zip(s.z.iter()) {
                    if point.iter().all(|v| v.is_finite()) && z.is_finite() {
                        range[0] = range[0].min(z);
                        range[1] = range[1].max(z);
                    }
                }
            } else if s
                .points
                .iter()
                .any(|point| point.iter().all(|v| v.is_finite()))
            {
                range[0] = range[0].min(0.);
                range[1] = range[1].max(0.);
            }
        }
        for volume in self.extras.volumes.iter().filter(|v| v.visible) {
            let (_, _, vz) = volume.extent();
            range[0] = range[0].min(vz[0]);
            range[1] = range[1].max(vz[1]);
        }
        super::resolve_limits(self.zlim, range, AxisTransform::default())
    }
}

/// The display form of `axes` under the legacy light appearance.
pub fn display_view(axes: &Axes) -> Cow<'_, Axes> {
    display_view_in(axes, ResolvedAppearance::default())
}

fn contour_slice_map(axes: &Axes) -> Option<std::sync::Arc<Vec<[f64; 3]>>> {
    axes.series
        .iter()
        .any(|s| s.slice_contour_level.is_some())
        .then(|| axes.extras.effective_colormap())
}

fn paint_contour_slice(series: &mut Series, map: &[[f64; 3]], clim: [f64; 2]) {
    if let Some(level) = series.slice_contour_level.take() {
        // Use the existing line stroke so an open contour stays open.
        series.kind = PlotKind::Line;
        series.color = super::colormap::scaled_rgb(map, clim, level);
    }
}

fn display_flat(axes: &Axes) -> Cow<'_, Axes> {
    let Some(map) = contour_slice_map(axes) else {
        return Cow::Borrowed(axes);
    };
    let mut display = axes.clone();
    let clim = axes.color_limits();
    display.extras.clim = Some(clim);
    for s in &mut display.series {
        paint_contour_slice(s, &map, clim);
    }
    Cow::Owned(display)
}

/// The display form of `axes`; box and grid colours resolve against `appearance`.
pub fn display_view_in(axes: &Axes, appearance: ResolvedAppearance) -> Cow<'_, Axes> {
    if !axes.is_3d() {
        return display_flat(axes);
    }
    Cow::Owned(Projection::new(axes).display(axes, appearance, true))
}

/// The same display axes for UI painting over geometry prepared earlier with
/// [`display_view`]: identical series list, labels and limits, but the data
/// series keep their shared samples instead of being projected again.
pub fn display_frame_in(axes: &Axes, appearance: ResolvedAppearance) -> Cow<'_, Axes> {
    if !axes.is_3d() {
        return display_flat(axes);
    }
    Cow::Owned(Projection::new(axes).display(axes, appearance, false))
}

/// Display copy of a whole figure: every 3-D axes replaced by its projection.
pub fn display_figure(figure: &super::Figure) -> Cow<'_, super::Figure> {
    display_figure_in(figure, ResolvedAppearance::default())
}

pub fn display_figure_in(
    figure: &super::Figure,
    appearance: ResolvedAppearance,
) -> Cow<'_, super::Figure> {
    if !figure
        .all_axes()
        .any(|(_, a)| a.is_3d() || a.series.iter().any(|s| s.slice_contour_level.is_some()))
    {
        return Cow::Borrowed(figure);
    }
    let mut copy = figure.clone();
    for axes in copy.all_axes_mut() {
        if let Cow::Owned(display) = display_view_in(axes, appearance) {
            *axes = display;
        }
    }
    Cow::Owned(copy)
}

/// One resolved 3-D view of an axes.
struct Projection {
    matrix: [[f64; 4]; 4],
    /// x, y, z limits in display (log-transformed) coordinates.
    limits: [[f64; 2]; 3],
    transforms: [AxisTransform; 3],
    /// Data-space limits for tick values.
    data_limits: [[f64; 2]; 3],
    /// Unit-box coordinate of each axis' back plane (farther from the viewer).
    back: [f64; 3],
}

impl Projection {
    fn new(axes: &Axes) -> Self {
        let (x, y) = axes.bounds();
        let z = axes.zbounds();
        let [tx, ty] = axes.axis_transforms();
        let transforms = [tx, ty, AxisTransform::default()];
        let data_limits = [x, y, z];
        let mut limits = [[0.; 2]; 3];
        for (axis, (range, transform)) in data_limits.iter().zip(transforms).enumerate() {
            let mut forward = range.map(|v| transform.forward(v));
            if !forward.iter().all(|v| v.is_finite()) || forward[0] >= forward[1] {
                forward = [0., 1.];
            }
            limits[axis] = forward;
        }
        let matrix = viewmtx(axes.view[0], axes.view[1]);
        // Row 3 points toward the viewer: the plane with the smaller depth
        // contribution is the back one. Edge-on planes take the lower limit.
        let back = std::array::from_fn(|axis| if matrix[2][axis] < 0. { 1. } else { 0. });
        Self {
            matrix,
            limits,
            transforms,
            data_limits,
            back,
        }
    }
    /// Unit-box coordinates of a data point; None when a coordinate is not
    /// finite. The point may lie outside the box. Roundoff within [`EDGE`] of a
    /// face is snapped onto it, so a point exactly at a limit counts as inside.
    fn unit_coordinates(&self, point: [f64; 3]) -> Option<[f64; 3]> {
        let mut result = [0.; 3];
        for axis in 0..3 {
            let value = self.transforms[axis].forward(point[axis]);
            let scaled = unit(value, self.limits[axis]);
            if !scaled.is_finite() {
                return None;
            }
            result[axis] = if (-EDGE..0.).contains(&scaled) {
                0.
            } else if (1. ..=1. + EDGE).contains(&scaled) {
                1.
            } else {
                scaled
            };
        }
        Some(result)
    }
    /// Unit-box coordinates of a data point, None outside the limits.
    fn unit_point(&self, point: [f64; 3]) -> Option<[f64; 3]> {
        self.unit_coordinates(point)
            .filter(|p| p.iter().all(|v| (0. ..=1.).contains(v)))
    }
    fn screen(&self, unit_point: [f64; 3]) -> [f64; 2] {
        let [u, v, _] = project(&self.matrix, unit_point);
        [u, v]
    }

    /// Depth sort (painter's algorithm) of the surface series of an axes:
    /// Patch series with z (surf, mesh). Each face's key is the mean depth
    /// (third `viewmtx` row, toward the viewer) of its projected vertices;
    /// faces are emitted far to near, ties in their original order (series
    /// order, then face order), with the per-face colour lists permuted
    /// alike. Several visible surfaces are merged into one sorted draw list
    /// in the first one's slot; each face keeps its own surface's style (fill
    /// alpha, outline, line width and style) through a per-face style table.
    /// The other surfaces keep their slot, own style and legend entry with no
    /// faces. Lines, markers and 2-D patches keep creation order.
    fn sorted_surfaces(&self, axes: &Axes) -> Vec<Option<SurfaceDraw>> {
        let mut result: Vec<Option<SurfaceDraw>> = vec![None; axes.series.len()];
        let members: Vec<usize> = axes
            .series
            .iter()
            .enumerate()
            .filter(|(_, s)| s.visible && is_surface(s))
            .map(|(i, _)| i)
            .collect();
        let Some(&first) = members.first() else {
            return result;
        };
        let mut faces = Vec::new();
        for (slot, &i) in members.iter().enumerate() {
            let slot = u8::try_from(slot).unwrap_or(u8::MAX);
            self.surface_faces(&axes.series[i], i, slot, &mut faces);
        }
        faces.sort_by(|a, b| a.depth.total_cmp(&b.depth));
        result[first] = Some(self.emit_faces(&axes.series, &members, &faces));
        for &i in &members[1..] {
            result[i] = Some(SurfaceDraw::default());
        }
        result
    }

    /// Depth keys of the faces of one surface series; a face with a
    /// non-finite vertex sorts last (it is not drawn).
    fn surface_faces(&self, series: &Series, index: usize, slot: u8, out: &mut Vec<DepthFace>) {
        let width = series.patch.map_or(4, |p| p.vertices_per_face());
        let count = series.patch_face_count();
        out.reserve(count);
        for face in 0..count {
            let mut depth = 0.;
            for corner in 0..width {
                let k = face * width + corner;
                let [x, y] = series.points.get(k).unwrap_or([f64::NAN; 2]);
                let z = series.z.get(k).copied().unwrap_or(f64::NAN);
                depth += self
                    .unit_coordinates([x, y, z])
                    .map_or(f64::NAN, |p| project(&self.matrix, p)[2]);
            }
            out.push(DepthFace {
                depth: depth / width as f64,
                series: index,
                slot,
                face,
            });
        }
    }

    /// Projected vertices, per-face colours and (several surfaces) per-face
    /// styles of sorted faces. `members` are the surfaces' series indices.
    fn emit_faces(&self, series: &[Series], members: &[usize], faces: &[DepthFace]) -> SurfaceDraw {
        let head = &series[members[0]];
        let merged = members.len() > 1;
        // A merged display stream has one width. Duplicate the last corner
        // of triangles when a quad surface is present so all faces stay in
        // depth order and use their original indices and styles.
        let patch = members
            .iter()
            .filter_map(|&i| series[i].patch)
            .max_by_key(|face| face.vertices_per_face());
        let width = patch.map_or(4, |face| face.vertices_per_face());
        let mut points = Vec::with_capacity(faces.len() * width);
        let fills = merged || head.face_colors.is_some();
        let edges = if merged {
            members.iter().any(|&i| series[i].has_face_outline())
        } else {
            head.face_edge_colors.is_some()
        };
        let mut fill = Vec::with_capacity(if fills { faces.len() } else { 0 });
        let mut edge = Vec::with_capacity(if edges { faces.len() } else { 0 });
        let mut owner = Vec::with_capacity(if merged { faces.len() } else { 0 });
        for f in faces {
            let s = &series[f.series];
            let source_width = s.patch.map_or(4, |face| face.vertices_per_face());
            for corner in 0..width {
                let k = f.face * source_width + corner.min(source_width - 1);
                let [x, y] = s.points.get(k).unwrap_or([f64::NAN; 2]);
                let z = s.z.get(k).copied().unwrap_or(f64::NAN);
                points.push(
                    self.unit_point([x, y, z])
                        .map_or([f64::NAN; 2], |p| self.screen(p)),
                );
            }
            if fills {
                fill.push(s.face_fill_color(f.face).unwrap_or(s.color));
            }
            if edges {
                edge.push(s.face_outline_color(f.face).unwrap_or([0; 3]));
            }
            if merged {
                owner.push(f.slot);
            }
        }
        let styles = merged.then(|| {
            let table = members.iter().map(|&i| FaceStyle::of(&series[i])).collect();
            std::sync::Arc::new(FaceStyles::new(table, owner))
        });
        SurfaceDraw {
            points,
            patch,
            fill: fills.then(|| fill.into()),
            edges: edges.then(|| edge.into()),
            styles,
        }
    }

    /// The projected-point loop for the samples of a data series that are
    /// drawn one by one (markers, patches, lines without a stroke). Points
    /// outside the limits or with a non-finite coordinate are not drawn.
    /// Surface faces are depth sorted by [`Self::sorted_surfaces`] instead.
    fn series_points(&self, series: &Series) -> Vec<[f64; 2]> {
        let mut output = Vec::with_capacity(series.points.len());
        for (index, [x, y]) in series.points.iter().enumerate() {
            let z = if series.is_3d() { series.z[index] } else { 0. };
            output.push(
                self.unit_point([x, y, z])
                    .map_or([f64::NAN; 2], |p| self.screen(p)),
            );
        }
        output
    }

    /// The projected polyline of a stroked line series, cut at the faces of
    /// the unit box: a segment fully outside is dropped, a crossing one ends
    /// at the face. Non-finite coordinates and dropped parts break the line.
    fn clipped_line(&self, series: &Series) -> Vec<[f64; 2]> {
        let mut output: Vec<[f64; 2]> = Vec::with_capacity(series.points.len());
        let emit = |output: &mut Vec<[f64; 2]>, point: [f64; 3], new_run: bool| {
            if new_run && output.last().is_some_and(|last| !last[0].is_nan()) {
                output.push([f64::NAN; 2]);
            }
            output.push(self.screen(point));
        };
        let mut previous: Option<[f64; 3]> = None;
        // The last emitted point is `previous` itself (the run continues).
        let mut connected = false;
        for (index, [x, y]) in series.points.iter().enumerate() {
            let z = if series.is_3d() { series.z[index] } else { 0. };
            let Some(point) = self.unit_coordinates([x, y, z]) else {
                previous = None;
                connected = false;
                continue;
            };
            match previous {
                None => {
                    connected = point.iter().all(|v| (0. ..=1.).contains(v));
                    if connected {
                        emit(&mut output, point, true);
                    }
                }
                Some(from) => {
                    let inside = |p: &[f64; 3]| p.iter().all(|v| (0. ..=1.).contains(v));
                    let range = if inside(&from) && inside(&point) {
                        Some((0., 1.))
                    } else {
                        clip_unit_segment(from, point)
                    };
                    if let Some((t0, t1)) = range {
                        if !(connected && t0 == 0.) {
                            emit(&mut output, point_along(from, point, t0), true);
                        }
                        emit(&mut output, point_along(from, point, t1), false);
                        connected = t1 == 1.;
                    } else {
                        connected = false;
                    }
                }
            }
            previous = Some(point);
        }
        output
    }

    fn display(&self, axes: &Axes, appearance: ResolvedAppearance, project: bool) -> Axes {
        let colors = axes.resolve_colors(appearance);
        let mut display = axes.clone();
        display.series.clear();
        for volume in &mut display.extras.volumes {
            volume.projected_bounds = Some(super::projected_display_bounds(axes, volume));
            volume.display_view = Some(axes.view);
        }
        let contour_map = contour_slice_map(axes);
        let clim = axes.color_limits();
        if contour_map.is_some() {
            display.extras.clim = Some(clim);
        }
        // Marker lines are split (see below); their cut polylines follow the
        // data series so the data series keep their indices.
        let mut strokes = Vec::new();
        let mut surfaces = self.sorted_surfaces(axes);
        for (index, s) in axes.series.iter().enumerate() {
            let mut copy = s.clone();
            copy.z = std::sync::Arc::from([]);
            copy.indexed_patch = None;
            if let Some(map) = &contour_map {
                paint_contour_slice(&mut copy, map, clim);
            }
            if let Some(draw) = surfaces[index].take() {
                copy.points = draw.points.into();
                copy.patch = draw.patch.or(copy.patch);
                // The series keeps its own colour, edge colour, alpha and
                // line style: the legend reads them.
                copy.face_colors = draw.fill;
                copy.face_edge_colors = draw.edges;
                // The merged faces live on the first mesh. Its `face_color_none`
                // would hide a filled neighbor. Per-face styles already give an
                // indexed unfilled mesh alpha 0, so drop only this host flag.
                // A series drawn in its own slot keeps the flag and is not filled.
                let merged = draw.styles.is_some();
                copy.face_styles = draw.styles;
                if merged && s.indexed_patch.is_some() && s.face_color_none {
                    copy.face_color_none = false;
                }
                display.series.push(copy);
                continue;
            }
            match s.kind {
                PlotKind::Line | PlotKind::Scatter | PlotKind::Patch => {}
                PlotKind::Stairs | PlotKind::Stem => {
                    // Their 2-D geometry (risers, stems) is not projected: draw
                    // the samples as a plain line in 3-D.
                    copy.kind = PlotKind::Line;
                    copy.markers = s.markers && s.kind == PlotKind::Stairs;
                }
                // Bars have no 3-D form yet (bar3 is a later task).
                PlotKind::Bar => copy.visible = false,
            }
            let stroked = copy.kind == PlotKind::Line && copy.pattern != LinePattern::None;
            // A cut line ends on a box face where no sample sits, so a marker
            // line is drawn as two series: this one keeps the style (legend
            // sample, visibility) and shows the in-box markers only, the
            // legend-hidden stroke carries the cut polyline. The split depends
            // on the style alone, so the paint copy has the same series list.
            let mut stroke = (stroked && copy.markers).then(|| {
                let mut line = copy.clone();
                line.identity = Default::default();
                line.baseline_identity = Default::default();
                line.markers = false;
                line.label.clear();
                line.legend_hidden = true;
                line
            });
            if project && copy.visible {
                match (&mut stroke, stroked) {
                    (Some(line), _) => {
                        line.points = self.clipped_line(s).into();
                        let mut markers = Vec::new();
                        for point in self.series_points(s) {
                            markers.extend([point, [f64::NAN; 2]]);
                        }
                        copy.points = markers.into();
                    }
                    (None, true) => copy.points = self.clipped_line(s).into(),
                    (None, false) => copy.points = self.series_points(s).into(),
                }
            }
            display.series.push(copy);
            strokes.extend(stroke);
        }
        display.series.extend(strokes);
        display.baselines.clear();
        display.extras.yy = None;
        display.extras.texts.clear();
        for text in &axes.extras.texts {
            let mut copy = text.clone();
            match self.unit_point(text.position) {
                Some(p) => {
                    let [u, v] = self.screen(p);
                    copy.position = [u, v, 0.];
                }
                None => copy.visible = false,
            }
            display.extras.texts.push(copy);
        }
        let blend_alpha = |rgb: [u8; 3], alpha: f64| -> [u8; 3] {
            let alpha = alpha.clamp(0., 1.);
            std::array::from_fn(|i| {
                (f64::from(rgb[i]) * alpha + f64::from(colors.background[i]) * (1. - alpha)).round()
                    as u8
            })
        };
        let blend = |rgb: [u8; 3]| blend_alpha(rgb, colors.grid_alpha);
        // MATLAB's MinorGridAlpha where the major grid is translucent.
        let minor_alpha = if axes.colors.is_some_and(|c| c.grid.is_some()) || !appearance.legacy {
            super::MINOR_GRID_ALPHA
        } else {
            colors.grid_alpha
        };
        let ticks = [0, 1, 2].map(|axis| self.ticks(axes, axis));
        let frame = self.frame();
        // Grid lines on the three back planes, one series per grid colour;
        // minor grids first and dotted, so major lines draw over them.
        let grids = [axes.xgrid, axes.ygrid, axes.zgrid];
        let grid_colors = [colors.grid_x, colors.grid_y, colors.grid_y];
        let plane_lines = |axis: usize, positions: &mut dyn Iterator<Item = f64>| {
            let mut points = Vec::new();
            for position in positions {
                for plane in (0..3).filter(|&p| p != axis) {
                    // On back plane `plane`, run along the remaining axis.
                    let along = 3 - axis - plane;
                    let mut start = [0.; 3];
                    start[axis] = position;
                    start[plane] = self.back[plane];
                    let mut end = start;
                    end[along] = 1.;
                    points.extend([self.screen(start), self.screen(end), [f64::NAN; 2]]);
                }
            }
            points
        };
        for axis in (0..3).filter(|&axis| axes.minor_grid[axis]) {
            let positions = self.minor_ticks(axis);
            let points = plane_lines(axis, &mut positions.into_iter());
            let mut minor = decoration(points, blend_alpha(grid_colors[axis], minor_alpha));
            minor.pattern = LinePattern::Dotted;
            minor.line_width = Some(0.5);
            display.series.push(minor);
        }
        for axis in 0..3 {
            if !grids[axis] {
                continue;
            }
            let points = plane_lines(axis, &mut ticks[axis].iter().map(|&(p, _)| p));
            display
                .series
                .push(decoration(points, blend(grid_colors[axis])));
        }
        // Box: outer edges of the back planes (Box on, BoxStyle 'back') plus
        // the three tick-carrying axis lines with their tick marks.
        let mut points = Vec::new();
        if axes.box_outline {
            for [a, b] in self.back_plane_outer_edges() {
                points.extend([self.screen(a), self.screen(b), [f64::NAN; 2]]);
            }
        }
        for (axis, edge) in frame.edges.iter().enumerate() {
            points.extend([self.screen(edge[0]), self.screen(edge[1]), [f64::NAN; 2]]);
            for &(position, _) in &ticks[axis] {
                let mut start = edge[0];
                start[axis] = position;
                let mut end = start;
                let inward = frame.inward[axis];
                end[inward] += if start[inward] > 0.5 {
                    -TICK_LENGTH
                } else {
                    TICK_LENGTH
                };
                points.extend([self.screen(start), self.screen(end), [f64::NAN; 2]]);
            }
        }
        display.series.push(decoration(points, colors.x));
        // Limits: the projected box with equal u/v scale, padded for labels.
        let corners =
            (0..8).map(|i| self.screen([(i & 1) as f64, ((i >> 1) & 1) as f64, (i >> 2) as f64]));
        let mut bounds = [
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ];
        for [u, v] in corners {
            bounds = [
                bounds[0].min(u),
                bounds[1].max(u),
                bounds[2].min(v),
                bounds[3].max(v),
            ];
        }
        let center = [(bounds[0] + bounds[1]) / 2., (bounds[2] + bounds[3]) / 2.];
        let span = (bounds[1] - bounds[0]).max(bounds[3] - bounds[2]).max(1e-6);
        let half = span / BOX_SHARE / 2.;
        display.xlim = Some([center[0] - half, center[0] + half]);
        display.ylim = Some([center[1] - half, center[1] + half]);
        // Tick labels and axis labels as text objects.
        let font = axes.extras.font_size.unwrap_or(10.);
        let labels = [&axes.xlabel, &axes.ylabel, &axes.zlabel];
        let label_colors = [colors.xlabel, colors.ylabel, colors.ylabel];
        let tick_colors = [
            colors.x,
            colors.y,
            axes.colors
                .and_then(|c| c.z)
                .map_or(colors.y, super::AxesColors::rgb8),
        ];
        let rgb = |c: [u8; 3]| c.map(|v| f64::from(v) / 255.);
        for axis in 0..3 {
            let edge = frame.edges[axis];
            let direction = if axis == 2 {
                [-1., 0.]
            } else {
                let middle = self.screen(midpoint(edge));
                let outward = [middle[0] - center[0], middle[1] - center[1]];
                let length = outward[0].hypot(outward[1]);
                if length > 1e-12 {
                    [outward[0] / length, outward[1] / length]
                } else {
                    [0., -1.]
                }
            };
            let (horizontal, vertical) = alignment(direction);
            for (position, label) in &ticks[axis] {
                let mut point = edge[0];
                point[axis] = *position;
                let [u, v] = self.screen(point);
                let gap = TICK_LABEL_GAP * span;
                let mut text = TextObject::new(
                    u + direction[0] * gap,
                    v + direction[1] * gap,
                    label.clone(),
                );
                text.font_size = font;
                text.interpreter = axes.extras.tick_label_interpreter;
                text.horizontal = horizontal;
                text.vertical = vertical;
                text.rotation = axes.extras.rulers[axis].tick_label_rotation;
                text.color = axes.colors.is_some().then(|| rgb(tick_colors[axis]));
                display.extras.texts.push(text);
            }
            if !labels[axis].is_empty() {
                let [u, v] = self.screen(midpoint(edge));
                let gap = AXIS_LABEL_GAP * span;
                let mut text = TextObject::new(
                    u + direction[0] * gap,
                    v + direction[1] * gap,
                    labels[axis].clone(),
                );
                let style = axes.extras.label_styles[axis];
                text.font_size = style.font_size.unwrap_or(font * 1.1);
                text.font_size_explicit = style.font_size.is_some();
                text.bold = style.bold.unwrap_or(false);
                text.italic = style.italic.unwrap_or(false);
                text.interpreter = axes.extras.label_interpreters[axis + 1];
                text.horizontal = TextAlign::Center;
                text.vertical = if axis == 2 {
                    VerticalAlign::Bottom
                } else {
                    vertical
                };
                if axis == 2 {
                    text.rotation = 90.;
                }
                text.color = axes.colors.is_some().then(|| rgb(label_colors[axis]));
                display.extras.texts.push(text);
            }
        }
        display.xlabel.clear();
        display.ylabel.clear();
        display.zlabel.clear();
        display.xscale = AxisScale::Linear;
        display.yscale = AxisScale::Linear;
        display.xgrid = false;
        display.ygrid = false;
        display.zgrid = false;
        display.minor_grid = [false; 3];
        display.zlim = None;
        display.view = VIEW_2D;
        display.frame_hidden = true;
        display
    }

    /// Ticks of one axis: (unit-box position, label).
    fn ticks(&self, axes: &Axes, axis: usize) -> Vec<(f64, String)> {
        let transform = self.transforms[axis];
        let range = self.data_limits[axis];
        let limits = self.limits[axis];
        let ruler = &axes.extras.rulers[axis];
        let (raw_values, step) = if let Some(manual) = &ruler.ticks {
            let step = if manual.len() >= 2 {
                super::manual_tick_step(manual)
            } else {
                1.
            };
            (manual.clone(), step)
        } else if !ruler.kind.is_numeric() {
            axes.effective_ticks(axis)
        } else if transform.scale == AxisScale::Log {
            (transform.ticks(limits), 0.)
        } else {
            super::svg::ticks(range)
        };
        let values: Vec<(f64, String)> = raw_values
            .into_iter()
            .enumerate()
            .map(|(idx, t)| {
                let label = if let Some(manual_labels) = &ruler.tick_labels {
                    manual_labels.get(idx).cloned().unwrap_or_default()
                } else if !ruler.kind.is_numeric()
                    || ruler.ticks.is_some()
                    || ruler.exponent != 0
                    || ruler.tick_label_format.is_some()
                {
                    let data_val = if transform.scale == AxisScale::Log && ruler.ticks.is_none() {
                        transform.inverse(t)
                    } else {
                        t
                    };
                    axes.format_tick_label(axis, data_val, step, transform)
                } else if transform.scale == AxisScale::Log && ruler.ticks.is_none() {
                    transform.label(t)
                } else {
                    super::svg::tick_text(t, step)
                };
                (t, label)
            })
            .collect();
        values
            .into_iter()
            .filter_map(|(value, label)| {
                let forward = if transform.scale == AxisScale::Log && ruler.ticks.is_none() {
                    value
                } else {
                    transform.forward(value)
                };
                let position = unit(forward, limits);
                (position.is_finite() && (-1e-9..=1. + 1e-9).contains(&position))
                    .then_some((position.clamp(0., 1.), label))
            })
            .collect()
    }

    /// Minor tick positions of one axis in unit-box coordinates.
    fn minor_ticks(&self, axis: usize) -> Vec<f64> {
        let transform = self.transforms[axis];
        let limits = self.limits[axis];
        let display = if transform.scale == AxisScale::Log {
            super::minor_ticks(transform, limits, 0.)
        } else {
            let range = self.data_limits[axis];
            let (_, step) = super::svg::ticks(range);
            super::minor_ticks(transform, range, step)
                .into_iter()
                .map(|v| transform.forward(v))
                .collect()
        };
        display
            .into_iter()
            .map(|v| unit(v, limits))
            .filter(|p| p.is_finite() && (0. ..=1.).contains(p))
            .collect()
    }

    /// The edges of the three back planes that do not touch the shared back
    /// corner.
    fn back_plane_outer_edges(&self) -> Vec<[[f64; 3]; 2]> {
        let mut edges = Vec::with_capacity(6);
        for plane in 0..3 {
            for along in (0..3).filter(|&a| a != plane) {
                // Edge parallel to `along` on plane `plane`; the third axis sits
                // at the front value (away from the back corner).
                let fixed = 3 - plane - along;
                let mut start = [0.; 3];
                start[plane] = self.back[plane];
                start[fixed] = 1. - self.back[fixed];
                let mut end = start;
                start[along] = 0.;
                end[along] = 1.;
                edges.push([start, end]);
            }
        }
        edges
    }

    fn frame(&self) -> Frame {
        let lowest = |candidates: [[[f64; 3]; 2]; 2]| {
            let key = |edge: &[[f64; 3]; 2]| self.screen(midpoint(*edge));
            let (a, b) = (key(&candidates[0]), key(&candidates[1]));
            // Lowest on screen; a tie goes to the left one.
            if (a[1] - b[1]).abs() > 1e-12 {
                if a[1] < b[1] {
                    candidates[0]
                } else {
                    candidates[1]
                }
            } else if a[0] <= b[0] {
                candidates[0]
            } else {
                candidates[1]
            }
        };
        let segment = |axis: usize, fixed: [f64; 3]| {
            let mut start = fixed;
            let mut end = fixed;
            start[axis] = 0.;
            end[axis] = 1.;
            [start, end]
        };
        // x and y labels sit on bottom-face (z = lower limit) edges.
        let x = lowest([segment(0, [0., 0., 0.]), segment(0, [0., 1., 0.])]);
        let y = lowest([segment(1, [0., 0., 0.]), segment(1, [1., 0., 0.])]);
        // z labels sit on the vertical edge furthest left on screen.
        let mut z = segment(2, [0., 0., 0.]);
        let mut left = f64::INFINITY;
        for corner in [[0., 0.], [1., 0.], [0., 1.], [1., 1.]] {
            let edge = segment(2, [corner[0], corner[1], 0.]);
            let u = self.screen(midpoint(edge))[0];
            if u < left - 1e-12 {
                left = u;
                z = edge;
            }
        }
        Frame {
            edges: [x, y, z],
            // Tick marks point into the box along y, x and x respectively.
            inward: [1, 0, 0],
        }
    }
}

/// A surface series: a Patch with z (surf, mesh).
fn is_surface(series: &Series) -> bool {
    series.kind == PlotKind::Patch && series.patch.is_some() && series.is_3d()
}

/// Depth key of one surface face: its series, that series' position among
/// the axes' surfaces (its style-table slot) and its face index.
struct DepthFace {
    depth: f64,
    series: usize,
    slot: u8,
    face: usize,
}

/// The display form of one surface slot: sorted projected faces.
#[derive(Clone, Default)]
struct SurfaceDraw {
    points: Vec<[f64; 2]>,
    patch: Option<super::PatchFaces>,
    fill: Option<std::sync::Arc<[[u8; 3]]>>,
    edges: Option<std::sync::Arc<[[u8; 3]]>>,
    styles: Option<std::sync::Arc<FaceStyles>>,
}

/// The three tick-carrying axis lines (x, y, z) in unit-box coordinates.
struct Frame {
    edges: [[[f64; 3]; 2]; 3],
    inward: [usize; 3],
}

fn midpoint([a, b]: [[f64; 3]; 2]) -> [f64; 3] {
    std::array::from_fn(|i| (a[i] + b[i]) / 2.)
}

fn alignment(direction: [f64; 2]) -> (TextAlign, VerticalAlign) {
    let horizontal = if direction[0] < -0.3 {
        TextAlign::Right
    } else if direction[0] > 0.3 {
        TextAlign::Left
    } else {
        TextAlign::Center
    };
    let vertical = if direction[1] < -0.3 {
        VerticalAlign::Top
    } else if direction[1] > 0.3 {
        VerticalAlign::Bottom
    } else {
        VerticalAlign::Middle
    };
    (horizontal, vertical)
}

/// A legend-hidden box, grid or tick line in the axes' own colours.
fn decoration(points: Vec<[f64; 2]>, rgb: [u8; 3]) -> Series {
    let mut series = Series::new(points, PlotKind::Line);
    series.color = rgb;
    series.explicit_color = true;
    series.line_width = Some(0.5);
    series.pattern = LinePattern::Solid;
    series.legend_hidden = true;
    series
}
