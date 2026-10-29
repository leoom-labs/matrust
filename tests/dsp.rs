use matrust::{
    Engine, Matrix,
    dsp::{self, Band, Filter},
    simulation::{Block, Model, Node},
};
use std::sync::atomic::{AtomicBool, Ordering};
fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 2e-11 * (1. + b.abs()), "{a} != {b}");
}
fn row(x: &[f64]) -> Matrix {
    Matrix::from_column_slice(1, x.len(), x).unwrap()
}
#[test]
fn owned_result_storage_preserves_allocation_and_checks_dimensions() {
    let data = vec![1., 2., 3., 4.];
    let ptr = data.as_ptr();
    let m = Matrix::from_column_vec(2, 2, data).unwrap();
    assert_eq!(ptr, m.data().as_ptr());
    assert_eq!(m.get(0, 1), 3.);
    assert!(Matrix::from_column_vec(2, 3, vec![1.]).is_err());
}
#[test]
fn streaming_matches_recurrence_across_arbitrary_chunks() {
    let mut f = Filter::new(&[2., 1.], &[2., -1.]).unwrap();
    let mut samples = [1., 0., 0., 0., 0.];
    let cancel = AtomicBool::new(false);
    f.process(&mut samples[..2], &cancel).unwrap();
    let state = f.state().to_vec();
    let mut resumed = Filter::new(&[2., 1.], &[2., -1.]).unwrap();
    resumed.set_state(&state).unwrap();
    resumed.process(&mut samples[2..], &cancel).unwrap();
    assert_eq!(samples, [1., 1., 0.5, 0.25, 0.125]);
    assert!(Filter::new(&[1.], &[0.]).is_err());
    assert!(Filter::new(&[f64::NAN], &[1.]).is_err());
    cancel.store(true, Ordering::Relaxed);
    assert!(f.process(&mut samples, &cancel).is_err());
}
#[test]
fn fir_symmetry_reference_gain_and_rejection() {
    let cancel = AtomicBool::new(false);
    for (kind, cuts, pass, stop) in [
        (Band::Lowpass, vec![0.2], 0., 0.8),
        (Band::Highpass, vec![0.5], 1., 0.),
        (Band::Bandpass, vec![0.3, 0.6], 0.45, 0.),
        (Band::Bandstop, vec![0.3, 0.6], 0., 0.45),
    ] {
        let b = dsp::fir1(64, &cuts, kind).unwrap();
        for (x, y) in b.iter().zip(b.iter().rev()) {
            close(*x, *y);
        }
        let h = dsp::response(&b, &[1.], &[pass, stop], &cancel).unwrap();
        close(h[0][0].hypot(h[0][1]), 1.);
        assert!(h[1][0].hypot(h[1][1]) < 0.003);
    }
    assert_eq!(dsp::fir1(7, &[0.2], Band::Highpass).unwrap().len(), 9);
    assert!(dsp::fir1(64, &[0.8, 0.2], Band::Bandpass).is_err());
}
#[test]
fn butterworth_cutoff_and_endpoints_for_all_orders() {
    let cancel = AtomicBool::new(false);
    assert!(dsp::butter_sos(2, 1e-300, Band::Lowpass).is_err());
    for n in 1..=20 {
        for band in [Band::Lowpass, Band::Highpass] {
            let sos = dsp::butter_sos(n, 0.2, band).unwrap();
            let mut gains = [1.; 3];
            for r in 0..sos.rows() {
                let c: Vec<_> = (0..6).map(|col| sos.data()[col * sos.rows() + r]).collect();
                let h = dsp::response(&c[..3], &c[3..], &[0., 0.2, 1.], &cancel).unwrap();
                for (gain, v) in gains.iter_mut().zip(h) {
                    *gain *= v[0].hypot(v[1]);
                }
            }
            close(gains[1], std::f64::consts::FRAC_1_SQRT_2);
            close(gains[if band == Band::Lowpass { 0 } else { 2 }], 1.);
            assert!(gains[if band == Band::Lowpass { 2 } else { 0 }] < 1e-12);
        }
    }
}

#[test]
fn sos_simulation_matches_batch_impulse_and_rejects_unstable_output() {
    let sos = dsp::butter_sos(8, 0.2, Band::Lowpass).unwrap();
    let sections = (0..sos.rows())
        .map(|r| std::array::from_fn(|c| sos.data()[c * sos.rows() + r]))
        .collect();
    let mut model = Model {
        version: 1,
        positions: Vec::new(),
        sample_time: 0.01,
        steps: 256,
        nodes: vec![
            Node {
                name: "Impulse".into(),
                block: Block::Step {
                    time: 0.01,
                    before: 1.,
                    after: 0.,
                },
                inputs: vec![],
            },
            Node {
                name: "Filter".into(),
                block: Block::SosFilter { sections },
                inputs: vec![0],
            },
            Node {
                name: "Output".into(),
                block: Block::Scope,
                inputs: vec![1],
            },
        ],
    };
    let cancel = AtomicBool::new(false);
    let mut impulse = vec![0.; 256];
    impulse[0] = 1.;
    let expected = dsp::sosfilt(&sos, &row(&impulse), &cancel).unwrap();
    assert_eq!(
        expected.data(),
        model.run(&cancel).unwrap().traces[0].values.data()
    );
    model.nodes[1].block = Block::Filter {
        b: vec![1.],
        a: vec![1., -1e100],
    };
    assert!(matches!(model.run(&cancel),Err(e) if e.contains("nonfinite")));
}
#[test]
fn filtering_uses_first_nonsingleton_dimension_and_parallel_results_match() {
    let cancel = AtomicBool::new(false);
    let x = Matrix::from_column_slice(3, 2, &[1., 2., 3., 4., 5., 6.]).unwrap();
    assert_eq!(
        dsp::filter(&[1., 1.], &[1.], &x, &cancel).unwrap().data(),
        &[1., 3., 5., 4., 9., 11.]
    );
    assert_eq!(
        dsp::filter(&[1., 1.], &[1.], &row(&[1., 2., 3.]), &cancel)
            .unwrap()
            .data(),
        &[1., 3., 5.]
    );
    let x = Matrix::filled(10000, 8, 1.).unwrap();
    let b = dsp::fir1(40, &[0.2], Band::Lowpass).unwrap();
    let run = |n| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .unwrap()
            .install(|| dsp::filter(&b, &[1.], &x, &cancel).unwrap())
    };
    assert_eq!(run(1).data(), run(4).data());
}
#[test]
fn script_filtering_and_response_work() {
    let mut e = Engine::default();
    e.execute(include_str!("../examples/filter_design.m"))
        .unwrap();
    assert_eq!(e.variables["y"].len(), 2000);
    e.execute("freqz(b,1,512)").unwrap();
    assert_eq!(e.figure.unwrap().series[0].points.len(), 512);
}
#[test]
fn delayed_feedback_and_simultaneous_state_updates() {
    let model = Model {
        version: 1,
        positions: Vec::new(),
        sample_time: 0.01,
        steps: 5,
        nodes: vec![
            Node {
                name: "One".into(),
                block: Block::Constant { value: 1. },
                inputs: vec![],
            },
            Node {
                name: "Add".into(),
                block: Block::Sum,
                inputs: vec![0, 2],
            },
            Node {
                name: "Memory".into(),
                block: Block::UnitDelay { initial: 0. },
                inputs: vec![1],
            },
            Node {
                name: "Scope".into(),
                block: Block::Scope,
                inputs: vec![1],
            },
            Node {
                name: "Second delay".into(),
                block: Block::UnitDelay { initial: 0. },
                inputs: vec![2],
            },
            Node {
                name: "Delayed scope".into(),
                block: Block::Scope,
                inputs: vec![4],
            },
        ],
    };
    let run = model.run(&AtomicBool::new(false)).unwrap();
    assert_eq!(run.traces[0].values.data(), &[1., 2., 3., 4., 5.]);
    assert_eq!(run.traces[1].values.data(), &[0., 0., 1., 2., 3.]);
    let mut bad = model.clone();
    bad.nodes[2].block = Block::Gain { gain: 1. };
    assert!(bad.validate().unwrap_err().contains("Algebraic loop"));
    bad.nodes[2].inputs = vec![999];
    assert!(bad.validate().is_err());
    bad.nodes[2].inputs = vec![usize::MAX];
    assert!(bad.validate().is_err());
}
#[test]
fn simulation_filter_agrees_with_batch_and_model_roundtrips() {
    let m = Model::default();
    let cancel = AtomicBool::new(false);
    let sim = m.run(&cancel).unwrap();
    let x: Vec<_> = sim
        .time
        .data()
        .iter()
        .map(|t| {
            (std::f64::consts::TAU * 10. * t).sin() + 0.3 * (std::f64::consts::TAU * 200. * t).sin()
        })
        .collect();
    let Block::Filter { b, a } = &m.nodes[3].block else {
        panic!()
    };
    let expected = dsp::filter(b, a, &row(&x), &cancel).unwrap();
    for (a, b) in expected.data().iter().zip(sim.traces[0].values.data()) {
        close(*a, *b);
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("filter.mrsim");
    m.save(&path).unwrap();
    let reopened = Model::load(&path).unwrap();
    assert_eq!(
        reopened.run(&cancel).unwrap().traces[0].values.data(),
        sim.traces[0].values.data()
    );
    cancel.store(true, Ordering::Relaxed);
    assert!(m.run(&cancel).is_err());
}
