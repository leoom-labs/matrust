//! Filter Designer: analysis views, outputs and the three-area layout.
//! Expected values: Butterworth −3.0103 dB at the cutoff (definition), sums
//! of impulse responses equal the DC gain; no MATLAB.
use super::designer_preview::{POINTS, Preview, prepare};
use super::*;
use crate::filter_spec::{Coefficients, Method, Response, Spec, Structure, Units};

fn spec(response: Response, method: Method, order: usize) -> Spec {
    let mut s = Spec::default();
    s.set_units(Units::Hz);
    s.fs = 1000.;
    s.reset_frequencies();
    s.set_response(response);
    s.set_method(method);
    s.minimum = false;
    s.order = order;
    s
}
fn run(spec: &Spec) -> Preview {
    prepare(&mut matrust::Engine::default(), spec).unwrap_or_else(|e| panic!("{e}: {spec:?}"))
}

#[test]
fn butterworth_views_have_the_textbook_values() {
    for structure in [Structure::Sos, Structure::Direct] {
        let mut s = spec(Response::Lowpass, Method::Butterworth, 4);
        s.fc = [125., 0.];
        s.structure = structure;
        let p = run(&s);
        assert_eq!(p.freq.len(), POINTS);
        assert!(p.magnitude_db[0].abs() < 1e-9);
        // 125 Hz of a 500 Hz Nyquist is grid point 256.
        assert!((p.freq[256] - 125.).abs() < 1e-12);
        assert!(
            (p.magnitude_db[256] + 3.0103).abs() < 1e-3,
            "{}",
            p.magnitude_db[256]
        );
        assert_eq!((p.order, p.poles.len(), p.zeros.len()), (4, 4, 4));
        assert!(p.stable);
        assert!(
            p.zeros
                .iter()
                .all(|z| (z[0] + 1.).abs() < 1e-3 && z[1].abs() < 1e-3)
        );
        let dc: f64 = p.impulse.iter().sum();
        assert!((dc - 1.).abs() < 1e-4, "{dc}");
        assert!((p.step.last().unwrap() - 1.).abs() < 1e-4);
        // Phase delay meets group delay at low frequency; both finite.
        assert!((p.phase_delay[1] - p.group_delay[1]).abs() < 0.05);
        assert!(p.group_delay.iter().all(|v| v.is_finite()));
        assert_eq!(p.phase_delay[0], p.group_delay[0]);
        assert!(p.phase_delay[0].is_finite());
        assert!(p.phase_delay[1..].iter().all(|v| v.is_finite()));
        let rows = super::designer_ui::info_rows(&p);
        assert!(rows.contains(&("Stable", "Yes".into())));
        let text = super::designer_ui::coefficient_text(&p);
        match structure {
            Structure::Sos => {
                assert!(text.starts_with("sos = [") && text.contains("g = "));
                assert!(rows.contains(&("Sections", "2".into())));
            }
            Structure::Direct => assert!(text.starts_with("b = [") && text.contains("\na = [")),
        }
    }
}

#[test]
fn review_1111_phase_delay_at_dc_is_the_group_delay() {
    // A symmetric FIR of length order+1 has delay order/2 at every frequency,
    // including DC (Oppenheim & Schafer). phasedelay fills w = 0 the same way.
    let mut s = spec(Response::Lowpass, Method::Window, 30);
    s.fc = [125., 0.];
    let p = run(&s);
    assert_eq!(p.phase_delay.len(), POINTS);
    assert!(
        (p.phase_delay[0] - 15.).abs() < 1e-6,
        "DC phase delay {}",
        p.phase_delay[0]
    );
    assert_eq!(p.phase_delay[0], p.group_delay[0]);
}
#[test]
fn direct_form_and_sections_give_the_same_response() {
    let mut s = spec(Response::Bandpass, Method::Elliptic, 4);
    s.fpass = [100., 200.];
    let sos = run(&s);
    s.structure = Structure::Direct;
    let direct = run(&s);
    for (a, b) in sos.magnitude_db.iter().zip(&direct.magnitude_db) {
        assert!((a - b).abs() < 1e-6 || *a < -100., "{a} vs {b}");
    }
}

#[test]
fn fir_impulse_is_its_taps() {
    let mut s = spec(Response::Lowpass, Method::Window, 30);
    s.fc = [100., 0.];
    let p = run(&s);
    let Coefficients::Tf { b, .. } = &p.coefficients else {
        panic!()
    };
    assert_eq!(p.impulse.len(), 31);
    for (h, b) in p.impulse.iter().zip(b) {
        assert!((h - b).abs() < 1e-12);
    }
    // Linear phase: group delay is order/2 samples everywhere it is defined.
    assert!((p.group_delay[10] - 15.).abs() < 1e-6);
    assert!(super::designer_ui::info_rows(&p).contains(&("Type", "FIR".into())));
}

#[test]
fn notch_removes_50_hz_at_fs_1_khz() {
    let mut s = spec(Response::Notch, Method::NotchIir, 2);
    s.f0 = 50.;
    s.bandwidth = 5.;
    let p = run(&s);
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let h = p.coefficients.response(&[0.1, 0.0, 0.5], &cancel).unwrap();
    assert!(h[0][0].hypot(h[0][1]) < 1e-9, "{:?}", h[0]);
    assert!((h[1][0].hypot(h[1][1]) - 1.).abs() < 1e-6);
    assert!(p.stable);
}

#[test]
fn minimum_order_elliptic_bandpass_meets_its_mask() {
    let mut s = spec(Response::Bandpass, Method::Elliptic, 4);
    s.minimum = true;
    s.fpass = [100., 200.];
    s.fstop = [80., 230.];
    s.apass = 1.;
    s.astop = 60.;
    let p = run(&s);
    assert_eq!(p.mask.len(), 3);
    let nyquist = 500.;
    for band in &p.mask {
        let db = band.db.unwrap();
        for (f, m) in p.freq.iter().zip(&p.magnitude_db) {
            let f = f / nyquist;
            if f < band.f0 || f > band.f1 {
                continue;
            }
            if band.pass {
                assert!(*m >= -db - 1e-6 && *m <= 1e-6, "pass {f}: {m}");
            } else {
                assert!(*m <= -db + 1e-6, "stop {f}: {m}");
            }
        }
    }
}

#[test]
fn block_folds_the_gain_into_the_first_section() {
    let mut s = spec(Response::Highpass, Method::Chebyshev1, 4);
    s.fpass = [100., 0.];
    let p = run(&s);
    let Coefficients::Sos { sos, g } = &p.coefficients else {
        panic!()
    };
    let matrust::simulation::Block::SosFilter { sections } =
        crate::designer_output::block(&p.coefficients).unwrap()
    else {
        panic!()
    };
    assert_eq!(sections.len(), sos.len());
    assert!((sections[0][0] - sos[0][0] * g).abs() < 1e-15);
    assert_eq!(sections[1], sos[1]);
}

#[test]
fn apply_and_export_scripts_run_in_the_engine() {
    let signal = "x = sin(2*pi*(0:199)/20) + sin(2*pi*(0:199)/3);";
    for structure in [Structure::Sos, Structure::Direct] {
        let mut s = spec(Response::Lowpass, Method::Butterworth, 4);
        s.fc = [100., 0.];
        s.structure = structure;
        let script = s.script().unwrap();
        for zero_phase in [false, true] {
            let mut engine = matrust::Engine::default();
            engine.execute(signal).unwrap();
            engine
                .execute(&s.apply_script(&script, "x", zero_phase))
                .unwrap();
            assert_eq!(engine.variables["y"].len(), 200);
            engine.execute("[b0, a0] = butter(4, 0.2);").unwrap();
            let call = if zero_phase { "filtfilt" } else { "filter" };
            engine.execute(&format!("y0 = {call}(b0, a0, x);")).unwrap();
            let (y, y0) = (engine.variables["y"].data(), engine.variables["y0"].data());
            for (a, b) in y.iter().zip(y0) {
                assert!((a - b).abs() < 1e-9, "{structure:?} {call}: {a} vs {b}");
            }
        }
    }
}

#[test]
fn digital_filter_export_matches_the_design() {
    let x = "x = sin(2*pi*(0:99)/7);";
    let mut cases = Vec::new();
    for response in [Response::Lowpass, Response::Bandstop] {
        let mut s = spec(response, Method::Butterworth, 4);
        s.structure = Structure::Direct;
        cases.push(s.clone());
        s.set_method(Method::Chebyshev2);
        s.minimum = false;
        s.structure = Structure::Direct;
        cases.push(s.clone());
        s.set_method(Method::Window);
        s.minimum = false;
        cases.push(s);
    }
    for s in cases {
        let call = s.designfilt().unwrap_or_else(|| panic!("{s:?}"));
        let mut engine = matrust::Engine::default();
        engine
            .execute(&format!(
                "{x}{}{call}y1 = filter(b, a, x); y2 = filter(d, x);",
                s.script().unwrap()
            ))
            .unwrap_or_else(|e| panic!("{e}\n{call}"));
        let (y1, y2) = (engine.variables["y1"].data(), engine.variables["y2"].data());
        for (a, b) in y1.iter().zip(y2) {
            assert!((a - b).abs() < 1e-8, "{}: {a} vs {b}", s.title());
        }
    }
    let mut minimum = spec(Response::Lowpass, Method::Elliptic, 4);
    minimum.minimum = true;
    assert!(minimum.designfilt().is_none());
}

fn find_text(shapes: &[egui::epaint::ClippedShape], wanted: &str) -> Option<egui::Rect> {
    fn walk(shape: &egui::epaint::Shape, wanted: &str) -> Option<egui::Rect> {
        match shape {
            egui::epaint::Shape::Text(t) if t.galley.text() == wanted => {
                Some(t.galley.rect.translate(t.pos.to_vec2()))
            }
            egui::epaint::Shape::Vec(v) => v.iter().find_map(|s| walk(s, wanted)),
            _ => None,
        }
    }
    shapes.iter().find_map(|s| walk(&s.shape, wanted))
}

#[test]
fn three_areas_and_a_message_never_moves_the_fields() {
    let size = egui::vec2(1440., 900.);
    let ctx = egui::Context::default();
    let mut app = crate::Workspace::with_context(ctx.clone(), true);
    app.toolkits.open_designer_demo(true);
    *app.toolkits.spec_mut() = spec(Response::Lowpass, Method::Elliptic, 6);
    let frame = |app: &mut crate::Workspace| {
        let mut out = None;
        for _ in 0..3 {
            out = Some(crate::headless::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                },
                |root| app.ui(root),
            ));
        }
        out.unwrap().shapes
    };
    let shapes = frame(&mut app);
    let specs = find_text(&shapes, "SPECS").expect("SPECS area");
    let analysis = find_text(&shapes, "ANALYSIS").expect("ANALYSIS area");
    let output = find_text(&shapes, "OUTPUT").expect("OUTPUT area");
    assert!(specs.left() < analysis.left() && analysis.left() < output.left());
    for tab in super::designer_preview::View::ALL {
        assert!(find_text(&shapes, tab.label()).is_some(), "{tab:?} tab");
    }
    let astop = find_text(&shapes, "Astop (dB)").expect("Astop field");
    app.toolkits.spec_mut().apass = -1.;
    let message = app.toolkits.spec_mut().problems()[0].1.clone();
    let shapes = frame(&mut app);
    assert!(
        find_text(&shapes, &message).is_some(),
        "message shown: {message}"
    );
    assert_eq!(find_text(&shapes, "Astop (dB)"), Some(astop));
}

#[test]
fn apply_reads_an_input_named_like_a_design_variable() {
    for (structure, input) in [(Structure::Direct, "b"), (Structure::Sos, "sos")] {
        let mut s = spec(Response::Lowpass, Method::Butterworth, 4);
        s.structure = structure;
        let script = s.script().unwrap();
        let mut engine = matrust::Engine::default();
        engine
            .execute(&format!("{input} = sin(2*pi*(0:99)/20);"))
            .unwrap();
        engine
            .execute(&s.apply_script(&script, input, false))
            .unwrap();
        assert_eq!(engine.variables["y"].len(), 100, "{input}");
    }
}

#[test]
fn a_failed_or_stopped_apply_leaves_the_workspace_as_it_was() {
    let s = spec(Response::Lowpass, Method::Butterworth, 4);
    let apply = s.apply_script(&s.script().unwrap(), "missing", false);
    let mut engine = matrust::Engine::default();
    engine.execute("keep = 1;").unwrap();
    let job = |source: &str| crate::worker::Job::ApplyDesign(source.into(), std::env::temp_dir());
    assert!(crate::worker::perform(&mut engine, job(&apply)).is_err());
    assert_eq!(engine.variables.keys().collect::<Vec<_>>(), ["keep"]);
    engine
        .cancellation()
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let fine = s.apply_script(&s.script().unwrap(), "keep", false);
    assert!(crate::worker::perform(&mut engine, job(&fine)).is_err());
    assert_eq!(engine.variables.keys().collect::<Vec<_>>(), ["keep"]);
}

#[test]
fn a_long_fir_keeps_its_views_without_pole_zero_roots() {
    let mut s = spec(Response::Lowpass, Method::Window, 256);
    s.units = Units::Normalized;
    s.fc = [0.45, 0.];
    let p = run(&s);
    assert_eq!(p.magnitude_db.len(), POINTS);
    assert_eq!(p.impulse.len(), 257);
    assert!(crate::designer_output::block(&p.coefficients).is_ok());
}
