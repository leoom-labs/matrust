// NumPy fixtures and end-to-end checks through the real shared-script host.
mod image_viewer_tests {
    use super::*;
    use apps::image_viewer::{
        Contrast, Crop, DISPLAY_SIDE, ImageViewer, MAX_DISTANCES, Tool, image_shape, pixel,
    };
    use apps::{App, AppKind, Host, Snapshot};

    fn reference() -> serde_json::Value {
        serde_json::from_str(include_str!("../../tests/fixtures/apps/image_viewer/reference.json")).unwrap()
    }
    fn expected(name: &str) -> Vec<f64> {
        reference()[name]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_f64().unwrap())
            .collect()
    }
    fn workspace(code: &str) -> (matrust::Engine, Snapshot) {
        let mut e = matrust::Engine::default();
        e.execute(code).unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&e.workspace());
        (e, ws)
    }
    fn viewer() -> Host<ImageViewer> {
        let mut host = Host::new(ImageViewer::default());
        host.app.source = "I".into();
        host
    }
    fn compute(host: &mut Host<ImageViewer>, ws: &Snapshot) -> Result<apps::Outputs, String> {
        let outputs = apps::run(&mut matrust::Engine::default(), host.request(ws)?)?;
        host.app.receive(&outputs)?;
        Ok(outputs)
    }
    fn values(a: &matrust::Array) -> Vec<f64> {
        match a {
            matrust::Array::Numeric(a) => a.data().to_vec(),
            matrust::Array::Typed(a) => (0..a.len()).map(|i| a.real(i)).collect(),
            matrust::Array::Logical(a) => a.data().iter().map(|v| f64::from(*v)).collect(),
            _ => panic!("expected image data"),
        }
    }
    fn close(a: &[f64], b: &[f64]) {
        assert_eq!(a.len(), b.len());
        for (a, b) in a.iter().zip(b) {
            assert!((a - b).abs() < 2e-13, "{a} != {b}");
        }
    }
    fn wait(host: &mut Host<ImageViewer>, ws: &Snapshot, ctx: &egui::Context) {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            host.refresh(ctx, ws);
            if !host.running() {
                break;
            }
            assert!(Instant::now() < deadline, "image preview timed out");
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(
            host.status().1,
            toolkits::AppStatus::Success,
            "{}",
            host.status().0
        );
    }
    fn main_frame(
        app: &mut ImageViewer,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        crate::headless::run(
            ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800., 600.),
                )),
                events,
                ..Default::default()
            },
            |ui| app.main_ui(ui),
        )
    }

    fn full_frame(
        host: &mut Host<ImageViewer>,
        ws: &Snapshot,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        crate::headless::run(
            ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1440., 900.),
                )),
                events,
                ..Default::default()
            },
            |ui| apps::image_viewer::show(host, ui, ws, false, &mut None),
        )
    }
    fn image_rect(output: &egui::FullOutput, id: egui::TextureId) -> egui::Rect {
        fn find(shape: &egui::Shape, id: egui::TextureId) -> Option<egui::Rect> {
            match shape {
                egui::Shape::Rect(r)
                    if r.brush.as_ref().is_some_and(|b| b.fill_texture_id == id) =>
                {
                    Some(r.rect)
                }
                egui::Shape::Mesh(m) if m.texture_id == id => Some(m.calc_bounds()),
                egui::Shape::Vec(v) => v.iter().find_map(|s| find(s, id)),
                _ => None,
            }
        }
        output
            .shapes
            .iter()
            .find_map(|s| find(&s.shape, id))
            .expect("image texture quad")
    }
    fn pointer(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }
    fn drag(app: &mut ImageViewer, ctx: &egui::Context, from: egui::Pos2, to: egui::Pos2) {
        main_frame(app, ctx, vec![egui::Event::PointerMoved(from)]);
        main_frame(app, ctx, vec![pointer(from, true)]);
        main_frame(app, ctx, vec![egui::Event::PointerMoved(to)]);
        main_frame(app, ctx, vec![pointer(to, false)]);
    }

    #[test]
    fn image_viewer_pointer_crop_distance_pan_wheel_and_overview() {
        let (_, ws) = workspace("I=uint8(reshape(1:42,6,7));");
        let mut host = viewer();
        compute(&mut host, &ws).unwrap();
        let ctx = egui::Context::default();
        let frame = main_frame(&mut host.app, &ctx, vec![]);
        let id = host.app.texture_id().unwrap();
        let rect = image_rect(&frame, id);
        let at = |col: f32, row: f32| {
            rect.min
                + egui::vec2(
                    (col - 0.5) * rect.width() / 7.,
                    (row - 0.5) * rect.height() / 6.,
                )
        };
        host.app.tool = Tool::Distance;
        drag(&mut host.app, &ctx, at(1., 1.), at(4., 5.));
        assert_eq!(host.app.lines, vec![[[1., 1.], [4., 5.]]]);
        compute(&mut host, &ws).unwrap();
        close(
            &host.app.current.as_ref().unwrap().distances[0],
            &expected("distance"),
        );
        host.app.tool = Tool::Crop;
        drag(&mut host.app, &ctx, at(2., 3.), at(5., 5.));
        assert_eq!(
            host.app.crop,
            Some(Crop {
                x: 2,
                y: 3,
                width: 3,
                height: 2
            })
        );
        compute(&mut host, &ws).unwrap();
        close(
            &values(&host.app.current.as_ref().unwrap().image),
            &expected("crop_values"),
        );
        host.app.tool = Tool::Pan;
        host.app.zoom = Some(200.);
        main_frame(&mut host.app, &ctx, vec![]);
        drag(
            &mut host.app,
            &ctx,
            egui::pos2(350., 300.),
            egui::pos2(390., 320.),
        );
        assert!(host.app.pan.length() > 0.);
        let overview = main_frame(&mut host.app, &ctx, vec![]);
        let frame = main_frame(
            &mut host.app,
            &ctx,
            vec![
                egui::Event::PointerMoved(egui::pos2(400., 300.)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: egui::vec2(0., -100.),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        assert!(host.app.zoom.unwrap() < 200.);
        assert!(!frame.shapes.is_empty());
        let id = host.app.texture_id().unwrap();
        let meshes=overview.shapes.iter().filter(|s|matches!(&s.shape,egui::Shape::Mesh(m) if m.texture_id==id)||matches!(&s.shape,egui::Shape::Rect(r) if r.brush.as_ref().is_some_and(|b|b.fill_texture_id==id))).count();
        assert_eq!(meshes, 2, "zoomed view draws main texture and overview");
        // Full framework frame exercises Inputs, Output and histogram together.
        assert!(!full_frame(&mut host, &ws, &ctx, vec![]).shapes.is_empty());
    }

    #[test]
    fn image_viewer_pixel_readout_classes_and_rgb_numpy() {
        let fixture = reference();
        for class in ["uint8", "uint16", "double", "single", "logical"] {
            let sample_values = &fixture["pixel_values"][class];
            let code_values = sample_values
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
                .join(" ");
            let (e, ws) = workspace(&format!("I={class}([{code_values}]);"));
            let mut host = viewer();
            compute(&mut host, &ws).unwrap();
            let s = host.app.current.as_ref().unwrap();
            assert_eq!(s.image.class(), class);
            for col in 1..=3 {
                let text = pixel(&s.image, &s.map, 1, col).unwrap();
                assert!(
                    text.contains(&format!("Row 1 · column {col} · {class}")),
                    "{text}"
                );
                assert!(
                    text.contains(&format!(
                        "[{}]",
                        apps::number(sample_values[col - 1].as_f64().unwrap())
                    )),
                    "{text}"
                );
            }
            assert!(pixel(&s.image, &s.map, 0, 1).is_none());
            assert!(pixel(&s.image, &s.map, 1, 4).is_none());
            assert_eq!(values(&s.image), values(&e.array("I").unwrap()));
            let (_, ws) = workspace(&format!(
                "I=cat(3,{class}({}),{class}({}),{class}({}));",
                sample_values[0], sample_values[1], sample_values[2]
            ));
            let mut host = viewer();
            compute(&mut host, &ws).unwrap();
            let s = host.app.current.as_ref().unwrap();
            assert_eq!(image_shape(&s.image).unwrap(), [1, 1, 3]);
            let text = pixel(&s.image, &s.map, 1, 1).unwrap();
            let list = sample_values
                .as_array()
                .unwrap()
                .iter()
                .map(|x| apps::number(x.as_f64().unwrap()))
                .collect::<Vec<_>>()
                .join(", ");
            assert!(text.ends_with(&format!("[{list}]")), "{text}");
        }
    }

    #[test]
    fn image_viewer_indexed_pixel_rgb_and_adjusted_colormap() {
        for (class, indices) in [
            ("uint8", "0 1 2"),
            ("uint16", "0 1 2"),
            ("double", "1 2 3"),
            ("single", "1 2 3"),
        ] {
            let (_, ws) = workspace(&format!("I={class}([{indices}]); M=[1 0 0;0 .5 0;0 0 1];"));
            let mut host = viewer();
            host.app.colormap = "M".into();
            compute(&mut host, &ws).unwrap();
            let s = host.app.current.as_ref().unwrap();
            for (col, rgb) in reference()["indexed_rgb"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
            {
                let text = pixel(&s.image, &s.map, 1, col + 1).unwrap();
                let rgb = rgb
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| apps::number(v.as_f64().unwrap()))
                    .collect::<Vec<_>>()
                    .join(", ");
                assert!(
                    text.contains("index ") && text.ends_with(&format!("RGB [{rgb}]")),
                    "{text}"
                );
            }
            assert_eq!(s.histogram.len(), 3);
            host.app.contrast = Contrast::Window;
            host.app.window = [0.2, 0.8];
            compute(&mut host, &ws).unwrap();
            let s = host.app.current.as_ref().unwrap();
            assert_eq!(s.image.class(), class);
            close(&values(&s.map), &[1., 0., 0., 0., 0.5, 0., 0., 0., 1.]);
        }
    }

    #[test]
    fn image_viewer_crop_rectangle_equals_indexing_numpy() {
        let (mut e, ws) = workspace("I=reshape(1:42,6,7);");
        let mut host = viewer();
        host.app.crop = Some(Crop::from_points([5, 5], [2, 3]));
        compute(&mut host, &ws).unwrap();
        let s = host.app.current.as_ref().unwrap();
        assert_eq!(
            s.crop,
            Some(Crop {
                x: 2,
                y: 3,
                width: 3,
                height: 2
            })
        );
        assert_eq!(s.image.dims().as_ref(), &[3, 4]);
        close(&values(&s.image), &expected("crop_values"));
        e.execute("indexed=I(3:5,2:5);").unwrap();
        assert_eq!(values(&s.image), values(&e.array("indexed").unwrap()));
        host.app.crop = Some(Crop::from_points([7, 6], [7, 6]));
        compute(&mut host, &ws).unwrap();
        let s = host.app.current.as_ref().unwrap();
        assert_eq!(s.image.dims().as_ref(), &[1, 1]);
        assert_eq!(values(&s.image), [42.]);
        host.app.crop = Some(Crop {
            x: 7,
            y: 6,
            width: 1,
            height: 0,
        });
        assert!(
            compute(&mut host, &ws)
                .unwrap_err()
                .contains("crop exceeds")
        );
        assert_eq!(values(&host.app.current.as_ref().unwrap().image), [42.]);
        host.app.crop = Some(Crop {
            x: 0,
            y: 1,
            width: 1,
            height: 1,
        });
        assert!(host.request(&ws).is_err());
    }

    #[test]
    fn image_viewer_distance_known_points_numpy() {
        let (_, ws) = workspace("I=uint8(zeros(6,7));");
        let mut host = viewer();
        host.app.lines = vec![
            [[1., 1.], [4., 5.]],
            [[4., 5.], [1., 1.]],
            [[1., 1.], [1., 1.]],
        ];
        compute(&mut host, &ws).unwrap();
        let distances = &host.app.current.as_ref().unwrap().distances;
        close(&distances[0], &expected("distance"));
        assert_eq!(distances[1][4], 5.);
        close(&[distances[1][5]], &[expected("distance")[5] - 180.]);
        assert_eq!(&distances[2][4..], &[0., 0.]);
        host.app.lines = vec![[[1., 1.], [f64::NAN, 2.]]];
        assert!(host.request(&ws).is_err());
        host.app.lines = vec![[[1., 1.], [8., 1.]]];
        assert!(compute(&mut host, &ws).is_err());
        host.app.lines = vec![[[1., 1.], [1., 1.]]; MAX_DISTANCES + 1];
        assert!(host.request(&ws).is_err());
    }

    #[test]
    fn image_viewer_contrast_stretchlim_numpy_and_histogram() {
        for (source, fixture) in [
            ("uint8(reshape(0:255,16,16))", "saturate_uint8"),
            ("reshape(0:255,16,16)/255", "saturate_double"),
            ("uint16(reshape(0:255,16,16)*257)", "saturate_uint16"),
            (
                "cat(3,uint8(0:255),uint8(255:-1:0),uint8(50*ones(1,256)))",
                "saturate_rgb",
            ),
        ] {
            let (_, ws) = workspace(&format!("I={source};"));
            let mut host = viewer();
            host.app.contrast = Contrast::Saturate;
            compute(&mut host, &ws).unwrap();
            let s = host.app.current.as_ref().unwrap();
            close(&s.limits, &expected(fixture));
            for counts in &s.histogram {
                assert_eq!(counts.iter().sum::<f64>(), 256.);
            }
        }
        let (_, ws) = workspace("I=uint8([20 30 100 200]);");
        let mut host = viewer();
        host.app.contrast = Contrast::DataRange;
        compute(&mut host, &ws).unwrap();
        close(
            &host.app.current.as_ref().unwrap().limits,
            &expected("match_range"),
        );
        let (_, ws) = workspace("I=uint8(reshape(0:255,16,16));");
        let mut host = viewer();
        host.app.contrast = Contrast::Window;
        host.app.window = [0.2, 0.8];
        compute(&mut host, &ws).unwrap();
        close(
            &values(&host.app.current.as_ref().unwrap().image),
            &expected("window_values"),
        );
        for window in [[0.5, 0.5], [0.8, 0.2], [-1., 1.], [0., f64::NAN]] {
            host.app.window = window;
            assert!(host.request(&ws).is_err());
        }
        let (_, ws) = workspace("I=logical([0 1]);");
        let mut host = viewer();
        host.app.contrast = Contrast::Window;
        host.app.window = [0.2, 0.8];
        compute(&mut host, &ws).unwrap();
        assert_eq!(host.app.current.as_ref().unwrap().image.class(), "logical");
    }

    #[test]
    fn image_viewer_generated_script_fresh_engine_matches_atomic_export() {
        let (source, ws) = workspace("I=uint8(reshape(0:255,16,16));");
        let mut host = viewer();
        host.app.crop = Some(Crop {
            x: 2,
            y: 3,
            width: 6,
            height: 5,
        });
        host.app.contrast = Contrast::Window;
        host.app.window = [0.2, 0.8];
        host.app.lines.push([[1., 1.], [4., 5.]]);
        compute(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(script.contains("imcrop(") && script.contains("imadjust("));
        assert!(script.contains("strcmp(class("));
        assert!(!script.contains("isa("));
        let saved = match host.generate(&ws, 42).unwrap() {
            toolkits::Action::SaveAppScript(AppKind::ImageViewer, s) => s,
            _ => panic!("wrong action"),
        };
        assert!(saved.ends_with(&script));
        let mut fresh = matrust::Engine::default();
        fresh.insert_array("I", source.array("I").unwrap()).unwrap();
        fresh.execute(&saved).unwrap();
        let exported = match host.export(&ws).unwrap() {
            toolkits::Action::ExportScript(s) => s,
            _ => panic!("wrong action"),
        };
        let mut e = matrust::Engine::default();
        e.insert_array("I", source.array("I").unwrap()).unwrap();
        e.execute(&exported).unwrap();
        for name in host.app.exported(&ws) {
            let a = e.array(&name).unwrap();
            let b = fresh.array(&name).unwrap();
            assert_eq!(a.class(), b.class());
            assert_eq!(a.dims(), b.dims());
            assert_eq!(values(&a), values(&b));
        }
        assert_eq!(
            values(&fresh.array("viewed_image").unwrap()),
            values(&host.app.current.as_ref().unwrap().image)
        );
        // A runtime error restores every public output, even a pre-existing value.
        e.execute("viewed_image=99; image_distances=98; viewed_map=97; I=uint8(1);")
            .unwrap();
        assert!(e.execute(&exported).is_err());
        assert_eq!(values(&e.array("viewed_image").unwrap()), [99.]);
        assert_eq!(values(&e.array("image_distances").unwrap()), [98.]);
        assert_eq!(values(&e.array("viewed_map").unwrap()), [97.]);
    }

    #[test]
    fn image_viewer_export_names_parser_clobber_and_last_good() {
        let (_, ws) = workspace("I=uint8([0 255]); viewed_image=42; iv_source1=0; iv_map1=0;");
        let mut host = viewer();
        assert!(host.request(&ws).err().unwrap().contains("exists"));
        host.app.replace = true;
        compute(&mut host, &ws).unwrap();
        let old = values(&host.app.current.as_ref().unwrap().image);
        for name in [
            "mrexp_old",
            "mrexp_had",
            "matrust_export_error",
            "mrexp_complete",
            "size",
            "bad name",
            "1x",
            "out); intrusion=1; %",
            "out\nerror('bad')",
        ] {
            host.app.image_name = name.into();
            assert!(host.request(&ws).is_err(), "{name}");
            assert_eq!(values(&host.app.current.as_ref().unwrap().image), old);
        }
        host.app.image_name = host.app.distance_name.clone();
        assert!(host.request(&ws).is_err());
        host.app.image_name = "viewed_image".into();
        for bad in ["I; intrusion=1", "I(1)", "I\nerror('bad')", "I);x=1;%"] {
            host.app.source = bad.into();
            assert!(host.request(&ws).is_err());
        }
        host.app.source = "I".into();
        host.app.colormap = "M;intrusion=1".into();
        assert!(host.request(&ws).is_err());
        host.app.colormap.clear();
        let mut broken = compute(&mut host, &ws).unwrap();
        broken.remove("iv_probe_hist");
        assert!(host.app.receive(&broken).is_err());
        assert_eq!(values(&host.app.current.as_ref().unwrap().image), old);
        host.app.file_source = true;
        host.app.path = "/missing/image.png".into();
        assert!(compute(&mut host, &ws).is_err());
        assert_eq!(values(&host.app.current.as_ref().unwrap().image), old);
    }

    #[test]
    fn image_viewer_temp_png_imfinfo_and_file_script() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("quote's image.png");
        {
            let file = std::fs::File::create(&path).unwrap();
            let mut encoder = png::Encoder::new(file, 3, 2);
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[0, 17, 255, 128, 1, 64])
                .unwrap();
        }
        let ws = Snapshot::default();
        let mut host = viewer();
        host.app.file_source = true;
        host.app.path = path.to_str().unwrap().into();
        compute(&mut host, &ws).unwrap();
        let s = host.app.current.as_ref().unwrap();
        assert_eq!(s.source_size, [2, 3]);
        assert_eq!(s.image.class(), "uint8");
        assert_eq!(values(&s.image), [0., 128., 17., 1., 255., 64.]);
        assert!(
            s.file_info.contains("3 × 2 · uint8 · 8 bit · grayscale"),
            "{}",
            s.file_info
        );
        assert!(s.file_info.contains(&format!(
            "{} bytes",
            std::fs::metadata(&path).unwrap().len()
        )));
        let script = host.app.script(&ws).unwrap();
        assert!(script.contains("imread(") && script.contains("quote''s"));
        let mut fresh = matrust::Engine::default();
        fresh.execute(&script).unwrap();
        assert_eq!(
            values(&fresh.array("viewed_image").unwrap()),
            values(&s.image)
        );
        host.app.path.push('\n');
        assert!(host.request(&ws).is_err());
    }

    #[test]
    fn image_viewer_headless_empty_singleton_tool_tab_and_texture_cache() {
        let kind = AppKind::ImageViewer;
        assert_eq!(AppKind::from_tab(kind.tab()), Some(kind));
        assert_eq!(
            serde_json::to_string(&kind.tab()).unwrap(),
            "\"image_viewer\""
        );
        let tab: matrust::session::ToolTab = serde_json::from_str("\"image_viewer\"").unwrap();
        assert_eq!(tab, kind.tab());
        for width in [820., 1600.] {
            let ctx = egui::Context::default();
            let mut desktop = Workspace::with_context(ctx.clone(), true);
            desktop.toolkits.open_tool(toolkits::Tool::App(kind));
            assert!(
                !frame_at(&mut desktop, &ctx, egui::vec2(width, 950.), vec![])
                    .shapes
                    .is_empty()
            );
            desktop.close_tool_tab(toolkits::Tool::App(kind));
        }
        let (_, ws) = workspace("I=uint8(127);");
        let mut host = viewer();
        compute(&mut host, &ws).unwrap();
        let ctx = egui::Context::default();
        assert!(!full_frame(&mut host, &ws, &ctx, vec![]).shapes.is_empty());
        let first = main_frame(&mut host.app, &ctx, vec![]);
        assert!(!first.shapes.is_empty());
        let id = host.app.texture_id().unwrap();
        host.app.image_name = "renamed_image".into();
        compute(&mut host, &ws).unwrap();
        let second = main_frame(&mut host.app, &ctx, vec![]);
        assert_eq!(host.app.texture_id(), Some(id));
        assert!(!second.shapes.is_empty());
        host.app.tool = Tool::Distance;
        host.app.zoom = Some(1.);
        host.app.pan = egui::vec2(24., 16.);
        let frame = main_frame(
            &mut host.app,
            &ctx,
            vec![egui::Event::PointerMoved(egui::pos2(400., 320.))],
        );
        assert!(!frame.shapes.is_empty());
        let (_, ws) = workspace("I=uint8(128);");
        compute(&mut host, &ws).unwrap();
        let frame = main_frame(&mut host.app, &ctx, vec![]);
        assert_ne!(host.app.texture_id(), Some(id));
        assert!(!frame.shapes.is_empty());
    }

    fn large_image_lock() -> std::sync::MutexGuard<'static, ()> {
        static LARGE: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LARGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[test]
    fn image_viewer_refuses_oversized_image_before_the_run() {
        let limit = matrust::matrix::MAX_ELEMENTS;
        let error =
            apps::image_viewer::reject_image_size(5000, 4000, 3, limit + 3).unwrap_err();
        assert!(error.contains("5000-by-4000-by-3"), "{error}");
        assert!(error.contains(&format!("{}", limit + 3)), "{error}");
        assert!(error.contains(&format!("limit of {limit}")), "{error}");
        assert!(apps::image_viewer::reject_image_size(3000, 4000, 3, 3000 * 4000 * 3).is_ok());
        let (_, ws) = workspace("I=uint8([0 255]);");
        let host = viewer();
        assert!(host.request(&ws).is_ok());
        let script = host.app.script(&ws).unwrap();
        assert!(script.contains("numel("));
        assert!(script.contains(&format!("above the limit of {limit}")));
        assert!(script.contains("strcmp(class("));
        assert!(!script.contains("isa("));
        // A plan error (the size check is one) is refused before the runner starts.
        let ctx = egui::Context::default();
        let mut host = viewer();
        host.app.source = "missingImage".into();
        host.refresh(&ctx, &ws);
        assert!(!host.running());
        assert_eq!(host.status().1, toolkits::AppStatus::Error);
        assert!(host.app.current.is_none());
    }

    #[test]
    fn image_viewer_run_error_stays_on_the_status_line() {
        let (_, ws) = workspace("I=uint8([0 255]);");
        let mut host = viewer();
        compute(&mut host, &ws).unwrap();
        let old = values(&host.app.current.as_ref().unwrap().image);
        // The crop guard runs inside the engine. Its error is the status line.
        host.app.crop = Some(Crop {
            x: 7,
            y: 6,
            width: 1,
            height: 0,
        });
        let ctx = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            host.refresh(&ctx, &ws);
            if !host.running() && host.status().0.contains("crop exceeds") {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "in-run error did not reach the status line: {}",
                host.status().0
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(host.status().1, toolkits::AppStatus::Error);
        assert_eq!(values(&host.app.current.as_ref().unwrap().image), old);
    }

    #[test]
    fn image_viewer_engine_large_rgb_png_input() {
        let _serial = large_image_lock();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.png");
        {
            let file = std::fs::File::create(&path).unwrap();
            let mut encoder = png::Encoder::new(file, 4000, 3000);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&vec![0; 36_000_000])
                .unwrap();
        }
        let mut engine = matrust::Engine::with_limits(matrust::engine::Limits::local());
        engine
            .execute(&format!(
                "I=imread('{}');",
                path.to_str().unwrap().replace('\'', "''")
            ))
            .unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        let mut host = viewer();
        let ctx = egui::Context::default();
        wait(&mut host, &ws, &ctx);
        assert_eq!(
            image_shape(&host.app.current.as_ref().unwrap().image).unwrap(),
            [3000, 4000, 3]
        );
    }

    #[test]
    fn image_viewer_large_rgb_runner_frames_and_stop() {
        let _serial = large_image_lock();
        // Same memory guards as the desktop Runner (`Limits::local()`).
        let mut e = matrust::Engine::with_limits(matrust::engine::Limits::local());
        let image = matrust::TypedArray::from_reals(
            matrust::NumericClass::Uint8,
            3000,
            4000 * 3,
            (0..36_000_000).map(|i| (i % 256) as f64),
        )
        .unwrap()
        .with_dims(&[3000, 4000, 3])
        .unwrap();
        e.insert_array("I", matrust::Array::Typed(image)).unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&e.workspace());
        let mut host = viewer();
        let ctx = egui::Context::default();
        let started = Instant::now();
        host.refresh(&ctx, &ws);
        assert!(host.running());
        // Rendering a pending large run does not execute the image pipeline.
        for _ in 0..3 {
            assert!(!main_frame(&mut host.app, &ctx, vec![]).shapes.is_empty());
        }
        wait(&mut host, &ws, &ctx);
        let s = host.app.current.as_ref().unwrap();
        assert_eq!(image_shape(&s.image).unwrap(), [3000, 4000, 3]);
        assert_eq!(image_shape(&s.display).unwrap(), [1500, 2000, 3]);
        assert!(s.display.dims()[0] <= DISPLAY_SIDE && s.display.dims()[1] <= DISPLAY_SIDE);
        for channel in &s.histogram {
            assert_eq!(channel.iter().sum::<f64>(), 12_000_000.);
        }
        let computation = started.elapsed();
        let frame_started = Instant::now();
        main_frame(&mut host.app, &ctx, vec![]);
        let first_frame = frame_started.elapsed();
        assert!(!full_frame(&mut host, &ws, &ctx, vec![]).shapes.is_empty());
        let id = host.app.texture_id().unwrap();
        host.app.zoom = Some(1.);
        main_frame(&mut host.app, &ctx, vec![]);
        assert_eq!(host.app.texture_id(), Some(id));
        eprintln!(
            "image_viewer 4000x3000 RGB Runner={:?} first texture frame={first_frame:?}",
            computation
        );
        // Cancel an actual app request and ensure a stale result is not delivered.
        host.app.contrast = Contrast::Saturate;
        host.refresh(&ctx, &ws);
        assert!(host.running());
        apps::image_viewer::stop(&mut host);
        let deadline = Instant::now() + Duration::from_secs(10);
        while host.running() {
            assert!(Instant::now() < deadline);
            host.refresh(&ctx, &ws);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(host.request(&ws).err().unwrap().contains("stopped"));
        assert_eq!(
            host.app.current.as_ref().unwrap().image.dims().as_ref(),
            &[3000, 4000, 3]
        );
        assert!(host.status().0.contains("stopped"));
        // Resume through the visible control, then complete full-resolution
        // per-channel contrast/histograms through the Runner.
        let frame = full_frame(&mut host, &ws, &ctx, vec![]);
        let mut positions = Vec::new();
        for shape in &frame.shapes {
            tool_tab_texts(&shape.shape, "Resume", &mut positions);
        }
        let resume = *positions.first().expect("Resume control");
        for pressed in [true, false] {
            full_frame(
                &mut host,
                &ws,
                &ctx,
                vec![egui::Event::PointerMoved(resume), pointer(resume, pressed)],
            );
        }
        wait(&mut host, &ws, &ctx);
        for pair in host.app.current.as_ref().unwrap().limits.chunks(2) {
            close(pair, &expected("saturate_uint8"));
        }
    }

    #[test]
    fn image_viewer_release_pipeline_one_and_n_workers() {
        let workers = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(8);
        for workers in [1, workers] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap();
            pool.install(||{
                let (_,ws)=workspace("I=uint8(reshape(mod(0:786431,256),512,512,3));");let mut host=viewer();host.app.contrast=Contrast::Saturate;
                let started=Instant::now();compute(&mut host,&ws).unwrap();
                eprintln!("image_viewer release pipeline workers={workers} 512x512 RGB saturate+histogram {:?}",started.elapsed());
            });
        }
    }

    #[test]
    fn image_viewer_rejects_unsupported_shapes_and_types() {
        for code in [
            "I=[];",
            "I=complex(ones(2),ones(2));",
            "I=ones(2,2,2);",
            "I=ones(2,2,3,2);",
            "I=int16(ones(2));",
            "I=sparse(ones(2));",
            "I=sparse(logical(ones(2)));",
            "I={1};",
        ] {
            let (_, ws) = workspace(code);
            assert!(viewer().request(&ws).is_err(), "{code}");
        }
        let (_, ws) = workspace("I=uint8(ones(2)); M=[1 NaN 0];");
        let mut host = viewer();
        host.app.colormap = "M".into();
        assert!(compute(&mut host, &ws).is_err());
    }
}
