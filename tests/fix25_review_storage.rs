#![cfg(feature = "plot-export")]

use matrust::{Engine, files, plot::LabelStyle};
use resvg::usvg::roxmltree::Document;

fn check_svg_font(svg: &str, label: &str, size: f32, weight: Option<&str>, angle: Option<&str>) {
    let doc = Document::parse(svg).unwrap();
    let text = doc
        .descendants()
        .find(|node| node.has_tag_name("text") && node.text() == Some(label))
        .unwrap_or_else(|| panic!("missing SVG text {label:?}"));
    let attribute = |name| text.ancestors().find_map(|node| node.attribute(name));
    let actual = attribute("font-size").unwrap().parse::<f32>().unwrap();
    assert!(
        (actual - size).abs() < 1e-3,
        "{label}: expected {size}px, got {actual}px"
    );
    assert_eq!(attribute("font-weight"), weight, "{label}");
    assert_eq!(attribute("font-style"), angle, "{label}");
}

#[test]
fn native_figures_roundtrip_axes_and_shared_heading_styles() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = Engine::default();
    engine.set_file_directory(dir.path()).unwrap();
    let axes = "
        plot3(1:3,1:3,1:3);
        title('AxesTitle','FontSize',24,'FontWeight','normal','FontAngle','italic');
        xlabel('AxesX','FontSize',16,'FontWeight','bold','FontAngle','italic');
        ylabel('AxesY','FontSize',11,'FontWeight','bold','FontAngle','normal');
        zlabel('AxesZ','FontSize',14,'FontWeight','bold','FontAngle','italic');
    ";
    engine.execute(axes).unwrap();
    engine
        .execute("savefig('single.mfig'); close all; openfig('single.mfig'); ax=gca;")
        .unwrap();
    engine
        .execute(
            "assert(get(ax.XLabel,'FontSize')==16);
             assert(strcmp(ax.XLabel.FontWeight,'bold'));
             assert(strcmp(ax.XLabel.FontAngle,'italic'));",
        )
        .unwrap();
    let style = |size, bold, italic| LabelStyle {
        font_size: Some(size),
        bold: Some(bold),
        italic: Some(italic),
    };
    let axes_title = style(24.0, false, true);
    let axes_labels = [
        style(16.0, true, true),
        style(11.0, true, false),
        style(14.0, true, true),
    ];
    let figure = engine.figure.as_ref().unwrap();
    assert_eq!(figure.extras.title_style, axes_title);
    assert_eq!(figure.extras.label_styles, axes_labels);

    engine
        .execute("close all; t=tiledlayout(1,2); nexttile;")
        .unwrap();
    engine.execute(axes).unwrap();
    engine
        .execute(
            "nexttile; plot(1:3); xlabel('Unstyled');
             title(t,'SharedTitle','FontSize',21,'FontWeight','normal','FontAngle','italic');
             xlabel(t,'SharedX','FontSize',12,'FontWeight','bold','FontAngle','italic');
             ylabel(t,'SharedY','FontSize',17,'FontWeight','bold','FontAngle','normal');
             savefig('tiled.mfig'); close all; openfig('tiled.mfig');",
        )
        .unwrap();
    let figure = engine.figure.as_ref().unwrap();
    let layout = figure.layout.as_ref().unwrap();
    assert_eq!(layout.title_style, style(21.0, false, true));
    assert_eq!(
        layout.label_styles,
        [style(12.0, true, true), style(17.0, true, false)]
    );
    let (_, styled_tile) = figure
        .all_axes()
        .find(|(_, a)| a.title == "AxesTitle")
        .unwrap();
    assert_eq!(styled_tile.extras.title_style, axes_title);
    assert_eq!(styled_tile.extras.label_styles, axes_labels);
    let (_, unstyled_tile) = figure
        .all_axes()
        .find(|(_, a)| a.xlabel == "Unstyled")
        .unwrap();
    assert_eq!(
        unstyled_tile.extras.label_styles,
        [LabelStyle::default(); 3]
    );
}

#[test]
fn factory_text_size_keeps_the_legacy_export_size() {
    let mut engine = Engine::default();
    engine
        .execute(
            "set(0,'DefaultTextFontSize',10); plot(1:3); text(2,2,'ExplicitDefault');
             set(0,'DefaultTextFontSize','factory'); t=text(2,2,'FactoryDefault');
             assert(t.FontSize==10); assert(get(0,'DefaultTextFontSize')==10);
             d=get(0,'default'); assert(d.defaultTextFontSize==10);
             set(0,'DefaultTextFontSize',18); text(2,2,'NumericRoot');
             f=gcf; set(f,'DefaultTextFontSize','factory'); text(2,2,'FigureFactory');
             assert(get(f,'DefaultTextFontSize')==10);
             set(f,'DefaultTextFontSize','remove'); text(2,2,'RestoredRoot');",
        )
        .unwrap();
    let svg = engine.figure.as_ref().unwrap().svg();
    check_svg_font(&svg, "ExplicitDefault", 17.0, Some("normal"), None);
    check_svg_font(
        &svg,
        "FactoryDefault",
        10.0 * 4.0 / 3.0,
        Some("normal"),
        None,
    );
    check_svg_font(&svg, "NumericRoot", 30.6, Some("normal"), None);
    check_svg_font(
        &svg,
        "FigureFactory",
        10.0 * 4.0 / 3.0,
        Some("normal"),
        None,
    );
    check_svg_font(&svg, "RestoredRoot", 30.6, Some("normal"), None);
}

#[test]
fn right_ylabel_accepts_font_styles_in_all_setter_forms() {
    let mut engine = Engine::default();
    engine
        .execute("yyaxis left; plot(1:3); yyaxis right; plot(2:4); ylabel('DefaultRight');")
        .unwrap();
    let svg = engine.figure.as_ref().unwrap().svg();
    check_svg_font(&svg, "DefaultRight", 16.0, None, None);

    engine
        .execute(
            "h=ylabel('StyledRight','FontSize',16,'FontWeight','bold','FontAngle','italic');
             assert(get(h,'FontSize')==16);
             assert(strcmp(get(h,'FontWeight'),'bold'));
             assert(strcmp(get(h,'FontAngle'),'italic'));",
        )
        .unwrap();
    let svg = engine.figure.as_ref().unwrap().svg();
    check_svg_font(&svg, "StyledRight", 27.2, Some("bold"), Some("italic"));

    engine
        .execute(
            "set(h,'FontSize',18,'FontWeight','normal','FontAngle','normal');
             assert(h.FontSize==18);
             assert(strcmp(h.FontWeight,'normal')); assert(strcmp(h.FontAngle,'normal'));
             h.FontSize=20; h.FontWeight='bold'; h.FontAngle='italic';
             ax=gca; assert(ax.YLabel.FontSize==20);
             assert(strcmp(h.FontWeight,'bold')); assert(strcmp(h.FontAngle,'italic'));",
        )
        .unwrap();
    let svg = engine.figure.as_ref().unwrap().svg();
    check_svg_font(&svg, "StyledRight", 34.0, Some("bold"), Some("italic"));
    assert!(
        engine
            .execute("ylabel('Invalid','FontWeight','normal','FontSize',0);")
            .is_err()
    );
    assert_eq!(engine.figure.as_ref().unwrap().svg(), svg);
}

#[test]
fn replacement_legend_on_yyaxis_starts_with_default_style() {
    let mut engine = Engine::default();
    engine
        .execute(
            "yyaxis left; plot(1:3); h=legend('OldLegend','FontSize',30, ...
                                            'FontWeight','bold','FontAngle','italic');
             plot(2:4); l=legend('FreshLegend');
             assert(~isgraphics(h)); assert(l.FontSize==9);
             assert(strcmp(l.FontWeight,'normal')); assert(strcmp(l.FontAngle,'normal'));",
        )
        .unwrap();
    let svg = engine.figure.as_ref().unwrap().svg();
    check_svg_font(&svg, "FreshLegend", 12.0, None, None);
}

#[test]
fn tiled_label_style_edits_change_the_figure_snapshot() {
    let mut engine = Engine::default();
    engine
        .execute("t=tiledlayout(1,1); nexttile; plot(1:3); h=xlabel(t,'SharedX');")
        .unwrap();
    for edit in [
        "h.FontSize=24;",
        "h.FontWeight='bold';",
        "h.FontAngle='italic';",
    ] {
        let saved = engine.figure.clone().unwrap();
        engine.execute(edit).unwrap();
        assert!(
            !saved.same_snapshot(engine.figure.as_ref().unwrap()),
            "{edit}"
        );
    }
}

#[test]
fn line_handles_do_not_advertise_font_weight_or_angle() {
    let mut engine = Engine::default();
    engine
        .execute(
            "h=plot(1:3); t=title('Title'); l=legend('Legend');
             assert(~isprop(h,'FontAngle')); assert(~isprop(h,'FontWeight'));
             assert(isprop(t,'FontAngle')); assert(isprop(t,'FontWeight'));
             assert(isprop(l,'FontAngle')); assert(isprop(l,'FontWeight'));",
        )
        .unwrap();
}

#[test]
fn old_native_text_infers_explicit_size_only_for_nondefault_values() {
    let mut engine = Engine::default();
    engine
        .execute(
            "plot(1:3); text(2,2,'OldCustom','FontSize',18); text(2,2,'OldDefault');
             text(2,2,'NewExplicitDefault','FontSize',10); text(2,2,'NewImplicitCustom','FontSize',18);",
        )
        .unwrap();
    // Old native metadata stored the point size without an explicitness flag.
    let bytes = files::encode_native_figure(engine.figure.as_ref().unwrap()).unwrap();
    let metadata_end = 12 + u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let mut metadata: serde_json::Value = serde_json::from_slice(&bytes[12..metadata_end]).unwrap();
    for text in &mut metadata["texts"].as_array_mut().unwrap()[..2] {
        text.as_object_mut().unwrap().remove("font_size_explicit");
    }
    metadata["texts"][3]["font_size_explicit"] = false.into();
    let metadata = serde_json::to_vec(&metadata).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut old_bytes = bytes[..8].to_vec();
    old_bytes.extend_from_slice(&(metadata.len() as u32).to_le_bytes());
    old_bytes.extend_from_slice(&metadata);
    old_bytes.extend_from_slice(&bytes[metadata_end..]);
    let path = dir.path().join("legacy.mfig");
    std::fs::write(&path, old_bytes).unwrap();
    let figure = files::load_figure(&path).unwrap();
    let svg = figure.svg();
    check_svg_font(&svg, "OldCustom", 30.6, Some("normal"), None);
    check_svg_font(&svg, "OldDefault", 10.0 * 4.0 / 3.0, Some("normal"), None);
    check_svg_font(&svg, "NewExplicitDefault", 17.0, Some("normal"), None);
    check_svg_font(&svg, "NewImplicitCustom", 24.0, Some("normal"), None);
}
