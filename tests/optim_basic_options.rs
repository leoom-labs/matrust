//! Options, Display, OutputFcn, PlotFcns, extra parameters, and 4th output
//! `output` struct for fzero, fminbnd, and fminsearch.
use matrust::{Array, Engine};

fn scalar(e: &Engine, name: &str) -> f64 {
    e.variables[name].data()[0]
}

fn vec(e: &Engine, name: &str) -> Vec<f64> {
    e.variables[name].data().to_vec()
}

fn text_of(e: &Engine, name: &str) -> String {
    match e.array(name).unwrap_or_else(|| panic!("missing {name}")) {
        Array::Character(c) => String::from_utf16_lossy(c.units()),
        other => panic!("expected char for {name}, got {other:?}"),
    }
}

#[test]
fn fminsearch_output_struct_and_limits() {
    let mut e = Engine::default();
    let res = e
        .execute("[x, fv, ef, out] = fminsearch(@(x) (x - 3)^2, 0);")
        .unwrap();
    assert!(!res.output.iter().any(|l| l.contains("Note:")));
    assert!((scalar(&e, "x") - 3.0).abs() < 1e-3);
    assert!(scalar(&e, "fv") < 1e-6);
    assert_eq!(scalar(&e, "ef"), 1.0);
    e.execute("it = out.iterations; fc = out.funcCount; alg = out.algorithm; msg = out.message;")
        .unwrap();
    assert!(scalar(&e, "it") > 0.0);
    assert!(scalar(&e, "fc") > 0.0);
    assert_eq!(text_of(&e, "alg"), "Nelder-Mead simplex direct search");
    assert!(!text_of(&e, "msg").is_empty());

    // MaxFunEvals 5 -> exitflag 0
    let res = e
        .execute("[x2, fv2, ef2, out2] = fminsearch(@(x) (x(1)-1)^2 + (x(2)+2)^2, [10 10], optimset('MaxFunEvals', 5, 'Display', 'off'));")
        .unwrap();
    assert!(!res.output.iter().any(|l| l.contains("Note:")));
    assert!(res.output.is_empty());
    assert_eq!(scalar(&e, "ef2"), 0.0);
    e.execute("fc2 = out2.funcCount;").unwrap();
    assert_eq!(scalar(&e, "fc2"), 5.0);

    // MaxIter 3 -> exitflag 0
    e.execute("[~, ~, ef3, out3] = fminsearch(@(x) (x(1)-1)^2 + (x(2)+2)^2, [10 10], optimset('MaxIter', 3, 'Display', 'off'));")
        .unwrap();
    assert_eq!(scalar(&e, "ef3"), 0.0);
    e.execute("it3 = out3.iterations;").unwrap();
    assert_eq!(scalar(&e, "it3"), 3.0);
}

#[test]
fn fzero_extra_params_tolx_and_output_struct() {
    let mut e = Engine::default();
    let res = e
        .execute("[x, fv, ef, out] = fzero(@(x, a) x - a, 0, [], 3);")
        .unwrap();
    assert!(!res.output.iter().any(|l| l.contains("Note:")));
    assert!((scalar(&e, "x") - 3.0).abs() < 1e-12);
    assert!(scalar(&e, "fv").abs() < 1e-12);
    assert_eq!(scalar(&e, "ef"), 1.0);
    e.execute("alg = out.algorithm; ii = out.intervaliterations; bx = out.bracketx; by = out.brackety; msg = out.message;")
        .unwrap();
    assert_eq!(text_of(&e, "alg"), "bisection, interpolation");
    assert!(scalar(&e, "ii") > 0.0);
    assert_eq!(vec(&e, "bx").len(), 2);
    assert_eq!(vec(&e, "by").len(), 2);
    assert!(!text_of(&e, "msg").is_empty());

    // TolX 1e-3 uses fewer function evaluations than TolX 1e-12
    e.execute("[~, ~, ef1, o1] = fzero(@(x) x^3 - 2*x - 5, [2 3], optimset('TolX', 1e-3));")
        .unwrap();
    e.execute("[~, ~, ef2, o2] = fzero(@(x) x^3 - 2*x - 5, [2 3], optimset('TolX', 1e-12));")
        .unwrap();
    assert_eq!(scalar(&e, "ef1"), 1.0);
    assert_eq!(scalar(&e, "ef2"), 1.0);
    e.execute("fc1 = o1.funcCount; fc2 = o2.funcCount;")
        .unwrap();
    assert!(scalar(&e, "fc1") < scalar(&e, "fc2"));

    // MaxIter limit on fzero -> exitflag 0
    e.execute(
        "[~, ~, ef0] = fzero(@(x) x^3 - 2*x - 5, [2 3], optimset('MaxIter', 1, 'Display', 'off'));",
    )
    .unwrap();
    assert_eq!(scalar(&e, "ef0"), 0.0);
}

#[test]
fn fminbnd_extra_params_limits_and_output_struct() {
    let mut e = Engine::default();
    let res = e
        .execute("[x, fv, ef, out] = fminbnd(@(x, a, b) (x - a)^2 + b, 0, 5, optimset('TolX', 1e-8), 2.5, 4);")
        .unwrap();
    assert!(!res.output.iter().any(|l| l.contains("Note:")));
    assert!((scalar(&e, "x") - 2.5).abs() < 1e-6);
    assert!((scalar(&e, "fv") - 4.0).abs() < 1e-10);
    assert_eq!(scalar(&e, "ef"), 1.0);
    e.execute("it = out.iterations; fc = out.funcCount; alg = out.algorithm; msg = out.message;")
        .unwrap();
    assert!(scalar(&e, "it") > 0.0);
    assert!(scalar(&e, "fc") > 0.0);
    assert_eq!(
        text_of(&e, "alg"),
        "golden section search, parabolic interpolation"
    );
    assert!(!text_of(&e, "msg").is_empty());

    // MaxIter 2 -> exitflag 0
    e.execute("[~, ~, ef0] = fminbnd(@cos, 2, 4, optimset('MaxIter', 2, 'Display', 'off'));")
        .unwrap();
    assert_eq!(scalar(&e, "ef0"), 0.0);
}

#[test]
fn display_iter_tables_match_iterations() {
    let mut e = Engine::default();

    // fminsearch Display 'iter'
    let res = e
        .execute("[x, fv, ef, out] = fminsearch(@(x) (x - 3)^2, 0, optimset('Display', 'iter'));")
        .unwrap();
    assert!(!res.output.iter().any(|l| l.contains("Note:")));
    assert!(res.output.iter().any(|l| l.contains("Iteration")
        && l.contains("Func-count")
        && l.contains("min f(x)")
        && l.contains("Procedure")));
    e.execute("it = out.iterations;").unwrap();
    let it = scalar(&e, "it") as usize;
    let rows: Vec<_> = res
        .output
        .iter()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let index = fields.next()?.parse::<usize>().ok()?;
            let evaluations = fields.next()?.parse::<usize>().ok()?;
            Some((index, evaluations, line))
        })
        .collect();
    assert_eq!(rows.len(), it + 1);
    assert_eq!((rows[0].0, rows[0].1), (0, 1));
    assert_eq!(rows[0].2.split_whitespace().count(), 3);
    assert_eq!((rows[1].0, rows[1].1), (1, 2));
    assert!(rows[1].2.ends_with("initial simplex"));
    for (index, &(iteration, _, line)) in rows.iter().enumerate().skip(2) {
        assert_eq!(iteration, index);
        assert!(
            [
                "reflect",
                "expand",
                "contract inside",
                "contract outside",
                "shrink"
            ]
            .into_iter()
            .any(|procedure| line.ends_with(procedure))
        );
    }

    // fminbnd Display 'iter'
    let res = e
        .execute("[x, fv, ef, out] = fminbnd(@cos, 2, 4, optimset('Display', 'iter'));")
        .unwrap();
    assert!(!res.output.iter().any(|l| l.contains("Note:")));
    assert!(res.output.iter().any(|l| l.contains("Func-count")
        && l.contains("x")
        && l.contains("f(x)")
        && l.contains("Procedure")));
    e.execute("it = out.iterations;").unwrap();
    let it = scalar(&e, "it") as usize;
    let proc_rows = res
        .output
        .iter()
        .filter(|l| l.contains("initial") || l.contains("golden") || l.contains("parabolic"))
        .count();
    assert_eq!(proc_rows, it + 1);

    // fzero Display 'iter' with scalar x0 (includes interval-search table + Brent table)
    let res = e
        .execute("[x, fv, ef, out] = fzero(@(x) x^3 - 2*x - 5, 2, optimset('Display', 'iter'));")
        .unwrap();
    assert!(!res.output.iter().any(|l| l.contains("Note:")));
    assert!(
        res.output
            .iter()
            .any(|l| l.contains("Func-count") && l.contains("f(a)") && l.contains("f(b)"))
    );
    assert!(
        res.output
            .iter()
            .any(|l| l.contains("Func-count") && l.contains("f(x)") && l.contains("Procedure"))
    );
    e.execute("it = out.iterations; ii = out.intervaliterations;")
        .unwrap();
    let it = scalar(&e, "it") as usize;
    let ii = scalar(&e, "ii") as usize;
    let brent_rows = res
        .output
        .iter()
        .filter(|l| {
            (l.contains("initial") && !l.contains("initial interval"))
                || l.contains("interpolation")
                || l.contains("bisection")
        })
        .count();
    let interval_rows = res
        .output
        .iter()
        .filter(|l| l.contains("initial interval") || l.contains("search"))
        .count();
    assert_eq!(brent_rows, it + 1);
    assert_eq!(interval_rows, ii + 1);
}

#[test]
fn outputfcn_stop_and_optimoptions_and_plotfcns() {
    let dir = tempfile::tempdir().unwrap();
    let fn_path = dir.path().join("stop_at_two.m");
    std::fs::write(
        &fn_path,
        "function stop = stop_at_two(x, optimValues, state)\n\
         global LOG_STATES LOG_ITERS;\n\
         LOG_STATES{end+1} = state;\n\
         LOG_ITERS(end+1) = optimValues.iteration;\n\
         stop = (optimValues.iteration >= 2);\n\
         end\n",
    )
    .unwrap();

    let mut e = Engine::default();
    e.set_function_directory(dir.path()).unwrap();
    e.execute("global LOG_STATES LOG_ITERS; LOG_STATES = {}; LOG_ITERS = [];")
        .unwrap();
    e.execute("[x, fv, ef, out] = fminsearch(@(x) (x - 3)^2, 0, optimset('OutputFcn', @stop_at_two, 'Display', 'off'));")
        .unwrap();
    assert_eq!(scalar(&e, "ef"), -1.0);
    e.execute("s1 = LOG_STATES{1}; s_last = LOG_STATES{end}; n_states = numel(LOG_STATES);")
        .unwrap();
    assert_eq!(text_of(&e, "s1"), "init");
    assert_eq!(text_of(&e, "s_last"), "done");
    assert!(scalar(&e, "n_states") >= 3.0);

    // fzero with optimoptions + FunValCheck 'on'
    e.execute("opts = optimoptions('fzero', 'StepTolerance', 1e-6, 'Display', 'final');")
        .unwrap();
    let res = e
        .execute("[xz, fvz, efz] = fzero(@cos, [1 2], opts);")
        .unwrap();
    assert_eq!(scalar(&e, "efz"), 1.0);
    assert!(!res.output.is_empty());

    // FunValCheck 'on' errors on NaN
    let err = e
        .execute("fminsearch(@(x) NaN, 1, optimset('FunValCheck', 'on'));")
        .unwrap_err();
    assert!(
        err.contains("non-finite") || err.contains("complex"),
        "{err}"
    );

    // PlotFcns @optimplotx and @optimplotfval run cleanly
    e.execute("fminsearch(@(x) sum((x - [1 2]).^2), [0 0], optimset('PlotFcns', {@optimplotx, @optimplotfval}, 'MaxIter', 3, 'Display', 'off'));")
        .unwrap();
}
