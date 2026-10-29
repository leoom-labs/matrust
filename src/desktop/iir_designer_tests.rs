use super::*;
use std::sync::atomic::AtomicBool;

#[test]
fn iir_designer_script_block_and_portable_export_retain_exact_coefficients() {
    for band in [Band::Lowpass, Band::Highpass] {
        for order in [1, 6, 20] {
            let tools = Toolkits {
                iir: true,
                order,
                band,
                analysis_points: 32,
                ..Default::default()
            };
            let request = tools.iir_design();
            let expected = matrust::dsp::butter_sos(order, 0.1, band).unwrap();
            let Block::SosFilter { sections } = request.block(&AtomicBool::new(false)).unwrap()
            else {
                panic!("SOS");
            };
            assert_eq!(sections.len(), order.div_ceil(2));
            for (r, row) in sections.iter().enumerate() {
                for (c, value) in row.iter().enumerate() {
                    assert_eq!(*value, expected.data()[c * expected.rows() + r]);
                }
            }
            let script = request.portable_script(&AtomicBool::new(false)).unwrap();
            assert!(!script.contains("butter_sos("));
            let mut engine = matrust::Engine::default();
            engine.execute(&script).unwrap();
            assert_eq!(engine.variables["sos"].data(), expected.data());
            engine.execute(&tools.preview_script().unwrap()).unwrap();
            assert_eq!(engine.variables["sos"].data(), expected.data());
            assert_eq!(engine.variables["response"].len(), 32);
        }
    }
    let tools = Toolkits {
        iir: true,
        order: 6,
        ..Default::default()
    };
    assert_eq!(
        tools.design_script().unwrap(),
        "sos = butter_sos(6, 0.1, 'low');"
    );
    assert!(tools.exported_script().unwrap_err().contains("background"));
}

#[test]
fn iir_designer_invalid_admission_and_cancelled_block_preserve_model() {
    let mut tools = Toolkits {
        iir: true,
        order: 6,
        ..Default::default()
    };
    let original = tools.model.clone();
    for (order, band, rate, cutoff) in [
        (0, Band::Lowpass, 1000., 50.),
        (21, Band::Lowpass, 1000., 50.),
        (6, Band::Bandpass, 1000., 50.),
        (6, Band::Bandstop, 1000., 50.),
        (6, Band::Lowpass, 0., 50.),
        (6, Band::Lowpass, f64::NAN, 50.),
        (6, Band::Lowpass, 1000., 0.),
        (6, Band::Lowpass, 1000., 500.),
        (6, Band::Lowpass, 1000., f64::INFINITY),
    ] {
        let request = crate::iir_designer::Design {
            family: crate::iir_designer::Family::Butterworth,
            ripple_db: 1.,
            order,
            band,
            sample_rate: rate,
            cutoff,
            upper: cutoff,
        };
        assert!(request.script().is_err());
        assert!(tools.begin_iir_block(request).is_err());
        assert!(tools.begin_iir_save(request).is_err());
        assert!(tools.pending_design_model.is_none() && !tools.saving_design);
        assert!(tools.model.same_storage(&original));
    }
    let request = tools.iir_design();
    tools.begin_iir_block(request).unwrap();
    assert!(!tools.finish_filter_block(request.block(&AtomicBool::new(true))));
    assert!(tools.model.same_storage(&original));
    assert!(tools.pending_design_model.is_none());
}

#[test]
fn iir_designer_block_worker_captures_settings_and_rejects_changed_model() {
    let mut tools = Toolkits {
        iir: true,
        order: 6,
        ..Default::default()
    };
    let request = tools.iir_design();
    tools.begin_iir_block(request).unwrap();
    tools.order = 20;
    tools.cutoff = 250.;
    let mut worker = crate::worker::Worker::new(egui::Context::default());
    worker
        .send
        .send(crate::worker::Job::IirBlock(request).into())
        .unwrap();
    let reply = worker
        .receive
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    assert!(tools.finish_filter_block(reply.fir_block.unwrap()));
    let Block::SosFilter { sections } = &tools.model.nodes.last().unwrap().block else {
        panic!("SOS");
    };
    assert_eq!(
        sections.len(),
        3,
        "worker uses the accepted order-6 request"
    );
    let next = tools.iir_design();
    tools.begin_iir_block(next).unwrap();
    tools.model.steps += 1;
    let changed = tools.model.clone();
    assert!(!tools.finish_filter_block(next.block(&AtomicBool::new(false))));
    assert!(tools.model.same_storage(&changed));
    worker.shutdown();
}

#[test]
fn iir_designer_portable_save_uses_worker_and_keeps_model_and_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("design.m");
    let mut tools = Toolkits {
        iir: true,
        order: 6,
        ..Default::default()
    };
    let original = tools.model.clone();
    let request = tools.iir_design();
    tools.begin_iir_save(request).unwrap();
    assert!(tools.begin_iir_save(request).is_err());
    assert!(tools.begin_iir_block(request).is_err());
    let mut worker = crate::worker::Worker::new(egui::Context::default());
    worker
        .send
        .send(
            crate::worker::Job::SaveIirDesign(Box::new(crate::iir_designer::Save {
                design: request,
                path: path.clone(),
            }))
            .into(),
        )
        .unwrap();
    let reply = worker
        .receive
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    assert!(reply.vars.is_empty());
    tools.finish_design_save(reply.design_save.unwrap());
    assert!(!tools.saving_design);
    assert!(tools.model.same_storage(&original));
    assert!(tools.message.starts_with("Saved"));
    let mut engine = matrust::Engine::default();
    engine
        .execute(&std::fs::read_to_string(path).unwrap())
        .unwrap();
    assert_eq!(
        engine.variables["sos"].data(),
        matrust::dsp::butter_sos(6, 0.1, Band::Lowpass)
            .unwrap()
            .data()
    );
    worker.shutdown();
}

#[test]
fn iir_designer_save_stop_is_visible_in_compact_window_and_does_not_change_model() {
    use super::fir_designer_tests::{frame, label};
    let ctx = egui::Context::default();
    let mut tools = Toolkits {
        iir: true,
        order: 6,
        designer_open: true,
        ..Default::default()
    };
    let original = tools.model.clone();
    tools.begin_iir_save(tools.iir_design()).unwrap();
    for _ in 0..5 {
        frame(&ctx, &mut tools, true, vec![]);
    }
    let rect = tools.designer_region().unwrap();
    assert!(
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640., 480.)).contains_rect(rect)
    );
    for _ in 0..12 {
        frame(
            &ctx,
            &mut tools,
            true,
            vec![
                egui::Event::PointerMoved(rect.center()),
                egui::Event::MouseWheel {
                    phase: egui::TouchPhase::Move,
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0., -100.),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    for _ in 0..20 {
        frame(&ctx, &mut tools, true, vec![]);
    }
    let output = frame(&ctx, &mut tools, true, vec![]).0;
    let pos = label(&output, "Stop filter design")
        .expect("IIR save Stop remains reachable")
        .center();
    let mut stopped = false;
    for pressed in [true, false] {
        let (_, action) = frame(
            &ctx,
            &mut tools,
            true,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        stopped |= matches!(action, Some(Action::StopModel));
    }
    assert!(stopped);
    tools.finish_design_save(Err("Filter operation cancelled".into()));
    assert!(!tools.saving_design && tools.model.same_storage(&original));
}
