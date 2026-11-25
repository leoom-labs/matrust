// Signal Analyzer app (apps/signal_analyzer.rs): the generated script
// reproduces the spectrum, derived signals and measurements the app shows,
// every preprocessing step is a script line, timetables bring their rate,
// Extract region, export names, errors, the saved script and a rendered tab.
mod signal_analyzer_tests {
    use super::*;
    use apps::signal_analyzer::{
        Analysis, Method, SignalAnalyzer, Step, StepKind, Taper, View,
    };
    use apps::{App, AppKind, Host, Snapshot};
    use toolkits::Tool;

    /// Two sines (50 Hz and 120 Hz) sampled at 1000 Hz plus seeded noise.
    const SETUP: &str = "rng(42); n = 0:1999; \
        x = sin(2*pi*50*n/1000) + 0.5*sin(2*pi*120*n/1000) + 0.2*randn(1, 2000); clear n";

    fn snapshot(engine: &matrust::Engine) -> Snapshot {
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        ws
    }
    fn engine(code: &str) -> matrust::Engine {
        let mut engine = matrust::Engine::default();
        engine
            .execute(code)
            .unwrap_or_else(|e| panic!("{code}\n{e}"));
        engine
    }
    /// One private run, synchronously, delivered to the app.
    fn analyse(host: &mut Host<SignalAnalyzer>, ws: &Snapshot) -> Result<(), String> {
        host.app.prepare(ws);
        let request = host.request(ws)?;
        let outputs = apps::run(&mut matrust::Engine::default(), request)?;
        host.app.receive(&outputs)
    }
    fn analysis(host: &Host<SignalAnalyzer>) -> &Analysis {
        host.app.current.as_ref().expect("an analysis")
    }
    fn data(engine: &matrust::Engine, name: &str) -> Vec<f64> {
        match engine.array(name) {
            Some(matrust::Array::Numeric(m)) => m.data().to_vec(),
            other => panic!("{name}: {other:?}"),
        }
    }
    fn number(engine: &matrust::Engine, name: &str) -> f64 {
        data(engine, name)[0]
    }
    fn set_rate(app: &mut SignalAnalyzer, name: &str, hz: &str) {
        let rate = app.rates.entry(name.into()).or_default();
        rate.text = hz.into();
        rate.edited = true;
    }
    /// The app at 1000 Hz on x, analysed.
    fn analyser(ws: &Snapshot) -> Host<SignalAnalyzer> {
        let mut host = Host::new(SignalAnalyzer::default());
        host.app.prepare(ws);
        assert_eq!(host.app.selected.as_deref(), Some("x"));
        set_rate(&mut host.app, "x", "1000");
        analyse(&mut host, ws).unwrap();
        host
    }
    fn add(
        host: &mut Host<SignalAnalyzer>,
        ws: &Snapshot,
        kind: StepKind,
    ) -> Result<String, String> {
        host.app.step_kind = kind;
        let step = host.app.form_step(ws)?;
        host.app.add_step(ws, step)
    }

    #[test]
    fn signal_analyzer_generated_script_reproduces_spectrum_derived_signals_and_measurements() {
        let ws = snapshot(&engine(SETUP));
        let mut host = analyser(&ws);
        // pwelch defaults on 2000 samples: 444-sample Hamming segments and
        // nfft 512, so bins lie 1000/512 Hz apart and the peak is the 50 Hz sine.
        let a = analysis(&host);
        assert_eq!((a.f.len(), a.pxx.len()), (257, 257));
        assert_eq!(*a.f.last().unwrap(), 500.);
        assert!(
            (a.peak_frequency - 50.).abs() <= 1000. / 512.,
            "{}",
            a.peak_frequency
        );
        let at = |a: &Analysis, hz: f64| {
            let k =
                a.f.iter()
                    .position(|f| (f - hz).abs() <= 1000. / 1024.)
                    .unwrap();
            a.pxx[k]
        };
        let tone120 = at(a, 120.);
        // Lowpass at 80 Hz: a new signal x_lp, x kept.
        host.app.edge = "80".into();
        assert_eq!(add(&mut host, &ws, StepKind::Lowpass).unwrap(), "x_lp");
        assert_eq!(host.app.selected.as_deref(), Some("x_lp"));
        assert!(host.app.ticked.contains("x") && host.app.ticked.contains("x_lp"));
        analyse(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.starts_with(
                "x_lp = lowpass(x, 80, 1000);\n\
                 [pxx, f] = pwelch(x_lp, [], [], [], 1000);\n\
                 [~, fsg, tsg, psg] = spectrogram(x_lp, [], [], [], 1000);\n\
                 figure\n"
            ),
            "{script}"
        );
        assert!(
            script.contains("plot((0:numel(x)-1)/1000, x, (0:numel(x_lp)-1)/1000, x_lp)"),
            "{script}"
        );
        let a = analysis(&host).clone();
        let a = &a;
        assert!((a.peak_frequency - 50.).abs() <= 1000. / 512.);
        // 120 Hz lies in the filter's transition band: down by more than 10 dB.
        assert!(at(a, 120.) < tone120 / 10., "the 120 Hz tone is filtered");
        // One line per plotted signal; the selected one is the exported pxx.
        let names: Vec<&str> = a.spectra.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["x", "x_lp"]);
        // The script in a fresh Engine: the same pxx/f, derived signal and
        // measurements, bit for bit.
        let fresh = engine(&format!("{SETUP}\n{script}"));
        assert_eq!(data(&fresh, "pxx"), a.pxx);
        assert_eq!(data(&fresh, "f"), a.f);
        let direct = engine(&format!("{SETUP}\ny = lowpass(x, 80, 1000);"));
        assert_eq!(data(&fresh, "x_lp"), data(&direct, "y"));
        let measured = engine(&format!(
            "{SETUP}\n{script}\n\
             m_n = numel(x_lp); m_min = min(x_lp); m_max = max(x_lp); m_mean = mean(x_lp);\n\
             m_rms = rms(x_lp); m_std = std(x_lp); m_p2p = max(x_lp) - min(x_lp);\n\
             m_rms2 = sqrt(mean(x_lp.^2));"
        ));
        assert_eq!(number(&measured, "m_n"), a.samples);
        assert_eq!(a.samples, 2000.);
        assert_eq!(a.duration, 2.);
        assert_eq!(number(&measured, "m_min"), a.min);
        assert_eq!(number(&measured, "m_max"), a.max);
        assert_eq!(number(&measured, "m_mean"), a.mean);
        assert_eq!(number(&measured, "m_rms"), a.rms);
        assert!((number(&measured, "m_rms2") - a.rms).abs() < 1e-12);
        assert_eq!(number(&measured, "m_std"), a.std);
        assert_eq!(number(&measured, "m_p2p"), a.peak_to_peak);
        // The Output column shows those numbers.
        let rows = host.app.results();
        assert_eq!(rows[0], ("Signal", "x_lp".to_owned()));
        assert!(rows.contains(&("RMS", apps::short(a.rms))));
        assert!(rows.contains(&("Peak-to-peak", apps::short(a.peak_to_peak))));
        assert!(!rows.iter().any(|(label, _)| *label == "Peak frequency"));
        host.app.view = View::Spectrum;
        let rows = host.app.results();
        assert!(rows.contains(&(
            "Peak frequency",
            format!("{} Hz", apps::short(a.peak_frequency))
        )));
        assert!(rows.contains(&("Peak power", format!("{} dB/Hz", apps::short(a.peak_db)))));
        // The spectrogram the app draws is the script's psg in dB.
        let image = engine(&format!("{SETUP}\n{script}\nsgdb = 10*log10(psg + eps);"));
        assert_eq!(a.image.db.data(), data(&image, "sgdb"));
        assert_eq!((a.image.rows, a.image.cols), (257, 8));
        assert_eq!(a.image.f, data(&fresh, "fsg"));
        assert_eq!(a.image.t, data(&fresh, "tsg"));
    }

    #[test]
    fn signal_analyzer_each_preprocessing_step_is_a_script_line() {
        let ws = snapshot(&engine(SETUP));
        let mut host = analyser(&ws);
        type Set = fn(&mut SignalAnalyzer);
        let cases: [(StepKind, Set, &str); 7] = [
            (StepKind::Detrend, |a| a.linear = true, "x_dt = detrend(x);"),
            (
                StepKind::Detrend,
                |a| a.linear = false,
                "x_dt1 = detrend(x, 'constant');",
            ),
            (
                StepKind::Lowpass,
                |a| a.edge = "100".into(),
                "x_lp = lowpass(x, 100, 1000);",
            ),
            (
                StepKind::Highpass,
                |a| a.edge = "30".into(),
                "x_hp = highpass(x, 30, 1000);",
            ),
            (
                StepKind::Bandpass,
                |a| a.band = ["40".into(), "60".into()],
                "x_bp = bandpass(x, [40 60], 1000);",
            ),
            (
                StepKind::MovingMean,
                |a| a.span = "5".into(),
                "x_mm = movmean(x, 5);",
            ),
            (
                StepKind::Resample,
                |a| a.new_rate = "250".into(),
                "x_rs = resample(x, 1, 4);",
            ),
        ];
        for (kind, set, line) in cases {
            host.app.select("x");
            set(&mut host.app);
            let name = add(&mut host, &ws, kind).unwrap();
            assert!(line.starts_with(&format!("{name} = ")), "{name}: {line}");
            let script = host.app.script(&ws).unwrap();
            assert!(script.contains(&format!("{line}\n")), "{script}");
        }
        // A step on a derived signal chains from it.
        host.app.select("x_lp");
        host.app.span = "9".into();
        assert_eq!(
            add(&mut host, &ws, StepKind::MovingMean).unwrap(),
            "x_lp_mm"
        );
        let script = host.app.script(&ws).unwrap();
        assert!(script.contains("x_lp_mm = movmean(x_lp, 9);\n"), "{script}");
        assert_eq!(
            host.app.history("x_lp_mm"),
            "x → lowpass 100 Hz → moving mean of 9"
        );
        // Resampling gives the new rate; the script runs and leaves each signal.
        let rs = host.app.resolve(&ws, "x_rs").unwrap();
        assert_eq!(rs.fs, Some(250.));
        analyse(&mut host, &ws).unwrap();
        let fresh = engine(&format!("{SETUP}\n{script}"));
        assert_eq!(data(&fresh, "x_rs").len(), 500);
        for name in ["x_dt", "x_dt1", "x_lp", "x_hp", "x_bp", "x_mm", "x_lp_mm"] {
            assert_eq!(data(&fresh, name).len(), 2000, "{name}");
        }
        // The plotted spectra follow each signal's own rate.
        let a = analysis(&host);
        let rs_line = a.spectra.iter().find(|l| l.name == "x_rs").unwrap();
        assert_eq!(*rs_line.f.last().unwrap(), 125.);
        // Deleting x_lp deletes x_lp_mm made from it; x stays.
        assert_eq!(host.app.selected.as_deref(), Some("x_lp_mm"));
        let gone = host.app.delete("x_lp");
        assert_eq!(gone, ["x_lp", "x_lp_mm"]);
        assert_eq!(
            host.app.selected.as_deref(),
            Some("x"),
            "back to the source"
        );
        let script = host.app.script(&ws).unwrap();
        assert!(!script.contains("x_lp"), "{script}");
        assert!(host.app.order(&ws).contains(&"x".to_owned()));

        // Spectrum and spectrogram settings: only non-default arguments.
        host.app.select("x");
        host.app.welch.window = "256".into();
        host.app.welch.overlap = "50".into();
        host.app.welch.nfft = "512".into();
        host.app.spectrogram.window = "128".into();
        host.app.spectrogram.overlap = "75".into();
        let script = host.app.script(&ws).unwrap();
        assert!(script.contains("[pxx, f] = pwelch(x, 256, 128, 512, 1000);\n"));
        assert!(script.contains("[~, fsg, tsg, psg] = spectrogram(x, 128, 96, [], 1000);\n"));
        // An overlap without a window length keeps the default.
        host.app.welch.window.clear();
        assert!(
            host.app
                .script(&ws)
                .unwrap()
                .contains("[pxx, f] = pwelch(x, [], [], 512, 1000);\n")
        );
        host.app.method = Method::Periodogram;
        host.app.taper = Taper::Hann;
        analyse(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.contains("[pxx, f] = periodogram(x, hann(numel(x)), [], 1000);\n"),
            "{script}"
        );
        let fresh = engine(&format!("{SETUP}\n{script}"));
        assert_eq!(data(&fresh, "pxx"), analysis(&host).pxx);
        assert_eq!(analysis(&host).f.len(), 1025, "periodogram nfft 2048");
        host.app.taper = Taper::Rectangular;
        host.app.periodogram_nfft = "4096".into();
        assert!(
            host.app
                .script(&ws)
                .unwrap()
                .contains("[pxx, f] = periodogram(x, [], 4096, 1000);\n")
        );
        // Samples units: steps made in Hz say they need a rate.
        host.app.method = Method::Welch;
        host.app.welch = Default::default();
        host.app.spectrogram = Default::default();
        host.app.periodogram_nfft.clear();
        host.app.rates.get_mut("x").unwrap().samples = true;
        let error = host.app.script(&ws).unwrap_err();
        assert!(
            error.contains("x_hp: x has no sample rate, so highpass 30 Hz cannot run"),
            "{error}"
        );
        // Without them: no rate in any call, trailing defaults dropped,
        // normalized passbands and rate factors.
        for name in ["x_dt", "x_dt1", "x_lp", "x_hp", "x_bp", "x_mm", "x_rs"] {
            host.app.delete(name);
        }
        let script = host.app.script(&ws).unwrap();
        assert!(script.starts_with("[pxx, f] = pwelch(x);\n"), "{script}");
        assert!(script.contains("spectrogram(x, [], [], []);\n"), "{script}");
        assert!(
            script.contains("plot(0:numel(x)-1, x)\nxlabel('Samples')"),
            "{script}"
        );
        host.app.edge = "1.5".into();
        let error = add(&mut host, &ws, StepKind::Lowpass).unwrap_err();
        assert!(error.contains("below the Nyquist frequency 1"), "{error}");
        host.app.edge = "0.2".into();
        add(&mut host, &ws, StepKind::Lowpass).unwrap();
        host.app.select("x");
        host.app.new_rate = "0.5".into();
        add(&mut host, &ws, StepKind::Resample).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.starts_with("x_lp = lowpass(x, 0.2);\nx_rs = resample(x, 1, 2);\n"),
            "{script}"
        );
        assert_eq!(host.app.history("x_rs"), "x → resampled ×0.5");
        host.app.select("x");
        analyse(&mut host, &ws).unwrap();
        let a = analysis(&host);
        assert!((a.f.last().unwrap() - std::f64::consts::PI).abs() < 1e-12);
        let rows = host.app.results();
        assert!(
            rows.contains(&("Duration", "2000 samples".to_owned())),
            "{rows:?}"
        );
    }

    #[test]
    fn signal_analyzer_timetable_input_uses_its_row_time_rate() {
        let ws = snapshot(&engine(
            "n = (0:999)'; tt = timetable(seconds(n/500), sin(2*pi*60*n/500)); \
             td = timetable(datetime(2026, 1, 1) + seconds((0:99)'/200), (1:100)'); \
             two = timetable(seconds(n/500), n, 2*n); clear n",
        ));
        let mut host = Host::new(SignalAnalyzer::default());
        host.app.prepare(&ws);
        // One-variable timetables are signals; two variables are not.
        assert_eq!(host.app.order(&ws), ["td", "tt"]);
        assert_eq!(host.app.rates["tt"].text, "500");
        assert_eq!(host.app.rates["td"].text, "200");
        host.app.select("tt");
        analyse(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.contains("[pxx, f] = pwelch(tt.Var1, [], [], [], 500);\n"),
            "{script}"
        );
        assert_eq!(host.app.inputs(), ["td", "tt"]);
        let a = analysis(&host);
        assert_eq!(*a.f.last().unwrap(), 250.);
        assert!(
            (a.peak_frequency - 60.).abs() <= 500. / 256.,
            "{}",
            a.peak_frequency
        );
        assert_eq!(a.duration, 2.);
        // A derived signal of a timetable reads its variable.
        host.app.edge = "100".into();
        assert_eq!(add(&mut host, &ws, StepKind::Lowpass).unwrap(), "tt_lp");
        assert!(
            host.app
                .script(&ws)
                .unwrap()
                .starts_with("tt_lp = lowpass(tt.Var1, 100, 500);\n")
        );
        analyse(&mut host, &ws).unwrap();
        // A typed rate wins over the row times.
        set_rate(&mut host.app, "tt", "1000");
        assert!(
            host.app
                .script(&ws)
                .unwrap()
                .starts_with("tt_lp = lowpass(tt.Var1, 100, 1000);\n")
        );
    }

    #[test]
    fn signal_analyzer_extract_region_makes_x_i1_i2() {
        let ws = snapshot(&engine(SETUP));
        let mut host = analyser(&ws);
        // Not zoomed: nothing to extract.
        host.app.view_range = Some([-0.1, 2.1]);
        assert_eq!(host.app.region(), None);
        // The visible 0.5 s .. 0.9 s holds samples 501 to 901.
        host.app.view_range = Some([0.5, 0.9]);
        assert_eq!(host.app.region(), Some((501, 901)));
        host.app.extract_requested = true;
        analyse(&mut host, &ws).unwrap();
        assert_eq!(host.app.selected.as_deref(), Some("x_ext"));
        assert_eq!(
            host.app.derived("x_ext").unwrap().step,
            Step::Extract(501, 901)
        );
        let script = host.app.script(&ws).unwrap();
        assert!(script.starts_with("x_ext = x(501:901);\n"), "{script}");
        assert!(
            script.contains("0.5 + (0:numel(x_ext)-1)/1000, x_ext)"),
            "the region keeps its times: {script}"
        );
        let a = analysis(&host);
        assert_eq!(a.samples, 401.);
        let trace = a.traces.iter().find(|t| t.name == "x_ext").unwrap();
        assert_eq!(trace.start, 0.5);
        let fresh = engine(&format!("{SETUP}\n{script}\ny = x(501:901);"));
        assert_eq!(data(&fresh, "x_ext"), data(&fresh, "y"));
        assert_eq!(data(&fresh, "x_ext"), trace.y.data());
        // A region of the extracted signal counts from its own first sample.
        host.app.view_range = Some([0.6, 0.7]);
        assert_eq!(host.app.region(), Some((101, 201)));
        // In samples units the region starts at sample 500, and the plotted
        // range keeps its parentheses (the colon binds loosest).
        host.app.rates.get_mut("x").unwrap().samples = true;
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.contains("plot(0:numel(x)-1, x, 500 + (0:numel(x_ext)-1), x_ext)"),
            "{script}"
        );
        engine(&format!("{SETUP}\n{script}"));
    }

    #[test]
    fn signal_analyzer_errors_name_the_problem_and_keep_the_last_analysis() {
        let mut eng = engine(&format!("{SETUP}\nnote = 'abc';"));
        let ws = snapshot(&eng);
        let mut host = analyser(&ws);
        // A variable that is not a numeric vector.
        host.app.select("note");
        let error = host.request(&ws).err().unwrap();
        assert!(error.contains("note is not a numeric vector"), "{error}");
        assert!(
            host.app.order(&ws).contains(&"note".to_owned()),
            "listed to untick"
        );
        host.app.select("x");
        host.app.ticked.remove("note");
        // Passbands at or above Nyquist are refused before any run.
        host.app.edge = "600".into();
        let error = add(&mut host, &ws, StepKind::Lowpass).unwrap_err();
        assert!(error.contains("Nyquist frequency 500 Hz"), "{error}");
        host.app.band = ["100".into(), "700".into()];
        let error = add(&mut host, &ws, StepKind::Bandpass).unwrap_err();
        assert!(error.contains("Nyquist"), "{error}");
        host.app.band = ["60".into(), "40".into()];
        assert!(add(&mut host, &ws, StepKind::Bandpass).is_err());
        host.app.edge = "fast".into();
        assert!(add(&mut host, &ws, StepKind::Highpass).is_err());
        assert!(host.app.derived.is_empty());
        // A rate change can put an existing step above Nyquist: the engine's
        // message reaches the status line through the threaded runner, and
        // the plots keep the last good analysis.
        host.app.edge = "400".into();
        add(&mut host, &ws, StepKind::Lowpass).unwrap();
        analyse(&mut host, &ws).unwrap();
        let good = analysis(&host).rms;
        set_rate(&mut host.app, "x", "500");
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
        assert!(text.contains("strictly inside Nyquist"), "{text}");
        assert_eq!(analysis(&host).rms, good);
        set_rate(&mut host.app, "x", "1000");
        // Bad spectral fields and a window longer than the signal.
        host.app.welch.window = "many".into();
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("pwelch window length"), "{error}");
        host.app.welch.window = "5000".into();
        let error = analyse(&mut host, &ws).unwrap_err();
        assert!(error.contains("window longer than x"), "{error}");
        host.app.welch.window.clear();
        // A derived signal whose source left the workspace.
        eng.execute("clear x").unwrap();
        let ws = snapshot(&eng);
        host.app.prepare(&ws);
        host.app.selected = Some("x_lp".into());
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("x is no longer in the workspace"), "{error}");
        // Output names that would replace a signal are refused.
        let ws = snapshot(&engine(SETUP));
        host.app.pxx_name = "x".into();
        host.app.pxx_edited = true;
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("is a signal's name"), "{error}");
    }

    fn open_signal_analyzer(
        app: &mut Workspace,
        ctx: &egui::Context,
        size: egui::Vec2,
    ) -> egui::FullOutput {
        app.toolkits.open_tool(Tool::App(AppKind::SignalAnalyzer));
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let output = frame_at(app, ctx, size, vec![]);
            let host = &app.toolkits.apps.signal;
            if host.app.current.is_some() && !host.running() {
                return frame_at(app, ctx, size, vec![]);
            }
            assert!(Instant::now() < deadline, "{:?}", host.status());
            drop(output);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    /// Waits for the app's run of its current inputs.
    fn settle(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2) -> egui::FullOutput {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let output = frame_at(app, ctx, size, vec![]);
            if !app.toolkits.apps.signal.running() {
                return frame_at(app, ctx, size, vec![]);
            }
            assert!(Instant::now() < deadline);
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
    fn texts(shape: &egui::epaint::Shape, found: &mut Vec<String>) {
        match shape {
            egui::epaint::Shape::Text(t) => found.push(t.galley.text().to_owned()),
            egui::epaint::Shape::Vec(v) => v.iter().for_each(|s| texts(s, found)),
            _ => {}
        }
    }
    fn painted_prefix(output: &egui::FullOutput, prefix: &str) -> Option<String> {
        let mut found = Vec::new();
        for s in &output.shapes {
            texts(&s.shape, &mut found);
        }
        found.into_iter().find(|t| t.starts_with(prefix))
    }

    #[test]
    fn signal_analyzer_tab_renders_three_areas_and_every_view() {
        let size = egui::vec2(1440., 900.);
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.run(SETUP.into());
        finish(&mut app);
        set_rate(&mut app.toolkits.apps.signal.app, "x", "1000");
        let output = open_signal_analyzer(&mut app, &ctx, size);
        assert_eq!(
            app.toolkits.selected_tool(),
            Some(Tool::App(AppKind::SignalAnalyzer))
        );
        for label in [
            "INPUTS",
            "SIGNAL",
            "OUTPUT",
            "Time",
            "Spectrum",
            "Spectrogram",
            "Signals",
            "Sample rate of x",
            "Preprocess x",
            "Make x_lp",
            "Stacked",
            "Extract region",
            "Export to workspace",
            "Generate script",
            "RMS",
            "Spectrum power name",
        ] {
            assert!(painted(&output, label), "{label} not painted");
        }
        let rms = apps::short(analysis(&app.toolkits.apps.signal).rms);
        assert!(painted(&output, &rms), "RMS value {rms}");
        // The cursor readout: t1, t2, their distance and its inverse.
        let [t1, t2] = app.toolkits.apps.signal.app.cursors.unwrap();
        let readout = painted_prefix(&output, "t1 ").expect("cursor readout");
        assert_eq!(
            readout,
            format!(
                "t1 {} s · t2 {} s · Δt {} s · 1/Δt {} Hz",
                apps::short(t1),
                apps::short(t2),
                apps::short(t2 - t1),
                apps::short(1. / (t2 - t1))
            )
        );
        assert!(t1 < t2 && t1 > 0. && t2 < 2.);
        let (text, status) = app
            .toolkits
            .app_status(Tool::App(AppKind::SignalAnalyzer), false)
            .unwrap();
        assert_eq!(status, toolkits::AppStatus::Success);
        assert!(text.starts_with("x · 2000 samples at 1000 Hz"), "{text}");
        // Preprocess through the UI: type a passband edge and make x_lp.
        app.toolkits.apps.signal.app.edge = "80".into();
        click_label_at(&mut app, &ctx, size, "Make x_lp");
        let output = settle(&mut app, &ctx, size);
        assert_eq!(
            app.toolkits.apps.signal.app.selected.as_deref(),
            Some("x_lp")
        );
        assert!(painted(&output, "Derived signal x_lp"));
        assert!(painted(&output, "x → lowpass 80 Hz"));
        // Every view renders; Spectrum adds the peak rows.
        click_label_at(&mut app, &ctx, size, "Spectrum");
        let output = settle(&mut app, &ctx, size);
        assert_eq!(app.toolkits.apps.signal.app.view, View::Spectrum);
        for label in ["Peak frequency", "Peak power", "Welch", "Periodogram"] {
            assert!(painted(&output, label), "{label}");
        }
        click_label_at(&mut app, &ctx, size, "Spectrogram");
        let output = settle(&mut app, &ctx, size);
        assert_eq!(app.toolkits.apps.signal.app.view, View::Spectrogram);
        assert!(painted(&output, "dB"), "colour scale");
        assert!(painted(&output, "FFT length (nfft)"));
        click_label_at(&mut app, &ctx, size, "Time");
        click_label_at(&mut app, &ctx, size, "Stacked");
        let output = settle(&mut app, &ctx, size);
        assert!(app.toolkits.apps.signal.app.stacked);
        assert!(painted(&output, "x_lp"));
        // A narrow host stacks OUTPUT under the main view and still renders.
        let narrow = egui::vec2(900., 700.);
        let output = frame_at(&mut app, &ctx, narrow, vec![]);
        assert!(painted(&output, "OUTPUT"));
        // Closing the tab keeps the inputs for the next open.
        app.close_tool_tab(Tool::App(AppKind::SignalAnalyzer));
        assert!(app.toolkits.apps.signal.app.derived("x_lp").is_some());
    }

    #[test]
    fn signal_analyzer_export_creates_names_without_clobbering_and_saves_the_script() {
        let size = egui::vec2(1440., 900.);
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.run(format!("{SETUP}\npxx = 1; f = 2; fsg = 3; x_lp = 7;"));
        finish(&mut app);
        set_rate(&mut app.toolkits.apps.signal.app, "x", "1000");
        open_signal_analyzer(&mut app, &ctx, size);
        {
            let ws = &app.toolkits.apps.workspace;
            let host = &mut app.toolkits.apps.signal;
            host.app.edge = "80".into();
            // x_lp is taken: the derived signal becomes x_lp1.
            assert_eq!(add(host, ws, StepKind::Lowpass).unwrap(), "x_lp1");
        }
        settle(&mut app, &ctx, size);
        let shown = analysis(&app.toolkits.apps.signal).pxx.clone();
        // Generate script: the header and the same text, which reruns.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("signals.m");
        let script = {
            let ws = &app.toolkits.apps.workspace;
            app.toolkits.apps.signal.app.script(ws).unwrap()
        };
        assert!(script.contains("[pxx1, f1] = pwelch(x_lp1, "), "{script}");
        app.toolkits.apps.test_save_path = Some(path.clone());
        click_label_at(&mut app, &ctx, size, "Generate script");
        finish(&mut app);
        frame_at(&mut app, &ctx, size, vec![]);
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(
            saved.starts_with("% Signal Analyzer (MatRust app), generated "),
            "{saved}"
        );
        assert!(saved.ends_with(&script), "{saved}");
        assert!(
            app.toolkits.apps.signal.message.starts_with("Saved"),
            "{}",
            app.toolkits.apps.signal.message
        );
        assert!(
            app.toolkits.apps.pid.message.is_empty(),
            "PID Tuner untouched"
        );
        // Export: the same text in the workspace, under unused names. Its
        // figure takes the docked work area (as any plotting run does), so
        // the app tab is selected again to see the next run.
        click_label_at(&mut app, &ctx, size, "Export to workspace");
        finish(&mut app);
        for _ in 0..3 {
            frame_at(&mut app, &ctx, size, vec![]);
        }
        app.select_tool_tab(Tool::App(AppKind::SignalAnalyzer));
        settle(&mut app, &ctx, size);
        let value = |app: &Workspace, name: &str| match &app.variables[name] {
            matrust::Array::Numeric(m) => m.data().to_vec(),
            other => panic!("{name}: {}", other.class()),
        };
        for (name, kept) in [("pxx", 1.), ("f", 2.), ("fsg", 3.), ("x_lp", 7.)] {
            assert_eq!(value(&app, name), [kept], "{name} is kept");
        }
        assert_eq!(value(&app, "pxx1"), shown, "{}", app.status);
        assert_eq!(value(&app, "f1").len(), 257);
        assert_eq!(value(&app, "x_lp1").len(), 2000);
        for name in ["fsg1", "tsg", "psg"] {
            assert!(app.variables.contains_key(name), "{name}");
        }
        let message = &app.toolkits.apps.signal.message;
        assert!(
            message.contains("x_lp1") && message.contains("pxx1") && message.contains("f1"),
            "{message}"
        );
        // Taken now: the next defaults move on; x_lp1 shows once, as derived.
        {
            let ws = &app.toolkits.apps.workspace;
            let signal = &app.toolkits.apps.signal.app;
            assert_eq!(
                signal.spectrum_names(ws).unwrap(),
                ("pxx2".to_owned(), "f2".to_owned())
            );
            assert_eq!(signal.order(ws).iter().filter(|n| *n == "x_lp1").count(), 1);
        }
        // The app tab renders again after the exported script's figure.
        let output = settle(&mut app, &ctx, size);
        assert!(
            painted(&output, "Generate script"),
            "{:?}",
            app.toolkits.selected_tool()
        );
        let fresh = engine(&format!(
            "{SETUP}\npxx = 1; f = 2; fsg = 3; x_lp = 7;\n{saved}"
        ));
        let rerun = data(&fresh, "pxx1");
        assert_eq!(rerun, shown);
    }
}
