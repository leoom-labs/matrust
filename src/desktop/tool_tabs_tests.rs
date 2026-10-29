// Filter Designer and Block Simulation are closable tabs in the main work area.
fn tool_tab_texts(shape: &egui::epaint::Shape, label: &str, found: &mut Vec<egui::Pos2>) {
    match shape {
        egui::epaint::Shape::Text(t) if t.galley.text() == label => {
            found.push(t.pos + t.galley.size() * 0.5)
        }
        egui::epaint::Shape::Vec(v) => v.iter().for_each(|s| tool_tab_texts(s, label, found)),
        _ => {}
    }
}
// Tool tab labels sit in the strip below the toolstrip buttons of the same name.
fn tool_tab_label(output: &egui::FullOutput, label: &str) -> Option<egui::Pos2> {
    let mut found = Vec::new();
    output.shapes.iter().for_each(|s| tool_tab_texts(&s.shape, label, &mut found));
    found.into_iter().max_by(|a, b| a.y.total_cmp(&b.y))
}
fn tool_tab_on_row(output: &egui::FullOutput, label: &str, row: egui::Pos2) -> Option<egui::Pos2> {
    let mut found = Vec::new();
    output.shapes.iter().for_each(|s| tool_tab_texts(&s.shape, label, &mut found));
    found.into_iter().find(|p| (p.y - row.y).abs() < 1.)
}
fn tool_tab_close(output: &egui::FullOutput, tab: egui::Pos2) -> egui::Pos2 {
    let mut found = Vec::new();
    output.shapes.iter().for_each(|s| tool_tab_texts(&s.shape, "\u{d7}", &mut found));
    found
        .into_iter()
        .filter(|p| p.x > tab.x && (p.y - tab.y).abs() < 8.)
        .min_by(|a, b| a.x.total_cmp(&b.x))
        .expect("tool tab close button")
}
fn tool_tab_click(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2, point: egui::Pos2) {
    for pressed in [true, false] {
        pointer_frame(app, ctx, size, point, pressed);
    }
}
fn no_tool_windows(ctx: &egui::Context) -> bool {
    ["Filter Designer", "block_simulation"]
        .iter()
        .all(|id| ctx.memory(|m| m.area_rect(egui::Id::new(*id))).is_none())
}

#[test]
fn apps_toolstrip_opens_tools_as_selected_tabs_without_windows() {
    use toolkits::Tool;
    let size = egui::vec2(1440., 900.);
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    assert!(app.central_command() && app.toolkits.tabs().next().is_none());
    click_label_at(&mut app, &ctx, size, "Apps");
    click_label_at(&mut app, &ctx, size, "Filter Designer");
    assert_eq!(app.toolkits.selected_tool(), Some(Tool::Designer));
    let output = frame_at(&mut app, &ctx, size, vec![]);
    let designer = app.toolkits.designer_region().expect("designer fills the main area");
    assert!(app.toolkits.model_document_region().is_none());
    let tab = tool_tab_label(&output, "Filter Designer").unwrap();
    assert!(tab.y < designer.top(), "tab strip above the tool content");
    click_label_at(&mut app, &ctx, size, "Block Simulation");
    assert_eq!(app.toolkits.tabs().collect::<Vec<_>>(), [Tool::Designer, Tool::Model]);
    assert_eq!(app.toolkits.selected_tool(), Some(Tool::Model));
    assert_eq!(app.generic_save_target(), document_save::Pane::Model);
    let output = frame_at(&mut app, &ctx, size, vec![]);
    let (model, _) = app.toolkits.model_document_region().expect("model fills the main area");
    assert!(app.toolkits.designer_region().is_none());
    // Same main area for both tools: the rect the documents use, not a floating window.
    assert_eq!(model, designer);
    assert!(no_tool_windows(&ctx));
    let designer_tab = tool_tab_label(&output, "Filter Designer").unwrap();
    let model_tab = tool_tab_label(&output, "Block Simulation").unwrap();
    assert!(designer_tab.x < model_tab.x && (designer_tab.y - model_tab.y).abs() < 1.);
    let command = tool_tab_on_row(&output, "Command", designer_tab).expect("Command tab");
    assert!(command.x < designer_tab.x);
    tool_tab_click(&mut app, &ctx, size, designer_tab);
    assert_eq!(app.toolkits.selected_tool(), Some(Tool::Designer));
    frame_at(&mut app, &ctx, size, vec![]);
    assert!(app.toolkits.designer_region().is_some());
    tool_tab_click(&mut app, &ctx, size, command);
    assert_eq!(app.toolkits.selected_tool(), None);
    assert!(app.central_command() && app.toolkits.designer_open && app.toolkits.model_open);
    frame_at(&mut app, &ctx, size, vec![]);
    assert!(app.toolkits.designer_region().is_none() && app.toolkits.model_document_region().is_none());
}

#[test]
fn closing_a_dirty_model_tab_keeps_the_unsaved_model_like_the_former_window() {
    use toolkits::Tool;
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, path, script) = model_document_app(&ctx, dir.path());
    let size = egui::vec2(1440., 900.);
    app.toolkits.open_tool(Tool::Designer);
    app.show_model_document();
    let model = serde_json::to_string(&app.toolkits.model).unwrap();
    let original = std::fs::read(&path).unwrap();
    assert!(app.toolkits.model_dirty());
    let output = frame_at(&mut app, &ctx, size, vec![]);
    let tab = tool_tab_label(&output, "Block Simulation *").expect("dirty marker on the tab");
    tool_tab_click(&mut app, &ctx, size, tool_tab_close(&output, tab));
    // Existing close path: hide the view, keep unsaved work, no prompt, no write.
    assert!(!app.toolkits.model_open && app.toolkits.model_dirty());
    assert_eq!(serde_json::to_string(&app.toolkits.model).unwrap(), model);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(std::fs::read_to_string(&script).unwrap(), "old=1;\n");
    assert!(ctx.memory(|m| m.top_modal_layer()).is_none());
    assert!(app.figure_close.is_none() && app.pending_close.is_none() && !app.allow_close);
    assert_eq!(app.generic_save_target(), document_save::Pane::Figure);
    // The neighbouring tool tab takes over the main area.
    assert_eq!(app.toolkits.selected_tool(), Some(Tool::Designer));
    let output = frame_at(&mut app, &ctx, size, vec![]);
    let tab = tool_tab_label(&output, "Filter Designer").unwrap();
    tool_tab_click(&mut app, &ctx, size, tool_tab_close(&output, tab));
    assert!(!app.toolkits.designer_open && app.toolkits.selected_tool().is_none());
    frame_at(&mut app, &ctx, size, vec![]);
    assert!(app.toolkits.tabs().next().is_none());
    click_label_at(&mut app, &ctx, size, "Menu");
    click_label_at(&mut app, &ctx, size, "Tools");
    click_label_at(&mut app, &ctx, size, "Block Simulation");
    assert_eq!(app.toolkits.selected_tool(), Some(Tool::Model));
    assert_eq!(serde_json::to_string(&app.toolkits.model).unwrap(), model);
}

#[test]
fn add_to_block_model_selects_the_block_simulation_tab() {
    use toolkits::Tool;
    let size = egui::vec2(1440., 900.);
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.toolkits.open_tool(Tool::Designer);
    let blocks = app.toolkits.model.nodes.len();
    click_label_at(&mut app, &ctx, size, "Add to block model");
    finish(&mut app);
    frame_at(&mut app, &ctx, size, vec![]);
    assert_eq!(app.toolkits.model.nodes.len(), blocks + 1, "{}", app.toolkits.message);
    assert_eq!(app.toolkits.selected_tool(), Some(Tool::Model));
    assert_eq!(app.toolkits.tabs().collect::<Vec<_>>(), [Tool::Designer, Tool::Model]);
    assert!(app.toolkits.model_document_region().is_some());
}

#[test]
fn layout_modes_and_reset_keep_tools_open_and_return_to_documents() {
    use toolkits::Tool;
    let size = egui::vec2(1440., 900.);
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.run("plot([1 2 3]);".into());
    finish(&mut app);
    app.toolkits.open_tool(Tool::Designer);
    app.toolkits.open_tool(Tool::Model);
    for mode in [
        layout::PaneMode::Editor,
        layout::PaneMode::Figure,
        layout::PaneMode::Split,
    ] {
        app.select_tool_tab(Tool::Model);
        app.select_document_layout(mode);
        assert_eq!((app.toolkits.selected_tool(), app.layout.mode), (None, mode));
        frame_at(&mut app, &ctx, size, vec![]);
        assert!(app.toolkits.model_document_region().is_none());
    }
    app.select_tool_tab(Tool::Designer);
    app.reset_layout(&ctx);
    assert!(app.toolkits.designer_open && app.toolkits.model_open);
    assert!(app.toolkits.selected_tool().is_none() && app.central_command());
    // Legacy layouts without document tabs get one tab back to their documents.
    app.layout.center = None;
    app.select_tool_tab(Tool::Designer);
    let output = frame_at(&mut app, &ctx, size, vec![]);
    assert!(app.toolkits.designer_region().is_some() && no_tool_windows(&ctx));
    let documents = tool_tab_label(&output, "Editor and figure").expect("legacy documents tab");
    tool_tab_click(&mut app, &ctx, size, documents);
    assert!(app.toolkits.selected_tool().is_none());
    assert!(app.toolkits.designer_open && app.toolkits.model_open);
}

#[test]
fn closing_the_shown_tool_hands_the_save_target_to_the_fallback_tool() {
    use toolkits::Tool;
    let mut app = Workspace::with_context(egui::Context::default(), true);
    app.toolkits.open_tool(Tool::Model);
    app.select_tool_tab(Tool::Model);
    app.select_document_layout(layout::PaneMode::Editor);
    assert_eq!(app.generic_save_target(), document_save::Pane::Script);
    app.toolkits.open_tool(Tool::Designer);
    app.select_tool_tab(Tool::Designer);
    app.close_tool_tab(Tool::Designer);
    assert_eq!(app.toolkits.selected_tool(), Some(Tool::Model));
    assert_eq!(app.generic_save_target(), document_save::Pane::Model);
    // Nothing left: the documents take Save/Close back from the hidden model.
    app.close_tool_tab(Tool::Model);
    assert_eq!(app.toolkits.selected_tool(), None);
    assert_eq!(app.generic_save_target(), document_save::Pane::Script);
}

#[test]
fn open_tool_tabs_survive_session_recovery_in_order() {
    use toolkits::Tool;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let mut app = Workspace::with_context(egui::Context::default(), true);
    assert!(app.snapshot().layout.tools.is_none());
    app.toolkits.open_tool(Tool::Model);
    app.toolkits.open_tool(Tool::Designer);
    app.select_tool_tab(Tool::Model);
    matrust::session::SessionStore::new(&path).save(app.snapshot()).unwrap();
    let saved = matrust::session::Session::load(&path).unwrap();
    let mut restored = Workspace::with_context(egui::Context::default(), true);
    restored.restore_session(saved);
    assert_eq!(restored.toolkits.tabs().collect::<Vec<_>>(), [Tool::Model, Tool::Designer]);
    assert_eq!(restored.toolkits.selected_tool(), Some(Tool::Model));
    assert_eq!(restored.generic_save_target(), document_save::Pane::Model);
    // A session saved while the documents were shown restores them selected.
    app.select_document_layout(layout::PaneMode::Editor);
    matrust::session::SessionStore::new(&path).save(app.snapshot()).unwrap();
    let saved = matrust::session::Session::load(&path).unwrap();
    let mut restored = Workspace::with_context(egui::Context::default(), true);
    restored.restore_session(saved);
    assert_eq!(restored.toolkits.tabs().count(), 2);
    assert_eq!(restored.toolkits.selected_tool(), None);
}

#[test]
fn session_without_tool_tabs_restores_no_tools() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let app = Workspace::with_context(egui::Context::default(), true);
    matrust::session::SessionStore::new(&path).save(app.snapshot()).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("\"tools\""), "absent field is not written");
    let saved = matrust::session::Session::load(&path).unwrap();
    assert!(saved.layout.tools.is_none());
    let mut restored = Workspace::with_context(egui::Context::default(), true);
    restored.restore_session(saved);
    assert!(restored.toolkits.tabs().next().is_none());
}
