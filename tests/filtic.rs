//! Analytic/history integration checks; no executed MATLAB reference is claimed.
use matrust::{Engine, Matrix, dsp};
use std::sync::atomic::{AtomicBool, Ordering};

fn row(values: &[f64]) -> Matrix {
    Matrix::from_column_slice(1, values.len(), values).unwrap()
}

#[test]
fn analytic_states_history_order_padding_orientation_and_irrelevant_tails() {
    let cancel = AtomicBool::new(false);
    for (b, a, y, x, expected) in [
        (
            vec![1., 2., 3.],
            vec![1.],
            vec![],
            Some(vec![4., 5.]),
            vec![23., 12.],
        ),
        (vec![1.], vec![1., -0.5], vec![2.], None, vec![1.]),
        (
            vec![1., 2., 3.],
            vec![1., -0.5],
            vec![5.],
            Some(vec![3., 4.]),
            vec![20.5, 9.],
        ),
        (
            vec![1., 2., 3.],
            vec![1.],
            vec![],
            Some(vec![4.]),
            vec![8., 12.],
        ),
        (
            vec![1.],
            vec![1., -0.5, 0.25],
            vec![2.],
            None,
            vec![1., -0.5],
        ),
        (vec![1., 2., 3.], vec![1., -0.5], vec![], None, vec![0., 0.]),
    ] {
        let state = dsp::filtic(&b, &a, &y, x.as_deref(), &cancel).unwrap();
        assert_eq!((state.rows(), state.cols()), (expected.len(), 1));
        assert_eq!(state.data(), expected);
        let mut engine = Engine::default();
        for column in [false, true] {
            for (name, values) in [
                ("b", b.as_slice()),
                ("a", &a),
                ("y", &y),
                ("x", x.as_deref().unwrap_or(&[])),
            ] {
                let matrix = row(values);
                engine.variables.insert(
                    name.into(),
                    if column { matrix.transpose() } else { matrix },
                );
            }
            engine.execute("z=filtic(b,a,y,x);").unwrap();
            assert_eq!(engine.variables["z"].data(), expected);
            assert_eq!(
                (engine.variables["z"].rows(), engine.variables["z"].cols()),
                (expected.len(), 1)
            );
        }
    }
    assert_eq!(
        dsp::filtic(
            &[1., 2., 3.],
            &[1., -0.5],
            &[5., f64::NAN],
            Some(&[3., 4., f64::INFINITY]),
            &cancel
        )
        .unwrap()
        .data(),
        &[20.5, 9.]
    );
    let z = dsp::filtic(&[3.], &[1.], &[f64::NAN], Some(&[f64::INFINITY]), &cancel).unwrap();
    assert_eq!((z.rows(), z.cols(), z.len()), (0, 1, 0));
    for shape in [(0, 0), (1, 0), (0, 1)] {
        let mut engine = Engine::default();
        engine.variables.insert(
            "empty".into(),
            Matrix::from_column_vec(shape.0, shape.1, vec![]).unwrap(),
        );
        engine
            .execute("z=filtic([1 2],[1 -0.5],empty,empty); q=filtic(3,1,empty);")
            .unwrap();
        assert_eq!(engine.variables["z"].data(), &[0.]);
        assert_eq!(
            (engine.variables["q"].rows(), engine.variables["q"].cols()),
            (0, 1)
        );
    }
}

fn recurrence(b: &[f64], a: &[f64], input: &[f64]) -> Vec<f64> {
    // Independent direct difference equation over chronological samples, not
    // a delay-state implementation or a call to filter/filtic.
    let mut output = Vec::with_capacity(input.len());
    for i in 0..input.len() {
        let mut value = 0.;
        for (j, coefficient) in b.iter().take(i + 1).enumerate() {
            value += coefficient * input[i - j];
        }
        for (j, coefficient) in a.iter().take(i + 1).enumerate().skip(1) {
            value -= coefficient * output[i - j];
        }
        output.push(value);
    }
    output
}

fn near(a: &[f64], b: &[f64]) {
    assert_eq!(a.len(), b.len());
    for (&a, &b) in a.iter().zip(b) {
        assert!((a - b).abs() <= 2e-13, "{a} versus {b}");
    }
}

#[test]
fn independent_recurrence_histories_resume_existing_filter_and_match_final_state() {
    let cancel = AtomicBool::new(false);
    let input: Vec<_> = (0..48).map(|i| (i % 9) as f64 / 8. - 0.5).collect();
    for (b, a) in [
        (vec![0.5, 0.25, 0.125, 0.0625], vec![1., -0.5, 0.125]),
        (vec![0.5, -0.125], vec![1., -0.5, 0.125, -0.0625]),
        (vec![1., 0.5, 0.25], vec![1.]),
    ] {
        let expected = recurrence(&b, &a, &input);
        for split in [1, 3, 17] {
            let x_history: Vec<_> = input[..split].iter().rev().copied().collect();
            let y_history: Vec<_> = expected[..split].iter().rev().copied().collect();
            let zi = dsp::filtic(&b, &a, &y_history, Some(&x_history), &cancel).unwrap();
            let (_, zf) =
                dsp::filter_with_state(&b, &a, &row(&input[..split]), None, None, &cancel).unwrap();
            near(zi.data(), zf.data());
            let (continued, _) =
                dsp::filter_with_state(&b, &a, &row(&input[split..]), Some(&zi), None, &cancel)
                    .unwrap();
            near(continued.data(), &expected[split..]);
            let (column, _) = dsp::filter_with_state(
                &b,
                &a,
                &row(&input[split..]).transpose(),
                Some(&zi),
                None,
                &cancel,
            )
            .unwrap();
            near(column.data(), continued.data());
        }
    }
}

#[test]
fn strict_types_shapes_arity_and_failures_preserve_workspace_and_aliases() {
    let mut engine = Engine::default();
    engine.execute("b=[1 2 3]; a=[1 -0.5]; x=[3 4]; y=5; bx=b; xx=x; args={b,a,y,x}; z=filtic(args{:}); out=29; h=figure(5);").unwrap();
    assert_eq!(engine.variables["z"].data(), &[20.5, 9.]);
    let before = engine.workspace();
    for call in [
        "filtic(b,a)",
        "filtic(b,a,y,x,1)",
        "filtic([],a,y)",
        "filtic(b,[],y)",
        "filtic([1 2;3 4],a,y)",
        "filtic(b,[1 2;3 4],y)",
        "filtic(b,a,[1 2;3 4])",
        "filtic(b,a,y,[1 2;3 4])",
        "filtic(b,a,zeros(0,2))",
        "filtic(b,a,y,zeros(2,0))",
        "filtic(b,2,y,x)",
        "filtic(b,-1,y,x)",
        "filtic(b,0,y,x)",
        "filtic([1 Inf],1,y,x)",
        "filtic(b,[1 NaN],y,x)",
        "filtic(b,a,NaN,x)",
        "filtic(b,a,y,[Inf 4])",
        "filtic(logical(b),a,y,x)",
        "filtic(b,logical(a),y,x)",
        "filtic(b,a,logical(y),x)",
        "filtic(b,a,y,logical(x))",
        "filtic(complex(b,0),a,y,x)",
        "filtic(b,a,complex(y,0),x)",
        "filtic('abc',a,y,x)",
        "filtic(b,a,y,char('a'))",
        "filtic(b,a,{1},x)",
        "filtic(b,a,y,\"text\")",
        "filtic(b,a,h,x)",
    ] {
        assert!(engine.execute(&format!("out={call};")).is_err(), "{call}");
        for (name, value) in &before {
            assert!(
                engine.array(name).unwrap().same_snapshot(value),
                "{name}: {call}"
            );
        }
    }
    assert!(
        engine
            .array("b")
            .unwrap()
            .same_snapshot(&engine.array("bx").unwrap())
    );
    assert!(
        engine
            .array("x")
            .unwrap()
            .same_snapshot(&engine.array("xx").unwrap())
    );
    let cancel = engine.cancellation();
    cancel.store(true, Ordering::Relaxed);
    assert!(
        engine
            .execute("out=filtic(b,a,y,x);")
            .unwrap_err()
            .contains("cancelled")
    );
    assert!(engine.array("out").unwrap().same_snapshot(&before["out"]));
    cancel.store(false, Ordering::Relaxed);
    engine.execute("out=filtic(b,a,y,x);").unwrap();
    assert_eq!(engine.variables["out"].data(), &[20.5, 9.]);
}

#[test]
fn bounded_order_overflow_and_one_eight_serial_contexts() {
    let cancel = AtomicBool::new(false);
    let excess = vec![1.; 4098];
    assert!(
        dsp::filtic(&excess, &[1.], &[], None, &cancel)
            .unwrap_err()
            .contains("4096")
    );
    assert!(dsp::filtic(&[1.], &excess, &[], None, &cancel).is_err());
    assert!(dsp::filtic(&[], &[1.], &[], None, &cancel).is_err());
    assert!(dsp::filtic(&[1.], &[], &[], None, &cancel).is_err());
    assert!(
        dsp::filtic(&[1., f64::MAX], &[1.], &[], Some(&[2.]), &cancel)
            .unwrap_err()
            .contains("overflowed")
    );
    for a0 in [0., -0., -1., 2., f64::from_bits(1), f64::NAN, f64::INFINITY] {
        assert!(
            dsp::filtic(&[1.], &[a0], &[], None, &cancel)
                .unwrap_err()
                .contains("a(1)=1")
        );
    }
    let b = Matrix::from_column_vec(1, 4097, vec![1.; 4097]).unwrap();
    let x = Matrix::from_column_vec(1, 4096, vec![1.; 4096]).unwrap();
    let aliases = (b.clone(), x.clone());
    for workers in [1, 8] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
            .install(|| {
                assert_eq!(rayon::current_num_threads(), workers);
                let z = dsp::filtic(b.data(), &[1.], &[], Some(x.data()), &cancel).unwrap();
                assert_eq!((z.rows(), z.cols()), (4096, 1));
                for (k, value) in z.data().iter().enumerate() {
                    assert_eq!(*value, (4096 - k) as f64);
                }
            });
    }
    assert_eq!(b.data().as_ptr(), aliases.0.data().as_ptr());
    assert_eq!(x.data().as_ptr(), aliases.1.data().as_ptr());
}

#[test]
fn user_function_and_index_precedence_multiple_output_preflight_and_completion() {
    let mut local = Engine::default();
    local
        .execute("[a,b]=filtic(7); function [a,b]=filtic(x); a=x;b=8;end")
        .unwrap();
    assert_eq!(local.variables["a"].data(), &[7.]);
    assert_eq!(local.variables["b"].data(), &[8.]);
    let mut indexed = Engine::default();
    indexed.execute("filtic=[11 13]; z=filtic(2);").unwrap();
    assert_eq!(indexed.variables["z"].data(), &[13.]);
    let mut engine = Engine::default();
    assert!(engine.execute("[z,q]=filtic(figure(9));").is_err());
    assert!(engine.figure_ids().is_empty());
    assert!(matrust::session::FUNCTIONS.contains(&"filtic"));
}
