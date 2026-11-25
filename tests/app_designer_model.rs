//! Engine 9b2: App Designer non-UI model, MATLAB classdef code generation,
//! round-trip reload, read-only `.mlapp` extraction, and `-batch` done-check.

use matrust::engine::app_designer_model::{
    AppCallback, AppComponent, AppComponentKind, AppGridLayout, AppGridTrack, AppLoadResult,
    AppModel, AppPrivateProperty, AppPropertyValue, AppValue, GridSpanSpec, generate_m,
    is_valid_identifier, load_m_source, open_app_file, open_mlapp, parse_generated_m,
};
use std::io::Write;
use std::path::Path;
use std::process::Command;

#[test]
fn model_operations_add_move_rename_remove_and_properties() {
    let mut model = AppModel::new("DemoApp");
    model.grid = Some(AppGridLayout::new("GridLayout", 3, 2));

    let btn_name = model.next_component_name(AppComponentKind::Button);
    assert_eq!(btn_name, "Button");
    model
        .add_component(AppComponent::new("RunBtn", AppComponentKind::Button).in_grid(1, 1))
        .unwrap();
    model
        .add_component(AppComponent::new("StatusLbl", AppComponentKind::Label).in_grid(1, 2))
        .unwrap();
    model
        .add_component(
            AppComponent::new("GainField", AppComponentKind::NumericEditField).in_grid(2, 1),
        )
        .unwrap();
    model
        .add_component(AppComponent::new("ModeDrop", AppComponentKind::DropDown).in_grid(2, 2))
        .unwrap();

    // Set valid properties
    model
        .set_property("RunBtn", "Text", AppPropertyValue::Text("Run".into()))
        .unwrap();
    model
        .set_property(
            "GainField",
            "Limits",
            AppPropertyValue::Limits([0.0, 100.0]),
        )
        .unwrap();
    model
        .set_property("GainField", "Value", AppPropertyValue::Number(5.0))
        .unwrap();
    model
        .set_property(
            "ModeDrop",
            "Items",
            AppPropertyValue::Lines(vec!["Sine".into(), "Cosine".into()]),
        )
        .unwrap();
    model
        .set_property("ModeDrop", "Value", AppPropertyValue::Text("Sine".into()))
        .unwrap();

    assert_eq!(
        model.get_property("RunBtn", "Text"),
        Some(AppPropertyValue::Text("Run".into()))
    );
    assert_eq!(
        model.get_property("GainField", "Value"),
        Some(AppPropertyValue::Number(5.0))
    );

    // Unsupported property for kind is rejected
    assert!(
        model
            .set_property("RunBtn", "Limits", AppPropertyValue::Limits([0.0, 1.0]))
            .is_err()
    );
    assert!(
        model
            .set_property("StatusLbl", "Value", AppPropertyValue::Number(1.0))
            .is_err()
    );

    // Add callback
    let method = model.default_callback_method_name("RunBtn", "ButtonPushedFcn");
    assert_eq!(method, "RunBtnButtonPushed");
    model
        .add_callback(AppCallback {
            component: "RunBtn".into(),
            event: "ButtonPushedFcn".into(),
            method_name: method.clone(),
            body: "app.StatusLbl.Text = 'Done';".into(),
        })
        .unwrap();

    // Moving onto an occupied cell is refused
    assert!(
        model
            .move_component_grid("RunBtn", GridSpanSpec::Single(1), GridSpanSpec::Single(2))
            .is_err()
    );
    // Moving to an empty cell succeeds
    model
        .move_component_grid("RunBtn", GridSpanSpec::Single(3), GridSpanSpec::Single(1))
        .unwrap();
    assert_eq!(
        model.component("RunBtn").unwrap().grid_row,
        Some(GridSpanSpec::Single(3))
    );

    // Rename component updates callbacks
    model.rename_component("RunBtn", "GoButton").unwrap();
    assert!(model.component("RunBtn").is_none());
    assert!(model.component("GoButton").is_some());
    assert_eq!(model.callbacks[0].component, "GoButton");

    // Rename callback
    model
        .rename_callback("RunBtnButtonPushed", "GoButtonPushed")
        .unwrap();
    assert_eq!(model.callbacks[0].method_name, "GoButtonPushed");

    // Removing component leaves orphaned callback for validate() to flag
    model.remove_component("GoButton").unwrap();
    let problems = model.validate();
    assert!(
        problems
            .iter()
            .any(|p| p.code == "missing_callback_component"),
        "expected missing_callback_component problem, got {problems:?}"
    );

    // Removing the orphaned callback restores clean validation
    model.remove_callback("GoButtonPushed").unwrap();
    assert!(model.validate().is_empty());
}

#[test]
fn validate_catches_identifiers_duplicates_grid_conflicts_and_missing_targets() {
    assert!(is_valid_identifier("MyApp_1"));
    assert!(!is_valid_identifier(""));
    assert!(!is_valid_identifier("1Bad"));
    assert!(!is_valid_identifier("has space"));
    assert!(!is_valid_identifier("for"));
    assert!(!is_valid_identifier(&"a".repeat(64)));

    let mut model = AppModel::new("123BadClass");
    model.grid = Some(AppGridLayout::new("GridLayout", 2, 2));
    model
        .components
        .push(AppComponent::new("bad name", AppComponentKind::Button).in_grid(1, 1));
    model
        .components
        .push(AppComponent::new("DupComp", AppComponentKind::Label).in_grid(1, 2));
    model
        .components
        .push(AppComponent::new("DupComp", AppComponentKind::Label).in_grid(1, 2));
    model.callbacks.push(AppCallback {
        component: "GhostButton".into(),
        event: "ButtonPushedFcn".into(),
        method_name: "GhostPushed".into(),
        body: String::new(),
    });

    let problems = model.validate();
    let codes: Vec<&str> = problems.iter().map(|p| p.code).collect();
    assert!(codes.contains(&"bad_identifier"), "codes: {codes:?}");
    assert!(codes.contains(&"duplicate_name"), "codes: {codes:?}");
    assert!(codes.contains(&"grid_cell_conflict"), "codes: {codes:?}");
    assert!(
        codes.contains(&"missing_callback_component"),
        "codes: {codes:?}"
    );

    // Code generation refuses invalid models
    assert!(generate_m(&model).is_err());
}

#[test]
fn codegen_quoting_safety_and_round_trip_with_edited_bodies() {
    let mut model = AppModel::new("RichApp");
    model.figure.title = "Bob's \"Demo\" App".into();
    model.figure.position = [120.0, 80.0, 720.0, 500.0];
    model.grid = Some(AppGridLayout {
        name: "GridLayout".into(),
        row_height: vec![
            AppGridTrack::Pixels(40.0),
            AppGridTrack::Weight(1.5),
            AppGridTrack::Fit,
        ],
        column_width: vec![AppGridTrack::Weight(1.0), AppGridTrack::Weight(2.0)],
    });

    model.private_properties.push(AppPrivateProperty {
        name: "Counter".into(),
        default_expr: Some("0".into()),
    });
    model.private_properties.push(AppPrivateProperty {
        name: "Cache".into(),
        default_expr: None,
    });
    model.startup_fcn = Some("app.Counter = 42;\napp.InfoLabel.Text = 'Ready';".into());

    let mut lbl = AppComponent::new("InfoLabel", AppComponentKind::Label).in_grid(1, 1);
    lbl.props.text = Some("It's a 'quoted' label".into());
    model.add_component(lbl).unwrap();

    let mut btn = AppComponent::new("ActionBtn", AppComponentKind::Button).in_grid(1, 2);
    btn.props.text = Some("Click 'Me'".into());
    model.add_component(btn).unwrap();

    let mut num = AppComponent::new("NumField", AppComponentKind::NumericEditField).in_grid(2, 1);
    num.props.limits = Some([-10.0, f64::INFINITY]);
    num.props.value = Some(AppValue::Number(3.5));
    model.add_component(num).unwrap();

    let mut dd = AppComponent::new("ChoiceDrop", AppComponentKind::DropDown).in_grid(2, 2);
    dd.props.items = Some(vec!["A's".into(), "B, C".into()]);
    dd.props.value = Some(AppValue::Text("A's".into()));
    model.add_component(dd).unwrap();

    let mut chk = AppComponent::new("FlagBox", AppComponentKind::CheckBox).in_grid(3, 1);
    chk.props.text = Some("Enable".into());
    chk.props.value = Some(AppValue::Bool(true));
    model.add_component(chk).unwrap();

    let mut tbl = AppComponent::new("DataTable", AppComponentKind::Table).in_grid(3, 2);
    tbl.props.column_name = Some(vec!["X".into(), "Y's".into()]);
    model.add_component(tbl).unwrap();

    model
        .add_callback(AppCallback {
            component: "ActionBtn".into(),
            event: "ButtonPushedFcn".into(),
            method_name: "ActionBtnPushed".into(),
            body: "app.Counter = app.Counter + 1;\nif app.Counter > 0\n    app.InfoLabel.Text = 'OK';\nend".into(),
        })
        .unwrap();

    let code = model.generate_m().unwrap();
    // Quotes are properly doubled in generated MATLAB code
    assert!(code.contains("app.UIFigure.Name = 'Bob''s \"Demo\" App';"));
    assert!(code.contains("app.InfoLabel.Text = 'It''s a ''quoted'' label';"));

    // Exact round trip
    let reloaded = parse_generated_m(&code).unwrap();
    assert_eq!(reloaded, model);

    // Editing a callback body between markers reloads with the edited body
    let edited_code = code.replace(
        "app.Counter = app.Counter + 1;",
        "app.Counter = app.Counter + 10; % edited",
    );
    let AppLoadResult::Editable(reloaded_edited) = load_m_source(&edited_code) else {
        panic!("expected Editable result");
    };
    assert!(
        reloaded_edited.callbacks[0]
            .body
            .contains("app.Counter = app.Counter + 10; % edited")
    );
    assert_eq!(reloaded_edited.components, model.components);

    // Newline injection in a text property is refused
    let mut bad = model.clone();
    bad.components[0].props.text = Some("hello'\ndisp('injected')".into());
    assert!(bad.generate_m().is_err());

    // Hand-written classdef (CounterApp.m) opens read-only with a clear message
    let counter_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/app_base/CounterApp.m");
    let AppLoadResult::ReadOnlyCode {
        code: ro_code,
        reason,
    } = open_app_file(&counter_path).unwrap()
    else {
        panic!("expected CounterApp.m to open as ReadOnlyCode");
    };
    assert!(ro_code.contains("classdef CounterApp < matlab.apps.AppBase"));
    assert!(reason.contains("read-only"));
}

#[test]
fn round_trip_without_grid_uses_positions() {
    let mut model = AppModel::new_without_grid("PosApp");
    let mut panel = AppComponent::new("MainPanel", AppComponentKind::Panel)
        .with_position([10.0, 20.0, 300.0, 200.0]);
    panel.props.title = Some("Controls".into());
    model.add_component(panel).unwrap();

    let mut ta = AppComponent::new("NotesArea", AppComponentKind::TextArea)
        .with_parent("MainPanel")
        .with_position([15.0, 15.0, 200.0, 100.0]);
    ta.props.value = Some(AppValue::Lines(vec![
        "First line".into(),
        "Second's line".into(),
    ]));
    model.add_component(ta).unwrap();

    let code = model.generate_m().unwrap();
    let reloaded = parse_generated_m(&code).unwrap();
    assert_eq!(reloaded, model);
}

fn write_zip_file(path: &Path, entries: &[(&str, &[u8])]) {
    let file = std::fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default();
    for (name, data) in entries {
        zip.start_file(*name, opts).unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap();
}

#[test]
fn mlapp_opens_read_only_code_and_rejects_broken_archives() {
    let dir = tempfile::tempdir().unwrap();

    // 1. Valid .mlapp with <w:p>, <w:t>, CDATA, and XML entities
    let valid_path = dir.path().join("Sample.mlapp");
    let doc_xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>classdef Sample &lt; matlab.apps.AppBase</w:t></w:r></w:p>
    <w:p/>
    <w:p><w:r><w:t><![CDATA[    % CDATA: x < y & z > 0]]></w:t></w:r></w:p>
    <w:p><w:r><w:t>    % Entities: &apos;hi&apos; &amp; &quot;ok&quot;</w:t></w:r></w:p>
    <w:p><w:r><w:t>end</w:t></w:r></w:p>
  </w:body>
</w:document>"#;
    write_zip_file(&valid_path, &[("matlab/document.xml", doc_xml.as_bytes())]);

    let extracted = open_mlapp(&valid_path).unwrap();
    assert_eq!(
        extracted,
        "classdef Sample < matlab.apps.AppBase\n\n    % CDATA: x < y & z > 0\n    % Entities: 'hi' & \"ok\"\nend"
    );

    let AppLoadResult::ReadOnlyCode { code, reason } = open_app_file(&valid_path).unwrap() else {
        panic!("expected .mlapp to open as ReadOnlyCode");
    };
    assert_eq!(code, extracted);
    assert!(reason.contains("read-only"));

    // 2. Broken .mlapp: missing matlab/document.xml
    let missing_part = dir.path().join("MissingPart.mlapp");
    write_zip_file(&missing_part, &[("other.txt", b"hello")]);
    let err = open_mlapp(&missing_part).unwrap_err();
    assert!(
        err.starts_with("Cannot open MissingPart.mlapp: ")
            && err.ends_with("MatRust opens .mlapp files read-only as code; save apps as .m."),
        "unexpected error: {err}"
    );

    // 3. Broken .mlapp: not a zip archive
    let not_zip = dir.path().join("Corrupt.mlapp");
    std::fs::write(&not_zip, b"not a zip file").unwrap();
    let err = open_mlapp(&not_zip).unwrap_err();
    assert!(
        err.starts_with("Cannot open Corrupt.mlapp: ")
            && err.ends_with("MatRust opens .mlapp files read-only as code; save apps as .m."),
        "unexpected error: {err}"
    );

    // 4. Broken .mlapp: document.xml without classdef
    let no_classdef = dir.path().join("NoClass.mlapp");
    let no_class_xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>x = 1 + 2;</w:t></w:r></w:p></w:body></w:document>"#;
    write_zip_file(
        &no_classdef,
        &[("matlab/document.xml", no_class_xml.as_bytes())],
    );
    let err = open_mlapp(&no_classdef).unwrap_err();
    assert!(
        err.starts_with("Cannot open NoClass.mlapp: ")
            && err.ends_with("MatRust opens .mlapp files read-only as code; save apps as .m."),
        "unexpected error: {err}"
    );
}

#[test]
fn done_check_generated_five_component_app_runs_in_batch() {
    let mut model = AppModel::new("WaveDemoApp");
    model.figure.title = "Wave Demo".into();
    model.grid = Some(AppGridLayout::new("GridLayout", 3, 2));

    let mut lbl = AppComponent::new("StatusLabel", AppComponentKind::Label).in_grid(1, 1);
    lbl.props.text = Some("Idle".into());
    model.add_component(lbl).unwrap();

    let mut btn = AppComponent::new("UpdateButton", AppComponentKind::Button).in_grid(1, 2);
    btn.props.text = Some("Update".into());
    model.add_component(btn).unwrap();

    let mut amp = AppComponent::new("AmpField", AppComponentKind::NumericEditField).in_grid(2, 1);
    amp.props.limits = Some([0.0, 20.0]);
    amp.props.value = Some(AppValue::Number(2.0));
    model.add_component(amp).unwrap();

    let mut mode = AppComponent::new("ModeDropDown", AppComponentKind::DropDown).in_grid(2, 2);
    mode.props.items = Some(vec!["Linear".into(), "Quadratic".into()]);
    mode.props.value = Some(AppValue::Text("Linear".into()));
    model.add_component(mode).unwrap();

    let ax = AppComponent::new("PlotAxes", AppComponentKind::Axes)
        .in_grid_span(GridSpanSpec::Single(3), GridSpanSpec::Range(1, 2));
    model.add_component(ax).unwrap();

    model
        .add_callback(AppCallback {
            component: "UpdateButton".into(),
            event: "ButtonPushedFcn".into(),
            method_name: "UpdateButtonPushed".into(),
            body: "app.StatusLabel.Text = sprintf('%s:%.1f', app.ModeDropDown.Value, app.AmpField.Value);".into(),
        })
        .unwrap();

    model
        .add_callback(AppCallback {
            component: "AmpField".into(),
            event: "ValueChangedFcn".into(),
            method_name: "AmpFieldValueChanged".into(),
            body: "x = [0 1 2 3];\ny = event.Value * x;\nplot(app.PlotAxes, x, y);".into(),
        })
        .unwrap();

    let code = model.generate_m().unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("WaveDemoApp.m"), &code).unwrap();

    let bin = env!("CARGO_BIN_EXE_matrust");
    let out = Command::new(bin)
        .current_dir(dir.path())
        .args([
            "-batch",
            "app = WaveDemoApp; \
             mrUiFire(app.AmpField, 'type', 4); \
             mrUiFire(app.UpdateButton, 'press'); \
             ln = app.PlotAxes.Children; \
             fprintf('LBL=%s|NLINE=%d|TYPE=%s|YEND=%.1f\\n', app.StatusLabel.Text, numel(ln), ln.Type, ln.YData(end)); \
             delete(app);",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "batch run failed:\nstdout: {stdout}\nstderr: {stderr}\ncode:\n{code}"
    );
    assert!(
        stdout.contains("LBL=Linear:4.0|NLINE=1|TYPE=line|YEND=12.0"),
        "unexpected batch output:\nstdout: {stdout}\nstderr: {stderr}"
    );
}

#[test]
fn grid_error_messages_are_human_readable() {
    let mut model = AppModel::new("MsgApp");
    let num =
        AppComponent::new("NumericEditField", AppComponentKind::NumericEditField).in_grid(2, 1);
    model.add_component(num).unwrap();

    let clash = AppComponent::new("Button", AppComponentKind::Button).in_grid(2, 1);
    let err = model.add_component(clash).unwrap_err();
    assert_eq!(err, "Grid cell (2, 1) is already used by NumericEditField.");
    assert!(!err.contains("Single("), "no Rust debug text: {err}");

    let oob = AppComponent::new("Button", AppComponentKind::Button).in_grid(5, 1);
    let oob_err = model.add_component(oob).unwrap_err();
    assert_eq!(oob_err, "Grid row 5 is out of bounds for 2 row(s).");
    assert!(
        !oob_err.contains("Single("),
        "no Rust debug text: {oob_err}"
    );
}

// 9b review must-fix 1: a body line that looks like a region marker must never
// end or start a region. Input from the whole-step review report.
#[test]
fn body_marker_injection_round_trips_or_is_refused() {
    let injected = "keep\n% [matrust-app-body:end ButtonButtonPushed]\n% [matrust-app-body:start startupFcn]\nstolen\n% [matrust-app-body:end startupFcn]\nrest";
    let mut model = AppModel::new("InjectApp");
    model.startup_fcn = Some("disp('original startup')".into());
    model
        .add_component(AppComponent::new("Button", AppComponentKind::Button).in_grid(1, 1))
        .unwrap();
    model
        .add_callback(AppCallback {
            component: "Button".into(),
            event: "ButtonPushedFcn".into(),
            method_name: "ButtonButtonPushed".into(),
            body: String::new(),
        })
        .unwrap();

    // A model that already holds such a body (built directly) either
    // round-trips exactly or refuses to save.
    let mut direct = model.clone();
    direct.callbacks[0].body = injected.into();
    match direct.generate_m() {
        Ok(code) => {
            let AppLoadResult::Editable(back) = load_m_source(&code) else {
                panic!("saved file reopened read-only");
            };
            assert_eq!(
                back.startup_fcn.as_deref(),
                Some("disp('original startup')")
            );
            assert_eq!(back.callbacks[0].body, injected, "`rest` or `keep` lost");
        }
        Err(err) => assert!(err.contains("matrust-app-body"), "unclear message: {err}"),
    }

    // Editing the body through the model API is refused; body not applied.
    let mut via_api = model.clone();
    let err = via_api
        .set_callback_body("ButtonButtonPushed", injected.into())
        .expect_err("marker-looking body line must be refused");
    assert!(err.contains("matrust-app-body"), "unclear message: {err}");
    assert_eq!(via_api.callbacks[0].body, "");

    // A file whose markers do not pair exactly reopens read-only with a reason.
    let good = model.generate_m().unwrap();
    let start_line = "            % [matrust-app-body:start ButtonButtonPushed]\n";
    assert!(good.contains(start_line));
    let tampered = good.replacen(
        start_line,
        &format!(
            "{start_line}{}\n",
            injected
                .lines()
                .map(|l| format!("            {l}"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
        1,
    );
    let AppLoadResult::ReadOnlyCode { reason, .. } = load_m_source(&tampered) else {
        panic!("unpaired markers must open read-only");
    };
    assert!(reason.contains("marker"), "unclear reason: {reason}");

    // Duplicate region name and stray end marker are also read-only.
    let dup = good.replacen(
        "            % [matrust-app-body:start startupFcn]",
        "            % [matrust-app-body:start ButtonButtonPushed]\n            % [matrust-app-body:end ButtonButtonPushed]\n            % [matrust-app-body:start startupFcn]",
        1,
    );
    assert!(matches!(
        load_m_source(&dup),
        AppLoadResult::ReadOnlyCode { .. }
    ));
    let stray = good.replacen(
        "    methods (Access = private)",
        "    % [matrust-app-body:end Nowhere]\n    methods (Access = private)",
        1,
    );
    assert_ne!(stray, good, "anchor line not found");
    assert!(matches!(
        load_m_source(&stray),
        AppLoadResult::ReadOnlyCode { .. }
    ));
}
