use matrust::simulation::{Block, Model, Node};
use std::sync::atomic::AtomicBool;

fn chain(input: f64, blocks: Vec<Block>) -> Model {
    let mut nodes = vec![Node {
        name: "Input".into(),
        block: Block::Constant { value: input },
        inputs: vec![],
    }];
    for block in blocks.into_iter().chain([Block::Scope]) {
        let index = nodes.len();
        nodes.push(Node {
            name: format!("Block {index}"),
            block,
            inputs: vec![index - 1],
        });
    }
    Model {
        nodes,
        steps: 4,
        ..Model::default()
    }
}

fn output(model: &Model) -> Vec<f64> {
    model.run(&AtomicBool::new(false)).unwrap().traces[0]
        .values
        .data()
        .to_vec()
}

#[test]
fn simulation_arithmetic_abs_signed_zero_extremes_and_bias_composition() {
    for (input, expected) in [
        (-f64::MAX, f64::MAX),
        (-2., 2.),
        (-0., 0.),
        (0., 0.),
        (f64::MIN_POSITIVE, f64::MIN_POSITIVE),
        (f64::MAX, f64::MAX),
    ] {
        for value in output(&chain(input, vec![Block::Abs])) {
            assert_eq!(value.to_bits(), expected.to_bits());
        }
    }
    assert_eq!(
        output(&chain(-3., vec![Block::Bias { bias: 1. }, Block::Abs])),
        [2.; 4]
    );
}

#[test]
fn simulation_arithmetic_saturation_boundaries_signed_zero_and_equal_limits() {
    for (input, expected) in [
        (-3_f64, -2_f64),
        (-2., -2.),
        (-0., -0.),
        (1., 1.),
        (2., 2.),
        (3., 2.),
    ] {
        let model = chain(
            input,
            vec![Block::Saturation {
                lower: -2.,
                upper: 2.,
            }],
        );
        for value in output(&model) {
            assert_eq!(value.to_bits(), expected.to_bits());
        }
    }
    for input in [-f64::MAX, 4., f64::MAX] {
        assert_eq!(
            output(&chain(
                input,
                vec![Block::Saturation {
                    lower: 4.,
                    upper: 4.
                }]
            )),
            [4.; 4]
        );
    }
}

#[test]
fn simulation_arithmetic_bounds_arity_overflow_and_cancel_reject_cleanly() {
    for block in [
        Block::Bias { bias: f64::NAN },
        Block::Saturation {
            lower: f64::NEG_INFINITY,
            upper: 1.,
        },
        Block::Saturation {
            lower: 0.,
            upper: f64::INFINITY,
        },
        Block::Saturation {
            lower: 2.,
            upper: -2.,
        },
    ] {
        assert!(chain(1., vec![block]).validate().is_err());
    }
    for block in [
        Block::Abs,
        Block::Bias { bias: 1. },
        Block::Saturation {
            lower: -1.,
            upper: 1.,
        },
    ] {
        let mut model = chain(1., vec![block]);
        model.nodes[1].inputs.clear();
        assert!(model.validate().is_err());
        model.nodes[1].inputs = vec![0, 0];
        assert!(model.validate().is_err());
    }
    assert!(
        chain(f64::MAX, vec![Block::Bias { bias: f64::MAX }])
            .run(&AtomicBool::new(false))
            .err()
            .unwrap()
            .contains("nonfinite")
    );
    assert!(
        chain(1., vec![Block::Abs])
            .run(&AtomicBool::new(true))
            .is_err()
    );
}

#[test]
fn simulation_arithmetic_delay_feedback_keeps_simultaneous_state_rules() {
    let model = Model {
        nodes: vec![
            Node {
                name: "Delay".into(),
                block: Block::UnitDelay { initial: -2. },
                inputs: vec![3],
            },
            Node {
                name: "Magnitude".into(),
                block: Block::Abs,
                inputs: vec![0],
            },
            Node {
                name: "Offset".into(),
                block: Block::Bias { bias: 0.5 },
                inputs: vec![1],
            },
            Node {
                name: "Limit".into(),
                block: Block::Saturation {
                    lower: -3.,
                    upper: 3.,
                },
                inputs: vec![2],
            },
            Node {
                name: "Output".into(),
                block: Block::Scope,
                inputs: vec![0],
            },
        ],
        steps: 5,
        ..Model::default()
    };
    assert_eq!(output(&model), [-2., 2.5, 3., 3., 3.]);
    let mut algebraic = model;
    algebraic.nodes[0].block = Block::Bias { bias: 0. };
    assert!(
        algebraic
            .validate()
            .err()
            .unwrap()
            .contains("Algebraic loop")
    );
}

#[test]
fn simulation_arithmetic_native_files_preserve_bits_and_legacy_models() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("arithmetic.mrsim");
    let mut model = chain(
        -2.,
        vec![
            Block::Abs,
            Block::Bias { bias: -0. },
            Block::Saturation {
                lower: -0.,
                upper: 1.,
            },
        ],
    );
    model
        .set_layout(
            (0..model.nodes.len())
                .map(|i| [24. + i as f32 * 170., 24.])
                .collect(),
        )
        .unwrap();
    model.save(&path).unwrap();
    let reopened = Model::load(&path).unwrap();
    assert!(model.same_storage(&reopened));
    assert_eq!(output(&model), output(&reopened));
    let mut changed = reopened.clone();
    changed.nodes[2].block = Block::Bias { bias: 0. };
    assert!(!reopened.same_storage(&changed));
    changed = reopened.clone();
    changed.nodes[3].block = Block::Saturation {
        lower: 0.,
        upper: 1.,
    };
    assert!(!reopened.same_storage(&changed));

    let legacy = r#"{"version":1,"sample_time":0.001,"steps":2,"nodes":[{"name":"old source","block":{"type":"Constant","value":2.0},"inputs":[]},{"name":"old gain","block":{"type":"Gain","gain":3.0},"inputs":[0]},{"name":"old scope","block":{"type":"Scope"},"inputs":[1]}]}"#;
    std::fs::write(&path, legacy).unwrap();
    assert_eq!(output(&Model::load(&path).unwrap()), [6.; 2]);
}

#[test]
fn simulation_arithmetic_drafts_are_storable_but_nonfinite_save_is_atomic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("draft.mrsim");
    let draft = chain(
        1.,
        vec![Block::Saturation {
            lower: 2.,
            upper: 1.,
        }],
    );
    assert!(draft.validate().is_err());
    draft.save(&path).unwrap();
    assert!(Model::load(&path).unwrap().same_storage(&draft));
    let saved = std::fs::read(&path).unwrap();
    let invalid = chain(1., vec![Block::Bias { bias: f64::NAN }]);
    assert!(invalid.save(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), saved);
}

#[test]
fn simulation_arithmetic_signed_zero_limits_and_bias_preserve_ieee_operations() {
    for (input, bias, expected) in [(-0_f64, -0_f64, -0_f64), (-0., 0., 0.), (0., -0., 0.)] {
        for value in output(&chain(input, vec![Block::Bias { bias }])) {
            assert_eq!(value.to_bits(), expected.to_bits());
        }
    }
    for (input, lower, upper, expected) in [
        (-0_f64, 0_f64, 0_f64, -0_f64),
        (0., -0., -0., 0.),
        (-1., -0., 0., -0.),
        (1., -0., 0., 0.),
        (-1., 0., -0., 0.),
        (1., 0., -0., -0.),
    ] {
        for value in output(&chain(input, vec![Block::Saturation { lower, upper }])) {
            assert_eq!(value.to_bits(), expected.to_bits());
        }
    }
}

#[test]
fn simulation_arithmetic_session_recovery_keeps_parameter_bits_and_dirty_baseline() {
    use matrust::{
        document::Document,
        session::{Session, SessionStore},
        simulation::document::ModelFile,
    };
    let dir = tempfile::tempdir().unwrap();
    let session_path = dir.path().join("session.json");
    let model_path = dir.path().join("baseline.mrsim");
    let baseline = chain(
        -0.,
        vec![
            Block::Abs,
            Block::Bias { bias: -0. },
            Block::Saturation {
                lower: -0.,
                upper: 0.5,
            },
        ],
    );
    baseline.save(&model_path).unwrap();
    let mut model = baseline.clone();
    model.nodes[2].block = Block::Bias { bias: 0. };
    model.nodes[3].block = Block::Saturation {
        lower: -0.,
        upper: -0.,
    };
    let session = Session {
        layout_presets: Default::default(),
        export: Default::default(),
        workspace: None,
        workspace_manifest: None,
        preview: None,
        eps_renderer: None,
        figure_manifest: None,
        figures: None,
        layout: Default::default(),
        version: 1,
        documents: vec![Document::new()],
        active: 0,
        folder: dir.path().to_owned(),
        recent: vec![],
        history: vec![],
        model: model.clone(),
        model_path: Some(model_path.clone()),
        model_baseline: Some(baseline.clone()),
    };
    let mut store = SessionStore::new(&session_path);
    store.save(session.clone()).unwrap();
    let recovered = Session::load(&session_path).unwrap();
    assert!(recovered.model.same_storage(&model));
    assert!(
        recovered
            .model_baseline
            .as_ref()
            .unwrap()
            .same_storage(&baseline)
    );
    let file = ModelFile::restored(recovered.model_path, recovered.model_baseline);
    assert!(file.dirty(&recovered.model));
    assert!(!file.dirty(&baseline));
    assert!(file.external_change().unwrap().is_none());
    assert_eq!(output(&recovered.model), output(&model));
    let previous = std::fs::read(&session_path).unwrap();
    let mut invalid = session;
    invalid.model.nodes[2].block = Block::Bias { bias: f64::NAN };
    assert!(store.save(invalid).is_err());
    assert_eq!(std::fs::read(&session_path).unwrap(), previous);
}
