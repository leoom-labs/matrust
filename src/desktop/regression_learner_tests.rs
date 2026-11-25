// All expected fitted numbers come from NumPy least squares or the published
// definitions of RMSE, MSE, MAE and R-squared. Script parity is checked separately.
mod regression_learner_tests {
    use super::*;
    use apps::regression_learner::{
        FitResult, ModelEntry, ModelSpec, Preset, RegressionLearner, Sort, Spec, Validation, View,
        script_text,
    };
    use apps::{App, AppKind, Host, Snapshot};
    use std::collections::BTreeMap;

    fn data(nonlinear: bool) -> matrust::Engine {
        let mut state = 1121_u32;
        let mut uniform = || {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            f64::from(state) / 4294967296.
        };
        let (mut a, mut b, mut y) = (Vec::new(), Vec::new(), Vec::new());
        for i in 0..200 {
            let (x1, x2, noise) = (
                (uniform() - 0.5) * 4.,
                (uniform() - 0.5) * 4.,
                (uniform() - 0.5) * 0.05,
            );
            if nonlinear {
                let x = -3. + 6. * i as f64 / 199.;
                a.push(x);
                y.push(x.sin() + noise);
            } else {
                a.push(x1);
                b.push(x2);
                y.push(3. * x1 - 2. * x2 + 0.5 * x1 * x2 + noise);
            }
        }
        if !nonlinear {
            a.extend(b);
        }
        let mut engine = matrust::Engine::default();
        engine
            .insert_array(
                "X",
                matrust::Array::Numeric(
                    matrust::Matrix::from_column_vec(200, if nonlinear { 1 } else { 2 }, a)
                        .unwrap(),
                ),
            )
            .unwrap();
        engine
            .insert_array(
                "y",
                matrust::Array::Numeric(matrust::Matrix::from_column_vec(200, 1, y).unwrap()),
            )
            .unwrap();
        engine
    }
    fn workspace(engine: &matrust::Engine) -> Snapshot {
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        ws
    }
    fn values(engine: &matrust::Engine, name: &str) -> Vec<f64> {
        match engine.array(name) {
            Some(matrust::Array::Numeric(m)) => m.data().to_vec(),
            a => panic!("{name}: {a:?}"),
        }
    }
    fn metrics(r: &FitResult) -> [f64; 4] {
        [r.rmse, r.rsquared, r.mse, r.mae]
    }
    fn settle(app: &mut RegressionLearner) {
        let ctx = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(120);
        while app.running() {
            app.poll(&ctx);
            assert!(Instant::now() < deadline, "{}", app.message);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn mismatched_rows_show_the_counts_and_fit_the_900_pixel_model_row() {
        let mut e = matrust::Engine::default();
        e.execute("X = zeros(60,2); y = zeros(50,1);").unwrap();
        let ws = workspace(&e);
        let mut h = Host::new(RegressionLearner::default());
        h.app.train(&ws, false);
        settle(&mut h.app);
        let message = "X has 60 rows, y has 50";
        assert_eq!(
            apps::regression_learner::status(&h),
            (message.into(), toolkits::AppStatus::Error)
        );
        let ctx = egui::Context::default();
        crate::theme::configure(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900., 800.));
        let output = crate::headless::run(
            &ctx,
            egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            },
            |ui| apps::regression_learner::show(&mut h, ui, &ws, false, &mut None),
        );
        assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Text(t)
            if t.galley.text() == message && !t.galley.elided && s.clip_rect.intersect(screen).contains_rect(t.visual_bounding_rect()))));
    }

    fn trained(spec: Spec, preset: Preset, ws: &Snapshot) -> Host<RegressionLearner> {
        let mut host = Host::new(RegressionLearner::default());
        host.app.spec = spec;
        host.app.models = vec![ModelEntry::new(preset)];
        host.app.train(ws, false);
        settle(&mut host.app);
        assert!(host.app.models[0].result.is_some(), "{}", host.app.message);
        host
    }
    fn reproduce(
        host: &Host<RegressionLearner>,
        source: &matrust::Engine,
        ws: &Snapshot,
    ) -> matrust::Engine {
        let script = host.app.script(ws).unwrap();
        let mut engine = matrust::Engine::default();
        for (name, array) in source.workspace() {
            engine.insert_array(&name, array).unwrap();
        }
        engine
            .execute(&script)
            .unwrap_or_else(|e| panic!("{e}\n{script}"));
        engine
            .execute("shown = [rl_result.rmse rl_result.rsquared rl_result.mse rl_result.mae];")
            .unwrap();
        let result = host.app.models[host.app.selected].result.as_ref().unwrap();
        for (a, b) in values(&engine, "shown").iter().zip(metrics(result)) {
            assert!(a == &b || (a.is_nan() && b.is_nan()), "{a} != {b}");
        }
        for (label, value) in [
            ("RMSE", result.rmse),
            ("R-squared", result.rsquared),
            ("MSE", result.mse),
            ("MAE", result.mae),
        ] {
            assert!(host.app.results().contains(&(label, apps::short(value))));
        }
        engine
    }

    #[test]
    fn regression_learner_synthetic_and_nonlinear_script_metrics_match_shown() {
        for nonlinear in [false, true] {
            let source = data(nonlinear);
            let ws = workspace(&source);
            for preset in [
                Preset::Linear,
                Preset::Interactions,
                Preset::FineTree,
                Preset::LinearSvm,
                Preset::SquaredExponentialGp,
                Preset::Bagged,
            ] {
                let mut host = Host::new(RegressionLearner::default());
                host.app.models = vec![ModelEntry::new(preset)];
                host.app.models[0].model.gp_linear_basis = nonlinear;
                host.app.train(&ws, false);
                settle(&mut host.app);
                assert!(
                    host.app.models[0].result.is_some(),
                    "{}: {}",
                    preset.label(),
                    host.app.message
                );
                let engine = reproduce(&host, &source, &ws);
                assert_eq!(values(&engine, "rl_predictions").len(), 200);
                assert!(host.app.models[0].result.as_ref().unwrap().seconds >= 0.);
            }
        }
    }

    #[test]
    fn regression_learner_numpy_same_folds_and_preprocessing_reference() {
        // Set only when regenerating: the reference program consumes membership,
        // never fitted MatRust values. The regular gate only reads fixtures.
        let exporting = std::env::var_os("MATRUST_REGRESSION_EXPORT_FOLDS").is_some();
        let reference: BTreeMap<String, serde_json::Value> = serde_json::from_str(include_str!(
            "../../tests/fixtures/regression_learner/reference.json"
        ))
        .unwrap();
        let mut exported = BTreeMap::<String, Vec<f64>>::new();
        for (name, nonlinear, validation, pca) in [
            ("synthetic", false, Validation::KFold, false),
            ("nonlinear", true, Validation::KFold, false),
            ("holdout", false, Validation::Holdout, false),
            ("resubstitution", false, Validation::Resubstitution, false),
            ("pca", false, Validation::KFold, true),
            ("outliers", false, Validation::KFold, false),
        ] {
            let mut source = data(nonlinear);
            if name == "outliers" {
                source.execute("y(1:17:200) = y(1:17:200)+30;").unwrap();
            }
            let ws = workspace(&source);
            for (model, preset) in [
                ("linear", Preset::Linear),
                ("interactions", Preset::Interactions),
                ("robust", Preset::Robust),
            ] {
                let spec = Spec {
                    validation,
                    standardize: pca,
                    pca,
                    variance: "40".into(),
                    ..Default::default()
                };
                let host = trained(spec, preset, &ws);
                let r = host.app.models[0].result.as_ref().unwrap();
                exported.insert(name.into(), r.folds.clone());
                if exporting {
                    continue;
                }
                let expected = &reference[&format!("{name}_{model}")];
                for (actual, want) in metrics(r)
                    .iter()
                    .zip(expected["metrics"].as_array().unwrap())
                {
                    assert!(
                        (actual - want.as_f64().unwrap()).abs() < 1e-8,
                        "{name} {model}: {actual} vs {want}"
                    );
                }
                let fold_fixture: BTreeMap<String, Vec<f64>> = serde_json::from_str(include_str!(
                    "../../tests/fixtures/regression_learner/folds.json"
                ))
                .unwrap();
                assert_eq!(
                    r.folds, fold_fixture[name],
                    "partition membership changed; regenerate NumPy references"
                );
                let prediction: Vec<f64> = expected["predicted"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|v| v.as_f64())
                    .collect();
                for (actual, want) in r.predicted.iter().zip(prediction) {
                    assert!(
                        (actual - want).abs() < 1e-8,
                        "{name} {model}: {actual} vs {want}"
                    );
                }
                let mut engine = reproduce(&host, &source, &ws);
                engine
                    .execute("allPredicted = trainedModel.predictFcn(X);")
                    .unwrap();
                for (actual, want) in values(&engine, "allPredicted")
                    .iter()
                    .zip(expected["all_predictions"].as_array().unwrap())
                {
                    assert!(
                        (actual - want.as_f64().unwrap()).abs() < 1e-8,
                        "all-data {name} {model}: {actual} vs {want}"
                    );
                }
            }
        }
        if exporting {
            std::fs::write(
                "tests/fixtures/regression_learner/folds.json",
                serde_json::to_string_pretty(&exported).unwrap() + "\n",
            )
            .unwrap();
        }
    }

    #[test]
    fn regression_learner_holdout_resubstitution_and_perfect_linear_formula() {
        let mut source = data(false);
        source.execute("y = 3*X(:,1)-2*X(:,2); ideal = y;").unwrap();
        let ws = workspace(&source);
        for validation in Validation::ALL {
            let spec = Spec {
                validation,
                ..Default::default()
            };
            let host = trained(spec, Preset::Linear, &ws);
            let result = host.app.models[0].result.as_ref().unwrap();
            assert!((result.rsquared - 1.).abs() < 1e-12);
            assert!(result.rmse < 1e-12);
            assert_eq!(
                result.actual.len(),
                if validation == Validation::Holdout {
                    50
                } else {
                    200
                }
            );
            let mut engine = reproduce(&host, &source, &ws);
            engine.execute("trainingCount = trainedModel.NumObservations; p = trainedModel.predictFcn(X); difference = max(abs(p - ideal));").unwrap();
            assert_eq!(values(&engine, "trainingCount"), [200.]);
            assert!(values(&engine, "difference")[0] < 1e-11);
        }
    }

    #[test]
    fn regression_learner_table_missing_responses_and_all_data_export() {
        let mut source = data(false);
        source.execute("y([2 9 50]) = NaN; T = table(X(:,1), X(:,2), y, repmat({'ignore'},200,1), 'VariableNames', {'one','two','response','text'});").unwrap();
        let ws = workspace(&source);
        let matrix = trained(Spec::default(), Preset::Interactions, &ws);
        let spec = Spec {
            use_table: true,
            table: "T".into(),
            response: "response".into(),
            predictors: vec!["one".into(), "two".into()],
            ..Default::default()
        };
        let mut table = trained(spec, Preset::Interactions, &ws);
        let r = table.app.models[0].result.as_ref().unwrap();
        assert_eq!(r.dropped, 3);
        assert_eq!(r.records.len(), 197);
        assert!(!r.records.contains(&2.));
        assert!(!r.records.contains(&9.));
        assert!(!r.records.contains(&50.));
        assert_eq!(
            metrics(r),
            metrics(matrix.app.models[0].result.as_ref().unwrap())
        );
        assert!(table.app.message.contains("3 missing responses dropped"));
        table.app.export_predictions = true;
        let mut engine = reproduce(&table, &source, &ws);
        engine.execute("count = trainedModel.NumObservations; n = trainedModel.RegressionModel.NumObservations;").unwrap();
        assert_eq!(values(&engine, "count"), [197.]);
        assert_eq!(values(&engine, "n"), [197.]);
        assert_eq!(values(&engine, "validationPredictions").len(), 197);
        assert!(table.app.script(&ws).unwrap().contains("T.('one')"));
        let mut invalid = table.app.spec.clone();
        invalid.predictors.push("text".into());
        assert!(
            script_text(
                &invalid,
                &ModelSpec::new(Preset::Linear),
                &ws,
                "model",
                None
            )
            .is_err()
        );
    }

    #[test]
    fn regression_learner_failures_continue_keep_last_good_and_stop_cancels() {
        let mut source = data(false);
        let ws = workspace(&source);
        let mut host = trained(Spec::default(), Preset::Linear, &ws);
        let previous = metrics(host.app.models[0].result.as_ref().unwrap());
        host.app.spec.folds = "2".into();
        host.app.spec.x = "X(1:4,:)".into();
        host.app.spec.y = "y(1:4)".into();
        host.app.models.push(ModelEntry::new(Preset::FineTree));
        host.app.train(&ws, true);
        settle(&mut host.app);
        assert!(
            host.app.models[0].status.contains("degrees of freedom"),
            "{}",
            host.app.models[0].status
        );
        assert_eq!(
            metrics(host.app.models[0].result.as_ref().unwrap()),
            previous
        );
        assert!(host.app.models[1].result.is_some());
        assert!(host.app.error);
        assert!(matches!(
            apps::regression_learner::status(&host).1,
            toolkits::AppStatus::Error
        ));
        // Constant responses have undefined R-squared, while supported models train.
        source.execute("y = ones(200,1);").unwrap();
        let constant_ws = workspace(&source);
        let constant = trained(Spec::default(), Preset::FineTree, &constant_ws);
        let r = constant.app.models[0].result.as_ref().unwrap();
        assert_eq!(r.mse, 0.);
        assert!(r.rsquared.is_nan());
        // Cancel both an active job and queued models. A retired result must not arrive later.
        let mut app = RegressionLearner::default();
        app.models.clear();
        app.models.extend([
            ModelEntry::new(Preset::SquaredExponentialGp),
            ModelEntry::new(Preset::FineTree),
        ]);
        app.train(&ws, true);
        app.poll(&egui::Context::default());
        app.stop();
        settle(&mut app);
        assert!(
            app.models
                .iter()
                .all(|m| m.status == "Cancelled" && m.result.is_none())
        );
        app.selected = 1;
        app.train(&ws, false);
        settle(&mut app);
        assert!(app.models[1].result.is_some());
        assert!(app.models[0].result.is_none());
    }

    #[test]
    fn regression_learner_all_presets_have_supported_calls_and_bounds() {
        let mut source = data(false);
        // Exercise every preset through Runner. Exact GP can reject a valid
        // configuration when its bounded optimizer exhausts its work budget.
        // Successful 200-row GP comparison is checked separately above.
        source.execute("X=X(1:60,:); y=y(1:60);").unwrap();
        let ws = workspace(&source);
        let mut app = RegressionLearner::default();
        app.models.clear();
        app.models
            .extend(Preset::ALL.into_iter().map(ModelEntry::new));
        app.spec.folds = "2".into();
        app.train(&ws, true);
        settle(&mut app);
        for entry in &app.models {
            assert!(
                entry.result.is_some()
                    || (matches!(
                        entry.model.preset,
                        Preset::SquaredExponentialGp
                            | Preset::MaternGp
                            | Preset::ExponentialGp
                            | Preset::RationalQuadraticGp
                    ) && entry
                        .status
                        .contains("exact likelihood optimization work limit exceeded")),
                "{}: {}",
                entry.model.preset.label(),
                entry.status
            );
        }
        let fine = ModelSpec::new(Preset::FineTree).fit_call("X", "y").unwrap();
        assert!(fine.contains("'MinLeafSize', 4"));
        assert!(
            ModelSpec::new(Preset::MediumTree)
                .fit_call("X", "y")
                .unwrap()
                .contains("'MinLeafSize', 12")
        );
        assert!(
            ModelSpec::new(Preset::CoarseTree)
                .fit_call("X", "y")
                .unwrap()
                .contains("'MinLeafSize', 36")
        );
        let mut bad = Spec::default();
        for seed in ["NaN", "Inf", "-1", "1e20", "1.5"] {
            bad.seed = seed.into();
            assert!(
                script_text(&bad, &ModelSpec::new(Preset::Linear), &ws, "model", None).is_err()
            );
        }
        let too_many = Spec {
            x: "ones(20001,1)".into(),
            y: "ones(20001,1)".into(),
            ..Default::default()
        };
        let script = script_text(
            &too_many,
            &ModelSpec::new(Preset::FineTree),
            &ws,
            "model",
            None,
        )
        .unwrap();
        assert!(
            matrust::Engine::default()
                .execute(&script)
                .unwrap_err()
                .contains("20000")
        );
    }

    #[test]
    fn regression_learner_export_names_injection_saved_script_and_snapshot() {
        let mut source = data(false);
        let ws = workspace(&source);
        let mut host = trained(Spec::default(), Preset::Linear, &ws);
        for bad in [
            "X",
            "y",
            "not a name",
            "mrexp_old",
            "mrexp_had",
            "mrexp_complete",
            "matrust_export_error",
            "rl_result",
            "regressionLearnerRun",
            "regressionLearnerRobust",
            "a); injected = 7; %",
        ] {
            host.app.model_name = bad.into();
            assert!(host.app.script(&ws).is_err(), "{bad}");
        }
        host.app.model_name = "trainedModel".into();
        host.app.export_predictions = true;
        host.app.prediction_name = "trainedModel".into();
        assert!(host.app.script(&ws).is_err());
        host.app.prediction_name = "validationPredictions".into();
        let script = host.app.script(&ws).unwrap();
        let Some(toolkits::Action::SaveAppScript(kind, saved)) = host.generate(&ws, 1700000000)
        else {
            panic!("save action");
        };
        assert_eq!(kind, AppKind::RegressionLearner);
        assert_eq!(saved, apps::saved_script(kind, 1700000000, &script));
        let Some(toolkits::Action::ExportScript(export)) = host.export(&ws) else {
            panic!("export action");
        };
        source
            .execute("trainedModel = 17; validationPredictions = 23;")
            .unwrap();
        source.execute(&export).unwrap();
        source
            .execute("count = trainedModel.NumObservations;")
            .unwrap();
        assert_eq!(values(&source, "count"), [200.]);
        // Reserved wrapper names and input injection cannot enter a generated program.
        for injection in [
            "X); injected = 1; %",
            "X); injected = 1; (X",
            "X) %",
            "X)\nclear y\n(X",
            "X); error('oops'); (X",
        ] {
            let spec = Spec {
                x: injection.into(),
                ..Default::default()
            };
            assert!(
                script_text(&spec, &ModelSpec::new(Preset::Linear), &ws, "model", None).is_err(),
                "{injection}"
            );
        }
        let mut bad = ModelSpec::new(Preset::FineTree);
        bad.leaf = "4); injected = 1; (4".into();
        assert!(script_text(&Spec::default(), &bad, &ws, "model", None).is_err());
        source.execute("y = y+1;").unwrap();
        assert!(
            host.app
                .script(&workspace(&source))
                .unwrap_err()
                .contains("changed")
        );
        // Editing settings keeps the displayed fit and its saved configuration.
        host.app.spec.folds = "7".into();
        assert!(host.app.script(&ws).unwrap().contains("'KFold', 5"));
    }

    #[test]
    fn regression_learner_transactional_failure_and_cancel_restore_outputs() {
        let source = data(false);
        let ws = workspace(&source);
        let mut host = trained(Spec::default(), Preset::Linear, &ws);
        host.app.export_predictions = true;
        let script = host.app.script(&ws).unwrap();
        let (body, functions) = apps::split_local_functions(&script);
        let failing = format!("{body}error('deliberate failure');\n{functions}");
        let names = host.app.exported(&ws);
        let wrapped = apps::all_or_nothing(&failing, &names);
        let mut engine = data(false);
        engine
            .execute("trainedModel = 17; validationPredictions = 23;")
            .unwrap();
        assert!(engine.execute(&wrapped).is_err());
        assert_eq!(values(&engine, "trainedModel"), [17.]);
        assert_eq!(values(&engine, "validationPredictions"), [23.]);
        let cancelled = apps::all_or_nothing(&script, &names);
        engine
            .cancellation()
            .store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(apps::execute_export(&mut engine, &cancelled).is_err());
        assert_eq!(values(&engine, "trainedModel"), [17.]);
        assert_eq!(values(&engine, "validationPredictions"), [23.]);
    }

    #[test]
    fn regression_learner_each_tab_headless_zero_one_twelve_models_and_sorting() {
        let source = data(false);
        let ws = workspace(&source);
        let host = trained(Spec::default(), Preset::Linear, &ws);
        let prototype = host.app.models[0].clone();
        for count in [0, 1, 12] {
            let ctx = egui::Context::default();
            let mut host = Host::new(RegressionLearner::default());
            host.app.models = Preset::ALL
                .into_iter()
                .take(count)
                .map(|p| {
                    let mut m = prototype.clone();
                    m.model = ModelSpec::new(p);
                    m
                })
                .collect();
            for view in View::ALL {
                host.app.view = view;
                for size in [egui::vec2(1440., 900.), egui::vec2(800., 700.)] {
                    let output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                            ..Default::default()
                        },
                        |ui| {
                            apps::regression_learner::show(&mut host, ui, &ws, false, &mut None);
                        },
                    );
                    assert!(!output.shapes.is_empty());
                    assert!(!host.app.running());
                }
            }
            host.app.sort = Sort::Name;
            assert_eq!(host.app.order().len(), count);
        }
        let mut sorting = RegressionLearner::default();
        sorting.models.clear();
        for rmse in [3., 1., 2.] {
            let mut entry = prototype.clone();
            entry.result.as_mut().unwrap().rmse = rmse;
            sorting.models.push(entry);
        }
        sorting.sort = Sort::Rmse;
        assert_eq!(sorting.order(), [1, 2, 0]);
        sorting.descending = true;
        assert_eq!(sorting.order(), [0, 2, 1]);
        // A fresh app is a regular tool tab and survives its session encoding.
        let kind = AppKind::RegressionLearner;
        assert!(kind.is_built());
        assert_eq!(AppKind::from_tab(kind.tab()), Some(kind));
        assert_eq!(
            serde_json::from_str::<matrust::session::ToolTab>(
                &serde_json::to_string(&kind.tab()).unwrap()
            )
            .unwrap(),
            kind.tab()
        );
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.toolkits.open_tool(toolkits::Tool::App(kind));
        let output = frame_at(&mut app, &ctx, egui::vec2(1440., 900.), vec![]);
        assert!(
            output
                .shapes
                .iter()
                .any(|s| text_position(&s.shape, "Train all").is_some())
        );
        app.close_tool_tab(toolkits::Tool::App(kind));
        assert!(!app.toolkits.apps.regression.app.running());
    }
}
