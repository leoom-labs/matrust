//! Bounded independent filled faces. Vertex data stays in Series' shared payload.
use super::{AxisTransform, Matrix, PlotKind, Result, Series};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

pub const MAX_PATCH_FACES: usize = 10_000;

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
            return Err("Patch exceeds 10000 faces".into());
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
        )
    }
    pub(crate) fn patch_from_matrices_with_ledger(
        x: &Matrix,
        y: &Matrix,
        color: [u8; 3],
        cancel: &AtomicBool,
        mut ledger: super::coordinates::GeometryLedger,
    ) -> Result<Self> {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        if x.rows() != y.rows() || x.cols() != y.cols() || !matches!(x.rows(), 3 | 4) {
            return Err(
                "Patch XData and YData must have equal three-by-N or four-by-N shape".into(),
            );
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
            (_, None) if self.points.patch().is_some() => {
                Err("Stored Patch coordinates require Patch kind".into())
            }
            (_, None) => Ok(()),
        }
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

#[cfg(test)]
mod tests {
    use super::*;
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
