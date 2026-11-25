//! Pairwise distances, condensed order (1,2),(1,3),...,(m-1,m).
//! Mahalanobis distances use a Cholesky solve, never a covariance inverse.
use super::*;
use crate::FunctionHandle;
use rayon::prelude::*;
use std::sync::Arc;

pub(super) struct Metric {
    pub(super) name: String,
    power: f64,
    scale: Vec<f64>,
    chol: Option<nalgebra::linalg::Cholesky<f64, nalgebra::Dyn>>,
    handle: Option<FunctionHandle>,
}
impl Metric {
    pub(super) fn new(
        e: &Engine,
        caller: &str,
        x: &Matrix,
        v: Option<&Value>,
        parameter: Option<&Value>,
    ) -> Result<Self> {
        let handle = match v {
            Some(Value::Container(Array::Handle(h))) => Some(h.clone()),
            _ => None,
        };
        let name = if handle.is_some() {
            "handle".into()
        } else if let Some(v) = v {
            e.text_value(v.clone())?.to_lowercase()
        } else {
            "euclidean".into()
        };
        if ![
            "handle",
            "euclidean",
            "squaredeuclidean",
            "sqeuclidean",
            "seuclidean",
            "mahalanobis",
            "cityblock",
            "minkowski",
            "chebychev",
            "cosine",
            "correlation",
            "spearman",
            "hamming",
            "jaccard",
        ]
        .contains(&name.as_str())
        {
            return Err(format!("{caller}: unknown distance '{name}'"));
        }
        let mut metric = Self {
            name,
            power: 2.,
            scale: vec![],
            chol: None,
            handle,
        };
        if parameter.is_some()
            && !["seuclidean", "minkowski", "mahalanobis"].contains(&metric.name.as_str())
        {
            return Err(format!(
                "{caller}: this distance does not accept a parameter"
            ));
        }
        let p = x.cols();
        if metric.name == "spearman" {
            size(e, caller, p, 8)?;
        }
        if metric.name == "minkowski" {
            if let Some(v) = parameter {
                metric.power = scalar(e, caller, v)?;
            }
            if metric.power <= 0. {
                return Err(format!("{caller}: Minkowski exponent must be positive"));
            }
        }
        if metric.name == "seuclidean" || metric.name == "mahalanobis" {
            let keep = clean_rows(e, caller, x, None)?;
            if let Some(v) = parameter {
                let m = real(e, caller, v)?;
                if metric.name == "seuclidean" {
                    if m.len() != p || m.data().iter().any(|v| !v.is_finite() || *v < 0.) {
                        return Err(format!(
                            "{caller}: scale must be a nonnegative vector matching columns"
                        ));
                    }
                    metric.scale = m.data().to_vec();
                } else {
                    if m.rows() != p || m.cols() != p || m.data().iter().any(|v| !v.is_finite()) {
                        return Err(format!("{caller}: covariance must match columns"));
                    }
                    let cov = dense(&m);
                    for i in 0..p {
                        for j in 0..i {
                            if (cov[(i, j)] - cov[(j, i)]).abs() > 1e-12 * cov[(i, j)].abs().max(1.)
                            {
                                return Err(format!("{caller}: covariance must be symmetric"));
                            }
                        }
                    }
                    metric.chol = cov.cholesky();
                }
            } else {
                if keep.len() < 2 {
                    return Err(format!(
                        "{caller}: insufficient complete rows to estimate scale/covariance"
                    ));
                }
                let mean: Vec<_> = (0..p)
                    .map(|j| {
                        keep.iter()
                            .map(|&r| x.data()[r + j * x.rows()])
                            .sum::<f64>()
                            / keep.len() as f64
                    })
                    .collect();
                if metric.name == "seuclidean" {
                    for (j, &mean) in mean.iter().enumerate() {
                        if j.is_multiple_of(256) {
                            e.check_cancel()?;
                        }
                        metric.scale.push(
                            (keep
                                .iter()
                                .map(|&r| (x.data()[r + j * x.rows()] - mean).powi(2))
                                .sum::<f64>()
                                / (keep.len() - 1) as f64)
                                .sqrt(),
                        );
                    }
                } else {
                    size(e, caller, p, p)?;
                    let z = DMatrix::from_fn(keep.len(), p, |r, c| {
                        x.data()[keep[r] + c * x.rows()] - mean[c]
                    });
                    let cov = z.transpose() * z / (keep.len() - 1) as f64;
                    metric.chol = cov.cholesky();
                }
            }
            if metric.name == "mahalanobis" && metric.chol.is_none() {
                return Err(format!("{caller}: covariance must be positive definite"));
            }
        }
        Ok(metric)
    }
    pub(super) fn batch(
        &self,
        e: &mut Engine,
        caller: &str,
        xi: &[f64],
        xj: &Matrix,
        out: &mut Execution,
    ) -> Result<Vec<f64>> {
        if let Some(h) = &self.handle {
            let mut v = e.call_handle(
                h,
                vec![value(1, xi.len(), xi.to_vec())?, Value::Matrix(xj.clone())],
                1,
                None,
                out,
            )?;
            if v.is_empty() {
                return Err(format!("{caller}: distance handle must return a value"));
            }
            let d = real(e, caller, &v.remove(0))?;
            if d.rows() != xj.rows() || d.cols() != 1 {
                return Err(format!(
                    "{caller}: distance handle must return one column with one entry per comparison row"
                ));
            }
            if d.data().iter().any(|v| *v < 0. || v.is_infinite()) {
                return Err(format!("{caller}: invalid custom distance"));
            }
            let mut result = d.data().to_vec();
            for (r, v) in result.iter_mut().enumerate() {
                if xi.iter().any(|v| v.is_nan())
                    || (0..xj.cols()).any(|c| xj.data()[r + c * xj.rows()].is_nan())
                {
                    *v = f64::NAN;
                }
            }
            return Ok(result);
        }
        self.numeric_batch(&e.cancel, xi, xj, 0)
    }
    fn numeric_batch(
        &self,
        cancel: &std::sync::atomic::AtomicBool,
        xi: &[f64],
        xj: &Matrix,
        start: usize,
    ) -> Result<Vec<f64>> {
        let mut d = vec![0.; xj.rows() - start];
        let compute = |(chunk, output): (usize, &mut [f64])| -> Result<()> {
            crate::dsp::check_cancel(cancel)?;
            let mut point = vec![0.; xj.cols()];
            for (i, result) in output.iter_mut().enumerate() {
                let r = start + chunk * 256 + i;
                for (c, v) in point.iter_mut().enumerate() {
                    *v = xj.data()[r + c * xj.rows()];
                }
                *result = self.pair(xi, &point);
            }
            Ok(())
        };
        // Use only a caller-configured pool, and never spawn a nested pool.
        // The work threshold is benchmarked by examples/stats_models_probe.rs.
        if d.len().saturating_mul(xj.cols()) >= 262_144
            && crate::parallel::current_worker_count() > 1
        {
            d.par_chunks_mut(256).enumerate().try_for_each(compute)?;
        } else {
            for chunk in d.chunks_mut(256).enumerate() {
                compute(chunk)?;
            }
        }
        Ok(d)
    }
    pub(super) fn pair(&self, x: &[f64], y: &[f64]) -> f64 {
        if x.iter().chain(y).any(|v| v.is_nan()) {
            return f64::NAN;
        }
        match self.name.as_str() {
            "euclidean" => x.iter().zip(y).fold(0_f64, |s, (a, b)| s.hypot(a - b)),
            "squaredeuclidean" | "sqeuclidean" => {
                x.iter().zip(y).map(|(a, b)| (a - b).powi(2)).sum()
            }
            "cityblock" => x.iter().zip(y).map(|(a, b)| (a - b).abs()).sum(),
            "chebychev" => x
                .iter()
                .zip(y)
                .map(|(a, b)| (a - b).abs())
                .fold(0., f64::max),
            "minkowski" => {
                let scale = x
                    .iter()
                    .zip(y)
                    .map(|(a, b)| (a - b).abs())
                    .fold(0., f64::max);
                if scale == 0. {
                    0.
                } else {
                    scale
                        * x.iter()
                            .zip(y)
                            .map(|(a, b)| ((a - b).abs() / scale).powf(self.power))
                            .sum::<f64>()
                            .powf(1. / self.power)
                }
            }
            "seuclidean" => x
                .iter()
                .zip(y)
                .zip(&self.scale)
                .map(|((a, b), s)| ((a - b) / s).powi(2))
                .sum::<f64>()
                .sqrt(),
            "mahalanobis" => {
                let diff =
                    nalgebra::DVector::from_iterator(x.len(), x.iter().zip(y).map(|(a, b)| a - b));
                let solved = self
                    .chol
                    .as_ref()
                    .expect("validated covariance")
                    .solve(&diff);
                diff.dot(&solved).max(0.).sqrt()
            }
            "cosine" => cosine(x, y, false),
            "correlation" => cosine(x, y, true),
            "spearman" => cosine(&ranks(x), &ranks(y), true),
            "hamming" => x.iter().zip(y).filter(|(a, b)| a != b).count() as f64 / x.len() as f64,
            "jaccard" => {
                let union = x
                    .iter()
                    .zip(y)
                    .filter(|(a, b)| **a != 0. || **b != 0.)
                    .count();
                if union == 0 {
                    f64::NAN
                } else {
                    x.iter().zip(y).filter(|(a, b)| a != b).count() as f64 / union as f64
                }
            }
            _ => f64::NAN,
        }
    }
}
fn cosine(x: &[f64], y: &[f64], center: bool) -> f64 {
    let mx = if center {
        x.iter().sum::<f64>() / x.len() as f64
    } else {
        0.
    };
    let my = if center {
        y.iter().sum::<f64>() / y.len() as f64
    } else {
        0.
    };
    let mut dot = 0.;
    let mut xx = 0.;
    let mut yy = 0.;
    for (&a, &b) in x.iter().zip(y) {
        dot += (a - mx) * (b - my);
        xx += (a - mx).powi(2);
        yy += (b - my).powi(2);
    }
    (1. - dot / (xx * yy).sqrt()).clamp(0., 2.)
}
fn ranks(x: &[f64]) -> Vec<f64> {
    let mut order: Vec<_> = (0..x.len()).collect();
    order.sort_by(|&i, &j| x[i].total_cmp(&x[j]));
    let mut ranks = vec![0.; x.len()];
    let mut start = 0;
    while start < x.len() {
        let mut end = start + 1;
        while end < x.len() && x[order[end]] == x[order[start]] {
            end += 1;
        }
        for &i in &order[start..end] {
            ranks[i] = (start + end - 1) as f64 / 2.;
        }
        start = end;
    }
    ranks
}

pub(super) fn call(
    e: &mut Engine,
    name: &str,
    a: &[Value],
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if a.is_empty() {
        return Err(format!("{name}: missing input"));
    }
    if name == "squareform" {
        return squareform(e, a);
    }
    if name == "knnsearch" {
        return knn(e, a, out);
    }
    let x = real(e, name, &a[0])?;
    let offset = if name == "pdist2" { 2 } else { 1 };
    if a.len() < offset {
        return Err(format!("{name}: missing Y"));
    }
    let y = if name == "pdist2" {
        real(e, name, &a[1])?
    } else {
        x.clone()
    };
    if x.cols() != y.cols() || x.cols() == 0 {
        return Err(format!(
            "{name}: data must have matching nonzero column counts"
        ));
    }
    let mut pos = offset;
    let mut metric = None;
    if let Some(v) = a.get(pos)
        && !text(v).is_some_and(|s| {
            s.eq_ignore_ascii_case("smallest") || s.eq_ignore_ascii_case("largest")
        })
    {
        metric = Some(v);
        pos += 1;
    }
    let param = if a.get(pos).is_some_and(|v| text(v).is_none()) {
        let v = &a[pos];
        pos += 1;
        Some(v)
    } else {
        None
    };
    let mut selection = None;
    for (key, v) in options(name, &a[pos..], &["smallest", "largest"])? {
        if name != "pdist2" || selection.is_some() {
            return Err(format!("{name}: invalid selection option"));
        }
        let k = integer(e, name, v)?;
        if k > x.rows() {
            return Err(format!("{name}: K exceeds X rows"));
        }
        selection = Some((k, key == "largest"));
    }
    if name == "pdist" {
        let n = condensed_len(e, name, x.rows())?;
        let metric = Metric::new(e, name, &x, metric, param)?;
        if metric.handle.is_none()
            && n.saturating_mul(x.cols()) >= 262_144
            && crate::parallel::current_worker_count() > 1
        {
            let cancel = &e.cancel;
            let m = x.rows();
            let offsets: Vec<_> = (0..m).map(|i| i * (2 * m - i - 1) / 2).collect();
            let mut result = vec![0.; n];
            result.par_chunks_mut(1024).enumerate().try_for_each(
                |(chunk, output)| -> Result<()> {
                    crate::dsp::check_cancel(cancel)?;
                    let start = chunk * 1024;
                    let mut i = offsets.partition_point(|&offset| offset <= start) - 1;
                    let mut j = i + 1 + start - offsets[i];
                    let mut xi = row(&x, i);
                    let mut xj = vec![0.; x.cols()];
                    for (index, distance) in output.iter_mut().enumerate() {
                        if index.is_multiple_of(64) {
                            crate::dsp::check_cancel(cancel)?;
                        }
                        for (c, v) in xj.iter_mut().enumerate() {
                            *v = x.data()[j + c * m];
                        }
                        *distance = metric.pair(&xi, &xj);
                        j += 1;
                        if j == m {
                            i += 1;
                            j = i + 1;
                            xi = row(&x, i);
                        }
                    }
                    Ok(())
                },
            )?;
            return Ok(vec![value(1, n, result)?]);
        }
        let mut result = Vec::with_capacity(n);
        for i in 0..x.rows().saturating_sub(1) {
            e.check_cancel()?;
            if metric.handle.is_none() {
                result.extend(metric.numeric_batch(&e.cancel, &row(&x, i), &x, i + 1)?);
                continue;
            }
            let source = &x;
            let tail = Matrix::from_column_vec(
                x.rows() - i - 1,
                x.cols(),
                (0..x.cols())
                    .flat_map(|c| {
                        (i + 1..source.rows()).map(move |r| source.data()[r + c * source.rows()])
                    })
                    .collect(),
            )?;
            result.extend(metric.batch(e, name, &row(&x, i), &tail, out)?);
        }
        return Ok(vec![value(1, n, result)?]);
    }
    let rows = selection.map_or(x.rows(), |(k, _)| k);
    // Even selected results preflight the pair count to bound work.
    size(e, name, x.rows(), y.rows())?;
    size(e, name, rows, y.rows())?;
    let metric = Metric::new(e, name, &x, metric, param)?;
    // The custom contract is distfun(XI,YJ), so preserve operand direction
    // even for a user function that is not symmetric.
    let custom = if metric.handle.is_some() {
        let mut full = vec![0.; x.rows() * y.rows()];
        for i in 0..x.rows() {
            e.check_cancel()?;
            let distances = metric.batch(e, name, &row(&x, i), &y, out)?;
            for (j, &distance) in distances.iter().enumerate() {
                full[i + j * x.rows()] = distance;
            }
        }
        Some(full)
    } else {
        None
    };
    let mut ds = vec![0.; rows * y.rows()];
    let mut indices = if selection.is_some() {
        vec![0.; rows * y.rows()]
    } else {
        vec![]
    };
    for j in 0..y.rows() {
        e.check_cancel()?;
        let distances = if let Some(full) = &custom {
            full[j * x.rows()..(j + 1) * x.rows()].to_vec()
        } else {
            metric.batch(e, name, &row(&y, j), &x, out)?
        };
        if let Some((k, largest)) = selection {
            let mut order: Vec<_> = (0..x.rows()).collect();
            sort_distances(&mut order, &distances, largest);
            for i in 0..k {
                ds[i + j * rows] = distances[order[i]];
                indices[i + j * rows] = (order[i] + 1) as f64;
            }
        } else {
            ds[j * rows..(j + 1) * rows].copy_from_slice(&distances);
        }
    }
    let mut result = vec![value(rows, y.rows(), ds)?];
    if selection.is_some() {
        result.push(value(rows, y.rows(), indices)?);
    }
    Ok(result)
}
pub(super) fn condensed_len(e: &Engine, name: &str, m: usize) -> Result<usize> {
    let n = m
        .checked_mul(m.saturating_sub(1))
        .map(|v| v / 2)
        .ok_or_else(|| format!("{name}: output too large"))?;
    size(e, name, 1, n)?;
    Ok(n)
}
pub(super) fn vector_order(n: usize) -> Result<usize> {
    let m = ((1. + (1. + 8. * n as f64).sqrt()) / 2.).round() as usize;
    if m.checked_mul(m.saturating_sub(1)).map(|v| v / 2) != Some(n) {
        return Err("squareform/linkage: invalid condensed vector length".into());
    }
    Ok(m)
}
pub(super) fn expand(e: &Engine, name: &str, v: &Matrix) -> Result<Matrix> {
    let m = vector_order(v.len())?;
    size(e, name, m, m)?;
    let mut result = vec![0.; m * m];
    let mut k = 0;
    for i in 0..m {
        for j in i + 1..m {
            result[i + j * m] = v.data()[k];
            result[j + i * m] = v.data()[k];
            k += 1;
        }
    }
    Matrix::from_column_vec(m, m, result)
}
fn squareform(e: &Engine, a: &[Value]) -> Result<Vec<Value>> {
    if !(1..=2).contains(&a.len()) {
        return Err("squareform: expected one array and optional direction".into());
    }
    let x = real(e, "squareform", &a[0])?;
    let direction = if a.len() == 2 {
        e.text_value(a[1].clone())?.to_lowercase()
    } else {
        String::new()
    };
    if !["", "tomatrix", "tovector"].contains(&direction.as_str()) {
        return Err("squareform: direction must be tomatrix or tovector".into());
    }
    let vector = x.is_empty() || x.rows() == 1 || x.cols() == 1;
    if direction == "tomatrix" || (direction.is_empty() && vector) {
        if !vector {
            return Err("squareform: tomatrix requires a vector".into());
        }
        return Ok(vec![Value::Matrix(expand(e, "squareform", &x)?)]);
    }
    if x.rows() != x.cols() {
        return Err("squareform: expected a square matrix".into());
    }
    let n = condensed_len(e, "squareform", x.rows())?;
    let mut data = Vec::with_capacity(n);
    for i in 0..x.rows() {
        e.check_cancel()?;
        if x.data()[i + i * x.rows()] != 0. {
            return Err("squareform: diagonal must be zero".into());
        }
        for j in i + 1..x.rows() {
            let v = x.data()[i + j * x.rows()];
            let w = x.data()[j + i * x.rows()];
            if v != w && !(v.is_nan() && w.is_nan()) {
                return Err("squareform: matrix must be symmetric".into());
            }
            data.push(v);
        }
    }
    Ok(vec![value(1, n, data)?])
}
fn sort_distances(order: &mut [usize], d: &[f64], largest: bool) {
    order.sort_by(|&i, &j| {
        let cmp = match (d[i].is_nan(), d[j].is_nan()) {
            (true, false) => std::cmp::Ordering::Greater,
            (false, true) => std::cmp::Ordering::Less,
            _ => {
                if largest {
                    d[j].total_cmp(&d[i])
                } else {
                    d[i].total_cmp(&d[j])
                }
            }
        };
        cmp.then(i.cmp(&j))
    });
}
fn knn(e: &mut Engine, a: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
    if a.len() < 2 {
        return Err("knnsearch: expected X and Y".into());
    }
    let x = real(e, "knnsearch", &a[0])?;
    let y = real(e, "knnsearch", &a[1])?;
    if x.cols() != y.cols() || x.cols() == 0 {
        return Err("knnsearch: X and Y columns must match".into());
    }
    let mut k = 1;
    let mut metric = None;
    let mut parameter = None;
    let mut ties = false;
    for (key, v) in options(
        "knnsearch",
        &a[2..],
        &[
            "k",
            "distance",
            "includeties",
            "sortindices",
            "p",
            "scale",
            "cov",
        ],
    )? {
        match key.as_str() {
            "k" => k = integer(e, "knnsearch", v)?,
            "distance" => metric = Some(v),
            "includeties" => ties = boolean(e, "knnsearch", v)?,
            "sortindices" => {
                boolean(e, "knnsearch", v)?;
            }
            _ => parameter = Some(v),
        }
    }
    let keep = clean_rows(e, "knnsearch", &x, None)?;
    if k > keep.len() {
        return Err("knnsearch: K exceeds complete X rows".into());
    }
    size(e, "knnsearch", x.rows(), y.rows())?;
    size(e, "knnsearch", k, y.rows())?;
    let metric = Metric::new(e, "knnsearch", &x, metric, parameter)?;
    if ties && y.rows() > crate::script_containers::MAX_CELL_SLOTS {
        return Err("knnsearch: cell output too large".into());
    }
    let mut idx = vec![f64::NAN; k * y.rows()];
    let mut dist = idx.clone();
    let mut icell = Vec::new();
    let mut dcell = Vec::new();
    for j in 0..y.rows() {
        e.check_cancel()?;
        let d = metric.batch(e, "knnsearch", &row(&y, j), &x, out)?;
        let mut order = keep.clone();
        sort_distances(&mut order, &d, false);
        if ties {
            let mut count = k;
            while count < order.len() && d[order[count]] == d[order[k - 1]] {
                count += 1;
            }
            if row(&y, j).iter().any(|v| v.is_nan()) {
                count = 0;
            }
            icell.push(Arc::new(Array::Numeric(Matrix::from_column_vec(
                1,
                count,
                order[..count].iter().map(|i| (i + 1) as f64).collect(),
            )?)));
            dcell.push(Arc::new(Array::Numeric(Matrix::from_column_vec(
                1,
                count,
                order[..count].iter().map(|&i| d[i]).collect(),
            )?)));
        } else if !row(&y, j).iter().any(|v| v.is_nan()) {
            for i in 0..k {
                idx[j + i * y.rows()] = (order[i] + 1) as f64;
                dist[j + i * y.rows()] = d[order[i]];
            }
        }
    }
    if ties {
        Ok(vec![
            Value::Container(Array::Cell(crate::CellArray::from_shared(
                y.rows(),
                1,
                icell,
                &e.cancel,
            )?)),
            Value::Container(Array::Cell(crate::CellArray::from_shared(
                y.rows(),
                1,
                dcell,
                &e.cancel,
            )?)),
        ])
    } else {
        Ok(vec![value(y.rows(), k, idx)?, value(y.rows(), k, dist)?])
    }
}
