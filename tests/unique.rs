//! Hand-authored MatRust contract cases. Genuine MATLAB reference remains pending.
use matrust::{Array, Engine, LogicalMatrix, Matrix, array::UniqueOrder};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
};

fn put(engine: &mut Engine, input: Array) {
    engine
        .merge_workspace(BTreeMap::from([
            ("A".into(), input.clone()),
            ("alias".into(), input),
        ]))
        .unwrap();
}
fn values(array: &Array) -> Vec<f64> {
    match array {
        Array::Numeric(a) => a.data().to_vec(),
        Array::Logical(a) => a.data().iter().map(|&v| f64::from(v)).collect(),
        _ => panic!("real/logical result"),
    }
}
fn verify(engine: &Engine, original: &Array, expected: &[f64], ia: &[f64], ic: &[f64]) {
    let c = engine.array("C").unwrap();
    assert_eq!(c.is_logical(), original.is_logical());
    assert_eq!(
        (c.rows(), c.cols()),
        if original.rows() == 1 {
            (1, expected.len())
        } else {
            (expected.len(), 1)
        }
    );
    let actual = values(&c);
    assert_eq!(actual.len(), expected.len());
    for (got, want) in actual.iter().zip(expected) {
        assert_eq!(got.to_bits(), want.to_bits());
    }
    for (name, want) in [("ia", ia), ("ic", ic)] {
        let a = &engine.variables[name];
        assert_eq!((a.rows(), a.cols()), (want.len(), 1));
        assert_eq!(a.data(), want);
    }
    let source = values(original);
    for (rank, &index) in ia.iter().enumerate() {
        assert_eq!(actual[rank].to_bits(), source[index as usize - 1].to_bits());
    }
    for (at, &rank) in ic.iter().enumerate() {
        let got = actual[rank as usize - 1];
        let want = source[at];
        assert!(
            got == want || (got.is_nan() && want.is_nan()),
            "reconstruction {at}"
        );
        if want.is_nan() {
            assert_eq!(got.to_bits(), want.to_bits());
        }
    }
    assert!(engine.array("A").unwrap().same_snapshot(original));
    assert!(engine.array("alias").unwrap().same_snapshot(original));
}

#[test]
fn public_shapes_classes_first_indices_and_group_reconstruction_one_and_eight() {
    for workers in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            assert_eq!(rayon::current_num_threads(), workers);
            for logical in [false, true] {
                for (rows, cols) in [
                    (0, 0),
                    (1, 0),
                    (0, 1),
                    (0, 3),
                    (3, 0),
                    (1, 1),
                    (1, 4),
                    (4, 1),
                    (2, 2),
                ] {
                    let raw = if rows * cols == 0 {
                        vec![]
                    } else if rows * cols == 1 {
                        vec![1.]
                    } else if logical {
                        vec![1., 0., 1., 0.]
                    } else {
                        vec![3., 1., 3., 2.]
                    };
                    let a: Array = if logical {
                        LogicalMatrix::from_column_vec(
                            rows,
                            cols,
                            raw.iter().map(|&v| v as u8).collect(),
                            &AtomicBool::new(false),
                        )
                        .unwrap()
                        .into()
                    } else {
                        Matrix::from_column_vec(rows, cols, raw).unwrap().into()
                    };
                    for stable in [false, true] {
                        let mut e = Engine::default();
                        put(&mut e, a.clone());
                        e.execute(if stable {
                            "[C,ia,ic]=unique(A,'stable');"
                        } else {
                            "[C,ia,ic]=unique(A);"
                        })
                        .unwrap();
                        let (c, ia, ic) = if a.is_empty() {
                            (vec![], vec![], vec![])
                        } else if a.len() == 1 {
                            (vec![1.], vec![1.], vec![1.])
                        } else if logical && stable {
                            (vec![1., 0.], vec![1., 2.], vec![1., 2., 1., 2.])
                        } else if logical {
                            (vec![0., 1.], vec![2., 1.], vec![2., 1., 2., 1.])
                        } else if stable {
                            (vec![3., 1., 2.], vec![1., 2., 4.], vec![1., 2., 1., 3.])
                        } else {
                            (vec![1., 2., 3.], vec![2., 4., 1.], vec![3., 1., 3., 2.])
                        };
                        verify(&e, &a, &c, &ia, &ic);
                    }
                }
            }
        });
    }
}

#[test]
fn ieee_payloads_first_signed_zero_and_column_major_matrix_are_explicit_contracts() {
    let n1 = f64::from_bits(0x7ff8000000000031);
    let n2 = f64::from_bits(0xfff8000000000042);
    let tiny = f64::from_bits(1);
    let a: Array = Matrix::from_column_vec(
        2,
        5,
        vec![
            n1,
            -0.,
            2.,
            0.,
            n2,
            f64::NEG_INFINITY,
            2.,
            f64::INFINITY,
            tiny,
            -tiny,
        ],
    )
    .unwrap()
    .into();
    for (option, c, ia, ic) in [
        (
            "sorted",
            vec![
                f64::NEG_INFINITY,
                -tiny,
                -0.,
                tiny,
                2.,
                f64::INFINITY,
                n1,
                n2,
            ],
            vec![6., 10., 2., 9., 3., 8., 1., 5.],
            vec![7., 3., 5., 3., 8., 1., 5., 6., 4., 2.],
        ),
        (
            "stable",
            vec![
                n1,
                -0.,
                2.,
                n2,
                f64::NEG_INFINITY,
                f64::INFINITY,
                tiny,
                -tiny,
            ],
            vec![1., 2., 3., 5., 6., 8., 9., 10.],
            vec![1., 2., 3., 2., 4., 5., 3., 6., 7., 8.],
        ),
    ] {
        let mut e = Engine::default();
        put(&mut e, a.clone());
        e.execute(&format!("[C,ia,ic]=unique(A,'{option}');"))
            .unwrap();
        verify(&e, &a, &c, &ia, &ic);
    }
    for zero in [0.0f64, -0.] {
        let a: Array = Matrix::from_column_vec(1, 3, vec![zero, -zero, zero])
            .unwrap()
            .into();
        let mut e = Engine::default();
        put(&mut e, a.clone());
        e.execute("[C,ia,ic]=unique(A);").unwrap();
        verify(&e, &a, &[zero], &[1.], &[1., 1., 1.]);
    }
}

#[test]
fn ignored_outputs_options_csl_and_public_result_class_remain_independent() {
    let mut e = Engine::default();
    e.execute("A=[3 1 3 2];alias=A;args={A,'stable'};empty=cell(0,1);order=char('stable');[C,ia,ic]=unique(args{:});[C2,ia2]=unique(A,order);[~,~,only_ic]=unique(A,'stable');[~,only_ia,~]=unique(A,'stable');[only_C,~,~]=unique(A,'stable');[~,~,~]=unique(A);[~]=unique(A);from_empty=unique(A,empty{:});").unwrap();
    for (first, second) in [
        ("C", "C2"),
        ("C", "only_C"),
        ("ia", "ia2"),
        ("ia", "only_ia"),
        ("ic", "only_ic"),
    ] {
        assert_eq!(e.variables[first].data(), e.variables[second].data());
    }
    assert_eq!(e.variables["from_empty"].data(), &[1., 2., 3.]);
    assert!(
        e.array("A")
            .unwrap()
            .same_snapshot(&e.array("alias").unwrap())
    );
}

#[test]
fn unsupported_forms_preserve_destinations_and_established_argument_effects() {
    let mut e = Engine::default();
    e.execute("C=17;ia=19;ic=23;fourth=29;A=[3 1 3];alias=A;")
        .unwrap();
    let before = e.workspace();
    for call in [
        "unique()",
        "unique(A,'rows')",
        "unique(A,'first')",
        "unique(A,'legacy')",
        "unique(A,'sorted','stable')",
        "unique(A,1)",
        "unique(A,\"stable\")",
        "unique({1})",
    ] {
        assert!(e.execute(&format!("[C,ia,ic]={call};")).is_err(), "{call}");
        for (name, a) in &before {
            assert!(e.array(name).unwrap().same_snapshot(a), "{name}: {call}");
        }
    }
    assert!(
        e.execute("[C,ia,ic,fourth]=unique(figure(9));")
            .unwrap_err()
            .contains("does not support 4")
    );
    assert!(e.figure_ids().is_empty());
    assert!(
        e.execute("C=unique(figure(10),'sorted','extra');")
            .unwrap_err()
            .contains("expects")
    );
    assert_eq!(e.figure_ids(), vec![10]);
    assert!(e.array("C").unwrap().same_snapshot(&before["C"]));
    assert!(e.execute("C=unique(disp(123));").is_err());
    assert!(e.failed_output.output.iter().any(|s| s.contains("123")));
    e.execute("g=gcf;").unwrap();
    assert!(e.execute("C=unique(g);").is_err());
    let stop = e.cancellation();
    stop.store(true, Ordering::Relaxed);
    assert!(
        e.execute("[C,ia,ic]=unique(A);")
            .unwrap_err()
            .contains("cancelled")
    );
    assert!(e.array("C").unwrap().same_snapshot(&before["C"]));
    stop.store(false, Ordering::Relaxed);
    e.execute("[C,ia,ic]=unique(A);").unwrap();
    assert_eq!(e.variables["C"].data(), &[1., 3.]);
}

#[test]
fn user_local_file_functions_array_indexing_help_and_completion_keep_precedence() {
    let mut local = Engine::default();
    local
        .execute("[a,b,c,d]=unique(4);function [w,x,y,z]=unique(v);w=v;x=2;y=3;z=4;end")
        .unwrap();
    assert_eq!(local.variables["a"].data(), &[4.]);
    assert_eq!(local.variables["d"].data(), &[4.]);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("unique.m"),
        "function y=unique(x,z)\ny=x+z;\nend\n",
    )
    .unwrap();
    let mut file = Engine::default();
    file.set_function_directory(dir.path()).unwrap();
    file.execute("answer=unique(7,8);").unwrap();
    assert_eq!(file.variables["answer"].data(), &[15.]);
    let mut variable = Engine::default();
    variable
        .execute("unique=[11 19];answer=unique(2);")
        .unwrap();
    assert_eq!(variable.variables["answer"].data(), &[19.]);
    assert!(
        variable
            .execute("[a,b]=unique(1);")
            .unwrap_err()
            .contains("Array indexing")
    );
    assert!(matrust::session::FUNCTIONS.contains(&"unique"));
    let help = Engine::default().execute("help").unwrap().output.join("\n");
    assert!(help.contains("unique") && help.contains("distinct NaNs"));
}

#[test]
fn merge_boundaries_and_parallel_candidate_match_analytic_first_groups() {
    let pools = [1, 8].map(|workers| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
    });
    for n in [8191usize, 8192, 8193, 524289] {
        for logical in [false, true] {
            let a: Array = if logical {
                LogicalMatrix::from_column_vec(
                    1,
                    n,
                    (0..n).map(|i| u8::from(i % 2 == 0)).collect(),
                    &AtomicBool::new(false),
                )
                .unwrap()
                .into()
            } else {
                Matrix::from_column_vec(1, n, (0..n).map(|i| ((i * 37) % 1024) as f64).collect())
                    .unwrap()
                    .into()
            };
            for stable in [false, true] {
                for (slot, pool) in pools.iter().enumerate() {
                    pool.install(|| {
                        assert_eq!(rayon::current_num_threads(), if slot == 0 { 1 } else { 8 });
                        let mut e = Engine::default();
                        put(&mut e, a.clone());
                        e.execute(if stable {
                            "[C,ia,ic]=unique(A,'stable');"
                        } else {
                            "[C,ia,ic]=unique(A);"
                        })
                        .unwrap();
                        let k = if logical { 2 } else { 1024 };
                        let expected: Vec<_> = (0..k)
                            .map(|i| {
                                if logical {
                                    if stable { (1 - i) as f64 } else { i as f64 }
                                } else if stable {
                                    ((i * 37) % 1024) as f64
                                } else {
                                    i as f64
                                }
                            })
                            .collect();
                        let source = values(&a);
                        let ia: Vec<_> = expected
                            .iter()
                            .map(|v| source[..k].iter().position(|x| x == v).unwrap() as f64 + 1.)
                            .collect();
                        let ic: Vec<_> = (0..n)
                            .map(|i| {
                                if logical {
                                    if stable {
                                        (i % 2 + 1) as f64
                                    } else {
                                        (2 - i % 2) as f64
                                    }
                                } else if stable {
                                    (i % 1024 + 1) as f64
                                } else {
                                    ((i * 37) % 1024 + 1) as f64
                                }
                            })
                            .collect();
                        verify(&e, &a, &expected, &ia, &ic);
                    });
                }
            }
        }
    }
    let false_array: Array = LogicalMatrix::filled(1, 65537, false, &AtomicBool::new(false))
        .unwrap()
        .into();
    let result = false_array
        .unique_values(UniqueOrder::Stable, [true; 3], &AtomicBool::new(false))
        .unwrap();
    assert_eq!(values(&result.values.unwrap()), vec![0.]);
    assert_eq!(result.first_indices.unwrap().data(), &[1.]);
    assert!(
        result
            .group_indices
            .unwrap()
            .data()
            .iter()
            .all(|&v| v == 1.)
    );
}
