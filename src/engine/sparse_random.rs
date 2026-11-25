//! Random sparse constructors.
//!
//! Stream consumption is intentional and stable: positions are drawn first,
//! sorted and deduplicated, then values are drawn in that sorted order.  Thus
//! a seeded engine gives the same sparse pattern and values independently of
//! allocation details.

use super::{Engine, Value, sparse_fns};
use crate::{
    Result,
    sparse::{Builder, MAX_NNZ, SparseMatrix},
};

pub(super) fn random(engine: &mut Engine, name: &str, args: &[Value]) -> Result<Value> {
    match name {
        "sprandsym" => symmetric(engine, args),
        "sprand" | "sprandn" => rectangular(engine, name, args),
        _ => unreachable!(),
    }
}

fn density(engine: &Engine, name: &str, value: &Value) -> Result<f64> {
    let d =
        scalar(engine, value).map_err(|_| format!("{name}: density must be a scalar in [0, 1]"))?;
    if !d.is_finite() || !(0. ..=1.).contains(&d) {
        return Err(format!("{name}: density must be in [0, 1]"));
    }
    Ok(d)
}

fn scalar(engine: &Engine, value: &Value) -> Result<f64> {
    if value.dims().iter().product::<usize>() != 1 || matches!(value, Value::Complex(_)) {
        return Err("not a real scalar".into());
    }
    value.clone().matrix(&engine.cancel)?.as_scalar()
}

fn count(engine: &Engine, name: &str, value: &Value) -> Result<usize> {
    let n = scalar(engine, value)?;
    if !n.is_finite() || n < 0. || n.fract() != 0. || n >= 2f64.powi(53) {
        return Err(format!("{name}: sizes must be nonnegative integers"));
    }
    Ok(n as usize)
}

fn target(name: &str, total: u64, density: f64) -> Result<usize> {
    let target = (density * total as f64).round();
    if !target.is_finite() || target > MAX_NNZ as f64 || target > usize::MAX as f64 {
        return Err(format!(
            "{name}: requested nonzeros exceed the sparse limit of {MAX_NNZ}"
        ));
    }
    Ok(target as usize)
}

fn rectangular(engine: &mut Engine, name: &str, args: &[Value]) -> Result<Value> {
    if args.len() == 1 {
        let s = sparse_fns::to_sparse(engine, name, &args[0])?;
        let mut b = Builder::new(s.rows(), s.cols(), false, s.nnz())?;
        for c in 0..s.cols() {
            engine.check_cancel()?;
            for k in s.column(c) {
                b.push(
                    s.rowidx()[k],
                    if name == "sprand" {
                        engine.random.next_f64()
                    } else {
                        engine.random.next_normal()
                    },
                )?;
            }
            b.end_column();
        }
        return Ok(sparse_fns::wrap(b.finish()));
    }
    if args.len() != 3 {
        return Err(format!("{name}: wrong number of input arguments"));
    }
    let rows = count(engine, name, &args[0])?;
    let cols = count(engine, name, &args[1])?;
    let d = density(engine, name, &args[2])?;
    let total = (rows as u64)
        .checked_mul(cols as u64)
        .ok_or_else(|| format!("{name}: dimensions overflow the addressable element count"))?;
    let n = target(name, total, d)?;
    let positions = positions(engine, total, n)?;
    let mut b = Builder::new(rows, cols, false, positions.len())?;
    let mut next = 0;
    for c in 0..cols {
        engine.check_cancel()?;
        while next < positions.len() && positions[next] / rows as u64 == c as u64 {
            let r = (positions[next] % rows as u64) as usize;
            let value = if name == "sprand" {
                engine.random.next_f64()
            } else {
                engine.random.next_normal()
            };
            b.push(r, value)?;
            next += 1;
        }
        b.end_column();
    }
    // `total == 0` has no positions, so the division above is never evaluated.
    Ok(sparse_fns::wrap(b.finish()))
}

fn positions(engine: &mut Engine, total: u64, count: usize) -> Result<Vec<u64>> {
    let mut result = Vec::new();
    result
        .try_reserve_exact(count)
        .map_err(|_| "sparse random: out of memory for positions")?;
    for i in 0..count {
        if i % 1024 == 0 {
            engine.check_cancel()?;
        }
        result.push((engine.random.next_f64() * total as f64).floor() as u64);
    }
    result.sort_unstable();
    result.dedup();
    Ok(result)
}

fn symmetric(engine: &mut Engine, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err("sprandsym: wrong number of input arguments".into());
    }
    let n = count(engine, "sprandsym", &args[0])?;
    let density = density(engine, "sprandsym", &args[1])?;
    let lower = (n as u64)
        .checked_mul(n as u64 + 1)
        .ok_or("sprandsym: dimensions overflow the addressable element count")?
        / 2;
    let wanted = target("sprandsym", lower, density)?;
    if n > 1 && wanted > (MAX_NNZ / 2) as usize {
        return Err(format!(
            "sprandsym: requested nonzeros exceed the sparse limit of {MAX_NNZ}"
        ));
    }
    let positions = positions(engine, lower, wanted)?;
    let mut entries = Vec::new();
    entries
        .try_reserve_exact(positions.len().saturating_mul(2))
        .map_err(|_| "sprandsym: out of memory for entries")?;
    for (i, &position) in positions.iter().enumerate() {
        if i % 1024 == 0 {
            engine.check_cancel()?;
        }
        let (row, col) = lower_coordinate(n, position);
        let value = engine.random.next_normal();
        entries.push((row, col, value));
        if row != col {
            entries.push((col, row, value));
        }
    }
    engine.check_cancel()?;
    Ok(sparse_fns::wrap(SparseMatrix::from_triplets(
        n,
        n,
        &entries,
        false,
        &engine.cancel,
    )?))
}

/// Packed lower triangle in column-major order to `(row, column)`.
fn lower_coordinate(n: usize, position: u64) -> (usize, usize) {
    let start = |c: usize| -> u64 { ((c as u128 * (2 * n as u128 - c as u128 + 1)) / 2) as u64 };
    let (mut lo, mut hi) = (0usize, n);
    while lo + 1 < hi {
        let mid = lo + (hi - lo) / 2;
        if start(mid) <= position {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + (position - start(lo)) as usize, lo)
}
