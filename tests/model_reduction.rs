//! gram, hsvd, balreal, balred, modred and filtord. Expected numbers come from
//! SciPy/NumPy (solve_continuous_lyapunov, solve_discrete_lyapunov,
//! sqrt(eig(Wc*Wo)), an independent NumPy balanced truncation/residualization);
//! regenerate with tool/refs/model_reduction/model_reduction.py. Never MATLAB.
use matrust::{Array, Engine, engine::Limits};

const SISO: &str = "A=[-2 1 0 0.5;0 -3 1 0;0.2 0 -1.5 1;0 0.4 0 -4]; B=[1;0;1;0.5]; \
                    C=[1 0.5 0 1]; D=0; sys=ss(A,B,C,D);";
const MIMO: &str = "A=[-1 0.5 0 0 0.1;0 -2 1 0 0;0 0 -3 0.5 0;0.3 0 0 -4 1;0 0 0.2 0 -0.5]; \
                    B=[1 0;0 1;1 0.5;0 0.2;0.4 0]; C=[1 0 0.5 0 0.2;0 1 0 1 0.3]; \
                    D=[0.1 0;0 0.2]; sys=ss(A,B,C,D);";
const DISC: &str = "A=[0.5 0.2 0 0;-0.1 0.6 0.1 0;0 0 0.3 0.4;0 0.05 0 -0.2]; B=[1;0.5;0;1]; \
                    C=[1 0 1 0.5]; D=0.05; sys=ss(A,B,C,D,0.1);";
const UNSTABLE: &str = "sys=zpk([-1 -4],[1 -2 -3 -6],2);";

#[rustfmt::skip]
const SISO_WC: [f64; 16] = [0.31590937484746295, 0.08449836397359965, 0.37740322351991873, 0.09464077144265233, 0.08449836397359965, 0.0363170329830921, 0.10895109894927626, 0.017543126729975097, 0.37740322351991873, 0.1089510989492763, 0.45583714574704826, 0.10827507391658883, 0.09464077144265233, 0.017543126729975107, 0.10827507391658882, 0.03300431267299751];
#[rustfmt::skip]
const SISO_WO: [f64; 16] = [0.25495983921234294, 0.16834663329612176, 0.04959839212342446, 0.19725429641117068, 0.1683466332961218, 0.11461787023713571, 0.03935804351898091, 0.1262674435382122, 0.04959839212342444, 0.03935804351898095, 0.02623869567932056, 0.03223733368713544, 0.19725429641117065, 0.12626744353821223, 0.032237333687135446, 0.15771612047318023];
#[rustfmt::skip]
const SISO_HSV: [f64; 4] = [0.4741537410686669, 0.01616419054480553, 0.0009415132076672834, 7.643565862910834e-07];
#[rustfmt::skip]
const MIMO_WC: [f64; 25] = [0.6099935817596197, 0.15170922541061543, 0.28612965393741757, 0.11234126016938643, 0.34138969054312146, 0.15170922541061543, 0.3246450692106666, 0.14929013842133368, 0.06010797746157239, 0.06675487689094901, 0.28612965393741746, 0.1492901384213336, 0.21639670337588296, 0.04838022025530078, 0.13702916454310554, 0.11234126016938643, 0.060107977461572405, 0.048380220255300796, 0.0315869621255506, 0.07264547045138643, 0.34138969054312146, 0.06675487689094903, 0.1370291645431055, 0.07264547045138642, 0.21481166581724231];
#[rustfmt::skip]
const MIMO_WO: [f64; 25] = [0.5072254756271081, 0.10207868929418587, 0.1633177705115568, 0.024084918757023878, 0.2053336225865717, 0.10207868929418584, 0.27551967232354657, 0.08084828400310846, 0.1754111002300111, 0.2353143121810863, 0.16331777051155683, 0.08084828400310849, 0.07745169374703535, 0.033752227449087256, 0.13253398618998727, 0.024084918757023895, 0.1754111002300111, 0.03375222744908723, 0.12921902843113592, 0.11064322520040715, 0.20533362258657173, 0.23531431218108625, 0.13253398618998724, 0.11064322520040713, 0.39235317491812916];
#[rustfmt::skip]
const MIMO_HSV: [f64; 5] = [0.9219516859737091, 0.23124229611554759, 0.04933609396253211, 0.020366174062869807, 0.002522294419101795];
#[rustfmt::skip]
const DISC_WC: [f64; 16] = [1.5252584480339475, 0.6594729330341337, 0.26430832186085923, 0.9097485036456399, 0.6594729330341336, 0.3012312354658481, 0.08601349186644243, 0.46939454175769785, 0.26430832186085923, 0.08601349186644247, 0.16367055951434745, -0.06786384187859308, 0.9097485036456396, 0.46939454175769796, -0.06786384187859308, 1.0326720700557406];
#[rustfmt::skip]
const DISC_WO: [f64; 16] = [1.3105551822400034, 0.18921369309839947, 1.1813431582563492, 0.6668074658047494, 0.18921369309839942, 0.1837755989842677, 0.11604147157268342, 0.09306197984309161, 1.1813431582563492, 0.11604147157268348, 1.108571697015608, 0.5998198329319986, 0.6668074658047494, 0.09306197984309163, 0.5998198329319986, 0.345208644013935];
#[rustfmt::skip]
const DISC_HSV: [f64; 4] = [2.166365104983416, 0.10996902529810008, 0.012925903754756008, 0.005856285805189876];
#[rustfmt::skip]
const UNSTABLE_HSV: [f64; 4] = [f64::INFINITY, 0.026232704560680862, 0.01808101392333521, 0.00021518270083770185];

fn run(code: &str) -> Engine {
    let mut e = Engine::default();
    e.execute(code)
        .unwrap_or_else(|err| panic!("{err}\n{code}"));
    e
}
fn parts(e: &Engine, name: &str) -> (usize, usize, Vec<(f64, f64)>) {
    let a = e.array(name).unwrap_or_else(|| panic!("missing {name}"));
    let data = match &a {
        Array::Numeric(m) => m.data().iter().map(|&x| (x, 0.)).collect(),
        Array::Complex(m) => m.data().iter().map(|x| (x.re, x.im)).collect(),
        _ => panic!("{name}: expected a numeric array, got {}", a.class()),
    };
    (a.rows(), a.cols(), data)
}
fn real(e: &Engine, name: &str) -> Vec<f64> {
    parts(e, name).2.into_iter().map(|(x, _)| x).collect()
}
fn scalar(e: &Engine, name: &str) -> f64 {
    let v = real(e, name);
    assert_eq!(v.len(), 1, "{name}");
    v[0]
}
fn close(actual: &[f64], expected: &[f64], tol: f64, what: &str) {
    assert_eq!(actual.len(), expected.len(), "{what}: length");
    for (i, (a, b)) in actual.iter().zip(expected).enumerate() {
        let ok = if b.is_infinite() {
            a == b
        } else {
            (a - b).abs() <= tol * b.abs().max(1.)
        };
        assert!(ok, "{what}[{i}]: {a} vs {b}");
    }
}
fn close_complex(e: &Engine, name: &str, re: &[f64], im: &[f64], tol: f64) {
    let (_, _, v) = parts(e, name);
    let (a_re, a_im): (Vec<f64>, Vec<f64>) = v.into_iter().unzip();
    close(&a_re, re, tol, &format!("{name} re"));
    close(&a_im, im, tol, &format!("{name} im"));
}

#[test]
fn gramians_match_scipy_and_solve_the_lyapunov_equations() {
    for (setup, wc, wo, discrete) in [
        (SISO, &SISO_WC[..], &SISO_WO[..], false),
        (MIMO, &MIMO_WC[..], &MIMO_WO[..], false),
        (DISC, &DISC_WC[..], &DISC_WO[..], true),
    ] {
        let residuals = if discrete {
            "rc=norm(A*Wc*A'-Wc+B*B',1); ro=norm(A'*Wo*A-Wo+C'*C,1);"
        } else {
            "rc=norm(A*Wc+Wc*A'+B*B',1); ro=norm(A'*Wo+Wo*A+C'*C,1);"
        };
        let e = run(&format!(
            "{setup} Wc=gram(sys,'c'); Wo=gram(sys,'o'); {residuals} \
             sc=norm(Wc-Wc',1); so=norm(Wo-Wo',1);"
        ));
        let n = (wc.len() as f64).sqrt() as usize;
        assert_eq!(parts(&e, "Wc").0, n);
        assert_eq!(parts(&e, "Wc").1, n);
        close(&real(&e, "Wc"), wc, 1e-10, "Wc");
        close(&real(&e, "Wo"), wo, 1e-10, "Wo");
        for r in ["rc", "ro", "sc", "so"] {
            assert!(scalar(&e, r) < 1e-10, "{setup} {r} = {}", scalar(&e, r));
        }
    }
    // tf input is realized first; the gramian of 1/(s+1) is 1/2.
    let e = run("w=gram(tf(1,[1 1]),'c'); v=gram(tf(1,[1 1]),'o');");
    close(&real(&e, "w"), &[0.5], 1e-12, "first order");
    close(&real(&e, "v"), &[0.5], 1e-12, "first order");
}

#[test]
fn hsvd_matches_numpy_and_reports_unstable_modes_as_inf() {
    for (setup, hsv) in [
        (SISO, &SISO_HSV[..]),
        (MIMO, &MIMO_HSV[..]),
        (DISC, &DISC_HSV[..]),
        (UNSTABLE, &UNSTABLE_HSV[..]),
    ] {
        let e = run(&format!("{setup} h=hsvd(sys);"));
        let (rows, cols, _) = parts(&e, "h");
        assert_eq!((rows, cols), (hsv.len(), 1), "{setup}");
        close(&real(&e, "h"), hsv, 1e-10, setup);
    }
    // Offset moves the boundary: with Offset 0.6 only |p| < 0.4 counts as
    // stable, so the pair with |p| = 0.567 is reported as Inf.
    let e = run(&format!("{DISC} h=hsvd(sys,'Offset',0.6);"));
    let h = real(&e, "h");
    assert_eq!(h.iter().filter(|x| x.is_infinite()).count(), 2, "{h:?}");
    assert!(h[..2].iter().all(|x| x.is_infinite()));
}

#[test]
fn balreal_gives_equal_diagonal_gramians_and_consistent_transforms() {
    for setup in [SISO, MIMO, DISC] {
        let e = run(&format!(
            "{setup} [sb,g,T,Ti]=balreal(sys); h=hsvd(sys); [Ab,Bb,Cb,Db]=ssdata(sb); \
             ec=norm(gram(sb,'c')-diag(g),1); eo=norm(gram(sb,'o')-diag(g),1); \
             ei=norm(T*Ti-eye(size(A,1)),1); ea=norm(T*A*Ti-Ab,1); eb=norm(T*B-Bb,1); \
             ecc=norm(C*Ti-Cb,1); ed=norm(D-Db,1);"
        ));
        close(&real(&e, "g"), &real(&e, "h"), 1e-12, "g = hsvd");
        for r in ["ec", "eo", "ei", "ea", "eb", "ecc", "ed"] {
            assert!(scalar(&e, r) < 1e-9, "{setup} {r} = {}", scalar(&e, r));
        }
    }
    // Unstable modes come first and are left as they are, with g = Inf.
    let e = run(&format!(
        "{UNSTABLE} [sb,g,T,Ti]=balreal(sys); [Ab,Bb,Cb,Db]=ssdata(sb); [A,B,C,D]=ssdata(sys); \
         p=Ab(1,1); ei=norm(T*Ti-eye(4),1); ea=norm(T*A*Ti-Ab,1); \
         s=0.7i; ef=abs(evalfr(sb,s)-evalfr(sys,s)); \
         gs=gram(ss(Ab(2:4,2:4),Bb(2:4),Cb(2:4),0),'c'); egs=norm(gs-diag(g(2:4)),1);"
    ));
    close(&real(&e, "g"), &UNSTABLE_HSV, 1e-10, "unstable g");
    close(&real(&e, "p"), &[1.], 1e-10, "unstable pole kept");
    for r in ["ei", "ea", "ef", "egs"] {
        assert!(scalar(&e, r) < 1e-9, "unstable {r} = {}", scalar(&e, r));
    }
}

#[test]
#[rustfmt::skip]
fn balred_and_modred_match_numpy_reduced_models() {
    let w = "w=[0 0.5 2 10];";
    let e = run(&format!(
        "{SISO} {w} r=balred(sys,2); t=balred(sys,2,'StateElimMethod','Truncate'); \
         m=modred(sys,[3 4]); mt=modred(sys,logical([0 0 1 1]),'Truncate'); \
         for k=1:4, gr(k)=evalfr(r,1i*w(k)); gt(k)=evalfr(t,1i*w(k)); \
         gm(k)=evalfr(m,1i*w(k)); gmt(k)=evalfr(mt,1i*w(k)); end; nr=order(r); nm=order(m);"
    ));
    assert_eq!(scalar(&e, "nr"), 2.);
    assert_eq!(scalar(&e, "nm"), 2.);
    close_complex(&e, "gr", &[0.9787543655413273, 0.8473827980562927, 0.312150089609526, 0.030831484383953323], &[0.0, -0.3240956952529582, -0.42418569924386723, -0.14232672421267978], 1e-10);
    close_complex(&e, "gt", &[0.9806358632269458, 0.8466408094108931, 0.31364377032138857, 0.029513542729420056], &[0.0, -0.3258238231717352, -0.42305272150031203, -0.14099476354162666], 1e-10);
    close_complex(&e, "gm", &[0.9787543655413271, 0.9188770711498346, 0.5066174421844525, 0.15019588393623862], &[0.0, -0.21947293527138026, -0.42874059626636946, -0.14725270199220947], 1e-10);
    close_complex(&e, "gmt", &[0.5, 0.47058823529411764, 0.25, 0.019230769230769232], &[0.0, -0.11764705882352941, -0.25, -0.09615384615384615], 1e-10);

    let e = run(&format!("{MIMO} r=balred(sys,2); g1=evalfr(r,1i); dc=dcgain(r);"));
    close_complex(&e, "g1", &[0.7260006280645337, 0.14749979105813005, 0.13687139256133868, 0.6642065672759737], &[-0.7087229265948489, -0.41983430998079685, -0.1942056222482373, -0.26960685058503525], 1e-10);
    close(&real(&e, "dc"), &[1.6775953727725095, 0.808829158337941, 0.4081784544719944, 0.9036107685110366], 1e-10, "mimo dc");

    let e = run(&format!(
        "{DISC} {w} r=balred(sys,2); t=balred(sys,2,'StateElimMethod','Truncate'); \
         for k=1:4, z=exp(1i*0.1*w(k)); gr(k)=evalfr(r,z); gt(k)=evalfr(t,z); end; ts=r.Ts;"
    ));
    assert_eq!(scalar(&e, "ts"), 0.1);
    close_complex(&e, "gr", &[3.296055495103376, 3.2714419110961175, 2.9292224187616442, 0.009507461215822799], &[0.0, -0.3296402381260829, -1.2220253131962977, -1.9088245403107513], 1e-10);
    close_complex(&e, "gt", &[3.312838572301605, 3.2871388190647295, 2.9317445881393045, 0.020602704129998848], &[0.0, -0.33558093182545107, -1.2386231088806463, -1.8963448711903284], 1e-10);

    // The unstable pole at 1 is kept; one stable state is left (MatchDC).
    let e = run(&format!(
        "{UNSTABLE} r=balred(sys,2); w=[0 0.5 2]; for k=1:3, g(k)=evalfr(r,1i*w(k)); end; \
         p=sort(real(pole(r)));"
    ));
    close_complex(&e, "g", &[-0.22222222222222227, -0.18125330931676276, -0.06842010134358273], &[0.0, -0.11269311591272349, -0.11930327100588871], 1e-10);
    let p = real(&e, "p");
    assert_eq!(p.len(), 2);
    assert!((p[1] - 1.).abs() < 1e-10 && p[0] < 0., "{p:?}");
}

#[test]
fn balred_error_is_within_twice_the_discarded_hankel_singular_values() {
    for (setup, n) in [(SISO, 4), (MIMO, 5), (DISC, 4)] {
        for method in ["MatchDC", "Truncate"] {
            for r in 1..n {
                let e = run(&format!(
                    "{setup} h=hsvd(sys); rs=balred(sys,{r},'StateElimMethod','{method}'); \
                     bound=2*sum(h({r}+1:end)); w=[0 logspace(-3,3,300)]; err=0; \
                     for k=1:numel(w), if sys.Ts>0, s=exp(1i*w(k)*sys.Ts); else, s=1i*w(k); end; \
                     err=max(err,norm(evalfr(sys,s)-evalfr(rs,s))); end"
                ));
                let (err, bound) = (scalar(&e, "err"), scalar(&e, "bound"));
                // The bound is attained when one state is discarded; allow
                // the absolute rounding of the smallest singular value.
                assert!(
                    err <= bound * (1. + 1e-9) + 1e-12,
                    "{setup} {method} r={r}: {err} > {bound}"
                );
                assert!(err > 0.);
            }
        }
    }
}

#[test]
fn match_dc_keeps_the_dc_gain() {
    for setup in [SISO, MIMO, DISC] {
        for r in 1..4 {
            let e = run(&format!(
                "{setup} rs=balred(sys,{r}); m=modred(sys,{r}+1:size(A,1)); \
                 e1=norm(dcgain(rs)-dcgain(sys),1); e2=norm(dcgain(m)-dcgain(sys),1);"
            ));
            for x in ["e1", "e2"] {
                assert!(
                    scalar(&e, x) < 1e-10,
                    "{setup} r={r} {x} = {}",
                    scalar(&e, x)
                );
            }
        }
    }
}

#[test]
fn filtord_counts_fir_iir_sos_and_digital_filter_orders() {
    let e = run(
        "[b,a]=butter(5,0.3); n1=filtord(b,a); n2=filtord(fir1(20,0.3)); \
         [z,p,k]=butter(6,0.25); sos=zp2sos(z,p,k); n3=filtord(sos); \
         n4=filtord([1 2 0 0],[1 0.5]); n5=filtord([0 0 1]); n6=filtord(1,[1 -0.9 0 0.2]); \
         n7=filtord([1 1 0 1 0.5 0; 1 0 1 1 0 0.25]); \
         d=designfilt('lowpassfir','FilterOrder',20,'CutoffFrequency',0.4); n8=filtord(d); \
         d=designfilt('lowpassiir','FilterOrder',6,'HalfPowerFrequency',0.3); n9=filtord(d);",
    );
    // SciPy: butter(5) has 6 denominator taps, firwin(21) 21 taps, butter(6, sos) 3 sections.
    for (name, expected) in [
        ("n1", 5.),
        ("n2", 20.),
        ("n3", 6.),
        ("n4", 1.),
        ("n5", 2.),
        ("n6", 3.),
        ("n7", 3.),
        ("n8", 20.),
        ("n9", 6.),
    ] {
        assert_eq!(scalar(&e, name), expected, "{name}");
    }
}

#[test]
fn reduction_errors_are_explicit() {
    for (code, message) in [
        ("gram(tf(1,[1 -1]),'c')", "must be stable"),
        ("gram(tf(1,[1 1]))", "not enough input arguments"),
        ("gram(tf(1,[1 1]),'x')", "TYPE must be 'c' or 'o'"),
        ("gram([1 2],'c')", "tf, zpk or ss model"),
        ("hsvd(tf(1,[1 1]),'Foo',1)", "unknown option 'Foo'"),
        ("[a,b]=hsvd(tf(1,[1 1]))", "not supported yet"),
        ("balred(tf(1,[1 3 2]),[1 2])", "model array"),
        ("balred(tf(1,[1 3 2]),3)", "must not exceed the model order"),
        ("balred(tf(1,[1 3 2]),1.5)", "nonnegative integer"),
        ("balred(zpk([],[1 -2],1),0)", "unstable modes"),
        (
            "balred(tf(1,[1 3 2]),1,'StateElimMethod','bad')",
            "unknown state elimination method",
        ),
        (
            "balred(tf(1,[1 3 2]),1,'Bogus',1)",
            "unknown option 'Bogus'",
        ),
        ("modred(ss(-1,1,1,0),3)", "between 1 and 1"),
        ("modred(ss([-1 0;0 0],[1;1],[1 1],0),2)", "singular"),
        ("balreal(ss([-1 0;0 -2],[1;0],[1 0],0))", "not minimal"),
        ("[a,b]=balred(tf(1,[1 1]),1)", "too many output arguments"),
        ("filtord(ones(2,3))", "must be a vector"),
        ("filtord([1 2],[0 1])", "first denominator coefficient"),
        ("filtord()", "not enough input arguments"),
    ] {
        let err = Engine::default()
            .execute(code)
            .err()
            .unwrap_or_else(|| panic!("{code} should fail"))
            .to_string();
        assert!(err.contains(message), "{code}: {err}");
    }
}

fn chars(e: &Engine, name: &str) -> String {
    match e.array(name).unwrap_or_else(|| panic!("missing {name}")) {
        Array::Character(c) => c.to_utf8(&e.cancellation()).expect(name),
        other => panic!("{name}: {}", other.class()),
    }
}

#[test]
fn review_b6_balred_options_struct_is_accepted_by_balred() {
    let mut e = Engine::default();
    let out = e
        .execute("opts = balredOptions('StateProjection','Truncate')")
        .unwrap_or_else(|err| panic!("{err}"));
    let shown = out.output.join("\n");
    assert!(shown.contains("StateProjection"), "{shown}");
    assert!(shown.contains("Truncate"), "{shown}");
    e.execute("method = opts.StateProjection; d = balredOptions; dm = d.StateProjection; a = balredOptions('StateElimMethod','Truncate'); am = a.StateProjection;")
        .unwrap();
    assert_eq!(chars(&e, "method"), "Truncate");
    assert_eq!(chars(&e, "dm"), "MatchDC");
    assert_eq!(chars(&e, "am"), "Truncate");
    // Last name wins when both aliases are set. A foreign name is an error.
    e.execute("both = balredOptions('StateElimMethod','Truncate','StateProjection','MatchDC'); bm = both.StateProjection;")
        .unwrap();
    assert_eq!(chars(&e, "bm"), "MatchDC");
    for (code, message) in [
        (
            "balredOptions('FreqIntervals',[1 2])",
            "unknown option 'FreqIntervals'",
        ),
        (
            "balredOptions('StateProjection','AbsTol')",
            "use 'MatchDC' or 'Truncate'",
        ),
        ("balredOptions('StateProjection')", "name-value pairs"),
        ("balred(tf(1,[1 1]),0,struct('A',1))", "unknown option 'A'"),
        (
            "balred(tf(1,[1 1]),0,balredOptions,'Offset',1e-6)",
            "not both",
        ),
    ] {
        let err = Engine::default().execute(code).expect_err(code).to_string();
        assert!(err.contains(message), "{code}: {err}");
    }
    // The options form matches the older name/value form, including the default.
    let e = run("sys = tf(1,[1 6 11 6]); \
         old = balred(sys,1,'StateElimMethod','Truncate'); \
         neu = balred(sys,1,balredOptions('StateProjection','Truncate')); \
         ali = balred(sys,1,balredOptions('StateElimMethod','Truncate')); \
         def = balred(sys,1,balredOptions); \
         plain = balred(sys,1); \
         ao = old.A; an = neu.A; aa = ali.A; ad = def.A; ap = plain.A;");
    let truncated = real(&e, "ao");
    assert_eq!(truncated, real(&e, "an"));
    assert_eq!(truncated, real(&e, "aa"));
    assert_ne!(truncated, real(&e, "ap"));
    assert_eq!(real(&e, "ad"), real(&e, "ap"));
}

#[test]
fn review_b6_balancing_scratch_respects_a_small_memory_budget() {
    // 16 n-by-n blocks (two Lyapunov solves plus the split). For n = 128 that
    // is 2 MiB, so a 1 MiB budget must refuse before any Gramian is allocated.
    // The model itself is one 128-by-128 matrix, about 128 KiB.
    const MIB: usize = 1024 * 1024;
    let mut big = Engine::with_limits(Limits {
        evaluation_units: None,
        memory_bytes: MIB,
        ..Limits::browser()
    });
    big.execute("sys = ss(-eye(128), ones(128,1), ones(1,128), 0);")
        .unwrap_or_else(|err| panic!("model should fit: {err}"));
    for code in [
        "h = hsvd(sys);",
        "g = gram(sys,'c');",
        "r = balred(sys, 1);",
        "[sb, g] = balreal(sys);",
    ] {
        let err = big.execute(code).expect_err(code).to_string();
        assert!(err.contains("memory limit"), "{code}: {err}");
        assert!(!err.contains("Workspace"), "{code}: refused late: {err}");
    }
    // modred does not balance. Its elimination blocks for dropping 64 of 128
    // states are about 29k doubles (232 KiB). A 180 KiB budget holds the model
    // (128 KiB) and refuses before those blocks are allocated.
    let mut mid = Engine::with_limits(Limits {
        evaluation_units: None,
        memory_bytes: 180 * 1024,
        ..Limits::browser()
    });
    mid.execute("sys = ss(-eye(128), ones(128,1), ones(1,128), 0);")
        .unwrap_or_else(|err| panic!("model should fit: {err}"));
    let err = mid
        .execute("m = modred(sys, 65:128);")
        .expect_err("modred")
        .to_string();
    assert!(err.contains("memory limit"), "{err}");
    assert!(!err.contains("Workspace"), "refused late: {err}");
    // The same budgets still reduce a one-state model.
    let mut small = Engine::with_limits(Limits {
        evaluation_units: None,
        memory_bytes: 180 * 1024,
        ..Limits::browser()
    });
    small
        .execute("s = ss(-1, 1, 1, 0); h = hsvd(s); g = gram(s,'c'); r = balred(s, 0); m = modred(s, 1);")
        .unwrap_or_else(|err| panic!("{err}"));
    close(&real(&small, "h"), &[0.5], 1e-12, "h");
}
