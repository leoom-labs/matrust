//! Layout metadata overhead, not a numerical speedup or MATLAB comparison.
use matrust::simulation::{Block, Model, Node, Simulation};
use std::{hint::black_box, path::Path, sync::atomic::AtomicBool, time::Instant};

fn same_samples(a: &Simulation, b: &Simulation) {
    assert_eq!(
        bytemuck::cast_slice::<f64, u64>(a.time.data()),
        bytemuck::cast_slice::<f64, u64>(b.time.data())
    );
    assert_eq!(a.traces.len(), b.traces.len());
    for (a, b) in a.traces.iter().zip(&b.traces) {
        assert_eq!(a.name, b.name);
        assert_eq!(
            bytemuck::cast_slice::<f64, u64>(a.values.data()),
            bytemuck::cast_slice::<f64, u64>(b.values.data())
        );
    }
}

fn main() -> Result<(), String> {
    let cancel = AtomicBool::new(false);
    if std::env::args().any(|a| a == "--bench") {
        let workers = rayon::current_num_threads();
        println!("workers,nodes,steps,layout,repetition,milliseconds");
        for (nodes, steps) in [(5, 1000), (5, 100_000), (512, 1000)] {
            let mut automatic = Model {
                steps,
                ..Model::default()
            };
            if nodes == 512 {
                automatic.nodes = (0..nodes)
                    .map(|i| Node {
                        name: format!("Block {i}"),
                        block: if i == 0 {
                            Block::Constant { value: 1. }
                        } else if i == nodes - 1 {
                            Block::Scope
                        } else {
                            Block::Gain { gain: 0.9999 }
                        },
                        inputs: if i == 0 { vec![] } else { vec![i - 1] },
                    })
                    .collect();
            }
            let mut positioned = automatic.clone();
            positioned.set_layout(
                (0..nodes)
                    .map(|i| [24. + (i % 5) as f32 * 170., 24. + (i / 5) as f32 * 100.])
                    .collect(),
            )?;
            // These untimed runs warm both paths and prove identical results.
            same_samples(&automatic.run(&cancel)?, &positioned.run(&cancel)?);
            for repetition in 0..7 {
                let layouts = if repetition % 2 == 0 {
                    [("automatic", &automatic), ("positioned", &positioned)]
                } else {
                    [("positioned", &positioned), ("automatic", &automatic)]
                };
                for (layout, model) in layouts {
                    let start = Instant::now();
                    drop(black_box(model.run(&cancel)?));
                    let ms = start.elapsed().as_secs_f64() * 1000.;
                    println!("{workers},{nodes},{steps},{layout},{repetition},{ms:.6}");
                }
            }
        }
    } else {
        let dir = Path::new("target/evidence/models/layout");
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let mut model = Model::default();
        model.save(&dir.join("legacy.mrsim"))?;
        let legacy = serde_json::to_vec(&model).unwrap();
        let before = model.run(&cancel)?;
        model.set_layout(vec![
            [24., 8.],
            [24., 96.],
            [212., 48.],
            [400., 48.],
            [588., 48.],
        ])?;
        model.save(&dir.join("positioned.mrsim"))?;
        let reopened = Model::load(&dir.join("positioned.mrsim"))?;
        assert!(reopened.same_storage(&model));
        same_samples(&before, &reopened.run(&cancel)?);
        model.set_layout(vec![])?;
        assert_eq!(serde_json::to_vec(&model).unwrap(), legacy);
        println!("Wrote legacy and custom-layout model fixtures; exact simulation bits unchanged");
    }
    Ok(())
}
