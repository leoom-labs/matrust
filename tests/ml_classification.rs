//! Public API contracts and independent NumPy/SciPy/sklearn references.
use matrust::{Array, Engine};
fn run(e: &mut Engine, s: &str) {
    e.execute(s).unwrap_or_else(|err| panic!("{s}: {err}"));
}
fn numbers(e: &Engine, s: &str) -> Vec<f64> {
    match &e.workspace()[s] {
        Array::Numeric(m) => m.data().to_vec(),
        _ => panic!("expected numeric {s}"),
    }
}
fn close(a: &[f64], b: &[f64], tol: f64) {
    assert_eq!(a.len(), b.len());
    for (i, (a, b)) in a.iter().zip(b).enumerate() {
        assert!((a - b).abs() <= tol, "[{i}] {a:0.16e} != {b:0.16e}");
    }
}
const THREE: &str = "X=[-3 -1;-2 0;-1 -2;-2 1;-4 0;-3 2;0 3;1 2;2 4;1 5;-1 4;2 2;4 -2;5 -1;3 0;6 -3;4 1;5 0];Y=[ones(6,1);2*ones(6,1);3*ones(6,1)];Q=[-1 1;1 1;3 1;0 3];";
const BINARY: &str =
    "X=[-2 0.5;-1 -0.5;0 1;0.5 -1;1 0.7;2 -0.2;0.2 0.3;-0.3 -0.8];Y=[1;1;1;2;2;2;2;1];";
#[test]
#[rustfmt::skip]
fn discriminant_posteriors_and_properties_match_numpy(){
    let mut e=Engine::default();run(&mut e,THREE);
    for(kind,expected)in [
        ("linear",vec![0.9476083631220397,0.002331195161149301,7.733293955895516e-8,0.00032790199841740427,0.05237715127000214,0.9691295983111968,0.2418009367666021,0.9995180569153256,1.4485607958231866e-5,0.028539206527653762,0.7581989859004583,0.00015404108625713457]),
        ("quadratic",vec![0.8832284825921293,0.0020343651452869216,2.2541664492147195e-8,0.0002856386452162646,0.116770883202474,0.978916362906828,0.13768718203480415,0.9995578622737111,6.342053967624952e-7,0.0190492719478851,0.8623127954235313,0.00015649908107260576])]{
        run(&mut e,&format!("m=fitcdiscr(X,Y,'DiscrimType','{kind}');[label,score,cost]=predict(m,Q);mu=m.Mu;sig=m.Sigma;prior=m.Prior;cf=m.Coeffs;"));
        close(&numbers(&e,"score"),&expected,1e-10);close(&numbers(&e,"cost"),&expected.iter().map(|z|1.-z).collect::<Vec<_>>(),1e-10);
        close(&numbers(&e,"mu"),&[-2.5,0.8333333333333334,4.5,0.,3.3333333333333335,-0.8333333333333334],1e-10);
        close(&numbers(&e,"prior"),&[1./3.;3],1e-10);
        if kind=="linear"{close(&numbers(&e,"sig"),&[1.1888888888888889,-0.6111111111111112,-0.6111111111111112,1.877777777777778],1e-10);run(&mut e,"pair=cf(1,2);constant=pair.Const;linear=pair.Linear;");close(&numbers(&e,"constant"),&[1.6602470447602613],1e-10);close(&numbers(&e,"linear"),&[-4.46274405631558,-3.227520255013946],1e-10);}
        else{run(&mut e,"assert(isequal(size(sig),[2 2 3]));");}
    }
}
#[test]
fn discriminant_variants_priors_and_singular_covariance() {
    let mut e = Engine::default();
    run(&mut e, THREE);
    for kind in [
        "diagLinear",
        "diagQuadratic",
        "pseudoLinear",
        "pseudoQuadratic",
    ] {
        run(
            &mut e,
            &format!(
                "m=fitcdiscr(X,Y,'DiscrimType','{kind}','Prior','uniform');[y,s]=predict(m,X);assert(isequal(y,Y));assert(max(abs(sum(s,2)-1))<1e-12);"
            ),
        );
    }
    run(
        &mut e,
        "a=fitcdiscr(X,Y,'Gamma',1);b=fitcdiscr(X,Y,'DiscrimType','diagLinear');[ya,sa]=predict(a,Q);[yb,sb]=predict(b,Q);assert(max(abs(sa(:)-sb(:)))<1e-12);m=fitcdiscr(X,Y,'Prior',[1 2 3]);p=m.Prior;assert(max(abs(p-[1 2 3]/6))<1e-12);",
    );
    for kind in ["pseudoLinear", "pseudoQuadratic"] {
        run(
            &mut e,
            &format!(
                "m=fitcdiscr([0 0;1 1;3 3;4 4],[1;1;2;2],'DiscrimType','{kind}');[y,s]=predict(m,[0 0;4 4]);assert(isequal(y,[1;2]));assert(all(isfinite(s(:))));"
            ),
        );
    }
}
#[test]
#[rustfmt::skip]
fn linear_objectives_match_scipy(){
    let mut e=Engine::default();run(&mut e,BINARY);
    for(learner,regularization,expected)in[
        ("svm","ridge",vec![-2.5714285714285703,-0.999999999999999,-0.3214285714285717,0.9999999999999999,0.999999999999999,2.5357142857142843,0.17142857142857126,-0.04285714285714232]),
        ("svm","lasso",vec![-2.,-1.,0.,0.5,1.,2.,0.2,-0.3]),
        ("logistic","ridge",vec![-2.304561993997557,-1.125119109388368,-0.1555719084558151,0.5600869440547612,0.9651001847239158,2.1361472419860394,0.12229580520135,-0.3330923995501973]),
        ("logistic","lasso",vec![-2.464554376818807,-1.2713513847674383,-0.07814839271606963,0.5184531033096148,1.1150545993352992,2.308257591386668,0.16049220569420414,-0.43610929033148016])]{
        run(&mut e,&format!("m=fitclinear(X,Y,'Learner','{learner}','Regularization','{regularization}','Lambda',0.15,'Solver','sparsa');b=m.Beta;bias=m.Bias;decision=X*b+bias;[y,s]=predict(m,X);"));
        close(&numbers(&e,"decision"),&expected,1e-6);
        let expected_score:Vec<_>=expected.iter().map(|f|if learner=="logistic"{1./(1.+(-f).exp())}else{*f}).collect();
        close(&numbers(&e,"s")[8..],&expected_score,1e-6);
    }
}
#[test]
#[rustfmt::skip]
fn ecoc_coding_and_decoding_match_sklearn(){
    let mut e=Engine::default();run(&mut e,THREE);
    for(coding,expected,labels)in[
        ("onevsone",vec![-0.23214285714284433,-0.6964285714286047,-1.1607142857143649,-0.7857142857143113,-0.375,-0.1250000000001027,-0.33333333333333326,0.,-1.2261904761902835,-0.6785714285712927,-0.16666666666666674,-1.2142857142854322],vec![1.,2.,3.,2.]),
        ("onevsall",vec![-0.095238095238115,-0.28571428571427576,-0.8271708683472992,-0.7609710550886476,-0.6890756302520811,-0.380952380952391,-0.44901960784313727,0.,-0.8714285714285516,-0.4142857142857242,-0.21764705882352933,-1.1117647058823528],vec![1.,1.,3.,2.])]{
        run(&mut e,&format!("m=fitcecoc(X,Y,'Coding','{coding}');[y,s]=predict(m,Q);cm=m.CodingMatrix;"));close(&numbers(&e,"y"),&labels,0.);close(&numbers(&e,"s"),&expected,1e-6);
    }
    for learner in ["'linear'","'tree'","'knn'","'discriminant'","'naivebayes'","templateSVM('Standardize',true)","templateTree('MaxNumSplits',5)","templateLinear('Learner','logistic')","templateDiscriminant('DiscrimType','diagLinear')"]{run(&mut e,&format!("m=fitcecoc(X,Y,'Learners',{learner});[y,s]=predict(m,X);assert(isequal(y,Y));"));}
}
#[test]
#[rustfmt::skip]
#[allow(clippy::approx_constant)] // Independent reference prints the first weight as ln(2).
fn boosting_weights_and_regression_boosters_match_numpy_sklearn(){
    let mut e=Engine::default();run(&mut e,"X=(0:9)';Y=[1;1;2;1;2;2;1;2;2;2];m=fitcensemble(X,Y,'Method','AdaBoostM1','NumLearningCycles',3,'Learners',templateTree('MaxNumSplits',1));w=m.TrainedWeights;[y,s]=predict(m,X);");
    close(&numbers(&e,"w"),&[0.6931471805599453,0.7331685343967135,0.4054651081081643],1e-10);
    for(method,expected)in[
        ("GentleBoost",vec![-2.3782141777831405,-2.3782141777831405,0.31274180392501644,0.31274180392501644,0.31274180392501644,0.31274180392501644,0.31274180392501644,1.6909559817081568,1.6909559817081568,1.6909559817081568]),
        ("LogitBoost",vec![-1.0874650201358877,-1.0874650201358877,0.41253497986411225,0.41253497986411225,0.41253497986411225,0.41253497986411225,-0.19200485368992748,0.8664795660086653,0.8664795660086653,0.8664795660086653])]{run(&mut e,&format!("m=fitcensemble(X,Y,'Method','{method}','NumLearningCycles',3);[y,s]=predict(m,X);"));close(&numbers(&e,"s")[10..],&expected,1e-10);}
    run(&mut e,"X=(0:11)';Y=[1;1;2;1;2;2;3;2;3;3;1;3];m=fitcensemble(X,Y,'Method','AdaBoostM2','NumLearningCycles',3);w=m.TrainedWeights;[y,s]=predict(m,X);");
    close(&numbers(&e,"w"),&[0.5108256237659907,0.42290514512894556,0.3836080040457592],1e-10);
    close(&numbers(&e,"s")[..12],&[0.6080809876209065,0.6080809876209065,0.6080809876209065,0.6080809876209065,0.6080809876209065,0.6080809876209065,0.30620749861214036,0.30620749861214036,0.28693233057058876,0.28693233057058876,0.28693233057058876,0.28693233057058876],1e-10);
}
#[test]
fn bag_and_rusboost_use_rng_and_tree_importance() {
    let mut e = Engine::default();
    run(&mut e, THREE);
    for method in ["Bag", "RUSBoost"] {
        run(
            &mut e,
            &format!(
                "rng(12);a=fitcensemble(X,Y,'Method','{method}','NumLearningCycles',12);[ya,sa]=predict(a,X);rng(12);b=fitcensemble(X,Y,'Method','{method}','NumLearningCycles',12);[yb,sb]=predict(b,X);assert(isequal(ya,yb));assert(isequal(sa,sb));z=resubLoss(a);imp=predictorImportance(a);assert(all(imp>=0));"
            ),
        );
    }
    run(
        &mut e,
        "rng(9);m=fitcensemble(X,Y,'NumLearningCycles',50);o=oobLoss(m);assert(o>=0 && o<=1);m=fitctree([0 0;1 0;4 0;5 0],[1;1;2;2],'MinParentSize',2);imp=predictorImportance(m);",
    );
    close(&numbers(&e, "imp"), &[0.5, 0.], 1e-10);
}
#[test]
fn cvpartition_masks_stratification_and_rng() {
    let mut e = Engine::default();
    run(
        &mut e,
        "g=[ones(6,1);2*ones(6,1);3*ones(6,1)];rng(123);c=cvpartition(g,'KFold',3);n=c.NumObservations;k=c.NumTestSets;ts=c.TestSize;tr=c.TrainSize;mask=test(c,1);rng(123);d=cvpartition(g,'KFold',3);assert(isequal(mask,test(d,1)));for i=1:3,t=test(c,i);r=training(c,i);assert(all(t~=r));assert(sum(t)==6);assert(sum(t(1:6))==2);assert(sum(t(7:12))==2);assert(sum(t(13:18))==2);end;rng(11);a=repartition(c);rng(11);b=repartition(c);assert(isequal(test(a,1),test(b,1)));",
    );
    close(&numbers(&e, "n"), &[18.], 0.);
    close(&numbers(&e, "k"), &[3.], 0.);
    close(&numbers(&e, "ts"), &[6.; 3], 0.);
    close(&numbers(&e, "tr"), &[12.; 3], 0.);
    run(
        &mut e,
        "c=cvpartition(7,'KFold',3);assert(isequal(c.TestSize,[3 2 2]));c=cvpartition(10,'Holdout',0.3);assert(c.TestSize==3);assert(sum(training(c))==7);c=cvpartition(4,'Leaveout');assert(c.NumTestSets==4);assert(sum(test(c,3))==1);c=cvpartition(4,'Resubstitution');assert(all(test(c)));assert(all(training(c)));",
    );
}
#[test]
fn crossval_for_every_new_classifier_and_partition_forms() {
    let mut e = Engine::default();
    run(&mut e, THREE);
    for fit in ["fitcdiscr", "fitcecoc", "fitcensemble"] {
        run(
            &mut e,
            &format!(
                "rng(7);m={fit}(X,Y);c=cvpartition(Y,'KFold',3);v=crossval(m,'CVPartition',c);[yp,s]=kfoldPredict(v);l=kfoldLoss(v);assert(size(s,1)==18);assert(l>=0 && l<=1);v=crossval(m,'Holdout',0.25);l=kfoldLoss(v);assert(l>=0 && l<=1);"
            ),
        );
    }
    run(&mut e, BINARY);
    run(
        &mut e,
        "m=fitclinear(X,Y,'Learner','logistic');v=crossval(m,'KFold',2);l=kfoldLoss(v);assert(l>=0 && l<=1);v=crossval(m,'Leaveout','on');l=kfoldLoss(v,'LossFun','hinge');assert(isfinite(l));",
    );
    for fit in ["fitcknn", "fitctree", "fitcsvm", "fitcnb"] {
        run(
            &mut e,
            &format!(
                "m={fit}(X,Y);c=cvpartition(Y,'KFold',2);v=crossval(m,'CVPartition',c);l=kfoldLoss(v);assert(l>=0 && l<=1);"
            ),
        );
    }
}
#[test]
fn shared_losses_margin_edge_and_table_response() {
    let mut e = Engine::default();
    run(&mut e, "X=[0;1;4;5];Y=[1;1;2;2];");
    for fit in [
        "fitcknn",
        "fitctree",
        "fitcsvm",
        "fitcnb",
        "fitcdiscr",
        "fitclinear",
        "fitcecoc",
        "fitcensemble",
    ] {
        let extra = if fit == "fitctree" {
            ",'MinParentSize',2"
        } else {
            ""
        };
        run(
            &mut e,
            &format!(
                "m={fit}(X,Y{extra});[y,s]=predict(m,X);ma=margin(m,X,Y);ed=edge(m,X,Y);assert(abs(ed-mean(ma))<1e-12);le=loss(m,X,Y);lr=resubLoss(m);assert(le==lr);"
            ),
        );
    }
    for fit in [
        "fitcknn",
        "fitctree",
        "fitcsvm",
        "fitcnb",
        "fitcdiscr",
        "fitclinear",
        "fitcecoc",
        "fitcensemble",
    ] {
        run(&mut e, &format!("m={fit}(X,Y);"));
        for loss in [
            "classiferror",
            "binodeviance",
            "exponential",
            "hinge",
            "mincost",
        ] {
            run(
                &mut e,
                &format!("l=loss(m,X,Y,'LossFun','{loss}');assert(isfinite(l));"),
            );
        }
    }
    run(
        &mut e,
        "t=table([0;1;4;5],categorical({'a';'a';'b';'b'}),'VariableNames',{'feature','response'});",
    );
    for fit in ["fitcdiscr", "fitclinear", "fitcecoc", "fitcensemble"] {
        for spec in [
            "'response'",
            "'ResponseVarName','response'",
            "'response ~ feature'",
        ] {
            run(
                &mut e,
                &format!(
                    "m={fit}(t,{spec});y=predict(m,t);assert(isequal(y,t.response));assert(resubLoss(m)==0);"
                ),
            );
        }
    }
    run(
        &mut e,
        "m=fitcdiscr([0;1;2;4;5],[1;1;NaN;2;2]);assert(m.NumObservations==4);m=fitclinear([0;1;2;4;5],single([1;1;NaN;2;2]));assert(m.NumObservations==4);",
    );
}
#[test]
fn numeric_loss_formulas_and_table_validation() {
    let mut e = Engine::default();
    run(
        &mut e,
        "m=fitcknn([0;1;4;5],[1;1;2;2],'NumNeighbors',3);ma=margin(m,[0.5;4.5],[1;2]);ed=edge(m,[0.5;4.5],[1;2]);h=loss(m,[0.5;4.5],[1;2],'LossFun','hinge');b=loss(m,[0.5;4.5],[1;2],'LossFun','binodeviance');ex=loss(m,[0.5;4.5],[1;2],'LossFun','exponential');",
    );
    close(&numbers(&e, "ma"), &[1. / 3., 1. / 3.], 1e-10);
    close(&numbers(&e, "ed"), &[1. / 3.], 1e-10);
    close(&numbers(&e, "h"), &[5. / 6.], 1e-10);
    close(&numbers(&e, "b"), &[0.6574996392981435], 1e-10);
    close(&numbers(&e, "ex"), &[0.9545147720593408], 1e-10);
    run(
        &mut e,
        "t=table([0;1;4;5],{'a';'a';'b';'b'},'VariableNames',{'x','response'});m=fitcdiscr(t,'response');assert(loss(m,t,'response')==0);assert(edge(m,t,'response')>0);m=fitcknn([0;1],[1;2]);c=cvpartition(2,'Resubstitution');v=crossval(m,'CVPartition',c);assert(kfoldLoss(v)==0);",
    );
}

#[test]
fn new_names_help_examples_and_errors() {
    let mut e = Engine::default();
    for name in [
        "fitcdiscr",
        "fitclinear",
        "fitcecoc",
        "fitcensemble",
        "templateSVM",
        "templateTree",
        "templateLinear",
        "templateDiscriminant",
        "cvpartition",
        "training",
        "test",
        "repartition",
        "loss",
        "margin",
        "edge",
        "oobLoss",
        "predictorImportance",
    ] {
        let page = matrust::help::page(name).unwrap_or_else(|| panic!("missing help {name}"));
        for line in page.lines().filter_map(|l| l.strip_prefix("Example: ")) {
            run(&mut e, line);
        }
    }
    for code in [
        "fitcdiscr([1;2],[1;2;3])",
        "fitclinear([1;2],[1;1])",
        "fitclinear([1;2;3],[1;2;3])",
        "fitcdiscr([1;2],[1;2],'Gamma',2)",
        "fitclinear([1;2],[1;2],'Lambda',-1)",
        "fitcecoc([1;2],[1;2],'Coding','invalid')",
        "fitcensemble([1;2],[1;2],'LearnRate',0)",
        "fitcensemble([1;2],[1;2],'NumLearningCycles',1001)",
        "cvpartition(4,'KFold',5)",
        "cvpartition(4,'Holdout',1)",
        "fitcdiscr([0;1;2;3],[1;1;2;2],'uifigure',1)",
        "templateLinear('Solver','invalid')",
    ] {
        assert!(e.execute(code).is_err(), "{code}");
    }
}

#[test]
#[ignore = "release-profile timing; run alone with --ignored --nocapture"]
fn benchmark_classification_worker_policy() {
    let workers: usize = std::env::var("MATRUST_ML_BENCH_WORKERS")
        .unwrap_or_else(|_| "1".into())
        .parse()
        .unwrap();
    matrust::initialize_worker_pool(workers).unwrap();
    let mut e = Engine::default();
    run(
        &mut e,
        "rng(41);X=[randn(100,4)-3;randn(100,4);randn(100,4)+3];Y=[ones(100,1);2*ones(100,1);3*ones(100,1)];B=X(1:200,:);Z=Y(1:200);",
    );
    for (name, code) in [
        ("discriminant", "m=fitcdiscr(X,Y);[y,s]=predict(m,X);"),
        (
            "linear_logistic",
            "m=fitclinear(B,Z,'Learner','logistic');[y,s]=predict(m,B);",
        ),
        ("ecoc", "m=fitcecoc(X,Y);[y,s]=predict(m,X);"),
        (
            "bag",
            "rng(9);m=fitcensemble(X,Y,'NumLearningCycles',10,'Learners',templateTree('MaxNumSplits',7));[y,s]=predict(m,X);",
        ),
    ] {
        run(&mut e, code);
        let start = std::time::Instant::now();
        for _ in 0..10 {
            run(&mut e, code);
        }
        println!(
            "{name}: workers={workers}, n=300 (linear n=200), p=4, fit+predict mean_ms={:.3}",
            start.elapsed().as_secs_f64() * 100.
        );
    }
}

#[test]
#[rustfmt::skip]
fn seeded_iris_sized_discriminants_match_numpy() {
    let mut state=42u32;
    let mut rows=Vec::new();
    for center in [[-2.,-1.],[0.,2.],[2.,-1.]] {
        for _ in 0..50 {
            let mut point=[0.;2];
            for j in 0..2 {
                state=state.wrapping_mul(1664525).wrapping_add(1013904223);
                point[j]=center[j]+state as f64/4294967296.*2.-1.;
            }
            rows.push(point);
        }
    }
    let mut e=Engine::default();
    let data=(0..2).flat_map(|j|rows.iter().map(move|point|point[j])).collect();
    e.insert_array("X",Array::Numeric(matrust::Matrix::from_column_vec(150,2,data).unwrap())).unwrap();
    run(&mut e,"Y=[ones(50,1);2*ones(50,1);3*ones(50,1)];Q=[-0.5 0;0.5 0;0 1;1 -1];");
    for (kind,expected) in [
        ("linear",vec![0.7724774156979446, 0.003301118613720455, 4.441994977762488e-05, 5.195958846797711e-06, 0.22733361028230833, 0.5036516320052453, 0.9999504312839486, 5.3728352791485766e-06, 0.00018897401974697335, 0.4930472493810343, 5.1487662739254035e-06, 0.999989431205874]),
        ("quadratic",vec![0.9344150288931629, 0.025206862430796603, 0.00038345563513371173, 3.240754911674225e-05, 0.06552769519366422, 0.30063261085218185, 0.9996003445280393, 1.39554029717984e-06, 5.7275913173072406e-05, 0.6741605267170216, 1.6199836826920064e-05, 0.9999661969105862]),
    ] {
        run(&mut e,&format!("m=fitcdiscr(X,Y,'DiscrimType','{kind}');[y,s]=predict(m,Q);assert(m.NumObservations==150);"));
        close(&numbers(&e,"s"),&expected,1e-10);
    }
}
#[test]
fn crossval_one_class_fold_errors_without_panic() {
    // Review item 2: a 2-row, 2-class training set split in two folds trains each fold on one class.
    let mut e = Engine::default();
    for fit in [
        "fitcecoc([0;1],[1;2],'Coding','onevsall','Learners','knn')",
        "fitcecoc([0;1],[1;2],'Learners','tree')",
        "fitcecoc([0;1],[1;2])",
        "fitcknn([0;1],[1;2])",
        "fitctree([0;1],[1;2])",
        "fitcnb([0;1],[1;2],'DistributionNames','kernel')",
        "fitcdiscr([0;1],[1;2],'DiscrimType','pseudolinear')",
        "fitcsvm([0;1],[1;2])",
        "fitclinear([0;1],[1;2])",
        "fitcensemble([0;1],[1;2],'Method','Bag','NumLearningCycles',2)",
    ] {
        let code = format!("m={fit};c=crossval(m,'KFold',2);y=kfoldPredict(c);");
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| e.execute(&code)));
        assert!(caught.is_ok(), "{code} panicked");
    }
    assert!(e.execute("fitcecoc([0;1],[1;1])").is_err());
    let err = e
        .execute(
            "m=fitcecoc([0;1],[1;2],'Coding','onevsall','Learners','knn');c=crossval(m,'KFold',2);",
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("two response classes"), "{err}");
}
