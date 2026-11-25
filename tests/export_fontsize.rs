#![cfg(feature = "plot-export")]

use matrust::{Engine, export};
use resvg::usvg::{FontStyle, Group, Node, Tree};

struct ParsedSpan {
    text: String,
    font_size: f32,
    weight: u16,
    style: FontStyle,
}

fn parsed_spans(svg: &str) -> Vec<ParsedSpan> {
    fn visit(group: &Group, out: &mut Vec<ParsedSpan>) {
        for node in group.children() {
            match node {
                Node::Group(g) => visit(g, out),
                Node::Text(text) => {
                    for chunk in text.chunks() {
                        if !chunk.text().is_empty() {
                            for span in chunk.spans() {
                                out.push(ParsedSpan {
                                    text: chunk.text().to_owned(),
                                    font_size: span.font_size().get(),
                                    weight: span.font().weight(),
                                    style: span.font().style(),
                                });
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let tree = Tree::from_str(svg, &export::svg_render_options()).unwrap();
    let mut out = Vec::new();
    visit(tree.root(), &mut out);
    out
}

fn find_span<'a>(spans: &'a [ParsedSpan], label: &str) -> &'a ParsedSpan {
    spans
        .iter()
        .find(|s| s.text == label)
        .unwrap_or_else(|| panic!("missing text {label:?}"))
}

fn assert_approx(actual: f32, expected: f32, label: &str) {
    assert!(
        (actual - expected).abs() < 1e-3,
        "{label}: expected {expected}, got {actual}"
    );
}

fn scalar_var(engine: &Engine, name: &str) -> f64 {
    engine.variables[name].data()[0]
}

#[test]
fn title_export_fontsize_and_readback() {
    let mut engine = Engine::default();
    engine.execute("plot(1:3); title('DefaultTitle');").unwrap();
    let default_svg = engine.figure.as_ref().unwrap().svg();
    let default_spans = parsed_spans(&default_svg);
    assert_approx(
        find_span(&default_spans, "DefaultTitle").font_size,
        20.0,
        "default title",
    );

    let mut engine = Engine::default();
    engine
        .execute("plot(1:3); t=title('T','FontSize',24);")
        .unwrap();
    let svg = engine.figure.as_ref().unwrap().svg();
    let spans = parsed_spans(&svg);
    assert_approx(find_span(&spans, "T").font_size, 40.8, "explicit title");

    engine
        .execute(
            "g=get(t,'FontSize'); d=t.FontSize; \
             set(t,'FontSize',20); g2=get(t,'FontSize'); t.FontSize=24; d2=t.FontSize;",
        )
        .unwrap();
    assert_eq!(scalar_var(&engine, "g"), 24.0);
    assert_eq!(scalar_var(&engine, "d"), 24.0);
    assert_eq!(scalar_var(&engine, "g2"), 20.0);
    assert_eq!(scalar_var(&engine, "d2"), 24.0);
}

#[test]
fn xlabel_export_fontsize_and_readback() {
    let mut engine = Engine::default();
    engine.execute("plot(1:3); xlabel('DefaultX');").unwrap();
    let default_svg = engine.figure.as_ref().unwrap().svg();
    let default_spans = parsed_spans(&default_svg);
    assert_approx(
        find_span(&default_spans, "DefaultX").font_size,
        16.0,
        "default xlabel",
    );

    let mut engine = Engine::default();
    engine
        .execute("plot(1:3); xl=xlabel('X','FontSize',16);")
        .unwrap();
    let svg = engine.figure.as_ref().unwrap().svg();
    let spans = parsed_spans(&svg);
    assert_approx(find_span(&spans, "X").font_size, 27.2, "explicit xlabel");

    engine
        .execute(
            "g=get(xl,'FontSize'); d=xl.FontSize; \
             set(xl,'FontSize',14); g2=get(xl,'FontSize'); xl.FontSize=16; d2=xl.FontSize;",
        )
        .unwrap();
    assert_eq!(scalar_var(&engine, "g"), 16.0);
    assert_eq!(scalar_var(&engine, "d"), 16.0);
    assert_eq!(scalar_var(&engine, "g2"), 14.0);
    assert_eq!(scalar_var(&engine, "d2"), 16.0);
}

#[test]
fn ylabel_export_fontsize_and_readback() {
    let mut engine = Engine::default();
    engine.execute("plot(1:3); ylabel('DefaultY');").unwrap();
    let default_svg = engine.figure.as_ref().unwrap().svg();
    let default_spans = parsed_spans(&default_svg);
    assert_approx(
        find_span(&default_spans, "DefaultY").font_size,
        16.0,
        "default ylabel",
    );

    let mut engine = Engine::default();
    engine
        .execute("plot(1:3); yl=ylabel('Y','FontSize',15);")
        .unwrap();
    let svg = engine.figure.as_ref().unwrap().svg();
    let spans = parsed_spans(&svg);
    assert_approx(find_span(&spans, "Y").font_size, 25.5, "explicit ylabel");

    engine
        .execute(
            "g=get(yl,'FontSize'); d=yl.FontSize; \
             set(yl,'FontSize',12); g2=get(yl,'FontSize'); yl.FontSize=15; d2=yl.FontSize;",
        )
        .unwrap();
    assert_eq!(scalar_var(&engine, "g"), 15.0);
    assert_eq!(scalar_var(&engine, "d"), 15.0);
    assert_eq!(scalar_var(&engine, "g2"), 12.0);
    assert_eq!(scalar_var(&engine, "d2"), 15.0);
}

#[test]
fn zlabel_export_fontsize_and_readback() {
    let mut engine = Engine::default();
    engine
        .execute("plot3(1:3,1:3,1:3); zlabel('DefaultZ');")
        .unwrap();
    let default_svg = engine.figure.as_ref().unwrap().svg();
    let default_spans = parsed_spans(&default_svg);
    assert_approx(
        find_span(&default_spans, "DefaultZ").font_size,
        11.0 * 4.0 / 3.0,
        "default zlabel",
    );

    let mut engine = Engine::default();
    engine
        .execute("plot3(1:3,1:3,1:3); zl=zlabel('Z','FontSize',14);")
        .unwrap();
    let svg = engine.figure.as_ref().unwrap().svg();
    let spans = parsed_spans(&svg);
    assert_approx(find_span(&spans, "Z").font_size, 23.8, "explicit zlabel");

    engine
        .execute(
            "g=get(zl,'FontSize'); d=zl.FontSize; \
             set(zl,'FontSize',18); g2=get(zl,'FontSize'); zl.FontSize=14; d2=zl.FontSize;",
        )
        .unwrap();
    assert_eq!(scalar_var(&engine, "g"), 14.0);
    assert_eq!(scalar_var(&engine, "d"), 14.0);
    assert_eq!(scalar_var(&engine, "g2"), 18.0);
    assert_eq!(scalar_var(&engine, "d2"), 14.0);
}

#[test]
fn subtitle_export_fontsize_and_readback() {
    let mut engine = Engine::default();
    engine
        .execute("plot(1:3); subtitle('DefaultSub');")
        .unwrap();
    let default_svg = engine.figure.as_ref().unwrap().svg();
    let default_spans = parsed_spans(&default_svg);
    assert_approx(
        find_span(&default_spans, "DefaultSub").font_size,
        17.0,
        "default subtitle",
    );

    let mut engine = Engine::default();
    engine
        .execute("plot(1:3); s=subtitle('Sub','FontSize',12);")
        .unwrap();
    let svg = engine.figure.as_ref().unwrap().svg();
    let spans = parsed_spans(&svg);
    assert_approx(
        find_span(&spans, "Sub").font_size,
        20.4,
        "explicit subtitle",
    );

    engine
        .execute(
            "g=get(s,'FontSize'); d=s.FontSize; \
             set(s,'FontSize',15); g2=get(s,'FontSize'); s.FontSize=12; d2=s.FontSize;",
        )
        .unwrap();
    assert_eq!(scalar_var(&engine, "g"), 12.0);
    assert_eq!(scalar_var(&engine, "d"), 12.0);
    assert_eq!(scalar_var(&engine, "g2"), 15.0);
    assert_eq!(scalar_var(&engine, "d2"), 12.0);
}

#[test]
fn text_object_export_fontsize_and_readback() {
    let mut engine = Engine::default();
    engine
        .execute("plot(1:3); text(2,2,'DefaultTxt');")
        .unwrap();
    let default_svg = engine.figure.as_ref().unwrap().svg();
    let default_spans = parsed_spans(&default_svg);
    assert_approx(
        find_span(&default_spans, "DefaultTxt").font_size,
        10.0 * 4.0 / 3.0,
        "default text",
    );

    let mut engine = Engine::default();
    engine
        .execute("plot(1:3); tx=text(2,2,'Txt','FontSize',18);")
        .unwrap();
    let svg = engine.figure.as_ref().unwrap().svg();
    let spans = parsed_spans(&svg);
    assert_approx(find_span(&spans, "Txt").font_size, 30.6, "explicit text");

    engine
        .execute(
            "g=get(tx,'FontSize'); d=tx.FontSize; \
             set(tx,'FontSize',20); g2=get(tx,'FontSize'); tx.FontSize=18; d2=tx.FontSize;",
        )
        .unwrap();
    assert_eq!(scalar_var(&engine, "g"), 18.0);
    assert_eq!(scalar_var(&engine, "d"), 18.0);
    assert_eq!(scalar_var(&engine, "g2"), 20.0);
    assert_eq!(scalar_var(&engine, "d2"), 18.0);
}

#[test]
fn legend_export_fontsize_and_readback() {
    let mut engine = Engine::default();
    engine.execute("plot(1:3); legend('DefaultLeg');").unwrap();
    let default_svg = engine.figure.as_ref().unwrap().svg();
    let default_spans = parsed_spans(&default_svg);
    assert_approx(
        find_span(&default_spans, "DefaultLeg").font_size,
        12.0,
        "default legend",
    );

    let mut engine = Engine::default();
    engine
        .execute("plot(1:3); lg=legend('a','FontSize',14);")
        .unwrap();
    let svg = engine.figure.as_ref().unwrap().svg();
    let spans = parsed_spans(&svg);
    assert_approx(find_span(&spans, "a").font_size, 23.8, "explicit legend");

    engine
        .execute(
            "g=get(lg,'FontSize'); d=lg.FontSize; \
             set(lg,'FontSize',16); g2=get(lg,'FontSize'); lg.FontSize=14; d2=lg.FontSize;",
        )
        .unwrap();
    assert_eq!(scalar_var(&engine, "g"), 14.0);
    assert_eq!(scalar_var(&engine, "d"), 14.0);
    assert_eq!(scalar_var(&engine, "g2"), 16.0);
    assert_eq!(scalar_var(&engine, "d2"), 14.0);
}

#[test]
fn bold_and_italic_export_and_readback() {
    let mut engine = Engine::default();
    engine
        .execute(
            "plot(1:3); \
             t=title('TitleBI','FontWeight','bold','FontAngle','italic'); \
             xl=xlabel('XBI','FontWeight','bold','FontAngle','italic'); \
             yl=ylabel('YBI','FontWeight','bold','FontAngle','italic'); \
             s=subtitle('SubBI','FontWeight','bold','FontAngle','italic'); \
             tx=text(2,2,'TextBI','FontWeight','bold','FontAngle','italic'); \
             lg=legend('LegBI','FontWeight','bold','FontAngle','italic'); \
             tw=double(strcmp(t.FontWeight,'bold')); ta=double(strcmp(t.FontAngle,'italic')); \
             xw=double(strcmp(get(xl,'FontWeight'),'bold')); xa=double(strcmp(get(xl,'FontAngle'),'italic')); \
             lw=double(strcmp(lg.FontWeight,'bold')); la=double(strcmp(get(lg,'FontAngle'),'italic'));",
        )
        .unwrap();
    let svg = engine.figure.as_ref().unwrap().svg();
    let spans = parsed_spans(&svg);
    for label in ["TitleBI", "XBI", "YBI", "SubBI", "TextBI", "LegBI"] {
        let span = find_span(&spans, label);
        assert!(span.weight >= 700, "{label} weight {}", span.weight);
        assert_eq!(span.style, FontStyle::Italic, "{label} style");
    }
    assert_eq!(scalar_var(&engine, "tw"), 1.0);
    assert_eq!(scalar_var(&engine, "ta"), 1.0);
    assert_eq!(scalar_var(&engine, "xw"), 1.0);
    assert_eq!(scalar_var(&engine, "xa"), 1.0);
    assert_eq!(scalar_var(&engine, "lw"), 1.0);
    assert_eq!(scalar_var(&engine, "la"), 1.0);

    let mut engine3d = Engine::default();
    engine3d
        .execute(
            "plot3(1:3,1:3,1:3); \
             zl=zlabel('Z','FontWeight','bold','FontAngle','italic'); \
             zw=double(strcmp(zl.FontWeight,'bold')); za=double(strcmp(get(zl,'FontAngle'),'italic'));",
        )
        .unwrap();
    let svg3d = engine3d.figure.as_ref().unwrap().svg();
    let spans3d = parsed_spans(&svg3d);
    let zspan = find_span(&spans3d, "Z");
    assert!(zspan.weight >= 700, "Z weight {}", zspan.weight);
    assert_eq!(zspan.style, FontStyle::Italic, "Z style");
    assert_eq!(scalar_var(&engine3d, "zw"), 1.0);
    assert_eq!(scalar_var(&engine3d, "za"), 1.0);
}

fn text_bbox(svg: &str, label: &str) -> resvg::usvg::Rect {
    fn visit(group: &Group, label: &str) -> Option<resvg::usvg::Rect> {
        for node in group.children() {
            match node {
                Node::Group(g) => {
                    if let Some(r) = visit(g, label) {
                        return Some(r);
                    }
                }
                Node::Text(text) if text.chunks().iter().any(|c| c.text() == label) => {
                    return Some(text.abs_bounding_box());
                }
                _ => {}
            }
        }
        None
    }
    let tree = Tree::from_str(svg, &export::svg_render_options()).unwrap();
    visit(tree.root(), label).unwrap_or_else(|| panic!("missing text {label:?}"))
}

#[test]
fn empty_title_fontsize_preserves_subtitle_baseline() {
    let mut baseline_eng = Engine::default();
    baseline_eng.execute("plot(1:3); subtitle('Sub');").unwrap();
    let baseline_svg = baseline_eng.figure.as_ref().unwrap().svg();

    let mut eng = Engine::default();
    eng.execute("plot(1:3); title('','FontSize',40); subtitle('Sub');")
        .unwrap();
    let svg = eng.figure.as_ref().unwrap().svg();
    assert!(
        svg.contains(r#"y="29.6"#),
        "expected subtitle at y=29.60 in {svg}"
    );
    assert_eq!(
        text_bbox(&svg, "Sub"),
        text_bbox(&baseline_svg, "Sub"),
        "subtitle bbox should match no-title baseline"
    );
}

#[test]
fn large_fontsize_labels_and_tiled_headings_fit_inside_canvas() {
    let mut eng = Engine::default();
    eng.execute(
        "plot(1:3); title('BigTitle','FontSize',40); \
         xlabel('BigX','FontSize',40); ylabel('BigY','FontSize',40);",
    )
    .unwrap();
    let svg = eng.figure.as_ref().unwrap().svg();
    let tree = Tree::from_str(&svg, &export::svg_render_options()).unwrap();
    let size = tree.size();
    for label in ["BigTitle", "BigX", "BigY"] {
        let b = text_bbox(&svg, label);
        assert!(
            b.left() >= 0.0
                && b.top() >= 0.0
                && b.right() <= size.width()
                && b.bottom() <= size.height(),
            "{label} bbox {b:?} outside canvas {size:?}"
        );
    }

    let mut eng3d = Engine::default();
    eng3d
        .execute("plot3(1:3,1:3,1:3); zlabel('BigZ','FontSize',40);")
        .unwrap();
    let svg3d = eng3d.figure.as_ref().unwrap().svg();
    let tree3d = Tree::from_str(&svg3d, &export::svg_render_options()).unwrap();
    let size3d = tree3d.size();
    let bz = text_bbox(&svg3d, "BigZ");
    assert!(
        bz.left() >= 0.0
            && bz.top() >= 0.0
            && bz.right() <= size3d.width()
            && bz.bottom() <= size3d.height(),
        "BigZ bbox {bz:?} outside canvas {size3d:?}"
    );

    let mut tiled = Engine::default();
    tiled
        .execute(
            "tl=tiledlayout(1,2); nexttile; plot(1:3); nexttile; plot(1:3); \
             title(tl,'SharedT','FontSize',40); \
             xlabel(tl,'SharedX','FontSize',40); \
             ylabel(tl,'SharedY','FontSize',40);",
        )
        .unwrap();
    let tiled_svg = tiled.figure.as_ref().unwrap().svg();
    let tiled_tree = Tree::from_str(&tiled_svg, &export::svg_render_options()).unwrap();
    let tiled_size = tiled_tree.size();
    for label in ["SharedT", "SharedX", "SharedY"] {
        let b = text_bbox(&tiled_svg, label);
        assert!(
            b.left() >= 0.0
                && b.top() >= 0.0
                && b.right() <= tiled_size.width()
                && b.bottom() <= tiled_size.height(),
            "{label} bbox {b:?} outside tiled canvas {tiled_size:?}"
        );
    }
}

#[test]
fn large_legend_fontsize_grows_row_pitch_and_clip() {
    let mut eng = Engine::default();
    eng.execute("plot(1:3,1:3,1:3,3:-1:1); legend('First','Second','FontSize',40);")
        .unwrap();
    let svg = eng.figure.as_ref().unwrap().svg();
    let b1 = text_bbox(&svg, "First");
    let b2 = text_bbox(&svg, "Second");
    assert!(
        b2.top() >= b1.bottom(),
        "legend rows overlap: first={b1:?}, second={b2:?}"
    );
    assert!(
        b1.height() > 30.0 && b2.height() > 30.0,
        "legend text clipped: first={b1:?}, second={b2:?}"
    );
}

#[test]
fn default_svg_coordinates_match_baseline() {
    let mut eng = Engine::default();
    eng.execute("plot(1:3); title('T'); xlabel('X'); ylabel('Y'); subtitle('S'); legend('L');")
        .unwrap();
    let svg = eng.figure.as_ref().unwrap().svg();
    assert!(
        svg.contains(r#"<rect x="70" y="81" width="850" height="409" fill="white"/>"#),
        "{svg}"
    );
    assert!(
        svg.contains(r#"<text x="70" y="32" font-size="20">T</text>"#),
        "{svg}"
    );
    assert!(svg.contains(r##"<text x="495" y="53.60000038146973" text-anchor="middle" font-family="Arial" font-size="17" font-weight="normal" fill="#242424">S</text>"##), "{svg}");
    assert!(
        svg.contains(r#"<text x="480" y="546" text-anchor="middle">X</text>"#),
        "{svg}"
    );
    assert!(
        svg.contains(
            r#"<text transform="translate(18,280) rotate(-90)" text-anchor="middle">Y</text>"#
        ),
        "{svg}"
    );
    assert!(
        svg.contains(
            r##"<rect x="700" y="91" width="220" height="34" fill="white" stroke="#aaa"/>"##
        ),
        "{svg}"
    );
    assert!(
        svg.contains(r#"<rect x="742" y="97" width="168" height="22"/>"#),
        "{svg}"
    );
}
