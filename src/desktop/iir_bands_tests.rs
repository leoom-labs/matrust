use super::*;
use std::sync::atomic::AtomicBool;

#[test]
fn single_section_iir_apply_button_always_uses_causal_sos() {
    use super::fir_designer_tests::{frame, label};
    for band in [
        Band::Lowpass,
        Band::Highpass,
        Band::Bandpass,
        Band::Bandstop,
    ] {
        let ctx = egui::Context::default();
        let mut tools = Toolkits {
            iir: true,
            order: 1,
            band,
            cutoff: 100.,
            upper: 200.,
            sample_rate: 1000.,
            input: "x".into(),
            designer_open: true,
            ..Default::default()
        };
        for _ in 0..8 {
            frame(&ctx, &mut tools, false, vec![]);
        }
        let output = frame(&ctx, &mut tools, false, vec![]).0;
        let button = label(&output, "Apply to y")
            .expect("Apply remains reachable")
            .center();
        let mut applied = None;
        for pressed in [true, false] {
            let (_, action) = frame(
                &ctx,
                &mut tools,
                false,
                vec![
                    egui::Event::PointerMoved(button),
                    egui::Event::PointerButton {
                        pos: button,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            if let Some(Action::Apply(design, input)) = action {
                applied = Some((design, input));
            }
        }
        let (design, input) = applied.expect("real Apply action");
        assert_eq!(input, "x");
        assert!(matches!(design, crate::designer_apply::Design::Iir(_)));
        let mut engine = matrust::Engine::default();
        engine.execute("x=sin((0:127)/7);").unwrap();
        crate::designer_apply::perform(
            &mut engine,
            crate::designer_apply::Request {
                design,
                input,
                folder: std::env::current_dir().unwrap(),
                view: None,
            },
        )
        .unwrap();
        let expected = matrust::dsp::sosfilt(
            &engine.variables["sos"],
            &engine.variables["x"],
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(engine.variables["sos"].rows(), 1);
        assert_eq!(engine.variables["y"].data(), expected.data());
    }
}

#[test]
fn iir_bands_preview_apply_block_and_export_share_coefficients() {
    let cancel = AtomicBool::new(false);
    for band in [Band::Bandpass, Band::Bandstop] {
        for order in [1, 3, 20] {
            let tools = Toolkits {
                iir: true,
                order,
                band,
                cutoff: 100.,
                upper: 200.,
                sample_rate: 1000.,
                analysis_points: 32,
                ..Default::default()
            };
            let design = tools.iir_design();
            assert!(
                design
                    .summary()
                    .unwrap()
                    .contains(&format!("result order {}", 2 * order))
            );
            let coefficients =
                matrust::dsp::butter_sos_with_cutoffs(order, &[0.2, 0.4], band, &cancel).unwrap();
            let Block::SosFilter { sections } = design.block(&cancel).unwrap() else {
                panic!("SOS block required")
            };
            assert_eq!(sections.len(), order);
            for (row, values) in sections.iter().enumerate() {
                for (column, &value) in values.iter().enumerate() {
                    assert_eq!(value.to_bits(), coefficients.get(row, column).to_bits());
                }
            }
            let mut engine = matrust::Engine::default();
            engine.execute(&tools.preview_script().unwrap()).unwrap();
            assert_eq!(engine.variables["sos"].data(), coefficients.data());
            engine
                .execute("x=sin((0:255)/7); y=sosfilt(sos,x);")
                .unwrap();
            assert!(engine.variables["y"].data().iter().all(|x| x.is_finite()));
            let export = design.portable_script(&cancel).unwrap();
            engine.execute(&export).unwrap();
            assert_eq!(engine.variables["sos"].data(), coefficients.data());
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("band.m");
            crate::iir_designer::save(
                crate::iir_designer::Save {
                    design,
                    path: path.clone(),
                },
                &cancel,
            )
            .unwrap();
            assert_eq!(std::fs::read_to_string(path).unwrap(), export);
        }
    }
}

#[test]
fn iir_bands_admission_and_worker_requests_preserve_pending_model() {
    let mut tools = Toolkits {
        iir: true,
        band: Band::Bandstop,
        order: 4,
        cutoff: 100.,
        upper: 200.,
        sample_rate: 1000.,
        ..Default::default()
    };
    let original = tools.model.clone();
    let request = tools.iir_design();
    for upper in [100., 0., 500., f64::INFINITY, f64::NAN] {
        let invalid = crate::iir_designer::Design { upper, ..request };
        assert!(tools.begin_iir_block(invalid).is_err());
        assert!(tools.begin_iir_save(invalid).is_err());
        assert!(tools.model.same_storage(&original));
    }
    tools.begin_iir_block(request).unwrap();
    tools.upper = 300.;
    tools.band = Band::Bandpass;
    let result = request.block(&AtomicBool::new(false));
    assert!(tools.finish_filter_block(result));
    let Block::SosFilter { sections } = &tools.model.nodes.last().unwrap().block else {
        panic!("SOS")
    };
    let expected = matrust::dsp::butter_sos_with_cutoffs(
        4,
        &[0.2, 0.4],
        Band::Bandstop,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(sections[0][0], expected.get(0, 0));
    let changed = tools.model.clone();
    let next = tools.iir_design();
    tools.begin_iir_block(next).unwrap();
    assert!(!tools.finish_filter_block(next.block(&AtomicBool::new(true))));
    assert!(tools.model.same_storage(&changed));
    // An unused upper edge does not affect scalar lowpass admission.
    let large = crate::iir_designer::Design {
        sample_rate: f64::MAX,
        cutoff: f64::MAX * 0.1,
        upper: f64::MAX * 0.6,
        band: Band::Lowpass,
        ..request
    };
    assert!(large.validate().is_ok());
    assert!(
        crate::iir_designer::Design {
            band: Band::Bandpass,
            ..large
        }
        .validate()
        .is_err()
    );
}
