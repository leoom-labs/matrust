// Reference values are fresh engine executions of public scripts and direct
// physical identities; no expectations are derived from the app view alone.
mod comm_apps_tests {
    use super::*;
    use apps::ber_analysis::{self as ber, BerAnalysis, Modulation};
    use apps::waveform_generator::{self as wave, Pulse, WaveformGenerator};
    use apps::{App, AppKind, Host, Snapshot};
    use matrust::{Array, Complex64, Engine};

    fn numbers(e: &Engine, n: &str) -> Vec<f64> {
        match e.array(n) {
            Some(Array::Numeric(m)) => m.data().to_vec(),
            a => panic!("missing numeric {n}: {a:?}"),
        }
    }
    fn complex(a: &Array) -> Vec<Complex64> {
        match a {
            Array::Complex(m) => m.data().to_vec(),
            Array::Numeric(m) => m.data().iter().map(|&x| Complex64::new(x, 0.)).collect(),
            _ => panic!("expected numeric waveform"),
        }
    }
    fn close(a: &[f64], b: &[f64], tol: f64) {
        assert_eq!(a.len(), b.len());
        for (a, b) in a.iter().zip(b) {
            assert!(
                (a - b).abs() <= tol || (a.is_nan() && b.is_nan()),
                "{a} != {b}"
            );
        }
    }
    fn same(a: &Array, b: &Array) {
        let a = complex(a);
        let b = complex(b);
        assert_eq!(a.len(), b.len());
        for (a, b) in a.iter().zip(b) {
            assert!((*a - b).norm() < 1e-12, "{a:?} != {b:?}");
        }
    }
    fn settle<A: App>(h: &mut Host<A>, ctx: &egui::Context, ws: &Snapshot) {
        let until = Instant::now() + Duration::from_secs(90);
        loop {
            h.refresh(ctx, ws);
            if !h.running() {
                break;
            }
            assert!(Instant::now() < until, "{}", h.status().0);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn run_ber(h: &mut Host<BerAnalysis>, ctx: &egui::Context, ws: &Snapshot) {
        h.app.start().unwrap();
        settle(h, ctx, ws);
        assert_ne!(h.status().1, toolkits::AppStatus::Error, "{}", h.status().0);
        assert!(!h.app.datasets.is_empty());
    }
    fn run_wave(h: &mut Host<WaveformGenerator>, ctx: &egui::Context, ws: &Snapshot) {
        h.app.start(ws).unwrap();
        settle(h, ctx, ws);
        assert_ne!(h.status().1, toolkits::AppStatus::Error, "{}", h.status().0);
        assert!(h.app.current.is_some());
    }
    #[test]
    fn comm_apps_ber_theory_matches_berawgn_qam_and_coherent_bfsk() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        for (kind, m, call) in [
            (Modulation::Qam, 16, "berawgn(0:2:10,'qam',16)"),
            (Modulation::Fsk, 2, "berawgn(0:2:10,'fsk',2,'coherent')"),
        ] {
            let mut h = Host::new(BerAnalysis::default());
            h.app.spec.modulation = kind;
            h.app.spec.order = m;
            run_ber(&mut h, &ctx, &ws);
            let mut e = Engine::default();
            e.execute(&format!("[b,s]={call};")).unwrap();
            let d = &h.app.datasets[0];
            close(&d.ber, &numbers(&e, "b"), 1e-14);
            close(&d.ser, &numbers(&e, "s"), 1e-14);
        }
    }
    #[test]
    fn comm_apps_qpsk_monte_carlo_seeded_replay_and_theory() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(BerAnalysis::default());
        h.app.spec.simulation = true;
        h.app.spec.modulation = Modulation::Psk;
        h.app.spec.order = 4;
        h.app.spec.ebno = "4".into();
        h.app.spec.target_errors = 2000;
        h.app.spec.max_bits = 300_000;
        run_ber(&mut h, &ctx, &ws);
        let d = &h.app.datasets[0];
        let mut e = Engine::default();
        e.execute(&h.app.script(&ws).unwrap()).unwrap();
        e.execute("b=ber_results.BER; n=ber_results.Errors; t=ber_results.Bits; theory=berawgn(4,'psk',4,'nondiff');").unwrap();
        close(&d.ber, &numbers(&e, "b"), 0.);
        close(&d.errors, &numbers(&e, "n"), 0.);
        close(&d.bits, &numbers(&e, "t"), 0.);
        assert!(
            (d.ber[0] / numbers(&e, "theory")[0] - 1.).abs() < 0.2,
            "{}",
            d.ber[0]
        );
        assert!(d.bits[0] <= 300_000.);
    }
    #[test]
    fn comm_apps_stop_keeps_partial_points_and_replays() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(BerAnalysis::default());
        h.app.spec.simulation = true;
        h.app.spec.modulation = Modulation::Psk;
        h.app.spec.order = 4;
        h.app.spec.ebno = "0:0.25:20".into();
        h.app.spec.target_errors = 100;
        h.app.spec.max_bits = 200_000;
        h.app.start().unwrap();
        h.refresh(&ctx, &ws);
        let until = Instant::now() + Duration::from_secs(30);
        // A completed point is published before the next begins.
        while h.app.completed_points() == 0 {
            h.refresh(&ctx, &ws);
            assert!(Instant::now() < until, "{}", h.status().0);
            std::thread::sleep(Duration::from_millis(1));
        }
        h.app.request_stop();
        settle(&mut h, &ctx, &ws);
        assert!(!h.app.datasets.is_empty(), "{}", h.status().0);
        let d = &h.app.datasets[0];
        assert!(d.partial);
        assert!(!d.x.is_empty() && d.x.len() < 81);
        let mut e = Engine::default();
        e.execute(&h.app.script(&ws).unwrap()).unwrap();
        e.execute("b=ber_results.BER; partial=ber_results.settings.Partial;")
            .unwrap();
        close(&d.ber, &numbers(&e, "b"), 0.);
        assert_eq!(numbers(&e, "partial"), vec![1.]);
    }
    #[test]
    fn comm_apps_invalid_ebno_keeps_result_and_rejects_injection() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(BerAnalysis::default());
        run_ber(&mut h, &ctx, &ws);
        let before = h.app.datasets[0].ber.clone();
        for bad in [
            "0:2:10; evil=1",
            "[1 2] % comment",
            "system('oops')",
            "0:1e-100:10",
            "[1 NaN]",
            "2:-1:0",
            "[0 0]",
            "[1 2;3 4]",
        ] {
            h.app.spec.ebno = bad.into();
            h.app.error = h.app.start().err();
            assert!(h.app.error.is_some(), "accepted {bad}");
            h.refresh(&ctx, &ws);
            assert_eq!(h.status().1, toolkits::AppStatus::Error);
            assert_eq!(h.app.datasets[0].ber, before);
        }
        assert_eq!(
            ber::numeric_text("[-2 0 1+2]", 128).unwrap(),
            vec![-2., 0., 3.]
        );
    }
    #[test]
    fn comm_apps_berfit_overlay_matches_direct_engine_fit() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(BerAnalysis::default());
        h.app.spec.simulation = true;
        h.app.spec.modulation = Modulation::Psk;
        h.app.spec.order = 4;
        h.app.spec.ebno = "0:2:6".into();
        h.app.spec.target_errors = 200;
        h.app.fit_requested = true;
        run_ber(&mut h, &ctx, &ws);
        let d = &h.app.datasets[0];
        assert_eq!(d.fit.len(), 4);
        let mut e = Engine::default();
        e.execute(&format!(
            "x={};y=berfit(x,{},x,[],'exp');",
            ber::vector(&d.x),
            ber::vector(&d.ber)
        ))
        .unwrap();
        close(
            &d.fit.iter().map(|p| p[1]).collect::<Vec<_>>(),
            &numbers(&e, "y"),
            1e-14,
        );
    }
    #[test]
    fn comm_apps_waveform_each_type_matches_export_and_script() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        for (kind, ofdm) in [
            (Modulation::Psk, false),
            (Modulation::Qam, false),
            (Modulation::Pam, false),
            (Modulation::Fsk, false),
            (Modulation::Msk, false),
            (Modulation::Qam, true),
            (Modulation::Psk, true),
        ] {
            let mut h = Host::new(WaveformGenerator::default());
            h.app.spec.modulation = kind;
            h.app.spec.ofdm = ofdm;
            h.app.spec.order = if kind == Modulation::Msk { 2 } else { 4 };
            h.app.spec.symbols = 16;
            h.app.spec.sps = if kind == Modulation::Fsk { 8 } else { 2 };
            run_wave(&mut h, &ctx, &ws);
            let mut e = Engine::default();
            e.execute(&h.app.script(&ws).unwrap()).unwrap();
            same(
                &h.app.current.as_ref().unwrap().waveform,
                &e.array("waveform").unwrap(),
            );
            let Some(toolkits::Action::ExportScript(code)) = h.export(&ws) else {
                panic!()
            };
            let mut exported = Engine::default();
            exported.execute(&code).unwrap();
            same(
                &e.array("waveform").unwrap(),
                &exported.array("waveform").unwrap(),
            );
            assert!(
                matches!(e.array("waveform"), Some(Array::Complex(_))),
                "{kind:?} ofdm={ofdm}"
            );
            assert_eq!(e.array("waveform").unwrap().cols(), 1);
            assert_eq!(
                h.app.current.as_ref().unwrap().charts.len(),
                5,
                "{kind:?} ofdm={ofdm}"
            );
        }
    }
    #[test]
    fn comm_apps_rrc_matched_filter_recovers_symbols() {
        let mut s = wave::Spec {
            pulse: Pulse::Root,
            span: 32,
            symbols: 128,
            ..Default::default()
        };
        s.sps = 4;
        let mut e = Engine::default();
        e.execute(&wave::build_script(&s, &["w".into(), "cfg".into()]).unwrap())
            .unwrap();
        e.execute("r=complex(upfirdn(real(w),cfg.Filter,1,cfg.SamplesPerSymbol),upfirdn(imag(w),cfg.Filter,1,cfg.SamplesPerSymbol)); r=r(cfg.Span+1:cfg.Span+cfg.Symbols); ref=cfg.ReferenceSymbols; delta=max(abs(r-ref));").unwrap();
        assert!(
            numbers(&e, "delta")[0] < 0.003,
            "{:?}",
            numbers(&e, "delta")
        );
    }
    #[test]
    fn comm_apps_frequency_offset_and_papr_direct_identity() {
        let mut s = wave::Spec {
            symbols: 64,
            sps: 4,
            ..Default::default()
        };
        let names = ["w".into(), "cfg".into()];
        let mut e = Engine::default();
        e.execute(&wave::build_script(&s, &names).unwrap()).unwrap();
        let clean = complex(&e.array("w").unwrap());
        s.frequency = 12500.;
        s.phase = 30.;
        e.execute(&wave::build_script(&s, &names).unwrap()).unwrap();
        let rotated = complex(&e.array("w").unwrap());
        for (i, (&a, &b)) in clean.iter().zip(&rotated).enumerate() {
            let p = std::f64::consts::TAU * s.frequency * i as f64 / s.sample_rate
                + std::f64::consts::PI / 6.;
            assert!((b - a * Complex64::new(p.cos(), p.sin())).norm() < 1e-12);
        }
        e.execute("app_p=cfg.PAPR; app_power=cfg.Power;").unwrap();
        let powers = rotated.iter().map(|z| z.norm_sqr()).collect::<Vec<_>>();
        let mean = powers.iter().sum::<f64>() / powers.len() as f64;
        let papr = 10. * (powers.iter().copied().fold(0., f64::max) / mean).log10();
        close(&numbers(&e, "app_p"), &[papr], 1e-12);
        close(&numbers(&e, "app_power"), &[mean], 1e-12);
    }
    #[test]
    fn comm_apps_ofdm_nulls_and_pilots_in_centered_fft() {
        let s = wave::Spec {
            ofdm: true,
            modulation: Modulation::Qam,
            order: 16,
            symbols: 4,
            sps: 1,
            nulls: "[1 2 33 64]".into(),
            pilots: "[8 56]".into(),
            ..Default::default()
        };
        let mut e = Engine::default();
        e.execute(&wave::build_script(&s, &["w".into(), "cfg".into()]).unwrap())
            .unwrap();
        e.execute("frames=reshape(w,80,4); freq=fftshift(fft(frames(17:80,:),64),1); zero=abs(freq([1 2 33 64],:)); pilots=freq([8 56],:);").unwrap();
        assert!(numbers(&e, "zero").iter().all(|v| *v < 1e-12));
        for p in complex(&e.array("pilots").unwrap()) {
            assert!((p - Complex64::new(1., 0.)).norm() < 1e-12);
        }
    }
    #[test]
    fn comm_apps_workspace_bits_captured_and_names_rollback() {
        let ctx = egui::Context::default();
        let mut ws = Snapshot::default();
        let mut source = Engine::default();
        source.execute("bits=uint8(mod((1:64)',2));").unwrap();
        ws.sync(
            &[("bits".into(), source.array("bits").unwrap())]
                .into_iter()
                .collect(),
        );
        let mut h = Host::new(WaveformGenerator::default());
        h.app.spec.random = false;
        h.app.spec.symbols = 32;
        run_wave(&mut h, &ctx, &ws);
        let script = h.app.script(&ws).unwrap();
        source.execute("bits=zeros(64,1);").unwrap();
        let mut e = Engine::default();
        e.execute(&script).unwrap();
        same(
            &h.app.current.as_ref().unwrap().waveform,
            &e.array("waveform").unwrap(),
        );
        for name in [
            "mrexp_old",
            "matrust_export_error",
            "app_comm_wave",
            "x;evil=1",
            "for",
            "a%",
        ] {
            h.app.names[0] = name.into();
            assert!(h.app.script(&ws).is_err(), "accepted {name}");
            assert!(ber::checked_name(name).is_err());
        }
        h.app.names = ["waveform".into(), "cfg".into()];
        let code = apps::all_or_nothing("waveform=3;cfg=4;error('forced');", &h.app.exported(&ws));
        e.execute("waveform=17;cfg=19;").unwrap();
        assert!(e.execute(&code).is_err());
        assert_eq!(numbers(&e, "waveform"), vec![17.]);
        assert_eq!(numbers(&e, "cfg"), vec![19.]);
        let mut invalid = wave::Spec {
            frequency: f64::INFINITY,
            ..Default::default()
        };
        assert!(invalid.validate().is_err());
        invalid = wave::Spec {
            ofdm: true,
            nulls: "[1];evil=1".into(),
            ..Default::default()
        };
        assert!(invalid.validate().is_err());
    }
    #[test]
    fn script_launchers_keep_the_last_mode_and_focus() {
        use apps::distribution_explorer::Mode;
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        for (code, selected, caption) in [
            (
                "disttool; randtool; disttool; imtool",
                AppKind::ImageViewer,
                "Choose a workspace image",
            ),
            (
                "imtool; bertool",
                AppKind::BitErrorRateAnalysis,
                "Eb/N0 (dB)",
            ),
            (
                "bertool; imtool",
                AppKind::ImageViewer,
                "Choose a workspace image",
            ),
        ] {
            app.run(code.into());
            finish(&mut app);
            assert!(app.toolkits.apps.is_open(AppKind::DistributionExplorer));
            assert_eq!(app.toolkits.apps.explorer.app.mode, Mode::Explore);
            assert_eq!(
                app.toolkits.selected_tool(),
                Some(toolkits::Tool::App(selected)),
                "{code}"
            );
            let texts = clear_panes_texts(&super::frame(&mut app, &ctx, vec![]));
            assert!(
                texts
                    .iter()
                    .any(|t| t.text.contains(caption) && t.clip.contains_rect(t.glyphs)),
                "{caption} after {code}"
            );
        }
    }

    #[test]
    fn command_launchers_select_the_apps_menu_windows() {
        let mut app = Workspace::with_context(egui::Context::default(), true);
        let dir = tempfile::tempdir().unwrap();
        let run = |app: &mut Workspace, command: &str| {
            app.submit(worker::Job::ScriptInFolder(
                command.into(),
                dir.path().to_owned(),
            ));
            let until = Instant::now() + Duration::from_secs(20);
            while app.busy {
                app.receive();
                assert!(Instant::now() < until, "{command}");
                std::thread::sleep(Duration::from_millis(2));
            }
        };
        for (command, kind) in [
            ("signalAnalyzer", AppKind::SignalAnalyzer),
            ("fvtool", AppKind::FilterVisualizer),
            ("windowDesigner", AppKind::WindowDesigner),
            ("signalMultiresolutionAnalyzer", AppKind::MultiresAnalyzer),
            ("waveletSignalDenoiser", AppKind::WaveletDenoiser),
            ("waveletTimeFrequencyAnalyzer", AppKind::WaveletTfAnalyzer),
            ("pidTuner", AppKind::PidTuner),
            ("controlSystemDesigner", AppKind::ControlSystemDesigner),
            ("linearSystemAnalyzer", AppKind::LinearSystemAnalyzer),
            ("modelReducer", AppKind::ModelReducer),
            ("systemIdentification", AppKind::SystemIdentification),
            ("curveFitter", AppKind::CurveFitter),
            ("cftool", AppKind::CurveFitter),
            ("splineTool", AppKind::SplineTool),
            ("distributionFitter", AppKind::DistributionFitter),
            ("disttool", AppKind::DistributionExplorer),
            ("randtool", AppKind::DistributionExplorer),
            ("classificationLearner", AppKind::ClassificationLearner),
            ("regressionLearner", AppKind::RegressionLearner),
            ("imageViewer", AppKind::ImageViewer),
            ("imtool", AppKind::ImageViewer),
            ("imageSegmenter", AppKind::ImageSegmenter),
            ("colorThresholder", AppKind::ColorThresholder),
            ("imageRegionAnalyzer", AppKind::ImageRegionAnalyzer),
            ("imageBatchProcessor", AppKind::ImageBatchProcessor),
            ("dataCleaner", AppKind::DataCleaner),
            ("uiimport", AppKind::ImportTool),
        ] {
            run(&mut app, command);
            assert!(app.toolkits.apps.is_open(kind), "{command}: {}", app.status);
            assert_eq!(
                app.toolkits.selected_tool(),
                Some(toolkits::Tool::App(kind)),
                "{command}"
            );
            use apps::distribution_explorer::Mode;
            match command {
                "disttool" => assert_eq!(app.toolkits.apps.explorer.app.mode, Mode::Explore),
                "randtool" => assert_eq!(app.toolkits.apps.explorer.app.mode, Mode::Samples),
                _ => {}
            }
            // An alias must open its own target even when it follows the canonical command.
            app.close_tool_tab(toolkits::Tool::App(kind));
        }
        run(&mut app, "randtool");
        assert_eq!(
            app.toolkits.apps.explorer.app.mode,
            apps::distribution_explorer::Mode::Samples
        );
        run(&mut app, "disttool");
        assert_eq!(
            app.toolkits.apps.explorer.app.mode,
            apps::distribution_explorer::Mode::Explore
        );
        run(
            &mut app,
            "ans=19; imageViewer(); uiimport(); assert(ans==19); imtool()",
        );
        let image = toolkits::Tool::App(AppKind::ImageViewer);
        let import = toolkits::Tool::App(AppKind::ImportTool);
        assert!(app.toolkits.is_open(import));
        assert_eq!(app.toolkits.selected_tool(), Some(image));
    }

    #[test]
    fn comm_apps_commands_open_desktop_and_session_tabs() {
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx, true);
        let dir = tempfile::tempdir().unwrap();
        for (cmd, kind) in [
            ("bertool", AppKind::BitErrorRateAnalysis),
            (
                "wirelessWaveformGenerator",
                AppKind::WirelessWaveformGenerator,
            ),
        ] {
            assert!(
                Engine::default()
                    .execute(cmd)
                    .unwrap_err()
                    .ends_with(&format!("{cmd} requires the desktop"))
            );
            app.submit(worker::Job::ScriptInFolder(
                cmd.into(),
                dir.path().to_owned(),
            ));
            let until = Instant::now() + Duration::from_secs(20);
            while app.busy {
                app.receive();
                assert!(Instant::now() < until);
                std::thread::sleep(Duration::from_millis(2));
            }
            assert!(app.toolkits.apps.is_open(kind), "{}", app.status);
            assert_eq!(AppKind::from_tab(kind.tab()), Some(kind));
            let json = serde_json::to_string(&kind.tab()).unwrap();
            assert_eq!(
                serde_json::from_str::<matrust::session::ToolTab>(&json).unwrap(),
                kind.tab()
            );
        }
    }
    fn frame<A: App>(h: &mut Host<A>, ctx: &egui::Context, ws: &Snapshot, width: f32) {
        let out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 800.),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    h.show(ui, ws, false, &mut None);
                });
            },
        );
        assert!(!out.shapes.is_empty());
    }
    #[test]
    fn comm_apps_headless_empty_loaded_error_and_narrow_frames() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut b = Host::new(BerAnalysis::default());
        let mut w = Host::new(WaveformGenerator::default());
        frame(&mut b, &ctx, &ws, 1440.);
        frame(&mut w, &ctx, &ws, 1440.);
        run_ber(&mut b, &ctx, &ws);
        let d = b.app.datasets[0].clone();
        b.app.datasets = vec![d; 10];
        for i in 0..10 {
            b.app.datasets[i].label = format!("dataset {i}");
        }
        frame(&mut b, &ctx, &ws, 1440.);
        frame(&mut b, &ctx, &ws, 640.);
        w.app.spec.ofdm = true;
        w.app.spec.modulation = Modulation::Qam;
        w.app.spec.order = 64;
        w.app.spec.symbols = 4096;
        w.app.spec.sps = 1;
        run_wave(&mut w, &ctx, &ws);
        for view in 0..4 {
            w.app.view = view;
            frame(&mut w, &ctx, &ws, 1440.);
            frame(&mut w, &ctx, &ws, 640.);
        }
        b.app.error = Some("Invalid Eb/N0 expression".into());
        w.app.error = Some("Invalid subcarrier indices".into());
        frame(&mut b, &ctx, &ws, 640.);
        frame(&mut w, &ctx, &ws, 640.);
        assert!(w.app.current.is_some());
    }
    #[test]
    fn comm_apps_theory_matches_independent_python_fixture() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/apps/comm/reference.json"
        ))
        .unwrap();
        for (kind, order, key) in [
            (Modulation::Psk, 4, "QPSK"),
            (Modulation::Fsk, 2, "BFSK"),
            (Modulation::Qam, 16, "QAM16"),
        ] {
            let spec = ber::Spec {
                modulation: kind,
                order,
                ..Default::default()
            };
            let mut e = Engine::default();
            e.execute(&ber::build_script(&spec, &spec.grid().unwrap(), "r").unwrap())
                .unwrap();
            e.execute("b=r.BER;").unwrap();
            let expected = fixture[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_f64().unwrap())
                .collect::<Vec<_>>();
            close(&numbers(&e, "b"), &expected, 1e-14);
        }
    }
    #[test]
    fn comm_apps_generated_scripts_save_and_replay_in_temporary_folder() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let dir = tempfile::tempdir().unwrap();
        let mut b = Host::new(BerAnalysis::default());
        run_ber(&mut b, &ctx, &ws);
        let mut w = Host::new(WaveformGenerator::default());
        w.app.spec.symbols = 16;
        run_wave(&mut w, &ctx, &ws);
        for action in [b.generate(&ws, 0), w.generate(&ws, 0)] {
            let Some(toolkits::Action::SaveAppScript(kind, code)) = action else {
                panic!()
            };
            let path = dir.path().join(format!("{:?}.m", kind));
            std::fs::write(&path, &code).unwrap();
            let saved = std::fs::read_to_string(path).unwrap();
            assert!(saved.starts_with("% "));
            Engine::default().execute(&saved).unwrap();
        }
    }
    #[test]
    #[ignore = "release workload timing; run explicitly with --ignored --nocapture"]
    fn comm_apps_release_workloads() {
        let workers = std::env::var("MATRUST_COMM_WORKERS")
            .unwrap_or_else(|_| "1".into())
            .parse::<usize>()
            .unwrap();
        matrust::initialize_worker_pool(workers).unwrap();
        let s = wave::Spec {
            ofdm: true,
            modulation: Modulation::Qam,
            order: 64,
            symbols: 4096,
            sps: 1,
            ..Default::default()
        };
        let script = wave::build_script(&s, &["w".into(), "cfg".into()]).unwrap();
        let mut e = Engine::with_limits(matrust::engine::Limits::local());
        let begin = Instant::now();
        e.execute(&script).unwrap();
        let wave_elapsed = begin.elapsed();
        assert_eq!(e.array("w").unwrap().len(), 327680);
        let filtered = wave::Spec {
            pulse: Pulse::Root,
            ..s.clone()
        };
        let code = wave::build_script(&filtered, &["filtered".into(), "fcfg".into()]).unwrap();
        let start = Instant::now();
        e.execute(&code).unwrap();
        let filtered_elapsed = start.elapsed();
        let fsk = wave::Spec {
            modulation: Modulation::Fsk,
            order: 4,
            symbols: 4096,
            sps: 8,
            pulse: Pulse::Root,
            ..Default::default()
        };
        let code = wave::build_script(&fsk, &["filtered_fsk".into(), "fskcfg".into()]).unwrap();
        let start = Instant::now();
        e.execute(&code).unwrap();
        let fsk_elapsed = start.elapsed();
        let b = ber::Spec {
            modulation: Modulation::Psk,
            order: 4,
            ebno: "0:2:8".into(),
            simulation: true,
            target_errors: 500,
            max_bits: 200000,
            ..Default::default()
        };
        let script = ber::build_script(&b, &b.grid().unwrap(), "r").unwrap();
        let begin = Instant::now();
        e.execute(&script).unwrap();
        eprintln!(
            "comm_apps release workers={workers} OFDM64QAM nfft=64 CP=16 frames=4096 samples=327680 elapsed={wave_elapsed:?}; QPSK MC grid=0:2:8 target=500 maxbits=200000 elapsed={:?}; RRC OFDM elapsed={filtered_elapsed:?}; RRC FSK elapsed={fsk_elapsed:?}; local limits maxwave=1048576 maxbits=2000000",
            begin.elapsed()
        );
    }
    #[test]
    fn comm_apps_all_monte_carlo_modems_replay_and_respect_bit_limits() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        for kind in Modulation::SIM {
            let mut h = Host::new(BerAnalysis::default());
            h.app.spec = ber::Spec {
                simulation: true,
                modulation: kind,
                order: if kind == Modulation::Msk { 2 } else { 4 },
                ebno: "2".into(),
                target_errors: 100,
                max_bits: 8192,
                ..Default::default()
            };
            run_ber(&mut h, &ctx, &ws);
            let d = &h.app.datasets[0];
            assert!(d.bits[0] > 0. && d.bits[0] <= 8192.);
            assert!(d.ber[0] >= 0. && d.ber[0] <= 1.);
            let mut e = Engine::default();
            e.execute(&h.app.script(&ws).unwrap()).unwrap();
            e.execute("b=ber_results.BER; errors=ber_results.Errors; bits=ber_results.Bits;")
                .unwrap();
            close(&d.ber, &numbers(&e, "b"), 0.);
            close(&d.errors, &numbers(&e, "errors"), 0.);
            close(&d.bits, &numbers(&e, "bits"), 0.);
        }
        assert_eq!(
            ber::numeric_text("linspace(-2,2,3)", 128).unwrap(),
            vec![-2., 0., 2.]
        );
    }
    #[test]
    fn comm_apps_impaired_raised_cosine_waveform_replays_and_error_retains_result() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(WaveformGenerator::default());
        h.app.spec = wave::Spec {
            modulation: Modulation::Pam,
            pulse: Pulse::Raised,
            noise: true,
            frequency: 1250.,
            phase: 30.,
            iq_gain: 1.5,
            iq_phase: 11.25,
            dc_i: 0.2,
            dc_q: -0.1,
            ..Default::default()
        };
        run_wave(&mut h, &ctx, &ws);
        let before = h.app.current.as_ref().unwrap().waveform.clone();
        let mut e = Engine::default();
        e.execute(&h.app.script(&ws).unwrap()).unwrap();
        same(&before, &e.array("waveform").unwrap());
        h.app.spec.frequency = f64::NAN;
        h.app.error = h.app.start(&ws).err();
        h.refresh(&ctx, &ws);
        assert_eq!(h.status().1, toolkits::AppStatus::Error);
        assert!(before.same_snapshot(&h.app.current.as_ref().unwrap().waveform));
    }
    #[test]
    fn comm_apps_multiple_selected_datasets_export_as_struct_array() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(BerAnalysis::default());
        run_ber(&mut h, &ctx, &ws);
        h.app.spec = ber::Spec {
            simulation: true,
            modulation: Modulation::Psk,
            order: 4,
            ebno: "4".into(),
            target_errors: 100,
            max_bits: 8192,
            ..Default::default()
        };
        run_ber(&mut h, &ctx, &ws);
        h.app.datasets[0].selected = true;
        let Some(toolkits::Action::ExportScript(script)) = h.export(&ws) else {
            panic!()
        };
        let mut e = Engine::default();
        e.execute(&script).unwrap();
        e.execute("count=numel(ber_results); b1=ber_results(1).BER; b2=ber_results(2).BER;")
            .unwrap();
        assert_eq!(numbers(&e, "count"), vec![2.]);
        close(&h.app.datasets[0].ber, &numbers(&e, "b1"), 0.);
        close(&h.app.datasets[1].ber, &numbers(&e, "b2"), 0.);
        for bad in ["mrexp_old", "app_comm_ber_0", "for", "x;evil=1"] {
            h.app.name = bad.into();
            assert!(h.export(&ws).is_none(), "accepted {bad}");
        }
    }
    #[test]
    fn comm_apps_pulse_filters_cover_fsk_msk_and_ofdm() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        for (kind, ofdm) in [
            (Modulation::Fsk, false),
            (Modulation::Msk, false),
            (Modulation::Qam, true),
        ] {
            for pulse in [Pulse::Raised, Pulse::Root] {
                let mut h = Host::new(WaveformGenerator::default());
                h.app.spec = wave::Spec {
                    modulation: kind,
                    order: if kind == Modulation::Msk { 2 } else { 4 },
                    ofdm,
                    pulse,
                    symbols: 8,
                    sps: 8,
                    ..Default::default()
                };
                run_wave(&mut h, &ctx, &ws);
                let mut e = Engine::default();
                e.execute(&h.app.script(&ws).unwrap()).unwrap();
                same(
                    &h.app.current.as_ref().unwrap().waveform,
                    &e.array("waveform").unwrap(),
                );
                let expected = if ofdm {
                    (64 + 16) * 8 * 8 + 10 * 8
                } else {
                    8 * 8 + 10 * 8
                };
                assert_eq!(e.array("waveform").unwrap().len(), expected);
                assert_eq!(h.app.current.as_ref().unwrap().charts.len(), 5);
            }
        }
    }
    #[test]
    fn comm_apps_click_launches_ber_theory_monte_carlo_and_waveform() {
        let ws = Snapshot::default();
        let find_label = |output: &egui::FullOutput, label: &str| {
            let mut found = Vec::new();
            for shape in &output.shapes {
                tool_tab_texts(&shape.shape, label, &mut found);
            }
            *found.first().unwrap_or_else(|| panic!("missing {label}"))
        };
        let click_events = |pos: egui::Pos2| {
            [true, false]
                .into_iter()
                .flat_map(move |pressed| {
                    [
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]
                })
                .collect::<Vec<_>>()
        };

        // 1. BER Analysis: Theory + Monte Carlo via real click through ber::show
        for simulation in [false, true] {
            let ctx = egui::Context::default();
            let mut h = Host::new(BerAnalysis::default());
            h.app.spec.simulation = simulation;
            h.app.spec.ebno = "0:2:4".into();
            h.app.spec.target_errors = 20;
            h.app.spec.max_bits = 2000;
            let run_frame = |h: &mut Host<BerAnalysis>, events: Vec<egui::Event>| {
                ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1440., 800.),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            ber::show(h, ui, &ws, false, &mut None);
                        });
                    },
                )
            };
            let out = run_frame(&mut h, vec![]);
            let pos = find_label(&out, "Add dataset");
            for ev in click_events(pos) {
                run_frame(&mut h, vec![ev]);
            }
            let until = Instant::now() + Duration::from_secs(60);
            while h.app.datasets.is_empty() {
                run_frame(&mut h, vec![]);
                assert_eq!(h.app.error, None);
                assert!(
                    Instant::now() < until,
                    "BER simulation={simulation} timed out: {}",
                    h.status().0
                );
                std::thread::sleep(Duration::from_millis(2));
            }
            assert!(!h.app.running);
            assert_ne!(h.status().1, toolkits::AppStatus::Error, "{}", h.status().0);
        }

        // 2. Waveform Generator via real click through wave::show + OFDM column-vector check
        let ctx = egui::Context::default();
        let mut h = Host::new(WaveformGenerator::default());
        h.app.spec.ofdm = true;
        h.app.spec.modulation = Modulation::Qam;
        h.app.spec.order = 16;
        h.app.spec.symbols = 4;
        h.app.spec.sps = 1;
        h.app.spec.nulls = "[1 33]".into();
        h.app.spec.pilots = "[12 54]".into();
        let run_frame = |h: &mut Host<WaveformGenerator>, events: Vec<egui::Event>| {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1440., 1600.),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        wave::show(h, ui, &ws, false, &mut None);
                    });
                },
            )
        };
        let out = run_frame(&mut h, vec![]);
        let pos = find_label(&out, "Generate waveform");
        for ev in click_events(pos) {
            run_frame(&mut h, vec![ev]);
        }
        let until = Instant::now() + Duration::from_secs(60);
        while h.app.current.is_none() {
            run_frame(&mut h, vec![]);
            assert_eq!(h.app.error, None);
            assert!(
                Instant::now() < until,
                "Waveform generation timed out: {}",
                h.status().0
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(!h.app.running);
        assert_ne!(h.status().1, toolkits::AppStatus::Error, "{}", h.status().0);
        let script = h.app.script(&ws).unwrap();
        assert!(
            script.contains("[1;33]"),
            "expected column vector [1;33] in:\n{script}"
        );
        assert!(
            script.contains("[12;54]"),
            "expected column vector [12;54] in:\n{script}"
        );
    }

    #[test]
    fn qpsk_constellation_has_four_visible_markers() {
        fn markers(shape: &egui::Shape, points: &mut Vec<egui::Pos2>, accent: egui::Color32) {
            match shape {
                egui::Shape::Vec(v) => v.iter().for_each(|s| markers(s, points, accent)),
                egui::Shape::Circle(c) if c.fill == accent => {
                    assert!(2. * c.radius >= 4.);
                    if !points.iter().any(|p| p.distance(c.center) < 1.) {
                        points.push(c.center);
                    }
                }
                _ => (),
            }
        }
        let ctx = egui::Context::default();
        let mut wave = Host::new(WaveformGenerator::default());
        run_wave(&mut wave, &ctx, &Snapshot::default());
        wave.app.view = 2;
        let output =
            crate::headless::run(&ctx, egui::RawInput::default(), |ui| wave.app.main_ui(ui));
        let mut points = Vec::new();
        for s in &output.shapes {
            markers(&s.shape, &mut points, tokens::palette(&ctx).accent);
        }
        assert_eq!(points.len(), 4);
    }
}
