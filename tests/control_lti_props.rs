//! Public LTI property and response behavior. Numerical references are generated
//! by tool/refs/control/lti_delays.py with NumPy, SciPy and python-control.
use matrust::{Array, Engine};
use std::collections::BTreeMap;

fn run(code: &str) -> Engine {
    let mut e = Engine::default();
    e.execute(code).unwrap();
    e
}

fn numeric(e: &Engine, name: &str) -> Vec<f64> {
    match e.array(name).unwrap() {
        Array::Numeric(m) => m.data().to_vec(),
        Array::Logical(m) => m.data().iter().map(|&v| f64::from(v)).collect(),
        value => panic!("{name} is {}", value.class()),
    }
}

fn close(e: &Engine, name: &str, expected: &[f64], tol: f64) {
    let actual = numeric(e, name);
    assert_eq!(actual.len(), expected.len(), "{name}");
    for (&a, &b) in actual.iter().zip(expected) {
        assert!(
            (a - b).abs() <= tol * (1. + b.abs()),
            "{name}: {actual:?} != {expected:?}"
        );
    }
}

fn fixture() -> BTreeMap<String, Vec<f64>> {
    serde_json::from_str(include_str!("fixtures/control/lti_delays.json")).unwrap()
}

#[test]
fn control_tf_storage_preserves_scale_and_pads_public_coefficients() {
    let mut e = run(
        "g=tf([2 0 -1],[3 1 0]); den=g.Denominator{1}; [n,d]=tfdata(g,'v'); \
         r=series(tf(1,[1 1]),tf(2,[1 2])); rn=r.Numerator{1}; \
         [rc,dc]=tfdata(r); tn=rc{1}; td=dc{1}; \
         h=tf(3,[3 3]); a=step(h,0:0.5:1); b=step(tf(1,[1 1]),0:0.5:1);",
    );
    close(&e, "den", &[3., 1., 0.], 0.);
    close(&e, "d", &[3., 1., 0.], 0.);
    close(&e, "n", &[2., 0., -1.], 0.);
    close(&e, "rn", &[0., 0., 2.], 0.);
    close(&e, "tn", &[0., 0., 2.], 0.);
    close(&e, "td", &[1., 3., 2.], 0.);
    assert_eq!(numeric(&e, "a"), numeric(&e, "b"));
    let display = e.execute("disp(g)").unwrap().output.join("\n");
    assert!(display.contains("3 s^2 + s"), "{display}");
}

#[test]
fn control_lti_scaled_siso_tf_uses_normalized_poles_for_automatic_responses() {
    let e = run(
        "g=tf(1e200,[1e200 1e200]); p=pole(g); [y,t]=step(g); last=[t(end) y(end)]; \
         [m,phase,w]=bode(g); first=[w(1) m(1) phase(1)]; \
         [gm,pm]=margin(g); infinite_margins=isinf(gm) && isinf(pm);",
    );
    // python-control pole and margins, SciPy step response, and 1/(1+jw) at w=0.1.
    close(&e, "p", &[-1.], 1e-12);
    close(&e, "last", &[7., 0.999_088_118_034_445_4], 1e-12);
    close(
        &e,
        "first",
        &[0.1, 0.995_037_190_209_989_2, -5.710_593_137_499_643],
        1e-12,
    );
    close(&e, "infinite_margins", &[1.], 0.);
}

#[test]
fn control_lti_older_value_decodes_with_default_properties() {
    use std::sync::atomic::AtomicBool;
    let mut e = run("g=tf(1,[1 1]); oldnum={1};");
    let Array::Struct(mut g) = e.array("g").unwrap() else {
        panic!("Expected TF")
    };
    let cancel = AtomicBool::new(false);
    for property in [
        "Name",
        "Notes",
        "UserData",
        "InputDelay",
        "OutputDelay",
        "IODelay",
    ] {
        g = g.without_field(property, &cancel).unwrap();
    }
    g = g
        .with_field("Numerator", e.array("oldnum").unwrap(), &cancel)
        .unwrap();
    e.insert_array("old", g.into()).unwrap();
    e.execute("n=old.Numerator{1}; d=old.InputDelay; old.InputDelay=0.5; y=step(old,0:0.5:1);")
        .unwrap();
    close(&e, "n", &[0., 1.], 0.);
    close(&e, "d", &[0.], 0.);
    close(&e, "y", &[0., 0., 1. - (-0.5_f64).exp()], 1e-12);
}

#[test]
fn control_lti_property_round_trip() {
    let e = run(
        "g=tf(1,[1 1],InputDelay=0.5,OutputDelay=0.25,IODelay=0.125,Variable='p', \
         Name='plant',InputName='force',OutputName={'position'},Notes={'first';'second'},UserData=struct('id',7)); \
         h=g; h.InputDelay=0.75; h.OutputDelay=1; h.IODelay=0; h.Variable='s'; \
         h.Name='copy'; h.InputName='target'; h.OutputName='angle'; h.Notes={'new'}; h.UserData.id=9; \
         p=get(g); set(g,'InputD',1.25,'Notes',{'edited'}); \
         hz=set(h,'Ts',0.1,'Variable','q','InputDelay',2); \
         z=zpk([],-1,2,'InputDelay',0.5,'Name','factor'); z=set(z,'Ts',-1,'InputDelay',2); \
         a=[p.InputDelay p.OutputDelay p.IODelay g.InputDelay h.InputDelay h.OutputDelay hz.InputDelay hz.Ts z.Ts z.InputDelay]; \
         texts=strcmp(p.Variable,'p') && strcmp(g.Notes{1},'edited') && strcmp(p.Notes{2},'second') \
          && strcmp(h.Name,'copy') && strcmp(h.InputName{1},'target') && strcmp(h.OutputName{1},'angle') \
          && strcmp(hz.Variable,'q') && strcmp(get(z,'name'),'factor'); \
         c={g}; set(c{1},'InputDelay',2); nested=c{1}.InputDelay; \
         data=[p.UserData.id h.UserData.id];",
    );
    close(
        &e,
        "a",
        &[0.5, 0.25, 0.125, 1.25, 0.75, 1., 2., 0.1, -1., 2.],
        0.,
    );
    close(&e, "texts", &[1.], 0.);
    close(&e, "data", &[7., 9.], 0.);
    close(&e, "nested", &[2.], 0.);
}

#[test]
fn control_lti_named_properties_survive_supported_operations() {
    let e = run(
        "s=ss(-1,1,1,0,'StateName','mode','InputName','cmd','OutputName','pos', \
         'Name','plant','InputDelay',0.5,'OutputDelay',0.25,'IODelay',0.25, \
         'Notes',{'identified'},'UserData',struct('version',3)); \
         s.StateName={'state'}; t=tf(s); s2=ss(t); \
         d=c2d(s,0.25,'zoh','Name','sampled','Variable','q'); \
         c=ss(-2,1,1,0,'StateName','controller','OutputName','result'); \
         r=series(s,c); p=parallel(s,s); f=feedback(pade(s,0),c); \
         ps=get(s); pd=get(d); pt=get(t); p2=get(s2); pr=get(r); pp=get(p); pf=get(f); \
         names=strcmp(ps.StateName{1},'state') && strcmp(pd.StateName{1},'state') \
          && strcmp(pt.Name,'plant') && strcmp(p2.Name,'plant') && strcmp(pd.Name,'sampled') \
          && strcmp(pd.Variable,'q') && strcmp(pr.InputName{1},'cmd') && strcmp(pr.OutputName{1},'result') \
          && strcmp(pf.OutputName{1},'pos') && strcmp(pf.StateName{2},'controller') \
          && strcmp(pd.Notes{1},'identified'); \
         delays=[pd.InputDelay pd.OutputDelay pd.IODelay pp.InputDelay pp.OutputDelay pp.IODelay]; \
         meta=[pr.UserData.version p2.UserData.version pd.UserData.version];",
    );
    close(&e, "names", &[1.], 0.);
    close(&e, "delays", &[2., 1., 1., 0.5, 0.25, 0.25], 0.);
    close(&e, "meta", &[3., 3., 3.], 0.);
}

#[test]
fn control_lti_inverse_variable_matches_ascending_powers() {
    let mut e = run("d=tf([1 -2 5],[1 3 4],0.2,'Variable','z^-1'); \
         a=tf(2,[1 -0.5],0.2); b=a; b.Variable='z^-1'; \
         c=tf(2,[1 -0.5],'Ts',0.2,'Variable','z^-1'); \
         [n0,d0]=tfdata(b,'v'); [n1,d1]=tfdata(c,'v'); \
         va=evalfr(a,2); vb=evalfr(b,2); vc=evalfr(c,2); ok=abs(va-vb)<1e-12 && abs(vc-2*va)<1e-12; \
         e=set(b,'num',[1 2],'den',[1 3 4]); en=e.Numerator{1};");
    let out = e.execute("disp(d)").unwrap().output.join("\n");
    assert!(out.contains("1 - 2 z^-1 + 5 z^-2"), "{out}");
    close(&e, "ok", &[1.], 0.);
    close(&e, "n0", &[0., 2.], 0.);
    close(&e, "n1", &[2., 0.], 0.);
    close(&e, "d0", &[1., -0.5], 0.);
    close(&e, "d1", &[1., -0.5], 0.);
    close(&e, "en", &[1., 2., 0.], 0.);
}

#[test]
fn control_lti_delay_time_responses_use_exact_shift() {
    let e = run("a=step(tf(1,[1 1],'InputDelay',0.5),0:0.5:1); \
         g=tf(1,[1 1],'InputDelay',0.125,'OutputDelay',0.125,'IODelay',0.125); t=0:0.25:1; \
         b=step(g,t); i=impulse(g,t); z=lsim(g,ones(1,5),t); f=lsim(g,t,t,'foh'); \
         gd=c2d(tf(1,[1 1],'InputDelay',0.5),0.25); sd=step(gd,t); samples=gd.InputDelay; \
         yd=impulse(gd,t); y0=impulse(c2d(tf(1,[1 1]),0.25),t); \
         shifted=all(yd==[0;0;y0(1:3)]);");
    let data = fixture();
    close(&e, "a", &[0., 0., 1. - (-0.5_f64).exp()], 1e-6);
    for (name, reference) in [("b", "zoh"), ("z", "zoh"), ("i", "impulse"), ("f", "foh")] {
        close(&e, name, &data[reference], 1e-12);
    }
    close(
        &e,
        "sd",
        &[0., 0., 0., 1. - (-0.25_f64).exp(), 1. - (-0.5_f64).exp()],
        1e-12,
    );
    close(&e, "samples", &[2.], 0.);
    close(&e, "shifted", &[1.], 0.);
}

#[test]
fn control_lti_output_delay_leaves_lsim_states_at_the_current_time() {
    let e = run("s=ss(-1,1,1,0,'InputDelay',0.25,'OutputDelay',0.5); \
         [y,t,x]=lsim(s,ones(1,5),0:0.25:1,2);");
    let expected_y = [
        0.,
        0.,
        2.,
        2. * (-0.25_f64).exp(),
        2. * (-0.5_f64).exp() + 1. - (-0.25_f64).exp(),
    ];
    let expected_x =
        [0_f64, 0.25, 0.5, 0.75, 1.].map(|t| 2. * (-t).exp() + 1. - (-(t - 0.25).max(0.)).exp());
    close(&e, "y", &expected_y, 1e-12);
    close(&e, "x", &expected_x, 1e-12);
}

#[test]
fn control_lti_series_static_gain_preserves_the_plant_output_delay() {
    let e = run("g=ss(-1,1,1,0,'OutputDelay',1); t=0:.5:1; u=[0 0 0]; \
         r=series(g,1); y=lsim(r,u,t,1); \
         q=series(2,g); z=lsim(q,u,t,1); delays=[r.OutputDelay q.OutputDelay];");
    // Shift the SciPy free response as in tool/refs/control/lti_mimo.py.
    close(&e, "y", &[0., 0., 1.], 1e-12);
    close(&e, "z", &[0., 0., 1.], 1e-12);
    close(&e, "delays", &[1., 1.], 0.);
}

#[test]
fn control_lti_delay_phase_nyquist_freqresp_and_margin() {
    let e = run("g=tf(2,[1 1],'InputDelay',0.5); [mag,phase]=bode(g,12); \
         r=freqresp(g,12); [x,y]=nyquist(g,12); \
         fr=[mag phase real(r) imag(r)]; ny=[x y]; \
         [gm,pm,wcg,wcp]=margin(g); margins=[gm pm wcg wcp]; \
         q=c2d(g,0.25); u=c2d(tf(2,[1 1]),0.25); \
         zr=freqresp(q,3)/freqresp(u,3); err=abs(zr-exp(-1.5i));");
    let data = fixture();
    close(&e, "fr", &data["frequency"], 1e-12);
    close(&e, "ny", &data["frequency"][2..], 1e-12);
    close(&e, "margins", &data["margin"], 1e-9);
    close(&e, "err", &[0.], 1e-12);
}

#[test]
fn control_lti_pade_and_damp_outputs() {
    let mut e = run("[n,d]=pade(0.5,1); [n3,d3]=pade(0.5,3); \
         g=tf(1,[1 1],'InputDelay',0.5,'Name','g'); r=pade(g,1); \
         [rn,rd]=tfdata(r,'v'); same=strcmp(r.Name,g.Name); delay=r.InputDelay; \
         a=pade(tf(1,1,'InputDelay',0.5,'OutputDelay',0.25),1); [an,ad]=tfdata(a,'v'); \
         [w,z,p]=damp(tf(1,[1 3 2])); ans=17;");
    let data = fixture();
    close(&e, "n", &[-1., 4.], 0.);
    close(&e, "d", &[1., 4.], 0.);
    close(&e, "n3", &data["pade_num"], 1e-14);
    close(&e, "d3", &data["pade_den"], 1e-14);
    close(&e, "rn", &[0., -1., 4.], 0.);
    close(&e, "rd", &[1., 5., 4.], 0.);
    close(&e, "an", &[1., -12., 32.], 0.);
    close(&e, "ad", &[1., 12., 32.], 0.);
    close(&e, "same", &[1.], 0.);
    close(&e, "delay", &[0.], 0.);
    close(&e, "w", &[1., 2.], 1e-12);
    close(&e, "z", &[1., 1.], 1e-12);
    close(&e, "p", &[-1., -2.], 1e-12);
    let table = e
        .execute("damp(tf([2 5 1],[1 0 2 -3]));")
        .unwrap()
        .output
        .join("\n");
    // Complete tables from the continuous and discrete examples on the damp page.
    assert_eq!(
        table,
        concat!(
            "         Pole              Damping       Frequency      Time Constant\n",
            "                                       (rad/seconds)      (seconds)\n\n",
            "  1.00e+00                -1.00e+00       1.00e+00        -1.00e+00\n",
            " -5.00e-01 + 1.66e+00i     2.89e-01       1.73e+00         2.00e+00\n",
            " -5.00e-01 - 1.66e+00i     2.89e-01       1.73e+00         2.00e+00",
        )
    );
    let discrete = e
        .execute("damp(tf([5 3 1],[1 6 4 4],0.01));")
        .unwrap()
        .output
        .join("\n");
    assert_eq!(
        discrete,
        concat!(
            "         Pole             Magnitude     Damping       Frequency      Time Constant\n",
            "                                                    (rad/seconds)      (seconds)\n\n",
            " -3.02e-01 + 8.06e-01i     8.61e-01     7.74e-02       1.93e+02         6.68e-02\n",
            " -3.02e-01 - 8.06e-01i     8.61e-01     7.74e-02       1.93e+02         6.68e-02\n",
            " -5.40e+00                 5.40e+00    -4.73e-01       3.57e+02        -5.93e-03",
        )
    );
    close(&e, "ans", &[17.], 0.);
    let out = e
        .execute("disp(g); disp(c2d(g,0.25));")
        .unwrap()
        .output
        .join("\n");
    for part in ["Input delay: 0.5", "z^-2"] {
        assert!(out.contains(part), "{part}: {out}");
    }
}

#[test]
fn control_lti_pade_output_filter_preserves_the_named_plant_state() {
    let e = run(
        "g=ss(-1,1,1,0,'OutputDelay',1,'StateName','x'); r=pade(g,1); \
         [y,t,x]=step(r,[0 .5]); names=r.StateName; \
         plant_state=x(:,strcmp(names,'x'));",
    );
    // python-control step response of the original plant.
    close(&e, "plant_state", &[0., 0.393_469_340_287_366_6], 1e-12);
}

#[test]
fn control_lti_unequal_parallel_delays_preserve_each_path() {
    let mut e = run(
        "a=tf(1,[1 1],'InputDelay',0.5); b=tf(2,[1 2],'OutputDelay',0.25); \
         p=parallel(a,b); t=0:0.25:2; w=[0.1 1 4]; \
         dy=max(abs(step(p,t)-step(a,t)-step(b,t))); \
         fi=max(abs(freqresp(p,w)-freqresp(a,w)-freqresp(b,w))); \
         q=series(tf(1,[1 3]),p); \
         fs=max(abs(freqresp(q,w)-freqresp(tf(1,[1 3]),w).*freqresp(p,w))); \
         pd=c2d(p,0.25); ad=c2d(a,0.25); bd=c2d(b,0.25); \
         dd=max(abs(step(pd,t)-step(ad,t)-step(bd,t))); \
         pa=pade(p,2); fp=max(abs(freqresp(pa,w)-freqresp(pade(a,2),w)-freqresp(pade(b,2),w))); \
         class_ok=strcmp(class(p),'ss');",
    );
    for name in ["dy", "fi", "fs", "dd", "fp"] {
        close(&e, name, &[0.], 1e-12);
    }
    close(&e, "class_ok", &[1.], 0.);
    let out = e.execute("disp(p)").unwrap().output.join("\n");
    assert!(out.contains("Internal delays (seconds)"), "{out}");
    assert!(
        e.execute("f=tf(p)")
            .unwrap_err()
            .contains("internal delays")
    );
}

#[test]
fn control_lti_validation_rejects_invalid_properties_without_mutation() {
    let mut e = run("g=tf(1,[1 1],'InputDelay',0.5);");
    for (code, diagnostic) in [
        ("g.InputDelay=-1", "nonnegative"),
        ("g.OutputDelay=Inf", "finite"),
        ("g.IODelay=[1 2]", "real scalar"),
        ("g.Ts=0.1", "integer number"),
        ("g.Ts=-2", "sample time"),
        ("g.Variable='z'", "continuous-time"),
        ("g.Name=42", "scalar text"),
        ("g.InputName={'a','b'}", "name"),
        ("g.Notes=7", "text"),
        ("g.StateName='x'", "property"),
        ("s=ss(-1,1,1,0,'StateName',{'a','b'})", "one name per state"),
        ("set(g,'InputDelay',1,'OutputDelay',-1)", "nonnegative"),
        ("q=c2d(g,0.2)", "fractional InputDelay"),
        ("q=feedback(g,1)", "pade(sys,N)"),
        ("[n,d]=pade(0.5,1.25)", "integer"),
    ] {
        let error = e.execute(code).unwrap_err();
        assert!(error.contains(diagnostic), "{code}: {error}");
    }
    e.execute("delay=g.InputDelay; ts=g.Ts;").unwrap();
    close(&e, "delay", &[0.5], 0.);
    close(&e, "ts", &[0.], 0.);
}

#[test]
fn control_lti_delayed_margin_wide_rational_band() {
    let e = run(
        "a=tf(5,conv([1 1],[1 200]),'InputDelay',2); [g,p,wg,wp]=margin(a); low=[g wg]; \
         b=tf(1000,[1 100 0],'InputDelay',10); [g2,p2,wg2,wp2]=margin(b); integrator=[g2 p2 wg2 wp2];",
    );
    let data = fixture();
    close(&e, "low", &data["low_gain_margin"], 1e-9);
    assert_eq!(numeric(&e, "p"), [f64::INFINITY]);
    assert!(numeric(&e, "wp")[0].is_nan());
    close(&e, "integrator", &data["integrator_margin"], 1e-9);
}

#[test]
fn control_lti_unspecified_sample_time_design_and_plots() {
    let e = run("s=ss(0.5,1,1,0,-1); k=lqr(s,1,1); kd=dlqr(0.5,1,1,1); \
         [m,p,w]=bode(s,{0.2,20}); endpoints=[w(1) w(end)]; \
         a=ss(0.5*eye(2),eye(2),eye(2),zeros(2),-1); v=allmargin(a); \
         wg=v(1).GMFrequency; gm=v(2).GainMargin; pzmap(s);");
    // Scalar DARE gives P^2 - P/4 - 1 = 0, K = P/(2+2P).
    let p = (1. + 65_f64.sqrt()) / 8.;
    close(&e, "k", &[p / (2. + 2. * p)], 1e-12);
    close(&e, "k", &numeric(&e, "kd"), 1e-12);
    close(&e, "endpoints", &[0.2, std::f64::consts::PI], 1e-14);
    close(&e, "wg", &[std::f64::consts::PI], 1e-12);
    close(&e, "gm", &[1.5], 1e-12);
    let axes = e.figure.as_ref().unwrap().all_axes().next().unwrap().1;
    assert!(
        axes.series
            .iter()
            .any(|s| s.pattern == matrust::plot::LinePattern::Dotted)
    );
}

#[test]
fn control_lti_inverse_zpk_preserves_factors_and_relative_degree() {
    let mut e = run("g=zpk([0.5 0.5],[-1i 1i 0],2,0.1,'Variable','z^-1'); \
         h=zpk([0 0],0.25,3,1,'Variable','q^-1');");
    let table = e.execute("disp(g)").unwrap().output.join("\n");
    assert!(table.contains("2 z^-1 (1-0.5z^-1)^2"), "{table}");
    assert!(table.contains("(1+z^-2)"), "{table}");
    let table = e.execute("disp(h)").unwrap().output.join("\n");
    assert!(table.contains("q^-1 (1-0.25q^-1)"), "{table}");
}

macro_rules! rejects_delayed_model {
    ($test:ident, $code:literal) => {
        #[test]
        fn $test() {
            let mut e = run("c=mpc(tf(1,[1 1]),0.1);");
            for property in ["InputDelay", "OutputDelay", "IODelay"] {
                e.execute(&format!(
                    "g=tf(1,[1 1],'{property}',0.5); d=tf(1,[1 -0.5],1,'{property}',1);"
                ))
                .unwrap();
                let error = e.execute($code).unwrap_err();
                assert!(
                    error.contains("delay") && error.contains("pade(sys,N)"),
                    "{} with {property}: {error}",
                    $code
                );
            }
        }
    };
}

rejects_delayed_model!(pidtune_delay, "r=pidtune(g,'PI')");
rejects_delayed_model!(rlocus_delay, "r=rlocus(g,2)");
rejects_delayed_model!(idtf_delay, "r=idtf(g)");
rejects_delayed_model!(idpoly_delay, "r=idpoly(d)");
rejects_delayed_model!(idss_delay, "r=idss(g)");
rejects_delayed_model!(mpc_plant_delay, "r=mpc(g,0.1)");
rejects_delayed_model!(mpc_noise_delay, "c.Model.Noise=g");
rejects_delayed_model!(mpc_disturbance_delay, "c.Model.Disturbance=g");
rejects_delayed_model!(mpc_output_disturbance_delay, "setoutdist(c,'model',g)");
rejects_delayed_model!(mpc_input_disturbance_delay, "setindist(c,'model',g)");
rejects_delayed_model!(ctrb_delay, "r=ctrb(g)");
rejects_delayed_model!(obsv_delay, "r=obsv(g)");
rejects_delayed_model!(lqr_delay, "r=lqr(g,1,1)");
rejects_delayed_model!(ssdata_external_delay, "[a,b,c,d]=ssdata(g)");
rejects_delayed_model!(gram_delay, "r=gram(g,'c')");
rejects_delayed_model!(hsvd_delay, "r=hsvd(g)");
rejects_delayed_model!(balreal_delay, "r=balreal(g)");
rejects_delayed_model!(balred_delay, "r=balred(g,0)");
rejects_delayed_model!(modred_delay, "r=modred(g,1)");

#[test]
fn control_lti_margin_resolves_several_delay_turns_in_one_bracket() {
    let e = run(
        "g=tf(0.5,1,'InputDelay',0.001); [gm,pm,wg,wp]=margin(g); a=allmargin(g); turns=a.GMFrequency;",
    );
    let first = std::f64::consts::PI / 0.001;
    close(&e, "gm", &[2.], 1e-12);
    close(&e, "wg", &[first], 1e-12);
    let expected: Vec<_> = (0..8).map(|k| first * (2 * k + 1) as f64).collect();
    close(&e, "turns", &expected, 1e-12);
}
