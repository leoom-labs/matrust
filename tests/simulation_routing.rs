use matrust::simulation::{Block, Model, Node, ProductInput as P, SwitchCriteria as C};
use std::sync::atomic::AtomicBool;

fn fixture(values: &[f64], block: Block) -> Model {
    let mut nodes: Vec<_> = values
        .iter()
        .enumerate()
        .map(|(i, &value)| Node {
            name: format!("Input {i}"),
            block: Block::Constant { value },
            inputs: vec![],
        })
        .collect();
    nodes.push(Node {
        name: "Operation".into(),
        block,
        inputs: (0..values.len()).collect(),
    });
    nodes.push(Node {
        name: "Output".into(),
        block: Block::Scope,
        inputs: vec![values.len()],
    });
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
fn scalar_product_port_order_reciprocal_bounds_and_signed_zero() {
    for (values, operations, expected) in [
        (vec![3., 4.], vec![P::Multiply; 2], 12.),
        (vec![3., 4.], vec![P::Multiply, P::Divide], 0.75),
        (
            vec![4., 3., 2.],
            vec![P::Divide, P::Multiply, P::Divide],
            0.375,
        ),
        (vec![4.], vec![P::Divide], 0.25),
        (vec![-0.], vec![P::Multiply], -0.),
        (vec![2.; 32], vec![P::Multiply; 32], 4_294_967_296.),
    ] {
        let expected: f64 = expected;
        let model = fixture(&values, Block::Product { operations });
        for y in output(&model) {
            assert_eq!(y.to_bits(), expected.to_bits());
        }
    }
    for n in [0, 33] {
        let m = fixture(
            &vec![1.; n],
            Block::Product {
                operations: vec![P::Multiply; n],
            },
        );
        assert!(m.validate().is_err());
        assert!(m.validate_storage().is_err());
    }
    let mut m = fixture(
        &[1., 2.],
        Block::Product {
            operations: vec![P::Multiply],
        },
    );
    assert!(m.validate().unwrap_err().contains("requires 1 inputs"));
    m.nodes[2].inputs = vec![usize::MAX];
    assert!(m.validate().is_err());
    let mut bounded = fixture(
        &[1.],
        Block::Product {
            operations: vec![P::Multiply],
        },
    );
    for i in 0..255 {
        bounded.nodes.push(Node {
            name: format!("Many ports {i}"),
            block: Block::Product {
                operations: vec![P::Multiply; 32],
            },
            inputs: vec![0; 32],
        });
    }
    bounded.steps = 20_000;
    assert!(bounded.validate().is_ok());
    bounded.steps = 40_000;
    assert!(
        bounded
            .validate()
            .unwrap_err()
            .contains("storage/work limit")
    );
}
#[test]
fn scalar_unary_minus_is_bit_exact_and_switch_obeys_boundary_and_port_order() {
    for x in [
        0.,
        -0.,
        f64::MAX,
        -f64::MAX,
        f64::from_bits(1),
        -f64::from_bits(1),
    ] {
        for y in output(&fixture(&[x], Block::UnaryMinus)) {
            assert_eq!(y.to_bits(), (-x).to_bits());
        }
    }
    for (criteria, threshold, control, first) in [
        (C::Greater, 2., 1., false),
        (C::Greater, 2., 2., false),
        (C::Greater, 2., 3., true),
        (C::GreaterEqual, 2., 1., false),
        (C::GreaterEqual, 2., 2., true),
        (C::Nonzero, 99., 0., false),
        (C::Nonzero, 99., -0., false),
        (C::Nonzero, 99., -f64::from_bits(1), true),
    ] {
        let m = fixture(
            &[-0., control, 7.],
            Block::Switch {
                criteria,
                threshold,
            },
        );
        for y in output(&m) {
            assert_eq!(
                y.to_bits(),
                if first {
                    (-0_f64).to_bits()
                } else {
                    7_f64.to_bits()
                }
            );
        }
    }
}
#[test]
fn scalar_routing_rejects_nonfinite_intermediates_and_eager_hidden_branches() {
    for (values, operations) in [
        (vec![1., 0.], vec![P::Multiply, P::Divide]),
        (vec![f64::MAX, 2., 0.], vec![P::Multiply; 3]),
        (vec![f64::from_bits(1), 0.], vec![P::Divide, P::Multiply]),
    ] {
        assert!(
            fixture(&values, Block::Product { operations })
                .run(&AtomicBool::new(false))
                .err()
                .unwrap()
                .contains("nonfinite")
        );
    }
    for threshold in [f64::NAN, f64::INFINITY] {
        assert!(
            fixture(
                &[1., 0., 2.],
                Block::Switch {
                    criteria: C::Nonzero,
                    threshold
                }
            )
            .validate()
            .is_err()
        );
    }
    let mut m = fixture(
        &[1., 0., 2.],
        Block::Switch {
            criteria: C::Greater,
            threshold: 0.,
        },
    );
    m.nodes[0] = Node {
        name: "Hidden invalid".into(),
        block: Block::Product {
            operations: vec![P::Divide],
        },
        inputs: vec![1],
    };
    assert!(
        m.run(&AtomicBool::new(false))
            .err()
            .unwrap()
            .contains("Hidden invalid")
    );
    assert!(
        fixture(&[1.], Block::UnaryMinus)
            .run(&AtomicBool::new(true))
            .is_err()
    );
}
#[test]
fn scalar_routing_topological_order_and_delay_feedback() {
    let m = Model {
        steps: 5,
        nodes: vec![
            Node {
                name: "Out".into(),
                block: Block::Scope,
                inputs: vec![1],
            },
            Node {
                name: "Delay".into(),
                block: Block::UnitDelay { initial: 2. },
                inputs: vec![2],
            },
            Node {
                name: "Switch".into(),
                block: Block::Switch {
                    criteria: C::Nonzero,
                    threshold: 0.,
                },
                inputs: vec![3, 4, 1],
            },
            Node {
                name: "Negate".into(),
                block: Block::UnaryMinus,
                inputs: vec![5],
            },
            Node {
                name: "Control".into(),
                block: Block::Constant { value: 1. },
                inputs: vec![],
            },
            Node {
                name: "Product".into(),
                block: Block::Product {
                    operations: vec![P::Multiply, P::Divide],
                },
                inputs: vec![1, 4],
            },
        ],
        ..Model::default()
    };
    assert_eq!(output(&m), [2., -2., 2., -2., 2.]);
    let mut looped = m.clone();
    looped.nodes[1].block = Block::UnaryMinus;
    assert!(looped.validate().unwrap_err().contains("Algebraic loop"));
    // A control-selected branch still creates a scheduling dependency.
    let mut hidden = m;
    hidden.nodes[2].inputs[2] = 2;
    assert!(hidden.validate().unwrap_err().contains("Algebraic loop"));
}
#[test]
fn scalar_routing_native_roundtrip_defaults_strict_fields_and_atomic_invalid_save() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("routing.mrsim");
    for block in [
        Block::Product {
            operations: vec![P::Multiply, P::Divide, P::Multiply],
        },
        Block::Switch {
            criteria: C::GreaterEqual,
            threshold: -0.,
        },
    ] {
        let mut m = fixture(&[2., 1., 3.], block);
        m.set_layout(
            (0..m.nodes.len())
                .map(|i| [24. + i as f32 * 180., 24.])
                .collect(),
        )
        .unwrap();
        m.save(&path).unwrap();
        let reopened = Model::load(&path).unwrap();
        assert!(m.same_storage(&reopened));
        assert_eq!(output(&m), output(&reopened));
        let saved = std::fs::read(&path).unwrap();
        m.nodes[3].block = Block::Switch {
            criteria: C::Nonzero,
            threshold: f64::NAN,
        };
        assert!(m.save(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), saved);
    }
    assert!(
        matches!(serde_json::from_str::<Block>(r#"{"type":"Product"}"#).unwrap(),Block::Product{operations} if operations==vec![P::Multiply;2])
    );
    assert!(matches!(
        serde_json::from_str::<Block>(r#"{"type":"Switch"}"#).unwrap(),
        Block::Switch {
            criteria: C::Greater,
            threshold: 0.
        }
    ));
    for text in [
        r#"{"type":"Product","operations":["+"]}"#,
        r#"{"type":"Product","multiplication":"Matrix"}"#,
        r#"{"type":"Switch","criteria":"u2 == 0"}"#,
        r#"{"type":"UnaryMinus","type_name":"single"}"#,
    ] {
        assert!(serde_json::from_str::<Block>(text).is_err(), "{text}");
    }
    Model::default().save(&path).unwrap();
    assert!(Model::default().same_storage(&Model::load(&path).unwrap()));
}
#[test]
fn scalar_routing_worker_configs_are_bit_identical_and_model_immutable() {
    let model = fixture(
        &[4., 3., 2.],
        Block::Product {
            operations: vec![P::Divide, P::Multiply, P::Divide],
        },
    );
    let original = model.clone();
    let mut expected = None;
    for workers in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        let actual = pool
            .install(|| output(&model))
            .into_iter()
            .map(f64::to_bits)
            .collect::<Vec<_>>();
        if let Some(expected) = &expected {
            assert_eq!(&actual, expected);
        } else {
            expected = Some(actual);
        }
        assert!(model.same_storage(&original));
    }
}
