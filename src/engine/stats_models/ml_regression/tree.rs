//! CART squared-error splits, weakest-link pruning, LSBoost and bootstrap bagging.
use super::*;
use std::{cmp::Ordering, collections::BinaryHeap};

#[derive(Clone, Debug)]
struct Node {
    split: Option<(usize, f64, usize, usize)>,
    mean: f64,
    risk: f64,
    count: usize,
    gain: f64,
}
#[derive(Clone, Debug)]
pub(super) struct Tree {
    nodes: Vec<Node>,
}
struct Work {
    gain: f64,
    node: usize,
    rows: Vec<usize>,
    feature: usize,
    cut: f64,
}
impl PartialEq for Work {
    fn eq(&self, b: &Self) -> bool {
        self.gain == b.gain && self.node == b.node
    }
}
impl Eq for Work {}
impl PartialOrd for Work {
    fn partial_cmp(&self, b: &Self) -> Option<Ordering> {
        Some(self.cmp(b))
    }
}
impl Ord for Work {
    fn cmp(&self, b: &Self) -> Ordering {
        self.gain
            .total_cmp(&b.gain)
            .then_with(|| b.node.cmp(&self.node))
    }
}
fn node(y: &[f64], ids: &[usize]) -> Node {
    let mean = ids.iter().map(|&i| y[i]).sum::<f64>() / ids.len() as f64;
    let risk = ids.iter().map(|&i| (y[i] - mean).powi(2)).sum();
    Node {
        split: None,
        mean,
        risk,
        count: ids.len(),
        gain: 0.,
    }
}
fn candidate(
    e: &Engine,
    x: &Matrix,
    y: &[f64],
    ids: Vec<usize>,
    (index, n): (usize, &Node),
    opts: &TreeOptions,
    work: &mut usize,
) -> Result<Option<Work>> {
    if ids.len() < opts.parent.max(opts.leaf.saturating_mul(2)) || n.risk == 0. {
        return Ok(None);
    }
    *work = work.saturating_add(ids.len().saturating_mul(x.cols()));
    if *work > 100_000_000 {
        return Err("fitrtree: split search work limit exceeded".into());
    }
    let mut order = ids.clone();
    let mut best = (0., 0, 0.);
    for j in 0..x.cols() {
        e.check_cancel()?;
        let col = &x.data()[j * x.rows()..(j + 1) * x.rows()];
        order.sort_by(|&a, &b| col[a].total_cmp(&col[b]).then(a.cmp(&b)));
        let mut sum = 0.;
        let total: f64 = order.iter().map(|&i| y[i] - n.mean).sum();
        for k in 1..order.len() {
            sum += y[order[k - 1]] - n.mean;
            if k < opts.leaf || order.len() - k < opts.leaf || col[order[k - 1]] >= col[order[k]] {
                continue;
            }
            let gain = sum * sum / k as f64 + (total - sum).powi(2) / (order.len() - k) as f64
                - total * total / order.len() as f64;
            if gain > best.0 {
                let lo = col[order[k - 1]];
                let hi = col[order[k]];
                let mid = lo / 2. + hi / 2.;
                best = (gain, j, if mid > lo { mid } else { hi });
            }
        }
    }
    if !best.0.is_finite() {
        return Err("fitrtree: numerical overflow; rescale the response".into());
    }
    if best.0 <= 1e-14 * n.risk {
        return Ok(None);
    }
    Ok(Some(Work {
        gain: best.0,
        node: index,
        rows: ids,
        feature: best.1,
        cut: best.2,
    }))
}
pub(super) fn train(
    e: &Engine,
    x: &Matrix,
    y: &[f64],
    ids: &[usize],
    opts: &TreeOptions,
) -> Result<Tree> {
    let mut nodes = vec![node(y, ids)];
    if !nodes[0].risk.is_finite() || !nodes[0].mean.is_finite() {
        return Err("fitrtree: numerical overflow; rescale the response".into());
    }
    let mut queue = BinaryHeap::new();
    let mut work = 0;
    if opts.splits > 0
        && let Some(w) = candidate(e, x, y, ids.to_vec(), (0, &nodes[0]), opts, &mut work)?
    {
        queue.push(w);
    }
    for _ in 0..opts.splits {
        e.check_cancel()?;
        let Some(w) = queue.pop() else {
            break;
        };
        let (left, right): (Vec<_>, Vec<_>) = w
            .rows
            .into_iter()
            .partition(|&i| x.data()[i + w.feature * x.rows()] < w.cut);
        let a = nodes.len();
        let b = a + 1;
        nodes[w.node].split = Some((w.feature, w.cut, a, b));
        nodes[w.node].gain = w.gain;
        nodes.push(node(y, &left));
        nodes.push(node(y, &right));
        for (rows, index) in [(left, a), (right, b)] {
            if let Some(w) = candidate(e, x, y, rows, (index, &nodes[index]), opts, &mut work)? {
                queue.push(w);
            }
        }
    }
    Ok(Tree { nodes })
}
impl Tree {
    pub(super) fn bytes(&self) -> usize {
        self.nodes.len() * std::mem::size_of::<Node>()
    }
    pub(super) fn predict(&self, q: &[f64]) -> f64 {
        let mut i = 0;
        while let Some((j, cut, a, b)) = self.nodes[i].split {
            i = if q[j] < cut { a } else { b };
        }
        self.nodes[i].mean
    }
    pub(super) fn importance(&self, p: usize) -> Vec<f64> {
        let mut imp = vec![0.; p];
        let branches = self
            .nodes
            .iter()
            .filter(|n| n.split.is_some())
            .count()
            .max(1) as f64;
        for n in &self.nodes {
            if let Some((j, ..)) = n.split {
                imp[j] += n.gain / self.nodes[0].count as f64 / branches;
            }
        }
        imp
    }
    pub(super) fn view(&self, names: &[String]) -> String {
        let mut s = String::from("Decision tree for regression\n");
        for (i, n) in self.nodes.iter().enumerate() {
            if let Some((j, c, a, b)) = n.split {
                s.push_str(&format!(
                    "{}  if {} < {} then node {} else node {}\n",
                    i + 1,
                    names[j],
                    c,
                    a + 1,
                    b + 1
                ));
            } else {
                s.push_str(&format!("{}  fit = {}\n", i + 1, n.mean));
            }
        }
        s
    }
    pub(super) fn property(&self, e: &Engine, name: &str, names: &[String]) -> Result<Value> {
        let n = self.nodes.len();
        match name {
            "NumNodes" => value(1, 1, vec![n as f64]),
            "NodeMean" => value(n, 1, self.nodes.iter().map(|n| n.mean).collect()),
            "CutPoint" => value(
                n,
                1,
                self.nodes
                    .iter()
                    .map(|n| n.split.map_or(f64::NAN, |(_, c, _, _)| c))
                    .collect(),
            ),
            "Children" => value(
                n,
                2,
                (0..2)
                    .flat_map(|side| {
                        self.nodes.iter().map(move |n| {
                            n.split.map_or(0., |(_, _, a, b)| {
                                if side == 0 {
                                    (a + 1) as f64
                                } else {
                                    (b + 1) as f64
                                }
                            })
                        })
                    })
                    .collect(),
            ),
            "CutPredictor" => strings(
                e,
                &self
                    .nodes
                    .iter()
                    .map(|n| n.split.map_or(String::new(), |(j, ..)| names[j].clone()))
                    .collect::<Vec<_>>(),
            ),
            _ => Err(format!("RegressionTree: unknown property '{name}'")),
        }
    }
    fn prune_level(&mut self) {
        let n = self.nodes.len();
        let mut risks = vec![0.; n];
        let mut leaves = vec![1usize; n];
        let mut alpha = vec![f64::INFINITY; n];
        let mut reachable = vec![false; n];
        reachable[0] = true;
        for i in 0..n {
            if reachable[i]
                && let Some((_, _, a, b)) = self.nodes[i].split
            {
                reachable[a] = true;
                reachable[b] = true;
            }
        }
        for i in (0..n).rev() {
            risks[i] = self.nodes[i].risk;
            if let Some((_, _, a, b)) = self.nodes[i].split {
                risks[i] = risks[a] + risks[b];
                leaves[i] = leaves[a] + leaves[b];
                if reachable[i] {
                    alpha[i] = ((self.nodes[i].risk - risks[i]) / (leaves[i] - 1) as f64).max(0.);
                }
            }
        }
        let min = alpha.iter().copied().fold(f64::INFINITY, f64::min);
        if min.is_finite() {
            for (i, a) in alpha.iter().enumerate() {
                if *a <= min + 1e-12 * min.abs().max(1.) {
                    self.nodes[i].split = None;
                    self.nodes[i].gain = 0.;
                }
            }
        }
    }
    fn compact(&mut self) {
        let mut order = vec![0];
        let mut k = 0;
        while k < order.len() {
            if let Some((_, _, a, b)) = self.nodes[order[k]].split {
                order.push(a);
                order.push(b);
            }
            k += 1;
        }
        let mut map = vec![0; self.nodes.len()];
        for (i, &j) in order.iter().enumerate() {
            map[j] = i;
        }
        self.nodes = order
            .iter()
            .map(|&i| {
                let mut n = self.nodes[i].clone();
                if let Some((j, c, a, b)) = n.split {
                    n.split = Some((j, c, map[a], map[b]));
                }
                n
            })
            .collect();
    }
}
pub(super) fn prune(e: &mut Engine, m: &Model, args: &[Value]) -> Result<Value> {
    let Kind::Tree(original) = &m.kind else {
        return Err("prune: expected a RegressionTree".into());
    };
    if args.len() > 2 {
        return Err("prune: give one of Level or Nodes".into());
    }
    let mut t = original.clone();
    let mut level = 0;
    for (key, v) in options("prune", args, &["level", "nodes"])? {
        if key == "level" {
            let k = nonnegative(e, v, "prune")?;
            if k.fract() != 0. || k > 20_000. {
                return Err("prune: invalid Level".into());
            }
            level = k as usize;
        } else {
            let ids = real(e, "prune", v)?;
            for &i in ids.data() {
                if !i.is_finite() || i < 1. || i.fract() != 0. || i > t.nodes.len() as f64 {
                    return Err("prune: invalid node index".into());
                }
                t.nodes[i as usize - 1].split = None;
                t.nodes[i as usize - 1].gain = 0.;
            }
            level = 0;
        }
    }
    for _ in 0..level {
        e.check_cancel()?;
        if t.nodes[0].split.is_none() {
            break;
        }
        t.prune_level();
    }
    t.compact();
    let mut pruned = m.clone();
    pruned.kind = Kind::Tree(t);
    super::super::super::objects::mlr_new(e, pruned)
}
#[derive(Clone, Debug)]
pub(super) struct Ensemble {
    pub trees: Vec<Tree>,
    pub inbag: Vec<Vec<bool>>,
    pub bag: bool,
    base: f64,
    rate: f64,
}
pub(super) fn ensemble(
    e: &mut Engine,
    x: &Matrix,
    y: &[f64],
    config: (bool, usize, f64),
    opts: &TreeOptions,
) -> Result<Ensemble> {
    let (bag, cycles, rate) = config;
    if cycles.saturating_mul(x.len()) > 20_000_000 {
        return Err("fitrensemble: training work limit exceeded".into());
    }
    size(e, "fitrensemble", cycles, x.rows().saturating_mul(20))?;
    let n = y.len();
    let base = if bag {
        0.
    } else {
        y.iter().sum::<f64>() / n as f64
    };
    let mut fit = vec![base; n];
    let mut trees = Vec::with_capacity(cycles);
    let mut inbag = Vec::new();
    for _ in 0..cycles {
        e.check_cancel()?;
        let target: Vec<_> = if bag {
            y.to_vec()
        } else {
            y.iter().zip(&fit).map(|(y, f)| y - f).collect()
        };
        let ids = if bag {
            let mut seen = vec![false; n];
            let ids = (0..n)
                .map(|_| {
                    let i = (e.random.next_f64() * n as f64) as usize;
                    seen[i] = true;
                    i
                })
                .collect::<Vec<_>>();
            inbag.push(seen);
            ids
        } else {
            (0..n).collect()
        };
        let t = train(e, x, &target, &ids, opts)?;
        if !bag {
            for (i, f) in fit.iter_mut().enumerate() {
                *f += rate * t.predict(&row(x, i));
            }
        }
        trees.push(t);
    }
    Ok(Ensemble {
        trees,
        inbag,
        bag,
        base,
        rate,
    })
}
impl Ensemble {
    pub(super) fn predict(&self, q: &[f64]) -> f64 {
        let sum: f64 = self.trees.iter().map(|t| t.predict(q)).sum();
        if self.bag {
            sum / self.trees.len() as f64
        } else {
            self.base + self.rate * sum
        }
    }
    pub(super) fn importance(&self, p: usize) -> Vec<f64> {
        let mut v = vec![0.; p];
        for t in &self.trees {
            for (a, b) in v.iter_mut().zip(t.importance(p)) {
                *a += b / self.trees.len() as f64;
            }
        }
        v
    }
}
pub(super) fn oob_loss(e: &Engine, m: &Model, args: &[Value]) -> Result<Value> {
    let Kind::Ensemble(s) = &m.kind else {
        return Err("oobLoss: expected a bagged ensemble".into());
    };
    if !s.bag {
        return Err("oobLoss: only Method Bag has out-of-bag observations".into());
    }
    let mut cumulative = true;
    for (_, v) in options("oobLoss", args, &["mode"])? {
        let mode = opt_text(e, v, "oobLoss")?;
        if !["cumulative", "ensemble"].contains(&mode.as_str()) {
            return Err("oobLoss: Mode must be cumulative or ensemble".into());
        }
        cumulative = mode == "cumulative";
    }
    let n = m.y.len();
    let mut sums = vec![0.; n];
    let mut counts = vec![0usize; n];
    let mut losses = Vec::new();
    for (k, t) in s.trees.iter().enumerate() {
        e.check_cancel()?;
        for i in 0..n {
            if !s.inbag[k][i] {
                sums[i] += t.predict(&row(&m.x, i));
                counts[i] += 1;
            }
        }
        let mut loss = 0.;
        let mut used = 0;
        for i in 0..n {
            if counts[i] > 0 {
                loss += (m.y.data()[i] - sums[i] / counts[i] as f64).powi(2);
                used += 1;
            }
        }
        losses.push(if used == 0 {
            f64::NAN
        } else {
            loss / used as f64
        });
    }
    if !cumulative {
        return value(1, 1, vec![*losses.last().unwrap()]);
    }
    value(losses.len(), 1, losses)
}
