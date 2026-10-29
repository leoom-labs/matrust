//! Release-only local I/O observations; configured workers do not imply parallel codec execution.
use matrust::{Array, CellArray, CharacterRow, Matrix, files};
use std::{
    collections::BTreeMap,
    hint::black_box,
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};
fn check(a: &Array, b: &Array) {
    assert_eq!(
        (a.class(), a.rows(), a.cols()),
        (b.class(), b.rows(), b.cols())
    );
    match (a, b) {
        (Array::Character(a), Array::Character(b)) => assert_eq!(a.units(), b.units()),
        (Array::Numeric(a), Array::Numeric(b)) => {
            for (a, b) in a.data().iter().zip(b.data()) {
                assert_eq!(a.to_bits(), b.to_bits());
            }
        }
        (Array::Cell(a), Array::Cell(b)) => {
            for i in 0..a.len() {
                check(a.get_linear(i).unwrap(), b.get_linear(i).unwrap());
            }
        }
        _ => panic!("Unexpected benchmark type"),
    }
}
fn main() {
    if cfg!(debug_assertions) {
        panic!("Build this measurement harness in release mode");
    }
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    assert!(
        args.is_empty() || (args.len() == 1 && args[0] == "reverse"),
        "Only reverse is supported"
    );
    let reverse = !args.is_empty();
    let stop = AtomicBool::new(false);
    let mut cases = Vec::new();
    for n in [16, 4096, 65536] {
        let value: Array =
            CharacterRow::from_units((0..n).map(|i| (i % 65536) as u16).collect(), &stop)
                .unwrap()
                .into();
        cases.push((format!("char_{n}"), value));
    }
    let leaf = Arc::new(Array::Numeric(
        Matrix::from_column_vec(
            4096,
            1,
            (0usize..4096)
                .map(|i| if i.is_multiple_of(2) { -0. } else { i as f64 })
                .collect(),
        )
        .unwrap(),
    ));
    for n in [16, 1024] {
        let value: Array = CellArray::from_shared(1, n, vec![leaf.clone(); n], &stop)
            .unwrap()
            .into();
        cases.push((format!("cell_shared_{n}"), value));
    }
    let text = Arc::new(Array::Character(
        CharacterRow::from_units(vec![0, 0xd800, 955, 0xdc00], &stop).unwrap(),
    ));
    let inner: Array = CellArray::from_shared(2, 2, vec![text; 4], &stop)
        .unwrap()
        .into();
    let inner = Arc::new(inner);
    cases.push((
        "cell_nested_256".into(),
        CellArray::from_shared(1, 256, vec![inner; 256], &stop)
            .unwrap()
            .into(),
    ));
    if reverse {
        cases.reverse();
    }
    println!("case,workers,operation,repetition,elapsed_ns,logical_bytes,file_bytes");
    let workers = if reverse { [8, 1] } else { [1, 8] };
    let ops = if reverse {
        ["read", "save"]
    } else {
        ["save", "read"]
    };
    for worker in workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(worker)
            .build()
            .unwrap();
        for (name, value) in &cases {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("fixture.mat");
            let values = BTreeMap::from([("A".to_owned(), value.clone())]);
            files::save_array_workspace_v73(&path, &values, &stop).unwrap();
            for op in ops {
                for sample in 0..9 {
                    let elapsed = pool.install(|| {
                        let start = Instant::now();
                        let result = if op == "save" {
                            files::save_array_workspace_v73(&path, &values, &stop).unwrap();
                            None
                        } else {
                            let catalog = files::inspect_workspace(&path, &stop).unwrap();
                            Some(
                                files::load_selected_arrays(
                                    &catalog,
                                    &values.keys().cloned().collect(),
                                    &stop,
                                )
                                .unwrap(),
                            )
                        };
                        let elapsed = start.elapsed().as_nanos();
                        (elapsed, result)
                    });
                    let ns = elapsed.0;
                    let retained = elapsed.1;
                    // Validation and returned-array destruction stay outside the interval.
                    if let Some(actual) = &retained {
                        check(value, &actual["A"]);
                    } else {
                        let cat = files::inspect_workspace(&path, &stop).unwrap();
                        let actual = files::load_selected_arrays(
                            &cat,
                            &values.keys().cloned().collect(),
                            &stop,
                        )
                        .unwrap();
                        check(value, &actual["A"]);
                    }
                    if sample >= 2 {
                        println!(
                            "{name},{worker},{op},{},{ns},{},{}",
                            sample - 2,
                            value.bytes(),
                            std::fs::metadata(&path).unwrap().len()
                        );
                    }
                    black_box(retained);
                    assert!(value.same_snapshot(&values["A"]));
                }
            }
        }
    }
}
