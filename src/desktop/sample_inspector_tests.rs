fn sample_click(app: &mut Workspace, ctx: &egui::Context, plot: egui::Id, point: [f64; 2]) {
    for _ in 0..3 {
        frame(app, ctx, vec![]);
    }
    let memory = egui_plot::PlotMemory::load(ctx, plot).unwrap();
    let pos = memory
        .transform()
        .position_from_point(&PlotPoint::new(point[0], point[1]));
    for pressed in [true, false] {
        let _ = crate::headless::run(ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1440., 900.),
                )),
                events: vec![
                    egui::Event::ModifiersChanged(egui::Modifiers::CTRL),
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::CTRL,
                    },
                ],
                ..Default::default()
            },
            |root| app.ui(root),
        );
    }
    assert!(
        app.busy && app.sample_inspector.pending.is_some(),
        "Ctrl+click must submit an inspection job"
    );
    finish(app);
}
#[test]
fn sample_inspector_click_selects_original_data_copies_and_clears_without_editing() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    // Retain the compact-tab sample-inspection route after the fresh default changed.
    app.layout.workspace_dock = None;
    assert!(!app.workspace_on_left());
    app.run("x=1:1000; y=x; plot(x,y);".into());
    finish(&mut app);
    let before = app.figure.as_ref().unwrap().clone();
    let dirty = app.figure_documents[&app.current_figure].dirty;
    let variable = app.variables["x"].as_numeric().unwrap().data().as_ptr();
    // Force the display to only endpoints: picking must still use original samples.
    app.plot_points[0] = vec![PlotPoint::new(1., 1.), PlotPoint::new(1000., 1000.)];
    sample_click(
        &mut app,
        &ctx,
        egui::Id::new("workspace_figure"),
        [500., 500.],
    );
    assert_eq!(
        app.sample_inspector
            .marker(app.current_figure, app.figure.as_ref().unwrap())
            .unwrap()
            .1,
        [500., 500.]
    );
    assert!(app.figure.as_ref().unwrap().same_snapshot(&before));
    assert_eq!(app.figure_documents[&app.current_figure].dirty, dirty);
    assert_eq!(
        app.variables["x"].as_numeric().unwrap().data().as_ptr(),
        variable
    );
    for _ in 0..3 {
        frame_at(&mut app, &ctx, egui::vec2(960., 650.), vec![]);
    }
    // The right Workspace leaves compact mode using editor/figure tabs.
    // Select the actual figure before inspecting its current transform/readout.
    let compact = egui::vec2(960., 650.);
    click_label_at(&mut app, &ctx, compact, "Plot figure");
    for _ in 0..3 {
        frame_at(&mut app, &ctx, compact, vec![]);
    }
    let plot = egui_plot::PlotMemory::load(&ctx, egui::Id::new("workspace_figure")).unwrap();
    assert!(
        plot.transform().frame().height() >= 200.,
        "Sample readout must preserve usable compact plot height"
    );
    assert!(plot.transform().frame().right() <= compact.x);
    let output = frame_at(&mut app, &ctx, compact, vec![]);
    let pos = output
        .shapes
        .iter()
        .find_map(|s| text_position(&s.shape, "Copy sample"))
        .unwrap();
    let mut text = None;
    for pressed in [true, false] {
        let output = frame_at(
            &mut app,
            &ctx,
            compact,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        for command in output.platform_output.commands {
            if let egui::OutputCommand::CopyText(value) = command {
                text = Some(value);
            }
        }
    }
    assert_eq!(
        text.as_deref(),
        Some("Series\tSample\tX\tY\n1\t500\t500\t500\n")
    );
    click_label_at(&mut app, &ctx, compact, "Clear sample");
    assert!(
        app.sample_inspector
            .marker(app.current_figure, app.figure.as_ref().unwrap())
            .is_none()
    );
}
#[test]
fn sample_inspector_subplots_pick_inactive_axes_and_invalidate_replaced_data() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.run(
        "subplot(1,2,1);loglog([-100 -10 -1],[-100 -10 -1]);subplot(1,2,2);plot([1 2],[3 4]);"
            .into(),
    );
    finish(&mut app);
    let before = app.figure.as_ref().unwrap().clone();
    let mut positions: Vec<_> = before.all_axes().map(|(p, _)| p).collect();
    positions.sort();
    let left = positions[0];
    assert_ne!(before.active_position(), left);
    let plot_id = egui::Id::new(("subplot", app.current_figure, left));
    sample_click(&mut app, &ctx, plot_id, [-1., -1.]);
    let marker = app
        .sample_inspector
        .marker(app.current_figure, app.figure.as_ref().unwrap())
        .unwrap();
    assert_eq!(marker, (left, [-10., -10.]));
    assert!(app.figure.as_ref().unwrap().same_snapshot(&before));
    app.run("title('Other axes');".into());
    finish(&mut app);
    assert_eq!(
        app.sample_inspector
            .marker(app.current_figure, app.figure.as_ref().unwrap()),
        Some(marker)
    );
    app.run("subplot(1,2,1);plot([1 2],[8 9]);".into());
    finish(&mut app);
    assert!(
        app.sample_inspector
            .marker(app.current_figure, app.figure.as_ref().unwrap())
            .is_none()
    );
}
#[test]
fn sample_inspector_bad_query_preserves_selection_and_does_not_retain_point_buffers() {
    let ctx = egui::Context::default();
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.run("plot([1 2 3],[2 3 4]);".into());
    finish(&mut app);
    let position = app.figure.as_ref().unwrap().active_position();
    let query = matrust::plot::SampleQuery {
        transforms: [matrust::plot::AxisTransform::default(); 2],
        bounds: [0., 4., 0., 5.],
        pointer: [2., 3.],
        pixels: [800., 600.],
    };
    app.inspect_plot_sample(position, query);
    finish(&mut app);
    let selected = app
        .sample_inspector
        .marker(app.current_figure, app.figure.as_ref().unwrap());
    let weak = (app.figure.as_ref().unwrap().series[0].points).downgrade();
    let refs = weak.strong_count();
    let mut bad = query;
    bad.pixels[0] = 0.;
    app.inspect_plot_sample(position, bad);
    finish(&mut app);
    assert_eq!(
        app.sample_inspector
            .marker(app.current_figure, app.figure.as_ref().unwrap()),
        selected
    );
    assert_eq!(weak.strong_count(), refs);
    assert!(
        app.console
            .iter()
            .any(|line| line.contains("Sample inspection"))
    );
    let mut empty = query;
    empty.bounds = [10., 20., 10., 20.];
    empty.pointer = [15., 15.];
    app.inspect_plot_sample(position, empty);
    finish(&mut app);
    assert!(
        app.sample_inspector
            .marker(app.current_figure, app.figure.as_ref().unwrap())
            .is_none()
    );
}

#[test]
fn compact_figure_actions_keep_real_undo_redo_and_title_without_extra_rows() {
    let ctx=egui::Context::default();let mut app=legacy_sample_desktop(ctx.clone(),true);let size=egui::vec2(960.,650.);
    app.run("kept=[1 2;3 4];plot([0 1],[1 0]);title('Original title');".into());finish(&mut app);app.show_plot_pane();
    let data=app.variables["kept"].clone();let original=app.figure.clone().unwrap();let mut updated=original.clone();updated.title="Changed title".into();
    app.submit(Job::EditFigure(Box::new(worker::FigureEdit {view_before:None,id:app.current_figure,original:original.clone(),updated:updated.clone()})));finish(&mut app);
    for _ in 0..4 {frame_at(&mut app,&ctx,size,vec![]);}
    let output=frame_at(&mut app,&ctx,size,vec![]);
    assert!(output.shapes.iter().any(|s|text_position(&s.shape,"Changed title").is_some()));
    assert!(!output.shapes.iter().any(|s|text_position(&s.shape,"Undo edit").is_some()));
    let memory=egui_plot::PlotMemory::load(&ctx,egui::Id::new("workspace_figure")).unwrap();assert!(memory.transform().frame().height()>=200.);
    click_label_at(&mut app,&ctx,size,"Actions");click_label_at(&mut app,&ctx,size,"Undo edit");finish(&mut app);
    assert!(original.same_snapshot(app.figure.as_ref().unwrap()));
    click_label_at(&mut app,&ctx,size,"Actions");click_label_at(&mut app,&ctx,size,"Redo edit");finish(&mut app);
    assert!(updated.same_snapshot(app.figure.as_ref().unwrap()));assert!(data.same_snapshot(&app.variables["kept"]));
}
