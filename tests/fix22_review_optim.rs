//! Regression cases from the Fix 22 whole-step review.
use matrust::{Array, Engine, plot::PlotKind};

fn scalar(e: &Engine, name: &str) -> f64 {
    e.variables[name].data()[0]
}

fn chars(e: &Engine, name: &str) -> String {
    let Some(Array::Character(value)) = e.array(name) else {
        panic!("Expected character value for {name}");
    };
    String::from_utf16_lossy(value.units())
}

#[test]
fn item_02_fzero_accepts_a_subnormal_bracket_at_tolx() {
    let mut e = Engine::default();
    e.execute(
        "u=eps(0); [x,f,flag,o]=fzero(@(x)x/u-5.5,[5*u 6*u],optimset('TolX',u,'MaxIter',4));          fc=o.funcCount; it=o.iterations;",
    )
    .unwrap();
    assert_eq!(scalar(&e, "flag"), 1.);
    assert_eq!(scalar(&e, "fc"), 2.);
    assert_eq!(scalar(&e, "it"), 0.);
    assert_eq!(scalar(&e, "f").abs(), 0.5);
    let u = scalar(&e, "u");
    assert!([5. * u, 6. * u].contains(&scalar(&e, "x")));
}

#[test]
fn item_03_fminsearch_caps_initialization_reflection_and_shrink_calls() {
    let mut e = Engine::default();
    e.execute("[~,~,flag,o]=fminsearch(@(x)0,[1 1],optimset('MaxFunEvals',5)); fc=o.funcCount;")
        .unwrap();
    assert_eq!(scalar(&e, "flag"), 0.);
    assert_eq!(scalar(&e, "fc"), 5.);
    for cap in 0..=7 {
        e.execute(&format!(
            "global calls; calls=0;              [~,~,flag,o]=fminsearch(@(x)counted_constant(x),[1 1],optimset('MaxFunEvals',{cap}));              fc=o.funcCount; msg=o.message;              function y=counted_constant(x); global calls; calls=calls+1; y=0; end"
        ))
        .unwrap();
        assert_eq!(scalar(&e, "calls"), f64::from(cap));
        assert_eq!(scalar(&e, "fc"), f64::from(cap));
        assert_eq!(scalar(&e, "flag"), 0.);
        assert!(chars(&e, "msg").contains("increase MaxFunEvals"));
    }
}

#[test]
fn item_04_fzero_caps_endpoint_and_nan_retry_calls() {
    let mut e = Engine::default();
    for (objective, interval, cap, body) in [
        ("@cos", "[1 2]", 1, "cos(x)"),
        ("@(x)x^2/x", "[-1 1]", 3, "x^2/x"),
    ] {
        let call = format!(
            "[~,~,flag,o]=fzero({objective},{interval},optimset('MaxFunEvals',{cap}));              fc=o.funcCount; msg=o.message;"
        );
        e.execute(&call).unwrap();
        assert_eq!(scalar(&e, "fc"), f64::from(cap));
        assert_eq!(scalar(&e, "flag"), 0.);
        assert!(chars(&e, "msg").contains("increase MaxFunEvals"));
        e.execute(&format!(
            "global calls; calls=0; {}              function y=counted(x); global calls; calls=calls+1; y={body}; end",
            call.replace(objective, "@counted")
        ))
        .unwrap();
        assert_eq!(scalar(&e, "calls"), f64::from(cap));
    }
}

#[test]
fn item_05_fzero_uses_one_iteration_cap_for_both_phases() {
    let mut e = Engine::default();
    for (cap, brent_iterations) in [(2, 0.), (3, 1.)] {
        e.execute(&format!(
            "[~,~,flag,o]=fzero(@(x)x-.021,0,optimset('MaxIter',{cap}));              search=o.intervaliterations; it=o.iterations; msg=o.message;"
        ))
        .unwrap();
        assert_eq!(scalar(&e, "flag"), 0.);
        assert_eq!(scalar(&e, "search"), 2.);
        assert_eq!(scalar(&e, "it"), brent_iterations);
        assert!(chars(&e, "msg").contains("increase MaxIter"));
    }
}

#[test]
fn item_06_fminbnd_reports_a_limit_at_the_convergence_boundary() {
    let mut e = Engine::default();
    for (option, cap) in [("MaxIter", 8), ("MaxFunEvals", 9)] {
        e.execute(&format!(
            "[x,f,flag,o]=fminbnd(@sin,0,2*pi,optimset('{option}',{cap}));              it=o.iterations; fc=o.funcCount; msg=o.message;"
        ))
        .unwrap();
        assert_eq!(scalar(&e, "flag"), 0.);
        assert_eq!(scalar(&e, "it"), 8.);
        assert_eq!(scalar(&e, "fc"), 9.);
        assert!((scalar(&e, "x") - 1.5 * std::f64::consts::PI).abs() < 1e-4);
        assert!(chars(&e, "msg").contains(&format!("increase {option}")));
    }
    e.execute("[~,~,flag,o]=fminbnd(@sin,0,2*pi,optimset('MaxFunEvals',0)); fc=o.funcCount;")
        .unwrap();
    assert_eq!(scalar(&e, "flag"), 0.);
    assert_eq!(scalar(&e, "fc"), 0.);
}

#[test]
fn item_07_fzero_honors_init_stop_before_searching_for_a_bracket() {
    let mut e = Engine::default();
    e.execute(
        "[~,~,flag,o]=fzero(@(x)x^2+1,0,optimset('MaxFunEvals',2,'OutputFcn',@(x,v,s)true));          fc=o.funcCount;",
    )
    .unwrap();
    assert_eq!(scalar(&e, "flag"), -1.);
    assert_eq!(scalar(&e, "fc"), 1.);
    e.execute(
        "global states; states={};          [x,f,flag,o]=fzero(@(x)x^2+1,0,optimset('MaxFunEvals',2,'OutputFcn',@stop_now));          s1=states{1}; s2=states{2}; n=numel(states); ii=o.intervaliterations;          function y=stop_now(x,v,s); global states; states{end+1}=s; y=true; end",
    )
    .unwrap();
    assert_eq!(scalar(&e, "flag"), -1.);
    assert_eq!(scalar(&e, "x"), 0.);
    assert_eq!(scalar(&e, "f"), 1.);
    assert_eq!(scalar(&e, "ii"), 0.);
    assert_eq!(scalar(&e, "n"), 2.);
    assert_eq!(chars(&e, "s1"), "init");
    assert_eq!(chars(&e, "s2"), "done");
}

#[test]
fn item_08_fminsearch_reports_the_start_before_building_the_simplex() {
    let mut e = Engine::default();
    let result = e.execute(
        "[~,~,flag,o]=fminsearch(@(x)(x-3)^2,0,optimset('Display','iter','MaxIter',1));          it=o.iterations; fc=o.funcCount;",
    ).unwrap();
    let rows: Vec<Vec<&str>> = result
        .output
        .iter()
        .filter_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            fields.first()?.parse::<usize>().ok()?;
            Some(fields)
        })
        .collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0], ["0", "1", "9"]);
    assert_eq!(rows[1], ["1", "2", "8.9985", "initial", "simplex"]);
    assert_eq!(scalar(&e, "flag"), 0.);
    assert_eq!(scalar(&e, "it"), 1.);
    assert_eq!(scalar(&e, "fc"), 2.);
    e.execute(
        "[x,f,flag,o]=fminsearch(@(x)(x-3)^2,0,optimset('OutputFcn',@(x,v,s)true));          it=o.iterations; fc=o.funcCount;",
    ).unwrap();
    assert_eq!(scalar(&e, "x"), 0.);
    assert_eq!(scalar(&e, "f"), 9.);
    assert_eq!(scalar(&e, "flag"), -1.);
    assert_eq!(scalar(&e, "it"), 0.);
    assert_eq!(scalar(&e, "fc"), 1.);
}

#[test]
fn item_09_fzero_outputs_the_final_bracket_and_paired_residuals() {
    let mut e = Engine::default();
    e.execute(
        "[x,f,flag,o]=fzero(@(x)x^2-2,[1 2]); bx=o.bracketx; by=o.brackety; residuals=bx.^2-2;",
    )
    .unwrap();
    assert_eq!(scalar(&e, "flag"), 1.);
    let bx = e.variables["bx"].data();
    let by = e.variables["by"].data();
    assert_eq!(bx.len(), 2);
    assert!(bx[0] <= scalar(&e, "x") && scalar(&e, "x") <= bx[1]);
    assert!(
        bx.iter()
            .all(|x| (x - std::f64::consts::SQRT_2).abs() < 1e-14)
    );
    assert!(by[0] <= 0. && by[1] >= 0.);
    assert_eq!(by, e.variables["residuals"].data());
}

#[test]
fn item_10_fminbnd_displays_the_parabolic_trial_even_if_rejected() {
    let mut e = Engine::default();
    let result = e
        .execute("fminbnd(@sin,0,2*pi,optimset('Display','iter'));")
        .unwrap();
    assert!(
        result.output.iter().any(|line| {
            line.split_whitespace()
                .eq(["4", "5.08984", "-0.929607", "parabolic"])
        }),
        "{:?}",
        result.output
    );
}

#[test]
fn item_11_fzero_done_keeps_the_last_procedure() {
    let mut e = Engine::default();
    e.execute(
        "global procedures states indexes; procedures={}; states={}; indexes=[];          opts=optimset('OutputFcn',@record_step);          [~,~,flag,o]=fzero(@(x)x^2-2,[1 2],opts); it=o.iterations;          last=procedures{end-1}; done=procedures{end}; done_state=states{end};          last_state=states{end-1}; last_index=indexes(end-1); done_index=indexes(end);          function y=record_step(x,v,s); global procedures states indexes;          procedures{end+1}=v.procedure; states{end+1}=s; indexes(end+1)=v.iteration; y=false; end",
    )
    .unwrap();
    assert_eq!(scalar(&e, "flag"), 1.);
    assert_eq!(chars(&e, "last_state"), "iter");
    assert_eq!(chars(&e, "done_state"), "done");
    assert_eq!(chars(&e, "last"), "interpolation");
    assert_eq!(chars(&e, "done"), chars(&e, "last"));
    assert_eq!(scalar(&e, "last_index"), scalar(&e, "it"));
    assert_eq!(scalar(&e, "done_index"), scalar(&e, "it"));
}

#[test]
fn item_12_fzero_prints_the_search_row_for_a_partial_probe_pair() {
    let mut e = Engine::default();
    let result = e
        .execute(
            "[~,~,flag,o]=fzero(@(x)x^2+1,0,optimset('MaxFunEvals',2,'Display','iter'));              ii=o.intervaliterations; fc=o.funcCount;",
        )
        .unwrap();
    assert_eq!(scalar(&e, "flag"), 0.);
    assert_eq!(scalar(&e, "ii"), 1.);
    assert_eq!(scalar(&e, "fc"), 2.);
    assert_eq!(
        result
            .output
            .iter()
            .filter(|line| line.ends_with("initial interval"))
            .count(),
        1
    );
    let rows: Vec<_> = result
        .output
        .iter()
        .filter(|line| line.ends_with(" search"))
        .collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].split_whitespace().collect::<Vec<_>>(),
        ["2", "-0.02", "1.0004", "0", "1", "search"]
    );
}

#[test]
fn item_13_optimoptions_accepts_basic_solver_callbacks_and_value_checks() {
    let mut e = Engine::default();
    for (solver, objective, start) in [
        ("fzero", "@cos", "[1 2]"),
        ("fminbnd", "@sin", "0,2*pi"),
        ("fminsearch", "@(x)(x-3)^2", "0"),
    ] {
        e.execute(&format!(
            "opts=optimoptions('{solver}','OutputFcn',@(x,v,s)true,'PlotFcns',@optimplotfval,              'FunValCheck','on','TolX',1e-8,'TolFun',1e-6,'MaxIter',10,'MaxFunEvals',10,'Display','off');              [~,~,flag]={solver}({objective},{start},opts);"
        ))
        .unwrap();
        assert_eq!(scalar(&e, "flag"), -1.);
        assert_eq!(e.figure.as_ref().unwrap().series.len(), 1);
    }
    // OutputFcn is accepted for fminunc since corpus r2 W13 (MATLAB lists it); it warns when not called.
    for option in ["'PlotFcns',@optimplotfval", "'FunValCheck','on'"] {
        assert!(
            e.execute(&format!("optimoptions('fminunc',{option});"))
                .unwrap_err()
                .contains("unknown option")
        );
    }
}

#[test]
fn item_14_optimset_preserves_scalar_string_values_in_both_call_forms() {
    let mut e = Engine::default();
    for options in [r#"Display="iter""#, r#"'Display',"iter""#, "'Display',mode"] {
        let result = e
            .execute(&format!(
                r#"mode="iter"; [x,~,flag]=fminbnd(@sin,0,2*pi,optimset({options}));"#
            ))
            .unwrap();
        assert_eq!(scalar(&e, "flag"), 1.);
        assert!((scalar(&e, "x") - 1.5 * std::f64::consts::PI).abs() < 1e-4);
        assert!(result.output.iter().any(|line| line.contains("Func-count")));
    }
}

#[test]
fn item_15_optimplotfval_retains_long_history_in_one_line() {
    let mut e = Engine::default();
    e.execute(
        "[~,f,flag,o]=fminbnd(@(x)x,0,1e12,optimset('PlotFcns',@optimplotfval)); it=o.iterations;",
    )
    .unwrap();
    assert_eq!(scalar(&e, "flag"), 1.);
    assert_eq!(scalar(&e, "it"), 77.);
    let figure = e.figure.as_ref().unwrap();
    assert_eq!(figure.all_axes().count(), 1);
    let [line] = figure.series.as_slice() else {
        panic!("Expected one history line");
    };
    assert_eq!(line.kind, PlotKind::Line);
    assert_eq!(line.points.len(), 78);
    assert_eq!(line.points.get(0).unwrap()[0], 0.);
    assert_eq!(line.points.get(77).unwrap(), [77., scalar(&e, "f")]);
}

#[test]
fn item_16_each_plot_function_uses_its_own_axes() {
    let mut e = Engine::default();
    e.execute(
        "[x,f,flag,o]=fzero(@(x)sin(cosh(x)),1,optimset('PlotFcns',{@optimplotx,@optimplotfval}));          it=o.iterations; ii=o.intervaliterations;",
    )
    .unwrap();
    assert_eq!(scalar(&e, "flag"), 1.);
    assert!((scalar(&e, "x") - std::f64::consts::PI.acosh()).abs() < 1e-12);
    let figure = e.figure.as_ref().unwrap();
    assert_eq!(figure.all_axes().count(), 2);
    let series: Vec<_> = figure
        .all_axes()
        .map(|(_, axes)| {
            assert_eq!(axes.series.len(), 1);
            &axes.series[0]
        })
        .collect();
    let bar = series.iter().find(|s| s.kind == PlotKind::Bar).unwrap();
    let history = series.iter().find(|s| s.kind == PlotKind::Line).unwrap();
    assert_eq!(bar.points.get(0).unwrap()[1], scalar(&e, "x"));
    let steps = scalar(&e, "it") + scalar(&e, "ii");
    assert_eq!(history.points.len(), steps as usize + 1);
    assert_eq!(
        history.points.get(steps as usize).unwrap(),
        [steps, scalar(&e, "f")]
    );
}

#[test]
fn item_17_minimizers_accept_infinite_penalties_with_funvalcheck_on() {
    let mut e = Engine::default();
    e.execute("[x,f,flag]=fminbnd(@(x)1/(x<=0.5),0,1,optimset('FunValCheck','on'));")
        .unwrap();
    assert_eq!(scalar(&e, "flag"), 1.);
    assert!(scalar(&e, "x") <= 0.5);
    assert_eq!(scalar(&e, "f"), 1.);
    e.execute("[x,f,flag]=fminsearch(@(x)(x+1)^2+1/(x<=0.5),0.5,optimset('FunValCheck','on'));")
        .unwrap();
    assert_eq!(scalar(&e, "flag"), 1.);
    assert!((scalar(&e, "x") + 1.).abs() < 1e-4);
    assert!((scalar(&e, "f") - 1.).abs() < 1e-8);
    for objective in ["@(x)NaN", "@(x)1i"] {
        assert!(
            e.execute(&format!(
                "fminbnd({objective},0,1,optimset('FunValCheck','on'));"
            ))
            .unwrap_err()
            .contains("NaN or complex")
        );
    }
}

#[test]
fn item_18_termination_messages_report_the_value_and_scientific_tolerance() {
    let mut e = Engine::default();
    let result = e
        .execute("[~,f,flag,o]=fminsearch(@(x)(x-3)^2,0,optimset('MaxFunEvals',5)); msg=o.message;")
        .unwrap();
    assert_eq!(scalar(&e, "flag"), 0.);
    let line = format!("         Current function value: {:.6}", scalar(&e, "f"));
    assert_eq!(result.output.last().unwrap(), &line);
    assert_eq!(chars(&e, "msg"), result.output.join("\n"));
    let result = e
        .execute("[~,~,flag,o]=fminbnd(@sin,0,2*pi,optimset('Display','final')); msg=o.message;")
        .unwrap();
    assert_eq!(scalar(&e, "flag"), 1.);
    assert!(
        result
            .output
            .last()
            .unwrap()
            .ends_with("OPTIONS.TolX of 1.000000e-04")
    );
    assert_eq!(chars(&e, "msg"), result.output.join("\n"));
}

#[test]
fn review_f1_fzero_same_sign_bracket_errors_even_at_a_limit() {
    for opts in ["optimset('MaxFunEvals',2)", "optimset('MaxIter',0)"] {
        let mut e = Engine::default();
        let err = e
            .execute(&format!("x = fzero(@(x)x^2+1, [1 2], {opts});"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("must differ in sign"), "{opts}: {err}");
    }
    let mut e = Engine::default();
    e.execute("[x, f, flag] = fzero(@(x)x^2+1, [1 2], optimset('MaxFunEvals',1));")
        .unwrap();
    assert_eq!(scalar(&e, "flag"), 0.0);
}
