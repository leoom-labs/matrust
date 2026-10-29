use super::*;
use std::time::{Duration, Instant};

fn finish(app: &mut Workspace) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.busy && Instant::now() < deadline {
        app.receive();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!app.busy, "{}", app.status);
}
fn setup() -> (Workspace, egui::Context, tempfile::TempDir) {
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(false);
    let dir = tempfile::tempdir().unwrap();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.folder = dir.path().into();
    app.run("keep=1:12;alias=keep;figure(1);plot([1 2 4]);title('Tile one');figure(2);semilogx([1 10 100],[3 7 2]);title('Tile two');figure(3);subplot(1,2,1);plot([2 4]);subplot(1,2,2);plot([3 1]);figure(4);plot([9 2 6]);title('Tile four');figure(1);".into());
    finish(&mut app);
    app.show_plot_pane();
    app.layout.mode = crate::layout::PaneMode::Figure;
    (app, ctx, dir)
}
fn frame(
    app: &mut Workspace,
    ctx: &egui::Context,
    size: egui::Vec2,
    events: Vec<egui::Event>,
    discard: bool,
) -> egui::FullOutput {
    let modifiers = events
        .iter()
        .find_map(|e| match e {
            egui::Event::Key { modifiers, .. } => Some(*modifiers),
            _ => None,
        })
        .unwrap_or_default();
    let mut pass = 0;
    crate::headless::run(
        ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events: std::iter::once(egui::Event::ModifiersChanged(modifiers))
                .chain(events)
                .collect(),
            ..Default::default()
        },
        |root| {
            if discard && pass == 0 {
                root.ctx().request_discard("tile final pass admission");
            }
            pass += 1;
            app.ui(root);
        },
    )
}
fn settle(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2) {
    for _ in 0..16 {
        finish(app);
        frame(app, ctx, size, vec![], false);
    }
    finish(app);
}
fn texts(shape: &egui::epaint::Shape, label: &str, out: &mut Vec<(egui::Pos2, egui::Rect)>) {
    match shape {
        egui::epaint::Shape::Text(t) if t.galley.text() == label => {
            out.push((t.pos + t.galley.size() * 0.5, shape.visual_bounding_rect()))
        }
        egui::epaint::Shape::Vec(shapes) => {
            for shape in shapes {
                texts(shape, label, out);
            }
        }
        _ => {}
    }
}
fn label(
    app: &mut Workspace,
    ctx: &egui::Context,
    size: egui::Vec2,
    text: &str,
    rightmost: bool,
) -> egui::Pos2 {
    let mut found = Vec::new();
    for clipped in frame(app, ctx, size, vec![], false).shapes {
        let mut points = Vec::new();
        texts(&clipped.shape, text, &mut points);
        found.extend(
            points
                .into_iter()
                .filter(|(_, ink)| {
                    clipped.clip_rect.contains_rect(*ink) && ctx.content_rect().contains_rect(*ink)
                })
                .map(|(p, _)| p),
        );
    }
    found.sort_by(|a, b| a.x.total_cmp(&b.x));
    if rightmost {
        *found
            .last()
            .unwrap_or_else(|| panic!("Missing visible {text}"))
    } else {
        *found
            .first()
            .unwrap_or_else(|| panic!("Missing visible {text}"))
    }
}
fn visible_label(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2, text: &str) -> bool {
    let mut points = Vec::new();
    for clipped in frame(app, ctx, size, vec![], false).shapes {
        texts(&clipped.shape, text, &mut points);
    }
    points
        .iter()
        .any(|(_, ink)| ctx.content_rect().contains_rect(*ink))
}
fn click(
    app: &mut Workspace,
    ctx: &egui::Context,
    size: egui::Vec2,
    text: &str,
    right: bool,
    discard: bool,
) {
    // Single view keeps the mode and size choices in the figure card's
    // overflow menu; open it first.
    if matches!(
        text,
        "Single" | "2-up" | "4-up" | "Fit all figures" | "Readable size"
    ) && app.layout.figure_tiling.is_none()
        && !visible_label(app, ctx, size, text)
    {
        click(app, ctx, size, "Actions", true, false);
        for _ in 0..2 {
            frame(app, ctx, size, vec![], false);
        }
    }
    let p = label(app, ctx, size, text, right);
    for pressed in [true, false] {
        frame(
            app,
            ctx,
            size,
            vec![
                egui::Event::PointerMoved(p),
                egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            discard && !pressed,
        );
    }
}
fn key(app: &Workspace, id: u32) -> Key {
    Key {
        number: id,
        identity: app.all_figures[&id].identity.clone(),
    }
}
fn save_key() -> egui::Event {
    egui::Event::Key {
        key: egui::Key::S,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::CTRL,
    }
}

#[test]
fn visible_modes_and_reorder_keep_independent_plot_memory_and_shared_payloads() {
    for dark in [false, true] {
        let (mut app, ctx, _dir) = setup();
        app.layout.theme = Some(if dark {
            crate::session::DesktopTheme::Dark
        } else {
            crate::session::DesktopTheme::Light
        });
        app.apply_desktop_theme(&ctx);
        let size = egui::vec2(1600., 1000.);
        let original = app.all_figures.clone();
        let history = app.figure_history.clone();
        click(&mut app, &ctx, size, "4-up", false, true);
        settle(&mut app, &ctx, size);
        assert_eq!(app.layout.figure_tiling, Some(FigureTiling::Four));
        assert_eq!(ctx.global_style().visuals.dark_mode, dark);
        let keys: Vec<_> = (1..=4).map(|id| key(&app, id)).collect();
        let ids: Vec<_> = keys
            .iter()
            .map(|k| app.detached.docked.surface_id(k).unwrap())
            .collect();
        let first = *egui_plot::PlotMemory::load(&ctx, ids[0]).unwrap().bounds();
        let second = *egui_plot::PlotMemory::load(&ctx, ids[1]).unwrap().bounds();
        let point = label(&mut app, &ctx, size, "Tile two", false) + egui::vec2(0., 90.);
        frame(
            &mut app,
            &ctx,
            size,
            vec![egui::Event::PointerMoved(point), egui::Event::Zoom(2.)],
            false,
        );
        let zoomed = *egui_plot::PlotMemory::load(&ctx, ids[1]).unwrap().bounds();
        assert!(zoomed.width() < second.width());
        assert_eq!(
            *egui_plot::PlotMemory::load(&ctx, ids[0]).unwrap().bounds(),
            first
        );
        assert_eq!(app.current_figure, 1);
        assert_eq!(app.figure_history, history);
        click(&mut app, &ctx, size, "2-up", false, false);
        settle(&mut app, &ctx, size);
        let caption = presentation(&app, 2).unwrap().caption;
        click(&mut app, &ctx, size, &caption, true, false);
        click(&mut app, &ctx, size, "Figure 1", true, false);
        settle(&mut app, &ctx, size);
        assert!(app.detached.tiles.slots[0].as_ref().unwrap().same(&keys[1]));
        assert!(app.detached.tiles.slots[1].as_ref().unwrap().same(&keys[0]));
        click(&mut app, &ctx, size, "4-up", false, false);
        settle(&mut app, &ctx, size);
        for (k, id) in keys.iter().zip(&ids) {
            assert_eq!(app.detached.docked.surface_id(k), Some(*id));
        }
        assert_eq!(
            *egui_plot::PlotMemory::load(&ctx, ids[1]).unwrap().bounds(),
            zoomed
        );
        for (id, source) in original {
            assert!(app.all_figures[&id].same_snapshot(&source));
        }
        assert!(app.variables["keep"].same_snapshot(&app.variables["alias"]));
        assert!(
            root_cache_bytes(&app) + app.detached.retained_bytes() <= figure_geometry::CACHE_BYTES
        );
    }
}

#[test]
fn real_tile_actions_edit_save_undo_and_return_single_without_switching_during_paint() {
    let (mut app, ctx, dir) = setup();
    let size = egui::vec2(1600., 1000.);
    let path = dir.path().join("tile_two.mfig");
    app.submit(Job::SaveFigureById(2, path.clone(), None));
    finish(&mut app);
    let original = app.all_figures[&2].clone();
    let first = app.all_figures[&1].clone();
    click(&mut app, &ctx, size, "2-up", false, false);
    settle(&mut app, &ctx, size);
    click(&mut app, &ctx, size, "Actions", true, false);
    click(&mut app, &ctx, size, "Grid", false, true);
    settle(&mut app, &ctx, size);
    assert_eq!(app.current_figure, 2);
    assert_eq!(app.all_figures[&2].xgrid, !original.xgrid);
    click(&mut app, &ctx, size, "Save", true, false);
    settle(&mut app, &ctx, size);
    let saved = matrust::files::load_figure(&path).unwrap();
    assert_eq!(saved.xgrid, !original.xgrid);
    assert!(app.all_figures[&1].same_snapshot(&first));
    click(&mut app, &ctx, size, "Actions", true, false);
    click(&mut app, &ctx, size, "Undo edit", false, false);
    settle(&mut app, &ctx, size);
    assert!(app.all_figures[&2].same_snapshot(&original));
    click(&mut app, &ctx, size, "Single", false, false);
    settle(&mut app, &ctx, size);
    assert_eq!(app.layout.figure_tiling, None);
    assert_eq!(app.current_figure, 2);
    assert!(!app.plot_points.is_empty());
    assert!(
        original.series[0]
            .points
            .same_storage(&app.all_figures[&2].series[0].points)
    );
}

#[test]
fn simultaneous_pointer_save_discard_and_preview_return_keep_the_selected_tile() {
    let (mut app, ctx, _dir) = setup();
    let size = egui::vec2(1600., 1000.);
    click(&mut app, &ctx, size, "2-up", false, false);
    settle(&mut app, &ctx, size);
    app.document_save.intercepted = Some(Vec::new());
    let p = label(&mut app, &ctx, size, "Tile two", false);
    frame(
        &mut app,
        &ctx,
        size,
        vec![
            egui::Event::PointerMoved(p),
            egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::CTRL,
            },
            save_key(),
        ],
        true,
    );
    assert_eq!(
        app.document_save.intercepted.as_deref(),
        Some(
            &[crate::document_save::Request::Figure {
                figure: 2,
                save_as: false
            }][..]
        )
    );
    finish(&mut app);
    settle(&mut app, &ctx, size);
    app.document_save
        .activate(crate::document_save::Pane::Preview);
    app.document_save.preview_closed();
    frame(&mut app, &ctx, size, vec![save_key()], false);
    assert_eq!(app.document_save.intercepted.as_ref().unwrap().len(), 2);
    assert_eq!(
        app.document_save.intercepted.as_ref().unwrap()[1],
        crate::document_save::Request::Figure {
            figure: 2,
            save_as: false
        }
    );
}

#[test]
fn captured_revision_and_reused_number_reject_save_without_legacy_fallback() {
    let (mut app, ctx, _dir) = setup();
    let size = egui::vec2(1600., 1000.);
    click(&mut app, &ctx, size, "2-up", false, false);
    settle(&mut app, &ctx, size);
    let old = Authority {
        key: key(&app, 2),
        revision: revision(&app, 2).unwrap(),
    };
    app.document_save.activate_tile(old.clone());
    app.run("figure(2);title('new revision');figure(1);".into());
    finish(&mut app);
    app.document_save.intercepted = Some(Vec::new());
    crate::headless::run(&ctx, egui::RawInput::default(), |root| {
        app.document_save.queue(
            root.ctx().cumulative_frame_nr(),
            false,
            app.generic_save_target(),
        );
        app.finish_document_action(root.ctx());
    });
    assert!(app.document_save.intercepted.as_ref().unwrap().is_empty());
    assert!(app.status.contains("tile changed"));
    app.run("close(2);figure(2);plot([8 3]);figure(1);".into());
    finish(&mut app);
    assert!(!old.key.same(&key(&app, 2)));
    assert!(!app.dispatch_tile(&ctx, &old, Action::Close));
    assert!(app.all_figures.contains_key(&2));
}

#[test]
fn compact_controls_are_reachable_and_mode_change_is_deferred_until_final_pass() {
    for dark in [false, true] {
        let (mut app, ctx, _dir) = setup();
        app.layout.theme = Some(if dark {
            crate::session::DesktopTheme::Dark
        } else {
            crate::session::DesktopTheme::Light
        });
        app.apply_desktop_theme(&ctx);
        let size = egui::vec2(960., 650.);
        let original = app.all_figures.clone();
        click(&mut app, &ctx, size, "2-up", false, true);
        settle(&mut app, &ctx, size);
        assert_eq!(app.layout.figure_tiling, Some(FigureTiling::Two));
        for name in ["Single", "2-up", "4-up", "Tile one", "Tile two"] {
            let _ = label(&mut app, &ctx, size, name, false);
        }
        click(&mut app, &ctx, size, "4-up", false, true);
        settle(&mut app, &ctx, size);
        assert_eq!(app.layout.figure_tiling, Some(FigureTiling::Four));
        assert_eq!(ctx.global_style().visuals.dark_mode, dark);
        for (id, figure) in original {
            assert!(app.all_figures[&id].same_snapshot(&figure));
        }
    }
}

#[test]
fn discarded_pass_cannot_redirect_a_captured_tile_save_to_the_editor() {
    let (mut app, ctx, _dir) = setup();
    let size = egui::vec2(1600., 1000.);
    click(&mut app, &ctx, size, "2-up", false, false);
    settle(&mut app, &ctx, size);
    app.document_save.activate_tile(Authority {
        key: key(&app, 2),
        revision: revision(&app, 2).unwrap(),
    });
    app.document_save.intercepted = Some(Vec::new());
    let document = app.documents[app.active].clone();
    let mut passes = 0;
    crate::headless::run(
        &ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events: vec![
                egui::Event::ModifiersChanged(egui::Modifiers::CTRL),
                save_key(),
            ],
            ..Default::default()
        },
        |root| {
            if passes == 0 {
                root.ctx()
                    .request_discard("change document after capturing save");
            }
            app.ui(root);
            if passes == 0 {
                app.reveal_editor();
                app.layout.mode = crate::layout::PaneMode::Editor;
            }
            passes += 1;
        },
    );
    assert!(passes >= 2);
    assert!(app.document_save.intercepted.as_ref().unwrap().is_empty());
    assert!(app.status.contains("active document changed"));
    assert_eq!(app.documents[app.active].id, document.id);
    assert_eq!(app.documents[app.active].text, document.text);
    assert_eq!(app.documents[app.active].dirty, document.dirty);
}

fn scrollbar_drag(
    app: &mut Workspace,
    ctx: &egui::Context,
    size: egui::Vec2,
    start: egui::Pos2,
    end: egui::Pos2,
) {
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frame(
        app,
        ctx,
        size,
        vec![egui::Event::PointerMoved(start)],
        false,
    );
    frame(app, ctx, size, vec![button(start, true)], false);
    frame(app, ctx, size, vec![egui::Event::PointerMoved(end)], false);
    frame(app, ctx, size, vec![button(end, false)], false);
}
fn tile_scroll_observation(ctx: &egui::Context) -> (egui::Rect, egui::Vec2, egui::Vec2) {
    ctx.data(|data| {
        data.get_temp(egui::Id::new("figure_tile_scroll_observation"))
            .expect("The actual four-figure ScrollArea must have painted")
    })
}

#[test]
fn compact_four_up_readable_scrollbar_reaches_lower_plot_and_preserves_action_ownership() {
    for dark in [false, true] {
        let (mut app, ctx, dir) = setup();
        app.layout.theme = Some(if dark {
            crate::session::DesktopTheme::Dark
        } else {
            crate::session::DesktopTheme::Light
        });
        app.apply_desktop_theme(&ctx);
        let size = egui::vec2(960., 650.);
        let path = dir.path().join("lower-right.mfig");
        app.submit(Job::SaveFigureById(4, path.clone(), None));
        finish(&mut app);
        click(&mut app, &ctx, size, "4-up", false, true);
        settle(&mut app, &ctx, size);
        click(&mut app, &ctx, size, "Readable size", false, true);
        settle(&mut app, &ctx, size);
        assert_eq!(app.layout.figure_tile_size, Some(FigureTileSize::Readable));
        let originals = app.all_figures.clone();
        let history = app.figure_history.clone();
        let ids: Vec<_> = [1, 2, 4]
            .into_iter()
            .map(|number| app.detached.docked.surface_id(&key(&app, number)).unwrap())
            .collect();
        let bounds: Vec<_> = ids
            .iter()
            .map(|id| *egui_plot::PlotMemory::load(&ctx, *id).unwrap().bounds())
            .collect();
        let (viewport, content, before) = tile_scroll_observation(&ctx);
        assert!(
            content.y > viewport.height() + 1.,
            "Must exercise real overflow"
        );
        assert_eq!(before.y, 0.);
        let scroll = ctx.global_style().spacing.scroll;
        let x = if scroll.floating {
            viewport.right() - scroll.bar_width * 0.5
        } else {
            viewport.right() + scroll.bar_inner_margin + scroll.bar_width * 0.5
        };
        let handle = (viewport.height().powi(2) / content.y)
            .max(scroll.handle_min_length)
            .min(viewport.height());
        let start = egui::pos2(x, viewport.top() + handle * 0.5);
        let end = egui::pos2(x, viewport.bottom() - 2.);
        // Only real pointer events move the actual scrollbar. Never inject or
        // overwrite ScrollArea state; wheel gestures belong to each plot.
        scrollbar_drag(&mut app, &ctx, size, start, end);
        settle(&mut app, &ctx, size);
        let (viewport, content, after) = tile_scroll_observation(&ctx);
        assert!(after.y > before.y + 1.);
        assert!(after.y >= content.y - viewport.height() - 1.);
        // The figure card is the right column, narrower than two readable
        // tiles: the lower-right plot also needs the horizontal scrollbar.
        assert!(
            content.x > viewport.width() + 1.,
            "Must exercise real overflow"
        );
        assert_eq!(after.x, 0.);
        let y = if scroll.floating {
            viewport.bottom() - scroll.bar_width * 0.5
        } else {
            viewport.bottom() + scroll.bar_inner_margin + scroll.bar_width * 0.5
        };
        let handle = (viewport.width().powi(2) / content.x)
            .max(scroll.handle_min_length)
            .min(viewport.width());
        let start = egui::pos2(viewport.left() + handle * 0.5, y);
        let end = egui::pos2(viewport.right() - 2., y);
        scrollbar_drag(&mut app, &ctx, size, start, end);
        settle(&mut app, &ctx, size);
        let (viewport, content, after) = tile_scroll_observation(&ctx);
        assert!(after.x >= content.x - viewport.width() - 1.);
        assert!(after.y >= content.y - viewport.height() - 1.);
        let title = label(&mut app, &ctx, size, "Tile four", false);
        let point = title + egui::vec2(0., 90.);
        assert!(
            viewport.contains(point),
            "Lower-right plot must be reachable"
        );
        frame(
            &mut app,
            &ctx,
            size,
            vec![egui::Event::PointerMoved(point), egui::Event::Zoom(2.)],
            false,
        );
        let lower = *egui_plot::PlotMemory::load(&ctx, ids[2]).unwrap().bounds();
        // Grid Undo restores the displayed pre-edit manual view, not the
        // original pre-zoom automatic limits. Figure4 is linear on both axes.
        assert_eq!(app.figure_views[&4].auto, [false; 2]);
        let mut expected_undo = originals[&4].clone();
        expected_undo.xlim = Some([lower.min()[0], lower.max()[0]]);
        expected_undo.ylim = Some([lower.min()[1], lower.max()[1]]);
        assert!(lower.width() < bounds[2].width());
        for index in [0, 1] {
            assert_eq!(
                *egui_plot::PlotMemory::load(&ctx, ids[index])
                    .unwrap()
                    .bounds(),
                bounds[index]
            );
        }
        assert_eq!(app.figure_history, history);
        for (number, original) in &originals {
            assert!(app.all_figures[number].same_snapshot(original));
        }
        click(&mut app, &ctx, size, "Actions", true, false);
        click(&mut app, &ctx, size, "Grid", false, true);
        settle(&mut app, &ctx, size);
        assert_eq!(app.current_figure, 4);
        assert_eq!(app.all_figures[&4].xgrid, !originals[&4].xgrid);
        click(&mut app, &ctx, size, "Save", true, false);
        settle(&mut app, &ctx, size);
        let saved = matrust::files::load_figure(&path).unwrap();
        assert_eq!(saved.xgrid, !originals[&4].xgrid);
        assert_eq!(saved.xlim, expected_undo.xlim);
        assert_eq!(saved.ylim, expected_undo.ylim);
        assert!(
            saved.series[0]
                .points
                .iter()
                .map(|point| point.map(f64::to_bits))
                .eq(originals[&4].series[0]
                    .points
                    .iter()
                    .map(|point| point.map(f64::to_bits)))
        );
        click(&mut app, &ctx, size, "Actions", true, false);
        click(&mut app, &ctx, size, "Undo edit", false, false);
        settle(&mut app, &ctx, size);
        for (number, original) in &originals {
            let expected = if *number == 4 {
                &expected_undo
            } else {
                original
            };
            assert!(
                app.all_figures[number].same_snapshot(expected),
                "Figure {number}: Undo must restore all pre-edit metadata, appearance and shared samples"
            );
        }
        for (index, number) in [1, 2, 4].into_iter().enumerate() {
            assert_eq!(
                app.detached.docked.surface_id(&key(&app, number)),
                Some(ids[index])
            );
        }
        assert_eq!(
            *egui_plot::PlotMemory::load(&ctx, ids[2]).unwrap().bounds(),
            lower
        );
        assert!(app.variables["keep"].same_snapshot(&app.variables["alias"]));
        assert!(
            root_cache_bytes(&app) + app.detached.retained_bytes() <= figure_geometry::CACHE_BYTES
        );
    }
}

fn all_visible_ink(output: &egui::FullOutput, screen: egui::Rect, label: &str) -> Vec<egui::Pos2> {
    fn visit(
        shape: &egui::Shape,
        clip: egui::Rect,
        screen: egui::Rect,
        label: &str,
        found: &mut Vec<egui::Pos2>,
    ) {
        match shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                let ink = text.visual_bounding_rect();
                assert!(!text.galley.elided, "{label} must not be elided");
                assert!(
                    clip.contains_rect(ink) && screen.contains_rect(ink),
                    "{label}: actual ink {ink:?} outside clip {clip:?} / viewport {screen:?}"
                );
                found.push(ink.center());
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    visit(shape, clip, screen, label, found);
                }
            }
            _ => {}
        }
    }
    let mut found = Vec::new();
    for shape in &output.shapes {
        visit(&shape.shape, shape.clip_rect, screen, label, &mut found);
    }
    found
}
/// Visible `label` controls of the tiles: the toolstrip has its own Save.
fn tile_controls(output: &egui::FullOutput, ctx: &egui::Context, label: &str) -> Vec<egui::Pos2> {
    let (viewport, _, _) = tile_scroll_observation(ctx);
    all_visible_ink(output, ctx.content_rect(), label)
        .into_iter()
        .filter(|ink| viewport.contains(*ink))
        .collect()
}
fn pointer_click(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2, point: egui::Pos2) {
    for pressed in [true, false] {
        frame(
            app,
            ctx,
            size,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            !pressed,
        );
    }
}
fn lower_right_control(
    app: &mut Workspace,
    ctx: &egui::Context,
    size: egui::Vec2,
    name: &str,
) -> egui::Pos2 {
    let output = frame(app, ctx, size, vec![], false);
    let mut found = tile_controls(&output, ctx, name);
    assert_eq!(
        found.len(),
        4,
        "Every tile must expose its {name} control without scrolling"
    );
    found.sort_by(|a, b| a.y.total_cmp(&b.y).then_with(|| a.x.total_cmp(&b.x)));
    found[3]
}

#[test]
fn default_fit_shows_all_four_artworks_and_lower_controls_in_both_native_sizes_and_themes() {
    for dark in [false, true] {
        for size in [egui::vec2(1440., 900.), egui::vec2(960., 650.)] {
            let (mut app, ctx, dir) = setup();
            app.layout.theme = Some(if dark {
                crate::session::DesktopTheme::Dark
            } else {
                crate::session::DesktopTheme::Light
            });
            app.apply_desktop_theme(&ctx);
            // Four ordinary artworks reproduce the actual capture failure. The
            // existing setup/subplot test separately exercises fitted nested axes.
            app.run("figure(3);clf;plot([4 1 5]);title('Tile three');figure(1);xlabel('X one');ylabel('Y one');figure(2);xlabel('X two');ylabel('Y two');figure(3);xlabel('X three');ylabel('Y three');figure(4);xlabel('X four');ylabel('Y four');figure(1);".into());
            finish(&mut app);
            let path = dir.path().join("fit-lower-right.mfig");
            app.submit(Job::SaveFigureById(4, path.clone(), None));
            finish(&mut app);
            let original = app.all_figures.clone();
            let history = app.figure_history.clone();
            click(&mut app, &ctx, size, "4-up", false, true);
            settle(&mut app, &ctx, size);
            assert_eq!(
                app.layout.figure_tile_size, None,
                "Default must fit without an opt-in"
            );
            assert_eq!(ctx.global_style().visuals.dark_mode, dark);
            let output = frame(&mut app, &ctx, size, vec![], false);
            for label in [
                "Fit all figures",
                "Readable size",
                "Tile one",
                "Tile two",
                "Tile three",
                "Tile four",
                "X one",
                "Y one",
                "X two",
                "Y two",
                "X three",
                "Y three",
                "X four",
                "Y four",
            ] {
                assert_eq!(
                    all_visible_ink(&output, ctx.content_rect(), label).len(),
                    1,
                    "Missing {label}"
                );
            }
            let (viewport, content, offset) = tile_scroll_observation(&ctx);
            for label in ["Save", "Actions"] {
                assert_eq!(tile_controls(&output, &ctx, label).len(), 4);
            }
            assert!(content.x <= viewport.width() + 0.01 && content.y <= viewport.height() + 0.01);
            assert_eq!(offset, egui::Vec2::ZERO);
            let ids: Vec<_> = (1..=4)
                .map(|n| app.detached.docked.surface_id(&key(&app, n)).unwrap())
                .collect();
            let bounds: Vec<_> = ids
                .iter()
                .map(|id| *egui_plot::PlotMemory::load(&ctx, *id).unwrap().bounds())
                .collect();
            for id in &ids {
                let memory = egui_plot::PlotMemory::load(&ctx, *id).unwrap();
                let plot_frame = *memory.transform().frame();
                assert!(
                    viewport.contains_rect(plot_frame)
                        && plot_frame.width() > 32.
                        && plot_frame.height() > 16.,
                    "Actual data frame must fit and remain usable: {plot_frame:?} in {viewport:?}"
                );
            }
            let point = egui_plot::PlotMemory::load(&ctx, ids[3])
                .unwrap()
                .transform()
                .frame()
                .center();
            frame(
                &mut app,
                &ctx,
                size,
                vec![egui::Event::PointerMoved(point), egui::Event::Zoom(2.)],
                false,
            );
            let zoomed = *egui_plot::PlotMemory::load(&ctx, ids[3]).unwrap().bounds();
            // Expected Undo state independently follows the actual displayed
            // linear-axis zoom, retaining every other original field and Arc.
            assert_eq!(app.figure_views[&4].auto, [false; 2]);
            let mut expected_undo = original[&4].clone();
            expected_undo.xlim = Some([zoomed.min()[0], zoomed.max()[0]]);
            expected_undo.ylim = Some([zoomed.min()[1], zoomed.max()[1]]);
            assert!(zoomed.width() < bounds[3].width());
            for i in 0..3 {
                assert_eq!(
                    *egui_plot::PlotMemory::load(&ctx, ids[i]).unwrap().bounds(),
                    bounds[i]
                );
            }
            assert_eq!(app.figure_history, history);
            for mode in ["Readable size", "Fit all figures", "2-up", "4-up"] {
                click(&mut app, &ctx, size, mode, false, true);
                settle(&mut app, &ctx, size);
            }
            assert_eq!(app.layout.figure_tile_size, None);
            for (i, n) in (1..=4).enumerate() {
                assert_eq!(app.detached.docked.surface_id(&key(&app, n)), Some(ids[i]));
            }
            assert_eq!(
                *egui_plot::PlotMemory::load(&ctx, ids[3]).unwrap().bounds(),
                zoomed
            );
            for (n, f) in &original {
                assert!(app.all_figures[n].same_snapshot(f));
            }
            let control = lower_right_control(&mut app, &ctx, size, "Actions");
            pointer_click(&mut app, &ctx, size, control);
            click(&mut app, &ctx, size, "Grid", false, true);
            settle(&mut app, &ctx, size);
            assert_eq!(app.current_figure, 4);
            assert_eq!(app.all_figures[&4].xgrid, !original[&4].xgrid);
            let control = lower_right_control(&mut app, &ctx, size, "Save");
            pointer_click(&mut app, &ctx, size, control);
            settle(&mut app, &ctx, size);
            let saved = matrust::files::load_figure(&path).unwrap();
            assert_eq!(saved.xgrid, !original[&4].xgrid);
            assert_eq!(saved.xlim, expected_undo.xlim);
            assert_eq!(saved.ylim, expected_undo.ylim);
            let control = lower_right_control(&mut app, &ctx, size, "Actions");
            pointer_click(&mut app, &ctx, size, control);
            click(&mut app, &ctx, size, "Undo edit", false, true);
            settle(&mut app, &ctx, size);
            for (n, f) in &original {
                let expected = if *n == 4 { &expected_undo } else { f };
                assert!(
                    app.all_figures[n].same_snapshot(expected),
                    "Figure {n}: Undo must restore all pre-edit metadata, appearance and shared samples"
                );
                assert!(
                    app.all_figures[n].series[0]
                        .points
                        .same_storage(&f.series[0].points)
                );
            }
            for (index, number) in (1..=4).enumerate() {
                assert_eq!(
                    app.detached.docked.surface_id(&key(&app, number)),
                    Some(ids[index])
                );
            }
            assert_eq!(
                *egui_plot::PlotMemory::load(&ctx, ids[3]).unwrap().bounds(),
                zoomed
            );
            assert!(app.variables["keep"].same_snapshot(&app.variables["alias"]));
            assert!(
                root_cache_bytes(&app) + app.detached.retained_bytes()
                    <= figure_geometry::CACHE_BYTES
            );
            click(&mut app, &ctx, size, "Single", false, true);
            settle(&mut app, &ctx, size);
            let output = frame(&mut app, &ctx, size, vec![], false);
            assert!(
                all_visible_ink(&output, ctx.content_rect(), "Fit all figures").is_empty(),
                "Single adds no sizing row"
            );
        }
    }
}

#[test]
fn fit_nested_axes_uses_each_existing_prepared_identity_without_inner_scroll_controls() {
    let (mut app, ctx, _dir) = setup();
    let size = egui::vec2(960., 650.);
    click(&mut app, &ctx, size, "4-up", false, true);
    settle(&mut app, &ctx, size);
    let source = app.all_figures[&3].clone();
    let surface = app.detached.docked.surface_id(&key(&app, 3)).unwrap();
    let output = frame(&mut app, &ctx, size, vec![], false);
    for name in ["Axes 1", "Axes 2"] {
        assert_eq!(all_visible_ink(&output, ctx.content_rect(), name).len(), 1);
    }
    // Only the outer user preference remains; a fitting tile cannot grow again
    // behind an independent subplot's historical readable-size minimum.
    assert!(all_visible_ink(&output, ctx.content_rect(), "Fit all axes").is_empty());
    assert_eq!(
        all_visible_ink(&output, ctx.content_rect(), "Readable size").len(),
        1
    );
    let (viewport, _, _) = tile_scroll_observation(&ctx);
    let ids: Vec<_> = source
        .all_axes()
        .map(|(position, _)| surface.with(("axes", position)))
        .collect();
    for id in &ids {
        let memory = egui_plot::PlotMemory::load(&ctx, *id).unwrap();
        assert!(viewport.contains_rect(*memory.transform().frame()));
    }
    click(&mut app, &ctx, size, "Readable size", false, true);
    settle(&mut app, &ctx, size);
    click(&mut app, &ctx, size, "Fit all figures", false, true);
    settle(&mut app, &ctx, size);
    assert_eq!(app.detached.docked.surface_id(&key(&app, 3)), Some(surface));
    for id in ids {
        assert!(egui_plot::PlotMemory::load(&ctx, id).is_some());
    }
    assert!(app.all_figures[&3].same_snapshot(&source));
}
