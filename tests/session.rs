use matrust::{
    document::Document,
    session::{self, History, Session},
    simulation::{Block, Model},
};
#[path = "../src/desktop/recovery.rs"]
mod recovery;
fn snapshot() -> Session {
    Session {
        layout_presets: Default::default(),
        export: Default::default(),
        workspace: None,
        workspace_manifest: None,
        preview: None,
        eps_renderer: None,
        figure_manifest: None,
        figures: None,
        layout: Default::default(),
        version: 1,
        documents: vec![Document::new()],
        active: 0,
        folder: std::env::current_dir().unwrap(),
        recent: vec![],
        history: vec![],
        model: Model::default(),
        model_path: None,
        model_baseline: None,
    }
}

#[test]
fn desktop_scale_recovery_bounds_preserve_other_preferences_and_unsaved_text() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let mut state = snapshot();
    state.documents[0].text = "unsaved = 42;".into();
    state.documents[0].changed();
    let mut json = serde_json::to_value(&state).unwrap();
    for (percent, expected) in [
        (75, 75),
        (100, 100),
        (125, 125),
        (150, 150),
        (0, 100),
        (74, 100),
        (151, 100),
        (65535, 100),
    ] {
        json["layout"] =
            serde_json::json!({"mode":"figure", "sidebar_width":300., "ui_scale_percent":percent});
        std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
        let restored = Session::load(&path).unwrap();
        assert_eq!(restored.layout.ui_scale_percent(), expected);
        assert_eq!(restored.layout.mode, session::PaneMode::Figure);
        assert_eq!(restored.layout.sidebar_width, Some(300.));
        assert_eq!(restored.documents[0].text, "unsaved = 42;");
        assert!(restored.documents[0].dirty);
    }
    json["layout"]
        .as_object_mut()
        .unwrap()
        .remove("ui_scale_percent");
    std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
    assert_eq!(Session::load(&path).unwrap().layout.ui_scale_percent(), 100);
}

#[test]
fn preview_recovery_reference_roundtrips_and_malformed_metadata_keeps_scripts() {
    use session::{PreviewFormat, PreviewState};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let mut state = snapshot();
    state.documents[0].text = "unsaved = 42;".into();
    state.documents[0].changed();
    state.preview = Some(PreviewState {
        navigation: Default::default(),
        path: dir.path().join("missing.TIFF"),
        page: 2,
        fit: false,
        zoom: 1.25,
        format: PreviewFormat::Tiff,
    });
    state.clone().save(&path).unwrap();
    assert_eq!(Session::load(&path).unwrap().preview, state.preview);
    let original = serde_json::to_value(&state).unwrap();
    for invalid in [
        serde_json::json!("bad"),
        serde_json::json!({}),
        serde_json::json!(null),
    ] {
        let mut value = original.clone();
        value["preview"] = invalid;
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let loaded = Session::load(&path).unwrap();
        assert!(loaded.preview.is_none());
        assert_eq!(loaded.documents[0].text, "unsaved = 42;");
    }
    for (key, value) in [
        ("path", serde_json::json!("relative.png")),
        ("page", serde_json::json!(1000)),
        ("zoom", serde_json::json!(0)),
        ("format", serde_json::json!("unknown")),
    ] {
        let mut bad = original.clone();
        bad["preview"][key] = value;
        std::fs::write(&path, serde_json::to_vec(&bad).unwrap()).unwrap();
        assert!(Session::load(&path).unwrap().preview.is_none());
    }
    let mut old = original;
    old.as_object_mut().unwrap().remove("preview");
    std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
    assert!(Session::load(&path).unwrap().preview.is_none());
}

#[test]
fn layout_recovery_is_backward_compatible_and_cannot_hide_unsaved_work() {
    use session::{DesktopLayout, PaneMode};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let mut state = snapshot();
    state.documents[0].text = "unsaved_signal = 42;".into();
    state.documents[0].changed();
    let layout = DesktopLayout {
        tools: None,
        editor_columns: None,
        panels: None,
        figure_tile_size: None,
        figure_tiling: None,
        center: None,
        theme: Some(session::DesktopTheme::Dark),
        toolstrip: session::ToolstripTab::Plots,
        toolstrip_collapsed: true,
        mode: PaneMode::Editor,
        sidebar_width: Some(300.),
        workspace_width: Some(220.),
        workspace_dock: Some(session::WorkspaceDock::BelowFiles),
        left_workspace_fraction: Some(0.6),
        console_height: Some(240.),
        editor_fraction: Some(0.6),
        figure_width: None,
        ui_scale_percent: Some(150),
    };
    state.layout = layout;
    state.clone().save(&path).unwrap();
    assert_eq!(Session::load(&path).unwrap().layout, layout);
    let mut json = serde_json::to_value(&state).unwrap();
    json.as_object_mut().unwrap().remove("layout");
    std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
    assert_eq!(
        Session::load(&path).unwrap().layout,
        DesktopLayout::default()
    );
    for invalid in [
        serde_json::json!(null),
        serde_json::json!("damaged preferences"),
        serde_json::json!({"sidebar_width": "wide"}),
        serde_json::json!({"mode": {}, "console_height": 240}),
        serde_json::json!({"sidebar_width": -1, "console_height": 1e20, "editor_fraction": 9}),
    ] {
        json["layout"] = invalid;
        std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
        let loaded = Session::load(&path).unwrap();
        assert_eq!(loaded.layout, DesktopLayout::default());
        assert_eq!(loaded.documents[0].text, "unsaved_signal = 42;");
        assert!(loaded.documents[0].dirty);
    }
    json["layout"] =
        serde_json::json!({"mode": "future_mode", "sidebar_width": 300., "future_setting": true});
    std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
    let loaded = Session::load(&path).unwrap();
    assert_eq!(loaded.layout.mode, PaneMode::Split);
    assert_eq!(loaded.layout.sidebar_width, Some(300.));
    assert_eq!(loaded.layout.toolstrip, session::ToolstripTab::Home);
    assert!(!loaded.layout.toolstrip_collapsed);
    json["layout"] =
        serde_json::json!({"mode":"editor", "toolstrip":"future_tab", "toolstrip_collapsed":true});
    std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
    let loaded = Session::load(&path).unwrap();
    assert_eq!(loaded.layout.mode, PaneMode::Editor);
    assert_eq!(loaded.layout.toolstrip, session::ToolstripTab::Home);
    assert!(loaded.layout.toolstrip_collapsed);
    assert_eq!(loaded.documents[0].text, "unsaved_signal = 42;");
    state.layout.console_height = Some(f32::INFINITY);
    state.layout.editor_fraction = Some(f32::NAN);
    state.save(&path).unwrap();
    let normalized = Session::load(&path).unwrap().layout;
    assert_eq!(normalized.console_height, None);
    assert_eq!(normalized.editor_fraction, None);
    assert_eq!(normalized.sidebar_width, Some(300.));
}

#[test]
fn recovery_preserves_unsaved_edits_without_overwriting_source() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("script.m");
    std::fs::write(&source, "x=1;").unwrap();
    let mut state = snapshot();
    state.documents[0] = Document::open(&source).unwrap();
    state.documents[0].text = "% αβ\nx=2;".into();
    state.documents[0].changed();
    state.model.nodes[3].inputs = vec![usize::MAX];
    let path = dir.path().join("session.json");
    state.save(&path).unwrap();
    let loaded = Session::load(&path).unwrap();
    assert_eq!(loaded.documents[0].text, "% αβ\nx=2;");
    assert!(loaded.documents[0].dirty);
    assert_eq!(std::fs::read_to_string(source).unwrap(), "x=1;");
    assert!(loaded.model.validate().is_err());
}
#[test]
fn clean_recovery_reads_external_file_changes_but_dirty_buffers_survive() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("script.m");
    let path = dir.path().join("session.json");
    std::fs::write(&source, "x=1;").unwrap();
    let mut state = snapshot();
    state.documents[0] = Document::open(&source).unwrap();
    state.clone().save(&path).unwrap();
    std::fs::write(&source, "x=9;").unwrap();
    assert_eq!(Session::load(&path).unwrap().documents[0].text, "x=9;");
    state.documents[0].text = "x=2;".into();
    state.documents[0].changed();
    state.save(&path).unwrap();
    assert_eq!(Session::load(&path).unwrap().documents[0].text, "x=2;");
}
#[test]
fn failed_snapshot_keeps_previous_recovery_and_nonfinite_models_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    snapshot().save(&path).unwrap();
    let previous = std::fs::read(&path).unwrap();
    let mut bad = snapshot();
    bad.active = 99;
    assert!(bad.save(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), previous);
    let mut bad = snapshot();
    bad.model.nodes[0].block = Block::Constant { value: f64::NAN };
    assert!(bad.save(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), previous);
    std::fs::write(&path, "{\"version\":999}").unwrap();
    assert!(Session::load(&path).is_err());
}

#[test]
fn escaped_json_cannot_grow_recovery_past_its_byte_limit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    snapshot().save(&path).unwrap();
    let before = std::fs::read(&path).unwrap();
    let mut state = snapshot();
    state.documents.clear();
    for _ in 0..8 {
        let mut doc = Document::new();
        doc.text = "\0".repeat(1_000_000);
        doc.changed();
        state.documents.push(doc);
    }
    assert!(state.save(&path).unwrap_err().contains("32 MiB"));
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn incomplete_models_can_be_saved_reopened_and_recovered_with_file_identity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("draft.mrsim");
    let mut model = Model::default();
    model.nodes[3].inputs = vec![usize::MAX];
    model.save(&path).unwrap();
    let loaded = Model::load(&path).unwrap();
    assert!(loaded.validate().is_err());
    assert_eq!(loaded.nodes[3].inputs, vec![usize::MAX]);
    let mut state = snapshot();
    state.model_path = Some(path.clone());
    state.model_baseline = Some(loaded.clone());
    state.model = loaded;
    state.model.steps = 300;
    let session_path = dir.path().join("session.json");
    state.save(&session_path).unwrap();
    let restored = Session::load(&session_path).unwrap();
    assert_eq!(restored.model_path, Some(path));
    assert_eq!(restored.model.steps, 300);
    assert_eq!(restored.model_baseline.unwrap().steps, 1000);
}
#[test]
fn recovery_has_one_owner_and_flush_orders_after_queued_autosaves() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let writer = recovery::Recovery::open(path.clone()).unwrap();
    assert!(recovery::Recovery::open(path.clone()).is_err());
    assert!(writer.queue(snapshot()));
    let mut final_state = snapshot();
    final_state.documents[0].text = "answer=42;".into();
    final_state.documents[0].changed();
    writer.flush(final_state).unwrap();
    assert!(writer.errors.try_recv().is_err());
    assert_eq!(
        Session::load(&path).unwrap().documents[0].text,
        "answer=42;"
    );
    drop(writer);
    assert!(recovery::Recovery::open(path).is_ok());
}
#[test]
fn history_navigation_retains_draft_and_bounds_storage() {
    let mut history = History::default();
    assert!(history.previous("draft").is_none());
    history.push("a=1".into());
    history.push("b=2".into());
    history.push("b=2".into());
    assert_eq!(history.entries.len(), 2);
    assert_eq!(history.previous("unfinished("), Some("b=2".into()));
    assert_eq!(history.previous("b=2"), Some("a=1".into()));
    assert_eq!(history.next_entry(), "b=2");
    assert_eq!(history.next_entry(), "unfinished(");
    for i in 0..300 {
        history.push(format!("x={i}"));
    }
    assert_eq!(history.entries.len(), 200);
    assert_eq!(history.entries[0], "x=100");
    history.clear();
    assert!(history.previous("").is_none());
}
#[test]
fn unicode_selection_and_cursor_completion_use_character_offsets() {
    assert_eq!(session::selected_text("αβ\nx=2;", 3..7).unwrap(), "x=2;");
    assert!(session::selected_text("αβ", 1..9).is_err());
    assert!(session::selected_text("x=1", 0..0).is_err());
    let mut command = "α + si(1)".to_string();
    let (prefix, range) = session::completion_at(&command, 6).unwrap();
    assert_eq!(prefix, "si");
    command.replace_range(range, "sin");
    assert_eq!(command, "α + sin(1)");
    let results = session::completions("sig", ["signal", "sigma", "signal"].into_iter());
    assert_eq!(results, vec!["sigma", "sign", "signal"]);
    let mut command = "sin(1)".to_string();
    let (prefix, range) = session::completion_at(&command, 2).unwrap();
    assert_eq!(prefix, "si");
    command.replace_range(range, "sin");
    assert_eq!(command, "sin(1)");
}
#[test]
fn folder_listing_sorts_directories_first_and_recent_files_are_deduplicated() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("z_directory")).unwrap();
    std::fs::write(dir.path().join("b.m"), "").unwrap();
    std::fs::write(dir.path().join("a.m"), "").unwrap();
    let entries = session::folder_entries(dir.path()).unwrap();
    assert_eq!(
        entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        vec!["z_directory", "a.m", "b.m"]
    );
    assert!(entries[0].directory);
    let mut recent = vec![];
    for i in 0..25 {
        session::remember(&mut recent, format!("{i}.m").into());
    }
    session::remember(&mut recent, "20.m".into());
    assert_eq!(recent.len(), 20);
    assert_eq!(recent[0], std::path::Path::new("20.m"));
}

#[test]
fn model_layout_session_recovers_positions_and_dirty_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("layout_session.json");
    let mut state = snapshot();
    state
        .model
        .set_layout(
            (0..state.model.nodes.len())
                .map(|i| [24. + i as f32 * 170., 40.])
                .collect(),
        )
        .unwrap();
    state.model_baseline = Some(state.model.clone());
    state.model.positions[0][1] = 80.;
    let expected = state.model.clone();
    state.save(&path).unwrap();
    let restored = Session::load(&path).unwrap();
    assert!(restored.model.same_storage(&expected));
    let file = matrust::simulation::document::ModelFile::restored(
        restored.model_path,
        restored.model_baseline,
    );
    assert!(file.dirty(&restored.model));
    assert_eq!(file.baseline().unwrap().positions[0][1], 40.);
}

#[test]
fn appearance_preferences_are_optional_and_bad_values_do_not_erase_layout_or_work() {
    use session::{DesktopTheme, PaneMode};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("appearance.json");
    let mut state = snapshot();
    state.documents[0].text = "unsaved_theme_work=42;".into();
    state.documents[0].changed();
    state.layout.mode = PaneMode::Editor;
    state.layout.sidebar_width = Some(300.);
    for preference in [
        DesktopTheme::Light,
        DesktopTheme::Dark,
        DesktopTheme::System,
    ] {
        state.layout.theme = Some(preference);
        state.clone().save(&path).unwrap();
        let loaded = Session::load(&path).unwrap();
        assert_eq!(
            loaded.layout.theme,
            (preference != DesktopTheme::System).then_some(preference)
        );
        assert_eq!(loaded.layout.sidebar_width, Some(300.));
    }
    let mut json = serde_json::to_value(&state).unwrap();
    for value in [
        serde_json::json!("future_theme"),
        serde_json::json!(42),
        serde_json::json!({"unsupported":true}),
        serde_json::json!(null),
    ] {
        json["layout"]["theme"] = value;
        std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
        let loaded = Session::load(&path).unwrap();
        assert_eq!(loaded.layout.theme, None);
        assert_eq!(loaded.layout.mode, PaneMode::Editor);
        assert_eq!(loaded.layout.sidebar_width, Some(300.));
        assert_eq!(loaded.documents[0].text, "unsaved_theme_work=42;");
        assert!(loaded.documents[0].dirty);
    }
    json["layout"].as_object_mut().unwrap().remove("theme");
    std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
    assert_eq!(Session::load(&path).unwrap().layout.theme, None);
}

#[test]
fn workspace_dock_preferences_preserve_legacy_layout_and_bad_optional_values() {
    use session::{DesktopLayout, DesktopTheme, WorkspaceDock};
    let legacy: DesktopLayout =
        serde_json::from_str(r#"{"workspace_width":265,"sidebar_width":240,"theme":"dark"}"#)
            .unwrap();
    assert_eq!(legacy.workspace_dock, None);
    assert_eq!(legacy.left_workspace_fraction, None);
    let selected = DesktopLayout {
        workspace_dock: Some(WorkspaceDock::BelowFiles),
        left_workspace_fraction: Some(0.6),
        ..legacy
    };
    assert_eq!(
        serde_json::from_str::<DesktopLayout>(&serde_json::to_string(&selected).unwrap()).unwrap(),
        selected
    );
    for bad in [
        serde_json::json!("unknown"),
        serde_json::json!({"bad":true}),
        serde_json::json!(3),
        serde_json::Value::Null,
    ] {
        let mut value = serde_json::to_value(selected).unwrap();
        value["workspace_dock"] = bad.clone();
        value["left_workspace_fraction"] = bad;
        let restored: DesktopLayout = serde_json::from_value(value).unwrap();
        assert_eq!(restored.workspace_dock, None);
        assert_eq!(restored.left_workspace_fraction, None);
        assert_eq!(restored.workspace_width, Some(265.));
        assert_eq!(restored.sidebar_width, Some(240.));
        assert_eq!(restored.theme, Some(DesktopTheme::Dark));
    }
    for boundary in [0.2_f32, 0.8_f32] {
        let layout = DesktopLayout {
            left_workspace_fraction: Some(boundary),
            ..selected
        };
        assert_eq!(
            serde_json::from_str::<DesktopLayout>(&serde_json::to_string(&layout).unwrap())
                .unwrap(),
            layout
        );
    }
    for bad in [-1., 0.19, 0.81, 2.] {
        let mut value = serde_json::to_value(selected).unwrap();
        value["left_workspace_fraction"] = serde_json::json!(bad);
        assert_eq!(
            serde_json::from_value::<DesktopLayout>(value)
                .unwrap()
                .left_workspace_fraction,
            None
        );
    }
    let mut layout = selected;
    layout.workspace_dock = Some(WorkspaceDock::Right);
    layout.left_workspace_fraction = Some(f32::INFINITY);
    layout.normalize();
    assert_eq!(layout.workspace_dock, None);
    assert_eq!(layout.left_workspace_fraction, None);
    assert_eq!(layout.workspace_width, Some(265.));
}

#[test]
fn fresh_desktop_policy_is_explicit_and_does_not_reinterpret_legacy_sessions() {
    use session::{DesktopLayout, WorkspaceDock};
    assert_eq!(DesktopLayout::default().workspace_dock, None);
    let fresh = DesktopLayout::fresh_desktop();
    assert_eq!(fresh.workspace_dock, Some(WorkspaceDock::BelowFiles));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("layout.json");
    let mut state = snapshot();
    state.layout = fresh;
    state.clone().save(&path).unwrap();
    assert_eq!(Session::load(&path).unwrap().layout, fresh);
    let mut legacy = serde_json::to_value(&state).unwrap();
    legacy.as_object_mut().unwrap().remove("layout");
    std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert_eq!(
        Session::load(&path).unwrap().layout,
        DesktopLayout::default()
    );
    legacy["layout"] = serde_json::json!({"workspace_width":265,"sidebar_width":240});
    std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let old = Session::load(&path).unwrap();
    assert_eq!(old.layout.workspace_dock, None);
    assert_eq!(old.layout.workspace_width, Some(265.));
    assert_eq!(old.layout.sidebar_width, Some(240.));
}

#[test]
fn optional_eps_renderer_is_bounded_and_does_not_hide_recovered_work() {
    let mut state = snapshot();
    state.documents[0].text = "keep = 42;".into();
    state.documents[0].changed();
    let mut json = serde_json::to_value(&state).unwrap();
    assert!(json.get("eps_renderer").is_none());
    for invalid in [
        serde_json::json!(7),
        serde_json::json!("relative.exe"),
        serde_json::json!({"unexpected":true}),
        serde_json::json!("x".repeat(32769)),
    ] {
        json["eps_renderer"] = invalid;
        let mut restored: Session = serde_json::from_value(json.clone()).unwrap();
        restored.validate().unwrap();
        assert!(restored.eps_renderer.is_none());
        assert_eq!(restored.documents[0].text, "keep = 42;");
    }
    let dir = tempfile::tempdir().unwrap();
    let renderer = dir.path().join("Ghostscript console.exe");
    state.eps_renderer = Some(renderer.clone());
    let path = dir.path().join("session.json");
    state.save(&path).unwrap();
    // Loading settings neither requires the executable to exist nor runs it.
    let restored = Session::load(&path).unwrap();
    assert_eq!(restored.eps_renderer, Some(renderer));
    assert_eq!(restored.documents[0].text, "keep = 42;");
}

#[test]
fn command_center_preferences_are_optional_tolerant_and_keep_empty_explicit_documents() {
    use session::{CenterLayout, DesktopLayout, PaneMode};
    let mut state = snapshot();
    state.documents[0].text.clear();
    state.documents[0].changed();
    state.layout = DesktopLayout {
        center: Some(CenterLayout {
            command: false,
            editor_revealed: true,
        }),
        mode: PaneMode::Editor,
        sidebar_width: Some(260.),
        theme: Some(session::DesktopTheme::Dark),
        ..DesktopLayout::fresh_desktop()
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("center.json");
    state.clone().save(&path).unwrap();
    let loaded = Session::load(&path).unwrap();
    assert_eq!(loaded.layout, state.layout);
    assert_eq!(loaded.documents[0].text, "");
    assert_eq!(loaded.documents[0].dirty, state.documents[0].dirty);
    let original = serde_json::to_value(&state).unwrap();
    for value in [
        serde_json::json!(null),
        serde_json::json!(5),
        serde_json::json!({"command":"yes","editor_revealed":true}),
        serde_json::json!({"command":true}),
        serde_json::json!({"command":true,"editor_revealed":false,"future":true}),
    ] {
        let mut json = original.clone();
        json["layout"]["center"] = value;
        std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
        let restored = Session::load(&path).unwrap();
        assert_eq!(restored.layout.center, None);
        assert_eq!(restored.layout.sidebar_width, Some(260.));
        assert_eq!(restored.layout.theme, state.layout.theme);
        assert_eq!(restored.documents[0].text, state.documents[0].text);
        assert_eq!(restored.documents[0].dirty, state.documents[0].dirty);
    }
    let mut legacy = original;
    legacy["layout"].as_object_mut().unwrap().remove("center");
    std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let loaded = Session::load(&path).unwrap();
    assert_eq!(loaded.layout.center, None);
    assert_eq!(loaded.layout.mode, PaneMode::Editor);
}

#[test]
fn independent_figure_tiling_is_optional_and_malformed_values_preserve_other_preferences() {
    use matrust::session::{DesktopLayout, DesktopTheme, FigureTiling};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tiles.json");
    for mode in [None, Some(FigureTiling::Two), Some(FigureTiling::Four)] {
        let mut state = snapshot();
        state.layout = DesktopLayout {
            figure_tiling: mode,
            theme: Some(DesktopTheme::Dark),
            sidebar_width: Some(260.),
            ..DesktopLayout::fresh_desktop()
        };
        state.clone().save(&path).unwrap();
        let restored = Session::load(&path).unwrap();
        assert_eq!(restored.layout, state.layout);
        assert_eq!(restored.documents.len(), state.documents.len());
        assert_eq!(restored.documents[0].text, state.documents[0].text);
    }
    let old: DesktopLayout =
        serde_json::from_str(r#"{"theme":"dark","sidebar_width":260}"#).unwrap();
    assert_eq!(old.figure_tiling, None);
    for bad in [
        serde_json::json!("eight"),
        serde_json::json!(4),
        serde_json::json!({"two":true}),
        serde_json::Value::Null,
    ] {
        let mut value = serde_json::to_value(DesktopLayout::fresh_desktop()).unwrap();
        value["figure_tiling"] = bad;
        value["theme"] = serde_json::json!("dark");
        value["sidebar_width"] = serde_json::json!(260);
        let recovered: DesktopLayout = serde_json::from_value(value).unwrap();
        assert_eq!(recovered.figure_tiling, None);
        assert_eq!(recovered.theme, Some(DesktopTheme::Dark));
        assert_eq!(recovered.sidebar_width, Some(260.));
        assert_eq!(recovered.center, DesktopLayout::fresh_desktop().center);
    }
}

#[test]
fn tile_sizing_roundtrips_and_old_or_malformed_preference_keeps_recovered_work() {
    use matrust::session::{DesktopLayout, DesktopTheme, FigureTileSize, FigureTiling};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tile-sizing.json");
    let mut state = snapshot();
    state.documents[0].text = "unsaved_keep = 42;".into();
    state.documents[0].changed();
    state.layout = DesktopLayout {
        figure_tiling: Some(FigureTiling::Four),
        figure_tile_size: Some(FigureTileSize::Readable),
        theme: Some(DesktopTheme::Dark),
        sidebar_width: Some(260.),
        ..DesktopLayout::fresh_desktop()
    };
    state.clone().save(&path).unwrap();
    assert_eq!(Session::load(&path).unwrap().layout, state.layout);
    let original = serde_json::to_value(&state).unwrap();
    for value in [
        None,
        Some(serde_json::json!("fit")),
        Some(serde_json::json!(null)),
        Some(serde_json::json!(17)),
        Some(serde_json::json!("huge")),
        Some(serde_json::json!({"readable":true})),
    ] {
        let mut json = original.clone();
        if let Some(value) = value {
            json["layout"]["figure_tile_size"] = value;
        } else {
            json["layout"]
                .as_object_mut()
                .unwrap()
                .remove("figure_tile_size");
        }
        std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
        let loaded = Session::load(&path).unwrap();
        assert_eq!(loaded.layout.figure_tile_size, None);
        assert_eq!(loaded.layout.figure_tiling, Some(FigureTiling::Four));
        assert_eq!(loaded.layout.theme, Some(DesktopTheme::Dark));
        assert_eq!(loaded.layout.sidebar_width, Some(260.));
        assert_eq!(loaded.layout.center, state.layout.center);
        assert_eq!(loaded.documents[0].text, state.documents[0].text);
        assert_eq!(loaded.documents[0].dirty, state.documents[0].dirty);
    }
    let fit = serde_json::to_value(DesktopLayout::fresh_desktop()).unwrap();
    assert!(
        fit.get("figure_tile_size").is_none(),
        "Old sessions need no migration field"
    );
}

#[test]
fn panel_intent_session_roundtrip_is_optional_tolerant_and_keeps_recovered_work() {
    use session::{PanelLayout, SidebarPanel};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("panels.json");
    let mut state = snapshot();
    state.documents[0].text = "unsaved = 42;".into();
    state.documents[0].changed();
    state.layout = session::DesktopLayout {
        center: Some(session::CenterLayout {
            command: true,
            editor_revealed: true,
        }),
        theme: Some(session::DesktopTheme::Dark),
        sidebar_width: Some(300.),
        ..session::DesktopLayout::fresh_desktop()
    };
    for sidebar in [
        SidebarPanel::Files,
        SidebarPanel::History,
        SidebarPanel::Examples,
        SidebarPanel::Hidden,
    ] {
        for (workspace, command) in [(false, false), (true, false), (false, true)] {
            let panels = PanelLayout {
                sidebar,
                workspace,
                command,
            };
            state.layout.panels = Some(panels);
            state.clone().save(&path).unwrap();
            let loaded = Session::load(&path).unwrap();
            assert_eq!(loaded.layout, state.layout);
            assert_eq!(loaded.documents[0].text, "unsaved = 42;");
            assert!(loaded.documents[0].dirty);
        }
    }
    let mut original = serde_json::to_value(&state).unwrap();
    original["layout"].as_object_mut().unwrap().remove("panels");
    let mut expected = state.layout;
    expected.panels = None;
    std::fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
    assert_eq!(Session::load(&path).unwrap().layout, expected);
    for invalid in [
        serde_json::json!(null),
        serde_json::json!("hidden"),
        serde_json::json!(false),
        serde_json::json!({}),
        serde_json::json!([]),
        serde_json::json!({"sidebar":"future_panel","workspace":false,"command":false}),
        serde_json::json!({"sidebar":"history","workspace":0,"command":false}),
        serde_json::json!({"sidebar":"examples","workspace":false}),
        serde_json::json!({"sidebar":"hidden","workspace":false,"command":false,"future":true}),
        serde_json::to_value(PanelLayout::default()).unwrap(),
    ] {
        let mut value = original.clone();
        value["layout"]["panels"] = invalid;
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let loaded = Session::load(&path).unwrap();
        assert_eq!(
            loaded.layout, expected,
            "Malformed panel intent must not discard unrelated layout"
        );
        assert_eq!(loaded.documents[0].text, "unsaved = 42;");
        assert!(loaded.documents[0].dirty);
    }
    let value = serde_json::to_value(session::DesktopLayout::fresh_desktop()).unwrap();
    assert!(value.get("panels").is_none());
}

#[test]
fn optional_editor_columns_validate_actual_document_indices_without_losing_work_or_other_layout() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("columns.json");
    let mut state = snapshot();
    state.documents[0].text = "left_dirty=1;\r\n".into();
    state.documents[0].changed();
    let mut right = Document::new();
    right.text = "right_dirty=2;\n".into();
    right.changed();
    state.documents.push(right);
    state.active = 1;
    state.layout.theme = Some(session::DesktopTheme::Dark);
    state.layout.center = Some(session::CenterLayout {
        command: false,
        editor_revealed: true,
    });
    state.layout.editor_columns = Some(session::EditorColumns { left: 0, right: 1 });
    state.clone().save(&path).unwrap();
    let restored = Session::load(&path).unwrap();
    assert_eq!(restored.layout, state.layout);
    assert_eq!(restored.active, 1);
    for i in 0..2 {
        assert_eq!(restored.documents[i].text, state.documents[i].text);
        assert!(restored.documents[i].dirty);
    }
    let mut json = serde_json::to_value(&state).unwrap();
    for invalid in [
        serde_json::json!(null),
        serde_json::json!("two"),
        serde_json::json!({}),
        serde_json::json!({"left":0,"right":0}),
        serde_json::json!({"left":0,"right":2}),
        serde_json::json!({"left":0,"right":32}),
        serde_json::json!({"left":-1,"right":1}),
        serde_json::json!({"left":0,"right":1,"extra":true}),
    ] {
        json["layout"]["editor_columns"] = invalid;
        std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
        let restored = Session::load(&path).unwrap();
        assert!(restored.layout.editor_columns.is_none());
        assert_eq!(restored.layout.center, state.layout.center);
        assert_eq!(restored.layout.theme, state.layout.theme);
        assert_eq!(restored.active, 1);
        for i in 0..2 {
            assert_eq!(restored.documents[i].text, state.documents[i].text);
            assert!(restored.documents[i].dirty);
        }
    }
    json["layout"]
        .as_object_mut()
        .unwrap()
        .remove("editor_columns");
    std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
    assert!(
        Session::load(&path)
            .unwrap()
            .layout
            .editor_columns
            .is_none()
    );
    // Structurally bounded, but the active document must belong to the shown pair.
    state.documents.push(Document::new());
    state.active = 2;
    state.save(&path).unwrap();
    assert!(
        Session::load(&path)
            .unwrap()
            .layout
            .editor_columns
            .is_none()
    );
}

#[test]
fn named_layouts_bound_names_count_and_preserve_preferences_through_replace_rename_delete() {
    use session::{DesktopLayout, DesktopTheme, EditorColumns, LayoutPresets};
    let mut presets = LayoutPresets::default();
    let layout = DesktopLayout {
        theme: Some(DesktopTheme::Dark),
        sidebar_width: Some(315.),
        editor_columns: Some(EditorColumns { left: 2, right: 7 }),
        ..DesktopLayout::fresh_desktop()
    };
    let wide_name = "\u{1f642}".repeat(64);
    presets.save(&wide_name, layout, true, true, false).unwrap();
    assert!(
        presets
            .get(&wide_name)
            .unwrap()
            .layout()
            .editor_columns
            .is_none()
    );
    assert!(presets.get(&wide_name).unwrap().split_editor());
    for name in [
        "".to_owned(),
        " ".to_owned(),
        "a\nb".to_owned(),
        "x".repeat(65),
        "\u{1f642}".repeat(65),
    ] {
        let before = presets.clone();
        assert!(presets.save(&name, layout, false, false, false).is_err());
        assert_eq!(presets, before);
    }
    for index in 1..16 {
        presets
            .save(&format!("Layout {index}"), layout, false, false, false)
            .unwrap();
    }
    assert_eq!(presets.entries().len(), 16);
    let before = presets.clone();
    assert!(
        presets
            .save("Seventeenth", layout, false, false, false)
            .is_err()
    );
    assert!(
        presets
            .save("Layout 1", DesktopLayout::default(), false, false, false)
            .is_err()
    );
    assert!(presets.rename("Layout 1", "Layout 2").is_err());
    assert_eq!(presets, before);
    presets.rename("Layout 1", "  Analysis  ").unwrap();
    assert_eq!(
        presets.get("Analysis").unwrap().layout(),
        before.get("Layout 1").unwrap().layout()
    );
    presets
        .save("Analysis", DesktopLayout::default(), false, false, true)
        .unwrap();
    assert_eq!(
        presets.get("Analysis").unwrap().layout(),
        DesktopLayout::default()
    );
    assert!(presets.remove("Analysis"));
    assert!(!presets.remove("Analysis"));
    let bytes = serde_json::to_vec(&presets).unwrap();
    assert!(bytes.len() <= session::MAX_LAYOUT_PRESET_BYTES);
    assert_eq!(
        serde_json::from_slice::<LayoutPresets>(&bytes).unwrap(),
        presets
    );
}

#[test]
fn malformed_optional_named_layouts_do_not_hide_dirty_recovery_or_old_layout() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.json");
    let mut source = snapshot();
    source.documents[0].text = "dirty_value = 42;".into();
    source.documents[0].changed();
    source.layout.sidebar_width = Some(315.);
    source
        .layout_presets
        .save("Keep", source.layout, false, false, false)
        .unwrap();
    let entry = serde_json::to_value(source.layout_presets.entries()[0].clone()).unwrap();
    for value in [
        serde_json::Value::Null,
        serde_json::json!({"unexpected":[1,2,3]}),
        serde_json::json!("malformed"),
        serde_json::json!(123),
        serde_json::Value::Array(vec![entry.clone(); 17]),
        serde_json::json!([{"name":"x","layout":false,"split_editor":false,"compact_figure":false}]),
        {
            let mut bad = entry.clone();
            bad["name"] = serde_json::json!("z".repeat(40_000));
            serde_json::json!([bad])
        },
        {
            let mut bad = entry.clone();
            bad["layout"]["editor_columns"] = serde_json::json!({"left":0,"right":1});
            serde_json::json!([bad])
        },
    ] {
        let mut json = serde_json::to_value(&source).unwrap();
        json["layout_presets"] = value;
        std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
        let recovered = Session::load(&path).unwrap();
        assert!(recovered.layout_presets.is_empty());
        assert_eq!(recovered.documents[0].text, source.documents[0].text);
        assert!(recovered.documents[0].dirty);
        assert_eq!(recovered.layout.sidebar_width, Some(315.));
    }
    let mut old = serde_json::to_value(&source).unwrap();
    old.as_object_mut().unwrap().remove("layout_presets");
    std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
    let recovered = Session::load(&path).unwrap();
    assert!(recovered.layout_presets.is_empty());
    assert!(recovered.documents[0].dirty);
    let mut mixed = serde_json::to_value(&source).unwrap();
    mixed["layout_presets"] = serde_json::json!([false, entry.clone(), entry]);
    std::fs::write(&path, serde_json::to_vec(&mixed).unwrap()).unwrap();
    assert_eq!(
        Session::load(&path).unwrap().layout_presets.entries().len(),
        1
    );
}

#[test]
fn named_layouts_use_real_background_recovery_without_changing_dirty_buffers_or_aliases() {
    use std::sync::atomic::AtomicBool;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.json");
    let mut engine = matrust::Engine::default();
    engine.execute("A=[-0 2 3];B=A;").unwrap();
    let mut source = snapshot();
    source.workspace = Some(engine.workspace());
    source.documents[0].text = "unsaved = 123;".into();
    source.documents[0].changed();
    source.history.push("A=[-0 2 3];B=A;".into());
    source
        .layout_presets
        .save(
            "Analysis",
            session::DesktopLayout {
                theme: Some(session::DesktopTheme::Dark),
                figure_tiling: Some(session::FigureTiling::Four),
                ..session::DesktopLayout::fresh_desktop()
            },
            true,
            false,
            false,
        )
        .unwrap();
    let writer = recovery::Recovery::open(path.clone()).unwrap();
    assert!(writer.queue(source.clone()));
    writer.flush(source.clone()).unwrap();
    drop(writer);
    let got = Session::load(&path).unwrap();
    assert_eq!(got.layout_presets, source.layout_presets);
    assert_eq!(got.documents[0].text, source.documents[0].text);
    assert_eq!(got.documents[0].dirty, source.documents[0].dirty);
    assert_eq!(got.history, source.history);
    let values = session::workspace::restore(
        &path,
        got.workspace_manifest.as_ref().unwrap(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let a = values["A"].as_numeric().unwrap();
    let b = values["B"].as_numeric().unwrap();
    assert_eq!(a.data().as_ptr(), b.data().as_ptr());
    assert_eq!(
        a.data().iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        vec![(-0.0_f64).to_bits(), 2.0_f64.to_bits(), 3.0_f64.to_bits()]
    );
}

#[test]
fn tool_tab_layout_round_trips_and_older_or_malformed_values_open_nothing() {
    use session::{DesktopLayout, ToolTab, ToolTabs};
    let older: DesktopLayout = serde_json::from_str(r#"{"workspace_width":265}"#).unwrap();
    assert_eq!(older.tools, None);
    assert!(!serde_json::to_string(&older).unwrap().contains("\"tools\""));
    let tools = ToolTabs {
        open: [Some(ToolTab::Model), Some(ToolTab::Designer)],
        selected: Some(ToolTab::Designer),
    };
    let saved = DesktopLayout {
        tools: Some(tools),
        ..older
    };
    let text = serde_json::to_string(&saved).unwrap();
    assert_eq!(serde_json::from_str::<DesktopLayout>(&text).unwrap(), saved);
    for bad in [
        serde_json::json!("designer"),
        serde_json::json!({"open":[null,null],"selected":null}),
        serde_json::json!({"open":["model","model"],"selected":null}),
        serde_json::json!({"open":["model",null],"selected":"designer"}),
        serde_json::json!({"open":["model",null],"selected":null,"extra":1}),
    ] {
        let mut value = serde_json::to_value(saved).unwrap();
        value["tools"] = bad;
        let restored: DesktopLayout = serde_json::from_value(value).unwrap();
        assert_eq!(restored.tools, None);
        assert_eq!(restored.workspace_width, Some(265.));
    }
}
