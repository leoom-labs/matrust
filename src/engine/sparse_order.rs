//! Sparse graph orderings.  AMD and COLAMD use faer's symbolic kernels;
//! SYMRCM is kept here because its level-structure traversal is small and can
//! poll cancellation between graph passes.

use super::{Engine, Value, sparse_fns};
use crate::{Matrix, Result, sparse::SparseMatrix};
use faer::dyn_stack::{MemBuffer, MemStack};
use faer::sparse::{
    SymbolicSparseColMatRef,
    linalg::{amd, colamd},
};

pub(super) fn order(engine: &mut Engine, name: &str, args: &[Value]) -> Result<Value> {
    if !(args.len() == 1 || args.len() == 2) {
        return Err(format!("{name}: wrong number of input arguments"));
    }
    // MATLAB has tuning options.  Accepting but deliberately ignoring them
    // keeps this deterministic across faer versions.
    let s = sparse_fns::to_sparse(engine, name, &args[0])?;
    let permutation = match name {
        "amd" | "symamd" => {
            square(name, &s)?;
            amd_order(engine, name, &s)?
        }
        "colamd" => colamd_order(engine, name, &s)?,
        "symrcm" => {
            square(name, &s)?;
            symrcm(engine, &s)?
        }
        _ => unreachable!(),
    };
    Ok(Value::Matrix(Matrix::from_column_vec(
        1,
        permutation.len(),
        permutation.into_iter().map(|p| p as f64 + 1.).collect(),
    )?))
}

fn square(name: &str, s: &SparseMatrix) -> Result<()> {
    if s.rows() != s.cols() {
        Err(format!("{name}: input must be square"))
    } else {
        Ok(())
    }
}

fn symbolic(s: &SparseMatrix) -> SymbolicSparseColMatRef<'_, usize> {
    SymbolicSparseColMatRef::new_checked(s.rows(), s.cols(), s.colptr(), None, s.rowidx())
}

fn amd_order(engine: &mut Engine, name: &str, s: &SparseMatrix) -> Result<Vec<usize>> {
    engine.check_cancel()?;
    let mut perm = vec![0; s.cols()];
    let mut inverse = vec![0; s.cols()];
    let mut mem = MemBuffer::try_new(amd::order_scratch::<usize>(s.cols(), s.nnz()))
        .map_err(|_| format!("{name}: out of memory for sparse ordering workspace"))?;
    amd::order(
        &mut perm,
        &mut inverse,
        symbolic(s),
        Default::default(),
        MemStack::new(&mut mem),
    )
    .map_err(|error| format!("{name}: sparse ordering failed ({error:?})"))?;
    engine.check_cancel()?;
    Ok(perm)
}

fn colamd_order(engine: &mut Engine, name: &str, s: &SparseMatrix) -> Result<Vec<usize>> {
    engine.check_cancel()?;
    let mut perm = vec![0; s.cols()];
    let mut inverse = vec![0; s.cols()];
    let mut mem = MemBuffer::try_new(colamd::order_scratch::<usize>(s.rows(), s.cols(), s.nnz()))
        .map_err(|_| format!("{name}: out of memory for sparse ordering workspace"))?;
    colamd::order(
        &mut perm,
        &mut inverse,
        symbolic(s),
        Default::default(),
        MemStack::new(&mut mem),
    )
    .map_err(|error| format!("{name}: sparse ordering failed ({error:?})"))?;
    engine.check_cancel()?;
    Ok(perm)
}

/// Reverse Cuthill-McKee of the undirected pattern.  A graph edge is present
/// for either A(i,j) or A(j,i), so the traversal is exactly on `S + S'`.
fn symrcm(engine: &mut Engine, s: &SparseMatrix) -> Result<Vec<usize>> {
    let n = s.rows();
    let mut adjacent = vec![Vec::<usize>::new(); n];
    for c in 0..n {
        engine.check_cancel()?;
        for k in s.column(c) {
            let r = s.rowidx()[k];
            if r != c {
                adjacent[c].push(r);
                adjacent[r].push(c);
            }
        }
    }
    for neighbours in &mut adjacent {
        neighbours.sort_unstable();
        neighbours.dedup();
    }
    let degree: Vec<_> = adjacent.iter().map(Vec::len).collect();
    for neighbours in &mut adjacent {
        neighbours.sort_unstable_by_key(|&v| (degree[v], v));
    }

    let mut starts: Vec<usize> = (0..n).collect();
    starts.sort_unstable_by_key(|&v| (degree[v], v));
    let mut emitted = vec![false; n];
    let mut marks = vec![0usize; n];
    let mut stamp = 0usize;
    let mut result = Vec::with_capacity(n);
    for start in starts {
        if emitted[start] {
            continue;
        }
        let root = pseudo_peripheral(engine, start, &adjacent, &degree, &mut marks, &mut stamp)?;
        let levels = bfs_levels(engine, root, &adjacent, &mut marks, &mut stamp)?;
        let mut component = levels.into_iter().flatten().collect::<Vec<_>>();
        for &v in &component {
            emitted[v] = true;
        }
        component.reverse();
        result.extend(component);
    }
    Ok(result)
}

/// George-Liu pseudo-peripheral search, starting at a minimum-degree vertex.
fn pseudo_peripheral(
    engine: &mut Engine,
    root: usize,
    adjacent: &[Vec<usize>],
    degree: &[usize],
    marks: &mut [usize],
    stamp: &mut usize,
) -> Result<usize> {
    let mut levels = bfs_levels(engine, root, adjacent, marks, stamp)?;
    loop {
        let candidate = *levels
            .last()
            .unwrap()
            .iter()
            .min_by_key(|&&v| (degree[v], v))
            .unwrap();
        let next = bfs_levels(engine, candidate, adjacent, marks, stamp)?;
        if next.len() <= levels.len() {
            return Ok(candidate);
        }
        levels = next;
    }
}

fn bfs_levels(
    engine: &Engine,
    root: usize,
    adjacent: &[Vec<usize>],
    marks: &mut [usize],
    stamp: &mut usize,
) -> Result<Vec<Vec<usize>>> {
    *stamp = stamp
        .checked_add(1)
        .ok_or("symrcm: graph traversal stamp overflow")?;
    let mark = *stamp;
    let mut queue = vec![root];
    let mut head = 0;
    marks[root] = mark;
    let mut levels = Vec::new();
    while head < queue.len() {
        engine.check_cancel()?;
        let end = queue.len();
        let mut level = Vec::with_capacity(end - head);
        while head < end {
            let v = queue[head];
            head += 1;
            level.push(v);
            for &u in &adjacent[v] {
                if marks[u] != mark {
                    marks[u] = mark;
                    queue.push(u);
                }
            }
        }
        levels.push(level);
    }
    Ok(levels)
}
