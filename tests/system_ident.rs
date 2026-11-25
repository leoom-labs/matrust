//! System identification: iddata, idpoly/idtf/idss/idproc, arx/armax/oe/bj,
//! tfest/ssest/n4sid/procest, compare/resid/predict/sim.
//! Independent references: tool/refs/system_ident/system_ident.py (NumPy
//! lstsq, SciPy least_squares on the published prediction errors, lfilter,
//! cont2discrete, closed-form frequency responses), closed-form process-model
//! recursions written below and hand formulas. Data: Park-Miller streams.
use matrust::Engine;

/// u: uniform [-1, 1] input, e: 0.3 * uniform noise; both Park-Miller.
const GEN: &str = "N=300; x=12345; u=zeros(N,1); for k=1:N, x=mod(16807*x,2147483647); u(k)=2*x/2147483647-1; end; x=777; e=zeros(N,1); for k=1:N, x=mod(16807*x,2147483647); e(k)=0.3*(2*x/2147483647-1); end; A=[1 -1.5 0.7]; B=[0 1 0.5];";

fn run(source: &str) -> Engine {
    let mut e = Engine::default();
    e.execute(GEN).unwrap();
    e.execute(source).unwrap();
    e
}
fn values(e: &mut Engine, expression: &str) -> Vec<f64> {
    e.execute(&format!("zz__=double({expression});")).unwrap();
    e.variables["zz__"].data().to_vec()
}
fn scalar(e: &mut Engine, expression: &str) -> f64 {
    let v = values(e, expression);
    assert_eq!(v.len(), 1, "{expression}");
    v[0]
}
fn close(got: &[f64], expected: &[f64], tol: f64, what: &str) {
    assert_eq!(got.len(), expected.len(), "{what}: length");
    for (i, (&g, &x)) in got.iter().zip(expected).enumerate() {
        assert!(
            (g - x).abs() <= tol * (1. + x.abs()),
            "{what}[{i}]: {g} != {x}"
        );
    }
}
fn error(e: &mut Engine, source: &str) -> String {
    e.execute(source).unwrap_err().to_string()
}

#[test]
fn iddata_properties_indexing_and_detrend() {
    let mut e = run("y=filter(B,A,u); d=iddata(y,u,0.1);");
    assert_eq!(values(&mut e, "size(d)"), [300., 1., 1.]);
    assert_eq!(scalar(&mut e, "d.Ts"), 0.1);
    assert_eq!(scalar(&mut e, "d.Tstart"), 0.);
    assert_eq!(scalar(&mut e, "strcmp(class(d),'iddata')"), 1.);
    assert_eq!(scalar(&mut e, "isa(d,'iddata')"), 1.);
    assert_eq!(scalar(&mut e, "isequal(d.OutputData,y)"), 1.);
    assert_eq!(scalar(&mut e, "isequal(d.InputData,u)"), 1.);
    // Sample selection keeps the time axis.
    e.execute("d2=d(11:20); d3=d(end-9:end);").unwrap();
    assert_eq!(values(&mut e, "size(d2.OutputData)"), [10., 1.]);
    assert!((scalar(&mut e, "d2.Tstart") - 1.0).abs() < 1e-12);
    assert_eq!(scalar(&mut e, "isequal(d2.InputData,u(11:20))"), 1.);
    assert_eq!(scalar(&mut e, "isequal(d3.OutputData,y(291:300))"), 1.);
    assert_eq!(scalar(&mut e, "isequal(d(1:5,1,1).OutputData,y(1:5))"), 1.);
    // detrend: mean (default) and least-squares line.
    e.execute("dd=detrend(d); dl=detrend(iddata(y+3+0.01*(1:300)',u,0.1),1);")
        .unwrap();
    assert!(scalar(&mut e, "abs(mean(dd.OutputData))") < 1e-14);
    assert!(scalar(&mut e, "abs(mean(dd.InputData))") < 1e-14);
    assert!(scalar(&mut e, "abs(sum(((1:300)'-150.5).*dl.OutputData))") < 1e-9);
    assert!(scalar(&mut e, "abs(mean(dl.OutputData))") < 1e-13);
    // Row vectors are channels in time; names and time series.
    e.execute("t=iddata(y','','Ts',2,'OutputName','level');")
        .unwrap();
    assert_eq!(values(&mut e, "size(t)"), [300., 1., 0.]);
    assert_eq!(scalar(&mut e, "t.Ts"), 2.);
    assert_eq!(scalar(&mut e, "strcmp(t.OutputName{1},'level')"), 1.);
    assert!(error(&mut e, "iddata(ones(5,1),ones(4,1))").contains("same number of samples"));
    assert!(error(&mut e, "iddata(y,u,0.1,'Colour',1)").contains("unknown option name 'Colour'"));
    assert!(error(&mut e, "iddata(y,u,-1)").contains("Ts"));
    assert!(error(&mut e, "d(301)").contains("exceeds"));
    let mut e2 = Engine::default();
    e2.execute("d=iddata(ones(4,1),ones(4,1),0.5)").unwrap();
}

#[test]
#[rustfmt::skip]
fn arx_matches_numpy_least_squares() {
    // NumPy lstsq on rows t >= max(na, nb+nk-1).
    let noisy = [-1.5037916204195134,0.7060101993686793,0.9704275038534886,0.49807762017191237];
    let mut e = run("y=filter(B,A,u)+filter(1,A,e); d=iddata(y,u,1); m=arx(d,[2 2 1]); f=arx(iddata(filter(B,A,u),u,1),[2 2 1]);");
    let got = [values(&mut e, "m.A(2:3)"), values(&mut e, "m.B(2:3)")].concat();
    close(&got, &noisy, 1e-10, "arx noisy");
    assert_eq!(values(&mut e, "m.B(1)"), [0.]);
    let free = [values(&mut e, "f.A"), values(&mut e, "f.B")].concat();
    close(&free, &[1., -1.5, 0.7, 0., 1., 0.5], 1e-10, "arx noise-free");
    assert_eq!(scalar(&mut e, "strcmp(class(m),'idpoly')"), 1.);
    assert_eq!(values(&mut e, "[m.C m.D m.F]"), [1., 1., 1.]);
    // Report: LossFcn is the mean squared one-step prediction error.
    assert!(scalar(&mut e, "m.Report.Fit.FitPercent") > 50.);
    assert!(scalar(&mut e, "abs(m.NoiseVariance-m.Report.Fit.LossFcn)") < 1e-15);
    // Time series AR model.
    let mut e = run("v=filter(1,A,e); ar=arx(iddata(v,[],1),2);");
    assert!(scalar(&mut e, "abs(ar.A(2)+1.5)") < 0.2);
    assert!(error(&mut e, "arx(iddata(v,u,1),[2 2])").contains("[na nb nk]"));
    assert!(error(&mut e, "arx(iddata(v,u,1),[2 2 1],'Focus','simulation')").contains("unknown option name 'Focus'"));
    assert!(error(&mut e, "arx(iddata(v,u,1),[2 -1 1])").contains("nonnegative integers"));
}

#[test]
fn pem_estimators_recover_noise_free_systems() {
    let mut e = run(
        "ya=filter(B,A,u); d=iddata(ya,u,1); ma=armax(d,[2 2 1 1]); mo=oe(d,[2 2 1]); \
         C=[1 0.4]; D=[1 -0.6]; mb=bj(d,[2 1 1 2 1]);",
    );
    let truth = [-1.5, 0.7, 1., 0.5];
    for (m, a) in [("ma", "A"), ("mo", "F"), ("mb", "F")] {
        let got = [
            values(&mut e, &format!("{m}.{a}(2:3)")),
            values(&mut e, &format!("{m}.B(2:3)")),
        ]
        .concat();
        close(&got, &truth, 1e-3, m);
        // Simulated response equals the data.
        assert!(
            scalar(&mut e, &format!("max(abs(sim({m},d).OutputData-ya))")) < 1e-6,
            "{m} response"
        );
    }
    assert_eq!(values(&mut e, "size(mb.C)"), [1., 2.]);
    assert_eq!(values(&mut e, "size(mb.D)"), [1., 2.]);
    assert!(error(&mut e, "oe(d,[2 2])").contains("[nb nf nk]"));
    assert!(error(&mut e, "bj(d,[2 1 1 2])").contains("[nb nc nd nf nk]"));
}

#[test]
#[rustfmt::skip]
fn pem_estimators_match_scipy_on_noisy_data() {
    // scipy.optimize.least_squares on the zero-initial-condition prediction
    // errors, started at the truth; fit of the zero-state simulation.
    let armax_theta = [-1.503360612184051,0.7056645946672838,0.9720997357245236,0.4917014016866432,0.4678274217117952];
    let armax_fit = 70.25689682108474;
    let oe_theta = [0.9846110813634666,0.5166447291298537,-1.4990120274185683,0.6999686097015672];
    let oe_fit = 92.44949737658774;
    let bj_theta = [0.9744176603572152,0.5200823846942351,0.3700672332725271,-0.5977695734410495,-1.4996050817485258,0.7012335542982214];
    let bj_fit = 88.23468016070808;
    let mut e = run("C=[1 0.5]; ya=filter(B,A,u)+filter(C,A,e); ma=armax(iddata(ya,u,1),[2 2 1 1]); \
        yo=filter(B,A,u)+e; mo=oe(iddata(yo,u,1),[2 2 1]); \
        yb=filter(B,A,u)+filter([1 0.4],[1 -0.6],e); mb=bj(iddata(yb,u,1),[2 1 1 2 1]);");
    let fit = |e: &mut Engine, m: &str, y: &str| {
        e.execute(&format!("[yh,fit,x0]=compare(iddata({y},u,1),{m},'InitialCondition','z');")).unwrap();
        scalar(e, "fit")
    };
    assert!((fit(&mut e, "ma", "ya") - armax_fit).abs() < 1e-3);
    assert!((fit(&mut e, "mo", "yo") - oe_fit).abs() < 1e-3);
    assert!((fit(&mut e, "mb", "yb") - bj_fit).abs() < 1e-3);
    // Fitted responses agree with the reference models' simulations.
    let resp = |e: &mut Engine, m: &str, b: &[f64], f: &[f64]| {
        let src = format!("max(abs(sim({m},iddata(u,u,1)).OutputData-filter([0 {} {}],[1 {} {}],u)))", b[0], b[1], f[0], f[1]);
        scalar(e, &src)
    };
    assert!(resp(&mut e, "ma", &armax_theta[2..4], &armax_theta[0..2]) < 1e-6);
    assert!(resp(&mut e, "mo", &oe_theta[0..2], &oe_theta[2..4]) < 1e-6);
    assert!(resp(&mut e, "mb", &bj_theta[0..2], &bj_theta[4..6]) < 1e-6);
    close(&values(&mut e, "ma.C(2)"), &armax_theta[4..5], 1e-5, "armax C");
    close(&values(&mut e, "[mb.C(2) mb.D(2)]"), &bj_theta[2..4], 1e-5, "bj C D");
}

#[test]
#[rustfmt::skip]
fn continuous_and_subspace_second_order() {
    // G(s) = 2/(s^2+0.6s+2), zero-order hold Ts = 0.2 (SciPy cont2discrete).
    let bd = [0.0,0.038193303249824284,0.03669195484290888];
    let ad = [1.0,-1.8120351786244244,0.8869204367171576];
    let c_re = [1.004112319297626,1.1102299762093577,1.4705882352941178,-0.5730659025787966,-0.2679938744257274];
    let c_im = [-0.030274743295405807,-0.19032513877874704,-0.8823529411764707,-2.063037249283668,-0.06891271056661562];
    let d_re = [1.003742773473886,1.0988754606323425,1.3728753868309758,-0.8716209034141507,-0.2721482145266577];
    let d_im = [-0.04031378245090953,-0.2454748494558785,-1.0230556425759298,-1.9469220802495153,0.013118963369851116];
    let mut e = run(&format!(
        "y=filter([{} {} {}],[{} {} {}],u); d=iddata(y,u,0.2); w=[0.1 0.5 1 1.5 3]; \
         mt=tfest(d,2,0); ms=ssest(d,2); mn=n4sid(d,2);",
        bd[0], bd[1], bd[2], ad[0], ad[1], ad[2]
    ));
    for (m, re, im) in [("mt", &c_re, &c_im), ("ms", &c_re, &c_im), ("mn", &d_re, &d_im)] {
        let h_re = values(&mut e, &format!("real(squeeze(freqresp({m},w)))"));
        let h_im = values(&mut e, &format!("imag(squeeze(freqresp({m},w)))"));
        for i in 0..5 {
            assert!((h_re[i] - re[i]).abs() < 1e-3 && (h_im[i] - im[i]).abs() < 1e-3, "{m} at w({i})");
        }
    }
    assert_eq!(scalar(&mut e, "mt.Ts"), 0.);
    assert_eq!(scalar(&mut e, "mn.Ts"), 0.2);
    close(&values(&mut e, "mt.Denominator"), &[1., 0.6, 2.], 1e-6, "tfest den");
    close(&values(&mut e, "mt.Numerator"), &[2.], 1e-6, "tfest num");
    assert_eq!(scalar(&mut e, "strcmp(class(ms),'idss')"), 1.);
    assert_eq!(values(&mut e, "size(mn.A)"), [2., 2.]);
    // Model order from a range (largest singular-value gap).
    e.execute("mr=n4sid(d,1:4);").unwrap();
    assert_eq!(values(&mut e, "size(mr.A)"), [2., 2.]);
    // Discrete tfest.
    e.execute("mz=tfest(d,2,1,'Ts',0.2);").unwrap();
    let h = values(&mut e, "real(squeeze(freqresp(mz,w)))");
    close(&h, &d_re, 1e-6, "discrete tfest");
}

#[test]
#[rustfmt::skip]
fn procest_recovers_process_models() {
    // P1D: exact zero-order-hold recursion with a fractional delay
    // (K=2, Tp1=3, Td=0.35, Ts=0.1: 3 whole samples + 0.05 s).
    let p2_bd = [0.0,0.006906527537562823,0.006354380701500739];
    let p2_ad = [1.0,-1.769960177578696,0.7788007830714049];
    let mut e = run(&format!(
        "T=0.1; K=2; Tp=3; dd=3; tau=0.05; ph=exp(-T/Tp); q=exp(-(T-tau)/Tp); x=0; y=zeros(N,1); \
         for k=1:N, y(k)=x; a=0; b=0; if k-dd>=1, a=u(k-dd); end; if k-dd-1>=1, b=u(k-dd-1); end; x=ph*x+K*b*(q-ph)+K*a*(1-q); end; \
         m1=procest(iddata(y,u,T),'P1D'); \
         y2=filter([{} {} {}],[{} {} {}],u); m2=procest(iddata(y2,u,T),'P2'); \
         a1=exp(-T/0.8); m3=procest(iddata(filter([0 1-a1],[1 -a1],u),u,T),'P1');",
        p2_bd[0], p2_bd[1], p2_bd[2], p2_ad[0], p2_ad[1], p2_ad[2]
    ));
    close(&values(&mut e, "[m1.Kp m1.Tp1 m1.Td]"), &[2., 3., 0.35], 1e-6, "P1D");
    close(&values(&mut e, "[m2.Kp m2.Tp1 m2.Tp2]"), &[1.5, 2., 0.5], 1e-6, "P2");
    close(&values(&mut e, "[m3.Kp m3.Tp1]"), &[1., 0.8], 1e-6, "P1");
    assert_eq!(scalar(&mut e, "strcmp(m1.Type,'P1D')"), 1.);
    assert!(scalar(&mut e, "m1.Report.Fit.FitPercent") > 99.999);
    // A delayed continuous model has no LTI equivalent: explicit error.
    assert!(error(&mut e, "tf(m1)").contains("delay"));
    close(&values(&mut e, "tf(m3).Denominator{1}"), &[1., 1.25], 1e-6, "P1 tf");
    assert!(error(&mut e, "procest(iddata(y,u,T),'Q1')").contains("process model type"));
    assert!(error(&mut e, "procest(iddata(y,u,T),'P1DI')").contains("not supported"));
}

#[test]
fn compare_fit_formula_and_initial_state() {
    // True ARX model on a window that starts in a nonzero state.
    let mut e = run(
        "y=filter(B,A,u); d=iddata(y(51:300),u(51:300),1); m=idpoly(A,B); \
         [yz,fz,xz]=compare(d,m,'InitialCondition','z'); [ye,fe,xe]=compare(d,m);",
    );
    let fz = scalar(&mut e, "fz");
    let formula = scalar(
        &mut e,
        "100*(1-norm(d.OutputData-yz.OutputData)/norm(d.OutputData-mean(d.OutputData)))",
    );
    assert!((fz - formula).abs() < 1e-10);
    assert!(fz < 99.);
    // The estimated initial state removes the transient exactly.
    assert!((scalar(&mut e, "fe") - 100.).abs() < 1e-8);
    assert_eq!(values(&mut e, "size(xe)"), [2., 1.]);
    assert_eq!(values(&mut e, "xz"), [0., 0.]);
    // Explicit initial state reproduces the estimated fit.
    e.execute("[~,fx]=compare(d,m,'InitialCondition',xe); [~,f1]=compare(d,m,1);")
        .unwrap();
    assert!((scalar(&mut e, "fx") - 100.).abs() < 1e-8);
    // Prediction horizon 1 for an ARX model with noise-free data.
    // (zero initial conditions: exact once the predictor has two past samples)
    e.execute("[y1,~]=compare(d,m,1);").unwrap();
    assert!(scalar(&mut e, "max(abs(y1.OutputData(3:end)-d.OutputData(3:end)))") < 1e-12);
    let f1 = scalar(
        &mut e,
        "100*(1-norm(d.OutputData-y1.OutputData)/norm(d.OutputData-mean(d.OutputData)))",
    );
    assert!((scalar(&mut e, "f1") - f1).abs() < 1e-10);
    assert!(error(&mut e, "compare(d,m,'InitialCondition','q')").contains("InitialCondition"));
    assert!(error(&mut e, "compare(d,idpoly(A,B,[],[],[],1,0.5))").contains("sample time"));
}

#[test]
#[rustfmt::skip]
fn resid_correlations_and_99_percent_bounds() {
    // True ARX model on the noisy ARX data: residuals are e exactly.
    let auto = [1.0,-0.04395365370633604,-0.0066006587585184994,0.00909809498116196,0.05862672015541053,0.013215625773376519];
    let cross = [-0.03560266833795662,0.017767872390355433,-0.007508653196368539,-0.06437541076975692,0.07986488361616723,0.045526023663094856,-0.09312658728941145,0.009173029742387009,-0.016617604331193,0.0723291356644588,-0.0881482930719991];
    let cross_bound = 0.14926875378744392;
    let z995 = 2.5758293035489004;
    let mut e = run("y=filter(B,A,u)+filter(1,A,e); d=iddata(y,u,1); m=idpoly(A,B,1,1,1,0.03,1); [E,R,S]=resid(d,m,5);");
    assert!(scalar(&mut e, "max(abs(E.OutputData-e))") < 1e-12);
    close(&values(&mut e, "S.AutoCorr"), &auto, 1e-10, "auto");
    close(&values(&mut e, "S.CrossCorr"), &cross, 1e-10, "cross");
    assert!((scalar(&mut e, "S.AutoBound") - z995 / 300f64.sqrt()).abs() < 1e-14);
    assert!((scalar(&mut e, "S.CrossBound") - cross_bound).abs() < 1e-10);
    assert_eq!(values(&mut e, "size(R)"), [6., 2., 2.]);
    assert!(scalar(&mut e, "abs(R(1,1,1)-mean(e.^2))") < 1e-14);
    // R(tau+1,i,j) = 1/N sum z_i(t) z_j(t+tau) (MathWorks resid page).
    assert!(scalar(&mut e, "abs(R(2,1,2)-sum(e(1:end-1).*u(2:end))/300)") < 1e-14);
    assert!(scalar(&mut e, "abs(R(2,2,1)-sum(u(1:end-1).*e(2:end))/300)") < 1e-14);
    assert_eq!(values(&mut e, "S.Lags'"), [0., 1., 2., 3., 4., 5.]);
    assert_eq!(values(&mut e, "S.CrossLags'"), (-5..=5).map(f64::from).collect::<Vec<_>>());
    assert_eq!(scalar(&mut e, "S.Confidence"), 99.);
    assert_eq!(values(&mut e, "size(resid(d,m))"), [300., 1., 1.]);
}

#[test]
fn predict_and_sim() {
    let mut e = run(
        "C=[1 0.5]; y=filter(B,A,u)+filter(C,A,e); d=iddata(y,u,1); m=idpoly(A,B,C,[],[],0.03,1);",
    );
    // One step: yhat = y - e; two steps: y - e(t) - h1 e(t-1), h1 = c1 - a1.
    assert!(scalar(&mut e, "max(abs(predict(m,d,1).OutputData-(y-e)))") < 1e-12);
    assert!(scalar(&mut e, "max(abs(predict(m,d).OutputData-(y-e)))") < 1e-12);
    assert!(
        scalar(
            &mut e,
            "max(abs(predict(m,d,2).OutputData-(y-e-2*[0;e(1:end-1)])))"
        ) < 1e-12
    );
    assert!(
        scalar(
            &mut e,
            "max(abs(predict(m,d,Inf).OutputData-filter(B,A,u)))"
        ) < 1e-12
    );
    assert!(scalar(&mut e, "max(abs(sim(m,d).OutputData-filter(B,A,u)))") < 1e-12);
    assert!(scalar(&mut e, "max(abs(sim(m,u)-filter(B,A,u)))") < 1e-12);
    assert!(error(&mut e, "predict(m,d,0)").contains("horizon"));
}

#[test]
fn models_become_lti_values() {
    let mut e = run("m=idpoly([1 -0.8],[0 0.5],[],[],[],1,0.1); g=tf(m); \
         c=idtf(2,[1 3 2]); s=idss([-1 0;1 -2],[1;0],[0 1],0); p=idproc('P1','Kp',2,'Tp1',4);");
    assert_eq!(scalar(&mut e, "strcmp(class(g),'tf')"), 1.);
    close(&values(&mut e, "g.Numerator{1}"), &[0., 0.5], 1e-15, "num");
    close(
        &values(&mut e, "g.Denominator{1}"),
        &[1., -0.8],
        1e-15,
        "den",
    );
    assert_eq!(scalar(&mut e, "g.Ts"), 0.1);
    // step/lsim/bode agree with the converted model.
    assert!(scalar(&mut e, "max(abs(step(m,1)-step(g,1)))") < 1e-14);
    assert!(
        scalar(
            &mut e,
            "max(abs(lsim(c,ones(11,1),0:0.1:1)-lsim(tf(2,[1 3 2]),ones(11,1),0:0.1:1)))"
        ) < 1e-12
    );
    assert!(scalar(&mut e, "abs(dcgain(c)-1)") < 1e-14);
    e.execute("[mag,ph]=bode(c,1);").unwrap();
    assert!((scalar(&mut e, "mag") - 2. / 10f64.sqrt()).abs() < 1e-12);
    close(&values(&mut e, "ss(s).A"), &[-1., 1., 0., -2.], 0., "ss A");
    close(
        &values(&mut e, "tf(p).Denominator{1}"),
        &[1., 0.25],
        1e-15,
        "proc den",
    );
    close(
        &values(&mut e, "tf(p).Numerator{1}"),
        &[0., 0.5],
        1e-15,
        "proc num",
    );
    assert_eq!(scalar(&mut e, "strcmp(class(p),'idproc')"), 1.);
    assert_eq!(scalar(&mut e, "isa(c,'idtf')"), 1.);
    assert_eq!(scalar(&mut e, "isstruct(c)"), 0.);
    // From LTI values.
    e.execute("q=idpoly(tf([0 0.5],[1 -0.8],0.1)); r=idtf(tf(2,[1 3 2]));")
        .unwrap();
    close(
        &values(&mut e, "[q.A q.B]"),
        &[1., -0.8, 0., 0.5],
        1e-15,
        "idpoly(tf)",
    );
    close(
        &values(&mut e, "r.Denominator"),
        &[1., 3., 2.],
        1e-15,
        "idtf(tf)",
    );
    // Property edits are seen by the next call.
    e.execute("m.B=[0 1];").unwrap();
    close(
        &values(&mut e, "tf(m).Numerator{1}"),
        &[0., 1.],
        1e-15,
        "edited",
    );
    assert!(error(&mut e, "idpoly([2 1],[0 1])").contains("monic"));
    assert!(error(&mut e, "idss([1 2],1,1,0)").contains("inconsistent"));
    assert!(error(&mut e, "idproc('P1','Td',1)").contains("not a parameter"));
    assert!(error(&mut e, "step(idproc('P1D'),1)").contains("NaN"));
}

// ------------------------------------------------- review batch 6 fixes --

fn quick_error(e: &mut Engine, source: &str) -> String {
    let start = std::time::Instant::now();
    let err = error(e, source);
    assert!(
        start.elapsed() < std::time::Duration::from_secs(1),
        "{source}: took {:?}",
        start.elapsed()
    );
    err
}

#[test]
fn review_b6_discrete_delay_multiplies_by_z_power() {
    // 1/(z-0.5) z^-2 at z = 2: 1/(1.5*4) = 1/6 (hand value).
    let mut e = run("m=idtf(1,[1 -0.5],1); m.IODelay=2; g=tf(m); \
         p=idpoly([1 -0.5],[0 1],[],[],[],1,1); p.IODelay=1; h=tf(p);");
    let at2 = "polyval(g.Numerator{1},2)/polyval(g.Denominator{1},2)";
    assert!((scalar(&mut e, at2) - 1. / 6.).abs() < 1e-15);
    let at2 = "polyval(h.Numerator{1},2)/polyval(h.Denominator{1},2)";
    assert!((scalar(&mut e, at2) - 1. / 3.).abs() < 1e-15);
    // Step response of z^-3/(1-0.5 z^-1): 0 0 0 1 1.5 1.75; sim agrees.
    close(
        &values(&mut e, "step(m,5)"),
        &[0., 0., 0., 1., 1.5, 1.75],
        1e-14,
        "step",
    );
    close(
        &values(&mut e, "sim(m,ones(6,1))"),
        &[0., 0., 0., 1., 1.5, 1.75],
        1e-14,
        "sim",
    );
    // bode: the delay adds -2 rad of phase at w = 1 rad/s.
    e.execute("[mg,ph]=bode(m,1); [mg0,ph0]=bode(tf(1,[1 -0.5],1),1);")
        .unwrap();
    assert!((scalar(&mut e, "mg-mg0")).abs() < 1e-12);
    // Phase difference -2 rad modulo 360 degrees.
    assert!((scalar(&mut e, "cos((ph-ph0)*pi/180)") - 2f64.cos()).abs() < 1e-9);
    assert!((scalar(&mut e, "sin((ph-ph0)*pi/180)") + 2f64.sin()).abs() < 1e-9);
}

#[test]
fn review_b6_resid_lag_convention() {
    // residuals [1;0;0], input [0;1;0]: R(2,1,2) = e(1) u(2) / 3 = 1/3.
    let mut e =
        run("d=iddata([1;0;0],[0;1;0],1); m=idpoly(1,0,[],[],[],1,1); [E,R,S]=resid(d,m,2);");
    close(&values(&mut e, "E.OutputData"), &[1., 0., 0.], 0., "E");
    assert!((scalar(&mut e, "R(2,1,2)") - 1. / 3.).abs() < 1e-15);
    assert_eq!(scalar(&mut e, "R(2,2,1)"), 0.);
    // S.CrossCorr(tau) = e(t+tau) u(t): nonzero at tau = -1 only.
    close(
        &values(&mut e, "S.CrossCorr'"),
        &[0., 1., 0., 0., 0.],
        1e-15,
        "cross",
    );
}

#[test]
fn review_b6_idss_time_series_and_mimo_innovations() {
    // x+ = 0.5 x + e, y = x + e: e = [1; 2-1; 3-1.5].
    let mut e = run("m=idss(0.5,zeros(1,0),1,zeros(1,0),1,[],1); E=resid(iddata([1;2;3],[],1),m);");
    close(&values(&mut e, "E.OutputData"), &[1., 1., 1.5], 1e-15, "E");
    let err = error(
        &mut e,
        "m2=idss(0.5,[1 1],1,[0 0],1,[],1); resid(iddata([1;2;3],[1;1;1],1),m2)",
    );
    assert!(err.contains("single-input"), "{err}");
    let err = error(
        &mut e,
        "m3=idss(0.5,1,[1;1],[0;0],[1 1],[],1); resid(iddata([1;2;3],[1;1;1],1),m3)",
    );
    assert!(err.contains("single-input"), "{err}");
}

#[test]
fn review_b6_admission_before_allocation() {
    let mut e = Engine::default();
    let err = quick_error(
        &mut e,
        "x = numel(idss([],zeros(0,1000000),zeros(1000000,0),[]));",
    );
    assert!(err.contains("limit") || err.contains("budget"), "{err}");
    let err = quick_error(
        &mut e,
        "d=iddata(1,1); x = size(d(ones(1,100000),ones(1,100000)));",
    );
    assert!(err.contains("limit") || err.contains("budget"), "{err}");
    let err = quick_error(&mut e, "m=idtf(1,[1 -0.5],1); m.IODelay=1e12; g=tf(m);");
    assert!(err.contains("delay"), "{err}");
    let err = quick_error(&mut e, "m=idtf(1,[1 -0.5],1); m.IODelay=1000; g=tf(m);");
    assert!(err.contains("degree limit"), "{err}");
    let err = quick_error(
        &mut e,
        "m=idtf(1,[1 -0.5],1); m.IODelay=1e300; y=sim(m,ones(3,1));",
    );
    assert!(err.contains("delay"), "{err}");
    e.execute("d=iddata(zeros(1e6,1),zeros(1e6,1),1);").unwrap();
    let err = quick_error(&mut e, "m=n4sid(d,[1 64]);");
    assert!(err.contains("work bound") || err.contains("limit"), "{err}");
    let err = quick_error(&mut e, "m=tfest(d,1,0,1e300,'Ts',1);");
    assert!(err.contains("shorter than the data"), "{err}");
}

#[test]
fn review_b6_resid_and_predict_bounded_and_cancellable() {
    let mut e = Engine::default();
    e.execute("m=idpoly([1 -0.5],[0 1],[1 0.3],[],[],1,1); d=iddata(zeros(1e6,1),zeros(1e6,1),1);")
        .unwrap();
    let err = quick_error(&mut e, "[E,R]=resid(d,m,'MaxLag',10000);");
    assert!(err.contains("work bound"), "{err}");
    let err = quick_error(&mut e, "y=predict(m,d,500000);");
    assert!(err.contains("work bound"), "{err}");
    // Within the bounds but seconds of work: Stop interrupts within ~0.2 s.
    e.execute("d=iddata(ones(1e5,1),ones(1e5,1),1);").unwrap();
    for source in ["[E,R]=resid(d,m,'MaxLag',10000);", "y=predict(m,d,50000);"] {
        let flag = e.cancellation();
        flag.store(false, std::sync::atomic::Ordering::Relaxed);
        let start = std::time::Instant::now();
        let stopper = std::thread::spawn({
            let flag = flag.clone();
            move || {
                std::thread::sleep(std::time::Duration::from_millis(200));
                flag.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        });
        let result = e.execute(source);
        stopper.join().unwrap();
        flag.store(false, std::sync::atomic::Ordering::Relaxed);
        let err = result.expect_err(source).to_string();
        assert!(err.contains("cancel"), "{source}: {err}");
        assert!(
            start.elapsed() < std::time::Duration::from_secs(2),
            "{source}: {:?}",
            start.elapsed()
        );
    }
}

#[test]
fn review_b6_constructor_options_apply() {
    let mut e = run(
        "a=idtf(1,[1 -0.5],'Ts',1); b=idtf(1,[1 -0.5],1,'IODelay',2,'InputName','force','OutputName',{'pos'}); \
         c=idpoly([1 -0.5],[0 1],'Ts',0.5,'NoiseVariance',2); s=idss(0.5,1,1,0,'Ts',1,'InputDelay',1); \
         v=idtf(1,[1 -0.5],1,'Variable','z^-1'); g=tf(b); h=tf(v);",
    );
    assert_eq!(scalar(&mut e, "a.Ts"), 1.);
    assert_eq!(scalar(&mut e, "b.IODelay"), 2.);
    assert!(
        (scalar(
            &mut e,
            "polyval(g.Numerator{1},2)/polyval(g.Denominator{1},2)"
        ) - 1. / 6.)
            .abs()
            < 1e-15
    );
    assert_eq!(scalar(&mut e, "strcmp(b.InputName{1},'force')"), 1.);
    assert_eq!(scalar(&mut e, "strcmp(b.OutputName{1},'pos')"), 1.);
    assert_eq!(scalar(&mut e, "c.Ts"), 0.5);
    assert_eq!(scalar(&mut e, "c.NoiseVariance"), 2.);
    assert_eq!(scalar(&mut e, "s.Ts"), 1.);
    assert_eq!(scalar(&mut e, "s.InputDelay"), 1.);
    // 1/(1 - 0.5 z^-1) = z/(z - 0.5): 4/3 at z = 2.
    assert!(
        (scalar(
            &mut e,
            "polyval(h.Numerator{1},2)/polyval(h.Denominator{1},2)"
        ) - 4. / 3.)
            .abs()
            < 1e-15
    );
    assert!(error(&mut e, "idtf(1,[1 2],'Foo',1)").contains("unknown option"));
    assert!(error(&mut e, "idss(0.5,1,1,0,'IODelay',1)").contains("unknown option"));
    assert!(error(&mut e, "idtf(1,[1 2],'Variable','z')").contains("sample time"));
    assert!(error(&mut e, "idpoly([1 -0.5],[0 1],'Ts',0)").contains("Ts"));
    assert!(error(&mut e, "idtf(1,[1 -0.5],1,'IODelay',0.5)").contains("whole samples"));
}

#[test]
fn review_b6_tfest_feedthrough_and_max_iterations() {
    // Noise-free first order with a direct term: exact recovery.
    let mut e = run("y=filter([0.5 0.3],[1 -0.6],u); d=iddata(y,u,1); \
         m=tfest(d,1,1,'Ts',1,'Feedthrough',true);");
    close(&values(&mut e, "m.Numerator"), &[0.5, 0.3], 1e-8, "num");
    close(&values(&mut e, "m.Denominator"), &[1., -0.6], 1e-8, "den");
    assert!(error(&mut e, "tfest(d,2,1,'Feedthrough',true)").contains("Feedthrough"));
    // MaxIterations bounds the PEM iterations.
    e.execute(
        "y2=filter(B,A,u)+filter([1 0.5],A,e); d2=iddata(y2,u,1); \
         m1=armax(d2,[2 2 1 1],'MaxIterations',1); m0=armax(d2,[2 2 1 1]);",
    )
    .unwrap();
    assert!(scalar(&mut e, "m1.Report.Termination.Iterations") <= 1.);
    assert!(scalar(&mut e, "m0.Report.Termination.Iterations") > 1.);
}

#[test]
fn review_b6_sample_time_checked_on_every_path() {
    let mut e = run("y=filter(B,A,u); d=iddata(y,u,2); m=idpoly(A,B,[],[],[],1,1);");
    for source in ["sim(m,d)", "predict(m,d,1)", "resid(d,m)", "compare(d,m,1)"] {
        let err = error(&mut e, source);
        assert!(err.contains("sample time"), "{source}: {err}");
    }
}
