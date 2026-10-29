use super::*;
use egui::{Event, Modifiers, Pos2, Rect, Vec2};
use matrust::simulation::{Block, Node};

#[test]
fn model_list_distinguishes_empty_document_from_unmatched_filter() {
    let ctx = egui::Context::default();
    let mut list = List::default();
    let mut canvas = Canvas::default();
    let mut selected = 0;
    let mut model = Model {
        version: 1,
        sample_time: 0.001,
        steps: 1000,
        nodes: Vec::new(),
        positions: Vec::new(),
    };
    list.query = "missing name".into();
    for expected in ["No blocks yet", "No matching blocks", "No blocks yet"] {
        let output = crate::headless::run(
            &ctx,
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(640., 480.))),
                ..Default::default()
            },
            |root| {
                egui::CentralPanel::default()
                    .show(root, |ui| list.show(ui, &model, &mut canvas, &mut selected));
            },
        );
        assert!(
            output
                .shapes
                .iter()
                .any(|shape| find(&shape.shape, expected).is_some())
        );
        let other = if expected == "No blocks yet" {
            "No matching blocks"
        } else {
            "No blocks yet"
        };
        assert!(
            !output
                .shapes
                .iter()
                .any(|shape| find(&shape.shape, other).is_some())
        );
        if model.nodes.is_empty() {
            model.nodes.push(Node {
                name: "Constant".into(),
                block: Block::Constant { value: 1. },
                inputs: vec![],
            });
        } else {
            model.nodes.clear();
        }
    }
}

#[test]
fn model_list_cache_tracks_names_types_and_topology_without_coefficient_copies() {
    let mut model = Model::default();
    model.nodes[0].block = Block::Filter {
        b: vec![0.5; 4097],
        a: vec![1.],
    };
    let address = match &model.nodes[0].block {
        Block::Filter { b, .. } => b.as_ptr(),
        _ => unreachable!(),
    };
    let mut list = List::default();
    list.sync(&model);
    let labels: Vec<_> = list.entries.iter().map(|e| e.label.as_ptr()).collect();
    for _ in 0..10 {
        list.sync(&model);
    }
    assert_eq!(
        labels,
        list.entries
            .iter()
            .map(|e| e.label.as_ptr())
            .collect::<Vec<_>>()
    );
    list.query = "  FILTER  ".into();
    list.sync(&model);
    assert!(list.visible.contains(&0));
    model.nodes[0].name = "\u{394}elta".into();
    list.query = "\u{3b4}ELTA".into();
    list.sync(&model);
    assert_eq!(list.visible, vec![0]);
    assert_eq!(list.entries[0].label, "1. \u{394}elta");
    assert_eq!(
        address,
        match &model.nodes[0].block {
            Block::Filter { b, .. } => b.as_ptr(),
            _ => unreachable!(),
        }
    );
    model.nodes.remove(0);
    list.sync(&model);
    assert!(list.visible.is_empty());
    list.query.clear();
    list.sync(&model);
    assert_eq!(list.visible, (0..model.nodes.len()).collect::<Vec<_>>());
    assert_eq!(list.entries[0].label, format!("1. {}", model.nodes[0].name));
    model.nodes[0].block = Block::Scope;
    list.query = "scope".into();
    list.sync(&model);
    assert!(list.visible.contains(&0));
    model.nodes.clear();
    list.sync(&model);
    assert!(list.entries.is_empty() && list.visible.is_empty());
}

fn find(shape: &egui::Shape, label: &str) -> Option<Rect> {
    match shape {
        egui::Shape::Text(text) if text.galley.text() == label => {
            Some(Rect::from_min_size(text.pos, text.galley.size()))
        }
        egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, label)),
        _ => None,
    }
}
fn point(output: &egui::FullOutput, label: &str) -> Pos2 {
    let (rect, clip) = output
        .shapes
        .iter()
        .find_map(|s| find(&s.shape, label).map(|r| (r, s.clip_rect)))
        .unwrap_or_else(|| panic!("Missing {label}"));
    assert!(
        clip.expand(0.1).contains_rect(rect),
        "Clipped {label}: {rect:?} in {clip:?}"
    );
    rect.center()
}
fn pointer(pos: Pos2, pressed: bool) -> Vec<Event> {
    vec![
        Event::PointerMoved(pos),
        Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        },
    ]
}

#[test]
fn model_list_virtualizes_512_rows_and_reveals_external_selection() {
    let model = Model {
        nodes: (0..512)
            .map(|i| Node {
                name: format!("Block{i}"),
                block: Block::Constant { value: i as f64 },
                inputs: vec![],
            })
            .collect(),
        ..Model::default()
    };
    let original = model.clone();
    let mut list = List::default();
    let mut canvas = Canvas::default();
    let mut selected = 0;
    let ctx = egui::Context::default();
    let frame = |list: &mut List, selected: &mut usize, canvas: &mut Canvas| {
        crate::headless::run(
            &ctx,
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(420., 360.))),
                ..Default::default()
            },
            |root| {
                egui::CentralPanel::default()
                    .show(root, |ui| list.show(ui, &model, canvas, selected));
            },
        )
    };
    for _ in 0..3 {
        frame(&mut list, &mut selected, &mut canvas);
    }
    assert!(list.rows_drawn < 12);
    selected = 511;
    canvas.select(511, Modifiers::NONE, 512, &mut selected);
    for _ in 0..3 {
        frame(&mut list, &mut selected, &mut canvas);
    }
    let output = frame(&mut list, &mut selected, &mut canvas);
    point(&output, "512. Block511");
    assert!(list.offset > 1000. && list.rows_drawn < 12);
    list.query = "Block17".into();
    for _ in 0..3 {
        frame(&mut list, &mut selected, &mut canvas);
    }
    assert_eq!(list.visible.len(), 11);
    assert_eq!(selected, 511);
    assert!(canvas.is_selected(511));
    list.query = "nothing matches".into();
    let output = frame(&mut list, &mut selected, &mut canvas);
    point(&output, "No matching blocks");
    assert_eq!(list.rows_drawn, 0);
    assert!(model.same_storage(&original));
    assert!(!canvas.take_changed());
}

#[test]
fn model_list_full_window_search_selection_and_clear_fit_both_viewports() {
    for size in [Vec2::new(1440., 900.), Vec2::new(960., 650.)] {
        let ctx = egui::Context::default();
        let mut tools = super::super::Toolkits {
            model_open: true,
            model_blocks_tab: true,
            ..Default::default()
        };
        let original = tools.model.clone();
        let frame = |tools: &mut super::super::Toolkits, events| {
            crate::headless::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                    events,
                    ..Default::default()
                },
                |root| {
                    let ctx = &root.ctx().clone();
                    tools.ui(ctx, false);
                    if ctx.current_pass_index() == 0 {
                        ctx.request_discard("check repeated layout");
                    }
                },
            )
        };
        for _ in 0..3 {
            frame(&mut tools, vec![]);
        }
        tools.block_list.query = "NOISE".into();
        for _ in 0..3 {
            frame(&mut tools, vec![]);
        }
        let output = frame(&mut tools, vec![]);
        point(
            &output,
            "Active block hidden by search; parameters still apply.",
        );
        let p = point(&output, "2. Noise");
        for pressed in [true, false] {
            frame(&mut tools, pointer(p, pressed));
        }
        assert_eq!(tools.selected, 1);
        assert!(tools.canvas.is_selected(1));
        let p = point(&frame(&mut tools, vec![]), "Clear");
        for pressed in [true, false] {
            frame(&mut tools, pointer(p, pressed));
        }
        assert!(tools.block_list.query.is_empty());
        assert_eq!(tools.block_list.visible.len(), 5);
        assert_eq!(tools.selected, 1);
        assert!(tools.model.same_storage(&original));
        assert!(!tools.take_diagram_change());
        tools.block_list.query = "missing".into();
        tools.restore_model(original.clone(), None, None);
        assert!(tools.block_list.query.is_empty());
    }
}

#[test]
fn model_list_search_accepts_text_without_diagram_shortcuts_and_bounds_input() {
    let ctx = egui::Context::default();
    let mut tools = super::super::Toolkits {
        model_open: true,
        model_blocks_tab: true,
        ..Default::default()
    };
    let original = tools.model.clone();
    let frame = |tools: &mut super::super::Toolkits, events| {
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
            },
        )
    };
    for _ in 0..3 {
        frame(&mut tools, vec![]);
    }
    let p = point(&frame(&mut tools, vec![]), "Find blocks by name or type");
    for pressed in [true, false] {
        frame(&mut tools, pointer(p, pressed));
    }
    frame(&mut tools, vec![Event::Text("Noise".into())]);
    assert_eq!(tools.block_list.visible, vec![1]);
    frame(
        &mut tools,
        vec![Event::Key {
            key: egui::Key::Delete,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert!(tools.model.same_storage(&original));
    frame(&mut tools, vec![Event::Text("x".repeat(256))]);
    assert_eq!(tools.block_list.query.chars().count(), 128);
    assert!(!tools.take_diagram_change());
}

#[test]
#[ignore = "manual release CPU frame measurement; excludes native rendering and the rest of the desktop"]
fn model_list_benchmark_cpu_frames() {
    use std::time::Instant;
    let workers = rayon::current_num_threads();
    for count in [16, 128, 512] {
        let model = Model {
            nodes: (0..count)
                .map(|i| Node {
                    name: format!("Block{i:03}"),
                    block: Block::Constant { value: i as f64 },
                    inputs: vec![],
                })
                .collect(),
            ..Model::default()
        };
        let original = model.clone();
        for mode in ["all", "filtered"] {
            let ctx = egui::Context::default();
            let mut list = List::default();
            if mode == "filtered" {
                list.query = "Block00".into();
            }
            let mut canvas = Canvas::default();
            let mut selected = 0;
            for repetition in 0..12 {
                let start = Instant::now();
                for _ in 0..100 {
                    let output = crate::headless::run(
                        &ctx,
                        egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(
                                Pos2::ZERO,
                                Vec2::new(420., 360.),
                            )),
                            ..Default::default()
                        },
                        |root| {
                            egui::CentralPanel::default()
                                .show(root, |ui| list.show(ui, &model, &mut canvas, &mut selected));
                        },
                    );
                    std::hint::black_box(output);
                }
                let milliseconds = start.elapsed().as_secs_f64() * 10.;
                assert!(list.rows_drawn < 12);
                assert!(model.same_storage(&original));
                if repetition > 0 {
                    println!(
                        "{workers},{count},{mode},100,{repetition},{milliseconds:.6},{},{}",
                        list.visible.len(),
                        list.rows_drawn
                    );
                }
            }
        }
    }
}
