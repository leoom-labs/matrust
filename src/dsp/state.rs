use super::{Filter, MAX_WORK, check_cancel};
use crate::{Matrix, Result};
use rayon::prelude::*;
use std::sync::atomic::AtomicBool;

#[derive(Default)]
pub struct FilterOptions<'a> {
    pub initial: Option<&'a Matrix>,
    /// MATLAB dimensions 1 (columns) and 2 (rows). None selects the first
    /// nonsingleton dimension, including zero-length dimensions.
    pub dimension: Option<usize>,
    pub final_state: bool,
}

pub fn filter_with_state(
    b: &[f64],
    a: &[f64],
    x: &Matrix,
    initial: Option<&Matrix>,
    dimension: Option<usize>,
    cancel: &AtomicBool,
) -> Result<(Matrix, Matrix)> {
    let (y, zf) = filter_with_options(
        b,
        a,
        x,
        FilterOptions {
            initial,
            dimension,
            final_state: true,
        },
        cancel,
    )?;
    Ok((y, zf.expect("final state requested")))
}

/// Direct-form II transposed filtering with shared normalized coefficients.
/// Independent channels parallelize above the measured work threshold. When
/// the caller only needs y, no final-state matrix is allocated.
pub fn filter_with_options(
    b: &[f64],
    a: &[f64],
    x: &Matrix,
    options: FilterOptions<'_>,
    cancel: &AtomicBool,
) -> Result<(Matrix, Option<Matrix>)> {
    let prototype = Filter::new(b, a)?;
    let order = prototype.state.len();
    let dimension = options
        .dimension
        .unwrap_or(if x.rows() == 1 { 2 } else { 1 });
    if !(1..=2).contains(&dimension) {
        return Err(
            "filter dimension must be 1 or 2 for the current two-dimensional engine".into(),
        );
    }
    let (samples, channels) = if dimension == 1 {
        (x.rows(), x.cols())
    } else {
        (x.cols(), x.rows())
    };
    let work = x.len().saturating_mul(order + 1);
    if work > MAX_WORK {
        return Err("Filter operation exceeds work limit; process shorter chunks".into());
    }
    let initial = options.initial.filter(|z| !z.is_empty());
    let shared = initial.is_some_and(|z| z.len() == order && (z.rows() == 1 || z.cols() == 1));
    if let Some(z) = initial
        && !shared
        && (z.rows() != order || z.cols() != channels)
    {
        return Err(format!(
            "Initial filter state must be a vector of length {order}, or a {order}-by-{channels} matrix"
        ));
    }
    // Validate before allocating. Empty input with many channels can still have
    // a large final-state result, despite requiring no sample calculations.
    check_cancel(cancel)?;
    let mut final_state = if options.final_state {
        Some(vec![0.; Matrix::checked_len(order, channels)?])
    } else {
        None
    };
    let initialize = |filter: &mut Filter, channel: usize| {
        if let Some(z) = initial {
            let start = if shared { 0 } else { channel * order };
            filter
                .state
                .copy_from_slice(&z.data()[start..start + order]);
        } else {
            filter.reset();
        }
    };
    if samples == 0 || channels == 0 {
        if let Some(states) = &mut final_state
            && order > 0
        {
            let mut filter = prototype.clone();
            for (channel, state) in states.chunks_mut(order).enumerate() {
                if channel % 1024 == 0 {
                    check_cancel(cancel)?;
                }
                initialize(&mut filter, channel);
                state.copy_from_slice(filter.state());
            }
        }
        return Ok((
            x.clone(),
            final_state
                .map(|z| Matrix::from_column_vec(order, channels, z))
                .transpose()?,
        ));
    }
    if order == 0 {
        let gain = prototype.b[0];
        let output = x.map(|value| gain * value + 0.);
        check_cancel(cancel)?;
        return Ok((
            output,
            final_state
                .map(|z| Matrix::from_column_vec(0, channels, z))
                .transpose()?,
        ));
    }
    // Gather row channels contiguously once. This retains the same optimized
    // sample kernel and permits safe parallel output writes without locks.
    let transposed = dimension == 2 && x.rows() > 1 && x.cols() > 1;
    let working = if transposed { x.transpose() } else { x.clone() };
    let mut data = working.data().to_vec();
    drop(working);
    let process = |filter: &mut Filter,
                   channel: usize,
                   values: &mut [f64],
                   state: Option<&mut [f64]>|
     -> Result<()> {
        initialize(filter, channel);
        filter.process(values, cancel)?;
        if let Some(state) = state {
            state.copy_from_slice(filter.state());
        }
        Ok(())
    };
    if channels > 1 && work >= 1_000_000 {
        if let Some(states) = &mut final_state
            && order > 0
        {
            data.par_chunks_mut(samples)
                .zip(states.par_chunks_mut(order))
                .enumerate()
                .try_for_each_init(
                    || prototype.clone(),
                    |filter, (channel, (values, state))| {
                        process(filter, channel, values, Some(state))
                    },
                )?;
        } else {
            data.par_chunks_mut(samples).enumerate().try_for_each_init(
                || prototype.clone(),
                |filter, (channel, values)| process(filter, channel, values, None),
            )?;
        }
    } else {
        // Reuse the state allocation for serial channels.
        let mut filter = prototype.clone();
        for (channel, values) in data.chunks_mut(samples).enumerate() {
            initialize(&mut filter, channel);
            filter.process(values, cancel)?;
            if let Some(states) = &mut final_state {
                states[channel * order..(channel + 1) * order].copy_from_slice(filter.state());
            }
        }
    }
    let output = if transposed {
        Matrix::from_column_vec(x.cols(), x.rows(), data)?.transpose()
    } else {
        Matrix::from_column_vec(x.rows(), x.cols(), data)?
    };
    check_cancel(cancel)?;
    Ok((
        output,
        final_state
            .map(|z| Matrix::from_column_vec(order, channels, z))
            .transpose()?,
    ))
}
