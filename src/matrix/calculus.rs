//! Real 2-D discrete differences and trapezoidal integration.
//! One cumulative lane always retains its serial f64 arithmetic order.
use super::{MAX_ELEMENTS, Matrix};
use crate::Result;
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};

const CHUNK: usize = 8192;
// Provisional only: parent must measure this family before promoting a claim.
const PAR_CALCULUS: usize = 512 * 1024;
const MAX_DIFF_WORK: u128 = 16 * MAX_ELEMENTS as u128;

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
fn parallel(len: usize) -> bool {
    len >= PAR_CALCULUS && rayon::current_num_threads() > 1
}

fn difference_pass(
    input: &[f64],
    rows: usize,
    dim: usize,
    output: &mut [f64],
    cancel: &AtomicBool,
) -> Result<()> {
    let out_rows = if dim == 1 { rows - 1 } else { rows };
    let work = |(chunk, dst): (usize, &mut [f64])| -> Result<()> {
        check(cancel)?;
        let first = chunk * CHUNK;
        if dim == 2 {
            for (i, out) in dst.iter_mut().enumerate() {
                *out = input[first + i + rows] - input[first + i];
            }
        } else {
            let mut done = 0;
            while done < dst.len() {
                let index = first + done;
                let row = index % out_rows;
                let source = index / out_rows * rows + row;
                let count = (out_rows - row).min(dst.len() - done);
                for (i, out) in dst[done..done + count].iter_mut().enumerate() {
                    *out = input[source + i + 1] - input[source + i];
                }
                done += count;
            }
        }
        Ok(())
    };
    if parallel(output.len()) {
        output
            .par_chunks_mut(CHUNK)
            .enumerate()
            .try_for_each(work)?;
    } else {
        output.chunks_mut(CHUNK).enumerate().try_for_each(work)?;
    }
    check(cancel)
}

#[derive(Clone, Copy)]
enum Spacing<'a> {
    Uniform(f64),
    Coordinates(&'a [f64]),
}
impl Spacing<'_> {
    fn step(self, i: usize) -> f64 {
        match self {
            Self::Uniform(_) => 1., // Scalar spacing multiplies completed sums.
            Self::Coordinates(x) => x[i] - x[i - 1],
        }
    }
}
fn spacing(x: Option<&Matrix>, length: usize) -> Result<Spacing<'_>> {
    match x {
        None => Ok(Spacing::Uniform(1.)),
        Some(x) if x.len() == 1 => Ok(Spacing::Uniform(x.data()[0])),
        Some(x) if (x.rows() == 1 || x.cols() == 1) && x.len() == length => {
            Ok(Spacing::Coordinates(x.data()))
        }
        _ => Err("Coordinates must be a real row/column vector matching the integration dimension, or scalar spacing".into()),
    }
}
fn weighted_half_sum(a: f64, b: f64, delta: f64) -> f64 {
    let sum = a + b;
    // Halving an odd subnormal sum first can lose a representable interval.
    // For finite delta, move the exact factor 1/2 to delta instead. If that
    // halving underflows, the product with a subnormal sum also rounds to zero.
    if sum.is_subnormal() && delta.is_finite() {
        return sum * (delta * 0.5);
    }
    let half = if sum.is_infinite() && a.is_finite() && b.is_finite() {
        a * 0.5 + b * 0.5
    } else {
        sum * 0.5
    };
    half * delta
}
fn scale(output: &mut [f64], spacing: Spacing<'_>, cancel: &AtomicBool) -> Result<()> {
    let Spacing::Uniform(factor) = spacing else {
        return check(cancel);
    };
    if factor == 1. {
        return check(cancel);
    }
    let work = |dst: &mut [f64]| -> Result<()> {
        check(cancel)?;
        for value in dst {
            *value *= factor;
        }
        Ok(())
    };
    if parallel(output.len()) {
        output.par_chunks_mut(CHUNK).try_for_each(work)?;
    } else {
        output.chunks_mut(CHUNK).try_for_each(work)?;
    }
    check(cancel)
}

impl Matrix {
    /// Fixed-axis diff policy. Higher-dimensional empties are unsupported.
    pub fn difference(&self, order: usize, dim: usize, cancel: &AtomicBool) -> Result<Self> {
        check(cancel)?;
        if order == 0 || order > MAX_ELEMENTS {
            return Err(format!(
                "Difference order must be a positive integer <= {MAX_ELEMENTS}"
            ));
        }
        if !(1..=2).contains(&dim) {
            return Err("diff supports only dimensions 1 and 2 of real 2-D arrays".into());
        }
        let length = if dim == 1 { self.rows() } else { self.cols() };
        let lanes = if dim == 1 { self.cols() } else { self.rows() };
        let remaining = length.saturating_sub(order);
        let shape = if dim == 1 {
            (remaining, lanes)
        } else {
            (lanes, remaining)
        };
        // Empty results are constant-work even for a very large admitted order.
        if remaining == 0 || lanes == 0 {
            return Self::from_column_vec(shape.0, shape.1, Vec::new());
        }
        let n = order as u128;
        let work = lanes as u128 * (n * length as u128 - n * (n + 1) / 2);
        if work > MAX_DIFF_WORK {
            return Err(format!(
                "Repeated differences exceed the {MAX_DIFF_WORK}-subtraction work limit"
            ));
        }
        let mut rows = self.rows();
        let mut cols = self.cols();
        let first_len = (length - 1) * lanes;
        let mut a = vec![0.; first_len];
        difference_pass(self.data(), rows, dim, &mut a, cancel)?;
        if dim == 1 {
            rows -= 1;
        } else {
            cols -= 1;
        }
        if order > 1 {
            let mut b = vec![0.; (length - 2) * lanes];
            for _ in 1..order {
                check(cancel)?;
                let next_len = if dim == 1 {
                    (rows - 1) * cols
                } else {
                    rows * (cols - 1)
                };
                b.truncate(next_len);
                difference_pass(&a, rows, dim, &mut b, cancel)?;
                std::mem::swap(&mut a, &mut b);
                if dim == 1 {
                    rows -= 1;
                } else {
                    cols -= 1;
                }
            }
        }
        check(cancel)?;
        Self::from_column_vec(shape.0, shape.1, a)
    }

    /// Trapezoid sums along one axis. None selects the first nonsingleton axis.
    /// Two-input script ambiguity is handled by dispatch, not this kernel.
    pub fn trapezoid(
        &self,
        x: Option<&Matrix>,
        dim: Option<usize>,
        cumulative: bool,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        check(cancel)?;
        let axis = dim.unwrap_or_else(|| self.first_nonsingleton_dimension());
        dimension(axis)?;
        let (rows, cols) = (self.rows(), self.cols());
        let length = match axis {
            1 => rows,
            2 => cols,
            _ => 1,
        };
        let spacing = spacing(x, length)?;
        let shape = if cumulative || axis > 2 {
            (rows, cols)
        } else if rows == 0 && cols == 0 && dim.is_none() {
            (1, 1)
        } else if axis == 1 {
            (1, cols)
        } else {
            (rows, 1)
        };
        let mut output = vec![0.; Self::checked_len(shape.0, shape.1)?];
        if length > 1 && !self.is_empty() {
            if axis == 1 || rows == 1 {
                // Contiguous columns, or a row vector viewed as a single lane.
                let lanes = self.len() / length;
                let output_length = if cumulative { length } else { 1 };
                let lanes_per_chunk = CHUNK.div_ceil(length);
                let chunk_len = lanes_per_chunk * output_length;
                let input = self.data();
                let run = |(chunk, dst): (usize, &mut [f64])| -> Result<()> {
                    check(cancel)?;
                    let first_lane = chunk * lanes_per_chunk;
                    for (offset, out) in dst.chunks_mut(output_length).enumerate() {
                        let start = (first_lane + offset) * length;
                        let lane = &input[start..start + length];
                        let mut acc = 0.;
                        for i in 1..length {
                            if i.is_multiple_of(CHUNK) {
                                check(cancel)?;
                            }
                            let term = weighted_half_sum(lane[i - 1], lane[i], spacing.step(i));
                            // Copy the first interval, as cumsum does, preserving -0.
                            acc = if i == 1 { term } else { acc + term };
                            if cumulative {
                                out[i] = acc;
                            }
                        }
                        if !cumulative {
                            out[0] = acc;
                        }
                    }
                    Ok(())
                };
                if parallel(self.len()) && lanes > 1 {
                    output
                        .par_chunks_mut(chunk_len)
                        .enumerate()
                        .try_for_each(run)?;
                } else {
                    output.chunks_mut(chunk_len).enumerate().try_for_each(run)?;
                }
            } else if !cumulative {
                // Independent row tiles. Keep accumulators small and source loads contiguous.
                const TILE: usize = 256;
                let input = self.data();
                let run = |(tile, dst): (usize, &mut [f64])| -> Result<()> {
                    check(cancel)?;
                    let first = tile * TILE;
                    for col in 1..cols {
                        if col.is_multiple_of(32) {
                            check(cancel)?;
                        }
                        let delta = spacing.step(col);
                        let left = (col - 1) * rows + first;
                        let right = col * rows + first;
                        for (i, acc) in dst.iter_mut().enumerate() {
                            let term = weighted_half_sum(input[left + i], input[right + i], delta);
                            *acc = if col == 1 { term } else { *acc + term };
                        }
                    }
                    Ok(())
                };
                if parallel(self.len()) && rows > TILE {
                    output.par_chunks_mut(TILE).enumerate().try_for_each(run)?;
                } else {
                    output.chunks_mut(TILE).enumerate().try_for_each(run)?;
                }
            } else {
                // Preserve prefix order across columns. Rows alone are independent.
                let input = self.data();
                for col in 1..cols {
                    check(cancel)?;
                    let delta = spacing.step(col);
                    let start = col * rows;
                    let (prior, rest) = output.split_at_mut(start);
                    let prior = &prior[start - rows..];
                    let current = &mut rest[..rows];
                    let run = |(chunk, dst): (usize, &mut [f64])| -> Result<()> {
                        check(cancel)?;
                        let first = chunk * CHUNK;
                        for (i, out) in dst.iter_mut().enumerate() {
                            let row = first + i;
                            let term = weighted_half_sum(
                                input[start - rows + row],
                                input[start + row],
                                delta,
                            );
                            *out = if col == 1 { term } else { prior[row] + term };
                        }
                        Ok(())
                    };
                    // Require a large current column, not merely a large whole matrix.
                    if parallel(rows) {
                        current
                            .par_chunks_mut(CHUNK)
                            .enumerate()
                            .try_for_each(run)?;
                    } else {
                        current.chunks_mut(CHUNK).enumerate().try_for_each(run)?;
                    }
                }
            }
        }
        scale(&mut output, spacing, cancel)?;
        Self::from_column_vec(shape.0, shape.1, output)
    }
}
