use matrust::{Array, Complex64, Engine};

// Expected values are calculated by hand and listed in column-major order.
fn result(expression: &str) -> Array {
    let mut engine = Engine::default();
    engine
        .execute(&format!("r={expression};"))
        .unwrap_or_else(|error| panic!("{expression}: {error}"));
    engine.array("r").unwrap()
}

fn numeric(expression: &str, class: &str, shape: (usize, usize), expected: &[f64]) {
    let array = result(expression);
    assert_eq!(array.class(), class, "{expression}");
    assert_eq!((array.rows(), array.cols()), shape, "{expression}");
    let numeric = array.to_numeric(&Default::default()).unwrap();
    assert_eq!(numeric.len(), expected.len());
    for (&actual, &expected) in numeric.data().iter().zip(expected) {
        assert!(
            actual == expected || (actual.is_nan() && expected.is_nan()),
            "{expression}: {actual} != {expected}"
        );
    }
}

fn complex(expression: &str, shape: (usize, usize), expected: &[(f64, f64)]) {
    let Array::Complex(matrix) = result(expression) else {
        panic!("{expression} did not return complex storage");
    };
    assert_eq!((matrix.rows(), matrix.cols()), shape, "{expression}");
    let expected: Vec<_> = expected
        .iter()
        .map(|&(re, im)| Complex64::new(re, im))
        .collect();
    assert_eq!(matrix.data(), expected, "{expression}");
}

#[test]
fn comparisons_broadcast_each_axis_in_operator_and_function_forms() {
    for (operator, function, expected) in [
        ("==", "eq", [1., 0., 0., 1., 0., 0.]),
        ("~=", "ne", [0., 1., 1., 0., 1., 1.]),
        ("<", "lt", [0., 0., 1., 0., 1., 1.]),
        ("<=", "le", [1., 0., 1., 1., 1., 1.]),
        (">", "gt", [0., 1., 0., 0., 0., 0.]),
        (">=", "ge", [1., 1., 0., 1., 0., 0.]),
    ] {
        for (left, right) in [
            ("[1;2]", "[1 2 3]"),
            ("single([1;2])", "single([1 2 3])"),
            ("int8([1;2])", "uint16([1 2 3])"),
            ("complex([1;2],3)", "complex([1 2 3],3)"),
        ] {
            numeric(
                &format!("{left}{operator}{right}"),
                "logical",
                (2, 3),
                &expected,
            );
            numeric(
                &format!("{function}({left},{right})"),
                "logical",
                (2, 3),
                &expected,
            );
        }
    }
    numeric(
        "[1 2 3]==[1;2]",
        "logical",
        (2, 3),
        &[1., 0., 0., 1., 0., 0.],
    );
    numeric("eq([1 2;2 1],[1 2])", "logical", (2, 2), &[1., 0., 1., 0.]);
    numeric(
        "eq(logical([1;0]),[1 2])",
        "logical",
        (2, 2),
        &[1., 0., 0., 0.],
    );
    numeric("('ab')'=='ab'", "logical", (2, 2), &[1., 0., 0., 1.]);
    numeric("'abc'=='b'", "logical", (1, 3), &[0., 1., 0.]);
    numeric(
        "eq([intmax('uint64');intmax('uint64')-uint64(1)], [intmax('uint64')-uint64(1) intmax('uint64')])",
        "logical",
        (2, 2),
        &[0., 1., 1., 0.],
    );
    numeric(
        "eq(complex([1;2],3),complex([1 2],[3 4]))",
        "logical",
        (2, 2),
        &[1., 0., 0., 0.],
    );
    numeric(
        "ne(complex([1;2],3),complex([1 2],[3 4]))",
        "logical",
        (2, 2),
        &[0., 1., 1., 1.],
    );
    numeric(
        "ge(complex([1;2],-10),complex([1 2],10))",
        "logical",
        (2, 2),
        &[1., 1., 0., 1.],
    );
    numeric(
        "eq(uint8([1;2]),complex([1 2],[0 4]))",
        "logical",
        (2, 2),
        &[1., 0., 0., 0.],
    );
}

#[test]
fn boolean_operators_and_functions_expand_and_use_complex_truth() {
    let and = [1., 0., 1., 0., 0., 0.];
    let or = [1., 1., 1., 1., 1., 0.];
    let xor = [0., 1., 0., 1., 1., 0.];
    for (expression, expected) in [
        ("([1;0]&[1 1 0])", and),
        ("and(single([1;0]),uint8([1 1 0]))", and),
        ("[1;0]|[1 1 0]", or),
        ("or(logical([1;0]),logical([1 1 0]))", or),
        ("xor(complex(0,[1;0]),logical([1 1 0]))", xor),
        ("complex(0,[1;0])&[1 1 0]", and),
        ("complex(0,[1;0])|[1 1 0]", or),
    ] {
        numeric(expression, "logical", (2, 3), &expected);
    }
}

#[test]
fn extrema_bits_and_complex_components_expand_with_their_existing_classes() {
    numeric(
        "max([1;5],[2 3 4])",
        "double",
        (2, 3),
        &[2., 5., 3., 5., 4., 5.],
    );
    numeric(
        "min(int8([1;2]),int8([0 3]))",
        "int8",
        (2, 2),
        &[0., 0., 1., 2.],
    );
    numeric(
        "max(single([NaN;2]),single([1 NaN]))",
        "single",
        (2, 2),
        &[1., 2., f64::NAN, 2.],
    );
    numeric(
        "min([NaN;2],[1 NaN])",
        "double",
        (2, 2),
        &[1., 1., f64::NAN, 2.],
    );
    numeric(
        "max(logical([1;0]),logical([0 1]))",
        "logical",
        (2, 2),
        &[1., 0., 1., 1.],
    );
    complex(
        "max(complex([1;4],[2;0]),complex([3 0],[0 5]))",
        (2, 2),
        &[(3., 0.), (4., 0.), (0., 5.), (0., 5.)],
    );
    for (function, expected) in [
        ("bitand", [8., 10., 4., 2.]),
        ("bitor", [14., 10., 14., 14.]),
        ("bitxor", [6., 0., 10., 12.]),
    ] {
        numeric(
            &format!("{function}(uint8([12;10]),uint8([10 6]))"),
            "uint8",
            (2, 2),
            &expected,
        );
        numeric(
            &format!("{function}([12;10],[10 6])"),
            "double",
            (2, 2),
            &expected,
        );
    }
    numeric(
        "bitshift(uint8([12;10]),[-1 2])",
        "uint8",
        (2, 2),
        &[6., 5., 48., 40.],
    );
    numeric(
        "bitshift([12;10],[-1 2])",
        "double",
        (2, 2),
        &[6., 5., 48., 40.],
    );
    complex(
        "complex([1;2],[3 4])",
        (2, 2),
        &[(1., 3.), (2., 3.), (1., 4.), (2., 4.)],
    );
}

#[test]
fn singleton_axes_expand_to_zero_and_oversized_outputs_are_rejected() {
    numeric("zeros(0,1)==[1 2 3]", "logical", (0, 3), &[]);
    numeric("xor(true(3,1),false(1,0))", "logical", (3, 0), &[]);
    numeric("max(int8(zeros(0,1)),int8([1 2]))", "int8", (0, 2), &[]);
    numeric("bitand([1;2],zeros(1,0))", "double", (2, 0), &[]);
    complex("complex(zeros(0,1),[1 2 3])", (0, 3), &[]);
    let error = Engine::default()
        .execute("r=zeros(8192,1)==zeros(1,8192);")
        .unwrap_err();
    assert!(error.contains("element limit"), "{error}");
}

#[test]
fn every_affected_dispatch_reports_the_incompatible_size_identifier() {
    for expression in [
        "[1 2]==[1 2 3]",
        "eq(int8([1 2]),int8([1 2 3]))",
        "le(complex([1 2],0),complex([1 2 3],0))",
        "[1 2]&[1 2 3]",
        "xor(logical([1 0]),logical([0 1 0]))",
        "max([1 2],[1 2 3])",
        "min(single([1 2]),single([1 2 3]))",
        "min(complex([1 2],1),complex([1 2 3],1))",
        "bitxor([1 2],[1 2 3])",
        "bitshift(uint8([1 2]),[1 2 3])",
        "complex([1 2],[1 2 3])",
        "zeros(0,2)==ones(2,2)",
    ] {
        let mut engine = Engine::default();
        let script = format!(
            "try; r={expression}; catch err; message=err.message; identifier=err.identifier; end"
        );
        engine
            .execute(&script)
            .unwrap_or_else(|error| panic!("{expression}: {error}"));
        assert!(engine.array("r").is_none(), "{expression}");
        for (name, expected) in [
            (
                "message",
                "Arrays have incompatible sizes for this operation.",
            ),
            ("identifier", "MATLAB:sizeDimensionsMustMatch"),
        ] {
            let Some(Array::Character(value)) = engine.array(name) else {
                panic!("{expression}: no captured {name}");
            };
            assert_eq!(
                value.units(),
                expected.encode_utf16().collect::<Vec<_>>(),
                "{expression}"
            );
        }
    }
}

/// MathWorks `bitand`, `bitor`, and `bitxor` pages require matching integer classes or a scalar
/// `double`, while `bitshift` allows non-scalar `double` or different-class integer shift counts.
#[test]
fn bit_logic_enforces_integer_operand_class_rules_without_breaking_bitshift_counts() {
    for bad in [
        "bitand(int8([12;10]), uint8([10 6]))",
        "bitor(int8([12;10]), uint8([10 6]))",
        "bitxor(int8([12;10]), uint8([10 6]))",
        "bitand(int8([12;10]), [10 6])",
        "bitor([12;10], int8([10 6]))",
        "bitxor(int8([12;10]), single([10 6]))",
        "bitand(int8([12;10]), true)",
    ] {
        let err = Engine::default().execute(bad).unwrap_err();
        assert!(
            err.contains(
                "Integers can only be combined with integers of the same class, or scalar doubles."
            ),
            "{bad}: {err}"
        );
    }

    numeric(
        "bitand(int8([12;-4]), int8([10 6]))",
        "int8",
        (2, 2),
        &[8., 8., 4., 4.],
    );
    numeric(
        "bitor(int8([12;-4]), int8([10 6]))",
        "int8",
        (2, 2),
        &[14., -2., 14., -2.],
    );
    numeric(
        "bitxor(int8([12;-4]), int8([10 6]))",
        "int8",
        (2, 2),
        &[6., -10., 10., -6.],
    );
    numeric("bitand(int8([12;-4]), 10)", "int8", (2, 1), &[8., 8.]);
    numeric("bitor(10, int8([12;-4]))", "int8", (2, 1), &[14., -2.]);
    numeric(
        "bitshift(uint8([12;10]), int8([-1 2]))",
        "uint8",
        (2, 2),
        &[6., 5., 48., 40.],
    );

    let big = result("bitand(uint64(9007199254740992) + uint64(3), uint64(3))");
    assert_eq!(big.class(), "uint64");
    let Array::Typed(big) = big else {
        panic!("big must be typed");
    };
    assert_eq!(big.int(0), 3);
}
