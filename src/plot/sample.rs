//! Exact original-sample selection using distances in the displayed axes.
use super::{Axes, AxisTransform, PlotKind};
use crate::Result;
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};

// Release probe on i7-14700: 1k linear samples regressed with scheduling;
// 64k samples improved for both linear and log axes. Keep smaller series serial.
const PARALLEL_AT: usize = 65536;
const CHUNK: usize = 8192;

#[derive(Clone, Copy, Debug)]
pub struct SampleQuery {
    pub transforms: [AxisTransform; 2],
    /// Bounds and pointer are in transformed display coordinates.
    pub bounds: [f64; 4],
    pub pointer: [f64; 2],
    pub pixels: [f64; 2],
}
impl SampleQuery {
    pub fn validate(self) -> Result<()> {
        for axis in 0..2 {
            let low = self.bounds[axis * 2];
            let high = self.bounds[axis * 2 + 1];
            if !low.is_finite()
                || !high.is_finite()
                || high <= low
                || !(high - low).is_finite()
                || !self.pointer[axis].is_finite()
                || !(low..=high).contains(&self.pointer[axis])
                || !self.pixels[axis].is_finite()
                || !(0.0..=65536.).contains(&self.pixels[axis])
                || self.pixels[axis] == 0.
            {
                return Err("Sample inspection requires a finite, nonempty visible view".into());
            }
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SampleHit {
    pub series: usize,
    /// Zero-based original sample index, before display reduction.
    pub index: usize,
    pub point: [f64; 2],
    pub distance_squared: f64,
}
fn better(a: Option<SampleHit>, b: Option<SampleHit>) -> Option<SampleHit> {
    match (a, b) {
        (Some(a), Some(b)) => Some(
            if a.distance_squared
                .total_cmp(&b.distance_squared)
                .then_with(|| a.series.cmp(&b.series))
                .then_with(|| a.index.cmp(&b.index))
                .is_le()
            {
                a
            } else {
                b
            },
        ),
        (a, b) => a.or(b),
    }
}
impl Axes {
    /// Search visible original samples inside this view. Ties select the first
    /// series/index deterministically. This does not retain or copy point data.
    pub fn nearest_sample(
        &self,
        query: SampleQuery,
        cancel: &AtomicBool,
    ) -> Result<Option<SampleHit>> {
        query.validate()?;
        self.validate()?;
        if query.transforms[0].scale != self.xscale || query.transforms[1].scale != self.yscale {
            return Err("Sample inspection transforms do not match the axes".into());
        }
        let check = || {
            if cancel.load(Ordering::Relaxed) {
                Err("Cancelled".to_owned())
            } else {
                Ok(())
            }
        };
        check()?;
        let span = [
            query.bounds[1] - query.bounds[0],
            query.bounds[3] - query.bounds[2],
        ];
        let mut best = None;
        for (series, data) in self.series.iter().enumerate().filter(|(_, s)| s.visible) {
            let offset = if data.kind == PlotKind::Bar {
                data.bar_offset
            } else {
                0.
            };
            let scan = |chunk: usize| {
                let start = chunk * CHUNK;
                let end = (start + CHUNK).min(data.points.len());
                let mut best = None;
                for index in start..end {
                    let point = data.points.get(index).expect("bounded sample index");
                    if index % 4096 == 0 && cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    if !data.vertex_face_accepts(index, query.transforms) {
                        continue;
                    }
                    let displayed = [
                        query.transforms[0].forward(point[0] + offset),
                        query.transforms[1].forward(point[1]),
                    ];
                    if !(query.bounds[0]..=query.bounds[1]).contains(&displayed[0])
                        || !(query.bounds[2]..=query.bounds[3]).contains(&displayed[1])
                    {
                        continue;
                    }
                    let dx = (displayed[0] - query.pointer[0]) / span[0] * query.pixels[0];
                    let dy = (displayed[1] - query.pointer[1]) / span[1] * query.pixels[1];
                    best = better(
                        best,
                        Some(SampleHit {
                            series,
                            index,
                            point,
                            distance_squared: dx * dx + dy * dy,
                        }),
                    );
                }
                best
            };
            let hit = if data.points.len() >= PARALLEL_AT && rayon::current_num_threads() > 1 {
                (0..data.points.len().div_ceil(CHUNK))
                    .into_par_iter()
                    .map(scan)
                    .reduce(|| None, better)
            } else {
                (0..data.points.len().div_ceil(CHUNK))
                    .map(scan)
                    .fold(None, better)
            };
            best = better(best, hit);
            check()?;
        }
        Ok(best)
    }
}
