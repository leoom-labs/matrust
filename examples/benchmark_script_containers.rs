//! Complete statement costs, including parse, bookkeeping and pool entry.
//! All operations are serial bounded metadata work; 8 is configuration only.
use matrust::{Array, CellArray, CharacterRow, Engine, Matrix};
use std::{collections::BTreeMap, hint::black_box, sync::atomic::AtomicBool, time::Instant};
fn main() {
    if cfg!(debug_assertions) {
        panic!("Use --release");
    }
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    assert!(arguments.is_empty() || arguments == ["--reverse"]);
    let reverse = !arguments.is_empty();
    let mut shapes = vec![(1, 1), (1, 8193), (64, 1024)];
    let mut workers = vec![1, 8];
    let mut jobs = vec![
        "cell_construct",
        "cell_shared_assign_clear",
        "cell_contents",
        "cell_gather",
        "cell_replace",
        "character_assign_clear",
        "numeric_assign_clear",
    ];
    if reverse {
        shapes.reverse();
        workers.reverse();
        jobs.reverse();
    }
    println!("workers,rows,cols,operation,sample,microseconds");
    for workers in workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        for &(rows, cols) in &shapes {
            let cancel = AtomicBool::new(false);
            // One 1-MiB numeric payload; copied into cells by shared ownership.
            let payload: Array = Matrix::filled(1, 131072, 3.).unwrap().into();
            let source: Array = CellArray::empty(rows, cols, &cancel)
                .unwrap()
                .replaced(0, payload.clone(), &cancel)
                .unwrap()
                .into();
            let text: Array = CharacterRow::from_text(&"x".repeat(rows * cols), &cancel)
                .unwrap()
                .into();
            for &operation in &jobs {
                let script = match operation {
                    "cell_construct" => format!("C=cell({rows},{cols});"),
                    "cell_shared_assign_clear" => "B=C;clear B;".into(),
                    "cell_contents" => "R=C{1};".into(),
                    "cell_gather" => "G=C(1:2:end);".into(),
                    "cell_replace" => "C{end}=A;".into(),
                    "character_assign_clear" => "B=S;clear B;".into(),
                    _ => "B=A;clear B;".into(),
                };
                let mut e = Engine::default();
                e.merge_workspace(BTreeMap::from([
                    ("A".into(), payload.clone()),
                    ("S".into(), text.clone()),
                ]))
                .unwrap();
                for iteration in 0..9 {
                    for name in ["B", "R", "G"] {
                        e.remove_array(name);
                    }
                    e.merge_workspace(BTreeMap::from([("C".into(), source.clone())]))
                        .unwrap();
                    let start = Instant::now();
                    let result = pool.install(|| e.execute(black_box(&script)));
                    let elapsed = start.elapsed();
                    result.unwrap();
                    assert!(payload.same_snapshot(&e.array("A").unwrap()));
                    assert!(text.same_snapshot(&e.array("S").unwrap()));
                    match operation {
                        "cell_construct" => {
                            let Array::Cell(c) = e.array("C").unwrap() else {
                                panic!()
                            };
                            assert_eq!((c.rows(), c.cols()), (rows, cols));
                            assert!(c.get_linear(0).unwrap().is_empty());
                        }
                        "cell_contents" => assert!(payload.same_snapshot(&e.array("R").unwrap())),
                        "cell_gather" => {
                            let Array::Cell(c) = e.array("G").unwrap() else {
                                panic!()
                            };
                            assert_eq!(c.len(), (rows * cols).div_ceil(2));
                            assert!(payload.same_snapshot(c.get_linear(0).unwrap()));
                        }
                        "cell_replace" => {
                            let Array::Cell(c) = e.array("C").unwrap() else {
                                panic!()
                            };
                            assert!(payload.same_snapshot(c.get_linear(c.len() - 1).unwrap()));
                        }
                        _ => {
                            assert!(!e.contains_array("B"));
                            assert!(source.same_snapshot(&e.array("C").unwrap()));
                        }
                    }
                    if iteration >= 2 {
                        println!(
                            "{workers},{rows},{cols},{operation},{},{:.6}",
                            iteration - 2,
                            elapsed.as_secs_f64() * 1e6
                        );
                    }
                }
            }
        }
    }
}
