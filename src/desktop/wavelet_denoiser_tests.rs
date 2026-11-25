// PyWavelets fixtures and integration through the real app host.
mod wavelet_denoiser_tests {
    use super::*;
    use apps::wavelet_denoiser::{
        Family, MAX_VERSIONS, Method, Noise, PLOT_BUCKETS, Rule, WaveletDenoiser,
    };
    use apps::{App, AppKind, Host, Snapshot};
    use toolkits::Tool;

    fn workspace(code: &str) -> Snapshot {
        let mut engine = matrust::Engine::default();
        engine.execute(code).unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        ws
    }
    fn with_signal(name: &str, values: &[f64]) -> Snapshot {
        let mut engine = matrust::Engine::default();
        let m = matrust::Matrix::from_column_vec(values.len(), 1, values.to_vec()).unwrap();
        engine
            .insert_array(name, matrust::Array::Numeric(m))
            .unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        ws
    }
    fn compute(host: &mut Host<WaveletDenoiser>, ws: &Snapshot) -> Result<apps::Outputs, String> {
        host.app.prepare(ws);
        let outputs = apps::run(&mut matrust::Engine::default(), host.request(ws)?)?;
        host.app.receive(&outputs)?;
        Ok(outputs)
    }
    fn data(a: &matrust::Array) -> Vec<f64> {
        match a {
            matrust::Array::Numeric(m) => m.data().to_vec(),
            a => panic!("{}", a.class()),
        }
    }
    fn values(engine: &matrust::Engine, name: &str) -> Vec<f64> {
        data(&engine.array(name).unwrap())
    }
    fn reference() -> serde_json::Value {
        serde_json::from_str(include_str!(
            "../../tests/fixtures/apps/wavelet_denoiser/reference.json"
        ))
        .unwrap()
    }
    fn floats(v: &serde_json::Value) -> Vec<f64> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_f64().unwrap())
            .collect()
    }
    fn configure(app: &mut WaveletDenoiser, wavelet: &str, method: Method, noise: Noise) {
        let s = app.settings();
        let (family, order) = if let Some(o) = wavelet.strip_prefix("db") {
            (Family::Daubechies, o)
        } else if let Some(o) = wavelet.strip_prefix("sym") {
            (Family::Symlet, o)
        } else if let Some(o) = wavelet.strip_prefix("coif") {
            (Family::Coiflet, o)
        } else if let Some(o) = wavelet.strip_prefix("bior") {
            (Family::Biorthogonal, o)
        } else {
            (Family::Haar, "")
        };
        s.set_family(family);
        s.order = order.into();
        s.set_method(method);
        s.noise = noise;
    }

    #[test]
    fn wavelet_denoiser_pywavelets_db4_universal_and_sym8_sure() {
        let r = reference();
        let signal = floats(&r["signal"]);
        let ws = with_signal("noisy", &signal);
        for case in r["cases"].as_array().unwrap() {
            let wavelet = case["wavelet"].as_str().unwrap();
            let method = match case["method"].as_str().unwrap() {
                "UniversalThreshold" => Method::Universal,
                "SURE" => Method::Sure,
                m => panic!("{m}"),
            };
            let noise = if case["level_dependent"].as_bool().unwrap() {
                Noise::LevelDependent
            } else {
                Noise::LevelIndependent
            };
            let mut host = Host::new(WaveletDenoiser::default());
            host.app.source = "noisy".into();
            configure(&mut host.app, wavelet, method, noise);
            host.app.settings().rule = Rule::Soft;
            let outputs = compute(&mut host, &ws).unwrap_or_else(|e| panic!("{wavelet}: {e}"));
            let y = data(&outputs["wsd_probe_y"]);
            let expected = floats(&case["denoised"]);
            assert_eq!(y.len(), expected.len());
            for (a, b) in y.iter().zip(&expected) {
                assert!((a - b).abs() <= 1e-10, "{wavelet}: {a} != {b}");
            }
            let thresholds = data(&outputs["wsd_probe_thresholds"]);
            let expected = floats(&case["thresholds"]);
            assert_eq!(thresholds.len() as u64, case["level"].as_u64().unwrap());
            for (a, b) in thresholds.iter().zip(&expected) {
                assert!(
                    (a - b).abs() <= 1e-12 * b.abs().max(1.),
                    "{wavelet}: {a} != {b}"
                );
            }
            // The drawn thresholds are the ones wdenoise applied: soft
            // thresholding zeroes |d| <= t and shrinks the rest by t.
            let view = host.app.current.as_ref().unwrap();
            assert_eq!(view.level, thresholds.len());
            for level in &view.levels {
                for (o, d) in level.original.iter().zip(&level.denoised) {
                    let shrunk = (o[1].abs() - level.threshold).max(0.).copysign(o[1]);
                    assert!((d[1] - shrunk).abs() <= 1e-12 * o[1].abs().max(1.));
                }
            }
            let stats = &view.stats[0].1;
            let removed: Vec<f64> = signal.iter().zip(&y).map(|(a, b)| a - b).collect();
            let rms = (removed.iter().map(|v| v * v).sum::<f64>() / y.len() as f64).sqrt();
            let snr = 10.
                * (signal.iter().map(|v| v * v).sum::<f64>()
                    / removed.iter().map(|v| v * v).sum::<f64>())
                .log10();
            assert!((stats[0] - snr).abs() < 1e-9 && (stats[1] - rms).abs() < 1e-12);
        }
    }

    #[test]
    fn wavelet_denoiser_zero_threshold_reconstructs_perfectly() {
        // Minimax with N <= 32 has threshold 0 (Donoho & Johnstone 1994).
        let x: Vec<f64> = (0..32)
            .map(|k| (k as f64 * 0.7).sin() + 0.1 * (k * k % 7) as f64)
            .collect();
        let ws = with_signal("x", &x);
        for wavelet in ["haar", "db4", "sym8", "coif2", "bior3.5"] {
            let mut host = Host::new(WaveletDenoiser::default());
            host.app.source = "x".into();
            configure(
                &mut host.app,
                wavelet,
                Method::Minimax,
                Noise::LevelIndependent,
            );
            let outputs = compute(&mut host, &ws).unwrap_or_else(|e| panic!("{wavelet}: {e}"));
            let y = data(&outputs["wsd_probe_y"]);
            for (a, b) in y.iter().zip(&x) {
                assert!((a - b).abs() <= 1e-10, "{wavelet}: {a} != {b}");
            }
            assert!(
                data(&outputs["wsd_probe_thresholds"])
                    .iter()
                    .all(|t| *t == 0.)
            );
        }
    }

    #[test]
    fn wavelet_denoiser_generated_script_export_and_names() {
        let r = reference();
        let signal = floats(&r["signal"]);
        // The output may replace the source: the script stages it first.
        let ws = with_signal("wsd_signal", &signal);
        let mut host = Host::new(WaveletDenoiser::default());
        host.app.source = "wsd_signal".into();
        host.app.samples = false;
        host.app.rate = "500".into();
        configure(
            &mut host.app,
            "db4",
            Method::Universal,
            Noise::LevelIndependent,
        );
        host.app.settings().level = "5".into();
        host.app.signal_name = "wsd_signal".into();
        host.app.settings_name = "struct".into();
        let outputs = compute(&mut host, &ws).unwrap();
        let shown = data(&outputs["wsd_probe_y"]);
        let script = host.app.script(&ws).unwrap();
        assert!(script.contains(
            "wdenoise(wsd_signal1, 5, 'Wavelet', 'db4', 'DenoisingMethod', 'UniversalThreshold', 'ThresholdRule', 'Soft', 'NoiseEstimate', 'LevelIndependent')"
        ), "{script}");
        let saved = match host.generate(&ws, 42).unwrap() {
            toolkits::Action::SaveAppScript(AppKind::WaveletDenoiser, text) => text,
            _ => panic!("wrong save action"),
        };
        assert!(saved.ends_with(&script));
        let mut fresh = matrust::Engine::default();
        for (name, a) in ws.variables() {
            fresh.insert_array(name, a.clone()).unwrap();
        }
        fresh.execute(&saved).unwrap();
        assert_eq!(values(&fresh, "wsd_signal"), shown);
        assert_eq!(
            fresh.array("wsd_signal").unwrap().dims().as_ref(),
            &[1000, 1]
        );
        fresh
            .execute(
                "check = [struct.Level, struct.SampleRate, double(strcmp(struct.Wavelet, 'db4')), \
                 double(strcmp(struct.DenoisingMethod, 'UniversalThreshold')), \
                 double(strcmp(struct.ThresholdRule, 'Soft')), \
                 double(strcmp(struct.NoiseEstimate, 'LevelIndependent'))];",
            )
            .unwrap();
        assert_eq!(values(&fresh, "check"), [5., 500., 1., 1., 1., 1.]);
        let export = match host.export(&ws).unwrap() {
            toolkits::Action::ExportScript(s) => s,
            _ => panic!("wrong export"),
        };
        let mut exported = matrust::Engine::default();
        for (name, a) in ws.variables() {
            exported.insert_array(name, a.clone()).unwrap();
        }
        exported.execute(&export).unwrap();
        assert_eq!(values(&exported, "wsd_signal"), shown);
        assert_eq!(host.app.exported(&ws), ["wsd_signal", "struct"]);

        // Checked names: reserved, invalid, duplicate or injected.
        for bad in [
            "mrexp_old",
            "mrexp_had",
            "matrust_export_error",
            "mrexp_complete",
            "1bad",
            "for",
            "x);bad=1;%",
            "",
        ] {
            host.app.signal_name = bad.into();
            assert!(host.app.script(&ws).is_err(), "{bad}");
            host.app.signal_name = "den".into();
            host.app.settings_name = bad.into();
            assert!(host.app.script(&ws).is_err(), "{bad}");
            host.app.settings_name = "opts".into();
        }
        host.app.settings_name = "den".into();
        assert!(host.app.script(&ws).unwrap_err().contains("duplicate"));
        host.app.settings_name = "opts".into();

        // A failed export restores both names (all or nothing).
        let export = match host.export(&ws).unwrap() {
            toolkits::Action::ExportScript(s) => s,
            _ => unreachable!(),
        };
        let mut engine = matrust::Engine::default();
        engine
            .execute("wsd_signal = [1 NaN 3 4]; den = 7; opts = 8;")
            .unwrap();
        assert!(engine.execute(&export).is_err());
        assert_eq!(values(&engine, "den"), [7.]);
        assert_eq!(values(&engine, "opts"), [8.]);
    }

    #[test]
    fn wavelet_denoiser_parser_validation_and_last_good_result() {
        let r = reference();
        let ws = with_signal("x", &floats(&r["signal"]));
        let mut host = Host::new(WaveletDenoiser::default());
        host.app.source = "x".into();
        compute(&mut host, &ws).unwrap();
        let previous = host.app.current.as_ref().unwrap().denoised.clone();
        for bad in [
            "0",
            "10",
            "-1",
            "1.5",
            "2^2",
            "Inf",
            "3); error('x'); %",
            "3\nerror('x')",
        ] {
            host.app.settings().level = bad.into();
            assert!(host.request(&ws).is_err(), "{bad}");
            assert_eq!(host.app.current.as_ref().unwrap().denoised, previous);
        }
        host.app.settings().level = "9".into();
        host.request(&ws).unwrap();
        host.app.settings().level = String::new();
        host.app.samples = false;
        for bad in ["0", "-5", "abc", "NaN", "1e400", "500); error('x'); %"] {
            host.app.rate = bad.into();
            assert!(host.request(&ws).is_err(), "{bad}");
        }
        host.app.rate = "500".into();
        // A rule that does not belong to the method is refused.
        host.app.settings().method = Method::Sure;
        host.app.settings().rule = Rule::Median;
        assert!(host.request(&ws).is_err());
        host.app.settings().set_method(Method::Sure);
        assert_eq!(host.app.settings().rule, Rule::Soft);
        for source in ["missing", "z", "c", "x);error('bad');%"] {
            host.app.source = source.into();
            let ws = workspace("x = 1:8; z = [1 2; 3 4]; c = [1+2i 3];");
            assert!(host.request(&ws).is_err(), "{source}");
        }
        // The status line shows the error and the view keeps the last result.
        host.app.source = "x".into();
        host.app.settings().level = "0".into();
        let ctx = egui::Context::default();
        host.refresh(&ctx, &ws);
        assert_eq!(host.status().1, toolkits::AppStatus::Error);
        assert!(host.status().0.contains("Level"));
        assert_eq!(host.app.current.as_ref().unwrap().denoised, previous);
        // Stop holds the run until an input changes.
        host.app.settings().level = String::new();
        host.app.halt();
        host.app.prepare(&ws);
        assert!(host.request(&ws).err().unwrap().contains("stopped"));
        host.app.resume();
        host.request(&ws).unwrap();
    }

    #[test]
    fn wavelet_denoiser_versions_timetable_and_bayes() {
        let ws = workspace(
            "n = (0:511)'; tt = timetable(seconds(n/200), sin(2*pi*3*n/200) + 0.2*sin(n.^2)); clear n",
        );
        let mut host = Host::new(WaveletDenoiser::default());
        host.app.choose("tt", &ws);
        assert!(!host.app.samples);
        assert_eq!(host.app.rate, "200");
        // Default Bayes median: the drawn threshold is the largest zeroed |d|.
        let outputs = compute(&mut host, &ws).unwrap();
        assert_eq!(data(&outputs["wsd_probe_x"]).len(), 512);
        let view = host.app.current.as_ref().unwrap();
        assert!(view.hz && (view.original[1][0] - 1. / 200.).abs() < 1e-15);
        for level in &view.levels {
            for (o, d) in level.original.iter().zip(&level.denoised) {
                if d[1] == 0. {
                    assert!(o[1].abs() <= level.threshold);
                }
            }
        }
        for method in [Method::Sure, Method::Universal, Method::Minimax] {
            host.app.keep().unwrap();
            host.app.settings().set_method(method);
        }
        host.app.keep().unwrap();
        host.app.settings().set_method(Method::Bayes);
        host.app.settings().rule = Rule::Mean;
        host.app.settings().noise = Noise::LevelDependent;
        assert_eq!(host.app.versions.len(), MAX_VERSIONS);
        assert!(host.app.keep().is_err());
        compute(&mut host, &ws).unwrap();
        let view = host.app.current.as_ref().unwrap();
        assert_eq!(view.stats.len(), MAX_VERSIONS);
        assert!(
            view.stats
                .iter()
                .all(|(_, s)| s[0].is_finite() && s[1] > 0.)
        );
        // Each row's statistics match running that row selected.
        let all = view.stats.clone();
        host.app.selected = 1;
        compute(&mut host, &ws).unwrap();
        let view = host.app.current.as_ref().unwrap();
        assert_eq!(
            view.stats
                .iter()
                .find(|(id, _)| *id == host.app.versions[1].id),
            all.iter().find(|(id, _)| *id == host.app.versions[1].id)
        );
        assert!(host.app.script(&ws).unwrap().contains("'SURE'"));
        host.app.remove();
        assert_eq!(host.app.versions.len(), MAX_VERSIONS - 1);
        assert!(host.app.script(&ws).unwrap().contains("tt.Var1"));
    }

    #[test]
    fn wavelet_denoiser_tool_tab_headless_empty_16_and_1e6_samples() {
        let kind = AppKind::WaveletDenoiser;
        let tab = kind.tab();
        assert_eq!(AppKind::from_tab(tab), Some(kind));
        assert_eq!(serde_json::to_string(&tab).unwrap(), "\"wavelet_denoiser\"");
        let decoded: matrust::session::ToolTab =
            serde_json::from_str("\"wavelet_denoiser\"").unwrap();
        assert_eq!(decoded, tab);
        for count in [0usize, 16, 1_000_000] {
            let ws = if count == 0 {
                Snapshot::default()
            } else {
                let x: Vec<f64> = (0..count)
                    .map(|k| (k as f64 * 0.01).sin() + 0.1 * ((k * 7919) % 13) as f64)
                    .collect();
                with_signal("x", &x)
            };
            let ctx = egui::Context::default();
            let mut desktop = Workspace::with_context(ctx.clone(), true);
            desktop.toolkits.open_tool(Tool::App(kind));
            let host = &mut desktop.toolkits.apps.wavelet;
            if count > 0 {
                host.app.source = "x".into();
                let start = Instant::now();
                compute(host, &ws).unwrap();
                eprintln!(
                    "wavelet_denoiser N={count} levels={} compute={:?}",
                    host.app.current.as_ref().unwrap().level,
                    start.elapsed()
                );
                let view = host.app.current.as_ref().unwrap();
                assert!(view.original.len() <= 2 * PLOT_BUCKETS);
                assert!(
                    view.levels
                        .iter()
                        .all(|l| l.original.len() <= 2 * PLOT_BUCKETS)
                );
            }
            for width in [820., 1600.] {
                let start = Instant::now();
                let output = frame_at(&mut desktop, &ctx, egui::vec2(width, 950.), vec![]);
                assert!(!output.shapes.is_empty());
                eprintln!(
                    "wavelet_denoiser N={count} width={width} frame={:?}",
                    start.elapsed()
                );
            }
            assert_eq!(desktop.toolkits.selected_tool(), Some(Tool::App(kind)));
            desktop.close_tool_tab(Tool::App(kind));
        }
    }

    #[test]
    fn review_b7_wavelet_denoiser_script_uses_symmetric_mode() {
        // The workspace mode is zero-padding. Preview, Export and the script
        // still denoise with symmetric extension, then put zpd back.
        let mut engine = matrust::Engine::default();
        engine
            .execute("dwtmode('zpd','nodisp'); x = [1 2 -1 4 0 3 -2 5];")
            .unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        let mut host = Host::new(WaveletDenoiser::default());
        host.app.source = "x".into();
        configure(
            &mut host.app,
            "db2",
            Method::Universal,
            Noise::LevelIndependent,
        );
        host.app.settings().rule = Rule::Soft;
        host.app.settings().level = "1".into();
        let outputs = compute(&mut host, &ws).unwrap();
        let preview = data(&outputs["wsd_probe_y"]);
        let script = host.app.script(&ws).unwrap();
        let call = "[wsd_denoised, wsd_coefficients, wsd_original] = wdenoise(wsd_signal, 1, \
                    'Wavelet', 'db2', 'DenoisingMethod', 'UniversalThreshold', 'ThresholdRule', \
                    'Soft', 'NoiseEstimate', 'LevelIndependent');";
        assert!(
            script.contains("wsd_dwtmode = dwtmode('status','nodisp');\ndwtmode('sym','nodisp');\n"),
            "{script}"
        );
        assert!(script.contains(call), "{script}");
        assert!(
            script.contains("dwtmode(wsd_dwtmode,'nodisp');\nclear wsd_dwtmode\n"),
            "{script}"
        );
        let mut replay = matrust::Engine::default();
        replay
            .execute("dwtmode('zpd','nodisp'); x = [1 2 -1 4 0 3 -2 5];")
            .unwrap();
        replay.execute(&script).unwrap();
        assert_eq!(values(&replay, "xden"), preview);
        replay
            .execute("kept = double(strcmp(dwtmode('status','nodisp'), 'zpd'));")
            .unwrap();
        assert_eq!(values(&replay, "kept"), [1.]);
        let export = match host.export(&ws).unwrap() {
            toolkits::Action::ExportScript(s) => s,
            _ => panic!("wrong export"),
        };
        assert!(export.contains("dwtmode('sym','nodisp')"), "{export}");
        let mut exported = matrust::Engine::default();
        exported
            .execute("dwtmode('zpd','nodisp'); x = [1 2 -1 4 0 3 -2 5];")
            .unwrap();
        exported.execute(&export).unwrap();
        assert_eq!(values(&exported, "xden"), preview);
        exported
            .execute("kept = double(strcmp(dwtmode('status','nodisp'), 'zpd'));")
            .unwrap();
        assert_eq!(values(&exported, "kept"), [1.]);
        // The same call under the workspace mode is a different result.
        let mut bare = matrust::Engine::default();
        bare.execute(
            "dwtmode('zpd','nodisp'); x = [1 2 -1 4 0 3 -2 5]; \
             y = wdenoise(x, 1, 'Wavelet', 'db2', 'DenoisingMethod', 'UniversalThreshold', \
             'ThresholdRule', 'Soft', 'NoiseEstimate', 'LevelIndependent');",
        )
        .unwrap();
        assert_ne!(values(&bare, "y"), preview);
    }

    #[test]
    fn six_coefficient_captions_fit_separate_rows() {
        let ctx = egui::Context::default();
        let mut host = Host::new(WaveletDenoiser::default());
        host.app.source = "x".into();
        host.app.settings().level = "6".into();
        compute(&mut host, &workspace("x=sin((1:512)/7);")).unwrap();
        let output = crate::headless::run(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1008., 860.),
                )),
                ..Default::default()
            },
            |ui| host.app.main_ui(ui),
        );
        let captions: Vec<_> = clear_panes_texts(&output)
            .into_iter()
            .filter(|t| t.text.ends_with(" coefficients)"))
            .collect();
        assert_eq!(captions.len(), 6);
        for (k, t) in captions.iter().enumerate() {
            assert!(t.text.starts_with(&format!("d{} ", k + 1)));
            assert!(t.clip.contains_rect(t.glyphs));
            assert!(t.glyphs.width() > t.glyphs.height());
            if k > 0 {
                assert!(captions[k - 1].glyphs.bottom() < t.glyphs.top());
            }
        }
    }
}
