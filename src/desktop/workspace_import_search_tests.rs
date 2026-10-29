use super::*;
use egui::{Event, Pos2, Rect, Vec2};
use std::sync::atomic::AtomicBool;

fn fixture(count: usize) -> (tempfile::TempDir, MatCatalog) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("variables.mat");
    let values = (0..count)
        .map(|i| (format!("signal_{i:05}"), matrust::Matrix::scalar(i as f64)))
        .collect();
    matrust::files::save_workspace(&path, &values).unwrap();
    let catalog = matrust::files::inspect_workspace(&path, &AtomicBool::new(false)).unwrap();
    (dir, catalog)
}
fn frame(
    ctx: &egui::Context,
    app: &mut Workspace,
    size: Vec2,
    events: Vec<Event>,
) -> egui::FullOutput {
    crate::headless::run(
        ctx,
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ctx| app.workspace_import_ui(ctx),
    )
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
fn click(ctx: &egui::Context, app: &mut Workspace, size: Vec2, pos: Pos2) {
    for pressed in [true, false] {
        frame(
            ctx,
            app,
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
        );
    }
}
fn finish(app: &mut Workspace) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while app.busy && Instant::now() < deadline {
        app.receive();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!app.busy, "{}", app.status);
}

#[test]
fn mat_search_catalog_index_borrows_and_filter_preserves_hidden_selection() {
    let (_dir, catalog) = fixture(10000);
    for index in [0, 999, 9999] {
        assert!(std::ptr::eq(
            catalog.variable(index).unwrap(),
            catalog.variables().nth(index).unwrap()
        ));
    }
    assert!(catalog.variable(10000).is_none() && catalog.variable(usize::MAX).is_none());
    let mut import = Import::new(catalog);
    import.selected.insert("signal_00000".into());
    import.query = " SIGNAL_0999 ".into();
    assert!(import.update_filter());
    assert_eq!(import.visible.len(), 10);
    let address = import.visible.as_ptr();
    assert!(!import.update_filter());
    assert_eq!(address, import.visible.as_ptr());
    import.select_matching();
    assert_eq!(import.selected.len(), 11);
    assert!(import.selected.contains("signal_00000"));
    import.query = "double".into();
    import.update_filter();
    assert_eq!(import.visible.len(), 10000);
    import.query = "no matching data".into();
    import.update_filter();
    assert!(import.visible.is_empty());
    import.select_matching();
    assert_eq!(import.selected.len(), 11);
}

#[test]
fn mat_search_virtual_rows_and_actions_fit_with_long_errors() {
    let (_dir, catalog) = fixture(10000);
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.workspace_import = Some(Import::new(catalog));
    app.workspace_import.as_mut().unwrap().error = Some("A long import diagnostic. ".repeat(200));
    for size in [
        Vec2::new(1440., 900.),
        Vec2::new(960., 650.),
        Vec2::new(640., 480.),
        Vec2::new(480., 320.),
    ] {
        for _ in 0..4 {
            frame(&ctx, &mut app, size, vec![]);
        }
        let output = frame(&ctx, &mut app, size, vec![]);
        for label in [
            "Import MAT workspace",
            "Load selected",
            "Refresh file",
            "Cancel",
            "Select supported",
            "Clear selection",
        ] {
            point(&output, label, size);
        }
        let rect = ctx
            .memory(|m| m.area_rect(egui::Id::new("workspace_import")))
            .unwrap();
        assert!(
            Rect::from_min_size(Pos2::ZERO, size).contains_rect(rect),
            "{rect:?} in {size:?}"
        );
        assert!(app.workspace_import.as_ref().unwrap().rows_drawn < 20);
    }
    let size = Vec2::new(480., 320.);
    let import = app.workspace_import.as_mut().unwrap();
    import.selected.insert("signal_00000".into());
    import.query = "signal_00001".into();
    for _ in 0..4 {
        frame(&ctx, &mut app, size, vec![]);
    }
    let output = frame(&ctx, &mut app, size, vec![]);
    point(&output, "1 selected variables hidden by filter", size);
    let rect = ctx
        .memory(|m| m.area_rect(egui::Id::new("workspace_import")))
        .unwrap();
    assert!(
        Rect::from_min_size(Pos2::ZERO, size).contains_rect(rect),
        "{rect:?}"
    );
    click(&ctx, &mut app, size, point(&output, "Cancel", size));
    assert!(app.workspace_import.is_none() && app.variables.is_empty());
}

#[test]
fn mat_search_real_typed_filter_loads_selected_only_and_preserves_workspace() {
    let (_dir, catalog) = fixture(30);
    let ctx = egui::Context::default();
    let size = Vec2::new(640., 480.);
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.run("keep=[3 4 5];".into());
    finish(&mut app);
    let keep = app.variables["keep"].clone();
    app.workspace_import = Some(Import::new(catalog));
    for _ in 0..4 {
        frame(&ctx, &mut app, size, vec![]);
    }
    let output = frame(&ctx, &mut app, size, vec![]);
    click(
        &ctx,
        &mut app,
        size,
        point(&output, "Find variables by name or type", size),
    );
    frame(
        &ctx,
        &mut app,
        size,
        vec![Event::Text("signal_00007".into())],
    );
    assert_eq!(app.workspace_import.as_ref().unwrap().visible.len(), 1);
    let output = frame(&ctx, &mut app, size, vec![]);
    click(&ctx, &mut app, size, point(&output, "Select matches", size));
    assert_eq!(app.workspace_import.as_ref().unwrap().selected.len(), 1);
    let output = frame(&ctx, &mut app, size, vec![]);
    click(&ctx, &mut app, size, point(&output, "Load selected", size));
    finish(&mut app);
    assert!(app.workspace_import.is_none());
    assert_eq!(app.variables.len(), 2);
    assert_eq!(
        app.variables["signal_00007"].as_numeric().unwrap().data(),
        &[7.]
    );
    assert!(keep.same_snapshot(&app.variables["keep"]));
}

#[test]
fn mat_search_unsupported_matches_are_not_selected_and_busy_escape_stops() {
    let catalog = matrust::files::inspect_workspace(
        std::path::Path::new("tests/fixtures/mat/mat-import-v7.mat"),
        &AtomicBool::new(false),
    )
    .unwrap();
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    let mut import = Import::new(catalog);
    import.query = "three_d".into();
    import.update_filter();
    import.select_matching();
    assert_eq!(import.visible.len(), 1);
    assert!(import.selected.is_empty());
    import.selected.insert("compact".into());
    app.workspace_import = Some(import);
    app.busy = true;
    let size = Vec2::new(480., 320.);
    for _ in 0..4 {
        frame(&ctx, &mut app, size, vec![]);
    }
    let output = frame(&ctx, &mut app, size, vec![]);
    point(&output, "1 selected variables hidden by filter", size);
    point(&output, "Stop", size);
    frame(
        &ctx,
        &mut app,
        size,
        vec![Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert!(app.worker.cancel.load(Ordering::Relaxed));
    assert!(app.workspace_import.is_some());
    app.busy = false;
}

#[test]
fn mat_search_compact_table_scrolls_horizontally_to_status_column() {
    let (_dir, catalog) = fixture(30);
    let ctx = egui::Context::default();
    let size = Vec2::new(640., 480.);
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.workspace_import = Some(Import::new(catalog));
    for _ in 0..4 {
        frame(&ctx, &mut app, size, vec![]);
    }
    let output = frame(&ctx, &mut app, size, vec![]);
    let pos = point(&output, "Name", size);
    for _ in 0..5 {
        frame(
            &ctx,
            &mut app,
            size,
            vec![
                Event::PointerMoved(pos),
                Event::MouseWheel {
                    phase: egui::TouchPhase::Move,
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(-200., 0.),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    for _ in 0..20 {
        frame(&ctx, &mut app, size, vec![]);
    }
    let output = frame(&ctx, &mut app, size, vec![]);
    point(&output, "Status", size);
    point(&output, "Load selected", size);
    assert!(app.workspace_import.as_ref().unwrap().selected.is_empty() && app.variables.is_empty());
}

#[test]
#[ignore = "manual release CPU measurement; metadata is prepared outside timing"]
fn mat_search_benchmark_cpu_frames() {
    let workers = rayon::current_num_threads();
    println!();
    for count in [100, 1000, 10000] {
        let (_dir, catalog) = fixture(count);
        for mode in ["all", "filtered"] {
            let ctx = egui::Context::default();
            let mut app = Workspace::with_context(ctx.clone(), true);
            let mut import = Import::new(catalog.clone());
            if mode == "filtered" {
                import.query = "signal_0000".into();
            }
            app.workspace_import = Some(import);
            let size = Vec2::new(960., 650.);
            for repetition in 0..12 {
                let start = Instant::now();
                for _ in 0..100 {
                    std::hint::black_box(frame(&ctx, &mut app, size, vec![]));
                }
                let ms = start.elapsed().as_secs_f64() * 10.;
                let import = app.workspace_import.as_ref().unwrap();
                assert!(import.rows_drawn < 20 && !app.busy && app.variables.is_empty());
                if repetition > 0 {
                    println!(
                        "{workers},{count},{mode},100,{repetition},{ms:.6},{},{}",
                        import.visible.len(),
                        import.rows_drawn
                    );
                }
            }
        }
    }
}

#[test]
fn mat_search_selects_and_loads_complex_double_without_replacing_unselected_alias() {
    let catalog = matrust::files::inspect_workspace(
        std::path::Path::new("tests/fixtures/mat/mat-import-v7.mat"),
        &AtomicBool::new(false),
    )
    .unwrap();
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.run("keep=7;".into());
    finish(&mut app);
    let keep = app.variables["keep"].clone();
    let mut import = Import::new(catalog);
    import.query = "complex double".into();
    import.update_filter();
    import.select_matching();
    assert_eq!(import.selected.len(), 1);
    assert!(import.selected.contains("complex_data"));
    app.workspace_import = Some(import);
    let size = Vec2::new(960., 650.);
    for _ in 0..4 {
        frame(&ctx, &mut app, size, vec![]);
    }
    let output = frame(&ctx, &mut app, size, vec![]);
    click(&ctx, &mut app, size, point(&output, "Load selected", size));
    finish(&mut app);
    assert!(app.workspace_import.is_none());
    let loaded = app.variables["complex_data"].as_complex().unwrap();
    assert_eq!(
        loaded.data(),
        &[
            matrust::Complex64::new(1., 3.),
            matrust::Complex64::new(2., 4.)
        ]
    );
    assert!(app.variables["keep"].same_snapshot(&keep));
}

#[test]
fn mat_import_complex_type_is_not_elided_and_controls_remain_visible() {
    use matrust::{Array, Complex64, ComplexMatrix, LogicalMatrix, Matrix};
    let cancel = AtomicBool::new(false);
    let values: std::collections::BTreeMap<String, Array> = [
        (
            "Empty".into(),
            ComplexMatrix::filled(0, 3, Complex64::new(0., 0.), &cancel)
                .unwrap()
                .into(),
        ),
        (
            "ZeroImag".into(),
            ComplexMatrix::filled(1, 3, Complex64::new(2., 0.), &cancel)
                .unwrap()
                .into(),
        ),
        (
            "Z".into(),
            ComplexMatrix::filled(2, 3, Complex64::new(1., 2.), &cancel)
                .unwrap()
                .into(),
        ),
        ("Real".into(), Matrix::scalar(3.).into()),
        ("Logical".into(), LogicalMatrix::scalar(true).into()),
    ]
    .into();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("type_labels.mat");
    matrust::files::save_array_workspace(&path, &values, &cancel).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let catalog = matrust::files::inspect_workspace(&path, &cancel).unwrap();

    fn inspect(shape: &egui::Shape, clip: Rect, screen: Rect, count: &mut usize) {
        match shape {
            egui::Shape::Text(text) if text.galley.text() == "complex double" => {
                // Galley text retains its original string even when ellipsized.
                // Check the actual layout flag and painted geometry as well.
                assert!(!text.galley.elided, "Complex type label was elided");
                let rect = Rect::from_min_size(text.pos, text.galley.size());
                assert!(
                    clip.expand(0.1).contains_rect(rect),
                    "Clipped type {rect:?} in {clip:?}"
                );
                assert!(screen.contains_rect(rect));
                *count += 1;
            }
            egui::Shape::Vec(shapes) => {
                for child in shapes {
                    inspect(child, clip, screen, count);
                }
            }
            _ => {}
        }
    }

    for size in [Vec2::new(1440., 900.), Vec2::new(960., 650.)] {
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        let keep: Array = Matrix::scalar(11.).into();
        app.variables.insert("keep".into(), keep.clone());
        app.workspace_import = Some(Import::new(catalog.clone()));
        for _ in 0..4 {
            frame(&ctx, &mut app, size, vec![]);
        }
        let output = frame(&ctx, &mut app, size, vec![]);
        let mut complex_labels = 0;
        for shape in &output.shapes {
            inspect(
                &shape.shape,
                shape.clip_rect,
                Rect::from_min_size(Pos2::ZERO, size),
                &mut complex_labels,
            );
        }
        assert_eq!(
            complex_labels, 3,
            "All three complex fixture types must be readable at {size:?}"
        );
        for label in [
            "Name",
            "Dimensions",
            "Type",
            "Memory",
            "Status",
            "Empty",
            "ZeroImag",
            "Z",
            "Real",
            "Logical",
            "double",
            "logical",
            "0 × 3",
            "1 × 3",
            "2 × 3",
            "Load selected",
            "Refresh file",
            "Cancel",
        ] {
            point(&output, label, size);
        }
        assert!(!app.busy && app.workspace_import.as_ref().unwrap().selected.is_empty());
        click(
            &ctx,
            &mut app,
            size,
            point(&output, "Select supported", size),
        );
        assert_eq!(app.workspace_import.as_ref().unwrap().selected.len(), 5);
        let output = frame(&ctx, &mut app, size, vec![]);
        point(&output, "Load selected", size);
        click(&ctx, &mut app, size, point(&output, "Cancel", size));
        assert!(app.workspace_import.is_none() && !app.busy);
        assert_eq!(app.variables.len(), 1);
        assert!(app.variables["keep"].same_snapshot(&keep));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn mat_import_worker_error_paints_readable_status_and_keeps_workspace() {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let (_dir, catalog) = fixture(1);
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        ctx.set_theme(theme);
        app.run("kept=[3 4];alias=kept;".into());
        finish(&mut app);
        let kept = app.variables["kept"].clone();
        // A real catalog becomes stale before the actual selected-load job.
        std::fs::write(catalog.path(), b"changed").unwrap();
        let mut import = Import::new(catalog);
        import.selected.insert("signal_00000".into());
        app.workspace_import = Some(import);
        let size = Vec2::new(960., 650.);
        for _ in 0..4 {
            frame(&ctx, &mut app, size, vec![]);
        }
        let output = frame(&ctx, &mut app, size, vec![]);
        click(&ctx, &mut app, size, point(&output, "Load selected", size));
        assert!(
            app.busy,
            "The real import button must submit its worker job"
        );
        finish(&mut app);
        let error = app
            .workspace_import
            .as_ref()
            .unwrap()
            .error
            .clone()
            .expect("Stale catalog rejects");
        assert!(!error.is_empty());
        for _ in 0..3 {
            frame(&ctx, &mut app, size, vec![]);
        }
        let output = frame(&ctx, &mut app, size, vec![]);
        let color = if theme == egui::Theme::Dark {
            crate::tokens::DARK.error
        } else {
            crate::tokens::LIGHT.error
        };
        crate::theme::assert_painted_status(&output, &error, color);
        assert_eq!(app.variables.len(), 2);
        assert!(kept.same_snapshot(&app.variables["kept"]));
        assert!(kept.same_snapshot(&app.variables["alias"]));
        assert!(
            app.workspace_import
                .as_ref()
                .unwrap()
                .selected
                .contains("signal_00000")
        );
    }
}
