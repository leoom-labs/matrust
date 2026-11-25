// End-to-end script, Runner and real egui interaction checks. Fixtures are
// the native AVI reader's existing ffmpeg-generated files; copies use tempdir.
mod video_viewer_tests {
    use super::*;
    use apps::{
        App, AppKind, Host, Snapshot,
        video_viewer::{self as vv, CACHE_CAP, Source, VideoViewer},
    };
    use matrust::engine::implay::{self, Launch};
    use std::sync::{Arc, Mutex};

    fn fixture(name: &str) -> String {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/video")
            .join(name)
            .to_string_lossy()
            .into_owned()
    }
    fn workspace(code: &str) -> (matrust::Engine, Snapshot) {
        let mut engine = matrust::Engine::default();
        engine.execute(code).unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        (engine, ws)
    }
    fn host() -> Host<VideoViewer> {
        Host::new(VideoViewer::default())
    }
    fn frame(
        h: &mut Host<VideoViewer>,
        ws: &Snapshot,
        ctx: &egui::Context,
        time: f64,
        width: f32,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Option<toolkits::Action>) {
        let mut action = None;
        let output = crate::headless::run(
            ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 800.),
                )),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| vv::show(h, ui, ws, false, &mut action),
        );
        (output, action)
    }
    fn pointer(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }
    fn click(
        h: &mut Host<VideoViewer>,
        ws: &Snapshot,
        ctx: &egui::Context,
        time: f64,
        name: &str,
    ) -> Option<toolkits::Action> {
        let pos = h.app.controls[name].center();
        frame(
            h,
            ws,
            ctx,
            time,
            1600.,
            vec![egui::Event::PointerMoved(pos), pointer(pos, true)],
        );
        frame(h, ws, ctx, time, 1600., vec![pointer(pos, false)]).1
    }
    fn wait(h: &mut Host<VideoViewer>, ws: &Snapshot, ctx: &egui::Context, time: f64) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            frame(h, ws, ctx, time, 1600., vec![]);
            if !h.running() {
                break;
            }
            assert!(Instant::now() < deadline, "Video Viewer Runner timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn open_array(h: &mut Host<VideoViewer>, ws: &Snapshot, ctx: &egui::Context, time: f64) {
        h.app.file_source = false;
        h.app.variable = "V".into();
        frame(h, ws, ctx, time, 1600., vec![]);
        click(h, ws, ctx, time, "Open");
        wait(h, ws, ctx, time);
        assert!(h.app.error.is_none(), "{:?}", h.app.error);
        assert_eq!(h.app.shown_index, 1);
        // The decode can finish in show's post-layout refresh. Draw the newly
        // available transport controls before the next simulated click.
        frame(h, ws, ctx, time, 1600., vec![]);
        assert!(h.app.controls.contains_key("Play"));
    }
    fn scalar(e: &matrust::Engine, name: &str) -> f64 {
        let matrust::Array::Numeric(m) = e.array(name).unwrap() else {
            panic!("numeric scalar")
        };
        m.data()[0]
    }
    fn assert_equal(e: &mut matrust::Engine, a: &str, b: &str) {
        e.execute(&format!("vv_equal = isequal({a}, {b});"))
            .unwrap();
        let matrust::Array::Logical(equal) = e.array("vv_equal").unwrap() else {
            panic!("logical equality")
        };
        assert_eq!(equal.data()[0], 1);
    }
    fn export_equals_saved(h: &mut Host<VideoViewer>, ws: &Snapshot, source: &matrust::Engine) {
        let script = h.app.script(ws).unwrap();
        let toolkits::Action::SaveAppScript(kind, saved) = h.generate(ws, 0).unwrap() else {
            panic!("script action")
        };
        assert_eq!(kind, AppKind::VideoViewer);
        assert!(saved.ends_with(&script));
        let mut fresh = matrust::Engine::default();
        for (name, array) in source.workspace() {
            fresh.insert_array(&name, array).unwrap();
        }
        fresh.execute(&saved).unwrap();
        let expected = fresh.array(&h.app.output_name).unwrap();
        let toolkits::Action::ExportScript(export) = h.export(ws).unwrap() else {
            panic!("export action")
        };
        let mut engine = source.clone();
        apps::execute_export(&mut engine, &export).unwrap();
        engine.insert_array("expected", expected).unwrap();
        engine
            .insert_array("preview", h.app.shown.as_ref().unwrap().array.clone())
            .unwrap();
        assert_equal(&mut engine, &h.app.output_name, "expected");
        assert_equal(&mut engine, &h.app.output_name, "preview");
    }

    #[test]
    fn video_viewer_file_and_workspace_exports_reproduce_shown_frame() {
        let ctx = egui::Context::default();
        let (source, ws) = workspace("V=reshape(uint8(0:119),2,4,3,5);");
        let mut h = host();
        open_array(&mut h, &ws, &ctx, 0.);
        h.app.seek(4);
        wait(&mut h, &ws, &ctx, 0.1);
        export_equals_saved(&mut h, &ws, &source);
        h.app.file_source = true;
        h.app.fps = f64::NAN; // Files always use their recorded rate.
        h.app.path = fixture("bgr24.avi");
        h.app.open_requested = true;
        wait(&mut h, &ws, &ctx, 0.2);
        h.app.seek(5);
        wait(&mut h, &ws, &ctx, 0.3);
        assert_eq!(h.app.current.as_ref().unwrap().properties.frames, 6);
        assert_eq!(h.app.current.as_ref().unwrap().properties.fps, 5.);
        let script = h.app.script(&ws).unwrap();
        assert!(script.contains("v = VideoReader("));
        assert!(script.contains("read(v, 5)"));
        export_equals_saved(&mut h, &ws, &source);
    }

    #[test]
    fn video_viewer_real_interactions_transport_track_export_and_generate() {
        let ctx = egui::Context::default();
        let (source, ws) = workspace("V=reshape(uint8(mod(0:359,256)),2,3,60);");
        let mut h = host();
        h.app.fps = 10.;
        open_array(&mut h, &ws, &ctx, 1.);
        click(&mut h, &ws, &ctx, 1., "Play");
        assert!(h.app.playing);
        for i in 1..=10 {
            let time = 1. + i as f64 / 10.;
            wait(&mut h, &ws, &ctx, time);
            assert!(
                h.app.index.abs_diff(i + 1) <= 1,
                "frame {} at {time}",
                h.app.index
            );
        }
        assert!(
            (h.app.achieved_fps - 10.).abs() <= 1.,
            "achieved {}",
            h.app.achieved_fps
        );
        eprintln!(
            "video_viewer play interaction achieved {:.2} fps at target 10 fps",
            h.app.achieved_fps
        );
        click(&mut h, &ws, &ctx, 2., "Pause");
        assert!(!h.app.playing);
        let paused = h.app.index;
        wait(&mut h, &ws, &ctx, 3.);
        assert_eq!(h.app.index, paused);
        click(&mut h, &ws, &ctx, 3., "Step");
        wait(&mut h, &ws, &ctx, 3.);
        assert_eq!(h.app.index, paused + 1);
        click(&mut h, &ws, &ctx, 3., "Back");
        wait(&mut h, &ws, &ctx, 3.);
        assert_eq!(h.app.index, paused);
        click(&mut h, &ws, &ctx, 3., "Last");
        wait(&mut h, &ws, &ctx, 3.);
        assert_eq!(h.app.shown_index, 60);
        click(&mut h, &ws, &ctx, 3., "First");
        wait(&mut h, &ws, &ctx, 3.);
        assert_eq!(h.app.shown_index, 1);
        let track = h.app.controls["Track"];
        let from = egui::pos2(track.left() + 8., track.center().y);
        let to = track.center();
        frame(
            &mut h,
            &ws,
            &ctx,
            3.,
            1600.,
            vec![egui::Event::PointerMoved(from), pointer(from, true)],
        );
        frame(
            &mut h,
            &ws,
            &ctx,
            3.,
            1600.,
            vec![egui::Event::PointerMoved(to)],
        );
        frame(&mut h, &ws, &ctx, 3., 1600., vec![pointer(to, false)]);
        wait(&mut h, &ws, &ctx, 3.);
        assert!((29..=32).contains(&h.app.index));
        for (key, expected) in [
            (egui::Key::Home, 1),
            (egui::Key::ArrowRight, 2),
            (egui::Key::ArrowLeft, 1),
            (egui::Key::End, 60),
        ] {
            frame(
                &mut h,
                &ws,
                &ctx,
                3.,
                1600.,
                vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            wait(&mut h, &ws, &ctx, 3.);
            assert_eq!(h.app.index, expected, "{key:?}");
            frame(
                &mut h,
                &ws,
                &ctx,
                3.,
                1600.,
                vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        for pressed in [true, false] {
            frame(
                &mut h,
                &ws,
                &ctx,
                3.,
                1600.,
                vec![egui::Event::Key {
                    key: egui::Key::Space,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        assert!(h.app.playing);
        click(&mut h, &ws, &ctx, 3., "Pause");
        wait(&mut h, &ws, &ctx, 3.);
        assert!(matches!(
            click(&mut h, &ws, &ctx, 3., "Export"),
            Some(toolkits::Action::ExportScript(_))
        ));
        assert!(matches!(
            click(&mut h, &ws, &ctx, 3., "Generate script"),
            Some(toolkits::Action::SaveAppScript(AppKind::VideoViewer, _))
        ));
        export_equals_saved(&mut h, &ws, &source);
    }

    #[test]
    fn video_viewer_corrupt_open_decode_error_and_cancel_keep_last_good_frame() {
        let ctx = egui::Context::default();
        let (source, ws) = workspace("V=reshape(uint8(0:59),2,3,10);");
        let mut h = host();
        open_array(&mut h, &ws, &ctx, 0.);
        let previous = h.app.shown.as_ref().unwrap().clone();
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.avi");
        std::fs::write(&bad, b"invalid AVI").unwrap();
        h.app.file_source = true;
        h.app.path = bad.to_string_lossy().into_owned();
        h.app.open_requested = true;
        wait(&mut h, &ws, &ctx, 1.);
        assert!(h.app.error.as_ref().unwrap().contains("VideoReader"));
        assert!(Arc::ptr_eq(&previous, h.app.shown.as_ref().unwrap()));
        assert_eq!(h.status().1, toolkits::AppStatus::Error);
        let (out, _) = frame(&mut h, &ws, &ctx, 1., 1600., vec![]);
        assert!(!out.shapes.is_empty());
        // Cancel a submitted open before delivering its result; even a fast
        // decoder's completed-but-unpolled delivery is retired by Runner.
        h.app.path = fixture("mjpeg.avi");
        h.app.open_requested = true;
        h.app.task_gate = Some(Arc::new(std::sync::atomic::AtomicBool::new(false)));
        frame(&mut h, &ws, &ctx, 1., 1600., vec![]);
        assert!(h.running());
        frame(&mut h, &ws, &ctx, 1., 1600., vec![]);
        click(&mut h, &ws, &ctx, 1., "Cancel");
        h.app.task_gate = None;
        wait(&mut h, &ws, &ctx, 2.);
        assert!(matches!(
            h.app.current.as_ref().unwrap().source,
            Source::Variable { .. }
        ));
        assert!(Arc::ptr_eq(&previous, h.app.shown.as_ref().unwrap()));
        // Open a valid temp copy, then truncate it after metadata/first frame
        // have succeeded. A subsequent uncached read reports the engine error.
        let file = dir.path().join("later-corrupt.avi");
        std::fs::copy(fixture("bgr24.avi"), &file).unwrap();
        h.app.path = file.to_string_lossy().into_owned();
        h.app.open_requested = true;
        wait(&mut h, &ws, &ctx, 3.);
        let good = h.app.shown.as_ref().unwrap().clone();
        std::fs::write(&file, b"corrupt").unwrap();
        h.app.seek(6);
        wait(&mut h, &ws, &ctx, 4.);
        assert!(h.app.error.as_ref().unwrap().contains("VideoReader"));
        assert!(Arc::ptr_eq(&good, h.app.shown.as_ref().unwrap()));
        click(&mut h, &ws, &ctx, 4., "First");
        wait(&mut h, &ws, &ctx, 4.);
        assert_eq!(h.app.shown_index, 1);
        // Restore a source that still exists for the script/export comparison.
        h.app.file_source = false;
        h.app.open_requested = true;
        wait(&mut h, &ws, &ctx, 5.);
        export_equals_saved(&mut h, &ws, &source);
    }

    #[test]
    fn video_viewer_export_names_reserved_and_runtime_rollback() {
        let ctx = egui::Context::default();
        let (mut engine, ws) = workspace("V=reshape(uint8(0:59),2,3,10); old=42;");
        let mut h = host();
        open_array(&mut h, &ws, &ctx, 0.);
        for name in [
            "mrexp_old",
            "mrexp_complete",
            "matrust_export_error",
            "bad name",
            "end",
            "read",
            "VideoReader",
            "V",
            "old",
        ] {
            h.app.output_name = name.into();
            assert!(h.app.script(&ws).is_err(), "{name}");
        }
        h.app.output_name = "old".into();
        h.app.replace = true;
        h.app.seek(8);
        wait(&mut h, &ws, &ctx, 1.);
        let toolkits::Action::ExportScript(script) = h.export(&ws).unwrap() else {
            panic!()
        };
        engine.execute("V=uint8(1);").unwrap();
        assert!(apps::execute_export(&mut engine, &script).is_err());
        assert_eq!(scalar(&engine, "old"), 42.);
        // The wrapper itself also restores the pre-existing exported name.
        assert!(engine.execute(&script).is_err());
        assert_eq!(scalar(&engine, "old"), 42.);
    }

    #[test]
    fn video_viewer_prefetch_cache_200_frames_is_bounded_and_evicts_oldest() {
        let ctx = egui::Context::default();
        let (_, ws) = workspace("V=zeros(256,256,200,'uint8');");
        let mut h = host();
        open_array(&mut h, &ws, &ctx, 0.);
        for k in (1..=200).step_by(4) {
            h.app.seek(k);
            wait(&mut h, &ws, &ctx, 0.);
            let cache = &h.app.current.as_ref().unwrap().cache;
            assert!(cache.bytes <= CACHE_CAP, "{}", cache.bytes);
        }
        let cache = &h.app.current.as_ref().unwrap().cache;
        assert!(cache.get(1).is_none());
        assert!(cache.get(200).is_some());
    }

    #[test]
    fn video_viewer_slow_decode_skips_to_latest_target_and_speed_loop_follow_time() {
        let ctx = egui::Context::default();
        let (_, ws) = workspace("V=zeros(2,3,200,'uint8');");
        let mut h = host();
        h.app.fps = 10.;
        open_array(&mut h, &ws, &ctx, 0.);
        let gate = Arc::new(std::sync::atomic::AtomicBool::new(false));
        h.app.task_gate = Some(gate.clone());
        h.app.seek(10);
        frame(&mut h, &ws, &ctx, 1., 1600., vec![]);
        assert!(h.running());
        h.app.playing = true;
        h.app.speed = 2.;
        frame(&mut h, &ws, &ctx, 1., 1600., vec![]);
        frame(&mut h, &ws, &ctx, 2., 1600., vec![]);
        assert_eq!(h.app.index, 30);
        assert_eq!(h.app.shown_index, 4);
        let next_gate = Arc::new(std::sync::atomic::AtomicBool::new(false));
        h.app.task_gate = Some(next_gate.clone());
        gate.store(true, std::sync::atomic::Ordering::Relaxed);
        let deadline = Instant::now() + Duration::from_secs(30);
        while h.app.shown_index == 4 {
            frame(&mut h, &ws, &ctx, 2., 1600., vec![]);
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            h.app.shown_index, 13,
            "present the newest decoded frame even behind the clock"
        );
        assert_eq!(h.app.index, 30);
        assert!(
            h.app.achieved_fps <= 2.,
            "reports displayed frames, not target fps"
        );
        click(&mut h, &ws, &ctx, 2., "Pause");
        assert_eq!(h.app.index, 13);
        next_gate.store(true, std::sync::atomic::Ordering::Relaxed);
        h.app.task_gate = None;
        wait(&mut h, &ws, &ctx, 2.);
        assert_eq!(
            h.app.shown_index, 13,
            "Pause holds the displayed frame while a read finishes"
        );
        assert!(
            h.app.current.as_ref().unwrap().cache.get(30).is_some(),
            "the in-flight read jumped to the latest target"
        );
        h.app.playing = true;
        h.app.seek(199);
        h.app.looping = true;
        h.app.speed = 0.5;
        frame(&mut h, &ws, &ctx, 3., 1600., vec![]);
        frame(&mut h, &ws, &ctx, 4., 1600., vec![]);
        assert_eq!(h.app.index, 4);
        h.app.playing = false;
        wait(&mut h, &ws, &ctx, 4.);
    }

    #[test]
    fn video_viewer_implay_worker_opens_registered_app_for_empty_file_and_array() {
        for call in [
            "implay".to_owned(),
            format!("implay('{}')", fixture("bgr24.avi")),
            "V=reshape(uint8(0:59),2,3,10); implay(V,12)".into(),
        ] {
            let ctx = egui::Context::default();
            let mut app = Workspace::with_context(ctx.clone(), true);
            app.run(call.clone());
            let deadline = Instant::now() + Duration::from_secs(30);
            while app.busy {
                app.receive();
                assert!(Instant::now() < deadline, "{call}: {}", app.status);
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(
                app.toolkits.apps.is_open(AppKind::VideoViewer),
                "{call}: {}",
                app.status
            );
            app.sync_app_workspace();
            let ws = &app.toolkits.apps.workspace;
            let h = &mut app.toolkits.apps.video;
            wait(h, ws, &ctx, 0.);
            if call == "implay" {
                assert!(h.app.current.is_none());
            } else {
                assert_eq!(h.app.shown_index, 1, "{call}: {:?}", h.app.error);
            }
            if call.contains("implay(V") {
                assert_eq!(h.app.current.as_ref().unwrap().properties.fps, 12.);
            }
        }
    }

    #[test]
    fn video_viewer_implay_scoped_hook_cli_shapes_and_render_states() {
        let mut e = matrust::Engine::default();
        assert!(
            e.execute("implay;")
                .unwrap_err()
                .contains("implay requires the desktop")
        );
        let slot = Arc::new(Mutex::new(None));
        let dest = slot.clone();
        let _hook = implay::present(move |launch| {
            *dest.lock().unwrap() = Some(launch);
            Ok(())
        });
        e.execute("implay;").unwrap();
        assert!(matches!(slot.lock().unwrap().take(), Some(Launch::Empty)));
        e.execute(&format!("implay('{}');", fixture("bgr24.avi")))
            .unwrap();
        assert!(matches!(slot.lock().unwrap().take(), Some(Launch::File(_))));
        for class in ["uint8", "uint16", "double", "single", "logical"] {
            for dimensions in ["2,3,5", "2,3,3,5", "2,3,1,5"] {
                e.execute(&format!("V={class}(ones({dimensions})); implay(V,7.5);"))
                    .unwrap();
                let Some(Launch::Array { name, array, fps }) = slot.lock().unwrap().take() else {
                    panic!()
                };
                assert_eq!(name, "V");
                assert_eq!(fps, 7.5);
                assert_eq!(implay::shape(&array).unwrap()[3], 5);
            }
        }
        e.execute("implay(V);").unwrap();
        assert!(matches!(
            slot.lock().unwrap().take(),
            Some(Launch::Array { fps: 20., .. })
        ));
        for code in [
            "implay(V,0)",
            "implay(V,Inf)",
            "implay(V,[1 2])",
            "implay(V,1i)",
            "implay(V,1,2)",
            "x=implay(V)",
            "V=zeros(2,2,2,2); implay(V)",
        ] {
            assert!(e.execute(code).is_err(), "{code}");
        }
        let ctx = egui::Context::default();
        let (_, ws) = workspace("V=zeros(2,3,20,'uint8');");
        let mut h = host();
        assert!(
            !frame(&mut h, &ws, &ctx, 0., 1600., vec![])
                .0
                .shapes
                .is_empty()
        );
        open_array(&mut h, &ws, &ctx, 1.);
        h.app.playing = true;
        assert!(
            !frame(&mut h, &ws, &ctx, 2., 1600., vec![])
                .0
                .shapes
                .is_empty()
        );
        h.app.playing = false;
        assert!(
            !frame(&mut h, &ws, &ctx, 3., 1600., vec![])
                .0
                .shapes
                .is_empty()
        );
        assert!(
            !frame(&mut h, &ws, &ctx, 4., 900., vec![])
                .0
                .shapes
                .is_empty()
        );
        let window = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900., 800.));
        for name in ["Export", "Generate script"] {
            assert!(h.app.controls[name].left() >= 280. && h.app.controls[name].right() <= 900.);
        }
        for attempt in 0..12 {
            if ["Export", "Generate script"]
                .iter()
                .all(|name| window.contains_rect(h.app.controls[name]))
            {
                break;
            }
            frame(
                &mut h,
                &ws,
                &ctx,
                4. + attempt as f64 / 60.,
                900.,
                vec![
                    egui::Event::PointerMoved(egui::pos2(700., 600.)),
                    egui::Event::MouseWheel {
                        phase: egui::TouchPhase::Move,
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0., -240.),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        for name in ["Export", "Generate script"] {
            assert!(
                window.contains_rect(h.app.controls[name]),
                "narrow {name}: {:?}",
                h.app.controls[name]
            );
        }

        h.app.error = Some("VideoReader: damaged frame".into());
        assert!(
            !frame(&mut h, &ws, &ctx, 5., 900., vec![])
                .0
                .shapes
                .is_empty()
        );
        assert!(AppKind::ALL.contains(&AppKind::VideoViewer));
        assert_eq!(
            AppKind::VideoViewer.tab(),
            matrust::session::ToolTab::VideoViewer
        );
    }

    #[test]
    fn video_viewer_sample_data_opens_motion_jpeg_and_script_runs() {
        let ctx = egui::Context::default();
        let (_, ws) = workspace("");
        let mut h = host();
        frame(&mut h, &ws, &ctx, 0., 900., vec![]);
        let open = h.app.controls["Open"];
        let sample = h.app.controls["Use sample data"];
        let cancel = h.app.controls["Cancel"];
        for (name, rect) in [
            ("Open", open),
            ("Use sample data", sample),
            ("Cancel", cancel),
        ] {
            assert!(
                rect.width() >= 32.
                    && rect.height() >= 16.
                    && rect.left() >= 0.
                    && rect.right() <= 280.,
                "{name} clipped: {rect:?}"
            );
        }
        assert!(
            !open.intersects(sample) && !sample.intersects(cancel),
            "buttons overlap: {open:?} {sample:?} {cancel:?}"
        );
        frame(&mut h, &ws, &ctx, 0., 1600., vec![]);
        click(&mut h, &ws, &ctx, 0.1, "Use sample data");
        wait(&mut h, &ws, &ctx, 0.1);
        assert!(h.app.error.is_none(), "{:?}", h.app.error);
        let video = h.app.current.as_ref().unwrap();
        assert!(video.sample);
        assert!(
            matches!(&video.source, Source::File(path) if path.ends_with("matrust-video-sample.avi"))
        );
        assert_eq!(video.properties.width, 160);
        assert_eq!(video.properties.height, 120);
        assert_eq!(video.properties.frames, 90);
        assert_eq!(video.properties.fps, 30.);
        assert_eq!(h.app.shown_index, 1);
        let first = h.app.shown.as_ref().unwrap().array.clone();
        assert_eq!(first.class(), "uint8");
        assert_eq!(first.dims().as_ref(), [120usize, 160, 3].as_slice());
        h.app.seek(45);
        wait(&mut h, &ws, &ctx, 0.2);
        assert_eq!(h.app.shown_index, 45, "{:?}", h.app.error);
        let mid = h.app.shown.as_ref().unwrap().array.clone();
        let mut compared = matrust::Engine::default();
        compared.insert_array("a", first).unwrap();
        compared.insert_array("b", mid).unwrap();
        compared.execute("d = isequal(a,b);").unwrap();
        let matrust::Array::Logical(equal) = compared.array("d").unwrap() else {
            panic!("logical equality")
        };
        assert_eq!(equal.data()[0], 0);
        let script = h.app.script(&ws).unwrap();
        assert!(script.contains("fullfile(tempdir,'matrust-video-sample.avi')"));
        assert!(script.contains("VideoWriter"));
        assert!(script.contains("'Motion JPEG AVI'"));
        assert!(script.contains("FrameRate"));
        assert!(script.contains("read(v, 45)"));
        let toolkits::Action::SaveAppScript(kind, saved) = h.generate(&ws, 0).unwrap() else {
            panic!("script action")
        };
        assert_eq!(kind, AppKind::VideoViewer);
        let mut fresh = matrust::Engine::default();
        fresh.execute(&saved).unwrap();
        fresh
            .insert_array("preview", h.app.shown.as_ref().unwrap().array.clone())
            .unwrap();
        assert_equal(&mut fresh, &h.app.output_name, "preview");
    }
}
