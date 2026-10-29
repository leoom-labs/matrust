//! Measure the exact desktop undo implementation, without GUI/frame/startup costs.
#[path = "../src/desktop/model_topology.rs"]
mod topology;
use matrust::simulation::{Block, Model, Node};
use std::{hint::black_box, time::Instant};

fn main() -> Result<(), String> {
    let workers = rayon::current_num_threads();
    println!(
        "workers,case,nodes,operation,cycles,repetition,retained_bytes,milliseconds_per_cycle"
    );
    for case in ["default", "dense", "large_filter"] {
        let mut model = Model::default();
        let index = if case == "dense" {
            model.nodes = (0..512)
                .map(|i| Node {
                    name: format!("Node {i}"),
                    block: if i == 0 {
                        Block::Constant { value: 1. }
                    } else {
                        Block::Sum
                    },
                    inputs: if i == 0 { vec![] } else { vec![0; 32] },
                })
                .collect();
            0
        } else {
            if case == "large_filter" {
                model.nodes[3].block = Block::Filter {
                    b: vec![0.; 4097],
                    a: vec![1.; 4097],
                };
            }
            3
        };
        model.set_layout(
            (0..model.nodes.len())
                .map(|i| [i as f32 * 160., 24.])
                .collect(),
        )?;
        let original = model.clone();
        let nodes = model.nodes.len();
        let mut probe = topology::Change::insert(
            &mut model,
            Node {
                name: "Probe".into(),
                block: Block::Gain { gain: 1. },
                inputs: vec![usize::MAX],
            },
        );
        // A full dense model intentionally rejects insertion at its block cap.
        if nodes == 512 {
            assert!(probe.is_err());
        } else {
            assert!(probe.as_mut().unwrap().apply(&mut model, false));
        }
        drop(probe);
        assert!(model.same_storage(&original));
        for operation in ["delete_undo", "undo_redo"] {
            for repetition in 0..12 {
                let cycles = 100;
                let mut pending = if operation == "undo_redo" {
                    Some(topology::Change::delete(&mut model, index)?)
                } else {
                    None
                };
                let mut retained_bytes = 0;
                let start = Instant::now();
                for _ in 0..cycles {
                    if let Some(change) = &mut pending {
                        retained_bytes = change.retained_bytes();
                        assert!(black_box(change.apply(&mut model, false)));
                        assert!(black_box(change.apply(&mut model, true)));
                    } else {
                        let mut change = topology::Change::delete(&mut model, index)?;
                        retained_bytes = change.retained_bytes();
                        assert!(black_box(change.apply(&mut model, false)));
                    }
                }
                let ms = start.elapsed().as_secs_f64() * 1000. / cycles as f64;
                if let Some(mut change) = pending {
                    assert!(change.apply(&mut model, false));
                }
                assert!(model.same_storage(&original));
                if repetition > 0 {
                    println!(
                        "{workers},{case},{nodes},{operation},{cycles},{repetition},{retained_bytes},{ms:.6}"
                    );
                }
            }
        }
    }
    Ok(())
}
