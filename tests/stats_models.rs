//! Independent NumPy/SciPy and analytic comparisons; regenerate with
//! bash tool/ref-python.sh tool/refs/stats_models/generate.py.
use matrust::{Array, Engine};
use serde::Deserialize;

#[derive(Deserialize)]
struct Reference {
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Case {
    name: String,
    code: String,
    rows: usize,
    cols: usize,
    data: Vec<f64>,
    atol: f64,
    rtol: f64,
}
fn array(e: &Engine, name: &str) -> Vec<f64> {
    match e.workspace().get(name).unwrap() {
        Array::Numeric(m) => m.data().to_vec(),
        other => panic!("unexpected {}", other.class()),
    }
}
fn run(e: &mut Engine, code: &str) {
    e.execute(code)
        .unwrap_or_else(|err| panic!("{code}: {err}"));
}
fn check_family(prefix: &str) {
    let reference: Reference =
        serde_json::from_str(include_str!("fixtures/stats_models/reference.json")).unwrap();
    for c in reference
        .cases
        .into_iter()
        .filter(|c| c.name.starts_with(prefix))
    {
        let mut e = Engine::default();
        run(&mut e, &c.code);
        let workspace = e.workspace();
        let got = workspace.get("zz").unwrap();
        assert_eq!(
            (got.rows(), got.cols()),
            (c.rows, c.cols),
            "{} shape",
            c.name
        );
        let got = array(&e, "zz");
        for (i, (&a, &b)) in got.iter().zip(&c.data).enumerate() {
            assert!(
                (a - b).abs() <= c.atol + c.rtol * b.abs(),
                "{}[{i}]: got {a:.17e}, expected {b:.17e}",
                c.name
            );
        }
    }
}
#[test]
fn regress_reference() {
    check_family("regress_");
}
#[test]
fn ridge_reference() {
    check_family("ridge_");
}
#[test]
fn lasso_reference() {
    check_family("lasso_");
}
#[test]
fn pca_reference() {
    check_family("pca_");
    check_family("pcacov");
}
#[test]
fn distances_reference() {
    check_family("pdist_");
    check_family("pdist2_");
    check_family("squareform");
}
#[test]
fn linkage_reference() {
    check_family("linkage_");
    check_family("inconsistent");
}
#[test]
fn partitions_reference() {
    check_family("kmeans_");
    check_family("kmedoids_");
}
#[test]
fn groups_neighbors_reference() {
    check_family("knn_");
    check_family("confusionmat_");
}

#[test]
fn regress_rank_deficiency_and_missing_rows() {
    let mut e = Engine::default();
    let output = e
        .execute("[b,bi,r,ri,s]=regress([1;2;NaN;4],[1 0 0;1 1 2;1 2 4;1 3 6]);")
        .unwrap();
    assert!(output.output.join("\n").contains("rank deficient"));
    assert!(array(&e, "b").contains(&0.));
    assert!(array(&e, "r")[2].is_nan());
    run(&mut e, "fit=[1 0 0;1 1 2;1 2 4;1 3 6]*b;");
    for (i, v) in array(&e, "fit").iter().enumerate() {
        assert!((v - (i + 1) as f64).abs() < 1e-12);
    }
}
#[test]
fn typed_inputs_reach_every_output_count() {
    let mut e = Engine::default();
    run(
        &mut e,
        "b=regress(single([1;2;4]),int32([1 0;1 1;1 3]));[bb,bi]=regress(single([1;2;4]),int32([1 0;1 1;1 3]));",
    );
    assert_eq!(array(&e, "b"), array(&e, "bb"));
    run(&mut e, "d=pdist(int32([0 0;3 4]));");
    assert_eq!(array(&e, "d"), [5.]);
}
#[test]
fn lasso_options_and_fitinfo() {
    let mut e = Engine::default();
    run(
        &mut e,
        "[b,f]=lasso([0;1;2;3],[2;5;8;11],'Lambda',[0 .1],'Weights',[1 2 2 1],'RelTol',1e-12);a=f.Alpha;l=f.Lambda;inter=f.Intercept;df=f.DF;m=f.MSE;n=f.PredictorNames;u=f.UseCovariance;",
    );
    assert_eq!(array(&e, "l"), [0., 0.1]);
    assert_eq!(array(&e, "a"), [1.]);
    assert!((array(&e, "inter")[0] - 2.).abs() < 1e-10);
    let err = e.execute("lasso([1;2],[1;2],'CV',2)").unwrap_err();
    assert!(err.contains("not supported yet"));
    assert!(
        e.execute("lassoPlot(1,struct())")
            .unwrap_err()
            .contains("not supported yet")
    );
}
#[test]
fn pca_nan_components_eigen_and_sign() {
    let mut e = Engine::default();
    run(
        &mut e,
        "[c,s,l,t,ex,m]=pca([1 2;2 4;NaN 1;3 6],'NumComponents',1);[ce,se,le]=pca([1 2;2 4;3 6],'Algorithm','eig');",
    );
    assert_eq!(array(&e, "c").len(), 2);
    assert!(array(&e, "s")[2].is_nan());
    assert!((array(&e, "l")[0] - 5.).abs() < 1e-12);
    assert!(array(&e, "c")[1] > 0.);
    assert!((array(&e, "ce")[0] - array(&e, "c")[0]).abs() < 1e-12);
    assert!(
        e.execute("pca([1 2;NaN 1;3 4],'Rows','all')")
            .unwrap_err()
            .contains("NaN")
    );
    assert!(
        e.execute("pca(eye(2),'Rows','pairwise')")
            .unwrap_err()
            .contains("not supported yet")
    );
}
#[test]
fn distance_selection_ties_handle_and_squareform_errors() {
    let mut e = Engine::default();
    run(
        &mut e,
        "[d,i]=pdist2([0;2;4],[1;3],'euclidean','Smallest',2);[dl,il]=pdist2([0;2;4],1,'euclidean','Largest',2);",
    );
    assert_eq!(array(&e, "i"), [1., 2., 2., 3.]);
    assert_eq!(array(&e, "d"), [1.; 4]);
    assert_eq!(array(&e, "il"), [3., 1.]);
    run(
        &mut e,
        "d=pdist([0 0;3 4;3 0],@(xi,xj) sqrt(sum((xj-repmat(xi,size(xj,1),1)).^2,2)));q=pdist2([0 0;3 4],[3 0],@(xi,xj) sqrt(sum((xj-repmat(xi,size(xj,1),1)).^2,2)));v=squareform(squareform([5 3 4]));",
    );
    assert_eq!(array(&e, "d"), [5., 3., 4.]);
    assert_eq!(array(&e, "q"), [3., 4.]);
    assert_eq!(array(&e, "v"), [5., 3., 4.]);
    assert!(
        e.execute("squareform([0 1;2 0])")
            .unwrap_err()
            .contains("symmetric")
    );
    assert!(
        e.execute("squareform([1 1;1 0])")
            .unwrap_err()
            .contains("diagonal")
    );
    run(&mut e, "d=pdist([0 0;NaN 1;3 4]);");
    let d = array(&e, "d");
    assert!(d[0].is_nan() && d[2].is_nan());
    assert_eq!(d[1], 5.);
}
#[test]
fn hierarchy_ties_cuts_and_validation() {
    let mut e = Engine::default();
    run(
        &mut e,
        "z=linkage([1 1 1],'single');t=cluster(z,'MaxClust',2);td=cluster(z,'Cutoff',1,'Criterion','distance');ti=cluster(z,'Cutoff',0);r=inconsistent(z,1);",
    );
    assert_eq!(array(&e, "z"), [1., 3., 2., 4., 1., 1.]);
    assert_eq!(array(&e, "t"), [1., 1., 1.]);
    assert_eq!(array(&e, "td"), [2., 3., 1.]);
    assert_eq!(array(&e, "ti"), [2., 3., 1.]);
    assert!(e.execute("cluster([1 1 1],'MaxClust',2)").is_err());
    assert!(
        e.execute("linkage([0 0;1 2],'ward','cityblock')")
            .unwrap_err()
            .contains("Euclidean")
    );
}
#[test]
fn kmeans_reproducible_starts_empty_actions_and_display() {
    let mut e = Engine::default();
    for start in ["plus", "sample", "uniform"] {
        run(
            &mut e,
            &format!(
                "rng(7);[i,c]=kmeans([0;1;9;10],2,'Start','{start}','Replicates',3);rng(7);[ii,cc]=kmeans([0;1;9;10],2,'Start','{start}','Replicates',3);"
            ),
        );
        assert_eq!(array(&e, "i"), array(&e, "ii"));
        assert_eq!(array(&e, "c"), array(&e, "cc"));
        let mut c = array(&e, "c");
        c.sort_by(f64::total_cmp);
        assert_eq!(c, [0.5, 9.5]);
    }
    assert!(
        e.execute("kmeans([0;1;9;10],2,'Start',[0;0],'EmptyAction','error')")
            .unwrap_err()
            .contains("empty")
    );
    run(
        &mut e,
        "[i,c]=kmeans([0;1;9;10],2,'Start',[0;0],'EmptyAction','singleton','OnlinePhase','on');[id,cd,sd]=kmeans([0;1;9;10],2,'Start',[0;0],'EmptyAction','drop');",
    );
    assert!(array(&e, "cd")[1].is_nan());
    assert!(array(&e, "sd")[1].is_nan());
    let result = e
        .execute("kmeans([0;1;9;10],2,'Start',[0;10],'Display','iter');")
        .unwrap();
    assert!(result.output.join("\n").contains("replicate"));
    let result = e
        .execute("kmeans([0;1;9;10],2,'Start',[0;10],'MaxIter',1);")
        .unwrap();
    assert!(result.output.join("\n").contains("MaxIter"));
}
#[test]
fn kmedoids_starts_and_handles() {
    let mut e = Engine::default();
    run(
        &mut e,
        "[i,c,s,d,m]=kmedoids([0;1;2;9;10;11],2,'Start',[0;11],'Distance',@(xi,xj) sum(abs(xj-xi),2));",
    );
    assert_eq!(array(&e, "c"), [1., 10.]);
    assert_eq!(array(&e, "m"), [2., 5.]);
    for start in ["plus", "sample", "cluster"] {
        run(
            &mut e,
            &format!(
                "rng(5);[i,c]=kmedoids([0;1;2;9;10;11],2,'Start','{start}','Replicates',2);rng(5);[ii,cc]=kmedoids([0;1;2;9;10;11],2,'Start','{start}','Replicates',2);"
            ),
        );
        assert_eq!(array(&e, "c"), array(&e, "cc"));
    }
    assert!(
        e.execute("kmedoids([0;1;2],2,'Algorithm','clara')")
            .unwrap_err()
            .contains("not supported yet")
    );
}
#[test]
fn neighbors_include_ties_and_nan_rows() {
    let mut e = Engine::default();
    run(
        &mut e,
        "[i,d]=knnsearch([0;2;NaN;4],[1;3],'K',1,'IncludeTies',true);a=i{1};b=i{2};[in,dn]=knnsearch([0;NaN;2],[1;NaN],'K',1,'SortIndices',false);",
    );
    assert_eq!(array(&e, "a"), [1., 2.]);
    assert_eq!(array(&e, "b"), [2., 4.]);
    assert_eq!(array(&e, "in")[0], 1.);
    assert!(array(&e, "in")[1].is_nan());
}
#[test]
fn confusion_group_classes_order_and_ignored_entries() {
    let mut e = Engine::default();
    run(
        &mut e,
        "[c,o]=confusionmat({'a';'b';''},{'b';'b';'a'},'Order',{'b';'a'});cs=confusionmat([\"a\";\"b\";\"\"],[\"b\";\"b\";\"a\"],'Order',[\"b\";\"a\"]);cn=confusionmat([1 NaN 2],[1 2 1]);cl=confusionmat([true false],[true true]);ch=confusionmat('aba','bba');",
    );
    assert_eq!(array(&e, "c"), [1., 1., 0., 0.]);
    assert_eq!(array(&e, "cs"), array(&e, "c"));
    assert_eq!(array(&e, "cn"), [1., 1., 0., 0.]);
    assert_eq!(array(&e, "ch"), [0., 0., 1., 0.]);
    run(
        &mut e,
        "ch=confusionmat(['aa';'bb';'aa'],['bb';'bb';'aa']);",
    );
    assert_eq!(array(&e, "ch"), [1., 0., 1., 1.]);
    assert!(e.execute("confusionmat([1 2],[1 2],'Order',1)").is_err());
}
#[test]
fn names_help_and_unsupported_nd() {
    let mut e = Engine::default();
    for name in [
        "regress",
        "ridge",
        "lasso",
        "pca",
        "pcacov",
        "kmeans",
        "kmedoids",
        "pdist",
        "pdist2",
        "squareform",
        "linkage",
        "cluster",
        "inconsistent",
        "knnsearch",
        "confusionmat",
    ] {
        assert!(matrust::session::FUNCTIONS.contains(&name));
        assert!(matrust::help::summary(name).is_some());
    }
    assert!(e.execute("pdist(ones(2,2,2))").is_err());
}

#[test]
fn pair_count_caps_fail_before_allocating_outputs() {
    let mut e = Engine::default();
    // Pair and search outputs sit above the 48 Mi element cap; inputs stay tiny.
    // linkage(5000) is refused by its n-by-10n workspace, which is still over the cap.
    for (name, code) in [
        ("pdist", "pdist(zeros(10034,1))"),
        ("pdist2", "pdist2(zeros(7095,1),zeros(7095,1))"),
        ("linkage", "linkage(zeros(5000,2))"),
        ("knnsearch", "knnsearch(zeros(7095,1),zeros(7095,1))"),
    ] {
        let err = e.execute(code).unwrap_err();
        assert!(
            err.contains(name) && err.contains("too large"),
            "{code}: {err}"
        );
    }
}

#[test]
fn spherical_and_cityblock_kmeans_fixed_start() {
    let mut e = Engine::default();
    run(
        &mut e,
        "[i,c]=kmeans([0;2;3;10;11;15],2,'Start',[0;15],'Distance','cityblock');",
    );
    assert_eq!(array(&e, "i"), [1., 1., 1., 2., 2., 2.]);
    assert_eq!(array(&e, "c"), [2., 11.]);
    for metric in ["cosine", "correlation"] {
        run(
            &mut e,
            &format!(
                "[i,c]=kmeans([1 2 3;2 4 6;3 2 1;6 4 2],2,'Start',[1 2 3;3 2 1],'Distance','{metric}');"
            ),
        );
        assert_eq!(array(&e, "i"), [1., 1., 2., 2.]);
    }
}

#[test]
fn pca_weights_and_full_space_tsquared() {
    let mut e = Engine::default();
    run(
        &mut e,
        "[c,s,l,t]=pca([1 2;2 1;4 5;8 4]);[cr,sr,lr,tr]=pca([1 2;2 1;4 5;8 4],'NumComponents',1);[cv,sv,lv]=pca([1 2;2 1;4 5;8 4],'VariableWeights','variance');[cw,sw,lw,tw,ew,mw]=pca([1 2;2 1;4 5;8 4],'Weights',[1 2 3 4]);",
    );
    for (a, b) in array(&e, "t").iter().zip(array(&e, "tr")) {
        assert!((a - b).abs() < 1e-12);
    }
    let expected = [
        2.188987589427283,
        1.290994448735806,
        2.188987589427283,
        -1.290994448735806,
    ];
    for (a, b) in array(&e, "cv").iter().zip(expected) {
        assert!((a - b).abs() < 1e-12);
    }
    for (a, b) in array(&e, "mw").iter().zip([4.9, 3.5]) {
        assert!((a - b).abs() < 1e-12);
    }
}

#[test]
fn clustering_consumes_the_documented_rand_stream() {
    let mut e = Engine::default();
    for (call, draws) in [
        ("kmeans([0;1;9;10],2,'Start','plus','Replicates',3)", 6),
        ("kmeans([0;1;9;10],2,'Start','sample','Replicates',3)", 6),
        (
            "kmeans([0 0;1 1;9 9;10 10],2,'Start','uniform','Replicates',2)",
            8,
        ),
        ("kmeans([0;1;9;10],2,'Start',[0;10])", 0),
        ("kmedoids([0;1;2;9;10;11],2)", 0),
        (
            "kmedoids([0;1;2;9;10;11],2,'Start','plus','Replicates',2)",
            4,
        ),
        ("kmedoids([0;1;2;9;10;11],2,'Start','cluster')", 2),
    ] {
        run(
            &mut e,
            &format!("rng(19);v={call};actual=rand;rng(19);discard=rand(1,{draws});expected=rand;"),
        );
        assert_eq!(array(&e, "actual"), array(&e, "expected"), "{call}");
    }
}

#[test]
fn distance_parameters_custom_direction_and_logical_group_order() {
    let mut e = Engine::default();
    run(
        &mut e,
        "d=pdist([0 0;3 4],'seuclidean',[3 4]);m=pdist([0 0;3 4],'minkowski',1);h=pdist([0 0;3 4],'mahalanobis',[9 0;0 16]);q=pdist2([1;2],[3;4],@(xi,yj) xi(1)+2*yj(:,1));[c,o]=confusionmat([true false],[true true]);",
    );
    assert!((array(&e, "d")[0] - 2_f64.sqrt()).abs() < 1e-12);
    assert_eq!(array(&e, "m"), [7.]);
    assert!((array(&e, "h")[0] - 2_f64.sqrt()).abs() < 1e-12);
    assert_eq!(array(&e, "q"), [7., 8., 9., 10.]);
    assert!(matches!(e.workspace().get("o"), Some(Array::Logical(_))));
    run(
        &mut e,
        "[b,f]=lasso(eye(2),[2;-1],'Intercept',false,'LambdaRatio',0,'NumLambda',3);l=f.Lambda;",
    );
    assert_eq!(array(&e, "l")[0], 0.);
}

#[test]
fn pca_uncentered_and_full_economy_reconstruction() {
    let mut e = Engine::default();
    run(
        &mut e,
        "[c,s,l,t,ex,m]=pca([1 0;0 2],'Centered',false);[cf,sf,lf]=pca([1 2 3 4;2 4 6 8],'Economy',false);back=sf*cf';",
    );
    assert_eq!(array(&e, "m"), [0., 0.]);
    assert_eq!(array(&e, "l"), [2., 0.5]);
    assert_eq!(array(&e, "lf").len(), 4);
    for (a, b) in array(&e, "back")
        .iter()
        .zip([-0.5, 0.5, -1., 1., -1.5, 1.5, -2., 2.])
    {
        assert!((a - b).abs() < 1e-12);
    }
    run(
        &mut e,
        "t=cluster([1 2 1;3 4 5],'MaxClust',2,'Criterion','inconsistent');",
    );
    assert_eq!(array(&e, "t"), [2., 2., 1.]);
}

#[test]
fn invalid_spherical_centers_and_restored_output_caps_are_errors() {
    let mut e = Engine::default();
    assert!(
        e.execute("kmeans([1 2;2 1],2,'Distance','cosine','Start',[0 0;1 2])")
            .unwrap_err()
            .contains("undefined")
    );
    // Half the rows are NaN. Valid rows times k stays under the 48 Mi element
    // cap (5017^2 = 25,170,289); restoring those clusters over every row does
    // not (10034*5017 = 50,340,578). Fail before building centers or D.
    let error = e
        .execute("kmeans([zeros(5017,1);zeros(5017,1)+NaN],5017)")
        .unwrap_err();
    assert!(error.contains("too large"), "{error}");
}

#[test]
fn squareform_scalar_direction_defaults_to_matrix() {
    let mut e = Engine::default();
    run(&mut e, "m=squareform(0);v=squareform(0,'tovector');");
    assert_eq!(array(&e, "m"), [0.; 4]);
    assert!(array(&e, "v").is_empty());
}

#[test]
fn empty_groups_and_complete_explicit_order() {
    let mut e = Engine::default();
    run(&mut e, "c=confusionmat([],[]);ch=confusionmat('a ','a');");
    assert!(array(&e, "c").is_empty());
    assert_eq!(array(&e, "ch"), [1.]);
    assert!(
        e.execute("confusionmat([1 2],[NaN 2],'Order',2)")
            .unwrap_err()
            .contains("Order")
    );
}

#[test]
fn configured_worker_counts_preserve_condensed_distances() {
    let mut outputs = Vec::new();
    for workers in [1, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        outputs.push(pool.install(|| {
            let mut e = Engine::default();
            run(&mut e, "rng(27);X=randn(300,12);d=pdist(X);");
            array(&e, "d")
        }));
    }
    assert_eq!(outputs[0], outputs[1]);
}

#[test]
fn clustering_display_obeys_the_host_output_budget() {
    let mut limits = matrust::engine::Limits::browser();
    limits.output_entries = Some(1);
    let mut e = Engine::with_limits(limits);
    let error = e
        .execute("kmeans([0;1;9;10],2,'Start',[0;10],'Display','iter');")
        .unwrap_err();
    assert!(error.contains("output exceeds"), "{error}");
}

#[test]
fn linkage_one_variable_observations_and_euclidean_condensed_validation() {
    let mut e = Engine::default();
    run(&mut e, "z=linkage([0;1;3]);");
    assert_eq!(array(&e, "z"), [1., 3., 2., 4., 1., 2.]);
    for method in ["centroid", "median", "ward"] {
        let error = e
            .execute(&format!("linkage([1 1 3],'{method}')"))
            .unwrap_err();
        assert!(error.contains("Euclidean"), "{error}");
    }
}
#[test]
fn distance_handle_without_output_is_an_error() {
    let mut e = Engine::default();
    e.set_function_directory(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stats_models"),
    )
    .unwrap();
    let err = e
        .execute("pdist([1 2;3 4],@no_output_distance)")
        .unwrap_err();
    assert!(err.to_string().contains("must return a value"), "{err}");
}
