use super::*;
use matrust::simulation::Node;
use std::sync::atomic::AtomicBool;

fn same_status(cache: &mut Cache, model: &Model) {
    assert_eq!(
        cache.status(model, true).map_err(str::to_owned),
        model.validate().map(|_| ())
    );
}

#[test]
fn model_validation_reuses_success_and_rechecks_errors_without_model_copies() {
    let mut model = Model::default();
    model.nodes[3].block = Block::Filter {
        b: vec![0.25; 4097],
        a: vec![1.],
    };
    let mut cache = Cache::default();
    for _ in 0..20 {
        same_status(&mut cache, &model);
    }
    assert_eq!(cache.evaluations, 1);
    // A replacement allocation with identical content does not invalidate the hint.
    model = model.clone();
    same_status(&mut cache, &model);
    assert_eq!(cache.evaluations, 1);
    model.nodes[3].inputs[0] = 3;
    for _ in 0..20 {
        same_status(&mut cache, &model);
    }
    assert_eq!(cache.evaluations, 21);
    model.nodes[3].inputs[0] = 2;
    same_status(&mut cache, &model);
    assert_eq!(cache.evaluations, 22);
    assert!(size_of::<Cache>() <= 128);
    assert!(cache.nodes.is_empty());
    let mut small = Cache::default();
    for _ in 0..10 {
        same_status(&mut small, &Model::default());
    }
    assert_eq!(small.evaluations, 10);
    assert!(small.stamp.is_none() && small.nodes.is_empty());
}

#[test]
fn model_validation_tracks_settings_topology_names_layout_and_replacements() {
    let original = Model::default();
    let mut cache = Cache::default();
    same_status(&mut cache, &original);
    let edits: &[fn(&mut Model)] = &[
        |m| m.sample_time = 0.01,
        |m| m.steps = 0,
        |m| m.nodes[0].name = m.nodes[1].name.clone(),
        |m| m.nodes[3].inputs[0] = usize::MAX,
        |m| m.nodes[3].inputs.push(0),
        |m| {
            m.nodes.remove(4);
        },
        |m| m.nodes.swap(0, 1),
        |m| m.version = 99,
        |m| m.positions.push([0., 0.]),
        |m| {
            m.set_layout(vec![[0., 0.]; m.nodes.len()]).unwrap();
        },
        |m| m.nodes[3].block = Block::Gain { gain: 2. },
    ];
    for edit in edits {
        let mut model = original.clone();
        edit(&mut model);
        assert_ne!(
            stamp(&original, &mut vec![], 0, true),
            stamp(&model, &mut vec![], 0, true)
        );
        let before = cache.evaluations;
        same_status(&mut cache, &model);
        assert_eq!(cache.evaluations, before + 1);
        same_status(&mut cache, &original);
        assert_eq!(cache.evaluations, before + 2);
    }
    let mut positioned = original.clone();
    positioned.set_layout(vec![[0., 0.]; 5]).unwrap();
    same_status(&mut cache, &positioned);
    let before = cache.evaluations;
    positioned.positions[0][1] = f32::NAN;
    same_status(&mut cache, &positioned);
    assert_eq!(cache.evaluations, before + 1);
}

#[test]
fn model_validation_tracks_every_parameter_and_coefficient_bits() {
    let pairs = [
        (
            Block::Constant { value: 0. },
            Block::Constant { value: -0. },
        ),
        (Block::Gain { gain: 1. }, Block::Gain { gain: f64::NAN }),
        (
            Block::UnitDelay { initial: 0. },
            Block::UnitDelay { initial: 1. },
        ),
        (
            Block::Sine {
                amplitude: 1.,
                frequency_hz: 10.,
                phase: 0.,
            },
            Block::Sine {
                amplitude: 2.,
                frequency_hz: 10.,
                phase: 0.,
            },
        ),
        (
            Block::Sine {
                amplitude: 1.,
                frequency_hz: 10.,
                phase: 0.,
            },
            Block::Sine {
                amplitude: 1.,
                frequency_hz: 500.,
                phase: 0.,
            },
        ),
        (
            Block::Sine {
                amplitude: 1.,
                frequency_hz: 10.,
                phase: 0.,
            },
            Block::Sine {
                amplitude: 1.,
                frequency_hz: 10.,
                phase: 1.,
            },
        ),
        (
            Block::Step {
                time: 0.,
                before: 0.,
                after: 1.,
            },
            Block::Step {
                time: 1.,
                before: 0.,
                after: 1.,
            },
        ),
        (
            Block::Step {
                time: 0.,
                before: 0.,
                after: 1.,
            },
            Block::Step {
                time: 0.,
                before: 1.,
                after: 1.,
            },
        ),
        (
            Block::Step {
                time: 0.,
                before: 0.,
                after: 1.,
            },
            Block::Step {
                time: 0.,
                before: 0.,
                after: 2.,
            },
        ),
        (
            Block::Filter {
                b: vec![1., 0.],
                a: vec![1.],
            },
            Block::Filter {
                b: vec![1., f64::NAN],
                a: vec![1.],
            },
        ),
        (
            Block::Filter {
                b: vec![1., 0.],
                a: vec![1.],
            },
            Block::Filter {
                b: vec![1., 0.],
                a: vec![0.],
            },
        ),
        (
            Block::Filter {
                b: vec![1., 0.],
                a: vec![1.],
            },
            Block::Filter {
                b: vec![1.],
                a: vec![0., 1.],
            },
        ),
    ];
    for (before, after) in pairs {
        let mut model = Model::default();
        let mut cache = Cache::default();
        model.nodes[3].block = before;
        same_status(&mut cache, &model);
        let before = stamp(&model, &mut vec![], 0, true);
        model.nodes[3].block = after;
        assert_ne!(before, stamp(&model, &mut vec![], 0, true));
        same_status(&mut cache, &model);
        assert_eq!(cache.evaluations, 2);
    }
    for column in 0..6 {
        let mut model = Model::default();
        let mut cache = Cache::default();
        model.nodes[3].block = Block::SosFilter {
            sections: vec![[1., 0., 0., 1., 0., 0.]],
        };
        same_status(&mut cache, &model);
        let before = stamp(&model, &mut vec![], 0, true);
        if let Block::SosFilter { sections } = &mut model.nodes[3].block {
            sections[0][column] = f64::NAN;
        }
        same_status(&mut cache, &model);
        assert_ne!(before, stamp(&model, &mut vec![], 0, true));
        assert_eq!(cache.evaluations, 2);
    }
    // A previously cached valid hint never authorizes invalid execution.
    let mut model = Model::default();
    Cache::default().status(&model, true).unwrap();
    model.nodes[3].inputs[0] = 3;
    assert!(model.run(&AtomicBool::new(false)).is_err());
}

#[test]
fn model_validation_parallel_stamp_matches_serial_at_thresholds_and_after_edits() {
    let serial = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let parallel = rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .build()
        .unwrap();
    for count in [128, 256, 512] {
        let mut model = Model {
            steps: 1,
            ..Model::default()
        };
        model.nodes = vec![Node {
            name: "source".into(),
            block: Block::Constant { value: 1. },
            inputs: vec![],
        }];
        for index in 1..count - 1 {
            model.nodes.push(Node {
                name: format!("f{index}"),
                block: Block::Filter {
                    b: vec![0.25; 4097],
                    a: vec![1.],
                },
                inputs: vec![0],
            });
        }
        model.nodes.push(Node {
            name: "scope".into(),
            block: Block::Scope,
            inputs: vec![count - 2],
        });
        let coefficients = bounded_coefficients(&model).unwrap();
        let mut nodes = vec![];
        let before = serial.install(|| stamp(&model, &mut vec![], coefficients, true));
        assert_eq!(
            before,
            parallel.install(|| stamp(&model, &mut nodes, coefficients, true))
        );
        assert_eq!(nodes.len(), if count == 512 { 512 } else { 0 });
        let mut cache = Cache::default();
        parallel.install(|| {
            same_status(&mut cache, &model);
            same_status(&mut cache, &model);
        });
        assert_eq!(cache.evaluations, 1);
        let mut busy = Cache::default();
        parallel.install(|| {
            busy.status(&model, false).unwrap();
            busy.status(&model, false).unwrap();
        });
        assert_eq!(busy.evaluations, 1);
        assert!(busy.nodes.is_empty());
        assert_eq!(busy.stamp, cache.stamp);
        if let Block::Filter { b, .. } = &mut model.nodes[count - 2].block {
            b[4096] = f64::NAN;
        }
        let after = serial.install(|| stamp(&model, &mut vec![], coefficients, true));
        assert_ne!(before, after);
        assert_eq!(
            after,
            parallel.install(|| stamp(&model, &mut nodes, coefficients, true))
        );
        parallel.install(|| same_status(&mut cache, &model));
        assert_eq!(cache.evaluations, 2);
        assert!(cache.nodes.capacity() <= 512);
    }
}

#[test]
fn model_validation_bounds_reject_oversized_drafts_and_recover() {
    let original = Model::default();
    let edits: &[fn(&mut Model)] = &[
        |m| {
            m.nodes.resize(
                513,
                Node {
                    name: "x".into(),
                    block: Block::Sum,
                    inputs: vec![],
                },
            )
        },
        |m| m.positions.resize(513, [0., 0.]),
        |m| m.nodes[0].name = "x".repeat(513),
        |m| m.nodes[0].inputs.resize(33, 0),
        |m| {
            m.nodes[3].block = Block::Filter {
                b: vec![0.; 4098],
                a: vec![1.],
            }
        },
        |m| {
            m.nodes[3].block = Block::Filter {
                b: vec![1.],
                a: vec![0.; 4098],
            }
        },
        |m| {
            m.nodes[3].block = Block::SosFilter {
                sections: vec![[0.; 6]; 65],
            }
        },
    ];
    let mut cache = Cache::default();
    for edit in edits {
        same_status(&mut cache, &original);
        let before = cache.evaluations;
        let mut model = original.clone();
        edit(&mut model);
        assert_eq!(
            cache.status(&model, true),
            Err("Model exceeds editor validation bounds")
        );
        assert_eq!(cache.evaluations, before);
        same_status(&mut cache, &original);
        assert_eq!(cache.evaluations, before + 1);
    }
}
