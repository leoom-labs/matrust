use super::*;

#[test]
fn model_clipboard_consumed_events_mark_recovery_and_undo_restores_snapshot() {
    for size in [egui::vec2(1440., 900.), egui::vec2(960., 650.)] {
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.toolkits.open_tool(toolkits::Tool::Model);
        let frame = |app: &mut Workspace, events| {
            crate::headless::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    events,
                    ..Default::default()
                },
                |root| {
                    let ctx = &root.ctx().clone();
                    app.ui(root);
                    if ctx.current_pass_index() == 0 {
                        ctx.request_discard("clipboard once per frame");
                    }
                },
            )
        };
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        let original = app.toolkits.model.clone();
        ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("model_diagram_keyboard")));
        app.recovery_dirty = false;
        let output = frame(&mut app, vec![egui::Event::Copy]);
        let text = output
            .platform_output
            .commands
            .into_iter()
            .find_map(|command| match command {
                egui::OutputCommand::CopyText(text) => Some(text),
                _ => None,
            })
            .expect("copy from focused model");
        assert!(!app.recovery_dirty, "copy changes no model state");
        frame(&mut app, vec![egui::Event::Paste(text)]);
        assert_eq!(app.toolkits.model.nodes.len(), original.nodes.len() + 1);
        assert!(
            app.recovery_dirty,
            "consumed Paste event must schedule recovery"
        );
        assert!(app.snapshot().model.same_storage(&app.toolkits.model));
        app.recovery_dirty = false;
        frame(
            &mut app,
            vec![egui::Event::Key {
                key: egui::Key::Z,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers {
                    ctrl: true,
                    command: true,
                    ..Default::default()
                },
            }],
        );
        assert!(app.toolkits.model.same_storage(&original));
        assert!(
            app.recovery_dirty,
            "consumed undo key must schedule recovery"
        );
        for repeat in [false, true] {
            app.recovery_dirty = false;
            frame(
                &mut app,
                vec![egui::Event::Key {
                    key: egui::Key::ArrowRight,
                    physical_key: None,
                    pressed: true,
                    repeat,
                    modifiers: egui::Modifiers {
                        ctrl: true,
                        command: true,
                        ..Default::default()
                    },
                }],
            );
            assert!(
                app.recovery_dirty,
                "each merged nudge must schedule recovery"
            );
        }
        app.recovery_dirty = false;
        let output = frame(&mut app, vec![egui::Event::Cut]);
        assert!(
            output
                .platform_output
                .commands
                .iter()
                .any(|command| matches!(command, egui::OutputCommand::CopyText(_)))
        );
        assert_eq!(app.toolkits.model.nodes.len(), original.nodes.len() - 1);
        assert!(
            app.recovery_dirty,
            "consumed Cut event must schedule recovery"
        );
        assert!(app.snapshot().model.same_storage(&app.toolkits.model));
        app.recovery_dirty = false;
        frame(&mut app, vec![egui::Event::Cut]);
        assert_eq!(app.toolkits.model.nodes.len(), original.nodes.len() - 1);
        assert!(
            !app.recovery_dirty,
            "repeated Cut with no selection does not change the model"
        );
    }
}
