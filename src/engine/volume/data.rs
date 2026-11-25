//! Scalar volume operations on column-major meshgrid arrays.
use super::super::nd_shape::gather;
use super::super::sci_more::{Method, sc2_method, sc2_spline_coefs, sc2_uniform, sc2_weights};
use super::super::{Engine, Value};
use crate::{Array, Matrix, NumericClass, Result, TypedArray};

fn validate_real(v: &Value, name: &str) -> Result<()> {
    match v {
        Value::Matrix(_) | Value::Logical(_) | Value::Container(Array::Typed(_)) => Ok(()),
        Value::Complex(_) => Err(format!("{name}: complex input is not supported yet")),
        Value::Text(_) | Value::QuotedString(_) | Value::Container(Array::Character(_)) => {
            Err(format!("{name}: input must be numeric"))
        }
        _ => Err(format!("{name}: input must be numeric or logical")),
    }
}
fn real(e: &Engine, v: &Value, name: &str) -> Result<Matrix> {
    validate_real(v, name)?;
    v.clone().matrix(&e.cancel)?.with_dims(&v.dims())
}
fn volume_dims(v: &Value, name: &str) -> Result<[usize; 3]> {
    let d = v.dims();
    if d.len() != 3 || d.contains(&0) {
        return Err(format!("{name}: V must be a nonempty 3-D array"));
    }
    Ok([d[0], d[1], d[2]])
}
fn volume(e: &Engine, v: &Value, name: &str) -> Result<(Matrix, [usize; 3])> {
    Ok((real(e, v, name)?, volume_dims(v, name)?))
}
fn array(d: &[usize], data: Vec<f64>) -> Result<Value> {
    let rows = d[0];
    let cols = d[1..].iter().product();
    Ok(Value::Matrix(
        Matrix::from_column_vec(rows, cols, data)?.with_dims(d)?,
    ))
}
fn float_array_like(v: &Value, d: &[usize], data: Vec<f64>) -> Result<Value> {
    if matches!(v, Value::Container(Array::Typed(t)) if t.class() == NumericClass::Single) {
        let single = TypedArray::from_reals(
            NumericClass::Single,
            d[0],
            d[1..].iter().product(),
            data.into_iter(),
        )?;
        return Ok(Value::Container(single.with_dims(d)?.into()));
    }
    array(d, data)
}
fn vector(e: &Engine, v: &Value, name: &str) -> Result<Vec<f64>> {
    Ok(real(e, v, name)?.data().to_vec())
}
fn text(v: &Value) -> Option<&str> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(s),
        _ => None,
    }
}
fn index(d: [usize; 3], y: usize, x: usize, z: usize) -> usize {
    y + d[0] * (x + d[1] * z)
}
fn coord(i: usize, n: usize, a: f64, b: f64) -> f64 {
    if n <= 1 {
        b
    } else {
        a + (b - a) * i as f64 / (n - 1) as f64
    }
}
fn grid(e: &Engine, d: [usize; 3]) -> Result<Vec<Vec<f64>>> {
    let n = d.iter().product::<usize>();
    e.preflight_elements(n.saturating_mul(3))?;
    let mut g = vec![
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
    ];
    for z in 0..d[2] {
        for x in 0..d[1] {
            for y in 0..d[0] {
                g[0].push((x + 1) as f64);
                g[1].push((y + 1) as f64);
                g[2].push((z + 1) as f64);
            }
        }
    }
    Ok(g)
}
fn flow(e: &Engine, args: &[Value], count: usize) -> Result<Vec<Value>> {
    if args.len() > 1 || count > 4 {
        return Err("flow: expected zero or one input and at most four outputs".into());
    }
    let n = if args.is_empty() {
        25
    } else {
        let a = vector(e, &args[0], "flow")?;
        if a.len() != 1 || !a[0].is_finite() || a[0] < 2. || a[0].fract() != 0. || a[0] > 1000. {
            return Err("flow: n must be an integer between 2 and 1000".into());
        }
        a[0] as usize
    };
    let d = [n, 2 * n, n];
    let len = d.iter().product::<usize>();
    e.preflight_elements(len.saturating_mul(if count > 1 { 4 } else { 1 }))?;
    let mut vs = Vec::with_capacity(len);
    let mut xyz = if count > 1 {
        vec![
            Vec::with_capacity(len),
            Vec::with_capacity(len),
            Vec::with_capacity(len),
        ]
    } else {
        vec![]
    };
    for z in 0..d[2] {
        e.check_cancel()?;
        for x in 0..d[1] {
            for y in 0..d[0] {
                let xx = coord(x, d[1], 0.1, 10.);
                let yy = coord(y, d[0], -3., 3.);
                let zz = coord(z, d[2], -3., 3.);
                let r = xx.hypot(yy).hypot(zz);
                let cosphi = (yy.hypot(zz)) / r;
                let vr = 2. / r * (3. / (2. - cosphi).powi(2) - 1.);
                // MGH_FLOW converts (longitude, latitude, radius) = (0, vphi, vr)
                // to Cartesian coordinates, whose norm is abs(vr).
                vs.push(vr.abs().ln());
                if count > 1 {
                    xyz[0].push(xx);
                    xyz[1].push(yy);
                    xyz[2].push(zz);
                }
            }
        }
    }
    if count <= 1 {
        return Ok(vec![array(&d, vs)?]);
    }
    let mut out = xyz
        .into_iter()
        .map(|v| array(&d, v))
        .collect::<Result<Vec<_>>>()?;
    out.push(array(&d, vs)?);
    out.truncate(count);
    Ok(out)
}
fn smooth(e: &mut Engine, args: &[Value]) -> Result<Vec<Value>> {
    if args.is_empty() || args.len() > 4 {
        return Err("smooth3: expected V, method, size and standard deviation".into());
    }
    let (m, d) = volume(e, &args[0], "smooth3")?;
    let method = if args.len() > 1 {
        text(&args[1]).ok_or("smooth3: invalid method")?
    } else {
        "box"
    };
    let box_filter = method.eq_ignore_ascii_case("box");
    if !box_filter && !method.eq_ignore_ascii_case("gaussian") {
        return Err("smooth3: method must be box or gaussian".into());
    }
    let s = if args.len() > 2 {
        vector(e, &args[2], "smooth3")?
    } else {
        vec![3.]
    };
    if !(s.len() == 1 || s.len() == 3)
        || s.iter().any(|&x| {
            !x.is_finite()
                || x < 1.
                || x > 255.
                || x.fract() != 0.
                || (x as usize).is_multiple_of(2)
        })
    {
        return Err("smooth3: size must contain odd positive integers".into());
    }
    let sd = if args.len() > 3 {
        vector(e, &args[3], "smooth3")?
    } else {
        vec![0.65]
    };
    if !(sd.len() == 1 || sd.len() == 3) || sd.iter().any(|v| !v.is_finite() || *v <= 0.) {
        return Err(
            "smooth3: standard deviation must be a positive scalar or 3-element vector".into(),
        );
    }
    let sizes = [
        s[0] as usize,
        s[if s.len() == 1 { 0 } else { 1 }] as usize,
        s[if s.len() == 1 { 0 } else { 2 }] as usize,
    ];
    e.preflight_elements(m.len().saturating_mul(2))?;
    e.charge(m.len().saturating_mul(sizes.iter().sum()))?;
    let mut input = m.data().to_vec();
    for axis in 0..3 {
        let k = sizes[axis];
        let h = k / 2;
        let mut weights = (0..k)
            .map(|i| {
                if box_filter {
                    1.
                } else {
                    (-0.5
                        * ((i as f64 - h as f64) / sd[if sd.len() == 1 { 0 } else { axis }])
                            .powi(2))
                    .exp()
                }
            })
            .collect::<Vec<_>>();
        let sum: f64 = weights.iter().sum();
        for w in &mut weights {
            *w /= sum;
        }
        let mut output = vec![0.; m.len()];
        for z in 0..d[2] {
            e.check_cancel()?;
            for x in 0..d[1] {
                for y in 0..d[0] {
                    if y % 8192 == 0 {
                        e.check_cancel()?;
                    }
                    let pos = [y, x, z];
                    let mut val = 0.;
                    for (i, w) in weights.iter().enumerate() {
                        let mut p = pos;
                        p[axis] = (pos[axis] as isize + i as isize - h as isize)
                            .clamp(0, d[axis] as isize - 1)
                            as usize;
                        val += w * input[index(d, p[0], p[1], p[2])];
                    }
                    output[index(d, y, x, z)] = val;
                }
            }
        }
        input = output;
    }
    Ok(vec![float_array_like(&args[0], &d, input)?])
}
fn crop(e: &Engine, name: &str, args: &[Value], count: usize) -> Result<Vec<Value>> {
    let explicit = args.len() == 5;
    if !(args.len() == 2 || explicit) || count > 4 {
        return Err(format!(
            "{name}: expected V and parameter, optionally preceded by X,Y,Z"
        ));
    }
    let v = if explicit { 3 } else { 0 };
    validate_real(&args[v], name)?;
    let d = volume_dims(&args[v], name)?;
    let param = vector(e, &args[v + 1], name)?;
    let mut coords = Vec::new();
    if explicit {
        for val in &args[..3] {
            validate_real(val, name)?;
            if val.dims().as_ref() != d {
                return Err(format!("{name}: coordinate arrays must match V"));
            }
            if name == "subvolume" {
                coords.push(real(e, val, name)?);
            }
        }
    }
    if name == "subvolume" && explicit {
        for z in 0..d[2] {
            e.check_cancel()?;
            for x in 0..d[1] {
                for y in 0..d[0] {
                    let i = index(d, y, x, z);
                    if coords[0].data()[i] != coords[0].data()[index(d, 0, x, 0)]
                        || coords[1].data()[i] != coords[1].data()[index(d, y, 0, 0)]
                        || coords[2].data()[i] != coords[2].data()[index(d, 0, 0, z)]
                    {
                        return Err(
                            "subvolume: coordinate arrays must have meshgrid structure".into()
                        );
                    }
                }
            }
        }
    }
    let mut selected = [Vec::new(), Vec::new(), Vec::new()];
    if name == "reducevolume" {
        if !(param.len() == 1 || param.len() == 3)
            || param
                .iter()
                .any(|&x| !x.is_finite() || x < 1. || x.fract() != 0. || x > usize::MAX as f64)
        {
            return Err("reducevolume: reduction factors must be positive integers".into());
        }
        // Factors are [Rx Ry Rz], while storage order is [y x z].
        for a in 0..3 {
            let p = if param.len() == 1 { 0 } else { [1, 0, 2][a] };
            selected[a] = (0..d[a]).step_by(param[p] as usize).collect();
        }
    } else {
        if param.len() != 6 || param.iter().any(|x| x.is_infinite()) {
            return Err("subvolume: limits must contain six finite numbers or NaNs".into());
        }
        for a in 0..3 {
            let c = [1, 0, 2][a];
            let lo = param[2 * c];
            let hi = param[2 * c + 1];
            if !lo.is_nan() && !hi.is_nan() && lo > hi {
                return Err("subvolume: invalid limits".into());
            }
            for i in 0..d[a] {
                let x = if explicit {
                    let mut p = [0, 0, 0];
                    p[a] = i;
                    coords[c].data()[index(d, p[0], p[1], p[2])]
                } else {
                    (i + 1) as f64
                };
                if (lo.is_nan() || x >= lo) && (hi.is_nan() || x <= hi) {
                    selected[a].push(i);
                }
            }
        }
    }
    let od = [selected[0].len(), selected[1].len(), selected[2].len()];
    let len = od.iter().product::<usize>();
    e.preflight_elements(len.saturating_mul(count.max(1) + 1))?;
    let mut positions = Vec::with_capacity(len);
    for &z in &selected[2] {
        e.check_cancel()?;
        for &x in &selected[1] {
            for &y in &selected[0] {
                positions.push(index(d, y, x, z));
            }
        }
    }
    if count <= 1 {
        return Ok(vec![gather(e, &args[v], &positions, &od)?]);
    }
    let mut result = Vec::with_capacity(count);
    for c in 0..count.min(3) {
        e.check_cancel()?;
        result.push(if explicit {
            gather(e, &args[c], &positions, &od)?
        } else {
            let (stride, size) = [(d[0], d[1]), (1, d[0]), (d[0] * d[1], d[2])][c];
            let values = positions
                .iter()
                .map(|&i| (i / stride % size + 1) as f64)
                .collect();
            array(&od, values)?
        });
    }
    if count == 4 {
        result.push(gather(e, &args[v], &positions, &od)?);
    }
    Ok(result)
}
fn bounds(e: &Engine, args: &[Value]) -> Result<Vec<Value>> {
    if ![1, 2, 4, 5].contains(&args.len()) {
        return Err("volumebounds: expected V, optionally X,Y,Z and color data".into());
    }
    let explicit = args.len() >= 4;
    let vi = if explicit { 3 } else { 0 };
    let (m, d) = volume(e, &args[vi], "volumebounds")?;
    let coords = if explicit {
        let mut c = Vec::new();
        for a in &args[..3] {
            let v = real(e, a, "volumebounds")?;
            if v.dims().as_ref() != m.dims().as_ref() {
                return Err("volumebounds: coordinates must match V".into());
            }
            c.push(v.data().to_vec());
        }
        c
    } else {
        grid(e, d)?
    };
    let color = if args.len() == vi + 2 {
        real(e, &args[vi + 1], "volumebounds")?
    } else {
        m
    };
    let mut result = Vec::new();
    for v in coords
        .iter()
        .map(Vec::as_slice)
        .chain(std::iter::once(color.data()))
    {
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        let mut found = false;
        for &x in v {
            if !x.is_nan() {
                found = true;
                min = min.min(x);
                max = max.max(x);
            }
        }
        if !found {
            min = f64::NAN;
            max = f64::NAN;
        }
        result.extend([min, max]);
    }
    Ok(vec![array(&[1, 8], result)?])
}

/// Replace each line parallel to an axis by the four descending powers per
/// interval. `stride` is the number of entries between successive knots.
fn spline_axis(e: &Engine, data: &[f64], axis: &[f64], stride: usize) -> Result<Vec<f64>> {
    let pieces = 4 * (axis.len() - 1);
    let mut result = vec![0.; data.len() / axis.len() * pieces];
    let mut line = vec![0.; axis.len()];
    for (block, out) in data
        .chunks_exact(axis.len() * stride)
        .zip(result.chunks_exact_mut(pieces * stride))
    {
        for start in 0..stride {
            e.check_cancel()?;
            for (sample, row) in line.iter_mut().zip(block.chunks_exact(stride)) {
                *sample = row[start];
            }
            for (interval, [a, b, c, d]) in
                sc2_spline_coefs(e, axis, &line)?.into_iter().enumerate()
            {
                // Store powers of the fractional position in each interval, so
                // coefficients for different axes do not compound unit scales.
                let h = axis[interval + 1] - axis[interval];
                let coefs = [a * h * h * h, b * h * h, c * h, d];
                for (degree_index, coef) in coefs.into_iter().enumerate() {
                    out[start + (4 * interval + degree_index) * stride] = coef;
                }
            }
        }
    }
    Ok(result)
}

struct VolumeSpline {
    coefs: Vec<f64>,
    x_stride: usize,
    z_stride: usize,
}

impl VolumeSpline {
    fn new(e: &mut Engine, data: &[f64], axes: &[Vec<f64>], queries: usize) -> Result<Self> {
        let [nx, ny, nz] = std::array::from_fn(|i| axes[i].len());
        let x_stride = 4 * (ny - 1);
        let z_stride = x_stride.saturating_mul(4 * (nx - 1));
        let y_size = x_stride.saturating_mul(nx).saturating_mul(nz);
        let xy_size = z_stride.saturating_mul(nz);
        let xyz_size = z_stride.saturating_mul(4 * (nz - 1));
        let line_workspace = 16 * nx.max(ny).max(nz);
        e.preflight_elements(
            xy_size
                .saturating_add(xyz_size)
                .saturating_add(line_workspace)
                .saturating_add(queries),
        )?;
        e.charge(
            data.len()
                .saturating_add(y_size)
                .saturating_add(xy_size)
                .saturating_mul(16)
                .saturating_add(queries.saturating_mul(64)),
        )?;

        // The spline solve is linear in its samples. Interpolating each
        // coefficient along the next axis constructs the tensor-product pieces.
        let y_pieces = spline_axis(e, data, &axes[1], 1)?;
        let xy_pieces = spline_axis(e, &y_pieces, &axes[0], x_stride)?;
        drop(y_pieces);
        let coefs = spline_axis(e, &xy_pieces, &axes[2], z_stride)?;
        Ok(Self {
            coefs,
            x_stride,
            z_stride,
        })
    }

    fn sample(&self, [ix, iy, iz]: [usize; 3], [x, y, z]: [f64; 3]) -> f64 {
        let start = 4 * (iy + self.x_stride * ix + self.z_stride * iz);
        let mut value = 0.;
        for k in 0..4 {
            let mut plane = 0.;
            for j in 0..4 {
                let i = start + k * self.z_stride + j * self.x_stride;
                let c = &self.coefs[i..i + 4];
                let line = ((c[0] * y + c[1]) * y + c[2]) * y + c[3];
                plane = plane * x + line;
            }
            value = value * z + plane;
        }
        value
    }
}

pub(super) fn interp(e: &mut Engine, args: &[Value]) -> Result<Vec<Value>> {
    let explicit = args.len() >= 7 && text(&args[4]).is_none();
    let base = if explicit { 7 } else { 4 };
    if args.len() < base || args.len() > base + 2 {
        return Err("interp3: invalid arguments".into());
    }
    let vi = if explicit { 3 } else { 0 };
    if args.len() == 4 && args[vi].dims().len() == 2 {
        return Err("Output argument \"varargout{3}\" (and maybe others) not assigned during call to \"meshgridvectors\".".into());
    }
    let (m, d) = volume(e, &args[vi], "interp3")?;
    let invalid_method = "Invalid interpolation type specified.";
    let method = if args.len() > base {
        text(&args[base])
            .and_then(sc2_method)
            .filter(|&method| method != Method::Pchip)
            .ok_or(invalid_method)?
    } else {
        Method::Linear
    };
    let extra = if args.len() > base + 1 {
        let x = vector(e, &args[base + 1], "interp3")?;
        if x.len() != 1 {
            return Err("interp3: extrapolation value must be scalar".into());
        }
        x[0]
    } else {
        f64::NAN
    };
    let axes = if explicit {
        let mut axes = Vec::new();
        for c in 0..3 {
            let v = real(e, &args[c], "interp3")?;
            let n = d[[1, 0, 2][c]];
            let data = if v.len() == n {
                v.data().to_vec()
            } else if v.dims().as_ref() == m.dims().as_ref() {
                (0..n)
                    .map(|i| {
                        let mut p = [0, 0, 0];
                        p[[1, 0, 2][c]] = i;
                        v.data()[index(d, p[0], p[1], p[2])]
                    })
                    .collect()
            } else {
                return Err("interp3: grid must match V".into());
            };
            if v.len() != n {
                for z in 0..d[2] {
                    e.check_cancel()?;
                    for x in 0..d[1] {
                        for y in 0..d[0] {
                            let i = index(d, y, x, z);
                            let axis_index = [x, y, z][c];
                            if v.data()[i] != data[axis_index] {
                                return Err(
                                    "interp3: grid arrays must have meshgrid structure".into()
                                );
                            }
                        }
                    }
                }
            }
            if data.len() < 2
                || data
                    .windows(2)
                    .any(|w| !w[0].is_finite() || !w[1].is_finite() || w[0] >= w[1])
            {
                return Err("interp3: grid vectors must be strictly increasing".into());
            }
            axes.push(data);
        }
        axes
    } else {
        vec![
            (1..=d[1]).map(|x| x as f64).collect(),
            (1..=d[0]).map(|x| x as f64).collect(),
            (1..=d[2]).map(|x| x as f64).collect(),
        ]
    };
    let qi = if explicit { 4 } else { 1 };
    let q = (qi..qi + 3)
        .map(|i| real(e, &args[i], "interp3"))
        .collect::<Result<Vec<_>>>()?;
    let shape = q[0].dims().into_owned();
    if q.iter().any(|a| a.dims().as_ref() != shape.as_slice()) {
        return Err("interp3: query arrays must have equal sizes".into());
    }
    e.preflight_elements(q[0].len())?;
    let cubic = method == Method::Cubic && axes.iter().all(|axis| sc2_uniform(axis));
    let spline = method == Method::Spline || (method == Method::Cubic && !cubic);
    if (cubic || spline) && d.iter().any(|&x| x < 2) {
        return Err("interp3: cubic and spline require at least two points per axis".into());
    }
    if cubic {
        // The end extension can use six terms per axis.
        e.charge(q[0].len().saturating_mul(6 * 6 * 6))?;
    }
    let mut spline_grid = None;
    let mut out = Vec::with_capacity(q[0].len());
    for i in 0..q[0].len() {
        if i % 8192 == 0 {
            e.check_cancel()?;
        }
        let mut idx = [0; 3];
        let mut t = [0.; 3];
        let mut outside = false;
        for a in 0..3 {
            let x = q[a].data()[i];
            let axis = &axes[a];
            if axis.len() < 2 || !x.is_finite() || x < axis[0] || x > axis[axis.len() - 1] {
                outside = true;
                break;
            }
            let k = axis
                .partition_point(|v| *v <= x)
                .saturating_sub(1)
                .min(axis.len() - 2);
            idx[a] = k;
            t[a] = (x - axis[k]) / (axis[k + 1] - axis[k]);
        }
        if outside {
            out.push(extra);
            continue;
        }
        if spline {
            let grid = match &spline_grid {
                Some(grid) => grid,
                None => spline_grid.insert(VolumeSpline::new(e, m.data(), &axes, q[0].len())?),
            };
            out.push(grid.sample(idx, t));
        } else if cubic {
            let [wx, wy, wz] =
                std::array::from_fn(|a| sc2_weights(&axes[a], q[a].data()[i], Method::Cubic, true));
            let mut val = 0.;
            for &(z, w_z) in &wz {
                let mut plane = 0.;
                for &(x, w_x) in &wx {
                    let mut line = 0.;
                    for &(y, w_y) in &wy {
                        line += w_y * m.data()[index(d, y, x, z)];
                    }
                    plane += w_x * line;
                }
                val += w_z * plane;
            }
            out.push(val);
        } else if method == Method::Nearest {
            for a in 0..3 {
                if t[a] >= 0.5 {
                    idx[a] += 1;
                }
            }
            out.push(m.data()[index(d, idx[1], idx[0], idx[2])]);
        } else {
            let mut val = 0.;
            for z in 0..2 {
                for x in 0..2 {
                    for y in 0..2 {
                        let w = if x == 0 { 1. - t[0] } else { t[0] }
                            * if y == 0 { 1. - t[1] } else { t[1] }
                            * if z == 0 { 1. - t[2] } else { t[2] };
                        val += w * m.data()[index(d, idx[1] + y, idx[0] + x, idx[2] + z)];
                    }
                }
            }
            out.push(val);
        }
    }
    Ok(vec![float_array_like(&args[vi], &shape, out)?])
}
pub(in crate::engine) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
) -> Result<Vec<Value>> {
    match name {
        "flow" => flow(e, args, count),
        "smooth3" => smooth(e, args),
        "reducevolume" | "subvolume" => crop(e, name, args, count),
        "volumebounds" => bounds(e, args),
        "interp3" => interp(e, args),
        _ => unreachable!(),
    }
}
