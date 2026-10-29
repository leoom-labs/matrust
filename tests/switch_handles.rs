use matrust::{Array, Engine};
use std::sync::atomic::Ordering;

fn number(e: &Engine, name: &str) -> f64 {
    e.array(name)
        .unwrap_or_else(|| panic!("{name} is not defined"))
        .scalar(0, 0)
        .unwrap()
        .into_real()
        .unwrap()
}
fn row(e: &Engine, name: &str) -> Vec<f64> {
    match e.array(name).unwrap() {
        Array::Numeric(m) => m.data().to_vec(),
        Array::Logical(m) => m.data().iter().map(|&v| f64::from(v)).collect(),
        other => panic!("{name} is {}", other.class()),
    }
}
fn text(e: &Engine, name: &str) -> String {
    let Array::Character(c) = e.array(name).unwrap() else {
        panic!("{name} is not char");
    };
    String::from_utf16(c.units()).unwrap()
}
fn run(e: &mut Engine, source: &str) -> Vec<String> {
    e.execute(source)
        .unwrap_or_else(|error| panic!("{source}\n{error}"))
        .output
}
fn fails(e: &mut Engine, source: &str) -> String {
    e.execute(source)
        .err()
        .unwrap_or_else(|| panic!("expected an error: {source}"))
}

/// The error without its "Line N, column M: " position prefix.
fn plain(e: &mut Engine, source: &str) -> String {
    let error = fails(e, source);
    if let Some(rest) = error.strip_prefix("Line ")
        && let Some((_, message)) = rest.split_once(": ")
    {
        return message.to_owned();
    }
    error
}

// ---- switch ----

#[test]
fn switch_matches_numbers_text_and_cells_and_first_match_wins() {
    let mut e = Engine::default();
    run(
        &mut e,
        "x = 2;
switch x
  case 1
    r = 'one';
  case 2
    r = 'two';
  case {2, 3}
    r = 'later';
  otherwise
    r = 'other';
end",
    );
    assert_eq!(text(&e, "r"), "two");
    run(
        &mut e,
        "switch 'abc'\n case {'x','abc'}\n  r = 'cell';\n case 'abc'\n  r = 'plain';\nend",
    );
    assert_eq!(text(&e, "r"), "cell");
    // Text compares exactly: case matters and a number never equals a character.
    run(
        &mut e,
        "switch 'abc'\n case 'ABC'\n  r = 'ci';\n case 97\n  r = 'num';\n otherwise\n  r = 'exact';\nend",
    );
    assert_eq!(text(&e, "r"), "exact");
    // Logical true equals 1; a cell case holds mixed types.
    run(
        &mut e,
        "switch true\n case {'t', 1}\n  r = 'hit';\n otherwise\n  r = 'miss';\nend",
    );
    assert_eq!(text(&e, "r"), "hit");
    run(&mut e, "switch 2.5\n case 2.5\n  n = 1;\n end");
    assert_eq!(number(&e, "n"), 1.);
}

#[test]
fn switch_without_match_empty_bodies_and_no_fall_through() {
    let mut e = Engine::default();
    run(
        &mut e,
        "r = 0; switch 5\n case 1\n  r = 1;\n case 2\n  r = 2;\nend",
    );
    assert_eq!(number(&e, "r"), 0.);
    run(&mut e, "r = 0; switch 1\nend");
    run(&mut e, "r = 0; switch 1\n case 1\n case 2\n  r = 2;\nend");
    assert_eq!(number(&e, "r"), 0.);
    run(&mut e, "r = 0; switch 1\n otherwise\nend");
    run(
        &mut e,
        "r = 0; switch 1\n case 1\n  r = r + 1;\n case 1\n  r = r + 10;\nend",
    );
    assert_eq!(number(&e, "r"), 1.);
    // Case expressions after the match are not evaluated.
    run(
        &mut e,
        "switch 1\n case 1\n  r = 7;\n case missing_name\n  r = 8;\nend",
    );
    assert_eq!(number(&e, "r"), 7.);
    // NaN never matches, not even NaN.
    run(
        &mut e,
        "r = 0; switch NaN\n case NaN\n  r = 1;\n otherwise\n  r = 2;\nend",
    );
    assert_eq!(number(&e, "r"), 2.);
}

#[test]
fn switch_one_line_forms_for_the_command_window() {
    let mut e = Engine::default();
    run(
        &mut e,
        "x = 3; switch x, case 1, y = 10; case 3, y = 30; otherwise, y = 0; end",
    );
    assert_eq!(number(&e, "y"), 30.);
    run(
        &mut e,
        "switch 'go', case 'stop', y = 1; case 'go', y = 2; end",
    );
    assert_eq!(number(&e, "y"), 2.);
    // A string right after a keyword and after whitespace inside brackets.
    run(&mut e, "s = 'a'; t = [s 'x' 'y']; c = {s 'z'};");
    assert_eq!(text(&e, "t"), "axy");
    run(&mut e, "m = [1 2 3]'; n = [m' ; m'];");
    assert_eq!(row(&e, "m"), vec![1., 2., 3.]);
}

#[test]
fn switch_in_loops_supports_break_continue_and_nesting() {
    let mut e = Engine::default();
    run(
        &mut e,
        "total = 0; seen = 0;
for k = 1:10
  switch mod2(k)
    case 0
      continue
    case 1
      total = total + k;
  end
  switch k
    case 7
      break
  end
  seen = k;
end
function r = mod2(k)
  r = k - 2 * floor(k / 2);
end",
    );
    // Odd numbers 1,3,5,7 are added before the break at k = 7.
    assert_eq!(number(&e, "total"), 16.);
    assert_eq!(number(&e, "seen"), 5.);
    run(
        &mut e,
        "n = 0; for a = 1:3
  switch a
    case {1, 2}
      switch a
        case 1
          n = n + 1;
        otherwise
          n = n + 20;
      end
    otherwise
      n = n + 300;
  end
end",
    );
    assert_eq!(number(&e, "n"), 321.);
    run(
        &mut e,
        "i = 0; while true\n i = i + 1;\n switch i\n  case 4\n   break\n end\nend",
    );
    assert_eq!(number(&e, "i"), 4.);
}

#[test]
fn switch_works_inside_functions_and_returns() {
    let mut e = Engine::default();
    run(
        &mut e,
        "a = sign3(-4); b = sign3(0); c = sign3(9);
function r = sign3(x)
  switch x < 0
    case true
      r = -1;
      return
  end
  switch x
    case 0
      r = 0;
    otherwise
      r = 1;
  end
end",
    );
    assert_eq!(
        (number(&e, "a"), number(&e, "b"), number(&e, "c")),
        (-1., 0., 1.)
    );
}

#[test]
fn switch_rejects_bad_subjects_and_bad_syntax() {
    let mut e = Engine::default();
    for source in [
        "switch {1}\n case 1\n  x = 1;\nend",
        "switch [1 2]\n case 1\n  x = 1;\nend",
    ] {
        let error = fails(&mut e, source);
        assert!(error.contains("SWITCH expression"), "{error}");
    }
    for source in [
        "switch 1\n x = 1;\nend",
        "switch 1\n case 1\n x = 1;\n",
        "case 1\n",
        "switch 1\n otherwise\n x = 1;\n case 2\nend",
        "if true\n case 1\nend",
    ] {
        assert!(e.execute(source).is_err(), "{source}");
    }
    // A cancelled run stops inside a switch body.
    let cancel = e.cancellation();
    cancel.store(true, Ordering::Relaxed);
    assert!(fails(&mut e, "switch 1\n case 1\n  x = 1;\nend").contains("cancelled"));
    cancel.store(false, Ordering::Relaxed);
}

// ---- function handles ----

#[test]
fn anonymous_handles_capture_a_snapshot_and_call_with_any_arity() {
    let mut e = Engine::default();
    run(&mut e, "a = 5; f = @(x) x + a; a = 100; r = f(1);");
    assert_eq!(number(&e, "r"), 6.);
    run(&mut e, "g = @(x, y) x .* y + 1; r = g([1 2 3], 2);");
    assert_eq!(row(&e, "r"), vec![3., 5., 7.]);
    run(&mut e, "z = @() 42; r = z();");
    assert_eq!(number(&e, "r"), 42.);
    // Nested handles and handles that return handles.
    run(
        &mut e,
        "adder = @(a) @(b) a + b; add3 = adder(3); r = add3(4); s = adder(10)(1);",
    );
    assert_eq!(number(&e, "r"), 7.);
    assert_eq!(number(&e, "s"), 11.);
    run(
        &mut e,
        "k = 2; h = @(x) arrayfun(@(y) y * k, x); r = h([1 2 3]); k = 0;",
    );
    assert_eq!(row(&e, "r"), vec![2., 4., 6.]);
    run(&mut e, "r = h([1 2 3]);");
    assert_eq!(row(&e, "r"), vec![2., 4., 6.]);
    // A missing argument fails only when the body reads it.
    run(&mut e, "p = @(x, y) x; r = p(9);");
    assert_eq!(number(&e, "r"), 9.);
    assert_eq!(
        plain(&mut e, "q = @(x, y) y; q(9)"),
        "Not enough input arguments."
    );
    assert_eq!(plain(&mut e, "p(1, 2, 3)"), "Too many input arguments.");
    // varargin collects trailing arguments in a cell.
    run(
        &mut e,
        "v = @(varargin) numel(varargin); r = v(1, 'a', 3); s = v();",
    );
    assert_eq!((number(&e, "r"), number(&e, "s")), (3., 0.));
}

#[test]
fn captured_variables_are_snapshots_even_for_arrays_and_cells() {
    let mut e = Engine::default();
    run(
        &mut e,
        "A = [1 2 3]; C = {10, 20}; f = @() A(2) + C{2}; A(2) = 0; C{2} = 0; r = f();",
    );
    assert_eq!(number(&e, "r"), 22.);
    // The handle shares the captured storage instead of copying it.
    run(&mut e, "B = ones(1, 1000); g = @() B(1);");
    let before = match e.array("B").unwrap() {
        Array::Numeric(m) => m.data().as_ptr(),
        _ => unreachable!(),
    };
    run(&mut e, "B(1) = 5; r = g();");
    assert_eq!(number(&e, "r"), 1.);
    let after = match e.array("B").unwrap() {
        Array::Numeric(m) => m.data().as_ptr(),
        _ => unreachable!(),
    };
    assert_ne!(before, after);
}

#[test]
fn named_handles_resolve_builtins_and_user_functions_at_call_time() {
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(
        folder.path().join("triple.mr"),
        "function y = triple(x)\ny = 3 * x;\nend\n",
    )
    .unwrap();
    let mut e = Engine::default();
    e.set_function_directory(folder.path()).unwrap();
    run(&mut e, "s = @sin; r = s(0); t = @triple; u = t(14);");
    assert_eq!(number(&e, "r"), 0.);
    assert_eq!(number(&e, "u"), 42.);
    // Created before the function exists; resolved when called.
    run(&mut e, "late = @quad4;");
    let error = fails(&mut e, "late(1)");
    assert!(error.contains("quad4"), "{error}");
    std::fs::write(
        folder.path().join("quad4.mr"),
        "function y = quad4(x)\ny = 4 * x;\nend\n",
    )
    .unwrap();
    run(&mut e, "r = late(2);");
    assert_eq!(number(&e, "r"), 8.);
    // A local function of the submitted script.
    run(
        &mut e,
        "h = @twice; r = h(21);\nfunction y = twice(x)\ny = 2 * x;\nend",
    );
    assert_eq!(number(&e, "r"), 42.);
    // Multiple outputs through a handle to a user function.
    std::fs::write(
        folder.path().join("both.mr"),
        "function [a, b] = both(x)\na = x;\nb = -x;\nend\n",
    )
    .unwrap();
    run(&mut e, "hb = @both; [p, q] = hb(3);");
    assert_eq!((number(&e, "p"), number(&e, "q")), (3., -3.));
    // A function without outputs runs for its effect.
    std::fs::write(
        folder.path().join("noisy.mr"),
        "function noisy(x)\ndisp(x);\nend\n",
    )
    .unwrap();
    let output = run(&mut e, "hn = @noisy; hn(5)");
    assert_eq!(output.len(), 1);
    assert!(output[0].contains('5'), "{output:?}");
    assert!(e.array("ans").is_none());
}

#[test]
fn func2str_str2func_and_class() {
    let mut e = Engine::default();
    run(
        &mut e,
        "h = @(x) x.^2 + 1; a = func2str(h); g = @sin; b = func2str(g); c = class(h);
k = str2func('sin'); d = k(0); m = str2func('@(x) x+1'); n = m(1); o = func2str(m);
w = @(x,y) [x, y; y x]; p = func2str(w); q = func2str(@(s) [s 'x']);",
    );
    assert_eq!(text(&e, "a"), "@(x)x.^2+1");
    assert_eq!(text(&e, "b"), "sin");
    assert_eq!(text(&e, "c"), "function_handle");
    assert_eq!(number(&e, "d"), 0.);
    assert_eq!(number(&e, "n"), 2.);
    assert_eq!(text(&e, "o"), "@(x)x+1");
    // Whitespace separators print as commas, as MATLAB does.
    assert_eq!(text(&e, "p"), "@(x,y)[x,y;y,x]");
    assert_eq!(text(&e, "q"), "@(s)[s,'x']");
    // Round trip: func2str output rebuilds an equivalent handle.
    run(
        &mut e,
        "h2 = str2func(func2str(h)); r = h2(3); t = func2str(h2);",
    );
    assert_eq!(number(&e, "r"), 10.);
    assert_eq!(text(&e, "t"), "@(x)x.^2+1");
    run(
        &mut e,
        "n2 = str2func('@sin'); r = n2(0); f3 = func2str(n2);",
    );
    assert_eq!(text(&e, "f3"), "sin");
    // Handles are 1x1 function_handle values in the workspace.
    let value = e.array("h").unwrap();
    assert_eq!(
        (value.rows(), value.cols(), value.class()),
        (1, 1, "function_handle")
    );
    assert!(fails(&mut e, "func2str(3)").contains("function handle"));
    assert!(fails(&mut e, "str2func('1bad')").contains("invalid"));
    // str2func captures nothing from the workspace.
    run(&mut e, "zz = 5; cap = str2func('@(x) x + zz');");
    assert!(fails(&mut e, "cap(1)").contains("'zz'") || e.array("zz").is_some());
}

#[test]
fn handle_display_follows_matlab() {
    let mut e = Engine::default();
    let output = run(&mut e, "h = @(x) x.^2 + 1");
    assert_eq!(output, vec!["h = \n\n    @(x)x.^2+1\n".to_string()]);
    let output = run(&mut e, "g = @sin");
    assert_eq!(output, vec!["g = \n\n    @sin\n".to_string()]);
    assert!(run(&mut e, "q = @(x) x;").is_empty());
    let output = run(&mut e, "disp(h)");
    assert_eq!(output, vec!["    @(x)x.^2+1".to_string()]);
    let output = run(
        &mut e,
        "[a, b] = deal2(@sin, @(t) t)\nfunction [x, y] = deal2(p, q)\nx = p;\ny = q;\nend",
    );
    assert!(output[0].starts_with("a = \n\n    @sin"), "{output:?}");
    assert!(output[1].starts_with("b = \n\n    @(t)t"), "{output:?}");
}

#[test]
fn handles_live_in_cells_pass_to_functions_and_cannot_be_used_as_numbers() {
    let mut e = Engine::default();
    run(
        &mut e,
        "fs = {@sin, @(x) x + 1, @numel}; f = fs{2}; r = f(1); s = apply(fs{3}, [4 5 6]);
function y = apply(g, v)
  y = g(v);
end",
    );
    assert_eq!(number(&e, "r"), 2.);
    assert_eq!(number(&e, "s"), 3.);
    run(&mut e, "c = class(fs{1}); n = numel(fs);");
    assert_eq!(text(&e, "c"), "function_handle");
    assert_eq!(number(&e, "n"), 3.);
    assert!(e.execute("h = @sin; x = h + 1;").is_err());
    assert!(e.execute("h = @sin; h(1) = 2;").is_err());
    assert!(e.execute("h = @sin; x = h{1};").is_err());
}

#[test]
fn feval_calls_names_and_handles() {
    let mut e = Engine::default();
    run(
        &mut e,
        "a = feval('cos', 0); b = feval(@sin, 0); c = feval(@(x, y) x - y, 5, 3);
d = feval(@twice, 4);
function y = twice(x)
  y = 2 * x;
end",
    );
    assert_eq!(
        (
            number(&e, "a"),
            number(&e, "b"),
            number(&e, "c"),
            number(&e, "d")
        ),
        (1., 0., 2., 8.)
    );
    assert!(fails(&mut e, "feval(3)").contains("function handle or function name"));
    assert!(fails(&mut e, "feval(@nosuchfunction_xyz, 1)").contains("nosuchfunction_xyz"));
    assert!(fails(&mut e, "feval()").contains("feval"));
}

#[test]
fn arrayfun_collects_uniform_and_cell_results() {
    let mut e = Engine::default();
    run(&mut e, "r = arrayfun(@(k) k*2, 1:3);");
    assert_eq!(row(&e, "r"), vec![2., 4., 6.]);
    run(
        &mut e,
        "r = arrayfun(@(a, b) a + b, [1 2; 3 4], [10 20; 30 40]);",
    );
    let Array::Numeric(m) = e.array("r").unwrap() else {
        panic!()
    };
    assert_eq!((m.rows(), m.cols()), (2, 2));
    assert_eq!(m.data(), &[11., 33., 22., 44.]);
    run(&mut e, "t = arrayfun(@(k) k > 1, [1 2 3]); c = class(t);");
    assert_eq!(text(&e, "c"), "logical");
    assert_eq!(row(&e, "t"), vec![0., 1., 1.]);
    run(
        &mut e,
        "c = arrayfun(@(k) k * [1 1], 1:2, 'UniformOutput', false);",
    );
    let Array::Cell(c) = e.array("c").unwrap() else {
        panic!()
    };
    assert_eq!((c.rows(), c.cols()), (1, 2));
    assert_eq!(
        c.get_linear(1).unwrap().as_numeric().unwrap().data(),
        &[2., 2.]
    );
    run(
        &mut e,
        "c = arrayfun(@(k) k, 1:3, 'uniformoutput', 0); n = numel(c);",
    );
    assert_eq!(number(&e, "n"), 3.);
    run(
        &mut e,
        "[mx, mn] = arrayfun(@(k) deal2(k), 1:3);\nfunction [a, b] = deal2(k)\na = k;\nb = -k;\nend",
    );
    assert_eq!(row(&e, "mx"), vec![1., 2., 3.]);
    assert_eq!(row(&e, "mn"), vec![-1., -2., -3.]);
    run(
        &mut e,
        "e = arrayfun(@(k) k, zeros(0, 3)); c = arrayfun(@(k) k, [], 'UniformOutput', false);",
    );
    let Array::Numeric(m) = e.array("e").unwrap() else {
        panic!()
    };
    assert_eq!((m.rows(), m.cols()), (0, 3));
    let Array::Cell(c) = e.array("c").unwrap() else {
        panic!()
    };
    assert_eq!(c.len(), 0);
    // Function names work and effects run once per element, in order.
    run(&mut e, "r = arrayfun('sqrt', [4 9]);");
    assert_eq!(row(&e, "r"), vec![2., 3.]);
    let output = run(&mut e, "arrayfun(@(k) disp(k), 1:3);");
    assert_eq!(output.len(), 3);
    assert!(
        output[0].contains('1') && output[2].contains('3'),
        "{output:?}"
    );
    // Characters and cells are mapped element by element.
    run(&mut e, "d = arrayfun(@(ch) isempty(ch), 'AB');");
    assert_eq!(row(&e, "d"), vec![0., 0.]);
    run(&mut e, "r = arrayfun(@(ch) ch, 'abc'); k = class(r);");
    assert_eq!(text(&e, "r"), "abc");
}

#[test]
fn arrayfun_reports_matlab_style_errors() {
    let mut e = Engine::default();
    let error = fails(&mut e, "arrayfun(@(k) [k k], 1:3)");
    assert!(
        error.contains(
            "Non-scalar in Uniform output, at index 1, output 1.\nSet 'UniformOutput' to false."
        ),
        "{error}"
    );
    let error = fails(&mut e, "arrayfun(@(k) 1:k, 1:3)");
    assert!(error.contains("index 2"), "{error}");
    let error = fails(&mut e, "arrayfun(@(a, b) a + b, 1:3, 1:4)");
    assert!(
        error.contains("All of the input arguments must be of the same size and shape."),
        "{error}"
    );
    assert!(error.contains("Input #3"), "{error}");
    let error = fails(&mut e, "arrayfun(@(k) k, 1:3, 'ErrorHandler', @(s, k) 0)");
    assert!(
        error.contains("ErrorHandler is not supported yet"),
        "{error}"
    );
    let error = fails(&mut e, "arrayfun(@(k) {1, k}, 1:2)");
    assert!(error.contains("Set 'UniformOutput' to false"), "{error}");
    assert!(fails(&mut e, "arrayfun(@(k) k)").contains("Not enough input arguments"));
    assert!(fails(&mut e, "arrayfun(3, 1:2)").contains("function handle or function name"));
    // A class change between elements is an error in uniform mode.
    let error = fails(
        &mut e,
        "arrayfun(@(k) iff3(k), 1:2)\nfunction r = iff3(k)\nif k == 1\nr = true;\nelse\nr = 2;\nend\nend",
    );
    assert!(
        error.ends_with(
            "Mismatch in type of outputs, at index 2, output 1 (logical versus double).\nSet 'UniformOutput' to false."
        ),
        "{error}"
    );
}

#[test]
fn cellfun_uniform_nonuniform_and_legacy_names() {
    let mut e = Engine::default();
    run(&mut e, "r = cellfun(@numel, {'ab', 'cde'});");
    assert_eq!(row(&e, "r"), vec![2., 3.]);
    run(
        &mut e,
        "c = cellfun(@(s) [s 'x'], {'a', 'b'}, 'UniformOutput', false);",
    );
    let Array::Cell(c) = e.array("c").unwrap() else {
        panic!()
    };
    let Array::Character(first) = c.get_linear(0).unwrap().as_ref() else {
        panic!()
    };
    let Array::Character(second) = c.get_linear(1).unwrap().as_ref() else {
        panic!()
    };
    assert_eq!(String::from_utf16(first.units()).unwrap(), "ax");
    assert_eq!(String::from_utf16(second.units()).unwrap(), "bx");
    run(
        &mut e,
        "C = {1, [], 'abc', {}, zeros(0, 3)}; a = cellfun('isempty', C); b = cellfun(@isempty, C); n = cellfun('length', C); m = cellfun('numel', C); d = cellfun('ndims', C); p = cellfun('prodofsize', C); ca = class(a);",
    );
    assert_eq!(row(&e, "a"), vec![0., 1., 0., 1., 1.]);
    assert_eq!(row(&e, "b"), vec![0., 1., 0., 1., 1.]);
    assert_eq!(row(&e, "n"), vec![1., 0., 3., 0., 0.]);
    assert_eq!(row(&e, "m"), vec![1., 0., 3., 0., 0.]);
    assert_eq!(row(&e, "d"), vec![2., 2., 2., 2., 2.]);
    assert_eq!(row(&e, "p"), vec![1., 0., 3., 0., 0.]);
    assert_eq!(text(&e, "ca"), "logical");
    run(
        &mut e,
        "L = cellfun(@(a, b) a + b, {1, 2; 3, 4}, {10, 20; 30, 40});",
    );
    let Array::Numeric(m) = e.array("L").unwrap() else {
        panic!()
    };
    assert_eq!((m.rows(), m.cols()), (2, 2));
    assert_eq!(m.data(), &[11., 33., 22., 44.]);
    run(
        &mut e,
        "u = cellfun(@upper2, {'a', 'b'}, 'UniformOutput', false);\nfunction y = upper2(s)\ny = [s s];\nend",
    );
    assert_eq!(e.array("u").unwrap().len(), 2);
    run(
        &mut e,
        "[lo, hi] = cellfun(@(v) deal2(v), {1, 5, 3});\nfunction [a, b] = deal2(v)\na = v;\nb = v * 10;\nend",
    );
    assert_eq!(row(&e, "lo"), vec![1., 5., 3.]);
    assert_eq!(row(&e, "hi"), vec![10., 50., 30.]);
    // Errors.
    let error = fails(&mut e, "cellfun(@(x) x, {1, [2 3]})");
    assert!(
        error.contains("Non-scalar in Uniform output, at index 2, output 1."),
        "{error}"
    );
    let error = fails(&mut e, "cellfun(@numel, [1 2 3])");
    assert!(
        error.contains("Input #2 expected to be a cell array, was double instead."),
        "{error}"
    );
    let error = fails(&mut e, "cellfun(@numel, {1, 2}, {3})");
    assert!(error.contains("same size and shape"), "{error}");
    let error = fails(&mut e, "cellfun('size', {1, 2}, 1)");
    assert!(error.contains("not supported yet"), "{error}");
    let error = fails(&mut e, "cellfun(@numel, {1}, 'ErrorHandler', @(s, x) 0)");
    assert!(error.contains("not supported yet"), "{error}");
}

#[test]
fn handle_recursion_stops_cleanly_at_the_depth_limit() {
    let mut e = Engine::default();
    // A handle that calls itself through an argument never terminates on its own.
    let error = fails(&mut e, "g = @(f, n) f(f, n + 1); g(g, 1)");
    assert!(error.contains("exceeds 128 levels"), "{error}");
    // The engine remains usable and keeps earlier workspace variables.
    run(&mut e, "ok = g(@(f, n) n * 2, 4);");
    assert_eq!(number(&e, "ok"), 10.);
    // Recursion through a named handle to a local function.
    let error = fails(
        &mut e,
        "h = @loop; h(1)\nfunction y = loop(n)\nh2 = @loop;\ny = h2(n + 1);\nend",
    );
    assert!(
        error.contains("exceeds") || error.contains("depth"),
        "{error}"
    );
    // Long handle-in-handle chains are bounded.
    let error = fails(
        &mut e,
        "f = @(x) x; for k = 1:200\n f = @(x) f(x) + 1;\nend",
    );
    assert!(error.contains("limited to 100 levels"), "{error}");
}

#[test]
fn handle_calls_observe_cancellation_and_loop_budgets() {
    let mut e = Engine::default();
    let cancel = e.cancellation();
    cancel.store(true, Ordering::Relaxed);
    assert!(fails(&mut e, "f = @(x) x + 1; f(1)").contains("cancelled"));
    assert!(fails(&mut e, "r = arrayfun(@(x) x, 1:5);").contains("cancelled"));
    cancel.store(false, Ordering::Relaxed);
    // Cancelling in the middle of a long arrayfun stops it.
    let stop = cancel.clone();
    let thread = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(20));
        stop.store(true, Ordering::Relaxed);
    });
    let error = e
        .execute("while true\n r = arrayfun(@(x) sum(sin(1:2000)) + x, 1:20000);\nend")
        .unwrap_err();
    thread.join().unwrap();
    assert!(
        error.contains("cancelled") || error.contains("exceeds"),
        "{error}"
    );
    cancel.store(false, Ordering::Relaxed);
    // Every element counts against the 1000000 step budget.
    let error = fails(
        &mut e,
        "r = arrayfun(@(x) x, 1:600000); s = arrayfun(@(x) x, 1:600000);",
    );
    assert!(error.contains("1000000"), "{error}");
    run(&mut e, "after = 1;");
}

#[test]
fn saving_handles_is_not_supported_yet() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("h.mat");
    let path = path.to_str().unwrap().replace('\\', "/");
    let mut e = Engine::default();
    for version in ["-v7", "-v7.3", "-v6"] {
        let error = fails(
            &mut e,
            &format!("h = @(x) x + 1; save('{path}', 'h', '{version}');"),
        );
        assert!(
            error.contains("function handles are not supported yet"),
            "{version}: {error}"
        );
        let error = fails(
            &mut e,
            &format!("c = {{1, @sin}}; save('{path}', 'c', '{version}');"),
        );
        assert!(
            error.contains("function handles are not supported yet"),
            "{version}: {error}"
        );
    }
    assert!(!std::path::Path::new(&path).exists());
}

#[test]
fn handle_recursion_fits_a_one_megabyte_stack() {
    // The Windows main thread has 1 MiB; the depth limit must stop first.
    let thread = std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(|| {
            let mut e = Engine::default();
            let error = fails(&mut e, "g = @(f, n) f(f, n + 1); g(g, 1)");
            assert!(error.contains("exceeds 128 levels"), "{error}");
            let error = fails(
                &mut e,
                "h = @(f, n) [f(f, n + 1), 1] .* 2 + abs(f(f, n));\nh(h, 1)",
            );
            assert!(error.contains("exceeds"), "{error}");
            let error = fails(&mut e, "c = @(f, n) arrayfun(@(k) f(f, k), n); c(c, 1)");
            assert!(error.contains("exceeds"), "{error}");
            let error = fails(
                &mut e,
                "h = @loop; h(1)\nfunction y = loop(n)\nh2 = @loop;\ny = h2(n + 1);\nend",
            );
            assert!(error.contains("exceeds"), "{error}");
        })
        .unwrap();
    thread.join().unwrap();
}

#[test]
fn session_recovery_rejects_handles_explicitly() {
    use matrust::{document::Document, session::Session, simulation::Model};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let mut e = Engine::default();
    run(&mut e, "keep = 1; h = @(x) x + 1;");
    let make = |workspace| Session {
        layout_presets: Default::default(),
        export: Default::default(),
        workspace: Some(workspace),
        workspace_manifest: None,
        preview: None,
        eps_renderer: None,
        figure_manifest: None,
        figures: None,
        layout: Default::default(),
        version: 1,
        documents: vec![Document::new()],
        active: 0,
        folder: std::env::current_dir().unwrap(),
        recent: vec![],
        history: vec![],
        model: Model::default(),
        model_path: None,
        model_baseline: None,
    };
    let error = make(e.workspace()).save(&path).unwrap_err();
    assert!(
        error.contains("function handles is not supported yet"),
        "{error}"
    );
    run(&mut e, "clear h; s.f=@sin; c={s};");
    make(e.workspace()).save(&path).unwrap();
    let manifest = Session::load(&path).unwrap().workspace_manifest.unwrap();
    assert_eq!(manifest.skipped, ["c", "s"]);
    assert!(manifest.entries.contains_key("keep"));
    assert!(manifest.skipped_note().unwrap().contains("structs"));
    // Without either unsupported container the same session saves.
    let mut values = e.workspace();
    values.remove("s");
    values.remove("c");
    make(values).save(&path).unwrap();
}

#[test]
fn handles_stored_in_cells_built_in_loops_and_applied_directly() {
    let mut e = Engine::default();
    run(
        &mut e,
        "fs = cell(1, 3); for k = 1:3, fs{k} = @(x) x + k; end
r = fs{2}(10); v = cellfun(@(f) f(1), fs); w = adder(1)(2)(3);
function h = adder(a)
  h = @(b) @(c) a + b + c;
end",
    );
    assert_eq!(number(&e, "r"), 12.);
    assert_eq!(row(&e, "v"), vec![2., 3., 4.]);
    assert_eq!(number(&e, "w"), 6.);
    let error = fails(&mut e, "x = [1 2 3]; y = x(2)(1);");
    assert!(error.contains("not supported yet"), "{error}");
    // A space before the second parenthesis keeps matrix elements separate.
    run(&mut e, "h = @(x) x; m = [h(1) (2)];");
    assert_eq!(row(&e, "m"), vec![1., 2.]);
}

#[test]
fn new_names_are_completable_and_documented() {
    for name in [
        "switch",
        "case",
        "otherwise",
        "func2str",
        "str2func",
        "feval",
        "arrayfun",
        "cellfun",
    ] {
        assert!(matrust::session::FUNCTIONS.contains(&name), "{name}");
    }
    for word in [
        "switch", "func2str", "str2func", "feval", "arrayfun", "cellfun",
    ] {
        assert!(matrust::engine::HELP.contains(word), "{word}");
    }
}

#[test]
fn mapping_admits_its_result_size_before_reserving_or_calling() {
    let mut e = Engine::default();
    // Any call would fail with "Undefined"; the budgets reject the request first.
    let error = fails(
        &mut e,
        "r = arrayfun(@(v) nosuch_map_fn(v), zeros(4096, 4096));",
    );
    assert!(error.contains("1000000 statement/loop steps"), "{error}");
    let error = fails(
        &mut e,
        "r = arrayfun(@(v) nosuch_map_fn(v), zeros(1, 1000001), 'UniformOutput', false);",
    );
    assert!(error.contains("1000000 statement/loop steps"), "{error}");
    // 999000 elements x 32 outputs plus the inputs computed in the same
    // statement exceed the 32 Mi element evaluation budget.
    let folder = tempfile::tempdir().unwrap();
    let names: Vec<String> = (1..=32).map(|k| format!("o{k}")).collect();
    std::fs::write(
        folder.path().join("many32.mr"),
        format!(
            "function [{}] = many32(x, y)\no1 = nosuch_map_fn(x);\nend\n",
            names.join(", ")
        ),
    )
    .unwrap();
    e.set_function_directory(folder.path()).unwrap();
    run(&mut e, "x = zeros(999, 1000);");
    let error = fails(
        &mut e,
        &format!("[{}] = arrayfun(@many32, x + 0, x + 0);", names.join(", ")),
    );
    assert!(error.contains("evaluation budget"), "{error}");
    // Ordinary sizes still map.
    run(&mut e, "r = arrayfun(@(v) v + 1, zeros(300, 300));");
    assert_eq!(e.array("r").unwrap().len(), 90000);
}

#[test]
fn standalone_handle_calls_request_no_outputs_like_direct_calls() {
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(
        folder.path().join("which_out.mr"),
        "function y = which_out()\nif nargout == 0\n  disp('none');\nelse\n  y = 7;\nend\nend\n",
    )
    .unwrap();
    std::fs::write(
        folder.path().join("show_one.mr"),
        "function y = show_one()\ndisp(1);\nend\n",
    )
    .unwrap();
    std::fs::write(
        folder.path().join("five.mr"),
        "function y = five()\ny = 5;\nend\n",
    )
    .unwrap();
    let mut e = Engine::default();
    e.set_function_directory(folder.path()).unwrap();
    // Direct and through a handle or feval: standalone sees nargout 0.
    let direct = run(&mut e, "which_out()");
    assert_eq!(direct.len(), 1);
    assert!(direct[0].contains("none"), "{direct:?}");
    for source in ["h = @which_out; h()", "feval(h)", "feval('which_out')"] {
        assert_eq!(run(&mut e, source), direct, "{source}");
    }
    assert!(e.array("ans").is_none());
    // Assigned, the same function returns its value.
    run(
        &mut e,
        "a = which_out(); b = h(); c = feval(h); d = feval('which_out');",
    );
    for name in ["a", "b", "c", "d"] {
        assert_eq!(number(&e, name), 7., "{name}");
    }
    // An unassigned output is fine for a standalone call.
    let output = run(&mut e, "s = @show_one; s()");
    assert_eq!(output.len(), 1);
    assert!(output[0].contains('1'), "{output:?}");
    assert!(e.array("ans").is_none());
    assert!(fails(&mut e, "t = s();").contains("not assigned"));
    // A returned value still becomes ans, shown as for a direct call.
    let direct = run(&mut e, "five()");
    run(&mut e, "clear ans");
    assert!(e.array("ans").is_none());
    assert_eq!(run(&mut e, "k = @five; k()"), direct);
    assert_eq!(number(&e, "ans"), 5.);
    run(&mut e, "clear ans");
    assert_eq!(run(&mut e, "feval(k)"), direct);
    assert_eq!(number(&e, "ans"), 5.);
    // Anonymous handles keep their behaviour.
    run(&mut e, "q = @() 3; q();");
    assert_eq!(number(&e, "ans"), 3.);
}

#[test]
fn named_handles_to_constants_ignore_workspace_variables() {
    let mut e = Engine::default();
    run(
        &mut e,
        "pi = 3; eps = 1; Inf = 0; NaN = 0; i = 2; j = 3;
hp = @pi; he = @eps; hi = @Inf; hn = @NaN; hi2 = @i; hj = @j;
a = hp(); b = he(); c = hi(); d = hn(); z = hi2(); w = hj(); f = feval('pi'); g = feval(@pi);",
    );
    assert_eq!(number(&e, "a"), std::f64::consts::PI);
    assert_eq!(number(&e, "b"), f64::EPSILON);
    assert_eq!(number(&e, "c"), f64::INFINITY);
    assert!(number(&e, "d").is_nan());
    assert_eq!(number(&e, "f"), std::f64::consts::PI);
    assert_eq!(number(&e, "g"), std::f64::consts::PI);
    for name in ["z", "w"] {
        let value = e.array(name).unwrap().scalar(0, 0).unwrap();
        assert!(
            matches!(value, matrust::Scalar::Complex(c) if c == matrust::Complex64::new(0., 1.)),
            "{name}: {value:?}"
        );
    }
    // The variable itself is unchanged.
    assert_eq!(number(&e, "pi"), 3.);
}

// ---- fix round 2: reference rows ----

fn word(e: &mut Engine, source: &str) -> String {
    run(e, &format!("r = 'none';\n{source}"));
    text(e, "r")
}

#[test]
fn switch_compares_one_character_and_numbers_by_value() {
    let mut e = Engine::default();
    // w4, w6, w16: the reference matches a character against a number by its code.
    assert_eq!(
        word(
            &mut e,
            "switch 'a'\n case 97\n  r = 'num';\n otherwise\n  r = 'other';\nend"
        ),
        "num"
    );
    assert_eq!(
        word(
            &mut e,
            "switch 97\n case 'a'\n  r = 'chr';\n otherwise\n  r = 'o';\nend"
        ),
        "chr"
    );
    assert_eq!(
        word(
            &mut e,
            "switch 'b'\n case {'a', 98}\n  r = 'mixed';\n otherwise\n  r = 'o';\nend"
        ),
        "mixed"
    );
    // Rows that already matched stay the same: w5 w7 w8 w9 w10 w13 w15.
    assert_eq!(
        word(
            &mut e,
            "switch true\n case 1\n  r = 'one';\n otherwise\n  r = 'other';\nend"
        ),
        "one"
    );
    assert_eq!(
        word(
            &mut e,
            "switch 3\n case [3 4]\n  r = 'arr';\n otherwise\n  r = 'oth';\nend"
        ),
        "oth"
    );
    assert_eq!(
        word(
            &mut e,
            "switch 'abc'\n case 'ABC'\n  r = 'ci';\n otherwise\n  r = 'cs';\nend"
        ),
        "cs"
    );
    assert_eq!(
        word(
            &mut e,
            "switch ''\n case ''\n  r = 'e';\n otherwise\n  r = 'o';\nend"
        ),
        "e"
    );
    assert_eq!(
        word(
            &mut e,
            "switch 2\n case {1, 2}\n  r = 'first';\n case 2\n  r = 'second';\nend"
        ),
        "first"
    );
    assert_eq!(
        word(
            &mut e,
            "switch 2.5\n case {1, 2.5}\n  r = 'cellnum';\n otherwise\n  r = 'o';\nend"
        ),
        "cellnum"
    );
    // A different code or a different class of value still does not match.
    assert_eq!(
        word(
            &mut e,
            "switch 'a'\n case 98\n  r = 'num';\n case {99, 'b'}\n  r = 'cell';\n otherwise\n  r = 'other';\nend"
        ),
        "other"
    );
    assert_eq!(
        word(
            &mut e,
            "switch 'a'\n case 97.5\n  r = 'num';\n otherwise\n  r = 'other';\nend"
        ),
        "other"
    );
}

#[test]
fn calls_with_the_wrong_number_of_arguments_use_matlab_texts() {
    let mut e = Engine::default();
    // e1, e7: too many arguments.
    assert_eq!(
        plain(&mut e, "h = @(x) x + 1; h(1, 2)"),
        "Too many input arguments."
    );
    assert_eq!(
        plain(&mut e, "h = @sin; h(1, 2)"),
        "Too many input arguments."
    );
    assert_eq!(
        plain(&mut e, "h = @(x) x + 1; feval(h, 1, 2)"),
        "Too many input arguments."
    );
    assert_eq!(
        plain(&mut e, "arrayfun(@(x) x, 1:3, 1:3)"),
        "Too many input arguments."
    );
    assert_eq!(
        plain(
            &mut e,
            "g = @one_in; g(1, 2)\nfunction y = one_in(x)\ny = x;\nend"
        ),
        "Too many input arguments."
    );
    // e2, e8: a parameter that was not passed and is read; it shadows functions.
    assert_eq!(
        plain(&mut e, "h = @(x, y) x + y; h(1)"),
        "Not enough input arguments."
    );
    assert_eq!(
        plain(&mut e, "g = @(pi) pi; g()"),
        "Not enough input arguments."
    );
    assert_eq!(
        plain(&mut e, "g = @(x, f) f(x); g(1)"),
        "Not enough input arguments."
    );
    assert_eq!(
        plain(&mut e, "h = @sin; h()"),
        "Not enough input arguments."
    );
    // A parameter that is never read can stay unset, and a passed one shadows pi.
    run(&mut e, "p = @(x, y) x * 2; r = p(4);");
    assert_eq!(number(&e, "r"), 8.);
    run(&mut e, "g = @(pi) pi + 1; r = g(2);");
    assert_eq!(number(&e, "r"), 3.);
    // An inner parameter of the same name is not the outer one.
    run(&mut e, "q = @(x, y) arrayfun(@(y) y, x); r = q(5);");
    assert_eq!(number(&e, "r"), 5.);
}

#[test]
fn uniform_output_type_changes_use_matlab_texts() {
    let mut e = Engine::default();
    let set = "Set 'UniformOutput' to false.";
    // a3, a4, a5
    assert_eq!(
        plain(&mut e, "x = {true, 2}; arrayfun(@(k) x{k}, 1:2)"),
        format!(
            "Mismatch in type of outputs, at index 2, output 1 (logical versus double).\n{set}"
        )
    );
    assert_eq!(
        plain(&mut e, "cellfun(@(x) x, {1, 'a'})"),
        format!("Mismatch in type of outputs, at index 2, output 1 (double versus char).\n{set}")
    );
    assert_eq!(
        plain(&mut e, "cellfun(@(x) x, {1, true})"),
        format!(
            "Mismatch in type of outputs, at index 2, output 1 (double versus logical).\n{set}"
        )
    );
    // The output number counts too (second output of a two-output map).
    assert_eq!(
        plain(
            &mut e,
            "[a, b] = arrayfun(@two, 1:2)\nfunction [p, q] = two(k)\np = k;\nq = 1 > k;\nif k == 2\nq = 'c';\nend\nend"
        ),
        format!("Mismatch in type of outputs, at index 2, output 2 (logical versus char).\n{set}")
    );
    // a10: a function handle result is not a uniform value, at any index.
    let handle = format!("function_handle output type is not supported. {set}");
    assert_eq!(plain(&mut e, "arrayfun(@(x) @sin, 1:2)"), handle);
    assert_eq!(plain(&mut e, "c = {1, @sin}; cellfun(@(x) x, c)"), handle);
    // Real and complex results still combine (a9).
    run(&mut e, "z = arrayfun(@(x) 1 + 2i * (x > 1), 1:3);");
    run(&mut e, "re = real(z); im = imag(z);");
    assert_eq!(row(&e, "re"), vec![1., 1., 1.]);
    assert_eq!(row(&e, "im"), vec![0., 2., 2.]);
}

#[test]
fn uniform_one_by_one_cell_results_join_into_one_cell_array() {
    let mut e = Engine::default();
    // a2: the reference returns the 1x3 cell {1, 2, 3} instead of an error.
    run(
        &mut e,
        "r = arrayfun(@(x) {x}, 1:3); c = class(r); n = size(r);",
    );
    assert_eq!(text(&e, "c"), "cell");
    assert_eq!(row(&e, "n"), vec![1., 3.]);
    let Array::Cell(r) = e.array("r").unwrap() else {
        panic!()
    };
    for (i, want) in [1., 2., 3.].into_iter().enumerate() {
        let Array::Numeric(m) = r.get_linear(i).unwrap().as_ref() else {
            panic!()
        };
        assert_eq!(m.data(), &[want]);
    }
    // The shape of the input is kept, and the cell may hold any value.
    run(
        &mut e,
        "q = cellfun(@(x) {[x x]}, {1, 2; 3, 4}); n = size(q);",
    );
    assert_eq!(row(&e, "n"), vec![2., 2.]);
    let Array::Cell(q) = e.array("q").unwrap() else {
        panic!()
    };
    let Array::Numeric(m) = q.get_linear(1).unwrap().as_ref() else {
        panic!()
    };
    assert_eq!(m.data(), &[3., 3.]);
    // A cell that is not 1x1 is still not a scalar, and cells do not mix with numbers.
    let error = plain(&mut e, "arrayfun(@(x) {x, x}, 1:2)");
    assert!(
        error.starts_with("Non-scalar in Uniform output, at index 1"),
        "{error}"
    );
    let error = plain(&mut e, "t = {{1}, 2}; arrayfun(@(k) t{k}, 1:2)");
    assert!(
        error
            .starts_with("Mismatch in type of outputs, at index 2, output 1 (cell versus double)."),
        "{error}"
    );
}

#[test]
fn legacy_cellfun_isreal_follows_matlab_storage_rules() {
    let mut e = Engine::default();
    // c4: char is real, complex(1,0) stays complex, 1+0i is real.
    run(
        &mut e,
        "r = cellfun('isreal', {1, complex(1, 0), 1 + 2i, 'a', 1 + 0i}); c = class(r);",
    );
    assert_eq!(row(&e, "r"), vec![1., 0., 0., 1., 1.]);
    assert_eq!(text(&e, "c"), "logical");
    // The plain function agrees; logicals are real, cells and handles are not.
    run(
        &mut e,
        "a = isreal(1 + 0i); b = isreal(complex(1, 0)); d = isreal(1 + 2i); l = cellfun('isreal', {true, {}, @sin, [], zeros(2)});",
    );
    assert_eq!(
        (number(&e, "a"), number(&e, "b"), number(&e, "d")),
        (1., 0., 0.)
    );
    assert_eq!(row(&e, "l"), vec![1., 0., 0., 1., 1.]);
    // Arithmetic whose imaginary parts all cancel gives a real result, as in MATLAB.
    run(
        &mut e,
        "p = (1 + 2i) * (1 - 2i); s = (3 + 2i) - 2i; k = class(p); rp = isreal(p); m = [1+2i 3] - [2i 0];",
    );
    assert_eq!(number(&e, "p"), 5.);
    assert_eq!(number(&e, "s"), 3.);
    assert_eq!(row(&e, "m"), vec![1., 3.]);
    assert_eq!((text(&e, "k"), number(&e, "rp")), ("double".into(), 1.));
    // A result that keeps a nonzero imaginary part stays complex.
    run(&mut e, "w = (1 + 2i) + 1; t = isreal(w);");
    assert_eq!(number(&e, "t"), 0.);
}

#[test]
fn struct_fields_call_handles_and_capture_snapshots() {
    let mut e = Engine::default();
    run(
        &mut e,
        "s.f=@sin; y=s.f(2); s.n=5; field='n'; f=@(x)s.f(x)+s.(field); old=s; s.f=@cos; s.n=100; field='f'; z=f(2); oldvalue=old.f(2); nested.c={@cos}; w=nested.c{1}(0);",
    );
    assert_eq!(number(&e, "y"), 2_f64.sin());
    assert_eq!(number(&e, "z"), 2_f64.sin() + 5.);
    assert_eq!(number(&e, "oldvalue"), 2_f64.sin());
    assert_eq!(number(&e, "w"), 1.);
    run(
        &mut e,
        "s.factory=@(x)@(y)x+y; chained=s.factory(3)(4); s(2).f=@cos; indexed=s(2).f(0);",
    );
    assert_eq!(number(&e, "chained"), 7.);
    assert_eq!(number(&e, "indexed"), 1.);
    let Array::Struct(old) = e.array("old").unwrap() else {
        panic!()
    };
    let Array::Handle(handle) = old.get(old.field_index("f").unwrap(), 0).unwrap().as_ref() else {
        panic!()
    };
    assert_eq!(handle.name(), Some("sin"));
}

#[test]
fn struct_field_handle_calls_preserve_output_counts_and_errors() {
    let mut e = Engine::default();
    let output = run(
        &mut e,
        "s.f=@pair; s.f(7); [a,b]=s.f(7); s.fail=@error; try\nswitch a\ncase 7\ns.fail('Test:handle','value %d',b);\notherwise\nerror('unexpected');\nend\ncatch err\nmsg=err.message; id=err.identifier;\nend\nfunction [a,b]=pair(x)\nif nargout==0\ndisp('standalone');\nelse\na=x; b=x+1;\nend\nend",
    );
    assert_eq!(output, ["standalone"]);
    assert_eq!(number(&e, "a"), 7.);
    assert_eq!(number(&e, "b"), 8.);
    assert_eq!(text(&e, "msg"), "value 8");
    assert_eq!(text(&e, "id"), "Test:handle");
}

#[test]
fn mapping_struct_inputs_and_outputs_keeps_handles_and_shape() {
    let mut e = Engine::default();
    run(
        &mut e,
        "s=arrayfun(@(x)struct('n',x,'f',@sin),[1 3;2 4]); nums=arrayfun(@(x)x.n,s); vals=arrayfun(@(x)x.f(x.n),s); c=cellfun(@(x)struct('n',x,'f',@cos),{5,6}); cells=arrayfun(@(x)struct('n',x,'f',@sin),1:2,'UniformOutput',false); structs=cellfun(@(x)x,cells); copied=cellfun(@(x)x,cells,'UniformOutput',false); n=structs(2).n; v=c(2).f(0); h=copied{1}; v2=h.f(0);",
    );
    let s = e.array("s").unwrap();
    assert_eq!((s.rows(), s.cols(), s.class()), (2, 2, "struct"));
    assert_eq!(row(&e, "nums"), [1., 2., 3., 4.]);
    assert_eq!(
        row(&e, "vals"),
        [1_f64.sin(), 2_f64.sin(), 3_f64.sin(), 4_f64.sin()]
    );
    assert_eq!(number(&e, "n"), 2.);
    assert_eq!(number(&e, "v"), 1.);
    assert_eq!(number(&e, "v2"), 0.);
    run(
        &mut e,
        "parts={struct('a',1,'b',@sin),struct('b',@sin,'a',2)}; ordered=cellfun(@(x)x,parts); n=ordered(2).a;",
    );
    assert_eq!(number(&e, "n"), 2.);
    assert!(fails(&mut e, "cellfun(@(x)x,{struct('a',1),struct('b',2)})").contains("Field names"));
    assert!(fails(&mut e, "arrayfun(@(x)struct(),1:65537)").contains("65536"));
}

#[test]
fn equality_class_and_whos_cover_structs_and_handles() {
    let mut e = Engine::default();
    run(
        &mut e,
        "h=@sin; f=@(x)x+1; alias=f; other=@(x)x+1; s=struct('f',h,'n',1); t=struct('n',true,'f',@sin); same=isequal(s,t); different=isequal(s,struct('f',@cos,'n',1)); flags=[isequal(h,@sin),isequal(h,@cos),isequal(f,alias),isequal(f,other),isequal({s,f},{t,alias}),isequal(s,h),isequal(NaN,NaN),isequal('a',97),isequal(1,true,complex(1,0))]; cs=class(s); ch=class(s.f);",
    );
    assert_eq!(number(&e, "same"), 1.);
    assert_eq!(number(&e, "different"), 0.);
    assert_eq!(row(&e, "flags"), [1., 0., 1., 0., 1., 0., 0., 1., 1.]);
    assert_eq!(text(&e, "cs"), "struct");
    assert_eq!(text(&e, "ch"), "function_handle");
    let output = run(&mut e, "whos").join("\n");
    assert!(
        output
            .lines()
            .any(|line| line.starts_with("s ") && line.ends_with("struct"))
    );
    assert!(
        output
            .lines()
            .any(|line| line.starts_with("h ") && line.ends_with("function_handle"))
    );
    let display = run(&mut e, "shown=s").join("\n");
    assert!(display.contains("f:"), "{display}");
    assert!(display.contains("@sin"), "{display}");
    assert!(fails(&mut e, "isequal(s)").contains("Not enough input arguments."));
}

#[test]
fn capture_depth_bound_follows_handles_inside_struct_fields() {
    let mut e = Engine::default();
    let error = fails(&mut e, "s.f=@(x)x; for k=1:110\ns.f=@(x)s.f(x);\nend");
    assert!(error.contains("limited to 100 levels"), "{error}");
}

#[test]
fn struct_handle_persistence_rejects_before_replacing_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("combined.mat");
    std::fs::write(&path, b"keep existing file").unwrap();
    let quoted = path
        .to_str()
        .unwrap()
        .replace('\\', "/")
        .replace('\'', "''");
    let mut e = Engine::default();
    run(&mut e, "s.f=@sin; c={s};");
    for version in ["-v6", "-v7", "-v7.3"] {
        for variable in ["s", "c"] {
            let error = fails(
                &mut e,
                &format!("save('{quoted}','{variable}','{version}')"),
            );
            assert!(
                error.contains("saving structs is not supported yet"),
                "{error}"
            );
            assert_eq!(std::fs::read(&path).unwrap(), b"keep existing file");
        }
    }
}
