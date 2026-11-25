use matrust::plot::text_markup::Interpreter;
use matrust::plot::{TilePadding, TileSpacing};
use matrust::{Array, Engine, files};

fn quote(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\'', "''")
}

fn vec_val(a: &Array) -> Vec<f64> {
    match a {
        Array::Numeric(m) => m.data().to_vec(),
        _ => panic!("expected numeric array, got {a:?}"),
    }
}

#[test]
fn fix16_review_native_subplot_shapes_survive_save_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.mrfig");
    let mut e = Engine::default();
    e.execute(&format!(
        "subplot(1,2,1); xline(2); \
         subplot(1,2,2); rectangle('Position',[1 1 2 3]); \
         savefig('{}');",
        quote(&path)
    ))
    .unwrap();

    let mut e2 = Engine::default();
    e2.execute(&format!("openfig('{}');", quote(&path)))
        .unwrap();
    let fig = e2.figure.as_ref().unwrap();
    let mut panels: Vec<_> = fig.all_axes().collect();
    panels.sort_by_key(|(p, _)| *p);
    assert_eq!(panels.len(), 2);
    assert_eq!(panels[0].1.extras.constant_lines.len(), 1);
    assert!((panels[0].1.extras.constant_lines[0].value - 2.0).abs() < 1e-12);
    assert_eq!(panels[1].1.extras.rectangles.len(), 1);
    assert_eq!(
        panels[1].1.extras.rectangles[0].position,
        [1.0, 1.0, 2.0, 3.0]
    );
}

#[test]
fn fix16_review_native_tiledlayout_labels_spacing_padding_and_linkaxes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("layout.mrfig");
    let mut e = Engine::default();
    e.execute(&format!(
        "t = tiledlayout(1,2); \
         t.TileSpacing = 'compact'; t.Padding = 'tight'; \
         a = nexttile; plot(a, 1:3); \
         b = nexttile; plot(b, 1:5); \
         title(t, 'T_0'); \
         xlabel(t, 'Time_1', 'Interpreter', 'none'); \
         ylabel(t, 'Amp^2', 'Interpreter', 'latex'); \
         linkaxes([a b], 'x'); \
         savefig('{}');",
        quote(&path)
    ))
    .unwrap();

    let mut e2 = Engine::default();
    e2.execute(&format!("openfig('{}');", quote(&path)))
        .unwrap();
    {
        let fig = e2.figure.as_ref().unwrap();
        let layout = fig.layout.as_ref().unwrap();
        assert_eq!(layout.title, "T_0");
        assert_eq!(layout.xlabel, "Time_1");
        assert_eq!(layout.ylabel, "Amp^2");
        assert_eq!(layout.text_interpreters[1], Interpreter::None);
        assert_eq!(layout.text_interpreters[2], Interpreter::Latex);
        assert_eq!(layout.tile_spacing, TileSpacing::Compact);
        assert_eq!(layout.padding, TilePadding::Tight);
        assert_eq!(layout.linked_axes.len(), 1);
        assert!(layout.linked_axes[0].x);
        assert!(!layout.linked_axes[0].y);
    }

    // Propagate limit edits after reopen using live handles.
    e2.execute(
        "f = gcf; ch = f.Children; \
         ax1 = ch(1); ax2 = ch(2); \
         xlim(ax1, [2 4]); \
         x1 = ax1.XLim; x2 = ax2.XLim;",
    )
    .unwrap();
    let ws = e2.workspace();
    assert_eq!(vec_val(&ws["x1"]), vec![2.0, 4.0]);
    assert_eq!(vec_val(&ws["x2"]), vec![2.0, 4.0]);
}

#[test]
fn fix16_review_native_stacked_bar_groups_stay_independent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bars.mrfig");
    let mut e = Engine::default();
    e.execute(&format!(
        "bar([1 2], 'stacked'); hold on; bar([3 4], 'stacked'); savefig('{}');",
        quote(&path)
    ))
    .unwrap();

    let mut e2 = Engine::default();
    e2.execute(&format!("openfig('{}');", quote(&path)))
        .unwrap();
    let fig = e2.figure.as_ref().unwrap();
    assert_eq!(fig.axes.series.len(), 2);
    let y_axis = fig.axes.axis_transforms()[1];
    let span0_0 = fig.axes.bar_y_span(0, 0, y_axis).unwrap().unwrap();
    let span1_0 = fig.axes.bar_y_span(1, 0, y_axis).unwrap().unwrap();
    let span1_1 = fig.axes.bar_y_span(1, 1, y_axis).unwrap().unwrap();
    assert!((span0_0[0] - 0.0).abs() < 1e-12 && (span0_0[1] - 1.0).abs() < 1e-12);
    assert!((span1_0[0] - 0.0).abs() < 1e-12 && (span1_0[1] - 3.0).abs() < 1e-12);
    assert!((span1_1[0] - 0.0).abs() < 1e-12 && (span1_1[1] - 4.0).abs() < 1e-12);
}

#[test]
fn fix16_review_native_size_cap_rejects_before_replacement() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.mrfig");
    let mut e = Engine::default();
    e.execute(&format!("plot(1:3); savefig('{}');", quote(&path)))
        .unwrap();
    let original_bytes = std::fs::read(&path).unwrap();

    let mut e_scatter = Engine::default();
    e_scatter
        .execute("n = 1000; scatter(1:n, 1:n, ones(1, n));")
        .unwrap();
    let scatter_fig = e_scatter.figure.as_ref().unwrap();
    let encoded_len = files::encode_native_figure(scatter_fig).unwrap().len();
    assert!(encoded_len > 20_000);

    // Just over the cap: save must refuse with a clear error before touching the existing file.
    let cap = encoded_len - 1;
    let err = files::save_figure_with_limit(&path, scatter_fig, cap).unwrap_err();
    assert!(err.contains("save limit"), "{err}");
    assert_eq!(std::fs::read(&path).unwrap(), original_bytes);
}

#[test]
fn fix16_review_native_text_and_colororder_survive_save_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("text_co.mrfig");
    let mut e = Engine::default();
    e.execute(&format!(
        "plot(1:2); hold on; colororder([1 0 0]); \
         text(1, 1, 'x_1', 'Interpreter', 'none'); \
         savefig('{}');",
        quote(&path)
    ))
    .unwrap();

    let mut e2 = Engine::default();
    e2.execute(&format!("openfig('{}');", quote(&path)))
        .unwrap();
    {
        let fig = e2.figure.as_ref().unwrap();
        assert_eq!(fig.axes.extras.texts.len(), 1);
        let t = &fig.axes.extras.texts[0];
        assert_eq!(t.string, "x_1");
        assert_eq!(t.interpreter, Interpreter::None);
        assert_eq!(t.position, [1.0, 1.0, 0.0]);
        assert_eq!(
            fig.axes.extras.color_order.as_deref(),
            Some(&vec![[1.0, 0.0, 0.0]])
        );
    }

    // Next held plot uses the preserved colororder, and the text object remains editable.
    e2.execute(
        "plot(1:2); \
         ax = gca; ch = ax.Children; \
         txt = ch(1); txt.String = 'edited_2';",
    )
    .unwrap();
    let fig = e2.figure.as_ref().unwrap();
    assert_eq!(fig.axes.series.len(), 2);
    assert_eq!(fig.axes.series[1].color, [255, 0, 0]);
    assert_eq!(fig.axes.extras.texts[0].string, "edited_2");
}
