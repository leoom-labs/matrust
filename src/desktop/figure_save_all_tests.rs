fn figure_save_all_app(ctx: &egui::Context, root: &std::path::Path, count: u32) -> Workspace {
    let mut app = legacy_sample_desktop(ctx.clone(), true);
    app.run("keep=[11 22 33];".into());
    finish(&mut app);
    for id in 1..=count {
        app.run(format!(
            "figure({id});plot([1 2],[{id} {}]);title('Saved {id}');",
            id + 1
        ));
        finish(&mut app);
        app.submit(Job::SaveFigure(root.join(format!("{id}.mfig")), None));
        finish(&mut app);
        app.run(format!("title('Local {id}');"));
        finish(&mut app);
    }
    app
}
fn finish_save_all_figures(app: &mut Workspace, ctx: &egui::Context) {
    for _ in 0..24 {
        finish(app);
        assert!(
            app.figure_conflict.is_none(),
            "Unexpected conflict: {}",
            app.status
        );
        if app.figure_close.is_none() {
            break;
        }
        frame(app, ctx, vec![]);
    }
    assert!(app.figure_close.is_none() && !app.busy, "{}", app.status);
    assert!(!app.allow_close && !app.pending_quit);
}
#[test]
fn figure_save_all_menu_saves_dirty_figures_and_views_without_closing_or_copying_data() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = figure_save_all_app(&ctx, dir.path(), 3);
    app.submit(Job::SaveFigureById(1, dir.path().join("1.fig"), None));
    finish(&mut app);
    app.run("figure(1);title('Local FIG');figure(3);".into());
    finish(&mut app);
    app.submit(Job::SaveFigure(dir.path().join("3.mfig"), None));
    finish(&mut app);
    let mut external = app.all_figures[&3].clone();
    external.title = "External clean figure".into();
    matrust::files::save_figure(&dir.path().join("3.mfig"), &external).unwrap();
    let untouched = std::fs::read(dir.path().join("3.mfig")).unwrap();
    app.figure_views.insert(
        2,
        figure_files::View {
            bounds: [1.1, 1.9, 2.2, 2.8],
            auto: [false; 2],
            dirty: true,
            revision: app.figure_documents[&2].revision,
        },
    );
    let figures = app.all_figures.clone();
    let workspace = app.variables["keep"].as_numeric().unwrap().data().as_ptr();
    app.documents[0].text = "Unsaved script remains here".into();
    app.documents[0].changed();
    click_label(&mut app, &ctx, "Menu");
    click_label(&mut app, &ctx, "File");
    click_label(&mut app, &ctx, "Figures");
    click_label(&mut app, &ctx, "Save all figures");
    finish_save_all_figures(&mut app, &ctx);
    assert_eq!(app.figure_ids, vec![1, 2, 3]);
    assert_eq!(app.current_figure, 3);
    assert!(app.figure_ids.iter().all(|id| !app.figure_dirty(*id)));
    assert_eq!(std::fs::read(dir.path().join("3.mfig")).unwrap(), untouched);
    for id in [1, 2] {
        let saved =
            matrust::files::load_figure(app.figure_documents[&id].path.as_ref().unwrap()).unwrap();
        assert_eq!(
            saved.title,
            if id == 1 {
                "Local FIG".into()
            } else {
                format!("Local {id}")
            }
        );
        assert_eq!(saved.series[0].points, figures[&id].series[0].points);
        if id == 2 {
            assert_eq!(saved.xlim, Some([1.1, 1.9]));
            assert_eq!(saved.ylim, Some([2.2, 2.8]));
        }
    }
    assert!(
        figures
            .iter()
            .all(|(id, f)| (f.series[0].points).same_storage(&app.all_figures[id].series[0].points))
    );
    assert_eq!(
        workspace,
        app.variables["keep"].as_numeric().unwrap().data().as_ptr()
    );
    assert!(app.documents[0].dirty);
    assert_eq!(app.documents[0].text, "Unsaved script remains here");
    app.begin_save_all_figures();
    assert!(app.figure_close.is_none());
    assert_eq!(app.status, "No modified figures to save");
}
#[test]
fn figure_save_all_conflicts_resume_after_reload_or_rechecked_overwrite() {
    for reload in [false, true] {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let mut app = figure_save_all_app(&ctx, dir.path(), 3);
        let path = dir.path().join("2.mfig");
        let mut external = app.all_figures[&2].clone();
        external.title = "External".into();
        matrust::files::save_figure(&path, &external).unwrap();
        app.begin_save_all_figures();
        for _ in 0..2 {
            frame(&mut app, &ctx, vec![]);
            finish(&mut app);
        }
        assert_eq!(app.figure_conflict.as_ref().unwrap().id, 2);
        assert!(!app.figure_dirty(1));
        assert!(app.figure_dirty(2) && app.figure_dirty(3));
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]);
            assert!(!app.busy);
        }
        external.title = "External again".into();
        matrust::files::save_figure(&path, &external).unwrap();
        if reload {
            click_label(&mut app, &ctx, "Reload figure");
            finish(&mut app);
        } else {
            click_label(&mut app, &ctx, "Overwrite figure");
            finish(&mut app);
            assert!(app.figure_conflict.is_some());
            assert!(app.status.contains("changed again"));
            assert_eq!(
                matrust::files::load_figure(&path).unwrap().title,
                "External again"
            );
            click_label(&mut app, &ctx, "Overwrite figure");
            finish(&mut app);
        }
        finish_save_all_figures(&mut app, &ctx);
        assert_eq!(
            matrust::files::load_figure(&path).unwrap().title,
            if reload { "External again" } else { "Local 2" }
        );
        assert_eq!(
            matrust::files::load_figure(&dir.path().join("3.mfig"))
                .unwrap()
                .title,
            "Local 3"
        );
        assert_eq!(app.current_figure, 3);
        assert_eq!(app.figure_ids, vec![1, 2, 3]);
    }
}
#[test]
fn figure_save_all_cancel_preserves_completed_saves_and_unsaved_figures() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = figure_save_all_app(&ctx, dir.path(), 3);
    let path = dir.path().join("2.mfig");
    let mut external = app.all_figures[&2].clone();
    external.title = "External".into();
    matrust::files::save_figure(&path, &external).unwrap();
    click_label(&mut app, &ctx, "Menu");
    click_label(&mut app, &ctx, "File");
    click_label(&mut app, &ctx, "Figures");
    click_label(&mut app, &ctx, "Save all figures");
    for _ in 0..2 {
        frame(&mut app, &ctx, vec![]);
        finish(&mut app);
    }
    click_label(&mut app, &ctx, "Cancel");
    for _ in 0..3 {
        frame(&mut app, &ctx, vec![]);
        assert!(!app.busy);
    }
    assert!(app.figure_close.is_none() && app.figure_conflict.is_none());
    assert_eq!(
        matrust::files::load_figure(&dir.path().join("1.mfig"))
            .unwrap()
            .title,
        "Local 1"
    );
    assert_eq!(
        matrust::files::load_figure(&path).unwrap().title,
        "External"
    );
    assert_eq!(
        matrust::files::load_figure(&dir.path().join("3.mfig"))
            .unwrap()
            .title,
        "Saved 3"
    );
    assert!(app.figure_dirty(2) && app.figure_dirty(3));
    assert_eq!(app.figure_ids, vec![1, 2, 3]);
    assert!(!app.pending_quit && !app.allow_close);
}
#[test]
fn figure_save_all_io_failure_stops_later_saves_without_undoing_completed_files() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = figure_save_all_app(&ctx, dir.path(), 3);
    app.begin_save_all_figures();
    frame(&mut app, &ctx, vec![]);
    finish(&mut app);
    // Inject the same destination failure as a newly chosen folder disappearing
    // before publication, without requiring the operating-system filename picker.
    app.figure_documents.get_mut(&2).unwrap().path = Some(dir.path().join("missing/2.mfig"));
    frame(&mut app, &ctx, vec![]);
    finish(&mut app);
    assert!(app.figure_close.is_none() && app.figure_conflict.is_none());
    for _ in 0..3 {
        frame(&mut app, &ctx, vec![]);
        assert!(!app.busy);
    }
    assert_eq!(
        matrust::files::load_figure(&dir.path().join("1.mfig"))
            .unwrap()
            .title,
        "Local 1"
    );
    for id in [2, 3] {
        assert!(app.figure_dirty(id));
        assert_eq!(
            matrust::files::load_figure(&dir.path().join(format!("{id}.mfig")))
                .unwrap()
                .title,
            format!("Saved {id}")
        );
    }
    assert_eq!(app.figure_ids, vec![1, 2, 3]);
    assert!(!app.pending_quit && !app.allow_close);
}
#[test]
fn figure_save_all_compact_controls_escape_and_close_guards_work_for_sixteen_figures() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = figure_save_all_app(&ctx, dir.path(), 16);
    app.begin_save_all_figures();
    assert!(app.file_action_pending());
    app.begin_save_all_scripts();
    assert!(app.script_save_queue.is_none());
    app.request_figure_close(1);
    assert!(matches!(
        app.figure_close.as_ref().unwrap().action,
        figure_files::Closing::SaveOnly
    ));
    app.request_quit(&ctx);
    assert!(!app.pending_quit && !app.allow_close);
    // Freeze only dispatch while exercising the actual compact modal geometry.
    app.busy = true;
    let mut output = frame_at(&mut app, &ctx, egui::vec2(960., 650.), vec![]);
    for _ in 0..2 {
        output = frame_at(&mut app, &ctx, egui::vec2(960., 650.), vec![]);
    }
    let button = output
        .shapes
        .iter()
        .find_map(|s| text_position(&s.shape, "Cancel remaining figure saves"))
        .unwrap();
    assert!(egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(960., 650.)).contains(button));
    // Escape is accepted even while the current file operation is busy.
    frame_at(
        &mut app,
        &ctx,
        egui::vec2(960., 650.),
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert!(app.worker.cancel.load(Ordering::Relaxed));
    app.busy = false;
    assert!(app.figure_close.is_none());
    assert!(app.figure_ids.iter().all(|id| app.figure_dirty(*id)));
    assert_eq!(app.figure_ids.len(), 16);
    assert!(!app.allow_close && !app.pending_quit);
    for id in 1..=16 {
        assert_eq!(
            matrust::files::load_figure(&dir.path().join(format!("{id}.mfig")))
                .unwrap()
                .title,
            format!("Saved {id}")
        );
    }
}
