use matrust::{Engine, Matrix, dsp, syntax};
use std::sync::atomic::{AtomicBool, Ordering};
fn row(values: &[f64]) -> Matrix {
    Matrix::from_column_slice(1, values.len(), values).unwrap()
}

#[test]
fn chunks_preserve_final_state_and_initial_buffers_for_all_orientations() {
    let cancel = AtomicBool::new(false);
    let b = [0.2, 0.3, 0.2];
    let a = [2., -0.7, 0.2];
    let mut engine = Engine::default();
    engine.execute("b=[0.2 0.3 0.2]; a=[2 -0.7 0.2]; x=reshape(sin(1:96),32,3); zi=[0.1;0.2]; [y,z]=filter(b,a,x,zi); [first,state]=filter(b,a,x(1:11,:),zi); [last,final]=filter(b,a,x(12:end,:),state); joined=[first;last];").unwrap();
    assert_eq!(
        engine.variables["joined"].data(),
        engine.variables["y"].data()
    );
    assert_eq!(
        engine.variables["final"].data(),
        engine.variables["z"].data()
    );
    assert_eq!(engine.variables["zi"].data(), &[0.1, 0.2]);
    let x = &engine.variables["x"];
    let zi = Matrix::from_column_slice(2, 3, &[0.1, 0.4, 0.2, 0.5, 0.3, 0.6]).unwrap();
    let (column, zcol) = dsp::filter_with_state(&b, &a, x, Some(&zi), Some(1), &cancel).unwrap();
    let (rows, zrow) =
        dsp::filter_with_state(&b, &a, &x.transpose(), Some(&zi), Some(2), &cancel).unwrap();
    assert_eq!(rows.transpose().data(), column.data());
    assert_eq!(zrow.data(), zcol.data());
    let (yr, zr) = dsp::filter_with_state(
        &b,
        &a,
        &row(&[1., 2., 3.]),
        Some(&row(&[0.1, 0.2])),
        None,
        &cancel,
    )
    .unwrap();
    assert_eq!((yr.rows(), yr.cols()), (1, 3));
    assert_eq!((zr.rows(), zr.cols()), (2, 1));
    let (yc, zc) = dsp::filter_with_state(
        &b,
        &a,
        &row(&[1., 2., 3.]).transpose(),
        Some(&row(&[0.1, 0.2])),
        None,
        &cancel,
    )
    .unwrap();
    assert_eq!(yr.data(), yc.data());
    assert_eq!(zr.data(), zc.data());
}

#[test]
fn singletons_empty_inputs_order_zero_and_nonfinite_state() {
    let cancel = AtomicBool::new(false);
    let b = [1., 2., 3.];
    let a = [1.];
    let zi = row(&[4., 5.]);
    let (y, z) =
        dsp::filter_with_state(&b, &a, &row(&[1., 2., 3.]), Some(&zi), Some(1), &cancel).unwrap();
    assert_eq!(y.data(), &[5., 6., 7.]);
    assert_eq!(z.data(), &[7., 3., 9., 6., 11., 9.]);
    for (r, c, dim, channels) in [
        (0, 3, None, 3),
        (1, 0, None, 1),
        (3, 0, None, 0),
        (0, 0, None, 0),
        (0, 3, Some(2), 0),
    ] {
        let empty = Matrix::filled(r, c, 0.).unwrap();
        let (y, z) = dsp::filter_with_state(&b, &a, &empty, Some(&zi), dim, &cancel).unwrap();
        assert_eq!((y.rows(), y.cols()), (r, c));
        assert_eq!((z.rows(), z.cols()), (2, channels));
        for state in z.data().chunks(2) {
            assert_eq!(state, &[4., 5.]);
        }
    }
    let (y, z) =
        dsp::filter_with_state(&[3.], &[2.], &row(&[2., 4.]), None, None, &cancel).unwrap();
    assert_eq!(y.data(), &[3., 6.]);
    assert_eq!((z.rows(), z.cols()), (0, 1));
    let (y, _) = dsp::filter_with_state(
        &b,
        &a,
        &row(&[1.]),
        Some(&row(&[f64::NAN, 0.])),
        None,
        &cancel,
    )
    .unwrap();
    assert!(y.data()[0].is_nan());
    let input = row(&[1., f64::NAN, 3., 4., 5., 6., 7.]);
    let (y, z) = dsp::filter_with_state(&b, &a, &input, None, None, &cancel).unwrap();
    assert_eq!(y.data()[0], 1.);
    assert!(y.data()[1..4].iter().all(|v| v.is_nan()));
    assert_eq!(&y.data()[4..], &[22., 28., 34.]);
    assert_eq!(z.data(), &[32., 21.]);
    let mut filter = dsp::Filter::new(&b, &a).unwrap();
    let mut first = [1., f64::NAN, 3.];
    let mut second = [4., 5., 6., 7.];
    filter.process(&mut first, &cancel).unwrap();
    filter.process(&mut second, &cancel).unwrap();
    assert!(second[0].is_nan());
    assert_eq!(&second[1..], &[22., 28., 34.]);
}

#[test]
fn output_lists_are_atomic_bounded_and_respect_discard_and_display_order() {
    let mut engine = Engine::default();
    let output = engine
        .execute("[rows, cols, extra]=size(zeros(2,3))")
        .unwrap()
        .output;
    assert!(output[0].starts_with("\nrows ="));
    assert!(output[1].starts_with("\ncols ="));
    assert_eq!(engine.variables["extra"].data(), &[1.]);
    engine.execute("[~,state]=filter([1 2],1,[3 4]); [same same]=size(zeros(2,3)); [only]=size(zeros(2,3));").unwrap();
    assert_eq!(engine.variables["state"].data(), &[8.]);
    assert_eq!(engine.variables["same"].data(), &[3.]);
    assert_eq!(engine.variables["only"].data(), &[2., 3.]);
    assert!(!engine.variables.contains_key("ans"));
    let before = engine.variables.clone();
    for script in [
        "[state,missing]=filter([1 2],0,[3 4]);",
        "[state,missing]=filter([1 2],1,[3 4],[1 2]);",
        "[state,missing]=sin(1);",
        "[state,missing]=only(1);",
        "[state,missing]=1+2;",
        "[a,b,c]=filter(1,1,1);",
    ] {
        assert!(engine.execute(script).is_err(), "{script}");
        assert_eq!(engine.variables.len(), before.len());
        for (name, value) in &before {
            assert!(std::ptr::eq(
                value.data().as_ptr(),
                engine.variables[name].data().as_ptr()
            ));
        }
    }
    for script in [
        "[a,,b]=size(1)",
        "[a,]=size(1)",
        "[a;b]=size(1)",
        "[]=size(1)",
        "[a~]=size(1)",
    ] {
        assert!(syntax::parse(script).is_err(), "{script}");
    }
    assert!(syntax::parse(&format!("[{}]=size(1)", vec!["x"; 33].join(","))).is_err());
    engine
        .execute("[empty,~]=filter(ones(1,32),1,zeros(0,1000000));")
        .unwrap();
    assert_eq!(engine.variables["empty"].cols(), 1_000_000);
}

#[test]
fn invalid_dimensions_states_and_resource_limits_fail_before_workspace_commit() {
    let cancel = AtomicBool::new(false);
    let x = row(&[1., 2.]);
    for dim in [0, 3, usize::MAX] {
        assert!(dsp::filter_with_state(&[1., 2.], &[1.], &x, None, Some(dim), &cancel).is_err());
    }
    assert!(dsp::filter_with_state(&[1.], &[1.], &x, Some(&row(&[1.])), None, &cancel).is_err());
    // An empty signal can still request a final-state matrix beyond the bound.
    let empty = Matrix::filled(0, 1_000_000, 0.).unwrap();
    assert!(dsp::filter_with_state(&[1.; 32], &[1.], &empty, None, None, &cancel).is_err());
    assert!(
        dsp::filter(&[1.; 32], &[1.], &empty, &cancel)
            .unwrap()
            .is_empty()
    );
    cancel.store(true, Ordering::Relaxed);
    assert!(
        dsp::filter_with_state(&[1., 2.], &[1.], &x, None, None, &cancel)
            .unwrap_err()
            .contains("cancelled")
    );
    let mut engine = Engine::default();
    engine
        .execute("left=3; right=4; big=ones(8388608,1); alias=big; spare=big;")
        .unwrap();
    assert!(
        engine
            .execute("[left,right]=filter(1,1,ones(1,12582912));")
            .unwrap_err()
            .contains("Workspace exceeds")
    );
    assert_eq!(engine.variables["left"].data(), &[3.]);
    assert_eq!(engine.variables["right"].data(), &[4.]);
}

#[test]
fn parallel_state_results_match_serial_for_independent_channels() {
    let cancel = AtomicBool::new(false);
    let x = Matrix::from_column_vec(
        4096,
        8,
        (0..32768).map(|i| (i as f64 * 0.013).sin()).collect(),
    )
    .unwrap();
    let b = dsp::fir1(64, &[0.2], dsp::Band::Lowpass).unwrap();
    let zi = Matrix::from_column_vec(64, 8, (0..512).map(|i| i as f64 / 10000.).collect()).unwrap();
    let run = |threads, dim| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                dsp::filter_with_state(
                    &b,
                    &[1.],
                    &if dim == 1 { x.clone() } else { x.transpose() },
                    Some(&zi),
                    Some(dim),
                    &cancel,
                )
                .unwrap()
            })
    };
    for dim in [1, 2] {
        let (a, za) = run(1, dim);
        let (b, zb) = run(8, dim);
        assert_eq!(a.data(), b.data());
        assert_eq!(za.data(), zb.data());
    }
}

#[test]
fn saved_mat_workspace_resumes_the_same_filter_in_a_new_engine() {
    let mut first = Engine::default();
    first
        .execute("b=[0.2 0.3 0.2];a=[2 -0.7 0.2];[head,zf]=filter(b,a,[1 2;3 4;5 6]);")
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("filter-checkpoint.mat");
    matrust::files::save_workspace(&path, &first.variables).unwrap();
    let mut resumed = Engine::default();
    resumed.variables = matrust::files::load_workspace(&path).unwrap();
    resumed
        .execute("[tail,zf]=filter(b,a,[7 8;9 10],zf);joined=[head;tail];")
        .unwrap();
    first
        .execute("[whole,final]=filter(b,a,[1 2;3 4;5 6;7 8;9 10]);")
        .unwrap();
    assert_eq!(
        resumed.variables["joined"].data(),
        first.variables["whole"].data()
    );
    assert_eq!(
        resumed.variables["zf"].data(),
        first.variables["final"].data()
    );
}
