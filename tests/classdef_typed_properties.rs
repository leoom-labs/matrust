//! Typed and validated classdef properties without size restrictions, plus
//! matching function `arguments` block forms.
//!
//! Public reference: MathWorks "Validate Property Values"
//! (https://www.mathworks.com/help/matlab/matlab_oop/validate-property-values.html):
//! when a property specifies a class without a size restriction and no explicit
//! default value, MATLAB initializes it to the class's empty value
//! (`zeros(0,0,class)` for numeric, `string.empty` 0x0 for `string`, `{}` for `cell`).

use matrust::Engine;
use std::fs;

fn folder(files: &[(&str, &str)]) -> (tempfile::TempDir, Engine) {
    let dir = tempfile::tempdir().unwrap();
    for (name, text) in files {
        fs::write(dir.path().join(name), text).unwrap();
    }
    let mut e = Engine::default();
    e.set_function_directory(dir.path()).unwrap();
    (dir, e)
}

#[test]
fn done_check_typed_properties_without_size() {
    let (_dir, mut e) = folder(&[
        (
            "MyClass.m",
            "\
classdef MyClass
    properties
        Val double = 10
    end
    methods
        function obj = MyClass(v)
            if nargin > 0
                obj.Val = v;
            end
        end
    end
end
",
        ),
        (
            "P.m",
            "\
classdef P
    properties
        Name string = \"a\" % trailing comment
        Count uint32
        Age double
        Tags cell
        Owner MyClass = MyClass()
        W double {mustBeInteger} = 2
        V {mustBePositive} = 1
        % Line continuation in property declaration
        Continued ...
            uint32 ...
            {mustBePositive} ...
            = 4
        % File-command names as properties
        dir string = \"d\"
        save logical = true
    end
    properties (Hidden)
        Secret (1,1) double {mustBeNonnegative} = 0
    end
    properties (Constant)
        K uint32 = 7
    end
    methods
        function obj = P(count_in)
            if nargin > 0
                obj.Count = count_in;
            end
        end
        function show(obj)
            % Command syntax must still work inside methods
            format short
        end
    end
end
",
        ),
    ]);

    e.execute(
        "\
p = P();
name_eq = double(p.Name == \"a\");
count_cls = class(p.Count);
count_sz = size(p.Count);
age_cls = class(p.Age);
age_sz = size(p.Age);
tags_cls = class(p.Tags);
tags_sz = size(p.Tags);
owner_cls = class(p.Owner);
w_val = p.W;
v_val = p.V;
cont_val = double(p.Continued);
cont_cls = class(p.Continued);
dir_val = p.dir;
save_val = double(p.save);
k_cls = class(P.K);
",
    )
    .unwrap();

    assert_eq!(
        e.array("name_eq").unwrap().as_numeric().unwrap().data(),
        &[1.0]
    );
    assert_eq!(e.array("count_cls").unwrap().preview(32), "'uint32'");
    assert_eq!(
        e.array("count_sz").unwrap().as_numeric().unwrap().data(),
        &[0.0, 0.0]
    );
    assert_eq!(e.array("age_cls").unwrap().preview(32), "'double'");
    assert_eq!(
        e.array("age_sz").unwrap().as_numeric().unwrap().data(),
        &[0.0, 0.0]
    );
    assert_eq!(e.array("tags_cls").unwrap().preview(32), "'cell'");
    assert_eq!(
        e.array("tags_sz").unwrap().as_numeric().unwrap().data(),
        &[0.0, 0.0]
    );
    assert_eq!(e.array("owner_cls").unwrap().preview(32), "'MyClass'");
    assert_eq!(
        e.array("w_val").unwrap().as_numeric().unwrap().data(),
        &[2.0]
    );
    assert_eq!(
        e.array("v_val").unwrap().as_numeric().unwrap().data(),
        &[1.0]
    );
    assert_eq!(
        e.array("cont_val").unwrap().as_numeric().unwrap().data(),
        &[4.0]
    );
    assert_eq!(e.array("cont_cls").unwrap().preview(32), "'uint32'");
    assert_eq!(e.array("dir_val").unwrap().preview(32), "\"d\"");
    assert_eq!(
        e.array("save_val").unwrap().as_numeric().unwrap().data(),
        &[1.0]
    );
    assert_eq!(e.array("k_cls").unwrap().preview(32), "'uint32'");

    // Dot assignment: conversion and no size check
    e.execute(
        "\
p.Count = 2.7;
p.Name = 5;
p.Age = [1 2; 3 4];
c_after = double(p.Count);
c_after_cls = class(p.Count);
n_after = p.Name;
n_after_cls = class(p.Name);
age_after_sz = size(p.Age);
",
    )
    .unwrap();
    assert_eq!(
        e.array("c_after").unwrap().as_numeric().unwrap().data(),
        &[3.0]
    );
    assert_eq!(e.array("c_after_cls").unwrap().preview(32), "'uint32'");
    assert_eq!(e.array("n_after").unwrap().preview(32), "\"5\"");
    assert_eq!(e.array("n_after_cls").unwrap().preview(32), "'string'");
    assert_eq!(
        e.array("age_after_sz")
            .unwrap()
            .as_numeric()
            .unwrap()
            .data(),
        &[2.0, 2.0]
    );

    // Validator failure on dot assignment
    let err = e.execute("p.W = 1.5;").unwrap_err();
    assert!(
        err.contains("integer") || err.contains("mustBeInteger"),
        "expected mustBeInteger message, got: {err}"
    );

    // Owner of user class rejects a struct
    let err = e.execute("p.Owner = struct('Val', 1);").unwrap_err();
    assert!(
        err.contains("MyClass") || err.contains("Unable to convert"),
        "expected conversion error to MyClass, got: {err}"
    );

    // Constructor assignment converts
    e.execute("p2 = P(4.6); p2_c = double(p2.Count); p2_cls = class(p2.Count);")
        .unwrap();
    assert_eq!(
        e.array("p2_c").unwrap().as_numeric().unwrap().data(),
        &[5.0]
    );
    assert_eq!(e.array("p2_cls").unwrap().preview(32), "'uint32'");

    // Size+class form still enforces size
    let err = e.execute("p.Secret = [1 2];").unwrap_err();
    assert!(
        err.contains("size") || err.contains("(1,1)"),
        "expected size error for Secret, got: {err}"
    );
}

#[test]
fn typed_properties_string_empty_default_and_all_set_paths() {
    let (_dir, mut e) = folder(&[
        (
            "H.m",
            "\
classdef H < handle
    properties
        S string
        Count uint32
        W double {mustBeInteger} = 2
        Tags cell
        Arr double {mustBeNonnegative} = [1 2]
    end
end
",
        ),
        (
            "V.m",
            "\
classdef V
    properties
        Count uint32
        W double {mustBeInteger} = 1
    end
end
",
        ),
    ]);

    // Implicit string default without size is 0x0 string (string.empty)
    e.execute(
        "\
h = H();
s_cls = class(h.S);
s_sz = size(h.S);
",
    )
    .unwrap();
    assert_eq!(e.array("s_cls").unwrap().preview(32), "'string'");
    assert_eq!(
        e.array("s_sz").unwrap().as_numeric().unwrap().data(),
        &[0.0, 0.0]
    );

    // set() on handle class converts and validates
    e.execute("set(h, 'Count', 2.7); h_c = double(h.Count); h_cls = class(h.Count);")
        .unwrap();
    assert_eq!(e.array("h_c").unwrap().as_numeric().unwrap().data(), &[3.0]);
    assert_eq!(e.array("h_cls").unwrap().preview(32), "'uint32'");

    let err = e.execute("set(h, 'W', 1.5);").unwrap_err();
    assert!(
        err.contains("integer") || err.contains("mustBeInteger"),
        "expected mustBeInteger message on set(), got: {err}"
    );

    // Indexed assignment validates
    e.execute("h.Tags{1} = 'ok'; t1 = h.Tags{1};").unwrap();
    assert_eq!(e.array("t1").unwrap().preview(32), "'ok'");
    let err = e.execute("h.Arr(2) = -5;").unwrap_err();
    assert!(
        err.contains("nonnegative") || err.contains("mustBeNonnegative"),
        "expected mustBeNonnegative message on indexed assign, got: {err}"
    );

    // deal() converts and validates
    e.execute("v1 = V(); v2 = V(); [v1.Count, v2.Count] = deal(2.7, 3.2);")
        .unwrap();
    e.execute("c1 = double(v1.Count); c2 = double(v2.Count);")
        .unwrap();
    assert_eq!(e.array("c1").unwrap().as_numeric().unwrap().data(), &[3.0]);
    assert_eq!(e.array("c2").unwrap().as_numeric().unwrap().data(), &[3.0]);

    let err = e.execute("[v1.W, v2.W] = deal(1, 1.5);").unwrap_err();
    assert!(
        err.contains("integer") || err.contains("mustBeInteger"),
        "expected mustBeInteger message on deal(), got: {err}"
    );
}

#[test]
fn typed_properties_size_only_and_invalid_default() {
    let (_dir, mut e) = folder(&[
        (
            "SzOnly.m",
            "\
classdef SzOnly
    properties
        Row (1,3)
    end
end
",
        ),
        (
            "BadDefault.m",
            "\
classdef BadDefault
    properties
        W double {mustBeInteger} = 1.5
    end
end
",
        ),
    ]);

    e.execute("s = SzOnly(); sz = size(s.Row); s.Row = [4 5 6]; r = s.Row;")
        .unwrap();
    assert_eq!(
        e.array("sz").unwrap().as_numeric().unwrap().data(),
        &[1.0, 3.0]
    );
    assert_eq!(
        e.array("r").unwrap().as_numeric().unwrap().data(),
        &[4.0, 5.0, 6.0]
    );

    let err = e.execute("s.Row = [1 2];").unwrap_err();
    assert!(
        err.contains("size") || err.contains("(1,3)"),
        "expected size error, got: {err}"
    );

    // Construction default must be validated
    let err = e.execute("BadDefault()").unwrap_err();
    assert!(
        err.contains("integer") || err.contains("mustBeInteger"),
        "expected mustBeInteger on default, got: {err}"
    );
}

#[test]
fn arguments_block_class_without_size() {
    let (_dir, mut e) = folder(&[(
        "fn_typed.m",
        "\
function [xo, so, wo] = fn_typed(x, s, w)
    arguments
        x double
        s string = \"a\"
        w double {mustBeInteger} = 2
    end
    xo = x;
    so = s;
    wo = w;
end
",
    )]);

    e.execute(
        "\
[x1, s1, w1] = fn_typed(uint8([1 2; 3 4]));
x1_cls = class(x1);
x1_sz = size(x1);
s1_eq = double(s1 == \"a\");
",
    )
    .unwrap();
    assert_eq!(e.array("x1_cls").unwrap().preview(32), "'double'");
    assert_eq!(
        e.array("x1_sz").unwrap().as_numeric().unwrap().data(),
        &[2.0, 2.0]
    );
    assert_eq!(
        e.array("s1_eq").unwrap().as_numeric().unwrap().data(),
        &[1.0]
    );
    assert_eq!(e.array("w1").unwrap().as_numeric().unwrap().data(), &[2.0]);

    e.execute("[~, s2, ~] = fn_typed(1, 5); s2_cls = class(s2); s2_eq = double(s2 == \"5\");")
        .unwrap();
    assert_eq!(e.array("s2_cls").unwrap().preview(32), "'string'");
    assert_eq!(
        e.array("s2_eq").unwrap().as_numeric().unwrap().data(),
        &[1.0]
    );

    let err = e.execute("fn_typed(1, \"b\", 1.5);").unwrap_err();
    assert!(
        err.contains("integer") || err.contains("mustBeInteger"),
        "expected mustBeInteger in arguments block, got: {err}"
    );
}
