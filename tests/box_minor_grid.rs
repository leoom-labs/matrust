//! `box` (axes Box outline) and minor grids (`grid minor`, X/Y/ZMinorGrid).
//! Expected positions follow the definition used by Octave's tick code: four
//! minor ticks per linear major interval, 2..9 x each decade on log axes.
use matrust::plot::{AxisScale, AxisTransform, LinePattern, minor_ticks};
use matrust::{Array, Engine, files};
use std::sync::atomic::AtomicBool;

fn text(e: &Engine, name: &str) -> String {
    let Array::Character(c) = e.array(name).unwrap() else {
        panic!("{name} is not text")
    };
    c.to_utf8(&AtomicBool::new(false)).unwrap()
}
fn run(script: &str) -> Engine {
    let mut e = Engine::default();
    e.execute(script).unwrap();
    e
}
fn minor(e: &Engine) -> [bool; 3] {
    e.figure.as_ref().unwrap().minor_grid
}
fn major(e: &Engine) -> [bool; 3] {
    let f = e.figure.as_ref().unwrap();
    [f.xgrid, f.ygrid, f.zgrid]
}
/// Dotted minor grid paths in an SVG: (vertical, horizontal).
fn minor_paths(svg: &str) -> (usize, usize) {
    let mut counts = (0, 0);
    for part in svg.split("<path d=\"").skip(1) {
        let tag = part.split('>').next().unwrap();
        if tag.contains("stroke-dasharray=\"1 2\"") && tag.contains("stroke-width=\"0.5\"") {
            let d = tag.split('"').next().unwrap();
            counts.0 += usize::from(d.contains('V'));
            counts.1 += usize::from(d.contains('H'));
        }
    }
    counts
}
/// The element that draws the 2-D axes frame (just before the clipped data group).
fn frame(svg: &str) -> &str {
    let end = svg.find("<g clip-path").unwrap();
    let start = svg[..end].rfind("</g>").unwrap() + 4;
    &svg[start..end]
}
fn close(actual: &[f64], expected: &[f64]) {
    assert_eq!(actual.len(), expected.len(), "{actual:?} vs {expected:?}");
    for (a, b) in actual.iter().zip(expected) {
        assert!((a - b).abs() < 1e-12, "{actual:?} vs {expected:?}");
    }
}

#[test]
fn box_get_set_toggle_and_command_and_function_forms() {
    let mut e = run("plot(1:3); a = gca; b0 = get(a,'Box');");
    // A plot axes keeps the historical full outline (MATLAB's plot sets Box on).
    assert_eq!(text(&e, "b0"), "on");
    for (script, expected) in [
        ("box off", "off"),
        ("box", "on"),
        ("box", "off"),
        ("box on", "on"),
        ("box('off')", "off"),
        ("box(a,'on')", "on"),
        ("set(a,'Box','off')", "off"),
        ("set(a,'Box',1)", "on"),
        ("set(a,'Box',false)", "off"),
        ("box ON", "on"),
    ] {
        e.execute(&format!("{script}; b = get(a,'Box');")).unwrap();
        assert_eq!(text(&e, "b"), expected, "{script}");
        assert_eq!(e.figure.as_ref().unwrap().box_outline, expected == "on");
    }
    // A bare `box` statement without a semicolon toggles too.
    e.execute("box").unwrap();
    assert!(!e.figure.as_ref().unwrap().box_outline);
    e.execute("box").unwrap();
    assert!(e.figure.as_ref().unwrap().box_outline);
    let before = e.figure.clone().unwrap();
    for bad in [
        "box maybe",
        "box('on','off')",
        "set(a,'Box','maybe')",
        "box(a, 2)",
    ] {
        assert!(e.execute(bad).is_err(), "{bad}");
        assert!(before.same_snapshot(e.figure.as_ref().unwrap()), "{bad}");
    }
    // A handle target changes only that axes.
    let e = run(
        "subplot(2,1,1); plot(1:3); top = gca; subplot(2,1,2); plot(1:3); bottom = gca; \
         box(top,'off'); t = get(top,'Box'); u = get(bottom,'Box');",
    );
    assert_eq!((text(&e, "t"), text(&e, "u")), ("off".into(), "on".into()));
}

#[test]
fn minor_grid_properties_grid_minor_toggle_and_grid_off_clears_minor() {
    let mut e = run("plot(1:10); a = gca; x = get(a,'XMinorGrid'); z = get(a,'ZMinorGrid');");
    assert_eq!((text(&e, "x"), text(&e, "z")), ("off".into(), "off".into()));
    // `grid minor` toggles the minor grid only; the major grid stays as it was.
    e.execute("grid minor").unwrap();
    assert_eq!((minor(&e), major(&e)), ([true; 3], [false; 3]));
    e.execute("grid minor;").unwrap();
    assert_eq!((minor(&e), major(&e)), ([false; 3], [false; 3]));
    e.execute("grid on; grid('minor');").unwrap();
    assert_eq!((minor(&e), major(&e)), ([true; 3], [true; 3]));
    // `grid on` leaves the minor grid alone; `grid off` removes all grid lines.
    e.execute("grid on;").unwrap();
    assert_eq!(minor(&e), [true; 3]);
    e.execute("grid off;").unwrap();
    assert_eq!((minor(&e), major(&e)), ([false; 3], [false; 3]));
    e.execute("set(a,'XMinorGrid','on','YMinorGrid','off','ZMinorGrid',true);")
        .unwrap();
    assert_eq!(minor(&e), [true, false, true]);
    e.execute("grid(a,'minor'); x = get(a,'XMinorGrid'); y = get(a,'YMinorGrid');")
        .unwrap();
    assert_eq!(minor(&e), [false, true, false]);
    assert_eq!((text(&e, "x"), text(&e, "y")), ("off".into(), "on".into()));
    let before = e.figure.clone().unwrap();
    for bad in ["grid sideways", "set(a,'XMinorGrid','maybe')"] {
        assert!(e.execute(bad).is_err(), "{bad}");
        assert!(before.same_snapshot(e.figure.as_ref().unwrap()), "{bad}");
    }
}

#[test]
fn svg_box_off_draws_left_and_bottom_lines_and_default_output_is_unchanged() {
    let e = run("plot([0 10],[0 1]);");
    let original = e.figure.as_ref().unwrap().svg();
    assert!(
        frame(&original).starts_with("<rect"),
        "{}",
        frame(&original)
    );
    let mut e = e;
    e.execute("box off;").unwrap();
    let open = e.figure.as_ref().unwrap().svg();
    let outline = frame(&open);
    // One polyline: top-left down to bottom-left, then along the bottom.
    assert!(outline.starts_with("<path d=\"M"), "{outline}");
    let d = outline.split('"').nth(1).unwrap();
    assert_eq!(
        (d.matches('V').count(), d.matches('H').count()),
        (1, 1),
        "{d}"
    );
    // Turning the box back on restores the historical bytes exactly.
    e.execute("box on;").unwrap();
    assert_eq!(e.figure.as_ref().unwrap().svg(), original);
    // Explicit ruler colours: box on draws two x and two y edges, box off one each.
    e.execute("set(gca,'XColor',[1 0 0],'YColor',[0 0 1]);")
        .unwrap();
    for (state, edges) in [("on", 2), ("off", 1)] {
        e.execute(&format!("box {state};")).unwrap();
        let svg = e.figure.as_ref().unwrap().svg();
        let lines = frame(&svg);
        let x = lines.split("stroke=\"#ff0000\"").next().unwrap();
        assert_eq!(x.matches('H').count(), edges, "{state}: {lines}");
        let y = lines.split("<path d=\"").nth(2).unwrap();
        assert_eq!(
            y.split('"').next().unwrap().matches('V').count(),
            edges,
            "{state}: {lines}"
        );
    }
}

#[test]
fn svg_minor_grid_draws_dotted_thin_lines_at_four_per_major_interval() {
    // xlim [0 10]: majors every 2 (6 ticks), minor every 0.4 -> 26 - 6 = 20.
    // ylim [0 1]: majors every 0.2 (6 ticks), minor every 0.04 -> 20.
    let mut e = run("plot([0 10],[0 1]); xlim([0 10]); ylim([0 1]);");
    assert_eq!(minor_paths(&e.figure.as_ref().unwrap().svg()), (0, 0));
    e.execute("set(gca,'XMinorGrid','on');").unwrap();
    assert_eq!(minor_paths(&e.figure.as_ref().unwrap().svg()), (20, 0));
    e.execute("set(gca,'XMinorGrid','off','YMinorGrid','on');")
        .unwrap();
    assert_eq!(minor_paths(&e.figure.as_ref().unwrap().svg()), (0, 20));
    e.execute("grid on; set(gca,'XMinorGrid','on');").unwrap();
    let svg = e.figure.as_ref().unwrap().svg();
    assert_eq!(minor_paths(&svg), (20, 20));
    // Major lines stay solid and full width.
    let solid = svg
        .split("<path d=\"")
        .skip(1)
        .map(|p| p.split('>').next().unwrap())
        .filter(|t| t.contains("stroke=\"#262626\"") && !t.contains("dasharray"))
        .count();
    assert_eq!(solid, 12);
    e.execute("grid off;").unwrap();
    assert_eq!(minor_paths(&e.figure.as_ref().unwrap().svg()), (0, 0));
}

#[test]
fn log_axes_put_minor_lines_at_two_to_nine_times_each_decade() {
    // Values 1..100: 2..9 and 20..90.
    let expected: Vec<f64> = [
        2., 3., 4., 5., 6., 7., 8., 9., 20., 30., 40., 50., 60., 70., 80., 90.,
    ]
    .iter()
    .map(|v: &f64| v.log10())
    .collect();
    let log = AxisTransform {
        scale: AxisScale::Log,
        negative: false,
    };
    close(&minor_ticks(log, [0., 2.], 0.), &expected);
    // Negative-only data: display is -log10|v|, still ascending.
    let negative = AxisTransform {
        scale: AxisScale::Log,
        negative: true,
    };
    let mut mirrored: Vec<f64> = expected.iter().map(|v| -v).collect();
    mirrored.reverse();
    close(&minor_ticks(negative, [-2., 0.], 0.), &mirrored);
    // Sixteen decades: majors every 2 decades, the odd decades are minor.
    close(
        &minor_ticks(log, [0., 16.], 0.),
        &[1., 3., 5., 7., 9., 11., 13., 15.],
    );
    // Linear: every step/5, majors excluded, continued to both ends.
    let linear = AxisTransform::default();
    let values: Vec<f64> = (0..=25)
        .filter(|i| i % 5 != 0)
        .map(|i| f64::from(i) * 0.04)
        .collect();
    close(&minor_ticks(linear, [0., 1.], 0.2), &values);
    close(
        &minor_ticks(linear, [-0.1, 0.1], 0.2),
        &[-0.08, -0.04, 0.04, 0.08],
    );
    assert!(
        minor_ticks(linear, [0., 1.], 1e-9).is_empty(),
        "denser than the bound"
    );
    assert!(minor_ticks(linear, [1., 0.], 0.2).is_empty());
    // The SVG draws those sixteen positions on a loglog axes.
    let e = run("loglog([1 100],[1 100]); xlim([1 100]); ylim([1 100]); grid minor;");
    assert_eq!(minor_paths(&e.figure.as_ref().unwrap().svg()), (16, 16));
}

#[test]
fn three_d_box_off_drops_the_back_plane_outline_and_minor_grid_is_dotted() {
    let points = |e: &Engine| -> usize {
        let display = matrust::plot::view3d::display_view(e.figure.as_ref().unwrap());
        display.series.iter().map(|s| s.points.len()).sum()
    };
    let mut e = run("plot3([0 1],[0 1],[0 1]);");
    let boxed = points(&e);
    e.execute("box off;").unwrap();
    // Six back-plane outer edges, three points each (start, end, NaN break).
    assert_eq!(boxed - points(&e), 18);
    e.execute("box on; set(gca,'ZMinorGrid','on');").unwrap();
    let display = matrust::plot::view3d::display_view(e.figure.as_ref().unwrap()).into_owned();
    let dotted: Vec<_> = display
        .series
        .iter()
        .filter(|s| s.legend_hidden && s.pattern == LinePattern::Dotted)
        .collect();
    assert_eq!(dotted.len(), 1);
    assert_eq!(dotted[0].line_width, Some(0.5));
    assert_eq!(
        display.minor_grid, [false; 3],
        "the display copy draws no 2-D grid"
    );
}

#[test]
fn native_fig_and_plot_script_round_trip_box_and_minor_grids() {
    let directory = tempfile::tempdir().unwrap();
    let e = run("plot([1 2],[3 4]); box off; set(gca,'XMinorGrid','on');");
    let f = e.figure.clone().unwrap();
    for extension in ["mfig", "fig"] {
        let path = directory.path().join(format!("boxed.{extension}"));
        files::save_figure(&path, &f).unwrap();
        let loaded = files::load_figure(&path).unwrap();
        assert!(!loaded.box_outline, "{extension}");
        assert_eq!(loaded.minor_grid, [true, false, false], "{extension}");
    }
    // Defaults add nothing to a native file.
    let plain = run("plot([1 2],[3 4]);").figure.unwrap();
    let path = directory.path().join("plain.mfig");
    files::save_figure(&path, &plain).unwrap();
    let bytes = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned();
    assert!(!bytes.contains("\"box\"") && !bytes.contains("minor_grid"));
    let loaded = files::load_figure(&path).unwrap();
    assert!(loaded.box_outline && loaded.minor_grid == [false; 3]);
    // The MATLAB script sets the same state and replays.
    let script = files::matlab_plot_script(&f, &AtomicBool::new(false)).unwrap();
    assert!(script.contains("'Box','off'"), "{script}");
    assert!(
        script.contains("'XMinorGrid','on','YMinorGrid','off'"),
        "{script}"
    );
    let replay = run(&script);
    let replayed = replay.figure.as_ref().unwrap();
    assert!(!replayed.box_outline);
    assert_eq!(&replayed.minor_grid[..2], &[true, false]);
    let plain_script = files::matlab_plot_script(&plain, &AtomicBool::new(false)).unwrap();
    assert!(plain_script.contains("'Box','on'") && !plain_script.contains("MinorGrid"));
}

/// Layer 'top' moves the same major/minor grid and outward tick elements
/// after the clipped data group; nothing is duplicated, dropped or clipped.
#[test]
fn svg_layer_top_moves_major_minor_grid_and_ticks_after_the_data() {
    let script = "plot(1:10,'LineWidth',3); grid on; grid minor; set(gca,'TickDir','out');";
    let bottom = run(script).figure.unwrap().svg();
    let top = run(&format!("{script} set(gca,'Layer','top');"))
        .figure
        .unwrap()
        .svg();
    let elements = |svg: &str| {
        let mut parts: Vec<String> = svg
            .split('<')
            .filter(|p| !p.starts_with("g data-matrust-layer") && !p.starts_with("/g"))
            .map(str::to_owned)
            .collect();
        parts.sort();
        parts
    };
    assert_eq!(elements(&bottom), elements(&top));
    assert!(!bottom.contains("data-matrust-layer"));
    let (minor_v, minor_h) = minor_paths(&top);
    assert!(minor_v > 0 && minor_h > 0);
    // Bottom: grid and ticks precede the series; top: they follow it.
    let series = |svg: &str| svg.find("stroke-width=\"4\"").unwrap();
    let layer = top.find("<g data-matrust-layer=\"top\">").unwrap();
    assert!(series(&top) < layer);
    let group = &top[layer..];
    let group = &group[..group.find("</g>").unwrap()];
    assert_eq!(minor_paths(group), (minor_v, minor_h));
    assert_eq!(minor_paths(&bottom[..series(&bottom)]), (minor_v, minor_h));
    // The outward tick path moves out of the frame element, unclipped.
    let tick_element = "fill=\"none\" stroke=\"#777\"/>";
    assert!(frame(&bottom).contains(tick_element), "{}", frame(&bottom));
    assert!(!frame(&top).contains("<path"), "{}", frame(&top));
    assert!(group.contains(tick_element), "{group}");
    // The layer group directly follows the closed clip group.
    assert!(top[..layer].ends_with("</g>"));
    // Without grid or ticks, top paints exactly the default document.
    assert_eq!(
        run("plot(1:3);").figure.unwrap().svg(),
        run("plot(1:3); set(gca,'Layer','top');")
            .figure
            .unwrap()
            .svg()
    );
}
