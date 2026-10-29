//! Bounded real 2-D array transforms and cumulative arithmetic.
//! Cumulative lanes retain their sequential arithmetic order across worker counts.
use super::{MAX_ELEMENTS, Matrix};
use crate::Result;
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};

const CHUNK: usize = 8192;
// Release sweeps showed scheduling losses at 64K. Use a conservative 512K
// crossover; gains depend on shape/work. Raw samples are retained in research.
const PAR_TRANSFORM: usize = 512 * 1024;
const PAR_CUMULATIVE: usize = 512 * 1024;

fn check(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err("Calculation cancelled".into())
    } else {
        Ok(())
    }
}

fn dimension(dim: usize) -> Result<()> {
    if dim == 0 || dim > MAX_ELEMENTS {
        Err(format!(
            "Dimension must be a positive integer <= {MAX_ELEMENTS}"
        ))
    } else {
        Ok(())
    }
}

fn offset(k: i64) -> Result<()> {
    if k.unsigned_abs() > MAX_ELEMENTS as u64 {
        Err(format!("Diagonal offset magnitude exceeds {MAX_ELEMENTS}"))
    } else {
        Ok(())
    }
}

fn build(
    len: usize,
    cancel: &AtomicBool,
    fill: impl Fn(usize, &mut [f64]) -> Result<()> + Sync,
) -> Result<Vec<f64>> {
    check(cancel)?;
    let mut output = vec![0.; len];
    let work = |(chunk, dst): (usize, &mut [f64])| {
        check(cancel)?;
        fill(chunk * CHUNK, dst)
    };
    if len >= PAR_TRANSFORM && rayon::current_num_threads() > 1 {
        output
            .par_chunks_mut(CHUNK)
            .enumerate()
            .try_for_each(work)?;
    } else {
        output.chunks_mut(CHUNK).enumerate().try_for_each(work)?;
    }
    check(cancel)?;
    Ok(output)
}

fn scan(input: &[f64], out: &mut [f64], product: bool, cancel: &AtomicBool) -> Result<()> {
    let mut acc = input[0];
    out[0] = acc;
    for (input, out) in input[1..].chunks(CHUNK).zip(out[1..].chunks_mut(CHUNK)) {
        check(cancel)?;
        if product {
            for (&value, dst) in input.iter().zip(out) {
                acc *= value;
                *dst = acc;
            }
        } else {
            for (&value, dst) in input.iter().zip(out) {
                acc += value;
                *dst = acc;
            }
        }
    }
    Ok(())
}

impl Matrix {
    /// First nonsingleton dimension for a real two-dimensional array.
    pub fn first_nonsingleton_dimension(&self) -> usize {
        if self.rows() == 1 && self.cols() != 1 {
            2
        } else {
            1
        }
    }

    /// Reverse dimension 1 or 2. Higher bounded dimensions are singleton no-ops.
    pub fn flip_dimension(&self, dim: usize, cancel: &AtomicBool) -> Result<Self> {
        check(cancel)?;
        dimension(dim)?;
        let (rows, cols) = (self.rows(), self.cols());
        if self.is_empty() || dim > 2 || (dim == 1 && rows == 1) || (dim == 2 && cols == 1) {
            return Ok(self.clone());
        }
        let source = self.data();
        if rows == 1 || cols == 1 {
            let output = build(self.len(), cancel, |first, dst| {
                let end = source.len() - first;
                let count = dst.len();
                for (out, &value) in dst.iter_mut().zip(source[end - count..end].iter().rev()) {
                    *out = value;
                }
                Ok(())
            })?;
            return Self::from_column_vec(rows, cols, output);
        }
        let output = build(self.len(), cancel, |first, dst| {
            let mut done = 0;
            while done < dst.len() {
                let index = first + done;
                let (column, row) = (index / rows, index % rows);
                let count = (rows - row).min(dst.len() - done);
                let target = &mut dst[done..done + count];
                if dim == 1 {
                    let end = (column + 1) * rows - row;
                    for (out, &value) in
                        target.iter_mut().zip(source[end - count..end].iter().rev())
                    {
                        *out = value;
                    }
                } else {
                    let start = (cols - 1 - column) * rows + row;
                    target.copy_from_slice(&source[start..start + count]);
                }
                done += count;
            }
            Ok(())
        })?;
        Self::from_column_vec(rows, cols, output)
    }

    /// Retain a triangular portion, including the specified signed diagonal.
    pub fn triangular(&self, k: i64, upper: bool, cancel: &AtomicBool) -> Result<Self> {
        check(cancel)?;
        offset(k)?;
        let (rows, cols) = (self.rows(), self.cols());
        if self.is_empty() || (upper && k <= 1 - rows as i64) || (!upper && k >= cols as i64 - 1) {
            return Ok(self.clone());
        }
        let source = self.data();
        if rows == 1 || cols == 1 {
            // A vector's triangular portion is one contiguous interval. Avoid
            // scheduling or traversing zero-only chunks, especially when only
            // its first element survives. Allocation remains bounded.
            let len = self.len();
            let boundary = if rows == 1 {
                k + i64::from(!upper)
            } else {
                -k + i64::from(upper)
            };
            let cut = boundary.clamp(0, len as i64) as usize;
            // A scalar is both a row and column vector: use the same row-first
            // interpretation as the boundary calculation above.
            let keep_prefix = if rows == 1 { !upper } else { upper };
            let range = if keep_prefix { 0..cut } else { cut..len };
            let mut output = vec![0.; len];
            let work = |(chunk, dst): (usize, &mut [f64])| -> Result<()> {
                check(cancel)?;
                let start = range.start + chunk * CHUNK;
                dst.copy_from_slice(&source[start..start + dst.len()]);
                Ok(())
            };
            if range.len() >= PAR_TRANSFORM && rayon::current_num_threads() > 1 {
                output[range.clone()]
                    .par_chunks_mut(CHUNK)
                    .enumerate()
                    .try_for_each(work)?;
            } else {
                output[range.clone()]
                    .chunks_mut(CHUNK)
                    .enumerate()
                    .try_for_each(work)?;
            }
            check(cancel)?;
            return Self::from_column_vec(rows, cols, output);
        }
        let output = build(self.len(), cancel, |first, dst| {
            let mut done = 0;
            while done < dst.len() {
                let index = first + done;
                let (column, row) = (index / rows, index % rows);
                let count = (rows - row).min(dst.len() - done);
                // Bounds above make these signed differences safe even at extremes.
                let boundary = column as i64 - k + i64::from(upper);
                let cut = boundary.clamp(row as i64, (row + count) as i64) as usize - row;
                let range = if upper { 0..cut } else { cut..count };
                dst[done + range.start..done + range.end]
                    .copy_from_slice(&source[index + range.start..index + range.end]);
                done += count;
            }
            Ok(())
        })?;
        Self::from_column_vec(rows, cols, output)
    }

    /// Construct a diagonal matrix from a vector, or extract a matrix diagonal.
    /// The literal 0-by-0 empty input follows MATLAB's special empty result.
    pub fn diagonal(&self, k: i64, cancel: &AtomicBool) -> Result<Self> {
        check(cancel)?;
        offset(k)?;
        let shift = k.unsigned_abs() as usize;
        if self.rows() == 0 && self.cols() == 0 {
            if k != 0 {
                return Err("diag of a 0-by-0 input currently supports only offset zero".into());
            }
            return Ok(self.clone());
        }
        if self.rows() == 1 || self.cols() == 1 {
            let side = self
                .len()
                .checked_add(shift)
                .ok_or("Diagonal dimensions overflow")?;
            let len = Self::checked_len(side, side)?;
            let mut output = vec![0.; len];
            let start = if k >= 0 { shift * side } else { shift };
            for (i, &value) in self.data().iter().enumerate() {
                if i % CHUNK == 0 {
                    check(cancel)?;
                }
                output[start + i * (side + 1)] = value;
            }
            check(cancel)?;
            return Self::from_column_vec(side, side, output);
        }
        let (row, col) = if k >= 0 { (0, shift) } else { (shift, 0) };
        let count = self
            .rows()
            .saturating_sub(row)
            .min(self.cols().saturating_sub(col));
        let output = build(count, cancel, |first, dst| {
            for (i, out) in dst.iter_mut().enumerate() {
                let step = first + i;
                *out = self.data()[(col + step) * self.rows() + row + step];
            }
            Ok(())
        })?;
        Self::from_column_vec(count, 1, output)
    }

    /// Forward cumulative sum or product, including NaNs, along one dimension.
    /// Independent columns may run concurrently; one lane is never reassociated.
    pub fn cumulative(&self, dim: usize, product: bool, cancel: &AtomicBool) -> Result<Self> {
        check(cancel)?;
        dimension(dim)?;
        let (rows, cols) = (self.rows(), self.cols());
        if self.is_empty() || dim > 2 || (dim == 1 && rows == 1) || (dim == 2 && cols == 1) {
            return Ok(self.clone());
        }
        let source = self.data();
        let mut output = vec![0.; self.len()];
        let combine = |a, b| if product { a * b } else { a + b };
        if dim == 1 {
            // Group short columns so scheduling never creates one job per element.
            let columns_per_chunk = CHUNK.div_ceil(rows);
            let chunk_len = columns_per_chunk * rows;
            let work = |(chunk, dst): (usize, &mut [f64])| -> Result<()> {
                check(cancel)?;
                let start = chunk * chunk_len;
                for (column, out) in dst.chunks_mut(rows).enumerate() {
                    let input = &source[start + column * rows..start + (column + 1) * rows];
                    scan(input, out, product, cancel)?;
                }
                Ok(())
            };
            if self.len() >= PAR_CUMULATIVE && cols > 1 && rayon::current_num_threads() > 1 {
                output
                    .par_chunks_mut(chunk_len)
                    .enumerate()
                    .try_for_each(work)?;
            } else {
                output
                    .chunks_mut(chunk_len)
                    .enumerate()
                    .try_for_each(work)?;
            }
        } else if rows == 1 {
            scan(source, &mut output, product, cancel)?;
        } else {
            // Column-major contiguous row tiles: no strided copies or temporary transpose.
            for first in (0..rows).step_by(CHUNK) {
                check(cancel)?;
                let end = (first + CHUNK).min(rows);
                output[first..end].copy_from_slice(&source[first..end]);
            }
            for col in 1..cols {
                let start = col * rows;
                let (previous, rest) = output.split_at_mut(start);
                let previous = &previous[start - rows..];
                let current = &mut rest[..rows];
                for first in (0..rows).step_by(CHUNK) {
                    check(cancel)?;
                    let end = (first + CHUNK).min(rows);
                    for ((dst, &a), &b) in current[first..end]
                        .iter_mut()
                        .zip(&previous[first..end])
                        .zip(&source[start + first..start + end])
                    {
                        *dst = combine(a, b);
                    }
                }
            }
        }
        check(cancel)?;
        Self::from_column_vec(rows, cols, output)
    }
}
