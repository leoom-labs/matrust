//! Whole-model serial arithmetic timings; no Simulink or multicore speedup claim.
//! This also compiles against the old enum: --legacy-only avoids new JSON tags.
use matrust::simulation::{Block, Model, Node};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};

fn operation(kind: &str, index: usize) -> (&str, serde_json::Value) {
    let kind = if kind == "mixed" {
        ["abs", "bias", "saturation"][index % 3]
    } else {
        kind
    };
    let block = match kind {
        "abs" => serde_json::json!({"type":"Abs"}),
        "bias" => serde_json::json!({"type":"Bias","bias":0.001}),
        "saturation" => serde_json::json!({"type":"Saturation","lower":-0.2,"upper":0.2}),
        _ => serde_json::json!({"type":"Gain","gain":0.999}),
    };
    (kind, block)
}

fn fixture(kind: &str, count: usize, steps: usize) -> Result<(Model, f64), String> {
    let mut nodes = vec![Node {
        name: "Input".into(),
        block: Block::Constant { value: -0.25 },
        inputs: vec![],
    }];
    let mut expected: f64 = -0.25;
    for index in 1..count - 1 {
        let (operation, block) = operation(kind, index);
        expected = match operation {
            "abs" => expected.abs(),
            "bias" => expected + 0.001,
            "saturation" => expected.clamp(-0.2, 0.2),
            _ => expected * 0.999,
        };
        nodes.push(Node {
            name: format!("Block {index}"),
            block: serde_json::from_value(block).map_err(|e| e.to_string())?,
            inputs: vec![index - 1],
        });
    }
    nodes.push(Node {
        name: "Output".into(),
        block: Block::Scope,
        inputs: vec![count - 2],
    });
    Ok((
        Model {
            nodes,
            steps,
            ..Model::default()
        },
        expected,
    ))
}

fn main() -> Result<(), String> {
    let legacy_only = std::env::args().any(|arg| arg == "--legacy-only");
    let workers = rayon::current_num_threads();
    let cancel = AtomicBool::new(false);
    let kinds: &[&str] = if legacy_only {
        &["legacy_gain"]
    } else {
        &["legacy_gain", "abs", "bias", "saturation", "mixed"]
    };
    let mut cases = Vec::new();
    for nodes in [3, 64, 512] {
        for steps in [1_000, 100_000] {
            for kind in kinds {
                let (model, expected) = fixture(kind, nodes, steps)?;
                cases.push((*kind, model, expected));
            }
        }
    }
    for (_, model, expected) in &cases {
        for _ in 0..2 {
            let result = model.run(&cancel)?;
            assert!(
                result.traces[0]
                    .values
                    .data()
                    .iter()
                    .all(|v| v.to_bits() == expected.to_bits())
            );
        }
    }
    println!("workers,kind,nodes,steps,repetition,milliseconds");
    for repetition in 0..11 {
        for offset in 0..cases.len() {
            let index = if repetition % 2 == 0 {
                (offset + repetition) % cases.len()
            } else {
                (cases.len() - 1 - offset + repetition) % cases.len()
            };
            let (kind, model, expected) = &cases[index];
            let started = Instant::now();
            let result = black_box(model.run(&cancel)?);
            let elapsed = started.elapsed().as_secs_f64() * 1000.;
            assert_eq!(
                result.traces[0].values.data()[0].to_bits(),
                expected.to_bits()
            );
            assert_eq!(
                result.traces[0].values.data()[model.steps - 1].to_bits(),
                expected.to_bits()
            );
            drop(result);
            println!(
                "{workers},{kind},{},{},{repetition},{elapsed:.6}",
                model.nodes.len(),
                model.steps
            );
        }
    }
    Ok(())
}
