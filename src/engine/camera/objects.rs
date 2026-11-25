//! cameraIntrinsics and cameraParameters values: construction from
//! evaluated arguments and their property values.
use super::projection::{self, Mat3};
use super::{Num, err, logical_arg, num, text_arg};
use crate::engine::{Engine, Value};
use crate::{Matrix, Result};

/// Most world points, patterns or error entries one object holds.
pub(super) const MAX_POINTS: usize = 1_000_000;

/// Camera intrinsics (a cameraIntrinsics object, and the intrinsic part of
/// cameraParameters). `size` is None for cameraParameters without ImageSize.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::engine) struct Intrinsics {
    pub(in crate::engine) focal: [f64; 2],
    pub(in crate::engine) principal: [f64; 2],
    pub(in crate::engine) size: Option<[f64; 2]>,
    /// [k1 k2] or [k1 k2 k3].
    pub(in crate::engine) radial: Vec<f64>,
    pub(in crate::engine) tangential: [f64; 2],
    pub(in crate::engine) skew: f64,
}

impl Intrinsics {
    pub(in crate::engine) const PROPERTIES: [&'static str; 7] = [
        "FocalLength",
        "PrincipalPoint",
        "ImageSize",
        "RadialDistortion",
        "TangentialDistortion",
        "Skew",
        "K",
    ];
    /// K = [fx s cx; 0 fy cy; 0 0 1] as rows.
    pub(in crate::engine) fn k(&self) -> Mat3 {
        [
            [self.focal[0], self.skew, self.principal[0]],
            [0., self.focal[1], self.principal[1]],
            [0., 0., 1.],
        ]
    }
    /// Focal lengths, principal point and skew from the argument `name`:
    /// "K", or "IntrinsicMatrix" (= K.'); messages name it and its own form.
    pub(super) fn from_k(who: &str, name: &str, k: &Num) -> Result<([f64; 2], [f64; 2], f64)> {
        let transposed = name.ends_with("IntrinsicMatrix");
        let m = mat3(who, name, k)?;
        let m = if transposed {
            projection::transpose(&m)
        } else {
            m
        };
        if m[1][0] != 0. || m[2][0] != 0. || m[2][1] != 0. || m[2][2] != 1. {
            return Err(err(
                who,
                if transposed {
                    "IntrinsicMatrix must have the form [fx 0 0; s fy 0; cx cy 1] (the transpose of K)"
                } else {
                    "the intrinsic matrix K must have the form [fx s cx; 0 fy cy; 0 0 1]"
                },
            ));
        }
        let focal = [m[0][0], m[1][1]];
        if focal.iter().any(|f| !(f.is_finite() && *f > 0.)) {
            return Err(err(
                who,
                &format!("the focal lengths {name}(1,1) and {name}(2,2) must be positive"),
            ));
        }
        if !(m[0][2].is_finite() && m[1][2].is_finite() && m[0][1].is_finite()) {
            return Err(err(who, &format!("{name} must be finite")));
        }
        Ok((focal, [m[0][2], m[1][2]], m[0][1]))
    }
    pub(in crate::engine) fn property(&self, name: &str) -> Option<Result<Value>> {
        Some(match name {
            "FocalLength" => row(&self.focal),
            "PrincipalPoint" => row(&self.principal),
            "ImageSize" => match self.size {
                Some(s) => row(&s),
                None => Ok(super::empty()),
            },
            "RadialDistortion" => row(&self.radial),
            "TangentialDistortion" => row(&self.tangential),
            "Skew" => Ok(Value::Matrix(Matrix::scalar(self.skew))),
            "K" => mat3_value(&self.k()),
            "IntrinsicMatrix" => mat3_value(&projection::transpose(&self.k())),
            _ => return None,
        })
    }
}

/// A cameraParameters object.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::engine) struct Params {
    pub(in crate::engine) intr: Intrinsics,
    pub(in crate::engine) rotation_vectors: Vec<[f64; 3]>,
    pub(in crate::engine) translation_vectors: Vec<[f64; 3]>,
    /// M-by-2 (planar, z = 0) or M-by-3 world points; `world_cols` is 2 or 3.
    pub(in crate::engine) world: Vec<[f64; 3]>,
    pub(in crate::engine) world_cols: usize,
    pub(in crate::engine) units: String,
    pub(in crate::engine) estimate_skew: bool,
    pub(in crate::engine) num_radial: usize,
    pub(in crate::engine) estimate_tangential: bool,
    /// M-by-2-by-P column-major, or empty.
    pub(in crate::engine) errors: Vec<f64>,
    pub(in crate::engine) error_points: usize,
}

impl Params {
    pub(in crate::engine) const PROPERTIES: [&'static str; 20] = [
        "Intrinsics",
        "FocalLength",
        "PrincipalPoint",
        "Skew",
        "K",
        "RadialDistortion",
        "TangentialDistortion",
        "ImageSize",
        "RotationVectors",
        "TranslationVectors",
        "RotationMatrices",
        "NumPatterns",
        "WorldPoints",
        "WorldUnits",
        "EstimateSkew",
        "NumRadialDistortionCoefficients",
        "EstimateTangentialDistortion",
        "MeanReprojectionError",
        "ReprojectionErrors",
        "ReprojectedPoints",
    ];
    pub(in crate::engine) fn patterns(&self) -> usize {
        self.rotation_vectors.len()
    }
    /// Postmultiply rotation matrix of pattern i (rotationVectorToMatrix).
    pub(in crate::engine) fn rotation(&self, i: usize) -> Mat3 {
        projection::transpose(&projection::rodrigues(self.rotation_vectors[i]))
    }
    pub(in crate::engine) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + 8 * (3 * (self.rotation_vectors.len() * 2 + self.world.len()) + self.errors.len())
            + 64
    }
    fn mean_error(&self) -> f64 {
        let m = self.error_points;
        if self.errors.is_empty() || m == 0 {
            return f64::NAN;
        }
        // Missing points of partially detected patterns are NaN: left out.
        let pages = self.errors.len() / (2 * m);
        let (mut sum, mut count) = (0., 0usize);
        for p in 0..pages {
            let page = &self.errors[p * 2 * m..(p + 1) * 2 * m];
            for i in 0..m {
                let d = page[i].hypot(page[m + i]);
                if !d.is_nan() {
                    sum += d;
                    count += 1;
                }
            }
        }
        if count == 0 {
            f64::NAN
        } else {
            sum / count as f64
        }
    }
    /// World points reprojected with each pattern's extrinsics and the lens
    /// distortion: M-by-2-by-P column-major (empty without points or patterns).
    fn reprojected(&self) -> Vec<f64> {
        let (m, p) = (self.world.len(), self.patterns());
        let mut out = vec![0.; m * 2 * p];
        for k in 0..p {
            let r = self.rotation(k);
            let page = &mut out[k * 2 * m..(k + 1) * 2 * m];
            for (i, w) in self.world.iter().enumerate() {
                let (uv, _) =
                    projection::project(&self.intr, &r, self.translation_vectors[k], *w, true);
                page[i] = uv[0];
                page[m + i] = uv[1];
            }
        }
        out
    }
    pub(in crate::engine) fn property(&self, name: &str) -> Option<Result<Value>> {
        let p = self.patterns();
        Some(match name {
            // The object (with an ImageSize) is made by camera::member.
            "Intrinsics" if self.intr.size.is_none() => Ok(super::empty()),
            "FocalLength"
            | "PrincipalPoint"
            | "Skew"
            | "K"
            | "IntrinsicMatrix"
            | "RadialDistortion"
            | "TangentialDistortion"
            | "ImageSize" => {
                return self.intr.property(name);
            }
            "RotationVectors" => rows3(&self.rotation_vectors),
            "TranslationVectors" => rows3(&self.translation_vectors),
            "RotationMatrices" => {
                if p == 0 {
                    return Some(Ok(super::empty()));
                }
                let mut data = Vec::with_capacity(9 * p);
                for k in 0..p {
                    let r = self.rotation(k);
                    data.extend(projection::transpose(&r).into_iter().flatten());
                }
                array3(3, 3, p, data)
            }
            "NumPatterns" => Ok(Value::Matrix(Matrix::scalar(p as f64))),
            "WorldPoints" => {
                if self.world.is_empty() {
                    Ok(super::empty())
                } else {
                    let c = self.world_cols;
                    let m = self.world.len();
                    let data = (0..c).flat_map(|j| self.world.iter().map(move |w| w[j]));
                    Matrix::from_column_vec(m, c, data.collect()).map(Value::Matrix)
                }
            }
            "WorldUnits" => Ok(Value::Text(self.units.clone())),
            "EstimateSkew" => Ok(super::logical(self.estimate_skew)),
            "NumRadialDistortionCoefficients" => {
                Ok(Value::Matrix(Matrix::scalar(self.num_radial as f64)))
            }
            "EstimateTangentialDistortion" => Ok(super::logical(self.estimate_tangential)),
            "MeanReprojectionError" => Ok(Value::Matrix(Matrix::scalar(self.mean_error()))),
            "ReprojectionErrors" => {
                if self.errors.is_empty() {
                    Ok(super::empty())
                } else {
                    let m = self.error_points;
                    array3(m, 2, self.errors.len() / (2 * m), self.errors.clone())
                }
            }
            "ReprojectedPoints" => {
                if self.world.is_empty() || p == 0 {
                    Ok(super::empty())
                } else {
                    array3(self.world.len(), 2, p, self.reprojected())
                }
            }
            _ => return None,
        })
    }
}

/// Standard errors of estimated intrinsics (an intrinsicsEstimationErrors
/// object); 0 for parameters that were not estimated.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::engine) struct IntrinsicsErrors {
    pub(in crate::engine) skew: f64,
    pub(in crate::engine) focal: [f64; 2],
    pub(in crate::engine) principal: [f64; 2],
    pub(in crate::engine) radial: Vec<f64>,
    pub(in crate::engine) tangential: [f64; 2],
}

impl IntrinsicsErrors {
    pub(in crate::engine) const PROPERTIES: [&'static str; 5] = [
        "SkewError",
        "FocalLengthError",
        "PrincipalPointError",
        "RadialDistortionError",
        "TangentialDistortionError",
    ];
    pub(in crate::engine) fn property(&self, name: &str) -> Option<Result<Value>> {
        Some(match name {
            "SkewError" => Ok(Value::Matrix(Matrix::scalar(self.skew))),
            "FocalLengthError" => row(&self.focal),
            "PrincipalPointError" => row(&self.principal),
            "RadialDistortionError" => row(&self.radial),
            "TangentialDistortionError" => row(&self.tangential),
            _ => return None,
        })
    }
}

/// Standard errors of estimated extrinsics, one row per pattern (an
/// extrinsicsEstimationErrors object).
#[derive(Clone, Debug, PartialEq)]
pub(in crate::engine) struct ExtrinsicsErrors {
    pub(in crate::engine) rotation: Vec<[f64; 3]>,
    pub(in crate::engine) translation: Vec<[f64; 3]>,
}

impl ExtrinsicsErrors {
    pub(in crate::engine) const PROPERTIES: [&'static str; 2] =
        ["RotationVectorsError", "TranslationVectorsError"];
    pub(in crate::engine) fn property(&self, name: &str) -> Option<Result<Value>> {
        Some(match name {
            "RotationVectorsError" => rows3(&self.rotation),
            "TranslationVectorsError" => rows3(&self.translation),
            _ => return None,
        })
    }
    pub(in crate::engine) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>() + 48 * self.rotation.len() + 64
    }
}

/// A cameraCalibrationErrors object: the third output of
/// estimateCameraParameters. Both properties are objects, made by
/// camera::member.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::engine) struct CalibrationErrors {
    pub(in crate::engine) intr: IntrinsicsErrors,
    pub(in crate::engine) extr: ExtrinsicsErrors,
}

impl CalibrationErrors {
    pub(in crate::engine) const PROPERTIES: [&'static str; 2] =
        ["IntrinsicsErrors", "ExtrinsicsErrors"];
}

fn row(v: &[f64]) -> Result<Value> {
    Matrix::from_column_vec(1, v.len(), v.to_vec()).map(Value::Matrix)
}

fn rows3(v: &[[f64; 3]]) -> Result<Value> {
    if v.is_empty() {
        return Ok(super::empty());
    }
    let data = (0..3).flat_map(|j| v.iter().map(move |r| r[j])).collect();
    Matrix::from_column_vec(v.len(), 3, data).map(Value::Matrix)
}

pub(super) fn mat3_value(m: &Mat3) -> Result<Value> {
    let data = (0..3).flat_map(|j| (0..3).map(move |i| m[i][j])).collect();
    Matrix::from_column_vec(3, 3, data).map(Value::Matrix)
}

fn array3(rows: usize, cols: usize, pages: usize, data: Vec<f64>) -> Result<Value> {
    let m = Matrix::from_column_vec(rows, cols * pages, data)?;
    Ok(Value::Matrix(if pages > 1 {
        m.with_dims(&[rows, cols, pages])?
    } else {
        m
    }))
}

pub(super) fn mat3(who: &str, what: &str, n: &Num) -> Result<Mat3> {
    if n.dims != [3, 3] {
        return Err(err(who, &format!("{what} must be a 3-by-3 matrix")));
    }
    Ok([0, 1, 2].map(|i| [0, 1, 2].map(|j| n.data[i + 3 * j])))
}

fn finite(who: &str, what: &str, n: &Num) -> Result<()> {
    if n.data.iter().all(|x| x.is_finite()) {
        Ok(())
    } else {
        Err(err(who, &format!("{what} must be finite")))
    }
}

fn vector(who: &str, what: &str, n: &Num, lens: &[usize]) -> Result<Vec<f64>> {
    let is_vector = n.dims.len() == 2 && (n.dims[0] == 1 || n.dims[1] == 1);
    if !is_vector || !lens.contains(&n.data.len()) {
        let lens = lens
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(" or ");
        return Err(err(who, &format!("{what} must be a {lens}-element vector")));
    }
    finite(who, what, n)?;
    Ok(n.data.clone())
}

fn vec2(who: &str, what: &str, n: &Num) -> Result<[f64; 2]> {
    let v = vector(who, what, n, &[2])?;
    Ok([v[0], v[1]])
}

fn positive2(who: &str, what: &str, n: &Num) -> Result<[f64; 2]> {
    let v = vec2(who, what, n)?;
    if v.iter().any(|x| *x <= 0.) {
        return Err(err(who, &format!("{what} must be positive")));
    }
    Ok(v)
}

fn image_size(who: &str, n: &Num) -> Result<[f64; 2]> {
    let v = positive2(who, "ImageSize", n)?;
    if v.iter().any(|x| x.fract() != 0. || *x > 1e9) {
        return Err(err(
            who,
            "ImageSize must be [mrows ncols] with positive integers",
        ));
    }
    Ok(v)
}

fn radial(who: &str, n: &Num) -> Result<Vec<f64>> {
    if n.data.len() == 6 {
        return Err(err(
            who,
            "a 6-element RadialDistortion (rational model) is not supported yet",
        ));
    }
    vector(who, "RadialDistortion", n, &[2, 3])
}

fn scalar(who: &str, what: &str, n: &Num) -> Result<f64> {
    if n.data.len() != 1 {
        return Err(err(who, &format!("{what} must be a numeric scalar")));
    }
    finite(who, what, n)?;
    Ok(n.data[0])
}

/// Name-value pairs, names matched case-insensitively against `allowed`.
pub(super) fn pairs<'a>(
    e: &Engine,
    who: &str,
    args: &'a [Value],
    allowed: &[&'static str],
) -> Result<Vec<(&'static str, &'a Value)>> {
    if !args.len().is_multiple_of(2) {
        return Err(err(who, "name-value arguments must come in pairs"));
    }
    args.chunks(2)
        .map(|p| {
            let name =
                text_arg(e, &p[0]).ok_or_else(|| err(who, "expected a property name as text"))?;
            let canonical = allowed
                .iter()
                .find(|a| a.eq_ignore_ascii_case(&name))
                .ok_or_else(|| err(who, &format!("unknown name-value argument '{name}'")))?;
            Ok((*canonical, &p[1]))
        })
        .collect()
}

/// cameraIntrinsics(focalLength, principalPoint, imageSize, ...) and
/// cameraIntrinsics(K, imageSize, ...).
pub(super) fn construct_intrinsics(e: &Engine, args: &[Value]) -> Result<Intrinsics> {
    let who = "cameraIntrinsics";
    let usage = || {
        err(
            who,
            "expected cameraIntrinsics(focalLength, principalPoint, imageSize, Name=Value) or cameraIntrinsics(K, imageSize, Name=Value)",
        )
    };
    let positional = args
        .iter()
        .position(|a| text_arg(e, a).is_some())
        .unwrap_or(args.len());
    let (focal, principal, mut skew, size) = match &args[..positional] {
        [k, s] => {
            let k = num(e, who, "K", k)?;
            if k.dims != [3, 3] {
                return Err(usage());
            }
            let (f, c, sk) = Intrinsics::from_k(who, "K", &k)?;
            (f, c, sk, image_size(who, &num(e, who, "imageSize", s)?)?)
        }
        [f, c, s] => (
            positive2(who, "focalLength", &num(e, who, "focalLength", f)?)?,
            vec2(who, "principalPoint", &num(e, who, "principalPoint", c)?)?,
            0.,
            image_size(who, &num(e, who, "imageSize", s)?)?,
        ),
        _ => return Err(usage()),
    };
    let mut intr = Intrinsics {
        focal,
        principal,
        size: Some(size),
        radial: vec![0., 0.],
        tangential: [0., 0.],
        skew: 0.,
    };
    let allowed = ["RadialDistortion", "TangentialDistortion", "Skew"];
    for (name, v) in pairs(e, who, &args[positional..], &allowed)? {
        let n = num(e, who, name, v)?;
        match name {
            "RadialDistortion" => intr.radial = radial(who, &n)?,
            "TangentialDistortion" => intr.tangential = vec2(who, name, &n)?,
            _ => skew = scalar(who, name, &n)?,
        }
    }
    intr.skew = skew;
    Ok(intr)
}

fn rows_of(who: &str, what: &str, n: &Num, cols: &[usize]) -> Result<Vec<[f64; 3]>> {
    if n.data.is_empty() {
        return Ok(Vec::new());
    }
    if n.dims.len() != 2 || !cols.contains(&n.dims[1]) {
        let c = cols
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(" or ");
        return Err(err(who, &format!("{what} must be an M-by-{c} matrix")));
    }
    if n.dims[0] > MAX_POINTS {
        return Err(err(
            who,
            &format!("{what} may have at most {MAX_POINTS} rows"),
        ));
    }
    finite(who, what, n)?;
    let (m, c) = (n.dims[0], n.dims[1]);
    Ok((0..m)
        .map(|i| [0, 1, 2].map(|j| if j < c { n.data[i + m * j] } else { 0. }))
        .collect())
}

/// cameraParameters(Name=Value, ...).
pub(super) fn construct_params(e: &Engine, args: &[Value]) -> Result<Params> {
    let who = "cameraParameters";
    let allowed = [
        "K",
        "IntrinsicMatrix",
        "RadialDistortion",
        "TangentialDistortion",
        "ImageSize",
        "RotationVectors",
        "TranslationVectors",
        "WorldPoints",
        "WorldUnits",
        "EstimateSkew",
        "NumRadialDistortionCoefficients",
        "EstimateTangentialDistortion",
        "ReprojectionErrors",
    ];
    let mut p = Params {
        intr: Intrinsics {
            focal: [1., 1.],
            principal: [0., 0.],
            size: None,
            radial: vec![0., 0.],
            tangential: [0., 0.],
            skew: 0.,
        },
        rotation_vectors: Vec::new(),
        translation_vectors: Vec::new(),
        world: Vec::new(),
        world_cols: 2,
        units: "mm".into(),
        estimate_skew: false,
        num_radial: 2,
        estimate_tangential: false,
        errors: Vec::new(),
        error_points: 0,
    };
    let mut radial_given = false;
    let mut num_radial_given = None;
    let mut matrix_given = None;
    let mut errors_dims = None;
    for (name, v) in pairs(e, who, args, &allowed)? {
        match name {
            "K" | "IntrinsicMatrix" => {
                if matrix_given.is_some_and(|m| m != name) {
                    return Err(err(who, "specify K or IntrinsicMatrix, not both"));
                }
                matrix_given = Some(name);
                let (f, c, s) = Intrinsics::from_k(who, name, &num(e, who, name, v)?)?;
                (p.intr.focal, p.intr.principal, p.intr.skew) = (f, c, s);
            }
            "RadialDistortion" => {
                p.intr.radial = radial(who, &num(e, who, name, v)?)?;
                radial_given = true;
            }
            "TangentialDistortion" => {
                p.intr.tangential = vec2(who, name, &num(e, who, name, v)?)?;
            }
            "ImageSize" => {
                let n = num(e, who, name, v)?;
                p.intr.size = if n.data.is_empty() {
                    None
                } else {
                    Some(image_size(who, &n)?)
                };
            }
            "RotationVectors" => {
                p.rotation_vectors = rows_of(who, name, &num(e, who, name, v)?, &[3])?
            }
            "TranslationVectors" => {
                p.translation_vectors = rows_of(who, name, &num(e, who, name, v)?, &[3])?
            }
            "WorldPoints" => {
                let n = num(e, who, name, v)?;
                p.world = rows_of(who, name, &n, &[2, 3])?;
                p.world_cols = if p.world.is_empty() { 2 } else { n.dims[1] };
            }
            "WorldUnits" => {
                p.units = text_arg(e, v).ok_or_else(|| err(who, "WorldUnits must be text"))?;
            }
            "EstimateSkew" => p.estimate_skew = logical_arg(e, who, name, v)?,
            "EstimateTangentialDistortion" => p.estimate_tangential = logical_arg(e, who, name, v)?,
            "NumRadialDistortionCoefficients" => {
                let x = scalar(who, name, &num(e, who, name, v)?)?;
                if x != 2. && x != 3. {
                    return Err(err(who, "NumRadialDistortionCoefficients must be 2 or 3"));
                }
                num_radial_given = Some(x as usize);
            }
            _ => {
                let n = num(e, who, name, v)?;
                if n.data.is_empty() {
                    p.errors.clear();
                    errors_dims = None;
                    continue;
                }
                let ok = n.dims.len() <= 3 && n.dims[1] == 2;
                if !ok {
                    return Err(err(who, "ReprojectionErrors must be an M-by-2-by-P array"));
                }
                if n.data.len() > 2 * MAX_POINTS {
                    return Err(err(who, "ReprojectionErrors is too large"));
                }
                finite(who, name, &n)?;
                errors_dims = Some((n.dims[0], n.dims.get(2).copied().unwrap_or(1)));
                p.error_points = n.dims[0];
                p.errors = n.data;
            }
        }
    }
    match (radial_given, num_radial_given) {
        (true, Some(k)) if k != p.intr.radial.len() => {
            return Err(err(
                who,
                "NumRadialDistortionCoefficients must equal the number of RadialDistortion coefficients",
            ));
        }
        (true, _) => p.num_radial = p.intr.radial.len(),
        (false, Some(k)) => {
            p.num_radial = k;
            p.intr.radial = vec![0.; k];
        }
        (false, None) => {}
    }
    if p.rotation_vectors.len() != p.translation_vectors.len() {
        return Err(err(
            who,
            "RotationVectors and TranslationVectors must have the same number of rows",
        ));
    }
    if p.patterns() * p.world.len() > MAX_POINTS {
        return Err(err(who, "too many reprojected points"));
    }
    if let Some((m, pages)) = errors_dims {
        if !p.world.is_empty() && m != p.world.len() {
            return Err(err(
                who,
                "ReprojectionErrors must have one row per world point",
            ));
        }
        if p.patterns() > 0 && pages != p.patterns() {
            return Err(err(
                who,
                "ReprojectionErrors must have one page per pattern",
            ));
        }
    }
    Ok(p)
}

/// The intrinsics of a cameraParameters object as a cameraIntrinsics; None
/// without an ImageSize (the Intrinsics property is then []).
pub(super) fn intrinsics_of(p: &Params) -> Option<Intrinsics> {
    p.intr.size.map(|_| p.intr.clone())
}
