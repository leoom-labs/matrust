//! cov, corrcoef, movmean, movsum, detrend, discretize, hist, issorted and
//! range (reference forms; the corpus is tests/fixtures/fn/stats-expected.txt).
use super::{Engine, Value};
use crate::{Array, LogicalMatrix, Matrix, Result, syntax::Expr};

const NAMES: &[&str] = &[
    "cov",
    "corrcoef",
    "movmean",
    "movsum",
    "detrend",
    "discretize",
    "hist",
    "issorted",
    "range",
];

/// Functions with more than one output: [n, c] = hist(...), [y, e] = discretize(...).
pub(super) fn has_outputs(name: &str) -> bool {
    matches!(name, "hist" | "discretize")
}

const HIST_PLOT: &str = "hist with no output arguments draws a histogram; plotting is not supported yet. Use n = hist(y) or [n, c] = hist(y) for the counts";

/// Moving windows up to this length are summed directly (best accuracy);
/// longer ones use a running sum.
const DIRECT_WINDOW: usize = 32;
const SCAN_CHUNK: usize = 4096;

/// Element pitch of one lane (row or column) of a column-major matrix.
struct Lanes {
    count: usize,
    len: usize,
    start: usize,
    step: usize,
}

fn lanes(rows: usize, cols: usize, dim: usize) -> Lanes {
    if dim == 1 {
        Lanes {
            count: cols,
            len: rows,
            start: rows,
            step: 1,
        }
    } else {
        Lanes {
            count: rows,
            len: cols,
            start: 1,
            step: rows,
        }
    }
}

fn first_dim(m: &Matrix) -> usize {
    if m.rows() != 1 { 1 } else { 2 }
}

/// MATLAB's eps(x): distance from |x| to the next larger double.
fn statsx_eps(x: f64) -> f64 {
    let a = x.abs();
    if !a.is_finite() {
        return f64::NAN;
    }
    f64::from_bits(a.to_bits() + 1) - a
}

fn lapy2(x: f64, y: f64) -> f64 {
    let w = x.abs().max(y.abs());
    let z = x.abs().min(y.abs());
    if z == 0. {
        w
    } else {
        w * (1. + (z / w) * (z / w)).sqrt()
    }
}

fn text_of(value: &Value) -> Option<String> {
    match value {
        Value::Text(text) => Some(text.clone()),
        _ => None,
    }
}

fn dimension(value: &Value, name: &str) -> Result<usize> {
    let Value::Matrix(m) = value else {
        return Err(format!("{name} dimension must be 1 or 2"));
    };
    let d = m.as_scalar()?;
    if d == 1. || d == 2. {
        Ok(d as usize)
    } else if d.is_finite() && d >= 3. && d.fract() == 0. {
        Err(format!(
            "{name} along dimension 3 or higher is not supported yet"
        ))
    } else {
        Err(format!("{name} dimension must be a positive integer"))
    }
}

impl Engine {
    pub(super) fn statsx_call(&self, name: &str, args: &[Value]) -> Result<Option<Value>> {
        if !NAMES.contains(&name) {
            return Ok(None);
        }
        Ok(Some(self.statsx_outputs(name, args, 1)?.remove(0)))
    }

    /// A bare `hist(y)` statement plots in MATLAB.
    pub(super) fn statsx_no_output(&mut self, expression: &Expr) -> Result<()> {
        if let Expr::Call(name, _) = expression
            && name == "hist"
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            return Err(HIST_PLOT.into());
        }
        Ok(())
    }

    pub(super) fn statsx_outputs(
        &self,
        name: &str,
        args: &[Value],
        count: usize,
    ) -> Result<Vec<Value>> {
        self.check_cancel()?;
        if name == "hist" && count == 0 {
            return Err(HIST_PLOT.into());
        }
        // Convert options with the shared helper, leaving the data argument's
        // character matrix shape intact (issorted supports character rows).
        let mut normalized = args[..args.len().min(1)].to_vec();
        normalized.extend(self.text_arguments(args.get(1..).unwrap_or_default().to_vec())?);
        let args = normalized.as_slice();
        let maximum = if has_outputs(name) { 2 } else { 1 };
        if count > maximum {
            return Err(format!("{name} does not support {count} outputs"));
        }
        let mut values = match name {
            "cov" => vec![self.statsx_cov(args)?],
            "corrcoef" => vec![self.statsx_corrcoef(args)?],
            "movmean" => vec![self.statsx_moving(args, true)?],
            "movsum" => vec![self.statsx_moving(args, false)?],
            "detrend" => vec![self.statsx_detrend(args)?],
            "discretize" => self.statsx_discretize(args)?,
            "hist" => self.statsx_hist(args)?,
            "issorted" => vec![self.statsx_issorted(args)?],
            "range" => vec![self.statsx_range(args)?],
            _ => unreachable!("checked by NAMES"),
        };
        values.truncate(count.max(1));
        Ok(values)
    }

    fn statsx_budget(&self, elements: usize) -> Result<()> {
        if self
            .evaluated_elements
            .checked_add(elements)
            .is_none_or(|n| n > 32 * 1024 * 1024)
        {
            return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
        }
        Ok(())
    }

    fn statsx_poll(&self, index: usize) -> Result<()> {
        if index.is_multiple_of(SCAN_CHUNK) {
            #[cfg(test)]
            tests::cancel_after_progress(self, index);
            self.check_cancel()?;
        }
        Ok(())
    }

    fn statsx_sum(&self, values: impl Iterator<Item = f64>) -> Result<f64> {
        let mut sum = 0.;
        for (i, value) in values.enumerate() {
            self.statsx_poll(i)?;
            sum += value;
        }
        Ok(sum)
    }

    fn statsx_collect(&self, values: impl Iterator<Item = f64>) -> Result<Vec<f64>> {
        let mut out = Vec::with_capacity(values.size_hint().0);
        for (i, value) in values.enumerate() {
            self.statsx_poll(i)?;
            out.push(value);
        }
        Ok(out)
    }

    /// Real double data of a numeric or logical argument.
    fn statsx_real(&self, value: &Value, name: &str) -> Result<Matrix> {
        match value {
            Value::Matrix(m) => Ok(m.clone()),
            Value::Logical(l) => Matrix::from_column_vec(
                l.rows(),
                l.cols(),
                self.statsx_collect(l.data().iter().map(|&b| f64::from(b)))?,
            ),
            Value::Complex(_) => Err(format!("{name} of complex input is not supported yet")),
            _ => Err(format!("{name} requires a real numeric or logical array")),
        }
    }

    fn statsx_cov_matrix(&self, x: &Matrix, normalize_by_n: bool) -> Result<Matrix> {
        let (m, n) = (x.rows(), x.cols());
        if m == 0 && n == 0 {
            return Ok(Matrix::scalar(f64::NAN));
        }
        self.statsx_budget(Matrix::checked_len(n, n)?)?;
        if m == 0 {
            return Matrix::filled(n, n, f64::NAN);
        }
        let data = x.data();
        let mut centered = Vec::with_capacity(data.len());
        for j in 0..n {
            self.check_cancel()?;
            let column = &data[j * m..(j + 1) * m];
            let mean = column.iter().sum::<f64>() / m as f64;
            centered.extend(column.iter().map(|v| v - mean));
        }
        let denominator = if normalize_by_n || m == 1 {
            m as f64
        } else {
            (m - 1) as f64
        };
        let mut c = vec![0.; n * n];
        for j in 0..n {
            self.check_cancel()?;
            let b = &centered[j * m..(j + 1) * m];
            for i in 0..=j {
                let a = &centered[i * m..(i + 1) * m];
                let v = a.iter().zip(b).map(|(p, q)| p * q).sum::<f64>() / denominator;
                c[i + j * n] = v;
                c[j + i * n] = v;
            }
        }
        Matrix::from_column_vec(n, n, c)
    }

    /// x as observation columns: cov(x) or cov(x, y) data.
    fn statsx_observations(&self, args: &[Value], name: &str) -> Result<Matrix> {
        let x = self.statsx_real(&args[0], name)?;
        if let Some(y) = args.get(1) {
            let y = self.statsx_real(y, name)?;
            if x.len() != y.len() {
                return Err(format!(
                    "{name}: the number of elements in x and y must match"
                ));
            }
            let mut data = x.data().to_vec();
            data.extend_from_slice(y.data());
            return Matrix::from_column_vec(x.len(), 2, data);
        }
        if x.rows() == 1 {
            return Ok(x.transpose());
        }
        Ok(x)
    }

    fn statsx_cov(&self, args: &[Value]) -> Result<Value> {
        if args.is_empty() || args.len() > 3 {
            return Err("Use cov(x), cov(x, y), cov(x, flag) or cov(x, y, flag)".into());
        }
        let mut data = args;
        let mut by_n = false;
        let flag_last = args.len() == 3
            || (args.len() == 2 && matches!(&args[1], Value::Matrix(m) if m.len() == 1));
        if flag_last {
            let Value::Matrix(flag) = &args[args.len() - 1] else {
                return Err("cov normalization flag must be 0 or 1".into());
            };
            by_n = match flag.as_scalar()? {
                0. => false,
                1. => true,
                _ => return Err("cov normalization flag must be 0 or 1".into()),
            };
            data = &args[..args.len() - 1];
        }
        let x = self.statsx_observations(data, "cov")?;
        Ok(Value::Matrix(self.statsx_cov_matrix(&x, by_n)?))
    }

    fn statsx_corrcoef(&self, args: &[Value]) -> Result<Value> {
        if args.is_empty() || args.len() > 2 || args.iter().any(|a| matches!(a, Value::Text(_))) {
            return Err("Use corrcoef(x) or corrcoef(x, y); the 'alpha' and 'rows' options and the p-value outputs are not supported yet".into());
        }
        let x = self.statsx_observations(args, "corrcoef")?;
        let c = self.statsx_cov_matrix(&x, false)?;
        let n = c.rows();
        if c.cols() != n || x.is_empty() {
            return Ok(Value::Matrix(c));
        }
        let d: Vec<f64> = (0..n).map(|i| c.get(i, i).sqrt()).collect();
        let mut r = c.data().to_vec();
        for j in 0..n {
            for i in 0..n {
                let mut v = r[i + j * n] / d[i] / d[j];
                if v.abs() > 1. {
                    v /= v.abs();
                }
                if i == j {
                    v = if v.is_nan() {
                        v
                    } else {
                        v.signum() * f64::from(v != 0.)
                    };
                }
                r[i + j * n] = v;
            }
        }
        Ok(Value::Matrix(Matrix::from_column_vec(n, n, r)?))
    }

    fn statsx_moving(&self, args: &[Value], mean: bool) -> Result<Value> {
        let name = if mean { "movmean" } else { "movsum" };
        if args.len() < 2 {
            return Err(format!(
                "Use {name}(A, k[, dim][, nanflag][, 'Endpoints', e])"
            ));
        }
        let x = self.statsx_real(&args[0], name)?;
        let ArxMovingOptions {
            before,
            after,
            dim,
            omit_nan,
            fill,
            discard,
        } = arx_moving_options(name, args, first_dim(&x), false)?;
        let (rows, cols) = (x.rows(), x.cols());
        let layout = lanes(rows, cols, dim);
        let n = layout.len;
        let kept = arx_window_kept(before, after, n, discard);
        let (out_rows, out_cols) = if dim == 1 { (kept, cols) } else { (rows, kept) };
        self.statsx_budget(out_rows * out_cols)?;
        let data = x.data();
        let mut out = vec![0.; out_rows * out_cols];
        let out_lanes = lanes(out_rows, out_cols, dim);
        let width = before.min(n).saturating_add(after.min(n)).saturating_add(1);
        let mut lane = vec![0.; n];
        let mut result = vec![0.; n];
        for l in 0..layout.count {
            self.check_cancel()?;
            let base = if dim == 1 { l * layout.start } else { l };
            for (i, v) in lane.iter_mut().enumerate() {
                self.statsx_poll(i)?;
                *v = data[base + i * layout.step];
            }
            let window = MovingWindow {
                before,
                after,
                omit_nan,
                mean,
                fill,
            };
            if width <= DIRECT_WINDOW {
                moving_direct(self, &lane, &window, &mut result)?;
            } else {
                moving_running(self, &lane, &window, &mut result)?;
            }
            let base_out = if dim == 1 { l * out_lanes.start } else { l };
            let mut k = 0;
            for (i, &v) in result.iter().enumerate() {
                self.statsx_poll(i)?;
                let full = i >= before && i.checked_add(after).is_some_and(|e| e < n);
                if discard && !full {
                    continue;
                }
                out[base_out + k * out_lanes.step] = v;
                k += 1;
            }
        }
        Ok(Value::Matrix(Matrix::from_column_vec(
            out_rows, out_cols, out,
        )?))
    }

    fn statsx_detrend(&self, args: &[Value]) -> Result<Value> {
        if args.is_empty() || args.len() > 2 {
            return Err(
                "Use detrend(x), detrend(x, 'linear') or detrend(x, 'constant'); breakpoints are not supported yet"
                    .into(),
            );
        }
        let linear = match args.get(1) {
            None => true,
            Some(Value::Text(t)) => match t.to_ascii_lowercase().as_str() {
                "linear" | "l" => true,
                "constant" | "c" => false,
                _ => return Err("detrend type must be 'linear' or 'constant'".into()),
            },
            Some(Value::Matrix(m)) if m.len() == 1 && (m.get(0, 0) == 0. || m.get(0, 0) == 1.) => {
                m.get(0, 0) == 1.
            }
            Some(_) => return Err("detrend type must be 'linear', 'constant', 0 or 1".into()),
        };
        let x = self.statsx_real(&args[0], "detrend")?;
        self.statsx_budget(x.len())?;
        let row = x.rows() == 1;
        let (m, n) = if row {
            (x.cols(), 1)
        } else {
            (x.rows(), x.cols())
        };
        if x.is_empty() {
            return Ok(Value::Matrix(x));
        }
        let data = x.data();
        let mut out = Vec::with_capacity(data.len());
        if !linear || m == 1 {
            for j in 0..n {
                self.check_cancel()?;
                let column = &data[j * m..(j + 1) * m];
                let mean = self.statsx_sum(column.iter().copied())? / m as f64;
                for (i, value) in column.iter().enumerate() {
                    self.statsx_poll(i)?;
                    out.push(value - mean);
                }
            }
        } else {
            let fit = LineFit::new(self, m)?;
            for j in 0..n {
                self.check_cancel()?;
                fit.residuals(self, &data[j * m..(j + 1) * m], &mut out)?;
            }
        }
        Ok(Value::Matrix(Matrix::from_column_vec(
            x.rows(),
            x.cols(),
            out,
        )?))
    }

    fn statsx_discretize(&self, args: &[Value]) -> Result<Vec<Value>> {
        let usage = "Use discretize(x, edges[, values][, 'IncludedEdge', 'left'|'right'])";
        if args.len() < 2 {
            return Err(usage.into());
        }
        let x = self.statsx_real(&args[0], "discretize")?;
        let edges_value = self.statsx_real(&args[1], "discretize")?;
        let edges = edges_value.data();
        if edges.len() == 1 {
            return Err(
                "discretize with a number of bins is not supported yet; pass the bin edges".into(),
            );
        }
        if edges.len() < 2 || (edges_value.rows() != 1 && edges_value.cols() != 1) {
            return Err("discretize edges must be a vector with at least two elements".into());
        }
        if edges.iter().any(|e| e.is_nan()) || edges.windows(2).any(|w| w[0] > w[1]) {
            return Err("discretize edges must be monotonically nondecreasing".into());
        }
        let mut at = 2;
        let mut values = None;
        if let Some(v) = args.get(at)
            && !matches!(v, Value::Text(_))
        {
            let v = self.statsx_real(v, "discretize")?;
            if v.len() != edges.len() - 1 {
                return Err(
                    "discretize values must have one element per bin (numel(edges) - 1)".into(),
                );
            }
            values = Some(v);
            at += 1;
        }
        let mut right = false;
        while at < args.len() {
            let flag = text_of(&args[at]).unwrap_or_default().to_ascii_lowercase();
            match flag.as_str() {
                "includededge" => {
                    right = match args.get(at + 1).and_then(text_of) {
                        Some(e) if e.eq_ignore_ascii_case("left") => false,
                        Some(e) if e.eq_ignore_ascii_case("right") => true,
                        _ => {
                            return Err(
                                "discretize 'IncludedEdge' must be 'left' or 'right'".into()
                            );
                        }
                    };
                    at += 2;
                }
                "categorical" => {
                    return Err("discretize categorical output is not supported yet".into());
                }
                _ => return Err(usage.into()),
            }
        }
        self.statsx_budget(x.len() + edges.len())?;
        let last = edges.len() - 1;
        let mut out = Vec::with_capacity(x.len());
        for (i, &v) in x.data().iter().enumerate() {
            if i % 65536 == 0 {
                self.check_cancel()?;
            }
            let bin = if v.is_nan() {
                None
            } else if right {
                // (e_i, e_i+1], the first bin also holds e_1.
                let below = edges.partition_point(|&e| e < v);
                match below {
                    0 if v == edges[0] => Some(1),
                    b if (1..=last).contains(&b) => Some(b),
                    _ => None,
                }
            } else {
                // [e_i, e_i+1), the last bin also holds e_end.
                let upto = edges.partition_point(|&e| e <= v);
                match upto {
                    u if u == last + 1 && v == edges[last] => Some(last),
                    u if (1..=last).contains(&u) => Some(u),
                    _ => None,
                }
            };
            out.push(match (bin, &values) {
                (None, _) => f64::NAN,
                (Some(b), None) => b as f64,
                (Some(b), Some(values)) => values.data()[b - 1],
            });
        }
        Ok(vec![
            Value::Matrix(Matrix::from_column_vec(x.rows(), x.cols(), out)?),
            Value::Matrix(edges_value),
        ])
    }

    fn statsx_hist(&self, args: &[Value]) -> Result<Vec<Value>> {
        if args.is_empty() || args.len() > 2 {
            return Err("Use n = hist(y), hist(y, nbins) or hist(y, centers); [n, c] = hist(...) returns the bin centers".into());
        }
        let y = self.statsx_real(&args[0], "hist")?;
        let spec = match args.get(1) {
            None => Matrix::scalar(10.),
            Some(v) => self.statsx_real(v, "hist")?,
        };
        let spec_data = spec.data();
        if spec_data.is_empty() {
            return Err(
                "hist bins must be a positive integer or a vector of finite centers".into(),
            );
        }
        for (i, v) in spec_data.iter().enumerate() {
            self.statsx_poll(i)?;
            if !v.is_finite() {
                return Err(
                    "hist bins must be a positive integer or a vector of finite centers".into(),
                );
            }
        }
        let bins = if let [k] = spec_data {
            if *k < 1. || k.fract() != 0. || *k > 1e7 {
                return Err("hist number of bins must be a positive integer".into());
            }
            *k as usize
        } else {
            for (i, w) in spec_data.windows(2).enumerate() {
                self.statsx_poll(i)?;
                if w[0] > w[1] {
                    return Err("hist centers must be monotonically nondecreasing".into());
                }
            }
            spec_data.len()
        };
        let vector = y.rows().min(y.cols()) <= 1;
        let (m, n) = if vector {
            (y.len(), 1)
        } else {
            (y.rows(), y.cols())
        };
        self.statsx_budget(bins.checked_mul(n + 2).ok_or("hist result is too large")?)?;
        let row = |data: Vec<f64>| Matrix::from_column_vec(1, data.len(), data);
        if y.is_empty() {
            let centers = if spec_data.len() == 1 {
                self.statsx_collect((1..=bins).map(|i| i as f64))?
            } else {
                self.statsx_collect(spec_data.iter().copied())?
            };
            return Ok(vec![
                Value::Matrix(row(vec![0.; bins])?),
                Value::Matrix(row(centers)?),
            ]);
        }
        let (mut low, mut high) = (f64::NAN, f64::NAN);
        for (i, &v) in y.data().iter().enumerate() {
            self.statsx_poll(i)?;
            if !v.is_nan() {
                if low.is_nan() || v < low {
                    low = v;
                }
                if high.is_nan() || v > high {
                    high = v;
                }
            }
        }
        let (centers, edges) = if spec_data.len() == 1 {
            if low == high {
                low = low - (bins / 2) as f64 - 0.5;
                high = high + bins.div_ceil(2) as f64 - 0.5;
            }
            let width = (high - low) / bins as f64;
            let mut edges = self.statsx_collect((0..=bins).map(|k| low + width * k as f64))?;
            edges[bins] = high;
            let centers = self.statsx_collect(edges[..bins].iter().map(|e| e + width / 2.))?;
            (centers, edges)
        } else {
            let c = spec_data;
            let width = self.statsx_collect(c.windows(2).map(|w| w[1] - w[0]).chain([0.]))?;
            let mut edges = Vec::with_capacity(bins + 1);
            edges.push(c[0] - width[0] / 2.);
            for (i, (c, w)) in c.iter().zip(&width).enumerate() {
                self.statsx_poll(i)?;
                edges.push(c + w / 2.);
            }
            edges[0] = edges[0].min(low);
            edges[bins] = edges[bins].max(high);
            (self.statsx_collect(c.iter().copied())?, edges)
        };
        // Intervals ( ] as in the reference: each edge moves up by eps(edge).
        let shifted = self.statsx_collect(edges.iter().map(|&e| e + statsx_eps(e)))?;
        let top = shifted[bins];
        let mut counts = vec![0.; bins * n];
        for j in 0..n {
            self.check_cancel()?;
            for (i, &v) in y.data()[j * m..(j + 1) * m].iter().enumerate() {
                self.statsx_poll(i)?;
                if v.is_nan() || v > top {
                    continue;
                }
                let at = shifted.partition_point(|&e| e <= v).clamp(1, bins);
                counts[j * bins + at - 1] += 1.;
            }
        }
        let counts = if vector {
            row(counts)?
        } else {
            Matrix::from_column_vec(bins, n, counts)?
        };
        let centers = if vector {
            row(centers)?
        } else {
            Matrix::from_column_vec(bins, 1, centers)?
        };
        Ok(vec![Value::Matrix(counts), Value::Matrix(centers)])
    }

    fn statsx_issorted(&self, args: &[Value]) -> Result<Value> {
        let rows = match args {
            [_] => false,
            [_, Value::Text(flag)] if flag.eq_ignore_ascii_case("rows") => true,
            _ => {
                return Err(
                    "Use issorted(A) or issorted(A, 'rows'); other options are not supported yet"
                        .into(),
                );
            }
        };
        let sorted = match &args[0] {
            Value::Container(Array::Cell(cell)) => {
                if rows {
                    return Err("DIM argument not supported for cell arrays.".into());
                }
                if cell.rows() > 1 && cell.cols() > 1 {
                    return Err("Input must be a vector or 'rows' must be specified.".into());
                }
                let mut texts = Vec::with_capacity(cell.len());
                for i in 0..cell.len() {
                    self.statsx_poll(i)?;
                    match cell.get_linear(i)?.as_ref() {
                        Array::Character(c) if c.rows() <= 1 => texts.push(c.clone()),
                        _ => {
                            return Err(
                                "issorted cell input must be a cell array of character vectors"
                                    .into(),
                            );
                        }
                    }
                }
                let mut sorted = true;
                for (i, w) in texts.windows(2).enumerate() {
                    self.statsx_poll(i)?;
                    let (a, b) = (w[0].units(), w[1].units());
                    let mut order = a.len().cmp(&b.len());
                    for (a, b) in a.chunks(SCAN_CHUNK).zip(b.chunks(SCAN_CHUNK)) {
                        self.check_cancel()?;
                        let common = a.len().min(b.len());
                        let compared = a[..common].cmp(&b[..common]);
                        if !compared.is_eq() {
                            order = compared;
                            break;
                        }
                    }
                    if order.is_gt() {
                        sorted = false;
                        break;
                    }
                }
                sorted
            }
            value => {
                let x = match value {
                    Value::Text(text) => {
                        let units = self.statsx_collect(text.encode_utf16().map(f64::from))?;
                        Matrix::from_column_vec(1, units.len(), units)?
                    }
                    Value::Container(Array::Character(c)) => Matrix::from_column_vec(
                        c.rows(),
                        c.cols(),
                        self.statsx_collect(c.units().iter().map(|&u| f64::from(u)))?,
                    )?,
                    other => self.statsx_real(other, "issorted")?,
                };
                let ordered = |a: f64, b: f64| b.is_nan() || (!a.is_nan() && a <= b);
                if rows {
                    let (m, n) = (x.rows(), x.cols());
                    let mut sorted = true;
                    'rows: for i in 1..m {
                        self.statsx_poll(i)?;
                        for j in 0..n {
                            self.statsx_poll(j)?;
                            let (a, b) = (x.get(i - 1, j), x.get(i, j));
                            if a == b || (a.is_nan() && b.is_nan()) {
                                continue;
                            }
                            if !ordered(a, b) {
                                sorted = false;
                                break 'rows;
                            }
                            break;
                        }
                    }
                    sorted
                } else {
                    if x.rows() > 1 && x.cols() > 1 {
                        return Err("Input must be a vector or 'rows' must be specified.".into());
                    }
                    let mut sorted = true;
                    for (i, w) in x.data().windows(2).enumerate() {
                        self.statsx_poll(i)?;
                        if !ordered(w[0], w[1]) {
                            sorted = false;
                            break;
                        }
                    }
                    sorted
                }
            }
        };
        Ok(Value::Logical(LogicalMatrix::scalar(sorted)))
    }

    fn statsx_range(&self, args: &[Value]) -> Result<Value> {
        if args.is_empty() || args.len() > 2 {
            return Err("Use range(X) or range(X, dim)".into());
        }
        let x = Value::Matrix(self.statsx_real(&args[0], "range")?);
        let mut call = vec![x];
        if let Some(dim) = args.get(1) {
            dimension(dim, "range")?;
            call.push(Value::Matrix(Matrix::filled(0, 0, 0.)?));
            call.push(dim.clone());
        }
        let high = self.reduction_call("max", &call, 1, [true, false, false])?;
        let low = self.reduction_call("min", &call, 1, [true, false, false])?;
        let high = self.statsx_real(&high[0], "range")?;
        let low = self.statsx_real(&low[0], "range")?;
        Ok(Value::Matrix(high.zip(&low, |a, b| a - b)?))
    }
}

/// Shared window and endpoint parser for all seven moving statistics.
/// Keep the existing movmean/movsum path and the additional reducers in sync.
pub(super) struct ArxMovingOptions {
    pub(super) before: usize,
    pub(super) after: usize,
    pub(super) dim: usize,
    pub(super) omit_nan: bool,
    pub(super) fill: Option<f64>,
    pub(super) discard: bool,
}

/// Shared endpoint geometry; reducers differ only in how they aggregate the
/// admitted samples and the (possibly weighted) missing endpoint samples.
pub(super) fn arx_window_range(
    before: usize,
    after: usize,
    index: usize,
    len: usize,
) -> std::ops::Range<usize> {
    index.saturating_sub(before)..index.saturating_add(after).saturating_add(1).min(len)
}

pub(super) fn arx_window_kept(before: usize, after: usize, len: usize, discard: bool) -> usize {
    if discard {
        len.saturating_sub(before.saturating_add(after))
    } else {
        len
    }
}

pub(super) fn arx_window_missing(before: usize, after: usize, present: usize) -> usize {
    before + after + 1 - present
}

pub(super) fn arx_moving_options(
    name: &str,
    args: &[Value],
    default_dim: usize,
    default_omit: bool,
) -> Result<ArxMovingOptions> {
    let Value::Matrix(window) = &args[1] else {
        return Err(format!("{name} window must be numeric"));
    };
    let integer = |v: f64| v.is_finite() && v >= 0. && v.fract() == 0. && v <= 1e15;
    let (before, after) = match window.data() {
        [k] if integer(*k) && *k >= 1. => {
            let k = *k as usize;
            (k / 2, (k - 1) / 2)
        }
        [b, f] if integer(*b) && integer(*f) => (*b as usize, *f as usize),
        [_] | [_, _] => {
            return Err(if matches!(name, "movmean" | "movsum") {
                format!(
                    "{name} window must be a positive integer k or nonnegative integers [kb kf]; non-integer windows are not supported yet"
                )
            } else {
                "Window length must be a finite scalar positive integer or 2-element vector of finite nonnegative integers.".into()
            });
        }
        _ => {
            return Err(format!(
                "{name} window must be a scalar or a two-element vector"
            ));
        }
    };
    let mut dim = default_dim;
    let mut at = 2;
    if let Some(value @ Value::Matrix(_)) = args.get(at) {
        dim = dimension(value, name)?;
        at += 1;
    }
    let mut omit_nan = default_omit;
    // None: shrink; Some(v): fill with v; discard handled by `discard`.
    let mut fill = None;
    let mut discard = false;
    while at < args.len() {
        let flag = text_of(&args[at])
            .ok_or_else(|| format!("{name} expects an option name at argument {}", at + 1))?
            .to_ascii_lowercase();
        match flag.as_str() {
            "omitnan" => omit_nan = true,
            "includenan" => omit_nan = false,
            "endpoints" => {
                let value = args
                    .get(at + 1)
                    .ok_or_else(|| format!("{name} 'Endpoints' needs a value"))?;
                match value {
                    Value::Text(e) => match e.to_ascii_lowercase().as_str() {
                        "shrink" => (fill, discard) = (None, false),
                        "discard" => (fill, discard) = (None, true),
                        "fill" => (fill, discard) = (Some(f64::NAN), false),
                        _ => {
                            return Err(format!(
                                "{name} 'Endpoints' must be 'shrink', 'discard', 'fill' or a numeric scalar"
                            ));
                        }
                    },
                    Value::Matrix(v) if v.len() == 1 => {
                        (fill, discard) = (Some(v.as_scalar()?), false)
                    }
                    Value::Logical(v) if v.len() == 1 => {
                        (fill, discard) = (Some(f64::from(v.get(0, 0))), false)
                    }
                    _ => {
                        return Err(format!(
                            "{name} 'Endpoints' must be 'shrink', 'discard', 'fill' or a numeric scalar"
                        ));
                    }
                }
                at += 1;
            }
            "samplepoints" => {
                return Err(format!("{name} 'SamplePoints' is not supported yet"));
            }
            _ => return Err(format!("{name} does not recognize the option '{flag}'")),
        }
        at += 1;
    }
    Ok(ArxMovingOptions {
        before,
        after,
        dim,
        omit_nan,
        fill,
        discard,
    })
}

struct MovingWindow {
    before: usize,
    after: usize,
    omit_nan: bool,
    mean: bool,
    fill: Option<f64>,
}

impl MovingWindow {
    fn finish(&self, mut sum: f64, mut count: usize, present: usize) -> f64 {
        if let Some(fill) = self.fill {
            let missing = arx_window_missing(self.before, self.after, present);
            if missing > 0 && !(self.omit_nan && fill.is_nan()) {
                sum += fill * missing as f64;
                count += missing;
            }
        }
        if self.mean { sum / count as f64 } else { sum }
    }
}

/// One output per element: the window [i - before, i + after] clipped to the lane.
fn moving_direct(
    engine: &Engine,
    lane: &[f64],
    window: &MovingWindow,
    out: &mut [f64],
) -> Result<()> {
    let n = lane.len();
    for (i, slot) in out.iter_mut().enumerate() {
        // At most DIRECT_WINDOW additions per output, so bound work between
        // polls as well as the number of output elements.
        engine.statsx_poll(i * DIRECT_WINDOW)?;
        let range = arx_window_range(window.before, window.after, i, n);
        let mut sum = 0.;
        let mut count = 0usize;
        for &v in &lane[range.clone()] {
            if window.omit_nan && v.is_nan() {
                continue;
            }
            sum += v;
            count += 1;
        }
        *slot = window.finish(sum, count, range.len());
    }
    Ok(())
}

/// Compensated running sums retain small terms when a large sample leaves.
/// Rebuild once per window-width to bound accumulated roundoff in long lanes.
/// If finite arithmetic overflows, recompute the current window until it
/// becomes finite again, so an old overflow cannot poison subsequent windows.
fn moving_running(
    engine: &Engine,
    lane: &[f64],
    window: &MovingWindow,
    out: &mut [f64],
) -> Result<()> {
    #[derive(Default)]
    struct Sum {
        sum: f64,
        correction: f64,
        finite: usize,
        nan: usize,
        plus: usize,
        minus: usize,
    }
    impl Sum {
        fn add(&mut self, v: f64, sign: isize) {
            let counter = if v.is_nan() {
                &mut self.nan
            } else if v == f64::INFINITY {
                &mut self.plus
            } else if v == f64::NEG_INFINITY {
                &mut self.minus
            } else {
                let v = sign as f64 * v;
                let next = self.sum + v;
                if next.is_finite() {
                    // Neumaier compensation works for both additions and
                    // removals, including cancellation of the largest term.
                    self.correction += if self.sum.abs() >= v.abs() {
                        (self.sum - next) + v
                    } else {
                        (v - next) + self.sum
                    };
                } else {
                    self.correction = 0.;
                }
                self.sum = next;
                &mut self.finite
            };
            *counter = counter.wrapping_add_signed(sign);
        }
    }
    let n = lane.len();
    let period = (window.before + window.after + 1).min(n).max(1);
    let mut w = Sum::default();
    let mut hi_next = 0;
    let mut lo = 0;
    for (i, slot) in out.iter_mut().enumerate() {
        engine.statsx_poll(i)?;
        let range = arx_window_range(window.before, window.after, i, n);
        let hi = range.end - 1;
        let new_lo = range.start;
        if i.is_multiple_of(period) || !(w.sum + w.correction).is_finite() {
            w = Sum::default();
            lo = new_lo;
            hi_next = new_lo;
        }
        while lo < new_lo {
            engine.statsx_poll(lo)?;
            w.add(lane[lo], -1);
            lo += 1;
        }
        while hi_next <= hi {
            engine.statsx_poll(hi_next)?;
            w.add(lane[hi_next], 1);
            hi_next += 1;
        }
        if w.finite == 0 {
            w.sum = 0.;
            w.correction = 0.;
        }
        let count = w.finite + w.plus + w.minus + if window.omit_nan { 0 } else { w.nan };
        let sum = if (w.nan > 0 && !window.omit_nan) || (w.plus > 0 && w.minus > 0) {
            f64::NAN
        } else if w.plus > 0 {
            f64::INFINITY
        } else if w.minus > 0 {
            f64::NEG_INFINITY
        } else {
            w.sum + w.correction
        };
        *slot = window.finish(sum, count, hi - lo + 1);
    }
    Ok(())
}

/// Least-squares line through each column against two regressors (the
/// sample index k/N, k = 1..N, and ones): Householder QR with column pivoting, a
/// nonnegative diagonal and the blocked form of Q'b. This order of operations
/// reproduces the reference's rounding for detrend([3 3 3]) (corpus case 31).
struct LineFit {
    a: [Vec<f64>; 2],
    /// Pivoted column order: `order[k]` is the regressor column of step k.
    order: [usize; 2],
    v: [Vec<f64>; 2],
    tau: [f64; 2],
    r: [[f64; 2]; 2],
    t12: f64,
}

impl LineFit {
    fn new(engine: &Engine, n: usize) -> Result<Self> {
        let ramp = engine.statsx_collect((1..=n).map(|i| i as f64 / n as f64))?;
        let ones = vec![1.; n];
        let norm = |c: &[f64]| engine.statsx_sum(c.iter().map(|v| v * v)).map(f64::sqrt);
        let order = if norm(&ones)? > norm(&ramp)? {
            [1, 0]
        } else {
            [0, 1]
        };
        let a = [ramp, ones];
        let mut work = [
            engine.statsx_collect(a[order[0]].iter().copied())?,
            engine.statsx_collect(a[order[1]].iter().copied())?,
        ];
        let mut v = [vec![0.; n], vec![0.; n]];
        let mut tau = [0.; 2];
        let mut r = [[0.; 2]; 2];
        for k in 0..2.min(n) {
            let alpha = work[k][k];
            let xnorm = norm(&work[k][k + 1..])?;
            let beta = lapy2(alpha, xnorm);
            v[k][k] = 1.;
            if alpha != beta {
                tau[k] = (beta - alpha) / beta;
                let scale = 1. / (alpha - beta);
                for i in k + 1..n {
                    engine.statsx_poll(i)?;
                    v[k][i] = work[k][i] * scale;
                }
            }
            work[k][k] = beta;
            for column in work.iter_mut().skip(k + 1) {
                let mut w = 0.;
                for i in k..n {
                    engine.statsx_poll(i)?;
                    w += v[k][i] * column[i];
                }
                for i in k..n {
                    engine.statsx_poll(i)?;
                    column[i] -= tau[k] * v[k][i] * w;
                }
            }
            for (j, column) in work.iter().enumerate() {
                r[k][j] = column[k];
            }
        }
        let dot = engine.statsx_sum(v[0].iter().zip(&v[1]).map(|(p, q)| p * q))?;
        let t12 = -tau[0] * dot * tau[1];
        Ok(Self {
            a,
            order,
            v,
            tau,
            r,
            t12,
        })
    }

    fn residuals(&self, engine: &Engine, x: &[f64], out: &mut Vec<f64>) -> Result<()> {
        let dot = |v: &[f64]| engine.statsx_sum(v.iter().zip(x).map(|(p, q)| p * q));
        let (w1, w2) = (dot(&self.v[0])?, dot(&self.v[1])?);
        let z1 = self.tau[0] * w1;
        let z2 = self.t12 * w1 + self.tau[1] * w2;
        let b0 = x[0] - (self.v[0][0] * z1 + self.v[1][0] * z2);
        let b1 = x[1] - (self.v[0][1] * z1 + self.v[1][1] * z2);
        let c1 = b1 / self.r[1][1];
        let c0 = (b0 - self.r[0][1] * c1) / self.r[0][0];
        let mut coef = [0.; 2];
        coef[self.order[0]] = c0;
        coef[self.order[1]] = c1;
        for (i, &value) in x.iter().enumerate() {
            engine.statsx_poll(i)?;
            out.push(value - (self.a[0][i] * coef[0] + self.a[1][i] * coef[1]));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, sync::atomic::Ordering};

    thread_local! {
        static CANCEL_AFTER_PROGRESS: Cell<bool> = const { Cell::new(false) };
    }

    // Deterministic cancellation after at least one full chunk of scan work;
    // no sleeping, scheduler assumptions, or hooks in release builds.
    pub(super) fn cancel_after_progress(engine: &Engine, index: usize) {
        if index > 0 && CANCEL_AFTER_PROGRESS.replace(false) {
            engine.cancel.store(true, Ordering::Relaxed);
        }
    }

    fn cancels_during_scan(work: impl FnOnce(&Engine) -> Result<()>) {
        let engine = Engine::default();
        CANCEL_AFTER_PROGRESS.set(true);
        let result = work(&engine);
        CANCEL_AFTER_PROGRESS.set(false);
        assert_eq!(result.unwrap_err(), "Calculation cancelled");
        assert!(engine.cancel.load(Ordering::Relaxed));
    }

    #[test]
    fn moving_kernels_cancel_inside_one_lane() {
        let lane = vec![1.; 3 * SCAN_CHUNK];
        for width in [31, 41, 2 * SCAN_CHUNK + 1] {
            let window = MovingWindow {
                before: width / 2,
                after: width / 2,
                omit_nan: false,
                mean: false,
                fill: None,
            };
            let mut out = vec![0.; lane.len()];
            cancels_during_scan(|engine| {
                if width <= DIRECT_WINDOW {
                    moving_direct(engine, &lane, &window, &mut out)
                } else {
                    moving_running(engine, &lane, &window, &mut out)
                }
            });
        }
    }

    #[test]
    fn histogram_detrend_and_sorted_scans_cancel_after_progress() {
        let x = Value::Matrix(Matrix::filled(3 * SCAN_CHUNK, 1, 1.).unwrap());
        cancels_during_scan(|engine| engine.statsx_hist(std::slice::from_ref(&x)).map(|_| ()));
        for kind in ["constant", "linear"] {
            cancels_during_scan(|engine| {
                engine
                    .statsx_detrend(&[x.clone(), Value::Text(kind.into())])
                    .map(|_| ())
            });
        }
        // Exercise applying the QR fit, separately from constructing it.
        let fit = LineFit::new(&Engine::default(), 3 * SCAN_CHUNK).unwrap();
        cancels_during_scan(|engine| {
            fit.residuals(engine, &vec![1.; 3 * SCAN_CHUNK], &mut Vec::new())
        });
        cancels_during_scan(|engine| engine.statsx_issorted(std::slice::from_ref(&x)).map(|_| ()));
        let wide = Value::Matrix(Matrix::filled(2, 3 * SCAN_CHUNK, 1.).unwrap());
        cancels_during_scan(|engine| {
            engine
                .statsx_issorted(&[wide, Value::Text("rows".into())])
                .map(|_| ())
        });
    }
}
