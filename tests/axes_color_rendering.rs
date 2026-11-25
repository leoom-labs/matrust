//! Public rendering/export contracts; RGB fixtures are authored, not MATLAB output.
use matrust::{
    Engine,
    plot::{AxesColors, AxesGridStyle, BaseTheme, ColorMode, Figure},
};

fn fixture() -> Figure {
    let mut engine = Engine::default();
    engine.execute("plot([0 1 2],[0 2 1]);axis([0 4 0 4]);grid on;title('Title ink');xlabel('X ink');ylabel('Y ink');").unwrap();
    let mut figure = engine.figure.unwrap();
    figure.colors = Some(AxesColors {
        legend: None,
        background: Some([0.2, 0.4, 0.6]),
        x: Some([1., 0., 0.]),
        y: Some([0., 0., 1.]),
        z: None,
        title: Some([0., 1., 0.]),
        xlabel: Some([1., 0., 1.]),
        ylabel: Some([0., 1., 1.]),
        grid: Some(AxesGridStyle {
            color: [0.5; 3],
            alpha: 0.5,
            x_mode: ColorMode::Manual,
            y_mode: ColorMode::Manual,
            color_mode: ColorMode::Auto,
            alpha_mode: ColorMode::Manual,
        }),
    });
    figure.validate().unwrap();
    figure
}

#[test]
fn absent_axes_overrides_preserve_svg_bytes_and_explicit_fields_remain_host_independent() {
    let figure = fixture();
    let before = figure.clone();
    for theme in [BaseTheme::Light, BaseTheme::Dark] {
        let svg = figure.svg_with_theme(theme);
        for (label, color) in [
            ("Title ink", "#00ff00"),
            ("X ink", "#ff00ff"),
            ("Y ink", "#00ffff"),
        ] {
            assert!(
                svg.contains(&format!("fill=\"{color}\">{label}</text>")),
                "{label}: {svg}"
            );
        }
        assert!(svg.contains("fill=\"#336699\""));
        assert!(svg.contains("stroke=\"#ff0000\" stroke-opacity=\"0.5\""));
        assert!(svg.contains("stroke=\"#0000ff\" stroke-opacity=\"0.5\""));
        assert!(figure.same_snapshot(&before));
        assert!(
            figure.series[0]
                .points
                .same_storage(&before.series[0].points)
        );

        let mut absent = figure.clone();
        absent.colors = None;
        let legacy = absent.svg_with_theme(theme);
        absent.colors = Some(AxesColors::default());
        assert_eq!(absent.svg_with_theme(theme), legacy);
        assert!(legacy.contains("stroke-opacity=\"0.15\""));
        absent.appearance = None;
        assert!(!absent.svg_with_theme(theme).contains("stroke-opacity="));
    }
}

#[test]
fn zero_alpha_retains_directional_geometry_but_never_changes_samples_or_ticks() {
    let mut figure = fixture();
    let original = figure.clone();
    figure.colors.as_mut().unwrap().grid.as_mut().unwrap().alpha = 0.;
    let svg = figure.svg();
    assert!(svg.contains("stroke=\"#ff0000\" stroke-opacity=\"0\""));
    assert!(svg.contains("stroke=\"#0000ff\" stroke-opacity=\"0\""));
    assert!(svg.contains("font-size=\"12\" fill=\"#ff0000\""));
    assert!(svg.contains("font-size=\"12\" fill=\"#0000ff\""));
    assert_eq!(figure.bounds(), original.bounds());
    assert!(
        figure.series[0]
            .points
            .same_storage(&original.series[0].points)
    );
}

#[cfg(feature = "plot-export")]
#[test]
fn tight_export_preserves_each_axes_background_and_selected_axes_identity_atomically() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let dir = tempfile::tempdir().unwrap();
    let mut engine = Engine::default();
    engine.set_file_directory(dir.path()).unwrap();
    engine.execute("subplot(1,2,1);plot([0 1],[1 0]);set(gca,'Color',[0.2 0.4 0.6]);subplot(1,2,2);plot([0 1],[0 1]);set(gca,'Color',[0.7 0.3 0.1]);selected=gca;").unwrap();
    let original = engine.figure.clone().unwrap();
    engine.execute("exportgraphics(gcf,'both.svg','BackgroundColor','white');exportgraphics(selected,'selected.svg','BackgroundColor','white');").unwrap();
    let both = std::fs::read_to_string(dir.path().join("both.svg")).unwrap();
    let selected = std::fs::read_to_string(dir.path().join("selected.svg")).unwrap();
    assert!(both.contains("fill=\"#336699\"") && both.contains("fill=\"#b34d1a\""));
    assert!(!selected.contains("fill=\"#336699\""));
    assert!(selected.contains("fill=\"#b34d1a\""));
    assert!(original.same_snapshot(engine.figure.as_ref().unwrap()));
    for ((_, a), (_, b)) in original
        .all_axes()
        .zip(engine.figure.as_ref().unwrap().all_axes())
    {
        for (a, b) in a.series.iter().zip(&b.series) {
            assert!(a.points.same_storage(&b.points));
        }
    }
    let path = dir.path().join("keep.png");
    std::fs::write(&path, b"existing destination").unwrap();
    let cancel = engine.cancellation();
    cancel.store(true, Ordering::Relaxed);
    assert!(engine.execute("exportgraphics(gcf,'keep.png');").is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"existing destination");
    assert!(original.same_snapshot(engine.figure.as_ref().unwrap()));
    cancel.store(false, Ordering::Relaxed);
    // Actual raster and aspect-fit SVG consumers agree on opaque axes backgrounds.
    let figure = fixture();
    let rendered = matrust::export::render_figure_in_theme_with_cancel(
        &figure,
        [960, 560],
        BaseTheme::Dark,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(rendered.get_pixel(90, 80).0, [51, 102, 153, 255]);
}

#[test]
fn single_ruler_rgb_controls_implicit_label_but_explicit_label_wins_in_both_themes() {
    for direction in ["X", "Y"] {
        for mode in ["auto", "manual"] {
            let mut engine = Engine::default();
            engine.execute("plot([0 1 2],[0 2 1]);axis([0 4 0 4]);grid off;xlabel('X only');ylabel('Y only');").unwrap();
            engine
                .execute(&format!(
                    "set(gca,'{direction}Color',[1 0 0],'{direction}ColorMode','{mode}');"
                ))
                .unwrap();
            let original = engine.figure.clone().unwrap();
            for host in [BaseTheme::Light, BaseTheme::Dark] {
                let svg = original.svg_with_theme(host);
                assert!(svg.contains(&format!("fill=\"#ff0000\">{direction} only</text>")));
                let other = if direction == "X" { "Y" } else { "X" };
                assert!(!svg.contains(&format!("fill=\"#ff0000\">{other} only</text>")));
                assert!(svg.contains("font-size=\"12\" fill=\"#ff0000\""));
                assert!(original.same_snapshot(engine.figure.as_ref().unwrap()));
            }
            let label = if direction == "X" { "xlabel" } else { "ylabel" };
            engine
                .execute(&format!("{label}('{direction} only','Color',[0 1 0]);"))
                .unwrap();
            let explicit = engine.figure.as_ref().unwrap();
            for host in [BaseTheme::Light, BaseTheme::Dark] {
                let svg = explicit.svg_with_theme(host);
                assert!(svg.contains(&format!("fill=\"#00ff00\">{direction} only</text>")));
                assert!(svg.contains("font-size=\"12\" fill=\"#ff0000\""));
                assert!(
                    explicit.series[0]
                        .points
                        .same_storage(&original.series[0].points)
                );
            }
        }
    }
    // The historical no-ruler/no-label path retains exact lexical SVG behavior.
    let mut engine = Engine::default();
    engine
        .execute("plot([0 1],[1 0]);xlabel('Unstyled');")
        .unwrap();
    let mut figure = engine.figure.unwrap();
    figure.appearance = None;
    let before = figure.svg();
    figure.colors = Some(AxesColors::default());
    assert_eq!(figure.svg(), before);
}

/// Axes `Layer` in exported SVG pixels: opaque blue data covers the whole
/// box; red grid and green inward ticks show through only with `top`.
#[cfg(feature = "plot-export")]
#[test]
fn axes_color_rendering_layer_orders_grid_and_ticks_in_svg_pixels() {
    const SCALE: f32 = 4.;
    let render = |layer: &str| {
        let mut engine = Engine::default();
        engine
            .execute(&format!(
                "fill([0 10 10 0],[0 0 10 10],[0 0 1],'EdgeColor','none'); axis([0 10 0 10]); grid on;
                 set(gca,'GridColor',[1 0 0],'GridAlpha',1,'XColor',[0 0.8 0],'YColor',[0 0.8 0], ...
                     'TickDir','in','TickLength',[0.05 0.05],'Layer','{layer}');"
            ))
            .unwrap();
        let svg = engine.figure.as_ref().unwrap().svg();
        // The data box is the clip rectangle.
        let clip = svg.split("<clipPath").nth(1).unwrap();
        let attr = |name: &str| -> f32 {
            let start = clip.find(&format!(" {name}=\"")).unwrap() + name.len() + 3;
            clip[start..].split('"').next().unwrap().parse().unwrap()
        };
        let [left, top, width, height] = [attr("x"), attr("y"), attr("width"), attr("height")];
        let tree =
            resvg::usvg::Tree::from_str(&svg, &matrust::export::svg_render_options()).unwrap();
        let size = tree.size();
        let mut pixmap = resvg::tiny_skia::Pixmap::new(
            (size.width() * SCALE) as u32,
            (size.height() * SCALE) as u32,
        )
        .unwrap();
        pixmap.fill(resvg::tiny_skia::Color::WHITE);
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(SCALE, SCALE),
            &mut pixmap.as_mut(),
        );
        let sample = |x: f32, y: f32| {
            let p = pixmap
                .pixel((x * SCALE) as u32, (y * SCALE) as u32)
                .unwrap();
            [p.red(), p.green(), p.blue()]
        };
        let px = |v: f32| left + width * v / 10.;
        let py = |v: f32| top + height * (1. - v / 10.);
        let tick = width.max(height) * 0.05;
        [
            // Interior crossing of the x = 4 grid line, far from every tick.
            sample(px(4.), py(5.)),
            // Near-edge crossing: the x = 4 tick over the same grid line.
            sample(px(4.), top + height - tick / 2.),
            // Off-grid data.
            sample(px(5.), py(5.)),
        ]
    };
    let blue = [0, 0, 255];
    assert_eq!(render("bottom"), [blue; 3]);
    assert_eq!(render("Top"), [[255, 0, 0], [0, 204, 0], blue]);
}

/// Axes `Layer 'top'` draws grid lines and inward ticks above axes overlays
/// such as an opaque filled `rectangle` in exported SVG pixels.
#[cfg(feature = "plot-export")]
#[test]
fn axes_color_rendering_layer_top_draws_grid_and_ticks_over_rectangle() {
    const SCALE: f32 = 4.;
    let render = |layer: &str| {
        let mut engine = Engine::default();
        engine
            .execute(&format!(
                "rectangle('Position',[0 0 10 10],'FaceColor',[0 0 1],'EdgeColor','none');
                 axis([0 10 0 10]); grid on;
                 set(gca,'GridColor',[1 0 0],'GridAlpha',1,'XColor',[0 0.8 0],'YColor',[0 0.8 0], ...
                     'TickDir','in','TickLength',[0.05 0.05],'Layer','{layer}');"
            ))
            .unwrap();
        let svg = engine.figure.as_ref().unwrap().svg();
        let clip = svg.split("<clipPath").nth(1).unwrap();
        let attr = |name: &str| -> f32 {
            let start = clip.find(&format!(" {name}=\"")).unwrap() + name.len() + 3;
            clip[start..].split('"').next().unwrap().parse().unwrap()
        };
        let [left, top, width, height] = [attr("x"), attr("y"), attr("width"), attr("height")];
        let tree =
            resvg::usvg::Tree::from_str(&svg, &matrust::export::svg_render_options()).unwrap();
        let size = tree.size();
        let mut pixmap = resvg::tiny_skia::Pixmap::new(
            (size.width() * SCALE) as u32,
            (size.height() * SCALE) as u32,
        )
        .unwrap();
        pixmap.fill(resvg::tiny_skia::Color::WHITE);
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(SCALE, SCALE),
            &mut pixmap.as_mut(),
        );
        let sample = |x: f32, y: f32| {
            let p = pixmap
                .pixel((x * SCALE) as u32, (y * SCALE) as u32)
                .unwrap();
            [p.red(), p.green(), p.blue()]
        };
        let px = |v: f32| left + width * v / 10.;
        let py = |v: f32| top + height * (1. - v / 10.);
        let tick = width.max(height) * 0.05;
        [
            sample(px(4.), py(5.)),
            sample(px(4.), top + height - tick / 2.),
            sample(px(5.), py(5.)),
        ]
    };
    let blue = [0, 0, 255];
    assert_eq!(render("bottom"), [blue; 3]);
    assert_eq!(render("Top"), [[255, 0, 0], [0, 204, 0], blue]);
}

/// Layer 'top' must not paint the grid over the yyaxis right axis line.
#[cfg(feature = "plot-export")]
#[test]
fn axes_color_rendering_layer_top_keeps_yyaxis_right_axis_colour() {
    const SCALE: f32 = 4.;
    let mut engine = Engine::default();
    engine
        .execute(
            "yyaxis left; plot([0 1],[0 1]); ylim([0 1]); \
             yyaxis right; plot([0 1],[0 1]); ylim([0 1]); \
             xlim([0 1]); grid on; xticks([0 1]); \
             set(gca,'Layer','top','GridColor',[0 0 1],'GridAlpha',1,'YColor',[1 0 0]);",
        )
        .unwrap();
    let svg = engine.figure.as_ref().unwrap().svg();
    let layer = svg.find("<g data-matrust-layer=\"top\">").unwrap();
    let axis = svg.find("data-matrust-right-axis=\"true\"").unwrap();
    assert!(layer < axis, "right axis must follow the top group");
    let clip = svg.split("<clipPath").nth(1).unwrap();
    let attr = |name: &str| -> f32 {
        let start = clip.find(&format!(" {name}=\"")).unwrap() + name.len() + 3;
        clip[start..].split('"').next().unwrap().parse().unwrap()
    };
    let [left, top, width, height] = [attr("x"), attr("y"), attr("width"), attr("height")];
    let tree = resvg::usvg::Tree::from_str(&svg, &matrust::export::svg_render_options()).unwrap();
    let size = tree.size();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(
        (size.width() * SCALE) as u32,
        (size.height() * SCALE) as u32,
    )
    .unwrap();
    pixmap.fill(resvg::tiny_skia::Color::WHITE);
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(SCALE, SCALE),
        &mut pixmap.as_mut(),
    );
    let sample = |x: f32, y: f32| {
        let p = pixmap
            .pixel((x * SCALE) as u32, (y * SCALE) as u32)
            .unwrap();
        [p.red(), p.green(), p.blue()]
    };
    // The x = 1 grid line lies on the right axis. Mid-height misses the series.
    let on_axis = sample(left + width, top + height * 0.37);
    assert_eq!(on_axis, [255, 0, 0], "right axis line {on_axis:?}");
}
