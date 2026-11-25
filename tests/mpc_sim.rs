//! Independent state-space recurrence and public mpcmove hand-loop oracles; no MATLAB.
use matrust::{Array, Engine, Matrix};
fn mat(e: &Engine, name: &str) -> Matrix {
    match e.array(name).unwrap() {
        Array::Numeric(m) => m,
        a => panic!("{name}: {a:?}"),
    }
}
#[test]
fn three_sample_scalar_hand_loop() {
    let mut e = Engine::default();
    e.execute("c=mpc(ss(.8,.2,1,0,.1));[y,t,u]=sim(c,3,1);s=mpcstate(c);x=0;yh=zeros(3,1);uh=zeros(3,1);for k=1:3;yh(k)=x;uh(k)=mpcmove(c,s,x,1);x=.8*x+.2*uh(k);end").unwrap();
    for (a, b) in [("y", "yh"), ("u", "uh")] {
        let error = mat(&e, a)
            .data()
            .iter()
            .zip(mat(&e, b).data())
            .map(|(x, y)| (x - y).abs())
            .fold(0., f64::max);
        eprintln!("{a} hand-loop maximum error={error:e}");
        assert!(error <= 1e-12);
    }
    assert_eq!(mat(&e, "t").data(), &[0., 0.1, 0.2]);
}

fn close(a: &Matrix, b: &Matrix, tol: f64) {
    assert_eq!((a.rows(), a.cols()), (b.rows(), b.cols()));
    let error = a
        .data()
        .iter()
        .zip(b.data())
        .map(|(x, y)| (x - y).abs())
        .fold(0., f64::max);
    assert!(error <= tol, "maximum absolute error={error:e}");
}
#[test]
fn thirty_samples_match_hand_loop_and_constrained_step_settles() {
    let mut e = Engine::default();
    e.execute("c=mpc(ss(.8,.2,1,0,.1));c.MV.Min=-1.5;c.MV.Max=1.5;[y,t,u,xp,xc,o]=sim(c,30,1);s=mpcstate(c);x=0;yh=zeros(30,1);uh=zeros(30,1);for k=1:30;yh(k)=x;uh(k)=mpcmove(c,s,x,1);x=.8*x+.2*uh(k);end;ys=sim(c,100,1);").unwrap();
    close(&mat(&e, "y"), &mat(&e, "yh"), 1e-12);
    close(&mat(&e, "u"), &mat(&e, "uh"), 1e-12);
    assert!(
        mat(&e, "u")
            .data()
            .iter()
            .all(|v| *v <= 1.5 + 1e-9 && *v >= -1.5 - 1e-9)
    );
    assert!((mat(&e, "ys").get(99, 0) - 1.).abs() < 1e-8);
    assert!(e.figure.is_none());
}
#[test]
fn six_output_shapes_and_independent_initial_state_snapshots() {
    let mut e = Engine::default();
    e.execute("p=ss([.8 0;.1 .7],[.2 0;0 .3],eye(2),zeros(2),.2);p=setmpcsignals(p,'MO',2,'UO',1);c=mpc(p);s=mpcstate(c);s.Plant=[.4;.5];s.LastMove=[.1;.2];o=mpcsimopt(c);o.PlantInitialState=[2;3];o.ControllerInitialState=s;before=s.Plant;cov=s.Covariance;[y,t,u,xp,xc,used]=sim(c,4,[1 2],[],o);[y2,t2,u2]=sim(c,4,[1 2],o);after=s.Plant;aftercov=s.Covariance;last=s.LastMove;a=xc(1).LastMove;b=xc(4).LastMove;sizec=size(xc);first=xp(1,:);one=sim(c,1,[1 2],o);returned=used.ControllerInitialState.Plant;").unwrap();
    for (name, r, c) in [
        ("y", 4, 2),
        ("t", 4, 1),
        ("u", 4, 2),
        ("xp", 4, 2),
        ("one", 1, 2),
    ] {
        let a = mat(&e, name);
        assert_eq!((a.rows(), a.cols()), (r, c), "{name}");
    }
    assert_eq!(mat(&e, "sizec").data(), &[4., 1.]);
    assert_eq!(mat(&e, "first").data(), &[2., 3.]);
    close(&mat(&e, "before"), &mat(&e, "after"), 0.);
    close(&mat(&e, "before"), &mat(&e, "returned"), 0.);
    close(&mat(&e, "cov"), &mat(&e, "aftercov"), 0.);
    assert_eq!(mat(&e, "last").data(), &[0.1, 0.2]);
    close(&mat(&e, "y"), &mat(&e, "y2"), 0.);
    close(&mat(&e, "u"), &mat(&e, "u2"), 0.);
    assert_ne!(mat(&e, "a").data(), mat(&e, "b").data());
    let u = mat(&e, "u");
    let first = mat(&e, "a");
    for j in 0..2 {
        assert_eq!(first.get(j, 0), u.get(0, j));
    }
}
#[test]
fn preview_and_all_noise_channels_match_physical_order_hand_loop() {
    let mut e = Engine::default();
    // Input order UD, MV, MD; output order UO, MO. D acts only on MD/UD.
    e.execute("p=ss(.7,[.3 .2 .4],[2;1],[.1 0 .2;.3 0 .4],.1);p=setmpcsignals(p,'UD',1,'MV',2,'MD',3,'MO',2,'UO',1);c=mpc(p,[],3,2);r=[0 0;1 2;2 3;3 4;4 5];v=[0;1;2;3;4];o=mpcsimopt;o.RefLookAhead='on';o.MDLookAhead='on';o.UnmeasuredDisturbance=[.1;.2];o.InputNoise=[.05;.1];o.OutputNoise=[.2;.4];[y,t,u,xp,xc]=sim(c,5,r,v,o);s=mpcstate(c);x=0;yh=zeros(5,2);uh=zeros(5,1);xh=zeros(5,1);for k=1:5;d=o.UnmeasuredDisturbance(min(k,2));ni=o.InputNoise(min(k,2));no=o.OutputNoise(min(k,2));yy=[2;1]*x+[.1;.3]*d+[.2;.4]*v(k);yh(k,:)=yy';xh(k)=x;rr=r(min((k:k+2)',5),:);vv=v(min((k:k+3)',5));uh(k)=mpcmove(c,s,yy(2)+no,rr,vv);x=.7*x+.3*d+.2*(uh(k)+ni)+.4*v(k);end;finalprior=s.Plant;loggedprior=xc(5).Plant;o.RefLookAhead='off';o.MDLookAhead='off';[yn,tn,un]=sim(c,5,r,v,o);").unwrap();
    close(&mat(&e, "y"), &mat(&e, "yh"), 1e-12);
    close(&mat(&e, "u"), &mat(&e, "uh"), 1e-12);
    close(&mat(&e, "xp"), &mat(&e, "xh"), 1e-12);
    close(&mat(&e, "finalprior"), &mat(&e, "loggedprior"), 1e-12);
    assert!((mat(&e, "u").get(0, 0) - mat(&e, "un").get(0, 0)).abs() > 1e-4);
    assert_eq!(mat(&e, "y").get(0, 1), 0.03); // OutputNoise is excluded from y.
}
#[test]
fn constraints_off_is_private_and_disables_output_and_rate_bounds() {
    let mut e = Engine::default();
    e.execute("c=mpc(ss(.8,.2,1,0,.1));c.MV.Min=-.1;c.MV.Max=.1;c.MV.RateMin=-.01;c.MV.RateMax=.01;c.OV.Min=-.05;c.OV.Max=.05;o=mpcsimopt;o.Constraints='off';[y,t,u]=sim(c,5,1,o);d=mpc(ss(.8,.2,1,0,.1));[yd,td,ud]=sim(d,5,1);[yc,tc,uc]=sim(c,5,1);bound=c.MV.Max;rate=c.MV.RateMax;ov=c.OV.Max;").unwrap();
    close(&mat(&e, "u"), &mat(&e, "ud"), 1e-12);
    assert!(mat(&e, "u").get(0, 0) > 0.1);
    assert!(mat(&e, "uc").data().iter().all(|x| *x <= 0.1 + 1e-8));
    assert_eq!(mat(&e, "bound").data(), &[0.1]);
    assert_eq!(mat(&e, "rate").data(), &[0.01]);
    assert_eq!(mat(&e, "ov").data(), &[0.05]);
}
#[test]
fn open_loop_matches_lsim_and_holds_last_mv_without_observer() {
    let mut e = Engine::default();
    e.execute("p=ss(.8,.2,1,0,.1);c=mpc(p);o=mpcsimopt;o.OpenLoop='on';o.MVSignal=[1;2];o.PlantInitialState=.3;[y,t,u,xp,xc,used]=sim(c,5,0,o);[yl,tl,xl]=lsim(p,[1;2;2;2;2],t,.3);sz=size(xc);o.MVSignal=[];[yz,tz,uz]=sim(c,2,0,o);").unwrap();
    close(&mat(&e, "y"), &mat(&e, "yl"), 1e-12);
    close(&mat(&e, "xp"), &mat(&e, "xl"), 1e-12);
    assert_eq!(mat(&e, "sz").data(), &[0., 1.]);
    assert_eq!(mat(&e, "u").data(), &[1., 2., 2., 2., 2.]);
    assert_eq!(mat(&e, "uz").data(), &[0., 0.]);
}
#[test]
fn mismatch_nominal_offsets_and_continuous_dx_are_applied_once() {
    let mut e = Engine::default();
    e.execute("c=mpc(ss(-1,1,1,0),.2);c.Model.Nominal=struct('X',2,'U',3,'Y',4,'DX',.5);o=mpcsimopt;o.OpenLoop='on';o.MVSignal=3;[y,t,u,xp]=sim(c,3,4,o);a=exp(-.2);want=[2;2+.5*(1-a);2+.5*(1-a*a)];o.Model=struct('Plant',ss(-2,1,2,0),'Nominal',struct('X',5,'U',6,'Y',7,'DX',2));o.MVSignal=6;[ym,tm,um,xm]=sim(c,3,4,o);am=exp(-.4);wantm=[5;5+1-am;5+1-am*am];wanty=7+2*(wantm-5);original=c.Model.Nominal.X;").unwrap();
    close(&mat(&e, "xp"), &mat(&e, "want"), 1e-12);
    close(&mat(&e, "xm"), &mat(&e, "wantm"), 1e-12);
    close(&mat(&e, "ym"), &mat(&e, "wanty"), 1e-12);
    assert_eq!(mat(&e, "original").data(), &[2.]);
    e.execute("c=mpc(ss(.8,.2,1,0,.1));o=mpcsimopt;o.Model=ss(.5,.4,1,0,.1);[ym,tm,um]=sim(c,5,1,o);[yn,tn,un]=sim(c,5,1);").unwrap();
    assert!((mat(&e, "ym").get(2, 0) - mat(&e, "yn").get(2, 0)).abs() > 1e-3);
    // Different state dimension resets X/DX; U/Y still inherit.
    e.execute("c.Model.Nominal=struct('X',2,'DX',.4,'U',3,'Y',4);o.OpenLoop='on';o.MVSignal=3;o.Model=ss([.5 0;0 .4],[.4;.2],[1 1],0,.1);[y,t,u,xp]=sim(c,2,4,o);").unwrap();
    assert_eq!(mat(&e, "xp").data(), &[0., 0., 0., 0.]);
    assert_eq!(mat(&e, "y").data(), &[4., 4.]);
}
#[test]
fn zero_output_plots_two_axes_and_other_sim_objects_keep_dispatch() {
    let mut e = Engine::default();
    e.execute("c=mpc(ss(.8,.2,1,0,.1));sim(c,3,1);").unwrap();
    let axes: Vec<_> = e.figure.as_ref().unwrap().all_axes().collect();
    assert_eq!(axes.len(), 2);
    assert!(axes.iter().all(|(_, a)| a.series.len() == 1));
    e.execute("c=mpc(ss([.8 .1;0 .7],[.2 0;0 .3],eye(2),zeros(2),.1));sim(c,1,[1 2]);")
        .unwrap();
    let axes: Vec<_> = e.figure.as_ref().unwrap().all_axes().collect();
    assert_eq!(axes.len(), 2);
    assert!(axes.iter().all(|(_, a)| a.series.len() == 2));
    let mut e = Engine::default();
    e.execute("n=configure(fitnet(2),[0 1],[0 1]);a=sim(n,[0 1]);b=n([0 1]);sim(n,[0 1]);d=sim(idpoly([1 -.5],[0 1]),[1;0;0]);").unwrap();
    close(&mat(&e, "a"), &mat(&e, "b"), 0.);
    assert_eq!(mat(&e, "d").data(), &[0., 1., 0.5]);
    assert!(e.figure.is_none());
}
#[test]
fn option_validation_limits_and_cancellation() {
    for (source, message) in [
        ("o=mpcsimopt;o.Mystery=1;y=sim(c,2,1,o);", "Mystery"),
        ("o=mpcsimopt;o.OpenLoop='yes';y=sim(c,2,1,o);", "OpenLoop"),
        ("o=mpcsimopt;o.StatusBar=1;y=sim(c,2,1,o);", "StatusBar"),
        (
            "o=mpcsimopt;o.Constraints='maybe';y=sim(c,2,1,o);",
            "Constraints",
        ),
        (
            "o=mpcsimopt;o.RefLookAhead='yes';y=sim(c,2,1,o);",
            "RefLookAhead",
        ),
        ("o=mpcsimopt;o.MDLookAhead=0;y=sim(c,2,1,o);", "MDLookAhead"),
        (
            "o=mpcsimopt;o.InputNoise=[1 2];y=sim(c,2,1,o);",
            "InputNoise",
        ),
        (
            "o=mpcsimopt;o.OutputNoise=Inf;y=sim(c,2,1,o);",
            "OutputNoise",
        ),
        (
            "o=mpcsimopt;o.UnmeasuredDisturbance=1;y=sim(c,2,1,o);",
            "UnmeasuredDisturbance",
        ),
        ("o=mpcsimopt;o.MVSignal=[1 2];y=sim(c,2,1,o);", "MVSignal"),
        (
            "o=mpcsimopt;o.PlantInitialState=[1;2];y=sim(c,2,1,o);",
            "PlantInitialState",
        ),
        (
            "o=mpcsimopt;o.ControllerInitialState=1;y=sim(c,2,1,o);",
            "ControllerInitialState",
        ),
        (
            "o=mpcsimopt;o.Model=struct('Plant',ss(.8,.2,1,0,.1),'Extra',1);y=sim(c,2,1,o);",
            "Extra",
        ),
        ("y=sim(c,0,1);", "sample count"),
        ("y=sim(c,1.5,1);", "sample count"),
        ("y=sim(c,65537,1);", "sample count"),
        ("y=sim(c,2,[1 2]);", "reference"),
        ("[a,b,d,f,g,h,i]=sim(c,2,1);", "six outputs"),
        ("[a,b,d,f,g]=sim(c,14000,1);", "struct slots"),
        (
            "o=mpcsimopt;o.Model=setmpcsignals(ss(.8,.2,1,0,.1),'UD',1);y=sim(c,2,1,o);",
            "manipulated",
        ),
    ] {
        let mut e = Engine::default();
        e.execute("c=mpc(ss(.8,.2,1,0,.1));").unwrap();
        let err = e.execute(source).unwrap_err();
        assert!(err.contains(message), "{source}: {err}");
    }
    let mut e = Engine::default();
    e.execute("p=ss(.8,[.2 .4],1,[0 0],.1);p=setmpcsignals(p,'MV',1,'MD',2);c=mpc(p);o=mpcsimopt;o.Model=setmpcsignals(ss(.8,[.2 .4],1,[0 0],.1),'MV',2,'MD',1);").unwrap();
    assert!(
        e.execute("y=sim(c,2,1,0,o);")
            .unwrap_err()
            .contains("signal groups")
    );
    let mut e = Engine::default();
    e.execute("c=mpc(ss(.8,.2,1,0,.1));o=mpcsimopt;o.StatusBar='on';y=sim(c,1,1,o);")
        .unwrap();
    let mut limits = e.limits_active();
    limits.memory_bytes = 50_000;
    e.limits_set(limits);
    assert!(e.execute("y=sim(c,2,1);").unwrap_err().contains("memory"));
    let mut e = Engine::default();
    e.execute("c=mpc(ss(.8,.2,1,0,.1));").unwrap();
    e.cancellation()
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(e.execute("y=sim(c,2,1);").is_err());
    assert!(e.array("y").is_none());
}
#[test]
#[ignore = "release benchmark; run with RAYON_NUM_THREADS=1 and 4"]
fn benchmark_simulation() {
    for (samples, outputs) in [(300, 3), (300, 6)] {
        let mut e = Engine::default();
        e.execute("c=mpc(ss([.8 .1;0 .7],[.2 .05;.1 .3],eye(2),zeros(2),.1));c.MV(1).Max=1.5;c.MV(2).Max=1.5;").unwrap();
        let call = if outputs == 6 {
            format!("[y,t,u,xp,xc,o]=sim(c,{samples},[1 2]);")
        } else {
            format!("[y,t,u]=sim(c,{samples},[1 2]);")
        };
        e.execute(&call).unwrap();
        let now = std::time::Instant::now();
        for _ in 0..3 {
            e.execute(&call).unwrap();
        }
        eprintln!(
            "3 x {samples} samples, {outputs} outputs, 2 states/MVs/outputs, p=10/m=2: {:?}, RAYON_NUM_THREADS={:?}; limits: 65536 samples, 128 states, 200 QP variables/4000 inequalities",
            now.elapsed(),
            std::env::var("RAYON_NUM_THREADS")
        );
    }
}

#[test]
fn defaults_and_discrete_physical_nominal_hand_loop() {
    let mut e = Engine::default();
    e.execute("p=ss(.8,[.2 .4],1,[0 .3],.1);p=setmpcsignals(p,'MV',1,'MD',2);c=mpc(p,[],3,2);c.Model.Nominal=struct('X',2,'U',[3;4],'Y',5,'DX',.1);o=mpcsimopt();oc=mpcsimopt(c);same=isequal(o,oc);defaults=[isempty(o.PlantInitialState),isempty(o.ControllerInitialState),isempty(o.UnmeasuredDisturbance),isempty(o.InputNoise),isempty(o.OutputNoise),isempty(o.Model),isempty(o.MVSignal),strcmp(o.OpenLoop,'off'),strcmp(o.StatusBar,'off'),strcmp(o.RefLookAhead,'off'),strcmp(o.MDLookAhead,'off'),strcmp(o.Constraints,'on')];[y,t,u,xp,xc,used]=sim(c,6,[],[]);s=mpcstate(c);x=2;yh=zeros(6,1);uh=zeros(6,1);for k=1:6;yh(k)=5+x-2;uh(k)=mpcmove(c,s,yh(k),[],[]);x=2+.8*(x-2)+.2*(uh(k)-3)+.1;end;one=sim(c,1,[],[]);").unwrap();
    close(&mat(&e, "y"), &mat(&e, "yh"), 1e-12);
    close(&mat(&e, "u"), &mat(&e, "uh"), 1e-12);
    assert_eq!(mat(&e, "one").data(), &[5.]);
    for name in ["same", "defaults"] {
        let Array::Logical(a) = e.array(name).unwrap() else {
            panic!("{name}")
        };
        assert!(a.data().iter().all(|&x| x == 1));
    }
}

#[test]
fn mpc_physical_weight_columns_have_scalar_sim_anchor() {
    let mut e = Engine::default();
    e.execute("p=setmpcsignals(ss(.8,.2,[1;2],[0;0],.1),'MO',2,'UO',1); c=mpc(p,.1,1,1); c.Weights.OV=[1 0]; [y,t,u]=sim(c,2,[1 0]);").unwrap();
    assert!((mat(&e, "u").get(0, 0) - 4.).abs() < 1e-6);
    assert!((mat(&e, "y").get(1, 0) - 0.8).abs() < 1e-6);
    assert!((mat(&e, "y").get(1, 1) - 1.6).abs() < 1e-6);
}
