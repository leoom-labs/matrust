//! Filter response plots shared by the Filter Designer and the Filter
//! Visualization app: curves split at undefined values, stem plots and the
//! pole-zero plane. A caller passes one trace or several; nothing here
//! computes a response.
use eframe::egui;

/// One named trace of a curve or stem plot.
pub(crate) struct Series<'a> {
    pub name: &'a str,
    pub color: egui::Color32,
    pub x: &'a [f64],
    pub y: &'a [f64],
}

/// Zeros and poles of one filter. `marks` label a location that holds
/// several roots with their count.
pub(crate) struct Roots<'a> {
    pub zeros_name: &'a str,
    pub poles_name: &'a str,
    pub color: egui::Color32,
    pub zeros: &'a [[f64; 2]],
    pub poles: &'a [[f64; 2]],
    pub marks: &'a [([f64; 2], usize)],
}

/// The engine's series colour `index` (it wraps around).
pub(crate) fn series_color(index: usize) -> egui::Color32 {
    let [r, g, b] = matrust::plot::COLORS[index % matrust::plot::COLORS.len()];
    egui::Color32::from_rgb(r, g, b)
}

/// Splits a curve at undefined (NaN or infinite) values so each piece draws
/// as one line.
pub(crate) fn segments(x: &[f64], y: &[f64]) -> Vec<Vec<[f64; 2]>> {
    let mut out: Vec<Vec<[f64; 2]>> = vec![Vec::new()];
    for (x, y) in x.iter().zip(y) {
        if x.is_finite() && y.is_finite() {
            out.last_mut().unwrap().push([*x, *y]);
        } else if !out.last().unwrap().is_empty() {
            out.push(Vec::new());
        }
    }
    out.retain(|s| !s.is_empty());
    out
}

/// A fixed (not draggable, not zoomable) plot with labelled axes.
pub(crate) fn plot(id: impl egui::AsId, height: f32, x: &str, y: &str) -> egui_plot::Plot<'static> {
    egui_plot::Plot::new(id)
        .height(height)
        .allow_scroll(false)
        .allow_drag(false)
        .allow_zoom(false)
        .allow_boxed_zoom(false)
        .allow_axis_zoom_drag(false)
        .allow_double_click_reset(false)
        .auto_bounds(egui::Vec2b::TRUE)
        .x_axis_label(x.to_owned())
        .y_axis_label(y.to_owned())
        .show_background(false)
}

/// Each series as a line, broken where it is undefined.
pub(crate) fn curves(plot_ui: &mut egui_plot::PlotUi<'_>, series: &[Series<'_>]) {
    for s in series {
        for piece in segments(s.x, s.y) {
            plot_ui.line(
                egui_plot::Line::new(s.name, piece)
                    .color(s.color)
                    .width(1.5),
            );
        }
    }
}

/// Each series as stems from zero.
pub(crate) fn stems(plot_ui: &mut egui_plot::PlotUi<'_>, series: &[Series<'_>]) {
    for s in series {
        let points: Vec<[f64; 2]> =
            s.x.iter()
                .zip(s.y)
                .filter(|(x, y)| x.is_finite() && y.is_finite())
                .map(|(x, y)| [*x, *y])
                .collect();
        plot_ui.points(
            egui_plot::Points::new(s.name, points)
                .stems(0.)
                .radius(2.)
                .color(s.color),
        );
    }
}

/// The unit circle, then every filter's zeros (circles) and poles (crosses).
pub(crate) fn pole_zero(
    plot_ui: &mut egui_plot::PlotUi<'_>,
    circle: egui::Color32,
    roots: &[Roots<'_>],
) {
    let unit: Vec<[f64; 2]> = (0..=256)
        .map(|i| {
            let t = i as f64 / 256. * std::f64::consts::TAU;
            [t.cos(), t.sin()]
        })
        .collect();
    plot_ui.line(
        egui_plot::Line::new("Unit circle", unit)
            .color(circle)
            .width(1.),
    );
    for r in roots {
        plot_ui.points(
            egui_plot::Points::new(r.zeros_name, r.zeros.to_vec())
                .shape(egui_plot::MarkerShape::Circle)
                .filled(false)
                .radius(5.)
                .color(r.color),
        );
        plot_ui.points(
            egui_plot::Points::new(r.poles_name, r.poles.to_vec())
                .shape(egui_plot::MarkerShape::Cross)
                .radius(5.)
                .color(r.color),
        );
        for ([x, y], count) in r.marks {
            plot_ui.text(
                egui_plot::Text::new(
                    r.zeros_name,
                    egui_plot::PlotPoint::new(*x, *y),
                    format!("  {count}"),
                )
                .anchor(egui::Align2::LEFT_BOTTOM)
                .color(r.color),
            );
        }
    }
}
