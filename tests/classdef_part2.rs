//! classdef part 2: handle classes, inheritance, property get/set methods and
//! the property attributes they need. The class files are
//! tests/fixtures/classdef2/*.m. Semantics follow the MATLAB documentation
//! (handle classes, class hierarchies, property access methods); GNU Octave
//! 10.3 confirms the cases it supports (tests/fixtures/classdef2/octave/check.m, output
//! in tests/fixtures/classdef2/octave/check-expected.txt). Error texts and
//! display details are this project's own and documented in the classdef
//! help page.
use matrust::Engine;
use std::path::{Path, PathBuf};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/classdef2")
}
fn engine() -> Engine {
    let mut e = Engine::default();
    e.set_function_directory(&fixtures()).unwrap();
    e
}
fn output(e: &mut Engine, code: &str) -> String {
    match e.execute(code) {
        Ok(out) => out.output.join("\n"),
        Err(error) => panic!("{code}\n{error}"),
    }
}
fn error(e: &mut Engine, code: &str) -> String {
    match e.execute(code) {
        Ok(out) => panic!("{code} succeeded: {:?}", out.output),
        Err(error) => error,
    }
}
fn scalar(e: &Engine, name: &str) -> f64 {
    e.variables[name].data()[0]
}
fn text(e: &mut Engine, code: &str) -> String {
    output(e, &format!("disp({code})")).trim().to_owned()
}

// ---- handle classes ----

#[test]
fn handle_copies_alias_one_object() {
    let mut e = engine();
    output(&mut e, "a = Counter(1); b = a; b.count = 5; x = a.count;");
    assert_eq!(scalar(&e, "x"), 5.);
    // A method changes the object without returning it.
    output(&mut e, "a.bump(2); bump(b, 3); y = a.count;");
    assert_eq!(scalar(&e, "y"), 10.);
    // Copies inside containers and function arguments are the same object.
    output(&mut e, "c = {a}; s.h = a; s.h.count = 0; z = c{1}.count;");
    assert_eq!(scalar(&e, "z"), 0.);
    // The class name, isa handle, isobject.
    assert_eq!(text(&mut e, "class(a)"), "Counter");
    assert_eq!(text(&mut e, "isa(a, 'handle')"), "1");
    assert_eq!(text(&mut e, "isa(a, 'Counter')"), "1");
    assert_eq!(text(&mut e, "isobject(a)"), "1");
    // Display as for value classes.
    assert_eq!(
        output(&mut e, "a"),
        "\na = \n\n  Counter with properties:\n\n    count: 0\n    label: 'c'\n"
    );
}

#[test]
fn handle_identity_equality_and_isequal_by_value() {
    let mut e = engine();
    output(
        &mut e,
        "a = Box; b = a; c = Box; t1 = a == b; t2 = a == c; t3 = a ~= c; t4 = eq(a, b); t5 = ne(a, b);",
    );
    for (name, want) in [
        ("t1", true),
        ("t2", false),
        ("t3", true),
        ("t4", true),
        ("t5", false),
    ] {
        assert_eq!(e.logical_variables[name].data()[0] != 0, want, "{name}");
    }
    // isequal compares property values: two objects with equal values.
    assert_eq!(text(&mut e, "isequal(a, c)"), "1");
    output(&mut e, "c.v = 2;");
    assert_eq!(text(&mut e, "isequal(a, c)"), "0");
    assert_eq!(text(&mut e, "isequal(a, b)"), "1");
    let err = error(&mut e, "a == 1");
    assert!(
        err.contains("Undefined function 'eq' for input arguments of type 'Box'."),
        "{err}"
    );
}

#[test]
fn delete_runs_the_delete_method_and_invalidates_every_copy() {
    let mut e = engine();
    output(&mut e, "a = Counter(3); b = a;");
    assert_eq!(text(&mut e, "isvalid(a)"), "1");
    assert_eq!(output(&mut e, "delete(a)"), "deleting Counter 3");
    assert_eq!(text(&mut e, "isvalid(b)"), "0");
    assert_eq!(text(&mut e, "b.isvalid()"), "0");
    let err = error(&mut e, "b.count");
    assert!(err.contains("Invalid or deleted object."), "{err}");
    let err = error(&mut e, "b.count = 1;");
    assert!(err.contains("Invalid or deleted object."), "{err}");
    // The class name stays; the display names the deleted object.
    assert_eq!(text(&mut e, "class(b)"), "Counter");
    assert_eq!(
        output(&mut e, "b"),
        "\nb = \n\n  handle to deleted Counter\n"
    );
    assert_eq!(output(&mut e, "disp(b)"), "  handle to deleted Counter\n");
    // Deleting again does nothing; h.delete() is the same call.
    assert_eq!(output(&mut e, "delete(b)"), "");
    assert_eq!(
        output(&mut e, "c = Counter(4); c.delete();"),
        "deleting Counter 4"
    );
    // Subclass delete first, then the superclass's own.
    assert_eq!(
        output(&mut e, "t = Tracked; delete(t)"),
        "deleting Tracked\ndeleting Counter 0"
    );
    // Value objects have no delete/isvalid.
    let err = error(&mut e, "r = Rect; isvalid(r)");
    assert!(
        err.contains("Undefined function 'isvalid' for input arguments of type 'Rect'."),
        "{err}"
    );
}

#[test]
fn unreferenced_handles_are_freed_silently() {
    let mut e = engine();
    output(&mut e, "a = Box; b = a;");
    assert_eq!(e.cls_live_handles(), 1);
    output(&mut e, "clear a");
    assert_eq!(e.cls_live_handles(), 1);
    // The last reference goes: freed without running a delete method.
    assert_eq!(output(&mut e, "b = 1;"), "");
    assert_eq!(e.cls_live_handles(), 0);
    assert_eq!(output(&mut e, "c = Counter(9); c = 2;"), "");
    assert_eq!(e.cls_live_handles(), 0);
    // A temporary never stored and a reference cycle are freed too.
    output(&mut e, "x = Box().v;");
    assert_eq!(e.cls_live_handles(), 0);
    output(
        &mut e,
        "p = Node; q = Node; p.next = q; q.next = p; clear p q",
    );
    assert_eq!(e.cls_live_handles(), 0);
    // Still referenced from a struct: kept.
    output(&mut e, "s.n = Node; s.n.value = 4;");
    assert_eq!(e.cls_live_handles(), 1);
    assert_eq!(text(&mut e, "s.n.value"), "4");
}

#[test]
fn handle_reference_cycles_are_collected_during_a_run() {
    let mut e = engine();
    // 70000 two-node cycles (140000 handle objects, more than the 65536
    // that can exist at once), each dropped by the next iteration: the
    // collection at the handle table's threshold frees them during the loop.
    output(
        &mut e,
        "for i = 1:70000\n a = Node; b = Node; a.next = b; b.next = a;\nend\nclear a b",
    );
    assert!(e.cls_live_handles() < 16, "{}", e.cls_live_handles());
    // The same inside a function: the cycles it made since it was entered.
    output(&mut e, "n = makeCycles(70000);");
    assert_eq!(scalar(&e, "n"), 70000.);
    assert!(e.cls_live_handles() < 16, "{}", e.cls_live_handles());
    // Live objects survive the collections: a list kept in the workspace,
    // and an older object that only the caller's unfinished expression holds
    // while the function hangs a new object on it and builds cycles.
    output(
        &mut e,
        "list = Node; list.next = Node; list.next.value = 3; m = makeCycles(400);\n\
         h = Node; r = {h, attachVia(400)};",
    );
    assert_eq!(text(&mut e, "list.next.value"), "3");
    assert_eq!(text(&mut e, "exist('h', 'var')"), "0");
    assert_eq!(text(&mut e, "isvalid(r{1})"), "1");
    assert_eq!(text(&mut e, "isvalid(r{1}.next)"), "1");
    assert_eq!(text(&mut e, "r{1}.next.value"), "7");
    assert_eq!(e.cls_live_handles(), 4);
}

#[test]
fn cycles_dropped_by_called_functions_are_collected_by_the_caller() {
    let mut e = engine();
    // Each call builds a two-node cycle and drops it on return. The handle
    // table crosses its threshold inside the callee, where only the
    // callee's own cycles can go: the caller collects at its next statement
    // (65536 handle objects can exist at once; 40000 calls make 80000).
    output(&mut e, "for i = 1:40000, cyc(); end");
    assert_eq!(scalar(&e, "i"), 40000.);
    assert!(e.cls_live_handles() < 16, "{}", e.cls_live_handles());
    // Two levels down: cycLoop's own statements collect what cyc dropped.
    output(&mut e, "n = cycLoop(40000);");
    assert_eq!(scalar(&e, "n"), 40000.);
    assert!(e.cls_live_handles() < 16, "{}", e.cls_live_handles());
    // A doubly linked list whose nodes a method links both ways.
    output(
        &mut e,
        "c = Chain;\nfor i = 1:40000, c.push(i); c.push(-i); c.reset(); end\nc.push(5); c.push(6);",
    );
    assert_eq!(text(&mut e, "c.count"), "80002");
    assert_eq!(text(&mut e, "c.head.next.prev.value"), "6");
    assert!(e.cls_live_handles() < 16, "{}", e.cls_live_handles());
}

#[test]
fn handle_byte_cap_frees_unreferenced_objects_first() {
    let mut e = engine();
    // About 11 MiB per object (rand: zeros are stored compactly); at most
    // 256 MiB of handle object properties can exist, but objects no copy
    // refers to no longer count.
    output(&mut e, "for k = 1:30, x = Holder(zeros(1200)); end");
    output(&mut e, "for k = 1:30, x = Holder(rand(1200)); end");
    // No function returns in between (a construction is one): storing
    // into an object and dropping it, 30 times in one statement.
    let fill = |n: usize| {
        format!(
            "c = cell(1, {n}); for k = 1:{n}, c{{k}} = Box; end\n\
             for k = 1:{n}, c{{k}}.v = rand(1200); c{{k}} = []; end\n"
        )
    };
    output(&mut e, &format!("for r = 1\n{}end", fill(30)));
    // 23 dropped objects (about 253 MiB), then a new one or a plain array.
    output(&mut e, &format!("{}b = Big; z = rand(1000);", fill(23)));
    assert_eq!(text(&mut e, "size(b.data)"), "1200        1200");
    assert_eq!(text(&mut e, "size(z)"), "1000        1000");
    // Live objects still count.
    let err = error(&mut e, "for k = 1:30, keep{k} = Holder(rand(1200)); end");
    assert!(err.contains("256 MiB"), "{err}");
}

#[test]
fn private_delete_method_is_not_callable_from_outside() {
    let mut e = engine();
    output(&mut e, "g = Guarded;");
    for code in ["delete(g)", "g.delete()"] {
        let err = error(&mut e, code);
        assert!(
            err.contains("Cannot access private method 'delete' of class 'Guarded'."),
            "{code}: {err}"
        );
    }
    assert_eq!(text(&mut e, "isvalid(g)"), "1");
    // The class's own methods may delete it.
    assert_eq!(output(&mut e, "g.dispose()"), "deleting Guarded");
    assert_eq!(text(&mut e, "isvalid(g)"), "0");
}

#[test]
fn handle_is_abstract_and_handle_subclasses_inherit_it() {
    let mut e = engine();
    let err = error(&mut e, "h = handle();");
    assert!(err.contains("abstract class 'handle'"), "{err}");
    output(
        &mut e,
        "t = Tracked; u = t; u.bump(2); u.bump(3); n = t.count;",
    );
    assert_eq!(scalar(&e, "n"), 5.);
    assert_eq!(text(&mut e, "mat2str(t.history)"), "[2 5]");
    assert_eq!(text(&mut e, "isa(t, 'handle')"), "1");
    assert_eq!(text(&mut e, "isa(t, 'Counter')"), "1");
    assert_eq!(
        output(&mut e, "methods(t)"),
        "\nMethods for class Tracked:\n\n    Tracked\n    bump\n    delete\n    eq\n    isvalid\n    ne\n"
    );
    assert_eq!(
        output(&mut e, "superclasses(t)"),
        "\nSuperclasses for class Tracked:\n\n    Counter\n    handle\n"
    );
}

// ---- inheritance ----

#[test]
fn three_level_chain_with_superclass_constructors_and_methods() {
    let mut e = engine();
    output(&mut e, "q = Square(3);");
    assert_eq!(text(&mut e, "q.area()"), "9");
    assert_eq!(text(&mut e, "q.name"), "square");
    assert_eq!(text(&mut e, "q.describe()"), "square with area 9");
    // method@Super from each level.
    assert_eq!(text(&mut e, "q.tag()"), "Square>Rect>Shape");
    // isa up the chain, class the leaf.
    for class in ["Square", "Rect", "Shape"] {
        assert_eq!(text(&mut e, &format!("isa(q, '{class}')")), "1", "{class}");
    }
    assert_eq!(text(&mut e, "isa(q, 'handle')"), "0");
    assert_eq!(text(&mut e, "class(q)"), "Square");
    // Own properties first, then the superclasses'.
    assert_eq!(
        output(&mut e, "q"),
        "\nq = \n\n  Square with properties:\n\n       w: 3\n       h: 3\n    name: 'square'\n"
    );
    assert_eq!(
        output(&mut e, "properties(q)"),
        "\nProperties for class Square:\n\n    w\n    h\n    name\n"
    );
    assert_eq!(
        output(&mut e, "methods('Square')"),
        "\nMethods for class Square:\n\n    Square\n    area\n    describe\n    peek\n    tag\n"
    );
    let c = output(&mut e, "c = superclasses('Square')");
    assert!(c.contains("'Rect'") && c.contains("'Shape'"), "{c}");
    assert!(c.find("'Rect'") < c.find("'Shape'"), "{c}");
    // Value semantics are kept through the chain.
    output(&mut e, "r = q; r.w = 10; w = q.w;");
    assert_eq!(scalar(&e, "w"), 3.);
}

#[test]
fn protected_members_reach_subclasses_only() {
    let mut e = engine();
    output(&mut e, "r = Rect(2, 5);");
    assert_eq!(text(&mut e, "r.peek()"), "42");
    let err = error(&mut e, "r.secret");
    assert!(
        err.contains("Cannot access protected property 'secret' of class 'Rect'."),
        "{err}"
    );
    let err = error(&mut e, "r.secret = 1;");
    assert!(
        err.contains("Cannot set protected property 'secret' of class 'Rect'."),
        "{err}"
    );
}

#[test]
fn multiple_inheritance_left_to_right_and_ambiguity() {
    let mut e = engine();
    // Implicit superclass constructors run left to right.
    assert_eq!(
        output(&mut e, "b = Both;"),
        "Left constructor\nRight constructor"
    );
    assert_eq!(text(&mut e, "b.who()"), "Both:Left+Right");
    assert_eq!(text(&mut e, "b.side()"), "Left.side");
    assert_eq!(text(&mut e, "b.onlyRight()"), "Right.onlyRight");
    // A common base through both superclasses is one class, not ambiguous.
    assert_eq!(text(&mut e, "b.rootName()"), "Root.rootName");
    assert_eq!(
        output(&mut e, "properties(b)"),
        "\nProperties for class Both:\n\n    left\n    root\n    right\n"
    );
    assert_eq!(
        output(&mut e, "superclasses('Both')"),
        "\nSuperclasses for class Both:\n\n    Left\n    Root\n    Right\n"
    );
    for (name, want) in [("Left", "1"), ("Right", "1"), ("Root", "1"), ("Shape", "0")] {
        assert_eq!(text(&mut e, &format!("isa(b, '{name}')")), want, "{name}");
    }
    // A method both superclasses define and Both does not: ambiguous.
    for call in ["b.common()", "common(b)"] {
        let err = error(&mut e, call);
        assert!(
            err.contains("Method 'common' of class 'Both' is ambiguous: the superclasses 'Left' and 'Right' both define it"),
            "{call}: {err}"
        );
    }
}

#[test]
fn sealed_abstract_and_mixed_hierarchies_error() {
    let mut e = engine();
    let err = error(&mut e, "s = SubFinal;");
    assert!(
        err.contains("class 'Final' is Sealed and cannot be subclassed (line 1)"),
        "{err}"
    );
    assert!(err.contains("SubFinal.m"), "{err}");
    // Abstract methods (signatures only) make the class abstract; a subclass
    // implementing only some of them is abstract too.
    let err = error(&mut e, "v = Vehicle;");
    assert!(
        err.contains("Cannot create an object of the abstract class 'Vehicle': it does not implement the abstract methods drive, wheels."),
        "{err}"
    );
    let err = error(&mut e, "v = Bike;");
    assert!(
        err.contains("Cannot create an object of the abstract class 'Bike': it does not implement the abstract methods drive."),
        "{err}"
    );
    output(&mut e, "c = Car('Volvo'); [ok, msg] = c.drive(50);");
    assert_eq!(text(&mut e, "c.summary()"), "Volvo on 4 wheels");
    assert_eq!(text(&mut e, "msg"), "Volvo drove 50");
    assert_eq!(
        output(&mut e, "methods(c)"),
        "\nMethods for class Car:\n\n    Car\n    drive\n    summary\n    wheels\n"
    );
    let err = error(&mut e, "m = Mixed;");
    assert!(
        err.contains("class 'Mixed' cannot derive from both handle and value classes (handle: Counter; value: Root)"),
        "{err}"
    );
    assert_eq!(text(&mut e, "Rect(2, 3).area()"), "6");
    // Superclass calls outside class methods.
    let err = error(&mut e, "r = Rect; tag@Shape(r)");
    assert!(err.contains("valid only in methods of a subclass"), "{err}");
}

#[test]
fn superclass_constructor_call_is_found_in_code_not_text() {
    let mut e = engine();
    // `obj@Mark` only in a comment and in literals: Mark's constructor runs
    // implicitly, once, with no arguments.
    output(&mut e, "q = Quiet;");
    assert_eq!(text(&mut e, "q.runs"), "1");
    assert_eq!(text(&mut e, "q.args"), "0");
    assert_eq!(text(&mut e, "q.note"), "obj@Mark(5)");
    // An explicit call replaces the implicit one.
    output(&mut e, "d = Loud;");
    assert_eq!(text(&mut e, "d.runs"), "1");
    assert_eq!(text(&mut e, "d.args"), "1");
}

// ---- property get/set methods ----

#[test]
fn get_and_set_methods_run_on_outside_access_and_inside_other_methods() {
    let mut e = engine();
    output(&mut e, "t = Temp(100);");
    // The constructor's assignment went through set.celsius.
    assert_eq!(text(&mut e, "t.sets"), "1");
    assert_eq!(text(&mut e, "t.fahrenheit"), "212");
    assert_eq!(text(&mut e, "t.kelvin"), "373.1500");
    output(&mut e, "t.fahrenheit = 32;");
    assert_eq!(text(&mut e, "t.celsius"), "0");
    // set.fahrenheit assigned celsius: its set method ran once more.
    assert_eq!(text(&mut e, "t.sets"), "2");
    // A write in another method calls the set method too.
    output(&mut e, "t = t.warm(5);");
    assert_eq!(text(&mut e, "t.celsius"), "5");
    assert_eq!(text(&mut e, "t.sets"), "3");
    let err = error(&mut e, "t.celsius = -300;");
    assert!(err.contains("below absolute zero"), "{err}");
    // getfield and subsref read through the get method as well.
    assert_eq!(text(&mut e, "getfield(t, 'fahrenheit')"), "41");
}

#[test]
fn accessors_reach_storage_without_recursion() {
    let mut e = engine();
    // get.value increments reads and returns storage; set.value (no output,
    // a handle class) stores twice the value.
    output(&mut e, "g = Logged; a = g.value; b = g.value;");
    assert_eq!(scalar(&e, "a"), 1.);
    assert_eq!(text(&mut e, "g.reads"), "2");
    output(&mut e, "g.value = 5; v = g.value;");
    assert_eq!(scalar(&e, "v"), 10.);
    assert_eq!(text(&mut e, "g.reads"), "3");
}

#[test]
fn dependent_hidden_and_immutable_properties() {
    let mut e = engine();
    output(&mut e, "t = Temp(10);");
    // Dependent values are computed; Hidden ones are not listed or shown.
    assert_eq!(
        output(&mut e, "t"),
        "\nt = \n\n  Temp with properties:\n\n       celsius: 10\n    fahrenheit: 50\n        kelvin: 283.1500\n"
    );
    assert_eq!(
        output(&mut e, "properties(t)"),
        "\nProperties for class Temp:\n\n    celsius\n    fahrenheit\n    kelvin\n"
    );
    assert_eq!(text(&mut e, "isprop(t, 'sets')"), "1");
    // Dependent properties have no storage.
    let fields = output(&mut e, "s = struct(t); f = fieldnames(s)");
    assert!(
        !fields.contains("fahrenheit") && fields.contains("celsius"),
        "{fields}"
    );
    let err = error(&mut e, "t.kelvin = 5;");
    assert!(
        err.contains("In class 'Temp', no set method is defined for Dependent property 'kelvin'."),
        "{err}"
    );
    let err = error(&mut e, "n = NoGet; n.d");
    assert!(
        err.contains("In class 'NoGet', no get method is defined for Dependent property 'd'."),
        "{err}"
    );
    // A set method without a get method still runs on assignment.
    assert_eq!(output(&mut e, "n = NoGet; n.d = 4;"), "set.d 4");
    // immutable: set by the constructor only (not by a method it calls).
    output(&mut e, "b = Badge(3);");
    assert_eq!(text(&mut e, "b.owner"), "badge 3");
    for code in ["b.id = 4;", "b = b.rename(4);"] {
        let err = error(&mut e, code);
        assert!(
            err.contains("Cannot set immutable property 'id' of class 'Badge' outside the constructor of 'Badge'."),
            "{code}: {err}"
        );
    }
    assert_eq!(text(&mut e, "b.id"), "3");
}

#[test]
fn octave_confirmation_script_matches_its_fixture() {
    // tests/fixtures/classdef2/octave/<name>.m runs here unchanged; Octave 10.3's output
    // of the same script is the fixture.
    for name in ["check", "fix1-check", "fix2-check"] {
        let script = std::fs::read_to_string(fixtures().join(format!("octave/{name}.m"))).unwrap();
        let script: String = script
            .lines()
            .filter(|l| !l.trim_start().starts_with("addpath"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut e = engine();
        let ours = output(&mut e, &script);
        let expected =
            std::fs::read_to_string(fixtures().join(format!("octave/{name}-expected.txt")))
                .unwrap();
        let lines = |text: &str| -> Vec<String> {
            text.lines()
                .map(str::trim_end)
                .filter(|l| !l.is_empty())
                .map(str::to_owned)
                .collect()
        };
        assert_eq!(lines(&ours), lines(&expected), "{name}");
    }
}
