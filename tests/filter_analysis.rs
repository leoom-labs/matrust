use matrust::{
    Engine, Matrix,
    dsp::{self, Analysis, Band},
};
use std::{
    f64::consts::{PI, TAU},
    sync::atomic::AtomicBool,
};

fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 2e-10 * (1. + b.abs()), "{a} != {b}");
}

#[test]
fn analytic_delay_matches_linear_phase_and_first_order_closed_form() {
    let cancel = AtomicBool::new(false);
    let f: Vec<_> = (0..512).map(|i| i as f64 / 512.).collect();
    let b = dsp::fir1(64, &[0.2], Band::Lowpass).unwrap();
    for (i, delay) in dsp::group_delay(&b, &[1.], &f, &cancel)
        .unwrap()
        .into_iter()
        .enumerate()
    {
        if f[i] < 0.18 {
            close(delay, 32.);
        } else {
            assert!((delay - 32.).abs() < 1e-6, "Stopband delay: {delay}");
        }
    }
    let gd = dsp::group_delay(&[1.], &[1., -0.5], &f, &cancel).unwrap();
    for (&delay, &frequency) in gd.iter().zip(&f) {
        let cosine = (PI * frequency).cos();
        close(delay, (0.5 * cosine - 0.25) / (1.25 - cosine));
    }
    let scaled = dsp::group_delay(&[1e200], &[1e200, -0.5e200], &f, &cancel).unwrap();
    assert_eq!(gd, scaled);
    let singular = dsp::group_delay(&[1., -1.], &[1.], &[0., 0.5], &cancel).unwrap();
    assert!(singular[0].is_nan());
    close(singular[1], 0.5);
}

#[test]
fn phase_unwrap_and_gap_semantics() {
    let response: Vec<_> = (0..32)
        .map(|i| {
            let angle = -i as f64 * TAU / 8.;
            [angle.cos(), angle.sin()]
        })
        .collect();
    for (i, value) in dsp::phase_response(&response).into_iter().enumerate() {
        close(value, -(i as f64) * TAU / 8.);
    }
    let values = dsp::phase_response(&[[1., 0.], [0., 0.], [0., 1.], [f64::NAN, 0.], [1., 0.]]);
    assert!(values[1].is_nan() && values[3].is_nan());
    close(values[2], PI / 2.);
    close(values[4], 0.);
}

#[test]
fn impulse_and_step_match_known_recurrence_and_sos_batch() {
    let cancel = AtomicBool::new(false);
    for kind in [Analysis::Impulse, Analysis::Step] {
        let actual = dsp::analyze(kind, &[1.], &[1., -0.5], 64, &cancel).unwrap();
        for (i, &value) in actual.iter().enumerate() {
            close(
                value,
                if kind == Analysis::Impulse {
                    0.5f64.powi(i as i32)
                } else {
                    2. - 0.5f64.powi(i as i32)
                },
            );
        }
        let sos = dsp::butter_sos(8, 0.2, Band::Lowpass).unwrap();
        let mut input = vec![if kind == Analysis::Step { 1. } else { 0. }; 256];
        input[0] = 1.;
        let x = Matrix::from_column_vec(256, 1, input).unwrap();
        assert_eq!(
            dsp::analyze_sos(kind, &sos, 256, &cancel).unwrap(),
            dsp::sosfilt(&sos, &x, &cancel).unwrap().data()
        );
    }
    assert!(dsp::analyze(Analysis::Impulse, &[1.], &[1., -1e100], 256, &cancel).is_err());
}

#[test]
fn analysis_bounds_cancellation_and_parallel_results() {
    let cancel = AtomicBool::new(false);
    assert!(dsp::analyze(Analysis::Impulse, &[1.], &[1.], 0, &cancel).is_err());
    assert!(dsp::analyze(Analysis::Impulse, &[1.], &[1.], 65537, &cancel).is_err());
    assert!(dsp::group_delay(&[1.], &[1.], &[f64::NAN], &cancel).is_err());
    let b = dsp::fir1(256, &[0.2], Band::Lowpass).unwrap();
    let run = |threads| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| dsp::analyze(Analysis::GroupDelay, &b, &[1.], 8192, &cancel).unwrap())
    };
    assert_eq!(run(1), run(4));
    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    for kind in Analysis::ALL {
        assert!(dsp::analyze(kind, &[1.], &[1.], 512, &cancel).is_err());
    }
}

#[test]
fn script_analysis_returns_values_or_plots_without_replacing_ans() {
    let mut engine = Engine::default();
    engine.execute("ans=123; h=impz(1,[1 -0.5],16); s=stepz(1,[1 -0.5],16); p=phasez([0 0 1],1,64); gd=grpdelay([0 0 1],1,64);").unwrap();
    assert!(engine.figure.is_none());
    assert_eq!(engine.variables["h"].rows(), 16);
    for value in engine.variables["gd"].data() {
        close(*value, 2.);
    }
    engine.execute("grpdelay([0 0 1],1,64);").unwrap();
    assert_eq!(engine.variables["ans"].data(), &[123.]);
    assert_eq!(
        engine.figure.as_ref().unwrap().ylabel,
        "Group delay (samples)"
    );
    engine.execute("impz(1,[1 -0.5],8);").unwrap();
    assert_eq!(
        engine.figure.as_ref().unwrap().series[0]
            .points
            .get(7)
            .unwrap()[0],
        7.
    );
    engine
        .execute("q=max([0 0.2 1],0.1); r=min(q,0.5);")
        .unwrap();
    assert_eq!(engine.variables["r"].data(), &[0.1, 0.2, 0.5]);
    assert!(engine.execute("h=impz(1,[1 -0.5]);").is_err());
    engine.execute("sos=butter_sos(6,0.2); h=sosimpz(sos,256); g=sosgrpdelay(sos,512); p=sosphasez(sos,512); s=sosstepz(sos,256); m=sosfreqzmag(sos,512);").unwrap();
    close(engine.variables["m"].data()[0], 1.);
}
