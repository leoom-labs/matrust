//! Same-build character editor costs; no parallel kernel or old/new claim.
use matrust::{
    Array, CellArray, CharacterRow, Engine, Scalar,
    variable_edit::{self, Rect, Target},
};
use std::{
    collections::BTreeMap,
    hint::black_box,
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};

fn setup(rows: usize, cols: usize, nested: bool) -> (Engine, Target, Array) {
    let stop = AtomicBool::new(false);
    let leaf: Array = CharacterRow::from_column_major(
        rows,
        cols,
        (0..rows * cols).map(|i| i as u16).collect(),
        &stop,
    )
    .unwrap()
    .into();
    let shared = Arc::new(leaf.clone());
    let root: Array = if nested {
        CellArray::from_shared(1, 2, vec![shared.clone(), shared], &stop)
            .unwrap()
            .into()
    } else {
        leaf.clone()
    };
    let mut engine = Engine::default();
    engine
        .merge_workspace(BTreeMap::from([
            ("A".into(), root.clone()),
            ("Alias".into(), root.clone()),
        ]))
        .unwrap();
    let target = Target::new("A".into(), root, if nested { vec![0] } else { vec![] }).unwrap();
    (engine, target, leaf)
}
fn main() {
    if cfg!(debug_assertions) {
        panic!("Run this harness in release mode");
    }
    let reverse = std::env::args().skip(1).any(|a| a == "--reverse");
    let mut cases = Vec::new();
    for (rows, cols) in [(1, 1), (32, 32), (64, 64), (256, 256), (1, 65536)] {
        for nested in [false, true] {
            for workers in [1, 8] {
                for operation in ["copy_units", "paste_units", "prepared_units_control"] {
                    cases.push((rows, cols, nested, workers, operation));
                }
            }
        }
    }
    if reverse {
        cases.reverse();
    }
    println!("rows\tcols\tnested\tworkers\toperation\tsample\tnanoseconds");
    for (rows, cols, nested, workers, operation) in cases {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        let rect = Rect::new(0, 0, rows, cols).unwrap();
        let text = (0..rows)
            .map(|_| (0..cols).map(|_| "\\uD800").collect::<Vec<_>>().join("\t"))
            .collect::<Vec<_>>()
            .join("\n");
        let prepared = vec![Scalar::Real(55296.); rows * cols];
        for sample in 0..8 {
            let (mut engine, target, leaf) = setup(rows, cols, nested);
            let expected = variable_edit::copy_tsv(&engine, &target, rect).unwrap();
            // Includes pool entry. Setup, expected serialization, destruction and
            // correctness assertions stay outside the measured interval.
            let started = Instant::now();
            let (copied, committed) = pool.install(|| match operation {
                "copy_units" => (
                    Some(variable_edit::copy_tsv(&engine, &target, rect).unwrap()),
                    None,
                ),
                "paste_units" => (
                    None,
                    Some(variable_edit::paste_tsv(&mut engine, &target, rect, &text).unwrap()),
                ),
                _ => (
                    None,
                    Some(
                        variable_edit::apply_values(&mut engine, &target, rect, &prepared).unwrap(),
                    ),
                ),
            });
            let ns = started.elapsed().as_nanos();
            black_box(&committed);
            if let Some(copied) = copied {
                assert_eq!(copied, expected);
            } else {
                let result = engine.array("A").unwrap();
                let result = if let Array::Cell(cell) = &result {
                    cell.get_linear(0).unwrap().as_ref()
                } else {
                    &result
                };
                let Array::Character(value) = result else {
                    panic!()
                };
                assert_eq!((value.rows(), value.cols()), (rows, cols));
                assert!(value.units().iter().all(|u| *u == 0xd800));
            }
            assert!(
                engine
                    .array("Alias")
                    .unwrap()
                    .same_snapshot(target.expected_root())
            );
            if nested {
                let Array::Cell(cell) = engine.array("A").unwrap() else {
                    panic!()
                };
                assert!(cell.get_linear(1).unwrap().same_snapshot(&leaf));
            }
            if sample > 0 {
                println!("{rows}\t{cols}\t{nested}\t{workers}\t{operation}\t{sample}\t{ns}");
            }
        }
    }
}
