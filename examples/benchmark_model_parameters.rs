//! Exact desktop edit helpers; excludes UI, parsing, startup and fixture setup.
#[allow(dead_code)]
#[path = "../src/desktop/model_parameters.rs"]
mod parameters;
#[allow(dead_code)]
#[path = "../src/desktop/model_topology.rs"]
mod topology;
use matrust::simulation::{Block, Model};
use parameters::{Coefficients, Edit, Properties};
use std::{hint::black_box, time::Instant};

fn main() -> Result<(), String> {
    let workers = rayon::current_num_threads();
    println!("workers,coefficients,operation,cycles,repetition,milliseconds_per_cycle");
    for coefficients in [41, 4097] {
        let mut model = Model::default();
        model.nodes[3].block = Block::Filter {
            b: vec![0.25; coefficients],
            a: vec![1.],
        };
        let original = model.clone();
        let before = Properties::capture(&model.nodes[3]);
        model.nodes[3].name = "Renamed filter".into();
        let edit = Edit::node(&model, 3, before).unwrap();
        assert!(edit.apply(&mut model, false));
        let mut coefficient_edit = Coefficients::replace(
            &mut model,
            3,
            Block::Filter {
                b: vec![0.5; coefficients],
                a: vec![1.],
            },
        )?
        .unwrap();
        assert!(coefficient_edit.apply(&mut model));
        for operation in [
            "property_snapshot",
            "property_undo_redo",
            "coefficient_undo_redo",
        ] {
            let cycles = if operation == "coefficient_undo_redo" {
                100
            } else {
                1000
            };
            for repetition in 0..12 {
                let start = Instant::now();
                for _ in 0..cycles {
                    match operation {
                        "property_snapshot" => {
                            let snapshot = black_box(Properties::capture(&model.nodes[3]));
                            assert!(black_box(snapshot.matches(&model.nodes[3])));
                        }
                        "property_undo_redo" => {
                            assert!(black_box(edit.apply(&mut model, true)));
                            assert!(black_box(edit.apply(&mut model, false)));
                        }
                        _ => {
                            assert!(black_box(coefficient_edit.apply(&mut model)));
                            assert!(black_box(coefficient_edit.apply(&mut model)));
                        }
                    }
                }
                let ms = start.elapsed().as_secs_f64() * 1000. / cycles as f64;
                assert!(model.same_storage(&original));
                if repetition > 0 {
                    println!("{workers},{coefficients},{operation},{cycles},{repetition},{ms:.6}");
                }
            }
        }
    }
    Ok(())
}
