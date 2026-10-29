use super::*;
use matrust::simulation::{Block, Node};
use parameters::{Edit, Properties, Settings};
use std::sync::atomic::AtomicBool;

fn undo(canvas: &mut Canvas, model: &mut Model, redo: bool) {
    canvas.travel(model, &mut 0, redo, &mut String::new());
}
fn record(canvas: &mut Canvas, model: &Model, index: usize, before: Properties, id: egui::Id) {
    if let Some(edit) = Edit::node(model, index, before) {
        canvas.record_edit(edit, Some(id));
    }
}
fn pointer(model: &Model, index: usize) -> *const f64 {
    match &model.nodes[index].block {
        Block::Filter { b, .. } => b.as_ptr(),
        _ => panic!(),
    }
}

#[test]
fn model_parameters_scalar_variants_names_and_settings_preserve_exact_bits() {
    for (before, after) in [
        (
            Block::Constant { value: -0. },
            Block::Constant { value: 9. },
        ),
        (Block::Gain { gain: 2. }, Block::Gain { gain: -3. }),
        (Block::Bias { bias: -0. }, Block::Bias { bias: 2.5 }),
        (
            Block::Saturation {
                lower: -0.,
                upper: 1.,
            },
            Block::Saturation {
                lower: -2.,
                upper: 3.,
            },
        ),
        (
            Block::UnitDelay { initial: -0. },
            Block::UnitDelay { initial: 1. },
        ),
        (
            Block::Sine {
                amplitude: 1.,
                frequency_hz: 5.,
                phase: -0.,
            },
            Block::Sine {
                amplitude: 2.,
                frequency_hz: 7.,
                phase: 0.5,
            },
        ),
        (
            Block::Step {
                time: 1.,
                before: -0.,
                after: 2.,
            },
            Block::Step {
                time: 3.,
                before: 4.,
                after: 5.,
            },
        ),
    ] {
        let mut model = Model {
            nodes: vec![Node {
                name: "Before".into(),
                block: before,
                inputs: vec![],
            }],
            ..Model::default()
        };
        let original = model.clone();
        let mut canvas = Canvas::default();
        let snapshot = Properties::capture(&model.nodes[0]);
        model.nodes[0].block = after;
        model.nodes[0].name = "After".into();
        record(&mut canvas, &model, 0, snapshot, egui::Id::new("property"));
        let edited = model.clone();
        undo(&mut canvas, &mut model, false);
        assert!(model.same_storage(&original));
        undo(&mut canvas, &mut model, true);
        assert!(model.same_storage(&edited));
        let before = Settings::capture(&model);
        model.sample_time = -0.;
        model.steps = 0;
        canvas.record_edit(Edit::settings(&model, before).unwrap(), None);
        undo(&mut canvas, &mut model, false);
        assert!(model.same_storage(&edited));
        undo(&mut canvas, &mut model, true);
        assert_eq!(model.sample_time.to_bits(), (-0_f64).to_bits());
        assert_eq!(model.steps, 0);
    }
}

#[test]
fn model_parameters_coalesce_by_interaction_and_preserve_redo_boundaries() {
    let mut model = Model::default();
    let mut canvas = Canvas::default();
    let id = egui::Id::new("name");
    for name in ["A", "AB", "ABC"] {
        let before = Properties::capture(&model.nodes[0]);
        model.nodes[0].name = name.into();
        record(&mut canvas, &model, 0, before, id);
    }
    assert_eq!(canvas.history.len(), 1);
    canvas.finish_edit(None);
    for name in ["ABCD", "ABC"] {
        let before = Properties::capture(&model.nodes[0]);
        model.nodes[0].name = name.into();
        record(&mut canvas, &model, 0, before, id);
    }
    assert_eq!(canvas.history.len(), 1); // Second interaction returned to its start.
    let before = Properties::capture(&model.nodes[0]);
    model.nodes[0].name = "XYZ".into();
    record(&mut canvas, &model, 0, before, id);
    assert_eq!(canvas.history.len(), 2); // Must not merge into the older interaction.
    undo(&mut canvas, &mut model, false);
    assert_eq!(model.nodes[0].name, "ABC");
    let before = Properties::capture(&model.nodes[0]);
    model.nodes[0].name = "New branch".into();
    record(&mut canvas, &model, 0, before, id);
    assert_eq!(canvas.cursor, canvas.history.len());
    undo(&mut canvas, &mut model, false);
    undo(&mut canvas, &mut model, false);
    assert_eq!(model.nodes[0].name, "Signal");
}

#[test]
fn model_parameters_filter_and_sos_undo_swap_buffers_and_restore_simulation() {
    for sos in [false, true] {
        let mut model = Model::default();
        if sos {
            model.nodes[3].block = Block::SosFilter {
                sections: vec![[1., 0., 0., 1., 0., 0.]],
            };
        }
        let original = model.clone();
        let before = model.run(&AtomicBool::new(false)).unwrap();
        let original_pointer = if sos { None } else { Some(pointer(&model, 3)) };
        let mut canvas = Canvas::default();
        let block = if sos {
            Block::SosFilter {
                sections: vec![[0.25, 0., 0., 1., -0.5, 0.]],
            }
        } else {
            Block::Filter {
                b: vec![0.5, 0.5],
                a: vec![1.],
            }
        };
        canvas.replace_coefficients(&mut model, 3, block).unwrap();
        let changed = model.clone();
        let changed_pointer = if sos { None } else { Some(pointer(&model, 3)) };
        for _ in 0..3 {
            undo(&mut canvas, &mut model, false);
            assert!(model.same_storage(&original));
            if let Some(p) = original_pointer {
                assert_eq!(pointer(&model, 3), p);
            }
            let restored = model.run(&AtomicBool::new(false)).unwrap();
            assert_eq!(
                bytemuck::cast_slice::<f64, u64>(before.traces[0].values.data()),
                bytemuck::cast_slice::<f64, u64>(restored.traces[0].values.data())
            );
            undo(&mut canvas, &mut model, true);
            assert!(model.same_storage(&changed));
            if let Some(p) = changed_pointer {
                assert_eq!(pointer(&model, 3), p);
            }
        }
        let count = canvas.history.len();
        let duplicate = model.nodes[3].block.clone();
        canvas
            .replace_coefficients(&mut model, 3, duplicate)
            .unwrap();
        assert_eq!(canvas.history.len(), count);
    }
    let mut model = Model::default();
    let original = model.clone();
    let mut canvas = Canvas::default();
    assert!(
        canvas
            .replace_coefficients(
                &mut model,
                3,
                Block::Filter {
                    b: vec![1.],
                    a: vec![0.]
                }
            )
            .is_err()
    );
    assert!(model.same_storage(&original));
    assert!(canvas.history.is_empty());
}

#[test]
fn model_parameters_mixed_node_edits_restore_dirty_baseline_without_coefficient_copies() {
    let mut tools = super::super::Toolkits::default();
    let original = tools.model.clone();
    tools.add_block(Block::Filter {
        b: vec![1., 0.],
        a: vec![1.],
    });
    let index = tools.selected;
    let original_pointer = pointer(&tools.model, index);
    let before = Properties::capture(&tools.model.nodes[index]);
    tools.model.nodes[index].name = "Renamed filter".into();
    record(
        &mut tools.canvas,
        &tools.model,
        index,
        before,
        egui::Id::new("rename"),
    );
    tools
        .canvas
        .replace_coefficients(
            &mut tools.model,
            index,
            Block::Filter {
                b: vec![0.25; 129],
                a: vec![1.],
            },
        )
        .unwrap();
    tools.delete_block();
    assert!(!tools.model_dirty());
    for _ in 0..2 {
        undo(&mut tools.canvas, &mut tools.model, false);
    }
    assert!(tools.model_dirty());
    assert_eq!(pointer(&tools.model, index), original_pointer);
    undo(&mut tools.canvas, &mut tools.model, false);
    undo(&mut tools.canvas, &mut tools.model, false);
    assert!(tools.model.same_storage(&original));
    assert!(!tools.model_dirty());
    for _ in 0..4 {
        undo(&mut tools.canvas, &mut tools.model, true);
    }
    assert_eq!(tools.model.nodes.len(), 5);
}

#[test]
fn model_parameters_stale_history_keeps_newer_properties_and_coefficients() {
    for coefficient in [false, true] {
        let mut model = Model::default();
        let mut canvas = Canvas::default();
        if coefficient {
            canvas
                .replace_coefficients(
                    &mut model,
                    3,
                    Block::Filter {
                        b: vec![0.5],
                        a: vec![1.],
                    },
                )
                .unwrap();
        } else {
            let before = Properties::capture(&model.nodes[3]);
            model.nodes[3].name = "Edited".into();
            record(&mut canvas, &model, 3, before, egui::Id::new("name"));
        }
        model.nodes[3].name = "Newer untracked change".into();
        let newer = model.clone();
        undo(&mut canvas, &mut model, false);
        assert!(model.same_storage(&newer));
        assert!(canvas.history.is_empty());
        assert_eq!(canvas.history_bytes, 0);
    }
}

#[test]
fn model_parameters_coefficient_history_stays_within_memory_budget() {
    let mut model = Model::default();
    let mut canvas = Canvas::default();
    for i in 0..90 {
        canvas
            .replace_coefficients(
                &mut model,
                3,
                Block::Filter {
                    b: vec![i as f64; 4097],
                    a: vec![1.; 4097],
                },
            )
            .unwrap();
        assert!(canvas.history_bytes <= HISTORY_BYTES);
    }
    assert!(canvas.history.len() < HISTORY_LIMIT);
    assert_eq!(
        canvas.history_bytes,
        canvas
            .history
            .iter()
            .map(Change::retained_bytes)
            .sum::<usize>()
    );
    let address = pointer(&model, 3);
    let before = Properties::capture(&model.nodes[3]);
    model.nodes[3].name = "Longer name".into();
    record(&mut canvas, &model, 3, before, egui::Id::new("name"));
    undo(&mut canvas, &mut model, false);
    assert_eq!(pointer(&model, 3), address);
}

#[test]
fn model_parameters_noop_rename_keeps_add_undo_and_memory_accounting_valid() {
    let mut tools = super::super::Toolkits::default();
    let original = tools.model.clone();
    tools.add_block(Block::Gain { gain: 2. });
    let index = tools.selected;
    let name = tools.model.nodes[index].name.clone();
    let id = egui::Id::new("name growth");
    let before = Properties::capture(&tools.model.nodes[index]);
    tools.model.nodes[index].name.push_str(&"x".repeat(100));
    record(&mut tools.canvas, &tools.model, index, before, id);
    let before = Properties::capture(&tools.model.nodes[index]);
    tools.model.nodes[index].name.truncate(name.len());
    assert!(tools.model.nodes[index].name.capacity() > name.len());
    record(&mut tools.canvas, &tools.model, index, before, id);
    assert_eq!(tools.canvas.history.len(), 1);
    undo(&mut tools.canvas, &mut tools.model, false);
    assert!(tools.model.same_storage(&original));
    assert!(!tools.model_dirty());
    assert!(tools.canvas.history_bytes <= HISTORY_BYTES);
    assert_eq!(
        tools.canvas.history_bytes,
        tools
            .canvas
            .history
            .iter()
            .map(Change::retained_bytes)
            .sum::<usize>()
    );
}

#[test]
fn model_parameters_native_typing_and_dragging_group_once_across_layout_passes() {
    let mut results = Vec::new();
    for repeat in [false, true] {
        let ctx = egui::Context::default();
        let mut tools = super::super::Toolkits {
            model_open: true,
            selected: 0,
            ..Default::default()
        };
        let frame = |tools: &mut super::super::Toolkits, events: Vec<egui::Event>| {
            crate::headless::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(960., 650.))),
                    events,
                    ..Default::default()
                },
                |root| {
                    let ctx = &root.ctx().clone();
                    tools.ui(ctx, false);
                    if repeat && ctx.current_pass_index() == 0 {
                        ctx.request_discard("parameter edit once per input");
                    }
                },
            )
        };
        fn point(output: &egui::FullOutput, label: &str) -> Pos2 {
            fn find(shape: &egui::Shape, label: &str) -> Option<Pos2> {
                match shape {
                    egui::Shape::Text(t) if t.galley.text() == label => {
                        Some(t.pos + t.galley.size() * 0.5)
                    }
                    egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, label)),
                    _ => None,
                }
            }
            output
                .shapes
                .iter()
                .find_map(|s| find(&s.shape, label))
                .unwrap_or_else(|| panic!("Missing {label}"))
        }
        let pointer = |p: Pos2, pressed| {
            vec![
                egui::Event::PointerMoved(p),
                egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        for _ in 0..3 {
            frame(&mut tools, vec![]);
        }
        let p = point(&frame(&mut tools, vec![]), "Signal");
        frame(&mut tools, pointer(p, true));
        frame(&mut tools, pointer(p, false));
        frame(
            &mut tools,
            vec![egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::CTRL | egui::Modifiers::COMMAND,
            }],
        );
        frame(&mut tools, vec![egui::Event::Text("Renamed".into())]);
        frame(&mut tools, vec![egui::Event::Text(" source".into())]);
        assert_eq!(tools.model.nodes[0].name, "Renamed source");
        assert_eq!(tools.canvas.history.len(), 1);
        let p = point(&frame(&mut tools, vec![]), "Undo diagram");
        frame(&mut tools, pointer(p, true));
        frame(&mut tools, pointer(p, false));
        assert_eq!(tools.model.nodes[0].name, "Signal");
        let p = point(&frame(&mut tools, vec![]), "1.00");
        frame(&mut tools, pointer(p, true));
        frame(
            &mut tools,
            vec![egui::Event::PointerMoved(p + Vec2::new(20., 0.))],
        );
        frame(
            &mut tools,
            vec![egui::Event::PointerMoved(p + Vec2::new(50., 0.))],
        );
        frame(&mut tools, pointer(p + Vec2::new(50., 0.), false));
        assert_eq!(tools.canvas.history.len(), 1);
        let Block::Sine { amplitude, .. } = tools.model.nodes[0].block else {
            panic!()
        };
        assert!(amplitude > 1.);
        results.push(amplitude.to_bits());
        undo(&mut tools.canvas, &mut tools.model, false);
        assert!(matches!(
            tools.model.nodes[0].block,
            Block::Sine { amplitude: 1., .. }
        ));
    }
    assert_eq!(results[0], results[1]);
}
