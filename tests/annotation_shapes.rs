use matrust::{Array, Engine, GraphicsKind, files};
use std::sync::atomic::AtomicBool;

fn text(e: &Engine, name: &str) -> String {
    let Array::Character(value) = e.array(name).unwrap() else {
        panic!("Expected character row for {name}");
    };
    value.to_utf8(&AtomicBool::new(false)).unwrap()
}

#[test]
fn annotation_line_arrow_doublearrow_textarrow_rectangle_ellipse_handles() {
    let mut e = Engine::default();
    e.execute(
        "f = figure; \
         a = annotation('arrow', [.2 .5], [.2 .5]); \
         ax = a.X; ay = a.Y; apos = a.Position; atype = get(a, 'Type'); aunits = a.Units; \
         ta = annotation(f, 'textarrow', [.3 .5], [.6 .5], 'String', 'peak', 'FontSize', 12); \
         tstr = ta.String; tfs = get(ta, 'FontSize'); ttype = ta.Type; \
         da = annotation('doublearrow', [.1 .4], [.8 .8], 'HeadStyle', 'plain', 'HeadLength', 12, 'HeadWidth', 10); \
         dhs = da.HeadStyle; dhl = da.HeadLength; dhw = da.HeadWidth; dtype = da.Type; \
         ln = annotation('line', [.1 .9], [.1 .1], 'Color', 'b', 'LineWidth', 2, 'LineStyle', '--'); \
         lc = ln.Color; llw = ln.LineWidth; lls = ln.LineStyle; ltype = ln.Type; \
         el = annotation('ellipse', [.2 .2 .3 .3], 'FaceColor', [0 1 0], 'FaceAlpha', 0.5); \
         epos = el.Position; efc = el.FaceColor; efa = el.FaceAlpha; etype = el.Type; \
         rc = annotation('rectangle', [.5 .5 .2 .2], 'Color', 'r', 'FaceColor', 'r'); \
         rpos = rc.Position; rtype = rc.Type;",
    )
    .unwrap();

    assert_eq!(e.variables["ax"].data(), &[0.2, 0.5]);
    assert_eq!(e.variables["ay"].data(), &[0.2, 0.5]);
    assert!((e.variables["apos"].data()[0] - 0.2).abs() < 1e-12);
    assert!((e.variables["apos"].data()[1] - 0.2).abs() < 1e-12);
    assert!((e.variables["apos"].data()[2] - 0.3).abs() < 1e-12);
    assert!((e.variables["apos"].data()[3] - 0.3).abs() < 1e-12);
    assert_eq!(text(&e, "atype"), "arrowshape");
    assert_eq!(text(&e, "aunits"), "normalized");

    assert_eq!(text(&e, "tstr"), "peak");
    assert_eq!(e.variables["tfs"].data(), &[12.0]);
    assert_eq!(text(&e, "ttype"), "textarrowshape");

    assert_eq!(text(&e, "dhs"), "plain");
    assert_eq!(e.variables["dhl"].data(), &[12.0]);
    assert_eq!(e.variables["dhw"].data(), &[10.0]);
    assert_eq!(text(&e, "dtype"), "doubleendarrowshape");

    assert_eq!(e.variables["lc"].data(), &[0.0, 0.0, 1.0]);
    assert_eq!(e.variables["llw"].data(), &[2.0]);
    assert_eq!(text(&e, "lls"), "--");
    assert_eq!(text(&e, "ltype"), "lineshape");

    assert_eq!(e.variables["epos"].data(), &[0.2, 0.2, 0.3, 0.3]);
    assert_eq!(e.variables["efc"].data(), &[0.0, 1.0, 0.0]);
    assert_eq!(e.variables["efa"].data(), &[0.5]);
    assert_eq!(text(&e, "etype"), "ellipseshape");

    assert_eq!(e.variables["rpos"].data(), &[0.5, 0.5, 0.2, 0.2]);
    assert_eq!(text(&e, "rtype"), "rectangleshape");

    let Array::Graphics(arr) = e.array("a").unwrap() else {
        panic!("Expected graphics handle");
    };
    assert_eq!(
        arr.scalar_handle().unwrap().kind(),
        GraphicsKind::Annotation
    );
}

#[test]
fn annotation_set_dot_and_delete_removes() {
    let mut e = Engine::default();
    e.execute(
        "a = annotation('arrow', [.2 .5], [.2 .5]); \
         a.X = [.1 .6]; \
         set(a, 'Color', 'r', 'LineWidth', 3, 'HeadStyle', 'none'); \
         ax = a.X; ac = a.Color; alw = a.LineWidth; ahs = a.HeadStyle;",
    )
    .unwrap();
    assert_eq!(e.variables["ax"].data(), &[0.1, 0.6]);
    assert_eq!(e.variables["ac"].data(), &[1.0, 0.0, 0.0]);
    assert_eq!(e.variables["alw"].data(), &[3.0]);
    assert_eq!(text(&e, "ahs"), "none");
    assert_eq!(e.figure.as_ref().unwrap().annotations.len(), 1);

    e.execute("delete(a);").unwrap();
    assert_eq!(e.figure.as_ref().unwrap().annotations.len(), 0);
    assert!(e.execute("a.X").is_err());
}

#[test]
fn annotation_save_and_load_roundtrip() {
    let mut e = Engine::default();
    e.execute(
        "annotation('textarrow', [.2 .7], [.3 .8], 'String', 'peak', 'Color', 'b'); \
         annotation('rectangle', [.1 .2 .3 .4], 'FaceColor', 'r', 'FaceAlpha', 0.4);",
    )
    .unwrap();
    let fig = e.figure.as_ref().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ann.mfig");
    files::save_figure(&path, fig).unwrap();
    let restored = files::load_figure(&path).unwrap();
    assert_eq!(restored.annotations, fig.annotations);
}

#[test]
fn rendered_png_arrow_and_red_rectangle() {
    let dir = tempfile::tempdir().unwrap();

    // 1. Arrow from (.2,.2) to (.8,.8): dark pixels near midpoint and more dark pixels near tip than along shaft.
    let mut e = Engine::default();
    e.execute(
        "figure('Color', 'w'); \
         annotation('arrow', [.2 .8], [.2 .8], 'Color', 'k', 'LineWidth', 2, 'HeadLength', 16, 'HeadWidth', 16);",
    )
    .unwrap();
    let arrow_png = dir.path().join("arrow.png");
    matrust::export::export_figure(e.figure.as_ref().unwrap(), &arrow_png, 960, 560).unwrap();
    let img = image::open(&arrow_png).unwrap().to_rgb8();

    let count_dark = |cx: u32, cy: u32, r: u32| -> usize {
        let mut n = 0;
        for y in cy.saturating_sub(r)..=(cy + r).min(img.height() - 1) {
            for x in cx.saturating_sub(r)..=(cx + r).min(img.width() - 1) {
                let px = img.get_pixel(x, y);
                if px[0] < 80 && px[1] < 80 && px[2] < 80 {
                    n += 1;
                }
            }
        }
        n
    };

    // Midpoint (.5, .5) -> (480, 280); Tip (.8, .8) -> (768, 112).
    let mid_dark = count_dark(480, 280, 20);
    let tip_dark = count_dark(768, 112, 24);
    let shaft_dark = count_dark(480, 280, 24);
    assert!(
        mid_dark > 0,
        "Expected dark pixels near arrow midpoint, got {mid_dark}"
    );
    assert!(
        tip_dark > shaft_dark,
        "Expected more dark pixels near arrowhead tip ({tip_dark}) than along shaft ({shaft_dark})"
    );

    // 2. Red FaceColor rectangle has red interior.
    let mut e2 = Engine::default();
    e2.execute(
        "figure('Color', 'w'); \
         annotation('rectangle', [.3 .3 .4 .4], 'FaceColor', 'r', 'Color', 'r');",
    )
    .unwrap();
    let rect_png = dir.path().join("rect.png");
    matrust::export::export_figure(e2.figure.as_ref().unwrap(), &rect_png, 960, 560).unwrap();
    let rect_img = image::open(&rect_png).unwrap().to_rgb8();
    let center = rect_img.get_pixel(480, 280);
    assert!(
        center[0] > 200 && center[1] < 50 && center[2] < 50,
        "Expected red interior at rectangle center, got {center:?}"
    );
}
