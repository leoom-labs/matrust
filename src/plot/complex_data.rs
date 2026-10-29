//! Borrowed complex plotting coordinates. Only final real XY buffers are owned.
use super::{MAX_POINTS, MAX_SERIES, PlotKind, Series};
use crate::{ComplexMatrix, Matrix, Result};
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy)]
pub enum PlotData<'a> {
    Real(&'a Matrix),
    Complex(&'a ComplexMatrix),
}
impl PlotData<'_> {
    fn rows(self) -> usize {
        match self {
            Self::Real(m) => m.rows(),
            Self::Complex(m) => m.rows(),
        }
    }
    fn cols(self) -> usize {
        match self {
            Self::Real(m) => m.cols(),
            Self::Complex(m) => m.cols(),
        }
    }
    fn len(self) -> usize {
        self.rows() * self.cols()
    }
    fn vector(self) -> bool {
        self.rows() == 1 || self.cols() == 1
    }
    fn component(self, index: usize, imaginary: bool) -> f64 {
        match self {
            Self::Real(m) => m.data()[index],
            Self::Complex(m) => {
                let value = m.data()[index];
                if imaginary { value.im } else { value.re }
            }
        }
    }
}

fn check(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err("Calculation cancelled".into())
    } else {
        Ok(())
    }
}

/// MATLAB line-plot coordinates: one complex input uses real/imaginary parts;
/// explicit X/Y inputs use their real parts. No component matrices are copied.
/// Real-only calls retain their existing engine path. Parallel admission mirrors
/// that path (multiple series, >=100000 points), pending paired measurements.
pub fn series_from_plot_data(
    x: Option<PlotData<'_>>,
    y: PlotData<'_>,
    cancel: &AtomicBool,
) -> Result<Vec<Series>> {
    series_from_plot_data_with_budget(x, y, cancel, MAX_SERIES, MAX_POINTS)
}

pub(crate) fn series_from_plot_data_with_budget(
    x: Option<PlotData<'_>>,
    y: PlotData<'_>,
    cancel: &AtomicBool,
    remaining_series: usize,
    remaining_points: usize,
) -> Result<Vec<Series>> {
    check(cancel)?;
    if y.len() > MAX_POINTS {
        return Err("Plot exceeds 1000000 points".into());
    }
    let imaginary_y = x.is_none() && matches!(y, PlotData::Complex(_));
    let x = if imaginary_y { Some(y) } else { x };
    let mut transpose_y = false;
    let mut transpose_x = false;
    let (count, len) = if y.vector() {
        (1, y.len())
    } else {
        (y.cols(), y.rows())
    };
    let (count, len) = if let Some(x) = x {
        if x.vector() {
            if x.len() == len {
                (count, len)
            } else if !y.vector() && x.len() == y.cols() {
                transpose_y = true;
                (y.rows(), y.cols())
            } else {
                return Err("Plot vector length does not match the matrix rows or columns".into());
            }
        } else if y.vector() && y.len() == x.rows() {
            (x.cols(), x.rows())
        } else if y.vector() && y.len() == x.cols() {
            transpose_x = true;
            (x.rows(), x.cols())
        } else if x.rows() == y.rows() && x.cols() == y.cols() {
            (count, len)
        } else {
            return Err("Plot matrices must have equal sizes".into());
        }
    } else {
        (count, len)
    };
    if count.saturating_mul(len) > remaining_points.min(MAX_POINTS)
        || count > remaining_series.min(MAX_SERIES)
    {
        return Err("Plot exceeds 64 series or 1000000 points".into());
    }
    let build = |column: usize| -> Result<Series> {
        check(cancel)?;
        let mut points = Vec::new();
        points
            .try_reserve_exact(len)
            .map_err(|_| "Cannot allocate plot coordinates")?;
        for first in (0..len).step_by(4096) {
            check(cancel)?;
            for row in first..(first + 4096).min(len) {
                let a = x.map_or(row as f64 + 1., |m| {
                    let index = if m.vector() {
                        row
                    } else if transpose_x {
                        row * m.rows() + column
                    } else {
                        column * m.rows() + row
                    };
                    m.component(index, false)
                });
                let index = if y.vector() {
                    row
                } else if transpose_y {
                    row * y.rows() + column
                } else {
                    column * y.rows() + row
                };
                points.push([a, y.component(index, imaginary_y)]);
            }
            #[cfg(test)]
            cancellation_probe::completed(cancel);
            check(cancel)?;
        }
        Ok(Series::new(points, PlotKind::Line))
    };
    let result = if count > 1 && count * len >= 100_000 {
        (0..count).into_par_iter().map(build).collect()
    } else {
        (0..count).map(build).collect()
    };
    check(cancel)?;
    result
}

#[cfg(test)]
mod cancellation_probe {
    use std::{
        cell::Cell,
        sync::atomic::{AtomicBool, Ordering},
    };
    thread_local! { static ARMED: Cell<bool> = const { Cell::new(false) }; }
    pub(super) fn completed(cancel: &AtomicBool) {
        if ARMED.with(|a| a.replace(false)) {
            cancel.store(true, Ordering::Relaxed);
        }
    }
    #[test]
    fn cancellation_discards_partial_coordinate_buffer() {
        let cancel = AtomicBool::new(false);
        let z =
            crate::ComplexMatrix::filled(8193, 1, crate::Complex64::new(2., 3.), &cancel).unwrap();
        let original = z.data().as_ptr();
        ARMED.with(|a| a.set(true));
        let error =
            super::series_from_plot_data(None, super::PlotData::Complex(&z), &cancel).unwrap_err();
        assert!(error.contains("cancelled"));
        assert!(cancel.load(Ordering::Relaxed));
        assert_eq!(z.data().as_ptr(), original);
        assert!(z.data().iter().all(|v| *v == crate::Complex64::new(2., 3.)));
    }
    #[test]
    fn exhausted_aggregate_budget_rejects_before_sampling() {
        let cancel = AtomicBool::new(false);
        let z =
            crate::ComplexMatrix::filled(4097, 2, crate::Complex64::new(1., 2.), &cancel).unwrap();
        for (series, points) in [(1, 8194), (2, 8193), (0, 0)] {
            ARMED.with(|a| a.set(true));
            let error = super::series_from_plot_data_with_budget(
                None,
                super::PlotData::Complex(&z),
                &cancel,
                series,
                points,
            )
            .unwrap_err();
            assert!(error.contains("exceeds"));
            assert!(!cancel.load(Ordering::Relaxed));
            assert!(ARMED.with(|a| a.replace(false)));
        }
    }
}
