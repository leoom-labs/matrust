//! Reference-element quadrature rules and Lagrange shape functions for 2-D
//! linear (P1) and isoparametric quadratic (P2) triangles and boundary edges.
use crate::Result;

/// A 2-D quadrature point on the reference triangle
/// `{(xi, eta) : xi >= 0, eta >= 0, xi + eta <= 1}` (area = 1/2).
#[derive(Clone, Copy, Debug)]
pub(crate) struct TriQuadPoint {
    pub(crate) xi: f64,
    pub(crate) eta: f64,
    pub(crate) w: f64,
}

/// 3-point degree-2 interior Gauss rule on the reference triangle (weights sum to 1/2).
pub(crate) const TRI_QUAD_P1: [TriQuadPoint; 3] = [
    TriQuadPoint {
        xi: 1.0 / 6.0,
        eta: 1.0 / 6.0,
        w: 1.0 / 6.0,
    },
    TriQuadPoint {
        xi: 2.0 / 3.0,
        eta: 1.0 / 6.0,
        w: 1.0 / 6.0,
    },
    TriQuadPoint {
        xi: 1.0 / 6.0,
        eta: 2.0 / 3.0,
        w: 1.0 / 6.0,
    },
];

// 7-point degree-5 Dunavant rule on the reference triangle (weights sum to 1/2).
const SQRT15: f64 = 3.872_983_346_207_417;
const R1: f64 = (6.0 - SQRT15) / 21.0;
const S1: f64 = (9.0 + 2.0 * SQRT15) / 21.0;
const W1: f64 = (155.0 - SQRT15) / 2400.0;
const R2: f64 = (6.0 + SQRT15) / 21.0;
const S2: f64 = (9.0 - 2.0 * SQRT15) / 21.0;
const W2: f64 = (155.0 + SQRT15) / 2400.0;

pub(crate) const TRI_QUAD_P2: [TriQuadPoint; 7] = [
    TriQuadPoint {
        xi: 1.0 / 3.0,
        eta: 1.0 / 3.0,
        w: 9.0 / 80.0,
    },
    TriQuadPoint {
        xi: R1,
        eta: R1,
        w: W1,
    },
    TriQuadPoint {
        xi: S1,
        eta: R1,
        w: W1,
    },
    TriQuadPoint {
        xi: R1,
        eta: S1,
        w: W1,
    },
    TriQuadPoint {
        xi: R2,
        eta: R2,
        w: W2,
    },
    TriQuadPoint {
        xi: S2,
        eta: R2,
        w: W2,
    },
    TriQuadPoint {
        xi: R2,
        eta: S2,
        w: W2,
    },
];

/// 3-point Gauss-Legendre rule on `[0, 1]` (exact up to degree 5; weights sum to 1).
const SQRT_3_5: f64 = 0.774_596_669_241_483_4;
pub(crate) const EDGE_QUAD_3: [(f64, f64); 3] = [
    (0.5 * (1.0 - SQRT_3_5), 5.0 / 18.0),
    (0.5, 8.0 / 18.0),
    (0.5 * (1.0 + SQRT_3_5), 5.0 / 18.0),
];

/// P1 shape function values at `(xi, eta)`.
#[inline]
pub(crate) fn p1_shape(xi: f64, eta: f64) -> [f64; 3] {
    [1.0 - xi - eta, xi, eta]
}

/// P1 constant Jacobian determinant and physical gradients `[dN/dx]` and `[dN/dy]`.
#[inline]
pub(crate) fn p1_jacobian_and_grads(coords: &[[f64; 2]; 3]) -> Result<(f64, [f64; 3], [f64; 3])> {
    let x_xi = coords[1][0] - coords[0][0];
    let x_eta = coords[2][0] - coords[0][0];
    let y_xi = coords[1][1] - coords[0][1];
    let y_eta = coords[2][1] - coords[0][1];
    let det = x_xi * y_eta - x_eta * y_xi;
    if !(det.is_finite() && det > 0.0) {
        return Err(
            "solvepde: degenerate or inverted linear element (non-positive Jacobian)".into(),
        );
    }
    let inv = 1.0 / det;
    // dN/dxi = [-1, 1, 0], dN/deta = [-1, 0, 1]
    // dN/dx = ( y_eta * dN/dxi - y_xi * dN/deta) / det
    // dN/dy = (-x_eta * dN/dxi + x_xi * dN/deta) / det
    let dnx = [(-y_eta + y_xi) * inv, y_eta * inv, -y_xi * inv];
    let dny = [(x_eta - x_xi) * inv, -x_eta * inv, x_xi * inv];
    Ok((det, dnx, dny))
}

/// P2 Lagrange shape functions and reference derivatives at `(xi, eta)`.
///
/// Node ordering matches MATLAB / `FeMesh`:
/// `0=(0,0), 1=(1,0), 2=(0,1), 3=(0.5,0), 4=(0.5,0.5), 5=(0,0.5)`.
#[inline]
pub(crate) fn p2_shape(xi: f64, eta: f64) -> ([f64; 6], [f64; 6], [f64; 6]) {
    let l0 = 1.0 - xi - eta;
    let l1 = xi;
    let l2 = eta;
    let n = [
        l0 * (2.0 * l0 - 1.0),
        l1 * (2.0 * l1 - 1.0),
        l2 * (2.0 * l2 - 1.0),
        4.0 * l0 * l1,
        4.0 * l1 * l2,
        4.0 * l2 * l0,
    ];
    let dxi = [
        -(4.0 * l0 - 1.0),
        4.0 * l1 - 1.0,
        0.0,
        4.0 * (l0 - l1),
        4.0 * l2,
        -4.0 * l2,
    ];
    let deta = [
        -(4.0 * l0 - 1.0),
        0.0,
        4.0 * l2 - 1.0,
        -4.0 * l1,
        4.0 * l1,
        4.0 * (l0 - l2),
    ];
    (n, dxi, deta)
}

/// Isoparametric P2 Jacobian determinant and physical gradients at a quadrature point.
#[inline]
pub(crate) fn p2_jacobian_and_grads(
    coords: &[[f64; 2]; 6],
    dxi: &[f64; 6],
    deta: &[f64; 6],
) -> Result<(f64, [f64; 6], [f64; 6])> {
    let mut x_xi = 0.0;
    let mut x_eta = 0.0;
    let mut y_xi = 0.0;
    let mut y_eta = 0.0;
    for a in 0..6 {
        x_xi += dxi[a] * coords[a][0];
        x_eta += deta[a] * coords[a][0];
        y_xi += dxi[a] * coords[a][1];
        y_eta += deta[a] * coords[a][1];
    }
    let det = x_xi * y_eta - x_eta * y_xi;
    if !(det.is_finite() && det > 0.0) {
        return Err(
            "solvepde: degenerate or inverted quadratic element (non-positive Jacobian)".into(),
        );
    }
    let inv = 1.0 / det;
    let mut dnx = [0.0; 6];
    let mut dny = [0.0; 6];
    for a in 0..6 {
        dnx[a] = (y_eta * dxi[a] - y_xi * deta[a]) * inv;
        dny[a] = (-x_eta * dxi[a] + x_xi * deta[a]) * inv;
    }
    Ok((det, dnx, dny))
}

/// 1-D linear edge shape functions on `t in [0, 1]`.
#[inline]
pub(crate) fn edge_p1_shape(t: f64) -> ([f64; 2], [f64; 2]) {
    ([1.0 - t, t], [-1.0, 1.0])
}

/// 1-D quadratic isoparametric edge shape functions on `t in [0, 1]` for `[n0, n1, mid]`.
#[inline]
pub(crate) fn edge_p2_shape(t: f64) -> ([f64; 3], [f64; 3]) {
    let n = [
        (1.0 - t) * (1.0 - 2.0 * t),
        t * (2.0 * t - 1.0),
        4.0 * t * (1.0 - t),
    ];
    let dt = [4.0 * t - 3.0, 4.0 * t - 1.0, 4.0 - 8.0 * t];
    (n, dt)
}
