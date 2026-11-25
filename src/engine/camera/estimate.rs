//! estimateCameraParameters (piece 7d3a): single-camera calibration from a
//! planar pattern's point sets (Zhang's method).
//!
//! 1. A homography per view by the normalised DLT (smallest eigenvector of
//!    A'A after Hartley normalisation of both point sets).
//! 2. Closed-form intrinsics from the homographies (Zhang 2000, B = K^-T K^-1;
//!    without EstimateSkew the row b12 = 0 is added), computed on image
//!    coordinates centred and scaled to about 1 so the 6-by-6 system is well
//!    conditioned. Extrinsics per view from K^-1 H, made a rotation by the
//!    polar factor. Distortion starts at zero. With few views the closed
//!    form is barely determined (exactly, with two) and distorted points can
//!    put it far off or make B indefinite, so a second start is also used up
//!    to TWO_START_VIEWS views and whenever the closed form gives no K: the
//!    principal point held at the image centre (the centroid of the points
//!    without ImageSize), focal lengths from the same orthogonality
//!    constraints (OpenCV's start).
//! 3. Levenberg-Marquardt over all parameters minimising the reprojection
//!    error, central-difference Jacobian blocks per view accumulated into the
//!    dense normal equations, solved by Cholesky with Marquardt's diagonal
//!    damping; once per start, the lower-cost fit is kept.
//!
//! Partially detected patterns ([NaN NaN] rows) leave those points out; a
//! view with fewer than 4 points or no homography is not used. InitialK /
//! InitialIntrinsicMatrix replace the closed-form start, and
//! InitialRadialDistortion the zero distortion start. The third output holds
//! the standard errors sqrt(diag(s2 (J'J)^-1)) at the solution, s2 = residual
//! sum of squares / (2 Npoints - Nparameters) (OpenCV's
//! calibrateCameraExtended agrees to 1e-5 relative).
//!
//! Parallel pattern planes cannot give the intrinsics: an error when the
//! closed-form system has no unique solution or the fitted plane normals of
//! all views agree within about one degree.
//!
//! Pixel coordinates are MATLAB's (the centre of the first pixel is (1, 1));
//! the model is projection.rs's, so the result reprojects with worldToImage.
use super::objects::{
    CalibrationErrors, ExtrinsicsErrors, Intrinsics, IntrinsicsErrors, MAX_POINTS, Params, pairs,
};
use super::projection::{self, Mat3};
use super::{Camera, err, logical_arg, num, text_arg};
use crate::engine::{Engine, Execution, Value};
use crate::{LogicalMatrix, Result};

const WHO: &str = "estimateCameraParameters";
/// Most views (the normal equations are dense in 6 per view).
const MAX_VIEWS: usize = 100;
const MAX_ITERATIONS: usize = 300;
/// Up to this many views the fit is also started from a fixed principal
/// point. Measured on random noise-free sets with k1 -0.1..-0.35: from the
/// closed form alone 12 of 60 two-view sets failed (8 without a K, 4 in a
/// wrong minimum), 3 of 400 three-view and 1 of 400 four-view sets ended in a
/// wrong minimum, none of 400 five-view sets.
const TWO_START_VIEWS: usize = 4;

/// The finite points of one used view: world points, their image points and
/// their rows in imagePoints.
struct View {
    world: Vec<[f64; 2]>,
    observed: Vec<[f64; 2]>,
    rows: Vec<usize>,
}

/// What is estimated besides fx, fy, cx, cy and the extrinsics.
#[derive(Clone, Copy)]
struct Model {
    skew: bool,
    radial: usize,
    tangential: bool,
}

impl Model {
    fn intrinsic_count(&self) -> usize {
        4 + usize::from(self.skew) + self.radial + 2 * usize::from(self.tangential)
    }
    /// Intrinsics from the parameter vector [fx fy cx cy (s) k.. (p1 p2)].
    fn intrinsics(&self, x: &[f64]) -> Intrinsics {
        let mut i = 4;
        let skew = if self.skew {
            i += 1;
            x[4]
        } else {
            0.
        };
        let radial = x[i..i + self.radial].to_vec();
        i += self.radial;
        let tangential = if self.tangential {
            [x[i], x[i + 1]]
        } else {
            [0., 0.]
        };
        Intrinsics {
            focal: [x[0], x[1]],
            principal: [x[2], x[3]],
            size: None,
            radial,
            tangential,
            skew,
        }
    }
}

/// `[params, imagesUsed, estimationErrors] = estimateCameraParameters(...)`.
pub(super) fn estimate(
    e: &mut Engine,
    args: &[Value],
    nargout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if nargout > 3 {
        return Err("Too many output arguments.".into());
    }
    let positional = args
        .iter()
        .position(|a| text_arg(e, a).is_some())
        .unwrap_or(args.len());
    let [image_points, world_points] = &args[..positional] else {
        return Err(err(
            WHO,
            "expected estimateCameraParameters(imagePoints, worldPoints, Name, Value, ...)",
        ));
    };
    let mut model = Model {
        skew: false,
        radial: 2,
        tangential: false,
    };
    let mut size = None;
    let mut units = String::from("mm");
    let mut initial_k: Option<Mat3> = None;
    let mut initial_radial: Option<Vec<f64>> = None;
    for (name, v) in pairs(
        e,
        WHO,
        &args[positional..],
        &[
            "ImageSize",
            "WorldUnits",
            "EstimateSkew",
            "NumRadialDistortionCoefficients",
            "EstimateTangentialDistortion",
            "InitialIntrinsicMatrix",
            "InitialK",
            "InitialRadialDistortion",
        ],
    )? {
        match name {
            "ImageSize" => {
                let n = num(e, WHO, name, v)?;
                if n.data.is_empty() {
                    size = None;
                } else {
                    let ok = n.data.len() == 2
                        && n.data
                            .iter()
                            .all(|x| x.is_finite() && *x >= 1. && x.fract() == 0. && *x <= 1e9);
                    if !ok {
                        return Err(err(
                            WHO,
                            "ImageSize must be [mrows ncols] with positive integers",
                        ));
                    }
                    size = Some([n.data[0], n.data[1]]);
                }
            }
            "WorldUnits" => {
                units = text_arg(e, v).ok_or_else(|| err(WHO, "WorldUnits must be text"))?;
            }
            "EstimateSkew" => model.skew = logical_arg(e, WHO, name, v)?,
            "EstimateTangentialDistortion" => model.tangential = logical_arg(e, WHO, name, v)?,
            "InitialIntrinsicMatrix" | "InitialK" => {
                let n = num(e, WHO, name, v)?;
                initial_k = if n.data.is_empty() {
                    None
                } else {
                    let (f, c, s) = Intrinsics::from_k(WHO, name, &n)?;
                    Some([[f[0], s, c[0]], [0., f[1], c[1]], [0., 0., 1.]])
                };
            }
            "InitialRadialDistortion" => {
                let n = num(e, WHO, name, v)?;
                initial_radial = if n.data.is_empty() {
                    None
                } else {
                    let vector = n.dims.len() == 2 && (n.dims[0] == 1 || n.dims[1] == 1);
                    if !vector
                        || !(n.data.len() == 2 || n.data.len() == 3)
                        || n.data.iter().any(|x| !x.is_finite())
                    {
                        return Err(err(
                            WHO,
                            "InitialRadialDistortion must be a finite 2- or 3-element vector",
                        ));
                    }
                    Some(n.data)
                };
            }
            _ => {
                let n = num(e, WHO, name, v)?;
                model.radial = match n.data.as_slice() {
                    [x] if *x == 2. || *x == 3. => *x as usize,
                    _ => return Err(err(WHO, "NumRadialDistortionCoefficients must be 2 or 3")),
                };
            }
        }
    }

    if let Some(r) = &initial_radial
        && r.len() != model.radial
    {
        return Err(err(
            WHO,
            "InitialRadialDistortion must have NumRadialDistortionCoefficients elements",
        ));
    }
    let wn = num(e, WHO, "worldPoints", world_points)?;
    if wn.dims.len() != 2 || wn.dims[1] != 2 {
        let msg = if wn.dims.len() == 2 && wn.dims[1] == 3 {
            "M-by-3 (non-planar) worldPoints are not supported yet; pass the M-by-2 points of a planar pattern"
        } else {
            "worldPoints must be an M-by-2 matrix"
        };
        return Err(err(WHO, msg));
    }
    let m = wn.dims[0];
    if m < 4 {
        return Err(err(WHO, "at least 4 world points are needed"));
    }
    if m > MAX_POINTS {
        return Err(err(WHO, "too many world points"));
    }
    if wn.data.iter().any(|x| !x.is_finite()) {
        return Err(err(WHO, "worldPoints must be finite"));
    }
    let world: Vec<[f64; 2]> = (0..m).map(|i| [wn.data[i], wn.data[m + i]]).collect();

    let im = num(e, WHO, "imagePoints", image_points)?;
    let views = match im.dims.as_slice() {
        [r, 2] if *r == m => 1,
        [r, 2, p] if *r == m => *p,
        _ => {
            return Err(err(
                WHO,
                "imagePoints must be an M-by-2-by-P array with one row per world point",
            ));
        }
    };
    if views < 2 {
        return Err(err(
            WHO,
            "at least 2 views of the pattern (imagePoints M-by-2-by-P, P >= 2) are needed",
        ));
    }
    if views > MAX_VIEWS {
        return Err(err(
            WHO,
            &format!("at most {MAX_VIEWS} views are supported"),
        ));
    }
    if m.saturating_mul(views) > MAX_POINTS {
        return Err(err(WHO, "too many image points"));
    }
    // Partially detected patterns: a [NaN NaN] row is a missing point.
    let mut used = vec![false; views];
    let mut homographies = Vec::new();
    let mut observed: Vec<View> = Vec::new();
    for (k, page) in im.data.chunks_exact(2 * m).enumerate() {
        e.check_cancel()?;
        let mut view = View {
            world: Vec::new(),
            observed: Vec::new(),
            rows: Vec::new(),
        };
        for i in 0..m {
            let p = [page[i], page[m + i]];
            if p[0].is_nan() && p[1].is_nan() {
                continue;
            }
            if !(p[0].is_finite() && p[1].is_finite()) {
                return Err(err(
                    WHO,
                    "imagePoints must be finite, or [NaN NaN] for a point not detected",
                ));
            }
            view.world.push(world[i]);
            view.observed.push(p);
            view.rows.push(i);
        }
        // A view with fewer than 4 points or no homography is not used.
        if view.rows.len() < 4 {
            continue;
        }
        if let Some(h) = homography(&view.world, &view.observed) {
            used[k] = true;
            homographies.push(h);
            observed.push(view);
        }
    }
    if homographies.len() < 2 {
        return Err(err(
            WHO,
            "fewer than 2 views give a homography (at least 4 detected, non-collinear points per view are needed)",
        ));
    }
    if used.contains(&false) {
        let dropped: Vec<String> = (1..=views)
            .filter(|&k| !used[k - 1])
            .map(|k| k.to_string())
            .collect();
        let message = format!(
            "{WHO}: view(s) {} not used: fewer than 4 detected points or no homography (imagesUsed is false for them)",
            dropped.join(", ")
        );
        e.lang_utils_call("warning", &[Value::Text(message)], out)?;
    }

    let centre = {
        let n = observed.iter().map(|v| v.rows.len()).sum::<usize>() as f64;
        let mut c = [0., 0.];
        for p in observed.iter().flat_map(|v| v.observed.iter()) {
            c[0] += p[0] / n;
            c[1] += p[1] / n;
        }
        c
    };
    let degenerate = || {
        err(
            WHO,
            "the views are degenerate: the pattern poses are (nearly) parallel or too few; tilt the pattern differently between views",
        )
    };
    let mut starts = Vec::new();
    if let Some(k) = initial_k {
        starts.push(k);
    } else {
        match zhang_intrinsics(&homographies, centre, model.skew) {
            Zhang::Degenerate => return Err(degenerate()),
            Zhang::Unusable => {}
            Zhang::Found(k) => starts.push(k),
        }
    }
    if initial_k.is_none() && (starts.is_empty() || homographies.len() <= TWO_START_VIEWS) {
        // MATLAB pixels: the centre of an ncols-wide image is (ncols + 1) / 2.
        let principal = size.map_or(centre, |[rows, cols]| [(cols + 1.) / 2., (rows + 1.) / 2.]);
        starts.extend(fixed_principal_intrinsics(&homographies, principal));
    }
    let ni = model.intrinsic_count();
    let mut best: Option<(f64, Vec<f64>)> = None;
    for k0 in &starts {
        let mut x = vec![k0[0][0], k0[1][1], k0[0][2], k0[1][2]];
        if model.skew {
            x.push(k0[0][1]);
        }
        match &initial_radial {
            Some(r) => x.extend_from_slice(r),
            None => x.extend(std::iter::repeat_n(0., model.radial)),
        }
        x.extend(std::iter::repeat_n(0., 2 * usize::from(model.tangential)));
        let poses: Option<Vec<_>> = homographies
            .iter()
            .zip(&observed)
            .map(|(h, view)| extrinsics(k0, h, centroid(&view.world)))
            .collect();
        let Some(poses) = poses else {
            continue;
        };
        for (r, t) in &poses {
            x.extend_from_slice(r);
            x.extend_from_slice(t);
        }
        let f = levenberg_marquardt(e, &model, &observed, &mut x)?;
        let valid =
            f.is_finite() && x.iter().all(|v| v.is_finite()) && x[..2].iter().all(|v| *v > 0.);
        if valid && best.as_ref().is_none_or(|(b, _)| f < *b) {
            best = Some((f, x));
        }
    }
    let Some((_, x)) = best else {
        return Err(err(
            WHO,
            "the estimation did not converge: no usable start from the views (degenerate views, or points that do not fit a camera)",
        ));
    };
    let mut intr = model.intrinsics(&x);
    intr.size = size;
    let p = observed.len();
    let rotation_vectors: Vec<[f64; 3]> = (0..p)
        .map(|k| [x[ni + 6 * k], x[ni + 6 * k + 1], x[ni + 6 * k + 2]])
        .collect();
    let translation_vectors: Vec<[f64; 3]> = (0..p)
        .map(|k| [x[ni + 6 * k + 3], x[ni + 6 * k + 4], x[ni + 6 * k + 5]])
        .collect();
    if parallel_planes(&rotation_vectors) {
        return Err(degenerate());
    }
    // Missing points have NaN errors.
    let mut errors = vec![f64::NAN; m * 2 * p];
    for (k, view) in observed.iter().enumerate() {
        let r = projection::transpose(&projection::rodrigues(rotation_vectors[k]));
        let page = &mut errors[k * 2 * m..(k + 1) * 2 * m];
        for ((w, o), &i) in view.world.iter().zip(&view.observed).zip(&view.rows) {
            let (uv, _) =
                projection::project(&intr, &r, translation_vectors[k], [w[0], w[1], 0.], true);
            page[i] = uv[0] - o[0];
            page[m + i] = uv[1] - o[1];
        }
    }
    let estimation_errors = if nargout > 2 {
        e.check_cancel()?;
        Some(standard_errors(&model, &x, &observed))
    } else {
        None
    };
    let params = Params {
        intr,
        rotation_vectors,
        translation_vectors,
        world: world.iter().map(|w| [w[0], w[1], 0.]).collect(),
        world_cols: 2,
        units,
        estimate_skew: model.skew,
        num_radial: model.radial,
        estimate_tangential: model.tangential,
        errors,
        error_points: m,
    };
    let mut out = vec![super::new_object(e, Camera::Params(params))?];
    if nargout > 1 {
        out.push(Value::Logical(LogicalMatrix::from_column_vec(
            views,
            1,
            used.into_iter().map(u8::from).collect(),
            &e.cancel,
        )?));
    }
    if let Some(errors) = estimation_errors {
        out.push(super::new_object(e, Camera::CalibrationErrors(errors))?);
    }
    Ok(out)
}

/// Standard errors of the fitted parameters: sqrt(diag(s2 (J'J)^-1)) at the
/// solution, s2 = residual sum of squares / (2 Npoints - Nparameters).
/// Parameters not estimated (skew, tangential, k3 when off) get 0; NaN when
/// J'J is singular or there are no degrees of freedom left.
fn standard_errors(model: &Model, x: &[f64], views: &[View]) -> CalibrationErrors {
    let n = x.len();
    let points: usize = views.iter().map(|v| v.rows.len()).sum();
    let (jtj, _) = normal_equations(model, x, views);
    let dof = (2 * points).saturating_sub(n);
    let s2 = cost(model, x, views) / dof as f64;
    let std: Vec<f64> = match (dof > 0).then(|| inverse_diagonal(jtj, n)).flatten() {
        Some(d) => d.iter().map(|v| (s2 * v).sqrt()).collect(),
        None => vec![f64::NAN; n],
    };
    let mut i = 4;
    let skew = if model.skew {
        i += 1;
        std[4]
    } else {
        0.
    };
    let radial = std[i..i + model.radial].to_vec();
    i += model.radial;
    let tangential = if model.tangential {
        [std[i], std[i + 1]]
    } else {
        [0., 0.]
    };
    let ni = model.intrinsic_count();
    let view = |k: usize, o: usize| [0, 1, 2].map(|j| std[ni + 6 * k + o + j]);
    CalibrationErrors {
        intr: IntrinsicsErrors {
            skew,
            focal: [std[0], std[1]],
            principal: [std[2], std[3]],
            radial,
            tangential,
        },
        extr: ExtrinsicsErrors {
            rotation: (0..views.len()).map(|k| view(k, 0)).collect(),
            translation: (0..views.len()).map(|k| view(k, 3)).collect(),
        },
    }
}

/// Diagonal of the inverse of a symmetric positive definite matrix (row-major
/// n-by-n): with A = LL', diag(A^-1)_i = sum over k of (L^-1)_ki^2. None if
/// not positive definite.
fn inverse_diagonal(a: Vec<f64>, n: usize) -> Option<Vec<f64>> {
    let l = cholesky(a, n)?;
    let mut d = vec![0.; n];
    let mut col = vec![0.; n];
    for i in 0..n {
        // Column i of L^-1 by forward substitution (zero above row i).
        col.iter_mut().for_each(|v| *v = 0.);
        col[i] = 1. / l[i * n + i];
        d[i] += col[i] * col[i];
        for r in i + 1..n {
            let mut s = 0.;
            for k in i..r {
                s -= l[r * n + k] * col[k];
            }
            col[r] = s / l[r * n + r];
            d[i] += col[r] * col[r];
        }
    }
    Some(d)
}

/// `displayErrors(estimationErrors, cameraParams)`: the standard error table.
pub(super) fn display_errors(
    e: &Engine,
    errors: &CalibrationErrors,
    rest: &[Value],
) -> Result<String> {
    let who = "displayErrors";
    let [params] = rest else {
        return Err(err(
            who,
            "expected displayErrors(estimationErrors, cameraParams)",
        ));
    };
    let params = super::camera_of(e, params);
    let Some(Camera::Params(p)) = params.as_deref() else {
        return Err(err(who, "cameraParams must be a cameraParameters object"));
    };
    if p.patterns() != errors.extr.rotation.len() || p.intr.radial.len() != errors.intr.radial.len()
    {
        return Err(err(
            who,
            "cameraParams must be the cameraParameters estimated with these errors",
        ));
    }
    let entry = |v: f64, s: f64| format!("{v:>10.4} +/- {s:<8.4}");
    let line = |label: &str, values: &[f64], errs: &[f64]| {
        let entries: Vec<String> = values
            .iter()
            .zip(errs)
            .map(|(v, s)| entry(*v, *s))
            .collect();
        format!("{label:<25}[{}]\n", entries.join("  "))
    };
    let i = &errors.intr;
    let mut text = String::from(
        "\nStandard Errors of Estimated Camera Parameters\n----------------------------------------------\n\nIntrinsics\n----------\n",
    );
    text += &line("Focal length (pixels):", &p.intr.focal, &i.focal);
    text += &line("Principal point (pixels):", &p.intr.principal, &i.principal);
    if p.estimate_skew {
        text += &line("Skew (pixels):", &[p.intr.skew], &[i.skew]);
    }
    text += &line("Radial distortion:", &p.intr.radial, &i.radial);
    if p.estimate_tangential {
        text += &line("Tangential distortion:", &p.intr.tangential, &i.tangential);
    }
    text += "\nExtrinsics\n----------\nRotation vectors:\n";
    for (v, s) in p.rotation_vectors.iter().zip(&errors.extr.rotation) {
        text += &line("", v, s);
    }
    text += &format!("\nTranslation vectors ({}):\n", p.units);
    for (v, s) in p.translation_vectors.iter().zip(&errors.extr.translation) {
        text += &line("", v, s);
    }
    Ok(text)
}

fn centroid(p: &[[f64; 2]]) -> [f64; 2] {
    let n = p.len() as f64;
    p.iter()
        .fold([0., 0.], |[x, y], q| [x + q[0] / n, y + q[1] / n])
}

/// Hartley normalisation: centroid to the origin, mean distance sqrt(2).
fn normaliser(p: &[[f64; 2]]) -> Option<Mat3> {
    let [mx, my] = centroid(p);
    let n = p.len() as f64;
    let d = p.iter().map(|q| (q[0] - mx).hypot(q[1] - my)).sum::<f64>() / n;
    if !(d.is_finite() && d > 0.) {
        return None;
    }
    let s = std::f64::consts::SQRT_2 / d;
    Some([[s, 0., -s * mx], [0., s, -s * my], [0., 0., 1.]])
}

fn apply(t: &Mat3, p: [f64; 2]) -> [f64; 2] {
    [
        t[0][0] * p[0] + t[0][1] * p[1] + t[0][2],
        t[1][0] * p[0] + t[1][1] * p[1] + t[1][2],
    ]
}

fn mul(a: &Mat3, b: &Mat3) -> Mat3 {
    [0, 1, 2].map(|i| [0, 1, 2].map(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum::<f64>()))
}

/// Inverse of a similarity normaliser [s 0 a; 0 s b; 0 0 1].
fn unnormaliser(t: &Mat3) -> Mat3 {
    let s = t[0][0];
    [
        [1. / s, 0., -t[0][2] / s],
        [0., 1. / s, -t[1][2] / s],
        [0., 0., 1.],
    ]
}

/// Normalised DLT homography mapping world [x y 1] to image [u v 1].
fn homography(world: &[[f64; 2]], image: &[[f64; 2]]) -> Option<Mat3> {
    let tw = normaliser(world)?;
    let ti = normaliser(image)?;
    let mut ata = vec![0.; 81];
    for (w, p) in world.iter().zip(image) {
        let [x, y] = apply(&tw, *w);
        let [u, v] = apply(&ti, *p);
        let rows = [
            [x, y, 1., 0., 0., 0., -u * x, -u * y, -u],
            [0., 0., 0., x, y, 1., -v * x, -v * y, -v],
        ];
        for r in &rows {
            for i in 0..9 {
                for j in 0..9 {
                    ata[9 * i + j] += r[i] * r[j];
                }
            }
        }
    }
    let (values, vectors) = symmetric_eigen(ata, 9);
    // Rank 8 is required: a unique null vector.
    if !values[1].is_finite() || !values[8].is_finite() || values[1] <= 1e-10 * values[8] {
        return None;
    }
    let h = &vectors[0];
    if h.iter().any(|v| !v.is_finite()) {
        return None;
    }
    // A unique DLT solution can still collapse the plane to a line. Test
    // conditioning in Hartley coordinates to remove pixel and world units.
    let singular = nalgebra::Matrix3::from_row_slice(h)
        .try_svd(false, false, f64::EPSILON, 100)?
        .singular_values;
    // A condition number above 500 makes a view too close to a collapsed
    // plane to provide reliable calibration constraints.
    if singular.min() <= 2e-3 * singular.max() {
        return None;
    }
    let hn = [[h[0], h[1], h[2]], [h[3], h[4], h[5]], [h[6], h[7], h[8]]];
    let hm = mul(&mul(&unnormaliser(&ti), &hn), &tw);
    let scale = hm[2][2];
    if hm.iter().flatten().any(|v| !v.is_finite())
        || scale.abs() <= f64::EPSILON * hm.iter().flatten().fold(0f64, |a, v| a.max(v.abs()))
    {
        return None;
    }
    Some(hm.map(|r| r.map(|v| v / scale)))
}

/// Eigenvalues (ascending) and unit eigenvectors of a symmetric n-by-n matrix
/// (row-major) by cyclic Jacobi rotations.
fn symmetric_eigen(mut a: Vec<f64>, n: usize) -> (Vec<f64>, Vec<Vec<f64>>) {
    let mut v = vec![0.; n * n];
    for i in 0..n {
        v[i * n + i] = 1.;
    }
    for _ in 0..100 {
        let off: f64 = (0..n)
            .flat_map(|i| (0..n).filter(move |&j| j != i).map(move |j| (i, j)))
            .map(|(i, j)| a[i * n + j] * a[i * n + j])
            .sum();
        let total: f64 = a.iter().map(|x| x * x).sum();
        if off <= 1e-30 * total || off == 0. {
            break;
        }
        for p in 0..n {
            for q in p + 1..n {
                let apq = a[p * n + q];
                if apq == 0. {
                    continue;
                }
                let theta = (a[q * n + q] - a[p * n + p]) / (2. * apq);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.).sqrt());
                let t = if theta == 0. { 1. } else { t };
                let c = 1. / (t * t + 1.).sqrt();
                let s = t * c;
                for k in 0..n {
                    let (akp, akq) = (a[k * n + p], a[k * n + q]);
                    a[k * n + p] = c * akp - s * akq;
                    a[k * n + q] = s * akp + c * akq;
                }
                for k in 0..n {
                    let (apk, aqk) = (a[p * n + k], a[q * n + k]);
                    a[p * n + k] = c * apk - s * aqk;
                    a[q * n + k] = s * apk + c * aqk;
                }
                for k in 0..n {
                    let (vkp, vkq) = (v[k * n + p], v[k * n + q]);
                    v[k * n + p] = c * vkp - s * vkq;
                    v[k * n + q] = s * vkp + c * vkq;
                }
            }
        }
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&i, &j| a[i * n + i].total_cmp(&a[j * n + j]));
    let values = order.iter().map(|&i| a[i * n + i]).collect();
    let vectors = order
        .iter()
        .map(|&j| (0..n).map(|k| v[k * n + j]).collect())
        .collect();
    (values, vectors)
}

/// What the closed form gives.
enum Zhang {
    /// The constraints leave more than one solution: parallel pattern planes
    /// (or too few views for the unknowns).
    Degenerate,
    /// A unique solution that is not a camera (B not positive definite), as
    /// distorted or noisy points give with few views.
    Unusable,
    Found(Mat3),
}

/// Zhang's closed-form K from the homographies, on image coordinates
/// centred at `centre` and divided by 1000 (about a focal length) so B is
/// well conditioned.
fn zhang_intrinsics(hs: &[Mat3], centre: [f64; 2], skew: bool) -> Zhang {
    let scale = 1000.;
    let n = [
        [1. / scale, 0., -centre[0] / scale],
        [0., 1. / scale, -centre[1] / scale],
        [0., 0., 1.],
    ];
    let vrow = |h: &Mat3, i: usize, j: usize| -> [f64; 6] {
        let (a, b) = ([h[0][i], h[1][i], h[2][i]], [h[0][j], h[1][j], h[2][j]]);
        [
            a[0] * b[0],
            a[0] * b[1] + a[1] * b[0],
            a[1] * b[1],
            a[2] * b[0] + a[0] * b[2],
            a[2] * b[1] + a[1] * b[2],
            a[2] * b[2],
        ]
    };
    let mut vtv = vec![0.; 36];
    let mut add = |r: &[f64; 6]| {
        let norm = r.iter().map(|x| x * x).sum::<f64>().sqrt();
        if norm == 0. {
            return;
        }
        for i in 0..6 {
            for j in 0..6 {
                vtv[6 * i + j] += r[i] * r[j] / (norm * norm);
            }
        }
    };
    for h in hs {
        let h = mul(&n, h);
        // Columns scaled to unit size (the homography scale is free).
        let s = (0..3).map(|i| h[i][0] * h[i][0]).sum::<f64>().sqrt();
        let h = h.map(|r| r.map(|v| v / s));
        add(&vrow(&h, 0, 1));
        let (v11, v22) = (vrow(&h, 0, 0), vrow(&h, 1, 1));
        add(&[0, 1, 2, 3, 4, 5].map(|i| v11[i] - v22[i]));
    }
    if !skew {
        add(&[0., 1., 0., 0., 0., 0.]);
    }
    let (values, vectors) = symmetric_eigen(vtv, 6);
    // Exactly one null direction: the next eigenvalue must stand clear.
    if values[1].is_nan() || values[1] <= 1e-9 * values[5] {
        return Zhang::Degenerate;
    }
    let mut b = vectors[0].clone();
    if b[0] < 0. {
        b.iter_mut().for_each(|x| *x = -*x);
    }
    let [b11, b12, b22, b13, b23, b33] = [b[0], b[1], b[2], b[3], b[4], b[5]];
    let d = b11 * b22 - b12 * b12;
    if !(b11 > 0. && d > 0.) {
        return Zhang::Unusable;
    }
    let v0 = (b12 * b13 - b11 * b23) / d;
    let lambda = b33 - (b13 * b13 + v0 * (b12 * b13 - b11 * b23)) / b11;
    if lambda.is_nan() || lambda <= 0. {
        return Zhang::Unusable;
    }
    let alpha = (lambda / b11).sqrt();
    let beta = (lambda * b11 / d).sqrt();
    let gamma = if skew {
        -b12 * alpha * alpha * beta / lambda
    } else {
        0.
    };
    let u0 = gamma * v0 / beta - b13 * alpha * alpha / lambda;
    let kn = [[alpha, gamma, u0], [0., beta, v0], [0., 0., 1.]];
    let k = mul(&unnormaliser(&n), &kn);
    if k.iter().flatten().all(|v| v.is_finite()) {
        Zhang::Found(k)
    } else {
        Zhang::Unusable
    }
}

/// K with the principal point held at `principal` and no skew: each
/// homography's two orthogonality constraints (h1'Bh2 = 0, h1'Bh1 = h2'Bh2)
/// are then linear in 1/fx^2 and 1/fy^2, solved by least squares over the
/// views (the start of OpenCV's calibrateCamera). One focal length for both
/// axes when the two are not determined or not positive.
fn fixed_principal_intrinsics(hs: &[Mat3], principal: [f64; 2]) -> Option<Mat3> {
    let scale = 1000.;
    let norm = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    // Normal equations of the rows [a b] [1/fx^2; 1/fy^2] = c.
    let (mut aa, mut ab, mut bb, mut ac, mut bc) = (0., 0., 0., 0., 0.);
    for h in hs {
        let col = |j: usize| {
            [
                (h[0][j] - principal[0] * h[2][j]) / scale,
                (h[1][j] - principal[1] * h[2][j]) / scale,
                h[2][j],
            ]
        };
        let (h1, h2) = (col(0), col(1));
        let sum = [0, 1, 2].map(|i| h1[i] + h2[i]);
        let difference = [0, 1, 2].map(|i| h1[i] - h2[i]);
        for (u, v) in [(h1, h2), (sum, difference)] {
            let s = norm(u) * norm(v);
            if s.is_nan() || s <= 0. {
                continue;
            }
            let (a, b, c) = (u[0] * v[0] / s, u[1] * v[1] / s, -u[2] * v[2] / s);
            aa += a * a;
            ab += a * b;
            bb += b * b;
            ac += a * c;
            bc += b * c;
        }
    }
    let det = aa * bb - ab * ab;
    let mut inverse = [(bb * ac - ab * bc) / det, (aa * bc - ab * ac) / det];
    let usable = |v: f64| v.is_finite() && v > 0.;
    if !(det > 1e-12 * aa * bb && usable(inverse[0]) && usable(inverse[1])) {
        let one = (ac + bc) / (aa + 2. * ab + bb);
        if !usable(one) {
            return None;
        }
        inverse = [one, one];
    }
    let (fx, fy) = (scale / inverse[0].sqrt(), scale / inverse[1].sqrt());
    (fx.is_finite() && fy.is_finite()).then_some([
        [fx, 0., principal[0]],
        [0., fy, principal[1]],
        [0., 0., 1.],
    ])
}

/// True when the pattern planes of all views agree within about one degree
/// (the normal is the third column of the premultiply rotation matrix).
fn parallel_planes(rotation_vectors: &[[f64; 3]]) -> bool {
    let normals: Vec<[f64; 3]> = rotation_vectors
        .iter()
        .map(|v| {
            let r = projection::rodrigues(*v);
            [r[0][2], r[1][2], r[2][2]]
        })
        .collect();
    let limit = 1f64.to_radians().cos();
    normals.iter().enumerate().all(|(i, a)| {
        normals[i + 1..]
            .iter()
            .all(|b| a[0] * b[0] + a[1] * b[1] + a[2] * b[2] > limit)
    })
}

/// Rotation vector (premultiply, OpenCV form) and translation of one view;
/// `centre` is the centroid of the world points observed in this view.
fn extrinsics(k: &Mat3, h: &Mat3, centre: [f64; 2]) -> Option<([f64; 3], [f64; 3])> {
    // K^-1 for upper-triangular K.
    let (fx, s, cx, fy, cy) = (k[0][0], k[0][1], k[0][2], k[1][1], k[1][2]);
    let kinv = [
        [1. / fx, -s / (fx * fy), (s * cy - cx * fy) / (fx * fy)],
        [0., 1. / fy, -cy / fy],
        [0., 0., 1.],
    ];
    let a = mul(&kinv, h);
    let col = |j: usize| [a[0][j], a[1][j], a[2][j]];
    let norm = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let mut lambda = 2. / (norm(col(0)) + norm(col(1)));
    // The sign that puts the pattern in front of the camera: positive depth
    // at its centroid (the world origin may lie far off the pattern, behind
    // the camera plane).
    if (a[2][0] * centre[0] + a[2][1] * centre[1] + a[2][2]) * lambda < 0. {
        lambda = -lambda;
    }
    let r1 = col(0).map(|v| v * lambda);
    let r2 = col(1).map(|v| v * lambda);
    let r3 = [
        r1[1] * r2[2] - r1[2] * r2[1],
        r1[2] * r2[0] - r1[0] * r2[2],
        r1[0] * r2[1] - r1[1] * r2[0],
    ];
    let t = col(2).map(|v| v * lambda);
    let rp = [
        [r1[0], r2[0], r3[0]],
        [r1[1], r2[1], r3[1]],
        [r1[2], r2[2], r3[2]],
    ];
    let r = projection::rotation_vector(&rp)?;
    Some((r, t))
}

/// Residuals (reprojected - observed, u then v per point) of one view.
fn view_residuals(intr: &Intrinsics, pose: &[f64], view: &View, out: &mut [f64]) {
    let r = projection::transpose(&projection::rodrigues([pose[0], pose[1], pose[2]]));
    let t = [pose[3], pose[4], pose[5]];
    for (i, (w, o)) in view.world.iter().zip(&view.observed).enumerate() {
        let (uv, _) = projection::project(intr, &r, t, [w[0], w[1], 0.], true);
        out[2 * i] = uv[0] - o[0];
        out[2 * i + 1] = uv[1] - o[1];
    }
}

fn cost(model: &Model, x: &[f64], views: &[View]) -> f64 {
    let ni = model.intrinsic_count();
    let intr = model.intrinsics(&x[..ni]);
    let mut sum = 0.;
    for (k, view) in views.iter().enumerate() {
        let mut r = vec![0.; 2 * view.rows.len()];
        view_residuals(&intr, &x[ni + 6 * k..ni + 6 * k + 6], view, &mut r);
        sum += r.iter().map(|v| v * v).sum::<f64>();
    }
    sum
}

/// Central-difference step for parameter value v.
fn step(v: f64) -> f64 {
    1e-6 * v.abs().max(1e-2)
}

/// Normal equations J'J and J'r, central-difference Jacobian per view.
fn normal_equations(model: &Model, x: &[f64], views: &[View]) -> (Vec<f64>, Vec<f64>) {
    let ni = model.intrinsic_count();
    let n = x.len();
    let local = ni + 6;
    let mut jtj = vec![0.; n * n];
    let mut jtr = vec![0.; n];
    for (k, view) in views.iter().enumerate() {
        let rows = 2 * view.rows.len();
        let mut r = vec![0.; rows];
        let mut plus = vec![0.; rows];
        let mut minus = vec![0.; rows];
        let mut jac = vec![0.; local * rows];
        let mut lx: Vec<f64> = x[..ni].to_vec();
        lx.extend_from_slice(&x[ni + 6 * k..ni + 6 * k + 6]);
        let eval = |lx: &[f64], out: &mut [f64]| {
            view_residuals(&model.intrinsics(&lx[..ni]), &lx[ni..], view, out);
        };
        eval(&lx, &mut r);
        for j in 0..local {
            let v = lx[j];
            let h = step(v);
            lx[j] = v + h;
            eval(&lx, &mut plus);
            lx[j] = v - h;
            eval(&lx, &mut minus);
            lx[j] = v;
            for i in 0..rows {
                jac[j * rows + i] = (plus[i] - minus[i]) / (2. * h);
            }
        }
        let global = |j: usize| if j < ni { j } else { ni + 6 * k + (j - ni) };
        for a in 0..local {
            let ja = &jac[a * rows..(a + 1) * rows];
            let ga = global(a);
            jtr[ga] += ja.iter().zip(&r).map(|(p, q)| p * q).sum::<f64>();
            for b in 0..=a {
                let jb = &jac[b * rows..(b + 1) * rows];
                let s = ja.iter().zip(jb).map(|(p, q)| p * q).sum::<f64>();
                let gb = global(b);
                jtj[ga * n + gb] += s;
                if ga != gb {
                    jtj[gb * n + ga] += s;
                }
            }
        }
    }
    (jtj, jtr)
}

/// Solve the symmetric positive definite system by Cholesky; None if not
/// positive definite.
fn cholesky_solve(a: Vec<f64>, b: &[f64]) -> Option<Vec<f64>> {
    let n = b.len();
    let a = cholesky(a, n)?;
    let mut y = b.to_vec();
    for i in 0..n {
        for k in 0..i {
            y[i] -= a[i * n + k] * y[k];
        }
        y[i] /= a[i * n + i];
    }
    for i in (0..n).rev() {
        for k in i + 1..n {
            y[i] -= a[k * n + i] * y[k];
        }
        y[i] /= a[i * n + i];
    }
    Some(y)
}

/// Cholesky factor L (lower triangle of the returned row-major matrix) of a
/// symmetric positive definite n-by-n matrix; None if not positive definite.
fn cholesky(mut a: Vec<f64>, n: usize) -> Option<Vec<f64>> {
    for j in 0..n {
        let mut d = a[j * n + j];
        for k in 0..j {
            d -= a[j * n + k] * a[j * n + k];
        }
        if d.is_nan() || d <= 0. {
            return None;
        }
        let d = d.sqrt();
        a[j * n + j] = d;
        for i in j + 1..n {
            let mut s = a[i * n + j];
            for k in 0..j {
                s -= a[i * n + k] * a[j * n + k];
            }
            a[i * n + j] = s / d;
        }
    }
    Some(a)
}

/// Minimise the squared reprojection error from `x` in place; the final sum
/// of squares.
fn levenberg_marquardt(e: &Engine, model: &Model, views: &[View], x: &mut [f64]) -> Result<f64> {
    let n = x.len();
    let mut f = cost(model, x, views);
    let mut mu = 1e-3;
    for _ in 0..MAX_ITERATIONS {
        e.check_cancel()?;
        let (jtj, jtr) = normal_equations(model, x, views);
        if jtr.iter().all(|g| g.abs() <= 1e-300) {
            break;
        }
        let mut improved = false;
        let mut converged = false;
        for _ in 0..30 {
            let mut a = jtj.clone();
            for i in 0..n {
                a[i * n + i] += mu * jtj[i * n + i].max(1e-300);
            }
            let neg: Vec<f64> = jtr.iter().map(|g| -g).collect();
            let Some(dx) = cholesky_solve(a, &neg) else {
                mu *= 10.;
                continue;
            };
            let trial: Vec<f64> = x.iter().zip(&dx).map(|(a, b)| a + b).collect();
            let ft = cost(model, &trial, views);
            let small = dx
                .iter()
                .zip(x.iter())
                .all(|(d, v)| d.abs() <= 1e-12 * (v.abs() + 1e-6));
            if ft.is_finite() && ft <= f {
                let gain = f - ft;
                x.copy_from_slice(&trial);
                converged = small || gain <= 1e-16 * f;
                f = ft;
                mu = (mu / 3.).max(1e-12);
                improved = true;
                break;
            }
            if small {
                converged = true;
                break;
            }
            mu *= 10.;
        }
        if converged || !improved {
            break;
        }
    }
    Ok(f)
}
