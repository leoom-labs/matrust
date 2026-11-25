//! Lloyd updates, PAM BUILD/SWAP (Kaufman & Rousseeuw, Finding Groups in
//! Data, 1990), and Lance-Williams hierarchical updates. Ties use ascending
//! cluster identifiers, then ascending observation indices.
use super::distance::Metric;
use super::*;
use std::{cmp::Ordering, collections::BinaryHeap, sync::Arc};

pub(super) fn call(
    e: &mut Engine,
    name: &str,
    a: &[Value],
    out: &mut Execution,
) -> Result<Vec<Value>> {
    match name {
        "kmeans" | "kmedoids" => partition(e, name, a, out),
        "linkage" => linkage(e, a, out),
        "dendrogram" => dendrogram(e, a, out),
        "cluster" | "inconsistent" => tree_call(e, name, a),
        "confusionmat" => confusion(e, a),
        _ => Err(format!("{name}: not supported yet")),
    }
}

fn partition(e: &mut Engine, name: &str, a: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
    image_segment_partition(e, name, a, None, out)
}

// Image segmentation reuses the RNG, seeding, Lloyd updates and empty-cluster repair.
pub(super) fn image_segment_partition(
    e: &mut Engine,
    name: &str,
    a: &[Value],
    threshold: Option<f64>,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if a.len() < 2 {
        return Err(format!("{name}: expected X and k"));
    }
    let original = real(e, name, &a[0])?;
    let k = integer(e, name, &a[1])?;
    let keep = clean_rows(e, name, &original, None)?;
    let (n, p) = (keep.len(), original.cols());
    if n < k || p == 0 {
        return Err(format!("{name}: k exceeds complete rows or no variables"));
    }
    size(e, name, n, k)?;
    size(e, name, original.rows(), k)?;
    size(e, name, k, p)?;
    let source = &original;
    let mut x = Matrix::from_column_vec(
        n,
        p,
        (0..p)
            .flat_map(|c| {
                keep.iter()
                    .map(move |&r| source.data()[r + c * source.rows()])
            })
            .collect(),
    )?;
    let mut start = None;
    let mut reps = 1;
    let mut reps_set = false;
    let mut maxiter = 100;
    let mut metric_arg = None;
    let mut empty = "singleton".to_string();
    let mut online = false;
    let mut display = "off".to_string();
    for (key, v) in options(
        name,
        &a[2..],
        &[
            "start",
            "replicates",
            "maxiter",
            "distance",
            "emptyaction",
            "onlinephase",
            "display",
            "algorithm",
        ],
    )? {
        match key.as_str() {
            "start" => start = Some(v),
            "replicates" => {
                reps = integer(e, name, v)?;
                reps_set = true;
            }
            "maxiter" => maxiter = integer(e, name, v)?,
            "distance" => metric_arg = Some(v),
            "emptyaction" => {
                empty = text(v)
                    .ok_or("kmeans: EmptyAction must be text")?
                    .to_lowercase()
            }
            "onlinephase" => online = boolean(e, name, v)?,
            "display" => {
                display = text(v)
                    .ok_or("kmeans: Display must be text")?
                    .to_lowercase()
            }
            _ => {
                if !text(v).is_some_and(|s| s.eq_ignore_ascii_case("pam")) {
                    return Err("kmedoids: only Algorithm pam is supported; other algorithms are not supported yet".into());
                }
            }
        }
    }
    if !["singleton", "error", "drop"].contains(&empty.as_str())
        || !["off", "final", "iter"].contains(&display.as_str())
    {
        return Err(format!("{name}: invalid EmptyAction or Display"));
    }
    let default = Value::Text(
        if name == "kmeans" {
            "sqeuclidean"
        } else {
            "euclidean"
        }
        .into(),
    );
    let metric = Metric::new(e, name, &x, metric_arg.or(Some(&default)), None)?;
    if name == "kmeans"
        && ![
            "sqeuclidean",
            "squaredeuclidean",
            "cityblock",
            "cosine",
            "correlation",
        ]
        .contains(&metric.name.as_str())
    {
        return Err("kmeans: unsupported distance".into());
    }
    // Correlation first centers each observation; spherical methods then
    // normalize each observation, and retain means in this transformed space.
    if name == "kmeans" && ["cosine", "correlation"].contains(&metric.name.as_str()) {
        let mut data = x.data().to_vec();
        for r in 0..n {
            let mean = if metric.name == "correlation" {
                (0..p).map(|c| data[r + c * n]).sum::<f64>() / p as f64
            } else {
                0.
            };
            let norm = (0..p)
                .map(|c| (data[r + c * n] - mean).powi(2))
                .sum::<f64>()
                .sqrt();
            if norm == 0. {
                return Err(
                    "kmeans: zero-length or constant observations cannot use this distance".into(),
                );
            }
            for c in 0..p {
                let scale = if metric.name == "correlation" {
                    (p - 1) as f64
                } else {
                    1.
                };
                data[r + c * n] = (data[r + c * n] - mean) / norm * scale.sqrt();
            }
        }
        x = Matrix::from_column_vec(n, p, data)?;
    }
    let explicit = if let Some(v) = start {
        if text(v).is_none() {
            Some(real(e, name, v)?)
        } else {
            None
        }
    } else {
        None
    };
    if let Some(s) = &explicit {
        if s.rows() != k || s.cols() != p {
            return Err(format!(
                "{name}: Start must be k by p; N-D starts are not supported yet"
            ));
        }
        if s.data().iter().any(|v| !v.is_finite()) {
            return Err(format!("{name}: Start must be finite"));
        }
        if reps_set && reps != 1 {
            return Err(format!("{name}: a 2-D Start requires Replicates 1"));
        }
        reps = 1;
    }
    let mode = start
        .and_then(text)
        .unwrap_or(if name == "kmedoids" { "build" } else { "plus" })
        .to_lowercase();
    if !["plus", "sample", "uniform", "cluster", "build"].contains(&mode.as_str()) {
        return Err(format!("{name}: unknown Start"));
    }
    if name == "kmeans" && ["cluster", "build"].contains(&mode.as_str()) {
        return Err("kmeans: unsupported Start".into());
    }
    if name == "kmedoids" && mode == "uniform" {
        return Err("kmedoids: unsupported Start".into());
    }
    let all_dist = if name == "kmedoids" {
        size(e, name, n, n)?;
        let mut d = vec![0.; n * n];
        for j in 0..n {
            e.check_cancel()?;
            let col = metric.batch(e, name, &row(&x, j), &x, out)?;
            for i in 0..n {
                if !col[i].is_finite() {
                    return Err("kmedoids: distances must be finite".into());
                }
                d[i + j * n] = col[i];
            }
        }
        Some(d)
    } else {
        None
    };
    let mut best_cost = f64::INFINITY;
    let mut best = None;
    for rep in 0..reps {
        e.check_cancel()?;
        let (mut centers, mut medoids) = if let Some(s) = &explicit {
            let centers: Vec<_> = (0..k).map(|r| row(s, r)).collect();
            let mut medoids = Vec::new();
            if name == "kmedoids" {
                for c in &centers {
                    let i = (0..n)
                        .find(|&i| row(&x, i) == *c)
                        .ok_or("kmedoids: every Start row must be an observation")?;
                    if medoids.contains(&i) {
                        return Err("kmedoids: Start medoids must be distinct".into());
                    }
                    medoids.push(i);
                }
            }
            (centers, medoids)
        } else if name == "kmedoids" && mode == "build" {
            let d = all_dist.as_ref().expect("PAM distances");
            let m = pam_build(e, d, n, k)?;
            (m.iter().map(|&i| row(&x, i)).collect(), m)
        } else if name == "kmedoids" && mode == "cluster" {
            let d = all_dist.as_ref().expect("PAM distances");
            let count = n.div_ceil(10).max(k);
            let mut sample: Vec<_> = (0..n).collect();
            for i in 0..count {
                let j = i + (e.random.next_f64() * (n - i) as f64).floor() as usize;
                sample.swap(i, j);
            }
            sample.truncate(count);
            let mut sub = vec![0.; count * count];
            for i in 0..count {
                for j in 0..count {
                    sub[i + j * count] = d[sample[i] + sample[j] * n];
                }
            }
            // Preliminary PAM starts from the first k sampled observations.
            let mut medoids: Vec<_> = (0..k).collect();
            pam_swap(e, &sub, count, &mut medoids, maxiter)?;
            let medoids: Vec<_> = medoids.into_iter().map(|i| sample[i]).collect();
            (medoids.iter().map(|&i| row(&x, i)).collect(), medoids)
        } else {
            initial(e, &x, k, &mode, &metric, all_dist.as_deref())?
        };
        let mut active = vec![true; k];
        let mut labels = vec![usize::MAX; n];
        let mut converged = false;
        let mut iterations = 0;
        if let Some(d) = &all_dist {
            (converged, iterations) = pam_swap(e, d, n, &mut medoids, maxiter)?;
            centers = medoids.iter().map(|&i| row(&x, i)).collect();
            for i in 0..n {
                labels[i] = (0..k)
                    .min_by(|&a, &b| {
                        d[i + medoids[a] * n]
                            .total_cmp(&d[i + medoids[b] * n])
                            .then(a.cmp(&b))
                    })
                    .expect("nonzero k");
            }
            for (j, &i) in medoids.iter().enumerate() {
                labels[i] = j;
            }
        } else {
            for iteration in 0..maxiter {
                e.check_cancel()?;
                iterations = iteration + 1;
                let previous = labels.clone();
                assign(e, &x, &centers, &active, &metric, &mut labels)?;
                repair_empty(
                    name,
                    &x,
                    &mut centers,
                    &mut labels,
                    &mut active,
                    &metric,
                    &empty,
                )?;
                let image_segment_previous = threshold.map(|_| centers.clone());
                centers = centroids(&x, &labels, &active, &metric.name);
                if let (Some(t), Some(old)) = (threshold, image_segment_previous)
                    && old.iter().zip(&centers).all(|(a, b)| {
                        a.iter()
                            .zip(b)
                            .map(|(x, y)| (x - y).powi(2))
                            .sum::<f64>()
                            .sqrt()
                            < t
                    })
                {
                    converged = true;
                    break;
                }
                if display == "iter" {
                    e.emit(
                        out,
                        format!("{name}: replicate {}, iteration {}", rep + 1, iteration + 1),
                    )?;
                }
                if labels == previous {
                    converged = true;
                    break;
                }
            }
            if online {
                // Sequential exact objective updates: move one observation at
                // a time only when recomputing both affected centers improves it.
                converged = false;
                let remaining = maxiter.saturating_sub(iterations);
                for _ in 0..remaining {
                    e.check_cancel()?;
                    iterations += 1;
                    let mut changed = false;
                    for i in 0..n {
                        e.check_cancel()?;
                        let old = labels[i];
                        if labels.iter().filter(|&&v| v == old).count() <= 1 {
                            continue;
                        }
                        let baseline = objective(&x, &centers, &labels, &metric);
                        let mut best_move = None;
                        let mut best = baseline;
                        for target in 0..k {
                            if target == old || !active[target] {
                                continue;
                            }
                            labels[i] = target;
                            let trial = centroids(&x, &labels, &active, &metric.name);
                            let cost = objective(&x, &trial, &labels, &metric);
                            if cost < best - 1e-12 * baseline.abs().max(1.) {
                                best = cost;
                                best_move = Some((target, trial));
                            }
                        }
                        labels[i] = old;
                        if let Some((target, trial)) = best_move {
                            labels[i] = target;
                            centers = trial;
                            changed = true;
                        }
                    }
                    if !changed {
                        converged = true;
                        break;
                    }
                }
            }
        }
        if !converged {
            warning(
                e,
                out,
                &format!("{name}: MaxIter reached before convergence"),
            )?;
        }
        if name == "kmeans"
            && centers
                .iter()
                .zip(&active)
                .any(|(center, &active)| active && !metric.pair(center, center).is_finite())
        {
            return Err("kmeans: distance is undefined for an updated center".into());
        }
        let mut distances = vec![f64::NAN; original.rows() * k];
        let mut sums = vec![0.; k];
        for j in 0..k {
            if !active[j] {
                sums[j] = f64::NAN;
                continue;
            }
            for (i, &r) in keep.iter().enumerate() {
                let d = if let Some(all) = &all_dist {
                    all[i + medoids[j] * n]
                } else {
                    metric.pair(&row(&x, i), &centers[j])
                };
                distances[r + j * original.rows()] = d;
                if labels[i] == j {
                    sums[j] += d;
                }
            }
        }
        if sums
            .iter()
            .zip(&active)
            .any(|(v, &active)| active && !v.is_finite())
        {
            return Err(format!("{name}: total cluster distance is not finite"));
        }
        let cost = sums.iter().filter(|v| v.is_finite()).sum::<f64>();
        if !cost.is_finite() {
            return Err(format!("{name}: total distance overflow"));
        }
        if display != "off" {
            e.emit(
                out,
                format!(
                    "{name}: replicate {}, {} iterations, total distance {cost}",
                    rep + 1,
                    iterations
                ),
            )?;
        }
        if cost < best_cost {
            best_cost = cost;
            best = Some((labels, centers, sums, distances, medoids));
        }
    }
    let (labels, centers, sums, distances, medoids) =
        best.ok_or_else(|| format!("{name}: no finite clustering found"))?;
    let mut idx = vec![f64::NAN; original.rows()];
    for (i, &r) in keep.iter().enumerate() {
        idx[r] = (labels[i] + 1) as f64;
    }
    let mut cs = vec![0.; k * p];
    for j in 0..p {
        for i in 0..k {
            cs[i + j * k] = centers[i][j];
        }
    }
    let mut result = vec![
        value(original.rows(), 1, idx)?,
        value(k, p, cs)?,
        value(k, 1, sums)?,
        value(original.rows(), k, distances)?,
    ];
    if name == "kmedoids" {
        result.push(value(
            k,
            1,
            medoids.iter().map(|&i| (keep[i] + 1) as f64).collect(),
        )?);
    }
    Ok(result)
}
fn initial(
    e: &mut Engine,
    x: &Matrix,
    k: usize,
    mode: &str,
    metric: &Metric,
    cached: Option<&[f64]>,
) -> Result<(Vec<Vec<f64>>, Vec<usize>)> {
    if mode == "uniform" {
        let mut centers = vec![vec![0.; x.cols()]; k];
        for j in 0..x.cols() {
            let min = (0..x.rows())
                .map(|r| x.data()[r + j * x.rows()])
                .fold(f64::INFINITY, f64::min);
            let max = (0..x.rows())
                .map(|r| x.data()[r + j * x.rows()])
                .fold(f64::NEG_INFINITY, f64::max);
            for center in &mut centers {
                center[j] = min + (max - min) * e.random.next_f64();
            }
        }
        return Ok((centers, vec![]));
    }
    let mut order: Vec<_> = (0..x.rows()).collect();
    let mut chosen = Vec::new();
    let mut nearest = vec![f64::INFINITY; x.rows()];
    for j in 0..k {
        e.check_cancel()?;
        let u = e.random.next_f64();
        let index = if mode == "sample" {
            let at = j + (u * (x.rows() - j) as f64).floor() as usize;
            order.swap(j, at);
            order[j]
        } else if j == 0 {
            (u * x.rows() as f64).floor() as usize
        } else {
            let total = nearest.iter().sum::<f64>();
            let mut target = u * total;
            let mut index = (0..x.rows())
                .find(|i| !chosen.contains(i))
                .expect("remaining row");
            for (i, &weight) in nearest.iter().enumerate() {
                if weight > 0. {
                    index = i;
                    if target < weight {
                        break;
                    }
                    target -= weight;
                }
            }
            index
        };
        chosen.push(index);
        if mode != "sample" {
            let center = row(x, index);
            for (i, distance) in nearest.iter_mut().enumerate() {
                if i.is_multiple_of(256) {
                    e.check_cancel()?;
                }
                *distance = distance.min(cached.map_or_else(
                    || metric.pair(&row(x, i), &center),
                    |d| d[i + index * x.rows()],
                ));
            }
            nearest[index] = 0.;
        }
    }
    Ok((chosen.iter().map(|&r| row(x, r)).collect(), chosen))
}
fn assign(
    e: &Engine,
    x: &Matrix,
    centers: &[Vec<f64>],
    active: &[bool],
    metric: &Metric,
    labels: &mut [usize],
) -> Result<()> {
    if centers
        .iter()
        .zip(active)
        .any(|(center, &active)| active && !metric.pair(center, center).is_finite())
    {
        return Err("kmeans: distance is undefined for a starting or updated center".into());
    }
    for (i, label) in labels.iter_mut().enumerate() {
        if i.is_multiple_of(256) {
            e.check_cancel()?;
        }
        let point = row(x, i);
        let mut best = f64::INFINITY;
        *label = usize::MAX;
        for (j, c) in centers.iter().enumerate() {
            if !active[j] {
                continue;
            }
            let d = metric.pair(&point, c);
            if d < best {
                *label = j;
                best = d;
            }
        }
        if *label == usize::MAX {
            return Err("kmeans: no finite point-to-center distance".into());
        }
    }
    Ok(())
}
fn centroids(x: &Matrix, labels: &[usize], active: &[bool], metric: &str) -> Vec<Vec<f64>> {
    let mut centers = vec![vec![f64::NAN; x.cols()]; active.len()];
    for (j, center) in centers.iter_mut().enumerate() {
        if !active[j] {
            continue;
        }
        let members: Vec<_> = (0..x.rows()).filter(|&i| labels[i] == j).collect();
        if members.is_empty() {
            continue;
        }
        for (c, coordinate) in center.iter_mut().enumerate() {
            if metric == "cityblock" {
                let mut vals: Vec<_> = members
                    .iter()
                    .map(|&i| x.data()[i + c * x.rows()])
                    .collect();
                vals.sort_by(f64::total_cmp);
                *coordinate = (vals[(vals.len() - 1) / 2] + vals[vals.len() / 2]) / 2.;
            } else {
                *coordinate = members
                    .iter()
                    .map(|&i| x.data()[i + c * x.rows()])
                    .sum::<f64>()
                    / members.len() as f64;
            }
        }
    }
    centers
}
fn objective(x: &Matrix, centers: &[Vec<f64>], labels: &[usize], metric: &Metric) -> f64 {
    (0..x.rows())
        .map(|i| metric.pair(&row(x, i), &centers[labels[i]]))
        .sum()
}
fn repair_empty(
    name: &str,
    x: &Matrix,
    centers: &mut [Vec<f64>],
    labels: &mut [usize],
    active: &mut [bool],
    metric: &Metric,
    action: &str,
) -> Result<()> {
    for j in 0..centers.len() {
        if !active[j] || labels.contains(&j) {
            continue;
        }
        match action {
            "error" => return Err(format!("{name}: empty cluster encountered")),
            "drop" => active[j] = false,
            _ => {
                let mut best = None;
                let mut distance = f64::NEG_INFINITY;
                for i in 0..x.rows() {
                    if labels.iter().filter(|&&l| l == labels[i]).count() <= 1 {
                        continue;
                    }
                    let d = metric.pair(&row(x, i), &centers[labels[i]]);
                    if d > distance {
                        distance = d;
                        best = Some(i);
                    }
                }
                let i = best.ok_or("kmeans: cannot repair empty cluster")?;
                labels[i] = j;
                centers[j] = row(x, i);
            }
        }
    }
    Ok(())
}
fn pam_build(e: &Engine, d: &[f64], n: usize, k: usize) -> Result<Vec<usize>> {
    let mut medoids = Vec::new();
    let mut nearest = vec![f64::INFINITY; n];
    for _ in 0..k {
        let mut best = f64::INFINITY;
        let mut next = 0;
        for i in 0..n {
            e.check_cancel()?;
            if medoids.contains(&i) {
                continue;
            }
            let cost = (0..n).map(|r| nearest[r].min(d[r + i * n])).sum::<f64>();
            if cost < best {
                best = cost;
                next = i;
            }
        }
        medoids.push(next);
        for r in 0..n {
            nearest[r] = nearest[r].min(d[r + next * n]);
        }
    }
    Ok(medoids)
}

fn pam_swap(
    e: &Engine,
    d: &[f64],
    n: usize,
    medoids: &mut [usize],
    maxiter: usize,
) -> Result<(bool, usize)> {
    for iteration in 0..maxiter {
        e.check_cancel()?;
        let mut nearest = vec![f64::INFINITY; n];
        let mut second = nearest.clone();
        let mut owner = vec![0; n];
        for i in 0..n {
            for (slot, &medoid) in medoids.iter().enumerate() {
                let distance = d[i + medoid * n];
                if distance < nearest[i] {
                    second[i] = nearest[i];
                    nearest[i] = distance;
                    owner[i] = slot;
                } else if distance < second[i] {
                    second[i] = distance;
                }
            }
        }
        let cost = nearest.iter().sum::<f64>();
        let mut best = cost;
        let mut swap = None;
        for slot in 0..medoids.len() {
            for candidate in 0..n {
                e.check_cancel()?;
                if medoids.contains(&candidate) {
                    continue;
                }
                let mut trial = 0.;
                for i in 0..n {
                    if i.is_multiple_of(256) {
                        e.check_cancel()?;
                    }
                    let remaining = if owner[i] == slot {
                        second[i]
                    } else {
                        nearest[i]
                    };
                    trial += remaining.min(d[i + candidate * n]);
                }
                if trial < best - 8. * f64::EPSILON * cost.abs().max(1.) {
                    best = trial;
                    swap = Some((slot, candidate));
                }
            }
        }
        if let Some((slot, candidate)) = swap {
            medoids[slot] = candidate;
        } else {
            return Ok((true, iteration + 1));
        }
    }
    Ok((false, maxiter))
}

#[derive(Clone, Copy)]
struct Edge {
    distance: f64,
    a: usize,
    b: usize,
    sa: usize,
    sb: usize,
}
impl PartialEq for Edge {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Edge {}
impl PartialOrd for Edge {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Edge {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .distance
            .total_cmp(&self.distance)
            .then(other.a.cmp(&self.a))
            .then(other.b.cmp(&self.b))
    }
}
fn edge(distance: f64, sa: usize, sb: usize, ids: &[usize]) -> Edge {
    if ids[sa] < ids[sb] {
        Edge {
            distance,
            a: ids[sa],
            b: ids[sb],
            sa,
            sb,
        }
    } else {
        Edge {
            distance,
            a: ids[sb],
            b: ids[sa],
            sa: sb,
            sb: sa,
        }
    }
}
fn linkage(e: &mut Engine, a: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
    if !(1..=3).contains(&a.len()) {
        return Err(
            "linkage: expected X or condensed distances, optional method and metric".into(),
        );
    }
    let x = real(e, "linkage", &a[0])?;
    let method = if a.len() > 1 {
        e.text_value(a[1].clone())?.to_lowercase()
    } else {
        "single".into()
    };
    if ![
        "single", "complete", "average", "weighted", "centroid", "median", "ward",
    ]
    .contains(&method.as_str())
    {
        return Err("linkage: unsupported method".into());
    }
    // pdist produces a row; a column remains one-variable observations.
    let vector = x.rows() == 1;
    let observations = if vector {
        distance::vector_order(x.len())?
    } else {
        x.rows()
    };
    if observations < 2 {
        return Err("linkage: at least two observations required".into());
    }
    distance::condensed_len(e, "linkage", observations)?;
    // Bound the distance table and all heap entries before allocation.
    size(
        e,
        "linkage",
        observations,
        observations
            .checked_mul(10)
            .ok_or("linkage: workspace too large")?,
    )?;
    let distances = if vector {
        if a.len() == 3 {
            return Err("linkage: metric only applies to observation matrices".into());
        }
        distance::expand(e, "linkage", &x)?
    } else {
        let metric = Metric::new(e, "linkage", &x, a.get(2), None)?;
        if ["centroid", "median", "ward"].contains(&method.as_str()) && metric.name != "euclidean" {
            return Err("linkage: centroid, median and ward require Euclidean distance".into());
        }
        size(e, "linkage", x.rows(), x.rows())?;
        let mut d = vec![0.; x.rows() * x.rows()];
        for j in 0..x.rows() {
            e.check_cancel()?;
            let col = metric.batch(e, "linkage", &row(&x, j), &x, out)?;
            d[j * x.rows()..(j + 1) * x.rows()].copy_from_slice(&col);
        }
        Matrix::from_column_vec(x.rows(), x.rows(), d)?
    };
    let n = distances.rows();
    if distances.data().iter().any(|v| !v.is_finite() || *v < 0.) {
        return Err("linkage: distances must be finite and nonnegative".into());
    }
    if vector && ["centroid", "median", "ward"].contains(&method.as_str()) {
        euclidean_distances(e, &distances)?;
    }
    size(e, "linkage", n - 1, 3)?;
    let mut d = distances.data().to_vec();
    let mut ids: Vec<_> = (1..=n).collect();
    let mut counts = vec![1_f64; n];
    let mut active = vec![true; n];
    let mut heap = BinaryHeap::new();
    for i in 0..n {
        for j in i + 1..n {
            heap.push(edge(d[i + j * n], i, j, &ids));
        }
    }
    let mut z = vec![0.; (n - 1) * 3];
    for step in 0..n - 1 {
        e.check_cancel()?;
        let best = loop {
            let next = heap.pop().ok_or("linkage: no merge candidates")?;
            if active[next.sa]
                && active[next.sb]
                && ids[next.sa] == next.a
                && ids[next.sb] == next.b
            {
                break next;
            }
        };
        let (s, t) = (best.sa, best.sb);
        let (ns, nt) = (counts[s], counts[t]);
        z[step] = best.a as f64;
        z[step + n - 1] = best.b as f64;
        z[step + 2 * (n - 1)] = best.distance;
        active[t] = false;
        ids[s] = n + step + 1;
        counts[s] = ns + nt;
        for v in 0..n {
            if !active[v] || v == s {
                continue;
            }
            let (ds, dt) = (d[s + v * n], d[t + v * n]);
            let nv = counts[v];
            let dst = best.distance;
            let updated = match method.as_str() {
                "single" => ds.min(dt),
                "complete" => ds.max(dt),
                "average" => (ns * ds + nt * dt) / (ns + nt),
                "weighted" => (ds + dt) / 2.,
                "centroid" => ((ns * ds * ds + nt * dt * dt) / (ns + nt)
                    - ns * nt * dst * dst / (ns + nt).powi(2))
                .max(0.)
                .sqrt(),
                "median" => (0.5 * ds * ds + 0.5 * dt * dt - 0.25 * dst * dst)
                    .max(0.)
                    .sqrt(),
                _ => (((nv + ns) * ds * ds + (nv + nt) * dt * dt - nv * dst * dst)
                    / (nv + ns + nt))
                    .max(0.)
                    .sqrt(),
            };
            d[s + v * n] = updated;
            d[v + s * n] = updated;
            heap.push(edge(updated, s, v, &ids));
        }
    }
    Ok(vec![value(n - 1, 3, z)?])
}

// Schoenberg's Euclidean distance criterion: the double-centered squared
// distance matrix is positive semidefinite. Use the existing symmetric-eigen
// kernel; tolerate only floating-point noise in its smallest eigenvalues.
fn euclidean_distances(e: &Engine, d: &Matrix) -> Result<()> {
    let n = d.rows();
    let row_means: Vec<f64> = (0..n)
        .map(|i| (0..n).map(|j| d.data()[i + j * n].powi(2)).sum::<f64>() / n as f64)
        .collect();
    let mean = row_means.iter().sum::<f64>() / n as f64;
    let gram = DMatrix::from_fn(n, n, |i, j| {
        -0.5 * (d.data()[i + j * n].powi(2) - row_means[i] - row_means[j] + mean)
    });
    if gram.iter().any(|v| !v.is_finite()) {
        return Err("linkage: squared distances overflow".into());
    }
    let tolerance = 64. * f64::EPSILON * n as f64 * gram.norm().max(1.);
    e.check_cancel()?;
    let eigen = gram.symmetric_eigen();
    e.check_cancel()?;
    if eigen.eigenvalues.iter().any(|&v| v < -tolerance) {
        return Err("linkage: centroid, median and ward require Euclidean distances".into());
    }
    Ok(())
}

struct Tree {
    children: Vec<(usize, usize)>,
    heights: Vec<f64>,
    n: usize,
}
fn tree(e: &Engine, v: &Value) -> Result<Tree> {
    let z = real(e, "cluster/inconsistent", v)?;
    if z.cols() != 3 {
        return Err("cluster/inconsistent: Z must have three columns".into());
    }
    let n = z.rows() + 1;
    size(e, "cluster", n, 4)?;
    let mut used = vec![false; 2 * n - 1];
    let mut children = Vec::new();
    let mut heights = Vec::new();
    for i in 0..z.rows() {
        e.check_cancel()?;
        let mut pair = [0; 2];
        for (j, child) in pair.iter_mut().enumerate() {
            let v = z.data()[i + j * z.rows()];
            if !v.is_finite() || v < 1. || v.fract() != 0. || v >= (n + i + 1) as f64 {
                return Err("cluster/inconsistent: invalid cluster identifier".into());
            }
            let v = v as usize - 1;
            if used[v] {
                return Err("cluster/inconsistent: a node has multiple parents".into());
            }
            used[v] = true;
            *child = v;
        }
        let h = z.data()[i + 2 * z.rows()];
        if !h.is_finite() || h < 0. {
            return Err("cluster/inconsistent: heights must be finite and nonnegative".into());
        }
        children.push((pair[0], pair[1]));
        heights.push(h);
    }
    Ok(Tree {
        children,
        heights,
        n,
    })
}
fn inconsistency(e: &Engine, t: &Tree, depth: usize) -> Result<Vec<f64>> {
    let m = t.n - 1;
    let mut result = vec![0.; m * 4];
    for i in 0..m {
        e.check_cancel()?;
        let mut stack = vec![(t.n + i, 1)];
        let mut heights = Vec::new();
        while let Some((node, level)) = stack.pop() {
            if node < t.n {
                continue;
            }
            let j = node - t.n;
            heights.push(t.heights[j]);
            if level < depth {
                let (a, b) = t.children[j];
                stack.push((a, level + 1));
                stack.push((b, level + 1));
            }
        }
        let count = heights.len();
        let mean = heights.iter().sum::<f64>() / count as f64;
        let std = if count > 1 {
            (heights.iter().map(|h| (h - mean).powi(2)).sum::<f64>() / (count - 1) as f64).sqrt()
        } else {
            0.
        };
        result[i] = mean;
        result[i + m] = std;
        result[i + 2 * m] = count as f64;
        result[i + 3 * m] = if std > 0. {
            (t.heights[i] - mean) / std
        } else {
            0.
        };
    }
    Ok(result)
}
fn tree_call(e: &Engine, name: &str, a: &[Value]) -> Result<Vec<Value>> {
    if a.is_empty() {
        return Err(format!("{name}: expected linkage tree"));
    }
    let t = tree(e, &a[0])?;
    if name == "inconsistent" {
        if a.len() > 2 {
            return Err("inconsistent: expected Z and optional depth".into());
        }
        let depth = if a.len() == 2 {
            integer(e, name, &a[1])?
        } else {
            2
        };
        return Ok(vec![value(t.n - 1, 4, inconsistency(e, &t, depth)?)?]);
    }
    let mut maxclust = None;
    let mut cutoff = None;
    let mut criterion = "inconsistent".to_string();
    let mut criterion_set = false;
    let mut depth = 2;
    for (key, v) in options(name, &a[1..], &["maxclust", "cutoff", "criterion", "depth"])? {
        match key.as_str() {
            "maxclust" => maxclust = Some(integer(e, name, v)?),
            "cutoff" => cutoff = Some(scalar(e, name, v)?),
            "depth" => depth = integer(e, name, v)?,
            _ => {
                criterion_set = true;
                criterion = text(v)
                    .ok_or("cluster: Criterion must be text")?
                    .to_lowercase()
            }
        }
    }
    if maxclust.is_some() == cutoff.is_some()
        || !["distance", "inconsistent"].contains(&criterion.as_str())
    {
        return Err(
            "cluster: specify exactly one of MaxClust or Cutoff and a valid Criterion".into(),
        );
    }
    let m = t.n - 1;
    if maxclust.is_some() && !criterion_set {
        criterion = "distance".into();
    }
    let mut measure = if criterion == "distance" {
        t.heights.clone()
    } else {
        inconsistency(e, &t, depth)?[3 * m..].to_vec()
    };
    // A node can be cut only if every descendant also meets the criterion.
    for i in 0..m {
        let (a, b) = t.children[i];
        for child in [a, b] {
            if child >= t.n {
                measure[i] = measure[i].max(measure[child - t.n]);
            }
        }
    }
    let threshold = if let Some(k) = maxclust {
        if k >= t.n {
            f64::NEG_INFINITY
        } else {
            let mut levels = measure.clone();
            levels.sort_by(f64::total_cmp);
            levels[t.n - k - 1]
        }
    } else {
        cutoff.expect("cutoff supplied")
    };
    let mut labels = vec![0.; t.n];
    let mut next = 0.;
    let mut stack = vec![2 * t.n - 2];
    while let Some(node) = stack.pop() {
        e.check_cancel()?;
        if node < t.n
            || (if maxclust.is_some() {
                measure[node - t.n] <= threshold
            } else {
                measure[node - t.n] < threshold
            })
        {
            next += 1.;
            let mut leaves = vec![node];
            while let Some(leaf) = leaves.pop() {
                if leaf < t.n {
                    labels[leaf] = next;
                } else {
                    let (a, b) = t.children[leaf - t.n];
                    leaves.push(b);
                    leaves.push(a);
                }
            }
        } else {
            let (a, b) = t.children[node - t.n];
            stack.push(b);
            stack.push(a);
        }
    }
    Ok(vec![value(t.n, 1, labels)?])
}

#[derive(Clone, Debug)]
enum Group {
    Number(f64),
    Text(String),
}
impl Group {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Number(a), Self::Number(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
            (Self::Text(a), Self::Text(b)) => a.cmp(b),
            _ => Ordering::Equal,
        }
    }
    fn equal(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Number(a), Self::Number(b)) => a == b,
            (Self::Text(a), Self::Text(b)) => a == b,
            _ => false,
        }
    }
}
fn groups(e: &Engine, v: &Value) -> Result<(Vec<Option<Group>>, bool)> {
    match v {
        Value::Text(s) => Ok((
            vec![if s.trim_end().is_empty() {
                None
            } else {
                Some(Group::Text(s.trim_end().to_string()))
            }],
            true,
        )),
        Value::QuotedString(s) => Ok((
            vec![if s.is_empty() {
                None
            } else {
                Some(Group::Text(s.clone()))
            }],
            true,
        )),
        Value::Container(Array::Str(s)) => Ok((
            s.data()
                .iter()
                .map(|s| {
                    s.as_ref()
                        .filter(|s| !s.is_empty())
                        .map(|s| Group::Text(s.to_string()))
                })
                .collect(),
            true,
        )),
        Value::Container(Array::Character(c)) => {
            let rows = (0..c.rows()).map(|r| {
                String::from_utf16_lossy(
                    &(0..c.cols())
                        .map(|j| c.units()[r + j * c.rows()])
                        .collect::<Vec<_>>(),
                )
            });
            Ok((
                rows.map(|s| {
                    if s.trim_end().is_empty() {
                        None
                    } else {
                        Some(Group::Text(s.trim_end().into()))
                    }
                })
                .collect(),
                true,
            ))
        }
        Value::Container(Array::Cell(c)) => {
            let mut result = Vec::new();
            for i in 0..c.len() {
                let s = match c.get_linear(i)?.as_ref() {
                    Array::Character(c) => c.to_utf8(&e.cancel)?,
                    Array::Str(s) if s.len() == 1 => s.data()[0]
                        .as_ref()
                        .map_or(String::new(), |s| s.to_string()),
                    _ => return Err("confusionmat: cell groups must contain character rows".into()),
                };
                result.push(if s.is_empty() {
                    None
                } else {
                    Some(Group::Text(s))
                });
            }
            Ok((result, true))
        }
        _ => {
            let m = real(e, "confusionmat", v)?;
            if !m.is_empty() && m.rows() != 1 && m.cols() != 1 {
                return Err("confusionmat: numeric groups must be vectors".into());
            }
            size(e, "confusionmat", m.len(), 8)?;
            Ok((
                m.data()
                    .iter()
                    .map(|&v| {
                        if v.is_nan() {
                            None
                        } else {
                            Some(Group::Number(v))
                        }
                    })
                    .collect(),
                false,
            ))
        }
    }
}
fn confusion(e: &Engine, a: &[Value]) -> Result<Vec<Value>> {
    if a.len() < 2 {
        return Err("confusionmat: expected group and grouphat".into());
    }
    let (group, strings) = groups(e, &a[0])?;
    let (predicted, other) = groups(e, &a[1])?;
    if strings != other || group.len() != predicted.len() {
        return Err("confusionmat: groups must have matching types and lengths".into());
    }
    let mut order = None;
    for (_, v) in options("confusionmat", &a[2..], &["order"])? {
        let (items, kind) = groups(e, v)?;
        if kind != strings || items.iter().any(Option::is_none) {
            return Err("confusionmat: invalid Order".into());
        }
        order = Some(items.into_iter().flatten().collect::<Vec<_>>());
    }
    let explicit = order.is_some();
    let mut order =
        order.unwrap_or_else(|| group.iter().chain(&predicted).flatten().cloned().collect());
    if !explicit {
        if strings {
            let mut unique = Vec::new();
            for group in order {
                if !unique.iter().any(|g: &Group| g.equal(&group)) {
                    unique.push(group);
                }
            }
            order = unique;
        } else {
            order.sort_by(Group::cmp);
            order.dedup_by(|a, b| a.equal(b));
        }
    }
    for i in 0..order.len() {
        if i.is_multiple_of(256) {
            e.check_cancel()?;
        }
        if order[..i].iter().any(|v| v.equal(&order[i])) {
            return Err("confusionmat: Order contains duplicate groups".into());
        }
    }
    size(e, "confusionmat", order.len(), order.len())?;
    for (i, group) in group.iter().chain(&predicted).flatten().enumerate() {
        if i.is_multiple_of(256) {
            e.check_cancel()?;
        }
        if !order.iter().any(|v| v.equal(group)) {
            return Err("confusionmat: Order must contain every observed group".into());
        }
    }
    let n = order.len();
    let mut counts = vec![0.; n * n];
    for (i, (g, p)) in group.iter().zip(&predicted).enumerate() {
        if i.is_multiple_of(256) {
            e.check_cancel()?;
        }
        if let (Some(g), Some(p)) = (g, p) {
            let r = order.iter().position(|v| v.equal(g));
            let c = order.iter().position(|v| v.equal(p));
            if let (Some(r), Some(c)) = (r, c) {
                counts[r + c * n] += 1.;
            } else {
                return Err("confusionmat: Order must contain every observed group".into());
            }
        }
    }
    let order = if strings {
        let slots = order
            .into_iter()
            .map(|g| {
                if let Group::Text(s) = g {
                    Ok(Arc::new(Array::Character(crate::CharacterRow::from_text(
                        &s, &e.cancel,
                    )?)))
                } else {
                    unreachable!()
                }
            })
            .collect::<Result<Vec<_>>>()?;
        Value::Container(Array::Cell(crate::CellArray::from_shared(
            n, 1, slots, &e.cancel,
        )?))
    } else if matches!(&a[0], Value::Logical(_)) {
        Value::Logical(crate::LogicalMatrix::from_column_vec(
            n,
            1,
            order
                .iter()
                .map(|g| u8::from(matches!(g,Group::Number(v) if *v!=0.)))
                .collect(),
            &e.cancel,
        )?)
    } else {
        value(
            n,
            1,
            order
                .into_iter()
                .map(|g| {
                    if let Group::Number(v) = g {
                        v
                    } else {
                        unreachable!()
                    }
                })
                .collect(),
        )?
    };
    Ok(vec![value(n, n, counts)?, order])
}

/// Bottom-up coordinates with an iterative left-to-right traversal. Contract
/// the earliest n-P merges; label each contracted leaf in observation order.
fn dendrogram(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
    if args.is_empty() || args.len() > 2 {
        return Err("dendrogram: expected Z and optional leaf limit P; name/value options are not supported yet".into());
    }
    let t = tree(e, &args[0]).map_err(|s| s.replace("cluster/inconsistent", "dendrogram"))?;
    let p = match args.get(1) {
        None => 30,
        Some(v) => {
            let p = scalar(e, "dendrogram", v)?;
            if p < 0. || p.fract() != 0. || p > crate::matrix::MAX_ELEMENTS as f64 {
                return Err(
                    "dendrogram: P must be a bounded nonnegative integer (0 means all leaves)"
                        .into(),
                );
            }
            p as usize
        }
    };
    let leaves = if p == 0 { t.n } else { p.min(t.n) };
    if leaves.saturating_sub(1) > crate::plot::MAX_SERIES {
        return Err(format!(
            "dendrogram: plot supports at most {} leaves; use a smaller P",
            crate::plot::MAX_SERIES + 1
        ));
    }
    size(e, "dendrogram", t.n, 12)?;
    let cut = t.n - leaves;
    let mut minimum: Vec<usize> = (0..t.n).collect();
    for &(a, b) in &t.children {
        minimum.push(minimum[a].min(minimum[b]));
    }
    let mut stack = vec![2 * t.n - 2];
    let mut terminals = Vec::with_capacity(leaves);
    while let Some(node) = stack.pop() {
        e.check_cancel()?;
        if node < t.n + cut {
            terminals.push(node);
        } else {
            let (a, b) = t.children[node - t.n];
            stack.push(b);
            stack.push(a);
        }
    }
    let mut sorted = terminals.clone();
    sorted.sort_by_key(|&node| minimum[node]);
    let mut group = vec![0; 2 * t.n - 1];
    for (i, &node) in sorted.iter().enumerate() {
        group[node] = i + 1;
    }
    let mut membership = vec![0.; t.n];
    for &node in &terminals {
        stack.push(node);
        while let Some(child) = stack.pop() {
            e.check_cancel()?;
            if child < t.n {
                membership[child] = group[node] as f64;
            } else {
                let (a, b) = t.children[child - t.n];
                stack.push(b);
                stack.push(a);
            }
        }
    }
    let mut x = vec![0.; 2 * t.n - 1];
    let mut y = vec![0.; 2 * t.n - 1];
    let mut order = Vec::with_capacity(leaves);
    for (i, &node) in terminals.iter().enumerate() {
        x[node] = (i + 1) as f64;
        order.push(group[node] as f64);
    }
    let mut xs = Vec::with_capacity(4 * (leaves - 1));
    let mut ys = Vec::with_capacity(xs.capacity());
    for i in cut..t.n - 1 {
        e.check_cancel()?;
        let (a, b) = t.children[i];
        let h = t.heights[i];
        xs.extend([x[a], x[a], x[b], x[b]]);
        ys.extend([y[a], h, h, y[b]]);
        x[t.n + i] = (x[a] + x[b]) / 2.;
        y[t.n + i] = h;
    }
    let h = if leaves > 1 {
        e.call(
            "plot",
            vec![
                Value::Matrix(Matrix::from_column_vec(4, leaves - 1, xs)?),
                Value::Matrix(Matrix::from_column_vec(4, leaves - 1, ys)?),
            ],
            out,
        )?
    } else {
        // An empty plot still establishes the axes and honors hold.
        e.call("plot", vec![Value::Matrix(Matrix::filled(0, 0, 0.)?)], out)?;
        Value::Container(Array::Graphics(crate::GraphicsArray::from_shared(
            0,
            1,
            vec![],
            &e.cancel,
        )?))
    };
    if let Some(f) = e.figure.as_mut() {
        f.axes.extras.leaf_ticks = Some(Arc::new(order.iter().map(|v| format!("{v}")).collect()));
        f.axes.xlim = Some([0.5, leaves as f64 + 0.5]);
    }
    Ok(vec![
        h,
        value(t.n, 1, membership)?,
        value(1, leaves, order)?,
    ])
}
