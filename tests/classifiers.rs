//! scikit-learn references: tool/refs/classifiers/generate.py. Additional cases
//! use explicit hand-checkable neighbours, folds, counts and dual constraints.
use matrust::{Array, Engine};
use serde::Deserialize;
use std::collections::BTreeMap;

fn run(e: &mut Engine, source: &str) {
    e.execute(source)
        .unwrap_or_else(|err| panic!("{source}: {err}"));
}
fn numbers(e: &Engine, name: &str) -> Vec<f64> {
    match e.workspace().get(name).unwrap() {
        Array::Numeric(x) => x.data().to_vec(),
        x => panic!("unexpected {}", x.class()),
    }
}
fn close(a: &[f64], b: &[f64], tol: f64) {
    assert_eq!(a.len(), b.len());
    for (i, (&a, &b)) in a.iter().zip(b).enumerate() {
        assert!(
            (a - b).abs() <= tol,
            "element {i}: got {a}, expected {b}, tolerance {tol}"
        );
    }
}
#[derive(Deserialize)]
struct References {
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Case {
    name: String,
    code: String,
    values: BTreeMap<String, Expected>,
    atol: f64,
}
#[derive(Deserialize)]
struct Expected {
    rows: usize,
    cols: usize,
    data: Vec<f64>,
}
fn reference(name: &str) {
    let refs: References =
        serde_json::from_str(include_str!("fixtures/classifiers/reference.json")).unwrap();
    let case = refs.cases.into_iter().find(|c| c.name == name).unwrap();
    let mut e = Engine::default();
    run(&mut e, &case.code);
    for (name, value) in case.values {
        let w = e.workspace();
        let a = &w[&name];
        assert_eq!(
            (a.rows(), a.cols()),
            (value.rows, value.cols),
            "{} {name}",
            case.name
        );
        close(&numbers(&e, &name), &value.data, case.atol);
    }
}
macro_rules! ref_test {
    ($test:ident, $case:literal) => {
        #[test]
        fn $test() {
            reference($case);
        }
    };
}
ref_test!(crossval_kfold_sklearn_reference, "crossval_kfold");
ref_test!(knn_one_neighbor_reference, "knn_1");
ref_test!(knn_three_neighbor_scores_reference, "knn_3");
ref_test!(knn_euclidean_reference, "knn_euclidean");
ref_test!(knn_cityblock_reference, "knn_cityblock");
ref_test!(knn_chebychev_reference, "knn_chebychev");
ref_test!(knn_minkowski_reference, "knn_minkowski");
ref_test!(knn_standardize_reference, "knn_standardize");
ref_test!(tree_default_parent_reference, "tree_default");
ref_test!(tree_full_reference, "tree_full");
ref_test!(tree_max_splits_reference, "tree_one_split");
ref_test!(tree_min_leaf_reference, "tree_min_leaf");
ref_test!(tree_depth_two_reference, "tree_depth_two");
ref_test!(svm_overlap_linear_reference, "svm_overlap_linear");
ref_test!(svm_overlap_rbf_reference, "svm_overlap_rbf");
ref_test!(svm_linear_reference, "svm_linear");
ref_test!(svm_box_constraint_reference, "svm_box");
ref_test!(svm_rbf_decisions_reference, "svm_rbf");
ref_test!(svm_rbf_xor_reference, "svm_rbf_xor");
ref_test!(svm_standardize_reference, "svm_standardize");
ref_test!(
    nb_unbiased_parameters_and_posterior_reference,
    "nb_gaussian"
);

#[test]
fn knn_ties_use_sorted_classes_and_training_rows() {
    let mut e = Engine::default();
    run(
        &mut e,
        "m=fitcknn([0;2],[20;10],'NumNeighbors',2);[y,s]=predict(m,[1;3]);classes=m.ClassNames;",
    );
    assert_eq!(numbers(&e, "y"), vec![10., 10.]);
    assert_eq!(numbers(&e, "classes"), vec![10., 20.]);
    assert_eq!(numbers(&e, "s"), vec![0.5; 4]);
}
#[test]
fn cellstr_labels_properties_and_loss() {
    let mut e = Engine::default();
    run(
        &mut e,
        "m=fitcknn([0;1;4;5],{'low';'low';'high';'high'});y=predict(m,[.1;4.1]);n=m.NumObservations;z=double(strcmp(y,{'low';'high'}));loss=resubLoss(m);",
    );
    assert_eq!(numbers(&e, "n"), vec![4.]);
    assert_eq!(numbers(&e, "z"), vec![1., 1.]);
    assert_eq!(numbers(&e, "loss"), vec![0.]);
}
#[test]
fn string_logical_and_categorical_labels_preserve_type() {
    let mut e = Engine::default();
    run(
        &mut e,
        r#"m=fitcknn([0;1;4;5],["a";"a";"b";"b"]);y=predict(m,[.1;4.1]);"#,
    );
    assert!(matches!(e.workspace()["y"], Array::Str(_)));
    run(
        &mut e,
        "m=fitcnb([0;1;4;5],[false;false;true;true]);y=predict(m,[.1;4.1]);z=double(y);loss=resubLoss(m);",
    );
    assert!(matches!(e.workspace()["y"], Array::Logical(_)));
    assert_eq!(numbers(&e, "z"), vec![0., 1.]);
    run(
        &mut e,
        "m=fitctree([0;1;4;5],categorical({'a';'a';'b';'b'}),'MinParentSize',2);y=predict(m,[.1;4.1]);z=double(y);loss=resubLoss(m);",
    );
    assert!(matches!(e.workspace()["y"], Array::Categorical(_)));
    assert_eq!(numbers(&e, "z"), vec![1., 2.]);
    assert_eq!(numbers(&e, "loss"), vec![0.]);
}
#[test]
fn table_named_response_and_external_response_all_classifiers() {
    for fit in ["fitcknn", "fitctree", "fitcsvm", "fitcnb"] {
        let mut e = Engine::default();
        let options = if fit == "fitctree" {
            ",'MinParentSize',2"
        } else {
            ""
        };
        run(
            &mut e,
            &format!(
                "t=table([0;1;4;5],[0;2;3;5],[1;1;2;2],'VariableNames',{{'a','b','group'}});m={fit}(t,'group'{options});[y,s]=predict(m,t);loss=resubLoss(m);z=double(strcmp(m.ResponseName,'group'));pn=m.PredictorNames;z2=double(strcmp(pn,{{'a';'b'}}));"
            ),
        );
        assert_eq!(numbers(&e, "y"), vec![1., 1., 2., 2.]);
        assert_eq!(numbers(&e, "loss"), vec![0.]);
        assert_eq!(numbers(&e, "z"), vec![1.]);
        assert_eq!(numbers(&e, "z2"), vec![1., 1.]);
        run(
            &mut e,
            &format!(
                "t=table([0;1;4;5],[0;2;3;5],'VariableNames',{{'a','b'}});m={fit}(t,[1;1;2;2]{options});q=table([0;5],[0;5],'VariableNames',{{'b','a'}});y=predict(m,q);"
            ),
        );
        assert_eq!(numbers(&e, "y"), vec![1., 2.]);
    }
}
#[test]
fn svm_support_properties_and_duplicate_opposite_labels() {
    let mut e = Engine::default();
    run(
        &mut e,
        "m=fitcsvm([-2;-1;1;2],[1;1;2;2]);a=m.Alpha;b=m.Bias;sv=m.SupportVectors;beta=m.Beta;[y,s]=predict(m,[-1;1]);",
    );
    close(&numbers(&e, "a"), &[0.5, 0.5], 1e-7);
    close(&numbers(&e, "b"), &[0.], 1e-7);
    close(&numbers(&e, "sv"), &[-1., 1.], 1e-7);
    close(&numbers(&e, "beta"), &[1.], 1e-7);
    close(&numbers(&e, "s"), &[1., -1., -1., 1.], 1e-7);
    run(
        &mut e,
        "m=fitcsvm([0;0],[1;2],'BoxConstraint',.3);a=m.Alpha;b=m.Bias;[y,s]=predict(m,0);",
    );
    close(&numbers(&e, "a"), &[0.3, 0.3], 1e-7);
    close(&numbers(&e, "s"), &[0., 0.], 1e-7);
}
#[test]
fn tree_zero_split_and_leaf_class_fractions() {
    let mut e = Engine::default();
    run(
        &mut e,
        "m=fitctree([0;1;2;3],[2;1;1;1],'MaxNumSplits',0);[y,s]=predict(m,[0;3]);n=m.NumNodes;",
    );
    assert_eq!(numbers(&e, "y"), vec![1., 1.]);
    assert_eq!(numbers(&e, "s"), vec![0.75, 0.75, 0.25, 0.25]);
    assert_eq!(numbers(&e, "n"), vec![1.]);
}
#[test]
fn leaveout_refits_instead_of_reusing_training_predictions() {
    let mut e = Engine::default();
    run(
        &mut e,
        "m=fitcknn([0;1;2;3;4;5],[1;2;1;2;1;2]);r=resubLoss(m);cv=crossval(m,'Leaveout','on');[y,s]=kfoldPredict(cv);loss=kfoldLoss(cv);z=double(strcmp(class(cv),'ClassificationPartitionedModel'));k=cv.KFold;",
    );
    assert_eq!(numbers(&e, "r"), vec![0.]);
    assert_eq!(numbers(&e, "loss"), vec![1.]);
    assert_eq!(numbers(&e, "y"), vec![2., 1., 2., 1., 2., 1.]);
    assert_eq!(numbers(&e, "z"), vec![1.]);
    assert_eq!(numbers(&e, "k"), vec![6.]);
}
#[test]
fn crossval_rng_reproducible_default_and_kfold() {
    let mut e = Engine::default();
    run(
        &mut e,
        "x=(0:19)';y=[ones(10,1);2*ones(10,1)];m=fitcknn(x,y);rng(0);a=crossval(m);[ya,sa]=kfoldPredict(a);la=kfoldLoss(a);rng(0);b=crossval(m);[yb,sb]=kfoldPredict(b);lb=kfoldLoss(b);k=a.KFold;rng(7);c=crossval(m,'KFold',4);lc=kfoldLoss(c);",
    );
    assert_eq!(numbers(&e, "ya"), numbers(&e, "yb"));
    assert_eq!(numbers(&e, "sa"), numbers(&e, "sb"));
    assert_eq!(numbers(&e, "la"), numbers(&e, "lb"));
    assert_eq!(numbers(&e, "k"), vec![10.]);
    assert!((0.0..=1.0).contains(&numbers(&e, "lc")[0]));
}
#[test]
fn crossval_holdout_only_scores_held_out_rows() {
    let mut e = Engine::default();
    run(
        &mut e,
        "m=fitcknn((0:11)',[ones(6,1);2*ones(6,1)]);rng(0);cv=crossval(m,'Holdout',.25);[y,s]=kfoldPredict(cv);loss=kfoldLoss(cv);",
    );
    let y = numbers(&e, "y");
    assert_eq!(y.iter().filter(|x| x.is_finite()).count(), 3);
    let errors = y
        .iter()
        .enumerate()
        .filter(|&(i, &p)| p.is_finite() && p != if i < 6 { 1. } else { 2. })
        .count();
    assert_eq!(numbers(&e, "loss"), vec![errors as f64 / 3.]);
    let s = numbers(&e, "s");
    for i in 0..12 {
        if y[i].is_nan() {
            assert!(s[i].is_nan() && s[i + 12].is_nan());
        } else {
            assert_eq!(s[i] + s[i + 12], 1.);
        }
    }
}
#[test]
fn fold_scores_map_classes_missing_from_training() {
    let mut e = Engine::default();
    run(
        &mut e,
        "m=fitcknn([0;2;4],[1;2;3]);cv=crossval(m,'Leaveout','on');[y,s]=kfoldPredict(cv);loss=kfoldLoss(cv);",
    );
    assert_eq!(numbers(&e, "y"), vec![2., 1., 2.]);
    assert_eq!(numbers(&e, "s"), vec![0., 1., 0., 1., 0., 1., 0., 0., 0.]);
    assert_eq!(numbers(&e, "loss"), vec![1.]);
}
#[test]
fn crossval_refits_tree_nb_and_svm_with_original_options() {
    for fit in ["fitctree", "fitcnb", "fitcsvm"] {
        let mut e = Engine::default();
        let opts = match fit {
            "fitctree" => ",'MinParentSize',2",
            "fitcsvm" => ",'KernelFunction','rbf','Standardize',true",
            _ => "",
        };
        run(
            &mut e,
            &format!(
                "m={fit}([0;1;2;3;10;11;12;13],[1;1;1;1;2;2;2;2]{opts});cv=crossval(m,'Leaveout','on');y=kfoldPredict(cv);loss=kfoldLoss(cv);"
            ),
        );
        assert_eq!(numbers(&e, "y"), vec![1., 1., 1., 1., 2., 2., 2., 2.]);
        assert_eq!(numbers(&e, "loss"), vec![0.]);
    }
}
#[test]
fn classifier_assignment_preserves_immutable_model() {
    let mut e = Engine::default();
    run(
        &mut e,
        "x=[0;2];m=fitcknn(x,[1;2]);n=m;x(1)=99;clear m;y=predict(n,[0;2]);z=n.X;",
    );
    assert_eq!(numbers(&e, "y"), vec![1., 2.]);
    assert_eq!(numbers(&e, "z"), vec![0., 2.]);
}
#[test]
fn unsupported_options_sizes_and_invalid_forms_error() {
    let mut e = Engine::default();
    for (code, expected) in [
        ("fitcsvm([0;1;2],[1;2;3])", "fitcecoc"),
        ("fitcknn([0;1],[1;2;3])", "same number"),
        ("fitcknn([0;1],[1;2],'NumNeighbors',0)", "positive integer"),
        ("fitcknn([0;1],[1;2],'NumNeighbors',3)", "exceed"),
        ("fitctree([0;1],[1;2],'SplitCriterion','deviance')", "gdi"),
        (
            "fitcnb([0;1],[1;2],'DistributionNames','kernel')",
            "Gaussian",
        ),
        ("fitcnb([0;1],[1;2],'Prior','uniform')", "empirical"),
        ("fitcsvm([0;1],[1;2],'BoxConstraint',0)", "positive"),
        ("fitcsvm([0;1],[1;2],'KernelScale',0)", "positive"),
        ("fitcknn([0;NaN],[1;2])", "finite"),
        ("fitcknn([0;1],[1;NaN])", "finite"),
        ("m=fitcknn([0;1],[1;2]);predict(m,[0 1])", "same number"),
        ("m=fitcknn([0;1],[1;2]);crossval(m,'KFold',3)", "KFold"),
        ("m=fitcknn([0;1],[1;2]);crossval(m,'Holdout',1)", "Holdout"),
        ("m=fitcknn([0;1],[1;2]);crossval(m)", "folds"),
        ("[a,b]=predict()", "model"),
    ] {
        let err = e.execute(code).unwrap_err();
        assert!(err.contains(expected), "{code}: {err}");
    }
}
#[test]
fn classifier_help_pages_exist() {
    let mut e = Engine::default();
    for name in [
        "fitcknn",
        "fitctree",
        "fitcsvm",
        "fitcnb",
        "crossval",
        "predict",
        "resubLoss",
        "kfoldPredict",
        "kfoldLoss",
    ] {
        let out = e.execute(&format!("help {name}")).unwrap();
        assert!(out.output.concat().contains("Syntax:"), "{name}");
    }
}

#[test]
fn every_classifier_preserves_all_response_label_types() {
    for fit in ["fitcknn", "fitctree", "fitcsvm", "fitcnb"] {
        for (labels, kind) in [
            ("[10;10;20;20]", "double"),
            ("single([10;10;20;20])", "single"),
            ("int8([10;10;20;20])", "int8"),
            ("[false;false;true;true]", "logical"),
            ("{'a';'a';'b';'b'}", "cell"),
            (r#"["a";"a";"b";"b"]"#, "string"),
            ("categorical({'a';'a';'b';'b'})", "categorical"),
        ] {
            let mut e = Engine::default();
            let options = if fit == "fitctree" {
                ",'MinParentSize',2"
            } else {
                ""
            };
            run(
                &mut e,
                &format!(
                    "m={fit}([0;1;4;5],{labels}{options});y=predict(m,[.1;4.1]);loss=resubLoss(m);"
                ),
            );
            let workspace = e.workspace();
            assert_eq!(workspace["y"].class(), kind, "{fit} {kind}");
            assert_eq!(numbers(&e, "loss"), vec![0.]);
            match &workspace["y"] {
                Array::Numeric(m) => assert_eq!(m.data(), &[10., 20.]),
                Array::Logical(m) => assert_eq!(m.data(), &[0, 1]),
                Array::Typed(v) => {
                    assert_eq!(v.real(0), 10.);
                    assert_eq!(v.real(1), 20.);
                }
                Array::Categorical(c) => {
                    assert_eq!(c.text(0), "a");
                    assert_eq!(c.text(1), "b");
                }
                Array::Str(s) => {
                    assert_eq!(s.data()[0].as_deref(), Some("a"));
                    assert_eq!(s.data()[1].as_deref(), Some("b"));
                }
                _ => {
                    run(&mut e, "z=double(strcmp(y,{'a';'b'}));");
                    assert_eq!(numbers(&e, "z"), vec![1., 1.]);
                }
            }
        }
    }
    let mut e = Engine::default();
    run(&mut e, r#"m=fitcknn(1,"a");y=predict(m,1);"#);
    assert!(matches!(e.workspace()["y"], Array::Str(_)));
}
#[test]
fn svm_class_sign_is_independent_of_training_row_order() {
    let mut e = Engine::default();
    run(
        &mut e,
        "m=fitcsvm([1;2;-2;-1],[20;20;10;10]);[y,s]=predict(m,[-1;1]);beta=m.Beta;",
    );
    assert_eq!(numbers(&e, "y"), vec![10., 20.]);
    close(&numbers(&e, "s"), &[1., -1., -1., 1.], 1e-7);
    close(&numbers(&e, "beta"), &[1.], 1e-7);
}

#[test]
fn review_linear_svm_kernel_scale_applies_to_fit_and_predict() {
    let mut e = Engine::default();
    run(
        &mut e,
        "m=fitcsvm([-1;1],[1;2],'KernelScale',10); [y,s]=predict(m,1); a=m.Alpha; b=m.Beta;",
    );
    close(&numbers(&e, "s"), &[-0.02, 0.02], 1e-12);
    close(&numbers(&e, "a"), &[1., 1.], 1e-12);
    close(&numbers(&e, "y"), &[2.], 0.);
    // Beta acts on the scaled predictors (X/KernelScale).
    close(&numbers(&e, "b"), &[0.2], 1e-12);
    reference("svm_linear_scaled");
}

#[test]
fn review_kfold_stratifies_classes_and_is_rng_reproducible() {
    let mut e = Engine::default();
    run(
        &mut e,
        "m=fitcsvm([-2;-1;1;2],[1;1;2;2]); rng(1); cv=crossval(m,'KFold',2); [a,sa]=kfoldPredict(cv); la=kfoldLoss(cv); rng(1); cv=crossval(m,'KFold',2); [b,sb]=kfoldPredict(cv); assert(isequal(a,b)); assert(isequal(sa,sb)); assert(isequal(a,[1;1;2;2])); assert(la==0);",
    );
    // An imbalanced multiclass partition still distributes every observation.
    run(
        &mut e,
        "x=[-3;-2;-1;1;2;3;10;11]; y=[1;1;1;2;2;2;3;3]; m=fitcknn(x,y,'NumNeighbors',3); rng(3); cv=crossval(m,'KFold',3); [y,s]=kfoldPredict(cv); assert(size(s,1)==8); assert(all(isfinite(s(:))));",
    );
}
