use super::*;
use matrust::simulation::{Block, IntegratorMethod, Node};
use parameters::{Edit, Properties};
use std::sync::atomic::AtomicBool;

fn block(initial: f64, gain: f64) -> Block {
    Block::DiscreteIntegrator {
        initial,
        gain,
        method: IntegratorMethod::ForwardEuler,
    }
}
fn model() -> Model {
    Model {
        sample_time: 0.25,
        steps: 5,
        nodes: vec![
            Node {
                name: "Source".into(),
                block: Block::Constant { value: 2. },
                inputs: vec![],
            },
            Node {
                name: "Integrator".into(),
                block: block(-0., 1.),
                inputs: vec![0],
            },
            Node {
                name: "Scope".into(),
                block: Block::Scope,
                inputs: vec![1],
            },
        ],
        ..Model::default()
    }
}
fn find(shape: &egui::Shape, clip: Rect, text: &str) -> Option<Pos2> {
    match shape {
        egui::Shape::Text(t)
            if t.galley.text() == text && clip.contains_rect(t.visual_bounding_rect()) =>
        {
            Some(t.visual_bounding_rect().center())
        }
        egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, clip, text)),
        _ => None,
    }
}
fn frame(
    ctx: &egui::Context,
    tools: &mut super::super::Toolkits,
    size: Vec2,
    events: Vec<egui::Event>,
    busy: bool,
) -> egui::FullOutput {
    crate::headless::run(
        ctx,
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |root| {
            tools.ui(root.ctx(), busy);
            if root.ctx().current_pass_index() == 0 {
                root.ctx()
                    .request_discard("integrator insertion applies once");
            }
        },
    )
}
#[test]
fn actual_integrator_palette_pointer_preserves_model_io_guard_and_single_history_action() {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        for size in [Vec2::new(1440., 900.), Vec2::new(960., 650.)] {
            let ctx = egui::Context::default();
            ctx.set_theme(theme);
            let mut tools = super::super::Toolkits {
                model_open: true,
                add_kind: 15,
                ..Default::default()
            };
            tools.model.nodes.clear();
            tools.selected = 0;
            for _ in 0..3 {
                frame(&ctx, &mut tools, size, vec![], false);
            }
            let directory = tempfile::tempdir().unwrap();
            // Queue the actual model-save request without dispatching filesystem work.
            // Captured simulations permit diagram editing; model I/O is the real edit guard.
            assert!(!tools.save_model_path(&directory.path().join("model.mrsim"), None));
            assert!(tools.model_io_pending());
            let viewport = Rect::from_min_size(Pos2::ZERO, size);
            for pending in [true, false] {
                if !pending {
                    assert!(tools.model_write.take().is_some());
                }
                assert_eq!(tools.model_io_pending(), pending);
                for _ in 0..3 {
                    frame(&ctx, &mut tools, size, vec![], false);
                }
                let output = frame(&ctx, &mut tools, size, vec![], false);
                let pos = output
                    .shapes
                    .iter()
                    .find_map(|s| find(&s.shape, s.clip_rect.intersect(viewport), "Add block"))
                    .unwrap();
                for pressed in [true, false] {
                    frame(
                        &ctx,
                        &mut tools,
                        size,
                        vec![
                            egui::Event::PointerMoved(pos),
                            egui::Event::PointerButton {
                                pos,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ],
                        false,
                    );
                }
                assert_eq!(tools.model.nodes.len(), usize::from(!pending));
            }
            assert!(matches!(
                tools.model.nodes[0].block,
                Block::DiscreteIntegrator {
                    initial: 0.,
                    gain: 1.,
                    method: IntegratorMethod::ForwardEuler
                }
            ));
            assert_eq!(tools.model.nodes[0].inputs, [usize::MAX]);
            assert_eq!(tools.canvas.history.len(), 1);
            assert!(tools.take_diagram_change());
            let output = frame(&ctx, &mut tools, size, vec![], false);
            for label in [
                "Forward Euler; model sample time",
                "Initial output",
                "Integrator gain",
            ] {
                assert!(
                    output.shapes.iter().any(|s| find(
                        &s.shape,
                        s.clip_rect.intersect(viewport),
                        label
                    )
                    .is_some()),
                    "{label}"
                );
            }
            let before = tools.model.clone();
            tools.canvas.travel(
                &mut tools.model,
                &mut tools.selected,
                false,
                &mut String::new(),
            );
            assert!(tools.model.nodes.is_empty());
            tools.canvas.travel(
                &mut tools.model,
                &mut tools.selected,
                true,
                &mut String::new(),
            );
            assert!(tools.model.same_storage(&before));
        }
    }
}
#[test]
fn integrator_parameter_history_clipboard_and_real_session_keep_exact_model() {
    let mut source = model();
    let original = source.clone();
    let mut canvas = Canvas::default();
    let before = Properties::capture(&source.nodes[1]);
    let hash = topology::node_hash(&source.nodes[1]);
    source.nodes[1].block = block(3., 2.);
    assert_ne!(hash, topology::node_hash(&source.nodes[1]));
    canvas.record_edit(Edit::node(&source, 1, before).unwrap(), None);
    let edited = source.clone();
    canvas.travel(&mut source, &mut 1, false, &mut String::new());
    assert!(source.same_storage(&original));
    canvas.travel(&mut source, &mut 1, true, &mut String::new());
    assert!(source.same_storage(&edited));
    let text = clipboard::Fragment::capture(&source, &[0, 1, 2], 900.)
        .unwrap()
        .encode()
        .unwrap();
    let mut target = Model {
        sample_time: source.sample_time,
        steps: source.steps,
        nodes: vec![],
        ..Model::default()
    };
    clipboard::Change::insert(
        &mut target,
        clipboard::Fragment::decode(&text).unwrap(),
        900.,
        0.,
        vec![],
        HISTORY_BYTES,
    )
    .unwrap();
    assert!(matches!(
        target.nodes[1].block,
        Block::DiscreteIntegrator {
            initial: 3.,
            gain: 2.,
            ..
        }
    ));
    assert_eq!(
        target.run(&AtomicBool::new(false)).unwrap().traces[0]
            .values
            .data(),
        [3., 4., 5., 6., 7.]
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.json");
    let mut app = crate::Workspace::with_context(egui::Context::default(), true);
    app.toolkits.model = target.clone();
    matrust::session::SessionStore::new(&path)
        .save(app.snapshot())
        .unwrap();
    let restored = matrust::session::Session::load(&path).unwrap();
    assert!(restored.model.same_storage(&target));
    assert_eq!(
        restored.model.run(&AtomicBool::new(false)).unwrap().traces[0]
            .values
            .data(),
        [3., 4., 5., 6., 7.]
    );
    // Recovery/clipboard preserve finite invalid drafts so a typo remains editable.
    app.toolkits.model.nodes[1].block = block(3., 0.);
    let incomplete = app.toolkits.model.clone();
    assert!(incomplete.validate_storage().is_ok());
    assert!(incomplete.validate().is_err());
    let fragment = clipboard::Fragment::capture(&incomplete, &[0, 1, 2], 900.)
        .unwrap()
        .encode()
        .unwrap();
    assert!(clipboard::Fragment::decode(&fragment).is_ok());
    let invalid_path = directory.path().join("incomplete.json");
    matrust::session::SessionStore::new(&invalid_path)
        .save(app.snapshot())
        .unwrap();
    let restored = matrust::session::Session::load(&invalid_path).unwrap();
    assert!(restored.model.same_storage(&incomplete));
    assert!(restored.model.validate().is_err());
}
#[test]
fn existing_worker_simulation_publishes_integrator_trace_and_failed_retry_is_atomic() {
    let mut worker = crate::worker::Worker::new(egui::Context::default());
    worker
        .send
        .send(crate::worker::Job::Script("x=[1 2 3];alias=x;".into()).into())
        .unwrap();
    let initial = worker
        .receive
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    let shared = initial.vars["x"].clone();
    worker
        .send
        .send(crate::worker::Job::Simulate(model()).into())
        .unwrap();
    let reply = worker
        .receive
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    assert_eq!(
        reply.vars["sim_1"].as_numeric().unwrap().data(),
        [-0., 0.5, 1., 1.5, 2.]
    );
    assert!(reply.vars["x"].same_snapshot(&shared));
    assert!(reply.vars["alias"].same_snapshot(&shared));
    let trace = reply.vars["sim_1"].clone();
    let mut invalid = model();
    invalid.nodes[1].block = block(0., -1.);
    worker
        .send
        .send(crate::worker::Job::Simulate(invalid).into())
        .unwrap();
    let reply = worker
        .receive
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    assert!(reply.vars["sim_1"].same_snapshot(&trace));
    assert!(reply.vars["x"].same_snapshot(&shared));
    assert!(reply.result.err().unwrap().contains("positive finite gain"));
    worker.shutdown();
}
