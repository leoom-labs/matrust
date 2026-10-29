//! Matrix-building functions: expected values and structural checks.
use matrust::Engine;

fn show_after(setup: &str, expression: &str) -> String {
    let mut engine = Engine::default();
    let source = format!("{setup}\ndisp(mat2str({expression}))");
    engine
        .execute(&source)
        .unwrap_or_else(|e| panic!("{source}: {e}"))
        .output
        .join(" ")
}
fn show(expression: &str) -> String {
    show_after("", expression)
}
fn error(source: &str) -> String {
    Engine::default().execute(source).unwrap_err().to_string()
}

#[test]
fn registered_in_function_list() {
    for name in [
        "cat",
        "horzcat",
        "vertcat",
        "rot90",
        "squeeze",
        "sub2ind",
        "ind2sub",
        "magic",
        "hilb",
        "invhilb",
        "pascal",
        "toeplitz",
        "vander",
        "hadamard",
        "compan",
        "rosser",
        "wilkinson",
        "randperm",
        "accumarray",
        "isequal",
        "cummax",
        "cummin",
        "colon",
        "flintmax",
        "realmax",
        "realmin",
    ] {
        assert!(matrust::session::FUNCTIONS.contains(&name), "{name}");
    }
}

#[test]
fn concatenation_functions() {
    assert_eq!(show("cat(1,[1 2],[3 4])"), "[1 2;3 4]");
    assert_eq!(show("cat(2,[1 2],[3 4])"), "[1 2 3 4]");
    assert_eq!(show("horzcat([1;2],[3;4])"), "[1 3;2 4]");
    assert_eq!(show("vertcat([1 2],[3 4],[5 6])"), "[1 2;3 4;5 6]");
    assert_eq!(show("horzcat([],[1 2])"), "[1 2]");
    assert_eq!(show("horzcat()"), "[]");
    assert_eq!(show("cat(2,true,false)"), "[true false]");
    assert_eq!(show("horzcat('ab','cd')"), "'abcd'");
    assert_eq!(show("cat(3,[1 2])"), "[1 2]");
    // character matrices and numbers joined to text (reference: 'aB', ['ab';'cd'])
    assert_eq!(show("cat(2,'a',66)"), "'aB'");
    assert_eq!(show("class(horzcat('a',66))"), "'char'");
    assert_eq!(show("vertcat('ab','cd')"), "['ab';'cd']");
    assert_eq!(show("cat(1,'ab','cd')"), "['ab';'cd']");
    assert_eq!(show("size(vertcat('ab','cd','ef'))"), "[3 2]");
    assert_eq!(show("vertcat('','ab')"), "'ab'");
    // Reference behavior: one text for every shape mismatch of the function forms
    let mismatch = "Dimensions of matrices being concatenated are not consistent.";
    assert!(error("x = vertcat([1 2],[3]);").contains(mismatch));
    assert!(error("x = horzcat([1;2],[3]);").contains(mismatch));
    assert!(error("x = cat(2,[1;2],[3 4]);").contains(mismatch));
    assert!(error("x = vertcat('ab','c');").contains(mismatch));
    assert!(error("x = cat(0,1,2);").contains("Dimension must be a real positive finite integer."));
    assert!(error("x = cat(1.5,1,2);").contains("real positive finite integer"));
    assert!(error("x = cat();").contains("Not enough input arguments."));
    assert!(error("x = cat(3,1,2);").contains("N-dimensional"));
}

#[test]
fn concatenation_of_cells() {
    let shape = |setup: &str| show_after(setup, "size(c)");
    assert_eq!(shape("c = horzcat({1},{2});"), "[1 2]");
    assert_eq!(show_after("c = horzcat({1},{2});", "c{2}"), "2");
    assert_eq!(shape("c = cat(1,{1},{2});"), "[2 1]");
    assert_eq!(show_after("c = cat(1,{1},{'b'});", "c{2}"), "'b'");
    // column-major order across parts
    let stacked = "c = vertcat({1,2},{3,4});";
    assert_eq!(shape(stacked), "[2 2]");
    assert_eq!(show_after(stacked, "[c{1,2} c{2,1}]"), "[2 3]");
    let side = "c = horzcat({1;2},{3;4});";
    assert_eq!(show_after(side, "[c{1,2} c{2,2}]"), "[3 4]");
    // non-cell parts become one cell each (MATLAB doc: [100, {200, 300}, 'MATLAB'])
    let mixed = "c = horzcat(100,{200,300},'MATLAB');";
    assert_eq!(shape(mixed), "[1 4]");
    assert_eq!(show_after(mixed, "c{1}"), "100");
    assert_eq!(show_after(mixed, "c{4}"), "'MATLAB'");
    assert_eq!(shape("c = horzcat([],{1},{});"), "[1 1]");
    assert_eq!(
        show_after("c = horzcat({},{});", "[iscell(c) isempty(c)]"),
        "[true true]"
    );
    // an empty char is dropped like [] (reference: size(horzcat({1},'')) is [1 1])
    assert_eq!(shape("c = horzcat({1},'');"), "[1 1]");
    let mismatch = "Dimensions of matrices being concatenated are not consistent.";
    assert!(error("x = vertcat({1,2},{3});").contains(mismatch));
    assert!(error("x = horzcat({1;2},{3});").contains(mismatch));
}

#[test]
fn concatenation_skips_every_empty_shape() {
    // Reference cases from the round-3 review, including an otherwise N-D cat.
    for (expression, expected) in [
        ("horzcat(zeros(1,0),[1;2])", "[1;2]"),
        ("vertcat(zeros(0,3),[1 2])", "[1 2]"),
        ("horzcat([1 2],zeros(0,3))", "[1 2]"),
        ("vertcat([1;2],zeros(1,0))", "[1;2]"),
        ("cat(3,[1 2],[])", "[1 2]"),
        ("cat(3,zeros(0,3),[1 2],zeros(1,0))", "[1 2]"),
        ("cat(1,zeros(0,3),[1 2])", "[1 2]"),
        ("cat(2,zeros(1,0),[1;2])", "[1;2]"),
        ("horzcat(false(1,0),[true;false])", "[true;false]"),
        ("horzcat(complex(zeros(1,0)),[1;2])", "[1;2]"),
        ("class(horzcat([1 2],''))", "'char'"),
        ("vertcat(zeros(1,0),zeros(1,0))", "zeros(2,0)"),
    ] {
        assert_eq!(show(expression), expected, "{expression}");
    }
    assert_eq!(
        show_after("c = horzcat(cell(1,0),{1;2});", "[size(c) c{1} c{2}]"),
        "[2 1 1 2]"
    );
    assert_eq!(
        show_after("c = vertcat({1,2},cell(0,3));", "[size(c) c{1} c{2}]"),
        "[1 2 1 2]"
    );
}

#[test]
fn concatenation_of_mismatched_empties() {
    // rev4: when every part is empty the shapes are still checked, with
    // the reference's text (not the engine's "[A,B]" literal messages)
    let mismatch = "Dimensions of matrices being concatenated are not consistent.";
    for source in [
        "x = horzcat(zeros(2,0),zeros(3,0));",
        "x = vertcat(zeros(0,2),zeros(0,3));",
        "x = cat(2,zeros(2,0),zeros(3,0));",
        "x = cat(1,zeros(0,2),zeros(0,3));",
        "x = horzcat([],zeros(2,0),zeros(3,0));",
        "x = horzcat(false(2,0),false(3,0));",
        "x = vertcat(false(0,2),false(0,3));",
        "x = horzcat(complex(zeros(2,0)),complex(zeros(3,0)));",
        "x = vertcat(complex(zeros(0,2)),complex(zeros(0,3)));",
        "x = vertcat(char(zeros(0,2)),char(zeros(0,3)));",
    ] {
        assert!(error(source).contains(mismatch), "{source}");
    }
    // matching empties still join, and empties beside a nonempty part vanish
    assert_eq!(show("size(horzcat(zeros(2,0),zeros(2,0)))"), "[2 0]");
    assert_eq!(show("size(horzcat(zeros(0,2),zeros(0,3)))"), "[0 5]");
    assert_eq!(show("size(vertcat(false(0,2),false(0,2)))"), "[0 2]");
    assert_eq!(show("horzcat(zeros(2,0),[1;2],zeros(2,0))"), "[1;2]");
}

#[test]
fn character_matrices_reach_the_functions() {
    // rev4: a char matrix (not a row) failed text conversion before the call;
    // results stay char, as in the reference (vertcat('ab','cd'), rot90('abc'))
    let x = "x = vertcat('ab','cd');";
    for (expression, expected) in [
        ("isequal(x,x)", "true"),
        ("isequal(x,vertcat('ab','ce'))", "false"),
        ("isequal(x,[97 98;99 100])", "true"),
        ("isequal(x,'abcd')", "false"),
        ("rot90(x)", "['bd';'ac']"),
        ("rot90(x,2)", "['dc';'ba']"),
        ("squeeze(x)", "['ab';'cd']"),
        ("cat(1,x,'ef')", "['ab';'cd';'ef']"),
        ("cat(2,x,x)", "['abab';'cdcd']"),
        ("cat(3,x)", "['ab';'cd']"),
        ("horzcat(x,vertcat('e','f'))", "['abe';'cdf']"),
        ("vertcat(x,'ef',[103 104])", "['ab';'cd';'ef';'gh']"),
        ("class(rot90(x))", "'char'"),
        ("class(squeeze(x))", "'char'"),
        ("class(horzcat(x,x))", "'char'"),
        ("size(horzcat({1},x))", "[1 2]"),
    ] {
        assert_eq!(show_after(x, expression), expected, "{expression}");
    }
    assert_eq!(
        show_after(&format!("{x} c = horzcat({{1}},x);"), "c{2}"),
        "['ab';'cd']"
    );
    assert!(error(&format!("{x} y = vertcat(x,'e');")).contains("not consistent"));
    // the multi-output forms take char matrices too (codes as numbers)
    assert_eq!(
        show_after("x = vertcat('a','b'); [r,c] = ind2sub([10 10],x);", "[r c]"),
        "[7 10;8 10]"
    );
    for source in ["m = cummax(x);", "[m,i] = cummax(x);", "m = cummin(x);"] {
        assert!(
            error(&format!("{x} {source}")).contains("supports only numerical or logical inputs."),
            "{source}"
        );
    }
}

#[test]
fn rot90_and_squeeze() {
    assert_eq!(show("rot90([1 2;3 4])"), "[2 4;1 3]");
    assert_eq!(show("rot90([1 2 3;4 5 6],2)"), "[6 5 4;3 2 1]");
    assert_eq!(show("rot90([1 2 3;4 5 6],3)"), "[4 1;5 2;6 3]");
    assert_eq!(show("rot90([1 2 3;4 5 6],-1)"), "[4 1;5 2;6 3]");
    assert_eq!(show("rot90([1 2 3;4 5 6],4)"), "[1 2 3;4 5 6]");
    assert_eq!(show("rot90(zeros(0,3))"), "zeros(3,0)");
    assert_eq!(show("rot90([true false])"), "[false;true]");
    assert_eq!(show("rot90('abc',2)"), "'cba'");
    assert_eq!(show("rot90('abc')"), "['c';'b';'a']");
    assert_eq!(show("class(rot90('a'))"), "'char'");
    assert_eq!(show("rot90([1+2i 3])"), "[3+0i;1+2i]");
    assert_eq!(show("squeeze([1 2;3 4])"), "[1 2;3 4]");
    assert!(error("x = rot90([1 2],1.5);").contains("k must be an integer."));
    assert!(error("x = rot90([1 2],[1 2]);").contains("k must be a scalar."));
    assert!(error("x = rot90();").contains("Not enough input arguments."));
    assert!(error("x = squeeze(1,2);").contains("Too many input arguments."));
    let cell = "c = rot90({1,'a';2,'b'}); n = size(c);";
    assert_eq!(show_after(cell, "n"), "[2 2]");
    assert_eq!(show_after(cell, "c{1,1}"), "'a'");
    assert_eq!(show_after(cell, "c{2,1}"), "1");
}

#[test]
fn index_conversion() {
    assert_eq!(show("sub2ind([3 4],2,3)"), "8");
    assert_eq!(show("sub2ind([3 4],[1 2 3],[1 2 4])"), "[1 5 12]");
    assert_eq!(show("sub2ind([3 4 2],1,1,2)"), "13");
    assert_eq!(show("sub2ind([3 4],5)"), "5");
    assert_eq!(show("sub2ind([3 4],[1 2;3 1],[1 2;3 4])"), "[1 5;9 10]");
    assert!(error("x = sub2ind([3 4],4,1);").contains("Out of range subscript."));
    assert!(error("x = sub2ind([3 4],0,1);").contains("Out of range subscript."));
    assert!(
        error("x = sub2ind([3 4],[1 2],[1 2 3]);")
            .contains("The subscript vectors must all be of the same size.")
    );
    assert!(error("x = sub2ind(5,3);").contains("Size vector must have at least 2 elements."));
    // Reference quirks: only v < 1 and v > dim are rejected
    assert_eq!(show("sub2ind([3 4],1.5,1)"), "1.5");
    assert_eq!(show("sub2ind([3 4],NaN,1)"), "NaN");
    assert_eq!(show("sub2ind([3;4],1,2)"), "4");
    assert!(error("x = sub2ind([3 4]);").contains("Subscript indices must either be"));
    assert!(error("x = sub2ind([3 4],13);").contains("Out of range subscript."));
    assert_eq!(show("ind2sub([3 4],5)"), "5");
    assert_eq!(
        show_after("[r,c]=ind2sub([3 4],[5 7 12]);", "[r;c]"),
        "[2 1 3;2 3 4]"
    );
    assert_eq!(
        show_after("[r,c,d]=ind2sub([3 4],[5 12]);", "[r;c;d]"),
        "[2 3;2 4;1 1]"
    );
    assert_eq!(
        show_after("[r,c]=ind2sub([3 4 2],[5 20]);", "[r;c]"),
        "[2 2;2 7]"
    );
    assert_eq!(
        show_after("[r,c]=ind2sub([3 4],[1 2;3 4]);", "r"),
        "[1 2;3 1]"
    );
    // The reference has no range check: the last output absorbs the overflow
    assert_eq!(show_after("[r,c] = ind2sub([3 4],13);", "[r c]"), "[1 5]");
    assert_eq!(show("ind2sub([3 4],13)"), "13");
    assert_eq!(show("ind2sub(5,3)"), "3");
    assert_eq!(show_after("[r,c] = ind2sub(12,5);", "[r c]"), "[5 1]");
    assert_eq!(show_after("[r,c] = ind2sub([3;4],5);", "[r c]"), "[2 2]");
    assert_eq!(
        show_after("[r,c,d] = ind2sub([3 4],5);", "[r c d]"),
        "[2 2 1]"
    );
    // 9 outputs of 2^22 elements exceed the statement budget: refused up front
    assert!(
        error("x = ones(1,2^22); [a,b,c,d,e,f,g,h,k] = ind2sub([2 2],x);")
            .contains("evaluation budget")
    );
}

#[test]
fn magic_squares_match_matlab() {
    assert_eq!(show("magic(1)"), "1");
    assert_eq!(show("magic(2)"), "[1 3;4 2]");
    assert_eq!(show("magic(2.5)"), "[1 3;4 2]");
    assert!(error("x = magic(NaN);").contains("Subscript indices must either be"));
    assert!(error("x = magic(Inf);").contains("Maximum variable size"));
    assert_eq!(show("magic(3)"), "[8 1 6;3 5 7;4 9 2]");
    assert_eq!(show("magic(4)"), "[16 2 3 13;5 11 10 8;9 7 6 12;4 14 15 1]");
    assert_eq!(
        show("magic(6)"),
        "[35 1 6 26 19 24;3 32 7 21 23 25;31 9 2 22 27 20;8 28 33 17 10 15;30 5 34 12 14 16;4 36 29 13 18 11]"
    );
    assert_eq!(
        show_after("m = magic(8);", "m(1,:)"),
        "[64 2 3 61 60 6 7 57]"
    );
    assert_eq!(show("magic(0)"), "[]");
    assert_eq!(show("magic(-1)"), "[]");
    assert!(error("x = magic();").contains("Not enough input arguments."));
    assert!(error("x = magic(3,4);").contains("Too many input arguments."));
    // all three constructions are magic: equal row, column and diagonal sums
    for n in [5, 9, 12, 16, 10, 14, 18] {
        let source = format!(
            "m = magic({n}); t = {n}*({n}^2+1)/2; ok = all(sum(m) == t) && all(sum(m') == t) && sum(diag(m)) == t && sum(diag(fliplr(m))) == t && numel(unique(m(:))) == {n}^2;"
        );
        assert_eq!(show_after(&source, "ok"), "true", "magic({n})");
    }
}

#[test]
fn classic_matrices() {
    assert_eq!(show("hilb(2)"), "[1 0.5;0.5 0.333333333333333]");
    assert_eq!(show("invhilb(3)"), "[9 -36 30;-36 192 -180;30 -180 180]");
    assert_eq!(show("invhilb(2)"), "[4 -6;-6 12]");
    assert_eq!(show("pascal(4)"), "[1 1 1 1;1 2 3 4;1 3 6 10;1 4 10 20]");
    assert_eq!(show("pascal(3,1)"), "[1 0 0;1 -1 0;1 -2 1]");
    assert_eq!(show("pascal(3,2)"), "[1 1 1;-2 -1 0;1 0 0]");
    assert!(error("x = pascal(3,3);").contains("Second argument must be 0, 1, or 2."));
    assert_eq!(show_after("p = pascal(20);", "p(20,20)"), "35345263800");
    // quadratic construction: the old cubic P*P' took seconds here
    assert_eq!(
        show_after(
            "p = pascal(1500);",
            "[p(1,1500) p(1500,1) p(2,1500) p(3,4)]"
        ),
        "[1 1 1500 10]"
    );
    assert_eq!(show("toeplitz([1 2 3])"), "[1 2 3;2 1 2;3 2 1]");
    assert_eq!(
        show("toeplitz([1 2 3],[1 5 6 7])"),
        "[1 5 6 7;2 1 5 6;3 2 1 5]"
    );
    assert!(error("x = toeplitz([]);").contains("Index exceeds matrix dimensions."));
    assert!(error("x = toeplitz('ab');").contains("Inputs must be numeric."));
    // matrix arguments are read column by column; c(1) wins over r(1)
    assert_eq!(
        show("toeplitz([1 2;3 4])"),
        "[1 3 2 4;3 1 3 2;2 3 1 3;4 2 3 1]"
    );
    assert_eq!(
        show("toeplitz([1 2 3],[9 5 6 7])"),
        "[1 5 6 7;2 1 5 6;3 2 1 5]"
    );
    assert_eq!(
        show_after(
            "t = toeplitz([1+2i 3]);",
            "[real(t(:).') abs(imag(t(:).'))]"
        ),
        "[1 3 3 1 2 0 0 2]"
    );
    assert_eq!(show("vander([1 2 3])"), "[1 1 1;4 2 1;9 3 1]");
    assert_eq!(
        show("vander([1 2;3 4])"),
        "[1 1 1 1;27 9 3 1;8 4 2 1;64 16 4 1]"
    );
    assert!(error("x = vander([1 2 3],2);").contains("Too many input arguments."));
    assert!(error("x = invhilb(2.5);").contains("Size inputs must be integers."));
    assert!(error("x = pascal(2.5);").contains("Size inputs must be integers."));
    assert!(error("x = wilkinson(2.5);").contains("Size inputs must be integers."));
    assert!(error("x = wilkinson([1 2]);").contains("Size inputs must be scalar."));
    assert!(error("x = pascal(3,1,1);").contains("NEWCLASS input is not a string."));
    assert!(error("x = rosser(1);").contains("NEWCLASS input is not a string."));
    assert!(error("x = hilb(2,3);").contains("CLASSNAME must be 'double' or 'single'."));
    assert_eq!(show("hilb(2,'double')"), "[1 0.5;0.5 0.333333333333333]");
    assert_eq!(show("compan([1 8 15])"), "[-8 -15;1 0]");
    assert_eq!(show("compan([2 4 6 8])"), "[-2 -3 -4;1 0 0;0 1 0]");
    assert_eq!(show("compan(5)"), "[]");
    assert!(error("x = compan([]);").contains("Input argument must be a vector."));
    assert!(error("x = compan([1 2;3 4]);").contains("Input argument must be a vector."));
    assert!(error("x = compan('abc');").contains("not a numeric or logical array"));
    assert_eq!(
        show_after("c = compan([1+i 2 3]);", "[real(c(:).') imag(c(:).')]"),
        "[-1 1 -1.5 0 1 0 1.5 0]"
    );
    assert_eq!(
        show("wilkinson(5)"),
        "[2 1 0 0 0;1 1 1 0 0;0 1 0 1 0;0 0 1 1 1;0 0 0 1 2]"
    );
    assert_eq!(show("wilkinson(1)"), "0");
    assert_eq!(
        show_after("m = rosser();", "m(1,:)"),
        "[611 196 -192 407 -8 -52 -49 29]"
    );
    assert_eq!(
        show_after("m = rosser;", "m(8,:)"),
        "[29 -44 52 -23 208 208 -911 99]"
    );
    assert_eq!(
        show("hadamard(4)"),
        "[1 1 1 1;1 -1 1 -1;1 1 -1 -1;1 -1 -1 1]"
    );
    // hadamard(20): Hankel block read off the reference output
    assert_eq!(
        show_after("h = hadamard(20);", "h(2,:)"),
        "[1 -1 -1 1 1 -1 -1 -1 -1 1 -1 1 -1 1 1 1 1 -1 -1 1]"
    );
    assert_eq!(
        show_after("h = hadamard(20);", "h(:,20)'"),
        "[1 1 -1 -1 1 1 -1 -1 -1 -1 1 -1 1 -1 1 1 1 1 -1 -1]"
    );
    for n in [12, 20, 24, 40] {
        let source = format!("h = hadamard({n}); ok = isequal(h*h', {n}*eye({n}));");
        assert_eq!(show_after(&source, "ok"), "true", "hadamard({n})");
    }
    let rule = "n must be an integer and n, n/12 or n/20 must be a power of 2.";
    for source in [
        "hadamard(3)",
        "hadamard(28)",
        "hadamard(0)",
        "hadamard(2.5)",
        "hadamard(60)",
    ] {
        assert!(error(&format!("x = {source};")).contains(rule), "{source}");
    }
}

#[test]
fn randperm_is_a_permutation() {
    assert_eq!(show("sort(randperm(7))"), "[1 2 3 4 5 6 7]");
    assert_eq!(show("size(randperm(10,3))"), "[1 3]");
    assert_eq!(show("numel(unique(randperm(1000,40)))"), "40");
    assert_eq!(show("max(randperm(20000,5)) <= 20000"), "true");
    assert_eq!(show("size(randperm(0))"), "[1 0]");
    assert!(error("x = randperm(5,6);").contains("K must be less than or equal to N."));
    assert!(error("x = randperm(2.5);").contains("Size inputs must be integers."));
    assert!(error("x = randperm([1 2]);").contains("Size inputs must be scalar."));
    assert!(error("x = randperm(NaN);").contains("NaN and Inf not allowed."));
    assert!(error("x = randperm(5,-1);").contains("Inputs must be nonnegative scalar integers."));
    assert_eq!(show("size(randperm(-1))"), "[1 0]");
    // The one-argument form is the order of sort(rand(1,n)).
    assert_eq!(
        show_after(
            "rng(1); a = randperm(10); rng(1); b = randperm(10);",
            "isequal(a,b)"
        ),
        "true"
    );
    assert_eq!(
        show_after(
            "rng(3); a = randperm(6); rng(3); [s,b] = sort(rand(1,6));",
            "isequal(a,b)"
        ),
        "true"
    );
}

#[test]
fn randperm_reference_values_and_stream_consumption() {
    // Supplied reference rows plus the round-3 review's additional probes.
    // Draw counts compare the subsequent rand value with the same stream.
    for (seed, expression, expected, draws) in [
        (0, "randperm(5)", "[3 5 1 2 4]", 5),
        (0, "randperm(10)", "[6 3 7 8 5 1 2 4 9 10]", 10),
        (1, "randperm(7)", "[3 6 5 7 4 1 2]", 7),
        (0, "randperm(8,1)", "7", 1),
        (0, "randperm(100,2)", "[82 90]", 2),
        (0, "randperm(1000,5)", "[815 905 127 911 630]", 5),
        (0, "randperm(8,2)", "[5 6]", 4),
        (0, "randperm(8,3)", "[7 4 3]", 6),
        (0, "randperm(10,3)", "[9 5 4]", 6),
        (0, "randperm(4,2)", "[2 3]", 4),
        (0, "randperm(8,7)", "[3 2 4 8 6 7 1]", 14),
        (0, "randperm(10,5)", "[6 4 7 3 10]", 10),
        (0, "randperm(8,8)", "[7 2 8 6 5 1 4 3]", 7),
        (0, "randperm(10,10)", "[7 2 8 6 9 1 4 3 5 10]", 9),
        (0, "randperm(5,0)", "zeros(1,0)", 0),
        (0, "randperm(0,0)", "zeros(1,0)", 0),
        (0, "randperm(1,1)", "1", 0),
    ] {
        let setup = format!(
            "rng({seed}); p = {expression}; a = rand; rng({seed}); discarded = rand(1,{draws}); b = rand;"
        );
        assert_eq!(show_after(&setup, "p"), expected, "{expression}");
        assert_eq!(show_after(&setup, "isequal(a,b)"), "true", "{expression}");
    }
}

#[test]
fn randperm_samples_are_unique_and_bounded() {
    // Include sparse-map collisions, the algorithm boundary and full shuffles.
    for (n, k) in [(1000, 249), (1000, 250), (9, 2), (9, 3), (24, 23), (24, 24)] {
        for seed in 0..4 {
            let setup = format!("rng({seed}); p = randperm({n},{k});");
            let properties = format!("[size(p) numel(unique(p)) all(p>=1) all(p<={n})]");
            assert_eq!(show_after(&setup, &properties), format!("[1 {k} {k} 1 1]"));
        }
    }
}

#[test]
fn accumarray_groups() {
    assert_eq!(show("accumarray([1;2;4;2;4],101:105)"), "[101;206;0;208]");
    assert_eq!(show("accumarray([1;2;1],[10;20;30],[4 1])"), "[40;20;0;0]");
    assert_eq!(show("accumarray([1 1;2 2;1 1],[10;20;30])"), "[40 0;0 20]");
    assert_eq!(show("accumarray([1;3;3],1)"), "[1;0;2]");
    assert_eq!(show("accumarray([1;2;1],[10;20;30],[],'max')"), "[30;20]");
    assert_eq!(show("accumarray([1;2;1],[10;20;30],[],'min')"), "[10;20]");
    assert_eq!(show("accumarray([1;2;1],[2;3;4],[],'prod')"), "[8;3]");
    assert_eq!(
        show("accumarray([1;3;1],[5;6;7],[4 1],'max',-1)"),
        "[7;-1;6;-1]"
    );
    assert_eq!(show("accumarray([1;1],[NaN;2],[],'max')"), "2");
    assert_eq!(show("accumarray([1;1],[NaN;2])"), "NaN");
    assert_eq!(show("accumarray(zeros(0,1),zeros(0,1),[3 1])"), "[0;0;0]");
    assert!(
        error("x = accumarray([0;1],[5;6]);")
            .contains("First input SUBS must contain positive integer subscripts.")
    );
    assert!(
        error("x = accumarray([1;2;1],[10;20;30],[1 4]);")
            .contains("the third input SZ must be of the form [N 1].")
    );
    assert!(error("x = accumarray([1;3],[5;6],[2 1]);").contains("ALL(MAX(SUBS)<=SZ)"));
    assert!(error("x = accumarray([true;true],[1;2]);").contains("real, full, numeric matrix"));
    assert!(error("x = accumarray([],[]);").contains("at least one column"));
    assert!(error("x = accumarray([1;2],[5;6;7]);").contains("one element for each row"));
    assert!(error("x = accumarray([1;2],[1;2],[],'mean');").contains("sum, max, min and prod"));
    // SUBS without columns is rejected, not indexed
    assert!(error("x = accumarray(zeros(1,0),1);").contains("at least one column"));
    assert!(error("x = accumarray(zeros(3,0),[1;2;3]);").contains("at least one column"));
    // a row of 3 subscripts is one observation: VAL must have one element
    assert!(error("x = accumarray([1 3 3],[1 2 3]);").contains("one element for each row"));
}

#[test]
fn isequal_compares_by_value() {
    for (source, expected) in [
        ("isequal(1,1,1)", "true"),
        ("isequal([1 2],[1;2])", "false"),
        ("isequal(NaN,NaN)", "false"),
        ("isequal([1 NaN],[1 NaN])", "false"),
        ("isequal('a',97)", "true"),
        ("isequal('abc','abd')", "false"),
        ("isequal(true,1)", "true"),
        ("isequal(0,-0)", "true"),
        ("isequal([],zeros(0,3))", "false"),
        ("isequal([],'')", "true"),
        ("isequal({1,'a',[1 2]},{1,'a',[1 2]})", "true"),
        ("isequal({1,2},{1,3})", "false"),
        ("isequal({{1}},{{1}})", "true"),
        ("isequal({NaN},{NaN})", "false"),
        ("isequal({1},1)", "false"),
        ("isequal(1+2i,1+2i)", "true"),
        // structs: field order is ignored (reference rows)
        ("isequal(struct('a',1),struct('a',1))", "true"),
        ("isequal(struct('a',1),struct('a',2))", "false"),
        ("isequal(struct('a',1,'b',2),struct('b',2,'a',1))", "true"),
        ("isequal(struct('a',NaN),struct('a',NaN))", "false"),
        ("isequal(struct('a',1),struct('b',1))", "false"),
        ("isequal(struct('a',{1,2}),struct('a',{1,2}))", "true"),
        ("isequal(struct('a',{1,2}),struct('a',{1,3}))", "false"),
        ("isequal(struct('a',{}),struct('a',{}))", "true"),
        ("isequal(struct('a',1),1)", "false"),
        ("isequal(struct(),struct())", "true"),
        ("isequal(struct('a',{{1,2}}),struct('a',{{1,2}}))", "true"),
        (
            "isequal(struct('a',struct('b',[1 2])),struct('a',struct('b',[1 2])))",
            "true",
        ),
        ("isequal(1+0i,1)", "true"),
        ("isequal(ones(2),ones(2),ones(2,3))", "false"),
    ] {
        assert_eq!(show(source), expected, "{source}");
    }
    assert!(error("x = isequal(1);").contains("Not enough input arguments."));
}

#[test]
fn running_extrema() {
    assert_eq!(show("cummax([1 3 2 5 4])"), "[1 3 3 5 5]");
    assert_eq!(show("cummax([1 3 2;4 1 6])"), "[1 3 2;4 3 6]");
    assert_eq!(show("cummax([1 3 2;4 1 6],2)"), "[1 3 3;4 4 6]");
    assert_eq!(show("cummax([1 3 2;4 1 6],3)"), "[1 3 2;4 1 6]");
    assert_eq!(show("cummax([NaN 1 NaN 3 2])"), "[NaN 1 1 3 3]");
    assert_eq!(show("cummax([1;3;2])"), "[1;3;3]");
    assert_eq!(show("cummax(zeros(0,3))"), "zeros(0,3)");
    assert_eq!(show("cummin([5 3 4 1 2])"), "[5 3 3 1 1]");
    assert_eq!(show("cummin([5 3 4;1 6 2],2)"), "[5 3 3;1 1 1]");
    assert_eq!(show("cummin([NaN 5 NaN 3 4])"), "[NaN 5 5 3 3]");
    assert_eq!(show("cummax([1 3 2 5],2,'reverse')"), "[5 5 5 5]");
    assert_eq!(
        show_after("[m,i]=cummax([1 3 2 5 4]);", "[m;i]"),
        "[1 3 3 5 5;1 2 2 4 4]"
    );
    assert!(error("x = cummax([1 2],0);").contains("positive integer"));
    assert!(error("x = cummax([1 2],[1 2]);").contains("Dimension argument must be"));
    assert!(error("x = cummax(1,2,3);").contains("CUMMAX direction must be"));
    assert!(
        error("x = cummax('abc');").contains("CUMMAX supports only numerical or logical inputs.")
    );
    assert!(error("x = cummin('ba');").contains("CUMMIN supports only"));
    // logical in, logical out (reference: class(cummax([true false])) is logical)
    assert_eq!(show("cummax([true false true])"), "[true true true]");
    assert_eq!(show("class(cummin([true false]))"), "'logical'");
    // complex: by magnitude (reference: cummax([1+2i 3]) is [1+2i 3])
    assert_eq!(show("abs(cummax([1+2i 3 1]))"), "[2.23606797749979 3 3]");
    assert_eq!(show("real(cummax([3 1+2i]))"), "[3 3]");
    // rev4: a leading element that is NaN only in its imaginary part is kept
    // as it is (it was NaN+NaNi); the next non-NaN element takes over
    let lead = "z = [complex(1,NaN) 2 complex(5,NaN)];";
    for name in ["cummax", "cummin"] {
        let real = show_after(lead, &format!("real({name}(z))"));
        let imag = show_after(lead, &format!("isnan(imag({name}(z)))"));
        assert_eq!(real, "[1 2 2]", "{name}");
        assert_eq!(imag, "[true false false]", "{name}");
        assert_eq!(show(&format!("real({name}(complex(1,NaN)))")), "1");
        assert_eq!(
            show(&format!("real({name}([complex(3,NaN);complex(NaN,4);1]))")),
            "[3;NaN;1]"
        );
    }
    assert!(error("x = cummax();").contains("Not enough input arguments."));
}

#[test]
fn colon_and_constants() {
    assert_eq!(show("colon(1,3)"), "[1 2 3]");
    assert_eq!(show("colon(1,2,7)"), "[1 3 5 7]");
    assert_eq!(show("colon(7,-2,1)"), "[7 5 3 1]");
    assert_eq!(show("colon(3,1)"), "zeros(1,0)");
    assert_eq!(show("colon(1,0,3)"), "zeros(1,0)");
    assert_eq!(show("colon(0,0.1,0.3)"), "[0 0.1 0.2 0.3]");
    assert_eq!(show("isequal(colon(1,5),1:5)"), "true");
    assert!(error("x = colon(1);").contains("Not enough input arguments."));
    assert_eq!(show("flintmax == 2^53"), "true");
    assert_eq!(show("flintmax('double')"), "9.00719925474099e+15");
    assert_eq!(show("realmax"), "1.79769313486232e+308");
    assert_eq!(show("realmin"), "2.2250738585072e-308");
    assert_eq!(show("realmax('double') == realmax()"), "true");
    // The reference takes only a class name
    assert!(error("x = realmax(2,2);").contains("Too many input arguments."));
    for source in [
        "realmax(2)",
        "realmin([2 3])",
        "flintmax(1)",
        "flintmax(true)",
        "realmax('int8')",
        "realmax('Single')",
    ] {
        assert!(
            error(&format!("x = {source};")).contains("Input must be 'single' or 'double'."),
            "{source}"
        );
    }
    assert_eq!(show("colon('a','e')"), "'abcde'");
    assert_eq!(show("colon(1+2i,3)"), "[1 2 3]");
    assert_eq!(show("class(realmax('single'))"), "'single'");
}

#[test]
fn large_sizes_are_bounded() {
    assert!(error("x = magic(100000);").contains("limit"));
    assert!(error("x = hilb(1e9);").contains("limit"));
    assert!(error("x = toeplitz(1:70000);").contains("limit"));
}

/// Reference comparison helper (not a test): `MATBUILD_CASES=tool/matbuild-cases.txt
/// cargo test --release --test matbuild -- --ignored --nocapture` prints
/// `case | result` lines in the format of the reference case generator, so the
/// MATLAB and MatRust outputs can be diffed line by line.
#[test]
#[ignore]
fn print_cases() {
    let Ok(path) = std::env::var("MATBUILD_CASES") else {
        return;
    };
    for line in std::fs::read_to_string(path).unwrap().lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut engine = Engine::default();
        let (setup, expression) = line.split_once(";;").unwrap_or(("", line));
        let result = (|| -> Result<String, String> {
            if !setup.is_empty() {
                engine
                    .execute(&format!("{setup};"))
                    .map_err(|e| e.to_string())?;
            }
            engine
                .execute(&format!("r__ = {expression};"))
                .map_err(|e| e.to_string())?;
            match engine.execute("disp(mat2str(r__))") {
                Ok(shown) => Ok(shown.output.join(" ")),
                Err(_) => {
                    let size = engine
                        .execute("disp(mat2str(size(r__)))")
                        .map_err(|e| e.to_string())?;
                    Ok(format!("cell{}", size.output.join(" ")))
                }
            }
        })();
        match result {
            Ok(s) => println!("{line} | {s}"),
            Err(e) => println!("{line} | ERR: {e}"),
        }
    }
}

/// Run with MATBUILD_WORKERS=1 (then 4), --release --ignored --nocapture.
/// These paths stay serial; worker counts check overhead, not parallel speedup.
#[test]
#[ignore]
fn benchmark_round3_paths() {
    let workers: usize = std::env::var("MATBUILD_WORKERS")
        .expect("set MATBUILD_WORKERS to 1 or 4")
        .parse()
        .unwrap();
    matrust::initialize_worker_pool(workers).unwrap();
    let mut engine = Engine::default();
    engine.execute("rng(0); A = ones(1000,100);").unwrap();
    for (name, source) in [
        ("randperm_sparse_1000_5", "p = randperm(1000,5);"),
        ("randperm_sparse_1000000_5", "p = randperm(1000000,5);"),
        ("randperm_dense_100000_30000", "p = randperm(100000,30000);"),
        ("randperm_full_100000", "p = randperm(100000,100000);"),
        ("horzcat_empty_1000x100", "B = horzcat(zeros(1,0),A);"),
        ("vertcat_empty_1000x100", "B = vertcat(A,zeros(0,3));"),
    ] {
        engine.execute(source).unwrap();
        let mut times = Vec::new();
        for _ in 0..11 {
            let start = std::time::Instant::now();
            std::hint::black_box(engine.execute(source).unwrap());
            times.push(start.elapsed().as_secs_f64() * 1000.);
        }
        times.sort_by(f64::total_cmp);
        println!(
            "{name},workers={workers},reps=11,median_ms={:.6},min_ms={:.6}",
            times[5], times[0]
        );
    }
}
