/// Font of the first text shape with exactly this content.
fn text_font(shape: &egui::epaint::Shape, label: &str) -> Option<egui::FontId> {
    match shape {
        egui::epaint::Shape::Text(t) if t.galley.text() == label => t
            .galley
            .job
            .sections
            .first()
            .map(|section| section.format.font_id.clone()),
        egui::epaint::Shape::Vec(v) => v.iter().rev().find_map(|s| text_font(s, label)),
        _ => None,
    }
}

fn painted_font(output: &egui::FullOutput, label: &str) -> egui::FontId {
    output
        .shapes
        .iter()
        .find_map(|s| text_font(&s.shape, label))
        .unwrap_or_else(|| panic!("{label:?} was not painted"))
}

fn has_fill(shape: &egui::epaint::Shape, fill: egui::Color32, covers: egui::Rect) -> bool {
    match shape {
        egui::epaint::Shape::Rect(r) => r.fill == fill && r.rect.contains_rect(covers),
        egui::epaint::Shape::Vec(v) => v.iter().any(|s| has_fill(s, fill, covers)),
        _ => false,
    }
}

fn panel_rect(ctx: &egui::Context, name: &str) -> egui::Rect {
    egui::containers::panel::PanelState::load(ctx, egui::Id::new(name))
        .unwrap_or_else(|| panic!("panel {name} was not shown"))
        .outer_rect
}

#[test]
fn status_bar_is_canvas_filled_with_caption_text() {
    for size in [egui::vec2(1440., 900.), egui::vec2(960., 650.)] {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.status = "Status density probe".into();
        let mut output = frame_at(&mut app, &ctx, size, vec![]);
        for _ in 0..3 {
            output = frame_at(&mut app, &ctx, size, vec![]);
        }
        assert_eq!(
            painted_font(&output, "Status density probe").size,
            tokens::CAPTION
        );
        let status = panel_rect(&ctx, "status");
        let canvas = tokens::palette(&ctx).canvas;
        assert!(
            output
                .shapes
                .iter()
                .any(|s| has_fill(&s.shape, canvas, status.shrink(1.))),
            "status bar must be filled with the canvas colour at {size:?}"
        );
    }
}

#[test]
fn figure_empty_heading_is_heading_semibold() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.layout.mode = session::PaneMode::Figure;
    let mut output = frame_at(&mut app, &ctx, egui::vec2(1440., 900.), vec![]);
    for _ in 0..3 {
        output = frame_at(&mut app, &ctx, egui::vec2(1440., 900.), vec![]);
    }
    // The product chrome replaced the welcome title with a panel empty state.
    assert_eq!(visible_text_count(&output, chrome::NO_FIGURE), 1);
    let font = painted_font(&output, chrome::NO_FIGURE);
    assert_eq!(font, tokens::semibold(tokens::HEADING));
}

#[test]
fn toolstrip_tabs_keep_body_size_in_compact_windows() {
    for (size, expected) in [
        (egui::vec2(1440., 900.), tokens::BODY),
        (egui::vec2(960., 650.), tokens::BODY),
    ] {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        let mut output = frame_at(&mut app, &ctx, size, vec![]);
        for _ in 0..3 {
            output = frame_at(&mut app, &ctx, size, vec![]);
        }
        for label in ["Home", "Plots", "Apps"] {
            assert_eq!(
                painted_font(&output, label).size,
                expected,
                "{label} at {size:?}"
            );
        }
    }
}

/// A new bottom panel is sized from the parent's interact height. Dense rows
/// must say so, or the first frame leaves a gap that moves every panel above.
#[test]
fn compact_bottom_panels_sit_flush_on_the_first_frame() {
    let size = egui::vec2(960., 650.);
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    frame_at(&mut app, &ctx, size, vec![]);
    let status = panel_rect(&ctx, "status");
    assert_eq!(status.bottom(), size.y);
    let first = status.height();
    for _ in 0..3 {
        frame_at(&mut app, &ctx, size, vec![]);
    }
    assert_eq!(first, panel_rect(&ctx, "status").height());
}

/// Text shapes with exactly this content whose ink lies inside their clip.
fn visible_text_count(output: &egui::FullOutput, label: &str) -> usize {
    fn visit(shape: &egui::epaint::Shape, clip: egui::Rect, label: &str) -> usize {
        match shape {
            egui::epaint::Shape::Text(t) if t.galley.text() == label => {
                usize::from(clip.contains_rect(shape.visual_bounding_rect()))
            }
            egui::epaint::Shape::Vec(v) => v.iter().map(|s| visit(s, clip, label)).sum(),
            _ => 0,
        }
    }
    output
        .shapes
        .iter()
        .map(|s| visit(&s.shape, s.clip_rect, label))
        .sum()
}

// The 4-up tile rows are chrome. With 24 px rows a 960x650 tile kept 106 px
// for its axes, below the 109 px its title band needs, so the title was dropped
// (detached_figures::tiles default fit, which the Linux run skips).
#[test]
fn compact_four_up_tiles_use_dense_rows_and_keep_every_axes_title() {
    let titles = ["Tile one", "Tile two", "Tile three", "Tile four"];
    for (size, label) in [
        (egui::vec2(960., 650.), tokens::CAPTION),
        (egui::vec2(1440., 900.), tokens::BODY),
    ] {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let mut app = Workspace::with_context(ctx.clone(), true);
        let script: String = titles
            .iter()
            .enumerate()
            .map(|(i, t)| format!("figure({});plot([1 3 2]);title('{t}');", i + 1))
            .collect();
        app.run(script);
        let settle = |app: &mut Workspace| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while app.busy && std::time::Instant::now() < deadline {
                app.receive();
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            assert!(!app.busy, "{}", app.status);
        };
        settle(&mut app);
        app.show_plot_pane();
        app.layout.mode = layout::PaneMode::Figure;
        app.layout.figure_tiling = Some(crate::session::FigureTiling::Four);
        let mut output = frame_at(&mut app, &ctx, size, vec![]);
        for _ in 0..16 {
            settle(&mut app);
            output = frame_at(&mut app, &ctx, size, vec![]);
        }
        for title in titles {
            assert_eq!(
                visible_text_count(&output, title),
                1,
                "{title} must fit its tile at {size:?}"
            );
        }
        // The tile controls live in the card's overflow menu; the header
        // itself is the dense card header.
        assert_eq!(painted_font(&output, "Figures").size, tokens::BODY);
        let _ = label;
    }
}
