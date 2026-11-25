//! Pure data and geometry rules of the 2-D chart builtins (histogram, errorbar,
//! area, barh, pie, fill, polarplot, fplot, heatmap). No engine state; every
//! function is bounded by its caller's admitted sizes.
use crate::Result;

/// Most bins one histogram or histcounts call may produce (MATLAB documents
/// the same 65536 ceiling for automatic binning).
pub(crate) const P2_MAX_BINS: usize = 65_536;

// ---------------------------------------------------------------- binning

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum P2Norm {
    Count,
    Probability,
    Pdf,
    Cdf,
    CountDensity,
    CumCount,
}
impl P2Norm {
    pub(crate) fn parse(text: &str) -> Result<Self> {
        Ok(match text.to_ascii_lowercase().as_str() {
            "count" => Self::Count,
            "probability" => Self::Probability,
            "pdf" => Self::Pdf,
            "cdf" => Self::Cdf,
            "countdensity" => Self::CountDensity,
            "cumcount" => Self::CumCount,
            _ => {
                return Err(
                    "Normalization must be count, probability, pdf, cdf, countdensity or cumcount"
                        .into(),
                );
            }
        })
    }
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Count => "count",
            Self::Probability => "probability",
            Self::Pdf => "pdf",
            Self::Cdf => "cdf",
            Self::CountDensity => "countdensity",
            Self::CumCount => "cumcount",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum P2Method {
    Auto,
    Scott,
    Fd,
    Integers,
    Sturges,
    Sqrt,
}
impl P2Method {
    pub(crate) fn parse(text: &str) -> Result<Self> {
        Ok(match text.to_ascii_lowercase().as_str() {
            "auto" => Self::Auto,
            "scott" => Self::Scott,
            "fd" => Self::Fd,
            "integers" => Self::Integers,
            "sturges" => Self::Sturges,
            "sqrt" => Self::Sqrt,
            _ => {
                return Err("BinMethod must be auto, scott, fd, integers, sturges or sqrt".into());
            }
        })
    }
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Scott => "scott",
            Self::Fd => "fd",
            Self::Integers => "integers",
            Self::Sturges => "sturges",
            Self::Sqrt => "sqrt",
        }
    }
}

/// Binning request. At most one of `nbins`, `edges` and `width` is set.
#[derive(Clone, Debug)]
pub(crate) struct P2Bins {
    pub nbins: Option<usize>,
    pub edges: Option<Vec<f64>>,
    pub width: Option<f64>,
    pub limits: Option<[f64; 2]>,
    pub method: P2Method,
    pub norm: P2Norm,
}
impl Default for P2Bins {
    fn default() -> Self {
        Self {
            nbins: None,
            edges: None,
            width: None,
            limits: None,
            method: P2Method::Auto,
            norm: P2Norm::Count,
        }
    }
}

/// The "nice edge" placement shared with histcounts (see poly_interp).
pub(crate) struct P2Rules {
    pub auto: fn(&[f64]) -> Vec<f64>,
    pub picker: fn(f64, f64, Option<usize>, f64) -> Vec<f64>,
}

fn p2_min_max(x: &[f64]) -> Option<(f64, f64)> {
    let first = *x.first()?;
    Some(
        x.iter()
            .fold((first, first), |(l, h), &v| (l.min(v), h.max(v))),
    )
}

fn p2_linspace(lo: f64, hi: f64, n: usize) -> Vec<f64> {
    (0..=n)
        .map(|i| {
            if i == n {
                hi
            } else {
                lo + (hi - lo) * i as f64 / n as f64
            }
        })
        .collect()
}

/// Linear-interpolation quantile on sorted data at position (n-1)p.
fn p2_quantile(sorted: &[f64], p: f64) -> f64 {
    let h = (sorted.len() - 1) as f64 * p;
    let i = h.floor() as usize;
    let j = (i + 1).min(sorted.len() - 1);
    sorted[i] + (h - i as f64) * (sorted[j] - sorted[i])
}

fn p2_check_bins(n: usize) -> Result<()> {
    if n > P2_MAX_BINS {
        return Err(format!("Binning exceeds {P2_MAX_BINS} bins"));
    }
    Ok(())
}

/// Number of bins (or full edges) a bin method chooses for finite data.
fn p2_method_edges(finite: &[f64], method: P2Method, rules: &P2Rules) -> Result<Vec<f64>> {
    let Some((lo, hi)) = p2_min_max(finite) else {
        return Ok(vec![0., 1.]);
    };
    let n = finite.len() as f64;
    let edges = match method {
        P2Method::Auto => (rules.auto)(finite),
        P2Method::Scott => {
            let mean = finite.iter().sum::<f64>() / n;
            let var = if finite.len() > 1 {
                finite.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / (n - 1.)
            } else {
                0.
            };
            (rules.picker)(lo, hi, None, 3.5 * var.sqrt() / n.cbrt())
        }
        P2Method::Fd => {
            let mut sorted = finite.to_vec();
            sorted.sort_by(f64::total_cmp);
            let iqr = p2_quantile(&sorted, 0.75) - p2_quantile(&sorted, 0.25);
            let width = if iqr > 0. {
                2. * iqr / n.cbrt()
            } else {
                (hi - lo) / 10.
            };
            (rules.picker)(lo, hi, None, width)
        }
        P2Method::Integers => {
            let left = lo.round() - 0.5;
            let count = ((hi.round() + 0.5 - left).round() as usize).max(1);
            p2_check_bins(count)?;
            (0..=count).map(|i| left + i as f64).collect()
        }
        P2Method::Sturges => {
            let bins = (n.log2() + 1.).ceil().max(1.);
            (rules.picker)(lo, hi, None, (hi - lo) / bins)
        }
        P2Method::Sqrt => {
            let bins = n.sqrt().ceil().max(1.);
            (rules.picker)(lo, hi, None, (hi - lo) / bins)
        }
    };
    p2_check_bins(edges.len().saturating_sub(1))?;
    Ok(edges)
}

/// Bin edges. Rules, in order:
/// - explicit edges are used as given (nondecreasing, at least two);
/// - BinLimits [lo,hi] with NumBins n: n equal bins over the limits
///   (identical to numpy.histogram_bin_edges(x, n, range=(lo,hi)));
/// - BinLimits with BinWidth w: edges lo, lo+w, ..., the last edge is hi;
/// - BinLimits alone: the bin method's bin count on the data inside the limits,
///   spread evenly over the limits;
/// - BinWidth w alone: edges at multiples of w from w*floor(min/w) to the
///   first multiple at or above max;
/// - NumBins alone: the histcounts "nice" edges for that count;
/// - otherwise the bin method (auto: integer bins for integer data spanning at
///   most 50, else Scott's rule with nice edges).
pub(crate) fn p2_edges(data: &[f64], bins: &P2Bins, rules: &P2Rules) -> Result<Vec<f64>> {
    if let Some(edges) = &bins.edges {
        if edges.len() < 2
            || edges.iter().any(|v| v.is_nan())
            || edges.windows(2).any(|w| w[1] < w[0])
        {
            return Err("Bin edges must be a nondecreasing vector of at least two values".into());
        }
        p2_check_bins(edges.len() - 1)?;
        return Ok(edges.clone());
    }
    let finite: Vec<f64> = data.iter().copied().filter(|v| v.is_finite()).collect();
    if let Some([lo, hi]) = bins.limits {
        if !(lo.is_finite() && hi.is_finite() && lo <= hi) {
            return Err("BinLimits must be a finite increasing two-element vector".into());
        }
        if let Some(n) = bins.nbins {
            p2_check_bins(n)?;
            return Ok(p2_linspace(lo, hi, n));
        }
        if let Some(w) = bins.width {
            let count = (((hi - lo) / w) * (1. - 1e-12)).ceil().max(1.);
            if count > P2_MAX_BINS as f64 {
                return Err(format!("Binning exceeds {P2_MAX_BINS} bins"));
            }
            let count = count as usize;
            let mut edges: Vec<f64> = (0..count).map(|i| lo + i as f64 * w).collect();
            edges.push(hi);
            return Ok(edges);
        }
        let inside: Vec<f64> = finite
            .iter()
            .copied()
            .filter(|v| (lo..=hi).contains(v))
            .collect();
        let n = p2_method_edges(&inside, bins.method, rules)?.len() - 1;
        return Ok(p2_linspace(lo, hi, n.max(1)));
    }
    let Some((lo, hi)) = p2_min_max(&finite) else {
        return Ok(match bins.nbins {
            Some(n) => {
                p2_check_bins(n)?;
                (0..=n).map(|i| i as f64).collect()
            }
            None => vec![0., 1.],
        });
    };
    if let Some(w) = bins.width {
        let left = w * (lo / w).floor();
        let count = ((hi - left) / w).ceil().max(1.);
        if count > P2_MAX_BINS as f64 {
            return Err(format!("Binning exceeds {P2_MAX_BINS} bins"));
        }
        let count = count as usize;
        let mut edges: Vec<f64> = (0..=count).map(|i| left + i as f64 * w).collect();
        // Rounding may leave the maximum just past the last multiple.
        if *edges.last().unwrap() < hi {
            edges.push(left + (count + 1) as f64 * w);
        }
        return Ok(edges);
    }
    if let Some(n) = bins.nbins {
        p2_check_bins(n)?;
        return Ok((rules.picker)(lo, hi, Some(n), (hi - lo) / n as f64));
    }
    p2_method_edges(&finite, bins.method, rules)
}

/// Counts with half-open bins [e_k, e_k+1) and a closed last bin.
pub(crate) fn p2_counts(data: &[f64], edges: &[f64]) -> Vec<f64> {
    let nb = edges.len() - 1;
    let mut counts = vec![0.; nb];
    let (first, last) = (edges[0], edges[nb]);
    for &x in data {
        if x.is_nan() || x < first || x > last {
            continue;
        }
        let j = edges.partition_point(|&e| e <= x);
        counts[(j.max(1) - 1).min(nb - 1)] += 1.;
    }
    counts
}

/// Normalized values. `total` is the number of elements of the input data.
pub(crate) fn p2_normalize(counts: &[f64], edges: &[f64], total: usize, norm: P2Norm) -> Vec<f64> {
    let n = total.max(1) as f64;
    let width = |i: usize| edges[i + 1] - edges[i];
    let mut running = 0.;
    counts
        .iter()
        .enumerate()
        .map(|(i, &c)| match norm {
            P2Norm::Count => c,
            P2Norm::Probability => c / n,
            P2Norm::Pdf => c / (n * width(i)),
            P2Norm::CountDensity => c / width(i),
            P2Norm::Cdf => {
                running += c;
                running / n
            }
            P2Norm::CumCount => {
                running += c;
                running
            }
        })
        .collect()
}

/// One axis-aligned quadrilateral per bin, vertical edge first.
pub(crate) fn p2_bin_quads(edges: &[f64], values: &[f64], horizontal: bool) -> Vec<[f64; 2]> {
    let mut out = Vec::with_capacity(values.len() * 4);
    for (i, &v) in values.iter().enumerate() {
        let (a, b) = (edges[i], edges[i + 1]);
        let v = if v.is_finite() { v } else { 0. };
        let quad = [[a, 0.], [a, v], [b, v], [b, 0.]];
        out.extend(quad.map(|p| if horizontal { [p[1], p[0]] } else { p }));
    }
    out
}

/// Outline of a histogram drawn as stairs: down to zero at both ends.
pub(crate) fn p2_bin_outline(edges: &[f64], values: &[f64]) -> Vec<[f64; 2]> {
    let mut out = Vec::with_capacity(values.len() * 2 + 2);
    out.push([edges[0], 0.]);
    for (i, &v) in values.iter().enumerate() {
        out.push([edges[i], v]);
        out.push([edges[i + 1], v]);
    }
    out.push([edges[values.len()], 0.]);
    out
}

// ----------------------------------------------------------- error bars

/// Deltas of one error-bar series; empty vectors mean "no bar" in that direction.
#[derive(Clone, Debug, Default)]
pub(crate) struct P2Deltas {
    pub yneg: Vec<f64>,
    pub ypos: Vec<f64>,
    pub xneg: Vec<f64>,
    pub xpos: Vec<f64>,
}

/// Bar and cap segments separated by NaN. `cap` is the half cap length in
/// data units along x (for vertical bars) and along y (for horizontal bars).
pub(crate) fn p2_errorbar_segments(
    x: &[f64],
    y: &[f64],
    d: &P2Deltas,
    cap: [f64; 2],
) -> Vec<[f64; 2]> {
    const GAP: [f64; 2] = [f64::NAN; 2];
    let mut out = Vec::new();
    let segment = |a: [f64; 2], b: [f64; 2], out: &mut Vec<[f64; 2]>| {
        if a.iter().chain(&b).all(|v| v.is_finite()) {
            out.extend([a, b, GAP]);
        }
    };
    for i in 0..x.len() {
        let (xi, yi) = (x[i], y[i]);
        if !yi.is_finite() || !xi.is_finite() {
            continue;
        }
        if !d.yneg.is_empty() {
            let (lo, hi) = (yi - d.yneg[i], yi + d.ypos[i]);
            segment([xi, lo], [xi, hi], &mut out);
            if cap[0] > 0. {
                segment([xi - cap[0], lo], [xi + cap[0], lo], &mut out);
                segment([xi - cap[0], hi], [xi + cap[0], hi], &mut out);
            }
        }
        if !d.xneg.is_empty() {
            let (lo, hi) = (xi - d.xneg[i], xi + d.xpos[i]);
            segment([lo, yi], [hi, yi], &mut out);
            if cap[1] > 0. {
                segment([lo, yi - cap[1]], [lo, yi + cap[1]], &mut out);
                segment([hi, yi - cap[1]], [hi, yi + cap[1]], &mut out);
            }
        }
    }
    out
}

/// Data bounds including every error extent.
pub(crate) fn p2_errorbar_bounds(x: &[f64], y: &[f64], d: &P2Deltas) -> ([f64; 2], [f64; 2]) {
    let mut bx = [f64::INFINITY, f64::NEG_INFINITY];
    let mut by = bx;
    let grow = |b: &mut [f64; 2], v: f64| {
        if v.is_finite() {
            b[0] = b[0].min(v);
            b[1] = b[1].max(v);
        }
    };
    for i in 0..x.len() {
        grow(&mut bx, x[i]);
        grow(&mut by, y[i]);
        if !d.yneg.is_empty() {
            grow(&mut by, y[i] - d.yneg[i]);
            grow(&mut by, y[i] + d.ypos[i]);
        }
        if !d.xneg.is_empty() {
            grow(&mut bx, x[i] - d.xneg[i]);
            grow(&mut bx, x[i] + d.xpos[i]);
        }
    }
    (bx, by)
}

// ------------------------------------------------------------------ area

/// Upper curve of every stacked layer: cumulative sums of the columns of the
/// row-major-by-column data `y` (n rows, m columns, column-major).
pub(crate) fn p2_area_tops(y: &[f64], n: usize, m: usize) -> Vec<Vec<f64>> {
    let mut running = vec![0.; n];
    (0..m)
        .map(|j| {
            for i in 0..n {
                running[i] += y[j * n + i];
            }
            running.clone()
        })
        .collect()
}

/// Triangles filling between `lower` and `upper` over `x`, split where the
/// two curves cross so no triangle covers the wrong side.
pub(crate) fn p2_band_triangles(x: &[f64], lower: &[f64], upper: &[f64]) -> Vec<[f64; 2]> {
    let mut out = Vec::new();
    let quad = |a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2], out: &mut Vec<_>| {
        // a,b on the lower curve, c,d on the upper curve (c above b, d above a).
        out.extend([a, b, c, a, c, d]);
    };
    for i in 0..x.len().saturating_sub(1) {
        let (x0, x1) = (x[i], x[i + 1]);
        let (l0, l1, u0, u1) = (lower[i], lower[i + 1], upper[i], upper[i + 1]);
        if ![x0, x1, l0, l1, u0, u1].iter().all(|v| v.is_finite()) {
            continue;
        }
        let (d0, d1) = (u0 - l0, u1 - l1);
        if d0 * d1 < 0. {
            let t = d0 / (d0 - d1);
            let xm = x0 + t * (x1 - x0);
            let ym = l0 + t * (l1 - l0);
            out.extend([[x0, l0], [xm, ym], [x0, u0]]);
            out.extend([[xm, ym], [x1, l1], [x1, u1]]);
        } else {
            quad([x0, l0], [x1, l1], [x1, u1], [x0, u0], &mut out);
        }
    }
    out
}

/// Closed outline of one band, NaN-terminated.
pub(crate) fn p2_band_outline(x: &[f64], lower: &[f64], upper: &[f64]) -> Vec<[f64; 2]> {
    let mut out: Vec<[f64; 2]> = x.iter().zip(upper).map(|(&a, &b)| [a, b]).collect();
    out.extend(x.iter().zip(lower).rev().map(|(&a, &b)| [a, b]));
    if let Some(&first) = out.first() {
        out.push(first);
    }
    out.push([f64::NAN; 2]);
    out
}

// ---------------------------------------------------------------- barh

/// Bar rectangles for an n-by-m data set (column-major). `horizontal` puts the
/// value along x. Returns one quad list per column. Grouped layout uses the
/// group width min(0.8, m/(m+1.5)) of the category spacing, split equally.
pub(crate) fn p2_bar_rects(
    cats: &[f64],
    y: &[f64],
    m: usize,
    width: f64,
    stacked: bool,
    base: f64,
    horizontal: bool,
) -> Vec<Vec<[f64; 2]>> {
    let n = cats.len();
    let spacing = if n > 1 {
        let mut sorted = cats.to_vec();
        sorted.sort_by(f64::total_cmp);
        sorted
            .windows(2)
            .map(|w| w[1] - w[0])
            .filter(|d| *d > 0.)
            .fold(f64::INFINITY, f64::min)
    } else {
        1.
    };
    let spacing = if spacing.is_finite() { spacing } else { 1. };
    let mut pos = vec![base; n];
    let mut neg = vec![base; n];
    (0..m)
        .map(|j| {
            let mut quads = Vec::with_capacity(n * 4);
            for i in 0..n {
                let v = y[j * n + i];
                let v = if v.is_finite() { v } else { 0. };
                let (center, half, lo, hi) = if stacked || m == 1 {
                    let half = width * spacing / 2.;
                    if stacked {
                        let start = if v >= 0. { &mut pos[i] } else { &mut neg[i] };
                        let lo = *start;
                        *start += v;
                        (cats[i], half, lo, *start)
                    } else {
                        (cats[i], half, base, base + v)
                    }
                } else {
                    let group = (m as f64 / (m as f64 + 1.5)).min(0.8) * spacing;
                    let slot = group / m as f64;
                    let center = cats[i] - group / 2. + (j as f64 + 0.5) * slot;
                    (center, slot * width / 2., base, base + v)
                };
                let quad = [
                    [center - half, lo],
                    [center - half, hi],
                    [center + half, hi],
                    [center + half, lo],
                ];
                quads.extend(quad.map(|p| if horizontal { [p[1], p[0]] } else { p }));
            }
            quads
        })
        .collect()
}

// ------------------------------------------------------------------ pie

/// Slice fractions: x/sum(x) when sum(x) >= 1, otherwise x itself (a partial pie).
pub(crate) fn p2_pie_fractions(x: &[f64]) -> Result<Vec<f64>> {
    if x.iter().any(|v| !v.is_finite() || *v < 0.) {
        return Err("pie data must be finite and nonnegative".into());
    }
    let sum: f64 = x.iter().sum();
    if sum == 0. {
        return Err("pie data must contain at least one positive value".into());
    }
    Ok(if sum >= 1. {
        x.iter().map(|v| v / sum).collect()
    } else {
        x.to_vec()
    })
}

/// Percentage label: round(100 p) with "%", and "< 1%" for nonzero slices below 0.5 %.
pub(crate) fn p2_pie_label(p: f64) -> String {
    let percent = (100. * p).round();
    if percent < 1. {
        "< 1%".into()
    } else {
        format!("{percent}%")
    }
}

/// One slice: triangles of a fan from the (possibly exploded) centre, the
/// closed outline, and the slice's middle angle. Angles in radians.
pub(crate) struct P2Slice {
    pub triangles: Vec<[f64; 2]>,
    pub outline: Vec<[f64; 2]>,
    pub middle: f64,
    pub center: [f64; 2],
}
pub(crate) fn p2_pie_slice(start: f64, end: f64, explode: bool) -> P2Slice {
    let middle = (start + end) / 2.;
    let center = if explode {
        [0.1 * middle.cos(), 0.1 * middle.sin()]
    } else {
        [0., 0.]
    };
    let steps = (((end - start).abs() / (std::f64::consts::PI / 90.)).ceil() as usize).max(1);
    let arc: Vec<[f64; 2]> = (0..=steps)
        .map(|k| {
            let t = start + (end - start) * k as f64 / steps as f64;
            [center[0] + t.cos(), center[1] + t.sin()]
        })
        .collect();
    let mut triangles = Vec::with_capacity(steps * 3);
    for w in arc.windows(2) {
        triangles.extend([center, w[0], w[1]]);
    }
    let mut outline = vec![center];
    outline.extend(&arc);
    outline.extend([center, [f64::NAN; 2]]);
    P2Slice {
        triangles,
        outline,
        middle,
        center,
    }
}

// --------------------------------------------------------------- polar

/// "Nice" ticks covering [lo, hi]: steps 1, 2 or 5 times a power of ten,
/// about `target` intervals. Returns (first, step, count).
pub(crate) fn p2_nice_step(range: f64, target: f64) -> f64 {
    let raw = (range / target).max(f64::MIN_POSITIVE);
    let pow = 10f64.powf(raw.log10().floor());
    let rel = raw / pow;
    let factor = if rel <= 1. {
        1.
    } else if rel <= 2. {
        2.
    } else if rel <= 5. {
        5.
    } else {
        10.
    };
    factor * pow
}

/// Automatic RLim: 0 (or a nice floor of a negative minimum) to a nice ceiling.
pub(crate) fn p2_auto_rlim(rho: &[f64]) -> [f64; 2] {
    let finite = rho.iter().copied().filter(|v| v.is_finite());
    let (lo, hi) = finite.fold((0f64, f64::NEG_INFINITY), |(l, h), v| (l.min(v), h.max(v)));
    let hi = if hi.is_finite() && hi > lo {
        hi
    } else {
        lo + 1.
    };
    let step = p2_nice_step(hi - lo, 4.);
    [
        (lo / step).floor() * step,
        ((hi / step) * (1. - 1e-12)).ceil() * step,
    ]
}

/// Polar angle orientation: zero direction (radians, counterclockwise from +x)
/// and the sign of increasing theta.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct P2Orient {
    pub zero: f64,
    pub dir: f64,
}
impl P2Orient {
    pub(crate) fn point(self, theta: f64, rho: f64, rlim: [f64; 2]) -> [f64; 2] {
        let r = rho - rlim[0];
        if !theta.is_finite() || !rho.is_finite() || r < 0. {
            return [f64::NAN; 2];
        }
        let phi = self.zero + self.dir * theta;
        [r * phi.cos(), r * phi.sin()]
    }
}

/// Circle grid at every r tick above the minimum and spokes every 30 degrees.
pub(crate) fn p2_polar_grid(
    rlim: [f64; 2],
    step: f64,
    orient: P2Orient,
) -> (Vec<[f64; 2]>, Vec<f64>) {
    let radius = rlim[1] - rlim[0];
    let mut ticks = Vec::new();
    let mut k = 1usize;
    loop {
        let r = rlim[0] + step * k as f64;
        if r > rlim[1] * (1. + 1e-12) + 1e-300 || k > 20 {
            break;
        }
        ticks.push(r);
        k += 1;
    }
    if ticks
        .last()
        .is_none_or(|&r| (r - rlim[1]).abs() > step * 1e-9)
    {
        ticks.push(rlim[1]);
    }
    let mut out = Vec::new();
    for &r in &ticks {
        let rr = r - rlim[0];
        out.extend((0..=180).map(|i| {
            let t = std::f64::consts::PI * i as f64 / 90.;
            [rr * t.cos(), rr * t.sin()]
        }));
        out.push([f64::NAN; 2]);
    }
    for k in 0..12 {
        let phi = orient.zero + orient.dir * (k as f64 * std::f64::consts::PI / 6.);
        out.extend([
            [0., 0.],
            [radius * phi.cos(), radius * phi.sin()],
            [f64::NAN; 2],
        ]);
    }
    (out, ticks)
}

// --------------------------------------------------------------- fplot

/// Adaptive sampling of y = f(x) on [a, b]. Rule: start with `density`
/// equally spaced points including both limits. Each round evaluates the
/// midpoint of every still-active interval and keeps it as a sample; both
/// halves stay active when that midpoint deviates from the chord by more than
/// 0.1 % of the initial y range (or finiteness changes across the interval).
/// At most 10 rounds and 5000 points. `eval` maps x to y in batches.
pub(crate) fn p2_adaptive_samples(
    a: f64,
    b: f64,
    density: usize,
    eval: &mut dyn FnMut(&[f64]) -> Result<Vec<f64>>,
) -> Result<(Vec<f64>, Vec<f64>)> {
    const MAX_POINTS: usize = 5000;
    let n = density.clamp(2, MAX_POINTS);
    let mut xs = p2_linspace(a, b, n - 1);
    let mut ys = eval(&xs)?;
    let finite: Vec<f64> = ys.iter().copied().filter(|v| v.is_finite()).collect();
    let range = p2_min_max(&finite).map_or(1., |(lo, hi)| hi - lo);
    let tol = 1e-3 * if range > 0. { range } else { 1. };
    let min_width = (b - a).abs() * 1e-7;
    let mut active: Vec<bool> = vec![true; xs.len() - 1];
    for _ in 0..10 {
        let mids: Vec<usize> = (0..xs.len() - 1)
            .filter(|&i| active[i] && (xs[i + 1] - xs[i]).abs() > min_width)
            .collect();
        if mids.is_empty() || xs.len() + mids.len() > MAX_POINTS {
            break;
        }
        let mx: Vec<f64> = mids.iter().map(|&i| 0.5 * (xs[i] + xs[i + 1])).collect();
        let my = eval(&mx)?;
        let mut nx = Vec::with_capacity(xs.len() + mids.len());
        let mut ny = Vec::with_capacity(nx.capacity());
        let mut nactive = Vec::with_capacity(nx.capacity());
        let mut next = 0;
        for i in 0..xs.len() {
            nx.push(xs[i]);
            ny.push(ys[i]);
            if i + 1 == xs.len() {
                break;
            }
            if next < mids.len() && mids[next] == i {
                let (y0, y1, ym) = (ys[i], ys[i + 1], my[next]);
                let refine = if y0.is_finite() && y1.is_finite() && ym.is_finite() {
                    (ym - 0.5 * (y0 + y1)).abs() > tol
                } else {
                    !(y0.is_nan() && y1.is_nan() && ym.is_nan())
                };
                nx.push(mx[next]);
                ny.push(ym);
                nactive.extend([refine, refine]);
                next += 1;
            } else {
                nactive.push(false);
            }
        }
        xs = nx;
        ys = ny;
        active = nactive;
    }
    Ok((xs, ys))
}

// ------------------------------------------------------------- heatmap

/// Colour levels of the heatmap scale.
pub(crate) const P2_LEVELS: usize = 16;

/// Level of value v on [lo, hi] (None for NaN).
pub(crate) fn p2_level(v: f64, lo: f64, hi: f64) -> Option<usize> {
    if v.is_nan() {
        return None;
    }
    if hi <= lo {
        return Some(P2_LEVELS / 2);
    }
    let t = ((v - lo) / (hi - lo)).clamp(0., 1.);
    Some(((t * P2_LEVELS as f64) as usize).min(P2_LEVELS - 1))
}

/// Sequential ramp from a near-white tint of `base` to a darkened `base`.
pub(crate) fn p2_ramp(level: usize, base: [u8; 3]) -> [u8; 3] {
    let t = (level as f64 + 0.5) / P2_LEVELS as f64;
    let light = base.map(|c| 255. - 0.07 * (255. - f64::from(c)));
    let dark = base.map(|c| 0.62 * f64::from(c));
    [0, 1, 2].map(|k| (light[k] + t * (dark[k] - light[k])).round() as u8)
}

/// Black text on light cells, white on dark ones (relative luminance).
pub(crate) fn p2_text_on(rgb: [u8; 3]) -> [u8; 3] {
    let l = 0.2126 * f64::from(rgb[0]) + 0.7152 * f64::from(rgb[1]) + 0.0722 * f64::from(rgb[2]);
    if l < 140. { [255; 3] } else { [0; 3] }
}

/// Short cell label: up to 4 significant digits, like MATLAB's default %.4g.
pub(crate) fn p2_short(v: f64) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v == 0. {
        return "0".into();
    }
    let exponent = v.abs().log10().floor() as i32;
    if !(-5..4).contains(&exponent) {
        let mantissa = v / 10f64.powi(exponent);
        let m = format!("{mantissa:.3}");
        let m = m.trim_end_matches('0').trim_end_matches('.');
        return format!(
            "{m}e{}{:02}",
            if exponent < 0 { '-' } else { '+' },
            exponent.abs()
        );
    }
    let decimals = (3 - exponent).max(0) as usize;
    let s = format!("{v:.decimals$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

// ------------------------------------------------------- triangulation

/// Ear-clipping triangulation of one simple polygon (closing vertex optional).
/// Degenerate or self-intersecting input falls back to a fan.
#[cfg(test)]
pub(crate) fn p2_triangulate(poly: &[[f64; 2]]) -> Vec<[f64; 2]> {
    p2_triangulate_indices(poly)
        .into_iter()
        .map(|i| poly[i])
        .collect()
}

/// `p2_triangulate` as source-vertex indices, three per triangle. A repeated
/// consecutive vertex and a closing vertex equal to the first map to their
/// first occurrence, so per-vertex data follows the rendered corners.
pub(crate) fn p2_triangulate_indices(poly: &[[f64; 2]]) -> Vec<usize> {
    let mut source: Vec<usize> = Vec::with_capacity(poly.len());
    for (i, &p) in poly.iter().enumerate() {
        if source.last().map(|&j| poly[j]) != Some(p) {
            source.push(i);
        }
    }
    if source.len() > 1 && poly[source[0]] == poly[source[source.len() - 1]] {
        source.pop();
    }
    if source.len() < 3 {
        return Vec::new();
    }
    let pts: Vec<[f64; 2]> = source.iter().map(|&i| poly[i]).collect();
    let area: f64 = (0..pts.len())
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum();
    let sign = if area >= 0. { 1. } else { -1. };
    let cross = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| {
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    };
    let mut index: Vec<usize> = (0..pts.len()).collect();
    let mut out = Vec::with_capacity((pts.len() - 2) * 3);
    let mut guard = 0;
    while index.len() > 3 && guard < 4 * pts.len() * pts.len() {
        guard += 1;
        let n = index.len();
        let mut clipped = false;
        for k in 0..n {
            let (ia, ib, ic) = (index[(k + n - 1) % n], index[k], index[(k + 1) % n]);
            let (a, b, c) = (pts[ia], pts[ib], pts[ic]);
            if sign * cross(a, b, c) <= 0. {
                continue;
            }
            let inside = index.iter().any(|&j| {
                if j == ia || j == ib || j == ic {
                    return false;
                }
                let p = pts[j];
                sign * cross(a, b, p) >= 0.
                    && sign * cross(b, c, p) >= 0.
                    && sign * cross(c, a, p) >= 0.
            });
            if inside {
                continue;
            }
            out.extend([source[ia], source[ib], source[ic]]);
            index.remove(k);
            clipped = true;
            break;
        }
        if !clipped {
            // Not simple: fan the remaining vertices.
            for k in 1..index.len() - 1 {
                out.extend([source[index[0]], source[index[k]], source[index[k + 1]]]);
            }
            return out;
        }
    }
    if index.len() == 3 {
        out.extend([source[index[0]], source[index[1]], source[index[2]]]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> P2Rules {
        P2Rules {
            auto: |x| {
                let (lo, hi) = p2_min_max(x).unwrap();
                p2_linspace(lo, hi, 4)
            },
            picker: |lo, hi, n, _| p2_linspace(lo, hi, n.unwrap_or(3)),
        }
    }

    #[test]
    fn counts_close_the_last_bin_and_skip_outside_values() {
        let edges = [0., 1., 2., 3.];
        assert_eq!(
            p2_counts(&[0., 0.5, 1., 2.9, 3., 3.1, -1., f64::NAN], &edges),
            vec![2., 1., 2.]
        );
    }

    #[test]
    fn normalizations_follow_their_definitions() {
        let edges = [0., 1., 3.];
        let c = [2., 2.];
        assert_eq!(
            p2_normalize(&c, &edges, 4, P2Norm::Probability),
            vec![0.5, 0.5]
        );
        assert_eq!(p2_normalize(&c, &edges, 4, P2Norm::Pdf), vec![0.5, 0.25]);
        assert_eq!(p2_normalize(&c, &edges, 4, P2Norm::Cdf), vec![0.5, 1.]);
        assert_eq!(
            p2_normalize(&c, &edges, 4, P2Norm::CountDensity),
            vec![2., 1.]
        );
        assert_eq!(p2_normalize(&c, &edges, 4, P2Norm::CumCount), vec![2., 4.]);
    }

    #[test]
    fn limits_with_bin_count_are_uniform_and_width_is_aligned() {
        let b = P2Bins {
            nbins: Some(4),
            limits: Some([0., 2.]),
            ..P2Bins::default()
        };
        assert_eq!(
            p2_edges(&[0.3], &b, &rules()).unwrap(),
            vec![0., 0.5, 1., 1.5, 2.]
        );
        let b = P2Bins {
            width: Some(0.5),
            ..P2Bins::default()
        };
        assert_eq!(
            p2_edges(&[0.3, 1.2], &b, &rules()).unwrap(),
            vec![0., 0.5, 1., 1.5]
        );
    }

    #[test]
    fn triangulation_covers_a_concave_polygon() {
        let l = [[0., 0.], [2., 0.], [2., 1.], [1., 1.], [1., 2.], [0., 2.]];
        let t = p2_triangulate(&l);
        assert_eq!(t.len(), 12);
        let area: f64 = t
            .chunks(3)
            .map(|c| {
                0.5 * ((c[1][0] - c[0][0]) * (c[2][1] - c[0][1])
                    - (c[1][1] - c[0][1]) * (c[2][0] - c[0][0]))
                    .abs()
            })
            .sum();
        assert!((area - 3.).abs() < 1e-12);
    }

    #[test]
    fn adaptive_sampling_keeps_limits_and_refines_curvature() {
        let mut f = |x: &[f64]| Ok(x.iter().map(|v| v * v).collect());
        let (x, _) = p2_adaptive_samples(-1., 2., 5, &mut f).unwrap();
        assert_eq!((x[0], *x.last().unwrap()), (-1., 2.));
        assert!(x.len() > 5 && x.windows(2).all(|w| w[1] > w[0]));
        let mut line = |x: &[f64]| Ok(x.iter().map(|v| 3. * v + 1.).collect());
        assert_eq!(
            p2_adaptive_samples(0., 1., 7, &mut line).unwrap().0.len(),
            13
        );
    }

    #[test]
    fn short_labels_use_four_significant_digits() {
        assert_eq!(p2_short(1.23456), "1.235");
        assert_eq!(p2_short(12.), "12");
        assert_eq!(p2_short(123456.), "1.235e+05");
        assert_eq!(p2_short(-0.5), "-0.5");
    }
}
