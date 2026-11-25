//! 3-D line and scatter plots: plot3, scatter3, view, zlabel, zlim and the
//! projected display. Expected behaviour follows the published MathWorks
//! documentation (behaviour only, never MATLAB output). View matrices come
//! from GNU Octave 10.3 running tool/refs/plot3d/make_refs.m
//! (tests/fixtures/plot3d/viewmtx.json): Octave has no viewmtx, so the script
//! writes out MATLAB's documented viewmtx formula, T = Telev * Tazim.
use matrust::{
    Engine, files,
    plot::{Figure, PlotKind, view3d},
};

fn engine(source: &str) -> Engine {
    let mut e = Engine::default();
    e.execute(source).unwrap();
    e
}
fn run(source: &str) -> Figure {
    engine(source).figure.unwrap()
}
fn values(e: &mut Engine, expression: &str) -> Vec<f64> {
    e.execute(&format!("zz__ = double({expression});")).unwrap();
    e.variables["zz__"].data().to_vec()
}
fn error(source: &str) -> String {
    let text = Engine::default().execute(source).unwrap_err().to_string();
    // Drop the "Line 1, column 1: " location prefix.
    text.split_once(": ")
        .map_or(text.clone(), |(_, rest)| rest.to_owned())
}
fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-12 * (1. + b.abs())
}

#[derive(serde::Deserialize)]
struct Fixture {
    generator: String,
    limits: Vec<[f64; 2]>,
    points: Vec<[f64; 3]>,
    views: Vec<FixtureView>,
}
#[derive(serde::Deserialize)]
struct FixtureView {
    az: f64,
    el: f64,
    #[serde(rename = "T")]
    t: Vec<[f64; 4]>,
    uv: Vec<[f64; 2]>,
}
fn fixture() -> Fixture {
    serde_json::from_str(include_str!("fixtures/plot3d/viewmtx.json")).unwrap()
}

#[test]
fn view_matrix_matches_the_octave_viewmtx_formula_fixture() {
    let fixture = fixture();
    // Written by an Octave 10.3 run of the generator, not a port of it.
    assert!(
        fixture
            .generator
            .starts_with("tool/refs/plot3d/make_refs.m run with GNU Octave 10.3"),
        "{}",
        fixture.generator
    );
    assert_eq!(fixture.views.len(), 5);
    for view in &fixture.views {
        let t = view3d::viewmtx(view.az, view.el);
        for (row, expected) in t.iter().zip(&view.t) {
            for (a, b) in row.iter().zip(expected) {
                assert!(
                    close(*a, *b),
                    "viewmtx({}, {}): {a} vs {b}",
                    view.az,
                    view.el
                );
            }
        }
    }
}

#[test]
fn known_points_project_to_the_reference_screen_coordinates() {
    let fixture = fixture();
    for view in &fixture.views {
        let t = view3d::viewmtx(view.az, view.el);
        for (point, expected) in fixture.points.iter().zip(&view.uv) {
            let unit = std::array::from_fn(|i| view3d::unit(point[i], fixture.limits[i]));
            let [u, v, _] = view3d::project(&t, unit);
            assert!(
                close(u, expected[0]) && close(v, expected[1]),
                "view ({}, {}) point {point:?}: ({u}, {v}) vs {expected:?}",
                view.az,
                view.el
            );
        }
    }
}

#[test]
fn plot3_helix_is_one_line_with_zdata_and_the_3d_view() {
    let mut e = engine("t = 0:0.1:10; h = plot3(sin(t), cos(t), t);");
    let f = e.figure.as_ref().unwrap();
    assert_eq!(f.series.len(), 1);
    assert_eq!(f.series[0].kind, PlotKind::Line);
    assert_eq!(f.series[0].z.len(), 101);
    assert_eq!(f.view, view3d::VIEW_3D);
    assert!(f.is_3d());
    let t = values(&mut e, "t");
    assert_eq!(values(&mut e, "h.ZData"), t);
    assert_eq!(values(&mut e, "get(h, 'ZData')"), t);
    assert_eq!(values(&mut e, "view"), [-37.5, 30.]);
    assert_eq!(values(&mut e, "zlim"), [0., 10.]);
}

#[test]
fn plot3_matrix_columns_triplets_linespec_and_nan_breaks() {
    // One line per column; a vector matching the rows is shared.
    let f = run("x = [1 2 3]; Y = [1 4; 2 5; 3 6]; Z = [7 10; 8 11; 9 12]; plot3(x, Y, Z);");
    assert_eq!(f.series.len(), 2);
    assert_eq!(
        f.series[1].points.iter().collect::<Vec<_>>(),
        [[1., 4.], [2., 5.], [3., 6.]]
    );
    assert_eq!(&*f.series[1].z, &[10., 11., 12.]);
    // A vector matching the columns gives one line per row.
    let f = run("plot3([1 2], [1 2; 3 4; 5 6], [0 0; 1 1; 2 2]);");
    assert_eq!(f.series.len(), 3);
    assert_eq!(&*f.series[2].z, &[2., 2.]);
    // Several triplets, each with its own LineSpec.
    let f = run("plot3([1 2], [3 4], [5 6], 'r--o', [0 1], [0 1], [0 1], 'k:');");
    assert_eq!(f.series.len(), 2);
    assert_eq!(f.series[0].color, [255, 0, 0]);
    assert!(f.series[0].markers);
    assert_eq!(f.series[1].color, [0, 0, 0]);
    assert_eq!(f.series[1].pattern, matrust::plot::LinePattern::Dotted);
    // Name-value pairs that plot accepts.
    let f = run("plot3([1 2], [3 4], [5 6], 'LineWidth', 2, 'DisplayName', 'path');");
    assert_eq!(f.series[0].line_width, Some(2.));
    assert_eq!(f.series[0].label, "path");
    // NaN in any coordinate breaks the projected line.
    let f = run("plot3([1 2 3 4], [1 2 3 4], [1 NaN 3 4]);");
    let display = view3d::display_view(&f.axes);
    let points: Vec<_> = display.series[0].points.iter().collect();
    assert!(points[1].iter().all(|v| v.is_nan()));
    assert!(points[0].iter().chain(&points[2]).all(|v| v.is_finite()));
}

#[test]
fn plot3_size_mismatches_name_the_problem() {
    assert_eq!(
        error("plot3([1 2 3], [1 2], [1 2 3])"),
        "plot3: X, Y and Z must be vectors of the same length"
    );
    assert_eq!(
        error("plot3(ones(2,3), ones(3,2), ones(2,3))"),
        "plot3: X, Y and Z matrices must be the same size"
    );
    assert_eq!(
        error("plot3([1 2 3 4], ones(2,3), ones(2,3))"),
        "plot3: vector lengths must match the rows or columns of the X, Y and Z matrices"
    );
    assert_eq!(
        error("plot3([1 2], [3 4], [5 6], [7 8])"),
        "plot3: data must be given as X, Y, Z triplets of real numeric arrays"
    );
}

/// Fewer than three coordinates is an arity error, never a panic, for
/// plot3 and scatter3 alike, also after a leading axes handle.
#[test]
fn plot3_and_scatter3_with_fewer_than_three_arguments_error() {
    for name in ["plot3", "scatter3"] {
        let expected = format!("{name}: not enough input arguments; X, Y and Z are required");
        for call in [
            format!("{name}()"),
            format!("{name}([1 2])"),
            format!("{name}([1 2], [3 4])"),
            format!("ax = gca; {name}(ax)"),
            format!("ax = gca; {name}(ax, [1 2], [3 4])"),
        ] {
            let mut e = Engine::default();
            let message = e.execute(&call).unwrap_err().to_string();
            assert!(message.ends_with(&expected), "{call}: {message}");
            // The engine is still usable afterwards.
            e.execute(&format!("{name}([1 2], [3 4], [5 6]);")).unwrap();
        }
    }
}

#[test]
fn scatter3_sizes_colors_filled_and_markers_follow_scatter() {
    let f = run("scatter3([1 2 3], [3 1 2], [0 1 2]);");
    assert_eq!(f.series.len(), 1);
    assert_eq!(f.series[0].kind, PlotKind::Scatter);
    assert_eq!(&*f.series[0].z, &[0., 1., 2.]);
    assert_eq!(f.view, view3d::VIEW_3D);
    // S is a marker area, as for scatter: 36 points^2 -> 6 points.
    let f = run("scatter3([1 2], [3 4], [5 6], 36, 'r', 'filled');");
    let s = &f.series[0];
    assert_eq!(s.marker_size, Some(6.));
    assert_eq!(s.color, [255, 0, 0]);
    assert!(s.explicit_color && s.marker_filled);
    let reference = run("scatter([1 2], [3 4], 36);");
    assert_eq!(reference.series[0].marker_size, s.marker_size);
    let reference = run("scatter([1 2], [3 4], 'r');");
    assert_eq!(reference.series[0].color, s.color);
    let f = run("scatter3([1 2], [3 4], [5 6], [], [0 0.5 1], 'd');");
    assert_eq!(f.series[0].color, [0, 128, 255]);
    assert_eq!(
        f.series[0].marker_shape,
        Some(matrust::plot::MarkerShape::Diamond)
    );
    assert!(!f.series[0].marker_filled);
    let f =
        run("h = scatter3([1 2], [3 4], [5 6], 'MarkerEdgeColor', 'g'); set(h, 'ZData', [7 8]);");
    assert_eq!(
        f.series[0].marker_edge_color,
        matrust::plot::MarkerColor::Rgb([0, 255, 0])
    );
    assert_ne!(f.series[0].color, [0, 255, 0]);
    assert_eq!(&*f.series[0].z, &[7., 8.]);
}

/// Per-point S and C arrays work on both scatter and scatter3, and mismatched
/// lengths are rejected.
#[test]
fn scatter3_supports_per_point_sizes_and_colors_and_rejects_mismatches() {
    let f = run("scatter3([1 2], [3 4], [5 6], [10 20], [1 0 0; 0 1 0]);");
    assert_eq!(f.series[0].marker_sizes.as_deref(), Some(&[10., 20.][..]));
    assert_eq!(
        f.series[0].scatter_cdata,
        Some(matrust::plot::ScatterCData::Rgb(
            vec![[255, 0, 0], [0, 255, 0]].into()
        ))
    );
    let f2 = run("scatter([1 2], [3 4], [10 20], [1; 2]);");
    assert_eq!(f2.series[0].marker_sizes.as_deref(), Some(&[10., 20.][..]));
    assert_eq!(
        f2.series[0].scatter_cdata,
        Some(matrust::plot::ScatterCData::Indexed(vec![1., 2.].into()))
    );
    assert!(!error("scatter3([1 2], [3 4], [5 6], [10 20 30])").is_empty());
    assert!(!error("scatter3([1 2], [3 4], [5 6], 36, [1 2 3]')").is_empty());
}

#[test]
fn view_forms_outputs_and_hold_rules() {
    let mut e = engine("plot3([0 1], [0 1], [0 1]); view(45, 20);");
    assert_eq!(values(&mut e, "view"), [45., 20.]);
    e.execute("view([10 -5]); v = view;").unwrap();
    assert_eq!(values(&mut e, "v"), [10., -5.]);
    e.execute("[az, el] = view;").unwrap();
    assert_eq!(values(&mut e, "[az el]"), [10., -5.]);
    e.execute("view(3);").unwrap();
    assert_eq!(values(&mut e, "view"), [-37.5, 30.]);
    e.execute("ax = gca; view(ax, 2);").unwrap();
    assert_eq!(values(&mut e, "view"), [0., 90.]);
    assert!(!e.figure.as_ref().unwrap().is_3d());
    e.execute("view(ax, [30 40]); w = ax.View;").unwrap();
    assert_eq!(values(&mut e, "w"), [30., 40.]);
    // hold on keeps the view for both 2-D and 3-D commands.
    e.execute("hold on; plot([0 1], [1 0]); plot3([0 1], [1 1], [1 1]);")
        .unwrap();
    let f = e.figure.as_ref().unwrap();
    assert_eq!(f.view, [30., 40.]);
    assert_eq!(f.series.len(), 3);
    assert!(f.series[1].z.is_empty());
    // plot with hold off resets the view to 2-D; plot3 with hold off sets view(3).
    e.execute("hold off; plot([1 2], [3 4]);").unwrap();
    assert_eq!(e.figure.as_ref().unwrap().view, view3d::VIEW_2D);
    e.execute("view(3); hold on; plot3([1 2], [3 4], [5 6]);")
        .unwrap();
    assert_eq!(e.figure.as_ref().unwrap().view, view3d::VIEW_3D);
    let f = run("plot([1 2], [3 4]); hold on; plot3([1 2], [3 4], [5 6]);");
    assert_eq!(f.view, view3d::VIEW_2D);
    assert!(error("view(5)").contains("2 or 3"));
    assert!(error("view([1 2 3])").contains("not supported yet"));
}

#[test]
fn view_2_after_plot3_draws_the_plain_xy_plot() {
    let three = run("plot3([1 2 3], [4 6 5], [7 8 9]); view(2);");
    assert!(!three.is_3d());
    assert!(!view3d::display_view(&three.axes).frame_hidden);
    let plain = run("plot([1 2 3], [4 6 5]);");
    assert_eq!(three.svg(), plain.svg());
}

#[test]
fn zlim_zlabel_axis_and_grid() {
    let mut e = engine("plot3([0 1], [0 2], [3 5]);");
    assert_eq!(values(&mut e, "zlim"), [3., 5.]);
    e.execute("zlim([0 10]); L = zlim;").unwrap();
    assert_eq!(values(&mut e, "L"), [0., 10.]);
    assert_eq!(values(&mut e, "get(gca, 'ZLim')"), [0., 10.]);
    e.execute("m = get(gca, 'ZLimMode'); zlim auto; a = get(gca, 'ZLimMode');")
        .unwrap();
    assert_eq!(
        values(&mut e, "m"),
        "manual".bytes().map(f64::from).collect::<Vec<_>>()
    );
    assert_eq!(
        values(&mut e, "a"),
        "auto".bytes().map(f64::from).collect::<Vec<_>>()
    );
    assert_eq!(values(&mut e, "zlim"), [3., 5.]);
    e.execute("zlim('manual'); plot3([0 1], [0 1], [0 1]);")
        .unwrap();
    // A replacing plot3 starts from automatic limits again.
    assert_eq!(values(&mut e, "zlim"), [0., 1.]);
    e.execute("hold on; zlim('manual'); plot3([0 1], [0 1], [0 9]);")
        .unwrap();
    assert_eq!(values(&mut e, "zlim"), [0., 1.]);
    e.execute("hold off; plot3([0 1], [0 2], [3 5]); axis([0 2 -1 3 0 6]);")
        .unwrap();
    assert_eq!(values(&mut e, "axis"), [0., 2., -1., 3., 0., 6.]);
    assert_eq!(values(&mut e, "zlim"), [0., 6.]);
    e.execute("axis auto;").unwrap();
    assert_eq!(values(&mut e, "zlim"), [3., 5.]);
    e.execute("zlabel('Height'); zl = get(gca, 'ZLabel'); s = zl.String;")
        .unwrap();
    assert_eq!(e.figure.as_ref().unwrap().zlabel, "Height");
    assert_eq!(
        values(&mut e, "s"),
        "Height".bytes().map(f64::from).collect::<Vec<_>>()
    );
    e.execute("ax = gca; ax.ZLabel.String = 'Depth'; zlabel(ax, 'Z2');")
        .unwrap();
    assert_eq!(e.figure.as_ref().unwrap().zlabel, "Z2");
    e.execute("grid on;").unwrap();
    let f = e.figure.as_ref().unwrap();
    assert!(f.xgrid && f.ygrid && f.zgrid);
    assert_eq!(values(&mut e, "strcmp(get(gca, 'ZGrid'), 'on')"), [1.]);
    e.execute("grid off; set(gca, 'ZGrid', 'on');").unwrap();
    let f = e.figure.as_ref().unwrap();
    assert!(!f.xgrid && !f.ygrid && f.zgrid);
    assert!(error("plot3([0 1],[0 1],[0 1]); zlim([2 1])").contains("increasing"));
}

#[test]
fn view_zlim_zlimmode_and_zdata_properties() {
    let mut e = engine("h = plot3([1 2 3], [1 2 3], [4 5 6]); ax = gca;");
    e.execute("set(ax, 'View', [20 10]);").unwrap();
    assert_eq!(values(&mut e, "get(ax, 'View')"), [20., 10.]);
    e.execute("ax.ZLim = [0 8];").unwrap();
    assert_eq!(values(&mut e, "ax.ZLim"), [0., 8.]);
    e.execute("ax.ZLimMode = 'auto';").unwrap();
    assert_eq!(values(&mut e, "ax.ZLim"), [4., 6.]);
    // Setting ZData redraws: the projected line moves.
    let before = e.figure.as_ref().unwrap().svg();
    e.execute("h.ZData = [6 5 4];").unwrap();
    assert_eq!(values(&mut e, "h.ZData"), [6., 5., 4.]);
    assert_ne!(e.figure.as_ref().unwrap().svg(), before);
    e.execute("set(h, 'XData', [1 2], 'YData', [3 4], 'ZData', [5 6]);")
        .unwrap();
    assert_eq!(values(&mut e, "h.ZData"), [5., 6.]);
    assert!(
        e.execute("h.ZData = [1 2 3];")
            .unwrap_err()
            .to_string()
            .contains("one value per")
    );
    // A 2-D line has empty ZData.
    let mut e = engine("h = plot([1 2], [3 4]);");
    assert_eq!(values(&mut e, "numel(h.ZData)"), [0.]);
}

/// ZLimMode is a known axes property: a bad value names the allowed ones
/// instead of calling the property unrecognized.
#[test]
fn bad_zlimmode_value_gives_the_value_error() {
    for set in ["ax.ZLimMode = 'bogus';", "set(ax, 'ZLimMode', 'bogus');"] {
        let mut e = engine("plot3([0 1], [0 1], [0 1]); ax = gca;");
        let message = e.execute(set).unwrap_err().to_string();
        assert!(
            message.contains("ZLimMode must be auto or manual"),
            "{set}: {message}"
        );
        assert!(!message.contains("nrecognized"), "{set}: {message}");
    }
}

/// A replacing (hold off) plot on a yyaxis axes keeps the other side but
/// takes the view of the new plot: view(3) for plot3/scatter3, view(2) for plot.
#[test]
fn hold_off_plot3_on_a_yyaxis_axes_switches_to_the_3d_view() {
    let mut e = engine("yyaxis left; plot([1 2], [3 4]); yyaxis right; plot([1 2], [5 6]);");
    e.execute("yyaxis left; plot3([1 2], [3 4], [5 6]);")
        .unwrap();
    let f = e.figure.as_ref().unwrap();
    assert_eq!(f.view, view3d::VIEW_3D);
    assert!(f.extras.yy.is_some());
    assert_eq!(values(&mut e, "view"), [-37.5, 30.]);
    e.execute("scatter3([1 2], [3 4], [5 6]); view(10, 20); plot([1 2], [3 4]);")
        .unwrap();
    assert_eq!(e.figure.as_ref().unwrap().view, view3d::VIEW_2D);
    e.execute("view(10, 20); scatter3([1 2], [3 4], [5 6]);")
        .unwrap();
    assert_eq!(e.figure.as_ref().unwrap().view, view3d::VIEW_3D);
}

#[test]
fn svg_of_a_3d_plot_draws_projected_box_ticks_and_no_2d_frame() {
    let f = run(
        "t = 0:0.5:10; plot3(sin(t), cos(t), t); grid on; xlabel('X axis'); ylabel('Y axis'); zlabel('Z axis'); title('Helix');",
    );
    let svg = f.svg();
    // No 2-D frame rectangle around the data area.
    assert!(!svg.contains("fill=\"none\" stroke=\"#777\"/><g clip-path"));
    // The helix as one projected polyline (the first series path).
    let display = view3d::display_view(&f.axes);
    assert!(display.frame_hidden);
    assert_eq!(display.series[0].points.len(), 21);
    // Box and grid lines are extra legend-hidden series.
    let decorations: Vec<_> = display.series[1..].iter().collect();
    // x, y and z grid lines plus the box with its tick marks.
    assert_eq!(decorations.len(), 4);
    assert!(
        decorations
            .iter()
            .all(|s| s.legend_hidden && s.label.is_empty())
    );
    assert!(svg.matches("<path fill=\"none\"").count() >= 5);
    // Tick labels on all three axes and the axis labels as text.
    // sin(t) spans about [-0.98 1], cos(t) [-0.99 1] and t [0 10].
    for label in [
        "-0.5", "0.5", "1.0", "0", "4", "10", "X axis", "Y axis", "Z axis", "Helix",
    ] {
        assert!(svg.contains(&format!(">{label}</text>")), "missing {label}");
    }
    // Legends never list the box.
    let f = run("plot3([0 1], [0 1], [0 1]); legend('path');");
    let display = view3d::display_view(&f.axes);
    assert_eq!(display.legend_entries().count(), 1);
}

#[test]
fn tick_label_edges_follow_the_lowest_and_leftmost_rule() {
    // view(3): the x labels sit on the y = ymin edge, the y labels on the
    // x = xmax edge, and the z labels on the vertical edge at the left.
    let f = run("plot3([0 1], [0 1], [0 1]);");
    let display = view3d::display_view(&f.axes);
    let t = view3d::viewmtx(-37.5, 30.);
    let texts = &display.extras.texts;
    let at = |label: &str| {
        texts
            .iter()
            .find(|text| text.string == label)
            .map(|text| text.position)
            .unwrap()
    };
    // Bottom-face edge y = 0 projects lower than y = 1, so it carries the x
    // labels (the first "0.4" is the x tick), below the edge.
    let low = view3d::project(&t, [0.4, 0., 0.])[1];
    let high = view3d::project(&t, [0.4, 1., 0.])[1];
    assert!(low < high);
    let x_tick = at("0.4");
    assert!(x_tick[1] < low);
    // The x = 0 edge is lower than x = 1 and carries the y labels.
    let y_ticks: Vec<_> = texts.iter().filter(|t| t.string == "0.4").collect();
    assert_eq!(y_ticks.len(), 3);
    let y_edge = view3d::project(&t, [0., 0.4, 0.]);
    assert!(y_ticks[1].position[1] < y_edge[1] && y_ticks[1].position[0] < y_edge[0]);
    // Exactly three tick labels for each tick value 0 and 1 (x, y, z).
    assert_eq!(texts.iter().filter(|t| t.string == "1.0").count(), 3);
    // z labels sit left of every vertical edge.
    let leftmost = [[0., 0.], [1., 0.], [0., 1.], [1., 1.]]
        .map(|[x, y]| view3d::project(&t, [x, y, 0.5])[0])
        .into_iter()
        .fold(f64::INFINITY, f64::min);
    let z_labels: Vec<_> = texts
        .iter()
        .filter(|t| {
            t.horizontal == matrust::plot::TextAlign::Right
                && t.vertical == matrust::plot::VerticalAlign::Middle
        })
        .collect();
    assert_eq!(z_labels.len(), 6);
    assert!(z_labels.iter().all(|t| t.position[0] < leftmost));
}

#[test]
fn mrfig_round_trip_keeps_3d_state_and_2d_files_are_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("helix.mrfig");
    let f = run(
        "t = 0:0.25:5; plot3(sin(t), cos(t), t, 'r'); hold on; scatter3([0 1], [1 0], [2 3]); view(30, 15); zlim([-1 6]); zlabel('height'); grid on; set(gca, 'ZGrid', 'off');",
    );
    files::save_figure(&path, &f).unwrap();
    let loaded = files::load_figure(&path).unwrap();
    assert_eq!(loaded.view, [30., 15.]);
    assert_eq!(loaded.zlim, Some([-1., 6.]));
    assert_eq!(loaded.zlabel, "height");
    assert!(loaded.xgrid && !loaded.zgrid);
    assert_eq!(loaded.series.len(), 2);
    for (a, b) in f.series.iter().zip(&loaded.series) {
        assert_eq!(&*a.z, &*b.z);
        assert_eq!(
            a.points.iter().collect::<Vec<_>>(),
            b.points.iter().collect::<Vec<_>>()
        );
    }
    assert_eq!(loaded.svg(), f.svg());
    // Subplot containers carry each axes' 3-D state too.
    let f = run("subplot(1,2,1); plot3([0 1], [0 1], [0 1]); subplot(1,2,2); plot([1 2], [3 4]);");
    let path = dir.path().join("subplots.mrfig");
    files::save_figure(&path, &f).unwrap();
    let loaded = files::load_figure(&path).unwrap();
    assert_eq!(loaded.all_axes().filter(|(_, a)| a.is_3d()).count(), 1);
    assert_eq!(loaded.svg(), f.svg());
    // An older native figure still loads.
    let old = files::load_figure(std::path::Path::new("tests/fixtures/ui/signal.mfig")).unwrap();
    assert!(!old.is_3d() && old.series.iter().all(|s| s.z.is_empty()));
    // 2-D figures save the same bytes as before 3-D support: the fixtures were
    // written by the parent commit (tool/refs/plot3d/dump_2d_figures.rs).
    for (name, source, expected) in [
        (
            "plain_2d.mrfig",
            "plot([1 2 3], [4 5 6]); grid on; xlabel('x');",
            include_bytes!("fixtures/plot3d/plain_2d.mrfig").as_slice(),
        ),
        (
            "styled_2d.mrfig",
            "plot([0 1 2], [3 1 2], 'r--o'); hold on; scatter([1 2], [2 3]); legend('a', 'b'); title('t'); xlim([0 3]);",
            include_bytes!("fixtures/plot3d/styled_2d.mrfig").as_slice(),
        ),
        (
            "subplots_2d.mrfig",
            "subplot(1,2,1); plot([1 2], [3 4]); subplot(1,2,2); stem([1 2 3]); grid on;",
            include_bytes!("fixtures/plot3d/subplots_2d.mrfig").as_slice(),
        ),
    ] {
        let path = dir.path().join(name);
        files::save_figure(&path, &run(source)).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), expected, "{name} changed");
        let loaded = files::load_figure(&path).unwrap();
        assert!(
            loaded
                .all_axes()
                .all(|(_, a)| !a.is_3d() && a.series.iter().all(|s| s.z.is_empty()))
        );
    }
}

#[test]
fn drag_maps_pointer_motion_to_azimuth_and_clamped_elevation() {
    assert_eq!(view3d::drag_view([-37.5, 30.], [10., 0.]), [-32.5, 30.]);
    assert_eq!(view3d::drag_view([-37.5, 30.], [0., 20.]), [-37.5, 20.]);
    assert_eq!(view3d::drag_view([0., 80.], [0., -40.]), [0., 90.]);
    assert_eq!(view3d::drag_view([0., -80.], [0., 40.]), [0., -90.]);
    assert_eq!(view3d::drag_view([170., 0.], [40., 0.]), [-170., 0.]);
    assert_eq!(view3d::drag_view([0., 0.], [-360., 0.]), [180., 0.]);
}

/// Finite projected points of a display series, split at NaN breaks.
fn runs(points: impl Iterator<Item = [f64; 2]>) -> Vec<Vec<[f64; 2]>> {
    let mut runs: Vec<Vec<[f64; 2]>> = vec![Vec::new()];
    for point in points {
        if point.iter().any(|v| v.is_nan()) {
            if !runs.last().unwrap().is_empty() {
                runs.push(Vec::new());
            }
        } else {
            runs.last_mut().unwrap().push(point);
        }
    }
    runs.retain(|run| !run.is_empty());
    runs
}
/// Where a point given in unit-box coordinates lands on the screen.
fn screen(view: [f64; 2], unit: [f64; 3]) -> [f64; 2] {
    let [u, v, _] = view3d::project(&view3d::viewmtx(view[0], view[1]), unit);
    [u, v]
}
fn same_point(a: [f64; 2], b: [f64; 2]) -> bool {
    close(a[0], b[0]) && close(a[1], b[1])
}

#[test]
fn lines_are_clipped_to_the_axes_box_before_projection() {
    // The half of the line inside xlim [0 1] stays, ending on the x = 1 face.
    let f = run("plot3([0 2], [0 2], [0 2]); xlim([0 1]); ylim([0 2]); zlim([0 2]);");
    let display = view3d::display_view(&f.axes);
    let kept = runs(display.series[0].points.iter());
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].len(), 2);
    assert!(same_point(kept[0][0], screen(f.view, [0., 0., 0.])));
    assert!(same_point(kept[0][1], screen(f.view, [1., 0.5, 0.5])));
    // A line crossing the whole box is cut at both faces.
    let f = run("plot3([-1 2], [1 1], [1 1]); xlim([0 1]); ylim([0 2]); zlim([0 2]);");
    let display = view3d::display_view(&f.axes);
    let kept = runs(display.series[0].points.iter());
    assert_eq!(kept.len(), 1);
    assert!(same_point(kept[0][0], screen(f.view, [0., 0.5, 0.5])));
    assert!(same_point(kept[0][1], screen(f.view, [1., 0.5, 0.5])));
    // A segment fully outside is dropped; the other segment is kept.
    let f = run("plot3([2 3 0.5], [1 1 1], [1 1 1]); xlim([0 1]); ylim([0 2]); zlim([0 2]);");
    let display = view3d::display_view(&f.axes);
    let kept = runs(display.series[0].points.iter());
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].len(), 2);
    assert!(same_point(kept[0][0], screen(f.view, [1., 0.5, 0.5])));
    assert!(same_point(kept[0][1], screen(f.view, [0.5, 0.5, 0.5])));
    let f = run("plot3([2 3], [1 1], [1 1]); xlim([0 1]); ylim([0 2]); zlim([0 2]);");
    let display = view3d::display_view(&f.axes);
    assert!(runs(display.series[0].points.iter()).is_empty());
    // In, out and back in: two runs, each ending on the face.
    let f = run("plot3([0.5 2 0.5], [1 1 1], [1 1 1]); xlim([0 1]); ylim([0 2]); zlim([0 2]);");
    let display = view3d::display_view(&f.axes);
    let kept = runs(display.series[0].points.iter());
    assert_eq!(kept.len(), 2);
    assert!(same_point(kept[0][0], screen(f.view, [0.5, 0.5, 0.5])));
    assert!(same_point(kept[0][1], screen(f.view, [1., 0.5, 0.5])));
    assert!(same_point(kept[1][0], screen(f.view, [1., 0.5, 0.5])));
    assert!(same_point(kept[1][1], screen(f.view, [0.5, 0.5, 0.5])));
    // A line inside the box is unchanged: no extra points, no breaks.
    let f = run("plot3([0 1 2], [0 1 0], [0 1 2]);");
    let display = view3d::display_view(&f.axes);
    assert_eq!(runs(display.series[0].points.iter())[0].len(), 3);
    assert_eq!(display.series[0].points.len(), 3);
    // A point exactly on a limit counts as inside.
    let f = run("plot3([0 1], [0 1], [0 1]); xlim([0 1]);");
    let display = view3d::display_view(&f.axes);
    assert_eq!(runs(display.series[0].points.iter())[0].len(), 2);
}

#[test]
fn clipped_marker_lines_draw_markers_only_at_data_points() {
    let f = run("plot3([0 2], [0 2], [0 2], '-o'); xlim([0 1]); ylim([0 2]); zlim([0 2]);");
    let display = view3d::display_view(&f.axes);
    // The data series keeps its style (legend sample) and shows the marker of
    // the one sample inside the box; the cut polyline follows, legend-hidden,
    // without markers, so no marker sits on the cut face.
    let carrier = &display.series[0];
    assert!(carrier.markers && !carrier.legend_hidden);
    let marked = runs(carrier.points.iter());
    assert_eq!(marked.len(), 1);
    assert_eq!(marked[0].len(), 1);
    assert!(same_point(marked[0][0], screen(f.view, [0., 0., 0.])));
    let stroke = &display.series[1];
    assert!(!stroke.markers && stroke.legend_hidden && stroke.label.is_empty());
    let kept = runs(stroke.points.iter());
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].len(), 2);
    assert!(same_point(kept[0][1], screen(f.view, [1., 0.5, 0.5])));
    // The paint copy of the desktop lists the same series.
    let frame = view3d::display_frame_in(&f.axes, Default::default());
    assert_eq!(frame.series.len(), display.series.len());
    // scatter3 drops the markers outside the box by position.
    let f = run("scatter3([0.5 2 0.25], [1 1 1], [1 1 1]); xlim([0 1]); ylim([0 2]); zlim([0 2]);");
    let display = view3d::display_view(&f.axes);
    let points: Vec<[f64; 2]> = display.series[0].points.iter().collect();
    assert_eq!(points.len(), 3);
    assert!(points[1].iter().all(|v| v.is_nan()));
    assert!(points[0].iter().chain(&points[2]).all(|v| v.is_finite()));
    assert!(same_point(points[0], screen(f.view, [0.5, 0.5, 0.5])));
}

/// The native figure bytes of `figure` with the first series' metadata edited.
fn mrfig_with_first_series(
    figure: &Figure,
    path: &std::path::Path,
    edit: impl FnOnce(&mut serde_json::Value),
) {
    files::save_figure(path, figure).unwrap();
    let bytes = std::fs::read(path).unwrap();
    let size = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let mut metadata: serde_json::Value = serde_json::from_slice(&bytes[12..12 + size]).unwrap();
    edit(&mut metadata["series"][0]);
    let json = serde_json::to_vec(&metadata).unwrap();
    let mut crafted = bytes[..8].to_vec();
    crafted.extend((json.len() as u32).to_le_bytes());
    crafted.extend(json);
    crafted.extend_from_slice(&bytes[12 + size..]);
    std::fs::write(path, crafted).unwrap();
}

#[test]
fn a_crafted_mrfig_with_z_where_a_series_cannot_carry_it_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("crafted.mrfig");
    // A compact Patch series marked "z": true used to pass the length check
    // and then panic on a slice past the payload.
    let patch = run(
        "x=(1:3).'; y=x+2; a=x-.25; b=x+.25; c=y*0; patch('XData',[a b b a].','YData',[c c y y].','FaceColor',[.2 .4 .6],'EdgeColor',[1 0 1]);",
    );
    assert!(patch.series[0].points.patch().is_some());
    mrfig_with_first_series(&patch, &path, |series| {
        assert!(series.get("coordinates").is_some());
        series["z"] = true.into();
    });
    let error = files::load_figure(&path).unwrap_err().to_string();
    assert!(error.contains("z samples belong only to"), "{error}");
    // Without the flag the same file loads.
    mrfig_with_first_series(&patch, &path, |_| {});
    assert!(files::load_figure(&path).is_ok());
    // Bars, a dense Patch and a Line without z payload fail cleanly too.
    for (source, expect) in [
        ("bar([1 2 3]);", "z samples belong only to"),
        ("fill([0 1 1], [0 0 1], 'r');", "z samples belong only to"),
        ("plot([1 2 3]);", "truncated"),
    ] {
        let figure = run(source);
        mrfig_with_first_series(&figure, &path, |series| series["z"] = true.into());
        let error = files::load_figure(&path).unwrap_err().to_string();
        assert!(error.contains(expect), "{source}: {error}");
    }
}

#[test]
fn view_setters_with_outputs_return_the_new_angles() {
    let mut e = engine("plot3([0 1], [0 1], [0 1]);");
    e.execute("[az, el] = view(2);").unwrap();
    assert_eq!(values(&mut e, "[az el]"), [0., 90.]);
    assert!(!e.figure.as_ref().unwrap().is_3d());
    e.execute("[az, el] = view(3);").unwrap();
    assert_eq!(values(&mut e, "[az el]"), [-37.5, 30.]);
    assert!(e.figure.as_ref().unwrap().is_3d());
    e.execute("[az, el] = view(45, 20);").unwrap();
    assert_eq!(values(&mut e, "[az el]"), [45., 20.]);
    e.execute("[az, el] = view([10 -5]);").unwrap();
    assert_eq!(values(&mut e, "[az el]"), [10., -5.]);
    assert_eq!(values(&mut e, "view"), [10., -5.]);
    e.execute("ax = gca; [az, el] = view(ax, 60, 15);").unwrap();
    assert_eq!(values(&mut e, "[az el]"), [60., 15.]);
    assert_eq!(values(&mut e, "ax.View"), [60., 15.]);
    e.execute("[az, el] = view(ax);").unwrap();
    assert_eq!(values(&mut e, "[az el]"), [60., 15.]);
    // A bad argument still errors.
    assert!(error("[az, el] = view(5);").contains("2 or 3"));
}

#[test]
fn scatter3_rejects_a_line_style_like_scatter() {
    let scatter = error("scatter([1 2], [3 4], 'r-o')");
    assert_eq!(
        scatter,
        "scatter/bar support a single color letter; full line specs require plot"
    );
    for call in [
        "scatter3([1 2], [3 4], [5 6], 'r-o')",
        "scatter3([1 2], [3 4], [5 6], 36, 'r-o')",
        "scatter3([1 2], [3 4], [5 6], 'filled', '--')",
        "scatter3([1 2], [3 4], [5 6], 36, 'r', 'ro')",
    ] {
        assert_eq!(error(call), scatter, "{call}");
    }
    // A colour letter, a marker and Name,Value pairs are still accepted.
    let f = run("scatter3([1 2], [3 4], [5 6], 36, 'r', 'filled', 's', 'LineWidth', 2);");
    assert_eq!(f.series[0].color, [255, 0, 0]);
}

#[test]
fn a_2d_figure_with_only_the_z_grid_off_saves_the_same_bytes() {
    // Review rev3 F3: zgrid is written only for 3-D axes, so 2-D saves stay
    // byte-identical and older builds (deny_unknown_fields) still read them.
    let dir = tempfile::tempdir().unwrap();
    let plain = run("plot([1 2 3], [4 5 6]); grid on;");
    let z_off = run("plot([1 2 3], [4 5 6]); grid on; set(gca, 'ZGrid', 'off');");
    assert!(plain.zgrid && !z_off.zgrid);
    let (a, b) = (dir.path().join("a.mrfig"), dir.path().join("b.mrfig"));
    files::save_figure(&a, &plain).unwrap();
    files::save_figure(&b, &z_off).unwrap();
    assert_eq!(std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
}
