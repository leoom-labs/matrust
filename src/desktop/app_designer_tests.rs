// Desktop App Designer tests (Engine 9b3). Included from src/desktop.rs.
// Drives the UI through user actions (clicks, text input, keyboard shortcuts),
// not by editing AppModel directly.

mod app_designer_tests {
    use super::*;
    use apps::{
        AppKind,
        app_designer::{DesignerView, Selection},
    };
    use matrust::{
        Engine,
        engine::app_designer_model::{AppComponentKind, GridSpanSpec},
    };
    use std::io::Write as _;

    const SCREEN: egui::Vec2 = egui::vec2(1400.0, 900.0);

    fn step_with(app: &mut Workspace, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
        app.receive();
        crate::headless::run(
            ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SCREEN)),
                events,
                ..Default::default()
            },
            |ui| app.ui(ui),
        )
    }

    fn step(app: &mut Workspace, ctx: &egui::Context) -> egui::FullOutput {
        step_with(app, ctx, Vec::new())
    }

    fn wait_idle(app: &mut Workspace, ctx: &egui::Context) {
        let deadline = Instant::now() + Duration::from_secs(15);
        step(app, ctx);
        while (app.busy || app.toolkits.apps.app_designer.saving) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
            step(app, ctx);
        }
        assert!(
            !app.busy && !app.toolkits.apps.app_designer.saving,
            "worker stayed busy: {}",
            app.status
        );
    }

    fn collect_texts(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::Shape::Text(t) => {
                let rect = t.galley.rect.translate(t.pos.to_vec2());
                out.push((t.galley.text().to_owned(), rect));
            }
            egui::Shape::Vec(v) => {
                for s in v {
                    collect_texts(s, out);
                }
            }
            _ => {}
        }
    }

    fn texts(out: &egui::FullOutput) -> Vec<(String, egui::Rect)> {
        let mut res = Vec::new();
        for c in &out.shapes {
            collect_texts(&c.shape, &mut res);
        }
        res
    }

    fn find_text_rect(out: &egui::FullOutput, needle: &str) -> egui::Rect {
        let all = texts(out);
        all.iter()
            .find(|(t, _)| t == needle)
            .or_else(|| all.iter().find(|(t, _)| t.contains(needle)))
            .map(|(_, r)| *r)
            .unwrap_or_else(|| {
                let sample: Vec<&str> = all.iter().map(|(t, _)| t.as_str()).collect();
                panic!("could not find text '{needle}' in frame; texts: {sample:?}");
            })
    }

    fn click_pos(app: &mut Workspace, ctx: &egui::Context, pos: egui::Pos2) -> egui::FullOutput {
        step_with(
            app,
            ctx,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        step_with(
            app,
            ctx,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        )
    }

    fn click_text(app: &mut Workspace, ctx: &egui::Context, needle: &str) -> egui::FullOutput {
        let out = step(app, ctx);
        let rect = find_text_rect(&out, needle);
        click_pos(app, ctx, rect.center())
    }

    fn drag_between(
        app: &mut Workspace,
        ctx: &egui::Context,
        from: egui::Pos2,
        to: egui::Pos2,
    ) -> egui::FullOutput {
        step_with(
            app,
            ctx,
            vec![
                egui::Event::PointerMoved(from),
                egui::Event::PointerButton {
                    pos: from,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let mid = egui::pos2((from.x + to.x) * 0.5, (from.y + to.y) * 0.5);
        step_with(app, ctx, vec![egui::Event::PointerMoved(mid)]);
        step_with(app, ctx, vec![egui::Event::PointerMoved(to)]);
        step_with(
            app,
            ctx,
            vec![
                egui::Event::PointerMoved(to),
                egui::Event::PointerButton {
                    pos: to,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        )
    }

    fn focus_id_and_replace_text(
        app: &mut Workspace,
        ctx: &egui::Context,
        id: egui::Id,
        text: &str,
        press_enter: bool,
    ) -> egui::FullOutput {
        ctx.memory_mut(|m| m.request_focus(id));
        step(app, ctx);
        let mut events = vec![
            egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            },
            egui::Event::Text(text.to_owned()),
        ];
        if press_enter {
            events.push(egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
        }
        step_with(app, ctx, events)
    }

    fn open_designer(app: &mut Workspace, ctx: &egui::Context) {
        app.submit(worker::Job::ScriptInFolder(
            "appdesigner".into(),
            app.folder.clone(),
        ));
        wait_idle(app, ctx);
        assert!(app.toolkits.apps.is_open(AppKind::AppDesigner));
        step(app, ctx);
    }

    #[test]
    fn app_designer_command_launch_help_and_readonly_files() {
        // 1. CLI/headless engine rejects appdesigner clearly.
        let err = Engine::default().execute("appdesigner").unwrap_err();
        assert!(err.ends_with("appdesigner requires the desktop"), "{err}");
        assert!(Engine::default().execute("x = appdesigner()").is_err());
        assert!(Engine::default().execute("appdesigner('a','b')").is_err());

        // 2. Help page exists and documents scope.
        let help = Engine::default()
            .execute("help appdesigner")
            .unwrap()
            .output
            .join("\n");
        assert!(help.contains("appdesigner"), "{help}");
        assert!(help.contains(".mlapp"), "{help}");
        assert!(help.contains("uihtml"), "{help}");

        // 3. Desktop `appdesigner` opens a new app named app1.
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        open_designer(&mut app, &ctx);
        assert_eq!(app.toolkits.apps.app_designer.model.class_name, "app1");
        assert!(!app.toolkits.apps.app_designer.is_read_only());
        assert_eq!(
            AppKind::from_tab(matrust::session::ToolTab::AppDesigner),
            Some(AppKind::AppDesigner)
        );

        // 4. Hand-written classdef opens read-only with banner and editing disabled.
        let counter_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/app_base/CounterApp.m");
        app.submit(worker::Job::ScriptInFolder(
            format!("appdesigner('{}')", counter_path.display()),
            app.folder.clone(),
        ));
        wait_idle(&mut app, &ctx);
        assert!(app.toolkits.apps.app_designer.is_read_only());
        assert_eq!(app.toolkits.apps.app_designer.view, DesignerView::Code);
        let out = step(&mut app, &ctx);
        let frame_texts: Vec<String> = texts(&out).into_iter().map(|(t, _)| t).collect();
        assert!(
            frame_texts.iter().any(|t| t.contains("Read-only:")),
            "expected read-only banner, got: {frame_texts:?}"
        );

        // 5. Simple .mlapp opens read-only as code with the model's message.
        let dir = tempfile::tempdir().unwrap();
        let mlapp_path = dir.path().join("Sample.mlapp");
        {
            let file = std::fs::File::create(&mlapp_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default();
            zip.start_file("matlab/document.xml", opts).unwrap();
            zip.write_all(
                br#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>classdef Sample &lt; matlab.apps.AppBase</w:t></w:r></w:p>
    <w:p><w:r><w:t>end</w:t></w:r></w:p>
  </w:body>
</w:document>"#,
            )
            .unwrap();
            zip.finish().unwrap();
        }
        app.submit(worker::Job::ScriptInFolder(
            format!("appdesigner('{}')", mlapp_path.display()),
            dir.path().to_path_buf(),
        ));
        wait_idle(&mut app, &ctx);
        assert!(app.toolkits.apps.app_designer.is_read_only());
        let ro = app.toolkits.apps.app_designer.read_only.as_ref().unwrap();
        assert!(ro.code.contains("classdef Sample < matlab.apps.AppBase"));
        assert!(ro.reason.contains(".mlapp"), "{}", ro.reason);
    }

    #[test]
    fn app_designer_ugly_states_and_unsaved_close_confirmation() {
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        open_designer(&mut app, &ctx);

        // Place Button at (1, 1) and Label at (1, 2) via palette + cell clicks.
        click_text(&mut app, &ctx, "Button");
        click_text(&mut app, &ctx, "(1, 1)");
        assert_eq!(app.toolkits.apps.app_designer.model.components.len(), 1);

        click_text(&mut app, &ctx, "Label");
        click_text(&mut app, &ctx, "(1, 2)");
        assert_eq!(app.toolkits.apps.app_designer.model.components.len(), 2);

        // Ugly state 1: Invalid and duplicate name in inspector are refused and field shows why.
        assert_eq!(
            app.toolkits.apps.app_designer.selection,
            Selection::Component("Label".into())
        );
        focus_id_and_replace_text(
            &mut app,
            &ctx,
            egui::Id::new("app_designer_comp_name"),
            "123bad",
            true,
        );
        step(&mut app, &ctx);
        assert!(app.toolkits.apps.app_designer.model.component("Label").is_some());
        let out = step(&mut app, &ctx);
        let t1: Vec<String> = texts(&out).into_iter().map(|(t, _)| t).collect();
        let bad_name_hits = t1
            .iter()
            .filter(|t| t.contains("not a valid MATLAB identifier"))
            .count();
        assert_eq!(
            bad_name_hits, 1,
            "expected invalid identifier error exactly once (one message place), got {bad_name_hits}: {t1:?}"
        );
        assert!(
            !t1.iter().any(|t| t == "123bad"),
            "refused invalid name should be reverted in Name field, got: {t1:?}"
        );
        assert!(
            !t1.iter().any(|t| t == "No validation problems."),
            "must not show 'No validation problems.' next to an error: {t1:?}"
        );

        focus_id_and_replace_text(
            &mut app,
            &ctx,
            egui::Id::new("app_designer_comp_name"),
            "Button",
            true,
        );
        step(&mut app, &ctx);
        assert!(app.toolkits.apps.app_designer.model.component("Label").is_some());
        let out = step(&mut app, &ctx);
        let t2: Vec<String> = texts(&out).into_iter().map(|(t, _)| t).collect();
        assert_eq!(
            t2.iter().filter(|t| t.contains("Duplicate name")).count(),
            1,
            "expected duplicate name error once in Output panel, got: {t2:?}"
        );

        // Ugly state 2: Two components in one cell (refused with human-readable coordinates).
        focus_id_and_replace_text(
            &mut app,
            &ctx,
            egui::Id::new("app_designer_grid_col"),
            "1",
            true,
        );
        step(&mut app, &ctx);
        assert_eq!(app.toolkits.apps.app_designer.model.components.len(), 2);
        let err_msg = app
            .toolkits
            .apps
            .app_designer
            .last_error
            .clone()
            .unwrap_or_default();
        assert_eq!(
            err_msg, "Grid cell (1, 1) is already used by Button.",
            "expected human-readable cell conflict error without Rust debug text"
        );
        let out = step(&mut app, &ctx);
        let t_cell: Vec<String> = texts(&out).into_iter().map(|(t, _)| t).collect();
        assert_eq!(
            t_cell.iter().filter(|t| t.as_str() == err_msg).count(),
            1,
            "cell conflict error must appear once (Output panel only): {t_cell:?}"
        );
        assert!(
            !t_cell.iter().any(|t| t == "No validation problems."),
            "must not show 'No validation problems.' while an error is shown: {t_cell:?}"
        );

        // Ugly state 3: Callback for a deleted component is listed as a problem.
        // Select Button, add ButtonPushedFcn, then press Delete.
        click_text(&mut app, &ctx, "Button · Button");
        click_text(&mut app, &ctx, "+ ButtonPushedFcn");
        assert_eq!(app.toolkits.apps.app_designer.model.callbacks.len(), 1);
        click_text(&mut app, &ctx, "Delete Component");
        assert_eq!(app.toolkits.apps.app_designer.model.components.len(), 1);
        let problems = app.toolkits.apps.app_designer.problems();
        assert!(
            problems.iter().any(|p| p.code == "missing_callback_component"),
            "expected missing_callback_component problem, got: {problems:?}"
        );

        // Ugly state 4: Run with problems is blocked and says which problem.
        click_text(&mut app, &ctx, "Run");
        assert!(!app.busy, "Run must be blocked when problems exist");
        let run_err = app
            .toolkits
            .apps
            .app_designer
            .last_error
            .clone()
            .unwrap_or_default();
        assert!(
            run_err.contains("Cannot run:") && run_err.contains("ButtonPushed"),
            "expected blocked Run error naming callback problem, got: {run_err}"
        );

        // Ugly state 5: Unsaved changes on close asks before closing.
        assert!(app.toolkits.apps.app_designer.is_dirty());
        app.close_tool_tab(toolkits::Tool::App(AppKind::AppDesigner));
        assert!(
            app.toolkits.apps.is_open(AppKind::AppDesigner),
            "dirty App Designer should not close without confirmation"
        );
        assert!(app.toolkits.apps.app_designer.close_confirm);
        let out = step(&mut app, &ctx);
        let t_close: Vec<String> = texts(&out).into_iter().map(|(t, _)| t).collect();
        assert!(
            t_close.iter().any(|t| t.contains("Unsaved changes")),
            "expected unsaved changes prompt, got: {t_close:?}"
        );
        click_text(&mut app, &ctx, "Cancel");
        assert!(!app.toolkits.apps.app_designer.close_confirm);
        assert!(app.toolkits.apps.is_open(AppKind::AppDesigner));

        // Now confirm Discard & Close.
        app.close_tool_tab(toolkits::Tool::App(AppKind::AppDesigner));
        step(&mut app, &ctx);
        click_text(&mut app, &ctx, "Discard & Close");
        step(&mut app, &ctx);
        assert!(!app.toolkits.apps.is_open(AppKind::AppDesigner));
    }

    #[test]
    fn app_designer_drag_moves_component_between_cells() {
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        open_designer(&mut app, &ctx);

        click_text(&mut app, &ctx, "Button");
        click_text(&mut app, &ctx, "(1, 1)");
        let out = step(&mut app, &ctx);
        let from = texts(&out)
            .into_iter()
            .rfind(|(t, _)| t == "Button")
            .map(|(_, r)| r.center())
            .expect("canvas Button");
        let to = find_text_rect(&out, "(2, 2)").center();
        drag_between(&mut app, &ctx, from, to);

        let btn = app
            .toolkits
            .apps
            .app_designer
            .model
            .component("Button")
            .unwrap();
        assert_eq!(btn.grid_row, Some(GridSpanSpec::Single(2)));
        assert_eq!(btn.grid_column, Some(GridSpanSpec::Single(2)));
    }

    #[test]
    fn app_designer_done_check_build_save_run_fire_callbacks_and_reopen() {
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        open_designer(&mut app, &ctx);

        // 1. Expand default 2x2 grid to 3x2 via "+ Row".
        click_text(&mut app, &ctx, "+ Row");
        assert_eq!(
            app.toolkits.apps.app_designer.model.grid.as_ref().unwrap().rows(),
            3
        );
        assert_eq!(
            app.toolkits.apps.app_designer.model.grid.as_ref().unwrap().cols(),
            2
        );

        // 2. Place 5 components: Label (1,1), Button (1,2), NumericEditField (2,1),
        //    DropDown (2,2), Axes (3,1) spanning columns 1:2.
        click_text(&mut app, &ctx, "Label");
        click_text(&mut app, &ctx, "(1, 1)");

        click_text(&mut app, &ctx, "Button");
        click_text(&mut app, &ctx, "(1, 2)");

        click_text(&mut app, &ctx, "Edit Field (Numeric)");
        click_text(&mut app, &ctx, "(2, 1)");

        click_text(&mut app, &ctx, "Drop Down");
        click_text(&mut app, &ctx, "(2, 2)");

        click_text(&mut app, &ctx, "Axes");
        click_text(&mut app, &ctx, "(3, 1)");

        // Span UIAxes across columns 1:2 via the inspector Col field.
        focus_id_and_replace_text(
            &mut app,
            &ctx,
            egui::Id::new("app_designer_grid_col"),
            "1:2",
            true,
        );
        step(&mut app, &ctx);

        assert_eq!(app.toolkits.apps.app_designer.model.components.len(), 5);
        assert_eq!(
            app.toolkits.apps.app_designer.model.component("UIAxes").unwrap().kind,
            AppComponentKind::Axes
        );
        assert_eq!(
            app.toolkits.apps.app_designer.model.component("UIAxes").unwrap().grid_column,
            Some(GridSpanSpec::Range(1, 2))
        );

        // 3. Add 2 callbacks via inspector: ButtonPushedFcn and ValueChangedFcn.
        click_text(&mut app, &ctx, "Button · Button");
        click_text(&mut app, &ctx, "+ ButtonPushedFcn");

        click_text(&mut app, &ctx, "NumericEditField · Edit Field (Numeric)");
        click_text(&mut app, &ctx, "+ ValueChangedFcn");
        assert_eq!(app.toolkits.apps.app_designer.model.callbacks.len(), 2);

        // 4. Switch to Code View and write both callback bodies in the editable text areas.
        click_text(&mut app, &ctx, "Code View");
        assert_eq!(app.toolkits.apps.app_designer.view, DesignerView::Code);

        focus_id_and_replace_text(
            &mut app,
            &ctx,
            egui::Id::new(("app_designer_body", "ButtonButtonPushed")),
            "app.Label.Text = sprintf('%s:%.1f', app.DropDown.Value, app.NumericEditField.Value);",
            false,
        );
        focus_id_and_replace_text(
            &mut app,
            &ctx,
            egui::Id::new(("app_designer_body", "NumericEditFieldValueChanged")),
            "x = [0 1 2 3]; y = event.Value * x; plot(app.UIAxes, x, y);",
            false,
        );
        step(&mut app, &ctx);
        assert!(
            !app.toolkits.apps.app_designer.model.callbacks[0].body.is_empty(),
            "ButtonPushed body should be edited in Code View, got: {:?}",
            app.toolkits.apps.app_designer.model.callbacks
        );
        assert!(
            !app.toolkits.apps.app_designer.model.callbacks[1].body.is_empty(),
            "NumericEditFieldValueChanged body should be edited in Code View, got: {:?}",
            app.toolkits.apps.app_designer.model.callbacks
        );

        // 5. Save to a temp dir through the desktop save dialog path, then Run.
        let dir = tempfile::tempdir().unwrap();
        let saved_path = dir.path().join("DoneCheckApp.m");
        app.toolkits.apps.test_save_path = Some(saved_path.clone());
        click_text(&mut app, &ctx, "Save");
        wait_idle(&mut app, &ctx);
        assert!(saved_path.exists(), "saved .m file should exist");
        assert!(!app.toolkits.apps.app_designer.is_dirty());
        let saved_model = app.toolkits.apps.app_designer.model.clone();
        assert_eq!(saved_model.class_name, "DoneCheckApp");

        // 6. Click Run: launches DoneCheckApp on the engine so its uifigure opens as a 9a window.
        click_text(&mut app, &ctx, "Run");
        wait_idle(&mut app, &ctx);
        assert!(
            app.ui_windows.window_count() >= 1,
            "expected live 9a uifigure window after Run; status: {}",
            app.status
        );

        // 7. Fire both callbacks with the 9a test trigger (mrUiFire) and check Label.Text + plotted line.
        app.submit(worker::Job::ScriptInFolder(
            "a = DoneCheckApp; \
             mrUiFire(a.NumericEditField, 'type', 4); \
             mrUiFire(a.Button, 'press'); \
             ln = a.UIAxes.Children; \
             res_lbl = a.Label.Text; \
             res_lbl_ok = double(strcmp(a.Label.Text, 'Option 1:4.0')); \
             res_nline = numel(ln); \
             res_yend = ln.YData(end); \
             delete(a);"
                .into(),
            dir.path().to_path_buf(),
        ));
        wait_idle(&mut app, &ctx);
        let lbl_ok = match app.variables.get("res_lbl_ok") {
            Some(matrust::Array::Numeric(m)) => m.data()[0],
            other => panic!("expected numeric res_lbl_ok, got {other:?}"),
        };
        assert_eq!(
            lbl_ok,
            1.0,
            "expected Label.Text == 'Option 1:4.0', got: {:?}, code:\n{}",
            app.variables.get("res_lbl"),
            std::fs::read_to_string(&saved_path).unwrap()
        );
        let nline = match app.variables.get("res_nline") {
            Some(matrust::Array::Numeric(m)) => m.data()[0],
            other => panic!("expected numeric res_nline, got {other:?}"),
        };
        assert_eq!(nline, 1.0);
        let yend = match app.variables.get("res_yend") {
            Some(matrust::Array::Numeric(m)) => m.data()[0],
            other => panic!("expected numeric res_yend, got {other:?}"),
        };
        assert!((yend - 12.0).abs() < 1e-12, "expected 12.0, got {yend}");

        // 8. Reopen the saved file and get the exact same model.
        app.submit(worker::Job::ScriptInFolder(
            format!("appdesigner('{}')", saved_path.display()),
            dir.path().to_path_buf(),
        ));
        wait_idle(&mut app, &ctx);
        assert!(!app.toolkits.apps.app_designer.is_read_only());
        assert_eq!(app.toolkits.apps.app_designer.model, saved_model);
    }

    fn collect_rects(shape: &egui::Shape, out: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Rect(r) => out.push(r.rect),
            egui::Shape::Vec(v) => {
                for s in v {
                    collect_rects(s, out);
                }
            }
            _ => {}
        }
    }

    fn enclosing_button_rect(out: &egui::FullOutput, text_rect: egui::Rect) -> Option<egui::Rect> {
        let mut rects = Vec::new();
        for c in &out.shapes {
            collect_rects(&c.shape, &mut rects);
        }
        rects
            .into_iter()
            .filter(|r| {
                r.contains_rect(text_rect) && r.width() < 200.0 && r.height() < 60.0
            })
            .min_by(|a, b| a.area().total_cmp(&b.area()))
    }

    fn run_apps_window_at(
        app: &mut Workspace,
        ctx: &egui::Context,
        size: egui::Vec2,
    ) -> egui::FullOutput {
        use std::{cell::RefCell, rc::Rc};
        let captured: Rc<RefCell<Option<egui::FullOutput>>> = Rc::new(RefCell::new(None));
        let cap_clone = captured.clone();
        ctx.set_embed_viewports(false);
        egui::Context::set_immediate_viewport_renderer(move |ctx, mut viewport| {
            let mut input = egui::RawInput {
                viewport_id: viewport.ids.this,
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            };
            input.viewports.entry(viewport.ids.this).or_default().parent =
                Some(egui::ViewportId::ROOT);
            let out = crate::headless::run(ctx, input, |ui| (viewport.viewport_ui_cb)(ui));
            *cap_clone.borrow_mut() = Some(out);
        });
        for _ in 0..3 {
            let _ = crate::headless::run(
                ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ui| app.ui(ui),
            );
        }
        captured
            .borrow_mut()
            .take()
            .expect("expected immediate apps viewport output")
    }

    fn texts_with_clip(out: &egui::FullOutput) -> Vec<(String, egui::Rect, egui::Rect)> {
        let mut res = Vec::new();
        for clipped in &out.shapes {
            let mut local = Vec::new();
            collect_texts(&clipped.shape, &mut local);
            for (t, r) in local {
                res.push((t, r, clipped.clip_rect));
            }
        }
        res
    }

    fn run_designer_at(
        designer: &mut apps::app_designer::AppDesigner,
        ctx: &egui::Context,
        size: egui::Vec2,
    ) -> egui::FullOutput {
        let mut action = None;
        let mut run_once = |d: &mut apps::app_designer::AppDesigner| {
            crate::headless::run(
                ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        d.show(ui, false, &mut action);
                    });
                },
            )
        };
        // Two frames so egui panel/grid sizes settle.
        let _ = run_once(designer);
        run_once(designer)
    }

    #[test]
    fn app_designer_layout_rects_wide_and_narrow() {
        let ctx = egui::Context::default();
        let mut designer = apps::app_designer::AppDesigner::new_app();
        // Build a populated 4x3 layout matching the ui-review screenshots (/tmp/ad2.png, /tmp/ad12.png).
        {
            let grid = designer.model.grid.as_mut().unwrap();
            while grid.rows() < 4 {
                grid.row_height
                    .push(matrust::engine::app_designer_model::AppGridTrack::Weight(1.0));
            }
            while grid.cols() < 3 {
                grid.column_width
                    .push(matrust::engine::app_designer_model::AppGridTrack::Weight(1.0));
            }
        }
        for (kind, r, c) in [
            (AppComponentKind::Button, 1, 1),
            (AppComponentKind::Label, 1, 2),
            (AppComponentKind::NumericEditField, 2, 1),
            (AppComponentKind::DropDown, 2, 2),
            (AppComponentKind::Axes, 3, 1),
            (AppComponentKind::CheckBox, 3, 2),
            (AppComponentKind::Slider, 3, 3),
            (AppComponentKind::Switch, 4, 1),
            (AppComponentKind::Axes, 4, 2),
        ] {
            let name = designer.model.next_component_name(kind);
            let comp = matrust::engine::app_designer_model::AppComponent::new(name, kind)
                .in_grid(r, c);
            designer.model.add_component(comp).unwrap();
        }
        designer.selection = Selection::Component("DropDown".into());
        designer.sync_drafts(true);

        // --- 1. Wide (1400x900): three columns, zero overlap, zero horizontal clipping ---
        let wide = egui::vec2(1400.0, 900.0);
        let out_wide = run_designer_at(&mut designer, &ctx, wide);
        let wide_texts = texts_with_clip(&out_wide);

        // Palette labels must fit inside their clip rects (no "State Buttor" / "Edit Field (Te").
        for needle in [
            "Button",
            "State Button",
            "Edit Field (Text)",
            "Edit Field (Numeric)",
            "Radio Button",
            "Table",
            "Panel",
            "Tab",
        ] {
            let (_, rect, clip) = wide_texts
                .iter()
                .find(|(t, _, _)| t == needle)
                .unwrap_or_else(|| panic!("wide: palette label '{needle}' missing"));
            assert!(
                rect.left() >= clip.left() - 1.0 && rect.right() <= clip.right() + 1.0,
                "wide: palette label '{needle}' clipped horizontally: rect={rect:?}, clip={clip:?}"
            );
        }

        // Component list lines & Delete Component must fit inside the left panel's clip rect.
        for needle in [
            "NumericEditField · Edit Field (Numeric) (2,1)",
            "DropDown · Drop Down (2,2)",
            "Delete Component",
        ] {
            let (_, rect, clip) = wide_texts
                .iter()
                .find(|(t, _, _)| t == needle)
                .unwrap_or_else(|| panic!("wide: component list '{needle}' missing"));
            assert!(
                rect.left() >= clip.left() - 1.0 && rect.right() <= clip.right() + 1.0,
                "wide: component list '{needle}' clipped horizontally: rect={rect:?}, clip={clip:?}"
            );
        }

        // Inspector labels and controls must fit inside their clip rect and the 1400px window.
        let insp_hdr = wide_texts
            .iter()
            .find(|(t, _, _)| t == "INSPECTOR")
            .expect("wide: INSPECTOR header");
        for needle in ["INSPECTOR", "Name", "Rename", "Row", "Col", "Set", "Items (comma-separated)", "Value", "CALLBACKS"] {
            let (_, rect, clip) = wide_texts
                .iter()
                .find(|(t, _, _)| t == needle)
                .unwrap_or_else(|| panic!("wide: inspector text '{needle}' missing"));
            assert!(
                rect.left() >= clip.left() - 1.0
                    && rect.right() <= clip.right() + 1.0
                    && rect.right() <= wide.x + 1.0,
                "wide: inspector '{needle}' clipped or outside window: rect={rect:?}, clip={clip:?}"
            );
        }

        // Canvas figure title & cells must sit strictly between left panel and inspector.
        let (_, fig_title_rect, canvas_clip) = wide_texts
            .iter()
            .find(|(t, _, _)| t == "MATLAB App")
            .expect("wide: MATLAB App figure title");
        assert!(
            canvas_clip.right() <= insp_hdr.2.left() + 1.0,
            "wide: canvas clip ({}) must not overlap inspector clip ({})",
            canvas_clip.right(),
            insp_hdr.2.left()
        );
        assert!(
            fig_title_rect.left() >= canvas_clip.left() - 1.0,
            "wide: figure title clipped on left"
        );

        // Placing hint must not shift the palette vertically.
        let btn_y_before = wide_texts
            .iter()
            .find(|(t, _, _)| t == "Button")
            .expect("palette Button before")
            .1
            .top();
        designer.placing = Some(AppComponentKind::Axes);
        let out_placing = run_designer_at(&mut designer, &ctx, wide);
        let placing_texts = texts_with_clip(&out_placing);
        let btn_y_after = placing_texts
            .iter()
            .find(|(t, _, _)| t == "Button")
            .expect("palette Button after")
            .1
            .top();
        assert!(
            (btn_y_after - btn_y_before).abs() < 0.5,
            "Placing hint shifted palette Button from y={btn_y_before} to y={btn_y_after}"
        );
        assert!(
            placing_texts
                .iter()
                .any(|(t, _, _)| t == "Placing: Axes (click an empty cell, Esc to cancel)"),
            "expected Placing hint with Esc instruction in canvas header"
        );
        designer.placing = None;

        // Wide: Rename button frame itself (not just text) must sit fully inside inspector & window.
        let (_, rename_text_w, rename_clip_w) = wide_texts
            .iter()
            .find(|(t, _, _)| t == "Rename")
            .expect("wide: Rename text");
        let rename_btn_w = enclosing_button_rect(&out_wide, *rename_text_w)
            .expect("wide: Rename button rect");
        assert!(
            rename_btn_w.right() <= rename_clip_w.right() + 1.0
                && rename_btn_w.right() <= wide.x - 1.0,
            "wide: Rename button cut at right edge: btn={rename_btn_w:?}, clip={rename_clip_w:?}"
        );

        // Wide: Output panel must sit strictly below left/canvas/inspector panels and never clip header or notices.
        designer.set_notice("Saved /tmp/demo_app.m");
        let out_wide_notice = run_designer_at(&mut designer, &ctx, wide);
        let wn_texts = texts_with_clip(&out_wide_notice);
        let (_, out_hdr_w, out_clip_w) = wn_texts
            .iter()
            .find(|(t, _, _)| t == "OUTPUT · Problems (0)")
            .expect("wide: OUTPUT header");
        let (_, notice_w, notice_clip_w) = wn_texts
            .iter()
            .find(|(t, _, _)| t == "Saved /tmp/demo_app.m")
            .expect("wide: notice in Output panel");
        let (_, _, left_clip_w) = wn_texts
            .iter()
            .find(|(t, _, _)| t == "PALETTE")
            .expect("wide: PALETTE");
        let (_, _, insp_clip_w) = wn_texts
            .iter()
            .find(|(t, _, _)| t == "INSPECTOR")
            .expect("wide: INSPECTOR");
        let (_, _, canvas_clip_w) = wn_texts
            .iter()
            .find(|(t, _, _)| t == "MATLAB App")
            .expect("wide: canvas");
        assert!(
            out_clip_w.top() >= left_clip_w.bottom() - 1.0
                && out_clip_w.top() >= insp_clip_w.bottom() - 1.0
                && out_clip_w.top() >= canvas_clip_w.bottom() - 1.0,
            "wide: Output panel ({out_clip_w:?}) overlapped by upper panels (left={left_clip_w:?}, insp={insp_clip_w:?}, canvas={canvas_clip_w:?})"
        );
        assert!(
            out_hdr_w.top() >= out_clip_w.top() - 1.0
                && out_hdr_w.bottom() <= out_clip_w.bottom() + 1.0
                && notice_w.top() >= notice_clip_w.top() - 1.0
                && notice_w.bottom() <= notice_clip_w.bottom() + 1.0
                && notice_w.bottom() <= wide.y + 1.0,
            "wide: Output header or notice clipped: hdr={out_hdr_w:?}, notice={notice_w:?}, clip={out_clip_w:?}"
        );
        assert!(
            out_clip_w.height() >= 80.0,
            "wide: Output panel should reserve height for header + >=2 lines, got {}",
            out_clip_w.height()
        );

        // Wide: While an error is shown, "No validation problems." is NOT shown and error is unclipped.
        designer.set_error("Could not open a save dialog: portal unavailable.");
        let out_wide_err = run_designer_at(&mut designer, &ctx, wide);
        let we_texts = texts_with_clip(&out_wide_err);
        let (_, err_w, err_clip_w) = we_texts
            .iter()
            .find(|(t, _, _)| t.contains("Could not open a save dialog:"))
            .expect("wide: error in Output panel");
        assert!(
            err_w.top() >= err_clip_w.top() - 1.0 && err_w.bottom() <= err_clip_w.bottom() + 1.0,
            "wide: error text clipped: rect={err_w:?}, clip={err_clip_w:?}"
        );
        assert!(
            !we_texts.iter().any(|(t, _, _)| t == "No validation problems."),
            "wide: must not show 'No validation problems.' while an error is shown"
        );
        designer.clear_status();

        // --- 2. Narrow (760x900): 1-column palette, inspector strictly below canvas ---
        let narrow = egui::vec2(760.0, 900.0);
        let out_narrow = run_designer_at(&mut designer, &ctx, narrow);
        let narrow_texts = texts_with_clip(&out_narrow);

        // Every palette label fits in 1 column with no horizontal clipping ("Tal", "Pai").
        for needle in [
            "Button",
            "State Button",
            "Edit Field (Text)",
            "Edit Field (Numeric)",
            "Table",
            "Panel",
            "Tab",
            "Button Group",
        ] {
            let (_, rect, clip) = narrow_texts
                .iter()
                .find(|(t, _, _)| t == needle)
                .unwrap_or_else(|| panic!("narrow: palette label '{needle}' missing"));
            assert!(
                rect.left() >= clip.left() - 1.0 && rect.right() <= clip.right() + 1.0,
                "narrow: palette label '{needle}' clipped: rect={rect:?}, clip={clip:?}"
            );
        }

        // Inspector must be placed strictly BELOW the canvas's bottom row (UIAxes2 at row 4).
        let (_, uiaxes2_rect, _) = narrow_texts
            .iter()
            .find(|(t, _, _)| t == "UIAxes2")
            .expect("narrow: UIAxes2 in bottom canvas row");
        let (_, insp_narrow_rect, insp_narrow_clip) = narrow_texts
            .iter()
            .find(|(t, _, _)| t == "INSPECTOR")
            .expect("narrow: INSPECTOR header");
        assert!(
            insp_narrow_rect.top() >= uiaxes2_rect.bottom(),
            "narrow: INSPECTOR top ({}) overlaps bottom canvas row UIAxes2 bottom ({})",
            insp_narrow_rect.top(),
            uiaxes2_rect.bottom()
        );

        // Rename button and Name/Row/Col/Set fit inside the 760px window without clipping.
        for needle in ["INSPECTOR", "Name", "Rename", "Row", "Col", "Set"] {
            let (_, rect, clip) = narrow_texts
                .iter()
                .find(|(t, _, _)| t == needle)
                .unwrap_or_else(|| panic!("narrow: inspector '{needle}' missing"));
            assert!(
                rect.left() >= clip.left() - 1.0
                    && rect.right() <= clip.right() + 1.0
                    && rect.right() <= narrow.x + 1.0,
                "narrow: inspector '{needle}' clipped or outside 760px window: rect={rect:?}, clip={clip:?}, insp_clip={insp_narrow_clip:?}"
            );
        }
        let (_, rename_text_n, rename_clip_n) = narrow_texts
            .iter()
            .find(|(t, _, _)| t == "Rename")
            .expect("narrow: Rename text");
        let rename_btn_n = enclosing_button_rect(&out_narrow, *rename_text_n)
            .expect("narrow: Rename button rect");
        assert!(
            rename_btn_n.right() <= rename_clip_n.right() + 1.0
                && rename_btn_n.right() <= narrow.x - 1.0,
            "narrow: Rename button cut at right edge: btn={rename_btn_n:?}, clip={rename_clip_n:?}"
        );

        // Narrow: Output panel unclipped and strictly below upper panels, notice & error visible.
        designer.set_notice("Saved /tmp/demo_app.m");
        let out_narrow_notice = run_designer_at(&mut designer, &ctx, narrow);
        let nn_texts = texts_with_clip(&out_narrow_notice);
        let (_, out_hdr_n, out_clip_n) = nn_texts
            .iter()
            .find(|(t, _, _)| t == "OUTPUT · Problems (0)")
            .expect("narrow: OUTPUT header");
        let (_, notice_n, notice_clip_n) = nn_texts
            .iter()
            .find(|(t, _, _)| t == "Saved /tmp/demo_app.m")
            .expect("narrow: notice in Output panel");
        let (_, _, left_clip_n) = nn_texts
            .iter()
            .find(|(t, _, _)| t == "PALETTE")
            .expect("narrow: PALETTE");
        assert!(
            out_clip_n.top() >= left_clip_n.bottom() - 1.0,
            "narrow: Output panel ({out_clip_n:?}) overlapped by left panel ({left_clip_n:?})"
        );
        assert!(
            out_hdr_n.top() >= out_clip_n.top() - 1.0
                && out_hdr_n.bottom() <= out_clip_n.bottom() + 1.0
                && notice_n.top() >= notice_clip_n.top() - 1.0
                && notice_n.bottom() <= notice_clip_n.bottom() + 1.0
                && notice_n.bottom() <= narrow.y + 1.0,
            "narrow: Output header or notice clipped: hdr={out_hdr_n:?}, notice={notice_n:?}, clip={out_clip_n:?}"
        );
        designer.set_error("Could not open a save dialog: portal unavailable.");
        let out_narrow_err = run_designer_at(&mut designer, &ctx, narrow);
        let ne_texts = texts_with_clip(&out_narrow_err);
        assert!(
            ne_texts
                .iter()
                .any(|(t, r, c)| t.contains("Could not open a save dialog:")
                    && r.top() >= c.top() - 1.0
                    && r.bottom() <= c.bottom() + 1.0),
            "narrow: error text missing or clipped"
        );
        assert!(
            !ne_texts.iter().any(|(t, _, _)| t == "No validation problems."),
            "narrow: must not show 'No validation problems.' while an error is shown"
        );

        // --- 3. Separate Apps Window: no empty band under tab buttons (within 8 px) at both widths ---
        for (label, size) in [("wide", wide), ("narrow", narrow)] {
            let win_ctx = egui::Context::default();
            let mut app = Workspace::with_context(win_ctx.clone(), true);
            app.layout.window_layout = Some(matrust::session::WindowLayout::Separate);
            app.open_app(toolkits::Tool::App(AppKind::AppDesigner));
            let win_out = run_apps_window_at(&mut app, &win_ctx, size);
            let wt = texts_with_clip(&win_out);
            let (_, tab_rect, tab_clip) = wt
                .iter()
                .find(|(t, _, _)| t.starts_with("App Designer"))
                .unwrap_or_else(|| panic!("{label}: App Designer tab missing"));
            let (_, new_text_rect, toolbar_clip) = wt
                .iter()
                .find(|(t, _, _)| t == "New")
                .unwrap_or_else(|| panic!("{label}: New toolbar button missing"));
            let new_btn_rect = enclosing_button_rect(&win_out, *new_text_rect)
                .unwrap_or_else(|| panic!("{label}: New toolbar button rect missing"));
            let tab_bottom = tab_clip.bottom().max(tab_rect.bottom());
            let panel_gap = toolbar_clip.top() - tab_bottom;
            let btn_gap = new_btn_rect.top() - tab_bottom;
            assert!(
                (-1.0..=8.0).contains(&panel_gap) && (-1.0..=8.0).contains(&btn_gap),
                "{label}: empty band between tab row bottom ({tab_bottom}) and first content row (panel top={}, button top={}) is panel={panel_gap}px / btn={btn_gap}px (must be <= 8 px)",
                toolbar_clip.top(),
                new_btn_rect.top()
            );
        }
    }

    #[test]
    fn app_designer_placing_mode_occupied_cell_and_esc() {
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        open_designer(&mut app, &ctx);

        // Place Button at (1, 1) and Label at (1, 2).
        click_text(&mut app, &ctx, "Button");
        click_text(&mut app, &ctx, "(1, 1)");
        click_text(&mut app, &ctx, "Label");
        click_text(&mut app, &ctx, "(1, 2)");
        assert_eq!(app.toolkits.apps.app_designer.model.components.len(), 2);
        assert_eq!(
            app.toolkits.apps.app_designer.selection,
            Selection::Component("Label".into())
        );

        // Enter placing mode for Check Box; verify canvas header hint includes Esc instruction.
        click_text(&mut app, &ctx, "Check Box");
        assert_eq!(
            app.toolkits.apps.app_designer.placing,
            Some(AppComponentKind::CheckBox)
        );
        let out = step(&mut app, &ctx);
        let hint_texts: Vec<String> = texts(&out).into_iter().map(|(t, _)| t).collect();
        assert!(
            hint_texts
                .iter()
                .any(|t| t == "Placing: Check Box (click an empty cell, Esc to cancel)"),
            "expected updated placing hint, got: {hint_texts:?}"
        );

        // Click occupied cell (1, 1) (Button) while placing: selects Button in ONE click, ends placing, no error.
        let btn_pos = texts(&out)
            .into_iter()
            .rfind(|(t, _)| t == "Button")
            .map(|(_, r)| r.center())
            .expect("canvas Button preview");
        click_pos(&mut app, &ctx, btn_pos);
        assert_eq!(app.toolkits.apps.app_designer.model.components.len(), 2);
        assert_eq!(
            app.toolkits.apps.app_designer.selection,
            Selection::Component("Button".into()),
            "clicking occupied cell while placing should select that component in one click"
        );
        assert_eq!(
            app.toolkits.apps.app_designer.placing, None,
            "clicking occupied cell while placing should end placing mode"
        );
        assert!(
            app.toolkits.apps.app_designer.last_error.is_none(),
            "clicking occupied cell while placing should not emit an error: {:?}",
            app.toolkits.apps.app_designer.last_error
        );
        let out_after = step(&mut app, &ctx);
        assert!(
            !texts(&out_after)
                .iter()
                .any(|(t, _)| t.contains("already used by")),
            "must not display 'already used by' error when selecting occupied cell"
        );

        // Enter placing mode again and press Esc: ends placing mode.
        click_text(&mut app, &ctx, "Slider");
        assert_eq!(
            app.toolkits.apps.app_designer.placing,
            Some(AppComponentKind::Slider)
        );
        step_with(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert_eq!(
            app.toolkits.apps.app_designer.placing, None,
            "Esc must end placing mode"
        );
        let out_esc = step(&mut app, &ctx);
        assert!(
            !texts(&out_esc)
                .iter()
                .any(|(t, _)| t.starts_with("Placing:")),
            "Placing hint must clear after Esc"
        );
    }

    #[test]
    fn app_designer_save_fallback_end_to_end_save_run_and_callback() {
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        open_designer(&mut app, &ctx);

        // Place Button at (1,1) and Label at (1,2), add ButtonPushedFcn callback.
        click_text(&mut app, &ctx, "Button");
        click_text(&mut app, &ctx, "(1, 1)");
        click_text(&mut app, &ctx, "Label");
        click_text(&mut app, &ctx, "(1, 2)");
        click_text(&mut app, &ctx, "Button · Button");
        click_text(&mut app, &ctx, "+ ButtonPushedFcn");
        click_text(&mut app, &ctx, "Code View");
        focus_id_and_replace_text(
            &mut app,
            &ctx,
            egui::Id::new(("app_designer_body", "ButtonButtonPushed")),
            "app.Label.Text = 'FallbackOK';",
            false,
        );
        step(&mut app, &ctx);

        // Simulate missing native file dialog (test_save_path is None) and click Run on unsaved app.
        app.toolkits.apps.test_save_path = None;
        click_text(&mut app, &ctx, "Run");
        let out = step(&mut app, &ctx);

        // Fallback path bar must appear with a clear error shown ONCE (in Output panel only).
        assert!(
            app.toolkits.apps.app_designer.save_path_prompt.is_some(),
            "expected inline save path fallback prompt when native dialog is unavailable"
        );
        let frame_texts: Vec<String> = texts(&out).into_iter().map(|(t, _)| t).collect();
        let dialog_err_hits = frame_texts
            .iter()
            .filter(|t| t.contains("Could not open a save dialog:"))
            .count();
        assert_eq!(
            dialog_err_hits, 1,
            "expected save dialog fallback message exactly once, got {dialog_err_hits}: {frame_texts:?}"
        );
        assert!(
            !frame_texts.iter().any(|t| t == "No validation problems."),
            "must not show 'No validation problems.' alongside save dialog error"
        );

        // Type a .m path into the fallback field and press Enter -> saves AND runs!
        let dir = tempfile::tempdir().unwrap();
        let target_path = dir.path().join("FallbackApp.m");
        focus_id_and_replace_text(
            &mut app,
            &ctx,
            egui::Id::new("app_designer_save_path"),
            &target_path.to_string_lossy(),
            true,
        );
        wait_idle(&mut app, &ctx);

        assert!(target_path.exists(), "fallback save should write .m file");
        assert!(!app.toolkits.apps.app_designer.is_dirty());
        assert_eq!(app.toolkits.apps.app_designer.model.class_name, "FallbackApp");
        assert!(
            app.toolkits.apps.app_designer.last_error.is_none(),
            "stale error must clear after successful save: {:?}",
            app.toolkits.apps.app_designer.last_error
        );
        assert!(
            app.ui_windows.window_count() >= 1,
            "Run via fallback save must open the app's uifigure window; status: {}",
            app.status
        );

        // Fire the callback on the saved app and verify it updates Label.Text.
        app.submit(worker::Job::ScriptInFolder(
            "a = FallbackApp; \
             mrUiFire(a.Button, 'press'); \
             fallback_ok = double(strcmp(a.Label.Text, 'FallbackOK')); \
             delete(a);"
                .into(),
            dir.path().to_path_buf(),
        ));
        wait_idle(&mut app, &ctx);
        let ok = match app.variables.get("fallback_ok") {
            Some(matrust::Array::Numeric(m)) => m.data()[0],
            other => panic!("expected numeric fallback_ok, got {other:?}"),
        };
        assert_eq!(ok, 1.0, "expected callback to set Label.Text to 'FallbackOK'");
    }


    // One frame without `app.receive()`: the save job's reply stays queued,
    // so this input lands while the designer still shows "Saving…".
    fn step_while_saving(app: &mut Workspace, ctx: &egui::Context, events: Vec<egui::Event>) {
        crate::headless::run(
            ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SCREEN)),
                events,
                ..Default::default()
            },
            |ui| app.ui(ui),
        );
    }

    // 9b review must-fix 2: edits made while a save is in flight stay dirty.
    #[test]
    fn app_designer_edits_during_saving_stay_dirty_and_close_asks() {
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        open_designer(&mut app, &ctx);

        let dir = tempfile::tempdir().unwrap();
        let saved_path = dir.path().join("SavingEditApp.m");
        app.toolkits.apps.test_save_path = Some(saved_path.clone());
        click_text(&mut app, &ctx, "Save");
        assert!(app.toolkits.apps.app_designer.saving, "save should be in flight");

        // Edit the figure title through the inspector before the reply lands.
        let id = egui::Id::new("app_designer_fig_title");
        ctx.memory_mut(|m| m.request_focus(id));
        step_while_saving(&mut app, &ctx, Vec::new());
        step_while_saving(
            &mut app,
            &ctx,
            vec![
                egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                },
                egui::Event::Text("Edited While Saving".to_owned()),
            ],
        );
        assert_eq!(
            app.toolkits.apps.app_designer.model.figure.title,
            "Edited While Saving"
        );
        assert!(app.toolkits.apps.app_designer.saving, "reply must still be queued");

        wait_idle(&mut app, &ctx);
        let on_disk = std::fs::read_to_string(&saved_path).unwrap();
        assert!(!on_disk.contains("Edited While Saving"));
        assert!(
            app.toolkits.apps.app_designer.is_dirty(),
            "edits made during Saving… must stay dirty"
        );
        let msg = app.toolkits.apps.app_designer.message.clone();
        assert!(
            msg.contains("Saved") && msg.contains("newer edits not saved yet"),
            "notice should say newer edits are unsaved, got: {msg:?}"
        );

        // Closing now asks instead of dropping the edit.
        app.close_tool_tab(toolkits::Tool::App(AppKind::AppDesigner));
        assert!(app.toolkits.apps.is_open(AppKind::AppDesigner));
        assert!(app.toolkits.apps.app_designer.close_confirm);
        click_text(&mut app, &ctx, "Cancel");

        // A plain save with no later edits clears dirty.
        app.toolkits.apps.test_save_path = Some(saved_path.clone());
        click_text(&mut app, &ctx, "Save");
        wait_idle(&mut app, &ctx);
        assert!(!app.toolkits.apps.app_designer.is_dirty());
        assert!(std::fs::read_to_string(&saved_path).unwrap().contains("Edited While Saving"));
        app.close_tool_tab(toolkits::Tool::App(AppKind::AppDesigner));
        assert!(!app.toolkits.apps.is_open(AppKind::AppDesigner));
    }

    // 9b review must-fix 1 (UI side): a pasted body line that looks like a
    // region marker is refused in the one message place; Code View keeps one
    // editable region per method.
    #[test]
    fn app_designer_refuses_marker_lines_in_callback_bodies() {
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        open_designer(&mut app, &ctx);
        click_text(&mut app, &ctx, "Button");
        click_text(&mut app, &ctx, "(1, 1)");
        click_text(&mut app, &ctx, "Button · Button");
        click_text(&mut app, &ctx, "+ ButtonPushedFcn");
        click_text(&mut app, &ctx, "Code View");
        let method = app.toolkits.apps.app_designer.model.callbacks[0].method_name.clone();
        let injected = format!(
            "keep\n% [matrust-app-body:end {method}]\n% [matrust-app-body:start startupFcn]\nstolen\n% [matrust-app-body:end startupFcn]\nrest"
        );
        focus_id_and_replace_text(
            &mut app,
            &ctx,
            egui::Id::new(("app_designer_body", method.as_str())),
            &injected,
            false,
        );
        step(&mut app, &ctx);
        let d = &app.toolkits.apps.app_designer;
        assert!(
            !d.model.callbacks[0].body.contains("matrust-app-body"),
            "marker line must not be applied, body: {:?}",
            d.model.callbacks[0].body
        );
        let err = d.last_error.clone().unwrap_or_default();
        assert!(err.contains("matrust-app-body"), "expected refusal message, got: {err:?}");
        let out = step(&mut app, &ctx);
        let generated = texts(&out)
            .into_iter()
            .filter(|(t, _)| t.contains("stolen"))
            .count();
        assert_eq!(generated, 0, "Code View must not show the refused text");
    }
}
