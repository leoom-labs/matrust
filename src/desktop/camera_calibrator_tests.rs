// Camera Calibrator: sample data, folder/file launch, Calibrate, Export,
// generated script reproducibility, error states, and real egui interaction.
mod camera_calibrator_tests {
    use super::*;
    use apps::{
        App, AppKind, Host, Outputs, Snapshot,
        camera_calibrator::{self as cc, CameraCalibrator, ImageStatus},
    };
    use matrust::engine::camcal_app_hooks::{self, CalibrationValues, Launch, TransferScope};
    use std::sync::{Arc, Mutex};

    // The capacity checks deliberately fill the process-wide transfer queue.
    static EXPORT_TICKET_TESTS: Mutex<()> = Mutex::new(());

    fn host() -> Host<CameraCalibrator> {
        Host::new(CameraCalibrator::default())
    }

    fn frame(
        h: &mut Host<CameraCalibrator>,
        ws: &Snapshot,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Option<toolkits::Action>) {
        let mut action = None;
        let output = crate::headless::run(ctx, input(events), |ui| {
            cc::show(h, ui, ws, false, &mut action)
        });
        (output, action)
    }

    fn input(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1600., 1000.),
            )),
            events,
            ..Default::default()
        }
    }

    fn inputs_frame(h: &mut Host<CameraCalibrator>, ws: &Snapshot, ctx: &egui::Context) {
        cc::refresh(h, ctx, ws);
        crate::headless::run(ctx, input(vec![]), |ui| h.app.inputs_ui(ui, ws));
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
        h: &mut Host<CameraCalibrator>,
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
        action.or(frame(h, ws, ctx, vec![]).1)
    }

    fn wait(h: &mut Host<CameraCalibrator>, ws: &Snapshot, ctx: &egui::Context) {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            frame(h, ws, ctx, vec![]);
            if !h.running() && !h.app.view_running() {
                break;
            }
            assert!(Instant::now() < deadline, "Camera Calibrator timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
        frame(h, ws, ctx, vec![]);
    }

    fn logical(e: &mut matrust::Engine, code: &str) -> bool {
        e.execute(&format!("cc_test_flag = logical({code});"))
            .unwrap();
        let matrust::Array::Logical(flag) = e.array("cc_test_flag").unwrap() else {
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

    fn extrinsics_action(
        h: &mut Host<CameraCalibrator>,
        ws: &Snapshot,
        ctx: &egui::Context,
        button: &str,
    ) -> String {
        let mut action = click(h, ws, ctx, button);
        let deadline = Instant::now() + Duration::from_secs(10);
        while action.is_none() {
            assert!(h.app.error.is_none(), "{:?}", h.app.error);
            assert!(Instant::now() < deadline, "extrinsics request timed out");
            std::thread::sleep(Duration::from_millis(5));
            action = frame(h, ws, ctx, vec![]).1;
        }
        let Some(toolkits::Action::Script(code)) = action else {
            panic!("expected a figure publication script");
        };
        code
    }

    fn assert_undistorted(h: &Host<CameraCalibrator>, e: &mut matrust::Engine) {
        assert!(h.app.view_error.is_none(), "{:?}", h.app.view_error);
        let path = h.app.images[h.app.selected_image.unwrap()]
            .path
            .replace('\'', "''");
        e.execute(&format!(
            "cc_test_im = imread('{path}'); cc_test_original = double(cc_test_im); \
             cc_test_und = double(undistortImage(cc_test_im, cameraParams));"
        ))
        .unwrap();
        let Some(matrust::Array::Numeric(expected)) = e.array("cc_test_und") else {
            panic!("undistorted image");
        };
        let Some(matrust::Array::Numeric(original)) = e.array("cc_test_original") else {
            panic!("original image");
        };
        let shown = &h.app.view.as_ref().unwrap().axes.extras.images[0].data;
        assert_eq!(shown.len(), expected.data().len());
        assert_ne!(shown.as_ref(), original.data());
        assert!(
            shown
                .iter()
                .zip(expected.data())
                .all(|(a, b)| (a - b).abs() < 1e-12)
        );
    }

    fn visible_text(output: &egui::FullOutput, label: &str) -> bool {
        output.shapes.iter().any(|clipped| {
            matches!(&clipped.shape, egui::Shape::Text(text) if text.galley.text() == label)
        })
    }

    fn fixture_fit() -> (matrust::Engine, Vec<String>) {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/calib/images");
        let files: Vec<_> = (1..=4)
            .map(|k| {
                dir.join(format!("board_{k:02}.png"))
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        let quoted: Vec<_> = files
            .iter()
            .map(|path| format!("'{}'", path.replace('\'', "''")))
            .collect();
        let mut e = matrust::Engine::with_limits(matrust::engine::Limits::local());
        e.execute(&format!(
            "files = {{{}}}; info = imfinfo(files{{1}}); \
             [points, cc_board_size] = detectCheckerboardPoints(files); \
             world = generateCheckerboardPoints(cc_board_size, 25); \
             [cameraParams, ~, estimationErrors] = estimateCameraParameters(points, world, \
             'ImageSize', [info.Height, info.Width]);",
            quoted.join("; ")
        ))
        .unwrap();
        (e, files)
    }

    #[test]
    fn camera_calibrator_export_tickets_stay_until_consumed() {
        let (mut e, _) = fixture_fit();
        let values = CalibrationValues::capture(&e, ["cameraParams", "estimationErrors"]).unwrap();
        let _ticket_guard = EXPORT_TICKET_TESTS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _hook = camcal_app_hooks::present(|_| panic!("export must not open the app"));
        let scope = TransferScope::default();
        let stager = scope.stager();
        let names = ["savedParams", "savedErrors"];
        let tickets: Vec<_> = (0..5)
            .map(|_| stager.stage_export(&values, names).unwrap())
            .collect();
        for code in &tickets {
            e.execute(code).unwrap();
            assert!(logical(&mut e, "isa(savedParams, 'cameraParameters')"));
            assert!(logical(
                &mut e,
                "isa(savedErrors, 'cameraCalibrationErrors')"
            ));
        }
        let used = e.execute(&tickets[0]).unwrap_err();
        assert!(
            used.contains("export ticket was not found or was already used"),
            "{used}"
        );
        // Bounding pending transfers refuses a new stage without evicting accepted ones.
        let mut overflow = Vec::new();
        let mut refused = None;
        for _ in 0..32 {
            match stager.stage_export(&values, names) {
                Ok(code) => overflow.push(code),
                Err(err) => {
                    refused = Some(err);
                    break;
                }
            }
        }
        let refused = refused.expect("pending export limit should refuse new stage");
        assert!(refused.contains("too many pending exports"), "{refused}");
        for code in &overflow {
            e.execute(code).unwrap();
        }
    }

    #[test]
    fn camera_calibrator_detected_but_unused_row() {
        let (mut e, files) = fixture_fit();
        e.execute(
            "cc_mean_err = cameraParams.MeanReprojectionError;\n\
             cc_focal = cameraParams.FocalLength;\n\
             cc_pp = cameraParams.PrincipalPoint;\n\
             cc_radial = cameraParams.RadialDistortion;\n\
             cc_tang = cameraParams.TangentialDistortion;\n\
             cc_skew = cameraParams.Skew;\n\
             cc_imsize = cameraParams.ImageSize;\n\
             errors = cameraParams.ReprojectionErrors; cc_errors = errors(:,:,1:2);\n\
             cc_images_used = logical([1 0 1 1]); cc_estimator_used = logical([1 0 1]);",
        )
        .unwrap();
        let out: Outputs = [
            "cc_mean_err",
            "cc_focal",
            "cc_pp",
            "cc_radial",
            "cc_tang",
            "cc_skew",
            "cc_imsize",
            "cc_board_size",
            "cc_errors",
            "cc_images_used",
            "cc_estimator_used",
        ]
        .into_iter()
        .map(|name| (name.into(), e.array(name).unwrap()))
        .collect();
        let values = CalibrationValues::capture(&e, ["cameraParams", "estimationErrors"]).unwrap();
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = host();
        h.app.set_files(files);
        wait(&mut h, &ws, &ctx);
        let result = cc::make_result(h.app.spec(), &out, values).unwrap();
        assert_eq!(result.detected_mask, [true, false, true, true]);
        assert_eq!(result.used_mask, [true, false, false, true]);
        assert_eq!(result.images_used, 2);
        h.app.apply_result(result);
        let (output, _) = frame(&mut h, &ws, &ctx, vec![]);
        assert!(visible_text(&output, "board_03.png  —  unused"));
        assert!(visible_text(&output, "board_02.png  —  rejected"));
    }

    #[test]
    fn camera_calibrator_sample_data_calibrates_exports_and_reproduces() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = host();
        frame(&mut h, &ws, &ctx, vec![]);

        let sample_start = Instant::now();
        click(&mut h, &ws, &ctx, "Use sample data");
        let (loading, _) = frame(&mut h, &ws, &ctx, vec![]);
        assert!(visible_text(&loading, "Loading sample images…"));
        assert!(!visible_text(
            &loading,
            "No images loaded. Add images, a folder, or Use sample data."
        ));
        for control in [
            "Use sample data",
            "Add images",
            "Clear",
            "Calibrate",
            "Skew",
            "Fit",
        ] {
            assert!(!h.app.controls.1[control], "{control}");
        }
        let deadline = Instant::now() + Duration::from_secs(120);
        while h.app.images.is_empty() {
            frame(&mut h, &ws, &ctx, vec![]);
            assert!(Instant::now() < deadline, "sample import timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
        let preview = h
            .app
            .view
            .as_ref()
            .expect("preview must be ready with the sample rows");
        assert_eq!(preview.axes.extras.color_limits(), [0.0, 255.0]);
        wait(&mut h, &ws, &ctx);
        let sample_elapsed = sample_start.elapsed();
        assert_eq!(h.app.images.len(), 10);
        assert!(h.app.view.is_some());

        let mut sample_engine = matrust::Engine::with_limits(matrust::engine::Limits::local());
        let sample_dir = tempfile::tempdir().unwrap();
        let generation_start = Instant::now();
        sample_engine
            .execute(&format!(
                "cc_folder = matrust_camcal_sample('{}');\n{}",
                sample_dir.path().to_string_lossy().replace('\'', "''"),
                cc::SAMPLE_CODE
            ))
            .unwrap();
        eprintln!("sample generation only: {:?}", generation_start.elapsed());
        let t0 = Instant::now();
        click(&mut h, &ws, &ctx, "Calibrate");
        // Calibrate button is disabled while running.
        frame(&mut h, &ws, &ctx, vec![]);
        assert!(h.app.running, "calibration should still be running");
        assert!(!h.app.controls.1["Calibrate"]);
        if h.app.running {
            let rect = h.app.controls.0["Calibrate"];
            let pos = rect.center();
            frame(
                &mut h,
                &ws,
                &ctx,
                vec![egui::Event::PointerMoved(pos), pointer(pos, true)],
            );
            frame(&mut h, &ws, &ctx, vec![pointer(pos, false)]);
            assert!(!h.app.calibrate_requested);
        }
        wait(&mut h, &ws, &ctx);
        let elapsed = t0.elapsed();
        eprintln!(
            "sample generation and calibrate total (summed): {:?}",
            sample_elapsed + elapsed
        );
        assert!(h.app.error.is_none(), "{:?}", h.app.error);

        let res = h.app.result.as_ref().expect("calibration result");
        eprintln!(
            "camera_calibrator sample: elapsed={elapsed:?}, mean_err={:.6}, fx={:.3}, fy={:.3}, cx={:.3}, cy={:.3}, used={}/{}",
            res.mean_error,
            res.focal_length[0],
            res.focal_length[1],
            res.principal_point[0],
            res.principal_point[1],
            res.images_used,
            res.total_images,
        );
        assert_eq!(res.images_used, 10);
        assert_eq!(res.total_images, 10);
        assert!(
            h.app
                .images
                .iter()
                .all(|e| e.status == ImageStatus::Detected)
        );
        assert!((res.focal_length[0] - 800.0).abs() / 800.0 < 0.005);
        assert!((res.focal_length[1] - 790.0).abs() / 790.0 < 0.005);
        assert!((res.principal_point[0] - 322.0).abs() < 2.0);
        assert!((res.principal_point[1] - 238.0).abs() < 2.0);
        assert!(res.mean_error < 0.3, "mean_error={}", res.mean_error);
        let mean_error = res.mean_error;
        for name in ["end", "cc_world_points", "cc_files", "mrexp_old"] {
            h.app.names[0] = name.into();
            assert!(h.app.script(&ws).is_err(), "{name} must be rejected");
        }
        h.app.names[0] = "cameraParams".into();

        // Compare the native chart with the fitted engine object's errors.
        let _ticket_guard = EXPORT_TICKET_TESTS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let bars = h.app.result.as_ref().unwrap().per_view_errors.clone();
        assert_eq!(bars.len(), h.app.result.as_ref().unwrap().images_used);
        let Some(toolkits::Action::Script(code)) = click(&mut h, &ws, &ctx, "Export") else {
            panic!("export")
        };
        let mut reference = matrust::Engine::with_limits(matrust::engine::Limits::local());
        let _reference_hook = camcal_app_hooks::present(|_| Ok(()));
        reference.execute(&code).unwrap();

        // A closed app releases both fitted values and figures waiting for publication.
        let values =
            CalibrationValues::capture(&reference, ["cameraParams", "estimationErrors"]).unwrap();
        let mut closing = host();
        let stager = closing.app.transfers.stager();
        let names = ["savedParams", "savedErrors"];
        let abandoned: Vec<_> = (0..15)
            .map(|_| stager.stage_export(&values, names).unwrap())
            .collect();
        let (figure_code, reply) = stager.stage_figure(Default::default()).unwrap();
        assert!(click(&mut h, &ws, &ctx, "Export").is_none());
        assert!(h.message.is_empty());
        let refusal = h.app.error.as_deref().unwrap();
        assert!(refusal.contains("too many pending exports"), "{refusal}");
        assert_eq!(h.status(), (refusal.to_owned(), toolkits::AppStatus::Error));
        cc::close(&mut closing);
        assert!(matches!(
            reply.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Disconnected)
        ));
        for code in abandoned.iter().chain(std::iter::once(&figure_code)) {
            let error = reference.execute(code).unwrap_err();
            assert!(error.contains("export ticket was not found"), "{error}");
        }
        let error = stager.stage_export(&values, names).unwrap_err();
        assert!(error.contains("cancelled on close"), "{error}");
        let Some(toolkits::Action::Script(retry)) = click(&mut h, &ws, &ctx, "Export") else {
            panic!("closing the app should free capacity to export");
        };
        assert_eq!(
            h.message,
            "Exported cameraParams, estimationErrors to the workspace"
        );
        assert!(h.app.error.is_none(), "{:?}", h.app.error);
        assert_eq!(h.status().1, toolkits::AppStatus::Success);
        reference.execute(&retry).unwrap();

        reference
            .execute("cc_test_errors = cameraParams.ReprojectionErrors;")
            .unwrap();
        let matrust::Array::Numeric(errors) = reference.array("cc_test_errors").unwrap() else {
            panic!("errors")
        };
        let n = errors.dims()[0];
        assert_eq!(bars.len(), errors.data().len() / (2 * n));
        for (bar, page) in bars.iter().zip(errors.data().chunks_exact(2 * n)) {
            let mean = (0..n).map(|i| page[i].hypot(page[n + i])).sum::<f64>() / n as f64;
            assert!((bar - mean).abs() < 1e-12);
        }
        let selected = h
            .app
            .result
            .as_ref()
            .unwrap()
            .used_mask
            .iter()
            .enumerate()
            .filter_map(|(i, used)| used.then_some(i))
            .nth(2)
            .unwrap();
        assert_ne!(h.app.selected_image, Some(selected));
        click(&mut h, &ws, &ctx, "bar:2");
        assert_eq!(h.app.selected_image, Some(selected));

        let before: Vec<_> = h
            .app
            .images
            .iter()
            .map(|entry| entry.path.clone())
            .collect();
        h.app
            .append_files(vec![before[selected].clone(), "/tmp/ignored.gif".into()]);
        wait(&mut h, &ws, &ctx);
        assert_eq!(h.app.selected_image, Some(selected));
        assert!(
            h.app
                .images
                .iter()
                .all(|entry| entry.status == ImageStatus::Detected)
        );
        assert!(
            h.app
                .images
                .iter()
                .map(|entry| &entry.path)
                .eq(before.iter())
        );
        assert_eq!(h.app.result.as_ref().unwrap().per_view_errors, bars);
        assert!(!h.app.stale);

        let variables = reference.workspace_count();
        for (index, button) in ["Show extrinsics", "PatternCentric"]
            .into_iter()
            .enumerate()
        {
            reference
                .execute(&extrinsics_action(&mut h, &ws, &ctx, button))
                .unwrap();
            frame(&mut h, &ws, &ctx, vec![]);
            assert!(h.app.error.is_none(), "{:?}", h.app.error);
            assert_eq!(reference.workspace_count(), variables);
            assert_eq!(reference.figure_ids().len(), index + 1);
            let (_, figure) = reference.figures().last().unwrap();
            assert_eq!(figure.title, "Extrinsic Parameters Visualization");
        }

        // A desktop that already has 16 figures cannot publish another one.
        reference
            .execute("for k = 3:16, figure(k); end; clear k;")
            .unwrap();
        let code = extrinsics_action(&mut h, &ws, &ctx, "Show extrinsics");
        let error = reference.execute(&code).unwrap_err();
        assert!(error.contains("At most 16 figures"), "{error}");
        frame(&mut h, &ws, &ctx, vec![]);
        assert!(
            h.app
                .error
                .as_deref()
                .unwrap()
                .contains("At most 16 figures")
        );
        assert_eq!(reference.figure_ids().len(), 16);

        cc::submit_extrinsics(&mut h.app, &ctx, "InvalidView");
        let deadline = Instant::now() + Duration::from_secs(10);
        while h.app.error.is_none() {
            assert!(frame(&mut h, &ws, &ctx, vec![]).1.is_none());
            assert!(Instant::now() < deadline, "extrinsics error timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            h.app
                .error
                .as_deref()
                .unwrap()
                .contains("unknown view 'InvalidView'")
        );
        let error = h.app.error.as_deref().unwrap().to_owned();
        let (output, _) = frame(&mut h, &ws, &ctx, vec![]);
        assert_eq!(
            output
                .shapes
                .iter()
                .filter(|clipped| {
                    matches!(&clipped.shape, egui::Shape::Text(text) if text.galley.text() == error)
                })
                .count(),
            1,
        );

        click(&mut h, &ws, &ctx, "image:0");
        assert_eq!(h.app.selected_image, Some(0));
        click(&mut h, &ws, &ctx, "Show Undistorted");
        wait(&mut h, &ws, &ctx);
        assert_undistorted(&h, &mut reference);
        click(&mut h, &ws, &ctx, "Show Undistorted");
        wait(&mut h, &ws, &ctx);

        click(&mut h, &ws, &ctx, "image:3");
        wait(&mut h, &ws, &ctx);
        assert_eq!(h.app.selected_image, Some(3));
        assert_eq!(
            h.app.view.as_ref().unwrap().axes.extras.color_limits(),
            [0.0, 255.0]
        );

        // Export publishes the successful run even after its inputs change.
        std::fs::remove_file(&h.app.images[0].path).unwrap();
        let action = click(&mut h, &ws, &ctx, "Export");
        let Some(toolkits::Action::Script(export_code)) = action else {
            panic!("expected workspace export command");
        };
        assert_eq!(
            h.message,
            "Exported cameraParams, estimationErrors to the workspace"
        );
        let mut exported = matrust::Engine::with_limits(matrust::engine::Limits::local());
        let _hook = camcal_app_hooks::present(|_| panic!("export must not open the app"));
        exported.execute("cc_files = 1; cc_folder = 2; mrexp_complete = 3; cameraCalibrator = 4; builtin = 5;").unwrap();
        exported.execute(&export_code).unwrap();
        assert_eq!(exported.workspace_count(), 7);
        assert_eq!(scalar(&exported, "cc_files"), 1.0);
        assert_eq!(scalar(&exported, "cc_folder"), 2.0);
        assert_eq!(scalar(&exported, "mrexp_complete"), 3.0);
        assert_eq!(scalar(&exported, "cameraCalibrator"), 4.0);
        assert_eq!(scalar(&exported, "builtin"), 5.0);
        assert!(logical(
            &mut exported,
            "isa(cameraParams, 'cameraParameters')"
        ));
        assert!(logical(
            &mut exported,
            "isa(estimationErrors, 'cameraCalibrationErrors')"
        ));
        exported
            .execute("cc_err_ws = cameraParams.MeanReprojectionError;")
            .unwrap();
        assert_eq!(scalar(&exported, "cc_err_ws"), mean_error);

        // Script uses the run's settings and still works after the app cleans up its images.
        h.app.settings.estimate_skew = true;
        let Some(toolkits::Action::SaveAppScript(AppKind::CameraCalibrator, script)) =
            click(&mut h, &ws, &ctx, "Generate script")
        else {
            panic!("expected camera calibration script")
        };
        let old_dir = PathBuf::from(&h.app.images[0].path)
            .parent()
            .unwrap()
            .to_path_buf();
        assert!(!script.contains(old_dir.to_str().unwrap()));
        cc::close(&mut h);
        assert!(!old_dir.exists());
        h.app.choose(Launch::Empty);
        wait(&mut h, &ws, &ctx);
        assert!(h.app.images.is_empty());
        assert!(h.app.selected_image.is_none());
        assert!(h.app.view.is_none());
        assert!(h.app.result.is_none());
        assert!(!h.app.controls.1["Export"]);
        assert!(!h.app.controls.1["Generate script"]);

        let mut fresh = matrust::Engine::with_limits(matrust::engine::Limits::local());
        fresh.execute(&script).unwrap();
        fresh
            .execute("cc_err_script = cameraParams.MeanReprojectionError;")
            .unwrap();
        let err_script = scalar(&fresh, "cc_err_script");
        assert!(
            (err_script - mean_error).abs() < 1e-9,
            "script={err_script} vs app={mean_error}",
        );
        assert!(!logical(&mut fresh, "cameraParams.EstimateSkew"));
        click(&mut h, &ws, &ctx, "Use sample data");
        wait(&mut h, &ws, &ctx);
        let new_dir = PathBuf::from(&h.app.images[0].path)
            .parent()
            .unwrap()
            .to_path_buf();
        assert_ne!(old_dir, new_dir);
        assert!(!old_dir.exists());
        assert!(h.app.view.is_some());
        click(&mut h, &ws, &ctx, "Calibrate");
        wait(&mut h, &ws, &ctx);
        click(&mut h, &ws, &ctx, "Show Undistorted");
        wait(&mut h, &ws, &ctx);
        assert!(h.app.show_undistorted);
        click(&mut h, &ws, &ctx, "Remove");
        wait(&mut h, &ws, &ctx);
        assert_eq!(h.app.images.len(), 9);
        assert!(h.app.stale && h.app.result.is_none());
        assert!(h.app.show_undistorted && h.app.view.is_some());
        assert!(!h.app.controls.1["Export"] && !h.app.controls.1["Generate script"]);
        click(&mut h, &ws, &ctx, "Calibrate");
        wait(&mut h, &ws, &ctx);
        assert_eq!(h.app.result.as_ref().unwrap().images_used, 9);
        let Some(toolkits::Action::Script(code)) = click(&mut h, &ws, &ctx, "Export") else {
            panic!("export recalibration");
        };
        reference.execute(&code).unwrap();
        assert_undistorted(&h, &mut reference);
        click(&mut h, &ws, &ctx, "Clear");
        wait(&mut h, &ws, &ctx);
        assert!(!new_dir.exists());
    }

    #[test]
    fn camera_calibrator_fixtures_folder_and_error_states() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/calib/images");

        // Launch via cameraCalibrator(folder, 25) hook.
        let received = Arc::new(Mutex::new(None));
        let slot = received.clone();
        let _hook = camcal_app_hooks::present(move |launch| {
            *slot.lock().unwrap() = Some(launch);
            Ok(())
        });
        let mut eng = matrust::Engine::with_limits(matrust::engine::Limits::local());
        eng.execute(&format!(
            "cameraCalibrator('{}', 25);",
            dir.to_string_lossy().replace('\'', "''")
        ))
        .unwrap();
        let launch = received.lock().unwrap().take().unwrap();

        let mut h = host();
        h.app.choose(launch);
        wait(&mut h, &ws, &ctx);
        assert!(h.app.images.len() >= 10);
        frame(&mut h, &ws, &ctx, vec![]);
        click(&mut h, &ws, &ctx, "Calibrate");
        wait(&mut h, &ws, &ctx);
        assert!(h.app.error.is_none(), "{:?}", h.app.error);
        let res = h.app.result.as_ref().expect("fixture calibration");
        assert!(res.images_used >= 10);
        assert!(
            res.mean_error < 0.3,
            "fixture mean_error={}",
            res.mean_error
        );

        h.app.settings.square_size = -1.0;
        h.app.calibrate_requested = true;
        wait(&mut h, &ws, &ctx);
        assert!(h.app.result.is_none());
        assert!(
            h.app
                .images
                .iter()
                .all(|entry| entry.status == ImageStatus::Pending)
        );
        assert!(!h.app.controls.1["Export"]);
        assert!(!h.app.controls.1["Generate script"]);
        h.app.settings.square_size = 25.0;

        let empty_dir = tempfile::tempdir().unwrap();
        h.app
            .set_folder(empty_dir.path().to_string_lossy().into_owned());
        wait(&mut h, &ws, &ctx);
        assert!(h.app.images.is_empty());
        assert_eq!(
            h.app.error.as_deref(),
            Some("No matching images found in folder")
        );
        click(&mut h, &ws, &ctx, "Clear");
        wait(&mut h, &ws, &ctx);
        assert!(h.app.error.is_none());

        // Folder with one image -> error line, never a panic.
        let one_dir = tempfile::tempdir().unwrap();
        std::fs::copy(
            dir.join("board_01.png"),
            one_dir.path().join("board_01.png"),
        )
        .unwrap();
        h.app
            .set_folder(one_dir.path().to_string_lossy().into_owned());
        wait(&mut h, &ws, &ctx);
        assert_eq!(h.app.images.len(), 1);
        frame(&mut h, &ws, &ctx, vec![]);
        assert!(!h.app.controls.1["Calibrate"]);
        assert!(
            h.app
                .error
                .as_deref()
                .unwrap()
                .contains("need at least 2 images")
        );

        // 50+ images list scrolls without panic.
        let many: Vec<String> = (0..55)
            .map(|k| format!("/tmp/nonexistent_{k:02}.png"))
            .collect();
        h.app.set_files(many);
        wait(&mut h, &ws, &ctx);
        assert_eq!(h.app.images.len(), 55);
        frame(&mut h, &ws, &ctx, vec![]);
    }

    #[test]
    fn camera_calibrator_hook_cli_forms_and_app_kind() {
        let mut e = matrust::Engine::default();
        assert!(
            e.execute("cameraCalibrator;")
                .unwrap_err()
                .contains("cameraCalibrator requires the desktop")
        );

        let received = Arc::new(Mutex::new(Vec::new()));
        let log = received.clone();
        let _hook = camcal_app_hooks::present(move |launch| {
            log.lock().unwrap().push(launch);
            Ok(())
        });

        let calling_folder = tempfile::tempdir().unwrap();
        e.set_current_folder(calling_folder.path()).unwrap();
        e.execute("cameraCalibrator;").unwrap();
        e.execute("cameraCalibrator('/tmp/boards', 25);").unwrap();
        e.execute("cameraCalibrator('images', 25);").unwrap();
        e.execute("cameraCalibrator({'a.png', 'b.png'}, 30, 'cm');")
            .unwrap();

        // Rooted path on purpose: on Windows it lands on the current folder's drive.
        #[allow(clippy::join_absolute_paths)]
        let rooted_boards = e
            .current_folder()
            .unwrap()
            .join("/tmp/boards")
            .to_string_lossy()
            .into_owned();
        let got = received.lock().unwrap();
        assert_eq!(got.len(), 4);
        assert_eq!(got[0], Launch::Empty);
        assert_eq!(
            got[1],
            Launch::Folder {
                folder: rooted_boards,
                square_size: 25.0,
                units: None,
            }
        );
        assert_eq!(
            got[2],
            Launch::Folder {
                folder: e
                    .current_folder()
                    .unwrap()
                    .join("images")
                    .to_string_lossy()
                    .into_owned(),
                square_size: 25.0,
                units: None,
            }
        );
        assert_eq!(
            got[3],
            Launch::Files {
                files: ["a.png", "b.png"]
                    .iter()
                    .map(|p| e
                        .current_folder()
                        .unwrap()
                        .join(p)
                        .to_string_lossy()
                        .into_owned())
                    .collect(),
                square_size: 30.0,
                units: Some("cm".into()),
            }
        );

        drop(got);
        let mut app = CameraCalibrator::default();
        app.choose(Launch::Files {
            files: vec![],
            square_size: 25.0,
            units: Some("cm".into()),
        });
        app.choose(Launch::Files {
            files: vec![],
            square_size: 25.0,
            units: None,
        });
        assert_eq!(app.settings.units, cc::Units::Mm);
        assert!(
            e.execute("cameraCalibrator('/tmp/boards', 25, 'parsecs')")
                .unwrap_err()
                .contains("worldUnits")
        );

        for bad in [
            "cameraCalibrator('/tmp/boards')",
            "cameraCalibrator('/tmp/boards', -5)",
            "cameraCalibrator(123, 25)",
            "x = cameraCalibrator;",
        ] {
            assert!(e.execute(bad).is_err(), "{bad}");
        }

        assert!(AppKind::CameraCalibrator.is_built());
        assert_eq!(AppKind::CameraCalibrator.label(), "Camera Calibrator");
        assert_eq!(
            AppKind::from_tab(matrust::session::ToolTab::CameraCalibrator),
            Some(AppKind::CameraCalibrator)
        );
    }
    #[test]
    fn camera_calibrator_import_limit_and_retired_run() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/calib/images");
        let mut h = host();
        h.app.set_folder(dir.to_string_lossy().into_owned());
        inputs_frame(&mut h, &ws, &ctx);
        assert!(
            h.app.import_running() && h.app.images.is_empty(),
            "the first frame must draw while the folder scan is in flight"
        );
        wait(&mut h, &ws, &ctx);
        assert!(h.app.images.len() >= 10);
        click(&mut h, &ws, &ctx, "Calibrate");
        let launches = Arc::new(Mutex::new(Vec::new()));
        let captured = launches.clone();
        let _hook = camcal_app_hooks::present(move |launch| {
            captured.lock().unwrap().push(launch);
            Ok(())
        });
        let mut engine = matrust::Engine::with_limits(matrust::engine::Limits::local());
        let other = tempfile::tempdir().unwrap();
        std::fs::write(other.path().join("new-a.png"), []).unwrap();
        std::fs::write(other.path().join("new-b.png"), []).unwrap();
        engine
            .execute(&format!(
                "cameraCalibrator('{}', 250000);",
                other.path().display()
            ))
            .unwrap();
        h.app.choose(launches.lock().unwrap().pop().unwrap());
        wait(&mut h, &ws, &ctx);
        assert_eq!(h.app.settings.square_size, 250000.0);
        assert_eq!(h.app.images.len(), 2);
        assert_eq!(
            h.app.images[0].path,
            other.path().join("new-a.png").to_string_lossy()
        );
        assert!(h.app.result.is_none());
        h.app
            .set_files((0..501).map(|i| format!("/tmp/{i}.png")).collect());
        wait(&mut h, &ws, &ctx);
        assert!(h.app.images.is_empty());
        assert!(h.app.error.as_deref().unwrap().contains("500"));
        h.app
            .set_files(vec!["/tmp/a.png".into(), "/tmp/b.png".into()]);
        wait(&mut h, &ws, &ctx);
        h.app
            .append_files(vec!["/tmp/b.png".into(), "/tmp/c.png".into()]);
        wait(&mut h, &ws, &ctx);
        assert_eq!(
            h.app
                .images
                .iter()
                .map(|e| e.path.as_str())
                .collect::<Vec<_>>(),
            ["/tmp/a.png", "/tmp/b.png", "/tmp/c.png"]
        );
        h.app
            .append_files((0..501).map(|i| format!("/tmp/extra-{i}.png")).collect());
        wait(&mut h, &ws, &ctx);
        assert_eq!(h.app.images.len(), 3);
        assert!(h.app.error.as_deref().unwrap().contains("500"));
        h.app
            .set_files(vec!["/tmp/first.png".into(), "/tmp/second.png".into()]);
        frame(&mut h, &ws, &ctx, vec![]);
        h.app.append_files(vec!["/tmp/third.png".into()]);
        wait(&mut h, &ws, &ctx);
        assert_eq!(h.app.images.len(), 3);
        assert_eq!(h.app.images[2].path, "/tmp/third.png");
        click(&mut h, &ws, &ctx, "Clear");
        wait(&mut h, &ws, &ctx);
        assert!(h.app.images.is_empty());
        assert!(h.app.error.is_none());
    }

    #[test]
    fn camera_calibrator_add_images_after_reopen_and_disable_remove_during_import() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/calib/images");
        let folder = tempfile::tempdir().unwrap();
        std::fs::copy(
            fixture.join("board_01.png"),
            folder.path().join("board_01.png"),
        )
        .unwrap();
        let mut h = host();
        h.app
            .set_folder(folder.path().to_string_lossy().into_owned());
        wait(&mut h, &ws, &ctx);
        assert_eq!(h.app.images.len(), 1);
        assert!(!h.app.controls.1["Calibrate"]);

        cc::close(&mut h);
        h.app.choose(Launch::Empty);
        wait(&mut h, &ws, &ctx);
        click(&mut h, &ws, &ctx, "Add images");
        h.app.append_files(vec![
            fixture.join("board_02.png").to_string_lossy().into_owned(),
            fixture.join("board_03.png").to_string_lossy().into_owned(),
        ]);
        inputs_frame(&mut h, &ws, &ctx);
        assert!(
            h.app.import_running(),
            "queued add did not start after reopening"
        );
        assert!(!h.app.controls.1["Remove"]);
        assert!(!h.app.controls.1["Calibrate"]);

        wait(&mut h, &ws, &ctx);
        assert_eq!(h.app.images.len(), 3);
        assert!(h.app.controls.1["Calibrate"] && h.app.controls.1["Remove"]);
        assert!(!h.app.stale);
        assert!(visible_text(
            &frame(&mut h, &ws, &ctx, vec![]).0,
            "None yet"
        ));
        click(&mut h, &ws, &ctx, "Remove");
        assert_eq!(h.app.images.len(), 2);
        assert!(!h.app.stale);
        assert!(visible_text(
            &frame(&mut h, &ws, &ctx, vec![]).0,
            "None yet"
        ));
    }
    #[test]
    fn camera_calibrator_logical_preview() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("binary.png");
        let file = std::fs::File::create(&path).unwrap();
        let mut encoder = png::Encoder::new(file, 8, 8);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::One);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[0, 255, 0, 255, 0, 255, 0, 255])
            .unwrap();
        let uint16_path = dir.path().join("uint16.png");
        image::ImageBuffer::from_pixel(8, 8, image::Luma([32768_u16]))
            .save(&uint16_path)
            .unwrap();
        let mut h = host();
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        h.app.set_files(
            [path, uint16_path]
                .map(|p| p.to_string_lossy().into_owned())
                .to_vec(),
        );
        wait(&mut h, &ws, &ctx);
        let preview = h
            .app
            .view
            .as_ref()
            .unwrap_or_else(|| panic!("{:?}", h.app.view_error));
        assert_eq!(preview.axes.extras.color_limits(), [0.0, 1.0]);
        click(&mut h, &ws, &ctx, "image:1");
        wait(&mut h, &ws, &ctx);
        assert_eq!(
            h.app.view.as_ref().unwrap().axes.extras.color_limits(),
            [0.0, 65535.0]
        );
    }
    #[test]
    fn camera_calibrator_rejected_image_and_gif() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/calib/images");
        let dir = tempfile::tempdir().unwrap();
        for k in 1..=9 {
            let name = format!("board_{k:02}.png");
            std::fs::copy(fixture.join(&name), dir.path().join(name)).unwrap();
        }
        let reference = image::open(fixture.join("board_01.png")).unwrap();
        let blank =
            image::GrayImage::from_pixel(reference.width(), reference.height(), image::Luma([127]));
        blank.save(dir.path().join("blank.png")).unwrap();
        std::fs::write(dir.path().join("ignored.gif"), b"not an image").unwrap();
        let mut h = host();
        h.app.set_folder(dir.path().to_string_lossy().into_owned());
        wait(&mut h, &ws, &ctx);
        assert_eq!(h.app.images.len(), 10);
        std::fs::copy(
            fixture.join("board_10.png"),
            dir.path().join("added_later.png"),
        )
        .unwrap();
        click(&mut h, &ws, &ctx, "Calibrate");
        wait(&mut h, &ws, &ctx);
        assert!(h.app.error.is_none(), "{:?}", h.app.error);
        assert_eq!(h.app.result.as_ref().unwrap().total_images, 10);
        assert_eq!(h.app.result.as_ref().unwrap().images_used, 9);
        assert_eq!(
            h.app
                .images
                .iter()
                .filter(|e| e.status == ImageStatus::Rejected)
                .count(),
            1
        );
        h.app.settings.num_radial = 0;
        h.app.calibrate_requested = true;
        wait(&mut h, &ws, &ctx);
        assert!(h.app.error.is_some());
        assert!(h.app.result.is_none());
        assert_eq!(
            h.app
                .images
                .iter()
                .filter(|e| e.status == ImageStatus::Detected)
                .count(),
            9
        );
        assert_eq!(
            h.app
                .images
                .iter()
                .filter(|e| e.status == ImageStatus::Rejected)
                .count(),
            1
        );
        assert!(!h.app.controls.1["Export"]);
        assert!(!h.app.controls.1["Generate script"]);
        h.app.settings.num_radial = 2;
        h.app.calibrate_requested = true;
        cc::refresh(&mut h, &ctx, &ws);
        assert!(h.app.running);
        cc::stop(&mut h);
        wait(&mut h, &ws, &ctx);
        assert!(h.app.result.is_none());
        assert!(
            h.app
                .images
                .iter()
                .all(|entry| entry.status == ImageStatus::Pending)
        );
    }
    #[test]
    fn camera_calibrator_large_square_scales_world_points() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/calib/images");
        let received = Arc::new(Mutex::new(None));
        let slot = received.clone();
        let _hook = camcal_app_hooks::present(move |launch| {
            *slot.lock().unwrap() = Some(launch);
            Ok(())
        });
        let mut engine = matrust::Engine::with_limits(matrust::engine::Limits::local());
        engine
            .execute(&format!(
                "cameraCalibrator('{}', 250000);",
                dir.to_string_lossy().replace('\'', "''")
            ))
            .unwrap();
        let mut h = host();
        h.app.choose(received.lock().unwrap().take().unwrap());
        wait(&mut h, &ws, &ctx);
        assert_eq!(h.app.settings.square_size, 250000.0);
        click(&mut h, &ws, &ctx, "Calibrate");
        wait(&mut h, &ws, &ctx);
        assert!(h.app.error.is_none(), "{:?}", h.app.error);
        h.app.names = ["scaledParams".into(), "scaledErrors".into()];
        let _ticket_guard = EXPORT_TICKET_TESTS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(toolkits::Action::Script(code)) = click(&mut h, &ws, &ctx, "Export") else {
            panic!("export")
        };
        engine.execute(&code).unwrap();
        assert_eq!(engine.workspace_count(), 2);
        assert!(logical(
            &mut engine,
            "isa(scaledErrors, 'cameraCalibrationErrors')"
        ));
        engine
            .execute("cc_world = scaledParams.WorldPoints; cc_second_y = cc_world(2, 2);")
            .unwrap();
        assert_eq!(scalar(&engine, "cc_second_y"), 250000.0);
    }

    #[test]
    fn camera_calibrator_per_view_mean_ignores_nan_pairs() {
        let (mut e, files) = fixture_fit();
        e.execute(
            "cc_mean_err = cameraParams.MeanReprojectionError;\n\
             cc_focal = cameraParams.FocalLength;\n\
             cc_pp = cameraParams.PrincipalPoint;\n\
             cc_radial = cameraParams.RadialDistortion;\n\
             cc_tang = cameraParams.TangentialDistortion;\n\
             cc_skew = cameraParams.Skew;\n\
             cc_imsize = cameraParams.ImageSize;\n\
             errors = cameraParams.ReprojectionErrors;\n\
             errors(1, :, 1) = NaN;\n\
             cc_errors = errors(:,:,1:2);\n\
             cc_images_used = logical([1 1]); cc_estimator_used = logical([1 1]);",
        )
        .unwrap();
        let out: Outputs = [
            "cc_mean_err",
            "cc_focal",
            "cc_pp",
            "cc_radial",
            "cc_tang",
            "cc_skew",
            "cc_imsize",
            "cc_board_size",
            "cc_errors",
            "cc_images_used",
            "cc_estimator_used",
        ]
        .into_iter()
        .filter_map(|k| e.array(k).map(|v| (k.to_string(), v)))
        .collect();
        let values = CalibrationValues::capture(&e, ["cameraParams", "estimationErrors"]).unwrap();
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = host();
        h.app.set_files(files[..2].to_vec());
        wait(&mut h, &ws, &ctx);
        let res = cc::make_result(h.app.spec(), &out, values).unwrap();
        assert!(res.per_view_errors[0].is_finite(), "{:?}", res.per_view_errors);
        assert!(res.per_view_errors[1].is_finite());
    }

    #[test]
    fn camera_calibrator_uppercase_folder_imports_two() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let dir = tempfile::tempdir().unwrap();
        let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/calib/images");
        std::fs::copy(src.join("board_01.png"), dir.path().join("A.JPG")).unwrap();
        std::fs::copy(src.join("board_02.png"), dir.path().join("B.JPG")).unwrap();
        let mut h = host();
        h.app.choose(Launch::Folder {
            folder: dir.path().to_string_lossy().into_owned(),
            square_size: 25.0,
            units: None,
        });
        wait(&mut h, &ws, &ctx);
        assert!(h.app.error.is_none(), "{:?}", h.app.error);
        assert_eq!(h.app.images.len(), 2);
    }

    #[test]
    fn camera_calibrator_64_view_generated_script_returns_both_outputs() {
        let (_, files) = fixture_fit();
        let dir = tempfile::tempdir().unwrap();
        let repeated: Vec<String> = (0..64)
            .map(|i| {
                let dst = dir.path().join(format!("view_{i:02}.png"));
                std::fs::copy(&files[i % files.len()], &dst).unwrap();
                dst.to_string_lossy().into_owned()
            })
            .collect();
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = host();
        h.app.set_files(repeated);
        wait(&mut h, &ws, &ctx);
        click(&mut h, &ws, &ctx, "Calibrate");
        wait(&mut h, &ws, &ctx);
        assert!(h.app.error.is_none(), "{:?}", h.app.error);
        let script = h.app.script(&ws).unwrap();
        assert!(script.contains("% Extrinsics plots skipped:"), "{script}");
        let mut fresh = matrust::Engine::with_limits(matrust::engine::Limits::local());
        fresh.execute(&script).unwrap();
        assert!(logical(
            &mut fresh,
            "isa(cameraParams, 'cameraParameters')"
        ));
        assert!(logical(
            &mut fresh,
            "isa(estimationErrors, 'cameraCalibrationErrors')"
        ));
    }
}
