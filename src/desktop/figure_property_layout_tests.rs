use super::*;
use egui::{Event, Pos2, Rect, Vec2};

fn editor() -> Editor {
    let mut engine = matrust::Engine::default();
    engine.execute("subplot(1,2,1);plot([1 2 3],[2 4 3]);grid on;hold on;plot([1 2 3],[1 2 1]);subplot(1,2,2);plot([1 2],[3 4]);grid on;subplot(1,2,1);").unwrap();
    Editor::new(1, &engine.figure.unwrap(), Some([0., 4., 0., 5.]))
}
fn frame(
    ctx: &egui::Context,
    editor: &mut Editor,
    size: Vec2,
    events: Vec<Event>,
) -> (egui::FullOutput, Action) {
    let mut action = Action::None;
    let output = crate::headless::run(
        ctx,
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |root| {
            let ctx = &root.ctx().clone();
            action = editor.show(ctx);
        },
    );
    (output, action)
}
fn find(shape: &egui::Shape, label: &str) -> Option<Rect> {
    match shape {
        egui::Shape::Text(t) if t.galley.text() == label => {
            Some(Rect::from_min_size(t.pos, t.galley.size()))
        }
        egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, label)),
        _ => None,
    }
}
fn point(output: &egui::FullOutput, label: &str, size: Vec2) -> Pos2 {
    let (rect, clip) = output
        .shapes
        .iter()
        .find_map(|s| find(&s.shape, label).map(|r| (r, s.clip_rect)))
        .unwrap_or_else(|| panic!("Missing {label}"));
    assert!(
        clip.expand(0.1).contains_rect(rect),
        "Clipped {label}: {rect:?} in {clip:?}"
    );
    assert!(
        Rect::from_min_size(Pos2::ZERO, size).contains_rect(rect),
        "Outside {label}: {rect:?}"
    );
    rect.center()
}
fn click(ctx: &egui::Context, editor: &mut Editor, size: Vec2, pos: Pos2) -> Action {
    let mut action = Action::None;
    for pressed in [true, false] {
        action = frame(
            ctx,
            editor,
            size,
            vec![
                Event::PointerMoved(pos),
                Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        )
        .1;
    }
    action
}
fn escape(pressed: bool) -> Event {
    Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn property_layout_all_tabs_and_long_errors_fit_small_viewports() {
    for tab in [Tab::Axes, Tab::Series, Tab::Layout] {
        let ctx = egui::Context::default();
        let mut e = editor();
        let source = e.original.clone();
        e.tab = tab;
        e.draft.series[0].label = "Very long series label ".repeat(120);
        e.error = Some("Failed to apply figure properties. ".repeat(200));
        for size in [
            Vec2::new(1440., 900.),
            Vec2::new(960., 650.),
            Vec2::new(640., 480.),
            Vec2::new(480., 320.),
        ] {
            for _ in 0..4 {
                frame(&ctx, &mut e, size, vec![]);
            }
            let (output, _) = frame(&ctx, &mut e, size, vec![]);
            for label in [
                "Figure 1 properties",
                "Axes and labels",
                "Series",
                "Layout",
                "Apply changes",
                "Cancel",
                "Restore original",
            ] {
                point(&output, label, size);
            }
            let rect = ctx
                .memory(|m| m.area_rect(egui::Id::new("figure_properties")))
                .unwrap();
            assert!(
                Rect::from_min_size(Pos2::ZERO, size).contains_rect(rect),
                "{tab:?}: {rect:?} in {size:?}"
            );
        }
        assert!(e.original.same_snapshot(&source));
        assert!((e.draft.series[0].points).same_storage(&source.series[0].points));
    }
}

#[test]
fn property_layout_invalid_apply_restore_and_success_preserve_shared_samples() {
    let ctx = egui::Context::default();
    let size = Vec2::new(480., 320.);
    let mut e = editor();
    let source = e.original.clone();
    e.draft.title = "Changed draft".into();
    e.auto[0] = false;
    e.limits[0] = ["9".into(), "1".into()];
    for _ in 0..4 {
        frame(&ctx, &mut e, size, vec![]);
    }
    let (output, _) = frame(&ctx, &mut e, size, vec![]);
    assert!(matches!(
        click(&ctx, &mut e, size, point(&output, "Apply changes", size)),
        Action::None
    ));
    assert!(e.error.is_some() && !e.saving);
    assert_eq!(e.draft.title, "Changed draft");
    assert!(e.original.same_snapshot(&source));
    for _ in 0..4 {
        frame(&ctx, &mut e, size, vec![]);
    }
    let (output, _) = frame(&ctx, &mut e, size, vec![]);
    point(&output, e.error.as_ref().unwrap(), size);
    click(&ctx, &mut e, size, point(&output, "Restore original", size));
    assert!(e.error.is_none());
    assert!(e.draft.same_snapshot(&source));
    e.draft.title = "Applied title".into();
    e.error = Some("A previous failure".into());
    for _ in 0..4 {
        frame(&ctx, &mut e, size, vec![]);
    }
    let (output, _) = frame(&ctx, &mut e, size, vec![]);
    let Action::Apply(edit) = click(&ctx, &mut e, size, point(&output, "Apply changes", size))
    else {
        panic!("apply missing")
    };
    assert!(e.saving && e.error.is_none());
    assert_eq!(edit.updated.title, "Applied title");
    assert!(edit.original.same_snapshot(&source));
    assert!((edit.updated.series[0].points).same_storage(&source.series[0].points));
    for _ in 0..4 {
        frame(&ctx, &mut e, size, vec![]);
    }
    let (output, _) = frame(&ctx, &mut e, size, vec![]);
    point(&output, "Applying changes\u{2026}", size);
    assert!(matches!(
        frame(&ctx, &mut e, size, vec![escape(true)]).1,
        Action::None
    ));
}

#[test]
fn property_layout_escape_closes_series_popup_before_discarding_draft() {
    let ctx = egui::Context::default();
    let size = Vec2::new(640., 480.);
    let mut e = editor();
    e.show_series();
    for _ in 0..4 {
        frame(&ctx, &mut e, size, vec![]);
    }
    let (output, _) = frame(&ctx, &mut e, size, vec![]);
    let selected = format!("1. {}", e.draft.series[0].label);
    click(&ctx, &mut e, size, point(&output, &selected, size));
    assert!(egui::Popup::is_any_open(&ctx));
    assert!(matches!(
        frame(&ctx, &mut e, size, vec![escape(true)]).1,
        Action::None
    ));
    assert!(!egui::Popup::is_any_open(&ctx));
    frame(&ctx, &mut e, size, vec![escape(false)]);
    assert!(matches!(
        frame(&ctx, &mut e, size, vec![escape(true)]).1,
        Action::Cancel
    ));
}

#[test]
fn property_layout_tabs_keep_independent_scroll_positions() {
    let ctx = egui::Context::default();
    let size = Vec2::new(640., 480.);
    let mut e = editor();
    for _ in 0..4 {
        frame(&ctx, &mut e, size, vec![]);
    }
    let (output, _) = frame(&ctx, &mut e, size, vec![]);
    let scroll = point(&output, "Shared figure title", size);
    for _ in 0..8 {
        frame(
            &ctx,
            &mut e,
            size,
            vec![
                Event::PointerMoved(scroll),
                Event::MouseWheel {
                    phase: egui::TouchPhase::Move,
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0., -120.),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    for _ in 0..20 {
        frame(&ctx, &mut e, size, vec![]);
    }
    let (output, _) = frame(&ctx, &mut e, size, vec![]);
    point(&output, "Hold new plots", size);
    click(&ctx, &mut e, size, point(&output, "Layout", size));
    for _ in 0..4 {
        frame(&ctx, &mut e, size, vec![]);
    }
    let (output, _) = frame(&ctx, &mut e, size, vec![]);
    point(
        &output,
        "Move or resize the selected axes. Other axes keep their data.",
        size,
    );
    click(&ctx, &mut e, size, point(&output, "Axes and labels", size));
    for _ in 0..4 {
        frame(&ctx, &mut e, size, vec![]);
    }
    let (output, _) = frame(&ctx, &mut e, size, vec![]);
    point(&output, "Hold new plots", size);
    assert!(e.draft.same_snapshot(&e.original));
}

#[test]
fn line_width_properties_enable_edit_validate_and_share_samples() {
    let ctx = egui::Context::default();
    let size = Vec2::new(640., 480.);
    let mut e = editor();
    e.show_series();
    let original = e.original.clone();
    for _ in 0..4 {
        frame(&ctx, &mut e, size, vec![]);
    }
    let (output, _) = frame(&ctx, &mut e, size, vec![]);
    click(
        &ctx,
        &mut e,
        size,
        point(&output, "Custom line width", size),
    );
    assert_eq!(e.draft.series[0].line_width, Some(1.5));
    e.draft.series[0].line_width = Some(4.);
    let (output, _) = frame(&ctx, &mut e, size, vec![]);
    let action = click(&ctx, &mut e, size, point(&output, "Apply changes", size));
    let Action::Apply(change) = action else {
        panic!("Width edit was not applied")
    };
    assert_eq!(change.updated.series[0].line_width, Some(4.));
    assert!((change.updated.series[0].points).same_storage(&original.series[0].points));
    assert!(e.original.same_snapshot(&original));
    e.draft.series[0].line_width = Some(f64::INFINITY);
    assert!(e.candidate().is_err());
}

#[test]
fn marker_styles_properties_select_shape_size_and_keep_shared_samples() {
    let ctx = egui::Context::default();
    let size = Vec2::new(640., 480.);
    let mut engine = matrust::Engine::default();
    engine.execute("plot([1 2],'ro-');").unwrap();
    let original = engine.figure.unwrap();
    let mut e = Editor::new(1, &original, None);
    e.show_series();
    for _ in 0..4 {
        frame(&ctx, &mut e, size, vec![]);
    }
    let (output, _) = frame(&ctx, &mut e, size, vec![]);
    click(
        &ctx,
        &mut e,
        size,
        point(&output, "Custom marker size", size),
    );
    assert_eq!(e.draft.series[0].marker_size, Some(4.5));
    let (output, _) = frame(&ctx, &mut e, size, vec![]);
    click(&ctx, &mut e, size, point(&output, "Legacy default", size));
    for _ in 0..3 {
        frame(&ctx, &mut e, size, vec![]);
    }
    let (output, _) = frame(&ctx, &mut e, size, vec![]);
    click(&ctx, &mut e, size, point(&output, "Square", size));
    assert_eq!(
        e.draft.series[0].marker_shape,
        Some(matrust::plot::MarkerShape::Square)
    );
    e.draft.series[0].marker_size = Some(12.);
    let (output, _) = frame(&ctx, &mut e, size, vec![]);
    let Action::Apply(change) = click(&ctx, &mut e, size, point(&output, "Apply changes", size))
    else {
        panic!("Marker edit was not applied")
    };
    assert_eq!(change.updated.series[0].marker_size, Some(12.));
    assert!((change.updated.series[0].points).same_storage(&original.series[0].points));
    assert!(e.original.same_snapshot(&original));
}

fn outside_layout_editor() -> Editor {
    let mut engine = matrust::Engine::default();
    engine.execute("subplot('Position',[0.1 0.2 0.3 0.6]);plot([0 1],[0 1]);left=gca;subplot('Position',[0.6 0.2 0.3 0.6]);plot([0 1],[1 0]);right=gca;axes(left);legend('Left','Location','westoutside');axes(right);legend('Right','Location','eastoutside');").unwrap();
    let mut editor = Editor::new(1, &engine.figure.unwrap(), None);
    editor.show_layout();
    editor
}

fn preview_rectangle(output: &egui::FullOutput) -> (Rect, Rect) {
    fn find_rect(shape: &egui::Shape) -> Option<Rect> {
        match shape {
            egui::Shape::Rect(rect) if rect.fill == crate::tokens::LIGHT.surface_sunken => {
                Some(rect.rect)
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(find_rect),
            _ => None,
        }
    }
    let matches: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|s| find_rect(&s.shape).map(|r| (r, s.clip_rect)))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "Expected actual Layout diagram background"
    );
    matches[0]
}

fn layout_caption() -> &'static str {
    "Selected data box is blue; outer allocations are gray outlines and outside legends are gold. Overlapping allocations cannot be applied."
}

fn layout_context() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.set_theme(egui::Theme::Light);
    // Match the actual desktop's spacing; do not hide clipping by tightening it.
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(10., 9.);
        style.animation_time = 0.;
    });
    ctx
}

#[test]
fn layout_preview_complete_diagram_caption_and_footer_fit_compact_and_wide() {
    for size in [Vec2::new(960., 650.), Vec2::new(1440., 900.)] {
        let ctx = layout_context();
        let mut editor = outside_layout_editor();
        let source = editor.original.clone();
        for _ in 0..6 {
            frame(&ctx, &mut editor, size, vec![]);
        }
        let (output, _) = frame(&ctx, &mut editor, size, vec![]);
        let (rect, clip) = preview_rectangle(&output);
        assert!(
            clip.expand(0.1).contains_rect(rect),
            "Preview clipped: {rect:?} in {clip:?}"
        );
        assert!(Rect::from_min_size(Pos2::ZERO, size).contains_rect(rect));
        assert!(rect.height() >= 64. - 0.1);
        if size.y > 650. {
            assert!((rect.height() - rect.width() * 0.38).abs() < 0.1);
        } else {
            assert!(rect.height() < rect.width() * 0.38);
        }
        for label in [
            "Axes 1",
            "Axes 2",
            layout_caption(),
            "Apply changes",
            "Cancel",
            "Restore original",
        ] {
            let pos = point(&output, label, size);
            if label.starts_with("Axes ") {
                assert!(rect.contains(pos));
            }
        }
        // Inspect actual painted panel and allocation/legend geometry, not only
        // the existence of labels in an egui shape list.
        fn contained_shapes(shape: &egui::Shape, clip: Rect, preview: Rect, count: &mut usize) {
            match shape {
                egui::Shape::Rect(s) if preview.contains(s.rect.center()) && s.rect != preview => {
                    if s.fill == crate::tokens::LIGHT.accent_soft
                        || s.fill == crate::tokens::LIGHT.warning.gamma_multiply(0.3)
                        || s.stroke.color == crate::tokens::LIGHT.text_disabled
                    {
                        assert!(preview.expand(0.1).contains_rect(s.rect));
                        assert!(clip.expand(0.1).contains_rect(s.rect));
                        *count += 1;
                    }
                }
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        contained_shapes(shape, clip, preview, count);
                    }
                }
                _ => {}
            }
        }
        let mut geometric_shapes = 0;
        for s in &output.shapes {
            contained_shapes(&s.shape, s.clip_rect, rect, &mut geometric_shapes);
        }
        assert!(
            geometric_shapes >= 5,
            "Missing two allocations/two legends/active data box"
        );
        for _ in 0..5 {
            let (next, _) = frame(&ctx, &mut editor, size, vec![]);
            assert_eq!(
                preview_rectangle(&next),
                (rect, clip),
                "Settled preview geometry changed"
            );
        }
        assert!(editor.draft.same_snapshot(&source));
        for ((_, before), (_, after)) in source.all_axes().zip(editor.draft.all_axes()) {
            for (a, b) in before.series.iter().zip(&after.series) {
                assert!((a.points).same_storage(&b.points));
            }
        }
        assert!(matches!(
            click(&ctx, &mut editor, size, point(&output, "Cancel", size)),
            Action::Cancel
        ));
        assert!(editor.original.same_snapshot(&source));
    }
}

#[test]
fn expanded_layout_scroll_keeps_preview_size_stable_and_footer_reachable() {
    let ctx = layout_context();
    let size = Vec2::new(960., 650.);
    let mut editor = outside_layout_editor();
    let source = editor.original.clone();
    for _ in 0..6 {
        frame(&ctx, &mut editor, size, vec![]);
    }
    let (output, _) = frame(&ctx, &mut editor, size, vec![]);
    click(
        &ctx,
        &mut editor,
        size,
        point(&output, "Requested OuterPosition", size),
    );
    editor.error = Some("Could not apply the figure change because C:/analysis/results/reports/long-experiment-name/figure.mfig was replaced by another process. Reload the saved figure or choose another filename, then retry. The current figure data has been preserved.".into());
    for _ in 0..6 {
        frame(&ctx, &mut editor, size, vec![]);
    }
    let (output, _) = frame(&ctx, &mut editor, size, vec![]);
    for label in ["Outer left", "Apply changes", "Cancel", "Restore original"] {
        point(&output, label, size);
    }
    let scroll = point(&output, "Outer left", size);
    let initial_size = preview_rectangle(&output).0.size();
    let initial_top = preview_rectangle(&output).0.top();
    for _ in 0..8 {
        frame(
            &ctx,
            &mut editor,
            size,
            vec![
                Event::PointerMoved(scroll),
                Event::MouseWheel {
                    phase: egui::TouchPhase::Move,
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0., -120.),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    for _ in 0..20 {
        frame(&ctx, &mut editor, size, vec![]);
    }
    let (output, _) = frame(&ctx, &mut editor, size, vec![]);
    let (rect, clip) = preview_rectangle(&output);
    assert_eq!(rect.size(), initial_size, "Scrolling resized the preview");
    assert!(
        rect.top() < initial_top - 10.,
        "Actual scroll did not move content"
    );
    assert!(clip.expand(0.1).contains_rect(rect));
    for label in [
        "Axes 1",
        "Axes 2",
        layout_caption(),
        "Apply changes",
        "Cancel",
        "Restore original",
    ] {
        point(&output, label, size);
    }
    for _ in 0..5 {
        let (next, _) = frame(&ctx, &mut editor, size, vec![]);
        assert_eq!(
            preview_rectangle(&next),
            (rect, clip),
            "Scrolled preview jittered"
        );
    }
    assert!(editor.draft.same_snapshot(&source));
    for ((_, before), (_, after)) in source.all_axes().zip(editor.draft.all_axes()) {
        for (a, b) in before.series.iter().zip(&after.series) {
            assert!((a.points).same_storage(&b.points));
        }
    }
    click(
        &ctx,
        &mut editor,
        size,
        point(&output, "Restore original", size),
    );
    assert!(editor.error.is_none());
    assert!(editor.draft.same_snapshot(&source));
}

// Exercise the actual scroll container: a compact viewport need not expose
// every axes property simultaneously, but its footer must remain reachable.
fn scroll_to_axes_control(ctx: &egui::Context, e: &mut Editor, size: Vec2, label: &str) -> Pos2 {
    let (initial, _) = frame(ctx, e, size, vec![]);
    let body_clip = initial
        .shapes
        .iter()
        .find_map(|s| find(&s.shape, "Shared figure title").map(|_| s.clip_rect))
        .expect("Initial axes body is missing");
    let anchor = body_clip.center();
    for step in 0..24 {
        let (output, _) = frame(ctx, e, size, vec![]);
        for footer in ["Apply changes", "Cancel", "Restore original"] {
            point(&output, footer, size);
        }
        if output.shapes.iter().any(|s| {
            find(&s.shape, label).is_some_and(|r| {
                s.clip_rect.expand(0.1).contains_rect(r)
                    && Rect::from_min_size(Pos2::ZERO, size).contains_rect(r)
            })
        }) {
            return point(&output, label, size);
        }
        assert!(step < 23, "Could not reach {label} by bounded scrolling");
        frame(
            ctx,
            e,
            size,
            vec![
                Event::PointerMoved(anchor),
                Event::MouseWheel {
                    phase: egui::TouchPhase::Move,
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0., -80.),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        for _ in 0..6 {
            frame(ctx, e, size, vec![]);
        }
    }
    unreachable!()
}

#[test]
fn independent_grid_editor_toggles_real_controls_and_preserves_original_samples() {
    for size in [Vec2::new(640., 480.), Vec2::new(1440., 900.)] {
        // Each successful Apply submits a worker and disables the dialog.
        // Test each independent edit in a fresh dialog, not by clicking a
        // disabled Y control after submitting the X edit.
        for label in ["X grid", "Y grid"] {
            let ctx = egui::Context::default();
            let mut e = editor();
            e.tab = Tab::Axes;
            let original = e.original.clone();
            assert!(original.xgrid && original.ygrid);
            for _ in 0..4 {
                frame(&ctx, &mut e, size, vec![]);
            }
            let target = scroll_to_axes_control(&ctx, &mut e, size, label);
            click(&ctx, &mut e, size, target);
            let expected = (label != "X grid", label != "Y grid");
            assert_eq!((e.draft.xgrid, e.draft.ygrid), expected);
            assert!(e.original.same_snapshot(&original));
            let (output, _) = frame(&ctx, &mut e, size, vec![]);
            let Action::Apply(change) =
                click(&ctx, &mut e, size, point(&output, "Apply changes", size))
            else {
                panic!("Grid change not applied");
            };
            assert!(e.saving && e.error.is_none());
            assert_eq!((change.updated.xgrid, change.updated.ygrid), expected);
            assert!(change.original.same_snapshot(&original));
            for ((_, old), (_, updated)) in original.all_axes().zip(change.updated.all_axes()) {
                for (old, updated) in old.series.iter().zip(&updated.series) {
                    assert!((old.points).same_storage(&updated.points));
                }
            }
            assert!(e.original.same_snapshot(&original));
        }
    }
}
