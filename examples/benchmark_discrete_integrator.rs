//! Whole-model serial state update; worker counts are execution-context controls.
use matrust::simulation::{Block, IntegratorMethod, Model, Node};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};

fn fixture(count: usize, steps: usize, feedback: bool, integrator: bool) -> Model {
    let mut nodes = vec![Node {
        name: "Input".into(),
        block: Block::Constant { value: 0.125 },
        inputs: vec![],
    }];
    let mut output = 0;
    for i in 0..count {
        let state = nodes.len();
        nodes.push(Node {
            name: format!("State{i}"),
            block: if integrator {
                Block::DiscreteIntegrator {
                    initial: 1.,
                    gain: 1.,
                    method: IntegratorMethod::ForwardEuler,
                }
            } else {
                Block::UnitDelay { initial: 1. }
            },
            inputs: vec![if feedback { state + 1 } else { 0 }],
        });
        if feedback {
            nodes.push(Node {
                name: format!("Feedback{i}"),
                block: Block::Gain { gain: -1. },
                inputs: vec![state],
            });
        }
        output = state;
    }
    nodes.push(Node {
        name: "Output".into(),
        block: Block::Scope,
        inputs: vec![output],
    });
    Model {
        sample_time: 1. / 1024.,
        steps,
        nodes,
        ..Model::default()
    }
}
fn main() {
    if cfg!(debug_assertions) {
        panic!("Release required");
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert!(args.is_empty() || args == ["--reverse"]);
    let mut schedule = Vec::new();
    for count in [1, 16, 128] {
        for steps in [1024, 65536] {
            for feedback in [false, true] {
                for integrator in [false, true] {
                    for workers in [1, 8] {
                        schedule.push((count, steps, feedback, integrator, workers));
                    }
                }
            }
        }
    }
    if !args.is_empty() {
        schedule.reverse();
    }
    let stop = AtomicBool::new(false);
    println!("states,steps,feedback,integrator,workers,sample,elapsed_ns,output_bytes");
    for (count, steps, feedback, integrator, workers) in schedule {
        let model = fixture(count, steps, feedback, integrator);
        let before = model.clone();
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        let mut expected = Vec::with_capacity(steps);
        let mut state = 1_f64;
        for _ in 0..steps {
            expected.push(state);
            let input = if feedback { -state } else { 0.125 };
            state = if integrator {
                state + input / 1024.
            } else {
                input
            };
        }
        for iteration in 0..9 {
            let start = Instant::now();
            let result = pool.install(|| model.run(&stop)).unwrap();
            let elapsed = start.elapsed().as_nanos();
            black_box(&result);
            assert!(elapsed > 0);
            assert_eq!(result.traces.len(), 1);
            assert_eq!(
                result.traces[0]
                    .values
                    .data()
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>(),
                expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
            );
            for (n, t) in result.time.data().iter().enumerate() {
                assert_eq!(*t, n as f64 / 1024.);
            }
            assert!(model.same_storage(&before));
            if iteration >= 2 {
                println!(
                    "{count},{steps},{feedback},{integrator},{workers},{},{elapsed},{}",
                    iteration - 2,
                    steps * 16
                );
            }
            drop(result);
        }
    }
}
