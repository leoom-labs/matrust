use matrust::{Array, Engine, GraphicsKind};

fn as_bool(a: Array) -> bool {
    match a {
        Array::Logical(m) => m.as_scalar().unwrap(),
        _ => panic!("expected logical scalar"),
    }
}

fn as_text(a: Array) -> String {
    match a {
        Array::Character(c) => String::from_utf16_lossy(c.units()),
        _ => panic!("expected character row"),
    }
}

fn as_vec(a: Array) -> Vec<f64> {
    match a {
        Array::Numeric(m) => m.data().to_vec(),
        _ => panic!("expected numeric matrix"),
    }
}

fn as_handle_kind(a: Array) -> GraphicsKind {
    match a {
        Array::Graphics(g) => g.scalar_handle().unwrap().kind(),
        _ => panic!("expected graphics handle"),
    }
}

#[test]
fn layout_handles_no_output_statements_are_silent() {
    for command in ["subplot(2,1,1)", "tiledlayout(1,2)", "nexttile"] {
        let mut e = Engine::default();
        let result = e.execute(command).unwrap();
        assert!(result.output.is_empty(), "{command}: {:?}", result.output);
        assert!(e.array("ans").is_none(), "{command} set ans");
    }
}

#[test]
fn layout_handles_link_initial_union_and_keep_other_dimension() {
    let mut e = Engine::default();
    e.execute("a=subplot(1,2,1); b=subplot(1,2,2); xlim(a,[1 3]); xlim(b,[1 9]); linkaxes([a b],'x'); x1=xlim(a); x2=xlim(b); linkaxes([a b],'y'); xlim(a,[2 7]); x3=xlim(b);").unwrap();
    assert_eq!(as_vec(e.array("x1").unwrap()), vec![1., 9.]);
    assert_eq!(as_vec(e.array("x2").unwrap()), vec![1., 9.]);
    assert_eq!(as_vec(e.array("x3").unwrap()), vec![2., 7.]);
}

#[test]
fn layout_handles_replot_syncs_linked_limits() {
    let mut e = Engine::default();
    e.execute("a=subplot(1,2,1); plot(a,1:3); b=subplot(1,2,2); plot(b,1:3); linkaxes([a b],'x'); plot(a,1:9); xa=xlim(a); xb=xlim(b);").unwrap();
    assert_eq!(
        as_vec(e.array("xa").unwrap()),
        as_vec(e.array("xb").unwrap())
    );
    assert!(as_vec(e.array("xb").unwrap())[1] >= 9.);
}

#[test]
fn layout_handles_subplot_and_tiledlayout_basics() {
    let mut e = Engine::default();
    e.execute(
        "h=subplot(2,1,1); \
         ok=isgraphics(h); \
         tp=get(h,'Type'); \
         h2=subplot(2,1,2,'align'); \
         hp=subplot('Position',[0.15 0.2 0.7 0.6]);",
    )
    .unwrap();
    assert!(as_bool(e.array("ok").unwrap()));
    assert_eq!(as_text(e.array("tp").unwrap()), "axes");
    assert_eq!(as_handle_kind(e.array("h").unwrap()), GraphicsKind::Axes);
    assert_eq!(as_handle_kind(e.array("h2").unwrap()), GraphicsKind::Axes);
    assert_eq!(as_handle_kind(e.array("hp").unwrap()), GraphicsKind::Axes);

    e.execute(
        "clf; \
         f=gcf; \
         t=tiledlayout(1,2); \
         ok_t=isgraphics(t); \
         tp_t=get(t,'Type'); \
         gs=t.GridSize; \
         ts=t.TileSpacing; \
         pd=t.Padding; \
         par=t.Parent; \
         n0=numel(get(t,'Children')); \
         ax1=nexttile; \
         ax2=nexttile(t); \
         n2=numel(get(t,'Children'));",
    )
    .unwrap();
    assert!(as_bool(e.array("ok_t").unwrap()));
    assert_eq!(as_text(e.array("tp_t").unwrap()), "tiledlayout");
    assert_eq!(as_vec(e.array("gs").unwrap()), vec![1., 2.]);
    assert_eq!(as_text(e.array("ts").unwrap()), "loose");
    assert_eq!(as_text(e.array("pd").unwrap()), "loose");
    assert_eq!(
        as_handle_kind(e.array("par").unwrap()),
        GraphicsKind::Figure
    );
    assert_eq!(as_vec(e.array("n0").unwrap()), vec![0.]);
    assert_eq!(as_vec(e.array("n2").unwrap()), vec![2.]);
    assert_eq!(as_handle_kind(e.array("ax1").unwrap()), GraphicsKind::Axes);
    assert_eq!(as_handle_kind(e.array("ax2").unwrap()), GraphicsKind::Axes);
}

#[test]
fn layout_handles_axes_first_and_shared_labels() {
    let mut e = Engine::default();
    e.execute(
        "h=subplot(2,1,1); \
         other=subplot(2,1,2); \
         ln=plot(h,1:3); \
         p=get(ln,'Parent'); \
         same=isequal(p,h); \
         hold(h,'on'); \
         plot(h,[1 3],[3 1]); \
         title(h,'Top'); \
         xlabel(h,'X'); \
         ylabel(h,'Y'); \
         grid(h,'on'); \
         axis(h,[0 4 0 4]); \
         legend(h,'first','second');",
    )
    .unwrap();
    assert!(as_bool(e.array("same").unwrap()));
    assert_eq!(
        as_handle_kind(e.array("other").unwrap()),
        GraphicsKind::Axes
    );

    e.execute(
        "clf; \
         t=tiledlayout('flow'); \
         a1=nexttile; plot(a1,1:3); \
         a2=nexttile([1 1]); plot(a2,2:4); \
         title(t,'T'); \
         xlabel(t,'Shared X'); \
         ylabel(t,'Shared Y'); \
         s=t.Title.String; \
         sx=t.XLabel.String; \
         sy=t.YLabel.String;",
    )
    .unwrap();
    assert_eq!(as_text(e.array("s").unwrap()), "T");
    assert_eq!(as_text(e.array("sx").unwrap()), "Shared X");
    assert_eq!(as_text(e.array("sy").unwrap()), "Shared Y");
}

#[test]
fn layout_handles_linkaxes_and_clf_reset() {
    let mut e = Engine::default();
    e.execute(
        "a1=subplot(2,1,1); plot(a1,1:4); \
         a2=subplot(2,1,2); plot(a2,1:10); \
         linkaxes([a1 a2],'x'); \
         xlim(a1,[0 5]); \
         x2=xlim(a2); \
         linkaxes([a1 a2],'y'); \
         ylim(a1,[-2 8]); \
         y2=ylim(a2); \
         linkaxes([a1 a2],'off'); \
         xlim(a1,[1 2]); \
         x2_after=xlim(a2);",
    )
    .unwrap();
    assert_eq!(as_vec(e.array("x2").unwrap()), vec![0., 5.]);
    assert_eq!(as_vec(e.array("y2").unwrap()), vec![-2., 8.]);
    assert_eq!(as_vec(e.array("x2_after").unwrap()), vec![0., 5.]);

    // Verify desktop-style zoom propagation via Figure::sync_linked_axes
    e.execute("linkaxes([a1 a2],'xy');").unwrap();
    let fig = e.figure.as_mut().unwrap();
    fig.axes.xlim = Some([10., 20.]);
    fig.axes.ylim = Some([-5., 5.]);
    let active_id = fig.axes.identity.clone();
    fig.sync_linked_axes(&active_id, true, true);
    let (_, a1_axes) = fig
        .all_axes()
        .find(|(_, a)| !a.identity.same_object(&active_id))
        .unwrap();
    assert_eq!(a1_axes.xlim, Some([10., 20.]));
    assert_eq!(a1_axes.ylim, Some([-5., 5.]));

    // Verify clf(h), clf('reset'), and clf(h,'reset')
    e.execute(
        "f=figure; \
         c_default=get(f,'Color'); \
         set(f,'Color',[1 0 0]); \
         plot(1:3); \
         clf(f); \
         c_after_clf=get(f,'Color'); \
         clf('reset'); \
         c_after_reset=get(f,'Color'); \
         set(f,'Color',[0 1 0]); \
         clf(f,'reset'); \
         c_after_h_reset=get(f,'Color');",
    )
    .unwrap();
    let default_color = as_vec(e.array("c_default").unwrap());
    assert_eq!(as_vec(e.array("c_after_clf").unwrap()), vec![1., 0., 0.]);
    assert_eq!(as_vec(e.array("c_after_reset").unwrap()), default_color);
    assert_eq!(as_vec(e.array("c_after_h_reset").unwrap()), default_color);
}

#[cfg(feature = "plot-export")]
#[test]
fn layout_handles_png_renders_shared_title_above_both_tiles() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute(
        "t=tiledlayout(1,2); \
         nexttile(t); plot(1:3); \
         nexttile(t); plot([3 2 1]); \
         title(t,'SHARED TITLE'); \
         exportgraphics(gcf,'layout.png','BackgroundColor','white','Resolution',96); \
         title(t,'XXXXXXXXXXXX'); exportgraphics(gcf,'baseline.png','BackgroundColor','white','Resolution',96);",
    )
    .unwrap();
    let img = image::open(dir.path().join("layout.png"))
        .unwrap()
        .to_rgb8();
    let baseline = image::open(dir.path().join("baseline.png"))
        .unwrap()
        .to_rgb8();
    assert_eq!(img.dimensions(), baseline.dimensions());
    assert!(img.width() > 400 && img.height() > 200);
    // Find the horizontal top borders independently in each half of the image.
    let dark = |x, y| {
        let p = img.get_pixel(x, y);
        p[0] < 220 && p[1] < 220 && p[2] < 220
    };
    let border_y = |start, end| {
        (0..img.height() / 2)
            .find(|&y| (start..end).filter(|&x| dark(x, y)).count() > ((end - start) / 4) as usize)
            .expect("tile top border")
    };
    let left_border = border_y(0, img.width() / 2);
    let right_border = border_y(img.width() / 2, img.width());
    let top_border = left_border.min(right_border);
    let mut title_pixels = 0usize;
    let x0 = img.width() / 4;
    let x1 = 3 * img.width() / 4;
    for y in 0..top_border {
        for x in x0..x1 {
            let p = img.get_pixel(x, y);
            if (p[0] < 230 || p[1] < 230 || p[2] < 230) && p != baseline.get_pixel(x, y) {
                title_pixels += 1;
            }
        }
    }
    assert!(
        title_pixels > 20,
        "Expected rendered shared title pixels above both tiles, found {title_pixels}"
    );
}
