//! The 2-D geometric transformation objects: affinetform2d, rigidtform2d,
//! simtform2d and transltform2d (premultiply: `[x; y; 1] = A * [u; v; 1]`) and
//! the older affine2d (postmultiply: `[x y 1] = [u v 1] * T`, so `A = T.'`).
//! Written from the public MathWorks reference pages (constructor forms,
//! property relations, matrix forms); no MathWorks code.
//!
//! Structure checks (rigid, similarity, translation, last row/column) use the
//! absolute tolerance [`TOL`] on the 2-by-2 part, scaled by the scale factor
//! for similarities. Inverse point mapping solves with the LU factors of the
//! 2-by-2 part (partial pivoting); rigid and similarity use the transposed
//! rotation, which is exact.
use crate::Result;

pub(in crate::engine) type Mat2 = [[f64; 2]; 2];
/// Row-major: `m[row][col]`.
pub(in crate::engine) type Mat3 = [[f64; 3]; 3];

/// Absolute tolerance of the structure checks (MatRust's choice: well above
/// rounding noise of computed rotations, far below any deliberate shear).
pub(in crate::engine) const TOL: f64 = 1e-10;

const IDENTITY2: Mat2 = [[1., 0.], [0., 1.]];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine) enum Kind {
    Affine,
    Rigid,
    Sim,
    Transl,
    Affine2d,
}

impl Kind {
    pub(in crate::engine) fn class(self) -> &'static str {
        match self {
            Kind::Affine => "affinetform2d",
            Kind::Rigid => "rigidtform2d",
            Kind::Sim => "simtform2d",
            Kind::Transl => "transltform2d",
            Kind::Affine2d => "affine2d",
        }
    }
    pub(in crate::engine) fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "affinetform2d" => Kind::Affine,
            "rigidtform2d" => Kind::Rigid,
            "simtform2d" => Kind::Sim,
            "transltform2d" => Kind::Transl,
            "affine2d" => Kind::Affine2d,
            _ => return None,
        })
    }
}

/// One transformation object. Rigid and similarity keep the rotation angle
/// (degrees, in [-180, 180)) next to its matrix so a constructed angle shows
/// exactly as given.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::engine) enum Tform {
    /// affinetform2d: premultiply A, last row [0 0 1].
    Affine(Mat3),
    /// rigidtform2d: A = [r t; 0 0 1].
    Rigid { angle: f64, r: Mat2, t: [f64; 2] },
    /// simtform2d: A = [scale*r t; 0 0 1], scale > 0.
    Sim {
        scale: f64,
        angle: f64,
        r: Mat2,
        t: [f64; 2],
    },
    /// transltform2d: A = [1 0 tx; 0 1 ty; 0 0 1].
    Transl([f64; 2]),
    /// affine2d: postmultiply T, last column [0; 0; 1].
    Affine2d(Mat3),
}

/// Exact degree reduction `x = 360 m + 90 k + r`, `|r| <= 45`, as the
/// engine's sind/cosd (trig_more.rs) do, so `rigidtform2d(r, t).R` equals
/// `[cosd(r) -sind(r); sind(r) cosd(r)]` bit for bit.
fn reduce_degrees(x: f64) -> (u8, f64) {
    let r = x % 360.;
    let k = (r / 90.).round();
    let r = if k == 0. { r } else { r - 90. * k };
    ((k as i64).rem_euclid(4) as u8, r)
}

fn quadrant(x: f64, cosine: bool) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    let (k, r) = reduce_degrees(x);
    let k = if cosine { (k + 1) % 4 } else { k };
    if r == 0. {
        return match k {
            0 if x == 0. => x,
            0 | 2 => 0.,
            1 => 1.,
            _ => -1.,
        };
    }
    let t = r.to_radians();
    match k {
        0 => libm::sin(t),
        1 => libm::cos(t),
        2 => -libm::sin(t),
        _ => -libm::cos(t),
    }
}

pub(in crate::engine) fn sind(x: f64) -> f64 {
    quadrant(x, false)
}

pub(in crate::engine) fn cosd(x: f64) -> f64 {
    quadrant(x, true)
}

/// Four-quadrant arctangent in degrees, exact on the axes.
pub(in crate::engine) fn atan2d(y: f64, x: f64) -> f64 {
    if y == 0. && x > 0. {
        0.
    } else if x == 0. && y != 0. {
        90f64.copysign(y)
    } else if y == 0. && x < 0. {
        180.
    } else {
        libm::atan2(y, x).to_degrees()
    }
}

/// An angle in [-180, 180); angles already inside are returned unchanged.
pub(in crate::engine) fn wrap_angle(a: f64) -> f64 {
    if (-180. ..180.).contains(&a) {
        return a + 0.;
    }
    let r = (a + 180.).rem_euclid(360.) - 180.;
    if r >= 180. { -180. } else { r + 0. }
}

/// `[cosd(a) -sind(a); sind(a) cosd(a)]`.
pub(in crate::engine) fn rotation(angle: f64) -> Mat2 {
    let (c, s) = (cosd(angle), sind(angle));
    [[c, -s], [s, c]]
}

fn transpose2(m: &Mat2) -> Mat2 {
    [[m[0][0], m[1][0]], [m[0][1], m[1][1]]]
}

pub(in crate::engine) fn transpose3(m: &Mat3) -> Mat3 {
    let mut out = [[0.; 3]; 3];
    for (i, row) in m.iter().enumerate() {
        for (j, v) in row.iter().enumerate() {
            out[j][i] = *v;
        }
    }
    out
}

fn block(a: &Mat3) -> Mat2 {
    [[a[0][0], a[0][1]], [a[1][0], a[1][1]]]
}

fn compose(m: &Mat2, t: [f64; 2]) -> Mat3 {
    [
        [m[0][0], m[0][1], t[0]],
        [m[1][0], m[1][1], t[1]],
        [0., 0., 1.],
    ]
}

/// `-x` without producing a negative zero (displays as 0, compares equal).
fn neg(x: f64) -> f64 {
    -x + 0.
}

/// LU factors of a 2-by-2 matrix with partial pivoting.
#[derive(Clone, Copy, Debug)]
pub(in crate::engine) struct Lu2 {
    swap: bool,
    a: f64,
    b: f64,
    l: f64,
    d: f64,
}

impl Lu2 {
    /// None when the matrix is singular (a zero pivot) or not finite.
    pub(in crate::engine) fn new(m: &Mat2) -> Option<Self> {
        let (r0, r1, swap) = if m[1][0].abs() > m[0][0].abs() {
            (m[1], m[0], true)
        } else {
            (m[0], m[1], false)
        };
        if r0[0] == 0. || !r0[0].is_finite() {
            return None;
        }
        let l = r1[0] / r0[0];
        let d = r1[1] - l * r0[1];
        if d == 0. || !d.is_finite() {
            return None;
        }
        Some(Self {
            swap,
            a: r0[0],
            b: r0[1],
            l,
            d,
        })
    }
    /// The solution of `M [u; v] = [p; q]`.
    pub(in crate::engine) fn solve(&self, p: f64, q: f64) -> (f64, f64) {
        let (p, q) = if self.swap { (q, p) } else { (p, q) };
        let q = q - self.l * p;
        let v = q / self.d;
        let u = (p - self.b * v) / self.a;
        (u, v)
    }
}

/// Rotation within [`TOL`]: orthonormal columns, determinant +1.
fn is_rotation(m: &Mat2) -> bool {
    let [[a, b], [c, d]] = *m;
    (a * a + c * c - 1.).abs() <= TOL
        && (b * b + d * d - 1.).abs() <= TOL
        && (a * b + c * d).abs() <= TOL
        && a * d - b * c > 0.
}

/// `s * Q` with `Q` orthogonal (reflection allowed): the scale, if so.
fn similarity_scale(m: &Mat2) -> Option<f64> {
    let [[a, b], [c, d]] = *m;
    let s2 = (a * a + b * b + c * c + d * d) / 2.;
    if !s2.is_finite() || s2 <= 0. {
        return None;
    }
    let ok = (a * a + c * c - s2).abs() <= TOL * s2
        && (b * b + d * d - s2).abs() <= TOL * s2
        && (a * b + c * d).abs() <= TOL * s2;
    ok.then(|| s2.sqrt())
}

fn is_identity(m: &Mat2) -> bool {
    (m[0][0] - 1.).abs() <= TOL
        && m[0][1].abs() <= TOL
        && m[1][0].abs() <= TOL
        && (m[1][1] - 1.).abs() <= TOL
}

fn finite(who: &str, values: &[f64]) -> Result<()> {
    if values.iter().all(|v| v.is_finite()) {
        Ok(())
    } else {
        Err(format!("{who}: the values must be finite"))
    }
}

fn translation_checked(who: &str, t: [f64; 2]) -> Result<[f64; 2]> {
    finite(who, &t)?;
    Ok([t[0] + 0., t[1] + 0.])
}

impl Tform {
    pub(in crate::engine) fn kind(&self) -> Kind {
        match self {
            Tform::Affine(_) => Kind::Affine,
            Tform::Rigid { .. } => Kind::Rigid,
            Tform::Sim { .. } => Kind::Sim,
            Tform::Transl(_) => Kind::Transl,
            Tform::Affine2d(_) => Kind::Affine2d,
        }
    }
    pub(in crate::engine) fn class(&self) -> &'static str {
        self.kind().class()
    }
    pub(in crate::engine) fn identity(kind: Kind) -> Self {
        let i3 = compose(&IDENTITY2, [0., 0.]);
        match kind {
            Kind::Affine => Tform::Affine(i3),
            Kind::Rigid => Tform::Rigid {
                angle: 0.,
                r: IDENTITY2,
                t: [0., 0.],
            },
            Kind::Sim => Tform::Sim {
                scale: 1.,
                angle: 0.,
                r: IDENTITY2,
                t: [0., 0.],
            },
            Kind::Transl => Tform::Transl([0., 0.]),
            Kind::Affine2d => Tform::Affine2d(i3),
        }
    }
    /// rigidtform2d(angle, t): rotation by `angle` degrees counterclockwise
    /// about the origin, then translation.
    pub(in crate::engine) fn rigid(angle: f64, t: [f64; 2]) -> Result<Self> {
        finite("rigidtform2d", &[angle])?;
        let angle = wrap_angle(angle);
        Ok(Tform::Rigid {
            angle,
            r: rotation(angle),
            t: translation_checked("rigidtform2d", t)?,
        })
    }
    /// rigidtform2d(R, t); R must be a rotation matrix.
    pub(in crate::engine) fn rigid_from_rotation(r: Mat2, t: [f64; 2]) -> Result<Self> {
        let who = "rigidtform2d";
        finite(who, &[r[0][0], r[0][1], r[1][0], r[1][1]])?;
        if !is_rotation(&r) {
            return Err(format!(
                "{who}: R must be a rotation matrix [cosd(r) -sind(r); sind(r) cosd(r)] (orthonormal with determinant 1)"
            ));
        }
        Ok(Tform::Rigid {
            angle: wrap_angle(atan2d(r[1][0], r[0][0])),
            r,
            t: translation_checked(who, t)?,
        })
    }
    /// simtform2d(scale, angle, t).
    pub(in crate::engine) fn similarity(scale: f64, angle: f64, t: [f64; 2]) -> Result<Self> {
        let who = "simtform2d";
        finite(who, &[scale, angle])?;
        if scale <= 0. {
            return Err(format!("{who}: the scale must be positive"));
        }
        let angle = wrap_angle(angle);
        Ok(Tform::Sim {
            scale,
            angle,
            r: rotation(angle),
            t: translation_checked(who, t)?,
        })
    }
    /// simtform2d(scale, R, t); R must be a rotation matrix.
    pub(in crate::engine) fn similarity_from_rotation(
        scale: f64,
        r: Mat2,
        t: [f64; 2],
    ) -> Result<Self> {
        let who = "simtform2d";
        finite(who, &[scale])?;
        if scale <= 0. {
            return Err(format!("{who}: the scale must be positive"));
        }
        let Tform::Rigid { angle, r, t } =
            Self::rigid_from_rotation(r, t).map_err(|e| e.replace("rigidtform2d", who))?
        else {
            unreachable!("rigid_from_rotation makes a rigid transformation")
        };
        Ok(Tform::Sim { scale, angle, r, t })
    }
    /// transltform2d(tx, ty).
    pub(in crate::engine) fn translation(t: [f64; 2]) -> Result<Self> {
        Ok(Tform::Transl(translation_checked("transltform2d", t)?))
    }
    /// affinetform2d(A) with a premultiply 3-by-3 A.
    pub(in crate::engine) fn affine(a: Mat3) -> Result<Self> {
        Self::from_premultiply(Kind::Affine, a, "affinetform2d")
    }
    /// affine2d(T) with a postmultiply 3-by-3 T.
    pub(in crate::engine) fn affine2d(t: Mat3) -> Result<Self> {
        Self::from_premultiply(Kind::Affine2d, transpose3(&t), "affine2d")
    }
    /// The object of class `kind` whose premultiply matrix is `a`, or an
    /// error (prefixed with `who`) when `a` is not of that kind.
    pub(in crate::engine) fn from_premultiply(kind: Kind, a: Mat3, who: &str) -> Result<Self> {
        finite(who, a.as_flattened())?;
        if a[2][0].abs() > TOL || a[2][1].abs() > TOL || (a[2][2] - 1.).abs() > TOL {
            return Err(if kind == Kind::Affine2d {
                format!("{who}: the last column of T must be [0; 0; 1]")
            } else {
                format!("{who}: the last row of the matrix must be [0 0 1]")
            });
        }
        let m = block(&a);
        let t = [a[0][2] + 0., a[1][2] + 0.];
        if Lu2::new(&m).is_none() {
            let name = if kind == Kind::Affine2d { "T" } else { "A" };
            return Err(format!("{who}: {name} must be invertible (nonsingular)"));
        }
        Ok(match kind {
            Kind::Affine => Tform::Affine(compose(&m, t)),
            Kind::Affine2d => Tform::Affine2d(transpose3(&compose(&m, t))),
            Kind::Rigid => {
                if !is_rotation(&m) {
                    return Err(format!(
                        "{who}: the matrix is not a rigid transformation (its 2-by-2 part must be a rotation: orthonormal with determinant 1)"
                    ));
                }
                Tform::Rigid {
                    angle: wrap_angle(atan2d(m[1][0], m[0][0])),
                    r: m,
                    t,
                }
            }
            Kind::Sim => {
                let det = m[0][0] * m[1][1] - m[0][1] * m[1][0];
                let Some(scale) = similarity_scale(&m).filter(|_| det > 0.) else {
                    return Err(format!(
                        "{who}: the matrix is not a similarity transformation (its 2-by-2 part must be a positive scale times a rotation; reflection is not supported)"
                    ));
                };
                let r = [
                    [m[0][0] / scale, m[0][1] / scale],
                    [m[1][0] / scale, m[1][1] / scale],
                ];
                Tform::Sim {
                    scale,
                    angle: wrap_angle(atan2d(r[1][0], r[0][0])),
                    r,
                    t,
                }
            }
            Kind::Transl => {
                if !is_identity(&m) {
                    return Err(format!(
                        "{who}: the matrix is not a translation (its 2-by-2 part must be the identity)"
                    ));
                }
                Tform::Transl(t)
            }
        })
    }
    /// The premultiply matrix A (`T.'` for affine2d).
    pub(in crate::engine) fn matrix(&self) -> Mat3 {
        match self {
            Tform::Affine(a) => *a,
            Tform::Rigid { r, t, .. } => compose(r, *t),
            Tform::Sim { scale, r, t, .. } => compose(
                &[
                    [scale * r[0][0], scale * r[0][1]],
                    [scale * r[1][0], scale * r[1][1]],
                ],
                *t,
            ),
            Tform::Transl(t) => compose(&IDENTITY2, *t),
            Tform::Affine2d(t) => transpose3(t),
        }
    }
    /// The forward map applied to every point, in place: `x[i], y[i]` become
    /// the transformed coordinates. Polls `poll` every 65536 points.
    pub(in crate::engine) fn forward_points(
        &self,
        x: &mut [f64],
        y: &mut [f64],
        mut poll: impl FnMut() -> Result<()>,
    ) -> Result<()> {
        let a = self.matrix();
        for (i, (x, y)) in x.iter_mut().zip(y.iter_mut()).enumerate() {
            if i % 65536 == 65535 {
                poll()?;
            }
            let (u, v) = (*x, *y);
            *x = a[0][0] * u + a[0][1] * v + a[0][2];
            *y = a[1][0] * u + a[1][1] * v + a[1][2];
        }
        Ok(())
    }
    /// The inverse map applied to every point, in place.
    pub(in crate::engine) fn inverse_points(
        &self,
        x: &mut [f64],
        y: &mut [f64],
        mut poll: impl FnMut() -> Result<()>,
    ) -> Result<()> {
        let a = self.matrix();
        let (tx, ty) = (a[0][2], a[1][2]);
        // Rigid and similarity: the rotation's transpose is its inverse.
        let orthogonal = match self {
            Tform::Rigid { r, .. } => Some((transpose2(r), 1.)),
            Tform::Sim { scale, r, .. } => Some((transpose2(r), *scale)),
            Tform::Transl(_) => Some((IDENTITY2, 1.)),
            _ => None,
        };
        let lu = match orthogonal {
            Some(_) => None,
            None => Some(Lu2::new(&block(&a)).ok_or("transformation matrix is singular")?),
        };
        for (i, (x, y)) in x.iter_mut().zip(y.iter_mut()).enumerate() {
            if i % 65536 == 65535 {
                poll()?;
            }
            let (p, q) = (*x - tx, *y - ty);
            let (u, v) = match (&orthogonal, &lu) {
                (Some((rt, s)), _) => (
                    (rt[0][0] * p + rt[0][1] * q) / s,
                    (rt[1][0] * p + rt[1][1] * q) / s,
                ),
                (None, Some(lu)) => lu.solve(p, q),
                (None, None) => unreachable!("one of the two inverse forms is set"),
            };
            *x = u;
            *y = v;
        }
        Ok(())
    }
    /// The inverse transformation, of the same class.
    pub(in crate::engine) fn inverse(&self) -> Result<Self> {
        Ok(match self {
            Tform::Affine(a) => {
                let lu = Lu2::new(&block(a)).ok_or("invert: the transformation is singular")?;
                let (i00, i10) = lu.solve(1., 0.);
                let (i01, i11) = lu.solve(0., 1.);
                let (t0, t1) = lu.solve(neg(a[0][2]), neg(a[1][2]));
                Tform::Affine(compose(&[[i00, i01], [i10, i11]], [t0 + 0., t1 + 0.]))
            }
            Tform::Affine2d(t) => {
                let Tform::Affine(inv) = Tform::Affine(transpose3(t)).inverse()? else {
                    unreachable!("an affine inverse is affine")
                };
                Tform::Affine2d(transpose3(&inv))
            }
            Tform::Rigid { angle, r, t } => {
                let rt = transpose2(r);
                Tform::Rigid {
                    angle: if *angle == 0. { 0. } else { wrap_angle(-angle) },
                    r: rt,
                    t: [
                        neg(rt[0][0] * t[0] + rt[0][1] * t[1]),
                        neg(rt[1][0] * t[0] + rt[1][1] * t[1]),
                    ],
                }
            }
            Tform::Sim { scale, angle, r, t } => {
                let rt = transpose2(r);
                Tform::Sim {
                    scale: 1. / scale,
                    angle: if *angle == 0. { 0. } else { wrap_angle(-angle) },
                    r: rt,
                    t: [
                        neg((rt[0][0] * t[0] + rt[0][1] * t[1]) / scale),
                        neg((rt[1][0] * t[0] + rt[1][1] * t[1]) / scale),
                    ],
                }
            }
            Tform::Transl(t) => Tform::Transl([neg(t[0]), neg(t[1])]),
        })
    }
    /// Rotation and translation only (no reflection, no scaling), within [`TOL`].
    pub(in crate::engine) fn is_rigid(&self) -> bool {
        is_rotation(&block(&self.matrix()))
    }
    /// Rotation, isotropic scaling, reflection and translation, within [`TOL`].
    pub(in crate::engine) fn is_similarity(&self) -> bool {
        similarity_scale(&block(&self.matrix())).is_some()
    }
    /// Translation only, within [`TOL`].
    pub(in crate::engine) fn is_translation(&self) -> bool {
        is_identity(&block(&self.matrix()))
    }
    /// Bounding box `([xmin xmax], [ymin ymax])` of the transformed rectangle
    /// `x_limits` by `y_limits`. An affine map sends the rectangle to a
    /// parallelogram, so the extremes are at the transformed corners.
    pub(in crate::engine) fn output_limits(
        &self,
        x_limits: [f64; 2],
        y_limits: [f64; 2],
    ) -> ([f64; 2], [f64; 2]) {
        let mut x = [x_limits[0], x_limits[1], x_limits[0], x_limits[1]];
        let mut y = [y_limits[0], y_limits[0], y_limits[1], y_limits[1]];
        self.forward_points(&mut x, &mut y, || Ok(()))
            .expect("four points never poll");
        let range = |v: [f64; 4]| {
            [
                v.iter().copied().fold(f64::INFINITY, f64::min),
                v.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            ]
        };
        (range(x), range(y))
    }
    /// Public property names in display order.
    pub(in crate::engine) fn property_names(&self) -> &'static [&'static str] {
        match self {
            Tform::Affine(_) => &["Dimensionality", "A"],
            Tform::Rigid { .. } => &["Dimensionality", "RotationAngle", "Translation", "R", "A"],
            Tform::Sim { .. } => &[
                "Dimensionality",
                "Scale",
                "RotationAngle",
                "Translation",
                "R",
                "A",
            ],
            Tform::Transl(_) => &["Dimensionality", "Translation", "A"],
            Tform::Affine2d(_) => &["T", "Dimensionality"],
        }
    }
    /// Methods of this class (sorted, as `methods` lists them).
    pub(in crate::engine) fn method_names(&self) -> &'static [&'static str] {
        match self {
            Tform::Affine(_) | Tform::Affine2d(_) => &[
                "invert",
                "isRigid",
                "isSimilarity",
                "isTranslation",
                "outputLimits",
                "transformPointsForward",
                "transformPointsInverse",
            ],
            _ => &[
                "invert",
                "outputLimits",
                "transformPointsForward",
                "transformPointsInverse",
            ],
        }
    }
    /// A property value as rows, columns and column-major data.
    pub(in crate::engine) fn property(&self, name: &str) -> Option<(usize, usize, Vec<f64>)> {
        let mat3 = |m: &Mat3| (3, 3, (0..9).map(|k| m[k % 3][k / 3]).collect());
        let mat2 = |m: &Mat2| (2, 2, vec![m[0][0], m[1][0], m[0][1], m[1][1]]);
        if !self.property_names().contains(&name) {
            return None;
        }
        Some(match (name, self) {
            ("Dimensionality", _) => (1, 1, vec![2.]),
            ("A", _) => mat3(&self.matrix()),
            ("T", Tform::Affine2d(t)) => mat3(t),
            ("R", Tform::Rigid { r, .. } | Tform::Sim { r, .. }) => mat2(r),
            ("RotationAngle", Tform::Rigid { angle, .. } | Tform::Sim { angle, .. }) => {
                (1, 1, vec![*angle])
            }
            ("Scale", Tform::Sim { scale, .. }) => (1, 1, vec![*scale]),
            ("Translation", Tform::Rigid { t, .. } | Tform::Sim { t, .. } | Tform::Transl(t)) => {
                (1, 2, t.to_vec())
            }
            _ => return None,
        })
    }
    /// A copy with property `name` set to the real matrix `value`
    /// (`rows`-by-`cols`, column-major); other properties follow.
    pub(in crate::engine) fn with_property(
        &self,
        name: &str,
        rows: usize,
        cols: usize,
        value: &[f64],
    ) -> Result<Self> {
        let class = self.class();
        let scalar = |what: &str| -> Result<f64> {
            if value.len() != 1 {
                return Err(format!("{class}: {what} must be a numeric scalar"));
            }
            Ok(value[0])
        };
        let pair = || -> Result<[f64; 2]> {
            if value.len() != 2 || (rows != 1 && cols != 1) {
                return Err(format!(
                    "{class}: Translation must be a 2-element numeric vector [tx ty]"
                ));
            }
            Ok([value[0], value[1]])
        };
        let rot = || -> Result<Mat2> {
            if (rows, cols) != (2, 2) {
                return Err(format!("{class}: R must be a 2-by-2 rotation matrix"));
            }
            Ok([[value[0], value[2]], [value[1], value[3]]])
        };
        let full = |what: &str| -> Result<Mat3> {
            matrix3(
                class,
                what,
                rows,
                cols,
                value,
                self.kind() != Kind::Affine2d,
            )
        };
        if !self.property_names().contains(&name) {
            return Err(format!(
                "Unrecognized property '{name}' for class '{class}'."
            ));
        }
        match (name, self) {
            ("Dimensionality", _) => Err(format!(
                "You cannot set the read-only property 'Dimensionality' of {class}."
            )),
            ("A", _) => Self::from_premultiply(self.kind(), full("A")?, class),
            ("T", Tform::Affine2d(_)) => {
                Self::from_premultiply(Kind::Affine2d, transpose3(&full("T")?), class)
            }
            ("R", Tform::Rigid { t, .. }) => Self::rigid_from_rotation(rot()?, *t),
            ("R", Tform::Sim { scale, t, .. }) => {
                Self::similarity_from_rotation(*scale, rot()?, *t)
            }
            ("RotationAngle", Tform::Rigid { t, .. }) => Self::rigid(scalar("RotationAngle")?, *t),
            ("RotationAngle", Tform::Sim { scale, t, .. }) => {
                Self::similarity(*scale, scalar("RotationAngle")?, *t)
            }
            ("Scale", Tform::Sim { angle, r, t, .. }) => {
                let scale = scalar("Scale")?;
                finite(class, &[scale])?;
                if scale <= 0. {
                    return Err(format!("{class}: the scale must be positive"));
                }
                Ok(Tform::Sim {
                    scale,
                    angle: *angle,
                    r: *r,
                    t: *t,
                })
            }
            ("Translation", Tform::Rigid { angle, r, .. }) => Ok(Tform::Rigid {
                angle: *angle,
                r: *r,
                t: translation_checked(class, pair()?)?,
            }),
            (
                "Translation",
                Tform::Sim {
                    scale, angle, r, ..
                },
            ) => Ok(Tform::Sim {
                scale: *scale,
                angle: *angle,
                r: *r,
                t: translation_checked(class, pair()?)?,
            }),
            ("Translation", Tform::Transl(_)) => {
                Self::translation(pair()?).map_err(|e| e.replace("transltform2d", class))
            }
            _ => Err(format!(
                "Unrecognized property '{name}' for class '{class}'."
            )),
        }
    }
}

/// A 3-by-3 matrix from `rows`-by-`cols` column-major values; with
/// `allow_top_rows`, a 2-by-3 matrix gets the row [0 0 1] appended.
pub(in crate::engine) fn matrix3(
    who: &str,
    what: &str,
    rows: usize,
    cols: usize,
    value: &[f64],
    allow_top_rows: bool,
) -> Result<Mat3> {
    match (rows, cols) {
        (3, 3) => Ok(std::array::from_fn(|i| {
            std::array::from_fn(|j| value[i + 3 * j])
        })),
        (2, 3) if allow_top_rows => Ok(std::array::from_fn(|i| {
            std::array::from_fn(|j| {
                if i < 2 {
                    value[i + 2 * j]
                } else {
                    [0., 0., 1.][j]
                }
            })
        })),
        _ if allow_top_rows => Err(format!(
            "{who}: {what} must be a 3-by-3 matrix (or its 2-by-3 top rows)"
        )),
        _ => Err(format!("{who}: {what} must be a 3-by-3 matrix")),
    }
}
