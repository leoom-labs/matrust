//! Exact clipboard/history helpers; excludes native clipboard, rendering and simulation.
use eframe::egui::{Pos2, Rect, Vec2};
use matrust::simulation::{Block, Model, Node};
use std::{hint::black_box, time::Instant};
#[allow(dead_code)]
#[path = "../src/desktop/model_clipboard.rs"]
mod clipboard;
#[allow(dead_code)]
#[path = "../src/desktop/model_layout.rs"]
mod layout;
#[allow(dead_code)]
#[path = "../src/desktop/model_topology.rs"]
mod topology;

fn fixture(count: usize, filters: bool) -> Model {
    Model {
        steps: 4,
        nodes: (0..count)
            .map(|index| Node {
                name: format!("Block{index}"),
                inputs: if index == 0 {
                    vec![]
                } else if filters {
                    vec![0]
                } else {
                    vec![0; 32]
                },
                block: if index == 0 {
                    Block::Constant { value: 1. }
                } else if filters {
                    Block::Filter {
                        b: (0..4097).map(|i| (i % 7) as f64 / 10.).collect(),
                        a: vec![1.],
                    }
                } else {
                    Block::Sum
                },
            })
            .collect(),
        ..Model::default()
    }
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
    println!(
        "workers,case,operation,cycles,repetition,milliseconds_per_cycle,text_bytes,retained_bytes"
    );
    for (case, count, filters) in [
        ("dense16", 16, false),
        ("dense256", 256, false),
        ("filters64", 64, true),
    ] {
        let mut model = fixture(count, filters);
        let original = model.clone();
        let indices: Vec<_> = (0..count).collect();
        let addresses = pointers(&model);
        let text = clipboard::Fragment::capture(&model, &indices, 900.)
            .unwrap()
            .encode()
            .unwrap();
        for operation in ["copy_encode", "paste_undo", "undo_redo"] {
            let mut change = clipboard::Change::insert(
                &mut model,
                clipboard::Fragment::decode(&text).unwrap(),
                900.,
                32.,
                indices.clone(),
                4 * 1024 * 1024,
            )
            .unwrap();
            let pasted = model.clone();
            let pasted_addresses = pointers(&model);
            let retained = change.retained_bytes();
            if operation != "undo_redo" {
                assert!(change.apply(&mut model, false));
            }
            for repetition in 0..12 {
                let start = Instant::now();
                for _ in 0..10 {
                    match operation {
                        "copy_encode" => {
                            black_box(
                                clipboard::Fragment::capture(black_box(&model), &indices, 900.)
                                    .unwrap()
                                    .encode()
                                    .unwrap(),
                            );
                        }
                        "paste_undo" => {
                            let fragment = clipboard::Fragment::decode(black_box(&text)).unwrap();
                            let mut edit = clipboard::Change::insert(
                                &mut model,
                                fragment,
                                900.,
                                32.,
                                indices.clone(),
                                4 * 1024 * 1024,
                            )
                            .unwrap();
                            assert!(black_box(edit.apply(&mut model, false)));
                        }
                        _ => {
                            assert!(black_box(change.apply(&mut model, false)));
                            assert!(black_box(change.apply(&mut model, true)));
                        }
                    }
                }
                let milliseconds = start.elapsed().as_secs_f64() * 100.;
                assert!(model.same_storage(if operation == "undo_redo" {
                    &pasted
                } else {
                    &original
                }));
                assert_eq!(
                    pointers(&model),
                    if operation == "undo_redo" {
                        &pasted_addresses
                    } else {
                        &addresses
                    }
                    .clone()
                );
                if repetition > 0 {
                    println!(
                        "{workers},{case},{operation},10,{repetition},{milliseconds:.9},{},{retained}",
                        text.len()
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
