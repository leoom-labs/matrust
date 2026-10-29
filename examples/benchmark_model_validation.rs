//! Paired readiness-hint helpers; excludes model setup, rendering and simulation.
#[allow(dead_code)]
#[path = "../src/desktop/model_topology.rs"]
mod topology;
// Shared desktop module also contains file gestures outside this benchmark.
#[allow(dead_code)]
#[path = "../src/desktop/model_validation.rs"]
mod validation;
use matrust::simulation::{Block, Model, Node};
use std::{hint::black_box, time::Instant};

fn fixture(case: &str) -> Model {
    let mut model = Model::default();
    if case != "default" {
        model.nodes[3].block = Block::Filter {
            b: vec![0.25; 4097],
            a: vec![1.],
        };
    }
    if let Some(count) = case.strip_prefix("filters_") {
        let count: usize = count.parse().unwrap();
        model.steps = 1;
        model.nodes = vec![Node {
            name: "source".into(),
            block: Block::Constant { value: 1. },
            inputs: vec![],
        }];
        for index in 1..count - 1 {
            model.nodes.push(Node {
                name: format!("filter{index}"),
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
    }
    if case == "invalid_filter"
        && let Block::Filter { a, .. } = &mut model.nodes[3].block
    {
        a[0] = 0.;
    }
    if case == "invalid_graph" {
        model.nodes[3].inputs[0] = 3;
    }
    model
}

fn main() {
    let workers = rayon::current_num_threads();
    println!("workers,case,operation,cycles,repetition,milliseconds_per_cycle");
    for case in [
        "default",
        "large_filter",
        "filters_128",
        "filters_256",
        "filters_512",
        "invalid_filter",
        "invalid_graph",
    ] {
        let mut model = fixture(case);
        let original = model.clone();
        let expected = model.validate().map(|_| ());
        assert_eq!(expected.is_ok(), !case.starts_with("invalid"));
        let mut cache = validation::Cache::default();
        assert_eq!(cache.status(&model, true).map_err(str::to_owned), expected);
        let cycles = if case.starts_with("filters_") {
            10
        } else {
            1000
        };
        let operations = [
            "full_validation",
            "cached_unchanged",
            "cached_changed",
            "cached_busy",
        ];
        for repetition in 0..12 {
            // Rotate helper order to limit systematic temperature/order effects.
            for offset in 0..operations.len() {
                let operation = operations[(repetition + offset) % operations.len()];
                let start = Instant::now();
                for _ in 0..cycles {
                    match operation {
                        "full_validation" => {
                            black_box(black_box(&model).validate().is_ok());
                        }
                        "cached_unchanged" => {
                            black_box(cache.status(black_box(&model), true).is_ok());
                        }
                        "cached_busy" => {
                            black_box(cache.status(black_box(&model), false).is_ok());
                        }
                        _ => {
                            // Both step counts remain valid; each edit forces refresh.
                            model.steps = if model.steps == original.steps {
                                original.steps + 1
                            } else {
                                original.steps
                            };
                            black_box(cache.status(black_box(&model), true).is_ok());
                        }
                    }
                }
                let ms = start.elapsed().as_secs_f64() * 1000. / cycles as f64;
                assert!(model.same_storage(&original));
                assert_eq!(cache.status(&model, true).map_err(str::to_owned), expected);
                if repetition > 0 {
                    println!("{workers},{case},{operation},{cycles},{repetition},{ms:.9}");
                }
            }
        }
    }
}
