// App framework (apps/mod.rs): private runs, App tabs beside the two tools,
// session tabs, and the one script behind preview, export and save.
mod app_framework_tests {
    use super::*;
    use apps::{App, AppKind, Request, Runner};
    use toolkits::Tool;

    #[test]
    fn blank_expression_prompts_without_parsing() {
        for source in ["", " \t\n"] {
            assert_eq!(
                apps::single_expression(source).unwrap_err(),
                "Enter a variable name or expression"
            );
        }
        assert!(apps::single_expression("x(:, 2)").is_ok());
        assert!(apps::single_expression("x); other = 1; (x").is_err());
    }

    fn first_frame_with_hint(
        kind: AppKind,
        hint: &str,
    ) -> (Workspace, egui::Context, egui::FullOutput) {
        let ctx = egui::Context::default();
        let mut desktop = Workspace::with_context(ctx.clone(), true);
        desktop.toolkits.open_tool(Tool::App(kind));
        let output = frame(&mut desktop, &ctx, vec![]);
        let (text, status) = desktop.toolkits.apps.status(kind).unwrap();
        assert_eq!(text, hint);
        assert_eq!(status, toolkits::AppStatus::Plain);
        assert_eq!(status.color(&ctx), tokens::palette(&ctx).text_muted);
        for shape in &output.shapes {
            if let egui::Shape::Text(t) = &shape.shape {
                assert!(
                    !t.galley
                        .job
                        .sections
                        .iter()
                        .any(|s| s.format.color == tokens::palette(&ctx).error),
                    "{kind:?}: {}",
                    t.galley.text()
                );
            }
        }
        (desktop, ctx, output)
    }

    #[test]
    fn image_viewer_waits_for_a_source_on_the_first_frame() {
        let (desktop, _, _) =
            first_frame_with_hint(AppKind::ImageViewer, "Choose a workspace image");
        assert!(!desktop.toolkits.apps.image.running());
    }

    #[test]
    fn import_and_dicom_idle_hints_use_secondary_text() {
        first_frame_with_hint(AppKind::ImportTool, "Choose a file to import");
        first_frame_with_hint(AppKind::DicomBrowser, "Choose a folder and press Scan");
    }

    #[test]
    fn edited_idle_apps_show_validation_and_recover() {
        for (kind, hint, original, invalid, error) in [
            (
                AppKind::ColorThresholder,
                "Choose a workspace RGB image",
                "maskedRGBImage",
                "BW",
                "Mask and image export names must differ",
            ),
            (
                AppKind::ImageViewer,
                "Choose a workspace image",
                "image_distances",
                "viewed_image",
                "Image Viewer: export names must be distinct",
            ),
            (
                AppKind::ImportTool,
                "Choose a file to import",
                "data",
                "size",
                "Import Tool: size is a function the import calls; choose another name",
            ),
            (
                AppKind::DicomBrowser,
                "Choose a folder and press Scan",
                "dicomInfo",
                "dicomVolume",
                "Volume, metadata and collection need distinct names",
            ),
        ] {
            let (mut desktop, ctx, _) = first_frame_with_hint(kind, hint);
            let size = egui::vec2(1920., 1080.);
            click_label_at(&mut desktop, &ctx, size, original);
            for (value, expected, status) in [
                (invalid, error, toolkits::AppStatus::Error),
                (original, hint, toolkits::AppStatus::Plain),
            ] {
                frame_at(
                    &mut desktop,
                    &ctx,
                    size,
                    vec![
                        egui::Event::Key {
                            key: egui::Key::A,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::COMMAND,
                        },
                        egui::Event::Paste(value.into()),
                    ],
                );
                frame_at(&mut desktop, &ctx, size, vec![]);
                let texts = clear_panes_texts(&frame_at(&mut desktop, &ctx, size, vec![]));
                assert_eq!(
                    desktop.toolkits.apps.status(kind),
                    Some((expected.into(), status)),
                    "{kind:?}"
                );
                assert!(
                    texts
                        .iter()
                        .any(|t| t.text == expected && t.clip.contains_rect(t.glyphs)),
                    "{kind:?}: {expected}"
                );
            }
        }
    }

    #[test]
    fn empty_color_thresholder_shows_disabled_channel_ranges() {
        let (mut desktop, ctx, output) =
            first_frame_with_hint(AppKind::ColorThresholder, "Choose a workspace RGB image");
        let label_pos = |label| {
            output
                .shapes
                .iter()
                .find_map(|s| text_position(&s.shape, label))
                .unwrap_or_else(|| panic!("missing {label}"))
        };
        let red = label_pos("Red");
        let green = label_pos("Green");
        for label in ["Blue", "Minimum", "Maximum"] {
            label_pos(label);
        }
        let track = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::LineSegment { points: [a, b], .. }
                    if a.y == b.y
                        && (red.y..green.y).contains(&a.y)
                        && b.x - a.x > 100.
                        && a.x <= red.x
                        && b.x > red.x =>
                {
                    Some(a.lerp(*b, 0.3))
                }
                _ => None,
            })
            .expect("visible red channel slider track");
        let limits = desktop.toolkits.apps.color.app.limits.clone();
        for pressed in [true, false] {
            pointer_frame(&mut desktop, &ctx, egui::vec2(1440., 900.), track, pressed);
        }
        assert_eq!(desktop.toolkits.apps.color.app.limits, limits);
    }

    pub(super) fn layout_frame(
        ctx: &egui::Context,
        size: egui::Vec2,
        events: Vec<egui::Event>,
        draw: impl FnMut(&mut egui::Ui),
    ) -> egui::FullOutput {
        crate::headless::run(
            ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            draw,
        )
    }

    #[test]
    fn output_slot_contains_wide_table_and_wraps_result_text() {
        let (ctx, _desktop) = redesign_app(session::DesktopTheme::Light);
        let hits = (1..=100)
            .map(|n| format!("{n}: 1"))
            .collect::<Vec<_>>()
            .join(", ");
        for width in [1600., 900.] {
            let mut main = egui::Rect::NOTHING;
            let mut table = egui::Rect::NOTHING;
            let mut draw = |ui: &mut egui::Ui| {
                apps::three_areas(ui, AppKind::NeuralNetClustering, "Design", |ui, area| {
                    match area {
                        apps::Area::Inputs => (),
                        apps::Area::Main => {
                            main = ui.available_rect_before_wrap();
                            ui.allocate_space(main.size());
                        }
                        apps::Area::Output => {
                            apps::result_block(ui, &[("Hits by neuron", hits.clone())]);
                            if width == 1600. {
                                // Model Reducer's five-column comparison can exceed the slot.
                                table = ui.allocate_space(egui::vec2(400., 24.)).1;
                            }
                        }
                    }
                });
            };
            let size = egui::vec2(width, 1000.);
            layout_frame(&ctx, size, vec![], &mut draw);
            let output = layout_frame(&ctx, size, vec![], &mut draw);
            let texts = clear_panes_texts(&output);
            let heading = texts.iter().find(|t| t.text == "OUTPUT").unwrap();
            let values = texts.iter().find(|t| t.text == hits).unwrap();
            assert!(values.rect.height() > 2. * tokens::BODY);
            assert!(values.rect.right() <= width - tokens::L);
            if width == 1600. {
                let edge = width - apps::COLUMN;
                assert!(main.right() <= edge, "main {main:?} crossed {edge}");
                assert!(heading.rect.left() >= edge + tokens::L);
                assert!(heading.clip.contains_rect(heading.rect));
                assert!(table.right() > width, "exercise overflowing content");
            }
        }
    }

    #[test]
    fn output_labels_share_the_row_with_short_values() {
        let (ctx, _app) = redesign_app(session::DesktopTheme::Light);
        let rows = [
            ("Validation CE / % error", "0.1 / 5".into()),
            (
                "A long result label that must wrap within the Output panel",
                "125".into(),
            ),
        ];
        let mut draw = |ui: &mut egui::Ui| {
            apps::three_areas(
                ui,
                AppKind::NeuralNetPatternRecognition,
                "Training results",
                |ui, area| {
                    if area == apps::Area::Output {
                        apps::result_block(ui, &rows);
                    }
                },
            );
        };
        let size = egui::vec2(1600., 554.);
        layout_frame(&ctx, size, vec![], &mut draw);
        let texts = clear_panes_texts(&layout_frame(&ctx, size, vec![], &mut draw));
        for (i, (label, value)) in rows.iter().enumerate() {
            let label = texts.iter().find(|t| t.text == *label).unwrap();
            let value = texts.iter().find(|t| t.text == *value).unwrap();
            assert!(label.clip.contains_rect(label.rect) && value.clip.contains_rect(value.rect));
            assert!(label.rect.right() + tokens::S <= value.rect.left() + 1.);
            assert!(value.rect.height() < 2. * tokens::BODY);
            if i == 0 {
                assert_eq!(
                    label.rect.height(),
                    value.rect.height(),
                    "ordinary label wrapped"
                );
            } else {
                assert!(
                    label.rect.height() > value.rect.height(),
                    "long label should wrap"
                );
            }
        }
    }

    #[test]
    fn time_series_train_fits_wide_and_inputs_page_and_wheel_in_short_window() {
        use apps::neural_net_apps::{Mode, NeuralNet};
        let (ctx, _desktop) = redesign_app(session::DesktopTheme::Light);
        let mut app = NeuralNet::new(Mode::TimeSeries);
        let ws = apps::Snapshot::default();
        let mut draw = |ui: &mut egui::Ui| {
            apps::three_areas(ui, app.kind(), "Training results", |ui, area| {
                if area == apps::Area::Inputs {
                    app.inputs_ui(ui, &ws);
                }
            });
        };
        // The tab, toolbar and status bar occupy 96 px of the native window.
        let full = layout_frame(&ctx, egui::vec2(1600., 904.), vec![], &mut draw);
        let labels = clear_panes_texts(&full);
        for name in ["Train", "Input delays (1 to N)", "Feedback delays (1 to N)"] {
            let text = labels.iter().find(|t| t.text == name).unwrap();
            assert!(
                text.clip.contains_rect(text.rect),
                "{name}: {:?} vs {:?}",
                text.rect,
                text.clip
            );
        }
        let size = egui::vec2(900., 554.);
        let pos = egui::pos2(200., 220.);
        let y = |output: &egui::FullOutput| {
            clear_panes_texts(output)
                .into_iter()
                .find(|t| t.text == "Network")
                .map(|t| t.rect.top())
        };
        // Let the narrower viewport settle its scrollbar gutter and wrapped text.
        for _ in 0..20 {
            layout_frame(&ctx, size, vec![], &mut draw);
        }
        let initial = layout_frame(&ctx, size, vec![egui::Event::PointerMoved(pos)], &mut draw);
        let start = y(&initial).expect("network heading");
        layout_frame(
            &ctx,
            size,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::MouseWheel {
                    phase: egui::TouchPhase::Move,
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0., -48.),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            &mut draw,
        );
        // Finish the smooth wheel gesture before starting a separate key gesture.
        for _ in 0..60 {
            layout_frame(&ctx, size, vec![], &mut draw);
        }
        let moved = layout_frame(&ctx, size, vec![], &mut draw);
        assert!(
            y(&moved).unwrap() < start,
            "wheel did not move the Network heading"
        );
        layout_frame(
            &ctx,
            size,
            vec![egui::Event::Key {
                key: egui::Key::PageDown,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            &mut draw,
        );
        for _ in 0..60 {
            let output = layout_frame(&ctx, size, vec![], &mut draw);
            if clear_panes_texts(&output)
                .iter()
                .any(|t| t.text == "Train" && t.clip.contains_rect(t.rect))
            {
                return;
            }
        }
        panic!("PageDown did not reveal Train");
    }

    fn wait(runner: &mut Runner, ctx: &egui::Context) -> Result<apps::Outputs, String> {
        let deadline = Instant::now() + Duration::from_secs(20);
        while runner.result().is_none() {
            assert!(Instant::now() < deadline, "app run did not finish");
            runner.poll(ctx);
            std::thread::sleep(Duration::from_millis(2));
        }
        runner.result().unwrap().clone()
    }
    fn scalar(outputs: &apps::Outputs, name: &str) -> f64 {
        match &outputs[name] {
            matrust::Array::Numeric(m) => m.data()[0],
            other => panic!("{name}: {}", other.class()),
        }
    }

    #[test]
    fn app_framework_runner_returns_outputs_and_errors_surface() {
        let ctx = egui::Context::default();
        let mut runner = Runner::default();
        runner.submit(
            &ctx,
            Request {
                key: 1,
                inputs: vec![(
                    "x".into(),
                    matrust::Array::Numeric(matrust::Matrix::scalar(2.)),
                )],
                code: "y = 3*x; z = [1 2 3];".into(),
                outputs: vec!["y".into(), "z".into()],
            },
        );
        let outputs = wait(&mut runner, &ctx).unwrap();
        assert_eq!(scalar(&outputs, "y"), 6.);
        assert_eq!(outputs["z"].len(), 3);
        assert_eq!(outputs.len(), 2, "only the asked outputs come back");
        // The same key keeps its result instead of running again.
        runner.submit(
            &ctx,
            Request {
                key: 1,
                inputs: vec![],
                code: "error('not run')".into(),
                outputs: vec![],
            },
        );
        assert!(!runner.running() && runner.result().unwrap().is_ok());
        // The engine's own error text surfaces.
        runner.submit(
            &ctx,
            Request {
                key: 2,
                inputs: vec![],
                code: "error('app:boom', 'Boom %d', 7);".into(),
                outputs: vec![],
            },
        );
        let error = wait(&mut runner, &ctx).unwrap_err();
        assert!(error.contains("Boom 7"), "{error}");
        runner.submit(
            &ctx,
            Request {
                key: 3,
                inputs: vec![],
                code: "a = 1;".into(),
                outputs: vec!["b".into()],
            },
        );
        let error = wait(&mut runner, &ctx).unwrap_err();
        assert!(error.contains("did not create b"), "{error}");
        // An input name the workspace refuses is an error, not a panic.
        runner.submit(
            &ctx,
            Request {
                key: 4,
                inputs: vec![(
                    "1bad".into(),
                    matrust::Array::Numeric(matrust::Matrix::scalar(1.)),
                )],
                code: String::new(),
                outputs: vec![],
            },
        );
        assert!(wait(&mut runner, &ctx).is_err());
    }

    #[test]
    fn app_framework_new_key_cancels_the_running_job() {
        let ctx = egui::Context::default();
        let mut runner = Runner::default();
        let start = Instant::now();
        runner.submit(
            &ctx,
            Request {
                key: 10,
                inputs: vec![],
                code: "pause(60); late = 1;".into(),
                outputs: vec!["late".into()],
            },
        );
        std::thread::sleep(Duration::from_millis(50));
        runner.poll(&ctx);
        assert!(runner.running() && runner.result().is_none());
        runner.submit(
            &ctx,
            Request {
                key: 11,
                inputs: vec![],
                code: "v = 5;".into(),
                outputs: vec!["v".into()],
            },
        );
        let outputs = wait(&mut runner, &ctx).unwrap();
        assert_eq!(runner.key(), Some(11));
        assert_eq!(scalar(&outputs, "v"), 5.);
        assert!(!outputs.contains_key("late"));
        // The 60 s pause was cancelled, not waited for.
        assert!(start.elapsed() < Duration::from_secs(20));
        // Dropping a runner mid-run cancels and joins its job.
        let mut runner = Runner::default();
        runner.submit(
            &ctx,
            Request {
                key: 12,
                inputs: vec![],
                code: "pause(60);".into(),
                outputs: vec![],
            },
        );
        let start = Instant::now();
        drop(runner);
        assert!(start.elapsed() < Duration::from_secs(20));
        // stop() forgets the key: asking for it again runs again instead of
        // returning the cancelled job's error (inputs that went invalid and
        // came back, or a tab closed mid-run and reopened).
        let mut runner = Runner::default();
        let request = |code: &str| Request {
            key: 13,
            inputs: vec![],
            code: code.into(),
            outputs: vec!["w".into()],
        };
        runner.submit(&ctx, request("pause(60); w = 1;"));
        std::thread::sleep(Duration::from_millis(50));
        runner.stop();
        assert_eq!((runner.key(), runner.result().is_none()), (None, true));
        runner.submit(&ctx, request("w = 2;"));
        let outputs = wait(&mut runner, &ctx).unwrap();
        assert_eq!(scalar(&outputs, "w"), 2.);
    }

    #[test]
    fn app_framework_tool_tabs_order_close_and_restore_with_apps() {
        let pid = Tool::App(AppKind::PidTuner);
        let mut tools = toolkits::Toolkits::default();
        tools.open_tool(Tool::Model);
        tools.open_tool(pid);
        tools.open_tool(Tool::Designer);
        assert_eq!(
            tools.tabs().collect::<Vec<_>>(),
            [Tool::Model, pid, Tool::Designer]
        );
        assert_eq!(tools.selected_tool(), Some(Tool::Designer));
        assert_eq!(tools.tab_label(pid), "PID Tuner");
        // Unbuilt apps are listed but never open.
        for kind in AppKind::ALL.into_iter().filter(|k| !k.is_built()) {
            tools.open_tool(Tool::App(kind));
            assert!(!tools.is_open(Tool::App(kind)), "{kind:?}");
        }
        assert_eq!(
            AppKind::built().collect::<Vec<_>>(),
            [
                AppKind::CurveFitter,
                AppKind::SignalAnalyzer,
                AppKind::ImportTool,
                AppKind::PidTuner,
                AppKind::ControlSystemDesigner,
                AppKind::LinearSystemAnalyzer,
                AppKind::DistributionFitter,
                AppKind::DistributionExplorer,
                AppKind::DataCleaner,
                AppKind::Optimize,
                AppKind::SolveOde,
                AppKind::BasicFitting,
                AppKind::DataStatistics,
                AppKind::WindowDesigner,
                AppKind::SplineTool,
                AppKind::ClassificationLearner,
                AppKind::RegressionLearner,
                AppKind::ImageSegmenter,
                AppKind::ImageRegionAnalyzer,
                AppKind::ColorThresholder,
                AppKind::ImageViewer,
                AppKind::VideoViewer,
                AppKind::ImageBatchProcessor,
                AppKind::ModelReducer,
                AppKind::SystemIdentification,
                AppKind::FilterVisualizer,
                AppKind::WaveletDenoiser,
                AppKind::MultiresAnalyzer,
                AppKind::WaveletTfAnalyzer,
                AppKind::ComparisonTool,
                AppKind::Profiler,
                AppKind::NeuralNetFitting,
                AppKind::NeuralNetPatternRecognition,
                AppKind::NeuralNetClustering,
                AppKind::NeuralNetTimeSeries,
                AppKind::FuzzyLogicDesigner,
                AppKind::BitErrorRateAnalysis,
                AppKind::WirelessWaveformGenerator,
                AppKind::EconometricModeler,
                AppKind::MpcDesigner,
                AppKind::DicomBrowser,
                AppKind::RegistrationEstimator,
                AppKind::CameraCalibrator,
                AppKind::VolumeViewer,
                AppKind::AppDesigner,
                AppKind::PdeModeler,
            ]
        );
        assert_eq!(tools.selected_tool(), Some(Tool::Designer));
        // Closing the shown app hands the area to the first open tab.
        tools.select_tool(pid);
        tools.close_tool(pid);
        assert!(!tools.is_open(pid));
        assert_eq!(tools.selected_tool(), Some(Tool::Model));
        assert_eq!(
            tools.tabs().collect::<Vec<_>>(),
            [Tool::Model, Tool::Designer]
        );
        // Reopened, it moves to the end like an editor tab.
        tools.open_tool(pid);
        assert_eq!(
            tools.tabs().collect::<Vec<_>>(),
            [Tool::Model, Tool::Designer, pid]
        );
        // Session tabs keep the order and the shown app.
        let layout = tools.tab_layout().unwrap();
        use matrust::session::ToolTab;
        assert_eq!(
            layout.open.iter().collect::<Vec<_>>(),
            [ToolTab::Model, ToolTab::Designer, ToolTab::PidTuner]
        );
        assert_eq!(layout.selected, Some(ToolTab::PidTuner));
        let text = serde_json::to_string(&session::DesktopLayout {
            tools: Some(layout),
            ..Default::default()
        })
        .unwrap();
        let back: session::DesktopLayout = serde_json::from_str(&text).unwrap();
        let mut restored = toolkits::Toolkits::default();
        assert_eq!(restored.restore_tab_layout(back.tools), Some(pid));
        assert_eq!(
            restored.tabs().collect::<Vec<_>>(),
            [Tool::Model, Tool::Designer, pid]
        );
        // Everything closed: no tabs saved.
        for tool in [Tool::Model, Tool::Designer, pid] {
            tools.close_tool(tool);
        }
        assert!(tools.tab_layout().is_none() && tools.selected_tool().is_none());
    }

    #[test]
    fn app_framework_app_tabs_survive_session_recovery_and_old_sessions_load() {
        let pid = Tool::App(AppKind::PidTuner);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        let mut app = Workspace::with_context(egui::Context::default(), true);
        app.toolkits.open_tool(Tool::Designer);
        app.toolkits.open_tool(pid);
        app.toolkits.open_tool(Tool::Model);
        app.select_tool_tab(pid);
        matrust::session::SessionStore::new(&path)
            .save(app.snapshot())
            .unwrap();
        let saved = matrust::session::Session::load(&path).unwrap();
        let mut restored = Workspace::with_context(egui::Context::default(), true);
        restored.restore_session(saved);
        assert_eq!(
            restored.toolkits.tabs().collect::<Vec<_>>(),
            [Tool::Designer, pid, Tool::Model]
        );
        assert_eq!(restored.toolkits.selected_tool(), Some(pid));
        // A session written before the apps: the two-slot form with a null.
        let mut json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        json["layout"]["tools"] = serde_json::json!({"open": ["model", null], "selected": "model"});
        std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
        let saved = matrust::session::Session::load(&path).unwrap();
        let mut restored = Workspace::with_context(egui::Context::default(), true);
        restored.restore_session(saved);
        assert_eq!(restored.toolkits.tabs().collect::<Vec<_>>(), [Tool::Model]);
        assert_eq!(restored.toolkits.selected_tool(), Some(Tool::Model));
    }

    #[test]
    fn app_framework_preview_export_and_saved_script_are_the_same_text() {
        let ws = apps::Snapshot::default();
        let mut host = apps::Host::new(apps::pid_tuner::PidTuner::default());
        let script = host.app.script(&ws).unwrap();
        assert_eq!(
            script,
            "G = tf([1], [1 3 3 1]);\nC = pidtune(G, 'PID');\nstep(feedback(C*G, 1))\n"
        );
        // Preview: the script itself, then read-only probe lines.
        let request = host.request(&ws).unwrap();
        assert!(request.code.starts_with(&script));
        assert!(request.outputs.iter().all(|name| name.starts_with("app_")));
        // Export runs the same text in the workspace, inside the
        // all-or-nothing wrapper.
        let Some(toolkits::Action::ExportScript(export)) = host.export(&ws) else {
            panic!("export action")
        };
        assert!(export.contains(&format!("try\n{}\ncatch", script.trim_end())));
        // The saved script: a header saying which app and when, then the same text.
        let when = 1_794_389_400; // 2026-11-11 09:30 UTC
        let Some(toolkits::Action::SaveAppScript(kind, saved)) = host.generate(&ws, when) else {
            panic!("generate action")
        };
        assert_eq!(kind, AppKind::PidTuner);
        assert_eq!(
            saved,
            format!("{}{script}", apps::script_header(AppKind::PidTuner, when))
        );
        assert!(saved.starts_with("% PID Tuner (MatRust app), generated 2026-11-11 09:30 UTC.\n"));
        // The key follows the text and the inputs' values.
        let mut ws = apps::Snapshot::default();
        let first = host.request(&ws).unwrap().key;
        ws.sync(&BTreeMap::from([(
            "G".to_owned(),
            matrust::Array::Numeric(matrust::Matrix::scalar(1.)),
        )]));
        // G is taken now: the plant becomes G1, a new script and key.
        assert!(host.app.script(&ws).unwrap().starts_with("G1 = tf("));
        assert_ne!(host.request(&ws).unwrap().key, first);
    }

    #[test]
    fn app_framework_default_names_do_not_clobber_and_numbers_round_trip() {
        let taken = ["C", "C1", "x"];
        assert_eq!(apps::unused_name("C", |n| taken.contains(&n)), "C2");
        assert_eq!(apps::unused_name("G", |n| taken.contains(&n)), "G");
        assert_eq!(apps::number(3.), "3");
        assert_eq!(apps::number(-0.5), "-0.5");
        let wc = 50_f64.to_radians().tan();
        assert_eq!(apps::number(wc).parse::<f64>().unwrap(), wc);
        assert_eq!(apps::short(1234.5678), "1234.6");
        assert_eq!(apps::short(0.000123456), "1.2346e-4");
        assert_eq!(apps::short(f64::INFINITY), "Inf");
        assert_eq!(apps::short(f64::NAN), "—");
    }

    #[test]
    fn app_framework_snapshot_versions_follow_values_and_lists_models() {
        let mut engine = matrust::Engine::default();
        engine
            .execute(
                "G = tf(1,[1 1]); H = zpk([],[-1 -2],3); S = ss(-1,1,1,0); K = pid(1,2); x = 4;",
            )
            .unwrap();
        let mut ws = apps::Snapshot::default();
        ws.sync(&engine.workspace());
        assert_eq!(
            ws.lti_models().collect::<Vec<_>>(),
            [("G", "tf"), ("H", "zpk"), ("S", "ss")]
        );
        let (g, x) = (ws.version("G"), ws.version("x"));
        engine.execute("x = 5;").unwrap();
        ws.sync(&engine.workspace());
        assert_eq!(
            ws.version("G"),
            g,
            "an unchanged variable keeps its version"
        );
        assert_ne!(ws.version("x"), x);
        assert_eq!(
            matrust::engine::lti_class(ws.get("K").unwrap()),
            Some("pid")
        );
        assert_eq!(matrust::engine::lti_class(ws.get("x").unwrap()), None);
    }

    /// A failed Export writes no outputs: earlier values come back, new
    /// names are cleared, the error still reaches the user.
    #[test]
    fn failed_export_restores_or_clears_exported_names() {
        let names = vec!["G".to_string(), "C".to_string()];
        let script = "G = 7;\nC = 3;\nerror('app:x', 'export failed here');\n";
        let wrapped = apps::all_or_nothing(script, &names);
        let value = |a: &matrust::Array| match a {
            matrust::Array::Numeric(m) => m.data().to_vec(),
            other => panic!("not numeric: {}", other.class()),
        };
        let mut engine = matrust::Engine::default();
        engine.execute("G = 5; other = 1;").unwrap();
        let err = engine.execute(&wrapped).expect_err("export must fail");
        assert!(err.to_string().contains("export failed here"), "{err}");
        let ws = engine.workspace();
        assert_eq!(value(ws.get("G").unwrap()), vec![5.]);
        assert!(!ws.contains_key("C"));
        assert!(ws.contains_key("other"));
        assert!(!ws.contains_key("mrexp_had") && !ws.contains_key("mrexp_old"));
        // Success leaves the outputs and no helper variables.
        let ok = apps::all_or_nothing("G = 7;\nC = 3;\n", &names);
        engine.execute(&ok).unwrap();
        let ws = engine.workspace();
        assert_eq!(value(ws.get("C").unwrap()), vec![3.]);
        assert!(!ws.contains_key("mrexp_had"));
    }

    #[test]
    fn app_framework_export_reserved_names_are_rejected() {
        let ws = apps::Snapshot::default();
        for name in [
            "mrexp_old",
            "mrexp_had",
            "matrust_export_error",
            "mrexp_complete",
        ] {
            let mut app = apps::pid_tuner::PidTuner::default();
            app.controller_edited = true;
            app.controller_name = name.into();
            let error = app.script(&ws).expect_err(name);
            assert!(error.contains("reserved"), "{error}");
            let reserved = |result: Result<(), String>| {
                let error = result.expect_err(name);
                assert!(error.contains("reserved"), "{name}: {error}");
            };
            let mut optimize = apps::optimize_app::Optimize::default();
            optimize.names[0] = name.into();
            optimize.edited[0] = true;
            reserved(optimize.output_names(&ws, &[]).map(|_| ()));
            let mut ode = apps::solve_ode_app::SolveOde::default();
            ode.names[0].text = name.into();
            ode.names[0].edited = true;
            reserved(ode.output_names(&ws).map(|_| ()));
            let mut basic = apps::basic_fitting::BasicFitting::default();
            basic.fit_names[3] = name.into();
            reserved(basic.output_names(&ws).map(|_| ()));
            let mut stats = apps::data_statistics::DataStatistics::default();
            stats.x_name = name.into();
            stats.x_edited = true;
            reserved(stats.names(&ws).map(|_| ()));
            let mut curve = apps::curve_fitter::CurveFitter::default();
            curve.fit_mut().fit_name = name.into();
            curve.fit_mut().fit_edited = true;
            reserved(curve.output_names(&ws).map(|_| ()));
            let mut signal = apps::signal_analyzer::SignalAnalyzer::default();
            signal.pxx_name = name.into();
            signal.pxx_edited = true;
            reserved(signal.spectrum_names(&ws).map(|_| ()));
            let mut distribution = apps::distribution_fitter::DistributionFitter::default();
            distribution.fits[0].object = name.into();
            distribution.fits[0].object_edited = true;
            distribution.fits[0].export = true;
            reserved(distribution.object_names(&ws).map(|_| ()));
            let mut cleaner = apps::data_cleaner::DataCleaner::default();
            cleaner.output_name = name.into();
            cleaner.output_edited = true;
            reserved(cleaner.output(&ws).map(|_| ()));
            let mut control = apps::control_system_designer::ControlSystemDesigner::default();
            control.c_name = name.into();
            control.c_edited = true;
            reserved(control.script(&ws).map(|_| ()));
            let mut linear = apps::linear_system_analyzer::LinearSystemAnalyzer::default();
            linear.expression = "tf(1,[1 1])".into();
            linear.add_expression(&ws).unwrap();
            linear.prepare(&ws);
            if let Some((request, names)) = linear.shape_request_for(&ws) {
                let key = request.key;
                let result = apps::run(&mut matrust::Engine::default(), request);
                linear.receive_shapes(key, &names, &result);
            }
            linear.export_names.insert("y".into(), (name.into(), true));
            reserved(linear.script(&ws).map(|_| ()));
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("input.csv");
            std::fs::write(&path, "x,y\n1,2\n3,4\n").unwrap();
            let mut import = apps::import_tool::ImportTool::default();
            import.choose(path);
            import.name = name.into();
            import.name_edited = true;
            reserved(import.script(&ws).map(|_| ()));
        }
    }

    #[test]
    fn app_framework_export_cancel_and_early_return_restore_workspace() {
        for stop in ["return", "pause(60)", "error('export failure')"] {
            let mut engine = matrust::Engine::default();
            engine.execute("G=5; other=[8 9];").unwrap();
            let names = vec!["G".into(), "C".into()];
            let script = apps::all_or_nothing(&format!("G=7; C=3; {stop};"), &names);
            let cancel = engine.cancellation();
            let thread = (stop == "pause(60)").then(|| {
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(100));
                    cancel.store(true, Ordering::Relaxed);
                })
            });
            let result = worker::perform(
                &mut engine,
                worker::Job::ExportAppScript(script, std::env::current_dir().unwrap()),
            );
            if let Some(thread) = thread {
                thread.join().unwrap();
            }
            assert!(result.is_err(), "{stop} must not complete Export");
            assert_eq!(engine.variables["G"].data(), &[5.], "{stop}");
            assert!(!engine.contains_array("C"), "{stop}");
            assert_eq!(engine.variables["other"].data(), &[8., 9.]);
            for name in [
                "mrexp_had",
                "mrexp_old",
                "matrust_export_error",
                "mrexp_complete",
            ] {
                assert!(!engine.contains_array(name), "left {name} after {stop}");
            }
        }
    }

    #[test]
    fn app_framework_folder_change_invalidates_completed_and_running_jobs() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        for (folder, value) in [(a.path(), 1), (b.path(), 2)] {
            std::fs::write(
                folder.join("folder_value.m"),
                format!("function v=folder_value()\nv={value};\nend\n"),
            )
            .unwrap();
        }
        let ctx = egui::Context::default();
        let mut runner = Runner::default();
        let request = |pause: &str| Request {
            key: 73,
            inputs: vec![],
            code: format!("{pause}v=folder_value();"),
            outputs: vec!["v".into()],
        };
        runner.set_folder(Some(a.path()));
        runner.submit(&ctx, request(""));
        assert_eq!(scalar(&wait(&mut runner, &ctx).unwrap(), "v"), 1.);
        runner.set_folder(Some(b.path()));
        assert!(runner.key().is_none() && runner.result().is_none());
        runner.submit(&ctx, request(""));
        assert_eq!(scalar(&wait(&mut runner, &ctx).unwrap(), "v"), 2.);
        runner.stop();
        runner.submit(&ctx, request("pause(60);"));
        runner.set_folder(Some(a.path()));
        runner.submit(&ctx, request(""));
        assert_eq!(scalar(&wait(&mut runner, &ctx).unwrap(), "v"), 1.);
    }

    #[test]
    fn app_framework_export_success_keeps_local_functions_and_cleans_bookkeeping() {
        let mut engine = matrust::Engine::default();
        engine.execute("G=5; other=[8 9];").unwrap();
        let script = apps::all_or_nothing(
            "G=twice(7); C=3;\nfunction v=twice(x)\nv=2*x;\nend\n",
            &["G".into(), "C".into()],
        );
        worker::perform(
            &mut engine,
            worker::Job::ExportAppScript(script, std::env::current_dir().unwrap()),
        )
        .unwrap();
        assert_eq!(engine.variables["G"].data(), &[14.]);
        assert_eq!(engine.variables["C"].data(), &[3.]);
        for name in [
            "mrexp_had",
            "mrexp_old",
            "matrust_export_error",
            "mrexp_complete",
        ] {
            assert!(!engine.contains_array(name), "left {name}");
        }
    }

    #[test]
    fn app_framework_cancelled_solver_exports_restore_setup_and_outputs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("longobjective.m"),
            "function v=longobjective(x)\npause(60); v=sum(x.^2);\nend\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("longrhs.m"),
            "function v=longrhs(t,y)\npause(60); v=-y;\nend\n",
        )
        .unwrap();
        let mut engine = matrust::Engine::default();
        engine
            .execute("solution=12; t=13; y=14; keep=[8 9];")
            .unwrap();
        let before = engine.workspace();
        let mut ws = apps::Snapshot::default();
        ws.sync(&before);
        let mut optimize = apps::Host::new(apps::optimize_app::Optimize::default());
        optimize.app.spec.objective.source = apps::optimize_app::Source::File;
        optimize.app.spec.objective.file = "longobjective".into();
        optimize.app.names[0] = "solution".into();
        optimize.app.edited[0] = true;
        optimize.app.run();
        optimize.app.prepare(&ws);
        let mut ode = apps::Host::new(apps::solve_ode_app::SolveOde::default());
        ode.app.spec.source = apps::solve_ode_app::Source::Function;
        ode.app.spec.function = "@(t,y) longrhs(t,y)".into();
        ode.app.spec.y0 = "1".into();
        ode.app.spec.tspan = "[0 1]".into();
        ode.app.names[0].text = "t".into();
        ode.app.names[0].edited = true;
        ode.app.names[1].text = "y".into();
        ode.app.names[1].edited = true;
        ode.app.run();
        ode.app.prepare(&ws);
        let exports = [
            (optimize.export(&ws).unwrap(), optimize.app.exported(&ws)),
            (ode.export(&ws).unwrap(), ode.app.exported(&ws)),
        ];
        for (action, names) in exports {
            let toolkits::Action::ExportScript(script) = action else {
                panic!("export action");
            };
            engine.cancellation().store(false, Ordering::Relaxed);
            let cancel = engine.cancellation();
            let thread = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(100));
                cancel.store(true, Ordering::Relaxed);
            });
            let result = worker::perform(
                &mut engine,
                worker::Job::ExportAppScript(script, dir.path().into()),
            );
            thread.join().unwrap();
            assert!(result.unwrap_err().to_lowercase().contains("cancel"));
            for name in names {
                match before.get(&name) {
                    Some(old) => assert!(
                        old.same_snapshot(&engine.array(&name).unwrap()),
                        "changed {name}"
                    ),
                    None => assert!(!engine.contains_array(&name), "left {name}"),
                }
            }
            for name in [
                "mrexp_had",
                "mrexp_old",
                "matrust_export_error",
                "mrexp_complete",
            ] {
                assert!(!engine.contains_array(name), "left {name}");
            }
        }
    }
}
