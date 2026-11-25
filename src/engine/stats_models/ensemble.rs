//! Weighted CART ensembles, bootstrap OOB membership, and boosting updates.
use super::*;
#[derive(Clone, Debug)]
struct TreeNode {
    split: Option<(usize, f64, usize, usize)>,
    scores: Vec<f64>,
}
#[derive(Clone, Debug)]
struct Tree {
    nodes: Vec<TreeNode>,
    importance: Vec<f64>,
}
impl Tree {
    fn score(&self, x: &Matrix, i: usize) -> &[f64] {
        let mut at = 0;
        while let Some((j, t, l, r)) = self.nodes[at].split {
            at = if x.data()[i + j * x.rows()] <= t {
                l
            } else {
                r
            };
        }
        &self.nodes[at].scores
    }
    fn bytes(&self) -> usize {
        self.nodes
            .iter()
            .map(|n| 64 + 8 * n.scores.len())
            .sum::<usize>()
            + 8 * self.importance.len()
    }
}
#[derive(Clone, Debug)]
pub(in crate::engine) struct Ensemble {
    pub method: String,
    trees: Vec<Tree>,
    weights: Vec<f64>,
    oob: Vec<Vec<bool>>,
    classes: usize,
}
impl Ensemble {
    pub(in crate::engine) fn bytes(&self) -> usize {
        128 + self.trees.iter().map(Tree::bytes).sum::<usize>()
            + 8 * self.weights.len()
            + self.oob.iter().map(Vec::len).sum::<usize>()
    }
    pub(in crate::engine) fn property(&self, name: &str) -> Result<Value> {
        match name {
            "TrainedWeights" => value(1, self.weights.len(), self.weights.clone()),
            "NumTrained" => Ok(Value::Matrix(Matrix::scalar(self.trees.len() as f64))),
            "Method" => Ok(Value::Text(
                match self.method.as_str() {
                    "bag" => "Bag",
                    "adaboostm1" => "AdaBoostM1",
                    "adaboostm2" => "AdaBoostM2",
                    "gentleboost" => "GentleBoost",
                    "logitboost" => "LogitBoost",
                    _ => "RUSBoost",
                }
                .into(),
            )),
            _ => Err(format!("ClassificationEnsemble: unknown property '{name}'")),
        }
    }
    pub(in crate::engine) fn importance(&self) -> Vec<f64> {
        let p = self.trees[0].importance.len();
        let total = self.weights.iter().sum::<f64>();
        (0..p)
            .map(|j| {
                self.trees
                    .iter()
                    .zip(&self.weights)
                    .map(|(t, w)| t.importance[j] * w)
                    .sum::<f64>()
                    / total
            })
            .collect()
    }
    pub(in crate::engine) fn predict(
        &self,
        e: &Engine,
        x: &Matrix,
        oob: Option<usize>,
    ) -> Result<Matrix> {
        let n = x.rows();
        let k = self.classes;
        let mut scores = vec![0.; size(e, "predict", n, k)?];
        if (self.trees.len() as u128) * (n as u128) * (x.cols() as u128) > 200_000_000 {
            return Err("predict: ensemble work limit exceeded".into());
        }
        for i in 0..n {
            e.check_cancel()?;
            let mut sum = 0.;
            for (t, tree) in self.trees.iter().enumerate() {
                if oob.is_some() && !self.oob[t][i] {
                    continue;
                }
                sum += self.weights[t];
                let s = tree.score(x, i);
                if self.method == "bag"
                    || self.method == "adaboostm2"
                    || self.method == "rusboost" && k > 2
                {
                    for c in 0..k {
                        scores[i + c * n] += self.weights[t] * s[c];
                    }
                } else {
                    let f = if self.method == "adaboostm1" || self.method == "rusboost" {
                        if argmax(s) == 1 { 1. } else { -1. }
                    } else {
                        s[0]
                    };
                    scores[i] -= self.weights[t] * f;
                    scores[i + n] += self.weights[t] * f;
                }
            }
            if self.method == "bag" {
                for c in 0..k {
                    scores[i + c * n] = if sum == 0. {
                        f64::NAN
                    } else {
                        scores[i + c * n] / sum
                    };
                }
            }
        }
        Matrix::from_column_vec(n, k, scores)
    }
    pub(in crate::engine) fn oob_scores(&self, e: &Engine, x: &Matrix) -> Result<Matrix> {
        if self.method != "bag" {
            return Err("oobLoss: only Bag ensembles have out-of-bag observations".into());
        }
        self.predict(e, x, Some(0))
    }
}
fn argmax(s: &[f64]) -> usize {
    (0..s.len())
        .max_by(|&a, &b| s[a].total_cmp(&s[b]).then_with(|| b.cmp(&a)))
        .unwrap()
}
fn leaf(ids: &[usize], weights: &[f64], targets: &[Vec<f64>]) -> TreeNode {
    let mut scores = vec![0.; targets[0].len()];
    let mut sum = 0.;
    for &i in ids {
        sum += weights[i];
        for (c, s) in scores.iter_mut().enumerate() {
            *s += weights[i] * targets[i][c];
        }
    }
    if sum > 0. {
        for s in &mut scores {
            *s /= sum;
        }
    }
    TreeNode {
        split: None,
        scores,
    }
}
struct Candidate {
    node: usize,
    ids: Vec<usize>,
    feature: usize,
    threshold: f64,
    gain: f64,
}
struct Search<'a> {
    x: &'a Matrix,
    weights: &'a [f64],
    targets: &'a [Vec<f64>],
    min_leaf: usize,
}
fn candidate(
    e: &Engine,
    s: &Search<'_>,
    node: usize,
    ids: Vec<usize>,
    work: &mut usize,
) -> Result<Option<Candidate>> {
    if ids.len() < 2 * s.min_leaf {
        return Ok(None);
    }
    let dims = s.targets[0].len();
    let mut total = vec![0.; dims];
    let mut mass = 0.;
    for &i in &ids {
        mass += s.weights[i];
        for (c, t) in total.iter_mut().enumerate() {
            *t += s.weights[i] * s.targets[i][c];
        }
    }
    if mass <= 0. {
        return Ok(None);
    }
    let parent = total.iter().map(|t| t * t).sum::<f64>() / mass;
    let mut best = None;
    for j in 0..s.x.cols() {
        e.check_cancel()?;
        *work = work.saturating_add(ids.len() * dims);
        if *work > 50_000_000 {
            return Err("fitcensemble: tree split work limit exceeded".into());
        }
        let mut sorted = ids.clone();
        sorted.sort_unstable_by(|&a, &b| {
            s.x.data()[a + j * s.x.rows()]
                .total_cmp(&s.x.data()[b + j * s.x.rows()])
                .then(a.cmp(&b))
        });
        let mut left = vec![0.; dims];
        let mut lm = 0.;
        for t in 1..sorted.len() {
            let i = sorted[t - 1];
            lm += s.weights[i];
            for (c, l) in left.iter_mut().enumerate() {
                *l += s.weights[i] * s.targets[i][c];
            }
            let a = s.x.data()[i + j * s.x.rows()];
            let b = s.x.data()[sorted[t] + j * s.x.rows()];
            let rm = mass - lm;
            if t < s.min_leaf
                || sorted.len() - t < s.min_leaf
                || a == b
                || lm <= 1e-15
                || rm <= 1e-15
            {
                continue;
            }
            let gain = left
                .iter()
                .enumerate()
                .map(|(c, l)| l * l / lm + (total[c] - l).powi(2) / rm)
                .sum::<f64>()
                - parent;
            if gain > 1e-14 && best.is_none_or(|(g, _, _)| gain > g + 1e-12) {
                best = Some((gain, j, a * 0.5 + b * 0.5));
            }
        }
    }
    Ok(best.map(|(gain, feature, threshold)| Candidate {
        node,
        ids,
        feature,
        threshold,
        gain,
    }))
}
fn tree(
    e: &Engine,
    s: &Search<'_>,
    ids: Vec<usize>,
    splits: usize,
    work: &mut usize,
) -> Result<Tree> {
    let root_mass = ids.iter().map(|&i| s.weights[i]).sum::<f64>();
    let mut nodes = vec![leaf(&ids, s.weights, s.targets)];
    let mut pending = Vec::new();
    let mut importance = vec![0.; s.x.cols()];
    if let Some(c) = candidate(e, s, 0, ids, work)? {
        pending.push(c);
    }
    for _ in 0..splits {
        e.check_cancel()?;
        let Some(best) = (0..pending.len()).max_by(|&a, &b| {
            pending[a]
                .gain
                .total_cmp(&pending[b].gain)
                .then_with(|| pending[b].node.cmp(&pending[a].node))
        }) else {
            break;
        };
        let c = pending.swap_remove(best);
        let (left, right): (Vec<_>, Vec<_>) = c
            .ids
            .into_iter()
            .partition(|&i| s.x.data()[i + c.feature * s.x.rows()] <= c.threshold);
        let l = nodes.len();
        let r = l + 1;
        nodes[c.node].split = Some((c.feature, c.threshold, l, r));
        importance[c.feature] += c.gain;
        nodes.push(leaf(&left, s.weights, s.targets));
        nodes.push(leaf(&right, s.weights, s.targets));
        if let Some(c) = candidate(e, s, l, left, work)? {
            pending.push(c);
        }
        if let Some(c) = candidate(e, s, r, right, work)? {
            pending.push(c);
        }
    }
    // Mean impurity reduction per branch node, as in predictorImportance.
    let branches = (nodes.len() - 1) / 2;
    if branches > 0 {
        for v in &mut importance {
            *v /= branches as f64 * root_mass;
        }
    }
    Ok(Tree { nodes, importance })
}
#[allow(clippy::too_many_arguments)]
pub(super) fn fit(
    e: &mut Engine,
    x: &Matrix,
    codes: &[usize],
    classes: usize,
    method: &str,
    cycles: usize,
    rate: f64,
    splits: usize,
    min_leaf: usize,
) -> Result<Ensemble> {
    if classes < 2 {
        return Err("fitcensemble: at least two response classes are required".into());
    }
    if ["adaboostm1", "gentleboost", "logitboost"].contains(&method) && classes != 2 {
        return Err("fitcensemble: this Method requires binary responses".into());
    }
    if method == "adaboostm2" && classes < 3 {
        return Err("fitcensemble: AdaBoostM2 requires multiclass responses".into());
    }
    let n = x.rows();
    if cycles.saturating_mul(n).saturating_mul(classes) > 8_000_000 {
        return Err("fitcensemble: retained ensemble workspace limit exceeded".into());
    }
    size(e, "fitcensemble", n, classes + cycles)?;
    let mut weights = vec![1. / n as f64; n];
    let mut pair = vec![vec![0.; classes]; n];
    for i in 0..n {
        for (c, w) in pair[i].iter_mut().enumerate() {
            if c != codes[i] {
                *w = 1. / (n * (classes - 1)) as f64;
            }
        }
    }
    let mut f = vec![0.; n];
    let mut trees = Vec::new();
    let mut alphas = Vec::new();
    let mut oob = Vec::new();
    let mut work = 0;
    let mut retained = 0usize;
    for _ in 0..cycles {
        e.check_cancel()?;
        let mut fitweights = weights.clone();
        let mut ids: Vec<_> = (0..n).collect();
        let mut excluded = vec![false; n];
        if method == "bag" {
            ids.clear();
            excluded.fill(true);
            for _ in 0..n {
                let i = (e.random.next_f64() * n as f64) as usize;
                ids.push(i);
                excluded[i] = false;
            }
        }
        if method == "rusboost" {
            let mut groups = vec![Vec::new(); classes];
            for (i, &c) in codes.iter().enumerate() {
                groups[c].push(i);
            }
            let count = groups.iter().map(Vec::len).min().unwrap();
            ids.clear();
            for rows in &mut groups {
                for i in 0..count {
                    let j = i + (e.random.next_f64() * (rows.len() - i) as f64) as usize;
                    rows.swap(i, j);
                }
                ids.extend_from_slice(&rows[..count]);
            }
        }
        let mut targets = vec![vec![0.; classes]; n];
        if method == "gentleboost" {
            targets = (0..n)
                .map(|i| vec![if codes[i] == 1 { 1. } else { -1. }])
                .collect();
        } else if method == "logitboost" {
            targets = Vec::with_capacity(n);
            for i in 0..n {
                let prob = super::ml::sigmoid(2. * f[i]).clamp(1e-12, 1. - 1e-12);
                let v = prob * (1. - prob);
                targets.push(vec![((codes[i] as f64 - prob) / v).clamp(-10., 10.)]);
                fitweights[i] = v / n as f64;
            }
        } else {
            for i in 0..n {
                targets[i][codes[i]] = 1.;
            }
            if method == "adaboostm2" || method == "rusboost" && classes > 2 {
                for i in 0..n {
                    fitweights[i] = pair[i].iter().sum();
                }
            }
        }
        let search = Search {
            x,
            weights: &fitweights,
            targets: &targets,
            min_leaf,
        };
        let model = tree(e, &search, ids, splits, &mut work)?;
        let mut alpha = 1.;
        let mut perfect = false;
        if method == "adaboostm1" || method == "rusboost" && classes == 2 {
            let error = (0..n)
                .filter(|&i| argmax(model.score(x, i)) != codes[i])
                .map(|i| weights[i])
                .sum::<f64>();
            if error >= 0.5 - 1e-14 {
                if trees.is_empty() {
                    return Err("fitcensemble: weak learner has error at least one half".into());
                }
                break;
            }
            perfect = error < 1e-15;
            alpha = rate * 0.5 * ((1. - error.max(1e-15)) / error.max(1e-15)).ln();
            for i in 0..n {
                weights[i] *= (if argmax(model.score(x, i)) == codes[i] {
                    -alpha
                } else {
                    alpha
                })
                .exp();
            }
        } else if method == "adaboostm2" || method == "rusboost" && classes > 2 {
            let mut error = 0.;
            for i in 0..n {
                let s = model.score(x, i);
                for c in 0..classes {
                    if c != codes[i] {
                        error += 0.5 * pair[i][c] * (1. - s[codes[i]] + s[c]);
                    }
                }
            }
            if error >= 0.5 - 1e-14 {
                if trees.is_empty() {
                    return Err(
                        "fitcensemble: weak learner pseudo-loss is at least one half".into(),
                    );
                }
                break;
            }
            perfect = error < 1e-15;
            let beta = error.max(1e-15) / (1. - error.max(1e-15));
            alpha = rate * (-beta.ln());
            for i in 0..n {
                let s = model.score(x, i);
                for c in 0..classes {
                    if c != codes[i] {
                        pair[i][c] *= beta.powf(rate * 0.5 * (1. + s[codes[i]] - s[c]));
                    }
                }
            }
            let total = pair.iter().flatten().sum::<f64>();
            for row in &mut pair {
                for w in row {
                    *w /= total;
                }
            }
        } else if method == "gentleboost" || method == "logitboost" {
            alpha = rate * if method == "logitboost" { 0.5 } else { 1. };
            for i in 0..n {
                f[i] += alpha * model.score(x, i)[0];
                if method == "gentleboost" {
                    weights[i] = (-(if codes[i] == 1 { 1. } else { -1. }) * f[i]).exp();
                }
            }
        }
        let total = weights.iter().sum::<f64>();
        if !total.is_finite() || total <= 0. {
            return Err("fitcensemble: numerical overflow in observation weights".into());
        }
        for w in &mut weights {
            *w /= total;
        }
        retained = retained.saturating_add(model.bytes() + excluded.len());
        if retained > 128 * 1024 * 1024 {
            return Err("fitcensemble: retained models exceed 128 MiB".into());
        }
        e.preflight_elements(retained.div_ceil(8))?;
        trees.push(model);
        alphas.push(alpha);
        oob.push(excluded);
        if perfect {
            break;
        }
    }
    Ok(Ensemble {
        method: method.into(),
        trees,
        weights: alphas,
        oob,
        classes,
    })
}
