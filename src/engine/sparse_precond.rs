//! Incomplete factorizations of sparse matrices: `ichol` and `ilu`.
//!
//! Algorithms (own code): Saad, Iterative Methods for Sparse Linear Systems,
//! 2nd ed., chapter 10.
//! - `ichol` 'nofill': IC(0), left-looking by columns on the pattern of
//!   `tril(A)`; 'ict': the same with fill, dropping entries below
//!   `droptol*norm(A(k:end,k),1)`. `michol` adds every dropped value to the
//!   two diagonals it couples, so `L*L'*e == A*e` for `e = ones`.
//! - `ilu` 'nofill': ILU(0), IKJ by rows (Saad algorithm 10.4); 'crout':
//!   ILUC (Saad section 10.4.5), U by rows and L by columns; 'ilutp': ILUT by
//!   columns with row pivoting (threshold `thresh`), `P*A = L*U`.
//!   `milu` 'row'/'col' add the dropped values of a row of U / a column of L
//!   to the pivot. 'nofill' with 'col' factors `A.'` by rows and transposes.
//!
//! Options and outputs follow the public documentation; values are checked
//! against GNU Octave 10.3 (tests/sparse_part2.rs). Differences: 'ilutp' with
//! `milu = 'row'` is not supported.
//!
//! Fill is counted while it grows ('ict', 'crout', 'ilutp'), never ahead.
use super::*;
use crate::sparse::{MAX_NNZ, SparseMatrix, SparseValues};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

const NONE: usize = usize::MAX;
/// Columns between cancellation polls.
const POLL: usize = 256;

/// Plain CSC (or CSR, read with rows and columns swapped) of doubles.
#[derive(Clone, Debug, Default)]
struct Csc {
    n: usize,
    ptr: Vec<usize>,
    idx: Vec<usize>,
    val: Vec<f64>,
}
impl Csc {
    fn from_sparse(s: &SparseMatrix) -> Self {
        Self {
            n: s.cols(),
            ptr: s.colptr().to_vec(),
            idx: s.rowidx().to_vec(),
            val: (0..s.nnz()).map(|k| s.value(k)).collect(),
        }
    }
    fn range(&self, c: usize) -> std::ops::Range<usize> {
        self.ptr[c]..self.ptr[c + 1]
    }
    /// Transpose (CSC <-> CSR); indices stay sorted.
    fn transpose(&self) -> Self {
        let n = self.n;
        let mut count = vec![0usize; n + 1];
        for &i in &self.idx {
            count[i + 1] += 1;
        }
        for i in 0..n {
            count[i + 1] += count[i];
        }
        let ptr = count.clone();
        let mut next = count;
        let mut idx = vec![0; self.idx.len()];
        let mut val = vec![0.; self.idx.len()];
        for c in 0..n {
            for p in self.range(c) {
                let q = next[self.idx[p]];
                next[self.idx[p]] += 1;
                idx[q] = c;
                val[q] = self.val[p];
            }
        }
        Self { n, ptr, idx, val }
    }
    /// Entries with `idx >= c` (lower part of a CSC, or upper of a CSR).
    fn lower(&self) -> Self {
        self.keep(|i, c| i >= c)
    }
    fn keep(&self, f: impl Fn(usize, usize) -> bool) -> Self {
        let mut out = Self {
            n: self.n,
            ptr: vec![0],
            ..Self::default()
        };
        for c in 0..self.n {
            for p in self.range(c) {
                if f(self.idx[p], c) {
                    out.idx.push(self.idx[p]);
                    out.val.push(self.val[p]);
                }
            }
            out.ptr.push(out.idx.len());
        }
        out
    }
    fn into_sparse(self) -> Result<SparseMatrix> {
        let n = self.n;
        SparseMatrix::from_parts(n, n, self.ptr, self.idx, SparseValues::Double(self.val))
    }
}

/// Column builder for factors that grow (rows sorted at the end of each column).
struct Grow {
    m: Csc,
}
impl Grow {
    fn new(n: usize) -> Self {
        Self {
            m: Csc {
                n,
                ptr: vec![0],
                ..Csc::default()
            },
        }
    }
    fn push(&mut self, name: &str, i: usize, v: f64) -> Result<()> {
        if self.m.idx.len() as u64 >= MAX_NNZ {
            return Err(format!(
                "{name}: the factor needs more than {MAX_NNZ} nonzeros"
            ));
        }
        self.m.idx.push(i);
        self.m.val.push(v);
        Ok(())
    }
    /// Sort the current column by index and close it.
    fn end(&mut self) {
        let a = *self.m.ptr.last().unwrap();
        let mut pairs: Vec<(usize, f64)> = self.m.idx[a..]
            .iter()
            .copied()
            .zip(self.m.val[a..].iter().copied())
            .collect();
        if pairs.windows(2).any(|w| w[0].0 > w[1].0) {
            pairs.sort_unstable_by_key(|p| p.0);
            for (k, (i, v)) in pairs.into_iter().enumerate() {
                self.m.idx[a + k] = i;
                self.m.val[a + k] = v;
            }
        }
        self.m.ptr.push(self.m.idx.len());
    }
}

/// Columns linked by the index of their next unread entry: `head[r]` lists
/// the columns whose next entry sits at index `r`; `at[c]` is that entry.
struct Links {
    head: Vec<usize>,
    next: Vec<usize>,
    at: Vec<usize>,
}
impl Links {
    fn new(n: usize) -> Self {
        Self {
            head: vec![NONE; n],
            next: vec![NONE; n],
            at: vec![0; n],
        }
    }
    /// Take the list of columns whose next entry is at `r`.
    fn take(&mut self, r: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut c = std::mem::replace(&mut self.head[r], NONE);
        while c != NONE {
            out.push(c);
            c = self.next[c];
        }
        out
    }
    /// Point column `c` at entry `p` (of `idx`, ending at `end`).
    fn link(&mut self, c: usize, p: usize, end: usize, idx: &[usize]) {
        self.at[c] = p;
        if p < end {
            let r = idx[p];
            self.next[c] = self.head[r];
            self.head[r] = c;
        }
    }
}

fn pivot_error(name: &str) -> String {
    if name == "ichol" {
        "ichol: encountered a nonpositive pivot".into()
    } else {
        "ilu: encountered a pivot equal to 0".into()
    }
}

// ------------------------------------------------------------------ ichol

/// IC(0) of the lower triangle `l` (CSC, diagonal first in each column).
fn ichol0(l: &Csc, michol: bool, poll: &dyn Fn() -> Result<()>) -> Result<Csc> {
    let n = l.n;
    let mut v = l.val.clone();
    let mut pos = vec![NONE; n];
    let mut comp = vec![0.; n];
    let mut links = Links::new(n);
    for k in 0..n {
        if k % POLL == 0 {
            poll()?;
        }
        let (a, b) = (l.ptr[k], l.ptr[k + 1]);
        if a == b || l.idx[a] != k {
            return Err(pivot_error("ichol"));
        }
        for p in a..b {
            pos[l.idx[p]] = p;
        }
        v[a] += comp[k];
        for j in links.take(k) {
            let (pk, end) = (links.at[j], l.ptr[j + 1]);
            let lkj = v[pk];
            for p in pk..end {
                let i = l.idx[p];
                let u = v[p] * lkj;
                if pos[i] != NONE {
                    v[pos[i]] -= u;
                } else if michol {
                    v[a] -= u;
                    comp[i] -= u;
                }
            }
            links.link(j, pk + 1, end, &l.idx);
        }
        let d = v[a];
        if d.is_nan() || d <= 0. || d.is_infinite() {
            return Err(pivot_error("ichol"));
        }
        let s = d.sqrt();
        v[a] = s;
        for x in &mut v[a + 1..b] {
            *x /= s;
        }
        for p in a..b {
            pos[l.idx[p]] = NONE;
        }
        links.link(k, a + 1, b, &l.idx);
    }
    Ok(Csc {
        n,
        ptr: l.ptr.clone(),
        idx: l.idx.clone(),
        val: v,
    })
}

/// IC with threshold dropping of the lower triangle `l`.
fn icholt(l: &Csc, droptol: f64, michol: bool, poll: &dyn Fn() -> Result<()>) -> Result<Csc> {
    let n = l.n;
    let mut out = Grow::new(n);
    let mut w = vec![0.; n];
    let mut mark = vec![false; n];
    let mut rows: Vec<usize> = Vec::new();
    let mut comp = vec![0.; n];
    let mut links = Links::new(n);
    for k in 0..n {
        if k % POLL == 0 {
            poll()?;
        }
        let norm: f64 = l.val[l.range(k)].iter().map(|x| x.abs()).sum();
        mark[k] = true;
        rows.push(k);
        w[k] = comp[k];
        for p in l.range(k) {
            let i = l.idx[p];
            if !mark[i] {
                mark[i] = true;
                rows.push(i);
            }
            w[i] += l.val[p];
        }
        for j in links.take(k) {
            let (pk, end) = (links.at[j], out.m.ptr[j + 1]);
            let lkj = out.m.val[pk];
            for p in pk..end {
                let i = out.m.idx[p];
                if !mark[i] {
                    mark[i] = true;
                    rows.push(i);
                }
                w[i] -= out.m.val[p] * lkj;
            }
            links.link(j, pk + 1, end, &out.m.idx);
        }
        let tol = droptol * norm;
        let mut d = w[k];
        for &i in &rows {
            if i != k && w[i] != 0. && w[i].abs() < tol {
                if michol {
                    d += w[i];
                    comp[i] += w[i];
                }
                w[i] = 0.;
            }
        }
        if d.is_nan() || d <= 0. || d.is_infinite() {
            return Err(pivot_error("ichol"));
        }
        let s = d.sqrt();
        out.push("ichol", k, s)?;
        for &i in &rows {
            if i != k && w[i] != 0. {
                out.push("ichol", i, w[i] / s)?;
            }
        }
        for &i in &rows {
            w[i] = 0.;
            mark[i] = false;
        }
        rows.clear();
        out.end();
        let a = out.m.ptr[k];
        links.link(k, a + 1, out.m.ptr[k + 1], &out.m.idx);
    }
    Ok(out.m)
}

// -------------------------------------------------------------------- ilu

/// ILU(0) by rows of `r` (a CSR read as CSC); returns `L+U-I` in the same
/// layout. `modified`: dropped fill goes to the row's diagonal.
fn ilu0_rows(r: &Csc, modified: bool, poll: &dyn Fn() -> Result<()>) -> Result<Csc> {
    let n = r.n;
    let mut v = r.val.clone();
    let mut diag = vec![NONE; n];
    let mut pos = vec![NONE; n];
    for i in 0..n {
        if i % POLL == 0 {
            poll()?;
        }
        let range = r.range(i);
        for p in range.clone() {
            pos[r.idx[p]] = p;
            if r.idx[p] == i {
                diag[i] = p;
            }
        }
        let mut dropped = 0.;
        for p in range.clone() {
            let k = r.idx[p];
            if k >= i {
                break;
            }
            let ukk = v[diag[k]];
            v[p] /= ukk;
            let lik = v[p];
            for q in diag[k] + 1..r.ptr[k + 1] {
                let j = r.idx[q];
                let u = lik * v[q];
                if pos[j] != NONE {
                    v[pos[j]] -= u;
                } else if modified {
                    dropped += u;
                }
            }
        }
        if diag[i] == NONE {
            return Err(pivot_error("ilu"));
        }
        v[diag[i]] -= dropped;
        if v[diag[i]] == 0. || !v[diag[i]].is_finite() {
            return Err(pivot_error("ilu"));
        }
        for p in range {
            pos[r.idx[p]] = NONE;
        }
    }
    Ok(Csc {
        n,
        ptr: r.ptr.clone(),
        idx: r.idx.clone(),
        val: v,
    })
}

/// Unit lower L and upper U (CSC) from a CSR of `L+U-I`.
fn split_rows(m: &Csc) -> (Csc, Csc) {
    let l = with_unit_diagonal_rows(&m.keep(|j, i| j < i));
    let u = m.keep(|j, i| j >= i);
    (l.transpose(), u.transpose())
}

/// Append the unit diagonal to each row of a strictly lower CSR.
fn with_unit_diagonal_rows(l: &Csc) -> Csc {
    let mut out = Csc {
        n: l.n,
        ptr: vec![0],
        ..Csc::default()
    };
    for i in 0..l.n {
        for p in l.range(i) {
            out.idx.push(l.idx[p]);
            out.val.push(l.val[p]);
        }
        out.idx.push(i);
        out.val.push(1.);
        out.ptr.push(out.idx.len());
    }
    out
}

/// ILU(0) of `a` (CSC); `milu` 'off' | 'row' | 'col'.
fn ilu0(a: &Csc, milu: &str, poll: &dyn Fn() -> Result<()>) -> Result<(Csc, Csc)> {
    if milu != "col" {
        let m = ilu0_rows(&a.transpose(), milu == "row", poll)?;
        return Ok(split_rows(&m));
    }
    // Column sums: factor A.' by rows (L2*U2 = A.'), then L = U2.'*D^-1, U = D*L2.'.
    let m = ilu0_rows(a, true, poll)?; // rows of A.' are the columns of A
    let n = a.n;
    let d: Vec<f64> = (0..n)
        .map(|c| m.range(c).find(|&p| m.idx[p] == c).map_or(0., |p| m.val[p]))
        .collect();
    // m stored by rows of A.' = columns of A: entry (c, r) of the A.' factor
    // sits in "row" c at index r, i.e. element (r, c) of the transposed factor.
    // Read as CSC of A, column c holds entry (r, c) = M2(c, r).
    // L(r, c), r > c: U2(c, r)/U2(c, c) = M2(c, r)/d[c] -> column c, row r > c.
    // U(r, c), r <= c: D(r)*L2(c, r) = d[r]*M2(c, r) for r < c, U(c, c) = d[c].
    let mut l = Csc {
        n,
        ptr: vec![0],
        ..Csc::default()
    };
    let mut u = l.clone();
    for c in 0..n {
        for p in m.range(c) {
            let r = m.idx[p];
            if r < c {
                u.idx.push(r);
                u.val.push(d[r] * m.val[p]);
            } else if r == c {
                u.idx.push(c);
                u.val.push(d[c]);
                l.idx.push(c);
                l.val.push(1.);
            } else {
                l.idx.push(r);
                l.val.push(m.val[p] / d[c]);
            }
        }
        l.ptr.push(l.idx.len());
        u.ptr.push(u.idx.len());
    }
    Ok((l, u))
}

fn column_norms2(a: &Csc) -> Vec<f64> {
    (0..a.n)
        .map(|c| {
            // Scaled by the largest magnitude: no overflow or underflow of the squares.
            let v = &a.val[a.range(c)];
            let big = v.iter().fold(0f64, |m, x| m.max(x.abs()));
            if big == 0. || !big.is_finite() {
                return big;
            }
            big * v.iter().map(|x| (x / big) * (x / big)).sum::<f64>().sqrt()
        })
        .collect()
}

/// Crout ILU (ILUC) with dropping; returns unit lower L and upper U (CSC).
fn iluc(a: &Csc, droptol: f64, milu: &str, poll: &dyn Fn() -> Result<()>) -> Result<(Csc, Csc)> {
    let n = a.n;
    let rows_of_a = a.transpose();
    let norms = column_norms2(a);
    // U by rows (CSR), L by columns (CSC, strictly lower).
    let mut u = Grow::new(n);
    let mut l = Grow::new(n);
    let mut ul = Links::new(n); // rows of U linked by the column of their next entry
    let mut ll = Links::new(n); // columns of L linked by the row of their next entry
    let (mut z, mut w) = (vec![0.; n], vec![0.; n]);
    let (mut zm, mut wm) = (vec![false; n], vec![false; n]);
    let (mut zl, mut wl): (Vec<usize>, Vec<usize>) = (Vec::new(), Vec::new());
    let mut comp = vec![0.; n];
    for k in 0..n {
        if k % POLL == 0 {
            poll()?;
        }
        zm[k] = true;
        zl.push(k);
        for p in rows_of_a.range(k) {
            let j = rows_of_a.idx[p];
            if j >= k {
                if !zm[j] {
                    zm[j] = true;
                    zl.push(j);
                }
                z[j] += rows_of_a.val[p];
            }
        }
        for p in a.range(k) {
            let i = a.idx[p];
            if i > k {
                wm[i] = true;
                wl.push(i);
                w[i] += a.val[p];
            }
        }
        let lrow = ll.take(k); // columns i < k with L(k, i) != 0
        let ucol = ul.take(k); // rows i < k with U(i, k) != 0
        for &i in &lrow {
            let lki = l.m.val[ll.at[i]];
            for q in ul.at[i]..u.m.ptr[i + 1] {
                let j = u.m.idx[q];
                if !zm[j] {
                    zm[j] = true;
                    zl.push(j);
                }
                z[j] -= lki * u.m.val[q];
            }
        }
        for &i in &ucol {
            let uik = u.m.val[ul.at[i]];
            let start = ll.at[i];
            for p in start..l.m.ptr[i + 1] {
                let r = l.m.idx[p];
                if r == k {
                    continue;
                }
                if !wm[r] {
                    wm[r] = true;
                    wl.push(r);
                }
                w[r] -= uik * l.m.val[p];
            }
        }
        // Advance the lists past index k.
        for &i in &lrow {
            let p = ll.at[i];
            ll.link(i, p + 1, l.m.ptr[i + 1], &l.m.idx);
        }
        for &i in &ucol {
            let q = ul.at[i];
            ul.link(i, q + 1, u.m.ptr[i + 1], &u.m.idx);
        }
        // Rows of U not touched at step k still point at a column > k; rows
        // whose first entry is beyond k are linked when created.
        // milu: a dropped value goes to the diagonal of its row ('row') or
        // column ('col'): now for this pivot, later through `comp`.
        let mut pivot = z[k] + comp[k];
        for &j in &zl {
            if j != k && z[j] != 0. && z[j].abs() < droptol * norms[j] {
                match milu {
                    "row" => pivot += z[j],
                    "col" => comp[j] += z[j],
                    _ => {}
                }
                z[j] = 0.;
            }
        }
        for &i in &wl {
            if w[i] != 0. && w[i].abs() < droptol * norms[k] {
                match milu {
                    "row" => comp[i] += w[i],
                    "col" => pivot += w[i],
                    _ => {}
                }
                w[i] = 0.;
            }
        }
        if pivot == 0. || !pivot.is_finite() {
            return Err(pivot_error("ilu"));
        }
        z[k] = pivot;
        for &j in &zl {
            if z[j] != 0. {
                u.push("ilu", j, z[j])?;
            }
            z[j] = 0.;
            zm[j] = false;
        }
        zl.clear();
        u.end();
        for &i in &wl {
            if w[i] != 0. {
                l.push("ilu", i, w[i] / pivot)?;
            }
            w[i] = 0.;
            wm[i] = false;
        }
        wl.clear();
        l.end();
        // Row k of U: next entry after the diagonal; column k of L: first entry.
        let (ua, ub) = (u.m.ptr[k], u.m.ptr[k + 1]);
        ul.link(k, ua + 1, ub, &u.m.idx);
        let (la, lb) = (l.m.ptr[k], l.m.ptr[k + 1]);
        ll.link(k, la, lb, &l.m.idx);
    }
    // L gets its unit diagonal; U rows become columns.
    let mut lo = Csc {
        n,
        ptr: vec![0],
        ..Csc::default()
    };
    for c in 0..n {
        lo.idx.push(c);
        lo.val.push(1.);
        for p in l.m.range(c) {
            lo.idx.push(l.m.idx[p]);
            lo.val.push(l.m.val[p]);
        }
        lo.ptr.push(lo.idx.len());
    }
    Ok((lo, u.m.transpose()))
}

/// ILUTP by columns with row pivoting. Returns (L, U, perm) with
/// `A(perm, :) ~= L*U`, L unit lower and U upper in pivot order.
fn ilutp(
    a: &Csc,
    droptol: f64,
    thresh: f64,
    milu: &str,
    udiag: bool,
    poll: &dyn Fn() -> Result<()>,
) -> Result<(Csc, Csc, Vec<usize>)> {
    let n = a.n;
    let norms = column_norms2(a);
    let mut step = vec![NONE; n]; // original row -> pivot step
    let mut perm = vec![NONE; n]; // pivot step -> original row
    let mut l = Grow::new(n); // strictly lower part, original row indices
    let mut u = Grow::new(n); // pivot-step row indices
    let mut w = vec![0.; n];
    let mut mark = vec![false; n];
    let mut queued = vec![false; n];
    let mut list: Vec<usize> = Vec::new();
    let mut heap: BinaryHeap<Reverse<usize>> = BinaryHeap::new();
    // Current row order with the swaps so far: position -> original row and back.
    let mut at: Vec<usize> = (0..n).collect();
    let mut where_: Vec<usize> = (0..n).collect();
    for j in 0..n {
        if j % POLL == 0 {
            poll()?;
        }
        let tol = droptol * norms[j];
        let mut dropped = 0.;
        for p in a.range(j) {
            let r = a.idx[p];
            mark[r] = true;
            list.push(r);
            w[r] = a.val[p];
            if step[r] != NONE {
                queued[step[r]] = true;
                heap.push(Reverse(step[r]));
            }
        }
        while let Some(Reverse(t)) = heap.pop() {
            queued[t] = false;
            let r = perm[t];
            let utj = w[r];
            w[r] = 0.;
            if utj == 0. {
                continue;
            }
            if utj.abs() < tol {
                dropped += utj;
                continue;
            }
            u.push("ilu", t, utj)?;
            for p in l.m.range(t) {
                let r2 = l.m.idx[p];
                if !mark[r2] {
                    mark[r2] = true;
                    list.push(r2);
                }
                w[r2] -= l.m.val[p] * utj;
                let s = step[r2];
                if s != NONE && !queued[s] {
                    queued[s] = true;
                    heap.push(Reverse(s));
                }
            }
        }
        // Pivot among the unpivoted rows.
        let mut best = NONE;
        let mut big = 0.;
        for &r in &list {
            if step[r] == NONE && w[r].abs() > big {
                big = w[r].abs();
                best = r;
            }
        }
        // The diagonal is the row now at position j (after earlier swaps); it
        // stays the pivot when |w(diag)| >= thresh*max (thresh 0: never pivot).
        let diag = at[j];
        if best == NONE || w[diag].abs() >= thresh * big {
            best = diag;
        }
        let mut pivot = w[best];
        if pivot == 0. {
            if !udiag || tol == 0. {
                return Err(pivot_error("ilu"));
            }
            pivot = tol;
        }
        for &r in &list {
            if r != best && step[r] == NONE && w[r] != 0. && w[r].abs() < tol {
                dropped += w[r];
                w[r] = 0.;
            }
        }
        if milu == "col" {
            pivot += dropped;
        }
        if pivot == 0. || !pivot.is_finite() {
            return Err(pivot_error("ilu"));
        }
        step[best] = j;
        perm[j] = best;
        // Swap the pivot row into position j.
        let (pb, rj) = (where_[best], at[j]);
        at.swap(j, pb);
        where_[best] = j;
        where_[rj] = pb;
        u.push("ilu", j, pivot)?;
        u.end();
        for &r in &list {
            if r != best && step[r] == NONE && w[r] != 0. {
                l.push("ilu", r, w[r] / pivot)?;
            }
            w[r] = 0.;
            mark[r] = false;
        }
        list.clear();
        l.end();
    }
    // L rows to pivot order, with the unit diagonal.
    let mut lo = Grow::new(n);
    for c in 0..n {
        lo.push("ilu", c, 1.)?;
        for p in l.m.range(c) {
            lo.push("ilu", step[l.m.idx[p]], l.m.val[p])?;
        }
        lo.end();
    }
    Ok((lo.m, u.m, perm))
}

// ---------------------------------------------------------------- wiring

fn option_text(engine: &Engine, name: &str, field: &str, v: &Value) -> Result<String> {
    let s = match v {
        Value::Text(s) | Value::QuotedString(s) => s.clone(),
        Value::Container(Array::Character(c)) => c.to_utf8(&engine.cancel)?,
        _ => {
            return Err(format!(
                "{name}: {} must be a character vector",
                field.to_uppercase()
            ));
        }
    };
    Ok(s.to_lowercase())
}
fn option_number(engine: &Engine, name: &str, field: &str, v: &Value) -> Result<f64> {
    let bad = || {
        format!(
            "{name}: {} must be a non-negative real scalar",
            field.to_uppercase()
        )
    };
    if matches!(v, Value::Complex(_)) || v.dims().iter().product::<usize>() != 1 {
        return Err(bad());
    }
    let x = match v {
        Value::Container(Array::Sparse(s)) => s.get(0, 0),
        _ => v
            .clone()
            .matrix(&engine.cancel)
            .map_err(|_| bad())?
            .as_scalar()?,
    };
    if x.is_nan() || x < 0. || x.is_infinite() {
        return Err(bad());
    }
    Ok(x)
}

/// Fields of the options structure (`opts`/`setup`), names case-insensitive.
fn options(name: &str, v: Option<&Value>) -> Result<Vec<(String, Value)>> {
    match v {
        None => Ok(Vec::new()),
        Some(Value::Container(Array::Struct(s))) if s.len() == 1 => s
            .field_names()
            .iter()
            .enumerate()
            .map(|(i, f)| Ok((f.to_lowercase(), Value::from(s.get(i, 0)?.as_ref().clone()))))
            .collect(),
        Some(_) => Err(format!(
            "{name}: {} must be a structure",
            if name == "ichol" { "OPTS" } else { "SETUP" }
        )),
    }
}

/// The sparse square input, or the documented error.
fn input(name: &str, args: &[Value]) -> Result<SparseMatrix> {
    if args.is_empty() || args.len() > 2 {
        return Err(format!("Invalid call to {name}"));
    }
    match &args[0] {
        Value::Container(Array::Sparse(s)) if s.rows() == s.cols() => Ok(s.to_double()),
        _ => Err(format!("{name}: A must be a sparse square matrix")),
    }
}

/// Is `name` one of this module's functions?
pub(super) fn handles(name: &str) -> bool {
    matches!(name, "ichol" | "ilu")
}

/// `ichol(A, opts)` / `ilu(A, setup)` with `count` outputs (0 counts as 1).
pub(super) fn call(
    engine: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
) -> Result<Vec<Value>> {
    let a = input(name, args)?;
    let wrap = |s: SparseMatrix| Value::Container(Array::Sparse(s));
    let max = if name == "ichol" { 1 } else { 3 };
    if count > max {
        return Err(format!("{name}: too many output arguments"));
    }
    let count = count.max(1);
    let opts = options(name, args.get(1))?;
    engine.sparse.seen = true;
    if a.rows() == 0 {
        return Ok(vec![wrap(a); count]);
    }
    let cancel = || engine.check_cancel();
    let poll: &dyn Fn() -> Result<()> = &cancel;
    if name == "ichol" {
        let (mut kind, mut droptol, mut michol, mut diagcomp, mut upper) =
            ("nofill".to_string(), 0., false, 0., false);
        for (f, v) in &opts {
            match f.as_str() {
                "type" => {
                    kind = option_text(engine, name, f, v)?;
                    if kind != "nofill" && kind != "ict" {
                        return Err("ichol: TYPE must be \"nofill\" or \"ict\"".into());
                    }
                }
                "droptol" => droptol = option_number(engine, name, f, v)?,
                "michol" => {
                    michol = match option_text(engine, name, f, v)?.as_str() {
                        "on" => true,
                        "off" => false,
                        _ => return Err("ichol: MICHOL must be \"on\" or \"off\"".into()),
                    }
                }
                "diagcomp" => diagcomp = option_number(engine, name, f, v)?,
                "shape" => {
                    upper = match option_text(engine, name, f, v)?.as_str() {
                        "upper" => true,
                        "lower" => false,
                        _ => return Err("ichol: SHAPE must be \"lower\" or \"upper\"".into()),
                    }
                }
                _ => return Err(format!("ichol: unknown option '{f}'")),
            }
        }
        let full = Csc::from_sparse(&a);
        let mut lower = if upper {
            full.keep(|i, c| i <= c).transpose()
        } else {
            full.lower()
        };
        if diagcomp > 0. {
            for c in 0..lower.n {
                for p in lower.range(c) {
                    if lower.idx[p] == c {
                        lower.val[p] *= 1. + diagcomp;
                    }
                }
            }
        }
        let l = if kind == "nofill" {
            ichol0(&lower, michol, poll)?
        } else {
            icholt(&lower, droptol, michol, poll)?
        };
        let l = if upper { l.transpose() } else { l };
        engine.charge(l.idx.len())?;
        return Ok(vec![wrap(l.into_sparse()?)]);
    }
    let (mut kind, mut droptol, mut milu, mut udiag, mut thresh) =
        ("nofill".to_string(), 0., "off".to_string(), false, 1.);
    for (f, v) in &opts {
        match f.as_str() {
            "type" => {
                kind = option_text(engine, name, f, v)?;
                if !["nofill", "crout", "ilutp"].contains(&kind.as_str()) {
                    return Err(
                        "ilu: TYPE must be one of \"nofill\", \"crout\", or \"ilutp\"".into(),
                    );
                }
            }
            "droptol" => droptol = option_number(engine, name, f, v)?,
            "milu" => {
                milu = option_text(engine, name, f, v)?;
                if !["off", "row", "col"].contains(&milu.as_str()) {
                    return Err("ilu: MILU must be one of \"off\", \"row\", or \"col\"".into());
                }
            }
            "udiag" => {
                let x = option_number(engine, name, f, v)
                    .map_err(|_| "ilu: UDIAG must be 0 or 1".to_string())?;
                if x != 0. && x != 1. {
                    return Err("ilu: UDIAG must be 0 or 1".into());
                }
                udiag = x == 1.;
            }
            "thresh" => {
                thresh = option_number(engine, name, f, v)
                    .map_err(|_| "ilu: THRESH must be a scalar in the range [0, 1]".to_string())?;
                if thresh > 1. {
                    return Err("ilu: THRESH must be a scalar in the range [0, 1]".into());
                }
            }
            _ => return Err(format!("ilu: unknown option '{f}'")),
        }
    }
    let m = Csc::from_sparse(&a);
    let n = m.n;
    let (l, u, perm) = match kind.as_str() {
        "nofill" => {
            let (l, u) = ilu0(&m, &milu, poll)?;
            (l, u, None)
        }
        "crout" => {
            let (l, u) = iluc(&m, droptol, &milu, poll)?;
            (l, u, None)
        }
        _ => {
            if milu == "row" {
                return Err("ilu: MILU 'row' with type 'ilutp' is not supported".into());
            }
            let (l, u, perm) = ilutp(&m, droptol, thresh, &milu, udiag, poll)?;
            (l, u, Some(perm))
        }
    };
    engine.charge(l.idx.len() + u.idx.len())?;
    // With fewer than three outputs, L carries the permutation: P.'*L.
    let l = match (&perm, count) {
        (Some(perm), 1 | 2) => {
            let mut rows = Grow::new(n);
            for c in 0..n {
                for p in l.range(c) {
                    rows.push("ilu", perm[l.idx[p]], l.val[p])?;
                }
                rows.end();
            }
            rows.m
        }
        _ => l,
    };
    if count == 1 {
        // W = L + U - speye(n).
        let mut wm = Csc {
            n,
            ptr: vec![0],
            ..Csc::default()
        };
        for c in 0..n {
            // Literal L + U - I: with ilutp, L is permuted and its ones are off the diagonal.
            let mut col: Vec<(usize, f64)> = vec![(c, -1.)];
            for p in l.range(c) {
                col.push((l.idx[p], l.val[p]));
            }
            for p in u.range(c) {
                col.push((u.idx[p], u.val[p]));
            }
            col.sort_unstable_by_key(|e| e.0);
            let mut k = 0;
            while k < col.len() {
                let (i, mut v) = col[k];
                while k + 1 < col.len() && col[k + 1].0 == i {
                    k += 1;
                    v += col[k].1;
                }
                if v != 0. {
                    wm.idx.push(i);
                    wm.val.push(v);
                }
                k += 1;
            }
            wm.ptr.push(wm.idx.len());
        }
        return Ok(vec![wrap(wm.into_sparse()?)]);
    }
    let mut outs = vec![wrap(l.into_sparse()?), wrap(u.into_sparse()?)];
    if count == 3 {
        let p = match perm {
            Some(perm) => {
                let entries: Vec<(usize, usize, f64)> =
                    perm.iter().enumerate().map(|(j, &r)| (j, r, 1.)).collect();
                SparseMatrix::from_triplets(n, n, &entries, false, &engine.cancel)?
            }
            None => SparseMatrix::identity(n, n)?,
        };
        outs.push(wrap(p));
    }
    Ok(outs)
}
