//! Local serial transaction observations in one/eight configured worker contexts.
use matrust::{
    Array, CellArray, Complex64, ComplexMatrix, Engine, LogicalMatrix, Matrix, Scalar,
    variable_edit::{self, Rect, Target},
};
use std::{
    collections::BTreeMap,
    hint::black_box,
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};

fn numeric(kind: usize, rows: usize, cols: usize) -> Array {
    let stop = AtomicBool::new(false);
    match kind {
        0 => Matrix::from_column_vec(
            rows,
            cols,
            (0..rows * cols).map(|i| (i % 1000) as f64).collect(),
        )
        .unwrap()
        .into(),
        1 => LogicalMatrix::from_column_vec(
            rows,
            cols,
            (0..rows * cols)
                .map(|i| u8::from(i.is_multiple_of(2)))
                .collect(),
            &stop,
        )
        .unwrap()
        .into(),
        _ => ComplexMatrix::from_column_vec(
            rows,
            cols,
            (0..rows * cols)
                .map(|i| Complex64::new((i % 1000) as f64, -((i % 97) as f64)))
                .collect(),
            &stop,
        )
        .unwrap()
        .into(),
    }
}
fn nested(leaf: &Array) -> Array {
    let stop = AtomicBool::new(false);
    let shared = Arc::new(leaf.clone());
    let inner: Array = CellArray::from_shared(1, 2, vec![shared.clone(), shared.clone()], &stop)
        .unwrap()
        .into();
    CellArray::from_shared(1, 2, vec![Arc::new(inner), shared], &stop)
        .unwrap()
        .into()
}
fn leaf(root: &Array, nested: bool) -> &Array {
    if !nested {
        return root;
    }
    let Array::Cell(root) = root else { panic!() };
    let Array::Cell(inner) = root.get_linear(0).unwrap().as_ref() else {
        panic!()
    };
    inner.get_linear(0).unwrap()
}
fn bits(a: Scalar, b: Scalar) {
    let (a, b) = (a.into_complex(), b.into_complex());
    assert_eq!(
        (a.re.to_bits(), a.im.to_bits()),
        (b.re.to_bits(), b.im.to_bits())
    );
}
fn main() {
    if cfg!(debug_assertions) {
        panic!("Use --release");
    }
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    assert!(args.is_empty() || (args.len() == 1 && args[0] == "reverse"));
    let reverse = !args.is_empty();
    let mut workers = vec![1, 8];
    let mut shapes = vec![(1, 1), (16, 16), (64, 64), (1024, 16), (1024, 1024)];
    let mut kinds = vec![0, 1, 2];
    let mut depths = vec![false, true];
    let mut ops = vec!["literal", "paste_rect", "typed_rect", "copy_rect"];
    if reverse {
        workers.reverse();
        shapes.reverse();
        kinds.reverse();
        depths.reverse();
        ops.reverse();
    }
    println!(
        "workers,rows,cols,kind,depth,operation,patch_rows,patch_cols,repetition,elapsed_ns,root_bytes,leaf_bytes,text_bytes"
    );
    for workers in workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        for &(rows, cols) in &shapes {
            for &kind in &kinds {
                let original = numeric(kind, rows, cols);
                let pr = rows.min(64);
                let pc = cols.min(64);
                let rectangle = Rect::new(0, 0, pr, pc).unwrap();
                let values = (0..pr * pc)
                    .map(|i| match kind {
                        0 => Scalar::Real(31. + i as f64),
                        1 => Scalar::Logical(i.is_multiple_of(2)),
                        _ => Scalar::Complex(Complex64::new(31. + i as f64, -7.)),
                    })
                    .collect::<Vec<_>>();
                let paste = values
                    .chunks(pc)
                    .map(|row| {
                        row.iter()
                            .map(|&v| variable_edit::scalar_literal(v))
                            .collect::<Vec<_>>()
                            .join("\t")
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                for &deep in &depths {
                    let root = if deep {
                        nested(&original)
                    } else {
                        original.clone()
                    };
                    let path = if deep { vec![0, 0] } else { vec![] };
                    let mut template = Engine::default();
                    template
                        .merge_workspace(BTreeMap::from([
                            ("A".into(), root.clone()),
                            ("Alias".into(), root.clone()),
                        ]))
                        .unwrap();
                    let copied = variable_edit::copy_tsv(
                        &template,
                        &Target::new("A".into(), root.clone(), path.clone()).unwrap(),
                        rectangle,
                    )
                    .unwrap();
                    for &op in &ops {
                        for iteration in 0..9 {
                            let mut engine = Engine::default();
                            engine.merge_workspace(template.workspace()).unwrap();
                            let target =
                                Target::new("A".into(), root.clone(), path.clone()).unwrap();
                            // Pool entry is included. Input construction/validation and Engine/copied-text destruction are excluded.
                            // Small Committed result metadata drops inside the timed operation.
                            let start = Instant::now();
                            let output = pool.install(|| match op {
                                "literal" => {
                                    variable_edit::apply_literal(&mut engine, &target, 0, 0, "17")
                                        .unwrap();
                                    None
                                }
                                "paste_rect" => {
                                    variable_edit::paste_tsv(
                                        &mut engine,
                                        &target,
                                        rectangle,
                                        &paste,
                                    )
                                    .unwrap();
                                    None
                                }
                                "typed_rect" => {
                                    variable_edit::apply_values(
                                        &mut engine,
                                        &target,
                                        rectangle,
                                        &values,
                                    )
                                    .unwrap();
                                    None
                                }
                                "copy_rect" => Some(
                                    variable_edit::copy_tsv(&engine, &target, rectangle).unwrap(),
                                ),
                                _ => unreachable!(),
                            });
                            let elapsed = start.elapsed().as_nanos();
                            let updated = engine.array("A").unwrap();
                            let result = leaf(&updated, deep);
                            assert!(engine.array("Alias").unwrap().same_snapshot(&root));
                            assert_eq!(engine.workspace_bytes(), template.workspace_bytes());
                            if op == "copy_rect" {
                                assert_eq!(output.as_deref(), Some(copied.as_str()));
                                assert!(updated.same_snapshot(&root));
                                let text = output.as_ref().unwrap();
                                assert_eq!(text.lines().count(), pr);
                                for (row, line) in text.lines().enumerate() {
                                    assert_eq!(line.split('\t').count(), pc);
                                    for (col, field) in line.split('\t').enumerate() {
                                        bits(
                                            Scalar::parse_cell(field, kind == 2, kind == 1)
                                                .unwrap(),
                                            original.scalar(row, col).unwrap(),
                                        );
                                    }
                                }
                            } else {
                                assert!(!updated.same_snapshot(&root));
                                for col in 0..cols {
                                    for row in 0..rows {
                                        let expected = if op == "literal" && row == 0 && col == 0 {
                                            match kind {
                                                0 => Scalar::Real(17.),
                                                1 => Scalar::Logical(true),
                                                _ => Scalar::Complex(Complex64::new(17., 0.)),
                                            }
                                        } else if op != "literal" && row < pr && col < pc {
                                            values[row * pc + col]
                                        } else {
                                            original.scalar(row, col).unwrap()
                                        };
                                        bits(result.scalar(row, col).unwrap(), expected);
                                    }
                                }
                                if deep {
                                    let Array::Cell(changed) = &updated else {
                                        panic!()
                                    };
                                    let Array::Cell(old) = &root else { panic!() };
                                    assert!(
                                        changed
                                            .get_linear(1)
                                            .unwrap()
                                            .same_snapshot(old.get_linear(1).unwrap())
                                    );
                                    let Array::Cell(inner) =
                                        changed.get_linear(0).unwrap().as_ref()
                                    else {
                                        panic!()
                                    };
                                    assert!(inner.get_linear(1).unwrap().same_snapshot(&original));
                                }
                            }
                            if iteration >= 2 {
                                let (a, b) = if op == "literal" { (1, 1) } else { (pr, pc) };
                                let text_bytes = match op {
                                    "literal" => 2,
                                    "paste_rect" => paste.len(),
                                    "copy_rect" => copied.len(),
                                    _ => 0,
                                };
                                println!(
                                    "{workers},{rows},{cols},{kind},{},{op},{a},{b},{},{elapsed},{},{},{text_bytes}",
                                    if deep { 2 } else { 0 },
                                    iteration - 2,
                                    root.bytes(),
                                    original.bytes()
                                );
                            }
                            black_box(output);
                            black_box(engine);
                        }
                    }
                }
            }
        }
    }
}
