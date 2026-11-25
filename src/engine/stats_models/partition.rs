//! RNG-stream partitions shared by cvpartition and classification/regression crossval.
use super::*;
use crate::engine::classifier_objects::Labels;
#[derive(Clone, Debug)]
pub(in crate::engine) struct Partition {
    pub n: usize,
    pub tests: Vec<Vec<usize>>,
    pub resub: bool,
    groups: Vec<usize>,
    kind: String,
    parameter: f64,
}
impl Partition {
    pub(in crate::engine) fn bytes(&self) -> usize {
        128 + 8 * (self.groups.len() + self.tests.iter().map(Vec::len).sum::<usize>())
    }
    pub(in crate::engine) fn property(&self, name: &str) -> Result<Value> {
        match name {
            "NumObservations" => Ok(Value::Matrix(Matrix::scalar(self.n as f64))),
            "NumTestSets" => Ok(Value::Matrix(Matrix::scalar(self.tests.len() as f64))),
            "TestSize" => value(
                1,
                self.tests.len(),
                self.tests.iter().map(|v| v.len() as f64).collect(),
            ),
            "TrainSize" => value(
                1,
                self.tests.len(),
                self.tests
                    .iter()
                    .map(|v| if self.resub { self.n } else { self.n - v.len() } as f64)
                    .collect(),
            ),
            "Type" => Ok(Value::Text(self.kind.clone())),
            _ => Err(format!("cvpartition: unknown property '{name}'")),
        }
    }
}
pub(in crate::engine) fn nd_value(e: &Engine, dims: Vec<usize>, data: Vec<f64>) -> Result<Value> {
    let rows = dims[0];
    let cols = dims[1..].iter().product();
    size(e, "model property", rows, cols)?;
    Ok(Value::Matrix(
        Matrix::from_column_vec(rows, cols, data)?.with_dims(&dims)?,
    ))
}
fn shuffle(e: &mut Engine, rows: &mut [usize]) -> Result<()> {
    for i in (1..rows.len()).rev() {
        if i.is_multiple_of(1024) {
            e.check_cancel()?;
        }
        let j = (e.random.next_f64() * (i + 1) as f64) as usize;
        rows.swap(i, j);
    }
    Ok(())
}
const OBSERVATIONS_ERROR: &str = "cvpartition: NumObservations must be between 1 and 20000";
/// The 20000-row rule, checked before any per-observation vector is allocated.
fn check_observations(n: usize) -> Result<()> {
    if n == 0 || n > 20000 {
        return Err(OBSERVATIONS_ERROR.into());
    }
    Ok(())
}
pub(in crate::engine) fn make(
    e: &mut Engine,
    n: usize,
    groups: Vec<usize>,
    kind: &str,
    parameter: f64,
) -> Result<Partition> {
    if groups.len() != n {
        return Err(OBSERVATIONS_ERROR.into());
    }
    check_observations(n)?;
    if !parameter.is_finite() {
        return Err("cvpartition: partition parameter must be finite".into());
    }
    let count = match kind {
        "kfold" => {
            if parameter < 2. || parameter.fract() != 0. || parameter > n as f64 {
                return Err(
                    "cvpartition: KFold must be an integer between 2 and NumObservations".into(),
                );
            }
            parameter as usize
        }
        "holdout" => {
            if parameter <= 0. || parameter >= 1. {
                return Err("cvpartition: Holdout must be strictly between zero and one".into());
            }
            1
        }
        "leaveout" => n,
        "resubstitution" => 1,
        _ => return Err("cvpartition: unsupported partition type".into()),
    };
    let mut tests = vec![Vec::new(); count];
    let resub = kind == "resubstitution";
    if resub {
        tests[0] = (0..n).collect();
    } else if kind == "leaveout" {
        for (i, t) in tests.iter_mut().enumerate() {
            t.push(i);
        }
    } else {
        let classes = groups.iter().copied().max().unwrap() + 1;
        if classes > n {
            return Err("cvpartition: invalid groups".into());
        }
        let mut rows = vec![Vec::new(); classes];
        for (i, &c) in groups.iter().enumerate() {
            rows[c].push(i);
        }
        let mut offset = 0;
        if kind == "holdout" {
            let target = (n as f64 * parameter).ceil() as usize;
            let mut allocations: Vec<_> = rows
                .iter()
                .map(|g| (g.len() as f64 * parameter).floor() as usize)
                .collect();
            let mut order: Vec<_> = (0..classes).collect();
            order.sort_by(|&a, &b| {
                let a = rows[a].len() as f64 * parameter;
                let b = rows[b].len() as f64 * parameter;
                (b - b.floor()).total_cmp(&(a - a.floor()))
            });
            let remaining = target - allocations.iter().sum::<usize>();
            for &c in order.iter().take(remaining) {
                allocations[c] += 1;
            }
            for (c, g) in rows.iter_mut().enumerate() {
                shuffle(e, g)?;
                tests[0].extend_from_slice(&g[..allocations[c]]);
            }
        } else {
            for g in &mut rows {
                shuffle(e, g)?;
                for &i in g.iter() {
                    tests[offset % count].push(i);
                    offset += 1;
                }
            }
        }
    }
    for t in &mut tests {
        t.sort_unstable();
    }
    Ok(Partition {
        n,
        tests,
        resub,
        groups,
        kind: kind.into(),
        parameter,
    })
}
pub(in crate::engine) fn operation(
    e: &mut Engine,
    name: &str,
    p: &Partition,
    a: &[Value],
) -> Result<Value> {
    if name == "repartition" {
        if !a.is_empty() {
            return Err("repartition: expected one partition".into());
        }
        let q = make(e, p.n, p.groups.clone(), &p.kind, p.parameter)?;
        return e.ml_partition_new(q);
    }
    let fold = if a.is_empty() {
        if p.tests.len() != 1 {
            return Err(format!("{name}: fold index is required"));
        }
        0
    } else if a.len() == 1 {
        integer(e, name, &a[0])? - 1
    } else {
        return Err(format!("{name}: expected partition and fold index"));
    };
    if fold >= p.tests.len() {
        return Err(format!("{name}: fold index exceeds NumTestSets"));
    }
    let mut mask = vec![u8::from(name == "training"); p.n];
    for &i in &p.tests[fold] {
        mask[i] = u8::from(name == "test" || p.resub);
    }
    Ok(Value::Logical(crate::LogicalMatrix::from_column_vec(
        p.n, 1, mask, &e.cancel,
    )?))
}
pub(in crate::engine) fn call(e: &mut Engine, name: &str, a: &[Value]) -> Result<Value> {
    if name != "cvpartition" {
        return Err(format!("{name}: expected a cvpartition object"));
    }
    if a.len() < 2 || a.len() > 3 {
        return Err("cvpartition: expected observations and partition type/value".into());
    }
    let kind = super::ml::string(e, &a[1], name)?;
    let parameter = if a.len() == 3 {
        scalar(e, name, &a[2])?
    } else {
        0.
    };
    if ["kfold", "holdout"].contains(&kind.as_str()) && a.len() != 3
        || ["leaveout", "resubstitution"].contains(&kind.as_str()) && a.len() != 2
    {
        return Err("cvpartition: incorrect number of inputs for partition type".into());
    }
    let (n, groups) = if a[0].dims().iter().product::<usize>() == 1 && !a[0].is_character() {
        let n = integer(e, name, &a[0])?;
        check_observations(n)?;
        (n, vec![0; n])
    } else {
        let labels: Labels = super::classifiers::labels(e, &a[0], name)?;
        if matches!(&labels,Labels::Numeric(v) if v.iter().any(|x|x.is_nan()))
            || matches!(&labels,Labels::Typed(v) if (0..v.len()).any(|i|v.real(i).is_nan()))
        {
            return Err("cvpartition: missing group labels are not supported".into());
        }
        (labels.len(), labels.class_indices().1)
    };
    let p = make(e, n, groups, &kind, parameter)?;
    e.ml_partition_new(p)
}

/// Parse cross-validation options once, preserving supplied partition stratification.
pub(in crate::engine) fn for_crossval(
    e: &mut Engine,
    n: usize,
    groups: Vec<usize>,
    args: &[Value],
) -> Result<std::sync::Arc<Partition>> {
    if let [key, v] = args
        && e.text_value(key.clone())?
            .eq_ignore_ascii_case("CVPartition")
    {
        e.ml_partition_value(v)
            .ok_or_else(|| "crossval: CVPartition must be a cvpartition object".into())
    } else {
        if !args.is_empty() && args.len() != 2 {
            return Err("crossval: give one of CVPartition, KFold, Holdout or Leaveout".into());
        }
        let (kind, parameter) = if let [key, v] = args {
            let key = e.text_value(key.clone())?.to_ascii_lowercase();
            let p = if key == "leaveout" {
                if !e.text_value(v.clone())?.eq_ignore_ascii_case("on") {
                    return Err("crossval: Leaveout must be 'on'".into());
                }
                0.
            } else {
                v.clone().matrix(&e.cancel)?.as_scalar()?
            };
            (key, p)
        } else {
            ("kfold".into(), 10.)
        };
        if args.is_empty() && n < 10 {
            return Err("crossval: number of folds cannot exceed NumObservations".into());
        }
        Ok(std::sync::Arc::new(make(e, n, groups, &kind, parameter)?))
    }
}
