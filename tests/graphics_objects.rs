//! Graphics objects: dot syntax on handles, gca/gcf, text, colormap,
//! colorbar, clim, yyaxis and drawnow. Expected behaviour follows the
//! published MathWorks graphics documentation (behaviour only); colormap
//! values follow their closed-form definitions and the CC0 viridis table.
use matrust::engine::GraphicsFrame;
use matrust::{Engine, HostIo};
use std::sync::{Arc, Mutex};

fn values(e: &mut Engine, name: &str) -> Vec<f64> {
    e.execute(&format!("zz__ = double({name});")).unwrap();
    e.variables["zz__"].data().to_vec()
}
fn text(e: &mut Engine, name: &str) -> String {
    values(e, name)
        .iter()
        .map(|&c| char::from_u32(c as u32).unwrap())
        .collect()
}
fn svg(e: &Engine) -> String {
    e.figure.as_ref().unwrap().svg()
}

#[test]
fn dot_reads_and_writes_agree_with_get_and_set() {
    let mut e = Engine::default();
    e.execute("h = plot(1:4, [2 4 6 8]); ax = gca; f = gcf;")
        .unwrap();
    e.execute("a = ax.XLim; b = get(ax, 'XLim');").unwrap();
    assert_eq!(values(&mut e, "a"), values(&mut e, "b"));
    e.execute("ax.XLim = [1 10]; c = get(ax, 'XLim'); d = ax.xlim;")
        .unwrap();
    assert_eq!(values(&mut e, "c"), [1., 10.]);
    assert_eq!(values(&mut e, "d"), [1., 10.]);
    e.execute("set(ax, 'YLim', [-1 9]); y = ax.YLim;").unwrap();
    assert_eq!(values(&mut e, "y"), [-1., 9.]);
    // Indexed assignment through a property and nested handles.
    e.execute("ax.XLim(2) = 5; x2 = get(ax, 'XLim'); px = h.Parent.XLim;")
        .unwrap();
    assert_eq!(values(&mut e, "x2"), [1., 5.]);
    assert_eq!(values(&mut e, "px"), [1., 5.]);
    e.execute("ax.XScale = 'log'; s = get(ax, 'XScale');")
        .unwrap();
    assert_eq!(text(&mut e, "s"), "log");
    e.execute("ax.XScale = 'linear'; ax.FontSize = 14; fs = get(ax, 'FontSize');")
        .unwrap();
    assert_eq!(values(&mut e, "fs"), [14.]);
    e.execute("f.Color = [0.2 0.4 0.6]; fc = get(f, 'Color');")
        .unwrap();
    assert!(
        values(&mut e, "fc")
            .iter()
            .zip([0.2, 0.4, 0.6])
            .all(|(a, b)| (a - b).abs() < 0.01)
    );
    e.execute("ax.Title.String = 'Hello'; t = get(get(ax, 'Title'), 'String');")
        .unwrap();
    assert_eq!(text(&mut e, "t"), "Hello");
    assert_eq!(e.figure.as_ref().unwrap().axes.title, "Hello");
    e.execute("title('Second'); t2 = ax.Title.String;").unwrap();
    assert_eq!(text(&mut e, "t2"), "Second");
    // Series data.
    e.execute("h.YData = [1 1 1 1]; yd = get(h, 'YData'); xd = h.XData;")
        .unwrap();
    assert_eq!(values(&mut e, "yd"), [1.; 4]);
    assert_eq!(values(&mut e, "xd"), [1., 2., 3., 4.]);
    e.execute("set(h, 'XData', [1 2], 'YData', [5 6]); n = numel(h.XData);")
        .unwrap();
    assert_eq!(values(&mut e, "n"), [2.]);
    e.execute("co = ax.ColorOrder; ax.ColorOrder = [1 0 0; 0 0 1]; co2 = get(ax, 'ColorOrder');")
        .unwrap();
    assert_eq!(values(&mut e, "co").len(), 21);
    assert_eq!(values(&mut e, "co2"), [1., 0., 0., 0., 0., 1.]);
}

#[test]
fn property_names_are_case_insensitive_and_unknown_names_are_rejected() {
    let mut e = Engine::default();
    e.execute("plot(1:3); ax = gca; a = ax.xlim; b = ax.XLIM;")
        .unwrap();
    assert_eq!(values(&mut e, "a"), values(&mut e, "b"));
    let error = e.execute("q = ax.NoSuchThing;").unwrap_err();
    assert!(
        error.contains("Unrecognized property 'NoSuchThing' for class 'Axes'"),
        "{error}"
    );
    let error = e.execute("ax.Bogus = 3;").unwrap_err();
    assert!(
        error.contains("Unrecognized property 'Bogus' for class 'Axes'"),
        "{error}"
    );
    let error = e.execute("h = plot(1:3); h.Wobble = 1;").unwrap_err();
    assert!(error.contains("for class 'Line'"), "{error}");
}

#[test]
fn handle_arrays_give_comma_separated_lists() {
    let mut e = Engine::default();
    e.execute("h = plot([1 2; 3 4]); w = [h.LineWidth];")
        .unwrap_err(); // implicit LineWidth has no stored value yet
    e.execute("h = plot([1 2; 3 4]); set(h(1), 'LineWidth', 2); set(h(2), 'LineWidth', 3); w = [h.LineWidth]; c = {h.Type};")
        .unwrap();
    assert_eq!(values(&mut e, "w"), [2., 3.]);
    e.execute("h(2).LineWidth = 4; w2 = [h.LineWidth];")
        .unwrap();
    assert_eq!(values(&mut e, "w2"), [2., 4.]);
    assert!(
        e.execute("h.LineWidth = 1;")
            .unwrap_err()
            .contains("one graphics handle")
    );
}

#[test]
fn gca_and_gcf_create_a_figure_when_none_is_open() {
    let mut e = Engine::default();
    assert!(e.figure.is_none());
    e.execute("ax = gca; ok = isgraphics(ax); ok2 = ishandle(ax); t = ax.Type; f = ax.Parent; n = f.Number;")
        .unwrap();
    assert!(e.figure.is_some());
    assert_eq!(values(&mut e, "ok"), [1.]);
    assert_eq!(values(&mut e, "ok2"), [1.]);
    assert_eq!(text(&mut e, "t"), "axes");
    assert_eq!(values(&mut e, "n"), [1.]);
    e.execute("close all; g = gcf; m = g.Number; live = ishandle(1); dead = ishandle(ax);")
        .unwrap();
    assert_eq!(values(&mut e, "m"), [1.]);
    assert_eq!(values(&mut e, "live"), [1.]);
    assert_eq!(values(&mut e, "dead"), [0.]);
}

#[test]
fn text_objects_have_properties_and_draw_in_svg() {
    let mut e = Engine::default();
    e.execute("plot(0:10, 0:10); t = text(5, 5, 'Peak & <top>', 'Color', [1 0 0], 'FontSize', 15, 'HorizontalAlignment', 'center');")
        .unwrap();
    e.execute("p = t.Position; c = t.Color; s = t.String; fs = t.FontSize; ha = t.HorizontalAlignment; ty = t.Type;")
        .unwrap();
    assert_eq!(values(&mut e, "p"), [5., 5., 0.]);
    assert_eq!(values(&mut e, "c"), [1., 0., 0.]);
    assert_eq!(values(&mut e, "fs"), [15.]);
    assert_eq!(text(&mut e, "s"), "Peak & <top>");
    assert_eq!(text(&mut e, "ha"), "center");
    assert_eq!(text(&mut e, "ty"), "text");
    e.execute(
        "t.Rotation = 45; r = get(t, 'Rotation'); set(t, 'String', 'moved', 'Position', [2 3]);",
    )
    .unwrap();
    assert_eq!(values(&mut e, "r"), [45.]);
    let image = svg(&e);
    assert!(image.contains("data-matrust-text=\"true\""), "{image}");
    assert!(image.contains(">moved</text>"));
    assert!(image.contains("rotate(-45"));
    assert!(image.contains("fill=\"#ff0000\""));
    assert!(image.contains("font-size=\"20\""));
    assert!(image.contains("text-anchor=\"middle\" dominant-baseline=\"central\""));
    e.execute(
        "u = text([1 2], [3 4], {'a', 'b'}); k = numel(u); kids = numel(get(gca, 'Children'));",
    )
    .unwrap();
    assert_eq!(values(&mut e, "k"), [2.]);
    assert_eq!(values(&mut e, "kids"), [4.]);
    assert!(e.execute("text(1, 2, 'x', 'Bogus', 1)").is_err());
    // A replacing plot removes the axes' text objects.
    e.execute("plot(1:3); alive = isgraphics(t);").unwrap();
    assert_eq!(values(&mut e, "alive"), [0.]);
    assert!(!svg(&e).contains("data-matrust-text"));
}

#[test]
fn colormap_names_matrices_and_functions() {
    let mut e = Engine::default();
    e.execute("m = colormap; s = size(m);").unwrap();
    assert_eq!(values(&mut e, "s"), [256., 3.]);
    // Default: viridis end rows (CC0 table).
    let m = values(&mut e, "m");
    assert_eq!([m[0], m[256], m[512]], [0.267004, 0.004874, 0.329415]);
    e.execute("colormap hot; h = colormap; last = h(end, :);")
        .unwrap();
    assert_eq!(values(&mut e, "last"), [1., 1., 1.]);
    e.execute(
        "colormap(gray(3)); g = colormap; f = gcf; fg = f.Colormap; ag = get(gca, 'Colormap');",
    )
    .unwrap();
    assert_eq!(values(&mut e, "g"), [0., 0.5, 1., 0., 0.5, 1., 0., 0.5, 1.]);
    assert_eq!(values(&mut e, "fg"), values(&mut e, "g"));
    assert_eq!(values(&mut e, "ag"), values(&mut e, "g"));
    e.execute("j = jet(9); c = cool(2); hs = hsv(6); cp = copper(5); b = bone(4); p = isequal(parula(5), viridis(5));")
        .unwrap();
    let j = values(&mut e, "j");
    assert_eq!([j[4], j[13], j[22]], [0.5, 1., 0.5]);
    assert_eq!(values(&mut e, "c"), [0., 1., 1., 0., 1., 1.]);
    let hs = values(&mut e, "hs");
    assert_eq!([hs[2], hs[8], hs[14]], [0., 1., 0.]);
    assert_eq!(values(&mut e, "p"), [1.]);
    let cp = values(&mut e, "cp");
    assert_eq!([cp[4], cp[9], cp[14]], [1., 0.7812, 0.4975]);
    assert_eq!(values(&mut e, "b")[3], 1.);
    assert!(
        e.execute("colormap('nonesuch')")
            .unwrap_err()
            .contains("Unknown colormap")
    );
    assert!(e.execute("colormap([2 0 0])").is_err());
    e.execute("colormap default; d = colormap; same = isequal(d, viridis);")
        .unwrap();
    assert_eq!(values(&mut e, "same"), [1.]);
}

#[test]
fn colorbar_limits_label_and_svg() {
    let mut e = Engine::default();
    e.execute("plot(1:3); cb = colorbar; cb.Label.String = 'Depth'; lim = cb.Limits; ty = cb.Type; loc = cb.Location;")
        .unwrap();
    assert_eq!(values(&mut e, "lim"), [0., 1.]);
    assert_eq!(text(&mut e, "ty"), "colorbar");
    assert_eq!(text(&mut e, "loc"), "eastoutside");
    e.execute("clim([-2 6]); l2 = cb.Limits; c = clim; caxis([0 4]); c2 = caxis;")
        .unwrap();
    assert_eq!(values(&mut e, "l2"), [-2., 6.]);
    assert_eq!(values(&mut e, "c"), [-2., 6.]);
    assert_eq!(values(&mut e, "c2"), [0., 4.]);
    e.execute("cb.Limits = [1 3]; c3 = get(gca, 'CLim'); lab = cb.Label.String;")
        .unwrap();
    assert_eq!(values(&mut e, "c3"), [1., 3.]);
    assert_eq!(text(&mut e, "lab"), "Depth");
    let image = svg(&e);
    assert!(
        image.contains("data-matrust-colorbar=\"eastoutside\""),
        "{image}"
    );
    assert!(image.contains(">Depth</text>"));
    // viridis bottom row.
    assert!(image.contains("fill=\"#440154\""));
    e.execute("colorbar('southoutside'); o = cb.Orientation;")
        .unwrap();
    assert_eq!(text(&mut e, "o"), "horizontal");
    assert!(svg(&e).contains("data-matrust-colorbar=\"southoutside\""));
    e.execute("colorbar off; gone = isgraphics(cb);").unwrap();
    assert_eq!(values(&mut e, "gone"), [0.]);
    assert!(!svg(&e).contains("data-matrust-colorbar"));
}

#[test]
fn colorbar_title_supports_dot_chaining_get_set_independence_and_svg_geometry() {
    let mut e = Engine::default();
    e.execute(
        "plot(1:3); title('Axes Title'); h = colorbar; h.Label.String = 'Side Label'; h.Title.String = \"f value\";",
    )
    .unwrap();
    e.execute(
        "tt = h.Title.String; tl = h.Label.String; ta = gca().Title.String; ty = h.Title.Type; same_parent = isequal(h.Title.Parent, h); gt = get(get(h, 'title'), 'string');",
    )
    .unwrap();
    assert_eq!(text(&mut e, "tt"), "f value");
    assert_eq!(text(&mut e, "tl"), "Side Label");
    assert_eq!(text(&mut e, "ta"), "Axes Title");
    assert_eq!(text(&mut e, "ty"), "text");
    assert_eq!(values(&mut e, "same_parent"), [1.]);
    assert_eq!(text(&mut e, "gt"), "f value");

    let image = svg(&e);
    let cb_start = image
        .find("data-matrust-colorbar=\"eastoutside\"")
        .expect("vertical colorbar group");
    let cb_end = cb_start + image[cb_start..].find("</g>").expect("colorbar group end");
    let cb_group = &image[cb_start..cb_end];

    let mut outlines = cb_group
        .split('<')
        .filter_map(|tag| {
            tag.strip_prefix("rect ")?
                .split_once('>')
                .map(|(attrs, _)| attrs)
        })
        .filter(|attrs| attrs.contains("fill=\"none\"") && attrs.contains("stroke=\""));
    let rect_tag = outlines.next().expect("stroked colorbar outline rect");
    assert!(outlines.next().is_none(), "ambiguous colorbar outline");
    let parse_attr = |s: &str, name: &str| -> f64 {
        let pat = format!("{name}=\"");
        let i = s.find(&pat).expect(name) + pat.len();
        let j = i + s[i..].find('"').expect("closing quote");
        s[i..j].parse().expect("float attr")
    };
    let bar_x = parse_attr(rect_tag, "x");
    let bar_y = parse_attr(rect_tag, "y");

    let title_text_end = cb_group
        .find(">f value</text>")
        .expect("colorbar title in colorbar group");
    let title_tag_start = cb_group[..title_text_end]
        .rfind("<text ")
        .expect("title text start");
    let title_tag = &cb_group[title_tag_start..title_text_end];
    assert!(title_tag.contains("text-anchor=\"middle\""), "{title_tag}");
    let title_x = parse_attr(title_tag, "x");
    let title_y = parse_attr(title_tag, "y");
    assert!(
        (title_x - (bar_x + parse_attr(rect_tag, "width") / 2.)).abs() < 0.1,
        "title_x={title_x} bar_x={bar_x}"
    );
    assert!(title_y < bar_y, "title_y={title_y} bar_y={bar_y}");
    assert!(cb_group.contains(">Side Label</text>"), "{cb_group}");
    assert!(image.contains(">Axes Title<"), "{image}");

    // Repeat write via set() changes only ColorBar Title; repeated colorbar preserves it.
    e.execute(
        "set(h.Title, 'String', 'Updated Value'); h2 = colorbar; tt2 = h2.Title.String; tl2 = h2.Label.String; ta2 = gca().Title.String;",
    )
    .unwrap();
    assert_eq!(text(&mut e, "tt2"), "Updated Value");
    assert_eq!(text(&mut e, "tl2"), "Side Label");
    assert_eq!(text(&mut e, "ta2"), "Axes Title");

    // Second figure has an independent colorbar title; figure 1 stays unchanged.
    e.execute(
        "figure; plot(1:3); h_other = colorbar; h_other.Title.String = 'Other'; tt1 = h.Title.String; tt_other = h_other.Title.String;",
    )
    .unwrap();
    assert_eq!(text(&mut e, "tt1"), "Updated Value");
    assert_eq!(text(&mut e, "tt_other"), "Other");

    // Validation bound (> 4096 bytes) still rejects oversized titles.
    e.execute("long_title = repmat('a', 1, 5000);").unwrap();
    assert!(e.execute("h_other.Title.String = long_title;").is_err());
}

#[test]
fn yyaxis_keeps_two_sides_with_own_limits_colours_and_one_legend() {
    let mut e = Engine::default();
    e.execute("x = 1:10; yyaxis left; a = plot(x, x.^2); yyaxis right; b = plot(x, 1000 + x); ylim([1000 1020]); ylabel('Right side');")
        .unwrap();
    e.execute("r = ylim(); ax = gca; loc = ax.YAxisLocation; rc = ax.YColor; yyaxis left; l = ylim(); lc = ax.YColor; n = numel(ax.Children);")
        .unwrap();
    assert_eq!(values(&mut e, "r"), [1000., 1020.]);
    assert_eq!(text(&mut e, "loc"), "right");
    let l = values(&mut e, "l");
    assert!(l[0] <= 1. && l[1] >= 100. && l[1] < 1000., "{l:?}");
    let rc = values(&mut e, "rc");
    assert!((rc[0] - 221. / 255.).abs() < 1e-12, "{rc:?}");
    assert!((values(&mut e, "lc")[2] - 190. / 255.).abs() < 1e-12);
    assert_eq!(values(&mut e, "n"), [2.]);
    // Series take their side colour.
    e.execute("ca = a.Color; cbr = b.Color;").unwrap();
    // gem palette (series_palette.rs LIGHT), same as ColorOrder.
    assert_eq!(values(&mut e, "ca"), [17. / 255., 113. / 255., 190. / 255.]);
    assert_eq!(values(&mut e, "cbr"), [221. / 255., 84. / 255., 0.]);
    // A replacing plot on the left keeps the right side.
    e.execute("c = plot(x, -x); alive = [isgraphics(a) isgraphics(b) isgraphics(c)];")
        .unwrap();
    assert_eq!(values(&mut e, "alive"), [0., 1., 1.]);
    e.execute("legend('left', 'right');").unwrap();
    let image = svg(&e);
    assert!(
        image.contains("data-matrust-right-axis=\"true\""),
        "{image}"
    );
    assert!(image.contains(">1020</text>"));
    assert!(image.contains(">Right side</text>"));
    assert!(image.contains(">left<") && image.contains(">right<"));
    assert!(e.execute("yyaxis middle").is_err());
}

#[derive(Default)]
struct Frames(Arc<Mutex<Vec<GraphicsFrame>>>);
impl HostIo for Frames {
    fn read_line(&mut self, _prompt: &str) -> Option<String> {
        None
    }
    fn draw_frame(&mut self, frame: GraphicsFrame) {
        self.0.lock().unwrap().push(frame);
    }
}

#[test]
fn drawnow_in_a_loop_sends_one_frame_per_update() {
    let mut e = Engine::default();
    let frames = Arc::new(Mutex::new(Vec::new()));
    e.set_host_io(Some(Box::new(Frames(frames.clone()))));
    // No figure open: nothing to draw.
    e.execute("drawnow").unwrap();
    assert!(frames.lock().unwrap().is_empty());
    e.execute("x = linspace(0, 2*pi, 50); h = plot(x, sin(x)); for k = 1:50; h.YData = sin(x + k/5); drawnow; end")
        .unwrap();
    let frames = frames.lock().unwrap();
    assert_eq!(frames.len(), 50);
    // Each frame carries that iteration's data.
    let y = |f: &GraphicsFrame| {
        f.figure.as_ref().unwrap().axes.series[0]
            .points
            .get(0)
            .unwrap()[1]
    };
    assert!((y(&frames[0]) - (1_f64 / 5.).sin()).abs() < 1e-12);
    assert!((y(&frames[49]) - (10_f64).sin()).abs() < 1e-12);
    assert_eq!(frames[49].current, 1);
}

#[test]
fn pause_flushes_a_frame_and_drawnow_without_host_is_a_no_op() {
    let mut e = Engine::default();
    e.execute("plot(1:3); drawnow; drawnow limitrate").unwrap();
    assert!(e.execute("drawnow sideways").is_err());
    let frames = Arc::new(Mutex::new(Vec::new()));
    e.set_host_io(Some(Box::new(Frames(frames.clone()))));
    e.execute("pause(0.01)").unwrap();
    assert_eq!(frames.lock().unwrap().len(), 1);
}

/// Tick-label font size of the colorbar drawn in SVG (pixels = points * 4/3).
fn colorbar_svg_font(e: &Engine) -> String {
    let image = svg(e);
    let start = image.find("data-matrust-colorbar").expect("colorbar drawn");
    let group = &image[start..start + image[start..].find("</g>").unwrap()];
    let at = group.find("font-size=\"").unwrap() + "font-size=\"".len();
    group[at..at + group[at..].find('"').unwrap()].to_owned()
}

#[test]
fn colorbar_font_size_reads_back_as_set_and_matches_the_drawing() {
    let mut e = Engine::default();
    e.execute("plot(1:3); ax = gca; cb = colorbar; d = cb.FontSize;")
        .unwrap();
    // No explicit size: 90 % of the axes font size (10 pt), drawn at 9 pt.
    assert_eq!(values(&mut e, "d"), [9.]);
    assert_eq!(colorbar_svg_font(&e), "12");
    // An explicit size reads back unchanged (was 15 * 0.9 = 13.5) and is drawn.
    e.execute("cb.FontSize = 15; a = cb.FontSize; b = get(cb, 'FontSize');")
        .unwrap();
    assert_eq!(values(&mut e, "a"), [15.]);
    assert_eq!(values(&mut e, "b"), [15.]);
    assert_eq!(colorbar_svg_font(&e), "20");
    // A later axes FontSize leaves the explicit colorbar size alone.
    e.execute("ax.FontSize = 20; c = cb.FontSize;").unwrap();
    assert_eq!(values(&mut e, "c"), [15.]);
    assert_eq!(colorbar_svg_font(&e), "20");
    // Without an explicit size the colorbar follows 90 % of the axes size.
    e.execute("figure; plot(1:3); ax2 = gca; ax2.FontSize = 20; cb2 = colorbar; f = cb2.FontSize;")
        .unwrap();
    assert_eq!(values(&mut e, "f"), [18.]);
    assert_eq!(colorbar_svg_font(&e), "24");
}

#[test]
fn every_color_order_assignment_recolours_automatic_series_only() {
    let mut e = Engine::default();
    e.execute(
        "a = plot(1:3); hold on; b = plot(2:4, 'k'); ax = gca; \
         ax.ColorOrder = [1 0 0; 0 1 0; 0 0 1]; c = plot(3:5);",
    )
    .unwrap();
    e.execute("ca = a.Color; cb = b.Color; cc = c.Color;")
        .unwrap();
    assert_eq!(values(&mut e, "ca"), [1., 0., 0.]);
    assert_eq!(values(&mut e, "cb"), [0., 0., 0.]);
    // Plotted after the first ColorOrder: third row.
    assert_eq!(values(&mut e, "cc"), [0., 0., 1.]);
    // A second assignment recolours both automatic series (the one present
    // at the first assignment and the one plotted after it).
    e.execute(
        "ax.ColorOrder = [0 1 1; 1 0 1; 1 1 0]; ca = a.Color; cb = b.Color; cc = get(c, 'Color');",
    )
    .unwrap();
    assert_eq!(values(&mut e, "ca"), [0., 1., 1.]);
    assert_eq!(values(&mut e, "cb"), [0., 0., 0.]);
    assert_eq!(values(&mut e, "cc"), [1., 1., 0.]);
    let image = svg(&e);
    assert!(
        image.contains("#00ffff") && image.contains("#ffff00"),
        "{image}"
    );
    assert!(!image.contains("#ff0000") && !image.contains("#0000ff"));
    // And a third, through set.
    e.execute("set(ax, 'ColorOrder', [0.5 0.5 0.5]); ca = a.Color; cb = b.Color; cc = c.Color;")
        .unwrap();
    for name in ["ca", "cc"] {
        assert!(
            values(&mut e, name)
                .iter()
                .all(|v| (v - 128. / 255.).abs() < 1e-12),
            "{name}"
        );
    }
    assert_eq!(values(&mut e, "cb"), [0., 0., 0.]);
}
