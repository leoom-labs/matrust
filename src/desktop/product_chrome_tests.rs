fn chrome_text_positions(output: &egui::FullOutput) -> Vec<(String, egui::Pos2)> {
    fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Pos2)>) {
        match shape {
            egui::Shape::Text(text) => {
                out.push((text.galley.text().to_owned(), text.pos + text.galley.size() * 0.5));
            }
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk(shape, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for shape in &output.shapes {
        walk(&shape.shape, &mut out);
    }
    out
}
fn chrome_has_text(output: &egui::FullOutput, label: &str) -> bool {
    chrome_text_positions(output).iter().any(|(text, _)| text == label)
}
fn chrome_settled(app: &mut Workspace, ctx: &egui::Context) -> egui::FullOutput {
    for _ in 0..3 {
        frame(app, ctx, vec![]);
    }
    frame(app, ctx, vec![])
}
fn chrome_titles(output: &egui::FullOutput) -> Vec<String> {
    output
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .map(|viewport| {
            viewport
                .commands
                .iter()
                .filter_map(|command| match command {
                    egui::ViewportCommand::Title(title) => Some(title.clone()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn report_a_problem_opens_the_script_issue_form_with_version_and_system() {
    let url = chrome::report_url();
    assert!(url.starts_with("https://github.com/leoom-labs/matrust/issues/new?template=script.yml&"));
    assert!(url.contains(&format!("version={}", matrust::VERSION)));
    assert!(url.ends_with(&format!("os={}", std::env::consts::OS)));
}

#[test]
fn product_chrome_banner_names_version_and_help_only() {
    assert_eq!(matrust::VERSION, env!("CARGO_PKG_VERSION"));
    let app = Workspace::with_context(egui::Context::default(), true);
    assert_eq!(
        app.console,
        vec![
            format!("MatRust {}", matrust::VERSION),
            "Type help to list functions, or help plot for one function.".to_owned(),
        ]
    );
    assert_eq!(app.console, chrome::banner());
}

#[test]
fn product_chrome_status_bar_shows_run_state_and_variable_plurals() {
    assert_eq!(chrome::variable_count(0), "No variables");
    assert_eq!(chrome::variable_count(1), "1 variable");
    assert_eq!(chrome::variable_count(12), "12 variables");
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    let output = chrome_settled(&mut app, &ctx);
    assert!(chrome_has_text(&output, "Ready"));
    assert!(chrome_has_text(&output, "No variables"));
    assert!(chrome_text_positions(&output).iter().all(|(text, _)| !text.contains("workers")));
    app.run("first_value=1;".into());
    assert!(app.busy);
    assert_eq!(app.status, chrome::RUNNING);
    finish(&mut app);
    assert!(app.status.starts_with("Completed in ") && app.status.ends_with(" ms"), "{}", app.status);
    let output = chrome_settled(&mut app, &ctx);
    assert!(chrome_has_text(&output, &app.status.clone()));
    assert!(chrome_has_text(&output, "1 variable"));
    app.run("second_value=2;".into());
    finish(&mut app);
    assert!(chrome_has_text(&chrome_settled(&mut app, &ctx), "2 variables"));
    // A queued job keeps the run state on screen until its reply arrives.
    app.busy = true;
    app.status = chrome::RUNNING.into();
    let output = frame(&mut app, &ctx, vec![]);
    assert!(chrome_has_text(&output, chrome::RUNNING));
    app.busy = false;
}

#[test]
fn product_chrome_title_tracks_active_script_and_unsaved_marker() {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    assert_eq!(app.window_title(), "MatRust");
    assert_eq!(chrome_titles(&frame(&mut app, &ctx, vec![])), ["MatRust"]);
    assert!(chrome_titles(&frame(&mut app, &ctx, vec![])).is_empty(), "unchanged title is not resent");
    app.new_document("");
    assert_eq!(app.window_title(), "Untitled.mr - MatRust");
    app.documents[app.active].text = "x = 1;".into();
    app.documents[app.active].changed();
    assert!(app.documents[app.active].dirty);
    assert_eq!(chrome_titles(&frame(&mut app, &ctx, vec![])), ["*Untitled.mr - MatRust"]);
    app.documents = vec![Document::open(std::path::Path::new("examples/signals.m")).unwrap()];
    app.active = 0;
    assert_eq!(chrome_titles(&frame(&mut app, &ctx, vec![])), ["signals.m - MatRust"]);
    app.documents[0].text.push_str("\n% edited");
    app.documents[0].changed();
    assert_eq!(app.window_title(), "*signals.m - MatRust");
}

#[test]
fn product_chrome_help_menu_opens_function_help_shortcuts_and_about() {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    chrome_settled(&mut app, &ctx);
    click_label(&mut app, &ctx, "Help");
    click_label(&mut app, &ctx, "Function help");
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.command, "help ");
    assert!(ctx.memory(|m| m.has_focus(egui::Id::new("command_input"))));
    let cursor = egui::text_edit::TextEditState::load(&ctx, egui::Id::new("command_input"))
        .and_then(|state| state.cursor.char_range())
        .unwrap();
    assert_eq!(cursor.primary.index, egui::text::CharIndex("help ".len()));
    click_label(&mut app, &ctx, "Help");
    click_label(&mut app, &ctx, "Keyboard shortcuts");
    assert!(app.focus_areas.is_open());
    press(&mut app, &ctx, egui::Key::Escape);
    assert!(!app.focus_areas.is_open());
    let version = format!("Version {}", matrust::VERSION);
    for close_with_escape in [true, false] {
        click_label(&mut app, &ctx, "Help");
        click_label(&mut app, &ctx, "About MatRust");
        assert!(app.chrome.about);
        let output = chrome_settled(&mut app, &ctx);
        for label in [
            "MatRust",
            version.as_str(),
            "Numerical workspace for a subset of the MATLAB language.",
            "Fonts: Inter and JetBrains Mono (SIL Open Font License 1.1).",
            "Third-party notices",
            "Close",
        ] {
            assert!(chrome_has_text(&output, label), "{label}");
        }
        click_label(&mut app, &ctx, "Third-party notices");
        let output = chrome_settled(&mut app, &ctx);
        assert!(chrome_text_positions(&output).iter().any(|(text, _)| text.contains("notices/")));
        if close_with_escape {
            press(&mut app, &ctx, egui::Key::Escape);
        } else {
            click_label(&mut app, &ctx, "Close");
        }
        assert!(!app.chrome.about);
        assert!(!chrome_has_text(&chrome_settled(&mut app, &ctx), &version));
    }
}

#[test]
fn product_chrome_about_dialog_blocks_background_script_shortcuts_until_closed() {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    let size = egui::vec2(1440., 900.);
    chrome_settled(&mut app, &ctx);
    app.new_document("");
    app.new_document("");
    assert_eq!(app.documents.len(), 2);
    assert_eq!(app.active, 1);
    click_label(&mut app, &ctx, "Help");
    click_label(&mut app, &ctx, "About MatRust");
    chrome_settled(&mut app, &ctx);
    assert!(app.chrome.about && app.file_action_pending());
    command_first_key(&mut app, &ctx, size, egui::Key::N, egui::Modifiers::CTRL);
    command_first_key(&mut app, &ctx, size, egui::Key::Tab, egui::Modifiers::CTRL);
    assert_eq!((app.documents.len(), app.active), (2, 1));
    assert!(app.chrome.about);
    press(&mut app, &ctx, egui::Key::Escape);
    assert!(!app.chrome.about && !app.file_action_pending());
    command_first_key(&mut app, &ctx, size, egui::Key::N, egui::Modifiers::CTRL);
    assert_eq!((app.documents.len(), app.active), (3, 2));
}

#[test]
fn product_chrome_window_icon_is_accent_square_with_white_wave() {
    let icon = chrome::window_icon();
    assert_eq!((icon.width, icon.height), (256, 256));
    assert_eq!(icon.rgba.len(), 256 * 256 * 4);
    let pixel = |x: usize, y: usize| {
        let i = (y * 256 + x) * 4;
        [icon.rgba[i], icon.rgba[i + 1], icon.rgba[i + 2], icon.rgba[i + 3]]
    };
    let [r, g, b] = chrome::icon_accent();
    // The sine crosses the centre at half a period; the centre column above
    // and below the stroke and the quadrant centres opposite the wave are accent.
    for (x, y) in [(128, 64), (128, 192), (127, 96), (128, 160), (192, 64), (64, 192)] {
        assert_eq!(pixel(x, y), [r, g, b, 255], "({x}, {y})");
    }
    assert_eq!(pixel(128, 128), [255, 255, 255, 255]);
    assert_eq!(pixel(0, 0)[3], 0, "rounded corner is transparent");
    assert_eq!(pixel(0, 128)[3], 255, "edge midpoint is inside the square");
    let pixels = || (0..256 * 256).map(|k| pixel(k % 256, k / 256));
    let white = pixels().filter(|p| p == &[255, 255, 255, 255]).count();
    assert!((2_000..12_000).contains(&white), "{white}");
    // Anti-aliasing: the stroke edge has intermediate values.
    assert!(pixels().any(|p| p[3] == 255 && p[0] > r && p[0] < 255));
}

#[test]
fn product_chrome_workspace_empty_state_disappears_with_variables() {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    assert!(chrome_has_text(&chrome_settled(&mut app, &ctx), chrome::NO_VARIABLES));
    app.run("x = 1:10;".into());
    finish(&mut app);
    let output = chrome_settled(&mut app, &ctx);
    assert!(!chrome_has_text(&output, chrome::NO_VARIABLES));
    assert!(chrome_has_text(&output, "1 variable"));
}

#[test]
fn product_chrome_figure_empty_state_disappears_with_a_figure() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.layout.mode = layout::PaneMode::Figure;
    let output = chrome_settled(&mut app, &ctx);
    assert!(chrome_has_text(&output, chrome::NO_FIGURE));
    assert!(chrome_has_text(&output, chrome::NO_FIGURE_HINT));
    app.run("plot(1:10);".into());
    finish(&mut app);
    let output = chrome_settled(&mut app, &ctx);
    assert!(app.figure.is_some());
    assert!(!chrome_has_text(&output, chrome::NO_FIGURE));
    assert!(!chrome_has_text(&output, chrome::NO_FIGURE_HINT));
}

#[test]
fn product_chrome_editor_empty_state_offers_new_open_and_five_recent_files() {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    let dir = tempfile::tempdir().unwrap();
    app.recent = (1..=7).map(|i| dir.path().join(format!("recent_{i}.m"))).collect();
    let center = app.layout.center.as_mut().unwrap();
    center.command = false;
    app.layout.mode = layout::PaneMode::Editor;
    assert!(app.has_startup_placeholder());
    let placeholder = app.documents[0].id;
    let output = chrome_settled(&mut app, &ctx);
    let texts = chrome_text_positions(&output);
    let heading = texts.iter().find(|(text, _)| text == chrome::NO_SCRIPT).unwrap().1;
    let below = |label: &str| {
        texts
            .iter()
            .find(|(text, pos)| text == label && pos.y > heading.y)
            .map(|(_, pos)| *pos)
    };
    let new = below("New script").expect("primary New script button");
    assert!(below("Open script...").is_some());
    for i in 1..=5 {
        assert!(below(&format!("recent_{i}.m")).is_some(), "recent {i}");
    }
    assert!(below("recent_6.m").is_none() && below("recent_7.m").is_none());
    assert!(ctx.read_response(editor::id(placeholder)).is_none());
    for pressed in [true, false] {
        pointer_frame(&mut app, &ctx, egui::vec2(1440., 900.), new, pressed);
    }
    assert!(!app.has_startup_placeholder());
    assert_eq!(app.documents.len(), 1);
    let output = chrome_settled(&mut app, &ctx);
    assert!(!chrome_has_text(&output, chrome::NO_SCRIPT));
    assert!(ctx.read_response(editor::id(app.documents[0].id)).is_some());
}
