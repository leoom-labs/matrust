//! Expected values are worked by hand in column-major order. atan2 uses
//! pi/4 and the identity atan2(2, 1) = atan(2).
use matrust::{Array, Complex64, Engine};

fn real(e: &Engine, name: &str, shape: &[usize], expected: &[f64]) {
    let a = e.array(name).unwrap_or_else(|| panic!("{name} is missing"));
    assert_eq!(&*a.dims(), shape, "{name}");
    let Array::Numeric(m) = a else {
        panic!("{name} must be double");
    };
    assert_eq!(m.data(), expected, "{name}");
}

fn complex(e: &Engine, name: &str, shape: &[usize], expected: &[(f64, f64)]) {
    let a = e.array(name).unwrap();
    assert_eq!(&*a.dims(), shape, "{name}");
    let Array::Complex(m) = a else {
        panic!("{name} must be complex");
    };
    assert_eq!(
        m.data(),
        expected
            .iter()
            .map(|&(re, im)| Complex64::new(re, im))
            .collect::<Vec<_>>(),
        "{name}"
    );
}

#[test]
fn column_and_row_operators_and_function_forms() {
    let mut e = Engine::default();
    e.execute(
        r"
        a = [2;4]; b = [1 2 4];
        s = a+b; d = a-b; m = a.*b; r = a./b; l = a.\b; p = a.^b;
        S = plus(a,b); D = minus(a,b); M = times(a,b);
        R = rdivide(a,b); L = ldivide(a,b); P = power(a,b);
        row_matrix = [10 20 30] + [1 2 3; 4 5 6];
        basic_sum = [1;2] + [1 2 3]; scalar = 2.\8;
        ",
    )
    .unwrap();
    for (operator, function, expected) in [
        ("s", "S", [3., 5., 4., 6., 6., 8.]),
        ("d", "D", [1., 3., 0., 2., -2., 0.]),
        ("m", "M", [2., 4., 4., 8., 8., 16.]),
        ("r", "R", [2., 4., 1., 2., 0.5, 1.]),
        ("l", "L", [0.5, 0.25, 1., 0.5, 2., 1.]),
        ("p", "P", [2., 4., 4., 16., 16., 256.]),
    ] {
        real(&e, operator, &[2, 3], &expected);
        real(&e, function, &[2, 3], &expected);
    }
    real(&e, "row_matrix", &[2, 3], &[11., 14., 22., 25., 33., 36.]);
    real(&e, "basic_sum", &[2, 3], &[2., 3., 3., 4., 4., 5.]);
    real(&e, "scalar", &[1, 1], &[4.]);
}

#[test]
fn empty_and_nd_expansion() {
    let mut e = Engine::default();
    e.execute(
        r"
        a = reshape(1:4,2,1,2); b = [10 20 30];
        x = a+b; y = plus(b,a); q = [2 4].\a;
        z = zeros(0,3)+[1 2 3]; zr = [1 2 3]+zeros(0,3);
        zc = zeros(2,0).*[1;2]; zn = zeros(0,1,2)+b;
        cmp = a < [2 3 4]; mask = (a > 2) & [true false true];
        ci = a+1i; cmask = ci == [1+1i 2+1i];
        ",
    )
    .unwrap();
    let expected = [11., 12., 21., 22., 31., 32., 13., 14., 23., 24., 33., 34.];
    real(&e, "x", &[2, 3, 2], &expected);
    real(&e, "y", &[2, 3, 2], &expected);
    real(
        &e,
        "q",
        &[2, 2, 2],
        &[0.5, 1., 0.25, 0.5, 1.5, 2., 0.75, 1.],
    );
    real(&e, "z", &[0, 3], &[]);
    real(&e, "zr", &[0, 3], &[]);
    real(&e, "zc", &[2, 0], &[]);
    real(&e, "zn", &[0, 3, 2], &[]);
    for (name, shape, expected) in [
        (
            "cmp",
            &[2, 3, 2][..],
            &[1, 0, 1, 1, 1, 1, 0, 0, 0, 0, 1, 0][..],
        ),
        (
            "mask",
            &[2, 3, 2][..],
            &[0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 1, 1][..],
        ),
        ("cmask", &[2, 2, 2][..], &[1, 0, 0, 1, 0, 0, 0, 0][..]),
    ] {
        let a = e.array(name).unwrap();
        assert_eq!(&*a.dims(), shape, "{name}");
        let Array::Logical(a) = a else {
            panic!("{name} must be logical");
        };
        assert_eq!(a.data(), expected, "{name}");
    }
}

#[test]
fn typed_char_and_logical_operands() {
    let mut e = Engine::default();
    e.execute(
        r"
        saturated = int8([100;-100]) + int8([50 60]);
        saturated_nd = plus(reshape(int8([100 -100 120 -120]),2,1,2),int8([50 60]));
        wide = (uint64([1;3]) + uint64(9007199254740992)) + uint64([1 3]);
        s = single([2;4]) + [1 2 4]; t = ldivide(single([2;4]), [1 2 4]);
        u = rdivide(single([2;4]), [1 2 4]);
        c = ['a';'b'] + 'AB'; cl = 'AB' + [true;false];
        ll = [true;false] + [true false];
        ",
    )
    .unwrap();
    for (name, class, shape, expected) in [
        (
            "saturated",
            "int8",
            &[2, 2][..],
            &[127., -50., 127., -40.][..],
        ),
        (
            "saturated_nd",
            "int8",
            &[2, 2, 2][..],
            &[127., -50., 127., -40., 127., -70., 127., -60.][..],
        ),
        ("s", "single", &[2, 3][..], &[3., 5., 4., 6., 6., 8.][..]),
        (
            "t",
            "single",
            &[2, 3][..],
            &[0.5, 0.25, 1., 0.5, 2., 1.][..],
        ),
        ("u", "single", &[2, 3][..], &[2., 4., 1., 2., 0.5, 1.][..]),
    ] {
        let a = e.array(name).unwrap();
        assert_eq!(a.class(), class, "{name}");
        assert_eq!(&*a.dims(), shape, "{name}");
        let Array::Typed(a) = a else { unreachable!() };
        assert_eq!(
            (0..a.len()).map(|i| a.real(i)).collect::<Vec<_>>(),
            expected,
            "{name}"
        );
    }
    let Array::Typed(wide) = e.array("wide").unwrap() else {
        panic!("wide must be typed");
    };
    assert_eq!(
        (0..wide.len()).map(|i| wide.int(i)).collect::<Vec<_>>(),
        [
            9007199254740994,
            9007199254740996,
            9007199254740996,
            9007199254740998
        ]
    );
    real(&e, "c", &[2, 2], &[162., 163., 163., 164.]);
    real(&e, "cl", &[2, 2], &[66., 65., 67., 66.]);
    real(&e, "ll", &[2, 2], &[2., 1., 1., 0.]);
}

#[test]
fn complex_arithmetic_and_promotion() {
    let mut e = Engine::default();
    e.execute(
        r"
        a = [1i;2]; b = [1 2];
        s = a+b; d = b-a; m = a.*b; q = a.\b;
        p = power(a,b); r = [-4;9].^[0.5 1];
        pn = power(reshape([1i 2 2i 4],2,1,2),b);
        rc = reshape([-4 9 -16 25],2,1,2).^[0.5 1];
        ",
    )
    .unwrap();
    complex(&e, "s", &[2, 2], &[(1., 1.), (3., 0.), (2., 1.), (4., 0.)]);
    complex(
        &e,
        "d",
        &[2, 2],
        &[(1., -1.), (-1., 0.), (2., -1.), (0., 0.)],
    );
    complex(&e, "m", &[2, 2], &[(0., 1.), (2., 0.), (0., 2.), (4., 0.)]);
    complex(
        &e,
        "q",
        &[2, 2],
        &[(0., -1.), (0.5, 0.), (0., -2.), (1., 0.)],
    );
    complex(&e, "p", &[2, 2], &[(0., 1.), (2., 0.), (-1., 0.), (4., 0.)]);
    complex(&e, "r", &[2, 2], &[(0., 2.), (3., 0.), (-4., 0.), (9., 0.)]);
    complex(
        &e,
        "pn",
        &[2, 2, 2],
        &[
            (0., 1.),
            (2., 0.),
            (-1., 0.),
            (4., 0.),
            (0., 2.),
            (4., 0.),
            (-4., 0.),
            (16., 0.),
        ],
    );
    complex(
        &e,
        "rc",
        &[2, 2, 2],
        &[
            (0., 2.),
            (3., 0.),
            (-4., 0.),
            (9., 0.),
            (0., 4.),
            (5., 0.),
            (-16., 0.),
            (25., 0.),
        ],
    );
}

#[test]
fn everyday_binary_functions_expand() {
    let mut e = Engine::default();
    e.execute(
        "m = mod([5;7],[2 3]); r = rem([-5;-7],[2 3]); \
         h = hypot([3;0],[4 0]); a = atan2([1;2],[1 1 1]); \
         n = mod(reshape([5 7 8 9],2,1,2),[2 3]); \
         s = hypot(reshape(single([3 0 0 3]),2,1,2),[4 0]);",
    )
    .unwrap();
    real(&e, "m", &[2, 2], &[1., 1., 2., 1.]);
    real(&e, "r", &[2, 2], &[-1., -1., -2., -1.]);
    real(&e, "h", &[2, 2], &[5., 4., 3., 0.]);
    real(&e, "n", &[2, 2, 2], &[1., 1., 2., 1., 0., 1., 2., 0.]);
    // Explicit N-D decision (9fdfad5d): M=mod(reshape(1:4,2,1,2),[2 3]) has size [2 2 2] and M(:,:,2)=[1 0;0 1].
    e.execute("M = mod(reshape(1:4,2,1,2),[2 3]); M2 = M(:,:,2);")
        .unwrap();
    real(&e, "M", &[2, 2, 2], &[1., 0., 1., 2., 1., 0., 0., 1.]);
    real(&e, "M2", &[2, 2], &[1., 0., 0., 1.]);
    let m = e.variables.get("a").unwrap();
    assert_eq!((m.rows(), m.cols()), (2, 3));
    for (i, &actual) in m.data().iter().enumerate() {
        let expected = [std::f64::consts::FRAC_PI_4, 2_f64.atan()][i % 2];
        assert!((actual - expected).abs() < 1e-15, "{actual} vs {expected}");
    }
    let a = e.array("s").unwrap();
    assert_eq!(&*a.dims(), &[2, 2, 2]);
    let Array::Typed(a) = a else {
        panic!("s must be single");
    };
    assert_eq!(a.class().name(), "single");
    assert_eq!(
        (0..a.len()).map(|i| a.real(i)).collect::<Vec<_>>(),
        [5., 4., 3., 0., 4., 5., 0., 3.]
    );
}

#[test]
fn incompatible_sizes_have_matlab_message_and_identifier() {
    let mut e = Engine::default();
    for expression in [
        "[1 2]+[1 2 3]",
        "[1 2].^[1 2 3]",
        "[1 2].\\[1 2 3]",
        "[1i 2]+[1 2 3]",
        "power([1i 2],[1 2 3])",
        "int8([1 2]) + int8([1 2 3])",
        "plus(single([1 2]),[1 2 3])",
        "zeros(0,3)+ones(2,3)",
        "reshape(1:4,2,1,2)+ones(3,1)",
        "reshape(1:4,2,1,2)<ones(3,1)",
        "reshape(int8(1:4),2,1,2)+int8(ones(3,1))",
        "reshape([1i 2 3 4],2,1,2)==ones(3,1)",
        "mod([1 2],[1 2 3])",
    ] {
        e.execute(&format!(
            "try, x = {expression}; catch caught, \
             assert(strcmp(caught.message,'Arrays have incompatible sizes for this operation.')); \
             assert(strcmp(caught.identifier,'MATLAB:sizeDimensionsMustMatch')); end; \
             assert(exist('caught','var') == 1); clear caught;"
        ))
        .unwrap_or_else(|err| panic!("{expression}: {err}"));
    }
    let error = e.execute("x = zeros(8192,1)+zeros(1,8192);").unwrap_err();
    assert!(error.contains("element limit"), "{error}");
}

#[test]
fn expanded_output_obeys_the_statement_budget() {
    let mut e = Engine::with_limits(matrust::engine::Limits {
        evaluation_units: Some(4096),
        ..matrust::engine::Limits::default()
    });
    e.execute("a = zeros(128,1); b = zeros(1,128); c = reshape(a,64,1,2);")
        .unwrap();
    for expr in ["a+b", "mod(a,b)", "plus(c,b)", "single(a).*single(b)"] {
        let err = e.execute(&format!("x = {expr};")).unwrap_err();
        assert!(err.contains("evaluation budget"), "{expr}: {err}");
        assert!(e.array("x").is_none());
    }
}

/// MathWorks `mod` and `rem` pages allow combining an integer array with the same integer class
/// or a scalar `double` (and `single` with `double`), while rejecting mismatched integer classes,
/// non-scalar `double`, `single` + integer, or `logical` + integer.
#[test]
fn mod_rem_enforce_integer_operand_class_rules_and_preserve_precision() {
    let mut e = Engine::default();
    for bad in [
        "mod(int8([5;7]), uint8([2 3]))",
        "rem(int8([-5;-7]), uint8([2 3]))",
        "mod(int8([5;7]), [2 3])",
        "mod([5;7], int8([2 3]))",
        "rem(int8([-5;-7]), [2 3])",
        "mod(int8([5;7]), single([2 3]))",
        "mod(single([5;7]), int8([2 3]))",
        "mod(int8([5;7]), true)",
    ] {
        let err = e.execute(bad).unwrap_err();
        assert!(
            err.contains(
                "Integers can only be combined with integers of the same class, or scalar doubles."
            ),
            "{bad}: {err}"
        );
    }

    e.execute(
        "mi = mod(int8([5;-7]), int8([2 3])); \
         ri = rem(int8([5;-7]), int8([2 3])); \
         mu = mod(uint8([5;7]), uint8([2 3])); \
         ms = mod(int8([5;-7]), 3); \
         rs = rem(int8([5;-7]), 3); \
         ml = mod(-7, int8([2 3])); \
         sf = mod(single([5;7]), [2 3]); \
         bu = mod(uint64(9007199254740992) + uint64([1; 2]), uint64([2 3])); \
         bs = mod(uint64(9007199254740992) + uint64(1), 2); \
         bi = rem([-int64(9007199254740992) - int64(1); int64(9007199254740992) + int64(1)], int64([2 5]));",
    )
    .unwrap();

    for (name, class, shape, expected) in [
        ("mi", "int8", &[2, 2][..], &[1i128, 1, 2, 2][..]),
        ("ri", "int8", &[2, 2][..], &[1, -1, 2, -1][..]),
        ("mu", "uint8", &[2, 2][..], &[1, 1, 2, 1][..]),
        ("ms", "int8", &[2, 1][..], &[2, 2][..]),
        ("rs", "int8", &[2, 1][..], &[2, -1][..]),
        ("ml", "int8", &[1, 2][..], &[1, 2][..]),
        ("bu", "uint64", &[2, 2][..], &[1, 0, 0, 1][..]),
        ("bs", "uint64", &[1, 1][..], &[1][..]),
        ("bi", "int64", &[2, 2][..], &[-1, 1, -3, 3][..]),
    ] {
        let a = e.array(name).unwrap();
        assert_eq!(a.class(), class, "{name}");
        assert_eq!(&*a.dims(), shape, "{name}");
        let Array::Typed(a) = a else {
            panic!("{name} must be typed");
        };
        assert_eq!(
            (0..a.len()).map(|i| a.int(i)).collect::<Vec<_>>(),
            expected,
            "{name}"
        );
    }

    let sf = e.array("sf").unwrap();
    assert_eq!(sf.class(), "single");
    assert_eq!(&*sf.dims(), &[2, 2]);
    let Array::Typed(sf) = sf else {
        panic!("sf must be typed");
    };
    assert_eq!(
        (0..sf.len()).map(|i| sf.real(i)).collect::<Vec<_>>(),
        [1., 1., 2., 1.]
    );
}

fn typed_ints(e: &Engine, name: &str, class: &str, shape: &[usize]) -> Vec<i128> {
    let a = e.array(name).unwrap_or_else(|| panic!("{name} is missing"));
    assert_eq!(a.class(), class, "{name}");
    assert_eq!(&*a.dims(), shape, "{name}");
    let Array::Typed(a) = a else {
        panic!("{name} must be typed");
    };
    (0..a.len()).map(|i| a.int(i)).collect()
}

/// `2^53+1` is not an f64. `2^110` is an integral double above the old `1e30`
/// cutoff, so a lossy cast drops the low bit and both comparisons below fail.
#[test]
fn mod_rem_uint64_scalar_double_keeps_the_low_bit_past_1e30() {
    let mut e = Engine::default();
    e.execute(
        "a=uint64(9007199254740992)+uint64(1); b=2^110; m=mod(a,b); r=rem(a,b); \
         mu=mod(a,2^126); ru=rem(a,2^126); \
         mh=mod(a,2^127); rh=rem(a,2^127); \
         md=mod(b,a); rd=rem(b,a); \
         mb=mod(2^127,a); \
         x=-int64(9007199254740992)-int64(1); \
         sm=mod(x,b); sr=rem(x,b); \
         sh=mod(x,2^127); shr=rem(x,2^127); \
         fr=mod(int8(8),2.5); rr=rem(int8(-8),2.5); mr=mod(int8(-8),2.5); \
         mz=mod(int8(5),0); rz=rem(int8(5),0); \
         zn=mod(int8(5),NaN); zi=rem(int8(5),Inf);",
    )
    .unwrap();
    let exact = 9007199254740992i128 + 1;
    let lost = 9007199254740992i128;
    assert_ne!(exact, lost);
    for name in ["m", "r", "mu", "ru", "mh", "rh"] {
        let got = typed_ints(&e, name, "uint64", &[1, 1]);
        assert_eq!(got, [exact], "{name}");
        assert_ne!(got[0], lost, "{name}");
    }
    assert_eq!(typed_ints(&e, "md", "uint64", &[1, 1]), [16]);
    assert_eq!(typed_ints(&e, "rd", "uint64", &[1, 1]), [16]);
    assert_eq!(typed_ints(&e, "mb", "uint64", &[1, 1]), [1i128 << 21]);
    assert_eq!(typed_ints(&e, "sr", "int64", &[1, 1]), [-exact]);
    assert_eq!(typed_ints(&e, "shr", "int64", &[1, 1]), [-exact]);
    assert_eq!(typed_ints(&e, "sm", "int64", &[1, 1]), [i64::MAX as i128]);
    assert_eq!(typed_ints(&e, "sh", "int64", &[1, 1]), [i64::MAX as i128]);
    assert_eq!(typed_ints(&e, "fr", "int8", &[1, 1]), [1]);
    assert_eq!(typed_ints(&e, "rr", "int8", &[1, 1]), [-1]);
    assert_eq!(typed_ints(&e, "mr", "int8", &[1, 1]), [2]);
    assert_eq!(typed_ints(&e, "mz", "int8", &[1, 1]), [5]);
    assert_eq!(typed_ints(&e, "rz", "int8", &[1, 1]), [0]);
    assert_eq!(typed_ints(&e, "zn", "int8", &[1, 1]), [0]);
    assert_eq!(typed_ints(&e, "zi", "int8", &[1, 1]), [0]);
}

#[test]
fn negative_i128_boundary_mod_rem_by_minus_one_is_typed_zero() {
    let mut e = Engine::default();
    e.execute("m=mod(-(2^127),int8(-1)); r=rem(-(2^127),int8(-1));")
        .unwrap();
    for name in ["m", "r"] {
        assert_eq!(typed_ints(&e, name, "int8", &[1, 1]), [0]);
    }
}

/// N-D integer `mod`/`rem` streams into class-sized storage. A 128-element
/// `uint8` broadcast is 16 evaluation units at 1 byte each, and 256 units as
/// `i128`. The tight limit rejects it; the middle limit accepts it.
#[test]
fn mod_rem_nd_streams_typed_pages_and_obeys_a_small_limit() {
    let mut e = Engine::default();
    e.execute(
        "a = reshape(uint8([1 2 3 4]),2,1,2); b = uint8([2 3]); \
         m = mod(a,b); r = rem(a,b); p = m(:,:,2); \
         z = mod(uint8(zeros(0,1,2)), uint8([1 2 3])); \
         ze = rem(uint8(zeros(2,0)), uint8([1;2])); \
         zt = mod(uint8(zeros(2,3,0)), uint8(1));",
    )
    .unwrap();
    let page = [1i128, 0, 1, 2, 1, 0, 0, 1];
    assert_eq!(typed_ints(&e, "m", "uint8", &[2, 2, 2]), page);
    assert_eq!(typed_ints(&e, "r", "uint8", &[2, 2, 2]), page);
    assert_eq!(typed_ints(&e, "p", "uint8", &[2, 2]), [1, 0, 0, 1]);
    assert_eq!(typed_ints(&e, "z", "uint8", &[0, 3, 2]), []);
    assert_eq!(typed_ints(&e, "ze", "uint8", &[2, 0]), []);
    assert_eq!(typed_ints(&e, "zt", "uint8", &[2, 3, 0]), []);

    e.execute("c = reshape(uint8(1:16),4,1,4); d = uint8(1:8);")
        .unwrap();
    e.limits_set(matrust::engine::Limits {
        evaluation_units: Some(64),
        ..matrust::engine::Limits::default()
    });
    let err = e.execute("x = mod(c,d);").unwrap_err();
    assert!(err.contains("evaluation budget"), "{err}");
    assert!(e.array("x").is_none());

    e.limits_set(matrust::engine::Limits {
        evaluation_units: Some(200),
        ..matrust::engine::Limits::default()
    });
    e.execute("x = mod(c,d);").unwrap();
    let got = typed_ints(&e, "x", "uint8", &[4, 8, 4]);
    assert_eq!(got.len(), 128);
    for (i, value) in got.iter().enumerate() {
        let row = i % 4;
        let col = (i / 4) % 8;
        let page = i / 32;
        let dividend = (row + 4 * page + 1) as i128;
        let divisor = (col + 1) as i128;
        assert_eq!(*value, dividend % divisor, "index {i}");
    }
}
