//! P0 visible follow-ups (docs/tasks/FOLLOWUPS.md): number, char, cell and
//! struct display, `3.5+0i` narrowing, char conversions and sorting, and
//! complex `chol`/`qr`. Every expected string names its source: a reference
//! corpus case under `tests/fixtures/` (captured once from the reference
//! product; no new reference runs), a published documentation example, or the
//! display rule implemented in `src/display_format.rs`.
use matrust::{Array, Engine};

/// What the console shows for `source`: each entry plus the newline it adds.
fn console(e: &mut Engine, source: &str) -> String {
    e.execute(source)
        .unwrap_or_else(|error| panic!("{source}: {error}"))
        .output
        .iter()
        .map(|entry| format!("{entry}\n"))
        .collect()
}

fn fresh(source: &str) -> String {
    console(&mut Engine::default(), source)
}

fn class_of(e: &Engine, name: &str) -> String {
    let a = e.array(name).unwrap_or_else(|| panic!("{name} missing"));
    format!("{} [{} {}]", a.class(), a.rows(), a.cols())
}

// ---- 1. integer, single and char-matrix display ----------------------------

#[test]
fn integer_and_single_display_matches_reference_cases() {
    for (source, expected, origin) in [
        (
            "x = int8(5)",
            "\nx =\n\n  int8\n\n   5\n\n",
            "display/expected.txt 55",
        ),
        (
            "x = uint8([1 2 3])",
            "\nx =\n\n  uint8\n\n  1  2  3\n\n",
            "display/expected.txt 56",
        ),
        (
            "x = int8([1 2.5 -3.5 300])",
            "\nx =\n\n  int8\n\n     1     3    -4   127\n\n",
            "integers/expected.txt 29",
        ),
        (
            "x = int32(5) + int32([1 2; 3 4])",
            "\nx =\n\n  int32\n\n  6  7\n  8  9\n\n",
            "integers/expected-2.txt 18",
        ),
        (
            "x = abs(int8([-128 -5 5]))",
            "\nx =\n\n  int8\n\n  127    5    5\n\n",
            "integers/expected-2.txt 21",
        ),
        (
            "x = single(pi)",
            "\nx =\n\n    3.1416\n\n",
            "display/expected.txt 57",
        ),
        // Rule: single uses the double layout under `format short` (whole
        // values without a decimal point, 4 decimals otherwise).
        (
            "x = single([1 2 3])",
            "\nx =\n\n     1     2     3\n\n",
            "rule",
        ),
        (
            "x = single([1.5 2.25])",
            "\nx =\n\n    1.5000    2.2500\n\n",
            "rule",
        ),
    ] {
        assert_eq!(fresh(source), expected, "{source} ({origin})");
    }
}

/// `format long` shows single with 7 decimals: the published `format` page
/// example gives `3.1415927` for `single(pi)`.
#[test]
fn single_shows_seven_decimals_under_format_long() {
    let mut e = Engine::default();
    console(&mut e, "format long");
    assert_eq!(
        console(&mut e, "x = single(pi)"),
        "\nx =\n\n   3.1415927\n\n"
    );
}

#[test]
fn char_matrix_prints_one_row_per_line() {
    // display/expected.txt 49, probe_expected.txt P95 and P127.
    assert_eq!(fresh("x = ['ab';'cd']"), "\nx =\n\nab\ncd\n\n");
    assert_eq!(fresh("x = ['abc';'de ']"), "\nx =\n\nabc\nde \n\n");
    assert_eq!(fresh("disp(['ab';'cd'])"), "ab\ncd\n");
}

// ---- 2. `3.5+0i` narrows to real -------------------------------------------

/// The reference narrows an operation result whose imaginary parts are all
/// zero (probe_expected.txt P83 prints `3.5000`); `complex(a,0)` stays complex
/// (published `complex` page), so `isreal` differs between the two.
#[test]
fn zero_imaginary_literal_sum_narrows_but_complex_call_does_not() {
    assert_eq!(fresh("x = 3.5+0i"), "\nx =\n\n    3.5000\n\n");
    let mut e = Engine::default();
    console(&mut e, "a = isreal(3.5+0i); b = isreal(complex(3.5,0));");
    let Some(Array::Logical(a)) = e.array("a") else {
        panic!("a")
    };
    let Some(Array::Logical(b)) = e.array("b") else {
        panic!("b")
    };
    assert_eq!((a.data()[0], b.data()[0]), (1, 0));
    // Rule: complex scalar layout `re + imi` with a fixed-point zero part.
    assert_eq!(
        fresh("x = complex(3.5,0)"),
        "\nx =\n\n   3.5000 + 0.0000i\n\n"
    );
}

// ---- 3. char conversions ---------------------------------------------------

#[test]
fn double_of_text_and_disp_of_char_matrix() {
    let mut e = Engine::default();
    // Rule: whole doubles below 1000 print 6 wide.
    assert_eq!(
        console(&mut e, "x = double('abc')"),
        "\nx =\n\n    97    98    99\n\n"
    );
    assert_eq!(class_of(&e, "x"), "double [1 3]");
    assert_eq!(fresh("disp(['ab';'cd'])"), "ab\ncd\n");
    assert_eq!(
        fresh("x = double(['ab';'cd'])"),
        "\nx =\n\n    97    98\n    99   100\n\n"
    );
}

// ---- 4. sort / sortrows / unique keep char ----------------------------------

#[test]
fn sort_sortrows_and_unique_keep_char_class() {
    let mut e = Engine::default();
    assert_eq!(console(&mut e, "x = sort('hello')"), "\nx =\n\nehllo\n\n");
    assert_eq!(class_of(&e, "x"), "char [1 5]");
    assert_eq!(
        console(&mut e, "x = sort('hello', 'descend')"),
        "\nx =\n\nollhe\n\n"
    );
    assert_eq!(console(&mut e, "[x, i] = sort('cab');"), "", "suppressed");
    assert_eq!(class_of(&e, "x"), "char [1 3]");
    let Some(Array::Numeric(i)) = e.array("i") else {
        panic!("i")
    };
    assert_eq!(i.data(), &[2., 3., 1.]);
    assert_eq!(
        console(&mut e, "x = sortrows(['cb';'ab';'ba'])"),
        "\nx =\n\nab\nba\ncb\n\n"
    );
    assert_eq!(class_of(&e, "x"), "char [3 2]");
    assert_eq!(console(&mut e, "x = unique('hello')"), "\nx =\n\nehlo\n\n");
    assert_eq!(class_of(&e, "x"), "char [1 4]");
}

// ---- 5. cell text ellipsis and struct-array field list ----------------------

/// probe2_expected.txt Q23 (a 1x4 cell keeps 18 characters; the reference's
/// `evalc` stored the U+2026 mark as the byte 0x1A).
#[test]
fn long_text_in_a_four_column_cell_is_shortened() {
    assert_eq!(
        fresh(
            "x = {1, 'abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz', 2, 'abcdefghijklmnopqrstuvwxyz0123456789'}"
        ),
        "\nx = \n\n    [1]    'abcdefghijklmnopqr\u{2026}'    [2]    'abcdefghijklmnopqr\u{2026}'\n\n"
    );
}

/// envfiles-expected.txt 36 (1x3 keeps 27 characters; 28 and 29 stay whole)
/// and lang2-expected.txt 42 (a 1x2 cell shows 34 characters whole).
#[test]
fn cell_text_budget_depends_on_the_column_count() {
    assert_eq!(
        fresh("x = {0, 'No directories were removed.', 'MATLAB:RMDIR:NoDirectoriesRemoved'}"),
        "\nx = \n\n    [0]    'No directories were removed.'    'MATLAB:RMDIR:NoDirectoriesR\u{2026}'\n\n"
    );
    assert_eq!(
        fresh("x = {1, 'Directory already exists.', 'MATLAB:MKDIR:DirectoryExists'}"),
        "\nx = \n\n    [1]    'Directory already exists.'    'MATLAB:MKDIR:DirectoryExists'\n\n"
    );
    assert_eq!(
        fresh("x = {'MATLAB:nargoutchk:notEnoughOutputs', 'Not enough output arguments.'}"),
        "\nx = \n\n    'MATLAB:nargoutchk:notEnoughOutputs'    'Not enough output arguments.'\n\n"
    );
    // A single-column cell shows its text whole (cells-expected style, rule).
    let long = "a".repeat(120);
    assert_eq!(
        fresh(&format!("x = {{'{long}'}}")),
        format!("\nx = \n\n    '{long}'\n\n")
    );
    // Five and more columns share the four-column budget (lang3-expected.txt 98).
    assert_eq!(
        fresh("x = {'function', 'type', 'file', 'workspace', 'within_file_path'}"),
        "\nx = \n\n    'function'    'type'    'file'    'workspace'    'within_file_path'\n\n"
    );
    // The shortened value itself is untouched.
    let mut e = Engine::default();
    console(&mut e, "x = {1, 2, repmat('z', 1, 40)}; n = numel(x{3});");
    let Some(Array::Numeric(n)) = e.array("n") else {
        panic!("n")
    };
    assert_eq!(n.data(), &[40.]);
}

/// Rule: a doubled quote is never split at the cut.
#[test]
fn shortened_cell_text_keeps_doubled_quotes_whole() {
    // 26 letters then an apostrophe: the cut after 27 shown characters would
    // fall inside `''`; the mark follows the letters instead.
    let text = format!("{}''{}", "b".repeat(26), "c".repeat(10));
    assert_eq!(
        fresh(&format!("x = {{1, 2, '{text}'}}")),
        format!(
            "\nx = \n\n    [1]    [2]    '{}\u{2026}'\n\n",
            "b".repeat(26)
        )
    );
}

fn at_width(width: usize, source: &str) -> String {
    let mut e = Engine::default();
    e.set_console_width(width);
    console(&mut e, source)
}

/// Rule at narrow consoles (the desktop allows widths down to 20,
/// command_window.rs): four columns share `width / 4`, so widths 20, 24 and 31
/// give budgets 5, 6 and 7, below the 8 that `B - 8` kept characters need. A
/// text then keeps one character and the mark only when that is narrower than
/// the whole text: empty, one- and two-character texts stay whole.
#[test]
fn narrow_console_shortens_cell_text_only_when_narrower() {
    // Width 24: one-character texts stay whole (were `'a…'`).
    assert_eq!(
        at_width(24, "x = {'a', 'b', 'c', 'd'}"),
        "\nx = \n\n  Columns 1 through 3\n\n    'a'    'b'    'c'\n\n  Column 4\n\n    'd'\n\n"
    );
    // Width 20: the empty text stays `''` (was a lone `'…'`).
    assert_eq!(
        at_width(20, "x = {'', 'ab', 'abc', 'abcdefgh'}"),
        "\nx = \n\n  Columns 1 through 2\n\n    ''    'ab'\n\n  Columns 3 through 4\n\n    'a\u{2026}'    'a\u{2026}'\n\n"
    );
    // Width 31: a doubled quote at the cut is kept whole (`'abc` shows `''abc`).
    assert_eq!(
        at_width(31, "x = {1, 'ab', '''abc', 'abcdefghijklmnop'}"),
        "\nx = \n\n  Columns 1 through 3\n\n    [1]    'ab'    '''\u{2026}'\n\n  Column 4\n\n    'a\u{2026}'\n\n"
    );
    // No width from 20 to 31 shows a text wider than whole, a lone mark or a
    // mark on a text it did not cut.
    let texts = ["", "a", "ab", "abc", "''", "'a", "a'", "abcdefghijklmnop"];
    for width in 20..=31 {
        for text in texts {
            let quoted = format!("'{}'", text.replace('\'', "''"));
            let source = format!("x = {{1, 2, 3, '{}'}}", text.replace('\'', "''"));
            let out = at_width(width, &source);
            let shown = out
                .lines()
                .rev()
                .find(|line| !line.trim().is_empty())
                .and_then(|line| line.rsplit("    ").next())
                .unwrap_or_default()
                .trim_end()
                .to_string();
            if shown != quoted {
                let kept = shown
                    .strip_prefix('\'')
                    .and_then(|s| s.strip_suffix("\u{2026}'"))
                    .unwrap_or_else(|| panic!("width {width}, {quoted}: shows {shown}"));
                assert!(
                    !kept.is_empty()
                        && quoted.starts_with(&format!("'{kept}"))
                        && shown.chars().count() < quoted.chars().count(),
                    "width {width}, {quoted}: shows {shown}"
                );
            }
        }
    }
}

/// envfiles-expected.txt 41 and 47 (`dir` results): struct-array field names
/// are indented four spaces, not right-aligned like scalar `name: value` lines.
#[test]
fn struct_array_lists_field_names_four_spaces_in() {
    assert_eq!(
        fresh("s = struct('name', {'a', 'b'}, 'datenum', 1)"),
        "\ns = \n\n1x2 struct array with fields:\n\n    name\n    datenum\n\n"
    );
    assert_eq!(
        fresh("s = struct('a', {1, 2}, 'dd', 5); disp(s)"),
        "1x2 struct array with fields:\n\n    a\n    dd\n\n"
    );
    // A scalar struct still right-aligns (display/expected.txt struct cases).
    assert_eq!(
        fresh("s = struct('a', 1, 'dd', 5)"),
        "\ns = \n\n     a: 1\n    dd: 5\n\n"
    );
}

// ---- 7. complex chol and qr against NumPy/SciPy -----------------------------

const QR_CHOL: &str = include_str!("fixtures/followups/complex_qr_chol.txt");

struct Block {
    case: String,
    name: String,
    rows: usize,
    cols: usize,
    data: Vec<(f64, f64)>,
}

fn blocks() -> Vec<Block> {
    let mut out = Vec::new();
    let mut lines = QR_CHOL.lines();
    while let Some(header) = lines.next() {
        let parts: Vec<&str> = header
            .strip_prefix("@@ ")
            .expect("block header")
            .split(' ')
            .collect();
        let numbers: Vec<f64> = lines
            .next()
            .expect("block data")
            .split(' ')
            .map(|v| v.parse().expect("number"))
            .collect();
        out.push(Block {
            case: parts[0].into(),
            name: parts[1].into(),
            rows: parts[2].parse().unwrap(),
            cols: parts[3].parse().unwrap(),
            data: numbers.chunks(2).map(|p| (p[0], p[1])).collect(),
        });
    }
    out
}

fn literal(b: &Block) -> String {
    let re: Vec<String> = b.data.iter().map(|z| format!("{:?}", z.0)).collect();
    let im: Vec<String> = b.data.iter().map(|z| format!("{:?}", z.1)).collect();
    format!(
        "reshape(complex([{}], [{}]), {}, {})",
        re.join(" "),
        im.join(" "),
        b.rows,
        b.cols
    )
}

fn values(e: &Engine, name: &str) -> (usize, usize, Vec<(f64, f64)>) {
    match e.array(name).unwrap_or_else(|| panic!("{name} missing")) {
        Array::Complex(m) => (
            m.rows(),
            m.cols(),
            m.data().iter().map(|z| (z.re, z.im)).collect(),
        ),
        Array::Numeric(m) => (
            m.rows(),
            m.cols(),
            m.data().iter().map(|&v| (v, 0.)).collect(),
        ),
        other => panic!("{name}: unexpected {}", other.class()),
    }
}

/// Relative 1e-12 against the largest reference magnitude of each output.
fn assert_close(case: &str, name: &str, actual: &(usize, usize, Vec<(f64, f64)>), b: &Block) {
    assert_eq!((actual.0, actual.1), (b.rows, b.cols), "{case} {name} size");
    let scale = b.data.iter().map(|z| z.0.hypot(z.1)).fold(1f64, f64::max);
    for (k, (a, r)) in actual.2.iter().zip(&b.data).enumerate() {
        let err = (a.0 - r.0).hypot(a.1 - r.1);
        assert!(
            err <= 1e-12 * scale,
            "{case} {name}[{k}] = {a:?}, NumPy {r:?} (error {err:e})"
        );
    }
}

#[test]
fn complex_chol_and_qr_match_numpy_and_scipy() {
    let all = blocks();
    let calls = [
        ("chol_upper", "R = chol(A);"),
        ("chol_lower", "L = chol(A, 'lower');"),
        ("chol_notpd", "[R, p] = chol(A);"),
        ("chol_notpd_lower", "[L, p] = chol(A, 'lower');"),
        ("qr_full", "[Q, R] = qr(A);"),
        ("qr_econ", "[Q, R] = qr(A, 0);"),
        ("qr_pivot", "[Q, R, P] = qr(A);"),
        ("qr_pivot_econ", "[Q, R, p] = qr(A, 0);"),
    ];
    for (case, call) in calls {
        let parts: Vec<&Block> = all.iter().filter(|b| b.case == case).collect();
        let input = parts.iter().find(|b| b.name == "A").expect("input");
        let mut e = Engine::default();
        console(&mut e, &format!("A = {}; {call}", literal(input)));
        for b in parts.iter().filter(|b| b.name != "A") {
            assert_close(case, &b.name, &values(&e, &b.name), b);
        }
    }
}

/// The factors reproduce their input (checks independent of LAPACK sign choices).
#[test]
fn complex_factors_reconstruct_their_input() {
    let all = blocks();
    let a = all
        .iter()
        .find(|b| b.case == "qr_pivot" && b.name == "A")
        .unwrap();
    let h = all
        .iter()
        .find(|b| b.case == "chol_upper" && b.name == "A")
        .unwrap();
    let mut e = Engine::default();
    console(
        &mut e,
        &format!(
            "A = {}; H = {}; [Q, R, P] = qr(A); e1 = norm(A*P - Q*R) / norm(A); \
             e2 = norm(Q'*Q - eye(3)); R2 = chol(H); e3 = norm(R2'*R2 - H) / norm(H); \
             L = chol(H, 'lower'); e4 = norm(L*L' - H) / norm(H);",
            literal(a),
            literal(h)
        ),
    );
    for name in ["e1", "e2", "e3", "e4"] {
        let (_, _, v) = values(&e, name);
        assert!(v[0].0 < 1e-14, "{name} = {}", v[0].0);
    }
}
