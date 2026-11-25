use matrust::Engine;
use matrust::plot::text_markup::{Interpreter, Node, parse_nodes};

#[test]
fn text_latex_markup_nodes_and_fallback() {
    let frac = parse_nodes("$\\frac{a}{b}$", Interpreter::Latex);
    assert_eq!(frac.len(), 1);
    match &frac[0] {
        Node::Fraction { num, den, .. } => {
            assert!(matches!(&num[0], Node::Run(r) if r.text == "a"));
            assert!(matches!(&den[0], Node::Run(r) if r.text == "b"));
        }
        other => panic!("expected Fraction node, got {other:?}"),
    }

    let sub_sup = parse_nodes("$x_1^2$", Interpreter::Latex);
    assert_eq!(sub_sup.len(), 3);
    assert!(matches!(&sub_sup[0], Node::Run(r) if r.text == "x" && r.shift == 0.));
    assert!(matches!(&sub_sup[1], Node::Run(r) if r.text == "1" && r.shift < 0.));
    assert!(matches!(&sub_sup[2], Node::Run(r) if r.text == "2" && r.shift > 0.));

    let root = parse_nodes("$\\sqrt{x}$", Interpreter::Latex);
    assert_eq!(root.len(), 1);
    assert!(
        matches!(&root[0], Node::Sqrt { body, .. } if matches!(&body[0], Node::Run(r) if r.text == "x"))
    );

    let fallback = parse_nodes("$\\unsupported{x}$", Interpreter::Latex);
    assert_eq!(fallback.len(), 1);
    assert!(matches!(&fallback[0], Node::Run(r) if r.text == "$\\unsupported{x}$"));

    let mixed = parse_nodes(
        "plain $$\\sum_{i=1}^\\infty \\alpha \\cdot \\pm \\le \\ge \\left(\\mathrm{x}+\\mathbf{y}\\right)$$ tail",
        Interpreter::Latex,
    );
    let joined: String = mixed
        .iter()
        .map(|n| match n {
            Node::Run(r) => r.text.as_str(),
            _ => "",
        })
        .collect();
    assert!(joined.starts_with("plain ∑"));
    assert!(joined.contains("∞"));
    assert!(joined.contains("α · ± ≤ ≥ (x+y)"));
    assert!(joined.ends_with(" tail"));
}

#[test]
fn text_latex_legend_properties_and_save_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute(
        "plot(1:3,1:3); hold on; plot(1:3,[3 2 1]); \
         lg=legend('a','b','NumColumns',2,'Orientation','vertical','Box','off'); \
         nc=get(lg,'NumColumns'); or1=get(lg,'Orientation'); bx1=get(lg,'Box'); \
         lg.Orientation='horizontal'; or2=lg.Orientation; \
         lg.NumColumns=2; nc2=lg.NumColumns; \
         lg.Box='on'; bx2=lg.Box; \
         set(lg,'Box','off'); bx3=get(lg,'Box'); \
         ok=double(nc==2 && strcmp(or1,'vertical') && strcmp(bx1,'off') && strcmp(or2,'horizontal') && nc2==2 && strcmp(bx2,'on') && strcmp(bx3,'off')); \
         savefig(gcf,'leg.mfig'); close(gcf); \
         openfig('leg.mfig'); lg2=legend(); \
         nc_after=get(lg2,'NumColumns'); bx_after=get(lg2,'Box'); \
         ok2=double(nc_after==2 && strcmp(bx_after,'off'));",
    )
    .unwrap();
    assert_eq!(e.variables["nc"].data(), &[2.]);
    assert_eq!(e.variables["nc2"].data(), &[2.]);
    assert_eq!(e.variables["ok"].data(), &[1.]);
    assert_eq!(e.variables["nc_after"].data(), &[2.]);
    assert_eq!(e.variables["ok2"].data(), &[1.]);

    assert!(e.execute("set(lg2,'NumColumns',0);").is_err());
    assert!(e.execute("set(lg2,'NumColumns',1.5);").is_err());
    assert!(e.execute("set(lg2,'Orientation','diagonal');").is_err());
    assert!(e.execute("set(lg2,'Box','maybe');").is_err());
}

#[cfg(feature = "plot-export")]
#[test]
fn text_latex_png_fraction_and_legend_layout_pixels() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();

    // 1. Fraction label '$\\frac{1}{2}$' has two glyph bands with a bar between.
    e.execute(
        "plot([0 1],[0 1]); title('$\\frac{1}{2}$','Interpreter','latex'); \
         exportgraphics(gcf,'frac.png','Resolution',96);",
    )
    .unwrap();
    let frac_im = image::open(dir.path().join("frac.png")).unwrap().to_rgb8();
    let counts: Vec<usize> = (0..50.min(frac_im.height()))
        .map(|y| {
            (frac_im.width() * 35 / 100..frac_im.width() * 65 / 100)
                .filter(|&x| {
                    let p = frac_im.get_pixel(x, y).0;
                    p[0] < 120 && p[1] < 120 && p[2] < 120
                })
                .count()
        })
        .collect();
    let bands = counts
        .split(|&n| n == 0)
        .filter(|band| band.iter().any(|&n| n > 0))
        .count();
    assert!(
        bands >= 3,
        "fraction label must have two glyph bands with a bar between: {counts:?}"
    );

    // 2. Legend with Box off has no frame pixels; 4 entries with NumColumns 2 occupy two rows.
    let mut e2 = Engine::default();
    e2.set_file_directory(dir.path()).unwrap();
    e2.execute(
        "plot([0 1],[0 1]); hold on; plot([0 1],[1 0]); plot([0 1],[0.5 0.5]); plot([0 1],[0.2 0.8]); grid off; \
         lg_on=legend('a','b','c','d','NumColumns',2,'Box','on','Location','northeast'); \
         exportgraphics(gcf,'leg_on.png','Resolution',96); \
         set(lg_on,'Box','off'); \
         exportgraphics(gcf,'leg_off.png','Resolution',96);",
    )
    .unwrap();
    let svg_off = e2.figure.as_ref().unwrap().svg();
    assert!(
        !svg_off.contains("stroke=\"#aaa\""),
        "Box off SVG must not draw legend frame"
    );
    let im_on = image::open(dir.path().join("leg_on.png"))
        .unwrap()
        .to_rgb8();
    let im_off = image::open(dir.path().join("leg_off.png"))
        .unwrap()
        .to_rgb8();
    let frame_only_in_on = (0..im_on.height().min(im_off.height()))
        .flat_map(|y| (0..im_on.width().min(im_off.width())).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let a = im_on.get_pixel(x, y).0;
            let b = im_off.get_pixel(x, y).0;
            a[0] < 245 && a[1] < 245 && a[2] < 245 && b == [255, 255, 255]
        })
        .count();
    assert!(
        frame_only_in_on > 100,
        "Box off should have no legend frame pixels (frame_only_in_on={frame_only_in_on})"
    );

    // 4 entries with NumColumns 2 occupy two rows (two separated bands in legend area).
    let row_counts: Vec<usize> = (0..im_off.height() / 3)
        .map(|y| {
            (im_off.width() / 2..im_off.width() * 9 / 10)
                .filter(|&x| {
                    let p = im_off.get_pixel(x, y).0;
                    p[0] < 100 && p[1] < 100 && p[2] < 100
                })
                .count()
        })
        .collect();
    let row_bands = row_counts
        .split(|&n| n == 0)
        .filter(|band| band.iter().any(|&n| n > 2))
        .count();
    assert!(
        row_bands >= 2,
        "4 entries with NumColumns 2 must occupy two separated row bands: {row_counts:?}"
    );
}
