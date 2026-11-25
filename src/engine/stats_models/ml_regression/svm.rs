//! Epsilon-SVR dual SMO. The two nonnegative multipliers per observation
//! obey sum(sign * alpha)=0, 0<=alpha<=C. Pair steps exactly minimize the
//! restricted quadratic; convergence is checked against the dual KKT gap.
use super::*;
#[derive(Clone, Debug)]
pub(super) struct Svm {
    pub alpha: Vec<f64>,
    pub bias: f64,
    pub x: Matrix,
    pub means: Vec<f64>,
    pub scales: Vec<f64>,
    kernel: String,
    scale: f64,
    order: i32,
    epsilon: f64,
    c: f64,
    standardize: bool,
}
fn kernel(a: &[f64], b: &[f64], name: &str, scale: f64, order: i32) -> f64 {
    if name == "rbf" {
        (-a.iter()
            .zip(b)
            .map(|(a, b)| ((a - b) / scale).powi(2))
            .sum::<f64>())
        .exp()
    } else {
        let dot = a
            .iter()
            .zip(b)
            .map(|(a, b)| (a / scale) * (b / scale))
            .sum::<f64>();
        if name == "polynomial" {
            (1. + dot).powi(order)
        } else {
            dot
        }
    }
}
pub(super) fn train(e: &mut Engine, x: &Matrix, y: &[f64], opts: &SvmOptions) -> Result<Svm> {
    let (n, p) = (x.rows(), x.cols());
    if n > 2048 || n.saturating_mul(n).saturating_mul(p) > 100_000_000 {
        return Err("fitrsvm: SMO limit is 2048 rows and 100000000 kernel element products".into());
    }
    size(e, "fitrsvm", n, n + 8)?;
    let (means, scales) = if opts.standardize {
        means_scales(x)
    } else {
        (vec![0.; p], vec![1.; p])
    };
    let z = Matrix::from_column_vec(
        n,
        p,
        (0..p)
            .flat_map(|j| {
                let means = &means;
                let scales = &scales;
                (0..n).map(move |i| (x.data()[i + j * n] - means[j]) / scales[j])
            })
            .collect(),
    )?;
    let scale = if let Some(s) = opts.scale {
        s
    } else {
        // A seeded median-distance heuristic. Sampling consumes only the
        // engine's rng stream; its exact sequence is part of this implementation.
        let mut distances = Vec::new();
        for _ in 0..n.saturating_mul(4).clamp(16, 1024) {
            let a = (e.random.next_f64() * n as f64) as usize;
            let b = (e.random.next_f64() * n as f64) as usize;
            let d = (0..p)
                .map(|j| (z.data()[a + j * n] - z.data()[b + j * n]).powi(2))
                .sum::<f64>()
                .sqrt();
            if d > 0. && d.is_finite() {
                distances.push(d);
            }
        }
        distances.sort_by(f64::total_cmp);
        if distances.is_empty() {
            1.
        } else {
            distances[distances.len() / 2]
        }
    };
    let spread = iqr(y);
    let c = opts.c.unwrap_or(if opts.kernel == "rbf" && spread > 0. {
        spread / 1.349
    } else {
        1.
    });
    let epsilon = opts.epsilon.unwrap_or_else(|| default_epsilon(y));
    let rows: Vec<_> = (0..n).map(|i| row(&z, i)).collect();
    let mut gram = vec![0.; n * n];
    for i in 0..n {
        e.check_cancel()?;
        for j in 0..=i {
            let v = kernel(&rows[i], &rows[j], &opts.kernel, scale, opts.order);
            if !v.is_finite() {
                return Err("fitrsvm: kernel overflow; rescale or standardize predictors".into());
            }
            gram[i * n + j] = v;
            gram[j * n + i] = v;
        }
    }
    let (alpha, bias) = solve(e, &gram, y, c, epsilon)?;
    Ok(Svm {
        alpha,
        bias,
        x: z,
        means,
        scales,
        kernel: opts.kernel.clone(),
        scale,
        order: opts.order,
        epsilon,
        c,
        standardize: opts.standardize,
    })
}
pub(super) fn solve(
    e: &Engine,
    k: &[f64],
    y: &[f64],
    c: f64,
    epsilon: f64,
) -> Result<(Vec<f64>, f64)> {
    let n = y.len();
    let mut a = vec![0.; 2 * n];
    let mut f: Vec<_> = y.iter().map(|v| -v).collect();
    let sign = |i: usize| if i < n { 1. } else { -1. };
    let bound_tolerance = 1e-12 * c.min(1.);
    for iteration in 0usize..1_000_000 {
        if iteration.is_multiple_of(64) {
            e.check_cancel()?;
        }
        if iteration * n > 200_000_000 {
            return Err("fitrsvm: SMO work limit exceeded before KKT convergence".into());
        }
        let mut up = (f64::NEG_INFINITY, 0);
        let mut low = f64::INFINITY;
        for i in 0..2 * n {
            let s = sign(i);
            let v = -f[i % n] - s * epsilon;
            if ((s > 0. && a[i] < c - bound_tolerance) || (s < 0. && a[i] > bound_tolerance))
                && v > up.0
            {
                up = (v, i);
            }
            if (s > 0. && a[i] > bound_tolerance) || (s < 0. && a[i] < c - bound_tolerance) {
                low = low.min(v);
            }
        }
        if up.0 - low <= 1e-9 {
            return Ok(((0..n).map(|i| a[i] - a[i + n]).collect(), (up.0 + low) / 2.));
        }
        let i = up.1;
        let ii = i % n;
        let si = sign(i);
        let bound_i = if si > 0. { c - a[i] } else { a[i] };
        let mut best = (0., 0, 0.);
        for j in 0..2 * n {
            let sj = sign(j);
            if j == i {
                continue;
            }
            if !((sj > 0. && a[j] > bound_tolerance) || (sj < 0. && a[j] < c - bound_tolerance)) {
                continue;
            }
            let gap = up.0 + f[j % n] + sj * epsilon;
            if gap <= 0. {
                continue;
            }
            let bound = bound_i.min(if sj > 0. { a[j] } else { c - a[j] });
            let curvature =
                (k[ii * n + ii] + k[(j % n) * n + j % n] - 2. * k[ii * n + j % n]).max(0.);
            let d = if curvature > 0. {
                (gap / curvature).min(bound)
            } else {
                bound
            };
            let gain = d * (gap - 0.5 * d * curvature);
            if gain > best.0 {
                best = (gain, j, d);
            }
        }
        let (_, j, d) = best;
        if best.0 <= 0. || !d.is_finite() {
            return Err("fitrsvm: SMO stalled before KKT convergence".into());
        }
        a[i] = (a[i] + si * d).clamp(0., c);
        a[j] = (a[j] - sign(j) * d).clamp(0., c);
        for r in 0..n {
            f[r] += d * (k[r * n + ii] - k[r * n + j % n]);
        }
    }
    let up = (0..2 * n)
        .filter(|&i| {
            (sign(i) > 0. && a[i] < c - bound_tolerance) || (sign(i) < 0. && a[i] > bound_tolerance)
        })
        .map(|i| -f[i % n] - sign(i) * epsilon)
        .fold(f64::NEG_INFINITY, f64::max);
    let low = (0..2 * n)
        .filter(|&i| {
            (sign(i) > 0. && a[i] > bound_tolerance) || (sign(i) < 0. && a[i] < c - bound_tolerance)
        })
        .map(|i| -f[i % n] - sign(i) * epsilon)
        .fold(f64::INFINITY, f64::min);
    Err(format!(
        "fitrsvm: SMO iteration limit exceeded before KKT convergence (gap {})",
        up - low
    ))
}
impl Svm {
    pub(super) fn predict(&self, q: &[f64]) -> f64 {
        let z: Vec<_> = q
            .iter()
            .enumerate()
            .map(|(j, v)| (v - self.means[j]) / self.scales[j])
            .collect();
        self.bias
            + self
                .alpha
                .iter()
                .enumerate()
                .filter(|(_, a)| a.abs() > 1e-12)
                .map(|(i, a)| {
                    a * kernel(&row(&self.x, i), &z, &self.kernel, self.scale, self.order)
                })
                .sum::<f64>()
    }
    pub(super) fn property(&self, e: &Engine, name: &str) -> Result<Value> {
        let support: Vec<_> = (0..self.alpha.len())
            .filter(|&i| self.alpha[i].abs() > 1e-12)
            .collect();
        match name {
            "Alpha" => value(
                support.len(),
                1,
                support.iter().map(|&i| self.alpha[i]).collect(),
            ),
            "Bias" => value(1, 1, vec![self.bias]),
            "SupportVectors" => Ok(Value::Matrix(
                super::super::super::classifier_objects::take_rows(&self.x, &support)?,
            )),
            "IsSupportVector" => Ok(Value::Logical(LogicalMatrix::from_column_vec(
                self.alpha.len(),
                1,
                self.alpha
                    .iter()
                    .map(|v| u8::from(v.abs() > 1e-12))
                    .collect(),
                &e.cancel,
            )?)),
            "Mu" if self.standardize => value(1, self.means.len(), self.means.clone()),
            "Sigma" if self.standardize => value(1, self.scales.len(), self.scales.clone()),
            "Mu" | "Sigma" => value(0, 0, vec![]),
            "Epsilon" => value(1, 1, vec![self.epsilon]),
            "BoxConstraints" => value(self.alpha.len(), 1, vec![self.c; self.alpha.len()]),
            "KernelParameters" => structure(
                e,
                vec![
                    ("Function", Value::Text(self.kernel.clone())),
                    ("Scale", value(1, 1, vec![self.scale])?),
                ],
            ),
            "Beta" => {
                if self.kernel != "linear" {
                    return value(0, 0, vec![]);
                }
                value(
                    self.x.cols(),
                    1,
                    (0..self.x.cols())
                        .map(|j| {
                            self.alpha
                                .iter()
                                .enumerate()
                                .map(|(i, a)| {
                                    a * self.x.data()[i + j * self.x.rows()] / self.scale.powi(2)
                                })
                                .sum()
                        })
                        .collect(),
                )
            }
            _ => Err(format!("RegressionSVM: unknown property '{name}'")),
        }
    }
}
