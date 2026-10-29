use matrust::Engine;

#[test]
fn ordinary_svg_matches_preserved_795_bytes_with_explicit_axes_font_size() {
    let mut engine = Engine::default();
    engine
        .execute(include_str!("fixtures/svg_ordinary_795.m"))
        .unwrap();
    // This preserved scene predates the grids-off default. Reproduce its
    // explicit historical grid setting without changing either fixture.
    engine.execute("grid on;").unwrap();
    // This exact-byte scene also predates automatic figure theme intent.
    engine.figure.as_mut().unwrap().appearance = None;
    {
        let historical = engine.figure.as_mut().unwrap();
        historical.axes.palette_cycle = None; // Empty layout axes are not in all_axes_mut().
        for axes in historical.all_axes_mut() {
            axes.palette_cycle = None;
            for series in &mut axes.series {
                series.auto_palette = None;
            }
        }
    }
    // Preserve the historical fixture. Only its one axes-group default changes;
    // every other byte, including title/tick sizes and geometry, must still match.
    let historical = include_str!("fixtures/svg_ordinary_795.svg");
    let old_group = r##"<g font-family="Arial" fill="#242424">"##;
    let new_group = r##"<g font-family="Arial" font-size="16" fill="#242424">"##;
    assert_eq!(historical.matches(old_group).count(), 1);
    assert!(!historical.contains(new_group));
    let expected = historical.replacen(old_group, new_group, 1);
    assert_eq!(engine.figure.unwrap().svg(), expected);
}

#[test]
fn hazardous_lines_preserve_visible_samples_and_do_not_invent_edge_markers() {
    let mut engine = Engine::default();
    engine
        .execute("plot([-1e12 0 1 1e12],[1e12 0 1 -1e12],'r-o');axis([0 1 0 1]);grid off;")
        .unwrap();
    let figure = engine.figure.unwrap();
    let points = figure.series[0].points.clone();
    let svg = figure.svg();
    assert_eq!(svg.matches("<circle").count(), 2);
    assert!(svg.contains("cx=\"70.00\" cy=\"490.00\""));
    assert!(svg.contains("cx=\"920.00\" cy=\"60.00\""));
    assert!(!svg.contains("849999930") && !svg.contains("850000070"));
    assert!((points).same_storage(&figure.series[0].points));
    assert!(
        points
            .iter()
            .eq([[-1e12, 1e12], [0., 0.], [1., 1.], [1e12, -1e12]])
    );
}

#[test]
fn hazardous_none_style_keeps_only_original_visible_markers() {
    let mut engine = Engine::default();
    engine.execute("plot([-1e12 0.5 1e12],[0.5 0.5 0.5],'ro','LineStyle','none');axis([0 1 0 1]);grid off;").unwrap();
    let svg = engine.figure.unwrap().svg();
    assert_eq!(svg.matches("<circle").count(), 1);
    assert!(!svg.contains("stroke-dasharray"));
}

#[cfg(feature = "plot-export")]
#[test]
fn original_huge_coordinate_white_line_is_visible_in_transparent_svg_and_pdf_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = Engine::default();
    engine.set_file_directory(dir.path()).unwrap();
    engine
        .execute(include_str!("fixtures/svg_hazard_white_line.m"))
        .unwrap();
    let original = engine.figure.clone().unwrap();
    engine.execute("exportgraphics(gcf,'clear.svg','BackgroundColor','none');exportgraphics(gcf,'clear.pdf','BackgroundColor','none');").unwrap();
    assert!(original.same_snapshot(engine.figure.as_ref().unwrap()));
    let svg = std::fs::read_to_string(dir.path().join("clear.svg")).unwrap();
    let view: Vec<f64> = svg
        .split("viewBox=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .split_whitespace()
        .map(|v| v.parse().unwrap())
        .collect();
    let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).unwrap();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(
        tree.size().width().ceil() as u32,
        tree.size().height().ceil() as u32,
    )
    .unwrap();
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    let center = pixmap
        .pixel((495. - view[0]) as u32, (275. - view[1]) as u32)
        .unwrap();
    assert_eq!(
        (center.red(), center.green(), center.blue(), center.alpha()),
        (255, 255, 255, 255)
    );
    assert!(
        std::fs::read(dir.path().join("clear.pdf"))
            .unwrap()
            .starts_with(b"%PDF")
    );
    // Independent CairoSVG and Poppler/PDF rendering remains an external check;
    // merely writing a PDF is deliberately not called PDF visual validation.
}
