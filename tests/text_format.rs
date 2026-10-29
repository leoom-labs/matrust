use matrust::{Array, Engine};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::AtomicBool;

/// Counts the bytes each thread allocates, so a test can prove that a rejected
/// call copied nothing.
struct CountingAllocator;
thread_local! {
    static ALLOCATED: Cell<usize> = const { Cell::new(0) };
}
fn count(bytes: usize) {
    let _ = ALLOCATED.try_with(|n| n.set(n.get().saturating_add(bytes)));
}
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count(new_size);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn run(e: &mut Engine, code: &str) {
    e.execute(code)
        .unwrap_or_else(|err| panic!("{code}: {err}"));
}
fn text_of(e: &Engine, name: &str) -> String {
    match e.array(name).unwrap() {
        Array::Character(c) => c.to_utf8(&AtomicBool::new(false)).unwrap(),
        other => panic!("{name} is not char: {other:?}"),
    }
}
/// Evaluate an expression and return the character result.
fn s(expr: &str) -> String {
    let mut e = Engine::default();
    run(&mut e, &format!("r = {expr};"));
    text_of(&e, "r")
}
fn cell(expr: &str) -> Vec<String> {
    let mut e = Engine::default();
    run(&mut e, &format!("r = {expr};"));
    let Array::Cell(c) = e.array("r").unwrap() else {
        panic!("not a cell")
    };
    (0..c.len())
        .map(|i| match c.get_linear(i).unwrap().as_ref() {
            Array::Character(t) => t.to_utf8(&AtomicBool::new(false)).unwrap(),
            other => panic!("{other:?}"),
        })
        .collect()
}
fn logical(expr: &str) -> Vec<bool> {
    let mut e = Engine::default();
    run(&mut e, &format!("r = {expr};"));
    let Array::Logical(m) = e.array("r").unwrap() else {
        panic!("not logical")
    };
    m.data().iter().map(|&v| v != 0).collect()
}
fn numbers(expr: &str) -> Vec<f64> {
    let mut e = Engine::default();
    run(&mut e, &format!("r = {expr};"));
    let Array::Numeric(m) = e.array("r").unwrap() else {
        panic!("not numeric")
    };
    m.data().to_vec()
}
fn err(code: &str) -> String {
    Engine::default().execute(code).unwrap_err()
}

#[test]
fn sprintf_recycles_format_over_matrix_arguments() {
    assert_eq!(s("sprintf('%d,', [1 2;3 4])"), "1,3,2,4,");
    assert_eq!(s("sprintf('%d-%d;', 1, [2 3], 4)"), "1-2;3-4;");
    assert_eq!(s("sprintf('%d and %d', 1)"), "1 and ");
    assert_eq!(s("sprintf('hello')"), "hello");
    assert_eq!(s("sprintf('x=%d')"), "x=");
}

#[test]
fn sprintf_every_conversion() {
    assert_eq!(s("sprintf('%d|%i|%u', 5, -6, 7)"), "5|-6|7");
    assert_eq!(s("sprintf('%f', pi)"), "3.141593");
    assert_eq!(
        s("sprintf('%e|%E', 12345.678, 0.00012)"),
        "1.234568e+04|1.200000E-04"
    );
    assert_eq!(
        s("sprintf('%g|%g|%g|%G', 100000, 1e6, 0.0001, 1e-5)"),
        "100000|1e+06|0.0001|1E-05"
    );
    assert_eq!(s("sprintf('%x|%X|%o', 255, 255, 8)"), "ff|FF|10");
    assert_eq!(s("sprintf('%c%c%c', 72, 105, 33)"), "Hi!");
    assert_eq!(s("sprintf('%s', 'abc')"), "abc");
    assert_eq!(s("sprintf('100%%')"), "100%");
}

#[test]
fn sprintf_flags_width_and_precision() {
    assert_eq!(s("sprintf('[%5d]', 42)"), "[   42]");
    assert_eq!(s("sprintf('[%-5d]', 42)"), "[42   ]");
    assert_eq!(s("sprintf('[%+d]', 42)"), "[+42]");
    assert_eq!(s("sprintf('[% d]', 42)"), "[ 42]");
    assert_eq!(s("sprintf('[%05d]', -42)"), "[-0042]");
    assert_eq!(s("sprintf('[%#x]', 255)"), "[0xff]");
    assert_eq!(s("sprintf('[%#o]', 8)"), "[010]");
    assert_eq!(s("sprintf('[%8.3f]', pi)"), "[   3.142]");
    assert_eq!(s("sprintf('[%.0f]', 2.5)"), "[3]");
    assert_eq!(s("sprintf('[%#.0f]', 3)"), "[3.]");
    assert_eq!(s("sprintf('[%.3s]', 'abcdef')"), "[abc]");
    assert_eq!(s("sprintf('[%6s]', 'ab')"), "[    ab]");
    assert_eq!(s("sprintf('[%-6s]', 'ab')"), "[ab    ]");
    assert_eq!(s("sprintf('[%*d]', 5, 42)"), "[   42]");
    assert_eq!(s("sprintf('[%.*f]', 2, pi)"), "[3.14]");
    assert_eq!(s("sprintf('[%*.*f]', 8, 1, pi)"), "[     3.1]");
    assert_eq!(s("sprintf('%.3d', 5)"), "005");
}

#[test]
fn sprintf_character_arguments() {
    assert_eq!(s("sprintf('%s-%s', 'ab', 'cd')"), "ab-cd");
    assert_eq!(s("sprintf('%d', 'abc')"), "979899");
    assert_eq!(s("sprintf('%s', 65)"), "A");
    assert_eq!(s("sprintf('%c', 'abc')"), "abc");
    assert_eq!(s("sprintf('%s=%d;', 'a', 1, 'b', 2)"), "a=1;b=2;");
    // The reference consumes the empty char argument and applies field padding.
    assert_eq!(s("sprintf('[%5s]', '')"), "[     ]");
}

#[test]
fn sprintf_integer_conversion_of_non_integer_prints_exponent_form() {
    assert_eq!(s("sprintf('%d', 1.5)"), "1.500000e+00");
    assert_eq!(s("sprintf('%i', pi)"), "3.141593e+00");
    assert_eq!(s("sprintf('%s', pi)"), "3.141593e+00");
    assert_eq!(s("sprintf('%d', 1e10)"), "10000000000");
}

#[test]
fn sprintf_nan_and_inf() {
    assert_eq!(s("sprintf('%d %f %g', NaN, Inf, -Inf)"), "NaN Inf -Inf");
    assert_eq!(s("sprintf('[%6.2f]', NaN)"), "[   NaN]");
    assert_eq!(s("sprintf('[%-6d]', Inf)"), "[Inf   ]");
    assert_eq!(s("sprintf('[%+f]', Inf)"), "[+Inf]");
}

#[test]
fn sprintf_escapes() {
    assert_eq!(s("sprintf('a\\tb\\nc\\\\d')"), "a\tb\nc\\d");
    assert_eq!(s("sprintf('x\\ry\\az')"), "x\ry\u{7}z");
    assert_eq!(numbers("numel(sprintf('\\n'))"), vec![1.0]);
}

#[test]
fn sprintf_errors_are_explicit() {
    assert!(err("sprintf(5)").contains("sprintf: format must be a character row"));
    assert!(err("sprintf('%d', {1})").contains("not supported yet"));
    assert!(err("sprintf('%a', 1)").contains("not supported yet"));
}

#[test]
fn fprintf_prints_through_the_output_path() {
    let mut e = Engine::default();
    let out = e.execute("fprintf('hello\\n');").unwrap().output;
    assert_eq!(out, vec!["hello"]);
    let out = e
        .execute("fprintf('%d + %d = %d\\n', 1, 2, 3);")
        .unwrap()
        .output;
    assert_eq!(out, vec!["1 + 2 = 3"]);
    // No trailing newline: the next output continues on the same line.
    let out = e
        .execute("fprintf('a'); fprintf('b'); disp('c')")
        .unwrap()
        .output;
    assert_eq!(out, vec!["abc"]);
    // A dangling partial line is still shown when the run ends.
    let out = e.execute("fprintf('tail')").unwrap().output;
    assert_eq!(out, vec!["tail"]);
    let out = e
        .execute("fprintf(1, 'to %s\\n', 'stdout'); fprintf(2, 'err\\n');")
        .unwrap()
        .output;
    assert_eq!(out, vec!["to stdout", "err"]);
    let out = e.execute("fprintf('a\\nb\\n\\nc\\n');").unwrap().output;
    assert_eq!(out, vec!["a", "b", "", "c"]);
}

#[test]
fn fprintf_returns_byte_count_and_stays_silent() {
    let mut e = Engine::default();
    let out = e.execute("n = fprintf('abc\\n');").unwrap().output;
    assert_eq!(out, vec!["abc"]);
    assert_eq!(numbers_of(&e, "n"), vec![4.0]);
    // A bare call must not create a visible `ans`.
    let out = e.execute("fprintf('x\\n')").unwrap().output;
    assert_eq!(out, vec!["x"]);
    assert!(e.array("ans").is_none());
}
fn numbers_of(e: &Engine, name: &str) -> Vec<f64> {
    let Array::Numeric(m) = e.array(name).unwrap() else {
        panic!("not numeric")
    };
    m.data().to_vec()
}

#[test]
fn fprintf_rejects_file_ids_that_are_not_open() {
    assert!(err("fprintf(3, 'x')").contains("Invalid file identifier"));
    assert!(err("fprintf(1)").contains("format must be a character row"));
}

#[test]
fn num2str_scalars() {
    assert_eq!(s("num2str(pi)"), "3.1416");
    assert_eq!(s("num2str(1e10)"), "10000000000");
    assert_eq!(s("num2str(-0.5)"), "-0.5");
    assert_eq!(s("num2str(42)"), "42");
    assert_eq!(s("num2str(0)"), "0");
    assert_eq!(s("num2str(true)"), "1");
    assert_eq!(s("num2str('abc')"), "abc");
    assert_eq!(s("num2str(NaN)"), "NaN");
    assert_eq!(s("num2str(-Inf)"), "-Inf");
    assert_eq!(s("num2str(123.456)"), "123.456");
    assert_eq!(s("num2str(0.000123456)"), "0.00012346");
    assert_eq!(s("num2str([])"), "");
}

#[test]
fn num2str_precision_and_format() {
    assert_eq!(s("num2str(pi, 8)"), "3.1415927");
    assert_eq!(s("num2str(pi, '%10.5f')"), "3.14159");
    assert_eq!(s("num2str(1234567, 3)"), "1.23e+06");
    assert_eq!(s("num2str([1 2 3], '%d,')"), "1,2,3,");
}

#[test]
fn num2str_matrices() {
    let mut e = Engine::default();
    run(
        &mut e,
        "a = num2str([1 2;3 4]); sz = size(a); b = num2str([1.5 -2.25]); c = num2str([10 5]);",
    );
    assert_eq!(numbers_of(&e, "sz"), vec![2.0, 4.0]);
    let Array::Character(a) = e.array("a").unwrap() else {
        panic!()
    };
    // Column-major units of ["1  2"; "3  4"].
    let text: String = a
        .units()
        .iter()
        .map(|&u| char::from_u32(u32::from(u)).unwrap())
        .collect();
    assert_eq!(text, "13    24");
    assert_eq!(text_of(&e, "b"), "1.5       -2.25");
    assert_eq!(text_of(&e, "c"), "10   5");
}

#[test]
fn num2str_complex_and_int2str() {
    assert_eq!(s("num2str(1+2i)"), "1+2i");
    assert_eq!(s("num2str(1.5-2.5i)"), "1.5-2.5i");
    assert_eq!(s("int2str(2.5)"), "3");
    assert_eq!(s("int2str(-2.5)"), "-3");
    assert_eq!(s("int2str([1.2 3.7])"), "1  4");
    // int2str formats character codes (reference case C1).
    assert_eq!(s("int2str('ab')"), "97  98");
}

#[test]
fn mat2str_forms() {
    assert_eq!(s("mat2str(5)"), "5");
    assert_eq!(s("mat2str([1 2 3])"), "[1 2 3]");
    assert_eq!(s("mat2str([1 2;3 4])"), "[1 2;3 4]");
    assert_eq!(s("mat2str([1.5 NaN;Inf -Inf])"), "[1.5 NaN;Inf -Inf]");
    assert_eq!(s("mat2str(pi)"), "3.14159265358979");
    assert_eq!(s("mat2str(pi, 4)"), "3.142");
    assert_eq!(s("mat2str(true)"), "true");
    assert_eq!(s("mat2str([true false])"), "[true false]");
    assert_eq!(s("mat2str(zeros(0,3))"), "zeros(0,3)");
    assert_eq!(s("mat2str([1+2i 3-4i])"), "[1+2i 3-4i]");
    assert_eq!(s("mat2str('abc')"), "'abc'");
}

#[test]
fn strsplit_strjoin_round_trip() {
    assert_eq!(cell("strsplit('a,b,c', ',')"), vec!["a", "b", "c"]);
    assert_eq!(cell("strsplit('a b  c')"), vec!["a", "b", "c"]);
    assert_eq!(
        cell("strsplit('a,,b', ',', 'CollapseDelimiters', false)"),
        vec!["a", "", "b"]
    );
    assert_eq!(cell("strsplit('a-b+c', {'-', '+'})"), vec!["a", "b", "c"]);
    assert_eq!(cell("strsplit(',a,', ',')"), vec!["", "a", ""]);
    assert_eq!(s("strjoin(strsplit('a,b,c', ','), ',')"), "a,b,c");
    assert_eq!(s("strjoin({'x','y','z'})"), "x y z");
    assert_eq!(s("strjoin({'x','y'}, '\\n')"), "x\ny");
    assert_eq!(s("strjoin({}, ',')"), "");
}

#[test]
fn string_helpers() {
    assert_eq!(s("strcat('a ', 'b ', 'c')"), "abc");
    assert_eq!(cell("strcat({'a ', 'b'}, 'x')"), vec!["a x", "bx"]);
    assert_eq!(s("strrep('hello world', 'o', '00')"), "hell00 w00rld");
    assert_eq!(s("strrep('aaa', 'aa', 'b')"), "bb");
    assert_eq!(cell("strrep({'ab','cb'}, 'b', 'x')"), vec!["ax", "cx"]);
    assert_eq!(s("strtrim(sprintf('  hi \\t\\n'))"), "hi");
    assert_eq!(cell("strtrim({' a ', 'b  '})"), vec!["a", "b"]);
    assert_eq!(s("upper('abC1')"), "ABC1");
    assert_eq!(s("lower('ABc1')"), "abc1");
    assert_eq!(cell("upper({'ab','c'})"), vec!["AB", "C"]);
    assert_eq!(s("blanks(3)"), "   ");
    assert_eq!(s("['[', blanks(2), ']']"), "[  ]");
    assert_eq!(s("deblank('ab   ')"), "ab");
    assert_eq!(s("fliplr('abc')"), "cba");
    assert_eq!(logical("ischar('a')"), vec![true]);
    assert_eq!(logical("ischar(1)"), vec![false]);
    assert_eq!(logical("iscellstr({'a','b'})"), vec![true]);
    assert_eq!(logical("iscellstr({'a',1})"), vec![false]);
    assert_eq!(logical("iscellstr('a')"), vec![false]);
}

#[test]
fn strcmp_family() {
    assert_eq!(logical("strcmp('abc', 'abc')"), vec![true]);
    assert_eq!(logical("strcmp('abc', 'abd')"), vec![false]);
    assert_eq!(logical("strcmp('abc', 5)"), vec![false]);
    assert_eq!(
        logical("strcmp({'a','b','a'}, 'a')"),
        vec![true, false, true]
    );
    assert_eq!(logical("strcmp('a', {'a';'b'})"), vec![true, false]);
    assert_eq!(logical("strcmp({'a','b'}, {'a','c'})"), vec![true, false]);
    assert_eq!(logical("strcmpi('ABC', 'abc')"), vec![true]);
    assert_eq!(logical("strncmp('abcdef', 'abcxyz', 3)"), vec![true]);
    assert_eq!(logical("strncmp('abcdef', 'abcxyz', 4)"), vec![false]);
    assert_eq!(logical("strncmp('ab', 'abc', 3)"), vec![false]);
    assert_eq!(logical("strncmpi('ABcd', 'abXX', 2)"), vec![true]);
    let mut e = Engine::default();
    run(&mut e, "r = strcmp({'a','b';'a','a'}, 'a'); sz = size(r);");
    assert_eq!(numbers_of(&e, "sz"), vec![2.0, 2.0]);
}

#[test]
fn strfind_and_num2cell() {
    assert_eq!(numbers("strfind('abcabc', 'bc')"), vec![2.0, 5.0]);
    assert_eq!(numbers("strfind('aaa', 'aa')"), vec![1.0, 2.0]);
    let mut e = Engine::default();
    run(
        &mut e,
        "r = strfind('abc', 'x'); e1 = isempty(r); n = numel(strfind('abc', ''));",
    );
    assert_eq!(numbers_of(&e, "n"), vec![0.0]);
    assert_eq!(logical_of(&e, "e1"), vec![true]);
    run(
        &mut e,
        "c = num2cell([1 2;3 4]); sz = size(c); v = c{2,1}; l = num2cell([true false]); k = class(l{1});",
    );
    assert_eq!(numbers_of(&e, "sz"), vec![2.0, 2.0]);
    assert_eq!(numbers_of(&e, "v"), vec![3.0]);
    assert_eq!(text_of(&e, "k"), "logical");
    run(&mut e, "f = strfind({'ab','ba','x'}, 'a'); g = f{2};");
    assert_eq!(numbers_of(&e, "g"), vec![2.0]);
}
fn logical_of(e: &Engine, name: &str) -> Vec<bool> {
    let Array::Logical(m) = e.array(name).unwrap() else {
        panic!("not logical")
    };
    m.data().iter().map(|&v| v != 0).collect()
}

#[test]
fn text_functions_work_on_variables_and_report_bad_input() {
    let mut e = Engine::default();
    run(
        &mut e,
        "t = 'a;b;c'; p = strsplit(t, ';'); j = strjoin(p, '+'); n = numel(p);",
    );
    assert_eq!(text_of(&e, "j"), "a+b+c");
    assert_eq!(numbers_of(&e, "n"), vec![3.0]);
    assert!(err("strtrim(5)").contains("strtrim"));
    assert!(err("strjoin({1,2})").contains("strjoin"));
    assert!(err("num2str({1})").contains("num2str"));
    assert!(
        err("strsplit('a', 'b', 'DelimiterType', 'RegularExpression')")
            .contains("not supported yet")
    );
}

// ---- fix round 1 -------------------------------------------------------------

#[test]
fn fprintf_held_text_is_charged_to_the_output_budget() {
    let message = err("for k = 1:40, fprintf('%s', blanks(65536)); end");
    assert!(message.contains("Execution output exceeds"), "{message}");
    // Text below the budget still flushes at the end of the run.
    let mut e = Engine::default();
    let out = e
        .execute("for k = 1:3, fprintf('%s', 'ab'); end")
        .unwrap()
        .output;
    assert_eq!(out, vec!["ababab"]);
    // Held bytes are not charged twice when the next output continues the line.
    let out = e
        .execute("for k = 1:31, fprintf('%s', blanks(65536)); end; disp('x')")
        .unwrap()
        .output;
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].len(), 31 * 65536 + 1);
}

#[test]
fn num2str_format_and_result_are_bounded() {
    let message = err("num2str(ones(1,65536), blanks(65536))");
    assert!(
        message.contains("num2str: format repeated over the matrix exceeds 2 MiB"),
        "{message}"
    );
    // A star width makes one row far wider than the others: the padded block
    // would exceed the character limit.
    let message = err("num2str([70000 1;1 1], '%*d')");
    assert!(
        message.contains("num2str: result exceeds 65536 characters"),
        "{message}"
    );
    let message = err("num2str(ones(1,70000))");
    assert!(message.contains("exceeds 65536 characters"), "{message}");
    assert_eq!(s("num2str([1 2 3], '%d,')"), "1,2,3,");
}

#[test]
fn num2str_complex_aligns_each_column_once() {
    let mut e = Engine::default();
    run(&mut e, "a = num2str([1+2i 10+20i; 3+4i 5+6i]);");
    let Array::Character(a) = e.array("a").unwrap() else {
        panic!()
    };
    let (rows, cols) = (a.rows(), a.cols());
    let units: Vec<char> = a
        .units()
        .iter()
        .map(|&u| char::from_u32(u32::from(u)).unwrap())
        .collect();
    let row = |r: usize| (0..cols).map(|c| units[c * rows + r]).collect::<String>();
    assert_eq!(
        (rows, row(0), row(1)),
        (2, "1+2i   10+20i".into(), "3+4i    5+6i ".into())
    );
    let message = err("num2str(ones(1,70000)*(1+2i))");
    assert!(message.contains("exceeds 65536 characters"), "{message}");
}

#[test]
fn num2cell_checks_the_cell_bound_first() {
    let message = err("num2cell(ones(1,70000))");
    assert!(
        message.contains("num2cell: result exceeds 65536 cells"),
        "{message}"
    );
    assert_eq!(numbers("numel(num2cell(ones(1,65536)))"), vec![65536.0]);
    assert_eq!(
        numbers("numel(strsplit(blanks(20000), ' ', 'CollapseDelimiters', false))"),
        vec![20001.0]
    );
}

#[test]
fn strrep_replaces_overlapping_matches() {
    assert_eq!(s("strrep('aaa', 'aa', 'b')"), "bb");
    assert_eq!(
        s("strrep('abc 2 def 22 ghi 222 jkl 2222', '22', '*')"),
        "abc 2 def * ghi ** jkl ***"
    );
    assert_eq!(s("strrep('ababa', 'aba', 'X')"), "XX");
    assert_eq!(s("strrep('abc', 'x', 'y')"), "abc");
    assert_eq!(
        cell("strrep({'aaa', 'xaax'}, 'aa', 'b')"),
        vec!["bb", "xbx"]
    );
}

#[test]
fn mat2str_of_empty_arrays() {
    assert_eq!(s("mat2str([])"), "[]");
    assert_eq!(s("mat2str(logical([]))"), "[]");
    assert_eq!(s("mat2str(zeros(0,3))"), "zeros(0,3)");
    assert_eq!(s("mat2str(zeros(2,0))"), "zeros(2,0)");
    assert_eq!(s("mat2str(true(0,3))"), "zeros(0,3)");
    assert_eq!(s("mat2str('')"), "''");
}

#[test]
fn strncmp_equal_strings_shorter_than_n() {
    assert_eq!(logical("strncmp('ab', 'ab', 3)"), vec![true]);
    assert_eq!(logical("strncmp('ab', 'abc', 3)"), vec![false]);
    assert_eq!(logical("strncmpi('AB', 'ab', 5)"), vec![true]);
    assert_eq!(logical("strncmp({'ab','abc'}, 'ab', 3)"), vec![true, false]);
}

#[test]
fn strsplit_default_splits_on_all_whitespace() {
    assert_eq!(
        cell("strsplit(sprintf('a\\tb\\n c\\rd\\fe\\vf'))"),
        vec!["a", "b", "c", "d", "e", "f"]
    );
    assert_eq!(cell("strsplit(' a ')"), vec!["", "a", ""]);
}

#[test]
fn sprintf_hex_and_octal_of_large_integers() {
    assert_eq!(s("sprintf('%x', 2^63)"), "8000000000000000");
    assert_eq!(s("sprintf('%X', 2^64 - 2048)"), "FFFFFFFFFFFFF800");
    assert_eq!(s("sprintf('%o', 2^63)"), "1000000000000000000000");
    assert_eq!(s("sprintf('%x', 9.5e18)"), "83d6c7aab6360000");
    let message = err("sprintf('%x', 2^64)");
    assert!(
        message.contains("sprintf: %x/%o of values above 2^64 is not supported yet"),
        "{message}"
    );
    assert!(err("sprintf('%o', 2^70)").contains("not supported yet"));
    // Reference behavior: %d of an integer outside int64 prints as %e.
    assert_eq!(s("sprintf('%d', 2^70)"), "1.180592e+21");
}

#[test]
fn strfind_counts_utf16_units() {
    assert_eq!(numbers("strfind(char([55357 56832 97]), 'a')"), vec![3.0]);
    assert_eq!(
        numbers("strfind(char([55357 56832 97 55357 56832 97]), 'a')"),
        vec![3.0, 6.0]
    );
}

#[test]
fn strsplit_rejects_too_many_pieces_before_building_them() {
    let message = err("strsplit(blanks(65536), ' ', 'CollapseDelimiters', false)");
    assert!(
        message.contains("strsplit: result exceeds 65536 cells"),
        "{message}"
    );
}

#[test]
fn sprintf_complex_consumes_one_element_with_the_real_part() {
    assert_eq!(s("sprintf('%d,', [1+2i 3])"), "1,3,");
}

#[test]
fn strtrim_keeps_nul_but_deblank_removes_it() {
    assert_eq!(numbers("numel(strtrim(char([0 97 32])))"), vec![2.0]);
    assert_eq!(numbers("numel(deblank(char([97 0 32])))"), vec![1.0]);
}

/// Preserve row padding and shape when checking reference character matrices.
fn assert_char_rows(expr: &str, expected: &[&str]) {
    let mut e = Engine::default();
    run(&mut e, &format!("r = {expr};"));
    let Array::Character(c) = e.array("r").unwrap() else {
        panic!("{expr}: not character")
    };
    assert_eq!(c.rows(), expected.len(), "{expr}");
    for (r, expected) in expected.iter().enumerate() {
        assert_eq!(c.cols(), expected.encode_utf16().count(), "{expr}");
        let units: Vec<u16> = (0..c.cols())
            .map(|col| c.units()[col * c.rows() + r])
            .collect();
        assert_eq!(
            String::from_utf16(&units).unwrap(),
            *expected,
            "{expr}, row {r}"
        );
    }
}

#[test]
fn reference_num2str_and_int2str_column_widths() {
    assert_eq!(s("num2str([1 -2 3])"), "1 -2  3");
    assert_eq!(s("num2str([1.5 -2.25])"), "1.5       -2.25");
    assert_char_rows(
        "num2str([0.1 22.5;3 -4])",
        &["0.1         22.5", "  3           -4"],
    );
    assert_char_rows("int2str([1.4 2.6;-0.5 3])", &[" 1  3", "-1  3"]);
    assert_eq!(s("num2str([1+2i 3+4i])"), "1+2i   3+4i");
    assert_eq!(s("num2str([NaN Inf -Inf])"), "NaN  Inf -Inf");
}

#[test]
fn reference_sprintf_unsigned_fallback() {
    for conversion in ['u', 'x', 'X', 'o'] {
        assert_eq!(s(&format!("sprintf('%{conversion}',-1)")), "-1.000000e+00");
        assert_eq!(s(&format!("sprintf('%{conversion}',1.5)")), "1.500000e+00");
        assert_eq!(
            s(&format!("sprintf('%{conversion}',-1.5)")),
            "-1.500000e+00"
        );
    }
}

#[test]
fn reference_sprintf_rounds_exact_binary_halves_away_from_zero() {
    assert_eq!(s("sprintf('%.0f %.0f %.0f',0.5,1.5,2.5)"), "1 2 3");
    assert_eq!(s("sprintf('%.0f %.0f %.0f',-0.5,-1.5,-2.5)"), "-1 -2 -3");
    assert_eq!(s("sprintf('%.1f %.1f',0.25,0.35)"), "0.3 0.3");
    assert_eq!(s("sprintf('%.1f %.1f',-0.25,-0.35)"), "-0.3 -0.3");
    assert_eq!(
        s("sprintf('%.0e %.0e %.1g',2.5,9.5,250)"),
        "3e+00 1e+01 3e+02"
    );
    // The reference prints 17 significant digits, then zeros.
    assert_eq!(
        s("sprintf('%.54f',0.1)"),
        "0.100000000000000010000000000000000000000000000000000000"
    );
    assert_eq!(
        s("sprintf('%.0f %.0f',0.49999999999999994,0.5000000000000001)"),
        "0 1"
    );
}

#[test]
fn reference_sprintf_pads_empty_character_arguments() {
    assert_eq!(s("sprintf('[%5s]','')"), "[     ]");
    assert_eq!(s("sprintf('[%-5s]','')"), "[     ]");
    assert_eq!(s("sprintf('[%s]','')"), "[]");
    assert_eq!(s("sprintf('%s|%s|','a','')"), "a||");
    assert_eq!(s("sprintf('%d','')"), "");
}

#[test]
fn reference_mat2str_empty_logical_and_quoted_characters() {
    assert_eq!(s("mat2str(true(0,3))"), "zeros(0,3)");
    assert_eq!(s("mat2str(false(2,0))"), "zeros(2,0)");
    assert_eq!(s("mat2str(logical([]))"), "[]");
    assert_eq!(s("mat2str('abc')"), "'abc'");
    assert_eq!(s("mat2str('it''s')"), "'it''s'");
    assert_eq!(s("mat2str(reshape('acbd',2,2))"), "['ab';'cd']");
    assert_eq!(s("mat2str(reshape('a''b''',2,2))"), "['ab';'''''']");
}

#[test]
fn reference_strcat_trims_char_arguments_and_preserves_cell_contents() {
    assert_eq!(cell("strcat('a ',{'b '})"), vec!["ab "]);
    assert_eq!(cell("strcat({'a '},'b ','c')"), vec!["a bc"]);
    assert_eq!(cell("strcat({'a ','b'},'x')"), vec!["a x", "bx"]);
}

#[test]
fn reference_strncmp_zero_length() {
    assert_eq!(logical("strncmp('ab','cd',0)"), vec![true]);
    assert_eq!(logical("strncmpi('AB','cd',0)"), vec![true]);
    assert_eq!(logical("strncmp({'ab',''},'cd',0)"), vec![true, true]);
}

#[test]
fn reference_num2str_preserves_character_matrix_and_shared_storage() {
    assert_char_rows("num2str(reshape('acbd',2,2))", &["ab", "cd"]);
    let mut e = Engine::default();
    run(&mut e, "a = reshape('acbd',2,2); r = num2str(a);");
    let (Array::Character(a), Array::Character(r)) = (e.array("a").unwrap(), e.array("r").unwrap())
    else {
        panic!("not character")
    };
    assert!(a.same_storage(&r));
}

#[test]
fn reference_deblank_removes_only_shared_trailing_blank_columns() {
    assert_char_rows("deblank(reshape('acb     ',2,4))", &["ab", "c "]);
    assert_char_rows("deblank(reshape(' a b',2,2))", &["  ", "ab"]);
    assert_char_rows("deblank(reshape(char([97 98 0 32]),2,2))", &["a", "b"]);
}

/// Reference output for the round-3 guesses (run 2026-10-22), exactly.
#[test]
fn reference_round4_reference_values() {
    for (expr, expected) in [
        ("sprintf('%.1f %.1f',0.25,0.35)", "0.3 0.3"),
        ("sprintf('%.1f %.1f',-0.25,-0.35)", "-0.3 -0.3"),
        ("sprintf('%.0e %.0e %.1g',2.5,9.5,250)", "3e+00 1e+01 3e+02"),
        (
            "sprintf('%.54f',0.1)",
            "0.100000000000000010000000000000000000000000000000000000",
        ),
        ("mat2str(false(2,0))", "zeros(2,0)"),
        ("mat2str(logical([]))", "[]"),
        ("num2str([1 NaN Inf -Inf])", "1  NaN  Inf -Inf"),
        ("num2str([-10 2 -3])", "-10   2  -3"),
        ("num2str([1.5 -2.25],3)", "1.5     -2.25"),
        ("sprintf('%d',2^70)", "1.180592e+21"),
        ("sprintf('%d','')", ""),
        ("sprintf('a%d|',5,'')", "a5|a|"),
        ("sprintf('%d|',[])", "|"),
        ("sprintf('%.0f %.0f',0.5,-0.5)", "1 -1"),
        ("sprintf('%.2f',1.005)", "1.00"),
    ] {
        assert_eq!(s(expr), expected, "{expr}");
    }
    assert_char_rows(
        "num2str([1+2i 10+20i;3+4i 5+6i])",
        &["1+2i   10+20i", "3+4i    5+6i "],
    );
}

/// The runtime rounds its 17-digit string, not the exact binary value
/// (1.5047204674288635 is just below ...8635). Model value: reference check listed.
#[test]
fn sprintf_rounds_the_17_digit_string() {
    assert_eq!(
        s("sprintf('%.15f',1.5047204674288635)"),
        "1.504720467428864"
    );
    assert_eq!(s("sprintf('%f',2^70)"), "1180591620717411300000.000000");
    assert_eq!(s("sprintf('%.3e|%g|%.1f',0,0,0)"), "0.000e+00|0|0.0");
    assert_eq!(s("sprintf('%.1f|%.0f|%.2g',0.96,0.04,9.96)"), "1.0|0|10");
}

#[test]
fn sprintf_counts_arguments_before_copying_them() {
    let mut e = Engine::default();
    let before = ALLOCATED.with(Cell::get);
    run(&mut e, "x = zeros(1, 16777216);");
    // The counter sees the engine's allocations on this thread: 16M doubles.
    assert!(ALLOCATED.with(Cell::get) - before >= 16 << 23);
    for (call, who) in [
        ("s = sprintf('%d', x);", "sprintf"),
        ("n = fprintf('%d', x);", "fprintf"),
    ] {
        let before = ALLOCATED.with(Cell::get);
        let message = e.execute(call).unwrap_err();
        let allocated = ALLOCATED.with(Cell::get) - before;
        assert!(
            message.contains(&format!("{who}: too many arguments")),
            "{message}"
        );
        // Copying the argument list would take 16M items of 16+ bytes each.
        assert!(allocated < 16 << 20, "{call}: allocated {allocated} bytes");
    }
}
