//! Whole-model serial simulator timings, not per-block or Simulink comparisons.
use matrust::simulation::{Block, Model, Node, ProductInput as P, SwitchCriteria as C};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};
fn fixture(kind: &str, count: usize, steps: usize) -> (Model, f64) {
    let mut nodes = vec![
        Node {
            name: "One".into(),
            block: Block::Constant { value: 1. },
            inputs: vec![],
        },
        Node {
            name: "Signal".into(),
            block: Block::Constant { value: -0.25 },
            inputs: vec![],
        },
    ];
    let mut expected = -0.25;
    for i in 0..count {
        let prior = nodes.len() - 1;
        let active = if kind == "mixed" {
            ["product2", "unary", "switch"][i % 3]
        } else {
            kind
        };
        let (block, inputs) = match active {
            "product2" | "product32" => {
                let n = if active == "product2" { 2 } else { 32 };
                let mut inputs = vec![0; n];
                inputs[0] = prior;
                let mut operations = vec![P::Divide; n];
                operations[0] = P::Multiply;
                (Block::Product { operations }, inputs)
            }
            "unary" => {
                expected = -expected;
                (Block::UnaryMinus, vec![prior])
            }
            "switch" => (
                Block::Switch {
                    criteria: C::Greater,
                    threshold: 0.,
                },
                vec![prior, 0, 0],
            ),
            _ => (Block::Gain { gain: 1. }, vec![prior]),
        };
        nodes.push(Node {
            name: format!("Block {i}"),
            block,
            inputs,
        });
    }
    nodes.push(Node {
        name: "Output".into(),
        block: Block::Scope,
        inputs: vec![nodes.len() - 1],
    });
    (
        Model {
            nodes,
            steps,
            ..Model::default()
        },
        expected,
    )
}
fn main() -> Result<(), String> {
    let reverse = std::env::args().any(|arg| arg == "--reverse");
    let mut cases = Vec::new();
    for count in [8, 64, 256] {
        for steps in [1000, 10000] {
            for kind in [
                "legacy_gain",
                "product2",
                "product32",
                "unary",
                "switch",
                "mixed",
            ] {
                cases.push((kind, count, steps));
            }
        }
    }
    if reverse {
        cases.reverse();
    }
    let workers = if reverse { [8, 1] } else { [1, 8] };
    println!("kind,blocks,steps,workers,repeat,elapsed_ns");
    let cancel = AtomicBool::new(false);
    for workers in workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .map_err(|e| e.to_string())?;
        for &(kind, count, steps) in &cases {
            let (model, expected) = fixture(kind, count, steps);
            for _ in 0..2 {
                let result = pool.install(|| model.run(&cancel))?;
                if !result.traces[0]
                    .values
                    .data()
                    .iter()
                    .all(|&x| x == expected)
                {
                    return Err("Fixture output mismatch".into());
                }
            }
            for repeat in 0..7 {
                let started = Instant::now();
                pool.install(|| model.run(&cancel).map(|result| drop(black_box(result))))?;
                println!(
                    "{kind},{count},{steps},{workers},{repeat},{}",
                    started.elapsed().as_nanos()
                );
            }
        }
    }
    Ok(())
}
