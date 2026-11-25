//! Independent SciPy / exhaustive permutation / hand-formula references.
use matrust::{Array, Engine};
use serde::Deserialize;
fn run(e: &mut Engine, s: &str) {
    e.execute(s).unwrap_or_else(|err| panic!("{s}: {err}"));
}
fn nums(e: &Engine, n: &str) -> Vec<f64> {
    match e.workspace().get(n).unwrap() {
        Array::Numeric(m) => m.data().to_vec(),
        Array::Logical(m) => m.data().iter().map(|v| *v as f64).collect(),
        a => panic!("unexpected {}", a.class()),
    }
}
fn close(a: f64, b: f64, t: f64) {
    assert!(
        a == b || a.is_nan() && b.is_nan() || (a - b).abs() <= t * b.abs().max(1.),
        "{a:.17e} != {b:.17e}"
    )
}
fn check(s: &str, want: &[f64]) {
    let mut e = Engine::default();
    run(&mut e, s);
    let got = nums(&e, "zz");
    assert_eq!(got.len(), want.len(), "{s}");
    for (a, b) in got.into_iter().zip(want) {
        close(a, *b, 1e-12)
    }
}
#[derive(Deserialize)]
struct Reference {
    source: String,
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Case {
    code: String,
    values: Vec<f64>,
    tol: f64,
}
#[test]
fn scipy_and_permutation_references() {
    let r: Reference =
        serde_json::from_str(include_str!("fixtures/stats_tests/reference.json")).unwrap();
    assert!(r.source.contains("SciPy"));
    let mut e = Engine::default();
    let mut maxima = std::collections::BTreeMap::<String, f64>::new();
    for c in r.cases {
        run(&mut e, &c.code);
        let got = nums(&e, "zz");
        assert_eq!(got.len(), c.values.len());
        for (a, b) in got.into_iter().zip(c.values) {
            let diff = (a - b).abs();
            let key = c
                .code
                .split('=')
                .nth(1)
                .unwrap()
                .split('(')
                .next()
                .unwrap()
                .to_string();
            let max = maxima.entry(key).or_default();
            *max = max.max(diff);
            assert!(
                diff <= c.tol
                    * if c.code.contains("kstest") {
                        b.abs()
                    } else {
                        b.abs().max(1.)
                    }
                    || a == b,
                "{}: {a:.17e} != {b:.17e}",
                c.code
            );
        }
    }
    println!("Max absolute errors: {maxima:?}");
}
#[test]
fn student_shapes_nan_and_legacy() {
    check(
        "[h,p,ci,s]=ttest([0 0 0]); zz=[h p s.tstat];",
        &[f64::NAN; 3],
    );
    check(
        "[h,p,ci,s]=ttest([1 1 1]); zz=[h p s.tstat];",
        &[1., 0., f64::INFINITY],
    );
    check(
        "[h,p,ci,s]=ttest([1 NaN 3;2 5 4;3 7 NaN],2,'Dim',1); zz=[s.df size(ci)];",
        &[2., 1., 1., 2., 3.],
    );
    check(
        "[h,p,ci,s]=ttest([1 2 3;4 5 6],0,[],[],2); zz=[size(p) size(ci)];",
        &[2., 1., 2., 2.],
    );
    check(
        "[h,p,ci,s]=ttest2([1 2;3 4;5 6],[2 3;4 5],'Var','unequal'); zz=[size(s.sd) size(ci)];",
        &[2., 2., 2., 2.],
    );
    check(
        "[h,p,ci,s]=ttest([1 2 3],[1 1 1],0.1,'right',2); [j,q]=ttest([0 1 2],0,'Alpha',0.1,'Tail','right'); zz=[h-j p-q ci(2)];",
        &[0., 0., f64::INFINITY],
    );
    check(
        "[h,p,ci,s]=ttest2([1 2 3],[2 4 6],0.1,'left','unequal',2); [j,q]=ttest2([1 2 3],[2 4 6],'Al',0.1,'Ta','left','Var','unequal'); zz=[h-j p-q ci(1)];",
        &[0., 0., f64::NEG_INFINITY],
    );
    check(
        "[h,p,ci,s]=ttest(single([1 2 3])); [j,q]=ttest(uint8([1 2 3])); zz=[h-j p-q isa(p,'double')];",
        &[0., 0., 1.],
    );
}
#[test]
fn rank_ties_tails_and_caps() {
    check(
        "[p,h,s]=ranksum([4 5 5 6 7],[1 2 3],0.1); zz=[p s.ranksum islogical(h)];",
        &[2. / 56., 30., 1.],
    );
    check(
        "[p,h,s]=signrank([NaN 0 1 -1 2 3],0,0.1); zz=[p s.signedrank islogical(h)];",
        &[0.375, 8.5, 1.],
    );
    check(
        "[p,h,s]=signrank([1 2 3],[1 1 1],[]); zz=s.signedrank;",
        &[3.],
    );
    check(
        "[p,h,s]=ranksum([NaN],[2]); zz=[p h islogical(h)];",
        &[f64::NAN, 0., 1.],
    );
    let mut e = Engine::default();
    for s in [
        "ranksum(1:200,201:400,'Method','exact')",
        "signrank(1:4000,'Method','exact')",
    ] {
        assert!(e.execute(s).unwrap_err().contains("approximate"));
    }
    check(
        "[p,h,s]=signrank([1 -2 3 -4 5 -6],'Method','approximate'); zz=isfield(s,'zval');",
        &[1.],
    );
}
#[test]
fn trimmed_hand_formulas_and_options() {
    check("zz=trimmean([1 2 100],100,'weighted');", &[f64::NAN]);
    check(
        "zz=[trimmean([0 1 2 100],25,'round') trimmean([0 1 2 100],25,'floor') trimmean([0 1 2 100],25,'weighted')];",
        &[1.5, 25.75, 53. / 3.],
    );
    check("zz=trimmean([1 NaN 3;2 4 100;3 6 200],0);", &[2., 5., 101.]);
    check("zz=trimmean([0 1;2 100],25,'weighted','all');", &[53. / 3.]);
    check("zz=trimmean([NaN NaN],20);", &[f64::NAN]);
    let mut e = Engine::default();
    assert!(
        e.execute("trimmean([1 2],10,'round',[1 2])")
            .unwrap_err()
            .contains("not supported yet")
    );
}
#[test]
fn anova_group_contracts_tables_and_display() {
    check(
        "[p,t,s]=anova1([1 2 3 4 5 6],[2 1 2 1 NaN 3],'off'); zz=[s.n s.means s.df size(s.gnames)];",
        &[2., 2., 1., 3., 2., 6., 2., 3., 1.],
    );
    check(
        "[p,t,s]=anova1([1 2 3 4 5],{'b','a','b','','a'},'off'); zz=[strcmp(s.gnames{1},'b') strcmp(s.gnames{2},'a') s.n s.means];",
        &[1., 1., 2., 2., 2., 3.5],
    );
    check(
        "[p,t,s]=anova1([1 2 3;4 5 6],[2 1 2],'off'); zz=[s.n s.means size(t) isempty(t{3,5}) isempty(t{4,4})];",
        &[2., 4., 3.5, 3.5, 4., 6., 1., 1.],
    );
    check(
        "[p,t,s]=anova1([1 2 3 4],[\"z\" \"a\" \"\" \"z\"],'off'); zz=[s.n strcmp(s.gnames{1},'z')];",
        &[2., 1., 1.],
    );
    let mut e = Engine::default();
    let out = e.execute("anova1([1 2;3 5]);").unwrap();
    assert!(out.output.join("\n").contains("Groups"));
    assert!(
        e.execute("anova1([1 2;3 5],[],'off');")
            .unwrap()
            .output
            .is_empty()
    );
    check(
        "[p,t,s]=anova2([1 4 9;2 5 7;3 8 6],1,'off'); zz=[s.sigmasq s.df s.inter size(p) size(t) isnan(s.pval)];",
        &[10. / 3., 4., 0., 1., 2., 5., 6., 1.],
    );
    check(
        "[p,t,s]=anova2([1 4;3 6;7 6;9 8],2,'off'); zz=[s.sigmasq s.df s.inter size(p) size(t) t{4,2}];",
        &[2., 4., 1., 1., 3., 6., 6., 8.],
    );
    for s in [
        "anova2([1 2;3 NaN],1,'off')",
        "anova2([1 2;3 4;5 6],2,'off')",
    ] {
        assert!(e.execute(s).is_err())
    }
}
#[test]
fn ks_critical_values_and_legacy() {
    check(
        "[h,p,d,cv]=kstest([0.1 0.3 0.6 0.9],[0 0;1 1],0.3,'larger'); zz=[d isnan(cv) islogical(h)];",
        &[0.2, 1., 1.],
    );
    let mut e = Engine::default();
    run(
        &mut e,
        "[h,p,d,cv]=kstest([-1.5 -0.4 0.2 0.9 1.6]); [j,q,z,c]=kstest([cv 1],'CDF',[0 0;1 1],'Alpha',0.3); zz=cv;",
    );
    let cv = nums(&e, "zz")[0];
    assert!(cv > 0.5 && cv < 0.6);
    assert!(
        e.execute("kstest([0.1 1.1],'CDF',[0 0;1 1])")
            .unwrap_err()
            .contains("outside")
    );
    check(
        "[h,p,d,cv]=kstest([NaN]); zz=[h p d cv];",
        &[0., f64::NAN, f64::NAN, f64::NAN],
    );
}
#[test]
fn chi_tail_pooling_handles_expected_and_nan_h() {
    check(
        "[h,p,s]=chi2gof([1 2 3 4 5],'Ctrs',[1 2 3 4 5],'Frequency',[1 15 1 15 1],'Expected',[1 15 1 15 1]); zz=[s.O s.E s.df s.chi2stat];",
        &[16., 1., 16., 16., 1., 16., 2., 0.],
    );
    check(
        "[h,p,s]=chi2gof([1 2 3],'Expected',[1 1 1],'NParams',5,'EMin',0); zz=[h p s.df];",
        &[f64::NAN, f64::NAN, -3.],
    );
    check(
        "[h,p,s]=chi2gof([0 0.2 0.4 0.6 0.8 1],'NBins',3,'CDF',@(x) unifcdf(x,0,1),'EMin',0); zz=[sum(s.E) s.df];",
        &[6., 2.],
    );
    check(
        "[h,p,s]=chi2gof([0 0.2 0.4 0.6 0.8 1],'NBins',3,'CDF',{@unifcdf,0,1},'EMin',0); zz=s.df;",
        &[0.],
    );
}
#[test]
fn sampling_classes_orientation_indices_and_replay() {
    check(
        "a=randsample('RandStream',3); b=datasample('RandStream',3); zz=[ischar(a) numel(a) ischar(b) numel(b)];",
        &[1., 3., 1., 3.],
    );
    let mut e = Engine::default();
    for code in [
        "randsample(1e9,5)",
        "randsample(uint8([2 4 6 8]),3)",
        "randsample(logical([1 0 1]),2)",
        "randsample('abcd',3)",
        "randsample([\"a\" \"b\" \"c\"],2)",
        "datasample({1 2;3 4;5 6},2)",
        "datasample(single([1 2;3 4;5 6]),2)",
    ] {
        run(
            &mut e,
            &format!("rng(7); a={code}; rng(7); b={code}; zz=isequal(a,b);"),
        );
        assert_eq!(nums(&e, "zz"), [1.]);
    }
    check(
        "a=randsample(1e9,5); zz=[size(a) numel(unique(a)) all(a>=1) all(a<=1e9)];",
        &[5., 1., 5., 1., 1.],
    );
    check(
        "a=randsample('abcd',3); b=randsample([2;4;6],2); zz=[ischar(a) size(a) size(b)];",
        &[1., 1., 3., 2., 1.],
    );
    check(
        "x=[1 2;3 4;5 6]; [a,i]=datasample(x,2,'Replace',false); zz=isequal(a,x(i,:));",
        &[1.],
    );
    check(
        "x=[1 2;3 4;5 6];[a,i]=datasample(x,1,2,'Weights',[0 1]); zz=[i isequal(a,x(:,i))];",
        &[2., 1.],
    );
    check("a=randsample(3,1000,true,[0 1 0]); zz=all(a==2);", &[1.]);
    check(
        "[a,i]=datasample(1:5,3,'Replace',false,'Weights',[0 1 2 3 4]); zz=[numel(unique(i)) all(i>1)];",
        &[3., 1.],
    );
    // Fixed-seed multinomial Pearson bound (df=2, generous p<1e-6 threshold).
    run(
        &mut e,
        "rng(99); a=randsample(3,100000,true,[1 2 7]); zz=[sum(a==1) sum(a==2) sum(a==3)];",
    );
    let counts = nums(&e, "zz");
    let chi = counts
        .iter()
        .zip([10000., 20000., 70000.])
        .map(|(o, x)| (o - x).powi(2) / x)
        .sum::<f64>();
    assert!(chi < 27.64, "{counts:?}: {chi}");
}
#[test]
fn mnrnd_rows_totals_zero_categories_and_replay() {
    // Degenerate rows have one possible outcome (Octave statistics confirmed).
    check(
        "r=mnrnd([2;5],[0 1 0;1 0 0]); zz=[size(r) r(:)'];",
        &[2., 3., 0., 5., 2., 0., 0., 0.],
    );
    // Distinct totals, a zero-trial row and zero-probability categories.
    let mut e = Engine::default();
    run(
        &mut e,
        "n=[7;0;13;40;1]; p=[0.2 0 0.8;0.5 0.25 0.25;0.1 0.6 0.3;0 0.5 0.5;1/3 1/3 1/3]; rng(11); r=mnrnd(n,p); \
         zz=[size(r) isa(r,'double') all(isfinite(r(:))) all(r(:)>=0) isequal(r,round(r)) isequal(sum(r,2),n) \
         r(1,2) r(4,1) any(r(2,:))];",
    );
    assert_eq!(nums(&e, "zz"), [5., 3., 1., 1., 1., 1., 1., 0., 0., 0.]);
    // Resetting the stream replays the counts; another seed is still valid.
    run(
        &mut e,
        "rng(11); s=mnrnd(n,p); rng(12); t=mnrnd(n,p); zz=[isequal(r,s) isequal(sum(t,2),n) t(1,2) t(4,1)];",
    );
    assert_eq!(nums(&e, "zz"), [1., 1., 0., 0.]);
    // Scalar count with a probability matrix, column counts with one row,
    // and m vectors from scalar parameters.
    check(
        "a=mnrnd(4,[0.5 0.5;0 1]); b=mnrnd([3;0;2],[0 0.5 0.5]); c=mnrnd(6,[0.25 0.75],4); \
         zz=[size(a) sum(a,2)' a(2,:) size(b) sum(b,2)' b(:,1)' size(c) sum(c,2)'];",
        &[
            2., 2., 4., 4., 0., 4., 3., 3., 3., 0., 2., 0., 0., 0., 4., 2., 6., 6., 6., 6.,
        ],
    );
    // A fixed-seed bounded run must reach every eligible category and split
    // rows (a correct sampler fails these with probability below 1e-18; the
    // seed makes the outcome fixed, not a statistical threshold).
    run(
        &mut e,
        "rng(5); r=mnrnd(repmat(50,20,1),repmat([0.3 0.3 0.4],20,1)); \
         zz=[all(sum(r,1)>0) sum(sum(r>0,2)>1) isequal(sum(r,2),repmat(50,20,1))];",
    );
    assert_eq!(nums(&e, "zz"), [1., 20., 1.]);
    for (code, want) in [
        ("mnrnd([2;-1],[0.5 0.5])", "N must"),
        ("mnrnd(2.5,[0.5 0.5])", "N must"),
        ("mnrnd([2 3],[0.5 0.5])", "N must"),
        ("mnrnd([2;3;4],[0.5 0.5;0.2 0.8])", "rows"),
        ("mnrnd([2;3],[0.5 0.5;0.2 0.7])", "sum to 1"),
        ("mnrnd(2,[1.5 -0.5])", "P must"),
        ("mnrnd(2,[NaN 1])", "P must"),
        ("mnrnd(2,zeros(1,0))", "P must"),
        ("mnrnd(2,[0.5 0.5],2.5)", "M must"),
        ("mnrnd([1;2],[0.5 0.5],3)", "rows"),
        ("mnrnd(1e300,[0.5 0.5])", "N must"),
        ("mnrnd(2,[0.5 0.5],1e12)", "limit"),
        ("mnrnd(2)", "expected"),
    ] {
        let error = e.execute(code).unwrap_err();
        assert!(error.contains(want), "{code}: {error}");
    }
    // Validation precedes sampling: a rejected call leaves the stream alone.
    run(
        &mut e,
        "rng(4); a=rand(); rng(4); try, mnrnd([1;2],[0.5 0.5;1 1]); catch, end; b=rand(); zz=a==b;",
    );
    assert_eq!(nums(&e, "zz"), [1.]);
}
#[test]
fn bootstrap_joint_rows_draw_order_and_caps() {
    check(
        "x=[1;2;4;8]; rng(7); [b,i]=bootstrp(4,@mean,x); zz=[b(1)-mean(x(i(:,1))) size(i)];",
        &[0., 4., 4.],
    );
    check(
        "rng(7); [b,i]=bootstrp(3,[],[1;2;3]); rng(7); [c,j]=bootstrp(3,@(x) mean(x)+rand(),[1;2;3]); zz=isequal(i,j);",
        &[1.],
    );
    check(
        "[b,i]=bootstrp(3,@(x,y,z) mean(x-y)+z,[1;2;3],[11;12;13],2,'Options',struct('UseParallel',\"auto\")); zz=[b' size(i)];",
        &[-8., -8., -8., 3., 3.],
    );
    check(
        "[b,i]=bootstrp(3,@(x) double(isa(x,'uint8')),uint8([1;2;3]),'Weights',[0 1 0]); zz=[b' i(:)'];",
        &[1., 1., 1., 2., 2., 2., 2., 2., 2., 2., 2., 2.],
    );
    check(
        "[b,i]=bootstrp(2,@(x) double(iscell(x)),{1;2;3}); zz=b;",
        &[1., 1.],
    );
    let mut e = Engine::default();
    assert!(
        e.execute("bootstrp(1e9,[],[1;2])")
            .unwrap_err()
            .contains("too large")
    );
    // 2 replicates of 25,165,825 values is 50,331,650 elements, just above
    // the 48 Mi cap. The first replicate is built; the bootstat is refused
    // before it is allocated.
    assert!(
        e.execute("bootstrp(2,@(x) ones(1,25165825),[1;2])")
            .unwrap_err()
            .contains("too large")
    );
}
#[test]
fn errors_options_and_registry() {
    let mut e = Engine::default();
    e.set_function_directory(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stats_tests"),
    )
    .unwrap();
    run(&mut e, "stream=RandStream();");
    for n in [
        "ttest",
        "ttest2",
        "anova1",
        "anova2",
        "kstest",
        "chi2gof",
        "ranksum",
        "signrank",
        "trimmean",
        "randsample",
        "datasample",
        "bootstrp",
        "mnrnd",
    ] {
        assert!(matrust::session::FUNCTIONS.contains(&n));
        assert!(matrust::help::summary(n).is_some());
        let error = e.execute(&format!("{n}(stream)")).unwrap_err();
        assert!(
            error.contains("RandStream input is not supported yet"),
            "{error}"
        );
    }
    for s in [
        "ranksum([1 2;3 4],[1 2])",
        "signrank([1 2;3 4])",
        "kstest([1 2;3 4])",
        "chi2gof([1 2;3 4])",
        "ttest([1 2],'Alpha',0)",
        "ttest([1 2],'Al',0.1,'Alpha',0.2)",
        "datasample(1:3,4,'Replace',false)",
        "randsample({1 2},1)",
        "ttest([1 2],'Nonsense',2)",
    ] {
        assert!(e.execute(s).is_err(), "{s}");
    }
}

/// Optimized gate profile inherits release; run manually via bench.sh.
#[test]
#[ignore]
fn release_benchmarks() {
    use std::time::Instant;
    let cases = [
        (
            "ttest n=100000",
            "x=linspace(-2,3,100000);",
            "[h,p,ci,s]=ttest(x,0);",
            20,
        ),
        (
            "ttest2 n=100000 each",
            "x=linspace(-2,3,100000);y=x+0.1;",
            "[h,p,ci,s]=ttest2(x,y,'Vartype','unequal');",
            20,
        ),
        (
            "anova1 1000x3",
            "x=reshape(linspace(0,1,3000),1000,3);",
            "[p,t,s]=anova1(x,[],'off');",
            20,
        ),
        (
            "anova2 100x10 reps=2",
            "x=reshape(sin(1:1000),100,10);",
            "[p,t,s]=anova2(x,2,'off');",
            20,
        ),
        (
            "ranksum exact n=12+16",
            "",
            "[p,h,s]=ranksum(1:12,5:20,'method','exact');",
            20,
        ),
        ("signrank exact n=15", "", "[p,h,s]=signrank(1:15);", 20),
        (
            "trimmean n=100000",
            "x=sin(1:100000);",
            "m=trimmean(x,20);",
            20,
        ),
        (
            "kstest MTW n=500 order=201",
            "x=linspace(0.2,0.8,500);",
            "[h,p,d,cv]=kstest(x,'CDF',[0 0;1 1],'Alpha',0.3);",
            3,
        ),
        (
            "chi2gof n=100000",
            "x=sin(1:100000);",
            "[h,p,s]=chi2gof(x);",
            10,
        ),
        ("randsample n=1e9 k=5", "", "a=randsample(1e9,5);", 1000),
        (
            "datasample weighted n=10000 k=100",
            "x=1:10000;",
            "[a,i]=datasample(x,100,'Replace',false,'Weights',x);",
            10,
        ),
        (
            "bootstrp n=1000 nboot=100",
            "x=(1:1000)';",
            "[b,i]=bootstrp(100,@mean,x);",
            5,
        ),
    ];
    for (name, setup, code, reps) in cases {
        let mut e = Engine::default();
        run(&mut e, setup);
        run(&mut e, code);
        let start = Instant::now();
        for _ in 0..reps {
            run(&mut e, code);
        }
        println!(
            "{name}: reps={reps} elapsed_ms={:.3}",
            start.elapsed().as_secs_f64() * 1000.
        );
    }
}
#[test]
fn review_fixes_empty_sample_sign_zero_outer_bins_fast_tails() {
    // ttest2 with an empty y is NaN, not a panic.
    check("[~,p]=ttest2([1 2 3],[]); zz=p;", &[f64::NAN]);
    // rank statistic equal to its mean: no continuity correction, p = 1, z = 0.
    check(
        "[p,~,st]=ranksum([1:5 16:20],6:15,'method','approximate'); zz=[p st.zval];",
        &[1., 0.],
    );
    // observations outside narrow Edges land in the outer bins (sum(O) = n).
    check(
        "rng(3); [~,~,st]=chi2gof(randn(1,200),'Edges',[-1 -0.5 0 0.5 1]); zz=[sum(st.O) sum(st.E)];",
        &[200., 200.],
    );
    // far tails and large n stay fast (were 24-60 s before the budget/cv fixes).
    let t = std::time::Instant::now();
    let mut e = Engine::default();
    run(
        &mut e,
        "rng(1); h1=kstest(randn(1e5,1)); [~,p2]=kstest(randn(500,1)+1); [~,p3]=kstest(randi(5,300,1));",
    );
    assert!(nums(&e, "p2")[0] < 1e-10 && nums(&e, "p3")[0] < 1e-10);
    assert!(t.elapsed().as_secs() < 20, "{:?}", t.elapsed());
}

#[test]
fn review_1111_void_handles_error_instead_of_panic() {
    // disp returns no value. call_handle turned that into an empty Vec and
    // remove(0) panicked. MATLAB-style: the function returned no value.
    let mut e = Engine::default();
    for code in [
        "bootstrp(2,@disp,[1;2;3])",
        "chi2gof([1 2 3 4],'CDF',@disp)",
    ] {
        let err = e.execute(code).unwrap_err();
        assert!(err.contains("returned no value"), "{code}: {err}");
    }
}

#[test]
fn review_1111_chi2gof_rejects_nonfinite_specification() {
    // MathWorks chi2gof: Edges, Ctrs and Expected define the bins. A NaN used
    // to be dropped by vector(), so Edges [1 NaN 3 5] was tested as [1 3 5].
    // ±Inf outer edges are allowed; see review_s1113_chi2gof_infinite_outer_edges.
    let mut e = Engine::default();
    for code in [
        "chi2gof([1 2 3 4],'Edges',[1 NaN 3 5])",
        "chi2gof([1 2 3],'Ctrs',[1 NaN 3])",
        "chi2gof([1 2 3],'Ctrs',[1 Inf 3])",
        "chi2gof([1 2 3],'Expected',[1 NaN 1],'EMin',0)",
        "chi2gof([1 2 3],'Expected',[1 Inf 1],'EMin',0)",
    ] {
        let err = e.execute(code).unwrap_err();
        assert!(err.contains("must be finite"), "{code}: {err}");
    }
}

#[test]
fn review_s1113_chi2gof_infinite_outer_edges() {
    // Octave chi2gof / histc: outer edges may be ±Inf. Only NaN is rejected.
    // Data [1 2 3 4], edges [-Inf 2.5 Inf], expected [2 2] → O = [2 2],
    // chi2stat = 0, p = 1.
    check(
        "[h,p,s]=chi2gof([1 2 3 4],'Edges',[-Inf 2.5 Inf],'Expected',[2 2],'EMin',0); zz=[s.O s.chi2stat p];",
        &[2., 2., 0., 1.],
    );
}

#[test]
fn review_1111_ttest2_vectors_ignore_orientation() {
    // SciPy 1.18.1 ttest_ind([1,2,3,4],[2,3,4,6], equal_var=True):
    // t = -1.1677484162422844, p = 0.2872081404442227, df = 6.
    // 95% CI for the mean difference (t.ppf(0.975, 6), pooled se):
    // [-3.86926265228744, 1.36926265228744].
    // MathWorks ttest2: vectors need not share a length or an orientation.
    let want = &[
        0.2872081404442227,
        -1.1677484162422844,
        6.0,
        -3.86926265228744,
        1.36926265228744,
    ];
    // ci is 2x1 or 1x2 depending on the first sample's dimension, so index it.
    check(
        "[~,p,ci,s]=ttest2([1 2 3 4],[2 3 4 6]); zz=[p s.tstat s.df ci(1) ci(2)];",
        want,
    );
    check(
        "[~,p,ci,s]=ttest2([1 2 3 4],[2;3;4;6]); zz=[p s.tstat s.df ci(1) ci(2)];",
        want,
    );
    check(
        "[~,p,ci,s]=ttest2([1;2;3;4],[2 3 4 6]); zz=[p s.tstat s.df ci(1) ci(2)];",
        want,
    );
    // Identical samples: t = 0, p = 1, df = 6 (same SciPy call).
    check(
        "[~,p,~,s]=ttest2([1 2 3 4],[1;2;3;4]); zz=[p s.tstat s.df];",
        &[1., 0., 6.],
    );
    let mut e = Engine::default();
    let err = e.execute("ttest2([1 2;3 4],[1;2;3])").unwrap_err();
    assert!(err.contains("unreduced"), "{err}");
}
