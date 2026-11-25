// Curve Fitter app (apps/curve_fitter.rs): the generated script reproduces
// the coefficients and goodness the app shows for library, spline, custom and
// surface models; Exclude, Weights and bounds reach the script; table data;
// export names; errors; the fits list and a rendered tab.
mod curve_fitter_tests {
    use super::*;
    use apps::curve_fitter::{CurveFitter, Family, FitResult, Robust};
    use apps::{App, AppKind, Host, Snapshot};
    use toolkits::Tool;

    /// 31 curve points; a 7 x 6 grid of surface points.
    const DATA: &str = "x = (0:0.2:6)'; \
        y = 1.5 + 0.8*x - 0.12*x.^2 + 0.05*sin(7*x); \
        ye = 2.5*exp(-0.6*x) + 0.4 + 0.02*cos(9*x); \
        yg = 3*exp(-((x - 2.5)/0.9).^2) + 0.2 + 0.03*sin(11*x); \
        w = 1 + mod(0:30, 3)'; yshort = y(1:30); \
        xs = repmat((0:0.5:3)', 6, 1); ys = kron((0:0.4:2)', ones(7, 1)); \
        zs = 1 + 2*xs - 0.7*ys + 0.05*sin(5*xs.*ys);";

    fn workspace(setup: &str) -> Snapshot {
        let mut engine = matrust::Engine::default();
        engine.execute(setup).unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        ws
    }
    /// A curve fitter on `x` against `y`.
    fn curve(y: &str) -> Host<CurveFitter> {
        let mut host = Host::new(CurveFitter::default());
        let spec = &mut host.app.fit_mut().spec;
        spec.x = "x".into();
        spec.y = y.into();
        host
    }
    /// The edited model's coefficient query, synchronously.
    fn coefficients(host: &mut Host<CurveFitter>) {
        if let Some((model, request)) = host.app.coefficient_request() {
            let result = apps::run(&mut matrust::Engine::default(), request);
            host.app.receive_coefficients(model, &result);
        }
    }
    /// One private run, synchronously, delivered to the app.
    fn fit(host: &mut Host<CurveFitter>, ws: &Snapshot) -> Result<(), String> {
        host.app.prepare(ws);
        coefficients(host);
        let request = host.request(ws)?;
        let outputs = apps::run(&mut matrust::Engine::default(), request)?;
        host.app.receive(&outputs)
    }
    fn shown(host: &Host<CurveFitter>) -> &FitResult {
        host.app.fit().result.as_ref().expect("a fit")
    }
    fn values(engine: &matrust::Engine, name: &str) -> Vec<f64> {
        match engine.array(name) {
            Some(matrust::Array::Numeric(m)) => m.data().to_vec(),
            other => panic!("{name}: {other:?}"),
        }
    }
    /// Equal, NaN included.
    fn same(a: &[f64], b: &[f64]) -> bool {
        a.len() == b.len()
            && a.iter()
                .zip(b)
                .all(|(a, b)| a == b || (a.is_nan() && b.is_nan()))
    }
    /// `script` run in a fresh Engine on the test data.
    fn run_script(script: &str) -> matrust::Engine {
        let mut engine = matrust::Engine::default();
        engine.execute(DATA).unwrap();
        engine
            .execute(script)
            .unwrap_or_else(|e| panic!("{script}\n{e}"));
        engine
    }
    /// The goodness the app shows, in the order of `gof_of`.
    fn gof(r: &FitResult) -> Vec<f64> {
        vec![r.sse, r.rsquare, r.adjrsquare, r.rmse, r.dfe]
    }
    fn gof_of(engine: &mut matrust::Engine, gof: &str) -> Vec<f64> {
        engine
            .execute(&format!(
                "g_ = [{gof}.sse {gof}.rsquare {gof}.adjrsquare {gof}.rmse {gof}.dfe];"
            ))
            .unwrap();
        values(engine, "g_")
    }
    /// The script reproduces the shown fit: coefficients (when the model
    /// has them), goodness and the values at the data.
    fn assert_reproduces(host: &Host<CurveFitter>, ws: &Snapshot) -> String {
        let script = host.app.script(ws).unwrap();
        let r = shown(host);
        let mut engine = run_script(&script);
        assert!(same(&gof_of(&mut engine, "goodness"), &gof(r)), "{script}");
        if !r.coefficients.is_empty() {
            engine.execute("c_ = coeffvalues(fittedmodel);").unwrap();
            let c: Vec<f64> = r.coefficients.iter().map(|c| c.value).collect();
            assert_eq!(values(&engine, "c_"), c, "{script}");
        }
        let at = if r.z.is_some() {
            "v_ = fittedmodel(xs, ys);"
        } else {
            "v_ = fittedmodel(x);"
        };
        engine.execute(at).unwrap();
        assert!(same(&values(&engine, "v_"), &r.fitted), "{script}");
        // The Output block shows that goodness.
        let rows = host.app.results();
        assert!(rows.contains(&("SSE", apps::short(r.sse))), "{rows:?}");
        assert!(rows.contains(&("R-square", apps::short(r.rsquare))));
        script
    }

    #[test]
    fn curve_fitter_generated_script_reproduces_the_fit_shown() {
        let ws = workspace(DATA);
        // poly2: the plainest script.
        let mut host = curve("y");
        host.app.fit_mut().spec.terms = 2;
        fit(&mut host, &ws).unwrap();
        let script = assert_reproduces(&host, &ws);
        assert_eq!(
            script,
            "% Fit 1: y against x\nft = fittype('poly2');\n\
             [fittedmodel, goodness] = fit(x, y, ft);\nplot(fittedmodel, x, y)\n"
        );
        let r = shown(&host);
        assert_eq!(r.type_name, "poly2");
        assert_eq!(r.formula, "p1*x^2 + p2*x + p3");
        let names: Vec<&str> = r.coefficients.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["p1", "p2", "p3"]);
        // Confidence bounds bracket each coefficient; the true curve is
        // 1.5 + 0.8 x - 0.12 x^2 plus a small ripple.
        for (c, truth) in r.coefficients.iter().zip([-0.12, 0.8, 1.5]) {
            assert!(c.lower < c.value && c.value < c.upper, "{c:?}");
            assert!((c.value - truth).abs() < 0.05, "{c:?}");
        }
        assert_eq!(r.dfe, 28.);
        assert_eq!(r.curve.len(), 400);
        assert!(r.warning.is_none());
        assert!(
            host.app
                .summary()
                .unwrap()
                .starts_with("Fit 1: poly2 · R² "),
            "{:?}",
            host.app.summary()
        );
        // exp1 and gauss1: nonlinear library models.
        let mut host = curve("ye");
        host.app.fit_mut().spec.set_family(Family::Exponential);
        fit(&mut host, &ws).unwrap();
        assert_reproduces(&host, &ws);
        assert_eq!(shown(&host).type_name, "exp1");
        let mut host = curve("yg");
        host.app.fit_mut().spec.set_family(Family::Gaussian);
        fit(&mut host, &ws).unwrap();
        let script = assert_reproduces(&host, &ws);
        assert!(script.contains("ft = fittype('gauss1');"), "{script}");
        let b1 = &shown(&host).coefficients[1];
        assert_eq!(b1.name, "b1");
        assert!((b1.value - 2.5).abs() < 0.05, "{b1:?}");
        // Smoothing spline: the default p comes from fit's output, an explicit
        // p is in the script.
        let mut host = curve("y");
        host.app.fit_mut().spec.set_family(Family::SmoothingSpline);
        fit(&mut host, &ws).unwrap();
        let script = assert_reproduces(&host, &ws);
        assert!(!script.contains("SmoothingParam"), "{script}");
        let p = shown(&host).smoothing.unwrap();
        assert!(p > 0. && p < 1., "{p}");
        assert!(shown(&host).coefficients.is_empty());
        assert!(
            host.app
                .results()
                .contains(&("Smoothing p", apps::short(p)))
        );
        let spec = &mut host.app.fit_mut().spec;
        spec.default_smoothing = false;
        spec.smoothing = 0.9;
        fit(&mut host, &ws).unwrap();
        let script = assert_reproduces(&host, &ws);
        assert!(
            script.contains("opts = fitoptions(ft);\nopts.SmoothingParam = 0.9;\n")
                && script.contains("fit(x, y, ft, opts)"),
            "{script}"
        );
        assert_eq!(shown(&host).smoothing, Some(0.9));
        // Custom equation: the coefficient list is coeffnames(fittype(expr));
        // without a start point the engine warns and the app says so.
        let mut host = curve("ye");
        host.app.fit_mut().spec.set_family(Family::Custom);
        host.app.fit_mut().spec.equation = "a*exp(-b*x)+c".into();
        fit(&mut host, &ws).unwrap();
        let script = assert_reproduces(&host, &ws);
        assert!(script.contains("fittype('a*exp(-b*x)+c')"), "{script}");
        let info = host.app.coefficient_info(&host.app.fit().spec).cloned();
        let info = info.unwrap().unwrap();
        assert_eq!(info.names, ["a", "b", "c"]);
        assert!(info.nonlinear);
        let warning = shown(&host).warning.clone().unwrap();
        assert!(warning.contains("Start point not provided"), "{warning}");
        assert!(host.app.summary().unwrap().contains("Warning: Start point"));
        let b = shown(&host).coefficients[1].value;
        assert!((b - 0.6).abs() < 0.01, "{b}");
        // A poly11 surface of zs on (xs, ys), with its image grid.
        let mut host = Host::new(CurveFitter::default());
        let spec = &mut host.app.fit_mut().spec;
        (spec.x, spec.y, spec.z) = ("xs".into(), "ys".into(), "zs".into());
        spec.set_family(Family::Exponential);
        fit(&mut host, &ws).unwrap();
        assert_eq!(
            host.app.fit().spec.family,
            Family::Polynomial,
            "surfaces fit polynomials only"
        );
        let script = assert_reproduces(&host, &ws);
        assert!(
            script.contains("ft = fittype('poly11');")
                && script.contains("fit([xs, ys], zs, ft)")
                && script.contains("plot(fittedmodel, [xs, ys], zs)")
                && script.starts_with("% Fit 1: zs against xs and ys\n"),
            "{script}"
        );
        let r = shown(&host);
        assert_eq!(r.formula, "p00 + p10*x + p01*y");
        let grid = r.grid.as_ref().unwrap();
        for (got, want) in [grid.x.0, grid.x.1, grid.y.0, grid.y.1]
            .iter()
            .zip([0., 3., 0., 2.])
        {
            assert!((got - want).abs() < 1e-12, "{:?} {:?}", grid.x, grid.y);
        }
        assert_eq!(grid.values.len(), apps::curve_fitter::GRID.pow(2));
        // Row-major from the lowest y: the first value is f(0, 0), the last f(3, 2).
        let c: Vec<f64> = r.coefficients.iter().map(|c| c.value).collect();
        assert!((grid.values[0] - c[0]).abs() < 1e-12);
        let last = c[0] + 3. * c[1] + 2. * c[2];
        assert!((grid.values.last().unwrap() - last).abs() < 1e-9);
        // x degree 2, y degree 0 is poly20.
        let spec = &mut host.app.fit_mut().spec;
        (spec.x_degree, spec.y_degree) = (2, 0);
        fit(&mut host, &ws).unwrap();
        assert_eq!(shown(&host).type_name, "poly20");
        assert_reproduces(&host, &ws);
    }

    #[test]
    fn curve_fitter_exclude_and_weights_reach_the_script_and_change_the_fit() {
        let ws = workspace(DATA);
        let mut host = curve("y");
        host.app.fit_mut().spec.terms = 2;
        fit(&mut host, &ws).unwrap();
        let all = shown(&host).clone();
        // Clicked points: 1-based indices in the script.
        host.app.toggle_point(2);
        host.app.toggle_point(9);
        host.app.toggle_point(5);
        host.app.toggle_point(5);
        fit(&mut host, &ws).unwrap();
        let script = assert_reproduces(&host, &ws);
        assert!(
            script.contains("opts = fitoptions(ft);\nopts.Exclude = [3 10];\n"),
            "{script}"
        );
        let r = shown(&host);
        assert_eq!(r.dfe, all.dfe - 2.);
        assert_ne!(r.sse, all.sse);
        assert_ne!(r.coefficients[0].value, all.coefficients[0].value);
        // Excluded points stay in the data and the residuals.
        assert_eq!((r.x.len(), r.residuals.len()), (31, 31));
        // The x range: x < 0.5 drops 0, 0.2 and 0.4 (0.4 is clicked too).
        host.app.fit_mut().spec.x_min = "0.5".into();
        fit(&mut host, &ws).unwrap();
        let script = assert_reproduces(&host, &ws);
        assert!(
            script.contains("opts.Exclude = x < 0.5;\nopts.Exclude([3 10]) = true;\n"),
            "{script}"
        );
        assert_eq!(shown(&host).dfe, 31. - 4. - 3.);
        let mask = host.app.fit().spec.excluded_mask(&shown(&host).x);
        assert_eq!(mask.iter().filter(|m| **m).count(), 4);
        host.app.fit_mut().spec.x_max = "5".into();
        host.app.fit_mut().spec.excluded.clear();
        fit(&mut host, &ws).unwrap();
        let script = assert_reproduces(&host, &ws);
        assert!(
            script.contains("opts.Exclude = x < 0.5 | x > 5;\n"),
            "{script}"
        );
        host.app.fit_mut().spec.x_max = "0.1".into();
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("lower x limit is above"), "{error}");
        host.app.fit_mut().spec.x_max = "five".into();
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("upper x limit must be a number"), "{error}");
        (host.app.fit_mut().spec.x_min, host.app.fit_mut().spec.x_max) =
            (String::new(), String::new());
        // Weights: the variable is named in the script and changes the fit.
        host.app.fit_mut().spec.weights = "w".into();
        fit(&mut host, &ws).unwrap();
        let script = assert_reproduces(&host, &ws);
        assert!(
            script.starts_with("% Fit 1: y against x, weighted by w\n")
                && script.contains("opts.Weights = w;\n"),
            "{script}"
        );
        assert_eq!(host.app.inputs(), ["x", "y", "w"]);
        assert_ne!(
            shown(&host).coefficients[0].value,
            all.coefficients[0].value
        );
        // Robust and Normalize are fit options too.
        host.app.fit_mut().spec.robust = Robust::Bisquare;
        host.app.fit_mut().spec.normalize = true;
        fit(&mut host, &ws).unwrap();
        let script = assert_reproduces(&host, &ws);
        assert!(
            script.contains("opts.Normalize = 'on';\nopts.Robust = 'Bisquare';\n"),
            "{script}"
        );
        // Other data: the clicked points no longer mean anything.
        host.app.toggle_point(4);
        host.app.fit_mut().spec.y = "ye".into();
        host.app.prepare(&ws);
        assert!(host.app.fit().spec.excluded.is_empty());
    }

    #[test]
    fn curve_fitter_nonlinear_bounds_and_start_points() {
        let ws = workspace(DATA);
        let mut host = curve("ye");
        host.app.fit_mut().spec.set_family(Family::Exponential);
        fit(&mut host, &ws).unwrap();
        // exp1 has no offset, so the free rate is far above the true -0.6.
        let free = shown(&host).coefficients[1].value;
        assert!(free > -0.5, "{free}");
        // An upper bound below the free optimum is active; Lower stays default.
        host.app
            .fit_mut()
            .spec
            .bounds
            .insert("b".into(), ["".into(), "".into(), "-0.7".into()]);
        fit(&mut host, &ws).unwrap();
        let script = assert_reproduces(&host, &ws);
        assert!(script.contains("opts.Upper = [Inf -0.7];\n"), "{script}");
        assert!(!script.contains("Lower"), "{script}");
        assert!((shown(&host).coefficients[1].value + 0.7).abs() < 1e-6);
        // A start point needs every coefficient.
        host.app
            .fit_mut()
            .spec
            .bounds
            .insert("a".into(), ["2".into(), "".into(), "".into()]);
        let error = host.app.script(&ws).unwrap_err();
        assert!(
            error.contains("start value for every coefficient"),
            "{error}"
        );
        host.app.fit_mut().spec.bounds.get_mut("b").unwrap()[0] = "-0.8".into();
        fit(&mut host, &ws).unwrap();
        let script = assert_reproduces(&host, &ws);
        assert!(
            script.contains("opts.StartPoint = [2 -0.8];\nopts.Upper = [Inf -0.7];\n"),
            "{script}"
        );
        host.app.fit_mut().spec.bounds.get_mut("a").unwrap()[1] = "low".into();
        let error = host.app.script(&ws).unwrap_err();
        assert!(
            error.contains("lower bound of a must be a number"),
            "{error}"
        );
        // gauss1: a blank bound keeps the library default (c1 >= 0).
        let mut host = curve("yg");
        host.app.fit_mut().spec.set_family(Family::Gaussian);
        coefficients(&mut host);
        let info = host.app.coefficient_info(&host.app.fit().spec).cloned();
        let info = info.unwrap().unwrap();
        assert_eq!(info.names, ["a1", "b1", "c1"]);
        assert_eq!(info.lower, [f64::NEG_INFINITY, f64::NEG_INFINITY, 0.]);
        let bounds = &mut host.app.fit_mut().spec.bounds;
        bounds.insert("b1".into(), ["".into(), "2.6".into(), "".into()]);
        bounds.insert("a1".into(), ["".into(), "".into(), "Inf".into()]);
        fit(&mut host, &ws).unwrap();
        let script = assert_reproduces(&host, &ws);
        assert!(
            script.contains("opts.Lower = [-Inf 2.6 0];\nopts.Upper = [Inf Inf Inf];\n"),
            "{script}"
        );
        assert!((shown(&host).coefficients[1].value - 2.6).abs() < 1e-6);
        // Polynomials and splines have no bounds table: bounds never reach them.
        host.app.fit_mut().spec.set_family(Family::Polynomial);
        let script = host.app.script(&ws).unwrap();
        assert!(
            !script.contains("Lower") && !script.contains("opts"),
            "{script}"
        );
    }

    #[test]
    fn curve_fitter_table_columns_as_data() {
        let ws = workspace(&format!(
            "{DATA} T = table(x, y, 'VariableNames', {{'speed', 'power'}}); n = 4;"
        ));
        let listed = apps::curve_fitter::vectors(&ws);
        assert!(listed.contains(&("x".into(), 31)) && listed.contains(&("zs".into(), 42)));
        assert!(!listed.iter().any(|(name, _)| name == "T" || name == "n"));
        assert_eq!(apps::curve_fitter::tables(&ws), [("T".to_owned(), 31)]);
        assert_eq!(
            apps::curve_fitter::table_columns(&ws, "T"),
            [("speed".to_owned(), 31), ("power".to_owned(), 31)]
        );
        let mut host = Host::new(CurveFitter::default());
        let spec = &mut host.app.fit_mut().spec;
        spec.use_table = true;
        spec.table = "T".into();
        (spec.x, spec.y, spec.terms) = ("speed".into(), "power".into(), 2);
        let script = host.app.script(&ws).unwrap();
        assert_eq!(
            script,
            "% Fit 1: T.power against T.speed\nft = fittype('poly2');\n\
             [fittedmodel, goodness] = fit(T.speed, T.power, ft);\n\
             plot(fittedmodel, T.speed, T.power)\n"
        );
        assert_eq!(host.app.inputs(), ["T"]);
        fit(&mut host, &ws).unwrap();
        let from_table = shown(&host).clone();
        // The same numbers as the vectors x and y.
        let mut plain = curve("y");
        plain.app.fit_mut().spec.terms = 2;
        fit(&mut plain, &ws).unwrap();
        assert_eq!(
            from_table.coefficients,
            shown(&plain).coefficients,
            "table columns fit like the vectors"
        );
        let mut engine = matrust::Engine::default();
        engine
            .execute(&format!(
                "{DATA} T = table(x, y, 'VariableNames', {{'speed', 'power'}});"
            ))
            .unwrap();
        engine.execute(&script).unwrap();
        assert_eq!(gof_of(&mut engine, "goodness"), gof(&from_table));
        // Exclude by x range uses the column.
        host.app.fit_mut().spec.x_max = "4".into();
        let script = host.app.script(&ws).unwrap();
        assert!(script.contains("opts.Exclude = T.speed > 4;"), "{script}");
    }

    #[test]
    fn curve_fitter_default_names_do_not_clobber_and_names_are_checked() {
        let ws = workspace(&format!("{DATA} fittedmodel = 7; goodness = 1; ft = 2;"));
        let mut host = curve("y");
        assert_eq!(
            host.app.output_names(&ws).unwrap(),
            ("fittedmodel1".into(), "goodness1".into())
        );
        host.app.fit_mut().spec.robust = Robust::Lar;
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.contains("ft1 = fittype('poly1');\nopts = fitoptions(ft1);\n")
                && script.contains("[fittedmodel1, goodness1] = fit(x, y, ft1, opts);"),
            "{script}"
        );
        assert_eq!(
            host.app.exported(&ws),
            ["fittedmodel1", "goodness1", "ft1", "opts"]
        );
        // User names are checked.
        let entry = host.app.fit_mut();
        (entry.fit_name, entry.fit_edited) = ("x".into(), true);
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("is a data variable"), "{error}");
        let entry = host.app.fit_mut();
        (entry.fit_name, entry.gof_name, entry.gof_edited) = ("m".into(), "m".into(), true);
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("different names"), "{error}");
        host.app.fit_mut().gof_name = "2bad".into();
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("not a valid variable name"), "{error}");
    }

    #[test]
    fn curve_fitter_errors_show_the_engine_message_and_keep_the_last_fit() {
        let ws = workspace(DATA);
        let host = Host::new(CurveFitter::default());
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("choose X and Y"), "{error}");
        let mut host = curve("ye");
        fit(&mut host, &ws).unwrap();
        let good = shown(&host).coefficients.clone();
        // A custom equation that does not parse: the engine's message, from the
        // coefficient query and from the fit; the plots keep the last fit.
        host.app.fit_mut().spec.set_family(Family::Custom);
        host.app.fit_mut().spec.equation = "a*exp(-b*x".into();
        let error = fit(&mut host, &ws).unwrap_err();
        assert!(
            error.contains("not a library model name or a valid equation"),
            "{error}"
        );
        let info = host.app.coefficient_info(&host.app.fit().spec).cloned();
        assert!(info.unwrap().unwrap_err().contains("valid equation"));
        assert_eq!(shown(&host).coefficients, good);
        host.app.fit_mut().spec.equation = "  ".into();
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("type a custom equation"), "{error}");
        // Mismatched lengths: fit's own message.
        host.app.fit_mut().spec.set_family(Family::Polynomial);
        host.app.fit_mut().spec.y = "yshort".into();
        let error = fit(&mut host, &ws).unwrap_err();
        assert!(
            error.contains("X and Y must have the same number of rows"),
            "{error}"
        );
        // A missing variable never reaches the engine.
        host.app.fit_mut().spec.y = "gone".into();
        host.app.prepare(&ws);
        let error = host.request(&ws).err().unwrap();
        assert!(error.contains("gone does not exist"), "{error}");
        // Through the threaded runner, the status line carries the message.
        let ctx = egui::Context::default();
        host.app.fit_mut().spec.y = "yshort".into();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            host.refresh(&ctx, &ws);
            if !host.running() {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        let (text, status) = host.status();
        assert_eq!(status, toolkits::AppStatus::Error);
        assert!(text.contains("same number of rows"), "{text}");
        assert_eq!(shown(&host).coefficients, good);
    }

    #[test]
    fn curve_fitter_fits_list_new_duplicate_delete() {
        let ws = workspace(DATA);
        let mut host = curve("y");
        host.app.fit_mut().spec.terms = 2;
        fit(&mut host, &ws).unwrap();
        let first = shown(&host).clone();
        let key = host.request(&ws).unwrap().key;
        // Duplicate: same inputs and result, its own name and run.
        host.app.duplicate();
        assert_eq!((host.app.fits.len(), host.app.selected), (2, 1));
        assert_eq!(host.app.fit().name, "Fit 2");
        assert_eq!(host.app.fits[1].spec, host.app.fits[0].spec);
        assert_eq!(shown(&host).coefficients, first.coefficients);
        assert_ne!(host.request(&ws).unwrap().key, key);
        host.app.fit_mut().spec.terms = 3;
        fit(&mut host, &ws).unwrap();
        let script = assert_reproduces(&host, &ws);
        assert!(script.starts_with("% Fit 2: y against x\nft = fittype('poly3');"));
        assert_eq!(shown(&host).type_name, "poly3");
        assert!(shown(&host).sse < first.sse);
        // Each fit keeps its own inputs and result.
        host.app.select(0);
        assert_eq!(shown(&host).type_name, "poly2");
        assert!(host.app.script(&ws).unwrap().contains("'poly2'"));
        // New fit: the data, a default model.
        host.app.new_fit();
        assert_eq!(host.app.fit().name, "Fit 3");
        assert_eq!(
            (
                host.app.fit().spec.x.as_str(),
                host.app.fit().spec.y.as_str()
            ),
            ("x", "y")
        );
        assert_eq!(host.app.fit().spec.model(), "poly1");
        assert!(host.app.fit().result.is_none());
        fit(&mut host, &ws).unwrap();
        assert_eq!(shown(&host).type_name, "poly1");
        // Delete: the next fit is shown; the last fit stays.
        host.app.select(1);
        host.app.delete();
        let names: Vec<&str> = host.app.fits.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["Fit 1", "Fit 3"]);
        assert_eq!(host.app.fit().name, "Fit 3");
        host.app.delete();
        host.app.delete();
        assert_eq!(host.app.fits.len(), 1);
        assert_eq!(host.app.fit().name, "Fit 1");
        assert_eq!(shown(&host).type_name, "poly2");
        // Auto-fit off: edits wait for the Fit button.
        host.app.auto = false;
        host.app.fit_mut().spec.terms = 4;
        host.app.prepare(&ws);
        assert!(host.app.script(&ws).unwrap().contains("'poly2'"));
        host.app.apply();
        assert!(host.app.script(&ws).unwrap().contains("'poly4'"));
    }

    #[test]
    fn curve_fitter_smoothing_slider_and_click_targets() {
        use apps::curve_fitter::{nearest, smoothing_from_position, smoothing_position};
        assert_eq!(smoothing_from_position(0.), 0.);
        assert_eq!(smoothing_from_position(1.), 0.9);
        assert_eq!(smoothing_from_position(2.), 0.99);
        assert_eq!(smoothing_from_position(smoothing_position(0.9876)), 0.988);
        assert_eq!(smoothing_position(1.), 8.);
        let points = [
            egui::pos2(10., 10.),
            egui::pos2(30., 10.),
            egui::pos2(f32::NAN, 0.),
        ];
        assert_eq!(
            nearest(points.into_iter(), egui::pos2(27., 12.), 10.),
            Some(1)
        );
        assert_eq!(nearest(points.into_iter(), egui::pos2(60., 60.), 10.), None);
    }

    /// Frames until `done` holds for the app.
    fn settle(
        app: &mut Workspace,
        ctx: &egui::Context,
        size: egui::Vec2,
        done: impl Fn(&Host<CurveFitter>) -> bool,
    ) -> egui::FullOutput {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let output = frame_at(app, ctx, size, vec![]);
            let host = &app.toolkits.apps.curve;
            if !host.running() && done(host) {
                return frame_at(app, ctx, size, vec![]);
            }
            assert!(Instant::now() < deadline, "{:?}", host.status());
            drop(output);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    /// A data dot (filled circle marker) of a plot inside `area`.
    fn data_dot(shape: &egui::epaint::Shape, area: egui::Rect) -> Option<egui::Pos2> {
        match shape {
            egui::epaint::Shape::Circle(c)
                if (c.radius - 2.5).abs() < 1e-3 && area.contains(c.center) =>
            {
                Some(c.center)
            }
            egui::epaint::Shape::Vec(v) => v.iter().find_map(|s| data_dot(s, area)),
            _ => None,
        }
    }
    fn painted(output: &egui::FullOutput, label: &str) -> bool {
        output
            .shapes
            .iter()
            .any(|s| text_position(&s.shape, label).is_some())
    }
    fn open_with_data(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2) {
        app.run(DATA.into());
        finish(app);
        app.toolkits.open_tool(Tool::App(AppKind::CurveFitter));
        let spec = &mut app.toolkits.apps.curve.app.fit_mut().spec;
        (spec.x, spec.y, spec.terms) = ("x".into(), "y".into(), 2);
        settle(app, ctx, size, |host| host.app.fit().result.is_some());
    }

    #[test]
    fn curve_fitter_tab_renders_three_areas_in_the_main_window() {
        let size = egui::vec2(1440., 900.);
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        open_with_data(&mut app, &ctx, size);
        let output = frame_at(&mut app, &ctx, size, vec![]);
        assert_eq!(
            app.toolkits.selected_tool(),
            Some(Tool::App(AppKind::CurveFitter))
        );
        for label in [
            "INPUTS",
            "FITS",
            "OUTPUT",
            "Residuals",
            "New fit",
            "Duplicate",
            "Export to workspace",
            "Generate script",
            "X data",
            "Fit type",
            "Fit 1",
            "R²",
            "SSE",
            "Formula",
            "p1*x^2 + p2*x + p3",
            "Fit object name",
        ] {
            assert!(painted(&output, label), "{label} not painted");
        }
        let sse = apps::short(
            app.toolkits
                .apps
                .curve
                .app
                .fit()
                .result
                .as_ref()
                .unwrap()
                .sse,
        );
        assert!(painted(&output, &sse), "SSE value {sse}");
        let (text, status) = app
            .toolkits
            .app_status(Tool::App(AppKind::CurveFitter), false)
            .unwrap();
        assert_eq!(status, toolkits::AppStatus::Success);
        assert!(text.starts_with("Fit 1: poly2"), "{text}");
        // Clicking a data dot of the fit plot leaves the point out and the
        // fit reruns without it; a second click brings it back.
        let main = egui::Rect::from_x_y_ranges(apps::COLUMN..=size.x - apps::COLUMN, 0.0..=size.y);
        let dot = output
            .shapes
            .iter()
            .find_map(|s| data_dot(&s.shape, main))
            .expect("a data dot");
        for pressed in [true, false] {
            pointer_frame(&mut app, &ctx, size, dot, pressed);
        }
        settle(&mut app, &ctx, size, |host| {
            host.app.fit().result.as_ref().is_some_and(|r| r.dfe == 27.)
        });
        assert_eq!(app.toolkits.apps.curve.app.fit().spec.excluded.len(), 1);
        let script = {
            let ws = &app.toolkits.apps.workspace;
            app.toolkits.apps.curve.app.script(ws).unwrap()
        };
        assert!(script.contains("opts.Exclude = ["), "{script}");
        let output = frame_at(&mut app, &ctx, size, vec![]);
        let cross = output
            .shapes
            .iter()
            .any(|s| data_dot(&s.shape, main).is_some_and(|p| p.distance(dot) < 0.5));
        assert!(!cross, "the excluded point is drawn as a cross, not a dot");
        for pressed in [true, false] {
            pointer_frame(&mut app, &ctx, size, dot, pressed);
        }
        settle(&mut app, &ctx, size, |host| {
            host.app.fit().result.as_ref().is_some_and(|r| r.dfe == 28.)
        });
        assert!(app.toolkits.apps.curve.app.fit().spec.excluded.is_empty());
        // Residuals view.
        click_label_at(&mut app, &ctx, size, "Residuals");
        let output = frame_at(&mut app, &ctx, size, vec![]);
        assert!(painted(&output, "Residual"));
        assert_eq!(
            app.toolkits.apps.curve.app.view,
            apps::curve_fitter::View::Residuals
        );
        // Prediction bounds on the fit view: a new run brings them.
        app.toolkits.apps.curve.app.view = apps::curve_fitter::View::Fit;
        app.toolkits.apps.curve.app.show_bounds = true;
        settle(&mut app, &ctx, size, |host| {
            host.app
                .fit()
                .result
                .as_ref()
                .is_some_and(|r| !r.lower.is_empty())
        });
        // New fit from the list: a second row.
        click_label_at(&mut app, &ctx, size, "New fit");
        let output = settle(&mut app, &ctx, size, |host| {
            host.app.fits.len() == 2 && host.app.fit().result.is_some()
        });
        assert!(painted(&output, "Fit 2"));
        assert_eq!(app.toolkits.apps.curve.app.fit().spec.model(), "poly1");
        // A surface renders as an image with its points.
        let spec = &mut app.toolkits.apps.curve.app.fit_mut().spec;
        (spec.x, spec.y, spec.z) = ("xs".into(), "ys".into(), "zs".into());
        let output = settle(&mut app, &ctx, size, |host| {
            host.app
                .fit()
                .result
                .as_ref()
                .is_some_and(|r| r.grid.is_some())
        });
        assert!(painted(&output, "Degree in x"));
        assert!(painted(&output, "p00 + p10*x + p01*y"));
        // A narrow host stacks OUTPUT under the main view and still renders.
        let output = frame_at(&mut app, &ctx, egui::vec2(900., 700.), vec![]);
        assert!(painted(&output, "OUTPUT"));
        // Closing the tab keeps the fits for the next open.
        app.close_tool_tab(Tool::App(AppKind::CurveFitter));
        assert!(app.toolkits.tabs().next().is_none());
        assert_eq!(app.toolkits.apps.curve.app.fits.len(), 2);
    }

    #[test]
    fn curve_fitter_export_creates_the_fit_without_clobbering() {
        let size = egui::vec2(1440., 900.);
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.run("fittedmodel = 7; ft = 2;".into());
        finish(&mut app);
        open_with_data(&mut app, &ctx, size);
        let shown = app.toolkits.apps.curve.app.fit().result.clone().unwrap();
        click_label_at(&mut app, &ctx, size, "Export to workspace");
        finish(&mut app);
        frame_at(&mut app, &ctx, size, vec![]);
        assert!(
            matches!(&app.variables["fittedmodel"], matrust::Array::Numeric(m) if m.data() == [7.]),
            "fittedmodel is kept"
        );
        assert!(
            matches!(&app.variables["ft"], matrust::Array::Numeric(m) if m.data() == [2.]),
            "ft is kept"
        );
        assert!(app.variables.contains_key("fittedmodel1"), "{}", app.status);
        assert!(app.variables.contains_key("goodness"));
        assert!(app.toolkits.apps.curve.message.contains("fittedmodel1"));
        // The exported fit (a handle of the workspace's engine) has the
        // coefficients and goodness the app shows.
        app.run(
            "c_ = coeffvalues(fittedmodel1); k_ = double(isa(fittedmodel1, 'cfit')); \
             g_ = [goodness.sse goodness.rsquare goodness.adjrsquare goodness.rmse goodness.dfe];"
                .into(),
        );
        finish(&mut app);
        let number = |name: &str| match &app.variables[name] {
            matrust::Array::Numeric(m) => m.data().to_vec(),
            other => panic!("{name}: {other:?}"),
        };
        let c: Vec<f64> = shown.coefficients.iter().map(|c| c.value).collect();
        assert_eq!(number("c_"), c, "{}", app.status);
        assert_eq!(number("k_"), [1.]);
        assert!(same(&number("g_"), &gof(&shown)));
        // Taken now: the next defaults are fittedmodel2 and goodness1.
        frame_at(&mut app, &ctx, size, vec![]);
        let ws = &app.toolkits.apps.workspace;
        assert_eq!(
            app.toolkits.apps.curve.app.output_names(ws).unwrap(),
            ("fittedmodel2".into(), "goodness1".into())
        );
    }

    #[test]
    fn curve_fitter_generate_script_saves_header_and_the_same_script() {
        let size = egui::vec2(1440., 900.);
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fitted.m");
        let mut app = Workspace::with_context(ctx.clone(), true);
        open_with_data(&mut app, &ctx, size);
        let script = {
            let ws = &app.toolkits.apps.workspace;
            app.toolkits.apps.curve.app.script(ws).unwrap()
        };
        app.toolkits.apps.test_save_path = Some(path.clone());
        click_label_at(&mut app, &ctx, size, "Generate script");
        finish(&mut app);
        frame_at(&mut app, &ctx, size, vec![]);
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(
            saved.starts_with("% Curve Fitter (MatRust app), generated "),
            "{saved}"
        );
        assert!(saved.ends_with(&script), "{saved}");
        assert!(app.toolkits.apps.curve.message.starts_with("Saved"));
        // The saved file runs and reproduces the fit.
        let mut engine = run_script(&saved);
        let shown = app.toolkits.apps.curve.app.fit().result.clone().unwrap();
        assert!(same(&gof_of(&mut engine, "goodness"), &gof(&shown)));
    }
}
