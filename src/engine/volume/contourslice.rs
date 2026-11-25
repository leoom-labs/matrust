//! Contours of sampled volume slices, drawn as open patches.
use super::{
    super::{
        Engine, Value,
        plots_2d_more::contour::{mesh_at, slice_levels, slice_lines},
    },
    slice::{SampledSlice, sample, split_method},
};
use crate::{
    Result,
    plot::{MAX_POINTS, MAX_SERIES, PlotKind, Series, view3d},
};

const WHO: &str = "contourslice";
const MAX_TRACE_WORK: usize = 100_000_000;

pub(in crate::engine) fn call(e: &mut Engine, args: &[Value]) -> Result<Value> {
    let (args, method) = split_method(WHO, args)?;
    let (args, levels) = if matches!(args.len(), 5 | 8) {
        let (levels, args) = args.split_last().unwrap();
        (args, Some(levels))
    } else {
        (args, None)
    };
    let levels = slice_levels(levels)?;
    let slices = sample(e, WHO, args, method, MAX_SERIES)?;
    let mut figure = e.figure.clone().unwrap_or_default();
    let first = figure.axes.series.len();
    let mut used_series = figure.series_count();
    let mut used_points = figure.geometry_ledger()?.cost().source_units();
    let mut work_remaining = MAX_TRACE_WORK;
    let mut grouped: Vec<(f64, Vec<[f64; 2]>, Vec<f64>)> = Vec::new();
    for SampledSlice { xyz, values } in slices {
        let lines = slice_lines(e, &values, &levels, &mut work_remaining)?;
        let [x, y, z] = xyz;
        for (level, line) in lines {
            let index = if let Some(index) = grouped.iter().position(|g| g.0 == level) {
                index
            } else {
                used_series += 1;
                if used_series > MAX_SERIES {
                    return Err(format!(
                        "{WHO}: contour levels exceed the figure's {MAX_SERIES}-series limit; use fewer levels"
                    ));
                }
                grouped.push((level, Vec::new(), Vec::new()));
                grouped.len() - 1
            };
            let (_, points, heights) = &mut grouped[index];
            let separator = usize::from(!points.is_empty());
            used_points = used_points
                .saturating_add(line.len())
                .saturating_add(separator);
            if used_points > MAX_POINTS {
                return Err(format!(
                    "{WHO}: contour lines exceed the figure's {MAX_POINTS}-point limit"
                ));
            }
            if separator != 0 {
                points.push([f64::NAN, f64::NAN]);
                heights.push(f64::NAN);
            }
            for (i, [col, row]) in line.into_iter().enumerate() {
                if i.is_multiple_of(4096) {
                    e.check_cancel()?;
                }
                points.push([mesh_at(&x, col, row), mesh_at(&y, col, row)]);
                heights.push(mesh_at(&z, col, row));
            }
        }
    }
    let patches: Vec<_> = grouped
        .into_iter()
        .map(|(level, points, heights)| {
            let mut patch = Series::new(points, PlotKind::Patch);
            patch.z = heights.into();
            patch.slice_contour_level = Some(level);
            patch.explicit_color = true;
            patch.line_width = Some(0.5);
            patch
        })
        .collect();
    if patches.is_empty() {
        return e.graphics_series_result(&figure, first);
    }
    let defaults = e.graphics_plot_axes_defaults(&mut figure)?;
    if first == 0 && !figure.axes.hold && figure.axes.extras.images.is_empty() {
        figure.axes.view = view3d::VIEW_3D;
    }
    figure.add(patches)?;
    figure.prepare_legend_layout()?;
    figure.validate()?;
    let result = e.graphics_series_result(&figure, first)?;
    e.check_cancel()?;
    e.graphics_finish_axes_defaults(&figure, defaults);
    e.publish_graphics_figure(e.current_figure, figure);
    Ok(result)
}
