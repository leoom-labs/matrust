//! Independent small references; shaped-empty and zero-bit policies are MatRust
//! choices pending an authentic MATLAB reference. No vendor parity is inferred.
use matrust::{Array, Engine, LogicalMatrix, Matrix};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
};

const NAMES: [&str; 4] = ["union", "intersect", "setdiff", "setxor"];
fn array(values: &[f64], rows: usize, cols: usize, logical: bool) -> Array {
    if logical {
        LogicalMatrix::from_column_vec(
            rows,
            cols,
            values.iter().map(|&v| u8::from(v != 0.)).collect(),
            &AtomicBool::new(false),
        )
        .unwrap()
        .into()
    } else {
        Matrix::from_column_slice(rows, cols, values)
            .unwrap()
            .into()
    }
}
fn values(a: &Array) -> Vec<f64> {
    match a {
        Array::Numeric(a) => a.data().to_vec(),
        Array::Logical(a) => a.data().iter().map(|&v| f64::from(v)).collect(),
        _ => panic!("real/logical"),
    }
}
fn reference(a: &[f64], b: &[f64], name: &str, stable: bool) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    // Independent tiny O(N^2) search, no production grouping/sort comparator.
    let mut selected: Vec<(f64, Option<usize>, Option<usize>)> = Vec::new();
    for (i, &v) in a.iter().enumerate() {
        if a[..i].contains(&v) {
            continue;
        }
        let j = b.iter().position(|&x| x == v);
        let keep = match name {
            "union" => true,
            "intersect" => j.is_some(),
            _ => j.is_none(),
        };
        if keep {
            selected.push((v, Some(i), if name == "intersect" { j } else { None }));
        }
    }
    if matches!(name, "union" | "setxor") {
        for (j, &v) in b.iter().enumerate() {
            if b[..j].contains(&v) || a.contains(&v) {
                continue;
            }
            selected.push((v, None, Some(j)));
        }
    }
    if !stable {
        // Tiny stable insertion sort. NaNs compare last but never deduplicate.
        for i in 1..selected.len() {
            let mut j = i;
            while j > 0
                && ((selected[j - 1].0.is_nan() && !selected[j].0.is_nan())
                    || selected[j - 1].0 > selected[j].0)
            {
                selected.swap(j, j - 1);
                j -= 1;
            }
        }
    }
    (
        selected.iter().map(|r| r.0).collect(),
        selected
            .iter()
            .filter_map(|r| r.1.map(|i| i as f64 + 1.))
            .collect(),
        selected
            .iter()
            .filter_map(|r| r.2.map(|i| i as f64 + 1.))
            .collect(),
    )
}
fn check(a: Array, b: Array, name: &str, stable: bool) {
    let mut engine = Engine::default();
    engine
        .merge_workspace(BTreeMap::from([
            ("A".into(), a.clone()),
            ("alias".into(), a.clone()),
            ("B".into(), b.clone()),
        ]))
        .unwrap();
    let outputs = if name == "setdiff" {
        "[C,IA]"
    } else {
        "[C,IA,IB]"
    };
    let order = if stable { "stable" } else { "sorted" };
    engine
        .execute(&format!("{outputs}={name}(A,B,'{order}');"))
        .unwrap();
    let (c, ia, ib) = reference(&values(&a), &values(&b), name, stable);
    let actual = engine.array("C").unwrap();
    assert_eq!(
        values(&actual)
            .iter()
            .map(|x| x.to_bits())
            .collect::<Vec<_>>(),
        c.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
        "{name}/{order}"
    );
    let row = a.rows() == 1 && (name == "setdiff" || b.rows() == 1);
    assert_eq!(
        (actual.rows(), actual.cols()),
        if row { (1, c.len()) } else { (c.len(), 1) }
    );
    let logical =
        matches!(a, Array::Logical(_)) && (name == "setdiff" || matches!(b, Array::Logical(_)));
    assert_eq!(matches!(actual, Array::Logical(_)), logical);
    assert_eq!(engine.variables["IA"].data(), ia);
    assert_eq!(
        (engine.variables["IA"].rows(), engine.variables["IA"].cols()),
        (ia.len(), 1)
    );
    if name != "setdiff" {
        assert_eq!(engine.variables["IB"].data(), ib);
        assert_eq!(
            (engine.variables["IB"].rows(), engine.variables["IB"].cols()),
            (ib.len(), 1)
        );
    }
    for (key, before) in [("A", &a), ("alias", &a), ("B", &b)] {
        assert!(engine.array(key).unwrap().same_snapshot(before));
    }
}

#[test]
fn exhaustive_small_real_vectors_match_independent_first_occurrence_reference() {
    let mut cases = vec![vec![]];
    for a in [-1., 0., 1.] {
        cases.push(vec![a]);
        for b in [-1., 0., 1.] {
            cases.push(vec![a, b]);
        }
    }
    for a in &cases {
        for b in &cases {
            for name in NAMES {
                for stable in [false, true] {
                    check(
                        array(a, 1, a.len(), false),
                        array(b, b.len(), 1, false),
                        name,
                        stable,
                    );
                }
            }
        }
    }
}

#[test]
fn mixed_class_directionality_shapes_empties_and_contribution_indices_in_both_pools() {
    for workers in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            assert_eq!(rayon::current_num_threads(), workers);
            for al in [false, true] {
                for bl in [false, true] {
                    for (ar, ac) in [
                        (1, 4),
                        (4, 1),
                        (1, 1),
                        (0, 0),
                        (1, 0),
                        (0, 1),
                        (0, 3),
                        (3, 0),
                    ] {
                        for (br, bc) in [
                            (1, 4),
                            (4, 1),
                            (1, 1),
                            (0, 0),
                            (1, 0),
                            (0, 1),
                            (0, 3),
                            (3, 0),
                        ] {
                            for name in NAMES {
                                for stable in [false, true] {
                                    check(
                                        array(&[1., 0., 1., 0.][..ar * ac], ar, ac, al),
                                        array(&[0., 1., 0., 1.][..br * bc], br, bc, bl),
                                        name,
                                        stable,
                                    );
                                }
                            }
                        }
                    }
                }
            }
            // Both sides contribute nontrivial first positions; B shares one.
            for name in NAMES {
                for stable in [false, true] {
                    check(
                        array(&[7., 3., 7., 1., 3.], 1, 5, false),
                        array(&[4., 3., 2., 4.], 1, 4, false),
                        name,
                        stable,
                    );
                    check(
                        array(&[0., 1., 0.], 1, 3, true),
                        array(&[2., -0., 3.], 1, 3, false),
                        name,
                        stable,
                    );
                    check(
                        array(&[2., -0., 3.], 1, 3, false),
                        array(&[0., 1., 0.], 1, 3, true),
                        name,
                        stable,
                    );
                }
            }
        });
    }
}

#[test]
fn distinct_nan_payloads_first_zero_bits_and_infinities_are_not_sort_equivalence() {
    let n1 = f64::from_bits(0x7ff8_0000_0000_0001);
    let n2 = f64::from_bits(0xfff8_0000_0000_0051);
    for zero in [0_f64, -0.] {
        let a = array(
            &[n1, zero, -zero, f64::INFINITY, n1, f64::NEG_INFINITY, 1.],
            1,
            7,
            false,
        );
        let b = array(&[-zero, n2, f64::INFINITY, n1, 2.], 1, 5, false);
        for name in NAMES {
            for stable in [false, true] {
                check(a.clone(), b.clone(), name, stable);
            }
        }
    }
}

#[test]
fn masks_csl_stored_option_and_default_order_preserve_contributions() {
    for name in NAMES {
        let max = if name == "setdiff" { 2 } else { 3 };
        let mut e = Engine::default();
        e.execute(
            "A=[3 1 3 7];B=[1 4 4];args={A,B};empty=cell(0,1);ord=char([115 116 97 98 108 101]);",
        )
        .unwrap();
        e.execute(&format!(
            "C={name}(args{{:}});D={name}(A,B,empty{{:}});E={name}(A,B,'sorted');F={name}(A,B,ord);"
        ))
        .unwrap();
        assert_eq!(e.variables["C"].data(), e.variables["D"].data());
        assert_eq!(e.variables["C"].data(), e.variables["E"].data());
        for mask in 0..(1usize << max) {
            let targets: Vec<_> = (0..max)
                .map(|i| {
                    if mask & (1 << i) == 0 {
                        "~".to_string()
                    } else {
                        format!("out{i}")
                    }
                })
                .collect();
            e.execute(&format!("[{}]={name}(A,B,'stable');", targets.join(",")))
                .unwrap();
            let (c, ia, ib) = reference(&[3., 1., 3., 7.], &[1., 4., 4.], name, true);
            for (i, expected) in [c, ia, ib].iter().enumerate().take(max) {
                if mask & (1 << i) != 0 {
                    assert_eq!(e.variables[&format!("out{i}")].data(), expected);
                }
            }
        }
    }
}

#[test]
fn invalid_forms_output_ceilings_cancellation_and_precedence_retain_existing_semantics() {
    for name in NAMES {
        let mut e = Engine::default();
        e.execute("C=17;A=[3 1 3];B=[1 4];alias=A;").unwrap();
        let before = e.workspace();
        for args in [
            "",
            "A",
            "A,B,'stable','extra'",
            "A,B,'rows'",
            "A,B,'legacy'",
            "A,B,1",
            "A,B,\"stable\"",
            "ones(2),B",
            "complex(A,0),B",
            "A,complex(zeros(0,3))",
            "'abc',B",
            "A,{1}",
            "A,char([1 2])",
        ] {
            assert!(
                e.execute(&format!("C={name}({args});")).is_err(),
                "{name}({args})"
            );
            for (key, value) in &before {
                assert!(e.array(key).unwrap().same_snapshot(value));
            }
        }
        let too_many = if name == "setdiff" {
            "[C,x,y]"
        } else {
            "[C,x,y,z]"
        };
        assert!(
            e.execute(&format!("{too_many}={name}(figure(9),B);"))
                .unwrap_err()
                .contains("does not support")
        );
        assert!(e.figure_ids().is_empty());
        assert!(
            e.execute(&format!("C={name}(figure(10),B,'sorted','extra');"))
                .is_err()
        );
        assert_eq!(e.figure_ids(), vec![10]);
        let cancel = e.cancellation();
        cancel.store(true, Ordering::Relaxed);
        assert!(
            e.execute(&format!("C={name}(A,B);"))
                .unwrap_err()
                .contains("cancelled")
        );
        assert!(e.array("C").unwrap().same_snapshot(&before["C"]));
        cancel.store(false, Ordering::Relaxed);
        e.execute(&format!("C={name}(A,B);")).unwrap();
        let mut local = Engine::default();
        local
            .execute(&format!(
                "[a,b,c,d]={name}(8);function [w,x,y,z]={name}(v);w=v;x=2;y=3;z=4;end"
            ))
            .unwrap();
        assert_eq!(local.variables["a"].data(), &[8.]);
        assert_eq!(local.variables["d"].data(), &[4.]);
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(format!("{name}.m")),
            format!("function y={name}(x,z)\ny=x+z;\nend\n"),
        )
        .unwrap();
        let mut file = Engine::default();
        file.set_function_directory(dir.path()).unwrap();
        file.execute(&format!("answer={name}(7,8);")).unwrap();
        assert_eq!(file.variables["answer"].data(), &[15.]);
        let mut indexed = Engine::default();
        indexed
            .execute(&format!("{name}=[11 19];answer={name}(2);"))
            .unwrap();
        assert_eq!(indexed.variables["answer"].data(), &[19.]);
        assert!(matrust::session::FUNCTIONS.contains(&name));
    }
    let help = Engine::default().execute("help").unwrap().output.join("\n");
    assert!(
        help.contains("union/intersect/setdiff/setxor")
            && help.contains("vectors and shaped empties")
    );
}
