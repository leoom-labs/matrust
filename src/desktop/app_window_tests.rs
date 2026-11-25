//! Window layout: Separate windows (default) and Docked, both ways.
use super::*;
use matrust::session::{WindowLayout, WindowRect};
use std::{cell::RefCell, rc::Rc};

fn finish(app: &mut Workspace) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.busy && Instant::now() < deadline {
        app.receive();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!app.busy, "{}", app.status);
}
/// One root frame, then one frame of every deferred child (figure) viewport,
/// as eframe would run them.
fn frame(app: &mut Workspace, ctx: &egui::Context) -> egui::FullOutput {
    let output = crate::headless::run(
        ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1440., 900.),
            )),
            ..Default::default()
        },
        |ui| app.ui(ui),
    );
    for (id, child) in &output.viewport_output {
        let Some(callback) = child.viewport_ui_cb.clone() else {
            continue;
        };
        let mut input = egui::RawInput {
            viewport_id: *id,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900., 650.),
            )),
            ..Default::default()
        };
        let info = input.viewports.entry(*id).or_default();
        info.parent = Some(egui::ViewportId::ROOT);
        info.focused = Some(false);
        crate::headless::run(ctx, input, move |ui| callback(ui));
    }
    output
}
/// Immediate child viewports (Editor, Apps) run under a CPU backend that
/// records which viewport ids were shown this frame.
fn install_backend(ctx: &egui::Context) -> Rc<RefCell<Vec<egui::ViewportId>>> {
    let shown = Rc::new(RefCell::new(Vec::new()));
    let log = shown.clone();
    ctx.set_embed_viewports(false);
    egui::Context::set_immediate_viewport_renderer(move |ctx, mut viewport| {
        log.borrow_mut().push(viewport.ids.this);
        let mut input = egui::RawInput {
            viewport_id: viewport.ids.this,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000., 700.),
            )),
            ..Default::default()
        };
        input.viewports.entry(viewport.ids.this).or_default().parent = Some(egui::ViewportId::ROOT);
        crate::headless::run(ctx, input, |ui| (viewport.viewport_ui_cb)(ui));
    });
    shown
}
fn setup(
    layout: WindowLayout,
) -> (
    Workspace,
    egui::Context,
    Rc<RefCell<Vec<egui::ViewportId>>>,
    tempfile::TempDir,
) {
    let ctx = egui::Context::default();
    let shown = install_backend(&ctx);
    let dir = tempfile::tempdir().unwrap();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.folder = dir.path().into();
    app.layout.window_layout = Some(layout);
    for _ in 0..3 {
        frame(&mut app, &ctx);
    }
    shown.borrow_mut().clear();
    (app, ctx, shown, dir)
}
fn settle(
    app: &mut Workspace,
    ctx: &egui::Context,
    shown: &Rc<RefCell<Vec<egui::ViewportId>>>,
) -> egui::FullOutput {
    let mut output = None;
    for _ in 0..6 {
        finish(app);
        shown.borrow_mut().clear();
        output = Some(frame(app, ctx));
    }
    output.unwrap()
}

#[test]
fn product_default_is_separate_windows() {
    assert_eq!(WindowLayout::default(), WindowLayout::Separate);
    // Older sessions without the field load as "absent" = the product default.
    let old: session::DesktopLayout = serde_json::from_str("{}").unwrap();
    assert_eq!(old.window_layout, None);
    let mut app = Workspace::with_context(egui::Context::default(), true);
    app.layout.window_layout = Some(WindowLayout::Separate);
    assert!(app.separate_windows());
    app.layout.window_layout = Some(WindowLayout::Docked);
    assert!(!app.separate_windows());
}

#[test]
fn window_layout_and_placements_survive_a_session_round_trip() {
    let rect = WindowRect {
        x: 120.,
        y: 80.,
        width: 900.,
        height: 640.,
    };
    let layout = session::DesktopLayout {
        window_layout: Some(WindowLayout::Docked),
        editor_window: Some(rect),
        apps_window: Some(WindowRect { x: -1500., ..rect }),
        ..session::DesktopLayout::fresh_desktop()
    };
    let text = serde_json::to_string(&layout).unwrap();
    let back: session::DesktopLayout = serde_json::from_str(&text).unwrap();
    assert_eq!(back, layout);
    // Malformed values are dropped, not fatal.
    let bad: session::DesktopLayout = serde_json::from_str(
        r#"{"window_layout":"floating","editor_window":{"x":0,"y":0,"width":5,"height":5}}"#,
    )
    .unwrap();
    assert_eq!((bad.window_layout, bad.editor_window), (None, None));
}

#[test]
fn separate_layout_opens_the_editor_in_its_own_viewport_at_the_saved_place() {
    let (mut app, ctx, shown, _dir) = setup(WindowLayout::Separate);
    let saved = WindowRect {
        x: 200.,
        y: 150.,
        width: 800.,
        height: 600.,
    };
    app.layout.editor_window = Some(saved);
    app.set_mode(modes::Mode::Editor);
    settle(&mut app, &ctx, &shown);
    assert!(
        app.editor_window.detached,
        "Editor did not get its own window"
    );
    assert!(
        shown.borrow().contains(&editor_window::viewport()),
        "no Editor viewport shown"
    );
    assert_eq!(
        app.mode(),
        modes::Mode::Command,
        "main window keeps the Command Window"
    );
    assert_eq!(app.editor_window_placement(), Some(saved));
    let builder = app_window::builder("Editor", Some(saved), [1000., 700.], [480., 320.]);
    assert_eq!(builder.position, Some(egui::pos2(200., 150.)));
    assert_eq!(builder.inner_size, Some(egui::vec2(800., 600.)));
}

#[test]
fn docked_layout_keeps_the_editor_in_the_main_window() {
    let (mut app, ctx, shown, _dir) = setup(WindowLayout::Docked);
    app.set_mode(modes::Mode::Editor);
    settle(&mut app, &ctx, &shown);
    assert!(!app.editor_window.detached);
    assert!(!shown.borrow().contains(&editor_window::viewport()));
    assert_eq!(app.mode(), modes::Mode::Editor);
}

#[test]
fn separate_layout_opens_apps_in_one_native_window_and_docked_in_the_main_window() {
    for tool in [toolkits::Tool::Designer, toolkits::Tool::Model] {
        let (mut app, ctx, shown, _dir) = setup(WindowLayout::Separate);
        app.toolkits.open_tool(tool);
        settle(&mut app, &ctx, &shown);
        assert!(
            shown.borrow().contains(&app_window::apps_viewport()),
            "{tool:?}: no Apps viewport"
        );
        assert_ne!(
            app.mode(),
            modes::Mode::Apps,
            "{tool:?}: main window must not show the app"
        );

        let (mut app, ctx, shown, _dir) = setup(WindowLayout::Docked);
        app.toolkits.open_tool(tool);
        settle(&mut app, &ctx, &shown);
        assert!(
            !shown.borrow().contains(&app_window::apps_viewport()),
            "{tool:?}: docked app opened a window"
        );
        assert_eq!(app.mode(), modes::Mode::Apps);
    }
}

/// Figure windows are deferred viewports: like `detached_figures::tests`, they
/// need the Windows gate (headless Linux reports them embedded and docks them).
mod detached_figures {
    use super::*;

    #[test]
    fn separate_layout_pops_each_new_figure_into_a_native_viewport() {
        let (mut app, ctx, shown, _dir) = setup(WindowLayout::Separate);
        app.run("figure(1);plot([1 2 3]);figure(2);plot([3 2 1]);".into());
        let output = settle(&mut app, &ctx, &shown);
        for id in [1, 2] {
            assert!(app.detached.contains(id), "figure {id} stayed docked");
            let viewport = app.detached.viewport_of(id).unwrap();
            assert!(
                output.viewport_output.contains_key(&viewport),
                "figure {id}: no viewport"
            );
        }
    }

    #[test]
    fn docked_layout_keeps_figures_docked_and_switching_layout_works_both_ways() {
        let (mut app, ctx, shown, _dir) = setup(WindowLayout::Docked);
        app.run("figure(1);plot([1 2 3]);".into());
        settle(&mut app, &ctx, &shown);
        assert!(!app.detached.contains(1));
        app.set_window_layout(&ctx, WindowLayout::Separate);
        settle(&mut app, &ctx, &shown);
        assert!(
            app.detached.contains(1),
            "switching to Separate did not pop the figure out"
        );
        app.set_window_layout(&ctx, WindowLayout::Docked);
        settle(&mut app, &ctx, &shown);
        settle(&mut app, &ctx, &shown);
        assert!(
            !app.detached.contains(1),
            "switching to Docked did not dock the figure"
        );
        assert_eq!(app.layout.window_layout, Some(WindowLayout::Docked));
    }
}

/// MathWorks undocked Editor stays its own window
/// (https://www.mathworks.com/help/matlab/matlab_env/undock-and-dock-windows.html).
/// Reference value: the main window stays [`modes::Mode::Command`] and the
/// Editor viewport is focused.
#[test]
fn review_1111_second_editor_click_keeps_command_and_focuses_editor() {
    let (mut app, ctx, _shown, _dir) = setup(WindowLayout::Separate);
    app.set_mode(modes::Mode::Editor);
    settle(&mut app, &ctx, &_shown);
    assert!(app.editor_window.detached);
    assert_eq!(app.mode(), modes::Mode::Command);
    // The Editor tab / Ctrl+2 calls set_mode during the frame. A call between
    // frames stamps the previous pass, and settle's last frame has already
    // taken any queued viewport command.
    let output = crate::headless::run(
        &ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1440., 900.),
            )),
            ..Default::default()
        },
        |ui| {
            app.set_mode(modes::Mode::Editor);
            app.ui(ui);
        },
    );
    assert!(app.editor_window.detached);
    assert_eq!(app.mode(), modes::Mode::Command);
    assert!(app.central_command());
    let editor = editor_window::viewport();
    assert!(
        output.viewport_output.get(&editor).is_some_and(|viewport| {
            viewport
                .commands
                .iter()
                .any(|command| matches!(command, egui::ViewportCommand::Focus))
        }),
        "detached Editor was not focused"
    );
}

/// Reference value: a separate Filter Designer or Block Simulation stays
/// selected after Command, Editor, Figures and Ctrl+0. A docked tool is
/// cleared (selected tool is `None`).
#[test]
fn review_1111_mode_switch_deselects_only_a_docked_tool() {
    for tool in [toolkits::Tool::Designer, toolkits::Tool::Model] {
        let (mut app, ctx, shown, _dir) = setup(WindowLayout::Separate);
        app.toolkits.open_tool(tool);
        settle(&mut app, &ctx, &shown);
        assert_eq!(app.toolkits.selected_tool(), Some(tool));
        assert!(shown.borrow().contains(&app_window::apps_viewport()));
        for mode in [
            modes::Mode::Command,
            modes::Mode::Editor,
            modes::Mode::Figures,
        ] {
            app.set_mode(mode);
            settle(&mut app, &ctx, &shown);
            assert_eq!(
                app.toolkits.selected_tool(),
                Some(tool),
                "{tool:?} closed by {mode:?}"
            );
            assert!(
                shown.borrow().contains(&app_window::apps_viewport()),
                "{tool:?} window closed by {mode:?}"
            );
        }
        for pressed in [true, false] {
            crate::headless::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1440., 900.),
                    )),
                    events: vec![egui::Event::Key {
                        key: egui::Key::Num0,
                        physical_key: Some(egui::Key::Num0),
                        pressed,
                        repeat: false,
                        modifiers: egui::Modifiers::CTRL,
                    }],
                    ..Default::default()
                },
                |ui| app.ui(ui),
            );
        }
        settle(&mut app, &ctx, &shown);
        assert_eq!(
            app.toolkits.selected_tool(),
            Some(tool),
            "{tool:?} closed by Ctrl+0"
        );
    }
    let (mut app, ctx, shown, _dir) = setup(WindowLayout::Docked);
    app.toolkits.open_tool(toolkits::Tool::Designer);
    settle(&mut app, &ctx, &shown);
    assert_eq!(app.mode(), modes::Mode::Apps);
    app.set_mode(modes::Mode::Command);
    settle(&mut app, &ctx, &shown);
    assert_eq!(app.toolkits.selected_tool(), None);
}
