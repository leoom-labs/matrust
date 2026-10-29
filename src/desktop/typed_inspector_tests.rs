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
fn frame(app: &mut Workspace, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
    crate::headless::run(
        ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1100., 760.),
            )),
            events,
            ..Default::default()
        },
        |ctx| app.variable_editor(ctx),
    )
}
fn text(shape: &egui::Shape, clip: egui::Rect, found: &mut Vec<(String, egui::Rect)>) {
    match shape {
        egui::Shape::Text(t) => {
            let rect = t.visual_bounding_rect();
            if clip.contains_rect(rect) {
                found.push((t.galley.text().into(), rect));
            }
        }
        egui::Shape::Vec(v) => {
            for shape in v {
                text(shape, clip, found);
            }
        }
        _ => {}
    }
}
fn labels(output: &egui::FullOutput) -> Vec<(String, egui::Rect)> {
    let mut found = Vec::new();
    for shape in &output.shapes {
        text(&shape.shape, shape.clip_rect, &mut found);
    }
    found
}
fn settle(app: &mut Workspace, ctx: &egui::Context) -> egui::FullOutput {
    for _ in 0..3 {
        frame(app, ctx, vec![]);
    }
    frame(app, ctx, vec![])
}
fn click_at(app: &mut Workspace, ctx: &egui::Context, pos: egui::Pos2) -> egui::FullOutput {
    for pressed in [true, false] {
        frame(
            app,
            ctx,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    frame(app, ctx, vec![])
}
fn click(app: &mut Workspace, ctx: &egui::Context, label: &str) {
    let out = settle(app, ctx);
    let pos = labels(&out)
        .into_iter()
        .find(|(s, _)| s == label)
        .unwrap_or_else(|| panic!("Missing visible {label}"))
        .1
        .center();
    click_at(app, ctx, pos);
}

#[test]
fn typed_utf16_grid_preserves_units_and_worker_copies_nul_without_lossy_projection() {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        ctx.set_theme(theme);
        // A small matrix makes all nine exact units visible without horizontal scrolling.
        app.run("s=char([65 55296 55357;0 56320 56832;65 65534 65535]);Alias=s;".into());
        finish(&mut app);
        let original = app.variables["s"].clone();
        app.inspect_variable("s".into());
        let out = settle(&mut app, &ctx);
        let items = labels(&out);
        for expected in [
            "\\u0000", "\\uD800", "\\uDC00", "\\uD83D", "\\uDE00", "\\uFFFE", "\\uFFFF",
        ] {
            assert!(
                items.iter().any(|(s, _)| s == expected),
                "Missing complete {expected}"
            );
        }
        assert!(items.iter().any(|(s, _)| s == "Apply cell"));
        assert!(
            !items
                .iter()
                .any(|(s, _)| s.contains('\u{fffd}') || s == "Copy text row")
        );
        let pos = app.numeric_editor.cells[&(1, 0)].center();
        click_at(&mut app, &ctx, pos);
        click(&mut app, &ctx, "Copy cell");
        finish(&mut app);
        let copied = frame(&mut app, &ctx, vec![]);
        assert!(
            copied
                .platform_output
                .commands
                .iter()
                .any(|c| matches!(c,egui::OutputCommand::CopyText(s) if s == "\\u0000"))
        );
        assert!(app.variables["s"].same_snapshot(&original));
        assert!(app.variables["Alias"].same_snapshot(&original));
    }
}

#[test]
fn real_nested_cell_navigation_breadcrumbs_and_changed_root_reset_preserve_aliases() {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.run("A=[1 2;3 4];C={A,{char([0 55296]),A}};Alias=C;".into());
    finish(&mut app);
    let original = app.variables["C"].clone();
    let numeric = app.variables["A"].clone();
    app.inspect_variable("C".into());
    click(&mut app, &ctx, "Open {1,2}");
    click(&mut app, &ctx, "Open {1,1}");
    let out = settle(&mut app, &ctx);
    assert!(labels(&out).iter().any(|(s, _)| s == "\\uD800"));
    click(&mut app, &ctx, "Cell {1,2}");
    click(&mut app, &ctx, "Open {1,2}");
    let out = settle(&mut app, &ctx);
    assert!(labels(&out).iter().any(|(s, _)| s == "Copy selection"));
    assert!(labels(&out).iter().any(|(s, _)| s == "Apply cell"));
    assert!(app.variables["C"].same_snapshot(&original));
    assert!(app.variables["A"].same_snapshot(&numeric));
    app.run("C={99};".into());
    finish(&mut app);
    let out = settle(&mut app, &ctx);
    assert!(
        labels(&out)
            .iter()
            .any(|(s, _)| s == "Variable changed; returned to the root value.")
    );
    assert!(labels(&out).iter().any(|(s, _)| s == "Open {1,1}"));
    assert!(!labels(&out).iter().any(|(s, _)| s.starts_with("Cell {")));
    assert!(app.variables["Alias"].same_snapshot(&original));
    click(&mut app, &ctx, "Open {1,1}");
    click(&mut app, &ctx, "Root");
    assert!(
        labels(&settle(&mut app, &ctx))
            .iter()
            .any(|(s, _)| s == "Open {1,1}")
    );
}

#[test]
fn graphics_inspector_reports_real_live_deleted_kinds_and_updates_after_worker_close() {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.run("f=figure(7);h=plot([1 2]);a=gca;lg=legend('signal');t=annotation('textbox',[.1 .8 .5 .1],'String','note','Interpreter','none','EdgeColor','none');gone=figure(9);close(9);G=[f a h lg t gone];Alias=G;".into());
    finish(&mut app);
    assert!(
        app.variables.contains_key("G"),
        "Graphics fixture failed: {}",
        app.status
    );
    let original = app.variables["G"].clone();
    app.inspect_variable("G".into());
    let items = labels(&settle(&mut app, &ctx));
    for (index, kind) in [
        (1, "Figure"),
        (2, "Axes"),
        (3, "Series"),
        (4, "Legend"),
        (5, "TextBox"),
    ] {
        assert!(
            items
                .iter()
                .any(|(s, _)| s == &format!("Handle {index} | {kind} | Live in Figure 7"))
        );
    }
    assert!(
        items
            .iter()
            .any(|(s, _)| s == "Handle 6 | Figure | Deleted / unavailable")
    );
    assert!(
        !items
            .iter()
            .any(|(s, _)| s.contains("character/cell preview"))
    );
    app.run("close(7);figure(7);plot([9 8]);".into());
    finish(&mut app);
    let items = labels(&settle(&mut app, &ctx));
    assert_eq!(
        items
            .iter()
            .filter(|(s, _)| s.starts_with("Handle ") && s.contains("Deleted / unavailable"))
            .count(),
        6
    );
    assert!(app.variables["G"].same_snapshot(&original));
    assert!(app.variables["Alias"].same_snapshot(&original));
}

#[test]
fn large_typed_inspector_is_virtualized_and_modal_guards_block_navigation() {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.run("C=cell(1,65536);Alias=C;".into());
    finish(&mut app);
    let original = app.variables["C"].clone();
    app.inspect_variable("C".into());
    let out = settle(&mut app, &ctx);
    let items = labels(&out);
    assert!(out.shapes.len() < 1500);
    assert!(
        items
            .iter()
            .filter(|(s, _)| s.starts_with("Open {"))
            .count()
            < 32
    );
    let pos = items
        .iter()
        .find(|(s, _)| s == "Open {1,1}")
        .unwrap()
        .1
        .center();
    app.pending_quit = true;
    click_at(&mut app, &ctx, pos);
    assert!(
        labels(&settle(&mut app, &ctx))
            .iter()
            .any(|(s, _)| s == "Open {1,1}")
    );
    app.pending_quit = false;
    let scroll = items
        .iter()
        .find(|(s, _)| s == "Open {1,5}")
        .unwrap()
        .1
        .center();
    for _ in 0..4 {
        frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(scroll),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0., -160.),
                    modifiers: egui::Modifiers::NONE,
                    phase: egui::TouchPhase::Move,
                },
            ],
        );
    }
    let out = settle(&mut app, &ctx);
    assert!(!labels(&out).iter().any(|(s, _)| s == "Open {1,1}"));
    assert!(labels(&out).iter().any(|(s, _)| s.starts_with("Open {1,")));
    assert!(app.variables["C"].same_snapshot(&original));
    assert!(app.variables["Alias"].same_snapshot(&original));
}
