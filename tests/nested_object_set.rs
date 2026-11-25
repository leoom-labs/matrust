use matrust::Engine;
use std::time::{SystemTime, UNIX_EPOCH};

fn run(e: &mut Engine, s: &str) {
    e.execute(s).unwrap_or_else(|err| panic!("{s}: {err}"));
}

fn check(e: &mut Engine, s: &str) {
    run(e, &format!("assert({s});"));
}

#[test]
fn non_econ_nested_object_property_set_and_readonly_class_errors() {
    let mut e = Engine::default();

    // Network: one level inside struct/cell works like direct form; deeper nested keeps error.
    run(&mut e, "s.n=feedforwardnet(3);s.n.name='net1';");
    check(&mut e, "strcmp(s.n.name,'net1')");
    run(&mut e, "cn={feedforwardnet(3)};cn{1}.name='net2';");
    check(&mut e, "strcmp(cn{1}.name,'net2')");
    let err = e.execute("s.n.trainParam.epochs=10;").unwrap_err();
    assert!(
        err.contains("extract the network to a variable"),
        "unexpected network deep nested error: {err}"
    );

    // Other built-in objects with settable properties in containers.
    run(&mut e, "s.p=inputParser;s.p.FunctionName='myfn';");
    check(&mut e, "strcmp(s.p.FunctionName,'myfn')");
    run(&mut e, "s.f=mamfis('Name','tip');s.f.Name='tip2';");
    check(&mut e, "strcmp(s.f.Name,'tip2')");
    run(
        &mut e,
        "s.g=griddedInterpolant([1 2 3],[10 20 30]);s.g.Method='nearest';",
    );
    check(&mut e, "strcmp(s.g.Method,'nearest')");
    run(&mut e, "s.m=containers.Map('a',1);s.m('b')=2;");
    check(&mut e, "s.m('b')==2");

    // Variants with no settable properties must error with their own class name, never inputParser.
    run(
        &mut e,
        "ro.d=fitdist([1;2;3;4;5],'Normal');ro.r=fitlm([1;2;3;4],[2;4;6;8]);ro.p=cvpartition(10,'KFold',2);",
    );
    for (stmt, must_not) in [
        ("ro.d.Mean=0;", "inputParser"),
        ("ro.r.MSE=0;", "inputParser"),
        ("ro.p.NumTestSets=3;", "inputParser"),
    ] {
        let err = e.execute(stmt).unwrap_err();
        assert!(!err.contains(must_not), "{stmt}: {err}");
        assert!(err.contains("read-only"), "{stmt}: {err}");
    }
    let err = e.execute("ro.p.NumTestSets=3;").unwrap_err();
    assert!(err.contains("cvpartition"), "{err}");
}

#[test]
fn econ_nested_object_property_set_across_containers() {
    let mut e = Engine::default();

    // Struct field (arima) + value semantics.
    run(&mut e, "s.M=arima(1,0,0);copy=s.M;s.M.Constant=0.3;");
    check(&mut e, "s.M.Constant==0.3 && isnan(copy.Constant)");

    // Cell element (garch).
    run(&mut e, "c={garch(1,1)};c{1}.Constant=0.1;");
    check(&mut e, "c{1}.Constant==0.1");

    // Struct array element (egarch).
    run(
        &mut e,
        "sa(1).M=egarch(1,1);sa(2).M=egarch(1,1);sa(2).M.Constant=0.2;",
    );
    check(&mut e, "isnan(sa(1).M.Constant) && sa(2).M.Constant==0.2");

    // Two levels deep (gjr) including cell property inside model.
    run(
        &mut e,
        "d.a.M=gjr(1,1);d.a.M.Constant=0.05;d.a.M.GARCH{1}=0.7;",
    );
    check(&mut e, "d.a.M.Constant==0.05 && d.a.M.GARCH{1}==0.7");
}

#[test]
fn multi_output_methods_on_objects_in_containers_and_classdef() {
    let mut e = Engine::default();

    // [v,l]=infer(s.Mdl,y) and [e,v,l]=infer(c{1},y)
    run(
        &mut e,
        "rng(1);y=randn(30,1);s.Mdl=garch('Constant',0.1,'GARCH',0.8,'ARCH',0.1);[v,l]=infer(s.Mdl,y);",
    );
    check(&mut e, "numel(v)==30 && isfinite(l)");
    run(
        &mut e,
        "c={arima('Constant',0.05,'AR',0.4,'Variance',garch('Constant',0.1,'GARCH',0.8,'ARCH',0.1))};[res,vc,lc]=infer(c{1},y);",
    );
    check(&mut e, "numel(res)==30 && numel(vc)==30 && isfinite(lc)");

    // Classdef method [a,b]=m(s.o,x) written to a temp folder.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("matrust_nested_cls_{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let cls_path = dir.join("PairObj.m");
    std::fs::write(
        &cls_path,
        "classdef PairObj\n    properties\n        Scale = 2\n    end\n    methods\n        function [a, b] = m(obj, x)\n            a = obj.Scale * x;\n            b = obj.Scale + x;\n        end\n    end\nend\n",
    )
    .unwrap();
    e.set_function_directory(&dir).unwrap();
    run(&mut e, "so.o=PairObj();[a,b]=m(so.o,3);");
    let _ = std::fs::remove_dir_all(&dir);
    check(&mut e, "a==6 && b==5");
}
