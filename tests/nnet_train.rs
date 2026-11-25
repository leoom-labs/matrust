//! NumPy exact paths: tool/refs/nnet/train.py (published LM and SCG, no MATLAB).
use matrust::engine::nnet::{
    divide::Division,
    network::Network,
    train::{self, TrainOptions},
};
use matrust::{Array, Engine, Matrix};
use std::{
    ops::ControlFlow,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
fn run(e: &mut Engine, s: &str) {
    e.execute(s).unwrap_or_else(|err| panic!("{s}: {err}"));
}
fn check(e: &mut Engine, s: &str) {
    run(e, &format!("assert({s});"));
}
fn values(e: &Engine, name: &str) -> Vec<f64> {
    match e.array(name).unwrap() {
        Array::Numeric(m) => m.data().to_vec(),
        a => panic!("{a:?}"),
    }
}
fn near(actual: &[f64], expected: &[f64]) {
    assert_eq!(actual.len(), expected.len());
    for (a, b) in actual.iter().zip(expected) {
        assert!(
            (a - b).abs() <= 1e-8 * b.abs().max(1e-12),
            "{a:0.17} != {b:0.17}"
        );
    }
}
const FIT: &str = "x=[-1 -0.5 0 0.5 1]; t=[1 0.25 0 0.25 1]; n=fitnet(3); n.inputs{1}.processFcns={}; n.outputs{2}.processFcns={}; n=configure(n,x,t); n=setwb(n,(1:n.numWeightElements)'*0.025-0.25); n.divideFcn='dividetrain'; n.trainParam.epochs=5; n.trainParam.min_grad=0;";
const PATTERN: &str = "x=[-1 -0.7 -0.2 0.3 0.8 1;0.4 -0.2 0.1 -0.4 0.3 0.7]; t=[1 1 1 0 0 0;0 0 0 1 1 1]; n=patternnet(4); n.inputs{1}.processFcns={}; n.outputs{2}.processFcns={}; n=configure(n,x,t); n=setwb(n,sin((1:n.numWeightElements)')*0.4); n.divideFcn='dividetrain'; n.trainParam.epochs=5; n.trainParam.min_grad=0;";

#[test]
fn numpy_lm_five_epochs() {
    let mut e = Engine::default();
    run(&mut e, FIT);
    run(&mut e, "[n,tr]=train(n,x,t); w=getwb(n); perf=tr.perf;");
    // Generated independently by tool/refs/nnet/train.py.
    near(
        &values(&e, "w"),
        &[
            -0.7958569683513889,
            0.05959608853435025,
            0.7756414018638883,
            1.2682208180623236,
            0.6133935056889399,
            0.9146754152515921,
            1.8832057937526843,
            1.763341205782385,
            -0.9718004605455054,
            -1.3528054227290336,
        ],
    );
    near(
        &values(&e, "perf"),
        &[
            0.48781292577778607,
            0.17603119875479867,
            0.17485829794255855,
            0.17098675082174247,
            0.15657579251365708,
            0.026633188060759865,
        ],
    );
    check(&mut e, "tr.num_epochs==5 && isequal(tr.epoch,0:5)");
}

#[test]
fn numpy_scg_fit_five_epochs() {
    let mut e = Engine::default();
    run(&mut e, FIT);
    run(
        &mut e,
        "n.trainFcn='trainscg'; n.trainParam.epochs=5; n.trainParam.min_grad=0;",
    );
    run(&mut e, "[n,tr]=train(n,x,t); w=getwb(n); perf=tr.perf;");
    // Generated independently by tool/refs/nnet/train.py.
    near(
        &values(&e, "w"),
        &[
            -0.23389671365821368,
            -0.19904526442415799,
            -0.16385936793070646,
            -0.03186142871465619,
            -0.041085387188790616,
            -0.04670801243990055,
            0.46252715597880945,
            -0.08266298994545479,
            -0.05949163280030585,
            -0.036403614776900864,
        ],
    );
    near(
        &values(&e, "perf"),
        &[
            0.48781292577778607,
            0.17655306406619184,
            0.17537175392858276,
            0.17526301994773646,
            0.17504033038921568,
            0.17503934094584242,
        ],
    );
    check(&mut e, "tr.num_epochs==5 && isequal(tr.epoch,0:5)");
}

#[test]
fn numpy_scg_pattern_five_epochs() {
    let mut e = Engine::default();
    run(&mut e, PATTERN);
    run(&mut e, "[n,tr]=train(n,x,t); w=getwb(n); perf=tr.perf;");
    // Generated independently by tool/refs/nnet/train.py.
    near(
        &values(&e, "w"),
        &[
            0.30688558455920617,
            0.35138391414116593,
            -0.02226035684544407,
            -0.3249795862004356,
            -0.848494090097581,
            0.002671190902778342,
            0.7062373475044232,
            -0.027660497557839503,
            0.11058452450601176,
            -0.2081930615166127,
            -0.3644977502992268,
            -0.26358537948991917,
            0.4439514272609671,
            0.1203583297476376,
            0.832995912124815,
            -0.6880421027279944,
            -0.11625838493748003,
            -0.5686955105230131,
            -0.27906592127594493,
            0.704194905432177,
            -0.2486420987868006,
            0.5797638304850614,
        ],
    );
    near(
        &values(&e, "perf"),
        &[
            0.3390875190866933,
            0.20653823057695364,
            0.20653823057695364,
            0.20653823057695364,
            0.20653823057695364,
            0.20653823057695364,
        ],
    );
    check(&mut e, "tr.num_epochs==5 && isequal(tr.epoch,0:5)");
}
#[test]
fn all_stop_reasons_and_best_validation_weights() {
    for (param, stop) in [
        ("n.trainParam.epochs=0;", "Maximum epoch reached."),
        ("n.trainParam.goal=1e3;", "Performance goal met."),
        ("n.trainParam.min_grad=1e3;", "Minimum gradient reached."),
        ("n.trainParam.time=0;", "Maximum time elapsed."),
        (
            "n.trainParam.mu=1; n.trainParam.mu_max=0.1;",
            "Maximum MU reached.",
        ),
    ] {
        let mut e = Engine::default();
        run(&mut e, FIT);
        run(&mut e, param);
        run(&mut e, "[n,tr]=train(n,x,t);");
        check(
            &mut e,
            &format!("strcmp(tr.stop,'{stop}') && tr.num_epochs==0"),
        );
    }
    let mut e = Engine::default();
    run(
        &mut e,
        "x=[0 0]; t=[1 -1]; n=fitnet([]); n.inputs{1}.processFcns={}; n.outputs{1}.processFcns={}; n=configure(n,x,t); n=setwb(n,[0;0]); w0=getwb(n); n.divideFcn='divideind'; n.divideParam.trainInd=1; n.divideParam.valInd=2; n.trainParam.max_fail=2; n.trainParam.min_grad=0; [n,tr]=train(n,x,t);",
    );
    check(
        &mut e,
        "strcmp(tr.stop,'Validation stop.') && tr.num_epochs==2 && tr.best_epoch==0 && isequal(getwb(n),w0) && isequal(tr.val_fail,[0 1 2]) && tr.best_vperf==1",
    );
}
#[test]
fn division_rng_rounding_interleaving_and_indices() {
    let mut e = Engine::default();
    run(
        &mut e,
        "rng(0); [a,b,c]=dividerand(100); rng(0); [d,f,g]=dividerand(100);",
    );
    check(
        &mut e,
        "isequal(a,d) && isequal(b,f) && isequal(c,g) && numel(a)==70 && numel(b)==15 && numel(c)==15 && isequal(sort([a b c]),1:100)",
    );
    run(
        &mut e,
        "[a,b,c]=divideblock(10); [d,f,g]=divideint(12,1,1,1);",
    );
    check(
        &mut e,
        "isequal(a,1:7) && isequal(b,8:9) && c==10 && isequal(d,1:3:12) && isequal(f,2:3:12) && isequal(g,3:3:12)",
    );
    run(
        &mut e,
        "[a,b,c]=divideind(5,[1 3 7],[2 8],4); [d,f,g]=dividetrain(5);",
    );
    check(
        &mut e,
        "isequal(a,[1 3]) && b==2 && c==4 && isequal(d,1:5) && isempty(f) && isempty(g)",
    );
    run(&mut e, "[a,b,c]=divideblock(3,1e308,0,0);");
    check(&mut e, "isequal(a,1:3) && isempty(b) && isempty(c)");
    for s in [
        "dividerand(3,-1,1,1)",
        "divideind(3,[1 2],2,3)",
        "divideind(3,0,[],[])",
        "divideblock(3,0,0,0)",
    ] {
        assert!(e.execute(s).is_err(), "{s}");
    }
}
#[test]
fn weighted_sequences_hints_and_none_division() {
    let mut e = Engine::default();
    run(&mut e, FIT);
    run(
        &mut e,
        "a=train(n,x,t); b=train(n,{x(:,1:2),x(:,3:5)},{t(:,1:2),t(:,3:5)},{},{},[1 1 1 1 10],'useParallel','yes','useGPU','no','showResources','yes'); wa=getwb(a); wb=getwb(b);",
    );
    assert!(
        values(&e, "wa")
            .iter()
            .zip(values(&e, "wb"))
            .any(|(a, b)| (a - b).abs() > 1e-3)
    );
    run(
        &mut e,
        "[c,tr]=train(n,{x(:,1:2),x(:,3:5)},{t(:,1:2),t(:,3:5)}); n.divideMode='none'; [d,tr2]=train(n,x,t);",
    );
    check(
        &mut e,
        "isequal(getwb(a),getwb(c)) && isequal(getwb(a),getwb(d)) && isempty(tr2.valInd)",
    );
    run(&mut e, "n.trainFcn='traincgf';");
    let err = e.execute("train(n,x,t)").unwrap_err();
    assert!(
        err.contains("traincgf") && err.contains("not implemented"),
        "{err}"
    );
}
#[test]
fn configure_global_rng_and_sine_convergence() {
    let mut e = Engine::default();
    run(
        &mut e,
        "x=linspace(-pi,pi,81); t=sin(x); rng(1); [n,tr]=train(fitnet(10),x,t); y=n(x); err=mse(t,y);",
    );
    let err = values(&e, "err")[0];
    assert!(err < 1e-4, "mse={err}");
    run(&mut e, "rng(1); [m,tr2]=train(fitnet(10),x,t);");
    check(
        &mut e,
        "isequal(getwb(n),getwb(m)) && isequal(tr.trainInd,tr2.trainInd)",
    );
}
#[test]
fn plot_record_and_confusion_integration_and_help_examples() {
    let mut e = Engine::default();
    run(&mut e, PATTERN);
    run(
        &mut e,
        "[n,tr]=train(n,x,t); plotperform(tr); plottrainstate(tr); plotconfusion(t,n(x));",
    );
    for name in [
        "train",
        "dividerand",
        "divideblock",
        "divideint",
        "divideind",
        "dividetrain",
    ] {
        for line in matrust::help::page(name)
            .unwrap()
            .lines()
            .filter_map(|l| l.strip_prefix("Example:"))
        {
            run(&mut e, line.trim());
        }
    }
}
fn api_problem() -> (Network, Matrix, Matrix) {
    let x = Matrix::from_column_vec(1, 21, (0..21).map(|i| i as f64 * 0.1 - 1.).collect()).unwrap();
    let t = x.map(|v| v.sin());
    let mut net = Network::new(&[10], false, false).unwrap();
    net.configure(&x, &t, &AtomicBool::new(false)).unwrap();
    let w = (0..net.num_weight_elements())
        .map(|i| (i as f64 * 0.17).sin() * 0.4)
        .collect::<Vec<_>>();
    net.setwb(&w).unwrap();
    (net, x, t)
}
#[test]
fn app_progress_stop_and_cancel_are_atomic() {
    let (net, x, t) = api_problem();
    let original = net.getwb();
    let (stopped, tr) = train::train(&net, &x, &t, TrainOptions::default(), &mut |r| {
        if r.epoch == 2 {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    })
    .unwrap();
    assert_eq!(tr.stop, "User stop.");
    assert_eq!(tr.num_epochs, 2);
    assert_ne!(stopped.getwb(), original);
    assert_eq!(net.getwb(), original);
    let cancel = Arc::new(AtomicBool::new(false));
    let opts = TrainOptions {
        cancel: cancel.clone(),
        ..Default::default()
    };
    let result = train::train(&net, &x, &t, opts, &mut |r| {
        if r.epoch == 1 {
            cancel.store(true, Ordering::Relaxed);
        }
        ControlFlow::Continue(())
    });
    assert!(result.is_err());
    assert_eq!(net.getwb(), original);
    let mut e = Engine::default();
    run(&mut e, FIT);
    run(&mut e, "old=getwb(n);");
    e.cancellation().store(true, Ordering::Relaxed);
    assert!(e.execute("[n,tr]=train(n,x,t);").is_err());
    e.cancellation().store(false, Ordering::Relaxed);
    check(&mut e, "isequal(getwb(n),old)");
}
#[test]
fn training_workspace_and_bad_parameters_rejected() {
    let (net, x, t) = api_problem();
    let opts = TrainOptions {
        memory_bytes: 128,
        ..Default::default()
    };
    assert!(
        train::train(&net, &x, &t, opts, &mut |_| ControlFlow::Continue(()))
            .unwrap_err()
            .contains("memory")
    );
    let mut e = Engine::default();
    run(&mut e, FIT);
    for s in [
        "n.trainParam.mu_inc=1; train(n,x,t)",
        "train(n,x,t,{1},{})",
        "train(n,x,t,{},{},-1)",
        "train(n,x,t,'bogus','yes')",
    ] {
        assert!(e.execute(s).is_err(), "{s}");
    }
}
#[test]
fn release_training_benchmark_one_and_four_workers() {
    let (net, x, t) = api_problem();
    for trainer in ["trainlm", "trainscg"] {
        let mut expected: Option<Vec<f64>> = None;
        for workers in [1, 4] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap();
            pool.install(|| {
                let start = Instant::now();
                let mut opts = TrainOptions {train_fcn:trainer.into(), ..Default::default()};
                opts.parameters.insert("epochs".into(), 5.);
                let (result,tr) = train::train(&net,&x,&t,opts,&mut |_|ControlFlow::Continue(())).unwrap();
                let weights = result.getwb();
                if let Some(ref expected) = expected { near(&weights,expected); }
                else { expected = Some(weights); }
                eprintln!("nnet_train release benchmark: trainer={trainer}, workers={workers}, 1-10-1, samples=21, epochs={}, weights={}, memory_cap=256MiB, elapsed={:?}",tr.num_epochs,net.num_weight_elements(),start.elapsed());
            });
        }
    }
}
#[test]
fn processed_cascade_gradient_matches_central_difference() {
    let cancel = Arc::new(AtomicBool::new(false));
    let x = Matrix::from_column_vec(2, 3, vec![1., 5., 2., 5., 4., 5.]).unwrap();
    let t = Matrix::from_column_vec(2, 3, vec![-2., 7., 1., 7., 4., 7.]).unwrap();
    let mut net = Network::new(&[3, 2], false, true).unwrap();
    net.configure(&x, &t, &cancel).unwrap();
    let w = (0..net.num_weight_elements())
        .map(|i| (i as f64 + 1.) * 0.03 - 0.4)
        .collect::<Vec<_>>();
    net.setwb(&w).unwrap();
    let opts = TrainOptions {
        cancel,
        regularization: 0.2,
        normalization: "standard".into(),
        error_weights: Some(Matrix::from_column_vec(1, 3, vec![1., 2., 0.5]).unwrap()),
        division: Division::All,
        ..Default::default()
    };
    let objective = train::Objective::new(&net, &x, &t, &[0, 1, 2], &opts).unwrap();
    let exact = objective.evaluate(&net, false).unwrap().gradient;
    for i in 0..w.len() {
        let mut plus = net.clone();
        let mut minus = net.clone();
        let mut weights = w.clone();
        weights[i] += 1e-6;
        plus.setwb(&weights).unwrap();
        weights[i] -= 2e-6;
        minus.setwb(&weights).unwrap();
        let g = (objective.evaluate(&plus, false).unwrap().perf
            - objective.evaluate(&minus, false).unwrap().perf)
            / 2e-6;
        assert!(
            (exact[i] - g).abs() < 1e-8,
            "weight {i}: {} vs {g}",
            exact[i]
        );
    }
}

#[test]
fn command_line_progress_and_output_budget() {
    let mut e = Engine::default();
    run(&mut e, FIT);
    run(
        &mut e,
        "n.trainParam.showCommandLine=true; n.trainParam.show=2;",
    );
    let out = e.execute("[n,tr]=train(n,x,t);").unwrap().output.join("\n");
    for epoch in [0, 2, 4, 5] {
        assert!(out.contains(&format!("Epoch {epoch}/5")), "{out}");
    }
    assert!(!out.contains("Epoch 1/5"));
    run(&mut e, "n.trainParam.show=NaN;");
    assert!(e.execute("n=train(n,x,t);").unwrap().output.is_empty());
    let (net, x, t) = api_problem();
    let mut opts = TrainOptions {
        output_bytes: 1,
        ..Default::default()
    };
    opts.parameters.insert("showCommandLine".into(), 1.);
    assert!(
        train::train(&net, &x, &t, opts, &mut |_| ControlFlow::Continue(()))
            .unwrap_err()
            .contains("output limit")
    );
}

#[test]
fn scg_recovers_from_rejected_steps_and_converges() {
    let mut e = Engine::default();
    run(&mut e, PATTERN);
    run(
        &mut e,
        "n.trainParam.epochs=50; [n,tr]=train(n,x,t); y=n(x); err=crossentropy(t,y);",
    );
    assert!(values(&e, "err")[0] < 1e-4);
    check(&mut e, "isequal(vec2ind(y),vec2ind(t))");
}
