use crate::{Matrix, Result};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
mod appearance;
mod axes_colors;
#[cfg(test)]
mod axes_colors_tests;
pub use axes_colors::{AxesColors, AxesGridStyle, LegendColors, ResolvedAxesColors};
mod axis;
pub use appearance::{BaseTheme, ColorMode, FigureAppearance, FigureTheme, ResolvedAppearance};
mod baseline;
pub use baseline::{
    Baseline, BaselineObservation, BaselineStyle, BaselineValue, MAX_BASELINE_MEMBERS,
    MAX_BASELINES,
};
pub mod coordinates;
mod patch;
mod points;
pub use points::{PointFace, PointIter, SeriesPoints, WeakSeriesPoints};
mod textbox;
pub use patch::{
    ClippedFace, MAX_PATCH_FACES, PatchFaces, clip_face_edge, clip_patch_face, patch_screen_point,
};
pub use textbox::{MAX_TEXTBOX_BYTES, MAX_TEXTBOXES, TextAlign, TextBox};
mod complex_data;
pub(crate) use complex_data::series_from_plot_data_with_budget;
pub use complex_data::{PlotData, series_from_plot_data};
mod layout;
mod legend;
mod legend_state;
pub(crate) use legend_state::LegendMetadata;
pub use legend_state::LegendState;
mod legend_best;
mod legend_layout;
pub use layout::{Layout, Panel, Position};
pub use legend::LegendLocation;
pub use legend_best::{
    LEGEND_PROBE_LIMIT, LegendBestCache, LegendBestPrepared, LegendBestResult, LegendBestView,
    best_legend_location,
};
pub use legend_layout::{
    AxesPlacement, LegendGeometry, LegendRect, PositionConstraint, legend_geometry,
    outer_allocations,
};
mod tiled;
pub use tiled::{Arrangement, TileGrid};
mod canvas;
mod marker;
mod sample;
mod series_palette;
mod stamp;
pub use marker::{MarkerShape, validate_marker_size};
pub use series_palette::{AutoPalette, PaletteCycle, PaletteVersion};
pub(crate) use stamp::SnapshotStamp;
mod svg;
pub use axis::{AxisScale, AxisTransform};
pub use canvas::Canvas;
pub use sample::{SampleHit, SampleQuery};
/// Axis transforms paired with their two data-coordinate bounds.
pub type DisplayAxesBounds = ([AxisTransform; 2], ([f64; 2], [f64; 2]));

pub const MAX_SERIES: usize = 64;
pub const MAX_POINTS: usize = 1_000_000;
pub const COLORS: [[u8; 3]; 7] = [
    [0, 114, 189],
    [217, 83, 25],
    [237, 177, 32],
    [126, 47, 142],
    [119, 172, 48],
    [77, 190, 238],
    [162, 20, 47],
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum PlotKind {
    #[default]
    Line,
    Scatter,
    Bar,
    Stairs,
    Stem,
    Patch,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
pub enum LinePattern {
    // Preserve the old variants, discriminants and serde names.
    #[default]
    Solid = 0,
    Dashed = 1,
    Dotted = 2,
    DashDot = 3,
    None = 4,
}
#[derive(Clone, Debug)]
pub struct Series {
    /// Process-local identity; cloned snapshots retain it, decoded objects do not.
    pub identity: crate::ObjectIdentity,
    /// Latent chart-baseline token. Shared by an admitted Bar group, independent per Stem.
    /// Clones retain it; imports create fresh identities. It retains no samples.
    pub baseline_identity: crate::ObjectIdentity,
    /// Implicit group policy. Native absence decodes false; new Bar charts use true.
    pub bar_baseline_visible: bool,
    pub points: SeriesPoints,
    pub patch: Option<PatchFaces>,
    /// None retains the historical automatic stem baseline.
    pub base_value: Option<f64>,
    /// Independent constant Bar/Patch outline. None preserves historical fill-only appearance.
    pub face_edge_color: Option<[u8; 3]>,
    pub label: String,
    pub kind: PlotKind,
    pub color: [u8; 3],
    pub explicit_color: bool,
    /// None preserves stored historical RGB; never infer this from the color.
    pub auto_palette: Option<AutoPalette>,
    pub pattern: LinePattern,
    /// Explicit stroke width in typographic points; None preserves legacy defaults.
    pub line_width: Option<f64>,
    pub markers: bool,
    pub marker_shape: Option<MarkerShape>,
    pub marker_size: Option<f64>,
    pub marker_filled: bool,
    pub visible: bool,
    pub bar_width: f64,
    pub bar_offset: f64,
}
impl Series {
    pub fn new(points: Vec<[f64; 2]>, kind: PlotKind) -> Self {
        Self::with_points(points.into(), kind)
    }
    /// Real storage-bearing construction; normal Figure admission still validates
    /// kind, topology, style, identities and the complete resource ledger.
    pub fn with_points(points: SeriesPoints, kind: PlotKind) -> Self {
        Self {
            identity: crate::ObjectIdentity::default(),
            baseline_identity: crate::ObjectIdentity::default(),
            bar_baseline_visible: kind == PlotKind::Bar,
            points,
            patch: None,
            base_value: None,
            face_edge_color: None,
            label: String::new(),
            kind,
            color: COLORS[0],
            explicit_color: false,
            auto_palette: None,
            pattern: LinePattern::Solid,
            line_width: None,
            markers: kind == PlotKind::Stem,
            marker_shape: None,
            marker_size: None,
            marker_filled: false,
            visible: true,
            bar_width: 0.8,
            bar_offset: 0.,
        }
    }
    /// Checked resource accounting. Full model validation additionally checks
    /// style, face geometry and identities before publication.
    pub fn geometry_cost(&self) -> Result<coordinates::GeometryCost> {
        match (&self.points, self.kind, self.patch) {
            (SeriesPoints::Patch(points), PlotKind::Patch, Some(faces))
                if points.corners_per_face() == faces.vertices_per_face() =>
            {
                Ok(points.cost())
            }
            (SeriesPoints::Patch(_), _, _) => {
                Err("Stored Patch coordinates require matching Patch topology".into())
            }
            (SeriesPoints::Dense(points), PlotKind::Patch, Some(faces)) => {
                coordinates::GeometryCost::dense_faces(points, faces.vertices_per_face())
            }
            (_, PlotKind::Patch, None) | (_, _, Some(_)) => {
                Err("Patch geometry and series kind disagree".into())
            }
            (SeriesPoints::Dense(points), PlotKind::Bar, None) => {
                coordinates::GeometryCost::bars(points)
            }
            (SeriesPoints::Dense(points), _, None) => coordinates::GeometryCost::ordinary(points),
        }
    }
    pub fn stem_baseline(&self, axis: AxisTransform) -> f64 {
        self.base_value
            .unwrap_or(if axis.scale == AxisScale::Linear {
                0.
            } else if axis.negative {
                -1.
            } else {
                1.
            })
    }
    pub fn validate_base_value(&self) -> Result<()> {
        if self
            .base_value
            .is_some_and(|v| !v.is_finite() || self.kind != PlotKind::Stem)
        {
            return Err("BaseValue must be finite and belongs only to a stem series".into());
        }
        Ok(())
    }
    pub fn has_face_style(&self) -> bool {
        self.face_edge_color.is_some()
            || (self.kind == PlotKind::Patch
                && (self.pattern != LinePattern::None || self.line_width.is_some()))
    }
    pub fn face_edge_width(&self) -> f32 {
        self.stroke_width(2. / 3.)
    }
    pub fn validate_face_edge(&self) -> Result<()> {
        if self.face_edge_color.is_some() && !matches!(self.kind, PlotKind::Bar | PlotKind::Patch) {
            return Err("Independent face outlines belong only to Bar or Patch".into());
        }
        Ok(())
    }
    pub fn stroke_width(&self, legacy: f32) -> f32 {
        self.line_width
            .map_or(legacy, |points| (points * 4. / 3.) as f32)
    }
    pub fn line_spec(&mut self, spec: &str) -> Result<()> {
        if spec.is_empty() || spec.len() > 16 {
            return Err("Invalid line specification".into());
        }
        let (pattern, token) = if spec.contains("--") {
            (LinePattern::Dashed, "--")
        } else if spec.contains("-.") {
            (LinePattern::DashDot, "-.")
        } else if spec.contains(':') {
            (LinePattern::Dotted, ":")
        } else {
            (LinePattern::Solid, "-")
        };
        let has_line = spec.contains(token);
        self.pattern = pattern;
        let rest = spec.replacen(token, "", 1);
        let mut has_color = false;
        let mut has_marker = false;
        for c in rest.chars() {
            if "o.+x*sd^v<>".contains(c) {
                if has_marker {
                    return Err("Duplicate marker".into());
                }
                self.markers = true;
                has_marker = true;
                self.marker_filled = c == '.';
                if c != 'o' && c != '.' {
                    self.marker_shape = Some(MarkerShape::parse(&c.to_string())?);
                }
                continue;
            }
            let color =
                match c {
                    'r' => [255, 0, 0],
                    'g' => [0, 255, 0],
                    'b' => [0, 0, 255],
                    'c' => [0, 255, 255],
                    'm' => [255, 0, 255],
                    'y' => [255, 255, 0],
                    'k' => [0, 0, 0],
                    'w' => [255, 255, 255],
                    _ => return Err(
                        "Supported line specs: colors rgbcmykw, lines - -- : -., markers o . + x * s d ^ v < >"
                            .into(),
                    ),
                };
            if has_color {
                return Err("Duplicate color".into());
            }
            self.color = color;
            self.explicit_color = true;
            has_color = true;
        }
        if self.markers && !has_line && self.kind == PlotKind::Line {
            self.kind = PlotKind::Scatter;
        }
        Ok(())
    }
    /// Bound UI geometry without duplicating the full-resolution data.
    pub fn display_points(&self, max_points: usize) -> Vec<[f64; 2]> {
        self.display_points_axes(max_points, [AxisTransform::default(); 2])
    }
    pub fn display_points_axes(
        &self,
        max_points: usize,
        axes: [AxisTransform; 2],
    ) -> Vec<[f64; 2]> {
        self.display_points_axes_checked(max_points, axes, &|| Ok(()))
            .expect("Infallible display checkpoint")
    }
    /// Same display geometry, with bounded cooperative cancellation during scans.
    pub fn display_points_axes_cancellable(
        &self,
        max_points: usize,
        axes: [AxisTransform; 2],
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<Vec<[f64; 2]>> {
        self.display_points_axes_checked(max_points, axes, &|| display_check(cancel))
    }
    /// Only complete, admitted face shapes enter the fixed-scratch traversal.
    /// Invalid public model combinations keep the preexisting scalar behavior.
    fn complete_patch_width(&self) -> Option<usize> {
        if self.kind != PlotKind::Patch {
            return None;
        }
        let faces = self.patch?;
        faces.face_count(self.points.len()).ok()?;
        let width = faces.vertices_per_face();
        if self
            .points
            .patch()
            .is_some_and(|stored| stored.corners_per_face() != width)
        {
            return None;
        }
        Some(width)
    }
    /// Evaluate each face once without retaining expanded coordinates. The caller
    /// established a complete shape; point order and cancellation indices match
    /// the scalar path, including a checkpoint inside a triangle when needed.
    fn visit_patch_points_checked(
        &self,
        width: usize,
        axes: [AxisTransform; 2],
        check: &impl Fn() -> Result<()>,
        mut visit: impl FnMut([f64; 2], bool),
    ) -> Result<()> {
        for face_index in 0..self.points.len() / width {
            let face = self
                .points
                .face(face_index, width)
                .expect("complete Patch face");
            let accepted = face
                .as_slice()
                .iter()
                .all(|p| axes[0].accepts(p[0]) && axes[1].accepts(p[1]));
            for (corner, &point) in face.as_slice().iter().enumerate() {
                if (face_index * width + corner).is_multiple_of(4096) {
                    check()?;
                }
                visit(point, accepted);
            }
        }
        Ok(())
    }
    fn display_points_axes_checked(
        &self,
        max_points: usize,
        axes: [AxisTransform; 2],
        check: &impl Fn() -> Result<()>,
    ) -> Result<Vec<[f64; 2]>> {
        check()?;
        if !self.visible {
            return Ok(Vec::new());
        }
        if self.kind == PlotKind::Patch {
            if let Some(width) = self.complete_patch_width() {
                let mut output = Vec::new();
                output
                    .try_reserve_exact(self.points.len())
                    .map_err(|_| "Cannot allocate Patch display geometry")?;
                self.visit_patch_points_checked(width, axes, check, |point, accepted| {
                    output.push(if accepted {
                        [axes[0].forward(point[0]), axes[1].forward(point[1])]
                    } else {
                        [f64::NAN; 2]
                    });
                })?;
                check()?;
                return Ok(output);
            }
            let output: Vec<_> = self
                .points
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    if i.is_multiple_of(4096) {
                        check()?;
                    }
                    Ok(if self.vertex_face_accepts(i, axes) {
                        [axes[0].forward(p[0]), axes[1].forward(p[1])]
                    } else {
                        [f64::NAN; 2]
                    })
                })
                .collect::<Result<_>>()?;
            check()?;
            return Ok(output);
        }
        let budget = match self.kind {
            PlotKind::Stairs => max_points.max(18).div_ceil(2),
            PlotKind::Stem => max_points.max(18) / 3,
            _ => max_points,
        };
        let mut result = self.display_points_filtered(
            budget,
            |p| axes[0].accepts(p[0]) && axes[1].accepts(p[1]),
            check,
        )?;
        // Bars need their original edges transformed separately by the renderer.
        if self.kind != PlotKind::Bar {
            for (index, p) in result.iter_mut().enumerate() {
                if index.is_multiple_of(4096) {
                    check()?;
                }
                *p = [axes[0].forward(p[0]), axes[1].forward(p[1])];
            }
        }
        check()?;
        let output = match self.kind {
            PlotKind::Stairs => stair_geometry_checked(&result, check)?,
            PlotKind::Stem => {
                let mut output = Vec::with_capacity(result.len() * 3);
                for (index, p) in result.into_iter().enumerate() {
                    if index.is_multiple_of(4096) {
                        check()?;
                    }
                    output.extend(if p.iter().all(|v| v.is_finite()) {
                        [
                            [p[0], self.base_value.map_or(0., |v| axes[1].forward(v))],
                            p,
                            [f64::NAN; 2],
                        ]
                    } else {
                        [[f64::NAN; 2]; 3]
                    });
                }
                output
            }
            _ => result,
        };
        check()?;
        Ok(output)
    }

    fn display_points_filtered(
        &self,
        max_points: usize,
        valid: impl Fn(&[f64; 2]) -> bool,
        check: &impl Fn() -> Result<()>,
    ) -> Result<Vec<[f64; 2]>> {
        let points = self
            .points
            .dense()
            .ok_or("Non-Patch series requires dense samples")?;
        let max_points = max_points.max(6);
        if points.len() <= max_points || self.kind == PlotKind::Bar {
            let mut output = Vec::with_capacity(points.len());
            for chunk in points.chunks(4096) {
                check()?;
                output.extend_from_slice(chunk);
            }
            check()?;
            return Ok(output);
        }
        if matches!(self.kind, PlotKind::Scatter | PlotKind::Stem) {
            let mut initial = 0;
            for (i, p) in points.iter().enumerate() {
                if i.is_multiple_of(4096) {
                    check()?;
                }
                if valid(p) {
                    initial = i;
                    break;
                }
            }
            let mut extremes = [initial; 4];
            for (i, p) in points.iter().enumerate() {
                if i.is_multiple_of(4096) {
                    check()?;
                }
                if valid(p) {
                    for axis in 0..2 {
                        if p[axis] < points[extremes[2 * axis]][axis] {
                            extremes[2 * axis] = i;
                        }
                        if p[axis] > points[extremes[2 * axis + 1]][axis] {
                            extremes[2 * axis + 1] = i;
                        }
                    }
                }
            }
            let mut indexes = vec![0, points.len() - 1];
            indexes.extend(extremes);
            if max_points > 6 {
                indexes.extend((0..points.len()).step_by(points.len().div_ceil(max_points - 6)));
            }
            indexes.sort_unstable();
            indexes.dedup();
            check()?;
            return Ok(indexes.into_iter().map(|i| points[i]).collect());
        }
        let bucket_size = points.len().div_ceil(max_points / 6);
        let mut result = Vec::with_capacity(max_points);
        for bucket in points.chunks(bucket_size) {
            let mut gap = false;
            for (i, p) in bucket.iter().enumerate() {
                if i.is_multiple_of(4096) {
                    check()?;
                }
                if !valid(p) {
                    gap = true;
                    break;
                }
            }
            if gap {
                result.push([f64::NAN, f64::NAN]);
                continue;
            }
            let (mut min, mut max, mut xmin, mut xmax) = (0, 0, 0, 0);
            for i in 1..bucket.len() {
                if i.is_multiple_of(4096) {
                    check()?;
                }
                if bucket[i][1] < bucket[min][1] {
                    min = i;
                }
                if bucket[i][1] > bucket[max][1] {
                    max = i;
                }
                if bucket[i][0] < bucket[xmin][0] {
                    xmin = i;
                }
                if bucket[i][0] > bucket[xmax][0] {
                    xmax = i;
                }
            }
            let mut indexes = [0, min, max, xmin, xmax, bucket.len() - 1];
            indexes.sort_unstable();
            let mut previous = None;
            for i in indexes {
                if previous != Some(i) {
                    result.push(bucket[i]);
                    previous = Some(i);
                }
            }
        }
        check()?;
        Ok(result)
    }
}

/// Horizontal hold followed by a vertical transition at the next x coordinate.
/// Original samples occupy the even indexes; invalid samples break both edges.
pub fn stair_geometry(points: &[[f64; 2]]) -> Vec<[f64; 2]> {
    stair_geometry_checked(points, &|| Ok(())).expect("Infallible stair checkpoint")
}
fn stair_geometry_checked(
    points: &[[f64; 2]],
    check: &impl Fn() -> Result<()>,
) -> Result<Vec<[f64; 2]>> {
    check()?;
    let mut result = Vec::with_capacity(points.len().saturating_mul(2).saturating_sub(1));
    for (index, &point) in points.iter().enumerate() {
        if index.is_multiple_of(4096) {
            check()?;
        }
        if index > 0 {
            let previous = points[index - 1];
            result.push(
                if previous.iter().chain(point.iter()).all(|v| v.is_finite()) {
                    [point[0], previous[1]]
                } else {
                    [f64::NAN; 2]
                },
            );
        }
        result.push(point);
    }
    check()?;
    Ok(result)
}

#[derive(Clone, Debug)]
pub struct Axes {
    pub palette_cycle: Option<PaletteCycle>,
    /// Fixed imported/public RGB values, independent of figure theme and color modes.
    pub colors: Option<AxesColors>,
    /// Process-local identity; cloned snapshots retain it, decoded objects do not.
    pub identity: crate::ObjectIdentity,
    pub series: Vec<Series>,
    pub title: String,
    pub xlabel: String,
    pub ylabel: String,
    pub hold: bool,
    pub legend: bool,
    pub legend_state: Option<LegendState>,
    /// Empty preserves historical Bar/Stem behavior; explicit objects are axes-owned.
    pub baselines: Vec<Baseline>,
    pub legend_location: LegendLocation,
    /// None retains historical geometry. Pixel-derived layout is never persisted.
    pub placement: Option<AxesPlacement>,
    pub xgrid: bool,
    pub ygrid: bool,
    pub xlim: Option<[f64; 2]>,
    pub ylim: Option<[f64; 2]>,
    pub xscale: AxisScale,
    pub yscale: AxisScale,
}
impl Default for Axes {
    fn default() -> Self {
        Self {
            colors: None,
            palette_cycle: None,
            identity: crate::ObjectIdentity::default(),
            series: Vec::new(),
            title: String::new(),
            xlabel: String::new(),
            ylabel: String::new(),
            hold: false,
            legend: false,
            legend_state: None,
            baselines: Vec::new(),
            legend_location: LegendLocation::default(),
            placement: None,
            // New Cartesian axes start without grids; native legacy defaults are separate.
            xgrid: false,
            ygrid: false,
            xlim: None,
            ylim: None,
            xscale: AxisScale::Linear,
            yscale: AxisScale::Linear,
        }
    }
}
impl Axes {
    pub fn geometry_ledger(&self) -> Result<coordinates::GeometryLedger> {
        geometry_ledger(self.series.iter())
    }

    /// Compares an editing snapshot without scanning or cloning shared samples.
    /// Equal coordinates in a newly allocated series are a different snapshot.
    pub fn same_snapshot(&self, other: &Self) -> bool {
        self.same_metadata(other)
            && self
                .series
                .iter()
                .zip(&other.series)
                .all(|(a, b)| a.points.same_storage(&b.points))
    }
    /// Bounded comparison of axes/series styling, labels and layout requests.
    /// Sample identities and contents are deliberately excluded.
    pub fn same_metadata(&self, other: &Self) -> bool {
        self.palette_cycle == other.palette_cycle
            && self.colors == other.colors
            && self.title == other.title
            && self.xlabel == other.xlabel
            && self.ylabel == other.ylabel
            && self.hold == other.hold
            && self.legend == other.legend
            && self.same_legend_metadata(other)
            && self.same_baseline_metadata(other)
            && self.same_latent_baseline_metadata(other)
            && self.legend_location == other.legend_location
            && self.placement == other.placement
            && self.xgrid == other.xgrid
            && self.ygrid == other.ygrid
            && self.xlim == other.xlim
            && self.ylim == other.ylim
            && self.xscale == other.xscale
            && self.yscale == other.yscale
            && self.series.len() == other.series.len()
            && self.series.iter().zip(&other.series).all(|(a, b)| {
                a.label == b.label
                    && a.kind == b.kind
                    && a.patch == b.patch
                    && a.bar_baseline_visible == b.bar_baseline_visible
                    && a.base_value == b.base_value
                    && a.face_edge_color == b.face_edge_color
                    && a.color == b.color
                    && a.explicit_color == b.explicit_color
                    && a.auto_palette == b.auto_palette
                    && a.pattern == b.pattern
                    && a.line_width == b.line_width
                    && a.marker_shape == b.marker_shape
                    && a.marker_size == b.marker_size
                    && a.markers == b.markers
                    && a.marker_filled == b.marker_filled
                    && a.visible == b.visible
                    && a.bar_width == b.bar_width
                    && a.bar_offset == b.bar_offset
            })
    }
    pub fn point_count(&self) -> usize {
        self.series.iter().map(|s| s.points.len()).sum()
    }
    pub fn validate(&self) -> Result<()> {
        if let Some(cycle) = self.palette_cycle {
            cycle.validate()?;
        }
        if let Some(colors) = self.colors {
            colors.validate()?;
        }
        if let Some(placement) = self.placement {
            placement.validate()?;
        }
        self.geometry_ledger()?;
        self.validate_legend_state()?;
        self.validate_baselines()?;
        for (scale, limits) in [(self.xscale, self.xlim), (self.yscale, self.ylim)] {
            if let Some(limits) = limits {
                scale.validate_limits(limits)?;
            }
        }
        for s in &self.series {
            s.validate_auto_palette()?;
            s.validate_patch()?;
            s.validate_face_edge()?;
            s.validate_base_value()?;
            if let Some(size) = s.marker_size {
                validate_marker_size(size)?;
            }
            if let Some(width) = s.line_width {
                validate_line_width(width)?;
            }
            if !s.bar_width.is_finite() || s.bar_width <= 0. || !s.bar_offset.is_finite() {
                return Err("Invalid series geometry".into());
            }
            if s.kind == PlotKind::Bar
                && (s.points.len() > 10000
                    || !s.bar_width.is_finite()
                    || s.bar_width <= 0.
                    || !s.bar_offset.is_finite())
            {
                return Err("Invalid bar geometry or more than 10000 bars per series".into());
            }
            if s.label.len() > 4096 {
                return Err("Series label exceeds 4096 bytes".into());
            }
        }
        if self.yscale == AxisScale::Log && self.series.iter().any(|s| s.base_value.is_some()) {
            let axis = self.axis_transforms()[1];
            if self
                .series
                .iter()
                .any(|s| s.base_value.is_some_and(|v| !axis.accepts(v)))
            {
                return Err(
                    "A logarithmic stem BaseValue must be nonzero and match the axes sign".into(),
                );
            }
        }
        if [&self.title, &self.xlabel, &self.ylabel]
            .iter()
            .any(|s| s.len() > 4096)
        {
            return Err("Plot label exceeds 4096 bytes".into());
        }
        Ok(())
    }
    pub fn add(&mut self, mut series: Vec<Series>) -> Result<()> {
        for s in &series {
            s.validate_auto_palette()?;
            s.validate_patch()?;
            s.validate_face_edge()?;
            s.validate_base_value()?;
        }
        geometry_ledger(self.series.iter().chain(&series))?;
        if self
            .series
            .iter()
            .chain(&series)
            .any(|s| s.base_value.is_some())
        {
            let mut candidate = self.clone();
            candidate.series.extend(series.iter().cloned());
            candidate.validate()?;
        }
        let mut cycle = self.palette_cycle;
        if let Some(cycle) = cycle {
            cycle.validate()?;
        }
        for (i, s) in series.iter_mut().enumerate() {
            let index = self.series.len() + i;
            let retained_palette = s.auto_palette.is_some();
            if let Some(cycle) = &mut cycle {
                cycle.admit(s);
            }
            if !s.explicit_color && !retained_palette {
                // Keep historical fallback bytes; effective modern RGB is resolved
                // from explicit provenance, not baked into the document on paint.
                s.color = COLORS[index % COLORS.len()];
            }
            if s.label.is_empty() {
                s.label = format!("data{}", index + 1);
            }
        }
        let baseline_join = self.prepare_bar_baseline_join(&mut series)?;
        if !self.baselines.is_empty() {
            let mut candidate = self.clone();
            candidate.series.extend(series.iter().cloned());
            if let Some((index, members)) = &baseline_join {
                candidate.baselines[*index]
                    .members
                    .extend(members.iter().cloned());
            }
            candidate.validate()?;
        }
        if let Some((index, members)) = baseline_join {
            self.baselines[index].members.extend(members);
        }
        if let Some(state) = &mut self.legend_state
            && state.auto_update
        {
            state
                .members
                .extend(series.iter().map(|s| s.identity.clone()));
        }
        self.palette_cycle = cycle;
        self.series.extend(series);
        Ok(())
    }
    pub fn bounds(&self) -> ([f64; 2], [f64; 2]) {
        self.bounds_checked(&|| Ok(()))
            .expect("Infallible bounds checkpoint")
    }
    pub fn bounds_cancellable(
        &self,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<([f64; 2], [f64; 2])> {
        self.bounds_checked(&|| display_check(cancel))
    }
    fn bounds_checked(&self, check: &impl Fn() -> Result<()>) -> Result<([f64; 2], [f64; 2])> {
        check()?;
        let axes = self.axis_transforms_checked(check)?;
        self.bounds_with_axes_checked(axes, check)
    }
    /// One shared sign-inference pass followed by bounds; preparation uses both.
    pub fn display_axes_cancellable(
        &self,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<DisplayAxesBounds> {
        let check = || display_check(cancel);
        let axes = self.axis_transforms_checked(&check)?;
        let bounds = self.bounds_with_axes_checked(axes, &check)?;
        Ok((axes, bounds))
    }
    fn bounds_with_axes_checked(
        &self,
        axes: [AxisTransform; 2],
        check: &impl Fn() -> Result<()>,
    ) -> Result<([f64; 2], [f64; 2])> {
        check()?;
        if self.baselines.len() > MAX_BASELINES {
            return Err("Baseline record bound exceeded".into());
        }
        let (mut x, mut y) = (
            [f64::INFINITY, f64::NEG_INFINITY],
            [f64::INFINITY, f64::NEG_INFINITY],
        );
        for (series_index, s) in self.series.iter().enumerate().filter(|(_, s)| s.visible) {
            let baseline = matches!(s.kind, PlotKind::Bar | PlotKind::Stem)
                .then(|| self.render_baseline(series_index, axes[1]))
                .transpose()?;
            if let Some(width) = s.complete_patch_width() {
                s.visit_patch_points_checked(width, axes, check, |[a, b], accepted| {
                    if accepted {
                        x[0] = x[0].min(a);
                        x[1] = x[1].max(a);
                        y[0] = y[0].min(b);
                        y[1] = y[1].max(b);
                    }
                })?;
                continue;
            }
            for (i, [a, b]) in s.points.iter().enumerate() {
                if i.is_multiple_of(4096) {
                    check()?;
                }
                if s.vertex_face_accepts(i, axes) {
                    let (lo, hi) = if s.kind == PlotKind::Bar {
                        (
                            a + s.bar_offset - s.bar_width / 2.,
                            a + s.bar_offset + s.bar_width / 2.,
                        )
                    } else {
                        (a, a)
                    };
                    x[0] = x[0].min(if axes[0].accepts(lo) { lo } else { a });
                    x[1] = x[1].max(if axes[0].accepts(hi) { hi } else { a });
                    y[0] = y[0].min(b);
                    y[1] = y[1].max(b);
                    if let Some(baseline) = baseline
                        && axes[1].accepts(baseline)
                    {
                        y[0] = y[0].min(baseline);
                        y[1] = y[1].max(baseline);
                    }
                }
            }
        }
        // Independent visible baseline records affect y bounds, never x extents.
        for baseline in &self.baselines {
            check()?;
            if baseline.style.visible && baseline.style.pattern != LinePattern::None {
                let value = baseline.value.resolve(axes[1])?;
                y[0] = y[0].min(value);
                y[1] = y[1].max(value);
            }
        }
        fn expand(mut range: [f64; 2], axis: AxisTransform) -> [f64; 2] {
            if axis.scale == AxisScale::Log {
                if !range.iter().all(|x| x.is_finite()) {
                    return if axis.negative {
                        [-10., -1.]
                    } else {
                        [1., 10.]
                    };
                }
                if axis.forward(range[0]) == axis.forward(range[1]) {
                    let center = axis.forward(range[0]);
                    return [axis.inverse(center - 0.5), axis.inverse(center + 0.5)];
                }
                return range;
            }
            if !range.iter().all(|v| v.is_finite()) {
                return [0., 1.];
            }
            if range[0] == range[1] {
                let d = (range[0].abs() * 0.05).max(0.5);
                range = [(range[0] - d).max(-f64::MAX), (range[1] + d).min(f64::MAX)];
            }
            range
        }
        check()?;
        Ok((
            self.xlim.unwrap_or_else(|| expand(x, axes[0])),
            self.ylim.unwrap_or_else(|| expand(y, axes[1])),
        ))
    }
    pub fn axis_transforms(&self) -> [AxisTransform; 2] {
        self.axis_transforms_checked(&|| Ok(()))
            .expect("Infallible axis checkpoint")
    }
    pub fn axis_transforms_cancellable(
        &self,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<[AxisTransform; 2]> {
        self.axis_transforms_checked(&|| display_check(cancel))
    }
    fn axis_transforms_checked(
        &self,
        check: &impl Fn() -> Result<()>,
    ) -> Result<[AxisTransform; 2]> {
        check()?;
        let mut axes = [
            AxisTransform {
                scale: self.xscale,
                negative: false,
            },
            AxisTransform {
                scale: self.yscale,
                negative: false,
            },
        ];
        for (axis, limits) in [self.xlim, self.ylim].into_iter().enumerate() {
            if axes[axis].scale != AxisScale::Log {
                continue;
            }
            if let Some(range) = limits {
                axes[axis].negative = range[1] < 0.;
                continue;
            }
            let mut negative = false;
            'series: for series in self.series.iter().filter(|s| s.visible) {
                check()?;
                for (index, point) in series.points.iter().enumerate() {
                    if index.is_multiple_of(4096) {
                        check()?;
                    }
                    if series.vertex_face_accepts(index, [AxisTransform::default(); 2]) {
                        if point[axis] > 0. {
                            negative = false;
                            break 'series;
                        }
                        negative |= point[axis] < 0.;
                    }
                }
            }
            axes[axis].negative = negative;
        }
        check()?;
        Ok(axes)
    }
}
pub fn validate_line_width(width: f64) -> Result<()> {
    if !width.is_finite() || !(0.1..=50.).contains(&width) {
        return Err("Line width must be 0.1..50 points".into());
    }
    Ok(())
}
pub fn validate_limits(v: [f64; 2]) -> Result<()> {
    if !v.iter().all(|v| v.is_finite()) || v[0] >= v[1] {
        Err("Axis limits must be finite and increasing".into())
    } else {
        Ok(())
    }
}

pub fn series_from_matrices(x: Option<&Matrix>, y: &Matrix, kind: PlotKind) -> Result<Vec<Series>> {
    if kind == PlotKind::Patch {
        return Err("Patch needs explicit face geometry".into());
    }
    if y.len() > MAX_POINTS {
        return Err("Plot exceeds 1000000 points".into());
    }
    let vector = |m: &Matrix| m.rows() == 1 || m.cols() == 1;
    if matches!(kind, PlotKind::Stairs | PlotKind::Stem)
        && let Some(x) = x
        && ((!vector(y) && vector(x) && x.len() != y.rows())
            || (!vector(x) && (vector(y) || x.rows() != y.rows() || x.cols() != y.cols())))
    {
        return Err(
            "stairs/stem need matching vectors, matching matrices, or an X vector matching Y rows"
                .into(),
        );
    }
    let mut transpose = false;
    let mut transpose_x = false;
    let (count, len) = if vector(y) {
        (1, y.len())
    } else {
        (y.cols(), y.rows())
    };
    let (count, len) = if let Some(x) = x {
        if vector(x) {
            if x.len() == len {
                (count, len)
            } else if !vector(y) && x.len() == y.cols() {
                transpose = true;
                (y.rows(), y.cols())
            } else {
                return Err("Plot vector length does not match the matrix rows or columns".into());
            }
        } else if vector(y) && y.len() == x.rows() {
            (x.cols(), x.rows())
        } else if vector(y) && y.len() == x.cols() {
            transpose_x = true;
            (x.rows(), x.cols())
        } else if x.rows() == y.rows() && x.cols() == y.cols() {
            (count, len)
        } else {
            return Err("Plot matrices must have equal sizes".into());
        }
    } else {
        (count, len)
    };
    if count.saturating_mul(len) > MAX_POINTS
        || (kind == PlotKind::Bar && count.saturating_mul(len) > 10000)
    {
        return Err("Plot exceeds point or bar limit".into());
    }
    if count > MAX_SERIES || (kind == PlotKind::Bar && len > 10000) {
        return Err("Plot exceeds 64 series or 10000 bars per series".into());
    }
    let build = |c| -> Result<Series> {
        let points: Vec<_> = (0..len)
            .map(|r| {
                let a = x.map_or(r as f64 + 1., |m| {
                    if vector(m) {
                        m.data()[r]
                    } else if transpose_x {
                        m.get(c, r)
                    } else {
                        m.get(r, c)
                    }
                });
                let b = if vector(y) {
                    y.data()[r]
                } else if transpose {
                    y.get(c, r)
                } else {
                    y.get(r, c)
                };
                [a, b]
            })
            .collect();
        let mut s = Series::new(points, kind);
        if kind == PlotKind::Bar {
            if s.points.iter().any(|p| !p[0].is_finite())
                || s.points
                    .iter()
                    .zip(s.points.iter().skip(1))
                    .any(|(a, b)| a[0] >= b[0])
            {
                return Err("Bar x coordinates must be finite and strictly increasing".into());
            }
            let spacing = s
                .points
                .iter()
                .zip(s.points.iter().skip(1))
                .map(|(a, b)| b[0] - a[0])
                .reduce(f64::min)
                .unwrap_or(1.);
            s.bar_width = spacing * 0.8 / count as f64;
            s.bar_offset = (c as f64 - (count as f64 - 1.) / 2.) * s.bar_width;
            if !s.bar_width.is_finite() || s.bar_width <= 0. {
                return Err("Bar spacing cannot be represented".into());
            }
        }
        Ok(s)
    };
    if count > 1 && count * len >= 100_000 {
        (0..count).into_par_iter().map(build).collect()
    } else {
        (0..count).map(build).collect()
    }
}

/// A figure owns an active axes and optional non-overlapping subplot axes.
/// Dereferencing addresses the active axes for existing numerical clients.
#[derive(Clone, Debug)]
pub struct Figure {
    /// Process-local identity; cloned snapshots retain it, decoded objects do not.
    pub identity: crate::ObjectIdentity,
    pub axes: Axes,
    pub layout: Option<Layout>,
    pub textboxes: Vec<TextBox>,
    pub name: String,
    /// None preserves legacy fixed Light/manual appearance.
    pub appearance: Option<FigureAppearance>,
    /// Optional legacy MATLAB print preference; MatRust exports retain resolved RGB.
    pub invert_hardcopy: Option<bool>,
    /// Manual canvas RGB. Use set_canvas_color to record explicit assignments.
    pub color: [u8; 3],
}
impl Default for Figure {
    fn default() -> Self {
        Self {
            identity: Default::default(),
            axes: Axes {
                palette_cycle: Some(PaletteCycle::new(PaletteVersion::MatRustModern1)),
                ..Axes::default()
            },
            layout: None,
            textboxes: Vec::new(),
            name: String::new(),
            appearance: Some(FigureAppearance::default()),
            invert_hardcopy: None,
            color: [255; 3],
        }
    }
}
impl std::ops::Deref for Figure {
    type Target = Axes;
    fn deref(&self) -> &Axes {
        &self.axes
    }
}
impl std::ops::DerefMut for Figure {
    fn deref_mut(&mut self) -> &mut Axes {
        &mut self.axes
    }
}
impl Figure {
    /// New empty axes inherit saved document policy, never the current host theme.
    pub fn fresh_axes(&self) -> Axes {
        Axes {
            palette_cycle: self
                .appearance
                .and_then(|a| a.series_palette)
                .map(PaletteCycle::new),
            ..Axes::default()
        }
    }
    pub fn has_static_style_metadata(&self) -> bool {
        self.invert_hardcopy.is_some()
            || self
                .all_axes()
                .any(|(_, a)| a.colors.is_some_and(|c| c.legend.is_some()))
    }
    pub fn has_palette_metadata(&self) -> bool {
        self.appearance.is_some_and(|a| a.series_palette.is_some())
            || self.all_axes().any(|(_, a)| {
                a.palette_cycle.is_some() || a.series.iter().any(|s| s.auto_palette.is_some())
            })
    }
    pub fn geometry_ledger(&self) -> Result<coordinates::GeometryLedger> {
        geometry_ledger(self.all_axes().flat_map(|(_, axes)| axes.series.iter()))
    }

    pub fn new(x: &Matrix, y: &Matrix) -> Result<Self> {
        let mut result = Self::default();
        result.add(series_from_matrices(Some(x), y, PlotKind::Line)?)?;
        Ok(result)
    }
    pub fn point_count(&self) -> usize {
        self.all_axes().map(|(_, a)| a.point_count()).sum()
    }
    pub fn series_count(&self) -> usize {
        self.all_axes().map(|(_, a)| a.series.len()).sum()
    }
    pub fn same_snapshot(&self, other: &Self) -> bool {
        self.same_metadata(other)
            && self
                .all_axes()
                .zip(other.all_axes())
                .all(|((_, a), (_, b))| a.same_snapshot(b))
    }
    pub(crate) fn same_metadata(&self, other: &Self) -> bool {
        self.name == other.name
            && self.color == other.color
            && self.appearance == other.appearance
            && self.invert_hardcopy == other.invert_hardcopy
            && self.textboxes == other.textboxes
            && self.layout.as_ref().map(|l| (l.active, l.tiled, &l.title))
                == other.layout.as_ref().map(|l| (l.active, l.tiled, &l.title))
            && self.all_axes().count() == other.all_axes().count()
            && self
                .all_axes()
                .zip(other.all_axes())
                .all(|((p, a), (q, b))| p == q && a.same_metadata(b))
    }
    pub fn validate(&self) -> Result<()> {
        if self.name.len() > 4096 {
            return Err("Figure Name exceeds 4096 UTF-8 bytes".into());
        }
        textbox::validate_textboxes(&self.textboxes)?;
        if let Some(layout) = &self.layout {
            layout.validate()?;
        }
        if self.empty_layout() && !self.axes.same_metadata(&self.fresh_axes()) {
            return Err("Empty tiled layout contains hidden axes data".into());
        }
        self.geometry_ledger()?;
        let mut identities = Vec::with_capacity(1 + 32 + MAX_SERIES + textbox::MAX_TEXTBOXES);
        identities.push(&self.identity);
        for (_, axes) in self.all_axes() {
            if axes.baselines.len() > MAX_BASELINES {
                return Err("Axes exceeds 64 baseline objects".into());
            }
            for id in std::iter::once(&axes.identity)
                .chain(axes.series.iter().map(|s| &s.identity))
                .chain(axes.legend_state.iter().map(|s| &s.identity))
                .chain(axes.baselines.iter().map(|b| &b.identity))
            {
                if identities.iter().any(|previous| previous.same_object(id)) {
                    return Err("A graphics object identity appears twice in one figure".into());
                }
                identities.push(id);
            }
        }
        for textbox in &self.textboxes {
            if identities
                .iter()
                .any(|previous| previous.same_object(&textbox.identity))
            {
                return Err("A graphics object identity appears twice in one figure".into());
            }
            identities.push(&textbox.identity);
        }
        for (_, axes) in self.all_axes() {
            axes.validate()?;
        }
        if self
            .all_axes()
            .any(|(_, a)| a.placement.is_some() || a.legend_location.requires_extended_layout())
        {
            let geometry = self.resolved_axes_geometry()?;
            for (i, (_, a)) in geometry.iter().enumerate() {
                for (_, b) in &geometry[..i] {
                    if a.data.overlaps(b.data)
                        || a.legend.is_some_and(|r| {
                            r.overlaps(b.data) || b.legend.is_some_and(|s| r.overlaps(s))
                        })
                        || b.legend.is_some_and(|r| r.overlaps(a.data))
                    {
                        return Err("Resolved axes or outside legends overlap; edit the held layout boundaries".into());
                    }
                }
            }
            let canvas = LegendRect::new([0., 0., 1., 1.]);
            let original = self.normalized_axes_positions();
            let held: Vec<_> = self
                .all_axes()
                .zip(original)
                .map(|((_, a), (_, original))| {
                    let request = a.placement.map_or(original, |p| {
                        if !p.imported_solved && p.constraint == PositionConstraint::OuterPosition {
                            p.requested_outer
                        } else {
                            p.requested_inner
                        }
                    });
                    LegendRect::from_normalized(request, canvas)
                })
                .collect();
            for (i, r) in held.iter().enumerate() {
                if held[..i].iter().any(|other| r.overlaps(*other)) {
                    return Err("Held axes layout allocations overlap".into());
                }
            }
        }
        Ok(())
    }
    pub fn add(&mut self, series: Vec<Series>) -> Result<()> {
        geometry_ledger(
            self.all_axes()
                .flat_map(|(_, axes)| axes.series.iter())
                .chain(&series),
        )?;
        for s in &series {
            s.validate_auto_palette()?;
            s.validate_patch()?;
            s.validate_face_edge()?;
            s.validate_base_value()?;
        }
        self.axes.add(series)?;
        self.activate_tile();
        Ok(())
    }
}

fn geometry_ledger<'a>(
    series: impl IntoIterator<Item = &'a Series>,
) -> Result<coordinates::GeometryLedger> {
    let mut ledger = coordinates::GeometryLedger::default();
    for series in series {
        ledger.add(series.geometry_cost()?)?;
    }
    Ok(ledger)
}

fn display_check(cancel: &std::sync::atomic::AtomicBool) -> Result<()> {
    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
        Err("Figure display preparation cancelled".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod cancellable_display_tests {
    use super::*;
    use std::cell::Cell;
    fn checkpoint(counter: &Cell<usize>, stop: usize) -> Result<()> {
        let value = counter.get() + 1;
        counter.set(value);
        if value == stop {
            Err("deterministic display cancellation".into())
        } else {
            Ok(())
        }
    }
    #[test]
    fn long_geometry_and_bounds_stop_inside_scan_and_retry_preserves_aliases() {
        for kind in [
            PlotKind::Line,
            PlotKind::Bar,
            PlotKind::Scatter,
            PlotKind::Stem,
            PlotKind::Stairs,
        ] {
            let count = if kind == PlotKind::Bar { 10000 } else { 65537 };
            let series = Series::new(
                (0..count)
                    .map(|i| {
                        [
                            (i as f64 - count as f64) - 1.,
                            (i as f64 - count as f64) - 2.,
                        ]
                    })
                    .collect(),
                kind,
            );
            let shared = series.points.clone();
            let counter = Cell::new(0);
            assert!(
                series
                    .display_points_axes_checked(
                        128,
                        [AxisTransform::default(); 2],
                        &|| checkpoint(&counter, 3)
                    )
                    .is_err()
            );
            assert_eq!(counter.get(), 3);
            let output = series
                .display_points_axes_cancellable(
                    128,
                    [AxisTransform::default(); 2],
                    &std::sync::atomic::AtomicBool::new(false),
                )
                .unwrap();
            assert!(!output.is_empty());
            assert!(shared.same_storage(&series.points));
            let axes = Axes {
                series: vec![series],
                xscale: AxisScale::Log,
                yscale: AxisScale::Log,
                ..Default::default()
            };
            counter.set(0);
            assert!(axes.bounds_checked(&|| checkpoint(&counter, 4)).is_err());
            assert_eq!(counter.get(), 4);
            let transforms = axes
                .axis_transforms_cancellable(&std::sync::atomic::AtomicBool::new(false))
                .unwrap();
            assert!(transforms.iter().all(|axis| axis.negative));
            let (x, y) = axes
                .bounds_cancellable(&std::sync::atomic::AtomicBool::new(false))
                .unwrap();
            assert!(x[0] < x[1] && y[0] < y[1] && x[1] < 0. && y[1] < 0.);
        }
    }
    #[test]
    fn preparation_preserves_full_bars_and_log_sign_with_gaps() {
        let points = vec![[-3., -4.], [-2., -5.], [-1., -2.]];
        let bars = Series::new(points.clone(), PlotKind::Bar);
        let result = bars
            .display_points_axes_cancellable(
                1,
                [AxisTransform::default(); 2],
                &std::sync::atomic::AtomicBool::new(false),
            )
            .unwrap();
        assert_eq!(result, points);
        let mut axes = Axes {
            series: vec![Series::new(
                vec![[-4., -5.], [f64::NAN, 2.], [2., 3.]],
                PlotKind::Line,
            )],
            xscale: AxisScale::Log,
            yscale: AxisScale::Log,
            ..Default::default()
        };
        assert!(axes.axis_transforms().iter().all(|axis| !axis.negative));
        axes.xlim = Some([-10., -1.]);
        axes.ylim = Some([-10., -1.]);
        let transforms = axes
            .axis_transforms_cancellable(&std::sync::atomic::AtomicBool::new(false))
            .unwrap();
        assert!(transforms.iter().all(|axis| axis.negative));
        let points = axes.series[0]
            .display_points_axes_cancellable(
                100,
                transforms,
                &std::sync::atomic::AtomicBool::new(false),
            )
            .unwrap();
        assert_eq!(points[0][0], -4_f64.log10());
        assert!(points[1].iter().all(|v| v.is_nan()));
        assert!(points[2].iter().all(|v| v.is_nan()));
    }
}

#[cfg(test)]
#[path = "plot/face_once_tests.rs"]
mod face_once_tests;

#[cfg(test)]
mod appearance_tests;
