// Registration Estimator: scripts, Runner, export and real egui interaction.
// Images are synthetic (built here); file inputs are written to a tempdir.
mod registration_estimator_tests {
    use super::*;
    use apps::{
        App, AppKind, Host, Snapshot,
        registration_estimator::{
            self as re, Overlay, RegistrationEstimator, Source, Technique, TransformType,
        },
    };
    use matrust::engine::regest_app_hooks::{self, Launch};
    use std::sync::{Arc, Mutex};

    /// Fixed: three Gaussian blobs (80x96). Moving: fixed turned 6 degrees
    /// and shifted [3 -2] (a known rigid transformation) in the same frame.
    const IMAGES: &str = "[xx,yy] = meshgrid(1:96,1:80); \
        fix = exp(-((xx-40).^2+(yy-36).^2)/120) + 0.6*exp(-((xx-62).^2+(yy-50).^2)/60) \
        + 0.3*exp(-((xx-30).^2+(yy-58).^2)/40); \
        mov = imwarp(fix, rigidtform2d(6,[3 -2]), 'OutputView', imref2d(size(fix))); \
        clear xx yy;";

    fn workspace(code: &str) -> (matrust::Engine, Snapshot) {
        // The desktop's limits: large test images need more than the default budget.
        let mut engine = matrust::Engine::with_limits(matrust::engine::Limits::local());
        engine.execute(code).unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        (engine, ws)
    }
    fn host(engine: &matrust::Engine) -> Host<RegistrationEstimator> {
        let mut h = Host::new(RegistrationEstimator::default());
        h.app.catalogue = regest_app_hooks::catalogue(engine);
        h
    }
    fn frame_at(
        h: &mut Host<RegistrationEstimator>,
        ws: &Snapshot,
        ctx: &egui::Context,
        size: egui::Vec2,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Option<toolkits::Action>) {
        let mut action = None;
        let output = crate::headless::run(
            ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |ui| re::show(h, ui, ws, false, &mut action),
        );
        (output, action)
    }
    fn frame(
        h: &mut Host<RegistrationEstimator>,
        ws: &Snapshot,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Option<toolkits::Action>) {
        frame_at(h, ws, ctx, egui::vec2(1600., 1000.), events)
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
        h: &mut Host<RegistrationEstimator>,
        ws: &Snapshot,
        ctx: &egui::Context,
        name: &str,
    ) -> Option<toolkits::Action> {
        let rect = *h
            .app
            .controls
            .0
            .get(name)
            .unwrap_or_else(|| panic!("no control {name}"));
        let pos = rect.center();
        frame(
            h,
            ws,
            ctx,
            vec![egui::Event::PointerMoved(pos), pointer(pos, true)],
        );
        let action = frame(h, ws, ctx, vec![pointer(pos, false)]).1;
        // Settle hover/popup state.
        frame(h, ws, ctx, vec![]);
        action
    }
    /// Opens a combo box, then clicks one of its entries.
    fn pick(
        h: &mut Host<RegistrationEstimator>,
        ws: &Snapshot,
        ctx: &egui::Context,
        combo: &str,
        item: &str,
    ) {
        h.app.controls.0.remove(item);
        click(h, ws, ctx, combo);
        assert!(h.app.controls.0.contains_key(item), "{item} not listed");
        click(h, ws, ctx, item);
    }
    fn wait(h: &mut Host<RegistrationEstimator>, ws: &Snapshot, ctx: &egui::Context) {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            frame(h, ws, ctx, vec![]);
            if !h.running() && !h.app.view_running() {
                break;
            }
            assert!(Instant::now() < deadline, "Registration Estimator timed out");
            std::thread::sleep(Duration::from_millis(2));
        }
        frame(h, ws, ctx, vec![]);
    }
    fn select(h: &mut Host<RegistrationEstimator>, moving: &str, fixed: &str) {
        h.app.moving.from_file = false;
        h.app.moving.variable = moving.into();
        h.app.fixed.from_file = false;
        h.app.fixed.variable = fixed.into();
    }
    fn register(h: &mut Host<RegistrationEstimator>, ws: &Snapshot, ctx: &egui::Context) {
        h.app.register_requested = true;
        wait(h, ws, ctx);
        assert!(h.app.error.is_none(), "{:?}", h.app.error);
    }
    fn logical(e: &mut matrust::Engine, code: &str) -> bool {
        e.execute(&format!("re_test_flag = logical({code});"))
            .unwrap();
        let matrust::Array::Logical(flag) = e.array("re_test_flag").unwrap() else {
            panic!("logical")
        };
        flag.data()[0] == 1
    }
    fn scalar(e: &matrust::Engine, name: &str) -> f64 {
        let matrust::Array::Numeric(m) = e.array(name).unwrap() else {
            panic!("numeric {name}")
        };
        m.data()[0]
    }
    /// Plain values of the four outputs (objects become their properties),
    /// so results from different engines can be compared with isequal.
    fn outputs(e: &mut matrust::Engine, names: &[String; 4]) -> Vec<matrust::Array> {
        let [t, image, r, q] = names;
        e.execute(&format!(
            "re_v1 = {t}.A; re_v2 = class({t}); re_v3 = {image}; \
             re_v4 = [{r}.ImageSize {r}.XWorldLimits {r}.YWorldLimits]; re_v5 = {q};"
        ))
        .unwrap();
        (1..=5)
            .map(|k| e.array(&format!("re_v{k}")).unwrap())
            .collect()
    }
    fn assert_same(a: &[matrust::Array], b: &[matrust::Array]) {
        let mut e = matrust::Engine::default();
        for (k, (x, y)) in a.iter().zip(b).enumerate() {
            e.insert_array("re_a", x.clone()).unwrap();
            e.insert_array("re_b", y.clone()).unwrap();
            assert!(logical(&mut e, "isequal(re_a, re_b)"), "output {k} differs");
        }
    }
    /// Export (in a copy of the workspace engine) and the saved script (in
    /// a fresh engine) give the same values as each other and as the run.
    fn export_equals_saved(
        h: &mut Host<RegistrationEstimator>,
        ws: &Snapshot,
        source: &matrust::Engine,
    ) -> Vec<matrust::Array> {
        let script = h.app.script(ws).unwrap();
        let toolkits::Action::SaveAppScript(kind, saved) = h.generate(ws, 0).unwrap() else {
            panic!("script action")
        };
        assert_eq!(kind, AppKind::RegistrationEstimator);
        assert!(saved.ends_with(&script));
        let mut fresh = matrust::Engine::default();
        // A fresh engine: arrays copied, objects rebuilt from the catalogue.
        let geo = regest_app_hooks::catalogue(source);
        for (name, array) in source.workspace() {
            match geo.iter().find(|g| g.name == name) {
                Some(g) => g.install(&mut fresh).unwrap(),
                None => fresh.insert_array(&name, array).unwrap(),
            }
        }
        fresh.execute(&saved).unwrap();
        let names = h.app.names.clone();
        let expected = outputs(&mut fresh, &names);
        let toolkits::Action::ExportScript(export) = h.export(ws).unwrap() else {
            panic!("export action")
        };
        let mut engine = source.clone();
        apps::execute_export(&mut engine, &export).unwrap();
        let exported = outputs(&mut engine, &names);
        assert_same(&exported, &expected);
        let run = h.app.selected_run().unwrap();
        let mut shown = expected.clone();
        shown[2] = run.registered.clone();
        shown[4] = matrust::Array::Numeric(matrust::Matrix::scalar(run.quality));
        assert_same(&shown, &expected);
        expected
    }

    #[test]
    fn registration_estimator_export_and_saved_script_reproduce_runs() {
        let ctx = egui::Context::default();
        let (mut source, _) = workspace(IMAGES);
        source
            .execute("rgb = single(cat(3, mov, mov, mov)); Rfix = imref2d(size(fix)); t0 = rigidtform2d(-4, [0 0]);")
            .unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&source.workspace());
        let mut h = host(&source);
        select(&mut h, "mov", "fix");
        register(&mut h, &ws, &ctx);
        let run = h.app.selected_run().unwrap();
        assert_eq!(run.class, "rigidtform2d");
        let angle = run.matrix[1][0].atan2(run.matrix[0][0]).to_degrees();
        // The inverse of the known rotation; translation of the inverse too.
        assert!((angle + 6.).abs() < 1., "rotation {angle}");
        assert!(run.quality > 0.999, "correlation {}", run.quality);
        let script = h.app.script(&ws).unwrap();
        for part in [
            "[tform, movingRegistered, Rregistered, registrationQuality] = matrust_registration(mov, fix);",
            "[optimizer, metric] = imregconfig('monomodal');",
            "optimizer.MaximumIterations = 100;",
            "optimizer.MaximumStepLength = 0.0625;",
            "imregtform(movingGray, Rmoving, fixedGray, Rfixed, 'rigid', optimizer, metric, 'PyramidLevels', 3);",
            "registered = imwarp(moving, Rmoving, tform, 'OutputView', Rfixed);",
            "quality = sum(a .* b) / sqrt(sum(a .^ 2) * sum(b .^ 2));",
        ] {
            assert!(script.contains(part), "missing {part}\n{script}");
        }
        let first = export_equals_saved(&mut h, &ws, &source);
        // The quality value exported is the run's normalized correlation.
        let mut e = matrust::Engine::default();
        e.insert_array("q", first[4].clone()).unwrap();
        assert!(scalar(&e, "q") > 0.999);

        // RGB moving, an imref2d for fixed, an initial transformation and
        // other settings: registered on the gray version, colour kept.
        // (Mean squares needs matching intensity ranges: single like fix.)
        select(&mut h, "rgb", "fix");
        h.app.fixed.reference = "Rfix".into();
        h.app.settings = re::Settings {
            technique: Technique::Monomodal,
            transform: TransformType::Similarity,
            levels: 2,
            iterations: 150,
            step: 0.125,
            initial: "t0".into(),
        };
        register(&mut h, &ws, &ctx);
        let run = h.app.selected_run().unwrap();
        assert_eq!(h.app.runs.len(), 1, "other images start a new list");
        assert_eq!(run.class, "simtform2d");
        assert_eq!(run.registered.dims().as_ref(), [80usize, 96, 3].as_slice());
        assert_eq!(run.registered.class(), "single");
        assert!(run.quality > 0.99, "{}", run.quality);
        let script = h.app.script(&ws).unwrap();
        for part in [
            "matrust_registration(rgb, fix, Rfix, t0);",
            "function [tform, registered, Rregistered, quality] = matrust_registration(moving, fixed, Rfixed, tformInitial)",
            "Rmoving = imref2d(size(moving));",
            "movingGray = rgb2gray(moving);",
            "optimizer.MaximumIterations = 150;",
            "optimizer.MaximumStepLength = 0.125;",
            "'similarity', optimizer, metric, 'PyramidLevels', 2, 'InitialTransformation', tformInitial);",
        ] {
            assert!(script.contains(part), "missing {part}\n{script}");
        }
        assert!(!script.contains("Rfixed = imref2d"));
        export_equals_saved(&mut h, &ws, &source);

        // File inputs (PNG in a tempdir) read with imread by the script.
        let dir = tempfile::tempdir().unwrap();
        let moving_file = dir.path().join("moving image.png");
        let fixed_file = dir.path().join("fixed.png");
        source
            .execute(&format!(
                "imwrite(uint8(255*mov), '{}'); imwrite(uint8(255*fix), '{}');",
                moving_file.display(),
                fixed_file.display()
            ))
            .unwrap();
        h.app.moving.from_file = true;
        h.app.moving.path = moving_file.to_string_lossy().into_owned();
        h.app.fixed.from_file = true;
        h.app.fixed.path = fixed_file.to_string_lossy().into_owned();
        h.app.fixed.reference.clear();
        h.app.settings = re::Settings::default();
        register(&mut h, &ws, &ctx);
        let script = h.app.script(&ws).unwrap();
        assert!(script.contains("= matrust_registration();"), "{script}");
        assert!(script.contains("moving = imread('"));
        assert!(script.contains("fixed = imread('"));
        assert!(matches!(h.app.selected_run().unwrap().spec.moving, Source::File(_)));
        assert!(h.app.selected_run().unwrap().quality > 0.99);
        export_equals_saved(&mut h, &ws, &source);
    }

    #[test]
    fn registration_estimator_errors_inline_keep_last_result_and_export_rollback() {
        let ctx = egui::Context::default();
        let (mut source, _) = workspace(IMAGES);
        source
            .execute("Rsmall = imref2d([10 10]); mask = mov > 0.5; tiny = mov(1:8, 1:8); old = 42;")
            .unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&source.workspace());
        let mut h = host(&source);
        select(&mut h, "mov", "fix");
        register(&mut h, &ws, &ctx);
        let good = h.app.selected_run().unwrap().id;
        let view = h.app.view.clone().unwrap();
        // Size mismatch: an imref2d of another size than the moving image.
        // The engine's message shows inline; the last result and its view stay.
        h.app.moving.reference = "Rsmall".into();
        h.app.register_requested = true;
        wait(&mut h, &ws, &ctx);
        let error = h.app.error.clone().unwrap();
        assert!(error.contains("must match the size of moving"), "{error}");
        assert_eq!(h.status().1, toolkits::AppStatus::Error);
        assert_eq!(h.app.selected_run().unwrap().id, good);
        assert!(Arc::ptr_eq(&view, h.app.view.as_ref().unwrap()));
        let (out, _) = frame(&mut h, &ws, &ctx, vec![]);
        assert!(!out.shapes.is_empty());
        // Type mismatch: a logical image is refused before running.
        h.app.moving.reference.clear();
        h.app.moving.variable = "mask".into();
        h.app.register_requested = true;
        frame(&mut h, &ws, &ctx, vec![]);
        let error = h.app.error.clone().unwrap();
        assert!(error.starts_with("The moving image must be a real"), "{error}");
        assert!(!h.running());
        assert_eq!(h.app.selected_run().unwrap().id, good);
        assert!(Arc::ptr_eq(&view, h.app.view.as_ref().unwrap()));
        // Other valid images start a new list; too small for the pyramid
        // gives the engine's message inline.
        h.app.moving.variable = "tiny".into();
        h.app.register_requested = true;
        wait(&mut h, &ws, &ctx);
        assert!(h.app.error.clone().unwrap().contains("too small"));
        assert!(h.app.runs.is_empty());
        // Back on the original images, register; a failing run keeps it.
        h.app.moving.variable = "mov".into();
        register(&mut h, &ws, &ctx);
        let kept = h.app.selected_run().unwrap().id;
        let view = h.app.view.clone().unwrap();
        h.app.settings.levels = 8;
        h.app.register_requested = true;
        wait(&mut h, &ws, &ctx);
        assert!(h.app.error.clone().unwrap().contains("too small"));
        assert_eq!(h.app.selected_run().unwrap().id, kept);
        assert!(Arc::ptr_eq(&view, h.app.view.as_ref().unwrap()));
        h.app.settings.levels = 3;

        // Export names: invalid, reserved, duplicate, an input of the run.
        for (k, bad) in [
            "bad name",
            "end",
            "mrexp_old",
            "matrust_export_error",
            "mrre_x",
            "matrust_registration",
            "movingRegistered",
            "mov",
            "fix",
        ]
        .into_iter()
        .enumerate()
        {
            let mut names = h.app.names.clone();
            names[k % 4] = bad.into();
            if bad == "movingRegistered" && k % 4 == 1 {
                names[0] = bad.into();
            }
            let saved = std::mem::replace(&mut h.app.names, names);
            assert!(h.app.script(&ws).is_err(), "{bad}");
            h.app.names = saved;
        }
        // Rollback: an existing name keeps its value when Export fails.
        h.app.names[0] = "old".into();
        let toolkits::Action::ExportScript(script) = h.export(&ws).unwrap() else {
            panic!()
        };
        source.execute("mov = 'not an image';").unwrap();
        assert!(apps::execute_export(&mut source, &script).is_err());
        assert_eq!(scalar(&source, "old"), 42.);
        assert!(source.execute(&script).is_err());
        assert_eq!(scalar(&source, "old"), 42.);
        assert!(!source.contains_array("movingRegistered"));
        // The changed input makes the app ask for a new run.
        ws.sync(&source.workspace());
        frame(&mut h, &ws, &ctx, vec![]);
        assert!(h.app.runs.is_empty());
        assert!(h.app.script(&ws).is_err());
    }

    #[test]
    fn registration_estimator_use_sample_data_registers_and_script_rebuilds_pair() {
        let ctx = egui::Context::default();
        // A workspace variable named like the sample's own names stays untouched.
        let (source, ws) =
            workspace("fixed = 7; moving = 9; sampleFixed = 1; t0 = rigidtform2d(-4, [0 0]);");
        let mut h = host(&source);
        h.app.settings.transform = TransformType::Affine;
        // An Initial transformation chosen earlier is a workspace variable:
        // the sample run and its script must not keep it.
        frame(&mut h, &ws, &ctx, vec![]);
        pick(&mut h, &ws, &ctx, "initial", "initial:t0");
        assert_eq!(h.app.settings.initial, "t0");
        click(&mut h, &ws, &ctx, "Use sample data");
        assert!(h.app.moving.sample && h.app.fixed.sample);
        assert_eq!(h.app.settings.transform, TransformType::Rigid);
        assert!(h.app.settings.initial.is_empty(), "Initial transformation cleared");
        assert!(!h.running(), "Use sample data only selects; Register runs");
        assert!(h.app.runs.is_empty());
        click(&mut h, &ws, &ctx, "Register");
        wait(&mut h, &ws, &ctx);
        assert!(h.app.error.is_none(), "{:?}", h.app.error);
        let run = h.app.selected_run().unwrap();
        assert_eq!(run.spec.moving, Source::Sample);
        assert_eq!(run.spec.fixed, Source::Sample);
        assert!(run.inputs.is_empty(), "no workspace input");
        assert!(run.spec.settings.initial.is_empty());
        assert!(run.spec.variables().is_empty(), "the run reads no variable");
        assert_eq!(run.class, "rigidtform2d");
        let a = run.matrix;
        let angle = a[1][0].atan2(a[0][0]).to_degrees();
        assert!((angle + 8.).abs() < 0.05, "rotation {angle}");
        // Inverse of rigidtform2d(8,[6 -4]): R(-8) and -R(-8)*[6 -4].
        let (s8, c8) = 8f64.to_radians().sin_cos();
        let inverse = [-(c8 * 6. + s8 * -4.), -(-s8 * 6. + c8 * -4.)];
        for k in 0..2 {
            assert!((a[k][2] - inverse[k]).abs() < 0.05, "translation {:?} vs {inverse:?}", [a[0][2], a[1][2]]);
        }
        assert!(run.quality > 0.999, "correlation {}", run.quality);
        assert!(h.app.view.is_some(), "the overlay shows the sample run");
        let matrix = run.matrix;
        // Generate script: rebuilds the pair itself; a fresh engine with no
        // variables gives the same transformation.
        let script = h.app.script(&ws).unwrap();
        assert!(script.contains("= matrust_registration();"), "{script}");
        assert!(script.contains("function [moving, fixed] = matrust_registration_sample()"));
        assert!(script.contains("rigidtform2d(8, [6 -4])"));
        let toolkits::Action::SaveAppScript(_, saved) = h.generate(&ws, 0).unwrap() else {
            panic!("script action")
        };
        let mut fresh = matrust::Engine::default();
        fresh.execute(&saved).unwrap();
        fresh.execute("re_A = tform.A;").unwrap();
        let matrust::Array::Numeric(m) = fresh.array("re_A").unwrap() else {
            panic!("A")
        };
        for (r, row) in matrix.iter().enumerate() {
            for (c, value) in row.iter().enumerate() {
                let d = m.data()[r + 3 * c] - value;
                assert!(d.abs() < 1e-9, "A({r},{c}) differs by {d}");
            }
        }
        let mut names: Vec<String> = fresh.workspace().into_keys().collect();
        names.sort();
        assert_eq!(
            names,
            ["Rregistered", "movingRegistered", "re_A", "registrationQuality", "tform"]
        );
        // Export writes only the four outputs; the same-named variables stay.
        let toolkits::Action::ExportScript(export) = h.export(&ws).unwrap() else {
            panic!("export action")
        };
        let mut engine = source.clone();
        apps::execute_export(&mut engine, &export).unwrap();
        assert_eq!(scalar(&engine, "fixed"), 7.);
        assert_eq!(scalar(&engine, "moving"), 9.);
        assert_eq!(scalar(&engine, "sampleFixed"), 1.);
        assert!(!engine.contains_array("sampleMoving"));
        // Without the Initial transformation variable in the workspace the
        // same sample run still registers.
        let (bare, bare_ws) = workspace("fixed = 7;");
        let mut h2 = host(&bare);
        h2.app.settings.initial = "t0".into();
        frame(&mut h2, &bare_ws, &ctx, vec![]);
        click(&mut h2, &bare_ws, &ctx, "Use sample data");
        click(&mut h2, &bare_ws, &ctx, "Register");
        wait(&mut h2, &bare_ws, &ctx);
        assert!(h2.app.error.is_none(), "{:?}", h2.app.error);
        let again = h2.app.selected_run().unwrap();
        assert_eq!(again.class, "rigidtform2d");
        assert!(again.quality > 0.999, "correlation {}", again.quality);

        // Workspace switches the picker back; the button is disabled while a
        // run is in flight (a click changes nothing).
        click(&mut h, &ws, &ctx, "moving:Workspace");
        assert!(!h.app.moving.sample && h.app.fixed.sample);
        let (source, ws) = workspace(
            "k = 1:1024; big = sin(k'/23) * cos(k/37); \
             bigm = imwarp(big, rigidtform2d(4,[10 -6]), 'OutputView', imref2d(size(big))); \
             clear k;",
        );
        let mut h = host(&source);
        select(&mut h, "bigm", "big");
        h.app.settings = re::Settings {
            transform: TransformType::Affine,
            levels: 1,
            iterations: 100_000,
            step: 1.,
            ..re::Settings::default()
        };
        frame(&mut h, &ws, &ctx, vec![]);
        click(&mut h, &ws, &ctx, "Register");
        assert!(h.running());
        click(&mut h, &ws, &ctx, "Use sample data");
        assert!(h.running(), "the long run should still be optimising");
        assert!(!h.app.moving.sample && !h.app.fixed.sample, "disabled during a run");
        assert_eq!(h.app.settings.transform, TransformType::Affine);
        click(&mut h, &ws, &ctx, "Cancel");
        wait(&mut h, &ws, &ctx);
        click(&mut h, &ws, &ctx, "Use sample data");
        assert!(h.app.moving.sample && h.app.fixed.sample, "enabled again after the run");
    }

    #[test]
    fn registration_estimator_cancel_long_run_keeps_previous_result() {
        let ctx = egui::Context::default();
        let (source, ws) = workspace(
            "k = 1:2048; big = sin(k'/23) * cos(k/37); \
             big = big + exp(-(k'-1000).^2/80000) * exp(-(k-800).^2/80000); \
             bigm = imwarp(big, rigidtform2d(4,[10 -6]), 'OutputView', imref2d(size(big))); \
             clear k;",
        );
        let mut h = host(&source);
        select(&mut h, "bigm", "big");
        h.app.settings.transform = TransformType::Translation;
        h.app.settings.iterations = 3;
        register(&mut h, &ws, &ctx);
        let first = h.app.selected_run().unwrap().id;
        let first_view = h.app.view.clone().unwrap();
        assert_eq!(first_view.step, 2, "2048 px overlays are sampled for display");
        // A long run: affine, full resolution, many iterations.
        h.app.settings = re::Settings {
            transform: TransformType::Affine,
            levels: 1,
            iterations: 100_000,
            step: 1.,
            ..re::Settings::default()
        };
        click(&mut h, &ws, &ctx, "Register");
        assert!(h.running());
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(400) {
            frame(&mut h, &ws, &ctx, vec![]);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(h.running(), "the long run should still be optimising");
        assert!(h.app.running);
        let (out, _) = frame(&mut h, &ws, &ctx, vec![]);
        assert!(!out.shapes.is_empty(), "running state renders");
        let cancelled = Instant::now();
        click(&mut h, &ws, &ctx, "Cancel");
        assert_eq!(h.message, "Registration cancelled; earlier results kept");
        // The optimiser polls the cancel flag every iteration: its thread ends soon.
        wait(&mut h, &ws, &ctx);
        let stopped = cancelled.elapsed();
        eprintln!("registration_estimator cancel stopped the run after {stopped:?}");
        assert!(stopped < Duration::from_secs(5), "{stopped:?}");
        assert_eq!(h.app.runs.len(), 1);
        assert_eq!(h.app.selected_run().unwrap().id, first);
        assert!(Arc::ptr_eq(&first_view, h.app.view.as_ref().unwrap()));
        assert!(h.app.error.is_none());
    }

    #[test]
    fn registration_estimator_real_interactions_pick_register_cancel_select_export() {
        let ctx = egui::Context::default();
        let (source, ws) = workspace(IMAGES);
        let mut h = host(&source);
        frame(&mut h, &ws, &ctx, vec![]);
        pick(&mut h, &ws, &ctx, "moving:image", "moving:mov");
        assert_eq!(h.app.moving.variable, "mov");
        pick(&mut h, &ws, &ctx, "fixed:image", "fixed:fix");
        assert_eq!(h.app.fixed.variable, "fix");
        wait(&mut h, &ws, &ctx);
        let before = h.app.view.clone().expect("the before overlay");
        assert!(h.app.selected_run().is_none());
        click(&mut h, &ws, &ctx, "Register");
        assert!(h.running() || h.app.selected_run().is_some());
        wait(&mut h, &ws, &ctx);
        assert_eq!(h.app.runs.len(), 1);
        let after = h.app.view.clone().unwrap();
        assert!(!Arc::ptr_eq(&before, &after) && h.app.after);
        // Overlay modes, before/after and zoom.
        for mode in Overlay::ALL {
            click(&mut h, &ws, &ctx, mode.label());
            wait(&mut h, &ws, &ctx);
            assert_eq!(h.app.overlay, mode);
            let view = h.app.view.clone().unwrap();
            assert!(view.rows == 80 && view.cols == 96, "{mode:?}");
        }
        let diff = h.app.view.clone().unwrap();
        click(&mut h, &ws, &ctx, "Before");
        wait(&mut h, &ws, &ctx);
        assert!(!h.app.after);
        assert!(!Arc::ptr_eq(&diff, h.app.view.as_ref().unwrap()));
        click(&mut h, &ws, &ctx, "After");
        wait(&mut h, &ws, &ctx);
        assert!(h.app.after);
        click(&mut h, &ws, &ctx, "100%");
        assert!(!h.app.fit);
        click(&mut h, &ws, &ctx, "Fit");
        assert!(h.app.fit);
        // A second run with another transformation, through the combo box.
        click(&mut h, &ws, &ctx, "False color");
        pick(&mut h, &ws, &ctx, "transform", "transform:Translation");
        assert_eq!(h.app.settings.transform, TransformType::Translation);
        click(&mut h, &ws, &ctx, "Register");
        wait(&mut h, &ws, &ctx);
        assert_eq!(h.app.runs.len(), 2);
        assert_eq!(h.app.selected, Some(2));
        click(&mut h, &ws, &ctx, "Run 1");
        wait(&mut h, &ws, &ctx);
        assert_eq!(h.app.selected, Some(1));
        assert_eq!(
            h.app.selected_run().unwrap().spec.settings.transform,
            TransformType::Rigid
        );
        // Cancel through its button while a run is in flight.
        h.app.settings.iterations = 100_000;
        h.app.settings.levels = 1;
        h.app.settings.transform = TransformType::Affine;
        h.app.settings.step = 1e-3;
        click(&mut h, &ws, &ctx, "Register");
        if h.running() {
            click(&mut h, &ws, &ctx, "Cancel");
            assert_eq!(h.message, "Registration cancelled; earlier results kept");
        }
        wait(&mut h, &ws, &ctx);
        let runs = h.app.runs.len();
        assert!(runs == 2 || runs == 3);
        click(&mut h, &ws, &ctx, "Run 1");
        wait(&mut h, &ws, &ctx);
        assert!(matches!(
            click(&mut h, &ws, &ctx, "Export"),
            Some(toolkits::Action::ExportScript(_))
        ));
        assert!(matches!(
            click(&mut h, &ws, &ctx, "Generate script"),
            Some(toolkits::Action::SaveAppScript(
                AppKind::RegistrationEstimator,
                _
            ))
        ));
        export_equals_saved(&mut h, &ws, &source);
    }

    #[test]
    fn registration_estimator_hook_cli_launch_and_render_states() {
        let mut e = matrust::Engine::default();
        assert!(
            e.execute("registrationEstimator;")
                .unwrap_err()
                .contains("registrationEstimator requires the desktop")
        );
        let slot = Arc::new(Mutex::new(None));
        let dest = slot.clone();
        let _hook = regest_app_hooks::present(move |launch| {
            *dest.lock().unwrap() = Some(launch);
            Ok(())
        });
        e.execute("registrationEstimator;").unwrap();
        assert!(matches!(slot.lock().unwrap().take(), Some(Launch::Empty)));
        e.execute("m = rand(20,30,3); f = uint8(magic(20)); registrationEstimator(m, f);")
            .unwrap();
        let Some(Launch::Images { moving, fixed }) = slot.lock().unwrap().take() else {
            panic!("images launch")
        };
        assert_eq!((moving.0.as_str(), fixed.0.as_str()), ("m", "f"));
        for code in [
            "registrationEstimator(m)",
            "registrationEstimator(m, f, f)",
            "x = registrationEstimator(m, f)",
            "registrationEstimator(m, 'f')",
            "registrationEstimator(m > 0.5, f)",
            "registrationEstimator(m, ones(4,4,4,2))",
            "registrationEstimator(m, ones(20))",
            "registrationEstimator(int8(f), f)",
        ] {
            assert!(e.execute(code).is_err(), "{code}");
        }
        assert!(slot.lock().unwrap().is_none());

        // The desktop worker opens the registered app.
        for call in [
            "registrationEstimator",
            "mv = rand(32,40); fx = rand(32,40); registrationEstimator(mv, fx)",
        ] {
            let ctx = egui::Context::default();
            let mut app = Workspace::with_context(ctx.clone(), true);
            app.run(call.into());
            let deadline = Instant::now() + Duration::from_secs(30);
            while app.busy {
                app.receive();
                assert!(Instant::now() < deadline, "{call}: {}", app.status);
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(
                app.toolkits.apps.is_open(AppKind::RegistrationEstimator),
                "{call}: {}",
                app.status
            );
            let h = &app.toolkits.apps.registration;
            if call.contains("(mv, fx)") {
                assert_eq!(h.app.moving.variable, "mv");
                assert_eq!(h.app.fixed.variable, "fx");
            } else {
                assert!(h.app.moving.variable.is_empty());
            }
        }
        // Objects in the workspace reach the app's catalogue.
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.run("R = imref2d([20 30]); t = rigidtform2d(5, [1 2]); x = 1;".into());
        let deadline = Instant::now() + Duration::from_secs(30);
        while app.busy {
            app.receive();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        let catalogue = &app.toolkits.apps.registration.app.catalogue;
        let mut names: Vec<_> = catalogue.iter().map(|g| (g.name.as_str(), g.class)).collect();
        names.sort();
        assert_eq!(names, [("R", "imref2d"), ("t", "rigidtform2d")]);

        // Render: empty, running, result, error, narrow.
        let ctx = egui::Context::default();
        let (source, ws) = workspace(IMAGES);
        let mut h = host(&source);
        assert!(!frame(&mut h, &ws, &ctx, vec![]).0.shapes.is_empty());
        select(&mut h, "mov", "fix");
        h.app.register_requested = true;
        assert!(!frame(&mut h, &ws, &ctx, vec![]).0.shapes.is_empty());
        wait(&mut h, &ws, &ctx);
        assert!(h.app.selected_run().is_some());
        assert!(!frame(&mut h, &ws, &ctx, vec![]).0.shapes.is_empty());
        assert_eq!(h.status().1, toolkits::AppStatus::Success);
        h.app.error = Some("imregtform: synthetic failure".into());
        assert!(!frame(&mut h, &ws, &ctx, vec![]).0.shapes.is_empty());
        assert_eq!(h.status().1, toolkits::AppStatus::Error);
        h.app.error = None;
        let narrow = egui::vec2(900., 800.);
        assert!(!frame_at(&mut h, &ws, &ctx, narrow, vec![]).0.shapes.is_empty());
        let window = egui::Rect::from_min_size(egui::Pos2::ZERO, narrow);
        for name in ["Export", "Generate script"] {
            let r = h.app.controls.0[name];
            assert!(r.left() >= 280. && r.right() <= 900., "{name}: {r:?}");
        }
        for attempt in 0..16 {
            if ["Export", "Generate script"]
                .iter()
                .all(|name| window.contains_rect(h.app.controls.0[*name]))
            {
                break;
            }
            let _ = attempt;
            frame_at(
                &mut h,
                &ws,
                &ctx,
                narrow,
                vec![
                    egui::Event::PointerMoved(egui::pos2(600., 500.)),
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
                window.contains_rect(h.app.controls.0[name]),
                "narrow {name}: {:?}",
                h.app.controls.0[name]
            );
        }
        for name in ["Register", "Cancel"] {
            let r = h.app.controls.0[name];
            assert!(r.left() >= 0. && r.right() <= 280., "{name} clipped: {r:?}");
        }
        assert!(AppKind::ALL.contains(&AppKind::RegistrationEstimator));
        assert!(AppKind::RegistrationEstimator.is_built());
        assert_eq!(
            AppKind::RegistrationEstimator.tab(),
            matrust::session::ToolTab::RegistrationEstimator
        );
        assert_eq!(
            AppKind::from_tab(matrust::session::ToolTab::RegistrationEstimator),
            Some(AppKind::RegistrationEstimator)
        );
        assert_eq!(AppKind::RegistrationEstimator.label(), "Registration Estimator");
    }
}
