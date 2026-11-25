// Desktop UI window tests (Engine 9a3). Included from src/desktop.rs.
// Every test name contains `ui_window` and drives the UI via `crate::headless::run`.

mod ui_window_helpers {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::rc::Rc;

    pub(super) struct ViewportHarness {
        pub(super) shown: Rc<RefCell<Vec<egui::ViewportId>>>,
        pub(super) inputs: Rc<RefCell<BTreeMap<egui::ViewportId, egui::RawInput>>>,
        pub(super) outputs: Rc<RefCell<BTreeMap<egui::ViewportId, egui::FullOutput>>>,
    }

    pub(super) fn install_ui_viewport_backend(ctx: &egui::Context) -> ViewportHarness {
        let shown = Rc::new(RefCell::new(Vec::new()));
        let inputs = Rc::new(RefCell::new(BTreeMap::<egui::ViewportId, egui::RawInput>::new()));
        let outputs = Rc::new(RefCell::new(BTreeMap::<egui::ViewportId, egui::FullOutput>::new()));
        let shown_c = shown.clone();
        let inputs_c = inputs.clone();
        let outputs_c = outputs.clone();
        ctx.set_embed_viewports(false);
        egui::Context::set_immediate_viewport_renderer(move |ctx, mut viewport| {
            let id = viewport.ids.this;
            shown_c.borrow_mut().push(id);
            let mut input = inputs_c.borrow_mut().remove(&id).unwrap_or_else(|| egui::RawInput {
                viewport_id: id,
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(920., 680.),
                )),
                ..Default::default()
            });
            input.viewport_id = id;
            if input.screen_rect.is_none() {
                input.screen_rect = Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(920., 680.),
                ));
            }
            let info = input.viewports.entry(id).or_default();
            info.parent = Some(egui::ViewportId::ROOT);
            let out = crate::headless::run(ctx, input, |ui| (viewport.viewport_ui_cb)(ui));
            outputs_c.borrow_mut().insert(id, out);
        });
        ViewportHarness {
            shown,
            inputs,
            outputs,
        }
    }

    pub(super) fn step_frame(app: &mut Workspace, ctx: &egui::Context) -> egui::FullOutput {
        app.receive();
        crate::headless::run(
            ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280., 800.),
                )),
                ..Default::default()
            },
            |ui| app.ui(ui),
        )
    }

    pub(super) fn wait_idle(app: &mut Workspace, ctx: &egui::Context) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while (app.busy || app.ui_events_pending) && Instant::now() < deadline {
            step_frame(app, ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!app.busy, "worker stayed busy: {}", app.status);
    }

    pub(super) fn collect_texts_with_rects(
        shape: &egui::Shape,
        out: &mut Vec<(String, egui::Rect)>,
    ) {
        match shape {
            egui::Shape::Text(t) => {
                let rect = t.galley.rect.translate(t.pos.to_vec2());
                out.push((t.galley.text().to_owned(), rect));
            }
            egui::Shape::Vec(v) => {
                for s in v {
                    collect_texts_with_rects(s, out);
                }
            }
            _ => {}
        }
    }

    pub(super) fn viewport_texts(
        harness: &ViewportHarness,
        vp_id: egui::ViewportId,
    ) -> Vec<(String, egui::Rect)> {
        let mut out = Vec::new();
        if let Some(output) = harness.outputs.borrow().get(&vp_id) {
            for clipped in &output.shapes {
                collect_texts_with_rects(&clipped.shape, &mut out);
            }
        }
        out
    }

    pub(super) fn click_at(
        app: &mut Workspace,
        ctx: &egui::Context,
        harness: &ViewportHarness,
        vp_id: egui::ViewportId,
        pos: egui::Pos2,
    ) {
        let press = egui::RawInput {
            viewport_id: vp_id,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(920., 680.),
            )),
            events: vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        };
        harness.inputs.borrow_mut().insert(vp_id, press);
        step_frame(app, ctx);

        let release = egui::RawInput {
            viewport_id: vp_id,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(920., 680.),
            )),
            events: vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        };
        harness.inputs.borrow_mut().insert(vp_id, release);
        step_frame(app, ctx);
    }

    pub(super) fn replace_text_and_enter(
        app: &mut Workspace,
        ctx: &egui::Context,
        harness: &ViewportHarness,
        vp_id: egui::ViewportId,
        pos: egui::Pos2,
        text: &str,
    ) {
        click_at(app, ctx, harness, vp_id, pos);
        let type_input = egui::RawInput {
            viewport_id: vp_id,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(920., 680.),
            )),
            events: vec![
                egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                },
                egui::Event::Text(text.to_owned()),
                egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        };
        harness.inputs.borrow_mut().insert(vp_id, type_input);
        step_frame(app, ctx);
    }

    pub(super) fn var_scalar(app: &Workspace, name: &str) -> Option<f64> {
        match app.variables.get(name)? {
            matrust::Array::Numeric(m) if m.len() == 1 => Some(m.data()[0]),
            matrust::Array::Logical(m) if m.len() == 1 => Some(f64::from(m.data()[0])),
            _ => None,
        }
    }

    pub(super) fn var_vec(app: &Workspace, name: &str) -> Option<Vec<f64>> {
        match app.variables.get(name)? {
            matrust::Array::Numeric(m) => Some(m.data().to_vec()),
            _ => None,
        }
    }

    pub(super) fn var_text(app: &Workspace, name: &str) -> Option<String> {
        let never = std::sync::atomic::AtomicBool::new(false);
        match app.variables.get(name)? {
            matrust::Array::Character(c) => c.to_utf8(&never).ok(),
            matrust::Array::Str(s) => s
                .data()
                .first()
                .and_then(|v| v.as_deref().map(str::to_owned)),
            other => Some(other.preview(64)),
        }
    }
}

#[test]
fn ui_window_demo_opens_with_widgets_and_interactive_callbacks() {
    use ui_window_helpers::*;

    let ctx = egui::Context::default();
    let harness = install_ui_viewport_backend(&ctx);
    let mut app = Workspace::with_context(ctx.clone(), true);

    let demo_src = include_str!("../../assets/examples/ui_demo.m");
    app.run(demo_src.into());
    wait_idle(&mut app, &ctx);
    step_frame(&mut app, &ctx);
    step_frame(&mut app, &ctx);

    // 1. One window with all demo widgets.
    assert_eq!(app.ui_windows.window_count(), 1);
    let snap = app.ui_windows.snapshot();
    let fig_key = snap.windows[0].fig_key;
    assert_eq!(snap.windows[0].title, "UI Components Demo");
    // Expected exact widget count: grid + panelGrid + 23 controls/containers = 25 nodes.
    assert_eq!(
        snap.nodes.len(),
        25,
        "expected exact 25 UI nodes in demo, got {}",
        snap.nodes.len()
    );
    let vp_id = ui_windows::UiWindows::viewport_id(fig_key);
    assert!(harness.shown.borrow().contains(&vp_id));

    // 2. Synthetic click on push button fires ButtonPushedFcn (label text changes).
    let texts = viewport_texts(&harness, vp_id);
    let btn_rect = texts
        .iter()
        .find(|(t, _)| t == "Click Me")
        .map(|(_, r)| *r)
        .expect("Click Me button must be rendered");
    click_at(&mut app, &ctx, &harness, vp_id, btn_rect.center());
    wait_idle(&mut app, &ctx);
    step_frame(&mut app, &ctx);
    let after_click = viewport_texts(&harness, vp_id);
    assert!(
        after_click.iter().any(|(t, _)| t == "Clicked 1"),
        "expected label to update to 'Clicked 1', got: {:?}",
        after_click.iter().map(|(t, _)| t).collect::<Vec<_>>()
    );

    // 3. Typing in numeric field + Enter fires ValueChangedFcn with PreviousValue.
    let num_rect = after_click
        .iter()
        .find(|(t, _)| t == "10")
        .map(|(_, r)| *r)
        .expect("numeric field with '10' must be rendered");
    replace_text_and_enter(&mut app, &ctx, &harness, vp_id, num_rect.center(), "42");
    wait_idle(&mut app, &ctx);
    step_frame(&mut app, &ctx);
    assert_eq!(var_scalar(&app, "lastNumPrev"), Some(10.0), "PreviousValue should be 10");
    assert_eq!(var_scalar(&app, "lastNumVal"), Some(42.0), "Value should be 42");

    // 4. Bad numeric entry keeps the old value.
    let after_num = viewport_texts(&harness, vp_id);
    let num_rect_42 = after_num
        .iter()
        .find(|(t, _)| t == "42")
        .map(|(_, r)| *r)
        .expect("numeric field with '42' must be rendered");
    replace_text_and_enter(
        &mut app,
        &ctx,
        &harness,
        vp_id,
        num_rect_42.center(),
        "not_a_number",
    );
    wait_idle(&mut app, &ctx);
    step_frame(&mut app, &ctx);
    assert_eq!(
        var_scalar(&app, "lastNumVal"),
        Some(42.0),
        "bad numeric entry must keep old value 42"
    );
    let after_bad_texts = viewport_texts(&harness, vp_id);
    assert!(
        after_bad_texts.iter().any(|(t, _)| t == "42"),
        "field must restore '42' after bad input"
    );

    // 5. Editing a table cell fires CellEditCallback.
    let tbl_cell_rect = after_bad_texts
        .iter()
        .find(|(t, _)| t == "44")
        .map(|(_, r)| *r)
        .expect("table cell '44' must be rendered");
    replace_text_and_enter(&mut app, &ctx, &harness, vp_id, tbl_cell_rect.center(), "99");
    wait_idle(&mut app, &ctx);
    step_frame(&mut app, &ctx);
    assert_eq!(var_vec(&app, "lastCellEdit"), Some(vec![2.0, 2.0, 99.0]));

    // 6. set(lbl, 'Text', 'x') from script shows on the next frame.
    app.run("set(statusLbl, 'Text', 'x');".into());
    wait_idle(&mut app, &ctx);
    step_frame(&mut app, &ctx);
    let after_set = viewport_texts(&harness, vp_id);
    assert!(
        after_set.iter().any(|(t, _)| t == "x"),
        "expected 'x' from set(statusLbl,'Text','x'), got: {:?}",
        after_set.iter().map(|(t, _)| t).collect::<Vec<_>>()
    );
}

#[test]
fn ui_window_close_calls_close_request_fcn_and_delete_closes() {
    use ui_window_helpers::*;

    let ctx = egui::Context::default();
    let harness = install_ui_viewport_backend(&ctx);
    let mut app = Workspace::with_context(ctx.clone(), true);

    app.run(
        "closedFlag = 0; \
         fig = uifigure('Name', 'Closable', 'CloseRequestFcn', @(s,e) evalin('base', 'closedFlag = 1; delete(fig);'));"
            .into(),
    );
    wait_idle(&mut app, &ctx);
    step_frame(&mut app, &ctx);
    assert_eq!(app.ui_windows.window_count(), 1);
    let fig_key = app.ui_windows.snapshot().windows[0].fig_key;
    let vp_id = ui_windows::UiWindows::viewport_id(fig_key);

    // Simulate OS window close button.
    let mut close_input = egui::RawInput {
        viewport_id: vp_id,
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(560., 420.),
        )),
        ..Default::default()
    };
    close_input
        .viewports
        .entry(vp_id)
        .or_default()
        .events
        .push(egui::ViewportEvent::Close);
    harness.inputs.borrow_mut().insert(vp_id, close_input);
    step_frame(&mut app, &ctx);
    wait_idle(&mut app, &ctx);
    step_frame(&mut app, &ctx);

    assert_eq!(
        var_scalar(&app, "closedFlag"),
        Some(1.0),
        "CloseRequestFcn must have run"
    );
    assert_eq!(app.ui_windows.window_count(), 0, "window must be closed");

    // Now test delete(fig) from script closes the window.
    app.run("fig2 = uifigure('Name', 'ToDelete');".into());
    wait_idle(&mut app, &ctx);
    step_frame(&mut app, &ctx);
    assert_eq!(app.ui_windows.window_count(), 1);

    app.run("delete(fig2);".into());
    wait_idle(&mut app, &ctx);
    step_frame(&mut app, &ctx);
    assert_eq!(app.ui_windows.window_count(), 0);
}

#[test]
fn ui_window_uiconfirm_and_uiwait_uiresume_round_trip() {
    use ui_window_helpers::*;

    let ctx = egui::Context::default();
    let harness = install_ui_viewport_backend(&ctx);
    let mut app = Workspace::with_context(ctx.clone(), true);

    // 1. uiconfirm click returns the clicked option.
    app.run(
        "fig = uifigure('Name', 'ConfirmHost'); \
         picked = uiconfirm(fig, 'Pick an option', 'Choose', 'Options', {'Alpha', 'Beta'}, 'DefaultOption', 'Alpha');"
            .into(),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut beta_pos = None;
    let mut vp_id_found = None;
    while Instant::now() < deadline {
        step_frame(&mut app, &ctx);
        if app.ui_windows.window_count() == 1 {
            let vp_id =
                ui_windows::UiWindows::viewport_id(app.ui_windows.snapshot().windows[0].fig_key);
            vp_id_found = Some(vp_id);
            let texts = viewport_texts(&harness, vp_id);
            if let Some((_, rect)) = texts.iter().find(|(t, _)| t == "Beta") {
                beta_pos = Some(rect.center());
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let vp_id = vp_id_found.expect("ConfirmHost window must open");
    let beta_pos = beta_pos.expect("Beta option button must appear in modal sheet");
    click_at(&mut app, &ctx, &harness, vp_id, beta_pos);
    wait_idle(&mut app, &ctx);
    assert_eq!(var_text(&app, "picked").as_deref(), Some("Beta"));

    // 2. uiwait(fig) blocks the script (not the UI) until uiresume(fig) is fired by a button click,
    // and callback output produced during uiwait appears in the console.
    app.run(
        "resumed = 0; \
         wfig = uifigure('Name', 'WaitHost'); \
         wbtn = uibutton(wfig, 'Text', 'ResumeNow', 'Position', [40 40 140 32], ...
             'ButtonPushedFcn', @(s,e) evalin('base', 'disp(''uiwait_cb_output''); resumed = 1; uiresume(wfig);')); \
         uiwait(wfig); \
         afterWait = 1;"
            .into(),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut resume_pos = None;
    let mut wvp_id = None;
    while Instant::now() < deadline {
        let t0 = Instant::now();
        step_frame(&mut app, &ctx);
        assert!(
            t0.elapsed() < Duration::from_millis(200),
            "UI frame must not freeze during uiwait"
        );
        if let Some(win) = app
            .ui_windows
            .snapshot()
            .windows
            .iter()
            .find(|w| w.title == "WaitHost")
        {
            let id = ui_windows::UiWindows::viewport_id(win.fig_key);
            wvp_id = Some(id);
            let texts = viewport_texts(&harness, id);
            if let Some((_, rect)) = texts.iter().find(|(t, _)| t == "ResumeNow") {
                resume_pos = Some(rect.center());
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        app.busy,
        "script must still be blocked in uiwait before click"
    );
    click_at(
        &mut app,
        &ctx,
        &harness,
        wvp_id.expect("WaitHost viewport"),
        resume_pos.expect("ResumeNow button"),
    );
    wait_idle(&mut app, &ctx);
    assert_eq!(var_scalar(&app, "resumed"), Some(1.0));
    assert_eq!(var_scalar(&app, "afterWait"), Some(1.0));
    assert!(
        app.console.iter().any(|l| l.contains("uiwait_cb_output")),
        "callback output during uiwait must appear in caller output: {:?}",
        app.console
    );
}

#[test]
fn ui_window_callback_runs_off_ui_thread() {
    use ui_window_helpers::*;

    let ctx = egui::Context::default();
    let harness = install_ui_viewport_backend(&ctx);
    let mut app = Workspace::with_context(ctx.clone(), true);

    app.run(
        "slowDone = 0; \
         sfig = uifigure('Name', 'SlowCallback'); \
         sbtn = uibutton(sfig, 'Text', 'SlowWork', 'Position', [40 40 140 32], ...
             'ButtonPushedFcn', @(s,e) evalin('base', 'pause(1.0); slowDone = 1;'));"
            .into(),
    );
    wait_idle(&mut app, &ctx);
    step_frame(&mut app, &ctx);
    let fig_key = app.ui_windows.snapshot().windows[0].fig_key;
    let vp_id = ui_windows::UiWindows::viewport_id(fig_key);
    let btn_pos = viewport_texts(&harness, vp_id)
        .into_iter()
        .find(|(t, _)| t == "SlowWork")
        .map(|(_, r)| r.center())
        .expect("SlowWork button must be rendered");

    // Click and verify UI frame returns quickly while the 1 s callback runs on the worker thread.
    let click_start = Instant::now();
    click_at(&mut app, &ctx, &harness, vp_id, btn_pos);
    let frame_elapsed = click_start.elapsed();
    assert!(
        frame_elapsed < Duration::from_millis(300),
        "UI frame blocked on callback: {frame_elapsed:?}"
    );
    assert!(
        app.busy,
        "1 s callback should still be running on worker thread"
    );

    // Pump another UI frame while the callback is still running: must return immediately.
    let pump_start = Instant::now();
    step_frame(&mut app, &ctx);
    assert!(
        pump_start.elapsed() < Duration::from_millis(200),
        "subsequent UI frame blocked during 1 s callback"
    );

    wait_idle(&mut app, &ctx);
    assert_eq!(var_scalar(&app, "slowDone"), Some(1.0));
}

#[test]
fn ui_window_classic_figure_with_uicontrol_opens_and_fires() {
    use ui_window_helpers::*;

    let ctx = egui::Context::default();
    let harness = install_ui_viewport_backend(&ctx);
    let mut app = Workspace::with_context(ctx.clone(), true);

    app.run(
        "classicClicked = 0; \
         f = figure; \
         b = uicontrol(f, 'Style', 'pushbutton', 'String', 'ClassicGo', ...
             'Position', [30 30 120 30], 'Callback', 'classicClicked = 1;');"
            .into(),
    );
    wait_idle(&mut app, &ctx);
    step_frame(&mut app, &ctx);
    assert_eq!(app.ui_windows.window_count(), 1);
    let fig_key = app.ui_windows.snapshot().windows[0].fig_key;
    let vp_id = ui_windows::UiWindows::viewport_id(fig_key);
    let btn_pos = viewport_texts(&harness, vp_id)
        .into_iter()
        .find(|(t, _)| t == "ClassicGo")
        .map(|(_, r)| r.center())
        .expect("ClassicGo button must be rendered");
    click_at(&mut app, &ctx, &harness, vp_id, btn_pos);
    wait_idle(&mut app, &ctx);
    assert_eq!(var_scalar(&app, "classicClicked"), Some(1.0));
}

#[test]
fn ui_window_demo_confirm_and_progress_button_runs_to_done() {
    use ui_window_helpers::*;

    let ctx = egui::Context::default();
    let harness = install_ui_viewport_backend(&ctx);
    let mut app = Workspace::with_context(ctx.clone(), true);

    let demo_src = include_str!("../../assets/examples/ui_demo.m");
    app.run(demo_src.into());
    wait_idle(&mut app, &ctx);
    step_frame(&mut app, &ctx);

    let fig_key = app.ui_windows.snapshot().windows[0].fig_key;
    let vp_id = ui_windows::UiWindows::viewport_id(fig_key);
    let dlg_btn_pos = viewport_texts(&harness, vp_id)
        .into_iter()
        .find(|(t, _)| t == "Confirm & Progress")
        .map(|(_, r)| r.center())
        .expect("Confirm & Progress button must be rendered");

    click_at(&mut app, &ctx, &harness, vp_id, dlg_btn_pos);

    // Wait for the uiconfirm modal sheet ("Run" button) to appear over the demo window.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut run_pos = None;
    while Instant::now() < deadline {
        step_frame(&mut app, &ctx);
        let texts = viewport_texts(&harness, vp_id);
        if let Some((_, rect)) = texts.iter().find(|(t, _)| t == "Run") {
            run_pos = Some(rect.center());
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let run_pos = run_pos.expect("uiconfirm Run button must appear");
    click_at(&mut app, &ctx, &harness, vp_id, run_pos);
    wait_idle(&mut app, &ctx);
    step_frame(&mut app, &ctx);

    let after = viewport_texts(&harness, vp_id);
    assert!(
        after.iter().any(|(t, _)| t == "Progress done"),
        "expected statusLbl to become 'Progress done', got: {:?}",
        after.iter().map(|(t, _)| t).collect::<Vec<_>>()
    );
    assert!(
        !app.console
            .iter()
            .any(|line| line.contains("Error executing")),
        "demo Confirm & Progress must not warn/error: {:?}",
        app.console
    );
}

#[test]
fn ui_window_uialert_nonblocking_with_closefcn_uiresume_and_uiwait() {
    use ui_window_helpers::*;

    let ctx = egui::Context::default();
    let harness = install_ui_viewport_backend(&ctx);
    let mut app = Workspace::with_context(ctx.clone(), true);

    // uialert must return at once so uiwait(fig) can run; CloseFcn runs on OK and calls uiresume(fig).
    app.run(
        "alertReturned = 0; alertClosed = 0; afterWait = 0; \
         fig = uifigure('Name', 'AlertHost'); \
         uialert(fig, 'Check complete', 'Notice', 'CloseFcn', @(~,~) evalin('base', 'alertClosed = 1; uiresume(fig);')); \
         alertReturned = 1; \
         uiwait(fig); \
         afterWait = 1;"
            .into(),
    );

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut ok_pos = None;
    let mut vp_id_found = None;
    while Instant::now() < deadline {
        step_frame(&mut app, &ctx);
        if let Some(win) = app
            .ui_windows
            .snapshot()
            .windows
            .iter()
            .find(|w| w.title == "AlertHost")
        {
            let vp_id = ui_windows::UiWindows::viewport_id(win.fig_key);
            vp_id_found = Some(vp_id);
            let texts = viewport_texts(&harness, vp_id);
            if let Some((_, rect)) = texts.iter().find(|(t, _)| t == "OK") {
                ok_pos = Some(rect.center());
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        app.busy,
        "script must be blocked in uiwait until OK is clicked"
    );
    click_at(
        &mut app,
        &ctx,
        &harness,
        vp_id_found.expect("AlertHost viewport"),
        ok_pos.expect("OK button on uialert sheet"),
    );
    wait_idle(&mut app, &ctx);
    assert_eq!(var_scalar(&app, "alertReturned"), Some(1.0));
    assert_eq!(var_scalar(&app, "alertClosed"), Some(1.0));
    assert_eq!(var_scalar(&app, "afterWait"), Some(1.0));
}

#[test]
fn ui_window_drawnow_dispatches_uiprogressdlg_cancel_during_loop() {
    use ui_window_helpers::*;

    let ctx = egui::Context::default();
    let harness = install_ui_viewport_backend(&ctx);
    let mut app = Workspace::with_context(ctx.clone(), true);

    app.run(
        r#"cancelled = 0;
iters = 0;
fig = uifigure('Name', 'ProgressHost');
d = uiprogressdlg(fig, 'Title', 'Busy', 'Message', 'Working...', 'Cancelable', 'on');
while ~d.CancelRequested && iters < 2000
    iters = iters + 1;
    d.Value = min(1, iters / 2000);
    drawnow;
    pause(0.005);
end
cancelled = double(d.CancelRequested);
close(d);"#
            .into(),
    );

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut cancel_pos = None;
    let mut vp_id_found = None;
    while Instant::now() < deadline {
        step_frame(&mut app, &ctx);
        if let Some(win) = app
            .ui_windows
            .snapshot()
            .windows
            .iter()
            .find(|w| w.title == "ProgressHost")
        {
            let vp_id = ui_windows::UiWindows::viewport_id(win.fig_key);
            vp_id_found = Some(vp_id);
            let texts = viewport_texts(&harness, vp_id);
            if let Some((_, rect)) = texts.iter().find(|(t, _)| t == "Cancel") {
                cancel_pos = Some(rect.center());
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.busy, "loop must still be running before Cancel click");
    click_at(
        &mut app,
        &ctx,
        &harness,
        vp_id_found.expect("ProgressHost viewport"),
        cancel_pos.expect("Cancel button on uiprogressdlg sheet"),
    );
    wait_idle(&mut app, &ctx);
    assert_eq!(var_scalar(&app, "cancelled"), Some(1.0));
    assert!(
        var_scalar(&app, "iters").unwrap_or(2000.0) < 2000.0,
        "loop must terminate early after Cancel click"
    );
}

#[test]
fn ui_window_classic_figure_uiwait_uiresume_via_gcbf_and_props() {
    use ui_window_helpers::*;

    let ctx = egui::Context::default();
    let harness = install_ui_viewport_backend(&ctx);
    let mut app = Workspace::with_context(ctx.clone(), true);

    // Classic figure with Name, Position, axes title (must not override Name), and uiresume(gcbf).
    app.run(
        "classicResumed = 0; \
         f = figure('Name', 'ClassicTitle', 'Position', [120 140 480 320]); \
         title('AxesTitleIgnored'); \
         b = uicontrol(f, 'Style', 'pushbutton', 'String', 'ResumeClassic', ...
             'Position', [30 30 140 32], ...
             'Callback', @(~,~) evalin('base', 'classicResumed = 1; uiresume(gcbf);')); \
         uiwait(f); \
         afterClassicWait = 1;"
            .into(),
    );

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut btn_pos = None;
    let mut vp_id_found = None;
    while Instant::now() < deadline {
        step_frame(&mut app, &ctx);
        if let Some(win) = app.ui_windows.snapshot().windows.first() {
            assert!(
                win.title.contains("ClassicTitle") && !win.title.contains("AxesTitleIgnored"),
                "classic window title must come from figure Name, got: {}",
                win.title
            );
            assert_eq!(win.position, [120.0, 140.0, 480.0, 320.0]);
            let vp_id = ui_windows::UiWindows::viewport_id(win.fig_key);
            vp_id_found = Some(vp_id);
            let texts = viewport_texts(&harness, vp_id);
            if let Some((_, rect)) = texts.iter().find(|(t, _)| t == "ResumeClassic") {
                btn_pos = Some(rect.center());
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        app.busy,
        "classic uiwait(f) must block until uiresume(gcbf)"
    );
    click_at(
        &mut app,
        &ctx,
        &harness,
        vp_id_found.expect("classic viewport"),
        btn_pos.expect("ResumeClassic button"),
    );
    wait_idle(&mut app, &ctx);
    assert_eq!(var_scalar(&app, "classicResumed"), Some(1.0));
    assert_eq!(var_scalar(&app, "afterClassicWait"), Some(1.0));

    // Classic figure Visible = 'off' hides the window from the snapshot.
    app.run("set(f, 'Visible', 'off');".into());
    wait_idle(&mut app, &ctx);
    assert_eq!(app.ui_windows.window_count(), 0);
}

#[test]
fn ui_window_classic_radiobutton_click_toggles_and_fires_callback() {
    use ui_window_helpers::*;

    let ctx = egui::Context::default();
    let harness = install_ui_viewport_backend(&ctx);
    let mut app = Workspace::with_context(ctx.clone(), true);

    app.run(
        "radioCount = 0; radioVal = -1; \
         f = figure('Name', 'RadioClassic'); \
         r = uicontrol(f, 'Style', 'radiobutton', 'String', 'EnableMode', ...
             'Position', [30 30 140 28], 'Value', 0, ...
             'Callback', @(src,~) evalin('base', sprintf('radioCount = radioCount + 1; radioVal = %g;', get(src, 'Value'))));"
            .into(),
    );
    wait_idle(&mut app, &ctx);
    step_frame(&mut app, &ctx);

    let fig_key = app.ui_windows.snapshot().windows[0].fig_key;
    let vp_id = ui_windows::UiWindows::viewport_id(fig_key);
    let radio_pos = viewport_texts(&harness, vp_id)
        .into_iter()
        .find(|(t, _)| t == "EnableMode")
        .map(|(_, r)| r.center())
        .expect("EnableMode radiobutton must be rendered");

    click_at(&mut app, &ctx, &harness, vp_id, radio_pos);
    wait_idle(&mut app, &ctx);
    assert_eq!(var_scalar(&app, "radioCount"), Some(1.0));
    assert_eq!(var_scalar(&app, "radioVal"), Some(1.0));
    assert!(
        !app.console
            .iter()
            .any(|line| line.contains("no parent ButtonGroup")),
        "classic radiobutton must not error about ButtonGroup: {:?}",
        app.console
    );
}

#[test]
fn ui_window_drain_continues_after_callback_error() {
    use ui_window_helpers::*;

    let ctx = egui::Context::default();
    let harness = install_ui_viewport_backend(&ctx);
    let mut app = Workspace::with_context(ctx.clone(), true);

    app.run(
        "secondRan = 0; \
         fig = uifigure('Name', 'DrainErr'); \
         b1 = uibutton(fig, 'Text', 'Boom', 'Position', [20 20 100 30], ...
             'ButtonPushedFcn', @(~,~) error('boom_test')); \
         b2 = uibutton(fig, 'Text', 'Good', 'Position', [140 20 100 30], ...
             'ButtonPushedFcn', @(~,~) evalin('base', 'secondRan = 1;'));"
            .into(),
    );
    wait_idle(&mut app, &ctx);
    step_frame(&mut app, &ctx);

    // Queue a failing event (bad numeric edit / callback error) followed by b2 press before draining.
    let snap = app.ui_windows.snapshot();
    let b1_key = snap
        .nodes
        .values()
        .find(|n| matches!(&n.widget, matrust::engine::UiWidgetSnapshot::Button { text } if text == "Boom"))
        .expect("Boom button node")
        .key;
    let b2_key = snap
        .nodes
        .values()
        .find(|n| matches!(&n.widget, matrust::engine::UiWidgetSnapshot::Button { text } if text == "Good"))
        .expect("Good button node")
        .key;
    // Queue: (1) callback error on b1, (2) dispatch-level error (TypeNumeric on a push button), (3) valid press on b2.
    let _ = app
        .worker
        .ui_events
        .send(matrust::engine::UiEvent::Press { key: b1_key });
    let _ = app
        .worker
        .ui_events
        .send(matrust::engine::UiEvent::TypeNumeric {
            key: b1_key,
            value: 5.0,
        });
    let _ = app
        .worker
        .ui_events
        .send(matrust::engine::UiEvent::Press { key: b2_key });
    app.ui_events_pending = true;
    step_frame(&mut app, &ctx);
    wait_idle(&mut app, &ctx);

    let _ = harness;
    assert_eq!(
        var_scalar(&app, "secondRan"),
        Some(1.0),
        "draining must continue to run b2 after b1 errors"
    );
    assert!(
        app.console
            .iter()
            .any(|line| line.contains("Error executing ButtonPushedFcn callback: boom_test")),
        "expected callback error warning in console, got: {:?}",
        app.console
    );
    assert!(
        app.console
            .iter()
            .any(|line| line.contains("Error executing UI callback:")),
        "expected dispatch error warning in console, got: {:?}",
        app.console
    );
}

#[test]
fn ui_window_drawnow_expose_skips_queued_callbacks() {
    use ui_window_helpers::*;

    let ctx = egui::Context::default();
    let harness = install_ui_viewport_backend(&ctx);
    let mut app = Workspace::with_context(ctx.clone(), true);

    // `drawnow expose` flushes the window but must not run the queued press.
    // A later plain `drawnow` does.
    app.run(
        "hits = 0; \
fig = uifigure('Name', 'ExposeSkip'); \
b = uibutton(fig, 'Text', 'GoExpose', 'Position', [20 20 120 30], \
'ButtonPushedFcn', @(~,~) evalin('base', 'hits = hits + 1;')); \
t0 = tic; \
while toc(t0) < 4, drawnow expose; end; \
exposeHits = hits; \
drawnow; \
finalHits = hits;"
            .into(),
    );

    let deadline = Instant::now() + Duration::from_secs(3);
    let mut btn_pos = None;
    let mut vp_id_found = None;
    while Instant::now() < deadline {
        step_frame(&mut app, &ctx);
        if let Some(win) = app
            .ui_windows
            .snapshot()
            .windows
            .iter()
            .find(|w| w.title == "ExposeSkip")
        {
            let vp_id = ui_windows::UiWindows::viewport_id(win.fig_key);
            vp_id_found = Some(vp_id);
            let texts = viewport_texts(&harness, vp_id);
            if let Some((_, rect)) = texts.iter().find(|(t, _)| t == "GoExpose") {
                btn_pos = Some(rect.center());
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        app.busy,
        "expose loop must still be running when the button is clicked"
    );
    click_at(
        &mut app,
        &ctx,
        &harness,
        vp_id_found.expect("ExposeSkip viewport"),
        btn_pos.expect("GoExpose button"),
    );
    wait_idle(&mut app, &ctx);
    assert_eq!(
        var_scalar(&app, "exposeHits"),
        Some(0.0),
        "drawnow expose must not dispatch the queued button callback"
    );
    assert_eq!(
        var_scalar(&app, "finalHits"),
        Some(1.0),
        "a later drawnow must dispatch the callback expose skipped"
    );
}

#[test]
fn ui_window_classic_figure_without_uicontrols_uialert_and_uiconfirm() {
    use ui_window_helpers::*;

    let ctx = egui::Context::default();
    let harness = install_ui_viewport_backend(&ctx);
    let mut app = Workspace::with_context(ctx.clone(), true);

    // 1. uialert on a plain classic figure with no uicontrols: window must appear, OK runs CloseFcn.
    app.run(
        "plainAlertClosed = 0; \
         f = figure('Name', 'PlainClassic'); \
         uialert(f, 'Plain alert body', 'PlainAlert', ...
             'CloseFcn', @(~,~) evalin('base', 'plainAlertClosed = 1;'));"
            .into(),
    );
    wait_idle(&mut app, &ctx);

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut ok_pos = None;
    let mut vp_id_found = None;
    while Instant::now() < deadline {
        step_frame(&mut app, &ctx);
        if let Some(win) = app
            .ui_windows
            .snapshot()
            .windows
            .iter()
            .find(|w| w.title.contains("PlainClassic"))
        {
            let vp_id = ui_windows::UiWindows::viewport_id(win.fig_key);
            vp_id_found = Some(vp_id);
            let texts = viewport_texts(&harness, vp_id);
            if texts.iter().any(|(t, _)| t == "Plain alert body")
                && let Some((_, rect)) = texts.iter().find(|(t, _)| t == "OK")
            {
                ok_pos = Some(rect.center());
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    click_at(
        &mut app,
        &ctx,
        &harness,
        vp_id_found.expect("PlainClassic viewport for uialert"),
        ok_pos.expect("OK button on plain classic uialert"),
    );
    wait_idle(&mut app, &ctx);
    step_frame(&mut app, &ctx);
    assert_eq!(var_scalar(&app, "plainAlertClosed"), Some(1.0));
    assert_eq!(
        app.ui_windows.window_count(),
        0,
        "classic figure without uicontrols should leave UI windows once alert is dismissed"
    );

    // 2. uiconfirm on the same plain classic figure: sheet must appear and unblock on click.
    app.run(
        "ansChoice = uiconfirm(f, 'Proceed plain?', 'PlainConfirm', ...
             'Options', {'Yes', 'No'}, 'DefaultOption', 1); \
         pickedNo = strcmp(ansChoice, 'No');"
            .into(),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut no_pos = None;
    let mut confirm_vp = None;
    while Instant::now() < deadline {
        step_frame(&mut app, &ctx);
        if let Some(win) = app
            .ui_windows
            .snapshot()
            .windows
            .iter()
            .find(|w| w.title.contains("PlainClassic"))
        {
            let vp_id = ui_windows::UiWindows::viewport_id(win.fig_key);
            confirm_vp = Some(vp_id);
            let texts = viewport_texts(&harness, vp_id);
            if texts.iter().any(|(t, _)| t == "Proceed plain?")
                && let Some((_, rect)) = texts.iter().find(|(t, _)| t == "No")
            {
                no_pos = Some(rect.center());
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.busy, "uiconfirm must block until answered");
    click_at(
        &mut app,
        &ctx,
        &harness,
        confirm_vp.expect("PlainClassic viewport for uiconfirm"),
        no_pos.expect("No button on plain classic uiconfirm"),
    );
    wait_idle(&mut app, &ctx);
    step_frame(&mut app, &ctx);
    assert_eq!(var_scalar(&app, "pickedNo"), Some(1.0));
    assert_eq!(app.ui_windows.window_count(), 0);
}

#[test]
fn ui_window_stacked_uialert_and_uiconfirm_queue_in_order() {
    use ui_window_helpers::*;

    let ctx = egui::Context::default();
    let harness = install_ui_viewport_backend(&ctx);
    let mut app = Workspace::with_context(ctx.clone(), true);

    // Two uialerts on the same figure followed by uiwait: first alert's CloseFcn resumes uiwait,
    // and the second alert shows next so neither CloseFcn is dropped or leaked.
    app.run(
        "c1 = 0; c2 = 0; afterWait = 0; \
         fig = uifigure('Name', 'StackedSheets'); \
         uialert(fig, 'FirstAlert', 'A1', ...
             'CloseFcn', @(~,~) evalin('base', 'c1 = 1; uiresume(fig);')); \
         uialert(fig, 'SecondAlert', 'A2', ...
             'CloseFcn', @(~,~) evalin('base', 'c2 = 1;')); \
         uiwait(fig); \
         afterWait = 1;"
            .into(),
    );

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut ok1_pos = None;
    let mut fig_key_found = None;
    while Instant::now() < deadline {
        step_frame(&mut app, &ctx);
        if let Some(win) = app
            .ui_windows
            .snapshot()
            .windows
            .iter()
            .find(|w| w.title == "StackedSheets")
        {
            fig_key_found = Some(win.fig_key);
            let vp_id = ui_windows::UiWindows::viewport_id(win.fig_key);
            let texts = viewport_texts(&harness, vp_id);
            if texts.iter().any(|(t, _)| t == "FirstAlert")
                && let Some((_, rect)) = texts.iter().find(|(t, _)| t == "OK")
            {
                assert!(
                    !texts.iter().any(|(t, _)| t == "SecondAlert"),
                    "second alert must wait in queue behind first"
                );
                ok1_pos = Some(rect.center());
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let fig_key = fig_key_found.expect("StackedSheets figure");
    let vp_id = ui_windows::UiWindows::viewport_id(fig_key);
    assert!(app.busy, "uiwait(fig) must be waiting for first alert OK");
    click_at(&mut app, &ctx, &harness, vp_id, ok1_pos.expect("first OK"));
    wait_idle(&mut app, &ctx);
    assert_eq!(var_scalar(&app, "c1"), Some(1.0));
    assert_eq!(var_scalar(&app, "afterWait"), Some(1.0));
    assert_eq!(var_scalar(&app, "c2"), Some(0.0));

    // Second alert is now at the front of the queue.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut ok2_pos = None;
    while Instant::now() < deadline {
        step_frame(&mut app, &ctx);
        let texts = viewport_texts(&harness, vp_id);
        if texts.iter().any(|(t, _)| t == "SecondAlert")
            && let Some((_, rect)) = texts.iter().find(|(t, _)| t == "OK")
        {
            ok2_pos = Some(rect.center());
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    click_at(&mut app, &ctx, &harness, vp_id, ok2_pos.expect("second OK"));
    wait_idle(&mut app, &ctx);
    assert_eq!(var_scalar(&app, "c2"), Some(1.0));
    assert_eq!(app.ui_windows.pending_sheet_count(fig_key), 0);

    // uialert followed immediately by uiconfirm on the same figure: alert first, then confirm.
    app.run(
        "c3 = 0; \
         uialert(fig, 'BeforeConfirm', 'BC', 'CloseFcn', @(~,~) evalin('base', 'c3 = 1;')); \
         ans3 = uiconfirm(fig, 'AfterAlert', 'AC', 'Options', {'Go', 'Stop'}); \
         pickedGo = strcmp(ans3, 'Go');"
            .into(),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut ok3_pos = None;
    while Instant::now() < deadline {
        step_frame(&mut app, &ctx);
        let texts = viewport_texts(&harness, vp_id);
        if texts.iter().any(|(t, _)| t == "BeforeConfirm")
            && let Some((_, rect)) = texts.iter().find(|(t, _)| t == "OK")
        {
            ok3_pos = Some(rect.center());
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.busy, "uiconfirm must be waiting behind queued uialert");
    click_at(&mut app, &ctx, &harness, vp_id, ok3_pos.expect("third OK"));

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut go_pos = None;
    while Instant::now() < deadline {
        step_frame(&mut app, &ctx);
        let texts = viewport_texts(&harness, vp_id);
        if texts.iter().any(|(t, _)| t == "AfterAlert")
            && let Some((_, rect)) = texts.iter().find(|(t, _)| t == "Go")
        {
            go_pos = Some(rect.center());
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    click_at(&mut app, &ctx, &harness, vp_id, go_pos.expect("Go button"));
    wait_idle(&mut app, &ctx);
    assert_eq!(var_scalar(&app, "pickedGo"), Some(1.0));
    assert_eq!(var_scalar(&app, "c3"), Some(1.0));
    assert_eq!(app.ui_windows.pending_sheet_count(fig_key), 0);
}

#[test]
fn ui_window_deleted_figure_drops_pending_sheet_and_reused_key_is_clean() {
    use ui_window_helpers::*;

    let ctx = egui::Context::default();
    let harness = install_ui_viewport_backend(&ctx);
    let mut app = Workspace::with_context(ctx.clone(), true);

    // 1. Live figure with pending alert is deleted: sheet is dropped and CloseFcn must NOT run.
    app.run(
        "staleCloseRan = 0; \
         fig = uifigure('Name', 'WillDelete'); \
         uialert(fig, 'StaleAlertBody', 'StaleTitle', ...
             'CloseFcn', @(~,~) evalin('base', 'staleCloseRan = 1;'));"
            .into(),
    );
    wait_idle(&mut app, &ctx);
    step_frame(&mut app, &ctx);
    let fig_key = app.ui_windows.snapshot().windows[0].fig_key;
    assert_eq!(app.ui_windows.pending_sheet_count(fig_key), 1);

    app.run("delete(fig);".into());
    wait_idle(&mut app, &ctx);
    step_frame(&mut app, &ctx);
    assert_eq!(app.ui_windows.window_count(), 0);
    assert_eq!(
        app.ui_windows.pending_sheet_count(fig_key),
        0,
        "set_snapshot must prune pending sheets when figure disappears"
    );
    assert_eq!(
        var_scalar(&app, "staleCloseRan"),
        Some(0.0),
        "CloseFcn must not run for a deleted figure"
    );

    // Engine-side pending_alert_close must also be cleared: a stray AlertClosed does nothing.
    let _ = app
        .worker
        .ui_events
        .send(matrust::engine::UiEvent::AlertClosed { id: 1 });
    app.ui_events_pending = true;
    step_frame(&mut app, &ctx);
    wait_idle(&mut app, &ctx);
    assert_eq!(var_scalar(&app, "staleCloseRan"), Some(0.0));

    // 2. Key-reuse check: a new figure reusing the exact same fig_key must open without the old sheet.
    let reused_key: usize = 0xBEEF;
    let make_snap = |title: &str| matrust::engine::UiSnapshot {
        windows: vec![matrust::engine::UiWindowSnapshot {
            fig_key: reused_key,
            is_classic: false,
            classic_fig_id: None,
            title: title.into(),
            position: [100., 100., 400., 300.],
            color: [0.94, 0.94, 0.94],
            root_children: Vec::new(),
            progress_dialogs: Vec::new(),
        }],
        nodes: Default::default(),
    };
    let mut ui_wins = ui_windows::UiWindows::default();
    ui_wins.set_snapshot(make_snap("OldFig"));
    let leftover = ui_wins.receive_dialog(worker::DialogTicket {
        request: matrust::engine::DialogRequest::UiAlert {
            id: 42,
            parent_fig: reused_key,
            title: "OldAlert".into(),
            text: vec!["OldStaleText".into()],
            icon: matrust::engine::MessageIcon::Error,
        },
        reply: None,
        output: Vec::new(),
        clear: false,
    });
    assert!(leftover.is_none());
    assert_eq!(ui_wins.pending_sheet_count(reused_key), 1);

    // Figure leaves snapshot -> sheet dropped.
    ui_wins.set_snapshot(matrust::engine::UiSnapshot::default());
    assert_eq!(ui_wins.pending_sheet_count(reused_key), 0);

    // New figure reuses the same fig_key -> no stale sheet rendered.
    ui_wins.set_snapshot(make_snap("NewFig"));
    assert_eq!(ui_wins.pending_sheet_count(reused_key), 0);
    let _ = crate::headless::run(
        &ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280., 800.),
            )),
            ..Default::default()
        },
        |ui| {
            let _ = ui_wins.show(ui.ctx());
        },
    );
    let vp_id = ui_windows::UiWindows::viewport_id(reused_key);
    let texts = viewport_texts(&harness, vp_id);
    assert!(
        !texts.iter().any(|(t, _)| t == "OldStaleText"),
        "reused fig_key must not show stale alert: {texts:?}"
    );
}

#[test]
fn ui_window_uiresume_no_arg_only_resumes_gcf() {
    use ui_window_helpers::*;

    let ctx = egui::Context::default();
    let harness = install_ui_viewport_backend(&ctx);
    let mut app = Workspace::with_context(ctx.clone(), true);

    // f1 waits, f2 is gcf (not waiting): bare `uiresume` must do nothing; `uiresume(f1)` resumes.
    app.run(
        "bareRan = 0; afterF1 = 0; \
         f1 = figure('Name', 'OuterWait'); \
         b1 = uicontrol(f1, 'Style', 'pushbutton', 'String', 'BareResume', ...
             'Position', [20 20 130 30], ...
             'Callback', @(~,~) evalin('base', 'uiresume; bareRan = 1;')); \
         b2 = uicontrol(f1, 'Style', 'pushbutton', 'String', 'ResumeF1', ...
             'Position', [170 20 130 30], ...
             'Callback', @(~,~) evalin('base', 'uiresume(f1);')); \
         f2 = figure('Name', 'CurrentNotWaiting'); \
         uiwait(f1); \
         afterF1 = 1;"
            .into(),
    );

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut bare_pos = None;
    let mut f1_pos = None;
    let mut vp_id_found = None;
    while Instant::now() < deadline {
        step_frame(&mut app, &ctx);
        if let Some(win) = app
            .ui_windows
            .snapshot()
            .windows
            .iter()
            .find(|w| w.title.contains("OuterWait"))
        {
            let vp_id = ui_windows::UiWindows::viewport_id(win.fig_key);
            vp_id_found = Some(vp_id);
            let texts = viewport_texts(&harness, vp_id);
            bare_pos = texts
                .iter()
                .find(|(t, _)| t == "BareResume")
                .map(|(_, r)| r.center());
            f1_pos = texts
                .iter()
                .find(|(t, _)| t == "ResumeF1")
                .map(|(_, r)| r.center());
            if bare_pos.is_some() && f1_pos.is_some() {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let vp_id = vp_id_found.expect("OuterWait viewport");
    assert!(app.busy, "uiwait(f1) must be waiting");

    // Click BareResume: callback runs bare `uiresume` while gcf is f2 (not waiting).
    click_at(&mut app, &ctx, &harness, vp_id, bare_pos.expect("BareResume"));
    for _ in 0..20 {
        step_frame(&mut app, &ctx);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        app.busy,
        "bare uiresume must NOT release f1 when gcf (f2) is not waiting"
    );

    // Explicit uiresume(f1) releases the wait.
    click_at(&mut app, &ctx, &harness, vp_id, f1_pos.expect("ResumeF1"));
    wait_idle(&mut app, &ctx);
    assert_eq!(var_scalar(&app, "bareRan"), Some(1.0));
    assert_eq!(var_scalar(&app, "afterF1"), Some(1.0));
}

#[test]
fn ui_window_demo_layout_rects_desktop_and_narrow() {
    use ui_window_helpers::*;

    fn collect_rects(
        shape: &egui::Shape,
        clip: egui::Rect,
        out: &mut Vec<(egui::Rect, egui::Rect, bool)>,
    ) {
        match shape {
            egui::Shape::Rect(r) => {
                let has_stroke = r.stroke.width > 0.0 && r.stroke.color != egui::Color32::TRANSPARENT;
                out.push((r.rect, clip, has_stroke));
            }
            egui::Shape::Vec(v) => {
                for s in v {
                    collect_rects(s, clip, out);
                }
            }
            _ => {}
        }
    }

    fn viewport_rects(
        harness: &ViewportHarness,
        vp_id: egui::ViewportId,
    ) -> Vec<(egui::Rect, egui::Rect, bool)> {
        let mut out = Vec::new();
        if let Some(output) = harness.outputs.borrow().get(&vp_id) {
            for clipped in &output.shapes {
                collect_rects(&clipped.shape, clipped.clip_rect, &mut out);
            }
        }
        out
    }

    fn viewport_texts_with_clip(
        harness: &ViewportHarness,
        vp_id: egui::ViewportId,
    ) -> Vec<(String, egui::Rect, egui::Rect)> {
        let mut out = Vec::new();
        if let Some(output) = harness.outputs.borrow().get(&vp_id) {
            for clipped in &output.shapes {
                let mut texts = Vec::new();
                collect_texts_with_rects(&clipped.shape, &mut texts);
                for (t, r) in texts {
                    out.push((t, r, clipped.clip_rect));
                }
            }
        }
        out
    }

    fn step_at_size(
        app: &mut Workspace,
        ctx: &egui::Context,
        harness: &ViewportHarness,
        vp_id: egui::ViewportId,
        size: egui::Vec2,
    ) {
        harness.inputs.borrow_mut().insert(
            vp_id,
            egui::RawInput {
                viewport_id: vp_id,
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            },
        );
        step_frame(app, ctx);
    }

    let ctx = egui::Context::default();
    let harness = install_ui_viewport_backend(&ctx);
    let mut app = Workspace::with_context(ctx.clone(), true);

    let demo_src = include_str!("../../assets/examples/ui_demo.m");
    app.run(demo_src.into());
    wait_idle(&mut app, &ctx);
    step_frame(&mut app, &ctx);
    step_frame(&mut app, &ctx);

    let fig_key = app.ui_windows.snapshot().windows[0].fig_key;
    let vp_id = ui_windows::UiWindows::viewport_id(fig_key);

    // --- 1. Desktop (920x680): uitab child inside tab body + uitable fills its grid cell ---
    let texts = viewport_texts_with_clip(&harness, vp_id);
    let rects = viewport_rects(&harness, vp_id);

    let notes_rect = texts
        .iter()
        .find(|(t, _, _)| t == "Notes")
        .map(|(_, r, _)| *r)
        .expect("Notes tab label must render");
    let items_rect = texts
        .iter()
        .find(|(t, _, _)| t == "Items")
        .map(|(_, r, _)| *r)
        .expect("Items tab label must render");
    let ta_entry = texts
        .iter()
        .find(|(t, _, _)| t.contains("Line 1"))
        .cloned()
        .expect("Notes tab textarea must render");

    // TabGroup frame contains both Notes and Items tabs and spans Row 4 Col 2.
    let tab_frame = rects
        .iter()
        .filter(|(r, _, stroked)| {
            *stroked
                && r.contains(notes_rect.center())
                && r.contains(items_rect.center())
                && r.width() > 200.0
                && r.height() > 150.0
        })
        .map(|(r, _, _)| *r)
        .min_by(|a, b| a.area().total_cmp(&b.area()))
        .expect("TabGroup must render a bounded frame around its tab strip and body");

    // Textarea must be inside the tab body (below the tab strip, within the tab frame).
    assert!(
        ta_entry.1.top() >= notes_rect.bottom(),
        "textarea top ({}) must be below Notes tab strip bottom ({})",
        ta_entry.1.top(),
        notes_rect.bottom()
    );
    assert!(
        tab_frame.contains_rect(ta_entry.1),
        "textarea rect {:?} must lie inside tab frame {:?}",
        ta_entry.1,
        tab_frame
    );
    assert!(
        ta_entry.2.top() >= notes_rect.bottom(),
        "textarea clip rect {:?} must clip strictly to tab body below tab strip",
        ta_entry.2
    );

    // Table frame in Row 4 Col 3 must fill its grid cell (not sit small at top-left).
    let col_a_rect = texts
        .iter()
        .find(|(t, _, _)| t == "A")
        .map(|(_, r, _)| *r)
        .expect("table column A header must render");
    let col_b_rect = texts
        .iter()
        .find(|(t, _, _)| t == "B")
        .map(|(_, r, _)| *r)
        .expect("table column B header must render");
    let tbl_frame = rects
        .iter()
        .filter(|(r, _, stroked)| {
            *stroked
                && r.contains(col_a_rect.center())
                && r.contains(col_b_rect.center())
                && r.left() > tab_frame.right() - 4.0
        })
        .map(|(r, _, _)| *r)
        .max_by(|a, b| a.area().total_cmp(&b.area()))
        .expect("uitable must render a bordered frame");
    assert!(
        tbl_frame.width() > 280.0 && tbl_frame.height() > 200.0,
        "uitable must stretch to fill its grid cell, got {:?}",
        tbl_frame.size()
    );

    // Switch to Items tab and verify uilistbox renders inside the same tab body.
    click_at(&mut app, &ctx, &harness, vp_id, items_rect.center());
    wait_idle(&mut app, &ctx);
    step_frame(&mut app, &ctx);
    let items_texts = viewport_texts_with_clip(&harness, vp_id);
    let one_entry = items_texts
        .iter()
        .find(|(t, _, _)| t == "One")
        .cloned()
        .expect("Items tab listbox item 'One' must render after tab switch");
    assert!(
        one_entry.1.top() >= items_rect.bottom(),
        "listbox item 'One' top ({}) must be below tab strip ({})",
        one_entry.1.top(),
        items_rect.bottom()
    );
    assert!(
        tab_frame.contains_rect(one_entry.1),
        "listbox item 'One' {:?} must lie inside tab frame {:?}",
        one_entry.1,
        tab_frame
    );

    // Switch back to Notes tab before testing narrow layout.
    click_at(&mut app, &ctx, &harness, vp_id, notes_rect.center());
    wait_idle(&mut app, &ctx);
    step_frame(&mut app, &ctx);

    // --- 2. Narrow window (480x680): spinner '+', textarea right border, table horizontal scroll ---
    let narrow = egui::vec2(480.0, 680.0);
    step_at_size(&mut app, &ctx, &harness, vp_id, narrow);
    step_at_size(&mut app, &ctx, &harness, vp_id, narrow);

    let n_texts = viewport_texts_with_clip(&harness, vp_id);
    let n_rects = viewport_rects(&harness, vp_id);

    // 2a. Spinner '-' and '+' buttons must both remain fully inside the cell and unclipped.
    let minus_entry = n_texts
        .iter()
        .find(|(t, _, _)| t == "-")
        .cloned()
        .expect("spinner '-' button must render at 480x680");
    let plus_entry = n_texts
        .iter()
        .find(|(t, _, _)| t == "+")
        .cloned()
        .expect("spinner '+' button must render at 480x680");
    assert!(
        plus_entry.1.right() <= 460.5,
        "spinner '+' text {:?} must fit inside 480 px window grid",
        plus_entry.1
    );
    assert!(
        plus_entry.1.right() <= plus_entry.2.right() + 0.5,
        "spinner '+' text {:?} must not be clipped by {:?}",
        plus_entry.1,
        plus_entry.2
    );
    let plus_btn_box = n_rects
        .iter()
        .filter(|(r, _, stroked)| {
            *stroked && r.contains(plus_entry.1.center()) && r.width() <= 40.0
        })
        .map(|(r, clip, _)| (*r, *clip))
        .min_by(|a, b| a.0.area().total_cmp(&b.0.area()))
        .expect("spinner '+' button rect must be painted");
    assert!(
        plus_btn_box.0.right() <= plus_btn_box.1.right() + 0.5,
        "spinner '+' button {:?} must not be clipped on the right by {:?}",
        plus_btn_box.0,
        plus_btn_box.1
    );
    assert!(minus_entry.1.right() < plus_entry.1.left());

    // 2b. Textarea in Notes tab must shrink to fit inside the narrow tab body (not cut on right).
    let n_notes = n_texts
        .iter()
        .find(|(t, _, _)| t == "Notes")
        .map(|(_, r, _)| *r)
        .expect("Notes tab at 480x680");
    let n_tab_frame = n_rects
        .iter()
        .filter(|(r, _, stroked)| {
            *stroked && r.contains(n_notes.center()) && r.height() > 150.0
        })
        .map(|(r, _, _)| *r)
        .min_by(|a, b| a.area().total_cmp(&b.area()))
        .expect("narrow TabGroup frame");
    let n_ta = n_texts
        .iter()
        .find(|(t, _, _)| t.contains("Line 1"))
        .cloned()
        .expect("narrow textarea text");
    let n_ta_box = n_rects
        .iter()
        .filter(|(r, _, stroked)| {
            *stroked
                && r.contains(n_ta.1.center())
                && r.width() < n_tab_frame.width()
                && r.height() > 40.0
        })
        .map(|(r, clip, _)| (*r, *clip))
        .min_by(|a, b| a.0.area().total_cmp(&b.0.area()))
        .expect("narrow textarea frame");
    assert!(
        n_ta_box.0.right() <= n_tab_frame.right() + 0.5
            && n_ta_box.0.right() <= n_ta_box.1.right() + 0.5,
        "narrow textarea box {:?} must fit inside tab frame {:?} and clip {:?} without right-edge clipping",
        n_ta_box.0,
        n_tab_frame,
        n_ta_box.1
    );

    // 2c. Table at 480x680 fills its narrow cell and renders a horizontal scrollbar for Column B.
    let n_col_a = n_texts
        .iter()
        .find(|(t, _, _)| t == "A")
        .map(|(_, r, _)| *r)
        .expect("narrow table col A");
    let n_tbl_frame = n_rects
        .iter()
        .filter(|(r, _, stroked)| {
            *stroked && r.contains(n_col_a.center()) && r.left() > n_tab_frame.right() - 4.0
        })
        .map(|(r, _, _)| *r)
        .max_by(|a, b| a.area().total_cmp(&b.area()))
        .expect("narrow table frame");
    assert!(
        n_tbl_frame.width() > 80.0 && n_tbl_frame.height() > 200.0,
        "narrow table must fill its cell, got {:?}",
        n_tbl_frame.size()
    );
    let has_h_scrollbar = n_rects.iter().any(|(r, _, _)| {
        n_tbl_frame.contains_rect(*r)
            && r.top() > n_tbl_frame.bottom() - 24.0
            && r.height() >= 3.0
            && r.height() <= 16.0
            && r.width() >= 20.0
    });
    assert!(
        has_h_scrollbar,
        "narrow uitable {:?} must render a horizontal scrollbar when Column B overflows",
        n_tbl_frame
    );

    // --- 3. Show Alert button opens modal uialert sheet ---
    step_at_size(&mut app, &ctx, &harness, vp_id, egui::vec2(920.0, 680.0));
    let alert_btn_pos = viewport_texts(&harness, vp_id)
        .into_iter()
        .find(|(t, _)| t == "Show Alert")
        .map(|(_, r)| r.center())
        .expect("Show Alert button must render");
    click_at(&mut app, &ctx, &harness, vp_id, alert_btn_pos);
    wait_idle(&mut app, &ctx);
    step_frame(&mut app, &ctx);
    let alert_texts = viewport_texts(&harness, vp_id);
    assert!(
        alert_texts.iter().any(|(t, _)| t == "Demo alert message."),
        "Show Alert must open uialert sheet, got: {:?}",
        alert_texts.iter().map(|(t, _)| t).collect::<Vec<_>>()
    );
    let ok_pos = alert_texts
        .iter()
        .find(|(t, _)| t == "OK")
        .map(|(_, r)| r.center())
        .expect("uialert OK button");
    click_at(&mut app, &ctx, &harness, vp_id, ok_pos);
    wait_idle(&mut app, &ctx);
    step_frame(&mut app, &ctx);
    assert_eq!(app.ui_windows.pending_sheet_count(fig_key), 0);

    // --- 4. Confirm & Progress: clicking Cancel during progress stops early and sets 'Cancelled' ---
    let dlg_btn_pos = viewport_texts(&harness, vp_id)
        .into_iter()
        .find(|(t, _)| t == "Confirm & Progress")
        .map(|(_, r)| r.center())
        .expect("Confirm & Progress button");
    click_at(&mut app, &ctx, &harness, vp_id, dlg_btn_pos);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut run_pos = None;
    while Instant::now() < deadline {
        step_frame(&mut app, &ctx);
        if let Some((_, rect)) = viewport_texts(&harness, vp_id)
            .into_iter()
            .find(|(t, _)| t == "Run")
        {
            run_pos = Some(rect.center());
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    click_at(
        &mut app,
        &ctx,
        &harness,
        vp_id,
        run_pos.expect("uiconfirm Run button"),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut cancel_pos = None;
    while Instant::now() < deadline {
        step_frame(&mut app, &ctx);
        let texts = viewport_texts(&harness, vp_id);
        if texts.iter().any(|(t, _)| t.starts_with("Step "))
            && let Some((_, rect)) = texts.iter().find(|(t, _)| t == "Cancel")
        {
            cancel_pos = Some(rect.center());
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    click_at(
        &mut app,
        &ctx,
        &harness,
        vp_id,
        cancel_pos.expect("uiprogressdlg Cancel button"),
    );
    wait_idle(&mut app, &ctx);
    step_frame(&mut app, &ctx);
    let after_cancel = viewport_texts(&harness, vp_id);
    assert!(
        after_cancel.iter().any(|(t, _)| t == "Cancelled"),
        "expected statusLbl to become 'Cancelled' after progress cancel, got: {:?}",
        after_cancel.iter().map(|(t, _)| t).collect::<Vec<_>>()
    );
}

// le9a fix item 6: frame time of a 10000x10 uitable vs an empty uifigure
// (median of 50 headless frames) and engine snapshot cost. Measurement only;
// run with `-- --ignored --nocapture ui_window_uitable_frame_time`.
#[test]
#[ignore]
fn ui_window_uitable_frame_time_measure() {
    use ui_window_helpers::*;

    fn median_frame_ms(script: &str) -> f64 {
        let ctx = egui::Context::default();
        let _harness = install_ui_viewport_backend(&ctx);
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.run(script.into());
        wait_idle(&mut app, &ctx);
        for _ in 0..5 {
            step_frame(&mut app, &ctx);
        }
        assert_eq!(app.ui_windows.window_count(), 1);
        let mut times: Vec<f64> = (0..50)
            .map(|_| {
                let t = Instant::now();
                step_frame(&mut app, &ctx);
                t.elapsed().as_secs_f64() * 1e3
            })
            .collect();
        times.sort_by(f64::total_cmp);
        times[25]
    }
    let empty = median_frame_ms("f = uifigure('Name','Empty');");
    let table = median_frame_ms("f = uifigure('Name','Table'); t = uitable(f,'Data',rand(10000,10));");

    let mut engine = matrust::Engine::default();
    engine
        .execute("f = uifigure; t = uitable(f,'Data',rand(10000,10));")
        .unwrap();
    let mut snaps: Vec<f64> = (0..20)
        .map(|_| {
            let t = Instant::now();
            let s = matrust::engine::capture_ui_snapshot(&engine);
            let ms = t.elapsed().as_secs_f64() * 1e3;
            drop(s);
            ms
        })
        .collect();
    snaps.sort_by(f64::total_cmp);
    eprintln!(
        "UITABLE_MEASURE empty_frame_ms={empty:.3} table_frame_ms={table:.3} snapshot_ms={:.3}",
        snaps[10]
    );
}
