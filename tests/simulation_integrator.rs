use matrust::simulation::{Block, IntegratorMethod, Model, Node};
use std::sync::atomic::AtomicBool;

fn integrator(initial: f64, gain: f64) -> Block {
    Block::DiscreteIntegrator {
        initial,
        gain,
        method: IntegratorMethod::ForwardEuler,
    }
}
fn constant(initial: f64, gain: f64, input: f64, dt: f64, steps: usize) -> Model {
    Model {
        sample_time: dt,
        steps,
        nodes: vec![
            Node {
                name: "Input".into(),
                block: Block::Constant { value: input },
                inputs: vec![],
            },
            Node {
                name: "Integrator".into(),
                block: integrator(initial, gain),
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
fn output(model: &Model) -> Vec<f64> {
    model
        .run(&AtomicBool::new(false))
        .unwrap()
        .traces
        .remove(0)
        .values
        .data()
        .to_vec()
}
#[test]
fn forward_euler_first_output_and_constant_closed_form_are_exact() {
    for (initial, gain, input, dt) in [(2., 2., 3., 0.125), (-3., 0.5, -2., 0.25), (0., 1., 0., 1.)]
    {
        let model = constant(initial, gain, input, dt, 33);
        let before = model.clone();
        let result = output(&model);
        for (n, value) in result.iter().enumerate() {
            assert_eq!(*value, initial + n as f64 * gain * dt * input);
        }
        assert!(model.same_storage(&before));
    }
    let model = constant(-0., 1., 2., 0.25, 1);
    assert_eq!(output(&model)[0].to_bits(), (-0f64).to_bits());
}
#[test]
fn step_uses_previous_input_and_cascades_update_simultaneously() {
    let mut model = constant(0., 2., 1., 0.25, 6);
    model.nodes[0].block = Block::Step {
        time: 0.5,
        before: 0.,
        after: 2.,
    };
    assert_eq!(output(&model), [0., 0., 0., 1., 2., 3.]);
    model.nodes = vec![
        Node {
            name: "Second".into(),
            block: integrator(0., 1.),
            inputs: vec![1],
        },
        Node {
            name: "First".into(),
            block: integrator(0., 1.),
            inputs: vec![2],
        },
        Node {
            name: "Input".into(),
            block: Block::Constant { value: 1. },
            inputs: vec![],
        },
        Node {
            name: "Scope".into(),
            block: Block::Scope,
            inputs: vec![0],
        },
    ];
    model.sample_time = 1.;
    assert_eq!(output(&model), [0., 0., 1., 3., 6., 10.]);
}
#[test]
fn genuine_feedback_has_no_direct_feedthrough_and_delay_still_works() {
    let mut model = constant(1., 1., 0., 0.25, 9);
    model.nodes[0] = Node {
        name: "Negative feedback".into(),
        block: Block::Gain { gain: -1. },
        inputs: vec![1],
    };
    let got = output(&model);
    for (n, value) in got.iter().enumerate() {
        assert_eq!(*value, 0.75f64.powi(n as i32));
    }
    model.nodes[0].block = Block::Gain { gain: 1. };
    model.nodes[1].block = Block::UnitDelay { initial: -2. };
    assert_eq!(output(&model), vec![-2.; 9]);
    model.nodes[1].block = Block::Gain { gain: 1. };
    assert!(model.validate().unwrap_err().contains("Algebraic loop"));
}
#[test]
fn invalid_parameters_methods_ports_and_overflow_are_not_silently_accepted() {
    for (initial, gain) in [(f64::NAN, 1.), (0., 0.), (0., -1.), (0., f64::INFINITY)] {
        let model = constant(initial, gain, 1., 0.1, 3);
        assert!(model.validate().is_err());
        assert_eq!(
            model.validate_storage().is_ok(),
            initial.is_finite() && gain.is_finite()
        );
    }
    let default_method = serde_json::from_value::<Block>(
        serde_json::json!({"type":"DiscreteIntegrator","initial":0.,"gain":1.}),
    )
    .unwrap();
    assert!(matches!(
        default_method,
        Block::DiscreteIntegrator {
            method: IntegratorMethod::ForwardEuler,
            ..
        }
    ));
    for method in [
        "BackwardEuler",
        "Trapezoidal",
        "Accumulation: Forward Euler",
    ] {
        assert!(serde_json::from_value::<Block>(serde_json::json!({"type":"DiscreteIntegrator","initial":0.,"gain":1.,"method":method})).is_err());
    }
    assert!(
        serde_json::from_value::<Block>(
            serde_json::json!({"type":"DiscreteIntegrator","initial":0.,"gain":1.,"reset":true})
        )
        .is_err()
    );
    let mut model = constant(0., 1., 1., 1., 2);
    model.nodes[1].inputs.clear();
    assert!(model.validate().is_err());
    let model = constant(f64::MAX, 2., f64::MAX, 1., 1);
    assert!(
        model
            .run(&AtomicBool::new(false))
            .err()
            .unwrap()
            .contains("nonfinite next state at sample 0")
    );
    let model = constant(0., f64::MAX, 1., 2., 1);
    assert!(model.validate().is_err());
}
#[test]
fn native_model_roundtrip_preserves_bits_and_changed_parameters_dirty_the_document() {
    use matrust::simulation::document::ModelFile;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("integrator.mrsim");
    let mut model = constant(-0., 1.25, -2., 0.125, 8);
    model
        .set_layout(vec![[10., 10.], [210., 10.], [410., 10.]])
        .unwrap();
    model.save(&path).unwrap();
    let (mut reopened, file) = ModelFile::open(&path).unwrap();
    assert!(model.same_storage(&reopened));
    assert!(!file.dirty(&reopened));
    assert_eq!(output(&model), output(&reopened));
    if let Block::DiscreteIntegrator { initial, .. } = &mut reopened.nodes[1].block {
        *initial = 0.;
    }
    assert!(file.dirty(&reopened));
    let before = std::fs::read(&path).unwrap();
    if let Block::DiscreteIntegrator { gain, .. } = &mut reopened.nodes[1].block {
        *gain = f64::INFINITY;
    }
    assert!(reopened.save(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}
