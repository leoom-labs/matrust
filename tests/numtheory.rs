//! Number theory, special functions, bit operations and base conversion.
//! Expectations include reference cases, exact integer arithmetic and
//! 60-digit decimal values from tool/numtheory_ref.py. In particular,
//! erfcinv(5e-324) uses the higher-precision decimal calculation.
use matrust::{Array, Engine};

const FLINT: f64 = 9_007_199_254_740_992.0;

fn run(source: &str) -> Engine {
    let mut engine = Engine::default();
    engine
        .execute(source)
        .unwrap_or_else(|e| panic!("{source}: {e}"));
    engine
}

fn values(source: &str, name: &str) -> Vec<f64> {
    run(source).variables[name].data().to_vec()
}

fn shape(source: &str, name: &str) -> (usize, usize) {
    let engine = run(source);
    (engine.variables[name].rows(), engine.variables[name].cols())
}

fn error(source: &str) -> String {
    Engine::default().execute(source).unwrap_err()
}

fn assert_close(actual: &[f64], expected: &[f64], tolerance: f64) {
    assert_eq!(actual.len(), expected.len(), "{actual:?} vs {expected:?}");
    for (&a, &e) in actual.iter().zip(expected) {
        let same = a == e || (a.is_nan() && e.is_nan());
        assert!(
            same || ((a - e) / e).abs() <= tolerance,
            "{a:e} vs {e:e} in {actual:?}"
        );
    }
}

/// Rows of the character result `x`, read from its stored UTF-16 units.
fn shown(source: &str) -> Vec<String> {
    let Some(Array::Character(c)) = run(source).array("x") else {
        panic!("{source}: x is not character data");
    };
    let (rows, units) = (c.rows(), c.units());
    (0..rows)
        .map(|r| {
            let row: Vec<u16> = units.iter().skip(r).step_by(rows).copied().collect();
            String::from_utf16(&row).unwrap()
        })
        .collect()
}

#[test]
fn gcd_and_lcm_follow_scalar_expansion_and_sign_rules() {
    assert_eq!(values("x = gcd(12,18);", "x"), [6.0]);
    assert_eq!(values("x = gcd([12 18],6);", "x"), [6.0, 6.0]);
    assert_eq!(values("x = gcd(-12,18);", "x"), [6.0]);
    assert_eq!(values("x = gcd(0,0);", "x"), [0.0]);
    assert_eq!(values("x = lcm([4 6],[6 8]);", "x"), [12.0, 24.0]);
    assert!(error("x = lcm(0,5);").ends_with("Input arguments must contain positive integers."));
    assert!(error("x = lcm(-4,6);").ends_with("Input arguments must contain positive integers."));
    assert_eq!(shape("x = gcd([],5);", "x"), (0, 0));
    assert!(error("x = gcd(1.5,2);").contains("integer"));
    assert!(error("x = gcd(1);").contains("Not enough input arguments"));
    assert!(error("x = gcd([1 2 3],[1 2]);").contains("same size"));
    let engine = run("[g,c,d] = gcd(12,18);");
    let get = |n: &str| engine.variables[n].data()[0];
    assert_eq!((get("g"), get("c"), get("d")), (6.0, -1.0, 1.0));
    let engine = run("[g,c,d] = gcd([12 7],[18 5]);");
    for i in 0..2 {
        let (a, b) = ([12.0, 7.0][i], [18.0, 5.0][i]);
        let (g, c, d) = (
            engine.variables["g"].data()[i],
            engine.variables["c"].data()[i],
            engine.variables["d"].data()[i],
        );
        assert_eq!(a * c + b * d, g);
    }
}

#[test]
fn gcd_coefficients_terminate_at_the_extremes() {
    // realmax: q * v overflows in the classical recurrence; the exact remainder still ends it
    let engine = run("[g,c,d] = gcd(1.7976931348623157e308,3);");
    let get = |n: &str| engine.variables[n].data()[0];
    assert_eq!((get("g"), get("c")), (1.0, -1.0));
    assert!(get("d").is_finite() && (get("d") / (f64::MAX / 3.0) - 1.0).abs() < 1e-15);
    // consecutive Fibonacci numbers below 2^53: the longest Euclid chain, coefficients exact
    let (a, b) = (8_944_394_323_791_464i128, 5_527_939_700_884_757i128);
    let engine = run("[g,c,d] = gcd(8944394323791464,5527939700884757);");
    let get = |n: &str| engine.variables[n].data()[0] as i128;
    assert_eq!(get("g"), 1);
    assert_eq!(a * get("c") + b * get("d"), 1);
    let engine = run("[g,c,d] = gcd([0 7],[0 0]);");
    assert_eq!(engine.variables["g"].data(), [0.0, 7.0]);
    assert_eq!(engine.variables["c"].data(), [0.0, 1.0]);
}

#[test]
fn factorial_and_nchoosek() {
    assert_eq!(
        values("x = factorial([0 1 5 20]);", "x"),
        [1.0, 1.0, 120.0, 2_432_902_008_176_640_000.0]
    );
    assert_eq!(values("x = factorial(171);", "x"), [f64::INFINITY]);
    assert_close(
        &values("x = factorial(170);", "x"),
        &[7.257415615307994e306],
        1e-15,
    );
    assert_eq!(shape("x = factorial([]);", "x"), (0, 0));
    assert!(error("x = factorial(-1);").contains("non-negative"));
    assert!(error("x = factorial(2.5);").contains("non-negative"));
    assert_eq!(values("x = nchoosek(5,2);", "x"), [10.0]);
    assert_eq!(values("x = nchoosek(5,0);", "x"), [1.0]);
    assert_eq!(values("x = nchoosek(52,5);", "x"), [2_598_960.0]);
    assert!(error("x = nchoosek(3,5);").contains("between 0 and N"));
    assert!(error("x = nchoosek(1000,500);").contains("flintmax"));
    assert_eq!(
        values("x = nchoosek([1 2 3 4],2);", "x"),
        [1., 1., 1., 2., 2., 3., 2., 3., 4., 3., 4., 4.]
    );
    assert_eq!(shape("x = nchoosek([1;2;3],2);", "x"), (3, 2));
    assert_eq!(shape("x = nchoosek([1 2 3],0);", "x"), (1, 0));
    assert_eq!(shape("x = nchoosek([1 2 3],4);", "x"), (0, 4));
    assert_eq!(shape("x = nchoosek([1 2 3],3);", "x"), (1, 3));
    assert!(error("x = nchoosek(1:30,15);").contains("limit"));
    assert_eq!(shown("x = nchoosek('abc',2)"), ["ab", "ac", "bc"]);
}

#[test]
fn primes_isprime_factor() {
    assert_eq!(
        values("x = primes(30);", "x"),
        [2., 3., 5., 7., 11., 13., 17., 19., 23., 29.]
    );
    assert_eq!(shape("x = primes(1);", "x"), (1, 0));
    assert_eq!(shape("x = primes(2.5);", "x"), (1, 1));
    assert_eq!(values("x = numel(primes(1e6));", "x"), [78_498.0]);
    assert!(error("x = primes(Inf);").contains("finite"));
    let engine = run("x = isprime([1 2 3;4 5 6]);");
    assert_eq!(engine.logical_variables["x"].data(), [0, 0, 1, 1, 1, 0]);
    let engine = run("x = isprime(2^53-111);");
    assert_eq!(engine.logical_variables["x"].data(), [1]);
    assert!(error("x = isprime(-3);").contains("nonnegative"));
    assert_eq!(values("x = factor(60);", "x"), [2., 2., 3., 5.]);
    assert_eq!(values("x = factor(1);", "x"), [1.0]);
    assert_eq!(values("x = factor(97);", "x"), [97.0]);
    assert_eq!(
        values("x = factor(9007199254740991);", "x"),
        [6361., 69431., 20_394_401.]
    );
    assert!(error("[p,k] = factor(60);").ends_with("Too many output arguments."));
    assert!(error("x = factor([1 2]);").contains("scalar"));
    assert!(error("x = factor(-3);").contains("nonnegative"));
}

#[test]
fn gamma_family_matches_references() {
    assert_eq!(
        values("x = gamma([1 2 3 4 5]);", "x"),
        [1., 1., 2., 6., 24.]
    );
    assert_close(&values("x = gamma(0.5);", "x"), &[1.772453850905516], 1e-15);
    assert_close(
        &values("x = gamma(-1.5);", "x"),
        &[2.363271801207355],
        1e-15,
    );
    assert_close(
        &values("x = gamma(171);", "x"),
        &[7.257415615307994e306],
        1e-15,
    );
    assert_eq!(
        values("x = gamma([0 -0 172 Inf -Inf]);", "x"),
        [f64::INFINITY; 5]
    );
    assert_close(
        &values("x = gammaln([0.5 10 100]);", "x"),
        &[0.5723649429247001, 12.801827480081469, 359.1342053695754],
        1e-15,
    );
    assert!(error("x = gammaln(-1);").contains("nonnegative"));
    assert_close(&values("x = beta(2,3);", "x"), &[1.0 / 12.0], 1e-15);
    assert_close(
        &values("x = betaln(2,3);", "x"),
        &[(1.0f64 / 12.0).ln()],
        1e-14,
    );
    assert_eq!(values("x = beta(0,1);", "x"), [f64::INFINITY]);
}

#[test]
fn error_functions_match_references() {
    assert_close(
        &values("x = erf([0.5 1 -2]);", "x"),
        &[0.5204998778130465, 0.8427007929497149, -0.9953222650189527],
        1e-15,
    );
    assert_close(
        &values("x = erfc([0.5 3 10]);", "x"),
        &[
            0.4795001221869535,
            2.209049699858544e-05,
            2.088487583762545e-45,
        ],
        1e-15,
    );
    assert_close(
        &values("x = erfinv([0.5 0.9 -0.25]);", "x"),
        &[0.4769362762044699, 1.1630871536766743, -0.2253120550121781],
        1e-15,
    );
    assert_close(
        &values("x = erfcinv([0.5 1e-10 1.5]);", "x"),
        &[0.4769362762044699, 4.572824967389486, -0.4769362762044699],
        1e-15,
    );
    assert_eq!(
        values("x = erfinv([1 -1 1.5]);", "x")[..2],
        [f64::INFINITY, f64::NEG_INFINITY]
    );
    assert!(values("x = erfinv(1.5);", "x")[0].is_nan());
    assert_eq!(
        values("x = erfcinv([0 2]);", "x"),
        [f64::INFINITY, f64::NEG_INFINITY]
    );
    assert!(values("x = erf(NaN);", "x")[0].is_nan());
    // round trips across the whole range
    let residual = values(
        "y = erf(erfinv(-0.99:0.11:0.99)) - (-0.99:0.11:0.99); x = max(abs(y));",
        "x",
    );
    assert!(residual[0] < 1e-15);
    assert!(error("x = erf(1,2);").contains("Too many input arguments"));
    assert!(error("x = erf();").contains("Not enough input arguments"));
}

#[test]
fn erfcinv_tails_reach_the_smallest_subnormal() {
    // references: tool/numtheory_ref.py (60-digit decimal, correctly rounded)
    assert_close(
        &values("x = erfcinv([1e-20 1e-100 1e-300 5e-324]);", "x"),
        &[
            6.601580622355143,
            15.065574702592645,
            26.209469960516124,
            27.21329321081295,
        ],
        1e-15,
    );
    assert_close(
        &values("x = erfcinv([1e-310 2.2250738585072014e-308 1e-8]);", "x"),
        &[26.644806559364763, 26.54325845425098, 4.05223724387139],
        1e-15,
    );
    assert_close(
        &values("x = erfcinv(2-1e-15);", "x"),
        &[-5.666765021916332],
        1e-15,
    );
    assert_close(
        &values("x = erfc(20);", "x"),
        &[5.395865611607901e-176],
        1e-15,
    );
    assert_eq!(
        values("x = erfcinv([0 2]);", "x"),
        [f64::INFINITY, f64::NEG_INFINITY]
    );
    assert!(
        values("x = erfcinv([-1e-300 2.5 NaN]);", "x")
            .iter()
            .all(|v| v.is_nan())
    );
    // the tail is monotone and round-trips through erfc
    // (one ulp of t near 26 moves erfc(t) by ~2e-13 relative)
    let r = values(
        "p = 10.^(-(20:20:300)); t = erfcinv(p); x = [min(diff(t)) max(abs(erfc(t)./p-1))];",
        "x",
    );
    assert!(r[0] > 0.0 && r[1] < 1e-12, "{r:?}");
}

#[test]
fn bit_operations_on_doubles() {
    assert_eq!(values("x = bitand(12,10);", "x"), [8.0]);
    assert_eq!(values("x = bitor(12,10);", "x"), [14.0]);
    assert_eq!(values("x = bitxor([12 5],10);", "x"), [6.0, 15.0]);
    assert_eq!(
        values("x = bitand(2^53-1,2^52);", "x"),
        [4_503_599_627_370_496.0]
    );
    assert!(error("x = bitand(-1,1);").contains("ASSUMEDTYPE"));
    assert!(error("x = bitor(1.5,1);").contains("ASSUMEDTYPE"));
    assert_eq!(values("x = bitshift(16,[-1 1 2]);", "x"), [8., 32., 64.]);
    assert_eq!(values("x = bitshift(255,4,'uint8');", "x"), [240.0]);
    assert!(error("x = bitcmp(1,2);").ends_with("ASSUMEDTYPE must be an integer type name."));
    assert!(error("x = bitshift(255,4,8);").ends_with("ASSUMEDTYPE must be an integer type name."));
}

#[test]
fn bit_operations_accept_flintmax_and_default_to_uint64() {
    // inclusive bound: 2^53 itself is a valid double input
    assert_eq!(values("x = bitand(2^53,0);", "x"), [0.0]);
    assert_eq!(values("x = bitor(2^53,1);", "x"), [FLINT]);
    assert_eq!(values("x = bitxor(2^53,2^53);", "x"), [0.0]);
    assert_eq!(
        values("x = bitshift(2^53,[-1 0]);", "x"),
        [FLINT / 2.0, FLINT]
    );
    assert!(error("x = bitand(2^53+2,1);").contains("flintmax"));
    assert!(error("x = bitshift(2^53+2,1);").contains("flintmax"));
    assert!(error("x = bitcmp(2^53+2);").contains("flintmax"));
    // shifts and complements act on uint64 (MATLAB's assumed type for doubles), rounded to double
    assert_eq!(
        values("x = bitshift(1,[53 63 64]);", "x"),
        [FLINT, 2f64.powi(63), 0.0]
    );
    assert_eq!(
        values("x = bitshift(2^53-1,1);", "x"),
        [18_014_398_509_481_982.0]
    );
    assert_eq!(values("x = bitcmp(0);", "x"), [u64::MAX as f64]);
    assert_eq!(
        values("x = bitcmp(2^53);", "x"),
        [(u64::MAX - (1 << 53)) as f64]
    );
}

#[test]
fn base_conversion() {
    assert_eq!(shown("x = dec2bin(5)"), ["101"]);
    assert_eq!(shown("x = dec2bin(5,8)"), ["00000101"]);
    assert_eq!(shown("x = dec2bin(0)"), ["0"]);
    assert_eq!(shown("x = dec2hex(255)"), ["FF"]);
    assert_eq!(shown("x = dec2hex(255,4)"), ["00FF"]);
    assert_eq!(shown("x = dec2base(10,3)"), ["101"]);
    assert_eq!(shown("x = dec2base(35,36)"), ["Z"]);
    assert!(
        error("x = dec2base(10,'ab');")
            .ends_with("Second argument must be an integer, 2 <= B <= 36.")
    );
    assert_eq!(shown("x = dec2bin([5 9])"), ["0101", "1001"]);
    assert_eq!(values("x = size(dec2bin([1;2;3],4));", "x"), [3.0, 4.0]);
    assert_eq!(values("x = bin2dec('101');", "x"), [5.0]);
    assert_eq!(values("x = bin2dec('1 0 1');", "x"), [5.0]);
    assert_eq!(
        values("x = bin2dec(dec2bin([5 9 255]));", "x"),
        [5., 9., 255.]
    );
    assert_eq!(
        values("x = hex2dec(dec2hex([10 255 4096]));", "x"),
        [10., 255., 4096.]
    );
    assert_eq!(values("x = hex2dec('ff');", "x"), [255.0]);
    assert_eq!(values("x = base2dec('zz',36);", "x"), [1295.0]);
    assert_eq!(values("x = base2dec(dec2base(1000,7,6),7);", "x"), [1000.0]);
    assert!(
        error("x = bin2dec('102');")
            .ends_with("Binary string may consist only of characters 0 and 1")
    );
    assert!(
        error("x = hex2dec('G');")
            .ends_with("Input string found with characters other than 0-9, a-f, or A-F.")
    );
    assert!(error("x = dec2bin(-1);").contains("non-negative"));
    assert_eq!(shown("x = dec2bin(1.5)"), ["1"]);
    assert!(error("x = dec2base(10,37);").contains("2 <= B <= 36"));
    assert!(error("x = bin2dec(5);").contains("string"));
    assert_eq!(
        values("x = bin2dec(dec2bin(2^52));", "x"),
        [4_503_599_627_370_496.0]
    );
}

#[test]
fn base_conversion_rejects_oversized_text_before_building_it() {
    // 1e6 rows of 53 digits: the size check runs on the largest value, before any digits
    let start = std::time::Instant::now();
    assert!(error("x = dec2bin((2^53-1)*ones(1,1e6));").contains("65536"));
    assert!(error("x = dec2hex(1:2e4);").contains("65536"));
    assert!(error("x = dec2base(1,10,70000);").contains("65536"));
    assert!(start.elapsed().as_secs_f64() < 1.0);
    // widths still come from the largest element
    assert_eq!(shown("x = dec2bin([1 0 6])"), ["001", "000", "110"]);
    assert_eq!(values("x = size(dec2hex([0;4095]));", "x"), [2.0, 3.0]);
    assert_eq!(shown("x = dec2base([0 35],36,3)"), ["000", "00Z"]);
}

#[test]
fn nthroot_and_sign() {
    assert_eq!(values("x = nthroot(27,3);", "x"), [3.0]);
    assert_eq!(values("x = nthroot(-27,3);", "x"), [-3.0]);
    assert_eq!(values("x = nthroot(16,4);", "x"), [2.0]);
    assert_eq!(values("x = nthroot(-8,-3);", "x"), [-0.5]);
    assert_eq!(values("x = nthroot(0,-1);", "x"), [f64::INFINITY]);
    assert_close(
        &values("x = nthroot(8,[1 2 3]);", "x"),
        &[8.0, 8f64.sqrt(), 2.0],
        1e-15,
    );
    assert!(error("x = nthroot(-16,2);").contains("odd integer"));
}

#[test]
fn nthroot_takes_any_real_order_for_nonnegative_x() {
    assert_eq!(values("x = nthroot(4,0.5);", "x"), [16.0]);
    assert_eq!(values("x = nthroot([9 0],0.5);", "x"), [81.0, 0.0]);
    assert_close(&values("x = nthroot(8,1.5);", "x"), &[4.0], 1e-15);
    assert_close(&values("x = nthroot(2,-0.5);", "x"), &[0.25], 1e-15);
    assert!(error("x = nthroot(-8,1.5);").contains("If X is negative, N must be an odd integer."));
    assert!(error("x = nthroot(-8,[3 2]);").contains("odd integer"));
    assert_eq!(
        values("x = nthroot([0 0.5 1 2 Inf],0);", "x"),
        [0.0, 0.0, 1.0, f64::INFINITY, f64::INFINITY]
    );
    assert_eq!(values("x = sign([-2 0 3]);", "x"), [-1., 0., 1.]);
    assert!(values("x = sign(NaN);", "x")[0].is_nan());
    assert_eq!(values("x = sign(true);", "x"), [1.0]);
}

#[test]
fn user_functions_keep_precedence() {
    let engine = run("x = gcd(12,18); function y = gcd(a, b); y = 42; end");
    assert_eq!(engine.variables["x"].data(), [42.0]);
}

#[test]
fn nthroot_infinite_order_keeps_nan() {
    assert!(values("x = nthroot(NaN,Inf);", "x")[0].is_nan());
    assert!(values("x = nthroot(NaN,-Inf);", "x")[0].is_nan());
    assert!(values("x = nthroot(5,NaN);", "x")[0].is_nan());
    assert_eq!(values("x = nthroot([Inf 4 0],Inf);", "x"), [1.0, 1.0, 1.0]);
    assert_eq!(values("x = nthroot(4,-Inf);", "x"), [1.0]);
}

#[test]
fn assumed_integer_types_cover_sign_extension_and_widths() {
    // Width/sign expectations here use exact integer arithmetic; the uint8 example is reference evidence.
    for bits in [8, 16, 32, 64] {
        let unsigned = format!("uint{bits}");
        let signed = format!("int{bits}");
        assert_eq!(
            values(&format!("x = bitshift(1,{bits},'{unsigned}');"), "x"),
            [0.0]
        );
        assert_eq!(
            values(&format!("x = bitcmp(0,'{unsigned}');"), "x"),
            [(u64::MAX >> (64 - bits)) as f64]
        );
        assert_eq!(
            values(&format!("x = bitcmp([-1 0 1],'{signed}');"), "x"),
            [0.0, -1.0, -2.0]
        );
        assert_eq!(
            values(&format!("x = bitshift(-4,[-1 -100 1],'{signed}');"), "x"),
            [-2.0, -1.0, -8.0]
        );
        assert_eq!(
            values(&format!("x = bitshift(1,{},'{signed}');", bits - 1), "x"),
            [-2f64.powi(bits - 1)]
        );
    }
    assert_eq!(
        values("x = bitcmp([0 5 255],'uint8');", "x"),
        [255.0, 250.0, 0.0]
    );
    for source in [
        "x = bitshift(256,1,'uint8');",
        "x = bitcmp(128,'int8');",
        "x = bitcmp(-129,'int8');",
        "x = bitcmp(-1,'uint8');",
    ] {
        assert!(error(source).contains("ASSUMEDTYPE"));
    }
    for source in ["x = bitshift(1,2,'double');", "x = bitcmp(0,'uint7');"] {
        assert!(error(source).ends_with("ASSUMEDTYPE must be an integer type name."));
    }
}

#[test]
fn base_conversion_reference_errors_empty_and_flintmax() {
    assert!(
        error("x = hex2dec('F F');")
            .ends_with("Input string found with characters other than 0-9, a-f, or A-F.")
    );
    assert!(
        error("x = base2dec('12',2);")
            .ends_with("String 12 contains characters which cannot be converted to base 2.")
    );
    assert_eq!(shape("x = bin2dec('');", "x"), (0, 0));
    assert_eq!(values("x = bin2dec('   ');", "x"), [0.0]);
    assert_eq!(values("x = base2dec('baba','ab');", "x"), [10.0]);
    assert_eq!(shown("x = dec2bin(2^53)"), [format!("1{}", "0".repeat(53))]);
    assert_eq!(shown("x = dec2hex(2^53)"), ["20000000000000"]);
    assert_eq!(
        shown("x = dec2base(2^53,2)"),
        [format!("1{}", "0".repeat(53))]
    );
    for function in ["dec2bin", "dec2hex"] {
        assert!(error(&format!("x = {function}(2^53+2);")).contains("2^"));
    }
}

#[test]
fn nthroot_nonfinite_orders_follow_reference() {
    assert_eq!(values("x = nthroot(-8,Inf);", "x"), [-1.0]);
    assert_eq!(
        values("x = nthroot([-Inf -8 0 4 Inf],-Inf);", "x"),
        [-1.0, -1.0, 1.0, 1.0, 1.0]
    );
    assert!(values("x = nthroot(-8,NaN);", "x")[0].is_nan());
    assert!(values("x = nthroot(NaN,0);", "x")[0].is_nan());
}

#[test]
fn full_reference_additional_regressions() {
    let e = run("x = isprime([2^53 18446744073709551557 1e100]);");
    assert_eq!(e.logical_variables["x"].data(), [0, 0, 0]);
    assert!(
        error("x = nchoosek(5,2.5);")
            .ends_with("The second input has to be a non-negative integer.")
    );
    assert!(error("x = beta(-1,2);").ends_with("Input must be nonnegative."));
    assert!(error("x = primes([]);").ends_with("N must be a scalar"));
    assert!(
        error("x = erf('a');")
            .ends_with("Undefined function 'erf' for input arguments of type 'char'.")
    );
    assert_eq!(shown("x = dec2bin([0.5 1.5 3.9])"), ["00", "01", "11"]);
}
