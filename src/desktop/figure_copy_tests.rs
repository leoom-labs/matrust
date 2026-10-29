#[test]
fn figure_copy_toolbar_and_dialog_preserve_view_data_and_document_state() {
    fn click_copy(app: &mut Workspace, ctx: &egui::Context, label: &str) -> egui::ColorImage {
        // A dialog's first frame only measures its content (egui sizing pass).
        frame(app, ctx, vec![]);
        let output = frame(app, ctx, vec![]);
        let pos = output.shapes.iter().find_map(|s| text_position(&s.shape, label)).unwrap();
        let mut outputs = Vec::new();
        for pressed in [true, false] {
            outputs.push(frame(app, ctx, vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
                pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE,
            }]));
        }
        finish(app);
        outputs.push(frame(app, ctx, vec![]));
        outputs.into_iter().flat_map(|o| o.platform_output.commands).find_map(|c| {
            if let egui::OutputCommand::CopyImage(image) = c { Some(image) } else { None }
        }).expect("Worker must send the prepared image to the platform")
    }
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.run("keep=[1 2 3];plot(keep,[2 1 3]);title('Copy current view');".into());
    finish(&mut app);
    let figure = app.figure.as_ref().unwrap().clone();
    let variable = app.variables["keep"].as_numeric().unwrap().data().as_ptr();
    let document = app.figure_documents[&app.current_figure].clone();
    app.view_bounds = Some([1.25, 2.75, 0.5, 3.5]);
    app.export_settings.width = 400;
    app.export_settings.height = 300;
    // The dialog snapshots view limits and shares immutable point storage.
    app.begin_figure_export();
    let job = app.figure_export.as_ref().unwrap().copy_request().unwrap();
    let Job::CopyFigure(request) = job else { panic!("Wrong job") };
    assert!((request.figure.series[0].points).same_storage(&figure.series[0].points));
    assert_eq!(request.figure.xlim, Some([1.25, 2.75]));
    assert_eq!(request.figure.ylim, Some([0.5, 3.5]));
    let expected = figure_copy::prepare(&request, &std::sync::atomic::AtomicBool::new(false)).unwrap();
    let copied = click_copy(&mut app, &ctx, "Copy image");
    assert_eq!(copied, expected);
    assert!(app.figure_export.is_none());
    assert_eq!(app.variables["keep"].as_numeric().unwrap().data().as_ptr(), variable);
    assert!(app.figure.as_ref().unwrap().same_snapshot(&figure));
    assert_eq!(app.figure_documents[&app.current_figure].dirty, document.dirty);
    assert_eq!(app.figure_documents[&app.current_figure].path, document.path);
    // Toolstrip copy uses the remembered dimensions and current figure.
    app.layout.toolstrip = matrust::session::ToolstripTab::Plots;
    let copied = click_copy(&mut app, &ctx, "Copy figure");
    assert_eq!(copied.size, [400, 300]);
    assert!(copied.pixels.iter().all(|p| p.a() == 255));
    // Invalid settings keep the export dialog editable, without queueing work.
    app.begin_figure_export();
    app.figure_export.as_mut().unwrap().settings.width = 8192;
    app.figure_export.as_mut().unwrap().settings.height = 8192;
    app.copy_export_image();
    assert!(!app.busy);
    let dialog = app.figure_export.as_ref().unwrap();
    assert!(dialog.error.is_some() && !dialog.saving);
}
