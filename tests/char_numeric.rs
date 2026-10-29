//! Numeric expectations below come from the task's reference table.
//! Display expectations also have captured reference evidence in fixtures/display
//! (case 49 and P127). Edge cases document supported UTF-16 and size semantics;
//! they are not claimed as new live MATLAB reference evidence.
use matrust::{Array, CharacterRow, Engine};

fn run(source: &str) -> Engine {
    let mut engine = Engine::default();
    engine
        .execute(source)
        .unwrap_or_else(|error| panic!("{source}: {error}"));
    engine
}
fn numeric(engine: &Engine, name: &str, rows: usize, cols: usize, codes: &[f64]) {
    let Array::Numeric(value) = engine.array(name).unwrap() else {
        panic!("{name} must have double storage");
    };
    assert_eq!((value.rows(), value.cols()), (rows, cols), "{name}");
    assert_eq!(value.data(), codes, "{name}");
}
fn character(engine: &Engine, name: &str, rows: usize, cols: usize, codes: &[u16]) -> CharacterRow {
    let Array::Character(value) = engine.array(name).unwrap() else {
        panic!("{name} must have character storage");
    };
    assert_eq!((value.rows(), value.cols()), (rows, cols), "{name}");
    assert_eq!(value.units(), codes, "{name}");
    value
}

#[test]
fn double_of_a_character_row_uses_utf16_codes() {
    let engine = run("a=double('abc'); s='abc'; b=double(s); c=sqrt('d');");
    numeric(&engine, "c", 1, 1, &[10.]);
    for name in ["a", "b"] {
        numeric(&engine, name, 1, 3, &[97., 98., 99.]);
    }
}

#[test]
fn character_arithmetic_returns_double() {
    let engine = run("a='abc'+0; b='a'*2; c=-'a'; d=0+'abc'; s='abc'; e=s+0; f=+'a';");
    for name in ["a", "d", "e"] {
        numeric(&engine, name, 1, 3, &[97., 98., 99.]);
    }
    numeric(&engine, "b", 1, 1, &[194.]);
    numeric(&engine, "c", 1, 1, &[-97.]);
    numeric(&engine, "f", 1, 1, &[97.]);
}

#[test]
fn character_comparisons_are_logical_with_scalar_expansion() {
    let engine = run("a='abc'=='abd'; b='abc'<'b'; s='abc'; c=s~=char([97 98 100]); d=98>s;");
    for (name, expected) in [
        ("a", [1, 1, 0]),
        ("b", [1, 0, 0]),
        ("c", [0, 0, 1]),
        ("d", [1, 0, 0]),
    ] {
        let Array::Logical(value) = engine.array(name).unwrap() else {
            panic!("logical {name}");
        };
        assert_eq!((value.rows(), value.cols()), (1, 3));
        assert_eq!(value.data(), &expected);
    }
}

#[test]
fn elementary_functions_and_extrema_use_double_codes() {
    let engine = run(
        "a=abs('a'); b=mod('a',2); c=max('abc'); d=min('abc'); e=floor('abc'); f=round('abc'); g=ceil('abc'); h=fix('abc'); i=rem('a',2);",
    );
    for name in ["a", "d"] {
        numeric(&engine, name, 1, 1, &[97.]);
    }
    for name in ["b", "i"] {
        numeric(&engine, name, 1, 1, &[1.]);
    }
    numeric(&engine, "c", 1, 1, &[99.]);
    for name in ["e", "f", "g", "h"] {
        numeric(&engine, name, 1, 3, &[97., 98., 99.]);
    }
}

#[test]
fn character_colon_produces_character_rows() {
    let engine = run("a='a':'e'; x='a'; y='e'; b=x:2:y; c=y:-2:x; d=y:x;");
    character(&engine, "a", 1, 5, &[97, 98, 99, 100, 101]);
    character(&engine, "b", 1, 3, &[97, 99, 101]);
    character(&engine, "c", 1, 3, &[101, 99, 97]);
    character(&engine, "d", 1, 0, &[]);
}

#[test]
fn character_rows_stack_and_matrix_blocks_concatenate() {
    let engine = run("a=['ab';'cd']; x='ab'; y='cd'; b=[x;y]; c=[a a]; d=[a;b];");
    for name in ["a", "b"] {
        character(&engine, name, 2, 2, &[97, 99, 98, 100]);
    }
    character(&engine, "c", 2, 4, &[97, 99, 98, 100, 97, 99, 98, 100]);
    character(&engine, "d", 4, 2, &[97, 99, 97, 99, 98, 100, 98, 100]);
}

#[test]
fn unequal_character_widths_fail_without_changing_the_destination() {
    let mut engine = run("a='old';");
    let before = engine.array("a").unwrap();
    let error = engine.execute("a=['ab';'cde'];").unwrap_err();
    assert!(
        error.contains("Dimensions of matrices being concatenated are not consistent."),
        "{error}"
    );
    assert!(before.same_snapshot(&engine.array("a").unwrap()));
    assert!(
        engine
            .execute("a=[char([97;98]) 'c'];")
            .unwrap_err()
            .contains("Dimensions of matrices")
    );
}

#[test]
fn character_matrix_disp_and_echo_print_rows() {
    let mut engine = Engine::default();
    assert_eq!(
        engine.execute("disp(['ab';'cd'])").unwrap().output,
        ["ab\ncd"]
    );
    assert_eq!(
        engine.execute("y=['ab';'cd']").unwrap().output,
        ["\ny =\n\nab\ncd\n"]
    );
}

#[test]
fn double_of_character_matrix_preserves_column_major_shape() {
    let engine = run(
        "a=double(['ab';'cd']); s=['ab';'cd']; b=double(s); c=s+0; d=abs(s); e=floor(s); f=round(s); g=mod(s,2);",
    );
    for name in ["a", "b", "c", "d", "e", "f"] {
        numeric(&engine, name, 2, 2, &[97., 99., 98., 100.]);
    }
    numeric(&engine, "g", 2, 2, &[1., 1., 0., 0.]);
}

#[test]
fn char_of_cell_rows_pads_with_spaces() {
    let engine =
        run("a=char({'a','bcd'}); c={'a','bcd'}; b=char(c); d=char({'a','b';'cd','efg'});");
    for name in ["a", "b"] {
        character(&engine, name, 2, 3, &[97, 98, 32, 99, 32, 100]);
    }
    character(
        &engine,
        "d",
        4,
        3,
        &[97, 99, 98, 101, 32, 100, 32, 102, 32, 32, 32, 103],
    );
}

#[test]
fn strvcat_stacks_and_pads_character_rows() {
    let engine = run(
        "a=strvcat('a','bcd'); b=strvcat({'a','bcd'}); c=strvcat('', 'a', [], 'bcd'); d=strvcat();",
    );
    for name in ["a", "b", "c"] {
        character(&engine, name, 2, 3, &[97, 98, 32, 99, 32, 100]);
    }
    character(&engine, "d", 0, 0, &[]);
    assert!(matrust::session::FUNCTIONS.contains(&"strvcat"));
    let mut fresh = Engine::default();
    assert!(
        fresh
            .execute("help strvcat")
            .unwrap()
            .output
            .join("\n")
            .contains("Syntax: s = strvcat")
    );
    fresh.execute("strvcat('a', 'bcd')").unwrap();
}

#[test]
fn mixed_numeric_concatenation_selects_char_storage() {
    let engine = run("a=[65 'a']; b=['a' 65]; c=[65 66;'cd']; d=[char([97;98]) [65;66]];");
    character(&engine, "a", 1, 2, &[65, 97]);
    character(&engine, "b", 1, 2, &[97, 65]);
    character(&engine, "c", 2, 2, &[65, 99, 66, 100]);
    character(&engine, "d", 2, 2, &[97, 98, 65, 66]);
}

#[test]
fn utf16_units_include_surrogates_and_do_not_mutate_shared_char_storage() {
    let mut engine = run("s=char([0 955 55357 56832 55296]); alias=s;");
    let before = engine.array("s").unwrap();
    engine
        .execute("a=double(s); b=s+0; c=abs(s); d=floor(s); e=s==s; f=max(s); g=double('λ😀');")
        .unwrap();
    for name in ["a", "b", "c", "d"] {
        numeric(&engine, name, 1, 5, &[0., 955., 55357., 56832., 55296.]);
    }
    numeric(&engine, "f", 1, 1, &[56832.]);
    numeric(&engine, "g", 1, 3, &[955., 55357., 56832.]);
    assert!(before.same_snapshot(&engine.array("s").unwrap()));
    assert!(before.same_snapshot(&engine.array("alias").unwrap()));
}

#[test]
fn numeric_conversion_preserves_shaped_character_empties() {
    let engine = run(
        "a=double(''); s=char(zeros(1,0)); b=double(s); c=abs(s); d=double(char(zeros(0,3))); e=[s s]; f=char({'','ab'}); g=''+1; h=s+1;",
    );
    numeric(&engine, "a", 0, 0, &[]);
    numeric(&engine, "b", 1, 0, &[]);
    numeric(&engine, "c", 1, 0, &[]);
    numeric(&engine, "d", 0, 3, &[]);
    numeric(&engine, "g", 0, 0, &[]);
    numeric(&engine, "h", 1, 0, &[]);
    character(&engine, "e", 1, 0, &[]);
    character(&engine, "f", 2, 2, &[32, 97, 32, 98]);
}

#[test]
fn character_range_for_loop_keeps_the_iteration_variable_character() {
    let engine = run("s=''; for c='a':'e'\n s=[s c];\nend\nk=ischar(c);");
    character(&engine, "s", 1, 5, &[97, 98, 99, 100, 101]);
    character(&engine, "c", 1, 1, &[101]);
    assert_eq!(engine.logical_variables["k"].data(), &[1]);
}

#[test]
fn character_operands_work_in_complex_arithmetic_and_linear_solve() {
    let engine = run(
        "a='a'+1i; b=1i+'a'; c=complex('a','b'); d=char([2 0;0 4])\\[6;8]; e='b'./2; f='b'.^2;",
    );
    for name in ["a", "b", "c"] {
        let Array::Complex(z) = engine.array(name).unwrap() else {
            panic!("complex {name}");
        };
        assert_eq!(z.data()[0].re, 97.);
        assert_eq!(z.data()[0].im, if name == "c" { 98. } else { 1. });
    }
    numeric(&engine, "d", 2, 1, &[3., 2.]);
    numeric(&engine, "e", 1, 1, &[49.]);
    numeric(&engine, "f", 1, 1, &[9604.]);
}

#[test]
fn bounds_and_unsupported_inputs_are_rejected_atomically() {
    let mut engine = run("a='original'; x=char(ones(1,32769));");
    let original = engine.array("a").unwrap();
    for source in [
        "a=[x x];",
        "a=[x;x];",
        "a=char({'a',1});",
        "a=strvcat({'a',1});",
        "a=char(0):0.5:char(65535);",
    ] {
        assert!(engine.execute(source).is_err(), "{source}");
        assert!(
            original.same_snapshot(&engine.array("a").unwrap()),
            "{source}"
        );
    }
    for source in ["a={}+1;", "a=struct()+1;", "a=@sin+1;"] {
        let error = engine.execute(source).unwrap_err();
        assert!(
            error.contains("Arithmetic requires numeric or logical operands"),
            "{source}: {error}"
        );
    }
    assert!(
        engine
            .execute("a=1+{};")
            .unwrap_err()
            .contains("Expected a numeric or logical value")
    );
}

#[test]
fn character_colon_subscript_selects_every_element() {
    let engine = run(
        "x=1:5; a=x(':'); y=x; y(':')=0; m=[1 2;3 4]; b=m(':',1); c=':'; d=x(c); z=x; z(':')=[]; s='abc'; e=s(':');",
    );
    numeric(&engine, "a", 5, 1, &[1., 2., 3., 4., 5.]);
    numeric(&engine, "y", 1, 5, &[0.; 5]);
    numeric(&engine, "b", 2, 1, &[1., 3.]);
    numeric(&engine, "d", 5, 1, &[1., 2., 3., 4., 5.]);
    numeric(&engine, "z", 0, 0, &[]);
    character(&engine, "e", 3, 1, &[97, 98, 99]);
}

#[test]
fn transpose_operators_keep_character_class() {
    let engine = run("s='abc'; a=s'; b=s.'; c=('a':'c')'; m=['ab';'cd']; d=m'; e=m.';");
    for name in ["a", "b", "c"] {
        character(&engine, name, 3, 1, &[97, 98, 99]);
    }
    for name in ["d", "e"] {
        character(&engine, name, 2, 2, &[97, 98, 99, 100]);
    }
}

#[test]
fn rearranging_builtins_keep_character_class() {
    let engine = run("m=['ab';'cd']; s='abcd';");
    let cases: &[(&str, (usize, usize), &[u16])] = &[
        ("flip('abc')", (1, 3), &[99, 98, 97]),
        ("flip(m)", (2, 2), &[99, 97, 100, 98]),
        ("flip(m,2)", (2, 2), &[98, 100, 97, 99]),
        ("flipud(m)", (2, 2), &[99, 97, 100, 98]),
        ("fliplr(m)", (2, 2), &[98, 100, 97, 99]),
        ("fliplr('abc')", (1, 3), &[99, 98, 97]),
        ("diag('abc')", (3, 3), &[97, 0, 0, 0, 98, 0, 0, 0, 99]),
        ("diag(m)", (2, 1), &[97, 100]),
        ("tril(m)", (2, 2), &[97, 99, 0, 100]),
        ("triu(m)", (2, 2), &[97, 0, 98, 100]),
        ("transpose('abc')", (3, 1), &[97, 98, 99]),
        ("ctranspose(m)", (2, 2), &[97, 98, 99, 100]),
        ("rot90(m)", (2, 2), &[98, 97, 100, 99]),
        // Reference behavior: a scalar shift without dim moves rows, so a row is unchanged.
        ("circshift(s,1)", (1, 4), &[97, 98, 99, 100]),
        ("circshift(s,1,2)", (1, 4), &[100, 97, 98, 99]),
        ("circshift(m,1)", (2, 2), &[99, 97, 100, 98]),
        ("reshape(s,2,2)", (2, 2), &[97, 98, 99, 100]),
        (
            "repmat('ab',2,2)",
            (2, 4),
            &[97, 97, 98, 98, 97, 97, 98, 98],
        ),
        ("repmat(m,1,2)", (2, 4), &[97, 99, 98, 100, 97, 99, 98, 100]),
    ];
    let mut engine = engine;
    for &(source, (rows, cols), units) in cases {
        engine
            .execute(&format!("r={source};"))
            .unwrap_or_else(|error| panic!("{source}: {error}"));
        let Array::Character(value) = engine.array("r").unwrap() else {
            panic!("{source} must keep character storage");
        };
        assert_eq!((value.rows(), value.cols()), (rows, cols), "{source}");
        assert_eq!(value.units(), units, "{source}");
    }
}

#[test]
fn double_quoted_literals_keep_their_earlier_behaviour() {
    // Pinned to main 756505b: double-quoted text is not a char array here.
    let mut engine = run("a=max(\"abc\");");
    numeric(&engine, "a", 1, 1, &[99.]);
    for (source, message) in [
        ("b=size(\"abc\");", "Expected an array"),
        ("b=\"abc\"==\"abd\";", "Expected an array"),
        ("b=double(\"abc\");", "Expected a numeric or logical value"),
        ("b=[\"ab\" 'c'];", "Expected an array"),
        (
            "b=\"abc\"+0;",
            "Arithmetic requires numeric or logical operands",
        ),
        ("b=abs(\"a\");", "abs requires numeric arguments"),
    ] {
        let error = engine.execute(source).unwrap_err();
        assert!(error.contains(message), "{source}: {error}");
        assert!(engine.array("b").is_none(), "{source}");
    }
}

#[test]
fn char_to_logical_conversion_errors_but_logical_operators_accept_char() {
    let mut engine = run("L=false(1,3); n=~'a b'; o='a'&1; t=0;\nif 'ab'\n t=1;\nend");
    let before = engine.array("L").unwrap();
    for source in ["L=logical('abc');", "L(1)='a';", "L(2)=char(1);"] {
        let error = engine.execute(source).unwrap_err();
        assert!(
            error.contains("Conversion to logical from char is not possible."),
            "{source}: {error}"
        );
        assert!(
            before.same_snapshot(&engine.array("L").unwrap()),
            "{source}"
        );
    }
    let Array::Logical(n) = engine.array("n").unwrap() else {
        panic!("logical n");
    };
    assert_eq!(n.data(), &[0, 0, 0]);
    assert_eq!(engine.logical_variables["o"].data(), &[1]);
    numeric(&engine, "t", 1, 1, &[1.]);
}

#[test]
fn indexed_char_writes_follow_the_destination_class() {
    let mut engine = run("x=1:3; x(2)='a'; z=[1i 2]; z(1)='a';");
    numeric(&engine, "x", 1, 3, &[1., 97., 3.]);
    let Array::Complex(z) = engine.array("z").unwrap() else {
        panic!("complex z");
    };
    assert_eq!((z.data()[0].re, z.data()[0].im), (97., 0.));
    // MATLAB would create a char row; that growth is not implemented, so it
    // must fail rather than silently create a double.
    assert!(engine.execute("w(2)='a';").is_err());
    assert!(engine.array("w").is_none());
}

#[test]
fn fractional_double_concatenated_with_char_truncates() {
    // Reference behavior (checked 2026-10-24): [65.5 'a'] is the char 'Aa'.
    let mut engine = Engine::default();
    engine.execute("a=[65.5 'a']; d=double(a);").unwrap();
    numeric(&engine, "d", 1, 2, &[65., 97.]);
}
