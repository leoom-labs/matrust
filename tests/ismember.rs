//! Fixed MatRust subset contract; authentic MATLAB reference remains pending.
use matrust::{Array, Engine, LogicalMatrix, Matrix};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
};

fn real(rows: usize, cols: usize, values: &[f64]) -> Array {
    Matrix::from_column_slice(rows, cols, values)
        .unwrap()
        .into()
}
fn logical(rows: usize, cols: usize, values: &[u8]) -> Array {
    LogicalMatrix::from_column_vec(rows, cols, values.to_vec(), &AtomicBool::new(false))
        .unwrap()
        .into()
}
fn value(a: &Array, i: usize) -> f64 {
    match a {
        Array::Numeric(a) => a.data()[i],
        Array::Logical(a) => f64::from(a.data()[i]),
        _ => unreachable!(),
    }
}
fn expected(a: &Array, b: &Array) -> Vec<f64> {
    // Intentionally simple independent reference, only for tiny test inputs.
    (0..a.len())
        .map(|i| {
            (0..b.len())
                .find(|&j| value(a, i) == value(b, j))
                .map_or(0., |j| j as f64 + 1.)
        })
        .collect()
}
fn check_public(a: Array, b: Array) {
    let mut e = Engine::default();
    e.merge_workspace(BTreeMap::from([
        ("A".into(), a.clone()),
        ("alias".into(), a.clone()),
        ("B".into(), b.clone()),
    ]))
    .unwrap();
    e.execute("[L,P]=ismember(A,B);").unwrap();
    let output = e.array("L").unwrap();
    let Array::Logical(lia) = output else {
        panic!("logical membership")
    };
    let loc = &e.variables["P"];
    assert_eq!((lia.rows(), lia.cols()), (a.rows(), a.cols()));
    assert_eq!((loc.rows(), loc.cols()), (a.rows(), a.cols()));
    for ((&lia, &loc), expected) in lia.data().iter().zip(loc.data()).zip(expected(&a, &b)) {
        assert_eq!(lia, u8::from(expected != 0.));
        assert_eq!(loc.to_bits(), expected.to_bits());
    }
    assert!(e.array("A").unwrap().same_snapshot(&a));
    assert!(e.array("alias").unwrap().same_snapshot(&a));
    assert!(e.array("B").unwrap().same_snapshot(&b));
}

#[test]
fn all_class_pairs_shapes_and_empty_shapes_preserve_query_shape() {
    for workers in [1, 8] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
            .install(|| {
                assert_eq!(rayon::current_num_threads(), workers);
                for a_logical in [false, true] {
                    for b_logical in [false, true] {
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
                            let av = [1., 0., 1., 0.];
                            let ab = [1, 0, 1, 0];
                            let a = if a_logical {
                                logical(ar, ac, &ab[..ar * ac])
                            } else {
                                real(ar, ac, &av[..ar * ac])
                            };
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
                                let bv = [2., 1., -0., 1.];
                                let bb = [0, 1, 0, 1];
                                let b = if b_logical {
                                    logical(br, bc, &bb[..br * bc])
                                } else {
                                    real(br, bc, &bv[..br * bc])
                                };
                                check_public(a.clone(), b);
                            }
                        }
                    }
                }
            });
    }
}

#[test]
fn ieee_exact_equality_and_lowest_original_index_do_not_follow_sorted_rank() {
    let nan1 = f64::from_bits(0x7ff8_0000_0000_0001);
    let nan2 = f64::from_bits(0xfff8_0000_0000_0051);
    let tiny = f64::from_bits(1);
    let above = f64::from_bits(1_f64.to_bits() + 1);
    let a = real(
        1,
        12,
        &[
            2.,
            -0.,
            0.,
            nan1,
            nan2,
            f64::INFINITY,
            f64::NEG_INFINITY,
            tiny,
            -tiny,
            1.,
            above,
            f64::MAX,
        ],
    );
    for zero in [0_f64, -0.] {
        let b = real(
            12,
            1,
            &[
                nan2,
                2.,
                f64::INFINITY,
                zero,
                -zero,
                2.,
                f64::NEG_INFINITY,
                tiny,
                -tiny,
                above,
                nan1,
                1.,
            ],
        );
        check_public(a.clone(), b);
    }
    for logical_a in [false, true] {
        let a = if logical_a {
            logical(1, 3, &[0, 1, 0])
        } else {
            real(1, 6, &[-1., -0., 0., 1., above, nan1])
        };
        check_public(a, logical(3, 1, &[1, 0, 1]));
    }
    check_public(
        logical(1, 2, &[0, 1]),
        real(1, 5, &[nan1, f64::INFINITY, -0., 0., 1.]),
    );
}

#[test]
fn ignored_outputs_csl_and_empty_argument_expansion_keep_types_and_values() {
    let mut e = Engine::default();
    e.execute("A=[3 1 3 7];B=[1 3 3];alias=A;args={A,B};empty=cell(0,1);[L,P]=ismember(args{:});[~,Q]=ismember(A,B);[R,~]=ismember(A,B);S=ismember(A,B,empty{:});[~,~]=ismember(A,B);[~]=ismember(A,B);").unwrap();
    assert_eq!(e.variables["P"].data(), &[2., 1., 2., 0.]);
    assert_eq!(e.variables["P"].data(), e.variables["Q"].data());
    for name in ["L", "R", "S"] {
        let Array::Logical(a) = e.array(name).unwrap() else {
            panic!("logical")
        };
        assert_eq!(a.data(), &[1, 1, 1, 0]);
    }
    assert!(
        e.array("A")
            .unwrap()
            .same_snapshot(&e.array("alias").unwrap())
    );
}

#[test]
fn invalid_forms_cancel_and_output_count_preserve_destinations_without_rolling_back_arguments() {
    let mut e = Engine::default();
    e.execute("L=17;P=19;third=23;A=[3 1 3];B=[1 3];alias=A;")
        .unwrap();
    let before = e.workspace();
    for call in [
        "ismember()",
        "ismember(A)",
        "ismember(A,B,'rows')",
        "ismember(A,B,'legacy')",
        "ismember(A,B,'sorted')",
        "ismember(A,B,1)",
        "ismember([1 2;3 4],B)",
        "ismember(A,ones(2))",
        "ismember(complex(A,0),B)",
        "ismember(A,complex(B,0))",
        "ismember(complex(zeros(0,3)),B)",
        "ismember('abc',B)",
        "ismember(A,char([1 2]))",
        "ismember(A,\"abc\")",
        "ismember({1},B)",
        "ismember(A,cell(0,3))",
    ] {
        assert!(e.execute(&format!("[L,P]={call};")).is_err(), "{call}");
        for (name, a) in &before {
            assert!(e.array(name).unwrap().same_snapshot(a), "{name}: {call}");
        }
    }
    assert!(
        e.execute("[L,P,third]=ismember(figure(9),B);")
            .unwrap_err()
            .contains("does not support 3")
    );
    assert!(e.figure_ids().is_empty());
    assert!(
        e.execute("L=ismember(figure(10),B,'extra');")
            .unwrap_err()
            .contains("expects")
    );
    assert_eq!(e.figure_ids(), vec![10]);
    assert!(e.array("L").unwrap().same_snapshot(&before["L"]));
    e.execute("g=gcf;").unwrap();
    assert!(e.execute("L=ismember(g,B);").is_err());
    assert!(e.execute("L=ismember(disp(123),B);").is_err());
    assert!(
        e.failed_output
            .output
            .iter()
            .any(|line| line.contains("123"))
    );
    let stop = e.cancellation();
    stop.store(true, Ordering::Relaxed);
    assert!(
        e.execute("[L,P]=ismember(A,B);")
            .unwrap_err()
            .contains("cancelled")
    );
    assert!(e.array("L").unwrap().same_snapshot(&before["L"]));
    stop.store(false, Ordering::Relaxed);
    e.execute("[L,P]=ismember(A,B);").unwrap();
    assert_eq!(e.variables["P"].data(), &[2., 1., 2.]);
}

#[test]
fn local_file_function_and_array_index_precedence_help_and_completion() {
    let mut local = Engine::default();
    local
        .execute("[x,y,z]=ismember(4);function [a,b,c]=ismember(v);a=v;b=2;c=3;end")
        .unwrap();
    assert_eq!(local.variables["x"].data(), &[4.]);
    assert_eq!(local.variables["z"].data(), &[3.]);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("ismember.m"),
        "function y=ismember(x,z)\ny=x+z;\nend\n",
    )
    .unwrap();
    let mut file = Engine::default();
    file.set_function_directory(dir.path()).unwrap();
    file.execute("answer=ismember(7,8);").unwrap();
    assert_eq!(file.variables["answer"].data(), &[15.]);
    let mut variable = Engine::default();
    variable
        .execute("ismember=[11 19];answer=ismember(2);")
        .unwrap();
    assert_eq!(variable.variables["answer"].data(), &[19.]);
    assert!(
        variable
            .execute("[x,y]=ismember(1);")
            .unwrap_err()
            .contains("Array indexing")
    );
    assert!(matrust::session::FUNCTIONS.contains(&"ismember"));
    let help = Engine::default().execute("help").unwrap().output.join("\n");
    assert!(help.contains("ismember") && help.contains("first B indices"));
}

#[test]
fn ordered_real_sets_preserve_first_indices_for_zero_ties_nan_suffix_and_aliases() {
    let nan = f64::from_bits(0x7ff8_0000_0000_0061);
    let a = real(
        1,
        8,
        &[nan, -0., 0., 1., 2., f64::NEG_INFINITY, f64::INFINITY, -1.],
    );
    for zero in [0_f64, -0.] {
        check_public(
            a.clone(),
            real(
                9,
                1,
                &[
                    f64::NEG_INFINITY,
                    -1.,
                    zero,
                    -zero,
                    1.,
                    1.,
                    f64::INFINITY,
                    nan,
                    f64::NAN,
                ],
            ),
        );
    }
    check_public(a.clone(), real(1, 3, &[nan, f64::NAN, nan]));
    check_public(a.clone(), real(1, 1, &[-0.]));
    // Final inversion must retain the original last index of -1, not a rank.
    let mut b: Vec<_> = (0..8193).map(|i| i as f64).collect();
    b[8192] = -1.;
    check_public(a, real(8193, 1, &b));
}
