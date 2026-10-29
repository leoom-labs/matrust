use super::*;
use crate::iir_designer::Family;
use std::sync::atomic::AtomicBool;

fn cheby(band: Band, order: usize) -> Toolkits {
    Toolkits {
        designer_open: true,
        iir: true,
        iir_family: Family::Chebyshev1,
        ripple_db: 1.25,
        band,
        order,
        cutoff: 100.,
        upper: 200.,
        analysis_points: 32,
        ..Default::default()
    }
}
fn bits(a: &[f64]) -> Vec<u64> {
    a.iter().map(|v| v.to_bits()).collect()
}

#[test]
fn chebyshev_designer_four_forms_share_exact_worker_script_block_and_portable_sos() {
    let mut worker = crate::worker::Worker::new(egui::Context::default());
    worker
        .send
        .send(crate::worker::Job::Script("x=sin((0:31)/7); alias=x;".into()).into())
        .unwrap();
    let initial = worker
        .receive
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    let source = initial.vars["x"].clone();
    for band in [
        Band::Lowpass,
        Band::Highpass,
        Band::Bandpass,
        Band::Bandstop,
    ] {
        for order in [1, 5, 20] {
            let tools = cheby(band, order);
            let request = tools.iir_design();
            let (cuts, count) = request.cutoffs();
            let expected =
                matrust::dsp::cheby1(order, 1.25, &cuts[..count], band, &AtomicBool::new(false))
                    .unwrap()
                    .sos(&AtomicBool::new(false))
                    .unwrap();
            worker
                .send
                .send(crate::worker::Job::IirBlock(request).into())
                .unwrap();
            let reply = worker
                .receive
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            let Block::SosFilter { sections } = reply.fir_block.unwrap().unwrap() else {
                panic!("SOS")
            };
            assert_eq!(sections.len(), expected.rows());
            for (r, row) in sections.iter().enumerate() {
                for (c, value) in row.iter().enumerate() {
                    assert_eq!(value.to_bits(), expected.get(r, c).to_bits());
                }
            }
            worker
                .send
                .send(
                    crate::worker::Job::Script(format!(
                        "{}\ny=sosfilt(sos,x);",
                        tools.preview_script().unwrap()
                    ))
                    .into(),
                )
                .unwrap();
            let reply = worker
                .receive
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            assert_eq!(
                bits(reply.vars["sos"].as_numeric().unwrap().data()),
                bits(expected.data())
            );
            assert_eq!(reply.vars["response"].len(), 32);
            assert!(reply.vars["x"].same_snapshot(&source));
            assert!(reply.vars["alias"].same_snapshot(&source));
            let filtered = matrust::dsp::sosfilt(
                &expected,
                source.as_numeric().unwrap(),
                &AtomicBool::new(false),
            )
            .unwrap();
            assert_eq!(
                bits(reply.vars["y"].as_numeric().unwrap().data()),
                bits(filtered.data())
            );
            let portable = request.portable_script(&AtomicBool::new(false)).unwrap();
            assert!(!portable.contains("cheby1_sos("));
            assert!(portable.contains("ripple 1.25 dB"));
            let mut replay = matrust::Engine::default();
            replay.execute(&portable).unwrap();
            assert_eq!(bits(replay.variables["sos"].data()), bits(expected.data()));
        }
    }
    worker.shutdown();
}

#[test]
fn chebyshev_designer_validates_ripple_before_admission_and_preserves_cancelled_model() {
    let mut tools = cheby(Band::Bandpass, 5);
    let original = tools.model.clone();
    for ripple in [0., -1., f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        tools.ripple_db = ripple;
        let request = tools.iir_design();
        assert!(request.script().unwrap_err().contains("ripple"));
        assert!(tools.begin_iir_block(request).is_err());
        assert!(tools.begin_iir_save(request).is_err());
        assert!(tools.pending_design_model.is_none() && !tools.saving_design);
        assert!(tools.model.same_storage(&original));
    }
    tools.ripple_db = 1.;
    let request = tools.iir_design();
    tools.begin_iir_block(request).unwrap();
    assert!(!tools.finish_filter_block(request.block(&AtomicBool::new(true))));
    assert!(tools.model.same_storage(&original));
    tools.begin_iir_block(request).unwrap();
    assert!(tools.finish_filter_block(request.block(&AtomicBool::new(false))));
    assert_eq!(tools.model.nodes.len(), original.nodes.len() + 1);
    // Ripple is irrelevant to the unchanged Butterworth family and FIR path.
    tools.iir_family = Family::Butterworth;
    tools.ripple_db = f64::NAN;
    assert!(tools.iir_design().validate().is_ok());
    tools.iir = false;
    assert!(tools.design_script().unwrap().starts_with("b = fir1("));
    tools.open_designer_demo(true);
    assert_eq!(tools.iir_family, Family::Butterworth);
    assert_eq!(
        tools.design_script().unwrap(),
        "sos = butter_sos(6, 0.2, 'low');"
    );
}

#[test]
fn chebyshev_designer_worker_captures_family_ripple_and_rejects_late_model_adoption() {
    let mut tools = cheby(Band::Bandstop, 3);
    let request = tools.iir_design();
    tools.begin_iir_block(request).unwrap();
    tools.iir_family = Family::Butterworth;
    tools.ripple_db = 9.;
    tools.order = 20;
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
        panic!("SOS")
    };
    let expected = matrust::dsp::cheby1(
        3,
        1.25,
        &[0.2, 0.4],
        Band::Bandstop,
        &AtomicBool::new(false),
    )
    .unwrap()
    .sos(&AtomicBool::new(false))
    .unwrap();
    assert_eq!(sections[0][0].to_bits(), expected.get(0, 0).to_bits());
    tools.begin_iir_block(request).unwrap();
    tools.model.steps += 1;
    let changed = tools.model.clone();
    worker
        .send
        .send(crate::worker::Job::IirBlock(request).into())
        .unwrap();
    let reply = worker
        .receive
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    assert!(!tools.finish_filter_block(reply.fir_block.unwrap()));
    assert!(tools.model.same_storage(&changed));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chebyshev.m");
    tools.begin_iir_save(request).unwrap();
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
    tools.finish_design_save(reply.design_save.unwrap());
    assert!(!tools.saving_design);
    assert!(tools.model.same_storage(&changed));
    let text = std::fs::read_to_string(&path).unwrap();
    let mut replay = matrust::Engine::default();
    replay.execute(&text).unwrap();
    assert_eq!(bits(replay.variables["sos"].data()), bits(expected.data()));
    assert!(
        crate::iir_designer::save(
            crate::iir_designer::Save {
                design: request,
                path: path.clone()
            },
            &AtomicBool::new(true)
        )
        .is_err()
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), text);
    worker.shutdown();
}

fn frame(
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
                egui::vec2(960., 650.),
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
fn settled(ctx: &egui::Context, tools: &mut Toolkits, busy: bool) -> egui::FullOutput {
    for _ in 0..6 {
        frame(ctx, tools, busy, vec![]);
    }
    frame(ctx, tools, busy, vec![]).0
}
fn visible_ink(output: &egui::FullOutput, text: &str) -> Option<egui::Rect> {
    fn find(shape: &egui::Shape, text: &str) -> Option<egui::Rect> {
        match shape {
            egui::Shape::Text(t) if t.galley.text() == text => Some(t.visual_bounding_rect()),
            egui::Shape::Vec(v) => v.iter().rev().find_map(|s| find(s, text)),
            _ => None,
        }
    }
    output
        .shapes
        .iter()
        .rev()
        .find_map(|s| find(&s.shape, text).filter(|r| s.clip_rect.contains_rect(*r)))
}
fn ink(output: &egui::FullOutput, text: &str) -> egui::Rect {
    visible_ink(output, text).unwrap_or_else(|| panic!("full painted ink must be visible: {text}"))
}
fn click(ctx: &egui::Context, tools: &mut Toolkits, busy: bool, pos: egui::Pos2) -> Option<Action> {
    let mut action = None;
    for pressed in [true, false] {
        if let Some(value) = frame(
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
        )
        .1
        {
            assert!(action.is_none());
            action = Some(value);
        }
    }
    action
}

#[test]
fn chebyshev_designer_compact_both_themes_family_ripple_four_forms_and_actions_are_reachable() {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let ctx = egui::Context::default();
        crate::theme::configure(&ctx);
        ctx.set_theme(theme);
        let mut tools = Toolkits {
            designer_open: true,
            iir: true,
            order: 3,
            analysis_points: 32,
            ..Default::default()
        };
        let initial = tools.model.clone();
        let output = settled(&ctx, &mut tools, false);
        let pos = ink(&output, "Butterworth").center();
        click(&ctx, &mut tools, false, pos);
        let output = settled(&ctx, &mut tools, false);
        let pos = ink(&output, "Chebyshev I").center();
        click(&ctx, &mut tools, false, pos);
        assert_eq!(tools.iir_family, Family::Chebyshev1);
        let output = settled(&ctx, &mut tools, false);
        ink(&output, "Passband ripple (dB)");
        let ripple = ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::new("designer_ripple_response")))
            .unwrap();
        click(&ctx, &mut tools, false, ripple.center());
        frame(
            &ctx,
            &mut tools,
            false,
            vec![
                egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                },
                egui::Event::Text("2.5".into()),
                egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        assert_eq!(tools.ripple_db, 2.5);
        for band in [
            Band::Lowpass,
            Band::Highpass,
            Band::Bandpass,
            Band::Bandstop,
        ] {
            let output = settled(&ctx, &mut tools, false);
            let pos = ink(&output, tools.band.name()).center();
            click(&ctx, &mut tools, false, pos);
            let output = settled(&ctx, &mut tools, false);
            let pos = ink(&output, band.name()).center();
            click(&ctx, &mut tools, false, pos);
            assert_eq!(tools.band, band);
            let output = settled(&ctx, &mut tools, false);
            let window = tools.designer_region().unwrap();
            assert!(
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(960., 650.))
                    .contains_rect(window)
            );
            for text in [
                "Design + response",
                "Apply to y",
                "Save design script\u{2026}",
                "Add to block model",
            ] {
                assert!(window.contains_rect(ink(&output, text)));
            }
            let pos = ink(&output, "Apply to y").center();
            let Some(Action::Apply(crate::designer_apply::Design::Iir(request), input)) =
                click(&ctx, &mut tools, false, pos)
            else {
                panic!("Apply")
            };
            assert_eq!(input, "x");
            assert_eq!(request.family, Family::Chebyshev1);
            let script = request.script().unwrap();
            assert!(script.contains(&format!(
                "sos = cheby1_sos(3, 2.5e0, {}",
                if matches!(band, Band::Bandpass | Band::Bandstop) {
                    "[0.1 0.3]"
                } else {
                    "0.1"
                }
            )));
            assert_eq!(request.ripple_db, 2.5);
            let pos = ink(&output, "Design + response").center();
            assert!(matches!(
                click(&ctx, &mut tools, false, pos),
                Some(Action::Script(_))
            ));
            let pos = ink(&output, "Add to block model").center();
            let Some(Action::IirBlock(request)) = click(&ctx, &mut tools, false, pos) else {
                panic!("Add")
            };
            assert_eq!(request.family, Family::Chebyshev1);
            assert_eq!(request.ripple_db, 2.5);
        }
        let output = settled(&ctx, &mut tools, false);
        let window = tools.designer_region().unwrap();
        let footer = ink(&output, "Apply to y");
        let fields = [
            "Upper edge (Hz)",
            "Workspace input variable",
            "Analysis",
            "Analysis points / samples",
        ];
        let mut seen = [false; 4];
        for _ in 0..20 {
            let output = settled(&ctx, &mut tools, false);
            for (i, text) in fields.iter().enumerate() {
                seen[i] |= visible_ink(&output, text).is_some();
            }
            assert_eq!(
                ink(&output, "Apply to y"),
                footer,
                "scroll settings without moving actions"
            );
            frame(
                &ctx,
                &mut tools,
                false,
                vec![
                    egui::Event::PointerMoved(egui::pos2(window.center().x, window.top() + 100.)),
                    egui::Event::MouseWheel {
                        phase: egui::TouchPhase::Move,
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0., -32.),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert!(
            seen.into_iter().all(|v| v),
            "all settings reachable by actual bounded scrolling"
        );
        tools.begin_iir_save(tools.iir_design()).unwrap();
        let output = settled(&ctx, &mut tools, true);
        let pos = ink(&output, "Stop filter design").center();
        assert!(matches!(
            click(&ctx, &mut tools, true, pos),
            Some(Action::StopModel)
        ));
        assert!(tools.model.same_storage(&initial));
        tools.finish_design_save(Err("cancelled".into()));
    }
}

#[test]
fn chebyshev_designer_invalid_ripple_disables_real_actions_in_both_themes() {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let ctx = egui::Context::default();
        crate::theme::configure(&ctx);
        ctx.set_theme(theme);
        let mut tools = cheby(Band::Lowpass, 3);
        tools.ripple_db = 0.;
        let original = tools.model.clone();
        let output = settled(&ctx, &mut tools, false);
        ink(&output, "Invalid filter settings");
        for text in ["Design + response", "Apply to y", "Add to block model"] {
            let pos = ink(&output, text).center();
            assert!(click(&ctx, &mut tools, false, pos).is_none());
        }
        assert!(tools.pending_design_model.is_none() && !tools.saving_design);
        assert!(tools.model.same_storage(&original));
    }
}
