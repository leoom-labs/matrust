//! Full clipped-label and click regressions; no wheel events are needed for actions.
use super::fir_designer_tests::label;
use super::*;

fn frame(
    ctx: &egui::Context,
    tools: &mut Toolkits,
    size: egui::Vec2,
    busy: bool,
    events: Vec<egui::Event>,
) -> (egui::FullOutput, Option<Action>) {
    let mut action = None;
    let output = crate::headless::run(
        ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |root| {
            action = tools.ui(root.ctx(), busy);
        },
    );
    (output, action)
}
fn settled(
    ctx: &egui::Context,
    tools: &mut Toolkits,
    size: egui::Vec2,
    busy: bool,
) -> egui::FullOutput {
    for _ in 0..5 {
        frame(ctx, tools, size, busy, vec![]);
    }
    frame(ctx, tools, size, busy, vec![]).0
}
fn click(
    ctx: &egui::Context,
    tools: &mut Toolkits,
    size: egui::Vec2,
    busy: bool,
    pos: egui::Pos2,
) -> Option<Action> {
    let mut result = None;
    for pressed in [true, false] {
        let action = frame(
            ctx,
            tools,
            size,
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
        .1;
        if action.is_some() {
            result = action;
        }
    }
    result
}
const ACTIONS: [&str; 4] = [
    "Design + response",
    "Apply to y",
    "Save design script…",
    "Add to block model",
];
fn visible(output: &egui::FullOutput, window: egui::Rect) -> Vec<egui::Rect> {
    ACTIONS
        .iter()
        .map(|text| {
            let rect = label(output, text)
                .unwrap_or_else(|| panic!("{text} fully visible without scrolling"));
            assert!(window.contains_rect(rect), "{text} inside window");
            rect
        })
        .collect()
}

#[test]
fn designer_primary_actions_and_stop_are_visible_without_scroll_at_all_supported_sizes() {
    for size in [
        egui::vec2(640., 480.),
        egui::vec2(960., 650.),
        egui::vec2(1440., 900.),
    ] {
        for iir in [false, true] {
            let ctx = egui::Context::default();
            let mut tools = Toolkits {
                designer_open: true,
                iir,
                order: 6,
                ..Default::default()
            };
            let output = settled(&ctx, &mut tools, size, false);
            let window = tools.designer_region().unwrap();
            assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(window));
            let idle = visible(&output, window);
            if iir {
                tools.begin_iir_save(tools.iir_design()).unwrap();
            } else {
                tools.begin_fir_save(tools.fir_design()).unwrap();
            }
            let output = settled(&ctx, &mut tools, size, true);
            let busy = visible(&output, window);
            assert_eq!(idle, busy, "pending save must not move primary actions");
            let stop =
                label(&output, "Stop filter design").expect("Stop visible without scrolling");
            assert!(window.contains_rect(stop));
            assert!(matches!(
                click(&ctx, &mut tools, size, true, stop.center()),
                Some(Action::StopModel)
            ));
            tools.finish_design_save(Err("test cleanup".into()));
        }
    }
}

#[test]
fn designer_field_scrolling_keeps_footer_fixed_and_selected_design_intact() {
    let size = egui::vec2(640., 480.);
    let ctx = egui::Context::default();
    let mut tools = Toolkits {
        designer_open: true,
        fir_window: matrust::dsp::WindowKind::Blackman,
        fir_scale: false,
        band: Band::Bandstop,
        order: 7,
        analysis: Analysis::GroupDelay,
        ..Default::default()
    };
    let expected = tools.design_script().unwrap();
    let output = settled(&ctx, &mut tools, size, false);
    let window = tools.designer_region().unwrap();
    let before = visible(&output, window);
    // Scroll only fields, then footer: neither should change actions or settings.
    for pointer in [
        egui::pos2(window.center().x, window.top() + 100.),
        before[0].center(),
    ] {
        for delta in [-2000., 2000., -2000.] {
            frame(
                &ctx,
                &mut tools,
                size,
                false,
                vec![
                    egui::Event::PointerMoved(pointer),
                    egui::Event::MouseWheel {
                        phase: egui::TouchPhase::Move,
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0., delta),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            let output = settled(&ctx, &mut tools, size, false);
            assert_eq!(visible(&output, window), before);
            assert_eq!(tools.design_script().unwrap(), expected);
        }
    }
    let action = click(&ctx, &mut tools, size, false, before[0].center());
    let Some(Action::Script(script)) = action else {
        panic!("Design emits preview")
    };
    assert!(script.starts_with(&expected));
    assert!(script.contains("blackman(9)") && script.contains("noscale"));
}

#[test]
fn designer_footer_busy_and_invalid_actions_remain_visible_but_do_not_publish() {
    let size = egui::vec2(640., 480.);
    let ctx = egui::Context::default();
    let mut tools = Toolkits {
        designer_open: true,
        ..Default::default()
    };
    let original = tools.model.clone();
    for busy in [true, false] {
        if !busy {
            tools.cutoff = tools.sample_rate;
        }
        let output = settled(&ctx, &mut tools, size, busy);
        let window = tools.designer_region().unwrap();
        for button in visible(&output, window) {
            // Save is disabled too: this must never open the native file dialog.
            assert!(click(&ctx, &mut tools, size, busy, button.center()).is_none());
        }
        assert!(tools.pending_design_model.is_none() && !tools.saving_design);
        assert!(tools.model.same_storage(&original));
    }
}

#[test]
fn designer_footer_captures_selected_popup_value_and_retains_outer_action_precedence() {
    let size = egui::vec2(640., 480.);
    let ctx = egui::Context::default();
    let mut tools = Toolkits {
        designer_open: true,
        ..Default::default()
    };
    let output = settled(&ctx, &mut tools, size, false);
    click(
        &ctx,
        &mut tools,
        size,
        false,
        label(&output, "Hamming").unwrap().center(),
    );
    let output = frame(&ctx, &mut tools, size, false, vec![]).0;
    // Last painted matching label belongs to the popup, not the selected display.
    let choice = output
        .shapes
        .iter()
        .rev()
        .find_map(|shape| {
            label(
                &egui::FullOutput {
                    shapes: vec![shape.clone()],
                    ..Default::default()
                },
                "Hann",
            )
        })
        .expect("popup Hann option visible");
    click(&ctx, &mut tools, size, false, choice.center());
    assert_eq!(tools.fir_window, matrust::dsp::WindowKind::Hann);
    let output = settled(&ctx, &mut tools, size, false);
    let original = tools.model.clone();
    let request = click(
        &ctx,
        &mut tools,
        size,
        false,
        label(&output, "Add to block model").unwrap().center(),
    );
    let Some(Action::FirBlock(request)) = request else {
        panic!("FIR action")
    };
    assert_eq!(request.window, matrust::dsp::WindowKind::Hann);
    assert!(tools.pending_design_model.is_none());
    tools.model_resolved = true;
    assert!(matches!(
        frame(&ctx, &mut tools, size, false, vec![]).1,
        Some(Action::ModelResolved)
    ));
    assert!(tools.model.same_storage(&original));
    assert!(tools.pending_design_model.is_none());
    tools.begin_fir_block(request).unwrap();
    let output = settled(&ctx, &mut tools, size, true);
    let stop = label(&output, "Stop filter design").expect("block Stop without scroll");
    assert!(matches!(
        click(&ctx, &mut tools, size, true, stop.center()),
        Some(Action::StopModel)
    ));
    tools.finish_filter_block(Err("test cleanup".into()));
}

#[test]
fn butterworth_result_summary_stays_visible_with_one_section_help_and_field_scrolling() {
    const ONE_SECTION_HELP: &str = "One SOS row: use separate numerator/denominator vectors for zero-phase filtfilt in scripts.";
    for size in [
        egui::vec2(640., 480.),
        egui::vec2(960., 650.),
        egui::vec2(1440., 900.),
    ] {
        for band in [Band::Bandpass, Band::Bandstop] {
            for order in [1, 20] {
                let ctx = egui::Context::default();
                let mut tools = Toolkits {
                    designer_open: true,
                    iir: true,
                    band,
                    order,
                    sample_rate: 1000.,
                    cutoff: 100.,
                    upper: 200.,
                    ..Default::default()
                };
                let summary = tools.iir_design().summary().unwrap();
                let expected = tools.design_script().unwrap();
                let output = settled(&ctx, &mut tools, size, false);
                let window = tools.designer_region().unwrap();
                let initial_summary =
                    label(&output, &summary).expect("Full result summary visible before scrolling");
                assert!(window.contains_rect(initial_summary));
                let actions = visible(&output, window);
                assert!(initial_summary.bottom() <= actions[0].top());
                let mut one_section_help_seen = label(&output, ONE_SECTION_HELP).is_some();
                // Visit intermediate field positions as well as both ends: the
                // help may be between the top controls and the final script.
                for delta in std::iter::once(2000.)
                    .chain(std::iter::repeat_n(-48., 16))
                    .chain(std::iter::once(2000.))
                {
                    frame(
                        &ctx,
                        &mut tools,
                        size,
                        false,
                        vec![
                            egui::Event::PointerMoved(egui::pos2(
                                window.center().x,
                                window.top() + 100.,
                            )),
                            egui::Event::MouseWheel {
                                phase: egui::TouchPhase::Move,
                                unit: egui::MouseWheelUnit::Point,
                                delta: egui::vec2(0., delta),
                                modifiers: egui::Modifiers::NONE,
                            },
                        ],
                    );
                    let output = settled(&ctx, &mut tools, size, false);
                    assert_eq!(label(&output, &summary), Some(initial_summary));
                    assert_eq!(visible(&output, window), actions);
                    assert_eq!(tools.design_script().unwrap(), expected);
                    one_section_help_seen |= label(&output, ONE_SECTION_HELP).is_some();
                }
                if order == 1 {
                    assert!(
                        one_section_help_seen,
                        "One-section limitation remains accessible"
                    );
                }
                let Some(Action::Script(script)) =
                    click(&ctx, &mut tools, size, false, actions[0].center())
                else {
                    panic!("Visible primary action submits selected design");
                };
                assert!(script.starts_with(&expected));
                tools.begin_iir_save(tools.iir_design()).unwrap();
                let busy = settled(&ctx, &mut tools, size, true);
                assert_eq!(label(&busy, &summary), Some(initial_summary));
                assert_eq!(visible(&busy, window), actions);
                assert!(label(&busy, "Stop filter design").is_some());
                tools.finish_design_save(Err("test cleanup".into()));
            }
        }
    }
}
