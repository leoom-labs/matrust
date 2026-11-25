//! gplot, histfit, boxplot, perfcurve. AUC values checked by hand and with
//! sklearn.metrics.roc_auc_score; boxplot quartiles follow prctile's midpoint rule.
use matrust::Engine;

fn scalar(e: &Engine, name: &str) -> f64 {
    e.array(name)
        .unwrap()
        .scalar(0, 0)
        .unwrap()
        .into_real()
        .unwrap()
}

fn run(e: &mut Engine, source: &str) {
    e.execute(source)
        .unwrap_or_else(|err| panic!("{source}: {err}"));
}

#[test]
fn gplot_returns_nan_separated_segments() {
    let mut e = Engine::default();
    run(
        &mut e,
        "[X, Y] = gplot([0 1; 1 0], [0 0; 1 1]); ok = isequaln(X, [1;0;NaN;0;1;NaN]) && isequaln(Y, [1;0;NaN;0;1;NaN]);",
    );
    assert_eq!(scalar(&e, "ok"), 1.);
    run(
        &mut e,
        "A = sparse([0 1 1; 1 0 0; 1 0 0]); [X, Y] = gplot(A, [0 0; 1 0; 0 1]); n = numel(X); s = X(4);",
    );
    assert_eq!(scalar(&e, "n"), 12.);
    assert_eq!(scalar(&e, "s"), 0.);
    run(&mut e, "gplot([0 1; 1 0], [0 0; 1 1], 'r-');");
}

#[test]
fn histfit_draws_bars_and_a_scaled_density() {
    let mut e = Engine::default();
    run(
        &mut e,
        "rng(0); d = randn(200,1) + 5; h = histfit(d, 10); n = numel(h); y = get(h(end), 'YData'); m = max(y);",
    );
    assert!(scalar(&e, "n") >= 2.);
    // Peak of n*w*pdf with w = range/10 is near 200*w/(sigma*sqrt(2*pi)).
    run(
        &mut e,
        "w = (max(d) - min(d)) / 10; expect = 200 * w / (std(d) * sqrt(2*pi)); r = m / expect;",
    );
    let r = scalar(&e, "r");
    assert!((r - 1.).abs() < 0.02, "{r}");
    assert!(e.execute("histfit([])").is_err());
}

#[test]
fn boxplot_handles_columns_groups_and_outliers() {
    let mut e = Engine::default();
    run(
        &mut e,
        "x = [1 2 3 4 5 6 7 8 9 30]'; h = boxplot(x); [r, c] = size(h); o = get(h(7), 'YData'); b = get(h(5), 'YData'); md = get(h(6), 'YData');",
    );
    assert_eq!(scalar(&e, "r"), 7.);
    assert_eq!(scalar(&e, "c"), 1.);
    assert_eq!(scalar(&e, "o"), 30.);
    // prctile midpoint rule on 10 points: q1 = 3, median = 5.5, q3 = 8.
    run(
        &mut e,
        "q1 = min(b); q3 = max(b); med = md(1); up = get(h(1), 'YData'); top = max(up);",
    );
    assert_eq!(scalar(&e, "q1"), 3.);
    assert_eq!(scalar(&e, "q3"), 8.);
    assert_eq!(scalar(&e, "med"), 5.5);
    assert_eq!(scalar(&e, "top"), 9.);
    run(
        &mut e,
        "h = boxplot([x 2*x], 'Labels', {'a','b'}); k = size(h, 2);",
    );
    assert_eq!(scalar(&e, "k"), 2.);
    run(
        &mut e,
        "g = {'u','v','u','v','u','v','u','v','u','v'}'; h = boxplot(x, g); k = size(h, 2);",
    );
    assert_eq!(scalar(&e, "k"), 2.);
    assert!(e.execute("boxplot(x, 'Notch', 'on')").is_err());
}

#[test]
fn perfcurve_roc_and_auc() {
    let mut e = Engine::default();
    run(
        &mut e,
        "labels = [1 1 0 1 0 0 1 0]'; scores = [0.9 0.8 0.7 0.6 0.55 0.5 0.4 0.3]'; [X, Y, T, AUC] = perfcurve(labels, scores, 1); n = numel(X); x1 = X(1); y1 = Y(1); xe = X(end); ye = Y(end); t2 = T(2);",
    );
    assert_eq!(scalar(&e, "AUC"), 0.75);
    assert_eq!(scalar(&e, "n"), 9.);
    assert_eq!((scalar(&e, "x1"), scalar(&e, "y1")), (0., 0.));
    assert_eq!((scalar(&e, "xe"), scalar(&e, "ye")), (1., 1.));
    assert_eq!(scalar(&e, "t2"), 0.9);
    run(
        &mut e,
        "[~, ~, ~, A2] = perfcurve({'a','b','a','b'}, [0.9 0.2 0.6 0.7], 'a');",
    );
    assert_eq!(scalar(&e, "A2"), 0.75);
    // Tied scores form one step: sklearn roc_auc_score([1,0,1,0],[0.5,0.5,0.9,0.1]) = 0.875.
    run(
        &mut e,
        "[~, ~, ~, A3] = perfcurve([1 0 1 0], [0.5 0.5 0.9 0.1], 1);",
    );
    assert_eq!(scalar(&e, "A3"), 0.875);
    run(
        &mut e,
        "[R, P] = perfcurve([1 1 0 1 0]', [0.9 0.8 0.7 0.6 0.5]', 1, 'XCrit', 'reca', 'YCrit', 'prec'); p2 = P(2);",
    );
    assert_eq!(scalar(&e, "p2"), 1.);
    assert!(e.execute("perfcurve([1 0], [0.5], 1)").is_err());
}

#[test]
fn review_plot_sizes_fail_before_allocating_outputs() {
    let mut e = Engine::default();
    for code in ["histfit([1;2],1e300)", "histfit([1;2],Inf)"] {
        let err = e.execute(code).unwrap_err();
        assert!(
            err.contains("histfit") && (err.contains("limit") || err.contains("integer")),
            "{err}"
        );
    }
    let err = e
        .execute("[x,y]=gplot(ones(4097),zeros(4097,2));")
        .unwrap_err();
    assert!(
        err.contains("gplot: output size") && err.contains("50331648"),
        "{err}"
    );
    assert!(e.figure.is_none());
}

#[test]
fn review_boxplot_many_groups_is_bounded_and_group_order_is_stable() {
    let mut e = Engine::default();
    let err = e.execute("boxplot((1:200000)',(1:200000)')").unwrap_err();
    assert!(err.contains("boxplot: group count"), "{err}");
    assert!(e.figure.is_none());
    run(
        &mut e,
        "h=boxplot([2;10;4;20],[2;1;2;1]); a=get(h(6,1),'YData'); b=get(h(6,2),'YData'); assert(isequal(a,[3 3])); assert(isequal(b,[15 15]));",
    );
}

#[test]
fn review_categorical_plot_dispatch() {
    let mut e = Engine::default();
    run(
        &mut e,
        "h=boxplot([1;2],categorical({'a';'b'})); assert(isequal(size(h),[7 2])); [x,y,t,a]=perfcurve(categorical({'a';'b'}),[.9;.1],'a'); assert(a==1);",
    );
}

#[test]
fn boxplot_cancelled_grouping_keeps_the_figure_unchanged() {
    use std::sync::atomic::Ordering;
    let mut e = Engine::default();
    run(&mut e, "x=(1:200000)'; g=mod(x,8); plot([0 1],[2 3]);");
    let before = e.figure.clone().unwrap();
    let flag = e.cancellation();
    flag.store(true, Ordering::Relaxed);
    assert!(e.execute("boxplot(x,g)").is_err());
    flag.store(false, Ordering::Relaxed);
    assert!(before.same_snapshot(e.figure.as_ref().unwrap()));
    run(&mut e, "h=boxplot(x,g); assert(isequal(size(h),[7 8]));");
}
