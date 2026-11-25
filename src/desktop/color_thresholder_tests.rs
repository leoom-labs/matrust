// Independent fixtures: tool/ref-python.sh tool/refs/color_thresholder/generate.py.
mod color_thresholder_tests {
    use super::*;
    use apps::color_thresholder::{ColorThresholder, DISPLAY_SIDE, Space, View};
    use apps::{App, AppKind, Host, Snapshot};

    static LARGE_RUN: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn reference() -> serde_json::Value {
        serde_json::from_str(include_str!(
            "../../tests/fixtures/apps/color_thresholder/reference.json"
        ))
        .unwrap()
    }
    fn workspace(class: &str) -> (matrust::Engine, Snapshot) {
        let data = reference()["rgb8"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        let expr = match class {
            "double" => format!("[{data}]/255"),
            "single" => format!("single([{data}]/255)"),
            "uint16" => format!("uint16([{data}]*257)"),
            "logical" => format!("logical([{data}] > 127)"),
            _ => format!("uint8([{data}])"),
        };
        let mut e = matrust::Engine::default();
        e.execute(&format!("RGB = reshape({expr},2,4,3);")).unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&e.workspace());
        (e, ws)
    }
    fn host() -> Host<ColorThresholder> {
        let mut app = ColorThresholder::default();
        app.source = "RGB".into();
        Host::new(app)
    }
    fn compute(h: &mut Host<ColorThresholder>, ws: &Snapshot) -> Result<apps::Outputs, String> {
        let outputs = apps::run(&mut matrust::Engine::default(), h.request(ws)?)?;
        h.app.receive(&outputs)?;
        Ok(outputs)
    }
    fn numbers(a: &matrust::Array) -> Vec<f64> {
        match a {
            matrust::Array::Numeric(m) => m.data().to_vec(),
            matrust::Array::Logical(m) => m.data().iter().map(|v| *v as f64).collect(),
            matrust::Array::Typed(m) => m.to_f64_vec().unwrap(),
            _ => panic!("unexpected class {}", a.class()),
        }
    }
    fn expected(case: &serde_json::Value, name: &str) -> Vec<f64> {
        case[name]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect()
    }
    fn configure(h: &mut Host<ColorThresholder>, case: &serde_json::Value) {
        h.app.set_space(
            Space::ALL
                .into_iter()
                .find(|s| s.label() == case["space"].as_str().unwrap())
                .unwrap(),
        );
        h.app.limits =
            std::array::from_fn(|c| std::array::from_fn(|k| case["limits"][c][k].to_string()));
        h.app.invert = case["invert"].as_bool().unwrap();
    }

    #[test]
    fn color_thresholder_numpy_spaces_hue_wrap_invert_histograms() {
        let (_, ws) = workspace("uint8");
        for case in reference()["cases"].as_array().unwrap() {
            let mut h = host();
            configure(&mut h, case);
            let out = compute(&mut h, &ws).unwrap_or_else(|e| panic!("{case}: {e}"));
            assert_eq!(numbers(&out["ct_mask"]), expected(case, "mask"));
            let view = h.app.current.as_ref().unwrap();
            assert_eq!(view.total, 8);
            assert_eq!(
                view.percentage,
                100. * expected(case, "mask").iter().sum::<f64>() / 8.
            );
            assert_eq!(
                view.selected,
                expected(case, "mask").iter().sum::<f64>() as usize
            );
            for c in 0..3 {
                let hist = case["histograms"][c]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_f64().unwrap())
                    .collect::<Vec<_>>();
                if h.app.space == Space::Lab {
                    // Lab matrices differ slightly in published tabulation
                    // precision; only boundary bins have a nonzero interval.
                    for (bin, actual) in view.histograms[c].iter().enumerate() {
                        let bounds = &case["histogram_bounds"][c][bin];
                        assert!(
                            (bounds[0].as_f64().unwrap()..=bounds[1].as_f64().unwrap())
                                .contains(actual),
                            "Lab channel {c}, bin {bin}: {actual}"
                        );
                    }
                } else {
                    assert_eq!(
                        view.histograms[c],
                        hist,
                        "{} channel {c}",
                        h.app.space.label()
                    );
                }
                assert_eq!(view.histograms[c].iter().sum::<f64>(), view.total as f64);
            }
            assert_eq!(h.app.results().len(), 5);
        }
    }

    #[test]
    fn color_thresholder_generated_function_fresh_engine_export_and_save() {
        let dir = tempfile::tempdir().unwrap();
        for case in reference()["cases"].as_array().unwrap() {
            let (mut e, ws) = workspace("uint8");
            let mut h = host();
            configure(&mut h, case);
            for (view, fixture) in [
                (View::Black, "black"),
                (View::White, "white"),
                (View::Mask, "black"),
            ] {
                h.app.view = view;
                compute(&mut h, &ws).unwrap();
                let function = h.app.script(&ws).unwrap();
                assert!(function.starts_with("function [BW, maskedRGBImage] = createMask(RGB)"));
                assert!(function.contains("strcmp(class(RGB)"));
                assert!(!function.contains("isa("));
                assert!(function.contains(&format!(
                    "numel(RGB) > {}",
                    matrust::matrix::MAX_ELEMENTS
                )));
                let toolkits::Action::SaveAppScript(AppKind::ColorThresholder, saved) =
                    h.generate(&ws, 42).unwrap()
                else {
                    panic!("save action")
                };
                assert!(saved.ends_with(&function));
                std::fs::write(dir.path().join("createMask.m"), saved).unwrap();
                let mut fresh = matrust::Engine::default();
                fresh.set_current_folder(dir.path()).unwrap();
                fresh
                    .insert_array("source", ws.get("RGB").unwrap().clone())
                    .unwrap();
                fresh
                    .execute("[BW, maskedRGBImage] = createMask(source);")
                    .unwrap();
                assert_eq!(fresh.array("BW").unwrap().class(), "logical");
                assert_eq!(numbers(&fresh.array("BW").unwrap()), expected(case, "mask"));
                assert_eq!(fresh.array("maskedRGBImage").unwrap().class(), "uint8");
                assert_eq!(
                    numbers(&fresh.array("maskedRGBImage").unwrap()),
                    expected(case, fixture)
                );
                h.app.mask_name = "selected".into();
                h.app.image_name = "segmented".into();
                let toolkits::Action::ExportScript(export) = h.export(&ws).unwrap() else {
                    panic!("export action")
                };
                apps::execute_export(&mut e, &export).unwrap();
                assert_eq!(
                    numbers(&e.array("selected").unwrap()),
                    expected(case, "mask")
                );
                assert_eq!(
                    numbers(&e.array("segmented").unwrap()),
                    expected(case, fixture)
                );
            }
        }
    }

    #[test]
    fn color_thresholder_classes_file_gray_and_last_good_result() {
        let mut h = host();
        for class in ["uint8", "uint16", "single", "double", "logical"] {
            let (mut e, ws) = workspace(class);
            let out = compute(&mut h, &ws).unwrap();
            assert_eq!(numbers(&out["ct_mask"]), vec![1.; 8]);
            h.app.view = View::White;
            h.app.invert = true;
            let toolkits::Action::ExportScript(script) = h.export(&ws).unwrap() else {
                panic!()
            };
            apps::execute_export(&mut e, &script).unwrap();
            assert_eq!(e.array("maskedRGBImage").unwrap().class(), class);
            let white = match class {
                "uint8" => 255.,
                "uint16" => 65535.,
                _ => 1.,
            };
            assert_eq!(
                numbers(&e.array("maskedRGBImage").unwrap()),
                vec![white; 24]
            );
            h.app.invert = false;
            e.execute("RGB = RGB(:,:,1);").unwrap();
            let mut gray = Snapshot::default();
            gray.sync(&e.workspace());
            assert!(h.request(&gray).err().unwrap().contains("gray"));
            assert_eq!(h.app.current.as_ref().unwrap().selected, 8);
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rgb'fixture.png");
        let mut pixels = image::RgbImage::new(4, 2);
        let rgb = reference()["rgb8"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u8)
            .collect::<Vec<_>>();
        for y in 0..2 {
            for x in 0..4 {
                pixels.put_pixel(
                    x,
                    y,
                    image::Rgb(std::array::from_fn(|c| {
                        rgb[y as usize + x as usize * 2 + c * 8]
                    })),
                );
            }
        }
        pixels.save(&path).unwrap();
        h.app.file = true;
        h.app.source = path.to_string_lossy().into_owned();
        let out = compute(&mut h, &Snapshot::default()).unwrap();
        assert_eq!(
            numbers(&out["ct_original"]),
            rgb.iter().map(|v| *v as f64 / 255.).collect::<Vec<_>>()
        );
        let gray_path = dir.path().join("gray.png");
        image::GrayImage::new(4, 2).save(&gray_path).unwrap();
        h.app.source = gray_path.to_string_lossy().into_owned();
        assert!(
            compute(&mut h, &Snapshot::default())
                .err()
                .unwrap()
                .contains("gray")
        );
        assert_eq!(h.app.current.as_ref().unwrap().selected, 8);
        h.app.source = dir
            .path()
            .join("missing.png")
            .to_string_lossy()
            .into_owned();
        assert!(compute(&mut h, &Snapshot::default()).is_err());
        assert_eq!(h.app.current.as_ref().unwrap().selected, 8);
    }

    #[test]
    fn color_thresholder_parser_names_collisions_and_export_rollback() {
        let (mut e, ws) = workspace("uint8");
        let mut h = host();
        compute(&mut h, &ws).unwrap();
        for bad in [
            "0; error('injected')",
            "0) ; evil=1; %",
            "0 % comment",
            "[0 1]",
            "NaN",
            "Inf",
            "-1",
            "2",
        ] {
            h.app.limits[0][0] = bad.into();
            assert!(h.request(&ws).is_err(), "{bad}");
        }
        h.app.limits[0][0] = "0.8".into();
        h.app.limits[0][1] = "0.2".into();
        assert!(h.request(&ws).is_err());
        h.refresh(&egui::Context::default(), &ws);
        assert!(h.status().0.contains("Only Hue"));
        assert_eq!(h.app.current.as_ref().unwrap().selected, 8);
        assert!(h.app.receive(&apps::Outputs::default()).is_err());
        assert_eq!(h.app.current.as_ref().unwrap().selected, 8);
        h.app.set_space(Space::Rgb);
        for bad in [
            "",
            "a b",
            "1abc",
            "for",
            "end",
            "x; y=1",
            "mrexp_old",
            "mrexp_had",
            "mrexp_complete",
            "matrust_export_error",
        ] {
            h.app.mask_name = bad.into();
            assert!(h.request(&ws).is_err(), "{bad}");
        }
        h.app.mask_name = h.app.image_name.clone();
        assert!(h.request(&ws).is_err());
        h.app.mask_name = "RGB".into();
        h.app.image_name = "ct_source1".into();
        e.execute("ct_apply = 12; ct_view = 13; ct_source = 14;")
            .unwrap();
        let mut collision_ws = Snapshot::default();
        collision_ws.sync(&e.workspace());
        let out = compute(&mut h, &collision_ws).unwrap();
        assert_eq!(numbers(&out["ct_mask"]), vec![1.; 8]);
        let toolkits::Action::ExportScript(export) = h.export(&collision_ws).unwrap() else {
            panic!()
        };
        apps::execute_export(&mut e, &export).unwrap();
        assert_eq!(e.array("RGB").unwrap().class(), "logical");
        assert_eq!(e.array("ct_source1").unwrap().class(), "uint8");
        let previous = e.array("RGB").unwrap();
        assert!(apps::execute_export(&mut e, &export).is_err()); // source is now gray
        assert_eq!(numbers(&e.array("RGB").unwrap()), numbers(&previous));
        assert_eq!(h.app.current.as_ref().unwrap().selected, 8);
        h.app.source = "RGB; error('injected')".into();
        assert!(h.request(&collision_ws).is_err());
    }

    fn big_workspace() -> Snapshot {
        let data = matrust::TypedArray::from_ints(
            matrust::NumericClass::Uint8,
            3000,
            4000 * 3,
            std::iter::repeat_n(128, 3000 * 4000 * 3),
        )
        .unwrap();
        let image = matrust::Array::Typed(data)
            .with_dims(&[3000, 4000, 3])
            .unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&std::collections::BTreeMap::from([("RGB".into(), image)]));
        ws
    }

    #[test]
    fn color_thresholder_headless_empty_large_runner_stop_and_texture_cache() {
        let _serial = LARGE_RUN
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let kind = AppKind::ColorThresholder;
        assert_eq!(AppKind::from_tab(kind.tab()), Some(kind));
        assert_eq!(
            serde_json::to_string(&kind.tab()).unwrap(),
            "\"color_thresholder\""
        );
        let ctx = egui::Context::default();
        let mut h = host();
        let render = |h: &mut Host<ColorThresholder>| {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000., 950.),
                    )),
                    ..Default::default()
                },
                |ui| h.app.main_ui(ui),
            )
        };
        assert!(!render(&mut h).shapes.is_empty());
        let ws = big_workspace();
        let request = h.request(&ws).unwrap();
        assert!(request.inputs[0].1.same_snapshot(ws.get("RGB").unwrap()));
        let mut runner = apps::Runner::default();
        let start = Instant::now();
        runner.submit(&ctx, request);
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(runner.running());
        assert!(!render(&mut h).shapes.is_empty());
        runner.stop();
        let deadline = Instant::now() + Duration::from_secs(30);
        while runner.running() {
            runner.poll(&ctx);
            assert!(Instant::now() < deadline, "cancel did not finish");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(runner.result().is_none());
        runner.submit(&ctx, h.request(&ws).unwrap());
        let deadline = Instant::now() + Duration::from_secs(120);
        while runner.running() {
            runner.poll(&ctx);
            assert!(Instant::now() < deadline, "large image timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
        let out = runner.result().unwrap().as_ref().unwrap();
        for name in ["ct_original", "ct_masked", "ct_mask"] {
            let dims = out[name].dims();
            assert_eq!(&dims[..2], &[1500, 2000]);
            assert!(dims[0] <= DISPLAY_SIDE && dims[1] <= DISPLAY_SIDE);
        }
        h.app.receive(out).unwrap();
        assert_eq!(h.app.current.as_ref().unwrap().selected, 12_000_000);
        assert_eq!(
            h.app.current.as_ref().unwrap().histograms[0]
                .iter()
                .sum::<f64>(),
            12_000_000.
        );
        let first = render(&mut h);
        assert!(!first.textures_delta.set.is_empty());
        let second = render(&mut h);
        assert!(
            second.textures_delta.set.is_empty(),
            "unchanged frame rebuilt a texture"
        );
        h.app.receive(out).unwrap();
        assert!(
            render(&mut h).textures_delta.set.is_empty(),
            "unchanged delivered data rebuilt a texture"
        );
        eprintln!(
            "color_thresholder 4000x3000 full-resolution Runner and render elapsed={:?}",
            start.elapsed()
        );
        // Exercise the app's actual Stop and reload buttons as well as Runner.
        let frame = |h: &mut Host<ColorThresholder>, events| {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1600., 950.),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| apps::color_thresholder::show(h, ui, &ws, false, &mut None),
            )
        };
        let position = |output: &egui::FullOutput, label: &str| {
            let mut found = Vec::new();
            for shape in &output.shapes {
                tool_tab_texts(&shape.shape, label, &mut found);
            }
            *found.first().unwrap_or_else(|| panic!("missing {label}"))
        };
        frame(&mut h, vec![]);
        let stop = position(&frame(&mut h, vec![]), "Stop");
        for pressed in [true, false] {
            frame(
                &mut h,
                vec![
                    egui::Event::PointerMoved(stop),
                    egui::Event::PointerButton {
                        pos: stop,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
        assert!(h.request(&ws).err().unwrap().contains("stopped"));
        let deadline = Instant::now() + Duration::from_secs(30);
        while h.running() {
            h.refresh(&ctx, &ws);
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        h.refresh(&ctx, &ws);
        assert!(
            !h.running(),
            "Stop must not automatically restart next frame"
        );
        assert_eq!(h.app.current.as_ref().unwrap().selected, 12_000_000);
        let reload = position(&frame(&mut h, vec![]), "Recompute / reload");
        for pressed in [true, false] {
            frame(
                &mut h,
                vec![
                    egui::Event::PointerMoved(reload),
                    egui::Event::PointerButton {
                        pos: reload,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
        assert!(h.request(&ws).is_ok());
    }

    #[test]
    fn color_thresholder_framework_empty_and_loaded_layouts() {
        let (_, ws) = workspace("uint8");
        for width in [820., 1600.] {
            let ctx = egui::Context::default();
            let mut h = host();
            for loaded in [false, true] {
                let snapshot = if loaded { &ws } else { &Snapshot::default() };
                if loaded {
                    compute(&mut h, snapshot).unwrap();
                }
                let frame = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 950.),
                        )),
                        ..Default::default()
                    },
                    |ui| apps::color_thresholder::show(&mut h, ui, snapshot, false, &mut None),
                );
                assert!(!frame.shapes.is_empty());
                for label in [
                    "INPUTS",
                    "OUTPUT",
                    "IMAGE AND COLOUR RANGES",
                    "Minimum",
                    "Maximum",
                ] {
                    let mut found = Vec::new();
                    for shape in &frame.shapes {
                        tool_tab_texts(&shape.shape, label, &mut found);
                    }
                    assert!(!found.is_empty(), "missing {label} at width {width}");
                }
            }
        }
    }

    #[test]
    fn color_thresholder_4000x3000_double_lab_under_local_limits() {
        let _serial = LARGE_RUN
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let image = big_workspace().get("RGB").unwrap().clone();
        // Same memory guards as the desktop Runner (`Limits::local()`), not the browser default.
        let mut e = matrust::Engine::with_limits(matrust::engine::Limits::local());
        e.insert_array("RGB", image).unwrap();
        e.execute("RGB = im2double(RGB);").unwrap();
        let mut full_ws = Snapshot::default();
        full_ws.sync(&e.workspace());
        let mut h = host();
        h.app.set_space(Space::Lab);
        let start = Instant::now();
        let out = apps::run(&mut e, h.request(&full_ws).unwrap()).unwrap();
        h.app.receive(&out).unwrap();
        assert_eq!(h.app.current.as_ref().unwrap().selected, 12_000_000);
        eprintln!(
            "color_thresholder 4000x3000 double Lab Limits::local elapsed={:?}",
            start.elapsed()
        );
        e.execute("small = RGB(1:8:end,1:8:end,:);").unwrap();
        let mut small_ws = Snapshot::default();
        small_ws.sync(&std::collections::BTreeMap::from([(
            "RGB".into(),
            e.array("small").unwrap(),
        )]));
        // Full-resolution capacity is the Lab run above. Repeat the four spaces on 375-by-500.
        let workers = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(8);
        for count in [1, workers] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(count)
                .build()
                .unwrap();
            pool.install(|| {
                let mut e = matrust::Engine::with_limits(matrust::engine::Limits::local());
                for space in Space::ALL {
                    let mut h = host();
                    h.app.set_space(space);
                    let start = Instant::now();
                    let out = apps::run(&mut e, h.request(&small_ws).unwrap()).unwrap();
                    h.app.receive(&out).unwrap();
                    assert_eq!(h.app.current.as_ref().unwrap().selected, 187_500);
                    eprintln!("color_thresholder release workers={count} RGB=500x375 double space={} elapsed={:?}", space.label(), start.elapsed());
                }
            });
        }
    }

    #[test]
    fn color_thresholder_rejects_nonfinite_out_of_range_and_complex_input() {
        for value in ["NaN", "Inf", "-0.1", "1.1", "1i"] {
            let mut engine = matrust::Engine::default();
            engine
                .execute(&format!("RGB = ones(2,2,3); RGB(1) = {value};"))
                .unwrap();
            let mut ws = Snapshot::default();
            ws.sync(&engine.workspace());
            let error = compute(&mut host(), &ws).unwrap_err();
            assert!(error.contains("Color Thresholder"), "{value}: {error}");
        }
    }

    #[test]
    fn color_thresholder_texture_orientation_and_reference_colours() {
        let reference = reference();
        let case = &reference["cases"][0];
        let (_, ws) = workspace("uint8");
        let mut h = host();
        configure(&mut h, case);
        compute(&mut h, &ws).unwrap();
        let ctx = egui::Context::default();
        let frame = ctx.run_ui(egui::RawInput::default(), |ui| h.app.main_ui(ui));
        let images: Vec<_> = frame
            .textures_delta
            .set
            .values()
            .flat_map(|deltas| deltas.iter())
            .filter(|delta| delta.image.size() == [4, 2])
            .map(|delta| {
                let egui::ImageData::Color(image) = &delta.image;
                image
                    .pixels
                    .iter()
                    .map(|pixel| pixel.to_array())
                    .collect::<Vec<_>>()
            })
            .collect();
        let rgba = |data: &serde_json::Value| {
            (0..8)
                .map(|pixel| {
                    let at = pixel / 4 + (pixel % 4) * 2;
                    [
                        data[at].as_u64().unwrap() as u8,
                        data[at + 8].as_u64().unwrap() as u8,
                        data[at + 16].as_u64().unwrap() as u8,
                        255,
                    ]
                })
                .collect::<Vec<_>>()
        };
        assert!(images.contains(&rgba(&reference["rgb8"])));
        assert!(images.contains(&rgba(&case["black"])));
    }

    #[test]
    fn color_thresholder_refuses_oversized_image_before_the_run() {
        let limit = matrust::matrix::MAX_ELEMENTS;
        let error =
            apps::color_thresholder::reject_image_size(5000, 4000, limit + 3).unwrap_err();
        assert!(error.contains("5000-by-4000-by-3"), "{error}");
        assert!(error.contains(&format!("{}", limit + 3)), "{error}");
        assert!(error.contains(&format!("limit of {limit}")), "{error}");
        assert!(apps::color_thresholder::reject_image_size(3000, 4000, 3000 * 4000 * 3).is_ok());
        let (_, ws) = workspace("uint8");
        let h = host();
        // Under the cap, request() builds the run; it does not refuse.
        assert!(h.request(&ws).is_ok());
        let script = h.app.script(&ws).unwrap();
        assert!(script.contains(&format!("numel(RGB) > {limit}")));
        assert!(script.contains(&format!("above the limit of {limit}")));
        assert!(script.contains("strcmp(class(RGB)"));
        assert!(!script.contains("isa("));
    }

    #[test]
    fn color_thresholder_run_error_stays_on_the_status_line() {
        let (_, ws) = workspace("uint8");
        let mut h = host();
        compute(&mut h, &ws).unwrap();
        assert_eq!(h.app.current.as_ref().unwrap().selected, 8);
        let mut engine = matrust::Engine::default();
        engine.execute("RGB = ones(2,2,3); RGB(1) = NaN;").unwrap();
        let mut bad = Snapshot::default();
        bad.sync(&engine.workspace());
        let ctx = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            h.refresh(&ctx, &bad);
            if !h.running() && h.status().0.contains("finite") {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "in-run error did not reach the status line: {}",
                h.status().0
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(h.status().1, toolkits::AppStatus::Error);
        // The failed run does not replace the last good view.
        assert_eq!(h.app.current.as_ref().unwrap().selected, 8);
    }
}
