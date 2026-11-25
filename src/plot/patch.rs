//! Bounded independent filled faces. Vertex data stays in Series' shared payload.
use super::{AxisTransform, Matrix, PlotKind, Result, Series};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum IndexedColor {
    Rgb([u8; 3]),
    Flat,
    Interp,
    None,
}

impl IndexedColor {
    pub fn mapped(self) -> bool {
        matches!(self, Self::Flat | Self::Interp)
    }
}

/// Shared input matrices for a live indexed Patch. The renderer uses the
/// expanded face stream, as it does for other 3-D patches.
#[derive(Clone, Debug)]
pub struct IndexedPatch {
    pub faces: Matrix,
    pub vertices: Matrix,
    pub colors: Option<Matrix>,
    pub normals: Option<Matrix>,
    pub face_color: IndexedColor,
    pub color_range: Option<[f64; 2]>,
    pub mapped_with: Option<MappedWith>,
}

/// Colormap and colour limits an indexed patch's colours were last mapped with.
pub type MappedWith = (std::sync::Arc<Vec<[f64; 3]>>, [f64; 2]);

impl IndexedPatch {
    /// Expand each connectivity row in corner order after the figure budget check.
    pub(crate) fn expand(
        &self,
        mut checkpoint: impl FnMut() -> Result<()>,
    ) -> Result<(Vec<[f64; 2]>, Vec<f64>)> {
        let (faces, vertices) = (&self.faces, &self.vertices);
        let has_z = vertices.cols() == 3;
        let mut points = Vec::with_capacity(faces.len());
        let mut z = Vec::with_capacity(if has_z { faces.len() } else { 0 });
        for row in 0..faces.rows() {
            if row.is_multiple_of(4096) {
                checkpoint()?;
            }
            for col in 0..faces.cols() {
                let index = faces.get(row, col);
                if !(1. ..=vertices.rows() as f64).contains(&index) || index.fract() != 0. {
                    return Err(
                        "Patch Faces must contain valid one-based indices into Vertices".into(),
                    );
                }
                let vertex = index as usize - 1;
                let xy = [vertices.get(vertex, 0), vertices.get(vertex, 1)];
                if !xy.iter().all(|p| p.is_finite()) {
                    return Err("Indexed patch vertices must be finite".into());
                }
                points.push(xy);
                if has_z {
                    let value = vertices.get(vertex, 2);
                    if !value.is_finite() {
                        return Err("Indexed patch vertices must be finite".into());
                    }
                    z.push(value);
                }
            }
        }
        Ok((points, z))
    }
    pub(crate) fn geometry_cost(
        &self,
        expanded: super::coordinates::GeometryCost,
        z_len: usize,
    ) -> Result<super::coordinates::GeometryCost> {
        let color_count = if self.face_color.mapped() && self.colors.is_some() {
            self.faces.rows()
        } else {
            0
        };
        let bytes = self.faces.retained_bytes()
            + self.vertices.retained_bytes()
            + self.colors.as_ref().map_or(0, Matrix::retained_bytes)
            + self.normals.as_ref().map_or(0, Matrix::retained_bytes)
            + z_len * 8
            + color_count * 3;
        expanded.with_additional_storage(bytes)
    }
}

/// Patch faces one figure holds in total (surf/mesh quads count one each).
pub const MAX_PATCH_FACES: usize = 100_000;
/// Faces of low-level `patch` objects (and their figure budget): the bound
/// they had before surfaces raised the figure cap.
pub const MAX_PATCH_OBJECT_FACES: usize = 10_000;

/// Draw style shared by the faces one surface contributes to a merged,
/// depth-sorted draw list (display copy only, never saved).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceStyle {
    /// Fill opacity (FaceAlpha); None is opaque, 0 draws no fill.
    pub alpha: Option<f64>,
    /// Whether the faces draw an outline (EdgeColor other than 'none').
    pub outline: bool,
    pub edge_width: f32,
    pub pattern: super::LinePattern,
}
impl FaceStyle {
    pub fn of(series: &Series) -> Self {
        // An indexed mesh hides its fill with `face_color_none` and keeps the
        // requested FaceAlpha. The draw style uses alpha 0 so a merged neighbor
        // list does not paint that mesh. Positional patches are unchanged.
        let alpha = if series.face_color_none && series.indexed_patch.is_some() {
            Some(0.)
        } else {
            series.face_alpha
        };
        Self {
            alpha,
            outline: series.face_edge_color.is_some() || series.face_edge_colors.is_some(),
            edge_width: series.face_edge_width(),
            pattern: series.pattern,
        }
    }
}
/// Per-face styles of a merged surface draw list: face k is drawn with
/// `styles[owner[k]]` (one style per contributing surface, at most 64).
#[derive(Clone, Debug, PartialEq)]
pub struct FaceStyles {
    styles: Vec<FaceStyle>,
    owner: Vec<u8>,
}
impl FaceStyles {
    pub fn new(styles: Vec<FaceStyle>, owner: Vec<u8>) -> Self {
        Self { styles, owner }
    }
    pub fn get(&self, face: usize) -> Option<FaceStyle> {
        let k = *self.owner.get(face)?;
        self.styles.get(usize::from(k)).copied()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchFaces {
    vertices_per_face: u8,
}
impl PatchFaces {
    pub fn new(vertices_per_face: usize) -> Result<Self> {
        if !matches!(vertices_per_face, 3 | 4) {
            return Err("Patch needs three or four vertices per face".into());
        }
        Ok(Self {
            vertices_per_face: vertices_per_face as u8,
        })
    }
    pub fn vertices_per_face(self) -> usize {
        usize::from(self.vertices_per_face)
    }
    pub fn face_count(self, vertices: usize) -> Result<usize> {
        let width = self.vertices_per_face();
        if !matches!(width, 3 | 4) || !vertices.is_multiple_of(width) {
            return Err("Patch needs three or four vertices per face".into());
        }
        let count = vertices / width;
        if count > MAX_PATCH_FACES {
            return Err(format!(
                "Patch exceeds {MAX_PATCH_FACES} faces (a figure holds at most {MAX_PATCH_FACES} patch faces)"
            ));
        }
        Ok(count)
    }
    pub fn validate(self, vertices: &[[f64; 2]]) -> Result<()> {
        self.face_count(vertices.len())?;
        for face in vertices.chunks_exact(self.vertices_per_face()) {
            validate_face(face)?;
        }
        Ok(())
    }
}

pub(super) fn validate_face(face: &[[f64; 2]]) -> Result<()> {
    // Missing faces retain their exact source coordinates. They are not filled.
    if face.len() == 3 || face.iter().flatten().any(|v| !v.is_finite()) {
        return Ok(());
    }
    let [a, b, c, d] = face else {
        return Err("Unsupported patch face".into());
    };
    let vertical_first = a[0] == b[0] && b[1] == c[1] && c[0] == d[0] && d[1] == a[1];
    let horizontal_first = a[1] == b[1] && b[0] == c[0] && c[1] == d[1] && d[0] == a[0];
    if !vertical_first && !horizontal_first {
        return Err("Patch currently supports triangles and axis-aligned quadrilaterals".into());
    }
    Ok(())
}

impl Series {
    pub fn patch_from_matrices(
        x: &Matrix,
        y: &Matrix,
        color: [u8; 3],
        cancel: &AtomicBool,
    ) -> Result<Self> {
        Self::patch_from_matrices_with_ledger(
            x,
            y,
            color,
            cancel,
            super::coordinates::GeometryLedger::default(),
            0,
        )
    }
    /// `ledger` is the figure's (every series, figure-wide caps);
    /// `object_faces` the general faces the figure's low-level `patch`
    /// objects already hold (surfaces and contourf bands excluded), checked
    /// against the [`MAX_PATCH_OBJECT_FACES`] patch-object bound.
    pub(crate) fn patch_from_matrices_with_ledger(
        x: &Matrix,
        y: &Matrix,
        color: [u8; 3],
        cancel: &AtomicBool,
        mut ledger: super::coordinates::GeometryLedger,
        object_faces: usize,
    ) -> Result<Self> {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        if x.rows() != y.rows() || x.cols() != y.cols() || !matches!(x.rows(), 3 | 4) {
            return Err(
                "Patch XData and YData must have equal three-by-N or four-by-N shape".into(),
            );
        }
        if x.cols() > MAX_PATCH_OBJECT_FACES {
            return Err(format!("Patch exceeds {MAX_PATCH_OBJECT_FACES} faces"));
        }
        // Facts are optional: an ineligible descriptor or derived finite-bit
        // mismatch takes the original dense path. Stop is never swallowed.
        let descriptor = x.rectangle_coordinates(y, cancel);
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        if let Ok(Some(descriptor)) = descriptor
            && descriptor.matches_matrices(x, y, cancel)?
            && ledger.add(descriptor.cost()).is_ok()
        {
            let faces = PatchFaces {
                vertices_per_face: 4,
            };
            faces.face_count(x.len())?;
            let mut series = Series::with_points(
                super::SeriesPoints::Patch(std::sync::Arc::new(descriptor)),
                PlotKind::Patch,
            );
            series.patch = Some(faces);
            series.pattern = super::LinePattern::None;
            series.color = color;
            series.explicit_color = true;
            if cancel.load(Ordering::Relaxed) {
                return Err("Cancelled".into());
            }
            return Ok(series);
        }
        if object_faces.saturating_add(x.cols()) > MAX_PATCH_OBJECT_FACES {
            return Err(format!(
                "Patch face budget exceeds {MAX_PATCH_OBJECT_FACES} faces for patch objects in one figure"
            ));
        }
        ledger.add(super::coordinates::GeometryCost::dense_face_allocation(
            x.len(),
            x.rows(),
        )?)?;
        build(x, y, color, || {
            if cancel.load(Ordering::Relaxed) {
                Err("Cancelled".into())
            } else {
                Ok(())
            }
        })
    }

    pub fn patch_face_count(&self) -> usize {
        self.patch
            .and_then(|p| p.face_count(self.points.len()).ok())
            .unwrap_or(0)
    }

    pub(crate) fn validate_patch(&self) -> Result<()> {
        if self.patch_cdata.is_some_and(|v| !v.is_finite()) {
            return Err("Patch CData must be finite".into());
        }
        if self.patch_cdata.is_some() && self.kind != PlotKind::Patch {
            return Err("Scalar patch CData belongs only to Patch series".into());
        }
        if let Some(vertex) = &self.patch_vertex_cdata
            && (self.kind != PlotKind::Patch
                || self.patch_cdata.is_some()
                || self.patch.map(|p| p.vertices_per_face()) != Some(3)
                || vertex.corners.len() != self.points.len()
                || vertex.source.iter().any(|v| !v.is_finite())
                || vertex
                    .corners
                    .iter()
                    .any(|&i| i as usize >= vertex.source.len()))
        {
            return Err("Invalid per-vertex Patch CData".into());
        }
        if let Some(level) = self.slice_contour_level {
            if self.kind != PlotKind::Patch
                || self.patch.is_some()
                || self.points.patch().is_some()
                || self.points.len() < 2
                || !self.is_3d()
                || !level.is_finite()
                || self.face_colors.is_some()
                || self.face_edge_colors.is_some()
                || self.face_alpha.is_some()
                || self.face_edge_color.is_some()
                || self.markers
                || self.marker_shape.is_some()
                || self.marker_size.is_some()
                || self.marker_filled
            {
                return Err("Invalid contourslice patch".into());
            }
            return Ok(());
        }
        match (self.kind, self.patch) {
            (PlotKind::Patch, Some(faces)) => {
                if self.markers
                    || self.marker_shape.is_some()
                    || self.marker_size.is_some()
                    || self.marker_filled
                {
                    return Err("Patch markers are not supported".into());
                }
                if let Some(stored) = self.points.patch()
                    && stored.corners_per_face() != faces.vertices_per_face()
                {
                    return Err("Stored Patch descriptor and face width disagree".into());
                }
                let width = faces.vertices_per_face();
                let count = faces.face_count(self.points.len())?;
                self.validate_face_style(count)?;
                // Surface quads (with z) are general quadrilaterals.
                if self.is_3d() {
                    return Ok(());
                }
                for face in 0..count {
                    let mut corners = [[0.0; 2]; 4];
                    for (corner, value) in corners[..width].iter_mut().enumerate() {
                        *value = self
                            .points
                            .get(face * width + corner)
                            .ok_or("Incomplete Patch face")?;
                    }
                    validate_face(&corners[..width])?;
                }
                Ok(())
            }
            (PlotKind::Patch, None) | (_, Some(_)) => {
                Err("Patch geometry and series kind disagree".into())
            }
            (PlotKind::Bar, None) => {
                if self.face_edge_colors.is_some() {
                    return Err("Per-face edge colours belong only to Patch series".into());
                }
                self.validate_face_style(self.points.len())?;
                if self.points.patch().is_some() {
                    return Err("Stored Patch coordinates require Patch kind".into());
                }
                Ok(())
            }
            (_, None)
                if self.face_colors.is_some()
                    || self.face_edge_colors.is_some()
                    || self.face_alpha.is_some()
                    || self.face_color_none
                    || self.edge_alpha.is_some() =>
            {
                Err("Per-face colours and FaceAlpha belong only to Patch or Bar series".into())
            }
            (_, None) if self.points.patch().is_some() => {
                Err("Stored Patch coordinates require Patch kind".into())
            }
            (_, None) => Ok(()),
        }
    }

    /// Per-face colour lists hold one colour per face; FaceAlpha is in 0..=1.
    pub(crate) fn validate_face_style(&self, faces: usize) -> Result<()> {
        for list in [&self.face_colors, &self.face_edge_colors]
            .into_iter()
            .flatten()
        {
            if list.len() != faces {
                return Err("Per-face colour list length differs from the face count".into());
            }
        }
        let bad_alpha =
            |a: Option<f64>| a.is_some_and(|a| !a.is_finite() || !(0. ..=1.).contains(&a));
        if bad_alpha(self.face_alpha) {
            return Err("FaceAlpha must be a scalar in 0..1".into());
        }
        if bad_alpha(self.edge_alpha) {
            return Err("EdgeAlpha must be a scalar in 0..1".into());
        }
        if let Some(table) = &self.face_styles
            && (table.owner.len() != faces
                || table.styles.len() > usize::from(u8::MAX) + 1
                || table
                    .owner
                    .iter()
                    .any(|&k| usize::from(k) >= table.styles.len())
                || table.styles.iter().any(|s| bad_alpha(s.alpha)))
        {
            return Err("Per-face style list differs from the face count".into());
        }
        Ok(())
    }
    /// Fill colour of face `index`: its own colour when the series has a list.
    pub fn face_fill_color(&self, index: usize) -> Option<[u8; 3]> {
        self.face_colors
            .as_ref()
            .and_then(|c| c.get(index).copied())
    }
    /// Edge colour of face `index`, None when the face has no outline.
    pub fn face_outline_color(&self, index: usize) -> Option<[u8; 3]> {
        if !self.face_style(index).outline {
            return None;
        }
        match &self.face_edge_colors {
            Some(list) => list.get(index).copied(),
            None => self.face_edge_color,
        }
    }
    /// Draw style of face `index`: its own entry of a merged surface draw
    /// list, else the series' style.
    pub fn face_style(&self, index: usize) -> FaceStyle {
        self.face_styles
            .as_ref()
            .and_then(|t| t.get(index))
            .unwrap_or_else(|| FaceStyle::of(self))
    }
    /// Whether any face of this series draws an outline.
    pub fn has_face_outline(&self) -> bool {
        self.face_edge_color.is_some() || self.face_edge_colors.is_some()
    }

    /// Whole-face eligibility: partial missing polygons never affect fill/bounds.
    pub(crate) fn vertex_face_accepts(&self, index: usize, axes: [AxisTransform; 2]) -> bool {
        let Some(patch) = self.patch else {
            let Some(p) = self.points.get(index) else {
                return false;
            };
            return axes[0].accepts(p[0]) && axes[1].accepts(p[1]);
        };
        let width = patch.vertices_per_face();
        if !matches!(width, 3 | 4) {
            return false;
        }
        let start = index / width * width;
        self.points
            .range(start..start + width)
            .is_some_and(|mut face| face.all(|p| axes[0].accepts(p[0]) && axes[1].accepts(p[1])))
    }
}

fn build(
    x: &Matrix,
    y: &Matrix,
    color: [u8; 3],
    mut checkpoint: impl FnMut() -> Result<()>,
) -> Result<Series> {
    checkpoint()?;
    if x.rows() != y.rows() || x.cols() != y.cols() || !matches!(x.rows(), 3 | 4) {
        return Err("Patch XData and YData must have equal three-by-N or four-by-N shape".into());
    }
    let faces = PatchFaces {
        vertices_per_face: x.rows() as u8,
    };
    faces.face_count(x.len())?;
    // Validate all geometry before the result allocation or publication.
    for c in 0..x.cols() {
        if c.is_multiple_of(256) {
            checkpoint()?;
        }
        let mut face = [[0.; 2]; 4];
        for (r, p) in face[..x.rows()].iter_mut().enumerate() {
            *p = [x.get(r, c), y.get(r, c)];
        }
        validate_face(&face[..x.rows()])?;
    }
    let mut points = Vec::new();
    points
        .try_reserve_exact(x.len())
        .map_err(|_| "Cannot allocate patch vertices")?;
    for (i, (&a, &b)) in x.data().iter().zip(y.data()).enumerate() {
        if i.is_multiple_of(1024) {
            checkpoint()?;
        }
        points.push([a, b]);
    }
    checkpoint()?;
    let mut result = Series::new(points, PlotKind::Patch);
    result.patch = Some(faces);
    result.pattern = super::LinePattern::None;
    result.color = color;
    result.explicit_color = true;
    Ok(result)
}

/// At most eight vertices result from clipping a convex quadrilateral to a box.
/// The ninth slot is a defensive bound for repeated boundary vertices.
#[derive(Clone, Debug)]
pub struct ClippedFace {
    points: [[f64; 2]; 9],
    len: usize,
}
impl ClippedFace {
    pub fn points(&self) -> &[[f64; 2]] {
        &self.points[..self.len]
    }
    fn push(&mut self, point: [f64; 2]) -> bool {
        if self.len == self.points.len() {
            return false;
        }
        if self.len == 0 || self.points[self.len - 1] != point {
            self.points[self.len] = point;
            self.len += 1;
        }
        true
    }
    fn empty() -> Self {
        Self {
            points: [[0.; 2]; 9],
            len: 0,
        }
    }
}

// A rounded interpolation fraction can lose a small clipping boundary between
// opposite huge endpoints: t rounds to 0.5 and y=x incorrectly becomes y=0.
// Use the line equation instead, with independent power-of-two normalization
// and Kahan's compensated difference of products for its determinant. Only
// crossing edges reach here; horizontal/vertical and exact endpoint cases stay
// cheap. This is bounded binary64 arithmetic, not an exact-predicate kernel.
fn face_boundary_intersection(a: [f64; 2], b: [f64; 2], axis: usize, edge: f64) -> [f64; 2] {
    let other = 1 - axis;
    let mut point = [0.; 2];
    point[axis] = edge;
    if edge == a[axis] || a[other] == b[other] {
        point[other] = a[other];
        return point;
    }
    if edge == b[axis] {
        point[other] = b[other];
        return point;
    }
    // Crossing guarantees nonzero axis extent; unequal other coordinates
    // guarantee a nonzero scale there, even for subnormal-only inputs.
    let ex = libm::ilogb(a[axis].abs().max(b[axis].abs()));
    let ey = libm::ilogb(a[other].abs().max(b[other].abs()));
    let ax = libm::scalbn(a[axis], -ex);
    let bx = libm::scalbn(b[axis], -ex);
    let ay = libm::scalbn(a[other], -ey);
    let by = libm::scalbn(b[other], -ey);
    let dx = bx - ax;
    let dy = by - ay;
    let product = ay * bx;
    let residual = libm::fma(-ay, bx, product);
    let determinant = libm::fma(ax, by, -product) + residual;
    // When normalization erases a tiny endpoint, anchor at that endpoint
    // instead. It retains its literal offset from the line through the origin.
    let tiny_a = ax.abs() < f64::MIN_POSITIVE && ay.abs() < f64::MIN_POSITIVE;
    let tiny_b = bx.abs() < f64::MIN_POSITIVE && by.abs() < f64::MIN_POSITIVE;
    let value = if tiny_a || tiny_b {
        let anchor = if tiny_a { a } else { b };
        let (m, e) = libm::frexp(edge - anchor[axis]);
        libm::scalbn((dy / dx) * m, ey - ex + e) + anchor[other]
    } else if determinant == 0. {
        // Do not normalize edge first: a tiny viewport may be over 1074
        // exponents below the source. Split edge so the zero-intercept slope
        // still preserves that boundary (including subnormal boundaries).
        let (m, e) = libm::frexp(edge);
        libm::scalbn((dy / dx) * m, ey - ex + e)
    } else {
        let x = libm::scalbn(edge, -ex);
        libm::scalbn(libm::fma(dy, x, -determinant) / dx, ey)
    };
    // A genuine segment intersection is inside its finite endpoint range.
    // The clamp also bounds an outward final-rounding overflow near MAX.
    point[other] = value.clamp(a[other].min(b[other]), a[other].max(b[other]));
    point
}

/// Clip in display-data coordinates BEFORE conversion to f32/pixel coordinates.
/// This preserves far-outside intersections instead of clamping every vertex.
/// Input is one admitted convex face; bounds are [xmin,xmax,ymin,ymax].
pub fn clip_patch_face(face: &[[f64; 2]], bounds: [f64; 4]) -> ClippedFace {
    let mut polygon = ClippedFace::empty();
    if !matches!(face.len(), 3 | 4)
        || face.iter().flatten().any(|v| !v.is_finite())
        || bounds.iter().any(|v| !v.is_finite())
        || bounds[0] >= bounds[1]
        || bounds[2] >= bounds[3]
    {
        return polygon;
    }
    for &point in face {
        polygon.push(point);
    }
    for (axis, edge, lower) in [
        (0, bounds[0], true),
        (0, bounds[1], false),
        (1, bounds[2], true),
        (1, bounds[3], false),
    ] {
        if polygon.len == 0 {
            break;
        }
        let mut next = ClippedFace::empty();
        let inside = |p: [f64; 2]| {
            if lower {
                p[axis] >= edge
            } else {
                p[axis] <= edge
            }
        };
        let mut a = polygon.points[polygon.len - 1];
        for &b in polygon.points() {
            if inside(a) != inside(b) {
                let intersection = face_boundary_intersection(a, b, axis, edge);
                if !intersection[1 - axis].is_finite() || !next.push(intersection) {
                    return ClippedFace::empty();
                }
            }
            if inside(b) && !next.push(b) {
                return ClippedFace::empty();
            }
            a = b;
        }
        if next.len > 1 && next.points[0] == next.points[next.len - 1] {
            next.len -= 1;
        }
        polygon = next;
    }
    polygon
}

/// Clip only an original face edge, never a synthetic polygon clipping boundary.
/// Fixed stack work; shares the scaled boundary intersection used by fills.
pub fn clip_face_edge(mut a: [f64; 2], mut b: [f64; 2], bounds: [f64; 4]) -> Option<[[f64; 2]; 2]> {
    if a.into_iter().chain(b).chain(bounds).any(|v| !v.is_finite())
        || bounds[0] >= bounds[1]
        || bounds[2] >= bounds[3]
    {
        return None;
    }
    for (axis, edge, lower) in [
        (0, bounds[0], true),
        (0, bounds[1], false),
        (1, bounds[2], true),
        (1, bounds[3], false),
    ] {
        let inside = |p: [f64; 2]| {
            if lower {
                p[axis] >= edge
            } else {
                p[axis] <= edge
            }
        };
        let (ia, ib) = (inside(a), inside(b));
        if !ia && !ib {
            return None;
        }
        if ia != ib {
            let p = face_boundary_intersection(a, b, axis, edge);
            if p.iter().any(|v| !v.is_finite()) {
                return None;
            }
            if ia {
                b = p;
            } else {
                a = p;
            }
        }
    }
    Some([a, b])
}

/// Map an already clipped display-data vertex into a finite screen rectangle.
pub fn patch_screen_point(point: [f64; 2], bounds: [f64; 4], frame: [f64; 4]) -> [f64; 2] {
    let fraction = |value: f64, low: f64, high: f64| {
        let span = high - low;
        let offset = value - low;
        if span.is_finite() && offset.is_finite() {
            (offset / span).clamp(0., 1.)
        } else {
            ((value * 0.5 - low * 0.5) / (high * 0.5 - low * 0.5)).clamp(0., 1.)
        }
    };
    let x = fraction(point[0], bounds[0], bounds[1]);
    let y = fraction(point[1], bounds[2], bounds[3]);
    [
        frame[0] * (1. - x) + frame[2] * x,
        frame[3] * (1. - y) + frame[1] * y,
    ]
}

/// Per-vertex scalar CData of a positional fill/patch drawn with FaceColor
/// 'interp'. `source` keeps the caller's values (including a closing vertex);
/// `corners` maps each rendered triangle-stream point to its source vertex.
#[derive(Clone, Debug, PartialEq)]
pub struct VertexCData {
    pub source: std::sync::Arc<[f64]>,
    pub corners: std::sync::Arc<[u32]>,
}

impl VertexCData {
    /// Values of the three corners of rendered triangle `face`.
    pub fn triangle(&self, face: usize) -> Option<[f64; 3]> {
        let mut out = [0.; 3];
        for (k, v) in out.iter_mut().enumerate() {
            let i = *self.corners.get(face * 3 + k)? as usize;
            *v = *self.source.get(i)?;
        }
        Some(out)
    }
}

/// Bands one interpolated triangle may draw when it is alone in a series.
pub const MAX_INTERP_BANDS: usize = 256;
/// Bands one interpolated series draws in total (each triangle gets a share).
pub const MAX_INTERP_SERIES_BANDS: usize = 65_536;

/// Per-triangle band limit for a series of `faces` interpolated triangles.
pub fn interp_band_limit(faces: usize) -> usize {
    (MAX_INTERP_SERIES_BANDS / faces.max(1)).clamp(1, MAX_INTERP_BANDS)
}

/// Fill one triangle whose scalar CData varies linearly between its corners.
/// The scalar is interpolated across the face, then mapped through the
/// colormap with the CDataMapping 'scaled' row rule (`colormap::scaled_rgb`).
/// Rows are therefore constant over bands between iso-value lines; each band
/// is a convex polygon clipped to `bounds` (display-data coordinates), emitted
/// with its row colour. At most `max_bands` bands are emitted; when a triangle
/// crosses more rows, adjacent rows merge and take their middle row's colour.
pub fn interp_triangle_bands(
    tri: [[f64; 2]; 3],
    values: [f64; 3],
    bounds: [f64; 4],
    map: &[[f64; 3]],
    clim: [f64; 2],
    max_bands: usize,
    mut emit: impl FnMut(&[[f64; 2]], [u8; 3]),
) {
    let clipped = clip_patch_face(&tri, bounds);
    if clipped.points().len() < 3 || values.iter().any(|v| !v.is_finite()) {
        return;
    }
    // Affine c(p) = c0 + g . (p - p0), solved in coordinates relative to p0.
    let [p0, p1, p2] = tri;
    let (ux, uy) = (p1[0] - p0[0], p1[1] - p0[1]);
    let (vx, vy) = (p2[0] - p0[0], p2[1] - p0[1]);
    let det = ux * vy - uy * vx;
    let (d1, d2) = (values[1] - values[0], values[2] - values[0]);
    let gradient = if det != 0. && det.is_finite() {
        [(d1 * vy - d2 * uy) / det, (d2 * ux - d1 * vx) / det]
    } else {
        [0., 0.]
    };
    let value =
        |p: [f64; 2]| values[0] + gradient[0] * (p[0] - p0[0]) + gradient[1] * (p[1] - p0[1]);
    let m = map.len().max(1);
    let span = clim[1] - clim[0];
    let row = |v: f64| {
        let t = (v - clim[0]) / span;
        if t.is_nan() {
            0
        } else {
            ((t * m as f64).floor().max(0.) as usize).min(m - 1)
        }
    };
    let rgb = |r: usize| {
        map.get(r)
            .copied()
            .unwrap_or([0.; 3])
            .map(|c| (c * 255.).round() as u8)
    };
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for &p in clipped.points() {
        let v = value(p);
        lo = lo.min(v);
        hi = hi.max(v);
    }
    let (r0, r1) = (row(lo), row(hi));
    if r0 == r1 || !lo.is_finite() || !hi.is_finite() {
        emit(
            clipped.points(),
            rgb(row(value(tri_centroid(clipped.points())))),
        );
        return;
    }
    let step = (r1 - r0 + 1).div_ceil(max_bands.max(1));
    let boundary = |r: usize| clim[0] + span * (r as f64 / m as f64);
    let mut band: Vec<[f64; 2]> = Vec::with_capacity(12);
    let mut scratch: Vec<[f64; 2]> = Vec::with_capacity(12);
    let mut r = r0;
    while r <= r1 {
        let next = r + step;
        band.clear();
        band.extend_from_slice(clipped.points());
        if r > r0 {
            clip_half_plane(&band, &mut scratch, |p| value(p) - boundary(r));
            std::mem::swap(&mut band, &mut scratch);
        }
        if next <= r1 {
            clip_half_plane(&band, &mut scratch, |p| boundary(next) - value(p));
            std::mem::swap(&mut band, &mut scratch);
        }
        if band.len() >= 3 {
            emit(&band, rgb((r + (next.min(r1 + 1) - 1)) / 2));
        }
        r = next;
    }
}

fn tri_centroid(points: &[[f64; 2]]) -> [f64; 2] {
    let n = points.len().max(1) as f64;
    let s = points
        .iter()
        .fold([0., 0.], |a, p| [a[0] + p[0], a[1] + p[1]]);
    [s[0] / n, s[1] / n]
}

/// Keep the part of a convex polygon where `side(p) >= 0` (Sutherland-Hodgman).
fn clip_half_plane(input: &[[f64; 2]], out: &mut Vec<[f64; 2]>, side: impl Fn([f64; 2]) -> f64) {
    out.clear();
    let n = input.len();
    for i in 0..n {
        let (a, b) = (input[i], input[(i + 1) % n]);
        let (sa, sb) = (side(a), side(b));
        if sa >= 0. {
            out.push(a);
        }
        if (sa >= 0.) != (sb >= 0.) {
            let t = sa / (sa - sb);
            out.push([a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interp_bands_tile_the_triangle_and_respect_the_band_limit() {
        let map: Vec<[f64; 3]> = (0..256).map(|k| [k as f64 / 255., 0., 0.]).collect();
        let tri = [[0., 0.], [2., 0.], [0., 1.]];
        let area = |p: &[[f64; 2]]| {
            (0..p.len())
                .map(|i| {
                    let (a, b) = (p[i], p[(i + 1) % p.len()]);
                    a[0] * b[1] - b[0] * a[1]
                })
                .sum::<f64>()
                .abs()
                / 2.
        };
        for (limit, expected) in [(256, 256), (8, 8), (1, 1)] {
            let (mut count, mut total) = (0, 0.);
            let mut reds = Vec::new();
            interp_triangle_bands(
                tri,
                [0., 1., 0.],
                [-1., 3., -1., 2.],
                &map,
                [0., 1.],
                limit,
                |band, rgb| {
                    count += 1;
                    total += area(band);
                    reds.push(rgb[0]);
                },
            );
            assert_eq!(count, expected);
            assert!((total - 1.).abs() < 1e-12, "{total}");
            assert!(reds.windows(2).all(|w| w[0] < w[1]) || expected == 1);
        }
        // Constant values draw one polygon of their row.
        let mut rows = Vec::new();
        interp_triangle_bands(
            tri,
            [0.5; 3],
            [-1., 3., -1., 2.],
            &map,
            [0., 1.],
            8,
            |_, c| rows.push(c),
        );
        assert_eq!(rows, vec![[128, 0, 0]]);
        assert_eq!(interp_band_limit(1), MAX_INTERP_BANDS);
        assert_eq!(interp_band_limit(3000), MAX_INTERP_SERIES_BANDS / 3000);
    }
    #[test]
    fn patch_geometry_cancellation_is_bounded_and_retry_preserves_payload() {
        let x = Matrix::from_column_vec(
            4,
            1024,
            (0..4096).map(|i| [0., 1., 1., 0.][i % 4]).collect(),
        )
        .unwrap();
        let y = Matrix::from_column_vec(
            4,
            1024,
            (0..4096).map(|i| [0., 0., 2., 2.][i % 4]).collect(),
        )
        .unwrap();
        for stop in [2, 6, 8] {
            let mut calls = 0;
            assert!(
                build(&x, &y, [1, 2, 3], || {
                    calls += 1;
                    if calls == stop {
                        Err("Cancelled".into())
                    } else {
                        Ok(())
                    }
                })
                .is_err()
            );
            assert_eq!(calls, stop);
        }
        let a = build(&x, &y, [1, 2, 3], || Ok(())).unwrap();
        assert_eq!(a.patch_face_count(), 1024);
        assert!(
            a.points
                .iter()
                .enumerate()
                .all(|(i, p)| p[0].to_bits() == x.data()[i].to_bits()
                    && p[1].to_bits() == y.data()[i].to_bits())
        );
    }
    #[test]
    fn clip_extreme_rectangle_and_triangle_without_overflow() {
        let rectangle = [
            [-f64::MAX, -f64::MAX],
            [f64::MAX, -f64::MAX],
            [f64::MAX, f64::MAX],
            [-f64::MAX, f64::MAX],
        ];
        let clipped = clip_patch_face(&rectangle, [-1., 1., -1., 1.]);
        assert_eq!(clipped.points().len(), 4);
        assert!(
            clipped
                .points()
                .iter()
                .all(|p| p[0].abs() == 1. && p[1].abs() == 1.)
        );
        let triangle = [[-2., 0.], [2., 0.], [0., 2.]];
        let clipped = clip_patch_face(&triangle, [-1., 1., -1., 1.]);
        assert_eq!(clipped.points().len(), 4);
        assert!(
            clipped
                .points()
                .iter()
                .all(|p| p[0].abs() <= 1. && (0. ..=1.).contains(&p[1]))
        );
        let extreme = [[-f64::MAX, f64::MAX], [f64::MAX, f64::MAX], [0., 0.]];
        let clipped = clip_patch_face(&extreme, [-f64::MAX * 0.7, f64::MAX * 0.3, 0., f64::MAX]);
        assert!(clipped.points().len() >= 3);
        assert!(clipped.points().iter().flatten().all(|v| v.is_finite()));
        assert!(clipped.points().iter().any(|p| p[1] == f64::MAX));
    }

    #[test]
    fn extreme_original_edges_preserve_diagonals_slopes_and_direction() {
        let m = f64::MAX;
        for (a, b, expected) in [
            ([-m, -m], [m, m], [[-1., -1.], [1., 1.]]),
            ([-m, m], [m, -m], [[-1., 1.], [1., -1.]]),
            ([-m / 2., -m], [m / 2., m], [[-0.5, -1.], [0.5, 1.]]),
            ([-m, -m / 2.], [m, m / 2.], [[-1., -0.5], [1., 0.5]]),
        ] {
            assert_eq!(clip_face_edge(a, b, [-1., 1., -1., 1.]), Some(expected));
            assert_eq!(
                clip_face_edge(b, a, [-1., 1., -1., 1.]),
                Some([expected[1], expected[0]])
            );
        }
    }

    #[test]
    fn extreme_filled_triangles_retain_the_same_diagonal_boundary() {
        let m = f64::MAX;
        for (face, expected) in [
            (
                [[-m, -m], [m, m], [m, -m]],
                [[-1., -1.], [1., 1.], [1., -1.]],
            ),
            ([[-m, m], [m, -m], [m, m]], [[-1., 1.], [1., -1.], [1., 1.]]),
        ] {
            for points in [face, [face[2], face[1], face[0]]] {
                let clipped = clip_patch_face(&points, [-1., 1., -1., 1.]);
                assert_eq!(clipped.points().len(), 3);
                for point in expected {
                    assert!(
                        clipped.points().contains(&point),
                        "missing {point:?}: {:?}",
                        clipped.points()
                    );
                }
            }
        }
    }

    #[test]
    fn tiny_viewport_and_offset_endpoint_survive_normalization() {
        let tiny = f64::from_bits(1);
        let m = f64::MAX;
        assert_eq!(
            clip_face_edge([-m, -m], [m, m], [-tiny, tiny, -tiny, tiny]),
            Some([[-tiny, -tiny], [tiny, tiny]])
        );
        // y=x+tiny near the small endpoint; neither source normalization nor
        // a t rounded to one may erase the independently stored intercept.
        assert_eq!(
            face_boundary_intersection([-m, -m], [0., tiny], 0, -tiny),
            [-tiny, 0.]
        );
        assert_eq!(
            face_boundary_intersection([0., tiny], [-m, -m], 0, -tiny),
            [-tiny, 0.]
        );
    }

    #[test]
    fn compensated_determinant_retains_small_nonzero_intercept() {
        // Exact integer determinant differs from the rounded-product result:
        // (2^27+1)*(2^27-1) - 2^27*2^27 = -1.
        // Cross at x=0; exact y=1/(2^28+1), about 3.7e-9.
        let n = 134_217_728.;
        let a = [-n, -(n - 1.)];
        let b = [n + 1., n];
        let expected = 1. / (2. * n + 1.);
        assert_eq!(face_boundary_intersection(a, b, 0, 0.), [0., expected]);
        assert_eq!(face_boundary_intersection(b, a, 0, 0.), [0., expected]);
    }
}
