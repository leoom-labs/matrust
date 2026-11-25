use matrust::{Array, Engine, files};

fn run(script: &str) -> Engine {
    let mut engine = Engine::default();
    engine.execute(script).unwrap();
    engine
}

fn matrix_data(engine: &Engine, name: &str) -> Vec<f64> {
    match engine.workspace().get(name).unwrap() {
        Array::Numeric(m) => m.data().to_vec(),
        other => panic!("expected numeric array for {name}, got {other:?}"),
    }
}

fn text_value(engine: &Engine, name: &str) -> String {
    match engine.workspace().get(name).unwrap() {
        Array::Character(ch) => String::from_utf16_lossy(ch.units()),
        other => panic!("expected character array for {name}, got {other:?}"),
    }
}

fn cell_strings(engine: &Engine, name: &str) -> Vec<String> {
    match engine.workspace().get(name).unwrap() {
        Array::Cell(cell) => (0..cell.len())
            .map(|i| match cell.get_linear(i).unwrap().as_ref() {
                Array::Character(ch) => String::from_utf16_lossy(ch.units()),
                other => panic!("expected char cell, got {other:?}"),
            })
            .collect(),
        other => panic!("expected cell for {name}, got {other:?}"),
    }
}

#[test]
fn ticks_axes_xticks_set_get_and_auto() {
    let e = run(
        "plot(1:10); xticks(1:3); t = get(gca, 'XTick'); m = get(gca, 'XTickMode'); t2 = xticks;",
    );
    assert_eq!(matrix_data(&e, "t"), vec![1.0, 2.0, 3.0]);
    assert_eq!(matrix_data(&e, "t2"), vec![1.0, 2.0, 3.0]);
    assert_eq!(text_value(&e, "m"), "manual");

    let e2 = run(
        "plot(1:10); auto_t = xticks; xticks(1:3); xticks('auto'); restored = get(gca, 'XTick'); m2 = get(gca, 'XTickMode');",
    );
    assert_eq!(matrix_data(&e2, "restored"), matrix_data(&e2, "auto_t"));
    assert_eq!(text_value(&e2, "m2"), "auto");

    let e3 = run(
        "ax = gca; plot(ax, 1:10); xticks(ax, [2 5 8]); yticks(ax, [1 9]); tx = xticks(ax); ty = yticks(ax);",
    );
    assert_eq!(matrix_data(&e3, "tx"), vec![2.0, 5.0, 8.0]);
    assert_eq!(matrix_data(&e3, "ty"), vec![1.0, 9.0]);

    let e4 = run("plot3([0 1], [0 1], [0 4]); zticks([0 2 4]); tz = zticks;");
    assert_eq!(matrix_data(&e4, "tz"), vec![0.0, 2.0, 4.0]);
}

#[test]
fn ticks_axes_xticklabels_readback_and_pairing() {
    let e = run(
        "plot(1:3); xticks(1:3); xticklabels({'a','b','c'}); l = get(gca, 'XTickLabel'); l2 = xticklabels; m = get(gca, 'XTickLabelMode');",
    );
    assert_eq!(cell_strings(&e, "l"), vec!["a", "b", "c"]);
    assert_eq!(cell_strings(&e, "l2"), vec!["a", "b", "c"]);
    assert_eq!(text_value(&e, "m"), "manual");

    // Fewer labels than ticks blanks the rest; more labels ignores extra.
    let e2 = run("plot(1:4); xticks(1:4); yticks([]); xticklabels([\"x\" \"y\"]);");
    let svg = e2.figure.as_ref().unwrap().svg();
    assert!(svg.contains(">x</text>"));
    assert!(svg.contains(">y</text>"));
    assert!(!svg.contains(">3</text>"));

    let e3 = run("plot(1:2); xticks(1:2); xticklabels({'p','q','extra'});");
    let svg3 = e3.figure.as_ref().unwrap().svg();
    assert!(svg3.contains(">p</text>"));
    assert!(svg3.contains(">q</text>"));
    assert!(!svg3.contains("extra"));
}

#[test]
fn ticks_axes_xtickangle_and_rotation() {
    let e = run(
        "plot(1:5); xtickangle(45); r = get(gca, 'XTickLabelRotation'); a = xtickangle; ytickangle(30); yr = get(gca, 'YTickLabelRotation');",
    );
    assert_eq!(matrix_data(&e, "r"), vec![45.0]);
    assert_eq!(matrix_data(&e, "a"), vec![45.0]);
    assert_eq!(matrix_data(&e, "yr"), vec![30.0]);
    let svg = e.figure.as_ref().unwrap().svg();
    assert!(svg.contains("transform=\"rotate(-45 "));
    assert!(svg.contains("transform=\"rotate(-30 "));
}

#[test]
fn ticks_axes_properties_set_get_and_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let e = run(
        "plot(1:5); set(gca, 'TickDir', 'out', 'LineWidth', 1.5, 'TickLength', [0.02 0.04]); td = get(gca, 'TickDir'); lw = get(gca, 'LineWidth'); tl = get(gca, 'TickLength');",
    );
    assert_eq!(text_value(&e, "td"), "out");
    assert_eq!(matrix_data(&e, "lw"), vec![1.5]);
    assert_eq!(matrix_data(&e, "tl"), vec![0.02, 0.04]);

    let path = dir.path().join("ticks_axes.mfig");
    let fig = e.figure.as_ref().unwrap();
    files::save_figure(&path, fig).unwrap();
    let loaded = files::load_figure(&path).unwrap();
    assert_eq!(loaded.svg(), fig.svg());
}

#[test]
fn ticks_axes_numeric_ruler_properties() {
    let e = run("plot(1:1000:5000, 1:5); ax = gca; xlabel('Time'); \
         ax.XAxis.Exponent = 3; ex = ax.XAxis.Exponent; \
         ax.XAxis.TickLabelFormat = '%.1f'; fmt = ax.XAxis.TickLabelFormat; \
         ax.XAxis.Color = [1 0 0]; col = ax.XAxis.Color; \
         lbl = ax.XAxis.Label.String;");
    assert_eq!(matrix_data(&e, "ex"), vec![3.0]);
    assert_eq!(text_value(&e, "fmt"), "%.1f");
    assert_eq!(matrix_data(&e, "col"), vec![1.0, 0.0, 0.0]);
    assert_eq!(text_value(&e, "lbl"), "Time");
    let svg = e.figure.as_ref().unwrap().svg();
    assert!(svg.contains("×10^3"));
}

#[test]
fn ticks_axes_infinite_limits() {
    let e = run(
        "plot(1:10); ylim([0 inf]); yl = get(gca, 'YLim'); xlim([-inf 5]); xl = get(gca, 'XLim');",
    );
    assert_eq!(matrix_data(&e, "yl"), vec![0.0, 10.0]);
    assert_eq!(matrix_data(&e, "xl"), vec![1.0, 5.0]);

    let e2 = run(
        "plot(1:10); ax = gca; set(ax, 'YLim', [2 inf]); yl2 = ax.YLim; ax.YAxis.Limits = [-inf 8]; yl3 = ax.YAxis.Limits;",
    );
    assert_eq!(matrix_data(&e2, "yl2"), vec![2.0, 10.0]);
    assert_eq!(matrix_data(&e2, "yl3"), vec![1.0, 8.0]);
}

#[test]
fn ticks_axes_colororder_and_cycling() {
    let e = run(
        "colororder([1 0 0; 0 0 1]); h = plot(rand(3, 2)); c1 = h(1).Color; c2 = h(2).Color; cur = colororder;",
    );
    assert_eq!(matrix_data(&e, "c1"), vec![1.0, 0.0, 0.0]);
    assert_eq!(matrix_data(&e, "c2"), vec![0.0, 0.0, 1.0]);
    assert_eq!(matrix_data(&e, "cur"), vec![1.0, 0.0, 0.0, 0.0, 0.0, 1.0]);

    let e2 = run("colororder('gem'); c = colororder;");
    assert_eq!(matrix_data(&e2, "c").len(), 21);

    let mut e3 = Engine::default();
    assert!(e3.execute("colororder('nonexistent_palette');").is_err());
}

#[cfg(feature = "plot-export")]
#[test]
fn ticks_axes_rendered_png_ticks_and_tickdir_out() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.execute("plot(1:10); xticks([2 8]); yticks([]); box off;")
        .unwrap();
    let fig = e.figure.as_ref().unwrap();
    let png = dir.path().join("ticks_in.png");
    matrust::export::export_figure(fig, &png, 960, 560).unwrap();
    let img = image::open(&png).unwrap().to_rgb8();

    // Find the horizontal x-axis line (bottom row with many dark pixels).
    let is_dark = |p: &image::Rgb<u8>| p[0] < 220 && p[1] < 220 && p[2] < 220;
    let axis_y = (0..img.height())
        .rev()
        .find(|&y| {
            let count = (0..img.width())
                .filter(|&x| is_dark(img.get_pixel(x, y)))
                .count();
            count > 400
        })
        .expect("x-axis line not found");

    // Count distinct dark tick-mark column clusters just above the x-axis line (TickDir 'in').
    let tick_y = axis_y - 3;
    let mut clusters = 0usize;
    let mut in_cluster = false;
    // Skip the left y-axis line by starting after the axis line's left end + 10px.
    let left_x = (0..img.width())
        .find(|&x| is_dark(img.get_pixel(x, axis_y)))
        .unwrap();
    for x in (left_x + 10)..img.width() {
        let dark = is_dark(img.get_pixel(x, tick_y));
        if dark && !in_cluster {
            clusters += 1;
            in_cluster = true;
        } else if !dark {
            in_cluster = false;
        }
    }
    assert_eq!(
        clusters, 2,
        "expected 2 tick marks on x-axis for xticks([2 8])"
    );

    // Now test TickDir 'out': tick pixels appear below the x-axis line.
    let mut e_out = Engine::default();
    e_out
        .execute("plot(1:10); xticks([2 8]); yticks([]); box off; set(gca, 'TickDir', 'out');")
        .unwrap();
    let png_out = dir.path().join("ticks_out.png");
    matrust::export::export_figure(e_out.figure.as_ref().unwrap(), &png_out, 960, 560).unwrap();
    let img_out = image::open(&png_out).unwrap().to_rgb8();
    let out_tick_y = axis_y + 3;
    let mut out_clusters = 0usize;
    let mut in_out = false;
    for x in (left_x + 10)..img_out.width() {
        let dark = is_dark(img_out.get_pixel(x, out_tick_y));
        if dark && !in_out {
            out_clusters += 1;
            in_out = true;
        } else if !dark {
            in_out = false;
        }
    }
    assert_eq!(
        out_clusters, 2,
        "expected 2 outward tick marks below x-axis for TickDir 'out'"
    );
}

#[test]
fn ticks_axes_colororder_handles_preserve_figure_identity() {
    for target in ["gca", "gcf"] {
        let e = run(&format!(
            "plot(1:3); colororder({target}, [1 0 0; 0 0 1]); h=gca; v=get(h,'XTick');"
        ));
        assert_eq!(e.figures().count(), 1);
        assert!(!matrix_data(&e, "v").is_empty());
    }
}

#[test]
fn ticks_axes_dendrogram_fractional_tick_is_safe() {
    let e = run("dendrogram([1 2 1; 3 4 2]); xticks([0.5 1 2]);");
    let ax = &e.figure.as_ref().unwrap().axes;
    assert_eq!(ax.effective_tick_labels(0).len(), 3);
    assert!(e.figure.as_ref().unwrap().svg().contains("<svg"));
}

#[test]
fn ticks_axes_infinite_limits_figure_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = run("plot3(1:10,1:10,1:10); xlim([-inf 5]); ylim([0 inf]); zlim([-inf 8]);");
    let path = dir.path().join("infinite.mfig");
    files::save_figure(&path, e.figure.as_ref().unwrap()).unwrap();
    let f = files::load_figure(&path).unwrap();
    assert_eq!(f.xlim, Some([f64::NEG_INFINITY, 5.]));
    assert_eq!(f.ylim, Some([0., f64::INFINITY]));
    assert_eq!(f.zlim, Some([f64::NEG_INFINITY, 8.]));
    // The live engine figure (not the loaded copy, which bypasses the
    // figure registry) reports finite effective limits.
    e.execute("yl=ylim;").unwrap();
    assert_eq!(matrix_data(&e, "yl"), vec![0., 10.]);
}

#[test]
fn ticks_axes_irregular_manual_labels_have_precision() {
    let e = run("plot(1:3); xticks([1 2.5 2.75]);");
    let ax = &e.figure.as_ref().unwrap().axes;
    assert_eq!(ax.effective_tick_labels(0), ["1.00", "2.50", "2.75"]);
    let svg = e.figure.as_ref().unwrap().svg();
    assert!(svg.contains(">2.75</text>"));
}

#[test]
fn ticks_axes_z_ruler_color_is_independent() {
    let e = run(
        "plot3(1:3,1:3,1:3); ax=gca; ax.YAxis.Color=[0 1 0]; ax.ZAxis.Color=[1 0 0]; y=ax.YAxis.Color; z=ax.ZAxis.Color;",
    );
    assert_eq!(matrix_data(&e, "y"), vec![0., 1., 0.]);
    assert_eq!(matrix_data(&e, "z"), vec![1., 0., 0.]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("color.mfig");
    files::save_figure(&path, e.figure.as_ref().unwrap()).unwrap();
    assert_eq!(
        files::load_figure(&path).unwrap().axes.colors.unwrap().z,
        Some([1., 0., 0.])
    );
}

#[test]
fn ticks_axes_format_escaped_percent() {
    let e = run("plot(1:3); xticks([1 2]); ax=gca; ax.XAxis.TickLabelFormat='%g\\%';");
    assert_eq!(
        e.figure.as_ref().unwrap().axes.effective_tick_labels(0),
        ["1%", "2%"]
    );
    assert!(e.figure.as_ref().unwrap().svg().contains(">1%</text>"));
}

/// Axes `Layer` (GNU Octave 10.3: default `bottom`; `Top` reads back as
/// canonical `top`; a bad value is rejected and leaves `top`).
#[test]
fn ticks_axes_layer_round_trip_errors_and_native_file() {
    let mut e = run(
        "plot(1:3); ax = gca; d = get(ax, 'Layer'); p = double(isprop(ax, 'Layer'));
        set(gca, 'Layer', 'Top'); t = get(gca, 'Layer'); dt = ax.Layer;",
    );
    assert_eq!(text_value(&e, "d"), "bottom");
    assert_eq!(matrix_data(&e, "p"), vec![1.]);
    assert_eq!(text_value(&e, "t"), "top");
    assert_eq!(text_value(&e, "dt"), "top");
    // Bad values and a failing later pair leave the whole set unapplied.
    for bad in [
        "set(gca, 'Layer', 'middle');",
        "set(gca, 'Layer', 5);",
        "ax.Layer = 'front';",
        "set(gca, 'Layer', 'bottom', 'TickDir', 'sideways');",
    ] {
        assert!(e.execute(bad).is_err(), "{bad}");
        e.execute("v = get(gca, 'Layer');").unwrap();
        assert_eq!(text_value(&e, "v"), "top", "{bad}");
    }
    e.execute(
        "ax.Layer = \"BOTTOM\"; b = ax.Layer; set(ax, 'layer', 'tOp'); c = get(ax, 'Layer');",
    )
    .unwrap();
    assert_eq!(text_value(&e, "b"), "bottom");
    assert_eq!(text_value(&e, "c"), "top");

    // Per-axes state: a second subplot keeps the default.
    let e = run(
        "subplot(1,2,1); plot(1:3); a1 = gca; set(a1, 'Layer', 'top');
        subplot(1,2,2); plot(3:-1:1); a2 = gca; l1 = get(a1, 'Layer'); l2 = get(a2, 'Layer');",
    );
    assert_eq!(text_value(&e, "l1"), "top");
    assert_eq!(text_value(&e, "l2"), "bottom");

    // Native save/reopen keeps `top`; the default writes no field and an
    // absent field (any older file) reopens as `bottom`.
    let dir = tempfile::tempdir().unwrap();
    let fig = e.figure.as_ref().unwrap();
    let path = dir.path().join("layer_subplots.mfig");
    files::save_figure(&path, fig).unwrap();
    let loaded = files::load_figure(&path).unwrap();
    let layers = |f: &matrust::plot::Figure| -> Vec<(String, &'static str)> {
        f.all_axes()
            .map(|(_, a)| {
                (
                    a.title.clone() + &a.series.len().to_string(),
                    a.extras.layer.as_str(),
                )
            })
            .collect()
    };
    let before = layers(fig);
    assert_eq!(before.iter().filter(|(_, l)| *l == "top").count(), 1);
    assert_eq!(before.len(), 2);
    assert_eq!(layers(&loaded), before);
    // The current axes (the second subplot) stays bottom after reopening.
    assert!(!loaded.extras.layer.is_top());
    let single = run("plot(1:3); set(gca, 'Layer', 'top');");
    let path = dir.path().join("layer_top.mfig");
    files::save_figure(&path, single.figure.as_ref().unwrap()).unwrap();
    let bytes = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned();
    assert!(bytes.contains("\"layer\":\"top\""), "{bytes}");
    assert!(files::load_figure(&path).unwrap().extras.layer.is_top());
    let mut reopened = Engine::default();
    reopened
        .execute(&format!(
            "openfig('{}'); r = get(gca, 'Layer');",
            path.display()
        ))
        .unwrap();
    assert_eq!(text_value(&reopened, "r"), "top");
    let plain = run("plot(1:3);");
    let path = dir.path().join("layer_default.mfig");
    files::save_figure(&path, plain.figure.as_ref().unwrap()).unwrap();
    let bytes = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned();
    assert!(!bytes.contains("\"layer\""), "{bytes}");
    assert!(!files::load_figure(&path).unwrap().extras.layer.is_top());
}

/// Commands that set `Layer` (GNU Octave 10.3 probe, 2026-11-22): a replacing
/// image/imagesc and every contourf (held or not) give `top`; contour, a held
/// image and a later replacing plot stay `bottom`.
#[test]
fn ticks_axes_layer_set_by_image_and_contourf() {
    let e = run("figure; imagesc(magic(3)); a = get(gca, 'Layer');
        figure; image(magic(3)); b = get(gca, 'Layer');
        figure; contourf(peaks(5)); c = get(gca, 'Layer');
        figure; plot(1:3); hold on; contourf(peaks(5)); d = get(gca, 'Layer');
        figure; contour(peaks(5)); f = get(gca, 'Layer');
        figure; plot(1:3); hold on; image(magic(3)); g = get(gca, 'Layer');
        figure; imagesc(magic(3)); plot(1:3); h = get(gca, 'Layer');");
    for (name, want) in [
        ("a", "top"),
        ("b", "top"),
        ("c", "top"),
        ("d", "top"),
        ("f", "bottom"),
        ("g", "bottom"),
        ("h", "bottom"),
    ] {
        assert_eq!(text_value(&e, name), want, "{name}");
    }
}
