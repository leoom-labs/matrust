//! Parallel Computing Toolbox names run serially in MatRust (one worker).
use matrust::{Engine, syntax};

fn scalar(e: &Engine, name: &str) -> f64 {
    e.array(name)
        .unwrap()
        .scalar(0, 0)
        .unwrap()
        .into_real()
        .unwrap()
}

fn text(e: &mut Engine, source: &str) -> String {
    e.execute(source).unwrap().output.concat()
}

#[test]
fn parfor_runs_like_for_in_both_forms() {
    let mut e = Engine::default();
    e.execute("s = 0; parfor k = 1:4, s = s + k; end").unwrap();
    assert_eq!(scalar(&e, "s"), 10.);
    e.execute("parfor (k = 1:3, 4)\n x(k) = k^2;\nend\nt = sum(x);")
        .unwrap();
    assert_eq!(scalar(&e, "t"), 14.);
    e.execute("v = []; parfor k = [5 6], v(end+1) = k; end, w = v(2);")
        .unwrap();
    assert_eq!(scalar(&e, "w"), 6.);
}

#[test]
fn spmd_runs_its_body_once() {
    let mut e = Engine::default();
    e.execute("c = 0; spmd, c = c + 1; r = spmdIndex * 10 + spmdSize; end")
        .unwrap();
    assert_eq!(scalar(&e, "c"), 1.);
    assert_eq!(scalar(&e, "r"), 11.);
    e.execute("spmd (4)\n q = labindex + numlabs;\nend")
        .unwrap();
    assert_eq!(scalar(&e, "q"), 2.);
}

#[test]
fn open_parfor_and_spmd_blocks_wait_for_more_input() {
    assert!(syntax::is_incomplete("parfor k = 1:3"));
    assert!(syntax::is_incomplete("parfor (k = 1:3, 2)\n x = k;"));
    assert!(syntax::is_incomplete("spmd\n x = 1;"));
    assert!(!syntax::is_incomplete("spmd, x = 1; end"));
}

#[test]
fn pool_has_one_worker_and_gcp_returns_it() {
    let mut e = Engine::default();
    e.execute("p0 = gcp('nocreate'); e0 = isempty(p0);")
        .unwrap();
    assert_eq!(scalar(&e, "e0"), 1.);
    let shown = text(&mut e, "p = parpool(4);");
    assert!(
        shown.contains("1 worker (runs serially in MatRust)"),
        "{shown}"
    );
    e.execute("n = p.NumWorkers; same = gcp == p; c = strcmp(class(p), 'parallel.ProcessPool');")
        .unwrap();
    assert_eq!(scalar(&e, "n"), 1.);
    assert_eq!(scalar(&e, "same"), 1.);
    assert_eq!(scalar(&e, "c"), 1.);
    let err = e.execute("parpool").unwrap_err();
    assert!(err.contains("delete(gcp('nocreate'))"), "{err}");
    e.execute("delete(p); gone = isempty(gcp('nocreate'));")
        .unwrap();
    assert_eq!(scalar(&e, "gone"), 1.);
    e.execute("t = parpool('Threads'); tc = strcmp(class(t), 'parallel.ThreadPool');")
        .unwrap();
    assert_eq!(scalar(&e, "tc"), 1.);
}

#[test]
fn parfeval_returns_a_finished_future() {
    let mut e = Engine::default();
    e.execute(
        "F = parfeval(@(x, y) x * y, 1, 6, 7); y = fetchOutputs(F); \
         fin = strcmp(F.State, 'finished'); fc = strcmp(class(F), 'parallel.FevalFuture'); rd = F.Read;",
    )
    .unwrap();
    assert_eq!(scalar(&e, "y"), 42.);
    assert_eq!(scalar(&e, "fin"), 1.);
    assert_eq!(scalar(&e, "fc"), 1.);
    assert_eq!(scalar(&e, "rd"), 1.);
    e.execute("G = parfeval(gcp, @sqrt, 1, 81); g = fetchOutputs(G);")
        .unwrap();
    assert_eq!(scalar(&e, "g"), 9.);
}

#[test]
fn future_arrays_fetch_next_and_concatenate_outputs() {
    let mut e = Engine::default();
    e.execute(
        "a = parfeval(@(x) 10 * x, 1, 1); b = parfeval(@(x) 10 * x, 1, 2); c = parfeval(@(x) 10 * x, 1, 3);\n\
         F = [a b c]; [i1, v1] = fetchNext(F); [i2, v2] = fetchNext(F);\n\
         all = fetchOutputs(F); s = sum(all); n = size(all, 1);\n\
         cells = fetchOutputs(F, 'UniformOutput', false); isc = iscell(cells);",
    )
    .unwrap();
    assert_eq!(scalar(&e, "i1"), 1.);
    assert_eq!(scalar(&e, "v1"), 10.);
    assert_eq!(scalar(&e, "i2"), 2.);
    assert_eq!(scalar(&e, "v2"), 20.);
    assert_eq!(scalar(&e, "s"), 60.);
    assert_eq!(scalar(&e, "n"), 3.);
    assert_eq!(scalar(&e, "isc"), 1.);
    let err = e.execute("fetchNext(F); fetchNext(F);").unwrap_err();
    assert!(err.contains("all futures have been read"), "{err}");
}

#[test]
fn errors_in_futures_are_kept_until_fetch() {
    let mut e = Engine::default();
    e.execute("F = parfeval(@error, 0, 'boom'); k = strcmp(class(F.Error), 'MException');")
        .unwrap();
    assert_eq!(scalar(&e, "k"), 1.);
    let err = e.execute("fetchOutputs(F)").unwrap_err();
    assert!(
        err.contains("One or more futures resulted in an error"),
        "{err}"
    );
    assert!(err.contains("boom"), "{err}");
    e.execute("after = 3;").unwrap();
    assert_eq!(scalar(&e, "after"), 3.);
}

#[test]
fn batch_runs_functions_at_once() {
    let mut e = Engine::default();
    e.execute(
        "j = batch(@plus, 1, {2, 3}); wait(j); r = fetchOutputs(j); v = r{1}; ic = iscell(r);",
    )
    .unwrap();
    assert_eq!(scalar(&e, "v"), 5.);
    assert_eq!(scalar(&e, "ic"), 1.);
    let err = e.execute("batch(@plus, 1, 2)").unwrap_err();
    assert!(err.contains("cell array"), "{err}");
}

#[test]
fn gpu_arrays_are_ordinary_arrays() {
    let mut e = Engine::default();
    e.execute(
        "g = gpuArray([1 2 3]); h = gather(g * 2); s = sum(h); same = isequal(class(g), 'double');",
    )
    .unwrap();
    assert_eq!(scalar(&e, "s"), 12.);
    assert_eq!(scalar(&e, "same"), 1.);
    e.execute("[a, b] = gather(1, 2); t = a + b;").unwrap();
    assert_eq!(scalar(&e, "t"), 3.);
}

#[test]
fn parallel_names_exist() {
    let mut e = Engine::default();
    e.execute(
        "x = [exist('parfor') exist('spmd') exist('parpool') exist('gcp') exist('parfeval') \
         exist('batch') exist('gpuArray') exist('gather') exist('fetchOutputs')]; m = min(x);",
    )
    .unwrap();
    assert!(scalar(&e, "m") > 0.);
}

#[test]
fn review_pool_is_a_gc_root_until_deleted() {
    let mut e = Engine::default();
    e.execute("p=parpool; clear p; F=cell(1,1030); for k=1:1030,F{k}=parfeval(@plus,1,1,1);end; gone=isempty(gcp('nocreate'));").unwrap();
    assert_eq!(scalar(&e, "gone"), 0.);
    e.execute("p=gcp('nocreate'); assert(p.NumWorkers==1); delete(p); gone=isempty(gcp('nocreate')); clear p F; q=parpool; assert(q==gcp('nocreate')); delete(q);").unwrap();
    assert_eq!(scalar(&e, "gone"), 1.);
}

#[test]
fn review_worker_argument_expressions_are_evaluated_before_body() {
    let mut e = Engine::default();
    for code in [
        "parfor (i=1:2,error('must evaluate')),disp(i);end",
        "spmd (error('must evaluate')),disp(1);end",
        "spmd (1,error('must evaluate')),disp(1);end",
        "parfor (i=1:0,error('must evaluate')),disp(i);end",
    ] {
        let err = e.execute(code).unwrap_err();
        assert!(err.contains("must evaluate"), "{code}: {err}");
    }
    e.execute("ans=123; calls=0; function n=countworker(), global counter; counter=counter+1; n=4; end; global counter; counter=0; parfor (i=1:2,countworker()),calls=calls+1;end; spmd (countworker(),countworker()),calls=calls+1;end; assert(counter==3); assert(calls==3); assert(ans==123);").unwrap();
}
