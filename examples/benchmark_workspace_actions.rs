//! Release metadata-operation timings; array payloads remain shared.
#[path = "../src/desktop/workspace_edit.rs"]
#[allow(dead_code)]
mod workspace_edit;
use matrust::{Array, Engine, Matrix};
use std::{collections::BTreeMap, time::Instant};
use workspace_edit::{Action, Request};
fn main() {
    let workers = std::env::var("RAYON_NUM_THREADS").unwrap_or_else(|_| "default".into());
    println!("case,workers,variables,elements,repetitions,median_ms,min_ms,max_ms");
    for count in [1, 1000, 10000] {
        // Large alias payload does not scale with the number of metadata entries.
        let elements = if count == 1 {
            1_000_000
        } else if count == 1000 {
            1000
        } else {
            100
        };
        let matrix = Matrix::from_column_vec(elements, 1, vec![1.; elements]).unwrap();
        let baseline: BTreeMap<_, _> = (0..count)
            .map(|i| (format!("v{i}"), matrix.clone()))
            .collect();
        for rename in [true, false] {
            let mut times = Vec::new();
            for rep in 0..14 {
                let mut engine = Engine::default();
                engine.variables = baseline.clone();
                let targets = if rename {
                    BTreeMap::from([("v0".into(), Array::from(matrix.clone()))])
                } else {
                    engine.workspace()
                };
                let request = Request {
                    targets,
                    action: if rename {
                        Action::Rename("renamed".into())
                    } else {
                        Action::Remove { all: true }
                    },
                };
                let start = Instant::now();
                let change = workspace_edit::perform(&mut engine, request).unwrap();
                drop(change);
                let elapsed = start.elapsed().as_secs_f64() * 1000.;
                if rename {
                    assert_eq!(
                        engine.variables["renamed"].data().as_ptr(),
                        matrix.data().as_ptr()
                    );
                } else {
                    assert!(engine.variables.is_empty());
                }
                if rep >= 3 {
                    times.push(elapsed);
                }
            }
            times.sort_by(f64::total_cmp);
            println!(
                "{},{workers},{count},{elements},11,{:.6},{:.6},{:.6}",
                if rename { "rename" } else { "clear" },
                times[5],
                times[0],
                times[10]
            );
        }
    }
}
