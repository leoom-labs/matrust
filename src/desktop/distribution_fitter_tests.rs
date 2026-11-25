// Distribution Fitter app (apps/distribution_fitter.rs): on a fixed
// Weibull-like sample the generated script reproduces the parameters and
// NLogL the app shows; AIC/BIC ordering; censoring, frequency and exclusion
// rules reach the script; every display builds its plot calls; delta-method
// bounds against the closed form; export names; errors; a rendered tab.
mod distribution_fitter_tests {
    use super::*;
    use apps::distribution_fitter::{
        Bins, Display, DistributionFitter, Family, FitResult, Function, Scale, Sort,
    };
    use apps::{App, AppKind, Host, Snapshot};
    use toolkits::Tool;

    /// 40 Weibull(a = 2, b = 1.8) quantiles in a scrambled order, plus
    /// censoring flags, frequencies and a sample with a negative value.
    const DATA: &str = "u = mod((1:40)' * 0.6180339887498949, 1); \
        x = 2*(-log(1 - u)).^(1/1.8); \
        c = mod(1:40, 7)' == 0; \
        f = 1 + mod(0:39, 3)'; \
        neg = x - 1; \
        nan = NaN*ones(5, 1);";

    fn workspace(setup: &str) -> Snapshot {
        let mut engine = matrust::Engine::default();
        engine.execute(setup).unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        ws
    }
    /// A fitter on `x` with Normal (Fit 1), Weibull, Lognormal and Kernel fits.
    fn four_fits() -> Host<DistributionFitter> {
        let mut host = Host::new(DistributionFitter::default());
        host.app.data_sets[0].data = "x".into();
        for family in [Family::Weibull, Family::Lognormal, Family::Kernel] {
            host.app.new_fit(family).unwrap();
        }
        host
    }
    /// One private run, synchronously, delivered to the app.
    fn fit(host: &mut Host<DistributionFitter>, ws: &Snapshot) -> Result<(), String> {
        host.app.prepare(ws);
        let request = host.request(ws)?;
        let outputs = apps::run(&mut matrust::Engine::default(), request)?;
        host.app.receive(&outputs)
    }
    fn result(host: &Host<DistributionFitter>, k: usize) -> &FitResult {
        host.app.fits[k].result.as_ref().expect("a fit result")
    }
    fn values(engine: &matrust::Engine, name: &str) -> Vec<f64> {
        match engine.array(name) {
            Some(matrust::Array::Numeric(m)) => m.data().to_vec(),
            other => panic!("{name}: {other:?}"),
        }
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
    fn eval(engine: &mut matrust::Engine, code: &str) -> Vec<f64> {
        engine
            .execute(&format!("zz_ = {code};"))
            .unwrap_or_else(|e| panic!("{code}: {e}"));
        values(engine, "zz_")
    }

    #[test]
    fn distribution_fitter_generated_script_reproduces_the_fits_shown() {
        let ws = workspace(DATA);
        let mut host = four_fits();
        fit(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        for call in [
            "pd1 = fitdist(x, 'Normal');",
            "pd2 = fitdist(x, 'Weibull');",
            "pd3 = fitdist(x, 'Lognormal');",
            "pd4 = fitdist(x, 'Kernel');",
            "histogram(x, 'Normalization', 'pdf')",
            "legend({'x', 'Fit 1', 'Fit 2', 'Fit 3', 'Fit 4'})",
        ] {
            assert!(script.contains(call), "{call}\n{script}");
        }
        let mut engine = run_script(&script);
        for (k, pd) in ["pd1", "pd2", "pd3"].iter().enumerate() {
            let shown = result(&host, k);
            assert_eq!(
                eval(&mut engine, &format!("{pd}.ParameterValues")),
                shown.values,
                "{pd}"
            );
            assert_eq!(
                eval(&mut engine, &format!("{pd}.NLogL")),
                [shown.nlogl],
                "{pd}"
            );
            assert_eq!(shown.parameters, 2.);
            assert_eq!(shown.observations, 40.);
            assert_eq!(shown.intervals.len(), 2);
            assert!(
                shown
                    .intervals
                    .iter()
                    .zip(&shown.values)
                    .all(|(ci, v)| ci[0] < *v && *v < ci[1])
            );
        }
        // The Weibull fit recovers the sample's law.
        let weibull = result(&host, 1).clone();
        assert_eq!(weibull.names, ["A", "B"]);
        assert!((weibull.values[0] - 2.).abs() < 0.2, "{:?}", weibull.values);
        assert!(
            (weibull.values[1] - 1.8).abs() < 0.3,
            "{:?}",
            weibull.values
        );
        assert_eq!(
            eval(&mut engine, "[mean(pd2) var(pd2)]"),
            [weibull.mean, weibull.variance]
        );
        assert_eq!(
            eval(&mut engine, "pd2.ParameterCovariance(:)'"),
            weibull.covariance
        );
        assert_eq!(
            eval(&mut engine, "paramci(pd2)")[..],
            [
                weibull.intervals[0][0],
                weibull.intervals[0][1],
                weibull.intervals[1][0],
                weibull.intervals[1][1]
            ]
        );
        // The kernel estimate: its bandwidth and likelihood, no parameters.
        let kernel = result(&host, 3);
        assert_eq!(
            eval(&mut engine, "pd4.BandWidth"),
            [kernel.bandwidth.unwrap()]
        );
        assert_eq!(eval(&mut engine, "negloglik(pd4)"), [kernel.nlogl]);
        assert!(kernel.parameters.is_nan() && kernel.aic.is_nan());
        assert!(kernel.parameter_text().starts_with("bandwidth = "));
        // The fitted pdfs drawn are the script's pdf values on its grid.
        let curve = &weibull.curve;
        assert_eq!(curve.len(), 200);
        let pdf = eval(&mut engine, "pdf(pd2, xg)'");
        assert!(curve.iter().zip(&pdf).all(|(p, y)| p[1] == *y));
        // The Output block shows the selected fit.
        host.app.select(1);
        let rows = host.app.results();
        assert_eq!(rows[0], ("Fit", "Fit 2".to_owned()));
        assert_eq!(rows[1], ("Distribution", "Weibull".to_owned()));
        assert_eq!(rows[3], ("NLogL", apps::short(weibull.nlogl)));
        assert!(host.app.summary().unwrap().contains("lowest AIC: Fit 2"));
    }

    #[test]
    fn distribution_fitter_aic_bic_order_the_fits() {
        let ws = workspace(DATA);
        let mut host = four_fits();
        fit(&mut host, &ws).unwrap();
        for k in 0..3 {
            let r = result(&host, k);
            assert!((r.aic - (2. * r.nlogl + 4.)).abs() < 1e-12);
            assert!((r.bic - (2. * r.nlogl + 2. * 40_f64.ln())).abs() < 1e-12);
        }
        // Weibull data: the Weibull fit ranks above Normal and Lognormal;
        // the kernel estimate has no AIC or BIC and goes last there.
        for sort in [Sort::NLogL, Sort::Aic, Sort::Bic] {
            host.app.sort = sort;
            let order = host.app.order();
            let rank = |k: usize| order.iter().position(|i| *i == k).unwrap();
            assert!(
                rank(1) < rank(0) && rank(1) < rank(2),
                "{sort:?}: {order:?}"
            );
            if sort != Sort::NLogL {
                assert_eq!(order[0], 1, "{sort:?}");
                assert_eq!(order[3], 3, "{sort:?}");
            }
            let shown: Vec<f64> = order
                .iter()
                .map(|i| result(&host, *i).criterion(sort))
                .filter(|v| !v.is_nan())
                .collect();
            assert!(shown.windows(2).all(|w| w[0] <= w[1]), "{sort:?}");
        }
        host.app.sort = Sort::Created;
        assert_eq!(host.app.order(), [0, 1, 2, 3]);
    }

    #[test]
    fn distribution_fitter_censoring_and_frequency_reach_the_script() {
        let ws = workspace(DATA);
        let mut host = Host::new(DistributionFitter::default());
        host.app.data_sets[0].data = "x".into();
        host.app.fits[0].family = Family::Weibull;
        fit(&mut host, &ws).unwrap();
        let plain = result(&host, 0).clone();
        host.app.data_sets[0].censoring = "c".into();
        host.app.data_sets[0].frequency = "f".into();
        fit(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.contains("pd1 = fitdist(x, 'Weibull', 'Censoring', c, 'Frequency', f);"),
            "{script}"
        );
        assert!(script.contains("histogram(repelem(x, f), 'Normalization', 'pdf')"));
        let weighted = result(&host, 0);
        assert_ne!(weighted.values, plain.values);
        assert_ne!(weighted.nlogl, plain.nlogl);
        // Frequencies count: 40 rows with weights 1, 2, 3.
        assert_eq!(weighted.observations, 79.);
        assert_eq!(host.app.inputs(), ["c", "f", "x"]);
        let mut engine = run_script(&script);
        assert_eq!(eval(&mut engine, "pd1.ParameterValues"), weighted.values);
        assert_eq!(eval(&mut engine, "pd1.NLogL"), [weighted.nlogl]);
        // The cdf display passes them to ecdf and probplot.
        host.app.display = Display::Cdf;
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.contains("ecdf(x, 'Censoring', c, 'Frequency', f)"),
            "{script}"
        );
        host.app.display = Display::Probability;
        host.app.data_sets[0].censoring.clear();
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.contains("hp = probplot('normal', x, false(size(x)), f, 'noref');"),
            "{script}"
        );
        fit(&mut host, &ws).unwrap();
    }

    #[test]
    fn distribution_fitter_exclusion_rule_is_a_logical_index() {
        let ws = workspace(DATA);
        let mut host = Host::new(DistributionFitter::default());
        host.app.data_sets[0].data = "x".into();
        host.app.fits[0].family = Family::Weibull;
        let rule = host.app.new_rule().unwrap();
        host.app.rules[0].lower = "1".into();
        host.app.rules[0].upper = "3".into();
        host.app.rules[0].upper_strict = true;
        assert_eq!(host.app.rules[0].describe(), "1 ≤ x < 3");
        host.app.fits[0].rule = Some(rule);
        fit(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(script.contains("keep = x >= 1 & x < 3;"), "{script}");
        assert!(
            script.contains("pd1 = fitdist(x(keep), 'Weibull');"),
            "{script}"
        );
        let shown = result(&host, 0).clone();
        let mut engine = matrust::Engine::default();
        engine.execute(DATA).unwrap();
        let direct = eval(
            &mut engine,
            "fitdist(x(x >= 1 & x < 3), 'Weibull').ParameterValues",
        );
        assert_eq!(direct, shown.values);
        let kept = eval(&mut engine, "sum(x >= 1 & x < 3)")[0];
        assert_eq!(shown.observations, kept);
        assert!(kept < 40.);
        // A fit that is not exported uses a private index: same numbers.
        host.app.fits[0].export = false;
        fit(&mut host, &ws).unwrap();
        assert_eq!(result(&host, 0).values, shown.values);
        let script = host.app.script(&ws).unwrap();
        assert!(!script.contains("fitdist"), "{script}");
        assert!(
            host.app
                .exported(&ws)
                .iter()
                .all(|n| n != "keep" && n != "pd1")
        );
        // Limits that keep nothing are refused before the engine runs.
        host.app.fits[0].export = true;
        host.app.rules[0].lower = "3".into();
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("no value is inside"), "{error}");
        host.app.rules[0].lower = "a".into();
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("must be a number"), "{error}");
        // Deleting the rule frees its fits.
        host.app.delete_rule();
        assert_eq!(host.app.fits[0].rule, None);
        assert!(
            host.app
                .script(&ws)
                .unwrap()
                .contains("pd1 = fitdist(x, 'Weibull');")
        );
    }

    #[test]
    fn distribution_fitter_each_display_builds_its_plot_calls() {
        let ws = workspace(&format!("{DATA} k = round(3*x) + 1;"));
        let mut host = four_fits();
        // A second data set with a discrete fit on it.
        let counts = host.app.new_data_set().unwrap();
        host.app.data_sets[1].data = "k".into();
        host.app.new_fit(Family::Poisson).unwrap();
        host.app.fits[4].data_set = counts;
        host.app.bins = Bins::Scott;
        let cases: [(Display, &[&str]); 6] = [
            (
                Display::Density,
                &[
                    "histogram(x, 'Normalization', 'pdf', 'BinMethod', 'scott')",
                    "histogram(k, 'Normalization', 'pdf', 'BinMethod', 'scott')",
                    "xg = linspace(min([x(:); k(:)]), max([x(:); k(:)]), 200)';",
                    "xi = unique(round(linspace(",
                    "plot(xg, pdf(pd2, xg))",
                    "plot(xi, pdf(pd5, xi), '.-')",
                    "ylabel('Density')",
                ],
            ),
            (
                Display::Cdf,
                &[
                    "ecdf(x)",
                    "ecdf(k)",
                    "plot(xg, cdf(pd1, xg))",
                    "stairs(xi, cdf(pd5, xi))",
                ],
            ),
            (
                Display::Quantile,
                &[
                    "[qF, qX] = ecdf(x);",
                    "stairs(qF, qX([2:end end]))",
                    "pg = linspace(0.001, 0.999, 200)';",
                    "plot(pg, icdf(pd3, pg))",
                ],
            ),
            (
                Display::Probability,
                &[
                    "hp = probplot('weibull', x, 'noref');",
                    "hp1 = probplot('weibull', k, 'noref');",
                    "hf = probplot(gca, pd1);",
                    "legend([hp(1); hp1(1); hf; hf1; hf2; hf3], {'x', 'k', 'Fit 1', 'Fit 2', 'Fit 3', 'Fit 5'})",
                ],
            ),
            (
                Display::Survivor,
                &[
                    "ecdf(x, 'Function', 'survivor')",
                    "plot(xg, cdf(pd2, xg, 'upper'))",
                ],
            ),
            (
                Display::CumulativeHazard,
                &[
                    "ecdf(x, 'Function', 'cumulative hazard')",
                    "plot(xg, -log(cdf(pd2, xg, 'upper')))",
                ],
            ),
        ];
        host.app.scale = Scale::Weibull;
        for (display, calls) in cases {
            host.app.display = display;
            let script = host.app.script(&ws).unwrap();
            for call in calls {
                assert!(script.contains(call), "{display:?}: {call}\n{script}");
            }
            assert_eq!(script.matches("hold on").count(), 1, "{script}");
            assert!(script.contains("hold off"));
            fit(&mut host, &ws).unwrap_or_else(|e| panic!("{display:?}: {e}"));
            let shown = host.app.shown.as_ref().unwrap();
            assert_eq!(shown.display, display);
            assert_eq!(shown.data.len(), 2);
            for curve in &shown.data {
                if display == Display::Density {
                    assert!(!curve.bins.is_empty(), "{display:?}");
                } else {
                    assert!(!curve.points.is_empty(), "{display:?}");
                }
            }
            for (k, f) in host.app.fits.iter().enumerate() {
                let r = f.result.as_ref().unwrap();
                let none = display == Display::Probability && f.family == Family::Kernel;
                assert_eq!(r.curve.is_empty(), none, "{display:?} fit {k}");
            }
            if display == Display::Probability {
                // Marks at fixed probabilities, on the Weibull paper.
                let ticks = &shown.ticks;
                assert_eq!(ticks.len(), 15);
                let half = ticks.iter().find(|(_, l)| l == "0.5").unwrap().0;
                assert!((half - 2_f64.ln().ln()).abs() < 1e-12, "{half}");
            }
            run_script(&format!("k = round(3*x) + 1;\n{script}"));
        }
        // The empirical cdf steps through the sorted sample.
        host.app.display = Display::Cdf;
        fit(&mut host, &ws).unwrap();
        let steps = &host.app.shown.as_ref().unwrap().data[0].points;
        assert_eq!(steps.first().unwrap()[1], 0.);
        assert_eq!(steps.last().unwrap()[1], 1.);
        assert!(
            steps
                .windows(2)
                .all(|w| w[0][1] <= w[1][1] && w[0][0] <= w[1][0])
        );
    }

    /// Standard normal cdf and density.
    fn phi(z: f64) -> (f64, f64) {
        (
            0.5 * libm::erfc(-z / std::f64::consts::SQRT_2),
            (-0.5 * z * z).exp() / (2. * std::f64::consts::PI).sqrt(),
        )
    }

    #[test]
    fn distribution_fitter_bounds_match_the_closed_form_delta_method() {
        let ws = workspace(DATA);
        let mut host = Host::new(DistributionFitter::default());
        host.app.data_sets[0].data = "x".into();
        host.app.pane = apps::distribution_fitter::Pane::Evaluate;
        host.app.function = Function::Cdf;
        host.app.points = "0.5 1 2 3.5".into();
        host.app.point_bounds = true;
        fit(&mut host, &ws).unwrap();
        let r = result(&host, 0).clone();
        let (mu, sigma) = (r.values[0], r.values[1]);
        let c = &r.covariance;
        let z95 = 1.959963984540054;
        assert_eq!(r.evaluated.len(), 4);
        for [x, value, lo, hi] in &r.evaluated {
            // Logit-scale delta method with the analytic gradient of
            // Phi((x - mu)/sigma).
            let z = (x - mu) / sigma;
            let (f, d) = phi(z);
            let g = [-d / sigma, -d * z / sigma];
            let var = g[0] * g[0] * c[0] + 2. * g[0] * g[1] * c[2] + g[1] * g[1] * c[3];
            let se = var.sqrt() / (f * (1. - f));
            let logit = (f / (1. - f)).ln();
            let want = [
                1. / (1. + (-(logit - z95 * se)).exp()),
                1. / (1. + (-(logit + z95 * se)).exp()),
            ];
            assert!((value - f).abs() < 1e-12, "{x}");
            assert!((lo - want[0]).abs() < 1e-6, "{x}: {lo} vs {}", want[0]);
            assert!((hi - want[1]).abs() < 1e-6, "{x}: {hi} vs {}", want[1]);
        }
        // Quantiles of a positive family: bounds on the log scale bracket.
        host.app.fits[0].family = Family::Weibull;
        host.app.function = Function::Icdf;
        host.app.points = "0.1 0.5 0.9".into();
        fit(&mut host, &ws).unwrap();
        let r = result(&host, 0);
        let mut engine = run_script(&host.app.script(&ws).unwrap());
        let q = eval(&mut engine, "icdf(pd1, [0.1 0.5 0.9])");
        for (k, [_, value, lo, hi]) in r.evaluated.iter().enumerate() {
            assert!((value - q[k]).abs() < 1e-12);
            assert!(0. < *lo && lo < value && value < hi, "{lo} {value} {hi}");
        }
        // Survivor bounds mirror the cdf's; pdf has none.
        host.app.function = Function::Survivor;
        host.app.points = "2".into();
        fit(&mut host, &ws).unwrap();
        let [_, s, lo, hi] = result(&host, 0).evaluated[0];
        assert!(lo < s && s < hi);
        host.app.function = Function::Pdf;
        fit(&mut host, &ws).unwrap();
        assert!(result(&host, 0).evaluated[0][2].is_nan());
        // The cdf display's bounds come from the same code.
        host.app.display = Display::Cdf;
        host.app.bounds = true;
        fit(&mut host, &ws).unwrap();
        let r = result(&host, 0);
        assert_eq!(r.lower.len(), r.curve.len());
        assert!(
            r.lower
                .iter()
                .zip(&r.curve)
                .all(|(l, c)| l[1] <= c[1] + 1e-15)
        );
        let data = &host.app.shown.as_ref().unwrap().data[0];
        assert!(!data.lower.is_empty() && !data.upper.is_empty());
        // The exported script draws the same bounds (data and fit).
        let shown: Vec<f64> = r.lower.iter().map(|l| l[1]).collect();
        let script = host.app.script(&ws).unwrap();
        assert!(script.contains("'Bounds', 'on', 'Alpha', 0.05"), "{script}");
        let mut engine = run_script(&script);
        let lower = eval(&mut engine, "fLo");
        assert_eq!(lower.len(), shown.len(), "{script}");
        assert!(lower.iter().zip(&shown).all(|(a, b)| (a - b).abs() < 1e-9 || (a.is_nan() && b.is_nan())));
        assert!(eval(&mut engine, "exist('app_G')")[0] == 0.);
        // Points that are not numbers stay out of the run.
        host.app.points = "1 x".into();
        assert!(host.app.evaluate_points().is_err());
        host.app.function = Function::Icdf;
        host.app.points = "1.5".into();
        assert!(
            host.app
                .evaluate_points()
                .unwrap_err()
                .contains("from 0 to 1")
        );
    }

    #[test]
    fn distribution_fitter_fixed_parameters_and_kernel_options_are_non_default_only() {
        let ws = workspace(&format!("{DATA} n = round(8*u);"));
        let mut host = Host::new(DistributionFitter::default());
        host.app.data_sets[0].data = "x".into();
        let fit0 = &mut host.app.fits[0];
        fit0.family = Family::HalfNormal;
        fit0.fixed = "0".into();
        assert_eq!(fit0.options().unwrap(), "");
        fit0.fixed = "0.25".into();
        assert_eq!(fit0.options().unwrap(), ", 'mu', 0.25");
        fit0.family = Family::GeneralizedPareto;
        assert_eq!(fit0.options().unwrap(), ", 'Theta', 0.25");
        fit0.family = Family::Binomial;
        fit0.fixed.clear();
        assert!(fit0.options().unwrap_err().contains("number of trials N"));
        fit0.fixed = "10".into();
        assert_eq!(fit0.options().unwrap(), ", 'NTrials', 10");
        fit0.family = Family::Kernel;
        assert_eq!(fit0.options().unwrap(), "");
        fit0.smoother = apps::distribution_fitter::Smoother::Epanechnikov;
        fit0.bandwidth = "0.4".into();
        fit0.support = apps::distribution_fitter::Support::Positive;
        assert_eq!(
            fit0.options().unwrap(),
            ", 'Kernel', 'epanechnikov', 'Bandwidth', 0.4, 'Support', 'positive'"
        );
        fit(&mut host, &ws).unwrap();
        assert_eq!(result(&host, 0).bandwidth, Some(0.4));
        let fit0 = &mut host.app.fits[0];
        fit0.support = apps::distribution_fitter::Support::Bounded;
        fit0.support_lower = "0".into();
        fit0.support_upper = "9".into();
        assert!(fit0.options().unwrap().ends_with("'Support', [0 9]"));
        fit0.support_upper = "-1".into();
        assert!(fit0.options().is_err());
        // Every family has a fitdist name the engine knows.
        let mut engine = matrust::Engine::default();
        engine.execute(&format!("{DATA} n = round(8*u);")).unwrap();
        for family in Family::ALL {
            let data = match family {
                Family::Poisson | Family::NegativeBinomial | Family::Binomial => "n",
                Family::Beta => "x / 10",
                _ => "x",
            };
            let options = if family == Family::Binomial {
                ", 'NTrials', 10"
            } else {
                ""
            };
            let code = format!("p_ = fitdist({data}, '{}'{options});", family.name());
            engine
                .execute(&code)
                .unwrap_or_else(|e| panic!("{family:?}: {e}"));
        }
    }

    #[test]
    fn distribution_fitter_errors_keep_the_last_fits() {
        let ws = workspace(DATA);
        let mut host = Host::new(DistributionFitter::default());
        host.app.data_sets[0].data = "x".into();
        host.app.fits[0].family = Family::Weibull;
        fit(&mut host, &ws).unwrap();
        let good = result(&host, 0).clone();
        // Negative values for a Weibull fit: the engine's message.
        host.app.data_sets[0].data = "neg".into();
        let error = fit(&mut host, &ws).unwrap_err();
        assert!(error.contains("strictly positive"), "{error}");
        assert_eq!(result(&host, 0), &good, "the view keeps the last good fit");
        // All NaN: nothing left to fit.
        host.app.data_sets[0].data = "nan".into();
        host.app.fits[0].family = Family::Normal;
        let error = fit(&mut host, &ws).unwrap_err();
        assert!(error.contains("no observations"), "{error}");
        assert_eq!(result(&host, 0), &good);
        // Missing inputs never reach the engine.
        host.app.data_sets[0].data = "gone".into();
        let error = host.request(&ws).err().unwrap();
        assert!(error.contains("gone does not exist"), "{error}");
        host.app.data_sets[0].data.clear();
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("choose the data"), "{error}");
        // Binomial needs its N.
        host.app.data_sets[0].data = "x".into();
        host.app.fits[0].family = Family::Binomial;
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("number of trials"), "{error}");
        // Through the threaded runner, the status line carries the message.
        host.app.fits[0].family = Family::Weibull;
        host.app.data_sets[0].data = "neg".into();
        let ctx = egui::Context::default();
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
        assert!(text.contains("strictly positive"), "{text}");
        assert_eq!(result(&host, 0), &good);
    }

    #[test]
    fn distribution_fitter_object_names_never_clobber() {
        let ws = workspace(&format!("{DATA} pd1 = 7; pd3 = 1;"));
        let mut host = four_fits();
        let names: Vec<String> = host.app.object_names(&ws).unwrap().into_values().collect();
        assert_eq!(names, ["pd2", "pd4", "pd5", "pd6"]);
        // An edited name is the user's; another fit's default moves on.
        host.app.fits[1].object = "weib".into();
        host.app.fits[1].object_edited = true;
        let names = host.app.object_names(&ws).unwrap();
        assert_eq!(names[&host.app.fits[1].id], "weib");
        assert_eq!(names[&host.app.fits[0].id], "pd2");
        // Clashes are refused.
        host.app.fits[1].object = "x".into();
        assert!(
            host.app
                .object_names(&ws)
                .unwrap_err()
                .contains("data variable")
        );
        host.app.fits[1].object = "pd2".into();
        host.app.fits[0].object = "pd2".into();
        host.app.fits[0].object_edited = true;
        assert!(
            host.app
                .object_names(&ws)
                .unwrap_err()
                .contains("different names")
        );
        host.app.fits[1].object = "1bad".into();
        assert!(
            host.app
                .object_names(&ws)
                .unwrap_err()
                .contains("valid variable")
        );
        // Unchecked fits get no name and stay out of the export.
        let mut host = four_fits();
        host.app.fits[2].export = false;
        let exported = host.app.exported(&ws);
        assert_eq!(exported, ["pd2", "pd4", "pd5", "xg"]);
    }

    fn open_fitter(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2) -> egui::FullOutput {
        app.toolkits
            .open_tool(Tool::App(AppKind::DistributionFitter));
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let output = frame_at(app, ctx, size, vec![]);
            let host = &app.toolkits.apps.distribution;
            if host.app.shown.is_some() && !host.running() {
                return frame_at(app, ctx, size, vec![]);
            }
            assert!(Instant::now() < deadline, "{:?}", host.status());
            drop(output);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn painted(output: &egui::FullOutput, label: &str) -> bool {
        output
            .shapes
            .iter()
            .any(|s| text_position(&s.shape, label).is_some())
    }
    fn wait(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            frame_at(app, ctx, size, vec![]);
            if !app.toolkits.apps.distribution.running() {
                frame_at(app, ctx, size, vec![]);
                return;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn distribution_fitter_tab_renders_three_areas() {
        let size = egui::vec2(1440., 900.);
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.run(DATA.into());
        finish(&mut app);
        {
            let fitter = &mut app.toolkits.apps.distribution.app;
            fitter.data_sets[0].data = "x".into();
            fitter.new_fit(Family::Weibull).unwrap();
        }
        let output = open_fitter(&mut app, &ctx, size);
        assert_eq!(
            app.toolkits.selected_tool(),
            Some(Tool::App(AppKind::DistributionFitter))
        );
        for label in [
            "INPUTS",
            "DISTRIBUTIONS",
            "OUTPUT",
            "Data sets",
            "Exclusion rules",
            "Fits",
            "Density",
            "Probability plot",
            "Cumulative hazard",
            "Compare",
            "Selected fit",
            "Evaluate",
            "Export to workspace",
            "Generate script",
            "NLogL",
        ] {
            assert!(painted(&output, label), "{label} not painted");
        }
        let nlogl = apps::short(
            app.toolkits.apps.distribution.app.fits[1]
                .result
                .as_ref()
                .unwrap()
                .nlogl,
        );
        assert!(painted(&output, &nlogl), "NLogL value {nlogl}");
        let (text, status) = app
            .toolkits
            .app_status(Tool::App(AppKind::DistributionFitter), false)
            .unwrap();
        assert_eq!(status, toolkits::AppStatus::Success, "{text}");
        assert!(text.contains("lowest AIC: Fit 2"), "{text}");
        // Every display renders.
        for (label, display) in [
            ("Cumulative probability", Display::Cdf),
            ("Quantile", Display::Quantile),
            ("Probability plot", Display::Probability),
            ("Survivor", Display::Survivor),
            ("Cumulative hazard", Display::CumulativeHazard),
        ] {
            click_label_at(&mut app, &ctx, size, label);
            wait(&mut app, &ctx, size);
            let fitter = &app.toolkits.apps.distribution.app;
            assert_eq!(fitter.display, display);
            assert_eq!(
                fitter.shown.as_ref().unwrap().display,
                display,
                "{:?}",
                app.toolkits.apps.distribution.status()
            );
        }
        // The other Output views render.
        for label in ["Selected fit", "Evaluate"] {
            click_label_at(&mut app, &ctx, size, label);
            let output = frame_at(&mut app, &ctx, size, vec![]);
            assert!(painted(&output, label));
        }
        let output = frame_at(&mut app, &ctx, size, vec![]);
        assert!(painted(&output, "At x values"));
        // A narrow host stacks OUTPUT under the main view.
        let narrow = egui::vec2(900., 700.);
        let output = frame_at(&mut app, &ctx, narrow, vec![]);
        assert!(painted(&output, "OUTPUT"));
        app.close_tool_tab(Tool::App(AppKind::DistributionFitter));
        assert!(app.toolkits.tabs().next().is_none());
        assert!(app.toolkits.apps.distribution.app.shown.is_some());
    }

    #[test]
    fn distribution_fitter_export_creates_objects_without_clobbering() {
        let size = egui::vec2(1440., 900.);
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.run(format!("{DATA} pd1 = 7;"));
        finish(&mut app);
        {
            let fitter = &mut app.toolkits.apps.distribution.app;
            fitter.data_sets[0].data = "x".into();
            fitter.new_fit(Family::Weibull).unwrap();
        }
        open_fitter(&mut app, &ctx, size);
        let shown = app.toolkits.apps.distribution.app.fits[1]
            .result
            .clone()
            .unwrap();
        click_label_at(&mut app, &ctx, size, "Export to workspace");
        finish(&mut app);
        frame_at(&mut app, &ctx, size, vec![]);
        assert!(
            matches!(&app.variables["pd1"], matrust::Array::Numeric(m) if m.data() == [7.]),
            "pd1 is kept"
        );
        assert!(app.toolkits.apps.distribution.message.contains("pd2"));
        // The exported Weibull object (an engine object: read it in the
        // desktop's own engine) has the numbers the app shows.
        app.run("zz_values = pd3.ParameterValues; zz_nlogl = pd3.NLogL;".into());
        finish(&mut app);
        let read = |name: &str| match &app.variables[name] {
            matrust::Array::Numeric(m) => m.data().to_vec(),
            other => panic!("{name}: {other:?}"),
        };
        assert_eq!(read("zz_values"), shown.values, "{}", app.status);
        assert_eq!(read("zz_nlogl"), [shown.nlogl]);
        // pd2 and pd3 are taken now: the next defaults move on.
        frame_at(&mut app, &ctx, size, vec![]);
        let apps = &app.toolkits.apps;
        let names: Vec<String> = apps
            .distribution
            .app
            .object_names(&apps.workspace)
            .unwrap()
            .into_values()
            .collect();
        assert_eq!(names, ["pd4", "pd5"]);
    }

    #[test]
    fn distribution_fitter_generate_script_saves_header_and_the_same_script() {
        let size = egui::vec2(1440., 900.);
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fits.m");
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.run(DATA.into());
        finish(&mut app);
        app.toolkits.apps.distribution.app.data_sets[0].data = "x".into();
        open_fitter(&mut app, &ctx, size);
        let script = {
            let ws = &app.toolkits.apps.workspace;
            app.toolkits.apps.distribution.app.script(ws).unwrap()
        };
        app.toolkits.apps.test_save_path = Some(path.clone());
        click_label_at(&mut app, &ctx, size, "Generate script");
        finish(&mut app);
        frame_at(&mut app, &ctx, size, vec![]);
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(
            saved.starts_with("% Distribution Fitter (MatRust app), generated "),
            "{saved}"
        );
        assert!(saved.ends_with(&script), "{saved}");
        let mut engine = run_script(&saved);
        let shown = app.toolkits.apps.distribution.app.fits[0]
            .result
            .clone()
            .unwrap();
        assert_eq!(eval(&mut engine, "pd1.ParameterValues"), shown.values);
    }
}
