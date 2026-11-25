mod classification_learner_tests {
    use super::*;
    use crate::toolkits::Action;
    use apps::classification_learner::{
        ClassificationLearner, Model, Preset, Spec, Validation, View, script_text,
    };
    use apps::{App, Snapshot};

    fn workspace(setup: &str) -> Snapshot {
        let mut engine = matrust::Engine::default();
        engine.execute(setup).unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        ws
    }
    fn numbers(engine: &matrust::Engine, name: &str) -> Vec<f64> {
        match engine.array(name) {
            Some(matrust::Array::Numeric(m)) => m.data().to_vec(),
            other => panic!("{name}: {other:?}"),
        }
    }
    fn execute(ws: &Snapshot, code: &str) -> matrust::Engine {
        let mut engine = matrust::Engine::default();
        for (n, a) in ws.variables() {
            engine.insert_array(n, a.clone()).unwrap();
        }
        engine
            .execute(code)
            .unwrap_or_else(|e| panic!("{code}\n{e}"));
        engine
    }
    fn train(app: &mut ClassificationLearner, ws: &Snapshot, all: bool) {
        app.train(ws, all);
        let ctx = egui::Context::default();
        let start = std::time::Instant::now();
        while app.running() {
            app.refresh(&ctx);
            assert!(
                start.elapsed().as_secs() < 120,
                "Training timed out: {:?}",
                app.status()
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    fn app(preset: Preset) -> ClassificationLearner {
        let mut app = ClassificationLearner::default();
        app.add(preset);
        app
    }
    const SEPARABLE: &str = "X=[-4 -1;-3 1;-2 -1;-1 1;1 -1;2 1;3 -1;4 1];Y=[1;1;1;1;2;2;2;2];";
    #[test]
    fn classification_learner_smoke_script_and_separable_auc() {
        let ws = workspace(SEPARABLE);
        let mut app = app(Preset::FineKnn);
        app.spec.folds = "2".into();
        train(&mut app, &ws, false);
        assert!(app.models[0].error.is_none(), "{:?}", app.models[0].error);
        let r = app.models[0].result.as_ref().unwrap();
        assert_eq!(r.accuracy, 100.);
        assert_eq!(r.auc, vec![1., 1.]);
        assert_eq!(r.cost, 0.);
        assert_eq!(r.precision, vec![1., 1.]);
        assert_eq!(r.recall, vec![1., 1.]);
        assert_eq!(r.confusion, vec![4., 0., 0., 4.]);
        let script = app.script(&ws).unwrap();
        let mut engine = execute(&ws, &script);
        assert_eq!(numbers(&engine, "clAccuracy"), vec![r.accuracy]);
        engine.execute("n=trainedModel.NumObservations;").unwrap();
        assert_eq!(numbers(&engine, "n"), vec![8.]);
    }

    /// LCG uniforms and Box-Muller pairs: the Python reference independently
    /// generates precisely these Gaussian data, including the overlapping class.
    fn synthetic(binary: bool) -> Snapshot {
        let mut state = 917u32;
        let mut rows = Vec::new();
        let centers = if binary {
            vec![[-2., -1.], [2., 1.]]
        } else {
            vec![[-2., -0.7], [2., 0.7], [0.2, 0.1]]
        };
        for (k, center) in centers.iter().enumerate() {
            for _ in 0..50 {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                let u = (state as f64 + 0.5) / 4294967296.;
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                let v = (state as f64 + 0.5) / 4294967296.;
                let radius = (-2. * u.ln()).sqrt();
                let angle = std::f64::consts::TAU * v;
                let scale = if binary {
                    0.4
                } else if k == 2 {
                    1.2
                } else {
                    0.8
                };
                rows.push([
                    center[0] + scale * radius * angle.cos(),
                    center[1] + scale * radius * angle.sin(),
                ]);
            }
        }
        let n = rows.len();
        let x = (0..2)
            .flat_map(|j| rows.iter().map(move |r| r[j]))
            .collect();
        let y = (0..n).map(|i| (i / 50 + 1) as f64).collect();
        let mut engine = matrust::Engine::default();
        engine
            .insert_array(
                "X",
                matrust::Array::Numeric(matrust::Matrix::from_column_vec(n, 2, x).unwrap()),
            )
            .unwrap();
        engine
            .insert_array(
                "Y",
                matrust::Array::Numeric(matrust::Matrix::from_column_vec(n, 1, y).unwrap()),
            )
            .unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        ws
    }

    #[test]
    fn classification_learner_synthetic_replay_and_sklearn_same_folds() {
        let reference: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/classification_learner/reference.json"
        ))
        .unwrap();
        let export = std::env::var_os("MATRUST_CLASSIFICATION_REFERENCE_FOLDS");
        let mut exported = serde_json::Map::new();
        for binary in [false, true] {
            let ws = synthetic(binary);
            let key = if binary { "two" } else { "three" };
            let mut app = ClassificationLearner::default();
            for preset in [
                Preset::MediumTree,
                Preset::LinearDiscriminant,
                Preset::GaussianBayes,
                Preset::LinearSvm,
                Preset::FineKnn,
                Preset::MediumKnn,
            ] {
                app.add(preset);
            }
            train(&mut app, &ws, true);
            let mut fold_ids = Vec::new();
            for i in 0..app.models.len() {
                app.selected = i;
                let row = &app.models[i];
                assert!(
                    row.error.is_none(),
                    "{key} {:?}: {:?}",
                    row.model.preset,
                    row.error
                );
                let r = row.result.as_ref().unwrap();
                let script = app.script(&ws).unwrap();
                assert!(!script.contains("confusionchart") && !script.contains("resubPredict"));
                let engine = execute(&ws, &script);
                assert_eq!(numbers(&engine, "clAccuracy"), vec![r.accuracy]);
                assert_eq!(numbers(&engine, "clConfusion"), r.confusion);
                fold_ids = numbers(&engine, "clFoldId");
                for j in 0..r.classes.len() {
                    assert_eq!(
                        (0..r.classes.len())
                            .map(|k| r.confusion[j + k * r.classes.len()])
                            .sum::<f64>(),
                        50.
                    );
                }
                let reference_key = match row.model.preset {
                    Preset::LinearDiscriminant => Some("lda"),
                    Preset::GaussianBayes => Some("nb"),
                    Preset::FineKnn => Some("knn1"),
                    Preset::MediumKnn => Some("knn10"),
                    _ => None,
                };
                if export.is_none() {
                    assert_eq!(
                        fold_ids,
                        reference[key]["folds"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|v| v.as_f64().unwrap())
                            .collect::<Vec<_>>()
                    );
                    if let Some(model) = reference_key {
                        let expected = reference[key][model].as_f64().unwrap();
                        // GaussianNB's variance convention differs by a small
                        // finite-sample correction; the reference documents 2%.
                        let tolerance = reference["tolerance_percent"][model].as_f64().unwrap();
                        assert!(
                            (r.accuracy - expected).abs() <= tolerance,
                            "{key} {model}: {} versus {expected}",
                            r.accuracy
                        );
                    }
                }
            }
            exported.insert(key.into(), serde_json::json!(fold_ids));
        }
        if let Some(path) = export {
            std::fs::write(path, serde_json::to_string_pretty(&exported).unwrap()).unwrap();
        }
    }

    #[test]
    fn classification_learner_holdout_resubstitution_and_fold_local_preprocessing() {
        let ws = synthetic(false);
        for validation in [
            Validation::Holdout,
            Validation::Resubstitution,
            Validation::KFold,
        ] {
            let mut app = app(Preset::FineKnn);
            app.spec.validation = validation;
            app.spec.standardize = true;
            app.spec.pca = true;
            app.spec.variance = "80".into();
            train(&mut app, &ws, false);
            assert!(app.models[0].error.is_none(), "{:?}", app.models[0].error);
            let script = app.script(&ws).unwrap();
            assert!(
                script.find("for clFold").unwrap() < script.find("clMean = mean(clTrainX").unwrap()
            );
            assert!(script.contains("pca(clTrainX)"));
            assert!(script.contains("pca(clFinalX)"));
            let mut engine = execute(&ws, &script);
            assert_eq!(
                numbers(&engine, "clAccuracy"),
                vec![app.models[0].result.as_ref().unwrap().accuracy]
            );
            engine.execute("n=trainedModel.ClassificationModel.NumObservations;yp=trainedModel.predictFcn(X);").unwrap();
            assert_eq!(numbers(&engine, "n"), vec![150.]);
            assert_eq!(numbers(&engine, "yp").len(), 150);
            if validation == Validation::Resubstitution {
                assert_eq!(numbers(&engine, "clAccuracy"), vec![100.]);
            }
            if validation == Validation::Holdout {
                assert!(
                    numbers(&engine, "clPred")
                        .iter()
                        .filter(|v| v.is_finite())
                        .count()
                        < 150
                );
            }
        }
    }

    #[test]
    fn classification_learner_table_labels_and_missing_response() {
        for response in [
            "[1;1;NaN;1;2;2;2;2]",
            "categorical({'a';'a';'';'a';'b';'b';'b';'b'})",
            r#"["a";"a";missing;"a";"b";"b";"b";"b"]"#,
            "{'a';'a';'';'a';'b';'b';'b';'b'}",
            "logical([0;0;0;0;1;1;1;1])",
        ] {
            let setup = format!(
                "{SEPARABLE}Y={response};T=table(X(:,1),X(:,2),Y,'VariableNames',{{'first','second','response'}});"
            );
            let ws = workspace(&setup);
            for table in [false, true] {
                let mut app = app(Preset::FineKnn);
                app.spec.validation = Validation::Resubstitution;
                app.export_predictions = true;
                if table {
                    app.spec.table = true;
                    app.spec.x = "T".into();
                    app.spec.response = "response".into();
                    app.spec.predictors = vec!["first".into(), "second".into()];
                }
                train(&mut app, &ws, false);
                assert!(
                    app.models[0].error.is_none(),
                    "{response} table={table}: {:?}",
                    app.models[0].error
                );
                let r = app.models[0].result.as_ref().unwrap();
                assert_eq!(r.dropped, usize::from(!response.starts_with("logical")));
                assert_eq!(r.accuracy, 100.);
                let mut engine = execute(&ws, &app.script(&ws).unwrap());
                engine.execute("n=trainedModel.NumObservations;yp=predict(trainedModel,clX);same=isequal(yp,clLabels);sameValidation=isequal(validationPredictions,clLabels);").unwrap();
                assert_eq!(
                    numbers(&engine, "n"),
                    vec![if response.starts_with("logical") {
                        8.
                    } else {
                        7.
                    }]
                );
                assert!(
                    matches!(engine.array("same"),Some(matrust::Array::Logical(v)) if v.data()==[1])
                );
                assert!(
                    matches!(engine.array("sameValidation"),Some(matrust::Array::Logical(v)) if v.data()==[1])
                );
            }
        }
    }

    #[test]
    fn classification_learner_error_rows_continue_stop_and_keep_last_plot() {
        let ws = workspace("X=ones(12,2);Y=[ones(6,1);2*ones(6,1)];");
        let mut app = app(Preset::LinearDiscriminant);
        app.add(Preset::FineKnn);
        app.spec.validation = Validation::Resubstitution;
        train(&mut app, &ws, true);
        assert!(app.models[0].error.is_some());
        assert!(app.models[1].result.is_some());
        assert!(app.models[1].error.is_none());
        let accuracy = app.models[1].result.as_ref().unwrap().accuracy;
        app.models[1].model.parameter = "1000".into();
        train(&mut app, &ws, false);
        assert!(app.models[1].error.is_some());
        assert_eq!(app.models[1].result.as_ref().unwrap().accuracy, accuracy);
        // Unsupported options fail a row before starting a worker; next row runs.
        app.add(Preset::KernelBayes);
        app.add(Preset::FineKnn);
        train(&mut app, &ws, true);
        assert!(app.models[2].error.as_ref().unwrap().contains("Gaussian"));
        assert!(app.models[3].result.is_some());
        let one = workspace("X=[1;2;3;4];Y=ones(4,1);");
        let mut single = app_for_one_class();
        single.spec.validation = Validation::Resubstitution;
        train(&mut single, &one, true);
        assert!(
            single
                .models
                .iter()
                .all(|r| r.error.as_ref().is_some_and(|s| s.contains("classes")))
        );
        let ctx = egui::Context::default();
        let last = app.models[3].result.as_ref().unwrap().accuracy;
        app.selected = 3;
        app.train(&ws, false);
        app.refresh(&ctx);
        app.stop();
        for _ in 0..5 {
            app.refresh(&ctx);
        }
        assert!(app.stopped);
        assert!(app.models[3].result.is_some());
        // No cancelled result can subsequently replace the last successful plot.
        while app.running() {
            app.refresh(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(app.stopped);
        assert_eq!(app.models[3].result.as_ref().unwrap().accuracy, last);
        assert_eq!(app.models[3].error.as_deref(), Some("Stopped"));
        assert!(app.script(&ws).is_err());
    }
    fn app_for_one_class() -> ClassificationLearner {
        let mut a = app(Preset::MediumTree);
        a.add(Preset::FineKnn);
        a
    }

    #[test]
    fn classification_learner_export_names_parser_bounds_and_atomic_export() {
        let ws = workspace(SEPARABLE);
        let mut app = app(Preset::FineKnn);
        app.spec.validation = Validation::Resubstitution;
        train(&mut app, &ws, false);
        app.export_predictions = true;
        for name in [
            "mrexp_old",
            "mrexp_had",
            "matrust_export_error",
            "mrexp_complete",
            "X",
            "Y",
            "clPred",
            "bad;name",
            "1bad",
            "for",
            "end",
        ] {
            app.model_name = name.into();
            assert!(app.script(&ws).is_err(), "{name}");
        }
        app.model_name = "validationPredictions".into();
        assert!(app.script(&ws).is_err());
        app.model_name = "trainedModel".into();
        let Action::ExportScript(code) = app.export(&ws).unwrap() else {
            panic!()
        };
        let mut engine = execute(&ws, "trainedModel=123;validationPredictions=456;");
        apps::execute_export(&mut engine, &code).unwrap();
        engine.execute("n=trainedModel.NumObservations;").unwrap();
        assert_eq!(numbers(&engine, "n"), vec![8.]);
        let good = engine.workspace();
        let fail = apps::all_or_nothing(
            "trainedModel=7;error('test failure');",
            &["trainedModel".into()],
        );
        assert!(apps::execute_export(&mut engine, &fail).is_err());
        assert!(
            engine
                .array("trainedModel")
                .unwrap()
                .same_snapshot(&good["trainedModel"])
        );
        let Action::SaveAppScript(kind, saved) = app.generate(&ws, 0).unwrap() else {
            panic!()
        };
        assert_eq!(kind, apps::AppKind::ClassificationLearner);
        assert!(saved.ends_with(&app.script(&ws).unwrap()));
        for malicious in [
            "X);bad=1;clX=(X",
            "X % trailing",
            "X; Y",
            "X)\nerror('injected'); clX=(X",
        ] {
            let spec = Spec {
                x: malicious.into(),
                ..Spec::default()
            };
            assert!(
                script_text(
                    &spec,
                    &Model::new(Preset::FineKnn),
                    &ws,
                    "trainedModel",
                    None
                )
                .is_err()
            );
        }
        for seed in ["42;quit", "1 %comment", "-1", "4294967296"] {
            let spec = Spec {
                seed: seed.into(),
                ..Spec::default()
            };
            assert!(
                script_text(
                    &spec,
                    &Model::new(Preset::FineKnn),
                    &ws,
                    "trainedModel",
                    None
                )
                .is_err()
            );
        }
        app.spec.seed = "43".into();
        assert!(app.script(&ws).is_err());
        let spec = Spec {
            x: "zeros(10001,2)".into(),
            y: "ones(10001,1)".into(),
            ..Spec::default()
        };
        let code = script_text(
            &spec,
            &Model::new(Preset::FineKnn),
            &ws,
            "trainedModel",
            None,
        )
        .unwrap();
        let mut engine = matrust::Engine::default();
        assert!(engine.execute(&code).unwrap_err().contains("limits"));
    }

    #[test]
    fn classification_learner_headless_every_tab_zero_one_twelve_models() {
        let ws = workspace(SEPARABLE);
        let ctx = egui::Context::default();
        for count in [0, 1, 12] {
            let mut app = ClassificationLearner::default();
            for _ in 0..count {
                app.add(Preset::FineKnn);
            }
            if count > 0 {
                app.spec.validation = Validation::Resubstitution;
                train(&mut app, &ws, true);
            }
            for view in View::ALL {
                app.view = view;
                let output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1440., 900.),
                        )),
                        ..Default::default()
                    },
                    |ui| app.show(ui, &ws, false, &mut None),
                );
                assert!(!output.shapes.is_empty());
                for label in [
                    "INPUTS",
                    "VALIDATION",
                    "OUTPUT",
                    "Train selected",
                    "Train all",
                    "Generate script",
                ] {
                    assert!(painted(&output, label), "{count} {view:?}: {label}");
                }
            }
        }
        assert_eq!(
            apps::AppKind::from_tab(matrust::session::ToolTab::ClassificationLearner),
            Some(apps::AppKind::ClassificationLearner)
        );
    }
    #[test]
    fn classification_learner_other_presets_and_edited_options() {
        let ws = synthetic(true);
        let mut app = ClassificationLearner::default();
        for preset in [
            Preset::FineTree,
            Preset::CoarseTree,
            Preset::QuadraticDiscriminant,
            Preset::GaussianSvm,
            Preset::Logistic,
            Preset::Linear,
            Preset::BaggedTrees,
            Preset::BoostedTrees,
        ] {
            app.add(preset);
            let model = &mut app.models.last_mut().unwrap().model;
            if preset == Preset::GaussianSvm {
                model.scale = "1.5".into();
                model.parameter = "2".into();
            }
            if matches!(preset, Preset::BaggedTrees | Preset::BoostedTrees) {
                model.parameter = "3".into();
                model.rate = "0.2".into();
            }
        }
        app.spec.validation = Validation::Resubstitution;
        train(&mut app, &ws, true);
        for row in &app.models {
            assert!(
                row.error.is_none(),
                "{:?}: {:?}",
                row.model.preset,
                row.error
            );
            assert!(row.result.is_some());
        }
        for unsupported in [
            Preset::KernelBayes,
            Preset::QuadraticSvm,
            Preset::CubicSvm,
            Preset::CosineKnn,
            Preset::WeightedKnn,
            Preset::GaussianSvm,
        ] {
            assert!(Model::new(unsupported).fit("X", "Y").is_err());
        }
        let mut model = Model::new(Preset::FineTree);
        model.leaf = "1);error('bad');x=(1".into();
        assert!(model.fit("X", "Y").is_err());
        model = Model::new(Preset::LinearDiscriminant);
        model.parameter = "0.2".into();
        assert!(model.fit("X", "Y").unwrap().contains("'Gamma',0.2"));
    }

    fn painted(output: &egui::FullOutput, label: &str) -> bool {
        output
            .shapes
            .iter()
            .any(|s| text_position(&s.shape, label).is_some())
    }
}
