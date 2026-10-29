//! Off-UI preparation shared with the accepted ordinary/subplot renderers.
use super::{figure_view, subplot_view};
use egui_plot::PlotPoint;
use matrust::{
    Result,
    plot::{AxisTransform, Figure},
};
use std::sync::atomic::{AtomicBool, Ordering};

// One admitted model contains <=1,030,000 expanded vertices across <=64 series. Full
// bars/patches plus bounded line/stair/stem geometry fit in this allowance.
// Retained caches share a two-maximum-cache byte budget; smaller caches can
// coexist within it. Inactive geometry may be evicted without changing models.
pub(super) const ONE_CACHE_BYTES: usize =
    (2 * matrust::plot::MAX_POINTS + 65536) * std::mem::size_of::<PlotPoint>();
pub(super) const CACHE_BYTES: usize = 2 * ONE_CACHE_BYTES;
pub struct Ordinary {
    pub points: Vec<Vec<PlotPoint>>,
    pub transforms: [AxisTransform; 2],
    pub bounds: ([f64; 2], [f64; 2]),
    pub legend: Option<Box<matrust::plot::LegendBestPrepared>>,
}
#[path = "figure_scene.rs"]
pub(super) mod scene;

pub enum Geometry {
    Ordinary(Ordinary),
    Subplots(subplot_view::Canvas),
}
impl Geometry {
    #[cfg(test)]
    pub fn retained_points(&self) -> usize {
        match self {
            Self::Ordinary(p) => p.points.iter().map(Vec::len).sum(),
            Self::Subplots(p) => p.retained_points(),
        }
    }
    pub fn prepare(id: u32, figure: &Figure, cancel: &AtomicBool) -> Result<Self> {
        check(cancel)?;
        // The worker validates/owns the admitted snapshot. Do not scan it again
        // through Figure::validate here; each preparation scan is cancellable.
        figure.geometry_ledger()?;
        let result = if figure.layout.is_some() {
            Self::Subplots(subplot_view::Canvas::prepare(id, figure, cancel)?)
        } else {
            let (transforms, bounds) = figure.display_axes_cancellable(cancel)?;
            let mut points = Vec::with_capacity(figure.series.len());
            for index in 0..figure.series.len() {
                check(cancel)?;
                points.push(convert(
                    figure.axes.display_series_points_cancellable(
                        index,
                        8192 / figure.series.len().max(1),
                        transforms,
                        cancel,
                    )?,
                    cancel,
                )?);
            }
            Self::Ordinary(Ordinary {
                points,
                transforms,
                bounds,
                legend: prepare_legend(&figure.axes, cancel)?.map(Box::new),
            })
        };
        check(cancel)?;
        // This is an internal invariant, not a smaller plot-admission policy.
        if result.retained_bytes() > ONE_CACHE_BYTES {
            return Err("Prepared display geometry exceeded its internal accounting bound".into());
        }
        Ok(result)
    }
    pub fn retained_bytes(&self) -> usize {
        match self {
            Self::Ordinary(p) => p
                .points
                .iter()
                .map(|v| v.capacity() * std::mem::size_of::<PlotPoint>())
                .sum(),
            Self::Subplots(p) => p.retained_point_bytes(),
        }
    }
}

#[cfg(test)]
#[path = "detached_preparation_benchmark.rs"]
mod benchmark;
impl Ordinary {
    pub fn view(&self) -> figure_view::PreparedView<'_> {
        figure_view::PreparedView {
            points: &self.points,
            axes: self.transforms,
            bounds: self.bounds,
            legend: self.legend.as_deref(),
        }
    }
}
fn check(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err("Figure display preparation cancelled".into())
    } else {
        Ok(())
    }
}

pub(super) fn convert(source: Vec<[f64; 2]>, cancel: &AtomicBool) -> Result<Vec<PlotPoint>> {
    let mut result = Vec::with_capacity(source.len());
    for chunk in source.chunks(4096) {
        check(cancel)?;
        result.extend(chunk.iter().map(|&[x, y]| PlotPoint::new(x, y)));
    }
    check(cancel)?;
    Ok(result)
}

pub(super) fn prepare_legend(
    axes: &matrust::plot::Axes,
    cancel: &AtomicBool,
) -> Result<Option<matrust::plot::LegendBestPrepared>> {
    if axes.legend
        && axes.legend_entries().next().is_some()
        && axes.legend_location == matrust::plot::LegendLocation::Best
    {
        matrust::plot::LegendBestPrepared::new(axes, cancel).map(Some)
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod best_preparation_tests {
    use super::*;
    fn has_token(figure: &Figure) -> bool {
        let Geometry::Ordinary(prepared) =
            Geometry::prepare(1, figure, &AtomicBool::new(false)).unwrap()
        else {
            panic!("Ordinary fixture")
        };
        prepared.legend.is_some()
    }
    #[test]
    fn hidden_or_empty_best_skips_token_but_visible_labeled_snapshot_gets_fresh_token() {
        let mut engine = matrust::Engine::default();
        engine.execute("plot([1 2],[3 4]);").unwrap();
        let mut figure = engine.figure.unwrap();
        figure.legend_location = matrust::plot::LegendLocation::Best;
        figure.legend_state = None;
        figure.series[0].label.clear();
        figure.legend = true;
        let shared = figure.series[0].points.clone();
        assert!(!has_token(&figure));
        figure.series[0].label = "Signal".into();
        figure.legend = false;
        assert!(!has_token(&figure));
        figure.legend = true;
        assert!(has_token(&figure));
        assert!(shared.same_storage(&figure.series[0].points));
    }
}

#[cfg(test)]
mod compact_storage_tests {
    use super::*;
    use eframe::egui;
    use matrust::plot::{PlotKind, Series};
    #[test]
    fn maximum_compact_and_dense_banks_prepare_all_faces_and_paint_without_model_expansion() {
        let mut engine = matrust::Engine::default();
        engine.execute("x=(1:10000).';y=2+sin(x);a=x-.25;b=x+.25;c=y*0;patch('XData',[a b b a].','YData',[c c y y].','FaceColor',[.2 .4 .6],'EdgeColor','none');").unwrap();
        let mut figure = engine.figure.unwrap();
        let stop = AtomicBool::new(false);
        assert!(figure.series[0].points.patch().is_some());
        let mut dense = figure.series[0].clone();
        // A new real Patch object gets its own identity; only payload/style is shared.
        dense.identity = Default::default();
        dense.points = dense.points.materialize(&stop).unwrap().into();
        let line = Series::new(
            (0..950_000)
                .map(|i| [i as f64 + 1., (i % 11) as f64])
                .collect(),
            PlotKind::Line,
        );
        figure.add(vec![dense, line]).unwrap();
        figure.validate().unwrap();
        assert_eq!(
            figure.geometry_ledger().unwrap().cost().source_units(),
            1_000_000
        );
        let original = figure.clone();
        stop.store(true, Ordering::Relaxed);
        assert!(Geometry::prepare(1, &figure, &stop).is_err());
        stop.store(false, Ordering::Relaxed);
        let Geometry::Ordinary(prepared) = Geometry::prepare(1, &figure, &stop).unwrap() else {
            panic!("ordinary fixture")
        };
        for (series, points) in figure.series[..2].iter().zip(&prepared.points[..2]) {
            assert_eq!(points.len(), 40_000);
            for (source, point) in series.points.iter().zip(points) {
                assert_eq!(
                    source.map(f64::to_bits),
                    [point.x.to_bits(), point.y.to_bits()]
                );
            }
        }
        for dark in [false, true] {
            let ctx = egui::Context::default();
            ctx.set_visuals(if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(960., 650.));
            let mut output = None;
            for _ in 0..3 {
                output = Some(crate::headless::run(
                    &ctx,
                    egui::RawInput {
                        screen_rect: Some(screen),
                        ..Default::default()
                    },
                    |root| {
                        egui::CentralPanel::default().show(root, |ui| {
                            let view = figure_view::show_interactive_prepared(
                                ui,
                                &figure,
                                prepared.view(),
                                true,
                                None,
                            );
                            assert!(view.bounds.iter().all(|value| value.is_finite()));
                        });
                    },
                ));
            }
            let output = output.unwrap();
            assert!(!output.shapes.is_empty());
            let id = egui::Id::new(("detached_figure", egui::ViewportId::ROOT));
            let memory = egui_plot::PlotMemory::load(&ctx, id).unwrap();
            assert!(screen.contains_rect(*memory.transform().frame()));
            assert!(memory.transform().frame().width() > 500.);
        }
        assert!(figure.same_snapshot(&original));
        assert!(figure.series[0].points.dense().is_none());
    }
}
