use matrust::Engine;

#[cfg(feature = "plot-export")]
use resvg::usvg::{self, roxmltree::Document, roxmltree::Node};

#[cfg(feature = "plot-export")]
fn attr(node: Node<'_, '_>, key: &str) -> f64 {
    node.attribute(key).unwrap().parse().unwrap()
}

#[cfg(feature = "plot-export")]
fn glyph<'a, 'input>(doc: &'a Document<'input>, text: &str) -> Node<'a, 'input> {
    doc.descendants()
        .find(|node| node.has_tag_name("text") && node.text() == Some(text))
        .unwrap_or_else(|| panic!("missing glyph {text:?}"))
}

#[cfg(feature = "plot-export")]
fn glyph_ink(glyph: Node<'_, '_>) -> usvg::Rect {
    let text = &glyph.document().input_text()[glyph.range()];
    let svg =
        format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1">{text}</svg>"#);
    let tree = usvg::Tree::from_str(&svg, &matrust::export::svg_render_options()).unwrap();
    tree.root().children()[0].abs_stroke_bounding_box()
}

#[cfg(feature = "plot-export")]
#[test]
fn export_latex_subtitle_fraction_and_root_rules_in_each_text_role() {
    let math = r"$\frac{a}{bbbbbbbb}+\sqrt{x_1^2}$";
    for (command, base_size) in [
        ("xlabel(", 16.),
        ("ylabel(", 16.),
        ("title(", 20.),
        ("text(2,2,", 10. * 4. / 3.),
    ] {
        let mut engine = Engine::default();
        engine
            .execute(&format!(
                "plot(1:3); {command}'{math}','Interpreter','latex');"
            ))
            .unwrap();
        let svg = engine.figure.as_ref().unwrap().svg();
        let doc = Document::parse(&svg).unwrap();
        let group = doc
            .descendants()
            .find(|node| node.has_tag_name("text") && node.text() == Some("bbbbbbbb"))
            .unwrap_or_else(|| panic!("{command} missing denominator"))
            .parent()
            .unwrap();
        let operand = |text| {
            group
                .children()
                .find(|node| node.has_tag_name("text") && node.text() == Some(text))
                .unwrap_or_else(|| panic!("{command} missing glyph {text:?}"))
        };
        let num = operand("a");
        let den = operand("bbbbbbbb");
        let root = operand("√");
        let body = operand("x");
        let sub = operand("1");
        let sup = operand("2");
        let rules: Vec<_> = group
            .children()
            .filter(|node| node.has_tag_name("line"))
            .collect();
        assert_eq!(rules.len(), 2, "{command}");
        let [bar, overline] = [rules[0], rules[1]];
        for line in rules {
            assert_eq!(attr(line, "y1"), attr(line, "y2"), "{command}");
            assert!(attr(line, "stroke-width") > 0., "{command}");
        }
        assert_eq!(attr(num, "font-size"), attr(den, "font-size"), "{command}");
        assert!(
            (attr(den, "font-size") - base_size * 0.7).abs() < 0.001,
            "{command}"
        );
        assert!(
            attr(num, "y") < attr(bar, "y1") && attr(bar, "y1") < attr(den, "y"),
            "{command}"
        );
        let denominator = glyph_ink(den);
        assert!(
            attr(bar, "x1") <= f64::from(denominator.left()),
            "{command}"
        );
        assert!(
            attr(bar, "x2") >= f64::from(denominator.right()),
            "{command}"
        );
        assert!(attr(root, "x") < attr(body, "x"), "{command}");
        assert!(
            attr(overline, "y1") < attr(body, "y") - 0.8 * attr(body, "font-size"),
            "{command}"
        );
        assert!(attr(overline, "x1") <= attr(body, "x"), "{command}");
        assert!(
            attr(overline, "x2") >= f64::from(glyph_ink(sup).right()),
            "{command}"
        );
        assert!(
            attr(sub, "y") > attr(body, "y") && attr(sup, "y") < attr(body, "y"),
            "{command}"
        );
    }
}

#[test]
fn export_latex_subtitle_axes_forms_and_properties() {
    let mut engine = Engine::default();
    engine
        .execute(
            r"plot(1:3); ax=gca;
              empty=strcmp(ax.Subtitle.String,'');
              short=subtitle('First subtitle');
              initial=strcmp(get(gca,'Subtitle').String,'First subtitle');
              explicit=subtitle(ax,'Second subtitle');
              [t,s]=title('Main heading','Below it');
              ok=double(empty && initial && isequal(short,explicit) && isequal(s,ax.Subtitle) ...
                && isequal(t,ax.Title) && isequal(s.Parent,ax) && strcmp(s.String,'Below it') ...
                && strcmp(s.FontWeight,'normal') && s.FontSize<t.FontSize);
              set(s,'Color',[1 0 0],'FontSize',9,'Interpreter','latex');
              s.String='$\sqrt{x}$';
              styled=double(isequal(s.Color,[1 0 0]) && s.FontSize==9 ...
                && strcmp(s.Interpreter,'latex') && strcmp(s.String,'$\sqrt{x}$'));",
        )
        .unwrap();
    assert_eq!(engine.variables["ok"].data(), &[1.]);
    assert_eq!(engine.variables["styled"].data(), &[1.]);
    assert!(
        engine
            .execute("set(s,'String','Bad','FontSize',0);")
            .is_err()
    );
    assert!(engine.execute("[x,y,z]=title('Bad','Bad');").is_err());
    let figure = engine.figure.as_ref().unwrap();
    assert_eq!(figure.title, "Main heading");
    assert_eq!(figure.extras.subtitle.string, r"$\sqrt{x}$");
}

#[test]
fn export_latex_subtitle_targeted_axes_and_layout_survive_native_save() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = Engine::default();
    engine.set_file_directory(dir.path()).unwrap();
    engine
        .execute(
            r"tiles=tiledlayout(1,2); ax=nexttile; plot(1:3);
              other=nexttile; plot(1:4);
              [at,asub]=title(ax,'Panel heading','Panel subtitle');
              [heading,sub]=title(tiles,'Shared heading','Original');
              changed=subtitle(tiles,{'Shared subtitle';'$\sqrt{x}$'},'Interpreter','latex');
              same=double(isequal(at,ax.Title) && isequal(asub,ax.Subtitle) ...
                && isempty(other.Subtitle.String) && isequal(tiles.Title,heading) ...
                && isequal(tiles.Subtitle,sub) && isequal(changed,sub));
              sub.Color=[0 0 1];
              savefig(gcf,'headings.mfig'); close(gcf); openfig('headings.mfig');",
        )
        .unwrap();
    assert_eq!(engine.variables["same"].data(), &[1.]);
    let figure = engine.figure.unwrap();
    assert_eq!(figure.axes.extras.subtitle.string, "Panel subtitle");
    let subtitle = &figure.layout.as_ref().unwrap().subtitle;
    assert_eq!(subtitle.string, "Shared subtitle\n$\\sqrt{x}$");
    assert_eq!(subtitle.color, Some([0., 0., 1.]));
    let svg = figure.svg();
    assert!(svg.contains(">Shared subtitle</text>"));
    assert!(svg.contains(">Panel subtitle</text>"));
}

#[test]
fn export_latex_subtitle_survives_fig_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = Engine::default();
    engine.set_file_directory(dir.path()).unwrap();
    engine
        .execute(
            r#"plot(1:3); [t,s]=title('A',"B"); xlabel('X');
               set(s,'Interpreter','latex','Color',[0 0 1],'FontSize',18,'FontWeight','bold');
               savefig('headings.fig'); close(gcf); openfig('headings.fig');
               ax=gca; s=ax.Subtitle;
               ok=double(strcmp(ax.Title.String,'A') && strcmp(ax.XLabel.String,'X') ...
                 && isstring(s.String) && isequal(s.String,"B") && strcmp(s.Interpreter,'latex') ...
                 && isequal(s.Color,[0 0 1]) && s.FontSize==18 && strcmp(s.FontWeight,'bold'));"#,
        )
        .unwrap();
    assert_eq!(engine.variables["ok"].data(), &[1.]);
}

#[test]
fn export_latex_subtitle_title_font_options_apply_to_both_headings() {
    let mut engine = Engine::default();
    engine
        .execute(
            r"plot(1:3); [t,s]=title('A','B','FontSize',18,'FontWeight','bold');
              ordinary=double(t.FontSize==18 && s.FontSize==18 ...
                && strcmp(t.FontWeight,'bold') && strcmp(s.FontWeight,'bold'));
              layout=tiledlayout(1,2);
              [t,s]=title(layout,'C','D','FontSize',14,'FontWeight','normal');
              shared=double(t.FontSize==14 && s.FontSize==14 ...
                && strcmp(t.FontWeight,'normal') && strcmp(s.FontWeight,'normal'));",
        )
        .unwrap();
    assert_eq!(engine.variables["ordinary"].data(), &[1.]);
    assert_eq!(engine.variables["shared"].data(), &[1.]);
    assert!(
        engine
            .execute("title(layout,'E','F','FontSize',0);")
            .is_err()
    );
    assert_eq!(
        engine
            .figure
            .as_ref()
            .unwrap()
            .layout
            .as_ref()
            .unwrap()
            .title,
        "C"
    );
}

#[cfg(feature = "plot-export")]
#[test]
fn export_latex_subtitle_font_size_readback_preserves_size() {
    let mut engine = Engine::default();
    for command in [
        "plot(1:3); s=subtitle('Size check');",
        "tiles=tiledlayout(1,2); nexttile; plot(1:3); s=subtitle(tiles,'Size check');",
    ] {
        engine.execute("close all;").unwrap();
        engine.execute(command).unwrap();
        let measure = |engine: &Engine| {
            let figure = engine.figure.as_ref().unwrap();
            let subtitle = figure
                .layout
                .as_ref()
                .map_or(&figure.extras.subtitle, |layout| &layout.subtitle);
            let svg = figure.svg();
            let doc = Document::parse(&svg).unwrap();
            (
                subtitle.font_size_points(),
                attr(glyph(&doc, "Size check"), "font-size"),
            )
        };
        let before = measure(&engine);
        // Default 10 pt keeps the historical 17 px export subtitle.
        assert_eq!(before, (10., 17.), "{command}");
        engine.execute("s.FontSize=s.FontSize;").unwrap();
        assert_eq!(before, measure(&engine), "{command}");
    }
}

#[test]
fn export_latex_subtitle_dot_and_set_preserve_string_class() {
    let mut engine = Engine::default();
    engine
        .execute(
            r#"plot(1:3); s=subtitle("x"); s.String="y";
               set(s,'String',"y"); from_set=get(s,'String');
               s.String="z"; from_dot=s.String;
               tiles=tiledlayout(1,2); s=subtitle(tiles,"x"); s.String="y";
               ok=double(isstring(from_set) && isequal(from_set,"y") ...
                 && isstring(from_dot) && isequal(from_dot,"z") ...
                 && isstring(s.String) && isequal(s.String,"y"));"#,
        )
        .unwrap();
    assert_eq!(engine.variables["ok"].data(), &[1.]);
}

#[cfg(feature = "plot-export")]
#[test]
fn export_latex_subtitle_follows_heading_in_svg_and_exports_to_pdf() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = Engine::default();
    engine.set_file_directory(dir.path()).unwrap();
    engine
        .execute(
            r"plot(1:3); [t,s]=title('Heading','Context');
              xlabel('$\frac{a}{bbbbbbbb}+\sqrt{x}$','Interpreter','latex');
              exportgraphics(gcf,'headings.pdf');",
        )
        .unwrap();
    let svg = engine.figure.as_ref().unwrap().svg();
    let doc = Document::parse(&svg).unwrap();
    let title = glyph(&doc, "Heading");
    let subtitle = glyph(&doc, "Context");
    for (key, direction) in [("y", 1.), ("font-size", -1.)] {
        assert!(
            direction * (attr(subtitle, key) - attr(title, key)) > 0.,
            "{key}"
        );
    }
    assert_eq!(subtitle.attribute("font-weight"), Some("normal"));
    assert!(
        std::fs::read(dir.path().join("headings.pdf"))
            .unwrap()
            .starts_with(b"%PDF")
    );
}

#[cfg(feature = "plot-export")]
#[test]
fn export_latex_subtitle_preserves_plain_title_anchor() {
    let mut engine = Engine::default();
    engine.execute("plot(1:3); title('A');").unwrap();
    for command in ["", "subtitle('B');"] {
        if !command.is_empty() {
            engine.execute(command).unwrap();
        }
        let svg = engine.figure.as_ref().unwrap().svg();
        let doc = Document::parse(&svg).unwrap();
        let title = glyph(&doc, "A");
        assert_eq!(attr(title, "x"), 70.);
        assert_eq!(attr(title, "y"), 32.);
        assert_eq!(title.attribute("text-anchor").unwrap_or("start"), "start");
    }
}

#[cfg(feature = "plot-export")]
#[test]
fn export_latex_subtitle_preserves_multiline_title_first_baseline() {
    let mut engine = Engine::default();
    engine
        .execute("subplot('Position',[.1 .01 .5 .04]); plot(1:3); title(repmat({'A'},22,1));")
        .unwrap();
    let svg = engine.figure.as_ref().unwrap().svg();
    let doc = Document::parse(&svg).unwrap();
    let title = glyph(&doc, "A");
    assert_eq!(
        title
            .children()
            .filter(|node| node.has_tag_name("tspan"))
            .count(),
        21
    );
    assert_eq!(attr(title, "y"), 18.);
}

#[cfg(feature = "plot-export")]
#[test]
fn export_latex_subtitle_rotated_nested_fraction_stays_on_canvas() {
    let mut engine = Engine::default();
    engine
        .execute(r"plot(1:3); ylabel('$\frac{\frac{\frac{A}{B}}{C}}{\frac{D}{E}}$','Interpreter','latex');")
        .unwrap();
    let svg = engine.figure.as_ref().unwrap().svg();
    let doc = Document::parse(&svg).unwrap();
    let numerator = glyph(&doc, "A");
    let transform = numerator
        .parent_element()
        .unwrap()
        .attribute("transform")
        .unwrap();
    let (translate_x, translate_y) = transform
        .strip_prefix("translate(")
        .unwrap()
        .split_once(')')
        .unwrap()
        .0
        .split_once(',')
        .unwrap();
    let translate_x: f64 = translate_x.parse().unwrap();
    let translate_y: f64 = translate_y.parse().unwrap();
    // Rotation maps the local y coordinate to canvas x. The original baseline
    // was -1.278, and the measured box now starts 2 px inside the left edge.
    let baseline = translate_x + attr(numerator, "y");
    assert!(
        (baseline - (2. + 0.8 * 16. * 0.7_f64.powi(3))).abs() < 0.001,
        "{baseline}"
    );
    let width = attr(doc.root_element(), "width");
    let height = attr(doc.root_element(), "height");
    for operand in ["A", "B", "C", "D", "E"] {
        let ink = glyph_ink(glyph(&doc, operand));
        assert!(translate_x + f64::from(ink.top()) >= 0., "{operand}");
        assert!(translate_x + f64::from(ink.bottom()) <= width, "{operand}");
        assert!(translate_y - f64::from(ink.right()) >= 0., "{operand}");
        assert!(translate_y - f64::from(ink.left()) <= height, "{operand}");
    }
}

#[cfg(feature = "plot-export")]
#[test]
fn export_latex_subtitle_fraction_rule_spans_wide_ink() {
    let mut engine = Engine::default();
    engine
        .execute(r"plot(1:3); title('$\frac{a}{@@@@@@@@}$','Interpreter','latex');")
        .unwrap();
    let svg = engine.figure.as_ref().unwrap().svg();
    let doc = Document::parse(&svg).unwrap();
    let denominator = glyph(&doc, "@@@@@@@@");
    assert_eq!(denominator.attribute("textLength"), None);
    let rule = denominator
        .parent_element()
        .unwrap()
        .children()
        .find(|node| node.has_tag_name("line"))
        .unwrap();
    let ink = glyph_ink(denominator);
    // The old rule ran from x=458.28 to x=531.72. The wide operand needs
    // more than those 73.44 px even after the new layout recenters it.
    assert!(f64::from(ink.width()) > 73.44);
    assert!(attr(rule, "x1") <= f64::from(ink.left()));
    assert!(attr(rule, "x2") >= f64::from(ink.right()));
}

#[cfg(feature = "plot-export")]
#[test]
fn export_latex_subtitle_fraction_scripts_shift_whole_boxes() {
    let mut engine = Engine::default();
    engine
        .execute(r"plot(1:3); title('$x^{\frac{a}{b}}+x_{\frac{a}{b}}$','Interpreter','latex');")
        .unwrap();
    let svg = engine.figure.as_ref().unwrap().svg();
    let doc = Document::parse(&svg).unwrap();
    let group = glyph(&doc, "a").parent_element().unwrap();
    let baselines = |tag, text, key| {
        group
            .children()
            .filter(|node| node.has_tag_name(tag) && node.text() == text)
            .map(|node| attr(node, key))
            .collect::<Vec<_>>()
    };
    let base = attr(glyph(&doc, "x"), "y");
    let rules = baselines("line", None, "y1");
    // The rule is 0.35 em above the fraction baseline and scripts shift
    // that baseline by -0.5 or +0.25 times 0.35 of the 20 px parent size.
    let rule_offset = 20. * 0.7 * 0.35;
    assert!((rules[0] + rule_offset - base + 3.5).abs() < 0.001);
    assert!((rules[1] + rule_offset - base - 1.75).abs() < 0.001);
    for operand in ["a", "b"] {
        let y = baselines("text", Some(operand), "y");
        assert!((y[1] - y[0] - 5.25).abs() < 0.001);
    }
}

#[cfg(feature = "plot-export")]
#[test]
fn export_latex_subtitle_bold_weight_applies_to_scripts() {
    let mut engine = Engine::default();
    engine
        .execute("plot(1:3); subtitle('A_1^2','FontWeight','bold');")
        .unwrap();
    let svg = engine.figure.as_ref().unwrap().svg();
    let doc = Document::parse(&svg).unwrap();
    let subtitle = glyph(&doc, "A");
    assert_eq!(subtitle.attribute("font-weight"), Some("bold"));
    for digit in ["1", "2"] {
        let span = subtitle
            .children()
            .find(|node| node.has_tag_name("tspan") && node.text() == Some(digit))
            .unwrap();
        assert_eq!(span.attribute("font-weight"), Some("bold"));
        assert_eq!(span.attribute("font-size"), Some("70%"));
    }
}

#[cfg(feature = "plot-export")]
#[test]
fn export_latex_subtitle_writes_x_label_rules_to_svg_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = Engine::default();
    engine.set_file_directory(dir.path()).unwrap();
    engine
        .execute(r"plot(1:3); xlabel('$\frac{a}{b}+\sqrt{x}$','Interpreter','latex'); exportgraphics(gcf,'p.svg');")
        .unwrap();
    let svg = std::fs::read_to_string(dir.path().join("p.svg")).unwrap();
    let doc = Document::parse(&svg).unwrap();
    let numerator = glyph(&doc, "a");
    let denominator = glyph(&doc, "b");
    let radical = glyph(&doc, "√");
    let group = numerator.parent_element().unwrap();
    assert_eq!(radical.parent_element(), Some(group));
    let rules: Vec<_> = group
        .children()
        .filter(|node| node.has_tag_name("line"))
        .collect();
    let [bar, overline] = rules.try_into().expect("fraction bar and radical overline");
    assert!(attr(numerator, "y") < attr(bar, "y1") && attr(bar, "y1") < attr(denominator, "y"));
    assert!(attr(overline, "y1") < attr(glyph(&doc, "x"), "y"));
    for rule in [bar, overline] {
        assert_eq!(attr(rule, "y1"), attr(rule, "y2"));
        assert!(attr(rule, "x1") < attr(rule, "x2"));
    }
}

#[cfg(feature = "plot-export")]
#[test]
fn export_latex_subtitle_png_has_ink_in_both_heading_bands() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = Engine::default();
    engine.set_file_directory(dir.path()).unwrap();
    engine
        .execute(
            "plot(1:3); [t,s]=title('A','B'); axis off; exportgraphics(gcf,'headings.png','Resolution',96);",
        )
        .unwrap();
    let png = image::open(dir.path().join("headings.png"))
        .unwrap()
        .into_rgb8();
    // The crop starts 2 px above the title. Both headings are within the
    // first 45 rows at 96 dpi. The plot line contributes no dark gray pixels.
    let counts: Vec<_> = png
        .rows()
        .take(45)
        .map(|row| {
            row.filter(|pixel| pixel.0.iter().all(|&channel| channel < 120))
                .count()
        })
        .collect();
    let bands = counts
        .split(|&count| count == 0)
        .filter(|band| !band.is_empty())
        .count();
    assert_eq!(bands, 2, "heading rows: {counts:?}");
}
