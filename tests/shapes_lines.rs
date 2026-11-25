use matrust::{Array, Engine, GraphicsKind, files};
use std::sync::atomic::AtomicBool;

fn text(e: &Engine, name: &str) -> String {
    let Array::Character(value) = e.array(name).unwrap() else {
        panic!("Expected character row for {name}");
    };
    value.to_utf8(&AtomicBool::new(false)).unwrap()
}

#[test]
fn xline_and_yline_handles_and_properties() {
    let mut e = Engine::default();
    e.execute(
        "h = xline(2); v = h.Value; ax_x = h.InterceptAxis; tp = get(h, 'Type'); \
         h.Label = 'cut'; h.Alpha = 0.5; lbl = get(h, 'Label'); alp = h.Alpha; \
         ys = yline([1 3], '--r', 'lim'); n = numel(ys); \
         l1 = ys(1).Label; ls1 = ys(1).LineStyle; c2 = ys(2).Color; v2 = ys(2).Value; \
         ay = ys(1).InterceptAxis;",
    )
    .unwrap();
    assert_eq!(e.variables["v"].data(), &[2.]);
    assert_eq!(text(&e, "ax_x"), "x");
    assert_eq!(text(&e, "tp"), "constantline");
    assert_eq!(text(&e, "lbl"), "cut");
    assert_eq!(e.variables["alp"].data(), &[0.5]);

    assert_eq!(e.variables["n"].data(), &[2.]);
    assert_eq!(text(&e, "l1"), "lim");
    assert_eq!(text(&e, "ls1"), "--");
    assert_eq!(e.variables["c2"].data(), &[1., 0., 0.]);
    assert_eq!(e.variables["v2"].data(), &[3.]);
    assert_eq!(text(&e, "ay"), "y");

    let Array::Graphics(arr) = e.array("ys").unwrap() else {
        panic!();
    };
    assert_eq!(arr.len(), 2);
    assert_eq!(
        arr.get_linear(0).unwrap().kind(),
        GraphicsKind::ConstantLine
    );
}

#[test]
fn xline_follows_zoom_and_does_not_expand_y_limits() {
    let mut e = Engine::default();
    e.execute("plot(1:10, 1:10); h = xline(5);").unwrap();
    let (xb, yb) = e.figure.as_ref().unwrap().axes.bounds();
    assert!(xb[0] <= 1. && xb[1] >= 10.);
    assert!(yb[0] <= 1. && yb[1] >= 10.);
    e.execute("xlim([3 7]); ylim([2 8]);").unwrap();
    let (xb2, yb2) = e.figure.as_ref().unwrap().axes.bounds();
    assert_eq!(xb2, [3., 7.]);
    assert_eq!(yb2, [2., 8.]);
}

#[test]
fn rectangle_handles_and_properties() {
    let mut e = Engine::default();
    e.execute(
        "r = rectangle('Position', [1 1 2 3], 'Curvature', 1); \
         pos = r.Position; curv = r.Curvature; fc = r.FaceColor; ec = r.EdgeColor; \
         tp = get(r, 'Type'); \
         set(r, 'FaceColor', 'r', 'EdgeColor', 'none', 'LineWidth', 2, 'LineStyle', ':'); \
         fc2 = r.FaceColor; ec2 = r.EdgeColor; lw2 = r.LineWidth; ls2 = r.LineStyle;",
    )
    .unwrap();
    assert_eq!(e.variables["pos"].data(), &[1., 1., 2., 3.]);
    assert_eq!(e.variables["curv"].data(), &[1., 1.]);
    assert_eq!(text(&e, "fc"), "none");
    assert_eq!(e.variables["ec"].data(), &[0., 0., 0.]);
    assert_eq!(text(&e, "tp"), "rectangle");
    assert_eq!(e.variables["fc2"].data(), &[1., 0., 0.]);
    assert_eq!(text(&e, "ec2"), "none");
    assert_eq!(e.variables["lw2"].data(), &[2.]);
    assert_eq!(text(&e, "ls2"), ":");

    let Array::Graphics(arr) = e.array("r").unwrap() else {
        panic!();
    };
    assert_eq!(arr.scalar_handle().unwrap().kind(), GraphicsKind::Rectangle);
}

#[test]
fn patch_positional_color_alpha_and_3d() {
    let mut e = Engine::default();
    e.execute(
        "p = patch([0 1 1], [0 0 1], 'g', 'FaceAlpha', 0.3); \
         fa = p.FaceAlpha; fc = p.FaceColor; \
         p2 = patch([0 1 1], [0 0 1], [1 2 3], 'r', 'FaceColor', 'none', 'EdgeAlpha', 0.4, 'LineWidth', 2); \
         fc2 = p2.FaceColor; ea2 = p2.EdgeAlpha; lw2 = p2.LineWidth;",
    )
    .unwrap();
    assert_eq!(e.variables["fa"].data(), &[0.3]);
    assert_eq!(e.variables["fc"].data(), &[0., 1., 0.]);
    assert_eq!(text(&e, "fc2"), "none");
    assert_eq!(e.variables["ea2"].data(), &[0.4]);
    assert_eq!(e.variables["lw2"].data(), &[2.]);
}

#[test]
fn rendered_png_xline_rectangle_and_patch_alpha() {
    let dir = tempfile::tempdir().unwrap();

    // 1. xline(5) on plot(1:10) gives a vertical line of pixels at x=5 from bottom to top of the axes.
    let mut e = Engine::default();
    e.execute("figure('Color','w'); plot(1:10, 1:10, 'w'); xlim([0 10]); ylim([0 10]); xline(5, 'k', 'LineWidth', 2, 'Alpha', 1);")
        .unwrap();
    let png = dir.path().join("xline.png");
    matrust::export::export_figure(e.figure.as_ref().unwrap(), &png, 960, 560).unwrap();
    let img = image::open(&png).unwrap().to_rgb8();
    // Find columns that have dark pixels spanning a tall vertical run inside the plot area.
    let mut max_run = 0u32;
    let mut best_x = 0u32;
    for x in 100..860 {
        let mut run = 0u32;
        for y in 40..500 {
            let px = img.get_pixel(x, y);
            if px[0] < 80 && px[1] < 80 && px[2] < 80 {
                run += 1;
            }
        }
        if run > max_run {
            max_run = run;
            best_x = x;
        }
    }
    assert!(
        max_run > 300,
        "Expected vertical xline pixels spanning axes, max_run={max_run} at x={best_x}"
    );

    // 2. FaceColor red rectangle has red interior.
    let mut e2 = Engine::default();
    e2.execute("figure('Color','w'); xlim([0 10]); ylim([0 10]); rectangle('Position',[2 2 6 6],'FaceColor','r','EdgeColor','none');")
        .unwrap();
    let png2 = dir.path().join("rect.png");
    matrust::export::export_figure(e2.figure.as_ref().unwrap(), &png2, 960, 560).unwrap();
    let img2 = image::open(&png2).unwrap().to_rgb8();
    let center = img2.get_pixel(480, 280);
    assert!(
        center[0] > 230 && center[1] < 25 && center[2] < 25,
        "Expected red interior at center, got {:?}",
        center.0
    );

    // 3. FaceAlpha 0.3 green patch over white gives a light green (not pure green) interior.
    let mut e3 = Engine::default();
    e3.execute("figure('Color','w'); xlim([0 1]); ylim([0 1]); patch([0 1 1 0],[0 0 1 1],'g','FaceAlpha',0.3,'EdgeColor','none');")
        .unwrap();
    let png3 = dir.path().join("patch_alpha.png");
    matrust::export::export_figure(e3.figure.as_ref().unwrap(), &png3, 960, 560).unwrap();
    let img3 = image::open(&png3).unwrap().to_rgb8();
    let c3 = img3.get_pixel(480, 280);
    assert!(
        c3[1] > 220 && c3[0] > 140 && c3[2] > 140 && c3[1] > c3[0] + 40,
        "Expected light green interior from FaceAlpha 0.3 over white, got {:?}",
        c3.0
    );
}

#[test]
fn figure_save_load_roundtrip_preserves_shapes_and_lines() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute(
        "f = figure; plot(1:5, 1:5); \
         xline(2, '--r', 'vx'); yline(3, ':b', 'hy'); \
         rectangle('Position', [1 1 2 2], 'Curvature', 0.5, 'FaceColor', 'g'); \
         patch([0 1 1], [0 0 1], 'g', 'FaceAlpha', 0.3, 'EdgeAlpha', 0.5); \
         savefig(f, 'shapes.mfig');",
    )
    .unwrap();
    let before = e.figure.clone().unwrap();
    let decoded = files::load_figure(&dir.path().join("shapes.mfig")).unwrap();
    assert_eq!(before.svg(), decoded.svg());
    assert_eq!(decoded.axes.extras.constant_lines.len(), 2);
    assert_eq!(decoded.axes.extras.rectangles.len(), 1);
    assert_eq!(decoded.axes.series[1].face_alpha, Some(0.3));
    assert_eq!(decoded.axes.series[1].edge_alpha, Some(0.5));
}
