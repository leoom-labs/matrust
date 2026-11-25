// System Identification app: NumPy/SciPy fixtures (tool/refs/system_identification)
// and integration through the real app host.
mod system_identification_tests {
    use super::*;
    use apps::system_identification::{
        DISPLAY_POINTS, Detrend, Kind, MAX_MODELS, Source, SystemIdentification, View,
    };
    use apps::{App, AppKind, Host, Snapshot};
    use toolkits::Tool;

    fn reference() -> serde_json::Value {
        serde_json::from_str(include_str!(
            "../../tests/fixtures/apps/system_identification/reference.json"
        ))
        .unwrap()
    }
    fn numbers(value: &serde_json::Value) -> Vec<f64> {
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect()
    }
    /// Same memory guards as the desktop Runner.
    fn engine() -> matrust::Engine {
        matrust::Engine::with_limits(matrust::engine::Limits::local())
    }
    fn workspace(code: &str) -> Snapshot {
        let mut engine = engine();
        engine.execute(code).unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        ws
    }
    /// The fixture's input `u` and one case's output `y` as column vectors.
    fn fixture(case: &str) -> Snapshot {
        let r = reference();
        let mut engine = engine();
        for (name, values) in [("u", numbers(&r["u"])), ("y", numbers(&r[case]["y"]))] {
            let column = matrust::Matrix::from_column_vec(values.len(), 1, values).unwrap();
            engine.insert_array(name, column.into()).unwrap();
        }
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        ws
    }
    /// A host reading `u` and `y` at the fixture's sample time, no models.
    fn host() -> Host<SystemIdentification> {
        let mut host = Host::new(SystemIdentification::default());
        host.app.output = "y".into();
        host.app.input = "u".into();
        host.app.ts = "0.1".into();
        host.app.models.clear();
        host
    }
    fn compute(
        host: &mut Host<SystemIdentification>,
        ws: &Snapshot,
    ) -> Result<apps::Outputs, String> {
        host.app.prepare(ws);
        let outputs = apps::run(&mut engine(), host.request(ws)?)?;
        host.app.receive(&outputs)?;
        Ok(outputs)
    }
    /// An engine holding the workspace's variables, like the desktop's.
    fn desktop_engine(ws: &Snapshot) -> matrust::Engine {
        let mut engine = engine();
        for (name, array) in ws.variables() {
            engine.insert_array(name, array.clone()).unwrap();
        }
        engine
    }
    fn values(engine: &matrust::Engine, expression: &str) -> Vec<f64> {
        let mut e = engine.clone();
        e.execute(&format!("test_value = double({expression});"))
            .unwrap();
        match e.array("test_value").unwrap() {
            matrust::Array::Numeric(m) => m.data().to_vec(),
            a => panic!("{expression}: {}", a.class()),
        }
    }
    fn close(actual: &[f64], expected: &[f64], tolerance: f64, what: &str) {
        assert_eq!(actual.len(), expected.len(), "{what}: length");
        for (a, e) in actual.iter().zip(expected) {
            assert!(
                (a - e).abs() <= tolerance,
                "{what}: {a:.15} != {e:.15} (tolerance {tolerance})"
            );
        }
    }
    fn coefficient<'a>(host: &'a Host<SystemIdentification>, k: usize, label: &str) -> &'a [f64] {
        host.app.models[k]
            .result
            .as_ref()
            .unwrap()
            .coefficient(label)
            .unwrap()
    }
    fn export_code(host: &mut Host<SystemIdentification>, ws: &Snapshot) -> String {
        match host.export(ws).unwrap() {
            toolkits::Action::ExportScript(s) => s,
            _ => panic!("wrong export"),
        }
    }
    fn wait(host: &mut Host<SystemIdentification>, ctx: &egui::Context, ws: &Snapshot) {
        let start = Instant::now();
        loop {
            host.refresh(ctx, ws);
            if !host.running() {
                return;
            }
            assert!(start.elapsed() < Duration::from_secs(120), "run timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn system_identification_arx_recovers_known_arx221_coefficients() {
        // Noise-free data of A = [1 -1.5 0.7], B = [0 1 0.5]: least squares
        // on the estimation half returns the system itself.
        let r = reference();
        let ws = fixture("arx_free");
        let mut host = host();
        host.app.add(Kind::Arx, &ws).unwrap();
        assert_eq!(host.app.models[0].orders, "[2 2 1]");
        compute(&mut host, &ws).unwrap();
        close(
            coefficient(&host, 0, "A"),
            &numbers(&r["arx_free"]["a"]),
            1e-8,
            "A",
        );
        close(
            coefficient(&host, 0, "B"),
            &numbers(&r["arx_free"]["b"]),
            1e-8,
            "B",
        );
        close(coefficient(&host, 0, "A"), &[1., -1.5, 0.7], 1e-8, "true A");
        close(coefficient(&host, 0, "B"), &[0., 1., 0.5], 1e-8, "true B");
        for unit in ["C", "D", "F"] {
            assert_eq!(coefficient(&host, 0, unit), [1.]);
        }
        let result = host.app.models[0].result.as_ref().unwrap();
        assert!((result.fit - 100.).abs() < 1e-8, "{}", result.fit);
        assert!(result.loss < 1e-20 && result.fpe < 1e-20);
        let data = host.app.data.as_ref().unwrap();
        assert_eq!((data.samples, data.ts), (300, 0.1));
        assert_eq!((data.estimation, data.validation), ((1, 150), (151, 300)));

        // Noisy data: NumPy's closed-form least squares, loss and FPE.
        let ws = fixture("arx_noisy");
        compute(&mut host, &ws).unwrap();
        let expected = &r["arx_noisy"]["raw"];
        close(
            coefficient(&host, 0, "A"),
            &numbers(&expected["a"]),
            1e-9,
            "noisy A",
        );
        close(
            coefficient(&host, 0, "B"),
            &numbers(&expected["b"]),
            1e-9,
            "noisy B",
        );
        let result = host.app.models[0].result.as_ref().unwrap();
        close(
            &[result.loss],
            &[expected["loss"].as_f64().unwrap()],
            1e-12,
            "loss",
        );
        close(
            &[result.fpe],
            &[expected["fpe"].as_f64().unwrap()],
            1e-12,
            "FPE",
        );
        let rows = host.app.results();
        assert!(
            rows.iter()
                .any(|(l, v)| *l == "A" && v.starts_with("[1 -1.5"))
        );
        assert!(rows.iter().all(|(l, _)| *l != "C"));
    }

    #[test]
    fn system_identification_oe_and_tfest_noisy_first_order_within_reference_tolerance() {
        let r = reference();
        // y = 0.5 q^-1/(1 - 0.8 q^-1) u + e; SciPy least_squares on the
        // simulation error of the estimation half.
        let ws = fixture("oe_first_order");
        let mut host = host();
        host.app.add(Kind::Oe, &ws).unwrap();
        host.app.models[0].orders = "[1 1 1]".into();
        compute(&mut host, &ws).unwrap();
        let expected = &r["oe_first_order"];
        close(
            coefficient(&host, 0, "B"),
            &numbers(&expected["b"]),
            1e-6,
            "OE B",
        );
        close(
            coefficient(&host, 0, "F"),
            &numbers(&expected["f"]),
            1e-6,
            "OE F",
        );
        let truth = numbers(&expected["truth"]);
        assert!((coefficient(&host, 0, "B")[1] - truth[0]).abs() < 0.05);
        assert!((coefficient(&host, 0, "F")[1] - truth[1]).abs() < 0.05);
        let fit = host.app.models[0].result.as_ref().unwrap().fit;
        close(&[fit], &[expected["fit"].as_f64().unwrap()], 1e-5, "OE fit");

        // G(s) = 4/(s + 2) sampled with a zero-order hold at 0.1 s.
        let ws = fixture("tf_first_order");
        let mut host = self::host();
        host.app.add(Kind::TransferFunction, &ws).unwrap();
        host.app.models[0].poles = "1".into();
        host.app.models[0].zeros = "0".into();
        compute(&mut host, &ws).unwrap();
        let expected = &r["tf_first_order"];
        close(
            coefficient(&host, 0, "Numerator"),
            &numbers(&expected["numerator"]),
            1e-5,
            "numerator",
        );
        close(
            coefficient(&host, 0, "Denominator"),
            &numbers(&expected["denominator"]),
            1e-5,
            "denominator",
        );
        assert_eq!(coefficient(&host, 0, "IO delay"), [0.]);
        let truth = numbers(&expected["truth"]);
        assert!((coefficient(&host, 0, "Numerator")[0] - truth[0]).abs() < 0.3);
        assert!((coefficient(&host, 0, "Denominator")[1] - truth[1]).abs() < 0.2);
        let fit = host.app.models[0].result.as_ref().unwrap().fit;
        close(
            &[fit],
            &[expected["fit"].as_f64().unwrap()],
            1e-5,
            "tfest fit",
        );
    }

    #[test]
    fn system_identification_compare_fit_matches_reference_formula() {
        let r = reference();
        let ws = fixture("arx_noisy");
        let mut host = host();
        host.app.add(Kind::Arx, &ws).unwrap();
        compute(&mut host, &ws).unwrap();
        let result = host.app.models[0].result.as_ref().unwrap();
        // NumPy: 100*(1 - norm(y - yhat)/norm(y - mean(y))) on samples 151:300.
        let expected = r["arx_noisy"]["raw"]["fit"].as_f64().unwrap();
        close(&[result.fit], &[expected], 1e-8, "fit %");
        // The drawn curves give the same number through the same formula.
        let data = host.app.data.as_ref().unwrap();
        let y: Vec<f64> = data.measured.iter().map(|p| p[1]).collect();
        let yhat: Vec<f64> = result.output.iter().map(|p| p[1]).collect();
        assert_eq!((y.len(), yhat.len()), (150, 150));
        let mean = y.iter().sum::<f64>() / y.len() as f64;
        let norm = |v: Vec<f64>| v.iter().map(|x| x * x).sum::<f64>().sqrt();
        let error = norm(y.iter().zip(&yhat).map(|(a, b)| a - b).collect());
        let spread = norm(y.iter().map(|a| a - mean).collect());
        close(&[100. * (1. - error / spread)], &[expected], 1e-8, "curves");
        // Validation time starts at sample 151: (151 - 1) * 0.1 s.
        assert!((data.measured[0][0] - 15.).abs() < 1e-12);
        assert!((result.output[0][0] - 15.).abs() < 1e-12);
        // The script's own fit vector holds the same value.
        let mut fresh = desktop_engine(&ws);
        fresh.execute(&host.app.script(&ws).unwrap()).unwrap();
        assert_eq!(values(&fresh, "fit_percent"), [result.fit]);

        // An offset output with the means removed from each range.
        let ws = fixture("arx_offset");
        host.app.detrend = Detrend::Mean;
        compute(&mut host, &ws).unwrap();
        let expected = &r["arx_offset"]["detrended"];
        let result = host.app.models[0].result.as_ref().unwrap();
        close(
            coefficient(&host, 0, "A"),
            &numbers(&expected["a"]),
            1e-9,
            "detrended A",
        );
        close(
            coefficient(&host, 0, "B"),
            &numbers(&expected["b"]),
            1e-9,
            "detrended B",
        );
        close(
            &[result.fit],
            &[expected["fit"].as_f64().unwrap()],
            1e-8,
            "detrended fit",
        );
        close(
            &[result.fpe],
            &[expected["fpe"].as_f64().unwrap()],
            1e-12,
            "detrended FPE",
        );
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.contains("ze = detrend(ze);\nzv = detrend(zv);\n"),
            "{script}"
        );
    }

    #[test]
    fn system_identification_script_in_fresh_engine_gives_the_exported_coefficients_exactly() {
        let ws = fixture("arx_noisy");
        let mut host = host();
        for kind in Kind::ALL {
            host.app.add(kind, &ws).unwrap();
        }
        host.app.models[1].orders = "[2 2 1 1]".into();
        host.app.models[3].orders = "[2 1 1 2 1]".into();
        host.app.models[5].order = "1:4".into();
        // The process model stays in the app: only the probe estimates it.
        host.app.models[6].export = false;
        host.app.prepare(&ws);
        let key = host.request(&ws).unwrap().key;
        compute(&mut host, &ws).unwrap();
        // A finished run never changes the script, so it is not repeated.
        assert_eq!(host.request(&ws).unwrap().key, key);
        let names: Vec<String> = host.app.models.iter().map(|m| m.name.clone()).collect();
        assert_eq!(
            names,
            ["arx1", "armax1", "oe1", "bj1", "tf1", "ss1", "proc1"]
        );

        let script = host.app.script(&ws).unwrap();
        for line in [
            "data = iddata(y, u, 0.1);\n",
            "ze = data(1:150);\nzv = data(151:300);\n",
            "arx1 = arx(ze, [2 2 1]);\n",
            "armax1 = armax(ze, [2 2 1 1]);\n",
            "oe1 = oe(ze, [2 2 1]);\n",
            "bj1 = bj(ze, [2 1 1 2 1]);\n",
            "tf1 = tfest(ze, 2, 1);\n",
            "ss1 = n4sid(ze, 1:4);\nss1 = ssest(ze, size(ss1.A, 1));\n",
            "fit_percent = zeros(1, 6);\n",
            "[~, fit_percent(6)] = compare(zv, ss1);\n",
        ] {
            assert!(script.contains(line), "{line}\n{script}");
        }
        assert!(!script.contains("procest") && !script.contains("sysid_"));
        let saved = match host.generate(&ws, 42).unwrap() {
            toolkits::Action::SaveAppScript(AppKind::SystemIdentification, text) => text,
            _ => panic!("wrong save action"),
        };
        assert!(saved.starts_with("% System Identification (MatRust app)"));
        assert!(saved.ends_with(&script));

        let mut fresh = desktop_engine(&ws);
        fresh.execute(&saved).unwrap();
        let mut exported = desktop_engine(&ws);
        exported.execute(&export_code(&mut host, &ws)).unwrap();
        // The properties the Output column shows under their own names.
        let fields: [&[&str]; 6] = [
            &["A", "B"],
            &["A", "B", "C"],
            &["B", "F"],
            &["B", "C", "D", "F"],
            &["Numerator", "Denominator"],
            &["A", "B", "C", "D"],
        ];
        for (k, fields) in fields.iter().enumerate() {
            let name = &names[k];
            let result = host.app.models[k].result.as_ref().unwrap();
            for field in *fields {
                let shown = result.coefficient(field).unwrap();
                assert!(shown.iter().all(|v| v.is_finite()), "{name}.{field}");
                for engine in [&fresh, &exported] {
                    assert_eq!(
                        values(engine, &format!("{name}.{field}")),
                        shown,
                        "{name}.{field}"
                    );
                }
            }
            assert_eq!(
                values(&fresh, &format!("fit_percent({})", k + 1)),
                [result.fit],
                "{name}"
            );
            assert_eq!(
                values(&fresh, &format!("{name}.Report.Fit.FPE")),
                [result.fpe],
                "{name}"
            );
        }
        // n4sid chose the order of the range; the list shows it.
        let states = host.app.models[5].result.as_ref().unwrap().states.unwrap();
        assert_eq!(values(&fresh, "size(ss1.A, 1)"), [states as f64]);
        assert_eq!(
            host.app.models[5].describe(),
            format!("order 1:4, ssest, chose {states}")
        );
        assert!(script.contains("% ss1: State space, order 1:4, ssest\n"));
        assert!(script.contains("% tf1: Transfer function, poles 2, zeros 1\n"));
        assert!(script.contains("% bj1: BJ [nb nc nd nf nk]\n"));
        // The iddata objects are exported; the unticked model is not.
        assert_eq!(values(&exported, "size(data)"), [300., 1., 1.]);
        close(
            &values(&exported, "[size(ze, 1) size(zv, 1) zv.Tstart]"),
            &[150., 150., 15.],
            1e-12,
            "ranges",
        );
        assert!(exported.array("proc1").is_none());
        assert_eq!(
            host.app.exported(&ws),
            [
                "data",
                "ze",
                "zv",
                "arx1",
                "armax1",
                "oe1",
                "bj1",
                "tf1",
                "ss1",
                "fit_percent"
            ]
        );
        // The probe alone estimated the process model.
        let process = host.app.models[6].result.as_ref().unwrap();
        assert!(process.fit.is_finite() && !process.step.is_empty());
        assert_eq!(process.coefficient("Kp").unwrap().len(), 1);
    }

    #[test]
    fn system_identification_export_names_are_checked_and_export_is_all_or_nothing() {
        let ws = workspace(
            "u = sin((1:200)'/3) + sin((1:200)'/7); y = filter([0 1 0.5], [1 -1.5 0.7], u); \
             data = 7; arx1 = 8; keep = 9;",
        );
        let mut host = host();
        host.app.models = SystemIdentification::default().models;
        // The default names step past the workspace's variables once.
        compute(&mut host, &ws).unwrap();
        assert_eq!(host.app.names, ["data1", "ze", "zv", "fit_percent"]);
        assert_eq!(host.app.models[0].name, "arx2");
        for name in [
            "mrexp_old",
            "mrexp_had",
            "matrust_export_error",
            "mrexp_complete",
            "1bad",
            "for",
            "sysid_m1",
            "compare",
            "u",
            "y",
            "ze",
            "m);bad=1;%",
            "",
        ] {
            host.app.models[0].name = name.into();
            assert!(host.app.script(&ws).is_err(), "model {name}");
            assert!(host.request(&ws).is_err(), "model {name}");
        }
        host.app.models[0].name = "arx2".into();
        for k in 0..4 {
            let good = host.app.names[k].clone();
            for name in [
                "mrexp_old",
                "2x",
                "end",
                "arx",
                "u",
                "arx2",
                "z);clear all;%",
            ] {
                host.app.names[k] = name.into();
                assert!(host.app.script(&ws).is_err(), "names[{k}] = {name}");
            }
            host.app.names[k] = good;
        }
        host.app.names[1] = "zv".into();
        assert!(
            host.app
                .script(&ws)
                .unwrap_err()
                .contains("duplicate export name zv")
        );
        host.app.names[1] = "ze".into();
        // An unticked model's name is not exported, so it is not checked.
        host.app.models[0].name = "for".into();
        host.app.models[0].export = false;
        let script = host.app.script(&ws).unwrap();
        assert!(!script.contains("arx(") && !script.contains("compare"));
        assert_eq!(host.app.exported(&ws), ["data1", "ze", "zv"]);
        host.app.models[0].export = true;
        host.app.models[0].name = "keep".into();
        compute(&mut host, &ws).unwrap();

        // Export replaces `keep`; an error while exporting restores it.
        let code = export_code(&mut host, &ws);
        assert_eq!(
            host.message,
            "Exported data1, ze, zv, keep, fit_percent to the workspace"
        );
        let mut engine = desktop_engine(&ws);
        engine.execute(&code).unwrap();
        close(
            &values(&engine, "keep.A"),
            &[1., -1.5, 0.7],
            1e-8,
            "exported A",
        );
        assert_eq!(values(&engine, "data"), [7.]);
        // A workspace variable hides `compare`: the script fails after it
        // assigned every model, and nothing of it stays.
        let mut engine = desktop_engine(&ws);
        engine.execute("compare = 5;").unwrap();
        assert!(engine.execute(&code).is_err());
        assert_eq!(values(&engine, "keep"), [9.]);
        for name in ["data1", "ze", "zv", "fit_percent"] {
            assert!(engine.array(name).is_none(), "{name}");
        }
    }

    #[test]
    fn system_identification_mismatched_lengths_and_bad_fields_reach_the_status_line() {
        let ws = workspace(
            "u = sin((1:300)'/3) + sin((1:300)'/7); y = filter([0 1 0.5], [1 -1.5 0.7], u); \
             short_u = u(1:200); table = [u y]; z = 1 + 2i; step = u;",
        );
        let ctx = egui::Context::default();
        let mut host = host();
        host.app.add(Kind::Arx, &ws).unwrap();
        wait(&mut host, &ctx, &ws);
        assert_eq!(
            host.status().1,
            toolkits::AppStatus::Success,
            "{:?}",
            host.status()
        );
        assert!(host.status().0.contains("300 samples, Ts 0.1 s · 1 model"));
        let good = host.app.models[0].result.clone().unwrap();
        let data = host.app.data.clone().unwrap();

        // A mismatched input length is an error of the status line; the
        // views keep the last good result.
        host.app.input = "short_u".into();
        wait(&mut host, &ctx, &ws);
        let (text, status) = host.status();
        assert_eq!(status, toolkits::AppStatus::Error);
        assert!(
            text.contains("short_u has 200 samples and the output y has 300")
                && text.contains("same length"),
            "{text}"
        );
        assert_eq!(host.app.models[0].result.as_ref(), Some(&good));
        assert_eq!(host.app.data.as_ref(), Some(&data));
        assert!(host.app.steps(&ws).is_empty());
        for (input, message) in [
            ("", "choose the input u"),
            ("missing", "does not exist"),
            ("table", "real numeric vector"),
            ("z", "real numeric vector"),
            ("step", "hides the function step"),
            ("u); error('x'); %", "input u"),
        ] {
            host.app.input = input.into();
            let error = host.request(&ws).err().expect(input);
            assert!(error.contains(message), "{input}: {error}");
        }
        host.app.input = "u".into();

        // Every free-text field is one expression of its own kind.
        for bad in ["0", "-1", "Inf", "NaN", "0.1; x = 1", "0.1)+(1", "1/10", ""] {
            host.app.ts = bad.into();
            assert!(host.request(&ws).is_err(), "Ts {bad}");
        }
        host.app.ts = "0.1".into();
        for (range, bad) in [(0, "0"), (0, "301"), (0, "1.5"), (1, "1:2"), (1, "x")] {
            host.app.estimation[range] = bad.into();
            assert!(host.request(&ws).is_err(), "range {bad}");
            host.app.estimation[range].clear();
        }
        host.app.estimation = ["200".into(), "100".into()];
        assert!(
            host.request(&ws)
                .err()
                .unwrap()
                .contains("ends before it starts")
        );
        host.app.estimation = [String::new(), String::new()];
        for bad in [
            "[2 2]",
            "[2 2 1 1]",
            "[2 -2 1]",
            "[2 2.5 1]",
            "[2 2 21]",
            "[0 0 1]",
            "[2 2 1]); x=1; %",
            "ones(1,3)",
            "2 2 1",
            "2, 2, 1",
        ] {
            host.app.models[0].orders = bad.into();
            let error = host.request(&ws).err().expect(bad);
            assert!(error.contains("arx1"), "{bad}: {error}");
        }
        host.app.models[0].orders = " [2, 2,1] ".into();
        assert!(host.app.script(&ws).unwrap().contains("arx(ze, [2 2 1])"));
        host.app.models[0].set_kind(Kind::Oe);
        host.app.models[0].orders = "[0 2 1]".into();
        assert!(
            host.request(&ws)
                .err()
                .unwrap()
                .contains("nb must be at least 1")
        );
        host.app.models[0].set_kind(Kind::TransferFunction);
        for (poles, zeros, delay) in [
            ("0", "0", ""),
            ("2", "3", ""),
            ("2", "1", "-1"),
            ("2", "1", "Inf"),
            ("21", "1", ""),
            ("2", "1", "0.2); x=1; %"),
        ] {
            let model = &mut host.app.models[0];
            (model.poles, model.zeros, model.delay) = (poles.into(), zeros.into(), delay.into());
            assert!(host.request(&ws).is_err(), "{poles} {zeros} {delay}");
        }
        let model = &mut host.app.models[0];
        (model.poles, model.zeros, model.delay) = ("2".into(), "1".into(), "0.2".into());
        assert!(
            host.app
                .script(&ws)
                .unwrap()
                .contains("tfest(ze, 2, 1, 0.2);")
        );
        host.app.models[0].discrete = true;
        assert!(
            host.request(&ws)
                .err()
                .unwrap()
                .contains("IO delay in samples")
        );
        host.app.models[0].delay = "2".into();
        assert!(
            host.app
                .script(&ws)
                .unwrap()
                .contains("tfest(ze, 2, 1, 2, 'Ts', ze.Ts);")
        );
        host.app.models[0].set_kind(Kind::StateSpace);
        host.app.models[0].discrete = false;
        for bad in ["0", "4:2", "1:21", "1:2:9", "2.5", "[1 2]", "2); x=1; %"] {
            host.app.models[0].order = bad.into();
            assert!(host.request(&ws).is_err(), "order {bad}");
        }
        // A line break inside a field never reaches the script, not even
        // the comment above the estimation.
        for broken in ["1:\n4", "2\n", "\n2"] {
            host.app.models[0].order = broken.into();
            if let Ok(script) = host.app.script(&ws) {
                let known = |l: &str| l.starts_with('%') || l.contains(" = ");
                assert!(script.lines().all(known), "{script}");
                assert!(script.contains("% arx1: State space, order "), "{script}");
            }
        }
        host.app.models[0].order = "3".into();
        host.app.models[0].feedthrough = true;
        assert!(
            host.app
                .script(&ws)
                .unwrap()
                .contains("ssest(ze, 3, 'Feedthrough', true);")
        );
        host.app.models[0].refine = false;
        assert!(
            host.app
                .script(&ws)
                .unwrap()
                .contains("n4sid(ze, 3, 'Feedthrough', true);")
        );
        host.app.models[0].set_kind(Kind::Process);
        let model = &mut host.app.models[0];
        (model.process_poles, model.process_delay, model.process_zero) = (0, false, true);
        assert!(
            host.request(&ws)
                .err()
                .unwrap()
                .contains("a zero needs at least 1 pole")
        );
        let model = &mut host.app.models[0];
        (
            model.process_poles,
            model.process_zero,
            model.process_underdamped,
        ) = (1, false, true);
        assert!(
            host.request(&ws)
                .err()
                .unwrap()
                .contains("underdamped pair")
        );
        let model = &mut host.app.models[0];
        (model.process_poles, model.process_delay, model.process_zero) = (2, true, true);
        assert_eq!(model.process_type(), "P2DZU");
        assert!(
            host.app
                .script(&ws)
                .unwrap()
                .contains("procest(ze, 'P2DZU');")
        );
        // Nothing above replaced the last good result.
        assert_eq!(host.app.models[0].result.as_ref(), Some(&good));

        // An estimator's own error (too few samples) is shown the same way.
        host.app.models[0].set_kind(Kind::Arx);
        host.app.models[0].orders = "[2 2 1]".into();
        host.app.estimation = ["1".into(), "4".into()];
        wait(&mut host, &ctx, &ws);
        let (text, status) = host.status();
        assert_eq!(status, toolkits::AppStatus::Error);
        assert!(text.contains("too few samples"), "{text}");
        assert_eq!(host.app.models[0].result.as_ref(), Some(&good));
    }

    #[test]
    fn system_identification_iddata_source_ranges_detrend_and_views_of_every_model() {
        let ws = workspace(
            "t = (1:400)'; u = sign(sin(t/9)) + 0.3*sin(t/2); \
             y = filter([0 0.2 0.1], [1 -1.4 0.5], u) + 2 + 0.002*t; \
             z = iddata(y, u, 0.5); mimo = iddata([y y], u, 0.5); series = iddata(y, [], 0.5);",
        );
        assert_eq!(
            apps::system_identification::objects(&ws),
            [("z".to_owned(), 400)]
        );
        assert_eq!(apps::system_identification::vectors(&ws).len(), 3);
        let mut host = host();
        host.app.source = Source::Object;
        for (object, message) in [
            ("", "choose the iddata object"),
            ("u", "is not an iddata object"),
            ("mimo", "one output and one input channel"),
            ("series", "one output and one input channel"),
        ] {
            host.app.object = object.into();
            let error = host.request(&ws).err().expect(object);
            assert!(error.contains(message), "{object}: {error}");
        }
        host.app.object = "z".into();
        host.app.estimation = ["21".into(), "220".into()];
        host.app.validation = ["221".into(), String::new()];
        host.app.detrend = Detrend::Linear;
        for kind in Kind::ALL {
            host.app.add(kind, &ws).unwrap();
        }
        host.app.models[4].discrete = true;
        host.app.models[5].refine = false;
        host.app.models[6].process_delay = false;
        host.app.models[6].process_poles = 2;
        compute(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(script.starts_with("% System Identification: iddata object z, 400 samples\n"));
        for line in [
            "ze = z(21:220);\nzv = z(221:400);\n",
            "ze = detrend(ze, 1);\nzv = detrend(zv, 1);\n",
            "tf1 = tfest(ze, 2, 1, 'Ts', ze.Ts);\n",
            "ss1 = n4sid(ze, 1:10);\n",
            "proc1 = procest(ze, 'P2');\n",
        ] {
            assert!(script.contains(line), "{line}\n{script}");
        }
        assert!(!script.contains("iddata(") && !script.contains("ssest"));
        assert_eq!(host.app.inputs(), ["z"]);
        assert_eq!(
            host.app.steps(&ws),
            [
                "z: 400 samples",
                "Split: estimation 21:220, validation 221:400",
                "Detrend: remove the linear trend of each range",
                "Estimate 7 models and compare on validation",
            ]
        );
        let data = host.app.data.as_ref().unwrap();
        assert_eq!((data.samples, data.ts), (400, 0.5));
        assert_eq!((data.estimation, data.validation), ((21, 220), (221, 400)));
        assert_eq!(data.spans, [(10., 109.5), (110., 199.5)]);
        assert_eq!((data.output.len(), data.input.len()), (400, 400));
        // The linear trend is gone from the compared validation output.
        assert_eq!(data.measured.len(), 180);
        let mean = data.measured.iter().map(|p| p[1]).sum::<f64>() / 180.;
        assert!(mean.abs() < 1e-9, "{mean}");
        for model in &host.app.models {
            let r = model.result.as_ref().unwrap();
            let name = &model.name;
            // The data came from a second-order system: every structure fits
            // it up to the lines detrend took from the input and the output.
            assert!(r.fit > 90. && r.fit <= 100., "{name}: {}", r.fit);
            assert!(r.loss >= 0. && r.fpe >= r.loss, "{name}");
            assert_eq!(r.output.len(), 180, "{name}");
            // resid: lags 0..25 and -25..25 with the 99 % bounds.
            assert_eq!((r.auto.len(), r.cross.len()), (26, 51), "{name}");
            assert_eq!(
                (r.auto[0], r.cross[0][0], r.cross[50][0]),
                ([0., 1.], -25., 25.)
            );
            let bound = 2.5758293035489004 / 180_f64.sqrt();
            assert!((r.auto_bound - bound).abs() < 1e-12, "{name}");
            assert!(r.cross_bound > 0. && r.cross_bound.is_finite(), "{name}");
            assert!(
                r.step.len() > 10 && r.magnitude.len() == r.phase.len(),
                "{name}"
            );
            assert!(r.magnitude.len() > 10 && r.magnitude.iter().all(|p| p[0] > 0.));
            // DC gain of the true system: (0.2 + 0.1)/(1 - 1.4 + 0.5) = 3.
            let last = r.step.last().unwrap()[1];
            assert!((last - 3.).abs() < 0.3, "{name}: step ends at {last}");
        }
        // The arx row is near the system that made the data.
        close(coefficient(&host, 0, "A"), &[1., -1.4, 0.5], 0.05, "A");
        close(coefficient(&host, 0, "B"), &[0., 0.2, 0.1], 0.05, "B");
        // n4sid chose an order of the range 1:10.
        let states = host.app.models[5].result.as_ref().unwrap().states.unwrap();
        assert!((1..=10).contains(&states), "{states}");
        assert_eq!(
            host.app.models[5].describe(),
            format!("order 1:10, n4sid, chose {states}")
        );
        let rows = host.app.results();
        assert_eq!(rows[0], ("Model", "proc1".to_owned()));
        assert!(rows.iter().any(|(l, _)| *l == "Tp2") && rows.iter().all(|(l, _)| *l != "Td"));
    }

    #[test]
    fn system_identification_list_operations_show_hide_and_continuous_delay() {
        let ws = workspace(
            "t = (0:0.1:40)'; u = double(mod(floor(t/5), 2)); \
             y = zeros(size(u)); for k = 6:numel(u), y(k) = 0.9*y(k-1) + 0.2*u(k-5); end",
        );
        let mut host = host();
        host.app.models = SystemIdentification::default().models;
        host.app.models[0].orders = "[1 1 5]".into();
        host.app.duplicate(&ws).unwrap();
        assert_eq!(host.app.selected, 1);
        assert_eq!(host.app.models[1].orders, "[1 1 5]");
        assert_eq!(
            (
                host.app.models[0].name.as_str(),
                host.app.models[1].name.as_str()
            ),
            ("arx1", "arx2")
        );
        host.app.remove();
        assert_eq!((host.app.models.len(), host.app.selected), (1, 0));
        // A first-order lag with 0.4 s of delay: K = 2, T = 0.1/-log(0.9).
        host.app.add(Kind::Process, &ws).unwrap();
        host.app.add(Kind::TransferFunction, &ws).unwrap();
        let model = &mut host.app.models[2];
        (model.poles, model.zeros, model.delay) = ("1".into(), "0".into(), "0.4".into());
        // Validation starts at 25 s, after 5 s of zero input: compare
        // estimates the initial state, not the input still in the delay.
        host.app.validation[0] = "251".into();
        compute(&mut host, &ws).unwrap();
        close(
            coefficient(&host, 0, "B"),
            &[0., 0., 0., 0., 0., 0.2],
            1e-8,
            "B",
        );
        let tau = -0.1 / 0.9_f64.ln();
        let process = host.app.models[1].result.as_ref().unwrap();
        close(process.coefficient("Kp").unwrap(), &[2.], 1e-3, "Kp");
        close(process.coefficient("Tp1").unwrap(), &[tau], 1e-3, "Tp1");
        close(process.coefficient("Td").unwrap(), &[0.4], 1e-3, "Td");
        let rows = host.app.results();
        assert_eq!(rows[0], ("Model", "tf1".to_owned()));
        assert!(rows.iter().any(|(l, v)| *l == "IO delay" && v == "0.4"));
        // The delay holds the continuous step responses at zero for 0.4 s.
        for k in [1, 2] {
            let r = host.app.models[k].result.as_ref().unwrap();
            assert_eq!(&r.step[..2], [[0., 0.], [r.step[1][0], 0.]]);
            assert!((r.step[1][0] - 0.4).abs() < 1e-3, "{}", r.step[1][0]);
            assert!(r.step.iter().all(|p| p[0] >= 0.) && r.fit > 99.9);
            let end = r.step.last().unwrap()[1];
            assert!((end - 2.).abs() < 0.05, "{end}");
            // The phase keeps falling with the delay: below -180 degrees.
            assert!(r.phase.last().unwrap()[1] < -180.);
        }
        // Show/hide and the series colour do not touch the script.
        let request = host.request(&ws).unwrap().key;
        host.app.models[0].shown = false;
        host.app.models[1].color = 5;
        host.app.view = View::Bode;
        host.app.selected = 0;
        assert_eq!(host.request(&ws).unwrap().key, request);
        while host.app.models.len() < MAX_MODELS {
            host.app.add(Kind::Arx, &ws).unwrap();
        }
        assert!(
            host.app
                .add(Kind::Arx, &ws)
                .unwrap_err()
                .contains("at most 12 models")
        );
        assert!(host.app.duplicate(&ws).is_err());
        let mut names: Vec<&str> = host.app.models.iter().map(|m| m.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), MAX_MODELS);
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
    fn system_identification_tool_tab_renders_headless_empty_one_and_five_models() {
        let kind = AppKind::SystemIdentification;
        let tab = kind.tab();
        assert_eq!(AppKind::from_tab(tab), Some(kind));
        assert_eq!(
            serde_json::to_string(&tab).unwrap(),
            "\"system_identification\""
        );
        let decoded: matrust::session::ToolTab =
            serde_json::from_str("\"system_identification\"").unwrap();
        assert_eq!(decoded, tab);
        assert!(kind.is_built() && AppKind::ALL.contains(&kind) && !kind.detail().is_empty());
        assert_eq!(kind.label(), "System Identification");
        for width in [820., 1600.] {
            let size = egui::vec2(width, 950.);
            let ctx = egui::Context::default();
            let mut app = Workspace::with_context(ctx.clone(), true);
            app.run(
                "u = sin((1:300)'/3) + sin((1:300)'/7); y = filter([0 1 0.5], [1 -1.5 0.7], u);"
                    .into(),
            );
            finish(&mut app);
            app.toolkits.open_tool(Tool::App(kind));
            // Empty: no data chosen, the default model has nothing to show.
            let output = frame_at(&mut app, &ctx, size, vec![]);
            let text = painted(&output);
            for label in ["INPUTS", "IDENTIFICATION", "OUTPUT", "Time plot", "Models"] {
                assert!(text.iter().any(|t| t == label), "empty, {width}: {label}");
            }
            assert!(app.toolkits.apps.sysid.app.data.is_none());
            let status = app.toolkits.apps.sysid.status();
            assert_eq!(status.1, toolkits::AppStatus::Error);
            assert!(status.0.contains("choose the output y"), "{}", status.0);
            let host = &mut app.toolkits.apps.sysid;
            host.app.output = "y".into();
            host.app.input = "u".into();
            for kinds in [
                &[][..],
                &[Kind::Arx][..],
                &[
                    Kind::Arx,
                    Kind::Oe,
                    Kind::TransferFunction,
                    Kind::StateSpace,
                    Kind::Process,
                ][..],
            ] {
                let host = &mut app.toolkits.apps.sysid;
                host.app.models.clear();
                host.app.data = None;
                let ws = Snapshot::default();
                for kind in kinds {
                    host.app.add(*kind, &ws).unwrap();
                }
                let deadline = Instant::now() + Duration::from_secs(120);
                loop {
                    let output = frame_at(&mut app, &ctx, size, vec![]);
                    assert!(!output.shapes.is_empty());
                    let host = &app.toolkits.apps.sysid;
                    let ready = host.app.data.is_some()
                        && host.app.models.iter().all(|m| m.result.is_some())
                        && !host.running();
                    if ready {
                        break;
                    }
                    assert!(Instant::now() < deadline, "{:?}", host.status());
                    std::thread::sleep(Duration::from_millis(5));
                }
                let host = &app.toolkits.apps.sysid;
                assert_eq!(
                    host.status().1,
                    toolkits::AppStatus::Success,
                    "{:?}",
                    host.status()
                );
                assert_eq!(host.app.models.len(), kinds.len());
                for view in View::ALL {
                    app.toolkits.apps.sysid.app.view = view;
                    app.toolkits.apps.sysid.app.selected = kinds.len().saturating_sub(1);
                    let output = frame_at(&mut app, &ctx, size, vec![]);
                    let text = painted(&output);
                    for label in ["INPUTS", "OUTPUT", view.label(), "Steps", "Models"] {
                        assert!(
                            text.iter().any(|t| t == label),
                            "{} models, {width}, {view:?}: {label}",
                            kinds.len()
                        );
                    }
                    let model = app.toolkits.apps.sysid.app.model().map(|m| m.name.clone());
                    if let Some(name) = model {
                        assert!(text.iter().any(|t| t.contains(&name)), "{name}");
                    } else {
                        assert!(text.iter().any(|t| t == "No models yet."));
                    }
                }
            }
            assert_eq!(app.toolkits.selected_tool(), Some(Tool::App(kind)));
            app.close_tool_tab(Tool::App(kind));
        }
    }

    #[test]
    fn system_identification_100000_samples_stay_responsive_with_stop() {
        let ws = workspace(
            "N = 1e5; t = (1:N)'; u = sign(sin(t/37)) + 0.5*sin(t/11); \
             y = filter([0 1 0.5], [1 -1.5 0.7], u) + 0.01*sin(t/3); clear t N",
        );
        let ctx = egui::Context::default();
        let mut host = host();
        host.app.ts = "0.01".into();
        host.app.add(Kind::Arx, &ws).unwrap();
        host.app.add(Kind::Oe, &ws).unwrap();
        // The UI thread only submits: the frame returns at once.
        let start = Instant::now();
        host.refresh(&ctx, &ws);
        let submit = start.elapsed();
        assert!(host.running());
        assert!(submit < Duration::from_millis(200), "{submit:?}");
        // Every later frame is as quick while the models are estimated.
        let mut slowest = Duration::ZERO;
        loop {
            let frame = Instant::now();
            host.refresh(&ctx, &ws);
            slowest = slowest.max(frame.elapsed());
            if !host.running() {
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(120), "run timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
        let elapsed = start.elapsed();
        assert!(slowest < Duration::from_millis(200), "{slowest:?}");
        assert_eq!(
            host.status().1,
            toolkits::AppStatus::Success,
            "{:?}",
            host.status()
        );
        let data = host.app.data.as_ref().unwrap();
        assert_eq!(data.samples, 100_000);
        assert_eq!(
            (data.estimation, data.validation),
            ((1, 50_000), (50_001, 100_000))
        );
        // The views draw the extremes of 2000 stretches, not every sample.
        for points in [&data.output, &data.input, &data.measured] {
            assert!(
                points.len() > 2000 && points.len() <= DISPLAY_POINTS,
                "{}",
                points.len()
            );
            assert!(points.windows(2).all(|w| w[0][0] < w[1][0]));
        }
        let (low, high) = data
            .output
            .iter()
            .fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p[1]), b.max(p[1])));
        let y = match ws.get("y").unwrap() {
            matrust::Array::Numeric(m) => m.data().to_vec(),
            _ => unreachable!(),
        };
        assert_eq!(low, y.iter().copied().fold(f64::MAX, f64::min));
        assert_eq!(high, y.iter().copied().fold(f64::MIN, f64::max));
        for model in &host.app.models {
            let r = model.result.as_ref().unwrap();
            assert!(r.output.len() <= DISPLAY_POINTS && r.fit > 99., "{}", r.fit);
        }
        close(coefficient(&host, 0, "A"), &[1., -1.5, 0.7], 1e-2, "A");
        eprintln!(
            "system_identification 1e5 samples, ARX [2 2 1] + OE [2 2 1], compare, resid, step, bode: \
             {elapsed:?} (slowest frame {slowest:?})"
        );

        // Stop cancels a long estimation and keeps the last result.
        let fit = host.app.models[0].result.as_ref().unwrap().fit;
        host.app.add(Kind::Armax, &ws).unwrap();
        host.app.add(Kind::Process, &ws).unwrap();
        host.refresh(&ctx, &ws);
        assert!(host.running());
        host.app.stop();
        let stopping = Instant::now();
        wait(&mut host, &ctx, &ws);
        assert!(
            stopping.elapsed() < Duration::from_secs(10),
            "{:?}",
            stopping.elapsed()
        );
        assert!(host.status().0.contains("stopped"), "{:?}", host.status());
        assert_eq!(host.app.models[0].result.as_ref().unwrap().fit, fit);
        assert!(host.app.models[2].result.is_none());
        // Run estimates again; an edit would do the same.
        host.app.remove();
        host.app.remove();
        assert!(!host.app.is_stopped());
        wait(&mut host, &ctx, &ws);
        assert_eq!(
            host.status().1,
            toolkits::AppStatus::Success,
            "{:?}",
            host.status()
        );
        host.app.stop();
        assert!(host.app.is_stopped());
        host.app.run();
        wait(&mut host, &ctx, &ws);
        assert_eq!(host.status().1, toolkits::AppStatus::Success);
    }
}
