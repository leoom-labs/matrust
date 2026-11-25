// Exercise the worker-to-Command-Window path, including the UI's retention pass.
#[test]
fn output_window_small_worker_run_keeps_every_line() {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx, true);
    app.run("clc; for i=1:5, disp(i), end".into());
    finish(&mut app);
    assert_eq!(
        app.console
            .iter()
            .map(|line| line.trim())
            .collect::<Vec<_>>(),
        ["1", "2", "3", "4", "5"]
    );
}

#[test]
fn output_window_25000_disp_lines_keep_notice_and_newest_on_success_and_error() {
    for suffix in ["", "; error('after output')"] {
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.run(format!("clc; for i=1:25000, disp(i), end{suffix}"));
        finish(&mut app);
        let failed = !suffix.is_empty();
        assert_eq!(app.console.len(), 20_001);
        assert_eq!(
            app.console[0],
            format!(
                "... {} earlier output lines not shown",
                5000 + usize::from(failed)
            )
        );
        assert_eq!(app.console[1].trim(), if failed { "5002" } else { "5001" });
        let last_output = app.console.len() - 1 - usize::from(failed);
        assert_eq!(app.console[last_output].trim(), "25000");
        if failed {
            assert!(app.console.last().unwrap().starts_with("Error:"));
        }
        // Another receive/frame must not replace or remove the worker's notice.
        for _ in 0..4 {
            frame(&mut app, &ctx, vec![]);
        }
        assert!(app.console[0].starts_with("... 500"));
        app.run("disp('newest')".into());
        finish(&mut app);
        assert_eq!(app.console.len(), 20_001);
        assert_eq!(
            app.console[0],
            format!(
                "... {} earlier output lines not shown",
                5001 + usize::from(failed)
            )
        );
        assert_eq!(app.console.last().unwrap().trim(), "newest");
    }
}

#[test]
fn output_window_virtual_rows_preserve_multiline_text_and_scroll_to_notice() {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.console = (0..20_000)
        .map(|i| format!("output {i}\nsecond {i}\n"))
        .collect();
    app.console
        .insert(0, "... 5000 earlier output lines not shown".into());
    for _ in 0..8 {
        frame(&mut app, &ctx, vec![]);
    }
    let output = frame(&mut app, &ctx, vec![]);
    assert!(
        output
            .shapes
            .iter()
            .any(|s| text_position(&s.shape, "second 19999").is_some())
    );
    let text_shapes = output
        .shapes
        .iter()
        // Icon-font glyphs are text shapes too; count document/chrome text,
        // keeping the same visible-output budget as the painter-icon shell.
        .filter(|s| match &s.shape {
            egui::epaint::Shape::Text(text) => !text.galley.job.sections.iter().all(|section| {
                section.format.font_id.family == egui::FontFamily::Name(icons::FAMILY.into())
            }),
            _ => false,
        })
        .count();
    assert!(
        text_shapes < 150,
        "Only visible rows should be laid out: {text_shapes}"
    );
    let input = ctx
        .read_response(egui::Id::new("command_input"))
        .unwrap()
        .rect;
    frame(
        &mut app,
        &ctx,
        vec![
            egui::Event::PointerMoved(input.center() - egui::vec2(0., 80.)),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0., 2_000_000.),
                modifiers: egui::Modifiers::NONE,
                phase: egui::TouchPhase::Move,
            },
        ],
    );
    for _ in 0..16 {
        frame(&mut app, &ctx, vec![]);
    }
    let output = frame(&mut app, &ctx, vec![]);
    assert!(
        output
            .shapes
            .iter()
            .any(|s| text_position(&s.shape, &app.console[0]).is_some())
    );
    assert!(
        output
            .shapes
            .iter()
            .any(|s| text_position(&s.shape, "output 0").is_some())
    );
    assert!(
        output
            .shapes
            .iter()
            .any(|s| text_position(&s.shape, "second 0").is_some())
    );
}

#[test]
#[ignore = "release CPU frame-time measurement; run explicitly with --nocapture"]
fn output_window_frame_time_20000_entries() {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.console = (0..20_000).map(|i| format!("     {i}\n")).collect();
    for _ in 0..30 {
        frame(&mut app, &ctx, vec![]);
    }
    let mut timings = Vec::with_capacity(300);
    for _ in 0..300 {
        let start = Instant::now();
        std::hint::black_box(frame(&mut app, &ctx, vec![]));
        timings.push(start.elapsed().as_secs_f64() * 1000.);
    }
    timings.sort_by(f64::total_cmp);
    eprintln!(
        "20,000 entries, 1440x900, 30 warmup + 300 frames: CPU mean {:.3} ms, median {:.3} ms, p95 {:.3} ms, max {:.3} ms; headless, no GPU/presentation",
        timings.iter().sum::<f64>() / timings.len() as f64,
        timings[150],
        timings[284],
        timings[299],
    );
    assert_eq!(app.console.len(), 20_000);
}
