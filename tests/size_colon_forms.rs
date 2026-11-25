//! Size-row constructor forms, colon empty/nonscalar operands, and trailing
//! matrix/cell separators.
//!
//! Expected values and shapes follow:
//! - MathWorks `colon` page: empty operand gives 1x0 empty; nonscalar operand
//!   uses first element `j(1):i(1):k(1)`.
//! - MathWorks `eye`, `ones`, `zeros`, `rand`, `randn`, `randi`, `true`, `false`,
//!   `cell`, `NaN`, `Inf` pages: `f(sz)`, `f(sz, classname)`, `f(sz, 'like', p)`
//!   with `sz` as a row (including 1x1 -> square and empty `[]` / 1x0 -> 0x0);
//!   `eye` rejects >2-D size rows.
//! - MathWorks matrix/cell concatenation & GNU Octave 10.3 (`tool/ref-octave.sh`):
//!   trailing `,` or `;` before `]`/`}` is ignored; `[;]`, `[,]` give 0x0;
//!   `[1,,2]` is an error.

use matrust::{Array, Engine};

fn eval(code: &str) -> Engine {
    let mut engine = Engine::default();
    engine
        .execute(code)
        .unwrap_or_else(|e| panic!("{code}: {e}"));
    engine
}

fn shape(engine: &Engine, name: &str) -> (usize, usize) {
    let a = engine
        .array(name)
        .unwrap_or_else(|| panic!("missing {name}"));
    (a.rows(), a.cols())
}

fn class_name(engine: &Engine, name: &str) -> String {
    engine
        .array(name)
        .unwrap_or_else(|| panic!("missing {name}"))
        .class()
        .to_owned()
}

#[test]
fn corpus_repros_succeed() {
    let e = eval("x = []:[];");
    assert_eq!(shape(&e, "x"), (1, 0));
    assert_eq!(class_name(&e, "x"), "double");

    let e = eval("v = 'abc'; s = []; e = []; r = v(s:e);");
    assert_eq!(shape(&e, "r"), (1, 0));
    assert_eq!(class_name(&e, "r"), "char");

    let e = eval("x = [] + 2; y = x:x;");
    assert_eq!(shape(&e, "y"), (1, 0));
    assert_eq!(class_name(&e, "y"), "double");

    let e = eval("A = ones(2, 3); B = eye(size(A));");
    assert_eq!(shape(&e, "B"), (2, 3));
    assert_eq!(e.variables["B"].data(), &[1., 0., 0., 1., 0., 0.]);

    let e = eval("x = [1 2 3,];");
    assert_eq!(shape(&e, "x"), (1, 3));
    assert_eq!(e.variables["x"].data(), &[1., 2., 3.]);

    eval("axis([0,8,-.02,9,-15,9,]);");
}

#[test]
fn colon_empty_and_nonscalar_all_paths() {
    // Empty operands in expression, colon(), indexing, and for-loop.
    let e = eval(
        "a = []:5; b = 1:[]; c = 1:[]:5; \
         d = colon([], 5); f = colon(1, [], 5); \
         v = [10 20 30]; iv = v([]:2); jv = v(1:[]); \
         n = 0; for k = []:5, n = n + 1; end; \
         m = 0; for k = 1:[]:5, m = m + 1; end",
    );
    for var in ["a", "b", "c", "d", "f", "iv", "jv"] {
        assert_eq!(shape(&e, var), (1, 0), "{var}");
        assert_eq!(class_name(&e, var), "double", "{var}");
    }
    assert_eq!(e.variables["n"].data(), &[0.]);
    assert_eq!(e.variables["m"].data(), &[0.]);

    // Typed and character empty colon operands preserve class.
    let e = eval(
        "ti = int32([]):int32(5); ts = single(1):single([]); \
         tc = 'a':''; td = colon('a', ''); \
         nt = 0; for k = int32([]):int32(5), nt = nt + 1; end",
    );
    assert_eq!(shape(&e, "ti"), (1, 0));
    assert_eq!(class_name(&e, "ti"), "int32");
    assert_eq!(shape(&e, "ts"), (1, 0));
    assert_eq!(class_name(&e, "ts"), "single");
    assert_eq!(shape(&e, "tc"), (1, 0));
    assert_eq!(class_name(&e, "tc"), "char");
    assert_eq!(shape(&e, "td"), (1, 0));
    assert_eq!(class_name(&e, "td"), "char");
    assert_eq!(e.variables["nt"].data(), &[0.]);

    let e = eval("f = colon(int32([]), int32(5)); g = colon(single(1), single([]), single(4));");
    for (var, class) in [("f", "int32"), ("g", "single")] {
        assert_eq!(shape(&e, var), (1, 0));
        assert_eq!(class_name(&e, var), class);
    }

    // Nonscalar operands use their first element.
    let e = eval(
        "a = [1 2]:[4 5]; b = [1 9]:[2 8]:[7 10]; \
         c = colon([2 9], [5 1]); \
         v = [10 20 30 40]; iv = v([2 4]:[3 1]); \
         ti = int32([2 9]):int32([4 1]); \
         s = 0; for k = [1 9]:[3 1], s = s + k; end",
    );
    assert_eq!(e.variables["a"].data(), &[1., 2., 3., 4.]);
    assert_eq!(e.variables["b"].data(), &[1., 3., 5., 7.]);
    assert_eq!(e.variables["c"].data(), &[2., 3., 4., 5.]);
    assert_eq!(e.variables["iv"].data(), &[20., 30.]);
    assert_eq!(shape(&e, "ti"), (1, 3));
    assert_eq!(class_name(&e, "ti"), "int32");
    assert_eq!(e.variables["s"].data(), &[6.]);
}

#[test]
fn constructors_size_row_class_and_like() {
    // 2-element row, 1x1 row, empty [] and 1x0 row.
    for ctor in [
        "eye", "ones", "zeros", "rand", "randn", "true", "false", "cell", "nan", "NaN", "inf",
        "Inf",
    ] {
        let e = eval(&format!(
            "r2 = {ctor}([2 3]); r1 = {ctor}([3]); \
             re = {ctor}([]); r0 = {ctor}(ones(1,0));"
        ));
        assert_eq!(shape(&e, "r2"), (2, 3), "{ctor}([2 3])");
        assert_eq!(shape(&e, "r1"), (3, 3), "{ctor}([3])");
        assert_eq!(shape(&e, "re"), (0, 0), "{ctor}([])");
        assert_eq!(shape(&e, "r0"), (0, 0), "{ctor}(ones(1,0))");
    }

    // randi(imax, sz)
    let e = eval(
        "r2 = randi(9, [2 3]); r1 = randi(9, [3]); \
         re = randi(9, []); r0 = randi(9, ones(1,0)); \
         r3 = randi(9, [2 3 1]);",
    );
    assert_eq!(shape(&e, "r2"), (2, 3));
    assert_eq!(shape(&e, "r1"), (3, 3));
    assert_eq!(shape(&e, "re"), (0, 0));
    assert_eq!(shape(&e, "r0"), (0, 0));
    assert_eq!(shape(&e, "r3"), (2, 3));

    // Classname and 'like' forms.
    for ctor in ["eye", "ones", "zeros"] {
        let e = eval(&format!(
            "a = {ctor}([2 3], 'single'); b = {ctor}([2 3], 'int32'); \
             c = {ctor}([], 'uint8'); d = {ctor}([2 3], 'like', int16(1)); \
             f = {ctor}([3], 'like', single(1));"
        ));
        assert_eq!(shape(&e, "a"), (2, 3));
        assert_eq!(class_name(&e, "a"), "single");
        assert_eq!(class_name(&e, "b"), "int32");
        assert_eq!(shape(&e, "c"), (0, 0));
        assert_eq!(class_name(&e, "c"), "uint8");
        assert_eq!(class_name(&e, "d"), "int16");
        assert_eq!(shape(&e, "f"), (3, 3));
        assert_eq!(class_name(&e, "f"), "single");
    }

    let e = eval(
        "a = randi(10, [2 3], 'int32'); b = randi(10, [], 'single'); \
         c = randi(10, [2 3], 'like', uint16(1));",
    );
    assert_eq!(shape(&e, "a"), (2, 3));
    assert_eq!(class_name(&e, "a"), "int32");
    assert_eq!(shape(&e, "b"), (0, 0));
    assert_eq!(class_name(&e, "b"), "single");
    assert_eq!(shape(&e, "c"), (2, 3));
    assert_eq!(class_name(&e, "c"), "uint16");

    for ctor in ["rand", "randn", "nan", "NaN", "inf", "Inf"] {
        let e = eval(&format!(
            "a = {ctor}([2 3], 'single'); b = {ctor}([], 'single'); \
             c = {ctor}([2 3], 'like', single(1)); \
             d = {ctor}('single'); f = {ctor}('like', single(1));"
        ));
        assert_eq!(shape(&e, "a"), (2, 3));
        assert_eq!(class_name(&e, "a"), "single");
        assert_eq!(shape(&e, "b"), (0, 0));
        assert_eq!(class_name(&e, "b"), "single");
        assert_eq!(shape(&e, "c"), (2, 3));
        assert_eq!(class_name(&e, "c"), "single");
        assert_eq!(shape(&e, "d"), (1, 1));
        assert_eq!(class_name(&e, "d"), "single");
        assert_eq!(shape(&e, "f"), (1, 1));
        assert_eq!(class_name(&e, "f"), "single");
    }

    for ctor in ["true", "false"] {
        let e = eval(&format!(
            "a = {ctor}([2 3], 'logical'); b = {ctor}([], 'like', false); \
             c = {ctor}([2 3], 'like', true);"
        ));
        assert_eq!(shape(&e, "a"), (2, 3));
        assert_eq!(class_name(&e, "a"), "logical");
        assert_eq!(shape(&e, "b"), (0, 0));
        assert_eq!(class_name(&e, "b"), "logical");
        assert_eq!(shape(&e, "c"), (2, 3));
        assert_eq!(class_name(&e, "c"), "logical");
    }

    let e = eval("a = cell([2 3], 'cell'); b = cell([], 'like', {}); c = cell([2 3], 'like', {});");
    assert_eq!(shape(&e, "a"), (2, 3));
    assert_eq!(class_name(&e, "a"), "cell");
    assert_eq!(shape(&e, "b"), (0, 0));
    assert_eq!(class_name(&e, "c"), "cell");
    assert_eq!(shape(&e, "c"), (2, 3));

    // eye rejects >2-D size rows with N-dimensional error.
    let mut e = Engine::default();
    let err = e.execute("eye([2 3 4]);").unwrap_err();
    assert!(err.contains("N-dimensional"), "{err}");
    let err = e.execute("eye([2 3 4], 'single');").unwrap_err();
    assert!(err.contains("N-dimensional"), "{err}");
    let err = e.execute("eye([2 3 1]);").unwrap_err();
    assert!(err.contains("N-dimensional"), "{err}");
}

#[test]
fn matrix_and_cell_trailing_separators() {
    let e = eval(
        "a = [1 2 3,]; b = [1 2;]; c = [1 2; 3 4;]; \
         d = [;]; f = [,]; \
         g = [1, 2, ... \n % trailing comment\n ]; \
         ca = {1, 2,}; cb = {1 2; 3 4;}; cc = {;}; cd = {,};",
    );
    assert_eq!(shape(&e, "a"), (1, 3));
    assert_eq!(e.variables["a"].data(), &[1., 2., 3.]);
    assert_eq!(shape(&e, "b"), (1, 2));
    assert_eq!(e.variables["b"].data(), &[1., 2.]);
    assert_eq!(shape(&e, "c"), (2, 2));
    assert_eq!(e.variables["c"].data(), &[1., 3., 2., 4.]);
    assert_eq!(shape(&e, "d"), (0, 0));
    assert_eq!(shape(&e, "f"), (0, 0));
    assert_eq!(shape(&e, "g"), (1, 2));
    assert_eq!(e.variables["g"].data(), &[1., 2.]);
    assert!(matches!(e.array("ca"), Some(Array::Cell(_))));
    assert_eq!(shape(&e, "ca"), (1, 2));
    assert_eq!(shape(&e, "cb"), (2, 2));
    assert_eq!(shape(&e, "cc"), (0, 0));
    assert_eq!(shape(&e, "cd"), (0, 0));

    let mut e = Engine::default();
    assert!(e.execute("[1,,2]").is_err());
    assert!(e.execute("{1,,2}").is_err());
}

#[test]
fn matrix_and_cell_whitespace_before_paren_or_brace() {
    let e = eval(
        "v = [7 8]; \
         ws_mat = [v (1:2)]; \
         adj_mat = [v(1:2)]; \
         ws_cell = {v (1:2)}; \
         outside = v (2); \
         grouped = [(v (1)) 5]; \
         cont = [v ... \n (1:2)];",
    );
    assert_eq!(shape(&e, "ws_mat"), (1, 4));
    assert_eq!(e.variables["ws_mat"].data(), &[7., 8., 1., 2.]);
    assert_eq!(shape(&e, "adj_mat"), (1, 2));
    assert_eq!(e.variables["adj_mat"].data(), &[7., 8.]);
    assert!(matches!(e.array("ws_cell"), Some(Array::Cell(_))));
    assert_eq!(shape(&e, "ws_cell"), (1, 2));
    assert_eq!(e.variables["outside"].data(), &[8.]);
    assert_eq!(shape(&e, "grouped"), (1, 2));
    assert_eq!(e.variables["grouped"].data(), &[7., 5.]);
    assert_eq!(shape(&e, "cont"), (1, 4));
    assert_eq!(e.variables["cont"].data(), &[7., 8., 1., 2.]);

    let Some(Array::Cell(cells)) = e.array("ws_cell") else {
        panic!("expected cell array");
    };
    assert_eq!(
        cells.get_linear(0).unwrap().as_numeric().unwrap().data(),
        &[7., 8.]
    );
    assert_eq!(
        cells.get_linear(1).unwrap().as_numeric().unwrap().data(),
        &[1., 2.]
    );

    // First red case and explicit-comma control.
    let e = eval(
        "v = []; b = 1; j = 1; d = 3; \
         out_ws = [v (1:b)+(j-1)*d]; \
         out_comma = [v, (1:b)+(j-1)*d];",
    );
    assert_eq!(shape(&e, "out_ws"), (1, 1));
    assert_eq!(e.variables["out_ws"].data(), &[1.]);
    assert_eq!(e.variables["out_comma"].data(), &[1.]);

    // Two-block loop matching apply_BC boundary index construction.
    let e = eval(
        "N = [3 2]; q = 1; \
         b = prod([1 N(1:q-1)]); d = prod([1 N(1:q)]); r = prod(N); \
         top = []; bot = []; \
         for j = 1:r/d, \
             top = [top (1:b)+(j-1)*d]; \
             bot = [bot (d-b+1:d)+(j-1)*d]; \
         end",
    );
    assert_eq!(shape(&e, "top"), (1, 2));
    assert_eq!(e.variables["top"].data(), &[1., 4.]);
    assert_eq!(shape(&e, "bot"), (1, 2));
    assert_eq!(e.variables["bot"].data(), &[3., 6.]);

    // Chained access and call adjacency inside and outside brackets.
    let e = eval(
        "s.f = [10 20]; \
         chained_ws = [s.f (1)]; \
         chained_adj = [s.f(1) (2)]; \
         int_adj = [int64(9007199254740993)];",
    );
    assert_eq!(shape(&e, "chained_ws"), (1, 3));
    assert_eq!(e.variables["chained_ws"].data(), &[10., 20., 1.]);
    assert_eq!(shape(&e, "chained_adj"), (1, 2));
    assert_eq!(e.variables["chained_adj"].data(), &[10., 2.]);
    assert_eq!(shape(&e, "int_adj"), (1, 1));
    assert_eq!(class_name(&e, "int_adj"), "int64");
}
