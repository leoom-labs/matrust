//! Engine 9b1: `matlab.apps.AppBase` superclass, App Designer property types,
//! `registerApp`, `runStartupFcn`, `createCallbackFcn`, `clear app` lifetime,
//! and running a generated-layout App Designer class in `-batch`.
use matrust::Engine;
use std::path::Path;
use std::process::Command;

fn engine_in(dir: &Path) -> Engine {
    let mut engine = Engine::default();
    engine.set_function_directory(dir).unwrap();
    engine
}

fn scalar_num(e: &Engine, name: &str) -> f64 {
    e.variables[name].data()[0]
}

fn scalar_bool(e: &Engine, name: &str) -> bool {
    e.logical_variables[name].data()[0] != 0
}

fn char_var(e: &Engine, name: &str) -> String {
    match e.array(name).unwrap_or_else(|| panic!("missing {name}")) {
        matrust::Array::Character(c) => c
            .to_utf8(&std::sync::atomic::AtomicBool::new(false))
            .unwrap(),
        other => panic!("expected char for {name}, got {other:?}"),
    }
}

#[test]
fn app_base_superclass_methods_and_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("LifecycleApp.m"),
        r#"classdef LifecycleApp < matlab.apps.AppBase
    properties (Access = public)
        UIFigure matlab.ui.Figure
        Btn      matlab.ui.control.Button
        BtnNoEv  matlab.ui.control.Button
        Note     char = ''
        Calls    double = 0
        LastEv   char = ''
    end
    methods (Access = private)
        function startupFcn(app, prefix, num)
            app.Note = sprintf('%s:%d', prefix, num);
        end
        function onWithEvent(app, event)
            app.Calls = app.Calls + 1;
            app.LastEv = event.EventName;
        end
        function onNoEvent(app)
            app.Calls = app.Calls + 10;
        end
    end
    methods (Access = public)
        function app = LifecycleApp(varargin)
            app.UIFigure = uifigure('Visible', 'off');
            app.Btn = uibutton(app.UIFigure, 'push');
            app.Btn.ButtonPushedFcn = createCallbackFcn(app, @onWithEvent, true);
            app.BtnNoEv = uibutton(app.UIFigure, 'push');
            app.BtnNoEv.ButtonPushedFcn = createCallbackFcn(app, @onNoEvent, false);
            registerApp(app, app.UIFigure);
            if nargin == 0
                runStartupFcn(app, @(app)startupFcn(app, 'def', 7));
            else
                runStartupFcn(app, @(app)startupFcn(app, varargin{:}));
            end
        end
    end
end
"#,
    )
    .unwrap();

    let mut e = engine_in(dir.path());
    e.execute(
        r#"
        a = LifecycleApp('hello', 42);
        is_ab = isa(a, 'matlab.apps.AppBase');
        is_h = isa(a, 'handle');
        sc = superclasses(a);
        sc_n = numel(sc);
        sc1 = sc{1};
        sc2 = sc{2};
        sc_base = superclasses('matlab.apps.AppBase');
        sc_base1 = sc_base{1};
        valid_init = isvalid(a);
        note = a.Note;

        mrUiFire(a.Btn, 'press');
        calls1 = a.Calls;
        ev1 = a.LastEv;

        mrUiFire(a.BtnNoEv, 'press');
        calls2 = a.Calls;

        % Deleting app deletes registered figure even without an explicit delete method.
        f = a.UIFigure;
        b = a.Btn;
        delete(a);
        a_valid_after_del = isvalid(a);
        f_valid_after_del = isvalid(f);
        b_valid_after_del = isvalid(b);

        % Deleting figure deletes app.
        a2 = LifecycleApp();
        note2 = a2.Note;
        delete(a2.UIFigure);
        a2_valid_after_fig_del = isvalid(a2);

        % Closing figure without CloseRequestFcn deletes app.
        a3 = LifecycleApp();
        close(a3.UIFigure);
        a3_valid_after_close = isvalid(a3);

        % Closing figure WITH vetoing CloseRequestFcn keeps app alive; force closes it.
        a4 = LifecycleApp();
        a4.UIFigure.CloseRequestFcn = @(src, ev) set(src, 'Tag', 'vetoed');
        close(a4.UIFigure);
        a4_valid_after_veto = isvalid(a4);
        a4_tag = a4.UIFigure.Tag;
        close(a4.UIFigure, 'force');
        a4_valid_after_force = isvalid(a4);
        "#,
    )
    .unwrap();

    assert!(scalar_bool(&e, "is_ab"));
    assert!(scalar_bool(&e, "is_h"));
    assert_eq!(scalar_num(&e, "sc_n"), 2.0);
    assert_eq!(char_var(&e, "sc1"), "matlab.apps.AppBase");
    assert_eq!(char_var(&e, "sc2"), "handle");
    assert_eq!(char_var(&e, "sc_base1"), "handle");
    assert!(scalar_bool(&e, "valid_init"));
    assert_eq!(char_var(&e, "note"), "hello:42");
    assert_eq!(scalar_num(&e, "calls1"), 1.0);
    assert_eq!(char_var(&e, "ev1"), "ButtonPushed");
    assert_eq!(scalar_num(&e, "calls2"), 11.0);
    assert!(!scalar_bool(&e, "a_valid_after_del"));
    assert!(!scalar_bool(&e, "f_valid_after_del"));
    assert!(!scalar_bool(&e, "b_valid_after_del"));
    assert_eq!(char_var(&e, "note2"), "def:7");
    assert!(!scalar_bool(&e, "a2_valid_after_fig_del"));
    assert!(!scalar_bool(&e, "a3_valid_after_close"));
    assert!(scalar_bool(&e, "a4_valid_after_veto"));
    assert_eq!(char_var(&e, "a4_tag"), "vetoed");
    assert!(!scalar_bool(&e, "a4_valid_after_force"));
}

#[test]
fn app_base_all_21_property_types_defaults_and_mismatch_errors() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("AllTypesApp.m"),
        r#"classdef AllTypesApp < matlab.apps.AppBase
    properties (Access = public)
        Fig         matlab.ui.Figure
        Grid        matlab.ui.container.GridLayout
        Panel       matlab.ui.container.Panel
        TabGroup    matlab.ui.container.TabGroup
        Tab         matlab.ui.container.Tab
        BtnGroup    matlab.ui.container.ButtonGroup
        Btn         matlab.ui.control.Button
        StateBtn    matlab.ui.control.StateButton
        Lbl         matlab.ui.control.Label
        Edit        matlab.ui.control.EditField
        NumEdit     matlab.ui.control.NumericEditField
        Area        matlab.ui.control.TextArea
        Drop        matlab.ui.control.DropDown
        List        matlab.ui.control.ListBox
        Check       matlab.ui.control.CheckBox
        Radio       matlab.ui.control.RadioButton
        Slide       matlab.ui.control.Slider
        Spin        matlab.ui.control.Spinner
        Sw          matlab.ui.control.Switch
        Tbl         matlab.ui.control.Table
        Ax          matlab.ui.control.UIAxes
    end
    methods (Access = public)
        function app = AllTypesApp
        end
    end
end
"#,
    )
    .unwrap();

    let mut e = engine_in(dir.path());
    e.execute(
        r#"
        a = AllTypesApp;
        % Defaults before assignment are empty typed handles, not errors.
        def_empty = isempty(a.Fig) && isempty(a.Btn) && isempty(a.Ax) && isempty(a.Grid);
        def_cls_fig = class(a.Fig);
        def_cls_btn = class(a.Btn);
        def_cls_ax = class(a.Ax);
        def_isa_ok = isa(a.Btn, 'matlab.ui.control.Button') && ~isa(a.Btn, 'matlab.ui.control.Label');

        % Assign all 21 matching component kinds.
        a.Fig = uifigure('Visible', 'off');
        a.Grid = uigridlayout(a.Fig, [2 2]);
        a.Panel = uipanel(a.Grid);
        a.TabGroup = uitabgroup(a.Grid);
        a.Tab = uitab(a.TabGroup);
        a.BtnGroup = uibuttongroup(a.Panel);
        a.Btn = uibutton(a.Panel, 'push');
        a.StateBtn = uibutton(a.Panel, 'state');
        a.Lbl = uilabel(a.Panel);
        a.Edit = uieditfield(a.Panel, 'text');
        a.NumEdit = uieditfield(a.Panel, 'numeric');
        a.Area = uitextarea(a.Panel);
        a.Drop = uidropdown(a.Panel);
        a.List = uilistbox(a.Panel);
        a.Check = uicheckbox(a.Panel);
        a.Radio = uiradiobutton(a.BtnGroup);
        a.Slide = uislider(a.Panel);
        a.Spin = uispinner(a.Panel);
        a.Sw = uiswitch(a.Panel);
        a.Tbl = uitable(a.Panel);
        a.Ax = uiaxes(a.Panel);

        assigned_ok = isa(a.Fig, 'matlab.ui.Figure') ...
            && isa(a.Btn, 'matlab.ui.control.Button') ...
            && isa(a.StateBtn, 'matlab.ui.control.StateButton') ...
            && isa(a.NumEdit, 'matlab.ui.control.NumericEditField') ...
            && isa(a.Ax, 'matlab.ui.control.UIAxes');
        "#,
    )
    .unwrap();

    assert!(scalar_bool(&e, "def_empty"));
    assert_eq!(char_var(&e, "def_cls_fig"), "matlab.ui.Figure");
    assert_eq!(char_var(&e, "def_cls_btn"), "matlab.ui.control.Button");
    assert_eq!(char_var(&e, "def_cls_ax"), "matlab.ui.control.UIAxes");
    assert!(scalar_bool(&e, "def_isa_ok"));
    assert!(scalar_bool(&e, "assigned_ok"));

    // Wrong kinds assigned must produce MATLAB-style validation errors.
    for bad in [
        "a.Btn = a.Lbl;",
        "a.Btn = a.StateBtn;",
        "a.Edit = a.NumEdit;",
        "a.Fig = 42;",
        "a.Ax = a.Panel;",
    ] {
        let err = e
            .execute(bad)
            .expect_err(&format!("expected error for `{bad}`"));
        assert!(
            err.contains("Unable to convert value to matlab.ui."),
            "unexpected error for `{bad}`: {err}"
        );
    }

    e.execute("delete(a.Fig); delete(a);").unwrap();
}

#[test]
fn app_base_counter_app_callbacks_and_delete_close() {
    let fixture_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/app_base");
    let mut e = engine_in(&fixture_dir);
    e.execute(
        r#"
        app = CounterApp;
        lbl0 = app.CountLabel.Text;
        c0 = app.getCount();
        s0 = app.getStep();

        mrUiFire(app.IncrementButton, 'press');
        lbl1 = app.CountLabel.Text;
        c1 = app.getCount();

        mrUiFire(app.StepEditField, 'type', 5);
        lbl2 = app.CountLabel.Text;
        s2 = app.getStep();

        mrUiFire(app.IncrementButton, 'press');
        lbl3 = app.CountLabel.Text;
        c3 = app.getCount();

        fig = app.UIFigure;
        grid = app.GridLayout;
        lbl = app.CountLabel;
        ef = app.StepEditField;
        btn = app.IncrementButton;
        delete(app);
        all_invalid = ~isvalid(app) && ~isvalid(fig) && ~isvalid(grid) ...
            && ~isvalid(lbl) && ~isvalid(ef) && ~isvalid(btn);

        app2 = CounterApp;
        close(app2.UIFigure);
        app2_invalid = ~isvalid(app2);
        "#,
    )
    .unwrap();

    assert_eq!(char_var(&e, "lbl0"), "Count: 10 (step 2)");
    assert_eq!(scalar_num(&e, "c0"), 10.0);
    assert_eq!(scalar_num(&e, "s0"), 2.0);
    assert_eq!(char_var(&e, "lbl1"), "Count: 12 (step 2)");
    assert_eq!(scalar_num(&e, "c1"), 12.0);
    assert_eq!(char_var(&e, "lbl2"), "Count: 12 (step 5)");
    assert_eq!(scalar_num(&e, "s2"), 5.0);
    assert_eq!(char_var(&e, "lbl3"), "Count: 17 (step 5)");
    assert_eq!(scalar_num(&e, "c3"), 17.0);
    assert!(scalar_bool(&e, "all_invalid"));
    assert!(scalar_bool(&e, "app2_invalid"));
}

#[test]
fn app_base_clear_app_nargout_zero_keeps_app_alive_and_runs_in_batch() {
    let fixture_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/app_base");
    let mut e = engine_in(&fixture_dir);
    // Run CounterApp with nargout == 0: `clear app` runs inside the constructor,
    // no `ans` is created, and the app stays alive through its registered UIFigure.
    e.execute(
        r#"
        CounterApp;
        has_ans = exist('ans', 'var');
        "#,
    )
    .unwrap();
    assert_eq!(scalar_num(&e, "has_ans"), 0.0);
    // The app handle is still alive in the handle table and its figure is open.
    assert!(
        e.cls_live_handles() >= 1,
        "expected registered app handle to remain alive after `clear app` when nargout == 0"
    );
    // Find the button and label on the open figure (the app is only reachable
    // through its figure) and fire the button callback; state must update.
    e.execute(
        r#"
        btn = findall('Type', 'uibutton');
        lbl = findall('Type', 'uilabel');
        before_text = lbl.Text;
        mrUiFire(btn, 'press');
        after_text = lbl.Text;
        "#,
    )
    .unwrap();
    assert_eq!(char_var(&e, "before_text"), "Count: 10 (step 2)");
    assert_eq!(char_var(&e, "after_text"), "Count: 12 (step 2)");
    e.execute("close all;").unwrap();

    // Verify both `CounterApp` and `app = CounterApp;` run cleanly in `-batch`.
    let bin = env!("CARGO_BIN_EXE_matrust");
    let batch_zero = Command::new(bin)
        .current_dir(&fixture_dir)
        .args(["-batch", "CounterApp; disp('OK_ZERO');"])
        .output()
        .unwrap();
    assert!(
        batch_zero.status.success(),
        "batch zero-output failed: {}",
        String::from_utf8_lossy(&batch_zero.stderr)
    );
    assert!(
        String::from_utf8_lossy(&batch_zero.stdout).contains("OK_ZERO"),
        "stdout: {}",
        String::from_utf8_lossy(&batch_zero.stdout)
    );

    let batch_out = Command::new(bin)
        .current_dir(&fixture_dir)
        .args([
            "-batch",
            "app = CounterApp; mrUiFire(app.IncrementButton, 'press'); disp(app.CountLabel.Text); mrUiFire(app.StepEditField, 'type', 5); disp(app.CountLabel.Text); mrUiFire(app.IncrementButton, 'press'); disp(app.CountLabel.Text); delete(app);",
        ])
        .output()
        .unwrap();
    assert!(
        batch_out.status.success(),
        "batch with output failed: {}",
        String::from_utf8_lossy(&batch_out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&batch_out.stdout).contains("Count: 12 (step 2)"),
        "stdout: {}",
        String::from_utf8_lossy(&batch_out.stdout)
    );
    // Both callbacks fire by name in -batch: the edit field's ValueChangedFcn
    // updates the step, then the button uses it.
    let batch_stdout = String::from_utf8_lossy(&batch_out.stdout);
    let lines: Vec<&str> = batch_stdout
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("Count:"))
        .collect();
    assert_eq!(
        lines,
        [
            "Count: 12 (step 2)",
            "Count: 12 (step 5)",
            "Count: 17 (step 5)"
        ],
        "stdout: {batch_stdout}"
    );
}

#[test]
fn app_base_close_all_preserves_delete_output() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("LoudDeleteApp.m"),
        r#"classdef LoudDeleteApp < matlab.apps.AppBase
    properties (Access = public)
        UIFigure matlab.ui.Figure
    end
    methods (Access = public)
        function app = LoudDeleteApp
            app.UIFigure = uifigure('Visible', 'off');
            registerApp(app, app.UIFigure);
            if nargout == 0
                clear app
            end
        end
        function delete(app)
            disp('APP_DELETED_CLEANLY');
            delete(app.UIFigure);
        end
    end
end
"#,
    )
    .unwrap();
    let mut e = engine_in(dir.path());
    e.execute("LoudDeleteApp;").unwrap();
    let exec = e.execute("close all;").unwrap();
    assert!(
        exec.output
            .iter()
            .any(|l| l.contains("APP_DELETED_CLEANLY")),
        "expected delete output from `close all`, got: {:?}",
        exec.output
    );
}
