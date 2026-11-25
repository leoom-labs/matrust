//! Rotation vectors, the pinhole projection with lens distortion and the
//! checkerboard world points.
//!
//! Conventions (public MathWorks pages):
//! - rotvec2mat3d / rotmat2vec3d: premultiply, the Rodrigues matrix Rp of a
//!   column-point rotation (OpenCV's cv2.Rodrigues).
//! - rotationVectorToMatrix / rotationMatrixToVector, cameraParameters
//!   RotationMatrices and worldToImage(intrinsics, R, t, X): postmultiply,
//!   R = Rp.' and row points [Xc Yc Zc] = [X Y Z] * R + t.
//! - Distortion on normalized x = Xc/Zc, y = Yc/Zc with r2 = x^2 + y^2:
//!   xd = x (1 + k1 r2 + k2 r2^2 + k3 r2^3) + 2 p1 x y + p2 (r2 + 2 x^2),
//!   yd = y (1 + ...) + p1 (r2 + 2 y^2) + 2 p2 x y; then
//!   u = fx xd + s yd + cx, v = fy yd + cy.
use super::Intrinsics;

pub(super) type Mat3 = [[f64; 3]; 3];

/// The premultiply (column-point) Rodrigues matrix of an axis-angle vector.
pub(super) fn rodrigues(v: [f64; 3]) -> Mat3 {
    let theta = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if theta < f64::EPSILON {
        // First order: I + [v]x (exact identity for the zero vector).
        return [[1., -v[2], v[1]], [v[2], 1., -v[0]], [-v[1], v[0], 1.]];
    }
    let k = [v[0] / theta, v[1] / theta, v[2] / theta];
    let (s, c) = theta.sin_cos();
    let c1 = 1. - c;
    [
        [
            c + c1 * k[0] * k[0],
            c1 * k[0] * k[1] - s * k[2],
            c1 * k[0] * k[2] + s * k[1],
        ],
        [
            c1 * k[1] * k[0] + s * k[2],
            c + c1 * k[1] * k[1],
            c1 * k[1] * k[2] - s * k[0],
        ],
        [
            c1 * k[2] * k[0] - s * k[1],
            c1 * k[2] * k[1] + s * k[0],
            c + c1 * k[2] * k[2],
        ],
    ]
}

pub(super) fn transpose(m: &Mat3) -> Mat3 {
    let mut t = [[0.; 3]; 3];
    for (i, row) in m.iter().enumerate() {
        for (j, x) in row.iter().enumerate() {
            t[j][i] = *x;
        }
    }
    t
}

fn det(m: &Mat3) -> f64 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
}

fn max_abs(m: &Mat3) -> f64 {
    m.iter().flatten().fold(0f64, |a, x| a.max(x.abs()))
}

/// Frobenius norm without overflow of the squares.
fn frobenius(m: &Mat3) -> f64 {
    let big = max_abs(m);
    if big == 0. || !big.is_finite() {
        return big;
    }
    big * m
        .iter()
        .flatten()
        .map(|x| (x / big) * (x / big))
        .sum::<f64>()
        .sqrt()
}

/// The nearest rotation (orthogonal polar factor) by the scaled Newton
/// iteration X <- (g X + X^-T / g) / 2 with g = sqrt(|X^-T|_F / |X|_F), so a
/// scaled or badly conditioned matrix (c * R, R * diag(1, 1, 1e-25)) still
/// gives R. The scaling stops once a step changes X by at most 1e-2; the
/// result is returned after an unscaled step that changes X by at most 1e-10
/// (quadratic convergence: X is then orthogonal to rounding).
/// None for a singular (to rounding) or reflecting matrix, a non-finite
/// step, or no convergence in 50 steps.
fn nearest_rotation(m: &Mat3) -> Option<Mat3> {
    polar_rotation(m, 50)
}

fn polar_rotation(m: &Mat3, steps: usize) -> Option<Mat3> {
    // The polar factor of c * M (c > 0) is that of M: start at max |M| = 1.
    let big = max_abs(m);
    if big == 0. || !big.is_finite() {
        return None;
    }
    let mut x = m.map(|row| row.map(|v| v / big));
    // The sign of the determinant must be certain: |det| above the rounding
    // error of its six products.
    let products = (0..3)
        .map(|j| {
            let (a, b) = ((j + 1) % 3, (j + 2) % 3);
            x[0][j].abs() * ((x[1][a] * x[2][b]).abs() + (x[1][b] * x[2][a]).abs())
        })
        .sum::<f64>();
    if det(&x).abs() <= 8. * f64::EPSILON * products {
        return None;
    }
    let mut scaled = true;
    for _ in 0..steps {
        let d = det(&x);
        if !d.is_finite() || d <= 0. {
            return None;
        }
        // X^-T = cofactor(X) / det.
        let mut inv_t = [[0.; 3]; 3];
        for (i, row) in inv_t.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                let (r0, r1) = ((i + 1) % 3, (i + 2) % 3);
                let (c0, c1) = ((j + 1) % 3, (j + 2) % 3);
                *v = (x[r0][c0] * x[r1][c1] - x[r0][c1] * x[r1][c0]) / d;
            }
        }
        let g = if scaled {
            (frobenius(&inv_t) / frobenius(&x)).sqrt()
        } else {
            1.
        };
        if !g.is_finite() || g <= 0. {
            return None;
        }
        let mut next = [[0.; 3]; 3];
        let mut change = 0f64;
        for i in 0..3 {
            for j in 0..3 {
                next[i][j] = 0.5 * (g * x[i][j] + inv_t[i][j] / g);
                change = change.max((next[i][j] - x[i][j]).abs());
            }
        }
        if !change.is_finite() {
            return None;
        }
        x = next;
        if !scaled && change <= 1e-10 {
            return Some(x);
        }
        if change <= 1e-2 {
            scaled = false;
        }
    }
    None
}

/// The axis-angle vector of a premultiply rotation matrix (inverse of
/// [`rodrigues`]); the input is first replaced by its nearest rotation.
/// The angle is in [0, pi].
pub(super) fn rotation_vector(m: &Mat3) -> Option<[f64; 3]> {
    let r = nearest_rotation(m)?;
    let w = [r[2][1] - r[1][2], r[0][2] - r[2][0], r[1][0] - r[0][1]];
    let s = 0.5 * (w[0] * w[0] + w[1] * w[1] + w[2] * w[2]).sqrt();
    let c = ((r[0][0] + r[1][1] + r[2][2] - 1.) / 2.).clamp(-1., 1.);
    if s < 1e-5 && c < 0. {
        // Angle near pi: the axis from R + I = 2 k k' (largest column).
        let b = [
            [
                (r[0][0] + 1.) / 2.,
                (r[0][1] + r[1][0]) / 4.,
                (r[0][2] + r[2][0]) / 4.,
            ],
            [
                (r[0][1] + r[1][0]) / 4.,
                (r[1][1] + 1.) / 2.,
                (r[1][2] + r[2][1]) / 4.,
            ],
            [
                (r[0][2] + r[2][0]) / 4.,
                (r[1][2] + r[2][1]) / 4.,
                (r[2][2] + 1.) / 2.,
            ],
        ];
        let i = (0..3)
            .max_by(|&a, &b2| b[a][a].total_cmp(&b[b2][b2]))
            .unwrap();
        let n = b[i][i].max(0.).sqrt();
        let mut k = [b[0][i] / n, b[1][i] / n, b[2][i] / n];
        let norm = (k[0] * k[0] + k[1] * k[1] + k[2] * k[2]).sqrt();
        k = [k[0] / norm, k[1] / norm, k[2] / norm];
        let theta = s.atan2(c);
        // The sign of the axis follows the (small) skew part when present.
        let dot = k[0] * w[0] + k[1] * w[1] + k[2] * w[2];
        let sign = if dot < 0. { -1. } else { 1. };
        return Some([
            sign * theta * k[0],
            sign * theta * k[1],
            sign * theta * k[2],
        ]);
    }
    if s < f64::EPSILON {
        // Near identity: first order.
        return Some([w[0] / 2., w[1] / 2., w[2] / 2.]);
    }
    let theta = s.atan2(c);
    let f = theta / (2. * s);
    Some([w[0] * f, w[1] * f, w[2] * f])
}

/// Apply radial and tangential distortion to normalized coordinates.
pub(super) fn distort(intr: &Intrinsics, x: f64, y: f64) -> (f64, f64) {
    let r2 = x * x + y * y;
    let k = |i: usize| intr.radial.get(i).copied().unwrap_or(0.);
    let radial = 1. + r2 * (k(0) + r2 * (k(1) + r2 * k(2)));
    let [p1, p2] = intr.tangential;
    (
        x * radial + 2. * p1 * x * y + p2 * (r2 + 2. * x * x),
        y * radial + p1 * (r2 + 2. * y * y) + 2. * p2 * x * y,
    )
}

/// One world point (row form) to pixel coordinates and its camera depth.
pub(super) fn project(
    intr: &Intrinsics,
    r: &Mat3,
    t: [f64; 3],
    p: [f64; 3],
    apply_distortion: bool,
) -> ([f64; 2], f64) {
    let cam = [0, 1, 2].map(|j| p[0] * r[0][j] + p[1] * r[1][j] + p[2] * r[2][j] + t[j]);
    let (mut x, mut y) = (cam[0] / cam[2], cam[1] / cam[2]);
    if apply_distortion {
        (x, y) = distort(intr, x, y);
    }
    let [fx, fy] = intr.focal;
    let [cx, cy] = intr.principal;
    ([fx * x + intr.skew * y + cx, fy * y + cy], cam[2])
}

/// generateCheckerboardPoints: (rows-1)*(cols-1) inner corners, [x y],
/// origin at the corner of the top-left square; x runs along the board
/// columns, y down the rows, y varying fastest.
pub(super) fn checkerboard(rows: usize, cols: usize, square: f64) -> Vec<[f64; 2]> {
    let mut p = Vec::with_capacity((rows - 1) * (cols - 1));
    for j in 0..cols - 1 {
        for i in 0..rows - 1 {
            p.push([j as f64 * square, i as f64 * square]);
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_round_trip() {
        for v in [
            [0.3, -0.2, 0.9],
            [0., 0., 0.],
            [1e-9, 0., 0.],
            [3.1, 0.2, -0.1],
            [std::f64::consts::PI, 0., 0.],
            [0., -std::f64::consts::PI / 2., 0.],
        ] {
            let back = rotation_vector(&rodrigues(v)).unwrap();
            for i in 0..3 {
                assert!((back[i] - v[i]).abs() < 1e-12, "{v:?} -> {back:?}");
            }
        }
    }

    fn close(m: &Mat3, v: [f64; 3], tol: f64) {
        let back = rotation_vector(m).unwrap_or_else(|| panic!("{m:?}: not a rotation"));
        for i in 0..3 {
            assert!((back[i] - v[i]).abs() < tol, "{m:?} -> {back:?}, not {v:?}");
        }
    }

    /// The polar factor of c * R and of R * S (S symmetric positive
    /// definite) is R: scaling and stretching must not change the vector.
    #[test]
    fn nearest_rotation_ignores_scale_and_stretch() {
        for v in [[0.3, 0., 0.], [0.3, -0.2, 0.9], [-2.5, 1.1, 0.4]] {
            let r = rodrigues(v);
            for c in [1e20, 1e-20, 3.7, 1e150, 1e-150] {
                close(&r.map(|row| row.map(|x| c * x)), v, 1e-14);
            }
            // R * diag(1, 1, 1e-25) and R * diag(5, 1e-9, 1).
            close(&r.map(|row| [row[0], row[1], 1e-25 * row[2]]), v, 1e-14);
            close(&r.map(|row| [5. * row[0], 1e-9 * row[1], row[2]]), v, 1e-14);
            // R * [2 1 0; 1 3 1; 0 1 4].
            let s = [[2., 1., 0.], [1., 3., 1.], [0., 1., 4.]];
            let m = [0, 1, 2]
                .map(|i| [0, 1, 2].map(|j| (0..3).map(|k| r[i][k] * s[k][j]).sum::<f64>()));
            close(&m, v, 1e-14);
        }
    }

    #[test]
    fn nearest_rotation_rejects_what_is_not_a_rotation() {
        let r = rodrigues([0.3, -0.2, 0.9]);
        // Singular: exactly, to rounding, and a zero matrix.
        assert!(rotation_vector(&[[1., 2., 3.], [4., 5., 6.], [7., 8., 9.]]).is_none());
        assert!(rotation_vector(&[[0.1, 0.2, 0.3], [0.4, 0.5, 0.6], [0.7, 0.8, 0.9]]).is_none());
        assert!(rotation_vector(&r.map(|row| [row[0], row[1], 0.])).is_none());
        assert!(rotation_vector(&[[0.; 3]; 3]).is_none());
        // Reflections.
        assert!(rotation_vector(&[[1., 0., 0.], [0., 1., 0.], [0., 0., -1.]]).is_none());
        assert!(rotation_vector(&r.map(|row| row.map(|x| -x))).is_none());
        // Non-finite.
        assert!(rotation_vector(&[[f64::NAN; 3]; 3]).is_none());
        assert!(rotation_vector(&[[f64::INFINITY, 0., 0.], [0., 1., 0.], [0., 0., 1.]]).is_none());
    }

    /// A step limit that is too small gives None, never a half-way matrix.
    #[test]
    fn nearest_rotation_reports_no_convergence() {
        let r = rodrigues([0.3, -0.2, 0.9]);
        let m = r.map(|row| [row[0], row[1], 1e-25 * row[2]]);
        for steps in 0..4 {
            assert!(polar_rotation(&m, steps).is_none(), "{steps} steps");
        }
        assert!(polar_rotation(&m, 50).is_some());
    }
}
