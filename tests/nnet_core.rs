//! Independent numerical expectations: tool/refs/nnet/core.py (NumPy).
use matrust::{Array, Engine};
fn run(e: &mut Engine, s: &str) -> String {
    e.execute(s)
        .unwrap_or_else(|err| panic!("{s}: {err}"))
        .output
        .join("\n")
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
fn near(e: &Engine, name: &str, expected: &[f64]) {
    let actual = values(e, name);
    assert_eq!(actual.len(), expected.len());
    for (a, b) in actual.iter().zip(expected) {
        assert!(
            (a - b).abs() <= 1e-12 * b.abs().max(1e-12),
            "{name}: {a} != {b}"
        );
    }
}
fn fails(e: &mut Engine, s: &str, part: &str) {
    let err = e.execute(s).unwrap_err();
    assert!(err.contains(part), "{s}: {err}");
}
const DATA: &str = "x=[1 2 4;7 7 7;-2 0 3]; t=[2 4 8;-1 3 5];";
#[test]
fn forward_numpy_fit_and_pattern() {
    let mut e = Engine::default();
    run(&mut e, DATA);
    run(
        &mut e,
        "n=configure(fitnet([4 3]),x,t); n=setwb(n,(1:n.numWeightElements)'*0.02-0.3); y=n(x); z=sim(n,x);",
    );
    near(
        &e,
        "y",
        &[
            5.721366909129327,
            2.7776244320202546,
            5.453693839952347,
            2.494413997075757,
            5.02817684578707,
            2.044183755518855,
        ],
    );
    check(&mut e, "isequal(y,z)");
    run(
        &mut e,
        "p=configure(patternnet(5),x,eye(3)); p=setwb(p,(1:p.numWeightElements)'*0.02-0.3); y=p(x);",
    );
    near(
        &e,
        "y",
        &[
            0.3286737390075749,
            0.33331151939409726,
            0.3380147415983278,
            0.332773799248592,
            0.33333302007956706,
            0.3338931806718409,
            0.3398385383602459,
            0.33329128739050845,
            0.3268701742492456,
        ],
    );
    run(
        &mut e,
        "c=n({x(:,1),x(:,2),x(:,3)}); d=sim(n,{x(:,1),x(:,2)});",
    );
    check(&mut e, "isequal(c{1},z(:,1)) && isequal(d{2},z(:,2))");
}
#[test]
fn transfer_functions_and_derivatives_numpy() {
    let mut e = Engine::default();
    run(
        &mut e,
        "x=[-2 -0.5 0 0.5 2]; y=tansig(x); d=tansig('dn',x,y,[]);",
    );
    near(
        &e,
        "y",
        &[
            -0.9640275800758169,
            -0.46211715726000974,
            0.,
            0.46211715726000974,
            0.9640275800758169,
        ],
    );
    near(
        &e,
        "d",
        &[
            0.07065082485316443,
            0.7864477329659274,
            1.,
            0.7864477329659274,
            0.07065082485316443,
        ],
    );
    run(&mut e, "y=logsig(x); d=logsig('dn',x,y,[]);");
    near(
        &e,
        "y",
        &[
            0.11920292202211755,
            0.3775406687981454,
            0.5,
            0.6224593312018546,
            0.8807970779778823,
        ],
    );
    near(
        &e,
        "d",
        &[
            0.10499358540350652,
            0.2350037122015945,
            0.25,
            0.2350037122015945,
            0.1049935854035065,
        ],
    );
    run(&mut e, "y=radbas(x); d=radbas('dn',x,y,[]);");
    near(
        &e,
        "y",
        &[
            0.01831563888873418,
            0.7788007830714049,
            1.,
            0.7788007830714049,
            0.01831563888873418,
        ],
    );
    near(
        &e,
        "d",
        &[
            0.07326255555493671,
            0.7788007830714049,
            0.,
            -0.7788007830714049,
            -0.07326255555493671,
        ],
    );
    check(
        &mut e,
        "isnan(poslin(NaN)) && radbas(1e300)==0 && radbas('dn',1e300,0,[])==0",
    );
    check(&mut e, "isequal(purelin(x),x)");
    check(&mut e, "isequal(poslin(x),[0 0 0 0.5 2])");
    check(&mut e, "isequal(satlin(x),[0 0 0 0.5 1])");
    check(&mut e, "isequal(hardlim(x),[0 0 1 1 1])");
    check(&mut e, "isequal(purelin('dn',x,x,[]),ones(1,5))");
    check(&mut e, "isequal(poslin('dn',x,poslin(x),[]),[0 0 1 1 1])");
    check(&mut e, "isequal(satlin('dn',x,satlin(x),[]),[0 0 1 1 0])");
    run(
        &mut e,
        "a=softmax([1000 0;1000 0]); d=softmax('dn',[1000 0;1000 0],a,[]);",
    );
    near(&e, "a", &[0.5, 0.5, 0.5, 0.5]);
    near(
        &e,
        "d",
        &[0.25, -0.25, -0.25, 0.25, 0.25, -0.25, -0.25, 0.25],
    );
}
#[test]
fn process_round_trips_and_settings() {
    let mut e = Engine::default();
    run(&mut e, DATA);
    run(
        &mut e,
        "[r,ps]=removeconstantrows(x); z=removeconstantrows('reverse',r,ps); [y,mm]=mapminmax(r); z2=mapminmax('reverse',y,mm); [s,ms]=mapstd(r); z3=mapstd('reverse',s,ms);",
    );
    check(&mut e, "isequal(z,x)");
    check(&mut e, "norm(z2-r)<1e-12 && norm(z3-r)<1e-12");
    near(&e, "y", &[-1., -1., -1. / 3., -0.2, 1., 1.]);
    near(
        &e,
        "s",
        &[
            -0.8728715609439697,
            -0.9271726499455306,
            -0.2182178902359925,
            -0.13245323570650436,
            1.0910894511799618,
            1.0596258856520349,
        ],
    );
    check(
        &mut e,
        "isequal(ps.keep,[1 3]) && isequal(ps.remove,2) && ps.value==7",
    );
    check(
        &mut e,
        "isequal(mapminmax('apply',r,mm),y) && isequal(mapstd('apply',r,ms),s)",
    );
    run(
        &mut e,
        "[a,p]=mapstd(x); b=mapstd('reverse',a,p); [d,q]=mapminmax(r,0,1);",
    );
    check(&mut e, "norm(b-x)<1e-12 && isequal(a(2,:),x(2,:))");
    near(&e, "d", &[0., 0., 1. / 3., 0.4, 1., 1.]);
    run(
        &mut e,
        "[a,p]=removeconstantrows(ones(2,3)); b=removeconstantrows('reverse',a,p);",
    );
    check(&mut e, "isequal(b,ones(2,3))");
    fails(&mut e, "mapminmax('apply',x,mm)", "row count");
    fails(&mut e, "mapstd(x,0,0)", "parameters");
}
#[test]
fn performance_and_indices_numpy() {
    let mut e = Engine::default();
    run(
        &mut e,
        "err=[1 -2 3;-4 0 2]; y=[mse(err),mae(err),sse(err),sae(err)];",
    );
    near(&e, "y", &[5.666666666666667, 2., 34., 12.]);
    run(
        &mut e,
        "p=patternnet(2); t=eye(3); a=[0.8 0.1 0.2;0.1 0.7 0.3;0.1 0.2 0.5]; y=[perform(p,t,a),crossentropy(p,t,a),crossentropy([1 0 1],[0.8 0.3 0.9])];",
    );
    near(
        &e,
        "y",
        &[0.1414406306458764, 0.1414406306458764, 0.22839300363692283],
    );
    run(&mut e, "v=vec2ind(a); z=full(ind2vec([3 1 2],4));");
    near(&e, "v", &[1., 2., 3.]);
    near(&e, "z", &[0., 0., 1., 0., 1., 0., 0., 0., 0., 1., 0., 0.]);
    check(&mut e, "issparse(ind2vec([1 2]))");
    fails(&mut e, "ind2vec([0 1])", "size");
    run(
        &mut e,
        "n=configure(fitnet(1),[0 1],[0 1]); n=setwb(n,ones(n.numWeightElements,1)); n.performParam.regularization=0.25; y=mse(n,[0 1],[1 3]);",
    );
    near(&e, "y", &[2.125]);
    run(
        &mut e,
        "n=configure(n,[0 1],[0 4]); n.performParam.regularization=0; n.performParam.normalization='standard'; y=mse(n,[0 4],[1 6]);",
    );
    near(&e, "y", &[0.625]);
}
#[test]
fn value_semantics_properties_defaults_and_display() {
    let mut e = Engine::default();
    run(
        &mut e,
        "n=fitnet; m=n; m.trainParam.epochs=5; m.layers{1}.size=3; m.trainFcn='trainscg';",
    );
    check(
        &mut e,
        "n.layers{1}.size==10 && n.trainParam.epochs==1000 && m.layers{1}.size==3 && m.trainParam.epochs==1000",
    );
    check(
        &mut e,
        "m.trainParam.sigma==5e-5 && m.trainParam.min_grad==1e-6",
    );
    run(
        &mut e,
        "m.divideFcn='divideind'; m.divideParam.trainInd=[1 3];",
    );
    check(
        &mut e,
        "isequal(m.divideParam.trainInd,[1 3]) && strcmp(n.divideFcn,'dividerand')",
    );
    fails(&mut e, "n.noSuchProperty=1", "noSuchProperty");
    fails(&mut e, "n.layers{1}.noSuchProperty=1", "noSuchProperty");
    fails(&mut e, "n.trainParam.noSuchProperty=1", "noSuchProperty");
    fails(&mut e, "n.numLayers=4", "read-only");
    fails(&mut e, "n([1 2])", "not configured");
    check(
        &mut e,
        "isa(n,'network') && ~isa(n,'handle') && isprop(n,'trainFcn')",
    );
    check(&mut e, "any(strcmp(fieldnames(n),'IW'))");
    let text = run(&mut e, "n");
    for title in [
        "dimensions:",
        "connections:",
        "subobjects:",
        "functions:",
        "weight and bias values:",
    ] {
        assert!(text.contains(title), "{text}");
    }
    for (f, key, value) in [
        ("traingd", "lr", 0.01),
        ("traingdm", "mc", 0.9),
        ("traingdx", "lr_inc", 1.05),
        ("trainrp", "delta0", 0.07),
        ("trainbr", "mu", 0.005),
        ("trainbfg", "min_grad", 1e-6),
        ("traincgf", "min_grad", 1e-10),
    ] {
        run(&mut e, &format!("m.trainFcn='{f}';"));
        check(&mut e, &format!("m.trainParam.{key}=={value}"));
    }
}
#[test]
fn configure_weight_order_rng_and_cascade() {
    let mut e = Engine::default();
    run(&mut e, DATA);
    run(
        &mut e,
        "rng(42); a=configure(fitnet([4 3]),x,t); wa=getwb(a); rng(42); b=configure(fitnet([4 3]),x,t); wb=getwb(b); rng(43); c=init(a); wc=getwb(c);",
    );
    check(&mut e, "isequal(wa,wb) && ~isequal(wa,wc)");
    check(
        &mut e,
        "a.inputs{1}.size==3 && a.inputs{1}.processedSize==2 && a.layers{3}.size==2 && a.numWeightElements==35",
    );
    run(
        &mut e,
        "a=setwb(a,(1:35)'); v=getwb(a); iw=a.IW{1,1}; bb=a.b{2}; lw=a.LW{2,1};",
    );
    near(&e, "v", &(1..=35).map(|i| i as f64).collect::<Vec<_>>());
    near(&e, "iw", &[5., 6., 7., 8., 9., 10., 11., 12.]);
    near(&e, "bb", &[13., 14., 15.]);
    near(&e, "lw", &(16..=27).map(|i| i as f64).collect::<Vec<_>>());
    run(&mut e, "b=a; b.IW{1,1}=zeros(4,2); b.b{2}=ones(3,1);");
    check(
        &mut e,
        "a.IW{1,1}(1)==5 && b.IW{1,1}(1)==0 && a.b{2}(1)==13",
    );
    fails(&mut e, "b.IW{1,1}=ones(2)", "size");
    fails(&mut e, "setwb(a,[1 2])", "numWeightElements");
    run(
        &mut e,
        "rng(9); a=configure(fitnet(4),x,t); w=a.IW{1,1}; z=sqrt(sum(w.^2,2));",
    );
    near(&e, "z", &[2.8, 2.8, 2.8, 2.8]);
    run(
        &mut e,
        "c=cascadeforwardnet([2 2]); c.inputs{1}.processFcns={}; c.outputs{3}.processFcns={}; c=configure(c,[1 2],[0 1]); c=setwb(c,ones(c.numWeightElements,1)); y=c([1 2]);",
    );
    // NumPy: a1=tanh(x+1), a2=tanh(x+1+2*a1), y=x+1+2*a1+2*a2.
    near(&e, "y", &[5.926506249285259, 6.9899242882457076]);
}
#[test]
fn help_examples_and_bounds() {
    let mut e = Engine::default();
    for name in [
        "fitnet",
        "configure",
        "tansig",
        "mapminmax",
        "perform",
        "ind2vec",
    ] {
        assert!(!run(&mut e, &format!("help {name}")).contains("not support"));
    }
    fails(&mut e, "fitnet([100000 100000])", "weight");
    fails(&mut e, "fitnet(-1)", "size");
    fails(&mut e, "fitnet(2,'uifigure')", "trainFcn");
}

#[test]
fn standalone_api_cancellation_and_release_benchmark() {
    use matrust::Matrix;
    use matrust::engine::nnet::network::Network;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Instant;
    let cancel = AtomicBool::new(false);
    let x = Matrix::from_column_vec(
        256,
        1024,
        (0..256 * 1024).map(|i| (i as f64 * 0.01).sin()).collect(),
    )
    .unwrap();
    let t = Matrix::from_column_vec(
        4,
        1024,
        (0..4 * 1024).map(|i| (i as f64 * 0.03).cos()).collect(),
    )
    .unwrap();
    let mut n = Network::new(&[256, 128], false, false).unwrap();
    n.configure(&x, &t, &cancel).unwrap();
    let w: Vec<f64> = (0..n.num_weight_elements())
        .map(|i| (i as f64 * 0.1).sin() * 0.01)
        .collect();
    n.setwb(&w).unwrap();
    let mut baseline = None;
    for workers in [1, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            let y=n.forward(&x).unwrap();
            if let Some(ref expected)=baseline {let expected:&Vec<f64>=expected;for (a,b) in y.data().iter().zip(expected){assert!((a-b).abs()<1e-12);}}
            else {baseline=Some(y.data().to_vec());}
            let start=Instant::now();
            for _ in 0..4 {std::hint::black_box(n.forward(&x).unwrap());}
            eprintln!("nnet forward benchmark: workers={workers}, 256-[256,128]-4, samples=1024, repeats=4, elapsed={:?}, weights={}, cap=4000000",start.elapsed(),n.num_weight_elements());
        });
    }
    cancel.store(true, Ordering::Relaxed);
    assert!(n.forward_cancel(&x, &cancel).is_err());
}

#[test]
fn all_documented_nnet_examples_execute() {
    let names = "cascadeforwardnet configure crossentropy feedforwardnet fitnet getwb hardlim ind2vec init isconfigured logsig mae mapminmax mapstd mse patternnet perform poslin purelin radbas removeconstantrows sae satlin setwb softmax sse tansig vec2ind";
    for name in names.split_whitespace() {
        let page = matrust::help::page(name).unwrap();
        for line in page
            .lines()
            .filter_map(|line| line.strip_prefix("Example:"))
        {
            let mut e = Engine::default();
            run(&mut e, line.trim());
        }
    }
}

#[test]
fn processing_edits_missing_rows_and_atomic_errors() {
    let mut e = Engine::default();
    run(&mut e, DATA);
    run(
        &mut e,
        "n=configure(fitnet(2),x,t); m=n; m.inputs{1}.processFcns={}; m.outputs{2}.processFcns={}; m=configure(m,x,t); m.layers{1}.transferFcn='logsig'; y=m(x);",
    );
    check(
        &mut e,
        "n.inputs{1}.processedSize==2 && m.inputs{1}.processedSize==3 && isequal(size(y),size(t))",
    );
    check(
        &mut e,
        "islogical(n.biasConnect) && islogical(n.layerConnect)",
    );
    run(
        &mut e,
        "[a,p]=removeconstantrows([1 1 NaN;1 2 3]); b=removeconstantrows('reverse',a,p);",
    );
    near(&e, "b", &[1., 1., 1., 2., 1., 3.]);
    fails(&mut e, "m.layers{1}.size=1e10", "size");
    check(&mut e, "m.layers{1}.size==2");
    fails(&mut e, "m.trainParam.epochs=-1", "epochs");
    check(&mut e, "m.trainParam.epochs==1000");
    fails(&mut e, "m.inputs{1}.processSettings={1}", "read-only");
    fails(&mut e, "m.layerConnect=ones(2)", "recurrent");
    run(
        &mut e,
        "a=fitnet(2); a.userdata=fitnet(3); clear n m; b=a.userdata;",
    );
    check(&mut e, "b.layers{1}.size==3");
}

#[test]
fn constant_targets_init_stream_and_all_performance_forms() {
    let mut e = Engine::default();
    run(
        &mut e,
        "x=[0 1 2]; n=configure(fitnet(3),x,[5 5 5]); y=n(x);",
    );
    near(&e, "y", &[5., 5., 5.]);
    run(&mut e, "rng(8); a=init(n); rng(8); b=init(n);");
    check(&mut e, "isequal(getwb(a),getwb(b))");
    run(&mut e, "rng(8); r=rand; rng(8); a=init(n); s=rand;");
    check(&mut e, "r~=s");
    run(
        &mut e,
        "n=fitnet(2); t=[1 -2 3;-4 0 2]; z=zeros(2,3); y=[mse(n,t,z),mae(n,t,z),sse(n,t,z),sae(n,t,z)];",
    );
    near(&e, "y", &[5.666666666666667, 2., 34., 12.]);
    for name in ["mse", "mae", "sse", "sae"] {
        run(
            &mut e,
            &format!("n.performFcn='{name}'; p=perform(n,t,z); q={name}(n,t,z);"),
        );
        check(&mut e, "p==q");
    }
    run(
        &mut e,
        "n=configure(fitnet(2),[0 1],[0 4]); n.performParam.normalization='percent'; y=[mse(n,[0 4],[1 6]),mae(n,[0 4],[1 6]),sse(n,[0 4],[1 6]),sae(n,[0 4],[1 6])];",
    );
    near(&e, "y", &[0.15625, 0.375, 0.3125, 0.75]);
}

#[test]
fn network_equality_compares_values_including_nested_nan_userdata() {
    let mut e = Engine::default();
    run(
        &mut e,
        "a=fitnet(1); b=a; b.trainParam.epochs=a.trainParam.epochs; assert(isequal(a,b)); b.inputs{1}.size=a.inputs{1}.size; b.layers{1}.name=a.layers{1}.name; b.outputs{2}.size=a.outputs{2}.size; assert(isequal(a,b)); assert(isequaln(a,b)); assert(isequal({a}, {b})); assert(isequaln(struct('net',a),struct('net',b))); b.trainParam.epochs=7; assert(~isequal(a,b) && ~isequaln(a,b)); assert(~isequal({a},{b}));",
    );
    run(&mut e, "a=configure(a,[1 2 3],[2 4 6]);");
    for change in [
        "b.trainParam.epochs=7",
        "b.name='changed'",
        "b.userdata=42",
        "b.IW{1}=b.IW{1}+1",
        "b.LW{2,1}=b.LW{2,1}+1",
        "b.b{1}=b.b{1}+1",
        "b.inputs{1}.processParams{2}.ymin=-2",
        "b.layers{1}.transferFcn='purelin'",
    ] {
        run(
            &mut e,
            &format!(
                "b=a; {change}; assert(~isequal(a,b) && ~isequaln(a,b)); assert(~isequal(struct('net',a),struct('net',b)));"
            ),
        );
    }
    run(
        &mut e,
        "b=a; b.trainParam.epochs=a.trainParam.epochs; assert(isequal(a,b)); a.userdata=struct('values',{{NaN,1}}); b=a; b.name=a.name; assert(~isequal(a,a) && ~isequal(a,b)); assert(isequaln(a,b)); assert(~isequal({struct('net',a)},{struct('net',b)})); assert(isequaln({struct('net',a)},{struct('net',b)})); b.userdata.values{2}=2; assert(~isequaln(a,b)); h=inputParser; k=inputParser; assert(isequal(h,h) && ~isequal(h,k)); assert(isequaln({h},{h}) && ~isequaln({h},{k}));",
    );
}

#[test]
fn network_equality_covers_dynamic_som_and_fitted_settings() {
    let mut e = Engine::default();
    run(
        &mut e,
        "a=narxnet(1:2,1:2,1); b=a; b.inputs{1}.size=a.inputs{1}.size; assert(isequal(a,b)); b.outputs{2}.feedbackMode='open'; assert(isequal(a,b) && isequaln({a},{b})); b=closeloop(a); assert(~isequal(a,b)); b=removedelay(a,1); assert(~isequaln(a,b)); a=selforgmap([2 2]); b=a; b.layers{1}.dimensions=a.layers{1}.dimensions; assert(isequal(a,b)); b=selforgmap([2 2],20); assert(~isequal(a,b));",
    );
    run(
        &mut e,
        "a=configure(fitnet(1),[1 2 3],[2 4 6]); b=configure(fitnet(1),[2 3 4],[2 4 6]); b=setwb(b,getwb(a)); assert(~isequal(a,b) && ~isequaln(a,b)); a.trainParam.show=NaN; b=a; b.name=a.name; assert(~isequal(a,b) && isequaln(a,b)); a=fitnet(1); b=a; b.name=a.name; c=fitnet(1); c.userdata={a}; d=c; d.userdata={b}; assert(isequal(c,d));",
    );
}
