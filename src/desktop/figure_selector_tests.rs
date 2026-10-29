fn named_selector_setup(ctx: &egui::Context, root: &std::path::Path) -> Workspace {
    let mut app = figure_save_all_app(ctx, root, 2);
    for id in 1..=2 {
        let folder = root.join(format!("folder-{id}"));
        std::fs::create_dir(&folder).unwrap();
        app.submit(Job::SaveFigureById(id, folder.join("plot.mfig"), None));
        finish(&mut app);
    }
    app.run("figure(1);".into()); finish(&mut app);
    let original = app.figure.as_ref().unwrap().clone();
    let mut updated = original.clone(); updated.title = "Edited first plot".into();
    app.submit(Job::EditFigure(Box::new(worker::FigureEdit {
        id: 1, original, updated, view_before: None,
    })));
    finish(&mut app);
    app.run("figure(2);".into()); finish(&mut app);
    app.show_plot_pane();
    app.layout.mode = session::PaneMode::Figure;
    app
}

#[test]
fn named_selector_labels_distinguish_same_filenames_and_saved_or_untitled_figures() {
    let ctx = egui::Context::default(); let dir = tempfile::tempdir().unwrap();
    let mut app = named_selector_setup(&ctx, dir.path());
    let (one, detail_one) = app.figure_choice_label(1);
    let (two, detail_two) = app.figure_choice_label(2);
    assert_eq!(one, "1 * · plot.mfig"); assert_eq!(two, "2 · plot.mfig");
    assert!(detail_one.contains("folder-1") && detail_two.contains("folder-2"));
    assert!(detail_one.contains("Edited first plot") && detail_one.contains("unsaved changes"));
    app.run("figure(3);plot([1 2]);title('Unsaved response');figure(4);".into()); finish(&mut app);
    assert_eq!(app.figure_choice_label(3).0, "3 * · Unsaved response");
    assert_eq!(app.figure_choice_label(4).0, "4 · Untitled figure");
    assert!(app.figure_choice_label(3).1.contains("Not saved to a file"));
}

#[test]
fn named_selector_actual_popup_choice_preserves_dirty_views_history_and_shared_samples() {
    for size in [egui::vec2(1440.,900.), egui::vec2(640.,480.)] {
        let ctx = egui::Context::default(); let dir = tempfile::tempdir().unwrap();
        let mut app = named_selector_setup(&ctx, dir.path());
        let zoom = [1.1,1.8,1.2,2.5];
        app.figure_views.insert(1, figure_files::View {
            bounds: zoom, auto: [false,false], dirty: true,
            revision: app.figure_documents[&1].revision,
        });
        for _ in 0..3 { frame_at(&mut app, &ctx, size, vec![]); }
        let snapshots = app.all_figures.clone();
        let documents = app.figure_documents.clone();
        let history = app.figure_history.clone();
        let variable = app.variables["keep"].clone();
        let disk: Vec<_> = documents.values().map(|d| std::fs::read(d.path.as_ref().unwrap()).unwrap()).collect();
        click_label_at(&mut app, &ctx, size, "All figures");
        assert!(egui::Popup::is_any_open(&ctx));
        click_label_at(&mut app, &ctx, size, "1 * · plot.mfig");
        finish(&mut app);
        assert_eq!(app.current_figure, 1);
        assert_eq!(app.figure_documents, documents);
        assert_eq!(app.figure_history, history);
        assert!(app.figure_dirty(1));
        assert_eq!(app.figure_views[&1].bounds, zoom);
        assert!(app.figure_views[&1].dirty);
        assert!(variable.same_snapshot(&app.variables["keep"]));
        for (id, mut old) in snapshots {
            // Existing ScriptViewContext publishes the previously recorded GUI
            // zoom into engine metadata without copying points or adding undo.
            if id == 1 { old.xlim = Some([zoom[0],zoom[1]]); old.ylim = Some([zoom[2],zoom[3]]); }
            assert!(old.same_snapshot(&app.all_figures[&id]));
        }
        let after: Vec<_> = documents.values().map(|d| std::fs::read(d.path.as_ref().unwrap()).unwrap()).collect();
        assert_eq!(after, disk);
        assert!(app.figure_conflict.is_none() && !app.busy);
    }
}

#[test]
fn named_selector_busy_pointer_attempt_does_not_queue_navigation_or_open_popup() {
    let ctx = egui::Context::default(); let dir = tempfile::tempdir().unwrap();
    let mut app = named_selector_setup(&ctx, dir.path());
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    let original = std::mem::replace(&mut app.worker.send, send);
    app.busy = true;
    click_label(&mut app, &ctx, "All figures");
    assert!(!egui::Popup::is_any_open(&ctx));
    assert_eq!(app.current_figure, 2);
    assert!(matches!(receive.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty)));
    app.worker.send = original; app.busy = false;
}

#[test]
fn named_selector_unicode_labels_and_compact_header_stay_bounded() {
    let ctx = egui::Context::default(); let dir = tempfile::tempdir().unwrap();
    let mut app = named_selector_setup(&ctx, dir.path());
    let title = "e\u{301}📈".repeat(100);
    app.figure_documents.get_mut(&2).unwrap().path = None;
    app.all_figures.get_mut(&2).unwrap().title = title.clone();
    app.figure.as_mut().unwrap().title = title.clone();
    let (label, detail) = app.figure_choice_label(2);
    assert!(label.len() < 300 && label.ends_with('…'));
    assert!(detail.contains(&title));
    let size = egui::vec2(640.,480.);
    let mut output = frame_at(&mut app, &ctx, size, vec![]);
    for _ in 0..3 { output = frame_at(&mut app, &ctx, size, vec![]); }
    fn rect(shape: &egui::Shape, text: &str) -> Option<egui::Rect> {
        match shape {
            egui::Shape::Text(t) if t.galley.text() == text => Some(egui::Rect::from_min_size(t.pos,t.galley.size())),
            egui::Shape::Vec(v) => v.iter().find_map(|s| rect(s,text)), _ => None,
        }
    }
    // The tab strip is gone: the picker and the overflow icon share the header.
    for text in ["All figures", "Actions"] {
        let (bounds, clip) = output.shapes.iter().find_map(|s| rect(&s.shape,text).map(|r|(r,s.clip_rect))).unwrap_or_else(||panic!("Missing {text}"));
        assert!(egui::Rect::from_min_size(egui::Pos2::ZERO,size).contains_rect(bounds));
        assert!(clip.expand(0.1).contains_rect(bounds), "Clipped {text}");
        assert!(bounds.width() <= 180.);
    }
    click_label_at(&mut app, &ctx, size, "Actions");
    assert!(egui::Popup::is_any_open(&ctx));
    // Existing save and reload commands remain present in the reachable menu.
    let output = frame_at(&mut app, &ctx, size, vec![]);
    assert!(output.shapes.iter().any(|s| text_position(&s.shape,"Save figure").is_some()));
}

#[test]
fn named_selector_screenshot_hook_paints_real_choices_at_capture_and_keeps_them_open() {
    for size in [egui::vec2(1440., 900.), egui::vec2(960., 650.)] {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let mut app = named_selector_setup(&ctx, dir.path());
        app.run("figure(3);plot([1 2]);title('Unsaved response');figure(2);".into());
        finish(&mut app);
        for _ in 0..3 {
            frame_at(&mut app, &ctx, size, vec![]);
        }
        let documents = app.figure_documents.clone();
        let history = app.figure_history.clone();
        let variable = app.variables["keep"].clone();
        let disk: Vec<_> = documents.values().filter_map(|d| d.path.as_ref())
            .map(|p| std::fs::read(p).unwrap()).collect();
        let capture = dir.path().join("capture.png");
        app.screenshot_path = Some(capture.clone());
        app.screenshot_figure_selector_popup = true;
        app.screenshot_ready_frames = 0;
        app.screenshot_ready_target = 3;
        app.screenshot_requested = false;
        let mut capture_seen = false;
        for _ in 0..4 {
            let output = frame_at(&mut app, &ctx, size, vec![]);
            assert!(egui::Popup::is_any_open(&ctx));
            if app.screenshot_requested {
                capture_seen = true;
                // Non-current choices cannot be supplied by the collapsed
                // ComboBox. Require painted text inside both clip and viewport,
                // on the popup layer, not merely an open orphan memory ID.
                for id in [1, 2, 3] {
                    let label = app.figure_choice_label(id).0;
                    let point = output.shapes.iter().find_map(|s| {
                        text_position(&s.shape, &label).filter(|p| {
                            s.clip_rect.contains(*p)
                                && egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains(*p)
                                && ctx.layer_id_at(*p).is_some_and(|l| l.order == egui::Order::Foreground)
                        })
                    });
                    assert!(point.is_some(), "Missing painted popup choice {label} at capture");
                }
            }
        }
        assert!(capture_seen, "Capture readiness did not reach the ordinary three-frame target");
        assert_eq!(app.current_figure, 2);
        assert_eq!(app.figure_documents, documents);
        assert_eq!(app.figure_history, history);
        assert!(variable.same_snapshot(&app.variables["keep"]));
        let after: Vec<_> = documents.values().filter_map(|d| d.path.as_ref())
            .map(|p| std::fs::read(p).unwrap()).collect();
        assert_eq!(after, disk);
        assert!(!capture.exists()); // Headless emits a request, never a native image.
        assert!(!app.busy && app.figure_conflict.is_none());
    }
}

#[test]
fn named_selector_screenshot_popup_requires_capture_path_and_finished_recovery() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut app = named_selector_setup(&ctx, dir.path());
    app.screenshot_figure_selector_popup = true;
    for _ in 0..3 {
        frame(&mut app, &ctx, vec![]);
        assert!(!egui::Popup::is_any_open(&ctx));
    }
    app.screenshot_path = Some(dir.path().join("capture.png"));
    app.recovering_session = true;
    frame(&mut app, &ctx, vec![]);
    assert!(!egui::Popup::is_any_open(&ctx));
    app.recovering_session = false;
    frame(&mut app, &ctx, vec![]);
    assert!(egui::Popup::is_any_open(&ctx));
    assert_eq!(app.current_figure, 2);
}


#[test]
fn figure_simplification_caption_counts_only_visible_samples() {
    fn captions(shape: &egui::Shape, output: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::Shape::Text(text) if text.galley.text().contains("display simplified; saved data retains all samples") => {
                output.push((text.galley.text().to_owned(), egui::Rect::from_min_size(text.pos, text.galley.size())));
            }
            egui::Shape::Vec(shapes) => for shape in shapes { captions(shape, output); },
            _ => {}
        }
    }
    for (source, expected) in [
        ("plot([1 2 3]);hold on;plot([3 2 1],'Visible','off');", None),
        ("plot([1 2 3],'Visible','off');", None),
        ("plot([1 2 3]);hold on;plot(1:20000,'Visible','off');", None),
        ("plot(1:20000);hold on;plot([3 2 1],'Visible','off');", Some(20000)),
    ] {
        let ctx = egui::Context::default();
        let mut app = legacy_sample_desktop(ctx.clone(), true);
        app.run(source.into());
        finish(&mut app);
        app.layout.mode = session::PaneMode::Figure;
        let original = app.figure.as_ref().unwrap().clone();
        let visible: usize = original.series.iter().filter(|s| s.visible).map(|s| s.points.len()).sum();
        let displayed: usize = app.plot_points.iter().map(Vec::len).sum();
        if expected.is_some() { assert!(displayed < visible); }
        else { assert_eq!(displayed, visible); }
        for size in [egui::vec2(960., 650.), egui::vec2(1440., 900.)] {
            for _ in 0..4 { frame_at(&mut app, &ctx, size, vec![]); }
            let output = frame_at(&mut app, &ctx, size, vec![]);
            let mut found = Vec::new();
            for clipped in &output.shapes {
                let mut labels = Vec::new();
                captions(&clipped.shape, &mut labels);
                for (text, rect) in labels {
                    assert!(clipped.clip_rect.expand(0.1).contains_rect(rect), "Clipped warning: {rect:?}");
                    assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect));
                    found.push(text);
                }
            }
            let expected: Vec<_> = expected.into_iter().map(|n| format!("{n} visible samples (display simplified; saved data retains all samples)")).collect();
            assert_eq!(found, expected, "Incorrect warning for {source}");
        }
        let after = app.figure.as_ref().unwrap();
        assert!(after.same_snapshot(&original));
        for (a, b) in after.series.iter().zip(&original.series) {
            assert!((a.points).same_storage(&b.points));
        }
    }
}


#[test]
fn docked_ordinary_title_uses_theme_foreground_in_wide_and_compact_panes() {
    for selected_theme in [session::DesktopTheme::Light, session::DesktopTheme::Dark] {
        for size in [egui::vec2(1440., 900.), egui::vec2(640., 480.)] {
            let ctx = egui::Context::default();
            let mut app = legacy_sample_desktop(ctx.clone(), true);
            app.layout.theme = Some(selected_theme);
            app.apply_desktop_theme(&ctx);
            let title = if size.y >= 750. { "Signals and sampled measurements" } else { "Signal overview" };
            app.run(format!("A=[1 2 3];Alias=A;plot(A);title('{title}');xlabel('Time');ylabel('Amplitude');"));
            finish(&mut app);
            app.show_plot_pane();
            app.layout.mode = session::PaneMode::Figure;
            let original = app.figure.as_ref().unwrap().clone();
            let documents = app.figure_documents.clone();
            let history = app.figure_history.clone();
            let variables = app.variables.clone();
            let current = app.current_figure;
            assert!(original.layout.is_none());
            assert!(!app.detached.contains(current));
            for _ in 0..4 { frame_at(&mut app, &ctx, size, vec![]); }
            let focused = ctx.memory(|m| m.focused());
            let output = frame_at(&mut app, &ctx, size, vec![]);
            let appearance = original.resolve_appearance(figure_view::host_theme(&ctx));
            let expected = Color32::from_rgb(appearance.text[0], appearance.text[1], appearance.text[2]);
            theme::assert_painted_status(&output, title, expected);
            assert_eq!(ctx.memory(|m| m.focused()), focused);
            assert!(!app.busy && !app.file_action_pending());
            assert_eq!(app.current_figure, current);
            assert!(original.same_snapshot(app.figure.as_ref().unwrap()));
            assert!((original.series[0].points).same_storage(&app.figure.as_ref().unwrap().series[0].points));
            assert_eq!(app.figure_documents, documents);
            assert_eq!(app.figure_history, history);
            assert_eq!(app.variables.len(), variables.len());
            for (name, array) in variables { assert!(array.same_snapshot(&app.variables[&name])); }
            assert!(app.variables["A"].same_snapshot(&app.variables["Alias"]));
        }
    }
}


#[test]
fn docked_long_title_is_fully_painted_with_controls_and_axes_in_both_themes() {
    fn text_rect(shape: &egui::Shape, clip: egui::Rect, label: &str, found: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                assert!(!text.galley.elided,
                    "Visible title/control must not be elided: {label}; size={:?}, rows={}, ink={:?}, clip={clip:?}",
                    text.galley.size(), text.galley.rows.len(), text.visual_bounding_rect());
                let ink = text.visual_bounding_rect();
                assert!(clip.contains_rect(ink), "Full glyph ink must fit: {label}, {ink:?}, {clip:?}");
                assert!(text.galley.rows.len() <= 3);
                found.push(ink);
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes { text_rect(shape, clip, label, found); }
            }
            _ => {}
        }
    }
    fn painted(shape: &egui::Shape, clip: egui::Rect, labels: &mut Vec<(String, egui::Rect, egui::Rect)>) {
        if labels.len() >= 64 { return; }
        match shape {
            egui::Shape::Text(text) => labels.push((text.galley.text().chars().take(100).collect(), text.visual_bounding_rect(), clip)),
            egui::Shape::Vec(shapes) => { for shape in shapes { painted(shape, clip, labels); } }
            _ => {}
        }
    }
    fn visible(output: &egui::FullOutput, label: &str, case: &str) -> egui::Rect {
        let mut found = Vec::new();
        for shape in &output.shapes { text_rect(&shape.shape, shape.clip_rect, label, &mut found); }
        if found.len() != 1 {
            let mut labels = Vec::new();
            for shape in &output.shapes { painted(&shape.shape, shape.clip_rect, &mut labels); }
            assert_eq!(found.len(), 1, "Expected one visible {label:?}; {case}; bounded painted text={labels:?}");
        }
        found[0]
    }
    for selected_theme in [session::DesktopTheme::Light, session::DesktopTheme::Dark] {
        for (size, mode) in [
            (egui::vec2(960., 650.), session::PaneMode::Split),
            (egui::vec2(1440., 900.), session::PaneMode::Split),
            (egui::vec2(640., 480.), session::PaneMode::Figure),
        ] {
            for title in [
                "Compact coordinates and independent outlines",
                "Compact coordinates and independent outlines with shared immutable samples",
            ] {
                let ctx = egui::Context::default();
                let mut app = legacy_sample_desktop(ctx.clone(), true);
                app.layout.theme = Some(selected_theme);
                app.apply_desktop_theme(&ctx);
                assert_eq!(ctx.global_style().visuals.dark_mode, selected_theme == session::DesktopTheme::Dark);
                app.run(format!("A=[1 2 3];Alias=A;plot(A);title('{title}');xlabel('Time');ylabel('Amplitude');"));
                finish(&mut app);
                app.show_plot_pane();
                app.layout.mode = mode;
                let original = app.figure.as_ref().unwrap().clone();
                assert_eq!(original.xlabel, "Time");
                assert_eq!(original.ylabel, "Amplitude");
                assert_eq!(original.title, title);
                let documents = app.figure_documents.clone();
                let history = app.figure_history.clone();
                let variables = app.variables.clone();
                for _ in 0..5 { frame_at(&mut app, &ctx, size, vec![]); }
                let output = frame_at(&mut app, &ctx, size, vec![]);
                let case = format!("theme={selected_theme:?} size={size:?} mode={mode:?} title={title:?} plot_memory={:?}",
                    egui_plot::PlotMemory::load(&ctx, egui::Id::new("workspace_figure")));
                let title_rect = visible(&output, title, &case);
                let actions = visible(&output, "Actions", &case);
                // Mode choices, Reset view and the rest are in the overflow menu.
                let xlabel = visible(&output, "Time", &case);
                visible(&output, "Amplitude", &case);
                let plot = egui_plot::PlotMemory::load(&ctx, egui::Id::new("workspace_figure")).unwrap();
                let frame = *plot.transform().frame();
                assert!(frame.height() >= 40. && frame.width() >= 200., "Useful data area must remain: {case}, {frame:?}");
                assert!(title_rect.top() >= actions.bottom());
                assert!(title_rect.bottom() <= frame.top());
                assert!(xlabel.top() > actions.bottom() + 40., "Title must leave a useful plot area");
                let appearance = original.resolve_appearance(figure_view::host_theme(&ctx));
                let expected = Color32::from_rgb(appearance.text[0], appearance.text[1], appearance.text[2]);
                theme::assert_painted_status(&output, title, expected);
                assert!(original.same_snapshot(app.figure.as_ref().unwrap()));
                assert!(original.series[0].points.same_storage(&app.figure.as_ref().unwrap().series[0].points));
                assert_eq!(app.figure_documents, documents);
                assert_eq!(app.figure_history, history);
                assert_eq!(app.variables.len(), variables.len());
                for (name, value) in variables { assert!(value.same_snapshot(&app.variables[&name])); }
                assert!(app.variables["A"].same_snapshot(&app.variables["Alias"]));
                assert!(!app.busy && !app.file_action_pending());
            }
        }
    }
}
