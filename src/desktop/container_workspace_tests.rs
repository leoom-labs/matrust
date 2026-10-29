use super::*;
use std::time::{Duration, Instant};

fn finish_container_job(app: &mut Workspace) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while app.busy && Instant::now() < deadline {
        app.receive();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!app.busy, "{}", app.status);
}
fn text_shapes(shape: &egui::Shape, output: &mut Vec<String>) {
    match shape {
        egui::Shape::Text(text) => output.push(text.galley.text().into()),
        egui::Shape::Vec(shapes) => {
            for s in shapes {
                text_shapes(s, output);
            }
        }
        _ => {}
    }
}
#[test]
fn container_worker_snapshots_typed_inspector_and_metadata_edits_preserve_payloads() {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.run("A=[1 2 3];C={A,'label'};Alias=C;s=char([65 55296]);".into());
    finish_container_job(&mut app);
    assert!(app.variables["C"].same_snapshot(&app.variables["Alias"]));
    let original = app.variables.clone();
    for name in ["C", "s"] {
        app.inspect_variable(name.into());
        for _ in 0..4 {
            let _ = crate::headless::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(960., 650.),
                    )),
                    ..Default::default()
                },
                |ctx| app.variable_editor(ctx),
            );
        }
        let output = crate::headless::run(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(960., 650.),
                )),
                ..Default::default()
            },
            |ctx| app.variable_editor(ctx),
        );
        let mut text = Vec::new();
        for s in output.shapes {
            text_shapes(&s.shape, &mut text);
        }
        if name == "C" {
            assert!(
                text.iter()
                    .any(|s| s.starts_with("Open numeric or character cell contents to edit"))
            );
            assert!(text.iter().any(|s| s == "Open {1,1}"));
            assert!(!text.iter().any(|s| s == "Copy cell" || s == "Apply cell"));
        } else {
            assert!(text.iter().any(|s| s == "\\uD800"));
            assert!(text.iter().any(|s| s == "Copy cell"));
            assert!(text.iter().any(|s| s == "Apply cell"));
        }
        assert_eq!(
            workspace_type_label(&app.variables[name]),
            if name == "C" { "cell" } else { "char" }
        );
    }
    app.submit(Job::SetCell("C".into(), 0, 0, matrust::Scalar::Real(99.)));
    finish_container_job(&mut app);
    assert!(app.variables["C"].same_snapshot(&original["C"]));
    app.submit(Job::EditWorkspace(Box::new(workspace_edit::Request {
        targets: std::collections::BTreeMap::from([("C".into(), original["C"].clone())]),
        action: workspace_edit::Action::Rename("Renamed".into()),
    })));
    finish_container_job(&mut app);
    assert!(!app.variables.contains_key("C"));
    assert!(app.variables["Renamed"].same_snapshot(&original["C"]));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("retained.mat");
    std::fs::write(&path, b"retained").unwrap();
    app.submit(Job::SaveWorkspaceV73(path.clone()));
    finish_container_job(&mut app);
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let v73_catalog = matrust::files::inspect_workspace(&path, &cancel).unwrap();
    let v73_loaded = matrust::files::load_selected_arrays(
        &v73_catalog,
        &v73_catalog.variables().map(|v| v.name.clone()).collect(),
        &cancel,
    )
    .unwrap();
    assert_eq!(v73_loaded.len(), app.variables.len());
    let matrust::Array::Cell(cell) = &v73_loaded["Renamed"] else {
        panic!("Cell lost")
    };
    assert_eq!((cell.rows(), cell.cols()), (1, 2));
    assert_eq!(
        cell.get_linear(0).unwrap().as_numeric().unwrap().data(),
        &[1., 2., 3.]
    );
    let matrust::Array::Character(label) = cell.get_linear(1).unwrap().as_ref() else {
        panic!("Label lost")
    };
    assert_eq!(label.units(), &[108, 97, 98, 101, 108]);
    let matrust::Array::Character(text) = &v73_loaded["s"] else {
        panic!("UTF16 lost")
    };
    assert_eq!(text.units(), &[65, 55296]);
    assert!(app.variables["Renamed"].same_snapshot(&original["C"]));
    app.submit(Job::SaveWorkspace(path.clone()));
    finish_container_job(&mut app);
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let catalog = matrust::files::inspect_workspace(&path, &cancel).unwrap();
    let loaded = matrust::files::load_selected_arrays(
        &catalog,
        &catalog.variables().map(|v| v.name.clone()).collect(),
        &cancel,
    )
    .unwrap();
    assert_eq!(loaded["Renamed"].class(), "cell");
    assert_eq!(loaded["s"].preview(64), original["s"].preview(64));
    assert!(app.variables["Renamed"].same_snapshot(&original["C"]));
    app.submit(Job::EditWorkspace(Box::new(workspace_edit::Request {
        targets: std::collections::BTreeMap::from([("Renamed".into(), original["C"].clone())]),
        action: workspace_edit::Action::Remove { all: false },
    })));
    finish_container_job(&mut app);
    assert!(!app.variables.contains_key("Renamed"));
    for name in ["A", "Alias", "s"] {
        assert!(app.variables[name].same_snapshot(&original[name]));
    }
}

#[test]
fn v73_worker_still_rejects_graphics_descendants_without_replacing_destination() {
    let mut app = Workspace::with_context(egui::Context::default(), true);
    app.run("h=figure(7);G={h};".into());
    finish_container_job(&mut app);
    let original = app.variables.clone();
    assert!(matches!(original["h"], matrust::Array::Graphics(_)));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("retained.mat");
    std::fs::write(&path, b"retained").unwrap();
    app.submit(Job::SaveWorkspaceV73(path.clone()));
    finish_container_job(&mut app);
    assert_eq!(std::fs::read(&path).unwrap(), b"retained");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    for (name, value) in &original {
        assert!(app.variables[name].same_snapshot(value));
    }
}
