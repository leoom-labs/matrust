#[path = "../../tests/help_system.rs"]
mod basic_fitting_help_regressions;
#[path = "../../tests/math_functions.rs"]
mod basic_fitting_math_regressions;
#[path = "../../tests/polyval.rs"]
mod basic_fitting_polyval_regressions;
mod basic_fitting_tests {
    #[test]
    fn basic_fitting_findobj_reads_a_plotted_line() {
        let mut engine = matrust::Engine::default();
        engine
            .execute(
                "f=figure(7);plot(0:3,[1 2 5 10]);h=findobj(f,'Type','line');y=get(h(1),'YData');",
            )
            .unwrap();
        assert_eq!(engine.variables["y"].data(), &[1., 2., 5., 10.]);
    }
}

#[test]
fn basic_fitting_center_scale_engine_contract() {
    let mut engine = matrust::Engine::default();
    engine
        .execute(
            "[p,S,mu]=polyfit(10:14,[201 243 289 339 393],2);
        v=polyval(p,10:14,[],mu); r=S.normr;",
        )
        .unwrap();
    assert!(engine.variables["r"].data()[0] < 1e-10);
}

mod basic_fitting_app_tests {
    use super::*;
    use apps::basic_fitting::{BasicFitting, FitKind, ResidualStyle};
    use apps::{App, AppKind, Host, Snapshot};
    const DATA: &str = "x=(0:0.2:6)';y=1.5+0.8*x-0.12*x.^2+0.05*sin(7*x);figure(7);h=plot(x,y,'DisplayName','samples');";
    fn setup(code: &str) -> (matrust::Engine, Snapshot, Host<BasicFitting>) {
        let mut e = matrust::Engine::default();
        e.execute(code).unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&e.workspace());
        let mut host = Host::new(BasicFitting::default());
        host.app
            .sync_figures(&e.figures().map(|(n, f)| (n, f.clone())).collect());
        (e, ws, host)
    }
    fn fit(host: &mut Host<BasicFitting>, ws: &Snapshot) -> Result<(), String> {
        host.app.prepare(ws);
        let request = host.request(ws)?;
        let outputs = apps::run(&mut matrust::Engine::default(), request)?;
        host.app.receive(&outputs)
    }
    fn values(e: &matrust::Engine, n: &str) -> Vec<f64> {
        match e.array(n) {
            Some(matrust::Array::Numeric(m)) => m.data().to_vec(),
            other => panic!("{n}: {other:?}"),
        }
    }
    fn close(a: &[f64], b: &[f64], tol: f64) {
        assert_eq!(a.len(), b.len());
        for (a, b) in a.iter().zip(b) {
            assert!((a - b).abs() <= tol * (1. + b.abs()), "{a} != {b}");
        }
    }
    #[test]
    fn basic_fitting_scripts_preview_and_numpy_scipy_agree_for_all_fits() {
        let reference: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/basic_fitting/reference.json"
        ))
        .unwrap();
        let json_values = |value: &serde_json::Value| {
            value
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap())
                .collect::<Vec<_>>()
        };
        for normalize in [false, true] {
            let (_, ws, mut host) = setup(DATA);
            host.app.enabled = [true; 11];
            host.app.normalize = normalize;
            host.app.separate_residuals = true;
            host.app.evaluate = "[-1 0 0.5 3 6 7]".into();
            for selected in 0..11 {
                host.app.evaluate_fit = selected;
                fit(&mut host, &ws)
                    .unwrap_or_else(|err| panic!("{err}\n{}", host.app.script(&ws).unwrap()));
                let current = host.app.current.as_ref().unwrap();
                let script = host.app.script(&ws).unwrap();
                let mut e = matrust::Engine::default();
                e.execute(DATA).unwrap();
                e.execute(&script)
                    .unwrap_or_else(|err| panic!("{err}\n{script}"));
                let names = host.app.output_names(&ws).unwrap();
                for (i, result) in current.fits.iter().enumerate() {
                    e.execute(&format!(
                        "c_={}.coeff; n_={}.normresid;",
                        names[i], names[i]
                    ))
                    .unwrap();
                    close(&values(&e, "c_"), &result.coefficients, 0.);
                    close(&values(&e, "n_"), &[result.norm], 0.);
                    let key = match result.kind {
                        FitKind::Spline => "spline".into(),
                        FitKind::Pchip => "pchip".into(),
                        FitKind::Polynomial(d) => format!("poly{d}_{}", usize::from(normalize)),
                    };
                    let oracle = &reference["fits"][&key];
                    close(&[result.norm], &[oracle["norm"].as_f64().unwrap()], 1e-8);
                    if matches!(result.kind, FitKind::Polynomial(_)) {
                        close(&result.coefficients, &json_values(&oracle["coeff"]), 1e-7);
                    } else {
                        assert_eq!(result.breaks, 31);
                        assert_eq!(result.coefficient_count, 120);
                    }
                    if i == selected {
                        close(
                            &result.evaluated.iter().map(|p| p[1]).collect::<Vec<_>>(),
                            &json_values(&oracle["evaluated"]),
                            1e-7,
                        );
                        close(
                            &values(&e, "f1"),
                            &result.evaluated.iter().map(|p| p[1]).collect::<Vec<_>>(),
                            0.,
                        );
                    }
                }
                assert_eq!(current.data.len(), 31);
                assert_eq!(current.fits[selected].evaluated.len(), 6);
                assert!(
                    host.app
                        .results()
                        .iter()
                        .any(|(label, _)| *label == "norm(resid)")
                );
            }
        }
    }
    #[test]
    fn basic_fitting_applies_names_equations_residuals_and_evaluated_points() {
        for style in [
            ResidualStyle::Bar,
            ResidualStyle::Scatter,
            ResidualStyle::Line,
        ] {
            let (mut e, ws, mut host) = setup(DATA);
            host.app.enabled = [false; 11];
            for i in 0..5 {
                host.app.enabled[i] = true;
            }
            host.app.equations = true;
            host.app.normalize = true;
            host.app.residual_style = style;
            host.app.evaluate = "0:0.5:6".into();
            host.app.plot_evaluated = true;
            host.app.range_min = "-2".into();
            host.app.range_max = "8".into();
            fit(&mut host, &ws).unwrap();
            let script = host.app.script(&ws).unwrap();
            let Some(toolkits::Action::ExportScript(action)) = host.export(&ws) else {
                panic!("script action")
            };
            assert_eq!(
                action,
                apps::all_or_nothing(&script, &host.app.exported(&ws))
            );
            e.execute(&action)
                .unwrap_or_else(|err| panic!("{err}\n{action}"));
            let f = e.figures().find(|(n, _)| *n == 7).unwrap().1;
            assert_eq!(f.all_axes().count(), 2);
            let top = f
                .all_axes()
                .find(|(_, a)| a.series.iter().any(|s| s.label == "samples"))
                .unwrap()
                .1;
            for kind in &FitKind::ALL[..5] {
                assert!(top.series.iter().any(|s| s.label == kind.label()));
            }
            assert!(top.series.iter().any(|s| s.label == "evaluated points"));
            assert!(!top.extras.texts.is_empty());
            let curve = host.app.current.as_ref().unwrap().fits[0].curve.as_slice();
            assert_eq!(curve[0][0], -2.);
            assert_eq!(curve.last().unwrap()[0], 8.);
            // Apply has not replaced the source line or left its axes held.
            let source = top.series.iter().find(|s| s.label == "samples").unwrap();
            assert_eq!(source.points.len(), 31);
            assert!(!top.hold);
        }
    }
    #[test]
    fn basic_fitting_picks_second_line_and_tracks_figure_updates() {
        let (mut e, mut ws, mut host) = setup(&format!(
            "{DATA} hold on;h2=plot(x,2*x+3,'DisplayName','other');hold off;"
        ));
        host.app.selected_line = 1;
        host.app.residuals = false;
        host.app.enabled = [false; 11];
        host.app.enabled[2] = true;
        fit(&mut host, &ws).unwrap();
        close(
            &host.app.current.as_ref().unwrap().fits[0].coefficients,
            &[2., 3.],
            1e-12,
        );
        let key = host.request(&ws).unwrap().key;
        e.execute("set(h2,'YData',4*x-1,'DisplayName','changed');figure(8);plot([1 2],[7 9]);")
            .unwrap();
        ws.sync(&e.workspace());
        host.app
            .sync_figures(&e.figures().map(|(n, f)| (n, f.clone())).collect());
        assert!(
            host.app.lines[host.app.selected_line]
                .caption()
                .contains("changed")
        );
        fit(&mut host, &ws).unwrap();
        assert_ne!(host.request(&ws).unwrap().key, key);
        close(
            &host.app.current.as_ref().unwrap().fits[0].coefficients,
            &[4., -1.],
            1e-12,
        );
    }
    #[test]
    fn basic_fitting_workspace_data_and_evaluation_expression_inputs() {
        let (mut e, ws, mut host) = setup("x=0:6;y=2*x.^2+3*x+1;query=1:3;bf_x=29;");
        host.app.workspace_data = true;
        host.app.x = "x".into();
        host.app.y = "y".into();
        host.app.evaluate = "query/2".into();
        host.app.normalize = true;
        fit(&mut host, &ws).unwrap();
        assert!(host.app.inputs().contains(&"query".into()));
        e.execute(&host.app.script(&ws).unwrap()).unwrap();
        assert_eq!(values(&e, "bf_x"), vec![29.]);
        close(&values(&e, "f1"), &[3., 6., 10.], 1e-12);
        assert_eq!(e.figures().count(), 1);
    }
    #[test]
    fn basic_fitting_output_names_do_not_clobber_and_saved_text_is_identical() {
        let (mut e, mut ws, mut host) = setup(&format!("{DATA} fit=17;x1=19;f1=23;"));
        host.app.evaluate = "0:2".into();
        host.app.residuals = false;
        fit(&mut host, &ws).unwrap();
        let names = host.app.output_names(&ws).unwrap();
        assert_eq!(names, vec!["fit1", "x11", "f11"]);
        let script = host.app.script(&ws).unwrap();
        let Some(toolkits::Action::SaveAppScript(kind, saved)) = host.generate(&ws, 0) else {
            panic!("save action")
        };
        assert_eq!(kind, AppKind::BasicFitting);
        assert!(saved.ends_with(&script));
        e.execute(&saved).unwrap();
        assert_eq!(values(&e, "fit"), vec![17.]);
        assert_eq!(values(&e, "x1"), vec![19.]);
        assert_eq!(values(&e, "f1"), vec![23.]);
        // A name occupied after the snapshot is rejected at execution, too.
        let mut raced = matrust::Engine::default();
        raced
            .execute(&format!("{DATA} fit=17;x1=19;f1=23;fit1=99;"))
            .unwrap();
        assert!(
            raced
                .execute(&apps::all_or_nothing(&script, &names))
                .is_err()
        );
        assert_eq!(values(&raced, "fit1"), vec![99.]);
        assert!(raced.array("x11").is_none());
        ws.sync(&e.workspace());
        host.app.fit_names[3] = "fit".into();
        assert!(host.app.script(&ws).unwrap_err().contains("already used"));
        for reserved in ["mrexp_had", "mrexp_old", "matrust_export_error"] {
            host.app.fit_names[3] = reserved.into();
            assert!(host.app.script(&ws).unwrap_err().contains("reserved"));
        }
        host.app.fit_names[3] = "custom".into();
        host.app.evaluated_x_name = "custom".into();
        assert!(host.app.script(&ws).is_err());
    }
    #[test]
    fn basic_fitting_errors_keep_last_good_preview_and_reach_status_line() {
        let (mut e, mut ws, mut host) = setup(DATA);
        fit(&mut host, &ws).unwrap();
        let good = host.app.current.as_ref().unwrap().fits[0]
            .coefficients
            .clone();
        e.execute("set(h,'YData',y*NaN);").unwrap();
        ws.sync(&e.workspace());
        host.app
            .sync_figures(&e.figures().map(|(n, f)| (n, f.clone())).collect());
        assert!(fit(&mut host, &ws).unwrap_err().contains("NaN"));
        assert_eq!(
            host.app.current.as_ref().unwrap().fits[0].coefficients,
            good
        );
        let ctx = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            host.refresh(&ctx, &ws);
            if !host.running() {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(host.status().0.contains("NaN"));
        assert_eq!(host.status().1, toolkits::AppStatus::Error);
        let (_, short_ws, mut short) = setup("figure(2);plot(1:3,[2 3 4]);");
        short.app.enabled = [false; 11];
        short.app.enabled[10] = true;
        assert!(fit(&mut short, &short_ws).unwrap_err().contains("too few"));
        short.app.evaluate = "zeros(2001,1)".into();
        assert!(fit(&mut short, &short_ws).unwrap_err().contains("2000"));
        for bad in ["[1 2];clear x", "[1 2", "1:2\nclear x"] {
            short.app.evaluate = bad.into();
            assert!(short.app.script(&short_ws).is_err());
        }
    }
    #[test]
    fn basic_fitting_multi_axes_apply_preserves_neighbour_and_private_preview() {
        let (mut e, ws, mut host) = setup(
            "figure(7);subplot(1,2,1);plot(0:5,1:6,'DisplayName','left');subplot(1,2,2);plot(0:5,(0:5).^2,'DisplayName','right');",
        );
        host.app.selected_line = 1;
        let before = e.figures().find(|(n, _)| *n == 7).unwrap().1.clone();
        fit(&mut host, &ws).unwrap();
        assert!(before.same_snapshot(e.figures().find(|(n, _)| *n == 7).unwrap().1));
        e.execute(&host.app.script(&ws).unwrap()).unwrap();
        let after = e.figures().find(|(n, _)| *n == 7).unwrap().1;
        assert_eq!(after.all_axes().count(), 3);
        let left = after
            .all_axes()
            .find(|(_, a)| a.series.iter().any(|s| s.label == "left"))
            .unwrap()
            .1;
        assert_eq!(left.series.len(), 1);
        assert_eq!(left.series[0].points.get(0), Some([0., 1.]));
    }

    #[test]
    fn basic_fitting_tab_renders_and_apply_runs_in_workspace() {
        let size = egui::vec2(1440., 900.);
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.run(DATA.into());
        finish(&mut app);
        app.toolkits.apps.basic.app.residuals = false;
        app.toolkits
            .open_tool(toolkits::Tool::App(AppKind::BasicFitting));
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            frame_at(&mut app, &ctx, size, vec![]);
            let h = &app.toolkits.apps.basic;
            if h.app.current.is_some() && !h.running() {
                break;
            }
            assert!(Instant::now() < deadline, "{:?}", h.status());
            std::thread::sleep(Duration::from_millis(5));
        }
        let output = frame_at(&mut app, &ctx, size, vec![]);
        for label in [
            "INPUTS",
            "PREVIEW",
            "OUTPUT",
            "Apply to figure",
            "Generate script",
            "Evaluate at x",
        ] {
            assert!(
                output
                    .shapes
                    .iter()
                    .any(|s| text_position(&s.shape, label).is_some()),
                "{label}"
            );
        }
        click_label_at(&mut app, &ctx, size, "Apply to figure");
        finish(&mut app);
        frame_at(&mut app, &ctx, size, vec![]);
        let f = &app.all_figures[&7];
        assert!(
            f.series.iter().any(|s| s.label == "quadratic"),
            "{}",
            app.status
        );
        // Applying reveals the changed source figure. Select the still-open app
        // again before checking its compact layout.
        app.toolkits
            .select_tool(toolkits::Tool::App(AppKind::BasicFitting));
        let narrow = frame_at(&mut app, &ctx, egui::vec2(900., 700.), vec![]);
        assert!(
            narrow
                .shapes
                .iter()
                .any(|s| text_position(&s.shape, "INPUTS").is_some())
        );
    }
    #[test]
    fn basic_fitting_rejects_evaluation_statement_injection() {
        let (_, ws, mut host) = setup(DATA);
        for payload in [
            "1), clear victim, disp(1",
            "1); injected=123; (1",
            "1 % swallowed",
        ] {
            host.app.evaluate = payload.into();
            assert!(host.app.script(&ws).is_err(), "accepted {payload}");
        }
        for expression in ["[1; 2; 3]", "linspace(0, 1, 3)", "[1 2]'", "sum([1;2])"] {
            host.app.evaluate = expression.into();
            fit(&mut host, &ws).unwrap();
        }
    }

    #[test]
    fn basic_fitting_repeated_apply_replaces_only_owned_graphics() {
        for style in [
            ResidualStyle::Bar,
            ResidualStyle::Scatter,
            ResidualStyle::Line,
        ] {
            let (mut e, mut ws, mut host) = setup(&format!(
                "{DATA} hold on; foreign=plot(x,3*x,'DisplayName','foreign'); text(1,2,'keep me'); hold off;"
            ));
            host.app.residual_style = style;
            let original = e
                .figures()
                .find(|(n, _)| *n == 7)
                .unwrap()
                .1
                .normalized_axes_positions()[0]
                .1;
            host.app.equations = true;
            host.app.evaluate = "[1 2]".into();
            host.app.plot_evaluated = true;
            let mut expected = None;
            for _ in 0..3 {
                fit(&mut host, &ws).unwrap();
                e.execute(&host.app.script(&ws).unwrap()).unwrap();
                let f = e.figures().find(|(n, _)| *n == 7).unwrap().1;
                let top = f
                    .all_axes()
                    .find(|(_, a)| a.series.iter().any(|s| s.label == "samples"))
                    .unwrap()
                    .1;
                let actual = (
                    f.all_axes().count(),
                    top.series.len(),
                    top.extras.texts.len(),
                    f.normalized_axes_positions(),
                );
                if let Some(expected) = &expected {
                    assert_eq!(&actual, expected);
                } else {
                    expected = Some(actual);
                }
                assert!(top.series.iter().any(|s| s.label == "foreign"));
                assert!(top.extras.texts.iter().any(|t| t.string == "keep me"));
                e.execute("assert(isgraphics(h)); assert(isgraphics(foreign));")
                    .unwrap();
                ws.sync(&e.workspace());
                host.app
                    .sync_figures(&e.figures().map(|(n, f)| (n, f.clone())).collect());
            }
            host.app.residuals = false;
            fit(&mut host, &ws).unwrap();
            e.execute(&host.app.script(&ws).unwrap()).unwrap();
            let f = e.figures().find(|(n, _)| *n == 7).unwrap().1;
            assert_eq!(f.all_axes().count(), 1);
            close(&f.normalized_axes_positions()[0].1, &original, 1e-12);
        }
    }

    #[test]
    fn basic_fitting_reapply_preserves_user_graphics_in_residual_axes() {
        let (mut e, mut ws, mut host) = setup(DATA);
        fit(&mut host, &ws).unwrap();
        e.execute(&host.app.script(&ws).unwrap()).unwrap();
        e.execute("manual_residual=line([0 6],[0.1 0.1],'DisplayName','manual residual');")
            .unwrap();
        for _ in 0..2 {
            ws.sync(&e.workspace());
            host.app
                .sync_figures(&e.figures().map(|(n, f)| (n, f.clone())).collect());
            fit(&mut host, &ws).unwrap();
            e.execute(&host.app.script(&ws).unwrap()).unwrap();
            e.execute("assert(isgraphics(manual_residual));").unwrap();
            let f = e.figures().find(|(n, _)| *n == 7).unwrap().1;
            assert_eq!(f.all_axes().count(), 2);
            assert!(
                f.all_axes()
                    .any(|(_, a)| a.series.iter().any(|s| s.label == "manual residual"))
            );
        }
    }

    #[test]
    fn basic_fitting_delete_owned_text_and_residual_axes_preserves_source() {
        let mut e = matrust::Engine::default();
        e.execute("figure(7); subplot(2,1,1); source=plot([1 2],[3 4]); label=text(1,3,'owned'); subplot(2,1,2); b=gca; scatter([1 2],[0 1]);").unwrap();
        e.execute("delete(label); delete(b); alive=isgraphics(source); gone=isgraphics(b);")
            .unwrap();
        assert_eq!(e.logical_variables["alive"].data(), &[1]);
        assert_eq!(e.logical_variables["gone"].data(), &[0]);
        let f = e.figures().find(|(n, _)| *n == 7).unwrap().1;
        assert_eq!(f.all_axes().count(), 1);
        assert!(f.extras.texts.is_empty());
        assert_eq!(f.series.len(), 1);
    }
}

#[test]
fn basic_fitting_release_benchmark() {
    if std::env::var_os("BASIC_FITTING_BENCH").is_none() {
        return;
    }
    let n = std::thread::available_parallelism()
        .map_or(2, |n| n.get())
        .min(8);
    for workers in [1, n] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            let mut e = matrust::Engine::default();
            e.execute("x=linspace(1000,2000,20000);y=0.2*x.^2-3*x+sin(x);")
                .unwrap();
            let script = "[p,S,mu]=polyfit(x,y,9);v=polyval(p,x,[],mu);";
            for _ in 0..2 {
                e.execute(script).unwrap();
            }
            let start = std::time::Instant::now();
            for _ in 0..10 {
                std::hint::black_box(e.execute(script).unwrap());
            }
            eprintln!(
                "BASIC_FITTING_BENCH workers={workers} points=20000 degree=9 mean_ms={:.3}",
                start.elapsed().as_secs_f64() * 100.
            );
        });
    }
}

#[test]
fn basic_fitting_findobj_filters_scopes_and_rejects_unsupported_syntax() {
    let mut e = matrust::Engine::default();
    e.execute("f=figure(3);subplot(1,2,1);a=gca;h=plot(0:2,[2 4 6],'DisplayName','left');subplot(1,2,2);b=gca;k=plot(0:2,[1 4 9],'DisplayName','right');figure(8);plot(1:3);one=findobj(f,'Type','line','DisplayName','left');two=findobj(b,'Type','line');n1=numel(one);n2=numel(two);alln=numel(findobj(0,'Type','line'));axn=numel(findobj(f,'Type','axes'));fn=numel(findobj('Type','figure','Number',3));").unwrap();
    for (name, value) in [
        ("n1", 1.),
        ("n2", 1.),
        ("alln", 3.),
        ("axn", 2.),
        ("fn", 1.),
    ] {
        assert_eq!(e.variables[name].data(), &[value]);
    }
    e.execute("v=get(one(1),'YData');set(one(1),'DisplayName','renamed');")
        .unwrap();
    assert_eq!(e.variables["v"].data(), &[2., 4., 6.]);
    for source in [
        "z=findobj(f,'-regexp','Type','line');",
        "z=findobj(f,'Type','surface');",
        "z=findobj(f);",
        "z=findobj(0,'Number',3);",
    ] {
        assert!(e.execute(source).is_err(), "{source}");
    }
    e.execute("close(f);").unwrap();
    assert!(e.execute("z=findobj(f,'Type','line');").is_err());
    let cancel = e.cancellation();
    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(
        e.execute("z=findobj(0,'Type','figure');")
            .unwrap_err()
            .contains("cancel")
    );
}
