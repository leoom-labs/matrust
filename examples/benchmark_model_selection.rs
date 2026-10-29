//! Exact bulk-deletion helper; coefficient buffers move between model/history.
#[allow(dead_code)]
#[path = "../src/desktop/model_group_delete.rs"]
mod group_delete;
#[allow(dead_code)]
#[path = "../src/desktop/model_topology.rs"]
mod topology;
use matrust::simulation::{Block, Model, Node};
use std::{hint::black_box, time::Instant};

fn fixture(count: usize, filters: bool) -> Model {
    let mut model = Model {
        steps: 1,
        nodes: (0..count)
            .map(|index| Node {
                name: format!("Block{index}"),
                block: if filters {
                    Block::Filter {
                        b: vec![0.25; 4097],
                        a: vec![1.],
                    }
                } else {
                    Block::Sum
                },
                inputs: if filters { vec![0] } else { vec![0; 32] },
            })
            .collect(),
        ..Model::default()
    };
    model.nodes[0].block = Block::Constant { value: 1. };
    model.nodes[0].inputs.clear();
    model.nodes[count - 1].block = Block::Scope;
    model.nodes[count - 1].inputs = vec![count - 2];
    model.validate().unwrap();
    model
}
fn pointers(model: &Model) -> Vec<*const f64> {
    model
        .nodes
        .iter()
        .filter_map(|node| match &node.block {
            Block::Filter { b, .. } => Some(b.as_ptr()),
            _ => None,
        })
        .collect()
}
fn main() {
    let workers = rayon::current_num_threads();
    println!("workers,case,operation,cycles,repetition,milliseconds_per_cycle,retained_bytes");
    for (case, count, filters) in [
        ("dense16", 16, false),
        ("dense128", 128, false),
        ("dense512", 512, false),
        ("filters64", 64, true),
    ] {
        let mut model = fixture(count, filters);
        let original = model.clone();
        let addresses = pointers(&model);
        let indices: Vec<_> = (0..count).step_by(2).collect();
        for operation in ["delete_undo", "undo_redo"] {
            let mut change =
                group_delete::Change::delete(&mut model, &indices, 4 * 1024 * 1024).unwrap();
            let deleted = model.clone();
            let retained = change.retained_bytes();
            if operation == "delete_undo" {
                assert!(change.apply(&mut model, false));
            }
            for repetition in 0..12 {
                let start = Instant::now();
                for _ in 0..100 {
                    if operation == "delete_undo" {
                        let mut edit = group_delete::Change::delete(
                            black_box(&mut model),
                            &indices,
                            4 * 1024 * 1024,
                        )
                        .unwrap();
                        assert!(black_box(edit.apply(&mut model, false)));
                    } else {
                        assert!(black_box(change.apply(&mut model, false)));
                        assert!(black_box(change.apply(&mut model, true)));
                    }
                }
                let milliseconds = start.elapsed().as_secs_f64() * 10.;
                assert!(model.same_storage(if operation == "delete_undo" {
                    &original
                } else {
                    &deleted
                }));
                if operation == "delete_undo" {
                    assert_eq!(pointers(&model), addresses);
                }
                if repetition > 0 {
                    println!(
                        "{workers},{case},{operation},100,{repetition},{milliseconds:.9},{retained}"
                    );
                }
            }
            if operation == "undo_redo" {
                assert!(change.apply(&mut model, false));
            }
            assert!(model.same_storage(&original));
            assert_eq!(pointers(&model), addresses);
        }
    }
}
