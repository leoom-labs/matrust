// Image Segmenter app (apps/image_segmenter.rs): every step against
// scikit-image / SciPy references (tool/refs/image_segmenter/reference.py),
// the generated function in a fresh Engine, undo, active contour, Stop,
// export names, big images through the Runner and headless renders.
mod image_segmenter_tests {
    use super::*;
    use apps::image_segmenter::{
        ImageSegmenter, MAX_DISPLAY, MAX_STEPS, MorphOp, Shape, SourceKind, Step, StepKind,
    };
    use apps::{App, AppKind, Host, Snapshot};
    use toolkits::Tool;

    fn reference() -> serde_json::Value {
        serde_json::from_str(include_str!("../../tests/fixtures/apps/image_segmenter/reference.json"))
            .unwrap()
    }
    fn column(v: &serde_json::Value) -> Vec<f64> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_f64().unwrap())
            .collect()
    }
    fn mask(v: &serde_json::Value) -> Vec<bool> {
        column(v).into_iter().map(|x| x != 0.).collect()
    }
    /// A workspace holding `name` = class(data) of size rows × cols.
    fn image_ws(
        engine: &mut matrust::Engine,
        name: &str,
        rows: usize,
        cols: usize,
        data: Vec<f64>,
        class: &str,
    ) -> Snapshot {
        let m = matrust::Matrix::from_column_vec(rows, cols, data).unwrap();
        engine.insert_array(name, matrust::Array::Numeric(m)).unwrap();
        engine.execute(&format!("{name} = {class}({name});")).unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        ws
    }
    fn reference_ws(engine: &mut matrust::Engine) -> Snapshot {
        let r = reference();
        let (rows, cols) = (r["rows"].as_u64().unwrap() as usize, r["cols"].as_u64().unwrap() as usize);
        image_ws(engine, "X", rows, cols, column(&r["image"]), "uint8")
    }
    fn segmenter(variable: &str, steps: Vec<Step>) -> Host<ImageSegmenter> {
        let mut host = Host::new(ImageSegmenter::default());
        host.app.variable = variable.into();
        host.app.steps = steps;
        host
    }
    fn compute(host: &mut Host<ImageSegmenter>, ws: &Snapshot) -> Result<apps::Outputs, String> {
        host.app.prepare(ws);
        let outputs = apps::run(&mut matrust::Engine::default(), host.request(ws)?)?;
        host.app.receive(&outputs)?;
        Ok(outputs)
    }
    fn logical(engine: &matrust::Engine, name: &str) -> Vec<bool> {
        match engine.array(name).unwrap() {
            matrust::Array::Logical(m) => m.data().iter().map(|v| *v != 0).collect(),
            a => panic!("{name}: {}", a.class()),
        }
    }
    fn reals(engine: &matrust::Engine, name: &str) -> Vec<f64> {
        match engine.array(name).unwrap() {
            matrust::Array::Typed(t) => t.to_f64_vec().unwrap(),
            matrust::Array::Numeric(m) => m.data().to_vec(),
            a => panic!("{name}: {}", a.class()),
        }
    }
    fn step(kind: StepKind, edit: impl FnOnce(&mut Step)) -> Step {
        let mut s = Step::new(kind);
        edit(&mut s);
        s
    }
    fn view_mask(host: &Host<ImageSegmenter>) -> Vec<bool> {
        let v = host.app.view.as_ref().unwrap();
        assert_eq!(v.stride, 1);
        v.mask.clone()
    }
    fn mismatches(a: &[bool], b: &[bool]) -> usize {
        assert_eq!(a.len(), b.len());
        a.iter().zip(b).filter(|(x, y)| x != y).count()
    }
    fn request_error(host: &Host<ImageSegmenter>, ws: &Snapshot) -> String {
        match host.request(ws) {
            Err(error) => error,
            Ok(_) => panic!("expected an error"),
        }
    }
    fn otsu_then(kind: StepKind, edit: impl FnOnce(&mut Step)) -> Vec<Step> {
        vec![Step::new(StepKind::Otsu), step(kind, edit)]
    }

    #[test]
    fn image_segmenter_each_step_matches_scikit_image_and_scipy() {
        let r = reference();
        let mut engine = matrust::Engine::default();
        let ws = reference_ws(&mut engine);
        let morph = |op: MorphOp, shape: Shape, size: &str| {
            let size = size.to_owned();
            otsu_then(StepKind::Morphology, move |s| {
                s.op = op;
                s.shape = shape;
                s.size = size;
            })
        };
        let cases: Vec<(&str, Vec<Step>)> = vec![
            ("otsu", vec![Step::new(StepKind::Otsu)]),
            (
                "manual_0.5",
                vec![step(StepKind::Manual, |s| s.level = "0.5".into())],
            ),
            (
                "adaptive_bright_0.5",
                vec![step(StepKind::Adaptive, |s| s.sensitivity = "0.5".into())],
            ),
            (
                "adaptive_dark_0.6",
                vec![step(StepKind::Adaptive, |s| {
                    s.sensitivity = "0.6".into();
                    s.dark = true;
                })],
            ),
            ("fill_holes", otsu_then(StepKind::FillHoles, |_| {})),
            ("clear_border", otsu_then(StepKind::ClearBorder, |_| {})),
            (
                "remove_small_3",
                otsu_then(StepKind::RemoveSmall, |s| s.pixels = "3".into()),
            ),
            ("erode_disk_1", morph(MorphOp::Erode, Shape::Disk, "1")),
            ("dilate_disk_2", morph(MorphOp::Dilate, Shape::Disk, "2")),
            ("open_square_3", morph(MorphOp::Open, Shape::Square, "3")),
            ("close_disk_1", morph(MorphOp::Close, Shape::Disk, "1")),
            ("invert", otsu_then(StepKind::Invert, |_| {})),
            (
                "flood_6_13_0.1",
                vec![step(StepKind::FloodFill, |s| {
                    s.point = Some([6, 13]);
                    s.tolerance = "0.1".into();
                })],
            ),
        ];
        for (name, steps) in cases {
            let mut host = segmenter("X", steps);
            compute(&mut host, &ws).unwrap_or_else(|e| panic!("{name}: {e}"));
            let expected = mask(&r["masks"][name]);
            assert_eq!(mismatches(&view_mask(&host), &expected), 0, "{name}");
            let v = host.app.view.as_ref().unwrap();
            assert_eq!(v.area, expected.iter().filter(|b| **b).count(), "{name}");
            assert_eq!(v.regions as u64, r["counts"][name].as_u64().unwrap(), "{name}");
            assert_eq!((v.rows, v.cols, v.channels), (32, 40, 1));
        }

        // k-means: the brightest of three clusters.
        let mut engine = matrust::Engine::default();
        let ws = image_ws(&mut engine, "K", 32, 40, column(&r["kmeans"]["image"]), "uint8");
        let mut host = segmenter(
            "K",
            vec![step(StepKind::KMeans, |s| {
                s.k = "3".into();
                s.cluster = "1".into();
            })],
        );
        compute(&mut host, &ws).unwrap();
        assert_eq!(mismatches(&view_mask(&host), &mask(&r["kmeans"]["mask"])), 0);

        // Watershed: two overlapping discs become two regions, split by a
        // line inside the original mask.
        let mut engine = matrust::Engine::default();
        let discs = mask(&r["watershed"]["mask"]);
        let data = discs.iter().map(|b| f64::from(*b)).collect();
        let ws = image_ws(&mut engine, "W", 32, 40, data, "logical");
        let mut host = segmenter(
            "W",
            vec![
                step(StepKind::Manual, |s| s.level = "0.5".into()),
                Step::new(StepKind::Watershed),
            ],
        );
        compute(&mut host, &ws).unwrap();
        let v = host.app.view.as_ref().unwrap();
        assert_eq!(v.regions as u64, r["watershed"]["regions"].as_u64().unwrap());
        let split = view_mask(&host);
        assert!(split.iter().zip(&discs).all(|(s, d)| !s || *d));
        let removed = discs.iter().filter(|b| **b).count() - v.area;
        assert!((1..=40).contains(&removed), "watershed line of {removed} px");
    }

    #[test]
    fn image_segmenter_three_step_pipeline_matches_generated_function_in_fresh_engine() {
        let r = reference();
        let mut engine = matrust::Engine::default();
        let ws = reference_ws(&mut engine);
        let mut host = segmenter(
            "X",
            vec![
                Step::new(StepKind::Otsu),
                Step::new(StepKind::FillHoles),
                step(StepKind::RemoveSmall, |s| s.pixels = "3".into()),
            ],
        );
        compute(&mut host, &ws).unwrap();
        let expected = mask(&r["masks"]["pipeline"]);
        assert_eq!(mismatches(&view_mask(&host), &expected), 0);
        let v = host.app.view.as_ref().unwrap();
        assert_eq!(v.regions as u64, r["counts"]["pipeline"].as_u64().unwrap());

        let script = host.app.script(&ws).unwrap();
        assert!(script.contains("function [BW, maskedImage] = segmentImage(X)"));
        assert!(script.contains("[BW, maskedImage] = segmentImage(X);"));
        let saved = match host.generate(&ws, 42).unwrap() {
            toolkits::Action::SaveAppScript(AppKind::ImageSegmenter, text) => text,
            _ => panic!("wrong save action"),
        };
        assert!(saved.ends_with(&script));
        // Only the function, called on the image in a fresh Engine.
        let mut fresh = matrust::Engine::default();
        let image = engine.array("X").unwrap();
        fresh.insert_array("img", image.clone()).unwrap();
        let function = host.app.function(&ws).unwrap();
        fresh
            .execute(&format!("[m, mi] = segmentImage(img);\n{function}"))
            .unwrap();
        assert_eq!(mismatches(&logical(&fresh, "m"), &expected), 0);
        let original = reals(&engine, "X");
        let masked = reals(&fresh, "mi");
        assert_eq!(fresh.array("mi").unwrap().class(), "uint8");
        for ((m, x), b) in masked.iter().zip(&original).zip(&expected) {
            assert_eq!(*m, if *b { *x } else { 0. });
        }
        // The saved script in a fresh Engine with X.
        let mut fresh = matrust::Engine::default();
        fresh.insert_array("X", image.clone()).unwrap();
        fresh.execute(&saved).unwrap();
        assert_eq!(logical(&fresh, "BW"), expected);
        assert_eq!(reals(&fresh, "maskedImage"), masked);
        // Export: the same names in the workspace.
        let export = match host.export(&ws).unwrap() {
            toolkits::Action::ExportScript(s) => s,
            _ => panic!("wrong export"),
        };
        let mut exported = matrust::Engine::default();
        exported.insert_array("X", image).unwrap();
        exported.execute(&export).unwrap();
        assert_eq!(logical(&exported, "BW"), expected);
        assert!(exported.array("segmentImage").is_none());
    }

    #[test]
    fn image_segmenter_undo_restores_the_previous_mask_exactly() {
        let mut engine = matrust::Engine::default();
        let ws = reference_ws(&mut engine);
        let mut host = segmenter("X", Vec::new());
        assert!(!host.app.can_undo());
        host.app.add(StepKind::Manual).unwrap();
        host.app.steps[0].level = "0.45".into();
        compute(&mut host, &ws).unwrap();
        let first = view_mask(&host);
        let steps = host.app.steps.clone();
        host.app.add(StepKind::Morphology).unwrap();
        host.app.add(StepKind::Invert).unwrap();
        host.app.move_up(2);
        assert_eq!(host.app.steps[1].kind, StepKind::Invert);
        compute(&mut host, &ws).unwrap();
        assert_ne!(view_mask(&host), first);
        host.app.undo();
        assert_eq!(host.app.steps[2].kind, StepKind::Invert);
        host.app.remove(1);
        host.app.undo();
        host.app.undo();
        host.app.undo();
        assert_eq!(host.app.steps, steps);
        compute(&mut host, &ws).unwrap();
        assert_eq!(view_mask(&host), first);
        // A flood fill seed from a click is one undo step; clicks need a
        // selected flood fill step.
        host.app.selected = Some(0);
        assert!(!host.app.click(3, 4));
        host.app.add(StepKind::FloodFill).unwrap();
        assert!(request_error(&host, &ws).contains("click the image"));
        assert!(host.app.click(6, 13));
        assert_eq!(host.app.steps[1].point, Some([6, 13]));
        host.app.undo();
        assert_eq!(host.app.steps[1].point, None);
        // At most MAX_STEPS.
        while host.app.steps.len() < MAX_STEPS {
            host.app.add(StepKind::Invert).unwrap();
        }
        assert!(host.app.add(StepKind::Invert).is_err());
    }

    #[test]
    fn image_segmenter_active_contour_grows_a_seed_to_the_noisy_disc() {
        let r = reference();
        let n = r["contour"]["size"].as_u64().unwrap() as usize;
        let mut engine = matrust::Engine::default();
        let ws = image_ws(&mut engine, "D", n, n, column(&r["contour"]["image"]), "double");
        let truth = mask(&r["contour"]["truth"]);
        let iou = |m: &[bool]| {
            let both = m.iter().zip(&truth).filter(|(a, b)| **a && **b).count();
            let any = m.iter().zip(&truth).filter(|(a, b)| **a || **b).count();
            both as f64 / any as f64
        };
        // A sparse seed: the brightest noisy pixels inside the disc.
        let mut host = segmenter("D", vec![step(StepKind::Manual, |s| s.level = "0.85".into())]);
        compute(&mut host, &ws).unwrap();
        let seed = iou(&view_mask(&host));
        assert!(seed < 0.1, "seed IoU {seed}");
        host.app.add(StepKind::ActiveContour).unwrap();
        host.app.steps[1].iterations = "300".into();
        compute(&mut host, &ws).unwrap();
        let grown = iou(&view_mask(&host));
        assert!(grown > 0.95, "active contour IoU {grown}");
        // Invalid fields name the step and never reach a script.
        host.app.steps[1].iterations = "300; delete('x')".into();
        let error = request_error(&host, &ws);
        assert!(error.contains("step 2") && error.contains("Iterations"), "{error}");
        host.app.steps[1].iterations = "0".into();
        assert!(request_error(&host, &ws).contains("from 1 to 10000"));
    }

    /// Calls refresh until the run settles; each call returns quickly.
    fn settle(host: &mut Host<ImageSegmenter>, ctx: &egui::Context, ws: &Snapshot) -> Duration {
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut slowest = Duration::ZERO;
        loop {
            let start = Instant::now();
            host.refresh(ctx, ws);
            slowest = slowest.max(start.elapsed());
            if !host.running() {
                return slowest;
            }
            assert!(Instant::now() < deadline, "{:?}", host.status());
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn image_segmenter_stop_cancels_a_long_active_contour() {
        let ctx = egui::Context::default();
        let mut engine = matrust::Engine::default();
        engine
            .execute("rng(3); N = 0.4 + 0.2*rand(750, 1000);")
            .unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        let mut host = segmenter("N", vec![Step::new(StepKind::Otsu)]);
        settle(&mut host, &ctx, &ws);
        let before = host.app.view.as_ref().unwrap().generation;
        host.app.add(StepKind::ActiveContour).unwrap();
        // Edge-based contours on noise do not converge early.
        host.app.steps[1].iterations = "10000".into();
        host.app.steps[1].edge = true;
        host.refresh(&ctx, &ws);
        std::thread::sleep(Duration::from_millis(300));
        host.refresh(&ctx, &ws);
        assert!(host.running(), "the long contour is still running");
        let start = Instant::now();
        host.app.stop();
        settle(&mut host, &ctx, &ws);
        let stopped = start.elapsed();
        assert!(stopped < Duration::from_secs(5), "stop took {stopped:?}");
        let (status, _) = host.status();
        assert!(status.contains("stopped"), "{status}");
        // The last good view stays.
        assert_eq!(host.app.view.as_ref().unwrap().generation, before);
        // Editing the steps (or Run) computes again.
        host.app.steps[1].iterations = "2".into();
        settle(&mut host, &ctx, &ws);
        assert!(!host.app.is_stopped());
        assert!(host.app.view.as_ref().unwrap().generation > before);
        eprintln!("image_segmenter stop of a 750x1000 active contour took {stopped:?}");
    }

    #[test]
    fn image_segmenter_export_names_are_checked() {
        let mut engine = matrust::Engine::default();
        engine.execute("segmentImage = 3; BW = 7;").unwrap();
        let ws = reference_ws(&mut engine);
        let mut host = segmenter("X", vec![Step::new(StepKind::Otsu)]);
        for name in ["mrexp_old", "mrexp_had", "matrust_export_error", "mrexp_complete"] {
            host.app.mask_name = name.into();
            let error = host.app.script(&ws).unwrap_err();
            assert!(error.contains("reserved"), "{name}: {error}");
        }
        for (mask, masked, problem) in [
            ("1bad", "maskedImage", "valid variable name"),
            ("a b", "maskedImage", "valid variable name"),
            ("BW", "BW", "different names"),
            ("X", "maskedImage", "source image"),
            ("is_probe_gray", "maskedImage", "reserved"),
            ("BW", "for", "expression"),
        ] {
            host.app.mask_name = mask.into();
            host.app.masked_name = masked.into();
            let error = host.app.script(&ws).unwrap_err();
            assert!(error.contains(problem), "{mask}/{masked}: {error}");
        }
        // A workspace variable named segmentImage renames the function, so
        // the call is not an index; Export replaces BW and nothing else.
        host.app.mask_name = "BW".into();
        host.app.masked_name = "masked".into();
        compute(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(script.contains("= segmentImage1(X);"), "{script}");
        assert_eq!(host.app.exported(&ws), ["BW", "masked"]);
        let export = match host.export(&ws).unwrap() {
            toolkits::Action::ExportScript(s) => s,
            _ => panic!("wrong export"),
        };
        engine.execute(&export).unwrap();
        assert_eq!(engine.array("BW").unwrap().class(), "logical");
        assert_eq!(engine.array("masked").unwrap().class(), "uint8");
        assert_eq!(reals(&engine, "segmentImage"), [3.]);
        // A failing export restores the earlier values.
        let mut fail = matrust::Engine::default();
        fail.execute("X = 'not an image'; BW = 7;").unwrap();
        assert!(fail.execute(&export).is_err());
        assert_eq!(reals(&fail, "BW"), [7.]);
        assert!(fail.array("masked").is_none());
        // Sources: non-images are refused; files go through imread.
        let mut bad = matrust::Engine::default();
        bad.execute("C = {1}; Z = complex(ones(4), 1); V = ones(3, 3, 2);").unwrap();
        let mut bad_ws = Snapshot::default();
        bad_ws.sync(&bad.workspace());
        for name in ["C", "Z", "V", "missing"] {
            host.app.variable = name.into();
            assert!(host.app.script(&bad_ws).is_err(), "{name}");
        }
        host.app.source = SourceKind::File;
        host.app.file = "it's.png".into();
        let script = host.app.script(&bad_ws).unwrap();
        assert!(script.contains("segmentImage(imread('it''s.png'))"), "{script}");
        host.app.file = "a.png\nerror('x')".into();
        assert!(host.app.script(&bad_ws).is_err());
    }

    #[test]
    fn image_segmenter_file_source_reads_with_imread_and_says_rgb() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rgb.png");
        let mut engine = matrust::Engine::default();
        engine
            .execute(&format!(
                "A = zeros(20, 30, 3, 'uint8'); A(5:15, 8:20, 1) = 200; A(5:15, 8:20, 2) = 180; imwrite(A, '{}');",
                path.display()
            ))
            .unwrap();
        let ws = Snapshot::default();
        let mut host = Host::new(ImageSegmenter::default());
        host.app.source = SourceKind::File;
        host.app.file = path.display().to_string();
        host.app.steps = vec![Step::new(StepKind::Otsu)];
        compute(&mut host, &ws).unwrap();
        let v = host.app.view.as_ref().unwrap();
        assert_eq!((v.rows, v.cols, v.channels, v.regions, v.area), (20, 30, 3, 1, 11 * 13));
        let rows = host.app.results();
        assert!(rows.iter().any(|(_, value)| value.contains("RGB → gray")), "{rows:?}");
    }

    fn big_rgb(rows: usize, cols: usize) -> Snapshot {
        // Same memory guards as the desktop Runner (`Limits::local()`).
        let mut engine = matrust::Engine::with_limits(matrust::engine::Limits::local());
        engine
            .execute(&format!(
                "rng(5); P = uint8(60 + 40*rand({rows}, {cols}, 3)); P(100:400, 200:600, :) = 220;"
            ))
            .unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        ws
    }
    fn big_image_stays_responsive(rows: usize, cols: usize) {
        let ctx = egui::Context::default();
        let ws = big_rgb(rows, cols);
        let mut host = segmenter(
            "P",
            vec![
                Step::new(StepKind::Otsu),
                Step::new(StepKind::FillHoles),
                step(StepKind::RemoveSmall, |s| s.pixels = "100".into()),
            ],
        );
        let start = Instant::now();
        let slowest = settle(&mut host, &ctx, &ws);
        let elapsed = start.elapsed();
        assert!(slowest < Duration::from_millis(250), "UI call took {slowest:?}");
        let v = host.app.view.as_ref().unwrap_or_else(|| panic!("{:?}", host.status()));
        assert_eq!((v.rows, v.cols, v.channels), (rows, cols, 3));
        assert!(v.display[0] <= MAX_DISPLAY && v.display[1] <= MAX_DISPLAY);
        assert_eq!(v.stride, rows.max(cols).div_ceil(MAX_DISPLAY));
        assert_eq!(v.regions, 1);
        assert_eq!(v.area, 301 * 401);
        let image = apps::image_segmenter::compose(v, egui::Color32::RED, 0.5, false);
        assert_eq!(image.size, [v.display[1], v.display[0]]);
        eprintln!(
            "image_segmenter {rows}x{cols} RGB, 3 steps: {elapsed:?}, slowest UI call {slowest:?}"
        );
    }

    #[test]
    fn image_segmenter_1000x750_rgb_stays_responsive() {
        big_image_stays_responsive(750, 1000);
    }

    #[test]
    fn image_segmenter_4000x3000_rgb_stays_responsive() {
        big_image_stays_responsive(3000, 4000);
    }

    #[test]
    fn image_segmenter_click_maps_to_full_image_pixels() {
        use apps::image_segmenter::pixel;
        let rect = egui::Rect::from_min_size(egui::pos2(10., 20.), egui::vec2(200., 100.));
        assert_eq!(pixel(rect, egui::pos2(10., 20.), 3000, 4000), [1, 1]);
        assert_eq!(pixel(rect, egui::pos2(210., 120.), 3000, 4000), [3000, 4000]);
        assert_eq!(pixel(rect, egui::pos2(110., 70.), 3000, 4000), [1501, 2001]);
    }

    fn texts(shape: &egui::epaint::Shape, found: &mut Vec<String>) {
        match shape {
            egui::epaint::Shape::Text(t) => found.push(t.galley.text().to_owned()),
            egui::epaint::Shape::Vec(v) => v.iter().for_each(|s| texts(s, found)),
            _ => {}
        }
    }
    fn painted(output: &egui::FullOutput) -> Vec<String> {
        let mut found = Vec::new();
        for s in &output.shapes {
            texts(&s.shape, &mut found);
        }
        found
    }

    #[test]
    fn image_segmenter_tool_tab_renders_headless_with_no_image_and_0_1_10_steps() {
        let kind = AppKind::ImageSegmenter;
        let tab = kind.tab();
        assert_eq!(AppKind::from_tab(tab), Some(kind));
        assert_eq!(serde_json::to_string(&tab).unwrap(), "\"image_segmenter\"");
        assert!(kind.is_built() && AppKind::ALL.contains(&kind));
        for width in [820., 1600.] {
            let size = egui::vec2(width, 950.);
            let ctx = egui::Context::default();
            let mut app = Workspace::with_context(ctx.clone(), true);
            app.run("Q = uint8(40 + 30*magic(60)/3600*6); Q(20:40, 15:45) = 230;".into());
            finish(&mut app);
            app.toolkits.open_tool(Tool::App(kind));
            let output = frame_at(&mut app, &ctx, size, vec![]);
            assert!(painted(&output).iter().any(|t| t == "INPUTS"));
            assert!(app.toolkits.apps.segmenter.app.view.is_none());
            assert!(app.toolkits.apps.segmenter.status().0.contains("choose a workspace image"));
            app.toolkits.apps.segmenter.app.variable = "Q".into();
            for count in [0, 1, 10] {
                let host = &mut app.toolkits.apps.segmenter;
                host.app.steps.clear();
                host.app.selected = None;
                for k in 0..count {
                    let kind = [StepKind::Otsu, StepKind::FillHoles, StepKind::Invert][k % 3];
                    host.app.add(kind).unwrap();
                }
                let deadline = Instant::now() + Duration::from_secs(30);
                let output = loop {
                    let output = frame_at(&mut app, &ctx, size, vec![]);
                    let host = &app.toolkits.apps.segmenter;
                    if host.app.view.is_some() && !host.running() {
                        break frame_at(&mut app, &ctx, size, vec![]);
                    }
                    assert!(Instant::now() < deadline, "{:?}", host.status());
                    drop(output);
                    std::thread::sleep(Duration::from_millis(5));
                };
                let text = painted(&output);
                for label in ["INPUTS", "IMAGE AND MASK", "OUTPUT", "Add step", "Foreground area"] {
                    assert!(text.iter().any(|t| t == label), "{count} steps, {width}: {label}");
                }
                let host = &app.toolkits.apps.segmenter;
                assert_eq!(host.app.steps.len(), count);
                let (status, _) = host.status();
                assert!(status.starts_with("Image Segmenter:"), "{status}");
                assert!(host.app.view.is_some());
            }
            assert_eq!(app.toolkits.selected_tool(), Some(Tool::App(kind)));
            app.close_tool_tab(Tool::App(kind));
        }
    }
}
