use matrust::Engine;

#[cfg(feature = "plot-export")]
#[test]
fn fix16_review_render_png_position_maps_one_to_one() {
    for (w, h) in [(480u32, 280u32), (1920u32, 1120u32)] {
        let dir = tempfile::tempdir().unwrap();
        let mut e = Engine::default();
        e.set_file_directory(dir.path()).unwrap();
        e.execute(&format!(
            "figure('Position', [1 1 {w} {h}]); plot([1 2 3], [1 1 1], 'r-', 'LineWidth', 4); saveas(gcf, 'p.png');"
        ))
        .unwrap();
        let svg = e.figure.as_ref().unwrap().svg();
        // Extract the red line's path y-coordinate from the SVG.
        let path_marker = "stroke=\"#ff0000\"";
        let path_idx = svg.find(path_marker).expect("red line in svg");
        let d_start = svg[path_idx..].find("d=\"M").unwrap() + path_idx + 4;
        let d_end = svg[d_start..].find('"').unwrap() + d_start;
        let first_pt = svg[d_start..d_end].split(' ').next().unwrap();
        let mut coords = first_pt.split(',');
        let svg_x0: f64 = coords.next().unwrap().parse().unwrap();
        let svg_y0: f64 = coords.next().unwrap().parse().unwrap();

        let img = image::open(dir.path().join("p.png")).unwrap().to_rgb8();
        assert_eq!((img.width(), img.height()), (w, h));

        let is_red = |x: u32, y: u32| {
            let p = img.get_pixel(x, y).0;
            p[0] > 180 && p[1] < 80 && p[2] < 80
        };
        let cx = w / 2;
        let cy = svg_y0.round() as u32;
        assert!(
            (cy.saturating_sub(2)..=(cy + 2).min(h - 1)).any(|y| is_red(cx, y)),
            "expected red pixel at ({cx}, {cy}) for {w}x{h}"
        );
        // Line starts at svg_x0 (70) and ends at w - 40; left of svg_x0 must not have the line.
        assert!(
            !is_red((svg_x0 as u32).saturating_sub(20), cy),
            "unexpected red pixel left of plot box for {w}x{h}"
        );
        assert!(
            (cy.saturating_sub(2)..=(cy + 2).min(h - 1)).any(|y| is_red(w - 50, y)),
            "expected red pixel near right end of plot box for {w}x{h} (not clipped or shrunk)"
        );
    }
}

#[test]
fn fix16_review_render_tiledlayout_shared_xlabel_fits_height() {
    let mut e = Engine::default();
    e.execute(
        "t=tiledlayout(1,1); nexttile; plot(1:3); xlabel(t,'Time'); set(gcf,'Position',[1 1 600 400]);",
    )
    .unwrap();
    let svg = e.figure.as_ref().unwrap().svg();
    assert!(svg.contains("height=\"400\""));
    let label_pos = svg.find(">Time</text>").expect("shared xlabel in svg");
    let tag_start = svg[..label_pos].rfind("<text ").unwrap();
    let tag = &svg[tag_start..label_pos];
    let y_start = tag.find("y=\"").unwrap() + 3;
    let y_end = tag[y_start..].find('"').unwrap() + y_start;
    let y: f64 = tag[y_start..y_end].parse().unwrap();
    assert!(
        y > 370.0 && y < 400.0,
        "shared xlabel y={y} must fit inside 400px canvas footer"
    );
}

fn parse_attr(tag: &str, name: &str) -> f64 {
    let needle = format!("{name}=\"");
    let start = tag.find(&needle).unwrap() + needle.len();
    let end = tag[start..].find('"').unwrap() + start;
    tag[start..end].parse().unwrap()
}

fn tile_plot_box_in_figure(svg: &str, tile_idx: usize) -> [f64; 4] {
    let clip = format!("<clipPath id=\"axes{tile_idx}\"><rect ");
    let clip_pos = svg.find(&clip).unwrap();
    let nested_pos = svg[..clip_pos].rfind("<svg ").unwrap();
    let nested_end = svg[nested_pos..].find('>').unwrap() + nested_pos;
    let nested_tag = &svg[nested_pos..nested_end];

    let sx = parse_attr(nested_tag, "x");
    let sy = parse_attr(nested_tag, "y");
    let sw = parse_attr(nested_tag, "width");
    let sh = parse_attr(nested_tag, "height");
    let vb_start = nested_tag.find("viewBox=\"0 0 ").unwrap() + 12;
    let vb_end = nested_tag[vb_start..].find('"').unwrap() + vb_start;
    let mut vb = nested_tag[vb_start..vb_end].split_whitespace();
    let vbw: f64 = vb.next().unwrap().parse().unwrap();
    let vbh: f64 = vb.next().unwrap().parse().unwrap();

    let rect_end = svg[clip_pos..].find("/>").unwrap() + clip_pos;
    let rect_tag = &svg[clip_pos..rect_end];
    let rx = parse_attr(rect_tag, "x");
    let ry = parse_attr(rect_tag, "y");
    let rw = parse_attr(rect_tag, "width");
    let rh = parse_attr(rect_tag, "height");

    let left = sx + rx * (sw / vbw);
    let top = sy + ry * (sh / vbh);
    let right = sx + (rx + rw) * (sw / vbw);
    let bottom = sy + (ry + rh) * (sh / vbh);
    [left, top, right, bottom]
}

#[test]
fn fix16_review_render_tiledlayout_spacing_and_padding() {
    let mut e = Engine::default();
    e.execute("t=tiledlayout(1,2); nexttile; plot(1:3); nexttile; plot(3:-1:1);")
        .unwrap();
    let loose_svg = e.figure.as_ref().unwrap().svg();
    let loose_0 = tile_plot_box_in_figure(&loose_svg, 0);
    let loose_1 = tile_plot_box_in_figure(&loose_svg, 1);
    let loose_gap = loose_1[0] - loose_0[2];
    let loose_left_margin = loose_0[0];

    e.execute("t.TileSpacing='compact';").unwrap();
    let compact_svg = e.figure.as_ref().unwrap().svg();
    let compact_0 = tile_plot_box_in_figure(&compact_svg, 0);
    let compact_1 = tile_plot_box_in_figure(&compact_svg, 1);
    let compact_gap = compact_1[0] - compact_0[2];
    assert!(
        compact_gap < loose_gap && compact_gap > 0.0,
        "compact gap ({compact_gap}) must be smaller than loose gap ({loose_gap})"
    );

    e.execute("t.TileSpacing='none'; t.Padding='tight';")
        .unwrap();
    let none_tight_svg = e.figure.as_ref().unwrap().svg();
    assert_ne!(none_tight_svg, loose_svg);
    let nt_0 = tile_plot_box_in_figure(&none_tight_svg, 0);
    let nt_1 = tile_plot_box_in_figure(&none_tight_svg, 1);
    let none_gap = nt_1[0] - nt_0[2];
    assert!(
        none_gap.abs() < 1e-6,
        "TileSpacing='none' must make adjacent tiles touch, got gap={none_gap}"
    );
    assert!(
        nt_0[0] < loose_left_margin,
        "Padding='tight' outer margin ({}) must be smaller than loose ({loose_left_margin})",
        nt_0[0]
    );

    // Desktop layout helper shares the same TileSpacing / Padding properties.
    let layout = e.figure.as_ref().unwrap().layout.as_ref().unwrap();
    let panels: Vec<_> = e.figure.as_ref().unwrap().all_axes().collect();
    let p0 = panels
        .iter()
        .find(|(p, _)| p.left == 0)
        .map(|(p, _)| *p)
        .unwrap();
    let p1 = panels
        .iter()
        .find(|(p, _)| p.left == 1)
        .map(|(p, _)| *p)
        .unwrap();
    let insets_0 = layout.tiled_desktop_insets(p0);
    let insets_1 = layout.tiled_desktop_insets(p1);
    assert_eq!(insets_0[0], 0.0);
    assert_eq!(insets_0[2] + insets_1[0], 0.0);
}

#[test]
fn fix16_review_render_view3d_datetime_ticks_and_angle() {
    let mut e = Engine::default();
    e.execute("plot(datetime(2024,1,1:3),1:3); view(3); xtickangle(45);")
        .unwrap();
    let fig = e.figure.as_ref().unwrap();
    let svg = fig.svg();
    assert!(
        !svg.contains("19723"),
        "3D datetime plot must not emit raw numeric epoch days"
    );
    assert!(
        svg.contains("Jan") || svg.contains("2024"),
        "3D datetime plot must emit formatted date tick labels"
    );
    assert!(
        svg.contains("rotate(-45 "),
        "3D xtickangle(45) must rotate tick labels in SVG"
    );

    let std::borrow::Cow::Owned(projected) = matrust::plot::view3d::display_view(&fig.axes) else {
        panic!("expected 3D projection");
    };
    let rotated_date_ticks: Vec<_> = projected
        .extras
        .texts
        .iter()
        .filter(|t| {
            (t.rotation - 45.0).abs() < 1e-6
                && (t.string.contains("Jan") || t.string.contains("2024"))
        })
        .collect();
    assert!(
        !rotated_date_ticks.is_empty(),
        "projected 3D view used by desktop/SVG must carry date labels with 45 deg rotation"
    );
}

#[test]
fn fix16_review_render_plain_title_left_aligned() {
    let mut e = Engine::default();
    e.execute("plot(1:3); title('Runs');").unwrap();
    let svg = e.figure.as_ref().unwrap().svg();
    assert!(
        svg.contains(r#"<text x="70" y="32" font-size="20">Runs</text>"#),
        "plain single-line title must stay left-aligned at x=70: {svg}"
    );

    e.execute("title({'Runs','Line 2'});").unwrap();
    let multi_svg = e.figure.as_ref().unwrap().svg();
    assert!(
        multi_svg.contains(r#"text-anchor="middle""#),
        "multiline title should remain centered"
    );

    e.execute("title('\\alpha');").unwrap();
    let markup_svg = e.figure.as_ref().unwrap().svg();
    assert!(
        markup_svg.contains(r#"text-anchor="middle""#),
        "markup title should remain centered"
    );
}
