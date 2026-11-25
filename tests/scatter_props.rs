use matrust::{Engine, files, plot::MarkerColor};

fn matrix_data(engine: &Engine, name: &str) -> (usize, usize, Vec<f64>) {
    let m = &engine.variables[name];
    (m.rows(), m.cols(), m.data().to_vec())
}

fn text_val(engine: &Engine, name: &str) -> String {
    match engine.array(name) {
        Some(matrust::Array::Character(c)) => String::from_utf16_lossy(c.units()),
        other => panic!("expected char for {name}, got {other:?}"),
    }
}

#[test]
fn scatter_props_per_point_cdata_sizedata_and_dot_get_set() {
    let mut e = Engine::default();
    e.execute(
        "s = scatter(1:3, 1:3, [10 20 30], [1; 2; 3]);
         cd = s.CData;
         sz = s.SizeData;
         xd = s.XData;
         yd = s.YData;
         zd = s.ZData;
         mfc = s.MarkerFaceColor;
         mec = s.MarkerEdgeColor;
         mfa = s.MarkerFaceAlpha;
         mea = s.MarkerEdgeAlpha;
         lw = s.LineWidth;
         mk = s.Marker;
         cl = get(gca, 'CLim');",
    )
    .unwrap();
    assert_eq!(matrix_data(&e, "cd"), (3, 1, vec![1., 2., 3.]));
    assert_eq!(matrix_data(&e, "sz"), (1, 3, vec![10., 20., 30.]));
    assert_eq!(matrix_data(&e, "xd"), (1, 3, vec![1., 2., 3.]));
    assert_eq!(matrix_data(&e, "yd"), (1, 3, vec![1., 2., 3.]));
    assert_eq!(matrix_data(&e, "zd").2, Vec::<f64>::new());
    assert_eq!(text_val(&e, "mfc"), "none");
    assert_eq!(text_val(&e, "mec"), "flat");
    assert_eq!(matrix_data(&e, "mfa"), (1, 1, vec![1.]));
    assert_eq!(matrix_data(&e, "mea"), (1, 1, vec![1.]));
    assert_eq!(matrix_data(&e, "lw"), (1, 1, vec![0.5]));
    assert_eq!(text_val(&e, "mk"), "o");
    assert_eq!(matrix_data(&e, "cl"), (1, 2, vec![1., 3.]));

    e.execute(
        "s2 = scatter(1:3, 1:3, [10 20 30], [1; 2; 3], 'filled', 'd');
         mfc2 = s2.MarkerFaceColor;
         mk2 = s2.Marker;
         s2.MarkerFaceAlpha = 0.5;
         s2.MarkerEdgeAlpha = 0.25;
         mfa2 = s2.MarkerFaceAlpha;
         mea2 = s2.MarkerEdgeAlpha;
         s2.CData = [1 0 0];
         cd2 = s2.CData;
         s2.SizeData = 49;
         sz2 = s2.SizeData;
         s2.MarkerEdgeColor = 'none';
         mec2 = s2.MarkerEdgeColor;
         s2.MarkerFaceColor = 'auto';
         mfc3 = s2.MarkerFaceColor;",
    )
    .unwrap();
    assert_eq!(text_val(&e, "mfc2"), "flat");
    assert_eq!(text_val(&e, "mk2"), "d");
    assert_eq!(matrix_data(&e, "mfa2"), (1, 1, vec![0.5]));
    assert_eq!(matrix_data(&e, "mea2"), (1, 1, vec![0.25]));
    assert_eq!(matrix_data(&e, "cd2"), (1, 3, vec![1., 0., 0.]));
    assert_eq!(matrix_data(&e, "sz2"), (1, 1, vec![49.]));
    assert_eq!(text_val(&e, "mec2"), "none");
    assert_eq!(text_val(&e, "mfc3"), "auto");
    let s2 = &e.figure.as_ref().unwrap().series[0];
    assert!(s2.scatter_cdata.is_none());
    assert!(s2.marker_sizes.is_none());
    assert_eq!(s2.color, [255, 0, 0]);
    assert_eq!(s2.marker_face_color, MarkerColor::Auto);
    assert_eq!(s2.marker_edge_color, MarkerColor::None);
}

#[test]
fn scatter_props_scatter3_per_point_and_mfig_roundtrip() {
    let mut e = Engine::default();
    e.execute(
        "s = scatter3(1:3, 2:4, [5 6 7], [16 25 36], [1 0 0; 0 1 0; 0 0 1], 'filled', 's');
         set(s, 'MarkerFaceAlpha', 0.4, 'LineWidth', 1.5);
         zd = s.ZData;
         cd = s.CData;
         sz = s.SizeData;
         mfa = s.MarkerFaceAlpha;",
    )
    .unwrap();
    assert_eq!(matrix_data(&e, "zd"), (1, 3, vec![5., 6., 7.]));
    assert_eq!(
        matrix_data(&e, "cd"),
        (3, 3, vec![1., 0., 0., 0., 1., 0., 0., 0., 1.])
    );
    assert_eq!(matrix_data(&e, "sz"), (1, 3, vec![16., 25., 36.]));
    assert_eq!(matrix_data(&e, "mfa"), (1, 1, vec![0.4]));

    let f = e.figure.as_ref().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scatter3.mfig");
    files::save_figure(&path, f).unwrap();
    let loaded = files::load_figure(&path).unwrap();
    assert!(loaded.axes.same_metadata(&f.axes));
    assert_eq!(loaded.series[0].marker_sizes, f.series[0].marker_sizes);
    assert_eq!(loaded.series[0].scatter_cdata, f.series[0].scatter_cdata);
    assert_eq!(loaded.svg(), f.svg());
}

#[test]
fn scatter_props_indexed_cdata_colorbar_and_validation_errors() {
    let mut e = Engine::default();
    e.execute(
        "s = scatter(1:4, 1:4, 30, [10 20 30 40], 'filled');
         colorbar;
         colormap('hot');",
    )
    .unwrap();
    let f = e.figure.as_ref().unwrap();
    assert_eq!(f.axes.extras.color_limits(), [10., 40.]);
    let svg = f.svg();
    assert!(svg.contains("data-matrust-colorbar="));

    let before = f.clone();
    for bad in [
        "set(s, 'SizeData', [10 20]);",
        "set(s, 'CData', [1 2]);",
        "set(s, 'CData', [1 0 0; 0 1 0]);",
        "set(s, 'MarkerFaceAlpha', 1.5);",
        "set(s, 'MarkerEdgeAlpha', -0.1);",
        "set(s, 'MarkerEdgeColor', 'auto');",
        "set(s, 'SizeData', 0);",
    ] {
        assert!(e.execute(bad).is_err(), "{bad}");
        assert!(before.same_snapshot(e.figure.as_ref().unwrap()), "{bad}");
    }
}

#[cfg(feature = "plot-export")]
#[test]
fn scatter_props_rendered_png_blobs_and_sizes() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.execute(
        "scatter(1:2, 1:2, 200, [1 0 0; 0 0 1], 'filled');
         xlim([0 3]); ylim([0 3]); grid off;",
    )
    .unwrap();
    let png = dir.path().join("colors.png");
    matrust::export::export_figure(e.figure.as_ref().unwrap(), &png, 960, 560).unwrap();
    let img = image::open(&png).unwrap().to_rgb8();
    // Point (1, 1) is in the lower-left quadrant; (2, 2) is in the upper-right quadrant.
    let mut red_ll = 0usize;
    let mut blue_ur = 0usize;
    for (x, y, p) in img.enumerate_pixels() {
        let [r, g, b] = p.0;
        if r > 200 && g < 60 && b < 60 && x < 480 && y > 280 {
            red_ll += 1;
        }
        if b > 200 && r < 60 && g < 60 && x > 480 && y < 280 {
            blue_ur += 1;
        }
    }
    assert!(red_ll > 20, "expected red blob in lower-left, got {red_ll}");
    assert!(
        blue_ur > 20,
        "expected blue blob in upper-right, got {blue_ur}"
    );

    // Per-point SizeData: [20 400] -> second blob larger than first.
    let mut e2 = Engine::default();
    e2.execute(
        "scatter(1:2, 1:2, [20 400], 'r', 'filled');
         xlim([0 3]); ylim([0 3]); grid off;",
    )
    .unwrap();
    let png2 = dir.path().join("sizes.png");
    matrust::export::export_figure(e2.figure.as_ref().unwrap(), &png2, 960, 560).unwrap();
    let img2 = image::open(&png2).unwrap().to_rgb8();
    let mut first_blob = 0usize;
    let mut second_blob = 0usize;
    for (x, y, p) in img2.enumerate_pixels() {
        let [r, g, b] = p.0;
        if r > 200 && g < 60 && b < 60 {
            if x < 480 && y > 280 {
                first_blob += 1;
            } else if x > 480 && y < 280 {
                second_blob += 1;
            }
        }
    }
    assert!(first_blob > 0, "first_blob={first_blob}");
    assert!(
        second_blob > first_blob * 4,
        "expected second blob ({second_blob}) much larger than first ({first_blob})"
    );

    // MarkerFaceAlpha < 1 emits fill-opacity in SVG and blends in PNG.
    let mut e3 = Engine::default();
    e3.execute(
        "s = scatter(1:2, 1:2, 200, 'r', 'filled');
         s.MarkerFaceAlpha = 0.5;
         xlim([0 3]); ylim([0 3]); grid off;",
    )
    .unwrap();
    assert!(
        e3.figure
            .as_ref()
            .unwrap()
            .svg()
            .contains("fill-opacity=\"0.5\"")
    );
}

#[test]
fn f19_marker_edge_color_rgb_keeps_face_and_series_color() {
    let mut e = Engine::default();
    e.execute(
        "s = scatter(1:3, 1:3, 'filled');
         orig = s.CData;
         s.MarkerEdgeColor = [1 0 0];
         mec = s.MarkerEdgeColor;
         mfc = s.MarkerFaceColor;",
    )
    .unwrap();
    let s = &e.figure.as_ref().unwrap().series[0];
    assert_eq!(s.marker_edge_color, MarkerColor::Rgb([255, 0, 0]));
    assert_eq!(s.marker_face_color, MarkerColor::Flat);
    assert_ne!(s.color, [255, 0, 0]);
    assert_eq!(text_val(&e, "mfc"), "flat");
    assert_eq!(matrix_data(&e, "mec"), (1, 3, vec![1., 0., 0.]));
    // Also check constructor form:
    let mut e2 = Engine::default();
    e2.execute("scatter(1:3, 1:3, 'filled', 'MarkerEdgeColor', [0 1 0]);")
        .unwrap();
    let s2 = &e2.figure.as_ref().unwrap().series[0];
    assert_eq!(s2.marker_edge_color, MarkerColor::Rgb([0, 255, 0]));
    assert_ne!(s2.color, [0, 255, 0]);
}

#[test]
fn f13_auto_clim_stays_auto_and_updates_with_hold_on_and_cdata_changes() {
    let mut e = Engine::default();
    e.execute(
        "s1 = scatter(1:3, 1:3, 20, [1; 2; 3], 'filled');
         colorbar;
         cl1 = get(gca, 'CLim');
         cm1 = get(gca, 'CLimMode');
         hold on;
         s2 = scatter(4:6, 4:6, 20, [10; 20; 30], 'filled');
         cl2 = get(gca, 'CLim');
         cm2 = get(gca, 'CLimMode');",
    )
    .unwrap();
    assert_eq!(matrix_data(&e, "cl1"), (1, 2, vec![1., 3.]));
    assert_eq!(text_val(&e, "cm1"), "auto");
    assert_eq!(matrix_data(&e, "cl2"), (1, 2, vec![1., 30.]));
    assert_eq!(text_val(&e, "cm2"), "auto");
    let svg2 = e.figure.as_ref().unwrap().svg();
    assert!(svg2.contains("30"));

    e.execute(
        "s2.CData = [40; 50; 60];
         cl3 = get(gca, 'CLim');
         cm3 = get(gca, 'CLimMode');",
    )
    .unwrap();
    assert_eq!(matrix_data(&e, "cl3"), (1, 2, vec![1., 60.]));
    assert_eq!(text_val(&e, "cm3"), "auto");
    let svg3 = e.figure.as_ref().unwrap().svg();
    assert!(svg3.contains("60"));

    // Explicit CLim switches CLimMode to manual and is not overwritten by CData changes.
    e.execute(
        "clim([0 100]);
         cm4 = get(gca, 'CLimMode');
         s2.CData = [5; 6; 7];
         cl4 = get(gca, 'CLim');",
    )
    .unwrap();
    assert_eq!(text_val(&e, "cm4"), "manual");
    assert_eq!(matrix_data(&e, "cl4"), (1, 2, vec![0., 100.]));
}

#[test]
fn f6_native_mfig_preserves_clim_climmode_and_colormap() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.execute(
        "scatter(1:4, 1:4, 30, [10 20 30 40], 'filled');
         colormap('hot');",
    )
    .unwrap();
    let f = e.figure.as_ref().unwrap();
    let path = dir.path().join("indexed_auto.mfig");
    files::save_figure(&path, f).unwrap();
    let loaded = files::load_figure(&path).unwrap();
    assert!(loaded.axes.extras.clim.is_none());
    assert_eq!(loaded.axes.extras.color_limits(), [10., 40.]);
    assert_eq!(loaded.axes.extras.colormap, f.axes.extras.colormap);
    assert_eq!(loaded.svg(), f.svg());

    e.execute("clim([0 100]);").unwrap();
    let f_manual = e.figure.as_ref().unwrap();
    let path_manual = dir.path().join("indexed_manual.mfig");
    files::save_figure(&path_manual, f_manual).unwrap();
    let loaded_manual = files::load_figure(&path_manual).unwrap();
    assert_eq!(loaded_manual.axes.extras.clim, Some([0., 100.]));
    assert_eq!(loaded_manual.axes.extras.color_limits(), [0., 100.]);
    assert_eq!(loaded_manual.svg(), f_manual.svg());
}

#[test]
fn f3_matlab_fig_roundtrip_keeps_per_point_sizedata_cdata_and_alpha() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.execute(
        "s = scatter(1:3, 1:3, [16 36 64], [1 0 0; 0 1 0; 0 0 1], 'filled');
         s.MarkerFaceAlpha = 0.4;
         s.MarkerEdgeColor = [0 0 0];
         s.MarkerEdgeAlpha = 0.75;",
    )
    .unwrap();
    let f = e.figure.as_ref().unwrap();
    let path = dir.path().join("scatter_per_point.fig");
    files::save_figure(&path, f).unwrap();
    let loaded = files::load_figure(&path).unwrap();
    assert_eq!(loaded.series[0].marker_sizes, f.series[0].marker_sizes);
    assert_eq!(loaded.series[0].scatter_cdata, f.series[0].scatter_cdata);
    assert_eq!(
        loaded.series[0].marker_face_color,
        f.series[0].marker_face_color
    );
    assert_eq!(
        loaded.series[0].marker_edge_color,
        f.series[0].marker_edge_color
    );
    assert_eq!(
        loaded.series[0].marker_face_alpha,
        f.series[0].marker_face_alpha
    );
    assert_eq!(
        loaded.series[0].marker_edge_alpha,
        f.series[0].marker_edge_alpha
    );
    assert_eq!(loaded.svg(), f.svg());
}

#[test]
fn f10_decimated_scatter_maps_original_point_indices() {
    let mut e = Engine::default();
    e.execute(
        "n = 10000;
         x = 1:n;
         y = sin(x / 100);
         c = [repmat([1 0 0], n/2, 1); repmat([0 0 1], n/2, 1)];
         s = [repmat(16, 1, n/2), repmat(64, 1, n/2)];
         scatter(x, y, s, c, 'filled');",
    )
    .unwrap();
    let f = e.figure.as_ref().unwrap();
    let ser = &f.series[0];
    let axes = f.axis_transforms();
    let pts = ser.display_points_axes(8192, axes);
    let indices = ser.scatter_decimated_indices(8192, axes);
    assert!(pts.len() < ser.points.len());
    assert_eq!(pts.len(), indices.len());
    let map = f.axes.extras.effective_colormap();
    let clim = f.axes.extras.color_limits();
    // Last decimated point must be from the second half (blue, area 64), not index pts.len()-1.
    let last_orig = *indices.last().unwrap();
    assert!(last_orig >= 9990);
    assert_eq!(
        ser.scatter_color_at(last_orig, [0, 0, 0], &map, clim),
        [0, 0, 255]
    );
    assert_eq!(
        ser.marker_radius_at(last_orig),
        (64.0_f64.sqrt() * 2. / 3.) as f32
    );
}

#[test]
fn f11_filled_default_marker_stays_circle_and_sizedata_uses_area() {
    let mut e = Engine::default();
    e.execute(
        "s = scatter(1:3, 1:3, [36 100 196], 'filled');
         mk = s.Marker;
         hold on;
         p = plot(1:3, 1:3, 'o');",
    )
    .unwrap();
    assert_eq!(text_val(&e, "mk"), "o");
    let f = e.figure.as_ref().unwrap();
    let s = &f.series[0];
    assert_eq!(s.effective_marker(), matrust::plot::MarkerShape::Circle);
    assert_eq!(s.marker_radius_at(0), (36.0_f64.sqrt() * 2. / 3.) as f32);
    assert_eq!(s.marker_radius_at(1), (100.0_f64.sqrt() * 2. / 3.) as f32);
    assert_eq!(s.marker_radius_at(2), (196.0_f64.sqrt() * 2. / 3.) as f32);
    // Single-colour line path unchanged:
    let line = &f.series[1];
    assert_eq!(line.effective_marker(), matrust::plot::MarkerShape::Circle);
}

#[test]
fn f16_filled_sets_marker_edge_color_none() {
    let mut e = Engine::default();
    e.execute(
        "s = scatter(1:3, 1:3, 100, 'r', 'filled');
         s.MarkerFaceAlpha = 0.5;
         mec = s.MarkerEdgeColor;
         mfc = s.MarkerFaceColor;",
    )
    .unwrap();
    assert_eq!(text_val(&e, "mec"), "none");
    assert_eq!(text_val(&e, "mfc"), "flat");
    let f = e.figure.as_ref().unwrap();
    assert_eq!(f.series[0].marker_edge_color, MarkerColor::None);
    let svg = f.svg();
    assert!(svg.contains("fill-opacity=\"0.5\""));
    assert!(svg.contains("stroke=\"none\""));
}

#[test]
fn round1_explicit_dot_and_filled_marker() {
    let mut e = Engine::default();
    e.execute("scatter(1:3, 1:3, 20, [1;2;3], '.'); hold on; p = plot(1:3, 1:3, 'r.'); p.MarkerFaceAlpha = 0.5;").unwrap();
    let f = e.figure.as_ref().unwrap();
    for s in &f.series {
        assert_eq!(s.effective_marker(), matrust::plot::MarkerShape::Point);
    }
    assert_eq!(
        f.series[0].marker_radius(),
        (20_f64.sqrt() * 2. / 9.) as f32
    );
}

#[test]
fn round1_large_colormap_roundtrip() {
    let mut e = Engine::default();
    e.execute("scatter(1:3, 1:3, 20, [1;2;3]); colormap(parula(4096));")
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("large.mfig");
    files::save_figure(&path, e.figure.as_ref().unwrap()).unwrap();
    let loaded = files::load_figure(&path).unwrap();
    assert_eq!(
        loaded.extras.colormap,
        e.figure.as_ref().unwrap().extras.colormap
    );
}

#[test]
fn round1_delete_recomputes_scatter_clim() {
    let mut e = Engine::default();
    e.execute("a = scatter(1:2, 1:2, 20, [1;3]); hold on; b = scatter(1:2, 1:2, 20, [10;30]); colorbar; delete(b); cl = get(gca, 'CLim');").unwrap();
    assert_eq!(matrix_data(&e, "cl").2, vec![1., 3.]);
    let figure = e.figure.as_ref().unwrap();
    assert_eq!(figure.extras.color_limits(), [1., 3.]);
    let svg = figure.svg();
    let bar = svg
        .split("data-matrust-colorbar=")
        .nth(1)
        .unwrap()
        .split("</g>")
        .next()
        .unwrap();
    assert!(bar.contains(">1.0</text>") && bar.contains(">3.0</text>"));
    assert!(!bar.contains(">30</text>"));
    e.execute("a.CData = [4; 8]; updated = get(gca, 'CLim');")
        .unwrap();
    assert_eq!(matrix_data(&e, "updated").2, vec![4., 8.]);

    // Clearing CData to RGB or replacing/clearing the scatter recomputes auto CLim.
    e.execute(
        "a.CData = [1 0 0];
         cleared_rgb = get(gca, 'CLim');
         cm_rgb = get(gca, 'CLimMode');
         scatter(1:2, 1:2, 20, [5; 9]);
         replaced = get(gca, 'CLim');
         cm_rep = get(gca, 'CLimMode');
         clf;
         after_clf = get(gca, 'CLim');
         cm_clf = get(gca, 'CLimMode');",
    )
    .unwrap();
    assert_eq!(matrix_data(&e, "cleared_rgb").2, vec![0., 1.]);
    assert_eq!(text_val(&e, "cm_rgb"), "auto");
    assert_eq!(matrix_data(&e, "replaced").2, vec![5., 9.]);
    assert_eq!(text_val(&e, "cm_rep"), "auto");
    assert_eq!(matrix_data(&e, "after_clf").2, vec![0., 1.]);
    assert_eq!(text_val(&e, "cm_clf"), "auto");
}
