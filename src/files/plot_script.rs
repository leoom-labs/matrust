//! Data-only MATLAB plot reconstruction scripts; exporting never executes code.
use crate::{
    Result,
    plot::{AxisScale, Figure, LinePattern, PlotKind},
};
use std::{
    fmt::{self, Write},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

#[path = "bar_replay.rs"]
mod bar_replay;

const LIMIT: usize = 64 * 1024 * 1024;
pub(super) struct Text(pub(super) String);
impl Write for Text {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let needed = self.0.len().checked_add(s.len()).ok_or(fmt::Error)?;
        if needed > LIMIT {
            return Err(fmt::Error);
        }
        if needed > self.0.capacity() {
            let target = needed
                .max(self.0.capacity().saturating_mul(2))
                .clamp(65536, LIMIT);
            self.0
                .try_reserve_exact(target - self.0.len())
                .map_err(|_| fmt::Error)?;
        }
        self.0.push_str(s);
        Ok(())
    }
}
fn check(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err("Cancelled".into())
    } else {
        Ok(())
    }
}
struct Number(f64);
impl fmt::Display for Number {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        let v = self.0;
        if v.is_nan() {
            out.write_str("NaN")
        } else if v == f64::INFINITY {
            out.write_str("Inf")
        } else if v == f64::NEG_INFINITY {
            out.write_str("-Inf")
        } else {
            write!(out, "{v:.17e}")
        }
    }
}
fn number(v: f64) -> Number {
    Number(v)
}

fn text(s: &str) -> String {
    if s.chars().all(|c| c.is_ascii() && !c.is_ascii_control()) {
        format!("'{}'", s.replace('\'', "''"))
    } else {
        let units = s
            .encode_utf16()
            .map(|u| u.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        format!("char([{units}])")
    }
}
fn vector(values: &[f64]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|v| number(*v).to_string())
            .collect::<Vec<_>>()
            .join(" ")
    )
}
fn on(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}
fn scale(value: AxisScale) -> &'static str {
    if value == AxisScale::Log {
        "log"
    } else {
        "linear"
    }
}

/// Reconstructs original data and supported appearance with documented MATLAB
/// graphics commands. Does not replace an editable MFIG document or run code.
/// MATLAB rendering/execution is not currently verified by the licensed reference.
pub fn write_matlab_plot_script_with_cancel(
    path: &Path,
    figure: &Figure,
    cancel: &AtomicBool,
) -> Result<()> {
    write_matlab_plot_script_with_theme(path, figure, crate::plot::BaseTheme::Light, cancel)
}
/// Export resolved series RGB; MFIG retains automatic palette intent.
pub fn write_matlab_plot_script_with_theme(
    path: &Path,
    figure: &Figure,
    theme: crate::plot::BaseTheme,
    cancel: &AtomicBool,
) -> Result<()> {
    check(cancel)?;
    validate_matlab_plot_script_path(path)?;
    let script = matlab_plot_script_with_theme(figure, theme, cancel)?;
    check(cancel)?;
    super::atomic_write(path, script.as_bytes())
}

/// Validate the final script name, separately from any atomic staging path.
pub fn validate_matlab_plot_script_path(path: &Path) -> Result<()> {
    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if !path
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("m"))
        || !super::valid_name(name)
        || crate::syntax::keyword(name)
    {
        return Err("Use a script filename such as figure_1.m (letters, digits and underscores, starting with a letter; at most 63 characters).".into());
    }
    // A script named after one of its own graphics calls shadows that function.
    if [
        "figure",
        "axes",
        "subplot",
        "gca",
        "plot",
        "scatter",
        "stairs",
        "stem",
        "patch",
        "set",
        "title",
        "sgtitle",
        "xlabel",
        "ylabel",
        "legend",
        "annotation",
        "cell",
        "reshape",
        "char",
        "ones",
        "repmat",
        "size",
        "nan",
        "inf",
    ]
    .iter()
    .any(|reserved| name.eq_ignore_ascii_case(reserved))
    {
        return Err("Choose a script name that does not shadow a function this script calls, such as figure_1.m.".into());
    }
    Ok(())
}

/// Encode bounded text without a destination name, for atomic batch staging.
pub fn matlab_plot_script(figure: &Figure, cancel: &AtomicBool) -> Result<String> {
    matlab_plot_script_with_theme(figure, crate::plot::BaseTheme::Light, cancel)
}
pub fn matlab_plot_script_with_theme(
    figure: &Figure,
    theme: crate::plot::BaseTheme,
    cancel: &AtomicBool,
) -> Result<String> {
    check(cancel)?;
    figure.validate()?;
    baseline_replay_preflight(figure, cancel)?;
    let mut out = Text(String::new());
    // Two signed scientific f64 values take at most 53 bytes per row. Reserve
    // sample text once; metadata can grow geometrically within the same limit.
    // Fixed-size growth repeatedly copied large exports on the Windows heap.
    let points: usize = figure
        .all_axes()
        .flat_map(|(_, axes)| &axes.series)
        .map(|series| series.points.len())
        .sum();
    let capacity = points.saturating_mul(53).saturating_add(4096).min(LIMIT);
    out.0
        .try_reserve_exact(capacity)
        .map_err(|_| "Cannot allocate MATLAB plot script buffer".to_owned())?;
    encode(&mut out, figure, theme, cancel, None).map_err(|e| {
        if cancel.load(Ordering::Relaxed) {
            "Cancelled".into()
        } else {
            format!("Cannot encode MATLAB plot script (64 MiB maximum): {e}")
        }
    })?;
    check(cancel)?;
    Ok(out.0)
}
pub(super) fn encode(
    out: &mut impl Write,
    figure: &Figure,
    theme: crate::plot::BaseTheme,
    cancel: &AtomicBool,
    bundle: Option<&super::plot_bundle::Data>,
) -> fmt::Result {
    if baseline_replay_preflight(figure, cancel).is_err() {
        return Err(fmt::Error);
    }
    let literal = |s: &str| bundle.map_or_else(|| Ok(text(s)), |b| b.text(s));
    let appearance = figure.appearance_policy();
    let resolved = figure.resolve_appearance(theme);
    let mut figure_properties = if appearance.canvas_mode == crate::plot::ColorMode::Manual {
        format!(
            "'Color',{},'Name',{}",
            vector(&figure.color.map(|c| f64::from(c) / 255.)),
            literal(&figure.name)?
        )
    } else {
        format!("'Name',{}", literal(&figure.name)?)
    };
    if let Some(invert) = figure.invert_hardcopy {
        figure_properties.push_str(&format!(",'InvertHardcopy','{}'", on(invert)));
    }
    let mut sample_index = 0;
    writeln!(
        out,
        "% MATLAB plot reconstruction exported by MatRust.\n% Uses MATLAB graphics commands. MatRust replay is subject to its script and figure limits.\n% Original samples are included. Exact ordinary Bar batches use Bar objects (MATLAB R2024a+).\n% Other Bar geometry uses Patch appearance reconstruction and does not preserve linked Bar identity.\n% Rendering, fonts and legend placement may differ. Not a MATLAB FIG file.\n% This script creates a new figure and uses mr_* workspace variables.\nmr_figure = figure({});",
        figure_properties
    )?;
    if figure.has_palette_metadata() {
        writeln!(
            out,
            "% Series colors are a resolved RGB snapshot; save MFIG to retain automatic palette intent."
        )?;
    }
    // Theme uses documented figure properties. Automatic canvas omits Color;
    // manual canvas sets Color. No undocumented Figure ColorMode is emitted.
    writeln!(
        out,
        "% Figure Theme properties require MATLAB R2025a or newer."
    )?;
    match appearance.theme {
        crate::plot::FigureTheme::Auto => writeln!(out, "set(mr_figure,'ThemeMode','auto');")?,
        theme => writeln!(out, "set(mr_figure,'Theme','{}');", theme.as_str())?,
    }
    let shared_title = figure.layout.as_ref().map_or("", |l| l.title.as_str());
    for (a, ((panel, axes), (_, position))) in figure
        .all_axes()
        .zip(figure.normalized_axes_positions())
        .enumerate()
    {
        if cancel.load(Ordering::Relaxed) {
            return Err(fmt::Error);
        }
        let id = a + 1;
        let bar_plan = bar_replay::Plan::new(axes, cancel).map_err(|_| fmt::Error)?;
        let (xlim, ylim) = axes.bounds();
        let base = if axes.yscale == AxisScale::Log {
            if ylim[1] < 0. { -1. } else { 1. }
        } else {
            0.
        };
        let automatic = figure.layout.is_none() && axes.placement.is_none();
        // Grid positions describe panels, not normalized data boxes. Their
        // decorations and outside legends scale with the smallest panel. A
        // custom axes plus OuterPosition would instead use full-canvas margins.
        let grid = !automatic && panel.custom.is_none() && axes.placement.is_none();
        if automatic {
            // No explicit geometry was stored. Keep the ordinary axes mode;
            // even setting its default Position would create a custom layout.
            writeln!(
                out,
                "\n% Axes {id}\nmr_ax{id} = gca;\nset(mr_ax{id},'NextPlot','add');"
            )?;
        } else if grid {
            write!(
                out,
                "\n% Axes {id}\nsubplot({},{},[",
                panel.rows, panel.columns
            )?;
            for row in panel.top..panel.bottom {
                for column in panel.left..panel.right {
                    write!(
                        out,
                        "{} ",
                        usize::from(row) * usize::from(panel.columns) + usize::from(column) + 1
                    )?;
                }
            }
            writeln!(
                out,
                "]);\nmr_ax{id} = gca;\nset(mr_ax{id},'NextPlot','add');"
            )?;
        } else {
            writeln!(
                out,
                "\n% Axes {id}\nmr_ax{id} = axes('Parent',mr_figure,'Units','normalized','Position',{},'NextPlot','add');",
                vector(&position)
            )?;
        }
        writeln!(
            out,
            "mr_series = cell(1,{});\nmr_labels = cell(1,{});",
            axes.series.len(),
            axes.series.len()
        )?;
        let outer_position = if automatic || grid {
            None
        } else {
            axes.placement.map(|p| p.requested_outer).or_else(|| {
                axes.legend_location.is_outside().then(|| {
                    let [l, t, r, b] = panel.rectangle();
                    [l, 1. - b, r - l, b - t]
                })
            })
        };
        if let Some(outer) = outer_position {
            let inner = axes.placement.is_some_and(|p| {
                p.constraint == crate::plot::PositionConstraint::InnerPosition || p.imported_solved
            });
            writeln!(
                out,
                "set(mr_ax{id},'OuterPosition',{},'ActivePositionProperty','{}');",
                vector(&outer),
                if inner { "position" } else { "outerposition" }
            )?;
            if axes.placement.is_some_and(|p| p.imported_solved) {
                writeln!(
                    out,
                    "% Imported solved geometry: restore the saved inner box after creating the legend; MATLAB may derive different outer margins."
                )?;
            }
        }
        for (i, series) in axes.series.iter().enumerate() {
            let index = i + 1;
            sample_index += 1;
            let bar_member = bar_plan.member(i);
            if let Some(group) = bar_plan.starting(i) {
                bar_replay::emit_group(
                    out,
                    axes,
                    group,
                    id,
                    sample_index,
                    bundle.is_some(),
                    cancel,
                )?;
            }
            let coordinates = series.points.patch().and_then(|p| p.script_coordinates());
            if bar_member.is_some() {
                // The real batch constructor above already consumed these samples.
            } else if let Some(coordinates) = &coordinates {
                compact_coordinates(out, coordinates, sample_index, bundle.is_some(), cancel)?;
            } else if bundle.is_some() {
                writeln!(
                    out,
                    "mr_data = mr_samples_{sample_index};\nclear mr_samples_{sample_index};\nmr_x = mr_data(:,1); mr_y = mr_data(:,2);"
                )?;
            } else {
                writeln!(
                    out,
                    "% Original X/Y samples for axes {id}, series {index}\nmr_data = ["
                )?;
                for (j, point) in series.points.iter().enumerate() {
                    if j % 1024 == 0 && cancel.load(Ordering::Relaxed) {
                        return Err(fmt::Error);
                    }
                    writeln!(out, "{} {};", number(point[0]), number(point[1]))?;
                }
                writeln!(
                    out,
                    "];\nmr_data = reshape(mr_data,[],2);\nmr_x = mr_data(:,1); mr_y = mr_data(:,2);"
                )?;
            }
            let rgb = vector(&resolved.series_color(series).map(|v| f64::from(v) / 255.));
            let pattern = match series.pattern {
                LinePattern::Solid => "-",
                LinePattern::None => "none",
                LinePattern::Dashed => "--",
                LinePattern::Dotted => ":",
                LinePattern::DashDot => "-.",
            };
            let marker = if series.markers || series.kind == PlotKind::Scatter {
                series.effective_marker().symbol()
            } else {
                "none"
            };
            let fill = if series.marker_is_filled() {
                rgb.clone()
            } else {
                "'none'".into()
            };
            match series.kind {
                PlotKind::Patch => {
                    let width = series.patch.ok_or(fmt::Error)?.vertices_per_face();
                    let (x, y) = if coordinates.is_some() {
                        (
                            "[mr_x1 mr_x2 mr_x3 mr_x4].'".to_owned(),
                            "[mr_y1 mr_y2 mr_y3 mr_y4].'".to_owned(),
                        )
                    } else {
                        (
                            format!("reshape(mr_x,{width},[])"),
                            format!("reshape(mr_y,{width},[])"),
                        )
                    };
                    writeln!(
                        out,
                        "mr_h = patch('Parent',mr_ax{id},'XData',{x},'YData',{y},'FaceColor',{},'EdgeColor','none');",
                        rgb
                    )?;
                }
                PlotKind::Bar if bar_member.is_some() => {
                    let (member, _) = bar_member.unwrap();
                    writeln!(
                        out,
                        "mr_h = mr_bars({});\nset(mr_h,'FaceColor',{},'EdgeColor','none');",
                        member + 1,
                        rgb
                    )?;
                }
                PlotKind::Bar => {
                    // A single patch per series keeps arbitrary spacing and grouped offsets.
                    writeln!(
                        out,
                        "mr_left = mr_x + {} - {};\nmr_right = mr_x + {} + {};\nmr_base = {} * ones(size(mr_y));",
                        number(series.bar_offset),
                        number(series.bar_width / 2.),
                        number(series.bar_offset),
                        number(series.bar_width / 2.),
                        number(base)
                    )?;
                    writeln!(
                        out,
                        "mr_h = patch('Parent',mr_ax{id},'XData',[mr_left mr_right mr_right mr_left].','YData',[mr_base mr_base mr_y mr_y].','FaceColor',{},'EdgeColor','none');",
                        rgb
                    )?;
                }
                PlotKind::Scatter if !series.has_marker_style() => {
                    // Preserve implicit marker size/shape and its legacy legend
                    // appearance through the real Scatter constructor defaults.
                    writeln!(
                        out,
                        "mr_h = scatter(mr_ax{id},mr_x,mr_y,'MarkerEdgeColor',{},'MarkerFaceColor',{});",
                        rgb, fill
                    )?;
                }
                PlotKind::Scatter => {
                    writeln!(
                        out,
                        "mr_h = scatter(mr_ax{id},mr_x,mr_y,{},'Marker','{marker}','MarkerEdgeColor',{},'MarkerFaceColor',{});",
                        number(series.marker_size_points().powi(2)),
                        rgb,
                        fill
                    )?;
                }
                kind => {
                    let command = match kind {
                        PlotKind::Stairs => "stairs",
                        PlotKind::Stem => "stem",
                        _ => "plot",
                    };
                    if series.markers && !series.has_marker_style() {
                        // A genuine legacy LineSpec keeps absent marker metadata
                        // absent, including the distinct filled-point legend size.
                        writeln!(
                            out,
                            "mr_h = {command}(mr_ax{id},mr_x,mr_y,'{marker}','Color',{},'LineStyle','{pattern}','MarkerFaceColor',{});",
                            rgb, fill
                        )?;
                    } else {
                        writeln!(
                            out,
                            "mr_h = {command}(mr_ax{id},mr_x,mr_y,'Color',{},'LineStyle','{pattern}','Marker','{marker}','MarkerFaceColor',{});",
                            rgb, fill
                        )?;
                    }
                    if kind == PlotKind::Stem
                        && axes.baseline_for_series(&series.identity).is_none()
                    {
                        writeln!(
                            out,
                            "set(mr_h,'BaseValue',{});",
                            number(series.base_value.unwrap_or(base))
                        )?;
                    }
                }
            }
            if let Some(coordinates) = &coordinates {
                write!(out, "clear mr_x1 mr_x2 mr_x3 mr_x4 mr_y1 mr_y2 mr_y3 mr_y4")?;
                for source in 1..=coordinates.sources.len() {
                    write!(out, " mr_coordinate_{source}")?;
                }
                if bundle.is_some()
                    && coordinates.corners.iter().flatten().any(|c| {
                        matches!(c, crate::plot::coordinates::ScriptCoordinate::Constant(_))
                    })
                {
                    write!(out, " mr_constants_{sample_index}")?;
                }
                if bundle.is_some() && coordinates.has_steps() {
                    write!(out, " mr_steps_{sample_index}")?;
                }
                writeln!(out, ";")?;
            }
            if matches!(series.kind, PlotKind::Bar | PlotKind::Patch)
                && (series.has_face_style()
                    || series.line_width.is_some()
                    || (series.kind == PlotKind::Bar && series.pattern != LinePattern::Solid))
            {
                let edge = series.face_edge_color.map_or_else(
                    || "'none'".into(),
                    |rgb| vector(&rgb.map(|v| f64::from(v) / 255.)),
                );
                writeln!(
                    out,
                    "set(mr_h,'EdgeColor',{edge},'LineStyle','{pattern}','LineWidth',{});",
                    number(series.line_width.unwrap_or(0.5))
                )?;
            }
            if !matches!(series.kind, PlotKind::Bar | PlotKind::Patch)
                && series.kind != PlotKind::Scatter
                && series.has_marker_style()
            {
                writeln!(
                    out,
                    "set(mr_h,'MarkerSize',{});",
                    number(series.marker_size_points())
                )?;
            }
            if !matches!(series.kind, PlotKind::Bar | PlotKind::Patch)
                && let Some(width) = series.line_width
            {
                // Absence carries existing renderer-specific legacy line/marker
                // widths; materializing 1.5 points changes marker outlines.
                writeln!(out, "set(mr_h,'LineWidth',{});", number(width))?;
            }
            writeln!(
                out,
                "set(mr_h,'DisplayName',{},'Visible','{}');\nmr_series{{{index}}} = mr_h;\nmr_labels{{{index}}} = {};",
                literal(&series.label)?,
                on(series.visible),
                literal(&series.label)?
            )?;
            if bar_member.is_some_and(|(member, count)| member + 1 == count) {
                writeln!(out, "clear mr_bars;")?;
            }
        }
        if let Some(first) = bar_plan.first()
            && axes
                .baseline_for_series(&axes.series[first].identity)
                .is_none()
        {
            let style = axes
                .observe_series_baseline(first)
                .map_err(|_| fmt::Error)?
                .style;
            // Spell the source implicit policy, including historical off groups,
            // without manufacturing a separate line or identity.
            writeln!(
                out,
                "mr_baseline = get(mr_series{{{}}},'BaseLine');\nset(mr_baseline,'Color',{},'LineWidth',{},'LineStyle','-','Visible','{}');\nclear mr_baseline;",
                first + 1,
                vector(&style.color),
                number(style.width_points),
                on(style.visible)
            )?;
        }
        // Emit in record order, after every chart exists. A real chart
        // BaseLine query retains membership and does not add an implicit line.
        for baseline in &axes.baselines {
            if cancel.load(Ordering::Relaxed) {
                return Err(fmt::Error);
            }
            let member = axes
                .series
                .iter()
                .position(|s| s.identity.same_object(&baseline.members[0]))
                .ok_or(fmt::Error)?
                + 1;
            writeln!(out, "mr_baseline = get(mr_series{{{member}}},'BaseLine');")?;
            if bar_plan.canonical_record(axes, baseline) {
                // Preserve an explicit record even when its final properties
                // equal the implicit style. Visibility-only edits keep latent
                // topology; a genuine temporary width change materializes the
                // same chart-owned object before its exact final properties.
                writeln!(out, "set(mr_baseline,'LineWidth',1.25);")?;
            }
            write!(out, "set(mr_baseline,")?;
            if let crate::plot::BaselineValue::Value(value) = baseline.value {
                write!(out, "'BaseValue',{},", number(value))?;
            }
            let pattern = match baseline.style.pattern {
                LinePattern::Solid => "-",
                LinePattern::Dashed => "--",
                LinePattern::Dotted => ":",
                LinePattern::DashDot => "-.",
                LinePattern::None => "none",
            };
            writeln!(
                out,
                "'Color',{},'LineWidth',{},'LineStyle','{}','Visible','{}');\nclear mr_baseline;",
                vector(&baseline.style.color),
                number(baseline.style.width_points),
                pattern,
                on(baseline.style.visible)
            )?;
        }
        writeln!(
            out,
            "set(mr_ax{id},'XScale','{}','YScale','{}','XLim',{},'YLim',{},'XGrid','{}','YGrid','{}','Box','on');",
            scale(axes.xscale),
            scale(axes.yscale),
            vector(&xlim),
            vector(&ylim),
            on(axes.xgrid),
            on(axes.ygrid)
        )?;
        // Numeric limits establish the intended scale/domain while rebuilding.
        // Restore each automatic mode after the samples exist, using ordinary
        // public MATLAB calls so later edits can expand these axes again.
        for (command, limits) in [("xlim", axes.xlim), ("ylim", axes.ylim)] {
            if limits.is_none() {
                writeln!(out, "{command}(mr_ax{id},'auto');")?;
            }
        }
        let colors = axes.colors.unwrap_or_default();
        for (property, rgb) in [
            ("Color", colors.background),
            ("XColor", colors.x),
            ("YColor", colors.y),
        ] {
            if let Some(rgb) = rgb {
                writeln!(out, "set(mr_ax{id},'{property}',{});", vector(&rgb))?;
            }
        }
        let grid_style = if let Some(grid) = colors.grid {
            Some(grid)
        } else if colors.x.is_some() || colors.y.is_some() {
            // A legacy imported RGB-only record has fixed Light appearance.
            // Materialize only its existing resolved grid appearance, not an
            // assumed historical MATLAB factory mode or alpha.
            let mut grid = crate::plot::AxesGridStyle::from_appearance(
                figure.resolve_appearance(crate::plot::BaseTheme::Light),
                false,
            );
            if colors.x.is_some() {
                grid.x_mode = crate::plot::ColorMode::Manual;
            }
            if colors.y.is_some() {
                grid.y_mode = crate::plot::ColorMode::Manual;
            }
            Some(grid)
        } else {
            None
        };
        if let Some(grid) = grid_style {
            writeln!(
                out,
                "set(mr_ax{id},'GridColor',{},'GridAlpha',{},'XColorMode','{}','YColorMode','{}','GridColorMode','{}','GridAlphaMode','{}');",
                vector(&grid.color),
                number(grid.alpha),
                grid.x_mode.as_str(),
                grid.y_mode.as_str(),
                grid.color_mode.as_str(),
                grid.alpha_mode.as_str()
            )?;
        }
        for (command, label, rgb) in [
            ("title", &axes.title, colors.title),
            ("xlabel", &axes.xlabel, colors.xlabel),
            ("ylabel", &axes.ylabel, colors.ylabel),
        ] {
            writeln!(
                out,
                "{command}(mr_ax{id},{},'Interpreter','none'{});",
                literal(label)?,
                rgb.map_or_else(String::new, |rgb| format!(",'Color',{}", vector(&rgb)))
            )?;
        }
        let mut legend_colors = String::new();
        if let Some(colors) = axes.colors.and_then(|c| c.legend) {
            for (property, rgb) in [
                ("Color", colors.background),
                ("TextColor", colors.text),
                ("EdgeColor", colors.border),
            ] {
                if let Some(rgb) = rgb {
                    write!(legend_colors, ",'{property}',{}", vector(&rgb))?;
                }
            }
        }
        if let Some(state) = &axes.legend_state {
            // Preserve exact subset/order without editing or copying samples.
            let indices = axes
                .legend_members()
                .map(|(i, _)| (i + 1).to_string())
                .collect::<Vec<_>>()
                .join(" ");
            writeln!(out, "mr_legend_indices = [{indices}];")?;
            writeln!(
                out,
                "legend(mr_ax{id},[mr_series{{mr_legend_indices}}],mr_labels(mr_legend_indices),'Interpreter','none','Location','{}','Visible','{}','AutoUpdate','{}'{legend_colors});",
                axes.legend_location.name(),
                on(axes.legend),
                on(state.auto_update)
            )?;
        } else if (axes.legend
            || !axes.legend_location.is_default()
            || axes.colors.is_some_and(|c| c.legend.is_some()))
            && !axes.series.is_empty()
        {
            writeln!(
                out,
                "legend(mr_ax{id},[mr_series{{:}}],mr_labels,'Interpreter','none','Location','{}'{}{legend_colors});",
                axes.legend_location.name(),
                if axes.legend { "" } else { ",'Visible','off'" }
            )?;
        }
        if automatic || grid {
            writeln!(
                out,
                "set(mr_ax{id},'NextPlot','{}');",
                if axes.hold { "add" } else { "replace" }
            )?;
        } else {
            let hold_outer = outer_position.is_some()
                && axes.placement.is_none_or(|p| {
                    p.constraint == crate::plot::PositionConstraint::OuterPosition
                        && !p.imported_solved
                });
            writeln!(
                out,
                "set(mr_ax{id},'{}',{},'ActivePositionProperty','{}','NextPlot','{}');",
                if hold_outer {
                    "OuterPosition"
                } else {
                    "Position"
                },
                vector(&if hold_outer {
                    outer_position.unwrap()
                } else {
                    position
                }),
                // Position exports hold the inner box, not an automatically
                // allocated outer box computed before later axes exist.
                if hold_outer {
                    "outerposition"
                } else {
                    "position"
                },
                if axes.hold { "add" } else { "replace" }
            )?;
        }
    }
    if !shared_title.is_empty() {
        writeln!(
            out,
            "% Shared titles use sgtitle (MATLAB R2018b or later).\nsgtitle(mr_figure,{},'Interpreter','none');",
            literal(shared_title)?
        )?;
    }
    for textbox in &figure.textboxes {
        if cancel.load(Ordering::Relaxed) {
            return Err(fmt::Error);
        }
        writeln!(
            out,
            "annotation(mr_figure,'textbox',{},'String',{},'Interpreter','none','EdgeColor','none','HorizontalAlignment','{}','FontWeight','{}','FontSize',{},'Color',{},'Visible','{}');",
            vector(&textbox.position),
            literal(&textbox.text)?,
            match textbox.horizontal {
                crate::plot::TextAlign::Left => "left",
                crate::plot::TextAlign::Center => "center",
                crate::plot::TextAlign::Right => "right",
            },
            if textbox.bold { "bold" } else { "normal" },
            number(textbox.font_size),
            vector(&textbox.color.map(|v| f64::from(v) / 255.)),
            on(textbox.visible)
        )?;
    }
    if !figure.empty_layout() {
        writeln!(out, "set(mr_figure,'CurrentAxes',mr_ax1);")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn script_text_bounds_prevent_unbounded_allocation() {
        let mut out = Text(String::new());
        let chunk = "x".repeat(65536);
        for _ in 0..LIMIT / chunk.len() {
            out.write_str(&chunk).unwrap();
        }
        assert!(out.write_str("x").is_err());
        assert_eq!(out.0.len(), LIMIT);
        assert!(out.0.capacity() <= LIMIT);
    }
}

// This private attempt never constructs the public 64 MiB standalone buffer.
// The unchanged parser applies its 100,000-token/node checks to <=1 MB here.
const REPLAY_SOURCE_BYTES: usize = 1_000_000;
pub(super) struct ReplayText(pub(super) String);
impl Write for ReplayText {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let needed = self.0.len().checked_add(text.len()).ok_or(fmt::Error)?;
        if needed > REPLAY_SOURCE_BYTES {
            return Err(fmt::Error);
        }
        if needed > self.0.capacity() {
            let target = needed
                .max(self.0.capacity().saturating_mul(2))
                .clamp(65536, REPLAY_SOURCE_BYTES);
            self.0
                .try_reserve_exact(target - self.0.len())
                .map_err(|_| fmt::Error)?;
        }
        self.0.push_str(text);
        Ok(())
    }
}
pub(super) fn replay_inline(
    figure: &Figure,
    theme: crate::plot::BaseTheme,
    cancel: &AtomicBool,
) -> Result<Option<String>> {
    check(cancel)?;
    for (_, axes) in figure.all_axes() {
        for series in &axes.series {
            if let Some(points) = series.points.patch() {
                if points.needs_exact_companion(cancel)? {
                    return Ok(None);
                }
            } else {
                // The ordinary NaN literal has one canonical payload. Preserve
                // other raw payloads in MAT before a first Bar-to-Patch replay.
                // This scan is cancellable and makes no sample allocation.
                for (index, point) in series.points.iter().enumerate() {
                    if index.is_multiple_of(1024) {
                        check(cancel)?;
                    }
                    if point
                        .iter()
                        .any(|value| value.is_nan() && value.to_bits() != f64::NAN.to_bits())
                    {
                        return Ok(None);
                    }
                }
            }
        }
    }
    let mut out = ReplayText(String::new());
    let encoded = encode(&mut out, figure, theme, cancel, None).is_ok();
    check(cancel)?;
    if encoded && crate::syntax::parse(&out.0).is_ok() {
        Ok(Some(out.0))
    } else {
        // Drop the bounded failed attempt before allocating companion matrices.
        Ok(None)
    }
}
#[cfg(test)]
mod replay_buffer_tests {
    use super::*;
    #[test]
    fn replay_attempt_stops_at_source_bound_without_growing_to_standalone_capacity() {
        let mut text = ReplayText(String::new());
        for _ in 0..1000 {
            text.write_str(&"x".repeat(1000)).unwrap();
        }
        assert_eq!(text.0.len(), REPLAY_SOURCE_BYTES);
        assert!(text.0.capacity() <= REPLAY_SOURCE_BYTES);
        assert!(text.write_str("x").is_err());
        assert_eq!(text.0.len(), REPLAY_SOURCE_BYTES);
        assert_eq!(LIMIT, 64 * 1024 * 1024);
    }
}

/// Preserve raw sources and each scalar operation in its original order. The
/// generated expressions use ordinary MATLAB indexing/arithmetic/transpose.
fn compact_coordinates(
    out: &mut impl Write,
    coordinates: &crate::plot::coordinates::CoordinateScript,
    series: usize,
    bundle: bool,
    cancel: &AtomicBool,
) -> fmt::Result {
    use crate::plot::coordinates::{ScalarStep, ScriptCoordinate};
    for (index, source) in coordinates.sources.iter().enumerate() {
        let id = index + 1;
        if bundle {
            writeln!(
                out,
                "mr_coordinate_{id} = mr_coordinates_{series}_{id};\nclear mr_coordinates_{series}_{id};"
            )?;
        } else {
            writeln!(out, "mr_coordinate_{id} = [")?;
            for index in 0..source.len() {
                if index.is_multiple_of(1024) && cancel.load(Ordering::Relaxed) {
                    return Err(fmt::Error);
                }
                writeln!(out, "{};", number(source.value(index)))?;
            }
            writeln!(
                out,
                "];\nmr_coordinate_{id} = reshape(mr_coordinate_{id},[],1);"
            )?;
        }
    }
    for (corner, axes) in coordinates.corners.iter().enumerate() {
        for (axis, coordinate) in axes.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return Err(fmt::Error);
            }
            let name = format!("mr_{}{}", if axis == 0 { "x" } else { "y" }, corner + 1);
            match *coordinate {
                ScriptCoordinate::Constant(value) => {
                    let value = if bundle {
                        format!("mr_constants_{series}({})", axis * 4 + corner + 1)
                    } else {
                        number(value).to_string()
                    };
                    writeln!(out, "{name} = repmat({value},{},1);", coordinates.count)?;
                }
                ScriptCoordinate::Lane {
                    source,
                    offset,
                    stride,
                    steps,
                } => {
                    if coordinates.count == 0 {
                        // Empty lane keeps no payload dependence after evaluation.
                        writeln!(out, "{name} = reshape([],0,1);")?;
                    } else {
                        let first = offset + 1;
                        let last = offset + (coordinates.count - 1) * stride + 1;
                        writeln!(
                            out,
                            "{name} = reshape(mr_coordinate_{}({first}:{stride}:{last}),[],1);",
                            source + 1
                        )?;
                    }
                    for (slot, step) in steps.into_iter().enumerate() {
                        let Some(step) = step else {
                            continue;
                        };
                        let operand = if bundle {
                            format!("mr_steps_{series}({})", (axis * 2 + slot) * 4 + corner + 1)
                        } else {
                            number(step.operand()).to_string()
                        };
                        match step {
                            ScalarStep::Add(_) => writeln!(out, "{name} = {name} + {operand};")?,
                            ScalarStep::Subtract(_) => {
                                writeln!(out, "{name} = {name} - {operand};")?
                            }
                            ScalarStep::Multiply(_) => {
                                writeln!(out, "{name} = {name} * {operand};")?
                            }
                            ScalarStep::LeftAdd(_) => {
                                writeln!(out, "{name} = {operand} + {name};")?
                            }
                            ScalarStep::LeftSubtract(_) => {
                                writeln!(out, "{name} = {operand} - {name};")?
                            }
                            ScalarStep::LeftMultiply(_) => {
                                writeln!(out, "{name} = {operand} * {name};")?
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

// Genuine chart queries reconstruct only established member relationships.
// Arbitrary groups keep their explicit MFIG fallback; no yline equivalence.
pub(super) fn baseline_replay_preflight(figure: &Figure, cancel: &AtomicBool) -> Result<()> {
    for (_, axes) in figure.all_axes() {
        if axes.series.is_empty()
            && axes.legend_state.is_none()
            && axes.colors.is_some_and(|c| c.legend.is_some())
        {
            return Err("Empty unbound legend RGB requires MFIG; script replay needs an explicit legend object".into());
        }
        if figure.appearance.is_some()
            && axes.colors.is_some_and(|colors| {
                colors.grid.is_none() && (colors.x.is_some() || colors.y.is_some())
            })
        {
            return Err("Axes ruler RGB on a modern figure needs explicit grid state for script reconstruction; use MFIG".into());
        }
        let plan = bar_replay::Plan::new(axes, cancel)?;
        if plan.first().is_none() && axes.implicit_bar_baselines().next().is_some() {
            return Err("Visible arbitrary Bar baseline needs MFIG; Patch appearance replay cannot retain its linked identity".into());
        }
        for baseline in &axes.baselines {
            check(cancel)?;
            if plan.canonical_record(axes, baseline) {
                continue;
            }
            if baseline.members.len() != 1
                || axes
                    .series
                    .iter()
                    .find(|s| s.identity.same_object(&baseline.members[0]))
                    .is_none_or(|s| s.kind != PlotKind::Stem)
            {
                return Err("Script replay requires a canonical ordinary Bar group or an individual Stem baseline; use MFIG for arbitrary membership".into());
            }
            let member = axes
                .series
                .iter()
                .find(|s| s.identity.same_object(&baseline.members[0]))
                .unwrap();
            if !baseline.identity.same_object(&member.baseline_identity) {
                return Err("Script replay requires the Stem's own baseline identity; use MFIG to preserve an independent dormant baseline".into());
            }
        }
    }
    check(cancel)
}
