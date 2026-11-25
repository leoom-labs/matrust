use matrust::Engine;
use std::fs;

fn is_true(engine: &mut Engine, code: &str) -> bool {
    engine.execute(code).unwrap();
    engine.logical_variables["ans"].data() == [1]
}

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
fn abstract_method_signature_does_not_kill_command_syntax_in_local_function() {
    let (_dir, mut e) = folder(&[
        (
            "AbsDemo.m",
            "\
classdef AbsDemo
    methods (Abstract)
        function bare
        function r = area(obj)
    end
    methods (Static)
        function y = run()
            y = local_helper();
        end
    end
end
function y = local_helper()
    format long
    y = 1;
end
",
        ),
        (
            "AbsDemoImpl.m",
            "\
classdef AbsDemoImpl < AbsDemo
    methods
        function bare(obj)
            disp(obj);
        end
        function r = area(obj)
            r = 1;
        end
    end
end
",
        ),
    ]);
    e.execute("y = AbsDemo.run();").unwrap();
    assert_eq!(e.variables["y"].data(), &[1.0]);
}

#[test]
fn beep_variable_shadows_command_syntax() {
    let mut e = Engine::default();
    e.execute("beep = 3; x = 2; y = beep -x;").unwrap();
    assert_eq!(e.variables["y"].data(), &[1.0]);

    let mut e2 = Engine::default();
    e2.execute("beep = 3; x = 2;").unwrap();
    e2.execute("z = beep -x;").unwrap();
    assert_eq!(e2.variables["z"].data(), &[1.0]);
}

#[test]
fn classdef_property_with_no_default_uses_0x0_user_class_object_array() {
    let (_dir, mut e) = folder(&[
        (
            "C.m",
            "\
classdef C
    properties
        Value = 1
    end
end
",
        ),
        (
            "Holder.m",
            "\
classdef Holder
    properties
        Owner C
    end
end
",
        ),
        (
            "Node.m",
            "\
classdef Node
    properties
        Next Node
    end
end
",
        ),
    ]);
    e.execute("p = Holder;").unwrap();
    assert!(is_true(&mut e, "strcmp(class(p.Owner), 'C')"));
    assert!(is_true(&mut e, "isempty(p.Owner)"));
    assert!(is_true(&mut e, "isequal(size(p.Owner), [0 0])"));
    assert!(is_true(&mut e, "isobject(p.Owner)"));
    assert!(is_true(&mut e, "isa(p.Owner, 'C')"));
    assert!(is_true(&mut e, "~isstruct(p.Owner)"));
    assert!(e.execute("disp(p.Owner)").is_ok());
    assert!(e.execute("x = p.Owner.Value;").is_err());

    e.execute("n = Node;").unwrap();
    assert!(is_true(
        &mut e,
        "strcmp(class(n.Next), 'Node') && isempty(n.Next) && isequal(size(n.Next), [0 0])"
    ));
}

#[test]
fn rng_integer_class_seeds() {
    let mut e = Engine::default();
    assert!(is_true(
        &mut e,
        "rng(uint32(1)); a = rand(1, 3); rng(1); b = rand(1, 3); isequal(a, b)"
    ));
    assert!(is_true(
        &mut e,
        "rng(int32(1)); c = rand(1, 3); isequal(a, c)"
    ));
    assert!(is_true(
        &mut e,
        "rng(uint64(1)); d = rand(1, 3); isequal(a, d)"
    ));
    assert!(is_true(
        &mut e,
        "rng(single(1)); f = rand(1, 3); isequal(a, f)"
    ));
    assert!(e.execute("rng(int32(-1));").is_err());
}

#[test]
fn rng_generator_names_from_variables_and_struct_fields() {
    let mut e = Engine::default();
    assert!(is_true(
        &mut e,
        "g = 'philox'; rng(42, g); a = rand(1, 3); s = rng; rng(0); rng(42, s.Type); b = rand(1, 3); isequal(a, b)"
    ));
    assert!(is_true(
        &mut e,
        "gs = \"philox\"; rng(gs); c = rand(1, 3); rng(0, 'philox'); d = rand(1, 3); isequal(c, d)"
    ));
    assert!(is_true(
        &mut e,
        "g = 'combRecursive'; rng(g); e = rand(1, 3); rng(0, 'combRecursive'); f = rand(1, 3); isequal(e, f)"
    ));
}

#[test]
fn rng_rejects_twister_state_that_twists_to_all_zero() {
    let mut e = Engine::default();
    e.execute("s = rng; s.State = uint32([1; zeros(623, 1); 624]);")
        .unwrap();
    assert!(e.execute("rng(s);").is_err());
    // Generator remains usable and does not loop infinitely.
    e.execute("r = rand;").unwrap();
    let r = e.variables["r"].data()[0];
    assert!(r > 0.0 && r < 1.0);
}

#[test]
fn rng_default_with_generator_name() {
    let mut e = Engine::default();
    e.execute("rng(123); rng('default', 'twister'); r = rand;")
        .unwrap();
    let r = e.variables["r"].data()[0];
    assert!((r - 0.8147236863931789).abs() < 1e-15, "got {r}");
    assert!(is_true(&mut e, "rng('default'); isequal(r, rand)"));

    assert!(is_true(
        &mut e,
        "rng('default', 'philox'); a = rand(1, 3); rng(0, 'philox'); b = rand(1, 3); isequal(a, b)"
    ));
}

#[test]
fn ver_matlab_returns_24_2_and_r2024b() {
    let mut e = Engine::default();
    e.execute("v = ver('matlab');").unwrap();
    assert!(is_true(&mut e, "strcmp(v.Name, 'MatRust')"));
    assert!(is_true(&mut e, "strcmp(v.Version, '24.2')"));
    assert!(is_true(&mut e, "strcmp(v.Release, '(R2024b)')"));
}

#[test]
fn single_abstract_signature_keeps_command_syntax_in_local_function() {
    let (_dir, mut e) = folder(&[(
        "OneAbs.m",
        "\
classdef OneAbs
    methods (Abstract)
        function r = area(obj)
    end
    methods (Static)
        function y = run()
            y = local_helper();
        end
    end
end
function y = local_helper()
    format long
    y = 1;
end
",
    )]);
    e.execute("y = OneAbs.run();").unwrap();
    assert_eq!(e.variables["y"].data(), &[1.0]);
}

#[test]
fn negated_abstract_attribute_keeps_command_syntax_in_method() {
    check_concrete_method_command_syntax("~Abstract");
}

#[test]
fn false_abstract_attribute_keeps_command_syntax_in_method() {
    for attribute in ["Abstract=false", "Abstract='false'", "Abstract=\"false\""] {
        check_concrete_method_command_syntax(attribute);
    }
}

fn check_concrete_method_command_syntax(attribute: &str) {
    let source = format!(
        "\
classdef NotAbs
    methods ({attribute})
        function y = run(obj)
            format long
            y = 2;
        end
    end
end
"
    );
    let (_dir, mut e) = folder(&[("NotAbs.m", &source)]);
    e.execute("z = run(NotAbs);")
        .unwrap_or_else(|err| panic!("{attribute}: {err}"));
    assert_eq!(e.variables["z"].data(), &[2.0]);
}

#[test]
fn beep_variable_at_statement_start_is_an_expression() {
    let mut e = Engine::default();
    e.execute("beep = 3; x = 2; beep -x").unwrap();
    assert_eq!(e.variables["ans"].data(), &[1.0]);
}

#[test]
fn implicit_empty_user_class_property_keeps_its_validated_shape() {
    let (_dir, mut e) = folder(&[
        ("C.m", "classdef C\nend\n"),
        (
            "Holder.m",
            "classdef Holder\n    properties\n        Owner (1,:) C\n    end\nend\n",
        ),
    ]);
    e.execute("p = Holder;").unwrap();
    assert!(is_true(&mut e, "strcmp(class(p.Owner), 'C')"));
    assert!(is_true(&mut e, "isequal(size(p.Owner), [1 0])"));
}

#[test]
fn indexing_empty_user_class_object_keeps_its_class() {
    let (_dir, mut e) = folder(&[
        (
            "C.m",
            "classdef C\n    properties\n        Value = 1\n    end\nend\n",
        ),
        (
            "Holder.m",
            "classdef Holder\n    properties\n        Owner C\n    end\nend\n",
        ),
    ]);
    e.execute("p = Holder; q = p.Owner;").unwrap();
    assert!(is_true(&mut e, "strcmp(class(q(:)), 'C')"));
    assert!(is_true(&mut e, "strcmp(class(q([])), 'C')"));
    assert!(is_true(&mut e, "strcmp(class(repelem(q, 2)), 'C')"));
    assert!(is_true(&mut e, "strcmp(class(p.Owner(:)), 'C')"));
}

#[test]
fn empty_operands_do_not_bypass_unsupported_value_class_equality() {
    let (_dir, mut e) = folder(&[
        ("C.m", "classdef C\nend\n"),
        (
            "Holder.m",
            "classdef Holder\n    properties\n        Owner C\n    end\nend\n",
        ),
    ]);
    e.execute("obj = C; p = Holder; q = p.Owner;").unwrap();
    for name in ["eq", "ne"] {
        let expected = e.execute(&format!("{name}(obj, cell(1))")).unwrap_err();
        assert!(expected.contains(&format!("Undefined function '{name}'")));
        for args in ["obj, cell(0)", "q, cell(1)", "obj, q"] {
            assert_eq!(e.execute(&format!("{name}({args})")).unwrap_err(), expected);
        }
    }
}

#[test]
fn empty_equality_preserves_supported_dispatch() {
    let (_dir, mut e) = folder(&[
        ("H.m", "classdef H < handle\nend\n"),
        (
            "Color.m",
            "classdef Color\n    enumeration\n        Red\n    end\nend\n",
        ),
        (
            "Comparable.m",
            "\
classdef Comparable
    methods
        function result = eq(obj, other)
            result = isempty(other);
        end
    end
end
",
        ),
        (
            "Holder.m",
            "classdef Holder\n    properties\n        Link H\n        Choice Color\n    end\nend\n",
        ),
    ]);
    e.execute("h = H; p = Holder; link = p.Link; choice = p.Choice; c = Comparable;")
        .unwrap();
    for args in ["h, link", "link, h", "Color.Red, choice", "choice, 'Red'"] {
        assert!(is_true(
            &mut e,
            &format!("r = eq({args}); islogical(r) && isequal(size(r), [0 0])")
        ));
    }
    assert!(is_true(&mut e, "eq(c, cell(0))"));
    let expected = e.execute("eq(h, cell(1))").unwrap_err();
    assert_eq!(e.execute("eq(h, cell(0))").unwrap_err(), expected);
}

#[test]
fn empty_handle_equality_preserves_broadcast_shape() {
    let (_dir, mut e) = folder(&[
        ("H.m", "classdef H < handle\nend\n"),
        (
            "Holder.m",
            "classdef Holder\n    properties\n        Links (:,1) H\n        Owner (1,:) H\n    end\nend\n",
        ),
    ]);
    e.execute("p = Holder; ls = p.Links; owner = p.Owner; h = H;")
        .unwrap();
    for (expr, expected) in [
        ("eq(ls, h)", [0., 1.]),
        ("ne(h, ls)", [0., 1.]),
        ("eq(h, owner)", [1., 0.]),
        ("ne(owner, h)", [1., 0.]),
        ("eq(ls, ls)", [0., 1.]),
    ] {
        e.execute(&format!("r = {expr}; dims = size(r);")).unwrap();
        assert!(e.logical_variables.contains_key("r"), "{expr}");
        assert_eq!(e.variables["dims"].data(), &expected, "{expr}");
    }
    // Implicit expansion (corpus round 1): 0x1 against 1x0 is 0x0, as MATLAB.
    e.execute("r = zeros(0,1) == zeros(1,0); dims = size(r);")
        .unwrap();
    assert_eq!(e.variables["dims"].data(), &[0., 0.]);
}
