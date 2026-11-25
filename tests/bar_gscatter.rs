#[cfg(feature = "plot-export")]
use matrust::export;
use matrust::{Array, Engine, files};

#[cfg(feature = "plot-export")]
fn render_svg_pixels(svg: &str) -> resvg::tiny_skia::Pixmap {
    let tree = resvg::usvg::Tree::from_str(svg, &export::svg_render_options()).unwrap();
    let w = tree.size().width() as u32;
    let h = tree.size().height() as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h).unwrap();
    pixmap.fill(resvg::tiny_skia::Color::WHITE);
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    pixmap
}

#[cfg(feature = "plot-export")]
fn pixel_rgb(pixmap: &resvg::tiny_skia::Pixmap, x: u32, y: u32) -> [u8; 3] {
    let p = pixmap.pixel(x, y).unwrap();
    [p.red(), p.green(), p.blue()]
}

fn scalar_val(a: &Array) -> f64 {
    match a {
        Array::Numeric(m) => m.as_scalar().unwrap(),
        _ => panic!("expected numeric scalar, got {a:?}"),
    }
}

fn text_val(a: &Array) -> String {
    match a {
        Array::Character(c) => String::from_utf16_lossy(c.units()),
        Array::Str(s) => s.data()[0].as_deref().unwrap().to_owned(),
        _ => panic!("expected text, got {a:?}"),
    }
}

#[test]
fn bar_time_axes_accept_datetime_and_duration() {
    for (expression, ruler_class) in [
        ("bar(datetime(2024,1,1:3),[1 2 3])", "datetime"),
        ("bar(hours(1:3),[1 2 3])", "duration"),
    ] {
        let mut e = Engine::default();
        e.execute(&format!("{expression}; c = class(get(gca, 'XTick'));"))
            .unwrap();
        assert_eq!(text_val(&e.workspace()["c"]), ruler_class);
        assert_eq!(e.figure.as_ref().unwrap().axes.series[0].points.len(), 3);
    }
}

#[test]
fn bar_gscatter_stacked_and_grouped_handles_and_properties() {
    let mut e = Engine::default();
    e.execute(
        "b = bar([1 2; 3 4], 'stacked');\n\
         n = numel(b);\n\
         l1 = b(1).BarLayout;\n\
         l2 = b(2).BarLayout;\n\
         w1 = b(1).BarWidth;\n\
         b(1).FaceColor = 'flat';\n\
         b(1).CData(2,:) = [1 0 0];\n\
         fc = b(1).FaceColor;\n\
         cd = b(1).CData;\n\
         b(1).FaceAlpha = 0.5;\n\
         fa = b(1).FaceAlpha;\n\
         xd = b(1).XData;\n\
         yd = b(1).YData;\n\
         legend('First', 'Second');",
    )
    .unwrap();
    let ws = e.workspace();
    assert_eq!(scalar_val(&ws["n"]), 2.0);
    assert_eq!(text_val(&ws["l1"]), "stacked");
    assert_eq!(text_val(&ws["l2"]), "stacked");
    assert_eq!(scalar_val(&ws["w1"]), 0.8);
    assert_eq!(text_val(&ws["fc"]), "flat");
    assert_eq!(scalar_val(&ws["fa"]), 0.5);

    let Array::Numeric(cd) = &ws["cd"] else {
        panic!("expected numeric CData");
    };
    assert_eq!((cd.rows(), cd.cols()), (2, 3));
    assert!((cd.get(1, 0) - 1.0).abs() < 1e-6);
    assert!(cd.get(1, 1).abs() < 1e-6);
    assert!(cd.get(1, 2).abs() < 1e-6);
    // Row 1 keeps default series colour (not red).
    assert!(cd.get(0, 0) < 0.5);

    let Array::Numeric(xd) = &ws["xd"] else {
        panic!("expected numeric XData");
    };
    assert_eq!(xd.data(), &[1.0, 2.0]);
    let Array::Numeric(yd) = &ws["yd"] else {
        panic!("expected numeric YData");
    };
    assert_eq!(yd.data(), &[1.0, 3.0]);

    let fig = e.figure.as_ref().unwrap();
    assert!(fig.axes.legend);
    assert_eq!(fig.axes.series.len(), 2);
    assert_eq!(fig.axes.series[0].label, "First");
    assert_eq!(fig.axes.series[1].label, "Second");
    assert_eq!(fig.axes.series[0].bar_offset, 0.0);
    assert_eq!(fig.axes.series[1].bar_offset, 0.0);

    // Save and reload .mfig figure.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("stacked_bar.mfig");
    files::save_figure(&path, fig).unwrap();
    let loaded = files::load_figure(&path).unwrap();
    assert!(fig.axes.same_metadata(&loaded.axes));
    assert_eq!(fig.axes.series.len(), loaded.axes.series.len());
    for (a, b) in fig.axes.series.iter().zip(&loaded.axes.series) {
        assert_eq!(a.points, b.points);
    }

    // Switching BarLayout back to grouped re-lays the group.
    e.execute("b(1).BarLayout = 'grouped'; lg1 = b(1).BarLayout; lg2 = b(2).BarLayout;")
        .unwrap();
    let ws = e.workspace();
    assert_eq!(text_val(&ws["lg1"]), "grouped");
    assert_eq!(text_val(&ws["lg2"]), "grouped");
    let fig_grouped = e.figure.as_ref().unwrap();
    assert!(fig_grouped.axes.series[0].bar_offset < 0.0);
    assert!(fig_grouped.axes.series[1].bar_offset > 0.0);

    // Positional width and x, Y forms.
    e.execute("bar([10 20], [1 2; 3 4], 0.5, 'stacked');")
        .unwrap();
    let fig2 = e.figure.as_ref().unwrap();
    assert_eq!(fig2.axes.series[0].bar_relative_width, Some(0.5));
    assert!((fig2.axes.series[0].bar_width - 5.0).abs() < 1e-12);
}

#[cfg(feature = "plot-export")]
#[test]
fn bar_gscatter_rendered_flat_cdata_and_stacked_pixels() {
    // 1. FaceColor 'flat' with CData row 2 red -> bar 2 interior red, bar 1 default colour.
    let mut e = Engine::default();
    e.execute("b = bar([2 4]); b.FaceColor = 'flat'; b.CData(2,:) = [1 0 0];")
        .unwrap();
    let fig = e.figure.as_ref().unwrap();
    let svg = fig.svg();
    let pixmap = render_svg_pixels(&svg);
    let bounds = fig.axes.bounds();
    let (x_lim, y_lim) = (bounds.0, bounds.1);
    // Map data coordinates to SVG pixel coordinates (plot box is [70, 70, 920, 500]).
    let to_px = |dx: f64, dy: f64| -> (u32, u32) {
        let px = 70.0 + (dx - x_lim[0]) / (x_lim[1] - x_lim[0]) * 850.0;
        let py = 500.0 - (dy - y_lim[0]) / (y_lim[1] - y_lim[0]) * 430.0;
        (px.round() as u32, py.round() as u32)
    };
    let (x1, y1) = to_px(1.0, 1.0);
    let (x2, y2) = to_px(2.0, 2.0);
    let c1 = pixel_rgb(&pixmap, x1, y1);
    let c2 = pixel_rgb(&pixmap, x2, y2);
    assert_eq!(c2, [255, 0, 0], "bar 2 interior should be red");
    assert_ne!(c1, [255, 0, 0], "bar 1 interior should not be red");
    assert_ne!(c1, [255, 255, 255], "bar 1 interior should be filled");

    // 2. Stacked: top segment directly above bottom segment at the same x.
    let mut e2 = Engine::default();
    e2.execute("bar([1 2; 3 4], 'stacked');").unwrap();
    let fig2 = e2.figure.as_ref().unwrap();
    let bounds2 = fig2.axes.bounds();
    assert!(
        bounds2.1[1] >= 7.0,
        "stacked ylim upper bound should cover 3+4=7, got {:?}",
        bounds2.1
    );
    let svg2 = fig2.svg();
    let pixmap2 = render_svg_pixels(&svg2);
    let (x_lim2, y_lim2) = (bounds2.0, bounds2.1);
    let to_px2 = |dx: f64, dy: f64| -> (u32, u32) {
        let px = 70.0 + (dx - x_lim2[0]) / (x_lim2[1] - x_lim2[0]) * 850.0;
        let py = 500.0 - (dy - y_lim2[0]) / (y_lim2[1] - y_lim2[0]) * 430.0;
        (px.round() as u32, py.round() as u32)
    };
    // At x = 1: bottom segment is y in [0, 1], top segment is y in [1, 3].
    let (bx, by) = to_px2(1.0, 0.5);
    let (tx, ty) = to_px2(1.0, 2.0);
    let bottom_rgb = pixel_rgb(&pixmap2, bx, by);
    let top_rgb = pixel_rgb(&pixmap2, tx, ty);
    assert_ne!(bottom_rgb, [255, 255, 255]);
    assert_ne!(top_rgb, [255, 255, 255]);
    assert_ne!(
        bottom_rgb, top_rgb,
        "top and bottom segments have distinct series colours"
    );
}

#[test]
fn bar_gscatter_grouped_scatter_and_legend() {
    let mut e = Engine::default();
    e.execute(
        "h = gscatter(1:4, 1:4, [1 1 2 2]);\n\
         nh = numel(h);\n\
         t1 = h(1).Type;\n\
         ls1 = h(1).LineStyle;\n\
         mk1 = h(1).Marker;",
    )
    .unwrap();
    let ws = e.workspace();
    assert_eq!(scalar_val(&ws["nh"]), 2.0);
    assert_eq!(text_val(&ws["t1"]), "line");
    assert_eq!(text_val(&ws["ls1"]), "none");
    assert_ne!(text_val(&ws["mk1"]), "none");
    let fig = e.figure.as_ref().unwrap();
    assert!(fig.axes.legend);
    assert_eq!(fig.axes.series.len(), 2);
    assert_eq!(fig.axes.series[0].label, "1");
    assert_eq!(fig.axes.series[1].label, "2");

    // Cellstr, categorical, string grouping variables.
    e.execute("gscatter(1:3, [2 4 1], {'b', 'a', 'b'});")
        .unwrap();
    let fig_cell = e.figure.as_ref().unwrap();
    assert_eq!(fig_cell.axes.series.len(), 2);
    assert_eq!(fig_cell.axes.series[0].label, "a");
    assert_eq!(fig_cell.axes.series[1].label, "b");

    e.execute("gscatter(1:3, [2 4 1], categorical({'low', 'high', 'low'}));")
        .unwrap();
    let fig_cat = e.figure.as_ref().unwrap();
    assert_eq!(fig_cat.axes.series.len(), 2);
    assert_eq!(fig_cat.axes.series[0].label, "high");
    assert_eq!(fig_cat.axes.series[1].label, "low");

    e.execute("gscatter(1:3, [2 4 1], [\"x\", \"y\", \"x\"]);")
        .unwrap();
    let fig_str = e.figure.as_ref().unwrap();
    assert_eq!(fig_str.axes.series.len(), 2);
    assert_eq!(fig_str.axes.series[0].label, "x");
    assert_eq!(fig_str.axes.series[1].label, "y");

    // clr, sym, siz and 'doleg', 'off'.
    e.execute("h = gscatter(1:4, 1:4, [1 1 2 2], 'rb', 'o+', [8 12], 'doleg', 'off');")
        .unwrap();
    let fig_off = e.figure.as_ref().unwrap();
    assert!(!fig_off.axes.legend);
    assert_eq!(fig_off.axes.series[0].color, [255, 0, 0]);
    assert_eq!(fig_off.axes.series[1].color, [0, 0, 255]);
    assert_eq!(fig_off.axes.series[0].marker_size, Some(8.0));
    assert_eq!(fig_off.axes.series[1].marker_size, Some(12.0));
}

#[test]
fn bar_gscatter_review_regressions_and_edge_cases() {
    use std::sync::atomic::AtomicBool;

    // F25: single-series bar width does not get an extra 0.8 factor.
    let mut e = Engine::default();
    e.execute("b = bar(1:3, 0.5);").unwrap();
    let fig = e.figure.as_ref().unwrap();
    assert!((fig.axes.series[0].bar_width - 0.5).abs() < 1e-12);

    // F9: BarLayout changes honour GroupWidth.
    e.execute(
        "b = bar([1 3], [2 3; 4 5], 'GroupWidth', 0.5);\n\
         b(1).BarLayout = 'stacked';\n\
         b(1).BarLayout = 'grouped';",
    )
    .unwrap();
    let fig = e.figure.as_ref().unwrap();
    assert!((fig.axes.series[0].bar_offset - (-0.25)).abs() < 1e-12);
    assert!((fig.axes.series[1].bar_offset - 0.25).abs() < 1e-12);
    assert!((fig.axes.series[0].bar_width - 0.4).abs() < 1e-12);

    // F10: separate bar calls under hold on do not stack or relayout together.
    e.execute(
        "b1 = bar([1 2], 'stacked');\n\
         hold on;\n\
         b2 = bar([3 4], 'stacked');",
    )
    .unwrap();
    let fig = e.figure.as_ref().unwrap();
    assert_eq!(fig.axes.series.len(), 2);
    let span1 = fig
        .axes
        .bar_y_span(0, 0, fig.axes.axis_transforms()[1])
        .unwrap()
        .unwrap();
    let span2 = fig
        .axes
        .bar_y_span(1, 0, fig.axes.axis_transforms()[1])
        .unwrap()
        .unwrap();
    assert!((span1[0] - 0.0).abs() < 1e-12 && (span1[1] - 1.0).abs() < 1e-12);
    assert!((span2[0] - 0.0).abs() < 1e-12 && (span2[1] - 3.0).abs() < 1e-12);
    e.execute("b1(1).BarLayout = 'grouped'; l1 = b1(1).BarLayout; l2 = b2(1).BarLayout;")
        .unwrap();
    let ws = e.workspace();
    assert_eq!(text_val(&ws["l1"]), "grouped");
    assert_eq!(text_val(&ws["l2"]), "stacked");

    // Review item 9: scalar x with row vector Y in stacked bar.
    let mut e = Engine::default();
    e.execute("bs = bar(2020, [30 50 23], 'stacked'); ns = numel(bs);")
        .unwrap();
    assert_eq!(scalar_val(&e.workspace()["ns"]), 3.0);
    let fig = e.figure.as_ref().unwrap();
    assert_eq!(fig.axes.series[0].points.get(0), Some([2020.0, 30.0]));
    assert_eq!(fig.axes.series[2].points.get(0), Some([2020.0, 23.0]));

    // Review item 10: setting BarWidth on one member updates the whole stack/group.
    e.execute("b = bar([1 2; 3 4], 'stacked'); b(1).BarWidth = 0.4; w2 = b(2).BarWidth;")
        .unwrap();
    assert!((scalar_val(&e.workspace()["w2"]) - 0.4).abs() < 1e-12);

    // Review item 13: combined XData, YData, CData transaction.
    e.execute(
        "b = bar([1 2]);\n\
         set(b, 'XData', [1 2 3], 'YData', [4 5 6], 'CData', [1 0 0; 0 1 0; 0 0 1]);",
    )
    .unwrap();
    assert_eq!(e.figure.as_ref().unwrap().axes.series[0].points.len(), 3);

    // F21: stacked + styled bar script export does not fall back to patch and round-trips.
    e.execute(
        "b = bar([1 2; 3 4], 'stacked');\n\
         b(1).FaceColor = 'flat';\n\
         b(1).CData(2,:) = [1 0 0];\n\
         b(1).FaceAlpha = 0.5;\n\
         b(2).FaceColor = 'none';",
    )
    .unwrap();
    let src_fig = e.figure.clone().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let script_path = dir.path().join("stacked_styled.m");
    files::write_matlab_plot_script_with_cancel(&script_path, &src_fig, &AtomicBool::new(false))
        .unwrap();
    let doc = matrust::document::Document::open(&script_path).unwrap();
    assert!(doc.text.contains("mr_bars = bar("), "{}", doc.text);
    assert!(!doc.text.contains("patch("), "{}", doc.text);
    let mut replay = Engine::default();
    replay.execute_document(&doc.text, &script_path).unwrap();
    let rep_fig = replay.figure.unwrap();
    assert!(src_fig.axes.same_metadata(&rep_fig.axes));

    // F18: zero-column grouping input errors without panicking.
    assert!(e.execute("gscatter(1:3, 1:3, zeros(3, 0));").is_err());

    // Review item 7: gscatter on yyaxis right preserves left-side series.
    e.execute(
        "yyaxis left; plot(1:3, 1:3);\n\
         yyaxis right; gscatter(1:3, 10:12, [1 1 2]);",
    )
    .unwrap();
    let yy_fig = e.figure.as_ref().unwrap();
    assert_eq!(yy_fig.axes.series.len(), 3);
    assert!(yy_fig.axes.extras.yy.as_ref().unwrap().active_right);
}
