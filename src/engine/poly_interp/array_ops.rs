//! fft2, ifft2, fftshift, ifftshift, mode, histc, histcounts.
use super::{Engine, Value, eps_of, is_vector, matrix_value, real_matrix, scalar_of, text_of};
use crate::{
    Array, Complex64, LogicalMatrix, Result,
    dsp::{self, FftOptions, TransformDirection},
};

fn positive_integer(value: &Value, who: &str, what: &str) -> Result<usize> {
    let v = scalar_of(value, who, what)?;
    if !v.is_finite() || v < 1. || v.fract() != 0. || v > crate::matrix::MAX_ELEMENTS as f64 {
        return Err(format!("{who}: {what} must be a positive integer"));
    }
    Ok(v as usize)
}
fn array_value(array: Array) -> Result<Value> {
    match array {
        Array::Numeric(a) => Ok(Value::Matrix(a)),
        Array::Complex(a) => Ok(Value::Complex(a)),
        _ => Err("Fourier results must be double or complex double".into()),
    }
}
/// out[(i + sr) % rows, (j + sc) % cols] = data[i, j]
fn rotate<T: Copy>(data: &[T], rows: usize, cols: usize, sr: usize, sc: usize) -> Vec<T> {
    let mut out = Vec::with_capacity(data.len());
    if let Some(&first) = data.first() {
        out.resize(data.len(), first);
    }
    for j in 0..cols {
        let dj = (j + sc) % cols;
        for i in 0..rows {
            out[dj * rows + (i + sr) % rows] = data[j * rows + i];
        }
    }
    out
}
/// Sorted-lane mode: smallest of the most frequent values; NaNs never repeat.
fn lane_mode(lane: &mut [f64]) -> (f64, f64) {
    if lane.is_empty() {
        return (f64::NAN, 0.);
    }
    lane.sort_by(|a, b| a.total_cmp(b));
    let (mut best, mut best_count) = (lane[0], 0usize);
    let mut i = 0;
    while i < lane.len() {
        let mut j = i + 1;
        while j < lane.len() && lane[j] == lane[i] {
            j += 1;
        }
        if j - i > best_count {
            (best, best_count) = (lane[i], j - i);
        }
        i = j;
    }
    (best, best_count as f64)
}
fn bin_of(edges: &[f64], x: f64) -> usize {
    let j = edges.partition_point(|&e| e <= x);
    if j == 0 {
        0
    } else if j == edges.len() {
        if x == edges[j - 1] { j } else { 0 }
    } else {
        j
    }
}

fn min_max(x: &[f64]) -> Option<(f64, f64)> {
    let first = *x.first()?;
    Some(
        x.iter()
            .fold((first, first), |(l, h), &v| (l.min(v), h.max(v))),
    )
}
/// histcounts 'auto': one bin per integer for integer data
/// spanning at most 50, otherwise Scott's rule placed by `bin_picker`.
fn auto_edges(x: &[f64]) -> Vec<f64> {
    let Some((lo, hi)) = min_max(x) else {
        return vec![0., 1.];
    };
    let flint_half = 2f64.powi(52);
    if x.iter().all(|v| v.fract() == 0.) && hi - lo <= 50. && hi <= flint_half && lo >= -flint_half
    {
        let n = (hi - lo) as usize + 1;
        return (0..=n).map(|i| lo - 0.5 + i as f64).collect();
    }
    let n = x.len() as f64;
    let mean = x.iter().sum::<f64>() / n;
    let var = if x.len() > 1 {
        x.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / (n - 1.)
    } else {
        0.
    };
    bin_picker(lo, hi, None, 3.5 * var.sqrt() / n.cbrt())
}
/// Choose "nice" bin edges covering [xmin, xmax].
fn bin_picker(xmin: f64, xmax: f64, nbins: Option<usize>, raw_width: f64) -> Vec<f64> {
    let xscale = xmin.abs().max(xmax.abs());
    let xrange = xmax - xmin;
    let raw_width = raw_width.max(eps_of(xscale));
    let (left, right, width, count);
    if xrange > eps_of(xscale).sqrt().max(f64::MIN_POSITIVE) {
        let pow10 = 10f64.powf(raw_width.log10().floor());
        let rel = raw_width / pow10;
        match nbins {
            None => {
                let factor = if rel < 1.5 {
                    1.
                } else if rel < 2.5 {
                    2.
                } else if rel < 4. {
                    3.
                } else if rel < 7.5 {
                    5.
                } else {
                    10.
                };
                width = factor * pow10;
                left = (width * (xmin / width).floor()).min(xmin).max(-f64::MAX);
                count = ((xmax - left) / width).ceil().max(1.) as usize;
                right = (left + count as f64 * width).max(xmax).min(f64::MAX);
            }
            Some(n) => {
                let mut w = pow10 * rel.floor();
                left = (w * (xmin / w).floor()).min(xmin).max(-f64::MAX);
                if n > 1 {
                    let ll = (xmax - left) / n as f64;
                    let ul = (xmax - left) / (n - 1) as f64;
                    let p10 = 10f64.powf((ul - ll).log10().floor());
                    w = p10 * (ll / p10).ceil();
                }
                width = w;
                count = n;
                right = (left + n as f64 * width).max(xmax).min(f64::MAX);
            }
        }
    } else {
        // Nearly constant data: cover a unit width around it.
        let n = nbins.unwrap_or(1);
        let range = (n as f64 * eps_of(xscale)).ceil().max(1.);
        left = (2. * (xmin - range / 4.)).floor() / 2.;
        right = (2. * (xmax + range / 4.)).ceil() / 2.;
        width = (right - left) / n as f64;
        count = n;
    }
    if !width.is_finite() {
        return (0..=count)
            .map(|i| left + (right - left) * i as f64 / count as f64)
            .collect();
    }
    let mut edges: Vec<f64> = (0..count).map(|i| left + i as f64 * width).collect();
    edges.push(right);
    edges
}

impl Engine {
    pub(super) fn polyopt_fft2(&self, name: &str, args: &[Value]) -> Result<Value> {
        if args.is_empty() {
            return Err("Not enough input arguments.".into());
        }
        if args.len() != 1 && args.len() != 3 {
            return Err(if args.len() == 2 {
                "Not enough input arguments."
            } else {
                "Too many input arguments."
            }
            .into());
        }
        let input = match &args[0] {
            Value::Matrix(a) => Array::Numeric(a.clone()),
            Value::Logical(a) => Array::Logical(a.clone()),
            Value::Complex(a) => Array::Complex(a.clone()),
            _ => return Err(format!("{name} requires a real, logical or complex array")),
        };
        let size = if args.len() == 3 {
            Some((
                positive_integer(&args[1], name, "the row count")?,
                positive_integer(&args[2], name, "the column count")?,
            ))
        } else {
            None
        };
        let inverse = name == "ifft2";
        let direction = if inverse {
            TransformDirection::Inverse
        } else {
            TransformDirection::Forward
        };
        let budget = (32usize * 1024 * 1024)
            .saturating_sub(self.evaluated_elements)
            .checked_mul(8)
            .ok_or("FFT evaluation budget overflow")?;
        let symmetric = inverse && self.polyopt_conjugate_symmetric(&input, size)?;
        let first = dsp::fourier(
            &input,
            FftOptions {
                length: size.map(|s| s.0),
                dimension: Some(1),
            },
            direction,
            budget,
            &self.cancel,
        )?;
        let second = dsp::fourier(
            &first,
            FftOptions {
                length: size.map(|s| s.1),
                dimension: Some(2),
            },
            direction,
            budget,
            &self.cancel,
        )?;
        // The reference returns a real array whenever every imaginary part is zero.
        match second {
            Array::Complex(z) if symmetric || z.data().iter().all(|v| v.im == 0.) => {
                Ok(Value::Matrix(z.real(&self.cancel)?))
            }
            other => array_value(other),
        }
    }
    /// Exact conj-symmetry X(i,j) == conj(X(-i,-j)) of the array that is transformed:
    /// `input` cropped or zero-padded to `size` when given.
    fn polyopt_conjugate_symmetric(
        &self,
        input: &Array,
        size: Option<(usize, usize)>,
    ) -> Result<bool> {
        let (in_rows, in_cols) = (input.rows(), input.cols());
        let (rows, cols) = size.unwrap_or((in_rows, in_cols));
        let value = |i: usize, j: usize| -> Result<Complex64> {
            if i >= in_rows || j >= in_cols {
                return Ok(Complex64::new(0., 0.));
            }
            Ok(match input {
                Array::Numeric(a) => Complex64::new(a.get(i, j), 0.),
                Array::Logical(a) => Complex64::new(f64::from(a.data()[j * in_rows + i]), 0.),
                Array::Complex(a) => a.get(i, j)?,
                _ => return Err("ifft2 requires a numeric array".into()),
            })
        };
        for j in 0..cols {
            self.check_cancel()?;
            for i in 0..rows {
                let a = value(i, j)?;
                let b = value((rows - i) % rows, (cols - j) % cols)?;
                if a.re != b.re || a.im != -b.im {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    pub(super) fn polyopt_shift(&self, name: &str, args: &[Value]) -> Result<Value> {
        if args.is_empty() {
            return Err("Not enough input arguments.".into());
        }
        if args.len() > 2 {
            return Err("Too many input arguments.".into());
        }
        let dim = args
            .get(1)
            .map(|v| positive_integer(v, name, "the dimension"))
            .transpose()?;
        let (rows, cols) = match &args[0] {
            Value::Matrix(a) => (a.rows(), a.cols()),
            Value::Logical(a) => (a.rows(), a.cols()),
            Value::Complex(a) => (a.rows(), a.cols()),
            Value::Text(s) => (1, s.chars().count()),
            _ => return Err(format!("{name} requires a numeric, logical or char array")),
        };
        self.polyopt_admit(rows * cols)?;
        let amount = |n: usize| if name == "fftshift" { n / 2 } else { n - n / 2 };
        let sr = if dim.is_none_or(|d| d == 1) && rows > 0 {
            amount(rows)
        } else {
            0
        };
        let sc = if dim.is_none_or(|d| d == 2) && cols > 0 {
            amount(cols)
        } else {
            0
        };
        match &args[0] {
            Value::Matrix(a) => matrix_value(rows, cols, rotate(a.data(), rows, cols, sr, sc)),
            Value::Logical(a) => Ok(Value::Logical(LogicalMatrix::from_column_vec(
                rows,
                cols,
                rotate(a.data(), rows, cols, sr, sc),
                &self.cancel,
            )?)),
            Value::Complex(a) => {
                super::complex_value(self, rows, cols, rotate(a.data(), rows, cols, sr, sc))
            }
            Value::Text(s) => {
                let chars: Vec<char> = s.chars().collect();
                Ok(Value::Text(
                    rotate(&chars, rows, cols, sr, sc).into_iter().collect(),
                ))
            }
            _ => unreachable!(),
        }
    }

    pub(super) fn polyopt_mode(&self, args: &[Value], nout: usize) -> Result<Vec<Value>> {
        if args.is_empty() {
            return Err("Not enough input arguments.".into());
        }
        if args.len() > 2 {
            return Err("Too many input arguments.".into());
        }
        // Logical and char inputs keep their class in the first output.
        let class = &args[0];
        let x = match class {
            Value::Complex(_) => return self.cxr_mode(args, nout),
            Value::Matrix(_) | Value::Logical(_) => real_matrix(class, "mode")?,
            Value::Text(s) => crate::Matrix::from_column_vec(
                1,
                s.chars().count(),
                s.chars().map(|c| f64::from(u32::from(c))).collect(),
            )?,
            _ => return Err("mode requires a real numeric, logical or char array".into()),
        };
        let dim = match args.get(1) {
            Some(v) => positive_integer(v, "mode", "the dimension")?,
            None if x.rows() != 1 => 1,
            None => 2,
        };
        let (rows, cols) = (x.rows(), x.cols());
        if rows == 0 && cols == 0 && args.len() == 1 {
            return Ok(vec![
                matrix_value(1, 1, vec![f64::NAN])?,
                matrix_value(1, 1, vec![0.])?,
            ]
            .into_iter()
            .take(nout.max(1))
            .collect());
        }
        let (lanes, len, stride_lane, stride_elem, orows, ocols) = match dim {
            1 => (cols, rows, rows, 1, 1, cols),
            2 => (rows, cols, 1, rows, rows, 1),
            _ => (rows * cols, 1, 1, 1, rows, cols),
        };
        let mut modes = Vec::with_capacity(lanes);
        let mut counts = Vec::with_capacity(lanes);
        for lane in 0..lanes {
            self.check_cancel()?;
            let mut values: Vec<f64> = (0..len)
                .map(|k| {
                    x.data()[if dim <= 2 {
                        lane * stride_lane + k * stride_elem
                    } else {
                        lane
                    }]
                })
                .collect();
            let (m, f) = lane_mode(&mut values);
            modes.push(m);
            counts.push(f);
        }
        let first = match class {
            Value::Logical(_) if !modes.iter().any(|v| v.is_nan()) => {
                Value::Logical(LogicalMatrix::from_column_vec(
                    orows,
                    ocols,
                    modes.iter().map(|&v| u8::from(v != 0.)).collect(),
                    &self.cancel,
                )?)
            }
            Value::Text(_) if orows == 1 && !modes.iter().any(|v| v.is_nan()) => Value::Text(
                modes
                    .iter()
                    .filter_map(|&v| char::from_u32(v as u32))
                    .collect(),
            ),
            _ => matrix_value(orows, ocols, modes)?,
        };
        let mut out = vec![first];
        if nout > 1 {
            out.push(matrix_value(orows, ocols, counts)?);
        }
        Ok(out)
    }

    pub(super) fn polyopt_histc(&self, args: &[Value], nout: usize) -> Result<Vec<Value>> {
        if args.len() < 2 {
            return Err("Not enough input arguments.".into());
        }
        if args.len() > 3 {
            return Err("Too many input arguments.".into());
        }
        let x = real_matrix(&args[0], "histc")?;
        let edges = real_matrix(&args[1], "histc")?;
        if !edges.is_empty() && !is_vector(&edges) {
            return Err("histc: edges must be a vector".into());
        }
        let e = edges.data();
        if e.iter().any(|v| v.is_nan()) || e.windows(2).any(|w| w[1] < w[0]) {
            return Err("Edges vector must be monotonically non-decreasing.".into());
        }
        let dim = match args.get(2) {
            Some(v) => positive_integer(v, "histc", "the dimension")?,
            None if x.rows() != 1 => 1,
            None => 2,
        };
        if dim > 2 {
            return Err(
                "histc dimension must be 1 or 2; higher-dimensional arrays are not implemented"
                    .into(),
            );
        }
        let (rows, cols) = (x.rows(), x.cols());
        let ne = e.len();
        let (orows, ocols) = if dim == 1 { (ne, cols) } else { (rows, ne) };
        self.polyopt_admit(orows * ocols + rows * cols)?;
        let mut counts = vec![0.; orows * ocols];
        let mut bins = vec![0.; rows * cols];
        for j in 0..cols {
            if j % 1024 == 0 {
                self.check_cancel()?;
            }
            for i in 0..rows {
                let idx = j * rows + i;
                let b = bin_of(e, x.data()[idx]);
                bins[idx] = b as f64;
                if b > 0 {
                    if dim == 1 {
                        counts[j * orows + b - 1] += 1.;
                    } else {
                        counts[(b - 1) * orows + i] += 1.;
                    }
                }
            }
        }
        let mut out = vec![matrix_value(orows, ocols, counts)?];
        if nout > 1 {
            out.push(matrix_value(rows, cols, bins)?);
        }
        Ok(out)
    }

    pub(super) fn polyopt_histcounts(&self, args: &[Value], nout: usize) -> Result<Vec<Value>> {
        if args.is_empty() {
            return Err("Not enough input arguments.".into());
        }
        if args.len() > 2 || args.iter().skip(1).any(|a| text_of(a).is_some()) {
            return Err("histcounts supports histcounts(X), histcounts(X,nbins) and histcounts(X,edges); name-value options are not supported yet".into());
        }
        let x = real_matrix(&args[0], "histcounts")?;
        let finite: Vec<f64> = x.data().iter().copied().filter(|v| v.is_finite()).collect();
        let edges: Vec<f64> = match args.get(1) {
            None => auto_edges(&finite),
            Some(arg) => {
                let m = real_matrix(arg, "histcounts")?;
                if m.len() == 1 {
                    let nb = positive_integer(arg, "histcounts", "the number of bins")?;
                    if nb > 1 << 20 {
                        return Err("histcounts: too many bins".into());
                    }
                    match min_max(&finite) {
                        Some((lo, hi)) => bin_picker(lo, hi, Some(nb), (hi - lo) / nb as f64),
                        None => (0..=nb).map(|i| i as f64).collect(),
                    }
                } else if is_vector(&m) && m.len() >= 2 {
                    if m.data().iter().any(|v| v.is_nan())
                        || m.data().windows(2).any(|w| w[1] < w[0])
                    {
                        return Err("histcounts: edges must be a nondecreasing vector".into());
                    }
                    m.data().to_vec()
                } else {
                    return Err("histcounts: the second argument must be a bin count or a vector of at least two edges".into());
                }
            }
        };
        let nb = edges.len() - 1;
        self.polyopt_admit(2 * nb + 2)?;
        let mut counts = vec![0.; nb];
        for (i, &v) in x.data().iter().enumerate() {
            if i % 65536 == 0 {
                self.check_cancel()?;
            }
            let b = bin_of(&edges, v);
            if b > 0 {
                counts[(b - 1).min(nb - 1)] += 1.;
            }
        }
        let mut out = vec![matrix_value(1, nb, counts)?];
        if nout > 1 {
            out.push(matrix_value(1, nb + 1, edges)?);
        }
        Ok(out)
    }
}
