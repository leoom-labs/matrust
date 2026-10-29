use matrust::{Array, Engine};
use std::sync::atomic::{AtomicBool, Ordering};

fn ok(source: &str) -> (Engine, Vec<String>) {
    let mut e = Engine::default();
    let out = e
        .execute(source)
        .unwrap_or_else(|m| panic!("{source}\n{m}"));
    (e, out.output)
}

fn real(e: &Engine, name: &str) -> f64 {
    e.array(name)
        .unwrap_or_else(|| panic!("{name} is not defined"))
        .scalar(0, 0)
        .unwrap()
        .into_real()
        .unwrap()
}

fn numbers(e: &Engine, name: &str) -> Vec<f64> {
    e.array(name)
        .unwrap_or_else(|| panic!("{name} is not defined"))
        .as_numeric()
        .unwrap()
        .data()
        .to_vec()
}

fn text(e: &Engine, name: &str) -> String {
    match e
        .array(name)
        .unwrap_or_else(|| panic!("{name} is not defined"))
    {
        Array::Character(c) => c.to_utf8(&AtomicBool::new(false)).unwrap(),
        other => panic!("{name} is not char: {}", other.class()),
    }
}

fn fails(source: &str) -> String {
    let mut e = Engine::default();
    e.execute(source)
        .expect_err(&format!("{source} should fail"))
}

#[test]
fn fields_create_read_nest_and_use_dynamic_names() {
    let (e, _) = ok("s.a = 1; s.b = 'x'; s.c.d = 2; s.c.e.f = 'deep';
        name = 'dyn'; s.(name) = 7; t = s.(name); u = s.c.e.f; v = s.c.d + s.a;
        cls = class(s); n = numel(s); [r, c] = size(s); has = isfield(s, 'dyn');
        key = ['d', 'yn']; w = s.(key) * 2;");
    assert_eq!(real(&e, "t"), 7.);
    assert_eq!(text(&e, "u"), "deep");
    assert_eq!(real(&e, "v"), 3.);
    assert_eq!(text(&e, "cls"), "struct");
    assert_eq!((real(&e, "n"), real(&e, "r"), real(&e, "c")), (1., 1., 1.));
    assert_eq!(real(&e, "has"), 1.);
    assert_eq!(real(&e, "w"), 14.);
}

#[test]
fn indexing_into_field_values_reads_and_writes() {
    let (e, _) = ok(
        "m.x = [1 2 3]; m.x(2) = 20; y = m.x(2); z = m.x(end); m.x(end+1) = 4;
        k = numel(m.x); m.c = {1, 2}; m.c{2} = 'q'; w = m.c{2}; q = m.x(2:3);
        m.label = 'abc'; ch = m.label(2); mx = max(m.x);",
    );
    assert_eq!(real(&e, "y"), 20.);
    assert_eq!(real(&e, "z"), 3.);
    assert_eq!(real(&e, "k"), 4.);
    assert_eq!(text(&e, "w"), "q");
    assert_eq!(numbers(&e, "q"), [20., 3.]);
    assert_eq!(text(&e, "ch"), "b");
    assert_eq!(real(&e, "mx"), 20.);
}

#[test]
fn struct_array_grows_with_indexed_field_assignment() {
    let (e, _) = ok(
        "s.a = 1; s(2).a = 5; n = numel(s); [r, c] = size(s); x = s(2).a; y = s(1).a; v = [s.a];
        s(4).a = 9; n4 = numel(s); gap = isempty(s(3).a);
        s(2).b = 'new'; none = isempty(s(1).b); got = s(2).b; s(end+1).a = 6; n5 = numel(s);
        t(3).k = 1; tn = numel(t); tg = isempty(t(1).k); [tr, tc] = size(t);",
    );
    assert_eq!((real(&e, "n"), real(&e, "r"), real(&e, "c")), (2., 1., 2.));
    assert_eq!((real(&e, "x"), real(&e, "y")), (5., 1.));
    assert_eq!((real(&e, "n4"), real(&e, "gap")), (4., 1.));
    assert_eq!(numbers(&e, "v"), [1., 5.]);
    assert_eq!((real(&e, "none"), text(&e, "got")), (1., "new".into()));
    assert_eq!(real(&e, "n5"), 5.);
    assert_eq!((real(&e, "tn"), real(&e, "tg")), (3., 1.));
    assert_eq!((real(&e, "tr"), real(&e, "tc")), (1., 3.));
}

#[test]
fn whole_elements_and_loops_build_struct_arrays() {
    let (e, _) = ok("for i = 1:5; s(i).v = i^2; end; tot = sum([s.v]);
        s(2) = s(5); copied = s(2).v; s(1) = struct('v', 100); first = s(1).v;
        p = s(3); pv = p.v; sub = s(2:3); sn = numel(sub);");
    assert_eq!(real(&e, "tot"), 55.);
    assert_eq!(real(&e, "copied"), 25.);
    assert_eq!(real(&e, "first"), 100.);
    assert_eq!((real(&e, "pv"), real(&e, "sn")), (9., 2.));
    assert!(fails("s.a = 1; s(2) = struct('other', 1);").contains("dissimilar"),);
}

#[test]
fn struct_rows_concatenate_and_elements_delete() {
    let (e, _) = ok(
        "a.v = 1; b.v = 2; c = [a, b, a]; n = numel(c); [r, k] = size(c); w = [c.v];
        c(2) = []; m = numel(c); w2 = [c.v]; d = [c, b]; dn = numel(d);
        d([1 3]) = []; dm = numel(d); only = d.v; e0 = [a]; e0n = numel(e0);",
    );
    assert_eq!((real(&e, "n"), real(&e, "r"), real(&e, "k")), (3., 1., 3.));
    assert_eq!(numbers(&e, "w"), [1., 2., 1.]);
    assert_eq!((real(&e, "m"), numbers(&e, "w2")), (2., vec![1., 1.]));
    assert_eq!(
        (real(&e, "dn"), real(&e, "dm"), real(&e, "only")),
        (3., 1., 1.)
    );
    assert!(fails("a.v = 1; b.w = 2; c = [a, b];").contains("do not match"));
    let (vertical, _) = ok("a.v = 1; c = [a; a]; n=size(c,1);");
    assert_eq!(real(&vertical, "n"), 2.);
    assert!(
        fails("a.v = 1; c = [a, 5];").contains("Conversion to struct from double is not possible")
    );
}

#[test]
fn struct_function_spreads_cell_values_over_elements() {
    let (e, _) = ok(
        "s = struct('a', {1, 2, 3}, 'b', 'x'); n = numel(s); v = [s.a]; bb = s(3).b;
        t = struct('c', {{1, 2}}); k = class(t.c); tn = numel(t);
        em = struct('a', {}); en = numel(em); ee = isempty(em); f = fieldnames(em); fn = numel(f);
        z = struct(); zn = numfields(z); [er, ec] = size(em);",
    );
    assert_eq!(real(&e, "n"), 3.);
    assert_eq!(numbers(&e, "v"), [1., 2., 3.]);
    assert_eq!(text(&e, "bb"), "x");
    assert_eq!((text(&e, "k"), real(&e, "tn")), ("cell".into(), 1.));
    assert_eq!(
        (real(&e, "en"), real(&e, "ee"), real(&e, "fn")),
        (0., 1., 1.)
    );
    assert_eq!((real(&e, "er"), real(&e, "ec")), (0., 0.));
    assert_eq!(real(&e, "zn"), 0.);
    assert!(fails("s = struct('a', {1, 2}, 'b', {1, 2, 3});").contains("dimensions"));
    assert!(fails("s = struct('a');").contains("pairs"));
    assert!(fails("s = struct(1, 2);").contains("field names"));
}

#[test]
fn fieldnames_keep_creation_order_in_a_cell_column() {
    let (e, _) = ok(
        "s.zeta = 1; s.alpha = 2; s.mid = 3; f = fieldnames(s); f1 = f{1}; f2 = f{2};
        f3 = f{3}; [r, c] = size(f); cls = class(f);
        s = rmfield(s, 'alpha'); g = fieldnames(s); gn = numel(g); g2 = g{2};
        o = struct('b', 1, 'a', 2); of = fieldnames(o); o1 = of{1};
        s = setfield(s, 'extra', 9); x = getfield(s, 'extra'); nf = numfields(s);
        n.a.b = 1; n = setfield(n, 'a', 'b', 5); y = getfield(n, 'a', 'b');",
    );
    assert_eq!(
        (text(&e, "f1"), text(&e, "f2"), text(&e, "f3")),
        ("zeta".into(), "alpha".into(), "mid".into())
    );
    assert_eq!((real(&e, "r"), real(&e, "c")), (3., 1.));
    assert_eq!(text(&e, "cls"), "cell");
    assert_eq!((real(&e, "gn"), text(&e, "g2")), (2., "mid".into()));
    assert_eq!(text(&e, "o1"), "b");
    assert_eq!((real(&e, "x"), real(&e, "nf"), real(&e, "y")), (9., 3., 5.));
    assert!(fails("s.a = 1; s = rmfield(s, 'zz');").contains("zz"));
}

#[test]
fn isfield_and_isstruct_accept_names_cells_and_other_values() {
    let (e, _) = ok(
        "s.a = 1; s.b = 2; t1 = isfield(s, 'a'); t2 = isfield(s, 'z');
        t3 = isfield(s, {'a', 'z', 'b'}); t4 = isfield(5, 'a'); t5 = isstruct(s);
        t6 = isstruct(5); t7 = isfield(s, 5); t8 = isstruct({1}); [r, c] = size(t3);",
    );
    assert_eq!((real(&e, "t1"), real(&e, "t2")), (1., 0.));
    match e.array("t3").unwrap() {
        Array::Logical(m) => {
            assert_eq!(m.data(), &[1, 0, 1]);
            assert_eq!((m.rows(), m.cols()), (1, 3));
        }
        other => panic!("{}", other.class()),
    }
    assert_eq!(
        (real(&e, "t4"), real(&e, "t5"), real(&e, "t6")),
        (0., 1., 0.)
    );
    assert_eq!((real(&e, "t7"), real(&e, "t8")), (0., 0.));
}

#[test]
fn class_names_every_value_kind() {
    let (e, _) = ok(
        "a = class(1); b = class(true); c = class('x'); d = class({1}); f = class(struct());
        g = class(1 + 2i); h = class([1 2 3] > 1);",
    );
    for (name, expected) in [
        ("a", "double"),
        ("b", "logical"),
        ("c", "char"),
        ("d", "cell"),
        ("f", "struct"),
        ("g", "double"),
        ("h", "logical"),
    ] {
        assert_eq!(text(&e, name), expected, "{name}");
    }
}

#[test]
fn field_errors_are_explicit() {
    assert!(fails("s.a = 1; y = s.zz;").contains("Reference to non-existent field 'zz'"));
    assert!(fails("x = 5; y = x.a;").contains("non-structure"));
    assert!(fails("x = 5; x.a = 1;").contains("Scalar structure"));
    assert!(fails("s.a = 1; s.('bad name') = 2;").contains("Invalid field name"));
    assert!(fails("s.a = 1; s(2) = []; ").contains("out of range for deletion"));
    assert!(fails("s.c = {1}; s.c{1}.q = 2;").contains("Scalar structure"));
    assert!(fails("s.a = 1; t = s + 1;").contains("numeric"));
}

#[test]
fn assignment_shares_field_storage() {
    let (e, _) = ok("a.big = zeros(1000, 1000); b = a; b.x = 1; c = b; c.big(1) = 5;");
    let pointer = |name: &str| match e.array(name).unwrap() {
        Array::Struct(s) => s
            .get(s.field_index("big").unwrap(), 0)
            .unwrap()
            .as_numeric()
            .unwrap()
            .data()
            .as_ptr(),
        other => panic!("{}", other.class()),
    };
    assert_eq!(pointer("a"), pointer("b"));
    assert_ne!(pointer("a"), pointer("c"));
}

#[test]
fn struct_values_pass_through_functions_by_value() {
    let (e, _) = ok("p.x = 3; q = bump(p); r = q.x; r0 = p.x;
        function s = bump(s); s.x = s.x + 1; end");
    assert_eq!((real(&e, "r"), real(&e, "r0")), (4., 3.));
}

#[test]
fn try_catch_handles_builtin_and_error_function_failures() {
    let (e, _) = ok("r = 0; try; x = undefined_thing + 1; catch; r = 1; end
        try; error('Pkg:sub:id', 'value %d is %s (%5.2f)', 42, 'bad', 3.14159);
        catch err; m = err.message; id = err.identifier; end
        try; error('plain %d'); catch e2; m2 = e2.message; i2 = e2.identifier; end
        a = [1 2 3]; try; a(7); catch e3; i3 = e3.identifier; m3 = e3.message; end
        try; undefined_again; catch e4; i4 = e4.identifier; end
        [sr, sc] = size(err.stack); sc2 = class(err.stack); fl = isfield(err, 'message');
        try; error('Only:id'); catch e5; m5 = e5.message; i5 = e5.identifier; end
        try; error('has space: %s', 'x'); catch e6; m6 = e6.message; i6 = e6.identifier; end
        try; error('Id:x', '100%% done\\n'); catch e7; m7 = e7.message; end");
    assert_eq!(real(&e, "r"), 1.);
    assert_eq!(text(&e, "m"), "value 42 is bad ( 3.14)");
    assert_eq!(text(&e, "id"), "Pkg:sub:id");
    assert_eq!(
        (text(&e, "m2"), text(&e, "i2")),
        ("plain %d".into(), "".into())
    );
    assert_eq!(text(&e, "i3"), "MATLAB:badsubscript");
    assert!(!text(&e, "m3").is_empty());
    assert_eq!(text(&e, "i4"), "MATLAB:UndefinedFunction");
    assert_eq!((real(&e, "sr"), real(&e, "sc")), (0., 1.));
    // Reference behavior: a caught error is an MException object, not a struct, so isfield is false.
    assert_eq!((text(&e, "sc2"), real(&e, "fl")), ("struct".into(), 0.));
    assert_eq!(
        (text(&e, "m5"), text(&e, "i5")),
        (
            "If you specify a message identifier argument, you must specify the message text argument.".into(),
            "MATLAB:error:missingMessageArgument".into()
        )
    );
    assert_eq!(
        (text(&e, "m6"), text(&e, "i6")),
        ("has space: x".into(), "".into())
    );
    assert_eq!(text(&e, "m7"), "100% done\n");
}

#[test]
fn errors_from_user_functions_keep_their_raw_message_and_identifier() {
    let (e, _) = ok(
        "try; r = thrower(3); catch err; m = err.message; id = err.identifier; end
        try; v = reader(); catch err2; m2 = err2.message; end
        after = 1;
        function y = thrower(x); error('Lib:bad', 'bad input %d', x); end
        function y = reader(); a = [1 2 3]; y = a(9); end",
    );
    assert_eq!(text(&e, "m"), "bad input 3");
    assert_eq!(text(&e, "id"), "Lib:bad");
    assert!(!text(&e, "m2").starts_with("In "), "{}", text(&e, "m2"));
    assert_eq!(real(&e, "after"), 1.);
    // A handled failure leaves no stale location behind for the next error.
    let mut e = Engine::default();
    let error = e
        .execute("try; q = thrower(1); catch; end\nx = [1 2];\nx(5)\nfunction y = thrower(x); error('a'); end")
        .unwrap_err();
    assert!(error.starts_with("Line 3"), "{error}");
}

#[test]
fn nested_try_and_rethrow() {
    let (e, _) = ok("log = 0;
        try
          try
            error('inner:id', 'inner');
          catch e1
            log = log + 1;
            error('outer:id', 'from catch: %s', e1.message);
          end
          log = 100;
        catch e2
          log = log + 10;
          m = e2.message; id = e2.identifier;
        end
        try; try; error('Q:r', 'orig'); catch ee; rethrow(ee); end; catch e3; id3 = e3.identifier; m3 = e3.message; end
        es = struct('message', 'custom msg', 'identifier', 'S:t');
        try; error(es); catch e4; m4 = e4.message; i4 = e4.identifier; end
        try, error('one line'), catch e5, m5 = e5.message; end");
    assert_eq!(real(&e, "log"), 11.);
    assert_eq!(text(&e, "m"), "from catch: inner");
    assert_eq!(text(&e, "id"), "outer:id");
    assert_eq!(
        (text(&e, "id3"), text(&e, "m3")),
        ("Q:r".into(), "orig".into())
    );
    assert_eq!(
        (text(&e, "m4"), text(&e, "i4")),
        ("custom msg".into(), "S:t".into())
    );
    assert_eq!(text(&e, "m5"), "one line");
}

#[test]
fn error_inside_catch_propagates_and_stops_the_script() {
    let mut e = Engine::default();
    let error = e
        .execute("x = 1; try; error('a'); catch; error('b:c', 'second %d', 2); end; x = 2;")
        .unwrap_err();
    assert!(error.contains("second 2"), "{error}");
    assert_eq!(real(&e, "x"), 1.);
    let error = e
        .execute("try; y = 1; catch; end; error('Top:level', 'boom %d', 7)")
        .unwrap_err();
    assert!(error.contains("boom 7"), "{error}");
    assert!(e.execute("rethrow(5)").unwrap_err().contains("rethrow"));
    assert!(
        e.execute("error()")
            .unwrap_err()
            .contains("not enough input")
    );
}

#[test]
fn control_flow_passes_through_try_blocks() {
    let (e, _) = ok("total = 0;
        for i = 1:5; try; if i == 3; error('skip'); end; total = total + i; catch; continue; end; end
        last = 0; for i = 1:5; last = i; try; if i == 2; break; end; end; end
        try; z = 1; end");
    assert_eq!(real(&e, "total"), 12.);
    assert_eq!(real(&e, "last"), 2.);
    assert_eq!(real(&e, "z"), 1.);
}

#[test]
fn cancellation_inside_try_still_cancels() {
    let mut e = Engine::default();
    let flag = e.cancellation();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(80));
        flag.store(true, Ordering::Relaxed);
    });
    // Each pass costs far more than the step budget allows within the delay.
    let error = e
        .execute("caught = 0; try; while true; y = sin(linspace(0, 1, 200000)); end; catch; caught = 1; end")
        .unwrap_err();
    canceller.join().unwrap();
    assert!(error.contains("cancel"), "{error}");
    assert_eq!(real(&e, "caught"), 0.);
}

#[test]
fn run_limits_inside_try_are_not_catchable() {
    let mut e = Engine::default();
    let error = e
        .execute("c = 0; try; while true; x = 1; end; catch; c = 1; end")
        .unwrap_err();
    assert!(error.contains("1000000"), "{error}");
    assert_eq!(real(&e, "c"), 0.);
    let error = e
        .execute("c = 0; try; for i = 1:3000; disp(i); end; catch; c = 1; end")
        .unwrap_err();
    assert!(error.contains("output exceeds"), "{error}");
    assert_eq!(real(&e, "c"), 0.);
    let error = e
        .execute("c = 0; try; r = deep(1); catch; c = 1; end\nfunction y = deep(x); y = deep(x + 1); end")
        .unwrap_err();
    assert!(error.contains("exceeds"), "{error}");
    assert_eq!(real(&e, "c"), 0.);
}

#[test]
fn struct_display_matches_command_window_layout() {
    let (_, out) = ok(
        "s.a = 1; s.b = 'hello'; s.c = [1 2 3]; s.d = {1, 2}; s.e.f = 1; s.g = 1.5; s.h = []; s.k = true; s",
    );
    assert_eq!(
        out,
        [
            "\ns = \n\n    a: 1\n    b: 'hello'\n    c: [1 2 3]\n    d: {[1]  [2]}\n    e: [1x1 struct]\n    g: 1.5000\n    h: []\n    k: 1\n"
        ]
    );
    let (_, out) = ok("t(2).a = 5; t(1).b = 2; t");
    assert_eq!(
        out,
        ["\nt = \n\n1x2 struct array with fields:\n\n    a\n    b\n"]
    );
    let (_, out) = ok("z = struct()");
    assert_eq!(out, ["\nz = \n\nstruct with no fields.\n"]);
    let (_, out) = ok("s.a = 1; s.b = 'x'; disp(s)");
    assert_eq!(out, ["    a: 1\n    b: 'x'\n"]);
    let (_, out) = ok("s.a = 1; s.a = 2");
    assert_eq!(out, ["\ns = \n\n    a: 2\n"]);
    let (_, out) = ok("s.a = 1; s.a = 2;");
    assert!(out.is_empty());
    let (_, out) =
        ok("m = [1 2 3]; w.big = zeros(4, 4); w.n = -3; w.p = 2 + 3i; w.s = 1e6 + 0.5; w");
    assert_eq!(
        out,
        [
            "\nw = \n\n    big: [4x4 double]\n      n: -3\n      p: 2.0000 + 3.0000i\n      s: 1.0000e+06\n"
        ]
    );
}

#[test]
fn whos_and_workspace_report_struct_shape_and_class() {
    let (e, out) = ok("s.a = 1; whos");
    assert!(out[0].contains("struct"), "{}", out[0]);
    let array = e.array("s").unwrap();
    assert_eq!(
        (array.rows(), array.cols(), array.class()),
        (1, 1, "struct")
    );
    assert!(array.retained_bytes() > 0);
}

#[test]
fn saving_structs_is_not_supported_yet() {
    let error = fails("s.a = 1; save('structs_errors_unused.mat', 's')");
    assert!(error.contains("not supported yet"), "{error}");
    let error =
        fails("c = {1}; c{1} = 2; s.a = 1; t = {s}; save('structs_errors_unused.mat', 't')");
    assert!(error.contains("struct"), "{error}");
    assert!(!std::path::Path::new("structs_errors_unused.mat").exists());
}

#[test]
fn completion_list_and_help_cover_structs_and_errors() {
    for name in [
        "struct",
        "fieldnames",
        "isfield",
        "isstruct",
        "rmfield",
        "setfield",
        "getfield",
        "numfields",
        "error",
        "rethrow",
        "try",
        "catch",
    ] {
        assert!(matrust::session::FUNCTIONS.contains(&name), "{name}");
    }
    assert!(matrust::engine::HELP.contains("Structs:"));
    assert!(matrust::engine::HELP.contains("try ... catch"));
}

#[test]
fn error_format_arguments_are_counted_before_expansion() {
    let (e, _) = ok(
        "try; error('%d', zeros(1, 70000)); catch e1; m1 = e1.message; end
        try; error('%d %d', zeros(1, 40000), ones(1, 40000)); catch e2; m2 = e2.message; end
        try; error('Id:x', '%d', zeros(1, 70000)); catch e3; m3 = e3.message; end
        try; error('%d', zeros(1, 16000000)); catch e4; m4 = e4.message; end",
    );
    for name in ["m1", "m2", "m3", "m4"] {
        assert_eq!(text(&e, name), "error: more than 65536 format arguments");
    }
}

#[test]
fn char_format_arguments_count_their_units_and_messages_are_measured_first() {
    let (e, _) = ok(
        "a = 'a'; c = a(ones(1, 40000)); z = '\u{1234}'; w = z(ones(1, 25000));
        try; error('%d %d', c, c); catch e1; m1 = e1.message; end
        try; error('Id:x', '%d', c, 'bc', c); catch e2; m2 = e2.message; end
        try; error('%d', c, ones(1, 30000)); catch e3; m3 = e3.message; end
        try; error('%s', c); catch e4; m4 = e4.message; end
        try; error(w); catch e5; m5 = e5.message; end
        try; error('%s', w); catch e6; m6 = e6.message; end
        try; error(struct('message', w)); catch e7; m7 = e7.message; end
        try; rethrow(struct('message', w, 'identifier', '')); catch e8; m8 = e8.message; end
        try; error('%d,', 'ab'); catch e9; m9 = e9.message; end
        try; error('%d', c); catch e10; m10 = e10.message; end
        n4 = numel(m4);",
    );
    for name in ["m1", "m2", "m3"] {
        assert_eq!(text(&e, name), "error: more than 65536 format arguments");
    }
    assert_eq!(real(&e, "n4"), 40000.);
    assert_eq!(text(&e, "m5"), "error: the message exceeds 65536 bytes");
    assert_eq!(
        text(&e, "m6"),
        "error: a format argument exceeds 65536 bytes"
    );
    assert_eq!(text(&e, "m7"), "error: field 'message' exceeds 65536 bytes");
    assert_eq!(
        text(&e, "m8"),
        "rethrow: field 'message' exceeds 65536 bytes"
    );
    // Small char arguments keep supplying one code unit per numeric conversion.
    assert_eq!(text(&e, "m9"), "97,98,");
    assert_eq!(text(&e, "m10"), "error: message exceeds 65536 bytes");
}

#[test]
fn struct_rejects_repeated_field_names_and_other_paths_cannot_create_them() {
    let message = fails("s = struct('a', 1, 'a', 2);");
    assert!(
        message.ends_with(": Duplicate field names are not allowed"),
        "{message}"
    );
    assert!(fails("s = struct('a', 1, 'b', 2, 'a', {1, 2});").contains("Duplicate field names"));
    let (e, _) = ok("s = struct('a', 1, 'A', 2); n1 = numfields(s);
        t.a = 1; t.('a') = 2; t = setfield(t, 'a', 3); t(2).a = 4; n2 = numfields(t);
        u = rmfield(setfield(t(1), 'b', 1), 'b'); n3 = numfields(u); v = [t(1), t(2)];
        n4 = numfields(v);
        p.a = 1; p.b = 2; q.b = 3; q.a = 4; p(2) = q;
        a2 = p(2).a; b2 = p(2).b; names = fieldnames(p); first = names{1}; n5 = numel(names);");
    for name in ["n1", "n2", "n3", "n4"] {
        let expected = if name == "n1" { 2. } else { 1. };
        assert_eq!(real(&e, name), expected, "{name}");
    }
    // s(2) = t with the same fields in another order is copied by name.
    assert_eq!((real(&e, "a2"), real(&e, "b2")), (4., 3.));
    assert_eq!((text(&e, "first"), real(&e, "n5")), ("a".into(), 2.));
}

#[test]
fn struct_slot_bound_is_checked_before_allocation() {
    let (e, _) = ok("s.a = 1; s.b = 2; t = s(ones(1, 30000)); n = numel(t);
        try; u = s(ones(1, 40000)); catch e1; m1 = e1.message; end
        c = {1}; big = c(ones(1, 40000));
        try; v = struct('a', big, 'b', big); catch e2; m2 = e2.message; end");
    assert_eq!(real(&e, "n"), 30000.);
    assert_eq!(text(&e, "m1"), "Struct array exceeds 65536 slots");
    assert_eq!(text(&e, "m2"), "Struct array exceeds 65536 slots");
}

#[test]
fn setfield_paths_deeper_than_nesting_bound_are_rejected_up_front() {
    let (e, _) = ok("n = {'a'}; p16 = n(ones(1, 16)); p17 = n(ones(1, 17));
        t = setfield(struct(), p16{:}, 5); v = getfield(t, p16{:});
        try; setfield(struct(), p17{:}, 1); catch e1; m1 = e1.message; end");
    assert_eq!(real(&e, "v"), 5.);
    assert_eq!(
        text(&e, "m1"),
        "setfield: field path exceeds 16 nesting levels"
    );
    // A path long enough to exhaust the native stack if it recursed.
    let message = fails("n = {'a'}; p = n(ones(1, 50000)); s = setfield(struct(), p{:}, 1);");
    assert!(
        message.contains("setfield: field path exceeds 16 nesting levels"),
        "{message}"
    );
}

// Identifiers from the fix-round-3 reference findings (rows 19-23).
// Message parity is intentionally limited to the two requested trailing periods.
#[test]
fn builtin_failures_carry_matlab_identifiers_at_the_failure_site() {
    for (source, identifier, message) in [
        (
            "missing_function()",
            "MATLAB:UndefinedFunction",
            "Undefined function or variable 'missing_function'.",
        ),
        (
            "missing_variable",
            "MATLAB:UndefinedFunction",
            "Undefined function or variable 'missing_variable'.",
        ),
        (
            "[1 2]*[3 4]",
            "MATLAB:innerdim",
            "Cannot multiply 1x2 by 1x2",
        ),
        (
            "[1i 2]*[3 4]",
            "MATLAB:innerdim",
            "Complex matrix product inner dimensions must agree",
        ),
        (
            "s=struct('a',1); s.missing",
            "MATLAB:nonExistentField",
            "Reference to non-existent field 'missing'.",
        ),
        (
            "s=struct('a',1); s.('missing')",
            "MATLAB:nonExistentField",
            "Reference to non-existent field 'missing'.",
        ),
        (
            "s=struct('a',1); getfield(s,'missing')",
            "MATLAB:nonExistentField",
            "Reference to non-existent field 'missing'.",
        ),
        (
            "a=[1 2]; a(3)",
            "MATLAB:badsubscript",
            "Index must be an integer between 1 and 2",
        ),
        (
            "a=[1 2]; a(1:3)",
            "MATLAB:badsubscript",
            "Index must be an integer between 1 and 2",
        ),
        (
            "a=[1 2]; a([false false true])",
            "MATLAB:badsubscript",
            "Logical index contains a true value outside the array bounds",
        ),
        (
            "s=struct('a',1); s(2)=struct('b',2)",
            "MATLAB:heterogeneousStrucAssignment",
            "Subscripted assignment between dissimilar structures.",
        ),
    ] {
        for script in [
            format!("try; {source}; catch e; id=e.identifier; msg=e.message; end"),
            format!(
                "try; outer(); catch e; id=e.identifier; msg=e.message; end\nfunction outer(); inner(); end\nfunction inner(); {source}; end"
            ),
            format!(
                "try; try; {source}; catch e; rethrow(e); end; catch e; id=e.identifier; msg=e.message; end"
            ),
        ] {
            let (e, _) = ok(&script);
            assert_eq!(text(&e, "id"), identifier, "{script}");
            assert_eq!(text(&e, "msg"), message, "{script}");
        }
    }
    // Identifiers must never be inferred from user message text or a prior error.
    let (e, _) = ok("try; missing_function(); catch; end
        try; error('Undefined variable or function ''fake'''); catch e; id=e.identifier; end
        try; error('Custom:id','Cannot multiply 1x2 by 1x2'); catch e; custom=e.identifier; end
        try; sqrt(); catch e; plain=e.identifier; end
        try; [1i,2]*struct('x',{1,2}); catch e; unsupported=e.identifier; end");
    assert_eq!(text(&e, "id"), "");
    assert_eq!(text(&e, "custom"), "Custom:id");
    assert_eq!(text(&e, "plain"), "");
    assert_eq!(text(&e, "unsupported"), "");
}

#[test]
fn single_argument_error_distinguishes_identifiers_from_plain_messages() {
    for (argument, identifier, message) in [
        (
            "only:id",
            "MATLAB:error:missingMessageArgument",
            "If you specify a message identifier argument, you must specify the message text argument.",
        ),
        ("only id", "", "only id"),
        ("a:b c", "", "a:b c"),
    ] {
        let (e, _) = ok(&format!(
            "arg='{argument}'; try; error(arg); catch e; id=e.identifier; msg=e.message; end"
        ));
        assert_eq!(text(&e, "id"), identifier);
        assert_eq!(text(&e, "msg"), message);
    }
}

#[test]
fn numeric_concatenation_ignores_zero_by_zero_operands() {
    // Exercise the public numerical path as well as the engine's type dispatch.
    let one = matrust::Matrix::scalar(1.);
    let three = matrust::Matrix::scalar(3.);
    let empty = matrust::Matrix::filled(0, 0, 0.).unwrap();
    for (blocks, shape) in [
        (
            vec![vec![one.clone(), empty.clone(), three.clone()]],
            (1, 2),
        ),
        (vec![vec![one], vec![empty], vec![three]], (2, 1)),
    ] {
        let matrix = matrust::Matrix::concatenate(blocks).unwrap();
        assert_eq!((matrix.rows(), matrix.cols()), shape);
        assert_eq!(matrix.data(), [1., 3.]);
    }
    let (e, _) = ok("a=[]; h=[1,[],3]; v=[1;[];3]; hv=[1,a,3]; vv=[1;a;3];
        u=struct('a',{1,[],3}); fields=[u.a]; empty=[[],a;[]];
        edges=[[],1,3,[]]; emptyrows=[[];1;[];3;[]]; nested=[1,[[],[]],3];
        shaped=[zeros(0,2),[]]; shapedv=[zeros(2,0);[]];
        chars=['a',[],'b']; structs=[u,[]]; ns=numel(structs);");
    for name in ["h", "hv", "fields", "edges", "nested"] {
        let array = e.array(name).unwrap();
        assert_eq!((array.rows(), array.cols()), (1, 2), "{name}");
        assert_eq!(numbers(&e, name), [1., 3.], "{name}");
    }
    for name in ["v", "vv", "emptyrows"] {
        let array = e.array(name).unwrap();
        assert_eq!((array.rows(), array.cols()), (2, 1), "{name}");
        assert_eq!(numbers(&e, name), [1., 3.], "{name}");
    }
    for (name, expected) in [("empty", (0, 0)), ("shaped", (0, 2)), ("shapedv", (2, 0))] {
        let array = e.array(name).unwrap();
        assert_eq!((array.rows(), array.cols()), expected);
    }
    assert!(fails("[1,zeros(0,2),3]").contains("matching row counts"));
    assert_eq!(text(&e, "chars"), "ab");
    assert_eq!(real(&e, "ns"), 3.);
}

#[test]
fn bracket_character_literals_can_follow_a_field_with_whitespace() {
    let (e, _) = ok("try; error('boom'); catch e; msg=['[' e.message ']']; end
        literals=['a' 'b' 'c']; s.x=[1;2]; t=[s.x' s.x'];
        c={s.x 'tail'}; tail=c{2}; p=[(s.x ')];");
    assert_eq!(text(&e, "msg"), "[boom]");
    assert_eq!(text(&e, "literals"), "abc");
    assert_eq!(numbers(&e, "t"), [1., 2., 1., 2.]);
    assert_eq!(text(&e, "tail"), "tail");
    assert_eq!(numbers(&e, "p"), [1., 2.]);
}

#[test]
fn struct_displays_align_names_and_leave_no_fields_unindented() {
    let (_, out) = ok("s=struct('a',1,'dd',5); s; disp(s); s.a=2");
    assert_eq!(
        out,
        ["     a: 1\n    dd: 5\n", "\ns = \n\n     a: 2\n    dd: 5\n"]
    );
    let (_, out) = ok("s=struct('a',1,'dd',5)");
    assert_eq!(out, ["\ns = \n\n     a: 1\n    dd: 5\n"]);
    let (_, out) = ok("disp(struct()); z=struct()");
    assert_eq!(
        out,
        [
            "struct with no fields.",
            "\nz = \n\nstruct with no fields.\n"
        ]
    );
    let (_, out) = ok("s=struct('a',{1,2},'dd',5); disp(s); s");
    assert_eq!(
        out,
        [
            "1x2 struct array with fields:\n\n     a\n    dd\n",
            "\ns = \n\n1x2 struct array with fields:\n\n     a\n    dd\n"
        ]
    );
    let (_, out) = ok("s=struct('a',{},'dd',{}); s");
    assert_eq!(
        out,
        ["\ns = \n\n0x0 struct array with fields:\n\n     a\n    dd\n"]
    );
}

// Reference behavior: "if the message is empty, error takes no action". It is a no-op
// statement: nothing is raised, `ans` is not assigned and nothing is shown.
#[test]
fn error_with_an_empty_message_takes_no_action() {
    for call in [
        "error('')",
        "error('%s','')",
        "error('id:x','')",
        "error('id:x','%s','')",
        "error(struct('message','','identifier',''))",
        "m=''; error(m)",
    ] {
        for terminator in [";", ""] {
            let (e, out) = ok(&format!("before=1; {call}{terminator}\nafter=2;"));
            assert_eq!(
                (real(&e, "before"), real(&e, "after")),
                (1., 2.),
                "{call}{terminator}"
            );
            assert!(e.array("ans").is_none(), "{call}{terminator} assigned ans");
            assert!(out.is_empty(), "{call}{terminator} displayed {out:?}");
        }
        let (e, out) = ok(&format!(
            "try; {call}; x=1; catch; x=2; end
            try; try; {call}; y=1; catch inner; y=2; end; catch outer; y=3; end
            check(''); run();
            function check(msg); error(msg); end
            function run(); {call}; z=1; end"
        ));
        assert_eq!((real(&e, "x"), real(&e, "y")), (1., 1.), "{call}");
        assert!(e.array("ans").is_none() && out.is_empty(), "{call}");
    }
    // The common check pattern: only a non-empty message stops the script.
    let (e, _) = ok("msg=''; check(msg); a=1;
        try; check('bad'); a=2; catch err; a=3; text=err.message; end
        function check(msg); error(msg); end");
    assert_eq!(real(&e, "a"), 3.);
    assert_eq!(text(&e, "text"), "bad");
    // A formatted message that turns out empty is still empty; a non-empty
    // one raises as before, and so does an identifier without message text.
    let (e, _) = ok("try; error('id:x','%s%s','',''); x=1; catch; x=2; end
        try; error(' '); y=1; catch; y=2; end
        try; error('only:id'); z=1; catch; z=2; end
        try; error('%d', 0); w=1; catch; w=2; end");
    assert_eq!(
        (real(&e, "x"), real(&e, "y"), real(&e, "z"), real(&e, "w")),
        (1., 2., 2., 2.)
    );
    // rethrow is unchanged: it always raises what it is given.
    let (e, _) = ok("try; rethrow(struct('message','','identifier','')); x=1; catch; x=2; end");
    assert_eq!(real(&e, "x"), 2.);
    // An uncaught empty error() is skipped; the next real one still stops the run.
    assert!(fails("error(''); error('stop here')").contains("stop here"));
}
