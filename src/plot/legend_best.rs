//! Deterministic bounded approximation; not MATLAB's private best algorithm.
use super::{Axes, AxisTransform, LegendLocation, LegendRect, LinePattern, PlotKind};
use crate::Result;
use std::sync::atomic::{AtomicBool, Ordering};

pub const LEGEND_PROBE_LIMIT: usize = 2048;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LegendBestView {
    /// Transformed [xmin, xmax, ymin, ymax], matching the renderer's actual view.
    pub bounds: [f64; 4],
    pub transforms: [AxisTransform; 2],
    pub frame: LegendRect,
    pub extent: [f64; 2],
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LegendBestResult {
    pub location: LegendLocation,
    pub probes: usize,
    pub scores: [f64; 8],
}
#[derive(Clone, Debug, Default)]
pub struct LegendBestCache {
    metadata: Option<Axes>,
    points: Vec<super::WeakSeriesPoints>,
    view: Option<LegendBestView>,
    result: Option<LegendBestResult>,
}
/// Worker-created validated immutable snapshot for bounded viewport-dependent scoring.
/// Construction validates original payload; resolve never scans it beyond the
/// existing 2048 original-index probes. Fields are private and cannot be forged.
#[derive(Clone, Debug)]
pub struct LegendBestPrepared {
    source: Axes,
}
impl LegendBestPrepared {
    pub fn new(axes: &Axes, cancel: &AtomicBool) -> Result<Self> {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        // Existing full validation is bounded but not internally preemptible.
        // Call this constructor on the worker, never a renderer callback.
        axes.validate()?;
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        Ok(Self {
            source: axes.clone(),
        })
    }
    pub fn matches(&self, axes: &Axes) -> bool {
        self.source.same_snapshot(axes)
    }
    pub fn resolve(
        &self,
        cache: &mut LegendBestCache,
        view: LegendBestView,
        freeze_gesture: bool,
    ) -> Result<LegendBestResult> {
        // Always score the validated owned snapshot. The view carries current
        // transformed bounds, including a renderer's temporary manual view.
        cache.resolve_validated(&self.source, view, freeze_gesture)
    }
}

impl LegendBestCache {
    pub fn resolve(
        &mut self,
        axes: &Axes,
        view: LegendBestView,
        freeze_gesture: bool,
    ) -> Result<LegendBestResult> {
        axes.validate()?;
        self.resolve_validated(axes, view, freeze_gesture)
    }
    fn resolve_validated(
        &mut self,
        axes: &Axes,
        view: LegendBestView,
        freeze_gesture: bool,
    ) -> Result<LegendBestResult> {
        let same = self
            .metadata
            .as_ref()
            .is_some_and(|old| old.same_metadata(axes))
            && self.points.len() == axes.series.len()
            && self
                .points
                .iter()
                .zip(&axes.series)
                .all(|(a, b)| a.matches(&b.points));
        if let Some(result) = self.result
            && (freeze_gesture || (same && self.view == Some(view)))
        {
            return Ok(result);
        }
        let result = best_validated(axes, view, None)?;
        if !same {
            let mut metadata = axes.clone();
            let empty = super::SeriesPoints::from(Vec::new());
            for s in &mut metadata.series {
                s.points = empty.clone();
            }
            self.metadata = Some(metadata);
            self.points = axes.series.iter().map(|s| s.points.downgrade()).collect();
        }
        self.view = Some(view);
        self.result = Some(result);
        Ok(result)
    }
}

fn fraction(x: f64, low: f64, high: f64) -> f64 {
    let span = high - low;
    let delta = x - low;
    if span.is_finite() && delta.is_finite() {
        (delta / span).clamp(-1e6, 1e6)
    } else {
        ((x * 0.5 - low * 0.5) / (high * 0.5 - low * 0.5)).clamp(-1e6, 1e6)
    }
}

/// Clipped line length: finite bounded screen coordinates make all products safe.
fn clipped_length(a: [f64; 2], b: [f64; 2], r: LegendRect) -> f64 {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let mut low: f64 = 0.;
    let mut high: f64 = 1.;
    for (p, q) in [
        (-dx, a[0] - r.left),
        (dx, r.right - a[0]),
        (-dy, a[1] - r.top),
        (dy, r.bottom - a[1]),
    ] {
        if p == 0. {
            if q < 0. {
                return 0.;
            }
        } else if p < 0. {
            low = low.max(q / p);
        } else {
            high = high.min(q / p);
        }
        if low > high {
            return 0.;
        }
    }
    dx.hypot(dy) * (high - low)
}

pub fn best_legend_location(
    axes: &Axes,
    view: LegendBestView,
    cancel: Option<&AtomicBool>,
) -> Result<LegendBestResult> {
    let stopped = || cancel.is_some_and(|c| c.load(Ordering::Relaxed));
    if stopped() {
        return Err("Cancelled".into());
    }
    axes.validate()?;
    best_validated(axes, view, cancel)
}
fn best_validated(
    axes: &Axes,
    view: LegendBestView,
    cancel: Option<&AtomicBool>,
) -> Result<LegendBestResult> {
    let stopped = || cancel.is_some_and(|c| c.load(Ordering::Relaxed));
    if stopped() {
        return Err("Cancelled".into());
    }
    if axes.series.len() > super::MAX_SERIES
        || !view.extent.iter().all(|v| v.is_finite() && *v > 0.)
        || !view.frame.valid()
        || view.frame.width() > 1e6
        || view.frame.height() > 1e6
        || view.frame.values().iter().any(|x| x.abs() > 1e9)
        || !view.bounds.iter().all(|x| x.is_finite())
        || view.bounds[0] >= view.bounds[1]
        || view.bounds[2] >= view.bounds[3]
    {
        return Err("Invalid best legend viewport".into());
    }
    let available = view.frame.inset([4.; 4]);
    let extent = [
        view.extent[0].min(available.width()),
        view.extent[1].min(available.height()),
    ];
    let area = extent[0] * extent[1];
    if !area.is_normal() || !extent[0].hypot(extent[1]).is_finite() {
        return Err("Best legend extent has no representable positive area".into());
    }
    let mut candidates = [available; 8];
    for (r, anchor) in candidates.iter_mut().zip(LegendLocation::INSIDE) {
        *r = super::legend_layout::anchored(available, extent, anchor)?;
    }
    let map = |p: [f64; 2]| -> Option<[f64; 2]> {
        if !view.transforms[0].accepts(p[0]) || !view.transforms[1].accepts(p[1]) {
            return None;
        }
        let x = fraction(
            view.transforms[0].forward(p[0]),
            view.bounds[0],
            view.bounds[1],
        );
        let y = fraction(
            view.transforms[1].forward(p[1]),
            view.bounds[2],
            view.bounds[3],
        );
        let out = [
            view.frame.left + view.frame.width() * x,
            view.frame.bottom - view.frame.height() * y,
        ];
        out.iter().all(|x| x.is_finite()).then_some(out)
    };
    let eligible = |s: &super::Series| {
        s.visible
            && !s.points.is_empty()
            && (matches!(
                s.kind,
                PlotKind::Scatter | PlotKind::Bar | PlotKind::Stem | PlotKind::Patch
            ) || s.pattern != LinePattern::None
                || s.markers)
    };
    let count = axes.series.iter().filter(|s| eligible(s)).count();
    let mut scores = [0.; 8];
    let mut probes = 0;
    let mut baselines = Vec::new();
    for (series_index, s) in axes.series.iter().filter(|s| eligible(s)).enumerate() {
        let quota =
            LEGEND_PROBE_LIMIT / count + usize::from(series_index < LEGEND_PROBE_LIMIT % count);
        let source_count = if s.kind == PlotKind::Patch {
            s.patch_face_count()
        } else {
            s.points.len()
        };
        let n = source_count.min(quota);
        for k in 0..n {
            if probes % 64 == 0 && stopped() {
                return Err("Cancelled".into());
            }
            probes += 1;
            // Probe the original adjacent segment. Never join distant strata.
            let index = if n <= 1 {
                0
            } else {
                k * (source_count - 1) / (n - 1)
            };
            if s.kind == PlotKind::Patch {
                let width = s.patch.unwrap().vertices_per_face();
                let start = index * width;
                let mut face = [[0.; 2]; 4];
                for (out, p) in face.iter_mut().zip(
                    s.points
                        .range(start..start + width)
                        .ok_or("Incomplete Patch face")?,
                ) {
                    *out = [
                        view.transforms[0].forward(p[0]),
                        view.transforms[1].forward(p[1]),
                    ];
                }
                let clipped = super::clip_patch_face(&face[..width], view.bounds);
                if clipped.points().len() < 3 {
                    continue;
                }
                let mut box_bounds = [
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ];
                for &p in clipped.points() {
                    let p = super::patch_screen_point(
                        p,
                        view.bounds,
                        [
                            view.frame.left,
                            view.frame.top,
                            view.frame.right,
                            view.frame.bottom,
                        ],
                    );
                    box_bounds[0] = box_bounds[0].min(p[0]);
                    box_bounds[1] = box_bounds[1].min(p[1]);
                    box_bounds[2] = box_bounds[2].max(p[0]);
                    box_bounds[3] = box_bounds[3].max(p[1]);
                }
                // Best placement is already a bounded sampling heuristic.
                // Filled-face bounds conservatively score triangular occupancy.
                let footprint = LegendRect::new(box_bounds);
                for (score, rect) in scores.iter_mut().zip(candidates) {
                    if let Some(hit) = footprint.intersection(rect) {
                        *score += hit.width() * hit.height() / (extent[0] * extent[1]);
                    }
                }
                continue;
            }
            let raw = s
                .points
                .get(index)
                .ok_or("Legend sample index out of range")?;
            let Some(point) = map(raw) else {
                continue;
            };
            if s.kind == PlotKind::Stem && axes.baseline_for_series(&s.identity).is_none() {
                let value = s.stem_baseline(view.transforms[1]);
                if !baselines.contains(&value) {
                    baselines.push(value);
                }
            }
            let marker = s.markers || s.kind == PlotKind::Scatter;
            if marker {
                let radius = f64::from(s.marker_radius()) + f64::from(s.stroke_width(1.)) * 0.5;
                let footprint = LegendRect::new([
                    point[0] - radius,
                    point[1] - radius,
                    point[0] + radius,
                    point[1] + radius,
                ]);
                if let Some(visible) = footprint.intersection(view.frame) {
                    for (score, rect) in scores.iter_mut().zip(candidates) {
                        if let Some(hit) = visible.intersection(rect) {
                            *score += (hit.width() * hit.height()) / (extent[0] * extent[1]);
                        }
                    }
                }
            }
            let baseline = if let Some(binding) = axes.baseline_for_series(&s.identity) {
                binding.value.resolve(view.transforms[1])?
            } else {
                s.stem_baseline(view.transforms[1])
            };
            if s.kind == PlotKind::Bar {
                let mut left = raw[0] + s.bar_offset - s.bar_width / 2.;
                let mut right = raw[0] + s.bar_offset + s.bar_width / 2.;
                if !view.transforms[0].accepts(left) {
                    left = view.transforms[0].inverse(view.bounds[0]);
                }
                if !view.transforms[0].accepts(right) {
                    right = view.transforms[0].inverse(view.bounds[1]);
                }
                if let (Some(a), Some(b)) = (map([left, baseline]), map([right, raw[1]])) {
                    let bar = LegendRect::new([
                        a[0].min(b[0]),
                        a[1].min(b[1]),
                        a[0].max(b[0]),
                        a[1].max(b[1]),
                    ]);
                    if let Some(visible) = bar.intersection(view.frame) {
                        for (score, rect) in scores.iter_mut().zip(candidates) {
                            if let Some(hit) = visible.intersection(rect) {
                                *score += (hit.width() * hit.height()) / (extent[0] * extent[1]);
                            }
                        }
                    }
                }
                continue;
            }
            if s.kind == PlotKind::Scatter || s.pattern == LinePattern::None {
                continue;
            }
            let mut segments = [None; 2];
            if s.kind == PlotKind::Stem {
                segments[0] = map([raw[0], baseline]).map(|a| (a, point));
            } else if let Some(next_raw) = s.points.get(index + 1)
                && let Some(next) = map(next_raw)
            {
                if s.kind == PlotKind::Stairs {
                    let corner = [next[0], point[1]];
                    segments = [Some((point, corner)), Some((corner, next))];
                } else {
                    segments[0] = Some((point, next));
                }
            }
            for (a, b) in segments.into_iter().flatten() {
                for (score, rect) in scores.iter_mut().zip(candidates) {
                    // Stroke footprint padding remains inside the data viewport.
                    let pad = f64::from(s.stroke_width(2.)) * 0.5;
                    if let Some(hit) = rect.expand([pad; 4]).intersection(view.frame) {
                        *score += clipped_length(a, b, hit) / extent[0].hypot(extent[1]);
                    }
                }
            }
        }
    }
    // Distinct visible stem baselines are drawn once, including marker-only stems.
    for value in baselines {
        let y = view.frame.bottom
            - view.frame.height()
                * fraction(
                    view.transforms[1].forward(value),
                    view.bounds[2],
                    view.bounds[3],
                );
        if y.is_finite() {
            for (score, rect) in scores.iter_mut().zip(candidates) {
                *score += clipped_length([view.frame.left, y], [view.frame.right, y], rect)
                    / extent[0].hypot(extent[1]);
            }
        }
    }
    // The same bounded implicit-group enumeration used by native/SVG rendering.
    for index in axes.implicit_bar_baselines() {
        if stopped() {
            return Err("Cancelled".into());
        }
        let value = axes.render_baseline(index, view.transforms[1])?;
        let y = view.frame.bottom
            - view.frame.height()
                * fraction(
                    view.transforms[1].forward(value),
                    view.bounds[2],
                    view.bounds[3],
                );
        let pad = super::BaselineStyle::legacy_stem().width_points * 2. / 3.;
        if y.is_finite() {
            for (score, rect) in scores.iter_mut().zip(candidates) {
                if let Some(hit) = rect.expand([pad; 4]).intersection(view.frame) {
                    *score += clipped_length([view.frame.left, y], [view.frame.right, y], hit)
                        / extent[0].hypot(extent[1]);
                }
            }
        }
    }
    // Explicit strokes are bounded independent objects, with their own footprint.
    for baseline in &axes.baselines {
        if stopped() {
            return Err("Cancelled".into());
        }
        if !baseline.style.visible || baseline.style.pattern == LinePattern::None {
            continue;
        }
        let value = baseline.value.resolve(view.transforms[1])?;
        let y = view.frame.bottom
            - view.frame.height()
                * fraction(
                    view.transforms[1].forward(value),
                    view.bounds[2],
                    view.bounds[3],
                );
        let pad = baseline.style.width_points * 2. / 3.;
        if y.is_finite() {
            for (score, rect) in scores.iter_mut().zip(candidates) {
                if let Some(hit) = rect.expand([pad; 4]).intersection(view.frame) {
                    *score += clipped_length([view.frame.left, y], [view.frame.right, y], hit)
                        / extent[0].hypot(extent[1]);
                }
            }
        }
    }
    if stopped() {
        return Err("Cancelled".into());
    }
    let winner = (0..8)
        .min_by(|&a, &b| scores[a].total_cmp(&scores[b]).then(a.cmp(&b)))
        .unwrap();
    Ok(LegendBestResult {
        location: LegendLocation::INSIDE[winner],
        probes,
        scores,
    })
}

#[cfg(test)]
mod prepared_tests {
    use super::*;
    #[test]
    fn prepared_best_uses_validated_shared_snapshot_with_exact_original_probe_results() {
        let mut engine = crate::Engine::default();
        engine
            .execute("x=1:8193;semilogy(x,x);legend('Signal','Location','best');")
            .unwrap();
        let axes = &engine.figure.as_ref().unwrap().axes;
        let cancel = AtomicBool::new(false);
        let prepared = LegendBestPrepared::new(axes, &cancel).unwrap();
        assert!(
            axes.series[0]
                .points
                .same_storage(&prepared.source.series[0].points)
        );
        let view = LegendBestView {
            bounds: [0., 8200., 0., 4.],
            transforms: axes.axis_transforms(),
            frame: LegendRect::new([0., 0., 800., 600.]),
            extent: [140., 40.],
        };
        let expected = best_legend_location(axes, view, None).unwrap();
        let mut cache = LegendBestCache::default();
        assert_eq!(prepared.resolve(&mut cache, view, false).unwrap(), expected);
        assert_eq!(prepared.resolve(&mut cache, view, false).unwrap(), expected);
        assert!(prepared.matches(axes));
        let mut changed = axes.clone();
        changed.series[0].visible = false;
        assert!(!prepared.matches(&changed));
        changed = axes.clone();
        changed.series[0].points = axes.series[0].points.materialize(&cancel).unwrap().into();
        assert!(!prepared.matches(&changed));
        let invalid_view = LegendBestView {
            bounds: [1., 1., 0., 4.],
            ..view
        };
        assert!(prepared.resolve(&mut cache, invalid_view, false).is_err());
        let mut invalid = axes.clone();
        invalid.xlim = Some([1., 1.]);
        assert!(LegendBestPrepared::new(&invalid, &cancel).is_err());
        assert!(
            LegendBestCache::default()
                .resolve(&invalid, view, false)
                .is_err()
        );
        cancel.store(true, Ordering::Relaxed);
        assert!(LegendBestPrepared::new(axes, &cancel).is_err());
        cancel.store(false, Ordering::Relaxed);
        assert!(LegendBestPrepared::new(axes, &cancel).is_ok());
    }
}
