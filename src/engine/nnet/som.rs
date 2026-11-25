//! Kohonen batch SOM. Public contracts: MathWorks selforgmap, learnsomb,
//! trainbu, gridtop, hextop, randtop and {dist,linkdist,boxdist,mandist} pages.
//! Ordering epoch k uses radius 1+(initial-1)*max(0,1-k/coverSteps).
//! Empty neighborhoods retain their weights; ties select the first neuron.
use super::*;
use std::collections::VecDeque;
#[derive(Clone)]
pub(in crate::engine) struct Som {
    pub dimensions: Vec<usize>,
    pub topology: String,
    pub distance: String,
    pub positions: Matrix,
    pub distances: Matrix,
    pub cover: usize,
    pub neighbor: f64,
    pub(super) initial: Matrix,
}
impl Som {
    pub(super) fn bytes(&self) -> usize {
        (self.positions.len() + self.distances.len() + self.initial.len()) * 8
            + self.dimensions.len() * 8
    }
}
pub(super) fn handles(n: &str) -> bool {
    matches!(
        n,
        "selforgmap"
            | "gridtop"
            | "hextop"
            | "randtop"
            | "dist"
            | "linkdist"
            | "boxdist"
            | "mandist"
    )
}
fn dimensions(e: &Engine, v: &Value) -> Result<Vec<usize>> {
    let m = matrix(e, v)?;
    if m.rows() != 1 || m.cols() == 0 || m.cols() > 8 {
        return Err("SOM: dimensions must be a nonempty row vector of at most 8 dimensions".into());
    }
    let d = m
        .data()
        .iter()
        .map(|&x| size(x, false))
        .collect::<Result<Vec<_>>>()?;
    let n = d.iter().try_fold(1usize, |n, &x| {
        n.checked_mul(x).ok_or("SOM: dimension overflow")
    })?;
    if n > 2048 {
        return Err("SOM: at most 2048 neurons supported (bounded distance workspace)".into());
    }
    Ok(d)
}
fn positions(e: &mut Engine, dims: &[usize], kind: &str) -> Result<Matrix> {
    if !["gridtop", "hextop", "randtop"].contains(&kind) {
        return Err("SOM: unsupported topologyFcn".into());
    }
    let n = dims.iter().product();
    let mut v = vec![0.; n * dims.len()];
    for i in 0..n {
        let mut k = i;
        for (r, &d) in dims.iter().enumerate() {
            v[i * dims.len() + r] = (k % d) as f64;
            k /= d;
        }
        if kind == "hextop" && dims.len() > 1 {
            let y = v[i * dims.len() + 1];
            v[i * dims.len()] += (y as usize % 2) as f64 * 0.5;
            v[i * dims.len() + 1] *= 3f64.sqrt() / 2.;
        }
        if kind == "randtop" {
            for (r, &d) in dims.iter().enumerate() {
                v[i * dims.len() + r] = e.random.next_f64() * d.saturating_sub(1) as f64;
            }
        }
    }
    Matrix::from_column_vec(dims.len(), n, v)
}
pub(super) fn distances(e: &mut Engine, p: &Matrix, kind: &str) -> Result<Matrix> {
    if !["dist", "linkdist", "boxdist", "mandist"].contains(&kind) {
        return Err("SOM: unsupported distanceFcn".into());
    }
    let n = p.cols();
    let len = Matrix::checked_len(n, n)?;
    if n > 2048 || len.saturating_mul(24) > e.limits.memory_bytes {
        return Err("SOM: distance workspace limit exceeded".into());
    }
    e.charge(len.saturating_mul(p.rows().max(1)))?;
    let mut v = vec![0.; len];
    for j in 0..n {
        e.check_cancel()?;
        for i in 0..n {
            let d = (0..p.rows())
                .map(|r| (p.data()[r + i * p.rows()] - p.data()[r + j * p.rows()]).abs());
            v[i + j * n] = match kind {
                "boxdist" => d.fold(0., f64::max),
                "mandist" => d.sum(),
                _ => d.map(|x| x * x).sum::<f64>().sqrt(),
            };
        }
    }
    if kind == "linkdist" {
        let adjacent: Vec<Vec<usize>> = (0..n)
            .map(|i| {
                (0..n)
                    .filter(|&j| i != j && v[i + j * n] <= 1. + 1e-10)
                    .collect()
            })
            .collect();
        v.fill(f64::INFINITY);
        for s in 0..n {
            e.check_cancel()?;
            v[s + s * n] = 0.;
            let mut q = VecDeque::from([s]);
            while let Some(i) = q.pop_front() {
                for &j in &adjacent[i] {
                    if !v[j + s * n].is_finite() {
                        v[j + s * n] = v[i + s * n] + 1.;
                        q.push_back(j);
                    }
                }
            }
        }
    }
    if kind == "linkdist" {
        for x in &mut v {
            if !x.is_finite() {
                *x = n as f64;
            }
        }
    }
    Matrix::from_column_vec(n, n, v)
}
pub(super) fn defaults(c: &AtomicBool) -> Result<Array> {
    record(
        vec![
            ("epochs", num(1000.)),
            ("show", num(25.)),
            ("time", num(f64::INFINITY)),
            ("showWindow", Array::Logical(LogicalMatrix::scalar(true))),
            (
                "showCommandLine",
                Array::Logical(LogicalMatrix::scalar(false)),
            ),
        ],
        c,
    )
}
pub(super) fn call(e: &mut Engine, name: &str, args: &[Value]) -> Result<Vec<Value>> {
    if name != "selforgmap" {
        arity(name, args, 1)?;
        let m = if matches!(name, "gridtop" | "hextop" | "randtop") {
            positions(e, &dimensions(e, &args[0])?, name)?
        } else {
            let p = matrix(e, &args[0])?;
            if p.data().iter().any(|x| !x.is_finite()) {
                return Err("SOM: positions must be finite".into());
            }
            distances(e, &p, name)?
        };
        return Ok(vec![Value::Matrix(m)]);
    }
    if args.len() > 5 {
        return Err("selforgmap: too many inputs".into());
    }
    let d = if let Some(v) = args.first() {
        dimensions(e, v)?
    } else {
        vec![8, 8]
    };
    let cover = args
        .get(1)
        .map(|v| matrix(e, v).and_then(|m| size(m.as_scalar()?, true)))
        .transpose()?
        .unwrap_or(100);
    let neighbor = args
        .get(2)
        .map(|v| matrix(e, v).and_then(|m| m.as_scalar()))
        .transpose()?
        .unwrap_or(3.);
    if !neighbor.is_finite() || neighbor < 1. {
        return Err("selforgmap: initNeighbor must be finite and at least 1".into());
    }
    let topology = args
        .get(3)
        .map(|v| value_text(e, v))
        .transpose()?
        .unwrap_or("hextop".into());
    let distance = args
        .get(4)
        .map(|v| value_text(e, v))
        .transpose()?
        .unwrap_or("linkdist".into());
    let p = positions(e, &d, &topology)?;
    let distances = distances(e, &p, &distance)?;
    let mut o = properties::new_object("selforgmap", &[], "trainbu", "mse", &e.cancel)?;
    o.core.layers[0].size = p.cols();
    o.core.layers[0].transfer_fcn = "compet".into();
    o.core.layers[0].init_fcn = "initsompc".into();
    o.core.bias_connect[0] = false;
    for port in [&mut o.core.input, &mut o.core.output] {
        port.process_fcns.clear();
        port.process_params.clear();
    }
    o.props
        .insert("name".into(), string("Self-Organizing Map", &e.cancel)?);
    o.props
        .insert("divideFcn".into(), string("dividetrain", &e.cancel)?);
    o.props
        .insert("divideMode".into(), string("none", &e.cancel)?);
    o.props.insert(
        "plotFcns".into(),
        strings(
            &[
                "plotsomtop",
                "plotsomnc",
                "plotsomnd",
                "plotsomplanes",
                "plotsomhits",
                "plotsompos",
            ]
            .map(str::to_owned),
            &e.cancel,
        )?,
    );
    o.som = Some(Som {
        dimensions: d,
        topology,
        distance,
        positions: p,
        distances,
        cover,
        neighbor,
        initial: Matrix::filled(0, 0, 0.)?,
    });
    Ok(vec![super::super::objects::nnet_new(e, o)?])
}
pub(super) fn configure(e: &mut Engine, o: &mut Object, x: &Matrix) -> Result<()> {
    if x.cols() == 0 || x.rows() == 0 || x.data().iter().any(|v| !v.is_finite()) {
        return Err("SOM: finite nonempty inputs required".into());
    }
    let s = o.som.as_ref().unwrap().positions.cols();
    budget(
        e,
        &o.core,
        x.cols(),
        x.len().saturating_mul(4) + s.saturating_mul(x.rows()) * 4,
    )?;
    o.core.input.size = x.rows();
    o.core.output.size = s;
    o.core.resize_weights()?;
    o.core.input.range = (0..x.rows())
        .map(|r| {
            (0..x.cols())
                .map(|c| x.data()[r + c * x.rows()])
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| {
                    (lo.min(x), hi.max(x))
                })
        })
        .collect();
    // Principal component plane, centered at the data-range midpoint. Singular
    // vectors have a canonical sign (largest absolute component positive).
    let midpoint: Vec<_> = o
        .core
        .input
        .range
        .iter()
        .map(|&(lo, hi)| lo / 2. + hi / 2.)
        .collect();
    let mean: Vec<f64> = (0..x.rows())
        .map(|r| {
            (0..x.cols())
                .map(|c| x.data()[r + c * x.rows()] / x.cols() as f64)
                .sum()
        })
        .collect();
    let centered = nalgebra::DMatrix::from_fn(x.rows(), x.cols(), |r, c| {
        x.data()[r + c * x.rows()] - mean[r]
    });
    e.check_cancel()?;
    let svd = nalgebra::linalg::SVD::try_new(centered, true, false, f64::EPSILON, 10000)
        .ok_or("SOM: PCA did not converge")?;
    e.check_cancel()?;
    let u = svd.u.ok_or("SOM: missing principal vectors")?;
    let som = o.som.as_mut().unwrap();
    let mut w = vec![0.; s * x.rows()];
    for r in 0..x.rows() {
        for i in 0..s {
            w[i + r * s] = midpoint[r];
        }
    }
    for k in 0..som.positions.rows().min(svd.singular_values.len()) {
        let pivot = (0..x.rows())
            .max_by(|&a, &b| u[(a, k)].abs().total_cmp(&u[(b, k)].abs()))
            .unwrap();
        let scale = svd.singular_values[k] / (x.cols() as f64).sqrt() * u[(pivot, k)].signum();
        let (lo, hi) = (0..s)
            .map(|i| som.positions.data()[k + i * som.positions.rows()])
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| {
                (lo.min(x), hi.max(x))
            });
        if hi > lo {
            for i in 0..s {
                let a =
                    2. * (som.positions.data()[k + i * som.positions.rows()] - lo) / (hi - lo) - 1.;
                for r in 0..x.rows() {
                    w[i + r * s] += a * scale * u[(r, k)];
                }
            }
        }
    }
    som.initial = Matrix::from_column_vec(s, x.rows(), w)?;
    o.core.iw[0] = som.initial.clone();
    o.core.configured = true;
    Ok(())
}
pub(super) fn configure_call(e: &mut Engine, mut o: Object, args: &[Value]) -> Result<Vec<Value>> {
    if args.len() != 2 && args.len() != 3 {
        return Err("configure: SOM expects net,x[,t]".into());
    }
    configure(e, &mut o, &matrix(e, &args[1])?)?;
    Ok(vec![super::super::objects::nnet_new(e, o)?])
}
pub(super) fn initialize(_e: &mut Engine, o: &mut Object) -> Result<()> {
    if !o.core.configured {
        return Err("init: SOM is not configured".into());
    }
    o.core.iw[0] = o.som.as_ref().unwrap().initial.clone();
    Ok(())
}
pub(super) fn winners(e: &mut Engine, o: &Object, x: &Matrix) -> Result<Vec<usize>> {
    if !o.core.configured
        || x.rows() != o.core.input.size
        || x.data().iter().any(|v| !v.is_finite())
    {
        return Err("SOM: finite input dimensions must match configured network".into());
    }
    let w = &o.core.iw[0];
    budget(e, &o.core, x.cols(), x.len())?;
    let mut winners = Vec::with_capacity(x.cols());
    for c in 0..x.cols() {
        e.check_cancel()?;
        let mut best = 0;
        let mut score = f64::INFINITY;
        for i in 0..w.rows() {
            let d = (0..x.rows())
                .map(|r| (x.data()[r + c * x.rows()] - w.data()[i + r * w.rows()]).powi(2))
                .sum::<f64>();
            if d < score {
                score = d;
                best = i;
            }
        }
        winners.push(best);
    }
    Ok(winners)
}
pub(super) fn simulate(e: &mut Engine, o: &Object, v: &Value) -> Result<Value> {
    let x = matrix(e, v)?;
    let ids = winners(e, o, &x)?;
    // Full one-hot output uses the engine's ordinary dense numeric path. Admission
    // bounds S*Q before allocation; vec2ind therefore works without densification.
    let s = o.core.layers[0].size;
    budget(e, &o.core, x.cols(), s.saturating_mul(x.cols()))?;
    let mut y = vec![0.; Matrix::checked_len(s, x.cols())?];
    for (c, i) in ids.into_iter().enumerate() {
        y[i + c * s] = 1.;
    }
    Ok(Value::Matrix(Matrix::from_column_vec(s, x.cols(), y)?))
}
pub(super) fn train(e: &mut Engine, args: &[Value]) -> Result<Vec<Value>> {
    arity("train SOM", args, 2)?;
    let mut o = super::super::objects::nnet_value(e, &args[0])
        .ok_or("train: network required")?
        .as_ref()
        .clone();
    let x = matrix(e, &args[1])?;
    if !o.core.configured {
        configure(e, &mut o, &x)?;
    }
    let epochs = size(scalar(field(&o.props["trainParam"], "epochs")?)?, true)?;
    if epochs > 1_000_000 {
        return Err("train: epoch record limit exceeded".into());
    }
    let seconds = scalar(field(&o.props["trainParam"], "time")?)?;
    let som = o.som.as_ref().unwrap();
    let limit = if seconds.is_finite() {
        std::time::Duration::from_secs_f64(seconds.min(1e12))
    } else {
        std::time::Duration::MAX
    };
    let (weights, training) = som_train::train(
        &o.core.iw[0],
        &x,
        &som.distances,
        som_train::SomOptions {
            epochs,
            cover_steps: som.cover,
            initial_neighbor: som.neighbor,
            time_limit: limit,
            memory_bytes: e.limits.memory_bytes,
            cancel: &e.cancel,
        },
        &mut super::super::neural_app_hooks::report,
    )?;
    o.core.iw[0] = weights;
    let done = training.epochs.len() - 1;
    let tr = record(
        vec![
            ("trainFcn", string("trainbu", &e.cancel)?),
            ("num_epochs", num(done as f64)),
            ("trainParam", o.props["trainParam"].clone()),
            (
                "perf",
                Array::Numeric(Matrix::from_column_vec(
                    1,
                    training.epochs.len(),
                    training.epochs.iter().map(|r| r.perf).collect(),
                )?),
            ),
            (
                "time",
                Array::Numeric(Matrix::from_column_vec(
                    1,
                    training.epochs.len(),
                    training.epochs.iter().map(|r| r.time).collect(),
                )?),
            ),
            (
                "epoch",
                Array::Numeric(Matrix::from_column_vec(
                    1,
                    done + 1,
                    (0..=done).map(|x| x as f64).collect(),
                )?),
            ),
            ("stop", string(&training.stop, &e.cancel)?),
        ],
        &e.cancel,
    )?;
    Ok(vec![
        super::super::objects::nnet_new(e, o)?,
        Value::from(tr),
    ])
}
