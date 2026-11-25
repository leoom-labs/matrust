// Signal Multiresolution Analyzer: PyWavelets / PyEMD / NumPy fixtures
// (tool/refs/multires_analyzer/reference.py) and the real app host.
mod multires_analyzer_tests {
    use super::*;
    use apps::multires_analyzer::{BUCKETS, Method, MultiresAnalyzer, decimate};
    use apps::{App, AppKind, Host, Snapshot};
    use toolkits::Tool;

    fn reference() -> serde_json::Value {
        serde_json::from_str(include_str!(
            "../../tests/fixtures/apps/multires_analyzer/reference.json"
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
    fn engine_with(code: &str) -> matrust::Engine {
        let mut engine = matrust::Engine::default();
        engine.execute(code).unwrap();
        engine
    }
    fn workspace(code: &str) -> Snapshot {
        let mut ws = Snapshot::default();
        ws.sync(&engine_with(code).workspace());
        ws
    }
    fn compute(host: &mut Host<MultiresAnalyzer>, ws: &Snapshot) -> Result<apps::Outputs, String> {
        host.app.prepare(ws);
        let outputs = apps::run(&mut matrust::Engine::default(), host.request(ws)?)?;
        host.app.receive(&outputs)?;
        Ok(outputs)
    }
    fn values(a: &matrust::Array) -> Vec<f64> {
        match a {
            matrust::Array::Numeric(m) => m.data().to_vec(),
            a => panic!("{}", a.class()),
        }
    }
    fn close(actual: f64, expected: f64, tolerance: f64, what: &str) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{what}: {actual:.15} != {expected:.15} (tolerance {tolerance})"
        );
    }
    /// The MODWT fixture signal as `x` and a host on it.
    fn modwt_case(wavelet: &'static str) -> (Snapshot, Host<MultiresAnalyzer>, Vec<f64>) {
        let r = reference();
        let x = floats(&r["modwt_signal"]["x"]);
        let mut engine = matrust::Engine::default();
        let column = matrust::Matrix::from_column_vec(x.len(), 1, x.clone()).unwrap();
        engine
            .insert_array("x", matrust::Array::Numeric(column))
            .unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        let mut host = Host::new(MultiresAnalyzer::default());
        host.app.source = "x".into();
        host.app.fs = "64".into();
        host.app.wavelet = wavelet;
        host.app.level = "4".into();
        (ws, host, x)
    }

    #[test]
    fn multires_analyzer_modwt_components_sum_to_signal_and_match_pywavelets() {
        let r = reference();
        for wavelet in ["sym4", "db2"] {
            let (ws, mut host, x) = modwt_case(wavelet);
            compute(&mut host, &ws).unwrap_or_else(|e| panic!("{wavelet}: {e}"));
            let d = host.app.current.as_ref().unwrap();
            let case = &r[format!("modwt_{wavelet}")];
            assert_eq!(d.labels, ["D1", "D2", "D3", "D4", "S4"]);
            assert_eq!(d.components.dims().as_ref(), &[256, 5]);
            let c = values(&d.components);
            let expected = case["components"].as_array().unwrap();
            for (k, row) in expected.iter().enumerate() {
                for (t, v) in floats(row).iter().enumerate() {
                    close(c[k * 256 + t], *v, 1e-12, &format!("{wavelet} D{k}[{t}]"));
                }
            }
            // Components sum to the signal within 1e-10 (and the app says so).
            for (t, xt) in x.iter().enumerate() {
                let sum: f64 = (0..5).map(|k| c[k * 256 + t]).sum();
                close(sum, *xt, 1e-10, "sum of components");
            }
            assert!(d.error <= 1e-10, "{}", d.error);
            for (a, b) in values(&d.reconstruction).iter().zip(&x) {
                close(*a, *b, 1e-10, "reconstruction");
            }
            for (k, e) in floats(&case["energy_percent"]).iter().enumerate() {
                close(d.energy[k], *e, 1e-9, &format!("{wavelet} energy {k}"));
            }
            for (k, f) in floats(&case["centre_hz"]).iter().enumerate() {
                close(d.centre[k], *f, 1e-9, &format!("{wavelet} centre {k}"));
            }
            close(d.energy.iter().sum::<f64>(), 100., 1e-9, "energies");
        }
    }

    #[test]
    fn multires_analyzer_emd_two_separated_tones_give_two_leading_imfs() {
        let r = reference();
        let case = &r["emd_tones"];
        let ws = workspace("t = (0:999)'/1000; y = sin(2*pi*100*t) + sin(2*pi*10*t);");
        let mut host = Host::new(MultiresAnalyzer::default());
        host.app.source = "y".into();
        host.app.fs = "1000".into();
        host.app.set_method(Method::Emd);
        compute(&mut host, &ws).unwrap();
        let d = host.app.current.as_ref().unwrap();
        assert!(d.labels.len() >= 3, "{:?}", d.labels);
        assert_eq!(d.labels.last().unwrap(), "Residual");
        let pyemd = floats(&case["centre_hz"]);
        for (k, tone) in floats(&case["tones_hz"]).iter().enumerate() {
            // Within 3 % of the tone and of PyEMD's IMF (sifting details differ).
            close(
                d.centre[k],
                *tone,
                0.03 * tone,
                &format!("IMF {} vs tone", k + 1),
            );
            close(
                d.centre[k],
                pyemd[k],
                0.03 * tone,
                &format!("IMF {} vs PyEMD", k + 1),
            );
        }
        // The two tones carry nearly all the energy.
        assert!(d.energy[0] + d.energy[1] > 90., "{:?}", d.energy);
        assert!(d.error <= 1e-10, "{}", d.error);
    }

    #[test]
    fn multires_analyzer_include_exclude_sums_the_chosen_columns() {
        let (ws, mut host, x) = modwt_case("sym4");
        compute(&mut host, &ws).unwrap();
        let all = host.app.current.as_ref().unwrap().error;
        host.app.set_included(1, false);
        host.app.set_included(3, false);
        assert!(!host.app.included(1) && host.app.included(2));
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.contains("setdiff(1:size(components, 2), [2 4])"),
            "{script}"
        );
        compute(&mut host, &ws).unwrap();
        let d = host.app.current.as_ref().unwrap();
        let c = values(&d.components);
        let rec = values(&d.reconstruction);
        for t in 0..256 {
            let expected = c[t] + c[2 * 256 + t] + c[4 * 256 + t];
            close(rec[t], expected, 1e-13, "partial sum");
        }
        assert!(rec.iter().zip(&x).any(|(a, b)| (a - b).abs() > 1e-3));
        let rows = host.app.results();
        let row = |label: &str| rows.iter().find(|r| r.0 == label).unwrap().1.clone();
        assert_eq!(row("Components"), "3 of 5 included");
        assert_eq!(
            row("Reconstruction error (max abs)"),
            "include all components"
        );
        // Out-of-range indices are ignored; nothing included gives zeros.
        for k in 0..5 {
            host.app.set_included(k, false);
        }
        host.app.set_included(9, false);
        compute(&mut host, &ws).unwrap();
        let d = host.app.current.as_ref().unwrap();
        assert!(values(&d.reconstruction).iter().all(|v| *v == 0.));
        assert_eq!(d.reconstruction.dims().as_ref(), &[256, 1]);
        for k in 0..10 {
            host.app.set_included(k, true);
        }
        compute(&mut host, &ws).unwrap();
        assert_eq!(host.app.current.as_ref().unwrap().error, all);
        // A new method starts with every component included.
        host.app.set_included(0, false);
        host.app.set_method(Method::Ewt);
        assert!(host.app.excluded.is_empty());
    }

    #[test]
    fn multires_analyzer_vmd_and_ewt_components_rebuild_the_signal() {
        let ws = workspace(
            "t = (0:511)'/512; x = cos(2*pi*8*t) + 0.5*cos(2*pi*60*t) + 0.25*cos(2*pi*150*t);",
        );
        for (method, labels) in [(Method::Vmd, "Residual"), (Method::Ewt, "Mode 1")] {
            let mut host = Host::new(MultiresAnalyzer::default());
            host.app.source = "x".into();
            host.app.fs = "512".into();
            host.app.set_method(method);
            host.app.modes = "3".into();
            host.app.max_peaks = "3".into();
            compute(&mut host, &ws).unwrap_or_else(|e| panic!("{method:?}: {e}"));
            let d = host.app.current.as_ref().unwrap();
            assert!(d.labels.iter().any(|l| l == labels), "{:?}", d.labels);
            assert!(d.error < 1e-8, "{method:?} error {}", d.error);
            // Each tone lands in its own component (centres sorted either way).
            let mut centres = d.centre.clone();
            if method == Method::Vmd {
                centres.pop(); // residual
            }
            centres.sort_by(f64::total_cmp);
            for (c, tone) in centres.iter().zip([8., 60., 150.]) {
                close(*c, tone, 0.1 * tone, &format!("{method:?} centre"));
            }
        }
    }

    #[test]
    fn multires_analyzer_generated_script_in_fresh_engine_gives_exported_arrays_exactly() {
        for method in Method::ALL {
            let code = "t = (0:255)'/256; x = sin(2*pi*5*t) + 0.4*sin(2*pi*40*t) + 0.1*t;";
            let ws = workspace(code);
            let mut host = Host::new(MultiresAnalyzer::default());
            host.app.source = "x".into();
            host.app.fs = "256".into();
            host.app.set_method(method);
            host.app.max_imfs = "4".into();
            host.app.modes = "3".into();
            host.app.max_peaks = "3".into();
            host.app.set_included(1, false);
            host.app.components_name = "parts".into();
            host.app.reconstruction_name = "rebuilt".into();
            compute(&mut host, &ws).unwrap_or_else(|e| panic!("{method:?}: {e}"));
            let d = host.app.current.clone().unwrap();
            let saved = match host.generate(&ws, 42).unwrap() {
                toolkits::Action::SaveAppScript(AppKind::MultiresAnalyzer, text) => text,
                _ => panic!("wrong save action"),
            };
            assert!(saved.ends_with(&host.app.script(&ws).unwrap()));
            let mut fresh = engine_with(code);
            fresh.execute(&saved).unwrap();
            assert_eq!(
                values(&fresh.array("parts").unwrap()),
                values(&d.components)
            );
            assert_eq!(
                fresh.array("parts").unwrap().dims(),
                d.components.dims(),
                "{method:?}"
            );
            assert_eq!(
                values(&fresh.array("rebuilt").unwrap()),
                values(&d.reconstruction)
            );
            // No temporaries are left behind.
            let names: Vec<String> = fresh.workspace().keys().cloned().collect();
            assert_eq!(names, ["parts", "rebuilt", "t", "x"], "{method:?}");
            // Export: the same arrays, through the all-or-nothing wrapper.
            let export = match host.export(&ws).unwrap() {
                toolkits::Action::ExportScript(s) => s,
                _ => panic!("wrong export"),
            };
            let mut exported = engine_with(code);
            exported.execute(&export).unwrap();
            assert_eq!(
                values(&exported.array("parts").unwrap()),
                values(&d.components)
            );
            assert_eq!(
                values(&exported.array("rebuilt").unwrap()),
                values(&d.reconstruction)
            );
        }
    }

    #[test]
    fn multires_analyzer_export_names_fields_and_last_good_result() {
        let ws = workspace("x = sin(1:64)'; y = 1:10; m = magic(4); z = [1 NaN 2]; c = [1i 2];");
        let mut host = Host::new(MultiresAnalyzer::default());
        // An empty source picks the first usable vector.
        compute(&mut host, &ws).unwrap();
        assert_eq!(host.app.source, "x");
        let previous = values(&host.app.current.as_ref().unwrap().components);
        for name in [
            "mrexp_old",
            "mrexp_had",
            "matrust_export_error",
            "mrexp_complete",
            "1bad",
            "for",
            "sum",
            "setdiff",
            "a);b=1;%",
            "",
        ] {
            host.app.components_name = name.into();
            assert!(host.app.script(&ws).is_err(), "{name}");
        }
        host.app.components_name = "reconstruction".into();
        assert!(host.app.script(&ws).unwrap_err().contains("duplicate"));
        host.app.components_name = "x".into();
        assert!(host.app.script(&ws).is_ok());
        host.app.components_name = "components".into();
        for (field, bad) in [
            ("fs", "0"),
            ("fs", "-1"),
            ("fs", "Inf"),
            ("fs", "2*32"),
            ("fs", "64); error('x'); %"),
            ("level", "0"),
            ("level", "7"),
            ("level", "2.5"),
            ("level", "4\nerror('boom')"),
        ] {
            match field {
                "fs" => host.app.fs = bad.into(),
                _ => host.app.level = bad.into(),
            }
            assert!(host.request(&ws).is_err(), "{field} = {bad:?}");
            host.app.fs = "1".into();
            host.app.level = "4".into();
        }
        for method in [Method::Emd, Method::Vmd, Method::Ewt] {
            host.app.set_method(method);
            for bad in ["0", "100", "1.5", "NaN", "3); clear all; %"] {
                match method {
                    Method::Emd => host.app.max_imfs = bad.into(),
                    Method::Vmd => host.app.modes = bad.into(),
                    _ => host.app.max_peaks = bad.into(),
                }
                assert!(host.request(&ws).is_err(), "{method:?} {bad}");
            }
        }
        host.app.set_method(Method::Emd);
        host.app.max_imfs = "5".into();
        for bad in ["0", "2", "-0.1", "x"] {
            host.app.sift_tolerance = bad.into();
            assert!(host.request(&ws).is_err(), "sift {bad}");
        }
        host.app.set_method(Method::Vmd);
        host.app.modes = "3".into();
        host.app.penalty = "0".into();
        assert!(host.request(&ws).is_err());
        host.app.set_method(Method::Modwt);
        host.app.level = "2".into();
        for source in ["y", "m", "z", "c", "missing", "x);error('bad');%"] {
            host.app.source = source.into();
            let ok = source == "y";
            assert_eq!(host.request(&ws).is_ok(), ok, "{source}");
        }
        assert_eq!(
            values(&host.app.current.as_ref().unwrap().components),
            previous
        );
        // A field error reaches the status line; the view keeps its result.
        host.app.source = "x".into();
        host.app.level = "9".into();
        host.refresh(&egui::Context::default(), &ws);
        let (status, kind) = host.status();
        assert_eq!(kind, toolkits::AppStatus::Error);
        assert!(status.contains("Level"), "{status}");
        assert_eq!(
            values(&host.app.current.as_ref().unwrap().components),
            previous
        );
        // Export rolls both names back when the script fails at run time.
        host.app.level = "4".into();
        compute(&mut host, &ws).unwrap();
        let code = match host.export(&ws).unwrap() {
            toolkits::Action::ExportScript(s) => s,
            _ => unreachable!(),
        };
        let mut engine = engine_with("x = [1 NaN 2 3]'; components = 7;");
        assert!(engine.execute(&code).is_err());
        assert_eq!(values(&engine.array("components").unwrap()), [7.]);
        assert!(engine.array("reconstruction").is_none());
        // Stop holds the run until an input changes or Run.
        host.app.stop();
        assert!(host.request(&ws).err().unwrap().contains("stopped"));
        host.app.run();
        assert!(host.request(&ws).is_ok());
        host.app.stop();
        host.app.level = "3".into();
        host.app.prepare(&ws);
        assert!(host.request(&ws).is_ok());
    }

    #[test]
    fn multires_analyzer_decimation_keeps_extremes() {
        let y: Vec<f64> = (0..100_000).map(|k| ((k as f64) * 0.01).sin()).collect();
        let line = decimate(&y, 0.5);
        assert!(line.len() <= 2 * BUCKETS);
        let max = line.iter().map(|p| p[1]).fold(f64::MIN, f64::max);
        let min = line.iter().map(|p| p[1]).fold(f64::MAX, f64::min);
        assert_eq!(max, y.iter().copied().fold(f64::MIN, f64::max));
        assert_eq!(min, y.iter().copied().fold(f64::MAX, f64::min));
        assert!(line.windows(2).all(|w| w[0][0] < w[1][0]));
        assert_eq!(decimate(&[1., 2.], 0.5), [[0., 1.], [0.5, 2.]]);
    }

    #[test]
    fn multires_analyzer_tool_tab_headless_empty_32_and_1e5_samples() {
        let kind = AppKind::MultiresAnalyzer;
        let tab = kind.tab();
        assert_eq!(AppKind::from_tab(tab), Some(kind));
        assert!(kind.is_built() && AppKind::ALL.contains(&kind) && !kind.detail().is_empty());
        assert_eq!(
            serde_json::to_string(&tab).unwrap(),
            "\"multires_analyzer\""
        );
        let decoded: matrust::session::ToolTab =
            serde_json::from_str("\"multires_analyzer\"").unwrap();
        assert_eq!(decoded, tab);
        for samples in [0usize, 32, 100_000] {
            let code = if samples == 0 {
                String::new()
            } else {
                format!("x = sin((1:{samples})'/7) + 0.1*cos((1:{samples})'/2);")
            };
            let ws = workspace(&code);
            for (width, spectra) in [(820., false), (1600., true)] {
                let ctx = egui::Context::default();
                let mut desktop = Workspace::with_context(ctx.clone(), true);
                desktop.toolkits.open_tool(Tool::App(kind));
                let host = &mut desktop.toolkits.apps.mra;
                host.app.spectra = spectra;
                if samples > 0 {
                    let start = Instant::now();
                    compute(host, &ws).unwrap();
                    eprintln!(
                        "multires_analyzer modwt sym4 level 4 N={samples} compute={:?}",
                        start.elapsed()
                    );
                    let d = host.app.current.as_ref().unwrap();
                    assert_eq!(d.samples, samples);
                    assert!(d.signal_line.len() <= (2 * BUCKETS).max(32));
                    assert!(d.error < 1e-9);
                }
                let output = frame_at(&mut desktop, &ctx, egui::vec2(width, 950.), vec![]);
                assert!(!output.shapes.is_empty());
                assert_eq!(desktop.toolkits.selected_tool(), Some(Tool::App(kind)));
                desktop.close_tool_tab(Tool::App(kind));
            }
        }
    }

    #[test]
    fn review_b7_multires_vmd_script_replays_preview_seed() {
        // Workspace generator is seed 1. The preview engine starts at seed 0
        // and the script sets that seed, so Export matches the preview.
        let code = "rng(1); x = [1 -1]';";
        let ws = workspace(code);
        let mut host = Host::new(MultiresAnalyzer::default());
        host.app.source = "x".into();
        host.app.set_method(Method::Vmd);
        host.app.modes = "5".into();
        host.app.penalty = "1000".into();
        let run = |host: &mut Host<MultiresAnalyzer>| {
            compute(host, &ws).unwrap();
            let d = host.app.current.clone().unwrap();
            let script = host.app.script(&ws).unwrap();
            assert!(script.contains("rng(0,'twister');\n"), "{script}");
            assert!(
                script.contains(
                    "[components, mra_residual] = vmd(x(:), 'NumIMFs', 5, 'PenaltyFactor', 1000);"
                ),
                "{script}"
            );
            let mut replay = engine_with(code);
            replay.execute(&script).unwrap();
            assert_eq!(values(&replay.array("components").unwrap()), values(&d.components));
            assert_eq!(
                replay.array("components").unwrap().dims(),
                d.components.dims()
            );
            assert_eq!(
                values(&replay.array("reconstruction").unwrap()),
                values(&d.reconstruction)
            );
            let export = match host.export(&ws).unwrap() {
                toolkits::Action::ExportScript(s) => s,
                _ => panic!("wrong export"),
            };
            assert!(export.contains("rng(0,'twister')"), "{export}");
            let mut exported = engine_with(code);
            exported.execute(&export).unwrap();
            assert_eq!(
                values(&exported.array("components").unwrap()),
                values(&d.components)
            );
            (d, script)
        };
        let (full, _) = run(&mut host);
        // A direct vmd after rng(1) is a different decomposition.
        let mut bare = engine_with(code);
        bare.execute("[u, r] = vmd(x(:), 'NumIMFs', 5, 'PenaltyFactor', 1000); u = [u r];")
            .unwrap();
        assert_ne!(values(&bare.array("u").unwrap()), values(&full.components));
        // Excluding a component still replays the preview.
        host.app.set_included(0, false);
        let (part, script) = run(&mut host);
        assert!(script.contains("setdiff"), "{script}");
        assert_ne!(values(&part.reconstruction), values(&full.reconstruction));
        // EMD does not touch the generator.
        host.app.set_method(Method::Emd);
        let emd = host.app.script(&ws).unwrap();
        assert!(!emd.contains("rng("), "{emd}");
    }
}
