use matrust::Engine;
use matrust::plot::text_markup::{Interpreter, parse};

#[test]
fn text_interpreters_markup_runs() {
    for (source, expected, shift) in [
        ("x_1", "1", -0.25),
        ("m^2", "2", 0.5),
        ("x_{10}", "10", -0.25),
    ] {
        let runs = parse(source, Interpreter::Tex);
        assert_eq!(runs[1].text, expected);
        assert_eq!(runs[1].shift, shift);
    }
    assert_eq!(parse("\\alpha", Interpreter::Tex)[0].text, "α");
    assert_eq!(parse("\\foo", Interpreter::Tex)[0].text, "\\foo");
    assert_eq!(parse("x_1", Interpreter::None)[0].text, "x_1");
}

#[test]
fn text_interpreters_handle_properties() {
    let mut e = Engine::default();
    e.execute("t=title({'a','b'}); s=get(t,'String'); n=size(s,1); i=get(t,'Interpreter'); default_tex=double(strcmp(i,'tex'));")
        .unwrap();
    assert_eq!(e.variables["n"].data(), &[2.]);
    assert_eq!(e.variables["default_tex"].data(), &[1.]);
    let before = e.figure.as_ref().unwrap().svg();
    e.execute("set(t,'String','x_1','Interpreter','none'); xlabel('$x$','Interpreter','latex');")
        .unwrap();
    let after = e.figure.as_ref().unwrap().svg();
    assert_ne!(before, after);
    assert!(after.contains("x_1"));
    e.execute("set(t,'Interpreter','tex');").unwrap();
    assert!(e.figure.as_ref().unwrap().svg().contains("baseline-shift"));
}

#[cfg(feature = "plot-export")]
#[test]
fn text_interpreters_png_title_lines_and_greek() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute("plot([0 1],[0 1]); title({'AAAA','BBBB'}); exportgraphics(gcf,'lines.png','Resolution',96);").unwrap();
    let image = image::open(dir.path().join("lines.png")).unwrap().to_rgb8();
    let counts: Vec<_> = (0..60.min(image.height()))
        .map(|y| {
            (image.width() / 4..image.width() * 3 / 4)
                .filter(|&x| {
                    let p = image.get_pixel(x, y).0;
                    p[0] < 100 && p[1] < 100 && p[2] < 100
                })
                .count()
        })
        .collect();
    let bands = counts
        .split(|&n| n == 0)
        .filter(|band| band.iter().any(|&n| n > 2))
        .count();
    assert!(bands >= 2, "title needs two distinct bands: {counts:?}");
    e.execute("title('\\alpha'); exportgraphics(gcf,'greek.png','Resolution',96); set(get(gca,'Title'),'Interpreter','none'); exportgraphics(gcf,'literal.png','Resolution',96);").unwrap();
    let greek = image::open(dir.path().join("greek.png")).unwrap().to_rgb8();
    let literal = image::open(dir.path().join("literal.png"))
        .unwrap()
        .to_rgb8();
    let ink_width = |im: &image::RgbImage| {
        let xs: Vec<_> = (0..im.width())
            .filter(|&x| {
                (0..40.min(im.height())).any(|y| {
                    let p = im.get_pixel(x, y).0;
                    p[0] < 100 && p[1] < 100 && p[2] < 100
                })
            })
            .collect();
        xs.last().unwrap() - xs.first().unwrap()
    };
    assert_ne!(ink_width(&greek), ink_width(&literal));
}

#[cfg(feature = "plot-export")]
#[test]
fn text_interpreters_png_superscript_is_raised() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute("plot([0 1],[0 1]); xlabel('x^2'); exportgraphics(gcf,'sup.png','Resolution',96);")
        .unwrap();
    let im = image::open(dir.path().join("sup.png")).unwrap().to_rgb8();
    let columns: Vec<_> = (im.width() * 35 / 100..im.width() * 65 / 100)
        .map(|x| {
            (im.height().saturating_sub(30)..im.height())
                .filter(|&y| {
                    let p = im.get_pixel(x, y).0;
                    p[0] < 100 && p[1] < 100 && p[2] < 100
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let groups: Vec<_> = columns
        .split(|ys| ys.is_empty())
        .filter(|g| !g.is_empty())
        .collect();
    assert!(groups.len() >= 2, "expected separate glyphs");
    let top = |group: &[Vec<u32>]| {
        group
            .iter()
            .flat_map(|ys| ys.iter())
            .min()
            .copied()
            .unwrap()
    };
    assert!(
        top(groups.last().unwrap()) < top(groups[0]),
        "superscript must be above base"
    );
}

#[test]
fn text_interpreters_multiline_shapes() {
    let mut e = Engine::default();
    e.execute("t=title([\"a\",\"b\"]); s=get(t,'String'); r=size(s,1); c=size(s,2); u=xlabel(['one';'two']); v=get(u,'String'); r2=size(v,1); c2=size(v,2); z=ylabel(\"solo\"); zz=get(z,'String'); scalar_string=double(isa(zz,'string'));").unwrap();
    assert_eq!(e.variables["r"].data(), &[1.]);
    assert_eq!(e.variables["c"].data(), &[2.]);
    assert_eq!(e.variables["r2"].data(), &[2.]);
    assert_eq!(e.variables["c2"].data(), &[3.]);
    assert_eq!(e.variables["scalar_string"].data(), &[1.]);
}

#[test]
fn text_interpreters_legend_and_ticks_are_stored() {
    let mut e = Engine::default();
    e.execute("plot([0 1],[0 1]); lg=legend('x_1'); a=gca; p=get(lg,'Interpreter'); q=get(a,'TickLabelInterpreter'); ok=double(strcmp(p,'tex') && strcmp(q,'tex')); set(lg,'Interpreter','latex'); set(a,'TickLabelInterpreter','none'); p=get(lg,'Interpreter'); q=get(a,'TickLabelInterpreter'); ok2=double(strcmp(p,'latex') && strcmp(q,'none'));").unwrap();
    assert_eq!(e.variables["ok"].data(), &[1.]);
    assert_eq!(e.variables["ok2"].data(), &[1.]);
}

#[test]
fn text_interpreters_shared_title_and_text_multiline() {
    let mut e = Engine::default();
    e.execute("tiledlayout(1,1); nexttile; h=sgtitle({'top','bottom'},'Interpreter','none'); s=get(h,'String'); n=size(s,1); t=text(1,1,{'one','two'}); q=get(t,'String'); m=size(q,1); p=get(t,'Interpreter'); ok=double(strcmp(p,'tex'));").unwrap();
    assert_eq!(e.variables["n"].data(), &[2.]);
    assert_eq!(e.variables["m"].data(), &[2.]);
    assert_eq!(e.variables["ok"].data(), &[1.]);
}

#[test]
fn text_interpreters_script_backslash_commands() {
    let alpha = parse("x_\\alpha", Interpreter::Tex);
    assert_eq!(alpha.len(), 2);
    assert_eq!(alpha[0].text, "x");
    assert_eq!(alpha[1].text, "α");
    assert_eq!(alpha[1].shift, -0.25);

    let deg_c = parse("T (^\\circC)", Interpreter::Tex);
    assert_eq!(deg_c.len(), 3);
    assert_eq!(deg_c[0].text, "T (");
    assert_eq!(deg_c[0].shift, 0.);
    assert_eq!(deg_c[1].text, "∘");
    assert_eq!(deg_c[1].shift, 0.5);
    assert_eq!(deg_c[2].text, "C)");
    assert_eq!(deg_c[2].shift, 0.);
}

#[test]
fn text_interpreters_ignore_color_and_fontname() {
    for src in [
        "\\color[rgb]{1 0 0}hi",
        "\\color{red}hi",
        "\\fontname{Helvetica}hi",
        "\\fontsize{14}hi",
    ] {
        let runs = parse(src, Interpreter::Tex);
        let joined: String = runs.into_iter().map(|r| r.text).collect();
        assert_eq!(joined, "hi", "failed for {src}");
    }
}

#[test]
fn text_interpreters_set_string_scalar_on_legend_and_textbox() {
    let mut e = Engine::default();
    e.execute(
        "plot([0 1],[0 1]); lg=legend('a'); set(lg,'String',\"b\"); \
         tb=annotation('textbox',[0.1 0.1 0.2 0.2],'String','x','Interpreter','none','EdgeColor','none'); set(tb,'String',\"y\");",
    )
    .unwrap();
    let svg = e.figure.as_ref().unwrap().svg();
    assert!(svg.contains(">b<"));
    assert!(svg.contains(">y<"));
}

#[test]
fn text_interpreters_colorbar_and_yyaxis_right_ylabel_interpreter() {
    let mut e = Engine::default();
    e.execute(
        "plot([0 1],[0 1]); cb=colorbar; \
         set(get(cb,'Label'),'Interpreter','none'); \
         set(get(cb,'Label'),'Interpreter','tex'); \
         yyaxis right; plot([0 1],[0 1]); \
         set(get(gca,'YLabel'),'Interpreter','none'); \
         set(get(gca,'YLabel'),'Interpreter','latex');",
    )
    .unwrap();
}

#[test]
fn text_interpreters_multiline_xlabel_inside_canvas() {
    let mut e = Engine::default();
    e.execute("plot([0 1],[0 1]); xlabel({'x1','x2'});")
        .unwrap();
    let svg = e.figure.as_ref().unwrap().svg();
    let needle = ">x1<tspan";
    let pos = svg.find(needle).expect("xlabel x1 in SVG");
    let tag_start = svg[..pos].rfind("<text ").unwrap();
    let y_attr = svg[tag_start..pos].find("y=\"").unwrap() + tag_start + 3;
    let y_end = svg[y_attr..].find('"').unwrap() + y_attr;
    let y: f64 = svg[y_attr..y_end].parse().unwrap();
    assert!(y + 19.2 < 560.0, "2nd row y ({}) must be < 560", y + 19.2);

    let mut e2 = Engine::default();
    e2.execute("t=tiledlayout(1,1); nexttile; plot([0 1],[0 1]); xlabel(t,{'s1','s2'});")
        .unwrap();
    let svg2 = e2.figure.as_ref().unwrap().svg();
    let pos2 = svg2.find(">s1<tspan").expect("shared xlabel s1 in SVG");
    let tag_start2 = svg2[..pos2].rfind("<text ").unwrap();
    let y_attr2 = svg2[tag_start2..pos2].find("y=\"").unwrap() + tag_start2 + 3;
    let y_end2 = svg2[y_attr2..].find('"').unwrap() + y_attr2;
    let y2: f64 = svg2[y_attr2..y_end2].parse().unwrap();
    assert!(
        y2 + 19.2 < 560.0,
        "shared 2nd row y ({}) must be < 560",
        y2 + 19.2
    );
}

#[test]
fn text_interpreters_xticklabels_markup() {
    let mut e = Engine::default();
    e.execute(
        "plot([0 1],[0 1]); xticks([0 1]); xticklabels({'\\alpha','x^2'}); \
         lbl=xticklabels; ok=double(strcmp(lbl{1},'\\alpha') && strcmp(lbl{2},'x^2'));",
    )
    .unwrap();
    assert_eq!(e.variables["ok"].data(), &[1.]);
    let svg = e.figure.as_ref().unwrap().svg();
    assert!(svg.contains('α'), "SVG should render α tick label");
    assert!(
        svg.contains("baseline-shift=\"50%\"") && svg.contains(">2</tspan>"),
        "SVG should render superscript 2 run"
    );
}

#[test]
fn text_interpreters_unknown_command_and_script_prefix() {
    let mut e = Engine::default();
    e.execute("title('\\alphabet');").unwrap();
    let svg = e.figure.as_ref().unwrap().svg();
    assert!(svg.contains("\\alphabet"));
    assert!(!svg.contains("αbet"));
    let runs = parse("T (^\\circC)", Interpreter::Tex);
    assert!(runs.iter().any(|r| r.text == "∘" && r.shift == 0.5));
    assert!(runs.iter().any(|r| r.text == "C)" && r.shift == 0.));
}

#[test]
fn text_interpreters_tick_label_shared_runs() {
    // The desktop markup layout consumes these same runs for both axes and rotations.
    let runs = parse("x^2", Interpreter::Tex);
    assert_eq!(
        runs.iter().map(|r| r.text.as_str()).collect::<String>(),
        "x2"
    );
    assert_eq!(runs[1].shift, 0.5);
    assert_eq!(runs[1].scale, 0.7);
    assert_eq!(parse("x^2", Interpreter::None)[0].text, "x^2");
}

#[test]
fn text_interpreters_native_roundtrip() {
    use matrust::files::{load_figure, save_figure};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("text.mfig");
    let mut e = Engine::default();
    e.execute("plot([0 1],[0 1]); t=title({'a','b'},'Interpreter','none'); xlabel('x_1','Interpreter','none'); ylabel('y','Interpreter','latex'); zlabel('z','Interpreter','none'); lg=legend('x_1'); set(lg,'Interpreter','none'); set(gca,'TickLabelInterpreter','latex');").unwrap();
    let original = e.figure.as_ref().unwrap();
    save_figure(&path, original).unwrap();
    let loaded = load_figure(&path).unwrap();
    assert_eq!(
        loaded.extras.label_interpreters,
        original.extras.label_interpreters
    );
    assert_eq!(loaded.extras.label_lines, original.extras.label_lines);
    assert_eq!(loaded.extras.tick_label_interpreter, Interpreter::Latex);
    assert_eq!(loaded.extras.legend_interpreter, Interpreter::None);
    assert!(loaded.svg().contains("x_1"));
    e.figure = Some(loaded);
    e.execute("s=get(get(gca,'Title'),'String'); n=size(s,1);")
        .unwrap();
    assert_eq!(e.variables["n"].data(), &[2.]);
    let plain = Engine::default();
    let mut default_figure = plain.figure.unwrap_or_default();
    save_figure(&path, &default_figure).unwrap();
    default_figure = load_figure(&path).unwrap();
    assert_eq!(
        default_figure.extras.label_interpreters,
        [Interpreter::Tex; 4]
    );
    assert_eq!(
        default_figure.extras.tick_label_interpreter,
        Interpreter::Tex
    );
    assert_eq!(default_figure.extras.legend_interpreter, Interpreter::Tex);
}
