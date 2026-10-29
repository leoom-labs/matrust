use super::*;
use crate::fir_designer::WINDOWS;
use std::sync::atomic::AtomicBool;

#[test]
fn fir_designer_defaults_preserve_the_legacy_hamming_scaled_design() {
    let tools = Toolkits::default();
    assert_eq!(tools.fir_window, matrust::dsp::WindowKind::Hamming);
    assert!(tools.fir_scale);
    assert_eq!(tools.design_script().unwrap(), "b = fir1(64, 0.1, 'low');");
    let Block::Filter { b, a } = tools.fir_design().block(&AtomicBool::new(false)).unwrap() else {
        panic!("FIR block expected");
    };
    assert_eq!(b, matrust::dsp::fir1(64, &[0.1], Band::Lowpass).unwrap());
    assert_eq!(a, vec![1.]);
}

#[test]
fn fir_designer_nine_windows_and_scaling_match_script_preview_and_block_coefficients() {
    for (window, label, _) in WINDOWS {
        for band in [
            Band::Lowpass,
            Band::Highpass,
            Band::Bandpass,
            Band::Bandstop,
        ] {
            for scale in [false, true] {
                let tools = Toolkits {
                    fir_window: window,
                    fir_scale: scale,
                    band,
                    order: 7,
                    analysis_points: 32,
                    ..Default::default()
                };
                let request = tools.fir_design();
                let Block::Filter { b, .. } = request.block(&AtomicBool::new(false)).unwrap()
                else {
                    panic!("FIR");
                };
                assert_eq!(
                    b.len(),
                    if matches!(band, Band::Highpass | Band::Bandstop) {
                        9
                    } else {
                        8
                    }
                );
                let mut engine = matrust::Engine::default();
                engine.execute(&tools.preview_script().unwrap()).unwrap();
                assert_eq!(
                    engine.variables["b"].data(),
                    b.as_slice(),
                    "{label} {band:?} scale={scale}"
                );
                assert!(engine.figure.as_ref().unwrap().title.contains(label));
                assert_eq!(engine.variables["response"].len(), 32);
                let exported = tools.exported_script().unwrap();
                let mut reopened = matrust::Engine::default();
                reopened.execute(&exported).unwrap();
                assert_eq!(reopened.variables["b"].data(), b.as_slice());
            }
        }
    }
}

#[test]
fn fir_designer_invalid_designs_and_cancellation_do_not_publish_blocks() {
    let mut tools = Toolkits::default();
    let original = tools.model.clone();
    for (order, rate, cut, upper, band) in [
        (0, 1000., 50., 150., Band::Lowpass),
        (4097, 1000., 50., 150., Band::Lowpass),
        (64, 0., 50., 150., Band::Lowpass),
        (64, f64::NAN, 50., 150., Band::Lowpass),
        (64, 1000., 500., 150., Band::Lowpass),
        (64, 1000., f64::INFINITY, 150., Band::Lowpass),
        (64, 1000., 150., 50., Band::Bandpass),
    ] {
        tools.order = order;
        tools.sample_rate = rate;
        tools.cutoff = cut;
        tools.upper = upper;
        tools.band = band;
        assert!(tools.design_script().is_err());
        assert!(tools.request_fir_block().is_err());
        assert!(tools.pending_design_model.is_none());
        assert!(tools.model.same_storage(&original));
    }
    for (window, _, _) in WINDOWS {
        let tools = Toolkits {
            fir_window: window,
            ..Default::default()
        };
        assert!(tools.fir_design().block(&AtomicBool::new(true)).is_err());
    }
    let mut tools = Toolkits::default();
    let request = tools.request_fir_block().unwrap();
    assert!(!tools.finish_filter_block(request.block(&AtomicBool::new(true))));
    assert!(tools.model.same_storage(&original));
    assert!(tools.pending_design_model.is_none());
}

#[test]
fn fir_designer_zero_windows_boundary_lengths_and_mode_changes_are_explicit() {
    for window in [
        matrust::dsp::WindowKind::Hann,
        matrust::dsp::WindowKind::Blackman,
        matrust::dsp::WindowKind::Bartlett,
    ] {
        let mut tools = Toolkits {
            fir_window: window,
            order: 1,
            ..Default::default()
        };
        assert!(tools.design_script().unwrap_err().contains("all zero"));
        tools.fir_scale = false;
        let Block::Filter { b, .. } = tools.fir_design().block(&AtomicBool::new(false)).unwrap()
        else {
            panic!("FIR");
        };
        assert!(b.iter().all(|x| *x == 0.));
        tools.band = Band::Highpass;
        tools.fir_scale = true;
        assert!(tools.fir_design().block(&AtomicBool::new(false)).is_ok());
    }
    let mut tools = Toolkits {
        fir_window: matrust::dsp::WindowKind::Blackman,
        fir_scale: false,
        order: 4095,
        band: Band::Bandstop,
        ..Default::default()
    };
    assert!(tools.design_script().unwrap().contains("blackman(4097)"));
    tools.iir = true;
    tools.band = Band::Lowpass;
    tools.order = 6;
    assert!(!tools.design_script().unwrap().contains("blackman"));
    tools.iir = false;
    assert_eq!(tools.fir_window, matrust::dsp::WindowKind::Blackman);
    assert!(!tools.fir_scale);
    let mut extreme = tools.fir_design();
    extreme.sample_rate = f64::MAX;
    extreme.cutoff = f64::MAX / 4.;
    assert!(extreme.script().unwrap().contains("0.5"));
}

#[test]
fn fir_designer_worker_uses_existing_channel_and_model_publication_rejects_stale_results() {
    let mut tools = Toolkits {
        fir_window: matrust::dsp::WindowKind::Triangular,
        fir_scale: false,
        ..Default::default()
    };
    let original = tools.model.clone();
    let request = tools.request_fir_block().unwrap();
    assert!(tools.request_fir_block().is_err());
    let mut worker = crate::worker::Worker::new(egui::Context::default());
    worker
        .send
        .send(crate::worker::Job::FirBlock(request).into())
        .unwrap();
    let reply = worker
        .receive
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    assert!(tools.finish_filter_block(reply.fir_block.unwrap()));
    assert_eq!(tools.model.nodes.len(), original.nodes.len() + 1);
    assert!(tools.take_diagram_change());
    assert!(tools.message.contains("Triangular") && tools.message.contains("unscaled"));
    let request = tools.request_fir_block().unwrap();
    tools.model.steps += 1;
    let changed = tools.model.clone();
    assert!(!tools.finish_filter_block(request.block(&AtomicBool::new(false))));
    assert!(tools.model.same_storage(&changed));
    assert!(tools.message.contains("Model changed"));
    let request = tools.request_fir_block().unwrap();
    tools.file.path = Some(std::path::PathBuf::from("another-model.mrsim"));
    assert!(!tools.finish_filter_block(request.block(&AtomicBool::new(false))));
    assert!(tools.model.same_storage(&changed));
    worker.shutdown();
}

#[test]
fn fir_designer_normalization_toggle_changes_gain_and_failed_design_retains_workspace() {
    let mut tools = Toolkits {
        fir_window: matrust::dsp::WindowKind::Rectangular,
        order: 6,
        ..Default::default()
    };
    let Block::Filter { b: scaled, .. } =
        tools.fir_design().block(&AtomicBool::new(false)).unwrap()
    else {
        panic!("FIR");
    };
    tools.fir_scale = false;
    let Block::Filter { b: unscaled, .. } =
        tools.fir_design().block(&AtomicBool::new(false)).unwrap()
    else {
        panic!("FIR");
    };
    assert!((scaled.iter().sum::<f64>() - 1.).abs() < 1e-12);
    assert!((unscaled.iter().sum::<f64>() - 1.).abs() > 0.01);
    let mut engine = matrust::Engine::default();
    engine.execute("b = [42 43];").unwrap();
    // A worker-side normalization failure must not replace existing coefficients.
    assert!(
        engine
            .execute("b = fir1(1,0.1,'low',hann(2),'scale');")
            .is_err()
    );
    assert_eq!(engine.variables["b"].data(), &[42., 43.]);
}

pub(super) fn frame(
    ctx: &egui::Context,
    tools: &mut Toolkits,
    busy: bool,
    events: Vec<egui::Event>,
) -> (egui::FullOutput, Option<Action>) {
    let mut action = None;
    let output = crate::headless::run(
        ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640., 480.),
            )),
            events,
            ..Default::default()
        },
        |root| {
            action = tools.ui(root.ctx(), busy);
        },
    );
    (output, action)
}
pub(super) fn label(output: &egui::FullOutput, text: &str) -> Option<egui::Rect> {
    fn find(s: &egui::Shape, text: &str) -> Option<egui::Rect> {
        match s {
            egui::Shape::Text(t) if t.galley.text() == text => {
                Some(egui::Rect::from_min_size(t.pos, t.galley.size()))
            }
            egui::Shape::Vec(v) => v.iter().find_map(|s| find(s, text)),
            _ => None,
        }
    }
    output
        .shapes
        .iter()
        .find_map(|s| find(&s.shape, text).filter(|r| s.clip_rect.expand(0.1).contains_rect(*r)))
}
fn click(ctx: &egui::Context, tools: &mut Toolkits, busy: bool, pos: egui::Pos2) {
    for pressed in [true, false] {
        frame(
            ctx,
            tools,
            busy,
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
    }
}

#[test]
fn fir_designer_compact_controls_switch_all_windows_and_normalization() {
    let ctx = egui::Context::default();
    let mut tools = Toolkits {
        designer_open: true,
        ..Default::default()
    };
    for _ in 0..5 {
        frame(&ctx, &mut tools, false, vec![]);
    }
    for (kind, name, _) in WINDOWS {
        let output = frame(&ctx, &mut tools, false, vec![]).0;
        let selected = tools.fir_design().window_info().0;
        let rect = label(&output, selected).expect("selected window visible at 640x480");
        click(&ctx, &mut tools, false, rect.center());
        let output = frame(&ctx, &mut tools, false, vec![]).0;
        // Popup contains all nine choices; target may also be the selected value.
        let rect = output
            .shapes
            .iter()
            .rev()
            .find_map(|s| {
                let one = egui::FullOutput {
                    shapes: vec![s.clone()],
                    ..Default::default()
                };
                label(&one, name)
            })
            .expect("window choice visible");
        click(&ctx, &mut tools, false, rect.center());
        assert_eq!(tools.fir_window, kind);
    }
    let output = frame(&ctx, &mut tools, false, vec![]).0;
    click(
        &ctx,
        &mut tools,
        false,
        label(&output, "Scale passband gain").unwrap().center(),
    );
    assert!(!tools.fir_scale);
    assert!(tools.design_script().unwrap().contains("noscale"));
    let rect = tools.designer_region().unwrap();
    assert!(
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640., 480.)).contains_rect(rect)
    );
}

#[test]
fn fir_designer_compact_actions_are_reachable_busy_disabled_and_safe_when_superseded() {
    let ctx = egui::Context::default();
    let mut tools = Toolkits {
        designer_open: true,
        ..Default::default()
    };
    for _ in 0..5 {
        frame(&ctx, &mut tools, false, vec![]);
    }
    let window = tools.designer_region().unwrap();
    for _ in 0..12 {
        frame(
            &ctx,
            &mut tools,
            false,
            vec![
                egui::Event::PointerMoved(window.center()),
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
        frame(&ctx, &mut tools, false, vec![]);
    }
    let output = frame(&ctx, &mut tools, true, vec![]).0;
    let button = label(&output, "Add to block model").expect("action reachable by scroll");
    let before = tools.model.clone();
    click(&ctx, &mut tools, true, button.center());
    assert!(tools.pending_design_model.is_none());
    assert!(tools.model.same_storage(&before));
    assert!(label(&output, "Design + response").is_some());
    let output = frame(&ctx, &mut tools, false, vec![]).0;
    let pos = label(&output, "Add to block model").unwrap().center();
    let mut first_action = None;
    for pressed in [true, false] {
        let (_, action) = frame(
            &ctx,
            &mut tools,
            false,
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
        if action.is_some() {
            first_action = action;
        }
    }
    assert!(matches!(first_action, Some(Action::FirBlock(_))));
    assert!(
        tools.pending_design_model.is_none(),
        "generating an action must not reserve pending state"
    );
    // An outer dispatcher can accept a later modal action and discard this FIR action.
    // Exercise the real modal-resolution action generation before accepting either.
    tools.model_resolved = true;
    let final_action = frame(&ctx, &mut tools, false, vec![]).1;
    assert!(matches!(final_action, Some(Action::ModelResolved)));
    drop(first_action);
    assert!(tools.pending_design_model.is_none());
    assert!(tools.model.same_storage(&before));
    tools
        .request_fir_block()
        .expect("superseded action cannot leave Designer stuck");
    tools.finish_filter_block(Err("test cleanup".into()));
}

#[test]
fn fir_designer_compact_stop_remains_enabled_and_cancelled_result_keeps_model() {
    let ctx = egui::Context::default();
    let mut tools = Toolkits {
        designer_open: true,
        ..Default::default()
    };
    let original = tools.model.clone();
    tools.request_fir_block().unwrap();
    for _ in 0..5 {
        frame(&ctx, &mut tools, true, vec![]);
    }
    let window = tools.designer_region().unwrap();
    for _ in 0..12 {
        frame(
            &ctx,
            &mut tools,
            true,
            vec![
                egui::Event::PointerMoved(window.center()),
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
        .expect("Stop reachable at 640x480")
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
    assert!(!tools.finish_filter_block(Err("Filter design cancelled".into())));
    assert!(tools.model.same_storage(&original));
    assert!(tools.pending_design_model.is_none());
}

#[test]
fn fir_save_worker_captures_all_windows_and_scaling_without_mutating_workspace_or_model() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("design.m");
    let mut worker = crate::worker::Worker::new(egui::Context::default());
    worker
        .send
        .send(crate::worker::Job::Script("keep=[42 43]; plot(keep);".into()).into())
        .unwrap();
    let initial = worker
        .receive
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    let initial_vars = initial.vars;
    let figure_ids = initial.figure_ids;
    for (window, label, _) in WINDOWS {
        for scale in [false, true] {
            let mut tools = Toolkits {
                fir_window: window,
                fir_scale: scale,
                order: 7,
                band: Band::Highpass,
                ..Default::default()
            };
            let original = tools.model.clone();
            let request = tools.fir_design();
            let expected = request.exported_script().unwrap();
            tools.begin_fir_save(request).unwrap();
            assert!(
                tools
                    .begin_iir_save(crate::iir_designer::Design {
                        family: crate::iir_designer::Family::Butterworth,
                        ripple_db: 1.,
                        order: 6,
                        band: Band::Lowpass,
                        sample_rate: 1000.,
                        cutoff: 50.,
                        upper: 150.
                    })
                    .is_err()
            );
            assert!(tools.begin_fir_block(request).is_err());
            tools.fir_window = matrust::dsp::WindowKind::Rectangular;
            tools.fir_scale = !scale;
            tools.order = 20;
            worker
                .send
                .send(
                    crate::worker::Job::SaveFirDesign(Box::new(crate::fir_designer::Save {
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
            assert!(reply.result.is_ok());
            assert_eq!(reply.vars.len(), initial_vars.len());
            for (name, value) in &initial_vars {
                assert!(value.same_snapshot(&reply.vars[name]));
            }
            assert_eq!(reply.figure_ids, figure_ids);
            tools.finish_design_save(reply.design_save.unwrap());
            assert!(!tools.saving_design && tools.message.starts_with("Saved"));
            assert!(tools.model.same_storage(&original));
            let script = std::fs::read_to_string(&path).unwrap();
            assert_eq!(script, expected);
            assert!(script.contains(label));
            let mut engine = matrust::Engine::default();
            engine.execute(&script).unwrap();
            let Block::Filter { b, .. } = request.block(&AtomicBool::new(false)).unwrap() else {
                panic!("FIR")
            };
            assert_eq!(engine.variables["b"].data(), b.as_slice());
        }
    }
    // A cancelled request returns a save error through the actual channel and
    // leaves both the existing file and the immutable workspace snapshot intact.
    let original_file = std::fs::read(&path).unwrap();
    worker
        .cancel
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let mut tools = Toolkits::default();
    let request = tools.fir_design();
    tools.begin_fir_save(request).unwrap();
    let original = tools.model.clone();
    worker
        .send
        .send(
            crate::worker::Job::SaveFirDesign(Box::new(crate::fir_designer::Save {
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
    assert!(reply.design_save.as_ref().unwrap().is_err());
    for (name, value) in &initial_vars {
        assert!(value.same_snapshot(&reply.vars[name]));
    }
    tools.finish_design_save(reply.design_save.unwrap());
    assert!(!tools.saving_design && tools.model.same_storage(&original));
    assert_eq!(std::fs::read(path).unwrap(), original_file);
    worker.shutdown();
}

#[test]
fn fir_save_stop_is_available_and_unaccepted_requests_do_not_reserve_state() {
    let ctx = egui::Context::default();
    let mut tools = Toolkits {
        designer_open: true,
        ..Default::default()
    };
    let original = tools.model.clone();
    let request = tools.fir_design();
    let action = Action::SaveFir(crate::fir_designer::Save {
        design: request,
        path: std::path::PathBuf::from("unused.m"),
    });
    drop(action);
    assert!(!tools.saving_design);
    tools.begin_fir_save(request).unwrap();
    for _ in 0..5 {
        frame(&ctx, &mut tools, true, vec![]);
    }
    let window = tools.designer_region().unwrap();
    for _ in 0..12 {
        frame(
            &ctx,
            &mut tools,
            true,
            vec![
                egui::Event::PointerMoved(window.center()),
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
        .expect("FIR save Stop reachable at640x480")
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

#[test]
fn fir_designer_cosine_windows_minimum_order_and_promotion_match_public_scripts() {
    use matrust::dsp::WindowKind;
    // Unlike the existing all-zero two-tap Hann/Blackman/Bartlett guard, these
    // three windows have nonzero rounded endpoints, including negative Flat top.
    // Exercise actual two-/three-tap designs, both normalization choices and the
    // exported public script; ordinary order-seven loops do not cover this seam.
    for window in [
        WindowKind::BlackmanHarris,
        WindowKind::Nuttall,
        WindowKind::FlatTop,
    ] {
        for band in [
            Band::Lowpass,
            Band::Highpass,
            Band::Bandpass,
            Band::Bandstop,
        ] {
            for scale in [false, true] {
                let tools = Toolkits {
                    fir_window: window,
                    fir_scale: scale,
                    order: 1,
                    band,
                    ..Default::default()
                };
                let design = tools.fir_design();
                design.validate().unwrap();
                let promoted = matches!(band, Band::Highpass | Band::Bandstop);
                let effective = if promoted { 2 } else { 1 };
                assert_eq!(design.effective_order().unwrap(), effective);
                assert_eq!(
                    design.summary().unwrap().contains("order promoted"),
                    promoted
                );
                let script = design.exported_script().unwrap();
                assert!(script.contains(&format!("{}({})", design.window_info().1, effective + 1)));
                let Block::Filter { b, a } = design.block(&AtomicBool::new(false)).unwrap() else {
                    panic!("FIR block expected");
                };
                assert_eq!(b.len(), effective + 1);
                assert!(b.iter().all(|value| value.is_finite()));
                assert!(b.iter().any(|value| *value != 0.));
                assert_eq!(a, vec![1.]);
                let mut engine = matrust::Engine::default();
                engine.execute(&script).unwrap();
                assert_eq!(engine.variables["b"].data(), b.as_slice());
                if window == WindowKind::FlatTop && !scale && band == Band::Lowpass {
                    assert!(b.iter().all(|value| *value < 0.));
                }
            }
        }
    }
}
